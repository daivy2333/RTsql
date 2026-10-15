# tasks — 任务与里程碑路线

> 最后更新：2026-10-15（**AntiJoin 关联参数绑定期加固收尾归档**：change `2026-10-15-antijoin-correlated-param-hardening` 单 Iteration 单任务全 accepted——T1 `AntiJoinExecutor::extract_param_values` `filter_map` 静默丢弃改 `Result` 点名报错（`correlated parameter column not found in outer row: {列名}`，与前三执行器同模）+ 内联先行单测（RED `80 passed; 1 failed`〔`Ok(None)` 静默空集〕→ GREEN 81）；`extract_param_values` 全库四处收口；1 域 delta 合并（`correlated-subquery-cache` MODIFIED「关联参数外层绑定失败显式报错」枚举扩至四含 AntiJoin），域数 64 不变；全量 2030 tests / 0 failed / 2 ignored；来源：前 change Plan Review Finding 1 + 用户「我们继续解决这个」）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

30 个 Milestone：19 completed + 6 superseded（历史段已归档至 carrier `ARC-202609242151`，本文件仅保留活跃路线）+ 5 planned（初版后统一路线，2026-09-24 规划：MS18-MS27；2026-10-01 用户批准重排，milestone-planner 增补 MS28-MS29 承接 I077/I078/I079 与残留任务；MS23 于 2026-09-25 完成，MS24 于 2026-09-26 完成〔T04 声明期 DEFAULT 类型校验未实施，2026-10-01 重排并入 MS29-T02〕，MS19 于 2026-09-26 完成，MS18 于 2026-09-26 完成，MS25 于 2026-09-28 完成，MS20 一期于 2026-09-29 完成〔T02 二期跨文件 DML 待后续 change〕，MS21/MS26 于 2026-09-30 完成，MS28 于 2026-10-01 完成，MS29 于 2026-10-01 完成，MS20 于 2026-10-02 完成（T02 二期随 change `2026-10-02-ms20-cross-db-dml` 三 Iteration accepted 归档），MS27 于 2026-10-03 完成（change `2026-10-02-ms27-overflow-integrity` 三 Iteration accepted 归档））。★初版达成★。

统一执行序（2026-10-01 用户裁定重排——功能补齐前置、性能优化殿后独立成批；沿用「批准执行序与编号无关」既定先例，编号只表规划先后，执行以本序为准）：**MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）→ MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）→ MS20-T02 跨文件 DML 二期 → MS27 存储演进批（超页溢出 + 完整性校验）→ MS22 实测驱动性能批（殿后独立，量化定稿后转 ready）**。顺序为建议序非硬依赖（各项无环、前置均已满足）。编排依据（2026-10-01）：外部评审实证的自动化使用摩擦先行收口——并发硬失败 exit 4 → 可配置等待是多 agent 并发使用的语义前提，也是 MS22 持锁窗口治理的前置；输入通道与声明语义同代码面相邻（lifecycle.rs import + ddl_dml.rs 建表/类型面）；存储格式演进体量最大放功能带末尾；性能殿后、先量化再决定（MS08 纪律）。首批排除域维持：微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058。REPL（I067）、DETACH（I076）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）、WAL fsync 组提交（I049——串行单写者模型下无效，MS22 量化时剔除）等留 improvements 域未排期。
旧优化项历史分类（各 superseded MS 原范围段）随 `ARC-202609242151` carrier 留档；未排期候选以 D-candidates 与 improvements 台账为准。
## Milestone Roadmap

### MS18：one-shot 路径性能收口 — completed

- **Status**: completed（2026-09-26——change `2026-09-26-ms18-oneshot-fixed-cost` 双 Iteration 全 accepted 收尾并归档。T01 交付：checkpoint 零工作跳过——`CheckpointManager::checkpoint` 顶部守卫 `write_count == 0 && !BufferPool::has_dirty_pages()` → 整体无操作 `Ok(0)`，九步全量路径与「close 即落盘」逐字节不变；谓词经 Plan 调查修正（I062 原文件长度探针不可用：checkpoint 后 WAL 恒含标记记录；DDL 不写 WAL 故脏页条件必需——否则纯 DDL 会话丢表），安全论证按未决工作来源穷举（design D2）；新增 `tests/checkpoint_skip_test.rs` 6 场景 + 2 单测 RED→GREEN。T02 交付：两 flavor × 三组实测（R26 口径 ×3 中位），批准判据合取 RSS 臂 +2.1% FAIL → 保守分支保持 multi_thread；I063「worker 线程是 RSS 大头」假设证伪（两 flavor one-shot RSS ≈6.5MB），current_thread 时延收益 -68%（6.45→2.03ms）留档 design D5，若以时延为主判据可重启评估（用户未决，未登记 I）。README 双语对比板块刷新 One-shot 10.8→6.5ms、Peak RSS 16.7→16.4MiB + 注记/prose 修正）
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
- **Related changes**: `openspec/changes/archive/2026-09-26-ms18-oneshot-fixed-cost/`（2026-09-26 收尾归档）

### MS19：CLI 管理面小收口（删库子命令 + 错误可操作化 + 会话边界测试加固） — completed

- **Status**: completed（2026-09-26——change `2026-09-26-ms19-cli-admin-hardening` 三 Iteration 全 accepted 收尾并归档；T01/T02/T03 与用户批准扩围的 Iteration 002（T4/T5/T6 列解析静默面收口）已交付，增量规格合并 `openspec/specs/error-reporting/`（新）+ `cli-noninteractive-shell`/`sql-transaction-statements` 两处修改；I072 备份边界实测沉淀 Runbook R30；Iteration 002 的 blocked Cycle 经 Plan Review 裁定收窄 T6 校验谓词（语料 `sql-scalar-functions` R5 优先），当前 Cycle 修复后 accepted，输出别名排序能力边界登记 I075）
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
- **Related changes**: `openspec/changes/archive/2026-09-26-ms19-cli-admin-hardening/`（2026-09-26 收尾归档）

### MS20：ATTACH 式跨库交互 — completed

- **Status**: completed（一期 T01 于 2026-09-29 完成——change `2026-09-29-ms20-attach-cross-db` 三 Iteration 全 accepted 收尾并归档：ATTACH 注册面 + 限定名路由只读跨查 + 附加库只读边界 + CTAS 主库物化；全量 1458 tests。二期 T02 于 2026-10-02 完成——change `2026-10-02-ms20-cross-db-dml` 三 Iteration 全 accepted 收尾并归档：Iteration 000 附加库普通写入与生命周期（T1 目标事务/metadata/WAL/缓冲池/回滚按目标库路由、本地表名一致；T2 全部退出路径统一关闭附加库——成功/SQL 错误/输出错误/信号取消，关闭失败在原执行成功时 exit 1 点名、已有错误保留退出码，已提交数据不因关闭或自动 checkpoint 失败撤销；T3 显式主库事务 0A000 源求值前拒绝、只读 25006 优先、DDL/CTAS 附加目标既有拒绝保持、目标库 WAL 恢复两态一致）；Iteration 001 查询结果写入与完整使用说明（T4 INSERT SELECT 查询计划/源表注册/目标列映射/新计划节点穷尽消费面；T5 源完整物化原生 Value 行→单隐式事务写入→失败还原本语句全部行与索引、冲突动作复用、主库显式事务复用事务、空源 affected_rows=0、同表只消费物化前源行；T6 README 双语与 rtsql-docs 使用面同步）；Iteration 002 缺陷收口（T7 两段附加 FROM 项隐式短名关联名——全子句面可达、显式别名优先、字面含点不误登记；T8 恢复重放按 WAL logical_id 强制落位收敛、dense-assignment 硬失败删除、K05 收窄为物理落位失败）；新增 `tests/cross_db_dml_test.rs` 17 + `tests/insert_select_test.rs` 10 + `tests/recovery_replay_convergence_test.rs` 3 + cross_db_query_test 扩展与 planner/slotted_page 单测；全量 1739 tests / 0 failures / 2 ignored；增量规格 +1 域（`cross-db-dml`）+ 5 处修改，语料 61 → 62 域）
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
- **Related changes**: 一期 `openspec/changes/archive/2026-09-29-ms20-attach-cross-db/`（2026-09-29 收尾归档）；二期 `openspec/changes/archive/2026-10-02-ms20-cross-db-dml/`（2026-10-02 收尾归档）

### MS21：DDL 演进——ALTER TABLE 与二级索引 — completed

- **Status**: completed（2026-09-30——change `2026-09-29-ms21-ddl-evolution` 六 Iteration 全 accepted 收尾并归档。T1 索引条目维护一致性收口：rekey UPDATE 回滚按 rekey 语义回退（`read_prev_index_keys` 派生键区分 rekey/非 rekey）+ SET 唯一列为 NULL 的旧唯一键还原 + UPSERT `SlotNotFound` 容忍臂与 DELETE 对齐。T2 CREATE/DROP INDEX 二级索引端到端：catalog 登记段（secondary 尾段 + `update_secondary_index_root` + `CatalogRootSlot::Secondary`）+ `IndexManager::delete_exact` 精确原语 + 建索引存量回填 + 写路径六处挂钩（四执行器 + abort A/B 趟）+ 恢复重建扩展 + 非 PK 等值 `IndexScanAll` 路由 + DROP INDEX 页释放回落。T3 ALTER TABLE ADD/DROP COLUMN：宽度感知反序列化（`deserialize_tuple_width`）+ DEFAULT 覆盖层 + 尾部加列 + 内隐式事务逐行重写删列 + 守卫矩阵（索引引用/最后一列/多操作/事务内）。用户 2026-09-29 扩围：T4 TRUNCATE TABLE（checkpoint-first + 数据页释放 + 三类索引空重建 + 无墓碑不复活 + 事务内拒绝）；T5 CREATE/DROP VIEW（catalog kind=View 行持久 + 创建期验证封死循环 + CTE 通道展开与内联逐字节一致 + DML 只读拒绝 + plan cache 全清）。二次扩围（Iter000 Issue 候选）：T6 删除原语精确化（`LeafNode::delete` 精确键验证 + DELETE 未命中零副作用）；T7 回滚 NULL-rekey 还原补口。增量规格合并 4 新域（`sql-secondary-indexes` / `alter-table-column-evolution` / `truncate-table` / `sql-views`）+ `update-index-maintenance` 扩展至 5 Requirement，语料 52 → 56 域；全量 1541 tests / 0 failures / 2 ignored）
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
- **Related changes**: `openspec/changes/archive/2026-09-29-ms21-ddl-evolution/`（2026-09-30 收尾归档；T03 来源 change 已归档 `openspec/changes/archive/2026-09-25-ms24-write-surface-completion/`）

