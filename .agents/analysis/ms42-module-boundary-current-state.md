# MS42 模块边界现状事实面

> Snapshot: [SNAPSHOT](../docs/SNAPSHOT.md)
> Captured revision: 1f7dd6e794c61fb005264c46c6771400646dab44（master，工作区干净）
> Captured at: 2026-10-28
> See also: [模块化历史分析](workspace-crate-modularization.md)（R25）、[后续工作优先级分析](future-work-priority-and-dependencies.md)（R77）

为 [MS42「可组合的模块边界」](../docs/tasks.md) 的 plan 阶段提供当前代码事实面。全部结论来自静态读码与 grep 统计（方法与口径随条目注明），没有运行任何产品测试或基准。R25 的模块图与依赖环数字是 2026-09-24 采集，本文按当前 revision 更新并逐边核对。

## 目标与范围

回答七个问题：

1. 当前模块清单、规模与实际 use 依赖方向（更新 R25 的「11 模块、99 处 pub(crate)」）。
2. R25 记载的两条依赖环是否仍存在、精确位置。
3. network、加密、CLI 三个可选化候选的依赖事实面与 Cargo.toml 依赖的边界归属。
4. 跨模块共享可变状态的清单、读写层与所有权。
5. 把 CLI、network、加密变为 optional 时编译边界上要动的接缝事实。
6. 当前模块对外的错误/事件出口形态（供「模块向统一诊断报告信息的接口」约定引用）。
7. tests/ 对模块边界的锚定方式与 feature 组合验证可依托的命令面。

不包含：拆分设计、feature 方案选择、接口裁剪建议——这些属 plan 阶段决策。

## 一、模块清单、规模与依赖方向

### 1.1 模块与规模（2026-10-28 实测）

| 模块 | 文件数 | 行数 | 层（按 MS42 Scope 用词） |
|---|---:|---:|---|
| `src/database.rs` | 1 | 1,019 | 核心（协调器） |
| `src/pipeline.rs` | 1 | 4,175 | 核心（管道） |
| `src/plan_cache.rs` | 1 | 217 | 核心 |
| `src/result.rs` | 1 | 122 | 核心（类型化结果词表） |
| `src/profiling.rs` | 1 | 65 | 跨切面（计时） |
| `src/parser/` | 12 | 12,449 | 规划（解析 + PlanBuilder 七模块） |
| `src/executor/` | 50 | 13,983 | 执行 |
| `src/storage/` | 32 | 18,583 | 存储 |
| `src/transaction/` | 8 | 2,021 | 事务 |
| `src/wal/` | 7 | 3,657 | 持久化（WAL） |
| `src/network/` | 8 | 1,023 | 入口（服务/协议） |
| `src/cli/` | 4 | 3,726 | 入口（CLI） |
| `src/main.rs` | 1 | 6 | 入口（bin 装配） |
| 合计 | 128 | 61,065 | |

R25 口径更新：当时「11 个模块」，现为 **12 个 lib 模块 + 1 个 bin**（新增的 result.rs 是 MS31 类型化 API 产物，plan_cache.rs 当时已存在）。`pub(crate)` 从 99 处增至 **240 处**（`grep -rn 'pub(crate)' src/ | wc -l`），分布：parser/planner 108、executor 48、storage 家族（含 page_format/data/btree）59、pipeline.rs 8、wal 7、database.rs 7、cli 3。`pub` + `pub(crate)` 条目声明量级（排除 `#[cfg(test)]` 段，含 impl 内缩进项）：storage 408、executor 275、parser 118、transaction 64、wal 62、database 41、network 39、pipeline 26、cli 12、plan_cache 9、profiling 7、result 4。

### 1.2 依赖方向（排除 `#[cfg(test)]` 段后的全部 `crate::X` 引用）

统计方法：每个源文件取首个 `#[cfg(test)]` 之前的部分，数 `crate::X` 全部出现（use 声明 + 内联路径）。箭头 = 引用方向。

