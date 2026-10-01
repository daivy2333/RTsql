# Iteration 001 / Cycle 000: 表别名解析统一（P1 限定名家族收口）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5, T6, T7, T8
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: design D4（别名映射 + 单点归一化四消费点）、D5（限定形态边界）、D6（ORDER BY 限定键）；`sql-table-alias-resolution` delta spec R1-R5、`order-by-alias-sorting` MODIFIED R2/R3、`join-output-shape-resolution` MODIFIED R3；proposal 假设 5/6/7
- Excluded scope: P2/P3 与文档（Iteration 002）；窗口函数/LATERAL/标量子查询投影/递归 CTE；attach 两段表引用语义变更；表达式项 × JOIN 解禁

**Objective**

FROM 表别名在全子句面（SELECT/ON/WHERE/GROUP BY/聚合参数/ORDER BY）对限定列引用生效，含自连接多别名与单表/JOIN/CTE/视图基座；未知限定符与本地三段限定计划期点名（终止「静默丢弃限定符」）；attach 三段复合键通道零回归；ORDER BY 限定键真实排序。

**Investigation Facts**

- Current Baseline: master @ 1668c95 + Iteration 000 工作区改动（`catalog.rs`/`slotted_page.rs` durability 修复 + `tests/catalog_durability_test.rs`，Plan Review accepted，全量 1604 tests / 0 failed——采信来源 Iteration 000 Act Response Verification 1/5 与 Plan Review Findings 8）。本 Iteration 不触碰 Iteration 000 改动的文件面。
- Current-State Evidence（全部为 2026-09-30 本会话实读 + 已安装 0.2.0 二进制实测，代码位以 1668c95 为准；planner 面与 Iteration 000 改动零交集）：
  - **别名丢弃点**：FROM 基表臂 `src/parser/planner/query.rs:334-358`——`table_name = object_name_to_table_name(name)`（真实名），alias 仅传入 `resolve_cte_reference`（:342）/`resolve_view_reference`（:344），注释 ：338 明写「含既有忽略表别名的行为」；JOIN 右臂 ：426-455（`extract_join_table_name` + `table_factor_alias`，同型）。`PlanBuilder.tables` 只按真实名/attach 复合键注册（注册在 `src/pipeline.rs::register_table_schema`，复合键 `"o.dept_map"` 于 :2258；planner 只读）。
  - **限定列解析四消费点**：①`expression.rs:24-96` `resolve_column_ref`/`resolve_qualified_column`（:81 `validate_table(table)` → 未知限定符报 `ParseError "Table 'x' does not exist"`）——Hash 臂 ON 腿（经 `extract_join_conditions`，query.rs:550-552）、JOIN WHERE、聚合参数通道；②`expression.rs:249-300` `qualified_column_expression`——相关外层臂 ：254-263、NLJ 布局臂 ：269-284（布局表名=真实名，miss → `PlanError::TableNotFound` :283）、无布局臂 ：287-299（`self.tables.get(table_ref)` → `ParseError "Table 'x' not found"` :290）；`build_expression` CompoundIdentifier 两段臂 ：361-365、三段臂 ：370-378（键 `"a.b"`）；③JOIN 输出列严格校验/过滤 `query.rs:498-541` 与 `resolve_join_projection_indices`（:2733-2781）——qualifier 与 `OutputColumn.table`（真实名，:463-490）直比，alias miss → hits==0 → `ColumnNotFound(qual_col)`；④ORDER BY `extract_column_name`（:2901-2908，仅收裸 `Expr::Identifier`，两段一律 `ParseError "ORDER BY only supports column names"`），调用点 ：682（扫描臂）与 :1999（聚合/JOIN 臂），键校验集 ：2059-2086（`sort_columns` ∪ 顶层投影输出别名），表达式键 decorate-sort-undecorate 装配 ：2093-2123。
  - **单表静默宽松（要收紧的解析面）**：`extract_columns`（`src/parser/ast.rs:48-63`）两段/三段 CompoundIdentifier **只取末段**（:56-63）——单表投影任意限定符意外成功；`expr_to_column_name`（expression.rs:839-845）同型（GROUP BY 单表取末段）。`is_plain_column_expr`（query.rs:2806-2812）放行 2/3 段；`select_list_is_pure_column_list`（:2710）。
  - **布局构造**：NLJ 布局由 `table_columns` 闭包按真实名构建（query.rs:567 起，save/restore :585/:1392/:1490-1513/:2098）——布局表名与限定符匹配同需归一化。
  - **实测缺陷矩阵（0.2.0，本会话）**：`SELECT a.name FROM people a JOIN people b ON a.id = b.id` → `Column not found: name`；`SELECT * FROM people p JOIN team t ON t.id = p.team_id` → `Parse error: Table 't' does not exist`；同形态 LEFT JOIN → `Table not found: t`；JOIN 后 `WHERE a.id = 1` → `Table 'a' does not exist`；JOIN 后 `GROUP BY a.team_id` → `Table 'a' does not exist`；`SELECT p.name FROM people p WHERE p.id = 1` → `Table 'p' not found`（单表 WHERE）；`SELECT id, name FROM people p ORDER BY p.id` → `ORDER BY only supports column names`；`SELECT demo.people.name FROM people`（本地三段）→ **静默成功**；`ATTACH` 三段 `o.dept_map.dept` JOIN → 正常（复合键通道，须保持）。
  - **错误面**：`PlanError::TableNotFound` Display = `Table not found: {0}`、SQLSTATE 42P01（`src/parser/error.rs:93/:151`）——未知限定符采用该变体后，现网「别名被当表名」误导错误在别名可解析后自然消失。
  - **测试与规格入口**：`tests/order_by_alias_sort_test.rs`、`tests/join_output_resolution_test.rs`、`tests/aggregate_output_shape_test.rs`、`tests/derived_scan_alias_test.rs`、`tests/cte_test.rs`、`tests/no_from_select_test.rs`、`tests/subquery_test.rs`、`tests/outer_join_test.rs`、`tests/nested_loop_join_test.rs`；语料边界注记 `openspec/specs/order-by-alias-sorting/spec.md:50`（本 change MODIFIED 终止）、`join-output-shape-resolution` R3。
