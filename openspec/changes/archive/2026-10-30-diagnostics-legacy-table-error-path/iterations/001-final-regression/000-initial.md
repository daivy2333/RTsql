# Iteration 001 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 需求「收尾全量回归」与 M19 的分批执行约束。本 change 已完成的产品改动在本轮只被回归、不被修改：Iteration 000 的旧表输出时机收敛（`src/pipeline.rs` 的 `execute_inner` / `execute_staged` 打印点、`tests/diagnostics_statement_test.rs` 的两个新用例与「含有解析阶段」补断言）。Iteration 000 的 Plan Review 结论为 `accepted`。本 change 的 delta 规格（`specs/unified-diagnostics/spec.md`）已就位且未再改动，本轮不改写。
- Excluded scope: 任何产品代码、测试代码、夹具、`Cargo.toml` / `Cargo.lock`、criterion 门槛的修改；范围外缺陷的修复（ISS42 保持 open，待用户裁定，不在本 change 承载）；新的行为需求；把全量失败就地修掉（本轮不改代码，失败即停止并回报）；修改 `openspec/specs/` 权威正文（由 delta 在收尾时合并）。

**Objective**

在本 change 的默认构建上，按 M19 的分批规程把全部测试目标跑一遍（这是整个 change 唯一的一次全量）：枚举得到的集成测试目标与 `--lib` / `--bins` 全覆盖，每一批退出码 0 且 failed 计数为 0，`--test` 名字集合与枚举集合无缺失、无多余、无重复，且批间串行、每批单线程。

**Background**

本 change 的第一轮交付了旧三段计时表的输出时机收敛与解析阶段断言补强（`iterations/000-initial/000-initial.md`，Review Result `accepted`）。该轮只跑了受影响范围的定向验证，从未跑过全量——按 M19，全量预算在整个 change 一次，并放在收尾轮（`tasks.md` 的 Iteration Plan「Iteration 001：收尾全量」，与 MS44 的收尾轮同形：把行为失败与整库回归分在不同排障范围里）。本轮是本 change 的最后一轮：只执行验证，不再产生新的实现。

**Investigation Facts**

- Starting point: 产品代码树 = `9401b2d`（「act: 收敛旧三段计时表的输出时机并锁定解析阶段断言（RED→GREEN，9/9）」）；本轮 Plan Review 只写 change 文档、不动产品代码，因此 T3 开始时的产品代码就是 `9401b2d` 的树。分支 `changes/2026-10-30-diagnostics-legacy-table-error-path`（基点 master，merge-base `6412caf`），change 自创建起只在该分支上演进。
  - 前一轮的最终改动：`src/pipeline.rs`——`execute_inner` 移除「`execute_staged` 返回后无条件 `print_timings`」，把既有的 `total_start` 传入 `execute_staged`；`execute_staged` 新增 `total_start: Option<Instant>` 参数，并在两处打印旧表——缓存命中路径在 `execute_stage` 返回后（1538-1541）、未命中路径在 execute 阶段之后（1659-1663）。执行阶段之前的六个早退（解析失败、多语句拒绝、无语句、占位符编号失败、参数个数不匹配、规划失败）不经过打印点。`tests/diagnostics_statement_test.rs`——新增 `legacy_table_absent_before_execute_stage`（473）与 `legacy_table_present_on_execute_stage_failure`（549），并给 `parse_error_keeps_error_text_and_summary_fails` 补「含有解析阶段」断言（217）。
  - 前一轮的验证结论（沿用，来源为 `iterations/000-initial/000-initial.md` 的 Act Response；被覆盖代码此后未再变化，无公共规则 › 验证 列出的重跑触发条件）：RED `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` → 8 passed / 1 failed（失败点为「stderr 不含 `Time (µs)`」）；GREEN（T1 后）同一命令 → 9 passed / 0 failed；GREEN（T2 断言后）同一命令 → 9 passed / 0 failed；真实 CLI 三场景核对（成功导入 2 张旧表 / 规划期失败 0 张旧表 / 执行期失败恰好 1 张旧表）与基点 `e69c7e2` 的记录逐行一致；`openspec validate 2026-10-30-diagnostics-legacy-table-error-path --strict` → `Change '...' is valid`。
  - 变更前全量基线（沿用，来源为已归档 MS44 change 的收尾轮 Act Response，`openspec/changes/archive/2026-10-28-ms44-unified-diagnostics/iterations/003-final-regression/000-initial.md`）：15 批全部 exit 0、164 个测试二进制、2407 passed / 0 failed、目标集合对账成立。该基线取自 MS44 分支的产品树，不含本 change 的改动，也不是本轮的通过依据——本轮全量结论只来自本轮实跑。
