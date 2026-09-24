# MS13 分析函数：DATE/TIMESTAMP 类型 + 日期函数 + GROUP BY 表达式 + no-FROM SELECT + stats/sample/profile 薄命令

## Why

tasks.md MS13（执行序下一站，2026-09-14 用户裁定 MS08 剥离后）定义分析能力单一成果，三个 Task：

1. **MS13-T01 日期/时间类型**——分析最高频维度（R18 主题 7 排序第 4）依赖类型系统扩展（`tuple.rs` 格式变更，深水区）：当前 `CREATE TABLE t (d DATE)` 经 `convert_data_type` 未知类型回退 String（`src/parser/planner/ddl_dml.rs:276` `_ => ColumnType::String`），无真日期类型——比较靠字典序、无截断/抽取函数底座。
2. **MS13-T02 日期函数 + 分析薄命令**——date_trunc/interval/now 等（R18 主题 7）+ `stats/sample/profile` 薄命令（底层聚合/扫描 SQL）；随带 I043（abs 溢出/round 极端 digits，语义方向届时裁定）、I044（函数名大小写不敏感缺 SQL 层见证）。
3. **MS13-T03 I035 no-FROM SELECT**——`SELECT 1` 当前报 `Plan error: Missing required field: FROM clause`（`src/parser/planner/query.rs:123`、`src/parser/ast.rs:23`），常量表达式查询与无表探测不可达（improvements I035，已排期本项）。

底子就绪（R18 主题 7）：5 聚合 + GROUP BY（仅列名）+ HAVING + ORDER + LIMIT + 标量函数注册机制（MS11-T03）+ 表达式四件套（MS11-T01）；本 change 补齐类型底座 + 函数层 + 分桶能力 + 便利命令。

## 用户决策（2026-09-23 Gate 1 前裁定）

1. **类型范围**：DATE + TIMESTAMP 双类型（DATE=i32 天数、TIMESTAMP=i64 微秒，无时区）两个新 Value 变体 + 新 tuple tag；INTERVAL 只做表达式构造、不可作列类型存储。
2. **字面量策略**：写入强制解析 + 比较严格——INSERT/UPDATE 把 String 字面量写入 DATE/TIMESTAMP 列时强制解析（非法值显式报错，PostgreSQL unknown-literal 先例；CSV import 依赖此通路）；WHERE 比较保持严格同类型（需 `DATE '...'` 类型字面量或 CAST）。
3. **函数与分桶**：核心函数集 + GROUP BY 表达式/别名扩展——函数：now()/date()/year/month/day/hour/minute/second()/date_trunc(unit,x)/INTERVAL 算术（`d ± INTERVAL`）/datediff；GROUP BY 接受与 SELECT 项匹配的表达式或别名（含 `GROUP BY 1` 位置引用），`GROUP BY date_trunc('day',ts)` 分桶可达。
4. **分析命令**：三命令一次做全——stats=总行数+每列 null率/distinct/min/max/p50/p90/p99；sample=随机 N 行（默认 10，CLI 侧抽样）；profile=每列类型+min/max+top-k 高频值；全部走既有查询路径拉数后 CLI 计算，不动引擎。

## What Changes