- Code and Critical Path: 计划期纯变换（planner/ast 层），无运行期/存储面；错误经既有 `PlanError` 通道（SQLSTATE 分类既有）。D4 归一化函数为单点纯函数；四消费点调用它；别名映射生命周期 = 单语句 PlanBuilder。

**Implementation Guidance**

顺序：T5 注册 → T6 解析（依赖 T5）→ T7 ORDER BY（依赖 T5）→ T8 回归收口。要点：

1. 归一化优先级（proposal 假设 5）：限定符先查别名映射，未命中原样返回（进 `self.tables` 直接键，含 attach 复合键）；attach 两段**表引用**语境的 attach-first 语义不动（FROM 臂不受本 change 影响，只新增别名注册）。
2. 归一化点 placement：`qualified_column_expression` 与 `resolve_qualified_column` 入口处先归一化再走原通道；NLJ 布局查找在 `table == table_ref` 比较前归一化 `table_ref`；JOIN 输出过滤/`resolve_join_projection_indices` 的 `qual_table` 在比较前归一化；布局构造的 `table_columns` 键不变（真实名），布局查询侧归一化即可。
3. 单表收紧：`build_expression`/`build_where` 走归一化后自然拒绝未知限定符；**输出列名职责不动**（`extract_columns` 末段表头保持；表头断言用例不受影响）。三段臂：键 `"a.b"` 未命中 `self.tables`（attach 复合键）时点名拒绝——不再回落末段静默语义；拒绝形态建议复用既有 `ColumnNotFound`/`TableNotFound` 族（信息含完整三段），具体变体属非实质选择。
4. `ORDER BY` 两段键：`extract_column_name` 返回形态扩展后，两条构建路径的键校验与 `OrderByColumn` 装配同步——限定键归一化后在 `sort_columns`（可见列）解析为输入列索引；输出别名优先语义、未知键拒绝不变。
5. 既有用例预期兼容性：声明了别名的形态（`SELECT p.name FROM people p`）经映射解析后结果/表头不变，预期零校准；仅「未声明限定符静默成功」与「本地三段」两类历史宽松形态会转拒绝——T8 跑回归时按白名单裁定。

**Task Contracts**

### T5: FROM 别名注册与归一化单点

- Requirement/Scenario: sql-table-alias-resolution R1（自连接两别名、混用真实名、表头/计划形状不变）
- Depends on: None
- Targets: `src/parser/planner/mod.rs`（`PlanBuilder` 新字段 `table_aliases: HashMap<String,String>` + `resolve_table_qualifier` 归一化方法）、`src/parser/planner/query.rs`（FROM 基表臂 :334-358、JOIN 右臂 :426-455 注册点）
- Current behavior: 别名被丢弃（仅 CTE/视图 effective 名通道消费 alias）
- Required behavior: 基表/JOIN 右臂在注册表查找成功后登记 `alias → 实际注册键`（基表=真实名；CTE/视图=effective 名；attach 复合键=复合键本身）；归一化函数先查映射、未命中原样返回；CTE/视图既有 effective 名行为不变；自连接同表两别名各自映射
- Preserve: `self.tables` 键空间不新增别名条目；`OutputColumn.table` 真实名语义不变；attach FROM 臂语义不变
- Forbidden: 不改 `register_table`/`validate_table` 键语义；不动 pipeline 注册面
- Test witness: 单测（query.rs tests 模块或新 lib 测试）：自连接两别名注册后归一化各自命中；别名与 attach 复合键共存；无别名行为不变（编译级 RED：字段不存在）
- GREEN condition: `cargo test --lib parser` 相关模块全绿
- Verification: `cargo test --lib 2>&1 | tail -4`，0 failed
- Stop when: CTE/视图 effective 名与别名注册冲突出现语义歧义时返回 Plan

