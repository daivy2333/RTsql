# MS42 可组合的模块边界

对应路线：MS42。Gate 1 为 PASS，批准来自本次 goal 指令对 MS42 范围的授权，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed（场景草图与默认假设见 proposal.md）。
- 000：执行依据是 `iterations/000-feature-base-and-network/000-initial.md`。T1、T2 已完成，Act Response 为 reported，Review Result 为 accepted（2026-10-28，两条非阻塞 Minor 见该 Cycle Plan Review F1/F2）。
- 001：执行依据是 `iterations/001-crypto-optional/000-initial.md`。T3 已完成，Act Response 为 reported，Review Result 为 accepted（2026-10-28，两条非阻塞 Minor 见该 Cycle Plan Review F1/F2）。
- 002：执行依据是 `iterations/002-cli-optional/000-initial.md`。T4 已完成，Act Response 为 reported，Review Result 为 accepted（2026-10-28，三条非阻塞 Minor 见该 Cycle Plan Review F1/F2/F3）。
- 003：执行依据是 `iterations/003-combination-matrix-and-full-regression/000-initial.md`。T5、T6 已完成，Act Response 为 reported，Review Result 为 accepted（2026-10-28；三条陈述精度偏差 F1/F2/F3 已由 Plan Review 按代码事实修正 design，另记一条非阻塞 Minor F4，见该 Cycle Plan Review）。Map 内无剩余 Iteration。
- Persisted Evidence：none（全 change）。
- 无跳过项。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | features 表与 optional 依赖落地、死依赖处置；默认组合构建门与质量门退出码 0（改前改后等价） | done |
| T2 | 000 | T1 | network 可选化 + 测试面门；C1 组合构建门与验证集通过；默认组合协议测试通过 | done |
| T3 | 001 | 000 accepted | crypto 可选化 + 关闭态行为测试；C2 组合构建门与验证集通过；加密单元测试按组合门控 | done |
| T4 | 002 | 001 accepted | cli 可选化（bin required-features + 44 文件门）；C3/C4 组合构建门与验证集通过；默认组合 cli_test 通过 | done |
| T5 | 003 | 002 accepted（2026-10-28 已满足） | 五组合矩阵全绿（构建门 + 各验证集 + 双组合 clippy + fmt）；执行依据 `iterations/003-combination-matrix-and-full-regression/000-initial.md` | done |
| T6 | 003 | T5 | M19 分批全量（默认组合）每批 0 failed + 对账 0/0/0；design §5/§6 契约与实施后代码事实一致性核对；change 结构自检；执行依据 `iterations/003-combination-matrix-and-full-regression/000-initial.md` | done |

## 1. Feature 基座与 network 可选化（Iteration 000）

- [x] 1.1 T1：改前先跑默认组合基线命令（`cargo check --offline --lib --bins --tests`、`cargo fmt --check`、`cargo clippy --offline --all-targets -- -D warnings`）记录 GREEN。然后修改 `Cargo.toml`：按 design §2.1 增加 `[features]` 与 `[[bin]] required-features`，六个单一边界依赖转 optional，移除 `lru`、`anyhow`，`futures` 移入 `[dev-dependencies]`。首次构建不带 `--locked` 让 `Cargo.lock` 更新。改后同一组命令保持退出码 0。此阶段不加任何 cfg 门：非默认组合尚不可构建属预期，不作为本任务判据。
- [x] 1.2 T2：先记录 RED——`cargo check --offline --no-default-features --features cli,crypto --lib --tests` 因 network 模块未门控、tokio-util 未启用而失败（保留错误首行）。然后：`src/lib.rs` 的 `pub mod network` 加 `#[cfg(feature = "network")]`；按 design §2.3 规则给 network 锚定测试文件加门（含 `executor_test.rs` 函数级门）。GREEN = C1 组合构建门与验证集（design §2.5）通过，且默认组合 `cargo test --offline --test pg_protocol_test --test pg_integration_test --test network_server_test --test pg_messages_test --test network_protocol_test --test connection_limit_test` 全部通过。门控文件清单与计数记入 Act Response。

## 2. crypto 可选化（Iteration 001）

- [x] 2.1 T3：先写 `tests/crypto_off_build_test.rs`（文件级 `#![cfg(not(feature = "crypto"))]`，三个用例：合成 64B 加密头打开 → `IncompatibleHeader` 且消息含 `unknown feature flags`；明文库携带密钥打开与 0 字节携带密钥新建 → `InvalidKey` 且消息点名构建未包含加密能力、新建路径不写头；明文库建表/写入/查询/重开往返）。记录 RED——C2 组合尚不能编译（crypto 未门控、argon2/aes-gcm 未启用），测试无法运行。然后实施门控：`storage/mod.rs` 的 `mod crypto`、`file_header.rs` 的 `KNOWN_FLAGS_MASK`（crypto 关 = 0）与 decode 加密分支、`file_storage.rs` 加密分支与携带密钥拒绝（design §3 表）、file_header/file_storage 内加密正例单测加 `#[cfg(feature = "crypto")]`（argon2/aes-gcm 的 optional 声明已由 T1 完成）。GREEN = C2 组合构建门与验证集（含新测试目标）通过；默认组合 `cargo test --offline --test encryption_test --lib` 通过。

