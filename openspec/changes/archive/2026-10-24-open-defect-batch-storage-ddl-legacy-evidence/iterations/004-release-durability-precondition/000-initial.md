# Iteration 004 / Cycle 000: 释放可分配判定的耐久性前提

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T12, T13
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/specs/release-durability-precondition/spec.md` 的 RDP R1 / R2（本 Cycle 只闭合这两条 requirement 的行为语义）；MS35 已验收并继续有效的释放生命周期语义（`free_page` 进 `pending_frees`、`complete_free_release` 记完成号、`promote_free_releases` / `discard_free_releases` 决定可分配与否）；MS36 已验收的旧格式兼容面（本 Cycle 不改磁盘格式）；Iteration 002 已验收的释放路径有界读（本 Cycle 不动 `free_page` 集合语义）。
- Excluded scope: checkpoint 九步的顺序与取样时点（MS35 已验收）；MS35 已验收的释放完成编号取样时点；`DROP TABLE` 的崩溃边界模型（既有域 `drop-persistence-boundary`）；DDL 语句边界落盘的顺序语义（B-3，见 `design.md` D9）；真实断电模型；Iteration 003 遗留的「提交点窗口持久化不一致」（已作为 Issue 候选上报，不在本 Cycle）；同表并发 DDL 的串行化（新设计决定）。

**Objective**

让「释放被提升为可分配」这一判定**以锚点写已持久为准**：任何一次刷盘失败都不得使已发出的写永久丢失（该页保持脏，后续成功刷盘必定重写它），且未持久化的释放不得被 checkpoint 提前提升为可分配。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `0a08b14`（ahead of origin 9），MS34 + issue-fix + MS35 + MS36 + 本 change 的 Iteration 000–003 改动**全部未提交**；本 Cycle 在该现场上继续，不回滚、不清理、不提交既有改动。
  - 前序 Iteration 的验证结论（沿用，覆盖范围未变化）：`ddl_rewrite_consistency_test` 9/0、`ddl_catalog_transactional_test` 3/0、`ddl_crash_recovery_test` 6/0、`cargo test --lib` 420/0、`cargo build --tests` 0 警告（来源：Iteration 003 的 Act Response；Iteration 003 的 Plan Review 另独立复跑了 `ddl_rewrite_consistency_test`，结果 9/0）。全量回归按 change 预算归最后一个 Iteration（T21），本 Cycle 不跑全量。
  - 已知限制（Iteration 003 Plan Review 记录的范围外实质缺陷，**不在本 Cycle**）：「提交完成 → 目录应用」窗口内崩溃或语句被放弃后，重开仍按位置错位（关闭它需要持久标记或移动提交点 ⇒ 改磁盘格式 / 与 ISS21 / D5 冲突）。本 Cycle **不得**用持久标记或移动提交点来修 B-1。
- Current-State Evidence（Plan 已独立读码确认的行位；Act 不需回读即可执行）:
  - **脏标记在写盘之前就被清除**：`src/storage/buffer_pool.rs` 的三个刷盘函数都在「phase 1 收集脏页」时先 `frame_guard.dirty = false`，再在 phase 2 逐页 `self.storage.write_page(page_id, &page).await?`：
    - `flush_all`：`src/storage/buffer_pool.rs:216`（清脏 `:222`，写 `:229`）
    - `flush_pages`：`src/storage/buffer_pool.rs:239`（清脏 `:245`，写 `:255`）
    - `flush_all_except`：`src/storage/buffer_pool.rs:264`（清脏 `:276`，写 `:283`）
    - phase 2 的 `?` 在**第一页写失败时立即返回**：失败页及其**之后尚未尝试**的页都已被清脏 ⇒ 这些页的内存内容此后**永远不会被重写**（后续刷盘因「不脏」而跳过它们）。
    - 只读探针 `BufferPool::has_dirty_pages()`（`src/storage/buffer_pool.rs:297`）是既有公开可观测量。
  - **释放的可分配闸门（MS35 形态，本 Cycle 的修复落点）**：
    - `src/storage/async_storage.rs` 的 4 个默认实现方法（无释放生命周期的存储形态零行为变化）：`release_completion_limit`（`:50`，默认 0）、`complete_free_release`（`:65`，默认空实现）、`promote_free_releases`（`:72`，默认空实现）、`discard_free_releases`（`:80`，默认空实现）。
    - `src/storage/file_storage.rs` 的实现：`free_page`（`:504`，把页推进 `pending_frees`）、`release_completion_limit`（`:548`，取当前完成号）、`complete_free_release`（`:555`，记完成号）、`promote_free_releases`（`:570`，把完成号 < limit 的待释放页移入可分配集合）、`discard_free_releases`（`:586`，丢弃完成号 < limit 的待释放页）。
    - `src/storage/buffer_pool.rs:504` 是 `complete_free_release` 到存储的转发点。
    - `src/wal/checkpoint.rs` 的取样 / 提升 / 丢弃：零工作早返回路径在 `:179` 判 `!has_dirty_pages()`，`:186` 取样 `release_completion_limit()`，`:189` `promote_free_releases(limit)`，`:190` 返回；第 2 步在 `:207` 取样，`:208` `flush_all()`，成功则 `:212` 提升、失败则 `:217` 丢弃并 `:218` 返回错误。
  - **锚点写与「完成号」的时序**：`src/storage/data/table_manager.rs:1669-1672` 的注释写明「MS35（`free-page-lifecycle` R3）：释放完成标记——本次释放的锚点写（目录删除）已经发出。标记之前，这些页不得可分配」⇒ **完成号是在锚点写「已发出（页变脏）」时记录的，不是在它「已持久」时记录的**；锚点写就是 `drop_table` 里的 `self.catalog.delete_table(name).await?`（`src/storage/data/table_manager.rs:1604`，目录删除）。**注**：该注释括号里的行号指针 `:1414` 已过时（当前 `:1414` 是一处 `TableMeta` 构造，`:1604` 才是目录删除调用）——属过期注释指针，Act 若已改动该处注释可顺带更正，不单独构成任务。
  - **`DROP TABLE` 的锚点写不自带落盘**：`Catalog::delete_table`（`src/storage/catalog.rs:279`）只删目录行（`__tables` 链 `:281-287`、`__columns` 链 `:288-294`），**不调用** `flush_commit_point` ⇒ 目录删除这一锚点写**留在脏页里**，由**之后某一次**刷盘带出去。而 `complete_free_release` 已在 `drop_table` 内被调用（`src/storage/data/table_manager.rs:1672`）⇒ 该释放此刻已是「待提升」候选。
  - **DDL 边界的落盘在 checkpoint 之外**：`Catalog::flush_commit_point`（`src/storage/catalog.rs:269`）= `flush_all_except(&exclude)` + `flush_pages(commit_pages)`；调用点：`insert_table`（`src/storage/catalog.rs:259`，即 `CREATE TABLE`）、`update_table_tail`（`src/storage/catalog.rs:442`）、`register_secondary_index`（`src/storage/catalog.rs:615`）。⇒ **「刷盘失败发生在 checkpoint 之外」是可达的**。
  - **B-1 的完整因果链（Plan 独立推出，与 `design.md` D8 一致；并与既有域 `free-page-lifecycle` R1 的要求同形）**：
    1. `DROP TABLE t` ⇒ 目录删除（锚点写）发出、页变脏；`t` 的数据 / 索引页进 `pending_frees` 并被 `complete_free_release` 记为完成号 X（此时锚点写**未持久**）。
    2. 随后的某次**非 checkpoint** 刷盘（例：`CREATE TABLE u` 的 `insert_table` → `flush_commit_point` → `flush_all_except`；或直接调用 `db.buffer_pool.flush_all()`）因写盘失败而返回错误 ⇒ **当前实现**：该批页（含承载锚点写的目录页）已被清脏且未写出 ⇒ 锚点写**永久丢失**（磁盘上的目录仍注册着 `t`），而该页在内存里被标为「不脏」。
    3. 之后一次**成功的** checkpoint：`:207` 取样（limit ≥ X）→ `:208` `flush_all()` 成功（该目录页此刻「不脏」⇒ 被跳过 ⇒ 锚点写仍未落盘）→ `:212` `promote_free_releases(limit)` ⇒ **完成号 X 的释放被提升** ⇒ `t` 的页变成可分配。
    4. 结果：磁盘目录仍注册 `t`，而 `t` 的页已被发放给新对象 ⇒ 重开后静默损坏（两个对象共用同一页）。
    5. **与既有需求的关系**：既有域 `free-page-lifecycle` R1（MS35 已验收）要求「该释放的**锚点写**已被一次**在释放完成之后开始**且**成功完成**的 checkpoint 刷脏页写入数据文件」才成为可分配页；本缺陷正是**该条件被绕过**（锚点写从未落盘却仍被提升）⇒ B-1 不是新需求，而是既有已验收要求的**实现缺口**；RDP R2 把这条前提显式写成增量规格。
  - **测试入口与注入点**：`Database::arm_io_failure(IoFailPoint::DataWrite)`（`src/database.rs:896`）是一次性写盘失败注入（`src/storage/file_storage.rs:436` 在 `write_page` 里于任何字节落盘前消费；注入不产生半页、不触碰文件）；`IoFailPoint` 枚举在 `src/storage/file_storage.rs:88`；`Database::checkpoint()`、`Database::open`、`db.buffer_pool`（含 `has_dirty_pages()`）为既有公开入口。既有可参照的测试：`tests/free_page_lifecycle_test.rs`（13 条，含 `flush_failure_does_not_promote_pending_release`——它覆盖的是**失败发生在 checkpoint 内**的变体，MS35 已验收）、`tests/drop_persistence_test.rs`、`tests/checkpoint_test.rs`。**本 Cycle 的 RED 用例文件 `tests/release_durability_precondition_test.rs` 尚不存在**（由 T12 新建）。
- Code and Critical Path:
  - 入口：`Database::execute_sql` → `pipeline::execute` → `DropTableExecutor` / `CreateTableExecutor` / `AlterTableExecutor` → `TableManager::drop_table`（`src/storage/data/table_manager.rs`）→ `Catalog::delete_table`（目录锚点写）+ `FileStorage::free_page`（`pending_frees`）+ `BufferPool::complete_free_release`（完成号 X）。
  - 落盘链：`Catalog::flush_commit_point`（`src/storage/catalog.rs:269`）→ `BufferPool::flush_all_except` / `flush_pages`（`src/storage/buffer_pool.rs:264` / `:239`）→ `FileStorage::write_page`（`src/storage/file_storage.rs:429`）。
  - 提升链：`CheckpointManager::checkpoint`（`src/wal/checkpoint.rs`）→ `release_completion_limit`（取样）→ `flush_all` → `promote_free_releases` / `discard_free_releases`（`src/storage/file_storage.rs:570` / `:586`）→ `free_pages`（可分配集合）。
  - 状态变化：页的脏标记（缓冲池内）→ 磁盘内容；`pending_frees` → 完成号 → 可分配集合。外部影响：可分配集合决定后续 `allocate_page` 发放哪一页 ⇒ 提前提升 = 把仍被目录引用的页发给新对象 = 静默损坏。

**Implementation Guidance**

- 建议顺序：先 T12（两条 RED 用例，观察真实 RED），再 T13（最小实现），最后按 Verification 逐条跑。
- 修复必须落在**两个既有机制**上，不要另建旁路：(1) 三个刷盘函数的**脏标记生命周期**；(2) MS35 的**释放持久化闸门**（`src/storage/async_storage.rs` 的 4 个默认实现方法 + `src/wal/checkpoint.rs` 的取样 / 提升 / 丢弃）。
- 脏标记的生命周期要按「**本次刷盘未能全部落盘**」整体处理，不是只处理报错的那一页：`flush_all` / `flush_all_except` / `flush_pages` 的 phase 2 是逐页 `?` 提前返回，**失败页之后尚未尝试的页同样已被清脏** ⇒ 只恢复失败页会留下同样的丢失洞。恢复后，下一次成功刷盘必定重写这些页（它们仍是脏的）。
- 「已持久」的判定：释放只有在「其锚点写所在页已不脏（其写已被一次在释放完成之后开始的成功刷盘写出）」时才可被提升；否则该释放**不被提升**（继续留在待释放集合，等下一次 checkpoint 重判）。这与 `design.md` D8 的决策 2 一致。
- **提升判定必须能按「每个待释放项」回答「它的锚点写已持久了吗」**：闸门当前的 `complete_free_release(&[PageId])` 只携带**被释放的页**，不带**锚点写所在页** ⇒ 要做逐项判定，完成标记必须同时携带锚点写所在页（按既有闸门形态扩展，给出默认实现，保持无释放生命周期的存储形态零行为变化）。既有域 `free-page-lifecycle` R1 已把这条写成要求：「该释放的锚点写已被一次**在释放完成之后开始**且**成功完成**的 checkpoint 刷脏页写入数据文件」。
- **「尚未持久」与「已丢弃」必须分开**：因锚点写尚未持久而**未提升**的释放应当**继续留在待释放集合**（等下一次 checkpoint 重判），不得被当作「丢弃」而永久消失——否则 RDP R2 的「锚点写已落盘时释放被提升」将永远无法满足。既有 `discard_free_releases` 的语义（checkpoint 失败时丢弃该批）**不变**。
- 判定锚点写是否已持久，可以在 checkpoint 的**取样点**做（此时「该页仍为脏」即「其写尚未被任何刷盘写出」）；若在 checkpoint 自己的刷盘之后再判定，会让「该页仍为脏 ⇒ 不提升」这条前提失去可观察的落点。两种取法都允许，但必须让 RDP R2 的两个场景都能被真实构造与观察。
- 闸门能力的扩展按既有形态提供**默认实现**（无释放生命周期的存储形态零行为变化），不要把新语义塞进 `free_page` 的集合语义里，也不要新增旁路的状态通道。
- 取样时点不变：完成号仍在 checkpoint 第 2 步刷盘**之前**取（`src/wal/checkpoint.rs:207`）；零工作早返回路径（`:186` / `:189`）的语义不变（该路径已以 `!has_dirty_pages()` 为前置）。

**Task Definitions**

### T12: B-1 的先行 RED 用例（写盘失败保持脏标记 + 未持久化的释放不被提前提升）

- Requirement/Scenario: `specs/release-durability-precondition/spec.md` 的 RDP R1（写盘失败不丢弃脏页写）与 RDP R2（释放的可分配判定以锚点写已持久为准）；映射到 change 的 `tasks.md` RTM 行「RDP R1 刷盘失败不丢弃脏页写 / RDP R2 释放的可分配判定以锚点写已持久为准 → `tests/release_durability_precondition_test.rs::failed_write_keeps_page_dirty` / `::release_not_promoted_when_anchor_write_not_durable`」。
- Depends on: None
- Targets: `tests/release_durability_precondition_test.rs`（新建）
- Current behavior: 写盘失败后该批页在缓冲池里被标为「不脏」⇒ 其内容此后永不再写；一个「完成号已记、锚点写已丢」的释放会被之后一次成功的 checkpoint 提升为可分配。
- Required behavior: 两条用例各自**真实构造**下列时序并观察结果——
  - R1（对应 RDP R1 的两个场景）：给定一个已被修改（脏）的数据页，且下一次同型写盘被受控失败命中 ⇒ 刷盘返回错误 ⇒ **该页仍为脏**；之后一次成功刷盘 ⇒ 该页内容被写入存储、该页此后不再为脏 ⇒ 重开库后读到最新内容。
  - R2（对应 RDP R2 的「锚点写未落盘时释放不被提升」场景）：给定一个已完成的释放（完成号已记、锚点写已发出），其锚点写所在页的写盘失败过一次（该页此后仍为脏）⇒ 执行一次**不覆盖该页**的成功刷盘并走完 checkpoint ⇒ **该释放不被提升为可分配**；且重开该库时该页**不被判为可分配页**。
- Preserve: 真实文件库 + 真实 `Database::open` / `execute_sql` / `checkpoint` / `db.buffer_pool.flush_all()` / `db.buffer_pool.flush_pages()` 入口 + 既有一次性注入点 `Database::arm_io_failure(IoFailPoint::DataWrite)`；断言只落在用户可见结果（页是否仍脏、内容是否落盘、页是否被发放、重开后目录与数据是否一致）。
- Forbidden: 不用替身 / mock 存储绕过真实落盘路径；不新增判定脚本、封装命令、专用参数或身份指纹（不得引入 hash / revision pin / run-id / 时间顺序证明）；不得为 RED 用例放宽既有断言或修改既有测试的期望值；不得改磁盘格式；不得改 `free_page` 的集合语义。
- First test: 新建 `tests/release_durability_precondition_test.rs`，含 `failed_write_keeps_page_dirty` 与 `release_not_promoted_when_anchor_write_not_durable` 两条用例。构造路径（Plan 已核对可达，全部用既有公开入口）：
  - **R1**：建表 `t` + 插入行使其数据页变脏（此时该数据页是池中唯一的脏页）→ `db.arm_io_failure(IoFailPoint::DataWrite)` → `db.buffer_pool.flush_all()` 返回错误（错误文本含 `injected io failure (write_page)`）→ 观察 `db.buffer_pool.has_dirty_pages()` **仍为真**（该数据页仍是脏的）→ 再次 `db.buffer_pool.flush_all()`（注入已消费）成功 → `db.close()` 后重开库 → 读到该行（内容已落盘）。
  - **R2**：建表 `t` 并插入行 → 记录 `t` 的数据页与索引树根页（`db.get_table("t")` 的 `data_page_head` 与 `index_manager.root_page_id()`，与既有 `tests/free_page_lifecycle_test.rs::flush_failure_does_not_promote_pending_release` 的取法一致）→ `DROP TABLE t`（目录删除这一锚点写发出且未落盘；释放完成号已记）→ `db.arm_io_failure(IoFailPoint::DataWrite)` → `db.buffer_pool.flush_all()` 返回错误（承载锚点写的目录页被清脏且未写出）→ 再执行一次**不覆盖该目录页**的成功刷盘（`db.buffer_pool.flush_pages(&[某个其它页])`）→ `db.checkpoint()` 成功走完 → 观察该释放**未被提升**：`CREATE TABLE u` 之后 `u` 的数据页与索引树根页**都不得**等于 `t` 的（可分配集合未提前拿到 `t` 的页）→ `db.close()` 后重开库：`t` 的数据页与索引树根页**不得**进入可分配页集合（新建的表拿不到它们），且目录与数据面一致。
  - 观察 RED：`cargo test --test release_durability_precondition_test -- --test-threads=1` ⇒ 两条用例**必须真的失败**，且失败原因落在「页不再脏」（R1）/「释放被提前提升、`u` 拿到了 `t` 的页」（R2）这一形态上——不是断言写错、不是环境问题、不是报错形态不符。
- GREEN condition: 两条用例在本 Cycle 结束时通过（修复在 T13 落地后）。
- Verification: `cargo test --test release_durability_precondition_test -- --test-threads=1` ⇒ `2 passed; 0 failed`。失败含义：R1 失败 ⇒ 脏标记未恢复或恢复不完整；R2 失败 ⇒ 提升判定仍可在锚点写未持久时发生。
- Stop when: RED 的失败原因与上述形态不符（说明构造路径不成立或既有行为另有解释）⇒ 返回 Plan；或需要新增持久化状态 / 改磁盘格式才能让 R2 变绿 ⇒ 返回 Plan（该方向已由 D8 与本 Cycle 的 Excluded scope 排除）。

### T13: 刷盘失败保持脏标记 + 释放提升以锚点写已持久为准

- Requirement/Scenario: 同 T12（RDP R1 / RDP R2）；并保 RDP 的既有语义（释放生命周期与 `free_page` 集合语义不变）。
- Depends on: T12
- Targets: `src/storage/buffer_pool.rs::flush_all`（`:216`）、`src/storage/buffer_pool.rs::flush_pages`（`:239`）、`src/storage/buffer_pool.rs::flush_all_except`（`:264`）、`src/wal/checkpoint.rs`（取样 / 提升 / 丢弃段 `:186` / `:189` / `:207` / `:212` / `:217`）、`src/storage/async_storage.rs`（4 个默认实现方法 `:50` / `:65` / `:72` / `:80`）、`src/storage/file_storage.rs`（`release_completion_limit` `:548` / `complete_free_release` `:555` / `promote_free_releases` `:570` / `discard_free_releases` `:586`）。
- Current behavior: 刷盘失败后该批页被永久标为「不脏」（内容永不再写）；checkpoint 在刷盘成功后按完成号提升释放，不检查锚点写是否已持久 ⇒ 未持久化的释放会被提前提升为可分配。
- Required behavior: ① 一次刷盘未能把本次清脏的页全部写出时，这些页**全部恢复脏标记**（含失败页与其后未尝试的页）⇒ 下一次成功刷盘必定重写它们；② 释放被提升前必须确认其**锚点写已持久**（锚点写所在页已不脏、且其写已被一次「在释放完成之后开始」的成功刷盘写出）；未确认时该释放**不被提升**，且**继续留在待释放集合**等下一次 checkpoint 重判（不得被当作「丢弃」而永久消失）；⇒ 未持久化的释放不会变成可分配。③ 提升判定按**每个待释放项**进行（闸门须能回答「该释放的锚点写所在页是否仍为脏」），扩展按既有闸门形态给出默认实现。
- Preserve: MS35 已验收的释放完成编号语义与取样时点（完成号仍在刷盘前取）；checkpoint 九步的顺序；`free_page` 的集合语义（`pending_frees` → 完成号 → 可分配集合的形状与归属不变）；`has_dirty_pages()` 的只读语义；无释放生命周期的存储形态零行为变化（新闸门能力按默认实现给出）；既有 `promote_free_releases` / `discard_free_releases` 的对外语义（提升 / 丢弃）。
- Forbidden: 不改磁盘格式（保 MS36 的 G1 / G2 / G3 兼容面）；不改 `free_page` 的集合语义；不改 checkpoint 九步的顺序与取样时点；不引入新的持久化状态或旁路状态通道；不改 DDL 语句边界落盘的顺序语义（B-3）；不改 `drop-persistence-boundary` 的崩溃模型；不新增身份证明机制、框架外判定层或以 timer 阈值作行为判据。
- First test: 沿用 T12 的两条用例（此时为 RED）。
- GREEN condition: T12 的两条用例通过，且 `cargo test --test free_page_lifecycle_test -- --test-threads=1` 13/0、`cargo test --test drop_persistence_test -- --test-threads=1`、`cargo test --test checkpoint_test -- --test-threads=1` 全绿（既有 MS35 行为零破坏）。
- Verification: 见「Verification」段；任一既有测试由绿转红 ⇒ 修复越界（回到 Preserve / Forbidden 重新定位）。
- Stop when: 让 R2 变绿必须依赖持久化状态、磁盘格式变更或移动提交点 ⇒ 返回 Plan；或修复必须改动 `free_page` 集合语义 ⇒ 返回 Plan。

**Invariants**

- 磁盘格式不变（MS36 已验收的旧格式兼容面）；`pending_frees` / 完成号 / 可分配集合三者的形状与归属不变。
- checkpoint 九步的顺序、MS35 已验收的释放完成编号取样时点不变。
- 任何已发出的页写，在一次失败的刷盘之后**不会**永久丢失：失败后该页仍是脏的，后续成功刷盘必定重写它。
- 「可分配」的含义不变：一个页只有在不再被任何目录 / 树结构引用时才可被发放；提升判定只是该含义的实现。
- 无释放生命周期的存储形态（`AsyncStorage` 的 4 个默认实现方法）零行为变化。

**Non-goals**

- checkpoint 九步的顺序、零工作早返回路径的语义、取样时点（MS35 已验收）。
- MS35 已验收的释放完成编号语义。
- `DROP TABLE` 的崩溃边界模型（既有域 `drop-persistence-boundary`）。
- DDL 语句边界落盘的**顺序语义**（B-3，见 `design.md` D9）——本 Cycle 只保证「失败的刷盘不丢写」，不保证「失败的 DDL 语句不留目录面变更」在后续刷盘中如何显现（见 Risks）。
- 真实断电模型（本 Cycle 用既有一次性注入点表达写盘失败，不构造断电）。
- Iteration 003 遗留的「提交点窗口持久化不一致」（Issue 候选，另行承载）。
- 同表并发 DDL 的串行化。

**Acceptance**

1. **RDP R1（刷盘失败不丢弃脏页写）**——给定一个已被修改（脏）的数据页且下一次同型写盘被受控失败命中：刷盘返回错误，**该页仍为脏**；之后一次成功刷盘把该页内容写入存储、该页此后不再为脏；重开库后读到最新内容。（task: T12, T13；spec: `specs/release-durability-precondition/spec.md` RDP R1 的三个场景「数据页写失败后该页仍为脏」/「失败后的脏页在后续成功刷盘时被重写」/「DDL 边界落盘失败不丢弃目录写」；code: `src/storage/buffer_pool.rs` 三个刷盘函数；test: `tests/release_durability_precondition_test.rs::failed_write_keeps_page_dirty`；design: D8 决策 1）
2. **RDP R2（释放的可分配判定以锚点写已持久为准）**——给定一个已完成的释放，其锚点写所在页的写盘失败过一次（该页此后仍为脏）：执行一次**不覆盖该页**的成功刷盘并走完 checkpoint，该释放**不被提升为可分配**，且重开该库时该页**不被判为可分配页**；反向地，锚点写已被成功刷盘落盘的释放仍**会被提升**（既有 `free-page-lifecycle` R1 的时机语义不变），且提升后的页在重开库时与持久目录一致。（task: T12, T13；spec: RDP R2 的三个场景；code: `src/wal/checkpoint.rs` 取样 / 提升 / 丢弃段 + `src/storage/async_storage.rs` / `src/storage/file_storage.rs` 的闸门实现；test: `tests/release_durability_precondition_test.rs::release_not_promoted_when_anchor_write_not_durable`（新增）+ 既有 `tests/free_page_lifecycle_test.rs::released_page_allocatable_after_release_durable` / `::reopened_database_reuses_released_pages` / `::rebuild_never_marks_live_pages_free`（保持绿）；design: D8 决策 2）
3. **既有行为零破坏**——MS35 的释放生命周期用例（`tests/free_page_lifecycle_test.rs` 13 条，含 `flush_failure_does_not_promote_pending_release`、`released_page_not_allocatable_before_release_durable`、`released_page_allocatable_after_release_durable`）、`tests/drop_persistence_test.rs`、`tests/checkpoint_test.rs` 全部保持绿；`cargo test --lib` 保持 420/0。（task: T13；Invariant：MS35 已验收语义）
4. **实现边界**——修复只落在两个既有机制上（刷盘函数的脏标记生命周期 + MS35 的释放持久化闸门）；无持久化状态新增、无磁盘格式变更、无 `free_page` 集合语义变更、无身份证明机制、无框架外判定层、无 timer 阈值判据。（task: T13；Forbidden 段）

**Verification**

- RDP R1：`cargo test --test release_durability_precondition_test -- --test-threads=1` ⇒ `failed_write_keeps_page_dirty` 通过（行为类 scenario：真实文件库 + 真实写盘失败注入 + 真实重开读取 ⇒ 走真实入口）。
- RDP R2：同一命令 ⇒ `release_not_promoted_when_anchor_write_not_durable` 通过（真实 `DROP TABLE` → 真实的失败刷盘 → 一次不覆盖该页的成功刷盘 → 真实 checkpoint → 新表是否拿到被释放页 + 重开后的可分配页集合观察）。
- RDP R1 / R2 的 RED 形态：T12 阶段同一命令 ⇒ 两条用例**真的失败**，失败原因落在「页不再脏」（R1）/「释放被提前提升、新表拿到了被释放的页」（R2）上。
- 既有行为零破坏：`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 13 passed / 0 failed；`cargo test --test drop_persistence_test -- --test-threads=1` ⇒ 全绿；`cargo test --test checkpoint_test -- --test-threads=1` ⇒ 全绿；`cargo test --lib` ⇒ 420 passed / 0 failed。
- 编译面：`cargo build --tests` ⇒ 0 warning。
- 全量回归：**本 Cycle 不跑**（按 `tasks.md`「收尾验证：M19 分批全量回归」记在最后一个 Iteration，T21）。若 Act 认为改动触及面超出预期而需要追加定向回归，按公共规则「验证」追加并在本 Cycle 的 Act Response 记录理由。

