# Iteration 001 / Cycle 000：fixed32 字符串索引生命周期

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks：T4、T5。
- Acceptance gaps：None。
- Repair items：None。
- Inherited scope：R2 的存储层建立与回填，R4 的 DML、回滚和失败写入，R5 的干净重开、未决 WAL 恢复和旧库整数数据，R6 的现有 38/36 字节判界。R1 的生产格式选择已经关闭，本轮只回归现有树。
- Excluded scope：T6 的 SQL `CREATE INDEX` 字符串放行、等值路由和 EXPLAIN；T7 的取消、超时、内存库、只读、加密和附加库组合；T8 的全量、用户文档和全局格式门。ordered-variable、prefix-restart、文件头版本、条目宽度、`Value::to_key` 的类型范围、字符串主键和唯一约束。

**Objective**

在不改变现有 B-Tree 条目宽度的前提下，让二级索引能够保存完整转义后不超过 32 字节的字符串，并在回填、DML、回滚、失败写入、重开和恢复之后与存活行一致。超出该长度的合法字符串仍然写入行，且不进入树。SQL 声明和查询路由留在下一轮。

**Background**

保留现有 32 字节固定键，采用 fixed32。两种候选的热读没有同时守住已批准的 5% 吞吐线；页数和写、冷读收益不能单独切换生产格式。不采用 ordered-variable 或 prefix-restart，不降低筛选门槛，不重跑 Criterion。

该选择来自本次执行 goal，并与 Iteration 000 的 accepted Review 一致。000 的 Cycle 文件不改写。`tasks.md` 里 T1–T3 改为完成，依据就是该 accepted Review；审查里「当时没有改 tasks.md」的非阻塞发现保留。

**Investigation Facts**

- Starting point：000 的 Act Response 记录产品代码未改，只新增 `tests/support/index_key_encoding.rs`、`tests/index_key_encoding_evaluation_test.rs`、`benches/index_key_encoding_bench.rs` 和 `Cargo.toml` 的 bench 注册。accepted Review 核对 `src/` 不在这四个路径的 diff 里，并沿用测试 7 passed 与一次 Criterion。初筛未通过。规划时的 `storage::page_format::key::tests` 为 6 passed / 0 failed，`secondary_index_test` 为 27 passed / 0 failed；000 没有改这些产品路径，本轮展开前不重跑它们。工作区另有更早的未提交差异，本轮不得清理。
- 布局事实：`Key` 是 32 字节，`Eq`/`Ord` 比较 `full_data()`。`Key::deserialize` 会按最后一个非零字节推断 `len`，该推断不参与比较。叶条目为 `MAX_KEY_LEN + RowId::SIZE`（32+6=38），内部分隔条目为 32+4=36。页类型 `LEAF_NODE = 0x01`、`INTERNAL_NODE = 0x02`。`integrity::check_tree_page` 用这两个宽度判界。`file_header::FORMAT_VERSION` 为 1。
- 当前字符串行为：`Value::to_key` 只接受 `Value::Int`，返回 `i64::to_be_bytes()` 的 `Key`。`ColumnType` 有两套：执行器 `executor::value::ColumnType::String` 无长度；表模式 `page_format::ColumnType::String(u16)`。`TableMeta.columns` 使用后者。`build_create_index` 在修饰守卫之后只放行 `page_format::ColumnType::Int`，VARCHAR 样本由 `create_index_on_non_int_column_rejected` 拒绝。`extract_secondary_index_equality` 仍调用 `to_key`，所以字符串等值目前走扫描。
- 评估原型中的 fixed32 字符串编码位于 `tests/support/index_key_encoding.rs::encode_ordered_str`，再由 `fixed32_key_bytes` 在长度超过 32 时返回不支持。生产代码不能引用该测试模块。空字符串的 32 字节全零与整数 0 的树字节相同；000 已记录这是分域边界，同域没有碰撞。本轮不修改 `Key` 的比较或反序列化来消除它。

