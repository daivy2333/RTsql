# Iteration 002 / Cycle 001: 从第 07 批续跑尚未通过的全量批次

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 父 Cycle `iterations/002-full-regression/000-initial.md` 已经通过的第 00–06 批。通过标准仍是每一批进程退出码为 0，且该次输出里每一条 `test result:` 的 failed 计数为 0。`ignored` 不为 0 不构成失败。分批名单以本文件列出的命令为准，不再重算。
- Excluded scope: 产品代码、测试、规格、`Cargo.toml`。目录序列化。`tests/legacy_compat_test.rs` 的两条目录行长度断言。第 00–06 批的重跑。把第 07 批拆开。ISS41。`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/` 整目录。Issue 状态、全局 tasks、SNAPSHOT、M/R/I。为了通过而降低 failed 计数或退出码要求。

**Objective**

修正那两条目录行长度断言的 Review 已是 `accepted`：`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/003-current-catalog-row-length/000-initial.md`。按下面固定的命令串行跑完第 07–13 批。每一批退出码为 0，且该次每一条 `test result:` 的 failed 计数为 0。第 00–06 批不重跑。

**Background**

父 Cycle 的 Act Response 为 `blocked`。执行时 `N = 149`，`B = 13`，`n11 = 7`，`n12 = 6`。第 00–06 批退出码 0、failed 为 0。第 07 批退出码 101，`legacy_compat_test` 为 `14 passed; 2 failed`，Cargo 没有跑该批剩下的 8 个目标。第 08–13 批没有启动。

两条失败都是写出的目录行比 `g1_tables_row_len(...) + 9` 多 1 字节：`legacy_db_write_upgrades_touched_table_row_to_current_layout` 是 41 对 40（`tests/legacy_compat_test.rs:441`），`legacy_g2_db_opens_queries_and_writes` 是 42 对 41（同文件 1086 行）。`serialize_catalog_row` 在 kind 字节之后又写入 `explicit_primary_key` 这一个 `u8`。空的唯一根和二级索引使这段尾部成为 10 字节，当时的断言按 9 字节比较。这个尾字节来自另一变更已接受的 Cycle `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/001-planner-selection/001-replan.md`（Review Result 为 `accepted`）。它不属于加列实现。本变更不改目录序列化，也不改这两条断言。修正这两条断言的 Review 已是 `accepted`，现场长度断言已是 `g1_tables_row_len + 10`。

审查时 `/tmp/rtsql-batch-*.log` 对不上父 Cycle 的记录：没有第 01–07 批文件，却有第 00 批和第 08–13 批；第 00 批日志里的库测试是 running 421，父 Cycle 记录的是 425 passed。那些文件不是本变更的通过证据，不能用来跳过第 08–13 批。

**Investigation Facts**

- Starting point: 父 Cycle 的第 00–06 批沿用，不重跑。第 07–13 批尚未通过。本 Cycle 不改产品代码，没有 RED。变更前状态就是这次停批，不把第 00–06 批再跑一遍来重新观察。
- Current-State Evidence: 审查时 `find tests -maxdepth 1 -type f -name '*.rs' -printf '%f\n' | LC_ALL=C sort` 仍是 149 个目标。父 Cycle 记录的第 01–07 批命令与这份顺序的前 77 个名字一致。第 08–13 批是剩下的 72 个名字，每批 12 个。下面的命令按这次顺序写死。以后 `tests/*.rs` 增减，都不改这 7 条命令。
  等待条件已满足。`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/003-current-catalog-row-length/000-initial.md` 的 Review Result 为 `accepted`。`legacy_db_write_upgrades_touched_table_row_to_current_layout` 断言 `users.len()` 为 `g1_tables_row_len + 10`，尾段为 8 个 `0x00`、`0x01`、`0x00`（`tests/legacy_compat_test.rs` 442–455 行）。`legacy_g2_db_opens_queries_and_writes` 断言 `people_row` 长度为 `g1_tables_row_len + 10`，尾段为 `[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]`（1089–1102 行）。`audit` 与 `events` 仍等于 `g1_tables_row_len`，`notes` 仍等于 `g2_tables_row_len`。`g1_tables_row_len` 的计算仍停在数据页尾指针。`src/storage/catalog.rs` 的 `serialize_catalog_row` 仍在 kind 和视图 SQL 之后执行 `buf.push(if row.explicit_primary_key { 1 } else { 0 })`（1288 行）。本 Cycle 仍不改这两条断言，也不改这段序列化。
