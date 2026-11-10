# Iteration 000 / Cycle 000: 无键链按现有判定被 GC 看见

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS46；I038；M10 的现有 GC 判定；M19 的分批全量约束被本次 goal 收紧为「本 change 不跑全量，定向测试一律 `--test-threads=1`」
- Excluded scope: GC 速度、MVCC 可见性规则、索引格式、空闲页生命周期、刷盘、节点锁、分配器、clone 消除、SQL GC、让 DELETE 或全表 UPDATE 找到无键行、全量回归、MS47

**Objective**

`TableMeta::gc_table` 回收无键链上已提交且不是链头的旧版本，链头和未提交版本仍可读。带键索引链的回收结果与已提交删除留下的墓碑链保持基点行为。损坏数据页返回现有的点名错误。

**Investigation Facts**

- Starting point: 分支 `changes/2026-11-10-ms46-keyless-row-gc`，基点 `937463c7dfa8ab6bec400e2573b11bdd4b72103b`。规划开始时该分支没有本次变更文件，产品代码与该基点相同。变更前验证：在该工作目录执行 `cargo test --offline --test gc_test -- --test-threads=1`，退出码 0。输出末尾为 `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`。三则是 `test_gc_removes_old_versions`、`test_gc_preserves_uncommitted_versions`、`test_gc_multiple_keys`。
- Current-State Evidence:
  - `src/storage/data/table_manager.rs::TableMeta::gc_table`（约 474–503 行）是唯一实现。调用者只有 `tests/gc_test.rs` 和 `tests/overflow_page_test.rs::gc_releases_oversized_version_chain`。没有 SQL 调用。
  - 第一段：`self.index_manager.scan_all()` 返回 `Vec<(Vec<u8>, RowId)>`。对每个 `row_id` 循环 `buffer_pool.read_version_header`，`header.commit_tx_id().is_some() && current_id != row_id` 时调用 `crate::storage::delete_tuple_from_data_page`，计数加一。不使用 `Snapshot::is_visible`。
  - `read_version_header`（`buffer_pool.rs` 约 388 行）转调 `read_tuple_from_data_page`。页头槽数越界时该读已经返回 `StorageError::CorruptedDataPage`，Display 为 `corrupted data page {page_id}: {detail}`。
  - `VersionHeader`（`src/transaction/version_chain.rs`）：`commit_tx_id()` 在 `UNSET_TX_ID` 时为 `None`，否则为 `Some`。`mark_deleted` / `mark_aborted` 把提交号设为 `DELETED_TX_ID`，所以墓碑和已回滚版本的 `commit_tx_id()` 是 `Some`。`is_deleted()` 看这个哨兵。`mark_aborted` 另把 `create_tx_id` 写成 0。
  - 无键写入：`InsertExecutor` 在 `pk_value.to_key()` 为 `None` 时不调用主键 `insert`（`src/executor/insert.rs` 约 280–286 行）。`UpdateExecutor::update_row` 把新版本头写成 `with_next_version(old_row_id)`（`update.rs` 约 311 行）；新键 `to_key()` 为 `None` 时 `index_manager.delete(key)`（约 366–369 行）。全表更新只 `scan_all`（约 108 行）。`DeleteExecutor` 写空元组墓碑后 `index_manager.delete(key)`（`delete.rs` 约 181–228 行）。
  - 因此现有 SQL 能造出的无键旧版本链是：整数键行已提交，再把 `id` 更新为 NULL。非整数键列的单次插入没有旧版本。已经无键的行不能再被 `UPDATE` / `DELETE` 找到。
  - 链尾定义在 `src/executor/create_index.rs::backfill_tree`（约 119–174 行）：`create_tx_id != 0` 的 `next_version` 才把目标放入被指向集合。`DataScanExecutor::build_superseded_map`（`data_scan.rs` 约 241–306 行）给出数据页遍历的错误处理：槽数越界和槽负载越界返回错误，短于版本头的槽跳过，`next_page_id == 0` 结束。
  - `delete_tuple_from_data_page`（`data_page.rs` 约 351–414 行）先删槽，再 `free_overflow_chain`。不在该函数里提升空闲页。溢出见证已在 `gc_releases_oversized_version_chain`：提升前分配不到链页，`promote_free_releases(release_completion_limit())` 之后可以。
  - `deserialize_row` 在 `src/storage/page_format/tuple.rs`。`TableMeta` 有 `columns`、`defaults`、`pk_index`、`data_page_head`。`Database::get_table`、`execute_sql`、`begin`、`execute_in_tx`、`checkpoint`、`close` 均为公开方法。`execute_sql` 自动提交；显式事务内的语句走 `execute_in_tx`。`Database::open` 默认 Repeatable Read。`statement_snapshot`（`database.rs` 约 508–510 行）对 Repeatable Read 返回 `None`。`execute_stage_in_tx_routed` 把这个值传给扫描，所以 S4 的事务内 `SELECT` 仍是无快照扫描。不要改用 Read Committed。
  - 无快照的 `DataScan` 会产出未被抑制的未删除槽。未提交新版本的 `commit_tx_id()` 为 `None`，`superseder_suppresses` 对它返回 false，所以提交前的 `SELECT` 能同时看见旧版本和新版本。旧槽被删掉之后，扫描只应剩新版本。
  - `SlottedPage::delete_slot`（`slotted_page.rs` 约 444–461 行）把后面的目录项前移，`logical_id` 保存在目录项里，不改写成新下标。`DataScanExecutor::next`（`data_scan.rs` 约 418–423 行）用 `current_slot_index` 拼 `RowId` 去查替代 map。`build_superseded_map` 的键是 `slot.logical_id`（约 278 行）。未删槽时两者相同。删槽后链头的下标会落到某个旧逻辑 id 上；若该 id 仍是某个已提交版本的 `next_version` 目标，扫描会跳过实际上的链头。无键行的 `SELECT` 走这条扫描。因此本 Cycle 必须把这次查找改成该槽的 `logical_id`，否则 S7、S9 的 `SELECT` 会看不到仍在页上的链头。`superseder_suppresses` 本身不用改。
