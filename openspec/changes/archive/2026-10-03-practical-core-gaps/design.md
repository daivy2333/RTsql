# Design — practical relational core 缺口批

调查基线：HEAD 9a103c0（工作区含 SKILL.md 未提交文档修改，与本 change 无冲突）；本会话探针实测（release 二进制 2026-10-03 16:35 构建）与代码阅读核对。四项集中决策见 proposal。

## D1 建表首数据页初始化（R1）

**根因**：`create_table_with_constraints`（`src/storage/data/table_manager.rs:398`）在 `:424` 经 `storage().allocate_page()` 取得首数据页页号后直接进入索引创建与 catalog 写入，数据页保持零字节（page type 0x00）。`rtsql check` 的 slot/页类型校验将其判为损坏（两行 error、exit 1）。

**方案**：在 `allocate_page()` 之后、索引创建之前，镜像 `truncate_table` 的 head 原地重置先例（`table_manager.rs:1512-1517`）：`buffer_pool.get_page(page_id)` + `modify_page(|page| { SlottedPage::init(page, 0x03) })`，产生合法零 slot 数据页并置脏，随正常 close 落盘。

**通道覆盖**（单点收敛，无需多点修改——调查确认全部建表收敛于此入口）：CREATE TABLE 与 CTAS（`execute_ctas_routed` → `create_table_with_constraints`）、import 自动建表（CREATE TABLE DDL → `execute_sql` → 同）、restore（同）、ATTACH 目标库（同）。

**存量空表（集中决策 2：最小修复）**：打开路径不自愈。本会话探针已实证 0x00 head 页可直接 INSERT 且写入后 check 转绿（写路径对未初始化页自带 init 处理，与恢复重放「未初始化页 init」同源）；TRUNCATE 同样转绿（head 原地重置先例）。存量空表首次写入或 TRUNCATE 前维持 check error 报告——这是损坏检测不放松的代价，已被用户接受。

**失败语义**：init 失败 → `Err` 传播，与既有「allocate 后续步失败」的页生命周期语义一致，不新增清理路径。

**保留**：`check` 对 0x00 页的 error 分级、`validate_slots`、页 census 全部不动；损坏注入用例零回归。

## D2 非 INTEGER PRIMARY KEY 计划期拒绝（R4）

**拒绝点**：`build_create_table`（`src/parser/planner/ddl_dml.rs:996`）在 `extract_primary_key`（`:950`，列级与表级 `PRIMARY KEY` 已归一为列名）之后，按列名查 `column_defs` 的 `data_type: ColumnType`（UNIQUE 检查同型判断在 `:1130`）：显式 PK 存在且类型 ≠ `ColumnType::Int` → `PlanError::UnsupportedConstraint("NON-INTEGER PRIMARY KEY")`——复用 MS23 诚实化拒绝通道（错误携带能力名、SQLSTATE 既有映射），零副作用。

**精度边界（调查确认）**：检查只挂显式 PK 分支。未显式声明主键的「首列隐式主键」语义（`extract_primary_key` → `None`，引擎侧首列 fallback）不受波及——import 自动建表本就不显式声明 PK（`cli-noninteractive-shell` 既有 Requirement 明文），FLOAT/TEXT 首列重复合法行为零变化（S21）。

**存量可用**：拒绝面仅作用于新 DDL 计划期；存量 TEXT PK 表的 catalog 行、打开/读写/删除路径不触碰（S19）。

## D3 dump DDL 渲染适配（R5）

**渲染点**：`create_table_sql`（`src/cli/lifecycle.rs:963`）`:973` 的 `if col.column_name == table.pk_column` 追加 ` PRIMARY KEY`，加 `&& matches!(col.column_type, ColumnType::Int)`。

**调查确认的边界事实**：PK 列的 UNIQUE 被 PK 消费、不落独立 unique 标志（`ddl_dml.rs:1128` 与 `:1149` 两处 is_pk guard；MS23「PK 列声明 UNIQUE 消费为 PK」），故存量 TEXT PK 列的 catalog `col.unique == false`——渲染修复后无 `UNIQUE` 残留，产物为纯 `name TEXT`。INTEGER PK 表渲染逐字节不变。

**遗留兼容注记**：修复前生成的旧 dump 文件若含 `TEXT PRIMARY KEY` DDL，在新拒绝面下 restore 失败（exit 3）——该声明从未被强制，接受此不兼容并写入 SKILL.md/README 注记（T10）。

## D4 聚合表达式嵌入（R2）

