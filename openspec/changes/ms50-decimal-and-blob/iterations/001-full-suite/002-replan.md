# Iteration 001 / Cycle 002: 补齐中断批未运行的目标

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: 4.1 未勾选。B08 两次退出码都是 101。失败点之后的 10 个目标两次都没有运行。已修改的中止测试还要按本次选择再跑一次。
- Repair items: None
- Inherited scope: B00–B07 与 B09–B14 的退出码 0 和 failed 为 0。B08 里 `memory_db_test` 的 failed 为 0。T5 已完成。`tuple_bytes_v1` 首字节是 `0x0C`。每条命令带 `--test-threads=1`。产品行为保持不变。
- Excluded scope: 第二遍全量；重跑已通过的批；重跑 `memory_db_test`；重跑 `test_abort_insert_row_not_visible` 和 `test_multiple_aborts`；修改产品代码或其他测试；修改标签、解码或中止语义；合并 master；推送；改主仓库。

**Objective**

用 11 条单独命令补上这一次全量里还没有运行结果的部分，并再跑一次已经改过的中止测试。11 条都是退出码 0 且 failed 为 0 时，4.1 记为：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。

**Background**

父 Cycle 是 `iterations/001-full-suite/000-initial.md`。Plan Context 为 ready，Act Response 为 blocked，Review Result 为 `replan-required`。该文件已冻结，本 Cycle 不改写。

`iterations/001-full-suite/001-replan.md` 的 Review Result 为 `accepted`，Next Iteration 为 None。它只接受夹具修复。当时的验证边界是不补跑这 10 个目标，也不声明这一次全量完成。该文件已冻结，本 Cycle 不改写。

用户这次选择改变的是 T4 的完成口径。对应选项是：「只补跑 B08 里没跑到的 10 个目标，再跑已修改的那条中止测试。不再跑已通过的批，也不算第二遍全量。」这是本 Cycle 的批准来源，不是跳过。

**Investigation Facts**

- Starting point: 分支 `changes/ms50-decimal-and-blob`，HEAD `6c6193ab2ff1402bd95f68553198872e38fba67a`，工作区干净。相对该提交，`src/` 和 `tests/` 没有未提交 diff。全量开跑时的产品代码之后，`tests/` 只有 `tests/mvcc_abort_test.rs` 这一处夹具改动，并且已经在这个 HEAD 里。`001-replan.md` 的定向结果是退出码 0、`1 passed; 0 failed`、`2 filtered out`。那个结果说明夹具曾经通过。本 Cycle 仍要再跑这一条，不用它代替本次命令。
- Current-State Evidence:
  - 父 Cycle 的 Act Response 写明，B08 两次的汇总行都是 `15 passed; 0 failed`，然后 `2 passed; 1 failed`。失败用例是 `test_abort_update_reverts_to_previous_version`。cargo 在这个目标之后停止该批。两次都没有运行的 10 个目标，按该 Response 的顺序是：`mvcc_commit_test`、`mvcc_record_test`、`mvcc_tombstone_visibility_test`、`nested_loop_join_test`、`network_protocol_test`、`network_server_test`、`no_from_select_test`、`order_by_alias_sort_test`、`outer_join_test`、`overflow_page_test`。
  - 当前工作树 `ls tests/*.rs | sort` 仍是 166 行。按父 Cycle 已执行的切分，B08 是排序后的第 83 到第 94 个文件。第 83 个是 `memory_db_test`，第 84 个是 `mvcc_abort_test`，第 85 到第 94 个与上面 10 个名字相同，第 95 个是 `param_binding_test`。因此 B08 里失败点之前已经给出 failed 为 0 的目标是 `memory_db_test`。`param_binding_test` 落在下一批，不在这 10 个里面。
  - 这 10 个名字都有对应的 `tests/<名>.rs`。`--test` 的名字是文件名去掉 `.rs`。
  - `tests/mvcc_abort_test.rs` 的 `tuple_bytes_v1` 是 `[0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]`。同文件另外两个测试是 `test_abort_insert_row_not_visible` 和 `test_multiple_aborts`。B08 里该目标的 `2 passed` 是这两条。
  - `design.md` 的「中止夹具不占用值标签」和 `specs/decimal-blob-types/spec.md` 不因本 Cycle 修改。产品行为没有新的差异。
