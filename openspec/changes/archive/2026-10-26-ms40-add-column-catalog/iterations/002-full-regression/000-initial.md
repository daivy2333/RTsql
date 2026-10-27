# Iteration 002 / Cycle 000: 本变更的全量回归

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: ISS39、ISS40 已由 Iteration 000、001 落地的行为。本 Cycle 只验证这些行为仍在整套测试里通过。分批算法以本文件为准。
- Excluded scope: 产品代码、测试代码、规格、`Cargo.toml`。ISS41。Join、SMJ、Hash 构建侧。`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/` 整目录。Issue 状态、全局 tasks、SNAPSHOT、M/R/I。为失败批次改验收标准，以及预建尚未出现的缺陷的 Iteration。

**Objective**

按下面的算法跑完第 00 批和全部分集成测试批次。每一批进程退出码为 0，且该次输出里每一条 `test result:` 的 failed 计数为 0。少跑一批、任一批退出码非 0、任一批 failed 计数非 0，本 Iteration 未通过。

**Background**

这是本变更唯一一次全量回归。Iteration 001 的 Review Result 已是 `accepted`，本次不改那个文件。规划阶段不跑全量，也不跑定向测试。

**Investigation Facts**

- Starting point: Iteration 001 Cycle `iterations/001-inflight-narrowing-publish-layout/000-initial.md` 的 Act Response 为 `reported`，Plan Review 为 `accepted`。该 Cycle 未跑全量。沿用其定向结论，本 Cycle 不把这几条重跑当作 T3 的通过条件：
  1. 先行测试 `cargo test --test ddl_layout_publication_test add_column_waits_out_inflight_narrowing_and_reopen_matches -- --test-threads=1`：RED 退出码 101，`0 passed; 1 failed`；GREEN 后退出码 0，`1 passed; 0 failed`。
  2. `cargo test --test ddl_layout_publication_test -- --test-threads=1`：退出码 0，`11 passed; 0 failed`。含 `add_column_waits_out_inflight_narrowing_and_reopen_matches`、`concurrent_same_table_ddl_is_refused`、`abandoned_add_column_during_count_flush_is_not_durable`、`failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`。
  3. `cargo test --test ddl_rewrite_consistency_test`：退出码 0，`9 passed; 0 failed`。含 `read_during_narrowing_in_flight_is_refused`。第二条在第一条退出码为 0 之后执行。
  Iteration 001 已写入的产品行为：同表 `ADD COLUMN` 在拿 `tables` 写锁和写目录之前，按该表 `NarrowingState` 的只读条件等待（`owner != 0` 或 `published`）。正常路径发布安装的是删列在同步 `data_page_tail` 之后放进 pending 的那一份 `Arc<TableMeta>`。恢复期 `publish_column_narrowing` 仍从打开时的布局推导。列数刷盘暂停点仍在 `update_table_column_count_ddl` 返回 `Ok` 之后、`is_closed()` 之前。这些符号本 Cycle 不改。
  Iteration 000 已 accepted。其定向结论由 Iteration 001 沿用：`ddl_layout_publication_test` 当时 `10 passed; 0 failed`，`alter_table_test` `16 passed; 0 failed`，`ddl_crash_recovery_test` `6 passed; 0 failed`。本 Cycle 不单独重跑这三条来代替全量。
