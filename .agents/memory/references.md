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
  | tokio | 1.x | https://docs.rs/tokio | async 运行时（rt-multi-thread, macros, sync, time, net, fs, io-util, signal） |
  | sqlparser-rs | 0.44 | https://docs.rs/sqlparser | SQL 解析 |
  | async-trait | 0.1 | https://docs.rs/async-trait | async trait 支持 |
  | clap | 4 | https://docs.rs/clap | CLI 框架（derive/env，MS10-T01） |
  | clap_complete | 4 | https://docs.rs/clap_complete | shell 补全生成（bash/zsh/fish，MS17-T03） |
  | thiserror | 1.0 | https://docs.rs/thiserror | 错误类型派生 |
  | anyhow | 1.0 | https://docs.rs/anyhow | 错误处理 |
  | futures | 0.3 | https://docs.rs/futures | 异步原语 |
  | tokio-util | 0.7 | https://docs.rs/tokio-util | Tokio 工具（rt） |
  | serde | 1.0 | https://docs.rs/serde | 序列化框架 |
  | serde_json | 1.0 | https://docs.rs/serde_json | JSON 输出 |
  | rand | 0.8 | https://docs.rs/rand | 随机数 |
  | lru | 0.12 | https://docs.rs/lru | LRU 缓存（PlanCache） |
  | crc32fast | 1.4 | https://docs.rs/crc32fast | WAL CRC32 校验 |
  | dashmap | 6 | https://docs.rs/dashmap | 并发 HashMap（BufferPool vis_map/loading_locks） |
  | csv | 1 | https://docs.rs/csv | CSV 导入（`import --csv`，MS10-T05） |
  | argon2 | 0.6 | https://docs.rs/argon2 | Argon2id 密钥派生（MS17-T01） |
  | aes-gcm | 0.11 | https://docs.rs/aes-gcm | 页级 AES-256-GCM 加密（MS17-T01） |

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
  | libc | 0.2 | https://docs.rs/libc | 优雅停机测试信号注入（kill/SIGINT/SIGTERM，MS10-T02） |

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

## 已迁移的旧 analysis 文档（指针）

## R06: M19 DataScan 路径分析（已实施）

- **类型**: analysis（已实施迁移）
- **状态**: completed；[ARCHIVED 2026-09-24] `.agents/analysis/archive/m19-datascan-path.md`（ARC-202609242151 Artifact-Archive）
- **原因**: DataScan 已实施并归档到 M19 change；分析内容沉淀于 R28 分析文档（K19 实测性能，2026-09-24 K/D 退役迁移）与 project-model M02（数据页链表；原文所引 M22 为旧体系编号，现行 project-model 无此条）
- **Legacy**: R007

## R07: M21 页面级 MVCC 遗留项分析（已解决）

- **类型**: analysis（已实施迁移）
- **状态**: completed；[ARCHIVED 2026-09-24] `.agents/analysis/archive/m21-page-visibility-incomplete.md`（ARC-202609242151 Artifact-Archive）
- **原因**: M21 遗留项 (DELETE mark_deleted + 惰性 set_all_visible + benchmark) 全部完成（commit `78a3b01`）；现行权威为 spec `mvcc-tombstone-visibility`（mark_deleted 机制已经 MS09 墓碑 slot 化取代）与 project-model M10/M17（当前约束）；原 K12/K13 随 2026-09-24 K/D 退役入清理 carrier
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
- **内容**: MS06-T03 + MS06-T04 一并实施——T03 `WalWriter` 持 `Arc<Mutex<File>>` 单一持久句柄（5 个 IO 方法去逐次 open，错误与 LSN 语义保持；`tests/wal_handle_test.rs` 4 测试）；T04 `pipeline::execute_inner` 279 行 → 编排器 + parse/plan/execute 三 pub stage + profiling 三段计时 + 8 阶段单测 + 三阶段 criterion bench（文件级明细见归档 carrier）
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
- **内容**: 新增 `src/storage/catalog.rs`（~908 行，7 方法 + 二进制行序列化 + 链式 SlottedPage，10 单测）+ `IndexManager::from_root`/`root_page_id` + `TableManager` 重写（async new/`open_or_init`/保留名检查/跨页 `write_tuple`）+ `Database::close()` + `InsertExecutor` `Option<Arc<TableManager>>` + `AsyncStorage::page_count` + `StorageError::ReservedTableName`（文件级明细见归档 carrier）；`tests/schema_persistence_test.rs` 8 测试 + 14 个其他测试文件批量签名适配
- **关联能力 spec**: `schema-persistence`（7 Requirement / 14 Scenario：R1 系统表持久化 / R2 from_root / R3 保留名 / R4 tail 持久化 / R5 页 0/1 保留 / R6 catalog 写锁 / R7 系统表旁路 MVCC 与 WAL，明细见 spec）
- **基线**: 534 tests pass（516 基线 + 10 catalog 单测 + 8 schema 集成测试）
- **关键偏差**（已记录于 Act Response，0 阻塞）: InsertExecutor Option+fallback（~40 处旧调用零修改）、`update_table_tail` append+delete（SlottedPage 无 in-place API）、`Database::close()` 新增、tail 测试直写隔离 WAL buffer 干扰、`AsyncStorage::page_count` 新增——明细见归档 carrier
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
- **路径**: `.agents/runbooks/ms08-bench-comparison.md`
- **日期**: 2026-09-05
- **用途**: MS08 各 T（及后续同类性能优化）实施前后的 criterion 基线落盘、strace syscall 计数对比与 bench 变化的因果判读（含 `--` 透传、strace 5.16 syscall 名、对照组判定、机制归因等已验证要点）
- **状态**: active