- Code and Critical Path: 本 Cycle 不改产品代码，也不改测试。入口是本工作树根目录的 `cargo test`。工作树是 `/home/daivy/projects/RTsql-changes/ms50-decimal-and-blob`。10 个目标各覆盖一个集成测试文件。最后一条只覆盖 `test_abort_update_reverts_to_previous_version`。

**Implementation Guidance**

在本工作树根目录按 T4 的清单逐条启动。每一条都是单独一次进程。上一条进程退出后才启动下一条。先跑完 10 个未运行目标，再跑那一条中止测试。

判定取该 cargo 进程的退出码，以及输出里 `test result: ok.` 或 `test result: FAILED.` 汇总行的 failed。可以用一步 `grep -E '^test result:'` 阅读这些行。管道的退出码不代替 cargo 的退出码。`ignored` 可以不是 0。failed 必须是 0。

某一条退出码非 0，或任一汇总行 failed 非 0 时，不再启动后面的命令。Act Response 写 blocked。不改 `src/`，不改 `tests/`。

11 条全部满足后，勾选 `tasks.md` 的 4.1。完成记录使用该节已经写明的句子。不另写成又跑了一遍全量。

**Task Definitions**

### T4: 补齐这一次全量里尚未运行的目标

- Requirement/Scenario: `tasks.md` 第四节的完成口径。已通过部分沿用。中断批的剩余目标和修复后的那条测试补齐。
- Depends on: 父 Cycle 的 15 批记录，以及 T5 已把 v1 首字节改成 `0x0C`。
- Targets: 本工作树根目录的 11 条 `cargo test`。10 个集成测试文件是 `tests/mvcc_commit_test.rs`、`tests/mvcc_record_test.rs`、`tests/mvcc_tombstone_visibility_test.rs`、`tests/nested_loop_join_test.rs`、`tests/network_protocol_test.rs`、`tests/network_server_test.rs`、`tests/no_from_select_test.rs`、`tests/order_by_alias_sort_test.rs`、`tests/outer_join_test.rs`、`tests/overflow_page_test.rs`。中止测试是 `tests/mvcc_abort_test.rs` 的 `test_abort_update_reverts_to_previous_version`。
- Current behavior: B00–B07 与 B09–B14 已经退出码 0、failed 为 0。`memory_db_test` 在 B08 里 failed 为 0。上面 10 个目标没有运行结果。中止测试的夹具首字节已经是 `0x0C`，`001-replan.md` 里那次定向命令退出码 0、failed 为 0。4.1 仍未勾选。
- Required behavior:
  - 只在 `/home/daivy/projects/RTsql-changes/ms50-decimal-and-blob` 运行。
  - 开跑前 `git diff 6c6193ab2ff1402bd95f68553198872e38fba67a -- src tests` 为空。计划提交若只含本 Cycle 和 `tasks.md`，不算产品或测试改动。
  - 按下面的顺序，每条单独启动。上一条进程退出后才启动下一条。
  - 每条都带 `--test-threads=1`。
  - 每条把该 cargo 进程的退出码，以及每行 `test result: ok.` 或 `test result: FAILED.` 的 failed，写入 Act Response。
  - 11 条都是退出码 0，且每行这种汇总的 failed 都是 0 时，勾选 4.1。记录写成：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。
  - 任一条不满足时停止。4.1 保持未勾选。Act Response 为 blocked。
- Preserve: 已通过批的原记录。`memory_db_test` 不重跑。`tuple_bytes_v1` 保持 `[0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]`。T5 保持完成。`000-initial.md` 和 `001-replan.md` 不改写。产品行为保持 `design.md` 与规格的现有内容。
- Forbidden: 修改 `src/` 或其他测试；修改这条夹具；重跑 B00–B07、B09–B14 或 `memory_db_test`；不带测试名过滤器运行 `mvcc_abort_test`；运行 `test_abort_insert_row_not_visible` 或 `test_multiple_aborts`；把 11 条合成一条 shell 命令或一条带多个 `--test` 的 cargo 命令；把它们和已通过目标放在一起；第二遍全量；默认并行度；为计数新增脚本或判定层；checkout 其他分支；推送；改主仓库。
- First test: 本 Cycle 不改代码，没有新的先行失败测试。10 个目标没有可沿用的运行结果，清单第一条就是它们的入口。中止测试的改前失败沿用父 Cycle 的 B08：同一断言两次失败，left 是 `None`。改后的那次退出码 0 沿用 `001-replan.md`，只作为夹具已经改过的现场，不代替本 Cycle 的最后一条命令。
- GREEN condition: 下面 11 条全部退出码 0，且每条汇总行的 failed 为 0。4.1 使用规定句子勾选。
- Verification: 在本工作树根目录按顺序执行。失败表示该条退出码非 0，或汇总行 failed 非 0。

