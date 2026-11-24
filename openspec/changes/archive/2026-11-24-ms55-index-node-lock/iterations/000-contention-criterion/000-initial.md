# Iteration 000 / Cycle 000: 写下写争用判据或停住

## Plan Context

- Status: draft
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 无
- Inherited scope: `proposal.md` 的场景 1 至 7，`design.md` 的 D1、D2、D3。`openspec/specs/index-write-side-atomicity/spec.md` 的四条要求。M03 的 32 字节 Key。M08 的原子根页 id。M13 的 `spawn_blocking` 写单元。M17 的页帧锁。
- Excluded scope: 节点锁实现，Semaphore，latch coupling，crabbing，索引页格式，短于 32 字节的 Key，Tag 合并，脏页批量写，io_uring，代价模型，变长键，全量测试，合并 master。

**Objective**

为「写争用被测量证实」写出一条能走向加锁、也能走向保持现有 `write_gate` 的判据。写不出来时，本 Cycle 保持 `draft`，产品代码保持起点。

这个 Objective 没有达到。判据写不出来。Act 不得开始。

**Investigation Facts**

- Starting point: 生产代码基点 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`。本分支 `changes/2026-11-24-ms55-index-node-lock` 在计划写入前的 HEAD 是 `97b648ae069a274c40b54399db0647d0ecdc8782`。没有合并 master。格式变更的 Iteration 000 在 master 归档 Cycle 里 Review Result 为 `accepted`，Next Iteration 为 None。执行记录：`index_pages=4 separator_len=36 leaf_len=38 removable=0`；`header_len=22 tuple_tag=1 gap=0 slot_len=31`。本分支不重跑那两条测试。本分支源码仍是 `MAX_KEY_LEN` 为 32，叶 `entry_size` 为 `MAX_KEY_LEN + RowId::SIZE`，分隔 `entry_size` 为 `MAX_KEY_LEN + 4`，`VersionHeader::SIZE` 为 22。没有可沿用的 Explorer 分析。规划时没有跑 `cargo test`。
- Current-State Evidence: `IndexManager.write_gate` 的类型是 `Arc<Mutex<()>>`（`src/storage/btree/index_manager.rs`）。`new` 与 `from_root` 各自 `Arc::new(Mutex::new(()))`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 闭包的开头 `write_gate.lock().unwrap()`，闭包结束才释放。`insert` 在持锁期间完成 `BTree::insert`、`pause_at(AfterStructureChangeBeforeRootPublish)`、按需 `root_page_id.store`、`row_to_key.write().insert`。目录根同步 `sync_root_to_catalog` 在闭包外。`delete` 同样持锁完成查找、删除、暂停、按需发布根和删除反向映射。`delete_exact` 持锁调用 `BTree::delete_exact` 并按需删映射，合同是不改根。`update` 持锁调用 `BTree::update` 并更新映射，不发布根。`find_key_by_row_id` 持锁后只读映射，避免看见写单元的中间态。`search`、`search_all`、`scan_all`、`scan_range`、`collect_all_pages` 只 `load(Ordering::Acquire)` 根页 id，源码里没有 `write_gate.lock`。`index_manager.rs` 没有诊断调用。`BTree::insert_into_page_step` 在叶子 `insert` 返回 `Ok` 时返回 `Ok(None)`，这次不写父页，也不换根。页帧是每页一个 `Arc<Mutex<PageFrame>>`。`Database` 为 `Clone`，`table_manager` 与 `TableMeta.index_manager` 为 `Arc`。`Server::run` 对每个连接 `tokio::spawn`，处理函数调用 `Database::execute_sql`。`src/pipeline.rs` 没有 `Mutex`。两个连接可以同时进入同一索引的写单元。
- Code and Critical Path: 语句入口是 `Database::execute_sql` → `pipeline::execute`。主键写入在 `src/executor/insert.rs` 调用 `index_manager.insert`，删除在 `src/executor/delete.rs` 调用 `index_manager.delete`，更新在 `src/executor/update.rs` 按列变化调用 `update`、`delete` 或先 `delete` 再 `insert`。唯一索引走各自的 `IndexManager::{insert,delete,update}`。二级索引走 `insert` 与 `delete_exact`（`src/executor/delete.rs`、`update.rs`、`upsert.rs`）。回滚与版本恢复在 `src/transaction/manager.rs` 调用 `find_key_by_row_id`、`insert`、`delete`、`update`、`delete_exact`。这些调用各自进入目标 `IndexManager` 的阻塞闭包，闸门不跨索引共享。读路径 `search`、`scan_all`、`scan_range` 不经过闸门；`insert.rs` 的重复键预检调用的是 `search`。取消和回滚的现有合同由 `tests/index_write_atomicity_test.rs` 覆盖，本 Cycle 不跑、不改这些测试。`arm_write_pause` 只在测试装载后把已持锁的写单元停在根发布之前；未装载时写路径多一次未命中检查。

**Implementation Guidance**

没有可执行任务。不要修改 `src/`、`tests/`、`Cargo.toml`。不要新增打印测试。不要把比例、次数或暂停时长写进代码或执行记录来充当门槛。不要创建下一 Iteration 的目录。

**Task Definitions**

无。

`tasks.md` 不设 T1。一条测量任务必须写明怎样从打印走到「保持 `write_gate`」或「可以计划加锁」。这个条件就是缺失的判据。交给 Act 去选一个百分比，或者看到一次 `lock` 等待就加锁，都超出本 Cycle 的执行依据。

**Invariants**

Key 保持 32 字节。分隔条目保持 `MAX_KEY_LEN + 4`。叶条目保持 `MAX_KEY_LEN + RowId::SIZE`。`VersionHeader::SIZE` 保持 22。文件格式版本保持 1。

`insert` 与 `delete` 的结构变更、根页 id 发布和反向映射更新仍在同一个 `spawn_blocking` 闭包里，并由该索引的 `write_gate` 串行。取消不切开这个闭包。`find_key_by_row_id` 仍在看到落定映射之后才返回。`search`、`scan_all`、`scan_range` 仍不取得 `write_gate`。根页 id 仍用 `AtomicU64`。异步路径仍不持 `std::sync::Mutex` 跨 `.await`。

**Acceptance**

本 Cycle 没有可交给 Act 的验收。

计划侧已经完成的只有：场景 1 至 6 的当前行为写进了上面的 Investigation Facts；场景 7 的判据确认写不出来，并记入 `proposal.md` 的「写争用的可观察判据」和 `design.md` 的 D2。产品代码、测试和锁行为保持起点。

**Verification**

无命令。规划时没有运行 `cargo test`。不把格式变更那两次已经接受的打印再跑一遍。不把 `tests/index_write_atomicity_test.rs` 的并发插入当成争用门槛：那个测试断言键仍可查、完整性检查无 Error，不断言等待时间或等待次数。

**Gate 2 Readiness**

BLOCKED。写争用的可观察判据写不出来。

MS55 的原文是：节点锁只有在写争用被测量证实后才加，并保留写单元原子性、根发布、取消和回滚。未过门槛的保持原格式和现有锁。

`index-write-side-atomicity` 要求同一索引的结构变更串行执行，两个写者不得交错修改同一索引页，不得用过期根覆盖刚发布的根。规格没有写等待多久、等待多少次、或者哪一种页面不相交才改锁。

代码能证明的等待是：每个 `IndexManager` 有自己的 `write_gate: Arc<Mutex<()>>`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 里取得它，并持有到该闭包结束。不分裂的叶插入只修改那一片叶子。两次这种插入若落在不同叶子上，而且调用在时间上重叠，后一次会停在 `write_gate.lock()`，即使两次都不修改同一页，也不发布新根。`search`、`scan_all`、`scan_range` 不取得这把锁。

把两次调用排成重叠之后，这次等待一定出现。用「观察到一次就加锁」做门槛，保持现有锁的那一支到不了。里程碑写的是未过门槛则保持现有锁，所以判据必须能走向保持。耗时比例、等待次数，以及别的里程碑里「中位数更短」那种比较，都没有出现在 MS55、这份规格或上述等待的证明里。

`arm_write_pause` 可以把已经持有闸门的写单元停住。停住的时长由测试放行决定。诊断摘要里的阶段等待也不读取 `write_gate`。这两处都不是这道门槛。

检查项里因此失败的是：设计没有闭合采纳或保持的行为差异；RTM 的「写争用」一行是 Missing；没有能建立第一个先行测试的 Task Definition。没有 SKIPPED。没有用百分比把这项改成 PASS。

**Risks and Notes**

`find_key_by_row_id` 的等待是写单元中间态的排除，将来即使有了判据，也不能把它当成可拆掉的树等待。目录根同步在闸门之外，本 Cycle 不动这个边界。不同 `IndexManager` 的 `write_gate` 不共享。I025 的「每次操作 lock 整棵树」与 `search`、`scan_all`、`scan_range` 不一致，这个差别已经记在 Current-State Evidence 里，不构成加锁方案。

## Act Response

Plan Context 为 draft。本节留待 Act。现在不得填写成 `reported`。

## Plan Review

- Review Result: replan-required

**Findings**

父 Cycle 没有 Act。Plan Context 仍是 `draft`，其中的 Gate 2 仍是 BLOCKED。该 Plan Context 不改写。

父 Cycle 自己的目标已经达到：判据写不出来，所以停在 draft，不交给 Act。偏差原因是验收边界改变。用户随后批准的选择把本 Iteration 的结果定为保持现有 `write_gate`。重叠写停在 `write_gate.lock()` 的等待，按规格要求的同一索引结构写串行处理。这需要新的 Task Definition，并重新通过 Gate 2。父 Cycle 的执行依据里没有这项任务。

没有 Act 偏离，也没有新的产品代码证据。这不是 Minor finding。

**Evidence**

- 父 Cycle Plan Context：Status 为 `draft`；Gate 2 Readiness 为 BLOCKED；Task Definitions 为无。
- 用户原话：「选择通用最优的处理方式就好」。goal：「开始计划实施ms55，要求遵守各种规范」。
- 规划时 `git diff --stat 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml` 没有输出。
- 后继 Cycle：`iterations/000-contention-criterion/001-replan.md`。

**Follow-up Decision**

父 Cycle 的 Plan Context 不能改成可执行任务。保持决定要写进新的 Task Definition，并让新 Cycle 的 Gate 2 通过。因此创建同目录的 replan Cycle，不创建 rework repair item。

**Iteration Plan Update**

Iteration 000 的任务从无改为 T1。稳定结果改为：产品代码相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 无差异，现有 `write_gate` 保持，不加节点锁。验证从「无命令」改为 `git diff` 与符号阅读。不加锁的后继 Iteration 仍然不设。

**Next Cycle**

`iterations/000-contention-criterion/001-replan.md`

**Next Iteration**

None
