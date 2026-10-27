# sql-secondary-indexes Specification

## Purpose

约束用户级二级索引（`CREATE INDEX` / `DROP INDEX`）的端到端语义：单列、非唯一；声明 INT 或内部归一为字符串的列可建索引，支持域外类型与全形态修饰点名拒绝；完整编码可容纳的非 NULL 存量回填并持久化（干净重开、未决 WAL，以及崩溃恢复 `redo_count > 0` 索引去信任重建后与存活数据一致；超出键容量的字符串整值写入，等值查询扫描完整值，不截断、不限制字段长度）；索引名库级唯一；被索引列等值查询路由 `IndexScanAll` 可达且结果与全表扫描严格相等（不可路由谓词回落扫描）；INSERT/UPDATE（含 rekey）/DELETE/UPSERT 写路径与事务回滚、提交前取消还原条目；字符串索引不扩大非整数主键或唯一约束；DROP INDEX 页物理释放与查询回落扫描。来源：MS21（change `2026-09-29-ms21-ddl-evolution`，improvements I068；唯一索引基建复用 MS23）。

## Requirements

### Requirement: CREATE INDEX 建立单列 INT 二级索引

`CREATE INDEX <name> ON <table>(<columns>)` SHALL 在一列或多列上建立非唯一二级索引。每一列的声明类型 SHALL 是 INT，或内部归一为 ColumnType::String 的字符串类型。NULL 不入索引。重复的整键允许并存。索引名 SHALL 库级唯一。干净重开和未决 WAL 恢复后，索引与已提交的存活行一致。

单列字符串索引的键编码 SHALL 保持现有转义：非零字节原样，输入 `00` 写成 `00 FF`，结尾 `00 00`，总长不超过 32 字节才入树。新建的单列 INT 索引 SHALL 使用 8 字节大端并翻转首字节符号位，使键的字节序与有符号数值序一致。升级前已经存在的单列 INT 索引 SHALL 继续使用原来的大端字节，等值查找字节不变。

组合键 SHALL 按索引列序拼接各列编码。INT 分量使用上述翻转后的 8 字节。字符串分量使用上述转义。拼接结果超过 32 字节时，该行 SHALL NOT 入树。系统 SHALL NOT 截断键，也 SHALL NOT 因此拒绝该行的写入或给列增加长度上限。全部列都是 INT 且列数大于 4 时，计划期 SHALL 拒绝，因为 8×列数已经超过 32 字节，没有任何行能够入树。

重名、缺表、缺列、重复的索引列、支持域外类型 SHALL 点名拒绝且零副作用。`CREATE UNIQUE INDEX`、表达式索引、部分索引、`IF NOT EXISTS` 以及其他既有修饰 SHALL 继续在计划期点名拒绝。显式事务中的 CREATE/DROP INDEX SHALL 继续在副作用之前以 0A000 拒绝。本能力 SHALL NOT 扩大非整数主键或非整数 UNIQUE 的支持面。

#### Scenario: 建索引与存量行索引化

- **GIVEN** 表 `t(id INT PRIMARY KEY, age INT, name VARCHAR)` 含若干行
- **WHEN** `CREATE INDEX idx_age ON t(age)`
- **THEN** 建索引成功，`SELECT * FROM t WHERE age = 25` 返回与建索引前相同的行集，重复 age 的多行全部命中

#### Scenario: 组合索引建在已有行上

- **GIVEN** 表含 INT 列与字符串列，其中有重复组合、NULL 和编码后超过 32 字节的字符串
- **WHEN** 按这些列创建非唯一组合索引，并按全部索引列做等值查询
- **THEN** 建索引成功。编码能放入 32 字节的非 NULL 组合全部可查到，NULL 组合与超长组合不入树，查询结果仍与建索引前的全表扫描相同

#### Scenario: 超过四列的全整数索引被拒绝

- **GIVEN** 表有至少五个 INT 列
- **WHEN** 对这五个 INT 列执行 `CREATE INDEX`
- **THEN** 计划期拒绝，错误文本含 `MULTI-COLUMN INDEX EXCEEDS 32-BYTE KEY`，表和已有索引无变化

#### Scenario: 非 INT 列拒绝

- **GIVEN** 表有 BOOLEAN 列
- **WHEN** 单独或与 INT 列一起为该列创建索引
- **THEN** 计划期点名拒绝不支持的声明类型，零副作用

#### Scenario: 重复索引名拒绝

- **GIVEN** 已存在索引 `idx_age`
- **WHEN** `CREATE INDEX idx_age ON other(c)`
- **THEN** 点名拒绝，已有索引无变化

#### Scenario: 重复列拒绝

- **GIVEN** 表有列 `age`
- **WHEN** `CREATE INDEX idx_dup ON t(age, age)`
- **THEN** 计划期拒绝，错误文本含 `duplicate column`，表无变化

