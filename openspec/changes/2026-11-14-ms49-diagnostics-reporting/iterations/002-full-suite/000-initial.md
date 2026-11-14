# Iteration 002 / Cycle 000: 收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `tasks.md` 第四节已经锁定的一次 M19 分批全量。默认 feature。每条命令带 `--test-threads=1`。失败批只把同一条整批命令再跑一次。
- Excluded scope: 产品代码修改、第二次全量、非默认 feature 组合、改分批公式、基线差额归因、全局 tasks、SNAPSHOT、M/R/I 和 `openspec/specs/` 正文。

**Objective**

在默认构建上按已锁定的分批方式跑完这一次全量。每一批的进程退出码是 0，且该批 `test result:` 行的 failed 是 0。`--test` 名字的集合与执行时枚举的集成测试集合一致，无缺失、无多余、无重复。

**Investigation Facts**

- Starting point: Iteration 001 的 Act Response 已报告 T3。该 Cycle 的 Plan Review 为 accepted。产品代码在 HEAD `2b9ff96`。三条定向 GREEN 沿用该 Act Response，本轮开始前不单独重跑。全量尚未在本变更中运行。这是整个变更的唯一一次全量。
- Current-State Evidence:
  - 包名是 `rtsql`。`Cargo.toml` 有 `[[bin]]` 目标，`cargo test --offline --lib --bins` 是实在的第一批，不是空目标。
  - 集成测试是工作树根目录下 `tests/*.rs` 的顶层文件。`--test` 的名字是文件名去掉目录和 `.rs`。
  - 规划时执行 `ls tests/*.rs | sort`，得到 165 个文件。按下面的公式，B = 14，K = 3。K 小于等于 B。前 3 批各 11 个文件，其余 11 批各 12 个文件，规模都在 8 到 12。这只说明当前树能走通停止条件，不是执行时的 N。
  - 001 已通过的 `diagnostics_comparison_test`、`diagnostics_statement_test`、`diagnostics_facade_test` 都在这个目录里。对账要求它们留在枚举集合中，随所在批执行。
- Code and Critical Path: 本轮不改产品代码。入口是 `cargo test`。第一批覆盖库测试和 bin 测试。其后各批覆盖枚举到的集成测试。没有新的产品符号。

**Implementation Guidance**

先在工作树根目录重新枚举并算出批次，核对停止条件，再按顺序跑命令。批与批之间等上一条退出。cargo 在一批里遇到第一个失败目标后，不再运行该批其余目标，所以失败后的那一次复核必须是同一条整批命令。

**Task Definitions**

### T4: 按 M19 分批跑一次全量

