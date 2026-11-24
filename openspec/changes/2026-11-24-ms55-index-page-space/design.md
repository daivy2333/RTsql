## 背景

MS55 要先测量索引页空间，再决定瘦内部节点或 Tag 合并是否改格式。空间收益盖过格式兼容成本才改。未过门槛就保持原格式和现有锁。

本次只处理格式这一支，来源是 I029 和 I030。节点锁在变更 `2026-11-24-ms55-index-node-lock`，等本次格式结论被接受后再计划。当前起点的生产代码与 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 相同。分支 `changes/2026-11-24-ms55-index-page-space` 从这里切出。没有其他活跃变更被本分支依赖。

## 当前行为

内部节点槽的负载是 32 字节 `Key` 加 4 字节子页号。`InternalNode::insert_separator` 把 `entry_size` 设为 `MAX_KEY_LEN + 4`。`MAX_KEY_LEN` 是 32。叶槽是 `MAX_KEY_LEN + RowId::SIZE`，`RowId::SIZE` 是 6，共 38 字节。`leftmost_child` 放在页头的 `next_page_id`，不占分隔槽。内部节点不存 `RowId`，也不存行数据。

`string_key_uses_existing_tree_entry_width` 插入 200 个 `s0000` 这一形的、可放入 32 字节的字符串键之后，断言根页首字节是 `0x02`，第一枚分隔槽长度 36，最左子叶第一枚槽长度 38。规划时执行：

`cargo test --offline --lib storage::btree::index_manager::tests::string_key_uses_existing_tree_entry_width -- --test-threads=1`

退出码 0。`1 passed; 0 failed; 0 ignored; 0 measured; 465 filtered out`。这一次只证明现有断言成立，没有打印索引页数。

数据页槽由 `write_tuple_to_data_page` 写成 `version_header.to_bytes()` 后面直接接 tuple 字节。`VersionHeader::SIZE` 是 22，布局是 `create_tx_id` 8 字节、`commit_tx_id` 8 字节、`next_version` 6 字节。`read_tuple_from_data_page` 把槽的前 22 字节交给 `VersionHeader::from_bytes`，其余当作 tuple 或溢出定位。tuple 每个值自己带 tag，INT 是 `TAG_INT`（`0x01`）加 8 字节。页头 `SlottedPageHeader` 有 3 字节填充，用来把页头补到 16 字节，不属于 slot。

文件格式版本 `FORMAT_VERSION` 是 1。`decode` 只接受 `1..=FORMAT_VERSION`。没有第二套内部槽宽度或版本头宽度的读者。

写路径上的整树串行在 `IndexManager::write_gate`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 里取得它。`search`、`scan_all`、`scan_range` 不取得它。本次不改这把闸门。

## 目标与非目标

目标：用两条库测试量出可移除字节和索引页数。两处都是 0 时，执行记录写明保持现有内部节点槽和数据槽。生产逻辑不改。

非目标：节点锁、latch coupling、`write_gate` 的替换、ordered-variable、prefix-restart、短于 32 字节的分隔键、把按值 tag 并进 `VersionHeader`、改页头填充、改 `FORMAT_VERSION`、脏页批量写、io_uring、代价模型、全量测试。

## D1 格式和节点锁分成两个变更

格式结论看槽字节和现有读者还能不能读当前库。节点锁看写争用，以及写单元、根发布、取消和回滚还在不在。同一次恢复测试给不出争用，同一次并发测试给不出可移除字节。MS55 的分裂信号因此成立。

先做格式。I025 写明节点锁不改索引格式。锁若加在尚未裁定的槽布局上，格式一旦改动，争用测量就不再描述将要保留的树。所以 `2026-11-24-ms55-index-node-lock` 依赖本次执行记录里的格式结论，本批不展开它的 Cycle。

被替代的做法是把 I025、I029、I030 放进同一个变更，用一轮测试同时裁定格式和锁。那一轮的通过不能同时回答两件事。

