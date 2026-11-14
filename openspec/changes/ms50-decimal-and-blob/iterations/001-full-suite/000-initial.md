# Iteration 001 / Cycle 000: 里程碑唯一一次全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `tasks.md` 第四节已经锁定的一次 M19 分批全量。每条 `cargo test` 带 `--test-threads=1`。失败批只把同一条整批命令再跑一次。覆盖执行时本分支工作树上的测试目标。
- Excluded scope: 产品代码和测试代码修改；合并 master；第二次全量；为 `ms50-result-consumption` 再跑一次；改分批公式；用历史 passed 总数做通过条件；全局 tasks、SNAPSHOT、M/R/I 和 `openspec/specs/` 正文。

**Objective**

在本分支当前工作树上按 M19 分批跑完这一次全量。每一批的进程退出码是 0，且该批每一行 `test result:` 的 failed 是 0。`--test` 名字的集合与执行时枚举的集成测试集合一致，无缺失、无多余、无重复。

**Background**

Iteration 000 的返工 Cycle `iterations/000-decimal-blob/001-rework.md` 已 accepted。NUMERIC、字节别名和 BLOB 主键沿用已有 DECIMAL 与 BLOB 规则。全量留在本轮，并且是 MS50 两个变更合计的唯一一次。执行时树上有什么，这一次就覆盖什么。

**Investigation Facts**

- Starting point: 返工 Cycle 的 Act Response 为 reported，HEAD `4948ba553dceb2ad69e0d140375f74cd414c247f`。该 Response 的 `decimal_blob_test` 整文件结论是退出码 0、`8 passed; 0 failed`。accepted Review 确认三项缺口已关闭，并确认其他类型的 `Unsupported value type` 仍由 `build_where` 返回。本轮开始前不重跑那些定向命令。本变更还没有跑过全量。
- Current-State Evidence:
  - 包名是 `rtsql`。`Cargo.toml` 有 `[[bin]]` 目标，`cargo test --lib --bins` 是实在的第一批。
  - 集成测试是工作树根目录下 `tests/*.rs` 的顶层文件。`--test` 的名字是文件名去掉目录和 `.rs`。
  - 规划时在本工作树根目录执行 `ls tests/*.rs | sort`，得到 166 个文件，无重复。`B = (166 + 11) / 12 = 14`。`K = 14 × 12 − 166 = 2`。K 小于 B。前 2 个文件批各 11 个文件，其余 12 个文件批各 12 个文件，规模都在 8 到 12。这只说明当前树不会立刻命中停止条件，不是执行时的 N。
  - `decimal_blob_test` 在这个目录里。对账要求它留在枚举集合中，随所在批执行。
  - 分批规则来自 `.agents/runbooks/m19-batched-full-regression.md` 的等分规则和 `.agents/memory/project-model.md` 的 M19。本 Cycle 已把执行所需的命令、顺序、对账和失败处理写在 T4 里。
- Code and Critical Path: 本轮不改产品代码。入口是工作树根目录的 `cargo test`。第一批覆盖库测试和 bin 测试。其后各批覆盖枚举到的集成测试。没有新的产品符号。

**Implementation Guidance**

先在工作树根目录重新枚举并算出批次，核对停止条件，再按顺序跑命令。上一条进程退出后才开始下一条。同一时刻只有一批。cargo 在一批里遇到第一个失败目标后不再运行该批其余目标，所以失败后的那一次复核必须是同一条整批命令。

**Task Definitions**

### T4: 按 M19 分批跑一次全量

- Requirement/Scenario: `tasks.md` 第四节。本里程碑唯一一次全量，覆盖执行时本分支的树。
- Depends on: Iteration 000 accepted
- Targets: 工作树根目录的 `cargo test`。集成测试清单来自执行时的 `ls tests/*.rs | sort`。第一批目标是 `--lib --bins`。
- Current behavior: 返工 Cycle 的 `decimal_blob_test` 已退出码 0、failed 为 0。本变更还没有跑过全量。
- Required behavior:
  - 任何测试调用都带 `--test-threads=1`。禁止默认并行度。同一时刻只有一批在跑。上一条进程退出后才启动下一条。
  - 执行时重新枚举。在工作树根目录运行 `ls tests/*.rs | sort`。N 是这次输出的行数。不得使用本 Cycle 里写出的 166，也不得把任何文件数写进命令来代替这次枚举。
  - B 是不小于 N/12 的最小整数。正整数除法 `B = (N + 11) / 12` 与它相同。`K = B × 12 − N`。
  - K 大于 B 时停止并回到计划。任一集成测试批的文件数不在 8 到 12 之间时同样停止。不另造切分，也不改成一次无过滤的 `cargo test`。
  - K 小于等于 B 时，按这次排序：前 K 批各 11 个文件，其余各 12 个文件。文件批数为 B。
  - 第一批，且只有第一批，是 `cargo test --lib --bins -- --test-threads=1`。
  - 其后每一批是 `cargo test --test <名> --test <名> ... -- --test-threads=1`。名字是该批文件去掉目录和 `.rs`。
  - 对账在开跑前完成：全部 `--test` 名字的集合等于这次枚举的名字集合，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。对账用这两份名字列表比对，不另写程序。
  - 命令直接运行，退出码是该 cargo 进程的退出码。从该进程输出中以 `test result:` 开头的行读取 failed。可以用一步 `grep -E '^test result:'` 阅读这些行，但不得用管道的退出码代替 cargo 的退出码。`--lib --bins` 会有多行 `test result:`，每一行的 failed 都要是 0。
  - 每一批把进程退出码和 failed 计数写入 Act Response。退出码 0 且每一行 failed 为 0，该批通过。任一批未跑，不得声明全量通过。
  - 某一批退出码非 0 或 failed 非 0 时，只把该批的同一条命令再跑一次。第二次退出码 0 且 failed 为 0，该批通过，并注明首跑失败。第二次仍不满足，则 T4 失败。同一遍里尚未开始的后续批仍按顺序执行，每批仍只允许这一次复核。不重跑已经通过的批，不再开始第二遍全量。
  - 通过与否只看该批 cargo 进程的退出码和 `test result:` 的 failed 计数。