## R18: 产品可用性与 CLI 形态差距分析

- **类型**: analysis
- **路径**: `.agents/analysis/usability-gap-cli-form.md`
- **日期**: 2026-09-05（revision `709c85d`；同日四轮：形态/SQL 面 + 文件模型/隔离 + 非交互 CLI 与密钥/分析 + 安装分发实测）
- **用途**: 评估 RTsql 距"可用且好用"的差距并沉淀应用层设计空间（产品形态现状 / SQL 语义广度 / CLI 交互缺口 / 文件模型与初始化 / 多库隔离 / 非交互命令面 + sudo 式密钥 + 集中存储 + 分析能力 + 安装分发与格式版本策略 / 缺陷层+应用层双轨规划输入）；用户形态决策（非交互 CLI 数据库优先）下的后续 milestone/change 规划上下文
- **状态**: active

## R19: MS10-T03 文件格式头——打开链路、页寻址约束与放置方案

- **类型**: analysis
- **路径**: `git show a61891f:.agents/analysis/ms10-t03-file-format-header.md`
- **日期**: 2026-09-08（revision `268fa4f`）
- **用途**: MS10-T03（文件 magic/格式版本头）Plan 的实现调查输入——格式头校验插入点（`FileStorage::open` 锁后、页解析前）、`to_offset` 唯一偏移源与 catalog 保留页 0/1 约束、放置方案对比（前缀头偏移平移 vs 超级页）、非 RTsql 文件实测行为基线（8192B 垃圾文件 panic→abort exit 134）、CLI open 错误映射（exit 1/4）与 MS12-T01 头字段需求、测试影响面（storage_test / drop_table_free_test / file_storage_io_test 裸布局断言）
- **状态**: completed；[ARCHIVED 2026-10-20]（Artifact-Archive，原 `.agents/analysis/ms10-t03-file-format-header.md`）
- **原因**: MS10-T03 已实施并沉淀于规格语料 `database-file-format-header` 与 SNAPSHOT 文件头模块边界；计划期调查输入已消费，方案原文经取回命令可解析

## R20: MS10-T05 生命周期子命令实现上下文

- **类型**: analysis
- **路径**: `git show a61891f:.agents/analysis/ms10-t05-lifecycle-subcommands.md`
- **日期**: 2026-09-09（revision `a5b0a5f`；实施代码基线 `8827700`）
- **用途**: MS10-T05（`new/list/schema/dump/restore/import --csv`）Plan 的实现调查输入——`CliArgs` 扁平入口重构点与裸名/子命令名冲突边界、`execute_command_inner` 两阶段信号编排复用、`Database`/`Catalog` 可复用 API（系统表不可 SQL 查询、schema 必须走 `catalog().scan_tables/scan_columns`）、双 `ColumnType` 体系与 String(255) 固定转换决定的 dump 保真边界、csv 依赖缺口、`resolve_db_path` 目录 helper 提取点、测试模式与退出码矩阵零扩展结论
- **状态**: completed；[ARCHIVED 2026-10-20]（Artifact-Archive，原 `.agents/analysis/ms10-t05-lifecycle-subcommands.md`）
- **原因**: MS10-T05 已实施并沉淀于规格语料与 SNAPSHOT 生命周期子命令面（后续演进见 MS17/MS19/MS28/MS29 各 change）；计划期调查输入已消费，原文经取回命令可解析

