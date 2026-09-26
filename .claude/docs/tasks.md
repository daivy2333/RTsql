# tasks — 任务与里程碑路线

> 最后更新：2026-09-26（MS24 收尾归档——change `2026-09-25-ms24-write-surface-completion` 双 Iteration 全 accepted：Iteration 000 写入类型门（ISS04 端到端收口）+ 子集 INSERT/DEFAULT 持久化与应用；Iteration 001 UPSERT/REPLACE INTO 与其 replan Cycle 的 PK 预检补齐 + 回滚后墓碑行索引条目还原；增量规格合并 `sql-write-surface`（新）+ `insert-column-list-mapping`/`mvcc-tombstone-visibility`/`sql-constraint-enforcement` 三处修改，carrier 归档；全量 1239 tests。前次 2026-09-25：MS24 Iteration 000 Review finding 经用户指令并入 MS24-T04；ISS04 并入 MS24-T03；MS23 收尾归档）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

28 个 Milestone：14 completed + 6 superseded（历史段已归档至 carrier `ARC-202609242151`，本文件仅保留活跃路线）+ 8 planned（初版后统一路线，2026-09-24 规划：MS18-MS27；MS23 于 2026-09-25 完成，MS24 于 2026-09-26 完成〔T04 声明期 DEFAULT 类型校验未实施，作独立 planned 任务行留存〕）。★初版达成★。

统一执行序（2026-09-24 用户裁定初版后工作完全重排——编号与执行序无关，沿用「批准执行序与编号无关」既定先例；编号只表规划先后，执行以本序为准）：**MS23 数据完整性约束执行面（正确性红线最优先）→ MS24 SQL 写面补全 → MS19 CLI 管理面小收口 → MS18 one-shot 路径性能收口 → MS25 SQL 读面补全 → MS20 ATTACH 式跨库交互 → MS21 DDL 演进与二级索引 → MS26 嵌入式运行形态 → MS27 存储演进批 → MS22 实测驱动性能批（量化定稿后转 ready）**。顺序为建议序非硬依赖（各项无环、前置均已满足）。编排依据：正确性红线先于新功能（MS15/MS16 纪律）；同代码面相邻减少返工（MS23→MS24 唯一索引与约束语义、MS18→MS26 checkpoint 策略面、MS23→MS21 索引基建、MS26→MS27 存储后端覆盖）；小项批紧随正确性带；性能殿后、先量化再决定（MS08 纪律）。首批排除域维持：微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058。REPL（I067）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）等留 improvements 域未排期。
旧优化项历史分类（各 superseded MS 原范围段）随 `ARC-202609242151` carrier 留档；未排期候选以 D-candidates 与 improvements 台账为准。
## Milestone Roadmap

### MS18：one-shot 路径性能收口 — planned

- **Status**: planned
- **Dependencies**: None
- **Outcome**: 零写入会话 `close()` 不再全价执行 checkpoint（I062——WAL 未决记录为零/低于阈值时跳过重写，「close 即落盘」语义不变）；CLI runtime 规格经 RSS/时延/吞吐三组实测定形（I063——必要时切 `current_thread`）；one-shot 固定开销向 SQLite 量级收敛（实测基线：`SELECT 1` 10.8ms vs SQLite 1.15ms、RSS 16.7MiB vs 4.1MiB）
- **Rationale**: agent/脚本高频 one-shot 是本数据库的主消费形态，固定开销是当前最实测可得的最大体验差距（README 对比板块 + R26 runbook）；两项同属 open/close 生命周期资源开销域，共享时延/RSS/吞吐验证边界，聚合成单一阶段成果「one-shot 固定开销收敛」
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS18-T01 | I062：checkpoint 前检查 WAL 未决记录，为零/低于阈值时跳过重写直接返回（`WalWriter` 已有文件长度查询） | improvements I062（2026-09-24 实测登记） |
| MS18-T02 | I063：CLI runtime flavor 评估（multi_thread 默认 vs current_thread），以三组实测数据决定是否切换 | improvements I063（2026-09-24 实测登记，先量化再决定） |

- **Non-goals**: 引擎级分配器（I066→MS22 候选）；Server 面 runtime；扫描流式化（I065）；WAL fsync 合并（I049）与 writev 批量写回（I026）→MS22；加密性能 bench（I057）
- **Workload**: 1-2 个小 change（close 路径一项 + runtime 评估一项，可合可分）
- **Stable baseline**: `rtsql <db> "SELECT 1"` 时延与 RSS 较基线显著下降且行为不变；写入会话 close 仍完整 checkpoint；全量测试零修改通过
- **Verification boundary**: R26 runbook 时延/RSS 复测对比 + 写入会话落盘语义回归 + 全量零回归
- **Diagnostic boundary**: `src/database.rs` close/checkpoint 接线 + `src/main.rs` runtime 构造
- **Split signals**: current_thread 切换牵动引擎内部 `spawn_blocking`（WAL/页 I/O）深改时拆出独立 MS
- **Related changes**: None