**现状机制**（调查确认）：
- `is_aggregate_expr`（`aggregate.rs:30` 附近）只匹配顶层 `Expr::Function`，不递归——`MAX(id)+1` 判为非聚合项，落入表达式路径后 `build_expression` 对未注册聚合名报 `Unsupported expression type: MAX`（探针复现）。
- HAVING 编译器 `build_having_expression`（`aggregate.rs:21-154`）已在聚合输出行布局 `[group_cols..., agg_results...]` 上做「聚合调用 → 结果列引用」映射，但 match 只覆盖 Identifier/Function/Value/负数字面量四臂，`HAVING MAX(id)+1>2` 撞 `_ => UnsupportedExpression("EXPRESSION")` 兜底（`:154`）。
- 聚合查询 ORDER BY（`query.rs:2186` 分支）键只经 `extract_column_name` 编译为列名键，表达式键无通路。
- 常量混排：`SELECT MAX(id), 1` 的 `1` 被标 Expression 角色 → 既有聚合路径拒绝通道误报 `Non-aggregated column '1'`（探针复现）。

**方案**（三层推广，全部复用既有聚合输出行基建）：

1. **递归聚合提取基建**（aggregate.rs）：新增 `contains_aggregate_expr(&Expr) -> bool`（聚合函数名即真；BinaryOp/UnaryOp/Nested/CAST/CASE/COALESCE/标量函数参数递归下潜；子查询不下潜）。新增提取-改写遍历：自顶向下走表达式树，遇聚合调用节点（不进入其参数内部）经既有 `extract_aggregate_func` 规范化为 `AggregateFunc`，注册进共享 `aggregates/aggregate_args/agg_output_columns`（按 `result_column_name` 去重——同一调用两次出现共享一个输出列），节点改写为聚合输出行上的 `ColumnExpression{result_col_name}`。嵌套聚合（`MAX(MIN(x))`）由既有 `extract_single_column_arg` 的 `InvalidAggregateArgument` 通道自然拒绝（不进入参数内部，MIN 不会被提取）。
2. **SELECT 项分类扩展**（`query.rs:1341` 循环）：顶层命中维持既有路径；`contains_aggregate_expr` 命中 → 新「聚合嵌入表达式项」：提取-改写后按 Expression 角色进入既有条件 Projection 包装通道（MS13/MS25 混合投影基建）在聚合输出行上求值，输出名 = 别名优先否则表达式 Display。**纯常量项**（无聚合且无列引用）合法参与聚合查询——常量在任意行上求值恒定，不再被 NonAggregated 误拒（S9）。无 GROUP BY 的非聚合列引用维持既有拒绝（语义与文案不变）。
3. **HAVING 与 ORDER BY 同款编译器**：把 `build_having_expression` 扩展为完整表达式树编译（BinaryOp 算术/比较、LogicalOp、Nested、CAST、CASE/COALESCE、标量函数、负数字面量、聚合引用既有臂保持），`_` 兜底收窄为真正不可编译形态；聚合查询 ORDER BY 的非裸列键经同一编译器产出 `OrderByColumn.expr`（`Some`），SortExecutor 既有 decorate-sort-undecorate（MS25 T9，在未投影输入行上求值）直接消费——聚合输出行即 Sort 输入行，无需新执行器机制。表达式键豁免列集校验（输出别名键先例，`query.rs:2255` 段）。

**拒绝面保持**（S10）：WHERE/JOIN ON 内聚合——聚合名不在标量注册表（`function.rs REGISTRY`），既有「未注册函数名」点名通道自然拒绝（Act 以 e2e 固化）；嵌套聚合与常量项如上；NonAggregated 列引用不变。`DISTINCT` 量化词（`SUM(DISTINCT v)+1`）经既有 MS25 D7 命名管线（`max_distinct_v` 型结果列名）在提取-改写中天然保持（S11）。

**实现顺序**：提取基建 → SELECT 项 → HAVING → ORDER BY → 拒绝面 e2e 收口（依赖单向）。

## D5 UPDATE SET 值表达式与多列赋值（R3）

**现状机制**（调查确认）：`UpdateNode { table_name, key, column, new_value: Value }`（`plan.rs:276`）；`build_update`（`ddl_dml.rs:1824`）强制单赋值（`MULTI-COLUMN UPDATE` 拒绝）、右值仅四形态字面量（Value/TypedString 日期解析/NULL Identifier/其余 `UnsupportedValue`）；`UpdateExecutor`（`executor/update.rs`）写入链 coerce→类型门（FLOAT 升格）→NOT NULL→键列类型门→碰撞预检→版本链+索引维护（Step 7 rekey 三分支），全按单值驱动。

**方案**：

