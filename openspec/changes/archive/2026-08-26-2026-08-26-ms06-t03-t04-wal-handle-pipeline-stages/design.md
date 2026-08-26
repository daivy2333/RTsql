# Design：MS06-T03 WAL 持久句柄 + MS06-T04 Pipeline 三阶段拆分

> 关联 proposal：`proposal.md`
> 调查基线：revision `f392c73eb0dbfe2e15902777d2574ef892475427`（2026-08-26，干净工作区）

## 1. Current-State Evidence（Plan 已确认的实现事实）

### 1.1 T03 相关事实

**`src/wal/writer.rs`（180 行，全文已读）**

- `WalWriter` 字段：`wal_path: PathBuf` + `write_count: AtomicU64` + `checkpoint_threshold: u64`（L13-17）。不持有任何文件句柄
- `open()`（L21-37）：`OpenOptions create+append+read` 打开一次仅为确保文件存在，随即丢弃句柄
- 5 个 IO 方法全部在 `spawn_blocking` 内逐次 open：
  - `write_record`（L40-68）：open(append) → `seek(End(0))` → `stream_position()` 取 LSN → `write_all` → 返回 lsn
  - `fsync`（L71-86）：open(write) → `sync_all`
  - `truncate_to`（L89-105）：open(write) → `set_len(lsn)`
  - `get_current_lsn`（L129-147）：open(read) → `metadata().len()`
  - `write_batch`（L153-179）：open(append) → 逐条 `serialize_with_lsn` + `write_all` → `sync_all`
- 无 `#[cfg(test)]` 单元测试；行为测试在 `tests/wal_writer_test.rs`（5 个：含 `test_truncate_wal` L139 调用 truncate_to、`test_fsync_after_write`）

**调用方（grep 全仓确认）**

| 调用方 | 使用方法 | 形态 |
|---|---|---|
| `src/database.rs:34-37` | `WalWriter::open(path)` | `Arc<WalWriter>` 存入 `Database.wal_writer` |
| `src/wal/buffer.rs:25` | `get_current_lsn()`（do_flush L163）、`write_batch`（L175） | `Arc<WalWriter>` |
| `src/wal/checkpoint.rs:85,104` | `get_current_lsn()`、`write_record(Checkpoint)` | `Arc<WalWriter>` |
| `truncate_to` 生产调用方 | **无** —— 仅 `tests/wal_writer_test.rs:139` 使用 | — |
| 测试直接构造 | `tests/executor_test.rs` 5× `WalWriter::open(":memory:")`；checkpoint/recovery/wal_buffer 测试各自 open | `&self` 方法签名兼容 |

**LSN 双源现状**：WALBuffer 有逻辑 LSN 计数器（buffer.rs:23，从 1 起），但 `do_flush` 落盘前重算文件偏移 LSN（L162-171：base_offset = get_current_lsn() + 逐条累加序列化长度）。持久化层权威 LSN = 文件字节偏移。

**并发面**：WALBuffer.do_flush 可由 flush_loop 定时任务与 append 容量触发并发进入（互斥 buffer 取空后另一方空转）；CheckpointManager.checkpoint 的 `write_record` 可与 flush 的 `write_batch` 并发。现状两者各自开 fd 追加写——Checkpoint 记录可能插入到已计算 base_offset 的批量写入之前，造成 offset-LSN 偏移假设失效的潜在竞态窗口。

**新鲜基线（2026-08-26 @ f392c73）**：wal_writer_test 5 pass / wal_buffer_test 4 pass / pipeline_test 17 pass / dml_tx_id_test 6 pass / checkpoint_test 3 pass / recovery_e2e_test 6 pass，全绿退出码 0。

### 1.2 T04 相关事实

**`src/pipeline.rs`（908 行，全文已读）**

