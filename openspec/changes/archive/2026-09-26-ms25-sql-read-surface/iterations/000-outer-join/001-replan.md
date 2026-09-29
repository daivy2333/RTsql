# Iteration 000 / Cycle 001 (replan): OUTER JOIN 三态与 WHERE×JOIN 解禁

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T1（修订版——含 `in-subquery-join-rejection` R1 MODIFIED 与两处校准白名单）
- Acceptance gaps: None（replan Cycle 以修订后契约重新执行，非补缺口）
- Repair items: None
- Inherited scope: `sql-outer-join` R1–R4 全部场景 + `in-subquery-join-rejection` R1 MODIFIED（WHERE+JOIN 文案统一）；design D1/D2（含 replan 子查询交互段）；用户裁定链——Gate 2（2026-09-26）WHERE×JOIN 统一解禁含 INNER → Iteration 000 Act 阻塞（冲突 A/B）→ Plan Review `replan-required` → 用户裁定（2026-09-27）方案 (a)：维持统一解禁、两处见证前提退役校准、文案统一点名错误
- Excluded scope: T2–T7（Iteration 001–003）；等值外连接 Hash 性能路径；`Inner(Using/None)` 现状语义（`MissingOnClause` 保持）；Derived 表作 JOIN 右侧；SMJ；表达式项 × JOIN 解禁；`CrossJoin`/`LeftSemi` 等 `JoinOperator` 其余形态；IN 子查询 JOIN 能力解锁（维持拒绝）

**Objective**

与父 Cycle 一致并扩展：LEFT/RIGHT/FULL OUTER JOIN 计划期接受并端到端可达（匹配组合 + 未匹配 NULL 补齐 + ON 三值语义 + 非等值 ON）；WHERE/ORDER BY/LIMIT/聚合与 JOIN 结果组合可达（INNER/OUTER 统一）；子查询内 `WHERE + JOIN` 拒绝保留且文案统一为 `InSubqueryJoinUnsupported`；两处既有见证按校准白名单改写；`tests/outer_join_test.rs` 矩阵 + 既有套件（白名单外零校准）通过，全量串行零回归。

**Background**

父 Cycle（`000-initial.md`，已冻结）Act 在建立测试见证时命中 Gate 5 阻塞：T1 契约的 Preserve/Forbidden/GREEN/Stop-when 与 Required behavior ② 存在两处实质冲突——(A) `tests/nested_loop_join_test.rs:361` 第 9 用例即 WHERE×JOIN 拒绝见证，与「9 测试零修改」互斥；(B) `tests/subquery_test.rs:789` 与语料 `in-subquery-join-rejection` 场景逐字节锁定子查询内 WHERE+JOIN 的 `Unsupported statement type` 文案，D2 后改报 `InSubqueryJoinUnsupported`。Plan Review 证实两处均为 Plan 遗漏（计划调查未核对测试文件内容与子查询臂序），用户裁定方案 (a) 后本 Cycle 以修订契约重新执行。产品代码零改动（父 Cycle Act 证实），本 Cycle 从测试见证建立起步。

**Investigation Facts**