**Gate 2 Readiness**

- 需求与 BDD：PASS —— RDP R1 / R2 的 requirement 与场景取自 `specs/release-durability-precondition/spec.md`（change 已有的增量规格）；T12 的两条用例覆盖 RDP R1 的三个场景与 RDP R2 的「锚点写未落盘时释放不被提升」场景，RDP R2 的另两个场景由既有 MS35 用例覆盖（列入 GREEN 条件）；无场景缺口。
- 调查：PASS —— 根因链（清脏早于写 + 失败即提前返回 + 完成号在锚点写「已发出」时记录 + 目录删除不自带落盘 + DDL 边界落盘在 checkpoint 之外）已由 Plan 独立读码定位到具体行位；可达路径已核对（`DROP TABLE` → 覆盖锚点写页的失败刷盘 → 不覆盖该页的成功刷盘 → 后续 checkpoint）。无实质未知项。
- 设计：PASS —— 修复方向已由 `design.md` D8 决策 1 / 2 确定，并已与 MS35 的释放持久化闸门及既有域 `free-page-lifecycle` R1 对齐；本 Cycle 未引入新设计决定。
- 任务与追踪：PASS —— T12 / T13 已定义（Targets / Current / Required / Preserve / Forbidden / First test / GREEN / Verification / Stop when 齐备）；RTM 的两行指向 `tests/release_durability_precondition_test.rs` 的两条用例。
- 当前轮验证：PASS —— RED 用例尚未存在（T12 的职责即建立并观察 RED）；既有基线结论（`--lib` 420/0 等）沿用 Iteration 003 的 Act Response，覆盖范围未变化。
- Persisted Evidence：none —— Act Response 足以承载验证结论；T12 / T13 的验证结果可低成本重跑（单测试文件 + 三个既有测试文件 + `--lib`），无需持久化。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- **条件性风险（已知、不阻塞，须在 Act Response 中如实记录）**：恢复脏标记意味着**失败的刷盘不再丢弃写**，因此一个**失败的 DDL 语句**已发出的目录面变更，可能在之后某次成功刷盘时落地（当前实现下它被「丢弃」纯属清脏早于写的副作用）。这与既有域 `ddl-catalog-transactional` 的「失败的 DDL 语句不留目录面变更」在**字面**上存在张力。该张力落在本 Cycle 的 Excluded scope（DDL 语句边界落盘的顺序语义 = B-3，`design.md` D9）内：**本 Cycle 不裁决该语义**，但修复后该形态变得可达 ⇒ 作为范围外 Issue 候选上报（由 Recorder 登记 / 另找承载），不在本 Cycle 扩大处理。
- **实现陷阱（须在 T13 落实）**：三个刷盘函数的 phase 2 以 `?` 提前返回 ⇒ **失败页之后尚未尝试的页**同样已被清脏；只恢复报错的那一页会留下同样的丢失洞。恢复范围必须覆盖「本次清脏但未成功写出的全部页」。
- **非实质未知项**：具体采用哪种「锚点写已持久」的可观察判据（例如按页判定还是按批判定）属 Act 可在既定约束内决定的实现细节，只要最终满足 Acceptance 2 的可观察结果即可；若该选择会改变接口或错误语义，才需返回 Plan。
- **跳过项**：无。T12 / T13 均在本 Cycle 执行。

