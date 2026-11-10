## Purpose

规定 `gc_table` 如何回收无键行的已提交旧版本，以及带键行在同一次调用里保持原来的回收结果。

## ADDED Requirements

### Requirement: 无键链的已提交旧版本被回收

键位值不能进入主键索引的行（主键列上的值 `to_key()` 为 `None`，包括 NULL 和非整数键位），其数据页上的已提交旧版本 SHALL 被 `TableMeta::gc_table` 回收。回收判定 SHALL 与带键链相同：从该行当前链头沿 `next_version` 向下，`commit_tx_id()` 为 `Some` 且不是该链头的版本被删除。删除 SHALL 调用现有的 `delete_tuple_from_data_page`。返回的清理计数 SHALL 包含这些版本。链头本身 SHALL NOT 被删除。

#### Scenario: 整数键更新为 NULL 后旧版本被回收

- **GIVEN** 表 `t(id INT, v INT)` 已提交一行 `id = 1`，随后已提交地把 `v` 更新一次，再已提交地把 `id` 设为 NULL
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 2
- **AND** `SELECT id, v FROM t` 只返回该 NULL 键的当前值
- **AND** 设为 NULL 之前主键索引指向的槽不再能读到

#### Scenario: 同表的带键链与无键链一起回收

- **GIVEN** 同一张表上有一条仍带整数键的版本链和一条键已变为 NULL 的版本链，每条都有两个已提交旧版本
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 4
- **AND** 带键行的主键索引仍指向最新版本且该版本的值可读
- **AND** `SELECT` 仍返回无键行的当前值，且查询结果不含这两条链的旧值

#### Scenario: 无键旧版本的溢出页按现有释放点回收

- **GIVEN** 已提交超页行随后已提交地变成 NULL 键的页内短版本
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 1，短版本仍能被 `SELECT` 读到
- **AND** 在调用存储层的 `promote_free_releases` 之前，该超页版本的链页不能被 `allocate_page` 取回
- **AND** 提升之后，这些链页能被 `allocate_page` 取回

### Requirement: 链头与未提交版本不回收

`gc_table` SHALL NOT 删除本次遍历的链头，SHALL NOT 删除 `commit_tx_id()` 为 `None` 的版本。只有一个版本的无键行 SHALL 保持该版本，清理计数为 0。未提交的无键链头 SHALL 保留；它的已提交前驱 SHALL 按上一 Requirement 回收。

#### Scenario: 只插入 NULL 键的一行

- **GIVEN** 表 `t(id INT, v INT)` 只已提交插入一行，`id` 为 NULL
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 0
- **AND** `SELECT id, v FROM t` 仍返回这一行

#### Scenario: 非整数键列只有一个版本

- **GIVEN** 表 `t(s STRING, n INT)` 只已提交插入一行
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 0
- **AND** `SELECT s, n FROM t` 仍返回这一行

#### Scenario: 未提交的无键链头保留

- **GIVEN** 已提交的整数键行，在尚未提交的显式事务里被 `UPDATE` 为 NULL 键
- **WHEN** 在该事务提交前调用 `gc_table`，再在同一事务执行 `SELECT`
- **THEN** 返回的清理计数为 1
- **AND** `SELECT` 只返回这个未提交的新值
- **AND** 更新前主键索引指向的槽不再能读到

#### Scenario: 二级索引在回收后仍找到无键链头

- **GIVEN** 无键链头的非键整数列上有二级索引，索引条目指向该链头
- **WHEN** 调用 `gc_table`，再按该列的当前值做等值查询
- **THEN** 查询返回链头的当前值

### Requirement: 带键行的回收结果不变

主键索引 `scan_all` 能到达的版本链，`gc_table` SHALL 继续用现在的判定回收：索引指向的版本是链头，已提交且不是该链头的版本经 `delete_tuple_from_data_page` 删除，未提交链头保留。已提交 `DELETE` 之后不再出现在主键索引里的墓碑链 SHALL NOT 被回收。同一次调用 SHALL NOT 删除主键索引当前指向的槽。

#### Scenario: 带键行的三次已提交版本

- **GIVEN** 整数主键列上的一行经过插入和两次已提交的键更新
- **WHEN** 调用 `gc_table`
- **THEN** 清理计数至少为 2
- **AND** 主键索引仍指向最新版本，读出的值是最新键

#### Scenario: 带键未提交更新留下新版本

- **GIVEN** 已提交整数键行后，又有一次未提交的键更新
- **WHEN** 调用 `gc_table`
- **THEN** 清理计数至少为 1
- **AND** 索引指向的未提交版本仍在，且 `commit_tx_id()` 为 `None`

#### Scenario: 多条带键链各自留下最新版本

- **GIVEN** 三行整数键，每行都有三个已提交版本
- **WHEN** 调用 `gc_table`
- **THEN** 清理计数至少为 6
- **AND** 每一行的当前键仍能在主键索引里读到最新值

#### Scenario: 已提交删除的带键行不被回收

- **GIVEN** 已提交插入整数键行，再已提交 `DELETE` 该行
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 0
- **AND** 删除前该行的槽仍能读到

### Requirement: 损坏数据页按现有读路径报错

`gc_table` 遍历数据页链时，页头槽数超出槽目录容量，或槽负载越界，SHALL 返回现有的 `corrupted data page <页号>: ...` 错误并停止。SHALL NOT panic，SHALL NOT 把该页当成没有槽而返回成功。完好页上的回收判定不变。

#### Scenario: 页头槽数越界

- **GIVEN** 表上已有无键行，库能打开，该表数据页头的 `slot_count` 被写成 700
- **WHEN** 调用 `gc_table`
- **THEN** 返回的错误文本包含 `corrupted data page <该页号>` 和 `slot count 700`
- **AND** 进程不中止

#### Scenario: 槽偏移越界

- **GIVEN** 表上已有无键行，库能打开，该表数据页槽 0 的 offset 被写成 `0xFFFF`
- **WHEN** 调用 `gc_table`
- **THEN** 返回的错误文本包含 `corrupted data page <该页号>` 和 `slot 0`
- **AND** 进程不中止
