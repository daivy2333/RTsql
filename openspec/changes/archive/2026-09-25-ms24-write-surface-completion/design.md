# MS24 SQL 写面补全 — Design

> 调查基线：master 工作树 @ MS23 收尾后（2026-09-25，1152 tests 全量新鲜，本 change 涉及表面自该结论零变化）。
> 用户裁定记录：MS24 三任务单 change 双 Iteration；DO UPDATE SET = 字面量 + `excluded.col` + 旧行裸列引用（算术/函数点名拒绝）；DO UPDATE WHERE v1 点名拒绝；FLOAT 列整数值无损升格；冲突目标按 SQLite 对齐（省略=全仲裁、显式单列、组合/ON CONSTRAINT 精确拒绝、REPLACE INTO 实现；`INSERT OR ...` 因 GenericDialect 不可达维持解析层拒绝）。

## D1 DEFAULT 持久化通道

`CatalogColumnRow`（`src/storage/catalog.rs:71`）追加 `pub default_value: Option<Value>`；`serialize_catalog_column_row`（:738）在既有固定布局尾部追加 `u8 has_default` +（has_default=1 时）`u8 value_tag | payload`——tag 复用 tuple.rs TAG_* 值域，payload 定宽（Int i64-LE 8B / Float f64-LE 8B / Bool 1B / Date i32-LE 4B / Timestamp i64-LE 8B / String u16 len + bytes / Null 0B）；`deserialize_catalog_column_row`（:764）既有字段读完后按剩余长度可选读（不足 → None）。依据：现反序列化器顺序读、不拒绝尾随字节，checkpoint 位点 24B 兼容读与 MS23 unique_roots 追加为先例。`DEFAULT NULL` 声明以 has_default=1 + Null tag 全保真往返（dump 渲染不丢声明）。

`TableMeta`（`src/storage/data/table_manager.rs:50`）加性 `pub defaults: Vec<Option<Value>>`（与 `columns` 列序对齐，镜像 MS23 `not_null` 模式）：`create_table_with_constraints`（:307）入参 4 元组扩为 5 元组 `(name, type, not_null, unique, default)`（旧 `create_table`（:282）委托壳补 `None`）；`open_or_init`（:187 区）从 `CatalogColumnRow.default_value` 读回。`replace_index_manager` 继承不动（defaults 不经它传递，随 TableMeta 重建通道走 open 读回）。

`PlanBuilder`（`src/parser/planner/mod.rs:99`）加性 `table_defaults: HashMap<String, Vec<Option<Value>>>` + `set_table_defaults(name, defaults)`（镜像 `set_pk_column_type` 模式）；注册点 `pipeline.rs::register_table`（:1084）从 `TableMeta.defaults` 传递。

**替代方案（否决）**：不持久化、仅会话内存——重启后子集 INSERT 语义静默漂移（DEFAULT 列填 NULL），违反两态一致纪律；executor 侧填充——INSERT 行在 plan 期已重排为全宽行，DEFAULT 关键字与省略位填充需要 defaults 元数据在 plan 期可得（`DEFAULT` 关键字映射目标列），且 UpsertNode 的 excluded 引用同样消费 defaults；planner 通道单点覆盖两者。

## D2 子集 INSERT 与 DEFAULT 填充点

`extract_insert_values`（`ddl_dml.rs:219`）行值类型改为内部 `Vec<Vec<InsertValue>>`（`InsertValue = Val(Value) | DefaultKeyword`）：`Expr::Identifier` 值 `DEFAULT`（sqlparser 0.44 无 `Expr::Default` 变体，VALUES 中 DEFAULT 关键字落入既有 Identifier 臂，现被 `UnsupportedValue` 拒绝）映射 `DefaultKeyword`；其余臂保持。仅 `build_insert` 消费（grep 确认单调用点）。

`map_insert_values`（:151）行为变化：列清单从「恰为全列排列」放宽为「每项解析为互异已知列的子集」——未知列 `ColumnNotFound`、重复列、无清单行长度校验保持；行长度校验变为 `row.len() == columns.len()`（清单长度）；重排后对省略位（含 `DefaultKeyword` 位）填充 `table_defaults`（有 DEFAULT → 克隆；无 → `Value::Null`）。`DefaultKeyword` 在无清单分支同样按表列位等价填充（位置即列位——`INSERT INTO t VALUES (1, DEFAULT)` 合法，与 SQLite 一致；该形态现被 `UnsupportedValue` 拒绝，属新行为面，既有合法语句零变化）。输出恒为全宽 `Vec<Vec<Value>>`。NOT NULL 且无 DEFAULT 的省略列填 NULL 后由既有执行器门拒绝（单点强制，D3 不新增计划期重复路径）。全列清单与无清单 INSERT 的既有合法语句行为逐字节不变（D2 仅为放宽 + 填充；当前报错的 `DEFAULT` 关键字语句转为填充语义）。本条同时构成对主规格 `insert-column-list-mapping`「恰为全列排列」requirement 的 MODIFIED（变更内 delta 已携带）。

