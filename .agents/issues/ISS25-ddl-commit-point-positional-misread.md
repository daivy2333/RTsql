# DDL 提交点窗口内的位置性错读（窄 tuple 在未移位目录下按位置取值）

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-24
- Last updated: 2026-10-25（可达性坐实 + 并发读变体已修复；崩溃-重开变体仍开放）。2026-10-26 关闭：变体 (b) 已由该归档变更 Iteration 006 / T6 处置。
- Source: Act Response 的 Remaining Issues（候选 D）+ Plan Review 的 F6（design D5「不会静默错读」主张不准确、就地更正；该窗口判为 Non-goal），change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 Cycle `000-initial.md`（Review Result `accepted`）；更新依据见「事件记录」与「证据」段（承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 003）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 (58cf8f9c3 2026-10-13) + cargo 1.99.1、sqlparser-rs 0.44；登记时工作区 HEAD `0a08b14`（ahead of origin 9）；复现现场为 MS34 Issue 修复批实施期工作树（产品改动未提交）

## 缺陷描述

- 预期：DDL 提交完成、目录应用尚未发生的窗口内，读取已被窄化的行不应按旧列序错位取值——要么报明确错误，要么读出的值与列定义对应。
- 实际（**读码结论，2026-10-25 更新：可达性已实跑坐实；并发读变体已修复，崩溃-重开变体仍开放**）：宽度感知的解码按**位置**取值、尾部补 `Value::Null`、**不校验列序** ⇒ 2 宽的元组在未移位的 3 列目录下读作 `[id, w, NULL]`——`v` 列读到 `w` 的值、`w` 列读 NULL；影响面是全部已改写的行，且**无声**（不报错）。2026-10-25 的实测读数（窗口内崩溃后重开并查询）：重开后目录仍广告旧列数（`[ColumnMeta { name: "id", … }, ColumnMeta { name: "v", … }, ColumnMeta { name: "w", … }]`），读出 `[Int(1), Int(100), Null], [Int(2), Int(200), Null], [Int(3), Int(300), Null], [Int(4), Int(400), Null], [Int(5), Int(500), Null]` ⇒ 仍按位置错位（`v` 读到 `w` 的值、`w` 读 NULL）。**窗口内的并发读变体已由承载 change 修复**（见下）。
- 位置：`src/storage/page_format/tuple.rs:189-222`（`deserialize_tuple_width`；全部读路径经它——`src/executor/scan.rs`、`src/executor/data_scan.rs`、`src/executor/index_scan*.rs`，旧的严格入口 `deserialize_tuple` 不在读路径上）；重开库的活布局取自目录列行并按宣告列数截断（`src/storage/data/table_manager.rs:284-299`）。
- 根因：Confirmed（读码确定——按位置取值且无列序校验）；**可达性 2026-10-25 更新为 Confirmed**（并发读变体与崩溃-重开变体均由承载 change 的 Iteration 003 实跑坐实，见「事件记录」）。
- 触发条件（2026-10-25 更新为 Confirmed，两个变体）：(a) **窗口内的并发读**——改写已写出窄化行而目录尚未窄化期间读取该表（**已修复**：现为显式拒绝，不再静默错读）；(b) **窗口内的崩溃后重开并查询**——「提交完成 → 目录应用」的窗口内崩溃，在途状态（纯内存）随进程消失 ⇒ 重开后目录仍广告旧列数而数据面已是窄化后的已提交行 ⇒ 读仍按位置错位（**仍开放**）。

## 影响

