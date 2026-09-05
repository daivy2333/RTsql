# ms07-rest-tx-checkpoint-pushdown

## Purpose

MS07 剩余三项基础能力：T04 显式事务（`Database::begin/commit/rollback` + 事务内执行 + 多表回滚）；T05 Checkpoint 真实工作（恢复端消费位点、静默吞错显式化）；T06 谓词/LIMIT 下推到扫描。本能力（capability）覆盖三个 Iteration，每项独立验收。

## ADDED Requirements

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