## R21: ISS01 min_create_tx_id=0 毒化 all-invisible 快路径

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS01-min-create-tx-id-zero-poisoning.md`
- **日期**: 2026-09-14
- **用途**: 写路径首建页级可见性条目时 `min_create_tx_id` 被 `or_default()` 钉 0、`find_visible_version` all-invisible 快路径对该页永久失效（保守方向、损失优化）的缺陷台账；MS08 实测域量化与修复裁定的输入（002-rework Plan Review F4(c) 裁定残留）
- **状态**: closed（2026-09-23 fixed → MS17-T02 change 归档，哨兵语义双向闭合 0/MAX 毒化）[ARCHIVED 2026-09-24]

## R22: ISS02 IN×JOIN 子查询计划期误报 Subquery returns multiple columns

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS02-in-subquery-join-plan-rejection.md`
- **日期**: 2026-09-14
- **用途**: `IN (SELECT … JOIN …)` 计划期误报「requires single column」的缺陷台账——`get_subquery_first_column` 无 Join/NLJ 形态臂（等值 Hash 同源预存，`subquery.rs:437` fallback）、关联参数仅扫 WHERE 与 WHERE+JOIN 拒绝面的三面叠加边界；IN×JOIN 能力裁定与诊断文案修正决策的输入（MS09 Iteration 001 000-initial Plan Review F5 裁定残留）
- **状态**: closed（2026-09-23 fixed → MS17-T02 change 归档，最小诚实化拒绝；能力解锁留 improvement 候选）[ARCHIVED 2026-09-24]

## R23: ISS03 标量子查询 select-list 输出表头与行形状不一致

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS03-scalar-subquery-header-shape-mismatch.md`
- **日期**: 2026-09-14
- **用途**: `get_plan_output_columns` SubqueryEval 臂未计入执行器插入的标量列、表头 N 列对 N+1 值行（I034 同族缺口，SubqueryEval 臂在其修复范围外，e51c4a3 即预存）的缺陷台账；标量子查询输出形状修复独立小 change 规划的输入（MS09 Iteration 002 000-initial Plan Review F4 裁定残留）
- **状态**: closed（2026-09-23 fixed → MS17-T02 change 归档，SubqueryEval 臂按 `result_column_index` 插列）[ARCHIVED 2026-09-24]

## R24: 进程内 close→reopen 数据库测试配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/in-process-reopen-testing.md`
- **日期**: 2026-09-23
- **用途**: 集成测试同进程重开库文件的固定步骤（显式作用域 + 显式 `close()` 释放 advisory 锁、tempdir 路径坑、RC 重开免抬水位）；WAL 恢复/checkpoint/隔离级别/加密 with-without key 重开类测试的执行配方（MS17-T02 Iter001/002 双 Iteration 实证）
- **状态**: active

## R25: RTsql workspace crate 化与微内核数据库形态探索分析

- **类型**: analysis
- **路径**: `.agents/analysis/workspace-crate-modularization.md`
- **日期**: 2026-09-24（revision `7364bc9` 工作区，含 MS13/MS17-T02 未提交实施）
- **用途**: 初版后长期方向（用户裁定 2026-09-24，路线 B workspace crate 化）的调查输入——模块依赖实测图谱与两条真实依赖环（database→pipeline→executor→database、storage↔transaction 经 VersionHeader）、词表熔接点（Value/PhysicalPlan 约 20 变体/ColumnType 磁盘绑定）与现成接缝（AsyncStorage/REGISTRY/火山树/IsolationLevel）、Response 错层（core 反向依赖 network）、目标 crate 拓扑与四刀迁移顺序草图（词表下沉→存储域→语言域→组装层+可选件）、风险清单（断环成本/测试矩阵×无 CI I051/spec 条件化/过度拆分告诫）、三种模块化模式开源先例（SQLite 编译宏/SurrealDB kv-* features/GlueSQL+DataFusion trait 接缝/Materialize+GreptimeDB+RisingWave workspace/PostgreSQL 扩展/FoundationDB 角色分解）；同日补充——定位裁定（异步·嵌入式·CLI 三分句 + 决策过滤器 + 模块化为手段非身份 + 对路线 B 排序影响，2026-09-24 用户裁定，含 async-native 稀缺性查证）与关联方向 I059 注记（跨库交互 ATTACH 式，身份过滤器三词全沾）
- **状态**: active

