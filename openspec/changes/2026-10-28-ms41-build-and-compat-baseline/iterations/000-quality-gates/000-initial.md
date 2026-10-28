# Iteration 000 / Cycle 000: 质量检查基线

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1、T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 的 R1、R2。ISS36 的当前修复，以及保持 SQL、存储、错误语义和运行形态。
- Excluded scope: T3–T6，musl 与 macOS，README，依赖升级，`rust-toolchain.toml`，全量回归。

**Objective**

当前 stable 工具链上，`cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 退出码都为 0。被改写的路径保持测量时的可观察行为。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms41-build-and-compat-baseline`，产品代码与主分支基点 `a69fe86bbf08a04b0d0cc62d9183da4ae8a5008b` 相同。工具链 rustc 1.99.1（58cf8f9c3 2026-10-13）、rustfmt 1.10.0-stable、clippy 0.1.99。无 `rust-toolchain.toml`。2026-10-28 测量：`cargo fmt --check` 退出码 1；`cargo clippy --offline --lib -- -D warnings` 退出码 101；允许下述 5 个 lint 后的 `cargo clippy --offline --all-targets` 退出码 0。ISS36 写于 2026-10-25、HEAD `e9cb9d3` 的文件名单不作工作清单。
- Current-State Evidence:
  - fmt：6 处 diff。`tests/hash_join_build_side_test.rs` 1 处，`tests/sort_merge_join_test.rs` 5 处。内容是空白和换行。
  - lib deny 共 15 条。`clippy::single_match` 在 `src/cli/lifecycle.rs:637`，restore 循环里 `execute_stage` 的 `match` 只有 `Err` 臂有语句，`Ok(_) => {}`。`clippy::let_underscore_future` 在 `src/executor/alter_table.rs:77`，`let _ = tokio::spawn(...)`。注释写明丢弃句柄是为了脱离任务，结果由随后的 `rx.await` 取得。`clippy::double_must_use` 共 9 条，span 都在 `#[async_trait]`：`Executor` 2 条、`Protocol` 2 条、`AsyncStorage` 5 条。诊断写明 must_use 来自该属性宏，返回的 future 已经是 must_use。`clippy::needless_range_loop` 在 `src/executor/sort_merge_join.rs` 的 `merge_product`，`left_index..left_end` 与 `right_index..right_end` 只用来索引同一向量。`clippy::explicit_counter_loop` 在 `BufferPool::flush_all` 与 `flush_all_except`。`written` 从 0 递增，`write_page` 失败时把当前值交给 `restore_dirty_from`；此时它等于失败页下标，也等于此前成功写入的页数。
  - 全目标剩余警告 6 条。`tests/dml_tombstone_abandon_window_test.rs:89` 的 `id as i64`、`:172` 的 `page_id as u32`。`tests/ddl_rewrite_consistency_test.rs:159` 的 `*id as i64`、`:223` 的 `page_id as u32`、`:408` 的 `id as i64`。lint 标明两端类型相同。`tests/sql_differential_test.rs:244` 的 `Vec::new()` 后立刻四次 `push` 四条 `CREATE TABLE`，随机数从后面的循环才消耗。
  - `~/.cargo/config` 弃用提示在上述命令里出现。允许已知 lint 的全目标 clippy 仍退出码 0。
- Code and Critical Path:
  - `src/cli/lifecycle.rs` restore 分支调用 `execute_stage`。失败走 `sql_failure_status`，成功不渲染并继续下一条。
  - `src/executor/alter_table.rs` ADD COLUMN 把 `alter_add_column` 放进 `tokio::spawn`，本语句只 `rx.await`。DropColumn 不在这条 lint 上。
  - `src/executor/executor_trait.rs` 的 `Executor`、`src/network/protocol.rs` 的 `Protocol`、`src/storage/async_storage.rs` 的 `AsyncStorage` 都由 `async_trait` 0.1.89 展开。
  - `merge_product` 对相同连接键的左右行做笛卡尔积，行来自 `left_rows[i].1` 与 `right_rows[j].1`。调用者是 sort-merge join 执行器。
  - `flush_all` 与 `flush_all_except` 在写入失败时从失败页起调用 `restore_dirty_from`。调用路径包括 checkpoint 与 catalog flush。已有失败观察在 `tests/write_failure_state_test.rs` 与 `tests/release_durability_precondition_test.rs`。
  - 测试 lint 只位于各自测试辅助函数，不改变被测 SQL。

