## Purpose

索引项目依赖的内部产物和外部资料。条目使用 `Rxx` 编号，类型包括 dependency/external-doc/schema/runbook/analysis。

## Requirements

### Requirement: 参考可定位

参考 SHALL 记录类型、路径或 URL、版本或日期、用途和状态。

#### Scenario: 登记持久化产物

- **WHEN** 新分析、Runbook 或 Incident 需要跨会话复用
- **THEN** 使用递增 R 编号登记检索元数据

---

## 依赖文档

## R01: Cargo 运行时依赖

- **类型**: dependency
- **路径**: `Cargo.toml`
- **用途**: RTsql 运行时所需的 Rust crate 依赖
- **内容**:

  | 依赖 | 版本 | 链接 | 用途 |
  |---|---|---|---|
  | tokio | 1.x | https://docs.rs/tokio | async 运行时（rt-multi-thread, macros, sync, time, net, fs, io-util） |
  | sqlparser-rs | 0.44 | https://docs.rs/sqlparser | SQL 解析 |
  | async-trait | 0.1 | https://docs.rs/async-trait | async trait 支持 |
  | thiserror | 1.0 | https://docs.rs/thiserror | 错误类型派生 |
  | anyhow | 1.0 | https://docs.rs/anyhow | 错误处理 |
  | futures | 0.3 | https://docs.rs/futures | 异步原语 |
  | tokio-util | 0.7 | https://docs.rs/tokio-util | Tokio 工具（rt） |
  | serde | 1.0 | https://docs.rs/serde | 序列化框架 |
  | serde_json | 1.0 | https://docs.rs/serde_json | JSON 输出 |
  | rand | 0.8 | https://docs.rs/rand | 随机数 |
  | lru | 0.12 | https://docs.rs/lru | LRU 缓存（PlanCache） |
  | crc32fast | 1.4 | https://docs.rs/crc32fast | WAL CRC32 校验 |
  | dashmap | 6 | https://docs.rs/dashmap | 并发 HashMap（BufferPool vis_map） |

- **状态**: active
- **Legacy**: R001

## R02: Cargo 开发依赖

- **类型**: dependency
- **路径**: `Cargo.toml` [dev-dependencies]
- **用途**: RTsql 测试与基准所需依赖
- **内容**:

  | 依赖 | 版本 | 链接 | 用途 |
  |---|---|---|---|
  | criterion | 0.5 | https://bheisler.github.io/criterion.rs | 基准测试 |
  | rusqlite | 0.31 | https://docs.rs/rusqlite | SQLite 对比测试 |
  | tempfile | 3.x | https://docs.rs/tempfile | 测试临时目录 |
  | which | 6.0 | https://docs.rs/which | 查找可执行文件 |

- **状态**: active
- **Legacy**: R001

## R03: sqlparser-rs 0.44 关键 AST

- **类型**: external-doc
- **来源**: https://docs.rs/sqlparser/0.44
- **用途**: SQL 解析库的关键 AST 节点参考
- **内容**:

  | 类型 | 说明 |
  |---|---|
  | `GroupByExpr::All` | GROUP BY ALL |
  | `GroupByExpr::Expressions(Vec<Expr>)` | 显式分组列 |
  | `Expr::Function(Function)` | 函数调用（含聚合） |
  | `FunctionArg::Unnamed(FunctionArgExpr::Wildcard)` | COUNT(*) 的 * |
  | `FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))` | COUNT(col) 的 col |

- **状态**: active
- **Legacy**: R002

## 数据库设计参考

## R04: 数据库设计参考来源

- **类型**: external-doc
- **用途**: RTsql 架构设计的理论依据
- **内容**:

  | 主题 | 来源 |
  |---|---|
  | Volcano 迭代器模型 | Goetz Graefe "Volcano—An Extensible and Parallel Query Evaluation System" |
  | Hash Aggregation | 《数据库系统概论》聚合查询章节 |
  | MVCC | PostgreSQL MVCC 设计文档 |
  | B-Tree 页格式 | SQLite B-Tree 页格式文档 |
  | WAL | SQLite WAL 模式文档 |

- **状态**: active
- **Legacy**: R003

## 项目测试统计

## R05: 项目测试统计（2026-06-04）

