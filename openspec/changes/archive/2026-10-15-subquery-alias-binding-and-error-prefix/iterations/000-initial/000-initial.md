# Iteration 000 / Cycle 000: 子查询别名绑定修复与错误前缀单层化

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3, T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1（sql-table-alias-resolution ADDED）、R2（correlated-subquery-cache ADDED）、R3（error-reporting MODIFIED）；ISS16/ISS17/ISS18/ISS19 台账处置；proposal 默认假设 A1–A3
- Excluded scope: proposal Non-goals 全项（多级相关维持拒绝、JOIN 形 IN 维持拒绝、遮蔽语义调整、ast.rs 其他别名点、文档变更）

**Objective**

六个先行测试形状 GREEN（EXISTS×别名×非首列/首列/相关非 PK、IN×别名×索引、WHERE 相关标量、投影位相关标量 + 无别名零回归守卫）；三执行器绑参未命中点名报错；CLI 执行错误文案单层；e2e 真实 CLI 见证三 issue 形状；全量 `cargo test` 零回归。

**Investigation Facts**

- Current Baseline: 源码 HEAD `23495df`（2026-10-15 工作树），rtsql 0.2.0。全量基线 1999 tests / 0 failures / 2 ignored（采信 change `2026-10-14-iss15-...` 收尾记录，HEAD 未变）。缺陷现场：四 issue 最小复现已在全新库逐字验证（台账 ISS16–ISS19 证据节）。
- Current-State Evidence（根因链，代码级 Confirmed）：
  1. `src/parser/planner/subquery.rs:376-396` `extract_subquery_table_names`——`TableFactor::Table { name, .. }` 与 join 臂丢弃 `alias` 字段，输出仅基表名；五处消费：本文件 IN 臂 :41、EXISTS 臂 :101、`build_scalar_subquery_binding` :177、`src/parser/planner/query.rs:1428` 投影位路径，及分类本体。
  2. `src/parser/planner/expression.rs:258-272` `qualified_column_expression`——`self.inner_table_names` 非空且限定符 ∉ 内层集时改写为 `ParameterExpression("{table}.{column}")`（:269-270）。
  3. `src/parser/planner/subquery.rs:404-542` `extract_correlated_params`/`collect_outer_column_refs`——同一缺别名集做相关参数分类（限定符不在 inner_tables 即 CorrelatedParam）。
  4. 执行器绑参静默回落：`src/executor/semi_join.rs:151-166`、`src/executor/scalar_subquery_filter.rs:86-102`、`src/executor/subquery_eval.rs:160-175` 三处 `extract_param_values` 按裸列名查外层索引表，未命中 `.unwrap_or(0)` 取外层首列。四种错误表象（恒空集/截断/全放行/碰巧正确）均由此推出并与实测逐一吻合（design.md 根因链）。
  5. ISS19：`src/storage/error.rs:111` `#[error("execution error: {0}")]` 与 `src/pipeline.rs:1300` `format!("Execution error: {}", e)` 叠加；`tests/subquery_test.rs:632` 锁定双层形态。
  6. sqlparser 0.44 字段事实：`TableFactor::Table { name, alias: Option<TableAlias>, .. }`，`TableAlias { name: Ident, columns }`（registry `query.rs:986-989`）；Derived 臂在 `query.rs:430` 解构，别名同 `Option<TableAlias>`。
- Code and Critical Path: 计划器 `subquery.rs`（分类）→ `expression.rs`（改写）→ 计划节点 `SemiJoinNode/AntiJoinNode/ScalarSubqueryBinding/SubqueryEvalNode` → 执行器三 `extract_param_values` + `inject_correlated_values`（`src/executor/correlated.rs`）→ 错误上浆 `pipeline.rs:1300`。既有相关子查询用例：`tests/subquery_test.rs`（43 例）、`tests/scalar_subquery_comparison_test.rs`（14 例），别名形状零覆盖（本 change 补）。

**Implementation Guidance**

