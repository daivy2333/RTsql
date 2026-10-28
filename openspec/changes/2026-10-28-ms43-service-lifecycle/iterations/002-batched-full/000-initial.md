# Iteration 002 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: Iteration 000 与 001 已经落地的刷盘所有者和会话接线。`tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen` 保持断言重开后仍查到 `id = 7`。原「未提交行在 `close` 后不可见」保持撤回，不在本 Cycle 补上，也不记成已经满足。`tasks.md` 第 3 节的分批合同。`.agents/memory/project-model.md` 的 M19。
- Excluded scope: 产品代码、测试代码、`Cargo.toml`、`Cargo.lock`。第二次全量。非默认 feature 组合。新平台。诊断门面。把该行改回不可见。把函数名里的 `absent` 当成必须改名的验收。预建缺陷修复轮。`cargo fmt`、`cargo clippy`、bench、example、doctest。

**Objective**

在默认构建上按下面的分批规则跑完一次全量。每一批的 cargo 退出码为 0，且该批每一行 `test result:` 的 failed 为 0。一批都不能少。`--test` 名字的集合与执行当时的枚举集合相同，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。

**Background**

001 的 `002-replan.md` 已 accepted。那一轮只把 `uncommitted_row_absent_after_reopen` 改成断言重开后仍查到该行，并跑过 `cargo test --offline --test session_lifecycle_test`。那条定向结果不能代替本轮全量。全量预算是整个 change 的一次，留在本 Iteration。

**Investigation Facts**

- Starting point:
  - 工作树 `/home/daivy/projects/RTsql-changes/2026-10-28-ms43-service-lifecycle`，分支 `changes/2026-10-28-ms43-service-lifecycle`。展开本文件前 HEAD 为 `cfb6c076d802b6fb4b05829b9dff646dc25714a8`，工作区干净。
  - 001 Cycle 002 的 Act Response 为 reported，就在该提交。其 Review Result 为 accepted，与本文件同一批写入。Next Iteration 指向本文件。
  - 该提交的产品 diff 只有 `tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen`：断言改为 `rows == vec![vec![rtsql::Value::Int(7)]]`，失败文案改为重开后应查到该行。没有改 `src/`，也没有改 `flushed_uncommitted_insert_does_not_resurrect`。
  - 该 Act Response 的 GREEN：`cargo test --offline --test session_lifecycle_test` 退出码 0，`7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`。本 Cycle 不重跑这条，也不把它写成全量通过。
- Current-State Evidence:
  - 2026-10-28 在本工作树读取 `Cargo.toml`：包名 `rtsql`。没有 `[lib]`，没有 `[[test]]`。库入口是 `src/lib.rs`。`[[bin]]` 一条，`name = "rtsql"`，`path = "src/main.rs"`，`required-features = ["cli"]`。没有 `src/bin/`。默认 features 是 `cli`、`network`、`crypto`。因此不额外传 feature 时，`--lib` 测这个库，`--bins` 含这个二进制。
  - 集成测试目标名是 `tests/*.rs` 的文件主名。`tests/support/` 与 `tests/fixtures/` 不是 `tests/*.rs`，不要把它们当成 `--test` 目标。
  - 有多条 `[[bench]]`，且 `harness = false`。它们不进入本 Cycle 的 `cargo test`。
  - 规划时 `ls tests/*.rs | wc -l` 为 159。主名都是 `[a-z0-9_]`，没有重复。159、按 12 个一批得到的调用次数，以及任何历史 passed 总数，都不是通过门槛。执行时重新枚举。
  - M19：全量按 `--lib` / `--bins` 与各集成测试文件分批；批间顺序执行，同一时刻只有一批；每批 `--test-threads=1`。禁止一次无目标过滤的全量调用。禁止用默认并行度跑任何全量批次。定向测试可以用默认并行度，该例外不适用于本 Cycle。
  - `tasks.md` 的 T3 把批大小定为 12。最后一批可以少于 12。不采用「前面若干批 11 个、后面若干批 12 个」的切法。
- Code and Critical Path:
  - 本任务不改产品符号。入口是在工作树根目录执行的 `cargo test`。
  - 第一次调用只带 `--lib --bins`。其后每次调用只带该批的若干 `--test <主名>`。
  - 主名来自执行当时的 `ls tests/*.rs | sort`：去掉目录和 `.rs`。清单按这个排序连续切片，不重排，不交叉。
  - 库测试在 `src/` 内。二进制测试在 `src/main.rs`。集成测试的 crate 根是对应的 `tests/<主名>.rs`。

**Implementation Guidance**

在工作树根目录执行。不传 `--target`、`--features`、`--all-features`、`--no-default-features`。

先取清单，再切批，对账通过之后才跑第一批。清单是 `ls tests/*.rs | sort` 的全文。文件数是它的行数。主名是每行去掉 `tests/` 和 `.rs` 之后的文本。