- Code and Critical Path:
  - 入口：测试或维护代码调用 `TableMeta::gc_table(&self, buffer_pool)`。
  - 数据：索引头 `row_id` → `next_version` 链；无键链头在数据页上，不在主键索引里。
  - 状态变化：被删槽从数据页消失；溢出链进入现有待释放集合；索引头和无键链头的槽保留；清理计数是两段删除数之和。
  - 错误：页损坏或解码失败时返回现有 `Result` 错误，已删槽不补回。
  - 并发：不新增锁。

**Implementation Guidance**

先写 `tests/gc_test.rs` 的新用例并跑出规定的 RED，再改产品代码。第一段循环的判定文本保持 `commit_tx_id().is_some() && current_id != row_id`。第二段只追加无键链头。两段可以共用一个本地异步函数，第一段不得改成数据页扫描。`DataScanExecutor::next` 只把替代 map 的查询键从目录下标改成该槽的 `logical_id`。

选用数据页链全扫，不建无键链登记结构。理由在 `design.md` 的 Decision 1，这里不重复取舍过程。

**Task Definitions**

### T1: gc_table 回收无键旧版本并保持带键结果

- Requirement/Scenario: `row-version-gc` 的四个 Requirement；proposal S1–S11
- Depends on: None
- Targets:
  - `src/storage/data/table_manager.rs::TableMeta::gc_table`
  - `src/executor/data_scan.rs::DataScanExecutor::next`
  - `tests/gc_test.rs` 的新用例
- Current behavior:
  - 只有主键索引 `scan_all` 能到达的链会被回收。无键链头不在索引里，`gc_table` 对它返回的清理计数不包含它的旧版本，旧槽仍能读到。
  - 带键三则用例的断言在基点为 GREEN：三次已提交版本清理至少 2 且最新值仍在索引里；未提交带键更新清理至少 1 且未提交头的 `commit_tx_id()` 为 `None`；三行各三个版本清理至少 6 且当前键可读。
  - 已提交 `DELETE` 的带键行不在索引里，今天清理计数为 0，删除前的槽还在。
  - 只有一个无键版本时，今天清理计数已经是 0，`SELECT` 仍返回该行。
  - 损坏数据页上，空的主键索引不会让今天的 `gc_table` 去读该页，因此页头槽数 700 或槽 offset `0xFFFF` 时它仍返回 `Ok`。
