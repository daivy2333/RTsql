# Iteration 000 / Cycle 000: OUTER JOIN 三态与 WHERE×JOIN 解禁

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-outer-join` R1–R4 全部场景；design D1（NLJ 统一承载 + RIGHT 交换）与 D2（WHERE×JOIN 组合布局编译）；proposal 范围与决策记录（用户批准 2026-09-26）；WHERE+INNER JOIN 随 D2 统一解禁（Gate 2 用户裁定 2026-09-26，与 OUTER 同路径无分叉）
- Excluded scope: T2–T7（Iteration 001–003）；等值外连接 Hash 性能路径；`Inner(Using/None)` 现状语义（`MissingOnClause` 保持）；Derived 表作 JOIN 右侧；SMJ；表达式项 × JOIN 解禁；`CrossJoin`/`LeftSemi` 等 `JoinOperator` 其余形态

**Objective**

LEFT/RIGHT/FULL OUTER JOIN 计划期接受并端到端可达（匹配行组合 + 未匹配行 NULL 补齐 + ON 三值语义 + 非等值 ON），WHERE/ORDER BY/LIMIT/聚合与外连接结果组合可达，WHERE+JOIN（INNER 与 OUTER 统一）按组合布局编译可达；`tests/outer_join_test.rs` 矩阵 + 既有 JOIN 套件零校准通过，全量串行零回归。

**Investigation Facts**

- Current Baseline: master @ bc1d18d + MS19/MS18 收尾工作区（Rust 代码自 2026-09-26 MS18 Iteration 001 验证后零变化，全量 1285 tests / 0 failures / 2 ignored 串行——来源 MS18 Iteration 001 Act Response，accepted Review 采信；收尾归档为纯文档操作不改代码）。one-shot 定向基线：`cargo test --test nested_loop_join_test`（9 tests，MS09 落地后未改）。
- Current-State Evidence:
  - JOIN 计划臂：`build_from_clause_with_projection`（`src/parser/planner/query.rs:252-457`）。JOIN 类型判定 :318-322——`JoinOperator::Inner(JoinConstraint::On(expr))` → Some；`Inner(_)` → None（:335 `MissingOnClause` 拒绝）；其余 → `PlanError::UnsupportedJoinType`（`src/parser/error.rs:88` 文案 "Only INNER JOIN is supported"、:165 PG 0A000）。JOIN 右侧恒 `ScanNode`（`extract_join_table_name`，ast.rs:264，Derived 拒绝）。
  - 路由：纯等值 ON `is_pure_equi_join_on` → Hash `JoinNode`（:405）；否则 NLJ `NestedLoopJoinNode`（:445）——NLJ 谓词经 `self.join_column_layout = Some(layout)`（:435，layout = 全部 current_tables + right 的全列，绝对偏移）+ `build_where` 编译（:436）+ restore（:443）。
  - 输出列：`all_columns`（:339-366，全部组合列 OutputColumn{table, column, table_alias, column_index}）按 `qualified_columns`（SELECT 清单）过滤 → `output_columns`（:369-392）。
  - WHERE×JOIN 现状拒绝：`build_query` :844-851 `PhysicalPlan::Join(_)|NestedLoopJoin(_)` + `select.selection` → `UnsupportedStatement("WHERE CLAUSE ON JOIN")`。
  - WHERE 装配序：:840-971——非 JOIN 单表路径（PK 索引/OR Filter/下推 DataScan）；`proj_or_empty`（:833-837，sort_due 时空）；Filter 持 `projection` 裁剪（既有机制）。
  - 排序归属：`is_base_scan_chain`（:1670-1679）——Scan/DataScan/IndexScan/IndexScanAll 及 Filter(递归) 为真，Join 为假；`sort_projection`（:1220-1224）非扫描链 → 空（Sort 不裁剪）。`SortExecutor::compare_rows`（`src/executor/sort.rs:74-97`）列名 position 查找 + NULL 排尾（与 SQLite 相反，既有语义）。
  - NLJ 执行器（`src/executor/nested_loop_join.rs`）：右输入物化一次（:62-70）、左输入流式；组合行 `left_row ++ right_row` 全宽求值谓词（:96-115）；`build_output_row`（:46-57）按 `output_columns` 的 `table_alias` 归属从左/右侧提取——**与哪个执行器供行无关**（RIGHT 交换重写基础）；谓词 Unknown → 不匹配（同 Filter 内核）。
  - Hash 执行器（`src/executor/join.rs`）：`build_hash_key_*` NULL 键 → None 不匹配（:66-71）；本 Cycle 零触碰。
  - IN 子查询 JOIN 拒绝：`get_subquery_first_column`（`src/parser/planner/subquery.rs:386+`）对 `Join`/`NestedLoopJoin` 显式臂（MS17-T02）→ `InSubqueryJoinUnsupported`——外连接计划形状落入同臂。
  - 谓词编译上下文：`expression.rs:248-338` `resolve_column_ref` 两臂消费 `join_column_layout`——非限定名全表搜索（0→ColumnNotFound / >1→AmbiguousColumn / 1→偏移+位置）、限定名表偏移+表内位置。WHERE×JOIN 复用同一机制（D2），`expression.rs` 预计零修改。
  - 测试入口：`tests/nested_loop_join_test.rs`（9，plan 形状/语义/零回归矩阵）；`tests/planner_test.rs`（87）；`tests/cli_test.rs`（111/2i，含 JOIN 错误面）。集成测试形态 = TempDir + `Database::open` + SQL 断言（R24 runbook 同款）。
- Code and Critical Path: 执行器扩展 `nested_loop_join.rs`（LEFT 匹配旗标 + NULL 补齐产出；FULL 右匹配位图 + 左耗尽后右补齐）；plan 节点 `plan.rs::NestedLoopJoinNode` 加性 `join_type` 字段（`Inner`/`Left`/`Right`/`Full`，`#[derive]` 默认不影响既有构造点——NLJ 仅 query.rs:445 一处构造）；planner `query.rs` JOIN 臂扩展（LeftOuter/RightOuter/FullOuter + RIGHT 左右交换重写 + 外连接强制 `On` 约束 + 全外连接走 NLJ）与 WHERE 臂解禁（D2：WHERE 存在时 JOIN `output_columns` 承载全部组合列、Filter 以组合布局编译并持 SELECT 投影）。`pipeline.rs` NLJ 构造臂仅透传新字段（:721-742）。`get_plan_output_columns` Join/NLJ 臂按 `output_columns` 直出（:88-103）——全列形态自动正确，Filter 臂（:51-63）投影裁剪元数据既有。

