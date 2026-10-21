# Iteration 001 / Cycle 001: replan——分类/收集判定单源化与碰撞形状 SQLite 语义（T02-C1 修订）

## Plan Context

- Status: ready（2026-10-20 Gate 2 PASS；用户裁定并授权实施——原话：「我们直接给出replan cyc进行修复实施的同时解决碰撞形状的旧语义本身与 SQLite 标准相悖这个issue」）
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T02（C1 面，修订契约）
- Acceptance gaps: 父 Cycle `000-initial.md` 遗留——碰撞形状分类/收集两侧判定基准脱节致未绑定参数静默 NULL（父 Cycle Plan Review Finding 1，探针实锤：自相等变体旧 `{1,2,3}` / 现状 `{}` / SQLite `{1,2,3}`）
- Repair items: None（replan 使用修订后的全局 task 契约）
- Inherited scope: `sql-semantic-binding` R1（含新增碰撞场景）全部、R2（含例外）全部；父 Cycle 已交付的绑定作用域表基建（`binding.rs` ScopeEntry + 五登记点 + 六处快照/恢复 + `table_not_null` 通道）原样继承，本 Cycle 不重做；Iteration 000 差分网 GREEN 基线（2045/0/2，白名单 6 条目冻结）
- Excluded scope: C2 布局统一（Iteration 002）；执行器 `condition_column_index` 与其余执行期按名点；`ColumnRef` 载荷与 `get_plan_output_columns`（含差分发现 ③）；视图展开/CTE/attach 段登记完备化；差分白名单条目增删；①②④⑤ 缺陷处置；收集面扩展到 WHERE 外位置；A1 宽松形状改全遮蔽语义。

**Objective**

碰撞形状（外层同名未别名声明 + 子查询内层别名声明同表 + 基名限定引用）在四子查询面（IN / EXISTS / WHERE 标量 / 投影位标量）解析为外层相关参数并正确注入求值（对齐 SQLite）；分类与收集两侧判定经同一 `is_outer_reference` 谓词单源化；其余全部输入行为与父 Cycle 基线逐字节一致（含 A1 宽松内层解析、`derived.k` 病理形状错误面、白名单跳过行为）。

**Background**

父 Cycle Act 把分类切到作用域身份判定，但收集侧 `collect_outer_column_refs` 仍是纯名单判定：碰撞形状（名单经 A1 基名并列收录、内层作用域段只登记别名）分类产出 `ParameterExpression` 而收集不收录 → 执行期未绑定参数静默求值 NULL。2026-10-20 用户裁定：不恢复旧语义，碰撞形状直接对齐 SQLite 标准语义（别名遮蔽基名 → 外层相关参数），语义演进与回归修复合并本 Cycle 实施。语义方向与 A1 豁免的边界：碰撞形状改外层语义；无外层同名的基名引用维持 A1 宽松内层解析（2026-10-15 豁免在其非碰撞子集继续有效）。

**Investigation Facts**

