# MS41 构建与兼容验证基线

对应路线：MS41。来源：ISS36、I073、I093 中本批目标实际需要的部分。Gate 1 为 PASS，批准来自本次目标对 MS41 范围的授权，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed。
- 000：执行依据是 `iterations/000-quality-gates/000-initial.md`。Plan Context 为 ready。
- 001：只存在于本文件的 Iteration Plan，尚未展开 Cycle。
- 002：只存在于本文件的 Iteration Plan，尚未展开 Cycle。
- Persisted Evidence：none。
- 无跳过项。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | `cargo fmt --check` 退出码 0。只接受 rustfmt 写回的排版 | pending |
| T2 | 000 | T1 | `cargo clippy --offline --all-targets -- -D warnings` 退出码 0。改写前后列出的行为测试都通过 | pending |
| T3 | 001 | 000 accepted | gnu release 二进制对临时库完成建表、插入和查询 | pending |
| T4 | 001 | T3 | musl 静态二进制在本机完成同一冒烟；环境缺失则该分支记未完成 | pending |
| T5 | 001 | T3、T4 的实际结果 | macOS 无环境则记未完成。两份 README 与已取得的结果一致 | pending |
| T6 | 002 | 001 accepted | M19 全部分批各自 0 failed。缺一批不算通过 | pending |

## 1. 质量检查

- [ ] 1.1 T1：先确认 `cargo fmt --check` 退出码为 1，再执行 `cargo fmt`，然后检查退出码为 0。不手改排版，不改测试断言。
- [ ] 1.2 T2：按 `iterations/000-quality-gates/000-initial.md` 的改写清除 15 条 lib lint 和 6 条测试 lint。改写前行为测试为 GREEN，deny clippy 为 RED。改写后同一组测试为 GREEN，且 `cargo fmt --check` 与 deny clippy 退出码都为 0。

## 2. 目标平台运行

- [ ] 2.1 T3：`cargo build --locked --offline --release --bin rtsql` 退出码 0。`target/release/rtsql --format json <含斜杠的临时库>` 依次执行 `CREATE TABLE t (id INT)`、`INSERT INTO t (id) VALUES (1)`、`SELECT id FROM t`。三条退出码都为 0。查询 stdout 包含 `{"columns":["id"],"rows":[[1]]}`。
- [ ] 2.2 T4：缺失时 `rustup target add x86_64-unknown-linux-musl`。然后执行 `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`。`file` 含 `statically linked`，或 `ldd` 报告不是动态可执行文件。再用 `target/x86_64-unknown-linux-musl/release/rtsql` 重复 T3 的三条命令，通过条件相同。环境缺失或链接器不可用时记录命令与错误，不改依赖，该分支保持未完成。项目或依赖的编译错误则停止并回到计划。
- [ ] 2.3 T5：复查 `uname`、已安装 rustup target 和 Darwin 链接器。没有 macOS 执行环境则在 Act Response 记未完成。按 T3 与 T4 的实际结果更新 `README.md` 和 `README.zh-CN.md` 的构建与支持段落。未运行的平台不写成已验证。

## 3. 全量回归

- [ ] 3.1 T6：代码与文档都落地后，按 Iteration 002 的分批规则跑一次全量。每一批 `cargo test --offline … -- --test-threads=1` 退出码为 0，且该批每行 `test result:` 的 failed 为 0。记录各批计数。缺一批不得记通过。结束后再跑一次 `cargo fmt --check` 与 deny clippy，退出码都为 0。

## Iteration Plan

### Iteration 000：质量检查基线

