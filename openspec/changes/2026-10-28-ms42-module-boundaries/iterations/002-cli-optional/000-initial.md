# Iteration 002 / Cycle 000: cli 可选化与组合测试面

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS42 需求 R4（cli 可选组合）及 R1 默认组合等价不变量；proposal 默认假设 1–5；Iteration 000/001 的稳定结果（features 表、network 与 crypto 门控先例、cfg 剥空机制已验证）
- Excluded scope: 五组合矩阵收口与全量（T5/T6）、CLI 功能与输出变更、install.sh 与交叉脚本、其他 feature 的门、层契约核对

**Objective**

cli feature 门控落地后：C3（no-cli）与 C4（core-only）组合构建门与验证集通过；bin 经 `required-features` 进出构建图有直接可观察证据；默认组合 CLI 面由 `cli_test` 确认不变。三个 feature 门控至此齐备。

**Investigation Facts**

- Starting point: HEAD `8e9f2b0`（Iteration 001 accepted，Review 见 `../001-crypto-optional/000-initial.md`）。features 表已就绪——`cli = ["dep:clap", "dep:clap_complete", "dep:csv"]`，三依赖已 optional（T1）；network（Iteration 000）与 crypto（Iteration 001）门控已落地。C3/C4 组合当前不可编译：`src/lib.rs:3` 的 `pub mod cli;` 无条件编译，cli 模块树整体引用 clap。
- Current-State Evidence（本 Cycle 规划时逐点核实，基线 8e9f2b0）:
  - `src/lib.rs:3` 是 cli 的唯一门点；src 内 `crate::cli` 引用为零（src/cli/ 之外 grep 零命中）；`src/main.rs` 全部内容为 `rtsql::cli::run()`，bin 目标已在 T1 显式声明 `[[bin]] name = "rtsql" path = "src/main.rs" required-features = ["cli"]`——cli feature 关闭时 bin 目标被 Cargo 跳过、main.rs 不进入编译，main.rs 自身无需 cfg。
  - bin-only 测试文件 11 个（本 Review 枚举，= 44 bin 锚定 − 33 双锚定）：`composite_index_runtime_test`、`datetime_function_test`、`datetime_type_test`、`default_precheck_test`、`e2e_test`、`group_by_expr_test`、`import_input_surface_test`、`legacy_compat_test`、`query_memory_test`、`string_index_runtime_test`、`string_index_sql_test`。33 个双锚定文件在 Iteration 000 已带 `#![cfg(all(feature = "network", feature = "cli"))]` 门——cli 侧剥空已生效，本 Cycle 不动它们。
  - `cli_test.rs` 属 33 双锚定文件，默认组合照常编译运行；无任何测试 import `rtsql::cli`（Iteration 000 事实）。
  - `env!("CARGO_BIN_EXE_rtsql")` 的剥空机制已在 Iteration 000 验证（文件级 `#![cfg]` 在宏展开前剥除整个文件）。
  - 显式请求 bin 目标而 required-features 未满足时，cargo 以原生错误拒绝（含 `requires the features`/`cli` 语义）——这是 `required-features` 生效的直接可观察面，不需要检查 target 目录产物（旧构建可能残留同名二进制，产物存在性不是可靠判据）。
- Code and Critical Path: `src/lib.rs` 一行门 + 11 个测试文件各一行门；无其他代码路径变化（cli 模块内部零改动、Cargo.toml 零改动）。

**Implementation Guidance**

沿用 Iteration 000 的门施加形态：`#![cfg(feature = "cli")]` 插入 `//!` 文档块之后、首个内容行之前。11 个文件清单见 Current-State Evidence；执行时以 grep 重跑枚举为准（规则：`env!("CARGO_BIN_EXE_rtsql")` 命中且不含 `use rtsql::network::` 的文件），与清单差异记入 Act Response。

**Task Definitions**

### T4: cli feature 门控与 bin 测试面适配，C3/C4 组合成立