顺序 T1 → T2 → T3 → T4。T1 只改 `extract_subquery_table_names` 一个函数体（声明限定符 = `alias.as_ref().map(|a| a.name.value.clone()).unwrap_or_else(|| object_name_to_table_name(name))`，JOIN 臂与 Derived 臂同模板，Derived 无别名时跳过该臂）；消费方与 `qualified_column_expression` 的后续解析臂（`resolve_table_qualifier` 别名→真实键归一，`expression.rs:305`）零改动。T2 三处同构改动（签名 `Result` 化 + 调用点 `?` 上浆），错误文案统一 `correlated parameter column not found in outer row: {column}`。T3 单行 thiserror 属性变更 + 一处断言校准。T4 按 `tests/e2e_test.rs` 既有 journey 函数风格新增（参照 `e12_error_prefix_single_layer` 的错误断言与 `run_cli` 夹具）。A1 现状锁定：`FROM c x ... WHERE c.age = 1`（基名限定 + 别名内层）行为以先行测试锁定现状，不主动改语义。

**Task Contracts**

### T1: 声明限定符提取修复 + 先行集成测试矩阵

- Requirement/Scenario: R1；spec delta sql-table-alias-resolution 六 Scenario + 零回归 Scenario
- Depends on: None
- Targets: `src/parser/planner/subquery.rs::extract_subquery_table_names`
- Current behavior: `FROM c x ... x.age` 的 `x.age` 被分类为幻影相关参数，EXISTS/IN/标量四种静默错误表象（ISS16–ISS18）
- Required behavior: 内层集含声明限定符（别名优先，含 JOIN 臂与 Derived 别名）；别名限定引用解析为内层列，EXISTS/NOT EXISTS/IN/NOT IN/标量全部按真实语义返回——**含覆盖缺口审计新增四形状**（NOT EXISTS 别名全放行修复、NOT IN 别名全放行修复、EXISTS JOIN 双臂别名恒空集修复、EXISTS Derived 别名截断修复）；`EXPLAIN` 对应计划 `correlated_params` 从虚高值回落为真实相关参数数
- Preserve: 无别名/基名限定形状的现有结果；多级相关计划期拒绝；JOIN 形 IN 拒绝；`extract_subquery_table_names` 的返回值类型与调用签名
- Forbidden: 不改 `qualified_column_expression` 及其后解析臂；不改 `ast.rs` 其他别名点；不改任何执行器（T2 范围）
- First test: 新建 `tests/subquery_alias_binding_test.rs`，**形状覆盖矩阵**：算子 {EXISTS, NOT EXISTS, IN, NOT IN, WHERE 标量, 投影位标量} × 内层形态 {单表别名, 单表别名+JOIN 臂别名, 派生表别名} × 谓词列 {首列, 非首列} × 相关性 {非相关, 相关经外层 PK, 相关经外层非 PK} × IN 面 {无索引, 二级索引谓词}——按覆盖缺口归因（design.md）裁剪为可判定用例集（全组合不强制穷举，每维度至少一判别式用例），含四审计新形状的判别式期望（NOT EXISTS 全覆盖期望 {}、EXISTS JOIN 内层期望 {1}、Derived 内层期望 {1,2} 等，非碰巧正确），外加无别名基名限定零回归锚点与 A1 现状锁定用例；当前代码下修复形状断言失败（RED），零回归/A1 形状通过（变更前 GREEN）
- GREEN condition: 矩阵全量按真实语义期望通过（含四审计新形状）
- Verification: `cargo test --test subquery_alias_binding_test` 全绿；`cargo test --test subquery_test --test scalar_subquery_comparison_test` 零回归
- Stop when: 修复需触碰 `qualified_column_expression` 解析臂（说明分类修正不足，返回 Plan）；零回归形状失败（A1 假设破裂，按 Stop 返回 Plan 决策）

### T2: 三执行器绑参失败点名报错

