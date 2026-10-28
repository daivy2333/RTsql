# Iteration 002 / Cycle 000: 分批全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T6
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 的 R6。收尾时重新执行 R1、R2 的两条命令。M19 的分批约束。001 已通过的 gnu 冒烟、musl 冒烟，以及 macOS 未完成记录，都保持原样。
- Excluded scope: 产品代码、测试代码、`Cargo.toml`、`Cargo.lock`、两份 README。musl 测试套件。macOS 补跑。第二次全量。依赖升级、`rust-toolchain.toml`、`.cargo/config`、`install.sh`、`dist/`、CI。把 bench、example 或 doctest 加进通过条件。判定脚本。

**Objective**

在 gnu 宿主上按下面的分批规则跑完一次全量。每一批退出码为 0，且该批每一行 `test result:` 的 failed 为 0。一批都不能少。全部批次通过后，`cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 的退出码都为 0。

**Investigation Facts**

- Starting point:
  - 工作树 `/home/daivy/projects/RTsql-changes/2026-10-28-ms41-build-and-compat-baseline`，分支 `changes/2026-10-28-ms41-build-and-compat-baseline`。展开本 Cycle 时 HEAD 为 `c2ea652c09ff21f86d96181558bd1ad108feef7e`，工作区干净。
  - 001 的 Act Response 为 reported，提交 `a74c0b07c0b1ef672c5a33e1e47fc242f80ee9d6`。001 的 Review Result 为 accepted，提交 `c2ea652c09ff21f86d96181558bd1ad108feef7e`。Next Iteration 指向本轮，当时目录未创建。
  - 001 的产品 diff 只有 `README.md` 与 `README.zh-CN.md`。没有改 `src/`、`tests/`、`Cargo.toml` 或 `Cargo.lock`。
  - gnu：`cargo build --locked --offline --release --bin rtsql` 退出码 0。`target/release/rtsql` 对含 `/` 的临时库完成 `new`、`CREATE TABLE t (id INT)`、`INSERT INTO t (id) VALUES (1)`、`SELECT id FROM t`，退出码都为 0。`SELECT` stdout 为 `{"columns":["id"],"rows":[[1]]}`。
  - musl：`rustup target add x86_64-unknown-linux-musl` 退出码 0。带链接器 `/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc`、`--target x86_64-unknown-linux-musl` 和 `+crt-static` 的 `cargo rustc` 退出码 0。`ldd` 原句是 `statically linked`。musl 二进制对另一个临时库完成同一组 `new` 和三条 SQL，退出码都为 0，`SELECT` stdout 相同。
  - macOS 未完成。`uname -srm` 为 `Linux 6.18.40.1-microsoft-standard-WSL2 x86_64`。已安装 target 没有名称含 `apple` 或 `darwin` 的项。`x86_64-apple-darwin-gcc`、`o64-clang`、`x86_64-apple-darwin20.4-clang` 的 `command -v` 退出码都是 1。本 Cycle 不补跑 macOS。
  - 000 的 `cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 退出码都为 0。001 没有改 Rust 源码，Review 沿用了这两条。它们不代替本 Cycle 的收尾检查。规划阶段不重跑这两条，也不跑全量。