```text
database  → pipeline(11) storage(6) wal(2) transaction executor result plan_cache profiling
pipeline  → storage(10) parser(7) executor(3) result(2) transaction profiling database(1)
parser    → executor(48) storage(17)
executor  → storage(103) transaction(15) pipeline(13) database(13) wal(4) parser(3) profiling(2)
storage   → executor(18) transaction(6) wal(6)
transaction → storage(6) wal(3) database(1)
wal       → storage(14) transaction(1)
network   → executor(4) database(2) result(1)
cli       → storage(4) result(3) parser(3) pipeline(2) database(2) transaction executor(1)
plan_cache → executor
result    → executor storage
profiling → （零依赖，零入边之外只有 database/pipeline/executor 三个消费点）
```

关键横向/反向边（正向 data-flow 之外的部分）：

| 边 | 符号与位置 |
|---|---|
| storage → executor | `executor::Value` 27 处 + `ValueRef` 2 处：`src/storage/catalog.rs`、`src/storage/data/table_manager.rs`、`src/storage/page_format/tuple.rs`——值词表定义在执行层、存储逐字节依赖 |
| parser → executor | `executor::datetime` 12、`JoinSide` 8、计划节点类型（`DerivedScanNode`/`JoinNode`/`NestedLoopJoinNode`/`LimitNode`/`SubqueryEvalNode` 等）、`is_scalar_function` 2——规划层构造执行层计划词表 |
| executor → parser | `PlanError::SubqueryReturnsMultipleRow` 仅作错误文本：`src/executor/subquery_eval.rs:142,222`、`src/executor/scalar_subquery_filter.rs:152` |
| executor → database | `crate::database::Database` 具体类型，13 个文件：`{alter_table, anti_join, create_index, create_table, create_view, drop_index, drop_table, drop_view, join_related_config, scalar_subquery_filter, semi_join, subquery_eval, truncate}.rs`（DDL 执行器 + 子查询/Join 配置两族） |
| executor → pipeline | `RoutedTables` 12 + `create_executor_from_plan` 5：`{anti_join, join_related_config, scalar_subquery_filter, semi_join, subquery_eval}.rs`——子查询执行器重建臂 |
| pipeline → database | `use crate::database::Database;`（src/pipeline.rs:1）——execute/execute_stage 族以 `&Database` 为参数 |
| transaction → database | `use crate::database::Database;`（src/transaction/session.rs:9，产品代码；R25 时该边仅在测试） |
| storage → wal | `table_manager.rs`、`storage/error.rs`（`StorageError::WalError(String)` 字符串化） |
| wal → transaction | `src/wal/recovery.rs`（重放路径） |
| result → executor/storage | `Value`、`ColumnType`（src/result.rs:10-11）——结果词表依赖两低层词表 |

`profiling` 是唯一零依赖模块；`plan_cache` 只依赖 executor（PhysicalPlan）。

## 二、R25 两条依赖环现状

**环 1（database → pipeline → executor → database）：三条边全部仍存在，且比 R25 记载更密。**

- database → pipeline：11 处（`execute`/`execute_with_params`/`execute_in_tx[_with_params]`/`plan_prepared`/`materialize_params`/`execute_stage`/`parameter_count_mismatch`，src/database.rs:537-1013）。
- pipeline → executor：3 处（`create_executor_from_plan` 等）。
- executor → database：13 个文件持有 `Database`（R25 记 5 个；扩容来自 DDL 执行器族）。
- 新增维度：executor → pipeline（R25 未记）与 pipeline → database 直接边——子查询/Join 执行器调用 `create_executor_from_plan` 重建执行树并共享 `RoutedTables`，形成 executor ↔ pipeline 独立环。

**环 2（storage ↔ transaction）：仍存在，位置与 R25 基本一致。**

- storage → transaction：`VersionHeader`（`src/storage/data_page.rs`、`src/storage/data/table_manager.rs`、`src/storage/buffer_pool.rs`）、`TransactionManager`（`src/storage/integrity.rs`）、`TransactionError`（`src/storage/error.rs:5`），共 5 文件 6 处——数据页格式逐字节嵌入事务词表，性质未变。
- transaction → storage：`page_format` 3、`RowId` 2、`ColumnType` 2，在 `src/transaction/manager.rs`、`row_lock.rs`、`version_chain.rs`。

R25 之后的结构变化（对 MS42 有利的事实）：

