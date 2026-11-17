# Iteration 001 / Cycle 000: 一次分批全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `proposal.md` 的维持合同，`design.md` 的 D5，`tasks.md` 里 Iteration 001 的验证边界。上一轮 `iterations/000-record-and-keep/000-initial.md` 的 Review Result 为 `accepted` 后，本轮才展开。M19 的分批和 `--test-threads=1` 继续有效。
- Excluded scope: 不改刷盘和驱逐，也不改恢复或文件锁。不重跑计时。不加 `--ignored`。不写新测试或判定脚本，也不做第二遍全量。不预建修复轮次。不建第二个变更。

**Objective**

在没有产品改动的树上，按下面的切分把现有测试跑完一次。每一批 cargo 退出码为 0，且该批每一行 `test result:` 的 failed 为 0。这次全量是整个目标里唯一的一次。

**Investigation Facts**

- Starting point: 上一轮 Act Response 没有修改产品符号。该轮验证时 HEAD 是 `3f95a874fd45ef55bf844a80ceb7e9aeac1357fc`，`git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml` 无输出。Review 又在 `c9ff98e0a24dab4b18abc1f4f88b3573f57d15bb` 上做了只读核对，产品路径仍无差异。计时和锁等待沿用上一轮 Act Response，不重跑。本 Cycle 的计划提交落在该 HEAD 之后，仍然不改 `src/`、`tests/` 和 `Cargo.toml`。Act 从包含本文件的提交所在工作区开始。
- Current-State Evidence:
  - 全量还没有在本变更里跑过。通过条件不是 runbook 里记载的历史 passed 总数。那些数字属于别的工作区。
  - 计划写下时，在本工作区执行 `ls tests/*.rs | sort | wc -l` 得到 169。按等分规则这是 15 个文件批（11 批各 11 个文件，4 批各 12 个文件）再加单独的 `--lib --bins`，共 16 批。169 只说明写下时公式适用。执行时重新数，不沿用 169。
  - 集成测试目标名是文件名去掉目录和 `.rs`。`tests/cli_test.rs` 对应 `--test cli_test`。
  - `--lib --bins` 一批会打出两行 `test result:`，一行属于库，一行属于二进制。文件批通常一行。通过条件看每一行的 failed，不要求行数等于 1。
  - `cli_test.rs` 里长期 `#[ignore]` 的校准用例不在普通 `cargo test` 里运行。本轮命令不带 `--ignored`，避免把上一轮已经沿用的计时再跑一遍。
- Code and Critical Path:
  - 本轮没有产品符号要改。入口是本工作区根目录的 `cargo test`。工作区是 `/home/daivy/projects/RTsql-changes/2026-11-17-ms52-torn-tree-and-cross-process`。不要在主仓库里跑。
  - 调用链停在测试进程。失败时读取失败目标自己的断言。缓冲池、恢复和文件锁没有本变更的改动，不在本轮里修。

**Implementation Guidance**

先取得当次文件清单并切批，对账通过后再启动 cargo。每一批等上一批进程退出后再启动。全量中途不要改文件清单的切法。

某一批第一次失败时，cargo 不会再跑该批里排在失败目标后面的文件。允许把这一整批再跑一次，用来区分偶发失败。复核通过则记下首跑失败，并继续下一批。复核仍失败则停止。不要只重跑失败的那一个目标，也不要把它算成另一次全量。

**Task Definitions**

### T3: 跑一次分批全量

- Requirement/Scenario: `proposal.md` 的维持合同，加上 `tasks.md` Iteration 001 的验证边界。全量不产生新的产品行为。
- Depends on: 上一轮 Review Result 为 `accepted`。T1 和 T2 已记入该轮 Act Response。
- Targets: 不修改源码。命令在上述工作区根目录执行。对象是当次 `ls tests/*.rs | sort` 的文件，以及单独的 `--lib` 和 `--bins`。
- Current behavior: 本变更还没有跑过这套分批全量。`src/`、`tests/` 和 `Cargo.toml` 相对 `0019bf127df5304f26e89927e336150fc6472b11` 没有差异。
- Required behavior: 开跑前 missing、extra 和 duplicate 都是 0。正常路径上每一批启动一次 cargo。某一批首跑失败时，只允许把该整批再启动一次；复核通过后，该批按通过计，并继续后面的批次。作为通过依据的那一次，cargo 退出码为 0，且该次输出里每一行 `test result:` 的 failed 为 0。文件批覆盖当次清单里的每个文件恰好一次。全部批次跑完后，再次 `ls tests/*.rs | sort` 与开跑前的清单一致。缺一批、缺少 `test result:` 行，或清单在中途变化，都不能声明全量通过。
- Preserve: `src/`、`tests/` 和 `Cargo.toml` 相对上一轮结束时的内容不变。每条 `cargo test` 带 `--test-threads=1`。批与批串行，同一时刻只有一批。普通套件不包含 `--ignored`。
- Forbidden: 无过滤的 `cargo test`。默认并行度。第二遍全量，含以后的返工再预算一次分批全量。为了让某批通过去改产品代码。把判定写成单独的脚本。`--ignored`。重跑上一轮的计时。在主仓库执行。
- First test: 不新写测试。没有产品代码变更，所以没有另一次 RED。第一批就是：

