# Iteration 005 / Cycle 000: DML 墓碑的放弃窗口收敛（ISS24 的同形面）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T14, T15
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 既有域 `mvcc-tombstone-visibility`（MTV）的「墓碑自描述且不影响前驱版本」「未提交与已回滚墓碑不抑制、评估回溯前驱」「回滚后墓碑行的索引条目还原（PK 与唯一）」三条 requirement（本 Cycle 只在 **DML 路径**上闭合它们，**不改判据本身**）；Iteration 001 已验收的 DDL 改写路径成果（墓碑写序「先写不带链指针 → 登记 → 补链指针 → 再清理索引条目」+ 四处抑制集合构造点的 aborted 活性规则）；MS34 已验收的库属隐式事务回收语义（`src/database.rs:600` 的回收路径、`src/transaction/manager.rs:322` 的中止清理）；放弃夹具的既有形态（`tests/ddl_rewrite_consistency_test.rs:251` 与 `:638`）；本 change 的其余各域（IPO / DRC / RPBR / RDP）在本 Cycle 不动。
- Excluded scope: DDL 改写路径（Iteration 001 已闭合）；`VersionHeader::mark_aborted` 的链指针语义；既有 MTV 判据（`DataScanExecutor::superseder_suppresses`）与四处抑制集合构造点；UPDATE 路径的版本写序（**非墓碑**版本不抑制，见「Current-State Evidence」）；崩溃后未提交墓碑的可见性（已登记为 Issue 候选，由 Recorder 承载）；既有域 MTV 记录的「未提交期间 PK 点查不可达」边界；本 change 的 Iteration 006 / 007 与收尾全量（T21）。

**Objective**

被放弃（语句 future 被丢弃 / 超时）的 `DELETE` 在被回收后**不让被删行不可见**：该行在数据面**仍可见**（可见 id 集合完整、行数不减）、**点查仍命中**。DML 墓碑的写序与 DDL 改写路径**同形**——先写不带链指针 → 登记版本 → 补链指针 → 再清理索引条目——使「写出版本槽」与「登记版本」之间的放弃窗口内的墓碑**不压制任何前驱版本**。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `0a08b14`（ahead of origin 9）；MS34 + issue-fix + MS35 + MS36 + 本 change 的 Iteration 000–004 改动**全部未提交**；本 Cycle 在该现场上继续，不回滚、不清理、不提交既有改动。
  - 前序 Iteration 的验证结论（沿用，覆盖范围未变化）：`cargo test --lib` 420/0、`cargo build --tests` 0 警告（来源：Iteration 004 的 Act Response，2026-10-25；该结论覆盖当前 `src/` 与 `tests/` 的内容）。全量回归按 change 预算归最后一个 Iteration（T21），本 Cycle 不跑全量。
  - Iteration 000 / 001 / 002 / 003 / 004 的 Review Result 均为 **`accepted`** ⇒ 本 Cycle 不依赖任何未完成的 Iteration。
  - 缺陷的登记现场（Iteration 001 的 Plan Review 2026-10-25 独立复现）：真实文件库 + 真实 `Database::open` + 真实 `DELETE FROM t WHERE id = 1`（N = 30）+ 锚点式放弃 + 下一条写语句触发回收 ⇒ 可见 id 集合 **[2..30]**（`id = 1` 缺失）⇒ **行静默消失**（静默丢行级）。
  - 已知限制（不在本 Cycle）：崩溃后未提交墓碑的可见性（Issue 候选）；ISS25（读路径在途窄化）保持 `open`。
- Current-State Evidence（Plan 已独立读码确认的行位；Act 不需回读即可执行）:
  - **DML 删除路径的写序**（`src/executor/delete.rs::DeleteExecutor::delete_row`，`:139`）：`index_manager.search`（`:141`）→ `unique_keys_of_row`（`:154`）→ **墓碑写（带链指针）**：`VersionHeader::new(self.tx_id, None).with_next_version(*rid).mark_deleted()`（`:171-173`）经 `write_tuple_to_data_page` 写入（`:174-180`）→ `clear_all_visible` ×2（`:181-184`）→ **PK 条目删除**（`:196`）→ 唯一条目删除（`:200-202`）→ 二级条目删除（`:206-209`）→ **版本登记** `tx_manager.record_version`（`:217-221`）→ WAL `Delete`（`:228-234`）。⇒ **登记在最后**。
  - **REPLACE 的删除段同形**（`src/executor/upsert.rs::delete_conflict_row`，`:614`）：墓碑带链指针（`:651-655`）→ `clear_all_visible` → PK 条目删除（`:669-684`，含「删除前按在位守卫确认该键确实指向本冲突行」臂）→ 唯一条目删除（`:686-688`）→ 二级条目删除（`:690-694`）→ **登记**（`:696-701`）→ WAL（`:704-710`）。
  - **DDL 改写路径的参照形态（Iteration 001 已验收）**（`src/storage/data/table_manager.rs` 的 `alter_drop_column` DELETE 腿，`:1089-1125`）：墓碑**不带链指针**写（`:1089`：`VersionHeader::new(tx_id, None).mark_deleted()`）→ `clear_all_visible` → **登记** `tx_manager.record_version`（`:1097-1100`）→ **补链指针** `update_version_header_in_data_page`（`:1103-1111`）→ **再清理索引条目**（`:1114-1122`：PK / 唯一 / 二级）→ WAL（`:1124-1131`）。⇒ 本 Cycle 的目标形态**逐项对齐**它。
  - **为什么「未登记的墓碑」会压制其前驱**：`src/executor/data_scan.rs::build_superseded_map`（`:238`）按每个槽的 `next_version` 建立「被指向版本 →（指向者槽, 创建事务）」映射；`slot_is_superseded`（`:298`）沿链上溯；`superseder_suppresses`（`:344`）对墓碑槽（`vh.is_deleted()`，`:352`）在无快照路径下取 `!active_tx_ids.contains(&vh.create_tx_id())` ⇒ **删除者不在活跃集合即视为已提交删除** ⇒ 压制整条链。
  - **为什么回收看不到未登记的墓碑**：`src/database.rs::reclaim_abandoned_implicit_transactions`（`:600`）→ `reclaim_implicit_tx`（`:627`）先取 `tx_manager.tx_version_tables(tx_id)`（**只含已登记版本的表**）⇒ 放弃落在「写出版本槽」与「登记版本」之间时，该表**根本不在回收覆盖面内** ⇒ `abort_by_id` → `abort_cleanup_versions`（`src/transaction/manager.rs:322`）不会对它执行 ⇒ 墓碑的 `create_tx_id` 永不被 `mark_aborted` 中和。
  - **登记在先为何就能闭合**：登记 ⇒ 该表进入 `tx_version_tables` ⇒ `abort_cleanup_versions` 覆盖该表 ⇒ 其 `versions_created_by`（`manager.rs:547`，按创建事务 id 沿数据链找回**未登记**的版本）找回墓碑槽 ⇒ 中止清理把它 `mark_aborted`（`create_tx_id = 0` + 保留链指针）⇒ `superseder_suppresses` 的 `create_tx_id == 0` 分支不压制 ⇒ 回收后该行仍可见（与既有域 MTV 的「已回滚墓碑不抑制其前驱版本」一致）。
  - **不带链指针的墓碑是无害的**：扫描路径对自身版本为墓碑的槽直接跳过（`data_scan.rs:489` 的 `if vh.is_deleted() { return Ok(PageAction::SkipSlot); }`）；没有链指针 ⇒ 它不进入抑制集合 ⇒ 既不压制前驱、也不产出行。
  - **UPDATE 路径不受影响**：`src/executor/upsert.rs:480` 的新版本**带链指针**但**不是墓碑** ⇒ `superseder_suppresses` 的非墓碑分支要求 `commit_tx_id` 存在，未提交版本不抑制 ⇒ 放弃窗口内不压制 ⇒ **不需要改** UPDATE 路径（本 Cycle 的修复面仅限墓碑路径）。
  - **回收后的索引条目面**：`abort_cleanup_versions` 按「当前是否持有索引条目」分 A 趟 / B 趟；墓碑槽自身**没有**指向它的索引条目（DELETE 删的是指向**前驱**的条目）⇒ 落 B 趟 ⇒ `restore_tombstone_index_entries`（`manager.rs:607`）沿墓碑链回溯到第一个非本事务的存活版本并**重新插入**其 PK / 唯一 / 二级条目 ⇒ 既有域 MTV 的「回滚后墓碑行的索引条目还原」由此承载。
  - **放弃夹具的既有形态**（先例 `tests/ddl_rewrite_consistency_test.rs`）：`abandon_statement`（`:251`）——语句 future 以 `Box::pin(db.execute_sql(sql))` 起跑，锚点 future 以 `tokio::select! { biased; … }` 竞争，锚点命中即 `drop(statement)`（丢弃语句 future；执行器停在执行期，隐式事务留在活跃集里）；锚点 `anchor_slot_written`（`:638`）= 「链上已出现由活跃事务创建的版本槽（该槽尚未登记）」；`ANCHOR_TIMEOUT` 是**夹具失控兜底**，不是行为判据。
  - **触发回收的入口**：下一条写语句 → `reclaim_abandoned_implicit_transactions`（`src/executor/alter_table.rs:87`、`src/pipeline.rs:560` 与 `:884`、`src/database.rs:684` 与 `:882`）。
  - **既有域 MTV 的相关条款**（`openspec/specs/mvcc-tombstone-visibility/spec.md`）：「未提交与已回滚墓碑不抑制、评估回溯前驱」「墓碑自描述且不影响前驱版本」「回滚后墓碑行的索引条目还原（PK 与唯一）」；其**已知边界**记有「未提交删除期间的 PK 点查不可达为预存索引时序边界（索引条目删除即时、未提交期间不还原）」⇒ 本 Cycle **不**处理「未提交期间」的索引可见性，只保证**回收之后**的条目面。
