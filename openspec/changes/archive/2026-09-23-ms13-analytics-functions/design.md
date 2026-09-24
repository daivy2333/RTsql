# MS13 分析函数 — Design

## 调查基线

- Revision `7364bc9`（工作区 docs-only 增量，无代码改动）；936 tests 基线；本设计全部 Current-State Evidence 来自 2026-09-23 实读 + 二进制探针（见 proposal 与 Cycle Investigation Facts）。
- 关键探针结论（修正了一处规划假设）：`SELECT 1 + 1` **含 FROM 形态今天也不可达**——`ast.rs extract_columns` 的放行清单不含 `BinaryOp`（`Unsupported statement type`，探针实证）；而 planner 侧 `build_expression` 已有 BinaryOp 算术臂（`expression.rs:495`）。解锁 ast.rs 放行后，WITH-FORM 算术表达式项随之可用（同一通路，非 no-FROM 专属）。

## 决策

### D1 Value 表示：Date(i32 天) / Timestamp(i64 微秒)，无新依赖

- `Value::Date(i32)` = 自 0001-01-01（proleptic Gregorian）起的天数；解析期日历范围校验 0001-01-01..9999-12-31（i32 上界 ~3.65M 天，恒安全）。
- `Value::Timestamp(i64)` = Unix epoch 微秒；解析期同日历范围校验。
- 日历数学不引入 chrono/time 依赖：新增 `src/executor/datetime.rs` 纯函数模块（Howard Hinnant civil 算法 days_from_civil / civil_from_days + 字段抽取/截断/区间算术），穷举单测（闰年/月末/往返/边界年）锁定。依据：项目轻量依赖纪律（SNAPSHOT 技术栈无日期库），算法 ~100 行公有领域成熟实现。
- `ValueRef::Date(i32)` / `ValueRef::Timestamp(i64)`（Copy，零拷贝友好）。

### D2 tuple tag 与 catalog COL_TAG

- `TAG_DATE = 0x06`（4B LE 天数）、`TAG_TIMESTAMP = 0x07`（8B LE 微秒）——紧跟既有 0x01–0x05，负载定长。
- `compute_tuple_size` / `serialize_tuple` / `deserialize_tuple` / `deserialize_value_refs` 四点同步扩展（owned 与零拷贝语义一致）。
- `catalog.rs` `COL_TAG_DATE = 0x05`、`COL_TAG_TIMESTAMP = 0x06`（现值 0x01–0x04 之后的首两个空位）。
- WAL `tuple_data: Vec<u8>` 不透明字节流，新 tag 透明流过；恢复侧 `to_key()==None` 自动落入既有 keyless 桶（`recovery.rs:203` 实证），运行期/恢复两态一致零额外改动。

### D3 双 ColumnType 枚举同步扩展 + DDL 显式映射

- `executor::value::ColumnType` 与 `storage::page_format::ColumnType` 各加 `Date`、`Timestamp` 变体（无参数）。
- `convert_data_type`（ddl_dml.rs:276）：`DataType::Date → Date`；`DataType::Datetime(_) | DataType::Timestamp(_, TimezoneInfo::None) → Timestamp`；`TimestampTz`/带时区变体与 `DataType::Time`、`DataType::Interval` 显式拒绝（点名文案）；**未知类型回退 String 的兜底保持**（仅收窄 DATE/DATETIME/TIMESTAMP 三族不再落入）。
- `to_schema_column`（plan.rs:198）补两臂；`create_table_sql`（lifecycle.rs:567）渲染 `DATE` / `TIMESTAMP`。

### D4 字面量解析（datetime.rs 单点）

- 接受格式：Date `YYYY-MM-DD`；Timestamp `YYYY-MM-DD[ T]HH:MM:SS[.f{1,6}]`（纯日期 → 零点；f >6 位拒绝；时区后缀 Z/±HH 拒绝——无时区语义，DA1）。
- 解析失败返回 `None`；调用面转具名错误（见 D5/D6）。
- 输出格式（Display/JSON/dump）：Date `YYYY-MM-DD`；Timestamp `YYYY-MM-DD HH:MM:SS`（微秒 ≠0 追 `.{:06}`，DA5）。

### D5 类型字面量（TypedString）四个接线点

`Expr::TypedString { data_type, value }` 新臂（解析失败 → `PlanError::ParseError` 点名「invalid DATE/TIMESTAMP literal」）：