### MS19：CLI 管理面小收口（删库子命令 + 错误可操作化 + 会话边界测试加固） — planned

- **Status**: planned
- **Dependencies**: None
- **Outcome**: `rtsql delete <db>` 删库子命令闭环（I061——复用 `resolve_existing_db`，删除主文件与 `.wal`/`.checkpoint` 伴生文件并报告释放结果，打开中经 advisory 锁显式拒绝）；`PlanError` 携带特性名与错误分类（I070——`UnsupportedStatement`/`UnsupportedExpression` 点名不支持的能力，协议错误码分类面评估）；「会话事务活跃中遇边界子句」组合路径 e2e 锁定（I042——拒绝不改会话态、收尾回滚语义回归锁定）；文件对（`.db`+`.wal`）移动/拷贝/重命名配对行为实测与备份边界文档化（I072）
- **Rationale**: 均为 CLI 应用层小项，同属「日常管理与排错体验」主题——生命周期命令闭环 + 错误信息可操作 + 既有组合语义回归锁定 + 数据安全边界文档化；MS15/MS17 批处理先例，各项独立验收、单项失败不阻塞其余
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS19-T01 | I061：`rtsql delete <db>` 子命令（命令命名、dry-run/确认交互、路径形态边界随 change 调查定稿）；顺带 I072 文件对配对实测与备份边界文档化 | improvements I061/I072（2026-09-24 用户方向 + 路线补充） |
| MS19-T02 | I070：PlanError 特性名携带 + 错误分类（评估小改动面） | improvements I070（R18 主题 3） |
| MS19-T03 | I042：`tests/tx_statement_test.rs` 增加边界子句×活跃会话组合场景 e2e（可随 T01/T02 触碰 CLI 会话面时顺带） | improvements I042（MS11-T02 Review finding，2026-09-24 路线补充排期） |

- **Non-goals**: REPL（I067）；`install.sh --purge-data` 语义变更；纯文档项 I045/I064（用户裁定暂缓）
- **Workload**: 2-3 个独立小 change（T03 可并入 T01/T02 任一 change 顺带完成）
- **Stable baseline**: 生命周期子命令含 delete 闭环；不支持语句/表达式报错直接可读特性名；会话边界组合语义有 e2e 锁定；备份边界有实测依据的文档声明
- **Verification boundary**: delete 子命令独立测试（伴生文件清理/锁占用拒绝/路径形态/释放报告）+ 文件对配对行为矩阵实测记录 + 错误面快照测试 + tx_statement 组合场景 e2e + 全量零回归
- **Diagnostic boundary**: `src/cli/lifecycle.rs`（+`resolve.rs`）、`src/cli/mod.rs` run_sql 会话路径与 `src/parser/planner` 错误构造面
- **Split signals**: 错误分类牵动 PG 协议层大改时拆出；delete 交互设计膨胀时裁剪为最小语义
- **Related changes**: None

### MS20：ATTACH 式跨库交互 — planned

- **Status**: planned
- **Dependencies**: None（建议序在 MS18 后——one-shot 收敛对多库 CLI 场景有益，非硬前置）
- **Outcome**: 多 `.db` 文件经 `ATTACH` 关联检索与物化可用（I059，用户点名方向）——一期：attach 注册表 + 「别名.表」命名空间解析（I039 落地的表名消费点扩展）+ 只读跨查 + `CREATE TABLE AS SELECT` 物化（`new`+attach+CTAS 组合覆盖「数据库视作表」全场景）；二期：跨文件 DML（每文件独立提交，非原子 v1 语义文档化）
- **Rationale**: 用户方向登记（2026-09-24）：两个及以上 db 文件关联检索、跨库增删查改、跨库视图物化；嵌入式正统设计（SQLite ATTACH 同型，PostgreSQL 单连接锁死单库为反例）；一期完成即有独立项目价值（跨库查询/抽取/合并/分析），二期依赖一期 attach 基础设施——同一阶段成果「跨文件关联检索与物化」分两期推进
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS20-T01 | 一期：attach 注册面 + 命名空间解析 + 只读跨查 + CTAS | improvements I059（2026-09-24 用户方向） |
| MS20-T02 | 二期：跨文件 DML（独立提交语义 + 边界文档化） | 同上 |