- Current-State Evidence: 规划时只读了仓库布局，没有运行 `cargo test`。
  - 包根目录是 `/home/daivy/projects/RTsql`。`Cargo.toml` 没有 `autotests = false`，没有 `[[test]]`，没有 `[[bin]]`。集成测试目标由 Cargo 自动发现：`tests/*.rs` 的文件名去掉 `.rs` 就是 `--test` 的名字。
  - `src/lib.rs` 与 `src/main.rs` 都在。第 00 批的 `--lib` 覆盖库测试，`--bins` 覆盖 `src/main.rs` 这个二进制。没有 `src/bin/`。`[[bench]]` 不进入本回归。
  - `tests/` 下除顶层 `*.rs` 外有 `tests/fixtures/`。该目录不是集成测试目标。没有 `tests/*/main.rs`。
  - 规划时 `find tests -maxdepth 1 -type f -name '*.rs'` 得到 149 个文件。`tasks.md` 收尾验证和 SNAPSHOT 写的历史个数是 148。149 和 148 都不是执行用的 `N`。执行时按当时的 `tests/*.rs` 重算。
  - 工作区里另有 Join 计划期选择的未提交产品改动。那些改动不属于 T3。本 Cycle 不读、不写 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/`。全量仍按执行当时的工作区跑，不把 Join 目标从名单里拿掉。
- Code and Critical Path: 本 Iteration 没有产品代码路径。入口是包根目录的 `cargo test`。判定只看每次调用的进程退出码，以及该次输出中的 `test result:` 行。

**Implementation Guidance**

先列名单并做完分批检查，再启动第 00 批。检查不通过时，一条 `cargo test` 都不要跑。批次之间前一条进程退出并且判定为通过之后，才启动下一条。某一批未通过时停在该批，不启动后面的批，也不改代码。

**Task Definitions**

### T3: 本变更唯一一次全量回归

- Requirement/Scenario: 本变更全量回归（M19）。不映射新的产品场景。
- Depends on: T2
- Targets: 无产品符号。命令在包根目录 `/home/daivy/projects/RTsql` 执行。
- Current behavior: Iteration 000 与 001 的定向命令已经通过。全量尚未在本变更里跑过。
- Required behavior: 下面算法定义的每一批都实际跑过一次，且每一批通过。通过是指该次进程退出码为 0，并且该次输出中每一条 `test result:` 的 failed 计数都是 0。ignored 不为 0 不构成失败。少跑一批就是未通过。
- Preserve: Iteration 000、001 已落地的加列放弃回退和在途窄化发布。测试仍各自使用独立 `TempDir`，本 Cycle 不新增测试，也不改现有测试。
- Forbidden: 修改产品代码、测试、规格或构建配置。把 ISS41 或 Join 纳入修复。读写 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/`。使用无目标过滤的 `cargo test`、`cargo test --tests`、`cargo test --all-targets` 或 `--benches`。任何批次使用默认并行度，或使用不是 `--test-threads=1` 的线程数。沿用 148、149 或任何写在计划里的历史个数。检查失败后另选分法。为了通过而重跑已通过的批次，或在失败后自动再跑一次。把判定写成仓库里的脚本。
- First test: 无新测试。本任务不改代码，没有 RED。变更前状态沿用上面 Starting point 的定向结论。
- GREEN condition: 第 00 批和下面算出的每一个集成测试批次各跑一次，每次退出码为 0 且 failed 计数为 0。
- Verification: 按下列步骤执行。判定就是这些 `cargo test` 进程的退出码和它们打印的 `test result:`。

  1. 列出集成测试目标。在包根目录执行：

     ```text
     find tests -maxdepth 1 -type f -name '*.rs' -printf '%f\n' | LC_ALL=C sort
     ```

     每一行去掉末尾的 `.rs`，得到的字符串就是一个 `--test` 名字。顺序保持这条管道的 `LC_ALL=C` 字典序。`N` 是这个名单的个数。`N = 0` 时停止，不跑任何 `cargo test`，回到计划。

  2. 计算批次。`B = ceil(N / 12)`。`N` 为非负整数时，用整数除法 `B = (N + 11) / 12`。大小为 11 的批次数 `n11 = B * 12 - N`。大小为 12 的批次数 `n12 = N - 11 * B`。`n11 < 0` 或 `n12 < 0` 时停止，不跑任何 `cargo test`，回到计划，不另选分法。

  3. 切分。按第 1 步的顺序连续取名：先取 `n11` 个批次，每批 11 个目标；再取 `n12` 个批次，每批 12 个目标。`n11` 或 `n12` 为 0 时，那一组批次数是 0，不制造空批。切完后核对三件事：`n11 + n12 = B`，`11 * n11 + 12 * n12 = N`，拼回的名单与第 1 步逐项相同且每个目标恰好出现一次。任一件不成立就停止，不跑任何 `cargo test`，回到计划。任一批的目标数小于 8 或大于 12 时同样停止，不另选分法。

  4. 第 00 批，单独一次：

     ```text
     cargo test --lib --bins -- --test-threads=1
     ```

     这一批不参与 8 到 12 的目标计数。库、文档测试和二进制可能各有一条 `test result:`。每一条的 failed 计数都要是 0，进程退出码也要是 0。退出码为 0 但输出里没有 `test result:` 时，不把该批当作通过，停止并回到计划。

  5. 集成测试批次按第 3 步的顺序，每批一次：

     ```text
     cargo test --test <名> --test <名> … -- --test-threads=1
     ```

     一批里的名字全部放进同一次调用。该次每一条 `test result:` 的 failed 计数为 0，且进程退出码为 0，这批才通过。Cargo 在批内第一个失败目标后不再跑该批其余目标时，该批已经失败，不把剩下的目标拆出去另跑。

  6. 第 00 批和各集成测试批次严格串行。本 Cycle 同一时刻只启动一批。下一批只在上一批已经通过之后启动。任一批退出码非 0 或 failed 计数非 0 时，停在该批，不启动后续批次。此时未启动的批次算缺批，整次全量未通过。

  7. 名单在第 00 批之前列出一次并固定，批间不重算 `N`。不把中途新出现的文件补进剩余批次。某次调用因为目标不存在而失败时，该批未通过，停止并回到计划。

