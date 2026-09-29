## ADDED Requirements

### Requirement: CREATE TABLE AS SELECT 物化

`CREATE TABLE <name> AS <SELECT ...>` SHALL 在主库创建新表并将 SELECT 结果物化写入：列名 SHALL 取 SELECT 输出表头；列类型自 SELECT 输出形状推导（推导规则与全 NULL 列等边界以 design 定稿为准）；目标表已存在 SHALL 点名拒绝且零副作用（不覆盖、不删除既有表）。物化结果 SHALL 跨重启可读（经 catalog 持久化与数据页落盘常轨）。SELECT 源 SHALL 可含附加库引用（`cross-db-namespace-query`）。

CTAS SHALL NOT 携带约束声明（PK/UNIQUE/NOT NULL/DEFAULT 不带入新表）；需要约束的建表走既有 CREATE TABLE + INSERT 路径。CTAS 新表沿用「未声明主键时首列为隐式主键」既有语义：物化行首列 Int 重复值 SHALL 以 `DuplicateKey` 点名失败。任何物化失败（类型推导不可行、写入校验拒绝等）SHALL 保持零副作用——已插入行回滚、已建表清除，目标表不存在于失败结果中。

#### Scenario: 主库 CTAS 物化

- **GIVEN** 主库表 `t(a INT, b VARCHAR)` 含若干行
- **WHEN** `CREATE TABLE m AS SELECT a, b FROM t WHERE a > 10`
- **THEN** 主库出现表 `m`，含过滤后全部行；重开库后 `SELECT * FROM m` 结果一致

#### Scenario: 跨库 CTAS（数据库视作表）

- **GIVEN** 附加库 `o.u(k INT, v VARCHAR)` 含若干行，会话已 ATTACH
- **WHEN** `CREATE TABLE m AS SELECT * FROM o.u`
- **THEN** 主库物化表 `m` 含 `o.u` 全部行（一次性物化，后续附加库变化不影响 `m`）

#### Scenario: 目标已存在拒绝

- **GIVEN** 主库已有表 `m`
- **WHEN** `CREATE TABLE m AS SELECT ...`
- **THEN** 点名拒绝且既有表 `m` 零变化

#### Scenario: CTAS 目标写附加库拒绝

- **GIVEN** 会话已 `ATTACH ... AS o`
- **WHEN** `CREATE TABLE o.m AS SELECT ...`
- **THEN** 计划期点名拒绝（一期 CTAS 仅写主库）

#### Scenario: 表达式与别名输出形状

- **WHEN** `CREATE TABLE m AS SELECT a + 1 AS inc FROM t`
- **THEN** 表 `m` 含列 `inc`，值为求值结果（输出表头名生效）

#### Scenario: 隐式主键冲突失败零副作用

- **GIVEN** 主库表 `t(a INT, b VARCHAR)` 含行 (1,'x'),(1,'y')
- **WHEN** `CREATE TABLE m AS SELECT a, b FROM t`
- **THEN** 物化以 `DuplicateKey` 点名失败；失败后 `m` 不存在（SELECT/后续 CTAS 同名可重试）

#### Scenario: 源语句失败零副作用

- **WHEN** `CREATE TABLE m AS SELECT * FROM nope`（源表不存在）
- **THEN** 点名失败且主库无表 `m`

#### Scenario: 显式事务内 CTAS 拒绝

- **GIVEN** CLI 会话已 `BEGIN`（事务活跃）
- **WHEN** `CREATE TABLE m AS SELECT ...`
- **THEN** 点名拒绝（一期不支持事务内 CTAS），会话事务状态不变

#### Scenario: 混合形态与 LIKE/CLONE 拒绝

- **WHEN** `CREATE TABLE t (a INT) AS SELECT ...`（列清单与 query 并存）/ `CREATE TABLE t LIKE other` / `CREATE TABLE t CLONE other`
- **THEN** 计划期点名拒绝（SHALL NOT 静默忽略 query/LIKE/CLONE 成分——现状 `query` 字段被静默丢弃的诚实化收口）
