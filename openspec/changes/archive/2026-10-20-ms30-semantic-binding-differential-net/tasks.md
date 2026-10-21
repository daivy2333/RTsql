# Tasks

> Change: 2026-10-20-ms30-semantic-binding-differential-net
> Status: in-progress（2026-10-20：Gate 1 与计划获批；Iteration 000 Act `reported` → Review **`accepted`**——差分网 GREEN 纳入基线，全量 2045/0/2，白名单 6 条目〔1 known + 5 suspected〕；Iteration 001 Cycle 000 Act `reported` → Review **`replan-required`**〔碰撞形状语义对齐 SQLite〕→ replan Cycle `001-replan.md` Act `reported` → Review **`accepted`**——碰撞形状四子查询面外层相关参数化 + 分类/收集判定单源 `is_outer_reference`，全量 2046/0/2；Iteration 002（T02-C2）Cycle `000-initial.md` Act `reported` → Review **`accepted`**（2026-10-21）——JOIN 布局段与 ON 侧判定改自绑定作用域表窗口、`current_qualifiers` 平行记账移除，定向 223/0、全量 2046/0/2；同日用户裁定 003 范围扩充：遗留「投影臂无别名派生外层段登记不完备」并入（T04），与 T03 缺陷清偿同一 Iteration 承载〔原话「把这些问题和iter3本来要做的规划在一起解决掉」〕；Iteration 003 Cycle `000-initial.md` Act `reported`（2026-10-21）——四缺陷清偿 + T04 登记收口完成，白名单 6 → 1（仅剩 `ORD-NULL-ORDERING`），差分 Q7 新增聚合派生查询，定向 218/0、全量 2057/0/2（基线 2046 + 新增 11 个定向函数），`explain_test` 零校准；待 Plan Review）

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T01 | rusqlite 差分与性质测试网：`tests/sql_differential_test.rs`（固定种子数据集 + 差分子集 Q1-Q7 + 等价变换 + 白名单机制 + 归一化比较）+ `tests/structural_property_test.rs`（编码往返 / B-Tree 插删查 / 版本链可见性三组性质，固定种子）——全部 `cargo test` 集合内、无框架外判定层 | `sql-differential-testing` R1-R4 | 000 |
| T02 | 绑定阶段基建与收口：`PlanBuilder` 绑定作用域表（`binding.rs` 模块 + 加性 `ScopeEntry` 登记，FROM/JOIN 装配点同步登记）+ C1 关联子查询参数分类单点化（`qualified_column_expression` 优先级 1 改作用域身份查找，`inner_table_names` 名单降级兜底）+ C2 JOIN 布局与作用域表统一（布局段自作用域表派生、侧别判定改查作用域表） | `sql-semantic-binding` R1-R3 | 001, 002 |
| T03 | 差分白名单缺陷清偿：① 无键基表 JOIN/SemiJoin 空集（`src/executor/scan.rs` 枚举通道对 keyless 表回退数据页走查）/ ② TEXT 相关参数求值 NULL（`src/executor/predicate.rs` 参数树 owned 求值通道）/ ④ NOT IN NULL 三值折叠（`src/executor/anti_join.rs`）/ ③ 聚合派生表外层列引用（`src/parser/planner/query.rs` 派生臂注册列取 SELECT 清单名）；修复后移除对应 5 条 suspected 白名单条目 | `sql-semantics-conformance` R1-R4 | 003 |
| T04 | 投影位外层作用域段登记收口：`declared_qualifiers` 无别名 Derived 臂返回隐式注册名 `derived`（与派生臂 `query.rs` 注册名单源），投影位相关引用对无别名派生外层可达（2026-10-21 用户指令并入） | `sql-semantic-binding` R4 | 003 |

Requirement 权威文本：`specs/sql-differential-testing/spec.md`（4 Requirement ADDED）、`specs/sql-semantic-binding/spec.md`（4 Requirement ADDED，R4 随 003 规划增补）、`specs/sql-semantics-conformance/spec.md`（4 Requirement ADDED，随 003 规划定稿）。

T02 内部分两个 Iteration 承载（C1 → C2 依赖序），全局任务编号不变；每个 Iteration 有独立 Acceptance 与全量 Gate。

## Iteration Plan

### Iteration 000: 差分与性质测试网（先行独立验收）

- Tasks: T01
- Depends on: None
- Stable baseline: 差分套件（差分子集 + 等价变换）与性质套件（编码往返 / B-Tree / 版本链）全 GREEN 并纳入全量基线；白名单机制可用（初始条目按 Act 期分诊结果落定）；后续绑定 Iteration 以「差分网 GREEN」为重构保护基线。
- Verification boundary: `cargo test --test sql_differential_test` 与 `--test structural_property_test` 定向全绿；全量 `cargo test` 一次（Iteration Gate，分批串行——M19 先例），实测记录全量时间增长（R3 预算核对）。无端到端 scenario（新增面全部在 lib API 与纯函数层，经真实入口 `execute_sql`/公共存储 API 直测，符合验证分类）。
- Diagnostic boundary: `tests/sql_differential_test.rs`、`tests/structural_property_test.rs`；失败分诊时对照 `src/pipeline.rs::value_to_json`（归一化映射）、`src/storage/page_format/tuple.rs`（编码）、`src/storage/btree/index_manager.rs`（树操作）、`src/transaction/`（版本链）。
- Non-goals: 绑定层任何改动（T02）；差分发现疑似缺陷的修复（A4 分诊——收口面内归 T02 修复候选、范围外 Issue 候选报告）；fuzzing/探索引擎；rusqlite 升级。