- Current-State Evidence:
  - 2026-10-28 在本工作树读取：无 `rust-toolchain.toml`，无仓库内 `.cargo/`。`rustc 1.99.1 (58cf8f9c3 2026-10-13)`，`cargo 1.99.1 (5f94df478 2026-08-27)`。`cargo --version` 会打印 `~/.cargo/config` 已弃用的警告。不修改该文件，这个警告不是失败。
  - `pkg-config --exists sqlite3` 成功。这只说明本机有 sqlite 开发库。它不是通过条件，也不授权安装或更换依赖。
  - `Cargo.toml` 没有 `[lib]`、`[[bin]]`、`[[test]]`、`[[example]]`。库入口是 `src/lib.rs`，二进制入口是 `src/main.rs` 的 `main`，包名 `rtsql`。没有 `src/bin/`。因此 `--lib` 测的是这个库，`--bins` 测的是这个二进制。集成测试目标名就是 `tests/*.rs` 的文件主名。
  - 有 12 个 `[[bench]]`，`harness = false`。它们不进入本 Cycle 的 `cargo test`。
  - `tests/support/` 与 `tests/fixtures/` 不是 `tests/*.rs`。不要把它们当成 `--test` 目标。
  - 规划时 `ls tests/*.rs | wc -l` 为 157。主名都是 `[a-z0-9_]`。按 Implementation Guidance 的截断除法，`B=$(( (157 + 11) / 12 ))` 为 14，`K=$(( 14 * 12 - 157 ))` 为 11。若执行时文件数仍是 157，文件批是前 11 批各 11 个、后 3 批各 12 个，再加上最前面的 `--lib --bins`，共 15 次调用。157、14、15 以及历史上的 passed 总数都不是通过门槛。执行时重新数。
  - M19 要求全量按 `--lib` / `--bins` 与各集成测试文件分批，批间串行，每批 `--test-threads=1`。禁止一次无目标过滤的全量调用，也禁止用默认并行度跑全量批次。定向测试的默认并行度不适用于本 Cycle。
- Code and Critical Path:
  - 本任务不改产品符号。入口是在工作树根目录执行的 `cargo test`。
  - 第一次调用只带 `--lib --bins`。其后每次调用只带该批的若干 `--test <主名>`。
  - 主名来自执行当时的 `ls tests/*.rs | sort`：去掉目录和 `.rs`。清单按这个排序整段切片，不重排，不交叉。
  - 库与二进制的测试代码在 `src/` 内。集成测试的 crate 根是对应的 `tests/<主名>.rs`。`tests/support/` 只被测试引用，不是独立目标。

**Implementation Guidance**

在工作树根目录执行。当前 shell 若已设置 `CARGO_TARGET_DIR`，先取消它。001 里这个变量曾把构建产物写到工作树外。不设置 `RTSQL_HOME`。不传 `--target`。

先取清单，再算批次，然后才跑第一批。文件数 `N` 是 `ls tests/*.rs | sort` 的行数。用截断除法：`B=$(( (N + 11) / 12 ))`，`K=$(( B * 12 - N ))`。前 `K` 个文件批各 11 个文件，其后 `B - K` 个文件批各 12 个文件。`K` 为 0 时没有 11 个文件的批，全部文件批都是 12 个。`K > B`，或任一文件批的大小不是 11 或 12 时，停止，不要换成别的切法，也不要开始 `cargo test`。

调用顺序固定。第一次是 `--lib --bins`。文件批接在后面，按清单顺序。同一时刻只跑一批：上一条命令结束并记完退出码之后，才开始下一条。不要后台启动，不要并行开多条 `cargo test`。

某批里前面的目标失败时，cargo 不会再跑该批后面的目标。所以失败后的那一次复核必须重跑整批，不能只重跑已经打印失败的用例，也不能把没跑到的目标记成通过。

不要用管道替换 cargo 的退出码。`test result:` 行从该次 cargo 输出里读。可以在已经拿到 cargo 退出码之后，再用一步 `grep` 帮助摘行，但摘行命令的退出码不作批次结果。

第一批和全部文件批都按通过条件通过之后，再跑 fmt，然后跑 clippy。还有批没跑完，或有一批未通过时，不跑这两条。

**Task Definitions**

### T6: gnu 宿主上的一次分批全量，随后两条门的退出码为 0