## D3 写入类型一致门（ISS04）

新错误 `StorageError::ColumnTypeMismatch { column, expected, actual }`（`src/storage/error.rs`，镜像 `KeyTypeMismatch` thiserror 点名模式）。规则：值变体与列声明类型一致通过；`Value::Null` 豁免（NULL 性由 NOT NULL 门裁决）；日期族列 String 经 `coerce_datetime_write` 落类型后通过（门在 coerce 之后）；FLOAT 列接受 Int 值并**就地升格** `Value::Float(n as f64)`（写入序列化 tag 与 schema 一致）；其余跨类型拒绝。

**位置与优先级（保既有错误文本零变化）**：
- INSERT（`src/executor/insert.rs`）：既有序列 coerce → NOT NULL（:121）→ PK 键位门（:135）→ PK 重复预检（:151）→ UNIQUE 预检含 F1 守卫（:169）之后追加逐列一般门（PK 列与唯一列已被特有门覆盖，一般门对其仍校验但不改变优先级——特有门先行触发既有文本），任何写入前；升格就地改写行值。
- UPDATE（`src/executor/update.rs`）：既有 Step 1（KeyNotFound）→ NOT NULL（:92）→ PK 门 + rekey 预检（:111）→ coerce（:144）→ 读旧值 → UNIQUE 碰撞预检（:190）之后、serialize（Step 4）之前，对 SET 赋值列（单列）执行一般门（含升格改写 `new_value`）。
- 依据：单点覆盖 INSERT/UPDATE/dump/restore/import 全通道（import `csv_value` 按列类型产出、dump 按列类型生成类型化字面量已核实不误报）；执行器侧与 MS23 NOT NULL 同模式、零副作用；计划期通道需扩 PlanBuilder 全列类型且覆盖不了执行器直构路径。
- **否决方案**：计划期校验——需 PlanBuilder 全列类型通道且 `create_table` 之外的测试/工具构造路径不受覆盖；仅覆盖非键非唯一列——String PK 收 Int 值等同类损坏面继续漏网。

## D4 UPSERT 计划表示与解析面

`plan.rs` 新增（`PhysicalPlan::Upsert(UpsertNode)`）：

```rust
pub enum ConflictArbiter { All, Column(usize) }        // Column = PK 或唯一列位置
pub enum UpsertValueExpr { Literal(Value), Excluded(usize), Old(usize) }
pub struct UpsertAssignment { pub column: usize, pub expr: UpsertValueExpr }
pub enum ConflictAction {
    DoNothing,
    DoUpdate(Vec<UpsertAssignment>),
    Replace,                                            // REPLACE INTO：冲突行删除 + 重插
}
pub struct UpsertNode { table_name, values: Vec<Vec<Value>>, arbiter: ConflictArbiter, action: ConflictAction }
```

`build_plan` 的 `Statement::Insert` 分派停止以 `..` 丢弃字段，显式传 `on: &Option<OnInsert>` 与 `replace_into: bool` 给 `build_insert`：

- `on = None ∧ replace_into = true` → `arbiter=All, action=Replace`（SQLite 对齐：REPLACE 仲裁全部约束）。
- `replace_into ∧ on.is_some()`（语法可达的病态组合）→ 点名拒绝。
- `OnInsert::OnConflict(oc)`：
  - `conflict_target: None` → `All`；`Some(Columns(idents))` 长度=1 → 解析列位并校验**该列须有唯一性索引可仲裁**：INT 声明 PK 列（经既有 `primary_key_types` 通道判声明类型）或唯一索引列（经新增 `PlanBuilder.table_unique_columns: HashMap<String, Vec<usize>>` 加性通道——`register_table` 自 `TableMeta.unique_indexes` 注册，PlanBuilder 现无唯一列元数据）→ `Column(idx)`；否则 → SQLite 语义错误「ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint」——组合多列目标、非唯一列与非 INT 声明 PK（`to_key` 仅 Int 产键，无索引条目可仲裁）同文案，语义精确；`Some(OnConstraint(_))` → 点名拒绝（PostgreSQL 语法）。
  - `action: DoNothing` → `DoNothing`；`DoUpdate(du)`：`du.selection` 非空 → 点名拒绝「DO UPDATE WHERE is not supported」（v1 显式缺口）；assignments 逐个：`id.len()==1`、列解析 `ColumnNotFound` 同既有；值 `Expr::Value` → `Literal`、TypedString 日期族 → `Literal`（复用 build_update 臂模式）、NULL 标识 → `Literal(Null)`、`Identifier("DEFAULT")` → `Literal(列 DEFAULT 或 Null)`（D1 通道计划期可解析）、裸 `Identifier(col)` → `Old(idx)`（SQLite：DO UPDATE SET 裸列名指旧行值）、`CompoundIdentifier` 首段 `excluded` → `Excluded(idx)`、其余形态（BinaryOp/Function/嵌套等）→ 点名拒绝。