- Required behavior:
  - 保留第一段：`scan_all` 的每个 `row_id` 仍按 `commit_tx_id().is_some() && current_id != 该 row_id` 调用 `delete_tuple_from_data_page`。记下这些索引头。
  - 第二段从 `self.data_page_head` 沿页头 `next_page_id` 走到 0。被 `create_tx_id != 0` 的 `next_version` 指向的 `row_id` 不是链头。额外链头同时满足：不在被指向集合、不在索引头集合、`create_tx_id != 0`、`is_deleted()` 为假、解码后 `values[self.pk_index].to_key()` 为 `None`。解码用 `read_tuple_from_data_page` 和 `deserialize_row(&bytes, schema, &self.defaults)`，schema 来自 `self.columns`。
  - `DataScanExecutor::next` 在查替代 map 之前，读出当前下标上的槽，用该槽的 `logical_id` 组成 `RowId`。`build_superseded_map` 已经用 `logical_id` 做键（`data_scan.rs` 约 278 行）。现在的 `next` 用 `current_slot_index`（约 422 行）。`delete_slot` 会把后面的目录项前移并保留原来的 `logical_id`，删槽之后下标不再等于逻辑 id。不改这里，无键链头在 `SELECT` 里会被当成旧版本跳过。`superseder_suppresses` 和 map 的键不改。没有删过槽时，逻辑 id 仍等于下标，现有扫描结果不变。
  - 对每个无键链头使用与第一段相同的判定和 `delete_tuple_from_data_page`。走到索引头 `row_id` 时不删除，并停止这条链继续向下删除。
  - 页头槽数越界返回 `corrupted_data_page_slot_count_error`。槽负载越界返回 `corrupted_data_page_slot_error`。短于版本头或版本头解析失败的槽跳过。`read_tuple_from_data_page` 或 `deserialize_row` 的错误原样返回。
  - 返回值是两段删除个数之和。
  - 新用例的夹具和断言：
    - `gc_collects_committed_ancestors_of_keyless_head`：`Database::open` 临时库。`execute_sql`：`CREATE TABLE t (id INT, v INT)`；`INSERT INTO t VALUES (1, 10)`；`UPDATE t SET v = 11 WHERE id = 1`，受影响行数为 1。此时 `get_table("t")` 的主键索引 `search(&1i64.to_be_bytes())` 得到 `old`。再 `UPDATE t SET id = NULL, v = 12 WHERE id = 1`，受影响行数为 1。`gc_table` 返回 2。`read_tuple_from_data_page(old)` 为 `SlotNotFound`。`SELECT id, v FROM t` 的 `SqlOutcome::Rows` 只有一行 `[Value::Null, Value::Int(12)]`。
    - `gc_keeps_single_null_key_version`：`INSERT INTO t VALUES (NULL, 1)` 后 `gc_table` 返回 0，`SELECT id, v FROM t` 为 `[Value::Null, Value::Int(1)]`。
    - `gc_keeps_single_string_key_version`：`CREATE TABLE t (s STRING, n INT)`，`INSERT INTO t VALUES ('a', 1)`，`gc_table` 返回 0，`SELECT s, n FROM t` 为 `[Value::String("a"), Value::Int(1)]`。
    - `gc_keeps_uncommitted_keyless_head`：已提交 `INSERT (1, 10)` 后记下索引 `old`。`begin` 后 `execute_in_tx("UPDATE t SET id = NULL, v = 20 WHERE id = 1", &tx)`，受影响行数为 1，不 `commit`。`gc_table` 返回 1。`read_tuple_from_data_page(old)` 为 `SlotNotFound`。同一 `tx` 上 `execute_in_tx("SELECT id, v FROM t")` 只有 `[Value::Null, Value::Int(20)]`。
    - `gc_does_not_collect_keyed_delete_chain`：已提交 `INSERT (1, 10)` 后记下 `old`。`DELETE FROM t WHERE id = 1` 受影响行数为 1。`gc_table` 返回 0。`read_tuple_from_data_page(old)` 成功。
    - `gc_collects_keyed_and_keyless_chains_on_one_table`：带键行 `id=1` 插入 `v=10`，更新为 11，再更新为 12。无键行 `id=2` 插入 `v=20`，更新为 21，再 `SET id = NULL, v = 22`。每次更新受影响行数为 1。`gc_table` 返回 4。主键索引 `search(&1i64.to_be_bytes())` 读出的 `id` 为 1、`v` 为 12。`SELECT id, v FROM t` 含 `[Value::Int(1), Value::Int(12)]` 和 `[Value::Null, Value::Int(22)]`，没有 10、11、20、21。
    - `gc_secondary_index_finds_keyless_head`：`INSERT (1, 10)`，`UPDATE v = 11`，`CREATE INDEX iv ON t(v)`，`UPDATE SET id = NULL, v = 12`。`gc_table` 返回 2。`SELECT v FROM t WHERE v = 12` 返回 `[Value::Int(12)]`。
    - `gc_frees_overflow_pages_of_keyless_ancestor`：`CREATE TABLE t (id INT, v STRING)`。插入 `id=1` 和长度 10000 的 `v`。从索引头槽读出 locator，用 `overflow::decode_locator` 和 `overflow::collect_overflow_pages` 记下链页。再 `UPDATE` 为 `id` NULL、`v` 为短字符串 `'small'`，受影响行数为 1。`gc_table` 返回 1。`SELECT` 读到短字符串。在 `promote_free_releases` 之前，按链页个数调用 `allocate_page`，得到的页号与链页无交集。然后 `promote_free_releases(storage.release_completion_limit())`，再分配同样个数，排序后与链页相同。此用例不调用 `checkpoint`。
    - `gc_corrupt_slot_count_returns_named_error`：建成无键行后 `checkpoint`、`close`。数据页号取 `data_page_head.0 as u32`。按 `tests/data_page_bounded_read_test.rs` 的偏移把该页页内偏移 1 的 `u16` 写成 700 小端。文件偏移为 `HEADER_SIZE + page_id * Page::PAGE_SIZE + 1`。重新 `open` 成功。`gc_table` 的错误 `Display` 含 `corrupted data page {页号}` 和 `slot count 700`。
    - `gc_corrupt_slot_offset_returns_named_error`：同样先关库。槽 0 的 offset 字段在页内 `Page::PAGE_SIZE - Slot::SIZE + 2`，写成 `0xFFFFu16` 小端。重新打开成功。`gc_table` 的错误文本含 `corrupted data page {页号}` 和 `slot 0`。
  - 上述写语句都走 `Database::execute_sql` 或 `execute_in_tx`。`gc_table` 走 `get_table` 得到的 `TableMeta`。不新增判定用的辅助二进制。
  - 既有三则用例的函数体和断言不改。`gc_releases_oversized_version_chain` 的源码不改。
