# ms07-rest-tx-checkpoint-pushdown Specification

## Purpose
TBD - created by archiving change 2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown. Update Purpose after archive.

## Requirements

### Requirement: 显式事务 API 与原子性

`Database` SHALL 提供 `begin`/`commit`/`rollback` 公开方法，并允许把多条 DML/DDL 放进同一事务；事务内语句在 `commit` 前对其它并发事务不可见。无显式事务时 `execute_sql` 的行为保持现状（每条 DML 隐式自动提交）。

#### Scenario: begin 后执行多条语句并在 commit 一次性生效

- **GIVEN** 已打开的 `Database`
- **WHEN** 调用 `db.begin()` 得到事务 `tx`，随后在该事务内执行两条 `INSERT`（不同表或同一表），最后 `commit(tx)`
- **THEN** `commit` 返回成功后两张表的数据都持久可见
- **AND** 期间没有隐式自动提交发生（事务 id 在两条语句间复用；任一语句不触发独立 commit）

#### Scenario: rollback 撤销事务内全部未提交写入

- **GIVEN** 已打开的 `Database` 与一个活动事务 `tx`，事务内已 `INSERT`/`UPDATE` 若干行
- **WHEN** 调用 `rollback(tx)`
- **THEN** 事务内产生的所有版本被清理，表中不残留任何该事务的写入
- **AND** 索引随版本清理保持一致（无指向已删行的悬空索引）

#### Scenario: 错误语句使事务保持可用（不自动回滚）

- **GIVEN** 活动事务 `tx`，且其中一条语句执行返回错误（如非法 SQL 或约束失败）
- **WHEN** 继续在同一事务内执行另一条合法语句
- **THEN** 该事务仍为 Active、仍可 `commit`/`rollback`
- **AND** 错误返回值不会吞掉 `Transaction`（可继续使用）

#### Scenario: 重复 commit / commit 已 abort 事务可观察

- **GIVEN** 一个已 `commit`（或已 `rollback`）的事务 `tx_id`
- **WHEN** 再次对同一 `tx_id` 调用 commit/rollback
- **THEN** 返回明确错误（`AlreadyCommitted`/`AlreadyAborted`），且不破坏数据库状态
- **AND** 不产生重复的提交副作用

#### Scenario: 无显式事务时 execute_sql 保持隐式自动提交

- **GIVEN** 从未调用 `begin` 的 `Database`
- **WHEN** 调用 `execute_sql("INSERT ...")`
- **THEN** 行为与改造前一致：该语句自动 `begin → execute → commit`，返回成功即已持久化

#### Scenario: 事务 ID 在语句间复用且分配不冲突

- **GIVEN** 一个事务 `tx = db.begin()`
- **WHEN** 在该事务内执行多条 DML，并读取底层版本头
- **THEN** 所有写出的版本 `create_tx_id` 均为 `tx.id()`（不复用其它事务 id、不为占位 0）

### Requirement: Checkpoint 恢复端消费位点并缩减 redo

`RecoveryManager` SHALL 在恢复时消费 checkpoint 位点，跳过已被 checkpoint 覆盖的 WAL 前缀，使重启 redo 记录数随 checkpoint 收敛，而不是从 WAL 头全量重放。恢复路径的静默吞错 SHALL 改为显式传播的错误。

#### Scenario: checkpoint 后崩溃重启，redo 数量下降

- **GIVEN** 已写入若干事务、随后触发一次 `checkpoint` 的数据库
- **WHEN** 关闭并重启、执行 `full_recover`
- **THEN** 恢复只重放 checkpoint 位点之后的 WAL 记录（`redo_count` 显著小于 checkpoint 前所有记录数）
- **AND** 已持久化的数据与索引在空中 crash 后仍完整可查（无丢/无重）

#### Scenario: 无 checkpoint 时仍完整恢复

- **GIVEN** 从未 checkpoint 的数据库（无位点文件）
- **WHEN** 重启并 `full_recover`
- **THEN** 照旧从 WAL 头全量恢复已提交事务
- **AND** 行为与改造前一致

#### Scenario: 损坏/表缺失时恢复显式报错而非吞噬

- **GIVEN** WAL 中引用了已缺失的表，或数据页无法还原
- **WHEN** `full_recover` 重放该记录
- **THEN** 返回显式错误（不再是 `Err(_) => return Ok(())` / `let _ =` 静默跳过）
- **AND** 调用方（`Database::open`）可见该错误并据其决定是否中止打开

#### Scenario: checkpoint 位点文件损坏时安全退化为从头重放

