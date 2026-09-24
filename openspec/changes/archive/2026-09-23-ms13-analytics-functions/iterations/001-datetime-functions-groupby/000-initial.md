# Iteration 001 / Cycle 000: datetime 函数与分桶

## Plan Context

- Status: ready
- Iteration: 001-datetime-functions-groupby
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T6, T7, T8
- Depends on: Iteration 000（datetime 类型底座——本 Iteration 的全部函数与分桶消费其 Value 变体与 datetime.rs 算法）
- Stable baseline: 日期函数族（now/date/year~second/date_trunc/datediff）+ INTERVAL 算术 + `GROUP BY date_trunc(...)`/别名/位置分桶可用；I043/I044 收口；既有标量函数/聚合面零回归——Iteration 002 的 CLI 命令与 no-FORM 可在其上叠加
- Verification boundary: `tests/datetime_function_test.rs` + `tests/group_by_expr_test.rs` 全绿 + `tests/scalar_function_test.rs`（含新增 I043/I044 用例）全绿 + 既有全量零修改
- Diagnostic boundary: `src/executor/{function,datetime,aggregate}.rs`、`src/parser/planner/expression.rs`（Interval 臂）、`src/parser/planner/query.rs`（GROUP BY 解析/混合投影/条件包装）、`src/executor/plan.rs`（AggregateNode）
- Deferred tasks: T9–T13（Iteration 002）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 决策 1/3、DA1/DA7–DA9、design D10–D12/D15/D16；Iteration 000 全部既有面（Preserve 边界同源）
- Excluded scope: no-FROM/CLI 分析命令（Iteration 002）；strftime/to_date 族（change Out of Scope）；`ORDER BY <表达式文本>`（D12 Non-goal，别名已覆盖）

**Objective**

`SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev GROUP BY day` 端到端可用（三引用形态等价），日期函数族与 INTERVAL 算术在 WHERE/SELECT 双侧可达且语义锁定，abs/round 极端输入收口，函数名大小写不敏感获得 SQL 层见证。

**Background**

tasks.md MS13-T02 引擎侧（R18 主题 7 排序 4/5）；决策 3（2026-09-23）裁定核心函数集 + GROUP BY 表达式扩展。Iteration 000 已提供类型底座（accepted Review 2026-09-23）。

**Investigation Facts**

- Current Baseline: Iteration 000 最终 Act Response——19 修改 + 2 新增文件（详单见其 Changed Files），全量 **987 tests / 0 failed / 2 ignored**（936 基线 + 51 新增；1 处 BH-1 型校准见 spec R7 校准段）、clippy/fmt/validate 全 0；Review accepted（偏差六项全部非阻塞闭环）。工作区未提交（对照基线 7364bc9 + MS09 收尾 docs 增量）。
- Current-State Evidence（Iteration 000 后现状，本 Iteration 消费面）:
  - `src/executor/datetime.rs` 已就绪：`date_fields(i32) -> (i32,u32,u32)`、`ts_fields(i64) -> (i32,u32,u32,u32,u32,u32,u32)`、`trunc_ts(i64, TruncUnit) -> i64`（TruncUnit 六单位枚举带注释性 `#[allow(dead_code)]`，本 Iteration T7 消费 Month/Hour/Minute/Second）、`days_in_month`、`days_from_civil`/`civil_from_days`、`DAYS_0001_TO_1970`；日期值日历范围 0001-01-01..9999-12-31。
  - `Value::Date/Timestamp` 全链可用（比较/Hash/Display/序列化/渲染）；谓词层日期族同变体守卫生效（`predicate.rs:122-136`——Date×Timestamp/日期×非日期 = TypeMismatch）。
  - `function.rs`：REGISTRY `(&'static str, usize, usize)` + `eval_scalar(name, args)` 分派；`FunctionExpression::eval_owned` 求值序 D3（参数全求值→错误先传播→任一 NULL→NULL 跳类型校验→分派）；`string_arg`/`int_arg`/`float_arg` 严格类型 helper 先例；`check_scalar_function` 的 `argc >= min` 使 min=0 零参合法（now() 注册 (0,0) 即可，无 planner 零参拒绝门——R1 修订 carve-out 已入 delta）。
  - `ast.rs extract_columns` Function 放行门查 `is_scalar_function(name)`（`ast.rs:60`）——新注册函数名自动获得 SELECT 放行；`extract_single_col_static` 等聚合名分支不受影响。
  - ABS 臂现状 `function.rs:216` `Value::Int(n) => Ok(Value::Int(n.abs()))`（I043 目标：i64::MIN 溢出）；ROUND 现状 `10f64.powi(digits)` 乘除（`|digits|>308` → inf/0 污染）。
  - `tests/scalar_function_test.rs` 全部小写形态（I044：无大写/混合变体 SQL 层用例，grep 实证 MS11-T03 Review F1）。
  - GROUP BY 现状：`query.rs:704` `expr_to_column_name` 逐项提取（Function/TypedString/位置数字 → 报错——仅列名可达）；`query.rs:471` `has_expression_items && has_aggregates` 直接拒绝（`InvalidAggregateArgument "Expected column name"`）；非聚合列检查 `query.rs:716` `group_by.contains(col)`；`AggregateNode { group_by: Vec<String>, aggregates, output_columns, column_indices }`（`plan.rs:347`）；`aggregate.rs:279 extract_group_key` 按 column_indices 名字→索引取值；`build_output_rows` 输出 = group_key（GROUP BY 序）++ aggregates（投影序）；HAVING 经 `build_having(having, &agg_output_columns)` 在 Aggregate 之上。
  - SELECT 表达式项顶层 Projection 编译通路（`query.rs:507 build_expression(&table_name, item)`）就绪且经 Iteration 000 BinaryArith/TypedString 臂扩展。
  - BinaryOp 算术构建点：`expression.rs build_expression` BinaryOp 臂分流 Plus/Minus/Multiply/Divide → `BinaryArithExpression`（**数值严格守卫——非数值操作数 TypeMismatch**）。设计 D11：INTERVAL 腿不得走该节点，在 BinaryOp 构建点先探测 Interval 字面量腿分流到独立 `IntervalArithExpression`（Iteration 000 Review F2 澄清：`BinaryArithExpression` doc 注释提及 INTERVAL 属表述误导，以本契约为准）。
  - sqlparser 0.44 `Expr::Interval(Interval { value: Box<Expr>, leading_field: Option<DateTimeField>, leading_precision, last_field, fractional_seconds_precision })`；支持形态 `INTERVAL '1 day'`（leading_field=None，单位在字符串内）与 `INTERVAL 1 DAY`（leading_field=Some）；`YEAR TO MONTH` 类多字段形态 last_field=Some → 拒绝。DateTimeField 含 Year/Month/Day/Hour/Minute/Second（另含 Millisecond 等小数单位——仅接受六主力单位）。