### MS22：实测驱动性能优化（第一批） — planned（范围量化定稿后转 ready）

- **Status**: planned（候选范围经 profile/bench 定稿后转 ready——沿用 MS08「先量化再决定」纪律；2026-10-01 重排定位殿后独立成批，功能补齐带完成后启动）
- **Dependencies**: None 硬前置（建议序殿后——MS28 busy-timeout 等待语义先行，持锁窗口治理在其上才有意义；I062 已随 MS18 收口）
- **Outcome**: 经量化证实的性能积压按收益排序清收（第一批）。候选池：I049 WAL fsync 合并（组提交）/ I021 INSERT 多值批量 / I026 脏页 writev 批量写回 / I050 RowLockTable DashMap 化 / I024 Varint Key 变长编码 / I038 GC 无键行盲区 / I020 clone 消除 / I066 分配器评估（jemalloc/mimalloc）/ import 逐行 auto-commit 吞吐（I077-② 持锁窗口治理侧——攒批提交压缩最长持锁尾，与 MS28 busy-timeout 等待语义配套；2026-09-30 实测 ≈4.1 ms/行、2000 行 8.246s、30000 行 ≈123s，逐行提交含 `sync_all`；量化来源 change `2026-09-30-catalog-durability-and-cli-surface` Iteration 000 Act 实测）——每项先 bench/profile，达标立项，不达标退还 improvements 并留量化记录。**避坑注记（2026-10-01 重排）**：I049 组提交在串行单写者模型（one-shot + MS28 busy-timeout）下无效——单写者无并发提交可攒，量化定稿时剔除，除非 I071 serve 扩容路径启动
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

- **Status**: completed（2026-09-26——change `2026-09-25-ms24-write-surface-completion` 双 Iteration 全 accepted 收尾并归档；T01/T02/T03 已交付，增量规格合并 `openspec/specs/sql-write-surface/` 等四处。**残留**：T04 声明期 DEFAULT 类型校验未实施——2026-10-01 重排并入 MS29-T02）
- **Dependencies**: None（建议序在 MS23 后——T02 冲突检测面与 MS23-T01 唯一索引同面；T01 缺省语义消费 MS23 的 NOT NULL 语义）
- **Outcome**: `INSERT INTO t (col, ...) VALUES ...` 子集列清单可达——缺省列取 DEFAULT，无 DEFAULT 取 NULL、NOT NULL 列拒绝；`INSERT ... ON CONFLICT (cols) DO NOTHING | DO UPDATE SET ...` SQLite 子集语义可达（PK 与唯一索引冲突目标）；非键列写入值与列声明类型一致（ISS04）——类型不匹配值在任何写入前点名拒绝，消除「静默写坏」残余面；建表声明的 DEFAULT 字面量在计划期与列类型校验/归一（MS24-T04）——不存在「建表成功但应用默认值的 INSERT 必然失败」的声明/写入分裂形态
- **Rationale**: 同上调查——MS16 后 INSERT 列清单必须「恰为表列排列」（数量不符即拒），DEFAULT 解析持久化（`src/storage/data/table_manager.rs:28`）但无任何消费点、永不触发；UPSERT（ON CONFLICT / OR REPLACE）planner 全无匹配。两者是 agent 生成 SQL 的高频写入形态，与约束面同域相邻故紧随其后；ISS04（2026-09-25 MS23 Review 登记，R29）——MS16/MS23 已分别收口 PK 键列与唯一列的类型边缘，非键列写入面残余「静默写坏」，修复面（`build_update`/`extract_insert_values` 计划期门或执行器前置 + 两执行器）与本 MS 诊断边界完全同面，按同代码面相邻原则顺带收口；MS24-T04（2026-09-25 路线补充）——Iteration 000 Review 发现声明面零校验：Bool 列 `DEFAULT 1` 建表被接受（Iteration 000 D1 通道持久化任意字面量变体）、应用默认值的 INSERT 必然被 T03 写入类型门点名拒绝，声明期校验与写入门同规则面同文件（`ddl_dml.rs` DEFAULT 声明面），按 ISS04 并入先例顺带收口，补全 DEFAULT 闭环（解析→持久化→应用→渲染→声明期校验）最后一环
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS24-T01 | 子集列清单 INSERT + DEFAULT 应用（无 DEFAULT 列的缺省语义随调查定稿） | Explorer 2026-09-24：default_value 仅 catalog 持久化（`table_manager.rs:28`），insert 路径无消费 |
| MS24-T02 | UPSERT：ON CONFLICT DO NOTHING / DO UPDATE SET（冲突目标 PK/唯一索引） | Explorer 2026-09-24：`grep on_conflict\|or_replace` 零匹配 |
| MS24-T03 | ISS04：非键列写入值类型门——INSERT/UPDATE 值与列声明类型不一致在任何写入前点名拒绝（计划期 `build_update`/`extract_insert_values` 或执行器写入前置随调查定稿）；dump/restore/import 通道类型面是否纳入随调查裁定；可与 T01/T02 任一 change 顺带 | Issue ISS04（2026-09-25 MS23 Review 登记，R29） |
| MS24-T04（**未实施，planned → 2026-10-01 重排并入 MS29-T02**） | 声明期 DEFAULT 类型校验：CREATE TABLE 计划期校验/归一声明 DEFAULT 字面量与列声明类型一致（规则与写入类型门对齐——NULL 豁免、FLOAT 列 Int 升格、日期族 String 强制解析、其余跨类型建表点名拒绝零副作用），消除「建表成功但应用默认值的 INSERT 必然失败」分裂 | MS24 Iteration 000 Review finding（000-initial.md Follow-up Decision 2，2026-09-25；Bool 列 `DEFAULT 1` 写入期才拒绝） |

- **Non-goals**: 多值 INSERT 批量执行性能（I021——MS22 性能批候选）；REPLACE INTO 等价语法（随调查裁定）；部分索引/表达式索引冲突目标
- **Workload**: 2-4 change（T03/T04 均可顺带或独立小 change；T04 并入当前活跃 change 需用户批准扩围）
- **Stable baseline**: 省略列 INSERT 与 upsert 形态端到端可用，冲突行为与唯一索引运行期/恢复两态一致；写入值类型与列声明类型不一致被点名拒绝且零副作用；建表声明的 DEFAULT 要么与列类型兼容（含升格/解析归一）要么建表点名拒绝；全量零回归
- **Verification boundary**: 子集 INSERT/DEFAULT 矩阵 + upsert 语义矩阵（含 DO UPDATE rekey 与唯一索引交互）+ 写入值类型门拒绝矩阵 + 声明期 DEFAULT 拒绝/归一矩阵 + 恢复两态 + 全量零回归
- **Diagnostic boundary**: `src/parser/planner/ddl_dml.rs` INSERT/UPDATE 面 + `src/executor/{insert,update}.rs` + 索引冲突检测面
- **Split signals**: DO UPDATE 语义膨胀（级联类行为）时裁剪为 DO NOTHING 先行
- **Related changes**: `openspec/changes/archive/2026-09-25-ms24-write-surface-completion/`（2026-09-26 收尾归档）

### MS25：SQL 读面补全（外连接/DISTINCT/集合操作/CTE/ORDER BY 别名排序） — completed

- **Status**: completed（2026-09-28——change `2026-09-26-ms25-sql-read-surface` 五 Iteration 全部 accepted 收尾并归档。T1–T7（OUTER JOIN 三态 + WHERE×JOIN 解禁 / DISTINCT / 集合操作 / 聚合扩展 / CTE / EXPLAIN / ORDER BY 别名排序）与 T8–T13（JOIN 输出形状与名字解析收口、聚合输出形状一致性、DerivedScan 别座别名应用、信号注册时序硬化、聚合参数计划期解析统一、限定通配文案可操作化）全部交付；增量规格合并 +32 added / 5 modified / 1 removed，语料 40 → 49 域）
- **Dependencies**: None
- **Outcome**: agent 日常 SELECT 代数全形态可达——LEFT/RIGHT/FULL OUTER JOIN（非等值腿经 NLJ 通道扩展）、SELECT DISTINCT、UNION [ALL]/INTERSECT/EXCEPT、非递归 WITH（CTE，单次物化语义）；顺带聚合扩展（group_concat/COUNT(DISTINCT)）与 EXPLAIN 计划结构文本输出；ORDER BY 按顶层投影输出别名排序（I075——别名键从「静默保持输入序」升级为真实排序）
- **Rationale**: 同上调查——外连接在 `src/parser/planner/query.rs:319-321` 显式 UnsupportedJoinType（I015 只覆盖 join 算法不含外连接语义）；DISTINCT/集合操作/CTE 在 planner 与 executor 全无匹配。同主题「读面代数」聚合（MS13/MS17 批处理先例），各项独立 change 独立验收、单项失败不阻塞其余；2026-09-26 扩围——I075（ORDER BY 输出别名排序，用户授权登记）同属读面代数主题（Sort/Projection 链与 DISTINCT 同域、共享 D10 设计面），经用户批准以独立任务行并入本阶段
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS25-T01 | OUTER JOIN 三态（LEFT 必须；RIGHT/FULL 视 NLJ 改造成本随调查裁定） | Explorer 2026-09-24：`query.rs:319-321` 仅 `Inner(On)`，其余 `UnsupportedJoinType` |
| MS25-T02 | SELECT DISTINCT | Explorer 2026-09-24：planner/executor 无 Distinct 臂 |
| MS25-T03 | 集合操作 UNION [ALL]/INTERSECT/EXCEPT | Explorer 2026-09-24：无 SetOperation/UnionQuery 消费 |
| MS25-T04 | 非递归 CTE（WITH，单次物化语义） | Explorer 2026-09-24：无 With/Cte 构造臂 |
| MS25-T05 | 顺带小项（随 T01-T04 触碰相应面时随带，MS19-T03 模式）：group_concat/COUNT(DISTINCT)；EXPLAIN（自 MS21「届时裁定」转正） | Explorer 2026-09-24：聚合仅五件（`aggregate.rs:146-157`）；EXPLAIN 无臂 |
| MS25-T06 | ORDER BY 输出别名排序（别名解析序随调查定稿——SQLite 语义为输出名优先；牵动 D10「Sort 拥有投影裁剪」的 Sort/Projection 链，需独立调查；实现时同步修订语料 `sql-scalar-functions` R5 与 `error-reporting` R7 的「别名键静默保持输入序」语义） | I075（2026-09-26 用户授权登记；来源 MS19 Iteration 002 Plan Review Minor 4） |
| MS25-T07 | T08 JOIN 输出形状与名字解析收口（audit 实证三面静默缺陷，用户 2026-09-27 裁定并入） | change Iteration 004 / design D9 |
| MS25-T08 | T09 聚合输出形状一致性收口（直出行/表头错配，用户裁定不落账 Issue 并入） | change Iteration 004 / design D10 |
| MS25-T09 | T10 DerivedScan 基座别名应用收口（CTE / FROM 子查询表头回落源列名） | change Iteration 004 / design D11 |
| MS25-T10 | T11 信号处理器注册时序硬化（注册先于 open 路径锁获取） | change Iteration 004 / design D11 |
| MS25-T11 | T12 聚合参数计划期解析统一（未知列与别名引用静默 0/NULL → `ColumnNotFound` 点名，两基座一次收口） | change Iteration 004 / design D12 |
| MS25-T12 | T13 限定通配拒绝文案可操作化（`SELECT t.*` 泛化 `SELECT ITEM` → 能力点名） | change Iteration 004 / design D12 |

