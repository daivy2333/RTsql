# Iteration 003 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 需求 9（收尾全量回归）与 M19 的分批执行约束。本 change 已完成的产品改动（三轮的产品代码与新增测试）在本轮只被回归，不被修改：Iteration 000 的诊断门面（`src/diagnostics.rs`、`src/lib.rs`、`tests/diagnostics_facade_test.rs`）、Iteration 001 的语句阶段（`src/pipeline.rs`、`src/database.rs` 的 `commit` 接线、`tests/diagnostics_statement_test.rs`）、Iteration 002 的等待归属（`src/diagnostics.rs` 的等待类别、`src/pipeline.rs` 的 `parse` / `plan` open 阶段、`src/database.rs` 的 `commit` open 阶段、`src/transaction/manager.rs`、`src/wal/buffer.rs`、`src/storage/file_storage.rs`、`tests/diagnostics_wait_test.rs`）。三轮的 Plan Review 结论均为 `accepted`。
- Excluded scope: 任何产品代码、测试代码、夹具、`Cargo.toml` / `Cargo.lock`、criterion 门槛的修改；范围外缺陷的修复（ISS42 / ISS43 已在主仓库登记，处置留给用户与后续流程）；新的行为需求；把全量失败就地修掉（本轮不改代码，失败即停止并回报）。

**Objective**

在本 change 的默认构建上，按 M19 的分批规程把全部测试目标跑一遍（这是整个 change 唯一的一次全量）：枚举得到的集成测试目标与 `--lib` / `--bins` 全覆盖，每一批退出码 0 且 failed 计数为 0，`--test` 名字集合与枚举集合无缺失、无多余、无重复，且批间串行、每批单线程。

**Background**

本 change 分三轮交付了诊断门面、语句阶段与等待归属，三轮的 Plan Review 结论都是 `accepted`（`iterations/000-diagnostics-facade/000-initial.md`、`iterations/001-statement-phases/000-initial.md`、`iterations/002-wait-attribution/000-initial.md`）。三轮各自只跑了受影响范围的定向验证，从未跑过全量——按 M19，全量预算在整个 change 一次，并放在收尾轮（`tasks.md` 的 Iteration Plan「Iteration 003：收尾全量」，与 MS43 的收尾轮同形：避免行为失败与整库回归混在一次排障里）。本轮是本 change 的最后一轮：只执行验证，不再产生新的实现。

**Investigation Facts**

- Starting point: 产品代码树 = `c767973`（Iteration 002 的 Act 提交「act: 实施等待归属 T3（RED→GREEN，6/6）并记录 Act Response」）；本 Review 只改 change 文档，不动产品代码，因此 T4 开始时的产品代码就是 `c767973` 的树。分支 `changes/2026-10-28-ms44-unified-diagnostics`（基点 `e69c7e2`，change 自创建起只在该分支上演进）。
  - 前三轮的最终改动：000 = `src/diagnostics.rs`（门面、配置、上限、父子耗时）+ `src/lib.rs`（`pub mod diagnostics;`）+ `tests/diagnostics_facade_test.rs`；001 = `src/pipeline.rs`（`execute_with_params` 绑定点、`execute_inner` 拆出 `execute_staged`、`parse` / `plan` / `execute` 阶段）+ `src/database.rs`（`Database::commit` 上报 `commit` 子阶段）+ `tests/diagnostics_statement_test.rs`；002 = `src/diagnostics.rs`（等待类别 `WaitTotals`、内部 `StageNode`、跨任务 `WaitSlot`）+ `src/pipeline.rs`（`parse` / `plan` 改 open 阶段）+ `src/database.rs`（`commit` 改 open 阶段）+ `src/transaction/manager.rs`（会话闸门 Lock）+ `src/wal/buffer.rs`（等待者登记携带槽与时刻、`flush_locked` 回写 Io / Schedule）+ `src/storage/file_storage.rs`（读页 Io 与页数）+ `tests/diagnostics_wait_test.rs`。
  - 前三轮的验证结论（沿用，来源为各 Cycle 的 Act Response；被覆盖代码此后未再变化，无公共规则 › 验证 列出的重跑触发条件）：000 = `cargo test --offline --test diagnostics_facade_test` 退出码 0、10 passed / 0 failed；001 = `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 退出码 0、7 passed / 0 failed（`diagnostics_facade_test` 10 passed / 0 failed 同时沿用）；002 = `cargo test --offline --test diagnostics_wait_test` 退出码 0、6 passed / 0 failed，`cargo test --offline --test recovery_bounded_read_test` 退出码 0、6 passed / 0 failed，`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 退出码 0、7 passed / 0 failed，`cargo test --offline --test diagnostics_facade_test` 退出码 0、10 passed / 0 failed，`cargo test --offline --lib wal::` 17 passed / 0 failed，`--test wal_buffer_test --test wal_integration_test --test file_storage_io_test --test storage_test` 5 + 2 + 21 全 0 failed，`--test concurrent_test --test checkpoint_test --test free_page_lifecycle_test --test wal_auto_checkpoint_test` 2 + 10 + 12 + 11 全 0 failed。