Current-State Evidence 与关键路径：

| 符号 | 已读到的事实与本轮用途 |
|---|---|
| `Value::to_key` | Int 专用。PK、唯一索引和冲突仲裁继续调用它。二级路径改走新入口后，这个函数本体不扩大到 String。 |
| `IndexManager::{insert,delete_exact,search,search_all,scan_all}` | 调用方传入的字节经 `Key::new` 后按 32 字节比较。`insert` 把同一字节切片写入 `row_to_key`。`delete_exact` 用该切片做精确比较，并只删除指名的 `(key, rid)`。`from_root` 把 `row_to_key` 建成空表；重开后的删除仍靠调用方再次传入派生字节。本轮不改这些函数。 |
| `BTree::insert` / `LeafNode::insert` | 条目固定 38 字节，允许重复键。分裂沿现有路径。本轮用它证明字符串键不需要新条目格式。 |
| `CreateIndexExecutor` / `backfill_tree` | 公开执行器调用私有回填。回填在页闭包外装配 overflow，经 `deserialize_row` 和 DEFAULT overlay 取列值，再 `to_key`。失败时释放未登记树。测试从集成测试构造 `PhysicalPlan::CreateIndex`，不打开计划期类型门。 |
| `insert.rs`、`update.rs`、`delete.rs`、`upsert.rs` 的二级循环 | 目前都对二级列调用 `to_key`。更新和删除二级条目使用 `delete_exact`，不用 `delete` 或 `update`。唯一列和 PK 循环保持 `to_key`。 |
| `upsert.rs::arbitrate` | 只搜索 PK 和唯一索引。不改。 |
| `table_manager.rs::alter_drop_column` | 改写存活行时，用 `to_key` 收集旧二级键并给新行插入。索引列、PK、唯一列和最后一列的删除仍由既有守卫拒绝。未索引列的删除必须改用二级派生，否则字符串条目会留在旧 rid 上。 |
| `table_manager.rs` 的 TRUNCATE 段 | 为二级索引换上空树，不从行重新派生。不要为字符串加第二套回填。 |
| `recovery.rs::extract_index_keys` | 一次反序列化产出 PK、唯一和二级三组键。只改二级那一组。`rebuild_pk_indexes`、`read_prev_index_keys` 和 `restore_tombstone_index_entries` 已经调用它，不要在事务管理器里再写一套编码。 |
| `register_secondary_index` | 先写 catalog 段，再把 `SecondaryIndexMeta { name, column_index, index }` 换入 `TableMeta`，并挂 `CatalogRootSlot::Secondary`。字符串索引没有新的目录字段。 |
| `tests/secondary_index_test.rs` | 整数索引的建、重开、崩溃和 VARCHAR 拒绝已在这里。27 个测试是 T5 之后要复跑的受影响回归。不要把新字符串用例加进这个文件，以免把「仍拒绝 SQL 建字符串索引」和本轮存储层建索引混成一次计数变化。 |
| `tests/legacy_compat_test.rs::legacy_db_opens_queries_and_writes` | `tests/fixtures/legacy/legacy_g1.db` 复制到临时目录后可打开。`users` 的三行在该测试里呈现为整数、字符串、浮点。本轮在副本上追加字符串索引，不修改样本文件。 |
| `storage::integrity::check_integrity` | 公开只读检查。字符串树仍应得到 0 个 error。 |

**Implementation Guidance**

生产入口是 `Value::secondary_index_key(&self, column_type: &page_format::ColumnType) -> Option<Key>`，放在 `src/executor/value.rs`。所有二级调用点都调用它，禁止在各调用点手写转义。

编码与 `encode_ordered_str` 的字节一致：非零字节原样写入；输入字节 `00` 写成 `00 FF`；结果末尾追加 `00 00`。`ColumnType::String(_)` 且值是 `Value::String` 时，编码长度小于或等于 32 才 `Key::new(&encoded)`，否则 `None`。`String(u16)` 的声明长度不参与这个判断，也不拒绝行写入。`ColumnType::Int` 的结果必须与 `to_key` 相同，包括非 Int 值返回 `None`。其他列类型返回 `None`。`Value::Null` 返回 `None`。