- Code and Critical Path: planner（Interval 臂/GROUP BY 解析/混合投影）→ 执行器（function.rs 分派/datetime.rs 算法/aggregate.rs 分桶求值化）→ 测试三套件。

**Implementation Guidance**

- T6 先行（函数族独立可验）；T7 复用 T6 的 datetime helper；T8 最后（分桶依赖 date_trunc 可用后 e2e 才有意义）。
- 月份加法锚定算法：`add_months(y, m, d, n)`——目标月 `m+n` 进位年，日 `min(d, days_in_month(目标年月))`（DA8 同日锚定截月末，PostgreSQL 语义）；先月后微秒（IntervalParts months 与 micros 分别应用，顺序：months 先）。
- datediff 的 month/year 语义（DA8 截断）：months = (y2−y1)×12+(m2−m1)，若 (d2,t2) 时间点早于 (d1,t1) 的对应日时刻则再减 1（日历差方向）。
- `IntervalParts { months: i32, micros: i64 }` 解析：字符串形态按空白分隔 `<n> <unit>`（大小写不敏感，n 允许负号；多段如 `'1 day 2 hours'` 拒绝）；数字+leading_field 形态直接换算。微秒换算 day=86_400_000_000、hour/minute/second 依 datetime.rs 既有常量。
- GROUP BY 解析序实现锚点（D12）：列名 → SELECT 别名（`ExprWithAlias.alias` 等值，大小写不敏感）→ 表达式文本（双侧 `Expr::to_string()` 归一比较）→ 位置（`Expr::Value(Number)` 1-based 且 ≤ 投影项数）。全部不匹配 → `NonAggregatedColumn(项文本)`。
- 条件 Projection 包装判定：纯列名 GROUP BY 且 SELECT 序为「键项在前、聚合项在后」→ 既有直出路径零触碰；否则包 `ProjectionNode`（键项 → ColumnExpression{group 序}，聚合项 → ColumnExpression{k+聚合序}，name = SELECT 名）。
- `trunc_ts` 消费后移除 `#[allow(dead_code)]`。

**Behavioral Change**

- 当前：无日期函数（`year(...)` 等未注册名 → SELECT 位 `Unsupported statement type`/谓词位 `Unsupported expression type`）；无 INTERVAL；GROUP BY 仅列名且表达式项+聚合互斥；abs(i64::MIN) debug panic/release 回绕；round 极端 digits → inf/0/NaN。
- 目标：函数族 12 项注册可用（严格类型/NULL 短路 D3/单位大小写不敏感）；`d ± INTERVAL` 可达；GROUP BY 三引用形态 + 混合投影解锁（不匹配显式错误）；abs 溢出显式错误、round 饱和；大写/混合函数名有 SQL 层见证。
- 接口语义：`REGISTRY` 扩展与 `eval_scalar` 臂；`AggregateNode` 加性字段 `group_key_exprs: Vec<ExpressionRef>`（旧构造点统一补空/等价编译——列名项编译为 ColumnExpression 保持逐字节等价）；无新错误变体（ValueError::TypeMismatch/Overflow 类文案、PlanError::ParseError 点名）。

**Task Contracts**

### T6: 日期函数族 + I043/I044

- Requirement/Scenario: datetime-functions R1（抽取/截断/now/错误面/NULL）、R3（datediff）、R4（零回归）；sql-scalar-functions 修改 R1（大小写见证/零参 carve-out）、R3（abs 溢出/round 饱和）；设计 D10/D15
- Depends on: Iteration 000（类型底座；无本 Iteration 内前置）
- Targets: `src/executor/function.rs`（REGISTRY +12、eval_scalar 臂、date/ts 参数 helper、ABS/ROUND 修正）、`src/executor/datetime.rs`（datediff/add_months 若需补充纯函数）、`tests/datetime_function_test.rs`（新）、`tests/scalar_function_test.rs`（增）
- Current behavior: 12 函数名均未注册（未知名既有文案拒绝）；ABS i64::MIN panic/回绕；ROUND 极端 digits inf/0/NaN；scalar_function_test 无大写变体
- Required behavior: REGISTRY 增 `NOW(0,0)`/`DATE(1,1)`/`YEAR|MONTH|DAY(1,1)`/`HOUR|MINUTE|SECOND(1,1)`/`DATE_TRUNC(2,2)`/`DATEDIFF(3,3)`；eval_scalar 臂——now() 墙钟 Timestamp、date(x) Date 恒等/Timestamp 截断、year/month/day 接受 Date|Timestamp、hour/minute/second 接受 Timestamp（Date 视零点）、date_trunc(unit,x) 单位 String 大小写不敏感匹配六单位（Date 仅 year/month/day，其余单位对 Date 为类型错误；未知单位运行时错误点名 unit）、datediff(unit,a,b) 同族整单位截断差；参数严格类型（新 date_arg/ts_arg，非日期族 TypeMismatch）；NULL 短路沿用 eval_owned D3；ABS Int 臂 `checked_abs` 溢出 → 显式错误；ROUND digits 取整后 `>308 → Float(x)`、`<-308 → Float(0.0)`
- Required changes: 上述 + 测试——datetime_function_test ~20（抽取/截断四单位/now 非递减/datediff 日月/NULL/类型错误/单位错误/闰日截断）；scalar_function_test 增 I043 4 用例（abs MIN 溢出错误、round(1,1000)=1.0、round(1,-1000)=0.0、round(2.5,400)=2.5）+ I044 3 用例（UPPER/Abs/MiXeD_Length 大写混合形态与小写等价）
- Preserve: 既有十函数行为与文案逐字节不变；未注册名既有拒绝文案；`eval_owned` 求值序 D3；聚合五名互斥
- Forbidden: 不接受字符串日期参数（严格类型——`year('2024-01-15')` 是类型错误，CAST/类型字面量是显式通道）；不做 date_trunc 单位的 plan 期校验（unit 是运行时值）
- Test witness: datetime_function_test RED——`SELECT year(d) FROM ev` 当前 `Unsupported statement type`；`SELECT abs(i) FROM t` 含 i=i64::MIN 行当前 debug panic/release 回绕（测试断言显式错误）；`round(1,1000)` 当前 inf/NaN
- GREEN condition: 三套件全绿 + 既有 scalar_function_test 零修改
- Verification: `cargo test --test datetime_function_test --test scalar_function_test`
- Stop when: D3 求值序与新函数错误传播语义冲突且无法按 spec 场景裁决（返回 Plan）

