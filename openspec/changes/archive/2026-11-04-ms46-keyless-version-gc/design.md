# 无键链在索引走查之后按同一规则收集再删除

见 `proposal.md` 的 Why。这里只记录实现所依赖的代码事实和选择。规格见 `specs/keyless-version-gc/spec.md`。

规划时的产品代码在 `937463c`。本计划没有重跑测试。带键 GC 的当前合同来自 `gc_table` 的实现和 `tests/gc_test.rs` 的断言。

## Context

`TableMeta::gc_table`（`src/storage/data/table_manager.rs`，约 474–503 行）对 `index_manager.scan_all()` 的每个 `row_id` 沿 `VersionHeader::next_version` 走链。`commit_tx_id()` 为 `Some` 且 `current_id != row_id` 的版本放进列表，然后调用 `delete_tuple_from_data_page`。返回值是删掉的版本数。函数不读 `Snapshot`，不写 WAL，不改索引。

`tests/gc_test.rs` 的三个用例走这条路径。`test_gc_preserves_uncommitted_versions` 的断言是 `cleaned_count >= 1`，并检查未提交链头的 `commit_tx_id()` 为 `None`。用例上方的注释写 `cleaned_count == 0`，与断言不符。合同以断言为准。

键位没有 B-Tree 键时，`InsertExecutor` 不插主键索引（`src/executor/insert.rs`，约 280–286 行）。`UpdateExecutor` 把键改成 `to_key()` 为 `None` 的值时删除原索引条目（`src/executor/update.rs`，约 367–369 行）。之后 `scan_all` 看不到这条链。SQL 的 UPDATE / DELETE 只按主键索引定位，所以纯无键行不能再被 SQL 更新或删除；能产生多版本无键链的 SQL 路径是「带键行改成 NULL 键」。非 Int 主键表的行从插入起就不入索引，通常只有一个版本。

`delete_tuple_from_data_page`（`src/storage/data_page.rs`，约 351–415 行）先删槽，再对溢出定位调用 `free_overflow_chain`。`free_overflow_chain`（`src/storage/page_format/overflow.rs`，约 257–273 行）对链上每页 `free_page`，然后 `complete_free_release`。`tests/overflow_page_test.rs` 的 `gc_releases_oversized_version_chain` 断言：提升前这些页分配不到，`promote_free_releases` 之后分配到的是同一批页。

`table_has_live_rows`（同文件，约 1731–1794 行）走数据页链。`create_tx_id != 0` 的 `next_version` 才记入被指向集合。非墓碑槽才是候选。页头槽数越界和槽负载越界返回损坏数据页错误；短于版本头或头无法解析的槽跳过。`VersionHeader::mark_aborted` 把 `create_tx_id` 置 0 并设删除哨兵，后向指针保留。墓碑的 `commit_tx_id()` 是 `Some(DELETED_TX_ID)`，`is_deleted()` 为真。删除墓碑的元组是空字节（`src/executor/delete.rs`，约 181–188 行），索引条目在删除时去掉。

`read_tuple_from_data_page` 会装配溢出元组。`deserialize_row` 按列类型和默认值还原一行。`TableMeta.pk_index` 是主键列下标。`Value::to_key` 只对 `Int` 返回 `Some`。

数据页头的 `next_page_id` 在页字节的 5..9，小端 u32（`SlottedPageHeader` 的序列化）。`StorageError::Internal` 的展示前缀是 `internal error: `。

## Goals / Non-Goals

**Goals**

- `gc_table` 在现有索引走查之后，回收无键链上已提交的非链头版本，返回数包含它们。
- 链头、未设置 commit 的版本、带键链、已删除带键链、回滚后仍有效的版本，按 proposal 的场景留下。
- 删除只调用现有的 `delete_tuple_from_data_page`。

**Non-Goals**

- 不改索引走查循环的判定和删除顺序。
- 不改快照、空闲页提升、WAL、刷盘、索引格式。
- 不新增 SQL 或后台 GC。

