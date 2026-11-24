## Why

里程碑 MS55（`.agents/docs/tasks.md`）要求索引页的空间和锁粒度有测量。瘦内部节点或 Tag 合并只有在空间收益盖过格式兼容成本时才改格式。节点锁只有在写争用被测量证实后才加，并保留写单元原子性、根发布、取消和回滚。未过门槛的保持原格式和现有锁。

来源是 I029 与 I030。I025 的节点锁不能和格式结论用同一次恢复测试加上同一次并发测试来判断，所以另立变更 `2026-11-24-ms55-index-node-lock`，依赖本次记下的格式结论。本次不测量写争用，也不加节点锁。

不沿用变长键假设。M03 仍要求 B-Tree Key 固定 32 字节。已归档的 `openspec/changes/archive/2026-10-27-ms38-index-key-encoding-and-string-index/` 已保留 fixed32，不采用 ordered-variable 与 prefix-restart。本次不重开那两项。

本次 goal 批准的是 MS55 里格式这一支的范围。下面的默认假设由这次 goal 授权接受。

## What Changes

- 测量两处槽布局，并据此决定保持原格式，或在可移除字节不为 0 时停下来交给下一轮计划。本轮不改槽宽、不改 `VersionHeader`、不改按值 tag、不改文件格式版本。
- 内部节点：在与 `string_key_uses_existing_tree_entry_width` 相同的 200 个可放入 32 字节的字符串键上，读根页第一枚分隔槽和最左子叶的第一枚叶槽，并调用 `collect_all_pages`。可移除字节 = 分隔槽长度 − 32 − 4。页数写入执行记录，不作为门槛。
- 数据槽：用 `serialize_tuple` 序列化一个 `Value::Int(7)`，经 `write_tuple_to_data_page` 写入，再按 logical id 读原始槽字节。可移除字节 = 槽长 − `VersionHeader::SIZE` − tuple 字节数。按值 tag 算在 tuple 里面，不算可移除的槽 tag。
- 两处可移除字节都是 0：执行记录写明保持内部节点槽和数据槽的现有格式。生产逻辑与起点相同。
- 任一处可移除字节大于 0，或根页不是内部节点、槽字节对不上：测试失败或实施停止。不在本轮设计新布局，也不把按值 tag 并进版本头。

### 场景草图

1. 内部节点空间。前置：`IndexManager::new` 经由 `make_manager`，插入 200 个 `Value::String(format!("s{i:04}"))` 的二级索引键，列类型 `ColumnType::String(255)`。动作：读根页、根页第一枚分隔槽、最左子页第一枚叶槽，并 `collect_all_pages`。结果：根页首字节是 `0x02`，分隔槽长度 36，叶槽长度 38，可移除字节是 0，索引页数至少 2。测试打印 `index_pages`、`separator_len`、`leaf_len`、`removable`。失败边界：根页不是内部节点，槽长度读不出来，或页数小于 2。测试失败。不得把失败记成保持，也不得改 `insert_separator` 的条目宽度。
2. 保持内部节点格式。前置：场景 1 的可移除字节是 0。动作：结束 I029 的格式改动。结果：分隔条目仍是 32 字节 Key 加 4 字节子页号。`FORMAT_VERSION` 仍是 1。失败边界：可移除字节大于 0。停止，本轮不设计更短的分隔键。
3. 数据槽前缀。前置：`data_page::tests::setup` 的单列 INT 表。动作：`serialize_tuple` 写入 `Value::Int(7)`，`write_tuple_to_data_page` 落槽，再用 `with_page_data` 按 logical id 读原始槽。结果：槽从偏移 0 起等于该 `VersionHeader::to_bytes()`，长度 22；紧接着的字节等于序列化后的 tuple，首字节是 `TAG_INT`（`0x01`）；头和 tuple 之间的字节数是 0。测试打印 `header_len`、`tuple_tag`、`gap`、`slot_len`。失败边界：槽短于 22 字节，或前 22 字节与 `to_bytes()` 不一致。测试失败。不得把失败记成保持。
4. 保持数据槽格式。前置：场景 3 的 gap 是 0。动作：结束 I030 的格式改动。结果：`VersionHeader::SIZE` 仍是 22。tuple 仍按 M05 在每个值前面带 tag。失败边界：gap 大于 0。停止。不把按值 tag 并进版本头，也不改页头里那 3 字节填充。
5. 两条都保持。前置：场景 2 和场景 4 都成立。动作：把打印的页数、槽长度和「保持原格式」写入执行记录。结果：`insert_separator`、`VersionHeader`、`serialize_tuple`、`FORMAT_VERSION` 的实现与起点相同。失败边界：为了写结论而修改这些实现。
6. 节点锁留在另一个变更。前置：本次格式结论尚未被后续计划拿去测量争用。动作：不改 `write_gate`，不新增节点锁。结果：锁的测量与可能的加锁只出现在 `2026-11-24-ms55-index-node-lock`，并且要等本次执行记录里的格式结论。失败边界：在本变更里加锁，或在格式结论写下来之前展开那个变更的 Cycle。