- Current Baseline: 父 Cycle Act Response 工作区（五文件 +115/−3 + `binding.rs` 76 行，未提交）；全量 2045 passed / 0 failed / 2 ignored（父 Cycle Gate，M19 分批实测）；差分网 10/0 白名单 6 条目冻结。本 Cycle 在同一工作区上继续。
- Current-State Evidence（2026-10-20 本会话实读 + 探针实测；行号当前有效，Act 无需回读其他资料）:
  - **分类点**：`qualified_column_expression`（`expression.rs:258`，`&self`——不可改 `&mut`，`build_expression`（`expression.rs:330`）亦 `&self`，连锁波及全 planner）优先级 1（:269-281）：`inner_table_names` Some 时，内层段未命中（`binding_scope_hit(table_ref, boundary, len)`）且（外层段命中（`[0, boundary)`）或不在名单）→ `ParameterExpression(param_name)`。入参在调用点已小写化（:390-391 两段臂 / :399-404 三段臂），`param_name` = 小写 `"table.column"`。**分类判定已实现用户裁定的碰撞语义**（碰撞格产出参数），本 Cycle 仅单源化公式。
  - **收集点**：`extract_correlated_params`（`subquery.rs:452`，`&self`）取子查询 WHERE selection（:457-463）→ `collect_outer_column_refs`（:474，`&self`）遍历表达式树；CompoundIdentifier 两段臂（:481-490）谓词 = 纯名单成员资格（`!inner_tables.iter().any(eq_ignore_ascii_case)`）——**脱节点**：碰撞形状名单命中 → 不收录。遍历变体：BinaryOp / UnaryOp / Nested / Between / InList 腿 / InSubquery 腿 / Exists / CASE；嵌套子查询体内不进入收集（仅 :525-543 / :551-560 多级相关拒绝检查，名单机制 `has_outer_refs_outside`（:593）不动）。收集覆盖面 = 子查询 WHERE 位置引用（投影位置引用现状不收集，维持）。
  - **四个收集调用点**：IN 臂（`subquery.rs:85`）、EXISTS 臂（:145）、标量绑定臂 `build_scalar_subquery_binding`（:221）均在 build_query 后 restore 后调用；**投影位标量臂（`query.rs:1478`）在 build 前调用**（:1475-1478，注释「Detect correlated parameters before building the plan」）——本 Cycle 移至 build 后（快照可得性要求，见 Guidance ⑤）。
  - **作用域快照可得性**：四臂在 `restore_binding_scopes(saved_scopes_len)` 前，内层段名 = `binding_scopes[saved_scopes_len..].qualifier`、外层段名 = `binding_scopes[..saved_scopes_len].qualifier`；段 qualifier 均小写登记（基表臂 `query.rs:406-417`、派生臂 :470-473、JOIN 右臂 :544-546、CTE/视图 effective）。
  - **执行期注入**：`CorrelatedParam { outer_table, outer_column, param_name }`（`src/executor/plan.rs:658`，`param_name` 匹配 `ParameterExpression::param_name`——两侧均小写）；四执行器 `extract_param_values`（`semi_join.rs:155` / `anti_join.rs:157` / `subquery_eval.rs:149` / `scalar_subquery_filter.rs:89`）按限定名自外层行取值注入。既有相关子查询（外层未别名 + 限定引用，差分 Q5-04/Q5-07/Q6-06 形态）端到端工作——碰撞形状修复后即此形态，注入机制无需改动。
  - **探针基线**（父 Cycle Plan Review Finding 1，`tests/zz_review_probe.rs` 已删，SQL 与期望值由本契约「新增测试规格」承载）：碰撞自相等（`u.v = users.v`）RTsql 现状 `{}` / 旧代码 `{1,2,3}` / SQLite `{1,2,3}`；`+1` 变体现状与旧均 `{}` / SQLite `{1,2}`；非碰撞宽松形状（内层 `orders o` + 书写 `orders.m` + 外层无 orders）新旧一致空集；无别名派生显式 `derived.k` 两侧同款执行期错误 `Execution error: correlated parameter column not found in outer row: k`。
  - **差分网落点**：`tests/sql_differential_test.rs` Q5（:514，8 条）/ Q6（:532，8 条）硬编码清单，`diff_group` 按组循环；种子 `users(id INTEGER PRIMARY KEY, name TEXT, age INT, score FLOAT, active BOOL, joined TEXT)`；既有 Q5-03（外层别名 + 内层未别名）与 Q5-04/Q5-07（标准相关）均非碰撞形状——碰撞形状在网内缺席，即现状 GREEN 与回归并存的原因。
- Code and Critical Path: `src/parser/planner/binding.rs`（`is_outer_reference` 谓词单源 + 段名命中辅助）、`expression.rs::qualified_column_expression`（条件改用谓词，语义不变）、`subquery.rs`（`extract_correlated_params`/`collect_outer_column_refs` 增快照参 + 谓词 + 三臂快照捕获）、`query.rs`（投影臂收集后移 + 快照捕获）、`tests/subquery_alias_binding_test.rs`（碰撞回归函数）、`tests/sql_differential_test.rs`（Q5/Q6 追加）。消费链：分类（live 作用域）与收集（段名快照）→ 同一谓词 → SemiJoin/AntiJoin/SubqueryEval/ScalarSubqueryFilter 注入。

**Implementation Guidance**

建议顺序 ①→⑥（①② 纯基建零行为变化）：

