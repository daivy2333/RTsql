# Proposal: 子查询别名限定符绑定修复（ISS16–ISS18）与执行器错误前缀单层化（ISS19）

- Date: 2026-10-15
- Status: approved（2026-10-15 用户批准：「可以，交由你来跑plan」，范围 = 本 proposal What Changes 全项）
- 来源：2026-10-15 用户实测评测（ISS16/ISS17/ISS18/ISS19，台账 `.agents/issues/ISS16-*.md`–`ISS19-*.md` / R44–R47）+ 同日代码级根因调查（本 change design.md）

## Why

2026-10-15 实测评测在子查询面发现三个「静默错误结果」正确性缺陷与一个错误文案缺陷，全部 exit 0/3 无警告，且上游 subquery 相关测试全绿（形状覆盖缺口）：

| Issue | 现象（最小复现见台账） |
|---|---|
| ISS16 | 相关标量子查询外参未绑定：WHERE 形态静默空集；select list 形态输出塌缩或报 `Subquery returns multiple rows` |
| ISS17 | `EXISTS` 内层表带别名：四种静默错形态（谓词非首列恒空集、谓词首列按内层存活行数截断外层、相关经外层非 PK 列全放行、评测主库同形状全拒） |
| ISS18 | `IN` 内层带别名 ∧ 谓词由二级索引服务 → 恒空集 |
| ISS19 | 执行器路径错误文案双层前缀 `Execution error: execution error:`（`tests/subquery_test.rs:632` 锚点锁定该形态；ISS11 修复 parse/plan 漏斗后的执行器漏网点） |

根因已代码级确认（详见 design.md）：`extract_subquery_table_names` 丢弃表别名 → 内层别名限定引用（`x.age`）被 `qualified_column_expression` 与 `extract_correlated_params` 双重误判为相关外层引用（幻影 `ParameterExpression`）→ SemiJoin/ScalarSubqueryFilter/SubqueryEval 走相关路径，执行器按裸列名绑参并以 `unwrap_or(0)` 静默落到外层首列 → 随数据形态产生空集/截断/全放行/碰巧正确四种表象。ISS19 根因：`StorageError::ExecutionError` 的 thiserror 前缀与 `pipeline.rs` 漏斗前缀叠加。

**测试覆盖缺口归因（用户 2026-10-15 指令并入本 change 调查）**：缺陷长期潜伏的直接原因是形状覆盖缺口——既有 57 个子查询用例（`subquery_test.rs` 43 + `scalar_subquery_comparison_test.rs` 14）**内层表零别名**（全部基名限定或内层非限定引用），从未进入缺陷所在的「内层别名限定」路径；相关用例全部落在「碰巧正确」区（外层单键列 / PK 关联 / 幻影与真实参数同名同值）；AntiJoin（NOT IN/NOT EXISTS）零别名覆盖；`EXPLAIN` 的 `correlated_params` 虚高值无断言锁定；e2e 无子查询 journey；且 `subquery_test.rs:632` 一条断言主动把 ISS19 的双层文案锁定为期望形态。**同类面审计**（2026-10-15 Plan 基线探针）确认同根因的未覆盖形状至少还有四个：NOT EXISTS 内层别名（全放行）、NOT IN 内层别名（全放行）、EXISTS 内层 JOIN 双臂别名（恒空集）、EXISTS 内层派生表别名（截断）；全库 grep 确认锁定 bug 的断言仅此一处。机制性原因：测试按「当期已实现形状」编写，缺少「别名 × 算子 × 相关性 × 内外层形态 × 索引」的形状矩阵思维。覆盖修复并入本 change T1/T4（见下）。

## What Changes

