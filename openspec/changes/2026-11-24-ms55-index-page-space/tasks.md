# MS55 索引页空间的测量与保持

来源：`.agents/docs/tasks.md` 的 MS55，承接 I029 与 I030。I025 另立 `2026-11-24-ms55-index-node-lock`，本文件不给它派任务。约束：每条测试调用带 `--test-threads=1`，两条命令串行。不跑全量。

Gate 1：PASS。场景和范围在 `proposal.md`。批准，不是跳过。

Gate 2：PASS。执行依据在 `iterations/000-measure-and-keep/000-initial.md`。检查项都已做完。计划批准来自本次 goal，不是跳过。没有 BLOCKED，没有 SKIPPED。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。没有行为规格增量，`.openspec.yaml` 的 `skip_specs: true` 记录这个事实。
- 000：Plan Context 为 `ready`。Act 尚未开始。Review Result 为 `pending`。
- Persisted Evidence：none。
- 没有后续 Iteration。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 索引页测试打印页数和槽长度，可移除字节为 0 | 未完成 |
| T2 | 000 | T1 | 数据槽测试打印 gap 为 0，原始槽前 22 字节等于版本头 | 未完成 |
| T3 | 000 | T2 | 执行记录写明保持原格式，生产逻辑无差异 | 未完成 |

T1、T2、T3 的执行依据只在 `iterations/000-measure-and-keep/000-initial.md` 的 Task Definition。本文件不另写一套步骤。

## Iteration Plan

### Iteration 000: 测量可移除字节并保持现有槽格式

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: 200 个固定宽度字符串键上的索引页数、分隔槽 36 字节、叶槽 38 字节、内部节点可移除字节 0；单列 INT 槽的版本头 22 字节、gap 0。执行记录写明内部节点槽和数据槽都保持现有格式。`node.rs` 的条目宽度、`VersionHeader`、`serialize_tuple` 和 `FORMAT_VERSION` 与计划交接时相同。
- Verification boundary: 受影响回归集就是下面两条新库测试，按 Iteration 里的命令各跑一次，顺序执行。不跑全量。本变更不改生产逻辑，不触及跨模块公共面，也没有发布或迁移门槛。全量读不到执行记录里的保持结论。规划时已经跑过的 `string_key_uses_existing_tree_entry_width` 不重跑，除非 `src/storage/btree/` 里测试模块以外的代码相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 出现差异。
- Diagnostic boundary: 失败只看这两条测试、分隔槽宽度、叶槽宽度、`write_tuple_to_data_page` 拼出的槽字节，以及执行记录是否把 0 写成了保持。不把 `write_gate`、刷盘、io_uring 或代价模型纳入排查。
- Non-goals: 不改槽宽。不缩短 Key。不把按值 tag 并进版本头。不加节点锁。不展开 `2026-11-24-ms55-index-node-lock` 的 Cycle。不跑全量。

平衡：T1 和 T2 量的是两处槽，T3 把两处合成一个格式结论。只量一处就写「索引页格式保持」，另一处仍没有测量。拆成两轮会让下一变更依赖的格式结论停在半截。两处的失败都落在槽字节上，诊断范围相同。节点锁是另一个变更，不放进本轮。

## Requirements Traceability Matrix

要求来自 `proposal.md` 的场景和 `design.md` 的 D2、D3。没有行为规格增量，所以不映射到 `openspec/specs/`。

| Requirement | Task | First Test |
|---|---|---|
| 200 键树上分隔槽可移除字节为 0，并记下索引页数 | T1 | `index_page_separator_has_no_removable_byte` |
| 可移除字节为 0 时保持内部节点槽宽 | T3 | 同一测试通过后的执行记录 |
| 数据槽在版本头和 tuple 之间的字节为 0 | T2 | `data_slot_prefix_is_version_header` |
| gap 为 0 时保持 `VersionHeader` 与按值 tag | T3 | 同一测试通过后的执行记录 |
| 生产逻辑不改 | T3 | `git diff --name-only 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src` |

没有 Simplified。没有 Missing。