- **F6「Response 错层」已解除**。src 内除 network 自身外无人引用 `crate::network`（全仓 grep 为零）。核心结果载体是 `src/result.rs` 的 `SqlOutcome`/`SqlError`，`network::protocol::to_response` 只是 CLI/Server 边界转换器（result.rs 头注明确「本模块不依赖 network」）。network 现为**叶子模块**。
- database → network、pipeline → network、cli → network 三条 R25 记载的反向边全部消失。
- 但出现了 R25 未记的 **database ↔ transaction 模块环**（transaction/session.rs:9 产品代码引入 `Database`；`TransactionSession` 方法以 `&Database` 执行 `execute_in_tx`）。

## 三、可选依赖评估事实面

### 3.1 network（8 文件 / 1,023 行）

入边（network 消费什么）：`database::Database`（server.rs、handler.rs）、`executor::{Value, datetime}`（pg_messages.rs、pg_protocol.rs——Date/Timestamp OID 文本编码）、`result::{value_to_json, SqlError, SqlOutcome}`（protocol.rs:7）。出边：src 内为零。

**最大耦合不在 src 而在测试面**：`tests/` 中 **97 个文件** import `rtsql::network::protocol::{to_response, Response}` 作为查询断言词表（典型形态 `to_response(db.execute_sql(sql).await)`，如 tests/aggregate_test.rs:4,16）；另有 6 个协议专项测试（pg_protocol、pg_integration、network_server、connection_limit、pg_messages、network_protocol）。`tokio-util`（rt feature，CancellationToken）只被 `src/network/server.rs` 使用。

### 3.2 加密（argon2 0.6 + aes-gcm 0.11）

- 实现全部在 `src/storage/crypto.rs`（Argon2id KDF + Aes256Gcm PageCipher + OsRng nonce），消费者只有 `src/storage/file_storage.rs`（`cipher: Option<Arc<PageCipher>>` 字段 + `derive_key`，src/storage/file_storage.rs:14,57）与 `src/storage/file_header.rs`（`FLAG_ENCRYPTED`/`KNOWN_FLAGS_MASK`/32B 盐 + 12B KDF 参数段 + `validate_kdf_params`）。
- Database 面的接触是纯参数：`open_with_key`/`open_readonly_with_key`/`open_*_with_lock_wait` 透传 `Option<&str>`（src/database.rs:67-343），无任何加密类型外漏到 storage 之外。
- 关闭态语义现成：`KNOWN_FLAGS_MASK = FLAG_ENCRYPTED`（src/storage/file_header.rs:24-27），不认识 bit1 即按 unknown flags 拒绝——mask 收窄到 0 就是「无加密构建拒绝打开加密库」的行为事实。
- `rand` **不能**随加密一起 optional：产品代码另有 3 个非加密使用点——`file_storage.rs:298`（盐生成）、`pg_protocol.rs:34`（协议随机）、`cli/lifecycle.rs:1409`（`sample` 子命令蓄水池采样）。
- 测试锚定：tests/encryption_test.rs + cli_test.rs 部分用例（错钥 exit 5）。

### 3.3 CLI（4 文件 / 3,726 行）

- 装配点：`src/main.rs` 全部内容 = `rtsql::cli::run()`；Cargo.toml 无 `[[bin]]` 声明（默认单 bin `rtsql` + lib `rtsql` 同名）。
- 消费面（非测试，src/cli/mod.rs:7-18 + lifecycle.rs:10-16）：`database::Database`；`parser::PlanBuilder` + `parser::planner::{classify_attach_statement, classify_transaction_statement, AttachInfo, TxStatementKind}`；`pipeline::{parse_stage, plan_stage[_routed], execute_stage[_in_tx_routed], AttachRoute, RoutedTables, infer_ctas_column_types(pub(crate))}`；`result::{value_to_json, SqlOutcome}`；`storage::{StorageError, catalog::{CatalogColumnRow, CatalogKind, CatalogRow}, integrity::{check_integrity, FindingSeverity}, page_format::ColumnType}`；`transaction::{IsolationLevel, TransactionSession}`；`executor::Value`。CLI 直接消费 pipeline 阶段函数与 storage catalog/integrity 内部件——「入口层只走 lib 公共 API」今天不成立。
- 专属依赖：clap、clap_complete、csv 只被 cli 使用。
- 测试锚定：44 个测试文件经 `env!("CARGO_BIN_EXE_rtsql")` 子进程锚定 CLI 行为；无任何测试 import `rtsql::cli`。

