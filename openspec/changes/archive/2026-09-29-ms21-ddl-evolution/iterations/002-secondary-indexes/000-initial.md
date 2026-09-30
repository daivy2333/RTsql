# Iteration 002 / Cycle 000-initial: CREATE/DROP INDEX 二级索引端到端

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2-a…g（`tasks.md` T2 全部；spec `sql-secondary-indexes` R1–R5）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 000 的 A/B 趟派生键回滚机制（D7 挂钩 5 的地基）；001 的精确删除原语（`LeafNode::delete` 键相等验证）与 `delete.rs` 未命中早退（二级写路径禁用歧义原语的地基）；design D3–D8、D13–D15
- Excluded scope: ALTER（003）、TRUNCATE（004）、VIEW（005）；范围扫描/索引序/覆盖索引；组合、表达式、部分、UNIQUE 二级索引（拒绝面在 T2-c）；dump 携带索引（D15 非目标）

**Objective**

单列、非唯一、INT 列二级索引端到端可用：`CREATE INDEX` 建立并全量回填（拒绝矩阵全点名、显式事务内拒绝）、四执行器与回滚路径条目一致、干净重开与崩溃恢复两态一致、等值查询路由 `IndexScanAll` 且结果等价、`DROP INDEX` 物理释放并回落扫描；既有面零回归。

**Investigation Facts**

- Current Baseline: revision 0fe6971 + Iter000/001 未提交实施（工作区 7 文件 + change 目录）；全量 1468 passed / 0 failed / 2 ignored（Iter001 Act Response 分批新鲜结论，覆盖面未变化，直接采信）。
- Current-State Evidence:
  - **语句入口**：`Statement::CreateIndex { name: Option<ObjectName>, table_name, using, columns: Vec<OrderByExpr>, unique, concurrently, if_not_exists, include, nulls_distinct, predicate }`（sqlparser 0.44 `ast/mod.rs:1954`）现落 planner catch-all → `UnsupportedStatement`（`mod.rs:378`）；`DROP INDEX` 经 `Statement::Drop { object_type: ObjectType::Index, if_exists, names }` 走 `mod.rs:327-339` 的非 Table 拒绝臂。两处即分派扩展点。
  - **登记承载**：`CatalogRow`（`catalog.rs:57`，`unique_roots` :69）尾段序列化 :628-640、旧行兼容读 :703-729（MS23 先例）；`update_unique_index_root` :303（deserialize→mutate→serialize ordinal 定位）；`CatalogRootSlot`（`index_manager.rs:22`，`PrimaryKey{table}`/`Unique{table,ordinal}` 两变体）+ `set_catalog_context` :106 分派 :119-122；`TableMeta.unique_indexes: Vec<(usize, Arc<IndexManager>)>`（`table_manager.rs` :50 附近）即 `secondary_indexes` 的同构模板；承载通道——`create_table_with_constraints` :315、`open_or_init` :193、`attach_index_catalog_contexts` :162（`database.rs:113` open 后调用）、`replace_recovery_indexes` :523、`drop_table` :570 逐树释放。
  - **精确删除原语**：`BTree::delete_exact(&mut self, key: &[u8], row_id: RowId) -> Result<Option<PageId>>`（`btree.rs:1204`，不重平衡、根不变更返回 None——正确性无损）；`BTree::from_root` :130；`IndexManager::new` :57 / `from_root` :78；`search_all` :141 / `scan_all` :296 在位；叶层重复 key 并存既成事实（`node.rs:126-135` 判重被注释）。
  - **路由面**：PK 判定链 `query.rs:1468-1571`（`extract_pk_from_where_gated` :1475 → `IndexScan` :1481；DataScan 臂 :1510-1520 含 MS16 `pk_type_known_non_int` 门）原样保持；`IndexScanAll` 全链现成——plan 节点（`plan.rs:31,138`）、执行器（`index_scan_all.rs`）、pipeline 构造（`pipeline.rs:1022`）、plan_format 渲染（:113-118）、`get_plan_output_columns` 臂（`query.rs:92`）；planner 注册通道先例 `table_unique_columns`（`pipeline.rs:1822-1827`，`register_table` 注入）。
  - **DDL 执行面**：DDL 直包 Executor + 执行后 `plan_cache.clear()`（`pipeline.rs:260-269` / :743，成功失败都清）；显式事务内拒绝先例 = CTAS 的 in-tx execute 路径点名臂（`pipeline.rs:746-757`，`Response::Error` + `sqlstate 0A000`，CLI/lib 共用文案）——CREATE/DROP INDEX 同型臂落同一 match；`get_plan_output_columns` 的 DDL 空列臂 :145（`CreateTable | DropTable => Vec::new()`）即新变体落点。新增 `PhysicalPlan` 变体的其余穷尽匹配点（`plan.rs` 枚举+节点结构、`plan_format.rs` render/describe）由编译器指出逐点补臂。
  - **写路径挂钩点**（D7 六处）：`insert.rs:195` / :291 两个唯一挂钩区；`update.rs:299-353`（Step 7 四分支，old 值 :176 旧行反序列化）；`delete.rs:139-141` + `unique_keys_of_row` :49-75 键提取 helper；`upsert.rs` `insert_row` :314-318 / `apply_do_update` :519-541 / `delete_conflict_row:596-598`；`manager.rs::abort_cleanup_versions` A 趟（D1 派生键机制，secondary 臂镜像唯一臂）+ B 趟 `restore_tombstone_index_entries`（:401-478，回插扩展）。
  - **恢复面**：`rebuild_pk_indexes`（`recovery.rs:882-1146`）五步链；页扫描闭包 :91 / :961-1016；`extract_index_keys` :225-254（一次反序列化多提键的扩展点）；`replace_recovery_indexes` 换入 :1093；`redo_count == 0` 走 catalog 根装载、路径零变化。
  - **回填语义（design D5）**：遍历数据页链收集「被 `next_version` 指向」的 rid 集合（rebuild :961-1016 同型扫描），对链尾非墓碑版本无条件建条目——含活跃未提交版本，其回滚由 T2-d 扩展后的 abort_cleanup 二级清理兜底（两向一致）；索引列 NULL 跳过（`to_key` None；写入类型门保证 INT 列无非 Int 值）；先建空树回填、成功后一次性写 catalog 登记段（失败零副作用可重试，CTAS「建表→物化→登记」序先例）。
  - **错误面（design D14）**：复用既有变体——能力拒绝 `UnsupportedStatement(能力名)`（0A000）、`TableNotFound`（42P01）、`ColumnNotFound`（42703）、重名 `ExecutionError` 同型；不新增 `PlanError` 变体。