```text
cargo test --lib --bins -- --test-threads=1
```

- GREEN condition: 全部批次都满足 Required behavior，并且下面的产品 diff 仍无输出。
- Verification: 在工作区根目录按顺序做完这些事。判定只看 cargo 的退出码、`test result:` 行和产品 diff。
  1. `ls tests/*.rs | sort` 得到有序列表。`N` 是行数。`B = ceil(N / 12)`，整数运算写成 `(N + 11) / 12`。11 个文件的批数是 `B × 12 - N`。12 个文件的批数是 `N - 11 × B`。按字典序，前面的文件批各取 11 个文件，后面的各取 12 个。`--lib --bins` 不进入这 `B` 批，单独作为第一批。
  2. 开跑前核对：批次表里的文件与这次 `ls` 的结果一致，每个文件恰好一次。missing、extra 和 duplicate 都是 0。对账失败就停止，不要启动 cargo，这样唯一的一次全量不会花在漏目标的批次表上。不要另写对账脚本。
  3. 第一批运行上面的 `--lib --bins` 命令。随后按切好的顺序运行文件批。文件批的形式是 `cargo test --test <目标名> --test <目标名> ... -- --test-threads=1`。目标名是去掉目录和 `.rs` 后的名字。正常路径上每批启动一次。退出码是该 cargo 进程的退出码。`test result:` 行从同一次输出里抄出。输出很长时只保留这些行。可以用一步 `grep -E "^test result:"` 阅读同一次输出。管道的退出码不能代替 cargo 的退出码，也不要为了计数再启动一次 cargo。首跑失败时的那一次整批复核是唯一允许的额外启动。
  4. 全部批次结束后再执行一次 `ls tests/*.rs | sort`，并执行：

```text
git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml
```

- Stop when: 12 个文件的批数小于 0。此时停止并回到计划，不要改用别的切法。同一批第二次仍失败。失败要求修改缓冲池、恢复、文件锁或其他产品代码。清单在开跑后发生变化。以上任一情况都停止并报告已经跑到的批次和失败目标。不在本任务里开修复轮次，也不把剩余批次或整次分批再跑一遍。

**Invariants**

- 整个目标只有这一次分批全量。本 Cycle 用掉它。失败批次的一次整批复核属于这一次内部的偶发判别，不是第二次全量。以后的返工不得再排一次分批全量。
- 每条 `cargo test` 带 `--test-threads=1`。同一时刻只有一批。
- 不修改 `src/`、`tests/` 和 `Cargo.toml`。不新增行为规格，不改 `openspec/specs/`。

**Non-goals**

不因为全量失败去改刷盘和驱逐，也不改恢复或文件锁。不预建修复轮次。不重跑计时。不把历史 passed 总数对齐成通过条件。

**Acceptance**

- 当次文件清单按上面的等分规则切完，开跑前 missing、extra 和 duplicate 都是 0。
- 第一批是 `--lib --bins`，其后是字典序文件批。作为通过依据的那一次，退出码为 0，且每一行 `test result:` 的 failed 为 0。
- 若有一批首跑失败且整批复核通过，Act Response 同时记下首跑失败和复核通过。除此之外没有第二遍。
- 跑完后的 `ls tests/*.rs | sort` 与开跑前一致。产品 diff 无输出。
- Act Response 留下每一批的目标列表、cargo 退出码和 `test result:` 行。缺一批就不能声明通过。

**Verification**

- 维持合同的通过依据是产品 diff 无输出，以及本轮没有修改产品代码。
- 全量的通过依据是每一批 cargo 进程退出码 0，和同一次输出里每一行 `test result:` 的 failed 为 0。
- 这些命令就是真实的 `cargo test`。不另做判定脚本。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。计划由本次目标的批准离开 draft。