## Decisions

1. 保留索引走查，在它成功返回之后再做无键走查。只改成全表扫数据页会把「谁是链头」从索引指针换成另一种规则。已删除的带键链今天不在索引里，全表扫会把它们收掉；回滚槽若被当成链头，会把仍有效的前驱收掉。这两处都改变带键合同。

2. 无键链头必须同时满足：不在「`create_tx_id != 0` 的后向指针」所指向的集合里，自身 `is_deleted()` 为假，并且主键值 `to_key()` 为 `None`。`to_key()` 为 `Some` 的链头交给已经跑完的索引走查。已删除带键行的墓碑是 `is_deleted()`，它的前驱在被指向集合里，两条都不会被当成无键链头。

3. 无键链上的旧版本即使元组里还有整数键，也回收。它们不是索引当前指向的版本。跳过「旧版本仍可 `to_key`」会留下 MS46 要收的那些槽。

4. 判定谓词与索引走查逐条相同：`commit_tx_id().is_some() && current_id != head`。不调用 `Snapshot::is_visible`。加上快照会让无键链比带键链更晚回收，可见性政策就分叉了，也碰到 MS46 的拆分信号。

5. 无键版本先收集完再删除。收集阶段的读错误、损坏页错误或环错误返回时，一个无键槽都还没删。索引走查仍按今天的样子在自己的循环里删除；它返回错误时函数直接返回，不开始无键走查。无键删除循环中途若 `delete_tuple_from_data_page` 失败，已经删掉的无键槽不恢复，与带键循环今天的失败方式相同。

6. 数据页链用已访问页集合。再次插入同一页时返回 `StorageError::Internal`，文案为 `data page chain cycle while collecting keyless versions of table '{name}'`。从链头走后向指针时用已访问 `RowId` 集合，再次遇到时返回 `StorageError::Internal`，文案为 `version chain cycle while collecting keyless versions of table '{name}'`。槽的后向指针指向自己时，该槽在被指向集合里，不是链头，函数不为此单独报错。版本环的可观察情形是：链头指向 A，A 指向 A。

7. 页槽读取与 `table_has_live_rows` 使用同一套界检查：槽数越界、负载越界返回现有损坏错误；短槽和无法解析的头跳过。链头元组用 `read_tuple_from_data_page` 加 `deserialize_row`。读或反序列化失败则返回该错误，不把该槽当成无键链头。

8. 无键删除不调用索引的 insert、delete、update，不追加 WAL，不加新的锁。溢出释放只发生在现有的 `delete_tuple_from_data_page` 里面。

另一种登记结构（I038 原文里的「无键链登记」）要在每次更新维护一份新状态。现有数据页链和版本头已经能找到链头，不增加磁盘字段。

## Risks / Trade-offs

- 链头被误判成无键，索引走查已经删掉的旧槽会在第二次读取时变成 `SlotNotFound`，或者仍有效的行被删掉。主键 `to_key()` 为 `Some` 则跳过；S6、S7、S8、S9 把带键链头、已删除链和回滚锁住。
- 索引走查已经删除之后，无键收集才失败。那些带键旧版本不会恢复。这与今天 `gc_table` 在循环中途返回错误的方式相同。调用方看到的是 `Err`，不能把这次调用当成完整成功。
- 收集阶段把全部无键待删 `RowId` 放在内存里。这是一次 GC 调用里的版本 id 列表，不是新的磁盘结构。MS46 不要求改 GC 的内存或速度。

## Migration Plan

无格式迁移，无新的磁盘字段。部署就是合入 `gc_table` 的这次扩展。回滚是放弃变更分支；主分支在合并前不变。放弃后无键旧版本重新不被 GC 看见，带键 GC 与现在相同。

## Open Questions

无。接口、错误文案和带键边界都已在规格里写死。I038 的量化文字不是本次的未决项。