**Implementation Guidance**

实施顺序（TDD）：① `tests/outer_join_test.rs` 先行——全部用例在现状 `UnsupportedJoinType` / `"WHERE CLAUSE ON JOIN"` 拒绝下 RED；② `NestedLoopJoinNode` 加 `join_type` 字段 + 执行器 LEFT/FULL 扩展（LEFT：当前左行右组合耗尽且本轮无匹配 → 产出左行 + 保存侧列全 NULL——NULL 填充按 `output_columns` 归属（非保存侧每个 `column_index` 位填 `Value::Null`）；FULL：右行匹配位图，左输入耗尽后按右行序产出未匹配右行 + 左侧 NULL；谓词求值错误传播路径不变）；③ planner JOIN 臂：`LeftOuter/FullOuter(On)` → NLJ（`join_type` 对应），`RightOuter(On)` → 交换重写（left=右计划、right=左计划、`join_type: Left`、布局与谓词按交换后组合行编译、`output_columns` 保持原序）——全部外连接绕过 Hash 臂直入 NLJ；外连接遇 `Using/Natural/None` → `UnsupportedJoinType`；④ D2 WHERE×JOIN：`select.selection` 且基座为 JOIN 时不再拒绝——JOIN 节点 `output_columns` 用 `all_columns`（跳过 qualified 过滤），WHERE 谓词经 `join_column_layout`（组合全列布局）编译，包 `FilterNode{projection}`——投影 = SELECT 清单在组合形状上的索引（`sort_due` 时空、由 Sort 持有；`sort_projection` 计算需覆盖 Filter(Join-with-full-columns) 形状：SELECT 索引对组合 schema 计算）；⑤ 定向套件 + 全量串行。

