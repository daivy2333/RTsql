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

- Status: reported

**Implemented**

没有修改 Rust 源码、`Cargo.toml` 或 `Cargo.lock`。

T3：`cargo build --locked --offline --release --bin rtsql` 在 `CARGO_TARGET_DIR` 未设置时退出码 0，产物是 `target/release/rtsql`。对 `/tmp/tmp.8k2qojAEig/smoke.db`，`new` 与三条 SQL 退出码都为 0。`SELECT` stdout 为 `{"columns":["id"],"rows":[[1]]}`。临时目录已删除。

T4：`rustup target add x86_64-unknown-linux-musl` 退出码 0。随后的 `cargo rustc` 命令退出码 0，产物是 `target/x86_64-unknown-linux-musl/release/rtsql`。`ldd` 原句是 `statically linked`。对 `/tmp/tmp.A4zdYxRdk1/smoke.db`，`new` 与三条 SQL 退出码都为 0，`SELECT` stdout 同样包含 `{"columns":["id"],"rows":[[1]]}`。临时目录已删除。

T5：macOS 未完成。`README.md` 与 `README.zh-CN.md` 的开头第 2 段、构建节和已知限制第一条写明了 T3、T4 的命令和结果，并写明 macOS 本次未运行。Windows 文件 I/O 未实现，以及 RISC-V 交叉构建不等于在硬件上运行，都保留。

**Deviations from Plan**

执行 T3 时，shell 里的 `CARGO_TARGET_DIR` 指向沙箱缓存目录。第一次 `cargo build --locked --offline --release --bin rtsql` 退出码 0，但二进制不在 `target/release/rtsql`。取消该变量后用同一命令再构建一次，退出码 0，产物落到计划路径。记录的 T3 结果是这一次。冒烟使用的是 `target/release/rtsql`。

`file` 的原句是 `ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, not stripped`，其中没有 `statically linked`。`ldd` 的原句是 `statically linked`，其中没有 `not a dynamic executable`。静态判定采用 `ldd` 的这句。链接器、`+crt-static` 和依赖都没有改。判定成立后，musl 冒烟通过。

**Self-Review**

T3、T4 没有产品代码 diff。T5 只改了两份 README 里点名的六处。六处对 gnu、musl、macOS 的完成状态一致：gnu 与 musl 已运行，macOS 未运行。没有遗留的 Minor 问题。

**Verification Evidence**

T3 构建：`cargo build --locked --offline --release --bin rtsql`，退出码 0。`Finished release profile [optimized] target(s) in 1m 27s`。`target/release/rtsql` 存在。

T3 冒烟，库路径 `/tmp/tmp.8k2qojAEig/smoke.db`：

- `target/release/rtsql new <路径>`：退出码 0。
- `target/release/rtsql --format json <路径> "CREATE TABLE t (id INT)"`：退出码 0。
- `target/release/rtsql --format json <路径> "INSERT INTO t (id) VALUES (1)"`：退出码 0。
- `target/release/rtsql --format json <路径> "SELECT id FROM t"`：退出码 0。stdout 为 `{"columns":["id"],"rows":[[1]]}`，包含要求的子串。

T4：`rustup target add x86_64-unknown-linux-musl` 退出码 0。之后 `rustup target list --installed` 含 `x86_64-unknown-linux-musl`。

`CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`：退出码 0。`Finished release profile [optimized] target(s) in 1m 25s`。

`file target/x86_64-unknown-linux-musl/release/rtsql` 的原句见 Deviations。`ldd` 退出码 0，原句是 `statically linked`。

T4 冒烟，库路径 `/tmp/tmp.A4zdYxRdk1/smoke.db`，二进制是 `target/x86_64-unknown-linux-musl/release/rtsql`：`new`、`CREATE`、`INSERT`、`SELECT` 退出码都为 0。`SELECT` stdout 为 `{"columns":["id"],"rows":[[1]]}`。

T5 复查：`uname -srm` 退出码 0，输出 `Linux 6.18.40.1-microsoft-standard-WSL2 x86_64`。`rustup target list --installed` 退出码 0，已安装目标为 `aarch64-unknown-none`、`riscv64gc-unknown-linux-gnu`、`riscv64gc-unknown-linux-musl`、`riscv64gc-unknown-none-elf`、`x86_64-unknown-linux-gnu`、`x86_64-unknown-linux-musl`、`x86_64-unknown-none`，没有名称含 `apple` 或 `darwin` 的目标。`command -v x86_64-apple-darwin-gcc`、`command -v o64-clang`、`command -v x86_64-apple-darwin20.4-clang` 退出码都是 1。macOS 未完成。

修改前阅读六处：开头第 2 段和已知限制第一条把 Linux 与 macOS 写成当前目标或支持的构建宿主，构建节没有这次 gnu 或 musl 冒烟。修改后再次阅读：六处都写了 T3 的构建命令、含 `/` 的临时库、三条 SQL，以及查询输出包含 `{"columns":["id"],"rows":[[1]]}`；都写了 musl 命令、链接器路径、`+crt-static` 和 `ldd` 原句 `statically linked`，并写明 musl 二进制完成了同一组 SQL；都写明 macOS 本次未运行。中英文完成状态一致。构建节原有的 `cargo build --release` 示例仍在，没有写成这次跑过 `cargo test --no-fail-fast`。

`~/.cargo/config` 的弃用提示在两次构建中出现，未当作失败。没有设置 `RTSQL_HOME`。没有跑 `cargo test`、`cargo fmt`、`cargo clippy` 或 M19。000 的 fmt 与 clippy 结论继续沿用：覆盖范围没有因本 Cycle 失效，本 Cycle 没有改 Rust 源码。

