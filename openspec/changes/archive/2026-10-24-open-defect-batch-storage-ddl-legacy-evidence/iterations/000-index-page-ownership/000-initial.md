# Iteration 000 / Cycle 000: 索引页归属唯一性——合并页只释放一次，下降必然终止

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: 变更 `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 的 Iteration 000 成果：IPO R1（被合并掉的树页只释放一次）、IPO R2（被释放的页不同时属于两个对象）、IPO R3（索引插入路径的下降必然终止）；MS35 已验收的 `free-page-lifecycle` 判定时点语义保持不变。
- Excluded scope: Iteration 001（列改写的数据链一致性与放弃可见性）、Iteration 002（释放路径的有界读）、Iteration 003（改写窗口的读一致性）、Iteration 004（释放可分配判定的耐久性前提）、Iteration 005（旧格式兼容的证据补齐）；B-3（已被已验收规格显式界定为范围外既有行为）；ISS22 / ISS24 / ISS25 的修复面。

**Objective**

索引树删除路径上的叶合并与内部节点合并，对被合并掉的节点页**只释放一次**；被释放的页在进入可分配集合后**不会被交给两个对象**；索引插入路径的下降**必然终止**——`ALTER TABLE DROP COLUMN` 在索引树多层规模下不再因下降成环而栈溢出、进程 abort。ISS23 的归属结论（`design.md` D3 的两条路径之一）确定并写入 Act Response。

**Background**

None（initial Cycle；缺陷来源与候选机制见 change 的 `proposal.md` 与 `design.md` D2 / D3）。

**Investigation Facts**

- Starting point: HEAD `0a08b14`（ahead of origin 9）。工作区有 **55 项未提交改动**（MS34 取消/超时批、MS34 issue-fix 批〔ISS20/ISS21〕、MS35 空闲页批、MS36 兼容批的产物），属**既有现场**：不得回滚、不得清理、不得提交。本 Cycle 的改动在这些改动之上叠加。
  - `src/storage/btree/btree.rs` 已含 MS35 批新增的三处 `complete_free_release` 调用（`:622`、`:675`、`:889`）。`git diff src/storage/btree/btree.rs` 显示 **`free_page` 的重复调用行未被 MS35 改动**（`:610` 与 `:672` 两处 `free_page` 均为上下文行）⇒ **B-2 的重复释放是既有缺陷**，非 MS35 引入。
  - MS35 批的 `free-page-lifecycle` 增量（`free-page-lifecycle` 新域 + `drop-persistence-boundary` / `drop-table-physical-free` 两条 MODIFIED）**尚未合并进 `openspec/specs/`**；本 Cycle 的规格增量是**新域** `index-page-ownership`，不 MODIFY 该未合并域。
  - 既有验证结论（未被本次改动覆盖，可沿用）：M19 分批全量最近一次终态 **2192 passed / 0 failed / 2 ignored**（`tests/*.rs` 135 个文件 + `--lib --bins` = 136 目标 / 13 批）；来源 `.agents/runbooks/m19-batched-full-regression.md`（R48）与 MS35 / MS36 的收尾记录。本 Cycle **不跑全量**（收尾全量在 Iteration 005）。
- Current-State Evidence（Plan 已逐行确认，位置为**当前工作区**行号）:
  - **叶合并的重复释放**：`src/storage/btree/btree.rs:583` `fn merge_leaves` 在 `:610` 调 `self.loader.free_page(right_id)?`，在 `:622` 调 `self.loader.complete_free_release(&[right_id])`，并在 `:625-629` 返回 `MergeInfo { freed_page_id: right_id, separator_key: sep_key, new_root: None }`。
  - **上级阶段的再次释放**：`src/storage/btree/btree.rs:631` `fn delete_from_internal` 在收到 `Some(mi)` 且 `mi.new_root.is_none()` 时调用 `handle_child_merge`；`:655` `fn handle_child_merge` 在 `:672` 调 `self.loader.free_page(merge_info.freed_page_id)?`、在 `:675` 调 `complete_free_release(&[merge_info.freed_page_id])` ⇒ **同一页号在一次删除操作中被释放两次**。
  - **内部节点合并的第二条重复释放路径**：`src/storage/btree/btree.rs:852` `fn merge_internal_nodes` 在 `:877` 调 `self.loader.free_page(right_id)?`、在 `:889` 调 `complete_free_release(&[right_id])`，并返回 `MergeInfo { freed_page_id: right_id, ... }`；该 `MergeInfo` 经 `:724` `fn handle_internal_underflow` → `:699-704` `handle_child_merge` 的 `Some(pid)` 分支向上返回，最终在**祖父层**的 `handle_child_merge` `:672` **再次**释放同一页号。
  - **重复释放如何变成「同一页交给两个对象」**：`src/storage/buffer_pool.rs:493` `free_page` → `self.pages.remove(&page_id)` + `self.storage.free_page(page_id)` → `src/storage/file_storage.rs:504` `free_page` 把 `(page_id.0, None)` **推进待释放集合** ⇒ 同一页号出现两条待释放条目；`src/storage/file_storage.rs:555` `complete_free_release` 把 `pages` 里页号的**全部**未完成条目标记为完成（重复调用因 `entry.1.is_none()` 守卫而成为空操作）；`:568` `promote_free_releases(limit)` 对完成且编号 `< limit` 的条目 `free.push(page)` ⇒ **同一页号被推进可分配集合两次**；`:449` `allocate_page` 的 `self.free_pages.lock().unwrap().pop()` 可先后两次弹出**同一页号** ⇒ 两个对象各自持有并改写同一页 ⇒ 静默损坏。
  - **下降递归点**：`src/storage/btree/btree.rs:237` `fn insert_into_page` 的内部节点分支在 `:322-326` 用 `InternalNodeRef::find_child_page_id_binary` 选子页，在 `:327` 递归 `self.insert_into_page(child_page_id, key, row_id)?`；既有 gdb 现场（ISS23 登记）显示 35+ 层同 `page_id` 帧 ⇒ 下降成环 ⇒ 栈溢出、进程 abort。`:207` `pub fn insert` 的根分裂路径在 `:207-235` 处理新根发布。
  - **释放链路的其余释放点**（本 Cycle **不改**，仅登记）：`:766` `fn shrink_root` 在 `:779` 释放旧根（按注释**不发**完成标记，锚点写是目录 root 更新，由调用方发出）；`:790` / `:821` 的 redistribute 路径不释放页。
  - **既有测试入口**：`tests/btree_merge_test.rs`（叶合并既有用例）、`tests/btree_split_test.rs`、`tests/btree_scale_test.rs`、`tests/free_page_lifecycle_test.rs`（MS35 新增）、`tests/drop_table_free_test.rs`。**本 Cycle 新增** `tests/index_page_ownership_test.rs`。
- Code and Critical Path:
  - 调用链（删除→合并→释放）：`IndexManager::delete` → `BTree::delete`（`:408`）→ `delete_from_page`（`:421`）→ `delete_from_leaf`（`:442`）→ `handle_leaf_underflow`（`:475`）→ `merge_leaves`（`:583`，释放 `right_id`）→ 返回 `MergeInfo` → `delete_from_internal`（`:631`）→ `handle_child_merge`（`:655`，**再次释放** `merge_info.freed_page_id`）→ 必要时 `handle_internal_underflow`（`:724`）→ `merge_internal_nodes`（`:852`，释放 `right_id`）→ 其 `MergeInfo` 再上浮一层 → 祖父层 `handle_child_merge`（`:655`，**再次释放**）。
  - 状态所有权：页的「可分配」归属由 `FileStorage` 的 `free_pages`（可分配集合）与 `pending_frees`（待释放集合）持有；`release_id` 是释放完成编号；`promote_free_releases(limit)` / `discard_free_releases(limit)` 由 checkpoint 第 2 步驱动（`src/wal/checkpoint.rs:207-220`）。
  - 动态边：`SyncPageLoader::free_page`（`src/storage/btree/sync_loader.rs:35`）与 `SyncPageLoader::complete_free_release`（`:40`）同步转发到 `BufferPool` → `FileStorage`。
  - 外部影响：页号被交给两个对象 ⇒ 两个结构各自改写同一页 ⇒ 其一被覆盖 ⇒ 索引树结构损坏；损坏形态之一是「内部节点的子指针指向可达自身」⇒ 插入路径下降成环 ⇒ 栈溢出。

**Implementation Guidance**

- 建议顺序：先写 T1 的 RED 用例（把「同一页号在一次删除操作中只进入待释放集合一次 / 可分配集合中同页号不出现两次」变成可观察断言），确认修复前失败；再做 T2 的一处修复（删除 `handle_child_merge` `:672-675` 的重复释放——`merge_leaves` 与 `merge_internal_nodes` **两处都已经释放过该页**，`handle_child_merge` 的职责只是移除父节点分隔键与处理随后的 underflow）；再跑 T3 的复现判定。
- 关键取舍（`design.md` D2）：**不**改 `free_page` / `complete_free_release` / `pending_frees` 的集合语义（MS35 已验收的 `free-page-lifecycle` 判定时点不变）；**不**把释放点从 `merge_leaves` / `merge_internal_nodes` 后移到 `handle_child_merge`（那会改变 MS35 已验收的「锚点写 = 父节点分隔键移除（同一写单元内的树页写），此刻已发出」的时点）。修复点选在 `handle_child_merge` 侧，是因为它是**唯一**在两条合并路径上都会经过、又**不负责**释放的公共位置。
- T3 的判定协议见 `design.md` D3：B-2 修复后复现消失 ⇒ 归属结论为「B-2 的下游后果」；仍复现 ⇒ 归属结论为独立成因，须在本 Cycle 内定位到具体下降环（环上的页号序列 + 形成环的那次写入）并修复，**归属结论与定位证据写入 Act Response**；若定位所需的执行依据超出本 Cycle 的 Task Definition（需要新的自包含调查），按 `openspec-plan` 的 Review 分类退回 Plan（`replan-required`），**不得**在 Iteration 000 内扩大范围。
- **规模约束**：ISS23 的触发面约在 300 行以上（登记现场 N=300 / 600 / 2000）。**本 Cycle 不得使用 ≥ 300 行的表规模构造用例**；若某个 RED 用例确实需要更大规模，先解决 ISS23 本身（否则该用例无法在修复前被观察到）。
- 验证入口：新增 `tests/index_page_ownership_test.rs`，走真实入口（真实文件库 + 真实 `Database::open` / DML / `ALTER TABLE DROP COLUMN`）。可分配页集合用既有可观察量断言（如 `storage.page_count()`、完整性扫描），**不新增指标**；判定只在 `cargo test` 集合内。

**Task Definitions**

### T1: 被合并掉的页只进入待释放集合一次

- Requirement/Scenario: IPO R1（被合并掉的树页只释放一次）·场景「叶合并后被合并掉的页只进入待释放集合一次」「合并路径不释放已被其它路径释放的页」；IPO R2（被释放的页不同时属于两个对象）·场景「可分配集合中同一页号不出现两次」「释放后的页不被两个对象同时持有」。
- Depends on: None
- Targets: `tests/index_page_ownership_test.rs`（新增）；`src/storage/btree/btree.rs::BTree::merge_leaves` / `BTree::handle_child_merge` / `BTree::merge_internal_nodes`（作为被测对象，不在本任务修改）。
- Current behavior: 一个发生兄弟合并的删除操作使被合并掉的右叶页在 `merge_leaves` 与 `handle_child_merge` 各被释放一次 ⇒ 待释放集合中出现同一页号两条条目 ⇒ 提升后该页号在可分配集合中出现两次 ⇒ 可先后被分配给两个对象。
- Required behavior: 同一次删除操作中，被合并掉的页在待释放集合中只出现一次；其释放完成标记只发一次；提升后的可分配集合中该页号只出现一次；该页不会被两个对象同时持有。
- Preserve: MS35 已验收的 `free-page-lifecycle` 判定时点（释放完成标记的发出时机、checkpoint 第 2 步的取样与提升/丢弃语义）不变；`merge_leaves` / `merge_internal_nodes` 的锚点写语义（父节点分隔键移除属同一写单元）不变；叶合并后的树结构与键序不变。
- Forbidden: 不得修改 `FileStorage::free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` 的集合语义；不得修改 `pending_frees` / `free_pages` 的数据结构；不得在测试框架外建判定脚本或封装命令；不得引入身份证明机制（run-id / 指纹 / revision pin）；不得用 timer 阈值作行为判据。
- First test: `tests/index_page_ownership_test.rs`（新增，RED）。位置：真实文件库 + 真实 `Database::open`；动作：建表 → 插入足够跨过叶分裂的行 → 触发主键索引的删除路径使叶发生兄弟合并 → 观察释放集合。命令：`cargo test --test index_page_ownership_test -- --test-threads=1`。预期（修复前）：断言「同一页号只被释放一次」或「可分配集合中无重复页号」**失败**（现状为重复）。
- GREEN condition: 上述断言在修复后全部通过，且测试进程不 panic、不 abort。
- Verification: `cargo test --test index_page_ownership_test -- --test-threads=1` 退出码 0、failed 计数 0；既有 `cargo test --test btree_merge_test -- --test-threads=1`、`cargo test --test btree_split_test -- --test-threads=1`、`cargo test --test free_page_lifecycle_test -- --test-threads=1` 退出码 0、failed 计数 0（既有断言不得被弱化）。失败含义：T1 的 RED 未复现 ⇒ 触发条件不足以造成重复释放，须调整用例规模/形态（**不**放宽断言）；既有用例失败 ⇒ 修复越界。
- Stop when: 重复释放无法在真实入口下被观察（则回到 Plan 复核触发条件与断言口径）；或修复需要改动 MS35 已验收的集合语义 / 判定时点（则回到 Plan）。

### T2: 消除合并路径的重复释放

- Requirement/Scenario: IPO R1 · 场景「叶合并后被合并掉的页只进入待释放集合一次」「合并路径不释放已被其它路径释放的页」；IPO R2 · 场景「可分配集合中同一页号不出现两次」。
- Depends on: T1
- Targets: `src/storage/btree/btree.rs::BTree::handle_child_merge`（`:655-710`，其中 `:671-676` 的释放与完成标记）。
- Current behavior: `handle_child_merge` 在收到 `MergeInfo` 后再次释放 `merge_info.freed_page_id`；该页号已由 `merge_leaves`（叶合并）或 `merge_internal_nodes`（内部节点合并）释放过 ⇒ 同一页号两次进入待释放集合。
- Required behavior: `handle_child_merge` **不再**释放 `merge_info.freed_page_id`（该页的释放责任归属其产生方 `merge_leaves` / `merge_internal_nodes`）；该页在一次删除操作中只进入待释放集合一次，只发一次释放完成标记；提升后可分配集合中该页号只出现一次。
- Preserve: `handle_child_merge` 的其余职责不变——移除父节点中对应分隔键、随后按 `needs_action` 处理内部节点 underflow 或根收缩；叶合并与内部节点合并的锚点写语义与完成标记发出时机不变；树的键序与结构不变。
- Forbidden: 不得把释放点从 `merge_leaves` / `merge_internal_nodes` 后移到 `handle_child_merge`（会改变 MS35 已验收的锚点写时点）；不得改 `FileStorage` 的释放集合语义；不得顺手重构 `merge_leaves` / `merge_internal_nodes` 的其它逻辑；不得新增身份证明机制或判定层。
- First test: 复用 T1 的用例（变更前已 RED）。命令：`cargo test --test index_page_ownership_test -- --test-threads=1`。预期（修复前）：T1 的重复释放断言失败。
- GREEN condition: T1 全部用例通过；既有 `btree_merge_test` / `btree_split_test` / `free_page_lifecycle_test` / `drop_table_free_test` 通过。
- Verification: `cargo test --test index_page_ownership_test -- --test-threads=1` + `cargo test --test btree_merge_test -- --test-threads=1` + `cargo test --test btree_split_test -- --test-threads=1` + `cargo test --test free_page_lifecycle_test -- --test-threads=1` + `cargo test --test drop_table_free_test -- --test-threads=1`，各退出码 0、failed 计数 0。失败含义：合并路径的树结构或既有释放语义被改变。
- Stop when: 移除重复释放导致某一合并路径的页**未被释放**（既有释放断言失败且无法在本任务范围内修复）⇒ 回到 Plan；或修复需要改动 `FileStorage` 的集合语义 ⇒ 回到 Plan。

### T3: ISS23 的复现与归属判定

- Requirement/Scenario: IPO R3（索引插入路径的下降必然终止）·场景「大规模删列改写不因索引下降耗尽栈」「下降成环时插入失败而不是崩溃」。
- Depends on: T2
- Targets: `tests/index_page_ownership_test.rs`（新增用例）；必要时 `src/storage/btree/btree.rs::BTree::insert_into_page`（`:237-406`，内部节点分支 `:322-327` 的下降）与其内部节点分裂/分隔键插入路径。
- Current behavior: `ALTER TABLE DROP COLUMN` 的改写路径对主键索引逐行删除并重新插入条目；在索引树多层规模下，既有 gdb 现场（ISS23 登记）显示 `insert_into_page` 出现 35+ 层同 `page_id` 递归帧 ⇒ 下降成环 ⇒ 栈溢出、进程 abort。
- Required behavior: 改写路径 SHALL NOT 触发栈溢出或进程 abort；插入路径的下降 SHALL 必然终止——若结构已被破坏到某节点可达自身，插入 SHALL 返回错误而非耗尽栈；索引树结构保持自洽。
- Preserve: 正常索引插入 / 删除 / 查找的既有语义与性能形态不变；B-Tree 的键序、分裂中键与分隔键规则不变；既有 `btree_*` 用例的断言语义不变。
- Forbidden: 不得用「捕获 abort 后进程退出」当作通过条件（必须观察到正常返回或明确错误）；不得为绕开规模约束而把用例规模提到 ≥ 300 行（除非 ISS23 本身已解决）；不得引入身份证明机制、判定层或 timer 阈值判据；不得在归属结论未确定前扩大到 B-Tree 之外的模块。
- First test: `tests/index_page_ownership_test.rs`（新增，RED）：真实文件库 + 真实 `Database::open` → 建表并写入足以让主键索引达到多层的行（**行数 < 300**，见「规模约束」）→ 执行 `ALTER TABLE DROP COLUMN`。命令：`cargo test --test index_page_ownership_test -- --test-threads=1`。预期（修复前）：进程 abort / 栈溢出，或用例无法完成（此时记录 gdb 栈上的同 `page_id` 帧序列作为定位证据）。T2 完成后重跑同一用例即为 T3 的 GREEN 判据起点。
- GREEN condition: 改写语句正常返回（成功或明确错误），测试进程不 abort；插入路径在结构被破坏时返回错误而不耗尽栈；索引结构自洽（既有完整性断言通过）。
- Verification: `cargo test --test index_page_ownership_test -- --test-threads=1` 退出码 0、failed 计数 0；`cargo test --test btree_test -- --test-threads=1`、`cargo test --test btree_scale_test -- --test-threads=1` 退出码 0、failed 计数 0。Act Response SHALL 记录归属结论（B-2 的下游后果 / 独立成因）与定位证据。
- Stop when: 归属判定为独立成因且定位所需的执行依据超出本 Cycle 的 Task Definition（需要新的自包含调查或新的规格决定）⇒ 停止并退回 Plan（`replan-required`），在 Act Response 写明 Blocker Handoff；或需要 ≥ 300 行规模的用例才能复现 ⇒ 先解决 ISS23 本身（回到 Plan 复核范围）。

**Invariants**

- MS35 已验收的 `free-page-lifecycle` 语义不变：释放先进入待释放集合、锚点写发出后才标记完成、checkpoint 第 2 步按取样提升或丢弃、崩溃后未完成的释放不是可分配页。
- 索引树的键序、分裂中键、分隔键规则与既有测试断言不变。
- 页的「可分配」归属唯一：一个页号在任一时刻至多属于一个对象。
- 不改磁盘格式（`VersionHeader` 布局、目录行布局均不变）。
- 不引入身份证明机制；不建测试框架外的判定层；不用 timer 阈值作行为判据；不用默认并行度跑任何需要串行语义的用例（定向用例按本 Task Definition 的 `--test-threads=1` 执行）。
- 工作区既有未提交改动不回滚、不清理、不提交。

**Non-goals**

- DDL 改写面的数据链一致性与放弃可见性（Iteration 001：ISS22 / ISS24）。
- 释放路径的有界读（Iteration 002：B-4）。
- 改写窗口的读一致性（Iteration 003：ISS25）。
- 释放可分配判定的耐久性前提（Iteration 004：B-1）。
- 旧格式兼容的证据补齐（Iteration 005：C-1 / C-2 / C-3）。
- B-3（已被已验收规格显式界定为范围外既有行为）。
- `FileStorage` 释放集合语义的任何改动；`SlottedPageRef::get_slot_data` 的签名改造；完整性扫描的诊断分级。

**Acceptance**

1. **IPO R1**：同一次删除操作中，被合并掉的页在待释放集合中只出现一次，其释放完成标记只发一次（证据：T1/T2 的用例 + `src/storage/btree/btree.rs::BTree::handle_child_merge` 的释放点移除）。
2. **IPO R2**：提升后的可分配集合中该页号只出现一次；该页不会被两个对象同时持有（证据：T1/T2 的用例）。
3. **IPO R3**：`ALTER TABLE DROP COLUMN` 在索引树多层规模下不触发栈溢出 / 进程 abort；插入路径下降必然终止（证据：T3 的用例 + Act Response 的归属结论）。
4. 既有断言零回退：`btree_merge_test` / `btree_split_test` / `btree_test` / `btree_scale_test` / `free_page_lifecycle_test` / `drop_table_free_test` 全部退出码 0、failed 计数 0。
5. ISS23 的归属结论已确定并写入 Act Response（`design.md` D3 的两条路径之一，且给出定位证据）。

**Verification**

| Scenario | 判定方式 | 命令 / 步骤 | 通过依据 |
|---|---|---|---|
| 叶合并后被合并掉的页只进入待释放集合一次 | 真实入口 + 既有可观察量 | `cargo test --test index_page_ownership_test -- --test-threads=1` | 退出码 0、failed 0 |
| 可分配集合中同一页号不出现两次 | 同上（同文件内另一用例） | 同上 | 退出码 0、failed 0 |
| 释放后的页不被两个对象同时持有 | 同上（分配后校验结构自洽） | 同上 | 退出码 0、failed 0 |
| 大规模删列改写不因索引下降耗尽栈 | 真实入口（真实 `Database::open` + 真实 `ALTER TABLE DROP COLUMN`） | 同上 | 语句正常返回、进程不 abort |
| 下降成环时插入失败而不是崩溃 | 真实入口（破坏后的结构 + 真实插入） | 同上 | 返回错误、无栈溢出 |
| 既有行为零回退 | 真实入口（既有测试文件） | `cargo test --test btree_merge_test -- --test-threads=1` 等 6 条 | 各退出码 0、failed 0 |

**Gate 2 Readiness**

PASS——Step 5 审查计划 11 项全部通过；RTM 无 `Missing`、无 `Simplified`；设计闭合（行为差异、接口、错误语义与关键选择已在 `design.md` D2 / D3 明确）；本 Cycle 的 tasks / specs / design 一致；`Persisted Evidence` 模式明确；**用户批准计划**（goal 指令原文，见 `proposal.md` 的「授权与批准依据」）。

BLOCKED / SKIPPED：**None**。

**Persisted Evidence**

- Mode: none

理由：本 Cycle 的验收全部落在 `cargo test` 集合内的定向用例上（命令、实际结果、修改文件与符号写入 Act Response 即足够）；不存在「Act Response 装不下」「无法低成本重跑」「缺了就无法作决定」的情形；样本、gdb 栈等均为一次性定位输入，不是需要归档的验收证据。**Plan 不创建 `evidence/` 目录。**

**Risks and Notes**

- **ISS23 的归属尚未确定**（非实质未知项，已在 Task Definition T3 中给出判定协议与停止条件）：本 Cycle 的结论是「B-2 的下游后果」还是「独立成因」由 Act 的 RED 复现决定；若为独立成因且定位超出当前执行依据，按 `replan-required` 退回 Plan，**不在本 Iteration 内扩大范围**。
- **ISS23 的规模约束**（行数 < 300）可能使「多层索引」难以在 RED 阶段被观察到；若小规模无法复现多层下降，须在 Act Response 记录实际达到的层数，并按 T3 的「Stop when」退回 Plan 复核，**不得**擅自把规模提到 ≥ 300 行。
- `merge_internal_nodes` 的重复释放路径（经祖父层 `handle_child_merge`）在既有 gdb 现场中未必出现过，但读码确认其成立；T2 的修复点（`handle_child_merge`）覆盖两条路径，**修复后须验证内部节点合并路径也被覆盖**（用既有 `btree_merge_test` 的既有断言 + 新增用例中至少覆盖一条内部节点合并形态）。
- 本 Cycle **不跑全量回归**（收尾全量在 Iteration 005，全 change 一次预算）；若 Act 发现必须提前跑全量才能判定，须在 Act Response 记录理由并回报 Plan 追加预算。
- 既有验证结论沿用来源：MS35 / MS36 的收尾全量终态 2192/0/2 与 `.agents/runbooks/m19-batched-full-regression.md`（R48）；本 Cycle 未改动其覆盖范围之外的内容。

## Act Response

- Status: reported

### 任务完成清单

| 任务 | 状态 | 涉及文件与符号 |
|---|---|---|
| T1 B-2 先行 RED 用例 | done | 新增 `tests/index_page_ownership_test.rs`：`merged_index_page_enters_the_allocatable_set_once`（库级，真实 `Database::open` + DML）、`sibling_merge_releases_the_merged_page_once`（树级）、`internal_merge_releases_the_merged_page_once`（内部节点合并形态） |
| T2 消除合并路径的重复释放 | done | `src/storage/btree/btree.rs::BTree::handle_child_merge`（移除 `:671-676` 的 `free_page` + `complete_free_release`）；`BTree::MergeInfo`（移除因本次改动成为孤儿的 `freed_page_id` 字段及 4 处构造点） |
| T3 ISS23 复现与归属判定 | done | 新增 `tests/index_page_ownership_stack_overflow_test.rs::released_index_page_never_breaks_the_insert_descent`（ISS23 复现，250 行）；`src/storage/btree/btree.rs::BTree::insert_into_page`（下降路径守卫：成环时返回错误而非耗尽栈）；`tests/index_page_ownership_test.rs::insert_into_a_cyclic_structure_returns_an_error`、`::alter_table_drop_column_completes_without_exhausting_the_stack` |

change 状态同步：`tasks.md` 的 T1 / T2 / T3 → `done`；RTM 的 IPO R1 / R2 / R3 三行 → `Done`（First Test 列补记**实际用例名**）；Iteration Map 的 000 行 → 「Act 已完成（Review Result 待 Plan 填写）」。未触碰全局 `tasks.md` / `SNAPSHOT.md` / 项目记忆 / `openspec/specs/`，未归档 change，未创建下一 Cycle 或下一 Iteration，未登记或修改 Issue。

### 实际改动

**产品代码（1 个文件，3 处语义变更）——`src/storage/btree/btree.rs`**

1. **`BTree::handle_child_merge`（T2 的修复点）**：删除「再次释放 `merge_info.freed_page_id`」的整块（`free_page` + `complete_free_release`）。释放与释放完成标记由该页的**产出方**发出——`merge_leaves`（叶合并，`:640` 释放 / `:652` 标记）与 `merge_internal_nodes`（内部节点合并，`:877` 释放 / `:889` 标记）都在各自「父节点分隔键移除」这一锚点写之后释放并标记；`handle_child_merge` 是两条合并路径的公共下游，**不负责**释放。MS35 已验收的 `free-page-lifecycle` 判定时点（锚点写 = 父节点分隔键移除，标记此刻发出）**未变**；`FileStorage` 的释放集合语义**未改**。
2. **`BTree::MergeInfo`（孤儿清理）**：`freed_page_id` 字段移除（随本次改动成为孤儿，公共规则「Surgical Changes：清理由本次改动产生的孤儿」）。连带 4 处构造点（`:547` / `:654` / `:727` / `:921`）与 `MergeInfo` 的文档注释。**结构性收益**：页号不再随合并结果上浮 ⇒ 重复释放在结构上不可复现。
3. **`BTree::insert_into_page`（T3 的 R3 要求）**：拆为「守卫 + 步骤」两半——`insert_into_page` 持下降路径 `descent: &mut Vec<PageId>`，路径上出现重复页号即返回 `StorageError::Internal("index descent cycle detected at page N")`；`insert_into_page_step` 是原函数体（内部节点分支的递归下降点 `:355` 改为传入 `descent`）。正常插入的下降路径页号互不相同 ⇒ 守卫不触发，既有语义与性能形态不变（守卫成本 = 每层一次 `Vec::contains`，深度为 1–3）。

**测试代码（2 个新文件，6 个用例）**

- `tests/index_page_ownership_test.rs`（5 例）：`merged_index_page_enters_the_allocatable_set_once`、`sibling_merge_releases_the_merged_page_once`、`internal_merge_releases_the_merged_page_once`、`insert_into_a_cyclic_structure_returns_an_error`、`alter_table_drop_column_completes_without_exhausting_the_stack`。
- `tests/index_page_ownership_stack_overflow_test.rs`（1 例）：`released_index_page_never_breaks_the_insert_descent`。
- 观察口径全部落在既有可观察量上，**未新增任何指标**：`storage.release_completion_limit()`（释放完成编号）、`storage.promote_free_releases(limit)`（与 `src/wal/checkpoint.rs` 第 2 步同一条入口）、`storage.allocate_page()`（可分配集合的成员就是它交给调用方的页号）、`storage.page_count()`（已分配页号的高水位）、`InternalNodeRef` 的公开读面（树形与子指针遍历）。

### T1 / T2 的先行测试 RED 证据

**RED-1（库级，真实入口）**：`cargo test --test index_page_ownership_test -- --test-threads=1 merged_index_page_enters_the_allocatable_set_once`

```
assertion `left == right` failed: the allocatable set must not hand the same page id to two owners:
[6, 9, 9, 7, 7, 5, 5, 11, 12, ...]
  left: 61
 right: 64
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 4 filtered out
```

失败落在「可分配集合中同一页号不出现两次」这条断言上：页号 **9 / 7 / 5** 各出现两次（= 3 次叶合并，每次释放两遍）。**同一页号被交给两个对象**——B-2 的直接外部表现。

**RED-2（树级，`BTree` + 真实文件库）**：`cargo test --test index_page_ownership_test -- --test-threads=1 sibling_merge_releases_the_merged_page_once`

```
assertion `left == right` failed: the allocatable set must not hand the same page id to two owners:
[4, 4, 3, 3, 1, 1, 5, 6, 7, ...]
  left: 61
 right: 64
test result: FAILED. 0 passed; 1 failed; ...
```

**RED-3（内部节点合并形态）**：`cargo test --test index_page_ownership_test -- --test-threads=1 internal_merge_releases_the_merged_page_once`

```
assertion `left == right` failed: the allocatable set must not hand the same page id to two owners:
[398, 398, 397, 397, ... 372, 372, 371, 371, 370, 370, 369, 369, 368, 368]
  left: 32
 right: 64
```

重复页号中含 **401 / 351 / 301 / 251 / 201 / 151 / 100** —— 这些是删除前的**内部节点页** ⇒ `merge_internal_nodes` 的重复释放路径成立（Plan 风险项「内部节点合并路径也被覆盖」已由本用例覆盖并被断言）。

### T1 / T2 的 GREEN 证据

```
$ cargo test --test index_page_ownership_test -- --test-threads=1
running 5 tests
test alter_table_drop_column_completes_without_exhausting_the_stack ... ok
test insert_into_a_cyclic_structure_returns_an_error ... ok
test internal_merge_releases_the_merged_page_once ... ok
test merged_index_page_enters_the_allocatable_set_once ... ok
test sibling_merge_releases_the_merged_page_once ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.60s
```

### T3 的判定结论：**ISS23 是 B-2 的下游后果**（`design.md` D3 的第一条路径）

**归属结论**：`design.md` D3 的判定协议（先修 B-2，再复现判定）执行完毕——ISS23 的栈溢出**由页归属破坏（B-2）导致**，随本 Iteration 的修复闭合。**不是**独立成因：插入路径的分裂/分隔键逻辑未被改动即可消除该症状。

**复现（修复前，真实入口）**——`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1`：

```
running 1 test
test released_index_page_never_breaks_the_insert_descent ...
thread 'tokio-rt-worker' (944075) has overflowed its stack
fatal runtime error: stack overflow, aborting
Caused by:
  process didn't exit successfully: `.../index_page_ownership_stack_overflow_test-4fbc1f25d9d207db --test-threads=1`
  (signal: 6, SIGABRT: process abort signal)
```

这与 ISS23 登记的 gdb 现场同一症状形态（`thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT）。

**定位证据（环的页号序列）**——修复前 + 下降守卫（把栈溢出换成显式错误，使环可被观测）的实测：

```
DIAG double-free page 5 (merge_leaves already freed it)
DIAG double-free page 7 (merge_leaves already freed it)
DIAG double-free page 9 (merge_leaves already freed it)
DIAG cycle at page 0 descent path = [6, 9, 0]
```

即：被重复释放的页是 **5 / 7 / 9**（叶合并的右叶页）；插入路径 **6 → 9 → 0** —— 页 9 虽已被合并掉并释放，索引的内部节点仍指向它，而它的页内容已属于另一个对象（数据页），于是被误读为内部节点，其子指针指向 **0**；页 0（目录页）同样被误读为内部节点，其子指针仍为 0 ⇒ **下降在页 0 上成环** ⇒ 每一帧都是同一 `page_id` ⇒ 栈溢出。这与登记的「连续 35+ 层同 `page_id`、同一 key 指针」现场完全一致。

**因果链**：B-2（同一页号两次进入待释放集合）⇒ 提升后可分配集合里同一页号两次 ⇒ `allocate_page` 先后把同一页交给两个对象 ⇒ 索引树仍指向该页、而该页内容已属于另一个对象 ⇒ 下降误读 ⇒ 成环 ⇒ 栈溢出 ⇒ 进程 abort。

**隔离验证（判定 B-2 而非下降守卫是根因）**——同一复现序列（N=250，删除 125 行 → checkpoint → 两轮插入）：

| 代码状态 | 结果 |
|---|---|
| 修复前（B-2 在，无下降守卫） | `has overflowed its stack` → SIGABRT（**登记症状**） |
| **只修 B-2**（无下降守卫） | 通过，625 行全部可读，**无 abort** |
| B-2 修复 + 下降守卫 | 通过，625 行全部可读，无 abort |

⇒ 症状随 B-2 的修复消失；下降守卫只把「成环」从栈溢出换成显式错误（R3 要求的「插入返回错误而非耗尽栈」），**不能**替代 B-2 的修复。

**登记现场为何不复现（如实记录）**：ISS23 登记的夹具（`ALTER TABLE DROP COLUMN` 于 N=300/600/2000）在**当前工作区修复前后均不复现**（实测：`PROBE alter resp = AffectedRows { count: 0 }`）。原因是可核对的语义变化：登记现场（HEAD `0a08b14` + MS34 改动）里 `FileStorage::free_page` **直接** `free_pages.push(page_id.0)`（`git show HEAD:src/storage/file_storage.rs` 第 490–498 行）⇒ 同一语句内即可重复分配；MS35 之后释放先进入 `pending_frees`、由 checkpoint 第 2 步才提升（`src/storage/file_storage.rs:504-515`）⇒ 语句执行期间可分配集合为空，同一语句内不再重复分配。**缺陷本身仍在**（重复释放 → 重复提升），只是后果推迟到下一次 checkpoint 之后——因此复现序列必须是「删除（触发合并释放）→ checkpoint（提升）→ 插入（分配到被释放的页）」。本 Cycle 的复现用例正是该序列，**行数 250 < 300**（ISS23 的规模约束内；T3 的 Forbidden「不得为绕开规模约束而把用例规模提到 ≥ 300 行」已遵守）。

**修复后（GREEN）**：`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` → `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`。

### AR4 / AR5 的用例与判据

| 断言 | 用例 | 判据 |
|---|---|---|
| **AR4** 合并掉的树页只释放一次、只交给一个对象 | `merged_index_page_enters_the_allocatable_set_once`（库级）、`sibling_merge_releases_the_merged_page_once`（树级）、`internal_merge_releases_the_merged_page_once`（内部节点合并形态） | ① 可分配集合中同一页号**不出现两次**（把可分配集合取空后逐个比对，`distinct == allocated.len()`）；② **每个被释放的页只带一次释放完成标记**（`release_completion_limit()` 的增量 == 被释放的**不同**页数）；③ 被释放的页**不再被树引用**（树级用例的子指针遍历 + 叶集合比对） |
| **AR5** 插入路径的下降必然终止 | `released_index_page_never_breaks_the_insert_descent`（ISS23 复现，250 行）、`insert_into_a_cyclic_structure_returns_an_error`（结构被破坏到某节点可达自身）、`alter_table_drop_column_completes_without_exhausting_the_stack`（200 行、索引树多层） | ① 成环时插入**返回错误**（`index descent cycle detected at page N`）而非耗尽栈；② 改写语句**正常返回**、进程不 abort；③ 改写前后索引树的子指针遍历**无重复引用**（无页被两个节点同时引用） |

### Gate 3 / Gate 4 逐项结论

**Gate 3（每任务即时 Review）——逐项 PASS**

| 任务 | 结论 | 依据 |
|---|---|---|
| T1 | PASS | 先行测试在修复前确实 RED（RED-1/2/3 的失败输出逐条在案），失败落在「同一页号只被释放一次 / 可分配集合中无重复页号」的断言上，未放宽断言 |
| T2 | PASS | 修复面限于「同一页号在一次删除操作中被推进待释放集合两次」；未改 `free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` 的集合语义，未改磁盘格式，未把释放点后移（`merge_leaves` 的释放点与完成标记时机原样保留） |
| T3 | PASS | 判定协议执行完毕并给出定位证据；下降守卫只作用于插入路径，未扩大到 B-Tree 之外 |

**Gate 4（Task Definition 的验证全部满足）——PASS**

| 验证命令 | 结果 |
|---|---|
| `cargo test --test index_page_ownership_test -- --test-threads=1` | `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test btree_merge_test --test btree_split_test --test btree_test --test btree_scale_test --test free_page_lifecycle_test --test drop_table_free_test -- --test-threads=1` | 6 个文件全部 `ok`：**10 / 12 / 10 / 5 / 12 / 6 passed，0 failed**（合计 55 passed / 0 failed） |
| `cargo test --lib -- --test-threads=1`（改动模块 `btree.rs` 的单元测试定向验证） | `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo build --tests` | 无 warning（`MergeInfo.freed_page_id` 移除后无 dead_code 残留） |

**验证预算**：本 Iteration **未跑全量回归**（M19 分批全量是本 change 的收尾验证，预算记在 Iteration 005，一次）。上述均为定向验证，按 `--test-threads=1` 串行执行；未使用默认并行度跑任何用例；无 timer 阈值判据；未在测试框架外建判定层。

**沿用的未失效既有结论**：MS36 收尾的全量终态 **2192 passed / 0 failed / 2 ignored**（13 批 / 136 目标，对账 missing/extra/duplicate 全 0）来自 `.agents/runbooks/m19-batched-full-regression.md`（R48）与 MS35 / MS36 的收尾记录。本 Cycle 只改 `src/storage/btree/btree.rs` 与新增 2 个测试文件，**未触及**该全量覆盖范围之外的内容（该结论的覆盖范围未变），故直接沿用、未重跑。

### 自检发现、已修复内容与遗留 Minor 问题

**已修复（在本 Cycle 的执行依据内）**

1. `handle_child_merge` 的重复释放（T2 的目标缺陷）——已移除，证据见上。
2. `MergeInfo.freed_page_id` 孤儿字段——随本次改动移除（含 4 处构造点），避免 `dead_code` warning 与「页号随合并结果上浮」这一结构性回归入口。
3. 插入路径的下降成环无终止保证（T3 的 R3 要求）——已加下降守卫，成环时返回显式错误。

**遗留 Minor（本 Cycle 未修，不阻塞）**

1. **插入路径的下降守卫只覆盖 `insert_into_page`**：`search_from_page`（`btree.rs:148`）、`search_from_page_async`（`:176`）、`delete_from_page`（`:452`）的下降**仍无环检测**。B-2 修复后，正常操作不再产生环状结构，故这三处的无界递归只在**外部损坏的文件**上可达；R3 的 requirement 只点名插入路径，本 Cycle 未扩大到它。⇒ 登记为 Issue 候选（见下）。
2. `handle_child_merge` 内的说明注释块位于「父节点分隔键移除」与后续 underflow 处理之间——它记录的是「此处**不**释放」的理由，位置正确，但读起来需要上下文；不影响语义。

**自检（对照 Cycle 的 requirements / scenarios / Task Definitions / Invariants / Non-goals）**：IPO R1 / R2 / R3 三条 requirement 均有对应用例与判据；Invariants 逐条成立（MS35 的 `free-page-lifecycle` 判定时点未变、树的键序与分裂中键规则未变、页的可分配归属唯一、不改磁盘格式、未引入身份证明机制 / 判定层 / timer 阈值判据、未回滚工作区既有未提交改动）；Non-goals 全部未触碰（DDL 改写面的数据链与可见性、释放路径的有界读、改写窗口的读一致性、释放可分配判定的耐久性前提、旧格式兼容证据、B-3、`FileStorage` 释放集合语义）。

### Deviations from Plan

1. **用例名与 RTM 计划名不同**（`tasks.md` 的命名说明允许 Act 调整命名并记录实际名）：`merged_leaf_page_is_released_exactly_once` → `merged_index_page_enters_the_allocatable_set_once` / `sibling_merge_releases_the_merged_page_once` / `internal_merge_releases_the_merged_page_once`；`allocatable_set_has_no_duplicate_page` → 同上三例的可分配集合断言；`drop_column_rewrite_does_not_exhaust_the_stack` → `alter_table_drop_column_completes_without_exhausting_the_stack` + `released_index_page_never_breaks_the_insert_descent`。断言语义未放宽（实际断言更强：增加了「每个被释放的页只带一次完成标记」的对账与「被释放的页不再被树引用」）。RTM 已同步为实际名。
2. **新增第二个测试文件** `tests/index_page_ownership_stack_overflow_test.rs`（Plan 只列了 `tests/index_page_ownership_test.rs`）：ISS23 的复现用例在缺陷未修复时以**进程 abort** 失败，与其它用例同处一个测试二进制时 abort 会带走同批其它用例的观测结果（T1/T2 的 RED 观测需要可见）。独立成文件是保持可观测性的最小改动，不涉及产品代码。
3. **产品改动面略大于 T2 的最小面**：除 `handle_child_merge` 的释放点外，还移除了因本次改动成为孤儿的 `MergeInfo.freed_page_id`（公共规则要求清理孤儿），并按 T3 的 Required behavior（「插入路径在结构被破坏时返回错误而非耗尽栈」）在 `insert_into_page` 加了下降守卫。两项均在本 Cycle 的 Task Definition 目标内（Targets 明列 `insert_into_page`）。
4. **T3 的复现序列与登记现场不同**：登记夹具（`ALTER TABLE DROP COLUMN` 于 N=300）在当前工作区修复前后均不复现（MS35 的延迟释放语义所致，见上）；复现序列为「删除 → checkpoint → 插入」，行数 250（< 300，遵守规模约束）。归属结论由**隔离验证**给出（只修 B-2 即消除症状），不依赖登记夹具。
5. **T3 的 `ALTER` 用例不判数据链**：改写路径的双尾游标（ISS22）会丢行，属 Iteration 001 的修复面；该用例改判**索引树**结构（子指针遍历无重复引用），并断言改写前索引已达多层（`leaves >= 2`、`depth >= 1`）。这与 T3 的 GREEN condition「索引结构自洽」一致，且不依赖 ISS22 的修复。

### Persisted Evidence

**None required**（模式 `none`）——本 Cycle 的全部验收都落在 `cargo test` 集合内的定向用例上；命令、实际输出、文件与符号已完整写入本 Response；定位用的临时诊断（`DIAG` 输出）与一次性探针文件已删除，未创建 `evidence/` 目录。

### Experience Candidates

1. **Issue 候选（范围外、只报告不登记）**：**索引查找 / 删除路径的下降无环检测**——`src/storage/btree/btree.rs::BTree::search_from_page`（`:148`）、`::search_from_page_async`（`:176`）、`::delete_from_page`（`:452`）的下降仍是无界递归；结构损坏时可与本 Cycle 修复前的插入路径同形态耗尽栈并 abort 进程。证据：本 Cycle 的下降路径页号序列实测（`[6, 9, 0]` 成环于页 0）与插入路径同源；R3 的 requirement 只点名插入路径，故未在本 Iteration 扩大范围。
2. **Issue 候选（范围外、只报告不登记）**：**页的「可分配」归属唯一性缺少存储层兜底**——本 Cycle 证明 B-2 这类「同一页号两次进入待释放集合」的调用点错误会让同一页号被交给两个对象，且后果被推迟到下一次 checkpoint 之后才显现（登记现场即因此在当前工作区不复现）。修复面在调用点（已做），但存储层对「同一页号两次进入待释放集合」无任何观测或断言；若将来出现第二个调用点错误，同类破坏会再次静默发生。证据：RED-1/RED-2 的重复页号序列与「MS35 延迟释放使语句内不复现」的可核对语义差异。

### 未解决问题与停止条件

- **无 Blocker**：T1–T3 全部完成，Gate 3 / Gate 4 逐项 PASS，归属结论已确定并给出定位证据，**未触发 T3 的 Stop when**（归属结论为「B-2 的下游后果」，非独立成因；复现规模 250 行 < 300，未越出规模约束）。
- **遗留**：上面两条 Issue 候选（范围外，未登记）；下降守卫只覆盖插入路径。
- 本 Cycle 未跑全量回归（收尾全量在 Iteration 005）；未修改全局状态与规格库；未归档 change；未创建下一 Cycle 或下一 Iteration；`Plan Review` 由 Plan 填写。

## Plan Review

- Review Result: accepted

**Findings**

本节由独立阅读得出（读了 `src/storage/btree/btree.rs` 的实际代码、`tests/index_page_ownership_test.rs` 与 `tests/index_page_ownership_stack_overflow_test.rs` 全文、`src/storage/file_storage.rs` 的释放集合语义、`src/wal/checkpoint.rs` 的提升时机、`src/storage/data/table_manager.rs` 的改写循环、以及 change 的 `tasks.md` / `design.md` D2–D5 / `specs/index-page-ownership/spec.md`），**不以 Act 自检代替独立检查**；未重复运行 Act 已通过的行为验证（公共规则 › 验证）。结论先行：**Acceptance 1–5 全部满足，无阻塞项**；下列 F1–F5 为非阻塞 Minor finding，F6 为对归因结论与守卫必要性的裁定，F7 为两条范围外 Issue 候选（只报告不登记）。

**F1（Act 偏离 · 非阻塞 Minor）RED 证据的断言文案与最终代码不一致。** Act Response 的 RED-1/2/3 输出为 `the allocatable set must not hand the same page id to two owners: [...]` + `left: 61 / right: 64`，而最终代码（`tests/index_page_ownership_test.rs:140-147`）的文案是 `... ({} allocations, {} distinct, repeated page ids: {repeated:?})`。Act Response 的「Deviations from Plan」第 1 条已说明断言在 RED 观察后被**加强**（补入「每个被释放的页只带一次释放完成标记」的对账）。核对：加强后的断言在修复前**仍然 RED**——修复前每次合并发 2 次 `complete_free_release`（`file_storage.rs:559` 每次调用都 `fetch_add`）⇒ 标记增量 = 2N，而被释放的不同页数 = N ⇒ `markers_after - markers_before == released.len()` 不成立。**断言语义未放宽，最终断言比 RED 时更强**，不阻塞。

**F2（Plan 遗漏 · 非阻塞 Minor）成环用例只断言 `result.is_err()`，未钉住错误的身份。** `tests/index_page_ownership_test.rs:486-490` 断言插入返回错误，但未断言它是 `StorageError::Internal("index descent cycle detected at page N")`。判别力核对：**仍然有判别力**——去掉守卫后同一条用例会无界递归直至栈溢出并 abort 进程（该二进制崩溃 ⇒ 用例失败），故不可能因错误原因「通过」；而 `specs/index-page-ownership/spec.md` 的 R3 场景只要求「插入 SHALL 返回错误 / SHALL NOT 栈溢出」，`is_err()` 与规格一致。规格未要求错误文案 ⇒ 非阻塞。

**F3（Plan 遗漏 · 非阻塞 Minor）「被释放的页不再被树引用」的断言只比对叶集合。** `tests/index_page_ownership_test.rs:335-340` 把被释放页与 `leaves_after` 比对，未比对内部节点集合。对叶合并用例这是**正确**的集合（被释放的页就是叶）；内部节点合并形态由 `internal_merge_releases_the_merged_page_once`（断言被释放页中含删除前的内部节点页）与 `walk_index_tree` 的「同一页号被两个节点引用」检查（`:66-105`、`:436-439`、`:452-455`）另行覆盖。覆盖完整 ⇒ 非阻塞。

**F4（Act 偏离 · 非阻塞 Minor）新增测试文件未 rustfmt 合规。** `rustfmt --check --edition 2021 tests/index_page_ownership_test.rs` 报出差异（import 排序 `use rtsql::storage::btree::{InternalNodeRef, BTree, SyncPageLoader};`、`:217` 处 `fn collect_tree(...) {    let mut leaves = Vec::new();` 的签名与首行挤在一行、以及若干处换行）。对照：`src/storage/btree/btree.rs`（本 Cycle 的产品改动）与 `tests/index_page_ownership_stack_overflow_test.rs` **均合规**；仓库 `tests/*.rs` 137 个文件中 2 个非合规。纯观感问题，无语义影响，`cargo build --tests` 干净 ⇒ 非阻塞；建议后续触及该文件时一并 `rustfmt`。

**F5（Act 偏离 · 非阻塞 Minor）Act Response 的两处行号引用有偏差。** 「实际改动」第 1 条写 `merge_internal_nodes` 在 `:877` 释放 / `:889` 标记，实际为 `src/storage/btree/btree.rs:907` 释放 / `:919` 标记（`:877` 是 `redistribute_internal_right` 的 `update_parent_separator`，`:889` 是 `get_parent_separator_key`）；第 2 条写 4 处 `MergeInfo` 构造点为 `:547`/`:654`/`:727`/`:921`，实际为 `:549`/`:654`/`:727`/`:921`。**代码本身正确**，仅文案偏差 ⇒ 非阻塞。

**F6（新证据 · 裁定）归因结论与下降守卫的必要性与正确性——独立复核通过。**

- **(a) B-2 修复后「成环」是否确实不可能再出现**：就本 Cycle 覆盖的成因而言成立。核对 `src/storage/file_storage.rs`：`:514` `free_page` → `pending_frees.push((page_id.0, None))`；`:555-566` `complete_free_release` 每次调用 `release_id.fetch_add(1)` 一次，并把**所有**未完成且页号匹配的待释放条目标记完成；`:570-581` `promote_free_releases(limit)` 对每个完成条目 `free.push(page)`（**1:1 映射**）⇒ 只要同一页号进入待释放集合两次，可分配集合里就有两个该页号，`allocate_page`（`:449` 附近 `free_pages.pop()`）可先后把同一页交给两个对象。修复后 `merge_leaves`（`btree.rs:640` 释放 / `:652` 标记）与 `merge_internal_nodes`（`:907` 释放 / `:919` 标记）各自**只释放一次**，`handle_child_merge`（`:684-738`）已不再释放 ⇒ **该路径下同一页号不再两次进入待释放集合**，页归属唯一性成立。`MergeInfo.freed_page_id` 的移除使这一保证成为**结构性**的：页号不再随合并结果上浮，重复释放在结构上不可复现；全仓 grep 确认无其它消费者（`src/storage/btree/node.rs:23/43` 的 `LeafMergeResult` / `InternalMergeResult` 是**另一组未被调用的**节点层 API，`merge_right` 无任何调用点，与 `btree.rs::MergeInfo` 无耦合）。**取舍裁定**：Act 自评「将来若生产者忘记释放会变成泄漏而非双重释放」——**可接受**：泄漏是安静但可被后续断言（页数增长 / 完整性扫描）发现的有界失效，双重释放是静默的结构损坏；且「生产者忘记释放」在本结构下会同时缺少释放完成标记，与 AR4 的「标记数 == 被释放页数」对账相互独立地暴露。
- **(b) 守卫是否必要且正确、会不会掩盖 B-2 的复发**：**必要**——`specs/index-page-ownership/spec.md` R3 明文要求「下降再次回到已访问过的页时，插入 SHALL 失败并返回错误，SHALL NOT 耗尽栈」，守卫是该 requirement 的实现而非范围外扩张；且页归属唯一性只在**合并路径**上被证明闭合，外部损坏或将来新的释放调用点错误仍可能产生环状结构，此时守卫是「把进程 abort 换成显式错误」的唯一手段。**不会掩盖 B-2 的复发**——守卫只**检测**环（返回错误），既不阻止页被交给两个对象，也不影响 AR4 的两条断言（可分配集合无重复页号、标记数与被释放页数对账），二者互不依赖；B-2 若复发，AR4 仍会 RED，且 `released_index_page_never_breaks_the_insert_descent` 会再次 abort。**判定完备**——`descent.contains(&page_id)` 在**进入**该页前检查，覆盖自环（根指向自身）、互环与任意长度的环（路径上出现环必然重复访问某页）。**不误判合法路径**——合法 B-Tree 中每页唯一父节点，根到叶路径上的页号互不相同，守卫在合法结构上永不触发；实证上 `cargo test --lib` 420 用例与 6 个既有树相关测试文件（55 用例）全绿亦未观察到误触发。**触发后不留不一致状态**——守卫在**任何写入之前**触发（下降先于叶写入），错误经 `?` 逐层上抛 ⇒ 父层的分隔键插入（`btree.rs:362-370`）不会执行，本次插入未发生半途。**错误上下文**：`StorageError::Internal("index descent cycle detected at page N")` 点名成环闭合处的页号，足以定位；未携带完整下降路径（Act 的一次性诊断打印过 `[6, 9, 0]`，该探针已删除）——非阻塞的可改进项，不违反 R3。
- **(c) 隔离实验是否真的把两个变更分开**：Act 报告三态对照（修复前 → SIGABRT；**只修 B-2** → 通过且 625 行完好；B-2 + 守卫 → 通过）。独立佐证：Act 的定位诊断是在 **B-2 仍在**的状态下取得的——`DIAG double-free page 5/7/9 (merge_leaves already freed it)` 与 `DIAG cycle at page 0 descent path = [6, 9, 0]`，其中**被重复释放的页 9 正是下降路径重新进入的页** ⇒ 成环由双重释放直接产生；而守卫在因果上对「页被交给两个对象」是**惰性的**（它只能把已成形的环换成错误），因此它不可能是症状消失的原因。再叠加 RED-1/2/3：若只加守卫而不修 B-2，AR4 的断言仍会 RED（守卫不改变释放路径）⇒ **B-2 的修复是必要且充分的，守卫不是替代品**。裁定：**归属结论「ISS23 是 B-2 的下游后果」成立**。

**F7（范围外实质缺陷 · Issue 候选 · 不阻塞）** Act 报告的两条候选，本 Review 复核后确认成立、且确实在本 Iteration 的执行依据之外，**按公共规则只报告不登记**（登记与承载由 Recorder 按用户指令执行）：

1. **索引查找 / 删除路径的下降无环检测**——`btree.rs` 的 `search_from_page` / `search_from_page_async` / `delete_from_page` 仍是无界递归。B-2 修复后正常操作不再产生环状结构，该面只在**外部损坏的文件**上可达；R3 的 requirement 只点名插入路径。
2. **存储层对「同一页号两次进入待释放集合」无兜底观测**——`file_storage.rs` 无任何断言或计数能把该调用点错误当场暴露；本 Cycle 的后果被推迟到下一次 checkpoint 之后才显现（登记现场因此在当前工作区不复现，见下）。**注**：`design.md` D2 已明确否决「让 `free_page` 幂等」，故该候选**不是**幂等化，而是「可观测/断言」，与 D2 的否决不冲突。

**登记夹具不复现的独立复核（关系到 ISS23 能否关闭）**：`git show HEAD:src/storage/file_storage.rs` 的 `:490-498` 确认为 `free_pages.lock().unwrap().push(page_id.0)`（**直接**进可分配集合）；当前工作区 `:504-516` 为 `pending_frees.push(...)`，提升只发生在 `src/wal/checkpoint.rs:186-189` / `:207-212` 的第 2 步。⇒ **登记夹具（`ALTER TABLE DROP COLUMN` 于 N=300/600/2000）在当前工作区修复前后均不复现，原因是 MS35 的延迟释放使「同一语句内重复分配」不再可能，而非缺陷已消失**；缺陷仍在，只是**推迟到下一次 checkpoint 之后**（同一页号两条待释放条目 → 两条都被标记完成 → 都被提升 → 可分配集合里出现两次）。Act 的复现序列「删除 → checkpoint → 插入」（250 行）与该推理一致。⇒ **ISS23 的缺陷机制已被本 Iteration 闭合**（页不再被交给两个对象 + 下降必然终止），可由 Recorder 据本节结论关闭；登记夹具本身不再是可复现的验收面。

**Acceptance Gaps**

None——Acceptance 1–5 全部满足（逐条对照见 Evidence）。

**Evidence**

独立阅读（本次 Review 完成）：

- `src/storage/btree/btree.rs`：`:38-41` `MergeInfo`（已无 `freed_page_id`）；`:210-239` `insert`（每次插入新建 `descent`）；`:247-264` `insert_into_page`（守卫：重复页号 ⇒ `StorageError::Internal`；push → step → pop，无提前返回）；`:267+` `insert_into_page_step`（原函数体，`:358` 递归下降传入 `descent`）；`:613-658` `merge_leaves`（`:640` 释放、`:652` 完成标记、`:654` 构造 `MergeInfo`）；`:684-738` `handle_child_merge`（**无释放**，只移除父节点分隔键 + 处理 underflow / 根收缩）；`:882-925` `merge_internal_nodes`（`:907` 释放、`:919` 完成标记、`:921` 构造 `MergeInfo`）。
- `src/storage/file_storage.rs`：`:504-516` `free_page` → 待释放集合；`:555-566` `complete_free_release`（`fetch_add` 每次调用一次 + 标记全部匹配条目）；`:570-581` `promote_free_releases`（逐条目 1:1 提升）；`:530-532` `page_count`（页号高水位）。
- `src/wal/checkpoint.rs`：`:186-189` / `:207-212` —— 提升只发生在 checkpoint 第 2 步。
- `tests/index_page_ownership_test.rs`（全文 494 行）与 `tests/index_page_ownership_stack_overflow_test.rs`（全文 85 行）：断言均落在既有可观察量上（`release_completion_limit()` / `promote_free_releases()` / `allocate_page()` / `page_count()` / `InternalNodeRef` 公开读面 / 真实 `Database::open` + DML + `checkpoint`）；**未新增指标、无框架外判定脚本、无 timer 阈值判据、无身份证明机制**。
- 全仓 grep：`freed_page_id` 仅存于 `src/storage/btree/node.rs`（未被调用的 `merge_right` API，与 `btree.rs::MergeInfo` 无耦合）；`MergeInfo` 无 `btree.rs` 之外的消费者。
- `rustfmt --check --edition 2021`：`src/storage/btree/btree.rs` 与 `tests/index_page_ownership_stack_overflow_test.rs` 合规；`tests/index_page_ownership_test.rs` 不合规（F4）；`src/storage/data/table_manager.rs` 有既有偏差（**非本 Cycle 改动**，未触碰）。

沿用 Act 的未失效验证结论（来源：Act Response 的 Gate 4 表，本 Review 未重复运行）：

- `cargo test --test index_page_ownership_test -- --test-threads=1` → 5 passed / 0 failed；`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` → 1 passed / 0 failed；6 个既有树相关测试文件 → 55 passed / 0 failed；`cargo test --lib` → 420 passed / 0 failed；`cargo build --tests` 干净。覆盖范围未被本次 Review 的改动影响（本次 Review 未改任何产品代码或测试代码）⇒ 结论沿用有效。

**Follow-up Decision**

**接受**：Iteration 000 的 Acceptance 1–5 全部满足，实现与 `design.md` D2 / D3 的决策一致，未越界、未放宽断言、未引入判定层或身份证明机制；F1–F5 均为非阻塞 Minor（文案、观感、覆盖面已由其它用例补齐），不需要返工；F7 的两条候选属范围外实质缺陷，按公共规则报告为 Issue 候选，**不在本 Iteration 内扩大范围**。

**当前 Cycle 无需修复**：不创建 rework / replan Cycle；Iteration 000 冻结。

**后续承载建议（交 Recorder / change 所有者，本 Review 不代为执行）**：F7 的两条候选建议由 Recorder 登记为 Issue，并作为**本 change 的 Iteration 006**（索引下降路径的有界终止 + 存储层页归属兜底观测）承载——理由：与本 change 刚改动的模块同层、复用本 change 已有的收尾全量预算、且不与已验收的 `free-page-lifecycle` 语义冲突（候选 2 是**可观测/断言**而非 D2 已否决的 `free_page` 幂等化）。承载前需补 `design.md` 的决策段与对应规格增量（新增 requirement）——那是 change 级范围变更，不属本 Cycle 的执行依据。

**Iteration Plan Update**

不变——Iteration 000 的任务分配、依赖、验证边界与诊断边界保持原样（`tasks.md` 的 Iteration Plan 000 段）。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/001-ddl-rewrite-consistency/000-initial.md`（Iteration 001 / Cycle 000，状态 `ready`，承载 T4–T7：ISS22 + ISS24）