文件批按清单顺序连续切：除最后一批外，每批恰好 12 个主名；最后一批是 1 到 12 个。文件数能被 12 整除时，每一批都是 12 个，没有更短的一批。出现空批、某一非最后一批不是 12 个、最后一批是 0 个或大于 12 个、主名重复，或主名集合与清单不一致时，停止，不要换成别的切法，也不要开始 `cargo test`。

调用顺序固定。第一次是 `--lib --bins`，命令里不带 `--test`。文件批接在后面。同一时刻只跑一批：上一条命令结束，退出码和该次输出里的 `test result:` 行都记下之后，才开始下一条。不要后台启动，不要并行开多条 `cargo test`。

不要用管道替换 cargo 的退出码。先取得该次 cargo 的退出码，再从该次输出读取 `test result:` 行。摘行命令的退出码不是该批结果。

某一批失败时，用同一条命令把该批整批再跑一次。不能改成只过滤已经打印失败的用例，也不能把该次没打印到的目标记成通过。复核通过则在 Act Response 写明首跑失败，并继续下一批。复核仍失败则停止：不跑后续批次，不重跑已经通过的批，不改 `--test-threads`，不改产品文件。

全部批次按通过条件通过之后，再执行一次 `ls tests/*.rs | sort`。输出必须与开跑前的清单逐行相同。不同则本任务失败，不要按新清单再跑一轮全量。

**Task Definitions**

### T3: 默认构建上的一次分批全量，每批 failed 为 0

- Requirement/Scenario: `tasks.md` 的 T3。M19 的全量分批约束。design 里「Iteration 002 按 M19 把全量分批跑一次」。
- Depends on: 001 accepted。
- Targets: `src/lib.rs` 所代表的库测试，`src/main.rs` 所在二进制的测试，以及执行当时 `tests/*.rs` 的每个集成测试目标。不修改这些文件。
- Current behavior: 本分支还没有按本 Cycle 的命令跑过这组分批全量。001 只跑过 `cargo test --offline --test session_lifecycle_test`，退出码 0，`7 passed; 0 failed`。那不是全量。
- Required behavior:
  - 工作目录是本工作树根目录。用执行当时的 `ls tests/*.rs | sort` 按 Implementation Guidance 切批。每个文件的主名恰好进入一个文件批。
  - 第一批命令是 `cargo test --offline --lib --bins -- --test-threads=1`。
  - 每个文件批的命令是 `cargo test --offline --test <主名> --test <主名> ... -- --test-threads=1`。参数顺序与清单切片顺序相同。除最后一批外每批 12 个 `--test`；最后一批可以少于 12，但不能是 0。
  - 批间一条命令结束后再开始下一条。同一时刻只有一批。
  - 每一批的 cargo 退出码为 0。该批至少有一行 `test result:`。每一行里的 failed 都是 0。
  - 对账：全部 `--test` 名字的集合等于开跑前的枚举集合，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。结束后的 `ls tests/*.rs | sort` 与开跑前逐行相同。
  - Act Response 记下每一批的命令、cargo 退出码，以及该批全部 `test result:` 原行。开跑前的文件数和切批结果也记下。这些数字不是门槛。