调用 `IndexManager` 时传入 `key.as_bytes()`。这个长度来自 `Key::new` 的输入，包含结尾的 `00 00`。不要传入 `full_data()`，也不要先 `deserialize` 再取 `as_bytes()`。整数二级索引今天传入的是 8 字节大端，不是 32 字节填充；字符串沿用同一调用形状。

30 个非零字节的编码长度是 32，入树。31 个非零字节的编码长度是 33，不入树。每个原始 `00` 额外占一个字节。空串编码为 `00 00`，`"a"` 为 `61 00 00`，`"\0"` 为 `00 FF 00 00`。

T4 先放最小骨架：Int 委托 `to_key`，String 固定返回 `None`。字符串测试因此在运行期失败，不用编译失败充当 RED。骨架通过后实现 String 分支。T5 先写会看到空树的集成测试，确认 RED，再替换下列二级 `to_key`。替换时把列类型从 `table_meta.columns[column_index].1` 或 `extract_index_keys` 已建的 `schema[i]` 传入。

**Task Definitions**

### T4：现有树上的 fixed32 字符串键

- Requirement/Scenario：R1 的生产格式回归；R2/S05 的可容纳边界；R6 的现有条目判界。
- Depends on：None。000 已 accepted，布局选择已写入本 Cycle。
- Targets：`src/executor/value.rs::Value::secondary_index_key`；`src/executor/value.rs` 的测试；`src/storage/btree/index_manager.rs` 的测试模块。
- Current behavior：没有二级字符串派生。`IndexManager` 能保存任意不超过 32 字节的 `Key`，但产品路径不会为字符串产生这种键。叶 38 字节、内部 36 字节。
- Required behavior：上述编码生效。Int 分支与 `to_key` 的 `as_bytes()` 一致，样本至少覆盖 0、-1、`i64::MIN` 和 `i64::MAX`。字符串样本覆盖空串、共享前缀的两个不同值、多字节 UTF-8、尾部零字节、30 字节非零入树、31 字节非零返回 `None`。不同字符串的 `full_data()` 不同。向 `make_manager` 插入 200 个互异的可容纳键后，根页第一个字节是 `0x02`；其中一个子叶的槽长度是 38，根页上一枚分隔槽长度是 36；`check_tree_page` 没有 violation。`search_all` 找回全部 rid。同一键插入两个 rid 时，`delete_exact` 只删除被点名的一个。
- Preserve：`Value::to_key`、`Key` 的比较和反序列化、`node.rs` 的条目宽度、`integrity::check_tree_page` 的宽度、`FORMAT_VERSION`、`IndexManager` 的写单元。评估原型文件保留，生产代码不引用它们。
- Forbidden：修改 `node.rs`、`integrity.rs`、`file_header.rs` 或 B-Tree 算法；把 ordered-variable 或 prefix-restart 编译进 `src/`；重跑 Criterion；为了空串全零去改 `Key`。
- First test：在骨架上新增 `secondary_index_key_preserves_int_bytes` 与 `string_secondary_key_preserves_full_value_within_32`。前者随骨架通过。后者预期运行期失败，再实现 String 分支。然后新增 `string_key_uses_existing_tree_entry_width`，编码未完成时查找失败，完成后通过。
- GREEN condition：三个测试通过；31 字节非零得到 `None` 而不是截断后的 `Key`；页类型和槽长度符合现有常量。
- Verification：依次执行下面三条，每条都是 1 passed / 0 failed。`key.rs` 未改时不重跑既有 6 个 Key 测试。

