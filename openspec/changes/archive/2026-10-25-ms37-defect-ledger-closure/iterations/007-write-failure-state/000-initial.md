# Iteration 007 / Cycle 000: 写盘失败的状态面（在途写入不被按干净页驱逐 + 失败集合按页记录并可复位）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T9（ISS30）、T10（ISS31）
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - change `design.md` **D8**（写盘失败状态是内存态，不落盘；R1 保持，R2 改为按锚点页）。**不得另选方向**。
  - change `specs/write-failure-state/spec.md`。
  - change `specs/release-durability-precondition/spec.md` 的 **MODIFIED** R2（本 Cycle 写入；主规格里「失败之后本会话永不提升」那一句由此修订）。
  - Iteration 006 已交付的 DDL 提交点（窄化意图、恢复期发布、`ADD COLUMN` 补偿、窄化令牌）**零改动**。
  - `free-page-lifecycle` 的判定时点：释放完成编号、checkpoint 在刷盘前取样、刷盘成功才 `promote_free_releases`、刷盘失败走 `discard_free_releases`。**不改这个时点**。
- Excluded scope:
  - T11 / T12 → Iteration 008。
  - T13–T16 → Iteration 009。全量回归不在本 Cycle。
  - ISS30 / ISS31 台账条目的编辑。
  - 全局 `.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references。
  - Iteration 006 已报告、未登记的两个范围外候选（`ADD COLUMN` 与在途窄化并发；`ADD COLUMN` 在列数刷盘期间被放弃）。

**Objective**

1. 刷盘已经取走、写入尚未结束的页，驱逐路径不得把它当成干净页移出池。写盘失败后，该页仍留在池内，新内容仍可取回，失败以上抛错误结束（ISS30）。
2. 写盘失败按页记录。该页其后一次成功写入后离开失败集合。释放是否提升，看它的锚点页是否仍在失败集合内，不再看库级一次性编号（ISS31）。

**Investigation Facts**

- Starting point:
  - Iteration 006 / Cycle 000 的 Act Response（2026-10-26，`reported`）与本 Plan Review（`accepted`）是起点。006 改了 WAL 记录、恢复发布、目录补偿和窄化令牌；**没有**改 `evict_one`、`flush_all` 或 `last_failed_write`。006 的定向回归两批各 8 个目标、批内 `--test-threads=1`，全绿；`cargo test --lib` 420 passed / 0 failed。全量仍只预算在 Iteration 009 / T16。
  - 工作区产品改动未提交。本 Cycle 从当前代码实施，不回退 000–006。
- Current-State Evidence（行号是当前工作区，符号名是定位依据）：
  - `Database::open` 的缓冲池容量是 **100**（`src/database.rs` 的 `BufferPool::new(100, …)`）。小 SQL 夹具填不满池，**不能**用它稳定打出驱逐交错。
  - `src/storage/buffer_pool.rs::evict_one`：选中页后复制内容并放开帧锁；`dirty == false` 时直接 `pages.remove`。`dirty == true` 时先 `write_page`，失败则 `?` 返回、**不** remove。
  - `flush_all` / `flush_all_except` / `flush_pages`：持帧锁时把 `dirty` 清掉并复制页，然后在锁外 `write_page`。失败时 `restore_dirty_from` 把脏标记还给**仍在池内**的页。页若已不在池内，恢复跳过。
  - 交错窗口：`flush_*` 已清脏、写盘尚未返回时，`evict_one` 看见 `dirty == false`，把页当干净页移出池。随后写盘失败，池内和盘上都没有这次的新内容。这是 ISS30。
  - `src/storage/page_frame.rs::PageFrame` 只有 `dirty` / `ref_count` / `clock_bit`，没有「写入在途」标记。
  - `src/storage/file_storage.rs`：`last_failed_write: AtomicU64` 只在 `record_write_failure` 里写入，**没有**清除路径。`write_page` 在注入失败和真实 `write_page_blocking` 失败时都调用它，**不**带页号。`promote_free_releases`：完成编号 `id < limit` 且 `last_failed_write > id` 时不提升。一次失败之后，完成编号小于该失败编号的释放在本会话内不再提升，即使锚点页后来写成功。这是 ISS31。
  - `pending_frees` 的元素是 `(被释放页号, Option<完成编号>)`，**不**记录锚点页。
  - `complete_free_release(&pages)` 的调用点已经用注释写明锚点。锚点不是被释放的那一页：
    - `table_manager.rs` `drop_table`：目录删除（`catalog.delete_table`，改 `__tables` / `__columns` 链）。
    - `table_manager.rs` `truncate_table`：表头页原地重置 + `update_table_tail` 改到的目录页。
    - `table_manager.rs` 约 2525 行（旧树释放）：目录 root 更新所在页。
    - `table_manager.rs` 约 933 行、约 2492 行：注释写明**无锚点写**。
    - `executor/create_index.rs` 回填失败：无锚点写。
    - `executor/drop_index.rs`：目录摘除所在页。
    - `btree/index_manager.rs` 缩根：`sync_root_to_catalog` 写到的目录页。被释放的是旧根，旧根不是锚点。
    - `btree/btree.rs` 两处合并：父页 `parent_id`。被释放的是 `right_id`。
    - `wal/recovery.rs` 约 1660 行：目录 root 更新所在页。约 322 行的溢出链释放是失败清理，无锚点。
    - `page_format/overflow.rs` `free_overflow_chain`：失败清理无锚点；`delete_tuple_from_data_page` 的调用点锚点是正在删槽的数据页 `page_id`。
  - `update_field_in_chain` 已经返回被改页。`delete_table` 目前返回 `()`，链上实际改到的页号在 `delete_from_chain` 内部。
  - 既有注入点 `Database::arm_io_failure(IoFailPoint::DataWrite)` 只让**下一次** `write_page` 失败。它不能把「清脏」和「写盘返回」之间的驱逐插进去。
  - `tests/release_durability_precondition_test.rs::release_not_promoted_when_anchor_write_not_durable` 锁的是**修订前**的 R2：锚点页后来被 checkpoint 成功写盘，释放仍然不提升。修订后，这次 checkpoint 会把锚点页写出并移出失败集合，释放应当提升。该用例要改断言，不能当作回归去保住旧行为。
  - `src/wal/checkpoint.rs::checkpoint`：刷盘成功才 `promote_free_releases`，失败走 `discard_free_releases`。零工作早退也会 promote。这条顺序保持。
- Code and Critical Path:
  - 驱逐 / 刷盘：`buffer_pool.rs` 的 `evict_one`、`flush_all`、`flush_all_except`、`flush_pages`、`restore_dirty_from` ↔ `page_frame.rs::PageFrame`。
  - 失败集合与提升：`file_storage.rs` 的 `write_page`、`promote_free_releases`、`complete_free_release` ↔ `async_storage.rs` 的对应方法 ↔ 上面列出的调用点。
  - 观察入口：缓冲池容量为 1 的单测（T9）；真实 `Database::open` + `DROP TABLE` / `CREATE TABLE` + `arm_io_failure`（T10）。

**Implementation Guidance**

1. 先做 T9 的在途标记，再做 T10 的失败集合。T10 的「失败后页仍在池内」依赖 T9，否则失败页被驱逐后失败集合没有可复位的页内容。
2. `PageFrame` 增加内存字段 `write_in_flight: bool`。不进页字节，不进文件头。
3. 准备写盘的路径（三处 flush，以及 `evict_one` 自己的脏页写）在持帧锁时：若该页已经 `write_in_flight`，本轮不要清脏、不要再发一次写。否则复制字节、置 `write_in_flight`、清 `dirty`，放开锁后再 `write_page`。
4. 写盘返回后清 `write_in_flight`。失败则把 `dirty` 设回。成功则若等待期间又被标脏，保持脏，页留在池内。
5. `evict_one` 看见 `write_in_flight` 时与 `ref_count > 0` 同样处理：放回 clock，不 `remove`。自己的写盘失败：不 `remove`，返回错误。自己的写盘成功：再看 `dirty`；仍脏则不 `remove`，放回 clock。
6. 用 `failed_pages`（页号集合，内存，互斥保护）替换 `last_failed_write`。`write_page` 失败把该页号放入；该页号的 `write_page` 成功后移出。只替换这一处判据，不改 `release_id` 的取样和 `id < limit`。
7. `complete_free_release(pages, anchors)`。`pending_frees` 的每一项带上这次调用的锚点页号。`anchors` 为空表示注释里的「无锚点写」：提升不看失败集合。锚点非空时，任一锚点仍在 `failed_pages` 里则该项留在待释放集合。
8. 锚点页号从既有写入函数返回，不新造页号。`delete_table`、`update_table_tail`、`update_table_root`、目录摘除、`sync_root_to_catalog` 返回它们实际改过的页。`delete_from_chain` / `update_field_in_chain` 里已经有这个页。btree 合并的锚点是 `parent_id`，不是 `right_id`。
9. `free_overflow_chain` 增加锚点参数。失败清理传空。`delete_tuple_from_data_page` 传 `page_id`。

**Task Definitions**

### T9: 在途写入的页不被按干净页驱逐，写盘失败后新内容仍可取回（ISS30）

- Requirement/Scenario: WFS「驱逐不得丢弃未落盘的内容」（「写盘失败时被驱逐的页仍可取回」「在途写入的页不被按干净页驱逐」）；`release-durability-precondition` R1 保持
- Depends on: None
- Targets:
  - `src/storage/page_frame.rs::PageFrame`
  - `src/storage/buffer_pool.rs::evict_one`
  - `src/storage/buffer_pool.rs::flush_all`
  - `src/storage/buffer_pool.rs::flush_all_except`
  - `src/storage/buffer_pool.rs::flush_pages`
  - `src/storage/buffer_pool.rs::restore_dirty_from`
- Current behavior:
  - flush 清掉 `dirty` 之后、`write_page` 返回之前，`evict_one` 把该页当干净页 `remove`。
  - 随后写盘失败时，`restore_dirty_from` 找不到这一页。新内容不在池内，盘上仍是旧内容，调用方不一定看到与这次丢失对应的错误。
- Required behavior:
  1. 刷盘或驱逐已经复制出页内容并发出写盘时，该页处于「写入在途」。写入返回（成功或失败）后清除这个状态。
  2. 驱逐扫到写入在途的页时，不得把它当干净页移出池。
  3. 写盘失败：该页留在池内，脏标记恢复，池内字节仍是这次要写的新内容；错误上抛，不吞掉。
  4. 写盘成功，且等待期间页又被改脏：页留在池内，新的脏内容不被这次成功写盘的旧副本带走。
  5. 磁盘格式不变。
- Preserve:
  - R1：失败的刷盘把脏标记交还给仍在池内、且这次没写成功的页。`tests/release_durability_precondition_test.rs::failed_write_keeps_page_dirty` 的可观察结果保持。
  - 帧锁不跨 `.await`。
  - `evict_one` 对 `ref_count > 0` 和 clock 位的处理保持。
- Forbidden:
  - 改页字节布局、文件头、WAL 记录。
  - 为这个状态新增产品配置、命令参数或公共 SQL 行为。
  - 新增产品代码里的暂停钩子。单测用的闸门存储只存在于 `#[cfg(test)]`。
  - 跑全量回归。
