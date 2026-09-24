# datetime-type-system Specification

## Purpose

约束 DATE/TIMESTAMP 值类型的存储、DDL、字面量、写入边界与消费面语义：两类型 SHALL 作为一等 Value 变体经 tuple tag 持久化，DDL SHALL 显式映射不再回退 String，写入边界 SHALL 对 String 字面量强制解析，比较 SHALL 严格同类型，既有五类型语义 SHALL 零回归。来源：MS13-T01（change `2026-09-23-ms13-analytics-functions`）。

## ADDED Requirements

### Requirement: DATE/TIMESTAMP 值与存储格式

`Value`/`ValueRef` SHALL 支持 `Date(i32)`（自 0001-01-01 起的天数）与 `Timestamp(i64)`（Unix epoch 微秒）变体；tuple 序列化 SHALL 使用新 tag（Date 4 字节负载 / Timestamp 8 字节负载），owned 与零拷贝两套反序列化路径 SHALL 语义一致；catalog SHALL 持久化两类型的列类型描述（COL_TAG 扩展）。

#### Scenario: 序列化往返无损

- **GIVEN** Date 值 `2024-02-29` 与 Timestamp 值 `2024-01-15 10:30:00.123456`
- **WHEN** 经 serialize/deserialize（owned 与零拷贝两路径）往返
- **THEN** 值逐字节等值回读（微秒精度无损）

#### Scenario: 截断与损坏拒绝

- **GIVEN** tag 后负载字节数不足或 tag 未知
- **WHEN** 反序列化
- **THEN** 显式 StorageError（与既有五类型 malformed 拒绝同型）

### Requirement: DDL 显式类型映射

`convert_data_type` SHALL 将 `DataType::Date` 映射为 Date、`DataType::Timestamp`/`DataType::Datetime` 映射为 Timestamp；两类型 SHALL 经全链（executor ColumnType → storage ColumnType → catalog）持久化并经 schema 命令可见；INTERVAL 作列类型 SHALL 显式拒绝。

#### Scenario: 建表落列

- **GIVEN** `CREATE TABLE t (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)`
- **WHEN** 建表后 `schema` 查看输出
- **THEN** 列类型显示 DATE/TIMESTAMP（不再回退 STRING）

#### Scenario: INTERVAL 列类型拒绝

- **GIVEN** `CREATE TABLE t (i INTERVAL)`
- **WHEN** 执行
- **THEN** 显式拒绝（点名文案，exit 3）

### Requirement: 类型字面量与写入边界强制解析

`DATE '...'`/`TIMESTAMP '...'` 类型字面量（sqlparser `Expr::TypedString`）SHALL 在 plan 期解析为对应 Value（非法格式显式拒绝）；INSERT/UPDATE 把 String 字面量写入 DATE/TIMESTAMP 列时 SHALL 强制解析为对应类型值（非法值显式错误、零副作用）；两类型经 WAL 记录与恢复重放后 SHALL 等值回读。

#### Scenario: 类型字面量写入回读

- **GIVEN** 表含 DATE 列
- **WHEN** `INSERT INTO t VALUES (1, DATE '2024-01-15')`
- **THEN** 落库为 Date；`SELECT d FROM t` 等值回读，显示 `2024-01-15`

#### Scenario: 裸字符串强制解析

- **GIVEN** 表含 DATE 列
- **WHEN** `INSERT INTO t VALUES (1, '2024-01-15')`
- **THEN** 强制解析成功，落库为 Date（与类型字面量等价）

#### Scenario: 非法日期字符串拒绝

- **GIVEN** 表含 DATE 列
- **WHEN** `INSERT INTO t VALUES (1, 'not-a-date')` 或 `'2023-02-29'`
- **THEN** 显式解析错误（exit 3），零行落库

#### Scenario: 恢复两态一致

- **GIVEN** 含 Date/Timestamp 值的库经写入后 close
- **WHEN** 重开（含 WAL 重放路径）
- **THEN** 值等值回读（运行期与恢复后两态一致）

### Requirement: 比较与键控边界