- **Non-goals**: 递归 CTE；窗口函数（保持远期）；LATERAL 横向引用；代价模型（I016）；SMJ（I015 剩余未排期）
- **Workload**: 2-5 change（OUTER JOIN 与 CTE 各自独立；DISTINCT 与集合操作可合；T06 独立小-中 change）
- **Stable baseline**: 上述形态端到端全绿（含 NULL 三值语义、去重语义、CTE 单次物化）；`ORDER BY <输出别名>` 按别名输出列真实排序且与 DISTINCT/表达式查询/聚合的组合语义明确；INNER/子查询既有面零回归
- **Verification boundary**: 各形态独立测试 + 别名键排序矩阵（含与 DISTINCT/表达式项组合 + 语料修订后的既有用例校准白名单）+ 全量零回归
- **Diagnostic boundary**: `src/parser/planner/{query,subquery}.rs` + `src/executor/`（新 set/dedup 执行器、join 外连接扩展）
- **Split signals**: FULL OUTER JOIN 牵动执行器框架大改时拆出（LEFT/RIGHT 先行）；CTE 牵动 plan cache key 语义时单独成 change
- **Related changes**: `openspec/changes/archive/2026-09-26-ms25-sql-read-surface/`（2026-09-28 收尾归档）

### MS26：嵌入式运行形态（只读并发/checkpoint 水位/:memory:） — completed

- **Status**: completed（2026-09-30——change `2026-09-30-ms26-embedded-runtime` 四 Iteration 全 accepted 收尾并归档（执行序 000 只读模式 → 001 `:memory:` → 003 派生投影收口+收尾批 → 002 WAL 水位；编号与执行序无关）。T1/T2 只读打开模式：`FileStorage::open_readonly[_with_key]`（共享锁 + 头校验 helper 提取 + 写守卫）+ WAL 未决谓词（`WalPendingRecovery`，取锁后判定）+ `OpenMode`/`is_readonly`/三字段 Option 化 + `readonly_rejection` 13 写变体 25006（挂 2 叶子入口覆盖 6 公开入口）+ CLI `--readonly`（子命令组合 exit 2）+ ATTACH 只读传递。T3/T4 `:memory:`：`MemoryStorage`（页拷贝/free list 优先+写零/单调 `page_count`）+ `OpenMode::Memory` + `open_in_memory()`（构造裁剪零 WAL/恢复/checkpoint）+ CLI `:memory:` 特判（`--key`/`--readonly`/六子命令 exit 2 拒绝，早于 resolve 与父目录创建）。T8 派生基座投影收口（用户指令扩入，Act 001 Issue 候选本 change 解决不落账）：`DerivedScanNode.projection` + `base_schema`/无 WHERE 装配/`get_plan_output_columns`/`is_base_scan_chain` 四处接线——FROM 子查询/CTE/视图三通道外层投影裁剪行+表头、未知列 42703、CTAS 列名校正、恒等面不回归（delta spec `join-output-shape-resolution` +R5）；T9/T10/T7 收尾批：组合拒绝自动化用例、clippy --tests 5 条清零、孤儿 `:memory:.wal` 移除、README 双语 + rtsql-docs 文档三处。T5/T6 WAL 字节水位：`WalWriter` `bytes_appended`/`checkpoint_bytes_threshold` AtomicU64（写路径同源累加）+ `Database::commit` 尾部 `maybe_auto_checkpoint` 单点（四项合取：WAL 通道∧阈值>0∧达水位∧无其他活跃事务）+ pipeline 两站点收敛（CTAS 拆两段——提交失败才 drop_table，checkpoint 失败表存活）+ 死记录数阈值接口删除（含 2 测试消费方随删，Plan 调查错误经 Act 上报裁定）+ env `RTSQL_WAL_CHECKPOINT_BYTES`（CLI 顶部校验 exit 2 + open 后单点应用，非法对全命令生效经 Review 豁免——RTSQL_KEY 同位先例）+ 文档行。增量规格 +3 域（`readonly-open-mode` / `in-memory-database` / `wal-auto-checkpoint`）+ `database-file-lock` R1 按模式收窄 + `error-reporting` R2 增 25006 + `join-output-shape-resolution` +R5，语料 56 → 59 域；全量 1590 tests / 0 failures）
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

### MS27：存储演进批（超页溢出与完整性校验） — completed

- **Status**: completed（2026-10-03——change `2026-10-02-ms27-overflow-integrity` 三 Iteration 全 accepted 收尾并归档。T01 超页溢出页链：长字符串增量 tag 0x08〔u32 长度前缀，消除 u16 回绕；catalog DEFAULT 段同语义 T12 用户指令 replan 并入〕+ 溢出页格式 page_type 0x04 与链原语模块 `src/storage/page_format/overflow.rs`〔locator 0xFF、chunk ≤4080B、防环收集/释放〕+ 写路径 >4074B 溢出分支〔先链后 locator 末步落位、失败清理零泄漏、≤4074 逐字节不变〕+ 读路径两相装配〔DataScan `YieldOverflow` 出页闭包装配 / `find_visible_version` 出锁装配、内联零拷贝保持〕+ 恢复重放链页重分配与 locator 幂等〔CREATE INDEX 回填遗漏同轮修复〕+ GC/drop/TRUNCATE 释放 + 全写通道/加密/`:memory:`/只读 e2e + ~1MB 基线含恢复〔跨库 INSERT SELECT 走子进程——ATTACH 为 CLI 会话专属〕+ README 双语/rtsql-docs 文档；T02 完整性校验：方案定稿「仅 rtsql check」（design D7）——lib `src/storage/integrity.rs` 只读结构扫描〔页 census 0x00–0x04 分类、catalog 解析、逐表数据链 walk、bounds-checked slot 迭代、溢出链逐链校验、三类索引树遍历、损坏输入零 panic〕+ CLI `rtsql check <db>`〔error 级 exit 1 / info 级不改退出码〔未引用溢出链、恢复后 tail 良性领先〕、锁 4/钥 5/`:memory:`·`--readonly` exit 2/pending WAL exit 1、全程不修改文件〕+ README 双语/SKILL.md 文档。新增 `tests/overflow_page_test.rs` 27 + `tests/check_command_test.rs` 8 + integrity 单测 6；全量 1797 tests / 0 failures / 2 ignored；增量规格合并 +2 域（`overflow-page-storage` 5 Requirement / `data-integrity-verification` 2 Requirement），语料 62 → 64 域；范围外 Issue 候选一项〔WAL 恢复重放不持久化 data_page_tail，Plan Review 002 独立核实确认，待用户裁定〕）
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
- **Related changes**: `openspec/changes/archive/2026-10-02-ms27-overflow-integrity/`（2026-10-03 收尾归档）

### MS28：CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint） — completed

- **Status**: completed（2026-10-01——change `2026-10-01-ms28-cli-concurrency-checkpoint` 两 Iteration 全 accepted 收尾并归档。T1-T4 锁等待预算策略化：`try_lock_with_backoff` 改 deadline 循环（`LOCK_VISIBILITY_FLOOR` ≈8ms 下限恒存 + `DEFAULT_LOCK_WAIT` 5000ms）+ `FileStorage::open_with_lock`/`open_readonly_with_lock` 新核心（既有 open 家族委托壳零变化）+ `Database::open_with_lock_wait`（FileStorage 构造段专用线程 + oneshot 回传——锁等待期间 open future 让出、信号分支零改动即时中断）+ CLI `--busy-timeout` global flag/env `RTSQL_BUSY_TIMEOUT`（clap 校验 exit 2、flag 优先、`:memory:` 接受忽略、`delete` 探测不适用）单点解析穿线全部开库命令与 ATTACH + 存量锁用例显式短预算 D8 校准（断言语义零变化）。T5：`rtsql checkpoint <db>` 子命令（`resolve_existing_db` 缺失 exit 1 → rw 独占开库锁冲突经预算 exit 4 → 零工作谓词 checkpoint() 前取样 → 确认行两形态〔no pending work / wal truncated〕、`--readonly`/`:memory:` exit 2 守卫自动、对钥 exit 0/错钥 exit 5）。T6：SKILL.md 命令面与 README 双语逐条同步（含只读句 DETACH 不实声明移除——Plan Review finding）。新增 `tests/busy_timeout_test.rs` 18 例 + `tests/checkpoint_command_test.rs` 6 例；全量 1657 tests / 0 failures / 2 ignored；增量规格合并 3 域修改，域数 61 不变）
- **Dependencies**: None
- **Outcome**: 并发 one-shot 调用从「立即拒绝 exit 4」变为「可配置等待、逐个原子完成」（I077-①——`--busy-timeout`/env/lib 三通道，复用 `try_lock_with_backoff` 改策略，超时维持 `DatabaseLocked` exit 4 错误面，读共享锁与写独占锁两侧统一生效）；用户可主动落盘（I079——`rtsql checkpoint <db>` 子命令，外部备份/接管文件前有显式同步点）
- **Rationale**: 多 agent 并发使用是 one-shot 形态最常触发的场景，硬失败语义与 `--format json/csv/tsv` 的自动化定位对立（2026-10-01 外部评审 #1/#3 实证）；两项同属「库生命周期与并发的 CLI 面」，共享 `src/cli/` 子命令面与错误面验证边界，按 MS19「CLI 管理面小收口」先例聚合；MS22 的持锁窗口治理以本阶段等待语义为前提，故列功能带首项
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS28-T01 | I077-①：锁等待策略可配置——`--busy-timeout` / env / lib 三通道（沿用 `RTSQL_KEY`/`RTSQL_WAL_CHECKPOINT_BYTES` 通道先例），超时后维持既有 `DatabaseLocked` exit 4 错误面；读共享锁与写独占锁两侧统一生效 | improvements I077（2026-10-01 用户方向 + 外部评审 #1） |
| MS28-T02 | I079：`rtsql checkpoint <db>` 子命令（复用 `resolve_existing_db` + 开库 + `checkpoint()`；`--readonly`/`:memory:` 组合 exit 2） | improvements I079（2026-10-01 外部评审 #3 转登记） |