- Code and Critical Path: `src/storage/catalog.rs`（尾段+`update_secondary_index_root`）、`src/storage/btree/{index_manager.rs,btree.rs}`（Slot 变体、`delete_exact` 包装）、`src/storage/data/table_manager.rs`（`TableMeta.secondary_indexes` 承载五通道）、`src/wal/recovery.rs`（rebuild 扩展）、`src/parser/planner/{mod.rs,ddl_dml.rs,query.rs}`（分派、计划臂、路由）、`src/executor/`（新 `create_index.rs`/`drop_index.rs` + 四执行器挂钩）、`src/pipeline.rs`（构造臂、DDL 臂、in-tx 拒绝臂、注册通道）、`src/transaction/manager.rs`（回滚还原扩展）；见证 `tests/secondary_index_test.rs`（新）。

**Implementation Guidance**

按 tasks.md 顺序 a→g 串行推进（后项消费前项产物）。核心原则（design D3/D7）：二级索引写路径**一律 `delete_exact` + `insert`**，禁用歧义原语 `delete(key)` / `update(key, rid)`；PK/唯一索引路径零触碰。回滚还原镜像 000/001 机制：A 趟 secondary 臂按「反查键 vs prev tuple 派生键」四分支同型扩展（INSERT 形态 prev None → 删反查键；SET 索引列 rekey → 删新插旧；置 NULL → 仅删），B 趟从还原目标版本 tuple 派生二级键回插（`extract_index_keys` 扩展一次提取）。测试先 RED（拒绝矩阵可先 GREEN 后补语义用例，行为面用例必须 RED）。

**Task Contracts**

### T2-a: catalog 登记段与 TableMeta 承载

- Requirement/Scenario: R1（索引元数据 catalog 持久化、干净重开可用——承载面）；R4（删除跨重启持久——承载面）
- Depends on: None
- Targets: `src/storage/catalog.rs`（`CatalogRow` 尾段 `u32 sec_count | N × (u16 name_len | name | u32 col_index | u32 root)` + `update_secondary_index_root`）；`src/storage/btree/index_manager.rs`（`CatalogRootSlot::Secondary{table, ordinal}` + 分派）；`src/storage/data/table_manager.rs`（`TableMeta.secondary_indexes: Vec<SecondaryIndexMeta{ name, col_index, index }>` + `open_or_init` 装载 / `attach_index_catalog_contexts` 挂槽 / `replace_recovery_indexes` 扩展 / `drop_table` 逐树释放）
- Current behavior: 二级索引无任何登记与承载；旧行无尾段
- Required behavior: 尾段兼容读（旧行 → 空登记，行为 = 无二级索引）；root 变更经 `Secondary` 槽位写回 catalog；五通道与 `unique_indexes` 循环形态同构
- Preserve: 既有 `unique_roots`/DEFAULT 段序列化逐字节不变；`create_table` 委托壳签名不变
- Forbidden: 新增第三系统表（D4 已拒绝）；改动既有尾段布局
- Test witness: catalog 单测（尾段往返 + 旧行兼容读，`catalog_row_with_unique_roots_roundtrip` :1177 同型）——纯增量，无 RED 面
- GREEN condition: catalog/table_manager 单测绿 + 全量批 0（lib+bins）零回归
- Verification: `cargo test --lib`（0 failed）
- Stop when: 尾段与既有段布局冲突（结构性）——返回 Plan

### T2-b: `IndexManager::delete_exact` 精确删除原语

- Requirement/Scenario: R3（重复键多行并存的精确条目清理——承载面）
- Depends on: None
- Targets: `src/storage/btree/index_manager.rs`（新 `delete_exact(key, row_id)`：包装 `BTree::delete_exact` + `row_to_key` 按 (key, rid) 精确清理）
- Current behavior: `IndexManager` 无精确删除；`delete(key)` 对重复键只删首个且反向映射清理歧义
- Required behavior: `delete_exact` 只删指定 (key, rid) 条目；其余同 key 条目与反向映射不受扰；键存在但 rid 不匹配 → KeyNotFound 同型错误
- Preserve: `BTree::delete_exact` 不重平衡语义（D3/D15）；PK/唯一路径零触碰
- Forbidden: 修改 `BTree::delete_exact` 本体；新增重平衡
- Test witness: `index_manager_test` 直构（同 key 双 rid 注入 → delete_exact 单删 → `scan_all`/`find_key_by_row_id` 断言）——纯增量
- GREEN condition: 单测绿 + 全量批 0 零回归
- Verification: `cargo test --lib`
- Stop when: None

### T2-c: CREATE INDEX 计划臂与执行器（含回填、登记时机、拒绝矩阵、事务内拒绝）

