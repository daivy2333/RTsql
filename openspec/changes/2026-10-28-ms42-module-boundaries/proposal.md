# MS42 可组合的模块边界

对应路线：MS42。来源：I074（模块化路线 B 的 crate 内边界先行段）、I093（本批 feature 组合的兼容验证部分）、I095（仅实际触发的依赖适配；本 change 无被依赖真实阻碍的需求，未触发）。事实面来自 R79（`.agents/analysis/ms42-module-boundary-current-state.md`，2026-10-28，覆盖 revision 1f7dd6e；本分支基点 40e9ee8 相对它只新增两笔 docs 提交，事实未失效）。

Gate 1 为 PASS。批准来自本次 goal 指令对 MS42 范围的授权（「开始计划ms42的工作，规划然后实施……开始吧」），不是跳过。本文件不更新全局里程碑状态。

## Why

MS42 要求：依据实际调用明确核心、存储、事务、规划、执行和入口层的公共接口及状态归属；评估 CLI、网络和加密的可选依赖并形成所选组合的构建与行为验证；约定模块向后续统一诊断报告信息的接口。先在模块内验证边界，有实际收益再拆 crate。

当前事实（R79）：单 crate、无任何 `[features]` 表、src 内零 `cfg(feature)`；network、加密（`storage/crypto.rs`）、CLI（`src/cli/` + 单行 `main.rs`）三个可选化候选的依赖边界干净（tokio-util 仅 network/server.rs、clap/clap_complete/csv 仅 cli、argon2/aes-gcm 仅 storage 加密路径）；network 已是叶子模块（src 内零反向引用）。可选化不存在结构障碍，只存在测试锚定面适配（104 个测试文件 import `rtsql::network::`、44 个文件经 `CARGO_BIN_EXE_rtsql` 锚定 CLI）。

## What Changes

- 建立 `[features]` 表：`cli`、`network`、`crypto` 三个互相独立的 feature，`default = ["cli", "network", "crypto"]`；对应单一边界依赖（clap、clap_complete、csv / tokio-util / argon2、aes-gcm）转 optional 并由 feature 拉入。
- `src/lib.rs` 的 `pub mod network` 与 `pub mod cli` 加 `#[cfg(feature = …)]` 门；`[[bin]] rtsql` 显式声明并设 `required-features = ["cli"]`。
- 加密可选化：`storage/mod.rs` 的 `mod crypto`、`file_storage.rs` 加密分支、`file_header.rs` 的 `KNOWN_FLAGS_MASK`（crypto 关闭时为 0）按 feature 门控；crypto 关闭组合的行为合同见 delta spec（加密库按既有 unknown-flags 拒绝面拒绝；携带密钥的打开在锁之后以 `InvalidKey` 拒绝）。
- 测试锚定面适配：network 依赖的测试文件加 `#![cfg(feature = "network")]`（与 bin 锚定并存的文件用 `all(...)`；`executor_test.rs` 的函数内 import 用函数级门）；`CARGO_BIN_EXE_rtsql` 文件加 cli 门。不改写任何测试断言本体，不迁移 `to_response` 锚定词表。
- 依赖治理：移除 `lru`、`anyhow`（src/tests/benches/examples 全仓零引用）；`futures` 自运行时依赖移入 dev-dependencies（仅 `tests/concurrent_test.rs` 使用）。理由与备选见 design.md。
- 记录两份边界契约（design.md 承载，收尾时作为 project-model Mxx 候选移交 docs-maintainer）：六层公共接口与状态归属（含既有依赖环清单与处置）；模块向统一诊断报告信息的接口约定（面向 MS43/MS44）。
- 组合验证：所选五种组合（默认、三单关、全关）的构建门与按组合选定的行为验证；默认组合在整个 change 收尾时按 M19 分批串行跑一次全量。

## Capabilities

### New Capabilities

- `build-feature-combinations`（`specs/build-feature-combinations/spec.md`）：可选构建组合的行为合同——默认组合等价、cli/network/crypto 关闭组合的可用面与拒绝面。

### Modified Capabilities

无。默认组合的全部既有行为（SQL、存储格式、错误语义、退出码、CLI 面）不变，既有规格域零修改。

## Impact