## D2 可移除字节是空间收益

内部节点的可移除字节 = 分隔槽长度 − `MAX_KEY_LEN` − 4。数据槽的可移除字节 = 原始槽长度 − `VersionHeader::to_bytes().len()` − 本次写入的 tuple 长度。

索引页数来自同一次 200 键插入之后的 `collect_all_pages`。它写入执行记录，不参与改不改格式的比较。

被替代的做法是设一个百分比，例如页数下降 10% 才改格式。MS55、现有规格和这次读到的代码都没有这个数。用一个新写的比例代替「盖过兼容成本」，就是在猜门槛。

## D3 兼容成本是现有读者

当前构建写出的库要仍能被现有代码读开。内部节点 `get_key` 要求槽至少 `MAX_KEY_LEN + 4` 字节，并把前 32 字节当作 `Key`。数据页把槽的前 22 字节当作 `VersionHeader`。格式版本保持 1。

可移除字节为 0 时，槽宽不变，现有读者继续适用。没有被去掉的字节去支付另一套槽宽或另一套版本头。保持现在的格式。

可移除字节大于 0，或者根页不是内部节点、槽字节和 `to_bytes()` 对不上，本轮停止。不在本 Cycle 里写新布局，也不写兼容读者。

## D4 更短的分隔键不在候选里

I029 的测量对象是「内部节点只存 separator」相对现在多出来的字节。现在的 separator 就是 32 字节 `Key` 加子页号。再变短就要把 Key 收成前缀或变长编码。

M03 要求 Key 固定 32 字节。MS38 已关闭 ordered-variable 和 prefix-restart，并写明历史空间估算不能当作实测结论。MS55 写明不沿用变长键假设。把分隔键改短会改 M03 的验收边界，本次 goal 没有批准扩大到那一步。

被替代的做法是在测试里做一版前缀分隔键，再和 36 字节槽比页数。那一版已经越出本次批准的键宽。

## D5 按值 tag 不是槽 tag

I030 写的是每个 slot 省 1 字节，把槽上的 Tag byte 并进 `VersionHeader`。当前写入没有这样一个位于版本头之外、又不属于 tuple 的字节。INT 的 `0x01` 是 tuple 的第一个值 tag，M05 要求它留在值前面。

页头那 3 字节填充是每页 3 字节，不是每槽 1 字节。去掉它会改变 16 字节页头，后续偏移都跟着变。它不是 I030 的那 1 字节。

被替代的做法是把每个值的 tag 并进 22 字节版本头。那会改 M05，并且节省的是每个值 1 字节，不是 I030 写的每个 slot 1 字节。本轮不做。

## 风险

200 个键的索引页数由分裂实现决定。本轮只抄打印，不把某个页数写成采纳线，也不为了页数更好看而重跑。

`string_key_uses_existing_tree_entry_width` 已经在规划时通过。实施不重跑它，除非 `src/storage/btree/` 里测试模块以外的代码有了差异。新测试自己会再断言 36 和 38。

数据槽测试若编译失败，先核对符号是否仍是 `write_tuple_to_data_page`、`serialize_tuple`、`with_page_data`、`get_slot_by_logical_id`。符号还在而断言失败，就停止，不改写入实现。

## 回滚

没有格式迁移。去掉两条新测试和执行记录中的测量，就没有这次保持结论。已有库文件不用迁移。

## 实现顺序

1. 先写索引页测试并运行。可移除字节不是 0，或根页不是内部节点，就停止，不写保持结论，也不改 `node.rs`。
2. 再写数据槽测试并运行。gap 不是 0，或前 22 字节对不上，就停止，不改 `version_chain.rs` 和 `tuple.rs`。
3. 两处都是 0 时，在执行记录写明保持原格式，并确认生产逻辑相对本计划交接时的 `src/` 没有差异。两条测试都在。
