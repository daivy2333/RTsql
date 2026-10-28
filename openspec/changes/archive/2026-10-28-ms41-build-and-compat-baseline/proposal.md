## Why

MS41 要求项目质量检查能够通过，并在 Linux 本机与 x86_64-linux-musl 上取得构建和实际运行结果。ISS36 登记时的格式与 lint 名单来自 2026-10-25 的工作区，不能当作当前门禁结果。macOS 在本机没有可执行环境，该平台分支保持未完成，不阻塞其余可完成部分。

## What Changes

- 以当前工作区重新定位并修复 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`，使两条项目门在当前 stable 工具链上退出码为 0。
- 在 Linux x86_64 上用现有 lockfile 构建 release 二进制，并实际执行 CLI 建表、插入和查询。
- 为 `x86_64-unknown-linux-musl` 构建静态 release 二进制，确认静态链接，并在本机执行同一 CLI 冒烟。链接器使用已安装的 `x86_64-linux-musl-gcc`，只通过该次命令的环境变量指定。
- 按 M19 在 Linux gnu 宿主做一次分批串行全量回归。缺一批或任一批 failed 不为 0，都不算通过。
- 按实测结果更新 `README.md` 与 `README.zh-CN.md` 的构建说明和支持范围。未执行的 macOS 保持未完成，不写成已验证。
- 不改变 SQL、存储格式、错误语义或运行形态。clippy 修复只做保持这些行为的改写。

## Capabilities

### New Capabilities

无。本变更不引入新的产品行为。

### Modified Capabilities

无。数据库需求没有变化，因此 `.openspec.yaml` 设置 `skip_specs: true`。不编造规格来通过校验。若后续 Iteration 必须改变 SQL、存储、错误语义或运行形态，先修订计划并补 delta spec，再实施。

## Impact

- 代码：当前 fmt 差异在 `tests/hash_join_build_side_test.rs` 与 `tests/sort_merge_join_test.rs`。lib 的 15 条 deny lint 位于 `src/cli/lifecycle.rs`、`src/executor/alter_table.rs`、`src/executor/executor_trait.rs`、`src/executor/sort_merge_join.rs`、`src/network/protocol.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`。允许这 5 个 lint 后的全目标 clippy 退出码为 0，其余 6 条警告在 `tests/dml_tombstone_abandon_window_test.rs`、`tests/ddl_rewrite_consistency_test.rs` 和 `tests/sql_differential_test.rs`。
- 文档：`README.md`、`README.zh-CN.md`。
- 依赖与工具链：继续使用当前 `Cargo.lock` 和 stable 工具链。不新增 `rust-toolchain.toml`，不升级依赖，不写全局 Cargo 配置。
- 环境：Linux x86_64 WSL2，rustc 1.99.1。musl 链接器已在 `/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc`。`x86_64-unknown-linux-musl` target 当前未安装，实施时用 `rustup target add` 安装。没有 macOS 执行环境。
- 关联：MS41；ISS36；I073；I093 中仅本批目标实际需要的依赖兼容。约束 M19、以及本次目标对 M20 的承载方式。非目标包括 I052 的预编译矩阵、CI、crates.io 与 man 页。

## 需求

1. `cargo fmt --check` 在当前 stable rustfmt 上退出码为 0。格式化不改变程序逻辑。
2. `cargo clippy --offline --all-targets -- -D warnings` 退出码为 0。修复不改变 SQL 结果、存储字节、错误语义或运行形态。
3. Linux gnu 的 `cargo build --locked --offline --release --bin rtsql` 成功。产物对临时路径执行建表、插入 `1` 和 `SELECT`，退出码为 0，查询输出包含 `{"columns":["id"],"rows":[[1]]}`。
4. `x86_64-unknown-linux-musl` 的静态 release 二进制构建成功，`file` 或 `ldd` 显示静态链接，并在本机完成与第 3 条相同的 CLI 冒烟。
5. 没有 macOS 可执行环境时，macOS 分支记为未完成。不编造运行结果，也不因此阻断第 1–4 条和全量回归。
6. 代码改动完成后，按 M19 分批串行跑一次全量。每一批退出码为 0 且 failed 为 0。缺一批不得声明全量通过。
7. 两份 README 的构建与支持表述和上述实测一致。

## 不做

- 无失败证据的依赖升级，以及新增 `rust-toolchain.toml`。
- CI、预编译发布包、`dist/` 归档、crates.io 发布和 man 页。
- 新 SQL、存储格式或新的运行形态。
- 把交叉编译成功当作目标已运行。
- 为 musl 重编依赖系统 `libsqlite3` 的测试二进制。`rusqlite` 0.31 与 `libsqlite3-sys` 0.28 链接系统 sqlite，不属于产品二进制。
- 安装 macOS SDK、osxcross，或使用 sudo。
- 触犯上述非目标的发现不纳入任务。完成 MS41 所必需、且不触犯非目标的问题，在本 change 内新增 Iteration，不另开 change。

## 场景草图

### 格式门通过

- 前置状态：当前 stable rustfmt 下 `cargo fmt --check` 退出码为 1，差异只在两个测试文件的空白与换行。
- 动作：按 rustfmt 写回这些差异后再检查。
- 结果：`cargo fmt --check` 退出码为 0。
- 失败边界：格式化以外的代码变化不属于本场景。检查仍失败则本场景未完成。

### lint 门通过且行为保持

- 前置状态：`cargo clippy --offline --lib -- -D warnings` 退出码 101，lib 有 15 条 lint。允许这 5 个 lint 后，全目标检查退出码 0，另有 6 条测试警告。
- 动作：按设计中的保持行为改写清除 lib lint，再重复全目标 deny，直到退出码为 0。既有相关测试在改写前后都通过。
- 结果：全目标 deny clippy 退出码为 0。ADD COLUMN 仍以 oneshot 结果为准。sort-merge join 的行多重集合不变。flush 失败时仍从失败页起恢复脏位。
- 失败边界：某条 lint 无法在不改变 SQL、存储、错误或运行形态的前提下清除时停止，回到计划。不升级 `async-trait`，不把 `JoinHandle` 改成 await。

### Linux 本机二进制实际运行

- 前置状态：第 1、2 个场景已通过，工作区仍使用现有 lockfile。
- 动作：构建 gnu release 二进制，对含 `/` 的临时库路径执行 `CREATE TABLE`、`INSERT` 和 `SELECT`。
- 结果：构建和三条命令退出码都为 0。`SELECT` 的 `--format json` 输出包含 `{"columns":["id"],"rows":[[1]]}`。
- 失败边界：构建或冒烟失败时不把文档改成已验证。失败若要求新 SQL、存储格式或新运行形态，停止并回报。

### musl 静态二进制实际运行

- 前置状态：本机有 `x86_64-linux-musl-gcc`，rustup 尚未安装 `x86_64-unknown-linux-musl`。
- 动作：安装该 target。用该链接器和 `+crt-static` 构建 release 二进制，检查静态链接，再执行与 Linux 本机相同的 CLI 冒烟。
- 结果：二进制被判定为静态链接，冒烟退出码为 0，查询输出与 Linux 本机相同。
- 失败边界：只完成编译、没有执行二进制，不算完成。链接失败不自动改依赖版本。target 无法安装或链接器不可用时，该分支记为未完成，不阻断已通过的本机检查。

### macOS 无执行环境

- 前置状态：本机是 Linux x86_64，已安装 target 中没有 Apple 目标，PATH 中没有 Darwin 链接器。
- 动作：实施时重复这个环境检查。
- 结果：仍无执行环境时，Act Response 和 README 写明 macOS 未运行。Linux 与 musl 的完成状态不取决于它。
- 失败边界：不得把未运行写成通过，也不得为此安装交叉 SDK。

### 全量回归

- 前置状态：代码与文档改动已经完成。测试文件集合以执行当时的 `ls tests/*.rs | sort` 为准。
- 动作：`--lib --bins` 单独一批，集成测试按 M19 等分规则分批。批间串行，每批 `--test-threads=1`。
- 结果：每一批退出码为 0，且该批 `test result:` 的 failed 为 0。记录保留各批计数。
- 失败边界：缺一批、某批没有计数，或任一批 failed 不为 0，都不能声明全量通过。不使用一次无过滤的 `cargo test` 代替。

### 文档与结果一致

- 前置状态：上述平台分支各自已有实际结果或明确的未完成记录。
- 动作：更新两份 README 的构建与支持段落。
- 结果：已执行的 Linux gnu 与 musl 命令和结果可在文档中找到。macOS 不被写成本次已验证。
- 失败边界：文档超前于尚未取得的运行结果。

## 假设

1. 已承诺范围以 `.agents/docs/tasks.md` 的 MS41 为准。I073 的「测试套件」由 Linux gnu 上的一次 M19 全量承担。musl 的完成条件是静态产品二进制在本机实际运行，不是把全部测试二进制交叉到 musl。
2. 不固定工具链文件。ISS36 对两个 rustfmt 的对照说明差异来自源码未格式化。当前工具链在格式化后即可满足门禁。
3. 依赖版本保持不变。产品二进制在某个目标上因依赖无法构建或无法运行时，本轮任务停止并回到计划，不由实施自行升级。
4. musl 的「实际运行」指在这台 Linux x86_64 上执行同架构静态二进制。这不是 RISC-V 那种无法在本机执行的交叉产物。
5. 本次目标允许把完成 MS41 所必需的问题留在本 change 里新增 Iteration。这只覆盖 MS41，不覆盖新 SQL、存储格式、新运行形态、CI 或发布矩阵。
6. `~/.cargo/config` 的弃用提示是环境输出。它不作为项目代码的 lint 修复对象，也不通过修改用户主目录来消除。

## 回滚

撤销本分支上的格式、lint、文档改动即可回到基点 `a69fe86bbf08a04b0d0cc62d9183da4ae8a5008b` 的产品行为。不修改数据库文件、安装前缀或全局 Cargo 配置。实施时安装的 rustup target 留在工具链目录，回滚源码不会卸载它。

## Gate 1

- 状态：PASS。
- 批准来源：本次目标对 MS41 范围的授权。这不是跳过 Gate。
- BDD 缺口已扫描。场景缺口由上面的假设承接，没有未记录的实质缺口进入实施。