- First test:
  - 位置：`src/storage/buffer_pool.rs` 的 `#[cfg(test)]`。
  - 夹具：容量 **1** 的 `BufferPool`，存储的 `write_page` 在进入后通知测试并等待，然后返回 `StorageError::Io`。不写盘。
  - 步骤：放入一页并改成可识别的新字节且标脏 → 启动 `flush_all` → 等到 `write_page` 已进入 → 再 `get_page` 另一页（迫使驱逐）→ 放行写盘失败。
  - RED（变更前）：驱逐把已清脏的页移出池；失败后按页号取回的是盘上旧字节或取不到新字节。断言「失败后池内仍是新字节」失败。这是行为失败，不是编译失败。
- GREEN condition:
  - 同一夹具：写盘失败后，原页号上的字节仍是新字节，页仍在池内，`flush_all` 返回错误。
  - `failed_write_keeps_page_dirty` 仍绿。
  - `cargo test --lib -- --test-threads=1` 中本夹具为绿。
- Verification:
  - `cargo test --lib failed_flush_then_evict_keeps_the_new_bytes -- --test-threads=1` 先红后绿。
  - 本 Cycle 的定向回归见文末 Verification。不跑全量。
- Stop when: 必须改页格式或文件头；必须在产品路径加暂停钩子才能稳定复现；闸门存储被做成产品判定层或身份证明。