- **Non-goals**: 跨 attach 原子提交（master journal 级机制）；分布式/网络多库；每文件独立 RC 快照之外的隔离语义；crate 化「多文件会话」抽象（I074 排除域，若 I074 将来启动则顺势承接）
- **Workload**: 2-3 change（一期 attach+跨查+CTAS；二期 DML；视调查可合并）
- **Stable baseline**: 一期后跨库只读查询与 CTAS 物化 e2e 可用且单库行为零回归；二期后跨文件增删改可用且非原子边界明确；交叉 attach 死锁结构性不可能（try_lock 即 `DatabaseLocked` exit 4）
- **Verification boundary**: attach/命名空间/跨查/CTAS/跨文件 DML 独立测试 + 文件锁冲突场景 + 全量零回归
- **Diagnostic boundary**: `src/database.rs` attach 注册面 + planner 表名解析（`object_name_to_table_name` 消费点扩展）+ pipeline 执行器路由
- **Split signals**: 二期跨文件 DML 语义或锁交互超预期时拆出独立 MS，一期独立收口
- **Related changes**: None

### MS21：DDL 演进——ALTER TABLE 与二级索引 — planned

- **Status**: planned
- **Dependencies**: None（catalog 持久化通道由 MS07-T01 满足）
- **Outcome**: `ALTER TABLE ADD/DROP COLUMN` 与 `CREATE INDEX`/`DROP INDEX` 可用（I068）——列结构演进不再依赖 dump→改 DDL→restore 重建，非 PK 查询列可建二级索引；既有索引条目维护在删除与回滚路径上的一致性收口（回滚后键位点查与实际存活版本一致）；`CREATE VIEW`/`TRUNCATE` 届时一并裁定是否并入范围（EXPLAIN 已转正为 MS25-T05 顺带项，2026-09-24 统一重排）
- **Rationale**: SQL DDL 面最大功能缺口（R18 主题 2/3）；触及 catalog/序列化/重建路径的独立故障域，工作量中-大需独立阶段，与功能小项和性能批互不阻塞。T03 与本阶段同主题相邻（同为索引条目维护面、共享 `src/storage/btree/` 索引管理与执行器诊断边界），按聚合规则并入而非独立成阶段
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS21-T01 | CREATE/DROP INDEX 二级索引（catalog 登记 + 执行器 + 查询路由可达 + 恢复重建） | improvements I068（R18 主题 3） |
| MS21-T02 | ALTER TABLE ADD/DROP COLUMN（catalog/序列化/数据面演进） | 同上 |
| MS21-T03 | 索引条目维护一致性收口（删除与回滚路径，阶段内优先执行——正确性红线属性）：① `TransactionManager::abort_cleanup_versions` A 趟对 rekey UPDATE 回滚的条目回退按 rekey 语义处理（现以 `find_key_by_row_id` 所得新键执行 `update(key, prev)`，而 prev 版本 tuple 携带旧键 → 回滚改键 UPDATE 后新键条目指向旧键版本、旧键条目缺失、旧键等值点查漏行，`src/transaction/manager.rs`）；② `upsert.rs::delete_conflict_row` 的 `SlotNotFound` 容忍臂与 `delete.rs` 同形态对齐（无元组可读时仍按搜索键执行 `index_manager.delete`；仅索引条目指向不存在 slot 的夹具/损坏态可达） | MS24 Iteration 001 Plan Review Minor 2（replan Cycle）与父 Cycle Minor 4，2026-09-26；同源于 `sql-write-surface` R6 索引还原面 |

- **Non-goals**: DECIMAL/BLOB 类型（I069 深水区另议）；代价模型与 Join 重排（I016）；在线 schema 变更的并发语义精细化；已文档化的 UPSERT 仲裁内 F1 守卫角落（`sql-write-surface` R3 已知边界段——修复需反转「唯一列类型守卫先于仲裁」的产品语义顺序，另议）
- **Workload**: 2-4 change（INDEX 与 ALTER 各自独立验收；T03 一致性收口可独立小 change，与 T01/T02 无耦合）
- **Stable baseline**: 非 PK 列可建索引且查询计划可达、drop/restart 后索引一致；加列/删列后数据与 schema 持久化往返一致；删除与回滚（含 rekey）后 PK 与唯一索引条目与实际存活版本一致，旧键与唯一值点查均可达
- **Verification boundary**: ALTER/INDEX 独立测试（含崩溃恢复两态一致）+ T03 的回滚/删除两态矩阵（rekey UPDATE 回滚点查、失败语句无残留、恢复两态）+ 全量零回归
- **Diagnostic boundary**: `src/storage/catalog.rs` + `src/parser/planner/ddl_dml.rs` + `src/storage/btree/` 索引管理与执行器 + `src/executor/{delete,upsert}.rs` 条目清理臂 + `src/transaction/manager.rs::abort_cleanup_versions`
- **Split signals**: DROP COLUMN 触发全行重写格式变更过大时先收 ADD COLUMN + INDEX，DROP 另行评估；T03 若牵动 `abort_cleanup_versions` A 趟整体重做（超出「A 趟保持现状」语义）时拆为独立 change
- **Related changes**: None（T01/T02 尚未创建；T03 来源 change 已归档 `openspec/changes/archive/2026-09-25-ms24-write-surface-completion/`）

