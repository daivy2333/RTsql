# `last_failed_write` 永不复位 ⇒ 一次写盘失败后本会话内的释放都不再被提升

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（Iteration 004 的「`last_failed_write` 不复位」），Plan Review 的非阻塞 3 与「范围外实质缺陷」段，change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 004 Cycle `000-initial.md`（Review Result `accepted`）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `tests/release_durability_precondition_test.rs` 集成测试、真实文件库 + 一次性写盘失败注入点；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 预期：一次写盘失败只影响**该次失败所涉及的页**；失败处置完成后，后续的释放应恢复被提升。
- 实际：`last_failed_write` 一旦被置位就**永不复位** ⇒ 该库会话内的**所有后续释放**都不再被提升（即便失败早已被刷盘恢复）。
- 位置：`src/storage/buffer_pool.rs` 的 `last_failed_write` 标记与 `src/storage/file_storage.rs::record_write_failure`。
- 根因：Confirmed（承载 change 的 Plan Review 读码确认：标记只在失败时置位，没有任何复位路径）。
- 触发条件：库会话内发生**任一次**写盘失败。

## 影响

- 当前：**安全但有界**——保守方向是「宁可不释放」⇒ 空闲页得不到复用（`free_page` 集合增长变慢），不产生数据错误。属于**性能 / 资源回收**面上的有界退化，不是正确性缺陷。
- 页粒度细化：`last_failed_write` 是**库级**标志，而失败实际只影响**单个页** ⇒ 页粒度的细化（记录失败页号集合）留作后续决定。
- 与本 change 范围的关系：**超出**——承载 change 的 Iteration 004 引入该标记是为满足「释放的持久化前置条件」，未覆盖复位语义（该 Cycle 明确划为 Excluded scope）。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 004，Plan Review 非阻塞 3）：「`last_failed_write` 不复位 ⇒ 失败之后本会话内的释放都不再被提升」——安全但有界；页粒度细化留作后续决定。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在**写盘失败的状态生命周期**（`src/storage/buffer_pool.rs` / `src/storage/file_storage.rs`）→ 建议与 ISS30（驱逐交错）同域但**独立承载**：一条是「标记不复位」，一条是「已驱逐页内容丢失」，根因不同。
- 需要的决定：复位语义（何时复位、按库级还是页粒度）与「释放不被提升」的观察口径（是否需要向用户可见）。
- 2026-10-26 关闭。原因：失败标记改为按页记录并可复位，释放提升改按锚点页，由归档变更 Iteration 007 任务 T10 处置。该 Cycle `iterations/007-write-failure-state/000-initial.md` 的 Plan Review 为 `accepted`。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/007-write-failure-state/000-initial.md` 的 Plan Review 为 `accepted`（T10）。
- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/004-release-durability-precondition/000-initial.md` —— Plan Review 的非阻塞 3 与「范围外实质缺陷」段。
- 相关行为规格：`openspec/specs/release-durability-precondition/spec.md`（释放的持久化前置条件——要求「释放前该页的写已落盘」，未规定失败标记的复位语义）。