## 3. cli 可选化（Iteration 002）

- [x] 3.1 T4：先记录 RED——`cargo check --offline --no-default-features --features network,crypto --lib --tests` 因 cli 模块未门控、clap 未启用而失败。然后：`src/lib.rs` 的 `pub mod cli` 加 `#[cfg(feature = "cli")]`；`[[bin]]` 的 `required-features` 生效核验（无 cli 组合 `cargo build --offline --no-default-features` 不产出 bin）；按 design §2.3 规则给 bin 锚定测试文件加门（与 network 门组合成 `all(...)`）。GREEN = C3、C4 两组合构建门与验证集通过；默认组合 `cargo test --offline --test cli_test` 通过。

## 4. 组合矩阵与收尾全量（Iteration 003）

- [x] 4.1 T5：逐一执行 design §2.5 五组合的构建门与行为验证集（默认组合的行为验证即 4.2 的全量，此处只跑构建门）；补 `cargo fmt --check`、默认组合 deny clippy 与 core-only 组合 `cargo clippy --offline --no-default-features --lib --tests -- -D warnings`。任一组合失败即本任务失败。结果逐组合记录。
- [x] 4.2 T6：按 M19 + 等分批次规则在默认组合跑整个 change 的一次全量（`ls tests/*.rs | sort`，N 个文件，B = ceil(N/12)，前 B×12−N 批各 11、其余各 12，`--lib --bins` 单独成第一批；批间串行、批内 `--test-threads=1`；逐批记录计数与三项对账）。随后核对 design §5 层接口/状态归属与 §6 诊断约定同实施后的代码事实一致（发现偏差记入 Act Response 并修正 design 或按差异回报），完成 change 结构自检（tasks 状态、specs/design 与实现一致、Iteration/Cycle 齐全）。

## Iteration Plan

### Iteration 000：feature 基座与 network 可选化

- Tasks：T1、T2。
- Depends on：None。
- Stable result：`[features]` 表与依赖处置落地且默认组合行为零变化；C1（no-network）组合构建门与验证集通过。后续 Iteration 可直接在 features 表上追加门控。
- Verification boundary：默认组合构建/质量门与协议测试定向目标；C1 构建门 + `--lib` + 三核心目标。不跑全量。全量预算在 003，整个 change 一次。
- Diagnostic boundary：依赖解析与 lockfile 更新（T1）；network 门控残留引用与测试面门的枚举差异（T2）。crypto、cli 门控失败不属于本轮。
- Non-goals：crypto/cli 门控、组合矩阵收口、全量回归、层契约核对。

### Iteration 001：crypto 可选化与关闭态行为

- Tasks：T3。
- Depends on：000 accepted。
- Stable result：C2（no-crypto）组合成立：构建门与验证集通过，关闭态行为（加密头拒绝面、携带密钥拒绝面、明文全功能）有先行测试锚定；默认组合加密面回归通过。`database-encryption` 既有合同在默认组合零变化。
- Verification boundary：C2 构建门 + `--lib` + `crypto_off_build_test` + 两核心目标；默认组合 `--test encryption_test --lib`。不跑全量。
- Diagnostic boundary：crypto 门控粒度（cipher 分支、mask、KDF 校验分支）与关闭组合错误面；门控不涉 network/cli。
- Non-goals：加密功能变更、KDF 参数变更、`database-encryption` 域修改、`rand` 可选化、全量回归。

### Iteration 002：cli 可选化与组合测试面

- Tasks：T4。
- Depends on：001 accepted。
- Stable result：C3（no-cli）与 C4（core-only）组合成立；bin 经 `required-features` 进出构建图；全部 bin 锚定测试文件有门。三个 feature 的门控至此齐备。
- Verification boundary：C3/C4 构建门 + `--lib` + 核心目标；默认组合 `--test cli_test`。不跑全量。
- Diagnostic boundary：bin 缺席组合的 `env!` 展开与文件门遗漏；`all(...)` 组合门的正确性。
- Non-goals：CLI 功能与输出变更、install.sh/交叉脚本改动、全量回归。

### Iteration 003：组合矩阵与收尾全量

