# MS21 DDL 演进与索引面 — Design

调查输入：2026-09-29 五路只读实现调查（事务/回滚面、存储与恢复面、DDL 基建与 sqlparser AST、行格式与物理释放面、视图展开与 catalog 面），基线 1458 tests / 0 failures / 2 ignored（2026-09-29 MS20 收尾，工作区未变）。全部事实带 file:line，摘要见各 Iteration Plan Context。

## D1 — T1-①：A 趟 rekey 回退改为「删搜索键 + 按 Tuple 派生键条件回插」

`abort_cleanup_versions` A 趟（`src/transaction/manager.rs:318-355`）现以 `find_key_by_row_id` 反查所得键执行 `update(&key, prev)`（:325/:337）。rekey UPDATE 回滚时反查键是**新键**而 prev 版本 tuple 携带**旧键**——修复形态：

- `next_version() == Some(prev)` 分支改为：`delete(反查键)` 后，经 `read_tuple_from_data_page` 读 prev 版本 tuple（B 趟 :410-414 同型单次页读）+ `extract_index_keys`（`pub(crate)`，已在本文件 :427-428 消费）派生 prev 真实键；派生键为 `Some` 且 ≠ 反查键 → `insert(派生键, prev_row_id)`；派生键 == 反查键（非 rekey）→ 保持既有 `update(&key, prev)` 不变（零行为变化的快路径）；派生键 `None`（prev 键位为 NULL）→ 仅删除不回插。
- PK 臂（:321-328）与唯一索引臂（:334-342）同型同修。
- 不触碰 `update.rs`/`upsert.rs` 的 rekey 写侧（MS24 001-replan Forbidden 先例）。分桶趟（:300-316）语义不变。
- 恢复侧零改动：aborted 事务记录永不被重放（`recovery.rs:498-503, :527-533`），崩溃态索引由 rebuild 从数据页收敛，两态一致由既有 `recovery_matches_runtime_after_rekey` 断言模式（`tests/update_index_maintenance_test.rs:437`）覆盖。

## D2 — T1-②：仲裁命中键穿入 `delete_conflict_row`

`upsert.rs::delete_conflict_row`（:550-618）的 `SlotNotFound` 容忍臂（:567）返回 `(None, Vec::new())` 导致 PK 条目删除（:590-594）被跳过。对齐 `delete.rs:135`（按已知键无条件删条目）：调用点（REPLACE 臂 :644-656）把仲裁 `search` 命中所用的 PK 键值穿入 `delete_conflict_row`；slot 不可读时仍以该键执行 `index_manager.delete`，墓碑写入维持既有跳过。唯一条目在无 tuple 时无键可派生，保持跳过（与 delete.rs 的 `unique_keys_of_row` 容忍同型）。测试经 pub API 直构：建表插行后经 `table_meta.index_manager.insert(多余键, 悬空 rid)` 注入条目（`tests/drop_table_free_test.rs:102-134` 证明 `get_table().index_manager` 可达；直构先例 `tests/index_manager_test.rs:15`），REPLACE 命中该键后断言条目被清理。

## D3 — 二级索引重复键承载：平铺重复条目 + `IndexManager::delete_exact`

B-Tree 叶层重复 key 并存是既成事实（`node.rs:126-135` 判重被注释；`search_all`/`delete_by_key`/`delete_exact` 均在位 `btree.rs:1065-1288`）。选定**平铺重复条目**（同 key 多个 (key, rid) slot），拒绝复合键方案（需要前缀范围扫描新原语）：

- 新增 `IndexManager::delete_exact(key, row_id)`：包装既有 `BTree::delete_exact`（不重平衡、不回传根变更——正确性无损，仅叶稀疏）+ `row_to_key` 反向映射按 (key, rid) 精确清理。
- 二级索引的写路径**禁用**歧义原语 `delete(key)`（删第一个 ≥ 匹配位，`node.rs:222-235`）与 `update(key, rid)`——一律 `delete_exact` + `insert`。PK/唯一索引路径不动。
- 查询侧 `search_all(key)` 语义与预留执行器 `IndexScanAllExecutor`（`executor/index_scan_all.rs`）直接匹配，零新执行器。

## D4 — 索引登记承载：`__tables` 行尾随段扩展（拒绝第三系统表）

新系统表 `__indexes` 需要 bootstrap 预留页 2，而存量文件页 2 已是用户数据、`Catalog::open` 无第三根发现通道（`catalog.rs:106-169`；file_header 预留位方案触碰格式头演进）——拒绝。选定 **`CatalogRow` 尾随段二次扩展**（MS23 unique_roots、MS24 DEFAULT 段三次先例）：

