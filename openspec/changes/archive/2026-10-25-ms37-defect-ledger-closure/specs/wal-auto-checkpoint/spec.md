# wal-auto-checkpoint Delta

## MODIFIED Requirements

### Requirement: 自动 checkpoint 失败传播

自动 checkpoint 的任一步骤失败（I/O 错误等）SHALL 以错误响应报告触发该边界的提交语句，错误 SHALL 点名 checkpoint 失败；该语句的已提交数据 SHALL 不受影响（WAL 已 fsync，提交先于 checkpoint）。失败后水位仍超限，后续提交边界 SHALL 自然重试。

#### Scenario: checkpoint 失败诚实报告且数据不丢

- **GIVEN** 注入的 checkpoint 步骤失败（夹具态）
- **WHEN** 触发边界的提交完成
- **THEN** 收到点名 checkpoint 失败的错误；已提交行经重开可查（WAL 完整）

#### Scenario: 提交确认不包含 checkpoint，也不构成断电验证

- **GIVEN** 事务提交已经从 `WALBuffer::append_commit_and_wait` 返回；该返回发生在对应批次 `WalWriter::write_batch` 的 `sync_all` 返回之后
- **WHEN** 对照本要求里「已提交数据不受影响（WAL 已 fsync，提交先于 checkpoint）」
- **THEN** 该承诺是：这次 `sync_all` 已经返回，且 checkpoint 尚未被这次提交确认所包含
- **AND** 只观察到 `sync_all` 被调用，或只用进程 SIGKILL 后重开，都不足以把结果写成已验证的完整断电安全
- **AND** 本场景不新增断电后仍可恢复的承诺，也不改变 checkpoint 失败后已提交行经重开可查的既有条款
