# 任务与里程碑

> 最后更新：2026-06-03（M38 完成 — Phase 1 全部完成）

## 当前阶段：全维度性能优化 + 功能完善 + 并发控制

### 执行顺序（5 个 Phase，Phase 内可并行）

```
Phase 1 基础设施（改动小、风险低、后续都依赖）
  M41 → M30 → M38  ✅ 全部完成

Phase 2 存储引擎核心（读写路径重构）
  M20 → M19 → M21
  M20 → M36

Phase 3 并发控制（存储引擎稳定后才改锁）
  M31 → M40
  M34 → M32 → M42
  M48（独立）

Phase 4 上层功能（独立，可与 P2/P3 并行）
  M24 | M25 → M26 | M27 → M28 | M29 | M37 | M39 | M44

Phase 5 高级优化（依赖前面完成）
  M19+M31 → M22
  M23 → M33
  M31 → M35
  M43 | M45
```

### 依赖关系图

```
M41 ──→ M40        M20 ──→ M19 ──→ M21 ──→ M22(P5)
  └──→ M48         M20 ──→ M36 ──→ M37(P4)
M30（独立）         M34 ──→ M32 ──→ M42
M38 ──→ M29        M25 ──→ M26
M31 ──→ M35(P5)    M27 ──→ M28
  └──→ M22(P5)     M20 ──→ M39
M23 ──→ M33(P5)
```

---

## 优化路线图

| Phase | M | 优化项 | 预期收益 | 风险 |
|-------|---|--------|---------|------|
| **P1** | **M41** | 事务 ID AtomicU64 | 分配延迟 100ns→10ns | 低 |
| | **M30** | 连接并发 Semaphore | 防连接风暴 | 低 |
| | **M38** | BufWriter + TCP_NODELAY | write 调用 -99% | 低 |
| **P2** | **M20** | 零拷贝 SlottedPageRef | I/O ~20-30% 提速 | 低 |
| | **M19** | DataScan 路径 | 全表扫描 ~2x | 中 |
| | **M21** | 页面级 MVCC | ~10-15% 提速 | 中 |
| | **M36** | 零拷贝 ValueRef（✅ 2026-06-03, L025）| 堆分配 30万→0 | 中 |
| **P3** | **M31** | BufferPool DashMap+Semaphore | 并发读吞吐提升 | 低 |
| | **M40** | RowLockTable DashMap | 行锁争抢 -5-10x | 低 |
| | **M34** | WAL fsync 合并 | TPS 3-10x | 低 |
| | **M32** | WAL 写入背压 | 缓冲区限流 | 低 |
| | **M42** | 消息传递重构 | WAL 锁消除 | 中 |
| | **M48** | pread/pwrite 替代 seek+read | syscall -50% | 低 |
| **P4** | **M24** | 多隔离级别 | SQL 标准支持 | 高 |
| | **M25** | 多 Join 算法 | 小表 join 提速 | 中 |
| | **M26** | 代价模型+Join 重排 | 最优执行计划 | 中 |
| | **M27** | 关联子查询缓存 | 重复参数免执行 | 中 |
| | **M28** | 多层关联子查询 | 嵌套查询支持 | 低 |
| | **M29** | PG Extended Query | 预编译+二进制传输 | 中 |
| | **M37** | clone 消除 Arc/Cow | clone 开销 -90% | 中 |
| | **M39** | INSERT 批量执行 | 1000行 5-10x | 中 |
| | **M44** | 表定义持久化 | 重启不丢 schema | 中 |
| **P5** | **M22** | 预取 Prefetch | 大表 ~15-25% | 低 |
| | **M23** | Varint Key | 索引空间 ~70% 缩减 | 中 |
| | **M33** | B+Tree 节点级锁 | 并发索引访问 | 高 |
| | **M35** | 脏页 writev | Checkpoint 5-10x | 低 |
| | **M43** | 并行扫描 | 多核扫描提速 | 中 |
| | **M45** | io_uring | I/O 延迟 -30-50% | 高 |

---

## 已完成（M1-M18）

| 里程碑 | 内容 | 日期 |
|--------|------|------|
| M1-M6 | 项目初始化+SQL 解析+执行引擎+存储+事务+索引 | 2026-03 |
| M7-M12 | WAL+MVCC+Hash Join+子查询+B+Tree Split/Merge+PG 协议 | 2026-04 |
| M13-M18 | 聚合+关联子查询+B+Tree Redistribute+并发+优化器+基准测试 | 2026-05 |

