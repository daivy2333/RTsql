# SNAPSHOT

> 最后更新：2026-08-26（MS07-T01 提交并增量刷新；commit `3984b26`）
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

- `src/database.rs` — Database 协调器（含 `close()` 显式落盘，MS07-T01 落地）
- `src/pipeline.rs` — SQL 执行管道入口（含 DML 事务包裹，MS06-T01 落地）
- `src/parser/` — SQL 解析 + PlanBuilder
- `src/executor/` — 24 个执行器（Scan / DataScan / IndexScan / IndexScanAll / Filter / Join / Aggregate / Sort / Limit / SemiJoin / AntiJoin / SubqueryEval / Correlated / Insert / Update / Delete / CreateTable / DropTable / DerivedScan / Having / Predicate / ValueRef / Result 等；InsertExecutor 持有 `Option<Arc<TableManager>>` 走 `write_tuple` 路径，MS07-T01 落地）
- `src/storage/` — BufferPool（DashMap + Miss Semaphore + Per-Page Loading Locks）、AsyncStorage（含 `page_count()`，MS07-T01 落地）、FileStorage、DataPage
- `src/storage/catalog.rs` — Catalog（系统表 `__tables` / `__columns` SlottedPage 管理 + 二进制行序列化 + 链式页表 + 保留名常量，MS07-T01 落地）
- `src/storage/btree/` — B-Tree（IndexManager 含 `from_root` 路径 + `root_page_id` 访问器，MS07-T01 落地；LeafNode、InternalNode、redistribution-first merge）
- `src/storage/data/` — TableManager（`async new(bp, storage) -> Result<Arc<Self>>` + `open_or_init` 重建 + 保留名检查 + 跨页 tail 同步，MS07-T01 落地）
- `src/storage/page_format/` — SlottedPage（6B logical_id slot）
- `src/storage/page_visibility.rs` — PageVisibilityInfo（页面级 MVCC 摘要）
- `src/transaction/` — TransactionId（AtomicU64）、TransactionManager（begin/commit/abort 唯一 WAL 源）、Snapshot、VersionChain（含 DELETED_TX_ID 墓碑守卫，MS06-T01）、RowLock
- `src/wal/` — WalWriter / Reader / Buffer / Checkpoint / Recovery（K05 静默吞错遗留，下一 change 修复）
- `src/network/` — Server（Semaphore 并发限流）、PgProtocol（write_buf 批写 + TCP_NODELAY）、JsonProtocol

## 目录约定

- 源码: `src/`
- 集成测试: `tests/`（含新增 `tests/schema_persistence_test.rs` 8 测试，MS07-T01 落地）
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
- **最新 revision**: 3984b26（HEAD = MS07-T01 commit；领先 origin 3 commits）
- **ahead of origin**: 3 commits（7d37827 MS06-T03+T04 → 8884e0b MS07-T01 → 本地 f392c73 之上 1 个 `repo(_)`）
- **最新 tag**: M11
- **测试**: 534 tests pass, 0 failures（2026-08-26 MS07-T01 提交后；含 10 个新 catalog 单元测试 + 8 个新 schema persistence 集成测试；baseline 516 + 18 新增 = 534）
- **OpenSpec**: 10 capability specs validate PASS（decisions / dml-transaction-lifecycle / improvements / knowledge / pipeline-stage-decomposition / plancache-key-normalization / project-model / references / schema-persistence / wal-writer-handle-reuse，2026-08-26 MS07-T01 提交后）

## 同步状态

- `current` — 文档与代码一致（MS07-T01 提交后增量刷新；commit `3984b26`）

## 权威文档

- 公共规则: `CLAUDE.md`
- 项目模型: `openspec/specs/project-model/spec.md` (Mxx)
- 决策: `openspec/specs/decisions/spec.md` (Dxx)
- 知识: `openspec/specs/knowledge/spec.md` (Kxx)
- 参考: `openspec/specs/references/spec.md` (Rxx)
- 改进: `openspec/specs/improvements/spec.md` (Ixx)
- 任务与路线: `.claude/docs/tasks.md`
- 变更: `openspec/changes/`（当前无活跃 change；归档目录含 MS06-T01 + MS06-T02 + MS06-T03-T04 + MS07-T01 carrier）
- Legacy migration carrier: `.claude/legacy/2026-08-25-openspec-init-migration/`
- 新增能力 spec:
  - `openspec/specs/dml-transaction-lifecycle/spec.md`（MS06-T01 落地）
  - `openspec/specs/plancache-key-normalization/spec.md`（MS06-T02 落地）
  - `openspec/specs/wal-writer-handle-reuse/spec.md`（MS06-T03 落地）
  - `openspec/specs/pipeline-stage-decomposition/spec.md`（MS06-T04 落地）
  - `openspec/specs/schema-persistence/spec.md`（MS07-T01 落地，7 个 Requirement）
