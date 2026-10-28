# Iteration 001 / Cycle 000: 本机与 musl 运行

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3、T4、T5
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 的 R3、R4、R5、R7。000 已通过的 R1、R2 保持不变。SQL、存储、错误语义和运行形态保持 000 收尾时的行为。
- Excluded scope: T6，M19 全量，musl 测试套件，macOS 交叉编译，依赖升级，`rust-toolchain.toml`，`.cargo/config`，`install.sh`，`dist/`，CI。

**Objective**

gnu release 二进制对含 `/` 的临时库完成建表、插入和查询。musl 静态二进制在本机完成同一冒烟，或因环境缺失记为未完成。macOS 在无执行环境时记为未完成。两份 README 的构建与支持段落和这些结果一致。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms41-build-and-compat-baseline`，HEAD `0462a0bb3f44394c7ab9589093f78cc5170a4ed5`，工作区干净。000 的产品改动在 `592e43ba36a625df80ba14274f85cf15d20e95d1`。`592e43b..0462a0b` 只改了 000 的 Cycle 与 change `tasks.md`，没有改 `src/`、`tests/`、`Cargo.toml`、`Cargo.lock` 或两份 README。000 Act Response 为 reported，Review Result 为 accepted。沿用的结论：`cargo fmt --check` 退出码 0；`cargo clippy --offline --all-targets -- -D warnings` 退出码 0。ISS36 在 2026-10-25 的测量不作本轮门禁结果。本 Cycle 不重跑这两条，也不跑 M19。
- Current-State Evidence:
  - 本机 2026-10-28：`uname -srm` 为 `Linux 6.18.40.1-microsoft-standard-WSL2 x86_64`。rustc 1.99.1（58cf8f9c3 2026-10-13）。无 `rust-toolchain.toml`。`cargo metadata --offline --locked` 退出码 0。`~/.cargo/config` 只有 `git-fetch-with-cli`，没有默认 target；弃用提示可以出现，不修改该文件。
  - 已安装 target 含 `x86_64-unknown-linux-gnu`，不含 `x86_64-unknown-linux-musl`，也不含名称里有 `apple` 或 `darwin` 的 target。链接器 `/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc` 存在且可执行。`x86_64-apple-darwin-gcc`、`o64-clang`、`x86_64-apple-darwin20.4-clang` 都不在 PATH。`file`、`ldd`、`mktemp` 在 `/usr/bin`。
  - 二进制入口是 `src/main.rs` 的 `main`，调用 `rtsql::cli::run`。包名 `rtsql`，`Cargo.toml` 没有另立 `[[bin]]`。不带 `--target` 的 release 产物是 `target/release/rtsql`。
  - 主命令位置参数是 `<db> <sql>`，见 `CliArgs`。`--format json` 经 `kind` 固定为 `OutputKind::Json`，不随 TTY 变化。库路径不含 `/` 时进入 `$RTSQL_HOME/db` 或 `$HOME/.rtsql/db`。含 `/` 时 `resolve_db_path` 按字面路径使用，不创建目录。
  - 主命令在打开前检查路径存在。不存在则 `ExitStatus::General`，进程退出码 1，提示先 `rtsql new`。`rtsql new <路径>` 走 `lifecycle::new_db`：已存在则失败；否则创建父目录、建库并关闭。成功时无 stdout，退出码 0。
  - `CREATE TABLE t (id INT)` 的 `INT` 由 `convert_data_type` 收成 `ColumnType::Int`。`SELECT` 的 JSON 由 `render` 用 `serde_json` 生成，形状为 `{"columns":[...],"rows":[[...]]}`，整数没有引号，键之间没有空格。`emit_stdout` 在文本后再写一个换行。验收只要求查询 stdout 包含 `{"columns":["id"],"rows":[[1]]}`。
  - 两份 README 还没有这次 gnu 或 x86_64 musl 冒烟记录。英文第 2 段写 “targets Linux and macOS”。中文第 2 段写 “支持 Linux 和 macOS”。`## Build and test` 与 `## 构建与测试` 给出的是通用 `cargo build --release`。`## Known limitations` 与 `## 已知限制` 的第一条把 Linux 和 macOS 写成支持的构建宿主。RISC-V 小节写明交叉构建脚本本身不运行 RISC-V 二进制。