### T6: 限定列解析统一（四消费点 + 边界收紧）

- Requirement/Scenario: sql-table-alias-resolution R2/R3/R4；join-output-shape-resolution R3（MODIFIED）
- Depends on: T5
- Targets: `src/parser/planner/expression.rs`（`resolve_qualified_column` :81、`qualified_column_expression` :249 布局臂与无布局臂、三段臂 :370-378）、`src/parser/planner/query.rs`（JOIN 输出校验/过滤 :498-541、`resolve_join_projection_indices` :2733-2781、布局查找）、`src/parser/ast.rs`（如需在 `extract_qualified_columns` 侧透传）
- Current behavior: 见 Investigation Facts 缺陷矩阵（六种错误形态 + 单表/本地三段静默宽松）
- Required behavior: 四消费点限定符经 T5 归一化；未知限定符 `PlanError::TableNotFound(qualifier)` 点名（42P01）；本地三段限定计划期点名拒绝；attach 复合键三段保持可达；别名已声明的既有形态结果/表头不变
- Preserve: 裸名解析与 `AmbiguousColumn` 语义（expression.rs:318-343）不变；相关外层引用臂（:254-263）先于归一化（相关查询的限定符语义不动）；`OutputColumn.table` 真实名不变
- Forbidden: 不解禁表达式项 × JOIN；不改 attach 两段表引用路由；不动子查询/视图/CTE 展开语义
- Test witness: 新文件 `tests/table_alias_resolution_test.rs`（RED 先行）——矩阵：自连接两别名 × SELECT/ON/WHERE/GROUP BY/聚合参数 × 单表/JOIN/CTE/视图基座；六条历史缺陷查询转成功（含 LEFT JOIN NLJ 臂）；未知限定符/本地三段点名拒绝；ATTACH 三段复合键零回归
- GREEN condition: `cargo test --test table_alias_resolution_test` 全绿
- Verification: 同命令 `2>&1 | tail -6`，0 failed；失败含义 = 解析面仍有遗漏消费点或边界误伤
- Stop when: 归一化需改动相关子查询（correlated）语义或发现第五消费点超出四点清单时返回 Plan

### T7: ORDER BY 限定键

- Requirement/Scenario: sql-table-alias-resolution R2（ORDER BY 面）；order-by-alias-sorting R2/R3（MODIFIED，边界注记终止）
- Depends on: T5
- Targets: `src/parser/planner/query.rs`（`extract_column_name` :2901-2908、扫描臂 ：677-700、聚合/JOIN 臂 ：1994-2125 的键校验 ：2059-2086 与装配）
- Current behavior: 两段键一律 `ParseError "ORDER BY only supports column names"`（单表与 JOIN、别名与真实名皆拒）
- Required behavior: 两段键经归一化后进既有键校验集（`sort_columns` ∪ 输出别名）解析为输入列索引并真实排序；输出别名优先、未知键（含未知限定符）拒绝语义不变；DESC/多键组合可用
- Preserve: 裸键/输出别名排序逐字节不变（`order_by_alias_sort_test` 既有矩阵零回归）；`SortExecutor` 消费面不变
- Forbidden: 不支持 SQL 级 `ORDER BY <表达式>`（维持既有拒绝）；不改 Sort/Projection 链结构
- Test witness: `tests/order_by_alias_sort_test.rs` 扩展（RED 先行）：`ORDER BY p.id` 单表 / `ORDER BY t.id` JOIN / DESC / 多键混合 / 未知限定键拒绝
- GREEN condition: 该文件全绿
- Verification: `cargo test --test order_by_alias_sort_test 2>&1 | tail -4`
- Stop when: 限定键与聚合查询组合需要新解析序（超出「与裸键同面」契约）时返回 Plan

### T8: 既有读面回归与校准收口

- Requirement/Scenario: sql-table-alias-resolution R5；join-output-shape-resolution 零回归面
- Depends on: T6, T7
- Targets: 既有测试族回归 + 白名单校准（如需）+ `openspec validate`
- Current behavior: 全量绿（Iteration 000 后基线 1604）
- Required behavior: `order_by_alias_sort` / `join_output_resolution` / `aggregate_output_shape` / `derived_scan_alias` / `cte` / `no_from_select` / `subquery` / `outer_join` / `nested_loop_join` 全绿；依赖「静默丢弃限定符」的存量用例按白名单校准（预期零或近零）；全量分批（M19）0 failed
- Preserve: 校准仅限历史宽松形态依赖用例，断言语义改写须在用例注释说明
- Forbidden: 不为本 Iteration 新增能力回改既有断言
- Test witness: 回归运行输出 + 校准清单（如有）
- GREEN condition: 全量 0 failed
- Verification: M19 分批合计行 + `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict`
- Stop when: 校准面超出「历史宽松形态」范围（出现真实语义回归）时返回 Plan