#### Scenario: UNIQUE 与组合索引拒绝

- **WHEN** `CREATE UNIQUE INDEX u ON t(age)`
- **THEN** 计划期点名拒绝，零副作用。合法的多列非唯一索引不再使用这个拒绝；它的成功条件和超过四列全 INT 的拒绝见本需求的其他场景

#### Scenario: UNIQUE、表达式和部分索引仍拒绝

- **WHEN** 执行 `CREATE UNIQUE INDEX`、索引列上的表达式，或带 WHERE 的部分索引
- **THEN** 均计划期点名拒绝，零副作用

#### Scenario: 表或列不存在拒绝

- **WHEN** `CREATE INDEX i1 ON nope(c)` 或 `CREATE INDEX i2 ON t(nope)`
- **THEN** 点名拒绝，零副作用

#### Scenario: 旧整数索引字节保持

- **GIVEN** 升级前创建的单列 INT 二级索引及其已提交行
- **WHEN** 用新代码打开并按原等值查询
- **THEN** 命中行与升级前相同，不因为新的符号位编码而漏掉这些行

#### Scenario: 干净重开后索引可用

- **GIVEN** 已执行 `CREATE INDEX idx_age ON t(age)` 并显式 close
- **WHEN** 重新打开数据库
- **THEN** 索引仍可用，等值查询行集一致

#### Scenario: 崩溃恢复两态一致

- **GIVEN** 建索引后含新写入行，进程未 close 退出（崩溃模拟：WAL 重放 + 索引重建）
- **WHEN** 重新打开数据库
- **THEN** 等值查询行集与崩溃前运行期行为一致

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

被索引列与匹配类型字面量的等值谓词，在完整编码可放入 32 字节且符合裸列字面量形态时 SHALL 路由到二级索引。组合索引的等值路由 SHALL 要求谓词用等值约束索引的全部列，列在谓词中的书写顺序可以不同。EXPLAIN SHALL 显示该索引且不执行查询。支持域内的字符串比较 SHALL 使用完整值。索引路径与全表扫描的结果多重集合 SHALL 严格相等。单列、乱序多列和通配输出 SHALL 保持列元数据与投影契约：`IndexScanAll` 的列清单是基表全列，真实输出由投影决定。

NULL、类型不匹配、参数、限定列、OR、额外的非索引谓词、无索引，以及整键超过 32 字节的字面量 SHALL 保持扫描或既有错误，不因索引存在而少行。升级前的单列 INT 索引 SHALL 继续用原来的大端字节做等值路由。

#### Scenario: 等值路由计划形状可观察

- **GIVEN** 已建 `CREATE INDEX idx_age ON t(age)`
- **WHEN** `EXPLAIN SELECT * FROM t WHERE age = 25`
- **THEN** 计划文本含 `IndexScanAll` 和该索引名，且不返回查询行

#### Scenario: 组合等值与扫描相等

- **GIVEN** 组合索引覆盖两列，两列上都有重复值
- **WHEN** 对同一组等值分别在建索引前后查询，并交换谓词中的列顺序
- **THEN** 行集相同。建索引后的 EXPLAIN 点名该组合索引

#### Scenario: 超长组合等值改走扫描

- **GIVEN** 组合索引含字符串列，查询字面量的完整编码超过 32 字节
- **WHEN** 按该完整字面量做等值查询
- **THEN** 计划不是该索引的 `IndexScanAll`，结果只含真实相等的行

#### Scenario: 不可键控字面量回落行内求值

- **GIVEN** 已建二级索引
- **WHEN** 查询 NULL、与列类型不匹配的字面量，或参数占位符
- **THEN** 回落既有扫描语义，无路由相关的新错误

#### Scenario: 多叶等值连串双向严格相等

- **GIVEN** 重复键分布在多层树的多个叶子上
- **WHEN** 按该键做等值聚合
- **THEN** 行数与去重身份计数都等于全表扫描基线

#### Scenario: 共享前缀和边界长度不误匹配

- **GIVEN** 两个字符串共享长前缀但不相等，另有空串和内部零字节
- **WHEN** 按每个完整值等值查询
- **THEN** 只返回真实相等的全部行

#### Scenario: 建索引前后结果等价

- **GIVEN** 表含重复的被索引值
- **WHEN** 同一等值查询在建索引前后执行
- **THEN** 两次行集一致

#### Scenario: 无索引列零回归

- **WHEN** 查询一个没有二级索引的列
- **THEN** 行为与既有全表扫描一致

#### Scenario: 边界子树条目的 UPDATE 探针可达

- **GIVEN** 等值键分布在多层树的多个子树上
- **WHEN** 按主键更新其中一行的被索引列
- **THEN** 更新成功，旧键不再含该行，新键含该行

#### Scenario: 失败语句索引零副作用