- Requirement/Scenario: R1 全部场景
- Depends on: T2-a、T2-b
- Targets: `src/parser/planner/mod.rs`（`Statement::CreateIndex` 分派臂）、`src/parser/planner/ddl_dml.rs`（`build_create_index`：白名单臂 + 拒绝矩阵 + 新 `PhysicalPlan::CreateIndex` 节点）、`src/executor/create_index.rs`（新执行器：建空树 → 回填 → catalog 登记）、`src/pipeline.rs`（构造臂 + DDL 臂直包 + `plan_cache.clear()` + in-tx 拒绝臂 `:746-757` 同型 + 穷尽匹配补臂）
- Current behavior: `CREATE INDEX` 一律 `UnsupportedStatement`
- Required behavior: 单列 INT 列可建（存量行回填、NULL 跳过、PK 列允许冗余）；`UNIQUE`/组合/表达式/部分/`IF NOT EXISTS`/`CONCURRENTLY`/`USING`/`INCLUDE`/`NULLS DISTINCT` 修饰、非 INT 列、重名（库级唯一）、表或列不存在一律计划期点名拒绝零副作用；显式事务内执行期点名拒绝（0A000，CTAS 先例同型）；建树回填成功后一次登记（失败零副作用可重试）
- Preserve: PK 判定链与既有 DDL 分派臂零改动；plan cache 语义与 CREATE TABLE 同型
- Forbidden: 改动回填可见性判定为 MVCC 快照语义（按 D5 链尾非墓碑无条件）；新增 PlanError 变体（D14）
- Test witness: `tests/secondary_index_test.rs` 新增——拒绝矩阵（≥7 用例）+ 建索引与存量行索引化（含重复键多行）+ 干净重开可用；拒绝矩阵可直 GREEN，行为面先 RED（CREATE INDEX 现报 UnsupportedStatement）
- GREEN condition: 新用例绿 + `create_table`/`ctas`/`cli_test` 既有矩阵零修改通过
- Verification: `cargo test --test secondary_index_test --test ctas_test -- --test-threads=1`（0 failed）
- Stop when: 回填的链尾判定在运行期形态与 rebuild 扫描形态结构性不一致——返回 Plan

### T2-d: 写路径挂钩六处与回滚还原

- Requirement/Scenario: R3 全部 5 场景
- Depends on: T2-c（见证夹具经 SQL 建索引）
- Targets: `src/executor/{insert,update,delete,upsert}.rs`（D7 挂钩 1–4）、`src/transaction/manager.rs::abort_cleanup_versions`（挂钩 5：A 趟 secondary 臂 + B 趟回插扩展）
- Current behavior: 二级索引无写路径维护
- Required behavior: INSERT 新行入条目（NULL 跳过）；DELETE 按派生键 `delete_exact` 出条目；UPDATE 索引列 rekey 删旧插新 / 同值改 rid 用 `delete_exact`+`insert` / 置 NULL 仅删；UPSERT `insert_row`/`apply_do_update`/`delete_conflict_row` 同型；回滚后条目与存活版本一致（A/B 趟镜像 000/001 机制）；全部写路径**一律 `delete_exact`，禁用歧义原语**
- Preserve: PK/唯一索引既有分支逐字节不变；`delete.rs`/`upsert.rs` 的 T6/Iter000 修复语义不变
- Forbidden: 对二级索引使用 `IndexManager::delete(key)` / `update(key, rid)`；改写侧 rekey 逻辑
- Test witness: `tests/secondary_index_test.rs` 新增 R3 五场景用例（INSERT 可达 / DELETE 不命中 / rekey 新旧键 / 回滚还原含重开两态 / UPSERT 冲突清理），行为面先 RED
- GREEN condition: 新用例绿 + `insert`/`update`/`delete`/`upsert`/`explicit_tx`/`update_index_maintenance`/`mvcc_tombstone_visibility` 既有矩阵零修改通过
- Verification: `cargo test --test secondary_index_test --test explicit_tx_test --test upsert_test --test update_index_maintenance_test -- --test-threads=1`（0 failed）
- Stop when: 回滚形态出现 A/B 趟机制无法覆盖的链形态（如多语句同行链暴露新缺陷）——返回 Plan

### T2-e: 恢复重建扩展

- Requirement/Scenario: R1「崩溃恢复两态一致」+ R3 恢复两态
- Depends on: T2-a、T2-d
- Targets: `src/wal/recovery.rs::rebuild_pk_indexes`（D8 五步链扩展：空树创建、`extract_index_keys` 签名扩展兼提二级列键、非唯一不判重、批量 insert → `replace_recovery_indexes` 扩展版换入 → 逐 ordinal `update_secondary_index_root` → 旧树 tolerant 释放）
- Current behavior: rebuild 只重建 PK 与唯一索引
- Required behavior: 登记的二级索引崩溃后重建且内容与数据一致；非唯一索引不做跨链判重（跳过 :1040-1057 RedoFailed 分支）；`redo_count == 0` 路径零变化
- Preserve: PK/唯一重建语义逐字节不变；`redo_count == 0` 走 catalog 根装载
- Forbidden: 为二级索引新增判重或第二恢复通道
- Test witness: `tests/secondary_index_test.rs` 崩溃重开两态用例（T2-c 的干净重开用例之外，`:437` 模式 drop 不 close → 重开 → 等值行集一致）；先确保 T2-c/t2-d 用例中崩溃形态本就 RED 或缺失，本任务补齐
- GREEN condition: 两态用例绿 + `checkpoint_redo_reduction`/`wal_recovery_large`/`keyless_row` 既有恢复矩阵零修改通过
- Verification: `cargo test --test secondary_index_test --test wal_recovery_large_test -- --test-threads=1`（0 failed）
- Stop when: rebuild 扫描形态无法承载「链尾非墓碑」判定——返回 Plan

### T2-f: DROP INDEX 执行器