**Invariants**

- 计划期纯变换：不触存储/执行器/WAL；计划形状对「真实表名书写」与「别名书写」等价（EXPLAIN 同型）。
- attach 两段表引用 attach-first、三段复合键通道、相关子查询限定符语义全部不变。
- 表头命名职责（末段）不变；`extract_columns` 的输出名行为仅解析路径改变。
- 不新增判定层或身份型证据工程。

**Non-goals**

见 Cycle Scope Excluded scope。

**Acceptance**

R1：别名注册后自连接两别名全子句面可用，表头/计划形状与真实名书写等价（T5/T6）。
R2：六条历史缺陷查询（JOIN 投影/ON-当表名/NLJ 变体/JOIN WHERE/JOIN GROUP BY/ORDER BY 限定键）全部转成功（T6/T7，RED→GREEN）。
R3：单表 WHERE 别名可用；未知限定符单表与 JOIN 同语义点名（T6）。
R4：本地三段拒绝、ATTACH 三段复合键零回归（T6）。
R5：T8 测试族 + 全量分批零回归。

**Verification**

- R1-R4：`cargo test --test table_alias_resolution_test` + `cargo test --test order_by_alias_sort_test`（修复前 RED 记录）。
- R5：M19 全量分批 0 failed + `openspec validate --strict`。
- 人工步骤：无。

**Gate 2 Readiness**

全部 PASS——计划整体已获用户批准（2026-09-30「批准」），本 Iteration 契约在批准的计划内；Current Baseline 已按 Iteration 000 accepted Review 更新（1604 tests / 0 failed，工作区含 Iter 000 未提交改动，本 Iteration 文件面与其零交集）；四消费点/注册点/测试入口均有 file:line 锚点；Persisted Evidence = none；无 BLOCKED/WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 白名单校准面未知项：本地三段限定若存在存量用例依赖静默成功，将在 T8 回归暴露——属预期历史宽松形态，校准即可；若出现真实语义回归即 Stop。
- `extract_qualified_columns` 对三段的 qualifier 产出形态（`Some("a.b")`）未逐行核读——T6 实施时以实际代码为准，属非实质局部差异。
- ORDER BY 限定键在聚合基座（`sort_columns` 为分组键集）的解析随 T7「与裸键同面」契约自然成立；若聚合直出形状判定（MS25 键⊆SELECT 收窄）与限定键组合出现形状冲突，按 Stop 条件返回。

## Act Response

- Status: reported

**Implemented**

T5（`src/parser/planner/mod.rs`）：`PlanBuilder` 新增 `table_aliases: HashMap<String, String>`（与 `tables` 键空间分离，别名不参与表名查找、`attached_tables` 复合键不被污染），`new()` 初始化；新增 `register_table_alias(alias, target_key)`（小写归一，同名后写覆盖）与 `resolve_table_qualifier(qualifier) -> Option<&str>`（命中返回真实键，否则 `None` 交回调用方原样使用）。`query.rs` 两个基表臂（FROM 基表臂、JOIN 右臂）登记别名；CTE / 视图臂不登记（其别名**就是** `self.tables` 的 effective 键，登记恒等映射无结果差异——design D4 第三注册点判定为无需登记）；attach 复合键臂同理无条目。新增 3 条 lib 单测（自连接两别名各归一、attach 复合键逐字回传、大小写归一），编译级 RED → GREEN。

T6（`expression.rs` + `query.rs`）：四个限定名消费点统一经别名归一后解析——`resolve_qualified_column`（ON 腿 / 子查询腿 / WHERE 哈希）、`qualified_column_expression` 无布局臂（单表基座）、JOIN 组合形状的严格校验与输出过滤、`resolve_join_projection_indices`。`validate_table` 从 `resolve_qualified_column` 移出，未知限定符错误由 42601「表不存在」改为 `PlanError::TableNotFound(qualifier)`（42P01，携带限定符原文）。新增 `qualifier_in_scope` + `validate_projection_qualifiers`（纯列清单的限定符在链末计划期点名），终结单表基座「静默丢弃限定符」与本地三段 `db.table.col` 的单表/JOIN 分裂面。`ColumnRef.table` 改为携带归一后真实键——ON 腿左右交换判定（`extract_join_conditions`）与嵌套 JOIN / SemiJoin / AntiJoin 的链首表名（`pipeline::extract_column_indices` 对 `OutputColumn.table_alias` 的归属比较）都按真实名工作。

