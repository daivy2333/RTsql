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
- **内容**: 废弃 `openspec/specs/rules/`，规则合并到 AGENTS.md
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
- **内容**: MS06-T02 PlanCache DashMap + SQL 规范化（关联能力 spec `plancache-key-normalization` R1-R4；实施明细、基线与测试构成见归档 carrier）

## R15: 2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages/`
- **状态**: archived
- **内容**: MS06-T03 `WalWriter` 持久句柄 + T04 pipeline 三阶段分解与 bench（关联能力 spec `wal-writer-handle-reuse` R1-R4、`pipeline-stage-decomposition` R1-R8；明细见归档 carrier）

## R16: 2026-08-26-2026-08-26-ms07-t01-schema-persistence

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/`
- **状态**: archived
- **关联里程碑**: MS07-T01
- **内容**: schema 持久化——`src/storage/catalog.rs` + TableManager 重写 + `Database::close()` 等（关联能力 spec `schema-persistence` 7 Requirement；Plan Review accepted；关键偏差与遗留 Minor 清单见归档 carrier）

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
- **路径**: `git show e9c7735:.agents/issues/ISS04-non-key-column-write-type-validation-missing.md`
- **日期**: 2026-09-25（关闭 2026-09-26）
- **用途**: 非键非唯一列的写入值与列声明类型全程无校验（String/Float 静默写入 INT 列，读回才暴露）的缺陷台账——`build_update`/`extract_insert_values` 无计划期类型门、执行器类型门仅覆盖 PK 与唯一列、`serialize_tuple` 无 schema 交叉校验；类型校验面立项裁定的输入（MS23 Iteration 001 Plan Review F4 裁定残留；唯一列与 PK 键列边缘已分别由 MS23 F1 修复与 MS16 收口）。**已关闭**：`fixed`——由 change `2026-09-25-ms24-write-surface-completion` Iteration 000（R2 写入值类型一致门）端到端修复，行为规格 `openspec/specs/sql-write-surface/spec.md` R2，测试 `tests/write_type_conformance_test.rs`；台账保留为该修复面（`ColumnTypeMismatch` 错误面、FLOAT 升格、dump/restore/import 通道覆盖）的来源与边界记录
- **状态**: closed（2026-09-26 fixed → change `2026-09-25-ms24-write-surface-completion` Iteration 000 R2 写入值类型一致门；台账保留为该修复面的来源与边界记录）[ARCHIVED 2026-10-25]

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

## R48: M19 分批全量回归执行配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/m19-batched-full-regression.md`
- **日期**: 2026-10-23（2026-10-24 补充批计划与对账配方；**2026-10-25 补充等分批次规则与两个新实例**）
- **用途**: 给出「全量回归通过」结论时的分批执行配方（公共规则 M19）——`--lib --bins` 一批 + `tests/*.rs` 按文件名字典序每 10 个一批，每批一次独立 `cargo test … -- --test-threads=1`、批间顺序执行、同一时刻只跑一批，逐批记录 passed/failed/ignored 计数；含「cargo 在首个失败目标后中止该批后续目标」这一实测坑与失败复核方式；2026-10-24 补充**批计划生成（剩余 ≤12 取剩余、否则取 11）**、**覆盖对账（missing / extra / duplicate 三项均为 0）**与**总数对账（本次终态 − 基线终态 = 新增用例数逐项归因；「134 目标 / 135 条 `test result:` 行」两数各自成立）**三张对账表与批次表记录形态（来源 MS33 Iteration 000，14 批 2151/0/2 实证；MS34 Issue 修复批 Iteration 001，13 批 2171/0/2 实证，差额 +8 = 5 + 3）；**2026-10-25 补充「等分批次规则」**（`B = ceil(文件数 / 12)`；文件按字典序切片，前 `B×12 − 文件数` 批各取 11 个、其余各取 12 个 ⇒ 每批必落在 8–12 且不重不漏；等价表述：目标数 ÷ 批数得到的每批规模必须落在 8–12）与**两个新实例**（MS35 收尾 14 批 / 135 目标 / 2183-0-2，其末批只有 2 个目标、超出 8–12 的事实按原样记录；缺陷修复 change 收尾 14 批 / 143 目标 / 2231-0-2，覆盖对账 missing/extra/duplicate 全 0，目标数 143 与 `test result:` 行数 144 的口径说明，基线 2192/0/2 已失效后的差额归因 32 + 7 = 39）；另记录旧切分算法在特定文件数下会产出越界末批的坑
- **状态**: active

## R49: 进程内 checkpoint 前崩溃的复现与判别配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/in-process-crash-simulation.md`
- **日期**: 2026-10-23
- **用途**: 在同一进程内构造「checkpoint 之前进程结束」的持久化状态并观察重开结果的测试配方——崩溃助手形态 `wal_buffer.shutdown()` 后 `drop(db)` 不 `close`，两段会话（先 `close()`+`drop` 让行落盘且 WAL 截断，再执行被观察语句并崩溃），以及关键判别点「WAL 重放按 logical_id 幂等补回尚在 WAL 里的行，会掩盖持久化缺口，使用例改前即通过」；含清零索引树页导致重开栈溢出 abort 的实证形态；写 DROP/TRUNCATE 类语句的崩溃语义见证或判别「缺口是否真实存在」时使用（来源 MS33 Iteration 000，7 个崩溃用例实证）
- **状态**: active