- 当前：**读码结论**——若窗口可达，则窄化后的表在窗口内被整表按错误列序解释（每列读到相邻列的值），结果无声且错误。**2026-10-25 更新**：并发读变体的后果已消除（改为显式拒绝）；崩溃-重开变体的后果仍在（重开后仍按位置错读）。
- 与本 change 范围的关系：**超出**。承载 change 的最终 Plan Review 已把 design D5 的「不会静默错读」主张**就地更正为不准确**（design 是长期技术记录，随 change 归档），并判定该窗口属该 change 的 Non-goal（MS21 D10 的既有崩溃边界），作为范围外的后续工作交给承载。
- 对照面：现状（目录已窄化 + 数据仍宽）是**镜像方向**的同一类位置性错读（`w` 列读到 `v` 的值，宽行尾部被忽略），两者影响行集相当；可靠的差别只在「丢的是目录效果（目录未改写 ⇒ 该 DDL 可重做）还是数据改写（需重做改写）」。
- 登记时的诚实边界（2026-10-25 已失效，保留原话）：**本条未构造故障注入夹具、未实跑复现**，不得按「已复现」引用。⇒ 该边界在 2026-10-25 被承载 change Iteration 003 的实跑取代：可达性**已坐实**，并发读变体**已修复**，崩溃-重开变体**仍开放**（本 Issue 保持 `open` 的唯一理由）。

## 事件记录

- 2026-10-24（MS34 Issue 修复批 Iteration 001，读码阶段）：Plan Review F6 独立读码核对确认 design D5 的「不会静默错读」主张不准确——`deserialize_tuple_width` 按位置取值、尾部补 NULL、不校验列序，且全部读路径都走这条解码入口；该 Cycle 未构造崩溃点实跑验证（需新的故障注入夹具，超出该 Cycle 范围）。
- 2026-10-25（缺陷修复 change Iteration 003，Plan Context / Act Response / Plan Review 三处记录）：可达性**已实跑坐实**，两个触发变体的现状分开记录——
  - **变体 (a)「窗口内并发读」——已闭合**。承载 change 引入**纯内存**的窄化在途状态（`NarrowingState`：`table: String` + `in_flight: AtomicBool`；`ensure_readable()` 在窗口内给出点名该表的拒绝），并发布到 8 个读路径执行器（`data_scan` / `scan` / `index_scan` / `index_scan_all` / `create_index` / `delete` / `update` / `upsert`）。用户实际所见（`SqlError::message` 全文）：`Execution error: table 't' is being modified by an in-flight ALTER TABLE DROP COLUMN; read refused` ⇒ **显式拒绝**取代了静默错读。常态化用例已落地：`read_during_narrowing_in_flight_is_refused`、`read_after_narrowing_completes_returns_narrowed_rows`、`read_of_other_table_during_narrowing_is_unaffected`。豁免：改写自身的读（`table_manager.rs` 的改写读旧行、用旧 schema 解码，本就正确）。
  - **变体 (b)「窗口内崩溃后重开并查询」——仍开放**。在途状态是**纯内存**（不进 WAL / 目录，磁盘格式不变）⇒ 崩溃即消失。重开后目录仍广告 3 列而数据面已是 2 宽的已提交行 ⇒ 读仍错位（实测读数见「缺陷描述」段）。同族形态：该点之后**语句被放弃**（`pending` 析构 ⇒ 守卫清除 ⇒ 拒绝消失）同样不被拒绝 ⇒ 与崩溃变体同族（目录 3 列 + 2 宽已提交行）。
  - **归属**：承载 change 的 Plan Review 独立确认上述两个探针结论成立，且可由代码独立推出（`apply_column_drop` 才把活布局切到 `new_meta` 并窄化目录）；并确认该形态**不是**该 change 引入的（修复前同样错读）。

## 处置