```sh
cargo test --offline --lib secondary_index_key_preserves_int_bytes -- --test-threads=1
cargo test --offline --lib string_secondary_key_preserves_full_value_within_32 -- --test-threads=1
cargo test --offline --lib string_key_uses_existing_tree_entry_width -- --test-threads=1
```

- Stop when：可容纳字符串必须修改条目宽度、文件头或 `Key` 比较才能查找；或者空串全零在现有整数 0 的比较路径上失败。返回 Plan，不改格式门槛。

### T5：二级索引的回填与生命周期

- Requirement/Scenario：R2/S03 的存量回填；R4/S06、S07、S08；R5/S10、S11 和旧库；R6 的现有完整性检查。S04 的路由和 S09 的取消不属于本任务。
- Depends on：T4。
- Targets：`create_index.rs::backfill_tree`；`insert.rs`、`update.rs`、`delete.rs` 的二级循环；`upsert.rs` 中插入、改行和 `delete_conflict_row` 的三处二级循环；`table_manager.rs::alter_drop_column` 的旧二级键收集与新行插入；`recovery.rs::extract_index_keys` 的二级分支。新集成测试 `tests/string_secondary_index_test.rs`。
- Current behavior：这些二级位置都调用 `to_key`，字符串因此没有条目。`CREATE INDEX` 对非 INT 列在计划期拒绝。崩溃测试的现有形状是 `buffer_pool.flush_all`、写入、`wal_buffer.shutdown`、丢弃 `Database` 且不 `close`，再 `Database::open`。
- Required behavior：上述位置对二级列调用 `secondary_index_key`，列类型取自表模式。PK 与唯一列仍调用 `to_key`。回填、插入、更新、删除、UPSERT/REPLACE 使用同一派生。可容纳的非 NULL 字符串入树，NULL 和超容量字符串不入树。重复值删除一行时，另一行的条目还在。字符串改为另一字符串、改为 NULL、从 NULL 改回字符串后，`search_all` 与存活行一致。容量内与容量外的双向改值之后同样一致：容量外不在 `scan_all` 中，改回容量内后重新出现。显式事务中的插入、改值和删除在 `ROLLBACK` 后，树和 `SELECT *` 回到事务前。重复主键在写入之前被拒绝，已有字符串条目和行都不变。`ALTER TABLE ... DROP COLUMN` 删除另一未索引列之后，字符串条目的 rid 属于删除后的存活行。干净 `close` 后重开，以及不 `close` 的未决 WAL 重开，已提交行的 `search_all` 与重开前一致。`check_integrity` 的 error 数为 0。`legacy_g1.db` 的副本打开后，`users` 原有三行仍在；对其中的字符串列经 `CreateIndexExecutor` 建索引后，短名字能被 `search_all` 找到，按原整数主键查询仍返回原行。
- Preserve：计划期非 INT 拒绝，包括 `create_index_on_non_int_column_rejected`。`extract_secondary_index_equality`。回填的 overflow 装配、DEFAULT overlay、`narrowing.ensure_readable` 和失败释放。二级维护继续只用 `delete_exact`。TRUNCATE 的空树替换。目录里的二级字段仍是名称、列号和根页。
- Forbidden：放行 SQL `CREATE INDEX` 的字符串类型；让 `SELECT` 或 `EXPLAIN` 走字符串索引；修改 `query.rs` 的路由；增加摘要键、截断或溢出索引页；删除或改写 `tests/fixtures/legacy/`；清理无关工作区差异；运行全量；处理 ISS36。
- First test：新增 `string_index_backfills_duplicates_null_and_utf8`。表由 `Database::execute_sql` 建立并写入重复值、空串、多字节 UTF-8、尾部零字节、共享前缀和 NULL。索引由 `CreateIndexExecutor::new(PhysicalPlan::CreateIndex(...), Arc<Database>)` 建立。断言时用 `IndexManager::search_all` / `scan_all` 看条目，用 `execute_sql` 的 `SELECT *` 看行。接线前可容纳值不在树中，这是运行期 RED。
- GREEN condition：该测试以及下面 Verification 列出的新测试全部通过。超容量行出现在 `SELECT *`，不出现在 `scan_all`。SQL `CREATE INDEX` 对字符串列仍然点名拒绝。`secondary_index_test` 为 27 passed / 0 failed。
- Verification：先单独运行新测试文件，观察 RED，再改调用点。完成后执行 `cargo test --offline --test string_secondary_index_test -- --test-threads=1` 和 `cargo test --offline --test secondary_index_test -- --test-threads=1`。新文件 0 failed；既有文件 27 passed / 0 failed，其中 VARCHAR 建索引仍被拒绝。不跑 `--bench`，不跑无目标过滤的全量。
- Stop when：生命周期只有改计划期类型门或查询路由才能成立；超容量字符串必须被拒绝写入或截断才能通过测试；旧库打开必须改 `FORMAT_VERSION`。返回 Plan。