- Requirement/Scenario: R6 / 全量回归。收尾命令对应 R1、R2，作为本 Iteration 的收尾条件再跑一次。
- Depends on: None。001 已经 accepted。
- Targets: `src/lib.rs` 所代表的库测试，`src/main.rs::main` 所在二进制的测试，以及执行当时 `tests/*.rs` 的每个集成测试目标。不修改这些文件。
- Current behavior: 本分支还没有按本 Cycle 的命令跑过这组分批全量。000 的 fmt 与 clippy 曾退出码 0，001 没有重跑。macOS 仍是未完成。musl 只跑过产品二进制冒烟，没有跑过 musl 测试套件。
- Required behavior:
  - 执行前取消已设置的 `CARGO_TARGET_DIR`。工作目录是本工作树根目录。
  - 用执行当时的 `ls tests/*.rs | sort` 按 Implementation Guidance 切批。每个文件的主名恰好进入一个文件批。
  - 第一批命令是 `cargo test --offline --lib --bins -- --test-threads=1`。
  - 每个文件批的命令是 `cargo test --offline --test <主名> --test <主名> ... -- --test-threads=1`。参数顺序与清单切片顺序相同。
  - 每一批的 cargo 退出码为 0。该批至少有一行 `test result:`。每一行里的 failed 都是 0。
  - 跑完后再次 `ls tests/*.rs | sort`。输出与开跑前那次清单逐行相同。
  - 然后 `cargo fmt --check` 退出码 0。再然后 `cargo clippy --offline --all-targets -- -D warnings` 退出码 0。
  - Act Response 记下每一批的命令、cargo 退出码，以及该批全部 `test result:` 原行。开跑前的文件数和切批结果也记下。历史 passed 总数不作为门槛。
- Preserve: `--offline` 与 `-- --test-threads=1`。批间串行。SQL 结果、存储字节、错误语义、CLI 行为和 001 的 README 保持不动。macOS 保持未完成。musl 冒烟的结论保持为已在本机跑过产品二进制。
- Forbidden:
  - 一次不带目标过滤的 `cargo test`，包括 `cargo test --offline`、`cargo test --offline -- --test-threads=1`，以及任何把全部集成测试放进同一次调用的命令。
  - 全量批次省略 `--test-threads=1`，或用默认并行度跑任何一批。
  - 同时启动两批。
  - `--target`、`--doc`、`--examples`、`--benches`、`--all-targets`、`--release` 出现在这些 `cargo test` 里。clippy 的 `--all-targets` 只属于收尾那一条 clippy，不属于测试批。
  - `cargo test --target x86_64-unknown-linux-musl`，以及任何 musl 测试套件。
  - 补跑 macOS，或改两份 README。
  - 修改 `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`。运行 `cargo fmt` 写回。为了让门通过而改 lint 或断言。
  - 新增脚本、封装命令或用脚本退出码代替 cargo 退出码。把文件清单或 passed 总数写成固定门槛。
  - `cargo update`、改依赖、安装系统包、修改 `~/.cargo/config`。
- First test: 本任务在观察结果之前不改产品文件。先行观察就是第一批 `cargo test --offline --lib --bins -- --test-threads=1`。规划阶段没有跑它，所以这里没有预先看到的 GREEN 或 RED。预期该批退出码 0，且每一行 `test result:` 的 failed 为 0。
- GREEN condition: Required behavior 全部成立，且 `git status --short` 没有 `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`、`README.md`、`README.zh-CN.md` 的改动。
- Verification: 每一批的 cargo 退出码和 `test result:` 原行。前后两次 `ls tests/*.rs | sort`。收尾两条命令的退出码。`git status --short` 对上述路径为空。任一批没跑、某批没有 `test result:` 行、任一行 failed 不是 0，或 cargo 退出码不是 0，都表示全量没有通过。
- Stop when:
  - 切批不满足 `K <= B`，或任一文件批不是 11 或 12 个文件：停止，不开始 `cargo test`，回到计划。
  - 一批的 cargo 退出码不是 0，或没有 `test result:` 行，或任一行 failed 不是 0：用同一条命令把该批整批再跑一次。复核通过则在 Act Response 写明首跑失败，并继续下一批。复核仍不满足：停止。不跑后续批次，不跑 fmt 和 clippy，不改产品文件，回到计划。记录该批命令、两次退出码、两次的 `test result:` 原行，以及输出里能看到的失败测试名。
  - `--offline` 因缓存缺失不能编译：停止。不拉取依赖，不改锁文件，回到计划。
  - 结束时的 `ls tests/*.rs | sort` 与开跑前不一致，或某个主名没有恰好出现一次：停止，不改清单再跑一轮，回到计划。
  - fmt 或 clippy 退出码不是 0：停止。不执行 `cargo fmt` 写回，不改代码，回到计划。
  - 产品路径出现未提交改动：停止，不把这些改动当成通过，回到计划。

