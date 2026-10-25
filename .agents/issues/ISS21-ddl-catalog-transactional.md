# 被放弃的 ALTER TABLE DROP COLUMN 后目录已窄化、数据面仍为混合宽度（DDL 目录变更不在事务内）

- Status: closed
- Filed: 2026-10-24
- Closed: 2026-10-24
- Source: Act Response 的 Experience Candidates（C1）+ Plan Review 的 Issue 候选复核 F6（change `2026-10-24-ms34-cancellation-timeout-transaction` Iteration 001 Cycle `001-replan.md`，Review Result `accepted`）
- Environment: Linux x86_64 / WSL2、rustc 1.99.1 (58cf8f9c3 2026-10-13) + cargo 1.99.1、sqlparser-rs 0.44；登记时工作区 HEAD `0a08b14`（ahead of origin 9）；复现现场为 MS34 实施期工作树

## 缺陷描述

- 预期：被放弃的 DDL 语句整体回滚——目录列定义与数据面行宽保持一致。
- 实际：表处于「目录已窄化 + 数据面仍为混合宽度」的形态——回收只回滚数据面，目录侧的窄化不在事务内。
- 位置：`src/storage/data/table_manager.rs:792-802`（`alter_drop_column`：先改目录（`rewrite_column_rows_drop` + `update_table_column_count`）并 `flush_all`，窄化不在事务内；事务只覆盖随后的逐行 DELETE + INSERT 数据面改写）。执行路径涉及 `src/executor/alter_table.rs`。
- 根因：Confirmed（读码确认：目录改写与 `flush_all` 先于任何数据 WAL 记录，窄化不在事务边界内）。
- 触发：Unconfirmed（MS34 实施期未重新复现为独立故障事件，见事件记录）。

## 影响

- 当前：目录面与数据面的列宽不一致——后续查询按窄化后的目录解释旧数据。
- 触发条件：`ALTER TABLE DROP COLUMN` 的语句 future 被放弃（丢弃或超时）。
- 与本 change 范围的关系：DDL **目录变更的事务化**不在「取消/超时/事务收尾」的调用方语义面；MS34 的回收只覆盖数据面，目录侧不在回收范围内，其 Impact 不含目录事务化。MS34 已 accepted 归档，本缺陷不在 MS34 内修。

## 事件记录

None（登记时：MS34 实施期未复现为独立故障事件）。Iteration 000 与其 rework Cycle 的 Plan Review 均报告该形态；第三处库属隐式事务站点只补了回收契约的回归用例 `abandoned_alter_table_drop_column_is_reclaimed`（**不断言数据面形态**）。

最短可观察步骤（Plan Review 补齐，登记时未执行）：在库入口发起一条 `ALTER TABLE t DROP COLUMN v` 并在其执行期放弃该语句的 future → 触发下一条写语句的回收 → `close()` 后重开 → `SELECT * FROM t` 的目录列数（已窄化）与数据面行宽（仍按旧宽度存放）不一致。

- 2026-10-24（MS34 Issue 修复批 Iteration 001，修复前 RED 实测）：上述最短可观察步骤在承载 change 的 T4 先行 RED 阶段被完整复现——放弃 `ALTER TABLE t DROP COLUMN v` 后目录列定义 `["id","w"]`、列数 2、行宽 2，而语句执行前为 `["id","v","w"]` / 3 / 3；连续 5/5 稳定命中目录面断言。登记时「触发：Unconfirmed」的记述在该时点转为 Confirmed（登记时的记述不改写）。修复后同一断言集 GREEN（`cargo test --test ddl_catalog_transactional_test` → 3 passed / 0 failed，连续 3 次）。

## 处置

