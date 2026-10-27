# 仓库既有的 fmt / clippy 门在当前工具链下不可满足（无工具链版本固定，格式与 lint 基线漂移）

- Status: open
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（「本仓在当前工具链下 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 于 HEAD 即失败 ⇒ README 列出的这两个门禁目前不可满足」），change `2026-10-25-ms37-defect-ledger-closure` Iteration 000 Cycle `iterations/000-index-leaf-entry-criterion/000-initial.md` 的 Act Response `Experience Candidates` 段；同 Cycle 的 Plan Review 在 `Evidence` 段独立核对了该基线
- Environment: Linux x86_64 / WSL2 6.18.40.1、cargo 1.99.1 (5f94df478 2026-08-27)、rustc 1.99.1 (58cf8f9c3 2026-10-13)、rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)、clippy 0.1.99 (58cf8f9c35 2026-10-13)；仓库 HEAD `e9cb9d3`；**两次测量的工作区状态不同**——基线测量时未提交改动为 `.agents/docs/tasks.md`、`src/storage/integrity.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`；第二次测量时在途 T3 实施已加入 `src/storage/buffer_pool.rs`、`src/storage/data_page.rs`、`src/storage/error.rs`、`src/storage/mod.rs`、`src/storage/page_format/slotted_page.rs` 的改动与新文件 `tests/data_page_bounded_read_test.rs`

## 缺陷描述

- 预期：README「构建与测试」段列出的 `cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --check` 是**项目门** ⇒ 在项目自己的工具链下 SHALL 可通过；任何 change 的收尾 SHALL 能声明「项目门通过」。
- 实际：项目**无 `rust-toolchain.toml`**（仓库根既无 `rust-toolchain.toml` 也无 `rust-toolchain`）⇒ 工具链版本不被固定 ⇒ 在当前工具链下两个门在 **HEAD 即失败**：
  - `cargo fmt --check` ⇒ **退出码 1**，**50 处 diff 分布在 9 个既有文件**（4 个源文件 + 5 个测试文件）：`src/storage/btree/btree.rs`（3 处）、`src/storage/data/mod.rs`（1 处）、`src/storage/data/table_manager.rs`（6 处）、`src/storage/integrity.rs:478`（1 处）、`tests/ddl_rewrite_consistency_test.rs`（14 处）、`tests/free_page_lifecycle_test.rs`（6 处）、`tests/index_page_ownership_test.rs`（6 处）、`tests/index_read_bounded_read_test.rs`（5 处）、`tests/release_path_bounded_read_test.rs`（8 处）。
  - `cargo clippy --all-targets -- -D warnings` ⇒ **退出码 101**，lib 编译失败于 **17 条 lint 错误**（`src/storage/async_storage.rs`、`src/network/protocol.rs`、`src/executor/executor_trait.rs`、`src/transaction/manager.rs`、`src/storage/buffer_pool.rs`、`src/storage/file_storage.rs`、`src/pipeline.rs`、`src/parser/planner/ddl_dml.rs`、`src/cli/lifecycle.rs`）。**deny 运行在 lib 即中止 ⇒ test 目标根本未被 lint** ⇒ deny 模式的完整错误数**下界**为 17（test 侧的 15 条 warning 在 `-D warnings` 下同样会成为错误 ⇒ 上界 32）。
  - `cargo clippy --all-targets`（不加 `-D warnings`，可跑完全部目标）⇒ **退出码 0**，**32 条 warning**（lib 17 + test 侧 15；test 侧落在 `dml_tombstone_abandon_window_test.rs`、`ddl_rewrite_consistency_test.rs`、`sql_differential_test.rs`、`secondary_index_equality_test.rs`）。其中 2 条是 `~/.cargo/config` 弃用告警（**环境项，非项目代码**）⇒ 项目代码自身的 lint 发现为 30 条。