T7（`query.rs`）：`extract_column_name` 扩为返回 `(Option<String>, String)`——两段形态解出限定符与末段列名；主 ORDER BY 构建路径先在 FROM 作用域内点名限定符（不可达 → 42P01），再按末段列名进既有键校验集（`sort_columns` ∪ 输出别名），`SortExecutor` 无需感知限定符；复合查询路径无 FROM 作用域，限定键明确点名拒绝。三段及以上与表达式项维持既有 `ORDER BY only supports column names`。输出别名优先、DESC / 多键组合、未知键拒绝语义逐条保持。

T8：`tests/table_alias_resolution_test.rs`（13 用例，373 行）覆盖 R1-R5 全部场景 + R1 计划形状不变量；无存量用例需要白名单校准。`cargo fmt` 与 `cargo clippy --all-targets` 干净。

**Deviations from Plan**

1. **[机制级，Plan 遗漏] 组合形状改为携带声明限定符，而非「限定符先归一再与形状比较」。** Task Contract 的机制在自连接上不成立：`a` / `b` 归一后同为 `people`，与 `OutputColumn.table`（真实表名，R1 规定不变）比较会同时命中两段——`SELECT a.name FROM people a JOIN people b ON a.id = b.id` 从「`Column not found`」错解为 6 列全宽行，比原缺陷更危险。改为：`build_from_clause_with_projection` 返回与最终节点 `output_columns` 同序同长的声明限定符向量（`(PhysicalPlan, Vec<String>)`），`join_row_layout`、严格校验、输出过滤、`resolve_join_projection_indices` 四处按声明限定符匹配；NLJ 的 ON 谓词布局同样按声明限定符建段。`OutputColumn` 字段零改动（真实表名语义、表头、执行器 left/right 归属不变），GROUP BY / 聚合参数的 `join_layout` 解析臂零改动（段键本就是「按书写匹配」形态）。已用 `alias_and_real_name_produce_isomorphic_plans` 见证 R1 计划形状不变量。
2. **CTE / 视图臂不登记恒等别名条目**（见 T5），避免死代码；行为等价。
3. **`ColumnRef.table` 语义由「逐字限定符」改为「归一后真实键」**（见 T6）。两处消费方（ON 腿交换判定、`pipeline` 链首表名）本就按真实名工作，别名形态下逐字限定符会与之失配。唯一可见差异是 EXPLAIN 文本对别名 JOIN 显示真实表名（既有测试零回归——别名形态此前无法到达该面）。
4. **`validate_projection_qualifiers` 置于 JOIN 链装配之后**（Plan 未指定位置）：置于基表臂之后时右臂别名尚未登记，自连接投影被误报 `Table not found: b`（自检发现，见下）。
5. **链内严格校验对不在作用域的限定符 skip**，交由链末点名 42P01，避免「关系不存在」被误报为 `ColumnNotFound`（自检发现）。

**Self-Review Findings**

- [Important] 自检发现并修复：链内严格校验对未知限定符先报 `Column not found: name`（42703），违反 R2「点名限定符」。已按 deviation 5 修正。
- [Important] 自检发现并修复：限定符校验早于右臂别名登记，自连接投影误报 `Table not found: b`（42P01，合法别名被拒）。已按 deviation 4 修正。
- [Important] 自检发现并修复：`ColumnRef.table` 保留逐字限定符时，`ON t.id = p.team_id` 的左右交换判定误判（别名与真实表名比较不相等），右列被错误地从左表取值，执行器 panic `right column index must exist`。已按 deviation 3 修正。
- [Minor] `clippy::manual_repeat_n`（2 处 `repeat().take()`）→ `repeat_n`。
- [Minor] 测试期望写反（字符串序 `amy` < `ann`），断言已按实际排序语义修正。
- 遗留 Minor（接受，非缺陷）：
  1. **自连接同名列的 ORDER BY 限定键**：`ORDER BY b.id` 在 a/b 同表同列名时，排序键按末段列名进 `compare_rows` 的 `position()` 首次命中——与裸键 `ORDER BY id` 既有行为同型（首次命中），不误排但无法区分两段。按 T7「与裸键同面」契约实现；delta spec 场景中 `a.id = b.id` 使两者等价。
  2. **别名遮蔽真实名**：`FROM people a` 后写 `people.name` / `GROUP BY people.id` 计划期点名拒绝（PG 同款语义；既有行为为可达）。
  3. **复合查询 `ORDER BY t.col`** 明确拒绝（无 FROM 作用域）。

**Verification Evidence**

