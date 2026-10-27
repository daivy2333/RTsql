# 存储层其余页型的无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处）

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选 **IC-2**，change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 006 Cycle `000-initial.md`（Review Result `accepted`）；Iteration 007 的 Issue 候选 2 沿用同一编号
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `tests/release_path_bounded_read_test.rs` / `tests/index_read_bounded_read_test.rs` 集成测试；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 预期：存储层对页内容的读取 SHALL 经由**有边界检查**的路径；损坏页上的读取 SHALL 给出明确错误或按该页型的既定语义处理，SHALL NOT 造成越界切片 / 进程 abort。
- 实际：`SlottedPageRef::get_slot_data` 的**未检查调用点**分布在存储层其余页型——`src/storage/data_page.rs` / `src/storage/catalog.rs` / `src/storage/buffer_pool.rs` / `src/storage/data/table_manager.rs` / `src/wal/recovery.rs` / `src/executor/*`，**同形态约 17 处**。`get_slot_data` 直接按页内槽目录的 `offset` / `length` 切字节，**无范围检查** ⇒ 槽偏移或长度越界 ⇒ 越界切片 panic（panic 发生在异步执行路径 ⇒ 进程 abort）。
- 位置：`src/storage/page_format/slotted_page.rs::SlottedPageRef::get_slot_data`（无界读的实现点）；上述各页型的调用点分布面。
- 根因：Confirmed（读码确认——`get_slot_data` 无范围检查；调用点分布由全仓检索确定）。
- 触发条件：槽目录项的 `offset` / `length` 字段被损坏到越界 ⇒ 每次读取该槽的路径 panic。

## 影响

- 当前：**进程 abort**（panic 发生在异步执行路径）——数据页 / 目录页 / 恢复面 / 缓冲池的读取都可能因单个损坏槽而中止整个进程。
- 各页型的**正确行为不同**：索引树页是「下降决策」（读 / 写路径应为「明确错误」，释放路径为「计数 + 报出 + 跳过」），数据页是「行内容」、目录页是「目录行」⇒ 处置方向不能一次统一。
- 已被承载 change 覆盖的部分（不属本条）：索引树页的**释放路径**（`release-path-bounded-read` 域，Iteration 002）与**读 / 写路径**（`index-read-bounded-read` 域，Iteration 006）已分别修复 ⇒ 本条只覆盖**其余页型**。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 006，Act Response）：全仓 `get_slot_data` 的未检查调用点按页型清点并登记为 Issue 候选 IC-2；本 Cycle 明确不承载（Non-goals + 验证预算：每组页型需各自的损坏形态与恢复语义判定）。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在**存储层页内容读取的有界化** → 建议按**页型分组**另立 change（每组各自判定「损坏页在该页型的正确行为」），避免一次 change 混合不同处置方向。
- 夹具可复用：每个页型的字节级损坏注入沿用承载 change 已验证的手法（`slot_offset_field_offset` / `page_file_offset` / `corrupt_slot_offset`）。
- 与 IC-1 的边界：本条是**其余页型**的无界读点面（不含索引树叶页条目长度判据，另有 ISS26 登记），根因位置与承载面不同，**不合并**。
- **编号说明（避免交叉引用误读）**：MS36 的 Plan Review 曾把它的两条 Minor finding / Acceptance Gap 以「**ISS26** / **ISS27**」之**候选名**报告（见 `.agents/docs/tasks.md` 的 MS36 段），但那两条**从未登记为 Issue 文件**，且已由缺陷修复 change 的 Iteration 007 闭合（G2 文件级样本 E7 与其四项断言已落地；矩阵 G2 行已升格为「文件级样本（E7）」）⇒ 本 Issue 与那两条候选**无主题关系**，编号自 ISS26 起顺接（既有登记最大为 ISS25）。
- 2026-10-26 关闭。原因：数据页、目录页、恢复面三组无界读已分别由归档变更 Iteration 001（T3）、Iteration 004（T4）、Iteration 005（T5）处置。三个 Cycle 的 Plan Review 均为 `accepted`。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/001-data-page-bounded-read/000-initial.md`、`iterations/004-catalog-bounded-read/000-initial.md`、`iterations/005-recovery-bounded-read/000-initial.md` 的 Plan Review 均为 `accepted`（T3 / T4 / T5）。
- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/006-index-read-bounded-read/000-initial.md` —— Act Response 的 Issue 候选 IC-2。
- 沿用：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md` —— Issue 候选 IC-2（沿用 Iteration 006 的登记）。
- 移交记录：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/tasks.md`（收尾交接清单第 4 条）。
- 参照实现与既有修复面：`openspec/specs/release-path-bounded-read/spec.md`、`openspec/specs/index-read-bounded-read/spec.md`。
