# sql-constraint-enforcement Specification

## Purpose

定义建表约束的执行与诚实化语义：NOT NULL 在写路径强制且零副作用；INT 列 UNIQUE 经内部非 PK 唯一索引强制并在干净重开与崩溃恢复两态下一致；CHECK、FOREIGN KEY、方言项与不支持的 UNIQUE 形态在计划期点名拒绝；未声明的既有语义逐字节零回归。消除「DDL 静默接受、运行期永不生效」的静默错误结果类缺陷。来源：MS23（change `2026-09-24-ms23-constraint-enforcement`，2026-09-24 规划、2026-09-25 收尾；依据 Explorer 缺口调查「约束一半从未被执行」，用户批准解决）。MS24（change `2026-09-25-ms24-write-surface-completion`，2026-09-26 收尾）修改 R3：明确回滚的 DELETE / REPLACE 由复现行重新占用其唯一值。

## Requirements

### Requirement: NOT NULL 写入强制

声明 NOT NULL 的列，系统 SHALL 在 INSERT 与 UPDATE 写入任何数据页、WAL、版本链或索引之前校验目标值为非 NULL，违反时 SHALL 以点名该列的错误拒绝且不产生任何副作用（表行数、索引与事务状态不变）。未声明 NOT NULL 的列 SHALL NOT 新增 NULL 拒绝；未声明 NOT NULL 的 PK 列在键位为 NULL 时的既有无键行语义 SHALL 保持逐字节不变。

#### Scenario: INSERT 违反 NOT NULL 零副作用拒绝

- **GIVEN** 表 `t` 含声明 NOT NULL 的列 `name` 与普通 INT 主键列 `id`
- **WHEN** 执行 `INSERT INTO t VALUES (1, NULL)`
- **THEN** 报错且错误文本包含列名 `name`；随后查询 `t` 行数不变，数据库文件与索引无新条目

#### Scenario: UPDATE SET 违反 NOT NULL 拒绝

- **GIVEN** 表 `t` 中已存在主键为 1 的行且 `name` 声明 NOT NULL
- **WHEN** 执行 `UPDATE t SET name = NULL WHERE id = 1`
- **THEN** 报错且错误文本包含列名 `name`；该行 `name` 保持原值

#### Scenario: 非 NULL 写入成功

- **GIVEN** 表 `t` 含声明 NOT NULL 的列 `name`
- **WHEN** 插入与更新 `name` 为非 NULL 合法值
- **THEN** 操作成功，行为与无约束列一致

#### Scenario: 声明 NOT NULL 的 PK 列拒绝 NULL 键位

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY NOT NULL, v INT)`
- **WHEN** 执行 `INSERT INTO t VALUES (NULL, 1)`
- **THEN** 报 NOT NULL 违反且点名 `id`，不落无键行

#### Scenario: 未声明 NOT NULL 的既有 NULL 语义零回归

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, v INT)`（未声明 NOT NULL）
- **WHEN** 插入键位为 NULL 或其他不可键控值的行，以及普通列 NULL 值
- **THEN** 行为与既有语义逐字节一致（无键行落库不入索引、普通列 NULL 存取正常）

### Requirement: 约束诚实化拒绝

CREATE TABLE 解析期 SHALL 对以下约束形态以点名不支持特性的错误拒绝，SHALL NOT 静默忽略：列级 `CHECK`、列级 `FOREIGN KEY`、列级方言特定选项（DialectSpecific，如 AUTO_INCREMENT）、表级 `CHECK`、表级 `FOREIGN KEY`。无语义期望的选项（`Null`、`Comment`）SHALL 维持既有忽略。拒绝发生在计划期，SHALL NOT 创建表。

#### Scenario: 列级 CHECK 点名拒绝

- **GIVEN** 空数据库
- **WHEN** 执行 `CREATE TABLE t (a INT CHECK (a > 0))`
- **THEN** 报错且错误文本点名 CHECK 不支持；表 `t` 不存在

#### Scenario: 列级 FOREIGN KEY 点名拒绝

- **GIVEN** 空数据库
- **WHEN** 执行 `CREATE TABLE t (a INT, b INT REFERENCES other(id))`
- **THEN** 报错且错误文本点名 FOREIGN KEY 不支持；表 `t` 不存在

#### Scenario: 方言特定选项点名拒绝

- **GIVEN** 空数据库
- **WHEN** 执行含方言特定列选项（如 AUTO_INCREMENT）的建表语句
- **THEN** 报错且错误文本点名该选项不支持；表不存在