- Requirement/Scenario: R2；correlated-subquery-cache delta 两 Scenario
- Depends on: T1（幻影参数不再经 SQL 产生；加固面向未来漏分类）
- Targets: `src/executor/semi_join.rs::SemiJoinExecutorV2::extract_param_values`、`src/executor/scalar_subquery_filter.rs::ScalarSubqueryFilterExecutor::extract_param_values`、`src/executor/subquery_eval.rs::SubqueryEvalExecutor::extract_param_values`（含各自 `next()`/`eval_binding` 调用点上浆）
- Current behavior: 外层列名未命中 `.unwrap_or(0)` 静默取首列，产出错误行集
- Required behavior: 未命中返回 `StorageError::ExecutionError`，消息含未命中的外层列名，无数据行产出；命中路径行为与结果逐字节不变
- Preserve: 命中路径的缓存语义（`correlated_rows_cache`/`correlated_cache` 键与生命周期）；`extract_param_values` 对命中列的索引映射结果
- Forbidden: 不改计划器；不改缓存键结构；不新增 SQLSTATE 分类
- First test: lib 内联单测（`mod tests` 先例见 `src/executor/predicate.rs` 等），直接构造三执行器、correlated_params 引用外层不存在的列，断言 `next()` 报错且消息点名该列；当前代码下断言失败（RED：返回首列值继续执行）
- GREEN condition: 三执行器单测全绿；既有相关子查询用例（含 T1 新矩阵）全绿
- Verification: `cargo test --lib executor` 定向 + `cargo test --test subquery_alias_binding_test --test subquery_test` 零回归
- Stop when: 命中路径出现任何行为变化（返回 Plan）

### T3: 执行器错误前缀单层化

- Requirement/Scenario: R3；error-reporting delta「执行器错误单层前缀」Scenario
- Depends on: None（可与 T2 并行）
- Targets: `src/storage/error.rs:111`（`ExecutionError` 的 `#[error]` 属性）、`tests/subquery_test.rs:632`（锚点断言）
- Current behavior: CLI 输出 `Execution error: execution error: Subquery returns multiple rows ...`（双层）
- Required behavior: 输出恰一层 `Execution error: ` 前缀（`StorageError::ExecutionError` Display 无前缀，漏斗 `pipeline.rs:1300` 唯一赋予）；SQLSTATE 与退出码不变；其余 `StorageError` 变体文案零变化
- Preserve: parse/plan 通道单层形态（ISS11 已定）；`SubqueryReturnsMultipleRow` 的 PlanError Display 本体文案
- Forbidden: 不改 `pipeline.rs:1300` 漏斗；不借机重排其他错误变体文案
- First test: `tests/subquery_test.rs:632` 锚点断言由双层改为单层（同一触发语句）；当前代码下 RED（输出仍为双层）
- GREEN condition: 该断言绿；`cargo test --test subquery_test` 全绿；`grep -rn "execution error: execution error" tests/ src/` 零命中
- Verification: 定向 `cargo test --test subquery_test`；CLI 人工冒烟一次 `rtsql` 多行标量错误输出单层（e2e T4 承载自动化）
- Stop when: 有其他测试锁定双层形态未被清单覆盖（返回 Plan 补校准清单）

### T4: e2e 三 issue 形状真实 CLI 见证

- Requirement/Scenario: R1/R2/R3 端到端；三 issue 台账复现形状 + 对照形状
- Depends on: T1–T3 定稿
- Targets: `tests/e2e_test.rs`（新增 journey 函数，风格同 `e12_error_prefix_single_layer` 等既有函数）
- Current behavior: e2e 对子查询别名形状零见证、无子查询 journey（覆盖缺口归因面）
- Required behavior: 真实 CLI 子进程覆盖：ISS16 WHERE 相关标量（行集正确）与投影位相关标量（形状正确）；ISS17 EXISTS 别名四形态（至少两形态：谓词非首列 + 相关非 PK 全放行修复）；**审计新增：NOT EXISTS 内层别名（全放行修复）、EXISTS 内层 JOIN 双臂别名（恒空集修复）**；ISS18 IN×别名×索引；每形状带无别名对照；执行器错误输出单层断言（R3 端到端）。全部 exit 0 且行集/文案断言
- Preserve: 既有 e2e journey 函数零修改；夹具复用 `fixture()/run_cli()`
- Forbidden: 不改动产品代码；不为本任务新增夹具基础设施
- First test: e2e 新函数先行提交观察 RED（T1–T3 未落地前形状失败），T1–T3 落地后 GREEN；若 Act 采用 T1–T3 落地后一次性补测试，须在 Act Response 记录 RED 观察（CLI 直接运行台账命令的错误输出）
- GREEN condition: 新 journey 全绿，`cargo test --test e2e_test -- --test-threads=1` 全绿
- Verification: `cargo test --test e2e_test -- --test-threads=1`（定向）；Iteration Gate 全量一次
- Stop when: 夹具能力不足以构造形状（返回 Plan）

**Invariants**