- Preserve: 000 已经交付的产品行为。本轮结束时产品代码和测试代码与开跑前相同。
- Forbidden: 修改产品代码或测试代码；合并 master；使用规划时的文件数；一次不带 `--lib`、`--bins` 或 `--test` 过滤的 `cargo test`；默认并行度；第二遍全量；为对账或计数新增脚本、封装命令或判定层。
- First test: 本任务不改代码，没有先行失败测试。第一批命令就是验证入口。返工 Cycle 的 `decimal_blob_test` GREEN 不在开跑前单独重跑。
- GREEN condition: 这一遍的每一批都是退出码 0，且每一行 `test result:` 的 failed 为 0。失败批至多整批再跑一次，且第二次满足同一条件。对账无缺失、无多余、无重复。没有未跑的批。
- Verification: 按 Required behavior 的命令逐批执行。失败表示该批退出码非 0 或 failed 非 0，或对账不一致，或停止条件被命中。
- Stop when: K 大于 B，或任一集成测试批的文件数不在 8 到 12。要让全量通过就必须改产品代码、改测试、合并 master 或改分批公式时，停止并回到计划。不把失败当成修改公式或再开一遍全量的理由。

**Invariants**

- 这是本变更、也是 MS50 两个变更合计的唯一一次全量。
- 每条测试命令都带 `--test-threads=1`。
- 批间一条命令结束后再开始下一条。同一时刻只有一批。
- 产品代码和测试代码在本轮保持不变。

**Non-goals**

第二次全量、合并 master、改产品代码、改分批公式、用历史 passed 总数做通过条件。

**Acceptance**

- 执行时枚举的每个集成测试名字恰好出现在一个文件批里。
- 第一批是 `--lib --bins`，后续批不再带这两个过滤。
- 每一批退出码 0，且每一行 `test result:` 的 failed 为 0。失败批的一次整批复核若通过，计入通过，并保留首跑失败的记录。
- 未跑完的批不能用来声明全量通过。
- 工作树里的产品代码和测试代码相对开跑前没有改动。

**Verification**

按 T4 的分批命令执行。判定是 cargo 进程的退出码，以及该进程输出中 `test result:` 行的 failed 计数。库测试、bin 测试和集成测试都走项目已有的 `cargo test` 入口。这就是那一次全量。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。

计划批准来自目标原文「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」，不是跳过。检查项已对照本 Cycle、`tasks.md` 第四节、M19 和 runbook 的等分规则。规划时的 166 个文件按同一公式得到 K = 2、B = 14，文件批规模为 11 或 12。执行时仍重新枚举。

**Persisted Evidence**

- Mode: none

各批退出码和 failed 计数写入 Act Response。本轮不把输出存成文件。

**Risks and Notes**

- 规划时的 166 只用于确认当前树不会立刻命中 K 大于 B。执行时的 N 以当时的 `ls tests/*.rs | sort` 为准。
- cargo 在一批的第一个失败目标之后不再运行该批其余目标。复核必须重发同一条整批命令，不能把未运行的目标记成通过。
- 某一批没有退出时，不另开下一批，也不改命令重跑。
- 本轮不合并 master。另一个变更已经合回 master 的结果不在这次覆盖范围内，也不为此再增加一次全量。

## Act Response

- Status: blocked

**实际改动**

没有改产品代码，没有改测试代码。工作区在开跑前和写本 Response 前都是干净的，HEAD 仍是 `354aed6d997a6a1333665ea8d281d9a165a31c13`。本 Response 和 `tasks.md` 的本轮状态是这次要提交的内容。

**分批与对账**

执行时 `ls tests/*.rs | sort` 得到 N = 166，无重复。`decimal_blob_test` 在清单里。B = (166 + 11) / 12 = 14。K = 14 × 12 − 166 = 2。K 不大于 B。前 2 个文件批各 11 个文件，其余 12 个文件批各 12 个文件。规模都在 8 到 12。停止条件里的 K > B 和批规模越界没有命中。

名字列表按这次排序切分后，与枚举清单 `diff` 一致。missing 0，extra 0，duplicate 0。`--lib` 和 `--bins` 只在第一批。

