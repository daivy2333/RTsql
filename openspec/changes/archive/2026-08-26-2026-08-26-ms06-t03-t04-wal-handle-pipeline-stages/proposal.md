# Proposal：MS06-T03 + MS06-T04（WAL 句柄复用 + Pipeline 三阶段拆分）

> 关联里程碑：MS06 稳定性与正确性收口（剩余 T03、T04 两项，合并为一个 change）
> 关联 tasks 权威记录：`.claude/docs/tasks.md` MS06 节
> 用户裁决记录：2026-08-26 Gate 1 缺口集中决策，5 项全部按推荐默认确认（见文末）

## Why

MS06 的 4 类被掩盖稳定性问题中，T01（DML `tx_id=0`）、T02（PlanCache 锁）已完成归档；本 change 收口剩余两项：

**T03 — WAL 每写 open/close 文件句柄**

`src/wal/writer.rs` 中 `WalWriter` 只保存 `wal_path: PathBuf`，全部 5 个 IO 方法每次调用都重新 `OpenOptions::open` 再丢弃句柄：

| 方法 | 现状 | 问题 |
|---|---|---|
| `write_record`（L40-68） | 每次 spawn_blocking 内 open(append) → seek(End) → write → 丢句柄 | 每条记录一次 open syscall；高吞吐下 fd churn |
| `fsync`（L71-86） | 每次 open(write) → sync_all → 丢句柄 | 同上 |
| `truncate_to`（L89-105） | 每次 open(write) → set_len → 丢句柄 | 同上 |
| `get_current_lsn`（L129-147） | 每次 open(read) → metadata().len() → 丢句柄 | 同上 |
| `write_batch`（L153-179） | 每次 open(append) → 批量写 → sync_all → 丢句柄 | 同上 |

风险：任何异常路径下句柄关闭时序不可控；持续写入场景 fd 反复创建销毁。任务书验收：10K tx 压测句柄数 < 10。

附带发现的不一致：`write_record` 以文件末尾偏移为 LSN，而 `write_batch` 由调用方（WALBuffer）传入 LSN——两种 LSN 来源并存。用户裁决保持文件位置语义不变。

**T04 — `pipeline::execute_inner` 单函数不可观测**

`src/pipeline.rs:38` 起的 `execute_inner` 是约 200+ 行单函数：

- cache-hit 早退路径与 cache-miss 主路径重复 executor 创建 + 运行 + 计时代码
- DDL（CreateTable/Drop）/DML/查询三分支内联在同一个 match 中
- 5 个 profiling 计时段（`cache_hit_check` / `parse_and_plan` / `table_metadata_lookup` / `executor_creation` / `executor_execution`）以 `if profiling { ... }` 块形式内联散布全函数

后果：parse / plan / execute 三阶段无法独立单测、无法独立 micro-bench、阶段耗时不具备可归因边界。任务书验收：三阶段独立 micro-bench；单测可分别覆盖。

## What Changes

### T03：WalWriter 持久句柄（Iteration 000）

- `WalWriter` 结构体持有持久文件句柄（内部同步原语保护，具体选型见 design），5 个方法全部改为通过该句柄操作，删除逐次 open/close
- **错误语义不变**：IO 失败仍返回 `WalError::IoError` 上抛，不自动重试、不自动重开
- **LSN 文件位置语义不变**：`write_record` 仍以文件末尾偏移为 LSN；`write_batch` 继续由调用方传入 LSN
- 新增 fd 上界集成测试：10K tx 压测进程内 `/proc/self/fd` 计数 < 10（替代外部 lsof，Linux x86_64 是唯一支持平台）
- 调用方（WALBuffer / Checkpoint / Database）公开 API 不变：`Arc<WalWriter>` 共享形态与 `&self` async 方法签名预期保持

### T04：pipeline 三阶段拆分（Iteration 001）

- `execute_inner` 拆为 parse / plan / execute 三个独立阶段函数：
  - parse 阶段 = `parse_sql`
  - plan 阶段 = cache 查找 / 表注册 / `build_plan` / cache 写入
  - execute 阶段 = executor 创建 + 运行（含 DML 事务包裹）
