# catalog-update-persistence Specification

## Purpose
约束 catalog（`__tables` 系统表）行更新的持久化正确性：等长字段更新（tail/根页/列数等定宽字段）经原地覆写活行 slot 完成（零泄漏、零行副本增长、窒息页可写），任何更新失败点名传播、无静默成功路径；变长更新（如 CREATE INDEX 注册段）沿链跨页追加新副本并清理旧副本，同名行确定性去重；跨进程背靠背写入五面一致（count/扫描/MIN-MAX/点查/dump），索引根分裂写回持久。来源：change `2026-09-30-catalog-durability-and-cli-surface` Iteration 000（P0 one-shot 跨进程写入静默丢数据正确性红线收口）。

## Requirements

### Requirement: catalog 定长字段更新原地持久化

catalog 行的等长字段更新（`data_page_tail`、`index_root_page_id`、unique/secondary ordinal root、`column_count` 等定宽字段值变化）SHALL 通过原地覆写活行 slot 数据区完成：定位匹配表名的行 slot，校验新载荷与旧行等长后覆写字节，不追加新行、不删除旧行、不修改页头（slot_count / free_space_offset / next_logical_id 不变）。等长更新 SHALL NOT 依赖页剩余空间——在已无追加空间的 catalog 页上 SHALL 照常成功。更新 SHALL 在下一次干净关闭后持久可读（重开读回值与最后一次更新一致）。

#### Scenario: 多轮跨进程 tail 更新持久化

- **GIVEN** 一张表已持久化若干行且其 catalog 行位于更新残骸较多的页
- **WHEN** 连续多轮「打开 → 写入新数据页 → 干净关闭」（每轮为独立会话）
- **THEN** 每轮更新后的 tail 在重开读回与写入一致，全部轮次的数据页从 head 可达，行数单调递增

#### Scenario: 窒息页上的原地更新成功

- **GIVEN** `__tables` 页数据区仅剩空间不足以再追加一行（历史残骸形态）
- **WHEN** 该表发生等长字段更新（如 tail 推进）
- **THEN** 更新成功，页头计数不变化，不产生新的行副本，重开后读回新值

#### Scenario: 原地更新不产生行副本增长

- **GIVEN** 任一 catalog 页上的表行
- **WHEN** 连续执行 ≥200 次等长字段更新
- **THEN** 该页 slot 数量、free_space_offset 与更新前一致（或仅因本次活行存在而恒定），无残骸字节增长

### Requirement: catalog 更新失败诚实化

catalog 行更新的任何失败（页满且无法原地完成、I/O 错误、行不存在等）SHALL 以携带表名与原因的错误向上传播；SHALL NOT 存在静默成功路径。调用方（写入路径 tail 推进、truncate/ALTER 的根与计数写回、B-tree 分裂的 catalog 写回、CREATE INDEX 注册、恢复期写回）SHALL 将该错误传播为对应操作的可观察失败，写入副作用语义沿调用方既有事务/零副作用契约保持。

#### Scenario: 页满且非等长更新点名失败

- **GIVEN** `__tables` 页无追加空间且更新载荷与旧行不等长且链上无可用空间页
- **WHEN** 执行该更新
- **THEN** 返回携带表名的错误，调用方操作以失败呈现，不产生部分可见的新旧行双副本

#### Scenario: 写入路径 tail 推进失败零副作用

- **GIVEN** catalog 更新通道故障（注入的 I/O 失败）
- **WHEN** INSERT 触发数据页自动分配后的 tail 更新
- **THEN** 该语句以错误结束且已写入数据页的行不对外可见（沿既有事务提交边界语义），错误信息可定位

### Requirement: 变长 catalog 行更新跨页重定位

长度变化的 catalog 行更新（如 CREATE INDEX 追加二级索引段）SHALL 通过链上跨页追加新副本并清理旧副本完成：空间不足时沿 `__tables` 页链追加（必要时分配新页并链接），SHALL NOT 因当前页满而失败或静默丢弃。同表名行副本 SHALL 恰好一份；`scan_tables` 对同名多副本 SHALL 确定性去重（保留 logical_id 最大者），使两步清理的进程中断窗口不产生歧义读。

#### Scenario: CREATE INDEX 注册跨页持久

- **GIVEN** `__tables` 页剩余空间不足以容纳增长后的行
- **WHEN** `CREATE INDEX` 注册二级索引段
- **THEN** 注册成功，干净关闭重开后该索引仍在（可路由、可 DROP），同名行恰一份

#### Scenario: 中断窗口的双副本确定性读

- **GIVEN** 链上存在同名两份行副本（模拟追加后未及清理的中断现场）
- **WHEN** `scan_tables` / 表打开读取该表
- **THEN** 确定性去重（保留 logical_id 最大者），恰返回一份、无歧义（logical_id 为页内计数；跨页回绕情形下该规则是确定性收口而非时序断言，窗口本身由 design D2 接受）

### Requirement: 跨进程写序 durability

以 one-shot 形态连续多轮「独立会话写 → 关闭 → 下一会话追加」（每轮语句跨多个数据页、总量跨过 catalog 更新窒息阈值）SHALL 全程无静默丢失：行数单调递增且终值等于各轮写入之和；`COUNT(*)`、范围扫描、`MAX/MIN`、点查、dump 行数五者一致；文件增长与数据量线性同阶。

#### Scenario: 背靠背多轮大批量写入零丢失

- **GIVEN** 空库中一张带主键的表（历史上此形态在 ~10 轮后触发静默丢失）
- **WHEN** 15 轮独立会话各插入 1000 行（每轮单语句、轮间无其他操作）
- **THEN** 每轮后行数恰为轮数 × 1000，最终 dump 行数 = 15000，点查首/中/尾行全部命中

#### Scenario: 单进程大事务形态零回归

- **GIVEN** 同库同表
- **WHEN** 单会话内多语句/单事务/CSV import 写入超过窒息阈值行数
- **THEN** 行为与修复前一致（本就正确），count 与 dump 一致

### Requirement: 索引根更新持久化

PK、UNIQUE、二级索引 B-tree 的根分裂 catalog 写回 SHALL 在分裂发生时持久化（原地路径），干净关闭重开后树完整可用：PK 点查/扫描一致，UNIQUE 约束继续强制（重插同值拒绝），二级索引等值路由继续可达且结果与扫描等价。

#### Scenario: UNIQUE 树根分裂后重开仍强制

- **GIVEN** 带 INT UNIQUE 列的表已写入若干行
- **WHEN** 持续插入促使唯一索引树根分裂并干净关闭重开
- **THEN** 重开后再插入同值被唯一冲突拒绝；非冲突插入正常

#### Scenario: 二级索引根分裂后重开仍路由

- **GIVEN** 表已建二级索引并写入数据
- **WHEN** 持续插入促使该索引树根分裂并干净关闭重开
- **THEN** 重开后等值查询仍路由索引计划且结果与全扫描一致；`DROP INDEX` 可达

### Requirement: 既有语义零回归

本域修复 SHALL NOT 改变：页格式（页大小、slot 布局、header 布局）；lib `Database::open` 对缺失路径的建库行为；WAL / checkpoint / 恢复链路行为（MS18 零工作跳过、MS26 水位判定）；单进程全部既有写入语义；既有测试套件除本 change 白名单校准外零修改通过。

#### Scenario: 全量回归

- **GIVEN** change 收尾工作区
- **WHEN** 运行全量测试（M19 分批）
- **THEN** 0 failures；除登记的测试文件新增/校准外无既有用例修改