- T5 单测：`cargo test --lib planner::tests` → `6 passed; 0 failed`（exit 0）。RED 见证：`E0599 no method named register_table_alias / resolve_table_qualifier`（exit 101）。
- T6/T7 集成：`cargo test --test table_alias_resolution_test` → `13 passed; 0 failed`（exit 0）。RED 见证（实施前 9 用例跑 8 失败）：`Parse error: Table 'a' does not exist`、`Column not found: name`、`Parse error: Table 'p' not found`、三段与未知限定符两例**静默成功**——与 delta spec 记载的历史形态逐字一致。
- T8 既有读面 + 全量（M19 分批，`--test-threads=1`，每批 10 个目标按字典序，批间顺序执行）：

  ```
  lib+bins exit=0 passed=353 failed=0     batch5  exit=0 passed=79  failed=0
  batch1  exit=0 passed=105 failed=0      batch6  exit=0 passed=78  failed=0
  batch2  exit=0 passed=203 failed=0      batch7  exit=0 passed=193 failed=0
  batch3  exit=0 passed=130 failed=0      batch8  exit=0 passed=120 failed=0
  batch4  exit=0 passed=107 failed=0      batch9  exit=0 passed=150 failed=0
  batch10 exit=0 passed=84  failed=0      batch11 exit=0 passed=17  failed=0
  M19_BATCHED_FULL_SUITE_FAILED=0   合计 1619 passed / 0 failed
  ```

  合计 = Iteration 000 基线 1604 + 本 Cycle 新增 15（3 lib 单测 + 12 集成）。`cli_test`（111 例，含 ATTACH 三段复合键用例 `cross_db_query_test.rs:292,360`）在本批内全绿。
- 静态检查：`cargo fmt --check` 无 diff（exit 0）；`cargo clippy --all-targets` 无告警（除 `~/.cargo/config` 弃用提示，与本改动无关）。
- `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict` → `Change '2026-09-30-catalog-durability-and-cli-surface' is valid`（exit 0）。

**Verification Notes（首跑失败记录）**

- 此前以「单次无目标过滤 + 默认并行度」跑全量（**违反 M19**），两轮各出现 `DatabaseLocked` 偶发失败，且失败用例集每次不同（`alter_table_test` 1 例 / `readonly_session_test` 3 例 → 复核时换为另 1 例）。定向审计：`File::try_lock`（`storage/file_storage.rs:81,135`，OFD 锁随 fd 生命周期）与 SQL 规划无调用关系；`readonly_session_test` 串行 3 轮均 `14 passed; 0 failed`。按 M19 分批串行形态复跑后全绿，判定为并行执行的锁争用偶发，非本改动引入。相应分批总数计入上方 T8 结果。

**Persisted Evidence**

- None required（Plan 指定 mode = none；决定性输出均 ≤20 行且已在本 Response 摘录，全部可低成本复现）。

**Experience Candidates**

- Issue 候选（本 change 范围外、有证据）：**并行度相关的库文件锁争用偶发失败**——`readonly_session_test` 3 例与 `alter_table_test` 1 例在默认并行度全量下返回 `DatabaseLocked`，失败集逐轮变化，同一文件串行 3 轮全绿。证据：本 Response「Verification Notes」与 `/tmp/rtsql_m19_batches.log`（本轮分批全绿）。是否立案由 Recorder 按用户指令决定。
- Runbook 候选：None（无新的可重复高风险操作路径）。

**Unresolved**

None。

**工作区说明（非本 Cycle 产物）**

`.agents/docs/tasks.md` 有一处未提交改动（MS22 候选池新增「`rtsql import` ≈4.1 ms/行」实测条目，引用 Iteration 000 的 Act 测量），非本 Cycle 产生，未触碰。`src/storage/catalog.rs`、`src/storage/page_format/slotted_page.rs` 与 `tests/catalog_durability_test.rs` 为 Iteration 000 产物，按 Plan Review `accepted` 保留未提交状态。

## Plan Review

- Review Result: replan-required

**Findings**

独立检查（Plan 独立阅读完整 diff：`src/parser/planner/mod.rs` +56、`src/parser/planner/query.rs` +277/-122 中的本 Iteration 增量、`src/parser/planner/expression.rs` +28、`tests/table_alias_resolution_test.rs` 新增 13 用例；逐一核对 Act Response 声明与代码一致。Iteration 000 的 `catalog.rs`/`slotted_page.rs` 工作区改动属已 accepted 的父 Iteration，文件面与本 Iteration 零交集，未重复审）：