- Code and Critical Path:
  - 入口：`src/executor/delete.rs`（`DeleteExecutor::next` → `delete_row`）与 `src/executor/upsert.rs`（REPLACE 的冲突行删除段）。
  - 数据流：行定位（索引 `search`）→ 墓碑版本槽写入（数据页追加槽）→ 抑制集合的构建依据（槽的 `next_version`）→ 索引条目（PK / 唯一 / 二级）→ 版本登记（事务管理器的 `tx_versions` / `tx_version_tables`）→ WAL `Delete`。
  - 状态变化：数据页（新增墓碑槽 + 其 `next_version`）、索引树（条目删除）、事务管理器（`tx_versions` 登记）、WAL。
  - 读面影响：`DataScanExecutor` 的抑制集合与墓碑求值（`data_scan.rs:238` / `:298` / `:344`）、`TableManager::table_has_live_rows`（`table_manager.rs` 的活行判定，`next_version` 构造点之一）。

**Implementation Guidance**

- **顺序即修复面**：只调整**写入的先后**，不改变终态内容——墓碑槽的终态字节（`create_tx` = 删除者、`commit` = 删除哨兵、`next_version` → 被删行）与现状逐字节相同；索引条目集合与 WAL 记录也不变。
- **参照形态**：直接对齐 `src/storage/data/table_manager.rs:1089-1125` 的 DDL 侧实现——「不带链指针写 → `clear_all_visible` → `record_version` → `update_version_header_in_data_page` 补链指针 → 清理索引条目 → WAL」。补链指针用 `crate::storage::update_version_header_in_data_page`（定义在 `src/storage/data_page.rs:182`，已由 `src/storage/mod.rs:31-34` 公开重导出；`delete.rs` / `upsert.rs` 已从 `crate::storage` 导入同族函数）。
- **终态不变性检查**：改序后必须**逐字节**等价于现状的墓碑槽终态（否则恢复期的 WAL 重放与索引重建会读到不同的链形态）。
- **索引条目面的次序**：条目清理必须**在**链指针补写**之后**（DDL 侧同此）——放弃落在链指针之前时条目仍在位；落在链指针之后时，回收的中止清理负责还原条目。
- **不要引入新的抽象或公共辅助层**：修复面限于两个执行器文件的**语句次序**。

**Task Definitions**

### T14: 被放弃的 `DELETE` 不让被删行不可见——先行 RED 用例

- Requirement/Scenario: 既有域 MTV「未提交与已回滚墓碑不抑制、评估回溯前驱」·场景「墓碑事务回滚后扫描恢复」（DML 路径的同形面）；既有域 MTV「回滚后墓碑行的索引条目还原（PK 与唯一）」·场景「DELETE 回滚后 PK 等值点查可达」；既有域 MTV「墓碑自描述且不影响前驱版本」。
- Depends on: None
- Targets: `tests/dml_tombstone_abandon_window_test.rs`（新增用例）；作为被测对象：`src/executor/delete.rs::DeleteExecutor::delete_row`、`src/executor/upsert.rs::delete_conflict_row`、`src/transaction/manager.rs::TransactionManager::abort_cleanup_versions`、`src/executor/data_scan.rs` 的抑制集合与墓碑求值面、`src/database.rs::Database::reclaim_abandoned_implicit_transactions`（本任务**不**修改它们）。
- Current behavior: 被放弃（语句 future 被丢弃 / 超时）的 `DELETE` 在被回收后，让它**已删除的那一行**从扫描结果中消失（登记实测 30 → 29，缺失 `id = 1`），且该行的主键点查不再命中。根因（读码坐实）：墓碑**带链指针**写入且**版本登记在最后** ⇒ 放弃落在「写出版本槽」与「登记版本」之间时，该墓碑未被登记 ⇒ 该表不在回收覆盖面内（`tx_version_tables` 只含已登记版本的表）⇒ 墓碑永不被 `mark_aborted` 中和 ⇒ 回收后删除者已不在活跃集合 ⇒ `superseder_suppresses` 的墓碑分支判定为已提交删除 ⇒ 压制其前驱版本 ⇒ 该行不可见。
- Required behavior: 被放弃的 `DELETE` 在被回收后**不隐藏**它已删除的那一行——该行在数据面**仍可见**（可见 id 集合完整、行数不减）、**点查仍命中**（每个 id 恰好命中一行）；**REPLACE 的删除段同形**（同文件的对照用例覆盖）。
- Preserve: 既有域 MTV 的全部既有语义（已提交墓碑抑制整条链、未提交 / 已回滚墓碑不抑制、墓碑自描述、删除者回滚后条目还原、自身事务内删除对自己可见为已删）；既有域 MTV 记录的已知边界（「未提交删除期间的 PK 点查不可达」）**不因本任务改变**；Iteration 001 已验收的 DDL 改写路径行为；MS34 已验收的库属隐式事务回收语义；放弃夹具的既有形态。
- Forbidden: 不得只改断言不放宽——「行仍可见 / 行数不减 / 点查命中」的断言强度不得低于登记现场（30 → 29 的规模下逐行可数）；不得用 ≥ 300 行的表规模构造用例（ISS23 约束）；不得引入身份证明机制、判定层或 timer 阈值行为判据（`ANCHOR_TIMEOUT` 只作夹具失控兜底）；本任务**不改产品代码**；不得改动 `superseder_suppresses` 或抑制集合构造点（那是判据，不是本任务的修复面）。
- First test: `tests/dml_tombstone_abandon_window_test.rs`（新增，RED）——用例名 `abandoned_delete_keeps_the_deleted_row_visible`（计划名；Act 可改名并记录实际名）。位置：真实文件库 + 真实 `Database::open`；动作：建表 `t(id INT PRIMARY KEY, n INT)` → 写入 **N = 30** 行（`id = 1..30`）→ 以**锚点式放弃**丢弃 `DELETE FROM t WHERE id = 1` 的语句 future（锚点 = 数据链上已出现由该活跃事务创建的版本槽，与 `tests/ddl_rewrite_consistency_test.rs:638` 的 `anchor_slot_written` 同形）→ 触发回收（下一条写语句）→ 观察：可见 id 集合 = `1..=30`（完整）、行数 = 30、每个 id 的点查恰好命中 1 行。命令：`cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1`。预期（修复前）：「行仍可见 / 点查命中」断言**失败**（登记形态：可见 id = `[2..30]`，`id = 1` 缺失）。
- GREEN condition: 「可见 id 集合完整 / 行数不减 / 每个 id 点查恰好命中一行」三条断言在修复后通过；Act Response 写明放弃实际落在窗口的哪一段（登记前 / 登记后）与读数。
- Verification: `cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` 退出码 0、failed 计数 0。失败含义：放弃语义或墓碑可见性语义被改变。
- Stop when: 放弃夹具在 N = 30 的规模下**无法稳定**落进「写出版本槽」与「登记版本」之间的窗口（锚点不可达 / 夹具竞态）⇒ 停止并退回 Plan（`replan-required`），在 Act Response 写明 Blocker Handoff 与已排除的可能（夹具规模、锚点判据、回收触发点）；或缺失行的消失层被证明**不在**「墓碑压制前驱」这一层 ⇒ 写明 Blocker Handoff 返回 Plan。

### T15: DML 墓碑写序收敛（不带链指针写 → 登记 → 补链指针 → 再清理索引条目）