- Code and Critical Path:
  - T3 构建：`cargo build --locked --offline --release --bin rtsql`。不传 `--target`。
  - 建库：`target/release/rtsql new <含 / 的临时路径>` → `Command::New` → `new_db`。
  - 三条 SQL：同一二进制，`--format json <路径> "<语句>"` → `execute_main_command` → `run_sql` → `render`。语句分别是 `CREATE TABLE t (id INT)`、`INSERT INTO t (id) VALUES (1)`、`SELECT id FROM t`。每次调用只执行一条。
  - T4 构建：环境变量 `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER` 指向已有的 `x86_64-linux-musl-gcc`，命令是 `cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`。产物是 `target/x86_64-unknown-linux-musl/release/rtsql`。`new` 和三条 SQL 都用这个二进制。
  - T5 只改两份 README 里下面列出的段落。

**Implementation Guidance**

先做 T3，再做 T4，最后做 T5。临时库用 `mktemp -d` 放在系统临时目录，文件名自定，完整路径必须含 `/`。记录退出码和查询 stdout 后删除该目录。不设置 `RTSQL_HOME`，不使用裸库名，不写入 `~/.rtsql`。不改 Rust 源码、`Cargo.toml`、`Cargo.lock`。不写 `.cargo/config` 或 `~/.cargo/config`。不运行 `cargo test`、`cargo fmt` 或 `cargo clippy`。不跑 M19。

**Task Definitions**

### T3: gnu release 二进制完成 CLI 冒烟

- Requirement/Scenario: R3 / Linux 本机二进制实际运行
- Depends on: None。000 已经 accepted。
- Targets: `src/main.rs::main`，`src/cli/mod.rs` 的 `CliArgs`、`execute_main_command`、`run_sql`、`emit_stdout`，`src/cli/lifecycle.rs::new_db`，`src/cli/resolve.rs::resolve_db_path`，`src/cli/render.rs::render`，`src/parser/planner/ddl_dml.rs::convert_data_type`
- Current behavior: 000 之后没有构建过本轮要求的 gnu release 二进制，也没有用它执行这三条 SQL。主命令对不存在的库退出码为 1。
- Required behavior: `cargo build --locked --offline --release --bin rtsql` 退出码 0。对 `mktemp -d` 下的库路径，`target/release/rtsql new <路径>` 退出码 0。随后三条命令退出码都为 0：
  - `target/release/rtsql --format json <路径> "CREATE TABLE t (id INT)"`
  - `target/release/rtsql --format json <路径> "INSERT INTO t (id) VALUES (1)"`
  - `target/release/rtsql --format json <路径> "SELECT id FROM t"`
  查询 stdout 包含 `{"columns":["id"],"rows":[[1]]}`。
- Preserve: `--locked` 与 `--offline`。不传 `--target`。SQL 文本、存储格式、错误语义和退出码矩阵保持不动。
- Forbidden: 去掉 `--locked` 或 `--offline`。改 `Cargo.toml` 或 `Cargo.lock`。调用 `install.sh`。用裸库名或 `:memory:`。用 `cargo test` 代替这三条命令。把 CREATE 或 INSERT 的 `affected_rows` 数值写成额外通过条件。失败后修改产品代码。
- First test: 本任务不改 Rust。先行观察就是上面的构建、`new` 和三条 SQL。预期构建和四次二进制调用退出码都为 0，且查询 stdout 包含该子串。没有需要先看到失败的产品改动。
- GREEN condition: 与 Required behavior 相同。
- Verification: 各命令的退出码，以及 SELECT 的 stdout。构建非 0 表示 gnu release 没有得到。`new` 或 SQL 非 0 表示冒烟没有完成。stdout 不含该子串表示 JSON 行集与 R3 不一致。
- Stop when: 构建、`new` 或任一条 SQL 退出码非 0，或查询 stdout 不含该子串。缓存缺失导致 `--offline` 不能编译时同样停止。不在本 Cycle 改依赖或源码，不开始 T4、T5。

### T4: musl 静态二进制完成同一冒烟，或记环境未完成

