# B 树插入路径无限递归致栈溢出（约 300 行以上触发，进程 abort）

- Status: closed
- Filed: 2026-10-24
- Closed: 2026-10-25
- Source: Act Response 的 Remaining Issues（候选 B）+ Plan Review 的 F1（规模相关路径属范围外），change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 Cycle `000-initial.md`（Review Result `accepted`）；关闭依据见「处置」与「证据」段（承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 000）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 (58cf8f9c3 2026-10-13) + cargo 1.99.1、sqlparser-rs 0.44；登记时工作区 HEAD `0a08b14`（ahead of origin 9）；复现现场为 MS34 Issue 修复批实施期工作树（产品改动未提交）

## 缺陷描述

- 预期：表行数增长到数千行时，索引插入不崩溃、不产生无限下降。
- 实际：行数达到**约 300 以上**时，B 树插入路径无限递归 → `thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT（signal 6），整个测试进程 abort（不止当前语句失败）。
- 位置：`src/storage/btree/btree.rs:237-327`（`BTree::insert_into_page`；`:319-327` 内部节点取 `child_page_id` 后递归下降）。
- 根因：Inferred（gdb 现场显示 35+ 层递归帧停在 `src/storage/btree/btree.rs:327`，帧内为**同一 `page_id`、同一 key 指针** ⇒ 下降形成环；但环的形成机制未独立复核——该文件在承载 change 中未改动）。
- 触发条件：表行数达到约 300 以上并走**完成路径**的 `ALTER TABLE DROP COLUMN`（该路径的 INSERT 腿会向索引插入窄化后的行键）；实测 N=300 / 600 / 2000 均命中。

## 影响

- 当前：**整个进程 abort**（SIGABRT），需重开库；崩溃点与取消无关，出现在索引插入路径上，同一句柄的任何后续写语句都可能踩到。
- 与本 change 范围的关系：**超出**。承载 change 改的是索引写单元的原子性与写侧串行化（`src/storage/btree/index_manager.rs`），**未触及** `btree.rs` 的分裂 / 下降路径；该现象在 T5 改动**之前**即已复现。
- 与 ISS20（已关闭）的边界：ISS20 的 E1 是取消落在「结构变更已生效、根页 id 未发布」窗口后树结构损坏、沿子指针 0 无限下降；本条出现在**完成路径**、由行数规模触发、与取消无关 ⇒ 不在 ISS20 的缺陷面内。两者是否为同一根因的不同表现，**需在承载 change 中独立判定**（ISS20 的已修机制不覆盖本路径）。

## 事件记录

- 2026-10-24（MS34 Issue 修复批 Iteration 001，RED 阶段实测）：同候选 A 的夹具，N=300 / 600 / 2000 → `thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT（signal 6），整个测试进程 abort。gdb 现场：`BTree::insert_into_page` 连续 35+ 层停在 `src/storage/btree/btree.rs:327`（内部节点的 `child_page_id` 递归下降调用点），同一 `page_id`、同一 key 指针 ⇒ 下降形成环。
- 2026-10-25（缺陷修复 change Iteration 000，归因更正）：承载 change 在 B-2 层（`btree.rs:610` `merge_leaves` → `:672` `handle_child_merge`）复核出**同一页被释放两次** ⇒ 同一页号进空闲列表两次并被交给两个对象 ⇒ 后续插入沿该页号下降形成环。修复 B-2 后本缺陷的栈溢出**不再复现**（250 行、625 行两个规模均干净）；另加 `BTree::insert_into_page` 下降路径成环守卫作为第二道防线（成环 ⇒ 返回 `StorageError::Internal`，不再耗尽栈）。

## 处置