- **类型**: schema
- **用途**: 当前测试覆盖与基准测试清单
- **内容**:
  - 总测试数: 475 tests pass, 0 failures（2026-06-04 统计；M31 完成后 481 tests pass）
  - Executor 测试: executor_test.rs（29 tests，含 M19 DataScan 8 tests）
  - 聚合测试: aggregate_test.rs（19 tests）
  - B-Tree 测试: btree_test.rs + btree_split_test.rs + btree_merge_test.rs（22 tests）
  - Visibility 测试: visibility_test.rs（5 tests，含 M21 页面级 MVCC）
  - 基准测试: 8 套（micro/concurrent/scale/sqlite_compare/single/precise_compare/data_scan/visibility）
- **状态**: active（数值会随实施更新）
- **Legacy**: R004

## 已迁移的旧 analysis 文档（指针）

## R06: M19 DataScan 路径分析（已实施）

- **类型**: analysis（已实施迁移）
- **状态**: completed
- **原因**: DataScan 已实施并归档到 M19 change；分析内容已沉淀到 K19 (实测性能) + M22 (数据页链表)
- **Legacy**: R007

## R07: M21 页面级 MVCC 遗留项分析（已解决）

- **类型**: analysis（已实施迁移）
- **状态**: completed
- **原因**: M21 遗留项 (DELETE mark_deleted + 惰性 set_all_visible + benchmark) 全部完成（commit `78a3b01`）；内容已沉淀到 K12 (mark_deleted) + K13 (惰性 set_all_visible)
- **Legacy**: R008

## 已归档 Change 索引

## R08: 2026-06-03-consolidate-m41-tx-id-atomic

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-consolidate-m41-tx-id-atomic/`
- **状态**: archived
- **内容**: M41 事务 ID AtomicU64 实施（commit `634764d` + `ee9ceee`）
- **关联决策**: D09
- **关联知识**: K16

## R09: 2026-06-03-consolidate-rules-into-claude-md

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/`
- **状态**: archived
- **内容**: 废弃 `openspec/specs/rules/`，规则合并到 CLAUDE.md
- **legacy carrier**: `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/archive/spec.md`（旧 rules.md 内容）+ `archive/CLAUDE.md.before`（旧 CLAUDE.md 内容）

## R10: 2026-06-03-m20-zero-copy-slotted-page-ref

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-m20-zero-copy-slotted-page-ref/`
- **状态**: archived
- **内容**: M20 零拷贝 SlottedPageRef 实施
- **关联决策**: D12 的 predecessor
- **关联知识**: K09 (闭包设计), K17 (性能实测)

## R11: 2026-06-03-m36-zero-copy-value-ref

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-m36-zero-copy-value-ref/`
- **状态**: archived
- **内容**: M36 零拷贝 ValueRef 实施
- **关联知识**: K18 (性能与局限)

## R12: 2026-06-04-m19-datascan-path

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-04-m19-datascan-path/`
- **状态**: archived
- **内容**: M19 DataScan 路径实施
- **关联知识**: K19 (1.81x-2.44x 提速), K22 (数据页链表)

## R13: 2026-06-04-m21-page-visibility-map

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-04-m21-page-visibility-map/`
- **状态**: archived
- **内容**: M21 页面级 MVCC 实施
- **关联决策**: D11
- **关联知识**: K08, K09, K10, K12, K13