- `OnInsert::DuplicateKeyUpdate(_)` → 点名拒绝「ON DUPLICATE KEY UPDATE is not supported」。
- 拒绝文案载体：`PlanError::ParseError` 点名文本（`insert_count_error`/`UnsupportedConstraint` 既有先例）。
- UpsertNode.values 由 D2 全宽填充后传入。

## D5 UpsertExecutor 冲突仲裁与动作

新 `src/executor/upsert.rs::UpsertExecutor`（`PhysicalPlan::Upsert` 构造臂接线 pipeline）。逐行：

1. coerce（复用）→ NOT NULL（复用模式）→ PK 键位类型门（复用既有文本）。
2. 冲突仲裁搜索（该搜索兼作无冲突路径的既有预检）：
   - `All`：PK 值可键控（`to_key()`）→ `index_manager.search` 命中 → 冲突（PK 优先）；其后逐唯一列（NULL 跳过、非 Int 值 F1 守卫 `KeyTypeMismatch`、`search` 命中）→ 冲突。
   - `Column(idx)`：idx == `pk_index` → PK search；否则定位 `unique_indexes` 对应项（plan 期已保证存在）→ F1 守卫 + search。
   - 命中收集 `Vec<RowId>`（Replace 需全部冲突行去重；DoNothing/DoUpdate 用首个命中即定）。
3. 无冲突 → 既有 INSERT 序列（serialize → 数据页 → visibility → WAL Insert → record_version → PK 条目 → 唯一条目），count += 1。可观察行为与普通 INSERT 等价（InsertExecutor 保持不动，零回归锚点）。
4. `DoNothing` → 跳过该行（count 不加）。
5. `DoUpdate(assignments)`：读冲突行旧 tuple（deserialize）→ 逐 assignment 求值（`Literal` 经 coerce 落列类型 / `Excluded(i)` 取本行待插值（已 coerce）/ `Old(i)` 取旧值）→ final 行 = 旧值改赋值列 → final 行 NOT NULL（零副作用）→ 键位门（赋值触及 PK：Int/Null 门 + rekey 碰撞预检，镜像 update.rs:111-139）→ 唯一碰撞预检（触及唯一列：改值分支新键 search，镜像 update.rs:190-206；一般类型门对赋值列执行含升格，D3 规则）→ serialize final → 新版本（`with_next_version(冲突行 rid)`）写数据页 → visibility → WAL `Update{old_tuple, new_tuple}` → record_version → PK 维护四分支（非键 `update` / NULL `delete` / 同键 `update` / rekey `delete`+`insert`，update.rs:258-280 语义）→ 唯一索引四分支维护（update.rs:290-311 语义）→ count += 1。
6. `Replace`：对收集的全部冲突行去重后逐行执行既有删除语义（墓碑版本 `mark_deleted` + `with_next_version`、visibility、WAL `Delete`、record_version、PK 条目删除、唯一条目删除——delete.rs:105-166 镜像），随后执行步骤 3 的插入序列。受影响计数按**每个新行 +1**（一个 VALUES 行无论替换多少条冲突行均计 1，SQLite changes() 语义；D4 Replace 恒为 All 仲裁——`INSERT OR REPLACE`/`REPLACE INTO` 无目标概念，删除全部约束上的冲突行）。
7. `AffectedRows = 插入 + 更新 + 替换行数`（DO NOTHING 跳过行不计）。

事务回滚：record_version 混合 insert/update/delete 版本由既有 abort 清理通道处理（MS23 唯一条目修复含墓碑情形）；语句级 auto-commit 包裹语义不变。