1. **谓词单源**（`binding.rs`）：`pub(crate) fn is_outer_reference(inner_hit: bool, outer_hit: bool, in_declared_list: bool) -> bool { !in_declared_list || (!inner_hit && outer_hit) }`。真值表六格核对（inner_hit × outer_hit × in_list）：与分类点现条件 `!inner_hit && (outer_hit || !in_list)` 仅在「名单外 ∧ 内层命中」格（无别名派生 `derived` 病理格）不同——该格两侧可观察行为均为收集侧执行期错误（探针基线见证），语义中立。
2. **分类点**（`expression.rs:269-281`）：条件改用 `is_outer_reference(内层段命中, 外层段命中, 名单命中)`——判定语义不变（分类已正确产出碰撞参数），仅公式单源化；注释同步（碰撞形状 → 外层相关参数〔2026-10-20 裁定〕；「结论逐字节不变」更正为「碰撞形状外逐字节不变」）。
3. **收集点**（`subquery.rs`）：`extract_correlated_params` / `collect_outer_column_refs` 增参 `inner_scope: &[String], outer_scope: &[String]`（段名小写快照）；CompoundIdentifier 臂谓词改 `is_outer_reference(inner_scope 命中, outer_scope 命中, 名单命中)`（命中 = `any(eq_ignore_ascii_case)`）。多级相关拒绝与 `has_outer_refs_outside` 不动。
4. **三臂快照捕获**（`subquery.rs` IN :48 / EXISTS :116 / 标量 :200 三处）：`restore_binding_scopes` 调用前捕获 `inner_scope` / `outer_scope` 两个 `Vec<String>`，随 `inner_tables` 一并传入 extract；错误路径不收集、不需快照。
5. **投影臂收集后移**（`query.rs:1475-1505`）：`extract_correlated_params` 调用自 build 前移至 restore 后（与其余三臂同款，快照在其前捕获）；`inner_table_names = Some(...)` 与 `extract_subquery_table_names` 仍在 build 前置位。**已接受边界**：「构建失败且预收集本应先行拒绝（多级相关）」的双无效输入错误优先级变化（构建错误先行）——既有测试面零变化由全量 Gate 见证。
6. RED → 修复 → GREEN → 定向套件 → 差分网 → 全量（M19 分批）。

**Task Contracts**

### T02-C1(replan): 碰撞形状 SQLite 语义 + 分类/收集判定单源化

- Requirement/Scenario: `sql-semantic-binding` R1 Scenario「关联子查询参数分类消费绑定结果」+ Scenario「别名遮蔽基表名的外层同名解析（碰撞形状）」（全部）；R2 全部 2 场景（含例外）
- Depends on: Iteration 000（差分网 GREEN 保护基线）
- Targets: `src/parser/planner/binding.rs`、`src/parser/planner/expression.rs::qualified_column_expression`、`src/parser/planner/subquery.rs`（extract/collect + 三臂）、`src/parser/planner/query.rs`（投影臂）、`tests/subquery_alias_binding_test.rs`（新增函数）、`tests/sql_differential_test.rs`（Q5/Q6 追加）
- Current behavior: 碰撞形状分类产出 `ParameterExpression` 但收集侧名单判定不收录 → 未绑定参数静默求值 NULL（EXISTS 自相等返回空集、投影位标量返回 NULL）；投影位标量臂收集先于构建
- Required behavior: 碰撞形状四子查询面解析为外层相关参数、收集收录、执行期注入求值（EXISTS 自相等 `[1,2,3]`、`+10` 形 `[1,2]`、投影位标量每行自身值，夹具见「新增测试规格」）；分类与收集经同一 `is_outer_reference` 谓词；其余输入与父 Cycle 基线逐字节一致（A1 宽松内层解析、`derived.k` 错误面、白名单跳过行为）
- Preserve: 父 Cycle 交付的绑定作用域表基建（五登记点 / 六处快照恢复 / `table_not_null` 通道）原样；PhysicalPlan 节点词表与计划形状（`explain_test` 零校准）；执行器 / 存储 / 网络零改动；既有 2045 基线零回归；差分网白名单 6 条目与跳过行为冻结
- Forbidden: C2 布局面；执行器文件；`has_outer_refs_outside` 多级拒绝机制改动；既有测试断言修改（仅允许本契约点名的追加：一个回归函数 + 三条差分查询）；白名单条目增删；错误消息文本校准（投影臂双无效输入错误优先级边界除外，Guidance ⑤）
- First test: RED——`subquery_alias_binding_test` 新增碰撞函数（现状失败：断言① 空集 ≠ `[1,2,3]`、断言③ NULL 非整型）+ 差分网 Q5-09 / Q5-10 / Q6-09（现状 RTsql 空集/NULL vs SQLite 非空，差分失败）；先观察 RED 再修复
- GREEN condition: 新增回归函数与三条差分查询全绿；六定向套件 + `self_join_test` / `join_output_resolution_test` 全绿；`cargo test --test sql_differential_test` 全绿（白名单条目数不变，结构自检见证）；全量 `cargo test`（M19 分批一次）零失败零校准
- Verification: ① `cargo test --test subquery_alias_binding_test`（23+，含新函数）；② `cargo test --test sql_differential_test`（13 tests，含新增 3 查询）；③ 六定向套件 + `self_join_test` / `join_output_resolution_test`；④ 全量分批一次 rc=0；⑤ `explain_test` 计划形状零校准（全量内见证）
- Stop when: 修复后碰撞形状仍与 SQLite 分歧（注入机制另有缺口 → 返回 Plan）；谓词单源化迫使 `&self` → `&mut` 签名连锁或执行器 / PhysicalPlan 改动；A1 宽松形状或 `derived.k` 错误面发生变化（超出裁定范围 → 报告不扩面）