- Preserve:
  - 第一段的判定条件和删除函数。
  - 带键最新槽仍在索引里。
  - 已提交带键 `DELETE` 的旧槽不被删除。
  - `delete_tuple_from_data_page` 的释放顺序和提升时机。
  - `superseder_suppresses` 的可见性规则，以及未删槽页上的扫描结果。
  - UPDATE/INSERT/DELETE 的索引维护、页格式。
- Forbidden:
  - 用数据页扫描替换第一段 `scan_all`。
  - 修改 `delete_tuple_from_data_page`、`InsertExecutor`、`UpdateExecutor`、`DeleteExecutor`、`VersionHeader`。
  - 修改 `superseder_suppresses`、`build_superseded_map` 的键，或 `DataScanExecutor` 里除「用 `logical_id` 查替代 map」以外的行为。
  - 改索引格式、空闲页提升、刷盘、MVCC 的 `is_visible`。
  - 新增 SQL 或 CLI。
  - 吞掉 `SlotNotFound` 或解码错误来凑清理计数。
  - 修改 `openspec/specs/` 权威正文、`AGENTS.md`、`.agents/memory/project-model.md`。
  - 跑全量，或去掉 `--test-threads=1`，或一次命令带多个 `--test` 目标。
- First test:
  - 位置：`tests/gc_test.rs` 的新函数，名字见 Required behavior。
  - 变更前：既有三则、`gc_keeps_single_null_key_version`、`gc_keeps_single_string_key_version`、`gc_does_not_collect_keyed_delete_chain` 为 GREEN。
  - RED：`gc_collects_committed_ancestors_of_keyless_head` 得到 0 而不是 2；`gc_keeps_uncommitted_keyless_head` 得到 0 而不是 1，且旧槽仍在；`gc_collects_keyed_and_keyless_chains_on_one_table` 得到 2 而不是 4；`gc_secondary_index_finds_keyless_head` 的清理计数不是 2；`gc_frees_overflow_pages_of_keyless_ancestor` 的清理计数不是 1 或链页在提升后取不回；两个损坏页用例得到 `Ok` 而不是点名错误。
  - 命令：`cargo test --offline --test gc_test -- --test-threads=1`
  - 改 `gc_table` 之前先跑这条命令。RED 用例失败、保留用例通过，才改产品代码。只为确认 RED 的那一次失败不计入三次失败。