- Code and Critical Path: 没有产品代码路径。入口是包根目录 `/home/daivy/projects/RTsql` 的 `cargo test`。判定只看每次进程退出码，以及该次输出中的 `test result:` 行。

**Implementation Guidance**

Plan Context 仍是 `draft` 时，一条 `cargo test` 都不要跑。只有本文件的 Status 已改成 `ready` 之后才从第 07 批开始。批次之间前一条已经通过，才启动下一条。某一批未通过就停，不改代码，也不把该批没跑到的目标拆出去。

**Task Definitions**

### T3: 续跑第 07–13 批

- Requirement/Scenario: 本变更全量回归（M19）。不映射新的产品场景。通过标准与父 Cycle 相同。
- Depends on: T2。另外，启动命令之前，本文件的 Plan Context 必须已经是 `ready`。`ready` 只表示修正 `legacy_db_write_upgrades_touched_table_row_to_current_layout` 与 `legacy_g2_db_opens_queries_and_writes` 这两条目录行长度断言的那次 Review 已经是 `accepted`。该 Review 在 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/003-current-catalog-row-length/000-initial.md`，结论为 `accepted`。这个条件不由本 Cycle 改断言来满足。
- Targets: 无产品符号。命令在包根目录 `/home/daivy/projects/RTsql` 执行。
- Current behavior: 第 00–06 批已经通过。第 07 批失败并停住。第 08–13 批没有作为本变更的通过批次跑过。
- Required behavior: 下面 7 条命令各实际跑一次，且每次通过。通过是指该次进程退出码为 0，并且该次输出中每一条 `test result:` 的 failed 计数都是 0。少跑一条就是 T3 未通过。和第 00–06 批合在一起，仍是这一次全量，不是第二次全量。
- Preserve: 第 00–06 批的通过结论。Iteration 000、001 已落地的加列行为。测试仍各自使用独立 `TempDir`。
- Forbidden: 修改产品代码、测试、规格或构建配置。修改 `serialize_catalog_row` 或任何目录行布局。修改 `legacy_db_write_upgrades_touched_table_row_to_current_layout` 和 `legacy_g2_db_opens_queries_and_writes` 的长度断言。读写 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/`。重跑第 00 批或第 01–06 批。重新计算 `N` 或改写下面的目标名单。把第 07 批拆成已经跑过的 3 个目标和未跑的 8 个目标。使用无目标过滤的 `cargo test`、`cargo test --tests`、`cargo test --all-targets` 或 `--benches`。使用默认并行度，或使用不是 `--test-threads=1` 的线程数。用 `/tmp/rtsql-batch-*.log` 把第 08–13 批标成已通过。为了通过而重跑已通过的批次，或在失败后自动再跑一次。Plan Context 仍是 `draft` 时启动任何 `cargo test`。
- First test: 无新测试。本任务不改代码，没有 RED。
- GREEN condition: 第 07–13 批各跑一次，每次退出码为 0 且 failed 计数为 0。第 00–06 批保持未重跑。
- Verification: 按下列顺序各执行一次。判定就是这些进程的退出码和它们打印的 `test result:`。

  1. 第 07 批，11 个目标整批一次：

     ```text
     cargo test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test --test nested_loop_join_test --test network_protocol_test -- --test-threads=1
     ```

  2. 第 08 批：

     ```text
     cargo test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test --test param_binding_test --test parser_test --test pg_integration_test --test pg_messages_test --test pg_protocol_test --test pipeline_test --test plan_cache_test -- --test-threads=1
     ```

  3. 第 09 批：

     ```text
     cargo test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test --test projection_expression_test --test projection_test --test pushdown_test --test query_memory_test --test readonly_session_test --test recovery_bounded_read_test --test recovery_e2e_test -- --test-threads=1
     ```

  4. 第 10 批：

     ```text
     cargo test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test --test release_durability_precondition_test --test release_path_bounded_read_test --test runtime_test --test scalar_function_test --test scalar_subquery_comparison_test --test schema_persistence_test --test secondary_index_equality_test -- --test-threads=1
     ```

  5. 第 11 批：

     ```text
     cargo test --test secondary_index_test --test select_distinct_test --test self_join_test --test set_operations_test --test sort_merge_join_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test -- --test-threads=1
     ```

  6. 第 12 批：

     ```text
     cargo test --test subset_insert_test --test sync_loader_test --test table_alias_resolution_test --test table_manager_test --test truncate_test --test tx_statement_test --test typed_api_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test -- --test-threads=1
     ```

  7. 第 13 批：

     ```text
     cargo test --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_integration_test --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1
     ```

  同一时刻只启动一批。下一批只在上一批退出码为 0 且每条 `test result:` 的 failed 计数为 0 之后启动。退出码为 0 但输出里没有 `test result:` 时，该批不算通过。Cargo 在批内第一个失败目标后不再跑其余目标时，该批已经失败，不把剩下的目标拆出去另跑。