- 全量既有测试零修改通过（除 T3 声明的一行锚点校准）；多级相关/JOIN 形 IN 拒绝面不变；退出码矩阵不变；缓存 requirement 语义不变。
- T1–T3 均 SHALL NOT 改变无别名与基名限定形状的现有结果（A1）。

**Non-goals**

proposal Non-goals 全项；本 Cycle 不处理 ISS04、不关闭任何 issue（关闭随 change 收尾由用户指令执行）。

**Acceptance**

- R1：形状覆盖矩阵按 spec delta 期望全量通过（先行测试 `tests/subquery_alias_binding_test.rs` + e2e），含四审计新形状判别式期望；代码落点 `subquery.rs::extract_subquery_table_names`。覆盖缺口归因（design.md 专节）随 change 归档，作为后续测试编写的形状矩阵参照。
- R2：三执行器未命中点名报错（lib 单测），命中路径逐字节不变（T1 矩阵零回归承载）。
- R3：`Execution error:` 单层（subquery_test 锚点 + e2e 断言），其余错误面零回归。
- RTM：

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 子查询内层声明限定符参与外层引用分类 | T1 | `tests/subquery_alias_binding_test.rs` 六形状（RED 先行） | Covered |
| R1 端到端见证 | T4 | `tests/e2e_test.rs` 新 journey（ISS16/17/18 形状） | Covered |
| R2 关联参数外层绑定失败显式报错 | T2 | 三执行器 lib 内联单测（RED 先行） | Covered |
| R2 端到端/等价锚 | T4 | e2e 相关 EXISTS 形状 | Covered |
| R3 执行器错误单层前缀 | T3 | `tests/subquery_test.rs:632` 锚点校准（RED 先行） | Covered |
| R3 端到端 | T4 | e2e 错误单层断言 | Covered |

- 全量 `cargo test`（1999 基线 + 新增）零失败为 Iteration Gate。

**Verification**

逐 scenario 直接判定：集成/单测命令真实运行（上表）；跨接口 scenario（R1 端到端、R3 端到端）走真实 CLI 入口（e2e 子进程）。无身份型证据工程；全量一次为 Iteration Gate 时机，禁止任务内重复跑全量。

**Gate 2 Readiness**

PASS（逐项：BDD 缺口扫描完成且缺口以默认假设 A1–A3 闭合——用户在 proposal 批准时一并接受；场景草图 = 三域 delta spec；调查来自实际代码与逐字验证的缺陷现场，基线采信未失效结论；Task Contract 含变更面/先行测试/GREEN/验证/停止条件；调用者与测试入口已定位；**覆盖缺口归因与同类面审计完成（用户 2026-10-15 指令），四个同根因新形状确认并并入矩阵，无额外产品代码变更**；范围变化经用户同指令批准；Iteration 单批通过平衡审计（矩阵并入 T1，不碎化）；Persisted Evidence = none——全部结论可低成本重跑；RTM 无 Missing/Simplified。用户批准：2026-10-15「可以，交由你来跑plan」+ 同日覆盖缺口调查并入指令）。

**Persisted Evidence**

- Mode: none

全部验收经 `cargo test` 定向命令与 e2e 可低成本重跑；Act Response 足以承载验证结论；无一次性环境或不可复现现场。

**Risks and Notes**

- R1 风险：`EXPLAIN` 输出 `correlated_params` 计数在修复后回落（虚高值消失），若 e2e/集成有断言锁定旧计数需校准——T4 先行测试暴露时按 T3 同款锚点校准法处理并在 Act Response 记录。
- 非实质未知项：Derived 臂无别名时的提取行为（以现场字段为准，Act 按「无别名不输出」处理并在 Self-Review 记录）；A1 基名限定现状语义以先行测试锁定为准。
- 三 issue 的 close/归档不在本 Cycle；建议同批调查已在台账标注（ISS16/17/18 同根因）。

## Act Response

- Status: reported

### 已实施