- 尾段追加：`u32 sec_count | N × (u16 name_len | name | u32 col_index | u32 root)`；旧行缺尾段 → 空登记（兼容读同 `catalog.rs:700-720` 模式）。
- `TableMeta` 新增加性字段 `secondary_indexes: Vec<SecondaryIndexMeta{ name, col_index, index: Arc<IndexManager> }>`（`unique_indexes` 同构）。
- `CatalogRootSlot` 增第三变体 `Secondary{table, ordinal}`，`sync_root_to_catalog` 分派扩展（`index_manager.rs:21-30, 115-130`）；新增 `catalog.update_secondary_index_root(name, ordinal, root)`（`update_unique_index_root` 的 ordinal 定位模式 `catalog.rs:303-333`）。
- `open_or_init` 装载、`attach_index_catalog_contexts` enumerate 挂槽、`replace_recovery_indexes` 扩展、drop_table 逐树释放、TRUNCATE 根重置——全部按 `unique_indexes` 既有循环形态复制。

## D5 — CREATE INDEX 回填规则与事务边界

- **回填**：遍历数据页链收集「被 `next_version` 指向」的 rid 集合（rebuild 同型 `recovery.rs:961-1016`），对**链尾非墓碑版本无条件建条目**（含活跃未提交版本——其回滚由 T1 扩展后的 abort_cleanup 二级索引清理兜底，两向一致）；索引列值 NULL → 跳过（to_key None，同 PK/唯一语义，写入类型门保证 Int 列无非 Int 值）。
- **登记时机**：先建空树回填，成功后一次性写 catalog 登记段（失败零副作用可重试，CTAS「建表→物化→登记」序先例）。
- **显式事务内拒绝**（0A000，修订 proposal 假设 11）：回填一致性依赖无同表未提交写的稳定视图；CTAS 事务内拒绝先例 `pipeline.rs:746-755`。

## D6 — 查询路由：非 PK 臂扩展简单等值 → `IndexScanAll`

PK 判定链（`query.rs:1468-1571`）原样保持、优先级不变。在其未命中后的非 PK 臂（:1501-1570）前插入：`PlanBuilder` 新增加性通道 `table_secondary_indexes: HashMap<String, Vec<(String, String)>>`（索引名, 列名；`register_table_schema` 注入，`table_unique_columns` 同型 `pipeline.rs:1822-1827`）。WHERE 为单条件 `col = <Int 字面量>` 且 col 命中登记 → 构造 `IndexScanAllNode`（预留节点+执行器+渲染臂+输出列臂全部现成，`plan_format.rs:113-118`、`query.rs:92`）。其余形态（AND 嵌入、OR、非 Int 字面量）一律既有路径零回归。`EXPLAIN` 展示该索引节点名。

## D7 — 二级索引写路径挂钩面（六处，全部镜像既有唯一索引代码形状）

1. `insert.rs:291-295` 后：逐二级索引 `insert`（NULL 跳过）。
2. `update.rs:299-353`：SET 命中索引列时的四分支镜像（同值 update-rid 改用 delete_exact+insert；改值 delete_exact(old)+insert(new)；置 NULL delete_exact(old)），old 值取自 :176 旧行反序列化。
3. `delete.rs:139-141` 后：逐二级索引 `delete_exact(tuple 派生键, rid)`（扩展 :49-75 键提取 helper）。
4. `upsert.rs::insert_row:314-318` 后、`apply_do_update:519-541` 镜像、`delete_conflict_row:596-598` 后补 delete_exact。
5. `manager.rs::abort_cleanup_versions`：A 趟 D1 修复天然覆盖（按派生键回插/删除扩展到二级列表），B 趟 `restore_tombstone_index_entries` 扩展二级条目回插。
6. `recovery.rs::rebuild_pk_indexes`：见 D8。

## D8 — 恢复重建扩展

`rebuild_pk_indexes`（`recovery.rs:882-1146`）五步链扩展：步骤①为每登记索引新建空树；步骤② `extract_index_keys` 扩展签名兼提二级列键（一次反序列化多提键，现有形态 :225-254）；步骤③链尾回溯判定不变，**非唯一索引不做跨链判重**（跳过 :1040-1057 的 RedoFailed 分支）；步骤④批量 insert → `replace_recovery_indexes` 扩展版整表换入 → 逐 ordinal `update_secondary_index_root`；步骤⑤旧树 tolerant 释放循环复制。`redo_count == 0` 路径零变化（clean 打开走 catalog 根装载）。