- Requirement/Scenario: 既有域 MTV「墓碑自描述且不影响前驱版本」「未提交与已回滚墓碑不抑制、评估回溯前驱」「回滚后墓碑行的索引条目还原（PK 与唯一）」；本 change 的 RTM 行「MTV『未提交与已回滚墓碑不抑制、评估回溯前驱』（既有域，ISS24 的 **DML 同形面**）」。
- Depends on: T14
- Targets: `src/executor/delete.rs::DeleteExecutor::delete_row`（`:139`；墓碑写 `:167-186`、条目清理 `:196-210`、登记 `:217-221`）；`src/executor/upsert.rs::delete_conflict_row`（`:614`；墓碑写 `:649-666`、条目清理 `:669-694`、登记 `:696-702`）。
- Current behavior: 两条路径都是「**带链指针**写墓碑 → 清索引条目 → **最后**登记版本」⇒ 放弃落在「写出版本槽」与「登记版本」之间时，未登记的墓碑压制其前驱版本（见 T14 的 Current behavior）。
- Required behavior: 两条路径的写序与 DDL 改写路径**同形**：墓碑**不带链指针**写入 → `clear_all_visible` → **登记版本** → 用 `crate::storage::update_version_header_in_data_page` **补写链指针**（`next_version` → 被删行）→ **再**清理 PK / 唯一 / 二级索引条目 → WAL。⇒ 放弃落在窗口内时：窗口前段（登记前）的墓碑**不带链指针** ⇒ 不压制任何前驱版本、且索引条目仍在位；窗口后段（登记后）的墓碑被回收的中止清理中和（`mark_aborted`）⇒ 不压制、条目被还原。**终态不变**：墓碑槽的终态字节（`create_tx` = 删除者、删除哨兵、`next_version` → 被删行）、索引条目集合、WAL `Delete` 记录与修复前逐字节等价。
- Preserve: 既有域 MTV 的全部判据（`superseder_suppresses` 不动；四处抑制集合构造点不动）；`VersionHeader::mark_aborted` 的「保留链指针」语义；`abort_cleanup_versions` 的 A 趟 / B 趟分桶与 `restore_tombstone_index_entries` 的还原语义（**不改**）；`delete.rs` / `upsert.rs` 的「未命中零副作用早退」与「`SlotNotFound` 容忍臂」（不写墓碑、不登记悬空 rid）；WAL `Delete` 记录的字段语义（`row_id` 仍是**被删行**的 rid）；磁盘格式不变（保 MS36 的旧格式兼容面）；`free_page` 集合语义不变。
- Forbidden: 不得改 `VersionHeader` 的磁盘格式或 `mark_aborted` 的链指针语义；不得改 `DataScanExecutor::superseder_suppresses` 与抑制集合构造点；不得改 `abort_cleanup_versions` / `restore_tombstone_index_entries` 的语义（中止清理的覆盖面与条目还原**由登记的时点**保证，不由改清理逻辑保证）；不得引入新的公共抽象或跨模块重构；不得引入身份证明机制、判定层或 timer 阈值行为判据。
- First test: 复用 T14 的用例（变更前已 RED）；命令：`cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1`。预期（修复前）：「行仍可见 / 点查命中」断言失败。
- GREEN condition: T14 用例通过；墓碑槽终态与修复前逐字节等价（以同一条用例内对槽头的读数断言或等价观察锁定）；既有 `mvcc_tombstone_visibility_test` / `mvcc_abort_test` / `ddl_rewrite_consistency_test` / `index_manager_test` / `secondary_index_test` / `recovery_test` / `recovery_e2e_test` 通过。
- Verification: `cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` + `cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1` + `cargo test --test mvcc_abort_test -- --test-threads=1` + `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` + `cargo test --test index_manager_test -- --test-threads=1` + `cargo test --test secondary_index_test -- --test-threads=1` + `cargo test --test recovery_test -- --test-threads=1` + `cargo test --test recovery_e2e_test -- --test-threads=1` + `cargo test --lib -- --test-threads=1`，各退出码 0、failed 计数 0。失败含义：墓碑可见性语义、索引条目还原语义、DDL 侧既有成果或恢复重建语义被改变。
- Stop when: 修复需要改动 `VersionHeader` 的磁盘格式、`mark_aborted` 的链指针语义、`superseder_suppresses` 或中止清理的覆盖面 / 还原语义 ⇒ 停止并退回 Plan（`replan-required`）；或改序后**墓碑槽终态**无法与修复前逐字节等价 ⇒ 停止并退回 Plan（终态等价是恢复期 WAL 重放与索引重建的前提）；或新序导致**重复索引条目**（点查命中数 > 1，或回收后同键插入被假阳性 `DuplicateKey` 拒绝）⇒ 停止并退回 Plan（见「Risks and Notes」的条目面风险）。

**Invariants**

- 数据页：墓碑是**独立版本槽**（追加槽），被删版本的 header 不被就地改写（既有域 MTV 的「墓碑自描述且不影响前驱版本」）。
- 抑制集合：墓碑**只**通过 `next_version` 链接参与抑制；`create_tx_id == 0`（aborted 标记）的指向者不被承认（Iteration 001 已闭合的活性规则）。
- 事务记账：只有**登记**过的版本进入 `tx_versions` / `tx_version_tables`；中止清理的覆盖面由登记的时点决定。
- 索引条目：PK / 唯一 / 二级条目在**墓碑带链指针之后**清理；回收之后由中止清理还原。
- 磁盘格式与 WAL 记录格式不变（保 MS36 的旧格式兼容面）。

**Non-goals**

- DDL 改写路径（Iteration 001 已闭合）；`ADD COLUMN` 的宽度感知解码语义；既有 MTV 判据与四处抑制集合构造点；`VersionHeader::mark_aborted` 的链指针语义；UPDATE 路径的版本写序（非墓碑版本不抑制，见「Current-State Evidence」）；崩溃后未提交墓碑的可见性（Issue 候选，由 Recorder 承载）；既有域 MTV 记录的「未提交期间 PK 点查不可达」边界；本 change 的其它 Iteration 与收尾全量。

**Acceptance**

1. **被放弃的 `DELETE` 不隐藏被删行**：N = 30 的真实库上，被放弃的 `DELETE FROM t WHERE id = 1` 在回收后，可见 id 集合 = `1..=30`（登记现场为 `[2..30]`），行数 = 30（登记现场为 29）。〔MTV「未提交与已回滚墓碑不抑制、评估回溯前驱」；T14〕
2. **点查面未被破坏**：回收后每个 `id` 的等值点查**恰好命中一行**（含被删的那个 id）；回收后同键插入不被假阳性 `DuplicateKey` 拒绝。〔MTV「回滚后墓碑行的索引条目还原（PK 与唯一）」；T14 / T15〕
3. **墓碑终态不变**：修复后墓碑槽的终态字节与修复前逐字节等价（`create_tx_id` = 删除者、删除哨兵、`next_version` → 被删行）；索引条目集合与 WAL `Delete` 记录的字段语义不变。〔MTV「墓碑自描述且不影响前驱版本」；T15〕
4. **REPLACE 同形闭合**：REPLACE 的冲突行删除段与 `DELETE` 同形收敛（同形放弃窗口内不隐藏冲突行）。〔T15〕
5. **既有语义零回归**：既有 `mvcc_tombstone_visibility_test` / `mvcc_abort_test` / `ddl_rewrite_consistency_test` / 索引 / 恢复相关测试与 `cargo test --lib` 全绿；DDL 侧既有成果（Iteration 001）不受影响；磁盘格式不变。〔T15〕
6. **无身份证明机制 / 无框架外判定层**：判定全部落在 `cargo test` 内；夹具的 timer 只作失控兜底。〔T14 / T15〕

**Verification**

- Acceptance 1 / 2 / 4：`cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` 退出码 0、failed 计数 0（真实入口：真实文件库 + 真实 `Database::open` + 真实 `DELETE` 语句 + 锚点式放弃 + 下一条写语句触发回收；断言锚定可见 id 集合 / 行数 / 点查命中数）。
- Acceptance 3：同一次运行内对墓碑槽头的读数断言（`create_tx_id` = 删除者、`is_deleted()`、`next_version` → 被删行）。
- Acceptance 5：`cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1` + `cargo test --test mvcc_abort_test -- --test-threads=1` + `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` + `cargo test --test index_manager_test -- --test-threads=1` + `cargo test --test secondary_index_test -- --test-threads=1` + `cargo test --test recovery_test -- --test-threads=1` + `cargo test --test recovery_e2e_test -- --test-threads=1` + `cargo test --lib -- --test-threads=1`，各退出码 0、failed 计数 0。
- Acceptance 6：代码走查（无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明；无框架外脚本；无 timer 阈值行为判据）。
- **本 Iteration 不跑全量回归**（收尾全量归 Iteration 007 / T21，M19 分批串行纪律）。

**Gate 2 Readiness**

- PASS——需求与范围：既有域 MTV 的三条 requirement 映射到 T14 / T15；修复面限于两个执行器文件的写序；既有域文本**不需要**修改 ⇒ 本 Cycle **不产生规格增量**（既有域不在 `openspec/specs/` 的 MODIFY 面上，且本 change 只新增域）。
- PASS——调查：入口、目标符号、调用者 / 被调用者、数据流、状态变化、错误与并发边界、既有测试与验证命令均已写入「Investigation Facts」（行位可核，Act 不需回读）。
- PASS——BDD 场景覆盖：Happy（正常 `DELETE` 提交后行不可见，既有语义不变）、Sad（放弃后行仍可见）、Edge（放弃落在窗口的不同段；`SlotNotFound` 容忍臂；未命中零副作用臂）、组合（`DELETE` + REPLACE 同形；条目面与数据面的组合；既有域 MTV 的「回滚后条目还原」）均已映射到 T14 / T15 的 Required behavior 与 Forbidden。
- PASS——任务分轮：T14（RED）→ T15（写序收敛），单一 Cycle 内可独立验证与诊断。
- PASS——追踪：Cycle RTM（见下）与 change `tasks.md` 的 RTM 行一致（T14 / T15 ↔ MTV 的 DML 同形面）。
- PASS——当前轮验证：T14 的 RED 命令与其预期失败形态已写明；T15 的 GREEN 判据与回归清单已写明；Persisted Evidence = `none`（全部结论可低成本重跑）。
- 无 BLOCKED / SKIPPED 项。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| MTV「未提交与已回滚墓碑不抑制、评估回溯前驱」（既有域，ISS24 的 DML 同形面） | T14, T15 | `tests/dml_tombstone_abandon_window_test.rs::abandoned_delete_keeps_the_deleted_row_visible` | Covered |
| MTV「回滚后墓碑行的索引条目还原（PK 与唯一）」（既有域） | T14, T15 | 同上（点查命中数与同键插入断言） | Covered |
| MTV「墓碑自描述且不影响前驱版本」（既有域） | T15 | 同上（墓碑槽终态读数断言） | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（一个新增测试文件 + 8 个既有测试文件 + `cargo test --lib`），命令与结果写入 Act Response 即可。