测试夹具要点：LEFT/RIGHT/FULL 各 3 场景（匹配/NULL 键/非等值 ON）按 spec R1/R2 种子；组合面按 R3（WHERE 过滤补 NULL 行、裸列名 ORDER BY、IN 子查询 JOIN 拒绝）；WHERE×JOIN 矩阵含 INNER（此前拒绝形态转可用）+ 限定名/非限定名/双表同名列 AmbiguousColumn/未知列 ColumnNotFound/×ORDER BY 表头=SELECT 清单/×聚合分组。WHERE+INNER+ON true 等此前拒绝形态的行为扩展由新用例见证，既有用例零修改。

**Task Contracts**

### T1: OUTER JOIN 三态与 WHERE×JOIN 解禁（`sql-outer-join` R1–R4）

- Requirement/Scenario: R1 LEFT（匹配组合/NULL 键/非等值 ON）、R2 RIGHT/FULL（未匹配补齐）、R3 组合面（WHERE 过滤/ORDER BY/IN 子查询拒绝）、R4 零回归（INNER 零变化/其余形态拒绝保持）
- Depends on: None
- Targets: `src/parser/planner/query.rs::build_from_clause_with_projection`（JOIN 臂扩展 + RIGHT 交换 + WHERE 时全列 output_columns）、`src/parser/planner/query.rs::build_query` WHERE 段（解禁 + 组合布局编译 + SELECT 投影计算）、`src/parser/planner/query.rs::is_base_scan_chain` 及 `sort_projection` 计算面（Filter(Join 全列) 形态的 Sort 投影归属）、`src/executor/plan.rs::NestedLoopJoinNode`（加性 `join_type`）、`src/executor/nested_loop_join.rs::NestedLoopJoinExecutor`（LEFT/FULL 扩展）、`src/pipeline.rs::create_executor_from_plan` NLJ 臂（透传）、`tests/outer_join_test.rs`（新）
- Current behavior: LEFT/RIGHT/FULL 与 NATURAL/CROSS APPLY 等一律计划期 `UnsupportedJoinType`（"Only INNER JOIN is supported"，exit 3 / PG 0A000）；WHERE 与任何 JOIN 组合 → `UnsupportedStatement("WHERE CLAUSE ON JOIN")`；无 WHERE 的 INNER（Hash/NLJ）行为与计划形状如 MS09/MS16 交付
- Required behavior: ① `LEFT/RIGHT/FULL [OUTER] JOIN ... ON <谓词>` 计划期接受，经 NLJ 端到端可达：匹配组合按 INNER 语义；LEFT 左行无匹配 → 左值 + 右列全 NULL；RIGHT 右行无匹配 → 右值 + 左列全 NULL（计划期交换重写，输出列序不变）；FULL 两侧补齐；ON 三值语义 Unknown → 不匹配（补 NULL 分支）；非等值 ON 同通路。② WHERE 与 JOIN（INNER/OUTER 统一）组合：谓词按组合行布局编译（限定名表内位置、非限定名唯一命中/多命中 AmbiguousColumn/缺失 ColumnNotFound），过滤作用于补 NULL 后行流；查询表头与行形状 = SELECT 清单（含 ×ORDER BY：Sort 持组合形状上的 SELECT 投影或等价机制，D10 精神）；×LIMIT/×聚合分组可达。③ 外连接遇 `JoinConstraint::Using/Natural/None` → `UnsupportedJoinType` 点名；`CrossJoin`/`LeftSemi`/`RightSemi`/`LeftAnti`/`RightAnti`/`CrossApply`/`OuterApply` 维持 `UnsupportedJoinType`；`Inner(Using/None)` 维持 `MissingOnClause`；IN 子查询 JOIN 形态维持 `InSubqueryJoinUnsupported`（含外连接）
- Preserve: 无 WHERE 的 INNER 全路径（Hash/NLJ 计划形状、`output_columns` SELECT 过滤语义、结果、表头）逐字节不变——`tests/nested_loop_join_test.rs` 9 测试与既有 JOIN 用例零修改通过；`JoinExecutor`（join.rs）零触碰；谓词三值内核与 `compare_values` NULL 排序语义零变化；公开 API 签名不变（`NestedLoopJoinNode` 新字段由 planner 单一构造点填充）；M17/M18/M19 约束
- Forbidden: 不扩展 Hash JoinExecutor 的外连接路径；不解禁 `Inner(Using/None)`；不支持 Derived/子查询作 JOIN 右侧；不改 `extract_join_table_name` 拒绝面；不动排序 NULL 语义；不引入表达式项 × JOIN 解禁；不修改既有测试断言（外连接、WHERE×JOIN 均为新用例）；不加阈值/身份型证据机制
- Test witness: `tests/outer_join_test.rs` 全部用例先于实现运行——现状观察 RED（`UnsupportedJoinType` / `"WHERE CLAUSE ON JOIN"` 拒绝），实现后 GREEN；`cargo test --test outer_join_test` 退出码非 0（RED）→ 实现后 0
- GREEN condition: `tests/outer_join_test.rs` 全绿（三态矩阵 + WHERE×JOIN 矩阵 + 拒绝面）；`nested_loop_join_test`(9)/`planner_test`(87)/`cli_test`(111) 既有用例零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test outer_join_test --test nested_loop_join_test`（退出码 0，RED→GREEN 见证）；②`cargo test --test planner_test --test cli_test`（零校准）；③全量 `cargo test -- --test-threads=1`（M19 纪律，退出码 0）；④`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: RIGHT/FULL 扩展需要超出 NLJ 执行器局部改动的框架级重构（用户裁定的 split signal——停止并回报，LEFT 先行收口）；WHERE×JOIN 组合布局解析无法复用 `join_column_layout` 机制（需新解析通道——返回 Plan 裁定）；既有 JOIN 测试需要修改断言才能通过（说明语义变化，返回 Plan）