| Task | 改动 | 文件 / 符号 |
|---|---|---|
| T1 | `extract_subquery_table_names` 改为输出**可解析限定符**（别名优先 + 基表名并列保留），新增私有辅助 `declared_qualifiers`；JOIN 臂同规则，`Derived` 取别名，其余 `TableFactor` 变体不产出 | `src/parser/planner/subquery.rs`（+34/-7） |
| T2 | 三处 `extract_param_values` 由 `unwrap_or(0)` 静默回落改为 `Result`：未命中外层列报错 `correlated parameter column not found in outer row: {列名}`（`StorageError::ExecutionError`），调用点 `?` 传播 | `src/executor/semi_join.rs`、`src/executor/scalar_subquery_filter.rs`、`src/executor/subquery_eval.rs` |
| T3 | `ExecutionError` 的 thiserror 模板去掉自带 `execution error: ` 前缀，单层前缀由 pipeline 漏斗唯一赋予；锚点断言同步校准为单层 | `src/storage/error.rs`（+3/-1）、`tests/subquery_test.rs` |
| T4 | 5 个真实 CLI 子进程旅程见证 ISS16/ISS17/ISS18 台账形状与三个审计新增形状，各带无别名对照；含 R3 单层前缀端到端断言 | `tests/e2e_test.rs`（+172） |
| 测试支撑 | T2 单测需实例化执行器，新增 `#[cfg(test)]` 行序列执行器 `StubRowExecutor` | `src/executor/mod.rs`（+24） |
| 先行测试 | 新建形状覆盖矩阵：22 用例（算子 6 面 × 内层形态 3 面 × 谓词列位置 × 相关性 3 面 × IN 索引面 + EXPLAIN 计划面 + 拒绝面） | `tests/subquery_alias_binding_test.rs`（新建） |

TDD 顺序与 RED 见证：T1（修复前 `10 passed; 12 failed` → 豁免后 22 全绿）、T2（`3 failed` / 其余 403 passed → 406 全绿）、T3（锚点断言 RED → GREEN）、T4（首次 RED 为我的 SQL 写法错误，非缺陷信号，修正后 5 旅程 GREEN）。

### Blocker Handoff（T1 首次 GREEN 时命中，保留原文）

- **发现位置**：T1 / GREEN 验证步骤；T1 Task Contract 的 Stop-when「零回归形状失败（A1 假设破裂，按 Stop 返回 Plan 决策）」。
- **事实**：`SELECT id FROM a WHERE EXISTS (SELECT 1 FROM c1 y WHERE c1.age = 41)` 在修复前返回 `[1, 2]`，按「声明限定符仅取别名」实现后返回 `[]`。取证：`git stash push -- src/parser/planner/subquery.rs` 后同用例 GREEN，恢复后 FAILED。
- **机制**：内层集由基表名 `{c1}` 变为别名 `{y}` → `c1.age` 未命中 → `expression.rs:269-270` 改写为幻影 `ParameterExpression` → `semi_join.rs:151` 按裸列名未命中后取外层首列 → 恒空集。
- **与计划的偏差**：Implementation Guidance 给定的模板与默认假设 A1（「基名限定形状保持现状语义不变」）互斥。
- **当时给出的三方案**：①内层集并列保留基表名；②计划期点名拒绝 `TableNotFound` 42P01（需碰 `qualified_column_expression`，变更面扩大）；③接受回归并改写 A1。

### Blocker Resolution

- **用户指令原话**：「给出豁免，直接按你的建议进行改动，只需要在回复提及相关问题就行」。
- **裁定**：采纳方案 ①——内层集在别名之外并列保留基表名。风险与代价（用户已知悉）：本引擎在该写法上比标准 SQL 宽松（标准 SQL 在别名后禁用基表名，SQLite 报 `no such column`）；影子风险为外层别名与内层基表同名时的遮蔽语义——该语义按 proposal Non-goals 保持「内层优先」现状，未新增遮蔽面。
- **执行**：`declared_qualifiers` 对 `TableFactor::Table` 在 `alias != base` 时返回 `[alias, base]`，其余情况返回 `[base]`；仍为单函数体 + 辅助，未触碰 T1 Forbidden 列出的 `qualified_column_expression` 及其后解析臂、`ast.rs`、任何执行器。
- **R1 delta 正文**已按裁定补写「声明了别名时 SHALL 同时保留基表名」，并注明裁定来源。
- **恢复点**：T1 的 A1 用例。重跑后 `a1_base_name_qualifier_inside_aliased_inner` GREEN，矩阵 22/22。
- **豁免后重跑的 Gate**：T2/T3/T4 与 Iteration Gate 全部在裁定后的代码状态上重新验证（见验证记录），无沿用阻塞前结论。

### Deviations from Plan