- `execute_inner`（L38-316，约 279 行）结构：
  1. profiling init + total_start（L39-49）
  2. cache lookup + `cache_hit_check` 计时（L52-61）
  3. **cache-hit 早退路径**（L63-97）：executor 创建 + 执行 + 计时 + return —— 与主路径重复 executor/计时逻辑
  4. parse + `parse_and_plan` 计时（L100-115）+ empty check（L117-121）
  5. `match statements.first()`：
     - `CreateTable` 臂（L127-148）：`PlanBuilder::new().build_plan` → 直接包 `CreateTableExecutor::new(plan, Arc::new(database.clone()))` → 执行 → `plan_cache.clear()` → return
     - `Drop` 臂（L151-172）：同构
     - Query/Insert/Update/Delete 臂（L175-309）：`register_table`（含 `table_metadata_lookup` 计时）→ `build_plan` → `is_cacheable→put` → DML 判定 `begin()` → prefetch abort 用 table_meta → `create_executor_from_plan(plan, db, tx_id)`（失败则 abort）→ `execute_executor` → 按 Response 是否 Error 决定 commit/abort（commit 失败/abort 失败各有专属错误消息）→ print_timings → return
  6. fallback `"No statement executed"`（L313-315）
- 已存在的独立函数：`create_executor_from_plan`（L353-611，pub(crate)，DDL 变体 panic）、`execute_executor`（L319-346）、`register_table`（L877-902）、`is_cacheable`（L906-908，仅 Query 可缓存）、表名提取辅助族（L757-875）
- 错误消息格式契约：`"Parse error: {}"`、`"Plan error: {}"`、`"Table '{}' not found: {}"`（注意：table not found 无前缀直出）、`"Empty SQL"`、`"No statement executed"`、`"Execution error: {}"`、`"Commit failed: {}"`、`"Abort failed: {}"`

**`src/profiling.rs`（65 行，全文已读）**

- task_local `PROFILING_DATA: Arc<Mutex<HashMap<&'static str, Duration>>>`；`record_time` 为 insert（同名覆盖）；scope 未设置时 `.with()` 会 panic —— 所有 record_time/print_timings 调用必须保持在 `if profiling` 守卫内
- 开关：`is_profiling_enabled()` = 环境变量 `RTSQL_PROFILING` 存在
- 输出：stderr 表格，按耗时降序

**测试与基准**

- `tests/pipeline_test.rs` 17 tests（含 T02 的 cache hit / ddl clears / dml not cached）
- `tests/dml_tx_id_test.rs` 6 tests（T01 语义见证）
- bench 模式（benches/micro_bench.rs）：criterion + `tokio::runtime::Runtime` + `b.to_async(&rt)` + `benches/common`（setup_db/create_test_table/cleanup_db）；新增 bench 需在 `Cargo.toml` 登记 `[[bench]]` 条目

## 2. T03 设计：WalWriter 单句柄复用

### 目标结构

```rust
pub struct WalWriter {
    /// 打开一次的持久句柄：create + append + read（append 隐含写权限）
    file: Arc<std::sync::Mutex<std::fs::File>>,
    /// 保留用于诊断/错误信息
    wal_path: PathBuf,
    write_count: AtomicU64,
    checkpoint_threshold: u64,
}
```

### 方法级改造（对外签名全部不变）

| 方法 | 改造后流程 | 语义要点 |
|---|---|---|
| `open` | 打开 create+append+read → 包 `Arc<Mutex<File>>` 存入字段 | 不再丢弃句柄 |
| `write_record` | clone Arc → spawn_blocking：lock → seek(End(0)) → stream_position → write_all → 返回 lsn | LSN=写入前文件末尾偏移，与现状逐字节等价 |
| `fsync` | lock → sync_all | — |
| `truncate_to` | lock → set_len(lsn) | O_APPEND 保证后续追加落在新末尾；句柄 cursor 不影响 append 写点 |
| `get_current_lsn` | lock → metadata().len() | 持锁避免读到 truncate 中间态 |
| `write_batch` | lock → 逐条 serialize_with_lsn + write_all → sync_all | 调用方传 LSN 语义不变 |

### 关键技术选择