- **漂移不是静态的——在途实施正在加大它**（本次登记期间复测）：MS37 Iteration 001 的 T3（数据页组有界读）实施期间，工作区新增了 `src/storage/buffer_pool.rs`、`src/storage/data_page.rs`、`src/storage/error.rs`、`src/storage/mod.rs`、`src/storage/page_format/slotted_page.rs` 的改动与新文件 `tests/data_page_bounded_read_test.rs` ⇒ 同两个门的结果**随之变化**：
  - `cargo fmt --check` ⇒ 退出码 1，**59 处 diff / 12 个文件**（较基线 **+9 处 / +3 个文件**：`src/storage/buffer_pool.rs` 3、`src/storage/data_page.rs` 3、`tests/data_page_bounded_read_test.rs` 3）。这三处在基线测量时**不在**名单内 ⇒ 增量来自在途改动本身。
  - `cargo clippy --all-targets -- -D warnings` ⇒ 退出码 101，**18 条 lint 错误**（较基线 **+1**）：`src/storage/page_format/slotted_page.rs:143` 的 `enclosing Some and ? operator are unneeded`（位于在途新增的 `get_slot_data_checked` 内）。
  - ⇒ **不固定工具链、也不做重排，则每个未跑 `cargo fmt` 的 change 都在加大基线漂移**；选项 (b) 的「一次性重排」若不在漂移停止增长之后进行，会立即再次失效。
- 位置：门禁声明处 —— `README.md:324-325` 与 `README.zh-CN.md:324-325`（「构建与测试」/「构建与测试」代码块内的 `cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --check`）；工具链固定缺失处 —— 仓库根（无 `rust-toolchain.toml` / `rust-toolchain`）；漂移文件 —— 基线 9 个文件、含在途增量 12 个文件（逐条位置见上）。
- 根因：**Inferred** —— 工具链版本漂移：项目未固定工具链版本，而仓库内的书写格式对应的是另一个 rustfmt 版本。**未确认**：具体是哪个 rustfmt 版本能让现有格式通过（未做版本二分），以及归档 change 记录中「clippy / fmt / validate 全 0 / PASS」的说法（该说法来自 Act Response 的转述，Recorder 未独立核对归档记录本身）。
- 触发条件：在**当前**工具链（rustfmt 1.10.0 / clippy 1.99.1）下执行项目门 ⇒ 必然失败；与本条缺陷无关的改动**无法**改变这一事实。

## 影响

- 当前：**任何 change 的收尾都无法声明「项目门通过」**——门在 HEAD 上即失败 ⇒ 该声明要么无法做出，要么只能以「基线条件」的口径记录。
- 本次 MS37 的实际处理：Act 与 Plan Review **均如实记录为基线条件**，**未在 change 内修复**——重排 9 个既有文件（基线 50 处 diff）违反 Surgical Changes（这些文件与 MS37 的缺陷主题无关）。
- 漂移**持续增长**：在途的 T3 实施已使 fmt 从 50 处 / 9 个文件增至 59 处 / 12 个文件、deny clippy 从 17 条增至 18 条 ⇒ 若不先让门可满足，「一次重排」会在下一次未跑 fmt 的改动后再次失效。
- 附带效应：`cargo clippy --all-targets -- -D warnings` 在 lib 即失败 ⇒ 该门当前**只覆盖 lib**，test 目标的 lint 状态在 deny 模式下**未被观测**（非 deny 模式下为 15 条）。
- 分类：**交付流程类缺陷，非产品代码缺陷**——被漂移的是既有文件的书写格式与 lint 形态，不改变任何运行时行为。

## 事件记录