1. **计划面**：`UpdateNode` 改 `assignments: Vec<UpdateAssignment>`，`UpdateAssignment { column, value: UpdateValue }`，`UpdateValue::Literal(Value) | Expression(ExpressionRef)`（enum，字面量臂保留既有逐字节语义含 MS13 TypedString 日期解析）。`build_update` 解除单赋值限制；逐赋值：字面量形态 → Literal；`Expr::Identifier` → 表列校验（未知列 `ColumnNotFound`）+ Expression（对旧行列引用）；子查询形态显式点名拒绝；`contains_aggregate_expr` 命中点名拒绝；其余经 expression.rs 既有值表达式内核编译（算术/CAST/CASE/COALESCE/标量函数），列引用按 `TableMeta.columns` 序解析索引。`plan_format`/`correlated.rs`/`get_plan_output_columns` 消费臂穷尽补齐。
2. **执行器多列化**：逐命中行——先读全行 `values[]` 快照，对全部 Expression 右值基于同一快照求值（S14 标准 SQL 语义）；随后对每个赋值列按既有门序处理：`coerce_datetime_write` → 写入类型门（FLOAT 列 Int 升格改写）→ NOT NULL → 键列类型门（PK 赋值，Int/NULL 之外 `KeyTypeMismatch`）。**预检前置**：全部行的任何写入前完成 PK 碰撞预检（新键 `to_key()` search，既有 D4 预检推广）与变更唯一列碰撞预检（含非 Int 结果 `KeyTypeMismatch`）；预检通过后执行版本链写入与索引维护——PK 变更走既有 rekey 三分支（删旧键插新键），变更唯一列删旧条目插新条目，次级索引既有维护路径保持，未变更列零触碰。
3. **失败语义**：表达式求值失败或约束违反 → 语句失败，隐式事务回滚本语句全部已写行（既有 DML 包裹语义，保持）；预检类失败维持零副作用。
4. **零变化面**：`ON CONFLICT DO UPDATE SET` 三形态与 `REPLACE INTO`（UpsertExecutor 独立通道）、WHERE 主键等值要求、视图拒绝、附加库路由不动；UPDATE 无子查询能力边界保持（显式点名）。

**实现顺序**：计划面（含 plan 节点演进与消费臂）→ 执行器多列化 → e2e 矩阵（表达式形态 × 门序 × rekey/唯一/次级索引 × 回滚）。

## D6 文档同步（T10）

SKILL.md 本轮已按**现状**写入的 Aggregate/Write 两行边界（聚合顶层限定、UPDATE 字面量限定）随 R2/R3 落地改写为新能力；README.md/README.zh-CN.md 能力清单同步（聚合嵌套、UPDATE SET 表达式、非 INTEGER PK 进入拒绝清单、dump PK 渲染注记与旧 dump 兼容注记）。置于 Iteration 002 收尾（能力全部落地后一次写准）。

## D7 恢复重放目标表缺失的错误面诚实（R6/T11，2026-10-03 用户指令并入）

**来源**：Iteration 000 Act Response Issue 候选 + Plan Review Finding 4（lib 句柄 drop 不 `close()` → 未 checkpoint 的 catalog 脏页丢失 → 重开报 `WAL redo failed: table 't' lookup failed during redo: table not found: t`——「DDL 从未持久化」被诊断为「redo 目标表缺失」）。

**现状机制**（调查确认，2026-10-03）：`redo_record`（`src/wal/recovery.rs`）Insert/Update/Delete 三臂（`:722`/`:820`/`:947`）的 `table_name` 来自 **WAL 记录**，`table_manager.get_table` 失败一律归并为 `WalError::RedoFailed(format!("table '{}' lookup failed during redo: {}", …))`；缺失表的底层错误是 `StorageError::TableNotFound`（`src/storage/error.rs:90`，文案 `table not found: {0}`）；`WalError::RedoFailed` display 为 `WAL redo failed: {}`（`src/wal/record.rs:518`）。恢复期其余 `get_table` 消费点——重放前预扫描 `build_pk_version_maps`（`:349`）、索引重建（`:1040`）、未提交中止标记（`:1416`）、tail 持久化（`:670`）——全部遍历 **catalog 行**（`scan_tables()`），表按构造存在，不涉及本场景；目标表缺失只在 redo 三臂可达。

**场景构造**：DDL 不写 WAL（既有设计）→ CREATE TABLE 只落 catalog 脏页；句柄 drop 不 `close()` → 脏页丢失、WAL 留有已提交 DML 记录 → 重开恢复按记录表名 `get_table` 未命中。归并文案把根因（无 checkpoint 关闭）误诊为表缺失，排障指向性弱。

**方案（错误面指因，行为不变）**：三臂 `get_table` 失败分支对底层错误做变体判别——`StorageError::TableNotFound` → 定向文案，点名表名 + 陈述「WAL 已提交记录引用的表在 catalog 中缺失」+ 陈述最可能原因（上次关闭未经 checkpoint，如 lib 句柄未 `close()` 即 drop、未持久化 DDL 随脏页丢失）+ 明示打开拒绝；其余错误维持既有归并文案逐字节不变。仍走 `WalError::RedoFailed(String)`（不新增错误变体——失败类别不变：redo 中止），display 前缀 `WAL redo failed: {}` 不变；打开失败的传播路径、退出码、K05 显式报错语义全部不变（不跳过/不丢弃不可重放记录）。