**Risks and Notes**

1. **索引条目面的重复风险（非阻塞，须在 GREEN 中观察）**：新序把「条目清理」放到链指针补写**之后** ⇒ 放弃落在「登记之后、条目清理之前」时，回收的中止清理会经 `restore_tombstone_index_entries`（`manager.rs:607`）**重新插入**一条**仍然在位**的条目 ⇒ 叶节点允许重复键（`src/storage/btree/node.rs:122` 的既有语义「允许重复 key」）⇒ 可能出现同键的两条条目（都指向同一存活版本 ⇒ 点查命中数仍为 1）。⇒ T15 的 GREEN 须额外观察：点查命中数恒为 1、回收后同键插入不被假阳性 `DuplicateKey` 拒绝；若出现重复或假阳性 ⇒ 按 T15 的 Stop-when 退回 Plan（**不得**就地改 `restore_tombstone_index_entries` 的语义）。
2. **窗口落点的抖动**：锚点是「链上已出现本事务创建的版本槽」，它命中的那一刻可能已在登记之后 ⇒ 修复前的 RED 是否稳定命中「登记之前」取决于夹具与执行的时序。⇒ T14 的 RED 必须**实测**该形态（登记现场已是 30 → 29）；若 RED 稳定不出来，按 T14 的 Stop-when 退回 Plan（**不得**用放宽断言或改判据的方式凑出 RED）。
3. **REPLACE 的同形面**：REPLACE 的删除段与 `DELETE` 同形，但它的仲裁与「删除前按在位守卫确认该键指向本冲突行」臂（`upsert.rs:669-684`）必须**逐字节保留** ⇒ T15 改序时不得改动该臂的条件与次序。
4. **既有域 MTV 的已知边界不变**：本 Cycle 只保证**回收之后**的行可见与条目面；「未提交期间 PK 点查不可达」是既有边界，不在本 Cycle 的覆盖面。
5. **规模口径**：N = 30（< 300，ISS23 约束）；不得放宽到更大规模来规避夹具问题。
6. **不改磁盘格式**（保 MS36 的 G1/G2/G3 兼容面）；本 Cycle 不改 `free_page` 集合语义。

## Act Response

- Status: reported

### 任务状态

| Task | 状态 | 涉及文件与符号 |
|---|---|---|
| T14 | done | `tests/dml_tombstone_abandon_window_test.rs`（新建）：`abandoned_delete_keeps_the_deleted_row_visible`（`:257`）、`abandoned_replace_keeps_the_conflict_row_visible`（`:336`）、`committed_delete_keeps_the_canonical_tombstone_slot`（`:388`）；辅助 `setup`（`:45`）、`scan_id`（`:61`）、`scan_by_pk`（`:75`）、`pk_entry_count`（`:87`）、`chain_pages`（`:100`）、`dump_chain`（`:141`）、`anchor_slot_written`（`:195`）、`abandon_statement`（`:212`）、`trigger_reclaim`（`:248`） |
| T15 | done | `src/executor/delete.rs::DeleteExecutor::delete_row`（`:139`；墓碑写 `:177-202`、登记 `:204-212`、链指针补写 `:214-225`、条目清理 `:227-242`、WAL `:255-268`）；`src/executor/upsert.rs::delete_conflict_row`（`:614`；墓碑写 `:649-670`、登记 `:672-684`、链指针补写 `:686-697`、条目清理 `:699-723`、WAL `:725-732`） |

### T14 的先行测试 RED 证据（实际运行，修复前的产品代码）

先行顺序：先写 `tests/dml_tombstone_abandon_window_test.rs` 并运行观察到 RED，**再**改产品代码。命令与实际输出：

```
$ cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1
running 3 tests
test abandoned_delete_keeps_the_deleted_row_visible ... FAILED
test abandoned_replace_keeps_the_conflict_row_visible ... FAILED
test committed_delete_keeps_the_canonical_tombstone_slot ... ok

---- abandoned_delete_keeps_the_deleted_row_visible stdout ----
abandon-window reading: tx 31; version registered at the abandon point = false (false = 放弃落在「写出版本槽」与「登记版本」之间)

thread 'abandoned_delete_keeps_the_deleted_row_visible' panicked at tests/dml_tombstone_abandon_window_test.rs:272:5:
an abandoned DELETE must not hide the row it deleted; missing ids: [1] (visible: [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30])

---- abandoned_replace_keeps_the_conflict_row_visible stdout ----
abandon-window reading (REPLACE): tx 31; version registered at the abandon point = false (false = 放弃落在「写出版本槽」与「登记版本」之间)

thread 'abandoned_replace_keeps_the_conflict_row_visible' panicked at tests/dml_tombstone_abandon_window_test.rs:349:5:
an abandoned REPLACE must not hide the conflict row it deleted; missing ids: [1] (visible: [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30])

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
```

- **失败原因落在预期的断言上**：第一条断言（`missing ids: [1]`）失败——可见 id 集合为 `[2..30]`、29 行，`id = 1` 缺失，**与登记现场（30 → 29、缺 `id = 1`）逐项一致**；断言强度未被放宽（逐行可数、规模 N = 30 < 300）。
- **RED 的形状正确**：失败不是「行数不变但内容错」「点查命中 2 行」或其它形态，而是**被放弃的 `DELETE` 使其已删除的那一行从数据面消失**。
- **放弃落点**：修复前读数 `version registered at the abandon point = false` ⇒ 放弃落在「写出版本槽」与「登记版本」**之间**（Plan Context 登记的缺陷窗口），命中了 Plan Context「Risks and Notes 2」要求实测的形态。
- 第三条用例（`committed_delete_keeps_the_canonical_tombstone_slot`）在修复前**即为 GREEN**（`1 passed`）——它锁定的是既有终态（墓碑槽终态字节 + WAL `Delete` 记录字段 + 索引条目集合），修复后仍须为 GREEN ⇒ 用作终态等价的守门用例。
- **行号说明**：上面引用的 `:272:5` / `:349:5` 是**该次 RED 运行当时**的行号（逐字引用实际输出）；测试文件在 GREEN 之前做过一次 `rustfmt` 排版（见「自检发现」第 2 条），最终文件里对应的断言消息在 `:285` 与 `:364`。排版不改断言语义；排版后的最终工作区已重跑该目标（`3 passed; 0 failed`）。

### GREEN 证据与验证（实际命令与结果，最终工作区）

| 命令 | 结果 |
|---|---|
| `cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` | `3 passed; 0 failed` |
| `cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1` | `11 passed; 0 failed` |
| `cargo test --test mvcc_abort_test -- --test-threads=1` | `3 passed; 0 failed` |
| `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` | `9 passed; 0 failed` |
| `cargo test --test index_manager_test -- --test-threads=1` | `3 passed; 0 failed` |
| `cargo test --test secondary_index_test -- --test-threads=1` | `27 passed; 0 failed` |
| `cargo test --test recovery_test -- --test-threads=1` | `3 passed; 0 failed` |
| `cargo test --test recovery_e2e_test -- --test-threads=1` | `6 passed; 0 failed` |
| `cargo test --lib -- --test-threads=1` | `420 passed; 0 failed` |
| `cargo build --tests` | 0 warning |

**因本次改动触及覆盖面而重跑的既有目标**（一次调用一批、批内 `--test-threads=1`；逐目标的计数按该次运行的 `Running tests/<file>` 行与随后的 `test result:` 行配对读出）：

- 批 A（8 目标，合计 **134 passed / 0 failed**）：`constraint_enforcement_test` 26/0、`dml_tx_id_test` 6/0、`executor_test` 39/0、`full_table_dml_test` 11/0、`mvcc_commit_test` 4/0、`update_index_maintenance_test` 11/0、`upsert_test` 32/0、`visibility_test` 5/0。
- 批 B（5 目标，合计 **31 passed / 0 failed**）：`checkpoint_test` 2/0、`drop_persistence_test` 11/0、`free_page_lifecycle_test` 12/0、`index_page_ownership_stack_overflow_test` 1/0、`index_page_ownership_test` 5/0。