## Act Response

- Status: reported

### 任务状态

| Task | 状态 | 涉及文件与符号 |
|---|---|---|
| T12 | done | `tests/release_durability_precondition_test.rs`（新建）：`failed_write_keeps_page_dirty`、`release_not_promoted_when_anchor_write_not_durable`；辅助 `db_path` / `ok` / `fails` / `rows` |
| T13 | done | `src/storage/buffer_pool.rs`：`restore_dirty_from`、`flush_all`、`flush_pages`、`flush_all_except`；`src/storage/file_storage.rs`：`record_write_failure`、`promote_free_releases`、`last_failed_write` 字段 |

### 前次在途工作的独立核对结论（逐点）

前次派发留下的是 **Fix 1 的实现 + 一个未验证的测试文件**；Fix 2 完全未落地。逐点核对结果：

1. **`restore_dirty_from(&self, collected: &[(PageId, Page)], from: usize)` 的语义**——把 `collected[from..]` 的每一页重新置脏。核对三点：
   - **失败页及其后所有未写页都被归还**：`flush_all` / `flush_all_except` 以 `written`（已成功写出的页数）为起点调用 ⇒ 失败页（索引 `written`）与其后**尚未尝试**的页全部归还 ⇒ Plan Context 的「实现陷阱」已被覆盖（只恢复报错的那一页会留下同样的丢失洞）。
   - **并发写者重新置脏的页仍为脏**：恢复是幂等地置 `dirty = true`；若并发写者已改写该页，其内容此后仍需写出，置脏是正确状态。
   - **已被驱逐出池的页**：`if let Some(entry) = self.pages.get(page_id)` 跳过，不制造幽灵条目。