- Status: closed（2026-10-24，用户指令关闭）。关闭原因：`fixed`——缺陷面已由 change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 的 Iteration 001「DDL 目录事务化」端到端修复并通过验收，该 change 已收尾归档为 `openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/`（Cycle 000 Review Result `accepted`，T4–T6 全 done）。
- 修复落点（实现形态）：目录窄化**延后到语句提交点**生效——事务体只做守卫检查、新布局计算与事务内数据面改写，把待应用的目录变更以 `PendingColumnDrop` 交回调用方（`src/storage/data/table_manager.rs` 的 `alter_drop_column`），提交点由 `apply_column_drop` 应用，顺序为**先列行**（`rewrite_column_rows_drop`）→ **后列数**（`update_table_column_count_ddl`）→ **再切 live meta**（`src/executor/alter_table.rs` 的 `commit(tx)` → `guard.disarm()` → `apply_column_drop`）。
- 修复结论（逐条对应本 Issue 的结论面）：
  1. 目录窄化延后到提交点（`PendingColumnDrop` + `apply_column_drop`，先列行、后列数、再切 live meta）——目录面变更与数据面改写同生共死。
  2. 被放弃或失败的 DDL 不留任何目录面变更（放弃路径与执行期守卫失败路径均已钉住）。
  3. 放弃后重开库，目录列数与数据面行宽一致（close → reopen 后再次断言列定义、列数与每行行宽）。
- 行为规格：新域 `openspec/specs/ddl-catalog-transactional/spec.md`（3 Requirement / 4 Scenario）——R1 被放弃或失败的 DDL 语句不留下目录面变更、R2 目录变更与数据面改写同生共死、R3 放弃 DDL 之后重开库目录与数据面一致。
- 状态迁移记录：
  - 2026-10-24 `open` → `closed`：用户指令关闭；修复随归档 change 交付。回归承载：`tests/ddl_catalog_transactional_test.rs` 3 用例（修复前 5/5 连续稳定命中目录面断言的 RED；修复后 3 次连续 GREEN）+ `tests/statement_cancellation_test.rs::abandoned_alter_table_drop_column_is_reclaimed` 的断言补齐（只增不减）。
- 登记时建议与实际实现的对应（不改写登记时的记录）：登记时的建议是「要么窄化与数据面改写在同一可回滚单元内，要么窄化延后到语句确定完成之后」——实际实现取**后者**这一分支（目录窄化延后到语句确定完成之后）。
- 登记时的承载落点（建议，已完成）：落在 **DDL 目录面**由 DDL 目录事务化主题承载，建议与 **MS36「历史库与 dump 兼容」相邻排期**——该承载最终以非里程碑的 Issue 修复批交付（MS35 / MS36 的 Related changes 已登记该归档 change），相邻排期的最终裁定仍属 milestone-planner。
- 关闭时保留的诚实边界：承载 change 的最终 Plan Review 记录了一处**同类但不同**的残留——提交完成、目录尚未改写的崩溃窗口内，窄 tuple 在未移位的目录下按位置取值 ⇒ 位置性错读（`v` 列读到 `w` 的值）。该窗口属该 change 的 Non-goal，已作为独立 Issue 登记（`.agents/issues/ISS25-ddl-commit-point-positional-misread.md`），**不属本 Issue 的缺陷面**（本 Issue 的结论面是「被放弃的 DDL 留下目录面变更」，已修复）。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/iterations/001-caller-visible-outcomes-and-final-regression/001-replan.md` —— `Act Response` 的 Experience Candidates 段（C1）与 `Plan Review` 的 F6（根因位置独立读码核对；Review Result `accepted`）。
- Iteration 000 的两个 Cycle（该形态的首次报告与回收契约回归用例）：`openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/iterations/000-abandoned-implicit-transaction-reclaim/000-initial.md`、`.../001-rework.md`
- change 根目录（Impact、design、规格 delta）：`openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/`
- 登记时（2026-10-24）独立复核的代码位点：`src/storage/data/table_manager.rs:792-802`
- 关闭时证据：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md` —— Act Response 的 T4 / T5 / T6 实现说明与 Verification Evidence 表（修复前 RED 5/5 连续、修复后 GREEN 3 次连续、收尾全量 13 批 2171/0/2）；Plan Review 的 F5（提交点设计的独立核对：事务体内无目录写入、提交点顺序与守卫顺序正确）、F7（三条 requirement 的判别力核对）、F9（收尾全量的结构与覆盖对账）；行为规格 `openspec/specs/ddl-catalog-transactional/spec.md` R1–R3
- 关闭时实现位点：`src/storage/data/table_manager.rs:715-1015`（`alter_drop_column`）、`:1017-1045`（`PendingColumnDrop` 与 `apply_column_drop`）、`src/executor/alter_table.rs:80-146`（`commit` → `disarm` → `apply_column_drop`）、`src/storage/catalog.rs`（`update_table_column_count_ddl`）
