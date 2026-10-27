# sql-constraint-enforcement Delta

## MODIFIED Requirements

### Requirement: UNIQUE 形态与类型面诚实化

系统 SHALL 仅对声明类型为 INT 的列执行 UNIQUE 强制；非 INT 列（String/Float/Bool/Date/Timestamp）声明 UNIQUE SHALL 在建表计划期以点名错误拒绝。表级单列 `UNIQUE(col)` SHALL 等价映射为该列 UNIQUE 标志；表级多列组合 UNIQUE SHALL 点名拒绝。用户级 CREATE/DROP INDEX 语句 SHALL 维持既有不可用状态。

#### Scenario: 非 INT 列 UNIQUE 点名拒绝

- **GIVEN** 空数据库
- **WHEN** 分别执行 `name STRING UNIQUE`、`f FLOAT UNIQUE`、`b BOOL UNIQUE`、`d DATE UNIQUE`、`ts TIMESTAMP UNIQUE` 的建表语句
- **THEN** 均报错且错误文本点名 UNIQUE 仅支持 INT 列；表不存在

#### Scenario: 表级单列 UNIQUE 等价列级

- **GIVEN** 空数据库
- **WHEN** 执行 `CREATE TABLE t (id INT PRIMARY KEY, code INT, UNIQUE(code))`
- **THEN** 建表成功且 `code` 唯一性按「INT 列 UNIQUE 强制」语义生效；`schema` 输出渲染该约束

#### Scenario: 表级多列组合 UNIQUE 点名拒绝

- **GIVEN** 空数据库
- **WHEN** 执行 `CREATE TABLE t (a INT, b INT, UNIQUE(a, b))`
- **THEN** 报错且错误文本点名组合 UNIQUE 不支持；表不存在

#### Scenario: dump/restore 往返保持唯一强制

- **GIVEN** 含 INT UNIQUE 列的表已有数据
- **WHEN** `dump` 导出后对新库 `restore`
- **THEN** restore 成功且唯一强制在新库生效（INT UNIQUE 往返恒等）

#### Scenario: 存量非 INT UNIQUE 库的边界已知

- **GIVEN** 旧版本创建的库含非 INT 列 UNIQUE 标志（该约束从未被强制）
- **WHEN** dump 出的 DDL 在本 change 之后 restore
- **THEN** dump 的 DDL SHALL NOT 渲染该 `UNIQUE`（与 `PRIMARY KEY` 同型条件：仅整数类型的唯一列输出 `UNIQUE`），restore 因此成功
- **AND** 恢复后的表 SHALL **不含**该唯一约束——产物文本本身即声明了这一点（SHALL NOT 是被静默的部分恢复）
- **AND** **旧版本已导出的、含该 `UNIQUE` 声明的 dump 文本**仍被拒绝面挡住（拒绝文本点名 UNIQUE 类型限制）——该输入面的拒绝保持不变
