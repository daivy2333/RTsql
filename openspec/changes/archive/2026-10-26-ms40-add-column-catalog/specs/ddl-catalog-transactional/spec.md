# ddl-catalog-transactional Delta

## ADDED Requirements

### Requirement: 被放弃的 ADD COLUMN 不留下已落盘的列数上调

`ALTER TABLE … ADD COLUMN` 的语句 future 在列数刷盘期间被丢弃时（超时到期同样是丢弃该 future），目录列数 SHALL 回到语句执行前的值，可见列定义 SHALL 同时回到语句执行前的状态。列数刷盘已经成功时，该回退 SHALL 使已落盘的列数上调不再留在磁盘上：处理结束后不经调用方额外刷盘，进程崩溃再打开，新列 SHALL NOT 可见，列数 SHALL 仍是语句执行前的值。

已经把成功结果交回调用方之后再丢弃 future，该语句视为已提交，不适用本 Requirement。刷盘返回错误的失败路径不适用本 Requirement 的落盘回退，仍由 `ddl-layout-publication` 的失败补偿约束（补偿本身不刷盘）。

#### Scenario: 列数刷盘期间被放弃的 ADD COLUMN 重开后不可见

- **GIVEN** 表 `t` 有两列且已有数据
- **WHEN** `ALTER TABLE t ADD COLUMN m INT` 的语句 future 在列数刷盘期间被丢弃，且该次刷盘可能已经把上调后的列数写入磁盘
- **THEN** 库处理完这次放弃之后，目录列数 SHALL 等于语句执行前的值，可见列定义 SHALL 不含列 `m`
- **AND** 不经调用方 `close` 或 checkpoint，进程崩溃后重新打开该库，列数 SHALL 仍等于语句执行前的值，列 `m` SHALL NOT 可见