1. **DATE/TIMESTAMP 值类型与存储（T01）**——`Value`/`ValueRef` 新增 `Date(i32)`（儒略历天数，0001-01-01 起算）与 `Timestamp(i64)`（Unix epoch 微秒）变体；`tuple.rs` 新 tag 0x06/0x07（owned + 零拷贝两套 deserialize）；catalog `COL_TAG` 持久化扩展；DDL `convert_data_type` 映射 `DataType::Date/Datetime/Timestamp`（未知类型回退 String 的既有兜底收窄为显式映射）；类型字面量 `DATE '...'`/`TIMESTAMP '...'`（sqlparser `Expr::TypedString`）plan 期解析校验；写入边界强制解析（String 字面量 → Date/Timestamp 列）；比较/Hash/Display/`to_key`（→None，不可键控）；CAST 矩阵扩展（String↔Date/Timestamp 解析与格式化，数值跨族保持拒绝）；json/CLI 渲染；dump（类型化字面量）/restore/csv 面扩展；WAL 以 `tuple_data` 不透明字节流过（零改动）。
2. **日期函数与 INTERVAL 算术（T02 引擎侧）**——`function.rs` REGISTRY + 日期函数族：now()/date()/year/month/day/hour/minute/second()/date_trunc(unit,x)/datediff(unit,a,b)；`Expr::Interval` 表达式构造（`d + INTERVAL '1 day'`、`ts - INTERVAL '90 minutes'`，单位 year/month/day/hour/minute/second，月份算术按同日锚定、溢出截月末）；INTERVAL 作列类型显式拒绝。
3. **GROUP BY 表达式/别名/位置（T02 引擎侧）**——`group_by` 项除列名外接受：SELECT 别名、与 SELECT 项文本等价的表达式、`GROUP BY <1-based 位置>`；分桶键经表达式求值（复用投影项编译）。
4. **I043/I044 随带（T02）**——abs(i64::MIN) 改 `checked_abs` 显式溢出错误（消除 debug panic/release 回绕）；round 极端 digits SQLite 对齐饱和（|digits| 超界：正超界返回原值 Float、负超界返回 0.0）；`sql-scalar-functions` 大小写不敏感补 SQL 层 e2e 见证（大写/混合变体用例）。
5. **no-FROM SELECT（T03，I035）**——planner 虚拟单行输入臂：`SELECT <表达式项>` 无 FROM 产出单行（常量、标量函数、CASE/COALESCE/CAST 组合可达）；表头 = 表达式文本/别名；通配符/WHERE/ORDER BY/LIMIT/GROUP BY/聚合在 no-FROM 形态显式拒绝（点名文案）。
6. **stats/sample/profile 薄命令（T02 CLI 侧）**——`rtsql stats <db> <table>`、`rtsql sample <db> <table> [N]`、`rtsql profile <db> <table>`；CLI 侧全量拉取计算（分位数最近邻秩，仅数值列；top-k 限 k≤20 默认 5）；输出遵循 `--format` 四态与既有退出码分类。

Delta specs（草案，随实现调查定稿）：

- 新增 `datetime-type-system`：类型与存储格式 + DDL/字面量/写入强制 + 比较/键控边界 + CAST + 渲染/导入导出 + 恢复两态 + 既有语义零回归。
- 新增 `datetime-functions`：函数族语义 + INTERVAL 算术 + NULL/类型错误面 + 既有函数零回归。
- 新增 `group-by-expression`：表达式/别名/位置分桶 + 既有列名分组零回归。
- 新增 `no-from-select`：虚拟单行可达 + 拒绝面 + 既有 FROM 形态零回归。
- 新增 `cli-analytics-commands`：三命令输出契约 + 格式四态 + 错误面。
- 修改 `sql-scalar-functions`：R3 math 边界（I043 饱和/溢出语义锁定）+ R1 大小写见证（I044 场景补齐）。

## BDD 场景草图（缺口扫描结论）

覆盖面按 delta spec 场景落定；要点：

- **Happy**：建表落列 → 类型化 INSERT → SELECT 等值回读（Display `YYYY-MM-DD`）；日期函数抽取/截断/区间算术正确；`GROUP BY date_trunc(...)`/别名/位置三分桶等价；`SELECT 1+1`→`[[2]]`；stats 输出分位数。
- **Sad**：非法日期字符串（写入/CAST/date_trunc 单位不识别）显式错误且零副作用；Date vs String 比较类型错误；INTERVAL 作列类型拒绝；no-FROM 下 `*`/WHERE/聚合点名拒绝；表不存在 → 既有错误码。
- **Edge**：闰日 2024-02-29 合法 / 2023-02-29 拒绝；日期范围 0001-01-01..9999-12-31 越界拒绝；TIMESTAMP 微秒精度往返无损；abs(i64::MIN) 显式错误；空表 stats（null率/分位数边界）；偶数行 p50 双值平均。
- **兼容**：既有 936 测试零修改通过；`CREATE TABLE ... (d DATE)` 从「回退 String」变为真 Date 列（行为变化点，已知且预期——既有测试无 DATE DDL 用例，调查确认）；dump/restore 多代恒等扩展到含日期列库。