## R14: 2026-08-26-2026-08-25-ms06-t02-plancache-dashmap

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-08-26-2026-08-25-ms06-t02-plancache-dashmap/`
- **状态**: archived
- **内容**: MS06-T02 PlanCache DashMap + SQL 规范化（`HashMap + &mut self` → `DashMap + &self`；`normalize_sql_key` 公开函数：ASCII 折叠 + 空白折叠 + trim + 单引号 toggle 状态机；`Database.plan_cache: Arc<Mutex<PlanCache>>` → `Arc<PlanCache>`；`tests/plan_cache_test.rs` 7 集成测试 + 10 单测；T0 基线 clippy 归零 + 36 处表外 mechanical 修复）
- **关联能力 spec**: `plancache-key-normalization`（R1-R4）
- **基线**: 504 tests pass（487 基线 + 10 单测 + 7 集成测试）

## R15: 2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages/`
- **状态**: archived
- **内容**: MS06-T03 + MS06-T04 一并实施
  - **T03 (WAL 句柄复用)**: `WalWriter` 持 `Arc<std::sync::Mutex<std::fs::File>>` 单一持久句柄；5 个 IO 方法（`write_record` / `fsync` / `truncate_to` / `get_current_lsn` / `write_batch`）全部删除逐次 `OpenOptions::open`，改为 clone Arc → `spawn_blocking` → lock 内完成；错误语义与 LSN 文件位置语义保持；`tests/wal_handle_test.rs` 新增 4 测试（10K tx fd 净增量 < 10、LSN 偏移、truncate 后同句柄追加、4 任务并发一致）
  - **T04 (Pipeline 三阶段拆分)**: `pipeline::execute_inner` 279 行单函数 → 编排器 + `pub async fn parse_stage` / `pub async fn plan_stage` / `pub async fn execute_stage`；cache-hit 早退重复块删除；profiling 三段顶层计时（parse/plan/execute）替代旧 `parse_and_plan` 合并计时，子指标 `table_metadata_lookup` / `executor_creation` / `executor_execution` 由 `profiling: bool` 守卫；`#[cfg(test)] mod tests` 8 阶段单测；`benches/pipeline_stages_bench.rs` 三阶段独立 criterion bench
- **关联能力 spec**:
  - `wal-writer-handle-reuse`（R1-R4：句柄复用 / 错误语义 / LSN 语义 / fd 上界可验证）
  - `pipeline-stage-decomposition`（R1-R8：parse 终止 / plan 终止 / execute 终止 / cache-hit 跳过 / DML 事务包裹 / DDL 缓存失效 / 阶段级可测 / 三段顶层计时 / 独立 bench）
- **基线**: 516 tests pass（504 基线 + wal_handle 4 + pipeline 8 阶段单测）

## R16: 2026-08-26-2026-08-26-ms07-t01-schema-persistence

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/`
- **状态**: archived
- **关联里程碑**: MS07-T01（基础能力建设 / 系统表 `__tables` / `__columns` + Schema 页；最大单点）
- **Plan Review**: `accepted`（openspec-plan / 2026-08-26 18:58；RTM A1–A10 全部满足；11 项偏差 0 阻塞）
- **内容**:
  - 新增 `src/storage/catalog.rs`（~908 行）— `Catalog` 结构 + `bootstrap` / `open` / `insert_table` / `delete_table` / `scan_tables` / `scan_columns` / `update_table_tail` 7 方法 + 二进制行序列化 / 反序列化 + 链式 SlottedPage（`next_page_id` header 偏移 5..9） + 10 单元测试
  - `src/storage/btree/index_manager.rs` — 新增 `pub fn root_page_id()` 访问器 + `pub fn from_root(buffer_pool, root_page_id)` 路径（直接 `AtomicU64::new(root_page_id.0)`，不调 `BTree::new`）
  - `src/storage/data/table_manager.rs`（重写 ~345 行）— `new(buffer_pool, storage) -> Result<Arc<Self>>` async；`catalog: Arc<Catalog>` 字段 + `catalog()` 访问器 + `open_or_init()` 重建方法；`create_table` 末尾调 `catalog.insert_table`（失败时回滚 in-memory） + 保留名检查（`__tables` / `__columns` → `ReservedTableName`）；`drop_table` 同；新增 `write_tuple` 跨页同步 `data_page_tail`
  - `src/database.rs` — `TableManager::new(buffer_pool, storage).await?` + `open_or_init().await?`；新增 `pub async fn close()` 调 `buffer_pool.flush_all()`（schema 持久化必须显式落盘）
  - `src/executor/insert.rs` — `table_manager: Option<Arc<TableManager>>` + `with_table_manager(...)`；新路径走 `tm.write_tuple`，旧测试走 `write_tuple_to_data_page` fallback
  - `src/storage/error.rs` — 新增 `StorageError::ReservedTableName(String)` 变体
  - `src/storage/async_storage.rs` — 新增 `fn page_count(&self) -> u64` 方法（`TableManager::new` 据此分支 bootstrap/open）
  - `src/storage/page_format/tuple.rs` — `ColumnType` 加 `#[derive(Eq)]`
  - `src/storage/{mod,file_storage,data_page}.rs` — 适配签名
  - `src/transaction/manager.rs` — 适配签名
  - `src/{plan_cache.rs}` — 适配签名
  - `tests/table_manager_test.rs` — 6 个测试 `setup()` 加 `storage` + `.await`（API 兼容）
  - `tests/schema_persistence_test.rs`（新增 237 行 / 8 测试）— `test_create_table_writes_to_tables_page0` / `test_restart_recovers_table` / `test_restart_dml_works` / `test_drop_table_removes_from_catalog` / `test_restart_after_drop_table_gone` / `test_index_root_persists_across_restart` / `test_tables_is_reserved` / `test_data_page_tail_persists`
  - 14 个其他 test 文件批量改 `TableManager::new` 签名（plan_exec / executor / gc / mvcc_* / version_chain / concurrent / join / wal_* / btree_test / index_manager_test / pg_messages_test / plan_cache_test 等）
