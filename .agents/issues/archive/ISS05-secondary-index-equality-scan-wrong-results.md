# ISS05 二级索引等值扫描结果集错误（重复发射 + 索引条目缺失）

- Status: closed
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P0-1）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

非唯一二级索引的等值查询结果集与全表扫描不一致，静默错结果、无任何报错。两个独立机制叠加：

1. **重复发射（根因 Confirmed）**：`src/storage/btree/index_manager.rs:214-237` `search_all_from_page_async` 对**每一个**等于查找键的分隔键都收集其左右两个孩子且无 break——相邻两个等值分隔键共享的中间孩子被收集两次，递归逐层相乘（多行同值键形成等值分隔键连串时放大率随规模增长，实测 2.6×）。消费面：`src/executor/index_scan_all.rs:84`（唯一生产调用者）。
2. **索引条目缺失（现象 Confirmed，产生行 Unconfirmed）**：部分行（实测为最早插入批的 11 行）的索引条目不在树中——对缺失行 `UPDATE t SET n=51 WHERE id=…` 触发 `Execution error: Key not found`（delete_exact 找不到旧键条目）。候选来源：`src/executor/create_index.rs:105-183` 回填漏行，或 `src/storage/btree/btree.rs:262-295` 叶分裂/再插入丢条目；待 Plan 插桩定位。

潜伏变体：同步版 `src/storage/btree/btree.rs:1070-1131` `search_all_from_page` 带 break（只搜首个等值分隔键两侧）为漏行变体，当前无生产调用者。

预期：索引扫描结果集与 DataScan 严格相等（双向：不重复、不遗漏）。实际：30000 行表 `WHERE n=50` 真值 281 行，建索引后发射 493 次、仅覆盖 270 行；DROP INDEX 恢复、重建复现（确定性）。范围谓词与复合谓词（不路由 IndexScanAll）及唯一键索引不受影响。既有单测只构造 2-3 条目小树，等值分隔键连串需多叶规模，故未覆盖。

## 影响

正确性红线：带二级索引的等值查询静默返回错误结果（COUNT 虚高 + 漏行），外部评测实际命中。当前唯一规避为不建二级索引或避免等值谓词。

## 事件记录

- 2026-10-12：外部评测在 30000 行表上实测发现（`COUNT(*)` 558/309 波动因数据分布不同，本环境复现为 493/270/281）；Explorer 在 HEAD 构建上确定性复现并定位机制 A 代码行。检测：评测对比建索引前后计数；无引擎内报错。恢复：DROP INDEX。
- 2026-10-12（顺带发现，超出评测报告）：对缺失条目行的失败 UPDATE 留下索引副作用——报错后 COUNT 493→495、DISTINCT 270→271（abort 重插路径补写了条目），失败语句零副作用语义在索引面被违反；随本缺陷一并修复验收。

## 处置

closed（2026-10-14）——change `2026-10-13-e2e-verification-and-defect-closure` Iteration 001 T2 修复归档。机制调查修正原假设：回填路径无辜（描述第 2 条的「索引条目缺失」非回填漏行），真凶为等值下探遇等值分隔键跳过边界子树——`search_all_from_page_async` 改闭区间下探 + `BTree::delete_exact` 同集合下探；调查中定位的同面新缺陷一并修复：B1 `LeafNode::shift_slots_right` 叶链指针清零、B2 `IndexManager::scan_all` 内部页当叶读 + page-0 终止符（多层树全表 DML 静默失真）。验收即原建议判据：30000 行多叶等值连串结果集与全表扫描双向严格相等（不重复、不遗漏）+ 失败语句（含 UPDATE 旧键删除路径）索引零副作用；`tests/secondary_index_equality_test.rs` 落地。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（复现命令、代码行、对照实验：唯一键索引正确 / 8000 行 85% 同值表 2.6× 重复 / 缺失行 KeyNotFound 探针）；复现现场 `/tmp/rt-verify/t1.db`、`/tmp/rt-verify/t2.db`。
