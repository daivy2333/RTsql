# sql-secondary-indexes Specification

## Purpose

约束用户级二级索引（`CREATE INDEX` / `DROP INDEX`）的端到端语义：单列、非唯一；声明 INT 或内部归一为字符串的列可建索引，支持域外类型与全形态修饰点名拒绝；完整编码可容纳的非 NULL 存量回填并持久化（干净重开、未决 WAL，以及崩溃恢复 `redo_count > 0` 索引去信任重建后与存活数据一致；超出键容量的字符串整值写入，等值查询扫描完整值，不截断、不限制字段长度）；索引名库级唯一；被索引列等值查询路由 `IndexScanAll` 可达且结果与全表扫描严格相等（不可路由谓词回落扫描）；INSERT/UPDATE（含 rekey）/DELETE/UPSERT 写路径与事务回滚、提交前取消还原条目；字符串索引不扩大非整数主键或唯一约束；DROP INDEX 页物理释放与查询回落扫描。来源：MS21（change `2026-09-29-ms21-ddl-evolution`，improvements I068；唯一索引基建复用 MS23）。

## Requirements

### Requirement: CREATE INDEX 建立单列 INT 二级索引

`CREATE INDEX <name> ON <table>(<col>)` SHALL 在声明 INT 或内部归一为 ColumnType::String 的单列上建立非唯一二级索引，SHALL 对完整编码可由该布局容纳的非 NULL 存量值全量回填，并持久化名称、所属表、列和根页。INT 既有行为 SHALL 保持；字符串比较 SHALL 沿用 SQL 现有相等语义，允许重复值与空字符串，NULL 行 SHALL NOT 入索引。干净重开和未决 WAL 恢复后索引 SHALL 与存活数据一致。合法字符串超过该布局键容量时 SHALL 继续允许写入并由完整值扫描查询，SHALL NOT 截断、漏发或给行字段增加长度限制。

索引名 SHALL 库级唯一；重名、缺表、缺列与支持域外类型 SHALL 点名拒绝且零副作用。`CREATE UNIQUE INDEX`、组合索引、表达式索引、部分索引与 `IF NOT EXISTS` SHALL 继续在计划期点名拒绝。显式事务中的 CREATE/DROP INDEX SHALL 继续在副作用之前以 0A000 拒绝。字符串索引 SHALL NOT 扩大非整数主键或唯一约束支持面。

#### Scenario: 建索引与存量行索引化

- **GIVEN** 表 `t(id INT PRIMARY KEY, age INT, name VARCHAR)` 含若干行
- **WHEN** `CREATE INDEX idx_age ON t(age)`
- **THEN** 建索引成功；`SELECT * FROM t WHERE age = 25` 返回与建索引前完全一致的行集（含重复 age 值的多行全部命中）

#### Scenario: 干净重开后索引可用

- **GIVEN** 已执行 `CREATE INDEX idx_age ON t(age)` 并显式 close
- **WHEN** 重新打开数据库
- **THEN** 索引仍可用，等值查询行集一致

#### Scenario: 崩溃恢复两态一致

- **GIVEN** 建索引后含新写入行，进程未 close 退出（崩溃模拟：WAL 重放 + 索引重建）
- **WHEN** 重新打开数据库
- **THEN** 等值查询行集与崩溃前运行期行为一致

#### Scenario: 非 INT 列拒绝

- **GIVEN** 表 `t(id INT PRIMARY KEY, flag BOOLEAN)`，该列既非 INT 也非本批支持的字符串类型
- **WHEN** `CREATE INDEX idx_flag ON t(flag)`
- **THEN** 计划期点名拒绝不支持的声明类型，零副作用

#### Scenario: 重复索引名拒绝

- **GIVEN** 已存在索引 `idx_age`（任意表）
- **WHEN** `CREATE INDEX idx_age ON other(c)`
- **THEN** 点名拒绝（索引名库级唯一），既有索引零变化

#### Scenario: UNIQUE 与组合索引拒绝

- **WHEN** `CREATE UNIQUE INDEX u ON t(age)` / `CREATE INDEX c ON t(age, id)`
- **THEN** 均计划期点名拒绝（唯一性约束走建表声明；首版单列）