**新增测试规格**

`tests/subquery_alias_binding_test.rs` 新增测试函数 `collision_shape_alias_shadowing_resolves_outer`（文件既有 helper：`open_db` / `exec` / `exec_ok` / `rows` / `int_ids`）：

- 夹具：`users(id INTEGER PRIMARY KEY, v INT)` 三行 (1,10)/(2,20)/(3,30)；`orders(k INT, m INT)` 一行 (2,5)。
- 断言①（EXISTS 自相等）：`SELECT id FROM users WHERE EXISTS (SELECT 1 FROM users u WHERE u.v = users.v)` → `int_ids` = `[1,2,3]`。
- 断言②（EXISTS +10）：`SELECT id FROM users WHERE EXISTS (SELECT 1 FROM users u WHERE u.v = users.v + 10)` → `[1,2]`。
- 断言③（投影位标量碰撞）：`SELECT id, (SELECT MAX(v) FROM users u WHERE u.id = users.id) FROM users` → `(1,10)/(2,20)/(3,30)`。
- 断言④（A1 控制组）：`SELECT id FROM users WHERE EXISTS (SELECT 1 FROM orders o WHERE o.k = orders.m)` → 空集（宽松内层解析维持）。

`tests/sql_differential_test.rs` 追加（清单内注释标注 2026-10-20 replan 碰撞形状）：

- Q5-09：`SELECT name FROM users WHERE age = (SELECT MAX(age) FROM users u WHERE u.id <= users.id)`
- Q5-10：`SELECT id, (SELECT MAX(age) FROM users u WHERE u.id <= users.id) FROM users WHERE id <= 20`
- Q6-09：`SELECT id FROM users WHERE EXISTS (SELECT 1 FROM users u WHERE u.age = users.age)`

**Invariants**

- 碰撞形状语义 = 外层相关参数（对齐 SQLite，2026-10-20 用户裁定）；A1 宽松形状（无外层同名）= 内层解析维持。
- 分类与收集判定单源：`is_outer_reference` 唯一公式，两消费点输入同源（live 作用域 / 同一作用域表的段名快照）。
- 既有 2045 基线零失败；错误面（类型、文本、SQLSTATE）除 Guidance ⑤ 点名边界外零变化。
- 固定种子差分网确定性维持；白名单条目集与跳过行为冻结。
- 加性承载（D5）维持：PlanBuilder 公共 API 仅加法（extract/collect 为 `pub(crate)` 内部签名演进，不改公共面）。

**Non-goals**

碰撞形状外的语义演进（基名全遮蔽——A1 宽松形状改报错）；C2 布局统一（Iteration 002）；③⑤ 拒绝面开放；①②④ 执行器层缺陷；收集面扩展到 WHERE 外位置（投影位参数现状维持）；`derived.k` 病理形状行为变化；`has_outer_refs_outside` 多级拒绝机制。

**Acceptance**

