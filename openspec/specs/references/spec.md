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
- **状态**: completed；[ARCHIVED 2026-09-24] `.claude/analysis/archive/m19-datascan-path.md`（ARC-202609242151 Artifact-Archive）
- **原因**: DataScan 已实施并归档到 M19 change；分析内容沉淀于 R28 分析文档（K19 实测性能，2026-09-24 K/D 退役迁移）与 project-model M02（数据页链表；原文所引 M22 为旧体系编号，现行 project-model 无此条）
- **Legacy**: R007

## R07: M21 页面级 MVCC 遗留项分析（已解决）

- **类型**: analysis（已实施迁移）
- **状态**: completed；[ARCHIVED 2026-09-24] `.claude/analysis/archive/m21-page-visibility-incomplete.md`（ARC-202609242151 Artifact-Archive）
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

## R19: MS10-T03 文件格式头——打开链路、页寻址约束与放置方案

- **类型**: analysis
- **路径**: `.claude/analysis/ms10-t03-file-format-header.md`
- **日期**: 2026-09-08（revision `268fa4f`）
- **用途**: MS10-T03（文件 magic/格式版本头）Plan 的实现调查输入——格式头校验插入点（`FileStorage::open` 锁后、页解析前）、`to_offset` 唯一偏移源与 catalog 保留页 0/1 约束、放置方案对比（前缀头偏移平移 vs 超级页）、非 RTsql 文件实测行为基线（8192B 垃圾文件 panic→abort exit 134）、CLI open 错误映射（exit 1/4）与 MS12-T01 头字段需求、测试影响面（storage_test / drop_table_free_test / file_storage_io_test 裸布局断言）
- **状态**: active

## R20: MS10-T05 生命周期子命令实现上下文

- **类型**: analysis
- **路径**: `.claude/analysis/ms10-t05-lifecycle-subcommands.md`
- **日期**: 2026-09-09（revision `a5b0a5f`；实施代码基线 `8827700`）
- **用途**: MS10-T05（`new/list/schema/dump/restore/import --csv`）Plan 的实现调查输入——`CliArgs` 扁平入口重构点与裸名/子命令名冲突边界、`execute_command_inner` 两阶段信号编排复用、`Database`/`Catalog` 可复用 API（系统表不可 SQL 查询、schema 必须走 `catalog().scan_tables/scan_columns`）、双 `ColumnType` 体系与 String(255) 固定转换决定的 dump 保真边界、csv 依赖缺口、`resolve_db_path` 目录 helper 提取点、测试模式与退出码矩阵零扩展结论
- **状态**: active

## R21: ISS01 min_create_tx_id=0 毒化 all-invisible 快路径

- **类型**: issue
- **路径**: `.claude/issues/archive/ISS01-min-create-tx-id-zero-poisoning.md`
- **日期**: 2026-09-14
- **用途**: 写路径首建页级可见性条目时 `min_create_tx_id` 被 `or_default()` 钉 0、`find_visible_version` all-invisible 快路径对该页永久失效（保守方向、损失优化）的缺陷台账；MS08 实测域量化与修复裁定的输入（002-rework Plan Review F4(c) 裁定残留）
- **状态**: closed（2026-09-23 fixed → MS17-T02 change 归档，哨兵语义双向闭合 0/MAX 毒化）[ARCHIVED 2026-09-24]

## R22: ISS02 IN×JOIN 子查询计划期误报 Subquery returns multiple columns

- **类型**: issue
- **路径**: `.claude/issues/archive/ISS02-in-subquery-join-plan-rejection.md`
- **日期**: 2026-09-14
- **用途**: `IN (SELECT … JOIN …)` 计划期误报「requires single column」的缺陷台账——`get_subquery_first_column` 无 Join/NLJ 形态臂（等值 Hash 同源预存，`subquery.rs:437` fallback）、关联参数仅扫 WHERE 与 WHERE+JOIN 拒绝面的三面叠加边界；IN×JOIN 能力裁定与诊断文案修正决策的输入（MS09 Iteration 001 000-initial Plan Review F5 裁定残留）
- **状态**: closed（2026-09-23 fixed → MS17-T02 change 归档，最小诚实化拒绝；能力解锁留 improvement 候选）[ARCHIVED 2026-09-24]

## R23: ISS03 标量子查询 select-list 输出表头与行形状不一致

- **类型**: issue
- **路径**: `.claude/issues/archive/ISS03-scalar-subquery-header-shape-mismatch.md`
- **日期**: 2026-09-14
- **用途**: `get_plan_output_columns` SubqueryEval 臂未计入执行器插入的标量列、表头 N 列对 N+1 值行（I034 同族缺口，SubqueryEval 臂在其修复范围外，e51c4a3 即预存）的缺陷台账；标量子查询输出形状修复独立小 change 规划的输入（MS09 Iteration 002 000-initial Plan Review F4 裁定残留）
- **状态**: closed（2026-09-23 fixed → MS17-T02 change 归档，SubqueryEval 臂按 `result_column_index` 插列）[ARCHIVED 2026-09-24]

## R24: 进程内 close→reopen 数据库测试配方 Runbook

- **类型**: runbook
- **路径**: `.claude/runbooks/in-process-reopen-testing.md`
- **日期**: 2026-09-23
- **用途**: 集成测试同进程重开库文件的固定步骤（显式作用域 + 显式 `close()` 释放 advisory 锁、tempdir 路径坑、RC 重开免抬水位）；WAL 恢复/checkpoint/隔离级别/加密 with-without key 重开类测试的执行配方（MS17-T02 Iter001/002 双 Iteration 实证）
- **状态**: active