#### Scenario: 表或列不存在拒绝

- **WHEN** `CREATE INDEX i1 ON nope(c)` / `CREATE INDEX i2 ON t(nope)`
- **THEN** 均点名拒绝（表不存在 / 列不存在），零副作用

#### Scenario: 整数索引建表回填行为保持

- **GIVEN** 含重复整数值的表
- **WHEN** 对该 INT 列创建非唯一二级索引
- **THEN** 建索引成功，等值查询与创建前的行集一致

#### Scenario: 字符串存量回填及空值边界

- **GIVEN** 字符串列含重复值、空字符串、多字节 UTF-8 与 NULL
- **WHEN** 对该列创建非唯一二级索引并执行等值查询
- **THEN** 支持域内创建成功，非 NULL 值的全部匹配行可见，NULL 保持既有三值语义

#### Scenario: 干净重开与崩溃恢复内容一致

- **GIVEN** 已建字符串索引，随后完成合法增删改
- **WHEN** 分别在干净关闭后、存在未决 WAL 的非正常关闭后重新打开
- **THEN** 两条路径均恢复正确的已提交行与索引，完整性检查无 error

#### Scenario: 不支持的声明与事务边界拒绝

- **GIVEN** 重名、缺表、缺列、支持域外类型、修饰或显式事务中的建索引请求
- **WHEN** 执行该请求
- **THEN** 沿用既有分类点名拒绝，表和原有索引无副作用

### Requirement: 二级索引等值查询路由可达

被索引列与匹配类型字面量的等值谓词，在其完整编码可容纳且符合既有裸列字面量形态时 SHALL 可路由到二级索引，并由 EXPLAIN 显示索引节点且不执行内层查询。支持域内字符串匹配 SHALL 比较完整值，不得把共享前缀但不同的字符串当作相同值。索引路径与全表扫描的结果多重集合 SHALL 严格相等，不遗漏、不多发；单列、乱序多列和通配输出 SHALL 保持列元数据与投影契约。

NULL、类型不匹配、无索引、超出布局键容量和本批不支持的谓词形态 SHALL 保持既有扫描与错误语义。范围路由不属于本次扩展。

#### Scenario: 等值路由计划形状可观察

- **GIVEN** 已建 `CREATE INDEX idx_age ON t(age)`
- **WHEN** `EXPLAIN SELECT * FROM t WHERE age = 25`
- **THEN** 计划树含该索引的索引扫描节点（点名索引），且不执行内层

#### Scenario: 建索引前后结果等价

- **GIVEN** 表 `t` 含重复 age 值多行
- **WHEN** 同一 `SELECT * FROM t WHERE age = 25` 在建索引前与建索引后执行
- **THEN** 两次行集一致（仅计划形状可能不同）

#### Scenario: 不可键控字面量回落行内求值

- **GIVEN** 已建 `idx_age`
- **WHEN** `SELECT * FROM t WHERE age = NULL`（或与索引列类型不匹配的字面量形态）
- **THEN** 回落既有扫描语义，无路由相关错误（与 PK 类型感知路由同型）

#### Scenario: 无索引列零回归

- **WHEN** `SELECT * FROM t WHERE name = 'x'`（未建索引列）
- **THEN** 行为与既有语义一致（全扫 + 谓词下推）

**e2e 验证与缺陷清偿 change（T2/ISS05，2026-10-14 收尾）修改**：等值路由结果集 SHALL 与全表扫描（DataScan）严格相等——双向：不重复、不遗漏——任意重复键规模成立（多叶等值连串、≥2 层树）。同值键跨越「小于查找键的分隔键」边界散布的树形态下，全部命中条目 SHALL 可达（范围下探：首 ≥K 分隔键的前一子树至末 ≤K 分隔键的子树）。失败语句（含 UPDATE 旧键删除路径）SHALL 对索引零副作用（报错后索引条目集不变）。

#### Scenario: 多叶等值连串双向严格相等