- **`--lib` 420 基线**：本 Cycle 改到 `src/executor/delete.rs` / `src/executor/upsert.rs` ⇒ 该结论失效，**基线重跑**，420/0 **仍成立**。MS35 / MS36 的全量终态 2192/0/2 对当前工作区同样失效（按 change 预算归 Iteration 007 / T21）。
- **沿用与重跑的划分**：Iteration 000–004 的既有结果中，`index_page_ownership_test` / `index_page_ownership_stack_overflow_test` / `free_page_lifecycle_test` / `drop_persistence_test` / `checkpoint_test` 的覆盖**触及本次改动的 DML 删除路径** ⇒ 已重跑（结果同上）；`release_path_bounded_read_test` 7/0、`release_durability_precondition_test` 2/0、`ddl_catalog_transactional_test` 3/0、`ddl_crash_recovery_test` 6/0 的覆盖面未被本次改动触及 ⇒ **沿用**（来源：Iteration 002 / 004 的 Act Response）。
- **全量回归**：本 Cycle **不跑**（按 change 预算归最后一个 Iteration 007 / T21，M19 分批串行）。本 Cycle 只跑**定向**验证（Plan Context 的 Verification 段 9 条 + 2 个受影响批 13 个目标），全程**串行**执行、同一时刻只有一个 `cargo test` 在跑、批内 `--test-threads=1`、**无**一次无目标过滤的全量调用。

### 写入顺序改动与既有机制的复用

**改序（两条路径，`delete.rs` 与 `upsert.rs` 同形）**：

| 段 | 修复前 | 修复后（= DDL 改写路径同形） |
|---|---|---|
| 墓碑写 | `VersionHeader::new(tx, None).with_next_version(rid).mark_deleted()`（**带链指针**） | `VersionHeader::new(tx, None).mark_deleted()`（**不带链指针**） |
| 登记版本 | **最后**（条目清理之后） | 紧跟墓碑写之后（链指针补写之前） |
| 链指针 | 随墓碑一次写完 | `update_version_header_in_data_page` **补写**（`next_version` → 被删行） |
| 条目清理 | 登记之前 | 链指针补写**之后** |
| WAL `Delete` | 携带被删行的 rid | **不变**（携带被删行的 rid） |

- **复用 Iteration 001 的既有机制，未另造机制**：① 抑制集合的构造点与 `superseder_suppresses` **未动**；② 中止清理的覆盖面与条目还原仍由 `TransactionManager::abort_cleanup_versions`（`src/transaction/manager.rs`）的 **A/B 两趟分桶 + `restore_tombstone_index_entries`** 承担——本改动只是让墓碑**更早进入登记**，从而**更早进入 `tx_version_tables`**、**被回收看见**（Plan Context 的 Forbidden 明确要求「覆盖面由登记的时点保证，不由改清理逻辑保证」）；③ `VersionHeader::mark_aborted` 的「保留链指针」语义**未动**；④ `VersionHeader` 的磁盘格式**未动**（链指针字段本就在头内，只是写入时刻改变）；⑤ `free_page` 集合语义**未动**。
- **终态等价（Acceptance 3）**：`committed_delete_keeps_the_canonical_tombstone_slot` 在同一次运行内断言墓碑槽头的**原始 22 字节**等于 `VersionHeader::new(删除者 tx, None).with_next_version(被删行 rid).mark_deleted().to_bytes()`（`create_tx_id` = 删除者、删除哨兵、`next_version` → 被删行），并断言：提交后的删除已清理该键的索引条目（条目数 0）；WAL 中**恰好一条** `Delete` 记录，其 `table_name` = `t`、`row_id` = **被删行**的 rid（≠ 墓碑槽的 rid）⇒ WAL 记录字段语义与恢复期重建前提不变。

### 放弃实际落在窗口的哪一段（Plan Context GREEN condition 要求写明）

| 运行 | 读数（`version registered at the abandon point`） | 落段 | 结果 |
|---|---|---|---|
| 修复前（RED） | `false` | 「写出版本槽」与「登记版本」**之间**（缺陷窗口） | 可见 id = `[2..30]`、29 行、`id = 1` 缺失 ⇒ RED |
| 修复后（GREEN） | `true` | 「登记版本」**之后**（条目清理期间） | 可见 id = `1..=30`、30 行、逐 id 点查命中 1 ⇒ GREEN |

- 修复后落点后移的原因（可核）：改序后，「写出版本槽 → 登记版本」这一段内**没有 await 点**（墓碑写内部的 `get_page` await 发生在**写槽之前**；`record_version` 的 `tx_versions.write().await` 在无争用时首轮即 Ready）⇒ 基于任务让出的锚点只能在**登记之后**的下一个 await 点（PK 条目删除的 `spawn_blocking` await）命中。
- 该落点下 GREEN 的机制：墓碑**已登记** ⇒ 回收的中止清理看见它 ⇒ B 趟（无索引条目指向墓碑槽）⇒ `restore_tombstone_index_entries` 还原条目 + `mark_aborted` 中和墓碑 ⇒ 不压制前驱版本、条目还原 ⇒ 行仍可见。
- **如实上报的判别力边界**：修复后的夹具**无法再观测**到「登记之前」那一段（该段已无 await 点），因此「不带链指针写」对**该段**的保护是由机制推得的（不带链指针的墓碑**没有 `next_version`** ⇒ 根本不进入以 `next_version` 构造的抑制集合 ⇒ 不可能压制前驱版本），而非由本夹具直接观测；本夹具直接观测到的是**行为结论**（被放弃的 `DELETE` 不隐藏被删行、点查命中数、条目面），且**修复前**同一夹具正是在「登记之前」那段 RED。

### 条目面风险的实际观察（Plan Context「Risks and Notes 1」要求在 GREEN 中观察）

- 落点为「登记之后、条目清理期间」，回收的 `restore_tombstone_index_entries` 会重新插入该键的条目 ⇒ 实测：**该键的 PK 条目数 = 1**（`pk_entry_count(&db, 1) == 1`，断言在 `tests/dml_tombstone_abandon_window_test.rs:305`）⇒ 未出现残留的第二条条目。
- **回收后同键插入未被假阳性 `DuplicateKey` 拒绝**（Acceptance 2）：断言「该行真正被提交删除之后，同键可再次插入成功」——`DELETE FROM t WHERE id = 1`（提交）⇒ `INSERT INTO t VALUES (1, 777)` **成功** ⇒ 无残留条目挡住该键；随后点查 `id = 1` 仍命中 1 行。
- ⇒ T15 的 Stop-when（重复索引条目 / 假阳性 `DuplicateKey`）**未触发**。

### Gate 3（任务级 Review）

- **T14**：spec compliance —— 真实入口（真实文件库 + 真实 `Database::open` + 真实 `DELETE` / `REPLACE` 语句 + 锚点式放弃 + 下一条写语句触发回收）；断言锚定真实可观察行为（可见 id 集合、行数、逐 id 点查命中数、PK 条目数、同键再插入）；规模 N = 30（< 300，ISS23 约束）；无替身 / mock、无框架外脚本 / 判定层、无身份指纹 / revision pin / run-id / manifest / 时间顺序证明；`ANCHOR_TIMEOUT` 只作夹具失控兜底（触发即判失败，不是行为判据）✓。code quality —— `cargo build --tests` 0 warning；用例无「因错误原因通过」的路径（RED 已实测落在预期断言上）✓。
- **T15**：spec compliance —— 两条路径的写序与 `src/storage/data/table_manager.rs` 的 DDL `alter_drop_column` DELETE 腿**逐段同形**；`update_version_header_in_data_page` 的用法与 DDL 侧**完全一致**（含 `&[]` 元组字节）✓。Preserve —— `superseder_suppresses` 与四处抑制集合构造点**未动**；`mark_aborted` 的链指针语义**未动**；`abort_cleanup_versions` / `restore_tombstone_index_entries` 的 A/B 分桶与还原语义**未动**；「未命中零副作用早退」臂（`delete.rs:147-149`）**未动**；「`SlotNotFound` 容忍臂」**未动**（不写墓碑；登记对象仍为既有形态：写了墓碑登记墓碑槽、否则登记被删行的 rid / `old_values.is_some().then_some(rid)`）；`upsert.rs` 的「在位守卫」臂（`search(key) == Some(rid)` 才删 PK 条目）条件与在条目清理块内的次序**逐字保留**；WAL `Delete` 记录字段语义**未动**；磁盘格式**未动**；`free_page` 集合语义**未动** ✓。Forbidden —— 未新增公共抽象、未跨模块重构、未引入身份证明机制 / 判定层 / timer 阈值行为判据 ✓。
- **Gate 3 发现**：Critical 0、Important 0；Minor 3（见下）。

### Gate 4（Verified Completion）

- Task Definition 的 Verification **全部满足且基于本次真实运行**：`dml_tombstone_abandon_window_test` 3/0；`mvcc_tombstone_visibility_test` 11/0、`mvcc_abort_test` 3/0、`ddl_rewrite_consistency_test` 9/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`recovery_test` 3/0、`recovery_e2e_test` 6/0；`cargo test --lib` 420/0；`cargo build --tests` 0 warning。
- 本 Cycle **未跑全量回归**（按 change 预算归 Iteration 007 / T21），与 Plan Context 的 Verification 段一致 ⇒ 不构成 Gate 4 缺口。
- **完成声明有当次运行的真实输出** ✓；**未沿用任何已失效的结论**（`--lib` 基线与 MS35 / MS36 全量终态均已重跑 / 明确标注失效）。
- Acceptance 逐条落点：AR1（可见 id 完整、行数 30）✓ 实测；AR2（逐 id 点查命中 1 + 条目数 1 + 同键可再插入）✓ 实测；AR3（墓碑终态字节 / 索引条目集合 / WAL `Delete` 字段）✓ 实测；AR4（REPLACE 冲突行删除段同形）✓ 实测（修复前同样 RED）；AR5（既有语义零回归 + 磁盘格式不变）✓ 实测；AR6（无身份证明机制 / 无框架外判定层）✓ 走查。