---

## 详细规划

### Phase 1: 基础设施

#### M41: 事务 ID AtomicU64 无锁分配 ✅ 已完成 (2026-06-03)

- **问题**：`next_tx_id()` 用 `Mutex<u64>`，每次事务开始等锁
- **任务**：
  - [x] T1: `TransactionId::counter` 改为 `AtomicU64`（main 已有：tx_id.rs:4）
  - [x] T2: `next_tx_id()` 改用 `fetch_add(1, SeqCst)`（main 已有：tx_id.rs:15）
  - [x] T3: 事务开始时间戳同理改为 `AtomicU64`（begin() 复用 allocate，tx_id 即时间戳）
  - [x] T4: 微基准测试（Mutex vs AtomicU64）→ `benches/tx_id_bench.rs` 4 场景
- **结果**：单线程 5.1 ns/op（2.1x），10 线程 18.6 ns/op（4.6x），100 线程 22.5 ns/op（4.5x）
- **Commits**：`634764d` (feat) + `ee9ceee` (chore archive) — 已在 origin/master
- **ADR**：`architecture/spec.md` ADR-009
- **下一步**：Phase 1 可启动 M30（连接并发）+ M38（网络 BufWriter）

---

#### M30: 连接并发上限 ✅ 已完成 (2026-06-03)

- **问题**：PG 连接无限 `tokio::spawn`，连接风暴压垮系统
- **任务**：
  - [x] T1: `Server` 新增 `Arc<Semaphore>` 字段，配置 `max_connections`（默认 64）
  - [x] T2: accept 循环 `acquire_owned().await` 后再 spawn
  - [x] T3: 连接结束 `drop(permit)` 释放
  - [x] T4: 并发连接压测 + 超限测试
- **结果**：3 个连接限制测试全部通过（within-limit / over-limit queued / permit-release），全量回归 0 失败
- **改动**：`server.rs` +8 行（Semaphore）、`connection.rs` match→if let（startup 后保持存活）、`main.rs` / `pg_integration_test.rs` 更新签名、`connection_limit_test.rs` 新增 201 行
- **下一步**：Phase 1 可启动 M38（网络 BufWriter + TCP_NODELAY）

---

#### M38: 网络 BufWriter + TCP_NODELAY ✅ 已完成 (2026-06-03)

- **问题**：DataRow 逐行 `write_all()` + `flush()`，每行一次 syscall
- **任务**：
  - [x] T1: PgProtocol 新增 `write_buf: Vec<u8>`（8KB 缓冲），`send_startup_response` 和 `write_response` 均累积消息后单次 `write_all`+`flush`
  - [x] T2: `TCP_NODELAY` 在 `server.rs` accept 后立即 `stream.set_nodelay(true)`
  - [x] T3: SELECT 结果所有消息（RowDescription + N×DataRow + CommandComplete + ReadyForQuery）累积到 `write_buf`，一次 `write_all` + `flush`
  - [x] T4: 新增 2 个测试：100 行大结果批写入（超 8KB 自扩容）、4 批次缓冲复用验证
- **改动**：`server.rs` +3 行（set_nodelay）、`pg_protocol.rs` 重构 write 路径（+write_buf 字段，send_startup_response + write_response 全部批量化）、`pg_protocol_test.rs` +110 行（2 新测试）
- **结果**：全量测试通过（pg_protocol 9→11 tests, 0 失败），查询响应从 N+1 次 syscall 降为 2 次（1 write + 1 flush）
- **下一步**：Phase 1 全部完成！可启动 Phase 2（M20 零拷贝 或 M19 DataScan）

---

### Phase 2: 存储引擎核心

#### M20: 零拷贝 SlottedPageRef ✅ 已完成 (2026-06-03)