- Requirement/Scenario: R4（S4、S7 的 cli 侧；S1 默认组合等价不变量持续成立）
- Depends on: Iteration 001 accepted
- Targets: `src/lib.rs`（`pub mod cli;` 一行加 `#[cfg(feature = "cli")]`）；11 个 bin-only 测试文件（各 +1 行 cfg 头）
- Current behavior: cli 模块无条件编译；C3/C4 组合因 clap 未启用而编译失败；bin 目标在 feature 关闭时已被 `required-features` 声明约束（T1），但 cli 模块未门使 lib 无法构建；11 个 bin-only 测试文件在无 cli 组合因 `env!` 展开失败不可编译。
- Required behavior: cli 模块仅在 cli feature 开启时编译；11 个 bin-only 测试文件在关闭组合剥除为空目标；C3、C4 组合构建门与验证集通过；显式 `--bin rtsql` 构建在无 cli 组合被 cargo 原生错误拒绝；默认组合行为不变。
- Preserve: 33 个双锚定文件的门不动；`src/main.rs`、`src/cli/`、`Cargo.toml`、install.sh、`build-riscv64-musl.sh` 零改动；测试断言与用例本体零改动。
- Forbidden: CLI 子命令、输出或退出码变更；给 network/crypto 的门做任何改动；新增测试。
- First test: RED——`cargo check --offline --locked --no-default-features --features network,crypto --lib --tests` 失败（cli 模块引用未启用的 clap），保留错误首行。
- GREEN condition: 同命令退出码 0；C4 构建门通过；C3/C4 验证集通过；required-features 观察面成立；默认组合 `cli_test` 通过。
- Verification:
  1. RED 记录（命令与错误首行）。
  2. C3 构建门：`cargo check --offline --locked --no-default-features --features network,crypto --lib --tests` 退出码 0。
  3. C4 构建门：`cargo check --offline --locked --no-default-features --lib --tests` 退出码 0。
  4. C3 行为验证：`cargo test --offline --locked --no-default-features --features network,crypto --lib` 通过；`cargo test --offline --locked --no-default-features --features network,crypto --test storage_test --test parser_test --test executor_test` 通过。
  5. C4 行为验证：`cargo test --offline --locked --no-default-features --lib` 通过；`cargo test --offline --locked --no-default-features --test storage_test --test parser_test` 通过。
  6. required-features 观察面：`cargo build --offline --locked --no-default-features --bin rtsql` 非 0 退出，cargo 原生错误含「要求 cli feature」语义（如 `requires the features: cli`）；同时 `cargo check --offline --locked --lib --bins --tests`（默认组合）退出码 0 证明 cli 开启时 bin 正常。
  7. 默认组合回归：`cargo test --offline --locked --test cli_test` 全部通过（2 个长期 `#[ignore]` 用例按既有形态计 ignored）；`cargo check --offline --locked --lib --bins --tests`、`cargo fmt --check`、`cargo clippy --offline --locked --all-targets -- -D warnings` 退出码 0。
  8. 门控清单与计数（对 11 文件清单）记入 Act Response。
- Stop when: src 内出现 cli 模块的外部引用（grep `crate::cli` 复核，预期为零）；默认组合 cli_test 失败（默认组合回归，立即停止）；C3/C4 告警需 `#[allow]` 才能消除；门控文件计数出现规则外的形态（如 bin-only 文件同时含 network 引用——清单矛盾）。

**Invariants**

- 默认组合行为逐字节等价（CLI 全部子命令、退出码、输出）。
- 不新增跨层依赖环边；新增代码不引入 `eprintln!`/`println!` 直写出口。
- M19：不跑全量；定向命令默认并行度。无判定层，判定只看 cargo 退出码与原生输出（required-features 面以 cargo 原生错误为判据，不查产物存在性）。

**Non-goals**

五组合矩阵收口、全量回归、CLI 功能演进、脚本改动、install/发布路径、契约核对。

**Acceptance**

- R4 逐 Scenario：C3/C4 构建门与验证集（↔ delta spec「cli 可选组合」的「关闭 cli 的构建面」与「组合共存」的 core-only 部分）；required-features 观察面（↔「`rtsql` 二进制目标 SHALL NOT 构建」）；默认组合 `cli_test`（↔「默认组合 CLI 面不变」）；11 文件门 + 33 双锚定门共同构成 bin 锚定测试面的组合适配。
- 映射：proposal 需求 4、design §2.3 规则、tasks.md T4 交付依据。

**Verification**

逐 Scenario 最简直接判定（真实入口）：

| Scenario | 判定 |
|---|---|
| S4 no-cli 与 core-only | Verification 2–5（构建门 + `--lib` + 核心目标） |
| S4 required-words 面 | Verification 6（cargo 原生错误 + 默认组合 bin 正常） |
| S7 双锚定文件（cli 侧） | C3 构建门（33 个 `all(network, cli)` 门文件在 cli-off 剥空并编译通过） |
| S1 默认组合（触及面） | Verification 7（cli_test + 三质量命令） |
| S11 兼容 | 脚本零改动事实核对（无命令） |
| S2/S3/S5/S6/S8–S10 | 不属本 Cycle（S8 已在 Iteration 000 验证） |

