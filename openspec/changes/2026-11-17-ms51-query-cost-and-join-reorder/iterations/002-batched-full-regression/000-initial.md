# Iteration 002 / Cycle 000: 一次分批全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `tasks.md` 的 Iteration 002，以及 `.agents/runbooks/m19-batched-full-regression.md` 的等分批次规则。通过条件仍是每一批退出码 0，且该批 `test result:` 行的 failed 为 0。
- Excluded scope: 不新增产品行为。不改代价公式、叶序规则和 `JoinOrder` 文本。不修改 `openspec/specs/` 或 `.agents/`。不把全量再安排到 000 或 001。不预算第二次全量。

**Objective**

按执行时的 `tests/*.rs` 把集成测试等分批次跑完，`--lib --bins` 单独成最后一批。每一批退出码 0，且该批 `test result:` 行的 failed 为 0。这次运行用掉整个变更仅有的一次全量预算。

**Investigation Facts**

- Starting point: 001 的实现提交是 `cb232abcf8309847b785c9ade1f0db9df1d07c57`。合格查询会按估计更换左深叶序，EXPLAIN 在首行 `EXPLAIN` 之后插入 `JoinOrder estimated` 或 `JoinOrder written`。没有可用统计、以及无 ORDER BY、两表、外连接和 LIMIT，保持书写叶序且不插入该行。001 的 Act Response 记录了最终定向结果：`cargo test --offline --test join_reorder_test -- --test-threads=1` 退出码 0，8 passed，0 failed；`cargo test --offline --test table_statistics_test -- --test-threads=1` 退出码 0，12 passed，0 failed；`cargo test --offline --lib table_stats -- --test-threads=1` 退出码 0，14 passed，0 failed，446 filtered out。这三条覆盖到本 Cycle 展开时没有失效。本 Cycle 的计划提交不改产品代码。Act 从该计划提交开始。全量尚未为本次变更运行。
- Current-State Evidence:
  - 展开时 `ls tests/*.rs | wc -l` 的结果是 169。其中 `tests/table_statistics_test.rs` 与 `tests/join_reorder_test.rs` 是 000 和 001 新增的。这个 169 只说明展开时的现场。执行时重新 `ls tests/*.rs | sort`，不沿用 169，也不沿用计划里曾经写过的 167。
  - 等分规则：文件数 `N`，文件批数 `B = ceil(N / 12)`。按字典序，前 `B × 12 − N` 批各 11 个文件，其余各 12 个。`--lib --bins` 是第 `B + 1` 批。展开时的演算是 `N = 169`、`B = 15`、前 11 批各 11 个文件、后 4 批各 12 个文件，加上 lib 与 bin 共 16 批。文件数变化后这组数字作废。
  - 每一批是一次独立的 `cargo test`。批内 `--test-threads=1`。上一批进程退出后才启动下一批。`grep -E "^test result:"` 只用来读出 cargo 已经打印的行。判定看该次 cargo 的退出码，以及这些行里的 failed。
  - `--lib --bins` 一批通常打出 lib 与 bin 各一行 `test result:`。目标数是 `N` 个文件加上 lib 与 bin 两个目标。行数与目标数在这个形态下相同。两个数都写入 Act Response。旧的 2231 passed、143 个目标不是本轮通过条件。
  - 诊断边界仍是本变更的统计、代价和叶序。`tests/self_join_test.rs` 不调用 `ANALYZE`。没有统计的既有连接仍走书写装配。
- Code and Critical Path: 本任务的入口是上面的 `cargo test` 批次，不是新的产品符号。产品行为已经在 `src/table_stats.rs`、`src/pipeline.rs` 的 `plan_stage_impl` 与 `bump_schema_version`，以及 `src/parser/planner/query.rs` 的 `decide_leaf_order` 里。全量只观察这些行为有没有破坏既有测试。

**Implementation Guidance**