### T10: 失败集合按页记录，锚点页成功写入后释放可以提升（ISS31）

- Requirement/Scenario: WFS「写盘失败状态按页记录并可复位」+「释放的可分配判定按锚点页的失败状态表达」；`release-durability-precondition` R2（本 Cycle 的 MODIFIED）
- Depends on: T9
- Targets:
  - `src/storage/file_storage.rs` 的 `last_failed_write` / `record_write_failure` / `write_page` / `promote_free_releases` / `complete_free_release`
  - `src/storage/async_storage.rs::complete_free_release`
  - `src/storage/buffer_pool.rs::complete_free_release`
  - `src/storage/btree/sync_loader.rs::complete_free_release`
  - Investigation Facts 列出的全部 `complete_free_release` 调用点
  - `src/storage/catalog.rs::delete_table`（改为返回实际改过的页号）
  - `src/storage/page_format/overflow.rs::free_overflow_chain`
  - `tests/release_durability_precondition_test.rs::release_not_promoted_when_anchor_write_not_durable`（断言改到修订后的 R2）
- Current behavior:
  - 任意一次写盘失败把库级编号抬高且永不下降。完成编号更小的释放，即使锚点页后来写成功，本会话也不再提升。
  - 失败不区分页。未涉及的页也会被这个编号挡住。
