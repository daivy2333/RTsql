# 查询代价与 JOIN 重排

对应路线：`.agents/docs/tasks.md` 的 MS51。Gate 1：PASS，依据在 `proposal.md` 的 Gate 1 节。目标注入同时批准了需求和已记录的方案。这不是跳过。本文件不修改全局路线、改进台账或 `openspec/specs/`。

Gate 2：PASS。无 BLOCKED，无 SKIPPED。000、001 与 002 的 Review Result 为 `accepted`。002 的结论在 `iterations/002-batched-full-regression/000-initial.md`。

M23 只覆盖实施中新发现的实质缺陷。发现之后再追加轮次。本计划不预建。

全量预算：整个变更一次 M19 分批全量，只放在 Iteration 002。000 与 001 不追加。返工和重新计划不再预算第二次全量。定向测试不算这一次全量。用户原话：「本次最多一次全量」。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格增量在 `specs/query-cost-and-join-order/spec.md`、`specs/explain-plan-output/spec.md`、`specs/sql-parameter-binding/spec.md`。
- 000：执行依据是 `iterations/000-session-statistics/000-initial.md`。Plan Context 为 ready。Act Response 为 reported，Review Result 为 `accepted`。
- 001：已完成。执行依据是 `iterations/001-estimated-leaf-order/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 `accepted`。
- 002：已完成。执行依据是 `iterations/002-batched-full-regression/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 `accepted`。
- Persisted Evidence：none。
- 跳过项：无。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | `ANALYZE TABLE` 交回可见行的计数，失败不留统计，成功后缓存清空且版本戳加一。连接叶序不变 | 已完成 |
| T2 | 000 | T1 | 左深代价函数给出规格里的 506、408、90、80，以及平手、9 表和溢出的决定 | 已完成 |
| T3 | 001 | 000 accepted | 合格查询按估计换叶序，结果与未统计时的多重集合和 ORDER BY 一致。不合格查询的节点行不变 | 已实施 |
| T4 | 002 | 001 accepted | 按 M19 等分批次做一次全量，每批退出码 0 且 failed 为 0 | 已完成 |

## Iteration Plan

### Iteration 000: 会话统计与左深代价

- Tasks: T1, T2
- Depends on: None
- Stable result: 一次打开的数据库可以统计基表，并在失败时保持原统计和版本戳。代价函数对规格中的输入返回固定叶序或「不足以重排」。连接计划仍是书写叶序。
- Verification boundary: 只跑 T1、T2 的定向命令，都带 `--test-threads=1`，库测试与 `tests/table_statistics_test.rs` 分成两次 `cargo` 调用、依次执行。不跑全量。全量留在 002，因为用户原话是「本次最多一次全量」。本轮返工不得改成全量。
- Diagnostic boundary: `ANALYZE` 的计划与执行、会话统计的保存和失效、`src/table_stats.rs` 的计数与代价。连接执行器、页格式、目录尾段和构建侧交换不在本轮排查范围。
- Non-goals: 不改连接叶序，不输出 `JoinOrder` 行，不跑 M19 全量。

平衡：统计的收集、拒绝和失效是同一条语句路径上的一个结果。代价函数不读表，但 T3 要同时依赖它和统计快照，所以放在同一轮把它的整数定下来。重排放到 001，因为列序映射是另一个故障域。

### Iteration 001: 按估计更换叶序

- Tasks: T3
- Depends on: Iteration 000 accepted
- Stable result: 规格里的星型查询在统计之后把 `fact` 放在左叶，结果行与未统计时相同。无 ORDER BY、两表、外连接和 LIMIT 仍是书写叶序，且没有 `JoinOrder` 行。
- Verification boundary: 依次执行 `join_reorder_test`、`table_statistics_test` 和库内 `table_stats` 过滤，每次都带 `--test-threads=1`。不跑全量。本轮返工不得加入全量。
- Diagnostic boundary: `build_from_clause_with_projection` 的叶序替换、最外层列序、`EXPLAIN` 的 `JoinOrder` 行。统计的计数公式和页 I/O 不在本轮重开。
- Non-goals: 不交换单个连接的左右输入来选择构建侧。不重排不合格查询。不改 `sort_unstable_by`。不跑全量。