#### Scenario: 表级 CHECK 与 FOREIGN KEY 点名拒绝

- **GIVEN** 空数据库
- **WHEN** 执行含表级 `CHECK (...)` 或表级 `FOREIGN KEY ... REFERENCES ...` 的建表语句
- **THEN** 报错且错误文本点名对应约束类型；表不存在

#### Scenario: 无约束与受支持形态建表不受影响

- **GIVEN** 空数据库
- **WHEN** 执行仅含列定义、NOT NULL、DEFAULT、PRIMARY KEY 的建表语句
- **THEN** 建表成功，行为与本 change 之前一致

### Requirement: INT 列 UNIQUE 强制

声明 UNIQUE 的 INT 列，系统 SHALL 经该列专属的内部非 PK 唯一索引在写路径强制唯一性：INSERT 重复值 SHALL 以与主键重复同型的 `DuplicateKey` 错误拒绝且预检零副作用；UPDATE 维护唯一索引条目（改值碰撞预检拒绝、同值随行版本更新、置 NULL 删除条目）；DELETE 提交后同值可重插；事务回滚后唯一索引条目 SHALL 反映回滚前的占用关系——回滚的 INSERT / UPDATE 不留占用（后续同值可重插），回滚的 DELETE / REPLACE SHALL 由复现出的行重新占用其唯一值（后续同值插入以 `DuplicateKey` 拒绝）。唯一列 NULL 值 SHALL NOT 参与唯一性（允许多行 NULL）。非 PK 主键索引的既有语义 SHALL 保持不变。

**唯一列写入值类型一致**（F1 修复轮补强，2026-09-25 Review 验收）：计划期类型门只约束列声明类型；INT 唯一列的运行期写入值非 NULL 且非 Int（如 String/Float/Bool 字面量）SHALL 在任何存储副作用前以 `KeyTypeMismatch`（期望 INT、点名实际类型，`key-column-type-conformance` 同型变体）拒绝——INSERT 唯一预检与 UPDATE 碰撞预检同位执行。守卫仅对存在唯一索引的列生效，旧格式库非 INT unique 标志列（无索引）与无唯一列表路径不变。

#### Scenario: INSERT 重复值同型拒绝

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE)` 且已存在 `code = 7` 的行
- **WHEN** 执行 `INSERT INTO t VALUES (2, 7)`
- **THEN** 报 `DuplicateKey` 同型错误；随后查询 `t` 行数不变（预检零副作用）

#### Scenario: INSERT 不同值成功

- **GIVEN** 同上表且已存在 `code = 7`
- **WHEN** 执行 `INSERT INTO t VALUES (2, 8)`
- **THEN** 插入成功且可按 `code = 8` 查得该行

#### Scenario: UPDATE 改唯一列为已有值拒绝

- **GIVEN** 表中存在 `code = 7` 与 `code = 8` 两行
- **WHEN** 执行 `UPDATE t SET code = 7 WHERE id = <code=8 行的主键>`
- **THEN** 报 `DuplicateKey` 同型错误，`code = 8` 行保持原值

#### Scenario: UPDATE 非唯一列后唯一条目随行

- **GIVEN** 表中存在 `code = 7` 的行
- **WHEN** UPDATE 该行的非唯一列（产生新版本 row id）
- **THEN** 唯一索引条目指向该行最新版本，按 `code = 7` 的唯一性检查继续正确工作

#### Scenario: 多行 NULL 不冲突

- **GIVEN** 建表含 `code INT UNIQUE`
- **WHEN** 插入多行 `code` 为 NULL 的记录
- **THEN** 全部成功，NULL 不触发 DuplicateKey

#### Scenario: DELETE 后同值可重插

- **GIVEN** 已存在 `code = 7` 的行
- **WHEN** DELETE 该行后插入 `code = 7` 的新行
- **THEN** 插入成功

#### Scenario: 事务回滚后同值可重插

- **GIVEN** CLI 会话事务中插入 `code = 7` 后事务回滚（含显式 ROLLBACK 与语句失败自动回滚）
- **WHEN** 后续（自动提交）插入 `code = 7`
- **THEN** 插入成功，唯一索引无残留条目

#### Scenario: DELETE / REPLACE 回滚后同值被复现行占用（2026-09-26 扩围）

- **GIVEN** 表 `t(id INT PRIMARY KEY, code INT UNIQUE)` 存在行 `(1, 7)`
- **WHEN** 显式事务内删除该行（DELETE，或同键 REPLACE 新行）后 ROLLBACK
- **THEN** 复现行仍占用 `code = 7`：后续自动提交插入 `code = 7` 以 `DuplicateKey` 拒绝，表内不出现两条存活行共享该唯一值

#### Scenario: 同表多个 UNIQUE 列各自强制

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, a INT UNIQUE, b INT UNIQUE)`
- **WHEN** 分别插入 `a` 重复与 `b` 重复的行
- **THEN** 均以 DuplicateKey 同型拒绝，两列唯一性互不干扰