- Required behavior:
  1. 失败集合的成员是页号。失败的 `write_page(page)` 把该页放入；该页其后一次成功的 `write_page` 把它移出。其它页不因这次失败进入集合。
  2. 提升仍要求完成编号 `id < limit`（checkpoint 的取样时点不变）。此外，该项记录的锚点页有任一仍在失败集合内，则不提升，并留在待释放集合。锚点列表为空时，不因失败集合拒绝。
  3. 锚点页成功写入并离开集合之后，再触发提升，该释放被提升，随后分配可以拿到被释放的页。
  4. 刷盘失败仍走 `discard_free_releases`，不在失败的 checkpoint 里提升。
  5. `release_not_promoted_when_anchor_write_not_durable` 的 checkpoint 会成功重写锚点页。修订后的断言：这次 checkpoint 之后，新建表拿到被释放的页之一。锚点仍在失败集合内时不提升，由下面的新用例锁住。
- Preserve:
  - `free-page-lifecycle` 的编号、`id < limit`、失败刷盘丢弃、重开后按持久目录重建。
  - R1 的脏标记交还。
  - `:memory:` 与未覆盖该方法的测试存储继续走 trait 默认实现（无崩溃边界）。
  - Iteration 006 的 DDL 行为。
- Forbidden:
  - 保留 `last_failed_write` 再平行加一套页集合。
  - 用「当前所有脏页」代替调用点注明的锚点。
  - 改磁盘格式；跑全量；新增身份证明或产品暂停钩子。
- First test:
  - 位置：`tests/write_failure_state_test.rs`（新建）。真实文件库、真实 `Database::open`、`arm_io_failure(IoFailPoint::DataWrite)`。
  - `anchor_stays_unpromoted_while_its_page_is_failed`：`CREATE TABLE t` + 插入 + `DROP TABLE t`，记下 t 的数据页与索引根。武装一次写失败并 `flush_all`（失败，锚点页进入集合且仍脏）。在成功重写锚点页**之前**调用 `promote_free_releases(release_completion_limit())`。再 `CREATE TABLE u`。RED（变更前，若提升仍被库级编号挡住，这个 RED 可能已经是「不提升」从而用例误绿）——变更前库级编号也会不提升，所以**不能**用「不提升」单独当 RED。RED 载体是下一条。
  - `anchor_is_promoted_after_its_page_is_rewritten`：同一前置，写失败之后再 `checkpoint()`（成功刷盘把锚点页写出）。`CREATE TABLE u` 的数据页或索引根等于 t 释放的页号之一。变更前 `last_failed_write` 仍挡住，`assert` 失败。这是 RED。
  - `unrelated_page_failure_does_not_block_a_later_release`：一次写失败并随后成功重写该页（它离开集合）之后，另一次 `DROP TABLE` 的释放在 checkpoint 后被提升。变更前，更早的失败编号仍大于这次完成编号时，提升不发生，断言失败。若该次序下完成编号已经大于失败编号、变更前也会提升，则这条在变更前就是绿的，只作对照，RED 以 `anchor_is_promoted_after_its_page_is_rewritten` 为准。
- GREEN condition:
  - 锚点仍在失败集合内：新建表拿不到被释放页。
  - 锚点页被成功重写后：新建表拿到被释放页之一。
  - 无关页的失败不阻止另一次已完成释放在锚点不在集合内时被提升。
  - 修订后的 `release_not_promoted_when_anchor_write_not_durable` 为绿：checkpoint 成功之后释放被提升。
  - `free_page_lifecycle_test` 与 `drop_persistence_boundary_test` 为绿。若其中某条断言的是「锚点已成功重写后本会话仍永不提升」，按修订后的 R2 改这条断言，不改释放时机。其它失败回到 Plan。
- Verification:
  - 见文末。不跑全量。
- Stop when: 调用点没有可返回的锚点页号，而且注释也没有写「无锚点写」；必须改 `id < limit` 或刷盘失败时的 discard 才能让新断言成立；必须改磁盘格式。