- GREEN condition: 下面三条命令都退出码 0，且各自 `failed` 为 0。改产品代码之前，`keyless_eq_routing_test` 已经是 GREEN；改完仍是 GREEN。
- Verification:
  - 按这个顺序各执行一次，上一条退出后再执行下一条。每条都带 `--test-threads=1`。通过条件是该命令退出码 0 且输出里的 `failed` 为 0。
    1. `cargo test --offline --test gc_test -- --test-threads=1`
    2. `cargo test --offline --test overflow_page_test gc_releases_oversized_version_chain -- --test-threads=1`
    3. `cargo test --offline --test keyless_eq_routing_test -- --test-threads=1`
  - `gc_test` 失败表示 S1–S8、S10、S11 或带键三则里有断言未成立。溢出命令失败表示带键超页版本的现有释放被碰到了。第三条失败表示未删槽的无键扫描结果变了。
  - 不跑 `cargo test` 无过滤全量，不跑不带过滤的 `--lib`，不把多个集成测试目标放进同一条命令。
- Stop when:
  - 要让无键旧版本被回收，必须改 `Snapshot::is_visible`、第一段判定、`delete_tuple_from_data_page` 的释放顺序，或空闲页提升条件。停止并回报，不改这些。
  - 第二段对应当只删除一次的槽得到 `SlotNotFound`。停止并回报，不把这个错误改成跳过。
  - 无键链头或带键索引头被删除。停止并回报。
  - `SELECT` 在旧槽已删除后看不见链头，而且链头槽的 `logical_id` 已经用于查替代 map 仍然看不见。这时才停止。只因下标被当成逻辑 id 而看不见，按 Required behavior 改 `next`，不要改可见性规则。
  - 已提交带键 `DELETE` 的旧槽被删除。停止并回报。
  - `open` 在槽数 700 或槽 offset `0xFFFF` 的夹具上失败。这与 `data_page_bounded_read_test` 的打开结果不一致，停止并回报，不要改成「打开失败也算 GC 报错通过」。
  - 未提交夹具里，`SELECT` 在旧槽已删除后仍看不见 `[NULL, 20]`，或还看见旧值。停止并回报。
  - 要把链头留在 `SELECT` 结果里，就必须改 `superseder_suppresses` 的规则。停止并回报。允许的只有改用 `logical_id` 做查找。

**Invariants**

- 带键索引链的回收判定保持 `commit_tx_id().is_some() && current_id != 索引头`。
- 删除只通过 `delete_tuple_from_data_page`。
- 主键索引当前指向的槽在 `gc_table` 返回成功时仍可读。
- 不新增磁盘格式，不改读路径可见性。

**Non-goals**