2. **三个刷盘入口的覆盖**——`flush_all`（`:216`）、`flush_pages`（`:239`）、`flush_all_except`（`:264`）**三处均已完整落地**（前次汇报中「第三处只显示了片段」的疑虑不成立，diff 与当前文件均确认完整）。`flush_pages` 的 phase 1/2 是**逐页交错**的，失败时只有当页被清脏而未写出 ⇒ 只恢复当页即为完整恢复（此前的页已成功写出，其后的页从未被清脏）。
3. **「清脏早于写」这一根因是否被完整封堵**——已封堵：三处 phase 2 都不再把「清脏」与「写出」解耦，任何未成功写出的页都保持脏 ⇒ 已发出的页写不会永久丢失 ⇒ 下一次成功刷盘必定重写它们。
4. **我补齐的部分**——Fix 1 **无需补齐**（前次实现正确）；**Fix 2 全新实现**（见下）。

### T12 的先行测试 RED 证据（实际运行）

前次在途工作**已改产品代码**，RED 无法直接复现 ⇒ 按「临时还原 → 观察 RED → 恢复」的方式独立复核。还原方式：`src/storage/buffer_pool.rs` 的三处 phase 2 临时改回 `?` 直接传播、`src/storage/file_storage.rs` 的提升闸门临时改回无条件提升（写盘失败登记一并临时移除）。

- **变体 A（Fix 1 与 Fix 2 同时还原）**——`cargo test --test release_durability_precondition_test -- --test-threads=1` ⇒ `0 passed; 2 failed`：
  - `failed_write_keeps_page_dirty` panicked at `tests/release_durability_precondition_test.rs:88`：`a failed flush must leave the unwritten pages dirty: their content would otherwise be lost forever` ⇒ 失败原因落在**预期的断言**上（R1：页不再脏）。
  - `release_not_promoted_when_anchor_write_not_durable` panicked at `:147`：`a failed flush must leave the unwritten pages dirty`。
- **变体 B（仅还原 Fix 2，Fix 1 保留）**——同一命令 ⇒ `1 passed; 1 failed`：`release_not_promoted_when_anchor_write_not_durable` panicked at `:165`：`assertion left != right failed: the new table must not take the released data page 2: the release must not be promoted while its anchor write is not durable`（`left: 2, right: 2`）⇒ **失败形态正是「释放被提前提升、新表取走了被释放的页」**，与 Plan Context 对 R2 的 RED 描述一致。
- **恢复**：两个产品文件由还原前的副本逐字恢复；最终工作区 = 修复态，已重新验证（见下）。RED 观测期间未提交、未回滚任何其它既有改动。

### GREEN 证据与验证（实际命令与结果，最终工作区）

| 命令 | 结果 |
|---|---|
| `cargo test --test release_durability_precondition_test -- --test-threads=1` | `2 passed; 0 failed` |
| `cargo test --test free_page_lifecycle_test -- --test-threads=1` | `12 passed; 0 failed` |
| `cargo test --test drop_persistence_test -- --test-threads=1` | `11 passed; 0 failed` |
| `cargo test --test checkpoint_test -- --test-threads=1` | `2 passed; 0 failed` |
| `cargo test --test release_path_bounded_read_test -- --test-threads=1` | `7 passed; 0 failed` |
| `cargo test --test ddl_catalog_transactional_test -- --test-threads=1` | `3 passed; 0 failed` |
| `cargo test --test ddl_crash_recovery_test -- --test-threads=1` | `6 passed; 0 failed` |
| `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` | `9 passed; 0 failed` |
| `cargo test --test index_page_ownership_test -- --test-threads=1` | `5 passed; 0 failed` |
| `cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` | `1 passed; 0 failed` |
| `cargo test --lib` | `420 passed; 0 failed` |
| `cargo build --tests` | 0 warning |

- **`--lib` 420 基线**：本 Cycle 改到 `buffer_pool.rs`（MS35/MS36 的全量终态 2192/0/2 同样已失效）⇒ **基线重跑**，420/0 **仍成立**。
- **沿用与重跑的划分**：Iteration 000–003 的用例结果中，`release_path_bounded_read_test` / `ddl_catalog_transactional_test` / `ddl_crash_recovery_test` / `ddl_rewrite_consistency_test` / `index_page_ownership_test` / `index_page_ownership_stack_overflow_test` 的覆盖**触及本次改动的刷盘与释放路径** ⇒ 已重跑（结果同上）；`free_page_lifecycle_test` / `drop_persistence_test` / `checkpoint_test` 为 Plan Context 指定的既有行为零破坏判据，同样重跑。
- **全量回归**：本 Cycle **不跑**（按 change 预算归最后一个 Iteration 007 / T21，M19 分批串行）。本 Cycle 只跑定向单目标验证（每次一个 `--test <file>`，批内 `--test-threads=1`），未出现无目标过滤的全量调用。

### 四个根因要素的处置与「提前提升」不变量