- Tasks：T1、T2。
- Depends on：None。
- Stable result：当前 stable 上 `cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 都退出码 0。SQL 结果、存储字节、错误语义和运行形态保持测量时的行为。
- Verification boundary：fmt 检查，deny clippy，以及 Cycle 中点名的既有测试。定向测试用默认并行度。不跑全量。全量预算在 002，整个 change 一次。
- Diagnostic boundary：rustfmt 差异与每一条 clippy lint。平台链接、README 和全量失败不属于本轮。
- Non-goals：musl、macOS、README、依赖升级、工具链文件、全量回归。

### Iteration 001：本机与 musl 运行

- Tasks：T3、T4、T5。
- Depends on：000 accepted。
- Stable result：gnu release 二进制已完成 CLI 冒烟。musl 静态二进制已在本机完成同一冒烟，或因环境缺失而记为未完成。macOS 在无执行环境时记为未完成。两份 README 与这些结果一致。
- Verification boundary：真实 `rtsql` 二进制、临时路径和 JSON stdout。静态链接看 `file` 或 `ldd` 的原句。不跑全量，不把 `cargo test --target x86_64-unknown-linux-musl` 当作本轮通过条件。
- Diagnostic boundary：gnu 构建、musl 链接与执行、macOS 环境探测分开记录。某一支失败不回滚已经通过的另一支。依赖不兼容不在本轮改版本。
- Non-goals：发布归档、CI、安装脚本、系统 sqlite 的 musl 化、全量回归。

### Iteration 002：分批全量

- Tasks：T6。
- Depends on：001 accepted。
- Stable result：gnu 宿主上的一次全量，每一批 0 failed，且两条项目门仍为退出码 0。
- Verification boundary：整个 change 的一次全量，只在本轮。执行时运行 `ls tests/*.rs | sort`，设 N 为行数，`B = ceil(N / 12)`。前 `B * 12 - N` 个文件批各 11 个文件，其余文件批各 12 个。文件名去掉目录和 `.rs` 后作为 `--test` 目标。`--lib --bins` 单独作为第一批。其后按该清单顺序跑文件批。批间串行，同一时刻只有一批。每批命令是 `cargo test --offline` 加上该批的 `--lib --bins` 或重复的 `--test <名>`，最后是 `-- --test-threads=1`。覆盖对账要求每个当时的测试文件恰好出现一次。缺一批、某批没有 `test result:` 行，或任一行 failed 不为 0，都不算通过。失败批只整批复核一次；复核通过则记明首跑失败并按通过处理。不新增判定脚本。单次定向测试不使用本约束。历史 passed 总数不是门槛。本轮结束后 `cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 退出码为 0。
- Diagnostic boundary：全量失败的测试名和批次。不在本轮改平台链接命令，除非 001 的验收实际未满足；那是返工，不提前写进本计划。
- Non-goals：第二次全量、musl 测试套件、macOS 补跑、依赖升级。

## 平衡审计

000 把格式和 lint 放在一起，因为 MS41 的检查基线是两条命令同时可满足。拆开后每一轮只剩一次工具输出，不能单独成为后续平台验证所依赖的结果。二者的失败仍然按命令区分。

001 把 gnu、musl 和 macOS 放在一起，交付物是「能执行的目标有运行结果，不能执行的目标有未完成记录，文档与结果一致」。macOS 探测本身不构成独立成果，所以不单独成轮。三支的故障分开记录，musl 的环境缺失不撤销 gnu 冒烟。

002 单独承载全量。平台链接失败和测试失败不是同一个排查范围，而且全量是本 change 的一次高开销预算，要等代码和文档都稳定后再跑。

三轮都有独立验证和停止点。000 没有因为编号靠前就吃进后续平台和全量。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 fmt 退出码 0 | T1 | `cargo fmt --check` | Covered |
| R2 deny clippy 退出码 0 且行为保持 | T2 | `cargo clippy --offline --all-targets -- -D warnings` | Covered |
| R3 gnu 二进制 CLI 冒烟 | T3 | `target/release/rtsql --format json` 三条命令 | Covered |
| R4 musl 静态二进制实际运行 | T4 | musl 二进制的同一组命令，外加 `file` 或 `ldd` | Covered |
| R5 macOS 无环境则未完成 | T5 | `uname`、rustup target、Darwin 链接器复查 | Covered |
| R6 M19 分批全量各自 0 failed | T6 | Iteration 002 的分批 `cargo test` | Covered |
| R7 README 与结果一致 | T5 | 对照 T3、T4、T5 已记录的结果阅读两份 README | Covered |

## Gate 2

PASS。无 BLOCKED。无 SKIPPED。批准来自本次目标对已通过 Gate 2 的计划的授权，不是跳过。当前 Cycle 的 Plan Context 为 ready。