### MS22：实测驱动性能优化（第一批） — planned（范围量化定稿后转 ready）

- **Status**: planned（候选范围经 profile/bench 定稿后转 ready——沿用 MS08「先量化再决定」纪律）
- **Dependencies**: None 硬前置（建议序在 MS18 后——I062 先收 close/checkpoint 路径，避免与 I026/I049 同面冲突）
- **Outcome**: 经量化证实的性能积压按收益排序清收（第一批）。候选池：I049 WAL fsync 合并（组提交）/ I021 INSERT 多值批量 / I026 脏页 writev 批量写回 / I050 RowLockTable DashMap 化 / I024 Varint Key 变长编码 / I038 GC 无键行盲区 / I020 clone 消除 / I066 分配器评估（jemalloc/mimalloc）——每项先 bench/profile，达标立项，不达标退还 improvements 并留量化记录
- **Rationale**: MS08 实测驱动批处理先例的复活（2026-09-14 剥离时各项「先量化再决定」纪律随条目保留）；各项独立故障域但共享同一验收主题「实测证实的性能收益」，每项独立 change 独立验收，单项失败不阻塞其余
- **Scope**: 候选池见 Outcome；定稿范围以量化数据为准（规模：小-中 change × N）
- **Non-goals**: 撕裂树运行期根修（I031 中-大，量化后另行评估）、代价模型（I016）、扫描流式化（I065）、B+Tree 节点级锁（I025）、io_uring（I028）、瘦内部节点（I029）、合并 Tag byte（I030）——全部留 improvements 域
- **Workload**: 视定稿范围 N 个独立小-中 change，每 change 前后 bench
- **Stable baseline**: 定稿项各有 before/after bench 对比数据且全量零回归；未达标项有量化记录可退还
- **Verification boundary**: 每项 bench 对比（`--save-baseline` 纪律，R26/ms08-bench runbook）+ 全量测试零回归
- **Diagnostic boundary**: 各候选项在 improvements 条目自带诊断面
- **Split signals**: 定稿项 ≥4 或出现跨域大项（如 I031 入选）时拆第二批
- **Related changes**: None

### MS23：数据完整性约束执行面 — completed

- **Status**: completed（2026-09-25——change `2026-09-24-ms23-constraint-enforcement` 双 Iteration 全 accepted 收尾并归档；增量规格已合并 `openspec/specs/sql-constraint-enforcement/`；交付含 F1 修复轮：唯一列非 Int 写入值 `KeyTypeMismatch` 守卫）
- **Dependencies**: None
- **Outcome**: 建表声明的约束要么被强制要么被诚实拒绝——UNIQUE 经非 PK 唯一索引在写路径强制（重复 → DuplicateKey 同型拒绝）且崩溃恢复两态一致（复用恢复期索引去信任重建通道）；NOT NULL 写入前置零副作用拒绝；CHECK 与 FOREIGN KEY 建表计划期点名拒绝（消除「DDL 静默接受、运行期永不生效」类缺陷）
- **Rationale**: 2026-09-24 Explorer 定位缺口调查（用户批准「这些问题存在且应当解决」）发现的正确性红线——DDL 接受的约束一半从未被执行器消费：UNIQUE/NOT NULL 仅解析持久化进 catalog（`src/executor/plan.rs:227` → `ColumnSchema`），insert/update 执行器零消费；CHECK/FK 在 `src/parser/planner/ddl_dml.rs:390` 被注释明写 ignored。属 MS15/MS16 清过的「静默错误结果」同类，按正确性优先纪律先于一切新功能，居统一执行序首位
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS23-T01 | UNIQUE 强制：非 PK 唯一索引（内部建，不经用户 CREATE INDEX 语句）+ INSERT/UPDATE 冲突拒绝 + 恢复重建两态一致 | Explorer 2026-09-24：catalog 有 unique 标志、执行器零消费（`src/executor/insert.rs`/`update.rs` 无 unique 检查点） |
| MS23-T02 | NOT NULL 强制：写入前置校验，零副作用拒绝 | Explorer 2026-09-24：同上，insert/update 无 not_null 检查点 |
| MS23-T03 | CHECK/FOREIGN KEY 建表显式拒绝（点名不支持；可与 MS19-T02 错误面成果衔接） | Explorer 2026-09-24：`ddl_dml.rs:390` 注释明写 Null/ForeignKey/Check/DialectSpecific ignored |

