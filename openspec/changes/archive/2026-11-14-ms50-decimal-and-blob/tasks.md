# DECIMAL 与 BLOB

对应路线：MS50 的值类型这一支。Gate 1：PASS，结论在 `proposal.md` 的 Gate 1 节。批准原文是「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」。不是跳过。本文件不更新全局里程碑状态。

Gate 2：PASS。无 BLOCKED，无 SKIPPED。Iteration 000 的结论在 `iterations/000-decimal-blob/000-initial.md`，Review Result 为 `rework-required`。返工结论在 `iterations/000-decimal-blob/001-rework.md`，Review Result 为 `accepted`。Iteration 001 的首轮执行依据是 `iterations/001-full-suite/000-initial.md`，Plan Context 为 ready，Act Response 为 blocked，Review Result 为 `replan-required`。后继执行依据是 `iterations/001-full-suite/001-replan.md`，Plan Context 为 ready，Act Response 为 reported，Review Result 为 `accepted`。当前执行依据是 `iterations/001-full-suite/002-replan.md`，Plan Context 为 ready，Act Response 为 reported，Review Result 为 `accepted`。4.1 已勾选：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。不写成又跑了一遍全量。没有后继 Cycle，也没有下一 Iteration。

与 `ms50-result-consumption` 无依赖。两个分支都从当前 master 拉出。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格在 `specs/decimal-blob-types/spec.md`。
- 000：执行依据是 `iterations/000-decimal-blob/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 `rework-required`。
- 000 的返工：执行依据是 `iterations/000-decimal-blob/001-rework.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 `accepted`。NUMERIC、字节别名和 BLOB 主键三处缺口已关闭。
- 001：首轮执行依据是 `iterations/001-full-suite/000-initial.md`。Plan Context 为 ready。Act Response 为 blocked。Review Result 为 `replan-required`。这一次全量已跑完。文件批 B08 两次退出码 101。其中 10 个目标没有运行，不算通过。没有改产品代码。
- 001 的 replan：执行依据是 `iterations/001-full-suite/001-replan.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 `accepted`。只改了那条中止夹具的前版本首字节。定向命令退出码 0，failed 为 0。没有改产品代码。没有跑全量，也不声明这一次全量通过。
- 001 的补齐：执行依据是 `iterations/001-full-suite/002-replan.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 `accepted`。这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。11 条命令退出码都是 0，failed 都是 0。没有改产品代码，没有改测试。已通过的批没有重跑，也没有另开一遍全量。没有后继 Cycle，也没有下一 Iteration。
- Persisted Evidence：none。
- 跳过项：Iteration 000 不做全量。理由见下。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 新标签往返；旧 tuple 仍可读；1.50 与 1.5 数值相等 | 完成 |
| T2 | 000 | T1 | 声明、精确写入、比较、重开、超页、dump 往返、算术拒绝 | 完成 |
| T3 | 000 | T1 | G1 样本打开、查询、写入 | 完成 |
| T4 | 001 | 000 accepted | 这一次全量的已通过部分沿用；中断批的剩余目标和修复后的那条测试补齐。不写成又跑了一遍全量 | 完成 |
| T5 | 001 | T4 的这一次全量已经跑完 | 中止夹具的前版本重新不可解码，索引在中止后指回 v1 | 完成 |

## Iteration Plan

### Iteration 000: 两种类型可读写

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: DECIMAL 与 BLOB 可以声明、写入、比较和读回。旧库样本打开、查询和写入的结果明确。格式版本仍是 1，目录代际定义不变。
- Verification boundary: `tests/decimal_blob_test.rs` 与 tuple 往返定向测试通过。不跑全量。全量留在 Iteration 001，因为本轮只要证明新行为；整库回归是另一个验证边界。
- Diagnostic boundary: 值枚举、tuple 与 catalog 标签、写入门、比较和 dump 文本。失败时不把结果游标或 JOIN 重排算进来。
- Non-goals: 流式接口；DECIMAL 算术；索引键编码；改格式版本或目录代际；导入攒批。

### Iteration 001: 里程碑唯一一次全量

- Tasks: T4, T5
- Depends on: 000 accepted
- Stable result: 这一次分批全量仍是两个变更合计的唯一一次。已通过的批保持原记录。B08 从未运行的 10 个目标，以及修复后的那一条中止测试，已按 `iterations/001-full-suite/002-replan.md` 补齐。这些命令全部退出码 0 且 failed 为 0。T4 记为：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。不写成又跑了一遍全量。T5 已完成，不重开。4.1 已按这句勾选。
- Verification boundary: 不重跑 B00–B07、B09–B14，也不重跑 B08 里已经 failed 为 0 的 `memory_db_test`。10 个未运行目标各一条 `cargo test --test <名> -- --test-threads=1`，然后一条中止测试。上一条退出后才启动下一条。不把它们合成一条命令，也不和已通过目标放在一起。不跑第二遍全量。
- Diagnostic boundary: 某条命令退出码非 0 或 failed 非 0 时停在该命令。不改产品代码，不改其他测试，不改标签、解码或中止语义。
- Non-goals: 第二遍全量。重跑已通过的批和 `memory_db_test`。重跑 `mvcc_abort_test` 里另外两条。改产品代码或其他测试。改标签、解码或中止语义。