1. `build_expression`（expression.rs:170，SELECT/值表达式入口）
2. `build_where`（expression.rs:489，谓词入口；`WHERE d = DATE '...'` 的右腿）
3. `extract_insert_values`（ddl_dml.rs:219，`INSERT VALUES (DATE '...')`）
4. UPDATE SET 值提取（ddl_dml.rs:515，`SET d = DATE '...'`）

`ast.rs extract_columns` 放行清单补 `TypedString` 与 `BinaryOp`（含 `Expr::Interval` 腿的 SELECT 项随 BinaryOp 放行；BinaryOp 解锁同时使 WITH-FORM 算术项可达——行为扩展记入 spec 场景）。

### D6 写入边界强制解析（Insert/Update 执行器）

- InsertExecutor 序列化前逐列检查：目标列类型 `Date`/`Timestamp` 且值为 `Value::String` → `datetime::parse_*` 强制转换；失败 → 新 `StorageError::InvalidDateTime { value, expected }`（文案含原值与期望格式；CLI 经 SQL 失败面 exit 3）。
- 同列值既非 `Date`/`Timestamp`（类型化字面量）也非 `String`（强制）也非 `Null` → 同错误（防 tag/schema 漂移；比 MS16 仅键列检查扩到日期列，理由：日期列无既有落库面，首落地即收口）。
- UpdateExecutor SET 新值同规则（UpdateNode 携带 column + new_value；执行器有 table_meta schema）。
- CSV import：`csv_value` Date/Timestamp 臂——空字段 → NULL；非空 → String 透传（写入强制解析兜底校验）。

### D7 比较 / Hash / 键控 / 聚合 / 排序

- `equals`：Date×Date、Timestamp×Timestamp 同型比较；跨族（含 Date×Timestamp）→ `false`（与既有 String×Int 同型兜底）。
- `gt/lt/ge/le`：同型臂；跨族 → `Err(TypeMismatch)`（既有兜底行为）。
- `Hash` 补两臂；JOIN `right_hashmap: HashMap<Vec<Value>, _>` 与 GROUP BY 分桶自动生效。
- `to_key()` → `None`（两类型不可键控）；Date PK 走 MS16 `pk_type_known_non_int` 路由回退 DataScan（双保险：TypedString 腿也不满足 `has_pk_equality` 的 Value 腿结构探测）。
- `lt_agg`（MIN/MAX）补两臂；`add`/`div`（SUM/AVG）走既有 `_ => Null` 兜底（Date 无意义聚合 = Null，提案 Non-goal 已记）。
- `compare_values`（sort.rs:103）补 Date×Date / Timestamp×Timestamp 两臂——`_ => Ordering::Equal` 兜底会吞掉日期排序，必须显式。

### D8 CAST 矩阵扩展

- `CastType` 增 `Date`、`Timestamp`；planner CAST 目标类型映射补 `DataType::Date/Datetime/Timestamp` 臂。
- 新转换：String→Date/Timestamp（D4 解析，失败 `TypeMismatch`）；Date/Timestamp→String（D4 格式化）；Timestamp→Date（截断）；Date→Timestamp（零点）。其余跨族（数值/Bool×日期）保持拒绝。
- `evaluate_ref`：Date/Timestamp 是 Copy → 直接回 `ValueRef`（区别于 String 结果报错的三先例）。

### D9 渲染面

- `Value::Display` 按 D4；`value_to_json`（pipeline.rs:787）→ `json::Value::String(格式化文本)`；CLI 四格式与 `QueryPayload::Rows` 复用，render 零改动。
- dump：`sql_literal` 增类型化路径——dump 循环内已有 catalog 列（lifecycle.rs:162），按列类型对 Date/Timestamp 值输出 `DATE '...'` / `TIMESTAMP '...'`（DA6；restore 侧经 D5 接线解析，多代恒等）。

### D10 日期函数（function.rs REGISTRY 扩展）