**Invariants**

- INNER 无 WHERE 路径逐字节不变；`output_columns` 的 SELECT 清单过滤语义仅在「WHERE 存在」时让位于全列承载（Filter/Sort 承担收窄）。
- NULL 补齐只发生在 NLJ 执行器产出层（保存侧无匹配），不改写任何存储行或谓词语义。
- 外连接全部经 NLJ——Hash JoinExecutor 代码路径零变化。
- 组合布局编译是编译期计划行为（`join_column_layout` save/restore 严格配对），不留运行期状态。

**Non-goals**

- 等值外连接 Hash 性能路径（MS22 域）；`Inner(Using/None)`；Derived 右侧；SMJ；表达式项 × JOIN；T2–T7。

**Acceptance**

`sql-outer-join` R1–R4 全部场景可观察满足（映射见 tasks.md RTM 与本契约 GREEN condition）；既有 JOIN 面零回归（零校准见证）；全量串行零回归。Scenario→Task→Test 链路：RTM `sql-outer-join` 四行。

**Verification**

- S1–S3（LEFT 三场景）：`left_join_matched` / `left_join_null_key_no_match` / `left_join_non_equi_on`——测试自身断言 + cargo 退出码。
- S4–S5（RIGHT/FULL）：`right_join_unmatched_padded` / `full_join_both_sides_padded`。
- S6–S8（组合面）：`where_filters_padded_rows` / `order_by_on_outer_result` / `in_subquery_join_still_rejected`。
- S9–S10（零回归）：`nested_loop_join_test` 9 测试零修改 + `natural_join_rejected` 等拒绝面用例。
- WHERE×JOIN 矩阵：INNER/OUTER × 限定名/非限定名/AmbiguousColumn/ColumnNotFound/ORDER BY 表头/LIMIT/聚合（用例集见 Implementation Guidance ⑤）。
- 全量：`cargo test -- --test-threads=1` 退出码 0（串行，M19）。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 全 Covered）；调查完整（JOIN 臂/执行器/布局机制/拒绝面/测试入口全部 file:line 实证）；设计闭合（D1 交换重写、D2 组合布局、NULL 补齐语义、RIGHT/FULL 拆出信号均明确）；任务契约自包含（Act 无需回读外部资料）；验证直接判定（测试断言 + 退出码，无身份型证据工程）；无实质未知项。范围确认已裁定（用户 2026-09-26）：WHERE+INNER JOIN 随 D2 统一解禁（与 OUTER 同路径，无类型分叉）；计划整体获批，Status 转 ready。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果（测试断言自证 + cargo 退出码 + ≤20 行决定性输出）；无一次性环境、无不可复现结论。