**Persisted Evidence**

- Mode: none

各批的目标列表、退出码和 `test result:` 行写入 Act Response。这些行决定全量是否通过。不另存日志目录。全量不能靠重跑来补记录，所以响应里必须留下当次那几行。

**Risks and Notes**

- 执行时文件数若不是 169，按当次 `ls` 重算。12 个文件的批数若小于 0，属于 Stop when，不要在本轮另选切法。
- 某一批第一次失败后，该批后面的文件还没跑。复核必须重跑整批。
- 历史 passed 总数不是本轮阈值。ignored 可以不是 0。通过只看 failed 为 0 和退出码 0。
- 全量失败时不改产品代码。空闲页重建放弃仍不登记，也不为本轮预建修复。

## Act Response

Status: reported

T3 已完成。16 批的通过依据都是退出码 0，且同一次输出里每一行 `test result:` 摘要的 failed 为 0。Gate 3 与 Gate 4 通过。Gate 5 没有命中。`src/`、`tests/` 和 `Cargo.toml` 相对 `0019bf127df5304f26e89927e336150fc6472b11` 没有差异。跑完后的 `ls tests/*.rs | sort` 与开跑前一致。

### 实际改动

没有修改产品符号。本文件替换了占位执行记录。`tasks.md` 里 T3 的状态改为已完成，001 的执行记录改为 reported。Review Result 仍是 pending。

### 与计划的偏差

主机有 32 个逻辑处理器、7.6GiB 内存，交换区已经占满。编译作业数设为 2，避免系统在编译时杀掉进程，使这次全量中断。每条 `cargo test` 的目标参数仍按任务定义，并且带 `--test-threads=1`。输出颜色设为 `CARGO_TERM_COLOR=never`。执行环境注入了 `CARGO_TARGET_DIR`，命令里清掉了它，产物写在本工作区的 `target/`。

第 1 批第一次 `cargo test --lib --bins -- --test-threads=1` 的退出码是 0。两条摘要是 `460 passed; 0 failed` 和 `0 passed; 0 failed`。当时用 `^test result:` 读取摘要，这个模式还命中了单元测试行 `test result::tests::value_to_json_preserves_conversion_semantics` 和 `test result::tests::value_to_json_renders_datetime_family_as_da5_string`。脚本把这次结果当成失败，把同一整批又启动了一次。第二次退出码也是 0，摘要是 `460 passed; 0 failed`（16.57s）和 `0 passed; 0 failed`（0.00s）。脚本随后停住，第 2 批还没有启动。

通过依据采用第一次。摘要匹配改为冒号后有空格的 `test result: `。第 1 批没有第三次启动。第 2 批到第 16 批各启动一次。多出来的那一次只覆盖第 1 批。16 批全量没有再跑一遍。

### 自检

Gate 3 没有留下 Critical 或 Important，也没有遗留 Minor。产品树没有计划外修改。没有新增测试或判定脚本。`cli_test` 的 `2 ignored` 留在普通套件之外。

`crypto_off_build_test` 整文件处于 `#![cfg(not(feature = "crypto"))]` 之下，默认特性下摘要是 0 passed、0 failed。`network_server_test` 的测试代码在文件里被注释掉，摘要同样是 0 passed、0 failed。

### 验证

开跑前 `ls tests/*.rs | sort` 有 169 行。`B = (169 + 11) / 12 = 15`。11 个文件的批是 `15 × 12 - 169 = 11`。12 个文件的批是 `169 - 11 × 15 = 4`。批次表与这次清单逐项一致，missing、extra、duplicate 都是 0。12 个文件的批数是 4，停止条件没有命中。

批与批串行。命令没有 `--ignored`，也没有无过滤的 `cargo test`。下面每一批的摘要行按目标启动顺序排列，取自同一次 cargo。除第 1 批记录偏差里的那一次额外启动外，每批只有这一次。

第 1 批，退出码 0。`cargo test --lib --bins -- --test-threads=1`

