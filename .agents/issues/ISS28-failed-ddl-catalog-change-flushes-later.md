# 失败的 DDL 语句其目录面变更在后续刷盘落地（与「失败的 DDL 不留目录面变更」字面冲突）

- Status: open
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选 3（「失败的 DDL 语句其目录面变更在后续刷盘落地」，Plan Context Risks 的候选 3 被实测坐实）+ Plan Review 的 finding 5 / 非阻塞 2 与「范围外实质缺陷」段，change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 004 Cycle `000-initial.md`（Review Result `accepted`）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `tests/release_durability_precondition_test.rs` 集成测试、真实文件库 + 一次性写盘失败注入点；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 预期：按既有域 `ddl-catalog-transactional` R1，**失败的 DDL 语句不留下目录面变更**。
- 实际：承载 change 修复「刷盘失败保持脏标记」后，该形态**变得可达**：一条**失败**的 DDL 语句已发出的目录面变更（列行 + 表行列数）**在此后某次成功刷盘时落地** ⇒ 重开库后可见该失败 DDL 的目录面变更。
- 位置：`src/storage/catalog.rs::Catalog::flush_commit_point`（`:269`，语句边界的落盘 = `flush_all_except` + `flush_pages`）的失败语义 + 目录面变更在**语句边界落盘之前**已发出（`src/storage/data/table_manager.rs::TableManager::alter_add_column` 的 `insert_column_row` → `update_table_tail`）；脏标记生命周期由承载 change 的 `src/storage/buffer_pool.rs`（`restore_dirty_from` / `flush_all` / `flush_pages` / `flush_all_except`）与 `src/storage/file_storage.rs`（`record_write_failure`）承载。
- 根因：Confirmed（实测坐实——修复后可达已被真实运行对照确认）。
- 触发条件：目录面变更已发出之后，**语句边界的落盘**失败；该 DDL 随后有一次成功刷盘且被 `close()`。

## 影响

- 当前：一条**失败**的 DDL 在重开后留下目录面变更（新增列可见、行宽与目录自相矛盾面消失 ⇒ 目录与数据面**不再自相矛盾**，但「失败不留痕」的合同被打破）。
- 修复前的对照形态：同样的失败下目录面变更被**丢弃**（清脏早于写的副作用）⇒ 重开后 `SELECT id, n, m FROM keep` 报 `Column not found: m`，且该状态**自身不一致**（表行的列数已含新列、列行已丢失）。
- 与既有合同的**字面冲突**：既有域 `ddl-catalog-transactional` R1「被放弃或失败的 DDL 语句不留下目录面变更」与本形态在**字面**上冲突（承载 change 的 Plan Review 与 Act Response 均明确记录该张力，并声明该 Cycle **不裁决**该语义）。
- 潜在：已实测的形态是 `ADD COLUMN`（该语句在落盘之前不分配新页 ⇒ 一次性注入命中语句边界的落盘）；`DROP COLUMN` / `CREATE INDEX` **同型待验**。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 004，实测对照）：构造 = 真实入口 `ALTER TABLE keep ADD COLUMN m INT`（该 DDL 的目录面变更在语句边界落盘之前发出，且该语句在落盘之前不分配新页 ⇒ 一次性注入命中语句边界的落盘）→ 语句返回错误 `injected io failure (write_page)` → 一次成功刷盘 → `close()` → 重开。
  - **修复后**：`SELECT id, n, m FROM keep` ⇒ 行 `[1, 10, Null]` ⇒ **失败的 DDL 语句的目录面变更落地了** ⇒ 候选 3 的形态在本 Cycle 修复后**确实可达**。
  - **修复前**（临时还原 Fix 1 的对照运行）：同样失败，但重开后 `SELECT id, n, m FROM keep` ⇒ 错误 `Column not found: m` ⇒ 目录面变更被**丢弃**；且该状态**自身不一致**。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）；承载 change 建好后由 Recorder 更新本指针。
- 承载落点：落在 **DDL 语句边界落盘的顺序语义**（承载 change 的 `design.md` D9 / B-3）→ 建议独立 change 承载。
- **需要的决定（产品决定，超出实现层）**：两条互斥方向——① **回滚目录变更**（失败的 DDL 在语句失败时把已发出的目录面变更撤回，使 R1 的字面重新成立）；② **修订既有域**（承认「失败的 DDL 的目录面变更可能在后续刷盘落地」并修订 `ddl-catalog-transactional` R1 的表述与迁移指引）。⇒ 须由用户 / Plan 裁定方向后再定承载。
- 与承载 change 的关系：这是承载 change Iteration 004 修复「刷盘失败保持脏标记」后的**条件性副作用**——该修复使失败写不再被丢弃，代价是失败 DDL 的目录面变更可能落地；承载 change 的 Plan Review 明确把该语义划为 Excluded scope（D9 / B-3），**不由该 change 裁决**。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/004-release-durability-precondition/000-initial.md` —— Plan Context 的 Risks 候选 3、Act Response 的「候选 3 的实际观察结果」段（修复前后的对照运行与两个观察结果）、Issue 候选 1；Plan Review 的 finding 5（目录面变更与语句边界落盘的先后可由代码独立推出）与「范围外实质缺陷」段（根因位置、影响面与触发条件、可承载建议）。
- 既有合同：`openspec/specs/ddl-catalog-transactional/spec.md`（R1 被放弃或失败的 DDL 语句不留下目录面变更）。
- 承载 change 的设计记录（该语义被显式划为范围外）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/design.md`（D9）。