## Out of Scope / Non-goals

- 时区/时区感知时间戳（TIMESTAMPTZ）、TIME 类型、DATE/TIMESTAMP 精度修饰（`TIMESTAMP(3)`）。
- INTERVAL 作可存储列类型；`INTERVAL '1-2' YEAR TO MONTH` 复合形式。
- strftime/date_format/to_date 格式化解析函数族（后续批次）。
- 窗口函数（远期）；UDF；聚合扩展（percentile SQL 内建）。
- Date/Timestamp 可键控（B-tree 键扩展）——非 Int 键列走 MS16 路由回退，B-tree 零改动。
- REPL；分发（MS14，时点待裁定）；加密（MS12）。
- 时间类型上的 SUM/AVG 聚合（MIN/MAX 支持，SUM/AVG 保持 Null 语义）。

## 默认假设（用户未显式裁定，按合理默认补齐，可否决）

- **DA1** TIMESTAMP 无时区语义（存取即字面量时刻）；`now()` 取系统墙钟（`SystemTime`），返回 TIMESTAMP。
- **DA2** Date/Timestamp `to_key()` 返回 None（不可键控）；Date 列作 PRIMARY KEY 走 MS16 `pk_type_known_non_int` 路由回退 DataScan（keyless 行语义照常）。
- **DA3** no-FROM 形态仅支持表达式项/标量函数；WHERE/ORDER BY/LIMIT/GROUP BY/聚合/通配符在该形态显式拒绝（点名文案，exit 3）。
- **DA4** I043 语义：`abs(i64::MIN)` → 显式溢出错误（`checked_abs`，与严格类型哲学一致）；`round(x, digits)` |digits|>308 → digits 正超界返回原值 Float、负超界返回 0.0（SQLite `round(1,1000)=1.0`/`round(1,-1000)=0.0` 对齐）。
- **DA5** 显示与 JSON：Date `YYYY-MM-DD`；Timestamp `YYYY-MM-DD HH:MM:SS`（微秒≠0 追加 6 位小数）；json/CSV/TSV 均按上述字符串；`Value::Display` 同格式（不带引号差异沿用 String 现状）。
- **DA6** CSV import：DATE/TIMESTAMP 列字段按字符串读取，经写入强制解析通路落类型（非法值报错）；dump `sql_literal` 输出类型化字面量 `DATE '...'`/`TIMESTAMP '...'`，`create_table_sql` 渲染 `DATE`/`TIMESTAMP` 类型。
- **DA7** GROUP BY 匹配顺序：SELECT 别名 → 表达式文本等价（规范化比较）→ 位置引用；列名匹配保持既有优先语义；`GROUP BY ALL` 既有语义保持。
- **DA8** INTERVAL 单位集 year/month/day/hour/minute/second；month/year 算术按同日锚定（1-31 + 1 month = 2-28/29 溢出截月末，PostgreSQL 语义）；`datediff(unit, a, b)` 返回 b−a 的整单位数（截断）。
- **DA9** 函数严格类型：year/month/day 接受 Date/Timestamp；hour/minute/second 接受 Timestamp（Date 视为零点）；date(x) Date 恒等、Timestamp 截断到日；date_trunc 单位 year/month/day/hour/minute/second；单位参数为 String 字面量（大小写不敏感）。
- **DA10** stats 分位数 = 最近邻秩（nearest-rank，p50 偶数取双值平均）；分位数仅数值列（Int/Float）输出，非数值列该格 null；min/max 全类型（含 Date/Timestamp/String/Bool）；distinct 精确计数（HashSet）；top-k 仅 profile、k 默认 5 上限 20、仅 String 列（数值列 top-k 无分析价值，从缺）。
- **DA11** sample 抽样 = 全量拉取后 reservoir sampling（无引擎 RANDOM 支持）；N 默认 10，显式 0 报用法错（exit 2）。