- **GIVEN** 表 t1（id INTEGER PRIMARY KEY, n INTEGER）30000 行 n∈1..100 重复键分布，`CREATE INDEX i1 ON t1(n)` 前 DataScan 基线逐 k 记录
- **WHEN** 建索引后逐 k `SELECT COUNT(*), COUNT(DISTINCT id) FROM t1 WHERE n = k`
- **THEN** COUNT(*) 与 COUNT(DISTINCT id) 均等于基线（失败边界——修复前：COUNT 虚高 [实测 +22944 总量] 且 DISTINCT 漏行 [实测 476 条漏发]）

#### Scenario: 边界子树条目的 UPDATE 探针可达

- **GIVEN** 同上建索引后，取基线中存在但索引扫描曾漏发的行 id
- **WHEN** `UPDATE t1 SET n = 101 WHERE id = <该行>`
- **THEN** 成功执行且旧键条目删除（失败边界——修复前：`Execution error: Key not found`，且失败后索引计数被 abort 路径改变——零副作用被违反）

#### Scenario: 失败语句索引零副作用

- **GIVEN** 同上建索引后的表，记录某键 k 的 COUNT/DISTINCT 基线
- **WHEN** 构造一条执行失败的 UPDATE（如与唯一约束/键碰撞冲突）后重新计数
- **THEN** `WHERE n = k` 的 COUNT 与 DISTINCT 与失败前一致（失败边界——修复前：abort 重插改变索引内容）

#### Scenario: 既有回落与唯一索引零回归

- **GIVEN** 同上
- **WHEN** 范围谓词 `WHERE n > k`、复合谓词 `WHERE n = k AND id <= x`、唯一键索引点查
- **THEN** 全部行为与修复前一致

#### Scenario: 字符串等值查询可观察且与扫描相等

- **GIVEN** 含重复字符串值的表与该列的二级索引
- **WHEN** 对同一等值条件执行 EXPLAIN 和不同投影的 SELECT
- **THEN** EXPLAIN 显示对应索引且不执行查询，SELECT 表头与行逐位一致，结果与创建索引前的扫描严格相等

#### Scenario: 共享前缀和边界长度不误匹配

- **GIVEN** 支持域内两字符串共享长前缀，另有空串、多字节及尾部零字节样本
- **WHEN** 分别按完整值执行等值查询
- **THEN** 仅返回真实相等值的全部行，不截断合并不同值，不漏行

#### Scenario: 非可路由谓词保留既有行为

- **GIVEN** 字符串二级索引存在
- **WHEN** 查询 NULL、不同类型字面量、范围或未被本批路由的组合谓词
- **THEN** 沿用既有扫描求值与错误语义，结果不因索引存在而改变

#### Scenario: 超容量完整字符串扫描且允许跨边界改值

- **GIVEN** 表含两个共享长前缀但不同的字符串，其完整编码超出所选布局键容量，另有容量内字符串
- **WHEN** 创建索引、分别按完整长值查询，再把行值在容量内和容量外双向更新
- **THEN** 创建和合法写入成功，长值查询计划为扫描且仅返回真实匹配行；容量内值可走索引，跨界更新的新旧查询与全表扫描一致
- **AND** NULL 和长值分别按其语义跳过条目，不截断键，不限制表字段长度

### Requirement: 二级索引写路径条目维护

INSERT、UPDATE/rekey、DELETE、UPSERT 与 REPLACE SHALL 维护全部受影响的 INT 和字符串二级索引，使条目与存活行一致。字符串与 NULL 相互转换 SHALL 分别增删条目；重复键中一行变化 SHALL NOT 误删其他行。回滚、失败和提交前取消经既有回收后 SHALL 还原行与条目，不留半写结构或占用键；提交后结果未送达 SHALL 继续视为已提交。

#### Scenario: INSERT 后点查可达

- **GIVEN** 已建 `idx_age`
- **WHEN** `INSERT INTO t VALUES (10, 25, 'x')`
- **THEN** `SELECT * FROM t WHERE age = 25` 含新行

#### Scenario: DELETE 后点查不再命中

- **GIVEN** 已建 `idx_age`，行 (10, 25, 'x')
- **WHEN** `DELETE FROM t WHERE id = 10` 后 `SELECT * FROM t WHERE age = 25`
- **THEN** 不再含该行