**Invariants**

- 磁盘格式不变。失败集合和 `write_in_flight` 只在内存。
- 帧锁不跨 `.await`。
- 释放提升的时点仍是 checkpoint 刷盘成功之后的 `promote_free_releases`，外加零工作早退上的同一次调用。刷盘失败仍 discard。
- R1 保持：失败的刷盘不把没写成功的页留成「不脏」。
- Iteration 006 的 DDL 提交点、恢复发布、补偿和令牌保持。
- 不引入身份证明、判定层、新产品参数或配置。

**Non-goals**

- 全量回归（Iteration 009 / T16）。
- dump / restore（Iteration 008）。
- 把无锚点的释放改成永久不提升，或改成立即进入可分配集合。
- 修改 `free-page-lifecycle` 的编号空间。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A29 | 写盘已取走页内容、写入未结束时，驱逐不把该页当干净页移出池 | WFS「在途写入的页不被按干净页驱逐」 | D8 ISS30 | T9 | `PageFrame::write_in_flight`、`evict_one`、三处 flush | `failed_flush_then_evict_keeps_the_new_bytes` |
| A30 | 写盘失败后页仍在池内，新字节仍可取回，错误上抛 | WFS「写盘失败时被驱逐的页仍可取回」 | D8 ISS30 | T9 | 同上 | 同上 |
| A31 | 失败按页记录；该页成功写入后离开集合；其它页不进入 | WFS「写盘失败状态按页记录并可复位」「未受影响的页不进入失败集合」 | D8 ISS31 | T10 | `FileStorage::failed_pages`、`write_page` | `unrelated_page_failure_does_not_block_a_later_release` |
| A32 | 锚点页仍在失败集合内时不提升；成功重写后提升，分配拿到被释放页 | WFS 两个提升 Scenario；R2 MODIFIED | D8 ISS31 | T10 | `promote_free_releases`、调用点传入的锚点页号 | `anchor_stays_unpromoted_while_its_page_is_failed`、`anchor_is_promoted_after_its_page_is_rewritten` |
| A33 | R1、释放编号时点、刷盘失败 discard、重开按目录重建保持；006 的 DDL 行为保持 | R1；`free-page-lifecycle`；DLP | D8 | T9, T10 | 上述路径 | `failed_write_keeps_page_dirty` + `free_page_lifecycle_test` + `drop_persistence_boundary_test` + `cargo test --lib` |

**Verification**

- T9 的交错用容量 1 的缓冲池单测直接观察页字节。`arm_io_failure` 打不中这个窗口。
- T10 用真实库：`DROP TABLE` / `CREATE TABLE` / `flush_all` / `checkpoint` / `promote_free_releases`。断言是页号是否被新建表拿去，以及写盘错误是否返回。
- 定向回归只跑一批，目标按名字典序，批内 `--test-threads=1`：

```text
cargo test --test drop_persistence_boundary_test --test free_page_lifecycle_test --test release_durability_precondition_test --test write_failure_state_test -- --test-threads=1
```

- 然后单独跑 `cargo test --lib -- --test-threads=1`（D10）。两步串行，不同时跑。
- 不跑无目标过滤的全量，不用默认并行度跑这一批。全量仍只在 Iteration 009 / T16，按该处已写的分批规则。
- 通过条件：这一批 0 failed，且 `--lib` 0 failed。

**Gate 2 Readiness**

- Step 5：无 TBD。R2 的会话级永不提升已由本 Cycle 的 MODIFIED 规格修订，并与 `write-failure-state` 对齐，无未批准裁剪。tasks / specs / design D8 一致。Current-State Evidence 来自当前 `evict_one`、三处 flush、`last_failed_write`、`promote_free_releases` 和 `complete_free_release` 调用点。两个任务的入口、锚点页和 RED 形态已写明。全量不在本 Cycle。Act 只读本文件即可写第一个先行测试。验证直接看页字节、错误和页号是否被再次分配。无身份证明、无判定层。⇒ 全部 PASS。
- 用户批准：goal「继续把当前 ms 做完，记得遵守各种规范，尤其是多批次测试规范」构成 Gate 2 的批准。批准不降低判据。

**Cycle RTM**