## D9 — ADD COLUMN：尾随加列 + 宽度感知反序列化 + DEFAULT 覆盖层

- **tuple 层新增宽度感知读取**：`deserialize` 族增变体返回「实际解码值数」（tag 自描述逐值消耗，`tuple.rs:17-29`；不足 schema 宽度时以实际数为准而非 EOF——现状 EOF 在 `tuple.rs:159-161`）；超出 schema 宽度时截断（末列 DROP 的旧行）。值填充语义：`i >= 实际数` 的位由消费侧用 `TableMeta.defaults[i]`（列序对齐，`table_manager.rs:71`）覆盖，无 DEFAULT → NULL。该 helper 被全部 tuple 消费点统一采用（扫描四执行器、update/upsert/delete 读回、recovery `extract_*`——恢复键提取与运行期同源，两态一致）。
- **恢复安全论证**：DDL 不写 WAL；加列前重放的 INSERT 产生窄行（填充读取无害）、加列后重放产生宽行（原样）——两种宽度并存安全（页内 slot 独立 offset/length，`slotted_page.rs:202-246`）。
- **catalog**：`__columns` 追加单列行走 `append_to_chain`（新小 API `insert_column_row`）；`__tables.column_count` 经 `update_field_in_chain` 通道改写；内存 TableMeta 重建换入（`replace_index_manager` 先例 `table_manager.rs:483-513`）。
- **计划期校验**：类型经 `convert_data_type`；仅 DEFAULT/NOT NULL 可带；NOT NULL 无 DEFAULT 且表非空（数据页链存在活 slot）→ 拒绝；PK/UNIQUE/CHECK/FK → `UnsupportedConstraint` 点名拒绝。
- **写路径**：INSERT 子集清单缺省填充已按 `table_defaults` 通道工作（加列后 defaults 含新列即自动生效，`ddl_dml.rs:438-439`）；UPDATE 读回（填充后全宽）→ 改列 → 全宽写新版本（`update.rs:176/:250-261`），MS24 类型门不受宽度影响。

## D10 — DROP COLUMN：统一内隐式事务重写通道（恢复安全）

惰性读投影被否决：tuple 无列数标记，中间列删除后新旧行静默错位（`tuple.rs:158` 消费语义），且 DDL 不写 WAL、恢复按 row_id 位置寻址重放（`recovery.rs:574-668`）会把崩溃前重放的宽行复活到删列后的窄 schema 上。全行重写为「直接改页」同样不安全（同重放复活问题）。

选定**内隐式事务逐行 DELETE+INSERT 重写**（全部走既有 WAL Insert/Delete 记录与版本链，MS20 CTAS「隐式事务 InsertExecutor 落行」先例）：

1. 计划期守卫：目标非基表 / 列不存在 / 列被 PK、唯一、二级索引引用 / 表仅剩一列 → 点名拒绝零副作用。
2. 新列集与列 ordinal 平移（`pk_index`、`unique_indexes`、`defaults`、二级 `col_index` 同步重排）先算出。
3. 内隐式事务：逐活行 DELETE（墓碑 + 索引条目清理 + WAL Delete）→ INSERT 平移后窄行（索引条目重建 + WAL Insert）；整表一个事务提交——崩溃恢复重放得到全有或全无。
4. catalog：删除该列行、后续列行 ordinal 重写（append+delete，`update_field_in_chain` 策略；需新 matcher 变体支持按 (table, column_index) 定位）→ TableMeta 重建换入。
5. `plan_cache.clear()`；显式事务内拒绝（0A000，同 D5）。

## D11 — TRUNCATE：checkpoint-first 序

直接物理释放会被崩溃重放复活行（保留 catalog 行 → 重放 `get_table` 成功 → `redo_tuple_at_row_id` 位置寻址写回已清页）。选定顺序：

1. 计划期守卫：显式事务内拒绝、多表拒绝、非基表拒绝、缺失拒绝（proposal 假设 8）。
2. **先 `Database::checkpoint()`**（pub，`database.rs:239-260`）：刷脏页 + WAL 截断到位点——此后 WAL 无任何待重放的 pre-TRUNCATE DML。
3. 物理清除：`collect_data_pages` 收集 head 之后页全部 `free_page`（零化）；head 页 `SlottedPage::init` 原地重置（`catalog.rs:340-346` 先例）；`update_table_tail(head)` 回写（`catalog.rs:246-263`，head 字段无需新 API）。
4. 索引根重置：PK + 唯一 + 二级各建空树换入（`replace_index_manager`/扩展版）+ 各根 `update_*_root` 写 catalog；旧树 `collect_all_pages` 释放。
5. `plan_cache.clear()`。