- Stop when: Plan Context 仍是 `draft`；任一批退出码非 0 或 failed 计数非 0；缺一批；输出里没有 `test result:` 却要宣称通过；需要修改产品代码、目录序列化或那两条断言才能继续；需要重算名单或重跑第 00–06 批；需要打开另一变更的目录。

**Invariants**

- 全量仍是这一次。已通过的第 00–06 批不重复跑。
- 通过条件仍是退出码 0 且 failed 计数为 0。缺一批不得写成通过。
- 本 Cycle 的 diff 不包含产品代码、测试和规格。

**Non-goals**

- 把另一变更的目录尾字节修进本变更。
- 改写那两条长度断言，或放宽第 07 批的通过条件。
- ISS41。Join 的计划期选择、SMJ、Hash 构建侧。
- 预建下一次失败的 Iteration。

**Acceptance**

1. 第 00–06 批没有重跑。它们的通过结论仍是父 Cycle Act Response 里记录的退出码 0 和 failed 计数 0。
2. 第 07–13 批使用上面写死的命令，各执行一次，批间串行，都带 `-- --test-threads=1`。第 07 批仍是原来的 11 个目标。
3. 第 07–13 批每一批进程退出码为 0，且该次每一条 `test result:` 的 failed 计数为 0。
4. 缺一批、任一批退出码非 0，或任一批 failed 计数非 0，则 T3 未通过。
5. 本 Cycle 不修改产品代码、测试、目录序列化或那两条断言。

**Verification**

入口就是上面的 7 条 `cargo test`。通过与否只看每次进程的退出码和该次 `test result:` 的 failed 计数。

Act Response 按批记录：批次序号、完整命令、退出码、每一条 `test result:` 行。输出过长时只保留这些行。未启动的批次写明原因。第 00–06 批写明沿用父 Cycle，不复制那 7 批的长输出。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。等待条件已满足：`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/003-current-catalog-row-length/000-initial.md` 的 Review Result 为 `accepted`。两条长度断言现为 `g1_tables_row_len + 10`（`tests/legacy_compat_test.rs` 442–444 行、1089–1091 行）。计划审查 11 项通过；RTM 无 Missing、无 Simplified；本 Cycle 与 `tasks.md` 的 T3 以及 `design.md` 的全量一次约定一致；Persisted Evidence 为 none。用户要求在该 Review 已为 `accepted` 且其余 Gate 2 项通过时把本 Plan Context 改为 `ready`。第 07–13 批仍是每一批退出码 0 且 failed 为 0，第 00–06 批不重跑。