```text
test result: ok. 460 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.91s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

第 2 批，退出码 0。`cargo test --test aggregate_extension_test --test aggregate_output_shape_test --test aggregate_test --test alter_table_test --test attach_session_test --test btree_merge_test --test btree_scale_test --test btree_split_test --test btree_test --test busy_timeout_test --test cache_perf_test -- --test-threads=1`

```text
aggregate_extension_test
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.95s
aggregate_output_shape_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.86s
aggregate_test
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
alter_table_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
attach_session_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.27s
btree_merge_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
btree_scale_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.95s
btree_split_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
btree_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
busy_timeout_test
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.18s
cache_perf_test
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
```

第 3 批，退出码 0。`cargo test --test catalog_bounded_read_test --test catalog_durability_test --test chain_join_test --test check_command_test --test checkpoint_command_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test checkpoint_test --test cli_main_profiling_test --test cli_test --test composite_index_lifecycle_test -- --test-threads=1`

```text
catalog_bounded_read_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
catalog_durability_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 24.19s
chain_join_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
check_command_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.95s
checkpoint_command_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.05s
checkpoint_redo_reduction_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
checkpoint_skip_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
checkpoint_test
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
cli_main_profiling_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.82s
cli_test
test result: ok. 126 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 76.06s
composite_index_lifecycle_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

第 4 批，退出码 0。`cargo test --test composite_index_runtime_test --test concurrent_test --test connection_limit_test --test constraint_enforcement_test --test cross_db_dml_test --test cross_db_query_test --test crypto_off_build_test --test ctas_test --test cte_test --test data_page_bounded_read_test --test database_file_lock_test -- --test-threads=1`

```text
composite_index_runtime_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.10s
concurrent_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
connection_limit_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.82s
constraint_enforcement_test
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s
cross_db_dml_test
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.40s
cross_db_query_test
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.27s
crypto_off_build_test
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
ctas_test
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.69s
cte_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
data_page_bounded_read_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
database_file_lock_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

第 5 批，退出码 0。`cargo test --test datetime_function_test --test datetime_type_test --test ddl_catalog_transactional_test --test ddl_crash_recovery_test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test decimal_blob_test --test default_declaration_test --test default_precheck_test --test derived_scan_alias_test --test diagnostics_comparison_test -- --test-threads=1`

```text
datetime_function_test
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.45s
datetime_type_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.18s
ddl_catalog_transactional_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
ddl_crash_recovery_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.43s
ddl_layout_publication_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.62s
ddl_rewrite_consistency_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.37s
decimal_blob_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
default_declaration_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
default_precheck_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s
derived_scan_alias_test
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.90s
diagnostics_comparison_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

第 6 批，退出码 0。`cargo test --test diagnostics_facade_test --test diagnostics_statement_test --test diagnostics_wait_test --test dml_tombstone_abandon_window_test --test dml_tx_id_test --test drop_persistence_test --test drop_table_free_test --test e2e_test --test encryption_test --test executor_test --test explain_test -- --test-threads=1`

```text
diagnostics_facade_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
diagnostics_statement_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.27s
diagnostics_wait_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.46s
dml_tombstone_abandon_window_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
dml_tx_id_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
drop_persistence_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.86s
drop_table_free_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.78s
e2e_test
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 48.37s
encryption_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.08s
executor_test
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
explain_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

第 7 批，退出码 0。`cargo test --test explicit_tx_test --test expression_e2e_test --test file_header_test --test file_storage_io_test --test free_page_lifecycle_test --test full_table_dml_test --test gc_test --test group_by_expr_test --test hash_join_build_side_test --test import_input_surface_test --test index_key_encoding_evaluation_test -- --test-threads=1`

```text
explicit_tx_test
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
expression_e2e_test
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
file_header_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.56s
file_storage_io_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
free_page_lifecycle_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
full_table_dml_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.33s
gc_test
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
group_by_expr_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.04s
hash_join_build_side_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
import_input_surface_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.86s
index_key_encoding_evaluation_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.75s
```

第 8 批，退出码 0。`cargo test --test index_manager_test --test index_page_ownership_stack_overflow_test --test index_page_ownership_test --test index_range_routing_test --test index_read_bounded_read_test --test index_write_atomicity_test --test insert_column_list_test --test insert_select_test --test isolation_level_test --test join_output_resolution_test --test join_reorder_test -- --test-threads=1`

```text
index_manager_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
index_page_ownership_stack_overflow_test
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
index_page_ownership_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.42s
index_range_routing_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
index_read_bounded_read_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.09s
index_write_atomicity_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
insert_column_list_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
insert_select_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.85s
isolation_level_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
join_output_resolution_test
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.64s
join_reorder_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.09s
```

第 9 批，退出码 0。`cargo test --test join_test --test key_type_conformance_test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test -- --test-threads=1`

```text
join_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
key_type_conformance_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
keyless_eq_routing_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
keyless_row_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
legacy_compat_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s
limit_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
memory_db_test
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s
mvcc_abort_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
mvcc_commit_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
mvcc_record_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
mvcc_tombstone_visibility_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