- Requirement/Scenario: R4 / musl 静态二进制实际运行
- Depends on: T3
- Targets: 同上 CLI 路径。构建目标是 `x86_64-unknown-linux-musl`。链接器是 `/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc`。产物是 `target/x86_64-unknown-linux-musl/release/rtsql`。
- Current behavior: `x86_64-unknown-linux-musl` 未安装。链接器文件存在且可执行。还没有本轮的 musl 二进制，也没有用它执行冒烟。
- Required behavior:
  - 若 `rustup target list --installed` 仍没有 `x86_64-unknown-linux-musl`，先执行 `rustup target add x86_64-unknown-linux-musl`。
  - 然后执行：
    `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`
  - `file <产物>` 的输出含 `statically linked`，或者 `ldd <产物>` 的输出含 `not a dynamic executable`。前者已经满足时不必再跑 `ldd`。把实际看到的那句写入 Act Response。
  - 用该 musl 二进制，对另一个 `mktemp -d` 路径重复 T3 的 `new` 和三条 SQL。通过条件与 T3 相同。
  - 环境缺失时不要求二进制存在。Act Response 记录失败命令和错误，该分支为未完成。
- Preserve: 链接器只通过这一次命令的环境变量指定。`--locked`、`--offline` 和 `+crt-static` 保持不变。gnu 冒烟的结果不因本任务失败而撤销。
- Forbidden: 改 `Cargo.toml`、`Cargo.lock` 或把 `rusqlite` 改成 `bundled`。写 `.cargo/config`。运行 `cargo test --target x86_64-unknown-linux-musl`。生成 `dist/`。为了通过而更换链接器、去掉 `+crt-static` 或升级依赖。用 gnu 二进制代替 musl 二进制执行这四次调用。
- First test: 本任务不预先改源码。先行观察是 target 是否已安装、链接器是否可执行，以及随后的 `cargo rustc`。当前已知 target 未安装、链接器可执行。
- GREEN condition: 静态判定成立，且 musl 二进制的 `new` 与三条 SQL 达到 T3 的通过条件。环境缺失时的完成条件见 Stop when 的第一条，不把未运行写成通过。
- Verification: `rustup target add` 与 `cargo rustc` 的退出码，`file` 或 `ldd` 的原句，musl 二进制四次调用的退出码和 SELECT stdout。编译错误和链接错误按 Stop when 分开。
- Stop when:
  - `rustup target add` 退出码非 0，或链接器路径不存在、不可执行：记录命令与错误，不改依赖。该分支未完成，继续 T5。
  - 链接器已启动，但 gcc 或 ld 以缺少启动文件、找不到 musl 库这类诊断失败：同样记录，不改依赖。该分支未完成，继续 T5。
  - `cargo rustc` 报出本仓库或 `Cargo.lock` 里某个依赖的 rustc 编译错误：停止。不升级依赖，不改源码，不开始 T5，回到计划。
  - 产物既不含 `statically linked`，`ldd` 也不含 `not a dynamic executable`：停止，不改链接参数，不开始 T5，回到计划。
  - 静态判定已通过，但 musl 二进制的 `new` 或 SQL 失败，或 stdout 不含该子串：停止，不开始 T5，回到计划。

### T5: macOS 记未完成，两份 README 与实测一致

- Requirement/Scenario: R5 / macOS 无执行环境；R7 / 文档与结果一致
- Depends on: T3 已通过，且 T4 要么已通过，要么已按环境缺失记为未完成。T4 因编译错误、非静态产物或冒烟失败而停止时，本任务不开始。
- Targets:
  - `README.md` 开头第 2 段
  - `README.md` 的 `## Build and test`
  - `README.md` 的 `## Known limitations` 第一条
  - `README.zh-CN.md` 开头第 2 段
  - `README.zh-CN.md` 的 `## 构建与测试`
  - `README.zh-CN.md` 的 `## 已知限制` 第一条
- Current behavior: 上述段落把 Linux 和 macOS 写成当前目标或支持的构建宿主，没有这次 gnu 冒烟，也没有 x86_64 musl 的运行或未完成记录。2026-10-28 的复查是 Linux x86_64，没有 Apple target，PATH 中没有列出的 Darwin 链接器。
- Required behavior:
  - 实施时重跑 `uname -srm`、`rustup target list --installed`，并对 `x86_64-apple-darwin-gcc`、`o64-clang`、`x86_64-apple-darwin20.4-clang` 做 `command -v`。三者仍分别是 Linux x86_64、无 `apple`/`darwin` target、三个命令都找不到时，macOS 记为未完成。
  - 两份 README 的上列段落写明：Linux x86_64 gnu 已用 T3 的构建命令得到 release 二进制，并对含 `/` 的临时库完成建表、插入和查询，查询输出包含 `{"columns":["id"],"rows":[[1]]}`。
  - T4 通过时，同一段落写明 musl 命令、链接器路径、`+crt-static`、Act Response 里的静态判定原句，以及 musl 二进制完成了同一组 SQL。T4 只是环境未完成时，写明 `x86_64-unknown-linux-musl` 本次未运行以及记录的原因，不得写成已验证。
  - 同一段落写明 macOS 本次未运行，不得写成已验证。
  - 中英文对 gnu、musl、macOS 三者的完成状态一致。保留「Windows 文件 I/O 未实现」和「RISC-V 交叉构建不等于在硬件上运行」。
