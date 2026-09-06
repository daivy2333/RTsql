# SNAPSHOT

> 最后更新：2026-09-06（MS10-T01 提交并增量刷新；commit `03ff1b9`）
> 同步状态：current

## 项目身份

RTsql — 异步协程驱动的高性能嵌入式关系型数据库。以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

## 技术栈

- **语言**: Rust 2021 edition
- **构建工具**: Cargo
- **异步运行时**: Tokio (rt-multi-thread, macros, sync, time, net, fs, io-util)
- **SQL 解析**: sqlparser-rs 0.44
- **序列化**: serde + serde_json
- **并发原语**: dashmap, lru, tokio-util
- **CLI**: clap 4 (derive)
- **校验**: crc32fast
- **测试框架**: criterion.rs (benchmark) + tempfile + rusqlite (对比)
- **格式化**: rustfmt
- **Lint**: clippy
- **随机数**: rand 0.8

## 关键特性

- **轻量**: 单库静态链接，无外部服务依赖
- **便捷**: API 简洁（open / execute / query），支持内存模式与持久化单文件
- **高效**: 基于协程的异步 I/O、MVCC 无锁读、零拷贝页访问、DashMap 缓冲池

## 主要模块边界

- `src/database.rs` — Database 协调器（含 `close()` 显式落盘，MS07-T01；显式事务 API `begin/commit/rollback/execute_in_tx`，MS07-T04；`checkpoint_manager` 接线 + 公开 `checkpoint()` + `close()` 自动触发，MS07-T05）
- `src/pipeline.rs` — SQL 执行管道入口（含 DML 事务包裹，MS06-T01；用户事务执行路径 `execute_in_tx`/`execute_stage_in_tx`，MS07-T04；6 执行器构造点接线投影 `with_projection`，MS10-T01）
- `src/cli/` — CLI 非交互入口（one-shot 主命令 `rtsql <db> <sql>`：clap 参数化；`resolve_db_path` 名称解析——裸名→`$RTSQL_HOME/db/<name>.db`（默认 `~/.rtsql/`）、含 `/` 路径直开；`render` 四格式纯函数（table/json/csv/tsv，TTY 默认 table / 非 TTY 默认 json）；退出码 0/1/2/3 + 4/5 枚举留位；多语句显式拒绝护栏，MS10-T01）
- `src/parser/` — SQL 解析 + PlanBuilder；`planner/` 6 模块（mod/query/expression/aggregate/subquery/ddl_dml，`PlanBuilder` 三字段 pub(crate) + 公共 API 零变化，MS07-T03 落地）；query.rs 含 JOIN 表头臂 + `resolve_projection_indices` 投影解析 + 聚合 `input_schema` 统一（MS10-T01）
- `src/executor/` — 24 个执行器（Scan / DataScan / IndexScan / IndexScanAll / Filter / Join / Aggregate / Sort / Limit / SemiJoin / AntiJoin / SubqueryEval / Correlated / Insert / Update / Delete / CreateTable / DropTable / DerivedScan / Having / Predicate / ValueRef / Result 等；InsertExecutor 持有 `Option<Arc<TableManager>>` 走 `write_tuple` 路径，MS07-T01；DataScan 支持 `predicate` 行内谓词过滤与 `scan_cap` 提前封顶，OR/Sort/Aggregate 路径保留原节点，MS07-T06；DataScan 支持后继页预取 `with_prefetch(true)` 显式启用、默认关闭，MS08-T02；扫描/Filter/Sort 执行器 `with_projection` 真投影——谓词与 MVCC 判定后按投影裁剪，`SELECT *` 恒等，MS10-T01）
- `src/storage/` — BufferPool（DashMap + Miss Semaphore + Per-Page Loading Locks）、AsyncStorage（含 `page_count()`，MS07-T01）、FileStorage（页读写 `FileExt::read_exact_at`/`write_all_at` 位置参数化，每页 1 syscall，MS08-T01）、DataPage
- `src/storage/catalog.rs` — Catalog（系统表 `__tables` / `__columns` SlottedPage 管理 + 二进制行序列化 + 链式页表 + 保留名常量，MS07-T01）
- `src/storage/btree/` — B-Tree（IndexManager 含 `from_root` 路径 + `root_page_id` 访问器，MS07-T01；`collect_all_pages` 物理页枚举 pub async + visited 防环，MS07-T02；LeafNode、InternalNode、redistribution-first merge）
- `src/storage/data/` — TableManager（`async new(bp, storage) -> Result<Arc<Self>>` + `open_or_init` 重建 + 保留名检查 + 跨页 tail 同步，MS07-T01；`drop_table` 物理释放数据/索引页到 free-list + 私有 `collect_data_pages` 链遍历，MS07-T02）
- `src/storage/page_format/` — SlottedPage（6B logical_id slot）
- `src/storage/page_visibility.rs` — PageVisibilityInfo（页面级 MVCC 摘要）
- `src/transaction/` — TransactionId（AtomicU64）、TransactionManager（begin/commit/abort 唯一 WAL 源；`record_version` 按表聚合 + 多表回滚含墓碑，MS07-T04）、Snapshot、VersionChain（含 DELETED_TX_ID 墓碑守卫，MS06-T01）、RowLock
- `src/wal/` — WalWriter（含 `rewrite_truncate` 单临界区原地截断）/ Reader（带 LSN 读取）/ Buffer / Checkpoint（位点消费 + 九步重写截断，WAL 有界，MS07-T05）/ Recovery（位点过滤 redo + 全部失败显式 `Err`，K05 已修复，MS07-T05）
- `src/network/` — Server（Semaphore 并发限流）、PgProtocol（write_buf 批写 + TCP_NODELAY）、JsonProtocol