1. **T1 修复别名丢弃（唯一实质产品变更点，`src/parser/planner/subquery.rs::extract_subquery_table_names`）**：内层限定符集改为取每个 FROM 项的**声明限定符**——显式别名（`AS` 或隐式）优先，无别名取表名；JOIN 臂同规则；`TableFactor::Derived`（派生表）取别名。消费方（`qualified_column_expression` 的 ParameterExpression 分类、`collect_outer_column_refs`/`extract_correlated_params` 的相关参数分类、`try_build_where_subquery` 两处、`query.rs` 投影位子查询路径）经同一函数自动一致修复，零调用点改动。先行测试：`tests/subquery_alias_binding_test.rs` 新文件，**形状覆盖矩阵**（覆盖缺口归因的修复载体）：算子 {EXISTS, NOT EXISTS, IN, NOT IN, WHERE 标量, 投影位标量} × 内层形态 {单表别名, 单表别名+JOIN 臂别名, 派生表别名} × 谓词列 {首列, 非首列} × 相关性 {非相关, 相关经外层 PK, 相关经外层非 PK} × IN 面 {无索引, 二级索引谓词}，每形状期望值按真实语义推导（含 G1/G2/G4/G5 四个新审计形状的判别式期望，非碰巧正确），外加无别名基名限定零回归锚点（RED 先行）。
2. **T2 执行器绑参加固（防御纵深，三个执行器）**：`SemiJoinExecutorV2::extract_param_values`（`src/executor/semi_join.rs:151`）、`ScalarSubqueryFilterExecutor::extract_param_values`（`src/executor/scalar_subquery_filter.rs:86`）、`SubqueryEvalExecutor::extract_param_values`（`src/executor/subquery_eval.rs:160`）的 `unwrap_or(0)` 静默回落改为点名报错（`ExecutionError` 携带未命中的外层列名）。T1 后幻影参数不再产生，本项把任何未来漏分类从静默错结果转为显式报错。先行测试：执行器级单元测试构造外层缺列场景断言报错。
3. **T3 ISS19 单层前缀**：`src/storage/error.rs:111` `ExecutionError` 的 thiserror 前缀去除（`"execution error: {0}"` → `"{0}"`），单层 `Execution error: ` 由 `pipeline.rs:1300` 漏斗唯一赋予（与 ISS11「漏斗点单层赋予」同手法）。先行测试：`tests/subquery_test.rs:632` 锚点校准为单层（当前文案下 RED）。
4. **T4 e2e 见证**：`tests/e2e_test.rs` 以真实 CLI 子进程承载台账与审计形状：ISS16（WHERE 相关标量 + 投影位相关标量）、ISS17（EXISTS 别名谓词非首列 + 相关经非 PK 全放行修复）、**NOT EXISTS 内层别名、EXISTS 内层 JOIN 双臂别名**（审计新增）、ISS18（IN × 别名 × 索引谓词），每形状带无别名对照与 exit 0/行集断言，锁住回归面。

## Non-goals

- 多级相关子查询（维持既有计划期拒绝）与 JOIN 形 IN 子查询（维持既有诚实拒绝）——拒绝面零变化。
- 内层 FROM 派生表（DerivedScan）基座语义、子查询内集合运算拒绝面——零变化。
- 「外层别名与内层表同名遮蔽」的语义调整（ISS17 附带观察，保持内层优先的现状语义，另行决定）。
- 文档（README/SKILL.md）变更——文档已宣称相关子查询可用，修复使文档为真，无文案变化。
- `src/parser/ast.rs:354/362` 等其他别名丢弃点——非子查询限定符分类消费方，本 change 不扩面。

## 默认假设（集中决策项——已取默认值闭合，非 TBD；用户审计可调整）

- **A1** 内层 FROM 声明别名后，基表名限定引用（`FROM c x ... WHERE c.age = 1`）保持引擎现状语义不变，以先行测试锁定现状；若现状与预期不符，按既有行为锁定并另报 issue。
- **A2** T2 把静默回落改为显式报错属错误面收紧（仅 planner 漏分类路径可达），用户已隐含接受；不引入新的 SQLSTATE 分类，沿用 58000 兜底。
- **A3** 相关参数绑定按裸列名匹配的既有机制不改（多表外层同名列的歧义语义另题），本 change 只消除「幻影参数」产生源与静默回落。