**Persisted Evidence**

- Mode: none

每批的命令、退出码和 `test result:` 行放进 Act Response 就足以判断通过或未通过。不保存完整日志。不建 `evidence/` 目录。`/tmp` 上已有的批次日志不作为本 Cycle 的证据。

**Risks and Notes**

- 修正这两条断言的 Review 已是 `accepted`，本文件 Status 为 `ready`。实施从第 07 批开始。本 Cycle 仍不改这两条断言，也不改目录序列化。
- 工作区里的其他产品改动会进入当次命令。失败时停批并回到计划。不在本 Cycle 修改那些文件，也不打开另一变更的目录。
- 若第 07 批在断言修正之后仍因这两条长度差失败，停在该批。不改目录序列化，也不改断言。

## Act Response

- Status: reported

### Implemented

没有改产品代码、测试、规格或构建配置。没有改 `serialize_catalog_row`，也没有改 `legacy_db_write_upgrades_touched_table_row_to_current_layout` 和 `legacy_g2_db_opens_queries_and_writes` 的长度断言。

`tasks.md` 里 T3 从 `pending` 改为 `reported`，对应复选框改为完成。Iteration 002 仍未完成：本 Cycle 的 Review Result 仍是 `pending`。

### Deviations from Plan

无。

### Self-Review

Gate 3 通过。7 条命令与 Task Definition 写死的第 07–13 批一致，每条都带 `-- --test-threads=1`。批间串行，前一批退出码为 0 且该次每一条 `test result:` 的 failed 计数为 0 之后才启动下一批。第 07 批仍是原来的 11 个目标，没有拆开。第 00–06 批没有重跑。没有无目标过滤的 `cargo test`。没有读写 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/`。没有创建 `evidence/` 目录。

`network_server_test` 的 `test result:` 是 `0 passed; 0 failed`。failed 计数为 0，该批其余目标都有 `test result:`，退出码为 0，按通过条件计为通过。

Gate 4 通过。第 07–13 批各实际跑一次，退出码都是 0，该次每一条 `test result:` 的 failed 计数都是 0。和第 00–06 批合在一起仍是这一次全量。

Gate 5 未命中。没有遗留 Minor。

### Verification

第 00–06 批沿用父 Cycle `iterations/002-full-regression/000-initial.md` 的 Act Response：退出码 0，failed 计数 0。本 Cycle 未重跑，不复制那 7 批的输出。

工作目录 `/home/daivy/projects/RTsql`。下面每条命令各执行一次。

1. 第 07 批。退出码 0。

   `cargo test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test --test nested_loop_join_test --test network_protocol_test -- --test-threads=1`

   - `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s`
   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s`
   - `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.71s`
   - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`
   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`
   - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`
   - `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s`
   - `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s`
   - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`

2. 第 08 批。退出码 0。

   `cargo test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test --test param_binding_test --test parser_test --test pg_integration_test --test pg_messages_test --test pg_protocol_test --test pipeline_test --test plan_cache_test -- --test-threads=1`

   - `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s`
   - `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s`
   - `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s`
   - `test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.22s`
   - `test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.29s`
   - `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`
   - `test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s`
   - `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s`

3. 第 09 批。退出码 0。

   `cargo test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test --test projection_expression_test --test projection_test --test pushdown_test --test query_memory_test --test readonly_session_test --test recovery_bounded_read_test --test recovery_e2e_test -- --test-threads=1`

   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s`
   - `test result: ok. 107 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s`
   - `test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.93s`
   - `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s`
   - `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s`
   - `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s`
   - `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s`
   - `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.19s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s`

4. 第 10 批。退出码 0。

   `cargo test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test --test release_durability_precondition_test --test release_path_bounded_read_test --test runtime_test --test scalar_function_test --test scalar_subquery_comparison_test --test schema_persistence_test --test secondary_index_equality_test -- --test-threads=1`

   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s`
   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`
   - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s`
   - `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s`
   - `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.41s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s`
   - `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s`
   - `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 52.97s`