## R50: ISS20 取消 + 回滚下索引写侧无原子性（B 树结构损坏致进程 abort；键残留在主键索引）

- **类型**: issue
- **路径**: `git show e9c7735:.agents/issues/ISS20-index-write-side-atomicity.md`
- **日期**: 2026-10-24（关闭 2026-10-24）
- **用途**: 取消 + 回滚后索引写侧无原子性的缺陷台账——`IndexManager::insert` 把正向条目（`spawn_blocking` 内完成、tokio blocking 任务不可取消）与反向映射（await 之后才写）分两段写，`IndexManager::delete` 的合并/换根与 `insert` 之间无写者串行化，根页 id 与树内容不在同一临界区（E1：B 树结构损坏 → 后续索引插入沿内部节点子指针 0 无限下降 → 栈溢出 → 进程 SIGABRT；机制经读码确认、复现为竞态依赖）；`abort_cleanup_versions` 只按反向映射分桶 → 有正向条目、反向映射缺失的行被误判为墓碑而跳过（E2：首个键之外的键残留占位，同键再插报 `DuplicateKey`，行不可见；显式与隐式路径、RR 与 RC 同形）；存储/索引侧后续 change 的修复输入与回归判据来源（键释放的行为规格随该 change 进入 `openspec/specs/`）；MS34 规格收窄时留下的「键的释放不属本域保证面」已知边界注记的缺陷本体。**已关闭**：由 change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 000 端到端修复（写单元原子化 + 写侧串行化 + 取消/回滚释放全部键 + 索引结构自洽），行为规格 `openspec/specs/index-write-side-atomicity/spec.md`（4 Requirement / 5 Scenario）；台账保留为该修复面的来源与边界记录
- **状态**: closed（2026-10-24 fixed → change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 000「索引写侧原子性」；行为规格 `openspec/specs/index-write-side-atomicity/spec.md` 4 Requirement / 5 Scenario；台账保留为该修复面的来源与边界记录）[ARCHIVED 2026-10-25]

## R51: ISS21 被放弃的 ALTER TABLE DROP COLUMN 后目录已窄化、数据面仍为混合宽度（DDL 目录变更不在事务内）

- **类型**: issue
- **路径**: `git show e9c7735:.agents/issues/ISS21-ddl-catalog-transactional.md`
- **日期**: 2026-10-24（关闭 2026-10-24）
- **用途**: DDL 目录变更的事务化缺陷台账——`alter_drop_column` 先改目录（列行改写 + 列数更新）并 `flush_all`、窄化不在事务边界内，事务只覆盖随后的逐行 DELETE + INSERT 数据面改写，被放弃的 DDL 语句留下「目录已窄化 + 数据面仍为混合宽度」的不一致形态（后续查询按窄化后的目录解释旧数据）；MS34 实施期未重新复现，最短可观察步骤由 MS34 Plan Review 补齐（放弃 `ALTER TABLE t DROP COLUMN v` → 触发回收 → 重开库 → 对比目录列数与行宽）；DDL 目录事务化主题后续 change 的修复输入，与 MS36「历史库与 dump 兼容」相邻排期、由 milestone-planner 统一裁定。**已关闭**：由 change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 端到端修复（目录窄化延后到提交点：`PendingColumnDrop` + `apply_column_drop`，先列行、后列数、再切 live meta；被放弃/失败的 DDL 不留目录面变更；放弃后重开库目录与数据面一致），行为规格 `openspec/specs/ddl-catalog-transactional/spec.md`（3 Requirement / 4 Scenario）；台账保留为该修复面的来源与边界记录
- **状态**: closed（2026-10-24 fixed → change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001「DDL 目录事务化」；行为规格 `openspec/specs/ddl-catalog-transactional/spec.md` 3 Requirement / 4 Scenario；台账保留为该修复面的来源与边界记录）[ARCHIVED 2026-10-25]

## R52: ISS22 完成的 ALTER TABLE DROP COLUMN 在数据页写满后丢行（多页改写丢行）

