# sql-secondary-indexes Specification

## Purpose

约束用户级二级索引（`CREATE INDEX` / `DROP INDEX`）的端到端语义：单列、非唯一、声明 INT 列白名单与全形态修饰点名拒绝；建索引存量行全量回填与 catalog 持久化（干净重开与崩溃恢复 `redo_count > 0` 索引去信任重建通道扩展，两态一致）；索引名库级唯一；被索引列等值查询路由 `IndexScanAll` 可达且结果等价（不可键控字面量回落谓词下推）；INSERT/UPDATE（含 rekey）/DELETE/UPSERT 写路径与事务回滚还原的条目维护（PK 与唯一索引既有语义同型扩展）；DROP INDEX 页物理释放与查询回落扫描。来源：MS21（change `2026-09-29-ms21-ddl-evolution`，improvements I068；唯一索引基建复用 MS23）。

## Requirements

### Requirement: CREATE INDEX 建立单列 INT 二级索引

`CREATE INDEX <name> ON <table>(<col>)` SHALL 在 `<col>` 为声明 INT（含隐式 PK 之外的普通 INT 列）的单列上建立非唯一二级索引：建索引时 SHALL 对存量行全量索引化；索引元数据（名称、所属表、索引列、根页）SHALL 经 catalog 持久化，干净重开与崩溃恢复（`redo_count > 0` 索引去信任重建通道扩展）后索引 SHALL 可用且内容与数据一致（两态一致）。索引名 SHALL 库级唯一；重复名、表不存在、列不存在、列声明非 INT SHALL 计划期点名拒绝且零副作用。

`CREATE UNIQUE INDEX`、组合（多列）索引、表达式索引、部分索引（WHERE 子句）与 `IF NOT EXISTS` 子句 SHALL 计划期点名拒绝（能力可操作，MS19 错误面先例），SHALL NOT 静默忽略成分。对 PK 列建二级索引 SHALL 允许（冗余但行为等价，不做专门检测）。索引列值为 NULL 的行 SHALL NOT 入索引（`to_key` None，同 PK 与唯一索引既有语义；等值路由仅接受 Int 字面量，NULL 查询语义不受影响）。显式事务中的 CREATE INDEX 与 DROP INDEX SHALL 执行期点名拒绝（0A000；回填与登记一致性依赖无并发未提交写窗口——CTAS 事务内拒绝先例同型，拒绝先于执行器产生任何副作用）。

#### Scenario: 建索引与存量行索引化

- **GIVEN** 表 `t(id INT PRIMARY KEY, age INT, name VARCHAR)` 含若干行
- **WHEN** `CREATE INDEX idx_age ON t(age)`
- **THEN** 建索引成功；`SELECT * FROM t WHERE age = 25` 返回与建索引前完全一致的行集（含重复 age 值的多行全部命中）

#### Scenario: 干净重开后索引可用

- **GIVEN** 已执行 `CREATE INDEX idx_age ON t(age)` 并显式 close
- **WHEN** 重新打开数据库
- **THEN** 索引仍可用，等值查询行集一致

#### Scenario: 崩溃恢复两态一致

- **GIVEN** 建索引后含新写入行，进程未 close 退出（崩溃模拟：WAL 重放 + 索引重建）
- **WHEN** 重新打开数据库
- **THEN** 等值查询行集与崩溃前运行期行为一致

#### Scenario: 非 INT 列拒绝

- **GIVEN** 表 `t(id INT PRIMARY KEY, name VARCHAR)`
- **WHEN** `CREATE INDEX idx_name ON t(name)`
- **THEN** 计划期点名拒绝（首版仅 INT 列），零副作用

#### Scenario: 重复索引名拒绝

- **GIVEN** 已存在索引 `idx_age`（任意表）
- **WHEN** `CREATE INDEX idx_age ON other(c)`
- **THEN** 点名拒绝（索引名库级唯一），既有索引零变化

#### Scenario: UNIQUE 与组合索引拒绝

- **WHEN** `CREATE UNIQUE INDEX u ON t(age)` / `CREATE INDEX c ON t(age, id)`
- **THEN** 均计划期点名拒绝（唯一性约束走建表声明；首版单列）