**Gate 2 Readiness**

PASS（2026-10-28）。无 BLOCKED、无 SKIPPED。批准来自 goal 指令对 MS42 范围的授权（Iteration 001 accepted 后按 Map 展开本 Iteration）。RTM 对应行（R4）无 Missing；11 文件清单与门点唯一性已核实，无 TBD。

**Persisted Evidence**

- Mode: none

命令、退出码与输出行写入 Act Response 即可；组合构建可低成本用 cargo 命令复现，不满足 required 白名单。

**Risks and Notes**

- 11 文件清单以执行时 grep 重跑为准；差异记入 Act Response，不改变门控规则。
- C4 `--lib` 用例数将少于 C3（cli 模块内 `#[cfg(test)]` 单测剥除）；差额在 Act Response 记录即可，不构成对账门槛。
- `cargo build --bin rtsql` 在无 cli 组合的错误文案随 cargo 版本措辞有别，判定以「非 0 退出 + 点名 cli feature 语义」为准，不锚定整句原文。
- 本 Iteration 不做 core-only clippy（design §7 归 T5 矩阵）；若 C4 check 已见告警可提前记录但不阻塞本 Cycle。

## Act Response

- Status: reported
- Cycle: Iteration 002 / Cycle 000（initial）· Task T4
- 基线: HEAD `8fb2f42`，分支 `changes/2026-10-28-ms42-module-boundaries`

### 实际改动

产品代码 —— cli 唯一门点：

- `src/lib.rs:3` — `pub mod cli;` 前加 `#[cfg(feature = "cli")]`。cli 模块树仅在 cli feature 开启时编译。src 内 `crate::cli` 引用复核为 0，无其他调用点需改。

测试面 —— 11 个 bin-only 文件各 +1 行文件级门（`#![cfg(feature = "cli")]`）：

| 文件 | 门行位置 |
|---|---|
| `tests/composite_index_runtime_test.rs` | 3 |
| `tests/datetime_function_test.rs` | 12 |
| `tests/datetime_type_test.rs` | 12 |
| `tests/default_precheck_test.rs` | 9 |
| `tests/e2e_test.rs` | 12 |
| `tests/group_by_expr_test.rs` | 11 |
| `tests/import_input_surface_test.rs` | 7 |
| `tests/legacy_compat_test.rs` | 28 |
| `tests/query_memory_test.rs` | 5 |
| `tests/string_index_runtime_test.rs` | 3 |
| `tests/string_index_sql_test.rs` | 3 |

插入形态沿用 Iteration 000 既成门施加形态（`//!` 文档块之后、空行之后、首个内容行之前），与 33 个双锚定文件同构。测试断言与用例本体零改动。

### 门控清单与计数（Verification 8）

- 本 Cycle 新增 cli-only 文件级门：**11 个**，与 Plan Context 的 11 文件清单逐项一致。
- Iteration 000 既有的双锚定门 `#![cfg(all(feature = "network", feature = "cli"))]`：**33 个文件**，本 Cycle 未改动（diff 为证）。
- 枚举重跑：grep `CARGO_BIN_EXE_rtsql` 命中 **44 个**测试文件（66 处引用，且为 tests/ 下唯一 bin 环境变量名，无 `option_env!` 变体）；其中含 `use rtsql::network::` 者 **33 个**（＝既有双锚定门集合，两者完全同一集合），不含者 **11 个**（＝本 Cycle 加门集合）。与 Plan Context 清单**零差异**。
- 清单完备性的补充证据：C4 构建门 `--lib --tests` 编译**全部**集成测试目标并退出码 0 —— 若还有任何测试目标依赖 cli/network 而未加门，该命令必然失败。
- 零改动核对：`src/main.rs`、`src/cli/`、`Cargo.toml`、`install.sh`、`build-riscv64-musl.sh` 均不在本 Cycle diff 内（`git diff --name-only` 命中 0）。`Cargo.toml` 的 `cli` feature 与 `[[bin]] required-features = ["cli"]` 由 T1 已就位，本 Cycle 零改动。

### 与计划的偏差