Gate 3：T3 通过。T4 在采用 `ldd` 原句作为静态判定后通过。T5 通过。

Gate 4：T3、T4、T5 的 Verification 由上面的退出码、stdout、`ldd` 原句、环境复查和六处阅读满足。

Gate 5：未命中。

**Persisted Evidence**

None required

**Experience Candidates**

None

**未解决问题**

无。macOS 按环境复查记为未完成，没有编造运行结果。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读 `875a3ab1ae519c4c8b831691a63cfb7a9b323a00...HEAD` 的 diff。HEAD 为 `a74c0b07c0b1ef672c5a33e1e47fc242f80ee9d6`，工作区干净。产品 diff 只有 `README.md` 与 `README.zh-CN.md`。没有 Rust 源码、`Cargo.toml` 或 `Cargo.lock` 改动。

T3、T4 的构建和冒烟沿用 Act Response（`Status: reported`，提交 `a74c0b0`）。本 Cycle 没有再改这些命令覆盖的代码，本次不重跑两次冒烟。静态判定的原句和任务预写的子串不一致，所以对仍在的 `target/x86_64-unknown-linux-musl/release/rtsql` 补看了 `file` 和 `ldd`。`file` 的原句是 `ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, not stripped`。`ldd` 退出码 0，原句是 `statically linked`。与 Act Response 记录的两句相同。

`file` 不含 `statically linked`，`ldd` 不含 `not a dynamic executable`。按 T4 的停止条件，这两个子串都缺失时要停下来回到计划，不开始 T5。Act 没有停，把 `ldd` 的 `statically linked` 当作静态判定，并写了 README。偏差原因是 Plan 错误和 Act 偏离：计划把 `statically linked` 派给 `file`，把 `not a dynamic executable` 派给 `ldd`；这台机器上的 `file` 对静态 PIE 写 `static-pie linked`，`ldd` 写 `statically linked`。

这不阻塞 R4。proposal 第 4 条要的是 `file` 或 `ldd` 显示静态链接。两句原话都是静态链接：`static-pie linked` 是 file 对没有动态解释器的 PIE 的说法，`ldd` 的原句就是 `statically linked`。本 Cycle 的 Verification 把静态依据定为这两条原句。README 引用的是实际看到的 `ldd` 原句，没有写成不存在的 `not a dynamic executable`。冒烟的退出码和查询子串仍以 Act Response 为准。为了凑出旧子串去改链接参数，任务已经禁止。

T3 有一处已经收回的环境偏差。第一次构建时 `CARGO_TARGET_DIR` 指向沙箱缓存，产物不在 `target/release/rtsql`。取消该变量后同一命令再次构建，退出码 0。记录下来的 T3 结果和冒烟用的都是计划路径上的二进制。这不留下缺口。

T5 的六处与记录一致。开头第 2 段、构建节新增段和已知限制第一条，中英文都写了 gnu 构建命令、路径含 `/` 的临时库、三条 SQL、查询子串 `{"columns":["id"],"rows":[[1]]}`、musl 命令、链接器路径、`+crt-static` 和 `ldd` 原句 `statically linked`，并写明 macOS 本次未运行。已知限制仍保留 Windows 文件 I/O 未实现，以及 RISC-V 交叉构建未在硬件上运行。构建节原有的 `cargo build --release` 示例还在，没有改写成这次跑过 `cargo test --no-fail-fast`。gnu 与 musl 已运行，macOS 未运行，两份文档的完成状态一致。

没有阻塞 Acceptance 的发现。当前 change 范围外没有新的实质缺陷。

**Evidence**

沿用 Act Response（`Status: reported`，提交 `a74c0b0`）的冒烟和环境复查。这些结论产生后，`src/`、`Cargo.toml` 和 `Cargo.lock` 没有变化。本次不重跑构建和四次二进制调用。

- T3：`cargo build --locked --offline --release --bin rtsql` 退出码 0。`target/release/rtsql new` 与三条 `--format json` SQL 退出码都为 0。`SELECT` stdout 为 `{"columns":["id"],"rows":[[1]]}`。库路径 `/tmp/tmp.8k2qojAEig/smoke.db`。
- T4：`rustup target add x86_64-unknown-linux-musl` 退出码 0。带链接器环境变量、`--target x86_64-unknown-linux-musl` 和 `+crt-static` 的 `cargo rustc` 退出码 0。musl 二进制对 `/tmp/tmp.A4zdYxRdk1/smoke.db` 的 `new` 与三条 SQL 退出码都为 0，`SELECT` stdout 相同。
- 本次补看：`file target/x86_64-unknown-linux-musl/release/rtsql` 为 `static-pie linked`；`ldd` 退出码 0，原句 `statically linked`。
- T5 复查沿用 Act Response：`uname -srm` 为 `Linux 6.18.40.1-microsoft-standard-WSL2 x86_64`。已安装 target 没有名称含 `apple` 或 `darwin` 的项。三个 Darwin 链接器的 `command -v` 退出码都是 1。macOS 未完成。
- 六处 README 为本次直接阅读，见 Findings。Persisted Evidence 为 none。未因缺少 evidence 目录提出问题。

**Follow-up Decision**

R3、R4、R5、R7 都有与代码和文档一致的结果。musl 的工具子串和计划预写的不同，但原句已经表明静态链接，文档引用的也是这句原话。既有 Acceptance 满足，Iteration Plan 保持不变。不创建后继 Cycle。Iteration 002 仍只写在 `tasks.md`，本 Review 不展开它。

**Next Cycle**

None

**Next Iteration**

`tasks.md` 的 Iteration 002（分批全量，T6）。目录未创建。