## R26: RTsql vs SQLite 跨引擎对比基准与资源测量 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/sqlite-compare-benchmark.md`
- **日期**: 2026-09-26（2026-09-24 初版；2026-09-26 MS18 Iteration 000/001 重测复跑并新增步骤 7）
- **用途**: RTsql 对 SQLite 的性能与资源快照采集——引擎级 criterion 三段（insert/pk lookup/full scan，`--noplot` 降时参）+ `estimates.json` 精确均值提取 + CLI 级 time -v 负载（单参数 128KB 上限、2000 行安全负载）/库文件体积/50 次 one-shot 时延/二进制体积；含双实例污染、pkill 自匹配、E2BIG 静默失败等实测失败处理；**同变更前后对照基线（步骤 7，2026-09-26 新增）**——工作区含未提交前序批次、不可整体回退时，`git stash push -- <仅本轮产品文件>` → release 构建存二进制 → `git stash pop` → `git diff --stat` 逐字核对复原，取同机背靠背 before/after 基线（含 pop 冲突与复原核验失败处理 8）；结果固化为双语 README「性能与资源对比」板块
- **状态**: active

## R27: 测试与基准诊断 Runbook（K20/K21/K34/K35 迁移）

- **类型**: runbook
- **路径**: `.agents/runbooks/test-bench-diagnosis.md`
- **日期**: 2026-09-24
- **用途**: Rust 测试/bench 过长与假死的症状分类诊断路径（死锁/无限循环/setup 过重三分）、criterion 基线纪律（`--save-baseline` 实施前留档）、独立 WAL bench 的 tempdir leak 模式与 bench 技巧集（共享 runtime/RTsqlDirect/Throughput/black_box/线程争用）；knowledge spec 退役迁移产物，基准对比操作另见 R17/R26
- **状态**: active

## R28: 引擎模式与历史知识沉淀（K/D 退役迁移）

- **类型**: analysis
- **路径**: `.agents/analysis/engine-patterns-legacy-knowledge.md`
- **日期**: 2026-09-24
- **用途**: knowledge spec 退役（2026-09-24 用户指令）后的现役踩坑根因、代码模式与历史性能实测数据集（K01-K04/K06-K09/K14-K19/K22-K33 按原编号逐字保留）；当前约束类 K10/K11/K38 已升格 project-model M17/M18，基准方法论已迁 R27，陈旧条目（K05/K12/K13/K36/K37）随 carrier 留档
- **状态**: active

## R29: ISS04 非键列 INSERT/UPDATE 写入值类型校验缺失（类型不匹配值静默持久化）

- **类型**: issue
- **路径**: `.agents/issues/ISS04-non-key-column-write-type-validation-missing.md`
- **日期**: 2026-09-25（关闭 2026-09-26）
- **用途**: 非键非唯一列的写入值与列声明类型全程无校验（String/Float 静默写入 INT 列，读回才暴露）的缺陷台账——`build_update`/`extract_insert_values` 无计划期类型门、执行器类型门仅覆盖 PK 与唯一列、`serialize_tuple` 无 schema 交叉校验；类型校验面立项裁定的输入（MS23 Iteration 001 Plan Review F4 裁定残留；唯一列与 PK 键列边缘已分别由 MS23 F1 修复与 MS16 收口）。**已关闭**：`fixed`——由 change `2026-09-25-ms24-write-surface-completion` Iteration 000（R2 写入值类型一致门）端到端修复，行为规格 `openspec/specs/sql-write-surface/spec.md` R2，测试 `tests/write_type_conformance_test.rs`；台账保留为该修复面（`ColumnTypeMismatch` 错误面、FLOAT 升格、dump/restore/import 通道覆盖）的来源与边界记录
- **状态**: closed（2026-09-26 fixed → change `2026-09-25-ms24-write-surface-completion` Iteration 000 R2 写入值类型一致门；台账保留为该修复面的来源与边界记录）

## R30: 文件对备份边界实测 Runbook（.db + .wal 配对行为矩阵）