## R25: RTsql workspace crate 化与微内核数据库形态探索分析

- **类型**: analysis
- **路径**: `.claude/analysis/workspace-crate-modularization.md`
- **日期**: 2026-09-24（revision `7364bc9` 工作区，含 MS13/MS17-T02 未提交实施）
- **用途**: 初版后长期方向（用户裁定 2026-09-24，路线 B workspace crate 化）的调查输入——模块依赖实测图谱与两条真实依赖环（database→pipeline→executor→database、storage↔transaction 经 VersionHeader）、词表熔接点（Value/PhysicalPlan 约 20 变体/ColumnType 磁盘绑定）与现成接缝（AsyncStorage/REGISTRY/火山树/IsolationLevel）、Response 错层（core 反向依赖 network）、目标 crate 拓扑与四刀迁移顺序草图（词表下沉→存储域→语言域→组装层+可选件）、风险清单（断环成本/测试矩阵×无 CI I051/spec 条件化/过度拆分告诫）、三种模块化模式开源先例（SQLite 编译宏/SurrealDB kv-* features/GlueSQL+DataFusion trait 接缝/Materialize+GreptimeDB+RisingWave workspace/PostgreSQL 扩展/FoundationDB 角色分解）；同日补充——定位裁定（异步·嵌入式·CLI 三分句 + 决策过滤器 + 模块化为手段非身份 + 对路线 B 排序影响，2026-09-24 用户裁定，含 async-native 稀缺性查证）与关联方向 I059 注记（跨库交互 ATTACH 式，身份过滤器三词全沾）
- **状态**: active

## R26: RTsql vs SQLite 跨引擎对比基准与资源测量 Runbook

- **类型**: runbook
- **路径**: `.claude/runbooks/sqlite-compare-benchmark.md`
- **日期**: 2026-09-24
- **用途**: RTsql 对 SQLite 的性能与资源快照采集——引擎级 criterion 三段（insert/pk lookup/full scan，`--noplot` 降时参）+ `estimates.json` 精确均值提取 + CLI 级 time -v 负载（单参数 128KB 上限、2000 行安全负载）/库文件体积/50 次 one-shot 时延/二进制体积；含双实例污染、pkill 自匹配、E2BIG 静默失败等实测失败处理；结果固化为双语 README「性能与资源对比」板块
- **状态**: active

## R27: 测试与基准诊断 Runbook（K20/K21/K34/K35 迁移）

- **类型**: runbook
- **路径**: `.claude/runbooks/test-bench-diagnosis.md`
- **日期**: 2026-09-24
- **用途**: Rust 测试/bench 过长与假死的症状分类诊断路径（死锁/无限循环/setup 过重三分）、criterion 基线纪律（`--save-baseline` 实施前留档）、独立 WAL bench 的 tempdir leak 模式与 bench 技巧集（共享 runtime/RTsqlDirect/Throughput/black_box/线程争用）；knowledge spec 退役迁移产物，基准对比操作另见 R17/R26
- **状态**: active

## R28: 引擎模式与历史知识沉淀（K/D 退役迁移）

- **类型**: analysis
- **路径**: `.claude/analysis/engine-patterns-legacy-knowledge.md`
- **日期**: 2026-09-24
- **用途**: knowledge spec 退役（2026-09-24 用户指令）后的现役踩坑根因、代码模式与历史性能实测数据集（K01-K04/K06-K09/K14-K19/K22-K33 按原编号逐字保留）；当前约束类 K10/K11/K38 已升格 project-model M17/M18，基准方法论已迁 R27，陈旧条目（K05/K12/K13/K36/K37）随 carrier 留档
- **状态**: active

## R29: ISS04 非键列 INSERT/UPDATE 写入值类型校验缺失（类型不匹配值静默持久化）

- **类型**: issue
- **路径**: `.claude/issues/ISS04-non-key-column-write-type-validation-missing.md`
- **日期**: 2026-09-25（关闭 2026-09-26）
- **用途**: 非键非唯一列的写入值与列声明类型全程无校验（String/Float 静默写入 INT 列，读回才暴露）的缺陷台账——`build_update`/`extract_insert_values` 无计划期类型门、执行器类型门仅覆盖 PK 与唯一列、`serialize_tuple` 无 schema 交叉校验；类型校验面立项裁定的输入（MS23 Iteration 001 Plan Review F4 裁定残留；唯一列与 PK 键列边缘已分别由 MS23 F1 修复与 MS16 收口）。**已关闭**：`fixed`——由 change `2026-09-25-ms24-write-surface-completion` Iteration 000（R2 写入值类型一致门）端到端修复，行为规格 `openspec/specs/sql-write-surface/spec.md` R2，测试 `tests/write_type_conformance_test.rs`；台账保留为该修复面（`ColumnTypeMismatch` 错误面、FLOAT 升格、dump/restore/import 通道覆盖）的来源与边界记录
- **状态**: closed（2026-09-26 fixed → change `2026-09-25-ms24-write-surface-completion` Iteration 000 R2 写入值类型一致门；台账保留为该修复面的来源与边界记录）

<!-- arc: ARC-202609092322 --> 1 条已归档 (2026-09-09) → openspec/changes/archive/2026-09-09-ARC-202609092322/proposal.md
