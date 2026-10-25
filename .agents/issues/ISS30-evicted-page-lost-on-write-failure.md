# 并发驱逐 + 写盘失败 ⇒ 被驱逐页的内容既未落盘也不在缓冲池内

- Status: open
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（Iteration 004 的「驱逐与写盘失败的交错」），Plan Review 的 finding 4 与「范围外实质缺陷」段，change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 004 Cycle `000-initial.md`（Review Result `accepted`）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `tests/release_durability_precondition_test.rs` 集成测试、真实文件库 + 一次性写盘失败注入点；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 预期：被驱逐（evicted）出缓冲池的页，其内容要么已落盘，要么仍可在池内被取回；写盘失败时该页不得**既不落盘也不在池内**。
- 实际：页在**被驱逐**与**写盘**之间发生交错时，若随后的写盘失败，该页的**内存内容已被丢弃**（已驱逐）且**磁盘上仍是旧内容**（写盘失败）⇒ 该页的新内容**丢失**。
- 位置：`src/storage/buffer_pool.rs`（驱逐路径与写盘失败标记 `record_write_failure` / `last_failed_write`）与 `src/storage/file_storage.rs`（`record_write_failure`）。
- 根因：Confirmed（承载 change 的 Plan Review 独立确认该交错窗口存在；读码可推出）。
- 触发条件：页被驱逐出池（内存内容丢弃）与该页的写盘失败**并发交错**。

## 影响

- 当前：该页的已提交新内容丢失（磁盘仍是旧内容，池内也没有新内容）⇒ 后续读该页得到旧值；**无显式错误**暴露给用户。
- 与本 change 范围的关系：**超出**——承载 change 的 Iteration 004 修复的是「**释放**的持久化前置条件」（`last_failed_write` 使释放不被提升），未覆盖「驱逐 + 写盘失败」的交错。
- 潜在：与 `last_failed_write` 不复位（另立 Issue 登记）同属写盘失败的状态面，但根因位置不同（**驱逐交错** vs **失败状态不复位**），**不合并**。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 004，Plan Review）：作为范围外实质缺陷上报——「并发驱逐 + 写盘失败 ⇒ 被驱逐页内容既未落盘也不在池内」；该 Cycle 修复的释放持久化前置条件只覆盖**释放**路径。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在**缓冲池驱逐与写盘失败的交错**（`src/storage/buffer_pool.rs`）→ 建议独立 change 承载；需要的决定是「写盘失败时对**已驱逐**页的处置语义」（回写内存 / 保留在池内标记为脏并重试 / 提升为用户可见错误）。
- 与承载 change 的关系：承载 change 的 Iteration 004 明确把该交错划为 Excluded scope，**不由该 change 裁决**。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/004-release-durability-precondition/000-initial.md` —— Plan Review 的 finding 4 与「范围外实质缺陷」段。
- 相关行为规格：`openspec/specs/release-durability-precondition/spec.md`（释放的持久化前置条件——限定在**释放**路径，未涵盖驱逐交错）。