- **GIVEN** 已建二级索引并记录某键的行数
- **WHEN** 一条更新因既有约束失败
- **THEN** 该键的行数与失败前相同

#### Scenario: 字符串等值查询可观察且与扫描相等

- **GIVEN** 含重复字符串的表和该列的二级索引
- **WHEN** 对同一等值执行 EXPLAIN，以及单列、乱序多列和通配查询
- **THEN** EXPLAIN 显示该索引且不执行查询。查询表头与行对齐，结果与建索引前的扫描相同

#### Scenario: 既有回落与唯一索引零回归

- **GIVEN** 已有主键或整数唯一索引，以及升级前的单列 INT 二级索引
- **WHEN** 做唯一键点查，并对升级前的 INT 二级索引执行 `WHERE n > k`
- **THEN** 点查与升级前一致。该旧索引上的范围查询仍走扫描。新建索引的范围行为由范围条款规定

#### Scenario: 非可路由谓词保留既有行为

- **GIVEN** 字符串二级索引存在
- **WHEN** 查询 NULL、不同类型字面量，或未被范围条款接管的谓词
- **THEN** 沿用既有扫描求值与错误语义，结果不因索引存在而改变

#### Scenario: 超容量完整字符串扫描且允许跨边界改值

- **GIVEN** 两个共享长前缀但不同的字符串，其完整编码超过 32 字节，另有容量内字符串
- **WHEN** 创建索引、按完整长值查询，再把行值在容量内外双向更新
- **THEN** 创建和合法写入成功。长值查询不是该索引的等值扫描，且只返回真实匹配行。跨界更新后的查询与全表扫描一致

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

主键索引、唯一索引、键位等值路由和约束执行 SHALL 保持。页类型与 32 字节键槽不变。本变更只改那些直接断言「组合索引被拒绝」或「新行为出现前范围一定走扫描」的既有测试；其余既有测试 SHALL 不改预期而通过。本变更新编辑的文件 SHALL 能通过格式检查。全局 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 若只在未改文件上失败，且与已登记的 ISS36 一致，则记录该结果，不把 ISS36 的修复算进本变更。

#### Scenario: 既有索引与路由面零回归

- **WHEN** 运行主键等值、整数 UNIQUE、升级前单列 INT 二级索引等值，以及本变更未改写其预期的既有测试
- **THEN** 这些测试不改断言即通过。直接断言组合索引被拒绝的测试改为新的成功或容量拒绝

#### Scenario: 主键与唯一索引零回归

- **WHEN** 运行主键等值、整数 UNIQUE 和升级前单列 INT 二级索引等值的既有测试
- **THEN** 这些测试不改断言即通过

#### Scenario: 全局格式门的既有失败不被算成新范围

- **GIVEN** ISS36 仍打开，且失败位于本变更未修改的文件
- **WHEN** 收尾运行全局格式和 deny clippy
- **THEN** 结果被记录。本变更不修改那些无关文件来换取通过

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

### Requirement: 范围谓词路由到二级索引扫描

范围谓词 SHALL 在它正好约束某个二级索引的前缀时路由到该索引。前缀是索引列序上从头开始的一段：前面的列只有等值，最后一列可以是等值、单侧比较或闭开区间。比较符是 `<`、`<=`、`>`、`>=`，字面量可以写在列的任一侧。`BETWEEN` SHALL 按两端都包含的区间处理。`NOT BETWEEN`、OR、参数、限定列、表达式、类型不匹配，以及不能由同一个索引前缀解释的额外谓词 SHALL NOT 路由。

多个索引都能解释同一谓词时，SHALL 选择被约束前缀更长的索引；长度相同则选择不需要补扫超长行的索引；仍相同则选择创建更早的索引。

键是否落入范围 SHALL 由 32 字节键的字典序决定，不把键解码回行值后再比较。端点按下列哨兵计算，编码使用本变更的有序分量。设约束前缀的编码为 P，长度为 n：

- 包含的下界是 P 后补零到 32 字节，键大于等于它。
- 不包含的下界：n 小于 32 时是 P 后补 `FF` 到 32 字节，键大于它；n 等于 32 时是这 32 字节本身，键大于它。
- 包含的上界：n 小于 32 时是 P 后补 `FF` 到 32 字节，键小于等于它；n 等于 32 时是这 32 字节本身，键小于等于它。
- 不包含的上界是 P 后补零到 32 字节，键小于它。

升级前的单列 INT 索引使用旧的大端字节，其顺序不等于有符号数值序。这种索引上的范围谓词 SHALL 继续走全表扫描。新建的单列 INT 索引和全 INT 组合索引的每个非 NULL 值都能入树，范围扫描 SHALL 只读该树。字符串索引以及任何可能因超过 32 字节而漏掉非 NULL 行的索引，范围结果 SHALL 是树内命中的行，加上键派生为空且谓词成立的可见行。NULL 不满足范围比较，不得凭补扫被返回。两种路径的结果多重集合 SHALL 与全表扫描严格相等。

