# 完成的 ALTER TABLE DROP COLUMN 在数据页写满后丢行（多页改写丢行）

- Status: closed
- Filed: 2026-10-24
- Closed: 2026-10-25
- Source: Act Response 的 Remaining Issues（候选 A）+ Plan Review 的 F1（多页数据改写路径属范围外）与 F11 第 6 条（两条尾指针的并发面），change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 Cycle `000-initial.md`（Review Result `accepted`）；关闭依据见「处置」与「证据」段（承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001）
- Environment: Linux x86_64 / WSL2、rustc 1.99.1 (58cf8f9c3 2026-10-13) + cargo 1.99.1、sqlparser-rs 0.44；登记时工作区 HEAD `0a08b14`（ahead of origin 9）；复现现场为 MS34 Issue 修复批实施期工作树（产品改动未提交）

## 缺陷描述

- 预期：完成的（非放弃的）`ALTER TABLE t DROP COLUMN v` 改写完全部行后行数守恒，且目录面与数据面行宽一致。
- 实际：数据页写满后**静默丢行**——预置 N 行后执行 `ALTER TABLE t DROP COLUMN v`，`SELECT * FROM t` 的可见行数少于 N。实测 N=40 → 25 行；N=60 → 10 行；N=80 → 50 行；N=100 → 35 行；N≤30 → 行数完整。
- 位置：`src/storage/data/table_manager.rs:897-1007`（`alter_drop_column` 的逐行改写循环：DELETE 腿 `:926-957` 把墓碑写进 `old` 的元数据，INSERT 腿 `:959-1006` 把窄化行写进 `new_meta`）、`src/storage/data_page.rs:18`（`write_tuple_to_data_page` 的新页分配与链上 `next` 指针写入）、`src/executor/alter_table.rs`（DDL 执行路径）。
- 根因线索：DELETE 腿与 INSERT 腿各持一条独立的 `data_page_tail`（`old` 与 `new_meta` 两条 Mutex）却指向同一条物理页链；页写满时两腿各自新分配页并互相覆盖链上的 `next` 指针 ⇒ 先前分配的页成为孤儿。
- 根因：Inferred（机制经读码定位，行数丢失经实测确认；两腿尾指针如何具体互相覆盖页链需在承载 change 中独立复核）。
- 触发条件：表行数足以把数据页写满（实测 N>30 命中），且删列语句走**完成路径**（不放弃、不失败）。

## 影响

- 当前：完成路径的删列在多页规模下静默丢行（数据丢失，无报错、无告警）——已完成且已提交的 DDL 之后表内容与源表不等价。
- 与本 change 范围的关系：**超出**。该 change 只处理**放弃/失败**路径的目录面原子性（目录窄化延后到提交点），数据面多页改写路径本次未改动；该现象在 T5 改动**之前**即已复现（RED 阶段实测），change 的目录面改动未触及该路径。
- 潜在：同一页链推进缺陷对其他「DELETE + INSERT 两腿各持一条尾指针」的改写路径同样成立（该形态在 `alter_drop_column` 之外是否还有别的调用点，登记时未逐一核对）。

## 事件记录

- 2026-10-24（MS34 Issue 修复批 Iteration 001，RED 阶段实测）：夹具 `t(id INT PRIMARY KEY, v INT, w INT)` 预置 N 行 → `ALTER TABLE t DROP COLUMN v` → `SELECT * FROM t` 可见行数少于 N；实测 40→25、60→10、80→50、100→35，N≤30 行数完整。缺陷形态在 T5 目录面改动之前即已稳定复现。
- 2026-10-25（缺陷修复 change Iteration 001，RED → GREEN）：承载 change 的 T4 先行用例 `drop_column_across_page_boundary_keeps_every_row_on_one_chain`（N=40，真实文件库 + 真实 `ALTER TABLE t DROP COLUMN v`）在修复前 RED 输出 `completed DROP COLUMN must preserve every row; got 25 of 40`（left: 25, right: 40），与本 Issue 登记的 40→25 同形；页级普查另见孤儿页 `{4}`（链 `{2,5}`，全部数据页 `{2,4,5}`）⇒ 登记时的根因线索（两条独立 `data_page_tail` 互相覆盖链上 `next` 指针）在承载 change 中被独立复核成立。

## 处置

