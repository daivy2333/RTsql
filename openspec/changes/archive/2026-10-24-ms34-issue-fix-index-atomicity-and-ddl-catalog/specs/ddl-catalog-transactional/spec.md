# ddl-catalog-transactional Specification

## Purpose

DDL 语句的目录面变更（列定义的增删与列数）与数据面改写之间的原子性约定：被放弃或失败的 DDL 语句 SHALL NOT 留下目录面变更。本域承载 ISS21。

## Requirements

### Requirement: 被放弃或失败的 DDL 语句不留下目录面变更

`ALTER TABLE` 的目录面变更 SHALL 只在语句确定完成之后生效。语句被放弃（丢弃或超时）或执行失败时，该语句的目录面变更 SHALL NOT 生效：目录的列定义与列数 SHALL 保持语句执行前的状态。

#### Scenario: 被放弃的 DROP COLUMN 不窄化目录

- **GIVEN** 表 `t` 有三列且已有数据
- **WHEN** `ALTER TABLE t DROP COLUMN` 的语句在执行期被放弃，随后触发该语句的事务回收
- **THEN** 目录中 `t` 的列定义与列数与语句执行前一致（被删列仍在目录中，列数未变）

#### Scenario: 失败的 DROP COLUMN 不窄化目录

- **GIVEN** 表 `t` 有三列且已有数据
- **WHEN** `ALTER TABLE t DROP COLUMN` 在执行期失败，该语句的事务被中止
- **THEN** 目录中 `t` 的列定义与列数与语句执行前一致

### Requirement: 目录变更与数据面改写同生共死

`ALTER TABLE` 的目录面变更 SHALL 与该语句的数据面改写同生共死：数据面改写未完成时目录不得先行窄化；目录窄化生效时数据面改写已经完成。

#### Scenario: 完成的 DROP COLUMN 使目录与数据面一致

- **GIVEN** 表 `t` 有三列且已有数据
- **WHEN** `ALTER TABLE t DROP COLUMN` 执行完成
- **THEN** 目录中 `t` 的列数与列定义反映被删列的移除，数据面各行按新列数存放

### Requirement: 放弃 DDL 之后重开库目录与数据面一致

被放弃的 DDL 语句被回收之后，重新打开该库 SHALL 得到目录列定义与数据面行宽一致的表。

#### Scenario: 放弃 DROP COLUMN 后重开库列数与行宽一致

- **GIVEN** 一个被放弃的 `ALTER TABLE t DROP COLUMN` 之后的库
- **WHEN** 调用方关闭并重新打开该库，读取表 `t`
- **THEN** 目录列数与数据面行宽一致：列数等于语句执行前的列数，各行按该列数存放