- Stop when: `N = 0`；`n11` 或 `n12` 为负；任一批目标数不在 8 到 12；切分核对失败；任一批退出码非 0 或 failed 计数非 0；缺一批；输出里没有 `test result:` 却要宣称通过；需要改产品代码才能继续；执行依据要求把 ISS41 或 Join 写进本 Iteration。

**Invariants**

- 全量只在本 Iteration 跑这一次。已通过的批次不重复跑。
- 通过条件等于「算法产生的每一批都跑过，且各自退出码为 0、failed 计数为 0」。缺一批不得写成通过。
- 本 Cycle 的 diff 不包含产品代码、测试和规格。

**Non-goals**

- 新功能，以及为了让全量变绿而修改实现。
- ISS41。Join 的计划期选择、SMJ、Hash 构建侧。
- 把失败归到其他故障域之后在本 Iteration 里修。与加列故障域相同的新缺陷留给后续 Iteration。其他故障域仍留在 MS40，不新建里程碑。发现方只在 Act Response 里报告，不登记 Issue。

**Acceptance**

1. 执行时的 `tests/*.rs` 经 `LC_ALL=C` 排序后，每个目标恰好进入一个集成测试批次。批次大小只有 11 和 12，且都在 8 到 12 之内。`n11`、`n12` 均非负。
2. 第 00 批命令与每个集成测试批次命令各执行一次，批间串行，都带 `-- --test-threads=1`。
3. 每一批进程退出码为 0，且该次每一条 `test result:` 的 failed 计数为 0。
4. 缺一批、任一批退出码非 0，或任一批 failed 计数非 0，则 T3 未通过。
5. 本 Cycle 不修改产品代码。

**Verification**

本任务是跨文件的回归，入口就是上面的 `cargo test` 命令。通过与否只看每次进程的退出码和该次 `test result:` 的 failed 计数。

Act Response 按批记录：批次序号、完整命令、退出码、每一条 `test result:` 行。输出过长时只保留这些行。未启动的批次写明未启动的原因。缺一批时结论写未通过。

**Gate 2 Readiness**

PASS。计划审查、RTM、设计闭合、与 `tasks.md` / `design.md` 的一致性、Persisted Evidence、用户对既有计划的批准均已满足。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

每批的命令、退出码和 `test result:` 行放进 Act Response 就足以判断通过或未通过。不保存完整日志。不建 `evidence/` 目录。

**Risks and Notes**