先在工作区根目录执行 `ls tests/*.rs | sort`，按上面的公式切批，并核对文件名总数等于 `N`、排序后与 `ls` 的输出一致、没有文件进入两批。对账不成立时不要启动 cargo。

文件批全部跑完后，再跑 `--lib --bins`。某一批失败时，先单独跑失败的那个测试，再把该批原样再跑一次。两次都带 `--test-threads=1`。复核通过则该批记通过，并写明首跑失败。cargo 在批内第一个失败后不会再跑后面的测试二进制，所以复核必须是整批，不能只补后面没跑到的名字就当成整批通过。

复核仍失败，且失败来自统计、代价或叶序时，按 `specs/query-cost-and-join-order/spec.md`、`specs/explain-plan-output/spec.md` 和 `specs/sql-parameter-binding/spec.md` 修正。修正前先看到该失败测试是红的。修正后只重跑这个定向测试，命令带 `--test-threads=1`。不要为此再跑其余批次，也不要再排一次全量。失败与这些行为无关时，不要改其他模块。

**Task Definitions**

### T4: 按等分批次做一次全量

- Requirement/Scenario: 「既有语义零回归」。未收集统计时，既有查询和 EXPLAIN 保持本变更前的结果与节点行。
- Depends on: Iteration 001 accepted
- Targets: 无预定产品文件。只有某一批复核后仍失败，且失败来自统计、代价或叶序时，才修改对应的既有实现。
- Current behavior: 001 的三条定向测试已经通过。全量还没有为本次变更运行。
- Required behavior:
  - 执行时的 `ls tests/*.rs | sort` 与批次中的文件名比对，missing、extra、duplicate 都是 0。
  - 每个文件批的命令是 `cargo test --test <f1> --test <f2> ... -- --test-threads=1`。最后一批是 `cargo test --lib --bins -- --test-threads=1`。
  - 每一批退出码 0，且该批 `test result:` 行的 failed 为 0。Act Response 留下批号、目标、退出码、passed、failed、ignored。
  - 目标数与 `test result:` 行数都写明。lib 与 bin 各占一行时，这两个数相同。
- Preserve: 批次算法、`--test-threads=1` 和「整个变更只跑一次全量」。没有统计的语句不因本轮改动而改变结果。
- Forbidden: 一次无过滤的 `cargo test`；去掉 `--test-threads=1`；并行启动多批；全量失败后再跑一整轮；新增判定脚本或把判定写进测试框架之外的程序；为了对上旧的 passed 数字而改断言；修改 `openspec/specs/` 或 `.agents/`；改与本变更无关的失败。
- First test: 不新增测试。第一批文件批就是本任务的第一项观察。启动前只做 `ls` 与切批对账，不对账成功不跑 cargo。
- GREEN condition: 全部分批各退出码 0，且各批 `test result:` 的 failed 为 0。某一批首跑失败、整批复核通过时，该批按通过记，并写明首跑失败。
- Verification: 输入是仓库里的测试目标，断言是 cargo 的退出码和 `test result:` 行。计数管道只用 `grep -E "^test result:"`。
- Stop when: 批次清单与执行时的文件集合不一致；失败与统计、代价、叶序无关；修正后仍要靠第二次全量才能声称通过。这些情况停止并回到计划。

**Invariants**

- 全量结论等于每一批各自 failed 为 0。缺一批不能写成全量通过。
- 任何测试命令都带 `--test-threads=1`。批与批串行。
- 定向测试不算这次全量预算。返工不得再预算第二次全量。

**Non-goals**

不新增产品行为。不预建缺陷轮次。不把全量拆回 000 或 001。不交换连接的左右输入。

**Acceptance**

- 执行时的测试文件每个正好出现在一个文件批里。
- `--lib --bins` 单独成功一批。
- 每一批退出码 0，failed 为 0。
- Act Response 能按批核对退出码和 passed、failed、ignored。
- 本次变更之后没有第二份全量批次表。