- Requirement/Scenario: R4 全部 3 场景
- Depends on: T2-a、T2-c
- Targets: `src/parser/planner/{mod.rs,ddl_dml.rs}`（`ObjectType::Index` 分派 + `build_drop_index`）、`src/executor/drop_index.rs`（新执行器：登记移除 + `collect_all_pages` 树页释放到 free-list）、`src/pipeline.rs`（DDL 臂 + in-tx 拒绝臂）
- Current behavior: `DROP INDEX` 走非 Table 拒绝臂
- Required behavior: 删除登记 + 物理释放 + 查询回落扫描（执行后 `plan_cache.clear()`）；索引不存在点名拒绝；`IF EXISTS` 点名拒绝（假设 10）；删除跨重启持久；表数据与其余索引不受影响
- Preserve: `drop_table` 既有释放通道零改动（复用同型 free-list 机制）
- Forbidden: 逻辑释放/保留目录墓碑等替代方案
- Test witness: `tests/secondary_index_test.rs` 三场景用例（回落扫描行集一致 + EXPLAIN 形状不含索引节点 + 重开持久），行为面先 RED
- GREEN condition: 新用例绿 + `drop_table_free` 既有矩阵零修改通过
- Verification: `cargo test --test secondary_index_test --test drop_table_free_test -- --test-threads=1`（0 failed）
- Stop when: None

### T2-g: 查询路由与 EXPLAIN

- Requirement/Scenario: R2 全部 4 场景
- Depends on: T2-c（索引存在性）
- Targets: `src/pipeline.rs`（`register_table` 注入 `table_secondary_indexes: HashMap<String, Vec<(String, String)>>` 通道，`table_unique_columns` :1822-1827 同型）、`src/parser/planner/query.rs`（PK 判定链未命中后的非 PK 臂前插入：单条件 `col = <Int 字面量>` 且 col 命中登记 → `IndexScanAllNode`；其余形态既有路径）、`plan_format.rs`/`get_plan_output_columns` 若有新变体臂随 T2-c 已补
- Current behavior: 非 PK 等值一律 DataScan/谓词下推
- Required behavior: 登记列等值路由 `IndexScanAll`（EXPLAIN 点名索引节点、不执行内层）；结果与建索引前完全一致（重复键多行全命中）；非 Int/NULL 字面量回落行内求值（MS16 同型）；无索引列/表零回归
- Preserve: PK 判定链 `query.rs:1468-1571` 优先级与判定原样；MS15/MS16 既有路由语义
- Forbidden: 范围/ORDER BY/覆盖索引优化；对 OR/AND 嵌入形态做路由
- Test witness: `tests/secondary_index_test.rs` 四场景用例（EXPLAIN 形状 + 结果等价 + 不可键控回落 + 无索引零回归），路由用例先 RED（现无索引可路由；等价/回落用例可直 GREEN）
- GREEN condition: 新用例绿 + `pushdown`/`keyless_eq_routing`/`explain_test` 既有矩阵零修改通过
- Verification: `cargo test --test secondary_index_test --test pushdown_test --test keyless_eq_routing_test --test explain_test -- --test-threads=1`（0 failed）+ 全量分批零回归
- Stop when: None

**Invariants**

- 二级索引写路径一律 `delete_exact` + `insert`；`IndexManager::delete(key)` / `update(key, rid)` 对二级索引禁用（D3）。
- PK 索引、唯一索引、T1/T6/T7 已收口的条目一致性语义零触碰。
- 非 unique 二级索引不做跨链判重；NULL 索引列不入索引；`redo_count == 0` 恢复路径零变化。
- 拒绝面全部计划期点名零副作用（D14 错误面，不新增 PlanError 变体）；`IF [NOT] EXISTS` 诚实化拒绝（假设 10）。
- 不新增身份型证据工程或判定层。

**Non-goals**

- 范围扫描、ORDER BY 索引序、覆盖索引；组合/表达式/部分/UNIQUE 二级索引；dump/restore 携带索引（D15）；TRUNCATE 三类索引根重置联动（004 承接）；索引列负数键排序语义（继承 PK 现状，仅点查消费无影响，D15）。

**Acceptance**

1. `sql-secondary-indexes` R1–R5 全部场景有可观察见证（拒绝矩阵、回填、四执行器维护、回滚还原、两态一致、路由形状与等价、DROP 回落）。
2. RTM：

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| sql-secondary-indexes › R1 CREATE INDEX 建立（7 场景 + 修饰拒绝 + 事务内拒绝） | T2-a/b/c | secondary_index_test 建索引/回填/拒绝矩阵/重开 | Covered |
| sql-secondary-indexes › R2 等值查询路由可达（4 场景） | T2-g | secondary_index_test EXPLAIN 形状/等价/回落/零回归 | Covered |
| sql-secondary-indexes › R3 写路径条目维护（5 场景） | T2-d | secondary_index_test 四执行器 + 回滚还原矩阵 | Covered |
| sql-secondary-indexes › R4 DROP INDEX 移除（3 场景） | T2-f | secondary_index_test 回落/拒绝/持久 | Covered |
| sql-secondary-indexes › R5 既有语义零回归 | 全部 | 全量分批零回归 | Covered |
| update-index-maintenance（000/001 交付） | — | 已 accepted | Closed |

**Verification**

- 各 Task 契约矩阵（见上）逐项 0 failed。
- 全量：按 M19 分批形态（确定性分组、批内 `--test-threads=1`、批间顺序）全部批次 0 failed（基线 1468 + 本轮新增）。
- 静态门：`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-29-ms21-ddl-evolution` 全部通过。

**Gate 2 Readiness**

Gate 2 全部 PASS（2026-09-29 用户批准本 Iteration 计划——「批准」；无 BLOCKED、无 WAIVED 项）。调查（语句分派/登记承载/原语/路由/DDL 面/写挂钩/恢复链全部 file:line 定位）、设计（D3–D8、D13–D15 闭合，已随 change 计划批准）、任务契约完备（7 契约依赖有序、拒绝面与 Preserve/Forbidden 明确）、验证最简直接（原生 cargo test 退出码）、Persisted Evidence 无必要各项通过。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 回填对活跃未提交版本建条目 + abort_cleanup 兜底的「两向一致」依赖 T2-d 先于任何含未提交写的 T2-c/T2-g 断言——执行顺序 a→b→c→d→e→f→g 已按此排列；T2-c 自己的用例不含建索引后的未提交写。
- `leaf.rs` 层重复键并存的 `search`（首个命中）在二级索引下不保证确定性 rid——路由消费 `search_all` 多行（`IndexScanAll` 既有语义），契约已限定等值路由消费面；`delete_exact` 是唯一写侧清理通道。
- `extract_index_keys` 签名扩展（R3/恢复共用）是跨任务共享面——T2-d 与 T2-e 都消费，先到者扩展、后者复用，签名变化在 Act Response 记录。
- 新 `PhysicalPlan` 变体的穷尽匹配点以编译器指出为准（已知：`plan.rs`、`plan_format.rs` render/describe、`query.rs:145` 空列臂、`pipeline.rs` 构造/DDL/in-tx 臂）；`:1598` 列索引提取函数走 `_` 防御臂即可。
- 非实质选择留给 Act：`SecondaryIndexMeta` 结构命名与组织、拒绝文案措辞（点名能力，MS19 先例形态）、用例命名与断言组织。