- 注册：`NOW(0,0)`、`DATE(1,1)`、`YEAR/MONTH/DAY(1,1)`、`HOUR/MINUTE/SECOND(1,1)`、`DATE_TRUNC(2,2)`、`DATEDIFF(3,3)`。零参合法：`check_scalar_function` 的 min=0 天然放行（R1 修订的 carve-out）。
- 参数严格类型（datetime.rs helper）：`date_arg`（Date|Timestamp）、year/month/day 用 date_arg；hour/minute/second 用 ts_arg（Timestamp 直取 / Date 视为零点，DA9）；date(x)：Date 恒等 / Timestamp 截断；now()：`SystemTime` 墙钟微秒（每次求值取值，单行语义即语句时点，DA1）。
- date_trunc：unit 参数为 String 值（大小写不敏感匹配 year/month/day/hour/minute/second；未知 → ValueError 点名 unit）；Date 支持 year/month/day（hour+ 对 Date → 错误），Timestamp 支持全单位。实现 = 字段清零（纯 civil 数学）。
- datediff(unit, a, b)：同族 a,b；day/hour/minute/second = (b−a) 微秒差整除截断；month/year = 日历差（(y2−y1)×12+(m2−m1)，日/时余量不足减 1——PostgreSQL age 语义方向，DA8 截断）。

### D11 INTERVAL 表达式算术（datetime.rs）

- `Expr::Interval` 接线（build_expression + build_where 双入口）：支持 `INTERVAL 'N unit'`（字符串 + 内嵌单位，leading_field=None）与 `INTERVAL N UNIT`（leading_field=Some）；多字段形态（last_field / `YEAR TO MONTH` / fractional precision）显式拒绝；单位 year/month/day/hour/minute/second。
- 内部表示 `IntervalParts { months: i32, micros: i64 }`（y/m 入 months，d/h/m/s 入 micros）——非 Value 变体、不可存储。
- 算术节点 `IntervalArithExpression { left: ExpressionRef, op: Plus|Minus, interval: IntervalParts }`：left 为 Date → Date；Timestamp → Timestamp。月份算术同日锚定、溢出截月末（DA8）；微秒算术纯整数加减后日历范围校验。`Interval ± Interval`、`Interval ± Date`（Interval 在左）、独立投影项（`SELECT INTERVAL '1 day'`）显式拒绝。
- BinaryOp 构建点识别：一侧为 Interval 字面量且另一侧为 Date/Timestamp 表达式 → IntervalArithExpression；普通 BinaryOp 语义零变化。

### D12 GROUP BY 表达式/别名/位置

- 解析序（每个 GROUP BY 项）：列名（既有）→ SELECT 别名 → SELECT 项表达式文本等价（双侧 `Expr::to_string()` 归一比较）→ 位置（`Expr::Value(Number 1-based)`）。全部不匹配 → `NonAggregatedColumn(项文本)` 显式错误。
- `AggregateNode` 增 `group_key_exprs: Vec<ExpressionRef>`：列名项编译为 `ColumnExpression`（column_indices 已知索引，语义与现 `extract_group_key` 逐字节一致）；表达式项经 `build_expression` 编译。`extract_group_key` 改为求值 exprs（`group_by: Vec<String>` 保留作输出命名）。
- **混合投影解锁**（现状 `query.rs:471` 拒绝）：表达式项 + 聚合并存时，每个非聚合 SELECT 项必须解析到某 GROUP BY 键（别名/文本/位置），否则 `NonAggregatedColumn`；纯列名项保持既有 `group_by.contains` 检查。
- 输出行装配：聚合输出 = group_key（GROUP BY 序）++ aggregates（投影序）。SELECT 序恰为「全部键项在前、聚合项在后」且全部键为纯列名 → 既有直出路径零变化；否则（表达式项或交错序）在 Aggregate/Having 之上包一层 `ProjectionNode`：键项 → 该键位置的 `ColumnExpression`，聚合项 → k+聚合序 `ColumnExpression`，列名 = SELECT 名（别名/表达式文本）。Sort/Limit 叠加在 Projection 之上（输出列名 = 投影名，`ORDER BY 别名` 经既有名字查找生效）。
- `GROUP BY ALL` 既有语义保持（非聚合列清单不变）。
- Non-goals：`ORDER BY <表达式文本>`（别名已覆盖主用法，不动 ORDER BY 名字提取面）。

### D13 no-FROM SELECT（虚拟单行节点）