- R1/S2 + 碰撞场景（碰撞形状外层解析并注入）：四子查询面碰撞形状解析为外层相关参数并正确求值——回归函数断言①-③ + 差分 Q5-09 / Q5-10 / Q6-09 双引擎一致见证。映射：Task Contract / Guidance ③-⑤。
- A1 边界维持：断言④（宽松内层解析）+ 既有 alias 套件全绿见证。映射：回归函数 + 既有矩阵。
- R2/S1-S2（零回归 + 错误面保持）：全量零失败、错误断言零校准（投影臂双无效输入边界为例外并记录）。映射：全量 Gate。
- 判定单源（结构）：`is_outer_reference` 唯一公式、分类与收集两消费点（Review 代码核对）。映射：`binding.rs` + 两消费点。

**Verification**

| Scenario | 判定 | 通过依据 |
|---|---|---|
| R1 碰撞场景（四面） | 回归函数断言①-③ + 差分 Q5-09/Q5-10/Q6-09 | 全绿且双引擎一致 |
| A1 宽松维持 | 断言④ + 既有 alias 套件 | 空集 / 全绿 |
| R2 全量零回归 | 全量 `cargo test`（M19 分批一次） | rc=0、零校准 |
| R2 错误面 | 既有错误断言零校准 + `explain_test` | 零校准 |
| 判定单源 | 代码结构（Review 核对） | 唯一公式两消费点 |

**Gate 2 Readiness**

PASS——RTM 无 Missing/Simplified（R1 新场景与 R2 例外已入矩阵）；设计闭合（谓词真值表六格核对、四收集调用点与快照可得性、注入机制既有见证、投影臂后移边界已定位 file:line）；契约含完整执行边界与停止条件；Persisted Evidence `none`（验证结果 Act Response 承载，全部可低成本重跑）；用户已裁定语义方向并指令实施（原话见 Status）。

**Persisted Evidence**

- Mode: none

Act Response 承载定向与全量命令及结果；无不可低成本重跑项。

**Risks and Notes**

- R1（条件性）：投影臂收集后移的双无效输入错误优先级变化（Guidance ⑤）——既有测试面零变化由全量 Gate 见证；未测试面为裁定接受的显式边界。
- R2（非实质）：快照每臂 clone 两个 `Vec<String>`——planner 规模无感知；Act 可改迭代器借用等价局部实现。
- R3（边界记录）：A1 宽松形状与 SQLite（`no such column` 报错）仍相悖——裁定范围为碰撞形状；如需全遮蔽语义另行裁定。
- R4（条件性）：修复后若碰撞形状差分仍分歧，指向注入机制另有缺口（Stop when 返回 Plan），不擅自扩面。

---

## Act Response

- Status: reported

### 实际改动

先行测试（回归函数 + 三条差分查询）由用户预置于工作区，本 Act 观察其 RED 后实施修复：

- `src/parser/planner/binding.rs`：新增 `is_outer_reference(inner_hit, outer_hit, in_declared_list)` 谓词（判定单源；文档含与旧分类条件的真值表差异格说明——「名单外 ∧ 内层命中」格语义中立）。
- `src/parser/planner/expression.rs::qualified_column_expression`：分类条件改经谓词；注释同步碰撞裁定（2026-10-20，别名遮蔽基名）与「碰撞形状外逐字节不变」更正。
- `src/parser/planner/subquery.rs`：`extract_correlated_params` / `collect_outer_column_refs` 增 `inner_scope` / `outer_scope` 段名快照参（13 处递归点同步穿线）；CompoundIdentifier 臂改谓词判定；IN / EXISTS / 标量绑定三臂在 `restore_binding_scopes` 前捕获快照并传入；新增 `declared_from_qualifiers`（FROM 声明限定符提取），`extract_subquery_table_names` 委托同一实现。
- `src/parser/planner/query.rs`（投影位标量臂）：`extract_correlated_params` 自构建前移至构建后 restore 前（Guidance ⑤）；构建前按外层 FROM 声明限定符临时登记外层段（偏差 ①）；界恢复进臂前原值（偏差 ③）。
- 执行器、存储、`PhysicalPlan` 词表零改动；错误面零校准。

### Deviations from Plan