- **std::sync::Mutex（非 tokio::sync::Mutex）**：锁内是纯同步阻塞 IO 且在 spawn_blocking 内执行，tokio Mutex 的 async lock API 在此无用武之地；std Mutex 更轻。锁持有跨度 = 单次 IO 操作，无 await 嵌套
- **spawn_blocking 保留**：M13 异步执行原则要求阻塞 IO 不占用 runtime worker；现状即如此，仅把"open+op+drop"换成"lock+op"
- **串行化效应（接受的正面变化）**：全部写路径经单锁串行，自然消除 §1.1 所述 Checkpoint write_record 与 WALBuffer write_batch 并发交错的 offset-LSN 竞态窗口。吞吐特性与现状相当（现状每操作一次 open syscall，成本高于一次 lock/unlock）
- **错误语义零变化**：IO 失败仍 `WalError::IoError(e.to_string())` 上抛；无自动重试/重开（G3 裁决）。Mutex 中毒仅在持锁 panic 时发生 → unwrap panic，与库内现有 `.unwrap()` 风格一致（如原 plan_cache、flush_handle）
- **被否决的替代方案**：(a) `try_clone()` 每 操作克隆 fd —— 仍是多 fd，违背目标；(b) 专用 writer task + mpsc channel —— 架构级改动，超出稳定性收口边界（MS06 non-goal）；(c) tokio Mutex —— 见上

### 兼容性

- 公开方法签名不变（`&self` async）；`Send + Sync` 保持（`Arc<Mutex<File>>` + AtomicU64）
- `WalWriter::open(":memory:")` 测试用法不受影响（`:memory:.wal` 文件照常创建）
- WAL on-disk 格式、RecoveryManager 读路径零接触

### 测试见证

新增 `tests/wal_handle_test.rs`：

1. `test_fd_bound_under_10k_tx`：tempdir Database::open → CREATE TABLE → 10K 次 execute_sql(INSERT) 全程 `/proc/self/fd` 条目计数相对压测前基线的净增量 < 10（G2 裁决；Linux x86_64 是唯一支持平台）
2. `test_write_record_lsn_equals_file_offset`：顺序写多条，每条返回 LSN == 写前文件长度（读文件 metadata 对照），LSN 严格递增且首条 == 0
3. `test_truncate_then_append_same_handle`：写 n 条 → truncate_to(mid) → 再写 → 新记录落在截断后末尾，get_current_lsn 反映真实长度
4. `test_concurrent_writers_recovery_consistent`：N 任务并发共享 `Arc<WalWriter>` 各写 M 条 → close → RecoveryManager/WalReader 可完整解析无 CRC 错误

回归见证（必须全绿、不得修改）：wal_writer_test(5)、wal_buffer_test(4)、checkpoint_test(3)、recovery_test、recovery_e2e_test(6)、executor_test（其 5 处 setup 不动）。

## 3. T04 设计：parse/plan/execute 三阶段拆分

### 目标结构

`execute_inner` 从 279 行单函数变为编排器；三个新 pub 阶段函数（pub 是验收"独立 micro-bench"的硬性要求——bench 是外部 crate，须经 `rtsql::` 访问）：

```rust
/// 阶段 1：文本 → AST。Err 携带既有 "Parse error: {}" / "Empty SQL" 消息。
pub async fn parse_stage(sql: &str) -> Result<Vec<Statement>, String>

/// 阶段 2：AST → PhysicalPlan。DDL 与 DML/查询统一产出 plan。
/// 内部：DDL 走 PlanBuilder::new().build_plan；其余走 register_table + build_plan
/// + is_cacheable→put。Err 携带既有 "Plan error: {}" / "Table '{}' not found: {}" 消息。
pub async fn plan_stage(database: &Database, sql: &str, stmt: &Statement)
    -> Result<PhysicalPlan, String>

/// 阶段 3：plan 执行。按 plan 变体路由：
/// - CreateTable/DropTable 变体 → 直包 DDL Executor，成功后 plan_cache.clear()
/// - Insert/Update/Delete 变体 → begin → prefetch abort meta → create_executor(tx_id)
///   → 执行 → commit/abort（含 Commit failed/Abort failed 专属消息）
/// - 其余 → create_executor(None) → 执行
/// 缓存命中的 plan 也从此入口执行（缓存内容恒为 SELECT plan）。
pub async fn execute_stage(database: &Database, plan: PhysicalPlan) -> Response
```

编排器职责（留在 execute_inner）：profiling init/total_start、cache lookup（`cache_hit_check` 子指标）、命中则跳过 parse/plan 直接 execute_stage、未命中走三阶段、各终止点 print_timings。

