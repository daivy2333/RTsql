# Iteration 000 / Cycle 000: 计划形状与恢复正确性批

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1（IndexScanAll 列元数据契约统一）、T3（JOIN 基座 ORDER BY 限定键）、T2（恢复会话 WAL 收敛）
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: proposal R1/R3/R2 与场景 S1–S12；design D1/D3/D2；基线 1862 tests / 0 failures / 2 ignored（HEAD 9538647，SNAPSHOT 采信）
- Excluded scope: 子查询面（T4/T5，Iteration 001）；写面与 SQLSTATE（T6/T7，Iteration 002）；文档（T8，Iteration 003）；PK IndexScan 契约与范围谓词回落路径的任何改动

**Objective**

外部评测正确性红线三连清零：①二级索引等值扫描在任意投影组合下表头与行对齐、不 panic（执行路径与 EXPLAIN 双面）；②JOIN 基座 ORDER BY 两段限定键真实排序（含自连接同名列精确解析）；③崩溃恢复后纯读会话 WAL 收敛（check/readonly 不再永久拒绝、checkpoint 消息如实）。全量测试零回归。

**Investigation Facts**

- Current Baseline: HEAD 9538647、工作区干净、release 二进制新于全部源码；基线 1862 tests / 0 failures / 2 ignored（2026-10-04 practical-core-gaps 收尾采信，SNAPSHOT）。三缺陷均已于本会话实测复现（复现命令与输出见会话记录；S1 panic `query.rs:47:36` 与 `plan_format.rs:348:61`、S2 静默错位、S6 死循环逐字与 spec 失败边界吻合）。
- Current-State Evidence:
  - **T1**：`IndexScanAllNode`（`src/executor/plan.rs:155`）构造点唯一——`src/parser/planner/query.rs:1824-1843`（二级索引等值路由臂，MS21 引入提交 2f83dd9f）：`columns` 在 `!sort_due && projection_indices.is_some()` 分支赋 `projection_columns`（已裁剪），否则 `base_schema`；`projection` 恒为基表列号。消费方契约分裂：`get_plan_output_columns`（query.rs:95 `projected_columns(&node.columns, &node.projection)`，CLI 表头唯一来源 cli/mod.rs:861，另被派生/CTE/set-op/聚合输入 query.rs:1956/CTAS/视图/INSERT SELECT 宽度消费）与 EXPLAIN（plan_format.rs:124-134 → `columns()` 344-351）按「基表全列 × 投影号」裁剪 → 越界 panic（S1/S3）或错位（S2）；执行器构造 pipeline.rs:1411-1437 只消费 `node.projection`（行永远正确）；`pipeline.rs:2028-2037` `extract_column_indices` 的 IndexScanAll 臂按输出形状假设（唯一反向消费方，JOIN 侧不经单表 WHERE 路由、实际不可达）。PK 路由 `IndexScanNode` 臂（query.rs:1804-1813）装配同型但消费方（query.rs:88-94、plan_format.rs:111-119）持「columns 已收窄、不再裁剪」契约——**PK 臂与其消费契约不动**。测试锁定面：`tests/secondary_index_test.rs:340-357`（EXPLAIN 只断言节点名不锁形状）、`:359-380`（lib API 不经 CLI 表头，碰巧通过）、cli_test I034 子集投影表头只覆盖 DataScan——无任何测试断言 IndexScanAll 投影表头，修复为纯增量。
  - **T3**：失败链——`extract_column_name`（query.rs:3354-3364）解出两段名 → 主 ORDER BY 装配点（query.rs:2319-2364）`qualifier_in_scope`（query.rs:763-766）通过后**丢弃限定符**产出裸名键 → 键校验集 `sort_columns = get_plan_output_columns(plan_with_aggregate)`（query.rs:2382）在无 WHERE 无聚合 JOIN 基座上已被过滤为 SELECT 清单列（query.rs:617-640；`carry_all_columns` 判定 query.rs:1321 = `selection.is_some() || projection_has_aggregates`，ORDER BY 不参与）→ 裸名查不到 → `ColumnNotFound`（query.rs:2446）。带 WHERE 的 JOIN 正常（`carry_all_columns` 置位 → 全组合列 → `where_join_projection` 裁剪 query.rs:1773-1782/2397-2398）；单表正常（基座 Scan/DataScan 全 schema，Sort 拥有裁剪 query.rs:1717-1721/2395-2396）。既有设施：`PlanBuilder.table_aliases`（mod.rs:219）、`register_table_alias`（mod.rs:283-286）、`resolve_table_qualifier`（mod.rs:292-294）、`qualifier_matches_declared`（mod.rs:315-320）、`join_row_layout`（query.rs:3032-3042）、`resolve_layout_index`（query.rs:3048-3078）、`resolve_join_projection_indices`（query.rs:3122-3178）、`OrderByColumn.expr` 表达式键 decorate 通路（SortExecutor sort.rs:61-105，装配先例 query.rs:2464-2487）。spec `sql-table-alias-resolution`「限定名排序与既有语义零回归」（spec.md:102，JOIN 场景 spec.md:106-110）要求该行为——代码违约。回归守卫点：DISTINCT 键校验（query.rs:2434-2438）、集合操作尾 ORDER BY（query.rs:848-871，限定键 TableNotFound）、聚合基座（query.rs:2328-2344）。现有 JOIN 排序测试只有单表用例（tests/table_alias_resolution_test.rs:270/281/306）。
  - **T2**：楔死链——未提交元组只存在于 WAL 不在数据页；纯读读写打开的恢复 `will_redo=false`（recovery.rs:607-609）→ `redo_count=0` → `persist_replayed_tails`/`rebuild_pk_indexes` 跳过、`mark_uncommitted_aborted` 只读不置脏 → 零页写入零 WAL 追加 → close 零工作守卫（checkpoint.rs:150-156 `get_write_count()==0 && !has_dirty_pages()`）成立 → WAL 原样 → `assert_no_pending_wal`（database.rs:37-54，拒绝一切非 Checkpoint 记录）永久 exit 1；`RecoveryResult` 消费点（database.rs:331-340）只消费 tx 集合与水位，`redo_count` 无人消费；CheckpointManager 构造在恢复之后（database.rs:352-362）；CLI checkpoint zero_work 取样 lifecycle.rs:226-243（其注释「恢复重放的 redo 页经 BufferPool 置脏」对零 redo 形态不成立——顺带修正注释）；`Database::close` = `self.checkpoint()`（database.rs:555-557），doc 注释（database.rs:549-550）"crash-recovery replayed pages — still takes the full checkpoint path" 的前提在零 redo 形态不成立。测试锁定面：checkpoint.rs:224-281 单测（零追加+零脏页 → 0）、tests/checkpoint_skip_test.rs S1/S2/S6（marker-free 干净库跳过——修复后语义保持，该形态 `wal_had_tx_records=false`）、S5（redo>0 崩溃恢复收敛——本就成立零变化）、tests/check_command_test.rs:260-303（种子为已提交 INSERT，redo>0 路径零变化）。无任何测试锁定楔死形态。