**Implementation Guidance**

先做 T1，再做 T2。T2 的源码改动之后若 fmt 再次失败，只对 rustfmt 报告的差异执行 `cargo fmt`，不另做排版。不要运行 `cargo clippy --fix`。不要修改 `Cargo.toml`、`Cargo.lock` 或用户主目录的 Cargo 配置。

**Task Definitions**

### T1: fmt 检查退出码为 0

- Requirement/Scenario: R1 / 格式门通过
- Depends on: None
- Targets: `tests/hash_join_build_side_test.rs`、`tests/sort_merge_join_test.rs`
- Current behavior: `cargo fmt --check` 退出码 1，只报告这两个文件的空白与换行差异。
- Required behavior: `cargo fmt --check` 退出码 0。文件内容与 rustfmt 的写回结果一致。
- Preserve: 测试函数、断言和字面量。除 rustfmt 自身改动的空白外不改这些文件。
- Forbidden: 手写与 rustfmt 不同的排版。格式化产品代码里当前检查没有报告的文件。修改测试预期。
- First test: 先运行 `cargo fmt --check`。预期退出码 1，且 diff 仍只涉及上述两个文件。这是 RED。
- GREEN condition: 执行 `cargo fmt` 后，同一条 `cargo fmt --check` 退出码为 0。
- Verification: 上述两条命令的退出码。失败表示仍有未格式化差异，或 fmt 改到了检查范围之外的逻辑。
- Stop when: `cargo fmt` 产生非空白的语义 diff，或写回后检查仍非 0。

### T2: deny clippy 退出码为 0

- Requirement/Scenario: R2 / lint 门通过且行为保持
- Depends on: T1
- Targets:
  - `src/cli/lifecycle.rs` restore 循环中 `execute_stage` 的 `match`
  - `src/executor/alter_table.rs` ADD COLUMN 的 `tokio::spawn`
  - `src/executor/executor_trait.rs` 的 `Executor`
  - `src/network/protocol.rs` 的 `Protocol`
  - `src/storage/async_storage.rs` 的 `AsyncStorage`
  - `src/executor/sort_merge_join.rs::merge_product`
  - `src/storage/buffer_pool.rs::flush_all`
  - `src/storage/buffer_pool.rs::flush_all_except`
  - `tests/dml_tombstone_abandon_window_test.rs`
  - `tests/ddl_rewrite_consistency_test.rs`
  - `tests/sql_differential_test.rs::seed_statements`
- Current behavior: `cargo clippy --offline --all-targets -- -D warnings` 在 lib 的 15 条 lint 处失败，退出码 101，测试目标因此没有被这条 deny 命令扫完。允许这 5 个 lint 后的全目标检查退出码 0，只剩上述 6 条测试警告。
- Required behavior:
  - restore 的 `match` 改成 `if let Err(error) = execute_stage(...).await`。`Err` 臂仍调用原来的 `sql_failure_status`，参数不变。成功时继续循环。
  - `let _ = tokio::spawn(...)` 改成 `std::mem::drop(tokio::spawn(...))`。不 await `JoinHandle`。`rx.await` 仍是 ADD COLUMN 的结果来源，三条结果臂保持不变。
  - 在三个 trait 的 `#[async_trait]` 正上方各加 `#[allow(clippy::double_must_use)]`。trait 方法签名不变。
  - `merge_product` 用 `&left_rows[left_index..left_end]` 与 `&right_rows[right_index..right_end]` 的行引用调用 `build_output_row`，取每行的 `.1`。输出顺序与原来的双重下标循环相同。
  - 两处 flush 使用 `for (index, (page_id, page)) in to_write.iter().enumerate()`。`write_page` 失败时 `restore_dirty_from(&to_write, index)`。`index` 是失败页的位置。删除手写递增。成功页仍调用 `finish_write_in_flight(*page_id, true)`。
  - 删除 lint 点名的同类型转型：`id as i64`、`*id as i64`、`page_id as u32`。保留没有被警告的 `as u64`。
  - `seed_statements` 用包含原来四条 `CREATE TABLE` 字符串的 `vec![]` 作为初值。后续循环里的 `push` 文本和顺序不变。
  - `cargo clippy --offline --all-targets -- -D warnings` 退出码 0，`cargo fmt --check` 仍为 0。