| 根因要素 | 处置 |
|---|---|
| `buffer_pool.rs:216/239/264` 三处**清脏早于写** | 已修：三处 phase 2 改为「写失败即归还本次清脏但未写出的全部页」的脏标记（`restore_dirty_from`）⇒ 失败页与其后未尝试的页都保持脏 ⇒ 下一次成功刷盘必定重写 ⇒ **已发出的页写不会永久丢失** |
| `table_manager.rs:1669-1672` 完成号在锚点写「已发出」时记录 | **不改时点**（MS35 已验收的释放完成编号语义与取样时点保留）。改为在**提升判定**上补前置条件：提升按**每个待释放项**判定——该释放完成之后若发生过数据页写盘失败 ⇒ 覆盖它的那次刷盘未能把锚点写落盘 ⇒ 锚点写的持久化前提无法确认 ⇒ **该释放不被提升**，且**继续留在待释放集合**等下一次 checkpoint 重判（**不是丢弃**，否则「锚点写已落盘时释放被提升」永不成立） |
| `catalog.rs:279` 目录删除不自带落盘 | **不改**（属 DDL 语句边界落盘的顺序语义 = B-3 / `design.md` D9，本 Cycle Excluded scope）。其后果（锚点写只由之后某次刷盘带出）由上两行共同封堵 |
| `catalog.rs:269` DDL 边界的落盘在 checkpoint 之外 | **不改**（同上）。它使「刷盘失败发生在 checkpoint 之外」可达——正是 B-1 的触发条件；由上两行封堵 |

**「提前提升」不变量如何被保证**：提升前必须能回答「覆盖该释放的那次刷盘把锚点写落盘了吗」。实现上，写盘失败会从**释放完成编号同一计数器**取一个编号，于是「某次写失败发生在该释放完成之后」可用编号比较回答（失败编号 > 该释放的完成编号）；提升判定逐项比较该编号，命中即不提升。配合「清脏早于写」的修复：失败的刷盘不再丢弃写 ⇒ 锚点写不会永久丢失 ⇒ 后续一次成功刷盘必定把它写出 ⇒ 该释放在后续 checkpoint 可被确认并提升。**⇒ 未持久化的释放不会进入可分配集合，「提前提升」被封堵**；R2 用例真实观察到「释放未被提升」（新表拿不到被释放的页）。

**闸门形态**：判定落在**既有的释放闸门**内（`FileStorage::promote_free_releases` 与既有的 `release_id` 计数器）——**无新增 trait 方法、无新增持久化状态、无磁盘格式变更、`free_page` 集合语义未动、无释放生命周期的存储形态零行为变化**（新语义完全位于 `FileStorage` 的既有方法内）。

### 候选 3（失败的 DDL 目录面变更可能在后续刷盘落地）的实际观察结果

Plan Context 的 Risks 要求如实观察。构造（真实入口）：`ALTER TABLE keep ADD COLUMN m INT` —— 该 DDL 的目录面变更（列行 + 表行列数）在**语句边界落盘之前**发出，且该语句在落盘之前**不分配新页** ⇒ 一次性注入命中语句边界的落盘 ⇒ 语句返回错误 ⇒ 一次成功刷盘 ⇒ `close()` ⇒ 重开。

| 状态 | 观察结果 |
|---|---|
| **修复后**（本次实现） | 语句返回错误 `injected io failure (write_page)`；随后一次成功刷盘 + 重开后，`SELECT id, n, m FROM keep` ⇒ 行 `[1, 10, Null]` ⇒ **失败的 DDL 语句的目录面变更落地了** ⇒ 候选 3 的形态在本 Cycle 修复后**确实可达** |
| **修复前**（临时还原 Fix 1 的对照运行） | 同样失败，但重开后 `SELECT id, n, m FROM keep` ⇒ 错误 `Column not found: m` ⇒ 目录面变更被**丢弃**（清脏早于写的副作用）；且该状态自身**不一致**（表行的列数已含新列，而列行已丢失） |

结论：该形态落在 **B-3 / DDL 语句边界落盘的顺序语义**（`design.md` D9；既有域 `ddl-catalog-transactional` 的「失败的 DDL 语句不留目录面变更」）内，属本 Cycle 的 Excluded scope ⇒ **本 Cycle 不裁决、不修**，作为范围外 Issue 候选上报。

### Gate 3（任务级 Review）

- **T12**：spec compliance —— 两条用例各自真实构造 Plan Context 指定的时序（真实文件库 + 真实 `Database::open` / `execute_sql` / `checkpoint` / `buffer_pool.flush_all()` / `buffer_pool.flush_pages()` + 既有一次性注入点），断言只落在用户可见结果（页是否仍脏、内容是否落盘、页是否被发放、重开后目录与数据面是否一致）✓；无替身 / mock、无判定脚本、无身份指纹、未改磁盘格式、未改 `free_page` 集合语义 ✓。code quality —— diff 仅限计划文件；`cargo build --tests` 0 warning；无用例因错误原因通过的路径（RED 已独立复核，见上）✓。
- **T13**：spec compliance —— ① 脏标记生命周期覆盖**三个**刷盘入口与「本次清脏但未写出的全部页」✓；② 提升前置条件按**每个待释放项**判定，未提升者**留在待释放集合**（不丢弃）✓；③ 闸门按既有形态扩展，无新增 trait 方法 / 持久化状态 ✓。Preserve —— checkpoint 九步顺序、释放完成编号语义与取样时点、`free_page` 集合语义、`has_dirty_pages()` 只读语义、无释放生命周期形态的零行为变化，**均未变** ✓。Forbidden —— 未改磁盘格式 / checkpoint 顺序 / `free_page` 集合语义 / DDL 边界顺序语义 / 崩溃模型；未引入身份证明机制、框架外判定层、timer 阈值判据 ✓。
- **Gate 3 发现**：Critical 0、Important 0；Minor 2（见下）。

### Gate 4（Verified Completion）

- Task Definition 的 Verification 全部满足且基于**本次真实运行**：`release_durability_precondition_test` 2/0；既有行为零破坏三项全绿（`free_page_lifecycle_test` 12/0、`drop_persistence_test` 11/0、`checkpoint_test` 2/0）；`cargo test --lib` 420/0；`cargo build --tests` 0 warning。
- 本 Cycle 未跑全量回归（按 change 预算归 Iteration 007 / T21），与 Plan Context 的 Verification 段一致 ⇒ 不构成 Gate 4 缺口。
- **完成声明有当次运行的真实输出** ✓；**未沿用任何已失效的结论**（`--lib` 与 MS35/MS36 全量终态均已重跑 / 明确标注失效）。

### 自检发现、已修复内容与遗留 Minor

**已修复（本次实施中自行发现并修复）**

1. 在途测试文件的 R1 重开失败于 `DatabaseLocked`：旧 `Database` 句柄在重开时仍存活（`let db = …` 遮蔽但未 `drop`）⇒ 重开前必须释放句柄。
2. 在途测试文件的 R2 同样失败于 `DatabaseLocked`：测试把 `TableMeta`（持有 `Arc<IndexManager>` ⇒ 连带持有存储句柄）跨重开存活 ⇒ 改为只取页号并把 meta 限定在作用域内。
3. R2 的重开断言原按「新表恰好拿到被释放的数据页」书写，而实际分配顺序不同（树根与数据页从被释放集合中取用的次序相反）⇒ 改为断言「新表的两个页都来自被释放集合」。
4. 在途测试文件的辅助函数 `err` 被同名局部绑定遮蔽 ⇒ 改名 `fails`。
5. 本次改动文件的 `cargo fmt` 差异（一个多余空行、一处 `assert_ne!` 排版）已修正；**仓库其余既有的 fmt 差异未触碰**（不属本 Cycle 范围）。

**遗留 Minor（已记录，未在本 Cycle 处理）**

1. **闸门的失败粒度是进程级的**：任何一次数据页写盘失败都会阻止「在该释放完成之后」的释放被提升，而非只针对其锚点写所在页。按页判定需要把锚点写所在页带进完成标记（Plan Context 提到的扩展方向）并让目录删除回报其改动的页——改动面更大，而对所要求的场景无可观察差异。**建议留作后续精化，不在本 Cycle 扩大范围。**
2. **闸门偏保守**：某释放的锚点写所在页在其完成后发生过写失败时，该释放在本会话内**始终留在待释放集合**（其页在会话内不被复用；重开后按持久目录仍可复用）。这是 Plan Context 明确要求的「不被提升但不得被丢弃」的取舍。
3. Plan Context 记 `tests/free_page_lifecycle_test.rs` 为 13 条，**实际为 12 条**（全部通过）。

### Deviations from Plan

1. **Fix 2 的判据形态（实质性，已用实测定形）**：Plan Context 把前置条件表述为「锚点写所在页**已不脏**（其写已被一次在释放完成之后开始的成功刷盘写出）」，并允许在取样点或在 checkpoint 自己的刷盘之后判定。两种取法都实测过：
   - **取样点判定**（该页仍脏 ⇒ 不提升）：**破坏 MS35 已验收的 2 条用例**——`released_page_allocatable_after_release_durable` 与 `same_session_release_reuses_pages_after_release_durable`（二者都是已验收形态「DROP → checkpoint ⇒ 被释放的页在同一会话内被复用」，即 `free-page-lifecycle` R1 的时机语义）。证据：临时把 `promote_free_releases` 改为不提升任何释放 ⇒ `10 passed; 2 failed`，失败的正是这两条。
   - **checkpoint 刷盘之后判定**：判据**落空**（一次成功的 `flush_all` 之后不存在脏页）⇒ RDP R2 的「不被提升」永不可观测。证据：只保留 Fix 1 时，R2 用例的新表取走了被释放的页 2（提前提升）。
   - ⇒ **Acceptance 2 的可观察面与 Acceptance 3 的「`released_page_allocatable_after_release_durable` 保持绿」在纯脏标记判据下互斥**（两种流式在取样点的状态完全相同，唯一差别是那次失败的写）。实现的闸门把提升前置条件对**锚点写所在页的失败写**敏感——即 RDP R2 场景 1 的 GIVEN 原文「其锚点写所在页的写盘失败过一次」所陈述的语义——从而**同时满足**两条 Acceptance 的可观察面。闸门形态仍限于既有机制（见上）。