**Invariants**

- 叶条目 38 字节，内部分隔条目 36 字节，页类型 0x01 与 0x02，`FORMAT_VERSION` 保持 1。
- 二级字符串只在完整转义长度不超过 32 时入树。超容量值保留整行，不截断，不建摘要键。
- NULL 不入二级索引。重复键用 `delete_exact(key, rid)`。
- `Value::to_key` 仍只接受 Int。PK 与唯一索引的字节不变。
- 空串在字符串索引中是 32 字节全零。这与整数 0 分属不同树。不为此修改 `Key`。
- 新测试各自使用临时目录。遗留样本只作复制源。

**Non-goals**

SQL 字符串建索引、等值路由、EXPLAIN、范围谓词、参数路由、取消和超时、内存库、只读、加密、附加库、用户文档、全局 `cargo fmt` 与 clippy、全量测试、ISS36、评估原型的删除，以及 ordered-variable 与 prefix-restart 的生产实现。没有需要人工操作的验证项。

**Acceptance**

- A1 / R1、R6 / T4：Int 派生字节等于 `to_key`；可容纳字符串的编码、顺序和互异性成立；31 字节非零不产生键。
- A2 / R6 / T4：真实树上的插入、多层根、重复键 `delete_exact`、38/36 槽长度和 `check_tree_page` 通过。产品树算法和文件头无 diff。
- A3 / R2 / T5：执行器回填覆盖重复值、空串、UTF-8、尾部零和 NULL。`SELECT *` 与条目一致。SQL 建字符串索引仍被拒绝。
- A4 / R2、R5 / T5：超容量行可写、可从 `SELECT *` 读回，且不在 `scan_all` 中。容量两侧更新后，树与全表行一致。
- A5 / R4 / T5：改值、NULL 转换、删除重复行、ROLLBACK 和失败的主键插入之后，条目与已提交存活行一致。
- A6 / R4 / T5：删除未索引列之后，字符串条目仍对应存活行。
- A7 / R5 / T5：干净重开与不 close 的 WAL 重开，已提交行和 `search_all` 一致，完整性 error 数为 0。
- A8 / R5 / T5：`legacy_g1.db` 副本上的 `users` 原行还在，短字符串可入现有树，整数查询仍返回原行。
- A9：`secondary_index_test` 27 passed / 0 failed。

**Verification**

T4 与 T5 的命令见各自 Task Definition。判定直接看测试断言、`search_all`/`scan_all` 的 rid 集合、`SELECT *` 的行，以及 `check_integrity` 的 error 数。字符串查询在本轮不要求出现索引节点。

新文件中的测试名：

- `string_index_backfills_duplicates_null_and_utf8`
- `string_index_over_capacity_row_stays_and_is_absent_from_tree`
- `string_index_rekey_null_rollback_and_failed_write`
- `string_index_survives_drop_of_unindexed_column`
- `string_index_clean_reopen_and_pending_wal_agree`
- `legacy_db_accepts_string_index_without_losing_int_data`