- **类型**: issue
- **路径**: `git show e9c7735:.agents/issues/ISS22-drop-column-data-page-full-loses-rows.md`
- **日期**: 2026-10-24
- **用途**: 完成路径的删列在数据页写满后静默丢行的缺陷台账——`alter_drop_column` 的逐行改写循环中 DELETE 腿（写墓碑到 `old` 元数据）与 INSERT 腿（写窄化行到 `new_meta`）各持一条独立的 `data_page_tail` 却指向同一条物理页链，页写满时两腿各自新分配页并互相覆盖链上 `next` 指针 ⇒ 先前分配的页成为孤儿（实测 40→25、60→10、80→50、100→35 行，N≤30 行数完整）；根因位置 `src/storage/data/table_manager.rs:897-1007` 与 `src/storage/data_page.rs:18`；数据面改写域后续 change 的修复输入与「多页改写行数守恒」的回归判据来源，与 MS35「空闲页跨重启复用」相邻排期建议（由 milestone-planner 统一裁定）。**已关闭**：由 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001「DDL 改写一致性」端到端修复（DELETE 腿与 INSERT 腿统一经同一条尾游标 `self.write_tuple(&old, …)`，改写后 `*new_meta.data_page_tail` 同步为 `*old.data_page_tail`），行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md`；回归承载 `tests/ddl_rewrite_consistency_test.rs`（修复前 RED `got 25 of 40` → 修复后 6/0）；台账保留为该修复面的来源与边界记录
- **状态**: closed（2026-10-25 fixed → change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001「DDL 改写一致性」；行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md`；台账保留为该修复面的来源与边界记录）[ARCHIVED 2026-10-25]

## R53: ISS23 B 树插入路径无限递归致栈溢出（约 300 行以上触发，进程 abort）

- **类型**: issue
- **路径**: `git show e9c7735:.agents/issues/ISS23-btree-insert-infinite-recursion-stack-overflow.md`
- **日期**: 2026-10-24
- **用途**: 行数约 300 以上时 B 树插入路径无限递归 → 栈溢出 → SIGABRT（signal 6）致整个测试进程 abort 的缺陷台账——gdb 现场显示 35+ 层递归帧停在 `src/storage/btree/btree.rs:327` 的内部节点下降递归点，同一 `page_id`、同一 key 指针 ⇒ 下降形成环（机制待独立复核，该文件未被承载 change 修改）；与 ISS20 的 E1 边界（E1 是取消落在「结构变更已生效、根页 id 未发布」窗口后的树损坏，本条出现在完成路径、由行数规模触发，与取消无关）明确分开，需在承载 change 中独立判定二者是否同一根因的不同表现；存储 / 索引域后续 change 的修复输入，与 MS35 相邻排期建议（由 milestone-planner 统一裁定）。**已关闭（归因更正）**：承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 000「索引页归属」复核确认本条**不是独立成因**，而是 B-2（`btree.rs:610` `merge_leaves` → `:672` `handle_child_merge` 同一页被释放两次 ⇒ 同一页号进空闲列表两次并被交给两个对象）的**下游后果**；修复 B-2（移除重复释放 + 删除 `MergeInfo.freed_page_id` 字段 ⇒ 结构性防回归）后栈溢出不再复现（250 行 / 625 行均干净），另加 `BTree::insert_into_page` 下降路径成环守卫作为第二道防线（成环 ⇒ 返回 `StorageError::Internal`）；行为规格 `openspec/specs/index-page-ownership/spec.md`；台账保留为该修复面的来源与归因更正记录
- **状态**: closed（2026-10-25 fixed → change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 000「索引页归属」；归因更正为 B-2 的下游后果；行为规格 `openspec/specs/index-page-ownership/spec.md`；台账保留为该修复面的来源与归因更正记录）[ARCHIVED 2026-10-25]

## R54: ISS24 被放弃的 DDL 语句隐藏了它开始改写的那一行

- **类型**: issue
- **路径**: `git show e9c7735:.agents/issues/ISS24-abandoned-ddl-hides-rewritten-row.md`
- **日期**: 2026-10-24
- **用途**: 被放弃的 DDL 语句在回收后让「它已开始改写的那一行」从扫描结果中消失的缺陷台账——被放弃语句已为该行写下墓碑，回收把该墓碑标为 aborted（`src/transaction/manager.rs` 的 `abort_cleanup_versions` → `mark_aborted`），但该行仍不出现在扫描结果里（实测 30 行 → 29 行、2000 行 → 1999 行，缺 `id=2`；承载 change 修复前后同为 29/30）；中止 / 回收语义域后续 change 的修复输入，需与 MS34 已验收的库属隐式事务回收语义对齐。**已关闭**：由 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001「DDL 改写一致性」端到端修复——四层判别读数把缺失行钉到**第 4 层（墓碑未中和）+ 第 2 层（主键条目被删未还原）**，M1（链可达性）排除，M2（`pointed` 抑制集合不识别 aborted 标记）作为独立缺陷一并修复；修复形态为墓碑改为「无链指针写入 → 注册 → 链接 → 索引清理」的写入次序，`abort_cleanup_versions` 新增 `versions_created_by`（按创建者事务 id 取回被放弃事务创建但从未注册的版本，带环守卫）；行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md` 与 `openspec/specs/mvcc-tombstone-visibility/spec.md`；台账保留为该修复面的来源与边界记录
- **状态**: closed（2026-10-25 fixed → change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001「DDL 改写一致性」；行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md` + `openspec/specs/mvcc-tombstone-visibility/spec.md`；台账保留为该修复面的来源与边界记录）[ARCHIVED 2026-10-25]

