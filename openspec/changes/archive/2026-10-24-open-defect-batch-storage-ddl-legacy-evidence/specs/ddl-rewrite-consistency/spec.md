# ddl-rewrite-consistency Specification

## Purpose

`ALTER TABLE DROP COLUMN` 的**数据面**改写契约：改写 SHALL NOT 丢失已提交行；被放弃的改写 SHALL NOT 隐藏它已开始改写的行；改写在途期间读路径 SHALL NOT 静默返回位置错位的值。本域承载 ISS22（改写丢行）、ISS24（被放弃的 DDL 隐藏行）与 ISS25（提交点窗口内的位置性错读）。目录面的事务性由既有域 `ddl-catalog-transactional` 规定，墓碑的可见性由既有域 `mvcc-tombstone-visibility` 规定，本域不重述二者。

## ADDED Requirements

### Requirement: 列改写不丢失已提交行

`ALTER TABLE DROP COLUMN` 的数据面改写 SHALL NOT 丢失任何已提交行。改写过程中数据页链 SHALL 保持**单一链**：任何时刻该表的数据页链 SHALL 只有一条可达路径，被改写产生的所有新版本行 SHALL 位于该路径上。

#### Scenario: 跨页容量边界的改写后行数不减

- **GIVEN** 一张行数足以跨过数据页容量边界的表
- **WHEN** 对该表执行 `ALTER TABLE DROP COLUMN`
- **THEN** 改写完成后该表的已提交行数 SHALL 与改写前相同
- **AND** 改写后每个原有主键 SHALL 仍可查到，且其列值按新列数呈现

#### Scenario: 改写期间数据页链不分叉

- **GIVEN** 一张行数足以跨过数据页容量边界的表
- **WHEN** 对该表执行 `ALTER TABLE DROP COLUMN`
- **THEN** 该表的数据页链 SHALL 只有一条可达路径
- **AND** 链上 SHALL NOT 存在任何不可达的孤立数据页

#### Scenario: 改写后重开库行数不减

- **GIVEN** 一张行数足以跨过数据页容量边界的表，其 `ALTER TABLE DROP COLUMN` 已完成
- **WHEN** 关闭并重新打开该库并查询该表
- **THEN** 查到的已提交行数 SHALL 与改写前相同
- **AND** 完整性扫描的 error 级发现数为 0

### Requirement: 被放弃的列改写不隐藏已改写的行

`ALTER TABLE DROP COLUMN` 的语句被放弃（丢弃或超时）并完成回收之后，该表 SHALL NOT 少掉任何一行：该语句在放弃前已经改写过的行 SHALL 仍然可见，且其可见性 SHALL 与语句执行前一致。

#### Scenario: 放弃的删列改写后行数不减

- **GIVEN** 一张有三列且已有数据的表 `t`
- **WHEN** `ALTER TABLE t DROP COLUMN` 的语句在改写期被放弃，随后该语句的事务被回收
- **THEN** 回收完成之后 `t` 的可见行数 SHALL 与语句执行前相同
- **AND** 语句已开始改写的那一行 SHALL 仍然可见

#### Scenario: 已回滚的改写墓碑不抑制其前驱版本

- **GIVEN** 一次被放弃的列改写，其改写墓碑已被回滚中和
- **WHEN** 读取该表的行
- **THEN** 该墓碑 SHALL NOT 抑制它指向的前驱版本
- **AND** 任何以 `next_version` 链接构造的抑制集合 SHALL NOT 把「被已回滚版本指向的行」判为被抑制

#### Scenario: 放弃的删列改写后重开库行数不减

- **GIVEN** 一张有三列且已有数据的表 `t`，其 `ALTER TABLE t DROP COLUMN` 的语句被放弃并完成回收
- **WHEN** 关闭并重新打开该库并查询 `t`
- **THEN** 查到的行数 SHALL 与语句执行前相同
- **AND** 目录列数与数据面行宽一致（既有域 `ddl-catalog-transactional` 的保证面）

### Requirement: 改写在途期间读路径不静默错读

`ALTER TABLE DROP COLUMN` 的改写从开始到提交点之间，该表处于**窄化在途**状态。处于该状态的表 SHALL 被读路径**显式识别**：读路径 SHALL NOT 按旧列数对已窄化的行做位置取值并以尾部 NULL 补齐后返回。

#### Scenario: 在途窄化的表被读时显式拒绝

- **GIVEN** 表 `t` 有三列，`ALTER TABLE t DROP COLUMN` 的改写已写出窄化行而目录尚未窄化
- **WHEN** 读路径读取 `t`
- **THEN** 读路径 SHALL 给出明确的拒绝（点名该表正在被修改），SHALL NOT 返回任何按位置取值的结果
- **AND** 该拒绝 SHALL NOT 被表现为查询成功但列值错位

#### Scenario: 改写完成后读路径恢复正常

- **GIVEN** 表 `t` 的 `ALTER TABLE t DROP COLUMN` 已完成（目录已窄化）
- **WHEN** 读路径读取 `t`
- **THEN** 读路径 SHALL 正常返回窄化后的行，SHALL NOT 再给出在途拒绝

#### Scenario: 未改写的表读路径不受影响

- **GIVEN** 一个没有任何在途改写的表
- **WHEN** 读路径读取该表
- **THEN** 读路径 SHALL 正常返回该表的行，SHALL NOT 给出在途拒绝

#### Scenario: 窄行与宽行的解码语义不变

- **GIVEN** `ALTER TABLE ADD COLUMN` 之后遗留的窄行（合法形态）
- **WHEN** 读路径按当前 schema 解码该行
- **THEN** 解码 SHALL 保持既有的「窄行补 NULL + 声明 DEFAULT 叠加」语义
- **AND** 本域 SHALL NOT 改变 `ADD COLUMN` 路径的宽度感知解码行为
