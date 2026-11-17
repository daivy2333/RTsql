# Iteration 002 / Cycle 000: 一次分批全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `tasks.md` 的 Iteration 002，以及 `.agents/runbooks/m19-batched-full-regression.md` 的等分批次规则。通过条件仍是每一批退出码 0，且该批 `test result:` 行的 failed 为 0。
- Excluded scope: 不新增产品行为。不改代价公式、叶序规则和 `JoinOrder` 文本。不修改 `openspec/specs/` 或 `.agents/`。不把全量再安排到 000 或 001。不预算第二次全量。

**Objective**

按执行时的 `tests/*.rs` 把集成测试等分批次跑完，`--lib --bins` 单独成最后一批。每一批退出码 0，且该批 `test result:` 行的 failed 为 0。这次运行用掉整个变更仅有的一次全量预算。

**Investigation Facts**

- Starting point: 001 的实现提交是 `cb232abcf8309847b785c9ade1f0db9df1d07c57`。合格查询会按估计更换左深叶序，EXPLAIN 在首行 `EXPLAIN` 之后插入 `JoinOrder estimated` 或 `JoinOrder written`。没有可用统计、以及无 ORDER BY、两表、外连接和 LIMIT，保持书写叶序且不插入该行。001 的 Act Response 记录了最终定向结果：`cargo test --offline --test join_reorder_test -- --test-threads=1` 退出码 0，8 passed，0 failed；`cargo test --offline --test table_statistics_test -- --test-threads=1` 退出码 0，12 passed，0 failed；`cargo test --offline --lib table_stats -- --test-threads=1` 退出码 0，14 passed，0 failed，446 filtered out。这三条覆盖到本 Cycle 展开时没有失效。本 Cycle 的计划提交不改产品代码。Act 从该计划提交开始。全量尚未为本次变更运行。
- Current-State Evidence:
  - 展开时 `ls tests/*.rs | wc -l` 的结果是 169。其中 `tests/table_statistics_test.rs` 与 `tests/join_reorder_test.rs` 是 000 和 001 新增的。这个 169 只说明展开时的现场。执行时重新 `ls tests/*.rs | sort`，不沿用 169，也不沿用计划里曾经写过的 167。
  - 等分规则：文件数 `N`，文件批数 `B = ceil(N / 12)`。按字典序，前 `B × 12 − N` 批各 11 个文件，其余各 12 个。`--lib --bins` 是第 `B + 1` 批。展开时的演算是 `N = 169`、`B = 15`、前 11 批各 11 个文件、后 4 批各 12 个文件，加上 lib 与 bin 共 16 批。文件数变化后这组数字作废。
  - 每一批是一次独立的 `cargo test`。批内 `--test-threads=1`。上一批进程退出后才启动下一批。`grep -E "^test result:"` 只用来读出 cargo 已经打印的行。判定看该次 cargo 的退出码，以及这些行里的 failed。
  - `--lib --bins` 一批通常打出 lib 与 bin 各一行 `test result:`。目标数是 `N` 个文件加上 lib 与 bin 两个目标。行数与目标数在这个形态下相同。两个数都写入 Act Response。旧的 2231 passed、143 个目标不是本轮通过条件。
  - 诊断边界仍是本变更的统计、代价和叶序。`tests/self_join_test.rs` 不调用 `ANALYZE`。没有统计的既有连接仍走书写装配。
- Code and Critical Path: 本任务的入口是上面的 `cargo test` 批次，不是新的产品符号。产品行为已经在 `src/table_stats.rs`、`src/pipeline.rs` 的 `plan_stage_impl` 与 `bump_schema_version`，以及 `src/parser/planner/query.rs` 的 `decide_leaf_order` 里。全量只观察这些行为有没有破坏既有测试。

**Implementation Guidance**

先在工作区根目录执行 `ls tests/*.rs | sort`，按上面的公式切批，并核对文件名总数等于 `N`、排序后与 `ls` 的输出一致、没有文件进入两批。对账不成立时不要启动 cargo。

文件批全部跑完后，再跑 `--lib --bins`。某一批失败时，先单独跑失败的那个测试，再把该批原样再跑一次。两次都带 `--test-threads=1`。复核通过则该批记通过，并写明首跑失败。cargo 在批内第一个失败后不会再跑后面的测试二进制，所以复核必须是整批，不能只补后面没跑到的名字就当成整批通过。

