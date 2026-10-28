# Iteration 000 / Cycle 000: feature 基座与 network 可选化

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1、T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS42 全部需求中由本 Iteration 承载的部分（R1 默认组合等价与依赖处置、R2 network 可选组合）；proposal 默认假设 1–5 全部继续有效
- Excluded scope: crypto 门控（T3，Iteration 001）、cli 门控（T4，Iteration 002）、五组合矩阵与全量（T5/T6，Iteration 003）、层契约与诊断契约的收尾核对、任何依赖环与可见性改动、任何测试断言本体的改写

**Objective**

`[features]` 表与依赖处置落地后默认组合行为零变化；network feature 门控落地后 C1（no-network）组合的构建门与验证集通过，默认组合协议面由既有测试确认不变。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms42-module-boundaries`，HEAD `40e9ee8`（基点 master，工作区干净）。MS41 本机质量基线（fmt、deny clippy、分批全量）在覆盖未变化下直接沿用（来源：归档 change `2026-10-28-ms41-build-and-compat-baseline`；本分支基点相对其结论只含 docs 提交），本 Cycle 不重跑改前全量，以定向命令建立当次 GREEN。R79 事实面（revision 1f7dd6e）对本分支有效。
- Current-State Evidence:
  - `Cargo.toml`：17 个运行时依赖、无 `[features]` 表、无 `[[bin]]` 声明（bin 由 `src/main.rs` 自动发现，名为 `rtsql`，与 lib 同名）；dev-dependencies 为 criterion/rusqlite/tempfile/which/libc；`lru`、`anyhow` 全仓零引用；`futures` 仅 `tests/concurrent_test.rs`（4 处 `join_all`）。
  - `src/lib.rs` 全文 19 行：`pub mod cli;`（第 3 行）、`pub mod network;`（第 6 行）与其余 10 个模块声明 + 三组根重导出（executor/parser/result 来源，不含 cli/network）。
  - network 模块（8 文件）是叶子：src 内除自身零引用；`tokio-util`（rt feature）仅 `src/network/server.rs:10`（CancellationToken）。CLI 无 serve 子命令。
  - 测试锚定面（40e9ee8 实测）：104 个 tests 文件含 `use rtsql::network::`（97 个用 `network::protocol::{to_response, Response}`，其中 6 个协议/服务专项：pg_protocol_test、pg_integration_test、pg_messages_test、network_protocol_test、network_server_test、connection_limit_test；`executor_test.rs:497` 是唯一函数内 import 形态）；44 个文件含 `env!("CARGO_BIN_EXE_rtsql")`；两类重叠 33 个。无测试 import `rtsql::cli`。
  - src 内除 `#[cfg(test)]` 外零 `cfg(feature)`/`cfg(target)`；integration test 与被测包共享 feature cfg 集（Cargo 包级 feature 对 tests 目标生效）。
- Code and Critical Path:
  - `Cargo.toml` — T1 唯一改动文件（features、optional、`[[bin]]`、依赖增删；`Cargo.lock` 随首次构建更新）。
  - `src/lib.rs:6` — T2 的 network 门。`src/main.rs` 本 Cycle 不动（`[[bin]]` 声明在 T1 加入后，cli feature 在 C1 组合是开的，bin 正常构建）。
  - `tests/*.rs` — T2 加文件头/函数级 cfg 门；`executor_test.rs:497` 的函数级门。
  - 无产品代码路径变化：network 门在 feature 开启时是恒真条件，默认组合编译产物不变。

**Implementation Guidance**

T1 先于 T2：features 表落地后 T2 的 RED（C1 组合因 tokio-util 未启用而编译失败）才可观察。T2 的门控只做模块声明与测试文件头/函数属性，不进入 network 模块内部。若发现 src 内 network 之外有对 `crate::network` 的引用（R79 grep 为零，预期不出现），停止并按 Stop when 回报。`#![cfg(...)]` 必须是文件首个内层属性（`//!` 文档注释之后、任何 `use` 之前）。

**Task Definitions**

### T1: Cargo features 表与依赖处置，默认组合等价