- 2026-10-25（MS37 change Iteration 000 的 Act 阶段）：Act 实测 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 在 HEAD 即失败，判定为「与本 Cycle 的改动无关」的既有基线漂移，并作为 Issue 候选上报，处置建议为「由用户在收尾时裁定（固定门禁口径 or 另开一次范围明确的重排 change）」，本 Cycle 不做。
- 2026-10-25（MS37 change Iteration 000 的 Plan Review）：Review 独立核对该基线（fmt 标记 9 个文件；clippy 全仓 32 条 warning、本 Cycle 触碰的三个文件 0 条），并把「本 Cycle 触碰的文件自身无新增 finding」作为该 Cycle 的口径。
- 2026-10-25（本次登记）：Recorder 独立复跑三个命令（只读，不改文件），得到 fmt **退出码 1 / 50 处 diff / 9 个文件**、deny clippy **退出码 101 / 17 条 lib 错误**、非 deny clippy **退出码 0 / 32 条 warning** ⇒ 与 Act Response 的「17 条 lib 错误」和 Plan Review 的「32 条 warning」「9 个文件被标记」**逐项吻合**。⇒ 本条的证据来源由「子代理报告」升级为 **Recorder 亲跑 + 既有报告互相印证**。
- 2026-10-25（本次登记期间，第二次复测）：在 MS37 Iteration 001 的 T3 实施落到工作区后再次复测 ⇒ fmt 升至 **59 处 / 12 个文件**、deny clippy 升至 **18 条** ⇒ 记录「漂移随在途改动增长」这一事实，并定位到具体增量（fmt：`src/storage/buffer_pool.rs` / `src/storage/data_page.rs` / `tests/data_page_bounded_read_test.rs`；clippy：`src/storage/page_format/slotted_page.rs:143`）。**本条只登记，不修复**——这些文件由在途 Act 所有。
- 2026-10-25（工具链版本对照，针对处置候选 (a) 的前置条件「版本二分」；调度方取证 + Recorder 独立复核）：本机 rustup 装有**三个**工具链（`stable-x86_64-unknown-linux-gnu`〔active/default〕、`nightly-x86_64-unknown-linux-gnu`、`nightly-2026-02-25-x86_64-unknown-linux-gnu`）。
  - 调度方取证（直接采信）：执行 `cargo +nightly-2026-02-25 fmt --check`（较旧 nightly 的 rustfmt）⇒ **50 处 diff**，与同一次对照中 stable rustfmt 的结果**位置完全一致**（同一批 9 个文件：`src/storage/btree/btree.rs`、`src/storage/data/mod.rs`、`src/storage/data/table_manager.rs`、`src/storage/integrity.rs`、`tests/ddl_rewrite_consistency_test.rs`、`tests/free_page_lifecycle_test.rs`、`tests/index_page_ownership_test.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`）。
  - Recorder 独立复核（只读，**未修改任何仓库文件**；环境 Linux x86_64 / WSL2 6.18.40.1，HEAD `e9cb9d3`）：在**当前工作区**（含 MS37 在途改动）连跑 `cargo fmt --check` 与 `cargo +nightly-2026-02-25 fmt --check` ⇒ 两者均**退出码 1**、各 **50 处** `Diff in`，且**两份输出逐字节相同**（文件集合与行号位置一致；逐文件计数 `btree.rs` 3、`data/mod.rs` 1、`data/table_manager.rs` 6、`integrity.rs` 1、`ddl_rewrite_consistency_test.rs` 14、`free_page_lifecycle_test.rs` 6、`index_page_ownership_test.rs` 6、`index_read_bounded_read_test.rs` 5、`release_path_bounded_read_test.rs` 8）。参与对照的 rustfmt 版本：`1.10.0-stable (58cf8f9c35 2026-10-13)`（stable，cargo 1.99.1 / rustc 1.99.1）与 `1.8.0-nightly (859951e3c7 2026-02-24)`（nightly-2026-02-25，cargo 1.95.0-nightly 8cc0cb136 2026-02-17）。第三个工具链 `nightly`（cargo 1.101.0-nightly 3960fa69b 2026-10-12）**未能参与对照**：其 rustfmt 组件虽已安装，运行时报 `librustc_driver-8ddbd174ff1ac129.so: cannot open shared object file`（本机 nightly 工具链安装不完整；**环境项，非项目代码**；显式 `LD_LIBRARY_PATH` 亦未解决）。
  - 推论：**两个相隔约 8 个月的 rustfmt 版本给出逐字节相同的 diff** ⇒ 该漂移**不是** rustfmt 版本差异造成的，而是**代码确实未按 rustfmt 规则格式化**。
  - 对候选 (a) 的影响：**本机可得的两个可评估工具链上未发现可行版本** ⇒「固定到某个能通过的 rustfmt 版本」这条路在当前可得工具链上**走不通**；剩余可行处置为 (b)「以当前工具链一次性重排」或 (b')「一次性重排 + 固定当前工具链版本」。绝对命题「任何 rustfmt 版本都不可行」仍属**推断**（未做全版本枚举，见「证据」段未确认项 ①）。
  - 附带观测（当前工作区状态，Recorder 只读实测）：本次实测的 fmt diff 集合**与基线的 9 文件 / 50 处完全一致**——第二次测量记录的 +9 处 / +3 文件（`src/storage/buffer_pool.rs`、`src/storage/data_page.rs`、`tests/data_page_bounded_read_test.rs`）**已不在名单内** ⇒ 在途改动当前已是 rustfmt 干净，漂移集合回到基线的 9 文件 / 50 处。（**推断**：在途实施对其自身文件做过格式化；未核对执行者与执行时点。）此观测只更新**当前状态**，不改写第二次测量的历史记录。

## 处置

- 处置指针：**当前为空**——上层已在同一 change `2026-10-25-ms37-defect-ledger-closure` 内分配 Iteration 承载（由 Plan 写入具体 Iteration / Task）。
- 候选处置（**只记录，不实施，不裁定**）：
  - **(a) 固定工具链**：新增 `rust-toolchain.toml` 固定到能让现有格式通过的 rustfmt 版本（**该版本尚未确定**，需先做版本二分）。**2026-10-25 追加证据收紧**：本机可得的两个可评估工具链（stable rustfmt 1.10.0 / nightly-2026-02-25 rustfmt 1.8.0）经逐字节对照后**给出完全相同的 50 处 diff** ⇒ **在本机可得的工具链上未发现可行版本**（第三个工具链 `nightly` 因本机安装不完整未能评估）⇒ 该候选在当前环境下**实际不可行**，除非引入新的工具链版本再验。详见「事件记录」。
  - **(b) 一次性全仓重排**：以当前工具链重排上述漂移（基线 9 个文件 / 50 处；含在途增量则为 12 个文件 / 59 处）。规模为一个独立的、范围明确的改动。**须注意漂移在增长**（见上）⇒ 重排需与「此后每个 change 都跑 fmt / clippy」的口径同时落地，否则会立即再次失效。**2026-10-25 追加证据更新**：当前工作区的漂移集合已回到**基线的 9 个文件 / 50 处**（在途的 +3 文件已被格式化，见「事件记录」）⇒ (b) 的当前重排范围即基线 9 文件 / 50 处。
  - **(c) 维持现状**：在文档中记录该门在当前工具链下暂不可满足，并把收尾口径改为「触碰文件零新增 finding」。
- 需要 Plan / 用户判定的点：① 三个候选中选哪一个（或组合）；② 若选 (b)，重排 9 个既有文件与 MS37「缺陷清账」的**主题相关性**——把与主题无关的 9 个文件重排并入同一 change 违反 Surgical Changes ⇒ (b) 更适合**独立成 change**，而 (a) / (c) 更适合并入本 change；③ 收尾口径的措辞（「项目门通过」改为何种可验证的表述）。
- 与本 change 的其它承接物的关系：MS37 的收尾（Iteration 005 / T16 全量回归 + Maintainer 收尾）在裁定前**无法**声明 fmt / clippy 干净 ⇒ 该口径须由 Plan 写入收尾判据。

## 证据

- Issue 候选原文（不在本 Issue 复制）：`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/000-index-leaf-entry-criterion/000-initial.md` —— Act Response 的 `Experience Candidates` 段（Issue 候选：fmt / clippy 门禁不可满足）与同文件 §5「工具链基线事实」；同文件 `Plan Review` 段的 `Evidence` 段（fmt 9 个文件、clippy 32 条 warning 的独立核对）。
- 本次 Recorder 亲跑的只读复测（revision `e9cb9d3`；环境见 `Environment` 行；**各次测量的工作区状态不同**，见下）：
  - **基线测量**（工作区：`.agents/docs/tasks.md`、`src/storage/integrity.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs` 已改，MS37 change 目录未跟踪）：
    - `cargo fmt --check` ⇒ 退出码 **1**；`Diff in …` 行 50 行，分布在 **9** 个文件（逐条位置见「缺陷描述」段）。
    - `cargo clippy --all-targets -- -D warnings` ⇒ 退出码 **101**；`error: could not compile rtsql (lib) due to 17 previous errors` ⇒ **17** 条 lib lint 错误，分布在 **9** 个文件。
    - `cargo clippy --all-targets` ⇒ 退出码 **0**；**32** 条 warning（lib 17 + test 侧 15），其中 2 条为 `~/.cargo/config` 弃用告警。
  - **第二次测量**（在途 T3 实施落到工作区后）：fmt **59 处 / 12 个文件**、deny clippy **18 条**（新增 `src/storage/page_format/slotted_page.rs:143` 的 `enclosing Some and ? operator are unneeded`）。
  - **第三次测量（工具链版本对照；只读）**：当前工作区（含 MS37 在途改动，HEAD `e9cb9d3`）下 `cargo fmt --check` 与 `cargo +nightly-2026-02-25 fmt --check` 均退出码 **1**、各 **50 处** `Diff in`，且**两份输出逐字节相同**（文件集合与行号位置一致）⇒ 漂移非 rustfmt 版本差异所致；参与对照的 rustfmt 版本为 `1.10.0-stable (58cf8f9c35 2026-10-13)` 与 `1.8.0-nightly (859951e3c7 2026-02-24)`；第三个工具链 `nightly`（cargo 1.101.0-nightly 3960fa69b 2026-10-12）的 rustfmt 因本机缺 `librustc_driver-8ddbd174ff1ac129.so` 无法运行 ⇒ 未能参与对照。
  - 工具链版本：`cargo 1.99.1 (5f94df478 2026-08-27)`、`rustc 1.99.1 (58cf8f9c3 2026-10-13)`、`rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)`、`clippy 0.1.99 (58cf8f9c35 2026-10-13)`。
  - 门禁声明处：`README.md:324-325`、`README.zh-CN.md:324-325`；工具链固定缺失：仓库根无 `rust-toolchain.toml` / `rust-toolchain`。
  - **Recorder 的复测本身未修改任何仓库文件**（复测只读）；各次测量之间的差异来自**在途 Act 的实施改动**，不是复测造成的。
- **未确认项**：① 能让现有格式通过的 rustfmt 版本（(a) 的前提）——未做版本二分；**2026-10-25 追加部分二分结果**：本机可得的 stable rustfmt 1.10.0 与 nightly-2026-02-25 rustfmt 1.8.0 输出**逐字节相同** ⇒ 二者**均不可行**，结论限于「本机可得的工具链上未发现可行版本」；第三个工具链 `nightly` 因本机安装不完整（缺 `librustc_driver-*.so`）未能评估；全版本枚举仍未做；② 归档 change 记录中「clippy / fmt / validate 全 0 / PASS」的原话与其对应 revision——来自 Act Response 的转述，Recorder 未独立核对；③ `~/.cargo/config` 弃用告警是否应计入项目门（环境项）——随 (c) 的口径一并裁定；④ 「漂移增长归因于在途 T3 改动」依据的是：基线测量时 `src/storage/buffer_pool.rs` 与 `src/storage/data_page.rs` **未被修改**（与 HEAD 一致）且**不在** fmt 名单内 ⇒ 其现在的 3 处 diff 必来自在途改动；该归因对**新增测试文件**同理（基线时不存在）。