- Preserve: RISC-V 交叉构建小节、`install.sh` 小节、性能数字，以及已知限制里除第一条以外的条目。通用 `cargo build --release` 示例可以留在构建节，但不得改写成这次已经跑过 M19 或 `cargo test --no-fail-fast`。
- Forbidden: 安装 osxcross 或 macOS SDK。使用 sudo。编造 macOS 或 musl 运行输出。在 T4 未完成时把 musl 写成已验证。修改 Rust 或依赖来迁就文档。改动上列段落之外的产品说明来制造新的支持声明。
- First test: 修改前阅读上列六处，确认它们仍是 Current behavior 的措辞，且没有 T3、T4 的实测句。这是变更前状态。
- GREEN condition: 修改后再次阅读这六处，Required behavior 的事实都在，未运行的平台没有被写成已验证，两份文档的完成状态一致。
- Verification: 环境复查命令的输出，加上对这六处的直接阅读。阅读结果与 Act Response 中的 T3、T4 记录不一致，则文档未完成。
- Stop when: 复查不再是 Linux x86_64、出现 Apple target，或任一 Darwin 链接器出现在 PATH。此时停止并回到计划，不在本 Cycle 增加 macOS 构建。文档事实只能通过改上列段落以外的内容才成立时，同样停止并回到计划。

**Invariants**

- 不改变 SQL 结果、页格式、错误语义、CLI 退出码和 000 已经落地的 lint 改写。
- `Cargo.lock` 保持不变。
- 不把交叉编译成功当成目标已经运行。
- 不把 `~/.cargo/config` 的弃用提示当作项目失败。
- 全量回归不属于本 Cycle。

**Non-goals**

- T6 与 M19 全量。
- musl 上的测试二进制、系统 sqlite 的 musl 化、依赖升级、工具链文件、macOS 交叉编译、发布归档、CI 和 `install.sh`。

**Acceptance**

- R3：gnu 构建退出码 0。`new` 与三条 SQL 退出码都为 0。SELECT stdout 包含 `{"columns":["id"],"rows":[[1]]}`。
- R4：musl 产物满足静态判定，且同一组 `new` 与 SQL 通过。环境缺失时 Act Response 有命令和错误，该分支为未完成，gnu 结果仍算通过。
- R5：复查后仍无 macOS 执行环境时，Act Response 写明未完成，没有编造的运行结果。
- R7：两份 README 的指定段落与 T3、T4、T5 的实际结果一致。未运行的平台不是已验证。
- 本 Cycle 不改 Rust 源码和依赖。

**Verification**

- Linux 本机：真实执行 `target/release/rtsql`。通过依据是退出码和 SELECT stdout。
- musl：真实执行 musl 产物；静态依据是 `file` 或 `ldd` 的原句。环境缺失时依据是失败命令的退出码和错误文本。
- macOS：`uname -srm`、`rustup target list --installed` 和三次 `command -v` 的输出。
- 文档：阅读 T5 列出的六处，与 Act Response 对照。
- 不跑 `cargo test`，不跑 M19，不重跑 000 的 fmt 和 clippy。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

退出码、stdout 子串、`file` 或 `ldd` 原句、环境复查输出和 README 对照结论写入 Act Response 即可。这些命令可以在同一台机器上重跑。不保存完整编译日志。

**Risks and Notes**

- `rustup target add` 需要网络。网络失败按 T4 的环境缺失处理。
- `~/.cargo/config` 的弃用提示可以继续打印。
- README 的句子怎么组织由 Act 决定，只要 T5 列出的事实都在，且未运行的平台没有写成已验证。
- 002 仍只写在 `tasks.md`。本 Cycle 不展开它，也不实施 T3–T5 以外的修复。

## Act Response

- Status: 未开始

## Plan Review

- Review Result: pending