### 3.4 Cargo.toml 依赖归属（17 个运行时依赖，无 optional、无 features 表）

| 归属 | 依赖 | 事实 |
|---|---|---|
| 单一边界：仅 cli | clap、clap_complete、csv | 可选化 CLI 时随行 |
| 单一边界：仅 network | tokio-util | server.rs CancellationToken |
| 单一边界：仅存储加密 | argon2、aes-gcm | crypto.rs |
| 单一边界：仅 wal | crc32fast | writer/reader |
| 单一边界：仅 parser | sqlparser | parser/ 10 文件 |
| 跨边界 | tokio（全库）、async-trait（executor 37 + storage 5 + network 2 文件）、thiserror（storage/transaction/network 三个 error.rs）、serde+serde_json（result.rs value_to_json + cli 3 + network 2 文件）、dashmap（buffer_pool + plan_cache）、rand（4 个模块，见 3.2） | 可选化时不能整体随行 |
| **声明但未使用** | lru、anyhow | src/tests/benches 全无引用（`grep -rn 'use lru'` 与 anyhow 全仓为 0） |
| **仅测试使用却声明为运行时依赖** | futures | 只在 tests/concurrent_test.rs 用 `futures::future::join_all`（4 处） |

## 四、状态归属事实面（MS42 与 MS43 共同底座）

### 4.1 Database 协调器（src/database.rs:110，`#[derive(Clone)]`——克隆经 Arc 共享全部可变态）

| 字段 | 类型 | 所有权 / 读写层 |
|---|---|---|
| `buffer_pool` | `Arc<BufferPool>` | Database 持有；读写最广：storage（table_manager 41、catalog 32）、pipeline 16、executor（upsert/data_scan/delete/update/create_index/scan 等）、wal/checkpoint 12、storage/integrity 8 |
| `table_manager` | `Arc<TableManager>` | Database 持有；pipeline 18、cli/lifecycle 6、integrity 5、DDL 执行器族 |
| `transaction_manager` | `Arc<TransactionManager>` | Database 持有；pipeline 14（begin/commit/abort 编排）、database 12、executor/alter_table 4 |
| `wal_writer` | `Option<Arc<WalWriter>>`（仅 ReadWrite） | Database 持有；transaction/manager.rs 4（唯一 WAL 事务记录源）、pipeline 5、cli 零工作判定（lifecycle.rs:235） |
| `wal_buffer` | `Option<Arc<WALBuffer>>`（仅 ReadWrite） | Database 持有；**执行器写路径直接追加**（insert.rs:265、delete.rs:256、update.rs:326、upsert.rs:293/490/726、alter_table.rs:127）+ TransactionManager 内部 `RwLock<Option<Arc<WALBuffer>>>`（manager.rs:65）+ CLI shutdown（cli/mod.rs:1185）——同一对象三个层持有入口 |
| `plan_cache` | `Arc<PlanCache>` | Database 持有；pipeline 独占读写 |
| `schema_version` | `Arc<AtomicU64>` | Database 持有；`bump_schema_version` 单点在 pipeline；database 5 处（prepare/重计划）、pipeline 1 处；克隆共享、open 归零、不持久化 |
| `checkpoint_manager` | `Option<Arc<CheckpointManager>>` | Database 持有；仅 ReadWrite |
| `views` | `Arc<RwLock<HashMap<String, ViewMeta>>>` | Database 持有；写：executor create_view.rs:90 / drop_view.rs:66；读：pipeline（3592 注入 PlanBuilder）；planner 侧持有的是**注入快照副本**（`builder.set_views`，parser/planner/mod.rs:419），非共享 Arc |
| `query_memory_limit` | `Arc<AtomicU64>`（私有） | Database 持有；set/get 在 database，语句入口快照在 pipeline（2 处），env 通道在 cli（2 处）；克隆共享，变更只影响之后开始的语句 |
| `io_failpoint` | `IoFailPointSlot`（= `pub(crate) type Arc<Mutex<Option<IoFailPoint>>>`，storage/file_storage.rs:110） | Database 打开时分发同一单元给 FileStorage 与 WalWriter（writer.rs:37）；ATTACH 附加库各自独立单元 |
| `implicit_transactions` | `Arc<Mutex<HashSet<u64>>>`（私有） | Database 持有；守卫登记在 pipeline DML/CTAS 臂与 executor/alter_table（2 处）；三个回收入口在 database + pipeline 触发点 |
| `mode` / `isolation` / `path` | 私有，打开期定型 | `OpenMode` 与隔离级别运行期不变；readonly 会话 wal_writer/wal_buffer 为 None |