### T7: INTERVAL 表达式算术

- Requirement/Scenario: datetime-functions R2（日时算术/月末锚定/拒绝面）；设计 D11
- Depends on: T6（datetime helper 扩展共用模块）
- Targets: `src/executor/datetime.rs`（IntervalParts 解析/换算/add_months）、`src/executor/predicate.rs` 或 `datetime.rs`（IntervalArithExpression 节点）、`src/parser/planner/expression.rs`（build_expression/build_where 的 Expr::Interval 臂 + BinaryOp 分流）
- Current behavior: `d + INTERVAL '1 day'` 在 build_expression BinaryOp 臂落入 `BinaryArithExpression` 数值守卫 → 运行时 TypeMismatch（Date 非数值）；独立 `SELECT INTERVAL '1 day'` 经 ast 放行后同样类型错误
- Required behavior: `Expr::Interval` 双入口臂——两形态解析为 `IntervalParts { months, micros }`（字符串 `'N unit'` 与 `N UNIT`；多字段/小数单位/未知单位/多段字符串 → ParseError 点名）；`d + INTERVAL`/`ts ± INTERVAL` 编译为 `IntervalArithExpression { left, op, interval }`（left 为 Date→Date、Timestamp→Timestamp；months 先应用〔同日锚定截月末〕再 micros；结果日历范围校验）；`Interval ± Interval`、Interval 在左、独立投影项（`SELECT INTERVAL '1 day'`）显式拒绝（ParseError 点名）；BinaryOp 构建点在数值分流**之前**探测 Interval 腿
- Required changes: 上述 + datetime_function_test 增 INTERVAL 段（1 day/90 minutes 加减/1 month 月末锚定 2024-01-31→02-29/负区间/年算术/五类拒绝面）
- Preserve: `BinaryArithExpression` 数值语义零变化（Interval 分流在其之前，纯数值查询路径不经过新探测开销之外的改动）；WHERE/SELECT 双入口一致性
- Forbidden: INTERVAL 不落 Value 变体（不可存储/不可比较/不入索引）；不做 `Interval ± Interval` 聚合形态
- Test witness: datetime_function_test INTERVAL 段 RED——`SELECT d + INTERVAL '1 day' FROM ev` 当前 TypeMismatch 错误
- GREEN condition: INTERVAL 段全绿 + 既有面零回归
- Verification: `cargo test --test datetime_function_test`
- Stop when: sqlparser Interval AST 形态与调查记载不符（如字符串形态 leading_field 非 None）且无法局部适配（返回 Plan）

### T8: GROUP BY 表达式/别名/位置 + 混合投影

- Requirement/Scenario: group-by-expression R1（别名/表达式/位置三形态等价）、R2（不匹配显式错误/NULL 归并/既有零回归）；设计 D12
- Depends on: T6（date_trunc 可用是分桶 e2e 的前提）
- Targets: `src/parser/planner/query.rs`（GROUP BY 项解析序/混合投影解锁检查/条件 Projection 包装）、`src/executor/plan.rs`（AggregateNode.group_key_exprs 加性字段）、`src/executor/aggregate.rs`（extract_group_key 求值化）、`tests/group_by_expr_test.rs`（新）
- Current behavior: GROUP BY 仅列名（函数/位置 → expr_to_column_name 报错）；`SELECT date_trunc('day',ts) AS day, COUNT(*) ... GROUP BY day` 在 query.rs:471 被拒（Expected column name）
- Required behavior: GROUP BY 项按 D12 解析序解析（列名→别名→表达式文本→位置），解析结果编译为 `group_key_exprs`（列名 → ColumnExpression{column_indices 已知索引}，表达式 → build_expression 编译）；`extract_group_key` 改为逐行求值 exprs；`has_expression_items && has_aggregates` 解锁——每个表达式 SELECT 项必须解析到某 GROUP BY 键（否则 NonAggregatedColumn），纯列名项保持既有 contains 检查；输出装配——纯列名且「键在前聚合在后」序 → 既有直出路径零变化，否则 ProjectionNode 包装（键项/聚合项 ColumnExpression 重排，name=SELECT 名）；Sort/Limit 叠加于包装之上（ORDER BY 别名经名字查找生效）；`GROUP BY ALL` 既有语义保持
- Required changes: 上述 + group_by_expr_test ~10（date_trunc 别名/表达式/位置三形态逐字节等价/交错序 `COUNT(*)` 在前/NULL 键归并单组/不匹配表达式显式错误/纯列名既有形态零回归/纯列名交错序仍直出）
- Preserve: 纯列名 GROUP BY 全部既有行为（含输出行序/列名）逐字节不变；HAVING 既有机制；非聚合列检查语义
- Forbidden: 不动 `expr_to_column_name` 本体（新解析逻辑在 GROUP BY 消费侧）；不做 `ORDER BY <表达式文本>`（D12 Non-goal）；不做 HAVING 表达式扩展
- Test witness: group_by_expr_test RED——混合投影 `SELECT date_trunc('day',ts) AS day, COUNT(*) FROM ev GROUP BY day` 当前 `Expected column name` 拒绝；`GROUP BY 1` 当前 expr_to_column_name 报错
- GREEN condition: 套件全绿 + 既有聚合/GROUP BY 相关测试（aggregate_test/planner_test/pushdown_test 等含聚合用例）零修改
- Verification: `cargo test --test group_by_expr_test` + `cargo test --no-fail-fast` 全量（Iteration 收口）
- Stop when: 条件包装与 Sort/Having 叠加面出现超出 D12 记载的形态冲突（design R2 风险触发——返回 Plan 评估 replan）