- 代码：`Cargo.toml`（features、optional 依赖、`[[bin]]`、依赖增删）、`Cargo.lock`（随依赖增删更新）、`src/lib.rs`（两处模块门）、`src/storage/{mod.rs,file_header.rs,file_storage.rs,crypto.rs}`（加密门）、`src/main.rs`（经 `[[bin]]` 声明纳入 required-features，文件内容不变）、`tests/*.rs`（约 115 个文件加文件级或函数级 cfg 头，断言本体零改动）、新增 `tests/crypto_off_build_test.rs`（crypto 关闭组合行为，文件级 `#![cfg(not(feature = "crypto"))]`）。
- 行为：默认组合零变化。三个关闭组合是新增构建形态，其行为合同进 delta spec；其中唯一的运行期错误面新增是 crypto 关闭组合的携带密钥拒绝（`InvalidKey` + 新消息，CLI 退出码 5，复用既有密钥错误通道）。
- 文档：`design.md`（边界契约与裁定）；不修改 README（安装与默认构建路径不变）。
- 依赖：净减 `lru`、`anyhow` 两个运行时依赖；`futures` 移 dev；六个单一边界依赖转 optional。无版本变更、无新增依赖。
- 关联：MS42；I074、I093（feature 组合部分）、I095（未触发）；约束 M19（全量分批）、M20（本 change 按 goal 指令采用「Issue 登记 → 当前 change 新增 Iteration 承载」的变体，见 design.md）；MS41 本机质量基线沿用（见下）。
- 基线沿用：MS41 归档 change `2026-10-28-ms41-build-and-compat-baseline` 的 Linux 本机质量检查（fmt、deny clippy、分批全量）在覆盖未变化下直接沿用，本 change 不重跑改前基线；各任务用定向命令建立当次 GREEN。

## 需求

1. 默认组合（`default` features）构建产物与既有行为完全一致：lib、bin、测试全集照常编译运行，MS41 质量门（`cargo fmt --check`、`cargo clippy --offline --all-targets -- -D warnings`）保持退出码 0。
2. `network` 关闭组合：lib 构建通过且不含 network 模块，tokio-util 不进依赖图；组合内单元测试与既定核心行为测试目标通过；默认组合的协议面由既有协议测试锚定不变。
3. `crypto` 关闭组合：lib 构建通过，argon2/aes-gcm 不进依赖图；打开 `FLAG_ENCRYPTED` 头的库按既有 unknown-flags 拒绝面拒绝；携带密钥的打开（明文库或新建路径）在锁获取之后返回 `InvalidKey`，消息点名本构建未包含加密能力，CLI 退出码 5；明文库全功能可用。
4. `cli` 关闭组合：`rtsql` bin 不构建，clap/clap_complete/csv 不进依赖图；lib 与非 bin 锚定测试照常可用。
5. 三 feature 任意组合（含全关）lib 与测试目标集构建通过；每个非默认组合按 design.md 的验证集完成行为验证，`cargo test <组合> --lib` 通过。
6. 整个 change 收尾时在默认组合按 M19 分批串行跑一次全量：每批退出码 0 且 failed 为 0，批次表与三项对账（missing/extra/duplicate 为 0）记入 Act Response；非默认组合不承诺全量绿。
7. 六层公共接口与状态归属、诊断报告接口约定两份契约写入 `design.md`，且与实施后的代码事实一致（收尾核对）。

## 不做

- 拆 crate、固定 crate 数量、workspace 化（I074 路线 B：先 crate 内验证边界）。
- 解依赖环（环 1 database→pipeline→executor→database、环 2 storage↔transaction、database↔transaction）与词表归属迁移（`Value`/`ColumnType`/`PhysicalPlan`/`datetime` 留在现层，处置记入 design 契约）。
- 收紧 `pub` → `pub(crate)` 可见性（tests/ 深锚定 124+ 文件依赖现 pub 面）。
- 迁移 97 个文件的 `to_response` 断言词表到 `SqlOutcome` 锚定。
- `rand`、`serde`/`serde_json`、`dashmap`、`async-trait`、`thiserror`、`crc32fast`、`sqlparser`、`tokio` 的可选化（跨边界或非本批对象）。
- 异步协作改造、统一诊断系统的实现（MS43/MS44）、新 SQL、流式结果、存储格式变更、CI 建设。
- 无对象的依赖替换与全量依赖本地化（I095 未触发）。