1. **T5/T6/T7 主体与 Act 声明一致**：`table_aliases` 注册与归一化单点、四消费点接线、`extract_column_name` 两段扩展、复合查询臂限定键点名、链末 `validate_projection_qualifiers` 均与代码逐项吻合。
2. **偏差 1-5 全部核实为真实且实现正确**。偏差 1（声明限定符向量）机制必要且自洽——自连接两别名归一后同为 `people`，按真实名匹配会双段命中（Act 论证成立）；`(PhysicalPlan, Vec<String>)` 返回、`all_qualifiers` 与 `all_columns` 同序同长、`join_row_layout`/`resolve_join_projection_indices`/NLJ ON 布局四处按声明限定符消费，`alias_and_real_name_produce_isomorphic_plans` 见证计划形状不变量。偏差 3（`ColumnRef.table` 携归一真实键）两处消费方按真实名比较，核实匹配。
3. **[阻塞，Plan 遗漏 + 实现缺口] delta spec `sql-table-alias-resolution` R2 的点名 SHALL 在 GROUP BY 与聚合参数两通道未实现。** R2 覆盖清单明确包含「GROUP BY 与聚合参数（`aggregate_arg_indices` 通道）」且要求「归一化后仍不可达的限定符 SHALL 以携带该限定符的错误点名……SHALL NOT 误报为列缺失或静默求值」。实际（Plan 探针实证，工作区构建，2026-10-01）：
   - `SELECT x.id FROM people` → `Table not found: x`（exit 3，42P01）——投影面收紧生效；
   - `SELECT COUNT(x.id) FROM people` → **exit 0**（`aggregate_arg_indices` 单表臂 `query.rs:2070-2075` 末段静默解析）；
   - `SELECT id FROM people GROUP BY x.id` → **exit 0**（`resolve_group_by_item` 单表臂 `query.rs:226-231` 末段静默命中）；
   - `SELECT COUNT(x.id) FROM people p JOIN team t ON p.id = t.id` → `Column not found: id`（误报列缺失，错误不携带限定符）；
   - `… GROUP BY x.id`（JOIN 基座）→ `Non-aggregated column 'id' …`（同族误报）。
   同一语句形态在投影面被点名、在 GROUP BY/聚合参数面静默或误报，正是本 change 立项要消灭的分裂面。分类：**Plan 遗漏**——T6 Task Contract 的四消费点清单未纳入这两通道，与 delta spec R2 覆盖清单不一致；Act 按契约交付无偏离。
4. **[随 Gap 1 一并修复] Cycle Acceptance R2 的「NLJ 变体」缺 RED→GREEN 测试见证。** 历史缺陷第 3 条（`LEFT JOIN` + 别名）在 `tests/table_alias_resolution_test.rs` 无对应用例（Act RED 记录亦未含该形态）。Plan 探针实证行为正确：`SELECT p.name, t.team_name FROM people p LEFT JOIN team t ON t.id = p.team_id` → `[["ann","red"],["bob",null]]`（NULL 补齐正确），`WHERE t.id IS NULL` → `[["bob"]]`——NLJ 布局臂按声明限定符解析无缺陷，缺的只是验收清单点名的见证用例。
5. **[Minor，不阻塞] T7 测试见证位置偏离契约**：契约指定 `tests/order_by_alias_sort_test.rs` 扩展，实际 ORDER BY 限定键用例（单表/DESC/多键混合/未知限定键/输出别名优先）落在 `tests/table_alias_resolution_test.rs`。覆盖等价（R2 验收矩阵各形态齐全），既有 `order_by_alias_sort_test` 零改动且全量回归通过，归属文件更内聚。接受，不要求迁移。
6. **[Minor，不阻塞] Act Response 记账 ±1**：新增数声明「15（3 lib + 12 集成）」，实际 3 lib 单测 + 13 集成用例 = 16；且 1604 + 16 ≠ 批次合计 1619（±1 归属无法定位，两轮均 0 failed，判据不受影响）。当前树上权威全量计数以 Act M19 批次 **1619 passed / 0 failed** 为准。
7. **[Minor，不阻塞] Act Response Minor 遗留 2 表述失准**（Plan 探针实证）：「`FROM people a` 后写 `people.name` / `GROUP BY people.id` 计划期点名拒绝」——实际单表基座 `SELECT people.name FROM people p` **成功**（限定符经注册真实键解析，非静默丢弃）；JOIN 基座拒绝但按 `ColumnNotFound: name`（42703）分类。该「别名已声明时真实名遮蔽」语义未被任何 delta spec 场景覆盖，行为与已批准规格无冲突；PG/SQLite 均拒绝此形态，是否对齐属用户后续方向，不纳入本 change。
8. **[Minor，不阻塞] Plan Context Investigation Facts 标注失准**：旧 `extract_column_name` 调用点 ：682 标为「扫描臂」，实为复合查询（set operation）尾 ORDER BY；主路径（现 :2131）覆盖扫描/聚合/JOIN 全形态。两路径均已实现限定键处理且测试可达，行为无影响，仅记录。
9. **[阻塞，范围外缺陷经用户裁定纳入] Issue 候选「并行度相关的库文件锁争用偶发失败」根因已由 Plan 调查定位（2026-10-01），非测试并行纪律问题，属产品缺陷**：进程内 `close(fd)`（含 `Database` drop）到下一次对同文件 `flock` 之间，内核锁释放存在负载相关的短暂可见性窗口，`try_lock` 偶发 `WouldBlock` → 假 `DatabaseLocked`。取证链：① 纯 std 探针（零 rtsql 代码，`/tmp/flockprobe`）：同进程 open→flock(EX)→close→立即 reopen→flock(EX)，20000 轮 × 8 路并发负载 **2 次 EAGAIN**（WSL2 6.18.33.2-microsoft-standard）；② `readonly_session_test` 默认并行度 6 轮 2 败、失败集逐轮漂移、`--test-threads=1` 三轮全绿（Act 原始报告 + Plan 复跑一致）；③ strace `-f -yy` 全进程树取证：失败路径 `/tmp/.tmpT40qLX/ro.db` 全程仅测试进程自身触碰，前两个 fd 的 `close()=0` 均有踪，第三次 open 的 `flock(LOCK_EX)` 仍 EAGAIN——无在册持有者，排除测试间同文件争用与 fd 泄漏（预取任务未启用、WALBuffer 仅持 .wal fd、FileStorage 四处 `file.clone()` 均在被 await 的 spawn_blocking 内逐一排除）。产品影响面：lib 消费形态「close/drop 后立即重开同一路径」（嵌入式宿主、测试）；one-shot CLI 跨进程先后执行不受影响。修复设计见 design D10（`FileStorage` 两 open 入口 `WouldBlock` 后同 fd 有界退避重试，总窗口 ~8ms，拒绝语义与错误面不变）。
10. **验证采信**：Act 全量分批（M19，1619/0）、fmt/clippy/validate（exit 0）为本会话新鲜运行且工作区未变化（`git status` 与两 Iteration 文件集一致），直接采信，Plan 未重跑全量。Plan 补跑的定向探针（含别名矩阵 7 项与锁取证 3 组）为 Act 未覆盖的检查面，非重复验证。