**Risks and Notes**

- WHERE+INNER JOIN 从「报错」变「可用」是行为面扩大（非既有可用行为回归）——Gate 2 用户确认项；D2 统一路径下排除 INNER 需要人为分叉（额外拒绝臂）。
- RIGHT 交换重写后，`build_output_row` 按 `table_alias` 归属提取与输入侧无关（nested_loop_join.rs:46-57 实测基础）——交换正确性的关键依赖，实施时以 right_join 用例见证。
- FULL 的右匹配位图在左输入耗尽前不得产出——位图填充只发生在谓词求值分支，内存上界 = 右行数 × 1 bit（右输入已整物化，无新增量级）。
- WHERE×JOIN×ORDER BY 的表头形状依赖 Sort 投影归属修正（`is_base_scan_chain` 不含 Filter(Join)）——实施时以专用用例锁定表头=SELECT 清单。
- 非实质选择留给 Act：`join_type` 枚举命名与字段位置、NULL 补齐的辅助函数形态、`sort_projection` 修正的具体计算位置。

## Act Response

- Status: blocked

### 实施结果

- 实际改动：**零产品代码改动**（Plan Context 不可改写，未触碰）。
- 已完成：基线见证建立——既有 JOIN 套件现状 GREEN 确认（见验证段）。
- 部分完成：None。
- 未开始：T1 全部实施步骤（`NestedLoopJoinNode.join_type`、NLJ 执行器 LEFT/FULL 扩展、planner JOIN 臂 + RIGHT 交换重写、D2 WHERE×JOIN 解禁），理由见 Blocker Handoff。

### Blocker Handoff

- 命中 Gate：Gate 5——见证不能证明目标且必须改变测试策略；实际代码与契约存在实质冲突。
- 发现位置：T1，Phase 3「按 Task Contract 建立测试见证」步，写 `tests/outer_join_test.rs` 之前。
- 偏差分类：**Plan 遗漏 + Plan 错误**（非 Act 偏离、基线未变）。契约的 Required behavior ② 与自身 Preserve / Forbidden / GREEN condition / Stop-when 四处互斥；D2 同时推翻两处已验收语料。契约 Stop-when 条件在计划期即确定成立——无需实施即可判定，故不盲试。

#### 冲突 A：D2 统一解禁推翻契约点名的零校准见证

| 契约位置 | 内容 |
|---|---|
| `:51` Required behavior ② | 「WHERE 与 JOIN（INNER/OUTER 统一）组合」谓词按组合布局编译、可用 |
| `:52` Preserve | 「`tests/nested_loop_join_test.rs` 9 测试与既有 JOIN 用例**零修改**通过」 |
| `:53` Forbidden | 「**不修改既有测试断言**」 |
| `:55` GREEN condition | 「`nested_loop_join_test`(9) 零校准通过」 |
| `:57` Stop-when | 「既有 JOIN 测试需要修改断言才能通过 → **返回 Plan**」 |

- 实际情况：`tests/nested_loop_join_test.rs:361` `where_over_non_equi_join_stays_rejected`（doc `:355` 标为「T13 可选见证」）断言 `SELECT r.a, s.b FROM r JOIN s ON r.a < s.b WHERE r.a > 0` 返回 `Unsupported statement type`。D2 落地后该查询可用 → 用例必然失败。
- 根因：契约把「9 测试零修改」当作该文件的性质断言，但第 9 个用例正是 WHERE×JOIN 拒绝见证——Plan 调查未核对该文件内容。Preserve 的真实不变量（`:52` 前半句「无 WHERE 的 INNER 全路径逐字节不变」）与该用例无关。
- 影响：T1 GREEN condition 不可满足；达成 Required behavior ② 必须违反 Forbidden。