- Status: closed（2026-10-25，用户指令关闭）。关闭原因：`fixed`——缺陷面已由 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 001「DDL 改写一致性」端到端修复并通过验收，该 change 已收尾归档为 `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/`（Cycle 000 Review Result `accepted`，T4–T7 全 done；收尾全量 14 批 / 143 目标 / **2231 passed / 0 failed / 2 ignored**）。
- 修复落点（实现形态）：改写的 DELETE 腿与 INSERT 腿**统一经同一条尾游标**——DELETE 腿由 `write_tuple_to_data_page(&self.buffer_pool, &old, …)` 改为 `self.write_tuple(&old, …)`，INSERT 腿由 `self.write_tuple(&new_meta, …)` 改为 `self.write_tuple(&old, …)` ⇒ 两腿共用同一条 `data_page_tail`、同一条自愈（`heal_stale_tail`）与目录尾指针回写路径，消除两条独立尾指针互相覆盖链上 `next` 指针的路径；改写结束后把 `*new_meta.data_page_tail` 同步为 `*old.data_page_tail`（提交点 `apply_column_drop` 把 `new_meta` 装成活表 meta，尾游标必须指向真实链尾）。落点：`src/storage/data/table_manager.rs::TableManager::alter_drop_column`。
- 修复结论（逐条对应本 Issue 的结论面）：
  1. 完成路径行数守恒——N=40 跨页容量边界下 `SELECT * FROM t` 的行数 == 改写前（40 == 40），`SELECT id FROM t` 的 id 集合逐行相等。
  2. 数据页链不分叉——主键索引指向的行所在页全部在数据页链上（无行落在孤儿页）；库内首字节 `0x03` 的数据页（排除目录面两条页链）全部在链上，无「被 `next` 覆盖而掉出链」的页。
  3. 重开库一致——完成改写后 close → reopen：40 行全部可见、完整性扫描 error 级发现为 0。
- 行为规格：`openspec/specs/ddl-rewrite-consistency/spec.md`（改写后行数守恒 / 数据页链不分叉 / 重开库一致；含「跨页容量边界规模 + close/reopen + 完整性 error 级发现为 0」三要素同一条场景）。
- 状态迁移记录：
  - 2026-10-25 `open` → `closed`：用户指令关闭；修复随归档 change 交付。回归承载：`tests/ddl_rewrite_consistency_test.rs`（6 用例，含本 Issue 的 N=40 用例；修复前 RED `got 25 of 40`，修复后 `6 passed; 0 failed`）+ 收尾全量 14 批 / 143 目标 / 2231/0/2。
- 关闭时保留的诚实边界：登记时「根因：Inferred（两腿尾指针如何具体互相覆盖页链需在承载 change 中独立复核）」在关闭时点已由承载 change 复核成立；登记时「潜在：同一页链推进缺陷对其他『DELETE + INSERT 两腿各持一条尾指针』的改写路径同样成立（登记时未逐一核对其他调用点）」**未逐一核对**，本 Issue 的结论面仅覆盖 `alter_drop_column` 这一条路径；其余「两腿各持尾指针」的调用点若存在，属**未核对的潜在面**，不属本 Issue 的已验收主张。
- 登记时的承载落点（建议，已完成）：原建议落在存储 / 数据面改写层并与 MS35 相邻排期；实际承载为非里程碑的缺陷修复 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 001「DDL 改写一致性」。
- 与 ISS23 / ISS24 的边界不变：三条根因位置与触发条件不同，**不合并**（ISS23 已于 2026-10-25 独立关闭于 Iteration 000；ISS24 已于 2026-10-25 独立关闭于 Iteration 001）。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md` —— Act Response 的 Remaining Issues 候选 A（实测行数与根因线索）、Plan Review 的 F1（多页数据改写路径被明确划为范围外，由候选 A / B 承载）与 F11 第 6 条（`old` 与 `new_meta` 两条尾指针的并发面）。
- 登记时（2026-10-24）独立复核的代码位点：`src/storage/data/table_manager.rs:897-1007`（改写循环两腿）、`src/storage/data_page.rs:18`（`write_tuple_to_data_page`）
- 关闭时证据：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/001-ddl-rewrite-consistency/000-initial.md` —— Act Response 的 T4 / T5 实际改动（单一尾游标 + `new_meta.data_page_tail` 同步）与 AR1 判据（行数守恒 / id 集合逐行相等 / 页级普查无孤儿页）、RED → GREEN 表（`got 25 of 40` → `6 passed; 0 failed`）；Plan Review 的 F1–F5（ISS24 四层判别结论的独立复核、ISS22 与 ISS24 **不耦合**的结构性依据：N=30 时 30 行 + 3 个版本槽全在同一个数据页 ⇒ 该规模下链分叉在结构上不可能）与 Deviation 2 / 3 的裁定；行为规格 `openspec/specs/ddl-rewrite-consistency/spec.md`
- 关闭时实现位点：`src/storage/data/table_manager.rs::TableManager::alter_drop_column`（两腿统一经 `self.write_tuple(&old, …)`；改写后 `*new_meta.data_page_tail` 同步为 `*old.data_page_tail`）