### 4.2 会话级后台对象与内部并发状态（MS43 直接相关）

- `WALBuffer`（wal/buffer.rs:19-37）：`buffer: Mutex<Vec<(u64, WalRecord)>>`、`current_lsn: AtomicU64`、`flush_notify: Notify`（后台 do_flush 任务由 clone-self spawn，buffer.rs:108）、`pending_commits: Mutex<Vec<u64>>`、`commit_waiters: Mutex<HashMap<u64, Arc<Notify>>>`、`shutdown: AtomicBool`。提交通知是「commit_and_wait → notify → 后台 flush → waiter 唤醒」的现有消息通道。
- `WalWriter`（wal/writer.rs:24-37）：`file: Arc<Mutex<File>>`、`bytes_appended`/`checkpoint_bytes_threshold: AtomicU64`（自动 checkpoint 水位）、共享 io_failpoint。
- `TransactionManager`（transaction/manager.rs:59-66）：`tx_id_allocator`、`active_tx_ids: RwLock<HashSet>`、`tx_versions: RwLock<HashMap<tx, HashMap<表, HashSet<RowId>>>>`（按表聚合的未提交版本登记）、内部 `wal_buffer` 槽。
- `BufferPool`（M17 详载，此处仅登记归属）：pages DashMap、vis_map、per-page loading_locks、clock_hand；Database 持 Arc，全部层经它读写页。
- IndexManager（storage/btree）：根 `AtomicPageId` + 写侧闸门 `write_gate` + 反向映射同步 RwLock——同一 blocking 闭包内的不可分割写单元。
- **ATTACH 会话状态不在 Database**：`struct AttachSession { dbs: HashMap<String, Arc<Database>> }` 在 cli/mod.rs:756，注释明确「刻意不放在 Database——attach 是 CLI 会话概念」；每附加库是完整独立 Database（独立事务/WAL/缓冲池），pipeline 的 `AttachRoute`/`RoutedTables` 做逐语句计划期路由。这是现存的一个**明确的跨层状态归属裁定先例**。

## 五、feature 化最小阻力面（只列接缝事实）

1. **条件编译从零开始**：src/ 内除 `#[cfg(test)]` 外没有任何 `cfg(feature)`/`cfg(target)`/`cfg(unix)`（全仓 grep 为 0）。无既有 feature 包袱，也无既有组合验证。
2. **network 可选的编译面**：src 侧只需 `lib.rs:6` 的 `pub mod network` 加 cfg + tokio-util 转 optional。阻力集中在**测试面 97 个文件的 to_response 断言 + 6 个协议测试**——可选化后这些测试要么改锚 `SqlOutcome`（断言词表迁移），要么 network 保持 default-on 并接受「关闭组合下这些测试不编译」。
3. **crypto 可选的编译面**：门 `storage/crypto.rs` 全文件 + `file_storage.rs` 的 `cipher` 字段/4124B 页步长分支 + `file_header.rs` 的 FLAG_ENCRYPTED/KDF 校验分支；Database 与其余层零改动（key 是字符串参数）。关闭态打开加密库 = unknown flags 拒绝（现错误面）。argon2/aes-gcm 可转 optional，**rand 不行**（3 个非加密产品使用点，见 3.2）。
4. **CLI 可选的编译面**：`main.rs` 单行装配点；lib.rs `pub mod cli` 加 cfg；`[[bin]]` 加 `required-features`（或不设 feature、维持 bin 恒构建）；clap/clap_complete/csv 转 optional。**硬事实**：44 个测试文件使用 `env!("CARGO_BIN_EXE_rtsql")`——bin 目标不在构建图中时该宏展开失败，这些测试文件在「无 CLI 组合」下**不可编译**，组合验证的测试目标集必须按组合重排。
5. **共享词表位置**（任何裁剪都绕不开的四个点）：`Value` 在 executor/value.rs（storage 27 处、parser、network、cli、result 消费）；`ColumnType` 在 storage/page_format/tuple.rs（result、cli、transaction 消费）；`PhysicalPlan` 在 executor/plan.rs（parser 构造、pipeline/executor 消费、lib.rs 重导出）；`datetime` 编码在 executor/datetime.rs（parser 12 处、pg_protocol 2 处消费）。lib.rs 根重导出面：`{ColumnConstraint, ColumnDef, ColumnType, PhysicalPlan, Value}` + `{parse_sql, PlanBuilder, PlanError}` + `{ColumnMeta, SqlError, SqlOutcome}`。
6. **存储后端接缝现存**：`AsyncStorage` trait（storage/async_storage.rs），Database 以 `Arc<dyn AsyncStorage>` 持有（database.rs:332），FileStorage/MemoryStorage 是仅有的两个实现——feature 化存储后端的第 2 层接缝已经存在。