- **问题**：`read_tuple_from_data_page` / `find_visible_version` 返回 `Vec<u8>`，每行一次堆分配
- **方案**：纯闭包 API（用户已选择方案 A）
  - 新增 `BufferPool::with_page_data<F, R>(&self, PageId, F) -> Result<R>` — 闭包内零拷贝 `&[u8]`
  - `read_tuple_from_data_page` 改为 `async fn<F, R>(buffer_pool, row_id, f: F) -> Result<R>` — 闭包接收 `(VersionHeader, &[u8])`
  - `find_visible_version` 改为 `async fn<F, R>(&self, row_id, snapshot, f: F) -> Result<Option<R>>` — 闭包 `FnOnce(&[u8]) -> Result<R>`（修订：原 design 误，错误传播必须 Result<R>）
  - 引入 `VisibilityResult<R>` 辅助枚举
  - 删除编译不过的 `get_page_ref`
- **状态**：
  - [x] T1: 验证既有零拷贝类型（SlottedPageRef / LeafNodeRef / InternalNodeRef / PageDataGuard）
  - [x] T2: `with_page_data` 闭包 API 实现 + `VisibilityResult<R>` + 删除 `get_page_ref`
  - [x] T3: `read_tuple_from_data_page` 闭包形式（+ data_page.rs 5 单元测试迁移）
  - [x] T4: `find_visible_version` 闭包形式（+ design.md 决策 3 修订 F 返回 Result<R>）
  - [x] T5: `read_version_header` / `write_commit_tx_id` 闭包适配
  - [x] T6: 3 个 Scan 执行器闭包调用（ScanExecutor / IndexScanExecutor / IndexScanAllExecutor）
  - [x] T7: UpdateExecutor 闭包内 `.to_vec()` 适配
  - [x] T8: cargo test 0 失败 + cargo fmt 12 文件 + cargo clippy M20 范围内 0 warning
  - [x] T9: 性能验证（before-m20 baseline 留底 + 对比）
  - [x] T10: 归档
- **性能对比**（micro_bench，详见 learned/spec.md L024）：
  - delete/by_pk: **-8.33%** ✅
  - filter/where_value_gt_500: **-3.53%** ✅
  - sort/order_by_value_desc: **-4.56%** ✅
  - join/inner_join: **-2.46%** ✅
  - scan/full_table: +2.04%（噪声内）
  - limit/limit_10_offset_5: -0.60%（噪声内）
  - update/single_column: **+3.99%** ⚠️ 写路径回归（在 5% 阈值内）
- **vs 目标**：
  - ≥ 15% 提速 — ❌ 未达（实际 -2.46% 到 -8.33%）
  - 回归 < 5% — ✅ 通过
  - 原因：micro_bench 行数小（1K × 100B），现代分配器对 100KB Vec 已经极快；M19/M36 进一步消除分配可能达 15%+
- **改动文件**（12 个）：
  - `src/storage/buffer_pool.rs` (+98 -32) — `with_page_data` + `find_visible_version` 闭包形式
  - `src/storage/data_page.rs` (+24 -19) — `read_tuple_from_data_page` 闭包形式 + 5 单元测试
  - `src/storage/page_frame.rs` (+6 -0) — PageDataGuard 文档补充
  - `src/executor/scan.rs` (+17 -22) — ScanExecutor 闭包调用
  - `src/executor/index_scan.rs` (+20 -19) — IndexScanExecutor 闭包调用
  - `src/executor/index_scan_all.rs` (+17 -19) — IndexScanAllExecutor 闭包调用
  - `src/executor/update.rs` (+6 -2) — UpdateExecutor 闭包内 .to_vec()
  - `tests/storage_test.rs` (+33 -10) — 2 with_page_data 测试
  - `tests/executor_test.rs` (+23 -10) — 6 read_tuple_from_data_page 闭包适配
  - `tests/gc_test.rs` (+8 -3) — 2 read_tuple_from_data_page 闭包适配
  - `tests/version_chain_test.rs` (+13 -5) — 6 find_visible_version 闭包适配
  - `tests/mvcc_commit_test.rs` (+6 -3) — 3 find_visible_version 闭包适配
- **验证**：cargo test --lib --tests 0 失败（110 lib + 全部集成测试 ok）
- **下一步**：Phase 2 启动 M19 (DataScan) — ✅ 已完成（2026-06-04）；M21 (页面级 MVCC) 待启动

#### M36: 零拷贝 ValueRef ✅ 已完成 (2026-06-03)

