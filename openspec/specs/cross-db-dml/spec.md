# cross-db-dml Specification

## Purpose

约束可写 CLI 会话中经 ATTACH 别名对附加库的写入面：INSERT VALUES（含子集列、DEFAULT）与 UPSERT/REPLACE、UPDATE、DELETE 按既有单库语义路由到目标附加库；每条 DML 在目标文件独立提交，不承诺跨文件原子性；显式主库事务与只读会话的写入边界；写入资源绑定目标库自身事务/WAL/缓冲池/metadata 并在全部退出路径统一关闭（关闭失败不撤销已提交数据）；UPDATE/DELETE 保持既有语法边界（无子查询执行能力）。INSERT SELECT（源含附加库、目标为主库或附加库）见 `sql-write-surface`。来源：MS20-T02 二期（change `2026-10-02-ms20-cross-db-dml`，I059 用户方向）。

## Requirements

### Requirement: 附加库 DML 路由

可写 CLI 会话 SHALL 将已注册别名限定的 INSERT VALUES、UPDATE、DELETE 路由到对应附加库，并沿用单库语句已支持的类型、约束、DEFAULT、UPSERT/REPLACE 与索引维护语义。裸名 SHALL 仍写主库，未命中别名 SHALL 沿用扁平名回落。

#### Scenario: 附加库写入且主库不变

- **GIVEN** 主库和附加库 o 各有表 t
- **WHEN** 在非显式事务会话对 o.t 依次 INSERT、UPDATE、DELETE
- **THEN** 仅附加库 t 发生对应修改，关闭重开后结果一致

#### Scenario: 约束错误沿用单库保证

- **GIVEN** 附加库表声明 NOT NULL 与 INT UNIQUE
- **WHEN** 对附加表执行违反约束的 DML
- **THEN** 按单库错误拒绝，当前失败语句不遗留修改

### Requirement: 每文件独立提交

非显式事务会话的每条 DML SHALL 在目标数据库独立提交；后续语句失败 SHALL NOT 撤销此前其他数据库的已提交修改。系统 SHALL NOT 宣称跨文件原子提交。

#### Scenario: 后续失败保留此前提交

- **GIVEN** 主库与附加库已打开
- **WHEN** 成功写主库、成功写附加库，然后下一条 DML 失败
- **THEN** 前两条已提交修改保持，失败语句按单库失败语义处理，CLI 停止后续语句

### Requirement: 显式事务与只读边界

显式主库事务活跃时，附加库写入（含 INSERT SELECT、UPSERT/REPLACE）SHALL 在源求值和目标写入前以 SQLSTATE `0A000` 点名拒绝且不修改附加库；CLI SHALL 沿用既有失败收口回滚主库事务。只读会话 SHALL 优先按 SQLSTATE `25006` 拒绝写入。附加库 DDL 与附加目标 CTAS SHALL 继续拒绝。

#### Scenario: 显式事务中拒绝附加库写入

- **GIVEN** BEGIN 后存在主库未提交修改
- **WHEN** 执行附加库 INSERT、UPDATE 或 DELETE
- **THEN** 点名拒绝，附加库不变，主库未提交修改经既有错误收口回滚

#### Scenario: 只读会话拒绝附加写入

- **GIVEN** 主库与附加库以只读会话打开
- **WHEN** 执行附加库 DML
- **THEN** 返回只读写入拒绝且数据不变

### Requirement: 写入资源生命周期与文档

附加库写入 SHALL 使用目标库自身的事务管理器、WAL、缓冲池、TableManager 与本地表名；失败回滚 SHALL 还原目标库数据及其 PK/UNIQUE/二级索引。附加库 SHALL 在成功、SQL 错误、输出错误以及信号取消后统一关闭；关闭失败在原执行成功时 SHALL 使 CLI 返回 exit 1 并点名附加库，已有 SQL/锁/密钥/信号错误 SHALL 保留其退出码并在 stderr 报告关闭失败。已经完成的提交 SHALL NOT 因关闭或自动 checkpoint 失败而被撤销。锁等待 SHALL 沿用现有预算，超时 exit 4；SIGINT/SIGTERM SHALL 保持 130/143。文档 SHALL 明确各文件独立提交与显式事务边界。

#### Scenario: 关闭重开保留附加库写入

- **GIVEN** 本次调用向附加库成功提交修改
- **WHEN** 调用结束并在新调用中打开附加文件
- **THEN** 已提交结果可读，原主库无误写

#### Scenario: 文档说明部分成功边界

- **WHEN** 用户阅读跨库 DML 使用说明
- **THEN** 能看到此前文件提交不会被后续文件语句失败撤销，以及显式事务的限制

#### Scenario: 后续 SQL 错误仍关闭此前写入库

- **GIVEN** 本次调用已经向两个附加库成功提交修改
- **WHEN** 后续 SQL 失败退出
- **THEN** 先前提交保持，两个附加库均完成关闭；重开可读且锁已释放

#### Scenario: 信号取消关闭附加库

- **GIVEN** 附加库已有本次调用成功提交的修改
- **WHEN** 执行阶段收到 SIGINT 或 SIGTERM
- **THEN** 停止语句工作后关闭已注册附加库，退出 130 或 143，重开保留已提交数据，未提交修改按单库取消与恢复保证处理

#### Scenario: 目标库恢复保持索引一致

- **GIVEN** 附加库经路由执行成功的 INSERT/UPSERT/UPDATE/DELETE，WAL 保留提交记录而未 checkpoint
- **WHEN** 丢弃打开实例后重新打开附加文件并恢复
- **THEN** 目标库行、PK 点查、唯一值占用和二级索引查询一致，主库数据不改变

### Requirement: UPDATE DELETE 保持已有语法边界

UPDATE/DELETE SHALL 保持现有单目标、主键等值条件以及 UPDATE 单列字面量赋值的支持范围。本基线没有 UPDATE/DELETE 子查询执行能力，跨库路由 SHALL NOT 将子查询条件或赋值变为可执行计划，也 SHALL NOT 静默忽略子查询后只执行部分条件。

#### Scenario: 子查询赋值与条件继续拒绝

- **GIVEN** 主库、附加库目标表与子查询源表存在
- **WHEN** 执行 `UPDATE o.t SET v = (SELECT v FROM t) WHERE id = 1` 或 `DELETE FROM o.t WHERE id IN (SELECT id FROM t)`
- **THEN** 按既有不支持值类型或主键等值条件要求拒绝，主库和附加库不变