- **Non-goals**: FOREIGN KEY 强制执行（深水区，本批只诚实化）；用户级 CREATE/DROP INDEX（MS21-T01——复用本任务唯一索引基建，建议序在其后）；约束随 ALTER 演进（MS21 ALTER 域）；DEFAULT 应用与子集 INSERT（MS24-T01）
- **Workload**: 2 个小 change（T01 一个；T02+T03 诚实化可合一个）
- **Stable baseline**: 任意建表 DDL 声明的每条约束要么生效要么建表时报错；唯一冲突/NULL 拒绝 e2e；恢复两态一致；全量零回归
- **Verification boundary**: 约束拒绝矩阵测试 + 唯一索引恢复重建测试 + 全量零回归
- **Diagnostic boundary**: `src/executor/{insert,update,create_table}.rs` + `src/parser/planner/ddl_dml.rs` 约束面 + 恢复索引重建通道（`src/wal/recovery.rs` R7/R8）
- **Split signals**: 唯一索引牵动 B-Tree 多索引管理面大改时，先收 T02/T03 诚实化，T01 拆出独立收口
- **Related changes**: `openspec/changes/archive/2026-09-24-ms23-constraint-enforcement/`（2026-09-25 收尾归档）

### MS24：SQL 写面补全（子集 INSERT 与 UPSERT） — completed（T04 除外）

- **Status**: completed（2026-09-26——change `2026-09-25-ms24-write-surface-completion` 双 Iteration 全 accepted 收尾并归档；T01/T02/T03 已交付，增量规格合并 `openspec/specs/sql-write-surface/` 等四处。**残留**：T04 声明期 DEFAULT 类型校验未实施，随后续 change 收口，保留为本 MS 下独立 planned 任务行）
- **Dependencies**: None（建议序在 MS23 后——T02 冲突检测面与 MS23-T01 唯一索引同面；T01 缺省语义消费 MS23 的 NOT NULL 语义）
- **Outcome**: `INSERT INTO t (col, ...) VALUES ...` 子集列清单可达——缺省列取 DEFAULT，无 DEFAULT 取 NULL、NOT NULL 列拒绝；`INSERT ... ON CONFLICT (cols) DO NOTHING | DO UPDATE SET ...` SQLite 子集语义可达（PK 与唯一索引冲突目标）；非键列写入值与列声明类型一致（ISS04）——类型不匹配值在任何写入前点名拒绝，消除「静默写坏」残余面；建表声明的 DEFAULT 字面量在计划期与列类型校验/归一（MS24-T04）——不存在「建表成功但应用默认值的 INSERT 必然失败」的声明/写入分裂形态
- **Rationale**: 同上调查——MS16 后 INSERT 列清单必须「恰为表列排列」（数量不符即拒），DEFAULT 解析持久化（`src/storage/data/table_manager.rs:28`）但无任何消费点、永不触发；UPSERT（ON CONFLICT / OR REPLACE）planner 全无匹配。两者是 agent 生成 SQL 的高频写入形态，与约束面同域相邻故紧随其后；ISS04（2026-09-25 MS23 Review 登记，R29）——MS16/MS23 已分别收口 PK 键列与唯一列的类型边缘，非键列写入面残余「静默写坏」，修复面（`build_update`/`extract_insert_values` 计划期门或执行器前置 + 两执行器）与本 MS 诊断边界完全同面，按同代码面相邻原则顺带收口；MS24-T04（2026-09-25 路线补充）——Iteration 000 Review 发现声明面零校验：Bool 列 `DEFAULT 1` 建表被接受（Iteration 000 D1 通道持久化任意字面量变体）、应用默认值的 INSERT 必然被 T03 写入类型门点名拒绝，声明期校验与写入门同规则面同文件（`ddl_dml.rs` DEFAULT 声明面），按 ISS04 并入先例顺带收口，补全 DEFAULT 闭环（解析→持久化→应用→渲染→声明期校验）最后一环
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS24-T01 | 子集列清单 INSERT + DEFAULT 应用（无 DEFAULT 列的缺省语义随调查定稿） | Explorer 2026-09-24：default_value 仅 catalog 持久化（`table_manager.rs:28`），insert 路径无消费 |
| MS24-T02 | UPSERT：ON CONFLICT DO NOTHING / DO UPDATE SET（冲突目标 PK/唯一索引） | Explorer 2026-09-24：`grep on_conflict\|or_replace` 零匹配 |
| MS24-T03 | ISS04：非键列写入值类型门——INSERT/UPDATE 值与列声明类型不一致在任何写入前点名拒绝（计划期 `build_update`/`extract_insert_values` 或执行器写入前置随调查定稿）；dump/restore/import 通道类型面是否纳入随调查裁定；可与 T01/T02 任一 change 顺带 | Issue ISS04（2026-09-25 MS23 Review 登记，R29） |
| MS24-T04（**未实施，planned**） | 声明期 DEFAULT 类型校验：CREATE TABLE 计划期校验/归一声明 DEFAULT 字面量与列声明类型一致（规则与写入类型门对齐——NULL 豁免、FLOAT 列 Int 升格、日期族 String 强制解析、其余跨类型建表点名拒绝零副作用），消除「建表成功但应用默认值的 INSERT 必然失败」分裂；可随 MS24 收口顺带或独立小 change（并入当前活跃 change 需用户批准扩围） | MS24 Iteration 000 Review finding（000-initial.md Follow-up Decision 2，2026-09-25；Bool 列 `DEFAULT 1` 写入期才拒绝） |