平衡：叶序、列序和结果一致性要在同一次查询里一起看。拆成「只改 EXPLAIN」和「再改执行」会把同一个错误分成两轮。本轮工作量集中在这一处替换。

### Iteration 002: 一次分批全量

- Tasks: T4
- Depends on: Iteration 001 accepted
- Stable result: 执行时实际测试目标按 M19 等分批次各失败 0。本变更只使用这一次全量预算。
- Verification boundary: 按 `.agents/runbooks/m19-batched-full-regression.md` 的等分批次规则跑一次。文件批用 `B = ceil(N / 12)`，前 `B×12−N` 批各 11 个文件，其余各 12 个，`--lib --bins` 单独成一批。每批 `cargo test … -- --test-threads=1`，批间串行。禁止一次无过滤的 `cargo test`，禁止默认并行度。计划时 master 上有 167 个 `tests/*.rs`。000 与 001 会各增加一个测试文件，执行时以当时的 `ls tests/*.rs | sort` 重新生成清单，不沿用 167。返工不得再跑第二次全量。定向测试不算这次预算。
- Diagnostic boundary: 本变更的统计、代价和叶序。与这些无关的失败停止并报告，不在本轮改别的模块来换取全量通过。
- Non-goals: 不新增产品行为。不预建缺陷轮次。不把全量再安排到 000 或 001。

平衡：全量是收尾验证，不和叶序实现混在一轮。失败时的排查范围是整库，所以它自己成为一轮。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 会话内基表统计 | T1 | `analyze_counts_visible_rows` | Covered |
| 失败的统计不留下数字 | T1 | `analyze_missing_table` | Covered |
| 统计替换使已有计划失效 | T1, T3 | `analyze_clears_plan_cache` | Covered |
| 左深连接的代价 | T2 | `star_join_prefers_fact_on_the_left` | Covered |
| 估计只更换合格查询的叶序 | T3 | `star_join_explain_puts_fact_first` | Covered |
| EXPLAIN SELECT 计划树文本输出 | T3 | `star_join_explain_puts_fact_first` | Covered |
| 既有语义零回归 | T4 | M19 各批 `test result:` 的 failed 为 0 | Covered |
| 可复用预编译语句 | T3 | `prepared_join_matches_direct_after_analyze` | Covered |

没有 Simplified。没有 Missing。

## 1. 会话统计（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-session-statistics/000-initial.md` 的 T1 执行。先写会失败的测试，再实现 `ANALYZE`。本任务结束时连接叶序仍是书写顺序。

## 2. 左深代价（Iteration 000）

- [x] 2.1 T2：按同一 Cycle 的 T2 执行。先写会编译失败的代价测试，再实现函数。计划器在本任务里不调用它。

## 3. 叶序替换（Iteration 001）

001 展开之前不实施。展开时把下面的内容写入该轮的 Task Definition，不再选择另一套叶序或权重。

- [x] 3.1 T3：合格查询调用 000 已经定下的代价函数并替换左深叶序。

当前行为：000 完成后，`ANALYZE` 能保存统计，但 `build_from_clause_with_projection` 仍按书写顺序装配。EXPLAIN 没有 `JoinOrder` 行。

完成后的行为与 `specs/query-cost-and-join-order/spec.md` 的「估计只更换合格查询的叶序」一致，并带上该规格里星型数据的这些观察：

