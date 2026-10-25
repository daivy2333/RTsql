# DML 墓碑的「放弃窗口」复现配方

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/dml_tombstone_abandon_window_test.rs` 集成测试、tokio 多线程 test runtime、单用例自带 `tempfile::tempdir()` 现场
- Source: change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 005 / Cycle 000 Act Response 与 Plan Review（`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/005-dml-tombstone-abandon-window/000-initial.md`，2026-10-25，RED+GREEN）

## 适用范围

- 需要**确定性**复现「DML 语句被放弃时，它已写下的墓碑仍生效 ⇒ 被它删除 / 替换的那一行从数据面消失」这一缺陷形态时使用。
- 需要验证「墓碑写入顺序改为『无链指针 → 注册 → 链接 → 索引清理』之后，放弃窗口不再隐藏已改写的行」时使用——本配方的判据不依赖时序运气，锚点命中即冻结现场。
- 不适用：DDL 改写路径的放弃窗口（用 `.agents/runbooks/ddl-catalog-commit-point.md`）；索引写侧原子性的取消 / 回滚（用 `.agents/runbooks/index-write-side-atomicity-repro.md`）；进程崩溃 / 重开语义（用 `.agents/runbooks/in-process-crash-simulation.md`）；全量回归的分批执行（用 `.agents/runbooks/m19-batched-full-regression.md`）。

## 前置条件

- 夹具形态：`tests/dml_tombstone_abandon_window_test.rs` 的 `setup`（建 `t(id INT PRIMARY KEY, v INT)` 与 `t2`）与 `TempDir::new()` 现场。
- 放弃的注入点是**语句 future**（不是超时、不是连接中断）：`drop(statement)` 把执行器停在执行期，隐式事务留在活跃集里 ⇒ 下一条写语句触发**库属隐式事务回收**。
- 锚点语义：数据链上**已出现由活跃事务创建的版本槽**（`anchor_slot_written` 遍历 `transaction_manager.active_transactions()` 与数据链槽位，匹配 `slot.create_tx == tx_id`）。锚点只保证「写出了版本槽」，**不**保证「已登记版本」⇒ 两种放弃落点都要能区分（见「操作步骤」第 4 步的 `registered` 读数）。
- 夹具不携带任何运行身份（无 run-id / 指纹 / 时间戳）；`ANCHOR_TIMEOUT` 是**兜底 timer，不是行为判据**。

## 操作步骤

1. **建表并预置 30 行**（`t(id INT PRIMARY KEY, v INT)`，id = 1..=30；N = 30 是刻意选的小规模：让全部行落在同一数据页，从而把「链分叉」这一形态从缺陷判据里排除掉，使读数只反映墓碑 / 登记语义）。
2. **发起待放弃的语句**：`DELETE FROM t WHERE id = 1`（锚点式删除）。
3. **等锚点命中并冻结现场**：用 `tokio::select!` + `biased` 等待「锚点命中」与「语句跑完」二者之一；**锚点命中优先**，若语句先跑完则直接 panic（夹具不成立，不是产品行为）。
4. **读放弃落点**：`registered = !db.transaction_manager.get_tx_versions(tx_id).await.is_empty()`。
   - `registered == false` ⇒ 放弃落在「**写出版本槽**」与「**登记版本**」**之间**（本配方要命中的形态）；
   - `registered == true` ⇒ 放弃落在「登记版本」**之后**。
   该读数必须随证据一起记录——它决定本次命中的是哪个窗口。
5. **放弃语句**：`drop(statement)`（执行器停在执行期，隐式事务留在活跃集里）。
6. **触发回收**：执行下一条普通写语句 `INSERT INTO t2 VALUES (1)` —— 库属隐式事务回收先于该语句的隐式事务发生 ⇒ 被放弃事务的墓碑此刻被中和或不被中和。
7. **观察可见 id 集合**：全表扫出可见 id 列表（`scan_id`），与 1..=30 逐项比对。
8. **逐 id 点查命中**：对 1..=30 每个 id 做一次点查（`scan_by_pk`），确认命中**恰好 1 行**（不是 0 行、不是 2 行）。
9. **（守门用例）确认已提交的墓碑终态不变**：`committed_delete_keeps_the_canonical_tombstone_slot` 断言已提交 `DELETE` 的墓碑槽终态字节、WAL `Delete` 记录字段与索引条目集合**逐项不变** ⇒ 保证「修复只改顺序、不改终态」。

## 验证

- 成功判据：`cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` 退出码 0，输出 `test result: ok. 3 passed; 0 failed`（2026-10-25 实测）。
- **修复前 RED 形态**（2026-10-25 实测，`test result: FAILED. 1 passed; 2 failed`）：
  - ① `an abandoned DELETE must not hide the row it deleted; missing ids: [1] (visible: [2, 3, 4, …, 30])` ⇒ 可见 id 集合为 `[2..30]`、**29 行**、`id = 1` 缺失；
  - ② `an abandoned REPLACE must not hide the conflict row it deleted; missing ids: [1] (visible: [2, 3, …, 30])`；
  - ③ 第三条（已提交墓碑终态守门）修复前**即为 GREEN**（`1 passed`）。
  - 两条 RED 的伴随读数均为 `version registered at the abandon point = false` ⇒ 命中的是「写出版本槽 → 登记版本」窗口。
- 缺陷形态的正确性：失败不是「行数不变但内容错」「点查命中 2 行」或其它形态，而是**被放弃的语句使其已删除的那一行从数据面消失**（与登记现场「30 → 29、缺 `id = 1`」逐项一致）。
- 断言强度：逐行可数 + 逐 id 点查，规模 N = 30 < 300（刻意取小以排除链分叉）。
- 判据全部落在**可观察状态**上（可见 id 集合、点查命中行数、墓碑槽终态字节、WAL 记录字段、索引条目集合）；判定层在 `cargo test` 内，无项目测试框架之外的判定脚本。

## 失败处理

- **锚点未命中**（`the abandon anchor must be reachable: the fixture statement did not reach it in time`）：锚点只保证「版本槽已写」，确认写入规模足以产生版本槽（30 行、id = 1 单行删除会写出版本槽）；不要放宽超时当作命中。
- **语句先跑完**（`statement finished before the abandon anchor was reached`）：夹具不成立，不是产品行为——调整写入规模使锚点可达，不要把「跑完」当成「被放弃」。
- **看不到 `missing ids` 而是别的不一致**：确认本配方命中的是「写出版本槽 → 登记版本」窗口（读 `registered` 读数）；落在「登记版本之后」的放弃点不产生本形态。
- **`registered` 读数为 `true` 但仍复现缺失行**：那是另一个窗口（放弃落在登记之后）⇒ 按 `.agents/issues/` 中相关 Issue 的边界处理，不要与本形态混记。
- **`ANCHOR_TIMEOUT` timer 触发**：夹具失控——它是兜底不是判据；检查锚点轮询是否在 `yield_now` 上自旋。

## 回滚

不适用（测试配方，无产品状态变更）。夹具只作用于本用例的 `Database`；`TempDir` 随用例结束清理。

## 证据

- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/005-dml-tombstone-abandon-window/000-initial.md` —— Act Response 的 T14 先行测试 RED 证据（逐字的实际运行输出：`missing ids: [1]`、`version registered at the abandon point = false`、`1 passed; 2 failed`）与 GREEN 证据表（`3 passed; 0 failed`）；Plan Review 的独立复核。
- 见证代码：`tests/dml_tombstone_abandon_window_test.rs`（3 用例 + 辅助 `setup` / `scan_id` / `scan_by_pk` / `pk_entry_count` / `chain_pages` / `dump_chain` / `anchor_slot_written` / `abandon_statement` / `trigger_reclaim`）。
- 修复落点（顺序形态）：`src/executor/delete.rs::DeleteExecutor::delete_row`、`src/executor/upsert.rs::UpsertExecutor::delete_conflict_row`（墓碑写 → 登记 → 链指针补写 → 条目清理 → WAL）。
- 行为规格：`openspec/specs/mvcc-tombstone-visibility/spec.md`、`openspec/specs/ddl-rewrite-consistency/spec.md`。
- 相关配方：`.agents/runbooks/index-write-side-atomicity-repro.md`（索引写侧锚点式暂停与取消）、`.agents/runbooks/ddl-catalog-commit-point.md`（DDL 目录提交点）。
