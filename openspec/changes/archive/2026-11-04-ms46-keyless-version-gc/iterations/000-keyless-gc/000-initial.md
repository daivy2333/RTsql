# Iteration 000 / Cycle 000: 无键旧版本可回收且带键合同仍在

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 场景 S1–S11；`specs/keyless-version-gc/spec.md` 的三条 Requirement；`design.md` 的八条决定。带键 GC 的现有谓词和 `delete_tuple_from_data_page` 继续有效。
- Excluded scope: 快照可见性、空闲页提升规则、索引格式、WAL、刷盘、SQL 或后台 GC、MS47、全量回归、`openspec/specs/` 的权威正文。不改 `tests/gc_test.rs` 里三个既有用例的断言。不改 `gc_releases_oversized_version_chain`。

**Objective**

`TableMeta::gc_table` 在一次调用里回收无键链上已提交的非链头版本，链头和未设置 commit 的版本仍在，查询仍能读到链头。带键索引走查的结果与现在相同。已删除的带键链和回滚后仍有效的版本不被这次扩展收掉。无键旧版本若在溢出链上，释放后的可分配时机与现有超页 GC 用例相同。数据页链或版本链成环时返回内部错误，并且本次不删除无键版本。

**Investigation Facts**

- Starting point: 分支从 `937463c` 拉出。`gc_table` 只调用 `index_manager.scan_all()`。每个索引 `row_id` 是链头。沿 `next_version` 收集 `commit_tx_id().is_some() && current_id != row_id` 的版本，再对每个调用 `delete_tuple_from_data_page`。返回删掉的个数。不读快照，不写 WAL，不改索引。`tests/gc_test.rs` 三个用例覆盖带键回收、未提交链头留下且已提交前驱被回收（断言是 `cleaned_count >= 1`，不是文件头注释里的 `== 0`）、多键各自留下最新值。本计划没有重跑这些测试。
- Current-State Evidence: 无键行不进主键索引。`InsertExecutor` 在 `to_key()` 为 `None` 时跳过 `index_manager.insert`。`UpdateExecutor` 在新键为 `None` 时 `index_manager.delete` 旧键。SQL 的 UPDATE 和 DELETE 只按索引定位，所以「整数主键改成 NULL」是能留下多版本无键链的 SQL 路径。`delete_tuple_from_data_page` 删槽后，若槽带溢出定位则 `free_overflow_chain`：逐页 `free_page`，再 `complete_free_release`。`gc_releases_oversized_version_chain` 证明提升前链页不能分配，`promote_free_releases` 之后能分配到同一批页。`table_has_live_rows` 用 `create_tx_id != 0` 的后向指针建立被指向集合，非墓碑槽才当候选；槽数越界和负载越界返回损坏数据页错误，短于 22 字节或头解析失败的槽跳过。`mark_aborted` 把 `create_tx_id` 置 0。墓碑 `is_deleted()` 为真，删除时写入的元组是空字节，索引条目已去掉。`Database::get_table`、`buffer_pool`、`begin`、`execute_in_tx`、`rollback`、`execute_sql` 都是现有入口。`read_tuple_from_data_page` 会装配溢出。`deserialize_row(data, schema, defaults)` 还原一行。`TableMeta.pk_index` 是主键下标。`Value::to_key` 只对 `Int` 返回 `Some`。页头 `next_page_id` 在该页字节 5..9，小端 u32。`StorageError::Internal` 展示为 `internal error: {文案}`。
- Code and Critical Path: 改 `src/storage/data/table_manager.rs` 的 `TableMeta::gc_table`。读路径用现有的页槽检查、`read_version_header`、`read_tuple_from_data_page`、`deserialize_row`。删除只调用 `crate::storage::delete_tuple_from_data_page`。测试改 `tests/gc_test.rs` 和 `tests/overflow_page_test.rs`。不改 `src/executor/`、`src/transaction/snapshot.rs`、`src/storage/page_format/overflow.rs` 的释放函数、`src/storage/file_storage.rs` 的空闲页函数。

**Implementation Guidance**

索引走查那段循环保持现在的收集和删除顺序。它返回错误时立刻把错误返回，不开始无键走查。