- Code and Critical Path:
  - T1：`query.rs:1836-1840` 条件分支删除 → `columns` 恒 `base_schema.clone().unwrap_or_default()`；`pipeline.rs::extract_column_indices` IndexScanAll 臂按 `node.projection` 裁剪对齐 IndexScan 臂口径；数据流不变（执行器行路径零接触）。
  - T3：三处装配——① query.rs:1318-1323 `carry_all_columns` 加 `|| !query.order_by.is_empty()`；② query.rs:2390-2401 `sort_projection` 为裸 Join/NLJ 基座补臂（`resolve_join_projection_indices` 解析 SELECT 裁剪索引）；③ ORDER BY 限定键（query.rs:2350-2354 现丢弃限定符处）改为经 `resolve_table_qualifier` 归一 + `join_row_layout`/`resolve_layout_index` 解析组合行绝对索引，命中产出 `OrderByColumn { column: <显示名>, expr: Some(ColumnExpression { column_index }) }`（表达式键豁免列集校验，既有先例 query.rs:2464-2487；SortExecutor decorate 通路消费）；未命中（未知限定符）维持 `TableNotFound`/`ColumnNotFound` 既有拒绝。
  - T2：`src/wal/recovery.rs` `RecoveryResult` 增 `wal_had_tx_records: bool`（`records.iter().any(|(_, r)| !matches!(r, WalRecord::Checkpoint{..}))`，records 于 recovery.rs:558-559 可得）；`src/wal/checkpoint.rs` `CheckpointManager::new` 增参存字段、守卫加 `!self.pending_wal_at_open`；`src/database.rs` 构造点传参；`src/cli/lifecycle.rs:226-243` zero_work 取样 OR 同标志（`Database` 需暴露该状态或 checkpoint 返回值区分——实现取 `checkpoint()` 返回 0 时比对标志，机制留 Act 在「不改 checkpoint() 签名」约束下选择，消息两形态语义为契约）。

**Implementation Guidance**

顺序：T1 → T3（同文件相邻装配，先修形状契约再在其上加 JOIN 排序臂）→ T2（独立故障域）。T3 的 `sort_projection` JOIN 臂与 Iteration 001 T4 共享，本 Iteration 只按 T3 需求面实现（ORDER BY 存在时的 JOIN 基座形状），T4 届时复用。T2 遵守 D2 方案 A（否决 B/C）；`checkpoint()` 公开签名不变。三任务均不建身份型证据或判定层；测试使用项目既有测试栈（lib 单测 + tests/ e2e + assert 式断言）。

**Task Contracts**

### T1: IndexScanAll 任意投影组合输出对齐且不 panic

