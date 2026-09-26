# sql-write-surface Specification

## Purpose

定义 SQL 写入面的补全语义：子集列清单 INSERT 与 DEFAULT 的解析、持久化与应用闭环；写入值与列声明类型的强制一致门（ISS04）；UPSERT（ON CONFLICT DO NOTHING / DO UPDATE）与 REPLACE INTO 的冲突仲裁与原位更新语义；UPSERT 冲突目标与不支持形态的计划期点名拒绝；既有写面语义零回归。来源：MS24（change `2026-09-25-ms24-write-surface-completion`，2026-09-25 规划、2026-09-26 收尾）。

## Requirements

### Requirement: 子集列清单 INSERT 与 DEFAULT 应用

显式列清单的 INSERT SHALL 接受表列的任意子集（每项解析为互异已知列），SHALL 按清单→表列映射重排各行值（既有语义）；被省略的列 SHALL 取该列声明 DEFAULT 值，未声明 DEFAULT 时 SHALL 取 NULL。声明 NOT NULL 且无 DEFAULT 的省略列由既有执行器门以点名列名的错误零副作用拒绝。`VALUES` 中的 `DEFAULT` 关键字 SHALL 等价于该列为省略（取声明 DEFAULT 或 NULL）。未知列、重复列与（无清单时）行长度不符 SHALL 维持既有计划期点名拒绝。声明 DEFAULT 的建表 SHALL 将字面量默认值持久化进 catalog 并在重开数据库后继续生效；dump 生成的建表 DDL SHALL 渲染 DEFAULT 使 dump→restore 往返保真。全列清单与无清单 INSERT 的既有行为 SHALL 保持不变。

#### Scenario: 子集清单省略列取 DEFAULT

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, name STRING DEFAULT 'anon', score INT)`
- **WHEN** 执行 `INSERT INTO t (id, score) VALUES (1, 90)`
- **THEN** 插入成功；查询该行 `name` 为 `'anon'`

#### Scenario: 子集清单省略无 DEFAULT 的可空列取 NULL

- **GIVEN** 建表含可空且无 DEFAULT 的列 `score`
- **WHEN** 执行省略 `score` 的子集 INSERT
- **THEN** 插入成功；该行 `score` 为 NULL

#### Scenario: 省略 NOT NULL 且无 DEFAULT 的列被零副作用拒绝

- **GIVEN** 建表含 `name STRING NOT NULL`（无 DEFAULT）
- **WHEN** 执行省略 `name` 的子集 INSERT
- **THEN** 报错且错误文本点名列名 `name`；表行数不变

#### Scenario: VALUES 中的 DEFAULT 关键字

- **GIVEN** 建表含 `name STRING DEFAULT 'anon'` 与可空无 DEFAULT 的 `score INT`
- **WHEN** 执行 `INSERT INTO t (id, name, score) VALUES (1, DEFAULT, DEFAULT)`
- **THEN** 插入成功；`name` 为 `'anon'`，`score` 为 NULL

#### Scenario: DEFAULT 跨重启生效

- **GIVEN** 建表含 DEFAULT 声明并已关闭重开数据库
- **WHEN** 执行省略该列的子集 INSERT
- **THEN** 省略列取声明 DEFAULT 值（持久化语义与建表会话内一致）

#### Scenario: dump→restore 往返保真 DEFAULT

- **GIVEN** 建表含 DEFAULT 声明的表并写入数据
- **WHEN** 执行 dump 后向新库 restore
- **THEN** restore 生成表的 schema 文本含 DEFAULT 声明；对 restore 后的表执行子集 INSERT 行为与原库一致

#### Scenario: 未知列与重复列维持计划期拒绝

- **GIVEN** 任意表
- **WHEN** 执行含未知列名或重复列名的 INSERT
- **THEN** 计划期报错点名该列，表数据不变

#### Scenario: 全列清单与无清单既有行为零回归

- **GIVEN** 任意表（含乱序全列清单既有用例）
- **WHEN** 执行全列清单或无清单 INSERT
- **THEN** 行为与既有语义一致（含 MS16 乱序映射与行长度拒绝）

### Requirement: 写入值类型一致门

INSERT 与 UPDATE 在写入任何数据页、WAL、版本链或索引之前 SHALL 校验写入值的数据变体与目标列声明类型一致：日期族列的 String 值经既有强制解析通道落类型后视为一致；FLOAT 列 SHALL 接受整数值并无损升格为 FLOAT；NULL 值 SHALL 豁免（NULL 性由 NOT NULL 门裁决）；其余跨类型写入 SHALL 以点名列名、期望类型与实际类型的错误拒绝，且不产生任何副作用。既有 PK 键列门（`KeyTypeMismatch`）、INT 唯一列守卫与 NOT NULL 门的触发优先级及错误文本 SHALL 保持不变（一般类型门作为其后的最后写入前置校验覆盖全部列）。该门 SHALL 覆盖 INSERT、UPDATE、dump/restore 与 CSV import 的全部写入通道。

#### Scenario: 非键 INT 列拒绝 String 值

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, n INT)`
- **WHEN** 执行 `INSERT INTO t VALUES (1, 'abc')` 与 `UPDATE t SET n = 'abc' WHERE id = 1`
- **THEN** 均报错且错误文本点名 `n`、期望类型与实际类型；行数与既有行值不变