- 提高 GC 速度。
- 改变 MVCC 可见性规则。
- 改索引格式、空闲页生命周期或刷盘。
- 节点级锁、分配器、clone 消除。
- 新增 GC 的 SQL 或 CLI。
- 让 `DELETE` 或全表 `UPDATE` 找到无键行。
- 本 change 的全量回归。

**Acceptance**

- S1：`gc_collects_committed_ancestors_of_keyless_head` 清理计数为 2，旧槽 `SlotNotFound`，`SELECT` 只有 `[NULL, 12]`。
- S2：`gc_keeps_single_null_key_version` 计数为 0，行仍在。
- S3：`gc_keeps_single_string_key_version` 计数为 0，行仍在。
- S4：`gc_keeps_uncommitted_keyless_head` 计数为 1，旧槽消失，同一事务 `SELECT` 只有 `[NULL, 20]`。
- S5：既有三则用例不改断言且通过。
- S6：`gc_does_not_collect_keyed_delete_chain` 计数为 0，旧槽仍可读。
- S7：`gc_collects_keyed_and_keyless_chains_on_one_table` 计数为 4，带键最新值和无键当前值都在，旧值不在。
- S8：`gc_secondary_index_finds_keyless_head` 计数为 2，`WHERE v = 12` 返回 12。
- S9：`gc_frees_overflow_pages_of_keyless_ancestor` 计数为 1，提升前链页分不回，提升后能分回。
- S10、S11：两个损坏页用例的错误文本含页号和对应的 `slot count 700` 或 `slot 0`，进程不中止，打开成功。
- `gc_releases_oversized_version_chain` 仍通过。
- 代码范围只有 `gc_table` 及其完成本判定所需的本文件私有辅助、`DataScanExecutor::next` 的行身份查找，加上 `tests/gc_test.rs` 的新用例。

**Verification**

1. `cargo test --offline --test gc_test -- --test-threads=1`
2. 上一条退出后再执行 `cargo test --offline --test overflow_page_test gc_releases_oversized_version_chain -- --test-threads=1`
3. 上一条退出后再执行 `cargo test --offline --test keyless_eq_routing_test -- --test-threads=1`

两条都从本 change 的工作目录执行。通过条件是各自退出码 0，且 `test result:` 行的 `failed` 为 0。S1–S8 的写和读走 `execute_sql` / `execute_in_tx`，GC 走 `gc_table`。S9 的释放观察使用现有 `allocate_page` 与 `promote_free_releases`。S10、S11 直接看 `gc_table` 返回的错误文本。

本 Iteration 和本 change 都不跑全量。理由是用户原话：「这次自动化把全量测试放到最后来限制全量的次数」。风险：本 change 收尾前没有全量；合回后那一次失败时，排查范围包含 MS46 与 MS47。

**Gate 2 Readiness**

PASS。审查项全部通过。BLOCKED: 无。SKIPPED: 无。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 数据页链成环时与 `build_superseded_map` 一样没有环检测。本次不加。
- 删槽后若仍用目录下标查替代 map，`SELECT` 会漏掉未回收的链头。本 Cycle 用 `logical_id` 修正查找，不改抑制规则。这是 S7、S9 成立的必要条件，不是额外功能。
- 第一段已经删除版本之后第二段才报错，已删槽不补回。这与第一段中途失败的现有行为相同。
- 两次并发 `gc_table` 仍没有额外互斥。本次不加锁。
- 定向测试的 `--test-threads=1` 高于工作区里已提交的 M19「单次目标验证可用默认并行度」那一句。以本次 goal 为准。
- 全量不在本 Cycle 的通过条件里。不要把「未跑全量」写成 Gate 失败，也不要补一个全量 Iteration。

## Act Response

- Status: reported

**实际改动**