**测试落位**：新 `tests/recovery_error_surface_test.rs`（lib 级——该形态 CLI 不可达：CLI 全路径 close）。S22 RED＝drop 不 close 重开错误信息为旧归并文案；GREEN＝含定向指因要素（表名、catalog 缺失、无 checkpoint 关闭原因、打开拒绝）。S23 对照＝同序列走 `close()` → open 成功行可见（防定向文案误伤正常恢复）。既有恢复套件（recovery_tail_persistence / recovery_replay_convergence / wal_recovery_large 等）零回归。

## D8 聚合显示名重名时解析正确（R2/T12，Iteration 001 Plan Review 补强）

**来源**：Iteration 001 Plan Review 独立审读发现——T4 将条件包装的聚合项定位由旧**位置序**（`agg_seen` 逐角色递增，见该 diff 移除行）改为**显示名查表**（`aggregate_display_names.iter().position(eq_ignore_ascii_case)`），`compile_aggregate_row_expr` 的聚合 Function 引用臂同以显示名 `position` 首匹配解析。显示名唯一时两法等价；重名时首匹配歧义——探针实测四形态静默取错值（2026-10-03，target/debug 含 Iter 001 实施面）：

| 形态 | 实测 | 正确 |
|---|---|---|
| `SELECT MAX(a) AS m, g, MAX(b) AS m FROM t GROUP BY g` | 第三列 20/30（max(a) 值） | 200/300 |
| `SELECT MAX(a) AS max_b, g, MAX(b) FROM t GROUP BY g` | 第三列 20/30 | 200/300 |
| 同② + `HAVING MAX(b) > 150` | 空结果（按 max(a) 过滤） | 两组均保留 |
| 同② + 嵌入 `MAX(b)+1` | 21/31 | 201/301 |

修复前（HEAD）位置序机制对四形态均正确——属本 change 引入的回归，违反 T4 Preserve「混合投影包装不变」，修复面在本 change 自身变更面内。

**歧义根源**：`SELECT MAX(a) AS max_b, MAX(b)` 类形态下两个注册项显示名相同但 func/实参不同；名字 → 输出列的映射在该形态下不是单射。

**方案（要求与边界，机制留 Act）**：聚合输出引用的解析 SHALL 消除重名歧义——每处引用（wrap 装配聚合项、嵌入表达式聚合引用、HAVING 聚合引用、ORDER BY 表达式键聚合引用）SHALL 解析到与该调用对应的注册聚合输出。两条可行路径（Act 择一或等价方案）：(a) 引用解析按「注册项 func + 归一实参」结构匹配代替名字匹配——同一调用多次出现自然共享一个输出列（去重语义保持），显示名（可观察表头）零变化；(b) 显示名清单保持可观察值不变、内部为每注册项维护出现序号，引用按「名字 + 该名字第 k 次出现」解析（顶层聚合角色按 SELECT 序注册，k 次序对应）。约束：可观察表头（重名列保留）与「同一调用共享输出列」语义零变化；直出路径（无 T4 项、名字互异）逐字节不变；错误面零变化。

**测试落位**：`tests/aggregate_extension_test.rs` 新增重名矩阵（S24 四形态 + 共享输出列对照 `SELECT MAX(id), MAX(id)+1`）——RED＝四形态当前取错值（探针同源断言），GREEN＝各解析到正确注册输出；既有聚合五套件零回归。

## 责任边界

- **修改**：`table_manager.rs`（create init 单点）、`ddl_dml.rs`（PK 拒绝、build_update）、`plan.rs`（UpdateNode）、`executor/update.rs`、`planner/aggregate.rs`（提取基建 + HAVING 扩展 + 引用解析去歧义）、`planner/query.rs`（SELECT 项分类、ORDER BY 聚合分支、wrap 聚合定位去歧义）、`lifecycle.rs`（dump 渲染）、`wal/recovery.rs`（redo 三臂缺失表定向文案）、error 面如需新文案走既有变体。
- **保持**：UpsertExecutor/REPLACE 通道、`check` 校验逻辑与分级、恢复重放与 heal_stale_tail 的行为语义与打开结果、keyless row 语义、隐式主键语义、既有错误文案通道（NonAggregated/InvalidAggregateArgument/UnsupportedConstraint/KeyTypeMismatch/ColumnTypedMismatch 等）、`rtsql schema` 渲染语义、非缺失表类 redo 失败文案。
- **禁止**：非整数键真实强制、聚合进入 WHERE/ON、窗口函数/递归 CTE、UPDATE 子查询、open 期自愈、`check` 分级调整、恢复重放跳过/丢弃不可重放记录。