- 三张表都统计之后，`SELECT fact.id, d1.id, d2.id FROM d1 JOIN fact ON d1.id = fact.d1 JOIN d2 ON fact.d2 = d2.id ORDER BY fact.id, d1.id, d2.id` 的 EXPLAIN 含有一行，去掉缩进后等于 `JoinOrder estimated`。第一条以 `Scan ` 开头的行以 `Scan fact` 开头。结果行与同一数据上从未统计的会话相同。
- `SELECT *` 使用同一 FROM、ON 和 `ORDER BY fact.id` 时，表头列序与未统计时相同，行也相同。
- 加上 `WHERE fact.id = 1` 时，只返回这一行，且与未统计时相同。
- `GROUP BY fact.d1 ORDER BY fact.d1` 的分组和 `COUNT(*)` 与未统计时相同。
- 去掉 ORDER BY、只连接 `d1` 与 `fact`、改成 `LEFT JOIN`，或加上 `LIMIT 1` 时，没有 `JoinOrder` 行，第一条扫描仍是书写顺序的左表。带 LIMIT 的那一行与未统计时相同。
- 九张已统计基表的等值链，EXPLAIN 含有 `JoinOrder written`，第一条扫描仍是书写顺序的左表。
- 最小代价等于书写顺序时，EXPLAIN 含有 `JoinOrder estimated`，第一条扫描仍是书写顺序的左表。
- 预编译的星型查询在三张表统计之后执行，结果与直接执行相同。
- 星型查询已经出现 `JoinOrder estimated` 之后，成功 `CREATE INDEX`，再 EXPLAIN 时没有 `JoinOrder` 行。
- 关闭再打开之后，同一 EXPLAIN 没有 `JoinOrder` 行。

最外层连接交给 WHERE 和 ORDER BY 的列序、限定符序与书写装配相同。内层行按选中叶序拼接。替换之后调用现有的 `integer_pk_base_scan_join`，不按行数再交换左右。计划器调用 `choose_left_deep_order`，不另写 8 表上限。

必须保持：没有统计时，既有 EXPLAIN 节点行的字节不变。哈希连接仍构建右输入。

禁止：改 `sort_unstable_by`；把两表连接对调；重排外连接；为了统计改页格式；修改 `openspec/specs/` 或 `.agents/`。

先行测试：在 `tests/join_reorder_test.rs` 增加 `star_join_explain_puts_fact_first`。000 已完成而替换未做时，`cargo test --offline --test join_reorder_test star_join_explain_puts_fact_first -- --test-threads=1` 失败，因为第一条扫描仍是 `d1`。同一文件写上上面其余观察的测试。

GREEN：依次执行，都带 `--test-threads=1`：

```bash
cargo test --offline --test join_reorder_test -- --test-threads=1
cargo test --offline --test table_statistics_test -- --test-threads=1
cargo test --offline --lib table_stats -- --test-threads=1
```

停止条件：最外层列序无法与书写装配一致，或为了结果一致必须交换单个连接的左右输入，或必须把排序改成稳定排序。这些情况回到计划，不在本任务里改结果语义。

## 4. 一次全量（Iteration 002）

执行依据是 `iterations/002-batched-full-regression/000-initial.md`。本段不再另选批次算法。

- [x] 4.1 T4：按 Runbook 做一次分批全量，并记录每一批的退出码和 passed、failed、ignored。

当前行为：001 的定向测试已经通过。全量尚未为本次变更运行。

完成后：执行时的 `ls tests/*.rs | sort` 与批次清单比对，missing、extra、duplicate 都是 0。每一批退出码 0，且该批 `test result:` 行的 failed 为 0。`--lib --bins` 单独成一批。记录目标数，以及 `test result:` 行数。这两个数差在 lib 与 bin 各占一行时成立。

命令形态：

```bash
cargo test --test <f1> --test <f2> ... -- --test-threads=1
cargo test --lib --bins -- --test-threads=1
```

计数只用 `grep -E "^test result:"` 读出。判定看 cargo 的退出码和这些行。不新增判定脚本。

禁止：无过滤的 `cargo test`；去掉 `--test-threads=1`；并行启动多批；全量失败后再跑一整轮。

某一批失败时，按 Runbook 先单独复核该失败，再整批复核一次。复核通过则记通过，并写明首跑失败。失败来自本变更的叶序或统计时，在本轮按既有规格修正，再用对应的定向测试验证。不得为此再跑第二次全量。失败与本变更无关时，停止并报告，不改其他模块。

停止条件：批次清单与执行时的文件集合不一致，或需要第二次全量才能声称通过。