- Requirement/Scenario: R1 / S1–S5
- Depends on: None
- Targets: `src/parser/planner/query.rs`（IndexScanAllNode 构造臂 :1833-1843）、`src/pipeline.rs::extract_column_indices`（IndexScanAll 臂 :2028-2037）
- Current behavior: `SELECT name FROM q6 WHERE salary=90`（salary 有二级索引）panic exit 101（query.rs:47 / plan_format.rs:348 双点）；`SELECT name, id …` 静默表头错位；EXPLAIN 同型 panic。
- Required behavior: columns 恒为基表全列名、输出形状由 projection 决定；S1–S5 全部通过；`get_plan_output_columns`/EXPLAIN/聚合输入/派生/CTAS/INSERT SELECT 宽度各消费面在 IndexScanAll 输入下正确。
- Preserve: PK IndexScan 臂（query.rs:1804-1813）与其「已收窄」消费契约逐字节不动；执行器行路径（pipeline.rs:1436、index_scan_all.rs）不动；范围谓词/非索引回落路径不动。
- Forbidden: 不得改 IndexScanNode 消费方契约反向统一；不得为表头另建映射层（契约单点修复）。
- Test witness: `tests/secondary_index_test.rs` 新增投影矩阵（后位单列/乱序多列/EXPLAIN/PK 与范围零回归/扫描上聚合）——先写 RED（当前 panic 即 RED）。
- GREEN condition: 新矩阵全绿 + 既有 secondary_index/cli/projection 套件零回归。
- Verification: `cargo test --test secondary_index_test --test cli_test` 决定性输出 + 全量 `cargo test` 0 failed。
- Stop when: 修复引发非 IndexScanAll 面的形状回归（说明契约理解有误）→ 返回 Plan。

### T3: JOIN 基座 ORDER BY 限定键真实排序

- Requirement/Scenario: R3 / S10–S12
- Depends on: T1（同文件装配链，先保证形状契约正确）
- Targets: `src/parser/planner/query.rs`（carry_all_columns 判定 :1318-1323、sort_projection 装配 :2390-2401、ORDER BY 限定键装配 :2350-2354、键校验循环 :2421-2449）
- Current behavior: 无 WHERE 无聚合 JOIN 基座 `ORDER BY e.id`/`e.salary`/`emp.id` 全部 `Column not found` exit 3；自连接同名列裸名 first-hit 局限。
- Required behavior: S10 三形态真实排序；S11 自连接限定列精确解析；S12 单表/带 WHERE JOIN/聚合/DISTINCT/集合尾 ORDER BY 零变化；未知限定符维持点名拒绝。
- Preserve: 输出别名优先语义、既有裸键排序、未知键拒绝（spec 既有段）；集合操作尾 ORDER BY 限定键 `TableNotFound` 边界；DISTINCT 与聚合基座形状。
- Forbidden: 不得解锁三段限定名（仅 attach 复合键既有边界）；不改 SortExecutor（表达式键通路既有）。
- Test witness: `tests/table_alias_resolution_test.rs` 新增 JOIN 排序矩阵——先写 RED（当前 Column not found 即 RED）。
- GREEN condition: 新矩阵全绿 + order_by_alias_sort/join_output_resolution/select_distinct/set_operations 既有套件零回归。
- Verification: `cargo test --test table_alias_resolution_test --test order_by_alias_sort_test` + 全量。
- Stop when: 形状翻转波及 DISTINCT/集合尾产生语义变化 → 返回 Plan（design 需扩回归面）。

### T2: 恢复会话 WAL 收敛（含零 redo 形态）

- Requirement/Scenario: R2 / S6–S9
- Depends on: None
- Targets: `src/wal/recovery.rs`（RecoveryResult）、`src/wal/checkpoint.rs`（CheckpointManager::new + 零工作守卫）、`src/database.rs`（构造点接线）、`src/cli/lifecycle.rs`（checkpoint zero_work 取样与消息）
- Current behavior: 未提交事务 SIGKILL → 纯读读写打开恢复（数据正确）→ close 零工作跳过 → WAL 滞留 → check/`--readonly` 永久 exit 1、checkpoint 报 "no pending work; wal untouched"；marker-only WAL 同态楔死。
- Required behavior: 打开时 WAL 曾含事务记录的会话 close/checkpoint 执行完整 checkpoint 截断 WAL；此后 check exit 0、readonly 正常打开；checkpoint 消息 `wal truncated; checkpoint site advanced`；marker-free 干净库零工作跳过保持（S8）；redo>0 路径零变化（S9 对照既有）。
- Preserve: checkpoint() 公开签名；MS18 零工作语义在 `wal_had_tx_records=false` 形态逐字节保持；恢复数据正确性语义；checkpoint_skip_test S1/S2/S6 既有断言零修改通过。
- Forbidden: 不得改 readonly 拒绝面判定（assert_no_pending_wal）；不得在 open 内联 checkpoint（D2 否决方案 B）；不新建 CLI/协议字段。
- Test witness: 新 `tests/recovery_wal_convergence_test.rs`（SIGKILL 夹具镜像 check_command_test 既有崩溃夹具模式）——先写 RED（当前死循环形态即 RED）。
- GREEN condition: 新测试全绿 + checkpoint_skip/check_command/recovery_tail_persistence 套件零回归。
- Verification: `cargo test --test recovery_wal_convergence_test --test checkpoint_skip_test --test check_command_test` + 全量。
- Stop when: 标志传播需要改 checkpoint() 公开签名或 readonly 判定 → 返回 Plan。

**Invariants**