- **关联能力 spec**:
  - `schema-persistence`（7 个 Requirement / 14 个 Scenario）
    - R1: 系统表持久化 schema（New db bootstrap / Restart preserves DML / drop_table removes & persists）
    - R2: IndexManager::from_root path（from_root binds / from_root does not allocate）
    - R3: Reserved system table names（CREATE TABLE __tables rejected / DROP TABLE __tables rejected）
    - R4: data_page_tail persistence（Cross-page INSERT persists tail）
    - R5: page 0 / page 1 reservation（Fresh db allocates / Existing db recognizes）
    - R6: Catalog operations under write lock（Concurrent CREATE TABLE serialized / Catalog write failure leaves HashMap consistent）
    - R7: System tables bypass MVCC and WAL（Reads independent of transaction / DDL no WAL records）
- **基线**: 534 tests pass（516 基线 + 10 catalog 单测 + 8 schema 集成测试）
- **关键偏差**（已记录于 Act Response，0 阻塞）:
  - `InsertExecutor` `Option<Arc<TableManager>>` + fallback（~40 处旧调用零修改通过）
  - `update_table_tail` 用 append+delete 而非 in-place（SlottedPage 无 in-place API）
  - `Database::close()` 新增（restart 测试必需）
  - `test_data_page_tail_persists` 200 行直写替代 300 SQL INSERT（隔离 WAL buffer 满干扰）
  - `AsyncStorage::page_count` 新增 trait 方法（bootstrap/open 分支必需）
- **遗留 Minor**（划归后续 change）:
  - K05 recovery 静默吞错（`src/wal/recovery.rs:146-148/162-165/174-177`）— 下一 change 修复
  - MS07-T02 drop_table 物理页释放 — 独立 change
  - R-5：`IndexManager::from_root` 不验证 page 内容 — MS07-T02 处理
  - R-4：SQL parser 层保留名拦截未做（`TableManager::create_table` 入口已覆盖）
  - `tests/recovery_e2e_test.rs::test_data_pages_survive_restart` workaround 可去掉 — 随 K05 修复
  - `rtsql.db` / `:memory:.wal` 旧文件不向后兼容 — pre-release 阶段可接受
- **Persisted Evidence**: none（Plan 阶段声明；6 项验证命令均低成本可重跑；本审计已新鲜重跑）

## R17: MS08 bench 基线采集与前后对比判读 Runbook

- **类型**: runbook
- **路径**: `.claude/runbooks/ms08-bench-comparison.md`
- **日期**: 2026-09-05
- **用途**: MS08 各 T（及后续同类性能优化）实施前后的 criterion 基线落盘、strace syscall 计数对比与 bench 变化的因果判读（含 `--` 透传、strace 5.16 syscall 名、对照组判定、机制归因等已验证要点）
- **状态**: active

## R18: 产品可用性与 CLI 形态差距分析

- **类型**: analysis
- **路径**: `.claude/analysis/usability-gap-cli-form.md`
- **日期**: 2026-09-05（revision `709c85d`；同日四轮：形态/SQL 面 + 文件模型/隔离 + 非交互 CLI 与密钥/分析 + 安装分发实测）
- **用途**: 评估 RTsql 距"可用且好用"的差距并沉淀应用层设计空间（产品形态现状 / SQL 语义广度 / CLI 交互缺口 / 文件模型与初始化 / 多库隔离 / 非交互命令面 + sudo 式密钥 + 集中存储 + 分析能力 + 安装分发与格式版本策略 / 缺陷层+应用层双轨规划输入）；用户形态决策（非交互 CLI 数据库优先）下的后续 milestone/change 规划上下文
- **状态**: active