复核仍失败，且失败来自统计、代价或叶序时，按 `specs/query-cost-and-join-order/spec.md`、`specs/explain-plan-output/spec.md` 和 `specs/sql-parameter-binding/spec.md` 修正。修正前先看到该失败测试是红的。修正后只重跑这个定向测试，命令带 `--test-threads=1`。不要为此再跑其余批次，也不要再排一次全量。失败与这些行为无关时，不要改其他模块。

**Task Definitions**

### T4: 按等分批次做一次全量

- Requirement/Scenario: 「既有语义零回归」。未收集统计时，既有查询和 EXPLAIN 保持本变更前的结果与节点行。
- Depends on: Iteration 001 accepted
- Targets: 无预定产品文件。只有某一批复核后仍失败，且失败来自统计、代价或叶序时，才修改对应的既有实现。
- Current behavior: 001 的三条定向测试已经通过。全量还没有为本次变更运行。
- Required behavior:
  - 执行时的 `ls tests/*.rs | sort` 与批次中的文件名比对，missing、extra、duplicate 都是 0。
  - 每个文件批的命令是 `cargo test --test <f1> --test <f2> ... -- --test-threads=1`。最后一批是 `cargo test --lib --bins -- --test-threads=1`。
  - 每一批退出码 0，且该批 `test result:` 行的 failed 为 0。Act Response 留下批号、目标、退出码、passed、failed、ignored。
  - 目标数与 `test result:` 行数都写明。lib 与 bin 各占一行时，这两个数相同。
- Preserve: 批次算法、`--test-threads=1` 和「整个变更只跑一次全量」。没有统计的语句不因本轮改动而改变结果。
- Forbidden: 一次无过滤的 `cargo test`；去掉 `--test-threads=1`；并行启动多批；全量失败后再跑一整轮；新增判定脚本或把判定写进测试框架之外的程序；为了对上旧的 passed 数字而改断言；修改 `openspec/specs/` 或 `.agents/`；改与本变更无关的失败。
- First test: 不新增测试。第一批文件批就是本任务的第一项观察。启动前只做 `ls` 与切批对账，不对账成功不跑 cargo。
- GREEN condition: 全部分批各退出码 0，且各批 `test result:` 的 failed 为 0。某一批首跑失败、整批复核通过时，该批按通过记，并写明首跑失败。
- Verification: 输入是仓库里的测试目标，断言是 cargo 的退出码和 `test result:` 行。计数管道只用 `grep -E "^test result:"`。
- Stop when: 批次清单与执行时的文件集合不一致；失败与统计、代价、叶序无关；修正后仍要靠第二次全量才能声称通过。这些情况停止并回到计划。

**Invariants**

- 全量结论等于每一批各自 failed 为 0。缺一批不能写成全量通过。
- 任何测试命令都带 `--test-threads=1`。批与批串行。
- 定向测试不算这次全量预算。返工不得再预算第二次全量。

**Non-goals**

不新增产品行为。不预建缺陷轮次。不把全量拆回 000 或 001。不交换连接的左右输入。

**Acceptance**

- 执行时的测试文件每个正好出现在一个文件批里。
- `--lib --bins` 单独成功一批。
- 每一批退出码 0，failed 为 0。
- Act Response 能按批核对退出码和 passed、failed、ignored。
- 本次变更之后没有第二份全量批次表。

**Verification**

按 Task Definition 的命令串行执行。通过与否只看各次 cargo 的退出码和 `test result:` 行。不使用无过滤的 `cargo test`。不把 001 的三条定向命令在全量之外再跑一遍。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。目标注入同时批准了本轮计划。这不是跳过。

**Persisted Evidence**

- Mode: none

各批退出码和计数写入 Act Response。批次表可以低成本按同一公式重算，但通过结论不能靠再跑一次全量来补。

**Risks and Notes**

- 展开时的 169 和 16 批会过期。执行时以当时的 `ls` 为准。
- 某一批因本变更失败并被定向测试修好之后，其余批次仍是修改前的结果。设计已经接受这个缺口，因为第二次全量不在预算内。
- 与本变更无关的失败不在本轮修。