#### Scenario: 非键 STRING 列拒绝整数值

- **GIVEN** 建表含 `name STRING` 列
- **WHEN** 执行 `INSERT INTO t VALUES (1, 42)`（name 位为整数字面量）
- **THEN** 报错点名 `name` 与类型不一致；不落库

#### Scenario: FLOAT 列接受整数值升格

- **GIVEN** 建表含 `score FLOAT` 列
- **WHEN** 执行 `INSERT INTO t VALUES (1, 2)`（整数字面量）
- **THEN** 插入成功，`score` 读回为 FLOAT 值 `2.0` 语义（升格无损）

#### Scenario: 日期族 String 经强制解析保持一致

- **GIVEN** 建表含 `d DATE` 列
- **WHEN** 执行 `INSERT INTO t VALUES (1, '2026-09-25')`
- **THEN** 插入成功（String 经既有 coerce 落 DATE 类型，门不拒绝）；非法日期字符串仍被既有通道拒绝

#### Scenario: 既有键列与 NOT NULL 错误面优先级不变

- **GIVEN** 既有键位类型与 NOT NULL 拒绝矩阵用例
- **WHEN** 重复执行这些用例
- **THEN** 错误文本与触发行为逐字节不变（一般类型门不提前触发）

#### Scenario: 全通道覆盖（dump/restore/import）

- **GIVEN** 经 dump→restore 往返与 CSV import 通道
- **WHEN** 通道产生的写入值类型正确
- **THEN** 全部成功不误报；通道外构造的类型不匹配值仍被拒绝

> **已知边界（Iteration 000 Plan Review 裁定，2026-09-25）**：FLOAT 声明键列收整数值经升格落为 Float 无键行（`to_key` 仅 Int 产键，键编码扩展属 I024 域外）——该形态行不入 PK 索引：同键重复 INSERT 不触发 DuplicateKey、按 PK 等值的 UPDATE/DELETE 经索引不可达（KeyNotFound）；SELECT 经既有非 Int 键路由（谓词下推）可达，恢复期自 tuple 重建与运行期两态一致。该语义与非整数值 Float 键列的既有无键行语义（MS10-T05 先例）收敛。存量混合库边缘（本 change 之前写入的 Int-tag Float 键列行）：`UPDATE SET f = <整数值>` 的 rekey 碰撞预检以升格前值派生键，可能对同表其他遗留键条目保守误报 DuplicateKey——新库状态不可达，保守拒绝方向，不做工程化规避。

### Requirement: UPSERT 语义（DO NOTHING / DO UPDATE / REPLACE）