**与计划的偏差**

两处文档措辞和 Cycle 的 Task Definition 不一致。按 Cycle 执行，没有改 B/K 公式。

- `.agents/runbooks/m19-batched-full-regression.md` 的等分批次框把 `--lib --bins` 写成第 B+1 批。同一份 runbook 的操作步骤、M19、Cycle T4 和 `tasks.md` 第四节都把这一批放在最前。第一批是 `cargo test --lib --bins -- --test-threads=1`。
- 同一份 runbook 的失败处理写先单独复核失败用例，再整批复核。Cycle 只允许把该批同一条命令再跑一次。B08 按 Cycle 整批复核了一次，没有单独再跑 `mvcc_abort_test`。

**验证**

15 批都已启动。这是这一次全量，没有第二遍。每条命令都带 `--test-threads=1`。上一条进程退出后才启动下一条。判定是各批 cargo 进程的退出码，以及该进程输出里 `test result:` 汇总行的 failed。`^test result:` 还会命中库测试里名为 `result::...` 的用例行；那两行以 `... ok` 结束，不是汇总行。下面的 failed 只计 `test result: ok.` 或 `test result: FAILED.` 这种汇总行。

| 批 | 退出码 | 汇总行 | 各行 failed | 复核 |
|---|---|---|---|---|
| B00 `--lib --bins` | 0 | 2 | 0, 0 | 无 |
| B01 11 个文件 | 0 | 11/11 | 11 个 0 | 无 |
| B02 11 个文件 | 0 | 11/11 | 11 个 0 | 无 |
| B03 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B04 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B05 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B06 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B07 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B08 12 个文件 | 101，复核仍 101 | 2/12，复核仍 2/12 | 0 然后 1；复核相同 | 同一条命令一次，仍失败 |
| B09 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B10 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B11 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B12 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B13 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |
| B14 12 个文件 | 0 | 12/12 | 12 个 0 | 无 |

B00 汇总行：`446 passed; 0 failed; 0 ignored`，以及 `0 passed; 0 failed; 0 ignored`。

B02 有一行 `126 passed; 0 failed; 2 ignored`，对应 `cli_test`。failed 仍是 0。

B04 第 6 行是 `decimal_blob_test`：`8 passed; 0 failed`。

B08 两次的汇总行都是：

- `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
- `test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out`

失败用例是 `mvcc_abort_test` 的 `test_abort_update_reverts_to_previous_version`。`tests/mvcc_abort_test.rs:162`：`left == right` 不成立，说明是 `v1 should still be visible after tx2 abort - index should point back to v1`。left 是 `None`，right 是 `Some(RowId { page_id: 2, slot_id: 0 })`。两次断言相同。cargo 在这个目标之后停止该批，所以下面 10 个目标两次都没有运行：`mvcc_commit_test`、`mvcc_record_test`、`mvcc_tombstone_visibility_test`、`nested_loop_join_test`、`network_protocol_test`、`network_server_test`、`no_from_select_test`、`order_by_alias_sort_test`、`outer_join_test`、`overflow_page_test`。这 10 个不能记成通过。

已通过的批没有重跑。B09 到 B14 在 B08 失败后仍按顺序各跑了一次，全部退出码 0 且 failed 为 0。

`tests/mvcc_abort_test.rs` 相对 master 没有 diff。本 Cycle 没有改代码去让它通过，也没有为了对照再跑一遍。

**自检**

Gate 3：本轮没有产品 diff。T4 要求的全量通过没有达到。

Gate 4：未通过。B08 两次都不满足退出码 0 且 failed 为 0。

Gate 5：命中。要让全量通过就必须改产品代码或测试。这超出本 Cycle。没有改分批公式，没有合并 master，没有第二遍全量。

**Persisted Evidence**

None required。失败断言可以在上面两行里看完。没有创建 evidence 目录。

**Experience Candidates**

Issue 候选：`test_abort_update_reverts_to_previous_version`。证据是上面的 B08 两次结果。测试文件不在本分支相对 master 的 diff 里。本 Cycle 没有判定它是否由 DECIMAL 与 BLOB 的改动引起，也没有登记。

Runbook 候选：None。

**未解决**

T4 未完成。全量不能声明通过。

**Blocker Handoff**

- 位置：T4，B08 的第一次运行和一次整批复核。Gate 4 未通过，Gate 5 停止。
- 计划预期：每一批退出码 0，且每一行 `test result:` 的 failed 为 0。
- 实际情况：B08 两次退出码都是 101。`mvcc_abort_test` 有 1 failed。该批其余 10 个目标没有运行。其余 14 批退出码 0，failed 为 0。
- 影响：GREEN condition 不成立。不能声明这一次全量通过。
- 任务：对账和 15 批的启动已做完。B08 失败。没有未启动的批。
- 工作区：产品代码和测试代码无改动。
- Evidence：None required。
- 恢复条件：由 Plan 处理这个失败。本 Cycle 不能靠改产品代码、改测试、合并 master 或改分批公式继续。已通过的批不能重跑，也不能把这一次再跑成第二遍全量。