## R55: ISS25 DDL 提交点窗口内的位置性错读（窄 tuple 在未移位目录下按位置取值）

- **类型**: issue
- **路径**: `.agents/issues/ISS25-ddl-commit-point-positional-misread.md`
- **日期**: 2026-10-26
- **用途**: DDL 提交完成、目录应用尚未发生的窗口内按位置错读窄 tuple 的缺陷台账——`deserialize_tuple_width`（`src/storage/page_format/tuple.rs:189-222`）按位置取值、尾部补 `Value::Null`、不校验列序，全部读路径经它 ⇒ 2 宽元组在未移位的 3 列目录下读作 `[id, w, NULL]`（`v` 列读到 `w` 的值），影响全部已改写的行且无声；**登记时的措辞「未实跑复现」已不成立——可达性已由 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 003 实跑坐实**：变体 (a)「窗口内并发读」**已闭合**（引入纯内存的窄化在途状态，读路径在窗口内**显式拒绝**：用户实际所见 `Execution error: table 't' is being modified by an in-flight ALTER TABLE DROP COLUMN; read refused`，常态化用例已落地），变体 (b)「窗口内崩溃后重开并查询」**仍开放**（在途状态纯内存、崩溃即消失 ⇒ 重开后目录仍广告旧列数而数据面已是窄化后的已提交行 ⇒ 读仍按位置错位，实测重开后读出 `[Int(1), Int(100), Null], …`）；闭合变体 (b) 需要**持久标记**（改磁盘格式 ⇒ 破坏 MS36 的 G1/G2/G3 兼容面）或**移动提交点**（与 `ddl-catalog-transactional` R1「被放弃的 DDL 不留目录变更」冲突）⇒ 属范围外的设计决定；行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md`（Requirement「改写在途期间读路径不静默错读」，四个 Scenario 全部限定在**进程内**窗口 ⇒ 与已实现行为一致）
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R56: 索引写侧原子性的确定性复现配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/index-write-side-atomicity-repro.md`
- **日期**: 2026-10-24（**2026-10-25 补充锚点写持久化判定的取样点结论**）
- **用途**: 确定性复现「取消 / 回滚落在索引写单元中间」的配方——写侧暂停点夹具 `IndexManager::arm_write_pause(WritePausePosition::AfterStructureChangeBeforeRootPublish)`（语义位置固定为「结构变更已生效、根页 id 尚未发布」，修复前后不变；handle 提供到达计数 / 放行 / `disarm()`）+ `tests/index_write_atomicity_test.rs` 的 5 用例（插 92 行填满单叶 → 装载暂停点 → 后台插第 93 个键触发换根 → 轮询到达 → 丢弃语句 future → 放行并卸载 → 下一条写语句触发回滚 / 回收 → 断言键可达性 / 重复键 / 条目数 / 完整性）；含修复前 RED 形态（3/3 稳定命中产品断言）与修复后 GREEN、以及夹具失控兜底是 10s timer（触发即失败、不是行为判据）的说明；**2026-10-25 补充「锚点写是否已持久的取样点」结论**（取样点必须选在能观察到「该页仍为脏」的位置 ⇒ 此刻「仍脏」等价于「其写尚未被任何刷盘写出」；纯脏标记判据与已验收的释放后复用形态互斥——实测 `10 passed; 2 failed`，失败的正是 MS35 的两条已验收用例 ⇒ 两条流程在取样点上状态完全相同，唯一可区分的信号是**失败历史**；判据应对失败写敏感而非对脏标记敏感。该结论在释放持久化面实测定形，同族可复用于本配方的锚点写持久化判定）；写索引写侧缺陷的复现、修复验证与回归用例设计时使用（来源 MS34 Issue 修复批 Iteration 000；补充来源 缺陷修复 change Iteration 004）
- **状态**: active

## R57: DDL 目录变更的提交点配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/ddl-catalog-commit-point.md`
- **日期**: 2026-10-24（**2026-10-25 补充墓碑写入次序结论**）
- **用途**: 把 DDL 的目录面变更完全移出语句事务体、改到提交点之后应用的实施配方——守卫 → 事务内数据面改写 → `commit` → `guard.disarm()` → 应用目录变更（`PendingColumnDrop` / `apply_column_drop`：先列行、后列数、再切 live meta），以及「改写中途可停住」的锚点式放弃手法（逐行 `record_version` ⇒ 活跃事务已记录版本数 ≥ 2 即为锚点；实测 30 行规模即可命中；夹具在语句先跑完时显式 panic，不以「跑完也算放弃」放宽判据）；**2026-10-25 补充「墓碑写入次序」结论**（固定次序为「不带链指针写入 → 登记版本 → 补链指针 → 清理索引条目」；理由：墓碑一旦带链指针且删除者事务仍活跃，扫描的 `superseder_suppresses` 走「已提交删除」分支压制其前驱版本 ⇒ 该行立即不可见；墓碑未被中和而事务又已移出活跃集时回收无法中和它 ⇒ 被放弃的 DDL 隐藏了它开始改写的那一行；配合 `abort_cleanup_versions` 新增的 `versions_created_by`（按创建者事务 id 取回被放弃事务创建但从未注册的版本，带环守卫）覆盖更窄窗口）；写 DDL 目录面事务化类缺陷的修复与回归判据时使用（来源 MS34 Issue 修复批 Iteration 001；补充来源 缺陷修复 change Iteration 001）
- **状态**: active