| Requirement / Scenario | Task | First Test | Status |
|---|---|---|---|
| WFS「驱逐不得丢弃未落盘的内容」 | T9 | `failed_flush_then_evict_keeps_the_new_bytes` | Covered |
| WFS「写盘失败状态按页记录并可复位」 | T10 | `anchor_is_promoted_after_its_page_is_rewritten` | Covered |
| WFS「释放的可分配判定按锚点页的失败状态表达」 | T10 | `anchor_stays_unpromoted_while_its_page_is_failed` | Covered |
| R2 修订为按锚点页；R1 与释放时点保持 | T9, T10 | `release_durability_precondition_test` + `free_page_lifecycle_test` | Covered |

**Persisted Evidence**

- Mode: none

单测、一个集成测试目标和三个既有目标都可以重跑。结果写入 Act Response。不建 `evidence/` 目录。

**Risks and Notes**

- `anchor_stays_unpromoted_while_its_page_is_failed` 在变更前也可能不提升（库级编号同样挡住）。它锁修订后的「仍在集合内不提升」，RED 不以它为准。
- `release_not_promoted_when_anchor_write_not_durable` 在实现之后会因旧断言失败。这是规格修订，不是把旧 R2 修回去。
- `overflow.rs` 约 606 行的测试直接调用 `FileStorage::complete_free_release`，签名变化后要补上锚点参数。该测试的期望（提升后分配到同一页、且没有写失败）保持。
- 既有 fmt / clippy 基线仍按 `design.md` D13：只核对本 Cycle 触碰的文件，不重排无关文件。
- 跳过项：不跑全量；不改 006 的 DDL；不改磁盘格式；不登记 ISS30 / ISS31 的台账关闭。

**后续任务边界**

- 授权边界：无。
- 能力边界：无。
- 停止边界：本 Cycle 被接受后才展开 Iteration 008。Act 不创建下一 Iteration。
- 不在本 Cycle：T11–T16；ISS30 / ISS31 的台账指针；006 的两个未登记候选。

## Act Response

**状态**: `reported`
**Cycle**: Iteration 007 / Cycle 000（`initial`）　**Task**: T9 + T10
**Act 执行日期**: 2026-10-26

Plan Context 保持 `ready`。本文件没有 Plan Review。没有下一 Cycle，没有下一 Iteration。

### 接手时的实际状态

- Plan Context `ready`，Review Result 仍是 `pending`，没有后继 Cycle，没有既有 Act Response。
- 从当前工作区实施，没有回退 Iteration 000–006。

### RED / GREEN

T9 先行测试 `failed_flush_then_evict_keeps_the_new_bytes` 先于实现运行。失败点是断言「写盘失败后池内仍是新字节」，不是编译失败。实现后同一命令通过。`failed_write_keeps_page_dirty` 在实现后仍通过。

T10 先行测试在 `tests/write_failure_state_test.rs`，先于失败集合的实现运行：

| 用例 | 变更前 |
|---|---|
| `anchor_is_promoted_after_its_page_is_rewritten` | 失败。释放页是 `[2, 3]`，新建表拿到 `4` 和 `5` |
| `anchor_stays_unpromoted_while_its_page_is_failed` | 通过。库级失败编号同样挡住提升，按计划不作 RED 载体 |
| `unrelated_page_failure_does_not_block_a_later_release` | 通过。这次完成编号并不小于更早的失败编号，按计划只作对照 |

实现后这三条都通过。

### 实际改动

T9：

| 位置 | 改动 |
|---|---|
| `PageFrame::write_in_flight` | 内存标记。不进页字节，不进文件头 |
| `claim_dirty_for_write` | 已在途则本轮不发写、不清脏；否则复制字节、置在途、清脏 |
| `flush_all` / `flush_all_except` / `flush_pages` | 经上面的领取路径发写。成功只清在途标记；失败由 `restore_dirty_from` 交还脏标记并清在途 |
| `evict_one` | `write_in_flight` 与 `ref_count > 0` 同样放回 clock。自己的写失败不 `remove`，交还脏标记并返回错误。自己的写成功后若仍脏，不 `remove`，放回 clock |

闸门存储 `GatedWriteStorage` 只在 `buffer_pool.rs` 的 `#[cfg(test)]` 里。

T10：