- T1 模板偏离：按用户裁定改为「别名 + 基表名并列」（已在 Blocker Resolution 记录）。
- T1 Implementation Guidance 的 A1 默认假设被实测证伪并修正，新增 A1 回归锚点用例。
- T4 先行测试初稿含 `IN × 内层 JOIN` 形状，撞上既有 0A000 拒绝面（proposal Non-goals 明列的拒绝面零变化项）——已删除该形状，改补两条精确拒绝断言（文案 + SQLSTATE `0A000`，含「别名不得成为绕开拒绝面的通道」）。
- T2 先行测试初稿含三条直接调用 `extract_param_values` 的命中路径单测——签名变更前无法编译，会掩盖真实 RED，已移除；命中路径不变由集成矩阵与既有相关子查询套件承担。
- T2 契约要求 lib 单测即 `extract_param_values` 未命中分支，实操中改为经 `next()` 触发（规格场景 1 的原文形状），更贴近规格且编译前后一致。

### 自检发现

- A1 先行测试初稿误写为顶层单表形状（`SELECT id FROM c1 y WHERE c1.age = 41`）——该形状不经 `inner_table_names` 分类面，恒 GREEN、不验证契约意图。已改为子查询内层形状后暴露真实回归。测试编写错误，已修正。
- `tests/subquery_alias_binding_test.rs` 的 `error_msg` 辅助在改为精确匹配 `Response::Error` 后无调用点，`dead_code` 警告——已删除。
- `tests/e2e_test.rs` 中 `e14_execution_error_single_prefix` 草稿残留一条未使用查询（`unused variable`）——已删除。
- 三条 e2e 期望值系我按种子数据误判：`dept` 两行 vs `people` 三行、`CREATE INDEX` 仅支持单列 INT（`name` 被拒）、零计数分支应为 `[[3]]`——均按真实语义修正，非产品缺陷。
- 遗留 Minor：`tests/secondary_index_equality_test.rs:26` 有一条 `clippy::single_match` 警告，为本次改动前既有，未在本 change 范围内处理（未清理无关代码）。
- 未解决 Critical / Important：无。

### 验证记录

| 验证 | 命令 | 结果 |
|---|---|---|
| T1 RED | `cargo test --test subquery_alias_binding_test -- --test-threads=1`（stash 产品改动） | `10 passed; 12 failed` |
| T1 GREEN（豁免后） | 同上 | `22 passed; 0 failed` |
| T2 RED | `cargo test --lib` | `3 failed; 403 passed` |
| T2 GREEN | `cargo test --lib` | `406 passed; 0 failed` |
| T3 RED | `cargo test --test subquery_test -- ... test_correlated_subquery_error_not_cached` | FAILED（双层 → 单层期望） |
| T3 零残留 | `grep -rn "execution error: execution error" tests/ src/` | 零命中 |
| T4 | `cargo test --test e2e_test -- --test-threads=1` | `27 passed; 0 failed`（22 既有 + 5 新增） |
| Iteration Gate | 按 M19 分批串行：lib + bins + 13 批集成（每批 10 目标，`--test-threads=1`，批间顺序执行）+ e2e 独立批 | **2029 passed / 0 failed / 2 ignored** |
| 对账 | — | 2026-10-14 基线 1999 + 新矩阵 22 + lib 单测 3 + e2e 旅程 5 = **2029**，逐项吻合 |
| 格式 | `cargo fmt --check` | 0 处未格式化 |
| 静态 | `cargo clippy --all-targets` | 本次改动 0 警告（唯一警告为既有 `secondary_index_equality_test.rs:26`） |

分批明细（passed）：lib 406 / bins 0 / B01 148 / B02 182（含 2 ignored）/ B03 147 / B04 155 / B05 85 / B06 84 / B07 96 / B08 203 / B09 90 / B10 140 / B11 161 / B12 88 / B13 17 / e2e 27。

分批脚本缺陷记录（不影响结论）：末批仅 2 个目标时 `shift 10` 越界失败，循环未终止并重复运行末批目标；B01–B13 与 e2e 的首次结果均为 0 failed，重复轮次未计入统计。

### Persisted Evidence

None required——全部验收经单条定向命令或低成本重跑即可复现，Act Response 已承载前后态取证与全量分批计数；无一次性环境或不可复现现场。

### Experience Candidates

None——本 change 范围外未发现实质缺陷；分批脚本的 `shift` 越界缺陷属一次性命令失误，不构成可复用操作经验。