## 六、诊断接口约定点

- **错误出口是单层漏斗**：库公共面只有 `SqlError { message, sqlstate: Option<String> }`（result.rs:40）+ `Result<SqlOutcome, SqlError>`。SQLSTATE 分类源分散三处：`parser::error::PlanError::sqlstate()`（静态 &str，42601/42P01/42703 等，parser/error.rs:145）、pipeline 的 `storage_error_sqlstate`/`executor_creation_error`（pipeline.rs:1673-1686，含 53200）与 `"Execution error: "` 前缀单点、network/pg_messages.rs:266 `map_error_to_sqlstate`（PG 面兜底）。各层错误类型用 thiserror 三个独立 error.rs（storage/transaction/network）。CLI 侧 `ExitStatus::sqlstate` 渲染（cli/mod.rs:66）。
- **无日志框架**：log/tracing/env_logger 全库零引用。产品代码有 **35 个 `eprintln!`/`println!` 直写出口**（排除 cfg(test)）：table_manager.rs 12（DROP/TRUNCATE 释放路径 best-effort 告警）、wal/recovery.rs 7、profiling.rs 5（计时表）、btree/index_manager.rs 4、network/server.rs 3、executor/drop_index.rs 2、data_page.rs 1、database.rs 1。模块今天各自直接写 stderr，无等级、无开关、无统一格式——这是「模块向统一诊断报告信息的接口」要收编的现状出口清单。
- **既有 trace 类设施**：profiling.rs（65 行）是唯一的阶段计时设施——`task_local! PROFILING_DATA` + `record_time`/`get_timings`/`print_timings` + `is_profiling_enabled()`（env `RTSQL_PROFILING`）+ `with_profiling_scope`。产品接线点仅 4 处：pipeline（parse/plan/execute 三段计时）、database 1、executor/index_scan.rs、index_scan_all.rs（`index_manager_search[_all]`）。粒度是「阶段 + 两个索引算子」，无事件分类、无等待/资源维度、无失败与取消路径覆盖。R77 已建议统一诊断先复用它；本节补充的是精确接线位置与覆盖缺口。

## 七、测试锚定面

- **规模**：`tests/` 157 个集成测试文件；src 内 54 个文件含 `#[cfg(test)]`、437 个 `#[test]`/`#[tokio::test]` 单元测试。
- **锚定方式以「直达模块 pub 项」为主，非 lib 根重导出**：`rtsql::database::` 124 文件；`rtsql::network::protocol::to_response` 97；`rtsql::storage::` 约 60；`rtsql::transaction::` 41；`rtsql::executor::` 42；`storage::page_format::` 19；`rtsql::wal::` 10；lib 根重导出仅 9 文件。深锚定例：`storage::write_tuple_to_data_page` 5 文件、`storage::update_version_header_in_data_page` 2、`pipeline::{parse_stage, plan_stage}` 4、`storage::catalog::{CatalogRow, CatalogColumnRow}`、`transaction::TransactionSession`。
- **pub 项是事实上的公共接口**：收窄任何 `pub` → `pub(crate)` 会等比例影响上述测试文件；反之 240 处现有 `pub(crate)` 不可达集成测试，收窄方向上 crate 内部接缝今天已经是不可见面。
- **CLI 行为经真二进制锚定**：44 文件 `CARGO_BIN_EXE_rtsql` 子进程（退出码/stdout/stderr/信号），与 lib API 面正交。
- **feature 组合验证可依托的命令面**：构建面 `cargo build [--no-default-features | --features …]`（features 表尚不存在，须先建立）；行为面沿用 M19 分批形态——`.agents/runbooks/m19-batched-full-regression.md`：`cargo test --lib --bins -- --test-threads=1` 一批 + `tests/*.rs` 按字典序 8–12 个/批、批间串行；组合下的目标集需按第 5.4 条重排（44 个 bin 依赖文件在无 CLI 组合不可编译；97 个 to_response 文件在无 network 组合不可编译）。
- 顺带事实：SNAPSHOT「benches/ 8 套」已过时，实际 12 套（Cargo.toml 12 个 `[[bench]]`；其中 single/precise_compare 已不存在，现有 index_key_encoding/wal_group_commit/tx_id/buffer_pool_concurrency/pipeline_stages 等未计入）。

