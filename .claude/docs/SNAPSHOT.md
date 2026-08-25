# SNAPSHOT

> 最后更新：2026-08-25（MS06-T01 归档后增量刷新）
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

- `src/database.rs` — Database 协调器
- `src/pipeline.rs` — SQL 执行管道入口（含 DML 事务包裹，MS06-T01 落地）
- `src/parser/` — SQL 解析 + PlanBuilder
- `src/executor/` — 24 个执行器（Scan / DataScan / IndexScan / IndexScanAll / Filter / Join / Aggregate / Sort / Limit / SemiJoin / AntiJoin / SubqueryEval / Correlated / Insert / Update / Delete / CreateTable / DropTable / DerivedScan / Having / Predicate / ValueRef / Result 等）
- `src/storage/` — BufferPool（DashMap + Miss Semaphore + Per-Page Loading Locks）、AsyncStorage、FileStorage、DataPage
- `src/storage/btree/` — B-Tree（IndexManager、LeafNode、InternalNode、redistribution-first merge）
- `src/storage/data/` — TableManager（data_page_head 链表）
- `src/storage/page_format/` — SlottedPage（6B logical_id slot）
- `src/storage/page_visibility.rs` — PageVisibilityInfo（页面级 MVCC 摘要）
- `src/transaction/` — TransactionId（AtomicU64）、TransactionManager（begin/commit/abort 唯一 WAL 源）、Snapshot、VersionChain（含 DELETED_TX_ID 墓碑守卫，MS06-T01）、RowLock
- `src/wal/` — WalWriter / Reader / Buffer / Checkpoint / Recovery
- `src/network/` — Server（Semaphore 并发限流）、PgProtocol（write_buf 批写 + TCP_NODELAY）、JsonProtocol

## 目录约定

- 源码: `src/`
- 集成测试: `tests/`
- 单元测试: 文件内 `#[cfg(test)]`
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
- **最新 revision**: 936ec0f797993f7b17b3307efa1577063cba929d
- **M31 commits**: 6 commits ahead of origin
- **最新 tag**: M11
- **测试**: 487 tests pass, 0 failures（2026-08-25 MS06-T01 完成后；含 6 个新 dml_tx_id_test）
- **OpenSpec**: 6 capability specs validate PASS（decisions / dml-transaction-lifecycle / improvements / knowledge / project-model / references，2026-08-25 增量后）

## 同步状态

- `current` — 文档与代码一致（MS06-T01 归档后增量刷新）

## 权威文档

- 公共规则: `CLAUDE.md`
- 项目模型: `openspec/specs/project-model/spec.md` (Mxx)
- 决策: `openspec/specs/decisions/spec.md` (Dxx)
- 知识: `openspec/specs/knowledge/spec.md` (Kxx)
- 参考: `openspec/specs/references/spec.md` (Rxx)
- 改进: `openspec/specs/improvements/spec.md` (Ixx)
- 任务与路线: `.claude/docs/tasks.md`
- 变更: `openspec/changes/`（当前无活跃 change；归档目录含 MS06-T01 carrier）
- Legacy migration carrier: `.claude/legacy/2026-08-25-openspec-init-migration/`
- 新增能力 spec: `openspec/specs/dml-transaction-lifecycle/spec.md`（MS06-T01 落地）
