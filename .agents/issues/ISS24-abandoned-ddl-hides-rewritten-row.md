# 被放弃的 DDL 语句隐藏了它开始改写的那一行

- Status: closed
- Filed: 2026-10-24
- Closed: 2026-10-25
- Source: Act Response 的 Remaining Issues（候选 C）+ Plan Review 的 F4（行数断言收窄的理由：把范围外的既有缺陷写进验收面会长期 RED），change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 Cycle `000-initial.md`（Review Result `accepted`）；关闭依据见「处置」与「证据」段（承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 (58cf8f9c3 2026-10-13) + cargo 1.99.1、sqlparser-rs 0.44；登记时工作区 HEAD `0a08b14`（ahead of origin 9）；复现现场为 MS34 Issue 修复批实施期工作树（产品改动未提交）

## 缺陷描述

- 预期：被放弃（丢弃或超时）并被回收的 DDL 语句整体回滚——原本可见的行不应因该语句被放弃而变得不可见。
- 实际：被放弃的 `ALTER TABLE t DROP COLUMN v` 在回收后，把**它已经开始改写的那一行**从扫描结果中隐藏掉。实测 30 行 → 29 行、2000 行 → 1999 行，缺失的是 `id=2`。
- 位置：`src/storage/data/table_manager.rs`（改写循环为该行写下墓碑并登记版本）、`src/transaction/manager.rs:321`（`abort_cleanup_versions` → `mark_aborted` 的版本标记路径）、`src/executor/alter_table.rs`（中止 / 回收路径）。
- 根因：Inferred（线索：被放弃的语句已为该行写下墓碑；回收把该墓碑标为 aborted，但该行仍不出现在扫描结果里——墓碑与 aborted 版本对该行可见性的叠加作用未独立复核）。
- 触发条件：DDL 语句被放弃（语句 future 被丢弃或超时到期）→ 下一条写语句触发隐式事务回收 → 该 DDL 已经改写到该行的数据面。

## 影响

- 当前：被放弃的 DDL 让一行原本可见的数据消失（静默少一行，行数不守恒）；行数规模不影响命中概率（30 行与 2000 行同样命中 1 行）。
- 与本 change 范围的关系：**超出**。承载 change 保证的是**目录面**一致（目录窄化延后到提交点），未覆盖数据面「已开始改写的那一行」在放弃后的处置；该 change 修复前后实测同为 29/30 ⇒ 它的改动不影响该行为。
- 潜在：同一「墓碑 + aborted 版本叠加」形态对任何「先写墓碑再被放弃」的语句都可能成立（登记时未逐一核对其他语句类型）。

## 事件记录

- 2026-10-24（MS34 Issue 修复批 Iteration 001，RED 阶段实测）：放弃 `ALTER TABLE t DROP COLUMN v` → 下一条写语句触发回收 → `SELECT id FROM t` 缺 `id=2`（30 行 → 29 行；2000 行 → 1999 行）。修复前后实测同为 29/30，证实该行为与该 change 的目录面改动无关。
- 2026-10-25（缺陷修复 change Iteration 001，RED → GREEN）：承载 change 的 T6 先行用例 `abandoned_drop_column_keeps_rewritten_row_visible`（N=30）在修复前 RED 输出 `the abandoned rewrite must not hide the row it started rewriting; missing ids: [2] (visible: [1, 3, …, 30])`，与本 Issue 登记的 30→29、缺失 `id=2` 同形；四层判别读数随用例固化在测试代码中。

## 处置

- Status: closed（2026-10-25，用户指令关闭）。关闭原因：`fixed`——缺陷面已由 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 001「DDL 改写一致性」端到端修复并通过验收，该 change 已收尾归档为 `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/`（Cycle 000 Review Result `accepted`，T4–T7 全 done；收尾全量 14 批 / 143 目标 / **2231 passed / 0 failed / 2 ignored**）。
- **根因结论（登记时的 Inferred 在承载 change 中被四层判别协议钉死）**：
  - **M1（链可达性）排除**——缺失行（被放弃的改写已触及的行）的槽位所在页**在数据页链上可达**（`touched_pages_on_chain = true`）；Plan Review 独立复核给出更强的结构性依据：N=30 时 30 行 + 3 个版本槽**全部落在同一个数据页（page 2）** ⇒ 该规模下链分叉在结构上**不可能**发生 ⇒ M1 不是本症状的成因。
  - **M2（`pointed` 抑制集合不识别 aborted 标记）不是缺失行的成因，但是一个独立缺陷**——缺失行（`id=2`）版本链上唯一的指向者 `create_tx_id = 31`（≠ 0）⇒ 不存在已回滚指向者。M2 在**另一行**（被完整改写、其墓碑已被中和的行）上成立 ⇒ 按承载 change 的 D5 决策 3「无论是否为实测症状都修」一并修复。
  - **缺失行消失于第 4 层（墓碑未中和）+ 第 2 层（主键条目被删未还原）**——该墓碑的 `create_tx_id` 仍是被放弃事务的 id（**未被中和**）；回收把该事务移出活跃集后，扫描的 `superseder_suppresses` 走「已提交删除」分支 ⇒ 压制其前驱版本 ⇒ 该行不可见；同源损伤是该行的主键条目已被 DELETE 腿删除且未被还原（第 2 层）。
  - **实测根因**：放弃窗口落在「墓碑写入 + 主键条目删除」与「向事务管理器登记该版本」之间——墓碑已在链上（按删除者事务状态压制前驱版本），而事务管理器看不到它 ⇒ 回收无法中和它。