2. **RDP R2 场景 1 的重开半句**：「AND 重开该库时该页 SHALL NOT 被判为可分配页」在本实现下**不可满足**——R2 流程中锚点写已随 checkpoint 的刷盘落盘（RDP R1 要求失败页被后续成功刷盘重写），故持久目录不再登记被删对象，其页在重开后按目录重建为可分配页是**一致**的。⇒ 该半句与 RDP R1、且与 MS35 R1 的已验收时机语义不能同时成立。本 Cycle 的重开观察改为**一致性**不变量（被删表已不在目录中；新表复用的页来自被释放集合；完整性扫描 error 级发现数为 0），并把该半句的文本问题**上报 Plan Review**（属本 change 的增量规格文本，非范围外缺陷）。
3. 完成标记**未携带锚点写所在页**（Plan Context 的扩展方向）：见遗留 Minor 1——按页判定需要额外接线，本 Cycle 未做。
4. Plan Context 的 `free_page_lifecycle_test.rs` 条数 13 → 实际 12。

### Persisted Evidence

`None required` —— 全部结论可低成本重跑（单个新测试文件 + 9 个既有测试文件 + `cargo test --lib` + `cargo build --tests`），Act Response 已承载命令与结果摘录；未创建 `evidence/` 目录。

### change 状态同步

- `tasks.md`：**T12 → done**、**T13 → done**；RTM 的 **RDP R1 → Done**、**RDP R2 → Covered**（并注明场景 1 重开半句的偏差与本 Act Response 的指针）；Iteration Map 的 004 行与「Next Iteration」段同步为「已实施待审（`Act Response` 状态 `reported`，`Review Result` 待 Plan Review）」。
- 未改 `openspec/specs/`、未改全局 tasks / SNAPSHOT / 项目记忆、未归档 change、未创建后继 Cycle 或 Iteration、未填 `Plan Review`。

### Experience Candidates

1. **Issue 候选**——「失败的 DDL 语句其目录面变更在后续刷盘落地」（Plan Context 的候选 3）：修复后可达已被实测坐实（`ALTER TABLE … ADD COLUMN` 的目录面变更在语句失败后仍随下一次成功刷盘落盘，重开后可见）；与既有域 `ddl-catalog-transactional` 的「失败的 DDL 语句不留目录面变更」在字面上存在张力。**证据**：本 Act Response 的「候选 3 的实际观察结果」段（含修复前后的对照运行）。落在 B-3 / DDL 语句边界落盘的顺序语义内，`design.md` D9 已明确该 change 不承载 ⇒ **范围外，由 Recorder 登记或另找承载**；本 Cycle 只报告不登记。
2. **Runbook 候选**——B-1 / RDP R2 的复现配方：`DROP TABLE` → 锚点写已发出 → 注入一次写盘失败 → 一次不覆盖锚点页的成功刷盘 → checkpoint ⇒ 观察「释放未被提升（新表取不到被释放的页）」；以及「临时还原修复以复核 RED」的还原/恢复步骤。**证据**：本 Act Response 的 RED 与 GREEN 两段（命令与失败输出均在）。命中「已端到端验证成功且可重复」。

### 未解决问题与停止条件

- **未解决（需 Plan Review 裁决）**：RDP R2 增量规格文本中场景 1 的重开半句与其余两条（本次已实现的提升判定、RDP R1）不能同时成立——实现给出的是「释放不被提升 + 重开后目录与可分配集合一致」，文本要求的是「重开后该页不被判为可分配页」。请 Plan 在 Review 中裁决该文本（收紧或改写场景 1）。
- **未解决（非阻塞）**：闸门的进程级失败粒度（遗留 Minor 1）——是否按锚点写所在页精化，留作后续决定。
- **停止条件**：本 Cycle 的授权范围（T12 / T13）已完成且已验证——Act 写入反馈后**终止**；下一步由 `openspec-plan` 对本 Cycle 做 Review（`Review Result` 仍为 `pending`）。全量回归仍归 Iteration 007 / T21（按 M19 分批串行纪律执行），本 Cycle 未跑。

## Plan Review

- Review Result: accepted

**Findings**

按公共规则「实施反馈 Review」分类：先判是否阻塞本 Iteration 的既有 Acceptance，再判偏差原因（Plan 遗漏 / Plan 错误 / Act 偏离 / 现状变化 / 新证据）。下列核对均为**独立读码**（完整 diff 全文 + 相关既有代码与既有域），不以 Act Self-Review 代替。

1. **完整 diff 的独立核对 —— Act 的三处声明逐条成立（非阻塞）**。本次 Review 独立阅读 `git diff HEAD` 的三个改动面（`src/storage/buffer_pool.rs`、`src/storage/file_storage.rs`、`tests/release_durability_precondition_test.rs` 全文 234 行）与 change 的 `tasks.md` / `design.md`（D8）/ `proposal.md` / 规格增量，并对照既有域 `ddl-catalog-transactional`、`drop-persistence-boundary` 与 MS35 待合并的 `free-page-lifecycle`。
   - **Fix 1 的三个入口覆盖完整**：`flush_all`（`buffer_pool.rs:216`；phase 1 清脏 `:222`、phase 2 写 `:229`；失败于 `:239` 调 `restore_dirty_from(&to_write, written)`）、`flush_all_except`（`:297`；失败于 `:323` 同样自 `written` 起归还）、`flush_pages`（`:265`；**逐页交替** phase 1/2，失败于 `:286` 只归还失败的那一页）。⇒ `flush_pages` 不存在「已清脏但尚未尝试」的页 ⇒ 只归还失败页即完整；Act 对「第三处只显示片段」的疑虑**不成立**。
   - **Fix 2 落在既有闸门内**：`FileStorage::promote_free_releases`（`file_storage.rs:607`）+ 既有 `release_id` 计数器；**无新 trait 方法**（`AsyncStorage` 的四个默认实现未动）、**无持久状态**（`last_failed_write` 为 `AtomicU64` 内存字段 `:69`/`:220`/`:280`）、**无磁盘格式改动**、**`free_page` 集合语义未动**、`discard_free_releases`（`:632`）语义未动 ⇒ 无释放生命周期的存储形态零行为变化。
2. **Deviation (a)（取样点与 MS35 的冲突）——裁定：Act 的诊断成立，Plan 的机制表述被实测排除**。独立复核（不依赖 Act 的实验结论）：
   - **事实 A（取样点上锚点页必为脏）**：`Catalog::delete_table`（`catalog.rs:279`）只删目录行、**不调 `flush_commit_point`** ⇒ `DROP` 的目录删除（锚点写）留在脏页里；`complete_free_release` 随即在 `table_manager.rs:1672` 记完成号 ⇒ 随后 checkpoint 的**取样点**上锚点页必为脏。
   - **事实 B（MS35 已验收的提升发生在同一取样点）**：`tests/free_page_lifecycle_test.rs` 的 `released_page_allocatable_after_release_durable`（DROP → checkpoint ⇒ 被释放的页在同一会话内被复用）与 `same_session_release_reuses_pages_after_release_durable`（drop_table → create u → checkpoint → create v ⇒ v 复用被释放的页）都在 checkpoint 第 2 步取样后被提升 ⇒ 任何「取样点上锚点页仍脏 ⇒ 不提升」的判据都会拦掉 MS35 已验收的提升（Act 的临时「什么都不提升」实验给出 `10 passed; 2 failed`，失败的正是这两条；本次 Review 的读码复核与该实验结论一致）。
   - **事实 C（刷盘后取样判据落空）**：一次成功的 `flush_all` 之后不存在脏页 ⇒ 「锚点写所在页仍脏 ⇒ 不提升」在提升点上恒为假 ⇒ RDP R2 的「不被提升」永不可观测。
   - ⇒ **两条流式在取样点上的状态完全相同，唯一可区分的信号是失败历史**。Act 以「该释放完成之后发生过页写盘失败」为提升前置条件，是唯一能同时满足两条验收面的形态，且未改 checkpoint 九步、释放完成编号与取样时点。**偏差分类：Plan 错误（Plan 的机制表述不可实现），Act 的替代机制正确。** 代价（收紧 MS35 R1 的场景「释放持久后页可分配」在失败情形下不成立）已写入 `design.md` D8 与增量规格 R2 Requirement。
3. **Deviation (b)（RDP R2 场景 1 的重开半句）——裁定：该半句不可满足，且不是实现缺陷；已就地改写规格文本**。独立推导：RDP R1 要求失败页保持脏 ⇒ 后续一次成功刷盘**必定重写**它 ⇒ R2 流程中 checkpoint 自己的 `flush_all` 会把锚点页写出 ⇒ **锚点写在该 checkpoint 完成时已持久** ⇒ 持久目录不再登记被删对象 ⇒ 重开时按目录重建（既有 `free-page-lifecycle` R3 与 MS35 的「重开后释放的页可再次分配」）必然把这些页判为可分配页 ⇒ 「重开该库时该页 SHALL NOT 被判为可分配页」与 RDP R1、与 MS35 R1/R3 的已验收语义**不能同时成立**。⇒ **不返工代码**；已就地改写 `specs/release-durability-precondition/spec.md`（改写措辞见下文第 9 点与「Follow-up Decision」）。
4. **提升闸门的正确性 —— 成立（保守方向，无误放行）**。
   - **编号空间**：完成号取 `release_id.fetch_add(1)` 的**旧值**（`file_storage.rs:583`），失败号取 `fetch_add(1) + 1`（`:431`）⇒ 两者共用一个单调计数器，按时间严格递增（唯一例外：失败紧邻完成时两者可相等，此时失败**在先** ⇒ `last_failure > completion` 为假 ⇒ 放行，判定正确）⇒ `last_failed_write > completion_id` **确实等价于**「最近一次写盘失败发生在该释放完成之后」。
   - **与锚点写所在页的关联**：实现**不建立**页级关联——闸门是**进程级**的：任何一次页写盘失败（不限于锚点写所在页）都会拦住**该次失败之前完成**的全部待提升释放。⇒「锚点写所在页写失败但其它页成功」不会误判为可提升（保守地不提升）；「锚点写所在页此前已脏、后来被别的刷盘成功清掉」时，若其释放完成之后**没有**任何写盘失败，闸门**正确放行**。
   - **「提升 ⇒ 锚点写已持久」成立**：放行的必要条件是「该释放完成之后没有发生写盘失败」⇒ 其锚点写（完成号记录时发出）在此后的每一次写盘都成功 ⇒ 提升点（checkpoint 成功刷盘之后）锚点写已落盘 ⇒ RDP R2 的 Requirement 成立。
   - **收紧的方向与代价**：闸门只会**不提升**，不会在锚点写未持久时提升（安全方向）。代价：一次写盘失败之后，那些在失败之前完成、尚未提升的释放在本会话内**不再被提升**（`last_failed_write` 不复位 ⇒ 后续 checkpoint 的重判结论不变）⇒ 与 Iteration 004 诊断边界所列「永远不提升」方向一致，**非阻塞**（无损坏；重启后按持久目录重建即恢复）。