- Current-State Evidence:
  - 集成测试目标的唯一来源是 `tests/` 目录下的 `*.rs` 文件：`ls tests/*.rs | sort` 在 HEAD `9401b2d` 上给出 162 个文件（`diagnostics_statement_test` 在列；本 change 未新增测试文件，只修改既有文件）。本轮按 M19 在执行时现场枚举，不把 162 这个数写进通过条件。
  - 目标名映射：文件名去掉 `.rs` 即 `--test` 的名字；`--test <名>` 与文件名一一对应（`cargo` 由文件名推导目标名，不存在手写别名）。
  - 构建配置：`Cargo.toml` 的 `default = ["cli", "network", "crypto"]`。162 个测试文件中有 116 个带 `#![cfg(...)]` 门（70 个 `network`、33 个 `network` + `cli`、12 个 `cli`、1 个 `not(crypto)`）。默认特性下前 115 个照常编译运行；`tests/crypto_off_build_test.rs` 是 `#![cfg(not(feature = "crypto"))]`，在默认构建下该目标编译为 0 个用例（退出码 0、0 passed / 0 failed）——它仍必须出现在枚举与批次里，不得因「0 个用例」判为缺失。`tests/network_server_test.rs` 的用例整体位于注释块中（M8 起暂停 JsonProtocol 场景测试），同样编译为 0 个用例。
  - `benches/`（criterion 基准）与 `examples/` 不在 M19 规程选择的 `--lib` / `--bins` / `--test` 目标内，因此 criterion 门槛本轮不被执行，也不构成任何比较依据。
  - 离线可用性：`--offline` 所需的全部 dev-dependency（`criterion 0.5.1`、`rusqlite 0.31.0` + `libsqlite3-sys`、`tempfile`、`which`、`libc`、`futures`）在本机 cargo registry 中已有源码与 `.crate` 缓存；worktree 的 `target/` 目录已由前几轮构建产物填充，逐批执行复用增量产物。
  - 既有基线噪声（非本 change 引入）：`cargo fmt --check` 报两处既有漂移（`tests/diagnostics_facade_test.rs:216` 与 `tests/session_lifecycle_test.rs:15`，两处都不在本 change 的 diff 内）。它们不属于本轮判定面，也不影响测试退出码。
  - 本轮不改代码 ⇒ 没有先行测试需要建立，也没有新的验证命令之外的行为断言需要写。
- Code and Critical Path: 本轮不改任何产品代码与测试代码，不新增文件。执行对象是 change 分支上 `9401b2d` 的默认构建产物：`src/lib.rs` 的 lib 单测目标、`[[bin]]` 的 bin 单测目标、以及 `tests/*.rs` 的全部集成测试目标。行为承载面由前一轮交付的代码决定（旧表输出时机、诊断摘要、`[recovery]` 告警原文），本轮只观察它们在整套测试下是否仍然成立。

**Implementation Guidance**

- 本轮唯一的动作是执行 M19 的分批规程并如实记录。规程全文在下面 T3 的 Required behavior 与 Verification 里，Act 按该规程逐步执行，不自行增删命令、批次或过滤条件。
- 记录格式：每批在 Act Response 的「验证命令与实际结果」中写一行——批号、该批的完整命令、退出码、passed 计数、failed 计数；批次的划分（哪些名字在哪一批）一并写出，便于收尾核对覆盖。
- 不需要脚本来编排批次：M19 要求的「枚举 → 逐批一条命令 → 批间串行」直接由执行者在 shell 中逐条执行即可；不得为此新增脚本、封装命令或判定层。

**Task Definitions**

### T3: 一次分批串行的全量回归

