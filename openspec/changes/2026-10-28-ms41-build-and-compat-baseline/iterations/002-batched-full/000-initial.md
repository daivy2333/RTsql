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

- Status: 未执行

本 Cycle 创建时尚未实施。

## Plan Review

- Review Result: pending

实施完成前不在这里写审查结论。