- 存储格式、网络协议字节（除 Iteration 002 显式放宽项）、plan cache key 语义零变化。
- 全部既有 spec 语料行为零回归（本 change delta 均为 ADDED 或显式 MODIFIED）。
- 不建身份型证据工程/判定层；验证判定使用项目测试框架原生退出码与输出。
- T1/T3 均不得触碰 `IndexScanNode`（PK 臂）契约——两契约并存是现状事实，本 change 只统一 IndexScanAll 侧。

**Non-goals**

- 子查询面（T4/T5）、无 WHERE 写面（T6）、SQLSTATE 渲染（T7）、文档（T8）——后续 Iteration。
- JOIN 基座 WHERE 子查询、一般谓词 UPDATE/DELETE、IN/EXISTS 内集合操作——范围收口于 proposal。

**Acceptance**

- S1–S12（proposal 场景草图）全部可观察通过；R1/R2/R3 的 spec delta 场景与任务测试一一对应（RTM 见 tasks.md）。
- 全量 `cargo test` 0 failed（基线 1862 + 新增净增量）；`cargo fmt --check` 与 `cargo clippy --tests` 干净。

**Verification**

- 直接目标：`cargo test --test secondary_index_test --test table_alias_resolution_test --test recovery_wal_convergence_test`（各自新增用例 RED→GREEN）。
- 受影响边界：`cargo test --test checkpoint_skip_test --test check_command_test --test cli_test --test order_by_alias_sort_test --test select_distinct_test --test set_operations_test --test projection_test`。
- 集成 Gate：全量 `cargo test` 0 failed。
- 判定：各命令退出码 0 + 原生输出行数统计，不新增判定脚本。

**Gate 2 Readiness**

PASS——调查完整（三路 Explore 事实 + 本会话复现实测，HEAD 9538647）、设计闭合（D1/D2/D3 含否决路线）、任务契约齐备（T1/T3/T2 全字段）、RTM 8/8 Covered 无 Missing、验证逐 scenario 最简直接判定且无身份型证据工程、Persisted Evidence `none` 满足白名单。Gate 1 与计划于 2026-10-04 获用户批准（原话「批准」，含 proposal 四项默认决策确认）。

**Persisted Evidence**

- Mode: none

（全部验证可由测试命令 + Act Response 承载；崩溃夹具为测试内代码，无需留存现场。）

**Risks and Notes**

- T3 形状翻转（carry_all_columns 加 ORDER BY 条件）理论上影响 JOIN 基座表头形状——守卫点已列入回归面（join_output_resolution/aggregate_output_shape）；若出现非 ORDER BY 场景的表头漂移，属实质问题返回 Plan。
- T2 的 `wal_had_tx_records` 判定按记录类别（非 Checkpoint 即真），覆盖未提交记录、已提交记录、marker-only 三形态；与 `redo_count` 解耦是有意为之（S9 marker-only 形态 redo_count=0 但仍需收敛）。
- 行号为 9538647 revision 快照，Act 以符号定位为准。

## Act Response

- Status: blocked

### 实际改动

**T1（完成，Gate 3/4 通过）**

- `src/parser/planner/query.rs` — IndexScanAll 构造臂（原 :1833-1843）：删除 `!sort_due && projection_indices.is_some()` 条件分支，`columns` 恒置 `base_schema.clone().unwrap_or_default()`（D1 契约单点修复）；PK IndexScan 臂未动。
- `src/pipeline.rs::extract_column_indices` — IndexScanAll 臂改为按 `node.projection` 对 `node.columns` 裁剪后建立输出位映射（对齐 IndexScan 臂「已收窄」口径；空投影 = 全列）。
- `tests/secondary_index_test.rs` — 新增投影矩阵 5 用例（S1 后位列不 panic / S2 乱序行序 / S3 EXPLAIN 契约 / S4 PK·范围·通配零回归 / S5 扫描上聚合）。
- `tests/cli_test.rs` — 新增 `test_secondary_index_scan_projection_headers`（CLI 表头与行逐位对齐，S1 panic + S2 错位双形态的 RED 承载）。

RED 见证（修复前实测）：lib EXPLAIN 用例 panic `plan_format.rs:348:61` exit 101；CLI `SELECT name FROM q6 WHERE salary=90` panic `query.rs:47:36` exit 101；`SELECT name, id` 表头 `["id","name"]` vs 行 `["Ada",1]` 静默错位（与 proposal 逐字一致）。GREEN：S1–S5 全过 + cli_test 119/projection_test 6 零回归。

**T3（实现完成；S10/S12 GREEN，S11 阻塞——见 Blocker Handoff）**

