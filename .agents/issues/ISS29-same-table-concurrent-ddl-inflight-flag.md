# 同表并发 DDL 与在途状态布尔量冲突（后发 DDL 清除先行 DDL 的读拒绝）

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（Iteration 003 的「并发同表 DDL 的交互」）、Plan Review 的 finding 7 附带的范围外缺陷与「Follow-up Decision」，change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 003 Cycle `000-initial.md`（Review Result `accepted`）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `tests/ddl_rewrite_consistency_test.rs` 集成测试；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 预期：同一条表上的两个并发 DDL 不应互相破坏对方的正确性；处于窄化在途状态的表，其读路径 SHALL 继续被拒绝。
- 实际：在途状态是**单表一个布尔量**（`NarrowingState`：`table: String` + `in_flight: AtomicBool`）⇒ 两个交互缺陷：
  1. **重叠清除**：同表上并发的第二个 DDL 若先完成并清除该布尔量，会让**第一个**仍在窗口内的读**失去拒绝** ⇒ 窗口内的读退回到按位置取值（静默错读）。
  2. **豁免读按旧 schema 解码**：第二个 DDL 的**改写自身读**（豁免路径——它读的是尚未窄化的旧行、用**旧 schema** 解码）会把**已被第一个 DDL 窄化**的行按旧 schema 解码 ⇒ 解码错位。
- 位置：`src/storage/data/table_manager.rs`（`NarrowingState` / `NarrowingGuard` / `TableMeta.narrowing` / `alter_drop_column` 的发布 / `apply_column_drop` 的清除）与 8 个读路径执行器的在途检查点（`src/executor/data_scan.rs` / `scan.rs` / `index_scan.rs` / `index_scan_all.rs` / `create_index.rs` / `delete.rs` / `update.rs` / `upsert.rs`）。
- 根因：Confirmed（承载 change 的 Plan Review 独立确认；该形态**不是**该 change 引入的——同表并发 DDL 在此之前即不安全，两条改写争同一条数据链与目录）。
- 触发条件：同一条表上**并发**发起两个 `ALTER TABLE`（两个改写争同一条数据链与目录）。

## 影响

- 当前：并发同表 DDL 下可能出现窗口内读**失去拒绝**（回到静默错读）或改写自身读**按旧 schema 解码已窄化的行**。
- 与本 change 范围的关系：**超出**——承载 change 的 Iteration 003 明确记录该形态**本 Cycle 未引入也未修复**，且同表并发 DDL 在此之前即不安全（两条改写争同一条数据链与目录）⇒ 不扩大该面。
- 需要的设计决定：「**同表 DDL 串行化**」是一张新的设计决定（拒绝同表并发 DDL，或把在途状态从布尔量改为可区分多个在途改写的结构）。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 003，Act Response 与 Plan Review）：作为非阻塞 finding + 范围外 Issue 候选上报——「在途状态是**单表一个布尔量**，同表上并发的第二个 DDL 若先完成并清除状态，会让第一个仍在窗口内的读失去拒绝；同时第二个 DDL 的**改写自身读**（豁免路径）会把已被第一个 DDL 窄化的行按旧 schema 解码。同表并发 DDL 在本改动之前即不安全，本 Cycle 不扩大该面；该交互需要『同表 DDL 串行化』这一新设计决定。」
- 2026-10-25（缺陷修复 change Iteration 004，Excluded scope）：「同表并发 DDL 的串行化」被再次显式划为范围外。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在 **DDL 并发控制 / 在途状态建模** → 建议独立 change 承载「同表 DDL 串行化」这一设计决定；在该决定落地前，本条**不**应与其它 DDL 面混在同一承载中。
- 前置决定：需要先确定「同表 DDL 串行化」的具体形态（库入口拒绝同表并发 DDL / 语句级排队 / 在途状态改为可区分多改写的结构）⇒ 属设计决定，须由用户 / Plan 裁定。
- 2026-10-26 关闭。原因：同表并发窄化 DDL 的在途状态已改为带所有者令牌，后发窄化在改写开始前被显式拒绝，由归档变更 Iteration 006 任务 T8 处置。该 Cycle 的 Plan Review 为 `accepted`。`ADD COLUMN` 与在途窄化的交错不在本条内（该 Cycle 写明 ISS29 未覆盖这一形态），另见 ISS39。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/006-ddl-commit-point-unified-model/000-initial.md` 的 Plan Review 为 `accepted`（T8）。`ADD COLUMN` 交错见 `.agents/issues/ISS39-add-column-races-inflight-narrowing.md`。
- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/003-rewrite-window-read-consistency/000-initial.md` —— Act Response 的「并发同表 DDL 的交互」段（该 Cycle 未引入、未修复、需新设计决定）与 Plan Review 的范围外实质缺陷段；Iteration 004 的 Excluded scope（`iterations/004-release-durability-precondition/000-initial.md`）。
- 相关行为规格：`openspec/specs/ddl-rewrite-consistency/spec.md`（Requirement「改写在途期间读路径不静默错读」——四个 Scenario 全部限定在**单条**改写的进程内窗口，未涵盖同表并发 DDL）。