**否决方案**：复用 UpdateExecutor/InsertExecutor 实例组合——单行跨执行器编排引入多次 MVCC 快照与 tx 记录交错，可观察行为更难锁定；共享 helper 重构 UpdateExecutor——重构面与其 MS23 精调路径耦合，违反最小变更面（helper 抽取属等价控制流选择，留给 Act，契约以「UPDATE 语句行为逐字节不变」为界）。

## D6 两态一致与恢复

无新 WAL 记录类型（Insert/Update/Delete 三型皆有既有重放通道）；DO UPDATE 产 Update 记录（old_tuple/new_tuple 字节与 UPDATE 语句同构）、Replace 产 Delete+Insert 记录序列。恢复重放（位置寻址 + 版本链重建 + 墓碑重放）与 `redo_count > 0` 索引重建通道不变。干净重开消费 catalog 持久化根（唯一索引自 MS23 通道加载）。两态一致由 e2e 测试见证（重开 + 崩溃恢复两态，复用 `constraint_enforcement_test` 恢复测试模式）。

## D7 dump/schema DEFAULT 渲染

`create_table_sql`（`src/cli/lifecycle.rs:577`）对 `CatalogColumnRow.default_value` 非 None 的列渲染 ` DEFAULT <literal>`（日期族经 `typed_datetime_literal` 包裹、其余 `sql_literal`；`DEFAULT NULL` 渲染为 `DEFAULT NULL`）。dump 与 `rtsql schema` 共用该生成器，一处覆盖。`CatalogColumnRow` 增字段后 dump/restore/import 通道的类型面不受影响（值按列类型产出已核实）。

## D8 测试布点

- 类型门矩阵（INSERT/UPDATE × 各列类型 × 升格/豁免/优先级）→ 新 `tests/write_type_conformance_test.rs`（lib API，`constraint_enforcement_test` helpers 模式）。
- 子集 INSERT / DEFAULT（填充矩阵 / DEFAULT 关键字 / 拒绝面 / 重启 / dump→restore 往返 / schema 渲染）→ 新 `tests/subset_insert_test.rs` + cli_test 增量。
- UPSERT / REPLACE（仲裁矩阵 / DO NOTHING / DO UPDATE 三形态 / 碰撞预检 / REPLACE 多冲突行 / 受影响计数 / 恢复两态 / 拒绝面）→ 新 `tests/upsert_test.rs`。
- catalog 列行 default 序列化往返 / 旧行兼容读单测 → `src/storage/catalog.rs` `#[cfg(test)]`（既有 `sample_col` 夹具校准）。
- 计划期拒绝矩阵 → `tests/planner_test.rs` 增量。
- 预期校准：`tests/insert_column_list_test.rs::partial_list_rejected_at_plan_time_no_panic`（:126）按子集新语义重写（防回归意图由未知列/重复列/无 panic 新用例承接）；`create_table_with_constraints` 5 元组签名波及的测试构造点机械适配。

## D9 回滚后墓碑行的索引条目还原（2026-09-26 扩围，replan）

`TransactionManager::abort_cleanup_versions`（`src/transaction/manager.rs:271-335`）当前对每个记录的 rid 读版本头，以 `index_manager.find_key_by_row_id(rid)` 定位本事务写入的索引条目：命中则按 `next_version()` 回退（`update`）或移除（`delete`），随后把该版本中性化（`mark_aborted`）。DELETE 与 REPLACE 的删除段记录的是**墓碑 slot**（`delete.rs:147-151`、`upsert.rs:581-587`），墓碑从不入索引 → `find_key_by_row_id` 恒为 `None` → 整个条目还原被跳过（`manager.rs:292-314` 注释已自认「墓碑 slot 不入唯一索引……与 PK 同型」）。可观察后果：回滚后行在全表扫描可见（墓碑已中性化），但 PK 等值点查漏行，且其唯一值被释放后可再次插入——两条存活行共享同一唯一值，`UNIQUE` 约束静默失效。

键的来源：删除时条目已从索引移除，`IndexManager::delete`（`index_manager.rs:272-293`）同时清掉 `row_to_key` 反向映射，故前驱版本的键无法从索引反查，必须自其数据页 slot 的元组派生。

