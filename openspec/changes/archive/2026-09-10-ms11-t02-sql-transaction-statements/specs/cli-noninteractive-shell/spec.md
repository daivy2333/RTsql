# cli-noninteractive-shell Delta

## MODIFIED Requirements

### Requirement: 多语句分片逐条执行

CLI SHALL 对以 `;` 分隔的多条 SQL 语句逐条执行：每条语句独立 plan/execute；无活跃会话事务时每条语句独立 auto-commit 事务（逐条生效）。事务语句（`BEGIN`/`COMMIT`/`ROLLBACK`，语义见 `sql-transaction-statements`）SHALL 改变会话事务上下文：`BEGIN` 之后的语句 SHALL 在打开的事务内执行（不再逐条 auto-commit），直至 `COMMIT`/`ROLLBACK` 终结该事务。每条语句的结果 SHALL 顺序渲染到 stdout：DML/DDL/事务语句输出受影响行数，SELECT 输出查询结果（列名表头等既有渲染语义不变）；`json` 格式下每条语句输出一个独立 JSON 文档。任一语句失败时 SHALL 立即停止执行（fail-fast）：以退出码 3 报错，错误信息 SHALL 包含失败语句的序号（第 k 条/共 n 条）与失败语句文本；失败之前语句的生效状态 SHALL 如实注明——无会话事务时注明已生效（已提交），有会话事务时注明未提交且已随事务回滚（此时 CLI SHALL 在返回前显式回滚会话事务）；失败之后的语句 SHALL NOT 执行。CLI 调用结束时 SHALL NOT 存在未提交的会话事务：语句循环正常结束仍有打开事务时 SHALL 显式回滚并向 stderr 提示，退出码仍为 0。SQL 语法错误 SHALL 在任何语句执行前整体拒绝（零执行），错误信息保留解析器的行/列定位。lib 单结果执行路径（`pipeline::execute` 网络路径、`execute_in_tx` 显式事务路径）遇多语句 SHALL 显式报错，SHALL NOT 静默截断。

#### Scenario: 多条 DML 逐条执行全部生效

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); INSERT INTO t VALUES (2)"`
- **THEN** 退出码 0，stdout 顺序输出两段受影响行数结果
- **AND** 重开查询可见两行（两条独立事务均已提交）

#### Scenario: 顺序渲染混合语句结果

- **GIVEN** 表 `t(id INT)` 已有数据
- **WHEN** `rtsql db "INSERT INTO t VALUES (9); SELECT id FROM t"`（`--format json`，非 TTY）
- **THEN** stdout 先输出 INSERT 的受影响行文档（与单语句 DML 输出形状一致），再输出 SELECT 的 rows 文档（两个独立 JSON 文档）
- **AND** 退出码 0

#### Scenario: 中间语句失败 fail-fast（部分已生效）

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2); INSERT INTO t VALUES (3)"`
- **THEN** 退出码 3，stderr 错误信息包含失败语句序号（第 2 条/共 3 条）与失败语句文本
- **AND** 第 1 条 INSERT 已生效（重开可查 `id=1`），第 3 条未执行

#### Scenario: 语法错误整体拒绝（零执行）

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); SELEC typo"`
- **THEN** 退出码 3，stderr 错误信息含解析器行/列定位
- **AND** 任何语句均未执行（`t` 仍为空）

#### Scenario: 分号边界语义

- **GIVEN** 表 `users(id INT PRIMARY KEY, name STRING)` 已有一行（`id=1`，`name='Alice'`）
- **WHEN** `rtsql db "SELECT id FROM users;; SELECT name FROM users WHERE name = 'a;b';"`（连续分号、字符串字面量内分号、尾随分号）
- **THEN** 按两条语句执行（字符串字面量不被分片），退出码 0
- **AND** 第一条查询输出 1 行，第二条查询输出空行集（`'a;b'` 不匹配任何行）

#### Scenario: lib 单结果路径显式拒绝多语句

- **GIVEN** 已打开的 `Database`
- **WHEN** `Database::execute_sql("SELECT 1; SELECT 2")` 或 `execute_in_tx` 传入多语句
- **THEN** 返回 `Response::Error`，错误信息说明该路径仅支持单语句
- **AND** 不执行任何语句（无静默截断）

#### Scenario: 事务上下文内失败注明未提交并回滚（区别于 auto-commit 上下文）

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2)"`
- **THEN** 退出码 3，stderr 错误信息包含失败语句序号（第 3 条/共 3 条）并注明先前语句未提交、已随事务回滚
- **AND** 重开查询 `t` 为空（无部分生效，与 auto-commit 上下文的"已生效"注明不同）

#### Scenario: 调用收尾未提交事务隐式回滚并提示

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1)"`（无 COMMIT/ROLLBACK）
- **THEN** 退出码 0，stderr 输出未提交回滚提示
- **AND** 重开查询 `t` 为空