- Requirement/Scenario: R1（S1、S12 之默认组合部分；死依赖不可观察）
- Depends on: None
- Targets: `Cargo.toml`；`Cargo.lock`（随构建更新，不手改）
- Current behavior: 无 features 表；17 个运行时依赖全部必选；`--no-default-features` 等组合不可区分（等于全量构建）；bin 自动发现、恒构建。
- Required behavior: `[features]` `default = ["cli", "network", "crypto"]`，`cli = ["dep:clap", "dep:clap_complete", "dep:csv"]`，`network = ["dep:tokio-util"]`，`crypto = ["dep:argon2", "dep:aes-gcm"]`；六个依赖标 `optional = true`；`[[bin]] name = "rtsql" path = "src/main.rs" required-features = ["cli"]`；`lru`、`anyhow` 从依赖表移除；`futures` 移入 `[dev-dependencies]`（版本不变）。默认组合构建产物与质量门行为不变。此任务不加任何 cfg 门。
- Preserve: 依赖版本零变更（只动 optional 与增删）；默认组合 `cargo check`/`fmt`/`clippy` 全绿；`tests/concurrent_test.rs` 编译不受 futures 移动影响。
- Forbidden: 修改 src/ 与 tests/ 任何文件；升级或新增任何依赖；引入 cfg 门；修改 benches 声明。
- First test: 改前 GREEN 基线——`cargo check --offline --lib --bins --tests`、`cargo fmt --check`、`cargo clippy --offline --all-targets -- -D warnings` 三条命令退出码 0（重构分支：改前 GREEN，改后保持 GREEN）。
- GREEN condition: Cargo.toml 按 Required behavior 修改后，同三条命令退出码仍为 0；`cargo check --offline --no-default-features --features cli,crypto --lib --tests` 失败（tokio-util 缺失类错误）——该失败是 T2 的 RED 输入，记录错误首行，不修复。
- Verification: 上述命令的实际退出码与关键输出行；首次构建不带 `--locked`（lockfile 更新），随后命令可带 `--locked`；`git diff Cargo.toml` 与 Required behavior 逐项对应。
- Stop when: 离线解析失败（缓存缺件）；默认组合任一命令非 0；lockfile 更新引入版本变更。

### T2: network feature 门控与测试面适配，C1 组合成立

- Requirement/Scenario: R2（S2、S7、S8、S11）
- Depends on: T1
- Targets: `src/lib.rs`（`pub mod network;` 一行加 `#[cfg(feature = "network")]`）；`tests/*.rs`（按规则加门，断言本体不动）；`tests/executor_test.rs:497` 所在测试函数
- Current behavior: network 模块无条件编译；C1 组合因 `pub mod network` 无门而引用未启用的 tokio-util，编译失败；104 个 network 锚定测试文件在 C1 组合同样无法编译。
- Required behavior: network 模块仅在网络 feature 开启时编译；network 锚定测试文件在关闭组合剥除为空目标；`executor_test.rs` 仅含 import 的那个函数被剥除、文件其余用例保留；C1 组合构建门与验证集通过；默认组合行为不变。
- Preserve: 门控文件清单按确定性规则枚举（执行时重新 grep，见 Verification）；测试断言与用例本体零改动；默认组合全部测试照常编译运行。
- Forbidden: 给 `pub mod cli` 加门（T4 范围）；改写 `to_response` 断言词表；动 network 模块内部文件；新增测试。
- First test: RED——`cargo check --offline --no-default-features --features cli,crypto --lib --tests` 失败（T1 记录的错误仍在；本任务开工时重跑确认）。
- GREEN condition: 同命令退出码 0；C1 验证集（下条）全部通过；默认组合六协议测试目标全部通过。
- Verification:
  1. 枚举（规则，执行时重跑）：`grep -l 'rtsql::network' tests/*.rs` 得 network 锚定集；`grep -l 'env!("CARGO_BIN_EXE_rtsql")' tests/*.rs` 得 bin 锚定集；交集加 `#![cfg(all(feature = "network", feature = "cli"))]`，network-only 加 `#![cfg(feature = "network")]`（bin-only 文件本任务不动，属 T4）；`executor_test.rs` 的函数内 import 加函数级 `#[cfg(feature = "network")]`。清单与计数（对规划值 104/44/33）记入 Act Response。
  2. C1 构建门：`cargo check --offline --no-default-features --features cli,crypto --lib --bins --tests` 退出码 0。
  3. C1 行为验证：`cargo test --offline --no-default-features --features cli,crypto --lib` 通过；`cargo test --offline --no-default-features --features cli,crypto --test storage_test --test parser_test --test executor_test` 通过（executor_test 剥除后其余用例执行）。
  4. 默认组合协议面：`cargo test --offline --test pg_protocol_test --test pg_integration_test --test network_server_test --test pg_messages_test --test network_protocol_test --test connection_limit_test` 全部通过。
- Stop when: src 内出现 network 之外的 `crate::network` 引用；任一验证命令失败且非门控遗漏所致（门控遗漏按清单补门后重跑对应命令一次）；默认组合协议测试失败（默认组合回归，立即停止回报）。

**Invariants**