- Requirement/Scenario: proposal 需求 1–4 的行为面在本轮以整套测试的形式被回归一次；`tasks.md` 的 T3 与 M19「全量回归验证 SHALL 分批执行」。本轮不产生新的行为要求，场景是「在默认构建上跑完全部测试目标并逐批判定」。
- Depends on: T1、T2（Iteration 000 accepted，Plan Review 结论 `accepted`）。
- Targets: 无产品代码目标（本轮零代码改动）。执行对象：`src/lib.rs` 的 lib 单测目标、`[[bin]]` 的 bin 单测目标、`tests/*.rs` 的全部集成测试目标（执行时由 `ls tests/*.rs | sort` 现场枚举）。
- Current behavior: 本 change 从未跑过全量——Iteration 000 只跑了受影响范围的定向验证（见 Starting point 的沿用结论）。分支上带着 `src/pipeline.rs` 的打印点改动与 `tests/diagnostics_statement_test.rs` 的新增用例与补断言，尚未在整套测试下被观察。
- Required behavior: 按下面的 M19 分批规程执行一次全量，产出一份逐批的退出码与 passed / failed 计数，并完成目标集合对账。具体规程（Act 逐步执行，不得改动）：
  1. **枚举（执行时现场枚举）**：运行 `ls tests/*.rs | sort`，得到有序的集成测试文件列表；每个文件名去掉 `.rs` 得到一个 `--test` 名字，构成集合 `E`。不得把规划时的文件数量写死，也不得凭记忆列举。
  2. **第一批单独执行**：`cargo test --offline --lib --bins -- --test-threads=1`。`--lib` 与 `--bins` 只出现在这一批。
  3. **其余分批**：把 `E` 中的名字按步骤 1 的排序，每 12 个一批（最后一批可以少于 12 个）。每批一条命令，名字在批内保持步骤 1 的字典序：`cargo test --offline --test <名1> --test <名2> ... --test <名12> -- --test-threads=1`。
  4. **批间串行**：一条命令结束后再开始下一条；同一时刻只有一批在跑；批次之间不并发、不叠加。
  5. **禁止项**：不得运行不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`（无过滤全量，任何并行度都不行）；不得让任何全量批次使用默认并行度——每批都必须带 `-- --test-threads=1`。
  6. **对账**：全部批次用到的 `--test` 名字并集必须等于 `E`：无缺失、无多余、无重复；`--lib` / `--bins` 只出现在第一批。对账不成立即为失败，不补跑已通过的批。
  7. **逐批记录**：每批把命令、退出码、passed 计数、failed 计数写入 Act Response。退出码 0 且 failed 计数为 0 才算该批通过。任一批未执行，不得声明全量通过（全量 Gate 等价于「全部批次各自 0 failed」）。
  8. **失败处置**：某一批失败时，只把该批按同一命令、同一并行度再跑一次；第二次仍失败则本任务失败——不改并行度，不重跑已通过的批，不在本轮改代码。复核通过按通过处理，并在验证记录里注明首跑失败。
- Preserve: 产品代码、测试代码、夹具、`Cargo.toml` / `Cargo.lock`、criterion 门槛一律不改；Iteration 000 已 accepted 的行为面（旧表的输出时机与表内容、诊断摘要的阶段与严重级别、错误文本与 SQLSTATE、`[recovery]` 告警原文）不改；本轮是本 change 唯一的一次全量——不得因为某批失败而另起一次全量；`openspec/specs/` 权威正文与本 change 的 delta 不在本轮改写。
- Forbidden: 无过滤的 `cargo test`；默认并行度跑任何全量批；修改任何产品代码或测试代码（含注释与格式）；新增测试、fixture、脚本、封装命令或判定层；为判定「同一轮运行」引入 Hash / 指纹 / revision pin / run-id / 序号 / manifest / 时间顺序证明；跳过失败批或删减批次来凑通过；重跑已通过的批；把 criterion 基准当通过条件；把「之前跑过」当作本轮的实跑结论；修改 `openspec/specs/`。
- First test: 无先行测试——本轮不改代码，没有行为变更需要先行测试（公共规则 › TDD 的例外：本轮只有验证，没有实现变更）。首次执行即第一批：`cargo test --offline --lib --bins -- --test-threads=1`。
- GREEN condition: 全部批次的退出码均为 0 且 failed 计数均为 0，且步骤 6 的对账成立（`--test` 名字并集 == `E`，无缺失、无多余、无重复；`--lib` / `--bins` 只在第一批），且 Act Response 写齐了每一批的命令、退出码与 passed / failed 计数。
- Verification: 逐批的直接判定——每批命令的退出码与输出中的 failed 计数（退出码 0 且 failed 为 0 即该批通过）；全量结论等价于全部批次各自 0 failed 且对账成立。不引入人工核对表、脚本或任何判定层；不用 hash / revision / 时间戳等身份类字段作为通过条件。
- Stop when: 任一批第二次复核仍失败 → 停止，不改代码、不改并行度、不重跑已通过的批，写 Blocker Handoff 并把 Act Response 状态置为 `blocked`（写明失败批次、失败用例与实际输出），等待用户决定；枚举或对账不成立（缺失 / 多余 / 重复 / `--lib`、`--bins` 出现在非第一批）→ 停止并回报，不补跑；执行环境无法满足 M19 的前提（离线依赖不可用、机器承载不足）→ 停止并回报，不降级为无过滤全量或默认并行度。

**Invariants**

- M19 的分批约束在本轮全程成立：无过滤全量、默认并行度全量、跳过批次、重复跑已通过批次都不允许。
- 全量只跑一次；失败批的唯一复核机会用尽后即停止，不在本轮修代码。
- 验证结论只来自实际运行的退出码与 failed 计数；不引用「上次跑过」代替本轮实跑，也不把 revision / 时间戳 / 身份类信息当通过条件。
- Iteration 000 已 accepted 的行为面在本轮不改、不重新裁定。

**Non-goals**

- 不在本轮修任何失败（含本 change 引入的失败）——失败即停止并回报，由后续流程决定承载方式。
- 不改产品代码、测试代码或构建配置；不新增测试与基准；不改权威规格正文。
- 不承载 ISS42 的修复（范围外既有行为，待用户裁定）。
- 不做性能比较、不设数值开销门槛。

**Acceptance**

- 全部批次的退出码为 0 且 failed 计数为 0（`tasks.md` 的 T3；M19「全量 Gate 等价于全部批次各自 0 failed，缺一批不得声明全量通过」）。
- `--test` 名字集合等于 `ls tests/*.rs | sort` 去掉 `.rs` 后的集合：无缺失、无多余、无重复；`--lib` / `--bins` 只出现在第一批（对账可核对）。
- 批间串行、每批 `-- --test-threads=1`、全程无无过滤 `cargo test`（M19 分批执行约束）。
- Act Response 写齐每批的命令、退出码、passed / failed 计数，并写明全量结论所依据的分批方式（M19「验证记录中的全量结论 SHALL 注明分批执行方式并保留各批的 passed/failed 计数」）。
- 本轮的产品代码零改动：`git diff` 中本轮只有 change 文档。
- Requirement Traceability：`tasks.md` RTM 的「收尾全量（M19 分批预算）」→ T3（分批命令序列与对账），Covered。

**Verification**

- 逐批的直接判定（自动命令）：每批一次 `cargo test --offline ... -- --test-threads=1`，以退出码与输出中的 failed 计数判定；全量结论 = 全部批次各自 0 failed + 目标集合对账成立。
- 无需人工验证步骤；无端到端类人工场景（目标集合与批内用例数由命令输出直接给出）。
- 沿用 Iteration 000 的定向结论：本轮不重复执行 `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`（9 passed / 0 failed 已取得且覆盖范围未变化）；它同时是本轮全量批次的一部分（B5 或其所在批次），会在全量中被再次覆盖。
- 本轮是本 change 的唯一一次全量——不再另起全量。

**Gate 2 Readiness**

PASS。检查项均通过，无 BLOCKED，无 SKIPPED。调查基于本会话对 `Cargo.toml`（默认特性、dev-dependency）、`tests/` 目录（162 个目标、116 个 `#![cfg(...)]` 门、两个 0 用例目标）、`benches/` 与 `examples/`（不在 M19 选择集内）、`src/pipeline.rs` 与 `tests/diagnostics_statement_test.rs` 的实际阅读，以及 change 产物（`tasks.md` / `design.md` / `specs/unified-diagnostics/spec.md` / 前一轮 Cycle 文件）的核对；执行依据自包含（M19 规程全文在 Task Definition 的 Required behavior 与 Verification 里，Act 不需要回读本 change 的其他产物）；验证边界明确（一次全量，逐批退出码与 failed 计数 + 目标集合对账）；RTM 无 Missing、无 Simplified；Persisted Evidence 为 none；批准来自本次 goal 注入的授权（覆盖计划批准，检查标准不降低）。

**Persisted Evidence**

- Mode: none

命令、实际结果、逐批退出码与 passed / failed 计数、对账结论写入 Act Response 即可；失败批的整批复核记录也写在 Act Response。

**Risks and Notes**

- 逐批执行会产生多次编译；worktree 的 `target/` 已有前几轮的产物可复用，但批次之间不共享编译进程（每批一条独立命令）。
- 机器承载：M19 的分批规程正是为「无过滤全量一次性构建链接全部测试二进制并长时间独占机器」而立；不得为了省时间把多批合并成一条命令或改用默认并行度。
- 全量失败不等于「分批规则错了」——不把全量失败当成修改分批规则的理由（`tasks.md` 的 Iteration 001 Diagnostic boundary）。
- `tests/crypto_off_build_test.rs` 与 `tests/network_server_test.rs` 在默认构建下编译为 0 个用例（退出码 0、0 passed / 0 failed），仍必须出现在枚举与批次里，不判为缺失目标。
- 偶发失败类：MS44 收尾轮记录过 `tests/diagnostics_wait_test.rs` 的 `commit_wait_classes_present` 断言（`Queue > 0`）依赖唤醒延迟、可能偶发失败。按规程第 8 条，偶发失败只把该批再跑一次；第二次仍失败则本任务失败（`blocked`），不得改并行度、不得改断言、不得重跑其他已通过的批。
- 两处既有 rustfmt 漂移与 Iteration 000 的四条 Minor finding 不在本轮处理面。
- 本轮不做任何超出收尾验证的清理。

## Act Response

- Status: reported

### 实际改动

- 产品代码零改动、测试代码零改动、`Cargo.toml` / `Cargo.lock` 零改动、未新增任何测试、fixture、脚本、封装命令或判定层。
- 本轮唯一写入是 change 文档：本文件（Act Response）与 `tasks.md` 的 T3 状态行。
- `git status --porcelain` 在提交前为空，`git diff --stat HEAD` 为空——本轮开始与结束时工作区都只有 change 文档改动，产品树停在 `9401b2d`（本 Cycle 开始时的 HEAD `3391898` 的父提交内容）。

### 逐批清单

枚举（执行时现场执行）：`ls tests/*.rs | sort` → 162 个文件；去掉 `.rs` 得 162 个 `--test` 名字，集合 `E`。重名检查：`sort | uniq -d` 输出 0 行（无重名）。

全部 15 批串行执行，同一时刻只有一批在跑（一条命令结束后才发下一条）。每批命令形如 `cargo test --offline --test <名>... -- --test-threads=1`；B1 形如 `cargo test --offline --lib --bins -- --test-threads=1`。全程没有不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`，全部批次都带 `-- --test-threads=1`。

| 批 | 覆盖的测试目标 | 退出码 | passed | failed | ignored |
|---|---|---|---|---|---|
| B1 | `--lib`（`src/lib.rs` 单测）+ `--bins`（`src/main.rs` 单测）；无 `--test` 目标 | 0 | 443（lib）+ 0（bins） | 0 | 0 |
| B2 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test, catalog_bounded_read_test | 0 | 154 | 0 | 0 |
| B3 | catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_test, composite_index_lifecycle_test, composite_index_runtime_test, concurrent_test, connection_limit_test | 0 | 199 | 0 | 2 |
| B4 | constraint_enforcement_test, cross_db_dml_test, cross_db_query_test, crypto_off_build_test, ctas_test, cte_test, data_page_bounded_read_test, database_file_lock_test, datetime_function_test, datetime_type_test, ddl_catalog_transactional_test, ddl_crash_recovery_test | 0 | 155 | 0 | 0 |
| B5 | ddl_layout_publication_test, ddl_rewrite_consistency_test, default_declaration_test, default_precheck_test, derived_scan_alias_test, diagnostics_facade_test, diagnostics_statement_test, diagnostics_wait_test, dml_tombstone_abandon_window_test, dml_tx_id_test, drop_persistence_test, drop_table_free_test | 0 | 116 | 0 | 0 |
| B6 | e2e_test, encryption_test, executor_test, explain_test, explicit_tx_test, expression_e2e_test, file_header_test, file_storage_io_test, free_page_lifecycle_test, full_table_dml_test, gc_test, group_by_expr_test | 0 | 184 | 0 | 0 |
| B7 | hash_join_build_side_test, import_input_surface_test, index_key_encoding_evaluation_test, index_manager_test, index_page_ownership_stack_overflow_test, index_page_ownership_test, index_range_routing_test, index_read_bounded_read_test, index_write_atomicity_test, insert_column_list_test, insert_select_test, isolation_level_test | 0 | 76 | 0 | 0 |
| B8 | join_output_resolution_test, join_test, key_type_conformance_test, keyless_eq_routing_test, keyless_row_test, legacy_compat_test, limit_test, memory_db_test, mvcc_abort_test, mvcc_commit_test, mvcc_record_test, mvcc_tombstone_visibility_test | 0 | 113 | 0 | 0 |
| B9 | nested_loop_join_test, network_protocol_test, network_server_test, no_from_select_test, order_by_alias_sort_test, outer_join_test, overflow_page_test, param_binding_test, parser_test, pg_integration_test, pg_messages_test, pg_protocol_test | 0 | 133 | 0 | 0 |
| B10 | pipeline_test, plan_cache_test, plan_exec_test, planner_test, predicate_test, prefetch_test, prepared_statement_test, projection_expression_test, projection_test, pushdown_test, query_memory_test, readonly_session_test | 0 | 240 | 0 | 0 |
| B11 | recovery_bounded_read_test, recovery_e2e_test, recovery_error_surface_test, recovery_replay_convergence_test, recovery_tail_persistence_test, recovery_test, recovery_wal_convergence_test, release_durability_precondition_test, release_path_bounded_read_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test | 0 | 92 | 0 | 0 |
| B12 | schema_persistence_test, secondary_index_equality_test, secondary_index_test, select_distinct_test, self_join_test, session_lifecycle_test, set_operations_test, sort_merge_join_test, sort_test, sql_differential_test, statement_cancellation_test, storage_test | 0 | 147 | 0 | 0 |
| B13 | string_index_runtime_test, string_index_sql_test, string_secondary_index_test, structural_property_test, subquery_alias_binding_test, subquery_test, subset_insert_test, sync_loader_test, table_alias_resolution_test, table_manager_test, truncate_test, tx_statement_test | 0 | 163 | 0 | 0 |
| B14 | typed_api_test, update_expression_test, update_index_maintenance_test, upsert_test, value_test, version_chain_test, view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test | 0 | 142 | 0 | 0 |
| B15 | wal_record_test, wal_recovery_large_test, wal_writer_test, write_failure_state_test, write_path_slot_bounds_test, write_type_conformance_test（末批 6 个） | 0 | 52 | 0 | 0 |

- 批次划分：162 = 13 批 × 12 + 末批 6 个；B2–B14 各 12 个，B15 为 6 个。总批数 15（含 B1 的 `--lib`/`--bins`）。
- 合计：结果行共 164 行（lib 1 + bins 1 + 集成 162 个目标，各目标一行 `test result:`），passed 合计 2409（lib 443 + bins 0 + 集成 1966），failed 合计 0，ignored 2（B3 的 `cli_test`）。
- 没有任何一批失败，因此没有触发规程第 8 条的整批复核；本轮没有重复运行任何批次。

### 对账结论

- 全部批次用到的 `--test` 名字并集与 `E` 逐元素相等：`diff` 输出为空，`used_total=162`、`enum_total=162`。
- 无缺失、无多余、无重复（对实际使用的 162 个名字做 `sort | uniq -d`，输出 0 行）。
- `--lib` 与 `--bins` 只出现在 B1；B2–B15 全部是纯 `--test` 过滤。
- 规程第 6 步对账成立。

### 验证命令与实际结果

- 全量结论所依据的分批执行方式：M19 分批串行——现场枚举 `ls tests/*.rs | sort` 得到 162 个目标；第一批单独跑 `--lib --bins`；其余按该排序每 12 个一批（B2–B14），末批 6 个（B15）；每批一条 `cargo test --offline ... -- --test-threads=1`；批间串行，同一时刻只有一批在跑；全程无无过滤 `cargo test`，无默认并行度批次。
- 判定依据：每批的退出码与输出中的 `failed` 计数。15 批退出码全为 0，`failed` 全为 0。
- 观察到的相关细节（均为实际输出）：`crypto_off_build_test`（B4）与 `network_server_test`（B9）在默认构建下为 0 用例，退出码 0，不判为缺失目标；B5 的 `diagnostics_statement_test` 9 passed / 0 failed，与 Iteration 000 的定向结论一致；B5 的 `diagnostics_wait_test` 6 passed / 0 failed，MS44 收尾轮记录过的 `commit_wait_classes_present` 偶发断言本次未复现，无需启用规程第 8 条的复核。
- 沿用结论：Iteration 000 的定向验证（`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` → 9 passed / 0 failed）本轮不重复执行——该目标已作为 B5 的一部分被整套覆盖，且被覆盖代码此后未变化。

### 与计划的偏差

无。批次划分、命令形式、并行度、判定方式与 Task Definition 的 M19 规程完全一致。唯一实现性说明：读取每批输出时用 `awk` 单步管道提取 `test result:` 行并汇总计数（通用工具单步管道，读取既有输出），不是判定层——判定仍是退出码与 `failed` 计数；未新增任何脚本、fixture 或封装命令。

### 自检发现

- 未发现 Critical 或 Important 问题；无遗留 Minor 问题。
- 本轮不改代码，因此 Gate 3 的「先行测试」项不适用（公共规则 › TDD 的例外：只有验证，无实现变更）；Gate 3 的其余检查（无计划外修改、无新增警告、无身份证明机制、无判定层、无产物外改动）逐项通过——`git status --porcelain` 在写入 change 文档前为空，`git diff --stat HEAD` 无产品代码条目。
- Gate 4：Task Definition 的 Verification 全部满足（逐批退出码 0 + failed 0 + 对账成立 + Act Response 写齐每批计数）。

### Persisted Evidence

None required——命令、实际结果、逐批退出码与 passed / failed 计数、对账结论均已在本 Response 内，无失败批需要保留整批复核现场。

### Experience Candidates

None——本轮只有一次成功的全量回归，没有新的可复用操作路径（既有 Runbook 覆盖构建与测试命令），也没有发现本 change 范围外的实质缺陷。

### 未解决问题

- 无阻塞项，无 Blocker Handoff。
- ISS42 保持 open（`record_time` 在旧 scope 外 panic 的既有行为），按计划不在本 change 承载，本轮未触碰。
- 计划中记为既有噪声的 `cargo fmt --check` 两处漂移（`tests/diagnostics_facade_test.rs:216`、`tests/session_lifecycle_test.rs:15`）不在本轮判定面，本轮未处理。

## Plan Review

- Review Result: accepted

**Findings**

本 Review 独立核对了仓库现场、实际代码、完整 diff 与 change 产物；Act 已报告且覆盖范围未失效的验证结论直接沿用（来源：本 Cycle 的 Act Response），**未重跑任何批次**——M19 规定本 change 的全量预算只有一次，已由 Act 用掉，重跑既违反预算也违反公共规则 › 验证「禁止自行重复运行已通过的验证」。本轮能由静态检查独立复现的部分已全部复现（覆盖集合、对账、每批计数、总计），不能静态复现的部分（命令字面形式、批间时序）记录为 Minor A。

1. **本轮零产品代码改动——Act 声明与仓库现场一致（偏差归类：无）**

   `git diff --name-only 9401b2d 1a00906` 只输出三个 change 文档：`iterations/000-initial/000-initial.md`（上一轮的 Plan Review）、`iterations/001-final-regression/000-initial.md`（本 Cycle 的 Plan Context 与 Act Response）、`tasks.md`。产品树与测试树逐字节未变——`git diff --stat 6412caf HEAD` 显示自 merge-base 起只有 `src/pipeline.rs`（+17/−4）与 `tests/diagnostics_statement_test.rs`（+154/−0）两个代码文件，全部来自 Iteration 000 的 `9401b2d`。`git status --porcelain` 为空，工作区 clean。Act Response「实际改动」的四条声明（产品代码零改动、测试代码零改动、`Cargo.toml`/`Cargo.lock` 零改动、未新增测试/fixture/脚本/封装命令/判定层）与现场一致。

2. **逐批记录自洽——可由源码静态复现（偏差归类：无）**

   把 Act Response 表格里的 162 个 `--test` 名字抽出，与现场枚举集合逐元素比对：

   | 核对项 | 复核结果 |
   |---|---|
   | 枚举规模 | `ls tests/*.rs \| sort` → 162 个文件；去 `.rs` 后 162 个名字，`uniq -d` 输出 0 行 |
   | 批数与批大小 | 15 批：B1 = `--lib`/`--bins`，B2–B14 各 12 个，B15 末批 6 个；13×12 + 6 = 162，与枚举数相等 |
   | 对账 | 并集 = 枚举集合：无缺失、无多余、无重复（162 个用到的名字唯一且全部命中枚举集合） |
   | 分组顺序 | 批次内的名字序列与 `sort` 后的枚举顺序**完全一致**（逐元素相等），符合 M19「分组顺序固定（如目标名字典序）」 |
   | `--lib`/`--bins` 位置 | 只出现在 B1；B2–B15 的目标字段全部是 `--test` 名字 |
   | 每批 passed | 13 个集成批逐一等于该批文件的 `#[test]`/`#[tokio::test]` 属性计数，唯一两处差异是 Plan Context 已声明的两个 0 用例目标（下两行）；lib 443 = `src/` 内 443 个测试属性，bins 0 = `src/main.rs` 0 个测试属性 |
   | 两处已知差异 | B4：静态 158 − `crypto_off_build_test` 的 3（`#![cfg(not(feature = "crypto"))]`，默认组合含 `crypto` ⇒ 编译为 0 个用例）= 155，与报告一致；B9：静态 137 − `network_server_test` 的 4（整体位于 `/* */` 注释块内，M8 起暂停 JsonProtocol 场景测试 ⇒ 编译为 0 个用例）= 133，与报告一致 |
   | 总计 | lib 443 + bins 0 + 集成 1966 = **2409 passed**、**0 failed**、**2 ignored**（ignored 全部落在 B3 的 `cli_test`），与报告的 2409 / 0 / 2 逐位相符；结果行 164 行（lib 1 + bins 1 + 集成 162）自洽 |
   | 失败处置 | 报告中无失败批 ⇒ 未触发规程第 8 条的整批复核，未重跑任何批次，与 M19「缺一批不得声明全量通过」不冲突 |

   结论：批数、每批覆盖的目标、退出码、passed/failed/ignored 计数、总计与对账结论五者互相自洽，且可由源码静态复现；报告中的 `2409 passed / 0 failed` 与「枚举集合 = 执行集合、`--lib`/`--bins` 只在第一批」成立。

3. **M19 约束逐条成立（偏差归类：无）**

   依据 `.agents/memory/project-model.md` 的 M19：分批执行 ✓（15 批）；每组一次独立调用、批间顺序执行、同一时刻只有一批 ✓（Act 声明）；SHALL NOT 用一次无目标过滤的全量调用 ✓（B1 带 `--lib`/`--bins`，B2–B15 带 `--test`，无无过滤调用）；SHALL NOT 用默认并行度跑任何全量批次 ✓（每批 `-- --test-threads=1`，Act 声明）；分组顺序固定 ✓（静态复现）；批大小按机器能力在 8–12 ✓（12，末批 6）；验证记录注明分批方式并保留各批 passed/failed 计数 ✓（Act Response 的「逐批清单」表与「验证命令与实际结果」节）；全量 Gate 等价于「全部批次各自 0 failed」✓（15 批全 0 failed）。

4. **Acceptance 逐条满足**

   | # | Acceptance | 结论 | 依据 |
   |---|---|---|---|
   | 1 | 全部批次退出码 0 且 failed 计数为 0 | 满足 | 报告 15 批全 exit 0 / failed 0；每批 passed 计数已静态复现 |
   | 2 | `--test` 名字集合 = 枚举集合，无缺失/多余/重复，`--lib`/`--bins` 只在第一批 | 满足 | 静态复现：并集 = 枚举集合（162 == 162），无重复，`--lib`/`--bins` 仅 B1 |
   | 3 | 批间串行、每批 `-- --test-threads=1`、全程无无过滤 `cargo test` | 满足（Act 声明，命令字面形式不可二次取证，见 Minor A） | Act Response「逐批清单」与「验证命令与实际结果」 |
   | 4 | Act Response 写齐每批命令、退出码、passed/failed 计数并注明分批方式 | 满足 | 「逐批清单」表 15 行齐全（批号、目标、退出码、passed、failed、ignored）+「验证命令与实际结果」节写明分批执行方式 |
   | 5 | 本轮产品代码零改动 | 满足 | `git diff --name-only 9401b2d 1a00906` 只有三个 change 文档 |
   | 6 | RTM「收尾全量（M19 分批预算）」→ T3 Covered | 满足 | `tasks.md` RTM 行与 T3 的对账通过依据一致 |

5. **Iteration 000 推迟到 T3 的验收面在本格闭合（偏差归类：无）**

   Iteration 000 的 Acceptance 6（「`[recovery]` 等既有 stderr 告警原文不变」）在当时的 Plan Context 里明确推迟到 T3 复核，判定面是 `tests/recovery_bounded_read_test.rs` 所在批次。该目标落在 B11（exit 0、failed 0），故 Iteration 000 的推迟项现在闭合，本 change 不再有悬空的验收面。

6. **delta 规格与 Iteration 000 的实现一致（独立核对，偏差归类：无）**

   读 HEAD `9401b2d` 的 `src/pipeline.rs`：`print_timings` 全仓只有两个调用点（`src/pipeline.rs:1540` 缓存命中路径 `execute_stage` 返回后、`1662` 未命中路径 execute 阶段之后）；`grep -rn print_timings src/` 的其余命中是定义（`src/profiling.rs:33`）与 import（`src/pipeline.rs:21`）。六个执行阶段之前的早退（parse 失败、多语句拒绝、无语句、占位符编号失败、参数个数不匹配、规划失败）全部在两个打印点之前 `return Err`，路径上不存在打印点。逐条对照 delta：

   - Requirement 改写段落「SHALL 只在执行阶段被触达的返回路径输出这张表。执行阶段被触达指两种情形：计划缓存命中后直接执行，以及解析与规划都成功后进入执行」→ 实现侧正是 `1534-1542`（命中）与 `1651-1663`（未命中进入执行）两条，无第三条路径。
   - 「解析失败、规划失败、多语句拒绝、占位符编号失败、参数个数不匹配、无语句可执行等执行阶段之前的失败路径 SHALL NOT 输出这张表」→ 六条路径均无打印点，逐条成立。
   - 「进入执行阶段后，无论执行成功还是失败，SHALL 输出这张表」→ `1662` 在 `execute_stage` 返回之后打印，不区分 Ok/Err；命中路径 `1540` 同理。
   - 「该收敛 SHALL 只改变这张表的输出时机。语句返回的错误文本与 SQLSTATE SHALL 与收敛前相同」→ diff 未触及任何 `SqlError` 构造点（`git diff 6412caf 9401b2d -- src/pipeline.rs` 只含 `total_start` 传参与两处打印点）。
   - 场景「失败路径也输出旧的三段计时表」（GIVEN 已触达执行阶段）→ 对应 `1662`；场景「执行阶段之前的失败路径不输出旧的三段计时表」（GIVEN 尚未触达执行阶段）→ 对应六个早退；两个场景合起来覆盖失败路径的两半，描述准确。
   - 场景「旧开关仍输出三段耗时」限定「未命中计划缓存的语句」→ 与实现一致：未命中路径记 parse/plan/execute 三行；缓存命中路径不记 `execute` 行（与基点 `e69c7e2` 一致），场景文本未与实现冲突。
   - delta 保留权威规格该 Requirement 下的三个既有场景标题（OpenSpec MODIFIED 约束）并新增一个；`openspec validate 2026-10-30-diagnostics-legacy-table-error-path --strict` → `Change '2026-10-30-diagnostics-legacy-table-error-path' is valid`，退出码 0（本次实跑）。

   测试侧同样核对：`git diff 6412caf 9401b2d -- tests/diagnostics_statement_test.rs` 含两个新用例（`legacy_table_absent_before_execute_stage` 473、`legacy_table_present_on_execute_stage_failure` 549）与 `parse_error_keeps_error_text_and_summary_fails` 的 `stage_stat(&summaries[0], Module::Pipeline, "parse")` 补断言（217）；断言用的 `Time (µs)`、`duplicate column name`、`Duplicate key`、退出码 3 与 delta 的两个失败场景逐项对应，无判定层、无新增进程管理代码。

7. **design.md 与已实现行为一致（偏差归类：无）**

   D1（打印点回到「走到执行阶段的返回路径」，`total_start` 起点仍在缓存查找之前——`execute_inner` 建点在 `execute_staged` 调用之前，缓存查找在 `execute_staged` 内）✓；D2（失败路径判定走真实 CLI 子进程）✓；D3（delta 保留场景标题）✓；D4（「含有解析阶段」是锁定既有行为，产品零改动）✓；D5（验证预算：全量整个 change 一次放收尾轮）✓。`design.md` 的「规划时读到的现状」段落记录的是 `6412caf` 的旧行号，与 `9401b2d` 的实际行号有偏移，属历史现场记录，不构成与实现矛盾（决策正文与实现一致）。

8. **Minor findings（均不阻塞，不要求返工）**

   - Minor A（取证限制，非偏差）：每批命令的字面形式（`--offline`、`-- --test-threads=1`）与「批间串行、同一时刻只有一批」只能由 Act Response 承载——本 change 的 Persisted Evidence 为 `none`、无批次日志留存，而 M19 规定全量预算只有一次、不得重跑。数量、覆盖、对账、每批计数与总计已由源码静态复现，因此不构成 Acceptance 缺口。
   - Minor B（既有噪声，范围外）：`cargo fmt --check` 的两处既有漂移（`tests/diagnostics_facade_test.rs:216`、`tests/session_lifecycle_test.rs:15`）不在本 change 的 diff 内，本轮未触碰。
   - Minor C（范围外既有缺陷）：ISS42（`RTSQL_PROFILING` 对 CLI 主命令无效、`record_time` 在旧 scope 外 panic）保持 open，按用户裁定不由本 change 承载；本 change 未触碰 `src/profiling.rs`，其判定前提未变。

**Acceptance Gaps**

无阻塞缺口。Cycle 的 6 条 Acceptance 全部满足（逐条对照见 Findings 4），Iteration 000 推迟到 T3 的「既有告警原文不变」已在 B11 闭合（Findings 5）。三条 Minor 均不阻塞 Acceptance，不为其创建修复、不要求当前 Cycle 返工。

**Evidence**

- 仓库现场：`git status --porcelain`（空）、`git log --oneline -8`、`git diff --name-only 9401b2d 1a00906`（3 个 change 文档）、`git diff --stat 6412caf HEAD`（9 个文件，其中代码 2 个）、`git diff --name-only 6412caf a663360`（主仓库自基点起只动 `.agents/` 下三份文档，与本 change 无交集）。
- 实现核对：`grep -rn print_timings src/`（调用点仅 `src/pipeline.rs:1540`、`1662`）、`sed` 读 `src/pipeline.rs` 的 `execute_inner`（1425-1477）与 `execute_staged`（1490-1665）、`git diff 6412caf 9401b2d -- src/pipeline.rs`（只含 `total_start` 传参与两处打印点）。
- 测试核对：`git diff 6412caf 9401b2d -- tests/diagnostics_statement_test.rs`（两个新用例 + 217 行补断言）、静态统计 `tests/diagnostics_statement_test.rs` 的 9 个测试函数与 `tests/diagnostics_wait_test.rs` 的 6 个测试函数（与 Act 报告的 B5 内两个目标计数一致）。
- 逐批自洽核对（只读静态复核，不执行任何测试）：枚举 `ls tests/*.rs | sort` → 162；解析本 Cycle 的 Act Response 批次表并与枚举集合比对——162 个名字唯一、无缺失、无多余、顺序与字典序一致；批大小 12×13 + 6 = 162；每批 passed 与该批文件的测试属性计数逐一相等（两处已知 0 用例目标除外，扣除后完全相等）；lib 443 = `src/` 内 443 个测试属性、bins 0 = `src/main.rs` 0 个；总计 443 + 1966 = 2409 passed / 0 failed / 2 ignored（ignored 全在 B3 的 `cli_test`）。
- 规格与变更结构：`openspec validate 2026-10-30-diagnostics-legacy-table-error-path --strict` → `Change '2026-10-30-diagnostics-legacy-table-error-path' is valid`，退出码 0；`grep -n 'Requirement:\|Scenario:' openspec/specs/unified-diagnostics/spec.md` 与 change 内 delta 的标题对照（MODIFIED 保留三个既有场景标题 + 新增一个）；`openspec/changes/<change>/evidence/` 不存在——Persisted Evidence 模式为 `none`，按公共规则不据此提出问题。
- 沿用的验证结论（来源：本 Cycle 的 Act Response「逐批清单」「对账结论」「验证命令与实际结果」；被覆盖的产品与测试代码自 `9401b2d` 起未变化，无公共规则 › 验证 列出的重跑触发条件）：15 批全部 exit 0、failed 全为 0、2409 passed / 0 failed / 2 ignored、目标集合对账成立、无失败批因而未触发整批复核。
- 本 Review 实际执行的命令：`git status` / `git log` / `git diff`（只读）、`ls tests/*.rs | sort`、`grep -rn` / `grep -n` / `grep -c`（只读检索）、`sed` 读源码、只读静态核对脚本（解析 Act Response 的批次表并统计测试属性）、`openspec validate --strict`。**未运行任何 `cargo test`，未重跑任何批次。**

**Follow-up Decision**

接受。本轮零产品代码改动，仓库现场与 Act Response 声明一致；逐批记录在批数、覆盖目标、退出码、passed/failed/ignored 计数、总计与对账六个维度上互相自洽，且数量与对账部分可由源码静态复现；M19 的分批、串行、单线程与「缺一批不得声明全量通过」约束逐条成立；Cycle 的 6 条 Acceptance 全部满足，Iteration 000 推迟的验收面已在 B11 闭合；delta 规格与 design 与已实现行为逐条一致，权威规格的既有场景标题全部保留且 `openspec validate --strict` 通过。三条 Minor 均为非阻塞项（取证限制、范围外既有格式噪声、范围外 ISS42），按公共规则不为其创建修复、不要求当前 Cycle 返工。当前 Cycle 无需修复，不创建 rework / replan Cycle；Iteration 001 完成，本 change 无剩余 Iteration。

**Iteration Plan Update**

None。目标、范围、依赖、验证方式与验收边界均未改变：`tasks.md` 的 Iteration Plan 保持 000 = 旧表输出时机收敛（T1、T2）、001 = 收尾全量（T3）。

**Next Cycle**

None。

**Next Iteration**

None——`tasks.md` 的 Iteration Plan 中没有剩余 Iteration；本 change 的全部任务（T1、T2、T3）已归属并完成。