- Current-State Evidence:
  - 集成测试目标的唯一来源是 `tests/` 目录下的 `*.rs` 文件：`ls tests/*.rs | sort` 在 HEAD `c767973` 上给出 162 个文件（`diagnostics_facade_test` / `diagnostics_statement_test` / `diagnostics_wait_test` 三个本 change 新增文件在列）。本轮按 M19 在执行时现场枚举，不把 162 这个数写进通过条件。
  - 目标名映射：文件名去掉 `.rs` 即 `--test` 的名字；`--test <名>` 与文件名一一对应（`cargo` 由文件名推导目标名，不存在手写别名）。
  - 构建配置：`Cargo.toml` 的 `default = ["cli", "network", "crypto"]`。162 个测试文件中有 116 个带 `#![cfg(...)]` 门（70 个 `network`、33 个 `network` + `cli`、12 个 `cli`、1 个 `not(crypto)`）。默认特性下前 115 个照常编译运行；`tests/crypto_off_build_test.rs` 是 `#![cfg(not(feature = "crypto"))]`，在默认构建下该目标编译为 0 个用例（退出码 0、0 passed / 0 failed）——它仍必须出现在枚举与批次里，不得因「0 个用例」判为缺失。
  - `benches/`（12 个 criterion 基准）与 `examples/` 不在 M19 规程选择的 `--lib` / `--bins` / `--test` 目标内，因此 criterion 门槛本轮不被执行，也不构成任何比较依据。
  - 离线可用性：`--offline` 所需的全部 dev-dependency（`criterion 0.5.1`、`rusqlite 0.31.0` + `libsqlite3-sys 0.28.0`、`tempfile`、`which`、`libc`、`futures`）在本机 cargo registry 中已有源码与 `.crate` 缓存；worktree 的 `target/` 目录（4.1G）已由前几轮构建产物填充，逐批执行复用增量产物。
  - 既有基线噪声（非本 change 引入）：`cargo fmt --check` 报两处既有漂移（`tests/diagnostics_facade_test.rs:216` 属 Iteration 000、`tests/session_lifecycle_test.rs:15` 在本 change 的 diff 之外）。它们不属于本轮判定面，也不影响测试退出码。
  - 本轮不改代码 ⇒ 没有先行测试需要建立，也没有新的验证命令之外的行为断言需要写。
- Code and Critical Path: 本轮不改任何产品代码与测试代码，不新增文件。执行对象是 change 分支上 `c767973` 的默认构建产物：`src/lib.rs` 的 lib 单测目标、`[[bin]]` 的 bin 单测目标、以及 `tests/*.rs` 的 162 个集成测试目标。行为承载面由前三轮交付的代码决定（诊断门面、语句阶段、等待归属），本轮只观察它们在整套测试下是否仍然成立。

**Implementation Guidance**

- 本轮唯一的动作是执行 M19 的分批规程并如实记录。规程全文在下面 T4 的 Required behavior 与 Verification 里，Act 按该规程逐步执行，不自行增删命令、批次或过滤条件。
- 记录格式：每批在 Act Response 的「验证命令与实际结果」中写一行——批号、该批的完整命令、退出码、passed 计数、failed 计数。批次的划分（哪些名字在哪一批）也一并写出，便于收尾核对覆盖。
- 不需要脚本来编排批次：M19 要求的「枚举 → 逐批一条命令 → 批间串行」直接由执行者在 shell 中逐条执行即可；不得为此新增脚本、封装命令或判定层。

**Task Definitions**

### T4: 一次分批串行的全量回归