### 本次 goal 授权接受的默认假设

1. 格式与节点锁分成两个变更。本次只做 I029 与 I030 的空间测量。I025 依赖本次格式结论，本批不展开它的 Iteration。
2. 空间收益是可移除字节，不是一个百分比。内部节点的可移除字节是分隔槽里、32 字节 Key 和 4 字节子页号之外的字节。数据槽的可移除字节是版本头和 tuple 之间的字节。按值 tag 属于 tuple。页头 3 字节填充属于页，不属于 slot。
3. 兼容成本是现有读者还要能读当前构建写出的库：内部槽按 36 字节解析，数据槽前 22 字节按 `VersionHeader` 解析，文件格式版本保持 1。可移除字节为 0 时，没有字节收益去支付另一套读者。保持现有槽宽。
4. 索引页数只记入执行记录。它不决定改不改格式。
5. 可移除字节大于 0 时，本轮停止并回报。不在同一个 Cycle 里发明新布局或兼容读者。
6. 更短的分隔键、ordered-variable、prefix-restart 不在本次候选里。它们会改变 M03 的 32 字节键，也重开 MS38 已经关闭的编码选择。

### 已经选定的行为

- 本轮的稳定结果是两处格式保持，或一处测量与代码不一致时停住。生产上的 SQL 结果、错误文本、页格式和文件头保持现状。
- 不写行为规格增量。`.openspec.yaml` 使用 `skip_specs: true`。保持原格式不新增对外合同。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- 无。索引条目宽度、`VersionHeader` 和 tuple tag 的现有合同保持。

## Impact

- 代码：本轮只在 `src/storage/btree/index_manager.rs` 与 `src/storage/data_page.rs` 的测试模块里各加一个测试。不改 `node.rs` 的条目宽度，不改 `version_chain.rs`，不改 `tuple.rs`，不改 `file_header.rs`，不改 `write_gate`。
- 引用：MS55、I029、I030、M03、M05、M18、M19。M18 的恢复不变量不改。M19 要求测试命令带 `--test-threads=1`。全量不跑：本变更不改生产逻辑，不触及跨模块公共面，也没有发布或迁移门槛。
- 测试：两条 release 无关的库测试，顺序执行。索引这一条带 `--nocapture`，以便抄下页数。
- 数据：无格式变更，无迁移。明文页 4096 字节，文件头 64 字节，格式版本 1，均保持。

## 回滚

没有格式迁移。去掉这两条测试和执行记录里的测量，就没有这次保持结论。已有库文件不用迁移。

## Gate 1

PASS。需求、场景和范围来自 MS55 的 Outcome、Scope、Non-goals、Split signals 和 Consumes，以及上面由本次 goal 授权接受的默认假设。这是批准，不是跳过。没有 SKIPPED 项。
