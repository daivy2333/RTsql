# Iteration 001 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `tasks.md` 第 2 节的分批规程。上一轮已接受的主命令行为只被这次全量观察，不在本轮修改。产品改动停在 `2054329`：`src/cli/mod.rs` 的 `run_sql` 在 `RTSQL_PROFILING` 已设置时为进入 plan 与 execute 的语句建立 scope 并调用现有的 `print_timings`；新增 `tests/cli_main_profiling_test.rs`。`src/pipeline.rs` 的既有打印点、import 和诊断渲染没有改动。
- Excluded scope: 产品代码、测试、夹具和构建配置的修改。全量失败后在本轮里修。无过滤的 `cargo test`。默认并行度。重跑已经通过的批。改分批规则。改 `openspec/specs/` 的权威正文。

**Objective**

按下面抄入的分批规程，把默认构建上的一次全量跑完。每一批退出码 0 且 failed 为 0。全部 `--test` 名字与执行时的枚举集合一致，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。

**Background**

上一轮只跑了两条定向测试，没有跑全量。全量预算是整个变更一次，放在这一轮。这一轮只执行验证，不产生新的实现。

**Investigation Facts**

- Starting point: 产品代码以 `2054329` 为准。该提交相对 `b1cec3c` 的产品 diff 只有 `src/cli/mod.rs` 和 `tests/cli_main_profiling_test.rs`。上一轮 Review Result 为 `accepted`，结论写在 `iterations/000-main-command-profiling/000-initial.md`。定向结果不代替本轮全量：`cargo test --offline --test cli_main_profiling_test -- --test-threads=1` 在取样点移动之后为退出码 0、0 failed、10 passed；`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 在取样点移动之前为退出码 0、0 failed、9 passed，该次移动没有碰到这条测试覆盖的代码。两条都来自上一轮 Act Response，本轮仍要把它们所在的文件放进枚举后的批次里实跑。
- Current-State Evidence: 集成测试文件在执行时用 `ls tests/*.rs | sort` 取得。文件名去掉目录和 `.rs` 就是 `--test` 的名字。`tests/cli_main_profiling_test.rs` 已经在工作区里，会进入这次枚举。规划时的 162 和「T1 新增一个文件」都不是通过条件。本轮不改代码，没有新的先行测试。
- Code and Critical Path: 本轮没有代码目标。执行对象是当前分支默认构建的 lib 测试、bin 测试，以及枚举得到的全部集成测试。失败时按失败测试所在的批和文件定位，不在本轮改代码。

**Implementation Guidance**

按 T2 里的规程逐条执行。每批在 Act Response 写批号、完整命令、退出码和 failed 计数。批次划分一并写出。不要另写脚本或封装命令来判定通过。

**Task Definitions**

### T2: 一次分批串行的全量

- Requirement/Scenario: `tasks.md` 的 T2。上一轮主命令行为在默认构建上被整套测试观察一次。本轮不产生新的行为要求。
- Depends on: T1。上一轮 Review Result 为 `accepted`。
- Targets: 无产品代码目标。执行对象是 lib 测试、bin 测试，以及执行时 `ls tests/*.rs | sort` 得到的全部集成测试。
- Current behavior: 本变更还没有跑过全量。上一轮只跑了 `cli_main_profiling_test` 和 `diagnostics_statement_test` 这两条定向命令。
- Required behavior: T1 的 Iteration Review Result 为 `accepted` 之后，按下面的分批方式跑一次全量。这是整个变更的唯一一次全量。不改产品代码。失败时停止，不在本轮里修。

  分批是通过条件的一部分：

  1. 执行时枚举，不把规划时的文件数写进通过条件。集成测试文件来自 `ls tests/*.rs | sort`。规划时这组文件是 162 个；T1 会新增 `tests/cli_main_profiling_test.rs`。这两个数字都不是通过条件。
  2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
  3. 其余文件按该排序每 12 个一批，最后一批可以少于 12。12 取 M19 经验值 8–12 的上沿。文件名去掉目录和 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。
  4. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次使用默认并行度。
  5. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
  6. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
  7. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败，不改并行度，不为了凑通过而重跑其余已通过的批。
- Preserve: 上面 1–7 的分批方式。产品代码、测试、夹具和构建配置。上一轮已接受的主命令结果、错误文本、SQLSTATE，以及 import 旧表和诊断默认输出。`openspec/specs/` 的权威正文。
- Forbidden: 改分批规则。不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。全量批次使用默认并行度。修改产品代码或测试。新增脚本、封装命令或判定层。用 Hash、revision、run-id 或时间顺序充当通过条件。重跑已经通过的批。跳过未跑的批后声明全量通过。本轮把失败修进代码。
- First test: 无先行测试。本轮不改代码。第一批就是第一条验证命令：`cargo test --offline --lib --bins -- --test-threads=1`。
- GREEN condition: 每一批退出码 0 且 failed 为 0，第 5 条对账成立，第 6 条的逐批记录写进 Act Response。
- Verification: 每一批以该条命令的退出码和 failed 计数判定。全量通过等于每一批都通过，并且对账成立。任一批未跑，不得声明全量通过。
- Stop when: 某一批第二次仍失败。对账出现缺失、多余或重复，或者 `--lib` / `--bins` 出现在第一批之外。执行时改了产品代码，或改了上面 1–7 的分批方式。

**Invariants**

- 全量只跑这一次。失败批只按同一命令再跑一次。
- 通过只看退出码、failed 计数和对账结果。
- 上一轮已接受的行为在本轮不改。

**Non-goals**

- 新行为。
- 在本轮修复全量里暴露的失败。
- 补跑已经通过的批。
- 无过滤全量，或默认并行度的全量。

**Acceptance**

1. 每一批退出码 0 且 failed 为 0。任一批未跑，不得声明全量通过。
2. 全部 `--test` 名字的集合等于 `ls tests/*.rs | sort` 去掉目录和 `.rs` 之后的集合，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。
3. 批间串行。每一批都带 `--test-threads=1`。没有一条不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。
4. Act Response 写有每一批的退出码和 failed 计数。
5. 本轮的产品代码和测试没有改动。

**Verification**

| 判定 | 命令 | 通过依据 |
|---|---|---|
| 第一批 | `cargo test --offline --lib --bins -- --test-threads=1` | 退出码 0，failed 为 0 |
| 其余各批 | `cargo test --offline --test <名> ... -- --test-threads=1` | 每一批退出码 0，failed 为 0；名字集合与执行时枚举一致 |

批的划分以执行时的 `ls tests/*.rs | sort` 为准，每 12 个一批。不在本文件里预写文件名单。

**Gate 2 Readiness**

Gate 2: PASS

**Persisted Evidence**

- Mode: none

每一批的命令、退出码和 failed 计数写入 Act Response 即可。这些数字可以在同一工作区重跑。没有哪一条验收必须靠留下来的日志才能判断。全量输出不整份保存。

**Risks and Notes**

- 某一批失败，只说明该批里的测试失败。不把失败当成修改 12 个一批这条规则的理由。
- 枚举里的文件即使在默认构建下编译出 0 个用例，只要退出码 0 且 failed 为 0，该批仍按第 6 条算通过，文件名仍要留在对账集合里。
- 上一轮两条定向测试的通过记录不能填进本轮的批次结果。它们会随 `ls tests/*.rs | sort` 进入对应批次。

## Act Response

- Status: reported

15 批全部退出码 0，failed 为 0。执行时 `ls tests/*.rs | sort` 得到 163 个名字，无重复。全部 `--test` 名字与这个序列相同，无缺失、无多余、无重复。`--lib` 和 `--bins` 只在第 1 批。产品代码、测试、夹具和构建配置没有改动。

第 1 批跑了两次，两次都是退出码 0、failed 为 0。第 2–15 批各跑一次。批与批之间没有重叠。

### 实际改动

没有改产品代码、测试、夹具或构建配置。本轮写入这份 Act Response，并把 `tasks.md` 里的 T2 标为完成。

没有先行测试。本文件的 T2 写明本轮不改代码，第一批就是第一条验证命令。

### 与计划的偏差

第 1 批第一次的退出码是 0。汇总行是 `443 passed; 0 failed` 和 `0 passed; 0 failed`。统计时把 `result::tests` 下的用例行 `test result::tests::... ... ok` 算进了汇总，于是同一条命令又跑了一次。第二次退出码仍是 0，汇总行仍是 `443 passed; 0 failed` 和 `0 passed; 0 failed`。没有第三次，也没有改并行度，没有重跑第 2–15 批。

### 自检

Gate 3：工作区相对执行起点没有产品 diff。每条命令都带 `--offline` 和 `--test-threads=1`，并且带 `--lib`、`--bins` 或 `--test`。第 1 批多跑一次记在上面，没有留下要改代码的缺陷。

Gate 4：下面的退出码和 failed 计数来自当次命令输出。任一批都已跑过。

Gate 5：没有阻塞。

遗留 Minor：无。

### 批次划分与对账

第 1 批单独执行 lib 与 bins。其余文件按枚举顺序每 12 个一批，第 15 批 7 个。第 2–14 批各 12 个 `--test`。这些 `--test` 名字按批连起来，与 `ls tests/*.rs | sort` 去掉目录和 `.rs` 后的序列一致。

第 4 批 12 条汇总里有一条是 `0 passed; 0 failed`。该批退出码 0，failed 合计 0。对应文件名留在该批命令里，也留在对账集合里。

### 逐批结果

1. `cargo test --offline --lib --bins -- --test-threads=1`

第一次退出码 0，failed 0。第二次退出码 0，failed 0。

2. `cargo test --offline --test aggregate_extension_test --test aggregate_output_shape_test --test aggregate_test --test alter_table_test --test attach_session_test --test btree_merge_test --test btree_scale_test --test btree_split_test --test btree_test --test busy_timeout_test --test cache_perf_test --test catalog_bounded_read_test -- --test-threads=1`

退出码 0，failed 0。

3. `cargo test --offline --test catalog_durability_test --test chain_join_test --test check_command_test --test checkpoint_command_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test checkpoint_test --test cli_main_profiling_test --test cli_test --test composite_index_lifecycle_test --test composite_index_runtime_test --test concurrent_test -- --test-threads=1`

退出码 0，failed 0。

4. `cargo test --offline --test connection_limit_test --test constraint_enforcement_test --test cross_db_dml_test --test cross_db_query_test --test crypto_off_build_test --test ctas_test --test cte_test --test data_page_bounded_read_test --test database_file_lock_test --test datetime_function_test --test datetime_type_test --test ddl_catalog_transactional_test -- --test-threads=1`

退出码 0，failed 0。

5. `cargo test --offline --test ddl_crash_recovery_test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test default_declaration_test --test default_precheck_test --test derived_scan_alias_test --test diagnostics_facade_test --test diagnostics_statement_test --test diagnostics_wait_test --test dml_tombstone_abandon_window_test --test dml_tx_id_test --test drop_persistence_test -- --test-threads=1`

退出码 0，failed 0。

6. `cargo test --offline --test drop_table_free_test --test e2e_test --test encryption_test --test executor_test --test explain_test --test explicit_tx_test --test expression_e2e_test --test file_header_test --test file_storage_io_test --test free_page_lifecycle_test --test full_table_dml_test --test gc_test -- --test-threads=1`

退出码 0，failed 0。

7. `cargo test --offline --test group_by_expr_test --test hash_join_build_side_test --test import_input_surface_test --test index_key_encoding_evaluation_test --test index_manager_test --test index_page_ownership_stack_overflow_test --test index_page_ownership_test --test index_range_routing_test --test index_read_bounded_read_test --test index_write_atomicity_test --test insert_column_list_test --test insert_select_test -- --test-threads=1`

退出码 0，failed 0。

8. `cargo test --offline --test isolation_level_test --test join_output_resolution_test --test join_test --test key_type_conformance_test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test -- --test-threads=1`

退出码 0，failed 0。

9. `cargo test --offline --test mvcc_tombstone_visibility_test --test nested_loop_join_test --test network_protocol_test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test --test param_binding_test --test parser_test --test pg_integration_test --test pg_messages_test -- --test-threads=1`

退出码 0，failed 0。

10. `cargo test --offline --test pg_protocol_test --test pipeline_test --test plan_cache_test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test --test projection_expression_test --test projection_test --test pushdown_test --test query_memory_test -- --test-threads=1`

退出码 0，failed 0。

11. `cargo test --offline --test readonly_session_test --test recovery_bounded_read_test --test recovery_e2e_test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test --test release_durability_precondition_test --test release_path_bounded_read_test --test runtime_test --test scalar_function_test -- --test-threads=1`

退出码 0，failed 0。

12. `cargo test --offline --test scalar_subquery_comparison_test --test schema_persistence_test --test secondary_index_equality_test --test secondary_index_test --test select_distinct_test --test self_join_test --test session_lifecycle_test --test set_operations_test --test sort_merge_join_test --test sort_test --test sql_differential_test --test statement_cancellation_test -- --test-threads=1`

退出码 0，failed 0。

13. `cargo test --offline --test storage_test --test string_index_runtime_test --test string_index_sql_test --test string_secondary_index_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test --test subset_insert_test --test sync_loader_test --test table_alias_resolution_test --test table_manager_test --test truncate_test -- --test-threads=1`

退出码 0，failed 0。

14. `cargo test --offline --test tx_statement_test --test typed_api_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test -- --test-threads=1`

退出码 0，failed 0。

15. `cargo test --offline --test wal_integration_test --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1`

退出码 0，failed 0。

### Persisted Evidence

None required。没有创建 evidence 目录，没有保存完整测试输出。

### Experience Candidates

None

### 未解决问题

无。

## Plan Review

- Review Result: pending