### 自检发现、已修复内容与遗留 Minor

**已修复（本次实施中自行发现并修复）**

1. `delete.rs` / `upsert.rs` 两处方法文档注释仍写着旧的写序（`墓碑写 → 条目清理 → 登记`）⇒ 已按新序改写（`delete.rs:136-137`、`upsert.rs:605-608`），避免文档与实现分叉。
2. 新增测试文件存在 `rustfmt` 排版差异 ⇒ 已用 `rustfmt --edition 2021` 修正**本次新增/改动的三个文件**（`src/executor/delete.rs`、`src/executor/upsert.rs`、`tests/dml_tombstone_abandon_window_test.rs`），三者现均为 fmt-clean；**仓库其余既有的 fmt 差异未触碰**（不属本 Cycle 范围）。排版后重跑该目标仍为 `3 passed; 0 failed`，`cargo build --tests` 仍 0 warning。
3. 夹具的判读面：墓碑槽的元组字节为空 ⇒ `deserialize_row` 失败 ⇒ `first_int` 取 `None`；据此用「`first_int == Some(1)`」定位**被删行槽**、用「`deleted && next == 被删行`」定位**墓碑槽**（二者不会互相误判）。
4. `dump_chain` 对「槽数据长度不足 `VersionHeader::SIZE`」的槽直接跳过（不读半个版本头）——判读面更严格，不是放宽。

**实施过程中未发现新的产品代码缺陷**——本 Cycle 修复的即是承载缺陷本身（ISS24 的 DML 同形面）；既有域 MTV 的判据面（`superseder_suppresses`、四处抑制集合构造点、中止清理的 A/B 分桶与还原）全部未动。

**遗留 Minor（已记录，未在本 Cycle 处理）**

1. **夹具的判别力边界**（见上节）：修复后「登记之前」那一段不再含 await 点 ⇒ 本夹具只能观测到「登记之后」落点的行为结论；「不带链指针写」对该段的保护是机制推得而非本夹具直接观测。**未**为此新增夹具机制（如人为争用 `tx_versions` 锁）——那超出 Plan Context 规定的锚点形态（`anchor_slot_written` 同形），故如实上报而不自行扩范围。
2. **`IndexManager::delete` 的内部阻断任务**：`spawn_blocking` 的 B 树删除在语句 future 被丢弃后**仍会跑完**（`spawn_blocking` 任务不随 JoinHandle 丢弃而取消）⇒ 放弃落点处 PK 条目的删除可能「晚于」回收的条目还原；两种先后次序下实测条目数均为 1、可再插入成功 ⇒ 无可观察差异，但该并发面属既有实现，本 Cycle 不改。
3. 既有目标的条数与实际计数一致（`secondary_index_test` 27/0、`free_page_lifecycle_test` 12/0、`index_page_ownership_test` 5/0、`mvcc_tombstone_visibility_test` 11/0 等），与 Plan Context 的记载无偏差；Iteration 004 记录的 `free_page_lifecycle_test` 条数偏差（13 条说法）**不属本 Cycle**。

### Deviations from Plan

1. **夹具用例数多于 Plan 规定的 1 条**：Plan 的 First test 只规定 `abandoned_delete_keeps_the_deleted_row_visible`；实际新增 3 条——`abandoned_delete_keeps_the_deleted_row_visible`（AR1 / AR2）、`abandoned_replace_keeps_the_conflict_row_visible`（AR4 的 REPLACE 同形对照）、`committed_delete_keeps_the_canonical_tombstone_slot`（AR3 的墓碑终态字节 / WAL 字段 / 条目集合守门）。**理由**：AR3 要求「墓碑槽终态与修复前逐字节等价」的**锁定断言**，而该终态只在**已提交**的删除上才有确定形态（被放弃的删除的墓碑在回收后被 `mark_aborted` 中和 ⇒ 其 `create_tx_id` 变为 0，与修复前的终态不同形），故必须有一条独立的「提交后删除」用例来锁定终态；该用例在修复前即为 GREEN（是终态等价的守门，不是凑 RED 的用例）。**未放宽任何断言**。
2. **AR2 的「同键插入不被假阳性 `DuplicateKey` 拒绝」的实现形态**：该半句在「行仍可见」（AR1）的前提下字面不可满足——被删行仍可见 ⇒ 同键本就该被拒。Act 按**无残留条目**的语义实现：断言 (a) 回收后该键的 PK 条目数**恰好 1**（白盒直观测重复风险），(b) 该行**真正被提交删除之后**同键**可以再次插入成功**（行为观测假阳性拒绝），(c) 再插入后点查仍命中 1 行。**未**把断言改成「行不可见时同键可插入」（那会与 AR1 直接矛盾）。
3. **放弃落点后移到「登记之后」**：见「放弃实际落在窗口的哪一段」节。Plan Context 的 GREEN condition 明确要求写明落段并允许两种落段 ⇒ 属**授权内**；如实记录，不改判据、不改锚点形态凑落点。
4. **额外重跑 13 个既有测试目标**（2 批）：Plan Context 的 Verification 只列 9 条；因本改动触及 `DELETE` / `REPLACE` 路径，`upsert_test` / `constraint_enforcement_test` / `visibility_test` / `mvcc_commit_test` / `full_table_dml_test` / `executor_test` / `dml_tx_id_test` / `update_index_maintenance_test` / `index_page_ownership_test` / `index_page_ownership_stack_overflow_test` / `free_page_lifecycle_test` / `drop_persistence_test` / `checkpoint_test` 的覆盖面被触及 ⇒ 重跑（全部 0 failed）。属**加强**，不替代任何 Plan 指定的验证。

### Persisted Evidence

`None required` —— 全部结论可低成本重跑（1 个新增测试文件 + 8 个 Plan 指定目标 + 13 个受影响目标 + `cargo test --lib` + `cargo build --tests`），Act Response 已承载命令、退出码与 `test result:` 行；未创建 `evidence/` 目录。

### change 状态同步

- `tasks.md`：**T14 → done**、**T15 → done**；RTM 的 **MTV「未提交与已回滚墓碑不抑制、评估回溯前驱」（DML 同形面）行 → Done**（用例名与 Plan 计划名一致，无需改名）；Iteration Map 的 **005 行 → 已实施待审**（`Act Response` 状态 `reported`，`Review Result` 待 Plan Review）；「Next Iteration」段同步为「Iteration 005 已实施待审」。
- 未改 `openspec/specs/`、未改**全局** tasks / SNAPSHOT / 项目记忆、未归档 change、未创建后继 Cycle 或 Iteration、未填 `Plan Review`、未登记或修改 Issue（`.agents/issues/` 保持只读）。

### Experience Candidates

1. **Issue 候选（无新增）**：本 Cycle 未发现新的实质缺陷。承载缺陷（ISS24 的 DML 同形面）由本 Iteration 直接修复，**未**新建 Issue；ISS22–ISS25 的台账**未改动**（ISS25 依 Plan Review 裁定保持 `open`）。
2. **Runbook 候选——「DML 墓碑的放弃窗口」复现配方**：`CREATE TABLE t (id INT PRIMARY KEY, n INT)` → 写入 30 行 → 以锚点式放弃丢弃 `DELETE FROM t WHERE id = 1` 的语句 future（锚点 = 数据链上出现由该活跃事务创建的版本槽；`tokio::select!` + `biased`，锚点命中即 `drop(statement)`）→ 下一条写语句（`INSERT INTO t2 VALUES (1)`）触发库属隐式事务回收 → 观察可见 id 集合与逐 id 点查命中数。**证据**：本 Act Response 的 RED 与 GREEN 两段（命令与实际输出均在）与 `tests/dml_tombstone_abandon_window_test.rs`。命中「已端到端验证成功且可重复」——**但**该配方当前**只作为 Runbook 候选登记，不新建 Runbook**（Runbook 的建立由 Recorder 按用户指令执行）。

### 未解决问题与停止条件

- **未解决（需 Plan Review 裁决，非阻塞）**：修复后夹具只能观测到「登记之后」落点的行为结论，「不带链指针写」对「登记之前」那一段的保护是机制推得而非本夹具直接观测（见遗留 Minor 1）。请 Plan Review 裁定该判别力边界是否满足 AR1 / AR2 的验收强度；若要求更强的判别力，需 Plan 给出**不改夹具形态**或**经批准的夹具扩展**方案（Act 不自行扩范围）。
- **未解决（非阻塞）**：`IndexManager::delete` 的 `spawn_blocking` 任务在语句 future 被丢弃后仍会跑完（遗留 Minor 2）——可观察差异为零，仅作记录。
- **停止条件**：本 Cycle 的授权范围（T14 / T15）已完成且已验证——Act 写入反馈后**终止**；下一步由 `openspec-plan` 对本 Cycle 做 Review（`Review Result` 仍为 `pending`，由 Plan 填写）。全量回归仍归最后一个 Iteration 007 / T21（按 M19 分批串行纪律执行），本 Cycle 未跑。