## Act Response

- Status: reported
- Date: 2026-09-29

### 实际改动

按契约 a→g 串行实施（T2-e 的 rebuild 扩展随 T2-a 存储层先行落地，`extract_index_keys` 签名扩展先到者扩展、T2-d 消费——契约内容零变化，见偏差 5）。

**T2-a（catalog 登记段与 TableMeta 承载，D4）**：

- `src/storage/catalog.rs`：新 `CatalogSecondaryIndex { name, column_index, root }`；`CatalogRow.secondary_indexes` 尾段（`u32 sec_count | N × (u16 name_len | name | u32 col_index | u32 root)`，紧跟唯一根段之后）；序列化/反序列化对 MS23 时代行（无二级段）与更旧行（无尾段）兼容读为空；新方法 `update_secondary_index_root`（ordinal 定位，`update_unique_index_root` 同型）/ `append_secondary_index`（CREATE INDEX 登记写）/ `remove_secondary_index`（DROP INDEX，余者保持相对次序）。
- `src/storage/btree/index_manager.rs`：`CatalogRootSlot::Secondary { table, ordinal }` 第三变体 + `sync_root_to_catalog` 分派臂（root 变更持久化到 `secondary_indexes[ordinal]`）。
- `src/storage/data/table_manager.rs`：新 `SecondaryIndexMeta { name, column_index, index }`；`TableMeta.secondary_indexes` 承载（五通道同构 `unique_indexes`：`create_table_with_constraints` 空、`open_or_init` 自 catalog 段 `from_root` 装载、`attach_index_catalog_contexts` 逐 ordinal 挂槽、`replace_index_manager` 继承、`replace_recovery_indexes` 扩第三参数、`drop_table` 逐树 best-effort 释放）；新方法 `register_secondary_index`（catalog 段追加 + 内存换入 + catalog 上下文挂接，ordinal = 登记序）/ `take_secondary_index`（按名定位表、catalog 移除、余者 ordinal 平移重挂）/ `all_secondary_index_names`（计划期名单）。

**T2-b（`IndexManager::delete_exact`，D3）**：`src/storage/btree/index_manager.rs` 新原语——包装 `BTree::delete_exact`（不重平衡、根不变更，无根/目录同步）+ `row_to_key` 反向映射按 (key, rid) 精确清理（rid 现映射键 ≠ 目标键时保留）；键在树中 rid 不匹配 → `KeyNotFound`。

**T2-c（CREATE INDEX 计划臂与执行器，D5/D13/D14）**：

- `src/executor/plan.rs`：`PhysicalPlan::CreateIndex(CreateIndexNode { index_name, table_name, column_index, column_name })` 与 `PhysicalPlan::DropIndex(DropIndexNode { index_name })`（T2-f 同批入枚举，编译器驱动补臂）。
- `src/parser/planner/mod.rs`：`Statement::CreateIndex` 分派臂 + `Statement::Drop` 的 `ObjectType::Index` 分流至 `build_drop_index`（此前落非 Table 泛化拒绝臂）；`PlanBuilder` 三条加性通道——`table_secondary_indexes`（T2-g 契约通道）、`table_column_types`（INT-only 门输入）、`existing_index_names`（库级名单）。
- `src/parser/planner/ddl_dml.rs`：`build_create_index`——修饰拒绝矩阵（UNIQUE / CONCURRENTLY / IF NOT EXISTS / USING / INCLUDE / NULLS DISTINCT / 部分索引 WHERE / MULTI-COLUMN / EXPRESSION INDEX / 限定名 / 自动命名缺名 MissingField，全部能力点名）+ 表存在性（防御臂；pipeline 通道下 register_table 先行点名 42P01）+ 列存在性 `ColumnNotFound` + INT 声明门（非 INT 点名拒绝）+ 库级重名点名拒绝；`build_drop_index`——单名、非限定、IF EXISTS 点名拒绝（假设 10）、不存在按名单点名拒绝。PK 列冗余允许（不做专门检测）。
- `src/executor/create_index.rs`（新文件）：执行序=建空树（无 catalog 上下文）→ 回填（链尾非墓碑无条件建条目，含活跃未提交版本；NULL 跳过；`deserialize_tuple` 失败传播——损坏行拒绝登记）→ 一次性 `register_secondary_index`；回填失败树页 best-effort 释放后传播错误，登记未发生零副作用可重试。
- `src/executor/drop_index.rs`（新文件）：`take_secondary_index` 登记移除 + 旧树页 best-effort 释放到 free-list（drop_table 同型）。
- `src/pipeline.rs`：`execute_stage_routed` / `execute_stage_in_tx_routed` 的索引 DDL 直包臂（执行后 `plan_cache.clear()`）+ 显式事务内点名拒绝臂（0A000，CTAS 先例同型文案）+ `create_executor_from_plan` 防御 panic 臂。

**T2-d（写路径挂钩六处与回滚还原，D7）**：