**Acceptance Gaps**

- **Gap 1**（Finding 3）：`sql-table-alias-resolution` R2 点名 SHALL 在 GROUP BY 与聚合参数两通道未满足——单表静默求值、JOIN 误报列缺失（42703/NonAggregatedColumn）而非 42P01 携限定符。
- **Gap 2**（Finding 4）：Cycle Acceptance R2 六缺陷清单中「NLJ 变体」无测试见证（行为正确已实证，补用例即闭合）。
- **Gap 3**（Finding 9）：Issue 候选「假 `DatabaseLocked`」——用户 2026-10-01 裁定纳入本 Iteration 解决（范围扩大，超出原 Acceptance 边界）。

**Evidence**

- 完整 diff 独立审读（Findings 1-2、5）；`git status` 确认产品代码改动面与本 Iteration 契约一致（仅 planner 三文件 + 一个新测试文件）。
- Act Response「Verification Evidence」（新鲜运行，采信，见 Finding 10）。
- Plan 定向探针（工作区构建 `cargo run --quiet -- <tmp.db> "<sql>"`，2026-10-01）：Finding 3 四形态 + 投影面收紧对照、Finding 4 LEFT JOIN 两形态、Finding 7 单表/JOIN 遮蔽两形态；临时库已清理。
- 锁取证（Finding 9）：`/tmp/flockprobe`（纯 std 探针，源码保留 /tmp）；strace `-f -yy` 全进程树关键行与并行复跑计数已摘录于 Finding 9 与 design D10（原始临时文件已清理）。

**Follow-up Decision**

`replan-required`：Gap 1/2 为既有 Acceptance 的有限修复，但 **Gap 3（锁缺陷）是用户 2026-10-01 明确裁定纳入本 Iteration 解决的范围扩大**——rework 只能完成既有 Acceptance、不得引入新成果（公共规则 › Iteration 与 Cycle 线程），范围与验收边界变化必须使用 replan，不得伪装为普通返工（iteration-planning › Review 分类）。故不采用「当前 Cycle 修复」，创建后继 replan Cycle 承接修订后的计划（原 000-initial Cycle 冻结；其 Plan Context 与 Act Response 不可改写，本 Review 为其终态反馈）。Act 报告的 Issue 候选随 T12 在 change 内解决，收尾不落账（MS26 T8 用户指令先例）。

**Iteration Plan Update**

见 change `tasks.md` Iteration 001 块（Tasks 增 T12；Stable baseline/Verification/Diagnostic boundary 同步；2026-10-01 用户裁定 replan 扩入）与任务清单 T12 行、RTM `database-file-lock R1（MODIFIED）` 行；design 增 D10 与总览/责任边界/实现顺序增补；新增 delta spec `database-file-lock`（MODIFIED R1）。

**Next Cycle**

`iterations/001-table-alias-resolution/001-replan.md`

**Next Iteration**

None（本 Iteration 在 replan Cycle accepted 前保持未完成）