平衡审计：000 的三步合在一起才是「能声明、写入、比较、读回，并且旧库有明确结果」。只做标签或只做旧库都还不是这个结果，所以不拆成多个 Iteration。001 的全量与 000 的故障域不同，单独成轮。T5 只闭合这次全量已经打到的那一处标签碰撞，不构成另一项可独立交付的结果，所以留在 001。补齐仍是 T4 的剩余证明，和已经跑过的那一次全量是同一个结果，不另开一轮。目录仍是 `iterations/001-full-suite/`。`000-initial.md` 与 `001-replan.md` 已冻结。`002-replan.md` 的 Review Result 为 `accepted`。没有后继 Cycle，也没有下一 Iteration。

## 1. 值与标签（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-decimal-blob/000-initial.md` 的 T1 执行。

## 2. SQL 声明、写入、比较和读回（Iteration 000）

- [x] 2.1 T2：按同一 Cycle 的 T2 执行。依赖 T1 的值布局。

## 3. 旧库打开（Iteration 000）

- [x] 3.1 T3：按同一 Cycle 的 T3 执行。依赖 T1，不依赖 T2 的新类型语句。

## 4. 全量（Iteration 001）

- [x] 4.1 T4：15 批已经跑过一次，记录在 `iterations/001-full-suite/000-initial.md`。那是整个 MS50 的唯一一次全量，不再执行。`ms50-result-consumption` 不另跑。剩下的证明按 `iterations/001-full-suite/002-replan.md` 补齐。不改产品代码，不改其他测试。完成记录：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。

下面 1 到 9 记录已经执行过的那一次分批，只保留现场，不再执行。当前通过条件是本节末尾的完成口径。

1. 任何测试调用都带 `--test-threads=1`。禁止默认并行度。
2. 执行时重新枚举 `ls tests/*.rs | sort`，数量为 N。不把规划时的文件数写死。
3. 第一批：`cargo test --lib --bins -- --test-threads=1`。
4. 文件批数 B 是不小于 N/12 的最小整数。K = B×12−N。按字典序，前 K 批各 11 个文件，其余各 12 个。文件名去掉 `.rs` 作为 `--test` 名字。
5. K 大于 B，或任一文件批的个数不在 8 到 12 之间时，停止并回到计划。不改成一次无过滤的 `cargo test`。
6. 批与批严格串行。禁止一次不带 `--lib`、`--bins` 或 `--test` 的 `cargo test`。
7. 对账：全部 `--test` 名字等于枚举集合，无缺失、无多余、无重复。`--lib` 和 `--bins` 只在第一批。
8. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算通过。缺一批不得声明全量通过。
9. 某一批失败时只把该批再跑一次。第二次仍失败则本任务失败。不重跑已通过的批。

本轮两个分支没有代码依赖。执行时树上有什么，这一次就覆盖什么。不为了带上另一个分支再增加一次全量。

执行记录在 `iterations/001-full-suite/000-initial.md` 的 Act Response，状态是 blocked。15 批都已启动。B08 两次退出码 101，failed 为 1。该批失败点之前的 `memory_db_test` 汇总 failed 为 0。其后 10 个目标两次都没有运行，不算通过。B00–B07 与 B09–B14 退出码 0，failed 为 0。4.1 未勾选。`000-initial.md` 的 Review Result 为 `replan-required`。那 15 批不再执行。

完成记录：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。不写成又跑了一遍全量。执行记录在 `iterations/001-full-suite/002-replan.md` 的 Act Response，状态是 reported。Review Result 为 `accepted`。11 条命令退出码都是 0，failed 都是 0。没有改产品代码，没有改测试。已通过的批没有重跑。

## 5. 中止夹具回到不可解码回退（Iteration 001）

- [x] 5.1 T5：按 `iterations/001-full-suite/001-replan.md` 执行。只改 `test_abort_update_reverts_to_previous_version` 的 v1 负载首字节。证明命令是 `cargo test --test mvcc_abort_test test_abort_update_reverts_to_previous_version -- --test-threads=1`。不跑全量，不补跑 B08 其余目标。

执行记录在 `iterations/001-full-suite/001-replan.md` 的 Act Response，状态是 reported。定向命令退出码 0，failed 为 0。没有改产品代码。没有跑全量。Review Result 为 `accepted`。当时 4.1 尚未勾选。补齐之后，4.1 的完成记录是：这一次全量的已通过部分沿用，中断批的剩余目标和修复后的那条测试补齐。

T5 保持勾选，不重开。5.1 里的「不补跑」只约束 T5。T4 的补齐记录在 4.1 和 `002-replan.md`。

## 完整性

| Requirement | Task | First Test | Status |
|---|---|---|---|
| DECIMAL 与 BLOB 的值布局 | T1 | `decimal_and_blob_round_trip` | Covered |
| 声明类型 | T2 | `schema_shows_decimal_and_blob` | Covered |
| 精确写入与读回 | T2 | `exact_decimal_and_blob_round_trip_sql` | Covered |
| 同类型比较 | T2 | `decimal_orders_numerically` | Covered |
| 旧库打开结果 | T3 | `legacy_g1_opens_queries_and_writes` | Covered |