## 场景草图（BDD 扫描结果）

前置状态统一：feature 表与门控已落地，五种组合可按命令构造。

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Happy | 默认组合 | `cargo build` + 定向测试 | bin 存在、行为与 MS41 基线一致 | 任何既有测试失败即默认组合回归，阻塞 |
| S2 | Happy | `--no-default-features --features cli,crypto` | 构建门 + `--lib` + 核心目标 | 编译通过、核心测试绿；network 模块不存在 | lib 内残留 network 引用则编译失败 |
| S3 | Happy | `--no-default-features --features cli,network` | 构建门 + `--lib` + `crypto_off_build_test` + 核心目标 | 明文库全功能；加密头拒绝 | 见 S5/S6 |
| S4 | Happy | `--no-default-features --features network,crypto` 与全关 | 构建门 + `--lib` + 核心目标 | bin 不构建、lib 绿 | bin 锚定测试漏门则编译失败 |
| S5 | Sad | crypto 关闭组合 | 打开 flags bit0 置位的 64B 合成头文件 | `IncompatibleHeader`（unknown feature flags 既有面） | 若走密钥分支说明 mask 门失效 |
| S6 | Sad | crypto 关闭组合 | 携带密钥打开明文库 / `new --key` | 锁之后 `InvalidKey`，消息含「构建未包含加密」语义，CLI exit 5；新建路径头未写入 | 错误先于锁发生即违反顺序守卫 |
| S7 | Edge | 双锚定文件（33 个既用 to_response 又用 CARGO_BIN_EXE） | 各单关组合构建 | `all(...)` 门正确剥空，两组合均编译 | 漏 all 组合则某组合编译失败 |
| S8 | Edge | `executor_test.rs`（函数内 import network） | no-network 组合构建+运行 | 该测试函数剥除、文件其余用例照常 | 文件级门误伤其余用例 |
| S9 | 组合 | crypto 关闭组合运行 `tests/encryption_test.rs` 等 crypto 域测试 | （不在验证集内） | 预期失败，不构成缺陷；验证集按 design 显式选定 | 误将其计入组合验收则计划错误 |
| S10 | 组合路径 | 收尾全量 × M19 | `ls tests/*.rs \| sort` 等分批次 | 每批 0 failed，对账 missing/extra/duplicate=0 | 缺一批不得声明全量通过 |
| S11 | 兼容 | 默认组合 install.sh / build-riscv64-musl.sh 路径 | 脚本按默认 features 构建 | 与既有一致（脚本不传 feature flag） | — |
| S12 | 错误/取消/超时 | — | — | 不适用：本 change 无运行期控制流新增（除 S6 错误面）；构建失败即任务失败 | — |

已接受缺口：S9（非默认组合不全量）、S11 的 macOS 分支（MS41 已裁定不再等待，本 change 不新增平台验证）。

## 回滚

全部改动位于 change 分支的 `Cargo.toml`、`Cargo.lock`、src 门控与测试文件门。回滚 = 放弃或还原本分支提交（`git revert` 或按用户指令弃用分支），主分支 master 无任何合并前的接触；无数据、格式或外部状态迁移，库文件与磁盘格式零涉及，回滚后构建即恢复变更前形态。

## 默认假设（goal 授权范围内补齐，未扩大范围）

1. 所选组合矩阵为五种：默认、no-network、no-crypto、no-cli、core-only；不再验证其他排列（两两缺失的构建面由全关组合覆盖）。
2. crypto 关闭组合的携带密钥拒绝复用 `StorageError::InvalidKey` 与 CLI 退出码 5（既有密钥错误通道），仅消息文本不同。
3. 测试锚定面用 cfg 门适配，不迁移断言词表（备选对比见 design.md §2.4）。
4. 非默认组合的行为验证集按 design.md §7 显式选定；全量只在默认组合、只在收尾 Iteration 跑一次（M19 + goal 约束）。
5. 实施中发现的实质缺陷按 goal 指令处理：登记 Issue（调度器派发 recorder）后由本 change 的 Iteration Map 新增 Iteration 承载解决；不预设空 Iteration。