## R58: ISS26 索引树页级判据缺叶页条目长度检查（短于条目格式的叶条目被静默跳过）

- **类型**: issue
- **路径**: `.agents/issues/ISS26-index-leaf-entry-length-unchecked.md`
- **日期**: 2026-10-26
- **用途**: 索引树页级判据的缺陷台账——`integrity::check_tree_page` 的**叶分支只校验槽边界**，缺少内部节点分支已有的长度检查 ⇒ 槽负载短于条目格式（38 字节）的叶条目落在页内时 `violations` 为空、不被标记 ⇒ `node.rs` 的 `get_key` / `get_row_id` 返回 `None` ⇒ `read_leaf_pair` / `scan_all_async_from_root` / `find_key_position_binary` 把它当作「无此条目」⇒ **静默跳过**（与 `index-read-bounded-read` 域要求的「点名损坏的页与该索引树」方向相反）；既有缺陷、非回归；承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 006 的 Issue 候选 IC-1（Plan Review F6 判定为明确在 Plan 范围外），Iteration 007 沿用同一编号；IRBR 的 Requirement 正文已收敛为「未通过页级判据的索引树页」⇒ 该行为**不在**该域已验收主张之内；承载建议为独立 change（叶分支补长度检查，读 / 写路径与释放路径的处置方向相反），字节级损坏注入手法可复用
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R59: ISS27 存储层其余页型的无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处）

- **类型**: issue
- **路径**: `.agents/issues/ISS27-unbounded-slot-reads-other-page-types.md`
- **日期**: 2026-10-26
- **用途**: 存储层页内容读取无界化的缺陷台账——`SlottedPageRef::get_slot_data` 的**未检查调用点**分布在 `src/storage/data_page.rs` / `catalog.rs` / `buffer_pool.rs` / `data/table_manager.rs` / `src/wal/recovery.rs` / `src/executor/*`，**同形态约 17 处**；该函数直接按页内槽目录的 `offset` / `length` 切字节且**无范围检查** ⇒ 槽偏移或长度越界 ⇒ 越界切片 panic（发生在异步执行路径 ⇒ 进程 abort）；各页型的**正确行为不同**（索引树页是「下降决策」，数据页是「行内容」，目录页是「目录行」）⇒ 处置方向不能一次统一；已被承载 change Iteration 002（释放路径）与 Iteration 006（读 / 写路径）覆盖的**索引树页**面不属本条；承载建议为独立 change 并**按页型分组**（每组各自判定「损坏页在该页型的正确行为」），损坏注入手法可复用
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R60: ISS28 失败的 DDL 语句其目录面变更在后续刷盘落地

- **类型**: issue
- **路径**: `.agents/issues/ISS28-failed-ddl-catalog-change-flushes-later.md`
- **日期**: 2026-10-26
- **用途**: DDL 语句边界落盘顺序语义的缺陷台账——承载 change Iteration 004 修复「刷盘失败保持脏标记」后，一条**失败**的 DDL 语句已发出的目录面变更（列行 + 表行列数）**在此后某次成功刷盘时落地** ⇒ 重开库后可见该失败 DDL 的目录面变更；实测形态（真实入口 `ALTER TABLE keep ADD COLUMN m INT` + 一次性写盘失败注入）：修复后重开读出 `[1, 10, Null]`（目录面变更落地），修复前重开报 `Column not found: m`（目录面变更被丢弃、且表行列数与列行自相矛盾）；与既有域 `ddl-catalog-transactional` R1「被放弃或失败的 DDL 不留目录面变更」**字面冲突**；承载 change 明确把该语义划为 Excluded scope（D9 / B-3）**不裁决** ⇒ 需产品决定（回滚目录变更 vs 修订既有域的表述与迁移指引）
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R61: ISS29 同表并发 DDL 与在途状态布尔量冲突（后发 DDL 清除先行 DDL 的读拒绝）

- **类型**: issue
- **路径**: `.agents/issues/ISS29-same-table-concurrent-ddl-inflight-flag.md`
- **日期**: 2026-10-26
- **用途**: DDL 并发控制与在途状态建模的缺陷台账——在途状态是**单表一个布尔量**（`NarrowingState`：`table: String` + `in_flight: AtomicBool`）⇒ 两个交互缺陷：① **重叠清除**：同表上并发的第二个 DDL 若先完成并清除该布尔量，会让**第一个**仍在窗口内的读**失去拒绝** ⇒ 退回按位置取值（静默错读）；② **豁免读按旧 schema 解码**：第二个 DDL 的改写自身读（豁免路径）会把**已被第一个 DDL 窄化**的行按旧 schema 解码；该形态**不是**承载 change 引入的（同表并发 DDL 在此之前即不安全，两条改写争同一条数据链与目录），承载 change Iteration 003 明确不扩大该面、Iteration 004 再次划为范围外；闭合需要「**同表 DDL 串行化**」这一新的设计决定（拒绝同表并发 DDL / 语句级排队 / 在途状态改为可区分多改写的结构）
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R62: ISS30 并发驱逐 + 写盘失败 ⇒ 被驱逐页的内容既未落盘也不在缓冲池内

