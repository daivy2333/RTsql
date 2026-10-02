# sql-write-surface Delta — 声明期 DEFAULT 类型一致

## ADDED Requirements

### Requirement: 声明期 DEFAULT 类型一致

建表（`CREATE TABLE`）与加列（`ALTER TABLE ADD COLUMN`）声明的 DEFAULT 字面量 SHALL 在计划期与列声明类型校验/归一，规则与「写入值类型一致门」对齐：`DEFAULT NULL` SHALL 豁免（NULL 性由 NOT NULL 门裁决）；FLOAT 列的整数字面量 SHALL 无损升格为 FLOAT 归一后持久化；日期族（DATE/TIMESTAMP）列的 String 字面量 SHALL 经既有强制解析通道验证——解析成功视为一致，解析失败 SHALL 建表/加列计划期点名拒绝且零副作用；其余跨类型声明（如 BOOL 列 `DEFAULT 1`、INT 列 `DEFAULT 'x'`、STRING 列 `DEFAULT 1`、INT 列 `DEFAULT 1.5`）SHALL 计划期点名拒绝（错误点名列名、期望类型与实际值类型）且零副作用。类型字面量形态（TypedString，如 `DEFAULT DATE '2026-01-02'`）SHALL 在日期族列上解析并接受（目录存储形态与 String 字面量一致），在非日期族列上 SHALL 维持既有计划期拒绝。dump 生成的 `DEFAULT DATE '…'` / `DEFAULT TIMESTAMP '…'` 文本 SHALL 可经 restore 重建——dump→restore 往返保真对日期族 DEFAULT 成立（本 Requirement 落地修复现存的 TypedString 解析缺失缺陷）。声明期归一后的 catalog 默认值 SHALL 跨重启与 dump→restore 保真。既有建表与既有 DEFAULT 场景（无 DEFAULT、类型一致 DEFAULT、`DEFAULT NULL`）SHALL 逐字节零回归。

#### Scenario: BOOL 列 DEFAULT 整数字面量建表拒绝

- **GIVEN** 建表语句 `CREATE TABLE t (b BOOL DEFAULT 1)`
- **WHEN** 执行建表
- **THEN** 计划期点名拒绝（列 `b`、期望 BOOL、实际 INT）；表不存在，零副作用

#### Scenario: 跨类型声明拒绝矩阵

- **GIVEN** 建表语句 `CREATE TABLE t (a INT DEFAULT 'x')`、`CREATE TABLE t (s STRING DEFAULT 1)`、`CREATE TABLE t (a INT DEFAULT 1.5)`
- **WHEN** 分别执行建表
- **THEN** 均计划期点名拒绝且零副作用（错误含列名与类型不一致语义）

#### Scenario: FLOAT 列整数 DEFAULT 升格归一

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, score FLOAT DEFAULT 2)`
- **WHEN** 建表成功后执行省略 `score` 的 INSERT 并查询
- **THEN** `score` 读回 FLOAT `2.0`（声明期归一为 FLOAT 持久化）；重开库与 dump→restore 往返后行为一致；dump 渲染 `DEFAULT 2.0`

#### Scenario: 日期族 DEFAULT String 强制解析

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, d DATE DEFAULT '2026-01-02')`
- **WHEN** 建表成功后执行省略 `d` 的 INSERT 并查询
- **THEN** `d` 为 DATE `2026-01-02`（String 经解析通道验证一致）；建表 `d DATE DEFAULT '2026-13-99'`（非法日期）SHALL 计划期点名拒绝且零副作用

#### Scenario: TypedString 声明与 dump→restore 往返修复

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, d DATE DEFAULT DATE '2026-01-02')`（或 String 形态建表后 dump）
- **WHEN** 执行建表；对含日期族 DEFAULT 的表执行 dump 后向空库 restore
- **THEN** 建表成功；restore 成功重建（`DEFAULT DATE '2026-01-02'` 经 TypedString 解析臂接受），restore 后表 schema 含 DEFAULT 声明、省略列 INSERT 行为与原库一致（修复前：restore 报 `Unsupported value type` 失败——实证缺陷闭环）

#### Scenario: 非日期族 TypedString 维持拒绝

- **WHEN** 执行 `CREATE TABLE t (a INT DEFAULT INT '5')` 等非日期族类型字面量声明
- **THEN** 维持既有计划期拒绝（Unsupported value 语义不变）

#### Scenario: DEFAULT NULL 豁免

- **GIVEN** 建表 `CREATE TABLE t (b BOOL DEFAULT NULL)`
- **WHEN** 执行建表与省略 `b` 的 INSERT
- **THEN** 建表成功，该行 `b` 为 NULL（NULL 性由 NOT NULL 门裁决，声明期不拒绝）

#### Scenario: ALTER ADD COLUMN 同款规则

- **GIVEN** 任意已有表
- **WHEN** 执行 `ALTER TABLE t ADD COLUMN b BOOL DEFAULT 1`
- **THEN** 计划期点名拒绝且零副作用（表结构不变）；`ALTER TABLE t ADD COLUMN d DATE DEFAULT '2026-01-02'` 建列成功，存量行读回该列为声明默认值

#### Scenario: 既有 DEFAULT 场景零回归

- **GIVEN** 既有 DEFAULT 全部用例（`name STRING DEFAULT 'anon'`、`score INT DEFAULT 0`、子集 INSERT/DEFAULT 关键字/跨重启/dump→restore 矩阵）
- **WHEN** 重复执行
- **THEN** 行为与错误面逐字节一致（声明期校验只新增拒绝面，不改变既有接受面的持久化与应用语义）

### Requirement: CREATE TABLE 重复列名计划期拒绝

`CREATE TABLE` 的列定义 SHALL 在计划期拒绝重复列名（含经引号包裹与大小写归一后相同的名称，planner 列名小写归一语义一致）：错误 SHALL 点名重复列名，拒绝零副作用（表不存在于失败结果中）。该守卫使「重复列名表被静默建成且 dump→restore 必触发 `MultiplePrimaryKey`」的既有缺陷闭环（2026-10-01 实证：`CREATE TABLE dup ("a" INT, "a" INT)` 曾 rc=0 且 schema 渲染双 `PRIMARY KEY`）。CSV import 自动建表经常规建表通道执行，本守卫自然覆盖其重复表头形态。CTAS 输出列重复既有拒绝与普通建表唯一列名零回归。

#### Scenario: 重复列名建表拒绝

- **WHEN** 执行 `CREATE TABLE dup ("a" INT, "a" INT)`
- **THEN** 计划期点名拒绝（错误含重复列名 `a`）；表不存在，零副作用

#### Scenario: 归一后同名拒绝

- **WHEN** 执行 `CREATE TABLE t (A INT, a INT)`（引号/大小写归一后同名）
- **THEN** 计划期点名拒绝且零副作用（与裸名归一化语义一致）

#### Scenario: 唯一列名建表与 CTAS 既有面零回归

- **GIVEN** 既有建表与 CTAS 全部用例（列名互异建表成功；CTAS 输出列名重复既有计划期拒绝）
- **WHEN** 重复执行
- **THEN** 行为与错误面逐字节一致（本守卫只新增重复形态拒绝面）