- **Non-goals**: 多值 INSERT 批量执行性能（I021——MS22 性能批候选）；REPLACE INTO 等价语法（随调查裁定）；部分索引/表达式索引冲突目标
- **Workload**: 2-4 change（T03/T04 均可顺带或独立小 change；T04 并入当前活跃 change 需用户批准扩围）
- **Stable baseline**: 省略列 INSERT 与 upsert 形态端到端可用，冲突行为与唯一索引运行期/恢复两态一致；写入值类型与列声明类型不一致被点名拒绝且零副作用；建表声明的 DEFAULT 要么与列类型兼容（含升格/解析归一）要么建表点名拒绝；全量零回归
- **Verification boundary**: 子集 INSERT/DEFAULT 矩阵 + upsert 语义矩阵（含 DO UPDATE rekey 与唯一索引交互）+ 写入值类型门拒绝矩阵 + 声明期 DEFAULT 拒绝/归一矩阵 + 恢复两态 + 全量零回归
- **Diagnostic boundary**: `src/parser/planner/ddl_dml.rs` INSERT/UPDATE 面 + `src/executor/{insert,update}.rs` + 索引冲突检测面
- **Split signals**: DO UPDATE 语义膨胀（级联类行为）时裁剪为 DO NOTHING 先行
- **Related changes**: `openspec/changes/archive/2026-09-25-ms24-write-surface-completion/`（2026-09-26 收尾归档）

### MS25：SQL 读面补全（外连接/DISTINCT/集合操作/CTE） — planned

- **Status**: planned
- **Dependencies**: None
- **Outcome**: agent 日常 SELECT 代数全形态可达——LEFT/RIGHT/FULL OUTER JOIN（非等值腿经 NLJ 通道扩展）、SELECT DISTINCT、UNION [ALL]/INTERSECT/EXCEPT、非递归 WITH（CTE，单次物化语义）；顺带聚合扩展（group_concat/COUNT(DISTINCT)）与 EXPLAIN 计划结构文本输出
- **Rationale**: 同上调查——外连接在 `src/parser/planner/query.rs:319-321` 显式 UnsupportedJoinType（I015 只覆盖 join 算法不含外连接语义）；DISTINCT/集合操作/CTE 在 planner 与 executor 全无匹配。同主题「读面代数」聚合（MS13/MS17 批处理先例），各项独立 change 独立验收、单项失败不阻塞其余
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS25-T01 | OUTER JOIN 三态（LEFT 必须；RIGHT/FULL 视 NLJ 改造成本随调查裁定） | Explorer 2026-09-24：`query.rs:319-321` 仅 `Inner(On)`，其余 `UnsupportedJoinType` |
| MS25-T02 | SELECT DISTINCT | Explorer 2026-09-24：planner/executor 无 Distinct 臂 |
| MS25-T03 | 集合操作 UNION [ALL]/INTERSECT/EXCEPT | Explorer 2026-09-24：无 SetOperation/UnionQuery 消费 |
| MS25-T04 | 非递归 CTE（WITH，单次物化语义） | Explorer 2026-09-24：无 With/Cte 构造臂 |
| MS25-T05 | 顺带小项（随 T01-T04 触碰相应面时随带，MS19-T03 模式）：group_concat/COUNT(DISTINCT)；EXPLAIN（自 MS21「届时裁定」转正） | Explorer 2026-09-24：聚合仅五件（`aggregate.rs:146-157`）；EXPLAIN 无臂 |

- **Non-goals**: 递归 CTE；窗口函数（保持远期）；LATERAL 横向引用；代价模型（I016）；SMJ（I015 剩余未排期）
- **Workload**: 2-4 change（OUTER JOIN 与 CTE 各自独立；DISTINCT 与集合操作可合）
- **Stable baseline**: 上述形态端到端全绿（含 NULL 三值语义、去重语义、CTE 单次物化）；INNER/子查询既有面零回归
- **Verification boundary**: 各形态独立测试 + 全量零回归
- **Diagnostic boundary**: `src/parser/planner/{query,subquery}.rs` + `src/executor/`（新 set/dedup 执行器、join 外连接扩展）
- **Split signals**: FULL OUTER JOIN 牵动执行器框架大改时拆出（LEFT/RIGHT 先行）；CTE 牵动 plan cache key 语义时单独成 change
- **Related changes**: None