- Current Baseline: master @ bc1d18d + MS19/MS18 收尾工作区（Rust 代码自 2026-09-26 MS18 Iteration 001 验证后零变化，全量 1285 tests / 0 failures / 2 ignored 串行——父 Cycle Act Response 基线见证采信：`nested_loop_join_test` 9 passed、`subquery_test` 32 passed，本 replan 会话覆盖未变化）。本 change 已入库 delta：`specs/in-subquery-join-rejection/spec.md`（MODIFIED R1，WHERE+JOIN 场景文案统一）与 `specs/sql-outer-join/spec.md` R3 措辞修正/R4 校准白名单——`openspec validate --strict` 通过。
- Current-State Evidence:
  - JOIN 计划臂：`build_from_clause_with_projection`（`src/parser/planner/query.rs:252-457`）。JOIN 类型判定 :318-322——`Inner(On)` → Some；`Inner(_)` → None（:335 `MissingOnClause`）；其余 → `UnsupportedJoinType`（error.rs:88 "Only INNER JOIN is supported"、PG 0A000）。JOIN 右侧恒 `ScanNode`。
  - 路由：纯等值 ON `is_pure_equi_join_on` → Hash `JoinNode`（:405）；否则 NLJ `NestedLoopJoinNode`（:445），谓词经 `join_column_layout`（:435 置位 / :443 restore）在组合行绝对索引编译。
  - 输出列：`all_columns`（:339-366）按 `qualified_columns` 过滤 → `output_columns`（:369-392）。
  - WHERE×JOIN 现状拒绝：`build_query` :844-851 → `UnsupportedStatement("WHERE CLAUSE ON JOIN")`。
  - WHERE 装配序 :840-971（`proj_or_empty` :833-837 sort_due 时空）；排序归属 `is_base_scan_chain`（:1670-1679，Join 为假）与 `sort_projection`（:1220-1224）。
  - NLJ 执行器（`src/executor/nested_loop_join.rs`）：右输入物化一次（:62-70）、左流式；组合行全宽求值（:96-115）；`build_output_row`（:46-57）按 `table_alias` 归属提取——与供行执行器无关（RIGHT 交换基础）。Hash 执行器（join.rs）本 Cycle 零触碰。
  - 子查询拒绝面：`get_subquery_first_column`（`src/parser/planner/subquery.rs:386+`）——Filter 臂 :410 递归下探、Join/NestedLoopJoin 臂 :441 → `PlanError::InSubqueryJoinUnsupported`（error.rs:111 "IN subquery with JOIN is not supported"，PG 0A000 :168）。D2 后子查询 `WHERE + JOIN` 计划构建成功（Filter(Join)），由该臂拒绝——文案统一点。
  - 校准对象（白名单，唯一允许修改的两处既有测试）：① `tests/nested_loop_join_test.rs:361` `where_over_non_equi_join_stays_rejected`（断言 :374 `Unsupported statement type`）→ 改写为 `where_over_non_equi_join_reachable`（同查询断言可达且结果正确）；② `tests/subquery_test.rs:789` `test_in_subquery_where_join_keeps_existing_rejection`（断言 :801-802 `Unsupported statement type`）→ 期望文案改为 `IN subquery with JOIN is not supported`（函数名同步更名 `test_in_subquery_where_join_named_rejection`），doc 注释同步。两文件其余用例（8 + 31）零修改。
  - 测试入口：`tests/nested_loop_join_test.rs`（9）、`tests/subquery_test.rs`（32）、`tests/planner_test.rs`（87）、`tests/cli_test.rs`（111/2i）。集成测试形态 = TempDir + `Database::open` + SQL 断言。
- Code and Critical Path: 与父 Cycle 一致——执行器扩展 `nested_loop_join.rs`（LEFT 匹配旗标 + NULL 补齐；FULL 右匹配位图 + 左耗尽后右补齐）；`plan.rs::NestedLoopJoinNode` 加性 `join_type`；planner `query.rs` JOIN 臂（外连接 + RIGHT 交换 + 全外连接走 NLJ）与 WHERE 臂解禁（D2：WHERE 时 JOIN `output_columns` 全列承载 + 组合布局编译 + Filter/Sort 持 SELECT 投影）；`pipeline.rs` NLJ 臂透传。新增：子查询文案统一无需代码改动（:441 臂既有，D2 后自然接管）。

**Implementation Guidance**

实施顺序（TDD）：① `tests/outer_join_test.rs` 先行——全部用例现状 RED（`UnsupportedJoinType` / `"WHERE CLAUSE ON JOIN"` 拒绝）；② 执行两处白名单校准（改写/更名 + 期望更新，校准后用例在现状仍 RED——D2 未落地时原拒绝面仍在，改写后的可用性见证与文案统一见证均失败，构成 RED 的一部分）；③ `NestedLoopJoinNode` 加 `join_type` + NLJ 执行器 LEFT/FULL 扩展（NULL 补齐按 `output_columns` 归属；FULL 位图在左输入耗尽前不产出）；④ planner JOIN 臂（`LeftOuter/FullOuter(On)` → NLJ、`RightOuter(On)` 交换重写、外连接强制 `On`、外连接绕过 Hash 臂）+ D2 WHERE×JOIN 解禁（WHERE 时 JOIN 全列承载 + 组合布局编译 + SELECT 投影归属，`is_base_scan_chain`/`sort_projection` 覆盖 Filter(Join 全列) 形态）；⑤ 定向套件 + 全量串行。

测试夹具要点：spec R1/R2 种子（LEFT/RIGHT/FULL × 匹配/NULL 键/非等值 ON）；R3 组合面（WHERE 过滤补 NULL 行、裸列名 ORDER BY、IN 子查询 JOIN 拒绝）；WHERE×JOIN 矩阵含 INNER（此前拒绝形态转可用）+ 限定名/非限定名/同名列 AmbiguousColumn/未知列 ColumnNotFound/×ORDER BY 表头=SELECT 清单/×LIMIT/×聚合分组；子查询文案统一见证（`IN (SELECT … JOIN … WHERE …)` → `In subquery with JOIN is not supported`）。

**Task Contracts**

### T1: OUTER JOIN 三态与 WHERE×JOIN 解禁（修订版；`sql-outer-join` R1–R4 + `in-subquery-join-rejection` R1 MODIFIED）

