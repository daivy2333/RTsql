# Iteration 000 / Cycle 000: 测量可移除字节并保持槽格式

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Inherited scope: `proposal.md` 的场景 1 至 5，`design.md` 的 D2、D3、D4、D5。M03 的 32 字节 Key。M05 的按值 tag。`FORMAT_VERSION` 为 1。
- Excluded scope: 节点锁、`write_gate`、ordered-variable、prefix-restart、短于 32 字节的分隔键、把按值 tag 并进版本头、页头 3 字节填充、文件格式版本、全量测试，以及变更 `2026-11-24-ms55-index-node-lock` 的计划与 Cycle。

**Objective**

两条库测试量出内部节点可移除字节和数据槽 gap。两处都是 0 时，执行记录写明保持现有槽格式。`node.rs` 的条目宽度、`VersionHeader`、`serialize_tuple` 和 `FORMAT_VERSION` 保持与本 Cycle 交接时相同。

**Investigation Facts**

- Starting point: 生产代码基点 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`（当时 `master`）。分支 `changes/2026-11-24-ms55-index-page-space` 从这里切出。Gate 1 的范围提交是 `bfe6b928022490b8a12964ce1e9132e612004c34`。没有可沿用的 Explorer 分析。下面的布局是这次读代码、并跑了一次既有宽度测试的结果。`openspec list` 在切分支前没有活跃变更。`.agents/memory/improvements.md` 有一处与本 goal 无关的未提交修改，实施不得把它纳入提交。
- Current-State Evidence: `MAX_KEY_LEN` 为 32（`src/storage/page_format/key.rs`）。`RowId::SIZE` 为 6。`InternalNode::insert_separator` 的 `entry_size` 是 `MAX_KEY_LEN + 4`。叶条目的 `entry_size` 是 `MAX_KEY_LEN + RowId::SIZE`。`leftmost_child` 存在页头 `next_page_id`。内部节点不存行。`string_key_uses_existing_tree_entry_width` 在 200 个 `s0000` 形字符串键之后断言根页 `0x02`、分隔槽长度 36、叶槽长度 38。规划时命令 `cargo test --offline --lib storage::btree::index_manager::tests::string_key_uses_existing_tree_entry_width -- --test-threads=1` 退出码 0，输出含 `1 passed; 0 failed; 0 ignored; 0 measured; 465 filtered out`。该测试不打印页数。`VersionHeader` 为 22 字节：`create_tx_id` 8、`commit_tx_id` 8、`next_version` 6。`write_tuple_to_data_page` 先 `to_bytes()` 再 `extend_from_slice(tuple_bytes)`，中间不插入 tag。`read_tuple_from_data_page` 把前 22 字节交给 `from_bytes`。INT 的 tuple 是 `TAG_INT`（`0x01`）加 8 字节，`serialize_tuple` 对一个 INT 写出 9 字节。页头填充 3 字节，把 `SlottedPageHeader` 补到 16 字节。`FORMAT_VERSION` 为 1，页大小 4096，文件头 64 字节。`IndexManager::write_gate` 只被 `insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 内取得。`search`、`scan_all`、`scan_range` 不取得它。本次不改闸门。
- Code and Critical Path: 索引测量走已有 `make_manager`：临时文件上的 `FileStorage::open`、`BufferPool::new(10, storage)`、`IndexManager::new`。键用 `Value::String` 的 `secondary_index_key(&ColumnType::String(255))`，再 `IndexManager::insert`。读页用 `async_loader.load_page` 与 `SlottedPageRef::get_slot`。页数用 `IndexManager::collect_all_pages`。数据槽测量走 `data_page::tests::setup`：建单列 INT 表 `test`，主键列名 `id`。写入用 `serialize_tuple` 与 `write_tuple_to_data_page`。原始字节用 `BufferPool::with_page_data`、`SlottedPageRef::get_slot_by_logical_id`、`get_slot_data_checked`。不用 `read_tuple_from_data_page` 做这处测量，因为它已经按 22 字节切开，看不到头前是否多了一个字节。