- 默认组合行为逐字节等价：不改变任何 SQL 结果、错误语义、退出码、磁盘格式、输出。
- 不新增跨层依赖环边（design §5.3）。
- M19：全量只分批串行，本 Cycle 不跑全量；定向单目标命令用默认并行度。
- 不新增判定脚本/封装命令/证据工具；判定只看 cargo 退出码与原生输出。
- 新增代码不得引入新的 `eprintln!`/`println!` 直写出口（design §6.2 条 1）。

**Non-goals**

crypto 与 cli 门控、C2–C4 组合、组合矩阵收口、全量回归、层契约核对、测试断言迁移、依赖环处理、可见性收紧。

**Acceptance**

- R1（默认组合等价 + 死依赖移除不可观察）：T1 改前/改后三条基线命令均退出码 0；Cargo.toml 差异与 design §2.1 逐项一致。映射：proposal 需求 1、delta spec「默认组合行为等价」R1。
- R2（network 可选组合）：C1 构建门退出码 0 且验证集通过（映射 delta spec「network 可选组合」三 Scenario 的组合内部分）；默认组合六协议目标通过（「默认组合协议面不变」）；门控清单与计数记录完整。映射：proposal 需求 2。
- 完成声明基于当次运行；T1 的改前 GREEN 与 T2 的 RED/GREEN 均有命令与输出行。

**Verification**

逐 Scenario 最简直接判定（均真实入口）：

| Scenario | 判定 |
|---|---|
| S1 默认组合（本 Cycle 触及面） | T1 三条基线命令改前改后退出码 0 |
| S2 no-network 构建+核心行为 | C1 构建门 + `--lib` + 三核心测试目标退出码 0 |
| S7 双锚定文件 | C1 构建门退出码 0（33 个双锚定文件已带 `all(...)` 门，C1 即部分验证；cli 侧组合在 T4 复验） |
| S8 executor_test 函数级门 | C1 `--test executor_test` 通过且用例数 = 文件总用例数 − 1（剥除 network 用例） |
| S11 兼容 | install.sh/交叉脚本不动、不传 feature flag（事实核对，无命令） |
| S10/S3–S6/S9 | 不属本 Cycle |

**Gate 2 Readiness**

PASS（2026-10-28）。无 BLOCKED、无 SKIPPED。计划批准来自 goal 指令对 MS42 范围的授权。RTM 无 Missing、无 Simplified；接口与错误语义无 TBD（crypto 错误面在 design §3 已闭合，属 Iteration 001）。

**Persisted Evidence**

- Mode: none

命令、退出码与输出行写入 Act Response 即可；组合构建失败/通过可低成本复现（cargo 命令），不满足 required 白名单。

**Risks and Notes**

- 门控文件计数以执行时 grep 为准；与 104/44/33 的差异记入 Act Response，不改变门控规则本身。
- `executor_test.rs` 若存在规划未见的其他函数内 network 引用，按同一函数级门规则处理并记录。
- C1 组合 `--lib` 时长未知，首次执行记录实际耗时；异常超长按公共规则记录事实并回报，不静默改验证面。
- `dep:` 语法要求 Cargo ≥1.60（本机 1.99.1，满足）。
- futures 移 dev 后 `cargo check --all-targets`（含 benches）在默认组合仍编译（benches 不用 futures；concurrent_bench 属 dev 依赖域）。

## Act Response

- Status: reported

**实际改动**（107 files，+123/−28）

- `Cargo.toml`（T1）：新增 `[features]`（`default = ["cli", "network", "crypto"]`；`cli/network/crypto` 均为 `dep:` 形式）与 `[[bin]] name = "rtsql" path = "src/main.rs" required-features = ["cli"]`；clap、clap_complete、csv、tokio-util、argon2、aes-gcm 六依赖标 `optional = true`；移除 `lru`、`anyhow`；`futures = "0.3"` 移入 `[dev-dependencies]`（版本不变）。未加任何 cfg 门。
- `Cargo.lock`（T1，自动更新）：仅 19 行删除——`lru` 与其传递依赖 `allocator-api2` 整条移除；`anyhow` 因其他 crate 传递依赖仍在 lock 中。零版本变更、零新增。
- `src/lib.rs`（T2）：`pub mod network;` 上方加 `#[cfg(feature = "network")]`（+1 行）；`pub mod cli` 未动。
- `tests/`（T2，104 文件各 +1/−0）：33 个双锚定文件加 `#![cfg(all(feature = "network", feature = "cli"))]`；70 个 network-only 文件加 `#![cfg(feature = "network")]`（均插入于 `//!` 块之后、首个内容行之前；以 `//` 开头的文件插在注释前）；`tests/executor_test.rs` 的 `test_response_rows_serialization` 加函数级 `#[cfg(feature = "network")]`。断言与用例本体零改动。