5. **`restore_dirty_from` 的并发正确性 —— 成立（附一条非阻塞残余窗口）**。
   - **并发写者重新置脏**：归还只做 `frame_guard.dirty = true`（`buffer_pool.rs:256`）⇒ 并发写者在归还期间重新置脏的页**仍为脏**（置位幂等）；不存在把干净页误标为脏的路径（归还只作用于「本次刷盘清脏过」的页集合）。
   - **被驱逐的页**：归还用 `self.pages.get(page_id)`（`:254`）⇒ 已被驱逐出池的页**跳过**，不产生幽灵条目（正确）。**残余窗口（本次 Review 新发现）**：驱逐路径 `evict_one` 读到的脏标记若恰被并发刷盘的 phase 1 清掉，则该页**不经写盘**即被移出池；随后那次刷盘的写盘若失败，该页内容**既未落盘也不在池内**（修复前该损失在**任何**一次刷盘失败时发生，修复后只剩「并发驱逐 + 写盘失败」这一窄窗口）⇒ 非阻塞 Minor，见「Acceptance Gaps」非阻塞 1 与 Issue 候选 2。
   - **锁粒度与死锁**：`restore_dirty_from` 无 `.await`；锁序为 **DashMap 分片读锁 → 页帧 `Mutex`**，与 `flush_all` phase 1 的既有锁序一致；`evict_one` 在取页帧锁**之前**已释放 DashMap 守卫（`match self.pages.get(..)` 的临时值随语句结束析构）⇒ **无锁序反转、无跨 await 持锁**。
6. **RED 证据的可信度 —— 成立**。
   - **还原与恢复的完整收口**：当前工作区的三个改动面同时包含两处修复；其 mtime（`buffer_pool.rs` 15:50:14、`file_storage.rs` 15:46:48、测试 15:50:22）**早于** Act Response 的写入时间（15:53:07）⇒ 还原—重跑—恢复序列已完整收口，工作区确为 GREEN 版本。
   - **RED 的位置与消息与当前测试文件逐字对应**：Act 报告的三处 RED 位置（`:88`、`:147`、`:165`）与失败消息（「a failed flush must leave the unwritten pages dirty…」「the new table must not take the released data page 2: the release must not be promoted while its anchor write is not durable」）与 `tests/release_durability_precondition_test.rs` 的当前内容逐字吻合 ⇒ RED 是在**本版本**测试文件上观测到的。
   - **RED 的语义可复核**：撤掉 Fix 1 ⇒ 失败刷盘后该页不再为脏（R1 断言失败）；只留 Fix 1 ⇒ 释放被提前提升（新表取走被释放的数据页 2，R2 断言失败）⇒ 与实现语义一致。
7. **候选 3 的结论（失败的 DDL 其目录变更在后续刷盘落地）——可达且由本次修复引入**。
   - **可达（读码）**：`AlterTableOp::AddColumn` → `TableManager::alter_add_column`：目录面变更（`insert_column_row`）**先于任何页分配**发出 → 随后的 `update_table_column_count_ddl` → `Catalog::update_table_tail` → `Catalog::flush_commit_point`（`catalog.rs:269`）= `flush_all_except` + `flush_pages` ⇒ **语句边界的落盘发生在目录面变更之后**，且它在 checkpoint 之外 ⇒ 失败点可达。
   - **修复引入**：修复前该次失败的落盘已把目录页**清脏**且不归还 ⇒ 目录变更被丢弃（Act 实测：重开后 `Column not found: m`，且表行的列数已含新列而列行丢失——目录自相矛盾）；修复后脏标记归还 ⇒ 后续一次成功刷盘把目录变更带出去（Act 实测：重开后 `[1, 10, Null]`）。
   - ⇒ 与既有域 `ddl-catalog-transactional` 的「失败的 DDL 语句 SHALL NOT 留下目录面变更」存在**字面张力** ⇒ 报告为**范围外 Issue 候选**（见下），本 change 不承载。
8. **身份证明机制与判定层检查 —— 通过**。无 Hash / 指纹 / revision pin / run-id / session-id / peer pin / manifest / 时间顺序证明；`last_failed_write` 与 `release_id` 是**状态机顺序计数器**（非运行归属标识、不落盘）。判定全部在 `cargo test` 内（新测试文件 2 条用例 + 既有测试文件）；无框架外判定脚本、无新增 CLI、无 timer 阈值行为判据（`IoFailPoint` 是既有注入设施，MS33 Iter001）。
9. **规格增量逐条核对与就地更正**。

   | Requirement / Scenario | 已验收证据 | 核对结论 |
   |---|---|---|
   | RDP R1 Requirement（刷盘失败不丢弃脏页写） | `failed_write_keeps_page_dirty`（`:66`） | 成立（三个入口均覆盖） |
   | RDP R1 场景 1（数据页写失败后该页仍为脏） | 同上 | 成立 |
   | RDP R1 场景 2（失败后的脏页在后续成功刷盘时被重写） | 同上 | 成立 |
   | RDP R1 场景 3（DDL 边界落盘失败不丢弃目录写） | 同上的机制 + 候选 3 的实测 | 行为成立，但**原文断言了语句层面的结果** ⇒ 与 `ddl-catalog-transactional` 冲突 ⇒ 已改写为**存储层**保证并划清域边界 |
   | RDP R2 Requirement（提升 ⇒ 锚点写已持久） | 实现 + 编号推导（Finding 4） | 成立；已补记「追加前置条件」与对 `free-page-lifecycle` R1 的收紧关系 |
   | RDP R2 场景 1 | `release_not_promoted_when_anchor_write_not_durable`（`:123`） | 「不被提升」成立；**重开半句不可满足** ⇒ 已删去 |
   | RDP R2 场景 2 | 既有 `free_page_lifecycle_test`（12/0） | 成立；已补 GIVEN 限定（否则与实现的收紧相抵触） |
   | RDP R2 场景 3 | `release_not_promoted_when_anchor_write_not_durable` 的重开段 | 成立 |

   **就地更正的措辞（增量规格现只陈述已验收行为）**：
   - **场景 1**：删去「AND 重开该库时该页 SHALL NOT 被判为可分配页」（不可满足）；GIVEN 由「其锚点写所在页的写盘失败过一次（该页此后仍为脏）」改为「**其释放完成之后发生过一次页写盘失败**」（与实现的进程级闸门一致）；WHEN 保留「执行一次成功刷盘并走完 checkpoint」；THEN 保留「该释放 SHALL NOT 被提升为可分配」，并补「其页在本会话内不被发放」这一已验收的可观察面。
   - **新增场景「未被提升的释放在重开库后按持久目录重建」**：GIVEN「一个释放未被提升为可分配，且其锚点写已随一次成功刷盘落盘（持久目录不再登记旧对象）」→ WHEN「关闭并重新打开该库」→ THEN「打开时按持久目录重建的可分配页集合 SHALL 把该释放涉及的页判为可分配页（持久目录已不登记旧对象，其页可再次分配）」+ AND「完整性扫描的 error 级发现数为 0」。⇒ 原文不可达的行为被替换为**实际可达且已验收**的观察。
   - **场景 2**：GIVEN 补「**且其释放完成之后没有发生过页写盘失败**」⇒ 与实现的收紧一致（否则该场景在失败情形下与实现相抵触）。
   - **R2 Requirement**：补一段「本要求是既有域 `free-page-lifecycle` R1 的**追加前置条件**，不改动 R1 的判定时点……该追加条件是既有 R1 的**收紧**，不放松 R1 的任何条款」⇒ 收紧关系显式化，合并进 `openspec/specs/` 后无歧义。
   - **R1 场景 3**：标题与 THEN 改为**存储层**保证（「DDL 语句边界的落盘失败不丢弃已清脏的页写」：「这些页 SHALL 被写入存储（不得因前一次失败而丢失其写）」）⇒ 不再断言失败的 DDL 语句的目录面变更必须生效。
   - **Purpose**：补记「本域只规定**存储层**的保证……**DDL 语句层面的结果**（一条失败的 DDL 语句是否留下目录面变更）由既有域 `ddl-catalog-transactional` 规定，本域不裁决」。

**Acceptance Gaps**