```text
cargo test --test mvcc_commit_test -- --test-threads=1
cargo test --test mvcc_record_test -- --test-threads=1
cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1
cargo test --test nested_loop_join_test -- --test-threads=1
cargo test --test network_protocol_test -- --test-threads=1
cargo test --test network_server_test -- --test-threads=1
cargo test --test no_from_select_test -- --test-threads=1
cargo test --test order_by_alias_sort_test -- --test-threads=1
cargo test --test outer_join_test -- --test-threads=1
cargo test --test overflow_page_test -- --test-threads=1
cargo test --test mvcc_abort_test test_abort_update_reverts_to_previous_version -- --test-threads=1
```

- Stop when: 开跑前 `src/` 或 `tests/` 相对 `6c6193ab2ff1402bd95f68553198872e38fba67a` 有 diff。某一条退出码非 0 或 failed 非 0。要让剩余命令通过就必须改产品代码或其他测试。要把这些命令并进已通过目标，或重新跑 15 批。出现这些情况时停止并 blocked，不修产品。

**Invariants**

- 已经跑过的 15 批仍是这一次全量。这 11 条是中断处的补齐，再加一次已经改过的中止测试。
- 每条测试命令都带 `--test-threads=1`。
- 上一条进程退出后才启动下一条。同一时刻只有一条。
- 产品代码和其他测试在本 Cycle 保持不变。

**Non-goals**

第二遍全量、重跑已通过的批、重跑 `memory_db_test`、重跑同文件另外两条、修改产品代码或其他测试、修改标签或中止语义。

**Acceptance**

- 10 个未运行目标各有一次单独的 `cargo test --test <名>`，顺序与父 Cycle Act Response 的名单一致。
- 最后一条只运行 `test_abort_update_reverts_to_previous_version`。
- 11 条都是退出码 0，且每行 `test result: ok.` 或 `test result: FAILED.` 的 failed 为 0。
- 4.1 的完成记录是：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。
- 已通过的批没有重跑。结束时 `git diff 6c6193ab2ff1402bd95f68553198872e38fba67a -- src tests` 仍为空。
- 任一命令失败时，4.1 仍未勾选，后面的命令没有启动，产品代码和测试没有被修改。

**Verification**

按 T4 的 11 条命令执行。判定是每条 cargo 进程的退出码，以及该进程输出中汇总行的 failed。10 个集成测试和那一条中止测试都走项目已有的 `cargo test` 入口。

通过之后，`tasks.md` 的 4.1 使用 Acceptance 里的那句完成记录。这 11 条不写成又跑了一遍全量。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。

计划批准来自用户这次选择，对应选项是「只补跑 B08 里没跑到的 10 个目标，再跑已修改的那条中止测试。不再跑已通过的批，也不算第二遍全量」。不是跳过。检查项已对照父 Cycle 的 Act Response、`001-replan.md` 的夹具结果、当前 166 个测试文件的顺序，以及 `tasks.md` 修订后的 Iteration 001。规格和 `design.md` 没有新的行为差异。

**Persisted Evidence**

- Mode: none

每条命令的退出码和 failed 计数写入 Act Response。

**Risks and Notes**

- 10 个目标没有历史运行结果。某一条失败就停止，不在本 Cycle 里改代码再试。
- `001-replan.md` 的定向命令已经退出码 0。本 Cycle 的最后一条仍要实际运行。那次结果不计入这 11 条的通过。
- 这 11 条不另计一次全量。全量预算仍是父 Cycle 已经跑过的那 15 批。
- 名单以父 Cycle Act Response 为准。当前排序与该名单一致，本 Cycle 不按一份新的枚举改名单。

| 完成口径 | Task | First Test | Status |
|---|---|---|---|
| 已通过部分沿用，补齐未运行目标和那条中止测试 | T4 | `mvcc_commit_test` | Covered |

`tasks.md` 里原有的规格完整性表不新增行。本 Cycle 没有新的产品要求。