第 10 批，退出码 0。`cargo test --test nested_loop_join_test --test network_protocol_test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test --test param_binding_test --test parser_test --test pg_integration_test --test pg_messages_test -- --test-threads=1`

```text
nested_loop_join_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
network_protocol_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
network_server_test
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
no_from_select_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
order_by_alias_sort_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
outer_join_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
overflow_page_test
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.35s
param_binding_test
test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
parser_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
pg_integration_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.23s
pg_messages_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

第 11 批，退出码 0。`cargo test --test pg_protocol_test --test pipeline_test --test plan_cache_test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test --test projection_expression_test --test projection_test --test pushdown_test -- --test-threads=1`

```text
pg_protocol_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
pipeline_test
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
plan_cache_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
plan_exec_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
planner_test
test result: ok. 107 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
predicate_test
test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
prefetch_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s
prepared_statement_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
projection_expression_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
projection_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
pushdown_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
```

第 12 批，退出码 0。`cargo test --test query_memory_test --test readonly_session_test --test recovery_bounded_read_test --test recovery_e2e_test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test --test release_durability_precondition_test --test release_path_bounded_read_test -- --test-threads=1`

```text
query_memory_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
readonly_session_test
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.35s
recovery_bounded_read_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
recovery_e2e_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
recovery_error_surface_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
recovery_replay_convergence_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
recovery_tail_persistence_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
recovery_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
recovery_wal_convergence_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s
release_durability_precondition_test
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
release_path_bounded_read_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.87s
```

第 13 批，退出码 0。`cargo test --test result_consumption_test --test runtime_test --test scalar_function_test --test scalar_subquery_comparison_test --test schema_persistence_test --test secondary_index_equality_test --test secondary_index_test --test select_distinct_test --test self_join_test --test session_lifecycle_test --test set_operations_test --test sort_merge_join_test -- --test-threads=1`

```text
result_consumption_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
runtime_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
scalar_function_test
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
scalar_subquery_comparison_test
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
schema_persistence_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
secondary_index_equality_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 31.19s
secondary_index_test
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
select_distinct_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
self_join_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
session_lifecycle_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.08s
set_operations_test
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
sort_merge_join_test
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

第 14 批，退出码 0。`cargo test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test string_index_runtime_test --test string_index_sql_test --test string_secondary_index_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test --test subset_insert_test --test sync_loader_test -- --test-threads=1`

```text
sort_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
sql_differential_test
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.81s
statement_cancellation_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 29.09s
storage_test
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
string_index_runtime_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.12s
string_index_sql_test
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.53s
string_secondary_index_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
structural_property_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
subquery_alias_binding_test
test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
subquery_test
test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.64s
subset_insert_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
sync_loader_test
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

第 15 批，退出码 0。`cargo test --test table_alias_resolution_test --test table_manager_test --test table_statistics_test --test truncate_test --test tx_statement_test --test typed_api_test --test typed_batch_write_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test -- --test-threads=1`

```text
table_alias_resolution_test
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
table_manager_test
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
table_statistics_test
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
truncate_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
tx_statement_test
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.13s
typed_api_test
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
typed_batch_write_test
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
update_expression_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
update_index_maintenance_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
upsert_test
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
value_test
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
version_chain_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

第 16 批，退出码 0。`cargo test --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_integration_test --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1`

```text
view_test
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.34s
visibility_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s
wal_auto_checkpoint_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.97s
wal_buffer_test
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
wal_handle_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.07s
wal_integration_test
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
wal_record_test
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
wal_recovery_large_test
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.25s
wal_writer_test
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
write_failure_state_test
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
write_path_slot_bounds_test
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
write_type_conformance_test
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
```

```text
git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml
```

这条命令无输出，退出码 0。跑完后再次 `ls tests/*.rs | sort`，与开跑前的清单一致。HEAD 仍是 `a9256c3a4fc24d1b0b473d40ade32b30fe30c8a7`。写入本记录之前工作区是干净的。

Persisted Evidence：None required

Experience Candidates：分批读取摘要时要匹配 `test result: `，冒号后面有一个空格。`^test result:` 会命中模块名 `result` 的单元测试行。本轮第 1 批因此多启动了一次，两次都是退出码 0 且 failed 为 0。已有的分批全量操作说明可以补上这条边界。本轮不改那份说明。没有范围外的实质缺陷。