- **类型**: runbook
- **路径**: `.agents/runbooks/file-pair-backup-boundary.md`
- **日期**: 2026-09-26
- **用途**: 「拷贝/移动/重命名 RTsql 数据库文件时哪些部分必须成对携带」的实测复核路径——kill -9 构造未 checkpoint 尾部态（`MAX_ARG_STRLEN` 128 KiB 约束、定时命中与准入检查、先快照后开原库纪律）、裸名副本陷阱（主命令对不存在路径新建空库）、7 形态行数矩阵（成对完整 / 单拷静默不完整 / 改名丢尾部）；备份方案裁定与存储层改动后回归备份语义使用
- **状态**: active

## R31: one-shot CLI 子进程见证配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/cli-subprocess-witnessing.md`
- **日期**: 2026-09-30
- **用途**: 集成测试中以子进程端到端见证 one-shot CLI（退出码/stderr 文案/文件副作用/存活期 `.wal` 采样）的操作配方——`RTSQL_HOME` 预建 `db/`、裸名实参不带后缀、SQL 负载按 `MAX_ARG_STRLEN` 128 KiB 定尺、stdout 丢弃或落文件防管道阻塞、stderr 落文件带进断言、deadline+kill 兜底；写作 CLI e2e 测试夹具或排障「exit 1 + 采样恒 0 / 假死 / ArgumentListTooLong」时使用（来源 MS26 Iteration 002，3 轮首跑失败实证收敛）
- **状态**: active

## R32: 外部评测缺陷核实（2026-10-12 批）

- **类型**: analysis
- **路径**: `.agents/analysis/2026-10-12-external-eval-defect-verification.md`
- **日期**: 2026-10-12（revision `44d04d9`，HEAD 构建复现）
- **用途**: 2026-10-12 外部评测 13 项反馈（P0×2/P1×3/P2×7/待核对×1）的逐项核实与机制落点——全部确认存在：二级索引等值扫描重复发射（`search_all_from_page_async` 等值分隔键无 break 双侧收集、共享子树成倍重访）+ 索引条目缺失（UPDATE 探针 KeyNotFound 证实，产生行待 Plan 定位）、import 自动建表后未 checkpoint 崩溃砖化（DDL 不入 WAL + catalog 脏页仅池内 + redo 硬拒绝，check 指引的恢复路径不可达）、派生表/视图内层别名与表达式项外层不可见（`building_subquery` 抑制面，CTE 通道不受影响）、超大 DEFAULT 建表残留半表（DDL 无原子性）、IN 子查询 SemiJoin 投影窄形状 × ORDER BY 非输出列 42703、help 泄漏内部任务标记（cli/mod.rs:107-114）、错误前缀双层包装（pipeline.rs:113 + error.rs:74）、schema/dump 类型短名渲染（column_type_sql）、BUSY_TIMEOUT env 错误只提 flag、多语句 JSONL、DDL affected_rows:0、加密库 .wal 脏窗口明文实证；含失败 UPDATE 索引面副作用、同步版 search_all 漏行潜伏变体两项范围外发现与 Issue 候选清单；后续修复 change 的实现调查输入
- **状态**: active

## R33: ISS05 二级索引等值扫描结果集错误（重复发射 + 索引条目缺失）

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS05-secondary-index-equality-scan-wrong-results.md`
- **日期**: 2026-10-12
- **用途**: 非唯一二级索引等值查询静默错结果的缺陷台账——`search_all_from_page_async` 等值分隔键无 break 双侧收集共享子树成倍重访（根因 Confirmed，`index_manager.rs:214-237`）+ 索引条目缺失（UPDATE KeyNotFound 探针证实、产生行 Unconfirmed）；含失败 UPDATE 索引面副作用与同步版漏行潜伏变体；修复 change 立项与验收（结果集与 DataScan 严格相等）的输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，闭区间下探 + `delete_exact` 同集合下探，叶链破坏与 scan_all 同面缺陷一并修复，双向严格相等与零副作用验收）

## R34: ISS06 import 自动建表后未 checkpoint 崩溃整库砖化

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS06-crash-after-auto-create-table-bricks-db.md`
- **日期**: 2026-10-12
- **用途**: DDL catalog 脏页仅池内 + WAL 已有该表已提交行 + SIGKILL 后全命令 exit 1、`check` 恢复指引不可达死锁的缺陷台账（DDL 不入 WAL/池驱逐时序/redo 硬拒绝机制链）；恢复策略设计决策与修复 change 立项的输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，DDL 语句边界 catalog 有序落盘（列行先、表行最后 = commit point + `flush_commit_point` 接线），建后即崩恢复可达）

