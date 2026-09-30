## ADDED Requirements

### Requirement: ALTER TABLE ADD COLUMN 尾部加列

`ALTER TABLE <table> ADD COLUMN <col> <type>` SHALL 在表尾追加新列且不要求全表数据重写：存量行读回时新列 SHALL 按声明 `DEFAULT`（无 DEFAULT 则 NULL）填充；新写入行 SHALL 按全宽行落盘。加列 SHALL 支持六类列类型与 `DEFAULT`/`NOT NULL` 声明；`NOT NULL` 且无 DEFAULT 且表非空 SHALL 计划期点名拒绝（存量行无法满足）；`NOT NULL` + DEFAULT、可空无 DEFAULT 均 SHALL 可行。ADD COLUMN 中声明 `PRIMARY KEY`/`UNIQUE`/`CHECK`/FOREIGN KEY SHALL 计划期点名拒绝（唯一索引演进不在范围）。加列结果 SHALL 跨重启持久；dump/restore/import 通道 SHALL 保真（schema 渲染含新列，restore 往返一致）。

#### Scenario: 加列后存量行读回 NULL

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含若干行
- **WHEN** `ALTER TABLE t ADD COLUMN note VARCHAR`
- **THEN** 成功；`SELECT id, note FROM t` 全部行为 NULL；`INSERT INTO t (id, v, note) VALUES (9, 1, 'x')` 后新行读回 `'x'`

#### Scenario: 带 DEFAULT 加列存量行读回默认值

- **WHEN** `ALTER TABLE t ADD COLUMN flag INT DEFAULT 0`（表含存量行）
- **THEN** 存量行 `flag` 读回 `0`；子集 INSERT 省略该列时新行 `flag` 亦为 `0`（`sql-write-surface` DEFAULT 语义衔接）

#### Scenario: NOT NULL + DEFAULT 非空表可行

- **WHEN** `ALTER TABLE t ADD COLUMN flag INT NOT NULL DEFAULT 7`（表含存量行）
- **THEN** 成功，存量行读回 `7`

#### Scenario: NOT NULL 无 DEFAULT 非空表拒绝

- **GIVEN** 表 `t` 非空
- **WHEN** `ALTER TABLE t ADD COLUMN req VARCHAR NOT NULL`
- **THEN** 计划期点名拒绝且零副作用（列未加入）

#### Scenario: 约束声明拒绝

- **WHEN** `ALTER TABLE t ADD COLUMN u INT UNIQUE` / `... ADD COLUMN p INT PRIMARY KEY` / `... CHECK (...)`
- **THEN** 均计划期点名拒绝（点名能力），零副作用

#### Scenario: 加列持久与 dump→restore 保真

- **GIVEN** 加列（含 DEFAULT）后 dump
- **WHEN** restore 到新库
- **THEN** 新库 schema 含新列与 DEFAULT，数据读回一致；重开持久

### Requirement: ALTER TABLE DROP COLUMN 删列

`ALTER TABLE <table> DROP COLUMN <col>` SHALL 从 schema 移除该列，此后查询输出形状、INSERT 目标列集与 dump/restore 面 SHALL 与删列后 schema 一致；存量物理数据的处理策略（惰性读时投影 vs 全行重写）随 Phase 2 调查与 design 定稿，SHALL NOT 产生删列后可观察的幽灵数据（被删列值不出现在任何查询输出中）。被 PK、唯一索引或二级索引引用的列 SHALL 计划期点名拒绝；删除表中最后一列 SHALL 点名拒绝（表至少保留一列）；列不存在 SHALL 点名拒绝。删列结果 SHALL 跨重启持久，dump/restore 往返一致。

**split signal（proposal 假设 6）**：若调查确认删列需全行重写且改造成本超预期，本 Requirement 的实施拆出独立 change，本 change 先收 ADD COLUMN——届时本域 MODIFIED 收窄并在收尾说明。

#### Scenario: 删列后查询形状一致

- **GIVEN** 表 `t(a INT PRIMARY KEY, b INT, c VARCHAR)` 含若干行
- **WHEN** `ALTER TABLE t DROP COLUMN b`
- **THEN** `SELECT * FROM t` 输出 `a, c` 两列且行数据正确；`INSERT INTO t (a, c) VALUES (1, 'x')` 可行、`INSERT INTO t (a, b, c) ...` 点名拒绝（列不存在）

#### Scenario: 被索引引用的列拒绝

- **GIVEN** 已建 `CREATE INDEX idx_b ON t(b)`（或 b 为 UNIQUE/PK 声明）
- **WHEN** `ALTER TABLE t DROP COLUMN b`
- **THEN** 计划期点名拒绝（先删索引再删列），零副作用

#### Scenario: 删除最后一列拒绝

- **GIVEN** 表 `t(a INT PRIMARY KEY)`
- **WHEN** `ALTER TABLE t DROP COLUMN a`
- **THEN** 点名拒绝（表至少保留一列）

#### Scenario: 删列持久与 dump→restore 保真

- **GIVEN** 删列后 dump
- **WHEN** restore 到新库并重开
- **THEN** schema 与数据读回一致，被删列不出现在 schema 或数据中

#### Scenario: 列不存在拒绝

- **WHEN** `ALTER TABLE t DROP COLUMN nope`
- **THEN** 点名拒绝，零副作用

### Requirement: ALTER 语句形态与既有面边界

`ALTER TABLE` SHALL 仅接受单操作语句；`ALTER TABLE ... ADD COLUMN ..., DROP COLUMN ...` 多操作 SHALL 计划期点名拒绝。目标表不存在 SHALL 点名拒绝。对系统保留名表与视图执行 ALTER SHALL 点名拒绝。显式事务中的 ALTER SHALL 执行期点名拒绝（0A000；DROP COLUMN 的重写一致性与数据面宽度演进依赖无并发未提交写窗口，design D10/D13——CTAS/CREATE INDEX 事务内拒绝先例同型，拒绝先于执行器产生任何副作用）。既有 SELECT/INSERT/UPDATE/DELETE/dump-restore 语义 SHALL 零回归。

#### Scenario: 多操作单语句拒绝

- **WHEN** `ALTER TABLE t ADD COLUMN x INT, DROP COLUMN y`
- **THEN** 计划期点名拒绝（首版单操作），零副作用

#### Scenario: 目标不存在拒绝

- **WHEN** `ALTER TABLE nope ADD COLUMN x INT`
- **THEN** 点名拒绝

#### Scenario: 显式事务内拒绝

- **GIVEN** CLI 会话已 `BEGIN`
- **WHEN** `ALTER TABLE t ADD COLUMN x INT`（或 DROP COLUMN）
- **THEN** 执行期点名拒绝（0A000）且零副作用，会话事务状态不变

#### Scenario: 既有面零回归

- **WHEN** 运行完整测试套件
- **THEN** 除本 change 新增见证外既有测试零修改通过；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 通过