- **GIVEN** checkpoint 位点文件缺失或不足 16 字节
- **WHEN** 恢复
- **THEN** 安全退化为从头（或最近有效前缀）重放，不 panic、不丢已提交数据

### Requirement: 谓词与 LIMIT 下推到扫描

planner 生成的扫描（DataScan / IndexScan / IndexScanAll）SHALL 能携带 WHERE 谓词并在扫描行迭代内直接过滤；无 Sort/OrderBy 的 LIMIT SHALL 下推进扫描，使扫描提前停止产出。查询返回的语义结果与改造前完全一致。

#### Scenario: 谓词下推后查询结果不变

- **GIVEN** 非 PK WHERE 的 `SELECT ... WHERE <pred>`（原本生成 `Filter(Scan/DataScan)`）
- **WHEN** 把 `<pred>` 下沿到扫描执行器迭代
- **THEN** 返回的行集与改造前 `Filter(DataScan)` 完全一致（含 NULL/类型边界）
- **AND** 中间不额外物化不匹配的行

#### Scenario: LIMIT 下推后提前终止

- **GIVEN** 无 `ORDER BY` 的 `SELECT ... LIMIT n`（或 `LIMIT n OFFSET m`）
- **WHEN** 下推 LIMIT 到扫描
- **THEN** 扫描在产出 `n`（+`m` 偏移）行后停止向下遍历
- **AND** 返回行数与改造前一致

#### Scenario: 有 ORDER BY 时 LIMIT 不被过早提前终止

- **GIVEN** 含 `ORDER BY` 的 `SELECT ... ORDER BY c LIMIT n`
- **WHEN** 确定排序必需全量后截取
- **THEN** LIMIT 保持在 Sort 之上执行（不下推跳过排序），返回正确的前 `n` 行

#### Scenario: PK 等值仍走 IndexScan（下推不劣化既有路径）

- **GIVEN** `WHERE pk = val` 的查询（原本 IndexScan）
- **WHEN** 引入下推后
- **THEN** 仍生成 IndexScan（或被等价的下推 IndexScan 覆盖），不回归为全表 Filter

#### Scenario: 下推在混合/复杂谓词下降级保持正确

- **GIVEN** 含 OR 分支或算子无法行内评估的复杂谓词
- **WHEN** 判断无法安全下推
- **THEN** 保留原 `Filter` 节点（扫描不携带该谓词），结果不变

### Requirement: Checkpoint 零工作跳过

当 WAL 自上次 checkpoint 以来无追加记录（未决记录为零）**且**缓冲池无脏页**且打开时 WAL 不含事务记录**时，checkpoint SHALL 为整体无操作：SHALL NOT 刷脏页、SHALL NOT 写 checkpoint 位点、SHALL NOT 重写或截断 WAL。「打开时 WAL 含事务记录」（任一非 Checkpoint 记录，无论恢复重放是否实际发生 redo）SHALL 视同未决工作参与判定——该形态（恢复会话含零 redo 形态）SHALL NOT 触发零工作跳过，详见 `wal-recovery-replay-integrity`「恢复会话 WAL 收敛（含零 redo 形态）」。任一其余条件非空（WAL 有未决追加或缓冲池有脏页）时，checkpoint SHALL 保持既有全量行为（刷脏页 + 写位点 + WAL 重写截断）逐字节不变。跳过判定 SHALL 覆盖全部 checkpoint 触发路径（close 自动路径、显式 checkpoint 调用、open 恢复后路径），公开 API 签名 SHALL 不变。跳过与执行两种结果下，重开（含崩溃恢复）语义 SHALL 与既有行为等价；「close 即落盘」承诺 SHALL 不变——无未决工作时本无落盘工作，有未决工作时（含仅 DDL 产生的脏页）SHALL 照常落盘。

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

#### Scenario: 恢复会话（零 redo 形态）不触发零工作跳过

- **GIVEN** 打开时 WAL 含未提交事务记录（恢复重放零 redo、零页写入）的数据库
- **WHEN** 该会话未发生用户写入即 `close()` 或显式 checkpoint
- **THEN** SHALL NOT 触发零工作早退，执行完整 checkpoint 截断 WAL（失败边界——修复前：跳过、WAL 滞留致 check 永久拒绝）

#### Scenario: marker-only WAL 残留不跳过

- **GIVEN** WAL 仅含空事务标记记录（Begin/Commit 无数据记录）的崩溃残留库
- **WHEN** 纯读读写打开后 close
- **THEN** 执行完整 checkpoint 截断 WAL（该形态 `redo_count=0` 但 WAL 曾含事务记录）