- Requirement/Scenario: `tasks.md` 的 RTM 行「全量只跑一次且按 M19 分批」。
- Depends on: Iteration 001 accepted
- Targets: 工作树根目录的 `cargo test`。集成测试清单来自执行时的 `ls tests/*.rs | sort`。第一批目标是 `--lib --bins`。
- Current behavior: 001 的三条定向命令已经退出码 0、failed 为 0。本变更还没有跑过全量。
- Required behavior:
  - 任何测试调用都带 `--offline` 和 `--test-threads=1`。禁止默认并行度。同一时刻只有一批在跑。
  - 执行时重新枚举。在工作树根目录运行 `ls tests/*.rs | sort`。N 是这次输出的行数。不得使用本 Cycle 里写出的 165，也不得把任何文件数写进命令来代替这次枚举。
  - B 是不小于 N/12 的最小整数。正整数除法 `B = (N + 11) / 12` 与它相同。`K = B × 12 − N`。
  - K 大于 B 时停止并回到计划。任一集成测试批的文件数不在 8 到 12 之间时同样停止。不另造切分，也不改成一次无过滤的 `cargo test`。
  - K 小于等于 B 时，按这次排序：前 K 批各 11 个文件，其余各 12 个文件。文件批数为 B。
  - 第一批，且只有第一批，是 `cargo test --offline --lib --bins -- --test-threads=1`。
  - 其后每一批是 `cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。名字是该批文件去掉目录和 `.rs`。
  - 对账在开跑前完成：全部 `--test` 名字的集合等于这次枚举的名字集合，无缺失、无多余、无重复。`--lib` 和 `--bins` 只出现在第一批。对账用这两份名字列表比对，不另写程序。
  - 每一批把进程退出码和 failed 计数写入 Act Response。`--lib --bins` 会有多行 `test result:`，每一行的 failed 都要是 0。退出码 0 且 failed 为 0，该批通过。任一批未跑，不得声明全量通过。
  - 某一批退出码非 0 或 failed 非 0 时，只把该批的同一条命令再跑一次。第二次退出码 0 且 failed 为 0，该批通过，并注明首跑失败。第二次仍不满足，则 T4 失败。同一遍里尚未开始的后续批仍按顺序执行，每批仍只允许这一次复核。不重跑已经通过的批，不再开始第二遍全量。
  - 诊断输出不当正确性判据。通过与否只看该批测试进程的退出码和 failed 计数。
- Preserve: 001 已经交付的产品行为。本轮结束时产品代码与开跑前相同。
- Forbidden: 修改产品代码或测试代码；使用规划时的文件数；一次不带 `--lib`、`--bins` 或 `--test` 过滤的 `cargo test`；默认并行度；第二遍全量；非默认 feature；为对账或计数新增脚本、封装命令或判定层。
- First test: 本任务不改代码，没有先行失败测试。第一批命令就是验证入口。001 的三条 GREEN 不在开跑前单独重跑。
- GREEN condition: 这一遍的每一批都是退出码 0 且 failed 为 0。失败批至多整批再跑一次，且第二次满足同一条件。对账无缺失、无多余、无重复。没有未跑的批。
- Verification: 按 Required behavior 的命令逐批执行。失败表示该批退出码非 0 或 failed 非 0，或对账不一致，或停止条件被命中。
- Stop when: K 大于 B，或任一集成测试批的文件数不在 8 到 12。要让全量通过就必须改产品代码、改测试或改分批公式时，停止并回到计划。不把失败当成修改公式的理由。

**Invariants**

- 这是本变更唯一一次全量。
- 每条测试命令都带 `--test-threads=1`。
- 批间一条命令结束后再开始下一条。
- 产品代码在本轮保持不变。

**Non-goals**

第二次全量、改产品代码、非默认 feature 组合、改分批公式、用历史 passed 总数做通过条件。

**Acceptance**

- 执行时枚举的每个集成测试名字恰好出现在一个文件批里。
- 第一批是 `--lib --bins`，后续批不再带这两个过滤。
- 每一批退出码 0 且 failed 为 0。失败批的一次整批复核若通过，计入通过，并保留首跑失败的记录。
- 未跑完的批不能用来声明全量通过。
- 工作树里的产品代码相对开跑前没有改动。

**Verification**

按 T4 的分批命令执行。判定是 cargo 进程的退出码，以及该进程输出中 `test result:` 行的 failed 计数。语句和库测试都走项目已有的 `cargo test` 入口。这就是那一次全量。

**Gate 2 Readiness**

Gate 2：PASS。无 BLOCKED。无 SKIPPED。

批准来源是 goal「很好，按你推荐的来就行，默认全局最优」对 `tasks.md` 已写 T4 范围的确认。检查项已对照本 Cycle、`tasks.md` 第四节、design 里「全量只放在最后一轮」和提案里的一次 M19 分批通过，不是跳过。规划时的 165 个文件按同一公式得到 K = 3、B = 14，文件批规模为 11 或 12。执行时仍重新枚举。

**Persisted Evidence**

- Mode: none

各批退出码和 failed 计数写入 Act Response。本轮不把输出存成文件。

**Risks and Notes**

- 规划时的 165 只用于确认当前树不会立刻命中 K 大于 B。执行时的 N 以当时的 `ls tests/*.rs | sort` 为准。
- cargo 在一批的第一个失败目标之后不再运行该批其余目标。复核必须重发同一条整批命令，不能把未运行的目标记成通过。
- 某一批没有退出时，不另开下一批，也不改命令重跑。

## Plan Review

- Review Result: pending

## Act Response

- Status: reported

**实际改动**

没有产品代码或测试改动。本 Cycle 只更新 change 内的 `tasks.md`（T4 完成，Iteration 002 的 Act Response 为 reported，Review Result 仍为 pending）和本文件的 Act Response。

**与计划的偏差**

无。

**枚举与分批**

执行时在工作树根目录运行 `ls tests/*.rs | sort`。N = 165。B = (N + 11) / 12 = 14。K = B × 12 − N = 3。K 小于等于 B。前 3 个文件批各 11 个文件，其余 11 个文件批各 12 个文件，都在 8 到 12。文件批数为 14。加上第一批 `--lib --bins`，共 15 批。

**对账**

通过。全部 `--test` 名字集合等于这次枚举的 165 个名字，无缺失、无多余、无重复。名字按这次排序切分：前 33 个名字分入 3 个 11 文件批，后 132 个名字分入 11 个 12 文件批。`--lib` 和 `--bins` 只出现在第一批。`diagnostics_comparison_test`、`diagnostics_facade_test`、`diagnostics_statement_test` 都在枚举集合中，位于第 5 批。

**自检**

Gate 3：PASS。分批命令、串行、`--offline` 和 `--test-threads=1` 与 Task Definition 一致。开跑前产品代码 diff 为空，结束后仍为空。没有新增判定脚本。
Gate 4：PASS。15 批都已运行，退出码都是 0，每一行汇总 `test result:` 的 failed 都是 0。
Gate 5：未命中。
没有遗留问题。

**验证**

没有批次复核。每一行下面的 failed 都是该行汇总 `test result:` 的 failed 计数。

- 批 1：`cargo test --offline --lib --bins -- --test-threads=1`。退出码 0。failed：0，0。
- 批 2：11 个 `--test`，从 `aggregate_extension_test` 到 `cache_perf_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0。
- 批 3：11 个 `--test`，从 `catalog_bounded_read_test` 到 `composite_index_lifecycle_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0。
- 批 4：11 个 `--test`，从 `composite_index_runtime_test` 到 `database_file_lock_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0。
- 批 5：12 个 `--test`，从 `datetime_function_test` 到 `diagnostics_statement_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 6：12 个 `--test`，从 `diagnostics_wait_test` 到 `file_header_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 7：12 个 `--test`，从 `file_storage_io_test` 到 `index_range_routing_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 8：12 个 `--test`，从 `index_read_bounded_read_test` 到 `limit_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 9：12 个 `--test`，从 `memory_db_test` 到 `overflow_page_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 10：12 个 `--test`，从 `param_binding_test` 到 `prepared_statement_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 11：12 个 `--test`，从 `projection_expression_test` 到 `recovery_wal_convergence_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 12：12 个 `--test`，从 `release_durability_precondition_test` 到 `set_operations_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 13：12 个 `--test`，从 `sort_merge_join_test` 到 `subset_insert_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 14：12 个 `--test`，从 `sync_loader_test` 到 `version_chain_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。
- 批 15：12 个 `--test`，从 `view_test` 到 `write_type_conformance_test`。退出码 0。failed：0，0，0，0，0，0，0，0，0，0，0，0。

产品代码相对开跑前没有改动。`git status` 与 `git diff` 只含本 change 的 `tasks.md` 和本 Cycle 文件。

**Persisted Evidence**

None required。

**Experience Candidates**

None

**未解决问题**

无。