## 发现的问题与候选（只报告，不登记不修复）

1. **死依赖（构建配置候选，非行为缺陷）**：`lru`、`anyhow` 声明于 `[dependencies]` 但 src/tests/benches 全无引用；`futures` 仅 tests/concurrent_test.rs 使用却占运行时依赖位。处置属 MS42/I093 范围（移除或移 dev-dependencies），由 plan 决定纳入与否。
2. **文档漂移候选**：SNAPSHOT benches 计数（8 vs 实际 12）过时。属 docs-maintainer 日常修正，随下次 SNAPSHOT 刷新处理即可。
3. 行为缺陷：**未发现**（本次全部为结构/依赖/测试锚定事实，未做运行验证，也未发现与规格矛盾的静态事实）。

## 未确认项

1. `cargo build --no-default-features` 等组合的**实际编译失败清单**未产生——features 表不存在，任何组合结论都须待 feature 方案落地后按第 5 节接缝事实验证。
2. executor 13 文件持有 `Database` 的具体字段访问深度（是否只读协调器字段还是深调用）未逐一展开——plan 做环 1 拆解设计时需按文件逐一定位。
3. `Value`/`PhysicalPlan` 词表归属（executor 层内还是下沉独立层）是 R25 遗留的单点架构决策，本文只更新了消费面数字，未做方案判断。

## 关键文件索引

| 事实 | 位置 |
|---|---|
| Database 协调器字段清单 | src/database.rs:110-160 |
| pipeline → database 边 | src/pipeline.rs:1 |
| executor → database 13 文件 | src/executor/{alter_table,anti_join,create_index,create_table,create_view,drop_index,drop_table,drop_view,join_related_config,scalar_subquery_filter,semi_join,subquery_eval,truncate}.rs |
| executor → pipeline（子查询重建臂） | src/executor/{anti_join,join_related_config,scalar_subquery_filter,semi_join,subquery_eval}.rs |
| storage ↔ transaction 词表熔接 | src/storage/{data_page.rs,buffer_pool.rs,data/table_manager.rs,error.rs,integrity.rs}；src/transaction/{manager.rs,row_lock.rs,version_chain.rs} |
| transaction → database（新环） | src/transaction/session.rs:9 |
| 类型化结果词表（不依赖 network） | src/result.rs |
| network 边界转换器 | src/network/protocol.rs（to_response） |
| 加密接线 | src/storage/{crypto.rs,file_header.rs:24-27,file_storage.rs:14,57}；src/database.rs:67-343 |
| IoFailPointSlot 共享单元 | src/storage/file_storage.rs:110；src/wal/writer.rs:37 |
| WAL 后台任务与通知 | src/wal/buffer.rs:19-37,108 |
| ATTACH 状态归属先例 | src/cli/mod.rs:750-762 |
| 阶段计时接线点 | src/profiling.rs；src/pipeline.rs；src/executor/{index_scan,index_scan_all}.rs |
| SQLSTATE 分类三源 | src/parser/error.rs:145；src/pipeline.rs:1673-1686；src/network/pg_messages.rs:266 |
| 分批全量入口 | .agents/runbooks/m19-batched-full-regression.md |
| Cargo 依赖与死依赖 | Cargo.toml:8-26 |