### MS26：嵌入式运行形态（只读并发/checkpoint 水位/:memory:） — planned

- **Status**: planned
- **Dependencies**: None（建议序在 MS18 后——T02 自动水位与 I062/MS18-T01 同 checkpoint 策略面，先收 close 路径避免同面冲突）
- **Outcome**: 嵌入式多进程形态成立——只读打开模式经 advisory 共享锁多读单写（SQLite 模型：读读并发、读写互斥、写写互斥）；长会话 WAL 按大小水位自动 checkpoint（有界）；`:memory:` 库形态可用（测试/临时计算）
- **Rationale**: 同上调查——open 一律 try_lock 独占（MS10-T02），两进程连只读 SELECT 都互斥，「嵌入式」多读单写模型缺席；checkpoint 仅 close()/手动（`src/database.rs:240-248`），长会话 WAL 无界增长；`:memory:` 无任何匹配。运行形态三面同主题聚合
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS26-T01 | 只读打开模式：shared lock 多读单写；写锁已持有时只读者行为（拒开/等待）随调查裁定 | Explorer 2026-09-24：read_only/readonly 零匹配，open 全独占 |
| MS26-T02 | WAL 大小水位自动 checkpoint | Explorer 2026-09-24：checkpoint 仅 close()/手动触发 |
| MS26-T03 | `:memory:` 存储后端（自 FileStorage 抽象内存形态） | Explorer 2026-09-24：memory 库零匹配 |

- **Non-goals**: 多进程并发写（保持单写者）；读会话升级写（需重开）；Server 面多租户；跨进程 MVCC 快照语义精细化
- **Workload**: 2-3 小 change（可合可分）
- **Stable baseline**: 两进程并发只读同库可同时 SELECT；锁矩阵（读读/读写/写写）行为明确；长会话 WAL 有界；memory 库全 CRUD 可用；全量零回归
- **Verification boundary**: 并发读 e2e + 锁矩阵测试 + WAL 水位测试 + memory 库 e2e + 全量零回归
- **Diagnostic boundary**: `src/storage/file_storage.rs` 锁面 + checkpoint 策略接线（`src/database.rs`）+ 存储后端抽象
- **Split signals**: 只读模式牵动 MVCC 快照跨进程水位语义时拆出独立评估
- **Related changes**: None

### MS27：存储演进批（超页溢出与完整性校验） — planned

- **Status**: planned
- **Dependencies**: None（建议序在 MS26 后——T01 溢出页需同时覆盖 file/memory 两存储后端；T02 校验位与加密头 flag 同机制）
- **Outcome**: 超 ~4KB 元组经溢出页链存取可达（TEXT 长文档/JSON 场景；加密 4124B 步长联动、恢复两态一致）；数据完整性校验面可用——页校验和（格式头 flag 演进）或 `rtsql check` 全页结构扫描，方案随调查定稿（二选一或组合）
- **Rationale**: 同上调查——超页元组直接失败 `"No enough space in page"`（`src/storage/page_format/slotted_page.rs:212`）且无溢出页，agent 存长 JSON/文档为高频场景（I069 只覆盖 BLOB/DECIMAL 类型、不含此存储面）；WAL 帧有 CRC 但数据页无校验、无完整性校验命令——与 I072 备份边界同主题。两项同为存储格式演进域故聚合
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS27-T01 | 超页元组溢出页链（页格式演进 + 加密步长 + 恢复重放联动） | Explorer 2026-09-24：`slotted_page.rs:212` 超页容量即 Err |
| MS27-T02 | 完整性校验：页校验和位 或 `rtsql check` 扫描命令（方案随调查定稿） | Explorer 2026-09-24：数据页无 checksum；无 check/integrity 命令 |

- **Non-goals**: BLOB/DECIMAL 类型（I069 另议——溢出页是其前置但类型面独立）；压缩；在线重整/VACUUM 全库搬家
- **Workload**: 2 个中 change（各自独立验收）
- **Stable baseline**: 超 1 页元组明文/加密写入、读取、恢复往返一致；损坏页可检出并报告位置（采校验和或扫描方案时）；全量零回归
- **Verification boundary**: 超页往返 e2e（明文+加密）+ 恢复两态 + 损坏注入检出测试 + 全量零回归
- **Diagnostic boundary**: `src/storage/page_format/` + tuple 序列化 + `src/storage/file_header.rs` flag 面 + `src/wal/recovery.rs` 重放
- **Split signals**: 溢出页牵动 MVCC 版本链/GC 全链路改造超预期时先收 T02，T01 另行评估
- **Related changes**: None

## D-candidates（不归入当前 MS，待后续决定）