- **Non-goals**: FIFO 票据队列（flock 非 FIFO，公平性诉求留给 I071 serve 扩容路径）；跨进程多写者；`rtsql serve` 复活（I071，定位为串行模型达天花板后的扩容路径）；import 攒批提交（→MS22，持锁窗口治理侧）
- **Workload**: 1-2 个小 change
- **Stable baseline**: 并发 one-shot 调用全部接受、逐个原子完成（每次调用全程持锁、重叠结构性不可能）；外部备份前有显式落盘命令；全量测试零回归
- **Verification boundary**: 锁矩阵测试（等待/超时/错误面/读写两侧）+ checkpoint 命令 e2e（WAL 截断/位点推进/零工作早退语义保持）+ 全量零回归
- **Diagnostic boundary**: `src/storage/file_storage.rs` 锁面 + `src/cli/` 子命令面 + `src/database.rs` checkpoint 接线
- **Split signals**: busy-timeout 牵动锁恢复语义重设计时拆出独立收口
- **Related changes**: `openspec/changes/archive/2026-10-01-ms28-cli-concurrency-checkpoint/`（2026-10-01 收尾归档）

### MS29：输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验） — planned

- **Status**: completed（2026-10-01——change `2026-10-01-ms29-import-input-and-default-declaration` 两 Iteration 全 accepted 收尾并归档。Iteration 000：T1 import stdin `-` 通道（`csv::Reader::from_reader` stdin 臂、records 预收集/RFC4180/逐条 auto-commit/输出语义零变化）+ T2 免建表导入（`get_table` Err 臂自动建表：`csv_field_guess` 逐字段猜测〔空→Null、i64→Int、f64→Float、其余 String〕→ `infer_ctas_column_types` pub(crate) 复用推导 → 冲突 General exit 1 点名零副作用 → DDL 经 `quote_ident` 常规建表通道（保留名/重复列名守卫自然生效）→ 恒等列映射复用既有插入循环；首列隐式主键语义——首 Cycle T2 两场景 Gate 5 阻塞（Plan 错误 ×2：误判引擎支持无主键建表、误判重复列名守卫已存在）经 Plan Review replan 修订为「沿用引擎未声明 PK → 首列隐式主键」+ T8 守卫增入）+ T3 `:memory:` import 豁免（`lifecycle_db_target` Import 放行 + `--key` × `:memory:` Usage exit 2 + memory 路由跳过 resolve/exists）+ T8 planner 重复列名计划期守卫（`build_create_table` 归一后同名 `InvalidQuery` 点名——「重复列名表静默建成且不可 dump 往返」既有缺陷闭环，Issue 候选随闭环未落账）。Iteration 001：T4 `validate_default_value` 单点（NULL 豁免、FLOAT×Int 升格归一、日期族 `coerce_datetime_write` 验证、异型 `DefaultTypeMismatch` 点名）消费于 `build_create_table`（T8 守卫后）与 `build_alter_table` AddColumn + T5 `extract_default_value` TypedString 臂（Date/Timestamp 验证后 String 目录形态——日期族 DEFAULT dump→restore 往返修复，修复前 RED＝restore 报 `Unsupported value type`）+ 新 `PlanError::DefaultTypeMismatch`（58000）+ T6 ALTER 同款规则 + T7 SKILL.md/README 双语同步。新增 `tests/import_input_surface_test.rs` 12 例 + `tests/default_declaration_test.rs` 14 例 + planner_test +6（T8 3 + T4 3）；全量 1689 tests / 0 failures / 2 ignored；增量规格合并 2 域（`cli-noninteractive-shell` import MODIFIED、`sql-write-surface` +2 Requirement 现 7），域数 61 不变）
- **Dependencies**: None
- **Outcome**: 外部数据进入通道补齐（I078——import 收 stdin `-`、目标表不存在时按 CSV 表头建表〔类型推导复用 MS20 CTAS 矩阵〕、`:memory:` 导入豁免面，「echo CSV | rtsql :memory: import -」类 Unix 组合可达）；写面声明语义闭环（MS24-T04——CREATE TABLE/ALTER ADD COLUMN 计划期校验声明 DEFAULT 字面量与列类型一致，消除「建表成功但应用默认值的 INSERT 必然失败」分裂形态）；顺带闭环既有缺陷两项——日期族 DEFAULT 的 dump→restore 往返断裂（TypedString 解析缺失）与重复列名表静默建成（planner 守卫，replan 增入）
- **Rationale**: 输出侧 `--format json/csv/tsv` 管道友好而输入侧结构性残缺（外部评审 #7 实证——`:memory:` 拒绝生命周期子命令、import 需已存在表且不收 stdin）；T04 与 I078 共享 `src/cli/lifecycle.rs` import 面 + `src/parser/planner/ddl_dml.rs` 建表/类型面诊断边界，且 T04 单独成 milestone 过薄（只改一个校验门），按同代码面相邻原则并入
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS29-T01 | I078：import 收 `-` stdin（对齐 restore 先例）+ 目标表不存在时按 CSV 表头建表（类型推导复用 `infer_ctas_column_types` 矩阵先例）+ `:memory:` 豁免面（与「主命令缺库显式拒绝、创建通道收敛 `rtsql new`」既有语义协调） | improvements I078（2026-10-01 外部评审 #7 转登记） |
| MS29-T02 | MS24-T04：声明期 DEFAULT 类型校验（规则与写入类型门对齐——NULL 豁免、FLOAT 列 Int 升格、日期族 String 强制解析、其余跨类型建表点名拒绝零副作用） | MS24 Iteration 000 Review finding（2026-09-25），2026-10-01 重排并入 |

- **Non-goals**: 列改名/改类型等 schema 演进扩展；JSON/BLOB 承载（I069）；export 通道变更；CSV 之外的输入格式
- **Workload**: 1-2 个小 change
- **Stable baseline**: stdin/内存库导入组合端到端可达（形态随调查定稿）；「建表成功但应用默认值必然失败」分裂形态消除；dump/restore/import 保真不回归；全量零回归
- **Verification boundary**: stdin/建表导入矩阵（类型推导/表头失配/`:memory:`/缺库拒绝边界）+ DEFAULT 声明期拒绝矩阵 + dump/restore 保真 + 全量零回归
- **Diagnostic boundary**: `src/cli/lifecycle.rs` import 面 + `src/parser/planner/ddl_dml.rs` DEFAULT 声明面
- **Split signals**: CSV 表头建表的类型推导需改造 CTAS 矩阵时拆出独立收口
- **Related changes**: `openspec/changes/archive/2026-10-01-ms29-import-input-and-default-declaration/`（2026-10-01 收尾归档）

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
- **jemalloc/mimalloc 优化 (K37)**：已转入 improvements 台账 **I066**（2026-09-24 用户指令，权威位置移至 `.agents/memory/improvements.md`）

## 依赖关系图

初版后统一路线（2026-09-24 规划、2026-10-01 用户批准重排：功能补齐前置、性能殿后独立，均无硬前置，编号与执行序无关）：

```
MS23 约束面 ──(唯一索引/约束语义)──→ MS24 写面
MS23 ──(索引基建)──→ MS21 DDL/二级索引
MS18 one-shot 性能 ──(checkpoint 策略面)──→ MS26 运行形态 ──(存储后端)──→ MS27 存储演进
MS25 SQL 读面 · MS20 ATTACH 跨库（独立，序自由）
MS28 并发等待语义 ──(持锁窗口治理前提)──→ MS22 import 攒批（性能批候选）
全部功能带（MS28/MS29/MS20-T02/MS27）──→ MS22 实测驱动性能批（殿后独立，量化定稿后转 ready）
```

建议执行序：**MS23 ✓ → MS24 ✓ → MS19 ✓ → MS18 ✓ → MS25 ✓ → MS20 一期 ✓ → MS21 ✓ → MS26 ✓ → MS28 ✓ → MS29 ✓ → MS20-T02 ✓ → MS27 ✓ → MS22**（截至 2026-10-03 前十二项已完成，下一项 MS22 实测驱动性能批——候选范围量化定稿后转 ready）

无环；所有依赖均已满足。初版后工作（MS18-MS29）按上方统一执行序推进，编号只表规划先后；已完成历史全部随 `ARC-202609242151` 归档。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

