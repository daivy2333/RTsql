# 取消 + 回滚下索引写侧无原子性（B 树结构损坏致进程 abort；被取消语句的键残留在主键索引）

- Status: closed
- Filed: 2026-10-24
- Closed: 2026-10-24
- Source: Act Response 的 Experience Candidates（E1 / E2）+ Plan Review 的 Issue 候选复核 F6（change `2026-10-24-ms34-cancellation-timeout-transaction` Iteration 001 Cycle `001-replan.md`，Review Result `accepted`）
- Environment: Linux x86_64 / WSL2、rustc 1.99.1 (58cf8f9c3 2026-10-13) + cargo 1.99.1、sqlparser-rs 0.44；登记时工作区 HEAD `0a08b14`（ahead of origin 9）；复现现场为 MS34 实施期工作树（父 Cycle 的 `src/database.rs` / `src/pipeline.rs` / `src/executor/alter_table.rs` / `src/transaction/manager.rs` 未提交改动）

## 缺陷描述

**E1（进程级 abort）**

- 预期：显式事务内一条多行 `INSERT … SELECT` 被取消后调用方回滚，索引结构应保持一致；之后的索引写入不得崩溃。
- 实际：B 树结构损坏——之后的一次索引插入沿「内部节点的子指针 = 0」无限下降，栈溢出使整个进程 abort（SIGABRT）。
- 位置：`src/storage/btree/btree.rs:237-327`（`BTree::insert_into_page`：`:243` 取页、`:248` 叶子/内部判定 `data_guard[0] == LEAF_NODE`、`:319-327` 内部节点取 `child_page_id` 并递归下降）；损坏入口 `src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert`：B 树条目在 `spawn_blocking` 内写入，tokio 的 blocking 任务不可取消；反向映射 `row_to_key` 在其后的 await 之后才写）与 `src/storage/btree/index_manager.rs:261-282`（`IndexManager::delete`：回滚路径删条目时会合并/重分布叶子并可能换根）。两条写路径之间没有写者串行化，根页 id 的更新与树内容变更也不在同一临界区。
- 根因：Confirmed（机制经 Plan Review 独立读码确认；**复现为竞态依赖**——6 次复现尝试未命中）。

**E2（键残留）**

- 预期：被取消语句写入的键在回滚后应可再次插入。
- 实际：被取消语句写入的键（**首个键之外**）仍留在主键索引的正向条目里，同一个键再插入报 `DuplicateKey`；行本身不可见。
- 位置：`src/storage/btree/index_manager.rs:239-257`（两段写：正向条目在 `:244-248` 的 `spawn_blocking` 内完成，反向映射 `row_to_key.insert` 在 `:256` 才写——取消落在两者之间时，正向条目 key→row_id 已在、反向映射 row_id→key 缺失）与 `src/transaction/manager.rs:339-355`（`abort_cleanup_versions` 的 A/B 分桶只查 `find_key_by_row_id`；反向映射缺失的行被误判为「墓碑」而跳过，其正向索引条目不被移除）。
- 根因：Confirmed（首版 RED 实测 + 诊断现场 + Plan Review 独立复现）。
- 显式路径与隐式路径、RR 与 RC 同形。

**合并说明**：E1 与 E2 同属「索引写侧结构变更缺原子性」一个根因族——正向条目、反向映射与根页 id 未在同一临界区原子可见——故并入同一 Issue，不另开文件。

## 影响

- E1（当前）：**整个进程崩溃**（不止当前语句失败），且崩溃点与取消无关，出现在之后的任意索引写入上——同一句柄上任何后续写语句都可能踩到。取消窗口越大越容易触发（100ms 窗口取消前约 523 行 → 稳定复现；20ms 窗口约 100 行 → 本机 8 次未复现）。
- E2（当前）：被取消语句占用的键无法复用（`DuplicateKey`），且残留索引条目指向已中止的行。隐式回收路径同样受影响——MS34 的用例只复用了**首个**键，因此没有暴露这一点。
- 潜在：隐式事务的回收路径（同样调 `abort_by_id`）可能同形命中 E1；E1 的树结构损坏是否已落盘并在重开后复现，本次未确认。

