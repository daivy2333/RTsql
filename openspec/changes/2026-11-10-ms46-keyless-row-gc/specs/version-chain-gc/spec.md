# version-chain-gc Specification

## Purpose

`TableMeta::gc_table` 回收版本链上不再是链头的已提交版本。本域规定无键行的链也进入这次回收，并且带键行现有结果保持不变。

## ADDED Requirements

### Requirement: 无键行旧版本按既有谓词回收

主键值不能进入主键索引的行，`TableMeta::gc_table` SHALL 到达它的版本链。不能进入主键索引，指主键列声明类型不是 `Int`（或主键列位置不存在），或者该列类型是 `Int` 但链上最新一个物理带有主键列的版本，其主键值 `Value::to_key()` 为 `None`。

对这样的链，回收谓词 SHALL 与现有带键链相同：链头留下；`commit_tx_id()` 为 `Some` 且不是链头的版本，SHALL 经 `delete_tuple_from_data_page` 删除。`commit_tx_id()` 为 `Some` 包括普通提交号，也包括删除哨兵。谓词 SHALL NOT 额外因为 `is_deleted()` 为真或 `create_tx_id()` 为 0 而留下旧版本，也 SHALL NOT 查询快照或活跃事务集合。

链头是数据页链上没有其他槽的 `next_version` 指向它的槽。键身份取链头起沿 `next_version` 遇到的第一个物理主键列：空墓碑（元组实际列数不超过主键位置）SHALL NOT 被默认值补成 NULL 后再当成无键。整条链都没有物理主键列时，SHALL NOT 回收该链。

`gc_table` 的返回计数 SHALL 包含由此删除的版本。函数签名保持 `Result<usize>`。本变更 SHALL NOT 新增 `gc_table` 的调用者。

数据页链的 `next_page_id` 成环时，SHALL 返回 `StorageError::Internal`，其字符串为 `data page chain cycle while garbage collecting table '{name}'`，`{name}` 是表名。发现该环之后，SHALL NOT 再删除无键槽。无键链的 `next_version` 绕回本链已访问的 row_id 时，SHALL 返回 `StorageError::Internal`，其字符串为 `version chain cycle while garbage collecting table '{name}'`，并且 SHALL NOT 删除这条环上的槽。页头槽数越界或槽负载越界 SHALL 沿用数据页走查已有的明确错误。短于版本头的槽不是版本，SHALL 跳过。

#### Scenario: 已提交无键链回收旧版本并保留链头

- **GIVEN** 整数主键表中一行已提交两次非键更新，再已提交地把主键更新为 NULL
- **AND** 调用 `gc_table` 之前，这三个旧 row_id 仍能读回
- **WHEN** 调用 `gc_table`
- **THEN** 返回的清理计数为 3
- **AND** 这三个旧 row_id 为 `SlotNotFound`
- **AND** `SELECT` 只返回该 NULL 主键行的最新非键值

#### Scenario: 只有链头的无键行不回收

- **GIVEN** 整数主键表只有一行已提交的 NULL 主键，没有更旧版本
- **WHEN** 调用 `gc_table`
- **THEN** 清理计数为 0
- **AND** `SELECT` 仍返回该行

#### Scenario: 非 INT 主键表的已提交行不回收

- **GIVEN** `STRING PRIMARY KEY` 的表有已提交插入，且这些行没有更旧版本
- **WHEN** 调用 `gc_table`
- **THEN** 清理计数为 0
- **AND** `SELECT` 仍返回这些行

#### Scenario: 未提交无键链头保留且已提交前驱回收

- **GIVEN** 已提交的整数主键行上有一个尚未提交的更新，该更新把主键改为 NULL
- **WHEN** 调用 `gc_table`
- **THEN** 未提交链头仍可按原 row_id 读回，且 `commit_tx_id()` 为 `None`
- **AND** 已提交前驱为 `SlotNotFound`

#### Scenario: 同表的带键链与无键链一次回收

- **GIVEN** 同一张整数主键表里，一行仍由原整数键指向且有一个已提交旧版本，另一行是「已提交无键链回收旧版本并保留链头」的前置
- **WHEN** 调用一次 `gc_table`
- **THEN** 带键行的最新版本仍能用原键找到，它的旧版本被回收
- **AND** 无键链的清理结果与「已提交无键链回收旧版本并保留链头」相同
- **AND** 清理计数等于两边回收的版本数之和

#### Scenario: 无键旧版本的溢出链按既有释放规则回收

- **GIVEN** 无键链的旧版本槽是溢出行，链头是页内行
- **WHEN** 调用 `gc_table`
- **THEN** 旧版本槽被删除，其溢出链按 `delete_tuple_from_data_page` 的现有规则释放
- **AND** 这些溢出页在释放被提升之前不可分配，提升之后可被再次分配
- **AND** 链头仍可按原 row_id 读回

#### Scenario: 数据页链成环

- **GIVEN** 表的数据页 `next_page_id` 成环，且主键索引路径没有可删除的旧版本
- **WHEN** 调用 `gc_table`
- **THEN** 返回 `StorageError::Internal`，字符串为 `data page chain cycle while garbage collecting table '{name}'`
- **AND** 环被发现之后，该行的槽仍可按原 row_id 读回

#### Scenario: 无键版本链成环

- **GIVEN** 无键链头不在主键索引中，从它沿 `next_version` 会回到本链已经访问过的 row_id
- **WHEN** 调用 `gc_table`
- **THEN** 返回 `StorageError::Internal`，字符串为 `version chain cycle while garbage collecting table '{name}'`
- **AND** 这条环上的槽仍可按原 row_id 读回

### Requirement: 带键行 GC 合同保持

主键索引 `scan_all` 返回的 row_id 所标识的链，SHALL 只由 `gc_table` 现有的索引循环处理：沿 `scan_all` 的顺序，从该 row_id 走到 `next_version`，删除 `commit_tx_id()` 为 `Some` 且不等于该 row_id 的版本，删除函数为 `delete_tuple_from_data_page`。这条循环 SHALL NOT 增加环检测，SHALL NOT 改用快照判断。

键身份属于可进入主键索引的链，即使它的链头不在 `scan_all` 结果里，SHALL NOT 被无键路径回收。已提交 `DELETE` 会去掉索引条目并写下空元组墓碑；这种带键删除链属于本款。

#### Scenario: 索引仍指向的带键链保持现有清理结果

- **GIVEN** 整数主键行的链头仍在主键索引中，且链上有已提交旧版本
- **WHEN** 调用 `gc_table`
- **THEN** 旧的已提交版本被删除，链头仍由原索引键指向
- **AND** 现有 `tests/gc_test.rs` 的三个用例保持其断言
- **AND** `tests/overflow_page_test.rs` 的 `gc_releases_oversized_version_chain` 清理计数仍为 1，溢出页仍在释放提升前不可分配

#### Scenario: 已删除的带键链不回收

- **GIVEN** 整数主键行已提交一次非键更新，再已提交 `DELETE`，主键索引中已经没有该键
- **WHEN** 调用 `gc_table`
- **THEN** 该链贡献的清理计数为 0
- **AND** 删除前由索引记下的 row_id 仍可读回
