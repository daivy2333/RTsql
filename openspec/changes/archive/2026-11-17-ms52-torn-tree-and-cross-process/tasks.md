# MS52 撕裂树与跨进程访问的去留

来源：`.agents/docs/tasks.md` 的 MS52，承接 I031 和 I084。约束：每条 `cargo test` 带 `--test-threads=1`，全量按 `.agents/runbooks/m19-batched-full-regression.md` 分批串行。整个目标只预算一次全量，放在 Iteration 001。返工不再预算第二次。

Gate 1：PASS。需求、场景和范围来自 MS52，测量结果已写入 `proposal.md`。这是批准，不是跳过。没有行为规格增量，`.openspec.yaml` 的 `skip_specs: true` 记录这个事实。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。
- 000：完成。Cycle 为 `iterations/000-record-and-keep/000-initial.md`。Plan Context 为 ready。Act Response 为 reported，Review Result 为 `accepted`。
- 001：完成。Cycle 为 `iterations/001-batched-full/000-initial.md`。Plan Context 为 ready。Act Response 为 reported，Review Result 为 `accepted`。
- Persisted Evidence：none。
- 跳过项：无。不建第二个变更，因为两项都不进入产品实现。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 恢复数字和「不改刷盘、不改驱逐」写入执行记录，`src/` 与 `tests/` 无差异 | 已完成 |
| T2 | 000 | 无 | 锁等待数字和「保持文件锁」写入执行记录，`src/` 与 `tests/` 无差异 | 已完成 |
| T3 | 001 | T1、T2 | 一次 M19 分批全量，各批 failed 为 0 | 已完成 |

T1 与 T2 的执行依据在 `iterations/000-record-and-keep/000-initial.md`。T3 的执行依据在 `iterations/001-batched-full/000-initial.md`。本文件不另写一套步骤。

## Iteration Plan

### Iteration 000: 记录测量并维持刷盘、驱逐和文件锁

- Tasks: T1, T2
- Depends on: None
- Stable result: 恢复代价和锁等待有一次可引用的结果。刷盘、驱逐、恢复门控和文件锁保持 `0019bf1` 的代码。
- Verification boundary: 本轮不跑 `cargo test`。沿用规划时已经跑过的 release 计时和 CLI 锁等待。Act 只核对 `src/`、`tests/`、`Cargo.toml` 相对 `0019bf1` 没有差异。不跑全量。
- Diagnostic boundary: 差异若出现，只看是不是有人改了缓冲池、恢复或文件锁。不把全量套件、批量写或 io_uring 纳入本轮排查。
- Non-goals: 不实现结构感知刷盘、no-steal、每次提交的全量检查点、索引增量重放、跨进程快照。不展开 Iteration 001。不建第二个变更。

平衡：测量和「保持代码」是同一个结果。拆成刷盘一轮、文件锁一轮会把同一份「没有产品差异」切成两段排查。全量是另一套故障域，留在 001。

### Iteration 001: 一次分批全量

- Tasks: T3
- Depends on: Iteration 000 的 Review Result 为 `accepted`
- Stable result: 在没有产品改动的树上，现有测试按 M19 分批全部 failed 为 0。这次全量是整个目标里唯一的一次。
- Verification boundary: 按 runbook 的等分规则，用执行时的 `ls tests/*.rs | sort` 切批。计划时文件数是 169，按该规则为 15 个文件批（11 批各 11 个文件，4 批各 12 个文件）再加单独的 `--lib --bins`，共 16 批。执行时文件数变了就按现场重算，不沿用 169。每批 `--test-threads=1`，批与批串行。禁止一次无过滤的 `cargo test`，禁止默认并行度。返工不再跑第二遍全量。某一批失败时，可以把那一批再跑一次以区分偶发失败；再失败就停止，不重开整次分批。
- Diagnostic boundary: 失败目标自己的断言。缓冲池、恢复和文件锁没有本变更的改动，不在本轮里修。
- Non-goals: 不因为全量失败去改刷盘、驱逐、恢复或文件锁。不预建修复轮次。

平衡：全量不证明新的产品行为，它确认这棵没有产品差异的树仍能通过现有套件。它和 000 的计时不是同一个故障域，所以单独成轮。工作量是整次分批，不再往下拆。

## Requirements Traceability Matrix

这里的要求来自 `proposal.md` 的场景和决定。没有行为规格增量，所以不映射到 `openspec/specs/`。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 一万行和四万行的不洁重开有墙钟和行数，检查点后的打开更快，刷盘和驱逐保持原代码 | T1 | 规划时的 release 计时；Act 核对 `git diff 0019bf1 -- src tests Cargo.toml` 为空 | Covered |
| 持锁时只读 CLI 退出码 4，放锁后读到行，文件锁保持原代码 | T2 | 规划时的 CLI 锁等待；同一条 diff 为空 | Covered |
| 整个目标只做一次分批全量 | T3 | Iteration 001 展开后的分批 `cargo test` | Covered |

没有 Simplified。没有 Missing。

## T3: 跑一次分批全量

本任务的执行依据是 `iterations/001-batched-full/000-initial.md`。000 的 Cycle 不执行本任务。下面的条目保留归属和通过依据，命令细节以该文件为准。

- Requirement/Scenario: `proposal.md` 的维持合同，加上本文件 Iteration 001 的验证边界。全量不产生新的产品行为。
- Depends on: T1 和 T2 已记入执行记录，且 000 的 Review Result 为 `accepted`。
- Targets: 不修改源码。命令对象是当时工作区里的 `tests/*.rs`、`--lib` 和 `--bins`。
- Current behavior: `0019bf1` 上的测试套件尚未在本变更里跑过全量。
- Required behavior: 每一批退出码为 0，且该批 `test result:` 行的 failed 为 0。缺一批就不能声明全量通过。
- Preserve: `src/` 和 `tests/` 相对 000 结束时的内容不变。`--test-threads=1`。批间不并行。
- Forbidden: 无过滤的 `cargo test`；默认并行度；第二遍全量；为了让某批通过去改产品代码；把判定写成单独的脚本。
- First test: 不新写测试。第一批就是 `cargo test --lib --bins -- --test-threads=1`。
- GREEN condition: 全部批次都满足 Required behavior。
- Verification: 文件批用 `cargo test --test <按字典序的一批文件名> -- --test-threads=1`。计数用 runbook 里的一步 `grep -E "^test result:"`。失败的含义是现有套件在这棵没有产品改动的树上没有通过。
- Stop when: 同一批第二次仍失败，或失败要求修改缓冲池、恢复、文件锁或其他产品代码。停止并报告失败目标。不在本任务里开修复轮次。