`string_index_over_capacity_row_stays_and_is_absent_from_tree` 同时覆盖从容量内改到容量外，以及从容量外改回容量内。`legacy_db_accepts_string_index_without_losing_int_data` 使用 `users` 的实际字符串列序号；现有测试把该行显示为整数、字符串、浮点。`get_table` 的列类型若不是字符串，停止并返回 Plan。

既有 Key 6 passed 只在 `key.rs` 被改动时失效；本任务禁止改它，因此不重跑。000 的 Criterion 结论继续有效，不重跑。全量留在 T8，按 M19 分批，每批 `--test-threads=1`，本 Cycle 不消耗这次预算。

**RTM**

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 格式回归 | T4 | `string_key_uses_existing_tree_entry_width` | Covered |
| R2 回填与容量 | T4、T5 | `string_index_backfills_duplicates_null_and_utf8` | Covered |
| R4 生命周期 | T5 | `string_index_rekey_null_rollback_and_failed_write` | Covered |
| R5 重开、恢复、旧库 | T5 | `string_index_clean_reopen_and_pending_wal_agree` | Covered |
| R6 现有判界 | T4、T5 | `string_key_uses_existing_tree_entry_width` | Covered |

R3 的等值路由属于 T6，本 Cycle 不实施，不把未路由写成路由通过。取消和超时属于 T7。没有 Simplified。

**Gate 2 Readiness**

PASS。没有 BLOCKED，没有 SKIPPED。编码、32 字节容量、`None` 与行写入的错误语义、调用点和验证入口已经闭合。批准来自本次执行 goal，不是跳过 Gate 2。

**Persisted Evidence**

- Mode: none

测试命令和通过计数写入 Act Response 即可。定向测试可以重跑，没有不可再生的现场，也不需要保存整份输出才能决定下一轮。不创建 evidence 目录。

**Risks and Notes**

- 传入 `full_data()` 会使 `row_to_key` 的长度与 `as_bytes()` 不一致，`delete_exact` 的映射比较会错过。发现这种失败时改调用字节，不改 `Key::deserialize`。
- `secondary_index_key` 的参数是表模式上的 `page_format::ColumnType`。`executor::ColumnType::String` 没有长度参数，不能传给这个函数。
- 工作区里 000 的原型和更早的未提交差异都保留。ISS36 不在本轮处理，不运行全局 fmt 或 clippy 门，也不为它修改无关文件。
- 本 Cycle 不登记 Issue。范围外的实质缺陷只在 Act Response 里报告为候选。

## Act Response

- Status: reported

**实际改动**

- `src/executor/value.rs`：`Value::secondary_index_key`。`Int` 委托 `to_key`。`String` 由 `fixed32_string_key` 编码：非零字节原样写入，输入 `00` 写成 `00 FF`，末尾追加 `00 00`；完整长度超过 32 返回 `None`。声明长度不参与判断。其余列类型和 `Null` 返回 `None`。`to_key` 仍只接受 `Int`。
- `src/executor/create_index.rs`：`backfill_tree` 对二级列调用 `secondary_index_key`，插入字节用 `as_bytes()`。
- `src/executor/insert.rs`、`update.rs`、`delete.rs`：二级循环改用 `secondary_index_key`。主键和唯一列仍调用 `to_key`。更新继续用 `delete_exact`。
- `src/executor/upsert.rs`：插入、`apply_do_update`、`delete_conflict_row` 三处二级循环同样改用 `secondary_index_key`。
- `src/storage/data/table_manager.rs`：`alter_drop_column` 收集旧二级键和插入新行时改用 `secondary_index_key`。
- `src/wal/recovery.rs`：`extract_index_keys` 的二级分支改用 `secondary_index_key`。主键和唯一分支仍调用 `to_key`。
- `src/storage/btree/index_manager.rs`：只在测试模块增加 `string_key_uses_existing_tree_entry_width`。写路径未改。
- `tests/string_secondary_index_test.rs`：新增六个生命周期测试。索引由 `CreateIndexExecutor` 建立。