- `src/parser/planner/query.rs` — 三处：① `build_from_clause_with_projection` 调用点 `carry_all_columns` 加 `|| !query.order_by.is_empty()`（:1321）；② `sort_projection` 装配补裸 Join/NLJ 基座臂（`subquery_evals.is_empty()` 时经 `resolve_join_projection_indices` 解析 SELECT 裁剪索引，标量子查询项维持恒等承载，与 WHERE 路径 `filter_projection` 让位条件同款）；③ 新增 `PlanBuilder::resolve_order_by_layout_key`（:780，匹配序：① `qualifier_matches_declared` 直接匹配声明段键；② 全表名书写经段键归一〔别名 → 真实键〕匹配——单表面全表名恒可达语义在 JOIN 基座同规则；自连接多段归一同键首段命中，与既有裸名首匹配同源；两步未命中回落既有裸名键路径；命中段缺列 `ColumnNotFound` 点名）+ ORDER BY 装配闭包接线（`!has_aggregates && subquery_evals.is_empty()` 且布局存在时限定键产出 `ColumnExpression` 表达式键，decorate-sort-undecorate 通路既有；聚合查询 Sort 在聚合输出行上，限定键维持既有裸名键路径——首轮实施缺此门导致两个既有聚合用例执行期越界，自检发现后加门修复）。
- `tests/table_alias_resolution_test.rs` — 新增 JOIN 排序矩阵 3 用例（S10 别名/全表名/非主键 DESC/右表多键四形态真实排序 + S11 自连接限定列精确解析〔`#[ignore]`，见 Blocker Handoff〕+ S12 单表/带 WHERE JOIN/聚合/DISTINCT/集合尾 `TableNotFound` 五面零回归）。

RED 见证（修复前实测）：S10/S11 全形态 `Column not found` exit 3（与 proposal 一致）；S12 五面基线先于实现确认全过。GREEN：S10 四形态 + S12 五面 + order_by_alias_sort(14)/join_output_resolution(7)/select_distinct(12)/set_operations(14)/aggregate_output_shape(8) 零回归。

**T2（完成，Gate 3/4 通过）**

- `src/wal/recovery.rs` — `RecoveryResult` 增 `wal_had_tx_records: bool`（`records` 中任一非 `Checkpoint` 记录即真，按记录类别判定与 redo 解耦；空 WAL/WAL 缺失路径 Default=false）。
- `src/wal/checkpoint.rs` — `CheckpointManager` 增 `pending_wal_at_open` 字段；`new` 增第 4 参（直接构造方传 `false` 语义不变）；零工作守卫加 `!self.pending_wal_at_open` 合取项（MS18 全量路径逐字节不变）；增 `pending_wal_at_open()` 访问器。
- `src/database.rs` — open 恢复段提升 `pending_wal_at_open = recovery_result.wal_had_tx_records` 到构造作用域；`CheckpointManager::new` 构造点传参；新增 `pub fn wal_pending_at_open()` 访问器（无管理器 → false）。`checkpoint()` 公开签名不变。
- `src/cli/lifecycle.rs` — checkpoint 命令零工作取样改 `get_write_count()==0 && !has_dirty_pages() && !db.wal_pending_at_open()`（消息两形态契约成立），顺带修正「恢复重放 redo 页置脏」的失实注释（零 redo 形态不置脏）。
- 机械适配 5 个 `CheckpointManager::new` 测试调用点（`src/wal/checkpoint.rs` 单测、`tests/checkpoint_skip_test.rs`、`tests/checkpoint_test.rs` ×2、`tests/recovery_test.rs`，传 `false` 断言零变化）。
- 新增 `tests/recovery_wal_convergence_test.rs` 5 用例（S6 未提交 SIGKILL → 纯读会话收敛〔含已提交可见/未提交不可见数据正确性〕+ S7 checkpoint 消息报告实际执行 + S8 零工作跳过保持〔WAL/位点字节不变〕+ S9 marker-only 收敛 + redo>0 对照零变化；崩溃夹具沿用 `wal_buffer.shutdown() + drop` 纪律）。

RED 见证（修复前实测）：S6/S7/S9 三用例失败（check 永久 exit 1 / 消息 "no pending work; wal untouched" / marker-only 同态楔死）；S8 与 redo 对照本就通过（与 D2 既有测试核对结论一致）。GREEN：5 用例全过 + checkpoint_skip(11)/check_command(11)/checkpoint_command(6)/recovery_test(3) 零回归。

### 与计划的偏差

1. **T3 fix 3 增门**：Plan Context 建议对 JOIN 基座限定键一律布局解析；Act 发现聚合查询的 Sort 位于聚合输出行（非组合行）上，一律解析产生执行期 `Column index out of bounds`（两个既有聚合用例 RED）。偏差：加 `!has_aggregates && subquery_evals.is_empty()` 门（聚合限定键维持既有裸名键路径）。属契约内非实质局部差异（设计意图「Sort 输入行 = 组合行时布局解析」的显式化）。
2. **T3 布局解析实现为 `resolve_order_by_layout_key` 而非直接复用 `resolve_layout_index`**：后者不覆盖「声明别名后书写全表名」形态（spec S10 `ORDER BY emp.id` SHALL 可达）；新 helper 两步匹配（声明段键直接匹配 → 段键归一匹配）并复用 `qualifier_matches_declared`/`resolve_table_qualifier` 语义。属等价局部实现差异。
3. **T2 消息机制**：Plan 留 Act 二选一（暴露状态 vs 返回值区分）；`Database::checkpoint()` 返回 `Result<()>` 无区分能力，采用「Database 暴露 `wal_pending_at_open()` + CLI 取样 AND `!pending`」臂，符合「不改 checkpoint() 签名」约束。
4. **S11 用例 `#[ignore]`**：Blocker（见下）在案期间以 ignore 保留见证，避免全量 Gate 被已知阻塞项污染；解除阻塞后移除 ignore 即为 GREEN 判定。
5. **Minor：`tests/cross_db_dml_test.rs` 一处 6 行 → 1 行的 rustfmt 机械归一**（`cargo fmt` 对 HEAD 既有长行的格式漂移，非行为修改；`cargo fmt --check` 为本 Cycle Acceptance 项，保留该归一）。