| 位置 | 改动 |
|---|---|
| `FileStorage::failed_pages` | 替换 `last_failed_write`。失败的 `write_page` 放入该页，成功的 `write_page` 移出该页。写失败不再消耗 `release_id` |
| `PendingRelease` | 待释放项带上这次 `complete_free_release` 的锚点页号。空列表不因失败集合拒绝 |
| `promote_free_releases` | 仍要求 `id < limit`。任一锚点仍在 `failed_pages` 则该项留下 |
| `AsyncStorage::complete_free_release`、`BufferPool`、`SyncPageLoader` | 增加 `anchors` 参数 |
| `Catalog::delete_table` / `delete_from_chain` | 返回实际改过的页 |
| `update_table_tail` / `update_table_root` / `update_unique_index_root` / `update_secondary_index_root` / `remove_secondary_index` | 返回 `update_field_in_chain` 已经给出的页 |
| `sync_root_to_catalog` | 返回目录更新所在页 |
| `free_overflow_chain` | 增加锚点参数。失败清理传空；`delete_tuple_from_data_page` 传正在删槽的数据页 |
| 调用点 | `drop_table` 用目录删除页；`truncate_table` 的表头释放用表头页加尾指针页，旧树释放用目录 root 更新页，失败清理传空；`DROP INDEX` 用目录摘除页；btree 合并用 `parent_id`；缩根用目录 root 页；恢复期旧树释放用目录 root 更新页；无锚点写的路径传空 |

`tests/release_durability_precondition_test.rs` 的同会话断言改为：checkpoint 之后新建表拿到被释放页之一。`overflow.rs` 里直接调用 `complete_free_release` 的测试补了空锚点，期望未改。

### 与计划的偏差

- `evict_one` 自己的写盘失败时，页留在池内，同时也放回 clock。不放回的话，这一帧占着容量，之后的驱逐选不中它。
- 定向回归里的目标名是 `drop_persistence_test`。计划写的 `drop_persistence_boundary_test` 不是 cargo 目标名；仓库里对应的文件是 `tests/drop_persistence_test.rs`。
- `release_not_promoted_when_anchor_write_not_durable` 重开后的断言也改了。同会话的新建表已经拿走被释放页，这些页登记给了那张表，重开后不能再要求下一张表同时拿到它们。重开断言改为：`t` 不在，`u` 的行和页号仍在，下一张表不再拿到 `u` 的页。
- T10 夹具在 `DROP` 前先成功刷盘，使随后失败的 `flush_all` 打在目录删除写下的页上。否则失败可能落在非锚点页，`anchor_stays_unpromoted_while_its_page_is_failed` 锁不住「锚点仍在失败集合」。

### 自检

Gate 3：T9 的在途页不被当干净页移出，失败后新字节仍在池内且错误上抛。T10 的失败按页记录，锚点仍失败时不提升，成功重写后提升。`id < limit`、刷盘失败 discard、空锚点不看失败集合都还在。帧锁不跨 `.await`。磁盘格式未改。没有产品暂停钩子，没有身份证明或判定层。

计划范围内没有未解决的 Critical 或 Important。

遗留 Minor：`release_not_promoted_when_anchor_write_not_durable` 这个名字仍描述修订前的 R2。计划点名保留该用例并改断言，没有要求改名。

定向回归第一次跑到 `release_not_promoted_when_anchor_write_not_durable` 的重开断言失败（新建表拿到页 `4`、`5`，而旧断言要求同时拿到 `[2, 3]`）。按上面的偏差改完断言后，整批重新跑过。

### 验证

两步串行，批内 `--test-threads=1`。没有同时跑，没有无过滤全量，没有用默认并行度。

1. `cargo test --test drop_persistence_test --test free_page_lifecycle_test --test release_durability_precondition_test --test write_failure_state_test -- --test-threads=1`

   `drop_persistence_test` 11 passed；`free_page_lifecycle_test` 12 passed；`release_durability_precondition_test` 2 passed；`write_failure_state_test` 3 passed。0 failed。

2. `cargo test --lib -- --test-threads=1`

   421 passed，0 failed。

触碰的文件已 `rustfmt`。没有跑全量。

### Persisted Evidence

None required。

### Experience Candidates

None。

### Issue 候选

None。

### 未解决

无阻塞项。Minor 见自检。Iteration 007 未标完成，等待 Plan Review。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读了 `PageFrame::write_in_flight`、`claim_dirty_for_write`、`evict_one`、三处 flush、`restore_dirty_from`、`finish_write_in_flight`、`FileStorage::failed_pages`、`write_page`、`promote_free_releases`、`complete_free_release`，以及计划点名的锚点调用点。`last_failed_write` 在源码中已不存在。`src/wal/checkpoint.rs` 不在本轮工作区改动里：刷盘成功才 `promote_free_releases`，失败走 `discard_free_releases`，零工作早退仍是同一次提升。帧锁在 `write_page` 的 `.await` 之前释放。`write_in_flight` 与 `failed_pages` 不进页字节、不进文件头。闸门存储 `GatedWriteStorage` 只在 `buffer_pool.rs` 的 `#[cfg(test)]` 里。