## 事件记录

- 2026-10-24（MS34 实施期，E1）：父 Cycle 的 `cargo test --test statement_cancellation_test timeout_in_explicit_tx_is_not_a_rollback`（超时窗口 100ms）连续两次复现。检测与定位：gdb 现场 —— `thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT；栈上是 339+ 层 `BTree::insert_into_page`，帧内 `child_page_id_u32 = 0`（内部节点的子指针为 0 → 反复下降到第 0 页）；崩溃那次插入的键为被取消语句写入的键 111；`BTree::root_page_id = 75`，而该行的 `RowId.page_id` 也是 75（根页 id 指向了一个不是 B 树节点的页）。恢复：进程 abort，需重开库；损坏是否已落盘未确认。
- 2026-10-24（MS34 实施期，E2）：父 Cycle 2.2 用例首版（同表同键重复「放弃 → 提交」）实测 RED —— `the statement finished before the abandon anchor was reached: Err(SqlError { message: "Execution error: Duplicate key" })`。诊断现场（回滚前）：活跃事务的版本集合 = `{RowId{page 4, slot 0}, RowId{page 4, slot 1}, RowId{page 4, slot 2}}`；`find_key_by_row_id(slot 0) = Some(key 1)`、`find_key_by_row_id(slot 1) = None`、`find_key_by_row_id(slot 2) = Some(key 9999)`；索引里 `search(key 1) = Some(slot 0)`、`search(key 2) = Some(slot 1)`。回滚后：`search(key 1) = None`（已释放）、`search(key 2) = Some(slot 1)`（残留）。Plan Review 另独立复现：显式路径与隐式路径、RR 与 RC 各一组——键 2 报 `Duplicate key`，键 1 与 3..10 成功。
- 2026-10-24（MS34 Issue 修复批 Iteration 000，修复前 RED 实测）：本缺陷在承载 change 的 T1 先行 RED 阶段被**确定性**复现——写侧暂停点夹具把取消钉在「结构变更已生效、根页 id 尚未发布」的窗口内，实跑 3/3 连续稳定命中产品断言（键 47..92 不可达；显式与隐式路径各报 `Duplicate key`；并发写者只落 574/551/554 条，均 < 600）；修复后同组用例 5 次连续 GREEN（`test result: ok. 5 passed; 0 failed`）。登记时「E1 复现为竞态依赖、6 次复现尝试未命中」的记述在该时点被确定性夹具替代。

## 处置

- Status: closed（2026-10-24，用户指令关闭）。关闭原因：`fixed`——缺陷面已由 change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 的 Iteration 000「索引写侧原子性」端到端修复并通过验收，该 change 已收尾归档为 `openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/`（Cycle 000 Review Result `accepted`，T1–T3 全 done）。
- 修复落点：`src/storage/btree/index_manager.rs`——索引写单元把**结构变更 + 根页 id 发布 + 反向映射更新**合并进同一 `spawn_blocking` 闭包内的不可分割临界区（取消不留下半写状态）；同一索引的结构写侧经写侧闸门（`write_gate`）串行化，根页 id 的读取移入闸内（并发写者读不到彼此的中间根）；反向映射读（`find_key_by_row_id`）与写单元串行。夹具 `IndexManager::arm_write_pause` 把「结构变更已生效、根页 id 尚未发布」钉成确定窗口（语义位置修复前后不变）。
- 修复结论（逐条对应本 Issue 的 E1 / E2 结论面）：
  1. 索引写单元（结构变更 + 根页 id 发布 + 反向映射更新）不可被取消分割 —— E1 的「结构已生效、根页 id 未发布」半写窗口消失。
  2. 同一索引的结构写侧串行化 —— 并发写者不再互相覆盖中间根。
  3. 取消/回滚释放被取消语句占用的**全部**键 —— E2 的键残留消失（同键可再次插入、不报 `DuplicateKey`）。
  4. 取消/回滚后索引结构自洽 —— 后续写语句不再 abort，`check_integrity` 无 Error 级发现。
- 行为规格：新域 `openspec/specs/index-write-side-atomicity/spec.md`（4 Requirement / 5 Scenario）——R1 索引写单元不可被取消分割、R2 同一索引的结构写侧串行化、R3 取消或回滚释放被取消语句占用的全部键、R4 取消或回滚之后索引结构保持一致。`openspec/specs/statement-cancellation-outcome/spec.md` 的 R2 已知边界同步改写为「键的释放不由本域保证，由 `index-write-side-atomicity` R3/R4 承载」（原描述的缺陷机制已修复，不再陈述为当前行为）。
- 状态迁移记录：
  - 2026-10-24 `open` → `closed`：用户指令关闭；修复随归档 change 交付。回归承载：`tests/index_write_atomicity_test.rs` 5 用例（修复前 3/3 连续稳定命中产品断言的 RED，修复后 5 次连续 GREEN：`test result: ok. 5 passed; 0 failed`）+ `tests/statement_cancellation_test.rs` 既有 MS34 用例的增补断言（只增不减）。
- 登记时建议与实际实现的对应（不改写登记时的记录）：登记时的第 1/2 条建议按「写单元原子化 + 写侧串行化」实现；第 3 条（让 `abort_cleanup_versions` 的分桶不只依赖反向映射）**未改** `abort_cleanup_versions` 的分桶逻辑，改为把反向映射读与写单元串行——归档 change 的 Plan Review F1 判定该最小机制足以兑现 R3（不改变 R3 语义、不扩大保证面、不新增公共 API）；第 4 条回归用例已按建议落地。
- 登记时的承载落点（建议，已完成）：落在**存储/索引层**由存储域 change 承载，建议与 **MS35「空闲页跨重启复用」相邻排期**——该承载最终以非里程碑的 Issue 修复批交付（MS35 / MS36 的 Related changes 已登记该归档 change），相邻排期的最终裁定仍属 milestone-planner。
- 关闭时保留的诚实边界：登记时「E1 的树结构损坏是否已落盘并在重开后复现」这一形态未单独验证；修复消除了产生该损坏的机制，可观察面由 R4 覆盖（取消后索引结构自洽 + 完整性无 Error 级发现），未单独构造「损坏树已落盘后重开」的夹具。
- 与本 change 范围的关系（登记时记录，不改写）：修复点落在存储/索引层的写者串行化与取消时的原子性，不在「取消/超时/事务收尾」的调用方语义面，超出 MS34 的 Impact 与 design D1–D6；MS34 已 accepted 归档，本缺陷不在 MS34 内修。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/iterations/001-caller-visible-outcomes-and-final-regression/001-replan.md` —— `Act Response` 的 Experience Candidates 段（E1 / E2）与 `Plan Review` 的 F6（可承载格式复核、根因位置独立读码核对；Review Result `accepted`）。
- 父 Cycle（首版 RED 与 gdb 现场所在）：`openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/iterations/001-caller-visible-outcomes-and-final-regression/000-initial.md`
- change 根目录（Impact、design D1–D6、规格 delta 与已知边界注记）：`openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/`
- 登记时（2026-10-24）独立复核的代码位点：`src/storage/btree/index_manager.rs:239-257`、`:261-282`；`src/storage/btree/btree.rs:237-327`（`:243` 取页 / `:248` 叶子判定 / `:319-327` 下降递归）；`src/transaction/manager.rs:339-355`
- 关闭时证据：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/000-index-write-side-atomicity/000-initial.md` —— Act Response 的 T1 / T2 / T3 实现说明与 Verification Evidence 表（RED 3/3 连续、GREEN 5 次连续）；Plan Review 的 F1（反向映射读与写单元串行化的裁定）、F5（写闸临界区覆盖完整闭环、根页读移入闭包）、F8（四条 requirement 的判别力核对）；行为规格 `openspec/specs/index-write-side-atomicity/spec.md` R1–R4
- 关闭时行为规格：`openspec/specs/index-write-side-atomicity/spec.md`（4 Requirement / 5 Scenario）；`openspec/specs/statement-cancellation-outcome/spec.md` R2 已知边界的同步改写
