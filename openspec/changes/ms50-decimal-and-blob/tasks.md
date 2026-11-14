# DECIMAL 与 BLOB

对应路线：MS50 的值类型这一支。Gate 1：PASS，结论在 `proposal.md` 的 Gate 1 节。批准原文是「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」。不是跳过。本文件不更新全局里程碑状态。

Gate 2：PASS。无 BLOCKED，无 SKIPPED。Iteration 000 的结论在 `iterations/000-decimal-blob/000-initial.md`。Iteration 001 尚未展开。

与 `ms50-result-consumption` 无依赖。两个分支都从当前 master 拉出。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格在 `specs/decimal-blob-types/spec.md`。
- 000：执行依据是 `iterations/000-decimal-blob/000-initial.md`。Plan Context 为 ready。Act Response 为 reported，Review Result 仍为 pending。
- 001：只写在本 Iteration Plan。目录尚未创建。
- Persisted Evidence：none。
- 跳过项：Iteration 000 不做全量。理由见下。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 新标签往返；旧 tuple 仍可读；1.50 与 1.5 数值相等 | 完成 |
| T2 | 000 | T1 | 声明、精确写入、比较、重开、超页、dump 往返、算术拒绝 | 完成 |
| T3 | 000 | T1 | G1 样本打开、查询、写入 | 完成 |
| T4 | 001 | 000 accepted | 本里程碑唯一一次 M19 全量分批 | 待执行 |

## Iteration Plan

### Iteration 000: 两种类型可读写

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: DECIMAL 与 BLOB 可以声明、写入、比较和读回。旧库样本打开、查询和写入的结果明确。格式版本仍是 1，目录代际定义不变。
- Verification boundary: `tests/decimal_blob_test.rs` 与 tuple 往返定向测试通过。不跑全量。全量留在 Iteration 001，因为本轮只要证明新行为；整库回归是另一个验证边界。
- Diagnostic boundary: 值枚举、tuple 与 catalog 标签、写入门、比较和 dump 文本。失败时不把结果游标或 JOIN 重排算进来。
- Non-goals: 流式接口；DECIMAL 算术；索引键编码；改格式版本或目录代际；导入攒批。

### Iteration 001: 里程碑唯一一次全量

- Tasks: T4
- Depends on: 000 accepted
- Stable result: 本分支上的分批全量每一批 failed 为 0。这是 MS50 两个变更合计的唯一一次全量。
- Verification boundary: 按 M19 和 `.agents/runbooks/m19-batched-full-regression.md` 的等分规则执行。本轮不改产品代码。
- Diagnostic boundary: 失败的那一批。只复核该批一次。不重跑已经通过的批，也不再排一次全量。
- Non-goals: 用这次全量补新功能。为流式变更再跑一次全量。

平衡审计：000 的三步合在一起才是「能声明、写入、比较、读回，并且旧库有明确结果」。只做标签或只做旧库都还不是这个结果，所以不拆成多个 Iteration。001 只有全量，验证边界和故障域都与 000 不同，单独成轮。001 现在不建目录。

## 1. 值与标签（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-decimal-blob/000-initial.md` 的 T1 执行。

## 2. SQL 声明、写入、比较和读回（Iteration 000）

- [x] 2.1 T2：按同一 Cycle 的 T2 执行。依赖 T1 的值布局。

## 3. 旧库打开（Iteration 000）

- [x] 3.1 T3：按同一 Cycle 的 T3 执行。依赖 T1，不依赖 T2 的新类型语句。

## 4. 全量（Iteration 001）

- [ ] 4.1 T4：000 被接受之后才展开本轮。按下面的分批方式跑一次。这是整个 MS50 的唯一一次全量。`ms50-result-consumption` 不另跑。本轮不改产品代码。

分批是通过条件的一部分：

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

## 完整性

| Requirement | Task | First Test | Status |
|---|---|---|---|
| DECIMAL 与 BLOB 的值布局 | T1 | `decimal_and_blob_round_trip` | Covered |
| 声明类型 | T2 | `schema_shows_decimal_and_blob` | Covered |
| 精确写入与读回 | T2 | `exact_decimal_and_blob_round_trip_sql` | Covered |
| 同类型比较 | T2 | `decimal_orders_numerically` | Covered |
| 旧库打开结果 | T3 | `legacy_g1_opens_queries_and_writes` | Covered |