无键走查在索引走查成功之后：

1. 从 `data_page_head` 沿 `next_page_id` 走。已访问页再次出现时返回 `StorageError::Internal(format!("data page chain cycle while collecting keyless versions of table '{}'", self.name))`。此时还没有无键删除。
2. 每一页用与 `table_has_live_rows` 相同的界检查。槽数越界或负载越界：返回那个损坏错误，不删除无键版本。短于 `VersionHeader::SIZE` 或 `from_bytes` 失败：跳过该槽。`create_tx_id != 0` 且 `next_version()` 为 `Some` 时，把目标放进被指向集合。`is_deleted()` 为假的槽记下 `RowId`，作为候选。
3. 候选若不在被指向集合里，用 `read_tuple_from_data_page` 读元组，再用 `deserialize_row` 和本表的列类型、`defaults` 还原。读或反序列化失败就返回该错误，不把该槽当成无键链头，也不删除无键版本。主键值取 `values[self.pk_index]`。`to_key()` 为 `Some` 则跳过。为 `None` 才是无键链头。
4. 从该链头沿 `next_version` 走。`RowId` 再次出现时返回 `StorageError::Internal(format!("version chain cycle while collecting keyless versions of table '{}'", self.name))`，不删除本次已收集的无键版本。`commit_tx_id().is_some() && current_id != head` 的 id 放进待删列表。同一 id 只放一次。
5. 收集全部结束后，才对待删列表逐个调用 `delete_tuple_from_data_page`，成功一个就把返回计数加一。删除中途失败则返回该错误，已经删掉的不恢复。
6. 返回值是索引走查删掉的个数加上无键删除的个数。

无键路径不调用索引 insert、delete、update，不追加 WAL，不加锁，不调用 `Snapshot`。溢出页只由 `delete_tuple_from_data_page` 里现有的 `free_overflow_chain` 释放。

槽的后向指针指向自己时，该槽在被指向集合里，不是链头。版本环用例要做成：链头指向 A，A 指向 A。这样链头不在被指向集合里，走查会在 A 上第二次遇到 A。

**Task Definitions**

### T1: 无键旧版本可回收且带键合同仍在

- Requirement/Scenario: `keyless-version-gc` 的三条 Requirement。proposal 的 S1–S11。
- Depends on: None
- Targets: `src/storage/data/table_manager.rs::TableMeta::gc_table`；`tests/gc_test.rs`；`tests/overflow_page_test.rs::gc_releases_keyless_oversized_predecessor`
- Current behavior: `gc_table` 只从主键索引进入版本链。无键链的旧版本删不掉。带键链按 `commit_tx_id().is_some() && current_id != 索引 row_id` 删除，删除函数是 `delete_tuple_from_data_page`。已删除带键链不在索引里，所以不回收。回滚后的有效版本仍由索引指向，GC 不删它。超页旧版本在提升前不能分配，提升后能分配到原链页。
- Required behavior: 一次 `gc_table` 同时做到 S1–S11。签名不变。返回的 `usize` 包含无键回收数。S10 的错误文案是 `data page chain cycle while collecting keyless versions of table '{表名}'`。S11 的错误文案是 `version chain cycle while collecting keyless versions of table '{表名}'`。展示前缀仍是 `internal error: `。收集阶段出错时无键槽一个都不删。索引走查失败时不开始无键走查。
- Preserve: 索引走查循环的判定和「先收集、再在该键的循环里删除」。`delete_tuple_from_data_page` 及其溢出释放。`Snapshot`、空闲页提升、WAL、索引条目。三个既有带键用例的断言原文。`gc_releases_oversized_version_chain` 的断言原文。
- Forbidden: 用数据页走查替换索引走查。按快照或活跃事务留下旧版本。跳过「旧版本元组仍能 `to_key`」的槽。把已删除带键链或回滚后的有效版本收掉。在 GC 里写 WAL、改索引、加锁、改 `free_page` 或提升条件。新增 SQL 或后台任务。修改 `openspec/specs/`。本轮执行全量 `cargo test`。把既有用例的注释改成和断言一致（那不是本任务）。
- First test: 先只加测试，不改 `gc_table`。然后运行下面两条命令。既有用例应仍通过。新用例应失败：S1 的回收数今天是 0，S5 的无键超页链今天回收数是 0，S10 和 S11 今天不会返回规定的内部错误。这就是 RED。
- GREEN condition: 改完 `gc_table` 后，下面两条命令都是退出码 0、failed 为 0。S1–S11 的断言都在这两条里面。
- Verification: 定向命令，不是全量。