**Invariants**

- 全量结论只来自上述分批。缺一批就不能写成通过。
- 不改变 SQL 结果、页格式、错误语义、CLI 退出码、依赖锁和 001 已经写好的 README。
- 不把 musl 产品二进制冒烟写成 musl 测试套件已通过。
- 不把 macOS 未完成写成已验证。
- `~/.cargo/config` 的弃用警告可以出现，不据此改用户目录，也不把它当成项目失败。
- 已通过的批次不因为后面某一批失败而重跑。

**Non-goals**

- 第二次全量，musl 测试套件，macOS 构建或冒烟，依赖升级，发布归档，CI，`install.sh`。
- 把 bench 跑进本轮通过条件。
- 用历史 passed 总数、文件数 157 或调用次数 15 作为门槛。
- 在本 Cycle 里修改产品代码来修测试失败。失败在复核后仍在时，回到计划。

**Acceptance**

- R6：开跑前的每个 `tests/*.rs` 恰好出现在一个文件批里。`--lib --bins` 有单独的第一批。每一批 cargo 退出码为 0，该批每一行 `test result:` 的 failed 为 0，且该批至少有一行。结束时文件清单与开跑前一致。
- R1、R2 的收尾：全部测试批通过后，`cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 退出码都为 0。
- 本 Cycle 不改产品文件。macOS 仍为未完成。没有 musl 测试套件结果被写成通过。

**Verification**

- 全量：真实执行上面的 `cargo test --offline` 命令。通过依据是每条命令的退出码，以及该次输出中每一行 `test result:`。这是 gnu 宿主上的测试入口，不是另写的判定程序。
- 覆盖：开跑前和结束后的 `ls tests/*.rs | sort` 逐行相同。`--test` 主名与第一次清单一一对应。
- 收尾：`cargo fmt --check`，然后 `cargo clippy --offline --all-targets -- -D warnings`。通过依据是退出码 0。
- 范围：`git status --short` 不显示产品路径改动。
- 不跑无过滤全量，不用默认测试并行度，不跑 musl 测试，不补跑 macOS。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过。无 BLOCKED。无 SKIPPED。批准来自本次目标对展开 Iteration 002 的授权，不是跳过。

**Persisted Evidence**

- Mode: none

每一批的退出码和 `test result:` 原行写入 Act Response 即可。这些行就是通过或失败的可观察结果。不保存完整测试输出，也不另建 evidence 目录。

**Risks and Notes**

- 规划时文件数是 157，公式得到 14 个文件批。执行时文件数若变化，按当时的 `N` 重算。重算不满足 11 或 12 的切批条件时，停止条件已经要求回到计划。
- `test result:` 行里的 ignored 可以不是 0。通过条件只看 failed 是不是 0，以及 cargo 退出码。不把某一历史 ignored 数量写成门槛。
- `--lib --bins` 一次调用会为库和二进制各写出 `test result:` 行。文件批通常一个 `--test` 一行。行数是否等于某个旧总数，不是通过条件。
- 失败批只整批复核一次。不先单独重跑失败用例。复核通过后继续，不重跑已经通过的批次。
- 弃用警告可以继续打印。sqlite 开发库在规划时能被 `pkg-config` 看到；测试仍因环境失败时，按停止条件回到计划，不在本 Cycle 安装包或改依赖。

## Act Response

- Status: reported

**Implemented**

没有修改 `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`、`README.md` 或 `README.zh-CN.md`。

T6 在工作树根目录执行。开始测试前取消了 `CARGO_TARGET_DIR`，产物在工作树的 `target/`。没有设置 `RTSQL_HOME`，没有传 `--target`。

开跑前 `ls tests/*.rs | sort` 为 157 行。`B=$(( (157 + 11) / 12 ))` 为 14，`K=$(( 14 * 12 - 157 ))` 为 11。`K <= B`。前 11 个文件批各 11 个文件，后 3 个文件批各 12 个文件。加上最前面的 `--lib --bins`，共 15 次调用。每个主名恰好进入一个文件批。没有一批需要复核。

结束后再次 `ls tests/*.rs | sort`，与开跑前逐行相同（`cmp` 退出码 0，仍是 157 行）。

**Deviations from Plan**

无。

**Self-Review**

没有产品代码 diff。15 次 `cargo test` 都带 `--offline` 和 `-- --test-threads=1`，批间串行。没有无过滤全量，没有 musl 测试套件，没有补跑 macOS。`cargo fmt` 没有写回。Gate 3 通过。没有遗留的 Minor 问题。

库测试里有两条用例名以 `result::` 开头，输出行是 `test result::tests::... ... ok`。它们是用例行，不是 `test result: ok.` 汇总行。下面只记录带 `failed` 计数的汇总行。这 15 批的汇总行全部是 `0 failed`。

**Verification Evidence**

工作目录：`/home/daivy/projects/RTsql-changes/2026-10-28-ms41-build-and-compat-baseline`。分支 `changes/2026-10-28-ms41-build-and-compat-baseline`。

批 1：`cargo test --offline --lib --bins -- --test-threads=1`。退出码 0。

```
test result: ok. 436 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.77s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

文件批 1：`cargo test --offline --test aggregate_extension_test --test aggregate_output_shape_test --test aggregate_test --test alter_table_test --test attach_session_test --test btree_merge_test --test btree_scale_test --test btree_split_test --test btree_test --test busy_timeout_test --test cache_perf_test -- --test-threads=1`。退出码 0。

```
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.52s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.07s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.32s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.65s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 25.71s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.31s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.41s
```

文件批 2：`cargo test --offline --test catalog_bounded_read_test --test catalog_durability_test --test chain_join_test --test check_command_test --test checkpoint_command_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test checkpoint_test --test cli_test --test composite_index_lifecycle_test --test composite_index_runtime_test -- --test-threads=1`。退出码 0。

```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.87s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 40.01s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.81s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.22s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 126 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 114.64s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.86s
```

文件批 3：`cargo test --offline --test concurrent_test --test connection_limit_test --test constraint_enforcement_test --test cross_db_dml_test --test cross_db_query_test --test ctas_test --test cte_test --test data_page_bounded_read_test --test database_file_lock_test --test datetime_function_test --test datetime_type_test -- --test-threads=1`。退出码 0。

```
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.85s
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.53s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.03s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.27s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.67s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.33s
```

文件批 4：`cargo test --offline --test ddl_catalog_transactional_test --test ddl_crash_recovery_test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test default_declaration_test --test default_precheck_test --test derived_scan_alias_test --test dml_tombstone_abandon_window_test --test dml_tx_id_test --test drop_persistence_test --test drop_table_free_test -- --test-threads=1`。退出码 0。

```
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.34s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.77s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.98s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.63s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.54s
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.03s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.91s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.88s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.46s
```

文件批 5：`cargo test --offline --test e2e_test --test encryption_test --test executor_test --test explain_test --test explicit_tx_test --test expression_e2e_test --test file_header_test --test file_storage_io_test --test free_page_lifecycle_test --test full_table_dml_test --test gc_test -- --test-threads=1`。退出码 0。

```
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 75.99s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.62s
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.84s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 31.42s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

文件批 6：`cargo test --offline --test group_by_expr_test --test hash_join_build_side_test --test import_input_surface_test --test index_key_encoding_evaluation_test --test index_manager_test --test index_page_ownership_stack_overflow_test --test index_page_ownership_test --test index_range_routing_test --test index_read_bounded_read_test --test index_write_atomicity_test --test insert_column_list_test -- --test-threads=1`。退出码 0。

```
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.85s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.87s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.97s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.38s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.83s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.89s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

文件批 7：`cargo test --offline --test insert_select_test --test isolation_level_test --test join_output_resolution_test --test join_test --test key_type_conformance_test --test keyless_eq_routing_test --test keyless_row_test --test legacy_compat_test --test limit_test --test memory_db_test --test mvcc_abort_test -- --test-threads=1`。退出码 0。

```
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.39s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.72s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.62s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

文件批 8：`cargo test --offline --test mvcc_commit_test --test mvcc_record_test --test mvcc_tombstone_visibility_test --test nested_loop_join_test --test network_protocol_test --test network_server_test --test no_from_select_test --test order_by_alias_sort_test --test outer_join_test --test overflow_page_test --test param_binding_test -- --test-threads=1`。退出码 0。

```
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.02s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.90s
test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
```

文件批 9：`cargo test --offline --test parser_test --test pg_integration_test --test pg_messages_test --test pg_protocol_test --test pipeline_test --test plan_cache_test --test plan_exec_test --test planner_test --test predicate_test --test prefetch_test --test prepared_statement_test -- --test-threads=1`。退出码 0。

```
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.27s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 107 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.84s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

文件批 10：`cargo test --offline --test projection_expression_test --test projection_test --test pushdown_test --test query_memory_test --test readonly_session_test --test recovery_bounded_read_test --test recovery_e2e_test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test -- --test-threads=1`。退出码 0。

```
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.80s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.85s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

文件批 11：`cargo test --offline --test recovery_wal_convergence_test --test release_durability_precondition_test --test release_path_bounded_read_test --test runtime_test --test scalar_function_test --test scalar_subquery_comparison_test --test schema_persistence_test --test secondary_index_equality_test --test secondary_index_test --test select_distinct_test --test self_join_test -- --test-threads=1`。退出码 0。

```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.51s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.23s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 50.35s
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.43s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

文件批 12：`cargo test --offline --test set_operations_test --test sort_merge_join_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test string_index_runtime_test --test string_index_sql_test --test string_secondary_index_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test -- --test-threads=1`。退出码 0。

```
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.51s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 37.48s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.10s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.79s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.46s
test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.95s
```

文件批 13：`cargo test --offline --test subset_insert_test --test sync_loader_test --test table_alias_resolution_test --test table_manager_test --test truncate_test --test tx_statement_test --test typed_api_test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test value_test --test version_chain_test -- --test-threads=1`。退出码 0。

```
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.94s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.34s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.28s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

文件批 14：`cargo test --offline --test view_test --test visibility_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_integration_test --test wal_record_test --test wal_recovery_large_test --test wal_writer_test --test write_failure_state_test --test write_path_slot_bounds_test --test write_type_conformance_test -- --test-threads=1`。退出码 0。

```
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.55s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.52s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 19.64s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.93s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 22.44s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

清单：开跑前与结束后的 `ls tests/*.rs | sort` 逐行相同，157 行。汇总行共 159 行：库与二进制 2 行，11 个文件批各 11 行，3 个文件批各 12 行。

`cargo fmt --check` 退出码 0。随后 `cargo clippy --offline --all-targets -- -D warnings` 退出码 0。写入本 Response 之前，`git status --short` 为空。

`~/.cargo/config` 的弃用提示可以出现，未当作失败，也没有改该文件。macOS 仍为未完成。没有运行 musl 测试套件。

Gate 4：T6 的 Verification 由上述退出码、汇总行、前后清单和产品路径未改动满足。

Gate 5：未命中。

**Persisted Evidence**

None required

**Experience Candidates**

None

**未解决问题**

无。macOS 保持未完成。没有把 musl 产品二进制冒烟写成测试套件已通过。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读 `f0b1ec6e63eac83a68f84ff03b15ab3322d31892...HEAD`。HEAD 为 `46fb10431f41de89e023b8063a457550f27cf3ab`，阅读时工作区干净。这次提交只改了本 Cycle 文件和 `tasks.md`。`src/`、`tests/`、`Cargo.toml`、`Cargo.lock` 和两份 README 不在这次 diff 里。自 merge-base `a69fe86bbf08a04b0d0cc62d9183da4ae8a5008b` 到 `f0b1ec6` 的产品改动属于已经 accepted 的 000 与 001，本次不把它们重新当成 T6 的缺口。

T6 的 15 批沿用 Act Response（`Status: reported`，提交 `46fb104`）。需要重跑的情形没有出现：记录的命令、批大小和汇总行彼此一致，结论写出后 `tests/` 没有再改。本次不重跑这 15 批，也不重跑 fmt 和 clippy。

Act Response 写明开跑前 N=157，`B=$(( (157 + 11) / 12 ))` 为 14，`K=$(( 14 * 12 - 157 ))` 为 11，且 `K <= B`。当前工作树 `ls tests/*.rs | sort` 仍是 157 个主名。14 个文件批的 `--test` 主名按记录顺序接起来，与这份排序清单逐项相同，每个主名恰好一次。前 11 个文件批各 11 个文件，后 3 个文件批各 12 个文件。第一批是 `cargo test --offline --lib --bins -- --test-threads=1`。15 条命令都有 `--offline` 和 `-- --test-threads=1`，都带 `--lib --bins` 或至少一个 `--test`。没有不带目标过滤的 `cargo test`。这些 `cargo test` 里没有 `--target`、`--doc`、`--examples`、`--benches`、`--all-targets` 或 `--release`。

15 批的 cargo 退出码都是 0。汇总行 159 行：库与二进制 2 行，每个文件批的行数等于该批 `--test` 个数。每一行都含 `0 failed`。`cli_test` 那一行是 `2 ignored`；计划允许 ignored 不为 0。二进制那一行和 `network_server_test` 那一行是 `0 passed; 0 failed`，各自有一行汇总，failed 仍是 0。Self-Review 里提到的 `test result::` 用例行不是带 failed 计数的汇总行；汇总行数量已经等于 2 加 157，没有少记汇总行。

`cargo fmt --check` 退出码 0。随后 `cargo clippy --offline --all-targets -- -D warnings` 退出码 0。Response 写明写入前 `git status --short` 为空。macOS 仍是未完成。没有 musl 测试套件被写成通过。

没有阻塞 Acceptance 的发现。没有 Minor finding。当前 change 范围外没有新的实质缺陷。

**Evidence**

沿用 Act Response（`Status: reported`，提交 `46fb104`）中 15 批的命令、退出码和 `test result:` 原行，以及两条收尾命令的退出码。Persisted Evidence 为 none。未因缺少 evidence 目录提出问题。

- 清单：当前 157 个 `tests/*.rs` 主名与 14 个文件批的 `--test` 顺序一致，各出现一次。
- 切批：K=11，B=14。文件批为 11 个文件的 11 批，加上 12 个文件的 3 批，再加上单独的 `--lib --bins`，共 15 次调用。
- 结果：159 行汇总全部含 `0 failed`。15 次退出码都是 0。
- 收尾：fmt 退出码 0，clippy 退出码 0。

**Follow-up Decision**

R6，以及本轮收尾的 R1、R2，都有与上述记录一致的结果。本 Cycle 没有改产品文件。既有 Acceptance 满足，Iteration Plan 保持不变。不要求当前 Cycle 修复，不创建后继 Cycle。`tasks.md` 没有再往后的 Iteration。

**Next Cycle**

None

**Next Iteration**

None