**Task Definitions**

### T1: 打印索引页数并确认分隔槽没有可移除字节

- Requirement/Scenario: `proposal.md` 场景 1。D2 的内部节点可移除字节。D4 不把更短的 Key 算进候选。
- Depends on: None
- Targets: `src/storage/btree/index_manager.rs` 的 `tests` 模块，新测试 `index_page_separator_has_no_removable_byte`。复用同模块的 `make_manager`。
- Current behavior: 同文件已有 `string_key_uses_existing_tree_entry_width`，断言分隔槽 36、叶槽 38，不打印 `collect_all_pages` 的长度，也没有「可移除字节」这个数。`insert_separator` 写入 36 字节。
- Required behavior: 按 Verification 的命令，测试退出码 0，并有一行 `index_pages=<页数> separator_len=36 leaf_len=38 removable=0`。根页首字节是 `0x02`。`check_tree_page` 对根页和那片叶子都没有 violation。`collect_all_pages` 的长度至少为 2。`removable` 按 `separator.length as usize - MAX_KEY_LEN - 4` 计算，且 `MAX_KEY_LEN` 为 32。叶槽长度等于 `MAX_KEY_LEN + RowId::SIZE`。
- Preserve: `insert_separator`、`insert_separator_simple`、叶插入的 `entry_size`、页类型常量、`make_manager` 的建库方式、既有 `string_key_uses_existing_tree_entry_width` 的断言。
- Forbidden: 修改 `node.rs`、`key.rs`、`file_header.rs`。把分隔键改成短于 32 字节。为了页数而改分裂。调用 `write_gate` 以外的新锁。把打印行写成「保持」或「改格式」。
- First test: 先把这个测试写进 `index_manager.rs` 的 `tests`。生产代码先不动。第一次就运行 Verification 的命令。预期退出码 0，因为断言描述的是当前 `insert_separator`。这是直接看到的当前布局，不是先失败再改槽宽。
- GREEN condition: 同一命令退出码 0，打印行字段齐全，`removable=0`。生产函数的条目宽度没有被修改。
- Verification: 只跑一次 `cargo test --offline --lib storage::btree::index_manager::tests::index_page_separator_has_no_removable_byte -- --test-threads=1 --nocapture`。退出码非 0、缺少打印行、根页不是 `0x02`、`removable` 不是 0，或页数小于 2，都是测量无效。不改负载再跑，也不重跑规划时那条旧测试。
- Stop when: 不改 `node.rs` 就无法让 `removable` 为 0；根页在 200 个键之后仍是叶子；`collect_all_pages` 返回错误；或测试必须改用变长键才能编译。停止。不写 T3 的保持结论。T2 也不再跑。

测试按这个顺序做。插入循环与 `string_key_uses_existing_tree_entry_width` 相同：`i` 从 0 到 199，键为 `Value::String(format!("s{i:04}"))` 经 `secondary_index_key` 得到的字节，`RowId::new(i + 1, 1)`。然后读根页。根页第一枚槽是分隔槽，其 `leftmost_child` 指向要读的叶子，取叶子第一枚槽。`check_tree_page` 两页都无 violation 之后才计算 `removable` 并打印。打印使用上面那一个字段序，一行。

### T2: 打印数据槽 gap 并确认版本头紧贴 tuple