```text
cargo test --offline --test gc_test -- --test-threads=1
cargo test --offline --test overflow_page_test -- --test-threads=1
```

失败表示 S1–S11 有一条不成立，或既有 GC / 溢出用例被改坏。全量不在本任务。

- Stop when: 要让无键旧版本被回收，就必须改 `Snapshot` 的可见性、空闲页提升条件、索引格式、WAL 或刷盘。或者既有三个带键用例必须改断言才能通过。或者必须新增 SQL GC 才能达到 S1 的查询结果。这些都返回计划，不在本任务里扩范围。

新用例的名字和应当看到的结果：

1. `keyless_transition_collects_old_versions_and_selects_head`（S1）。`Database::open`。`CREATE TABLE t (id INT, v INT)`。插入 `(1, 1)`，再两次把 `v` 更新为 2 和 3，都经 `execute_sql` 提交。在把 `id` 改成 NULL 之前，用 `index_manager.search` 拿到当前 `RowId`，沿 `read_version_header` 的 `next_version` 记下整条链上的 id。然后 `UPDATE t SET id = NULL WHERE id = 1`。`gc_table` 的返回值等于记下的 id 个数。每个记下的 id，`read_tuple_from_data_page` 返回 `SlotNotFound`。`SELECT id, v FROM t` 只得到 NULL 和 3。
2. `uncommitted_keyless_head_keeps_itself_and_collects_predecessor`（S2）。插入 `(1, 10)` 并提交。记下索引里的 `RowId`。`begin` 之后 `execute_in_tx` 执行 `UPDATE t SET id = NULL WHERE id = 1`，不提交。`gc_table` 返回值大于等于 1。旧 `RowId` 为 `SlotNotFound`。同一事务里 `execute_in_tx("SELECT id, v FROM t")` 仍看到 NULL 和 10。
3. `single_null_key_row_survives_gc`（S3）。`INSERT INTO t (id, v) VALUES (NULL, 7)`。`gc_table` 返回 0。`SELECT` 仍是这一行。
4. `two_null_key_rows_both_survive_gc`（S4）。插入两行 NULL 主键，值不同。`gc_table` 返回 0。`SELECT` 仍是两行。
5. `deleted_keyed_chain_is_not_collected`（S7）。插入并更新出至少两个已提交版本，记下链上的 `RowId`。`DELETE` 该主键并提交。`gc_table` 返回 0。记下的槽仍能读到。`SELECT` 没有该行。
6. `rolled_back_update_surviving_version_remains`（S8）。插入 `id = 1` 并提交。`begin`，`execute_in_tx` 把 id 改成 2，`rollback`。`gc_table` 返回 0。`SELECT` 仍得到 1。
7. `mixed_keyed_and_keyless_gc_keeps_both_heads`（S9）。同一张表：一行保持整数主键并有已提交旧版本，另一行走 S1 的 NULL 键链。一次 `gc_table` 之后，两个链头都还能读到，两条链记下的旧 id 都是 `SlotNotFound`，`SELECT` 同时有这两行的当前值。回收数等于两条链旧版本数之和。
8. `keyless_gc_stops_on_data_page_cycle_without_deleting`（S10）。表上有一行 NULL 主键。把 `data_page_head` 那一页字节 5..9 写成该页自己的 id（小端 u32），用缓冲池现有的改页途径。`gc_table` 返回 `Err`，展示文案含表名和 `data page chain cycle`。该行的槽仍能读到。
9. `keyless_gc_stops_on_version_cycle_without_deleting`（S11）。写出已提交的 NULL 键链头，其 `next_version` 指向已提交版本 A，A 的 `next_version` 指向 A。用 `write_tuple_to_data_page` 与 `update_version_header_in_data_page` 组这条链，不插入索引。`gc_table` 返回 `Err`，展示文案含表名和 `version chain cycle`。链头和 A 的槽仍能读到。
10. `gc_releases_keyless_oversized_predecessor`（S5，放在 `tests/overflow_page_test.rs`）。复用 `setup_raw` 和 `gc_releases_oversized_version_chain` 的分配与提升断言。前驱是已提交超页槽。链头是已提交的页内槽，元组用 `serialize_tuple` 写成该表的 NULL 整数键，不调用 `index_manager.insert`。`gc_table` 返回 1。提升前分配不到那些链页，`promote_free_releases` 之后分配到的是同一批页。