- **AntiJoin 关联参数绑定期加固**（2026-10-15）：change `2026-10-15-antijoin-correlated-param-hardening` 单 Iteration 单任务全 accepted 收尾归档。来源：change `2026-10-15-subquery-alias-binding-and-error-prefix` Plan Review Finding 1（T2 三执行器加固漏掉的同构点）+ 用户指令「我们继续解决这个」（不落账独立 Issue，沿 2026-10-05 先例）。T1 `AntiJoinExecutor::extract_param_values`（`src/executor/anti_join.rs:151-175`）`filter_map` 静默丢弃改 `Result`：`map` + `.copied().ok_or_else(...)` 未命中外层列报错 `correlated parameter column not found in outer row: {列名}`（`StorageError::ExecutionError`，58000 兜底），唯一调用点 `?` 上浆；命中臂 `left_row[idx].clone()` 直接索引逐字节保持（A3 不统一三处 `.get(idx)` 回退差异）；先行单测 `missing_outer_column_reports_error_naming_column`（`right_plan`/`database` 给齐防落空——`:206` 守卫否则跳过被测路径；NOT EXISTS 形态聚焦绑定期）RED `80 passed; 1 failed`（`Ok(None)` 静默空集与预测逐字一致）→ GREEN 81；`extract_param_values` 全库四处全部 `Result` 化收口。零回归：矩阵 22 + subquery_test 43 定向全绿；Iteration Gate `2030 tests / 0 failed / 2 ignored`（2029 基线 + 本单测 1，对账吻合）。Plan Review accepted（独立 diff 阅读 + 本会话重跑复核 lib 81/0 与两套件全绿 + fmt 干净；信息项一条：实施由并行执行体完成、本会话清理其重叠写入产生的重复测试块后审计）。1 域 delta 合并（`correlated-subquery-cache` MODIFIED「关联参数外层绑定失败显式报错」——枚举扩至四含 AntiJoin、修复前形态补 `filter_map` 丢弃与缓存键短项化后果），域数 64 不变、validate --specs 64/64。ISS04 维持活动目录不动。

- **子查询别名限定符绑定修复（ISS16–ISS18）与执行器错误前缀单层化（ISS19）**（2026-10-15）：change `2026-10-15-subquery-alias-binding-and-error-prefix` 单 Iteration 四任务全 accepted 收尾归档。来源：2026-10-15 用户实测评测四缺陷（ISS16 相关标量外参未绑定、ISS17 EXISTS 内层别名四静默形态、ISS18 IN×别名×索引恒空集、ISS19 执行器双层前缀）+ 用户指令（「可以，交由你来跑plan」、覆盖缺口归因并入、Gate 5 豁免「按你的建议进行改动」、「开始审计」、「进行收尾吧提交同步」）。T1 `extract_subquery_table_names` 声明限定符化（新私有辅助 `declared_qualifiers`：别名优先 + 基表名并列保留〔Gate 5 豁免方案①——别名内层保持「别名+基名限定」宽松语义，A1 由证伪转锚点 `a1_base_name_qualifier_inside_aliased_inner`〕、JOIN 臂同规则、Derived 取别名）——`qualified_column_expression`/`extract_correlated_params` 五消费方自动一致修复，零调用点改动；先行矩阵 `tests/subquery_alias_binding_test.rs` 22 用例（算子 6 面 × 内层形态 3 面 × 谓词列位置 × 相关性 3 面 × IN 索引面 + EXPLAIN `correlated_params` 虚高回落双向断言 + JOIN 形 IN 拒绝面 ×2〔含「别名不得绕开拒绝面」〕+ A1 锚点 + 无别名零回归锚点 ×3），RED 10 passed/12 failed → 豁免后 22/22。T2 三执行器 `extract_param_values`（SemiJoin/ScalarSubqueryFilter/SubqueryEval）`unwrap_or(0)` 静默回落改 `Result` 点名报错 `correlated parameter column not found in outer row: {列名}`（58000 兜底不新增分类），命中路径与缓存键语义逐字节不变；lib 单测 ×3 经 `next()` 触发（新 `#[cfg(test)] StubRowExecutor`）。T3 `StorageError::ExecutionError` thiserror 前缀去除、单层 `Execution error: ` 由 pipeline 漏斗唯一赋予（ISS11 同手法）；`tests/subquery_test.rs:632` 锚点校准（RED 先行）+ `grep "execution error: execution error"` 零命中。T4 `tests/e2e_test.rs` +5 旅程（E14：ISS16 WHERE/投影位相关标量、ISS17 EXISTS 四形态 + 审计新增 NOT EXISTS 别名与 JOIN 双臂别名、ISS18 IN×别名×索引、ISS19 单层前缀端到端，各带无别名对照）27/27。同批覆盖缺口归因（既有 57 子查询用例内层零别名、AntiJoin 零覆盖、`:632` 锁定 bug 为缺口主因）与同类面审计四新形状（NOT EXISTS/NOT IN 别名全放行、EXISTS JOIN 双臂恒空集、Derived 别名截断）随 T1 矩阵与 T4 落地。全量 2029 tests / 0 failures / 2 ignored（1999 基线 + 矩阵 22 + lib 3 + e2e 5，M19 分批串行一次，对账吻合）；fmt / clippy 零新增。Plan Review accepted（独立 diff 阅读 + 消费方矩阵核查；非阻塞 Issue 候选一项：AntiJoin `extract_param_values` `filter_map` 静默丢弃未加固——T1 后经 SQL 不可触达、R2 显式只约束三执行器，用户未裁定落账，留档 Cycle Plan Review Findings 1）。3 域 delta 合并——`sql-table-alias-resolution` +1 ADDED（现 7 Requirement）/`correlated-subquery-cache` +1 ADDED（现 6）/`error-reporting` MODIFIED「错误消息前缀单层」追加执行器通道（现 8），域数 64 不变、validate --specs 64/64。同批台账闭环：ISS16–ISS19 建账即闭（issues/archive/、R44–R47 状态 closed 含各自修复摘要）。**下一里程碑 MS22 实测驱动性能批**（候选范围量化定稿后转 ready）。

- **ISS15 table 渲染截断与组合面 e2e 补齐**（2026-10-15）：change `2026-10-14-iss15-table-truncation-and-combination-gaps` 单 Iteration 五任务全 accepted 收尾归档。T1 `render_table` 列宽封顶 + 省略号截断（`CELL_WIDTH_CAP=64`、表头同规则、chars 边界安全、widths 按截断后文本计——≥65536 单元格 std::fmt 动态宽度 panic〔ISS15，exit 101〕单点关闭；主命令与 stats/sample/profile 经 `render_rows` 委托同面覆盖；json/csv/tsv 保真逐字节不变；lib 单测边界矩阵先行 RED〔4 failed 1 passed，panic 于 render.rs:85 与契约预测一致〕→ GREEN 17 passed）+ T2 e2e 大值×四格式（E8 扩展：csv/tsv 读回 2,000,000 字符全值保真、table exit 0 恰 3 行且数据行 64 字符 `...` 结尾）与大值×分析子命令（E9 扩展：独立 big_stats 表 70,000 字符值，stats/sample/profile × table exit 0、stats × json min/max 全值——用户实测 panic 路径）+ T3 import×加密库与分析子命令×加密库（E7 扩展：对钥 import exit 0 且 COUNT 可查、错钥 exit 5、`--key pw stats enc s --format json` exit 0——key 穿线通道首次见证）+ T4 `--readonly`×stats/sample/profile 拒绝矩阵 +3（readonly_session_test 既有循环断言自动覆盖，exit 2 点名 flag 归属）+ T5 README 双语与 SKILL.md table 截断说明（64/61/`...` 三处一致）；1 域 delta 合并（`cli-noninteractive-shell`「输出格式四态」MODIFIED +「大值行集输出组合（锁定）」「import 与分析子命令非默认运行形态（锁定）」2 ADDED），域数 64 不变、validate --specs 64/64；新增 lib 单测 5 例，e2e/readonly 均为既有函数内扩展零删除行；全量 1999 tests / 0 failures / 2 ignored（1994 基线净增 5；Iteration Gate M19 分批串行一次，T5 文档-only 未重跑）；ISS15 关闭归档（R43 → issues/archive/）；journey 层缺席项（TRUNCATE/ALTER/SIGINT/WAL 水位/completions/readonly 完整旅程/schema/list）维持分层现状不提升，TTY 默认格式分支自动化结构性不可达（用户接受）；来源：2026-10-14 用户实测评测〔ISS15 panic + 组合空格扫描确认四组零见证组合〕+ 用户裁定「issue 和这些 e2e 的修复一起规划到一个 change」。

- **e2e 旅程验证与缺陷清偿批（e2e 验证面 + ISS05–ISS14 全量收口）**（2026-10-14）：change `2026-10-13-e2e-verification-and-defect-closure` 六 Iteration 全 accepted 收尾归档。Iteration 000 e2e 旅程套件（T1：`tests/e2e_test.rs` 22 测试函数〔12 passed + 10 `#[ignore]` 承载 ISS 标记〕，`--ignored` 10/10 pre-fix RED 见证；夹具三处自检修复〔管道并发读防死锁、两处假阳性断言〕；E1–E13 覆盖全生命周期/导入/崩溃恢复/索引/并发锁/跨库/加密/大值/分析子命令/输出面/派生视图/文案/错误面）。Iteration 001 索引与树结构正确性（T2：ISS05 机制调查修正——回填无辜、真凶为等值下探遇等值分隔键跳过边界子树〔`search_all_from_page_async` 闭区间下探 + `delete_exact` 同集合下探〕；同面新缺陷 B1 `shift_slots_right` 叶链指针清零、B2 `scan_all` 内部页当叶读 + page-0 终止符〔多层树全表 DML 静默失真〕顺带修复；`insert_separator` 分隔键落位审计修正；30000 行多叶等值连串双向严格相等、失败语句索引零副作用）。Iteration 002 DDL 持久化与原子性（T3：ISS06 自动建表未 checkpoint 崩溃砖化窗口关闭——catalog 写序重排〔列行先、表行最后 = commit point〕+ `flush_commit_point` 有序冲刷接线 + 失败清理零残留；ISS08 建表失败零残留可重试〔`open_or_init` 按 column_count 裁剪孤儿列行〕；ISS14 DEFAULT 超限 `catalog_column_row_fits` 计划期点名预检；DROP 类维持 checkpoint-based 边界）。Iteration 003 planner 读面（T4：ISS07 派生表/视图内层别名与表达式列外层可见——`building_from_derived` 放行扩至 DISTINCT 门 + 五分类门 + `alias_reroutable` + 视图展开置位，IN/EXISTS/标量抑制逐字节保持；T5：ISS09 SemiJoin/AntiJoin 基座全宽承载——`output_columns` 基表全列 + 节点 `projection` 下游裁剪 + `sort_projection` 两消费面对齐，`ORDER BY <非输出列>` 可用）。Iteration 004 文案批与文档（T6：ISS10 help 用户视角改写/ISS11 错误前缀单层〔parse_stage 去前缀 + 漏斗点单层赋予〕/ISS12 `column_type_sql` 标准长名/ISS13 `RTSQL_BUSY_TIMEOUT` env 非法值点名 + 断言校准矩阵〔cli_test +3、九处渲染断言、pipeline/lifecycle 单测、full_table_dml 双层前缀、ctas/import_input 等逐条记录〕；T7 README 双语五条 + 六组命令人工实测）。Iteration 005 技能文档与 IN 首列（T8：`get_subquery_first_column` 增 `IndexScan`/`IndexScanAll` 两臂——Iteration 003 Issue 候选按用户 D-4 裁定并入修复，`projected_columns` 提 `pub(crate)` 两消费方规则同源；T9 SKILL.md 五条落点 + 原 192 行页满句改计划期点名）。新增 `tests/e2e_test.rs` 22 例 + `tests/secondary_index_equality_test.rs` + `tests/ddl_crash_recovery_test.rs` 6 + `tests/default_precheck_test.rs` 5 + `tests/subquery_test.rs` +8（r8 矩阵 5 + r5 扩展 3）等；全量 1994 tests / 0 failures / 2 ignored（1929 基线净增 65）；8 域 delta 合并（`sql-secondary-indexes`/`sql-views`/`cli-noninteractive-shell` 各 MODIFIED ×2、`join-output-shape-resolution` ADDED ×2、`error-reporting`/`sql-write-surface`/`wal-recovery-replay-integrity`/`in-subquery-join-rejection` 各 ADDED ×1——共 7 ADDED + 3 MODIFIED），域数 64 不变；每 Iteration 一次全量预算（M19 分批）逐轮执行；来源：2026-10-13 用户双指令 + 2026-10-12 外部评测 13 项反馈（R32 分析，10 项落账 ISS05–ISS14，ISS05 机制段以本 change 调查为准）