- Requirement/Scenario: R1 LEFT（匹配组合/NULL 键/非等值 ON）、R2 RIGHT/FULL（未匹配补齐）、R3 组合面（WHERE 过滤/ORDER BY/IN 子查询拒绝）、R4 零回归（含校准白名单）；`in-subquery-join-rejection` R1 三场景（单列/多列/WHERE+JOIN 文案统一）
- Depends on: None
- Targets: `src/parser/planner/query.rs::build_from_clause_with_projection`（JOIN 臂扩展 + RIGHT 交换 + WHERE 时全列 output_columns）、`src/parser/planner/query.rs::build_query` WHERE 段（解禁 + 组合布局编译 + SELECT 投影计算）、`src/parser/planner/query.rs::is_base_scan_chain` 及 `sort_projection` 计算面、`src/executor/plan.rs::NestedLoopJoinNode`（加性 `join_type`）、`src/executor/nested_loop_join.rs::NestedLoopJoinExecutor`（LEFT/FULL 扩展）、`src/pipeline.rs` NLJ 臂（透传）、`tests/outer_join_test.rs`（新）、`tests/nested_loop_join_test.rs::where_over_non_equi_join_stays_rejected`（白名单①改写）、`tests/subquery_test.rs::test_in_subquery_where_join_keeps_existing_rejection`（白名单②校准）
- Current behavior: LEFT/RIGHT/FULL 与 NATURAL/CROSS APPLY 等一律 `UnsupportedJoinType`；WHERE 与任何 JOIN 组合 → `UnsupportedStatement("WHERE CLAUSE ON JOIN")`；子查询 `WHERE + JOIN` 报 `Unsupported statement type`（:849 臂先触发）；无 WHERE 的 INNER（Hash/NLJ）如 MS09/MS16 交付
- Required behavior: ① `LEFT/RIGHT/FULL [OUTER] JOIN ... ON <谓词>` 计划期接受，经 NLJ 端到端可达：匹配组合按 INNER 语义；LEFT 左行无匹配 → 左值 + 右列全 NULL；RIGHT 右行无匹配 → 右值 + 左列全 NULL（计划期交换重写，输出列序不变）；FULL 两侧补齐；ON 三值语义 Unknown → 不匹配（补 NULL 分支）；非等值 ON 同通路。② WHERE 与 JOIN（INNER/OUTER 统一）组合：谓词按组合行布局编译（限定名表内位置、非限定名唯一命中/多命中 AmbiguousColumn/缺失 ColumnNotFound），过滤作用于补 NULL 后行流；查询表头与行形状 = SELECT 清单（含 ×ORDER BY：Sort 持组合形状上的 SELECT 投影或等价机制）；×LIMIT/×聚合分组可达。③ 子查询内 `WHERE + JOIN`（含 INNER/OUTER）计划期拒绝保留，文案统一为 `In subquery with JOIN is not supported`（与无 WHERE 的 JOIN 形态一致）。④ 外连接遇 `JoinConstraint::Using/Natural/None` → `UnsupportedJoinType`；`CrossJoin`/`LeftSemi`/`RightSemi`/`LeftAnti`/`RightAnti`/`CrossApply`/`OuterApply` 维持 `UnsupportedJoinType`；`Inner(Using/None)` 维持 `MissingOnClause`；IN 子查询 JOIN 形态（含外连接）维持 `InSubqueryJoinUnsupported`
- Preserve: 无 WHERE 的 INNER 全路径（Hash/NLJ 计划形状、`output_columns` SELECT 过滤语义、结果、表头）逐字节不变——`nested_loop_join_test` 9 用例中 8 个与 `subquery_test` 32 用例中 31 个零修改通过；`JoinExecutor`（join.rs）零触碰；谓词三值内核与 `compare_values` NULL 排序语义零变化；公开 API 签名不变；M17/M18/M19 约束；子查询拒绝语义本身（JOIN 形态不可达）不变
- Forbidden: 不扩展 Hash JoinExecutor 的外连接路径；不解禁 `Inner(Using/None)`；不支持 Derived/子查询作 JOIN 右侧；不改 `extract_join_table_name` 拒绝面；不动排序 NULL 语义；不引入表达式项 × JOIN 解禁；不解锁 IN 子查询 JOIN 能力（拒绝保留）；既有测试修改仅限校准白名单两处（① 改写为可用性见证、② 期望文案/函数名/doc 校准），其余断言零修改；不加阈值/身份型证据机制
- Test witness: `tests/outer_join_test.rs` 全部用例 + 两处校准后用例先于实现运行——现状观察 RED（`UnsupportedJoinType` / `"WHERE CLAUSE ON JOIN"` / 改写后用例因拒绝面仍在而失败），实现后 GREEN；`cargo test --test outer_join_test` 退出码非 0（RED）→ 实现后 0
- GREEN condition: `tests/outer_join_test.rs` 全绿；`nested_loop_join_test` 9（8 原样 + 1 改写）与 `subquery_test` 32（31 原样 + 1 校准）全绿；`planner_test`(87)/`cli_test`(111) 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test outer_join_test --test nested_loop_join_test --test subquery_test`（退出码 0，RED→GREEN 见证）；②`cargo test --test planner_test --test cli_test`（零校准）；③全量 `cargo test -- --test-threads=1`（M19 纪律，退出码 0）；④`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: RIGHT/FULL 扩展需要超出 NLJ 执行器局部改动的框架级重构（用户裁定的 split signal——停止并回报，LEFT 先行收口）；WHERE×JOIN 组合布局解析无法复用 `join_column_layout` 机制（需新解析通道——返回 Plan 裁定）；白名单外既有测试需要修改断言才能通过（说明超出已裁定的校准范围，返回 Plan）

