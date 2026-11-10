# 无键行版本链进入现有表 GC

来源：MS46，承接 I038。约束：M10、M19。需求与场景在 `proposal.md`，做法在 `design.md`，规格增量在 `specs/keyless-row-gc/spec.md`。

Gate 1 为 PASS，来自本次 goal 的需求批准。Gate 2 的结论写在 `iterations/000-keyless-chain-reachability/000-initial.md` 的 Plan Context。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与规格增量：已写入。
- 000：已展开。`iterations/000-keyless-chain-reachability/000-initial.md` 的 Plan Context 为本次规划的当前 Cycle。Review Result 为 `pending`。
- 后续 Iteration：无。
- 跳过项：无。

000 的任务级执行依据以 Cycle 里的 Task Definition 为准。本文件与它冲突时，以 Cycle 为准。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 无键行旧版本按既有判定回收，当前版本和未提交链头留下；带键行合同、释放顺序和损坏数据页的现有错误保持 | 未开始 |

## 1. 无键行版本链进入 GC（Iteration 000）

- [ ] 1.1 T1：按 `iterations/000-keyless-chain-reachability/000-initial.md` 的 T1 Task Definition 执行。先写该定义里的新测试并看到规定的 RED 与既有 GREEN，再只改 `TableMeta::gc_table`。规格 delta 已在本变更里，实施时不改 `openspec/specs/` 的权威正文。

完成后：`specs/keyless-row-gc/spec.md` 的场景都有对应断言。带键行三个既有用例仍通过。

必须保持：索引循环的判定、`delete_tuple_from_data_page`、写路径、查询可见性、空闲页提升。

禁止：新增 GC 调用点、落盘登记、快照水位、身份证明机制或判定层；在本轮跑全量；并行执行多个测试目标；去掉 `--test-threads=1`。

Verification：下面两条逐个执行，每条都带 `--test-threads=1`。先整份 `gc_test`，结束后再跑溢出文件里名字含 `gc_releases` 的用例。

```text
cargo test --offline --test gc_test -- --test-threads=1
cargo test --offline --test overflow_page_test gc_releases -- --test-threads=1
```

两条都不是全量。本变更没有收尾全量任务。

停止条件：见 Cycle 的 T1 `Stop when`。

## Iteration Plan

### Iteration 000: 无键行旧版本按既有 GC 规则回收

- Tasks: T1
- Depends on: None
- 状态：已展开。Cycle 为 `iterations/000-keyless-chain-reachability/000-initial.md`。
- Stable result: 调用 `gc_table` 后，无键行的已提交旧版本按带键行同一判定回收，当前无键版本仍可查询；带键行的回收结果和溢出页释放顺序与现在相同。
- Verification boundary: 上面两条定向命令，逐个执行，每条 `--test-threads=1`。不跑全量，也不把全量排进后续 Iteration。跳过本变更收尾全量的理由是用户原话：「这次自动化把全量测试放到最后来限制全量的次数」。全量留到 MS46 与 MS47 都收尾合回主分支之后，按 `.agents/runbooks/m19-batched-full-regression.md` 分批串行，只跑一次。风险：本变更收尾前没有全量；合回后那一次失败时，排查范围包含 MS46 与尚未开始的 MS47。
- Diagnostic boundary: `TableMeta::gc_table` 的索引一遍、无键起点选择和同一删除判定。带键三个旧用例失败时，先看索引循环是否被改写。无键当前行消失时，先看起点有没有被删。已删除带键行或回滚后的带键行消失时，先看无键起点是否把墓碑或 `create_tx_id == 0` 的槽算了进去。溢出页在提升前就能分配到时，先看是否绕过了 `delete_tuple_from_data_page`。
- Non-goals: 提高 GC 速度；改变查询的 MVCC 可见性；索引格式、空闲页生命周期、刷盘；节点级锁、分配器、clone 消除；新增 GC 调用点；让 `UPDATE` 能定位已有无键行；MS47；本变更内的全量。

## 平衡审计

只有这一轮。回收覆盖、带键合同和释放顺序是同一次 `gc_table` 改动的同一验收，拆开后前一轮不能单独声称带键结果未变。没有第二个可独立交付的结果，所以不再拆 Iteration，也不把全量做成后一轮。这一轮不是单项观察：它同时交付无键链可达、链头保留、带键链不扩大，以及损坏页的既有错误。