- **处理分趟**（确定性要求）：本事务的记录集合是 `HashSet<RowId>`（`record_version` 聚合），迭代顺序不确定，而 REPLACE 形态下「被删行前驱」与「替换行新版本」共享同一键。若逐个 rid 单趟处理，替换行的条目移除可能抹除墓碑行的还原结果。因此先按「该 rid 当前是否持有索引条目」把集合**一次性划分为两趟**（划分在任何写入前完成）：A 趟 = 有条目者（INSERT 新行 / UPDATE 新版本 / REPLACE 的替换行），执行既有回退或移除逻辑；B 趟 = 无条目者（墓碑），执行还原。B 趟在后，终态与集合顺序无关。
- **存活版本回溯**：从墓碑的 `next_version()` 链向前，跳过 `create_tx_id() == tx_id` 的版本（本事务自身创建、即将被中性化），取首个 `create_tx_id() != tx_id` 的版本为还原目标；链上无此类版本（如「插入后同事务删除」的新行）则无还原。回溯以迭代上限与 `SlotNotFound` 为终止条件，`read_tuple_from_data_page` 单次页读同时给出版本头与元组字节。
- **取键与重建**：复用 `wal/recovery.rs:227` 的 `extract_index_keys`（一次反序列化兼提 PK 键与各唯一列键，与恢复期索引重建同源 guaranteeing 键派生一致），提为 `pub(crate)` 并从 `wal/mod.rs` 暴露模块；还原时按 `pk_index` 与 `unique_indexes` 逐项 `insert(key, 存活 rid)`（条目在删除时已被移除，故为插入而非更新）。反序列化失败或 slot 缺失 → 跳过该行还原，不报错（与 `delete.rs` / `insert.rs` 的 SlotNotFound 容忍同型）。
- **不改动的面**：`delete.rs` / `insert.rs` / `update.rs` / `upsert.rs` 的写入与索引维护序列零改动；提交路径、恢复重放、`redo_count > 0` 索引重建通道零改动；`VersionHeader` 布局与语义零改动；`abort` 的 WAL `AbortTxn` 记录时序不变。
- **修复后 MS24 写面语义**：REPLACE 失败语句（先删后校验）经语句级 abort 后条目完整还原，失败写语句不再留下索引差异——`sql-write-surface` 的零副作用语义随之在三个动作上成立，README 写面段表述经复核后可保持（若仍有未覆盖面则按实测收口）。

**替代方案（否决）**：删除时延迟移除索引条目、改到 commit 时清理——改变 DELETE 的运行期可见索引状态（点查将命中已删除行），违反既有 `DELETE` 语义与 spec `sql-constraint-enforcement` R3「DELETE 后同值可重插」的运行期面，且属运行期路径大改；`record_version` 扩展为携带删除前的键集——需改 4 个执行器的记录调用点与 `tx_versions` 数据形状（MS07-T04 聚合结构），变更面大于单点还原且同样解决不了「目标版本回溯」；仅消除 MS24 新触发面（REPLACE 把类型门前置到删除之前）——只掩盖 auto-commit 触发面，显式事务 `BEGIN; DELETE; ROLLBACK` 的 PK 漏行与唯一值释放仍在，且 DO NOTHING 对类型非法值的跳过语义会随之前置而收紧（偏离 SQLite）；在 `transaction` 层另写一份取键 helper——与恢复期 `extract_index_keys` 的键派生逻辑重复且可能漂移，违反「优先复用项目已有实现」。

## D10 测试布点（2026-09-26 扩围）

- `insert_row` PK 重复预检（父 Cycle 阻塞发现 1）→ `tests/upsert_test.rs` 补两例：显式唯一列目标 + PK 冲突的 DO NOTHING 臂与 DO UPDATE 臂（`DuplicateKey`、行数不变、原行点查仍命中）。
- 回滚索引还原（D9）→ `tests/explicit_tx_test.rs` 增量：DELETE 回滚点查可达 / DELETE 回滚唯一值仍占用 / REPLACE 回滚原行完整复现 / 同事务 update→delete 回滚指向更新前版本 / 失败 REPLACE 语句回滚无残留 / 回滚后干净重开两态一致。
- README 写面段复核 → 修复后按实测确认零副作用表述是否成立；成立则保持，不成立则按实测收口措辞（不新增未来承诺）。

## Acceptance 映射

R1（子集 INSERT 与 DEFAULT）→ D1/D2/D7 → Iteration 000 任务 1.x；R2（类型门）→ D3 → 任务 1.x；R3（UPSERT 语义）→ D4/D5/D6 → Iteration 001 任务 2.x + replan 补齐 2.7；R4（冲突目标与拒绝面）→ D4/D5 → 2.1/2.2；R5（零回归）→ D2/D3/D5 的不变量面 + 全量回归 → 1.x/2.x 回归锚。2026-09-26 扩围：R6（回滚后墓碑行索引条目还原，`mvcc-tombstone-visibility` ADDED）→ D9 → 任务 2.9/2.10；R3'（`sql-constraint-enforcement` R3 回滚子句去歧义）→ D9 → 2.9。逐条 RTM 见 tasks.md。