**Invariants**

- INNER 无 WHERE 路径逐字节不变；`output_columns` 的 SELECT 清单过滤语义仅在「WHERE 存在」时让位于全列承载（Filter/Sort 承担收窄）。
- NULL 补齐只发生在 NLJ 执行器产出层（保存侧无匹配），不改写任何存储行或谓词语义。
- 外连接全部经 NLJ——Hash JoinExecutor 代码路径零变化。
- 子查询 JOIN 拒绝语义不变（IN 子查询含 JOIN 计划不可达），D2 仅统一其文案与触发臂。
- 组合布局编译是编译期计划行为（`join_column_layout` save/restore 严格配对），不留运行期状态。

**Non-goals**

- 等值外连接 Hash 性能路径（MS22 域）；`Inner(Using/None)`；Derived 右侧；SMJ；表达式项 × JOIN；IN 子查询 JOIN 能力解锁；T2–T7。

**Acceptance**

`sql-outer-join` R1–R4（含 R4 校准白名单）与 `in-subquery-join-rejection` R1 MODIFIED 全部场景可观察满足（映射见 tasks.md RTM）；白名单外既有 JOIN/子查询面零回归（零校准见证）；全量串行零回归。Scenario→Task→Test 链路：RTM `sql-outer-join` 四行 + `in-subquery-join-rejection` MODIFIED 行。

**Verification**

- S1–S3（LEFT）：`left_join_matched` / `left_join_null_key_no_match` / `left_join_non_equi_on`——测试断言 + cargo 退出码。
- S4–S5（RIGHT/FULL）：`right_join_unmatched_padded` / `full_join_both_sides_padded`。
- S6–S8（组合面）：`where_filters_padded_rows` / `order_by_on_outer_result` / `in_subquery_join_still_rejected`。
- S9–S10（零回归 + 白名单）：`nested_loop_join_test` 8 原样 + `where_over_non_equi_join_reachable`（改写）；`natural_join_rejected` 等拒绝面用例。
- S11（文案统一）：`test_in_subquery_where_join_named_rejection`（校准后）+ 子查询单列/多列 JOIN 拒绝用例零修改。
- WHERE×JOIN 矩阵：INNER/OUTER × 限定名/非限定名/AmbiguousColumn/ColumnNotFound/ORDER BY 表头/LIMIT/聚合。
- 全量：`cargo test -- --test-threads=1` 退出码 0（串行，M19）。

**Gate 2 Readiness**

PASS——修订维度全部闭合：冲突 A/B 的契约矛盾已消除（Preserve/Forbidden/GREEN/Stop-when 与 Required behavior 自洽，校准白名单有用户裁定与 delta spec 承载）；调查补齐（两处测试 file:line、子查询臂序、语料锁定条款全部实测）；任务契约自包含（Act 无需回读父 Cycle——Investigation Facts 含全部实施所需事实）；验证直接判定；无实质未知项。未变化维度（D1 执行器扩展、D2 组合布局机制、拒绝面）引用父 Cycle Gate 2 结论（材料未变）。用户裁定方案 (a)（2026-09-27）即修订计划批准，Status 转 ready。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- 两处校准是「前提退役」型（旧用例锁定的拒绝面正是本能力解除/统一的对象）——改写后用例在实现前 RED（拒绝面仍在）、实现后 GREEN，见证链完整。
- 白名单外若再出现需要校准的既有用例 → 超出已裁定范围，按 Stop-when 返回 Plan（不自行扩白名单）。
- WHERE×JOIN×ORDER BY 的表头形状依赖 Sort 投影归属修正——专用用例锁定表头=SELECT 清单。
- RIGHT 交换重写正确性依赖 `build_output_row` 按 `table_alias` 归属（与供行侧无关）——right_join 用例见证。
- 非实质选择留给 Act：`join_type` 枚举命名、NULL 补齐辅助函数形态、校准用例的具体断言组织。

