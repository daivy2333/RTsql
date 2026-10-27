# ddl-layout-publication Specification

## Purpose
`ALTER TABLE … DROP COLUMN` 的窄化意图作为事务作用域的写入日志记录，与该事务的提交记录同一次持久化；目录窄化在提交之后幂等发布，提交前失败的目录面被补偿，同表布局窄化串行拒绝，未发布的意图不因检查点丢失。来源：`2026-10-25-ms37-defect-ledger-closure`

## Requirements

### Requirement: 布局窄化的意图与数据面改写原子绑定

`ALTER TABLE … DROP COLUMN` 的布局窄化意图（被删列的序号与新列数）SHALL 作为**事务作用域的写入日志记录**写入该语句的事务记录序列，SHALL 与该事务的提交记录**同一次持久化**落地。SHALL NOT 把该意图只写入数据库文件（目录行）——目录行中的意图无法与「数据面窄化是否已提交」原子绑定，恢复时无法判定该窄化应当发布还是撤销。

#### Scenario: 窄化意图与数据面改写同一次持久化

- **GIVEN** 一个数据库，其某张表有三列且已有数据
- **WHEN** 执行 `ALTER TABLE t DROP COLUMN`，随后进程崩溃（写入日志含该语句的记录）
- **THEN** 该语句的窄化意图与该事务的提交状态 SHALL 在写入日志中**原子一致**（提交记录存在 ⇔ 意图已持久）
- **AND** 系统 SHALL NOT 出现「意图已持久但数据面未提交」或「数据面已提交但意图不存在」的状态

#### Scenario: 崩溃重开后已提交的窄化被发布

- **GIVEN** 一个数据库，其某张表有三列且已有数据；一次 `ALTER TABLE t DROP COLUMN` 的数据面改写已提交、目录窄化尚未发布时进程崩溃
- **WHEN** 重新打开该库并读取表 `t`
- **THEN** 恢复 SHALL 对该**已提交**的窄化意图发布目录窄化（改写列行 + 下调列数）
- **AND** 重开后的目录列数 SHALL 与数据面行宽一致
- **AND** 读取 SHALL NOT 按旧列序错位取值（SHALL NOT 静默返回错位的值）

#### Scenario: 未提交的窄化不被发布

- **GIVEN** 一个数据库，其某张表有三列且已有数据；一次 `ALTER TABLE t DROP COLUMN` 未提交（被放弃或失败）时进程崩溃
- **WHEN** 重新打开该库并读取表 `t`
- **THEN** 恢复 SHALL NOT 发布该窄化
- **AND** 目录列数 SHALL 与数据面行宽一致（列数等于语句执行前的列数，各行按该列数存放）

### Requirement: 提交点是事务提交，目录窄化是提交后的幂等发布

`ALTER TABLE` 的成功 / 失败 SHALL 由**事务提交**决定。目录窄化的**发布**（列行改写 + 列数下调）SHALL 是事务提交之后的**幂等**步骤：重复发布 SHALL 得到相同结果。发布失败 SHALL NOT 回滚已提交的数据面（数据已提交，回滚在物理上不可能），SHALL NOT 把已提交的语句改判为失败。

#### Scenario: 提交完成后发布的目录窄化与数据面一致

- **GIVEN** 一个数据库，其某张表有三列且已有数据
- **WHEN** 执行 `ALTER TABLE t DROP COLUMN` 并执行完成
- **THEN** 目录中该表的列数与列定义 SHALL 反映被删列的移除，数据面各行 SHALL 按新列数存放

#### Scenario: 发布的幂等性

- **GIVEN** 一个数据库，其某张表已完成一次 `ALTER TABLE t DROP COLUMN`
- **WHEN** 该窄化的发布步骤被重复执行（恢复路径与正常运行路径各一次）
- **THEN** 目录的列数与列定义 SHALL 与只发布一次的结果相同

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

### Requirement: 未发布的窄化意图不因检查点而丢失

写入日志是窄化意图的**唯一**持久载体。已提交但**尚未发布**的窄化意图 SHALL NOT 因检查点而丢失：检查点 SHALL 在该意图尚未发布时**先发布**在途窄化，或**推迟**截断。

#### Scenario: 检查点后崩溃重开仍发布窄化

- **GIVEN** 一个数据库，其某张表有三列且已有数据；一次 `ALTER TABLE t DROP COLUMN` 的数据面改写已提交、目录窄化尚未发布
- **WHEN** 该库执行一次检查点，随后进程崩溃
- **THEN** 重新打开该库并读取表 `t` 时，SHALL 仍发布该已提交的窄化
- **AND** 重开后的目录列数 SHALL 与数据面行宽一致

#### Scenario: 未提交的窄化不因检查点而被发布

- **GIVEN** 一个数据库，其某张表有三列且已有数据；一次 `ALTER TABLE t DROP COLUMN` 未提交
- **WHEN** 该库执行一次检查点，随后进程崩溃并重新打开
- **THEN** 恢复 SHALL NOT 发布该窄化
- **AND** 目录列数 SHALL 与数据面行宽一致