### Blocker Handoff

- **偏差 task/step/Gate**：T3 test witness R3-S11（`tests/table_alias_resolution_test.rs::self_join_same_name_columns_resolve_by_qualifier`，现 `#[ignore]`）；Gate 4。
- **Plan 预期 vs 实际**：Plan（design D3 / spec delta R3-S11 场景）假设自连接 Hash/NLJ Join 输出行本身正确，ORDER BY 布局解析即可实现「同名列按限定符精确解析」。实测：**自连接等值 JOIN 的输出行被既有执行层双重缺陷破坏**，与 ORDER BY 修复无关：
  1. **条件交换误判**（`src/parser/planner/expression.rs::resolve_qualified_column` :87-105 + `src/parser/planner/ddl_dml.rs::extract_join_conditions` :208-226）：`resolve_column_ref` 把书写限定符归一为真实表名，交换判定 `left_ref.table == right_table` 在自连接中对**两腿恒真**——`ON e1.id = e2.manager_id`（左腿先写形态）被误判交换，Hash 键取自错误一侧，行配对错误。
  2. **输出行归属失效**（`src/executor/join.rs::build_output_row` :112-122，`src/executor/nested_loop_join.rs` :79-102 同型）：`col.table_alias == left_table_name` 按真实表名归属，自连接两侧同真实名 → 全部列读自同一侧，`e1.x`/`e2.x` 投影静默读错表。
- **证据（改动前 release 二进制复现，非本 change 引入）**：`SELECT e1.name, e2.name FROM emp2 e1 JOIN emp2 e2 ON e1.id = e2.manager_id` → `[["r1","r1"],["r2","r2"]]`（期望 `[["m1","r1"],["m2","r2"]]`）；反写形态 `ON e2.manager_id = e1.id` → `[["m1","m1"],["m2","m2"]]`（配对正确但归属仍错）。既有语料唯一自连接用例 `ON a.id = b.id`（两侧行值恒同）恰好掩盖两缺陷。HEAD 9538647 + 本 change 代码同样成立（本会话插桩复核）。
- **影响**：R3-S11 的「各按限定符对应表的列排序」在该缺陷修复前不可观察；自连接含不同行配对的查询存在静默错误结果（正确性红线级别）。T3 的 ORDER BY 修复本身不依赖该缺陷（S10/S12 已见证）。
- **已完成**：T1 全部；T2 全部；T3 实现 + S10/S12 见证。
- **未开始**：后续 Iteration（001/002/003）——按 Iteration 顺序编排，本阻塞不涉其契约。
- **修改文件与工作区状态**：见上「实际改动」；工作区含本 change 目录与上述源/测试修改，未提交。
- **已通过 Gate**：T1 Gate 3/4；T2 Gate 3/4；T3 Gate 3（部分——S10/S12）。
- **恢复条件**：用户三选一——(a) 授权 replan：本 change 增补任务修复自连接条件交换与输出归属（新 spec delta，Pair/Plan 补调查后并入当前 Iteration 或新 Iteration）；(b) 裁定独立 change/Issue 处置，本 change S11 验收按 (a) 之外方式调整（Plan Review 裁定）；(c) 维持现状（S11 验收缺口显式保留）。恢复指令下达后 Act 从 Blocker Resolution 继续。

### 验证

- T1 目标：`cargo test --test secondary_index_test` → **27 passed / 0 failed**（exit 0）；`cargo test --test cli_test test_secondary_index_scan_projection_headers` → 1 passed。
- T3 目标：`cargo test --test table_alias_resolution_test` → **18 passed / 0 failed / 1 ignored**（exit 0）；回归面 order_by_alias_sort(14)/join_output_resolution(7)/select_distinct(12)/set_operations(14)/aggregate_output_shape(8) 全过。
- T2 目标：`cargo test --test recovery_wal_convergence_test` → **5 passed / 0 failed**（exit 0）；checkpoint_skip(11)/check_command(11)/checkpoint_command(6)/recovery_test(3) 零回归。
- 受影响边界：projection_test(6)/readonly_session/cli 错误面（经 cli_test 119 passed / 2 ignored）。
- 集成 Gate（全量，6 批 + lib + bins 分批串行，`cargo test` 原生输出）：批次 1（20 目标，含本 change 三目标套件）除 S11 `#[ignore]` 前的已知阻塞失败外全绿；批次 2–6 + lib 合计 **1605 passed / 0 failed**；lib 394 passed / 0 failed；bins 0。汇总（S11 ignore 后）：**1875 passed / 0 failed / 3 ignored**（基线 1862/0/2 + 净增 13 新用例，S11 转入 ignore 名单）。
- `cargo fmt --check` 干净；`cargo clippy --tests` 干净（新代码 0 告警）。