- Requirement/Scenario: proposal 需求 9（收尾全量回归）；M19「全量回归验证 SHALL 分批执行」；spec 库中 `openspec/specs/` 的既有行为要求在本轮以整套测试的形式被验证一次。本轮不产生新的行为要求，场景是「在默认构建上跑完全部测试目标并逐批判定」。
- Depends on: T3（Iteration 002 accepted，Plan Review 结论 `accepted`）。
- Targets: 无产品代码目标（本轮零代码改动）。执行对象：`src/lib.rs` 的 lib 单测目标、`[[bin]]` 的 bin 单测目标、`tests/*.rs` 的全部集成测试目标（执行时由 `ls tests/*.rs | sort` 现场枚举）。
- Current behavior: 本 change 从未跑过全量——三轮各自只跑了受影响范围的定向验证（见 Starting point 的沿用结论）。分支上带着三处产品代码改动与三个新增集成测试文件，尚未在整套测试下被观察。
- Required behavior: 按下面的 M19 分批规程执行一次全量，产出一份逐批的退出码与 passed / failed 计数，并完成目标集合对账。具体规程（Act 逐步执行，不得改动）：
  1. **枚举（执行时现场枚举）**：运行 `ls tests/*.rs | sort`，得到有序的集成测试文件列表；每个文件名去掉 `.rs` 得到一个 `--test` 名字，构成集合 `E`。不得把规划时的文件数量写死，也不得凭记忆列举。
  2. **第一批单独执行**：`cargo test --offline --lib --bins -- --test-threads=1`。`--lib` 与 `--bins` 只出现在这一批。
  3. **其余分批**：把 `E` 中除第一批外的名字按步骤 1 的排序，每 12 个一批（最后一批可以少于 12 个）。每批一条命令，名字在批内保持步骤 1 的字典序：
     `cargo test --offline --test <名1> --test <名2> ... --test <名12> -- --test-threads=1`
  4. **批间串行**：一条命令结束后再开始下一条；同一时刻只有一批在跑；批次之间不并发、不叠加。
  5. **禁止项**：不得运行不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`（无过滤全量，任何并行度都不行）；不得让任何全量批次使用默认并行度——每批都必须带 `-- --test-threads=1`。
  6. **对账**：全部批次用到的 `--test` 名字并集必须等于 `E`：无缺失、无多余、无重复；`--lib` / `--bins` 只出现在第一批。对账不成立即为失败，不补跑已通过的批。
  7. **逐批记录**：每批把命令、退出码、passed 计数、failed 计数写入 Act Response。退出码 0 且 failed 计数为 0 才算该批通过。任一批未执行，不得声明全量通过（全量 Gate 等价于「全部批次各自 0 failed」）。
  8. **失败处置**：某一批失败时，只把该批按同一命令、同一并行度再跑一次；第二次仍失败则本任务失败——不改并行度，不重跑已通过的批，不在本轮改代码。复核通过按通过处理，并在验证记录里注明首跑失败。
- Preserve: 产品代码、测试代码、夹具、`Cargo.toml` / `Cargo.lock`、criterion 门槛一律不改；三轮已 accepted 的行为面（诊断门面的配置与上限、语句阶段与父子耗时口径、等待类别与跨任务归属、`[recovery]` 告警原文、旧计时表的三段 stderr）不改；本轮是本 change 唯一的一次全量——不得因为某批失败而另起一次全量。
- Forbidden: 无过滤的 `cargo test`；默认并行度跑任何全量批；修改任何产品代码或测试代码（含注释与格式）；新增测试、fixture、脚本、封装命令或判定层；为判定「同一轮运行」引入 Hash / 指纹 / revision pin / run-id / 序号 / manifest / 时间顺序证明；跳过失败批或删减批次来凑通过；重跑已通过的批；把 criterion 基准当通过条件；把「之前跑过」当作本轮的实跑结论。
- First test: 无先行测试——本轮不改代码，没有行为变更需要先行测试（公共规则 › TDD 的例外：本轮只有验证，没有实现变更）。首次执行即第一批：`cargo test --offline --lib --bins -- --test-threads=1`。
- GREEN condition: 全部批次的退出码均为 0 且 failed 计数均为 0，且步骤 6 的对账成立（`--test` 名字并集 == `E`，无缺失、无多余、无重复；`--lib` / `--bins` 只在第一批），且 Act Response 写齐了每一批的命令、退出码与 passed / failed 计数。
- Verification: 逐批的直接判定——每批命令的退出码与输出中的 failed 计数（退出码 0 且 failed 为 0 即该批通过）；全量结论等价于全部批次各自 0 failed且对账成立。不引入人工核对表、脚本或任何判定层；不用 hash / revision / 时间戳等身份类字段作为通过条件。
- Stop when: 任一批第二次复核仍失败 → 停止，不改代码、不改并行度、不重跑已通过的批，写 Blocker Handoff 并把 Act Response 状态置为 `blocked`（写明失败批次、失败用例与实际输出），等待用户决定；枚举或对账不成立（缺失 / 多余 / 重复 / `--lib`、`--bins` 出现在非第一批）→ 停止并回报，不补跑；执行环境无法满足 M19 的前提（离线依赖不可用、机器承载不足）→ 停止并回报，不降级为无过滤全量或默认并行度。

**Invariants**

- M19 的分批约束在本轮全程成立：无过滤全量、默认并行度全量、跳过批次、重复跑已通过批次都不允许。
- 全量只跑一次；失败批的唯一复核机会用尽后即停止，不在本轮修代码。
- 验证结论只来自实际运行的退出码与 failed 计数；不引用「上次跑过」代替本轮实跑，也不把 revision / 时间戳 / 身份类信息当通过条件。
- 公开诊断面与既有 stderr 告警的行为面（Iteration 000 / 001 / 002 已 accepted 的部分）在本轮不改、不重新裁定。

**Non-goals**

- 不在本轮修任何失败（含本 change 引入的失败）——失败即停止并回报，由后续流程决定承载方式。
- 不改产品代码、测试代码或构建配置；不新增测试与基准。
- 不承载 ISS42 / ISS43 的修复（范围外既有行为，处置留给用户与后续流程）。
- 不做性能比较、不设数值开销门槛、不解释三者的快慢。

**Acceptance**

- 全部批次的退出码为 0 且 failed 计数为 0（proposal 需求 9；M19「全量 Gate 等价于全部批次各自 0 failed，缺一批不得声明全量通过」）。
- `--test` 名字集合等于 `ls tests/*.rs | sort` 去掉 `.rs` 后的集合：无缺失、无多余、无重复；`--lib` / `--bins` 只出现在第一批（对账可核对）。
- 批间串行、每批 `-- --test-threads=1`、全程无无过滤 `cargo test`（M19 分批执行约束）。
- Act Response 写齐每批的命令、退出码、passed / failed 计数，并写明全量结论所依据的分批方式（M19「验证记录中的全量结论 SHALL 注明分批执行方式并保留各批的 passed/failed 计数」）。
- 本轮的产品代码零改动：`git diff` 中本轮只有 change 文档（T4 只跑验证）。
- Requirement Traceability：proposal 需求 9 → T4（分批命令序列与对账），Covered。

**Verification**

- 逐批的直接判定（自动命令）：每批一次 `cargo test --offline ... -- --test-threads=1`，以退出码与输出中的 failed 计数判定；全量结论 = 全部批次各自 0 failed + 目标集合对账成立。
- 无需人工验证步骤；无端到端类人工场景（目标集合与批内用例数由命令输出直接给出）。
- 沿用前三轮的定向结论：本轮不重复执行那三条定向命令（`diagnostics_wait_test` / `recovery_bounded_read_test` / `diagnostics_statement_test` / `diagnostics_facade_test` 的 GREEN 已在本轮之前取得且覆盖范围未变化）；它们同时是本轮全量批次的一部分，会在全量中被再次覆盖。
- 本轮是本 change 的唯一一次全量——不再另起全量。

**Gate 2 Readiness**

PASS。检查项均通过，无 BLOCKED，无 SKIPPED。调查基于本会话对 `Cargo.toml`（默认特性、dev-dependency）、`tests/` 目录（162 个目标、116 个 `#![cfg(...)]` 门）、`benches/` 与 `examples/`（不在 M19 选择集内）、各轮 Cycle 文件与 Act Response 的实际阅读；执行依据自包含（M19 规程全文在 Task Definition 的 Required behavior 与 Verification 里，Act 不需要回读本 change 的其他产物）；验证边界明确（一次全量，逐批退出码与 failed 计数 + 目标集合对账）；RTM 无 Missing、无 Simplified；Persisted Evidence 为 none；批准来自本次 goal 对「把这个 change 做完」的授权（goal 注入授权，覆盖 Gate 1 / Gate 2 与计划批准，检查标准不降低）。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- **`commit_wait_classes_present` 的 `Queue > 0` 断言依赖唤醒延迟**（Iteration 002 Plan Review 的 Minor finding 1）：按实现，Queue = `notified().await` 实测耗时 − Io − Schedule，余量取决于等待任务被唤醒并再次轮询的延迟，不是结构性下界；该断言在本轮全量中可能出现偶发失败。按 M19 第 7 条与公共规则 › 验证，偶发失败的处理是「只把该批再跑一次」——第二次仍失败则本任务失败（`blocked`），不得改并行度、不得改断言、不得重跑其他已通过的批。若该批在首跑失败而复核通过，按公共规则 › 验证 记为通过，并在验证记录里注明首跑失败。
- `tests/crypto_off_build_test.rs` 在默认构建（`crypto` 开启）下编译为 0 个用例，该批输出 0 passed / 0 failed 且退出码 0——不是缺失目标，不得据此判定对账不成立。
- 逐批执行会产生多次编译；worktree 的 `target/` 已有前几轮的产物可复用，但批次之间不共享编译进程（每批一条独立命令）。
- 机器承载：M19 的分批规程正是为「无过滤全量一次性构建链接全部测试二进制并长时间独占机器」而立；不得为了省时间把多批合并成一条命令或改用默认并行度。
- 全量失败不等于「分批规则错了」——不把全量失败当成修改分批规则的理由（Iteration 003 的 Diagnostic boundary）。
- 本轮不做任何超出收尾验证的清理；两处既有 rustfmt 漂移与前两轮的 Minor finding 均不在本轮处理面。

## Act Response

- Status: reported
- Cycle: Iteration 003 / Cycle 000（收尾全量）
- 承载任务：T4（change `tasks.md` 4.1）

**实际改动**

零产品代码改动。本轮只执行 M19 的分批全量并如实记录：不改产品代码、测试代码、夹具、`Cargo.toml` / `Cargo.lock`、criterion 门槛，不新增测试、fixture、脚本、封装命令或判定层，不引入任何 Hash / 指纹 / revision pin / run-id / 序号 / manifest / 时间顺序证明。本轮工作区的唯一改动是本文件与 change `tasks.md` 的文档更新；执行验证前后 `git status --short` 均为空输出。

**与计划的偏差（Deviations from Plan）**

None（仅一处执行细节说明，非行为差异）。Plan Context 的 Required behavior 第 3 条写「把 `E` 中除第一批外的名字按步骤 1 的排序，每 12 个一批」；`E` 是集成测试目标名集合，第一批只含 `--lib` / `--bins`、不含任何 `--test` 名字，故本轮按「`E` 的 162 个名字按步骤 1 的排序切成每批 12 个」执行，产出 14 个 `--test` 批（13 批 × 12 + 末批 6）。这与该条字面读法同形，且正是对账成立的前提（全部 `--test` 名字并集恰为 `E`）。除此之外逐条按 Required behavior 第 1–8 条执行，无偏差。

**自检发现与修复**

None（本轮零代码改动，无可检对象；无 Critical / Important / Minor 发现）。

Gate 3（Task Review）以「执行依据合规性」核对：15 条命令逐条符合 Required behavior 第 2 / 3 / 5 条（第一批单独含 `--lib` `--bins`；其余每批 ≤ 12 个 `--test`；每批均带 `-- --test-threads=1`）；全程未运行任何不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`；无任何批使用默认并行度；批间串行（每批命令退出后才启动下一批，同一时刻只有一批在跑）；无失败批，故第 8 条的整批复核机会未动用；无已通过批被重跑；无批被跳过或删减。

**验证命令与实际结果**

执行时现场枚举：`ls tests/*.rs | sort` → 162 个文件，构成 `E` = 去掉 `.rs` 后的 162 个 `--test` 名字（枚举结果未写死进通过条件）。构建配置为 `Cargo.toml` 的 `default = ["cli", "network", "crypto"]`。

逐批结果（批号 | 覆盖目标数 | 退出码 | passed | failed）：

| 批 | 覆盖目标数 | 退出码 | passed | failed |
|---|---|---|---|---|
| B1 | `--lib` + `--bins`（2 个测试二进制） | 0 | 443 | 0 |
| B2 | 12 | 0 | 154 | 0 |
| B3 | 12 | 0 | 199 | 0 |
| B4 | 12 | 0 | 155 | 0 |
| B5 | 12 | 0 | 114 | 0 |
| B6 | 12 | 0 | 184 | 0 |
| B7 | 12 | 0 | 76 | 0 |
| B8 | 12 | 0 | 113 | 0 |
| B9 | 12 | 0 | 133 | 0 |
| B10 | 12 | 0 | 240 | 0 |
| B11 | 12 | 0 | 92 | 0 |
| B12 | 12 | 0 | 147 | 0 |
| B13 | 12 | 0 | 163 | 0 |
| B14 | 12 | 0 | 142 | 0 |
| B15 | 6 | 0 | 52 | 0 |
| **合计** | **164 个测试二进制（162 个集成测试目标 + lib + bin）** | **全部 0** | **2407** | **0** |

B1 的 443 passed 来自 `src/lib.rs` 的单测目标，`src/main.rs` 的 bin 目标为 0 个用例（0 passed / 0 failed，退出码 0）。

全部 15 条命令（原样，逐批串行执行，每批一行）：

```
B1  cargo test --offline --lib --bins -- --test-threads=1  ->  exit 0  |  443 passed  |  0 failed
B2  cargo test --offline --test aggregate_extension_test --test aggregate_output_shape_test --test aggregate_test --test alter_table_test --test attach_session_test --test btree_merge_test --test btree_scale_test --test btree_split_test --test btree_test --test busy_timeout_test --test cache_perf_test --test catalog_bounded_read_test -- --test-threads=1  ->  exit 0  |  154 passed  |  0 failed
B3  cargo test --offline --test catalog_durability_test --test chain_join_test --test check_command_test --test checkpoint_command_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test checkpoint_test --test cli_test --test composite_index_lifecycle_test --test composite_index_runtime_test --test concurrent_test --test connection_limit_test -- --test-threads=1  ->  exit 0  |  199 passed  |  0 failed
B4  cargo test --offline --test constraint_enforcement_test --test cross_db_dml_test --test cross_db_query_test --test crypto_off_build_test --test ctas_test --test cte_test --test data_page_bounded_read_test --test database_file_lock_test --test datetime_function_test --test datetime_type_test --test ddl_catalog_transactional_test --test ddl_crash_recovery_test -- --test-threads=1  ->  exit 0  |  155 passed  |  0 failed
B5  cargo test --offline --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test default_declaration_test --test default_precheck_test --test derived_scan_alias_test --test diagnostics_facade_test --test diagnostics_statement_test --test diagnostics_wait_test --test dml_tombstone_abandon_window_test --test dml_tx_id_test --test drop_persistence_test --test drop_table_free_test -- --test-threads=1  ->  exit 0  |  114 passed  |  0 failed
B6  cargo test --offline --test e2e_test --test encryption_test --test executor_test --test explain_test --test explicit_tx_test --test expression_e2e_test --test file_header_test --test file_storage_io_test --test free_page_lifecycle_test --test full_table_dml_test --test gc_test --test group_by_expr_test -- --test-threads=1  ->  exit 0  |  184 passed  |  0 failed
B7  cargo test --offline --test hash_join_build_side_test --test import_input_surface_test --test index_key_encoding_evaluation_test --test index_manager_test --test index_page_ownership_stack_overflow_test --test index_page_ownership_test --test index_range_routing_test --test index_read_bounded_read_test --test index_write_atomicity_test --test insert_column_list_test --test insert_select_test --test isolation_level_test -- --test-threads=1  ->  exit 0  |  76 passed  |  0 failed
B8  cargo test --offline --test join_output_resolution_test --test join_test --test key_type_conformance_test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test -- --test-threads=1  ->  exit 0  |  113 passed  |  0 failed
B9  cargo test --offline --test nested_loop_join_test --test network_protocol_test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test --test param_binding_test --test parser_test --test pg_integration_test --test pg_messages_test --test pg_protocol_test -- --test-threads=1  ->  exit 0  |  133 passed  |  0 failed
B10  cargo test --offline --test pipeline_test --test plan_cache_test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test --test projection_expression_test --test projection_test --test pushdown_test --test query_memory_test --test readonly_session_test -- --test-threads=1  ->  exit 0  |  240 passed  |  0 failed
B11  cargo test --offline --test recovery_bounded_read_test --test recovery_e2e_test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test --test release_durability_precondition_test --test release_path_bounded_read_test --test runtime_test --test scalar_function_test --test scalar_subquery_comparison_test -- --test-threads=1  ->  exit 0  |  92 passed  |  0 failed
B12  cargo test --offline --test schema_persistence_test --test secondary_index_equality_test --test secondary_index_test --test select_distinct_test --test self_join_test --test session_lifecycle_test --test set_operations_test --test sort_merge_join_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test -- --test-threads=1  ->  exit 0  |  147 passed  |  0 failed
B13  cargo test --offline --test string_index_runtime_test --test string_index_sql_test --test string_secondary_index_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test --test subset_insert_test --test sync_loader_test --test table_alias_resolution_test --test table_manager_test --test truncate_test --test tx_statement_test -- --test-threads=1  ->  exit 0  |  163 passed  |  0 failed
B14  cargo test --offline --test typed_api_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_integration_test -- --test-threads=1  ->  exit 0  |  142 passed  |  0 failed
B15  cargo test --offline --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1  ->  exit 0  |  52 passed  |  0 failed
```

**目标集合对账（Required behavior 第 6 条）**

- 枚举集合 `E`：`ls tests/*.rs | sort` → 162 个文件 → 162 个 `--test` 名字。
- 实际执行集合：从 B2–B15 十五条命令的运行输出中提取 `Running tests/<名>.rs` 行，得到 162 个 `--test` 名字。
- 差集核对：两个有序列表 `diff` 结果为空（完全相同）——**无缺失、无多余、无重复**。
- `--lib` / `--bins` 隔离：在 B2–B15 的运行输出中 `src/lib.rs` 与 `src/main.rs` 均不出现，只在 B1 出现——**`--lib` / `--bins` 只出现在第一批**。
- 0 用例目标按 Plan Context 的口径处理，不判为缺失：
  - `tests/crypto_off_build_test.rs`（B4）：默认构建开启 `crypto`，该目标为 `#![cfg(not(feature = "crypto"))]`，编译为 0 个用例，退出码 0、0 passed / 0 failed。
  - `tests/network_server_test.rs`（B9）：文件体内的用例整体处于 `/* ... */` 注释块中（M8 起暂停 JsonProtocol 场景测试），编译为 0 个用例，退出码 0、0 passed / 0 failed。
  - `src/main.rs`（B1 的 bin 目标）：0 个用例，退出码 0、0 passed / 0 failed。
  - 三个目标都实际参与了构建与执行（运行输出中有对应 `Running` 行），均出现在枚举集合与批次中，对账成立。
- 对账结论：**成立**。

**全量结论**

全量通过。本 change 唯一的一次全量回归按 M19 分批串行执行完成：15 个批次全部退出码 0、全部 failed 计数为 0，累计 164 个测试二进制、2407 passed / 0 failed；目标集合对账成立（162 个 `--test` 名字无缺失、无多余、无重复；`--lib` / `--bins` 只在第一批）。全量结论所依据的分批方式：现场枚举 → 第一批单独执行 `--lib --bins` → 其余按枚举排序每 12 个一批（末批 6 个）→ 批间串行 → 每批 `-- --test-threads=1`，无过滤 `cargo test` 一次未跑，无任何批使用默认并行度。

**Persisted Evidence**

None required（Mode: none）。全部判定依据是各批命令的退出码与输出中的 `test result:` 计数，可低成本按同一分批方式重跑；按公共规则 › 验证「验证辅助代码一旦需要独立协议、CLI、fixture、负向测试或审计器，即按身份证明机制处理」，不为本轮引入任何持久化协议或身份类字段。

**Experience Candidates**

None。

- Runbook 候选：无。本轮是仓库既有的 `cargo test` 分批执行，不含新的可复用高风险操作路径；M19 的分批规程已在 `tasks.md` 与本 Cycle 的 Plan Context 中写明，无需另立 Runbook。
- Issue 候选：无。本轮未发现当前 change 范围外的实质缺陷。ISS42 / ISS43 是变更范围外的既有行为，已在主仓库登记，处置沿用 Iteration 001 / 002 的判定（不在本 change 承载），本轮不重复报告。
- 既有 Minor finding 沿用前轮结论，本轮不处理：Iteration 001 遗留的两条（错误路径新增旧表输出、需求 5 的 spec 子句缺断言）；`tests/diagnostics_facade_test.rs:216` 与 `tests/session_lifecycle_test.rs:15` 两处既有 rustfmt 漂移。

**未解决问题**

None（本 Cycle 范围内）。无 Blocker Handoff，无 Blocker Resolution。

- Plan Context Risks 的两条已在本轮落地：`diagnostics_wait_test` 的 `commit_wait_classes_present` 首跑通过（6 passed / 0 failed），`Queue > 0` 断言未出现偶发失败，第 8 条的整批复核机会未动用；`crypto_off_build_test` 按上述判读口径处理，退出码 0、0 failed，不视为失败。
- 本 change 范围内无遗留失败、无失败批、无未跑批。

## Plan Review

- Review Result: accepted

**Findings**

1. **零产品代码改动：声明与仓库现场一致。**
   - `git diff --name-only 80fea5e f63591f` 只含两个 change 文档：`iterations/003-final-regression/000-initial.md` 与 `tasks.md`。产品代码（`src/`）、测试代码（`tests/`）、夹具、`Cargo.toml` / `Cargo.lock`、criterion 门槛均不在 diff 中——Act Response 的「零产品代码改动」属实。
   - `git diff --stat 80fea5e f63591f`：2 files changed, 106 insertions(+), 3 deletions(-)（Cycle 文件 +103 行 Act Response；`tasks.md` +3/-3 状态同步）。
   - 工作区 `git status --short` 空输出；分支 HEAD 仍是 Act 提交 `f63591f`，被覆盖代码此后未变化。
2. **逐批记录自洽（Plan 独立复核，未重跑全量）。**
   - 命令条数：Cycle 文件里 15 条 `cargo test` 命令 = 表中 15 行。B1 含 `--lib --bins`；B2–B15 的命令串中 `--lib` / `--bins` 出现次数为 0（grep 计数 1，仅 B1）。
   - 批内目标数：B2–B14 各 12 个 `--test`，B15 6 个 → 13×12 + 6 = 162，与枚举集合大小相等。
   - 目标集合对账：从命令串抽出 162 个 `--test` 名字，与 `ls tests/*.rs | sort` 去掉 `.rs` 后的 162 个名字 `diff` 为空（无缺失、无多余、无重复）；162 个名字按批展开后的顺序与字典序枚举逐行相同（分组顺序固定、批内有序）。
   - 逐批 passed 计数：Plan 用 `cargo test --offline --test <名> -- --list` 枚举每个目标的实际用例数（只枚举、不执行），逐批求和与 Act 表格比对——B2–B15 全部相等（154 / 199 / 155 / 114 / 184 / 76 / 113 / 133 / 240 / 92 / 147 / 163 / 142 / 52），集成批合计 1964。
   - 两处口径差异已核实来源：`tests/cli_test.rs` 有 2 个 `#[ignore]` 用例（不计入 passed，故 B3 = 201 − 2 = 199）；`cache_perf_test` 与 `index_page_ownership_stack_overflow_test` 的 `--list` 输出是「1 test」单数形式，首轮解析漏计，各补 1（B2、B7 各 +2）。
   - B1 独立核对：`cargo test --offline --lib -- --list` → `443 tests, 0 benchmarks`；`cargo test --offline --bins -- --list` → `0 tests, 0 benchmarks`，与 Act 的「443 来自 lib，bin 目标 0 个用例」一致。
   - 总计：443 + 1964 = 2407，与 Act 记录的 2407 相等；15 批退出码全 0、failed 全 0。
   - 0 用例目标：`cargo test --offline --test crypto_off_build_test -- --list` → `0 tests`；`--test network_server_test -- --list` → `0 tests`。两者都在枚举集合与批次中（B4 / B9），不判为缺失，与 Plan Context 的口径一致。
   - M19 合规：分批执行、批大小 12（经验值 8–12 内）、字典序固定分组、批间串行、每批 `-- --test-threads=1`、全程无无过滤 `cargo test`、全量只跑一次——Act Response 的记录与规程逐条一致。
3. **Acceptance 逐条核对。**
   - 「全部批次退出码为 0 且 failed 计数为 0」✓（15/15）。
   - 「`--test` 名字集合等于枚举集合，无缺失、无多余、无重复；`--lib` / `--bins` 只出现在第一批」✓（集合 diff 空、顺序一致、grep 计数 1）。
   - 「批间串行、每批 `-- --test-threads=1`、全程无无过滤 `cargo test`」✓（命令串逐条核对）。
   - 「Act Response 写齐每批的命令、退出码、passed / failed 计数，并写明全量结论所依据的分批方式」✓（15 行表格 + 15 条原样命令 + 对账段 + 全量结论段）。
   - 「本轮的产品代码零改动」✓（diff 只含两个 change 文档）。
   - 「M19 分批执行」✓（全量 Gate 等价于全部批次各自 0 failed，缺一批不得声明全量通过——15/15 批均有记录）。
4. **delta 与实现一致性（Plan 只改 change 产物，不改产品代码）。**
   - 需求 8：实现已在**所有**返回路径统一输出旧的三段计时表——`src/pipeline.rs:1456-1458` 的 `print_timings` 在 `execute_staged` 返回后无条件调用；变更前的 `e69c7e2:src/pipeline.rs` 在 parse / plan 失败等早退路径直接 `return`，不调 `print_timings`。因此 `RTSQL_PROFILING` 打开时失败语句现在也会打一张只含已完成阶段的旧表（parse 失败时阶段行为 `cache_hit_check` 与 `parse`）。这是 Iteration 001 Review 记录的非阻塞 Minor finding 1。
   - 本次处置：**不改产品代码**，把 delta 的需求 8 文字对齐到实现——新增一句「经语句管道执行的语句在该变量已设置时，每条返回路径都会输出这张表，含解析、规划等失败路径；失败路径的表只列出该语句返回前已完成的阶段」，并新增一个 Scenario「失败路径也输出旧的三段计时表」。`proposal.md` 的需求 8 原文、本 Cycle 的 Acceptance 与 Plan Context 均未改写。
   - 需求 5 的子句「含有解析阶段」：实现满足——`src/pipeline.rs:1547-1557` 在返回 parse 错误前先 `close_stage("parse", elapsed)`，`StageStack::finish`（`src/diagnostics.rs:624`）取走根的已完成子阶段，`collect_stats`（`src/diagnostics.rs:488`）把它收成 `StageStat`。但 `tests/diagnostics_statement_test.rs:184-215` 只断言 severity 与错误文本 / SQLSTATE，未锁定该子句——沿用 Iteration 001 的 Minor finding 2，按 Minor finding 口径记录，不返工。
   - delta 本次新增的句子与 Scenario 同样没有对应断言（与需求 5 的既有缺口同类），一并记录，供用户决定。
   - 需求 1 的默认档位：delta 原写「未修改过的会话配置 SHALL 三项采集都关闭，且输出详细程度为关闭」，实现的 `DiagnosticsConfig::default()`（`src/diagnostics.rs:90-100`）是三项采集全关、输出详细程度为 `Detail::Summary`。两者在该默认下不可观测（采集全关 → `record_statement` 返回 `None` → 没有摘要可渲染 → 不写出新的诊断 stderr），但文字与实现不一致，且 `design.md` 的决策记录同样写的是 `Detail::Off`。本次处置：**不改产品代码**，把 delta 的该句改为「三项采集都关闭，输出详细程度为 Summary。该默认下没有摘要可渲染，SHALL NOT 产生新的诊断 stderr」，并把 `design.md` 的默认档位改为 `Detail::Summary` 并注明原设计稿写的是 `Detail::Off`、两者在该默认下不可观测。`proposal.md` 的需求 1 原文（「默认全部关闭」）未改——其可观察含义（默认不采集、不输出）成立。
5. **Issue 候选：无新增。** ISS42 / ISS43 已在主仓库登记，均为变更范围外的既有行为，不在本 change 承载，本轮不重复报告。

非阻塞 Minor finding（记录，不返工）：

1. **旧开关错误路径新增旧表输出（Iteration 001 遗留，Act 偏离）**：本轮只把 delta 文字对齐到实现，未改产品代码。可选处置留给用户：(a) 保持现状——delta 与实现一致，规格如实记录该行为；(b) 收敛为「错误路径不打印旧表」——那是产品语义变更（改 `src/pipeline.rs` 的返回路径、补断言、改 delta），超出 MS44 已批准范围，需另起 change 并由用户裁定，本轮不实施。
2. **需求 5 的 spec 子句缺断言（Iteration 001 遗留）**：行为已满足，测试未锁定「含有解析阶段」。记录，不返工；如用户要求，可在后续 change 补断言。
3. **delta 需求 8 本次新增的句子与 Scenario 无对应断言**：与 finding 2 同类，记录。
4. **默认输出档位的文字与实现不一致**：delta 与 `design.md` 原写默认 `Detail::Off`，实现是 `Detail::Summary`（`src/diagnostics.rs:90-100`），在该默认下不可观测。本轮已把两处文字对齐到实现并保留原设计稿的记述。可选处置留给用户：若要求「默认档位即 `Off`」（用户只开采集、不设档位时也不写 stderr），那是产品语义变更，需另起 change 改默认值并补断言，本轮不实施。
5. **两处既有 rustfmt 漂移**（`tests/diagnostics_facade_test.rs:216`、`tests/session_lifecycle_test.rs:15`）非本变更引入，本轮未触碰。
6. **Act Response 的章节标题为中文变体**而非 Cycle 模板的英文名——语义项一一对应，属平台适配，不构成偏差。
7. **严重级别名称空间**：delta 允许 Error / Warn / Info / Debug / Trace，实现的 `Severity`（`src/diagnostics.rs:106-110`）只有 Info / Warn / Error 三个变体。delta 的措辞是「只使用」这五个名字（约束取值空间），实现取其子集，成立；但 Debug / Trace 当前不可产出，供维护者合并规格时知悉。

**Acceptance Gaps**

None。Acceptance 的 6 条逐条成立：全批 0 failed（15/15）、目标集合对账成立（162 = 162，无缺失 / 多余 / 重复，`--lib` / `--bins` 只在第一批）、批间串行与每批单线程、无无过滤全量、逐批记录完整、产品代码零改动。M19 的全量 Gate「全部批次各自 0 failed，缺一批不得声明全量通过」满足。

**Evidence**

- diff：`git diff --name-only 80fea5e f63591f` → 只有 `iterations/003-final-regression/000-initial.md` 与 `tasks.md`；`git diff --stat 80fea5e f63591f` → 2 files changed, 106 insertions(+), 3 deletions(-)。`git status --short` 空输出。
- 本次 Review 的写入范围：只有 change 产物——本 Cycle 文件的 `## Plan Review` 段、`tasks.md` 的状态同步、`specs/unified-diagnostics/spec.md`（需求 1 默认档位一句 + 需求 8 一句与一个 Scenario）、`design.md`（默认档位一句）。产品代码、测试代码、构建配置零改动。
- 验证沿用 Act Response（来源：Iteration 003 Cycle 000 的「验证命令与实际结果」；Act 提交后 HEAD 未再前进、被覆盖代码未变化，且 M19 的全量预算在本 change 只有一次且已由 Act 用尽——**Plan 未重跑任何批次，未重跑全量**）。
- Plan 独立执行的只读检查（不执行用例，不消耗全量预算）：
  - 从 Cycle 文件抽出 15 条命令与 162 个 `--test` 名字，与 `ls tests/*.rs | sort` 做集合与顺序比对（diff 空、展开顺序逐行相同）。
  - `cargo test --offline --lib -- --list` → `443 tests, 0 benchmarks`；`cargo test --offline --bins -- --list` → `0 tests, 0 benchmarks`。
  - 162 个集成目标逐个 `cargo test --offline --test <名> -- --list`，求和得每批实际用例数，与 Act 的 15 行表格逐批相等（B2–B15 全等，集成合计 1964）。
  - `cargo test --offline --test crypto_off_build_test -- --list` → `0 tests`；`cargo test --offline --test network_server_test -- --list` → `0 tests`。
  - 代码核对：`src/pipeline.rs:1425-1480`（`execute_inner` 的统一 `print_timings`）、`src/pipeline.rs:1547-1557`（parse 失败前的 `close_stage`）、`src/diagnostics.rs:488`（`collect_stats`）、`src/diagnostics.rs:624`（`StageStack::finish`）、`git show e69c7e2:src/pipeline.rs`（变更前错误路径不调 `print_timings`）。
  - 定位到 2 个定向用例被执行（`cache_perf_test`、`index_page_ownership_stack_overflow_test`）：首次 `--list` 解析未识别「1 test」单数输出而改为直接运行，各 1 passed / 0 failed。属 M19 允许的单目标定向运行，未重跑全量、未改动任何已通过批次。
- Mode: none，无 required Evidence；未因目录不存在提出问题。

**Follow-up Decision**

Acceptance 已满足且无阻塞项；7 条 Minor finding 均不阻塞（其一为已对齐 delta 的错误路径旧表输出，其二与三为 spec 断言缺口但行为已满足，其四为已对齐文字的默认档位，其五为既有格式化基线，其六为格式适配，其七为严重级别名称空间），按 iteration-planning › Review 分类判 `accepted`。Act Response 的 Issue 候选为 None，本轮独立核对也未发现当前 change 范围外的新实质缺陷。T1–T4 全部完成，本 change 无剩余 Iteration，按公共规则 › Iteration 与 Cycle 线程记录 `Next Iteration: None`，收尾（规格合并、归档、全局状态同步）交由 `openspec-docs-maintainer` 执行。

**Iteration Plan Update**

None（accepted，无 replan）。

**Next Cycle**

None（当前 Cycle 无需修复，accepted 无返工）。

**Next Iteration**

None（本 change 无剩余 Iteration：T1 / T2 / T3 已 accepted，T4 由本 Cycle 承载并 accepted）。