未改 `node.rs`、`integrity.rs`、`file_header.rs`、`Key` 的比较和反序列化、`query.rs`，也没有把评估原型编进 `src/`。

**与计划的偏差**

原文长度大于 30 字节时，`fixed32_string_key` 在分配编码缓冲区之前返回 `None`。最短编码长度是原文字节数加 2，这条路径和先编码再拒绝的结果相同。30 字节非零仍入树，31 字节非零仍返回 `None`。

`string_key_uses_existing_tree_entry_width` 在字符串分支完成之后加入。它的第一次可编译运行已经通过。在此之前，`string_secondary_key_preserves_full_value_within_32` 已在骨架上因空串得不到键而运行期失败。

UPSERT 与 `REPLACE` 的断言放在 `string_index_rekey_null_rollback_and_failed_write` 内，没有增加 Verification 之外的测试名。

**自检**

Gate 3：T4 的整数字节、可容纳字符串、31 字节 `None`、根页 `0x02`、叶槽 38、分隔槽 36、`check_tree_page`、`search_all` 和 `delete_exact` 均由对应测试覆盖。T5 的回填、容量两侧更新、改值、NULL、删除重复行、回滚、失败主键插入、UPSERT、`REPLACE`、删除未索引列、干净重开、未决 WAL、旧库和 SQL 拒绝均由新测试覆盖。主键与唯一路径仍调用 `to_key`。

计划范围内没有未解决的 Critical 或 Important。`src/storage/data/table_manager.rs` 第 729 行附近原有一处 `rustfmt` 差异，本轮没有改那一行，也没有把它算成新的格式问题。

**验证**

T4 骨架阶段：

```text
cargo test --offline --lib secondary_index_key_preserves_int_bytes -- --test-threads=1
1 passed; 0 failed
```

```text
cargo test --offline --lib string_secondary_key_preserves_full_value_within_32 -- --test-threads=1
0 passed; 1 failed
```

失败点是 `empty string fits`：骨架对字符串返回 `None`。

字符串分支完成，以及随后补上超长原文提前返回之后，定向结果为：

```text
cargo test --offline --lib secondary_index_key_preserves_int_bytes -- --test-threads=1
1 passed; 0 failed
```

```text
cargo test --offline --lib string_secondary_key_preserves_full_value_within_32 -- --test-threads=1
1 passed; 0 failed
```

```text
cargo test --offline --lib string_key_uses_existing_tree_entry_width -- --test-threads=1
1 passed; 0 failed
```

整数测试和树宽测试是在提前返回之前的完成代码上通过的。提前返回只改变长于 30 字节的字符串，这两条测试不覆盖该输入；30/31 字节边界由重新运行的字符串测试覆盖。

T5 接线前：

```text
cargo test --offline --test string_secondary_index_test -- --test-threads=1
0 passed; 6 failed
```

六个测试都停在可容纳字符串的 `search_all` 结果为 0。SQL 建字符串索引的拒绝和旧库原行断言已经走到后面的查找失败之前。

接线后：

```text
cargo test --offline --test string_secondary_index_test -- --test-threads=1
6 passed; 0 failed
```

```text
cargo test --offline --test secondary_index_test -- --test-threads=1
27 passed; 0 failed
```

其中 `create_index_on_non_int_column_rejected` 通过。没有运行 `--bench`、无过滤全量，也没有运行全局 `fmt` 或 clippy 门。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**未解决问题**

None。本轮没有发现当前 Cycle 范围外的实质缺陷。T6–T8 未执行。Review Result 仍为 pending。

## Plan Review

- Review Result: accepted

**Findings**

阻塞 Acceptance：没有。

非阻塞，共 2 条：