- Preserve: SQL 行、ADD COLUMN 的成功与错误、flush 失败后从失败页起恢复脏位、差分种子的 SQL 序列、trait 的调用方式。
- Forbidden: `cargo clippy --fix`。升级或替换 `async-trait`。删除 `#[async_trait]`。await ADD COLUMN 的 `JoinHandle`。用从 0 开始的枚举下标去索引完整的 `left_rows` 或 `right_rows`。把失败页下标写成 `index + 1`。修改 `restore_dirty_from`。改 `Cargo.toml` 或 `Cargo.lock`。新增 `rust-toolchain.toml`。修改 `~/.cargo/config`。
- First test: 改写前先运行下面的命令，预期都是退出码 0、failed 为 0。这是重构前的 GREEN。
  - `cargo test --offline --test sort_merge_join_test`
  - `cargo test --offline --test alter_table_test`
  - `cargo test --offline --test write_failure_state_test`
  - `cargo test --offline --test release_durability_precondition_test`
  - `cargo test --offline --test dml_tombstone_abandon_window_test`
  - `cargo test --offline --test ddl_rewrite_consistency_test`
  - `cargo test --offline --test sql_differential_test`
  - `cargo test --offline --test cli_test restore`
  然后运行 `cargo clippy --offline --all-targets -- -D warnings`。预期退出码 101。这是 RED。定向测试使用默认并行度。
- GREEN condition: 同一组测试再次退出码 0、failed 为 0。`cargo fmt --check` 退出码 0。`cargo clippy --offline --all-targets -- -D warnings` 退出码 0。
- Verification: 以上命令的退出码和测试输出里的 failed 计数。clippy 失败表示还有 deny lint。测试失败表示改写改变了被这些测试观察的行为。
- Stop when: `#[allow]` 之后 `double_must_use` 仍出现在这三个 trait 上。某条 lint 只能通过改变 SQL 结果、存储字节、错误文本、退出码或任务是否被 await 来消除。改写前的行为测试不是 GREEN。改写后 deny 命令出现计划未列出的 lint，且该 lint 不能用同样的行为保持规则消除。

**Invariants**

- 不改变 SQL 结果、页格式、错误语义、CLI 退出码和 ADD COLUMN 的完成方式。
- `Cargo.lock` 保持不变。
- 全量回归不属于本 Cycle。定向测试可以用默认并行度。
- 不把 `~/.cargo/config` 的弃用提示当作项目 lint。

**Non-goals**

- T3–T6。
- 依赖升级、工具链固定、musl、macOS、README、发布归档和 CI。

**Acceptance**

- R1：`cargo fmt --check` 退出码 0。
- R2：`cargo clippy --offline --all-targets -- -D warnings` 退出码 0。
- T2 列出的 8 条行为测试在改写后退出码 0、failed 为 0。
- 上述代码位置之外没有功能改动。

**Verification**

- 格式门：`cargo fmt --check` 的退出码。
- lint 门：`cargo clippy --offline --all-targets -- -D warnings` 的退出码。
- restore、ADD COLUMN、sort-merge join、flush 失败、差分种子和两个页链测试：上面列出的 `cargo test --offline --test ...`。这些命令直接运行既有测试，测试观察 SQL、错误文本或页状态。
- 本 Cycle 不跑 M19 全量。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

命令、退出码和 failed 计数写入 Act Response 即可。这些检查可以在同一工作区重跑。不要求保存完整编译日志。

**Risks and Notes**

- 若 `#[allow(clippy::double_must_use)]` 不能盖住宏展开，按 T2 的停止条件返回，不在本 Cycle 改依赖。
- 环境里的 Cargo 配置弃用提示可以继续打印。
- 后续 Iteration 的任务在 `tasks.md`。本 Cycle 不实施它们。

## Act Response

- Status: 未开始

## Plan Review

- Review Result: pending