#### Scenario: 唯一列非 Int 写入值类型化拒绝（F1 补强）

- **GIVEN** 建表 `CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE)`
- **WHEN** 执行 `INSERT INTO t VALUES (1, 'abc')` 或 `UPDATE t SET code = 1.5 WHERE id = 1`
- **THEN** 报 `KeyTypeMismatch`（期望 INT、点名实际类型），任何写入前拒绝；UPDATE 原行保持，INSERT 后表行数为 0

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

- **GIVEN** 旧版本创建的库含非 INT 列 UNIQUE 标志
- **WHEN** dump 出的 DDL 在本 change 之后 restore
- **THEN** restore 被拒绝面挡住（接受并文档化的兼容性破坏；拒绝文本点名 UNIQUE 类型限制）

### Requirement: UNIQUE 干净重开与崩溃恢复两态一致

INT 列唯一索引 SHALL 在两种打开状态下一致强制：干净关闭后重开 SHALL 从 catalog 持久化的索引根恢复唯一索引；崩溃恢复（WAL 重放后索引去信任重建）SHALL 从最终数据页重建唯一索引，重建发现同一唯一列存在跨链重复存活值时 SHALL 显式报错而非静默跳过。catalog 表行格式演进 SHALL 向后兼容（旧格式文件可打开，行为为无唯一索引）。`drop_table` SHALL 释放唯一索引页。

#### Scenario: 干净重开后强制保持

- **GIVEN** 含 `code INT UNIQUE` 的表已插入数据后正常关闭
- **WHEN** 重新打开并插入 `code` 重复行
- **THEN** DuplicateKey 拒绝（唯一索引自持久化根恢复）

#### Scenario: 崩溃恢复后强制保持

- **GIVEN** 含 `code INT UNIQUE` 的表有已提交写入后进程崩溃（WAL 含未 checkpoint 记录）
- **WHEN** 重开触发恢复重放与索引重建，之后插入 `code` 重复行
- **THEN** 恢复成功且 DuplicateKey 拒绝（唯一索引自数据页重建）

#### Scenario: 恢复重建发现跨链重复显式报错

- **GIVEN** 数据页最终状态中同一唯一列存在两个不同链的相同存活值（仅可经外部改动产生）
- **WHEN** 崩溃恢复执行唯一索引重建
- **THEN** 恢复以点名表与列的显式错误失败，不静默建立不完整索引

#### Scenario: 旧格式 catalog 行兼容打开

- **GIVEN** 本 change 之前创建的数据库文件（表行无唯一索引字段）
- **WHEN** 新版本打开该库
- **THEN** 打开成功，表无唯一索引且既有 DML 行为不变

#### Scenario: drop_table 释放唯一索引页

- **GIVEN** 含 INT UNIQUE 列的表
- **WHEN** DROP TABLE 后同进程新建表写入数据
- **THEN** 成功且文件页数不因旧唯一索引页泄漏而异常增长

### Requirement: 既有语义零回归

本 change SHALL NOT 改变无约束表的任何 DML/DDL 行为、主键索引既有语义（DuplicateKey/KeyTypeMismatch/无键行落库不入索引）、MVCC 可见性、事务、WAL/checkpoint/恢复的既有路径与全量既有测试结论；本 change 预期行为更新涉及的既有用例 SHALL 仅按新契约校准。

#### Scenario: 无约束表行为不变

- **GIVEN** 不含任何约束声明的表
- **WHEN** 执行既有 CRUD、导入导出与分析命令
- **THEN** 行为与输出与本 change 之前一致

#### Scenario: 主键既有语义不变

- **GIVEN** 既有主键语义用例集（重复键、键列类型错配、无键行）
- **WHEN** 本 change 落地后运行
- **THEN** 结论不变（除按新契约显式校准的用例外零修改通过）

#### Scenario: 全量测试零回归

- **WHEN** 运行全量测试套件
- **THEN** 全部通过（0 failures），失败仅为可归因于本 change 契约校准的用例