#### 冲突 B：子查询内 WHERE×JOIN 未被契约覆盖，且与已验收语料逐字节冲突

- Plan 缺口：D2 与 Current-State Evidence 均未讨论子查询上下文。
- 实际路径：`src/parser/planner/subquery.rs:410` `Filter` 臂递归下探 → `:441` `Join | NestedLoopJoin` 臂 → `PlanError::InSubqueryJoinUnsupported`（`src/parser/error.rs:111` 文案 `IN subquery with JOIN is not supported`，**不含** `Unsupported statement type`）。
- 实际情况：`tests/subquery_test.rs:789` `test_in_subquery_where_join_keeps_existing_rejection` 断言子查询内 `WHERE + JOIN` 报 `Unsupported statement type`。今日命中 `src/parser/planner/query.rs:849` 的 `WHERE CLAUSE ON JOIN`；D2 落地后子查询计划构建成功（`Filter(Join)`），改由 `:441` 臂拒绝 → 文案变为 `IN subquery with JOIN is not supported` → 用例失败。该用例在契约的零校准清单外，Plan 未预见。
- 语料冲突：`openspec/specs/in-subquery-join-rejection/spec.md:25-29` 场景「WHERE + JOIN 维持既有拒绝」要求错误信息**逐字节不变**；`openspec/specs/join-executor-selection/spec.md:62` R5 要求「既有全量测试 SHALL 零修改通过」。本 change 的 delta spec 集合（`specs/` 下 9 份）**不含**这两个 capability。

#### Act 未自行处置的理由

改上述两处既有断言 → 违反 Forbidden 明文；修订 `in-subquery-join-rejection` / `join-executor-selection` 语料 → 属 Plan/Maintainer 职责；把 D2 收窄为 OUTER-only 或改语义基准 → 属范围/需求裁定。任一处置都构成「自行补全 Plan 遗漏的设计或扩大变更面」（Act 禁止项），并命中 AGENTS.md「影响错误语义、测试策略或 Acceptance 的问题属于实质问题」。

#### 恢复条件（交 `openspec-plan`）

1. 裁定 D2 与两处既有见证的关系：**(a)** 明确修订两处断言 + 追加 `in-subquery-join-rejection` / `join-executor-selection` 的 delta spec（逐字节不变条款随之修订），并同步改写 Preserve / Forbidden / GREEN condition；或 **(b)** 收窄 D2（仅 OUTER 解禁，或推迟到独立 Cycle），保留既有见证零校准。
2. 若选 (a)：明确子查询内 WHERE×JOIN 的目标错误面（维持 `InSubqueryJoinUnsupported` 新文案或另裁），并纳入 `sql-outer-join` R3 或 `in-subquery-join-rejection` 修订。
3. 修订后 Cycle 计划须自洽可执行；Act 按「恢复阻塞」流程（`blocked → pending`）恢复，对 D2 段重新建立测试见证，未变化部分采信本 Response 的基线结论。

### 验证

- `cargo test --test nested_loop_join_test` → `9 passed; 0 failed; 0 ignored`，退出码 0（基线见证，含冲突 A 用例）。
- `cargo test --test subquery_test` → `32 passed; 0 failed; 0 ignored`，退出码 0。
- `cargo test --test subquery_test test_in_subquery_where_join_keeps_existing_rejection -- --exact` → `1 passed; 31 filtered out`，退出码 0（冲突 B 用例点名见证）。
- `openspec validate 2026-09-26-ms25-sql-read-surface --strict` → `Change '2026-09-26-ms25-sql-read-surface' is valid`，退出码 0（本会话早前运行，覆盖范围未变化，采信）。

### Persisted Evidence

None required——冲突由计划文本、已验收语料正文与源码 file:line 机械可判，不依赖一次性现场；基线结论可低成本重跑。

### Experience Candidates

None。

### 工作区状态

- 产品代码与测试代码：零改动，`git status` 与接手时一致。
- 本文件：仅追加本 Act Response。

