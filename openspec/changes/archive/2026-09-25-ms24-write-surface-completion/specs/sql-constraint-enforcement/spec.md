## MODIFIED Requirements

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
- **WHEN** DELETE 该行（提交）后插入 `code = 7` 的新行
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