崩溃窗口分析：步骤 2 前崩溃 → 旧状态完整（语句未生效）；步骤 3 中途 → 部分清除（数据丢失但 schema/索引结构完整，与 drop 的既有 DDL 崩溃窗口同类，诚实文档化）；不存在「复活已清行」形态。

## D12 — VIEW：SQL 文本持久化 + 创建期引用验证 + CTE 通道展开

- **承载**：`CatalogRow` 尾段加 `u8 kind`（Table/View；旧行缺省 Table）+ 视图行携带 `u16 len | SQL 文本`；视图列名复用 `__columns` 行（column_index 序）。存量文件零迁移（bootstrap 不动，页 0/1 断言不变）。SQL 文本超 ~3.5KB（slot 上限）→ 计划期拒绝（`PageFull` 前置长度检查）。
- **创建期验证封死循环**：CREATE VIEW 时对定义查询即时 `build_query` 验证（CTAS 先例 `ddl_dml.rs:942-966`）——引用对象必须是已存在的基表或已存在视图（此时被引用方已通过同一验证），自引用/循环因「引用尚不存在的视图」被拒，**运行期无需环检测**。列名 = 显式清单（长度校验 + 覆盖，`consume_with` 的 CTE 列别名同型 `query.rs:836-853`）或定义输出表头。
- **展开**：`register_table`（`pipeline.rs:1848-1919`）闸门——名未命中主库表且未命中 attach 路由 → 查视图注册表：命中则注入 builder 通道（`views: HashMap<String, ViewMeta{sql, columns}>`）不报错；未命中 → 既有 42P01。planner FROM/JOIN 名字钩子（`query.rs:332/:412`，`resolve_cte_reference` 同型）：命中视图 → `parse_sql(定义文本)` → 顶层上下文 `build_query`（CTE 通道语义，定义内 ORDER BY/LIMIT/集合/表达式项全可达，`query.rs:806-810`）→ 包 `DerivedScanNode`（显式列名覆盖 columns）→ `register_table`。同语句多次引用 clone 同一子计划（CTE `:882-883` 先例）。
- **DML 只读拒绝**：`build_insert/Update/Delete` 消费 builder 视图通道 → 命中即 `UnsupportedStatement("对视图的写入")` 点名拒绝（`reject_attached_write` 同位形 :262-269）。
- **命名空间**：视图与表各自独立存在性检查 + 交叉互查（CREATE VIEW 重名拒绝、CREATE TABLE 重名拒绝覆盖视图名、DROP TABLE 对视图名按表不存在拒绝、ALTER/TRUNCATE/CREATE INDEX 目标为视图拒绝）。
- **IN 子查询缺口补齐**：`get_subquery_first_column` 增 `DerivedScan` 臂（返回首列名，`subquery.rs:386-446` 现状兜底误报多列——CTE 同现状，随视图一并收口）。
- **plan cache**：CREATE/DROP VIEW 走 DDL 臂全量 `plan_cache.clear()`（引用视图的 SELECT 计划内嵌展开子树，全清封死复活窗口，`pipeline.rs:267-268` 同款）。
- **持久化加载**：`open_or_init` 对 kind=View 行解析入内存视图注册表（`Database` 持有，`RwLock<HashMap>`；`open_or_init` 全量载入先例 `table_manager.rs:193-275`）。

## D13 — 事务边界修订（对 proposal 假设 11 的修订，Gate 2 待批）

| 语句 | 显式事务内行为 | 依据 |
|---|---|---|
| CREATE INDEX / DROP INDEX | **拒绝（0A000）** | D5：回填一致性与 DROP 登记一致性依赖无并发未提交写窗口 |
| ALTER TABLE ADD/DROP | **拒绝（0A000）** | D10：重写事务与用户事务嵌套会产生未提交宽度混合行 |
| TRUNCATE | 拒绝（0A000） | 已批准假设 8 |
| CREATE VIEW / DROP VIEW | 沿 CREATE TABLE 现状（立即生效、不随回滚撤销） | catalog-only，`tx_statement_test.rs:348-365` 既有语义 |

## D14 — 错误面映射

复用既有变体与 SQLSTATE：能力拒绝 → `UnsupportedStatement(能力名)`（0A000）；表/视图不存在 → `TableNotFound`（42P01）；列不存在 → `ColumnNotFound`（42703）；约束声明 → `UnsupportedConstraint`（0A000）；索引列非 INT → `UnsupportedStatement` 点名。新增拒绝不新增 PlanError 变体（error.rs:139-156 映射表零扩展，全变体可枚举测试不受影响）。