### Iteration 001: 绑定作用域表基建与关联子查询分类收口（T02-C1）

- Tasks: T02（C1 面）
- Depends on: Iteration 000（差分网 GREEN 为保护基线）
- Stable baseline: `PlanBuilder` 携带绑定作用域表（声明限定符段 + 列清单 + 类型/可空信息）；关联参数分类与收集经同一 `is_outer_reference` 谓词判定（分类 live 作用域 / 收集段名快照）；碰撞形状（外层同名未别名 + 内层别名 + 基名引用，四子查询面）解析为外层相关参数并注入求值（2026-10-20 用户裁定对齐 SQLite）；无外层同名基名引用维持 A1 宽松内层解析；差分网（别名/子查询/相关面 + 碰撞新增）+ 既有全部子查询/别名矩阵套件零回归；错误面零校准（投影臂双无效输入错误优先级为已接受例外）。
- Verification boundary: 先行测试 = 差分网相关面（含 Q5/Q6 碰撞新增）+ 既有定向套件（`subquery_alias_binding_test`〔含碰撞回归函数〕 22+ / `subquery_test` 43 / `scalar_subquery_comparison_test` 14 / `table_alias_resolution_test` / `cte_test` / `view_test`）；全量 `cargo test` 一次（Iteration Gate，分批串行）。
- Diagnostic boundary: `src/parser/planner/binding.rs`（新）、`src/parser/planner/mod.rs`（PlanBuilder 字段与登记 API）、`src/parser/planner/expression.rs::qualified_column_expression`、`src/parser/planner/subquery.rs`（分类消费点）、`src/parser/planner/query.rs`（FROM/JOIN 登记点）。
- Non-goals: C2 布局统一（下一 Iteration）；执行器 `condition_column_index` 与其余执行期按名点；`ColumnRef` 载荷与 `get_plan_output_columns`；视图展开/CTE/attach 段的作用域登记完备化（D6 兜底覆盖，完备性留后续批次）。

### Iteration 002: JOIN 组合行布局与作用域表统一（T02-C2）

- Tasks: T02（C2 面）
- Depends on: Iteration 001（作用域表基建）
- Stable baseline: `join_column_layout` 段自作用域表派生（段键空间统一）；`expr_join_side`/`extract_join_conditions` 侧别判定查作用域表；EXPLAIN 计划形状零变化（词表不变锁定）；差分网（JOIN/自连接/链式面）+ 既有 JOIN 套件零回归。
- Verification boundary: 先行测试 = 差分网 JOIN 面 + 既有定向套件（`join_output_resolution_test` / `self_join_test` / `chain_join_test` / `nested_loop_join_test` / `outer_join_test` / `subquery_alias_binding_test` JOIN 形态）；`explain_test` 零校准；全量 `cargo test` 一次（Iteration Gate，分批串行）。
- Diagnostic boundary: `src/parser/planner/query.rs`（布局构造与五个消费点）、`src/parser/planner/ddl_dml.rs::extract_join_conditions/expr_join_side`、`src/parser/planner/mod.rs`（作用域表 API）。
- Non-goals: 执行器接口与 `condition_column_index`；Hash JOIN 条件键 `ColumnRef` 通道；剩余按名解析点收口（I080 后续批次）；plan cache 语义。

### Iteration 003: 差分白名单缺陷清偿与登记收口（T03 + T04）— implemented（Act `reported` 2026-10-21，待 Plan Review）

- Tasks: T03（①②③④ 四缺陷）、T04（投影臂无别名派生段登记收口）
- Depends on: Iteration 002（③ 与 002 同文件 `query.rs`，顺序编辑；002 已 `accepted`；002 契约冻结的白名单 6 条目自本 Iteration 起解冻移除）
- Stable baseline: 差分网白名单 6 → 1（仅剩 `ORD-NULL-ORDERING` 已知语义差异）；①②③④ 各有定向回归 + 差分双引擎一致见证；T04 形态有定向回归；全量零回归
- Verification boundary: 差分网（5 条 defect 条目移除后全绿 + Q7 聚合派生形态新增）+ 各缺陷定向套件 + 全量分批一次（M19）；详细验证契约见 `iterations/003-defect-clearance-and-scope-completion/000-initial.md`
- Diagnostic boundary: `src/executor/scan.rs`（①）、`src/executor/predicate.rs`（②）、`src/executor/anti_join.rs`（④）、`src/parser/planner/query.rs` 派生臂（③）、`src/parser/planner/subquery.rs::declared_qualifiers`（T04）
- Non-goals: ⑤ 谓词×IN 子查询 AND 组合开放（维持拒绝）；差分子集扩容 / fuzzing；`ORD-NULL-ORDERING` 已知语义差异条目；attach 隐式短名命中面扩面（002 Review 期调查定论：五 FROM 面臂已登记、D6 兜底覆盖、无缺陷证据）；视图/CTE 段登记再扩面（同前定论）