- **外部评测反馈批（计划形状/恢复收敛/JOIN 装配/子查询面/全表写/SQLSTATE/文档）**（2026-10-06）：change `2026-10-04-external-review-gaps` 四 Iteration 全 accepted 收尾归档。Iteration 000 计划形状与恢复正确性批（T1 IndexScanAll 列元数据契约统一——`columns` 恒 base_schema、输出形状由 projection 唯一决定，二级索引等值非首列投影 panic 与静默错位双缺陷关闭，EXPLAIN/聚合/CTAS/INSERT SELECT 消费面对齐；T2 恢复会话 WAL 收敛——`RecoveryResult.wal_had_tx_records` → 零工作守卫第三条件，未提交 SIGKILL 后纯读会话 close/checkpoint 收敛、`check`/`--readonly` 永久 exit 1 楔死解除、marker-only 形态收敛、MS18 零工作跳过保持；T3 JOIN 基座 ORDER BY 限定键；T9 自连接侧身份正确性——交换判定按声明限定符、`JoinSide`/`output_sides` 显式侧标志归属、双腿同侧 `InvalidQuery` 点名）〔含 000-initial blocked → 001-replan：T9 经用户裁定并入〕。Iteration 001 子查询面与 JOIN 装配收口批（T4 WHERE 标量子查询比较 + 投影位形状——`ScalarSubqueryFilterNode/Executor` 逐行参数注入 + 语句级缓存、多行拒绝/零行 NULL 三值、JOIN 基座维持拒绝、投影恒等门摘除消除全表列泄漏；T5 FROM 派生表内集合操作——`building_from_derived` 独立标志、IN/EXISTS 内维持拒绝；T11 多表链式 JOIN 装配正确性——非末步链前缀全组合承载、链前缀列绝对偏移重定基、条件键（真实表,列）限定解析，计划失败/panic/静默串列三形态全关闭；T10 JOIN 基座输出列序 SELECT 化——kept 三清单按 SELECT 序重建、WHERE/ORDER BY 裁剪路径随之）〔001-subquery/000-initial 原始计划未执行即被 001-replan 取代——用户 2026-10-05 裁定 T10/T11 并入〕。Iteration 002 写面与 CLI 可观测批（T6 无 WHERE 全表 UPDATE/DELETE——第一重门移除、执行器 `scan_all` 全表臂复用单行逻辑、逐行碰撞隐式事务回滚、附加库路由继承、TRUNCATE 对照零变化；T7 CLI 错误输出携带 SQLSTATE——文本行尾 ` [SQLSTATE xxxx]` + JSON `sqlstate` 字段缺省省略，退出码与文案零变化、PG 面零变化）。Iteration 003 文档同步批（T8 README 双语 + SKILL.md 按 D8 七项清单逐条对齐：JOIN 基座限定排序、标量子查询边界、全表写声明、SQLSTATE 输出面、FROM 集合操作、纯读恢复收敛话术；Act/Review 双语逐句位核对 + 人工实测六组命令）。新增 `tests/recovery_wal_convergence_test.rs` + `tests/scalar_subquery_comparison_test.rs` 14 + `tests/chain_join_test.rs` 9 + `tests/self_join_test.rs` + `tests/full_table_dml_test.rs` 10 + table_alias/set_operations/join_output_resolution/secondary_index/subquery/cli/readonly_session 等扩展；全量 1929 tests / 0 failures / 2 ignored（1882 基线 + Iter001 +35 + Iter002 +12）；10 域 delta 合并（`cli-noninteractive-shell` +1 / `cross-db-dml` ~1 / `join-output-shape-resolution` +2 / `ms07-rest-tx-checkpoint-pushdown` ~1 / `sql-expression-evaluation` +2 / `sql-secondary-indexes` +1 / `sql-set-operations` +1 / `sql-table-alias-resolution` +1~1 / `sql-write-surface` +1 / `wal-recovery-replay-integrity` +1——共 10 ADDED + 3 MODIFIED），域数 64 不变；来源：2026-10-04 外部评测 8 条反馈〔7 成立 1 不成立；8a/8b 随 T4/T6 语义修复消解、8c 当前版本不复现无工作项〕+ Iteration 000 Review 两范围外 Issue 候选（T10/T11，用户裁定并入不落账）。

- **practical relational core 缺口批（建表页初始化 + 聚合嵌入 + UPDATE 表达式 + 非 INT PK 诚实化 + 恢复错误面）**（2026-10-04）：change `2026-10-03-practical-core-gaps` 三 Iteration 全 accepted——Iteration 000 正确性与诚实化小批：T1 建表首数据页初始化（`create_table_with_constraints` 在 `allocate_page()` 后经 `SlottedPage::init` 初始化 0x03 页头，全建表通道单点覆盖；裸建表即含零行表的库 `rtsql check` exit 0 单行干净；`check` 对 0x00 页 error 分级保持；存量 0x00 head 最小修复语义——open 不自愈、首次写入/TRUNCATE 转绿）；T2 非 INTEGER 显式 PRIMARY KEY 计划期拒绝（`UnsupportedConstraint("NON-INTEGER PRIMARY KEY")` 五类型 × 列级/表级矩阵；隐式首列主键与 import 通道零波及；存量非 INT PK 表可用）；T3 dump DDL 渲染适配（`create_table_sql` 仅 INTEGER PK 输出 `PRIMARY KEY`，存量表 dump→restore 往返可达）。Iteration 001 聚合表达式嵌入：T4 提取-注册基建 + SELECT 项嵌入与常量混排（`contains_aggregate_expr` + 条件 Projection 包装通道）；T5 HAVING 完整表达式编译；T6 ORDER BY 聚合表达式键；T7 拒绝面收口（WHERE/ON 聚合、嵌套聚合、NonAggregated 固化）。Iteration 002 写面能力 + 遗留并入：T8 UPDATE 计划面多列化（`UpdateAssignment`/`UpdateValue` 节点演进、右值五臂路由、子查询 `UPDATE SUBQUERY` 点名）；T9 执行器多列化（同旧行快照求值、per-assignment 门序、预检前置、Step 7 按 `pk_assigned` 三分支）；T10 SKILL.md/README 双语同步；T11 恢复重放目标表缺失定向错误面（`redo_table_lookup_error` 变体判别——`TableNotFound` 四要素文案〔表名/catalog 缺失/未 checkpoint 关闭原因/打开拒绝〕，其余归并文案逐字节不变，K05 打开拒绝不变）；T12 聚合显示名重名去歧义（`compile_aggregate_row_expr` 按规范名解析 + `register_aggregate_calls` 去重键改规范名 + wrap 臂按注册序号定位，S24 四形态修复）。新增 `tests/update_expression_test.rs` 14 + `tests/recovery_error_surface_test.rs` 3 + aggregate_extension +5 + planner +7 + Iter000/001 测试扩展；全量 1862 tests / 0 failures / 2 ignored（1802 基线净增 60）；增量规格 5 域各 +1 Requirement（`data-integrity-verification` 现 3 / `wal-recovery-replay-integrity` 现 6 / `sql-aggregate-extensions` 现 5 / `sql-constraint-enforcement` 现 7 / `sql-write-surface` 现 9）+ `cli-noninteractive-shell` dump/restore MODIFIED，域数 64 不变；来源：2026-10-03 外部评测探针实测四缺口（2026-10-03 用户四项集中决策批准）+ Iter000 遗留 Issue 候选并入（用户指令）。

- **WAL 恢复重放 tail 收敛修复（恢复收尾写回 + 写路径自愈）**（2026-10-03）：change `2026-10-03-recovery-tail-persistence` 单 Iteration 双 Cycle 全 accepted——父 Cycle `000-initial` Act 阶段 T1 契约核读触发 Stop-when 阻塞返 Plan（Plan「三臂全经 `redo_tuple_at_row_id`」断言失实：Delete 墓碑臂直调 `write_tuple_to_data_page` 绕行，原 `table_last_page` 写回集合对该类表欠覆盖且计划内 INSERT 驱动测试不可见），Plan Review `rework-required`；rework Cycle `001-rework` 修复：T1-R1 写回集合改为 catalog 表清单 + 内存/持久 tail 差异比较（结构性覆盖任意重放臂，失败 `RedoFailed` 点名表、`redo_count == 0` 零变化）+ T1-R2 DELETE-only 重放见证（28B 墓碑 footprint 几何 + 夹具自检防失准，对旧机制必失败的直接 gap 见证）；T2 写路径 `heal_stale_tail` 自愈（内存 tail 页 `next ≠ 0` 沿链至 terminal、内存/持久双写回、健康路径单次探测零行走查、恢复面共享原语隔离保持）；T3 `check_command_test` 恢复后形态断言白名单校准（info 行 → 干净单行 exit 0）。新增 `tests/recovery_tail_persistence_test.rs` 4 用例 + `stale_tail_heals_on_write` 单元测试；全量 1802 tests / 0 failures / 2 ignored（1797 基线 + 净增 5）；增量规格 `wal-recovery-replay-integrity` +3 Requirement 现 5（域数 64 不变）；来源：MS27 Plan Review 002 确认的 Issue 候选（用户裁定立 change 解决，集中决策 A 双层修复）。

