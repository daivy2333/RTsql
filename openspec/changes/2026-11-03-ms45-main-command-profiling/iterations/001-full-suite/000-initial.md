# Iteration 001 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `tasks.md` 第 2 节的分批规程。上一轮已接受的主命令行为只被这次全量观察，不在本轮修改。产品改动停在 `2054329`：`src/cli/mod.rs` 的 `run_sql` 在 `RTSQL_PROFILING` 已设置时为进入 plan 与 execute 的语句建立 scope 并调用现有的 `print_timings`；新增 `tests/cli_main_profiling_test.rs`。`src/pipeline.rs` 的既有打印点、import 和诊断渲染没有改动。
- Excluded scope: 产品代码、测试、夹具和构建配置的修改。全量失败后在本轮里修。无过滤的 `cargo test`。默认并行度。重跑已经通过的批。改分批规则。改 `openspec/specs/` 的权威正文。

**Objective**

按下面抄入的分批规程，把默认构建上的一次全量跑完。每一批退出码 0 且 failed 为 0。全部 `--test` 名字与执行时的枚举集合一致，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。

**Background**

上一轮只跑了两条定向测试，没有跑全量。全量预算是整个变更一次，放在这一轮。这一轮只执行验证，不产生新的实现。

**Investigation Facts**

- Starting point: 产品代码以 `2054329` 为准。该提交相对 `b1cec3c` 的产品 diff 只有 `src/cli/mod.rs` 和 `tests/cli_main_profiling_test.rs`。上一轮 Review Result 为 `accepted`，结论写在 `iterations/000-main-command-profiling/000-initial.md`。定向结果不代替本轮全量：`cargo test --offline --test cli_main_profiling_test -- --test-threads=1` 在取样点移动之后为退出码 0、0 failed、10 passed；`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 在取样点移动之前为退出码 0、0 failed、9 passed，该次移动没有碰到这条测试覆盖的代码。两条都来自上一轮 Act Response，本轮仍要把它们所在的文件放进枚举后的批次里实跑。
- Current-State Evidence: 集成测试文件在执行时用 `ls tests/*.rs | sort` 取得。文件名去掉目录和 `.rs` 就是 `--test` 的名字。`tests/cli_main_profiling_test.rs` 已经在工作区里，会进入这次枚举。规划时的 162 和「T1 新增一个文件」都不是通过条件。本轮不改代码，没有新的先行测试。
- Code and Critical Path: 本轮没有代码目标。执行对象是当前分支默认构建的 lib 测试、bin 测试，以及枚举得到的全部集成测试。失败时按失败测试所在的批和文件定位，不在本轮改代码。

**Implementation Guidance**

按 T2 里的规程逐条执行。每批在 Act Response 写批号、完整命令、退出码和 failed 计数。批次划分一并写出。不要另写脚本或封装命令来判定通过。

**Task Definitions**

### T2: 一次分批串行的全量

- Requirement/Scenario: `tasks.md` 的 T2。上一轮主命令行为在默认构建上被整套测试观察一次。本轮不产生新的行为要求。
- Depends on: T1。上一轮 Review Result 为 `accepted`。
- Targets: 无产品代码目标。执行对象是 lib 测试、bin 测试，以及执行时 `ls tests/*.rs | sort` 得到的全部集成测试。
- Current behavior: 本变更还没有跑过全量。上一轮只跑了 `cli_main_profiling_test` 和 `diagnostics_statement_test` 这两条定向命令。
- Required behavior: T1 的 Iteration Review Result 为 `accepted` 之后，按下面的分批方式跑一次全量。这是整个变更的唯一一次全量。不改产品代码。失败时停止，不在本轮里修。

  分批是通过条件的一部分：

  1. 执行时枚举，不把规划时的文件数写进通过条件。集成测试文件来自 `ls tests/*.rs | sort`。规划时这组文件是 162 个；T1 会新增 `tests/cli_main_profiling_test.rs`。这两个数字都不是通过条件。
  2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
  3. 其余文件按该排序每 12 个一批，最后一批可以少于 12。12 取 M19 经验值 8–12 的上沿。文件名去掉目录和 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。
  4. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次使用默认并行度。
  5. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
  6. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
  7. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败，不改并行度，不为了凑通过而重跑其余已通过的批。
- Preserve: 上面 1–7 的分批方式。产品代码、测试、夹具和构建配置。上一轮已接受的主命令结果、错误文本、SQLSTATE，以及 import 旧表和诊断默认输出。`openspec/specs/` 的权威正文。
- Forbidden: 改分批规则。不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。全量批次使用默认并行度。修改产品代码或测试。新增脚本、封装命令或判定层。用 Hash、revision、run-id 或时间顺序充当通过条件。重跑已经通过的批。跳过未跑的批后声明全量通过。本轮把失败修进代码。
- First test: 无先行测试。本轮不改代码。第一批就是第一条验证命令：`cargo test --offline --lib --bins -- --test-threads=1`。
- GREEN condition: 每一批退出码 0 且 failed 为 0，第 5 条对账成立，第 6 条的逐批记录写进 Act Response。
- Verification: 每一批以该条命令的退出码和 failed 计数判定。全量通过等于每一批都通过，并且对账成立。任一批未跑，不得声明全量通过。
- Stop when: 某一批第二次仍失败。对账出现缺失、多余或重复，或者 `--lib` / `--bins` 出现在第一批之外。执行时改了产品代码，或改了上面 1–7 的分批方式。

**Invariants**

- 全量只跑这一次。失败批只按同一命令再跑一次。
- 通过只看退出码、failed 计数和对账结果。
- 上一轮已接受的行为在本轮不改。

**Non-goals**

- 新行为。
- 在本轮修复全量里暴露的失败。
- 补跑已经通过的批。
- 无过滤全量，或默认并行度的全量。

**Acceptance**

1. 每一批退出码 0 且 failed 为 0。任一批未跑，不得声明全量通过。
2. 全部 `--test` 名字的集合等于 `ls tests/*.rs | sort` 去掉目录和 `.rs` 之后的集合，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。
3. 批间串行。每一批都带 `--test-threads=1`。没有一条不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。
4. Act Response 写有每一批的退出码和 failed 计数。
5. 本轮的产品代码和测试没有改动。

**Verification**

| 判定 | 命令 | 通过依据 |
|---|---|---|
| 第一批 | `cargo test --offline --lib --bins -- --test-threads=1` | 退出码 0，failed 为 0 |
| 其余各批 | `cargo test --offline --test <名> ... -- --test-threads=1` | 每一批退出码 0，failed 为 0；名字集合与执行时枚举一致 |

批的划分以执行时的 `ls tests/*.rs | sort` 为准，每 12 个一批。不在本文件里预写文件名单。

**Gate 2 Readiness**

Gate 2: PASS

**Persisted Evidence**

- Mode: none

每一批的命令、退出码和 failed 计数写入 Act Response 即可。这些数字可以在同一工作区重跑。没有哪一条验收必须靠留下来的日志才能判断。全量输出不整份保存。

**Risks and Notes**

- 某一批失败，只说明该批里的测试失败。不把失败当成修改 12 个一批这条规则的理由。
- 枚举里的文件即使在默认构建下编译出 0 个用例，只要退出码 0 且 failed 为 0，该批仍按第 6 条算通过，文件名仍要留在对账集合里。
- 上一轮两条定向测试的通过记录不能填进本轮的批次结果。它们会随 `ls tests/*.rs | sort` 进入对应批次。

## Act Response

- Status: 尚未由 Act 填写

尚未执行。

## Plan Review

- Review Result: pending