**Invariants**

- Iteration 000 全部 Preserve/Forbidden 边界延续（类型底座行为零回归；谓词日期守卫、写入 coerce、WAL/恢复零改动）。
- D3 求值序（参数求值→错误先传播→NULL 短路）适用于全部新函数。
- 严格类型哲学：函数参数无隐式转换；INTERVAL 不可存储。
- 纯列名 GROUP BY 路径逐字节零变化（条件包装只在表达式/交错形态激活）。
- Evidence 预算与身份型证据禁令。

**Non-goals**

strftime/date_format/to_date；时区；`ORDER BY <表达式文本>`；HAVING 表达式；CLI 分析命令与 no-FORM（Iteration 002）；INTERVAL 存储。

**Acceptance**

datetime-functions R1–R4 + group-by-expression R1–R2 + sql-scalar-functions 修改 R1/R3 全场景经 `tests/datetime_function_test.rs`（~30 含 INTERVAL 段）+ `tests/group_by_expr_test.rs`（~10）+ scalar_function_test 新增 7 用例 + 全量零修改覆盖（映射见 change tasks.md RTM 对应行）。

**Verification**

- `cargo test --test datetime_function_test --test group_by_expr_test --test scalar_function_test`：全绿。
- `cargo test --no-fail-fast`：987 既有 + 新增全绿、零修改。
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`：0。
- 探针抽检：`SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev GROUP BY day` 端到端（Act Response 记录输出）。

**Gate 2 Readiness**

| 检查项 | 状态 | 证据 |
|---|---|---|
| 无 Missing requirement | PASS | RTM 对应行（datetime-functions R1–R4 / group-by-expression R1–R2 / sql-scalar-functions 修改 R1/R3）→ T6–T8 全映射 |
| 无未批准 Simplified | PASS | 无 Simplification（决策 3 与 DA7–DA9 经 Gate 1 批准；`ORDER BY <表达式文本>` 为 D12 记载 Non-goal 非裁剪） |
| 调查完整 | PASS | Investigation Facts：Iteration 000 accepted 现状（987/0/2 + 静态 0）+ 本 Iteration 消费面行号级证据（function.rs REGISTRY/求值序/ABS-ROUND 现状、query.rs:471/704/716、aggregate.rs:279、Interval AST 形态） |
| 设计闭合 | PASS | D10–D12/D15 无契约级 TBD；R2 交错序风险有 Stop when 定义（返 Plan 评估，非 Act 决定） |
| 任务可执行 | PASS | T6–T8 契约各含 Targets/Current/Required/Preserve/Forbidden/witness/GREEN/Verification/Stop |
| 分轮合理 | PASS | change 级 3 Iteration 审计结论维持；001 为函数→区间→分桶依赖递进单一成果 |
| 追踪完整 | PASS | RTM 链路齐（含本 Iteration 六 Requirement 行） |
| 验证充分 | PASS | 每任务 RED→GREEN + Iteration 收口全量零修改（T8 Verification）+ 探针 |
| 无身份型证据工程 | PASS | Persisted Evidence none；目标行为观察 |
| 无需 Act 决定的实质未知项 | PASS | F2 澄清（Interval 独立节点）已写入 T7 契约；负 months 锚定语义已定并指明测试锚点 |
| 产物一致 | PASS | validate --strict PASS（2026-09-23）；Cycle 契约与 design/tasks/specs 互检 |
| Persisted Evidence 契约 | PASS | Mode none |
| 用户批准计划 | PASS | 用户批准「批准」原话记录于会话（2026-09-23，Iteration 000 Review 交付后） |

**Persisted Evidence**

- Mode: none
- 全部验证经 Act Response 承载；无白名单情形。
- Budget: 不适用。

**Risks and Notes**

- design R2（聚合输出形状交错序包装与 Sort/Having 叠加）为已登记风险，Stop when 已定义——非 TBD，处置方向明确（返 Plan 评估 replan）。
- date_trunc 单位错误是运行时错误（unit 为值非计划期常量）——spec 场景已按运行时锁定，非未知项。
- IntervalParts 负数月份（`-1 month`）锚定语义：先加月（日历）后微秒，负 months 同算法（目标月 = m+n 进位）——与 PostgreSQL 一致方向，测试覆盖 2024-03-31 − 1 month = 2024-02-29。

## Act Response

- Status: reported

**Implemented**

T6–T8 全部完成（RED→GREEN 逐任务见证）：T6 日期函数族（REGISTRY +10 项 NOW(0,0)/DATE(1,1)/YEAR|MONTH|DAY(1,1)/HOUR|MINUTE|SECOND(1,1)/DATE_TRUNC(2,2)/DATEDIFF(3,3)；eval_scalar 新臂——now() 墙钟微秒 Timestamp、date() 恒等/截断、year/month/day 经 `ymd_arg`（Date|Timestamp）、hour/minute/second 经 `hms_arg`（Timestamp 直取/Date 视零点）、date_trunc 单位大小写不敏感六单位（Date 仅 year/month/day，时间单位类型错误）、datediff 同族整单位朝零截断；ABS Int 臂 `checked_abs` 溢出显式错误；ROUND digits >308 → Float(x)、<-308 → Float(0.0)；eval_scalar 错误载体 ValueError→String 以承载点名文案）；T7 INTERVAL 算术（datetime.rs `IntervalParts{months,micros}` + 字符串形态 `'N unit'`（单复数/大小写不敏感/负数）与数值+字段形态 `N UNIT` 解析（多段/多字段 TO/精度语法/未知字段单位点名拒绝）、`add_months_ymd` 同日锚定截月末、`add_interval_to_date/ts` 月先微秒后 + 0001..9999 范围校验；predicate.rs `IntervalArithExpression` 节点（NULL 传播、Date→Date/Timestamp→Timestamp、Sub 求值期取负 i64）；planner build_expression BinaryOp 臂数值分流前探测 Interval 腿（Interval 在左/双侧/乘除腿点名拒绝）+ 独立 `Expr::Interval` 臂点名拒绝 + build_where 比较臂解缠绕 helper；ast.rs 两放行清单 +Interval）；T8 GROUP BY 表达式分桶（`AggregateNode.group_key_exprs` 加性字段；planner `resolve_group_by_item` 按 D12 解析序（列名→别名（大小写不敏感）→SELECT 项表达式文本（双侧 to_string）→1-based 位置；聚合项命中拒绝；越界/不匹配 NonAggregatedColumn 点名）；`extract_group_key` 求值化（错误经 StorageError::ExecutionError 显式传播）；混合投影解锁——`has_expression_items && has_aggregates` 不再拒错，表达式项必须被某分组键匹配否则 NonAggregatedColumn，范围外混用（子查询/JOIN/通配）维持既有点名拒绝；条件 Projection 包装——直出判定 = 无表达式项 ∧ 全部键为列名 ∧ SELECT 序键前聚合后（D12 字面准则），否则包装 `ProjectionNode`（键项→分组位置、聚合项→k+聚合序，列名 = SELECT 名）位于 Having 之上、Sort/Limit 之下；包装形态 AggregateNode.output_columns 改行序命名（GROUP BY 键序 ++ 聚合序，HAVING 绑定与聚合行形状一致），直出形态保持 agg_output_columns 逐字节）。

**Changed Files and Symbols**

- 修改：`src/executor/function.rs`（REGISTRY +10、eval_scalar 新臂 ×10 + ABS/ROUND 收口、错误载体 String、`ymd_arg`/`hms_arg`/`date_val_arg` helper）、`src/executor/datetime.rs`（`parse_trunc_unit`/`trunc_date`/`datediff`/`IntervalParts`/`parse_interval_string`/`interval_parts_from_unit`/`add_months_ymd`/`add_interval_to_date`/`add_interval_to_ts` + TruncUnit 去 `#[allow(dead_code)]` + 5 组单测）、`src/executor/predicate.rs`（`IntervalArithExpression` 节点 + Expression impl）、`src/executor/mod.rs`（导出 IntervalArithExpression）、`src/executor/plan.rs`（AggregateNode 加性 `group_key_exprs`）、`src/executor/aggregate.rs`（AggregateExecutor `group_key_exprs` 参数 + `extract_group_key` 求值化）、`src/pipeline.rs`（AggregateNode 构造点接线）、`src/parser/planner/expression.rs`（BinaryOp 臂 Interval 分流 + `Expr::Interval` 独立拒绝臂 + `try_parse_interval`/`parse_interval_expr` + `unswallow_interval_comparison` helper）、`src/parser/planner/query.rs`（`SelectItemRole` 检测收集 + 混合解锁/范围外拒绝 + `resolve_group_by_item`/`GroupKey` + group_key_exprs 编译 + 直出判定与条件包装 + output_columns 行序/SELECT 序分形）、`src/parser/ast.rs`（extract_columns/extract_qualified_columns 两放行清单 +Interval）
- 测试：新增 `tests/datetime_function_test.rs`（19 e2e：函数族 12 + INTERVAL 7）、`tests/group_by_expr_test.rs`（12 e2e：三形态等价/交错序对齐/大小写/NULL 归并/不匹配点名/纯列名零回归/直出边界/位置错误面/HAVING/ORDER BY/GROUP BY ALL）；`tests/scalar_function_test.rs` +4（I043 abs/round ×3 + I044 大小写见证）；校准 `tests/projection_expression_test.rs` 1 处 + `tests/scalar_function_test.rs` 1 处（见偏差 7）