**门控清单与计数**（执行时 grep 重跑）

- network 锚定 104、bin 锚定 44、交集 33——与规划值 104/44/33 完全一致。
- network-only 71 = executor_test.rs（函数门）+ 其余 70（文件头门）；bin-only 11 本任务不动（T4 范围）。
- `executor_test.rs` 全文件仅 497 行一处 `use rtsql::network::Response;`，无规划外的函数内 network 引用。
- src/ 内 network 之外 `crate::network` 引用为零（grep 证实，Implementation Guidance 前提成立）。

**TDD 轨迹**

- T1（重构分支 GREEN→GREEN）：改前三命令退出码 0 → 改后同三命令退出码 0；C1 check 退出码 101 = T2 的 RED 输入。
- T2 RED：`error[E0433]: cannot find module or crate \`tokio_util\` in this scope`。该命令运行于 T1 验证时；T2 开工点工作区状态与其完全相同（门控施加前零改动），按公共规则 › 验证 沿用，不重复运行。
- T2 GREEN：同命令（加 `--bins`）退出码 0。

**验证命令与结果**（全部当次真实运行；均 `--offline`，首次构建后均 `--locked`）

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo check --offline --lib --bins --tests`（改前） | exit 0 |
| 2 | `cargo fmt --check`（改前） | exit 0 |
| 3 | `cargo clippy --offline --all-targets -- -D warnings`（改前） | exit 0 |
| 4 | 同 1（T1 改后，首次不带 `--locked`，lockfile 更新） | exit 0 |
| 5 | 同 2、同 3（T1 改后） | exit 0 / exit 0 |
| 6 | `cargo check --offline --locked --no-default-features --features cli,crypto --lib --tests`（T1 后） | exit 101，`error[E0433]: cannot find module or crate \`tokio_util\` in this scope`（预期失败，T2 RED） |
| 7 | `cargo check --offline --locked --no-default-features --features cli,crypto --lib --bins --tests`（T2 后） | exit 0，9.3s |
| 8 | `cargo test --offline --locked --no-default-features --features cli,crypto --lib` | 431 passed / 0 failed，exit 0，36.7s（含构建，首次 C1 --lib 实测时长） |
| 9 | C1 `--test storage_test --test parser_test --test executor_test` | executor 38 passed（全文件 39 个测试函数 − 1 剥除，S8 成立）、parser 6、storage 21，exit 0 |
| 10 | 默认组合 `--test pg_protocol_test --test pg_integration_test --test network_server_test --test pg_messages_test --test network_protocol_test --test connection_limit_test` | 11+6+0+9+5+3 全 passed / 0 failed，exit 0 |
| 11 | T2 后默认组合：check（--lib --bins --tests）、clippy（--all-targets -D warnings）、fmt --check | 均 exit 0 |

- S11 兼容事实核对：install.sh 与交叉脚本零改动；本 Cycle 未向任何外部入口传 feature flag。
- M19 遵守：本 Cycle 未跑全量；全部为定向命令（单目标默认并行度）。

**与计划的偏差**

- 无实质偏差。两点记录：Plan Context Current-State Evidence 写「17 个运行时依赖」，实际 `Cargo.toml` 为 18 个；Required behavior 处置集（6 optional + lru/anyhow/futures）全部命中，计数误差不改变处置内容。T2 RED 未重跑命令，沿用依据见 TDD 轨迹。

**自检发现与已修复内容**

- `network_server_test.rs` 默认组合运行 0 用例为 HEAD 固有形态：M8 起测试体整体处于 `/* */` 块注释（HEAD 版本核实），非本次门控造成。
- `storage_test.rs`、`parser_test.rs` 均不在 network 锚定集（membership 核实），C1 下完整执行，与计划将其选为 C1 行为验证一致。
- 无已修复问题；遗留 Minor：None。

**Persisted Evidence**

- None required（Mode: none；组合构建失败/通过均可低成本用 cargo 命令复现）。

**Experience Candidates**

- Runbook 候选：feature 门控测试面的枚举与施加流程——`grep -l` 两类锚定 → `comm` 求交集 → 单行内层属性插入（`//!` 块后）→ C1/默认双面验证。本 Cycle 已端到端验证成功且 T4（cli 门控，44 文件）可直接复用。证据：本 Act Response 验证记录。
- Issue 候选：None（未发现本 change 范围外的实质缺陷）。

**未解决问题**

- None。

## Plan Review

- Review Result: pending