| 标题 | 不建议做的原因 | 何时重评 |
|---|---|---|
| 代价模型 + Join 重排 | 价值/复杂度不匹配 | 视后续 join 性能实测（MS09-T02 已完成） |
| B+Tree 节点级锁 | ~500 行业务代码；460 tests 未证明有争用 | 视后续性能实测（MS08 已剥离至 improvements 域） |
| clone 消除 Arc/Cow | 零拷贝 ValueRef 教训（K18）：先量化再优化 | 视后续 clone 频率 profiling |
| io_uring | 高风险低收益 | 视后续整体性能实测 |
| 瘦内部节点 | 依赖 Varint Key | I024 实施后单独评估 |
| 合并 Tag byte | 改动序列化格式 | 暂搁 |

## 长期方向（未规划具体里程碑）

- **io_uring 集成**（改进项 I028）：Linux 5.1+ tokio-uring 批量提交
- **jemalloc/mimalloc 优化 (K37)**：已转入 improvements 台账 **I066**（2026-09-24 用户指令，权威位置移至 `openspec/specs/improvements/spec.md`）

## 依赖关系图

初版后统一路线（2026-09-24 两批规划并完全重排执行序，均无硬前置，编号与执行序无关）：

```
MS23 约束面 ──(唯一索引/约束语义)──→ MS24 写面
MS23 ──(索引基建)──→ MS21 DDL/二级索引
MS18 one-shot 性能 ──(checkpoint 策略面)──→ MS26 运行形态 ──(存储后端)──→ MS27 存储演进
MS25 SQL 读面 · MS20 ATTACH 跨库（独立，序自由）
全部殿后 ──→ MS22 实测驱动性能批（量化定稿后转 ready）
```

建议执行序：**MS23 → MS24 → MS19 → MS18 → MS25 → MS20 → MS21 → MS26 → MS27 → MS22**

无环；所有依赖均已满足。初版后工作（MS18-MS27）按上方统一执行序推进，编号只表规划先后；已完成历史全部随 `ARC-202609242151` 归档。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

- **MS24 SQL 写面补全（子集 INSERT 与 UPSERT）**（2026-09-26）：change `2026-09-25-ms24-write-surface-completion` 双 Iteration（000 写入类型门与子集 INSERT/DEFAULT / 001 UPSERT 与 REPLACE INTO，其 replan Cycle 承接 PK 预检补齐与回滚后墓碑行索引条目还原）全 accepted——非键列写入值类型门端到端生效（ISS04 收口，dump/restore/import 零误报）、子集列清单 INSERT 与 DEFAULT 持久化与应用闭环（跨重启与 dump→restore 保真）、`ON CONFLICT DO NOTHING/DO UPDATE`（字面量 + `excluded.col` + 旧行裸列三形态赋值）与 `REPLACE INTO` 端到端可用（冲突仲裁确定、碰撞预检零副作用、恢复两态一致）、删除者事务回滚后 PK 与唯一索引条目按存活版本还原；全量 1239 tests / 0 failures / 2 ignored。残留 MS24-T04（声明期 DEFAULT 类型校验）未实施。
- **MS23 数据完整性约束执行面**（2026-09-25）：change `2026-09-24-ms23-constraint-enforcement` 双 Iteration（000 约束诚实化与 NOT NULL 强制 / 001 UNIQUE 强制端到端）全 accepted——NOT NULL 写入零副作用强制、CHECK/FK/方言项与非 INT/组合 UNIQUE 计划期点名拒绝、INT 列 UNIQUE 经专属唯一索引端到端强制（INSERT/UPDATE/DELETE/回滚 + 干净重开与崩溃恢复两态一致）+ F1 修复轮（唯一列非 Int 值 `KeyTypeMismatch` 守卫）；全量 1152 tests / 0 failures / 2 ignored；同轮登记 ISS04（非键列写入类型校验缺失，R29）。

历史完成记录（2026-05-24～2026-09-24，36 行）已全部归档至 carrier
`openspec/changes/archive/2026-09-24-ARC-202609242151/`（`archive/tasks-completed-roadmap.md`）；
当前项目状态见 SNAPSHOT「仓库现场」与各归档 change carrier。
## 与 OpenSpec Changes 同步

- 每个 MSxx 内的 MSxx-Txx 实施时通过 `openspec/changes/<date>-<t-tag>/` 创建 change
- 完成的 change 通过 `openspec archive` 归档
- 归档的 change carrier 保持不可变
- 新发现的问题写 `openspec/specs/improvements/spec.md` (Ixx) — **注意**：Ixx 编号待重新审视，旧 Ixx 多数已重新归位到 MSxx-Txx
- 完整迁移的旧版 entry 记录在 `.claude/legacy/2026-08-25-openspec-init-migration/COVERAGE.md`

<!-- arc: ARC-202609242151 --> 6 块已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609242151/proposal.md