`INSERT ... ON CONFLICT [target] DO NOTHING` SHALL 在冲突行上跳过该行（不计入受影响行数），非冲突行正常插入；`DO UPDATE SET col = expr, ...` SHALL 在冲突行上原位更新（多列赋值可达），赋值表达式 SHALL 支持字面量（含日期族类型字面量与 NULL）、`excluded.col`（本行待插入值）与裸列名（冲突行旧值）三形态，其他表达式形态 SHALL 计划期点名拒绝。DO UPDATE 的写入 SHALL 镜像既有 UPDATE 语义：NOT NULL/类型/键位校验与碰撞预检在任何写入前零副作用，版本链新版本指向冲突行旧版本，WAL 记 Update 记录，PK 与唯一索引按既有 UPDATE 维护语义（含 rekey）。`REPLACE INTO` SHALL 等价于冲突时删除冲突行（既有删除墓碑与索引清理语义）后插入新行，每行计 1 行；非冲突行普通插入。多行语句 SHALL 逐行独立判定冲突与执行动作。冲突仲裁 SHALL 确定性进行：省略目标时 PK 索引优先、其后唯一索引按列序；显式目标仅仲裁该约束，行违反仲裁外约束时 SHALL 以既有 DuplicateKey 错误拒绝（SQLite 对齐）。崩溃恢复与干净重开后的 UPSERT 行为 SHALL 与运行期一致。

#### Scenario: DO NOTHING 冲突行跳过

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 已有行 `(1, 10)`
- **WHEN** 执行 `INSERT INTO t VALUES (1, 20) ON CONFLICT DO NOTHING`
- **THEN** 执行成功，受影响行数为 0；查询 `(1)` 行 `v` 仍为 10

#### Scenario: DO NOTHING 非冲突行正常插入

- **GIVEN** 同上表已有行 `(1, 10)`
- **WHEN** 执行多行 `INSERT INTO t VALUES (2, 30), (1, 99) ON CONFLICT DO NOTHING`
- **THEN** `(2,30)` 插入、`(1,…)` 跳过；受影响行数为 1

#### Scenario: DO UPDATE 原位更新（字面量与 excluded/旧行引用）

- **GIVEN** 建表 `t(id INT PRIMARY KEY, v INT, u INT UNIQUE)` 已有行 `(1, 10, 100)` 与 `(2, 20, 200)`
- **WHEN** 执行 `INSERT INTO t VALUES (1, 99, 100) ON CONFLICT (id) DO UPDATE SET v = excluded.v, u = u`
- **THEN** 冲突行更新为 `(1, 99, 100)`（`v` 取 excluded 新行值、`u` 取旧行值，赋值不引入新唯一冲突）；受影响行数计入；字面量赋值与多列赋值同可达；算术/函数表达式（如 `SET v = excluded.v + 1`）计划期点名拒绝

#### Scenario: DO UPDATE 碰撞预检零副作用

- **GIVEN** 表含 PK 行 `(1,…)` 与另一行唯一列值 `u=200`
- **WHEN** 执行 `INSERT ... VALUES (1, ..., 200) ON CONFLICT (id) DO UPDATE SET u = 200`
- **THEN** 报 DuplicateKey 错误；两行数据与索引条目均不变

#### Scenario: REPLACE INTO 冲突行删除重插

- **GIVEN** 表已有行 `(1, 10)`
- **WHEN** 执行 `REPLACE INTO t VALUES (1, 20)`
- **THEN** 冲突行被替换为 `(1, 20)`；受影响行数为 1；非冲突行普通插入

#### Scenario: 恢复两态一致

- **GIVEN** 执行过 DO UPDATE / DO NOTHING / REPLACE 的数据库
- **WHEN** 干净重开或经崩溃恢复后查询
- **THEN** 数据与索引状态与执行后一致（WAL Insert/Update/Delete 重放通道语义成立）