- **问题**：`deserialize_tuple` 每次 String 列做 `to_vec()` 堆分配，1K 行 × 300B/行 = 30万次分配
- **方案**：
  - 新增 `ValueRef<'a>` 零拷贝枚举（`src/executor/value_ref.rs`，含 9 个方法 + 10 测试）
  - 新增 `deserialize_value_refs` 借用 `&'a [u8]` 零 String 分配（`src/storage/page_format/tuple.rs`）
  - `Value::as_value_ref` owned-to-borrowed 视图
  - `Expression` trait 新增 `evaluate_ref<'a>` 抽象方法；`evaluate` 改为 trait 默认方法内部转调 `evaluate_ref().to_value()`
  - 3 个 Expression 实现补 `evaluate_ref`（ColumnExpression / ConstantExpression / ParameterExpression）
  - 3 个 Scan 执行器闭包改用 `deserialize_value_refs` + `.to_value()`
- **范围严格**：M36 不改 `Value` 枚举（M37 范围）、不改 `UpdateExecutor`/Sort/Aggregate/Join（写路径反正要 to_value）
- **验收**：详见 learned/spec.md L025（双标准：30万→0 AND ≥ 5%）
- **改动文件**：6 个（新建 value_ref.rs + 改 value.rs / tuple.rs / predicate.rs / 3 个 Scan + 2 个 mod.rs + 集成测试）
- **Commits**：ed81610 (T1) / 4f9a8e8 (T2) / 03c2deb (T3) / 3ce2672 (T4) / 9bc8d28 (T5) / 75199d6 (T6) / b75d307 (T8) / 95bb3f9 (T9+T10 docs) / 73076ac (T10 archive + push)
- **下一步**：Phase 2 启动 M19 (DataScan) — ✅ 已完成（2026-06-04）；M21 (页面级 MVCC) 待启动

---

#### M19: DataScan 路径 ✅ 已完成 (2026-06-04, L026)

- **问题**：Index→RowId→Data 每行两次页访问，全表扫描落后 SQLite ~4x
- **任务**：
  - [x] T1: `DataScanExecutor` 顺序扫描数据页，跳过索引层
    - 新增 `src/executor/data_scan.rs`（~155 行）
    - 流式 `next()` 沿 `data_page_head` → `next_page_id` 链表遍历
    - 无 `Vec<Vec<Value>>` 预加载，每行 1 次页访问
    - 单元测试 4 个：单数据页 / 空表 / 多页 / 流式顺序
  - [x] T2: MVCC 可见性检查（在 T1 基础上加入）
    - `with_page_data` 闭包内解析 VersionHeader（22B）
    - 不可见时 `find_visible_in_chain` 异步跨页查链（MAX 64 深度保护）
    - 单元测试 2 个：未提交不可见 / 已提交可见
  - [x] T3: 无 WHERE 条件时 Planner 自动选 DataScan
    - `PhysicalPlan::DataScan(DataScanNode)` 枚举变体
    - `planner.rs::build_query` 无 WHERE 分支返回 DataScan
    - `pipeline.rs::create_executor_from_plan` + `extract_column_indices` + `correlated.rs` + `aggregate input_schema` + `get_subquery_first_column` 全部支持 DataScan
    - 集成测试：`test_planner_no_where_routes_to_data_scan` + `test_planner_with_pk_equality_keeps_index_scan`
  - [x] T4: 有 WHERE 但无索引覆盖时也走 DataScan + 过滤
    - 新增 `has_pk_equality` 递归检查 AND 组合（含 PK → 保持 `Filter(Scan)` 兜底；不含 → `Filter(DataScan)`）
    - 集成测试：`test_planner_non_pk_where_routes_to_filter_data_scan` + `test_planner_pk_and_other_keeps_filter_scan`
  - [x] T5: 全表扫描基准测试对比
    - 新增 `benches/data_scan_bench.rs`（4 场景，1K/10K 行）
    - **实测：1K 1.81x / 10K 2.44x 提速**（达到预期 ~2x 目标）
    - 走 OpenSpec change：`m19-datascan-path`（已归档为 `2026-06-04-m19-datascan-path`）

- **改动文件**（共 8 个）：
  - `src/executor/data_scan.rs` (新增 ~155 行) — DataScanExecutor + PageAction 状态机
  - `src/executor/plan.rs` (+24 行) — DataScan 变体 + DataScanNode
  - `src/executor/mod.rs` (+2 行) — 导出
  - `src/pipeline.rs` (+12 行) — dispatch + extract_column_indices
  - `src/parser/planner.rs` (+60 行) — build_query 路由 + has_pk_equality + get_subquery_first_column
  - `tests/executor_test.rs` (+200 行) — 8 个 M19 测试
  - `tests/planner_test.rs` (2 行) — test_select_scan 期望 DataScan
  - `benches/data_scan_bench.rs` (新增 ~80 行) — criterion 对比
  - `Cargo.toml` (+3 行) — bench 入口