1. **Guidance ⑤ 快照可得性事实对投影臂不成立（Plan 调查遗漏）**：投影位循环在 `build_select` 中先于 `build_from_clause_with_projection`（query.rs 循环 ≈:1464 / FROM 构建 ≈:1576），投影臂运行时外层段不在作用域表（`saved_scopes_len` = 进臂前长度，`binding_scopes[..saved_scopes_len]` 不含外层 FROM 段）。仅「收集后移」无法达成四面目标——探针实证（临时 `tests/zz_act_probe.rs`，已删）：仅移收集后 `correlated_params=0`，`users.id` 分类落入内层单表回落臂解析为内层列（`ColumnExpression(id,0)`），过滤恒真、投影位标量每行返回全表 MAX。修复：投影臂构建内层前按 `declared_from_qualifiers(&select.from)` 临时登记外层段（空列清单，仅限定符参与命中判定），退出时与内层段一并截断。修复方向由 Cycle Objective（四子查询面）与既有作用域登记机制唯一确定；三个 WHERE 侧臂在 FROM 构建后运行、不受影响，未触及禁区（执行器/词表/错误面/A1 形状均未动）。已列 Plan Review 独立核对项。
2. **GREEN 条目「差分网 13 tests」为 Plan 笔误**：套件实为 10 个测试函数（与 Iteration 000 交付「差分网 10/0」一致）；新增 Q5-09/Q5-10/Q6-09 为既有测试函数内清单项，不增加测试函数数。
3. **界恢复卫生（本 Act 自检发现并修复）**：投影臂恢复 `binding_scope_boundary` 至进臂前原值而非登记后界，消除「界 > 表长」瞬时不一致（无可达消费者——`binding_scope_hit` 仅在四臂构建窗口内 `inner_table_names` 为 Some 时被读，纯卫生收口）。

### Gate 3 自检

- 已修复：偏差 3（界恢复原值）。
- 遗留 Minor：投影臂外层段临时登记对「外层无别名派生表」取不到 effective 名（`declared_qualifiers` 对无别名 Derived 返回空）——该形状书写限定符不可达（内部归一名），且 WHERE 侧臂外层快照取自 live 作用域不受影响；留 Iteration 002 登记完备化一并考虑。Iter002 若自作用域表派生 JOIN 布局，投影臂空列清单段需先解决（Split signal 相关，提请 Plan 注意）。
- 判定单源（结构）：`is_outer_reference` 唯一公式，分类（live 作用域）与收集（快照）两消费点均调用；白名单 6 条目与跳过行为冻结不变、无新分歧。
- Experience Candidates：None。

### 验证（全部真实运行）

- **RED（修复前）**：`cargo test --test subquery_alias_binding_test collision_shape_alias_shadowing_resolves_outer` → FAILED（断言① `left: []` ≠ `right: [1,2,3]`）；`cargo test --test sql_differential_test -- q5_scalar_subqueries q6_in_exists_subqueries` → FAILED（Q5-09 RTsql 0 行 vs SQLite 9 行；Q6-09 RTsql 0 行 vs SQLite 224 行）。
- **GREEN 定向**：`subquery_alias_binding_test` 23/0（含新回归函数五断言：EXISTS 自相等/`+10`、投影位标量、IN 面、A1 控制组）；`sql_differential_test` 10/0；`subquery_test` 43 / `scalar_subquery_comparison_test` 14 / `table_alias_resolution_test` 19 / `cte_test` 14 / `view_test` 19 / `self_join_test` 6 / `join_output_resolution_test` 19 / `explain_test` 10——全部 0 failed。
- **全量（M19 分批一次，最终代码）**：14 批 = `--lib` + 124 个集成目标按字典序每批 ≤10 目标，每批独立 `cargo test … -- --test-threads=1`、批间顺序执行、无无过滤全量调用；全部批次 rc=0，**合计 2046 passed / 0 failed / 2 ignored**（基线 2045 + 新增碰撞回归函数 1）。目标覆盖经 ran-vs-want 清单 diff 核对不重不漏。分批计数：

  | 批 | 目标（组） | passed/failed |
  |---|---|---|
  | 1 | lib | 407/0 |
  | 2 | aggregate…busy_timeout（10） | 148/0 |
  | 3 | cache_perf…concurrent（10） | 182/0 |
  | 4 | connection_limit…ddl_crash_recovery（10） | 147/0 |
  | 5 | default_declaration…explicit_tx（10） | 158/0 |
  | 6 | expression_e2e…insert_select（10） | 100/0 |
  | 7 | isolation_level…mvcc_commit（10） | 88/0 |
  | 8 | mvcc_record…parser（10） | 95/0 |
  | 9 | pg_integration…projection_expression（10） | 203/0 |
  | 10 | projection…runtime（10） | 64/0 |
  | 11 | scalar_function…sql_differential（10） | 139/0 |
  | 12 | storage…tx_statement（10） | 153/0 |
  | 13 | update_expression…wal_handle（10） | 122/0 |
  | 14 | wal_integration…write_type_conformance（5） | 40/0 |

  （125 个集成测试目标 = 第 2–13 批 12 批 × 10 + 第 14 批 5 个，另有 `--lib` 批；`sql_differential_test` / `structural_property_test` / `subquery_alias_binding_test` 分别落在第 11/12 批，均含新查询与回归函数。合计核对：集成 1639 + lib 407 = 2046。）