- Status: closed（2026-10-25，用户指令关闭）。关闭原因：`fixed`——缺陷面已由 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 000「索引页归属」端到端修复并通过验收，该 change 已收尾归档为 `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/`（Cycle 000 Review Result `accepted`，T1–T3 全 done；收尾全量 14 批 / 143 目标 / **2231 passed / 0 failed / 2 ignored**）。
- **归因更正（登记时的结论被承载 change 独立复核推翻，按公共规则「不改写已发生的历史」记于本段）**：登记时把本条记为「独立的规模相关缺陷，根因 Inferred（下降形成环，机制未独立复核）」。承载 change 的复核结论是——**本条不是独立成因，而是 B-2（合并路径的重复释放）的下游后果**：B-2 使同一页号两次进入空闲列表并被交给两个对象 ⇒ 索引树结构出现同一页号的多重归属 ⇒ 插入下降时沿该页号反复下降形成环 ⇒ 栈溢出。Plan Review 明确记载：「ISS23 的缺陷机制已被本 Iteration 闭合（页不再被交给两个对象 + 下降必然终止），可由 Recorder 据本节结论关闭」。
- 修复落点（实现形态）：
  1. **消除合并路径的重复释放**（结构性修复）：`BTree::handle_child_merge`（`src/storage/btree/btree.rs`）删除「再次释放 `merge_info.freed_page_id`」的整块（`free_page` + `complete_free_release`）。释放与释放完成标记改由该页的**产出方**发出——`merge_leaves`（叶合并，`:640` 释放 / `:652` 标记）与 `merge_internal_nodes`（内部节点合并，`:877` 释放 / `:889` 标记）都在各自「父节点分隔键移除」这一锚点写之后释放并标记。连带移除 `BTree::MergeInfo` 的 `freed_page_id` 字段与 4 处构造点（`:547` / `:654` / `:727` / `:921`）⇒ **页号不再随合并结果上浮 ⇒ 重复释放在结构上不可复现**（防回归）。
  2. **下降路径成环守卫**（第二道防线）：`BTree::insert_into_page` 拆为「守卫 + 步骤」两半——持下降路径 `descent: &mut Vec<PageId>`，路径上出现重复页号即返回 `StorageError::Internal("index descent cycle detected at page N")`；原函数体变为 `insert_into_page_step`。正常插入的下降路径页号互不相同 ⇒ 守卫不触发，既有语义与性能形态不变（守卫成本 = 每层一次 `Vec::contains`，深度为 1–3）。
- 修复结论（逐条对应本 Issue 的结论面）：
  1. 栈溢出不再复现——修复后 250 行与 625 行两个规模均干净，无 `has overflowed its stack`、无 SIGABRT。
  2. 完成路径的 `ALTER TABLE DROP COLUMN` 不再耗尽栈——用例 `alter_table_drop_column_completes_without_exhausting_the_stack`。
  3. 成环形态有确定结果而非耗尽栈——用例 `insert_into_a_cyclic_structure_returns_an_error` 返回明确错误。
- 行为规格：`openspec/specs/index-page-ownership/spec.md`（页不再被交给两个对象 + 下降必然终止）。
- 状态迁移记录：
  - 2026-10-25 `open` → `closed`：用户指令关闭；修复随归档 change 交付。回归承载：`tests/index_page_ownership_test.rs`（5 用例）+ `tests/index_page_ownership_stack_overflow_test.rs`（1 用例，ISS23 复现）；修复前 RED-1 / RED-2 / RED-3 稳定命中「可分配集合中同一页号不出现两次」（`[6, 9, 9, 7, 7, 5, 5, …]` / `[398, 398, 397, 397, …]`）+ 收尾全量 14 批 / 143 目标 / 2231/0/2。
- 关闭时保留的诚实边界：承载 change 的 Plan Review 同时确认「ISS22 与 ISS24 **不耦合**」（各自的 RED 症状与规模）——该结论只用于说明**本条与那两条**的边界，不改变本条自身的归因更正。登记时「与 ISS20（已关闭）的边界……需在承载 change 中独立判定」一句，其判定结果即上面的归因更正：ISS23 的机制由 B-2 的修复承担，ISS20 已修机制（写单元原子化 + 写侧串行化）**不覆盖**本路径，本条另有独立承载。
- 登记时的承载落点（建议，已完成）：原建议落在存储 / 索引层并与 MS35 相邻排期；实际承载为非里程碑的缺陷修复 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 000「索引页归属」。
- 与 ISS22 的边界不变：本条是**进程级栈溢出**（索引层），ISS22 是**丢行**（数据面改写层），根因位置不同，**不合并**（ISS22 已于 2026-10-25 独立关闭于 Iteration 001）。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md` —— Act Response 的 Remaining Issues 候选 B（N=300/600/2000 的实测与 gdb 现场）、Plan Review 的 F1（规模相关路径被明确划为范围外）。
- 登记时（2026-10-24）独立复核的代码位点：`src/storage/btree/btree.rs:319-327`（内部节点下降递归调用点）、`src/storage/btree/btree.rs:237-327`（`insert_into_page` 全函数）
- 关闭时证据：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/000-index-page-ownership/000-initial.md` —— Act Response 的 T1 / T2 / T3 实际改动（`handle_child_merge` 移除重复释放 + `MergeInfo.freed_page_id` 字段移除 + `insert_into_page` 下降成环守卫）与 T1 / T2 的先行测试 RED 证据（RED-1 库级 / RED-2 树级 / RED-3 内部节点合并形态，页号 9 / 7 / 5 各出现两次；重复页号含 401 / 351 / 301 / 251 / 201 / 151 / 100）；Plan Review 的归因结论（ISS23 的缺陷机制已被本 Iteration 闭合：页不再被交给两个对象 + 下降必然终止 ⇒ 可关闭）；行为规格 `openspec/specs/index-page-ownership/spec.md`
- 关闭时实现位点：`src/storage/btree/btree.rs`（`handle_child_merge` / `merge_leaves` / `merge_internal_nodes` / `MergeInfo` / `insert_into_page` + `insert_into_page_step`）
