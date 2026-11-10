# Iteration 001 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 需求 5 与场景 S10；tasks.md 里 T2 的分批规则；M19 的分批和 `--test-threads=1`；Iteration 000 已接受的通道实现
- Excluded scope: 修改产品代码；第二次全量；非默认 feature；新平台；拆 crate；替换 `Notify`；检查点或缓冲池的新通道；I093、I095、诊断上报、SQL 语义与性能；预建缺陷轮次

**Objective**

在默认构建上按 T2 的分批规则跑完一次全量。每一批退出码为 0，且 failed 为 0。`--test` 名字与执行时枚举的集成测试文件一致，无缺失、无多余、无重复。整个变更只此一次全量。某一批失败时只把该批再跑一次，这不算第二次全量。

**Investigation Facts**

- Starting point: Iteration 000 的 Review Result 为 accepted。产品实现停在 `01fd5f4f4142fb77a0b52a8ea5df1932652d8e51` 的 `src/wal/buffer.rs`：记录和待提交事务号走两条无界通道，`Cargo.toml` 仍是单个 package。该轮 Act Response「验证」节的五条 GREEN 为退出码 0、failed 为 0：`wal::buffer::tests` 9 passed，`wal_buffer_test` 5 passed，`session_lifecycle_test` 7 passed，`commit_wait_classes_present` 1 passed，`cancel_leaves_flush_loop_running` 1 passed。两条 diff 无输出。覆盖范围在该 Review 时未变。本 Cycle 不重跑这五条，也不把它们当成另一次全量。
- Current-State Evidence:
  - 集成测试目标是仓库根目录下 `tests/*.rs` 的顶层文件。Cargo 没有 `[[test]]`，没有关闭自动发现。`--test` 的名字是文件名去掉末尾 `.rs`。
  - `tests/fixtures/` 与 `tests/support/index_key_encoding.rs` 不是独立的 `--test` 目标。`ls tests/*.rs` 不会列出它们。
  - `Cargo.toml` 只有一个 `[package]`，名为 `rtsql`，没有 `[workspace]`。`[[bin]]` 只有 `rtsql`。同文件里的 `[[bench]]` 不进入本轮命令。
  - 第一批 `cargo test --offline --lib --bins` 跑库测试和 `rtsql` 二进制的测试，不跑 `tests/*.rs`。后面的每一批用若干个 `--test` 选择集成测试。`--` 之后的 `--test-threads=1` 传给该次调用里的测试进程。
  - 复核时 `ls tests/*.rs | sort` 得到 164 个文件。按 T2 计算，B = 14，K = 4，K 小于 B。前 4 批各 11 个文件，后 10 批各 12 个文件，都在 8 到 12 之间。这只说明当时规则可以执行。执行时必须重新枚举，164 不是命令输入。
- Code and Critical Path: 本 Cycle 不改产品符号。入口是仓库根目录的 `cargo test`。第一批过滤 `--lib --bins`。集成测试批过滤 `--test`。失败时按失败批和其中的测试目标排查。分批规则本身不因失败而改写。

**Implementation Guidance**

先枚举并切分。停止条件成立时，不要启动任何 `cargo test`。一批的命令结束后再开始下一批。已经通过的批不要重跑。失败批只把同一条命令再跑一次。

**Task Definitions**

### T2: 按 M19 分批跑一次全量

- Requirement/Scenario: proposal 需求 5，场景 S10。
- Depends on: Iteration 000 accepted
- Targets: 无产品符号。命令在仓库根目录执行。集成测试文件来自 `ls tests/*.rs | sort`。库和二进制测试只出现在第一批。
- Current behavior: 通道改动已在 Iteration 000 完成，定向测试已通过。全量还没有在本变更里跑过。
- Required behavior:
  1. 执行时枚举，不把规划时的文件数写死。集成测试文件来自 `ls tests/*.rs | sort`。下面记这个数量为 N。切分和对账都用这一份名单，不要再排序一次。
  2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
  3. N 为 0 时没有集成测试批。N 大于 0 时，B 是不小于 N / 12 的最小整数，K = B × 12 − N。K 小于等于 B 时，按该排序前 K 批各 11 个文件，其余 B − K 批各 12 个。文件名去掉末尾的 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> ... -- --test-threads=1`。
  4. K 大于 B，或任一集成测试批的文件数不在 8 到 12 之间时，停止并回到计划。不另造切分，不把多批合成一次无过滤的 `cargo test`。
  5. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib`、`--bins` 或 `--test` 过滤的 `cargo test`。禁止任何测试调用使用默认并行度。
  6. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
  7. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
  8. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败。不改并行度，不重跑已经通过的批，不因此再跑一遍全量。第一次失败且复核通过时，Act Response 写明该批第一次失败。