## Act Response

- Status: reported

### 实施结果

- 实际改动（8 个文件；含 Plan Review 裁定后的当前 Cycle 修复）：
  - `src/executor/plan.rs`——加性 `NestedLoopJoinType { Inner, Left, Full }` 枚举 + `NestedLoopJoinNode.join_type` 字段（`Inner` 为 `Default`，既有形态语义不变）；枚举与 `NestedLoopJoinType::Full` 变体 doc 按 Review 修复后的承载语义校正（RIGHT 由 `Full` 改述为 `Left` 承载）。
  - `src/executor/predicate.rs`——`ConstantPredicate { value: bool }`（`Predicate` + `evaluate_ternary` 两臂），供 `ON <布尔字面量>` 编译。
  - `src/executor/nested_loop_join.rs`——同一循环承载三态：`left_matched`（当前左行匹配旗标）、`right_matched`（Full 右行匹配位）、`drain_index`（Full 补齐游标）；`build_padded_row`（`output_columns` 归属侧取原值、缺席侧填 NULL）、`next_unmatched_right_row`。`build_output_row` 与 Inner 路径逐字未改；模块 doc 的 RIGHT 承载表述按修复校正为 `Left`（未匹配原左行丢弃）。
  - `src/executor/mod.rs`——导出 `NestedLoopJoinType`、`ConstantPredicate`。
  - `src/parser/planner/query.rs`——① JOIN 臂形态分类（`OuterJoinKind`：`Inner(On)` 编译 / `Inner(_)` → `MissingOnClause`；`LeftOuter(On)` → NLJ `Left`；`FullOuter(On)` → NLJ `Full`；`RightOuter(On)` → 交换左右输入并按 NLJ `Left` 承载；其余 → `UnsupportedJoinType`）；② 外连接绕过 Hash 臂；③ `carry_all_columns` 参数（WHERE 存在时 JOIN 节点承载全部组合列）；④ RIGHT 交换后的 ON 布局按「原右表在前」编译 + 布尔字面量 ON 常量编译；⑤ D2 WHERE 臂（`join_combined_columns` 组合形状布局 → `build_where` → `Filter` 承担 SELECT 裁剪，`IN`/`EXISTS` 谓词维持既有拒绝）；⑥ `resolve_join_projection_indices`（组合形状投影索引 + 纯列清单的 `ColumnNotFound` 点名）；⑦ `select_list_is_pure_column_list` 提取为共享 helper；⑧ `is_identity_filter_over_join` + Sort 投影归属三态（聚合 / 基扫描链 / `Filter(JOIN)`）。
  - `src/pipeline.rs`——NLJ 构造臂透传 `join_type`（1 行）。
  - `tests/outer_join_test.rs`（新，16 用例）——R1 三场景 + R2 两场景 + R3 三场景 + R4 拒绝面/计划形状 + WHERE×JOIN 矩阵（INNER/OUTER、限定名/非限定名/AmbiguousColumn/ColumnNotFound、ORDER BY 表头×LIMIT、GROUP BY 聚合、CLI 表头端到端、IN/EXISTS 边界）。
  - `tests/nested_loop_join_test.rs` / `tests/subquery_test.rs`——校准白名单两处 + `correlated_on_injection` 的机械字段初始化（见 Deviations 5）。
- Review 裁定修复（当前 Cycle，Plan Follow-up Decision 四项全部落地）：
  1. `src/parser/planner/query.rs:485-489` 承载路由收窄——`Right` 与 `Left` 同归 `NestedLoopJoinType::Left`，`Full → Full`；左右交换与交换后 ON 布局编译不变（`query.rs:446`/`:490`），`output_columns` 原序不变。
  2. `tests/outer_join_test.rs::right_join_unmatched_padded` 期望 3 行 → 2 行 `[(1,'x'),(NULL,'z')]`，doc 与断言消息改为「未匹配左行丢弃（保留它是 FULL JOIN 的定义）」，模块 doc「RIGHT 经交换重写为 FULL」行同步更正为 LEFT。
  3. `tests/outer_join_test.rs::outer_join_routes_to_nested_loop_join` RIGHT 承载类型期望 `Full → Left`（交换输入的两条断言保留）。
  4. 验证按契约重跑（见验证段）。
- 已完成：T1 全部 Required behavior ①–④、Preserve/Forbidden 全部约束、Verification 全部项；R2-S1 Acceptance gap 关闭。
- 部分完成：None。
- 未开始：None（T2–T7 属 Iteration 001–003，不在本 Cycle）。