## Plan Review

- Review Result: replan-required

**Findings**

独立核查（测试文件逐行阅读 + 源码 file:line + 语料正文）证实 Blocker Handoff 两处冲突全部成立，偏差分类正确：

- **冲突 A 证实（Plan 遗漏）**：`tests/nested_loop_join_test.rs:361` `where_over_non_equi_join_stays_rejected`（doc 自标「T13 可选见证」）断言非等值 JOIN + WHERE 返回 `Unsupported statement type`。D2 落地后该查询按 Required behavior ② 可达 → 用例必然失败。Plan 计划期声称「9 测试零修改」但未读取该文件内容——Preserve 的真实不变量（无 WHERE 的 INNER 全路径）与该用例无关，契约把文件级性质断言误当逐用例事实。
- **冲突 B 证实（Plan 遗漏 + 语料冲突）**：`tests/subquery_test.rs:789` 断言子查询内 `WHERE + JOIN` 报 `Unsupported statement type` 逐字节；语料 `in-subquery-join-rejection` 场景明文锁定同文案。D2 后子查询计划构建成功（`get_subquery_first_column` Filter 臂 subquery.rs:410 递归 → Join 臂 :441）→ 文案变为 `InSubqueryJoinUnsupported`。拒绝语义保留，仅文案统一——与 MS17-T02「点名拒绝、不误报」本意一致；逐字节锁定是 D2 前臂序（query.rs:849 先于 subquery.rs:441 触发）的偶然产物。
- **爆炸半径核查**：全测试套件 grep 证实受影响面恰为上述两处（`cli_test.rs:3980` 为 MERGE 拒绝见证，无关；其余 `Unsupported statement type` 匹配均为无关表面）。
- Act 处置正确：零产品代码改动、未改测试（Forbidden 明文）、未改语料（Plan/Maintainer 职责）、基线见证建立后阻塞上报。基线验证（nested_loop_join_test 9 passed / subquery_test 32 passed）本会话新鲜运行，采信。

**Acceptance Gaps**

无实施性缺口（零代码改动）——阻塞在计划文本与现实的冲突：T1 契约的 Preserve/Forbidden/GREEN condition/Stop-when 与 Required behavior ② 互斥（冲突 A），且本 change delta 集缺少 `in-subquery-join-rejection` 修订（冲突 B）。Acceptance 本身（`sql-outer-join` R1–R4）不变。

**Evidence**

- `tests/nested_loop_join_test.rs:355-378`、`tests/subquery_test.rs:786-803` 逐行阅读。
- `src/parser/planner/subquery.rs:410/441`、`src/parser/error.rs:111`、`src/parser/planner/query.rs:849`。
- `openspec/specs/in-subquery-join-rejection/spec.md`（WHERE+JOIN 场景逐字节锁定）、`openspec/specs/join-executor-selection/spec.md:62` R5。
- Act Response 验证段（基线见证，采信）。

**Follow-up Decision**

`replan-required`：修复需修订 T1 契约（Preserve/Forbidden/GREEN/Stop-when）+ 新增 `in-subquery-join-rejection` MODIFIED delta（requirement 面），超出当前 Cycle 有限修复边界；本 Cycle 含 Blocker Handoff，创建后继后冻结。用户裁定（2026-09-27）方案 (a)：维持 D2 统一解禁，两处见证按前提退役校准，文案统一为点名错误。replan 产物：change 侧 `in-subquery-join-rejection` MODIFIED delta + `sql-outer-join` R3 场景措辞修正/R4 校准白名单 + design D2 子查询交互段 + proposal 决策记录 5 + tasks T1/RTM 更新；Cycle 侧 `001-replan.md` 修订契约。

**Iteration Plan Update**

Iteration Plan（000–003 四 Iteration）不变；仅 Iteration 000 的执行契约与 change delta 集修订（`in-subquery-join-rejection` MODIFIED 并入 T1）。

**Next Cycle**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/000-outer-join/001-replan.md`

**Next Iteration**

None（Iteration 000 未完成，后继 Cycle 同目录）。