- `src/storage/data/table_manager.rs`
  - `TableMeta::gc_table` 仍先 `scan_all`。记下索引头后，对每个索引头调用 `delete_committed_old_versions`，停止集合为 `None`。
  - `TableMeta::keyless_chain_heads` 从 `data_page_head` 沿 `next_page_id` 走到 0。`create_tx_id != 0` 的 `next_version` 目标进入被指向集合。额外链头须同时不在该集合、不在索引头集合、`create_tx_id != 0`、`is_deleted()` 为假，并且 `deserialize_row` 后 `values[pk_index].to_key()` 为 `None`。页头槽数越界返回 `corrupted_data_page_slot_count_error`。槽负载越界返回 `corrupted_data_page_slot_error`。短于版本头或版本头解析失败的槽跳过。`read_tuple_from_data_page` 与 `deserialize_row` 的错误原样返回。
  - `delete_committed_old_versions` 的删除条件是 `commit_tx_id().is_some() && current_id != row_id`，删除函数是 `delete_tuple_from_data_page`。无键链走到索引头时不删除该槽，并停止继续向下删除。返回值是两段删除数之和。
- `src/executor/data_scan.rs`：`DataScanExecutor::next` 在查替代 map 之前读当前目录项，用该槽的 `logical_id` 组成 `RowId`。`superseder_suppresses` 与 `build_superseded_map` 的键未改。
- `tests/gc_test.rs`：新增 T1 点名的 10 个用例。既有三则用例的函数体和断言未改。

**与计划的偏差**

- 读 `logical_id` 时先把目录下标复制到局部变量，再交给页闭包。否则闭包与 `self.buffer_pool` 会同时借用 `self`，无法编译。查找键仍是该槽的 `logical_id`。
- 当前下标没有槽时（下标已越过槽数，或页头槽数越界因而未取槽），不再用目录下标合成 `RowId` 去查替代 map，直接进入原有闭包。越过槽数时由闭包结束本页；页头槽数越界时仍由闭包返回既有的槽数错误。未删槽的扫描结果由第三条验证覆盖。
- `gc_frees_overflow_pages_of_keyless_ancestor` 在分配见证前断言溢出链非空。空链不能靠两边都是空集合通过页号相等。本次链非空：提升前分配到的页与链页无交集，提升后排序与链页相同。

**自检**

- Gate 3 通过。第一段仍是 `scan_all`，判定文本仍是 `commit_tx_id().is_some() && current_id != row_id`。删除只走 `delete_tuple_from_data_page`。损坏页返回点名错误，不吞 `SlotNotFound` 或解码错误。可见性规则和 map 的键未改。diff 没有计划外的产品文件。编译输出没有新增警告。测试断言的是清理计数、槽是否仍在、`SELECT` 行和错误文本。
- Gate 4 通过。下面三条命令各自退出码 0，且 `failed` 为 0。
- Gate 5 未进入。
- 遗留 Minor：无。

**验证**

改产品代码之前，在本工作目录执行 `cargo test --offline --test gc_test -- --test-threads=1`。退出码 101。`test result: FAILED. 6 passed; 7 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s`。失败的 7 个是规定的 RED：无键祖先计数为 0 而不是 2、未提交链为 0 而不是 1、同表为 2 而不是 4、二级索引为 0 而不是 2、溢出祖先为 0 而不是 1，两个损坏页用例得到 `Ok(0)`。未提交链的失败停在计数断言，旧槽断言这次没有执行到。通过的 6 个是既有三则、单行 NULL、单行 STRING、已提交带键 DELETE。这次失败不计入三次失败。

改完后按顺序各执行一次，上一条退出后再执行下一条：

1. `cargo test --offline --test gc_test -- --test-threads=1`  
   退出码 0。`test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s`
2. `cargo test --offline --test overflow_page_test gc_releases_oversized_version_chain -- --test-threads=1`  
   退出码 0。`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 26 filtered out; finished in 0.00s`
3. `cargo test --offline --test keyless_eq_routing_test -- --test-threads=1`  
   退出码 0。`test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s`

这三条是 rustfmt 之前的运行。之后只对这三个 Rust 文件做了 rustfmt，只改排版，没有重跑。未跑全量。这不构成本 Cycle 的 Gate 失败。

**Persisted Evidence**

None required。

**Experience Candidates**

None

**未解决问题**

无。

## Plan Review

- Review Result: pending

**Findings**

Act 尚未执行。

**Follow-up Decision**

等待 Act。

**Next Cycle**

None

**Next Iteration**

None