### Deviations from Plan

1. **RIGHT 承载形态：按标准 SQL 实现为「交换重写为 LEFT」，与 design D1 一致**（原偏离已由 Plan Review 裁定修复）。首轮实现曾按 delta spec R2-S1 场景误写的 3 行语义实现为 `Full` 承载；Review 裁定该场景为起草错误（3 行恰是 FULL JOIN 定义）并已由 Plan 勘误，Act 按 Task Contract Required behavior ① 与 design D1 修复为 `Left` 承载——未匹配的原左行丢弃。`right_join_unmatched_padded` 见证 2 行语义。
2. **`NestedLoopJoinType` 不设 `Right` 变体**（Plan 括注建议 `Inner/Left/Right/Full`）。RIGHT 经计划期交换后与 `Left` 同构，独立变体会是行为完全相同的死变体；交换在节点的 `left`/`right` 上可见（`outer_join_routes_to_nested_loop_join` 见证左输入 = 原右表）。
3. **新增 `ConstantPredicate`**（Targets 未列）。delta spec R3-S3 的见证查询含 `LEFT JOIN l ON true`，而 `build_where` 无字面量臂（现状 `ON true` → `UnsupportedExpression("PREDICATE")`），无法到达子查询 JOIN 拒绝臂。新增类型是满足该场景的最小手段。**连带面**：布尔字面量 ON 在共享路径上编译，`INNER JOIN … ON true`（叉积）同步变为可达——既有测试无该形态断言。
4. **`IN (SELECT …)` / `EXISTS (SELECT …)` 谓词在 JOIN 基座上维持既有计划期拒绝**（`Unsupported statement type: WHERE CLAUSE ON JOIN`，逐字节不变）。SemiJoin/AntiJoin 的左输入按单表 schema 解析输出列（`build_output_columns_for_table`），与 JOIN 组合形状不同构——若一并解禁会静默产出错误行形状（MS15/MS16「静默错误结果」红线）。该形态不在 Required behavior ② 的矩阵内。新增 `where_subquery_over_join_stays_rejected` 锁定边界；嵌套在复合谓词内的子查询形态经既有 `build_where` 兜底 → `UnsupportedExpression` 点名。
5. **`tests/nested_loop_join_test.rs::correlated_on_injection` 增加字段初始化**（白名单外唯一改动）。Plan 括注称「NLJ 仅 query.rs:445 一处构造」，实测该用例以结构体字面量构造 `NestedLoopJoinNode`——加性字段使其必须显式给出取值。改动为 `join_type: NestedLoopJoinType::Inner` 一行 + 注释，**断言、见证意图与 RED/GREEN 语义均未变**（非实质机械适配）。
6. **D2 布局按表分组为连续段**（D2 原文「布局 = 组合全列」未指定分组粒度）。逐列一项的布局会使限定名解析臂（首个同名表项内查找）漏检同表后续列——`WHERE r.val` 曾误报 `Column not found: val`。按 `table_alias` 连续段分组后偏移与组合形状逐列一致。
7. **纯列清单的未知投影列在 D2 路径点名为 `ColumnNotFound`**（MS19 T5 同款模型）。既有无 WHERE 路径对未知投影列产出零宽行；D2 路径下 Filter 投影无法表达「零列」（空投影 = 恒等），故选择点名拒绝而非静默产出与 SELECT 清单不符的行形状。非纯列清单（聚合/别名/函数项）不校验——其 `qualified_columns` 携带输出名而非基列。
8. **三处 doc 注释随 Review 修复同步校正**（`query.rs` 形态分类注释、`plan.rs` 枚举与 `Full` 变体注释、`nested_loop_join.rs` 模块注释）——原文本描述 RIGHT 由 `Full` 承载，与修复后语义不符。纯注释改动，无行为面。

### 自检发现（Gate 3）

