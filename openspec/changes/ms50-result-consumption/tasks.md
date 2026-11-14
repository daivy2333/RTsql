# 结果消费

对应路线：MS50 的结果消费这一支。Gate 1：PASS，结论在 `proposal.md` 的 Gate 1 节。批准原文是「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」。不是跳过。本文件不更新全局里程碑状态。

Gate 2：PASS。无 BLOCKED，无 SKIPPED。Iteration 000 的结论在 `iterations/000-row-cursor/000-initial.md`。

与 `ms50-decimal-and-blob` 无依赖。两个分支都从当前 master 拉出。验收查询不能共用，所以分成两个变更。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格在 `specs/result-consumption/spec.md`。
- 000：执行依据是 `iterations/000-row-cursor/000-initial.md`。Plan Context 为 ready。
- Persisted Evidence：none。
- 跳过项：全量分批。理由见 Iteration Plan。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 查询语句按批量交回，拼接等于整段结果；零批量不推进；非行语句无副作用 | 待执行 |
| T2 | 000 | T1 | 未索取的行不会提前失败；整段入口仍整句失败；诊断计点不被游标调用 | 待执行 |
| T3 | 000 | T1 | 带参和显式事务入口满足同一拉取合同，丢弃游标不结束事务 | 待执行 |

## Iteration Plan

### Iteration 000: 公共行游标

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: 调用方可以用四个入口分批拿走查询行；不拉则根执行器不继续产出；`execute_sql` 仍一次交回完整结果。
- Verification boundary: `tests/result_consumption_test.rs` 全部通过，加上既有 `diagnostics_comparison_test` 里首行等待那一条定向测试。不跑全量。本里程碑只有一次全量分批，预算在 `ms50-decimal-and-blob` 的 Iteration 001。本轮跳过的理由：游标是新增入口，整段路径保持原函数；那一次全量留给改到值、页编码和目录解码的变更。
- Diagnostic boundary: `RowCursor`、`Database` 的四个打开方法、pipeline 查询臂的打开与拉取。失败时不把排序、聚合或连接的内部驻留当成游标缓冲。
- Non-goals: DECIMAL 与 BLOB；预编译游标；CLI 与协议改成边输出；修改诊断定义；取消算子内部物化。

平衡审计：三件事都是同一个游标合同，拆开不能单独成为后续变更可依赖的结果。故障域都在结果交接，不在存储格式。一个 Iteration 足够，不再按任务数拆开。

## 1. 查询拉取（Iteration 000）

- [ ] 1.1 T1：按 `iterations/000-row-cursor/000-initial.md` 的 T1 执行。

## 2. 背压与整段保持（Iteration 000）

- [ ] 2.1 T2：按同一 Cycle 的 T2 执行。依赖 T1 的游标已经存在。

## 3. 带参与事务（Iteration 000）

- [ ] 3.1 T3：按同一 Cycle 的 T3 执行。依赖 T1。

## 完整性

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 公共行游标分批交回 | T1, T3 | `batches_match_execute_sql` | Covered |
| 非行语句在副作用之前拒绝 | T1 | `insert_is_rejected_without_writing` | Covered |
| 结果交接处的背压 | T2 | `later_cast_failure_is_not_prefetched` | Covered |
| 整段入口与诊断口径保持 | T2 | `execute_sql_cast_returns_no_rows` | Covered |