- 修复落点（实现形态）：
  1. **墓碑写入次序**（第 4 层）：DELETE 腿的墓碑改为「**先写（不带链指针）→ 登记版本 → 补链指针 → 再清理索引条目**」。墓碑槽的**最终内容**（`create_tx` / 删除哨兵 / `next_version`）与 DeleteExecutor 形态逐字节同形 ⇒ 终态不变，只调整次序。
  2. **`abort_cleanup_versions` 新增 `versions_created_by`**（`src/transaction/manager.rs`）：中止清理在登记版本之外，按**创建者事务 id** 沿数据页链收回本事务创建但**未被登记**的版本（链成环显式报错，不无限走查）⇒ 覆盖「放弃落在写出版本槽与登记版本之间」的更窄窗口（该窗口下墓碑未被登记、回收完全看不到该表 ⇒ 缺失行 + 主键条目丢失）。
  3. **M2 独立缺陷**（四处 `pointed` 构造点行为一致）：抑制集合只承认「指向者**未被回滚**」的链接（以指向者的 `create_tx_id == 0` 为准，与 `superseder_suppresses` 同源）——`table_manager.rs::alter_drop_column` / `table_has_live_rows`、`executor/create_index.rs::backfill_tree`、`wal/recovery.rs::rebuild_pk_indexes`；并附带「链尾回溯**不以已回滚版本为起点**」的恢复面规则（否则恢复期同一存活版本被建两次条目）。
- 修复结论（逐条对应本 Issue 的结论面）：
  1. 被放弃的 DDL 不隐藏它开始改写的那一行——`SELECT id FROM t` 逐行可数（30 行且 id 集合完整，缺失即失败）。
  2. 点查仍命中——`SELECT * FROM t WHERE id = ?` 对 1..=30 每行命中 1 行。
  3. 重开库一致——close → reopen 后目录列定义 `["id","v","w"]`、列数 3、行数 30、每行行宽 3。
  4. 放弃留下的每个墓碑**已被中和**（`create_tx_id == 0`）。
  5. 两个更窄的放弃窗口一并闭合——「落在写出版本槽与登记版本之间」（RED：`missing ids: [1]`）与「落在窄化副本写入与登记之间」（RED：30 → 31，可见 `id=1` 重复）均已 GREEN。
- 行为规格：`openspec/specs/ddl-rewrite-consistency/spec.md`（被放弃的改写不隐藏已改写的行）+ `openspec/specs/mvcc-tombstone-visibility/spec.md`（已回滚墓碑不抑制其前驱版本，AR2 澄清）。
- 状态迁移记录：
  - 2026-10-25 `open` → `closed`：用户指令关闭；修复随归档 change 交付。回归承载：`tests/ddl_rewrite_consistency_test.rs`（6 用例，含 4 条放弃路径用例 + 1 条 M2 独立用例）；修复前 RED `missing ids: [2]` → 修复后 `6 passed; 0 failed` + 收尾全量 14 批 / 143 目标 / 2231/0/2。
- 关闭时保留的诚实边界：承载 change 的 Plan Review 记录了一处**同类但不同**的残留——「提交完成 → 目录应用」窗口内的**持久化**不一致（崩溃后重开仍按旧列数错读），该形态需持久标记或移动提交点（范围外的设计决定），已作为**独立** Issue 登记为 `.agents/issues/ISS25-ddl-commit-point-positional-misread.md` 的仍开放变体，**不属本 Issue 的结论面**（本 Issue 的结论面是「被放弃的 DDL 隐藏它开始改写的那一行」，已修复）。登记时「潜在：同一『墓碑 + aborted 版本叠加』形态对任何『先写墓碑再被放弃』的语句都可能成立（登记时未逐一核对其他语句类型）」**未逐一核对**，不属本 Issue 的已验收主张。
- 登记时的承载落点（建议，已完成）：原建议落在中止 / 回收语义（版本可见性域）并与 MS35 相邻排期；实际承载为非里程碑的缺陷修复 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 001「DDL 改写一致性」。
- 与 ISS22 / ISS23 的边界不变：三条根因位置与触发条件不同，**不合并**（ISS23 已于 2026-10-25 独立关闭于 Iteration 000；ISS22 已于 2026-10-25 独立关闭于 Iteration 001）。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md` —— Act Response 的 Remaining Issues 候选 C（实测行数与墓碑 / aborted 线索）、Plan Review 的 F4（行数断言收窄为「非空」的理由，明确该缺陷属范围外既有行为）。
- 登记时（2026-10-24）独立复核的代码位点：`src/storage/data/table_manager.rs:897-1007`（改写循环写墓碑并登记版本）、`src/transaction/manager.rs:321`（`abort_cleanup_versions`）
- 关闭时证据：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/001-ddl-rewrite-consistency/000-initial.md` —— Act Response 的 T6 / T7 实际改动与「ISS24 的判别结论」段（M1 读数 / M2 读数 / 层次读数 / 实测根因 / 两个更窄窗口）、AR2 的用例与判据、RED → GREEN 表（`missing ids: [2]` → `6 passed; 0 failed`）；Plan Review 的 F1–F5（四层判别结论的独立复核：M1 排除且给出结构性依据、M2 为独立缺陷、第 4 层 + 第 2 层与代码一致）、Deviation 1–3 的裁定（`abort_cleanup_versions` 扩面授权、墓碑写入次序为必要组成、恢复面伴随规则为 M2 的最小后果）；行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md` 与 `openspec/specs/mvcc-tombstone-visibility/spec.md`
- 关闭时实现位点：`src/storage/data/table_manager.rs::TableManager::alter_drop_column`（墓碑写序）、`src/transaction/manager.rs::TransactionManager::abort_cleanup_versions` + 自由函数 `versions_created_by`、`src/executor/create_index.rs::CreateIndexExecutor::backfill_tree`、`src/wal/recovery.rs::RecoveryManager::rebuild_pk_indexes`