## Plan Review

- Review Result: accepted

**Findings**

Plan Review 2026-10-25（`openspec-plan`，Review 模式）。独立阅读了完整 diff（`git diff` 的工作区 vs 索引部分 isolates 本 Cycle 的改动：`src/executor/delete.rs`、`src/executor/upsert.rs`；`tests/dml_tombstone_abandon_window_test.rs` 为新增未跟踪文件全文）、实际代码与 Act Response；**未**以 Act Self-Review 代替独立检查；**未**重复运行 Act 已通过且覆盖范围未变化的验证（来源：Act Response 的 GREEN 证据表与两个受影响批）。

- **F1（Act 偏离·非实质）Act Response 的「该段已无 await 点」表述不准确。** Act Response「放弃实际落在窗口的哪一段」节称：改序后「写出版本槽 → 登记版本」这一段内**没有 await 点**。独立读码：`delete.rs:177-202` 的墓碑写内部有 `await`（`write_tuple_to_data_page` → `get_page`），但它**发生在写槽之前**；`delete.rs:204-212` 的 `record_version` → `src/transaction/manager.rs:221-229` 的 `self.tx_versions.write().await` 是该段内**唯一**的 await 点，它在**无争用**时首轮即 Ready（tokio 信号量的快速路径），但**在 `tx_versions` 写锁被争用时返回 Pending** ⇒ 语句 future **可以**在该点挂起并被丢弃 ⇒ 「登记之前」这一段**是可达的放弃落点**。Act 自身文本已作限定（「无争用时首轮即 Ready」），但 Plan Context 要求写明的落段结论据此应按「无争用下不可达、争用下可达」表述。**结论不因此改变**（见 F4）。
- **F2（Act 偏离·非实质·Plan Context 授权内）放弃落点后移到「登记之后」。** Plan Context 的 T14 GREEN condition 明确要求「写明放弃实际落在窗口的哪一段（登记前 / 登记后）」⇒ 两种落段均被授权；Act 未改判据、未改锚点形态凑落点、未放宽断言 ⇒ 合规。
- **F3（Plan 错误·不阻塞）Acceptance 2 的字面读法与 Acceptance 1 自相矛盾。** Acceptance 1 要求回收后被删行**仍可见**；Acceptance 2 的后半句「回收后同键插入不被假阳性 `DuplicateKey` 拒绝」按字面读法（行仍可见时插入）**不可满足**（行仍可见 ⇒ 同键本就该被拒）。Act 按**无残留条目**语义实现：(a) 被回收键的 PK 条目数**恰好 1**（白盒直观测重复风险）、(b) 该行**真正被提交删除之后**同键**可再次插入成功**（行为观测假阳性拒绝）、(c) 再插入后点查仍命中 1 行——这是唯一自洽的读法，且**未**把断言改成与 AR1 矛盾的形式。判定：**Plan 的措辞自相矛盾**（非 Act 偏离）；AR2 以 (a)(b)(c) 三条为准 ⇒ 满足。本 Cycle **不产生规格增量**（Gate 2 决策，见 F8）⇒ 无可改写的规格文本；AR2 的措辞澄清归 **T20**（Iteration 007 的「规格增量与验收边界对齐」）在既有域对齐时一并处理。
- **F4（核心裁定·不构成 Acceptance Gap）「未注册窗口未被夹具直接观测」不阻塞验收。** 独立判定如下：
  1. **窗口内确有可放弃点**（见 F1）⇒ 「该段不可放弃」这一论证**不成立**；
  2. 但**防护不依赖该论证**：重排后窗口内的墓碑**不带链指针**（`delete.rs:181` / `upsert.rs:657`：`VersionHeader::new(tx_id, None).mark_deleted()`）⇒ `src/executor/data_scan.rs:238-292` 的 `build_superseded_map` **只**从 `vh.next_version()` 建链（`:262`）⇒ 无链指针的墓碑**根本不进入抑制集合** ⇒ `slot_is_superseded`（`:298`）不可能因它判 true ⇒ 它**不压制**任何前驱版本；扫描面对墓碑槽本身 `SkipSlot`（`data_scan.rs:489-491`）⇒ 也不产出行；条目清理在其**之后**（`delete.rs:227` / `upsert.rs:699`）⇒ PK / 唯一 / 二级条目**仍在位** ⇒ 点查仍命中。⇒ **窗口内的墓碑结构上无害，与该窗口是否可被放弃无关** ⇒ 缺陷已消解（结构性），不是「碰巧没测到」；
  3. **根因独立复核**：`superseder_suppresses`（`data_scan.rs:344-372`）对墓碑槽在无快照路径下取 `!active_tx_ids.contains(&vh.create_tx_id())` ⇒ **被放弃的删除者已不在活跃集合 ⇒ 其带链指针的墓碑被判定为已提交删除 ⇒ 压制整条链**。修复前「登记在最后」⇒ 放弃落在登记前 ⇒ 该表不在 `tx_version_tables`（`manager.rs:242-250`）内 ⇒ 回收的覆盖面（`database.rs:600` → `:628` → `manager.rs:322` 的 `abort_cleanup_versions`）不含该表 ⇒ 墓碑永不被 `mark_aborted` 中和 ⇒ 行不可见。修复后登记提前 ⇒ 回收**看得见**墓碑（`versions_created_by`（`manager.rs:547-593`）按 `create_tx_id` 找回未登记/已登记的版本槽）⇒ 中和 + `restore_tombstone_index_entries`（`manager.rs:607-681`）还原条目 ⇒ 行仍可见；
  4. **夹具的判别力**：同一夹具在**修复前**实测落在「登记之前」那段（读数 `registered = false`）并 RED，**修复后**落在「登记之后」并 GREEN ⇒ 该夹具对**目标行为**（被放弃的 `DELETE` 不隐藏被删行）具备判别力；「不带链指针」是**内部不变式**，已由上述代码路径独立坐实（`next_version` 是抑制集合的唯一来源），**不需要**新的夹具机制；强制 `tx_versions` 锁争用属**范围外的夹具扩展**（超出 Plan Context 规定的锚点形态），且**不增加**目标行为的判别力（争用下的落点与无争用下的落点共享同一行为结论）⇒ 不构成 Acceptance Gap，登记为**非阻塞 Minor finding**（夹具可观测性边界），不要求当前 Cycle 修复。