Date 与 Date、Timestamp 与 Timestamp SHALL 按时间序比较（gt/lt/ge/le/equals）；Date 与 Timestamp、日期与 String/数值 SHALL 显式类型错误（严格同类型，无跨族隐式转换）；两类型 SHALL 参与 Hash（GROUP BY/JOIN 键合法成分）与 MIN/MAX；`to_key()` SHALL 返回 None（不可键控），Date 列作 PRIMARY KEY 时 SHALL 走既有非 Int 键列路由回退（MS16 语义），SUM/AVG 对两类型 SHALL 保持既有 Null 语义。

#### Scenario: 时间序过滤与排序

- **GIVEN** DATE 列含 `2024-01-15`、`2024-02-29`、`2023-12-31`
- **WHEN** `WHERE d > DATE '2024-01-01'` 与 `ORDER BY d`
- **THEN** 过滤与排序按时间序正确

#### Scenario: 跨类型比较拒绝

- **GIVEN** DATE 列 d
- **WHEN** `WHERE d = '2024-01-15'`（String 字面量侧）
- **THEN** 显式类型错误（比较严格；类型字面量 `DATE '...'` 或 CAST 可达）

#### Scenario: Date 主键路由回退

- **GIVEN** DATE 列作 PRIMARY KEY 的表
- **WHEN** 键位等值过滤 `WHERE d = DATE '2024-01-15'`
- **THEN** 行可达（MS16 非 Int 键列 DataScan 回退，无静默漏行）

### Requirement: CAST 矩阵扩展

CAST SHALL 支持 String→Date、String→Timestamp（严格解析，失败显式错误）、Date/Timestamp→String（DA5 格式化）、Timestamp→Date（截断到日）、Date→Timestamp（零点扩展）；数值/Bool 与日期族的跨族转换 SHALL 保持显式拒绝。

#### Scenario: 解析与格式化双向

- **GIVEN** `CAST('2024-01-15' AS DATE)`、`CAST(d AS STRING)`、`CAST(ts AS DATE)`
- **WHEN** 求值
- **THEN** 分别得 Date、ISO 字符串、截断 Date

#### Scenario: 非法与跨族拒绝

- **GIVEN** `CAST('bad' AS DATE)`、`CAST(42 AS DATE)`
- **WHEN** 求值
- **THEN** 均显式类型错误

### Requirement: 渲染与导入导出面

CLI 四格式渲染 SHALL 输出 DA5 字符串形态；json 渲染为字符串；dump SHALL 输出类型化字面量与 `DATE`/`TIMESTAMP` 类型 DDL；restore 与 `import --csv` 对 DATE/TIMESTAMP 列 SHALL 经强制解析通路落类型；多代 dump/restore SHALL 恒等。

#### Scenario: dump/restore 恒等

- **GIVEN** 含 DATE/TIMESTAMP 列与值的库
- **WHEN** dump → 空库 restore → 再 dump
- **THEN** 两代 dump 文本恒等

#### Scenario: CSV import 落类型

- **GIVEN** CSV 含日期列数据且目标列为 DATE
- **WHEN** `import --csv`
- **THEN** 落库为 Date；非法日期字段报错（fail-fast 既有语义）

### Requirement: 既有语义零回归

无 DATE/TIMESTAMP 参与的既有五类型行为（DDL/写入/比较/序列化/恢复/CLI 全链）SHALL 零回归；既有全量测试套件 SHALL 零修改通过。

**校准（Iteration 000 Review 记录，BH-1 同型先例）**：`tests/expression_e2e_test.rs::cast_unknown_target_type_rejected` 原以 `CAST(v AS DATE)` 作为「未知 CAST 目标」示例——R5 将 DATE 纳入合法目标族后该夹具失效，示例改为仍被计划期拒绝的 TIME，断言意图（未知目标显式拒绝）不变；此一处为 R5 合法化的必然校准，不计入零回归破例。

#### Scenario: 既有全量零回归

- **GIVEN** 936 项既有测试基线
- **WHEN** 实施后全量运行
- **THEN** 全部通过（零修改；仅上表校准段注记的一处示例替换）