## R35: ISS07 派生表与视图内层别名/表达式项外层不可见

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS07-derived-view-inner-alias-invisible.md`
- **日期**: 2026-10-12
- **用途**: FROM 派生表与视图通道 `building_subquery` 抑制面致内层列别名与计算列外层 42703/58000、CTE 通道正常的三通道不一致缺陷台账（含语料 `sql-views` R3 矛盾）；修复 change 立项与语料同步处置的输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，`building_from_derived` 放行扩 + 视图展开同型置位，语料 `sql-views` 矛盾同步收敛）

## R36: ISS08 CREATE TABLE 失败残留半表（DDL 无原子性）

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS08-create-table-failure-residual-half-table.md`
- **日期**: 2026-10-12
- **用途**: 超大 DEFAULT 页满建表失败后残留可写半表、同名重建被拒的缺陷台账；预检先行拒绝或 catalog 写入回滚的方案取舍与立项输入（与 ISS14 同场景交叉）
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，建表失败零残留（孤儿列行清理/`open_or_init` 裁剪/页释放），同名可重试）

## R37: ISS09 IN/EXISTS 子查询（SemiJoin）下 ORDER BY 非输出列 42703

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS09-semijoin-orderby-non-output-column.md`
- **日期**: 2026-10-12
- **用途**: SemiJoin/AntiJoin `output_columns` 按投影窄集装配与排序键校验联合致非输出列拒绝、字面量 IN 不受影响的缺陷台账；SemiJoin 形状对齐 design D10 或校验放行的方案取舍输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，SemiJoin/AntiJoin 基座全宽承载 + `sort_projection` 两消费面对齐，ORDER BY 非输出列可用）

## R38: ISS10 CLI help 文本泄漏内部任务标记

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS10-help-internal-marker-leak.md`
- **日期**: 2026-10-12
- **用途**: `--busy-timeout`/`--readonly` help 含「MS28 T1：」「MS26 T1：」「proposal 假设 10」（`cli/mod.rs:107-114` doc comment）的文案缺陷台账；小文案批修复输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，help 用户视角改写，内部标记移出用户可见面）

## R39: ISS11 错误消息前缀双层包装

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS11-error-prefix-double-wrapping.md`
- **日期**: 2026-10-12
- **用途**: `Parse error: Parse error:` 嵌套（`pipeline.rs:113` 与 `error.rs:74` 两层包装、既有断言锁定双层形态）的文案缺陷台账；单一包装层选定与断言校准输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，`parse_stage` 去前缀 + 漏斗点单层赋予，断言校准矩阵随批落地）

## R40: ISS12 schema/dump DDL 渲染类型短名

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS12-ddl-renders-short-type-names.md`
- **日期**: 2026-10-12
- **用途**: 声明 INTEGER/TEXT 渲染为 INT/STRING（`lifecycle.rs:1025` `column_type_sql`）的保真度缺陷台账（往返可达、数据无损）；渲染声明名或文档化规范化语义的决策输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，`column_type_sql` 标准长名（INTEGER/TEXT/FLOAT/BOOLEAN/DATE/TIMESTAMP），dump→restore 多代幂等）

## R41: ISS13 RTSQL_BUSY_TIMEOUT 非法值报错只引用 flag

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS13-busy-timeout-env-error-mentions-flag-only.md`
- **日期**: 2026-10-12
- **用途**: env 通道非法值报错渲染为 flag 参数错误、未点名 `RTSQL_BUSY_TIMEOUT` 的排障误导缺陷台账；与 `RTSQL_KEY`/`RTSQL_WAL_CHECKPOINT_BYTES` 同族 env 校验面一并核对输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，env 非法值代码侧校验点名变量与实际值（`RTSQL_KEY` 同族核对非误报不动））

## R42: ISS14 超大 DEFAULT 建表失败报 Page full 不指名默认值过大

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS14-oversized-default-error-message.md`
- **日期**: 2026-10-12
- **用途**: DEFAULT 字面量超 catalog 页内容量时拒绝文案为泛化 Page full 的错误质量缺陷台账（文档已承认边界、错误面未承载）；与 ISS08 预检方向合并处置输入
- **状态**: closed（2026-10-14 fixed → change `2026-10-13-e2e-verification-and-defect-closure` 归档，`catalog_column_row_fits` 计划期点名预检（ALTER 同款），与 ISS08 零残留同批收敛）