#### Scenario: 表或列不存在拒绝

- **WHEN** `CREATE INDEX i1 ON nope(c)` / `CREATE INDEX i2 ON t(nope)`
- **THEN** 均点名拒绝（表不存在 / 列不存在），零副作用

### Requirement: 二级索引等值查询路由可达

被索引列上的等值谓词 `WHERE <col> = <Int 字面量>` SHALL 可路由到该二级索引（计划形状经 `EXPLAIN` 可观察，与 PK 键位等值路由同型）；索引存在与否 SHALL NOT 改变查询结果行集（结果等价，仅计划形状不同）。不可键控字面量（非 Int / NULL）SHALL 回落既有谓词下推行内求值（MS16 键列类型感知路由同型语义）。无索引列与未建索引的表 SHALL 保持既有扫描语义零回归。

#### Scenario: 等值路由计划形状可观察

- **GIVEN** 已建 `CREATE INDEX idx_age ON t(age)`
- **WHEN** `EXPLAIN SELECT * FROM t WHERE age = 25`
- **THEN** 计划树含该索引的索引扫描节点（点名索引），且不执行内层

#### Scenario: 建索引前后结果等价

- **GIVEN** 表 `t` 含重复 age 值多行
- **WHEN** 同一 `SELECT * FROM t WHERE age = 25` 在建索引前与建索引后执行
- **THEN** 两次行集一致（仅计划形状可能不同）

#### Scenario: 不可键控字面量回落行内求值

- **GIVEN** 已建 `idx_age`
- **WHEN** `SELECT * FROM t WHERE age = NULL`（或非 Int 字面量形态）
- **THEN** 回落既有扫描语义，无路由相关错误（与 PK 类型感知路由同型）

#### Scenario: 无索引列零回归

- **WHEN** `SELECT * FROM t WHERE name = 'x'`（未建索引列）
- **THEN** 行为与既有语义一致（全扫 + 谓词下推）

### Requirement: 二级索引写路径条目维护

INSERT、UPDATE（含索引列改值的 rekey）、DELETE、UPSERT（DO UPDATE 删冲突行 / REPLACE）SHALL 维护受影响表的全部二级索引条目与存活版本一致：新行/新值入条目、删行/旧值出条目；事务回滚后条目 SHALL 还原为与存活版本一致（PK 与唯一索引既有还原语义同型扩展）；崩溃恢复后两态一致。非唯一索引 SHALL 允许重复键多行并存。

#### Scenario: INSERT 后点查可达

- **GIVEN** 已建 `idx_age`
- **WHEN** `INSERT INTO t VALUES (10, 25, 'x')`
- **THEN** `SELECT * FROM t WHERE age = 25` 含新行

#### Scenario: DELETE 后点查不再命中

- **GIVEN** 已建 `idx_age`，行 (10, 25, 'x')
- **WHEN** `DELETE FROM t WHERE id = 10` 后 `SELECT * FROM t WHERE age = 25`
- **THEN** 不再含该行

#### Scenario: 索引列 rekey 后新旧键可达性一致

- **GIVEN** 已建 `idx_age`，行 (10, 25, 'x')
- **WHEN** `UPDATE t SET age = 30 WHERE id = 10`
- **THEN** `WHERE age = 30` 可达该行、`WHERE age = 25` 不再含该行（经索引路由与非索引路径结果一致）

#### Scenario: 事务回滚后条目还原

- **GIVEN** 已建 `idx_age`，CLI 会话 `BEGIN`
- **WHEN** `INSERT ...; UPDATE ... SET age ...; DELETE ...` 后 `ROLLBACK`
- **THEN** `WHERE age = ...` 等值查询行集与事务前一致（含重开两态）

#### Scenario: UPSERT 冲突行删除后条目清理

- **GIVEN** 已建 `idx_age`，行 (1, 25, 'a')
- **WHEN** `INSERT INTO t VALUES (1, 99, 'b') ON CONFLICT (id) DO UPDATE SET age = 99`（或 REPLACE 形态删旧行）
- **THEN** `WHERE age = 25` 不再含旧行、`WHERE age = 99` 含更新行

### Requirement: DROP INDEX 移除二级索引