- **状态保持 `open`（2026-10-25 更新，不得关闭）**。理由：ISS25 的登记触发条件含**两个变体**——(a) 窗口内的**并发读**：**已由承载 change Iteration 003 闭合**（在途窄化状态对读路径可见 ⇒ 点名该表的明确拒绝，不再返回按位置错位值）；(b) 窗口内的**崩溃后重开并查询**：**仍未闭合**（在途状态纯内存，崩溃即消失；重开后目录仍广告旧列数而数据面已是两宽的已提交行 ⇒ 读仍错位）。⇒ 处置指针与承载落点如下。
- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在 **DDL 提交点 / 读侧** → 由 DDL 目录事务化主题的后续 change 承载（project-model M20 的承载落点规则）。2026-10-25 更新：变体 (a) 的读侧处置已落在 `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 003；变体 (b) 仍未承载。
- 闭合变体 (b) 需要的**范围外设计决定**（2026-10-25 更新，承载 change 的 Plan Review 明确列出两条互斥路径）：
  - **路径 ①：引入持久标记**——改磁盘格式 ⇒ **破坏 MS36 已验收的 G1 / G2 / G3 兼容面**（旧库与旧 dump 的支持范围按目录行布局代际划分，持久标记会改变代际判别）。
  - **路径 ②：移动提交点**——与 `ddl-catalog-transactional` R1「**被放弃的 DDL 不留目录面变更**」**冲突**（把目录窄化提前到提交点之前 ⇒ 放弃的 DDL 会留下目录面变更）。
  - ⇒ 两条路径都超出当前 change 的范围与既有已验收契约 ⇒ 属**新的设计决定**，须由用户 / Plan 裁定后才能承载。
- 建议修复方向（2026-10-25 更新）：原两条互补路径中，① **读侧列序校验**已由变体 (a) 的显式拒绝形态覆盖（进程内窗口）；② **把「提交完成 → 目录应用」的窗口收窄到读侧可判定的形态**在变体 (b) 上**仍未实现**——纯内存状态无法跨崩溃存活，需要持久标记或提交点层面的设计决定。
- 验证前提（部分完成）：承载 change 需要**新的故障注入夹具**（提交完成、目录应用前崩溃）才能把本条从读码结论升级为已复现 ⇒ **2026-10-25 已满足**：夹具复用 `.agents/runbooks/in-process-crash-simulation.md` 的进程内崩溃形态（丢弃语句 future + 丢弃库句柄**不 close** ⇒ 脏页不落盘，重开走 WAL 恢复），实测读数已记录于「事件记录」段。
- 2026-10-26 关闭。原因：变体 (b)（窗口内崩溃后重开仍按位置错读）已由归档变更 Iteration 006 任务 T6 处置。窄化意图随事务写入 WAL，崩溃重开后由恢复发布已提交的窄化。该 Cycle 的 Plan Review 为 `accepted`（2026-10-26）。上文 2026-10-25「不得关闭」保留为当时记录。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/006-ddl-commit-point-unified-model/000-initial.md` 的 Plan Review 为 `accepted`（T6）。
- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md` —— Act Response 的 Remaining Issues 候选 D（读码结论与「未实跑验证」的明确声明）、Plan Review 的 F6（独立读码核对、design D5 就地更正、镜像方向对照与 Non-goal 裁定）。
- 登记时（2026-10-24）独立复核的代码位点：`src/storage/page_format/tuple.rs:189-222`（`deserialize_tuple_width`）、`src/storage/data/table_manager.rs:284-299`（重开库活布局 = 目录列行按宣告列数截断）
- 相关产物：`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/design.md`（D5「崩溃窗口（明确记录）」段的就地更正）
- 2026-10-25 更新时的证据（承载 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 003）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/003-rewrite-window-read-consistency/000-initial.md` —— Act Response 的「『显式拒绝』的用户实际所见」段（`Execution error: table 't' is being modified by an in-flight ALTER TABLE DROP COLUMN; read refused`）与「崩溃-重开变体」段（`PROBE_REOPEN_COLUMNS` / `PROBE_REOPEN_ROWS` / `PROBE_ABANDON_ROWS` 三个一次性探针的实际读数）；Plan Review 的 finding 7（提交点窗口的持久化不一致——独立确认两个探针结论成立且可由代码独立推出；该形态不是本 change 引入的）与「Follow-up Decision」的 ISS25 处置段（**不得关闭，状态保持 `open`**；登记时的现场措辞须更正为「可达性已实跑坐实；并发读变体已修复并有常态化用例；崩溃-重开变体仍开放」）。
- 2026-10-25 更新时核对的行为规格：`openspec/specs/ddl-rewrite-consistency/spec.md` —— Requirement「改写在途期间读路径不静默错读」及四个 Scenario（在途窄化的表被读时显式拒绝 / 改写完成后读路径恢复正常 / 未改写的表读路径不受影响 / 窄行与宽行的解码语义不变）。该域的 R3 全部限定在**进程内的在途窗口**，**没有任何一条声称崩溃 / 重开后的行为已修复** ⇒ 规格陈述与已实现行为一致。
