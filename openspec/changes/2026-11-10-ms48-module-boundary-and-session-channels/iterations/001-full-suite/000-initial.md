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

- Status: reported

**实际改动**

没有修改产品符号。`src/`、`tests/`、`Cargo.toml` 相对本 Cycle 起点没有 diff。本 Cycle 只更新本 Act Response，以及 change `tasks.md` 里 T2 的状态。

**与计划的偏差**

无。

**自检**

Gate 3：通过。需求 5 和 S10 由这一次分批结果覆盖。diff 没有计划外的产品修改，没有新增判定脚本，没有去掉过滤，没有使用默认并行度。

Gate 4：通过。15 条命令的退出码都是 0，每条输出里的 failed 都是 0。对账通过。

Gate 5：未触发。

没有需要修复的发现。没有遗留 Minor。

跳过项：无。Iteration 000 的五条定向测试按执行依据沿用，没有重跑。

**验证**

执行时 `ls tests/*.rs | sort` 得到 N=164。B=14，K=4。K 小于 B。前 4 个集成测试批各 11 个文件，后 10 个各 12 个，都在 8 到 12 之间。停止条件不成立。总批数 15。批与批之间上一条进程结束后才开始下一条。没有批次复核。

1. `cargo test --offline --lib --bins -- --test-threads=1`。退出码 0，failed 0。摘要两行：`445 passed; 0 failed`，`0 passed; 0 failed`。
2. `cargo test --offline --test aggregate_extension_test --test aggregate_output_shape_test --test aggregate_test --test alter_table_test --test attach_session_test --test btree_merge_test --test btree_scale_test --test btree_split_test --test btree_test --test busy_timeout_test --test cache_perf_test -- --test-threads=1`。退出码 0，failed 0。11 条摘要，每条 failed 为 0。
3. `cargo test --offline --test catalog_bounded_read_test --test catalog_durability_test --test chain_join_test --test check_command_test --test checkpoint_command_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test checkpoint_test --test cli_main_profiling_test --test cli_test --test composite_index_lifecycle_test -- --test-threads=1`。退出码 0，failed 0。11 条摘要，每条 failed 为 0。
4. `cargo test --offline --test composite_index_runtime_test --test concurrent_test --test connection_limit_test --test constraint_enforcement_test --test cross_db_dml_test --test cross_db_query_test --test crypto_off_build_test --test ctas_test --test cte_test --test data_page_bounded_read_test --test database_file_lock_test -- --test-threads=1`。退出码 0，failed 0。11 条摘要，每条 failed 为 0。
5. `cargo test --offline --test datetime_function_test --test datetime_type_test --test ddl_catalog_transactional_test --test ddl_crash_recovery_test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test default_declaration_test --test default_precheck_test --test derived_scan_alias_test --test diagnostics_facade_test --test diagnostics_statement_test -- --test-threads=1`。退出码 0，failed 0。11 条摘要，每条 failed 为 0。
6. `cargo test --offline --test diagnostics_wait_test --test dml_tombstone_abandon_window_test --test dml_tx_id_test --test drop_persistence_test --test drop_table_free_test --test e2e_test --test encryption_test --test executor_test --test explain_test --test explicit_tx_test --test expression_e2e_test --test file_header_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
7. `cargo test --offline --test file_storage_io_test --test free_page_lifecycle_test --test full_table_dml_test --test gc_test --test group_by_expr_test --test hash_join_build_side_test --test import_input_surface_test --test index_key_encoding_evaluation_test --test index_manager_test --test index_page_ownership_stack_overflow_test --test index_page_ownership_test --test index_range_routing_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
8. `cargo test --offline --test index_read_bounded_read_test --test index_write_atomicity_test --test insert_column_list_test --test insert_select_test --test isolation_level_test --test join_output_resolution_test --test join_test --test key_type_conformance_test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
9. `cargo test --offline --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test --test nested_loop_join_test --test network_protocol_test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
10. `cargo test --offline --test param_binding_test --test parser_test --test pg_integration_test --test pg_messages_test --test pg_protocol_test --test pipeline_test --test plan_cache_test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
11. `cargo test --offline --test projection_expression_test --test projection_test --test pushdown_test --test query_memory_test --test readonly_session_test --test recovery_bounded_read_test --test recovery_e2e_test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
12. `cargo test --offline --test release_durability_precondition_test --test release_path_bounded_read_test --test runtime_test --test scalar_function_test --test scalar_subquery_comparison_test --test schema_persistence_test --test secondary_index_equality_test --test secondary_index_test --test select_distinct_test --test self_join_test --test session_lifecycle_test --test set_operations_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
13. `cargo test --offline --test sort_merge_join_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test string_index_runtime_test --test string_index_sql_test --test string_secondary_index_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test --test subset_insert_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
14. `cargo test --offline --test sync_loader_test --test table_alias_resolution_test --test table_manager_test --test truncate_test --test tx_statement_test --test typed_api_test --test typed_batch_write_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。
15. `cargo test --offline --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_integration_test --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1`。退出码 0，failed 0。12 条摘要，每条 failed 为 0。

**对账**

用同一次枚举，文件名去掉末尾 `.rs`，与上面 14 条集成测试命令的 `--test` 名字按原顺序比较。164 个名字一致，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第 1 批。

**产品文件 diff**

`git diff --stat -- src tests Cargo.toml` 无输出。为空。

**Persisted Evidence**

None required

**Experience Candidates**

None

**未解决问题**

无。

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