- **MS27 存储演进批（超页溢出页链 + rtsql check 完整性校验）**（2026-10-03）：change `2026-10-02-ms27-overflow-integrity` 三 Iteration 全 accepted——Iteration 000 溢出表示与存储原语：T1 长字符串 tag 0x08（serialize/deserialize 三入口 + compute_tuple_size + 65535/65536 切换点）；T2 `overflow.rs` 溢出页格式（page_type 0x04）与链原语（locator 编解码/chunk 分解/链装配读/收集释放）；T3 写路径 >4074B 溢出分支（先链后 locator、失败清理零泄漏）；T4 读路径两相装配（`read_tuple_from_data_page` Overflow 装配 + DataScan `YieldOverflow` + `find_visible_version` 出锁装配、内联零拷贝保持）。Iteration 001 生命周期、恢复与全通道：T5 恢复重放链页重分配 + locator 末步落位幂等 + `build_pk_version_maps`/`rebuild_pk_indexes`/`derive_old_row_id` 四消费点两相装配（CREATE INDEX 回填遗漏同轮修复）；T6 GC/drop/TRUNCATE 溢出链释放（delete 单点 + `collect_overflow_chain_pages` 接线）；T7 全通道与形态 e2e + ~1MB 基线（矩阵/tag 切换点/CTAS/跨库/加密/`:memory:`/只读/dump-restore/import）；T8 README 双语 + SKILL.md 长值边界（含 DEFAULT 字面量 ~4KB catalog 页内容量约束）；T12 catalog DEFAULT 段长形式（`[0x08][u32 len]`，>65535B；2026-10-02 用户指令 replan 并入、design D11 表示一致性硬化）。Iteration 002 rtsql check：T9 lib `integrity.rs` 只读结构扫描（页 census/catalog/数据链 walk/bounds-checked slot/溢出链/三类索引树、损坏零 panic、自实现 walk 替代 tolerant 收集器）；T10 CLI `check` 子命令（readonly 编排、error/info 分级——恢复后 tail 良性领先 info 不改退出码、退出码 0/1/4/5/2 矩阵、主库文件全字节不变）；T11 README 双语命令表 + SKILL.md Lifecycle/Behavioral notes。新增 `tests/overflow_page_test.rs` 27 + `tests/check_command_test.rs` 8 + integrity 单测 6；全量 1797 tests / 0 failures / 2 ignored；增量规格 +2 域（`overflow-page-storage` / `data-integrity-verification`），语料 62 → 64 域；Plan Review 002 三项 Minor finding 记录不返工；范围外 Issue 候选一项（WAL 恢复重放不持久化 data_page_tail）待用户裁定。

- **MS20 二期跨文件 DML（附加库写入 + INSERT SELECT + 缺陷收口）**（2026-10-02）：change `2026-10-02-ms20-cross-db-dml` 三 Iteration 全 accepted——Iteration 000 附加库普通写入与生命周期：T1 目标事务/metadata/WAL/缓冲池/回滚路由（abort 表键用 metadata 本地名，四写执行器经既有路由取目标库资源）；T2 全部退出路径统一关闭附加库（AttachSession 所有权移 `execute_command_inner_mode` 编排器、select 外持有；关闭失败原执行成功时 exit 1 点名、已有错误/信号保留退出码）；T3 显式事务 0A000（源求值与 begin 前点名、主库目标+附加源不误拒）、只读 25006 优先、DDL/CTAS 附加目标拒绝保持、目标库恢复两态一致。Iteration 001 查询结果写入与文档：T4 `PhysicalPlan::InsertSelect` 节点 + `build_insert` 可变借用房 `build_query` 源编译、源表注册进 `extract_all_table_names`、目标列映射/全宽 DEFAULT/NULL 填充、新节点不进 plan cache；T5 源完整物化原生 Value 行（不经 JSON/SQL 往返）→ 单隐式事务写入、失败回滚本语句全部行与索引、ON CONFLICT/REPLACE 复用、主库显式事务复用事务、空源 affected_rows=0；T6 README 双语 + rtsql-docs 使用面。Iteration 002 缺陷收口：T7 两段附加 FROM 项隐式短名关联名（尾段经既有 `register_table_alias` 登记、`qualifier_matches_declared` 全子句面匹配接线、显式别名优先、字面含点不误登记）；T8 恢复重放按 WAL logical_id 强制落位（`SlottedPage::add_slot_with_logical_id` 恢复面专用、dense-assignment 硬失败删除、K05 收窄为页满/IO、运行期零变化）；新增 `tests/cross_db_dml_test.rs` 17 + `tests/insert_select_test.rs` 10 + `tests/recovery_replay_convergence_test.rs` 3 + cross_db_query_test +4 与 planner/slotted_page 单测；全量 1739 tests / 0 failures / 2 ignored；增量规格 +1 域 `cross-db-dml` + 5 处修改（`cross-db-namespace-query` 附加库只读边界收窄、`readonly-open-mode` R5、`sql-write-surface` +INSERT SELECT、`sql-table-alias-resolution` R1、`wal-recovery-replay-integrity` R1），语料 61 → 62 域。

- **MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）**（2026-10-01）：change `2026-10-01-ms29-import-input-and-default-declaration` 两 Iteration 全 accepted——Iteration 000 import 输入面：T1 stdin `-` 通道（对齐 restore 先例、records 预收集/逐条 auto-commit 零变化）；T2 目标表缺失按 CSV 表头自动建表（`csv_field_guess` 猜测 → `infer_ctas_column_types` pub(crate) 复用推导、冲突 General exit 1 点名零副作用、常规建表通道守卫自然生效、首列隐式主键语义〔INT 首列重复 DuplicateKey fail-fast、FLOAT/STRING 重复合法〕——首 Cycle T2 两场景 Gate 5 阻塞经 replan 修订〔Plan 错误 ×2：引擎无表级无主键、重复列名守卫不存在〕）；T3 `:memory:` import 豁免（`--key` exit 2、其余子命令维持拒绝）；T8 planner 重复列名计划期守卫（replan 增入，既有「静默建成且不可 dump 往返」缺陷闭环）；Iteration 001：T4 `validate_default_value` 单点建表校验/归一（NULL 豁免/FLOAT 升格/日期族解析验证/异型 `DefaultTypeMismatch` 点名）+ T5 TypedString 臂（日期族 DEFAULT dump→restore 往返修复）+ T6 ALTER ADD COLUMN 同款 + T7 SKILL.md/README 双语同步；新增 `tests/import_input_surface_test.rs` 12 例 + `tests/default_declaration_test.rs` 14 例 + planner_test +6；全量 1689 tests / 0 failures / 2 ignored；增量规格合并 2 域（`cli-noninteractive-shell` import MODIFIED、`sql-write-surface` +2 Requirement 现 7），域数 61 不变。

- **MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）**（2026-10-01）：change `2026-10-01-ms28-cli-concurrency-checkpoint` 两 Iteration 全 accepted——T1-T4 锁等待预算策略化（storage `open_with_lock`/`open_readonly_with_lock` deadline 循环 + lib `open_with_lock_wait` 专用线程让出、信号即时中断 + CLI `--busy-timeout`/env 三通道穿线全部开库命令与 ATTACH、默认 5000ms/显式 0 仅下限、存量锁用例 D8 校准断言零变化）；T5 `rtsql checkpoint <db>` 显式落盘（零工作明示两形态、缺库 exit 1、锁冲突经预算 exit 4、`--readonly`/`:memory:` exit 2、加密对钥 exit 0/错钥 exit 5）；T6 SKILL.md/README 双语命令面同步（含只读句 DETACH 不实声明移除）；新增 `tests/busy_timeout_test.rs` 18 例 + `tests/checkpoint_command_test.rs` 6 例；全量 1657 tests / 0 failures / 2 ignored；增量规格合并 3 域修改（`cli-noninteractive-shell` +2 Requirement、`database-file-lock` R1、`readonly-open-mode` R1 + Purpose 同步），语料域数 61 不变。

- **MS26 嵌入式运行形态（只读并发 + `:memory:` + WAL 水位 + 派生投影收口）**（2026-09-30）：change `2026-09-30-ms26-embedded-runtime` 四 Iteration 全 accepted——T1/T2 只读打开模式（共享锁多读单写 + WAL 未决拒绝 + 零写入会话 + 13 写变体 25006 + CLI `--readonly` + ATTACH 只读传递）；T3/T4 `:memory:` 库形态（`MemoryStorage` + `open_in_memory()` + CLI 特判与拒绝面、零文件退出即弃）；T8 派生基座外层投影收口（FROM 子查询/CTE/视图三通道裁剪行+表头、未知列 42703，2026-09-30 用户指令 Issue 候选本 change 解决）+ T9/T10/T7 收尾批（组合拒绝用例 / clippy 清零 / 孤儿移除 / 文档三处）；T5/T6 WAL 字节水位（提交边界 `maybe_auto_checkpoint` 单点、16 MiB 默认 + env/lib 双通道、0 禁用、顺延、失败传播且 CTAS 表存活、死接口删除）；增量规格 +3 域 + 3 处修改，语料 56 → 59 域；全量 1590 tests / 0 failures。