A29 / A30：`failed_flush_then_evict_keeps_the_new_bytes` 用容量 1 的池，在 `write_page` 已进入、尚未返回时再取另一页。失败后按原页号取回的是新字节，存储里仍是旧字节，`flush_all` 返回错误。若页已被按干净页移出，取回的会是存储上的旧字节。

A31 / A32：失败只把本次 `write_page` 的页号放入集合，该页成功写入后移出。`promote_free_releases` 仍要求 `id < limit`；锚点列表为空不看失败集合；任一锚点仍在集合内则该项留下。`drop_table` 用 `delete_table` 返回的目录页，`truncate_table` 用表头页加尾指针页、旧树用目录 root 页，失败清理与回填失败传空，`DROP INDEX` 用目录摘除页，btree 合并用 `parent_id`，缩根和恢复期旧树用目录 root 更新页，`delete_tuple_from_data_page` 传正在删槽的数据页。

A33：`failed_write_keeps_page_dirty` 仍在 `release_durability_precondition_test` 里，断言失败后仍脏、成功刷盘后不再脏、重开读到新行。`release_not_promoted_when_anchor_write_not_durable` 的同会话断言改为 checkpoint 之后新建表拿到被释放页之一；重开断言改为 `t` 不在、`u` 的行和页号仍在、下一张表拿不到 `u` 的页。这与修订后的 R2，以及「提升后的页在重开时不把仍被目录登记的页判为可分配」一致。

偏差分类：

- `evict_one` 自己的写失败时把页放回 clock。页留在池内且错误上抛，与 A30 一致。不放回会占住容量。非阻塞。
- 计划里的 cargo 目标名 `drop_persistence_boundary_test` 不是目标名。仓库文件是 `tests/drop_persistence_test.rs`，Act 跑的是 `drop_persistence_test`（11 passed）。这是计划错误，执行对准了 A33 要的那组用例。非阻塞。
- 重开断言的改写是修订后 R2 的必要结果：同会话的新建表已经拿走被释放页，不能再要求重开后的下一张表同时拿到它们。非阻塞。
- T10 夹具在 `DROP` 前先成功刷盘，使随后失败的 `flush_all` 打在目录删除写下的页上。观察入口仍是真实 `Database::open`、`DROP`、`flush_all`、`promote_free_releases`、`CREATE`。非阻塞。

非阻塞 Minor：`release_not_promoted_when_anchor_write_not_durable` 这个名字仍描述修订前的 R2。计划点名保留该用例并改断言，没有要求改名。T9 要求的「写盘成功且等待期间又被改脏则新内容留在池内」已写在 `evict_one` 与 `finish_write_in_flight` 里，验收表没有单独一行，也没有单独用例。

验证沿用 Act Response（2026-10-26），本审查没有重跑。沿用条件：报告的命令、计数与当前代码一致；没有「结论产生之后代码又被改过」的证据；输出计数与 006 的 `--lib` 420 相比多 1，对得上新增的那条池单测。首跑失败在改重开断言之前，改完后整批重跑通过，Act 已记下首跑失败。

分批核对：一批 4 个目标，名字典序，批内 `--test-threads=1`：`drop_persistence_test`、`free_page_lifecycle_test`、`release_durability_precondition_test`、`write_failure_state_test`。然后单独 `cargo test --lib -- --test-threads=1`。两步串行。没有无过滤全量，没有默认并行度。结果：11 / 12 / 2 / 3 passed，0 failed；`--lib` 421 passed，0 failed。

**Evidence**

- 代码：`src/storage/page_frame.rs` 的 `write_in_flight`；`src/storage/buffer_pool.rs` 的 `claim_dirty_for_write`、`evict_one`、`flush_all`、`flush_all_except`、`flush_pages`、`finish_write_in_flight`；`src/storage/file_storage.rs` 的 `failed_pages`、`write_page`、`promote_free_releases`；`tests/write_failure_state_test.rs`；`tests/release_durability_precondition_test.rs`；`src/storage/buffer_pool.rs` 的 `failed_flush_then_evict_keeps_the_new_bytes`。
- 验证来源：本 Cycle Act Response「验证」节。命令与计数见 Findings。本审查未重跑。

**Follow-up Decision**

A29–A33 都有对应代码和已运行的测试。偏差不改变验收条件，也不需要新的执行依据。当前 Cycle 不返工。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/008-dump-restore-contract/000-initial.md`