- `src/executor/insert.rs`：UNIQUE 条目区后逐二级 `insert`（NULL 跳过）。
- `src/executor/update.rs`：Step 7 后二级三支维护——同值 `delete_exact(old_key, old_rid)` + `insert(old_key, new_rid)` 换指；置 NULL 仅删；改值删旧插新。旧值快照 `secondary_old_values` 与 `unique_old_values` 同区同型。一律 `delete_exact` + `insert`（D3 禁用歧义原语）。
- `src/executor/delete.rs`：`unique_keys_of_row` 扩展为同趟兼提二级列键（SlotNotFound 容忍同型）；PK/唯一删除后逐二级 `delete_exact(key, rid)`。
- `src/executor/upsert.rs`：`insert_row` 镜像插入；`apply_do_update` 镜像 update 三支（`secondary_old_values` 快照同区）；`delete_conflict_row` 同趟提取二级键 + `delete_exact` 清理（容忍臂同型跳过）。
- `src/transaction/manager.rs`：A 趟 secondary 臂镜像唯一臂形状（派生键 Some → `delete_exact(反查键)` + `insert(派生键, prev)`；键位 NULL 仅删；派生不可得按反查键换指 prev——与唯一臂 `update` 回退终态等价且反查键条目确定在位；反查 None 的对称还原分支镜像 D17 纯插入）；B 趟 `restore_tombstone_index_entries` 同一趟派生二级键回插。`read_prev_index_keys` 签名兼提二级键（单一派生源）。
- `src/wal/recovery.rs`：`extract_index_keys` 签名扩展（`secondary_cols` 入参、三键组返回、`ExtractedIndexKeys` 别名）——一次反序列化多提键，rebuild 扫描与回滚还原共用。

**T2-e（恢复重建扩展，D8）**：`src/wal/recovery.rs::rebuild_pk_indexes` 五步链扩展——步骤 1 为 catalog 登记段逐索引建空树；步骤 2 扫描同趟提取二级键；步骤 3 链尾回溯**非唯一不判重**（`Vec<(key, rid)>` 收集，重复键多行全建条目）；步骤 4 批量 insert → `replace_recovery_indexes` 三参换入 → 逐 ordinal `update_secondary_index_root`；步骤 5 旧二级树 tolerant 释放。`redo_count == 0` 路径零变化。

**T2-g（查询路由与 EXPLAIN，D6）**：

- `src/parser/planner/query.rs`：PK 判定链未命中后、非 PK 臂前插入二级等值路由臂（单条件 `<登记列> = <Int 字面量>` 双向形态，命中 → `IndexScanAll`，`columns`/`projection` 装配镜像 IndexScan 臂）；`extract_secondary_index_equality` helper（AND/OR 嵌入不路由；非 Int/NULL 字面量 `to_key` None 自然回落；转换失败与 PK 提取同型传播）；`get_plan_output_columns` 新变体空列臂。
- `src/executor/plan.rs` / `src/executor/index_scan_all.rs`：`IndexScanAllNode.index_name: Option<String>` + 执行器 `with_index_manager` 通道（None = PK 树既有语义逐字节不变）。
- `src/pipeline.rs`：构造臂按登记名解析该树（未登记防御 `Internal`）；`register_table_schema` 注入 `table_column_types` / `table_secondary_indexes` 两通道。
- `src/executor/plan_format.rs`：`IndexScanAll` 渲染 `index=<名>` 参数仅 `Some` 时出现（None 渲染逐字节不变）；CreateIndex/DropIndex 渲染臂。
- 穷尽匹配补臂：`src/executor/correlated.rs`（叶节点臂）。

**测试见证**：新 `tests/secondary_index_test.rs` 22 用例（场景映射见文件头注记）——R1 建索引/回填等价/干净重开/崩溃两态/拒绝矩阵（非 INT、重名跨表、UNIQUE、组合、部分、IF NOT EXISTS、表列缺失、事务内 0A000）/PK 冗余允许；R2 EXPLAIN 点名索引节点/建索引前后行集等价/不可键控回落/无索引列零回归；R3 INSERT 可达/DELETE 清理/rekey 新旧键/回滚还原含重开两态/UPSERT DO UPDATE 与 REPLACE；R4 回落扫描含 EXPLAIN 无索引节点/缺失与 IF EXISTS 拒绝/删除跨重启持久。单测 +5：catalog 尾段往返与 MS23 兼容读与注册生命周期（catalog.rs 3）+ delete_exact 重复键矩阵（index_manager.rs 1）+ 登记重开与 take 平移（table_manager.rs 1）。

### 与计划的偏差

1. **重名/不存在的错误变体**（D14 措辞与 spec R1 的落点冲突，Act 裁定）：D14 写「重名 `ExecutionError` 同型」，但 `StorageError::ExecutionError` 是执行期存储错误，而 spec R1 明确「重复名……SHALL 计划期点名拒绝」。按「复用既有变体、不新增 `PlanError`」约束选 `PlanError::InvalidQuery`（58000 兜底分类），文案点名索引名（`index 'x' already exists` / `index 'x' does not exist`）；DROP INDEX 缺失同型。可观察语义（计划期、点名、零副作用）与 spec 一致。
2. **`IndexScanAllNode` 增 `index_name` 字段与执行器 `with_index_manager` 通道**：契约称「预留节点+执行器现成」，实际二者原仅消费 PK 树、无索引选择面。最小加性通道落地：`None` 为既有构造与渲染的逐字节不变路径，`Some` 时 pipeline 按登记名解析。属「非实质选择留给 Act」的承载形态。
3. **新增 builder 通道 `table_column_types` 与 `existing_index_names`**：契约 T2-c 的计划期矩阵需要逐列声明类型与库级重名/存在性名单，`table_secondary_indexes` 之外按既有加性通道先例补两条；`existing_index_names` 由 plan_stage 对 CreateIndex/DropIndex 语句一次性异步注入。
4. **plan_stage 分流细化**：DDL 分支由 `CreateTable | Drop` 收窄为 `CreateTable | Drop{Table}`——CREATE INDEX 走 register_table 通道（其目标表 schema 是计划期校验输入，表缺失在 register_table 即 42P01 点名），DROP INDEX 走无表 catch-all + 名单注入。两语句不进 plan cache（`is_cacheable` 仅 Query，现状即拒绝）。
5. **T2-e rebuild 扩展与 `extract_index_keys` 签名扩展先行于 T2-d 挂钩落地**（存储层 T2-a 编辑同文件顺带完成，避免二次触碰）：契约「先到者扩展、后者复用」授权该顺序；T2-d/T2-e 见证仍按各任务契约独立验证。
6. **catalog 二级段兼容读的边界语义**：登记段不完整（截断/UTF8 失败）→ 整段读为空（与唯一根段/DEFAULT 段 compat 读同型），不视为损坏错误。