- **F5（通过·写序与终态）** 独立核对写序：`delete.rs` = 墓碑写（无链指针，`:181`）→ `clear_all_visible` ×2（`:189-192`）→ `record_version`（`:204-212`）→ `update_version_header_in_data_page` 补链指针（`:214-225`）→ PK（`:228`）/唯一（`:232-234`）/二级条目清理 → WAL（`:255-268`）；`upsert.rs::delete_conflict_row` = 同序（`:649-670` / `:672-684` / `:686-697` / `:699-723` / `:725-732`）。**与 DDL 参照形态逐段同形**：`src/storage/data/table_manager.rs:1089-1131`（`alter_drop_column` 的 DELETE 腿）= 无链指针写 → `clear_all_visible` → `record_version` → `update_version_header_in_data_page` → 条目清理 → WAL ⇒ 一致。终态等价：`update_version_header_in_data_page`（`src/storage/data_page.rs:182-207`）在**同一槽的偏移**处就地覆写 `VersionHeader::SIZE` 字节 ⇒ 修复后的槽终态字节 = `VersionHeader::new(tx, None).with_next_version(rid).mark_deleted().to_bytes()`，与修复前**一次写完**的形态逐字节相同 ⇒ Acceptance 3 成立。登记对象形态未变（`delete.rs` 仍为 `tombstone_rid.unwrap_or(rid)`；`upsert.rs` 仍为 `tombstone_rid.or_else(|| old_values.is_some().then_some(rid))`）⇒ 两条路径各自的 `SlotNotFound` 容忍臂语义未变。
- **F6（通过·RED 形状与落点读数）** RED 输出（Act Response 逐字引用）落在**预期断言**上：`missing ids: [1] (visible: [2..30])`、29 行 ⇒ 形状为「被放弃的 `DELETE` 使其已删除的那一行从数据面消失」，与 Plan Context T14 First test 的预期形态一致（非「行数不变内容错」或「点查命中 2 行」）。落点读数 `registered = false` **可信**：`abandon_statement`（`tests/dml_tombstone_abandon_window_test.rs:212-245`）的读数取自 `tokio::select!` 锚点分支胜出**之后**（语句 future 已被丢弃 ⇒ 现场冻结）⇒ 该读数即放弃点的现场；且 `registered = false` 与「登记在最后」的修复前写序一致（登记发生于条目清理之后）⇒ 读数与代码路径互相印证。第三条用例在修复前即 GREEN（终态守门，非凑 RED）⇒ 合规。
- **F7（通过·upsert 在位守卫与条目面）** `upsert.rs:704-713` 的「在位守卫」臂（`search(key) == Some(rid)` 才删 PK 条目）**逐字保留**（diff 中该块为上下文行，未被触碰）；条目清理顺序（PK → 唯一 → 二级）与 `delete.rs` 一致且无遗漏（`upsert.rs:700-723` / `delete.rs:228-242`）。登记对象的「悬空 rid 不入 `tx_versions`」语义未变。
- **F8（通过·既有已验收语义零回归）** `superseder_suppresses`（`data_scan.rs:344`）与四处抑制集合构造点（`data_scan.rs:238`、`table_manager.rs:1242` 的 `table_has_live_rows`、`create_index.rs` 回填、`wal/recovery.rs` 的 `rebuild_pk_indexes`）**均未改动**；`VersionHeader::mark_aborted` 的「保留链指针」语义**未动**；`abort_cleanup_versions` 的 A/B 分桶与 `restore_tombstone_index_entries` **未动**；`free_page` 集合语义**未动**；磁盘格式**未动**。中间态（墓碑已写、链指针未补）对并发读者的可观察面与修复前**一致**：无快照扫描下，删除者在活跃集合内 ⇒ 墓碑本就不压制（`superseder_suppresses` 的墓碑分支要求删除者**不在**活跃集合）⇒ 改序不改变读面。既有目标 `mvcc_tombstone_visibility_test` 11/0、`mvcc_abort_test` 3/0、`ddl_rewrite_consistency_test` 9/0、`recovery_test` 3/0、`recovery_e2e_test` 6/0、`--lib` 420/0 均为 Act 在**最终工作区**的真实运行结果（来源：Act Response GREEN 证据表），覆盖范围自此未变 ⇒ 直接沿用。
- **F9（通过·无身份证明机制与无框架外判定层）** 走查新测试文件与两处产品改动：无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明；判定全部落在 `cargo test` 内（可见 id 集合、行数、逐 id 点查命中数、PK 条目数、同键再插入）；无框架外脚本或封装命令；`ANCHOR_TIMEOUT`（夹具失控兜底，触发即 `panic!` 判失败）**不是**行为判据；无 timer 阈值行为判据。
- **F10（澄清·非 finding）本 Iteration 不产生规格增量 ⇒ 不存在 `specs/dml-tombstone-abandon-window/spec.md`。** Gate 2 已决（Plan Context「Gate 2 Readiness」第一项）：该行为由**既有域** `mvcc-tombstone-visibility`（MTV）承载——「未提交与已回滚墓碑不抑制、评估回溯前驱」（被放弃的 DML 删除 = 一次终将回滚的删除，其墓碑被回收中和为 aborted ⇒ 不压制前驱）、「墓碑自描述且不影响前驱版本」（终态字节等价）、「回滚后墓碑行的索引条目还原（PK 与唯一）」（条目还原面）——本 change 的 RTM 行「MTV『未提交与已回滚墓碑不抑制、评估回溯前驱』（既有域，ISS24 的 **DML 同形面**）| T14, T15 | Done」与之对应（用例名 `abandoned_delete_keeps_the_deleted_row_visible` 与计划名一致）。故**本 Cycle 无规格增量文本需要核对或更正**；`openspec/specs/mvcc-tombstone-visibility/spec.md` **未因本 Cycle 修改**（既有域不在本 change 的 MODIFY 面上，本 change 只新增域）。
- **F11（Act 偏离·非实质）夹具用例数 3 > 计划的 1。** 计划的 First test 只规定 `abandoned_delete_keeps_the_deleted_row_visible`；新增的 `abandoned_replace_keeps_the_conflict_row_visible`（AR4 的同形对照）与 `committed_delete_keeps_the_canonical_tombstone_slot`（AR3 的终态守门）为**增量**：未放宽任何断言，规模 N = 30 < 300（ISS23 约束），守栏用例在修复前即 GREEN（不冒充 RED）⇒ 不阻塞。
- **F12（非阻塞 Minor·沿用 Act 的诚实边界）** ①夹具判别力边界（F4）：修复后夹具只能观测「登记之后」落点的行为结论，「不带链指针写」对「登记之前」那一段的保护由机制论证坐实而非夹具直接观测——记录，不要求当前 Cycle 修复（强制锁争用属范围外夹具扩展，且不增加目标行为的判别力）。②`IndexManager::delete` 的 `spawn_blocking` B 树删除在语句 future 被丢弃后仍会跑完（既有实现，未改动）——可观察差异为零，记录。③`free_page` 集合语义、`VersionHeader` 磁盘格式、MS36 的 G1/G2/G3 兼容面：本 Cycle 均未触碰。

**Acceptance Gaps**

无。Acceptance 1–6 逐条满足：AR1（可见 id 集合完整、行数 30）✔ 夹具实测（`tests/dml_tombstone_abandon_window_test.rs:262-292`，GREEN）；AR2（逐 id 点查命中 1 + 被回收键条目数恰好 1 + 行真正删除后同键可再插入 + 再插入后点查命中 1）✔ 夹具实测（`:294-328`，按 F3 的裁定读法）；AR3（墓碑槽终态 22 字节 + 索引条目集合 + WAL `Delete` 记录字段）✔ 夹具实测（`committed_delete_keeps_the_canonical_tombstone_slot`）；AR4（REPLACE 冲突行删除段同形）✔ 夹具实测（修复前同形状 RED）；AR5（既有语义零回归 + 磁盘格式不变）✔ 沿用 Act 的真实运行（`mvcc_tombstone_visibility_test` 11/0、`mvcc_abort_test` 3/0、`ddl_rewrite_consistency_test` 9/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`recovery_test` 3/0、`recovery_e2e_test` 6/0、`--lib` 420/0）与 F8 的代码走查；AR6（无身份证明机制 / 无框架外判定层）✔ F9 走查。

**Evidence**

- 代码与 diff（本 Cycle 的工作区改动 = `git diff` 的未暂存部分）：`src/executor/delete.rs:177-268`（`delete_row` 的墓碑写 / 登记 / 补链指针 / 条目清理 / WAL）、`src/executor/upsert.rs:649-732`（`delete_conflict_row` 同序）；`tests/dml_tombstone_abandon_window_test.rs`（新建，3 用例 + 9 个夹具辅助函数）。
- 机制坐实的读码位点：`src/executor/data_scan.rs:238`（`build_superseded_map`，`:262` 只认 `next_version`）、`:298`（`slot_is_superseded`）、`:344`（`superseder_suppresses`，墓碑分支 `:352`）、`:489-491`（墓碑槽 `SkipSlot`）；`src/transaction/manager.rs:221-229`（`record_version` 的写锁 await）、`:242-250`（`tx_version_tables` 只含已登记版本的表）、`:322`（`abort_cleanup_versions`）、`:547-593`（`versions_created_by`）、`:607-681`（`restore_tombstone_index_entries`）；`src/database.rs:600`（`reclaim_abandoned_implicit_transactions`）、`:628`（`reclaim_implicit_tx`）；`src/storage/data_page.rs:182-207`（`update_version_header_in_data_page` 就地覆写槽头）；`src/storage/data/table_manager.rs:1089-1131`（DDL 参照形态）；`src/storage/data/table_manager.rs:1242-1282`（`table_has_live_rows` 的 `pointed` 构造点，未改动）。
- 验证结论（**沿用 Act Response 的真实运行，覆盖范围未变化**，未重复运行）：`cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` → 3/0；`mvcc_tombstone_visibility_test` 11/0；`mvcc_abort_test` 3/0；`ddl_rewrite_consistency_test` 9/0；`index_manager_test` 3/0；`secondary_index_test` 27/0；`recovery_test` 3/0；`recovery_e2e_test` 6/0；`cargo test --lib -- --test-threads=1` → 420/0；`cargo build --tests` → 0 warning；额外受影响面两批 134/0 + 31/0。RED 证据（修复前的产品代码）：`1 passed; 2 failed`，失败断言 = `missing ids: [1] (visible: [2..30])`。本 Cycle 未跑全量（预算归 Iteration 007 / T21），与 Plan Context 一致 ⇒ 不构成 Gate 4 缺口。
- Persisted Evidence = `none` ⇒ 无 `evidence/` 目录，不据此提出问题。

**Follow-up Decision**

接受当前 Cycle。判据：既有 Acceptance 1–6 全部满足且无阻塞项；F1/F2/F11 为非实质偏离（表述精度 / 授权内 / 增量用例），F3 为 Plan 自身的措辞矛盾（已按唯一自洽读法裁定，澄清归 T20），F4 的判别力边界经独立读码判定**不构成 Acceptance Gap**（防护是结构性的，不依赖「该窗口不可放弃」这一不成立的论证）；F12 为记录性 Minor。**不要求当前 Cycle 修复**，不创建 rework / replan Cycle。Iteration 005 的 Iteration Plan **保持不变**（T14 / T15、依赖 001、验收边界、验证边界均无需修改）。当前 Iteration 完成 ⇒ 按 Iteration Map 展开**下一个** Iteration（006）。

**Iteration Plan Update**

None（Iteration 005 的 Iteration Plan 不变）。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/006-index-read-bounded-read/000-initial.md`（Iteration 006 / Cycle 000，承载 T22 / T23：索引树读路径的有界读与显式拒绝）。