#### Scenario: 索引列 rekey 后新旧键可达性一致

- **GIVEN** 已建 `idx_age`，行 (10, 25, 'x')
- **WHEN** `UPDATE t SET age = 30 WHERE id = 10`
- **THEN** `WHERE age = 30` 可达该行、`WHERE age = 25` 不再含该行（经索引路由与非索引路径结果一致）

#### Scenario: 事务回滚后条目还原

- **GIVEN** 已建 `idx_age`，CLI 会话 `BEGIN`
- **WHEN** `INSERT ...; UPDATE ... SET age ...; DELETE ...` 后 `ROLLBACK`
- **THEN** `WHERE age = ...` 等值查询行集与事务前一致（含重开两态）

#### Scenario: UPSERT 冲突行删除后条目清理

- **GIVEN** 已建 `idx_age`，行 (1, 25, 'a')
- **WHEN** `INSERT INTO t VALUES (1, 99, 'b') ON CONFLICT (id) DO UPDATE SET age = 99`（或 REPLACE 形态删旧行）
- **THEN** `WHERE age = 25` 不再含旧行、`WHERE age = 99` 含更新行

**e2e 验证与缺陷清偿 change（T2/ISS05，2026-10-14 收尾）追加**：树遍历原语 `IndexManager::scan_all` SHALL 对任意层树返回全部条目（内部页感知、不依赖叶子链指针、不以 page 0 为终止条件）；叶子链指针（`next_leaf_page_id`）SHALL 在任何叶内条目重排（非末位插入触发的整页重建）后保持不变。同值键写路径（INSERT/UPDATE/DELETE 的 `insert`/`delete_exact`）SHALL 对跨界散布形态的全部条目正确维护（删除可达、重插定位正确）。

#### Scenario: 多层树全量枚举完整

- **GIVEN** 30000 行表 + 二级索引（树 ≥2 层）
- **WHEN** 经内部全量枚举通道（无 WHERE 全表 UPDATE/DELETE、Scan 执行器）枚举
- **THEN** 枚举条目数 = 表行数（失败边界——修复前：`scan_all` 对内部根树返回空/碎片 [实测 190/30000]，全表 DML affected_rows 失真或整语句失败）

#### Scenario: 叶子链在乱序插入后保真

- **GIVEN** 以非升序键值持续插入形成的索引树（多次触发非末位插入与分裂）
- **WHEN** 逐叶检查 `next_leaf_page_id`
- **THEN** 除最右叶外全部非零且构成完整链（失败边界——修复前：~90% 叶子链指针被 `shift_slots_right` 重建清零）

#### Scenario: 全表 DML 多层树行数正确

- **GIVEN** ≥3 层 PK 树的大表（乱序主键分布）
- **WHEN** `UPDATE t SET …`（无 WHERE）/ `DELETE FROM t`（无 WHERE）
- **THEN** `affected_rows` = 实际行数且全部行被触达（失败边界——修复前：affected≈0 或 `Abort failed: Key not found` 整语句失败）

#### Scenario: 字符串改值和 NULL 转换维护新旧键

- **GIVEN** 已有字符串索引，且多行共享相同字符串
- **WHEN** 插入、改变其中一行字符串、在字符串与 NULL 间转换，或删除一行
- **THEN** 新旧键查询与存活行一致，其他重复值行保持可见

#### Scenario: 多步回滚和冲突更新保持条目一致

- **GIVEN** 已建字符串索引并保留既有主键和唯一约束
- **WHEN** 分别执行事务内多步写入后回滚、UPSERT/REPLACE，以及违反既有约束的失败写入
- **THEN** 成功路径条目与新行一致，回滚与失败路径保留原已提交行及条目

#### Scenario: 取消和超时沿用既有索引原子性

- **GIVEN** 修改字符串索引的语句处于写入阶段
- **WHEN** future 被丢弃或超时到期，随后触发既有回收
- **THEN** 提交前取消的语句不提交，相关键全部释放且结构自洽；已提交但未送达的结果不被撤销

### Requirement: DROP INDEX 移除二级索引