- 执行时工作区可能含有另一名实施者正在写入的 Join 产品改动。这些改动会进入当次全量。失败时停批并回到计划，不在本 Cycle 修改那些文件，也不打开 Join 变更目录。
- `cargo test --lib` 会带上库的文档测试。这属于第 00 批的同一次调用，不单独加一批，也不加开关把它排除。
- 规划时看到的 149 和文档里的 148 只说明历史个数已经不能用。执行用的 `N` 只来自当次第 1 步。

## Act Response

- Status: blocked

T3 未通过。停在集成测试第 07 批。该批退出码 101，`legacy_compat_test` 的 `test result:` 为 `14 passed; 2 failed`。第 07 批里 Cargo 停下后的 8 个目标，以及第 08–13 批，都没有启动。`tasks.md` 里的 T3 仍是未完成。

### Implemented

没有产品代码、测试或规格改动。

### 分批

执行时 `find tests -maxdepth 1 -type f -name '*.rs' -printf '%f\n' | LC_ALL=C sort` 得到 `N = 149`。`B = (149 + 11) / 12 = 13`。`n11 = 13 * 12 - 149 = 7`。`n12 = 149 - 11 * 13 = 6`。`n11 + n12 = 13 = B`。`11 * 7 + 12 * 6 = 149 = N`。先 7 批各 11 个目标，再 6 批各 12 个目标。批大小都在 8 到 12。核对通过后才启动第 00 批。名单在第 00 批之前固定，批间没有重算。

### Deviations from Plan

第 01–07 批把同一次 `cargo test` 的标准输出和标准错误重定向到 `/tmp/rtsql-batch-NN.log`，用来读出每一条 `test result:`。进程退出码仍是该次 `cargo` 的退出码。传给 `cargo` 的参数与计划命令相同，包含 `-- --test-threads=1`。这些日志不在变更目录里。第 00 批没有重定向。

### Self-Review

本 Cycle 没有产品 diff。Gate 3 没有代码改动可审。Gate 4 未通过：第 07 批退出码非 0，且有一条 `test result:` 的 failed 计数为 2；其后批次未跑。Gate 5 命中，停止。没有为通过而重跑已通过的批次，也没有在失败后自动再跑一次。

### Verification

第 00 批

`cargo test --lib --bins -- --test-threads=1`

退出码 0。

