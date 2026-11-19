# MS53 行锁的测量与保留

来源：`.agents/docs/tasks.md` 的 MS53，承接 I050。复制和分配器已经分别归档。约束：行锁测试调用带 `--test-threads=1`。计划调查已经跑过下面这一条命令，实施沿用，不重跑，也不并入其他测试目标。

Gate 1：PASS。需求、场景和范围来自 MS53 的行锁部分，以及已经确定的保留结论。批准覆盖这条测量口径。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。没有行为规格增量，`.openspec.yaml` 的 `skip_specs: true` 记录这个事实。
- 000：待执行。Cycle 为 `iterations/000-measure-and-keep/000-initial.md`。Plan Context 状态为 `ready`。
- Persisted Evidence：none。
- 没有后续 Iteration。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 执行记录写明持锁结构、调用点和基点上那一次测试输出 | 待执行 |
| T2 | 000 | T1 | 同一节写明保留现有 RowLockTable，且产品代码不变 | 待执行 |

T1 与 T2 的执行依据是 `iterations/000-measure-and-keep/000-initial.md` 的 Task Definition。本文件不另写一套步骤。

## Iteration Plan

### Iteration 000: 测量行锁持有结构并保留现有 RowLockTable

- Tasks: T1, T2
- Depends on: None
- Stable result: `RowLockTable::get_lock` 的持锁结构、三个既有测试在基点上的通过结果，以及「保留现有 RowLockTable，不迁到 DashMap」。`src/`、`Cargo.toml` 和 `tests/` 与起点相同。
- Verification boundary: 收尾验证沿用计划调查时的这一次 `cargo test --offline --lib transaction::row_lock::tests -- --test-threads=1`。覆盖范围是这三个测试和 `get_lock` 的现有代码。实施不重跑。不跑全量：本变更不改产品代码，不触及跨模块公共面，也没有发布或迁移门槛。全量读不到执行记录里的持锁结构。
- Diagnostic boundary: 失败只看 `src/` 是否仍与基点一致，执行记录是否抄对那一次输出，以及 `src/`、`Cargo.toml`、`tests/` 是否出现差异。不把节点锁、页 I/O、分配器或嵌套循环纳入排查。
- Non-goals: 不迁到 DashMap。不新增测试，也不新增 SQL 写路径。不改可见性和 `DatabaseLocked`。不改已经交付的复制和分配器。不展开下一轮。

平衡：结构和测试结果是同一份测量。只写保留结论而没有这次输出，测量就没有进入记录；只抄输出而不写保留，本轮没有可依赖的结论。两者放在一轮。没有第二个可独立验收的故障域，所以不拆。

## Requirements Traceability Matrix

这里的要求来自 `proposal.md` 的场景和决定。没有行为规格增量，所以不映射到 `openspec/specs/`。

| Requirement | Task | First Test |
|---|---|---|
| 记下 `locks` 的类型和 `get_lock` 的加锁路径 | T1 | `transaction::row_lock::tests` 在基点上的那一次运行 |
| 记下三个既有测试通过，且 40ms、20ms 不是换结构的门槛 | T1 | `transaction::row_lock::tests` 在基点上的那一次运行 |
| 记下调用点只有这三个测试 | T1 | 同一次运行；调用点以计划对 `*.rs` 的检索为准，实施只核对 `src/` 相对基点未变 |
| 记录保留现有 RowLockTable，且不改产品代码 | T2 | T1 的记录，以及 `git diff --name-only HEAD -- src Cargo.toml tests` |

没有 Simplified。没有 Missing。