### 自检发现与遗留

- 已修复：clippy 2 errors（模块 doc 列表缩进、`extract_index_keys` 返回类型 type_complexity → `ExtractedIndexKeys` 别名）；`cargo fmt` 对 13 文件排版归一后复跑目标矩阵与 clippy/fmt 全绿。
- 已修复（测试夹具）：rollback 两态用例首跑在崩溃重开腿报 `table 't' lookup failed during redo`——DDL 无 WAL，夹具缺 `flush_all()` 落盘步骤（`recovery_matches_runtime_after_rekey` 先例同款），补齐后 GREEN。属夹具问题非产品缺陷。
- 二级树零歧义原语核验：grep 全部二级索引调用点——写入面仅 `delete_exact`/`insert`/`search_all`/`find_key_by_row_id`（`.index.clone()` 两处为 Arc 复制），`delete(key)`/`update(key, rid)` 零命中，D3 不变量成立。
- 遗留 Minor：无新增。多语句同事务同行链回滚处理序（pre-existing，Iter000 记录）维持 Non-goal。

### 验证

- RED（先于实施）：`cargo test --test secondary_index_test` 首跑 **2 passed / 20 failed**——行为面用例全部在 `Unsupported statement type: CREATE INDEX` 缺臂处失败（两修饰拒绝用例按契约「拒绝矩阵可先 GREEN」经现 catch-all 恰好满足）；T2-c/g/f 实施后复跑 **14 passed / 8 failed**（失败集中于写路径腿——新写行不入索引，T2-d 预期 RED）；T2-d 实施后 **22 passed / 0 failed**（rollback 两态用例夹具修复后复跑同结果）。
- 契约矩阵（实施后）：`cargo test --test secondary_index_test --test ctas_test --test explicit_tx_test --test upsert_test --test update_index_maintenance_test --test wal_recovery_large_test --test drop_table_free_test --test pushdown_test --test keyless_eq_routing_test --test explain_test --test keyless_row_test --test mvcc_tombstone_visibility_test --test mvcc_abort_test -- --test-threads=1` → 逐目标 17/6/21/32/11/7/16/14/10/4/11/3，合计 **152 passed / 0 failed**。
- 全量（M19 分批形态：批 0 lib+bins + 94 集成目标按字典序 10 批、批内 `--test-threads=1`、批间顺序）：批 0 **332**/0、批 1 **98**/0、批 2 **205**/0/2i、批 3 **105**/0、批 4 **107**/0、批 5 **64**/0、批 6 **63**/0、批 7 **192**/0、批 8 **116**/0、批 9 **156**/0、批 10 **57**/0——**合计 1495 passed / 0 failed / 2 ignored**（= Iter001 基线 1468 + lib 单测 5 + secondary_index 22，计数吻合）。
- 静态门：`cargo clippy --all-targets -- -D warnings` 0 errors；`cargo fmt --check` 通过；`openspec validate 2026-09-29-ms21-ddl-evolution` → valid。

### Persisted Evidence

- None required（Plan 设 none；全部验证可由上述命令低成本重放）。

### Experience Candidates

- Runbook 候选：None（分批全量为 M19 现行约束，非新操作路径）。
- Issue 候选：None（实施中未发现 change 范围外实质缺陷）。

### 未解决问题

- None。

## Plan Review

- Review Result: accepted
- Date: 2026-09-29

### 独立检查

Plan 独立阅读完整工作区 diff（25 个修改文件 + 3 个新文件，+2285 行；含 000/001 已 accepted 内容）与新文件全文，逐项核对 Act Response 声明与代码一致：