1. Act 偏离，测试顺序。`string_key_uses_existing_tree_entry_width` 是在字符串分支已经能编译之后加入的。Act Response 写明它的第一次可编译运行已经通过，没有先看到「编码未完成时查找失败」。同一任务的 `string_secondary_key_preserves_full_value_within_32` 已在骨架上因空串得不到键而失败。树宽测试现在检查根页 `0x02`、分隔槽 36、叶槽 38、`check_tree_page` 无 violation、200 个键的 `search_all`，以及同一键两个 rid 时 `delete_exact` 只删除被点名的一个。Act 报告该测试 1 passed / 0 failed。A2 没有缺口。
2. Act 偏离，等价提前返回。`fixed32_string_key` 在原文字节数大于 30 时，于分配编码缓冲区之前返回 `None`。最短编码长度是原文字节数加结尾 `00 00`，所以这条路径和先编码再按 32 字节拒绝的结果相同。30 个非零字节仍得到长度 32 的键，31 个非零字节仍是 `None`。原文里的 `00` 不会走这条提前返回，仍由循环和结尾长度检查拒绝超长结果。

**Evidence**

本轮增量是未暂存 diff，以及未跟踪的 `tests/string_secondary_index_test.rs`。000 的评估原型、`node.rs`、`integrity.rs`、`src/parser/planner/query.rs` 和 `table_manager.rs`、`recovery.rs` 里更早的已暂存差异分开看，不计入本轮。

已读实现与测试：

- `Value::secondary_index_key`、`fixed32_string_key`、`Value::to_key`。Int 委托 `to_key`。字符串编码与 `tests/support/index_key_encoding.rs` 的 `encode_ordered_str` 字节规则一致：非零字节原样，输入 `00` 写成 `00 FF`，末尾 `00 00`，超过 32 返回 `None`。`src/` 不引用评估原型。
- 二级调用点都传入 `as_bytes()`：`create_index.rs::backfill_tree`，`insert.rs`、`update.rs`、`delete.rs` 的二级循环，`upsert.rs` 的插入、`apply_do_update` 和 `delete_conflict_row`，`table_manager.rs::alter_drop_column` 的旧键收集与新行插入，`recovery.rs::extract_index_keys` 的二级分支。更新和删列继续用 `delete_exact`。主键和唯一列仍调用 `to_key`。
- `ddl_dml.rs` 的 `CREATE INDEX` 类型门仍只放行 `ColumnType::Int`。`extract_secondary_index_equality` 仍调用 `to_key`。`node.rs`、`integrity.rs`、`file_header.rs`、`key.rs` 没有本轮未暂存 diff。
- 新集成测试六个名字与 Verification 清单一致。回填覆盖重复值、空串、UTF-8、尾部零和 NULL；容量两侧更新、改值、NULL、删除重复行、ROLLBACK、失败主键插入、UPSERT 和 REPLACE、删除未索引列、干净重开、不 `close` 的 WAL 重开、`legacy_g1.db` 副本都在断言里。SQL `CREATE INDEX` 对字符串列仍期望被拒绝。

验证沿用 Act Response，不重跑。沿用来源是本 Cycle 的 Act Response：T4 三条最终各为 1 passed / 0 failed，其中字符串测试在骨架上曾为 0 passed / 1 failed；T5 接线前 0 passed / 6 failed，接线后 `string_secondary_index_test` 6 passed / 0 failed，`secondary_index_test` 27 passed / 0 failed。代码与这些断言一致，结论产生后没有再改这些文件。未跑全量，未跑 Criterion，未跑全局 fmt 或 clippy。Persisted Evidence 为 none，不因缺少 evidence 目录记问题。

**Follow-up Decision**

A1–A9 都有对应代码和已报告的通过结果。两条发现不改变可观察合同，也不需要新的执行依据。不在当前 Cycle 返修，不创建 rework 或 replan。Iteration Plan 保持不变。002 按本批约束不展开。

**Next Cycle**

None

**Next Iteration**

Iteration 002（T6，SQL 声明与等值查询）仍只写在本变更 `tasks.md` 的 Iteration Plan 里。本次审查没有创建它的目录。