5. 第 11 批。退出码 0。

   `cargo test --test secondary_index_test --test select_distinct_test --test self_join_test --test set_operations_test --test sort_merge_join_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test -- --test-threads=1`

   - `test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.76s`
   - `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s`
   - `test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s`
   - `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s`
   - `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`
   - `test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.33s`
   - `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 46.16s`
   - `test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s`
   - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s`
   - `test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.65s`
   - `test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.20s`

6. 第 12 批。退出码 0。

   `cargo test --test subset_insert_test --test sync_loader_test --test table_alias_resolution_test --test table_manager_test --test truncate_test --test tx_statement_test --test typed_api_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test -- --test-threads=1`

   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.73s`
   - `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`
   - `test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s`
   - `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`
   - `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s`
   - `test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.30s`
   - `test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s`
   - `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s`
   - `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s`
   - `test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s`
   - `test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`

7. 第 13 批。退出码 0。

   `cargo test --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_integration_test --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1`

   - `test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.39s`
   - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.90s`
   - `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 31.33s`
   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s`
   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 31.51s`
   - `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s`
   - `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
   - `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.52s`
   - `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`
   - `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s`
   - `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s`
   - `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s`

没有未启动的批次。

### Persisted Evidence

None required。模式是 `none`。未创建 evidence 目录。

### Experience Candidates

None。

Issue 候选：None。

### 未解决问题

无阻塞项。Review Result 仍是 pending。未填写 Plan Review，未创建后继 Cycle 或后继 Iteration，未改全局 tasks、SNAPSHOT、M/R/I，未改 Issue，未归档。

## Plan Review

- Review Result: accepted

**Findings**

没有阻塞本轮 Acceptance 的偏差，也没有 Minor。

第 07–13 批的 7 条命令与 Task Definition 里写死的命令逐字相同，都带 `-- --test-threads=1`。目标数依次是 11、12、12、12、12、12、12，Act Response 里对应的 `test result:` 条数相同。每一条的 failed 计数都是 0，记录的进程退出码都是 0。第 07 批仍是原来的 11 个目标。Verification 里没有第 00–06 批的重跑输出，也没有无目标过滤的 `cargo test`。

第 08 批第一条是 `0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`。`tests/network_server_test.rs` 里的测试函数写在块注释中，默认 harness 没有可执行测试，但仍打印了 `test result:`。通过条件要求 failed 计数为 0，并且该次输出里要有 `test result:`。这条按通过计。

第 00–06 批沿用父 Cycle `iterations/002-full-regression/000-initial.md` 的 Act Response。该段写明 7 次退出码 0；其中的 `test result:` 共 68 条，failed 计数都是 0。本轮没有重跑。

`tests/legacy_compat_test.rs` 的两条长度断言仍是 `g1_tables_row_len + 10`，尾段仍是 8 个 0、`0x01`、`0x00`（442–455 行、1089–1102 行）。`src/storage/catalog.rs` 的 `serialize_catalog_row` 仍在 1288 行写入 `explicit_primary_key`。这两处与 Plan Context 记录的现场一致。Act 列出的文件改动只有 `tasks.md` 里 T3 的状态。没有 `evidence/` 目录。

**Evidence**

沿用本 Cycle Act Response 的第 07–13 批：命令、退出码 0、每条 `test result:` 的 failed 为 0。未重跑。第 00–06 批沿用父 Cycle Act Response，未重跑。Persisted Evidence 为 `none`。上述源码位置是本轮读过的现场。

**Follow-up Decision**

五条 Acceptance 都满足。Iteration Plan 保持不变。不需要在当前 Cycle 修复，也不需要 rework 或 replan。没有后续 Iteration，不展开。

**Next Cycle**

None

**Next Iteration**

None