```text
test result: ok. 425 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 25.19s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

第一条是 `src/lib.rs` 的单元测试，第二条是 `src/main.rs`。输出里没有 Doc-tests 段。

第 01 批

`cargo test --test aggregate_extension_test --test aggregate_output_shape_test --test aggregate_test --test alter_table_test --test attach_session_test --test btree_merge_test --test btree_scale_test --test btree_split_test --test btree_test --test busy_timeout_test --test cache_perf_test -- --test-threads=1`

退出码 0。11 条，failed 都是 0。

```text
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.15s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.92s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.55s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 27.41s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.34s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s
```

第 02 批

`cargo test --test catalog_bounded_read_test --test catalog_durability_test --test chain_join_test --test check_command_test --test checkpoint_command_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test checkpoint_test --test cli_test --test concurrent_test --test connection_limit_test -- --test-threads=1`

退出码 0。11 条，failed 都是 0。其中一条 ignored 为 2，按通过条件不构成失败。

```text
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.98s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 45.97s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.99s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.06s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 126 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 123.47s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.84s
```

第 03 批

`cargo test --test constraint_enforcement_test --test cross_db_dml_test --test cross_db_query_test --test ctas_test --test cte_test --test data_page_bounded_read_test --test database_file_lock_test --test datetime_function_test --test datetime_type_test --test ddl_catalog_transactional_test --test ddl_crash_recovery_test -- --test-threads=1`

退出码 0。11 条，failed 都是 0。

```text
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.08s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.72s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.87s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.23s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.34s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.35s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.10s
```

第 04 批

`cargo test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test default_declaration_test --test default_precheck_test --test derived_scan_alias_test --test dml_tombstone_abandon_window_test --test dml_tx_id_test --test drop_persistence_test --test drop_table_free_test --test e2e_test --test encryption_test -- --test-threads=1`

退出码 0。11 条，failed 都是 0。

```text
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.19s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.70s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.54s
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.16s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.99s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.56s
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 79.80s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.88s
```

第 05 批

`cargo test --test executor_test --test explain_test --test explicit_tx_test --test expression_e2e_test --test file_header_test --test file_storage_io_test --test free_page_lifecycle_test --test full_table_dml_test --test gc_test --test group_by_expr_test --test import_input_surface_test -- --test-threads=1`

退出码 0。11 条，failed 都是 0。

```text
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.93s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 34.26s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.34s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.99s
```

第 06 批

`cargo test --test index_manager_test --test index_page_ownership_stack_overflow_test --test index_page_ownership_test --test index_read_bounded_read_test --test index_write_atomicity_test --test insert_column_list_test --test insert_select_test --test isolation_level_test --test join_output_resolution_test --test join_test --test key_type_conformance_test -- --test-threads=1`

退出码 0。11 条，failed 都是 0。

```text
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.35s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.52s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.47s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.74s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.88s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.04s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
```

第 07 批

`cargo test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test --test nested_loop_join_test --test network_protocol_test -- --test-threads=1`

退出码 101。Cargo 在 `legacy_compat_test` 失败后停止，没有运行该批其余目标。

```text
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.63s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: FAILED. 14 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.64s
```

失败的两个测试：

- `legacy_db_write_upgrades_touched_table_row_to_current_layout`，`tests/legacy_compat_test.rs:441`。断言左侧 41，右侧 40。消息：the written table's catalog row must carry the current-layout tail (u32 unique roots | u32 secondary indexes | u8 kind), got 41 bytes。
- `legacy_g2_db_opens_queries_and_writes`，`tests/legacy_compat_test.rs:1086`。断言左侧 42，右侧 41。消息：the written table's catalog row must be rewritten in the current layout (unique roots | secondary indexes | kind), got 42 bytes。

该批未执行的目标：`limit_test`、`memory_db_test`、`mvcc_abort_test`、`mvcc_commit_test`、`mvcc_record_test`、`mvcc_tombstone_visibility_test`、`nested_loop_join_test`、`network_protocol_test`。

第 08–13 批未启动。原因是第 07 批未通过。

### Blocker Handoff

- Task / step / Gate：T3，集成测试第 07 批，Gate 4 未通过，Gate 5 停止。
- 计划预期：算法产生的每一批都跑过，退出码 0，且该次每条 `test result:` 的 failed 计数为 0。
- 实际情况：第 07 批退出码 101，三条 `test result:` 的 failed 计数是 0、0、2。整次全量未通过。
- 影响：缺第 07 批的剩余目标和第 08–13 批。T3 不能标完成。
- 已完成：分批核对；第 00–06 批通过。
- 部分完成：第 07 批已启动并失败。
- 未开始：第 08–13 批，以及第 07 批中 Cargo 未执行的 8 个目标。
- 修改文件：只写了本 Cycle 的 Act Response。没有改产品代码、测试、规格或构建配置。没有读写 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/`。没有改全局 tasks、SNAPSHOT、M/R/I、Issue。`tasks.md` 的 T3 仍是未完成。
- 已通过的 Gate：跑测试前的分批核对。第 00–06 批各自退出码 0 且 failed 为 0。整体 Gate 4 未通过。
- Evidence：None required。模式是 `none`。未创建 evidence 目录。
- 恢复条件：用户解决这次失败，或接受风险并明确要求继续；也可以调用 `openspec-plan`。已通过的第 00–06 批不重跑。本 Cycle 不改产品代码来让第 07 批变绿。

### Persisted Evidence

None required。

### Experience Candidates

None。

Issue 候选：None。失败留在本回复里，不登记 Issue。

### 未解决问题

第 07 批的两条目录行长度断言未通过。没有把失败归到某个故障域，也没有改实现。Review Result 仍是 pending。未填写 Plan Review，未创建后继 Cycle 或后继 Iteration，未归档。

## Plan Review

- Review Result: replan-required

**Findings**