EXPLAIN SHALL 显示 `IndexRangeScan` 和索引名，并且不执行查询。需要补扫时文本还含 `omitted=scan`；不需要时不含该标记。

#### Scenario: 新建整数索引的范围含负数

- **GIVEN** 本变更之后创建的单列 INT 二级索引，行值含负数、0 和正数
- **WHEN** 分别查询 `列 > 0`、`列 < 0`、`列 >= -1` 和 `列 BETWEEN -2 AND 2`
- **THEN** EXPLAIN 含 `IndexRangeScan` 和索引名，且不含 `omitted=scan`。结果与全表扫描相同，负数只出现在数值序确实落入区间的查询里

#### Scenario: 旧整数索引的范围仍扫描

- **GIVEN** 升级前创建的单列 INT 二级索引
- **WHEN** 查询 `列 > 0`
- **THEN** 计划不使用 `IndexRangeScan`。结果与全表扫描相同，等值查询仍命中原行

#### Scenario: 组合前缀的端点

- **GIVEN** 全 INT 组合索引 `(a, b)`，行含 `(1, 9223372036854775807)` 和 `(2, -9223372036854775808)`
- **WHEN** 查询 `a > 1`、`a = 1` 和 `a = 1 AND b > 2`
- **THEN** `a > 1` 含第二行、不含第一行。`a = 1` 含第一行、不含第二行。`a = 1 AND b > 2` 含第一行、不含第二行。三个结果都与全表扫描相同

#### Scenario: 字符串范围不漏超长值

- **GIVEN** 字符串二级索引，同时有可入树的短字符串和共享前缀但编码超过 32 字节的长字符串
- **WHEN** 用一个小于两者的字面量做大于比较
- **THEN** EXPLAIN 含 `IndexRangeScan`、索引名和 `omitted=scan`。短字符串与长字符串都在结果中，且与全表扫描相同

#### Scenario: 非前缀和析取不路由

- **GIVEN** 组合索引 `(a, b)`
- **WHEN** 只约束 `b`，或使用 OR，或在范围之外再加一个索引不能解释的谓词
- **THEN** 计划不使用 `IndexRangeScan`，结果与全表扫描相同

### Requirement: 组合二级索引的条目与存活行一致

INSERT、UPDATE、DELETE、UPSERT 和 REPLACE SHALL 用该索引登记的列序和编码世代维护组合键。任一索引列为 NULL，或拼接后超过 32 字节时，该行在这个索引中没有条目。一行的索引列发生变化时 SHALL 删除旧键并写入新键，不得误删其他行的重复键。回滚、失败和提交前取消经既有回收后 SHALL 恢复条目。提交后结果未送达 SHALL 继续视为已提交。未决 WAL 恢复重建 SHALL 使用同一派生。

删除被组合索引任一列引用的列 SHALL 拒绝，错误沿用「被二级索引引用、须先删除索引」。未被引用的列删除后，其余索引的列号 SHALL 按删除位置重新编号。`DROP INDEX` 后对应查询 SHALL 回到扫描，行集不变。

#### Scenario: 改组合中的一列

- **GIVEN** 组合索引 `(a, b)` 上有两行共享同一个 `a`、`b` 不同
- **WHEN** 只更新其中一行的 `b`，再按新旧组合查询
- **THEN** 新组合只含被更新的行，旧组合仍含另一行

#### Scenario: 回滚后条目回到原状

- **GIVEN** 显式事务修改组合索引的多行
- **WHEN** 回滚后按原组合查询，并重新打开数据库
- **THEN** 行和可路由查询的结果都与事务开始前相同

#### Scenario: 删除被引用列被拒绝

- **GIVEN** 组合索引使用表中的非主键列
- **WHEN** `ALTER TABLE DROP COLUMN` 删除其中一列
- **THEN** 语句失败，列和索引都还在

### Requirement: 唯一性边界保持在整数单列

组合二级索引 SHALL 允许重复的整键。`CREATE UNIQUE INDEX`、表级组合 UNIQUE、非 INT 列上的 UNIQUE，以及非整数主键 SHALL 继续按现有文本拒绝。本变更 SHALL NOT 把唯一约束扩展到字符串或组合键。

#### Scenario: 重复组合键可以并存

- **GIVEN** 两行在组合索引的全部列上值相同，且该键能放入 32 字节
- **WHEN** 创建该组合索引并按这个组合查询
- **THEN** 创建成功，两行都返回

#### Scenario: 组合唯一约束仍被拒绝

- **WHEN** 在建表时声明多列 UNIQUE，或执行 `CREATE UNIQUE INDEX`
- **THEN** 计划期点名拒绝，零副作用