- Requirement/Scenario: `proposal.md` 场景 3。D2 的数据槽可移除字节。D5 不把按值 tag 当成槽 tag。
- Depends on: T1
- Targets: `src/storage/data_page.rs` 的 `tests` 模块，新测试 `data_slot_prefix_is_version_header`。复用同模块的 `setup`。
- Current behavior: `write_tuple_to_data_page` 把 `VersionHeader::to_bytes()` 与 tuple 直接拼接。同文件已有 `write_read_single_tuple` 等往返测试，它们不读原始槽，也不计算 gap。
- Required behavior: 按 Verification 的命令，测试退出码 0，并有一行 `header_len=22 tuple_tag=1 gap=0 slot_len=<槽长>`。槽长等于 22 加 tuple 长度。tuple 长度是 9。槽的前 22 字节等于这次写入所用 `VersionHeader::new(1, None).to_bytes()`。其后的字节等于 `serialize_tuple` 写出的 `Value::Int(7)`。偏移 22 处的字节是 `TAG_INT`。
- Preserve: `write_tuple_to_data_page` 的拼接顺序、`VersionHeader::SIZE`、`serialize_tuple` 的 INT 布局、`setup` 建出的单列 INT 表、溢出路径。
- Forbidden: 修改 `version_chain.rs`、`tuple.rs`、`data_page.rs` 里测试模块以外的函数。用 `read_tuple_from_data_page` 代替原始槽比较。删掉页头填充来凑 1 字节。把 `0x01` 从 tuple 里挪走。
- First test: 先把这个测试写进 `data_page.rs` 的 `tests`。生产写入函数先不动。T1 已经按上面的命令通过之后，再跑本任务的命令。预期退出码 0，因为断言描述的是当前拼接。第一次失败就停止，不改写入去凑绿。
- GREEN condition: 同一命令退出码 0，打印行里 `header_len=22`、`tuple_tag=1`、`gap=0`。`write_tuple_to_data_page` 的函数体没有被修改。
- Verification: 只跑一次 `cargo test --offline --lib storage::data_page::tests::data_slot_prefix_is_version_header -- --test-threads=1 --nocapture`。退出码非 0、gap 不是 0、前 22 字节与 `to_bytes()` 不同，或偏移 22 不是 `TAG_INT`，都是测量无效。不改表结构再跑。
- Stop when: 不改 `write_tuple_to_data_page` 或 `VersionHeader` 就无法让 gap 为 0；`serialize_tuple` 对一个 INT 的长度不是 9；`get_slot_by_logical_id` 找不到刚写入的槽。停止。不写 T3 的保持结论。

测试按这个顺序做。`setup` 之后，`let vh = VersionHeader::new(1, None)`。`serialize_tuple(&[Value::Int(7)], &[ColumnType::Int], &mut buf)`，`buf` 至少 16 字节，记下写出的长度 `n`。`assert_eq!(n, 9)`。`write_tuple_to_data_page(&pool, &table, &vh, &buf[..n])` 得到 `RowId`。`with_page_data(PageId(row_id.page_id as u64), ...)` 里用 `get_slot_by_logical_id(row_id.slot_id)` 和 `get_slot_data_checked` 取原始字节。`gap = slot_data.len() - vh.to_bytes().len() - n`。断言和打印都用这个 `gap`。打印一行，字段序如上。

### T3: 把两处 0 写成保持原格式

- Requirement/Scenario: `proposal.md` 场景 2、4、5。D3 在可移除字节为 0 时保持现有读者能读的槽宽。
- Depends on: T2
- Targets: 本 Cycle 的 Act Response。不新增第三个测试。
- Current behavior: 执行记录里还没有这次页数、槽长度和保持结论。生产代码与 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 的 `src/` 相同。
- Required behavior: Act Response 抄下 T1 的 `index_pages`、`separator_len`、`leaf_len`、`removable` 和 T2 的 `header_len`、`tuple_tag`、`gap`、`slot_len`。写明内部节点槽保持 36 字节分隔条目，数据槽保持 22 字节版本头紧贴 tuple，不改格式。`git diff --name-only 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src` 只列出 `src/storage/btree/index_manager.rs` 和 `src/storage/data_page.rs`。这两份差异只增加上述测试，不改测试模块以外的函数。
- Preserve: `FORMAT_VERSION`、页大小、文件头长度、M03、M05、M18，以及 `index-write-side-atomicity` 里写单元、根发布、取消和回滚的现有合同。
- Forbidden: 在结论里写「改为瘦节点」或「合并 Tag」。修改 `Cargo.toml`。把 `.agents/memory/improvements.md` 的未提交内容纳入本次提交。创建 `2026-11-24-ms55-index-node-lock` 的 Cycle。
- First test: T1 与 T2 的两次命令已经退出码 0。没有第三条要先写红的测试。
- GREEN condition: Act Response 含两行测量和保持结论，并且上面的 `git diff --name-only` 只有那两个测试文件。
- Verification: 抄写时只使用这两次命令的实际打印。不重跑。然后运行 `git diff --name-only 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src`。出现第三个 `src/` 路径，或两行里有一个非 0 的 `removable` / `gap`，就不能写保持。
- Stop when: T1 或 T2 没有按各自命令通过；打印缺字段；或 `src/` 差异超出那两个测试。停止并回到计划。不在本 Cycle 设计新槽布局。

