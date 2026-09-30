# wal-auto-checkpoint Specification

## Purpose
约束 WAL 大小水位自动 checkpoint：读写会话在每个提交边界（auto-commit DML 提交后 / CTAS 隐式提交后 / 显式 COMMIT 后）检查 WAL 自上次 checkpoint 以来的累计追加字节，达水位（默认 16 MiB，env `RTSQL_WAL_CHECKPOINT_BYTES` 与 lib 设置器可调，0 禁用）即同步执行既有九步 checkpoint 使 `.wal` 物理回落；活跃事务顺延；checkpoint 失败点名传播且已提交数据不受影响（CTAS 表存活）；死记录数阈值接口删除。来源：MS26 T5/T6（change `2026-09-30-ms26-embedded-runtime`，长会话 WAL 有界）。

## Requirements

### Requirement: 提交边界字节水位触发

读写会话 SHALL 在每个提交边界（auto-commit 语句提交完成后 / 显式 `COMMIT` 完成后）检查 WAL 自上次 checkpoint 以来的累计追加字节数：达到水位阈值（默认 16 MiB，可配置，见「水位配置」Requirement）SHALL 同步执行一次完整 checkpoint（既有九步路径：刷脏页 → fsync → 写位点 → 重写截断 WAL → 重置计数，含 MS18 零工作跳过与位点事务水位闭包）。触发后 WAL 文件 SHALL 物理回落（重写截断），后续追加自文件头重新计 LSN（既有语义）。水位检查 SHALL 使用内存字节计数器（WAL 追加时累加、checkpoint 完成时归零），SHALL NOT 要求每语句一次文件 `metadata()` 调用。

#### Scenario: 长会话 WAL 有界

- **GIVEN** 低水位（如 64 KiB）配置的读写会话与可容纳多批的表
- **WHEN** 连续执行多批 INSERT 使 WAL 累计超过水位
- **THEN** 提交继续可用，`.wal` 文件长度在触发边界后物理回落（远小于累计追加量），数据完整（全量行可查）

#### Scenario: 触发后重开恢复代价有界

- **GIVEN** 同上长会话正常 close 后
- **WHEN** 重新打开数据库
- **THEN** 恢复重放仅涉及最后一批未 checkpoint 记录（位点语义不变，恢复行为与手动 checkpoint 后一致）

#### Scenario: 零工作跳过保持

- **GIVEN** 无任何写入的会话提交边界（纯 SELECT）
- **WHEN** 到达提交边界检查
- **THEN** 不触发 checkpoint I/O（MS18 零工作守卫语义保持）

### Requirement: 水位配置

水位阈值 SHALL 默认 16 MiB（16 × 1024 × 1024 字节）。配置通道 SHALL 覆盖：环境变量 `RTSQL_WAL_CHECKPOINT_BYTES`（正整数，字节数）与 lib 配置通道（形态随 design 定稿——打开后设置器）。配置为 0 SHALL 禁用自动 checkpoint（保持 close()/手动 checkpoint 既有语义）。非法配置（负数、非数字、溢出）SHALL 在打开时点名拒绝。

#### Scenario: 环境变量覆盖默认值

- **GIVEN** `RTSQL_WAL_CHECKPOINT_BYTES=65536` 的会话
- **WHEN** 追加约 64 KiB 的 WAL 记录后到达提交边界
- **THEN** 自动 checkpoint 触发（`.wal` 回落）

#### Scenario: 0 禁用

- **GIVEN** `RTSQL_WAL_CHECKPOINT_BYTES=0` 的长会话
- **WHEN** 累计追加远超默认水位的写入
- **THEN** 不发生自动 checkpoint（`.wal` 持续增长），close() 仍执行完整 checkpoint（既有语义）

### Requirement: 显式事务活跃时顺延

提交边界检查时若存在其他活跃事务（并发会话或未完成的显式事务），SHALL 顺延自动 checkpoint 到后续提交边界（顺延期间不阻塞、不失败，水位可能暂时超过阈值）；下一个无活跃事务的提交边界 SHALL 重新检查并在超阈值时触发。单会话串行（CLI one-shot / 显式事务提交后的自动路径）SHALL 正常触发，不受本条影响。

#### Scenario: 显式事务期间顺延

- **GIVEN** 低水位配置，会话 `BEGIN` 后写入使水位超限
- **WHEN** 事务内语句提交边界到达（事务仍活跃）
- **THEN** 不触发自动 checkpoint，事务继续；`COMMIT` 完成后的边界触发 checkpoint

### Requirement: 自动 checkpoint 失败传播

自动 checkpoint 的任一步骤失败（I/O 错误等）SHALL 以错误响应报告触发该边界的提交语句，错误 SHALL 点名 checkpoint 失败；该语句的已提交数据 SHALL 不受影响（WAL 已 fsync，提交先于 checkpoint）。失败后水位仍超限，后续提交边界 SHALL 自然重试。

#### Scenario: checkpoint 失败诚实报告且数据不丢

- **GIVEN** 注入的 checkpoint 步骤失败（夹具态）
- **WHEN** 触发边界的提交完成
- **THEN** 收到点名 checkpoint 失败的错误；已提交行经重开可查（WAL 完整）

### Requirement: 既有 checkpoint 语义零回归

手动 `Database::checkpoint()`、`close()` checkpoint、MS18 零工作跳过、位点水位推进（MS17-T02 D8）与恢复重放语义 SHALL 逐字节不变。既有死的记录数阈值接口（`should_checkpoint`/`set_checkpoint_threshold`/`get_checkpoint_threshold`，无消费方）SHALL 删除，SHALL NOT 留存与字节水位并存的第二套触发机制。

#### Scenario: 手动与 close checkpoint 零回归

- **WHEN** 运行 `tests/checkpoint_redo_reduction_test.rs`、`tests/checkpoint_skip_test.rs` 与完整测试套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改