- **已修复（首轮实现期，3 项）**：① Left/Full 补 NULL 行产出后未置位匹配旗标 → 下次 `next()` 在右组合耗尽态重复产出同一行（无限产出，实测进程被 OOM SIGKILL）；② Full 补齐游标复用 `current_right_index`（左输入耗尽时其值已等于右行数 → 补齐分支不可达）；③ D2 布局未按表分组（限定名列解析漏检）。
- **已修复（首轮自检）**：`clippy::needless_borrow` ×2（`&select` 冗余引用）。
- **已修复（Review 裁定轮）**：RIGHT JOIN 语义错误（保留未匹配左行 = FULL 语义）——路由改 `Left` 承载，测试期望与三处 doc 同步校正。修复后审读 `nested_loop_join.rs` 状态机：`right_matched` 位图与 `next_unmatched_right_row` 现仅 `Full` 路径可达（`join_type != Full` 早退），RIGHT 走 `Left` 补 NULL 单臂，无残留 Full 依赖；`build_padded_row` 的归属侧由 `left_table_name` 决定，交换后即原右表，输出列序不变。
- **已回滚（范围外污染）**：`rustfmt src/executor/mod.rs` 递归重排了未触碰的 `src/executor/delete.rs`（MS24 面），已 `git restore --worktree` 还原；格式化只施加于本 Cycle 触及的文件。
- **遗留 Minor 1（无既有测试断言，待 Plan 裁定）**：聚合经 D2 恒等 Filter 时，`AggregateNode.column_indices` 是列名 → 索引的 `HashMap`（末位胜出），两表同名列会使分组键解析到后出现的那一列。仅当同名列值不同且 ON 不等值时产生错误分组；既有聚合与本 Cycle 测试均以不冲突列名见证。修复需让聚合解析具备表限定能力（聚合面设计，超出本 Cycle 与 Iteration 001 的 DISTINCT 范围）。Review 判定为 Issue 候选待用户裁定，登记与否由用户决定。
- **遗留 Minor 2**：`UnsupportedJoinType` 的 Display 文本「Only INNER JOIN is supported」在本能力后对 `NATURAL LEFT/FULL JOIN` 已不精确（该措辞对 `CROSS APPLY` 等 INNER 以外形态变更前即不精确）。按「既有测试/语料零校准 + 措辞属 spec 面裁定」未改文本；`natural_join_rejected` 以注释标注该行为文案敏感行。

### Spec / design 文本发现（Act 只报告；Plan 已处置 1–3）

1. `specs/sql-outer-join/spec.md` R2-S1 场景原期望 3 行（含未匹配左行）——Plan 已勘误为 2 行并加注起草错误说明；实现与之一致。
2. R1-S2 场景 GIVEN 的 `l(id INT)` 无键位声明，其 `(NULL)` 行属无键行——无键行落库但不进索引（MS10-T05 键位语义），而 JOIN 输入走 `ScanNode` 索引遍历，故该行在 JOIN 内不可见。夹具已改用 `k INT PRIMARY KEY` 作行可见性锚（NULL 落在非键位列 `id`）——Plan 已在校准后的 spec 中对齐。
3. R4 场景「其余 JOIN 形态拒绝保持」把 NATURAL 归入 `UnsupportedJoinType`；`INNER NATURAL JOIN` 的既有行为是 `MissingOnClause`。Plan 已按「`natural_join_rejected` 分别锁定两族拒绝面」措辞。
4. design D1 的 RIGHT 段文字——Plan 已加勘误段（D1 语义自始正确，冲突根因为 R2-S1 场景起草错误）；design D2 已补记 IN/EXISTS × JOIN 维持既有拒绝的边界段。

### 验证

- RED 见证（修复轮，仅改测试期望、planner 未改）：`cargo test --test outer_join_test -- --test-threads=1` → `14 passed; 2 failed`（`right_join_unmatched_padded` 实际 3 行 vs 期望 2 行；`outer_join_routes_to_nested_loop_join` `left: Full, right: Left`），退出码非 0。
- GREEN（修复轮定向）：`cargo test --test outer_join_test --test nested_loop_join_test --test subquery_test -- --test-threads=1` → `outer_join_test 16 passed; nested_loop_join_test 9 passed; subquery_test 32 passed; 0 failed`，退出码 0。
- GREEN（修复轮全量，M19 纪律）：`cargo test -- --test-threads=1` → 合计 `1301 passed; 0 failed`（退出码 0；基线 1285 + 本 change 新增 16；`2 ignored` 来自 `cli_test`，与首轮同）。合计值由第二次同命令运行的逐 suite `test result` 求和得到——为取总数而非增强可靠性，结论与首跑一致。
- `cargo clippy --all-targets` → 无 warning（退出码 0，修复轮重跑）。
- `rustfmt --edition 2021 --check`（`plan.rs`/`nested_loop_join.rs`/`query.rs`/`outer_join_test.rs`）→ 0 差异（退出码 0，doc 校正后重跑）。
- `cargo check --all-targets` → 退出码 0（doc 校正后重跑，确认注释改动未破坏编译）。
- `openspec validate 2026-09-26-ms25-sql-read-surface --strict` → `Change '2026-09-26-ms25-sql-read-surface' is valid`（退出码 0，修复轮重跑）。
- 采信（覆盖未变化）：首轮 `planner_test` 87 / `cli_test` 111 passed; 2 ignored（零校准）——本轮仅改 planner 一处 match 臂与注释，planner/cli 计划形状与错误面断言覆盖的代码路径未变，且全量串行已包含并通过。

### Persisted Evidence