`DROP INDEX <name>` SHALL 删除该索引：索引页 SHALL 物理释放（free-list，drop_table 同型通道），此后对应等值查询 SHALL 回落既有扫描且行集与删除前一致；表数据与其余索引 SHALL 不受影响。索引不存在 SHALL 点名拒绝；`IF EXISTS` 子句 SHALL 点名拒绝（假设 10）。删除 SHALL 跨重启持久（catalog 登记移除）。

#### Scenario: 删索引后查询回落扫描

- **GIVEN** 已建 `idx_age` 并删除（`DROP INDEX idx_age`）
- **WHEN** `SELECT * FROM t WHERE age = 25` 与 `EXPLAIN` 同查询
- **THEN** 行集与删索引前一致；计划形状不再含该索引节点

#### Scenario: 索引不存在拒绝

- **WHEN** `DROP INDEX nope`
- **THEN** 点名拒绝

#### Scenario: 删索引跨重启持久

- **GIVEN** `DROP INDEX idx_age` 后显式 close
- **WHEN** 重新打开数据库
- **THEN** 索引不存在，等值查询行为与扫描语义一致，无残留错误

### Requirement: 既有语义零回归

PK 索引与唯一索引（MS23）、键位等值路由（MS15/MS16）、约束执行面、T1 索引条目一致性收口语义 SHALL 全部保持；本 change 前全部既有测试 SHALL 零修改通过（语义校准白名单除外）。

#### Scenario: 既有索引与路由面零回归

- **WHEN** 运行完整测试套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 通过

### Requirement: IndexScanAll 列元数据与输出形状契约

二级索引等值路由节点（IndexScanAll）的列元数据 SHALL 遵循统一契约：节点 `columns` 恒为基表全列名清单，真实输出形状由 `projection`（基表列号）唯一决定。任意 SELECT 投影组合——单列（含后位列与索引列自身）、乱序多列、通配——SHALL 产出与请求一致且表头与行逐位对齐的结果，SHALL NOT panic；`EXPLAIN` 渲染 SHALL 遵循同一契约（计划树显示真实输出形状）。以该扫描为输入的聚合/分组、派生表、CTAS、视图、INSERT SELECT 的列解析 SHALL 按基表列号语义正确映射。PK 等值路由（IndexScan，「columns 已收窄」契约）与非索引回落路径 SHALL 零变化。

#### Scenario: 后位单列投影不 panic 且表头正确

- **GIVEN** 表 `q6(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER, salary INTEGER)` 与 `CREATE INDEX idx_q6_sal ON q6 (salary)`，行 (1,'Ada',1,90)
- **WHEN** `SELECT name FROM q6 WHERE salary = 90`
- **THEN** 表头 `["name"]`、行 `[["Ada"]]`（失败边界——修复前：`index out of bounds` panic，exit 101）

#### Scenario: 乱序多列投影表头与行对齐

- **GIVEN** 同上
- **WHEN** `SELECT name, id FROM q6 WHERE salary = 90`
- **THEN** 表头 `["name","id"]`、行 `[["Ada",1]]` 逐位对齐（失败边界——修复前：静默错位，表头 `["id","name"]`）

#### Scenario: EXPLAIN 渲染遵循同一契约

- **GIVEN** 同上
- **WHEN** `EXPLAIN SELECT name FROM q6 WHERE salary = 90`
- **THEN** 计划树文本正常输出且含 IndexScanAll 与真实输出形状（失败边界——修复前：`plan_format.rs` panic）

#### Scenario: PK 路由与范围谓词零回归

- **GIVEN** 同上
- **WHEN** `SELECT name FROM q6 WHERE id = 1`；`SELECT name FROM q6 WHERE salary > 80`；`SELECT * FROM q6 WHERE salary = 90`
- **THEN** 全部行为与修复前一致（既有契约与回落路径不破坏）

#### Scenario: 二级索引扫描上的聚合映射正确

- **GIVEN** 同上
- **WHEN** `SELECT COUNT(*), MAX(id) FROM q6 WHERE salary = 90`
- **THEN** 聚合结果正确（聚合输入 schema 按基表列号映射，不再受错位元数据影响）