### Persisted Evidence

- Mode: none（Plan 设定）。Blocker 现场可由上列命令与 release/debug 二进制复现（未提交 → 楔死形态 / 自连接错行均为确定性复现），无需留存文件。

### Experience Candidates

- **Issue 候选（范围外实质缺陷）**：自连接等值 JOIN 双重缺陷——①条件交换判定用归一真实表名（`resolve_qualified_column` → `extract_join_conditions` :208-226），自连接左腿先写形态恒误判交换，行配对错误；②输出行归属 `col.table_alias == left_table_name`（`join.rs::build_output_row` :112-122，`nested_loop_join.rs` 同型）无法区分同表两侧，投影静默读错表。既有语料 `ON a.id = b.id`（两侧行值恒同）掩盖。证据：本 Act Response Blocker Handoff 的 release 二进制复现 + 会话插桩记录。建议用户调用 `openspec-experience-recorder` 登记，或经 replan 并入修复。

### 未解决问题

- R3-S11 验收缺口（见 Blocker Handoff 恢复条件 a/b/c）；Iteration 001/002/003 未开始。

## Plan Review

- Reviewed: 2026-10-04（Plan 独立审计：完整 diff 逐文件阅读 + Blocker 四条代码路径独立阅读 + 改动前 release 二进制行为级定向复现 + 新增测试断言逐条核对）
- Inputs: 本 Cycle Plan Context、Act Response（blocked）、工作区完整 diff（13 修改 + 2 新增）、`tests/recovery_wal_convergence_test.rs`、三任务目标测试新增段

### 代码与声明核对

- **T1（核实一致，Gate 3/4 采信）**：`query.rs` IndexScanAll 构造臂 `columns` 恒置 `base_schema.clone().unwrap_or_default()`（D1 契约单点修复）；PK `IndexScanNode` 臂条件分支逐字节原样（Preserve 满足）；`pipeline.rs::extract_column_indices` IndexScanAll 臂按 `node.projection` 裁剪对齐 IndexScan 口径，空投影 = 全列。测试见证非空洞：`secondary_index_test` 5 用例断言精确行值/表头/EXPLAIN 形状，`cli_test::test_secondary_index_scan_projection_headers` 断言逐位对齐。
- **T2（核实一致，Gate 3/4 采信）**：标志链完整——`RecoveryResult.wal_had_tx_records` 按记录类别判定（空 WAL 早退经 `..Default::default()` 覆盖为 false，语义正确）；`CheckpointManager::new` 第 4 参 + 守卫取反项置于首位（flag=false 时全量路径逐字节不变）；`database.rs` 构造点传参 + `wal_pending_at_open()` 访问器，`checkpoint()` 公开签名未变；`lifecycle.rs` 取样 AND `!pending` + 失实注释顺带修正。4 个直接构造调用点机械传 `false`、断言零变化（diff 核实）。新测试文件 5 用例与 S6–S9 一一对应。
- **T3 部分（核实一致，S10/S12 采信）**：三处装配（carry_all 加序 / sort_projection 裸 JOIN 臂 / 布局键解析）与契约相符；聚合基座不触布局解析（S12 聚合面零回归断言在案）；集合尾限定键 `TableNotFound` 拒绝边界有显式断言。

### 偏差分类（Plan 遗漏 / Plan 错误 / Act 偏离 / 基线变化 / 新证据）

1. **T3 聚合门**（Act 偏离，非实质）：`!has_aggregates && subquery_evals.is_empty()` 门为 D3「Sort 输入行 = 组合行时布局解析」意图的显式化；Act 自检发现两个既有聚合用例执行期越界后加门，回归面全绿。可接受。
2. **`resolve_order_by_layout_key` 替代 `resolve_layout_index`**（Act 偏离，等价实现增强，非实质）：核实 `resolve_layout_index` 仅有「书写限定符 ↔ 声明段键」单步匹配，确不覆盖「声明别名后书写全表名」形态（S10 `ORDER BY emp.id` SHALL）；新 helper 两步匹配复用既有语义原语。可接受。
3. **T2 机制选择**（Act 偏离，契约内，非实质）：Plan 留白二选一，Act 取「Database 访问器 + CLI 取样合取」臂，遵守「不改 `checkpoint()` 签名」约束。可接受。
4. **S11 `#[ignore]`**（Blocker 协议执行）：ignore 属性带阻塞注记指向 Blocker Handoff，解除后移除即为 GREEN 判定。程序正确。
5. **cross_db_dml_test 格式归一**（机械，非行为）：diff 核实为 rustfmt 对 HEAD 既有长行的等价重排，无语义变化。

### Blocker Handoff 独立核实（成立）

