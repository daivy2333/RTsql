# MS55 节点锁：判据未闭合

来源：`.agents/docs/tasks.md` 的 MS55，承接 I025。格式部分已由 `2026-11-24-ms55-index-page-space` 接受。本文件只规划锁这一支。

Gate 1：PASS。需求、场景和范围来自 MS55，以及 `proposal.md` 里由本次 goal 授权写入的选择。这是批准，不是跳过。

Gate 2：BLOCKED。写争用的可观察判据写不出来。正文在 `proposal.md` 的同名一节，决定和被替代方案在 `design.md` 的 D2。当前 Cycle 的 Plan Context 保持 `draft`。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。没有行为规格增量。`.openspec.yaml` 的 `skip_specs: true` 记录这个事实。
- 000：已展开 Cycle，未交给 Act。`iterations/000-contention-criterion/000-initial.md` 的 Plan Context 状态为 `draft`。Review Result 仍为 `pending`。
- Persisted Evidence：none。
- 没有加锁 Iteration。测量结论还不存在，不能预写「保持现有锁」或「加节点锁」的下一轮。

## 全部任务与归属

没有可执行任务。

采纳或保持需要一条能两头走的判据。该判据是实质未知项，不能留给 Act 现编比例或次数。因此不设 T1。

## Iteration Plan

### Iteration 000: 写下写争用判据，或停在写不出来

- Tasks: 无
- Depends on: 格式变更 Iteration 000 的 `accepted`。该结论已在 master 的归档 Cycle 里，本分支不合并它
- Stable result: 未达到。稳定结果本应是「保持现有 `write_gate`」或「加锁 Iteration 的前提已经写明」。两者都依赖尚未写出的判据
- Verification boundary: 无命令。本 Iteration 不改产品代码，不新增测试，不跑全量。全量读不到一份还不存在的判据
- Diagnostic boundary: 只核对判据是否仍然缺失，以及 `src/` 是否未被本变更修改。不排查刷盘、代价模型或槽格式
- Non-goals: 节点锁实现，latch coupling，Semaphore，索引页格式，Key 宽度，Tag 合并，脏页批量写，io_uring，代价模型，变长键

平衡：测量打印和加锁实现不能拆成两轮再开工。打印若没有事先写明的保持条件，数字没有归宿；把加锁放进同一轮，又是在判据出现之前改锁。合并这两件事仍然没有判据。所以本轮不设任务，也不为了凑一个可执行 Iteration 而加上「抄一段锁类型」这种不产生去留结论的任务。

没有第二个 Iteration。

## Requirements Traceability Matrix

要求来自 `proposal.md` 的场景。没有行为规格增量。

| Requirement | Task | First Test |
|---|---|---|
| 写争用被测量证实才加节点锁，未过门槛则保持现有锁 | Missing | Missing |
| 保持写单元、根发布、取消、回滚和同一索引的结构写串行 | Missing：没有改这些行为的任务 | 无。本批不改产品代码，也不新增测试 |
| 不改索引页格式 | Missing：没有改格式的任务 | 无。槽宽沿用已接受的格式结论 |

「写争用」这一行是 Gate 2 失败的原因。后两行没有任务，是因为本批不修改它们；这不是已批准的需求裁剪。没有 Simplified。