### 关键技术选择

- **DDL 归一化进 plan_stage**：现状 DDL 臂与查询臂的 build_plan 逻辑重复两份；统一后 stage_execute 以 `PhysicalPlan::CreateTable(_) | DropTable(_)` 变体判定路由（`create_executor_from_plan` 对 DDL 本就 panic，路由天然安全），无需额外枚举。**cache.clear() 时序保持**：仍在 DDL 执行成功之后（plan_stage 不做失效，否则 DDL 失败也会清缓存——行为变化，禁止）
- **profiling 三段顶层计时**：编排器在各 stage 调用前后 Instant 计时，`record_time("parse"/"plan"/"execute", ...)`（G4 裁决：允许废弃旧名 `parse_and_plan`）；子指标保留能力 = `profiling: bool` 参数传入 stage 函数内部守卫记录（如 `table_metadata_lookup` 在 plan_stage 内、`executor_creation`/`executor_execution` 在 execute_stage 内）。所有 record/print 调用严格处于 profiling 守卫下（task_local scope 未设置时 .with() 会 panic —— §1.2 已确认的约束）
- **错误载体选 String**：stage 函数返回 `Result<T, String>`（消息即最终 Response 文本），编排器统一包成 `Response::Error`。避免引入新错误类型扩大接口面
- **可见性 pub 的代价**：crate 公开 API 增加 3 个函数。接受理由：独立 bench 为验收硬性条款；函数带 doc 注明属于管道观测入口。替代方案（bench 经 feature-gate 或 #[doc(hidden)]）否决——增加复杂度无收益
- **cache-hit 路径复用**：命中缓存的 SELECT plan 直接进 execute_stage，删除现 L63-97 重复块

### 测试见证

1. `src/pipeline.rs` 内新增 `#[cfg(test)] mod tests`（沿用 plan_cache.rs 单测先例）：
   - parse_stage：合法 SQL 产语句；非法 SQL Err 含 "Parse error:"；空串 → "Empty SQL"
   - plan_stage：对已建表 SELECT 产出 Scan/DataScan 类计划；对不存在表 Err 含 "not found"；INSERT 语句产出 Insert plan 且写入 cache 后 `plan_cache_len()==1`（DML 不缓存断言反向：put 仅当 is_cacheable）
   - execute_stage：手工构造 ValueScan/简单 plan → 正确 Response；DDL plan → 执行后 cache 清空
2. 回归见证（不得修改）：pipeline_test(17)、dml_tx_id_test(6)、plan_cache_test、e2e_test、subquery/join/aggregate 等 executor 族
3. 新增 `benches/pipeline_stages_bench.rs` + Cargo.toml `[[bench]]` 登记项：三组 benchmark 分别调 parse_stage / plan_stage（每轮清 cache 或换 key 防止 hit）/ execute_stage（预热后跑预构建 plan），criterion + tokio Runtime + common helpers 模式

## 4. Invariants（两个 task 共同遵守）

- MS06 non-goals：不做任何性能优化、不加新 SQL 方言/执行器/隔离级别
- WAL on-disk 格式与恢复语义零变化
- `Database::open` / `execute_sql` 公开 API 签名零变化
- DML 必须运行在真实事务内（MS06-T01 spec `dml-transaction-lifecycle` 约束）
- 错误 Response 文本格式逐一保持（§1.2 列表）
- 现有 504 tests pass 基线不回退

## 5. Risks and Notes（非实质未知项）

- fd 上界测试运行时长未知（10K INSERT 含 group commit，预估秒级~十秒级）；Act 若实测 >60s 可报告并在 Response 建议，但不得擅自降额为 <10K（验收口径固定）
- `print_timings` 输出名变化（`parse_and_plan` → `parse`/`plan`）影响人工日志阅读习惯；仓库内无程序化消费者（grep 未见解析该输出的代码）
- criterion bench 的统计噪声不影响 PASS 判定——验收只要求可运行产出数据，无数值阈值（性能优化是 non-goal）
- `/proc/self/fd` 为 Linux 特有；项目声明支持平台即 Linux x86_64（SNAPSHOT），无需 cfg 门控
