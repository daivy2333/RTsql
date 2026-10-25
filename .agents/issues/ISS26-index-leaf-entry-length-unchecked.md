# 索引树页级判据缺叶页条目长度检查（短于条目格式的叶条目被静默跳过）

- Status: open
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选 **IC-1** + Plan Review 的 F6（叶页短条目缺口非回归、明确在 Plan 范围外），change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 006 Cycle `000-initial.md`（Review Result `accepted`）；Iteration 007 的 Issue 候选 1 沿用同一编号
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `tests/index_read_bounded_read_test.rs` 集成测试、tokio 多线程 test runtime、单用例自带 `tempfile::tempdir()` 现场；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 预期：`integrity::check_tree_page` 判为「未通过页级判据」的索引树页，其上的条目**不得**被读路径当作「没有这个条目」静默跳过。
- 实际：`integrity::check_tree_page` 的**叶分支只校验槽边界**，**缺少内部节点分支已有的长度检查** ⇒ 槽负载短于条目格式（**38 字节**）的叶条目落在页内时 `violations` 为空（不被标记为损坏）⇒ `node.rs` 的 `get_key` / `get_row_id` 返回 `None` ⇒ `read_leaf_pair` 的 `filter_map` / `scan_all_async_from_root` / `find_key_position_binary` 把它当作「无此条目」⇒ 该条目被**静默跳过**。
- 位置：`src/storage/integrity.rs::check_tree_page`（叶分支的长度检查缺口；内部节点分支已有该检查）、`src/storage/btree/node.rs`（`get_key` / `get_row_id` 的 `None` 返回面）。
- 根因：Confirmed（读码确认——叶分支与内部节点分支的判据不一致，缺口在页级判据本身）。
- 触发条件：索引树页的某个叶槽负载被截短到不足条目格式长度（38 字节）——需要字节级损坏注入才能构造。

## 影响

- 当前：**静默错读**——损坏的叶条目不被报告，点查返回「查不到该行」而不是明确错误，用户无从察觉（与 `index-read-bounded-read` 域要求「点名损坏的页与该索引树」的处置方向相反）。
- 既有验收面的边界：承载 change 的 `index-read-bounded-read` 增量已把 Requirement 正文**收敛为「读取到未通过页级判据的索引树页时」**⇒ 短叶条目**未被**页级判据捕获，故**不在**该域已验收主张之内；IRBR 的 Requirement / Scenario **标题**仍写「损坏的索引页」而正文已收紧（措辞残余张力另立 Issue 登记）。
- 与释放路径的关系：**处置方向相反**——读 / 写路径应为「明确错误」，释放路径应为「计数 + 报出 + 跳过」（`release-path-bounded-read` 域的尽力而为语义）。
- 潜在：判据扩展须同时复核 `check_tree_page` 的既有释放面用法（承载 change Iteration 002 的成果）不受破坏。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 006，Plan Review F6 复核）：确认该缺口**非回归**（承载 change 未改该判据）、**明确在 Plan 范围外**（T23 Forbidden + Iteration 006 的 Non-goals）⇒ 登记为 Issue 候选 IC-1，不要求当 Cycle 修复。
- 2026-10-25（缺陷修复 change Iteration 007，T20 措辞收敛）：IRBR 的 Requirement 正文已收敛为「未通过页级判据的索引树页」⇒ 该行为被**排除在已验收主张之外**，但**缺陷本身仍在**。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在**索引树页级判据**（`integrity::check_tree_page` 的叶分支）→ 建议独立 change 承载「叶页条目长度的页级判据」，按**读 / 写路径 = 明确错误**、**释放路径 = 计数 + 报出 + 跳过**两条**相反**的处置方向分别设计（与承载 change 的 D13 同构）。
- 夹具可复用：字节级损坏注入手法已在承载 change 中验证（`tests/release_path_bounded_read_test.rs` / `tests/index_read_bounded_read_test.rs` 共用的 `slot_offset_field_offset` / `page_file_offset` / `corrupt_slot_offset`）。
- 与 IC-2 的边界：本条是**索引树叶页条目长度**这一具体判据缺口，IC-2 是**其余页型**（数据页 / 目录页 / 恢复面 / 缓冲池）的无界读点，根因位置与承载面不同，**不合并**。
- **编号说明（避免交叉引用误读）**：MS36 的 Plan Review 曾把它的两条 Minor finding / Acceptance Gap 以「**ISS26** / **ISS27**」之**候选名**报告（见 `.agents/docs/tasks.md` 的 MS36 段），但那两条**从未登记为 Issue 文件**，且已由缺陷修复 change 的 Iteration 007 闭合（G2 文件级样本 E7 与其四项断言已落地；矩阵 G2 行已升格为「文件级样本（E7）」）⇒ 本 Issue 与那两条候选**无主题关系**，编号自 ISS26 起顺接（既有登记最大为 ISS25）。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/006-index-read-bounded-read/000-initial.md` —— Act Response 的 Issue 候选 IC-1、Plan Review 的 F6。
- 沿用与措辞收敛：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md` —— Issue 候选 IC-1（沿用 Iteration 006 的登记，本 Cycle 未改变其状态）、Plan Review 的 F14（IRBR 标题措辞残余）。
- 移交记录：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/tasks.md`（收尾交接清单第 4 条「Iteration 006 登记的两条 Issue 候选（IC-1 / IC-2）的移交」）。
- 行为规格（该行为**不在**其已验收主张内）：`openspec/specs/index-read-bounded-read/spec.md`（正文已收敛为「未通过页级判据的索引树页」）。