- Preserve: `--offline` 与 `-- --test-threads=1`。批间串行。001 已经落地的产品行为，包括重开后仍查到 `id = 7` 的那一行。原「关闭后不可见」保持撤回，本轮通过不把它记成已经满足。
- Forbidden:
  - 一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`，包括 `cargo test --offline` 和 `cargo test --offline -- --test-threads=1`。
  - 全量批次省略 `--test-threads=1`，或用默认并行度跑任何一批。
  - 同时启动两批。
  - 把全部集成测试放进同一次调用。
  - `--target`、`--doc`、`--examples`、`--benches`、`--all-targets`、`--release`、`--features`、`--all-features`、`--no-default-features` 出现在这些 `cargo test` 里。
  - 修改 `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`。为了让某一批通过而改断言、改产品代码，或把未提交行改回不可见。
  - 新增仓库内脚本、封装命令，或用脚本退出码代替 cargo 退出码。把规划时的文件数 159 或某次调用次数写成固定门槛。
  - 失败后只重跑单个用例、只重跑该批的一部分，或重跑已经通过的批来凑通过。第二次仍失败后再改并行度。
  - 第二次全量。非默认 feature 组合。新平台。诊断门面。
  - `cargo fetch`、`cargo update`、改依赖锁。
- First test: 本任务在观察结果之前不改产品文件。先行观察就是第一批 `cargo test --offline --lib --bins -- --test-threads=1`。规划阶段没有跑它，所以这里没有预先看到的 GREEN 或 RED。001 的 `session_lifecycle_test` 结果不能当作这一批的观察。预期该批退出码 0，且每一行 `test result:` 的 failed 为 0。
- GREEN condition: Required behavior 全部成立。任一批没跑，不得写成全量通过。
- Verification: 每一批的 cargo 退出码和该次全部 `test result:` 原行。开跑前与结束后的 `ls tests/*.rs | sort`。对账结果：`--test` 集合与枚举集合相同，`--lib` / `--bins` 只在第一批。任一批没跑、某批没有 `test result:` 行、任一行 failed 不是 0，或 cargo 退出码不是 0，都表示全量没有通过。
- Stop when:
  - 清单为空，或切批不满足「非最后一批恰好 12 个、最后一批为 1 到 12 个、主名无重复且集合与清单一致」：停止，不开始 `cargo test`，回到计划。
  - 一批的 cargo 退出码不是 0，或没有 `test result:` 行，或任一行 failed 不是 0：用同一条命令把该批整批再跑一次。复核通过则写明首跑失败，并继续下一批。复核仍不满足：停止。不跑后续批次，不重跑已通过的批，不改产品文件，回到计划。记录该批命令、两次退出码、两次的 `test result:` 原行，以及输出里能看到的失败测试名。
  - `--offline` 因缓存缺失不能编译：停止。不拉取依赖，不改锁文件，回到计划。
  - 结束时的 `ls tests/*.rs | sort` 与开跑前不一致，或某个主名没有恰好出现一次：停止，不按新清单再跑一轮，回到计划。
  - `src/`、`tests/`、`Cargo.toml` 或 `Cargo.lock` 出现本 Cycle 引入的改动：停止，不把这些改动当成通过，回到计划。

**Invariants**

- 全量结论只来自上述分批。缺一批就不能写成通过。
- 批间串行，批内 `--test-threads=1`。失败批只整批复核一次。
- 不改变 SQL 结果、页格式、错误语义，以及 001 已经写好的重开可见断言。
- 不把「未提交行在 `close` 后不可见」写成已经满足。
- 不把 `session_lifecycle_test` 的 7 passed 写成全量通过。
- 已通过的批次不因为后面某一批失败而重跑。

**Non-goals**

- 第二次全量，非默认 feature 组合，新平台，诊断门面，bench。
- 在本 Cycle 里修改产品代码或测试来修失败。复核后仍失败时，回到计划。
- 改 `uncommitted_row_absent_after_reopen` 的函数名。
- 用文件数 159 或某次调用次数作为门槛。

**Acceptance**

- 开跑前的每个 `tests/*.rs` 恰好出现在一个文件批里。除最后一批外每批 12 个；最后一批可以少于 12。`--lib --bins` 有单独的第一批，且不出现在文件批里。
- 每一批 cargo 退出码为 0，该批每一行 `test result:` 的 failed 为 0，且该批至少有一行。
- 结束后文件清单与开跑前一致。`--test` 名字无缺失、无多余、无重复。
- 本 Cycle 不改 `src/`、`tests/`、`Cargo.toml`、`Cargo.lock`。
- 原「关闭后不可见」仍是撤回，不是本轮补上的验收。

**Verification**

- 全量：真实执行上面的 `cargo test --offline` 命令。通过依据是每条命令的退出码，以及该次输出中每一行 `test result:`。这是默认构建上的测试入口。
- 对账：开跑前和结束后的 `ls tests/*.rs | sort` 逐行相同。`--test` 主名与第一次清单一一对应。`--lib` 和 `--bins` 只在第一批。
- 不跑无过滤全量，不用默认测试并行度，不跑第二次全量。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。

计划审查通过。RTM 里 T3 仍是 Covered，无 Missing，无 Simplified。本 Cycle 不改行为、接口和错误语义，与 `tasks.md` 第 3 节、design 的 Iteration 002 和 M19 一致。分批、批间串行、批内 `--test-threads=1`、对账和失败批只整批复核一次都写进了 Task Definition。Persisted Evidence 为 `none`。批准来自本目标在 001 accepted 之后展开 Iteration 002 的授权，不是跳过。T3 的通过条件没有在本 Cycle 里改写。

**Persisted Evidence**

- Mode: none

每一批的退出码、failed 计数和 `test result:` 原行写入 Act Response 即可。这些行就是通过或失败的可观察结果。不保存完整测试输出，也不另建 evidence 目录。

**Risks and Notes**

- 规划时文件数是 159。若执行时仍是 159，文件批是 13 批各 12 个，再加 1 批 3 个，连同最前面的 `--lib --bins` 共 15 次调用。执行时文件数若变化，按当时的清单重切。重切不满足 12 与「最后一批 1 到 12」时，停止条件已经要求回到计划。159 和 15 不是门槛。
- `test result:` 行里的 ignored 可以不是 0。通过条件只看 failed 是不是 0，以及 cargo 退出码。
- `--lib --bins` 一次调用会为库和二进制各写出 `test result:` 行。文件批通常一个 `--test` 一行。行数是否等于某个旧总数，不是通过条件。
- 失败批只整批复核一次。复核通过后继续，不重跑已经通过的批次。
- 函数名 `uncommitted_row_absent_after_reopen` 仍含 `absent`。本 Cycle 不改它。全量若在这条用例上失败，按停止条件回到计划，不把断言改回「行不存在」。

## Act Response

- Status: 待 Act 填写

## Plan Review

- Review Result: pending

**Findings**

尚未审查。

**Follow-up Decision**

尚未审查。

**Next Cycle**

None

**Next Iteration**

None