无实质偏差。两处非实质执行细节记录：

1. Plan Context 的 11 文件清单与执行时 grep 重跑枚举完全一致，无需按风险说明记录差异。
2. C3/C4 `--lib` 用例数差额 412 − 390 = 22：来源是 network/crypto 模块单测在 core-only 组合下剥除，**非** cli 侧剥除（cli 在 C3/C4 两组合中均未开启）。按 Plan Context 风险说明，差额不构成对账门槛。

### 验证命令与实际结果

**RED（改动前，Plan Context First test）**

```
cargo check --offline --locked --no-default-features --features network,crypto --lib --tests
→ 退出码 101
错误首行: error[E0432]: unresolved import `clap`
定位:     src/cli/mod.rs:19:5
```

**GREEN**

| # | 命令 | 退出码 | 实际结果 |
|---|---|---|---|
| V2 | `cargo check --offline --locked --no-default-features --features network,crypto --lib --tests` | 0 | C3 构建门通过，Finished dev profile，无 crate 告警 |
| V3 | `cargo check --offline --locked --no-default-features --lib --tests` | 0 | C4 构建门通过，Finished dev profile，无 crate 告警 |
| V4a | `cargo test --offline --locked --no-default-features --features network,crypto --lib` | 0 | 412 passed; 0 failed; 0 ignored |
| V4b | `cargo test --offline --locked --no-default-features --features network,crypto --test storage_test --test parser_test --test executor_test` | 0 | executor_test 39 passed、parser_test 6 passed、storage_test 21 passed；0 failed |
| V5a | `cargo test --offline --locked --no-default-features --lib` | 0 | 390 passed; 0 failed; 0 ignored |
| V5b | `cargo test --offline --locked --no-default-features --test storage_test --test parser_test` | 0 | parser_test 6 passed、storage_test 21 passed；0 failed |
| V6a | `cargo build --offline --locked --no-default-features --bin rtsql` | 101 | cargo 原生错误：`error: target 'rtsql' in package 'rtsql' requires the features: 'cli'` |
| V6b | `cargo check --offline --locked --lib --bins --tests` | 0 | 默认组合 `--lib --bins --tests` 通过 |
| V7a | `cargo test --offline --locked --test cli_test` | 0 | 126 passed; 0 failed; **2 ignored**（长期 `#[ignore]` 用例，按既有形态计 ignored） |
| V7b | `cargo fmt --check` | 0 | 无输出 |
| V7c | `cargo clippy --offline --locked --all-targets -- -D warnings` | 0 | Finished dev profile，无告警 |

**Stop-when 复核**：`grep -rn "crate::cli" src/` → **0** 命中（src 内不存在 cli 模块的外部引用）。

**M19 遵守**：本 Cycle 未跑全量套件，全量预算留在 Iteration 003（默认组合一次）。上述均为 Cycle Verification 列出的定向命令，单目标验证用默认并行度。判定只看 cargo 退出码与原生输出（required-features 面以 cargo 原生错误为判据，未查 target 目录产物存在性），未新增脚本、封装命令、专用参数、判定层或任何 Hash/指纹/run-id 类身份证明机制。

### 自检发现

- Gate 3（task 级 review）：diff 仅 12 文件 12 行新增，全部落在 T4 目标清单内；无计划外修改；无新增告警（clippy `-D warnings` 通过）；无 `#[allow]` 压告警；无新增 `eprintln!`/`println!` 直写出口；测试不会因错误原因通过（C3/C4 构建门 + cargo 原生错误双面确认门确实生效，而非空转）。
- 禁改项复核：CLI 子命令/输出/退出码零变更（cli 模块内部零改动）；network/crypto 的门零改动（33 个双锚定门文件 diff 为空）；未新增测试；`src/main.rs`、`src/cli/`、`Cargo.toml`、两个脚本零改动。
- 未发现 Critical / Important 问题，无需修复；**无遗留 Minor 问题**。
- 未产生 change 范围外的实质缺陷。

### Persisted Evidence

None required（Plan Context Persisted Evidence = `none`；组合构建可由 cargo 命令低成本复现，不满足 required 白名单；未创建 `evidence/` 目录）。

### Experience Candidates

None —— 本 Cycle 未发现可重复或高风险的操作路径缺口，也未发现当前 change 范围外的实质缺陷；预期 RED 与普通测试通过不构成候选。

### 未解决问题

无。

## Plan Review

- Review Result: pending
