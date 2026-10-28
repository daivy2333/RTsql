# Cargo feature 门控下测试面锚定文件的枚举与施加流程

- Status: active
- Last validated: 2026-10-28
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/` 集成测试套件（2026-10-28 实例：158 个 `tests/*.rs` 文件 + `--lib`/`--bins`）
- Source: change `2026-10-28-ms42-module-boundaries` Iteration 000 / Cycle 000 Act Response（`openspec/changes/archive/2026-10-28-ms42-module-boundaries/iterations/000-feature-base-and-network/000-initial.md`，2026-10-28，Runbook 候选原文见该文件「Experience Candidates」）、Iteration 002 / Cycle 000 Act Response（`.../iterations/002-cli-optional/000-initial.md`，cli 门控复用同一流程）、Iteration 003 / Cycle 000 Act Response（`.../iterations/003-combination-matrix-and-full-regression/000-initial.md`，组合矩阵与全量收口）

## 适用范围

- 需要给 Cargo feature 门控下的集成测试文件加门时：文件级 `#![cfg(...)]`（正向门，剥空文件）或函数级 `#[cfg(...)]`（只剥除个别用例）。
- 需要确定「某个关闭组合下哪些测试文件被剥空、哪些保留」时。
- 同源变体：加密域（crypto）的源侧模块门与关闭态行为测试——见「同源变体」段，与本流程同源，不另立 Runbook。
- 不适用：默认组合的全量回归执行（分批规则与批次对账见 `.agents/runbooks/m19-batched-full-regression.md`）；非默认组合的全量回归（关闭组合不承诺全量绿）；一次性试跑。

## 前置条件

- `Cargo.toml` 的 `[features]` 表已落地，可选依赖已标 `optional = true`（否则关闭组合的编译失败原因不是门控遗漏，RED 不可用）。
- 门控前的 RED 已观察并记录错误首行（关闭组合构建因未门控而失败）。
- 依赖可离线解析（命令带 `--offline`），lockfile 已就位（命令带 `--locked`）。
- 已确定每个 feature 的「关闭组合构造」与该组合的「核心验证目标池」（常可用目标：不 import 可选模块、不含 bin 宏、无该域正例的测试文件）。
- 明确每个 feature 的锚定判据（哪条 import / 哪个宏代表该 feature 依赖），锚定判据写在枚举命令里，不靠人工浏览。

## 操作步骤

### 1. 枚举两类锚定文件（执行时重跑）

两类锚定分别代表对 network 模块与 rtsql 二进制目标的依赖：

```bash
grep -l 'use rtsql::network::' tests/*.rs | sort > /tmp/net.txt      # network 依赖
grep -l 'env!("CARGO_BIN_EXE_rtsql")' tests/*.rs | sort > /tmp/bin.txt # bin 依赖
comm -12 /tmp/net.txt /tmp/bin.txt   # 双锚定
comm -23 /tmp/net.txt /tmp/bin.txt   # 仅 network
comm -13 /tmp/net.txt /tmp/bin.txt   # 仅 bin
```

`grep -l` 与 `comm` 只做枚举与计数，不做判定；判定全部落在 cargo 退出码与原生输出上。执行时必须重跑枚举，不沿用规划时的清单或计数。

### 2. 施加门

规则表（feature 名按实际改写）：

| 锚定集合 | 施加的门 | 形态 |
|---|---|---|
| 双锚定（network + bin） | `#![cfg(all(feature = "network", feature = "cli"))]` | 文件级，单行 |
| 仅 network | `#![cfg(feature = "network")]` | 文件级，单行 |
| 仅 bin | `#![cfg(feature = "cli")]` | 文件级，单行 |

- 插入位置：文件首个内层属性，即 `//!` 文档块之后、首个内容行之前；文档块与门之间留一个空行。
- 门以**单行**内层属性写入，不拆成多行。
- 门只加文件头/函数属性，**不改任何断言与用例本体**。
- 函数级 import 形态：若某文件只有个别测试函数体内 import 可选模块（如 `tests/executor_test.rs:495` 的 `use rtsql::network::Response;`），改用函数级 `#[cfg(feature = "network")]` 置于该函数的 `#[test]` 之上，文件其余用例不门。文件级门会误伤同文件内非该 feature 的用例。
- 门点唯一性：src/ 侧模块门（如 `pub mod network;`）每个可选模块只加一处；加门前先确认该模块在 src/ 内无其他引用点（grep `crate::<module>`）。

### 3. 双面验证

面 A（关闭组合）与面 B（默认组合）都要跑，两面缺一面不算通过：

```bash
# 面 A：关闭组合
cargo check  --offline --locked --no-default-features --features <关闭组合> --lib --bins --tests
cargo test   --offline --locked --no-default-features --features <关闭组合> --lib
cargo test   --offline --locked --no-default-features --features <关闭组合> --test <核心目标> ...
# core-only 组合补一条关闭组合的 deny clippy（剥空不得留告警）
cargo clippy --offline --locked --no-default-features --lib --tests -- -D warnings

# 面 B：默认组合
cargo test   --offline --locked --test <默认组合对应目标> ...
cargo fmt --check
cargo clippy --offline --locked --all-targets -- -D warnings
```

判定只看 cargo 退出码与原生 `test result:` 行；退出码非 0 即失败，不用任何额外判定层。

### 4. 组合矩阵与全量

五组合矩阵（C0 默认 / C1 no-network / C2 no-crypto / C3 no-cli / C4 core-only）的逐组合构建门、行为验证集与质量门，以及默认组合的 M19 分批全量——**不在本流程内重复规定**，按 `.agents/runbooks/m19-batched-full-regression.md` 执行。

### 同源变体：加密域（源侧门 + 关闭态专用测试文件）

crypto 的门点不在 `tests/` 锚定枚举面，而在 src/ 侧模块与分支，关闭态行为也不靠「剥空」而靠新增专用目标：

- 源侧门点：`src/storage/mod.rs` 的 `mod crypto;`、`src/storage/file_header.rs` 的 `KNOWN_FLAGS_MASK` 与 `decode` 加密分支、`src/storage/file_storage.rs` 的 `cipher` 字段与各加密分支（实例中 `src/` 内共 31 处 `#[cfg(feature = "crypto")]`）。
- 关闭态专用测试文件：新增 `tests/crypto_off_build_test.rs`，文件级**反向**门 `#![cfg(not(feature = "crypto"))]`——正向门剥空文件，反向门只在关闭组合存在，这是与锚定枚举流程同源、方向相反的一步。
- 关闭态用例的判定锚点：`StorageError::IncompatibleHeader`（消息含 `unknown feature flags`）、`StorageError::InvalidKey`（消息含 `encryption is not available in this build`，新建路径不写头字节）、明文库全功能往返。
- 专用目标在**默认组合下为 0 用例是正常形态**（不是失败信号）；在**关闭组合内必须存在并运行**，缺失即失败。
- 源侧门控的告警处理：关闭组合不得留 dead_code/unused 告警；用等效重构消除（随调用点一并 cfg），不得用 `#[allow]` 压告警。

## 验证

- **成功判据**：面 A（关闭组合构建门 + `--lib` + 核心目标 + 关闭组合 deny clippy）与面 B（默认组合对应目标 + fmt + deny clippy）各自 cargo 退出码全为 0；命令与输出取自实施时的同一批直接命令，不另建验证流程。
- **实际计数（MS42 实例，2026-10-28；历史现场）**：
  - 锚定集：network 锚定 **104** 文件、bin 锚定 **44** 文件、交集 **33**、仅 network **71**、仅 bin **11**。
  - 施加后的门分布：33 个 `#![cfg(all(feature = "network", feature = "cli"))]` + 70 个 `#![cfg(feature = "network")]` 文件门 + `tests/executor_test.rs:495` 1 个函数级门 = 104；另有 11 个 `#![cfg(feature = "cli")]` 文件门。
  - 计数是历史现场值。**执行时以 grep 重跑枚举为准**；计数漂移记入执行记录，不改规则。
- **面 A 实测结果（MS42 实例）**：
  - C1（no-network，`--no-default-features --features cli,crypto`）：构建门 exit 0；`--lib` **431 passed / 0 failed**；`--test storage_test --test parser_test --test executor_test` → executor_test **38 passed**（文件共 39 个测试函数 − 1 个被函数级门剥除）、parser 6、storage 21，exit 0。
  - C3（no-cli，`--features network,crypto`）：构建门 exit 0；`--lib` **412 passed / 0 failed**。
  - C4（core-only）：构建门 exit 0；`--lib` **390 passed / 0 failed**；`cargo clippy --offline --locked --no-default-features --lib --tests -- -D warnings` exit 0（关闭组合零告警）。
  - C2（no-crypto）：构建门 exit 0；`--lib` **419 passed / 0 failed**；关闭态专用目标 `crypto_off_build_test` 在 C2 内存在并通过（3 用例）。
- **面 B 实测结果（MS42 实例）**：默认组合六个协议目标 11+6+0+9+5+3 全 passed / 0 failed，exit 0（`network_server_test` 的 0 用例是该文件用例体整体处于块注释的既有形态，非门控成因）；`cargo fmt --check` 与 deny clippy exit 0。
- **组合矩阵与全量收口（MS42 Iteration 003）**：五组合构建门与四组合验证集、三条质量门退出码全 0；默认组合 M19 分批全量 15 批 / 159 个测试目标 / 160 条 `test result:` 行，合计 **2368 passed / 0 failed / 2 ignored**，覆盖对账 missing/extra/duplicate 全 0。批次规则与对账方法见 `.agents/runbooks/m19-batched-full-regression.md`。
- **计数归属口径**：Iteration 003 结果表在 C1/C2/C3/C4 的核心目标行把 storage 与 executor 的计数标签互换（该表写 C1「storage_test 38、parser_test 6、executor_test 21」）。三个计数值的集合与 Iteration 000/002 一致，仅标签归属不同；本 Runbook 采用经复核的归属——`executor_test.rs` 39 个测试函数（C1 剥除 1 → 38）、`storage_test.rs` 21 个、`parser_test.rs` 6 个（复核方式：按 `#[test]`/`#[tokio::test]` 逐文件计数）。C1 少 1 的目标必是 executor_test：其函数级 network 门是 C1 唯一的用例级剥除点。

## 失败处理

- **关闭组合构建失败、错误指向未启用依赖**（如 `error[E0433]: cannot find module or crate \`tokio_util\``）：说明 features 表已就位但门控未施加。按枚举清单补门后重跑该命令一次。
- **关闭组合构建失败、错误指向 `env!` 展开**：文件级 `#![cfg(...)]` 在条目与宏展开前剥除整个文件，因此 `env!("CARGO_BIN_EXE_rtsql")` 在 bin 缺席的组合不会展开。若仍报展开错误，说明该文件缺文件级门——**函数级门不够**（函数级门只剥除该函数，文件其余部分仍会展开宏）。
- **文件级门误伤同文件内非该 feature 的用例**（剥除后用例数少于预期，或默认组合下目标缺失）：改用函数级 `#[cfg(...)]`，只剥除该用例，文件其余用例保留。
- **关闭组合出现 dead_code/unused 告警**：随调用点一并 cfg 的等效重构消除；不得用 `#[allow]` 压告警。确实无法消除时停止回报。
- **关闭组合下某目标 0 用例**：先分清是「反向门专用目标」（`crypto_off_build_test` 在默认组合 0 用例，正常）还是「目标缺失」（关闭组合内该目标应存在并运行，缺失即失败）。
- **计数与规划值不一致**：以执行时 grep 重跑为准，把差异记入执行记录，不改规则、不改门控形态。
- **停止条件**：需要修改断言或用例本体、需要改 `Cargo.toml`（新增 feature、调整依赖可选性）、需要 `#[allow]` 才能消除的告警、或需要改产品代码才能让关闭组合通过——立即停止并按实际错误回报，不得改产品代码迁就门控。
- **不得用全量回归作为本流程的验证**：全量按 M19 分批串行执行，见 `.agents/runbooks/m19-batched-full-regression.md`。

## 回滚

- 本流程只新增测试文件头/函数属性行：回滚 = 撤销这些行（逐文件删门行，或 `git checkout -- tests/` 恢复该目录）。无数据迁移、无磁盘格式变更、无外部副作用。
- 同源变体（crypto）的回滚面是 src/ 内那些 `#[cfg(feature = "crypto")]` 行。`Cargo.toml` 的 `[features]` 表与 `optional` 声明是独立面：若回滚门控但保留 features 表，关闭组合仍会因依赖未启用而编译失败——两者需一并评估。
- 门控是编译期属性，回滚后默认组合行为回到原状（默认组合下门恒真）。

## 证据

- `openspec/changes/archive/2026-10-28-ms42-module-boundaries/iterations/000-feature-base-and-network/000-initial.md`——Act Response 的「门控清单与计数」「验证命令与结果」表（命令 1–11）、「Experience Candidates → Runbook 候选」原文；Plan Review 的门分布复算与锚定计数独立复跑 grep
- `openspec/changes/archive/2026-10-28-ms42-module-boundaries/iterations/001-crypto-optional/000-initial.md`——crypto 源侧门控与关闭态专用测试文件 `tests/crypto_off_build_test.rs` 的实际改动与验证结果
- `openspec/changes/archive/2026-10-28-ms42-module-boundaries/iterations/002-cli-optional/000-initial.md`——cli 门控复用同一枚举/施加流程，11 个 bin-only 文件门与清单完备性证据
- `openspec/changes/archive/2026-10-28-ms42-module-boundaries/iterations/003-combination-matrix-and-full-regression/000-initial.md`——五组合矩阵逐组合结果表、三条质量门、M19 分批全量批次表与三项对账
- `openspec/changes/archive/2026-10-28-ms42-module-boundaries/design.md` §2.2 门接缝清单、§2.3 测试面适配规则（确定性枚举规则的来源，随 change 归档）
- 2026-10-28 当次复核（本文档编写时，master，HEAD `50d9fb1`，工作区干净）：锚定计数复跑为 network 104 / bin 44 / 交集 33 / 仅 network 71 / 仅 bin 11；门分布复跑为 `all(network,cli)` 33 个、`network` 文件门 70 个、`cli` 文件门 11 个、`not(crypto)` 1 个，`tests/executor_test.rs` 的 network 门为函数级（无文件级门）。计数与归档产物一致。
- 相关配方：`.agents/runbooks/m19-batched-full-regression.md`（默认组合 M19 分批全量）、`.agents/runbooks/cli-subprocess-witnessing.md`（`env!("CARGO_BIN_EXE_rtsql")` 取二进制路径的既有约束）
