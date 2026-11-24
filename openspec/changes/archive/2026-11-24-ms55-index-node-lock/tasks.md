# MS55 节点锁：保持现有 write_gate

来源：`.agents/docs/tasks.md` 的 MS55，承接 I025。格式部分已由 `2026-11-24-ms55-index-page-space` 接受。本文件只规划锁这一支。

Gate 1：PASS。需求、场景和范围来自 MS55，以及 `proposal.md` 里由本次 goal 授权写入的选择。这是批准，不是跳过。没有需求裁剪。

Gate 2：`000-initial.md` 仍是 BLOCKED，Plan Context 保持 `draft`，不改写。`001-replan.md` 为 PASS，Plan Context 为 `ready`。决定和被替代方案在 `design.md` 的 D4。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。没有行为规格增量。`.openspec.yaml` 的 `skip_specs: true` 记录这个事实。
- 000 的 `000-initial.md`：Plan Context 状态为 `draft`。Gate 2 为 BLOCKED。Plan Context 不改写。Review Result 为 `replan-required`。Next Cycle 为 `001-replan.md`。
- 000 的 `001-replan.md`：Plan Context 状态为 `ready`。Gate 2 为 PASS。任务是 T1。Review Result 为 `accepted`。Next Cycle 与 Next Iteration 均为 None。
- Persisted Evidence：none。
- 没有加锁 Iteration。

## 全部任务与归属

### T1：保持现有 write_gate，产品代码停在起点

归属 Iteration 000。执行依据在 `iterations/000-contention-criterion/001-replan.md` 的 Task Definition。

结果：`src`、`tests`、`Cargo.toml` 相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 无差异。每个 `IndexManager` 仍有一把 `write_gate: Arc<Mutex<()>>`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 仍在 `spawn_blocking` 里取得它。`search`、`scan_all`、`scan_range` 仍不取得它。不加节点锁，不新写耗时比例或等待次数。

## Iteration Plan

### Iteration 000: 确认未过加锁门槛，保持现有 write_gate

- Tasks: T1
- Depends on: 格式变更 Iteration 000 的 `accepted`。该结论已在 master 的归档 Cycle 里，本分支不合并它
- Stable result: 产品代码相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 无差异。现有 `write_gate` 保持。不加节点锁
- Verification boundary: `git diff --exit-code 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml` 退出码为 0，并阅读 `index_manager.rs` 里上列符号。不新增测试，不跑全量，不跑 `tests/index_write_atomicity_test.rs`。本 Iteration 不改产品代码，受影响回归集为空。全量套件不预算：没有跨模块公共面的代码改动，未到发布或迁移门槛，也没有人要求全量
- Diagnostic boundary: diff 非空时只看 `src`、`tests`、`Cargo.toml` 里多出来的路径。符号与计划不符时只看 `index_manager.rs` 的 `write_gate` 取得点。不排查刷盘、代价模型或槽格式
- Non-goals: 节点锁，latch coupling，Semaphore，耗时比例，等待次数，暂停夹具，身份证明，索引页格式，Key 宽度，Tag 合并，脏页批量写，io_uring，代价模型，变长键

目录名保持 `iterations/000-contention-criterion/`。本 replan 不另建 Iteration 目录。

平衡：这一轮只有一个稳定结果，就是锁保持现状。测量打印和加锁实现都不进本轮。拆开「看 diff」和「看符号」得不到两个可独立验收的结果。没有第二个 Iteration。

## Requirements Traceability Matrix

要求来自 MS55 的 Outcome，以及 `proposal.md` 的场景。没有行为规格增量。没有 Simplified。

| Requirement | Task | First Test |
|---|---|---|
| 未过门槛则保持现有锁，不加节点锁 | T1 | 变更前 `git diff --exit-code 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml` 退出码为 0，并阅读 `IndexManager.write_gate`。不新增测试 |
| 保持写单元、根发布、取消、回滚，以及同一索引的结构写串行 | T1 | 同一条 diff。阅读 `insert`、`delete`、`delete_exact`、`update` 的 `spawn_blocking` 闭包 |
| `search`、`scan_all`、`scan_range` 不取得 `write_gate` | T1 | 同一条 diff。这三处源码没有 `write_gate.lock` |
| 不改索引页格式 | T1 | 同一条 diff。`src` 为空即覆盖槽宽与 `FORMAT_VERSION` |