- 修复后计划形状见证（探针摘录，探针文件已删）：投影位碰撞 `SubqueryEval (…, correlated_params=1)`，内层谓词右臂 `ParameterExpression { param_name: "users.id" }`。

### Persisted Evidence

None required（计划 none；全部验证可低成本重跑；临时探针 `tests/zz_act_probe.rs` 诊断后已删除，关键输出摘录于上）。

### 未解决问题

- 差分白名单 5 条 suspected 缺陷（①③④⑤ 与 ②）处置仍待用户裁定（000 Plan Review Follow-up Decision 范围，本 Cycle 契约明确排除）。
- 偏差 1 的投影臂外层段登记完备性（无别名派生/CTE/视图段的列清单）留 Iteration 002。

---

## Plan Review

- Review Result: accepted
- Reviewer: openspec-plan（本会话）

**Findings**

1. **[核定——通过] 判定单源结构（Acceptance「判定单源」项）**：`is_outer_reference`（`binding.rs:40-42`）唯一公式；分类消费点（`expression.rs::qualified_column_expression`，live 作用域命中）与收集消费点（`subquery.rs::collect_outer_column_refs` CompoundIdentifier 臂 :542-549，段名快照命中）均调用同一谓词；13 处递归点快照穿线完整（BinaryOp×2 / UnaryOp / Nested / Between×3 / InList / InSubquery / CASE operand·conditions·results·else）。
2. **[核定——通过；偏差分类 Plan 遗漏] 偏差 1（投影臂外层段临时登记）**：事实基础独立确认——投影位循环（`query.rs:1458-1527`）先于 `build_from_clause_with_projection`（:1586 调用），Guidance ⑤ 的快照可得性前提对投影臂不成立属实；修复实现与 Objective（四子查询面）一致：`declared_from_qualifiers` 临时登记、别名与基名并列（`subquery.rs:479` `vec![alias_name, base]`——嵌套语境下基名段使收集侧外层命中成立）、空列清单（本 Iteration `ScopeEntry.columns` 仅登记不消费，无可达消费面）、退出两路截断至 `saved_scopes_len`；未触禁区（执行器/词表/错误面/A1 形状）。
3. **[核定——通过] 偏差 3（界恢复原值）**：投影臂成功/错误两路均 `binding_scope_boundary = saved_outer_boundary`（进臂前值）；`saved_outer_boundary ≤ saved_scopes_len ≤ len` 恒成立（顶层为 0、外层臂界 ≤ 进入长度），`binding_scope_hit` 切片无越界可能，瞬时不一致消除。
4. **[核定——通过；偏差分类 Plan 错误] 偏差 2（差分网函数数笔误）**：`#[tokio::test]`/`#[test]` 计数 = 10；Q5-09/Q5-10/Q6-09 为既有函数清单内追加（Q5 8→10、Q6 8→9），SQL 与契约「新增测试规格」逐字一致。
5. **[核定——通过] Preserve/Forbidden 面**：执行器/存储/网络零改动（git status：tracked 改动仅 planner 五文件 + `pipeline.rs` + `subquery_alias_binding_test.rs`）；`join.rs::condition_column_index` 原样；`has_outer_refs_outside`（`subquery.rs:733`）与多级相关拒绝臂不在 diff；PhysicalPlan 词表零变化；错误文本零校准（四 planner 文件 diff 无错误格式改动）；`pipeline.rs` +3 行为父 Cycle not_null 通道（加性）；白名单 6 条目（1 known + 5 suspected）逐条核对未变，结构自检测试在场。
6. **[Minor——新证据，非阻塞] 嵌套碰撞形状不可达（探针实证）**：`EXISTS (SELECT (SELECT …) FROM …)` 形（投影位标量子查询位于子查询上下文）被既有 `building_subquery` 抑制在计划期拒绝（`Unsupported statement type: UNSUPPORTED SELECT ITEM EXPRESSION`），基名引用与别名引用两变体同款——投影臂仅顶层可达，其临时登记的「外层段」恰为顶层自身 FROM，一级相关语义封闭，分类/收集脱节在该嵌套形状无可观察面；本 Cycle 未改变该拒绝面。探针 `tests/zz_review_probe2.rs` 用后即删，工作区已恢复 Act 现场（git status 核对）。
7. **[Minor——基线变化，遗留关闭] 父 Cycle fmt 漂移清零**：父 Cycle Gate 3 报告 `sql_differential_test.rs` fmt 漂移；当前 `cargo fmt -- --check` 全工作区 exit 0 零 diff——该遗留 Minor 关闭。
8. **[Minor——记录] 回归函数断言数超出冻结规格一条**：规格列 4 断言、实际 5——多出的 IN 面断言（`id IN (SELECT u.id FROM users u WHERE u.v = users.v)` → `[1,2,3]`）为用户预置测试内容（Act 如实报告「五断言」），补齐 Objective 四子查询面中规格测试未覆盖的 IN 面；覆盖超集、需求无缺口、未触 Forbidden（回归函数即契约点名的新增单元）。