- **MS21 DDL 演进与二级索引（CREATE/DROP INDEX + ALTER TABLE + TRUNCATE + VIEW + 索引一致性收口）**（2026-09-30）：change `2026-09-29-ms21-ddl-evolution` 六 Iteration 全 accepted——T1 索引条目维护一致性收口（rekey UPDATE 回滚按 rekey 语义回退、SET 唯一列为 NULL 的旧键还原、UPSERT `SlotNotFound` 容忍臂与 DELETE 对齐）；T2 二级索引端到端（catalog secondary 尾段 + `CatalogRootSlot::Secondary` + `IndexManager::delete_exact` + 建索引回填 + 写路径六处挂钩 + 恢复重建扩展 + 非 PK 等值 `IndexScanAll` 路由 + DROP INDEX 页释放回落）；T3 ALTER TABLE ADD/DROP COLUMN（`deserialize_tuple_width` 宽度感知 + DEFAULT 覆盖层 + 尾部加列 + 内隐式事务逐行重写删列 + 索引引用/最后一列守卫）；用户扩围 T4 TRUNCATE TABLE（checkpoint-first 序 + 数据页释放 + 三类索引空重建 + 无墓碑崩溃不复活 + 事务内拒绝 + 空表幂等）、T5 CREATE/DROP VIEW（catalog kind=View 行持久 + 视图注册表 + 创建期验证封死循环 + CTE 通道展开与内联逐字节一致 + 显式列清单 + DML 只读拒绝 + 非基表 DDL 目标拒绝 + plan cache 全清 + 附加库引用创建期拒绝）；二次扩围 T6 删除原语精确化（`LeafNode::delete` 精确键验证，DELETE 未命中零副作用不误删邻近条目/不误报 KeyNotFound）、T7 回滚 NULL-rekey 还原补口（SET 唯一列为 NULL 回滚后旧值占用恢复）；增量规格 +4 域（`sql-secondary-indexes` / `alter-table-column-evolution` / `truncate-table` / `sql-views`）+ `update-index-maintenance` 扩展至 5 Requirement，语料 52 → 56 域；全量 1541 tests / 0 failures / 2 ignored（净增 4 测试文件 47 例 + catalog lib 单测）。

- **MS20 一期 ATTACH 式跨库交互（ATTACH 注册 + 跨库只读查询 + CTAS 主库物化）**（2026-09-29）：change `2026-09-29-ms20-attach-cross-db` 三 Iteration 全 accepted——T1 ATTACH 会话注册面（`resolve_db_path` 语义复用、独占锁 `try_lock` 冲突 exit 4、自附加 flock 同 inode 拒绝、重复别名/路径缺失零副作用拒绝、lib session-only 点名拒绝、DETACH 豁免登记 I076）；T2 两段限定名路由（`alias.table` 全表名消费点先查注册表、未命中扁平名回落零回归、三段列引用路由命中可用、`AttachRoute`/`RoutedTables` 计划期判定执行期取库、附加库缺表 42P01 点名、含附加库引用语句不进 plan cache）；T3 附加库只读边界（五写消费点计划期点名拒绝、DDL 不查 catalog 即标记）；T4 CTAS 主库物化（列名取 SELECT 输出表头、类型推导矩阵——全 NULL→String(255) 含零行退化/Int∪Float→Float/同型保留/其余混合 `CtasColumnTypeConflict` 建表前点名拒绝、输出列名重复计划期拒绝、目标已存在守卫前置物化、建表→`flush_all()`→隐式事务插入崩溃一致性序、失败 abort+尽力 drop 零副作用同名可重试、隐式主键 DuplicateKey、`CREATE TABLE o.m AS SELECT` 计划期拒绝、LIKE/CLONE/列清单+query/表约束+query 诚实化拒绝、显式事务内 execute 期点名拒绝 0A000、执行后 plan cache 全清）；Review 补跑 Act 未覆盖的全量串行 Gate（1458 passed / 0 failed / 2 ignored）；增量规格 +3 域合并（`sql-attach-session`/`cross-db-namespace-query`/`create-table-as-select`，含事务内收口语义与零行源退化两处校准），语料 49 → 52 域。
- **MS25 SQL 读面补全（外连接/DISTINCT/集合操作/CTE/ORDER BY 别名排序）**（2026-09-28）：change `2026-09-26-ms25-sql-read-surface` 五 Iteration 全 accepted——T1–T7（OUTER JOIN 三态经 NLJ 统一承载 LEFT/FULL + RIGHT 计划期左右交换、WHERE×JOIN 解禁含 INNER、SELECT DISTINCT 三分支链位、UNION[ALL]/INTERSECT/EXCEPT 左结合展开与非递归 CTE、COUNT/SUM/AVG DISTINCT 真实去重与 group_concat、EXPLAIN 计划树文本不执行内层、ORDER BY 输出别名真实排序含 decorate-sort-undecorate）与 T8–T13（JOIN 基座聚合参数/分组键完整形状解析 + 同名裸名歧义 `AmbiguousColumn` 点名 + 未知列 `ColumnNotFound`、聚合直出行/表头形状一致性收口、DerivedScan 基座普通列别名应用、信号处理器注册先于 open 锁获取、聚合参数两基座计划期解析——未知列与别名引用由静默 0/NULL 转 `ColumnNotFound` 点名、`SELECT t.*` 拒绝文案点名 QUALIFIED WILDCARD）；收口三处静默错误结果/静默不出错缺陷（DISTINCT 静默不去重、CTE 静默丢弃、COUNT DISTINCT 静默忽略）；增量规格合并 +32 added / 5 modified / 1 removed，语料 40 → 49 域；全量 1408 tests / 0 failures / 2 ignored。
- **MS18 one-shot 路径性能收口**（2026-09-26）：change `2026-09-26-ms18-oneshot-fixed-cost` 双 Iteration 全 accepted——checkpoint 零工作跳过（`CheckpointManager::checkpoint` 顶部守卫：`write_count == 0 && !BufferPool::has_dirty_pages()` → 整体无操作早退 `Ok(0)`，九步全量路径与「close 即落盘」逐字节不变；谓词经 Plan 调查修正为双条件——checkpoint 后 WAL 恒含标记记录致文件长度探针永假、DDL 不写 WAL 致单条件丢表，安全论证按未决工作来源穷举；新增 `tests/checkpoint_skip_test.rs` 6 场景 + 2 单测 RED→GREEN 见证，one-shot 时延 12.35→5.85ms 背靠背）+ runtime flavor 实测定形（两 flavor × 三组实测 R26 口径 ×3 中位，批准判据合取 RSS 臂 +2.1% FAIL → 保守分支保持 multi_thread；I063 RSS 假设证伪与 current_thread 时延 -68% 留档 design D5）+ README 双语对比板块刷新（One-shot 10.8→6.5 ms、Peak RSS 16.7→16.4 MiB、注记与 prose 归因修正）；增量规格合并 `openspec/specs/ms07-rest-tx-checkpoint-pushdown/`（+1 Requirement「Checkpoint 零工作跳过」，现 4 个）；全量 1285 tests / 0 failures / 2 ignored。
- **MS19 CLI 管理面小收口**（2026-09-26）：change `2026-09-26-ms19-cli-admin-hardening` 三 Iteration 全 accepted——`rtsql delete <db>` 删库子命令（锁探测同窗口删除、伴生清理、逐文件报告、`--dry-run`、缺失/占用拒绝、加密库免密钥、corrupt 可删）+ I072 文件对备份边界 7 形态实测与 README 双语声明（沉淀 Runbook R30）；`PlanError` 携带能力名 + PG 协议错误码按类分类（0A000/42601/42P01/42703/42702，其余 58000 兜底，JSON 字节不变）+ 会话事务 × 边界子句组合 e2e 锁定；列解析静默面收口（未知投影列/未知排序键计划期 `ColumnNotFound` 点名拒绝、纯查询 `col AS alias` 别名表头生效、WHERE/值表达式列缺失重分类 42703；T6 谓词经 Plan Review 裁定收窄为「输入列集 ∪ 顶层投影输出别名」以保持语料 R5 既有静默语义）；全量 1277 tests / 0 failures / 2 ignored。
- **MS24 SQL 写面补全（子集 INSERT 与 UPSERT）**（2026-09-26）：change `2026-09-25-ms24-write-surface-completion` 双 Iteration（000 写入类型门与子集 INSERT/DEFAULT / 001 UPSERT 与 REPLACE INTO，其 replan Cycle 承接 PK 预检补齐与回滚后墓碑行索引条目还原）全 accepted——非键列写入值类型门端到端生效（ISS04 收口，dump/restore/import 零误报）、子集列清单 INSERT 与 DEFAULT 持久化与应用闭环（跨重启与 dump→restore 保真）、`ON CONFLICT DO NOTHING/DO UPDATE`（字面量 + `excluded.col` + 旧行裸列三形态赋值）与 `REPLACE INTO` 端到端可用（冲突仲裁确定、碰撞预检零副作用、恢复两态一致）、删除者事务回滚后 PK 与唯一索引条目按存活版本还原；全量 1239 tests / 0 failures / 2 ignored。残留 MS24-T04（声明期 DEFAULT 类型校验）未实施。
- **MS23 数据完整性约束执行面**（2026-09-25）：change `2026-09-24-ms23-constraint-enforcement` 双 Iteration（000 约束诚实化与 NOT NULL 强制 / 001 UNIQUE 强制端到端）全 accepted——NOT NULL 写入零副作用强制、CHECK/FK/方言项与非 INT/组合 UNIQUE 计划期点名拒绝、INT 列 UNIQUE 经专属唯一索引端到端强制（INSERT/UPDATE/DELETE/回滚 + 干净重开与崩溃恢复两态一致）+ F1 修复轮（唯一列非 Int 值 `KeyTypeMismatch` 守卫）；全量 1152 tests / 0 failures / 2 ignored；同轮登记 ISS04（非键列写入类型校验缺失，R29）。

历史完成记录（2026-05-24～2026-09-24，36 行）已全部归档至 carrier
`openspec/changes/archive/2026-09-24-ARC-202609242151/`（`archive/tasks-completed-roadmap.md`）；
当前项目状态见 SNAPSHOT「仓库现场」与各归档 change carrier。
## 与 OpenSpec Changes 同步

- 每个 MSxx 内的 MSxx-Txx 实施时通过 `openspec/changes/<date>-<t-tag>/` 创建 change
- 完成的 change 通过 `openspec archive` 归档
- 归档的 change carrier 保持不可变
- 新发现的问题写 `.agents/memory/improvements.md` (Ixx) — **注意**：Ixx 编号待重新审视，旧 Ixx 多数已重新归位到 MSxx-Txx
- 完整迁移的旧版 entry 记录在 `.agents/legacy/2026-08-25-openspec-init-migration/COVERAGE.md`

<!-- arc: ARC-202609242151 --> 6 块已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609242151/proposal.md