- **T2-a**：`CatalogRow.secondary_indexes` 尾段紧跟唯一根段（`u32 sec_count | N × (u16 name_len | name | u32 col_index | u32 root)`）；MS23 时代行（唯一段后无二级段）与截断/UTF8 失败整段读空——与唯一根段/DEFAULT 段 compat 读先例同型；`update_secondary_index_root` ordinal 定位越界 `Internal`、`append/remove` 保相对次序；`CatalogRootSlot::Secondary` 分派臂；`TableMeta.secondary_indexes` 五通道（create 空 / `open_or_init` from_root / attach 挂槽 enumerate / `replace_recovery_indexes` 三参 / `drop_table` best-effort 释放）与 `unique_indexes` 循环同构——代码与契约一致。
- **T2-b**：`IndexManager::delete_exact` 包装既有 `BTree::delete_exact`（本体零修改，git diff 无 `btree.rs`——Forbidden 满足）+ 反向映射仅在仍指向本键时清除；单测覆盖同 key 双 rid 精确单删 / rid 不匹配 KeyNotFound / 缺失键不误删邻近。
- **T2-c/f**：`build_create_index` 拒绝矩阵 11 形态全点名（UNIQUE/CONCURRENTLY/IF NOT EXISTS/USING/INCLUDE/NULLS DISTINCT/部分 WHERE/组合/表达式/限定名/自动命名 MissingField）+ 附加库 `reject_attached_write` + 列存在 `ColumnNotFound` + INT 声明门 + 库级重名；`build_drop_index` 单名/非限定/IF EXISTS/不存在点名。表存在性经 `register_table` 通道（`extract_all_table_names` CreateIndex 臂 + plan_stage 分流收窄 `Drop{Table}`），防御臂齐备。
- **T2-d**：四执行器挂钩与 A/B 趟扩展逐处核对——insert 镜像唯一区后 NULL 跳过；update/apply_do_update 三支（同值 delete_exact+insert 换指 / 置 NULL 仅删 / 改值删旧插新）+ 旧值快照同区；delete 同趟兼提二级键后逐 `delete_exact(key, rid)`；upsert `delete_conflict_row` 同型 + 容忍臂跳过；A 趟 secondary 臂四分支 + D17 镜像还原分支；B 趟同趟派生回插。**D3 不变量独立核验**：grep 全部二级树调用点仅 `delete_exact`(6)/`insert`(10)/`find_key_by_row_id`/`search_all`/`collect_all_pages*`/`set_catalog_context`/`clone`，歧义原语 `delete(`/`update(` 零命中。
- **T2-e**：rebuild 五步链扩展（登记段建空树 → 同趟三键组提取 → 链尾回溯二级**不判重**全建条目 → 批量 insert + 三参换入 + 逐 ordinal 根写回 → 旧二级树 tolerant 释放）；`redo_count == 0` 路径零触碰；`extract_index_keys` 单一派生源签名扩展（运行期 `read_prev_index_keys` 与恢复扫描共用）。
- **T2-g**：路由臂插于 PK 判定链未命中后、非 PK 臂前；`extract_secondary_index_equality` 单条件双向裸列形态、AND/OR 嵌入不路由、非 Int/NULL 经 `to_key` None 回落；`IndexScanAllNode.index_name: None` 时构造与渲染逐字节不变（既有 IndexScanAll 全路径零触碰），`Some` 时 pipeline 按登记名解析（未登记防御 `Internal`）；EXPLAIN `index=<名>` 仅 Some 渲染；`is_cacheable` 仅 `Query`——两语句不进 plan cache（Act 声明核实）；DDL 直包臂执行后 `plan_cache.clear()`；in-tx 拒绝臂 0A000。
- **测试见证**：`tests/secondary_index_test.rs` 22 用例与文件头场景映射逐一对应 R1–R4 场景；单测 catalog 3 + index_manager 1 + table_manager 1 与 Act 声明一致；既有矩阵新增见证（explicit_tx rekey 矩阵/T6/T7、upsert 悬空条目）属 000/001 已 accepted 范围。
- **验证采信**：Act 本会话新鲜运行（RED 2/20→14/8→22/0 三段演进、契约矩阵 152/0、全量分批 1495/0/2i 计数自洽 = 1468+5+22、clippy/fmt/validate）覆盖未变化，直接采信不重跑；Plan 补跑检查仅代码/静态面（上述），无矛盾疑点。

### Findings（偏差分类）

1. **Plan 错误（已随 Review 勘误）**：四处 delta spec 句把显式事务内 DDL 拒绝写成「计划期点名拒绝」，而批准的契约（T2-c「执行期点名拒绝，CTAS 先例同型」）、design D5/D13（CTAS `execute_stage_in_tx` 先例）与实现（`execute_stage_in_tx_routed` 臂）均为**执行期**——builder 无事务上下文，计划期拒绝在本架构下不可达。Plan 已将 `sql-secondary-indexes` R1、`alter-table` R3 及其场景、`truncate-table` R1 共 4 句勘误为「执行期」（附「拒绝先于执行器产生任何副作用」语义注记）。用户可观察语义（0A000、零副作用、事务状态不变）不受影响，非阻塞。
2. **Act 偏离 1（重名/不存在 → `PlanError::InvalidQuery`）核实为合理**：design D14/Plan Context 原措辞「重名 `ExecutionError` 同型」与 spec R1「计划期点名拒绝」冲突——`StorageError::ExecutionError` 是执行期存储错误；Act 选既有 `PlanError::InvalidQuery`（error.rs:185 既有 58000 映射，映射表零扩展，D14「不新增变体」满足），计划期/点名/零副作用与 spec 一致。分类：Plan 措辞错误 + Act 以 spec 为准的正确裁定。
3. **Act 偏离 2–6 逐项核实**：`index_name` 字段与 `with_index_manager` 通道确为最小加性（None 全路径逐字节不变）；`table_column_types`/`existing_index_names` 两通道为契约矩阵必需输入且同先例形态；plan_stage 分流收窄使 CREATE INDEX 经 `register_table` 获得 schema 通道、DROP INDEX 走 catch-all + 名单注入，语义正确；T2-e 先行属契约授权顺序；compat 读宽忍语义与既有段一致。均非实质。
4. **Minor（不阻塞，记录不返工）**：DROP INDEX 显式事务内拒绝臂（`execute_stage_in_tx_routed` DropIndex 臂）无直接测试见证——CREATE 侧有见证（`create_index_inside_explicit_transaction_rejected`），DROP 侧同臂族同型。后续 Iteration 触碰事务/DDL 测试面时可顺带补一用例，不构成本 Iteration Acceptance 缺口（Acceptance 枚举的拒绝矩阵见证已存在）。
5. **基线记录**：工作区含 `.agents/memory/project-model.md` M19 修订（全量串行 → 分批执行，2026-09-29 用户指令，Act 直接修订）——与 Act Response「全量（M19 分批形态）」验证描述一致，非本 Iteration 产品代码。

### Acceptance Gaps

- None。RTM 五行全部 Covered：R1（建/回填/拒绝矩阵/两态/事务内拒绝/PK 冗余）、R2（EXPLAIN 形状/等价/回落/零回归）、R3（四执行器 + 回滚还原两态）、R4（回落/拒绝/持久）、R5（全量分批 1495/0/2i + 静态门）均有可观察见证。

### Iteration Plan Update

- None（Iteration Map 不变）。

### Next Cycle

- None。

### Next Iteration

- `iterations/003-alter-table-column-evolution/000-initial.md`（T3-a…d，已展开；Current Baseline 引用本 Iteration 最终 Act Response 与本 accepted Review）。