**Acceptance Gaps**

None——四项 Acceptance 全部满足：

- R1 碰撞场景（四面）：回归函数断言①-③ + IN 面 + 差分 Q5-09/Q5-10/Q6-09 双引擎一致（采信 replan Act GREEN 定向验证：`subquery_alias_binding_test` 23/0、`sql_differential_test` 10/0——Act 报告后覆盖表面未变化，只读 git 核对一致）。
- A1 边界维持：断言④控制组空集 + 既有 alias 套件全绿。
- R2 全量零回归 + 错误面保持：全量 2046/0/2（= 基线 2045 + 新增回归函数 1，计数自洽；采信 Act M19 分批 14 批实测）；错误断言零校准由 diff 无错误文本改动独立见证。
- 判定单源（结构）：Finding 1 代码核对。

**Evidence**

- 完整 diff 独立阅读：六 tracked 文件 +398/−39（父 Cycle +115/−3 与 replan 增量叠加）与 `binding.rs` 全文；Act Response 声明（谓词、三臂快照、13 处穿线、投影臂三偏差、`declared_from_qualifiers` 委托）逐项与代码比对一致。
- 探针实测：`cargo test --test zz_review_probe2 -- --nocapture`（嵌套碰撞两变体，结论见 Finding 6；文件已删）。
- `cargo fmt -- --check` exit 0（Finding 7）。
- 采信 Act Response 验证结论（未失效）：RED→GREEN 定向、六定向套件 + `self_join_test`/`join_output_resolution_test`/`explain_test`、全量 2046/0/2——本 Review 全程只读 git 操作，临时探针删除后工作区与 Act 报告现场一致。

**Follow-up Decision**

接受：Acceptance 全部满足、无阻塞 finding；偏差 1 为 Plan 遗漏的必要修复且本 Review 独立核对通过（Guidance ⑤ 前提错误 + 修复方向由 Objective 唯一确定），偏差 2/3 如实记录无争议；Minor 四项均为记录性（Finding 6 不可达性实证、Finding 7 遗留关闭、Finding 8 覆盖超集）。

遗留移交：

- Act Gate 3 遗留 Minor「投影臂无别名派生外层段登记不完备」（`declared_qualifiers` 对无别名 Derived 返回空——书写限定符不可达，纯 C1 痕迹）随 Iteration 002 Non-goals 记录；「投影臂空列清单段 × 布局派生」在 Iteration 002 Plan Context 以构造入口窗口边界设计消解（临时段在窗口外）。
- 差分白名单 5 条 suspected 缺陷处置仍待用户裁定（000 Plan Review Follow-up Decision，不在本 change 任一 Cycle 范围）。

**Next Cycle**

None

**Next Iteration**

`iterations/002-join-layout-unification/000-initial.md`（T02-C2：JOIN 布局段源自作用域表统一与侧别判定切换；Plan Context `draft`——待用户批准后转 `ready`）