- **验证**：
  - 全量回归 464/464 测试通过（含 8 M19 测试）
  - cargo fmt 0 diff
  - cargo clippy 无新 warning
  - criterion bench 输出：1K 1.81x / 10K 2.44x

- **下一步**：Phase 2 启动 M21 (页面级 MVCC) — ✅ 已完成（2026-06-04）；惰性设置 + 基准测试延后；下一步启动 M37 或 M31

---

#### M21: 页面级 MVCC ✅ 已完成 (2026-06-04)

- **问题**：每行 22B VersionHeader，逐行检查可见性
- **方案**：`PageVisibilityInfo`（9B/page 内存摘要）+ `DashMap` 快速路径，写路径自动清标志
- **任务**：
  - [x] T1: `PageVisibilityInfo` 结构体（`src/storage/page_visibility.rs`）+ BufferPool 集成（`DashMap` + 4 公开方法）+ 4 单元测试
  - [x] T2: `find_visible_version` + `DataScanExecutor` 页面级快速路径（`all_visible` 跳过逐行检查 / `all_invisible_for` 跳过整页）
  - [x] T3: INSERT/DELETE/UPDATE/COMMIT 四路径均调用 `clear_all_visible` 更新摘要
  - [ ] T4: 可见性检查基准测试 — ⏸️ 延后（需先实现惰性 `all_visible` 设置）
- **改动**（~10 文件）：`Cargo.toml` (dashmap), `page_visibility.rs` (新), `buffer_pool.rs`, `data_scan.rs`, `insert.rs`, `data_page.rs`, `update.rs`, `manager.rs`, `visibility_test.rs` (新 5 测试)
- **验证**：129 lib + 全量集成测试 0 failures, clippy 仅 2 pre-existing warnings
- **设计决策**：ADR-011（`DashMap` 纯内存 / 惰性设置延后 / COMMIT 路径清标志）
- **下一步**：Phase 2 启动 M21 惰性设置（T2.3）+ 基准测试（T4），然后 M37 或 M31

---

#### M36: 零拷贝 ValueRef

> ✅ **已迁移到上方 line 188 "M36: 零拷贝 ValueRef ✅ 已完成" 段。**
> 本段为 brainstorming 阶段占位，现已过时，删除。

---

### Phase 3: 并发控制

#### M31: BufferPool DashMap + Semaphore

- **问题**：`Arc<Mutex<HashMap>>` 读写都互斥
- **任务**：
  - [ ] T1: `pages` 改为 `DashMap<PageId, Frame>`（分片无锁读）
  - [ ] T2: 引入 `Semaphore(BUFFER_POOL_SIZE)` 限制 pin 页数
  - [ ] T3: PageGuard acquire 语义适配
  - [ ] T4: 并发读写基准测试

---

#### M40: RowLockTable DashMap

- **问题**：`Arc<Mutex<HashMap>>` 行锁获取/释放串行化
- **任务**：
  - [ ] T1: 引入 `dashmap` crate
  - [ ] T2: `locks` 改为 `DashMap<RowId, Arc<Mutex<()>>>`
  - [ ] T3: `get_lock()` 改用 `DashMap::entry()` API
  - [ ] T4: 并发锁获取基准测试

---

#### M34: WAL fsync 合并

- **问题**：每事务提交单独 fsync，系统调用开销巨大
- **任务**：
  - [ ] T1: `tokio::time::interval` 定时器，与 commit 触发并存
  - [ ] T2: 刷盘窗口累积多条记录，一次 `write_all` + 一次 `fsync`
  - [ ] T3: 可配置 `wal_writer_delay`（默认 2ms）
  - [ ] T4: TPS 基准测试对比

---

#### M32: WAL 写入背压

- **问题**：WAL 无背压，高并发缓冲区膨胀
- **任务**：
  - [ ] T1: `Semaphore(WAL_MAX_PENDING)` 限制等待刷盘事务数
  - [ ] T2: `append_commit_and_wait()` 中 acquire permit
  - [ ] T3: `do_flush()` 完成后释放 permit
  - [ ] T4: 高并发写入压测