- **类型**: issue
- **路径**: `.agents/issues/ISS30-evicted-page-lost-on-write-failure.md`
- **日期**: 2026-10-26
- **用途**: 缓冲池驱逐与写盘失败交错的缺陷台账——页在**被驱逐**（内存内容丢弃）与**写盘**之间交错时，若随后的写盘失败，该页的新内容**既未落盘也不在池内** ⇒ 磁盘仍是旧内容、池内也没有新内容 ⇒ 后续读该页得到旧值且**无显式错误**暴露给用户；承载 change Iteration 004 修复的是「**释放**的持久化前置条件」（`last_failed_write` 使释放不被提升），未覆盖驱逐交错 ⇒ 划为范围外实质缺陷；需要的决定是「写盘失败时对**已驱逐**页的处置语义」（回写内存 / 保留在池内标记为脏并重试 / 提升为用户可见错误）；与 ISS31（`last_failed_write` 不复位）根因位置不同，**不合并**
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R63: ISS31 `last_failed_write` 永不复位 ⇒ 一次写盘失败后本会话内的释放都不再被提升

- **类型**: issue
- **路径**: `.agents/issues/ISS31-last-failed-write-never-resets.md`
- **日期**: 2026-10-26
- **用途**: 写盘失败状态生命周期的缺陷台账——`last_failed_write` 一旦置位就**永不复位**（标记只在失败时置位，无任何复位路径）⇒ 该库会话内的**所有后续释放**都不再被提升，即便失败早已被刷盘恢复；当前是**安全但有界**的退化（保守方向为「宁可不释放」⇒ 空闲页复用变慢，不产生数据错误）；标记是**库级**而失败实际只影响**单个页** ⇒ 页粒度细化留作后续决定；承载 change Iteration 004 明确划为 Excluded scope；与 ISS30（驱逐交错）根因不同，**不合并**
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R64: ISS32 多表 dump 中靠后语句被范围外拒绝时，靠前语句已生效

- **类型**: issue
- **路径**: `.agents/issues/ISS32-multi-table-dump-partial-restore.md`
- **日期**: 2026-10-26
- **用途**: restore 提交语义的范围外观察台账——`restore` **逐条执行并各自 auto-commit** ⇒ 一份**多表** dump 中若**靠后**语句被范围外拒绝，**靠前**语句**已提交**；既有合同（`cli-noninteractive-shell` 已把「逐条 auto-commit、失败即停、失败前语句已生效」写成合同），MS36 的 `design.md` D4 明确**不改动**该语义并把本条记为「已知边界（不在该 change 修复）」；与 `legacy-format-compat` 的「SHALL NOT 静默地部分恢复后报告成功」**不冲突**（当前行为是显式报错 + 非零退出码 + 说明「previous statement(s) were committed」）⇒ **不是规格违背、不是新缺陷**；若要原子化（或全量预检）属功能语义变更，需独立 change 与独立验收
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R65: ISS33 `dump` 对非整数 `UNIQUE` 列无条件渲染 ⇒ 该类旧表的 dump 不可恢复

- **类型**: issue
- **路径**: `.agents/issues/ISS33-dump-renders-non-integer-unique.md`
- **日期**: 2026-10-26
- **用途**: dump DDL 渲染规则的范围外观察台账——`dump` 对含**非整数 `UNIQUE` 列**的表**无条件渲染** `UNIQUE` ⇒ 「旧库 → dump → 新库 restore」在该类表上被**计划期拒绝**（`UNIQUE (INT columns only)`）；不对称点：`dump` 对 `PRIMARY KEY` **有**类型条件（仅主键列类型为 INTEGER 时输出），对 `UNIQUE` **没有**同型条件；MS36 的 `design.md` D9 已给出修复形态（按 `PRIMARY KEY` 的同型规则条件渲染 `UNIQUE`）并明确「**这不是缺陷**」（既有行为已在 README 记录）⇒ 作为可选改进登记；修复会改变 dump 的产物内容 ⇒ 属功能语义变更，须独立验收并复核与 `legacy-format-compat` 拒绝面的关系
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R66: ISS34 `index-read-bounded-read` 域的 Requirement 标题与 Scenario 标题措辞残余