**Invariants**

Key 保持 32 字节。内部节点分隔槽保持 `Key` 加 4 字节子页号。叶槽保持 `Key` 加 `RowId`。数据槽前 22 字节保持 `VersionHeader`，tuple 的按值 tag 保持在值前面。文件格式版本保持 1。索引写单元仍在同一个 `spawn_blocking` 闭包里完成结构变更、根页发布和反向映射更新，并由 `write_gate` 串行。取消和回滚后的键释放合同不改。

**Acceptance**

- 场景 1：T1 的打印行含 `separator_len=36`、`leaf_len=38`、`removable=0`，且 `index_pages` 是一个不小于 2 的整数。
- 场景 2：T3 写明内部节点槽不改。对应 D4，没有短分隔键。
- 场景 3：T2 的打印行含 `header_len=22`、`tuple_tag=1`、`gap=0`。
- 场景 4：T3 写明数据槽不改。对应 D5，按值 tag 仍在 tuple 里。
- 场景 5：`git diff --name-only 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src` 只有两个测试文件。
- 场景 6：本 Cycle 不修改 `write_gate`，也不创建节点锁变更的 Iteration 目录。

**Verification**

1. `cargo test --offline --lib storage::btree::index_manager::tests::index_page_separator_has_no_removable_byte -- --test-threads=1 --nocapture`
2. 上一条退出码为 0 之后，再跑 `cargo test --offline --lib storage::data_page::tests::data_slot_prefix_is_version_header -- --test-threads=1 --nocapture`
3. 两条都退出码为 0 之后，`git diff --name-only 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src`

通过依据是两次测试退出码 0、两行打印的 `removable` 与 `gap` 都是 0，以及 `src` 差异只有上述两个文件。任一条测试失败就停，不把失败写成保持，也不重跑。不跑全量。

**Gate 2 Readiness**

PASS。计划批准来自本次 goal，检查项都已做完，不是跳过。计划审查里没有 TBD。没有未批准的需求裁剪。任务、`design.md` 和 `proposal.md` 使用同一套可移除字节。当前布局来自上述源码和规划时那一次宽度测试。三个任务的目标、断言和停止条件都已写明。调用入口是 `IndexManager::insert`、`collect_all_pages`、`write_tuple_to_data_page` 和原始槽读取。没有改到锁、刷盘或代价模型。Iteration 只有 000，三条任务都在其中。本 Cycle 自包含，实施不需要回读分析文档就能写下第一个测试。可移除字节大于 0 时停止，不留给实施去决定新槽宽。验证直接看槽字节和打印，没有身份证明或额外判定程序。没有 BLOCKED。没有 SKIPPED。

**Risks and Notes**

索引页数只抄 T1 的打印。不同的页数不构成重跑理由。规划时那条旧宽度测试已经通过，实施不重跑。`data_page` 在 `src/storage/mod.rs` 里是私有模块，库测试过滤名仍然是 `storage::data_page::tests::...`。`.agents/memory/improvements.md` 的工作区差异不属于本 Cycle。

## Act Response

Act 填写。计划交接时还没有实施结果。

## Plan Review

- Review Result: pending
