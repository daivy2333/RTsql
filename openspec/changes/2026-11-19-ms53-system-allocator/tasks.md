# MS53 系统分配器的测量与保留

来源：`.agents/docs/tasks.md` 的 MS53，承接 I066。行锁不在本变更。约束：这次测试调用带 `--test-threads=1`。只跑下面这一条命令，不并入其他测试目标。

Gate 1：PASS。需求、场景和范围来自 MS53 的分配器部分，以及已经确定的保留结论。批准覆盖这条测量口径。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。没有行为规格增量，`.openspec.yaml` 的 `skip_specs: true` 记录这个事实。
- 000：已执行。Cycle 为 `iterations/000-measure-and-keep/000-initial.md`。Plan Context 状态为 `ready`。Act Response 状态为 `reported`。Review Result 为 `pending`。
- Persisted Evidence：none。
- 没有后续 Iteration。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | release 测试打印六个数，行和错误文本符合设计，产品代码不变 | 已完成 |
| T2 | 000 | T1 | 执行记录抄写六个整数，并写明保留系统默认分配器 | 已完成 |

T1 与 T2 的执行依据是 `iterations/000-measure-and-keep/000-initial.md` 的 Task Definition。本文件不另写一套步骤。

## Iteration Plan

### Iteration 000: 测量执行路径的分配请求并保留系统分配器

- Tasks: T1, T2
- Depends on: None
- Stable result: 一条非等值连接在解析、计划、执行上的六个整数，以及「保留系统默认分配器」。`src/` 和 `Cargo.toml` 与起点相同。
- Verification boundary: 本轮只跑 `cargo test --offline --release --test ms53_allocator_measure -- --test-threads=1 --nocapture` 一次。收尾验证是这一条受影响测试。不跑全量：本变更不改产品代码，不触及跨模块公共面，也没有发布或迁移门槛。全量读不到这六个打印字段。
- Diagnostic boundary: 失败只看这个测试的计划根节点、行、列名、错误文本、SQLSTATE，以及六个数是否大于 0。不把行锁、页 I/O、诊断阶段或文件格式纳入排查。
- Non-goals: 不改 `src/`。不引入 `jemalloc` 或 `mimalloc`。不测量行锁。不新增诊断指标。不展开下一轮。

平衡：打印和保留结论是同一个结果。只写测试而不写结论，下一轮没有可依赖的记录；只写结论而没有这一次打印，测量就没有发生。两者放在一轮。没有第二个可独立验收的故障域，所以不拆。

## Requirements Traceability Matrix

这里的要求来自 `proposal.md` 的场景和决定。没有行为规格增量，所以不映射到 `openspec/specs/`。

| Requirement | Task | First Test |
|---|---|---|
| 解析、计划、执行三段的分配次数和字节数可以打印 | T1 | `allocator_stage_counts_and_sql_results` |
| 同一条语句的 SQL 结果在阶段执行和 `execute_sql` 上保持一致 | T1 | `allocator_stage_counts_and_sql_results` |
| 缺失表的错误文本和 SQLSTATE 保持 | T1 | `allocator_stage_counts_and_sql_results` |
| 记录保留系统默认分配器，且不引入新的分配器依赖 | T2 | T1 那一次命令的输出 |

没有 Simplified。没有 Missing。