- **无阻塞项**：Cycle 的 Acceptance 逐项满足——(1) RDP R1 三条场景由 `failed_write_keeps_page_dirty` 覆盖；(2) RDP R2 的「不被提升」由 `release_not_promoted_when_anchor_write_not_durable` 覆盖，其「重开后该页不被判为可分配页」半句经独立推导**不可满足**，已按本 Review 的裁定就地改写为可达且已验收的观察（改写的是**规格文本**，验收目标的行为未变）；(3) MS35 已验收的 `free_page_lifecycle_test` **12/0** 全绿（含 `flush_failure_does_not_promote_pending_release`、`released_page_not_allocatable_before_release_durable`、`released_page_allocatable_after_release_durable`）⇒ 既有行为零破坏；(4) 既有 `drop_persistence_test` / `checkpoint_test` / `release_path_bounded_read_test` / `ddl_*` / `index_page_ownership*` 全绿（沿用 Act 的实测）。
- **非阻塞 1**（并发残余窗口）：「并发驱逐 + 写盘失败」下被驱逐页的内容可能既未落盘也不在池内（Finding 5）⇒ 建议登记为 Issue 候选；不影响 RDP R1 的已验收场景（场景隐含「页仍在池内」）。
- **非阻塞 2**（闸门的进程级粒度）：闸门不区分「锚点写所在页」与其它页 ⇒ 一次写盘失败会拦住该次失败之前完成、尚未提升的全部释放在本会话内不再提升（安全方向；重启后按持久目录恢复）。已由 Act 自陈（遗留 Minor 1），本次 Review 复核为**保守且安全**，不阻塞。
- **非阻塞 3**（`last_failed_write` 不复位）：后续 checkpoint 的「重判」结论恒为「不提升」，与 Plan Context 所述「等下一次 checkpoint 重判」的字面预期不同（重判存在但不翻转）⇒ 已记入 `design.md` D8；不阻塞。
- **非阻塞 4**（RDP R1 场景 3 的**覆盖**形态）：DDL 边界落盘失败的实例由 Act 的候选 3 实测直接观察（目录写在后续刷盘落地），但**无常设用例**；通用机制由 `failed_write_keeps_page_dirty` 覆盖 ⇒ 覆盖缺口（非阻塞），可由后续 Iteration 的用例补齐。

**Evidence**

- 本次 Review **独立阅读**的代码与产物：`src/storage/buffer_pool.rs`（刷盘三入口 + `restore_dirty_from` + `evict_one` 全文）、`src/storage/file_storage.rs`（`write_page` / `record_write_failure` / `complete_free_release` / `release_completion_limit` / `promote_free_releases` / `discard_free_releases`）、`tests/release_durability_precondition_test.rs`（全文）、`src/storage/data/table_manager.rs`（`drop_table` 的锚点写与完成号、`alter_drop_column` 的 Iteration 001 参照形态、`alter_add_column`）、`src/executor/delete.rs` / `src/executor/upsert.rs`（Iteration 005 的目标面，只读）、`src/executor/data_scan.rs`（`build_superseded_map` / `slot_is_superseded` / `superseder_suppresses`）、`src/wal/checkpoint.rs`（第 2 步的取样 / 提升 / 丢弃）、`src/database.rs`（`reclaim_abandoned_implicit_transactions`）、`src/storage/catalog.rs`（`delete_table` / `flush_commit_point` / `update_table_tail`）、`tests/free_page_lifecycle_test.rs`（12 条用例）、`tests/ddl_rewrite_consistency_test.rs`（放弃夹具形态），以及 change 的 `tasks.md` / `design.md`（D8）/ `proposal.md` / 规格增量与既有域 `ddl-catalog-transactional` / `drop-persistence-boundary` / MS35 待合并的 `free-page-lifecycle`。
- 本次 Review **未运行**任何测试命令（按公共规则沿用 Act 已通过的结论）：Act Response 记录 `release_durability_precondition_test` 2/0、`free_page_lifecycle_test` 12/0、`drop_persistence_test` 11/0、`checkpoint_test` 2/0、`release_path_bounded_read_test` 7/0、`ddl_catalog_transactional_test` 3/0、`ddl_crash_recovery_test` 6/0、`ddl_rewrite_consistency_test` 9/0、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`cargo test --lib` 420/0、`cargo build --tests` 0 警告 ⇒ **覆盖范围未失效**（Act Response 的写入时间晚于三个改动面的最后修改时间，见 Finding 6）⇒ 不重复运行已通过的验证。全量回归仍归 Iteration 007 / T21（M19 分批串行）。
- **工作区状态**：本次 Review **未改产品代码**，只写了 change 产物（本段 Review、`specs/release-durability-precondition/spec.md`、`design.md` D8、`tasks.md` 的状态与文本对齐）；未创建临时探针文件，既有未提交改动未回滚、未清理、未提交。

**Issue Candidates**（只报告不登记；登记与承载由 Recorder / 后续 Plan 执行）

1. **可承载**：「失败的 DDL 语句其目录面变更在后续刷盘落地」——与既有域 `ddl-catalog-transactional` 的「失败的 DDL 语句 SHALL NOT 留下目录面变更」字面冲突。
   - **结论**：修复引入的**条件性副作用**，已实测可达（Act 的候选 3 实测 + 本次 Review 的读码复核）。**不在本 change 的承载范围**（`design.md` D9：B-3 / DDL 语句边界落盘的顺序语义不承载）⇒ 报告给 Recorder 择承载（另立 change 或后续里程碑）。**关闭它需要一次产品语义决定**：回滚目录面变更（需目录面变更的可撤销形态）或不回滚（需修订既有域 `ddl-catalog-transactional` 的字面条款）——该决定不由 Plan 单方裁定。
   - **根因位置**：`src/storage/catalog.rs::flush_commit_point`（`:269`）的失败语义 + 目录面变更在语句边界落盘**之前**已发出（`insert_column_row` / `update_table_tail`）。
   - **影响面与触发条件**：任何在目录面变更已发出后**语句边界落盘失败**的 DDL（`ADD COLUMN` 已实测；`DROP COLUMN` / `CREATE INDEX` 同型待验）；表现为一条**失败**的 DDL 在重开后留下目录面变更。
   - **承载落点**：由 Recorder 登记；承载形态建议为**另立 change**（修复面在 DDL 语句边界的事务语义，与本 change 的 007 收尾全量无耦合）；若要在本 change 内承载，须新开一个 Iteration 并排在 007（收尾全量）之前。
2. **可承载**：「并发驱逐 + 写盘失败」下被驱逐页的内容丢失（本次 Review 新发现的残余窗口，Finding 5 / 非阻塞 1）。
   - **结论**：既有窗口的**收窄**形态（修复前为「任何一次刷盘失败都丢页内容」）。修复面在**驱逐路径的脏标记读取**与刷盘 phase 1 的清脏时机之间的竞态（缓冲池并发面，非本 Cycle 的授权范围）。
   - **影响面与触发条件**：缓冲池达到容量上限触发驱逐、且驱逐与一次失败刷盘并发时；表现为该页内容既未落盘也不在池内。
   - **承载落点**：由 Recorder 登记（另立 change 或后续里程碑）；不影响本 change 的验收面。
3. **无其它 Issue 候选**：本次 Review 未发现新的实质缺陷面。

**Follow-up Decision**

- **当前 Cycle 无需修复**：无阻塞 Acceptance 缺口 ⇒ **不**创建 rework / replan Cycle；Iteration 004 的 `Plan Context` 与 `Act Response` **保持原样**（Plan Context 不可改写）。
- **本次 Review 就地更正的产物**（均为**文本**层：不改实现、不改验收目标的行为、不改磁盘格式）：
  1. `specs/release-durability-precondition/spec.md`：删去不可满足的「重开后该页不被判为可分配页」半句；场景 1 的 GIVEN 与实现对齐；场景 2 补 GIVEN 限定；新增「未被提升的释放在重开库后按持久目录重建」场景；R1 的 DDL 边界场景改写为存储层保证；R2 Requirement 补记「追加前置条件」与对 `free-page-lifecycle` R1 的收紧关系；Purpose 补记与 `ddl-catalog-transactional` 的域边界（措辞见 Finding 9）。
  2. `design.md` D8：决策 2 改为实际形态，并记录判据形态的实测定形、取样点冲突的推导与收紧的代价；「与 MS35 R1 的关系」段落记录收紧的具体后果。
  3. `tasks.md`：T13 描述、T12/T13 执行依据要点、Iteration 004 的 Stable result、RTM 的 RDP R2 状态（→ **Done**）、Iteration Map 的 004 行（→ **accepted**）与 005 行（→ **已展开**）、Next Iteration 段。
- **后续承载**：Issue 候选 1 / 2 ⇒ Recorder 登记（Plan 只报告不登记）。

**Iteration Plan Update**

- Iteration 004 的计划**不变**（Acceptance 逐项满足；两项实质 Deviation 均经裁定后由**文本更正**收敛，不需返工或重规划）；稳定结果与诊断边界按 `tasks.md` 的 Iteration 004 段落保留（其「锚点写所在页已不脏」表述已按 Finding 2 更正为实际形态）。
- change 层面的计划调整仅限：Iteration Map 的 004 / 005 行状态与 Next Iteration 段，以及上文列出的文本对齐项。**未**新增 / 删除 / 重排 Iteration；Iteration 007 继续承担收尾全量（T21）。

**Next Cycle**

None

**Next Iteration**

- Iteration 005 已展开：`iterations/005-dml-tombstone-abandon-window/000-initial.md`（Cycle 000，`Plan Context` 状态 `ready`，承载 T14–T15：DML 墓碑的放弃窗口收敛——被放弃的 `DELETE` 使其行不可见（实测可见 id `[2..30]`，`id=1` 缺失），根因＝墓碑带链指针写入 → 索引条目被删 → 版本注册；修复形态与 Iteration 001 同形）。
- 下一个待展开 Iteration 为 **006**（索引树读路径的有界读与显式拒绝）。