- profiling 观测重构为三段顶层计时（parse/plan/execute），保留子指标能力；开关机制沿用现有 `is_profiling_enabled()`；`print_timings` 输出名称允许调整
- 新增三阶段独立 micro-bench（benches/ 下新增一套）
- 新增阶段级单测：每阶段可脱离完整 pipeline 独立调用并断言
- 行为保持：DML 事务包裹语义（T01 成果）、DDL 后 `plan_cache.clear()`、错误 Response 格式、cache-hit 跳过 parse/plan

## Capabilities

### New Capabilities

- `wal-writer-handle-reuse`：WalWriter 句柄生命周期
  - 改前：5 个方法逐次 open/close，无句柄数上界保证，fd churn 随写入量线性发生
  - 改后：单一句柄全程复用，fd 数有静态上界且可用 `/proc/self/fd` 断言验证；错误与 LSN 对外语义零变化
  - 关联 M/K：M01（整体架构数据流）、M09（异步协程调度核心）、M13（异步执行原则）
- `pipeline-stage-decomposition`：执行管道阶段边界与可观测性
  - 改前：单函数混合三阶段职责，profiling 计时内联，无阶段级测试与基准入口
  - 改后：parse/plan/execute 三阶段独立函数，顶层三段计时，阶段级单测 + 独立 micro-bench；对外 Response 与缓存行为不变
  - 关联 M/K：M01（SQL→Parser→Plan→Executor 数据流）、M13（异步执行原则）

### Modified Capabilities

- 无。现有 `dml-transaction-lifecycle` spec 的行为约束（DML 必须运行在真实事务内）由 T04 显式 Preserve，不产生 delta。

## Impact

- **影响模块**：
  - `src/wal/writer.rs`（核心改写：句柄持有 + 5 方法改造）
  - `src/pipeline.rs`（核心改写：三阶段拆分 + 计时重构）
  - `src/profiling.rs`（可能微调：新增三段计时接口或调整 print_timings 分组；机制不变）
  - `tests/`（新增 fd 上界集成测试 + pipeline 阶段级单测所在文件待 design 定）
  - `benches/`（新增 pipeline 三阶段 bench）
  - `src/wal/buffer.rs`、`src/wal/checkpoint.rs`（调用方，预计零修改——API 兼容前提下仅回归验证）
- **影响接口**：
  - `WalWriter` 公开方法签名预期不变（`&self` async）；内部字段增加句柄 + 同步原语
  - `pipeline::execute_inner` 为私有函数，拆分不构成 crate 公开 API 变化
  - `Database::open` / `execute_sql` 公开 API 无变化
- **影响行为**：
  - 可观察行为差异仅限：fd 数从"随调用次数 churn"变为"常数上界"；profiling 输出的计时名称分组变化
  - SQL 执行结果、错误消息格式、WAL 文件格式、恢复语义全部不变
- **兼容性**：
  - WAL on-disk 格式零变化（只改句柄管理方式，不改 record 序列化）
  - RecoveryManager 读路径不受影响（reader 独立于 writer）
  - 现有 504 tests pass 基线必须保持
- **风险**：
  - 中：`Arc<WalWriter>` 多任务并发下持久句柄需要内部互斥；若用 `Mutex<File>`，写路径串行化程度需与现状对齐（现状 spawn_blocking 各自独立 open 也无跨请求共享锁）——design 定选型并用并发测试覆盖
  - 低：持句柄后 truncate_to + 后续 append 的位置语义依赖 O_APPEND 保证（append 写总是到文件末尾）——设计显式验证
  - 低：profiling 输出格式变化可能影响人工读日志习惯，无程序化消费者
- **回退方案**：git revert 本 change；两个 task 相互独立，可分别 revert

## 用户已确认决策（2026-08-26 Gate 1 缺口集中裁决）

| # | 缺口 | 决策 |
|---|---|---|
| G1 | T03 改造范围 | 全部 5 个方法复用同一句柄（超出 tasks.md 列出的 3 个方法行号范围，用户批准扩展） |
| G2 | T03 验收度量 | 进程内 `/proc/self/fd` 计数断言进 cargo test（替代外部 lsof 命令） |
| G3 | T03 写失败语义 | 保持现状：IoError 上抛，不自动重试重开 |
| G5 | T03 LSN 来源 | 保持文件位置语义：write_record 用文件末尾偏移，write_batch 由调用方传 LSN |
| G4 | T04 观测形态 | 重构为三段顶层计时（parse/plan/execute），保留子指标能力，输出名称允许调整，开关机制不变 |
