# MS46 无键行版本链进入 GC

来源：MS46，承接 I038。约束：M10、M19。需求与场景在 `proposal.md`，做法在 `design.md`，规格增量在 `specs/row-version-gc/spec.md`。

Gate 1 为 PASS，来自本次 goal 的需求批准。Gate 2 的结论写在 `iterations/000-keyless-chain-reachability/000-initial.md` 的 Plan Context。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与规格增量：已写入。
- 000：已展开。`iterations/000-keyless-chain-reachability/000-initial.md` 的 Plan Context 状态以该文件为准。Act Response 为 `reported`。Review Result 为 `pending`。
- 后续 Iteration：无。
- 跳过项：无。

000 的任务级执行依据以 Cycle 里的 Task Definition 为准。本文件与它冲突时，以 Cycle 为准。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 无键链的已提交旧版本被 `gc_table` 回收，链头与未提交版本留下，`SELECT` 仍读到链头；带键回收结果和带键墓碑链保持；损坏页返回现有错误 | 已实施，待 Review |

## 1. 无键链进入 gc_table（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-keyless-chain-reachability/000-initial.md` 的 T1 Task Definition 执行。先在 `tests/gc_test.rs` 写新用例并看到规定的 RED，再改 `TableMeta::gc_table`，并把 `DataScanExecutor::next` 的替代判断改为使用槽的 `logical_id`。规格 delta 已在本变更里，实施时不改 `openspec/specs/` 的权威正文。

当前行为：`gc_table` 只遍历主键索引。无键链的清理计数是 0，旧槽还在。带键三则用例已在基点通过。

完成后：proposal 场景 S1–S11 的可观察结果成立。既有三则带键用例的源码断言不改。`gc_releases_oversized_version_chain` 的源码不改。

必须保持：第一段 `scan_all` 的判定和 `delete_tuple_from_data_page`；带键墓碑链不回收；空闲页提升时机；`superseder_suppresses` 的可见性规则。

禁止：用数据页扫描取代第一段；改 `delete_tuple_from_data_page`；改 `superseder_suppresses` 或 map 的键；新增 SQL GC；改索引格式或空闲页合同；在本轮跑全量；用默认并行度跑测试。

先行测试与验证命令见 Cycle 的 T1。三条命令逐个执行，都带 `--test-threads=1`。

停止条件：见 Cycle 的 T1 `Stop when`。

## Iteration Plan

### Iteration 000: 无键链按现有判定被 GC 看见

- Tasks: T1
- Depends on: None
- 状态：Act Response 为 reported，Review Result 仍为 pending。
- Stable result: 调用 `gc_table` 后，无键行的已提交旧版本被回收，链头和未提交版本仍可读；带键行的最新版本和已提交删除留下的墓碑链与改前相同；损坏数据页返回 `corrupted data page` 错误。
- Verification boundary: T1 的三条定向命令，逐个执行，每条都带 `--offline -- --test-threads=1`。本 Iteration 不跑全量，本 change 也不另设收尾全量 Iteration。跳过本 change 收尾全量的理由是用户原话：「这次自动化把全量测试放到最后来限制全量的次数」。风险：本 change 收尾前没有全量；合回后那一次失败时，排查范围包含 MS46 与 MS47。合回主分支之后的那一次按 `.agents/runbooks/m19-batched-full-regression.md` 分批串行，不在本 change 的任务里。
- Diagnostic boundary: `TableMeta::gc_table` 的两段枚举，`DataScanExecutor::next` 用来查替代 map 的行身份，以及 `tests/gc_test.rs` 的新断言。带键三则失败时，先看第一段判定有没有被改写。没有删过槽的无键查询失败时，先看 `logical_id` 改动是否在未删槽的页上改变了结果。溢出页在提升前就能分配到，才去看 `delete_tuple_from_data_page`，那已经超出本 Iteration 的允许修改。
- Non-goals: GC 速度、MVCC 可见性规则、索引格式、空闲页生命周期、刷盘、节点锁、分配器、clone 消除、SQL GC 语句、让 DELETE 找到无键行、全量回归。

## 平衡审计

000 只有一个可交付结果：`gc_table` 看见无键链，并用现有判定回收，回收后 `SELECT` 仍读到链头。链头保留、未提交版本、带键墓碑、混表、二级索引、溢出释放和损坏页都靠这次回收成立。扫描改用 `logical_id` 是删槽之后链头仍能被 `SELECT` 读到的必要条件，单独拆出去没有可以交付的回收结果。拆成「先扫描链头」和「再删除」两轮，中间也没有可以单独验收的稳定结果。

不设收尾全量 Iteration。全量被用户明确放到 MS46 与 MS47 都合回主分支之后，只跑一次。把它写成 001 会在本 change 收尾时多跑一轮，直接违反这条限制。

没有预建「修复尚未发现的缺陷」的 Iteration。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 无键链的已提交旧版本被回收（整数键改为 NULL） | T1 | `gc_collects_committed_ancestors_of_keyless_head` | Covered |
| 无键链的已提交旧版本被回收（与带键链同表） | T1 | `gc_collects_keyed_and_keyless_chains_on_one_table` | Covered |
| 无键链的已提交旧版本被回收（超页前驱的释放时机） | T1 | `gc_frees_overflow_pages_of_keyless_ancestor` | Covered |
| 链头与未提交版本不回收（只有 NULL 键一行） | T1 | `gc_keeps_single_null_key_version` | Covered |
| 链头与未提交版本不回收（非整数键列一行） | T1 | `gc_keeps_single_string_key_version` | Covered |
| 链头与未提交版本不回收（未提交链头） | T1 | `gc_keeps_uncommitted_keyless_head` | Covered |
| 链头与未提交版本不回收（二级索引仍找到链头） | T1 | `gc_secondary_index_finds_keyless_head` | Covered |
| 带键行的回收结果不变（三次已提交版本） | T1 | `test_gc_removes_old_versions` | Covered |
| 带键行的回收结果不变（未提交带键更新） | T1 | `test_gc_preserves_uncommitted_versions` | Covered |
| 带键行的回收结果不变（三条带键链） | T1 | `test_gc_multiple_keys` | Covered |
| 带键行的回收结果不变（已提交删除） | T1 | `gc_does_not_collect_keyed_delete_chain` | Covered |
| 损坏数据页按现有读路径报错（页头槽数） | T1 | `gc_corrupt_slot_count_returns_named_error` | Covered |
| 损坏数据页按现有读路径报错（槽偏移） | T1 | `gc_corrupt_slot_offset_returns_named_error` | Covered |
