# cli-noninteractive-shell Delta

## MODIFIED Requirements

### Requirement: 生命周期子命令 dump 与 restore

`rtsql dump <db>` SHALL 向 stdout 输出逻辑 SQL 导出：按 catalog 顺序对每个用户表输出一行 CREATE TABLE DDL，随后按表 schema 列序输出全部行的 INSERT 语句；字符串字面量 SHALL 转义（单引号加倍），NULL 与布尔值 SHALL 输出为 SQL 字面量（NULL / TRUE / FALSE）。DDL 渲染 SHALL 仅在主键列声明类型为 INTEGER 时输出 `PRIMARY KEY`：显式声明 INTEGER PK 的表 SHALL 照常输出；非 INTEGER PK 声明的存量表（声明从未被强制）与隐式主键非 INTEGER 首列表 SHALL NOT 输出 `PRIMARY KEY`（输出与实际强制语义一致，产物在新拒绝面下可 restore 往返）。**DDL 渲染 SHALL 仅在唯一列声明类型为 INTEGER 时输出 `UNIQUE`**（与 `PRIMARY KEY` 同型条件）：显式声明 INTEGER UNIQUE 的表 SHALL 照常输出；非 INTEGER UNIQUE 声明的存量表（该约束从未被强制）SHALL NOT 输出 `UNIQUE`（该类表因此产出**不含该约束**的 DDL，产物在新拒绝面下可 restore 往返）。空库 SHALL 无输出并以退出码 0 退出。

`rtsql restore <db> <file|->` SHALL 从文件（`-` 表示 stdin）读取 SQL 文本，对目标库逐条执行（每条独立 auto-commit，逐条生效），成功时静默退出（退出码 0）。目标库文件不存在、或已含用户表时 SHALL 以退出码 1 拒绝。执行期任一语句失败 SHALL 立即停止：以退出码 3 报错，错误信息含失败语句序号（第 k 条/共 n 条），失败之前的语句已生效。**多表 dump 的部分恢复 SHALL 是显式可见的**：该失败 SHALL NOT 被报告为成功，SHALL 显式告知此前已执行的语句已生效。两条命令打开数据库的锁冲突均 SHALL 以退出码 4 报错；库文件不存在均 SHALL 以退出码 1 报错。进程退出前 SHALL 完成 close()。

**e2e 验证与缺陷清偿 change（T6/ISS12，2026-10-14 收尾）追加**：`schema` 与 `dump` 的 DDL 列类型 SHALL 渲染标准长名——`INTEGER`、`FLOAT`、`BOOLEAN`、`DATE`、`TIMESTAMP`、`TEXT`（不再输出引擎内部短名 `INT`/`STRING`/`BOOL`）。dump→restore 往返与多代 dump/restore 幂等 SHALL 保持（解析器接受长名）。

#### Scenario: dump-restore 往返等价

- **GIVEN** 库 `a` 含表 `users (id INT PRIMARY KEY, name STRING)` 及若干行数据
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** 全链退出码 0，库 `b` 具有与 `a` 相同的表结构与行数据（SELECT 比对验证）

#### Scenario: INTEGER PK 表 dump 输出逐字节不变

- **GIVEN** 含显式 INTEGER PRIMARY KEY 表的库
- **WHEN** `rtsql dump a`
- **THEN** DDL 含 `PRIMARY KEY`（渲染行为与修改前逐字节一致）

#### Scenario: 非 INTEGER PK 存量表 dump 渲染与往返

- **GIVEN** lib 通道构造的存量 TEXT PRIMARY KEY 表（或隐式主键非 INTEGER 首列表）
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** DDL 不含 `PRIMARY KEY`；restore 退出码 0，库 `b` 行数据等价（往返可达，不被非 INTEGER PK 计划期拒绝面阻断）

#### Scenario: INTEGER 唯一列的 dump 输出与往返

- **GIVEN** 含 INTEGER 类型 `UNIQUE` 列的表
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** DDL 含 `UNIQUE`；restore 退出码 0，唯一强制在新库生效（INT UNIQUE 往返恒等）

#### Scenario: 非 INTEGER UNIQUE 存量表 dump 渲染与往返

- **GIVEN** lib 通道构造的存量非 INTEGER 列带 `UNIQUE` 标志的表（该约束从未被强制）
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** DDL 不含 `UNIQUE`；restore 退出码 0，库 `b` 行数据等价（往返可达，不被非 INTEGER UNIQUE 计划期拒绝面阻断）
- **AND** 库 `b` 的该表**不含**该唯一约束——产物文本本身即声明了这一点（SHALL NOT 是被静默的部分恢复）

#### Scenario: dump 空库无输出

- **GIVEN** 库 `a` 存在且无用户表
- **WHEN** `rtsql dump a`
- **THEN** stdout 无输出，退出码 0

#### Scenario: restore 经 stdin 管道

- **GIVEN** 库 `a` 含表与数据，库 `b` 为 `new` 创建的空库
- **WHEN** `rtsql dump a | rtsql restore b -`
- **THEN** 退出码 0，库 `b` 与 `a` 数据等价

#### Scenario: restore 目标非空库拒绝

- **GIVEN** 库 `b` 已含用户表
- **WHEN** `rtsql restore b dump.sql`
- **THEN** stderr 报错，退出码 1，库内容不变

#### Scenario: restore 中途失败 fail-fast

- **GIVEN** dump 产物中第二条语句执行必然失败（如手工构造的重复主键）
- **WHEN** `rtsql restore b dump.sql`
- **THEN** 退出码 3，stderr 含失败语句序号；第一条语句的成效保留（重开可见），其后语句未执行

#### Scenario: 多表 dump 的部分恢复显式可见

- **GIVEN** 一个包含**多张表**的 dump 产物，其中**靠后**的一条语句会被拒绝面挡住（例如该语句声明了非 INTEGER 列的 `UNIQUE`）
- **WHEN** 对空库执行 `rtsql restore b dump.sql`
- **THEN** 退出码 3，stderr 含失败语句序号，并**显式告知此前已执行的语句已生效**
- **AND** 已执行的前序语句在库中生效（重开可见），SHALL NOT 被回滚
- **AND** restore SHALL NOT 报告成功

#### Scenario: dump/restore 库文件不存在报错

- **GIVEN** 目标库文件不存在
- **WHEN** `rtsql dump missing` 或 `rtsql restore missing dump.sql`
- **THEN** stderr 报错且信息含 `does not exist`，退出码 1，不创建任何文件

#### Scenario: dump/restore 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql dump <db>` 或 `rtsql restore <db> dump.sql`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息，退出码 4

#### Scenario: 类型名渲染与声明一致

- **GIVEN** `CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, ok BOOLEAN, score FLOAT)`
- **WHEN** `rtsql schema t` / `rtsql dump t`
- **THEN** DDL 输出 `INTEGER`/`TEXT`/`BOOLEAN`/`FLOAT`（失败边界——修复前：渲染 `INT`/`STRING`/`BOOL` 短名）

#### Scenario: dump→restore 往返与多代幂等

- **GIVEN** 同上库 dump
- **WHEN** restore 到新库后再 dump
- **THEN** 类型名与数据逐字节幂等