**Deviations from Plan**

1. **eval_scalar 错误载体 ValueError→String（T6）**：ABS 溢出与 date_trunc/datediff 未知单位需点名文案，ValueError 无消息变体（契约「无新错误变体」）。改返回 `Result<Value, String>`，既有类型错误文案经 `ValueError::Display` 逐字节保留（"Type mismatch"），eval_owned 经 `Box<dyn Error>::from` 承载。错误面文案与既有断言零冲突（既有测试仅断言 is_err）。
2. **datediff 负向月份对称朝零截断（T6）**：契约公式「(d2,t2) 早于 (d1,t1) 减 1」仅覆盖正向；spec R3「b−a 整单位数、绝对值按单位截断（非四舍五入）」为验收权威 → 负向对称加一回零（2024-03-14→2024-01-15 = -1）。测试锁定负向用例。
3. **abs(i64::MIN) 见证形态（T6）**：spec GIVEN「表含 i=-9223372036854775808 行」经 SQL 字面量面不可构造（|MIN| 越 i64 上界 → 字面量解析为 Float——预存解析面；且无 PK 表首列隐式 PK 触发键列类型预检）。改以 Int 算术 `0 - 9223372036854775807 - 1`（各中间值在 i64 内）构造同值实参，经 BinaryArithExpression 进入与列值入参完全相同的 ABS Int 臂；RED 经临时回退修复实测（core num overflow panic）。
4. **I043/I044 用例函数数（T6）**：I043 四断言分布于 3 个测试函数（round 两个正超界断言合并）；I044 单函数承载三形态（UPPER/Abs 正面 + `MiXeD_Length` 未注册名错误面——spec THEN「与小写形态结果与错误面逐字节一致」：`MiXeD_Length` 全名规范化 MIXED_LENGTH 不匹配 LENGTH，SHALL NOT 子串匹配，与 `mixed_length` 拒绝文案逐字节一致）。
5. **IntervalArithExpression 落位 predicate.rs（T7）**：契约允许「predicate.rs 或 datetime.rs」——与 CASE/COALESCE/CAST/BinaryArith 表达式节点同域。
6. **sqlparser INTERVAL 吞比较解缠绕（T7，重点）**：实测 sqlparser 0.44 把紧随 INTERVAL 的比较操作吞入 `interval.value`（`d + INTERVAL '1 day' > X` 解析为 `Plus(d, Interval{value: Gt('1 day', X)})`——调查记载的「value 为字面量」形态仅在与比较连用时失真，触发契约 Stop-when「AST 形态与调查记载不符」。按「可局部适配」分支继续：build_where 比较臂新增 `unswallow_interval_comparison`——算术腿 + Interval{比较} 形态还原为「区间算术 <比较> 右侧」谓词（重建仅含字面量的 Interval 经既有 parse 路径取值，畸形仍点名拒绝）；其余形状返回 None 交回既有臂。AND/OR 组合腿经递归同样覆盖（低优先级操作符不被吞）。
7. **既有测试 2 处校准（T8，BASELINE-CHANGED）**：`projection_expression_test::aggregate_expression_error_preserved` 与 `scalar_function_test::aggregate_mixed_with_scalar_function_rejected` 锁定旧「聚合×表达式混用 → Invalid aggregate argument」拒绝面——该形态正是 group-by-expression R1 合法化的对象（语义仍拒绝未分组项，通道改为 spec R2 规定的 NonAggregatedColumn）。按 BH-1 先例校准断言至新通道；请 Plan Review 在 delta spec R2 补校准段。
8. **直出路径判定取 D12 字面准则（T8）**：直出 = 无表达式项 ∧ 全部键为列名键 ∧ SELECT 序键前聚合后；GROUP BY 序 ≠ SELECT 键序的交错形态（如 `SELECT id, dept, COUNT(*) GROUP BY dept, id`）保持直出——既有「行按 GROUP BY 序、表头按 SELECT 序」错位为既有行为（design R2 记载），测试锁定该形状；GROUP BY 含多余键（SELECT 外键）同理直出保持。
9. **GROUP BY 裸标识符未知列从静默 NULL 分组改显式错误（T8）**：D12 解析序不匹配 → NonAggregatedColumn（spec R2「SHALL 显式错误」）；既有全量无依赖该静默行为的用例（全量绿佐证）。
10. **测试规模微调**：datetime_function_test 19 e2e（契约 ~20）+ datetime.rs 5 组单测；group_by_expr_test 12（契约 ~10）；多组断言经 ORDER BY/测试内排序规避 HashMap 迭代非确定。

