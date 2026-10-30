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

- Status: 尚未由 Act 填写

尚未执行。

## Plan Review

- Review Result: pending