## R43: ISS15 table 渲染列宽未封顶致 ≥65536 字符单元格 panic（exit 101）

- **类型**: issue
- **路径**: `.agents/issues/archive/ISS15-table-renderer-panics-on-64k-cell.md`
- **日期**: 2026-10-14
- **用途**: `--format table`（TTY 默认）下行集单元格字符数 ≥ 65536 时 `render_table` 未封顶列宽触发 std::fmt 动态宽度上限 panic 的输出面崩溃缺陷台账；影响 SELECT/stats/sample/profile 全部 table 渲染面，JSON/CSV/TSV 不受影响
- **状态**: closed（2026-10-15 fixed → change `2026-10-14-iss15-table-truncation-and-combination-gaps` 归档，`render_table` 列宽封顶 64 字符截断 + 组合面 e2e 见证补齐；规格 1 MODIFIED + 2 ADDED 合并）

## R44: ISS16 相关标量子查询外参未绑定（WHERE 静默空集 / select list 塌缩或报错）

- **类型**: issue
- **路径**: `.agents/issues/ISS16-correlated-scalar-subquery-unbound.md`
- **日期**: 2026-10-15
- **用途**: 相关标量子查询外层参数绑定失效的正确性缺陷台账（文档宣称 correlated included）；WHERE 形态静默空集、select list 形态输出塌缩或 `Subquery returns multiple rows` 报错；最小 2 表库即复现，上游 subquery 测试全绿的覆盖缺口输入
- **状态**: closed（2026-10-15 fixed → change `2026-10-15-subquery-alias-binding-and-error-prefix` 归档，`extract_subquery_table_names` 声明限定符化〔别名优先 + 基表名并列保留的用户豁免裁定〕消除幻影相关参数源；规格 `sql-table-alias-resolution` +1 Requirement）

## R45: ISS17 EXISTS 内层带别名静默错误结果（空集/按位截断/全放行/全拒）

- **类型**: issue
- **路径**: `.agents/issues/ISS17-exists-aliased-inner-silent-wrong-results.md`
- **日期**: 2026-10-15
- **用途**: `EXISTS` 内层表带别名时四种静默错误形态（非相关谓词非首列恒空集、谓词首列按内层存活行数截断外层、相关经非 PK 列全放行，评测主库同形状全拒）的缺陷台账；与 ISS16/ISS18 同属子查询别名/参数绑定面，同批调查输入
- **状态**: closed（2026-10-15 fixed → 同 change，`extract_subquery_table_names` 声明限定符化 + 三执行器绑参未命中点名报错加固；同批覆盖缺口审计四形状〔NOT EXISTS/NOT IN 别名全放行、EXISTS JOIN 双臂别名恒空集、EXISTS Derived 别名截断〕一并修复并由形状覆盖矩阵钉住）

## R46: ISS18 IN 内层带别名 ∧ 索引谓词静默恒空集

- **类型**: issue
- **路径**: `.agents/issues/ISS18-in-aliased-inner-index-predicate-silent-empty.md`
- **日期**: 2026-10-15
- **用途**: `IN` 子查询「内层别名 ∧ 二级索引服务谓词」合取触发恒空集的缺陷台账（别名无索引、无别名有索引均正确）；IN 反连接 × 索引自动服务交叉面的正确性输入
- **状态**: closed（2026-10-15 fixed → 同 change，`extract_subquery_table_names` 声明限定符化后别名谓词不再走幻影相关路径；矩阵 `in_alias_with_secondary_index_predicate` + e2e 见证钉住）

## R47: ISS19 执行器错误文案双层前缀（ISS11 同族残余）

- **类型**: issue
- **路径**: `.agents/issues/ISS19-execution-error-double-prefix.md`
- **日期**: 2026-10-15
- **用途**: 执行器子查询路径 `Execution error: execution error:` 双层前缀文案缺陷台账（ISS11 修复 parse/plan 漏斗后的执行器漏网点）；错误面断言校准与顺手收敛输入
- **状态**: closed（2026-10-15 fixed → 同 change T3，`StorageError::ExecutionError` Display 去 thiserror 前缀、单层 `Execution error: ` 由 pipeline 漏斗唯一赋予；锚点 `subquery_test.rs:632` 校准 + e2e 端到端断言）

<!-- arc: ARC-202609092322 --> 1 条已归档 (2026-09-09) → openspec/changes/archive/2026-09-09-ARC-202609092322/proposal.md
