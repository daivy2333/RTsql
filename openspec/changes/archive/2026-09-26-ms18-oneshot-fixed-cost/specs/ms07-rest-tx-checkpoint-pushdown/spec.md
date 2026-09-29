## ADDED Requirements

### Requirement: Checkpoint 零工作跳过

当 WAL 自上次 checkpoint 以来无追加记录（未决记录为零）**且**缓冲池无脏页时，checkpoint SHALL 为整体无操作：SHALL NOT 刷脏页、SHALL NOT 写 checkpoint 位点、SHALL NOT 重写或截断 WAL。任一条件非空（WAL 有未决追加或缓冲池有脏页）时，checkpoint SHALL 保持既有全量行为（刷脏页 + 写位点 + WAL 重写截断）逐字节不变。跳过判定 SHALL 覆盖全部 checkpoint 触发路径（close 自动路径、显式 checkpoint 调用、open 恢复后路径），公开 API 签名 SHALL 不变。跳过与执行两种结果下，重开（含崩溃恢复）语义 SHALL 与既有行为等价；「close 即落盘」承诺 SHALL 不变——无未决工作时本无落盘工作，有未决工作时（含仅 DDL 产生的脏页）SHALL 照常落盘。

#### Scenario: 零写入会话 close 跳过 checkpoint

- **GIVEN** checkpoint-clean 的已有数据库（WAL 仅含上次 checkpoint 标记、无脏页，位点文件已存在）
- **WHEN** 打开后仅执行只读语句（如 `SELECT`）随后 `close()`
- **THEN** close 不写位点、不重写 WAL（位点文件内容逐字节不变、`.wal` 内容不变）
- **AND** 重开后全部数据与 schema 完整可查（等价于既有行为）

#### Scenario: 从未 checkpoint 的新库零工作 close 不产生位点文件

- **GIVEN** 新建且未执行任何写语句的数据库（无既有位点文件）
- **WHEN** 打开后不执行语句（或仅只读）随后 `close()`
- **THEN** close 不创建位点文件（既有行为为无条件 checkpoint 必创建位点）
- **AND** 重开语义不变（无位点文件按既有「从未 checkpoint」路径处理）

#### Scenario: DML 会话 close 保持全量 checkpoint

- **GIVEN** 任意数据库
- **WHEN** 会话执行 INSERT/UPDATE/DELETE 后 `close()`
- **THEN** 既有全量 checkpoint 行为逐字节不变：脏页落盘、位点写入、WAL 重写截断
- **AND** 重开后数据完整、redo 收敛（既有恢复语义不变）

#### Scenario: DDL-only 会话 close 保持全量 checkpoint（schema 持久化依赖刷脏页）

- **GIVEN** 任意数据库
- **WHEN** 会话仅执行 CREATE TABLE（DDL 不写 WAL）后 `close()`
- **THEN** close 检出缓冲池脏页，执行全量 checkpoint（SHALL NOT 因 WAL 空而跳过）
- **AND** 重开后新建表存在、schema 完整（不因跳过丢失）

#### Scenario: crash 脏库打开后 close 保持全量 checkpoint

- **GIVEN** 上次会话异常退出留下非空 WAL 的数据库
- **WHEN** 打开（恢复重放）后仅执行只读语句并 `close()`
- **THEN** close 检出 WAL 非空，执行全量 checkpoint：位点推进、WAL 重写截断
- **AND** 重开 redo 收敛（既有恢复语义不变）

#### Scenario: 显式 checkpoint 零工作时无操作

- **GIVEN** WAL 空且无脏页的已打开数据库
- **WHEN** 调用公开 `checkpoint()`
- **THEN** 返回成功且不产生位点写入与 WAL 重写副作用
- **AND** 后续恢复语义不受影响