未解决问题：无。

### 跳过

- `--ignored` 和上一轮计时：SKIPPED: 本轮命令不带 `--ignored`，不重跑恢复计时或锁等待。`cli_test` 摘要里有 2 个 ignored。
- 第二遍全量：SKIPPED: 16 批都已有退出码 0 且 failed 为 0。第 1 批多出的那一次只覆盖同一批。
- 产品代码修复：SKIPPED: 没有一批在复核后仍然失败，没有要求修改缓冲池、恢复或文件锁。
- `openspec instructions apply`：SKIPPED: 该命令称 `tasks.md` 没有它认得的任务条目。执行依据是本 Cycle 的 Task Definition，不重建 `tasks.md`。

## Plan Review

- Review Result: accepted

**Findings**

16 批的通过依据都在本文件 Act Response 里。每一批退出码为 0，同一次输出的每一行 `test result:` 都含 `0 failed`。当前 `ls tests/*.rs | sort` 与记录中的文件批逐项相同：169 个文件，字典序，missing、extra、duplicate 都是 0。切分是 11 批各 11 个文件、4 批各 12 个文件，第 1 批单独为 `cargo test --lib --bins -- --test-threads=1`。16 条命令都带 `--test-threads=1`，没有 `--ignored`，也没有无过滤的 `cargo test`。`src/`、`tests/`、`Cargo.toml` 相对 `0019bf127df5304f26e89927e336150fc6472b11` 没有差异。

第 1 批因 `^test result:` 命中 `result` 模块的单元测试名，多启动了一次。Act Response 写明两次退出码都是 0，failed 都是 0。通过依据是第一次，摘要为 460 passed（16.91s）和 0 passed（0.00s）。这是 Act 偏离「正常路径每批启动一次」。额外的一次只覆盖第 1 批，全量预算仍按这一次计。它不形成 Acceptance 缺口，也不留下要改的产品代码。编译作业数设为 2、`CARGO_TERM_COLOR=never`，以及清掉注入的 `CARGO_TARGET_DIR`，都不改变 `--test-threads=1` 和批间串行。

偏差分类是 Act 偏离，属于非阻塞 Minor。Plan 遗漏、Plan 错误、现状变化和新证据都没有出现。

范围外没有实质缺陷候选。`tests/crypto_off_build_test.rs` 整文件在 `#![cfg(not(feature = "crypto"))]` 下，默认特性不编译其中用例，摘要是 0 passed、0 failed。`tests/network_server_test.rs` 的用例已整段注释，注释指向 `tests/e2e_test.rs` 的 PostgreSQL 协议集成测试，摘要同样是 0 passed、0 failed。验收只看 failed 为 0。`cli_test` 的 2 ignored 留在普通套件之外，与计划一致。

**Acceptance Gaps**

无。

**Evidence**

测试结果沿用本文件 Act Response 的「验证」，本次不重跑 `cargo test`。该节有 16 批、171 行 `test result:`：库和二进制两行，169 个文件各一行，全部为 `ok` 且含 `0 failed`。全量预算已经用掉。覆盖范围没有失效：测试发生在 `a9256c3a4fc24d1b0b473d40ade32b30fe30c8a7`，其后的 `c7e566611656cedd4b72a7386d02723e623536f4` 只改了本 Cycle 和 `tasks.md`。

本次只读核对了 merge-base `0019bf127df5304f26e89927e336150fc6472b11` 到 HEAD `c7e566611656cedd4b72a7386d02723e623536f4` 的 diff，共 7 个文件，都在 `openspec/changes/2026-11-17-ms52-torn-tree-and-cross-process/` 下。`git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml` 无输出，退出码 0。工作区干净。当前 `ls tests/*.rs | sort` 为 169 行，与记录的文件批顺序一致。`B = (169 + 11) / 12 = 15`，11 个文件的批数是 11，12 个文件的批数是 4。Persisted Evidence 为 none，没有证据目录。

**Follow-up Decision**

验收已经满足。当前 Cycle 没有要修的缺口，也不需要新的执行依据。Iteration Plan 保持不变。这是最后一轮，不展开下一轮。全量已经发生，本结论不要求再跑。摘要误匹配已记在 Act Response 的经验候选里，本审查不改运行说明。

**Next Cycle**

None

**Next Iteration**

None