**Blocker Handoff**

None required。

**Blocker Resolution**

（未发生阻塞。）

**Self-Review**

- Plan compliance: T6–T8 契约逐项覆盖（REGISTRY arity/求值序 D3/严格类型/锚定语义/拒绝面/解析序/直出零触碰均按契约；Preserve/Forbidden 无违反——BinaryArithExpression 数值语义零变化、INTERVAL 不落 Value 变体、expr_to_column_name 本体未动、HAVING 既有机制未动）；偏差 1–10 全部记录。
- Full diff reviewed: 是——本 Cycle 全部 10 个源文件 + 4 个测试文件 diff 审查（含跨任务交互：T8 表达式键消费 T6 date_trunc/INTERVAL 编译产物、T7 解缠绕与 T8 文本匹配的 to_string 归一互不影响、聚合行序命名与 HAVING 绑定一致性、包装位于 Having 之上使 HAVING 绑定行序命名正确、Sort 的 sort_columns=projection_columns 与包装列名一致（别名小写化两侧对齐））。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: (a) INTERVAL 字符串形态接受单复数单位（spec 示例 '90 minutes' 要求），date_trunc/datediff 单位仅单数（DA9 字面集）——两面边界已在 parse 层分立，未统一；(b) 表达式键文本匹配为双侧 `Expr::to_string()` 精确等值——函数名书写大小写差异（SELECT `DATE_TRUNC` vs GROUP BY `date_trunc`）不匹配（D12「归一比较」取字面实现；别名/位置形态不受影响）；(c) AggregateExecutor.group_by 字段现仅承载 is_empty 判定与命名（求值面由 group_key_exprs 接管），保留字段以维持语义与未来命名消费。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T6 RED | `cargo test --test datetime_function_test --test scalar_function_test`（实施前） | datetime_function_test 12/12 FAILED；scalar I043 3 例 FAILED（abs = debug overflow panic；round = NaN→json null）；I044 见证锁实施前即绿（预期，非行为变更） | 函数族/I043/I044 目标行为缺口 | RED 观察到 |
| T7 RED | 同上（INTERVAL 段追加后） | INTERVAL 7 用例全 FAILED（`Unsupported expression type`/`Unsupported statement type`） | INTERVAL 目标行为缺口 | RED 观察到 |
| T8 RED | `cargo test --test group_by_expr_test`（实施前） | 7 用例 FAILED（混合拒绝面 `Invalid aggregate argument`）+ 5 既有形状锁绿 | 三形态/混合解锁缺口 + 直出形状基线 | RED 观察到 |
| T6/T7 GREEN | `cargo test --test datetime_function_test --test scalar_function_test --lib` | `19 passed` + `32 passed` + lib `271 passed`（含 datetime.rs 新增 5 组单测） | datetime-functions R1/R3 + I043/I044 + INTERVAL R2 + 零回归面 | PASS |
| T8 GREEN | `cargo test --test group_by_expr_test` | `12 passed; 0 failed` | group-by-expression R1/R2 全场景 | PASS |
| 全量回归 | `cargo test --no-fail-fast`（实施后共 4 次） | `passed=1027 failed=0 ignored=2`（3 次）；1 次单例假失败特征与已登记 I041（resolve env 竞态，约 1/6 假失败源）一致，复跑即绿——用户裁定先例（Iteration 000 Review F3）采信 | 987 基线 + 40 新增（19+12+4+5）；既有校准 2 处见偏差 7 | PASS |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` | 0 error / 0 diff（exit 0） | 全 workspace | PASS |
| OpenSpec | `openspec validate 2026-09-23-ms13-analytics-functions` | `Change ... is valid` | change 结构 | PASS |
| CLI 探针 | `rtsql g "SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev GROUP BY day ORDER BY day"` | `{"columns":["day","count_star"],"rows":[["2024-01-15 00:00:00",2],["2024-01-16 00:00:00",1]]}` | 契约 Verification 探针（分桶 + 包装表头） | PASS |
| CLI 探针 | `rtsql g "SELECT id, ts - INTERVAL '90 minutes' FROM ev WHERE ts - INTERVAL '90 minutes' > TIMESTAMP '2024-01-16 07:00:00'"` | `{"columns":[...],"rows":[[3,"2024-01-16 07:30:00"]]}` | INTERVAL SELECT+WHERE（解缠绕通路） | PASS |
| CLI 探针 | `rtsql g "SELECT COUNT(*), dept FROM ev GROUP BY dept ORDER BY dept"` | `{"columns":["count_star","dept"],"rows":[[2,"a"],[1,"b"]]}` | 交错序包装对齐 | PASS |

**Persisted Evidence**

None required（Mode none：全部验证可低成本重跑，无一次性环境与 Issue 现场）。

**Experience Candidates**

None。

**Remaining Issues**

I041（resolve env 测试竞态，已登记既有项）本次全量运行复现一次假失败，复跑即绿，不新增登记。

**Commit or Diff Reference**

未提交（待用户触发）；对照基线 7364bc9（工作区含 Iteration 000 实施与 MS09 收尾 docs sync 增量，均非本 Cycle 改动）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查覆盖本 Cycle 全部 10 个源文件 + 4 个测试文件的当前实现（工作区即 Iteration 001 结果态；与 HEAD=7364bc9 的 diff 含 Iteration 000 已 accepted 面与 MS09 docs 增量，按文件逐一核对 Iteration 001 变更面）。核心核对结论：

- T6：REGISTRY +10 项（`NOW(0,0)` 为 spec R1 修订的零参 carve-out）与 `eval_scalar` 新臂逐项核对——`ymd_arg`/`hms_arg`/`date_val_arg` 严格类型、`date_trunc` Date 仅 year/month/day（时间单位 TypeMismatch）、未知单位点名、`datediff` 同族先验、ABS `checked_abs` 点名溢出、ROUND `>308 → Float(x)` / `<-308 → Float(0.0)`，与契约及 DA4/DA9 一致；`eval_scalar` 错误载体 `Result<Value, String>` 下既有 `ValueError::Display` 文案（"Type mismatch"）逐字节保留（Dev 1 核实）；`eval_owned` D3 求值序未动。
- T7：`IntervalParts` 两形态解析（单复数/大小写/负数/多段拒绝/checked 溢出）、`add_months_ymd` 同日锚定截月末（闰/平年/年进位/负向单测锁定）、`add_interval_to_date/ts` 月先微秒后 + 0001..9999 范围点名拒绝、`IntervalArithExpression`（NULL 传播/Sub 求值期取负/严格日期族）、planner 数值分流**前**探测 Interval 腿（Interval 在左/双侧/乘除/独立项全部点名拒绝）、`unswallow_interval_comparison` 仅还原「算术腿 + Interval{比较}」形态且比较操作符不命中时返回 None 交回既有臂——与契约及 Dev 6 一致。
- T8：`resolve_group_by_item` D12 解析序（列名→别名大小写不敏感→SELECT 项文本双侧 `to_string`→1-based 越界校验；聚合键全通道拒绝）逐分支核对；`group_key_exprs` 列名键 = 小写名 + `column_indices` 索引（与既有 `extract_group_key` 名字查索引语义一致）；混合解锁校验（表达式项必须被某键按投影项位置匹配，等价且强于文本重匹配）；直出判定 = 无表达式项 ∧ 全键列名 ∧ SELECT 序键前聚合后（D12 字面准则）；条件包装位于 Having 之上、Sort/Limit 之下；`node_output_columns` 分形（直出 `agg_output_columns` 逐字节 / 包装行序命名供 HAVING 绑定）。`expr_to_column_name` 本体与 HEAD 逐字节一致（Forbidden 遵守）。
- 测试：datetime_function_test 19 e2e 覆盖 datetime-functions R1–R3 全场景（含 now 非递减、NULL 短路、严格类型通道区分 `Unsupported` 与运行时错误）；group_by_expr_test 12 e2e 覆盖 R1/R2 全场景（三形态逐字节等价/NULL 归并/直出边界锁定/HAVING/ORDER BY/GROUP BY ALL）；scalar_function_test +4 函数（I043×3 + I044×1，4 断言分布见 Dev 4）；两处校准带 BH-1 注记（Dev 7）。

非阻塞 finding：

- **F1（Minor，承 Iteration 000 Review F2）**：`BinaryArithExpression` doc 注释仍表述服务「datetime INTERVAL legs in Iteration 001」——其数值守卫按 D11 不承载 INTERVAL（实由 `IntervalArithExpression` 承载，本 Review 已核对分流位置）。表述误导性轻微，随本 change 收尾归档时与 Iteration 000 F1 一并作语料库注记处理，不返工。
- **F2（Minor，契约张力，Dev 8 关联）**：T8 契约 Preserve「纯列名 GROUP BY 全部既有行为（含输出行序/列名）逐字节不变」与 Required behavior 直出准则（「键前聚合后」否则包装）在「聚合项在前的纯列名形态」（`SELECT COUNT(*), dept … GROUP BY dept`）上矛盾——该形态由直出转包装，既有「行按 GROUP BY 序、表头按 SELECT 序」错位被修正为对齐（`mixed_projection_interleaved_count_first_aligns_shape` 锁定新形状）。Act 取 design D12 字面准则为权威（Required behavior 同源），修正方向正确（消除既有显示错位缺陷）、既有测试零依赖（全量绿佐证）、spec 未锁定旧错位形态——非阻塞；契约措辞矛盾归 Plan。
- **F3（Minor，测试现象，沿用既有裁定）**：Plan Review 期间独立复跑四目标套件一次全绿（见 Evidence）；全量面的 I041（resolve env 竞态）特征假失败风险沿用 Iteration 000 Review F3 用户裁定（采信多轮运行中的一致绿，单例假失败复跑即绿，不新登记）。

**Deviation Classification**

- Dev 1（eval_scalar 错误载体 ValueError→String）→ **PLAN-OMISSION**（契约「无新错误变体」未预见 ValueError 无消息变体，而 spec 要求单位/溢出点名文案；Act 以 String 载体保既有文案逐字节，最小合规路径，正确）。
- Dev 2（datediff 负向对称朝零截断）→ **PLAN-OMISSION**（契约公式仅覆盖正向；spec R3「b−a 整单位数、绝对值按单位截断」为验收权威，负向对称截断为其必然推论；Act 扩展正确并以负向用例锁定）。
- Dev 3（abs(i64::MIN) 见证经 Int 算术构造实参）→ **ACT-DEVIATION**（见证形态非产品行为：spec GIVEN 的字面量经预存解析面不可构造，Act 以 `0 - 9223372036854775807 - 1` 构造同值实参进入同一 ABS Int 臂，RED 经临时回退实测；构造推理记录于测试注释，正确）。
- Dev 4（I043/I044 用例函数数与断言分布）→ **PLAN-OMISSION**（契约按「用例」计数，Act 按断言分布为 3+1 函数；断言面与 spec 场景一一对应含 `MiXeD_Length` 未注册名错误面等价，覆盖无缺口）。
- Dev 5（IntervalArithExpression 落位 predicate.rs）→ 非偏差（契约明列「predicate.rs 或 datetime.rs」两可选项）。
- Dev 6（sqlparser INTERVAL 吞比较解缠绕）→ **NEW-EVIDENCE**（触发契约 Stop-when「AST 形态与调查记载不符」：sqlparser 0.44 将紧随 INTERVAL 的比较吞入 `interval.value`；Act 按 Stop-when 的「可局部适配」分支以 `unswallow_interval_comparison` 还原书写意图，仅命中该形态、其余形状返回 None，AND/OR 经递归覆盖；e2e WHERE 腿用例锁定。处置正确）。
- Dev 7（projection_expression_test / scalar_function_test 2 处校准）→ **BASELINE-CHANGED**（BH-1 同型：旧断言锁定「聚合×表达式混用 → Invalid aggregate argument」拒绝通道，正是 R1 合法化对象；语义仍拒绝、通道改为 spec R2 规定的 NonAggregatedChannel。已按 Act 请求在 delta spec R2 增补校准段〔本次 Review 写入，闭环〕）。
- Dev 8（直出判定取 D12 字面准则）→ **PLAN-INVALID**（T8 契约 Preserve 与 Required behavior/design D12 在「聚合在前纯列名形态」上矛盾，见 F2；Act 取 design 字面准则实现并记录、测试锁定新旧两形状，歧义消解正确、方向为缺陷修正）。
- Dev 9（GROUP BY 裸标识符未知列从静默 NULL 分组改显式错误）→ **ACT-DEVIATION**（非阻塞：spec R2「无法解析 SHALL 显式错误」直接要求该行为，旧静默 NULL 分组为缺陷面；既有全量无依赖用例〔全量绿佐证〕）。
- Dev 10（测试规模微调 19/12 vs ~20/~10）→ 非偏差（契约为近似量级；场景覆盖经本 Review 逐条核对无缺口）。

**Acceptance Gaps**

None。datetime-functions R1–R4 + group-by-expression R1–R2 + sql-scalar-functions 修改 R1/R3 全部场景有对应见证（19 + 12 e2e + scalar 4 新函数 + 全量绿）；T6–T8 契约 Targets/Required behavior 逐项核对实现无缺失；Preserve/Forbidden（既有十函数文案逐字节、求值序 D3、`BinaryArithExpression` 数值语义零变化、INTERVAL 不落 Value 变体、`expr_to_column_name` 本体未动、HAVING 机制未动、纯列名键前聚合后直出逐字节）经 diff 审查确认遵守——Preserve 的唯一张力点见 F2/Dev 8（已裁定非阻塞）。

**Convergence**

N/A（首次 Review；无既有 gap 比较）。

**Evidence**

- 代码独立审查：本 Review 逐文件读取（function.rs 全读——REGISTRY/arity/求值序/ABS/ROUND 逐行核对；datetime.rs 全读——`parse_interval_string`/`add_months_ymd` 锚定/`datediff` 对称朝零/范围校验/5 组新单测核对；predicate.rs `BinaryArithExpression`/`IntervalArithExpression` 节点与 evaluate_ref Copy 直回；expression.rs BinaryOp 分流（Interval 探测先于数值臂）/独立 Interval 点名拒绝/`unswallow_interval_comparison` 重建仅字面量 Interval；query.rs `resolve_group_by_item` 四分支/`group_key_exprs` 编译/直出判定/条件包装/HAVING 绑定/`node_output_columns` 分形；aggregate.rs `extract_group_key` 求值化与 ExecutionError；pipeline.rs AggregateNode 接线；ast.rs 两放行清单 +Interval；`expr_to_column_name` 与 HEAD 逐字节比对）。
- 独立复跑（本次 Review，exit 0）：`cargo test --test datetime_function_test --test group_by_expr_test --test scalar_function_test --test datetime_type_test` → `19 + 16 + 12 + 32 passed; 0 failed`（scalar 32 = 28 既有 + 4 新函数；datetime_type 16 为 Iteration 000 套件回归确认）。
- 采信（覆盖范围未失效，git 基线检查一致——无新 commit、工作区即 Act 报告状态）：Act Response Verification Evidence 表——全量 `cargo test --no-fail-fast` 4 次运行 3 次 `passed=1027 failed=0 ignored=2`（987 基线 + 40 新增，既有校准 2 处见 Dev 7）、clippy `--all-targets -D warnings` / `fmt --check` 0、`openspec validate` PASS、三组 CLI 探针（分桶包装表头/INTERVAL WHERE 解缠绕通路/交错序对齐）。
- spec 校准：group-by-expression R2 校准段已写入（Dev 7 闭环）。

**Follow-up Decision**

Acceptance 全满足、无阻塞 finding、十项偏差全部非阻塞且闭环（Dev 7 校准段本次 Review 写入；F1/F2 为归档期注记）——接受本 Cycle，Iteration 001 完成。I041 竞态为既有登记项，不在本 change 范围。

**Iteration Plan Update**

None（Iteration Map 不变；change 级 3 Iteration 维持）。

**Next Cycle**

None（无 rework/replan）。

**Next Iteration**

`../002-nofrom-analytics-commands/000-initial.md`（已按 Map 展开，Status: draft 待 Gate 2 用户批准）