### 平衡审计

三个 Iteration 各形成一个内聚、独立验收的阶段成果：000 是可独立交付的测试基建（自身即 MS30-T01 的完整成果，无产品代码依赖）；001 是「分类判定单点化」一个可验证行为结果（差分保护下的重构，改动面 = planner 两文件族）；002 是「布局身份统一」一个可验证行为结果（依赖 001 的基建，改动面 = 布局构造点与侧别判定）。000 与 001/002 是保护网与被保护重构的关系，不可合并验收（差分网必须先 GREEN 才能承担保护职责——MS30 明示「差分网先行独立验收」）；001 与 002 不合并：C1/C2 各自触及独立的行为面与定向套件，合并会把两个可独立诊断的重构面耦在一个 Gate；无单项不成结果的过碎拆分（T01 一个任务承载一个测试文件组，C1/C2 各为一个连贯重构面）。Iteration 003 为缺陷清偿段（四个独立小修复 + 一个登记收口小修，共享「差分网条目清偿 + 遗留关闭」验收主题与同一保护网；003 内五个修复各自独立故障域、独立 RED→GREEN，聚合为一批是 2026-10-20/21 用户裁定与并入指令的结果）。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| sql-differential-testing R1（跨引擎差分 + 白名单） | T01 | `tests/sql_differential_test.rs` 差分子集断言（GREEN-first：测试与套件同批落地，套件本身即目标行为——T01 为新增测试能力，无既有产品行为变更，无 RED 形态；见 Iteration 000 契约 First test 说明） | Covered |
| sql-differential-testing R2（等价变换一致性） | T01 | 同上文件，等价变体互比断言 | Covered |
| sql-differential-testing R3（性质测试确定性可重现） | T01 | `tests/structural_property_test.rs` 三组性质断言 | Covered |
| sql-differential-testing R4（判定位于测试框架内） | T01 | 套件整体（`cargo test` 集合内、无框架外工具——结构性，由 Review 核对） | Covered |
| sql-semantic-binding R1（绑定结果与一期消费面，含碰撞形状场景） | T02 | Iteration 001：既有别名/子查询矩阵先行零回归 + 差分网相关面（重构类：先观察 GREEN 保持）+ replan 碰撞回归函数与差分 Q5/Q6 新增（RED→GREEN） | Covered |
| sql-semantic-binding R2（解析语义与错误面零回归，碰撞形状例外——2026-10-20 裁定） | T02 | Iteration 001/002：全量基线零失败 + 错误断言零校准（例外面由碰撞差分/回归测试锁定） | Covered |
| sql-semantic-binding R3（架构边界：词表/执行器不变） | T02 | Iteration 002：`explain_test` 计划形状零校准 | Covered |
| sql-semantics-conformance R1（无键基表 JOIN 行可见性） | T03 | Iteration 003：差分白名单 Q3-03/Q3-08/Q6-03 条目移除（RED：现状差分失败）+ 定向 keyless JOIN 测试 | Covered（Act `reported` 2026-10-21：定向 1 函数 RED→GREEN；差分三条目移除后双引擎一致） |
| sql-semantics-conformance R2（相关参数文本值保真） | T03 | Iteration 003：差分白名单 Q5-05 条目移除（RED）+ 定向 TEXT 相关参数测试 | Covered（Act `reported` 2026-10-21：定向 2 函数 RED→GREEN；差分 Q5-05 移除后双引擎一致；非参数路径零改动，全量零回归） |
| sql-semantics-conformance R3（NOT IN NULL 三值折叠） | T03 | Iteration 003：差分白名单 Q6-04 条目移除（RED）+ 定向 NOT IN NULL 测试 | Covered（Act `reported` 2026-10-21：定向 3 函数 RED→GREEN；差分 Q6-04 移除后双引擎一致；IN/NOT EXISTS 零回归） |
| sql-semantics-conformance R4（聚合派生表输出列引用） | T03 | Iteration 003：定向聚合派生别名键测试（RED：现状 `Column not found`）+ 差分 Q7 聚合派生形态新增 | Covered（Act `reported` 2026-10-21：定向 3 函数 RED→GREEN；差分 Q7-17 新增双引擎一致；`explain_test` 零校准） |
| sql-semantic-binding R4（投影位外层段登记完备） | T04 | Iteration 003：定向无别名派生外层投影位相关引用测试（RED：现状登记缺口） | Covered（Act `reported` 2026-10-21：定向 2 函数（形态 A 契约形状回归护栏 + 形态 B RED→GREEN）；既有碰撞回归零回归） |

RTM 说明：T01 无 RED 阶段——差分网是新增测试能力（无被修改的产品行为），「First test」即套件自身落地并 GREEN；T02 两个 Iteration 均为重构类任务（行为保持型），先行测试 = 既有基线 GREEN 保持 + 差分网 GREEN（重构 TDD 形态：先观察 GREEN，重构后保持 GREEN）。