- **类型**: issue
- **路径**: `.agents/issues/ISS34-irbr-spec-title-wording-residue.md`
- **日期**: 2026-10-26
- **用途**: 规格标题层措辞残余的台账——`openspec/specs/index-read-bounded-read/spec.md` 的 Purpose 与 Requirement **正文**已收敛为「读取到**未通过页级判据**的索引树页时」（刻意收窄：叶页短条目不被页级判据捕获），但 Requirement **标题**与 **4 个 Scenario 的标题 / GIVEN** 仍写「损坏的索引页」⇒ 标题层与正文层的覆盖面表述不一致，引用该域时会把 ISS26 的形态误算为已覆盖；正文（规范性 SHALL 条款）正确 ⇒ 非阻塞、不改变任何行为要求；承载 change Iteration 007 的 Plan Review F14 判为不阻塞并建议合并时顺带对齐，登记复核确认合并后**措辞残余仍可见**；承载落点为 docs-maintainer 在规格维护时对齐（只改标题 / GIVEN 措辞，不改 SHALL 条款、不改 Scenario 数与 Requirement 数）
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R67: DML 墓碑的「放弃窗口」复现配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/dml-tombstone-abandon-window.md`
- **日期**: 2026-10-25
- **用途**: 确定性复现「DML 语句被放弃时，它已写下的墓碑仍生效 ⇒ 被它删除 / 替换的那一行从数据面消失」的配方——30 行表 → 锚点式放弃 `DELETE FROM t WHERE id = 1`（锚点 = 数据链上已出现由活跃事务创建的版本槽）→ 读放弃落点（`registered` 读数区分「写出版本槽 → 登记版本」窗口与「登记版本之后」窗口）→ 丢弃语句 future ⇒ 下一条写语句触发库属隐式事务回收 → 观察可见 id 集合与逐 id 点查命中；含修复前 RED 形态（`missing ids: [1] (visible: [2..30])`、`version registered at the abandon point = false`、`1 passed; 2 failed`）与修复后 GREEN（`3 passed; 0 failed`），以及已提交墓碑终态守门用例（证明只改写入次序、不改终态）；断言强度为逐行可数 + 逐 id 点查，N = 30 < 300 刻意取小以排除链分叉；夹具不携带任何运行身份，`ANCHOR_TIMEOUT` 是兜底不是判据（来源 缺陷修复 change Iteration 005）
- **状态**: active

## R68: 索引树字节级损坏页的注入夹具配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/index-page-corruption-injection.md`
- **日期**: 2026-10-25
- **用途**: 在真实文件库上造出字节级损坏的索引树页（槽偏移越界）并观察两条路径各自表现的配方——页内槽目录项布局 `[logical_id:2][offset:2][length:2]`、第 `index` 项落在页尾 `PAGE_SIZE - (index+1)*Slot::SIZE`、offset 字段再 +2；文件偏移 `HEADER_SIZE + page * PAGE_SIZE + in_page`；注入值 `0xFFFF`（超过页长 ⇒ `get_slot_data` 的 `&data[start..end]` 必然越界）；形态为「写库 → 关闭 → 改坏页字节 → 重开 → 执行语句」（重开依赖 `database.rs` 的「重建失败只告警」行为）；释放路径（`DROP TABLE` / `TRUNCATE` / `DROP INDEX`）表现为**告警 + 跳过**（尽力而为，返回已收集的部分），读 / 写路径（`SELECT` / `INSERT` / `DELETE` / `UPDATE`）表现为**明确错误**并同时点名损坏的页与该索引树的根页（`corrupted index page {page_id} (index root page {index_root})`），两者方向相反；含修复前 RED 形态（panic 位点 `slotted_page.rs:128`，消息 `range start index 65535 out of range for slice of length 4096`，伴随 `PoisonError` ⇒ SIGABRT）与合法页零误报核对（N=46 / N=92 叶页无 violation）（来源 缺陷修复 change Iteration 002 与 Iteration 006）
- **状态**: active

## R69: G2 旧格式样本的产出配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/legacy-sample-production.md`
- **日期**: 2026-10-25
- **用途**: 为某个代际（G1 / G2）产出**真实来自旧构建**的库文件样本并固定入库的配方——隔离克隆（`git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy-g2`）→ `git checkout 6e8dcf5` → `cargo build --offline --bin rtsql` → 旧 CLI（`new` / `CREATE TABLE` / `INSERT` / `DROP TABLE` / `dump`）在**仓库之外**的暂存目录内产出样本 → **只把样本本体**拷入 `tests/fixtures/legacy/`（伴生 `.wal` / `.checkpoint` 不入库）→ 更新来源说明（revision / 构建方式 / 产物路径 / 工具链 / 代际判别特征 / 内容清单 / 预期结论 / 再生步骤）→ 由断言锁定派生量（与来源说明两处同源同步）→ 清理临时克隆与构建产物；纪律：目标代际的旧 revision 必须**先独立核实**（G2 = `6e8dcf5`，其目录行布局含唯一根段、不含二级索引段与表类型字节）、必须离线可构建（不可用即回 Plan，**不得**手工构造样本）、样本规模 ≪ 300 行、运行时**不做**版本嗅探 / hash / revision 比对（⇒ 样本来源的权威记录是来源说明 + 再生步骤，不是字节内的标记）、派生量不一致时**先核对样本字节是否被重新生成**、不得就地改断言常量迁就样本（来源 缺陷修复 change Iteration 007）
- **状态**: active