---

#### M42: 消息传递重构

- **问题**：多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**：WAL Writer→mpsc，提交等待→oneshot，Checkpoint→Notify，BufferPool→watch
- **任务**：
  - [ ] T1: WALBuffer `buffer: Mutex<Vec>` → `mpsc::channel`
  - [ ] T2: `commit_waiters: Mutex<HashMap>` → `oneshot::channel`
  - [ ] T3: Checkpoint 触发改用 `Notify`
  - [ ] T4: BufferPool eviction 改用 `watch::channel`
  - [ ] T5: 集成测试 + 性能对比

---

#### M48: pread/pwrite 替代 seek+read

- **问题**：文件读写用 `seek()+read()/write()` 两次 syscall，且非原子
- **任务**：
  - [ ] T1: `FileExt::read_at()` / `write_at()` 改用 `pread`/`pwrite`（单次 syscall）
  - [ ] T2: 验证 `tokio::fs::File` 已支持 `read_at`/`write_at`（底层即 pread/pwrite）
  - [ ] T3: 去除 `seek` 调用，所有文件操作基于 offset 参数
  - [ ] T4: syscall 计数对比（strace 验证）

---

### Phase 4: 上层功能

#### M24: 多隔离级别

- **问题**：只有 Repeatable Read
- **方案**：Read Committed（每语句刷新 snapshot）+ Serializable（SSI + 写偏序检测）
- **任务**：
  - [ ] T1: `IsolationLevel` 枚举
  - [ ] T2: `BEGIN TRANSACTION ISOLATION LEVEL ...` 语法
  - [ ] T3: Read Committed 实现
  - [ ] T4: Serializable SSI 实现
  - [ ] T5: ANSI SQL 隔离级别测试

---

#### M25: 多 Join 算法

- **问题**：只有 Hash Join
- **任务**：
  - [ ] T1: `JoinAlgorithm` 枚举 + PhysicalPlan 扩展
  - [ ] T2: `NestedLoopJoinExecutor` 实现
  - [ ] T3: `SortMergeJoinExecutor` 实现
  - [ ] T4: 启发式算法选择（小表 NLJ，有序 SMJ，默认 HJ）
  - [ ] T5: Join 算法基准测试

---

#### M26: 代价模型 + Join 重排

- **问题**：固定 join 顺序，无 cardinality/selectivity
- **任务**：
  - [ ] T1: `TableStatistics` 结构（行数/NDV/min/max/null_count）
  - [ ] T2: `ANALYZE TABLE` 命令 + 持久化
  - [ ] T3: `CostEstimator`（scan/join/filter 代价）
  - [ ] T4: Join 重排序（DP <10 表，贪心 ≥10）
  - [ ] T5: 代价驱动执行计划测试

---

#### M27: 关联子查询缓存

- **问题**：每行外层重新执行子查询
- **任务**：
  - [ ] T1: `SubqueryCache` 结构（参数值→结果集）
  - [ ] T2: `SubqueryEvalExecutor` 集成缓存
  - [ ] T3: LRU 淘汰 / 事务结束清空
  - [ ] T4: 关联子查询性能基准测试

---

#### M28: 多层关联子查询

- **问题**：显式拒绝多层嵌套
- **任务**：
  - [ ] T1: `extract_correlated_params` 改为递归遍历
  - [ ] T2: `inject_correlated_values` 支持多层注入
  - [ ] T3: 移除拒绝逻辑
  - [ ] T4: 多层嵌套测试用例

---

#### M29: PG Extended Query Protocol

- **问题**：只有 Simple Query
- **任务**：
  - [ ] T1: Parse/Bind/Describe/Execute 消息解析与序列化
  - [ ] T2: Prepared Statement 缓存与生命周期
  - [ ] T3: 二进制格式 DataRow 编码
  - [ ] T4: Close/Sync/Flush 消息
  - [ ] T5: psql 预编译语句集成测试

---

#### M37: clone 消除 Arc/Cow