- **缺陷 1（条件交换恒误判）代码层确认**：`resolve_qualified_column` 返回的 `ColumnRef.table` 携带归一真实键（`expression.rs` 注释明示「ON 腿左右交换判定按真实名工作」），自连接两腿归一同名 → `extract_join_conditions` 的 `left_ref.table == right_table` 判定恒真，左腿先写形态被误判交换。
- **缺陷 2（输出行归属失效）代码层确认**：`join.rs::build_output_row` 与 `nested_loop_join.rs::build_output_row` 均按 `col.table_alias == left_table_name` 归属，自连接两侧真实名相同 → 全部列恒读左侧行，右侧永不被读。
- **行为级独立复现**：以早于本 change 的 release 二进制（构建于 HEAD 9538647）实测——正向 `ON e1.id = e2.manager_id` 得 `[["r1","r1"],["r2","r2"]]`、反向 `ON e2.manager_id = e1.id` 得 `[["m1","m1"],["m2","m2"]]`，与 Act Response 证据逐字节一致。**非本 change 引入**，且属「自连接含不同行配对的查询静默错误结果」正确性红线级。
- **Blocker 定性正确**：两缺陷位于 JOIN 执行层（expression.rs / ddl_dml.rs / 两执行器），超出 T3 契约的 Preserve/Forbidden 边界与 D3 设计假设，属范围外实质缺陷；S11 在其修复前不可观察。

### Findings

- **Plan 调查遗漏（非交付缺陷）**：D3/R3-S11 设计假设「自连接 Hash/NLJ Join 输出行本身正确」未经调查验证即成立——既有语料 `ON a.id = b.id` 两侧行值恒同掩盖了执行层缺陷，Plan 调查未探测该形态。该遗漏不否定 T1/T2/T3 已交付部分，仅使 S11 不可验收。
- 其余无阻塞 finding；无身份型证据工程；无范围外改动（除格式归一与注释修正，均已核实等价）。
- Act 的验证结论（全量 1875 passed / 0 failed / 3 ignored、fmt/clippy 干净）**采信**——工作区自 Act 运行后未变化（git status 与 Act 文件清单逐一吻合，本审计未修改仓库任何文件），覆盖未失效。

### Acceptance Gaps

- **R3-S11**（自连接同名列按限定符精确解析）——唯一缺口，阻塞于范围外自连接双重缺陷；S1–S10、S12 及 R1/R2 全部场景已见证。

### Blocker 处理与用户决策点

恢复条件三选一（Blocker Handoff 原案），Plan Review 补充评估：

- **(a) replan 并入本 change（Plan 推荐）**：两缺陷为正确性红线（静默错误结果），与本 change「外部评测正确性批」主题同源、代码面相邻（expression.rs / ddl_dml.rs / 两执行器 + 已就位的 `#[ignore]` 见证与种子数据）；需新增 spec delta（JOIN 条件交换判定与输出行归属契约）与 Task Contract，走 replan Cycle。代价：本 change 体量增加一项中-小任务。
- **(b) 独立 change / Issue 处置**：调用 `openspec-experience-recorder` 登记 Issue，S11 验收按缺口显式保留或另行调整；本 change 收敛更快，但红线缺陷滞留主干期间自连接错误结果持续可达。
- **(c) 维持现状**：S11 缺口永久保留——与项目「正确性优先」纪律及既有 spec 承诺冲突，Plan 不推荐。

三案均改变 Acceptance 边界或范围（replan 语义）；选择属授权边界，**Review Result 在用户裁定前保持 `pending`**。

**用户裁定（2026-10-04，原话「同意，给出replan，把isuee也解决」）**：方案 (a)——replan 并入本 change 修复（R9/T9），Issue 随 T9 交付解决，不另立 change、不落账独立 Issue；S11 用例解除 `#[ignore]` 并入 T9 验收。

### Review 结论

- 当前 Cycle 修复意见：无（无需在当前契约内继续修复；S11 缺口超出当前 Task Contract 约束面，由 replan Cycle 承接）。
- Iteration Plan Update：已执行——`tasks.md` 增 R9/T9（task 1.4）、Iteration 000 扩围为 T1/T3/T2/T9（含 replan 追加平衡审计）、RTM 增 R9 行；`proposal.md` 增 Why-9/R9/S31–S37/replan 决策记录；`design.md` 增 D9 与实现顺序更新；`specs/sql-table-alias-resolution` delta 增 ADDED Requirement「自连接侧身份与输出归属正确性」。
- Next Cycle: `iterations/000-initial/001-replan.md`（replan，Status: draft——Gate 2 除「用户批准计划」外全 PASS，待用户审计批准后转 ready 交 Act）。
- Next Iteration: None（000 未完成）。
- Issue 候选 1：自连接双重缺陷——**随 T9 交付解决**（用户裁定，不落账）。
- Issue 候选 2（replan 调查新发现，范围外，只报告不落账）：**三表链式 JOIN 全形态已坏**——单 FROM ≥3 表因子：hash+hash 链与 hash+NLJ 混合链计划期 `Column not found` 失败、仅 SELECT 前两表形态 `join.rs:100` panic exit 101（replan 调查以改动前 release 二进制实测三种形态，HEAD 9538647 与工作区同）；T9 契约将其列为 Preserve 边界（错误面不变）。处置待用户裁定（Recorder 落账或另立 change）。
- 未确认问题：无。

- Review Result: replan-required