## 目录约定

- 源码: `src/`
- 集成测试: `tests/`（含新增 `tests/schema_persistence_test.rs` 8 测试，MS07-T01 落地；新增 `tests/drop_table_free_test.rs` 6 测试，MS07-T02 落地；新增 `tests/explicit_tx_test.rs` 8 测试，MS07-T04 落地；新增 `tests/checkpoint_redo_reduction_test.rs` 9 测试，MS07-T05 落地；新增 `tests/pushdown_test.rs` 15 测试，MS07-T06 落地；新增 `tests/file_storage_io_test.rs` 4 测试 + `tests/prefetch_test.rs` 3 测试，MS08-T01/T02 落地；新增 `tests/cli_test.rs` 12 测试 + `tests/projection_test.rs` 6 测试，MS10-T01 落地）
- 单元测试: 文件内 `#[cfg(test)]`（含新增 `src/storage/catalog.rs` 10 单元测试）
- 基准测试: `benches/` (8 套: micro / concurrent / scale / sqlite_compare / single / precise_compare / data_scan / visibility)
- OpenSpec: `openspec/`
- 状态文档: `.claude/docs/`
- 模板: `.claude/docs/templates/`
- 分析: `.claude/analysis/`（按需）
- Runbook: `.claude/runbooks/`（按需）
- Incident: `.claude/incidents/`（按需）
- Legacy carrier: `.claude/legacy/`

## 支持平台

- 当前: Linux x86_64
- AI 平台: Claude Code、Codex、OpenCode

## 仓库现场

- **分支**: master
- **最新 revision**: 03ff1b9（HEAD = MS10-T01 commit）
- **ahead of origin**: 2 commits（含本次 docs sync）
- **最新 tag**: M11
- **测试**: 614 tests pass, 0 failures（2026-09-06 MS10-T01 提交后；基线 585 + cli lib 单测 11 + cli_test 12 + projection_test 6）
- **OpenSpec**: 15 capability specs validate PASS（新增 cli-noninteractive-shell；2026-09-06 归档 ms10-t01-cli-shell change 后）

## 同步状态

- `current` — 文档与代码一致（MS10-T01 提交后增量刷新；commit `03ff1b9`）

## 权威文档

- 公共规则: `CLAUDE.md`
- 项目模型: `openspec/specs/project-model/spec.md` (Mxx)
- 决策: `openspec/specs/decisions/spec.md` (Dxx)
- 知识: `openspec/specs/knowledge/spec.md` (Kxx)
- 参考: `openspec/specs/references/spec.md` (Rxx)
- 改进: `openspec/specs/improvements/spec.md` (Ixx)
- 任务与路线: `.claude/docs/tasks.md`
- 变更: `openspec/changes/`（当前无活跃 change；归档目录含 MS06-T01 + MS06-T02 + MS06-T03-T04 + MS07-T01 + MS07-T02 + MS07-T03 + ms07-rest + ms08-t01-t02 + ms10-t01-cli-shell carrier）
- Legacy migration carrier: `.claude/legacy/2026-08-25-openspec-init-migration/`
- 新增能力 spec:
  - `openspec/specs/dml-transaction-lifecycle/spec.md`（MS06-T01 落地）
  - `openspec/specs/plancache-key-normalization/spec.md`（MS06-T02 落地）
  - `openspec/specs/wal-writer-handle-reuse/spec.md`（MS06-T03 落地）
  - `openspec/specs/pipeline-stage-decomposition/spec.md`（MS06-T04 落地）
  - `openspec/specs/schema-persistence/spec.md`（MS07-T01 落地，7 个 Requirement）
- `openspec/specs/drop-table-physical-free/spec.md`（MS07-T02 落地，7 个 Requirement）
- `openspec/specs/planner-module-decomposition/spec.md`（MS07-T03 落地，5 个 Requirement）
- `openspec/specs/ms07-rest-tx-checkpoint-pushdown/spec.md`（MS07-T04/T05/T06 落地，3 个 Requirement：R1 显式事务 / R2 Checkpoint / R3 谓词-LIMIT 下推）
- `openspec/specs/storage-io-optimization/spec.md`（MS08-T01/T02 落地，3 个 Requirement：R1 页 I/O 位置参数化 / R2 零接口零格式变更 / R3 DataScan 预取可选能力默认关闭）
- `openspec/specs/cli-noninteractive-shell/spec.md`（MS10-T01 落地，6 个 Requirement：R1 参数化入口与主命令 / R2 名称解析 / R3 列名表头 / R4 输出格式四态 / R5 多语句显式拒绝 / R6 扫描执行器真投影）