**Verification**

按 Task Definition 的命令串行执行。通过与否只看各次 cargo 的退出码和 `test result:` 行。不使用无过滤的 `cargo test`。不把 001 的三条定向命令在全量之外再跑一遍。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。目标注入同时批准了本轮计划。这不是跳过。

**Persisted Evidence**

- Mode: none

各批退出码和计数写入 Act Response。批次表可以低成本按同一公式重算，但通过结论不能靠再跑一次全量来补。

**Risks and Notes**

- 展开时的 169 和 16 批会过期。执行时以当时的 `ls` 为准。
- 某一批因本变更失败并被定向测试修好之后，其余批次仍是修改前的结果。设计已经接受这个缺口，因为第二次全量不在预算内。
- 与本变更无关的失败不在本轮修。

## Act Response

Status: reported

T4 已完成。16 批全部退出码 0，各批 `test result:` 汇总行的 failed 都是 0。Gate 3 与 Gate 4 通过。Gate 5 没有命中。没有修改产品代码。

### 实际改动

产品代码、`openspec/specs/` 和 `.agents/` 都没有改。

`tasks.md` 里 T4 的状态改为已实施，并勾选 4.1。002 的状态改为 Act Response 已写入，Review 尚未写入。

### 对账

执行时 `ls tests/*.rs | sort` 是 169 个文件，排序检查通过，文件名没有重复。`B = ceil(169 / 12) = 15`。前 `15 × 12 − 169 = 11` 批各 11 个文件，后 4 批各 12 个。第 16 批是 `--lib --bins`。

批次并集与这次排序后的 `ls` 逐行相同。missing 0，extra 0，duplicate 0。11 × 11 + 12 × 4 = 169。

### 与计划的偏差

无。没有首跑失败，没有单独复核，没有改统计、代价或叶序，也没有第二份全量。

### 自检

Gate 3 没有产品 diff。既有测试保持通过，没有为了对上旧的 passed 数字改断言。每一批都是独立的 `cargo test`，参数里都有 `--test-threads=1`。上一批进程退出后才启动下一批。没有无过滤的 `cargo test`，也没有把 001 的三条定向命令在全量之外再跑一遍。

自检没有留下 Critical、Important 或 Minor。

### 验证

目标数是 171：169 个文件，加上 lib 与 bin。汇总行是以 `test result: ok.` 开头的行，共 171 行，与目标数相同。合计 2504 passed，0 failed，2 ignored。这 2 个 ignored 都在第 2 批的 `cli_test`（该目标 126 passed，2 ignored）。

第 16 批的字面 `grep -E "^test result:"` 还有 2 行进度，测试名以 `result::tests::` 开头：`value_to_json_preserves_conversion_semantics` 和 `value_to_json_renders_datetime_family_as_da5_string`，两行都是 `ok`。它们没有 failed 字段。该批的汇总行是 lib 的 460 passed、0 failed、0 ignored，以及 bin 的 0 passed、0 failed、0 ignored。