## R70: 旧 dump 的兼容面配方 Runbook

- **类型**: runbook
- **路径**: `.agents/runbooks/legacy-dump-compat-surface.md`
- **日期**: 2026-10-25
- **用途**: 在真实 CLI 上验证「旧 dump → 新库 restore」与范围外拒绝 / 迁移路径的配方——旧 dump 的**短类型名**（`INT` / `STRING` / `BOOL` / `FLOAT` / `DATE` / `TIMESTAMP`）与**旧字面量**（`DATE '…'` / `TIMESTAMP '…'` / `TRUE`）当前构建照常接受（当前构建的 `dump` 则渲染长类型名 `INTEGER` / `TEXT` / `BOOLEAN`）；范围外三类拒绝一律**退出码 3** 并**定位到语句**（`NON-INTEGER PRIMARY KEY` / `UNIQUE (INT columns only)` / `NOT NULL constraint violation: column '<列名>'`，错误文本含「statement k of n failed」），各有实测迁移路径（去 `PRIMARY KEY` / 去 `UNIQUE` 或**改类型并同步转换行值** / 回填 `NULL` 或删除该行；另两类 `NewerFileVersion` 与 `NotADatabase` 为退出码 1）；关键的实测区分：**只改列类型不足**（仍退出 3，stderr `key column 'email' expects INT, got String`）；并记录**命令列必须写成可直接执行的形式**——`src/cli/resolve.rs` 的 `resolve_db_path` 把**裸名**解析到 `$RTSQL_HOME/db/<name>.db`（未设 `RTSQL_HOME` 时为 `$HOME/.rtsql/db/<name>.db`）⇒ 按裸名字面执行会以 `does not exist` 退出 1 ⇒ 文档 / 记录一律用含 `/` 的路径形式并加一句工作目录说明（来源 MS36 Iteration 001 与 缺陷修复 change Iteration 007 的 T19）
- **状态**: active

## R71: ISS35 页头槽数越界导致读路径 abort

- **类型**: issue
- **路径**: `.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`
- **日期**: 2026-10-26
- **用途**: 页头槽数超出槽目录容量时读路径整数下溢并 abort 的台账。已由归档变更 `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure` 的 Iteration 002 / T17、004 / T4、005 / T5 处置。
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R72: ISS37 写路径页头槽数下溢

- **类型**: issue
- **路径**: `.agents/issues/ISS37-write-path-slot-count-underflow.md`
- **日期**: 2026-10-26
- **用途**: 写路径用页头槽数做减法时整数下溢的台账。已由同一归档变更 Iteration 003 / T18 处置。
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R73: ISS38 写路径槽负载偏移无界写

- **类型**: issue
- **路径**: `.agents/issues/ISS38-write-path-slot-payload-offset-unbounded.md`
- **日期**: 2026-10-26
- **用途**: 写路径按槽偏移写入时不检查边界的台账。已由同一归档变更 Iteration 003 / T19 处置。
- **状态**: closed（2026-10-26 fixed → openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure）

## R74: ISS39 ADD COLUMN 与在途窄化 DDL 并发

- **类型**: issue
- **路径**: `.agents/issues/ISS39-add-column-races-inflight-narrowing.md`
- **日期**: 2026-10-26
- **用途**: 窄化发布与在途加列交错的台账。已由该归档变更 Iteration 001 / T2 处置。
- **状态**: closed（2026-10-27 fixed → openspec/changes/archive/2026-10-26-ms40-add-column-catalog）

## R75: ISS40 ADD COLUMN 在列数刷盘期间被放弃

- **类型**: issue
- **路径**: `.agents/issues/ISS40-add-column-abandoned-during-column-count-flush.md`
- **日期**: 2026-10-26
- **用途**: 列数刷盘期间放弃 ADD COLUMN 的台账。已由该归档变更 Iteration 000 / T1 处置。
- **状态**: closed（2026-10-27 fixed → openspec/changes/archive/2026-10-26-ms40-add-column-catalog）

## R76: ISS41 `Float(-0.0)` 与 `Float(0.0)` 的 `PartialEq` 相等、`Hash` 不同

- **类型**: issue
- **路径**: `.agents/issues/ISS41-value-float-signed-zero-hash-partialeq.md`
- **日期**: 2026-10-26
- **用途**: 负零与正零在哈希连接中漏匹配的台账。已由该归档变更 Iteration 002 处置。
- **状态**: closed（2026-10-27 fixed → openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side）

## R77: 后续工作的优先级与依赖分析

- **类型**: analysis
- **路径**: `.agents/analysis/future-work-priority-and-dependencies.md`
- **日期**: 2026-10-27
- **用途**: 检索未排期工作的排序建议、技术前置、按需启动条件、代码依据与验证边界，供后续路线和实施调查使用。
- **状态**: active

<!-- arc: ARC-202609092322 --> 1 条已归档 (2026-09-09) → openspec/changes/archive/2026-09-09-ARC-202609092322/proposal.md