- 新 `PhysicalPlan::SingleRow(SingleRowNode)`（无字段）+ `SingleRowExecutor`：恰产出一行空行 `vec![]`。
- planner `build_query`：`select.from.is_empty()` 时——表达式项/值字面量/标量函数/CASE/COALESCE/CAST/TypedString/算术项经 `build_expression` 编译为顶层 ProjectionNode（复用 has_expression_items 通路，输入 = SingleRow）；通配符（无列可通配）、WHERE、GROUP BY（含任何聚合项）、HAVING、ORDER BY、LIMIT 显式拒绝（`PlanError::ParseError` 点名「not supported without FROM」）。
- 接线面（NLJ 同型五点）：`create_executor_from_plan` 读分发两处（execute/execute_in_tx）、`get_plan_output_columns`（→ `vec![]`）、`extract_column_indices`（pipeline.rs:806， unreachable 但补臂）、表达式编译对空 schema 行求值（ColumnExpression 不可达——列引用在编译期 ColumnNotFound）。
- `is_plain_column_expr` 等检测循环不改（no-FROM 下标识符项经 build_expression 报 ColumnNotFound，符合 spec 拒绝语义）。

### D14 stats / sample / profile CLI（lifecycle.rs 同型新函数）

- `stats <db> <table>`：`SELECT *` 单次拉取（select_all_rows 同型，错误经 sql_failure_status → exit 3），CLI 侧计算：行数、每列 null 率/distinct(HashSet)/min/max（lt_agg 语义复用 Value 比较）/p50/p90/p99（数值列升序最近邻秩，p50 偶数双值平均；非数值列 null）。输出行 = 每列一行 `[column, type, null_rate, distinct, min, max, p50, p90, p99]`，经既有 `render` 四态。
- `sample <db> <table> [N]`：同拉取后 reservoir sampling（rand 0.8 已依赖）；N 默认 10；`0` 或非整数 → clap 用法错（exit 2）。输出 = 行集原样 render（列 = 表列）。
- `profile <db> <table>`：每列 `[column, type, min, max, top_k]`；top_k 仅 String 列（k 默认 5、`--top` 上限 20），频次降序、并列字典序，格式 `val(cnt), val(cnt)...`；多次执行输出确定。
- 三命令经 `execute_command_inner`（信号优雅停机复用）；表不存在 → SQL 失败面 exit 3；锁冲突 exit 4 既有映射复用。

### D15 I043 / I044

- `ABS` 臂 Int 分支改 `checked_abs().ok_or(ValueError::...)`——溢出显式运行时错误（i64::MIN；debug panic / release 回绕消除）。Float 分支不变。
- `ROUND` digits 先按现规则取整，再加边界卫兵：`digits > 308 → 返回 x 的 Float 形态`；`digits < -308 → 返回 0.0`（SQLite 对齐，spec 场景锁定）。
- I044：`tests/scalar_function_test.rs` 增大写/混合变体 e2e（UPPER/Abs/MiXeD_Length 至少三形）。

### D16 测试策略

- 新增 `tests/datetime_type_test.rs`（类型域：DDL/字面量/写入强制/比较/排序/PK 路由/恢复/dump-restore 恒等/CSV，~25 用例）、`tests/datetime_function_test.rs`（函数与 INTERVAL 与 date_trunc 分桶 e2e，~20）、`tests/group_by_expr_test.rs`（别名/表达式/位置/交错序/错误面/零回归，~10）、`tests/no_from_select_test.rs`（单行/拒绝面/含 FROM 算术解锁，~8）、`tests/cli_test.rs` 增三命令用例（~12）、`scalar_function_test.rs` 增 I043/I044（~6）；datetime.rs 单元测试（闰年/月末/往返/边界，~20）。
- 全量回归零修改为硬 Gate（936 基线 + 新增）；clippy/fmt/validate 全 0。

## 风险与既知边界

- **R1 日期算术正确性**：手写日历数学是本 change 最大正确性面——单测穷举（闰年 400 年规则、月末锚定、负区间、边界年）+ e2e 锚点用例（探针可对照 PostgreSQL 已知值）。
- **R2 GROUP BY 聚合输出形状**：既有「group 序 ++ 聚合序」直出路径与 SELECT 序的潜在错位是既有行为（非本 change 引入）；新包装路径只在表达式/交错形态激活，纯列名形态零触碰。交错形态若实现中发现 Sort/Having 叠加面超预期，回退裁剪「键项在前」约束需 replan（记入风险，不作 TBD）。
- **R3 全量拉取 stats 的大表性能**：已裁定（决策 4）CLI 侧计算；无上限拉取，文档化边界。
- **R4 ast.rs BinaryOp 放行的波及面**：WITH-FORM `SELECT id+1 FROM t` 从错误变为可达——表达式项既有拒绝面（子查询混用/JOIN 混用/通配混用）对算术项同步生效（同一 has_expression_items 路由），无新增静默面。