`DROP INDEX <name>` SHALL 删除该索引：索引页 SHALL 物理释放（free-list，drop_table 同型通道），此后对应等值查询 SHALL 回落既有扫描且行集与删除前一致；表数据与其余索引 SHALL 不受影响。索引不存在 SHALL 点名拒绝；`IF EXISTS` 子句 SHALL 点名拒绝（假设 10）。删除 SHALL 跨重启持久（catalog 登记移除）。

#### Scenario: 删索引后查询回落扫描

- **GIVEN** 已建 `idx_age` 并删除（`DROP INDEX idx_age`）
- **WHEN** `SELECT * FROM t WHERE age = 25` 与 `EXPLAIN` 同查询
- **THEN** 行集与删索引前一致；计划形状不再含该索引节点

#### Scenario: 索引不存在拒绝

- **WHEN** `DROP INDEX nope`
- **THEN** 点名拒绝

#### Scenario: 删索引跨重启持久

- **GIVEN** `DROP INDEX idx_age` 后显式 close
- **WHEN** 重新打开数据库
- **THEN** 索引不存在，等值查询行为与扫描语义一致，无残留错误

### Requirement: 既有语义零回归

PK 索引与唯一索引（MS23）、键位等值路由（MS15/MS16）、约束执行面、T1 索引条目一致性收口语义 SHALL 全部保持；本 change 前全部既有测试 SHALL 零修改通过（语义校准白名单除外）。

#### Scenario: 既有索引与路由面零回归

- **WHEN** 运行完整测试套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 通过

### Requirement: IndexScanAll 列元数据与输出形状契约

二级索引等值路由节点（IndexScanAll）的列元数据 SHALL 遵循统一契约：节点 `columns` 恒为基表全列名清单，真实输出形状由 `projection`（基表列号）唯一决定。任意 SELECT 投影组合——单列（含后位列与索引列自身）、乱序多列、通配——SHALL 产出与请求一致且表头与行逐位对齐的结果，SHALL NOT panic；`EXPLAIN` 渲染 SHALL 遵循同一契约（计划树显示真实输出形状）。以该扫描为输入的聚合/分组、派生表、CTAS、视图、INSERT SELECT 的列解析 SHALL 按基表列号语义正确映射。PK 等值路由（IndexScan，「columns 已收窄」契约）与非索引回落路径 SHALL 零变化。

#### Scenario: 后位单列投影不 panic 且表头正确

- **GIVEN** 表 `q6(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER, salary INTEGER)` 与 `CREATE INDEX idx_q6_sal ON q6 (salary)`，行 (1,'Ada',1,90)
- **WHEN** `SELECT name FROM q6 WHERE salary = 90`
- **THEN** 表头 `["name"]`、行 `[["Ada"]]`（失败边界——修复前：`index out of bounds` panic，exit 101）

#### Scenario: 乱序多列投影表头与行对齐

- **GIVEN** 同上
- **WHEN** `SELECT name, id FROM q6 WHERE salary = 90`
- **THEN** 表头 `["name","id"]`、行 `[["Ada",1]]` 逐位对齐（失败边界——修复前：静默错位，表头 `["id","name"]`）

#### Scenario: EXPLAIN 渲染遵循同一契约

- **GIVEN** 同上
- **WHEN** `EXPLAIN SELECT name FROM q6 WHERE salary = 90`
- **THEN** 计划树文本正常输出且含 IndexScanAll 与真实输出形状（失败边界——修复前：`plan_format.rs` panic）

#### Scenario: PK 路由与范围谓词零回归

- **GIVEN** 同上
- **WHEN** `SELECT name FROM q6 WHERE id = 1`；`SELECT name FROM q6 WHERE salary > 80`；`SELECT * FROM q6 WHERE salary = 90`
- **THEN** 全部行为与修复前一致（既有契约与回落路径不破坏）

#### Scenario: 二级索引扫描上的聚合映射正确

- **GIVEN** 同上
- **WHEN** `SELECT COUNT(*), MAX(id) FROM q6 WHERE salary = 90`
- **THEN** 聚合结果正确（聚合输入 schema 按基表列号映射，不再受错位元数据影响）