既有 `test_gc_removes_old_versions`、`test_gc_preserves_uncommitted_versions`、`test_gc_multiple_keys` 和 `gc_releases_oversized_version_chain` 不改断言，GREEN 时仍通过（S6）。

**Invariants**

- 带键行的链头仍是索引里的 `row_id`。无键行的链头按上面的被指向集合、非墓碑和 `to_key() == None` 识别。
- 回收谓词只有 `commit_tx_id().is_some() && current_id != head`。
- 物理释放只有 `delete_tuple_from_data_page`。
- `gc_table` 的签名和「返回本次删除的版本数」不变。
- 新增测试各自使用自己的 `TempDir`，不依赖执行顺序或并行度。

**Non-goals**

快照可见性、空闲页提升、索引格式、WAL、刷盘、SQL GC、速度、MS47、全量回归、改既有用例注释。

**Acceptance**

- S1–S4、S7–S11 在 `cargo test --offline --test gc_test -- --test-threads=1` 里为退出码 0、failed 为 0。
- S5 与既有超页 GC 用例在 `cargo test --offline --test overflow_page_test -- --test-threads=1` 里为退出码 0、failed 为 0。
- S6：三个既有带键用例的断言文本没有改，并且包含在前一条 `gc_test` 结果里。
- 代码只动 `gc_table` 和这两个测试文件。`git diff` 里没有 `snapshot.rs`、`overflow.rs` 的释放函数、`file_storage.rs`、执行器和 WAL。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 无键行的旧版本按既有 GC 规则回收 | T1 | `keyless_transition_collects_old_versions_and_selects_head` | Covered |
| 带键行的 GC 合同保持 | T1 | `deleted_keyed_chain_is_not_collected` | Covered |
| 走查不能结束时不删除无键版本 | T1 | `keyless_gc_stops_on_data_page_cycle_without_deleting` | Covered |

**Verification**

每条场景都由上表里的测试直接断言回收数、`SlotNotFound`、`SELECT` 或 `execute_in_tx` 的行、错误文案，或溢出页在提升前后的分配结果。S5 走现有的 `allocate_page` 与 `promote_free_releases`，不另写判定程序。S1、S3、S4、S7、S8、S9 的查询走 `Database::execute_sql` 或 `execute_in_tx`。GC 本身走 `gc_table`，因为没有 SQL 入口。两条 `cargo test` 都带单个 `--test`，不受 M19 的全量分批约束。全量不在本 Cycle。

**Gate 2 Readiness**

全部 PASS。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

定向测试可以重跑。全量不在本 Cycle。Act Response 记下命令和退出结果即可。

**Risks and Notes**

- 索引走查已经删除的带键旧版本，不会因为后面的无键收集失败而恢复。调用方收到 `Err` 时，这次调用不是完整成功。
- `test_gc_preserves_uncommitted_versions` 头上的注释与断言不一致。实施时不要按那条注释改 GC，也不要顺手改注释。
- 待删 `RowId` 列表只活在这一次调用的栈上。不为它加磁盘结构。
- I038 的「量化先行」不是本 Cycle 的未决项。

## Act Response

- Status: （实施时填写）

**Implemented**

（实施时填写）

**Verification Evidence**

（实施时填写）

**Persisted Evidence**

（实施时填写）

## Plan Review

- Review Result: pending

**Findings**

（Review 时填写）

**Follow-up Decision**

（Review 时填写）

**Next Cycle**

None

**Next Iteration**

（本 Iteration 被接受后才写。计划中的下一轮是 `iterations/001-batched-full/000-initial.md`，现在不创建。）