- Preserve: Iteration 000 的通道实现和单 package。默认 feature `cli`、`network`、`crypto`。M19 的分批顺序和 `--test-threads=1`。
- Forbidden: 修改 `src/`、`tests/`、`Cargo.toml` 或其他产品文件。增加 `--benches`、`--examples`、`--doc`、`--all-targets` 或第二条全量。去掉 `--offline` 或 `--test-threads=1`。用默认并行度。重跑已通过的批。把 164 写进命令。新增判定脚本、封装命令或统计程序。跑 Iteration 000 的五条定向命令来代替或附加这次全量。
- First test: 本任务不改产品代码，没有新的 RED。变更前的定向结果沿用 Iteration 000 Act Response 的「验证」节，五条都是退出码 0 且 failed 为 0。不要重跑那五条。
- GREEN condition: 第 1 到第 8 条都做到。每一批最终退出码为 0 且 failed 为 0。对账通过。全量只跑了这一次；失败批的那一次复核除外，复核只重复失败的那一条命令。
- Verification: 判定就是上述 `cargo test` 命令自己的退出码，以及该次输出里的 failed 计数。通过条件是退出码 0 且 failed 为 0。失败表示该批未过；第二次仍失败表示 T2 未完成。S10 的目标集合用同一次枚举对账。不新增判定层。
- Stop when: K 大于 B，或某个集成测试批的文件数不在 8 到 12 之间；某一批第二次仍失败；对账出现缺失、多余或重复；要通过测试就必须改产品代码、改分批规则、去掉过滤、使用默认并行度，或再跑一遍全量。出现这些情况时停止并回到计划。

**Invariants**

- 整个变更只跑一次全量。失败批的一次整批复核不是第二次全量。
- 每一条测试命令都带 `--test-threads=1`，并且带 `--lib`、`--bins` 或 `--test`。
- 同一时刻只跑一批。
- 产品代码保持 Iteration 000 接受时的行为。库仍是一个 package。

**Non-goals**

不改产品代码，不跑第二次全量，不测非默认 feature，不换平台，不拆 crate，不把 bench 纳入这组命令。

**Acceptance**

- S10：每一批都已执行，最终退出码为 0，failed 为 0。失败批只有在同命令复核通过时算通过，并写明第一次失败。
- S10：全部 `--test` 名字的集合等于当次 `ls tests/*.rs | sort` 得到的文件名集合（去掉末尾 `.rs`），无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。
- 需求 5：这是本变更的唯一一次全量。没有无过滤的 `cargo test`，没有缺少 `--test-threads=1` 的测试命令。
- 本 Cycle 结束时，`src/`、`tests/` 和 `Cargo.toml` 相对本 Cycle 起点没有 diff。

**Verification**

按 T2 的顺序执行各批。每批记录退出码和 failed 计数。全部批次都跑完且各自通过之后，用同一次枚举做对账。任一批没有跑，全量不算通过。不把全量再跑一遍来确认。

**Gate 2 Readiness**

Gate 2：PASS。审查项全部通过，无 BLOCKED，无 SKIPPED。

**Persisted Evidence**

- Mode: none

各批的退出码和 failed 计数写入 Act Response 即可。不保存完整测试输出。

**Risks and Notes**

- 复核时的 164、B = 14、K = 4 不是执行输入。文件数变化时按当次枚举重新计算。K 大于 B 或批大小越出 8 到 12 就停止。
- 第一批是单独的 `--lib --bins`，不参加 11 与 12 的文件切分。8 到 12 的检查只用于集成测试批。
- `[[bench]]` 不在本轮命令里。不要用 `--all-targets` 把它们加进来。
- goal 批准的是这一轮已经写在 tasks.md 里的范围。上面的 Gate 2 是检查后的结论。

## Act Response

- Status: 尚未执行

## Plan Review

- Review Result: pending

**Findings**

尚未 Review。

**Evidence**

尚未 Review。

**Follow-up Decision**

尚未 Review。

**Next Cycle**

None

**Next Iteration**

None