## Plan Review

- Review Result: accepted

**Findings**

1. **[Issue 候选，非阻塞｜偏差分类：Plan 遗漏]** `AntiJoinExecutor::extract_param_values`（`src/executor/anti_join.rs:151-161`）不在 T2 加固范围：未命中相关参数经 `filter_map` 被**静默丢弃**（缓存键缺该参数），与已加固三执行器的 `unwrap_or(0)` 同根因但形态更隐蔽。T1 后正常 SQL 不可触达（幻影参数源已消除，真实相关参数按构造必在外层行），与 R2 的可达性前提一致；R2 spec 显式只约束 SemiJoin/ScalarSubqueryFilter/SubqueryEval 三执行器，不阻塞本 Cycle Acceptance。但 T2 的防御目标（未来漏分类显式报错）在 NOT IN/NOT EXISTS 路径上仍有缺口。只报告不落账，是否登记新 Issue 由用户裁定。
2. **[Minor｜Act 声明精度]** Act Response 变更量计数与实际不符：`src/parser/planner/subquery.rs` 声明 +34/-7，实际 numstat +30/-7；`tests/e2e_test.rs` 声明 +172，实际 +255（内容全部在 T4 契约内，无未声明改动，仅计数偏低）。不阻塞。
3. **[信息，不处理]** `tests/subquery_alias_binding_test.rs` `where_scalar_subquery_correlated_zero_count_filters_row` 注释「内层无匹配时」与种子数据（全部外层行有匹配，断言 [] 由 `= 0` 分支命中）不严格对应；断言本身正确，真实零计数形状由 e2e `e14_subquery_alias_binding_where_scalar` 的 `[[3]]` 分支承载。

**Acceptance Gaps**

None。

**Evidence**

- 独立 diff 阅读（9 文件，产品 5 + 测试 3 + references 1）：T1 单函数修复与豁免裁定一致（`declared_qualifiers` 别名+基名并列、JOIN 臂同规则、Derived 取别名、其余变体不产出）；T2 三执行器同构 `Result` 化、命中路径逐字节保持（`row.get(idx).cloned().unwrap_or(Value::Null)` 未动）、错误文案统一点名列名；T3 Display 去前缀 + 唯一漏斗点保持；T4 五旅程内容逐一在契约范围内。
- 消费方核查：`extract_subquery_table_names` 全部八处消费（`subquery.rs:39/:99/:176/:502/:523/:594/:610`、`query.rs:1433`）均为限定符分类 / 相关参数提取 / `inner_table_names` 构建，无任何 catalog 查表消费，别名入集不引入新解析面；多级相关拒绝的 `all_allowed` 并集语义在别名入集后保持（内层集之外的限定符仍拒绝），遮蔽语义属 proposal Non-goals 明示的「内层优先」现状。
- 采信 Act 验证记录（HEAD 未动、覆盖范围未变）：Iteration Gate 2029 passed / 0 failed / 2 ignored，对账 1999+22+3+5=2029 吻合；fmt / clippy 零新增。
- 本会话重跑复核：`grep -rn "execution error: execution error" tests/ src/` 零命中（grep exit 1），与 Act 声明一致。
- 新矩阵 22 用例逐维核对契约：六算子 × 三内层形态 × 谓词列位置 × 相关性三态 × IN 索引面 + A1 锚点 + EXPLAIN `correlated_params` 回落断言（0 与 1 双向）+ JOIN 形 IN 拒绝面 ×2（含别名不得绕开拒绝面）。
- Blocker Handoff 处理合规：用户豁免原话保留、裁定（方案①并列保留）落码、R1 delta 补写豁免来源、A1 回归锚点新增、豁免后全部 Gate 在裁定代码状态上重跑。

**Follow-up Decision**

R1（矩阵 22/22 + e2e 五旅程 + 四审计新形状判别式期望）、R2（三执行器点名报错 + lib 单测）、R3（锚点校准 + e2e 端到端 + 零残留）全部满足，Iteration Gate 全绿，既有测试仅 `:632` 一处声明的锚点校准。三个 finding 均不阻塞 Acceptance；无 Acceptance Gap，无需 rework / replan。Finding 1 为范围外同类残留，作为 Issue 候选上报，随用户裁定。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

None（单 Iteration change，Iteration 000 完成）。