None required——全部结论由测试断言与退出码直接判定，可低成本重跑；无一次性环境。

### Experience Candidates

- **Issue 候选（无，未落账）**：D2 使 WHERE×JOIN 可达后，聚合在组合形状上的分组键解析受 `AggregateNode.column_indices` 末位胜出限制（两表同名列 + 非等值 ON → 错误分组）。证据：本 Response「遗留 Minor 1」+ `where_over_join_aggregate` 用例的列名布局。属 MS25 范围外（聚合解析面），由用户决定是否登记 Issue。
- Runbook：None。

### 未解决问题

- 遗留 Minor 1（聚合同名列歧义）为 Issue 候选，待用户裁定是否登记；Minor 2（`UnsupportedJoinType` 文案精度）留待将来错误面打磨。
- Iteration 000 其余 Acceptance 已满足，等待 Plan 重新 Review 裁定 `Review Result`。

## Plan Review

- Review Result: accepted

**Findings**

修复轮独立核查（四项修复逐一对照代码与测试、diff 增量审读）与 Act 声明完全一致：

1. **路由收窄**：`query.rs` 承载匹配现为 `Left | Right → NestedLoopJoinType::Left`、`Full → Full`，注释同步勘误（「RIGHT = 交换左右输入的 LEFT：保存侧（原右表）成为左输入，未匹配的原左行丢弃」）；左右交换（`:490`）与交换后布局编译（`:446`）不变，`output_columns` 原序不变——与 design D1（含勘误段）逐字一致。
2. **测试期望校正**：`right_join_unmatched_padded` 断言 2 行 `[(1,'x'),(NULL,'z')]`、消息「未匹配左行丢弃（保留它是 FULL JOIN 的定义）」；`outer_join_routes_to_nested_loop_join` RIGHT 承载期望 `Left`（交换输入两断言保留）；模块 doc 第 21 行与 R2-S1 doc 更正——三处与勘误后的 delta spec R2-S1（2 行）对齐。
3. **修复轮验证链成立**：RED 见证（仅改测试期望、planner 未改时 `14 passed; 2 failed`，失败正是两处目标用例——期望钉住勘误语义的合法见证）→ GREEN（outer_join 16 + nested_loop_join 9 + subquery_test 32 全绿）→ 全量串行 1301/0/2（= 基线 1285 + 新增 16）→ clippy/fmt/cargo check/validate 全部 0。`planner_test`/`cli_test` 采信理由成立（仅 match 臂与注释变化 + 全量串行已覆盖）。
4. 执行器审读确认无残留：`right_matched`/`drain_index` 仅 Full 路径可达（`join_type != Full` 早退），RIGHT 走 Left 补 NULL 单臂；`build_padded_row` 归属侧经 `left_table_name`（交换后 = 原右表）正确。

非阻塞 finding 沿用上一轮 Review 裁定（Deviation 2–8 全部接受；Minor 1 聚合同名列歧义为 Issue 候选待用户裁定；Minor 2 文案精度留档）。Deviation 8（三处 doc 注释同步校正）为修复轮的正确连带，接受。

**Acceptance Gaps**

无。R1 三场景（匹配/NULL 键/非等值 ON）、R2（RIGHT 2 行语义 + FULL 两侧补齐）、R3 组合面（WHERE 过滤补 NULL 行/ORDER BY/IN 子查询拒绝）、R4 零回归（白名单外零校准）+ `in-subquery-join-rejection` R1 MODIFIED（文案统一）全部由 RED→GREEN 或零校准见证；全量串行 1301/0/2 零回归。

**Evidence**

- 修复轮四项逐一对照：`src/parser/planner/query.rs:482-489`（承载匹配 + 勘误注释）、`tests/outer_join_test.rs:161-189`（R2-S1 2 行）、`:332-380`（计划形状 Right→Left + 交换断言）、`:21`（模块 doc）。
- `git diff --stat` 增量（597/113，较首轮 +13/-7 = 路由臂 + doc 校正 + 测试期望，无计划外面）。
- Act Response 修复轮验证 1–7（本会话新鲜运行，采信）；首轮已核实面（D2 矩阵/白名单/执行器状态机）覆盖未变化，沿用上轮 Review 结论。

**Follow-up Decision**

接受（accepted）：Acceptance 全部满足，无阻塞 finding。Minor 1 为 Issue 候选（聚合同名列歧义，MS25 范围外），登记与否由用户决定，本 Review 不落账；Minor 2 留档。Iteration 000 完成——展开 Iteration 001（`001-row-algebra`）。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/001-row-algebra/000-initial.md`（已按 Iteration Map 展开，Status: ready——change 级计划 D1–D8 与 T2/T3/T4 契约已于 2026-09-26 Gate 2 获用户批准）。
