# ddl-layout-publication Delta

## MODIFIED Requirements

### Requirement: 提交前失败的 DDL 目录面变更被补偿

`ALTER TABLE` 的目录面变更在事务提交之前因计划期拒绝、目录面写失败或刷盘返回错误而失败时，SHALL 被补偿：目录内容 SHALL 恢复为语句执行前的状态，且回退后的内容 SHALL 保持为待刷盘的脏页。该失败路径的补偿本身 SHALL NOT 刷盘，使后续任一次成功刷盘持久化的是回退后的内容。

`ALTER TABLE … DROP COLUMN` 在事务提交之前被放弃时，目录列定义与列数 SHALL 保持语句执行前的状态。

`ALTER TABLE … ADD COLUMN` 在列数刷盘已经成功、成功结果尚未交回调用方时被放弃的回退，不适用本 Requirement 的「补偿本身不刷盘」。该行为由 `ddl-catalog-transactional` 的「被放弃的 ADD COLUMN 不留下已落盘的列数上调」规定。

#### Scenario: 失败的 DDL 语句不留下目录面变更

- **GIVEN** 一个数据库，其某张表有三列且已有数据
- **WHEN** 执行 `ALTER TABLE t DROP COLUMN`，语句在事务提交之前失败
- **THEN** 该语句的目录面变更 SHALL 被补偿：目录的列定义与列数 SHALL 保持语句执行前的状态
- **AND** 其后任一次成功刷盘持久化的 SHALL 是回退后的内容

#### Scenario: 被放弃的 DDL 语句不留下目录面变更

- **GIVEN** 一个数据库，其某张表有三列且已有数据
- **WHEN** 执行 `ALTER TABLE t DROP COLUMN`，语句在事务提交之前被放弃
- **THEN** 目录的列定义与列数 SHALL 保持语句执行前的状态

#### Scenario: 失败的 `ADD COLUMN` 目录面被回退

- **GIVEN** 一个数据库，其某张表有两列且已有数据
- **WHEN** 执行 `ALTER TABLE t ADD COLUMN m INT`，该语句在发出目录面变更（列行已追加）之后失败（例如语句边界的刷盘失败）
- **THEN** 目录的**列定义**（新增的列行）与**列数** SHALL 同时恢复为语句执行前的状态
- **AND** 回退后的内容 SHALL **保持为待刷盘的脏页**（回退 SHALL NOT 自身刷盘），使后续任一次成功刷盘持久化的是**回退后的内容**
- **AND** 重新打开该库后该列 SHALL NOT 可见

### Requirement: 同表布局窄化 DDL 串行化

同一张表上的**布局窄化 DDL**（`ALTER TABLE … DROP COLUMN`）SHALL 至多有一条在途。在途状态 SHALL 由**带所有者令牌的每表状态**承载（不得是「单表一个布尔量」）：同表并发的布局窄化 DDL SHALL 被**显式拒绝**（错误 SHALL 点名该表），被拒绝的语句 SHALL NOT 启动数据面改写；在途状态 SHALL 只由令牌持有者清除。

令牌的取值 SHALL 仅作互斥所有权标记，SHALL NOT 承载任何身份语义（不得是 hash / 指纹 / revision / run-id / 时间证明）。

同表 `ALTER TABLE … ADD COLUMN` 在该表的窄化令牌持有期间 SHALL NOT 改写该表的活布局或目录列数。它 SHALL 等待令牌清除，然后按清除之后的活布局追加列。它 SHALL NOT 因窄化令牌被拒绝。正常路径发布安装的活布局 SHALL 是取得令牌时捕获的布局经窄化后的结果，SHALL NOT 是发布当时读取的、可能已被同表 `ADD COLUMN` 改过的活布局。恢复期发布仍从打开时载入的布局推导。目录列数已经等于目标列数时的幂等返回 SHALL NOT 替换活布局。

发布完成且等待中的同表 `ADD COLUMN` 也完成之后，同一进程内对该表的读取所见列集合、列数与行值 SHALL 与关闭后重新打开所见相同。改写开始到发布完成之间，读该表 SHALL 仍是既有的点名拒绝。

#### Scenario: 同表并发的窄化 DDL 被显式拒绝

- **GIVEN** 一个数据库，其某张表 `t` 上已有一条布局窄化 DDL 在途
- **WHEN** 对同一张表 `t` 并发执行另一条布局窄化 DDL
- **THEN** 后者 SHALL 以**明确的错误**结束，错误 SHALL 点名该表
- **AND** 后者 SHALL NOT 启动数据面改写（表的数据面与目录面 SHALL 不受该语句影响）

#### Scenario: 在途状态的清除不丢失读拒绝

- **GIVEN** 一个数据库，其某张表 `t` 上有布局窄化 DDL 在途
- **WHEN** 另一条同表窄化 DDL 被拒绝（在途状态的重叠清除路径）
- **THEN** 在途状态 SHALL NOT 被非持有者清除
- **AND** 该表的读拒绝 SHALL 保持，直到令牌持有者完成发布

#### Scenario: 跨表并发不受影响

- **GIVEN** 一个数据库，其某张表 `t` 上有布局窄化 DDL 在途
- **WHEN** 对**另一张**表并发执行布局窄化 DDL
- **THEN** 该语句 SHALL NOT 因表 `t` 的在途窄化而被拒绝

#### Scenario: 窄化令牌不约束 `ADD COLUMN`

- **GIVEN** 一个数据库，其某张表 `t` 上有布局窄化 DDL 在途
- **WHEN** 对同一张表 `t` 执行 `ALTER TABLE t ADD COLUMN`
- **THEN** 该语句 SHALL NOT 因窄化令牌而被拒绝（`ADD COLUMN` 不产生窄化发布，其同表并发由既有的表级互斥串行化）

#### Scenario: 在途窄化窗口内的同表 ADD COLUMN 不改写发布所用布局

- **GIVEN** 表 `t` 有三列 `id`、`v`、`w` 且已有数据；`ALTER TABLE t DROP COLUMN w` 已提交、目录窄化尚未发布
- **WHEN** 对 `t` 执行 `ALTER TABLE t ADD COLUMN m INT`，并在删列仍停在该窗口时观察 `t`
- **THEN** 该 `ADD COLUMN` SHALL NOT 返回因窄化令牌而拒绝的错误
- **AND** 删列仍停在该窗口时，`t` 的目录列数与列名 SHALL 保持发布前的值（此次加列尚未追加）
- **AND** 读取 `t` SHALL 仍被点名拒绝
- **AND** 放行删列后，两条语句都 SHALL 成功完成
- **AND** 完成后同一进程内 `SELECT * FROM t` 的列集合、列数与行值 SHALL 与关闭后重新打开该库所见相同：列 `w` 不在，列 `m` 在，原 `id` 与 `v` 的值保持，`m` 按无 DEFAULT 读回

#### Scenario: 另一张表的 ADD COLUMN 不等待在途窄化

- **GIVEN** 表 `t` 上有布局窄化 DDL 在途，另一张表 `u` 存在
- **WHEN** 对 `u` 执行 `ALTER TABLE u ADD COLUMN`
- **THEN** 该语句 SHALL 完成，不等待 `t` 的发布
- **AND** `t` 的目录列数与列名 SHALL NOT 被这次加列改写