| 批 | 退出码 | passed | failed | ignored | 汇总行 | 目标 |
|---|---|---|---|---|---|---|
| 1 | 0 | 149 | 0 | 0 | 11 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test |
| 2 | 0 | 195 | 0 | 2 | 11 | catalog_bounded_read_test, catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_main_profiling_test, cli_test, composite_index_lifecycle_test |
| 3 | 0 | 130 | 0 | 0 | 11 | composite_index_runtime_test, concurrent_test, connection_limit_test, constraint_enforcement_test, cross_db_dml_test, cross_db_query_test, crypto_off_build_test, ctas_test, cte_test, data_page_bounded_read_test, database_file_lock_test |
| 4 | 0 | 125 | 0 | 0 | 11 | datetime_function_test, datetime_type_test, ddl_catalog_transactional_test, ddl_crash_recovery_test, ddl_layout_publication_test, ddl_rewrite_consistency_test, decimal_blob_test, default_declaration_test, default_precheck_test, derived_scan_alias_test, diagnostics_comparison_test |
| 5 | 0 | 139 | 0 | 0 | 11 | diagnostics_facade_test, diagnostics_statement_test, diagnostics_wait_test, dml_tombstone_abandon_window_test, dml_tx_id_test, drop_persistence_test, drop_table_free_test, e2e_test, encryption_test, executor_test, explain_test |
| 6 | 0 | 133 | 0 | 0 | 11 | explicit_tx_test, expression_e2e_test, file_header_test, file_storage_io_test, free_page_lifecycle_test, full_table_dml_test, gc_test, group_by_expr_test, hash_join_build_side_test, import_input_surface_test, index_key_encoding_evaluation_test |
| 7 | 0 | 81 | 0 | 0 | 11 | index_manager_test, index_page_ownership_stack_overflow_test, index_page_ownership_test, index_range_routing_test, index_read_bounded_read_test, index_write_atomicity_test, insert_column_list_test, insert_select_test, isolation_level_test, join_output_resolution_test, join_reorder_test |
| 8 | 0 | 94 | 0 | 0 | 11 | join_test, key_type_conformance_test, keyless_eq_routing_test, keyless_row_test, legacy_compat_test, limit_test, memory_db_test, mvcc_abort_test, mvcc_commit_test, mvcc_record_test, mvcc_tombstone_visibility_test |
| 9 | 0 | 122 | 0 | 0 | 11 | nested_loop_join_test, network_protocol_test, network_server_test, no_from_select_test, order_by_alias_sort_test, outer_join_test, overflow_page_test, param_binding_test, parser_test, pg_integration_test, pg_messages_test |
| 10 | 0 | 224 | 0 | 0 | 11 | pg_protocol_test, pipeline_test, plan_cache_test, plan_exec_test, planner_test, predicate_test, prefetch_test, prepared_statement_test, projection_expression_test, projection_test, pushdown_test |
| 11 | 0 | 68 | 0 | 0 | 11 | query_memory_test, readonly_session_test, recovery_bounded_read_test, recovery_e2e_test, recovery_error_surface_test, recovery_replay_convergence_test, recovery_tail_persistence_test, recovery_test, recovery_wal_convergence_test, release_durability_precondition_test, release_path_bounded_read_test |
| 12 | 0 | 166 | 0 | 0 | 12 | result_consumption_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test, schema_persistence_test, secondary_index_equality_test, secondary_index_test, select_distinct_test, self_join_test, session_lifecycle_test, set_operations_test, sort_merge_join_test |
| 13 | 0 | 158 | 0 | 0 | 12 | sort_test, sql_differential_test, statement_cancellation_test, storage_test, string_index_runtime_test, string_index_sql_test, string_secondary_index_test, structural_property_test, subquery_alias_binding_test, subquery_test, subset_insert_test, sync_loader_test |
| 14 | 0 | 162 | 0 | 0 | 12 | table_alias_resolution_test, table_manager_test, table_statistics_test, truncate_test, tx_statement_test, typed_api_test, typed_batch_write_test, update_expression_test, update_index_maintenance_test, upsert_test, value_test, version_chain_test |
| 15 | 0 | 98 | 0 | 0 | 12 | view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test, wal_record_test, wal_recovery_large_test, wal_writer_test, write_failure_state_test, write_path_slot_bounds_test, write_type_conformance_test |
| 16 | 0 | 460 | 0 | 0 | 2 | `--lib`，`--bins` |

文件批的命令是 `cargo test --test <上表目标> -- --test-threads=1`。第 16 批的命令是 `cargo test --lib --bins -- --test-threads=1`。旧的 2231 passed、143 个目标不是本轮通过条件。

Persisted Evidence：None required

Experience Candidates：None

未解决问题：无。