阻塞，原因是现状变化。T3 的通过条件没有满足：第 07 批退出码 101，`legacy_compat_test` 的 `test result:` 为 `14 passed; 2 failed`，该批其余 8 个目标和第 08–13 批没有启动。Act Response 与本文件里的记录一致，也与现场代码一致。`N = 149`，`B = 13`，`n11 = 7`，`n12 = 6`。审查时按 `LC_ALL=C` 重列 `tests/*.rs`，仍是 149 个名字，第 01–07 批命令就是这份顺序的前 77 个。两条失败都是目录行比 `g1_tables_row_len + 9` 多 1 字节：441 行是 41 对 40，1086 行是 42 对 41。`serialize_catalog_row` 在 kind 之后固定再写一个 `explicit_primary_key` 字节；唯一根和二级索引都为空时，尾部是 10 字节，断言仍按 9 字节比较。这个字节写在已接受的 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/001-planner-selection/001-replan.md`（Review Result 为 `accepted`）。加列实现不写这段尾部。`tests/legacy_compat_test.rs` 不在工作区改动里。父 Cycle 禁止改产品代码，也禁止改这两条断言，所以不能在原执行依据里把第 07 批变成通过。

不阻塞。Act 把第 01–07 批的输出重定向到 `/tmp/rtsql-batch-NN.log`，传给 `cargo` 的参数没有改。审查时磁盘上的同名文件对不上这份记录：没有第 01–07 批，却有第 00 批和第 08–13 批；第 00 批日志里的库测试是 running 421，本文件记录的是 425 passed。这些文件不作为本 Cycle 的通过证据，第 08–13 批仍算未通过。第 02 批有一条 `ignored` 为 2，按原通过条件不算失败。

**Acceptance Gaps**

父 Cycle Acceptance 1 已满足：名单覆盖全部 149 个目标，批大小只有 11 和 12。Acceptance 5 已满足：本 Cycle 没有修改产品代码。Acceptance 2、3、4 未满足：第 07–13 批没有在退出码 0 且 failed 计数为 0 的条件下跑完。通过标准不改为“第 00–06 批通过即 T3 通过”。

**Evidence**

沿用父 Cycle Act Response 的第 00–06 批：退出码 0，failed 计数为 0。未重跑。第 07 批的失败位置与 `tests/legacy_compat_test.rs:441`、`:1086` 的 `g1_tables_row_len + 9` 断言一致。多出的字节与 `src/storage/catalog.rs` 里 `serialize_catalog_row` 的 `explicit_primary_key` 写入一致。另一变更中引入该字节的 Cycle 其 Review Result 为 `accepted`。本次审查没有运行 `cargo test`。

**Follow-up Decision**

原 Task Definition 要求在本 Cycle 跑完每一批，失败即停，并且禁止改产品和那两条断言。继续留在本 Cycle 只能重跑已通过的批次，或者把目录格式差异改进来。前者不缩小缺口，后者越过禁止范围。缺口需要新的执行依据：第 00–06 批沿用，第 07–13 批整批续跑，而且要等修正那两条断言的 Review 成为 `accepted`。验证方式变了，所以是 `replan-required`，不是当前 Cycle 修复，也不是 `rework-required`。

**Iteration Plan Update**

Iteration 002 仍只含 T3，不拆 Iteration。通过标准不变：每一批退出码 0，且该次每条 `test result:` 的 failed 计数为 0；缺一批仍是未通过。变化在续跑方式：第 00–06 批不重跑；第 07 批按原来的 11 个目标整批重跑；第 08–13 批使用同一次固定名单；后继 Cycle 在断言修正的 Review 为 `accepted` 之前保持 `draft`。`tasks.md` 的 T3 要点、Iteration 002 和收尾验证已按此改写。规格和 `design.md` 没有改：全量仍只在最后一个 Iteration 跑一次，方式仍指向 `tasks.md`。

**Next Cycle**

`openspec/changes/2026-10-26-ms40-add-column-catalog/iterations/002-full-regression/001-replan.md`

**Next Iteration**

None