## D15 — 非目标确认（新增）

- dump/restore 不携带 CREATE INDEX 与 CREATE VIEW 语句（dump 文本仍为 CREATE TABLE + INSERT；索引/视图不跨 dump 存续——文件对备份 R30 路径不受影响）。
- 范围扫描、ORDER BY 索引序、覆盖索引；二级索引负数键排序语义继承 PK 现状（i64 BE 字典序，仅点查消费无影响）。
- `delete_exact` 不做叶重平衡（现状 `btree.rs:1204-1208`，正确性无损）。

## D16 — T6：删除原语精确化（DELETE 未命中键零副作用；2026-09-29 用户指令扩围，来源 Iter000 Act Response Issue 候选 1）

缺陷（CLI 实测 repro 见 Iter000 Act Response）：`LeafNode::delete`（`node.rs:222-235`）与 `LeafNode::update`（:251-263）不同——命中位不验证键相等，`find_key_position` 的「首个 ≥」插入位被直接 `delete_slot`；唯一未设防调用点 `delete.rs:135` 在 `search` 未命中时仍无条件删键，且 `:169` 恒返回 `AffectedRows(1)`——中间缺失键误删邻近条目（点查漏行、UNIQUE 静默失效），大于全部键经 `IndexManager::delete`（`index_manager.rs:272-294`）传播 `KeyNotFound` 误报。

修复形态：

- `LeafNode::delete` 增加命中位键相等验证（取键方式与 `update` :258-262 同型），不等或越界 → `KeyNotFound`——歧义「插入位删除」原语对所有调用者消除。
- `DeleteExecutor::next` 在 search 未命中时早退 `AffectedRows(0)`（先于唯一键提取/墓碑/PK 删除/WAL）；命中路径（含 SlotNotFound 容忍臂——search 命中键在树中存在，精确删除成立）逐字节不变。
- 其余 `IndexManager::delete` 调用点（upsert/update/manager A 趟，见 Iter001 Plan Context 枚举）删除的均为调用时存在的键（反查映射或可读 tuple 派生），精确化后行为不变；唯一列删除面对损坏态从「静默误删」变「显式报错」，属改进。
- 与 T2-b `delete_exact` 的分工：D16 修「存在性歧义」（键不在树中），T2-b 修「重复键歧义」（同 key 多 rid 精确定位）——二级索引写路径两者皆需，互不替代。
- DELETE 计划面闭合性：`build_delete`（`ddl_dml.rs:1083-1105`）仅支持 PK 等值 WHERE，DeleteExecutor 是 DELETE 语句的全部执行面，修复面完整。

## D17 — T7：A 趟唯一臂 NULL-rekey 还原（2026-09-29 用户指令扩围，来源 Iter000 Act Response 遗留观察 / Issue 候选 2）

缺陷：UPDATE 唯一列 值→NULL 回滚后旧唯一键条目缺失（同值重插不再被 UNIQUE 拒绝，静默失效）。机制：分桶按 **PK** 反查（`manager.rs:300-316`）——UPDATE 未动 PK，版本落 A 趟；A 趟唯一臂以 `uindex.find_key_by_row_id` 反查为门——`update.rs` Step 7 唯一 rekey（:343-347）已删旧条目且 NULL 不入索引，反查 None → 整臂跳过 → 旧值占用丢失。对照：PK→NULL rekey 因 PK 条目同删落 B 趟，`restore_tombstone_index_entries` 已正确还原 PK 与唯一——缺陷恰为 A 趟唯一列维度的还原臂缺失（PK 不隐含 NOT NULL（`table_manager.rs:260` 仅声明面），但 PK→NULL 形态已由 B 趟覆盖，无需处理）。

修复形态：A 趟唯一臂反查 None 侧增加对称分支——prev 存在时经 `read_prev_index_keys`（Iter000 D1 已建，SlotNotFound/反序列化失败 → `Ok(None)` 容忍、键位 NULL → `Some((None,…))`）派生 prev 唯一键，`Some` → `uindex.insert(派生键, prev)` 还原，`None` → 跳过（与 D1「prev 键位 NULL 仅删除」臂镜像对称）。PK 臂无需同型（A 趟内 PK 反查恒 Some，否则落 B 趟由 B-pass 还原）。作用域：单语句 UPDATE——多语句同行链的处理序依赖为 pre-existing Minor（Iter000 已记录），不在本 Cycle。