> **已知边界（Iteration 001 Review Minor 5 裁定记录，2026-09-26）**：冲突仲裁在读取待插入行的唯一列值时先经「唯一列写入值类型一致」守卫（`sql-constraint-enforcement` R3 补强段）——同一行既有 PK 冲突、其唯一列写入值又为非 Int 非法类型时，该行以 `KeyTypeMismatch` 拒绝，而非按 DO NOTHING 跳过。方向安全（无静默写坏，且拒绝早于任何写入），但本 Requirement「DO NOTHING 在冲突行上跳过该行」在该角落不字面成立；同输入的纯 INSERT 报 `DuplicateKey`，两处均为拒绝面而非成功面。

### Requirement: UPSERT 冲突目标与拒绝面

冲突目标 SHALL 支持省略（仲裁 PK 与全部唯一索引）与显式单列（须解析为 INT 主键列或 INT 唯一列）；无法匹配任何既有唯一性约束的目标（含组合多列目标，引擎无组合唯一约束）SHALL 以「ON CONFLICT 子句不匹配任何 PRIMARY KEY 或 UNIQUE 约束」语义的错误拒绝；`ON CONSTRAINT`（PostgreSQL 语法）、MySQL `ON DUPLICATE KEY UPDATE`、`DO UPDATE WHERE` 子句 SHALL 计划期点名拒绝；`INSERT OR ...` 方言形态维持 sqlparser 解析层拒绝（GenericDialect 不解析，已知边界）。上述拒绝均 SHALL 发生在计划期且不产生任何写入。

#### Scenario: 显式单列目标命中 PK 与唯一列

- **GIVEN** 表含 INT 主键与 INT UNIQUE 列
- **WHEN** 分别执行 `ON CONFLICT (id)` 与 `ON CONFLICT (u)` 的 DO NOTHING 插入冲突行
- **THEN** 对应约束冲突被仲裁跳过，行为正确

#### Scenario: 组合目标与不匹配目标精确拒绝

- **GIVEN** 仅含单列唯一约束的表
- **WHEN** 执行 `ON CONFLICT (a, b)` 或目标为非唯一列
- **THEN** 计划期报错且语义为「不匹配任何 PRIMARY KEY 或 UNIQUE 约束」

#### Scenario: ON CONSTRAINT / ON DUPLICATE KEY UPDATE / DO UPDATE WHERE 点名拒绝

- **GIVEN** 任意表
- **WHEN** 分别执行含 `ON CONFLICT ON CONSTRAINT c`、`ON DUPLICATE KEY UPDATE`、`DO UPDATE ... WHERE` 的插入
- **THEN** 均计划期报错并点名不支持特性；无写入发生

#### Scenario: 非 INT 列目标拒绝

- **GIVEN** 表含非 INT 唯一声明不可能存在（既有诚实化拒绝），非 INT 主键列无索引
- **WHEN** 执行以非 INT 主键列为目标的 ON CONFLICT
- **THEN** 计划期拒绝（该列无唯一性索引可仲裁）

### Requirement: 既有写面语义零回归

除上述 requirement 明确改变的行为外，既有 INSERT/UPDATE/DELETE/REPLACE 解析、计划与执行语义 SHALL 保持不变：无冲突子句的 INSERT、单列 UPDATE、键位类型与唯一性错误面、无键行落库语义、事务与会话语义、dump/restore/import 通道行为、既有测试矩阵 SHALL 全部保持。

#### Scenario: 无冲突子句 INSERT 行为不变

- **GIVEN** 既有 INSERT 全部用例
- **WHEN** 重复执行
- **THEN** 行为与错误面逐字节一致

#### Scenario: 既有子集拒绝用例按新语义校准

- **GIVEN** MS16 的「子集清单计划期拒绝」用例
- **WHEN** 子集清单成为合法语义
- **THEN** 该用例按新语义校准（校准记录随 Act Response），其原有防回归意图（未知列/重复列拒绝、无 panic）由等价新用例承接