- **问题**：`Value::clone()` 在聚合/排序/JOIN 中反复调用
- **任务**：
  - [ ] T1: `Value::Text` 内部 `Arc<str>` 替代 `String`
  - [ ] T2: `Value::Bytes` 内部 `bytes::Bytes` 替代 `Vec<u8>`
  - [ ] T3: SQL 解析阶段 `Cow<'_, str>` 延迟分配
  - [ ] T4: clone 计数基准测试

---

#### M39: INSERT 批量执行

- **问题**：多值 INSERT 逐行执行
- **任务**：
  - [ ] T1: `InsertExecutor` 检测多值 INSERT，收集后批量执行
  - [ ] T2: B+Tree `bulk_insert(keys: &[Key])`
  - [ ] T3: WAL `append_batch(records: &[WalRecord])`
  - [ ] T4: 批量 INSERT 基准测试

---

#### M44: 表定义持久化

- **问题**：`TableManager` 纯内存，重启丢失
- **任务**：
  - [ ] T1: Schema Page 设计（系统表 `__tables` / `__columns`）
  - [ ] T2: `CREATE TABLE` 时写入系统表
  - [ ] T3: 启动时从系统表加载 schema
  - [ ] T4: `DROP TABLE` / `ALTER TABLE` 同步更新
  - [ ] T5: 重启持久化测试

---

### Phase 5: 高级优化

#### M22: 预取 Prefetch

- **问题**：顺序扫描逐页读，I/O 延迟未重叠
- **任务**：
  - [ ] T1: `Prefetcher` 双缓冲结构（当前页+预取页交替）
  - [ ] T2: DataScan 中 `spawn` 异步预取下一页
  - [ ] T3: 预取深度可配置（默认 2 页）
  - [ ] T4: 大表扫描基准测试

---

#### M23: Varint Key 编码

- **问题**：固定 32B Key，INT PK 浪费 ~28B
- **任务**：
  - [ ] T1: `Key` 内部 `Vec<u8>` 变长编码（Varint prefix + payload）
  - [ ] T2: B+Tree 比较逻辑适配变长 Key
  - [ ] T3: 插入时动态计算节点 split 点（基于实际字节占用）
  - [ ] T4: 索引空间基准测试

---

#### M33: B+Tree 节点级锁

- **问题**：每次操作 lock 整棵树
- **方案**：短期 Semaphore 限流 + 长期 latch coupling（crabbing protocol）
- **任务**：
  - [ ] T1: `Semaphore(MAX_TREE_OPS)` 限流
  - [ ] T2: 节点级 `RwLock` per node
  - [ ] T3: crabbing protocol（读：自顶向下释放父锁；写：持有全部路径锁到叶）
  - [ ] T4: 死锁检测 / 无死锁验证
  - [ ] T5: 并发索引访问基准测试

---

#### M35: 脏页 writev 批量写回

- **问题**：逐页 `write_at()` + 单独 `fsync()`
- **任务**：
  - [ ] T1: `flush_all()` 先收集脏页列表再批量写
  - [ ] T2: 连续 page_id 合并为单次 `write_at`
  - [ ] T3: 非连续脏页 `writev()`
  - [ ] T4: Checkpoint 性能基准测试

---

#### M43: 并行扫描

- **问题**：全表扫描单线程
- **任务**：
  - [ ] T1: 按页范围分区，每个分区 spawn 一个 scan task
  - [ ] T2: `mpsc` channel 汇聚分区结果
  - [ ] T3: 并行度可配置（默认 = CPU 核数）
  - [ ] T4: 聚合/排序场景的并行归约
  - [ ] T5: 大表扫描基准测试

---

#### M45: io_uring 批量提交

- **问题**：`tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
- **任务**：
  - [ ] T1: 引入 `tokio-uring` 或 `io-uring` crate
  - [ ] T2: `BufferPool` 读写改用 io_uring 提交
  - [ ] T3: WAL 刷盘改用 io_uring
  - [ ] T4: 批量提交（IOSQE_IO_LINK）合并 fsync
  - [ ] T5: I/O 延迟基准测试

---

### 长期方向（未规划具体里程碑）

| 方向 | 说明 | 难度 |
|------|------|------|
| M46 瘦内部节点 | B+Tree 内部节点只存 separator keys，不存完整 Key | 高，需重构 B+Tree 分裂/合并逻辑 |
| M47 合并 Tag byte | Slot Tag byte 合并进 VersionHeader，省 1 byte/slot | 低，但影响序列化格式兼容性 |