- Tasks：T5、T6。
- Depends on：002 accepted。
- Stable result：五组合矩阵全绿 + fmt/clippy 双组合门通过；默认组合按 M19 分批全量每批 0 failed 且对账 0/0/0；两份边界契约与代码事实核对一致；change 结构自检完成。
- Verification boundary：整个 change 的一次全量，只在本轮、只在默认组合（M19 + 等分批次规则 + 三项对账）；非默认组合只跑 §2.5 验证集。失败批只整批复核一次。
- Diagnostic boundary：按组合分列（构建失败归对应 feature 门；测试失败归目标域；全量失败按批定位）；契约核对偏差按「design 陈述 vs 代码事实」分列。
- Non-goals：第二次全量、非默认组合全量、新平台验证、诊断接口实现、依赖环消除。
- 执行依据（落地信息，2026-10-28）：`iterations/003-combination-matrix-and-full-regression/000-initial.md`，Plan Context 为 ready，Gate 2 PASS（无 BLOCKED、无 SKIPPED），Persisted Evidence 为 `none`。T5、T6 已完成（Act Response 见该 Cycle，状态 reported，2026-10-28）；Review Result 为 accepted（2026-10-28），本 change 的 Iteration Map 已全部完成，无剩余 Iteration。
- M19 全量预算（design §7 定形，写入本轮执行依据）：只在默认组合、整个 change 一次（本 Iteration 内执行，非默认组合不承诺全量绿）；`ls tests/*.rs | sort` 取 N 个文件，`B = ceil(N/12)`，前 `B×12−N` 批各 11 个、其余各 12 个，`--lib --bins` 单独成一批；批间串行（同一时刻只有一批在跑）、批内 `--test-threads=1`；SHALL NOT 用一次无目标过滤的全量调用跑完，SHALL NOT 用默认并行度跑任何全量批次；逐批记录 passed/failed/ignored 并做 missing/extra/duplicate 三项对账（均须为 0）；缺一批不得声明全量通过，失败批只整批复核一次。规划时基线（`163d1a9`）为 N = 158 → B = 14 → 10 批 × 11 + 4 批 × 12，`--lib --bins` 单独第 15 批 ⇒ 15 批 / 159 个测试目标 / 预期 160 条 `test result:` 行（执行时按实际文件集合重算）。
- 契约核对项（design §5/§6 逐项对照实施后代码事实）：§5.1 六层公共接口、§5.2 状态归属、§5.3 依赖方向与环清单、§6.1 现状出口清单与错误通道、§6.2 约定正文六条、§6.3 不落接口代码的理由。规划时已定位两处**计数类**偏差待 T6 裁定：§6.1「35 个直写出口 / wal/recovery 7」与实算 34 / 6 不符（`grep -c 'eprintln!' src/wal/recovery.rs` = 7 含文档注释处 1 处）；§5.2「12 字段」与 `pub struct Database` 实有 15 字段、R79 §4.1 表 13 行三者互不相等。计数类以代码事实为准修正 `design.md`，语义类按差异回报。R7/R8 为文档性验证（无自动测试），RTM 如实标注。

## 平衡审计

000 把 Cargo 基座与 network 门控合并：features 表单独不构成可验证的稳定结果（无门控消费它），network 是 src 侧最小的门控面，二者合成「C1 组合并默认组合零变化」这一独立可验收结果；二者的失败按依赖解析与门控残留分开记录。

001 单独承载 crypto：它是唯一有新行为语义（关闭态错误面）的 feature，先行测试与门控实现构成独立验收单元，与 network 门控分属不同故障域。

002 单独承载 cli：测试面最大（44 文件 + 双锚定组合门），且 C3/C4 两个组合的成立都以它为前提；与 crypto 的错误面工作分开排障。

003 承载矩阵收口与全量：矩阵要等三个门控齐备才有意义，全量是整个 change 的一次高开销预算，等代码稳定后执行；二者同属「收口验证」，T6 的契约核对依赖全部实施的最终形态。003 没有因编号靠后而稀释为纯文档轮——T5 是独立的矩阵验证成果。

四轮均有独立验证与停止点；Issue 承载 Iteration（design §8）在收到调度器转交时追加，保持同样审计标准。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 默认组合行为等价（含质量门、死依赖移除不可观察） | T1 | 改前/改后默认组合 `cargo check --lib --bins --tests` + fmt + deny clippy（GREEN→GREEN） | Covered |
| R2 network 可选组合（构建面、核心行为、默认协议面不变） | T2 | C1 组合构建门（改前 RED）+ 默认组合六协议测试目标 | Covered |
| R3 crypto 可选组合（加密头拒绝、携带密钥拒绝、明文全功能） | T3 | `tests/crypto_off_build_test.rs` 三用例（组合改前不可运行 = RED）+ 默认组合 `--test encryption_test --lib` | Covered |
| R4 cli 可选组合（bin 进出构建图、测试面门、默认 CLI 面不变） | T4 | C3/C4 构建门（改前 RED）+ 默认组合 `--test cli_test` | Covered |
| R5 任意所选组合构建与行为验证（矩阵 + 双组合 clippy + fmt） | T5 | design §2.5 五组合矩阵逐项执行 | Covered |
| R6 M19 分批全量一次通过（默认组合） | T6 | 等分批次分批 `cargo test`，逐批 0 failed + 对账 0/0/0 | Covered |
| R7 六层公共接口与状态归属契约记录且与代码事实一致 | T6 | 对照 design §5 与实施后代码逐项核对（文档性验证，无自动测试） | Covered |
| R8 诊断报告接口约定记录且与现状出口清单一致 | T6 | 对照 design §6 与实施后代码逐项核对（文档性验证，无自动测试） | Covered |

## Gate 2

PASS。无 BLOCKED。无 SKIPPED。计划批准来自本次 goal 指令（「开始计划ms42的工作，规划然后实施」）对 MS42 范围的授权。当前 Cycle 是 `iterations/000-feature-base-and-network/000-initial.md`，Plan Context 为 ready。
