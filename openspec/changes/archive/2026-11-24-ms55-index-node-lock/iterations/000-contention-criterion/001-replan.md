# Iteration 000 / Cycle 001: 保持现有 write_gate

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: 父 Cycle 没有可交给 Act 的任务。本 Cycle 把「保持现有 `write_gate`、不加节点锁」写成可观察的 T1。不补耗时比例或等待次数。
- Inherited scope: `proposal.md` 的场景 1 至 7。`design.md` 的 D1 与 D4。`openspec/specs/index-write-side-atomicity/spec.md` 的四条要求，本 Cycle 不改规格。M03 的 32 字节 Key。M08 的原子根页 id。M13 的 `spawn_blocking` 写单元。M17 的页帧锁。
- Excluded scope: 节点锁实现，Semaphore，latch coupling，crabbing，耗时比例，等待次数，新的测量测试，暂停夹具，身份证明，索引页格式，短于 32 字节的 Key，Tag 合并，脏页批量写，io_uring，代价模型，变长键，全量测试，合并 master，改写 `000-initial.md` 的 Plan Context。

**Objective**

Act 结束后，`src`、`tests`、`Cargo.toml` 相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 仍无差异。每个 `IndexManager` 仍有一把 `write_gate: Arc<Mutex<()>>`。写路径仍在 `spawn_blocking` 里取得它。`search`、`scan_all`、`scan_range` 仍不取得它。不加节点锁。

**Background**

父 Cycle 是 `iterations/000-contention-criterion/000-initial.md`。它的 Plan Context 状态为 `draft`，Gate 2 为 BLOCKED，原因是判据写不出来。该 Plan Context 不可改写。父 Cycle 没有 Act。它自己的目标（写不出判据就停在 draft、不交给 Act）已经达到。

本次 replan 改变的是本 Iteration 的验收边界。goal 原文是「开始计划实施ms55，要求遵守各种规范」。用户原话是「选择通用最优的处理方式就好」。这句原话批准在 goal 范围内按公共规则的全局最优解条款做选择。这是批准，不是跳过。

所选方案：不加节点锁，不改 `write_gate`，不新写耗时比例或次数。保持写单元原子性、根发布、取消和回滚。同样满足 MS55 的方案里，优先复用已有实现。现有 `write_gate: Arc<Mutex<()>>` 就是索引写侧的锁。两次落在不同叶子上的插入若在时间上重叠，后一次会停在 `write_gate.lock()`。这次等待是这把锁正在把同一索引的结构写串起来，规格 `index-write-side-atomicity` 要求这种串行。它不是另一条需要新锁才能解释的争用。MS55 允许未过门槛时保持现有锁。长期记录在 `design.md` 的 D4。

被替代方案，Act 不得实施：

- 把这一次必然等待当成争用已证实并加节点锁。这种等待在重叠写时一定出现，里程碑里「未过门槛则保持现有锁」会走不到。
- 新写一个耗时比例或等待次数。MS55、`index-write-side-atomicity` 和这次等待的证明里都没有这个数。

**Investigation Facts**

- Starting point: 工作区 `/home/daivy/projects/RTsql-changes/2026-11-24-ms55-index-node-lock`，分支 `changes/2026-11-24-ms55-index-node-lock`。写入本 Cycle 之前的 HEAD 是 `170cbc6f3b8dba6305ea82f7f5fb64a029175eff`。生产代码基点是 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`。规划时执行 `git diff --stat 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml`，没有输出。没有合并 master。不要读写 `/home/daivy/projects/RTsql`：那个目录在 master，并有与本变更无关的未提交文件 `.agents/memory/improvements.md`。不要 rebase。格式结论已经接受，不重测槽字节：内部节点槽 36 字节，数据槽版本头 22 字节紧贴 tuple，`removable=0`，`gap=0`。本分支源码仍是 `MAX_KEY_LEN` 为 32（`src/storage/page_format/key.rs`），`RowId::SIZE` 为 6（`src/storage/page_format/row_id.rs`），分隔条目为 `MAX_KEY_LEN + 4`，叶条目为 `MAX_KEY_LEN + RowId::SIZE`，`VersionHeader::SIZE` 为 22（`src/transaction/version_chain.rs`），`FORMAT_VERSION` 为 1（`src/storage/file_header.rs`）。规划时没有跑 `cargo test`。
- Current-State Evidence: `IndexManager.write_gate` 的类型是 `Arc<Mutex<()>>`，字段在 `src/storage/btree/index_manager.rs` 第 165 行。`new` 第 246 行和 `from_root` 第 265 行各自执行 `write_gate: Arc::new(Mutex::new(()))`。文件里没有第三处构造。取得锁的位置都在 `tokio::task::spawn_blocking` 闭包开头，守卫活到闭包结束：`insert` 第 453–455 行，`delete` 第 500–501 行，`delete_exact` 第 559–560 行，`update` 第 731–732 行，`find_key_by_row_id` 第 761–762 行。`insert` 的闭包顺序是 `BTree::insert`、`pause_at(AfterStructureChangeBeforeRootPublish)`、按需 `root_page_id.store`、`row_to_key.write().insert`；`sync_root_to_catalog` 在 `spawn_blocking` 的 `.await` 之后。`delete` 的闭包顺序是查找、`BTree::delete`、同一暂停点、按需发布根、按需删除反向映射；目录同步同样在闭包外。`delete_exact` 调用 `BTree::delete_exact` 后按需删映射，不发布根。`update` 调用 `BTree::update` 后更新映射，不发布根。`find_key_by_row_id` 取得闸门后读 `row_to_key`。每一处都是闭包外先 `clone`，闭包内再 `lock`：`insert` 第 450 行，`delete` 第 497 行，`delete_exact` 第 557 行，`update` 第 729 行，`find_key_by_row_id` 第 760 行。全文件的 `write_gate.lock` 只有这五处。`search`（第 330 行）、`scan_all`（第 580 行）、`scan_range`（第 643 行）只 `load(Ordering::Acquire)` 根页 id，函数体内没有 `write_gate`。`search_all`（第 338 行）和 `collect_all_pages`（第 784 行）同样不取得它。`BTree::insert_into_page_step`（`src/storage/btree/btree.rs`）在叶子 `insert` 返回 `Ok` 时返回 `Ok(None)`，这次不写父页，也不换根。页帧是 `Arc<Mutex<PageFrame>>`（`src/storage/page_frame.rs`）。`pipeline.rs` 没有 `Mutex`。`Database` 为 `Clone`（`src/database.rs`）。`Server::run` 对每个连接 `tokio::spawn`（`src/network/server.rs`）。
- Code and Critical Path: 语句入口是 `Database::execute_sql`。主键预检在 `src/executor/insert.rs` 调用 `index_manager.search`，写入调用 `index_manager.insert`；唯一索引走各自的 `insert`，二级索引走 `insert`。删除在 `src/executor/delete.rs` 调用 `search`、`scan_all` 和 `index_manager.delete`，二级索引走 `delete_exact`。更新在 `src/executor/update.rs` 按列变化调用 `update`、`delete` 或先 `delete` 再 `insert`，二级索引走 `delete_exact` 与 `insert`。回滚与版本恢复在 `src/transaction/manager.rs` 调用 `find_key_by_row_id`、`insert`、`delete`、`update`、`delete_exact`。这些调用进入目标 `IndexManager` 自己的阻塞闭包。闸门不跨索引共享。重复键预检走 `search`，不取得 `write_gate`。本任务不修改这些调用者。`arm_write_pause` 只在测试装载后把已持锁的写单元停在根发布之前；未装载时写路径多一次未命中检查。本任务不改这个暂停点，也不把它的停留时长当成门槛。

**Task Definitions**

### T1: 保持现有 write_gate，产品代码停在起点

- Requirement/Scenario: MS55 Outcome 的「未过门槛的保持原格式和现有锁」，以及保留写单元原子性、根发布、取消和回滚。`proposal.md` 场景 1 至 7。规格「同一索引的结构写侧串行化」继续由现有 `write_gate` 承担。
- Depends on: None
- Targets: `src/storage/btree/index_manager.rs` 的 `IndexManager.write_gate`、`IndexManager::new`、`IndexManager::from_root`、`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id`、`search`、`scan_all`、`scan_range`。`src/`、`tests/`、`Cargo.toml` 整体相对基点不变。
- Current behavior: 每个 `IndexManager` 有一把 `write_gate: Arc<Mutex<()>>`，只在 `new` 和 `from_root` 里 `Arc::new(Mutex::new(()))`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 闭包开头 `write_gate.lock().unwrap()`，持有到闭包结束。`insert` 与 `delete` 在该闭包内完成结构变更、暂停点、按需根页 id 发布和反向映射更新；目录根同步在闭包外。`delete_exact` 与 `update` 在闭包内完成结构变更和反向映射，不发布根。`find_key_by_row_id` 在闭包内读映射。`search`、`scan_all`、`scan_range` 不取得 `write_gate`。`src`、`tests`、`Cargo.toml` 相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 无差异。
- Required behavior: 与 Current behavior 相同。不加节点锁。不改 `write_gate` 的类型、创建点和取得点。不新写耗时比例或等待次数。不新增暂停夹具。写单元原子性、根发布、取消和回滚保持。
- Preserve: Key 保持 32 字节。分隔条目保持 `MAX_KEY_LEN + 4`。叶条目保持 `MAX_KEY_LEN + RowId::SIZE`。`VersionHeader::SIZE` 保持 22。`FORMAT_VERSION` 保持 1。根页 id 仍是 `AtomicU64`，异步读路径仍用 `Ordering::Acquire` 加载。异步路径仍不持 `std::sync::Mutex` 跨 `.await`。页帧锁仍是单次读页或改页的 `Arc<Mutex<PageFrame>>`。`search_all` 与 `collect_all_pages` 仍不取得 `write_gate`。`find_key_by_row_id` 仍在看到落定映射之后才返回。不同 `IndexManager` 的 `write_gate` 仍不共享。
- Forbidden: 修改 `src/`、`tests/`、`Cargo.toml`。加节点锁，或把页帧锁扩成跨页的结构写锁。改 `write_gate` 的类型、粒度或共享范围。新增耗时比例、等待次数、测量打印、暂停夹具或身份证明。为了留下测量而新增测试框架、夹具或判定层。把 `sync_root_to_catalog` 移入闭包，或把根发布、反向映射移出闭包。合并 master，rebase，读写 `/home/daivy/projects/RTsql`，改写 `000-initial.md` 的 Plan Context。创建 `evidence/` 目录。创建下一 Cycle 或下一 Iteration。
- First test: 变更前观察，不新增测试。计划时 `git diff --stat 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml` 没有输出，上列符号与 Current behavior 一致。当前已经是目标状态，没有 RED。Act 不得只引用这次计划观察作为完成证据。
- GREEN condition: Act 不修改 `src/`、`tests/`、`Cargo.toml`。Act 自己执行 `git diff --exit-code 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml`，退出码为 0。Act 自己阅读上列符号，仍与 Current behavior 一致。
- Verification: 在本工作区执行上面的 `git diff --exit-code`。通过是退出码 0 且没有 diff 文本。失败是退出码非 0，表示这三处相对基点有差异；停止并返回 Plan，不要改产品代码把 diff 抹平。符号阅读核对：`write_gate` 字段类型、`new` 与 `from_root` 的构造、五处 `lock` 都在对应 `spawn_blocking` 闭包内、`search` 与 `scan_all` 与 `scan_range` 没有 `write_gate.lock`。任一处与 Current behavior 不一致，停止并返回 Plan，不要改代码去凑这份描述。不要跑 `cargo test`，不要跑 `tests/index_write_atomicity_test.rs`，不要新增测试。空 diff 加上符号阅读已经能判定生产函数在改动前后都是基点上的函数。不要把同一条 diff 再跑一遍来增加信心。
- Stop when: 开工时该 diff 的退出码已经不是 0。符号与 Current behavior 不一致。要证明「保持」必须新增测试、计数器、暂停夹具或节点锁。出现需要改接口、错误语义或锁粒度的实质问题。

**Invariants**

同一索引的结构变更、根页 id 发布和反向映射更新仍在同一个 `spawn_blocking` 闭包里，并由该索引的 `write_gate` 串行。取消不切开这个闭包。`find_key_by_row_id` 仍读到写单元落定后的映射。`search`、`scan_all`、`scan_range` 仍不取得 `write_gate`。格式宽度与 `FORMAT_VERSION` 不变。M08 与 M13 不变。

**Acceptance**

1. `git diff --exit-code 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml` 的退出码为 0。这覆盖场景 1 的格式保持，以及场景 2 的写单元、取消和回滚代码保持。
2. `IndexManager.write_gate` 仍是 `Arc<Mutex<()>>`。`new` 与 `from_root` 各自构造一把。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 仍在 `spawn_blocking` 闭包内取得它并持有到闭包结束。这覆盖场景 4、场景 5、场景 6，以及规格「同一索引的结构写侧串行化」。
3. `search`、`scan_all`、`scan_range` 仍不调用 `write_gate.lock`。这覆盖场景 3。
4. 产品代码里没有新的节点锁、耗时比例、等待次数或为测量而加的暂停夹具。这覆盖场景 7 和 D4。

**Verification**

场景 1 至 7 都由 T1 的同一条 `git diff --exit-code` 和 `index_manager.rs` 符号阅读判定。不经过新的测试入口。不运行全量套件。不把 `tests/index_write_atomicity_test.rs` 的并发插入当成这次决定的证据：那个测试断言键仍可查、完整性检查无 Error，不断言等待时间或等待次数，而且本 Cycle 不改它。

Persisted Evidence：none。命令、退出码、读到的符号写入 Act Response 即可。

**Gate 2 Readiness**

PASS。检查项全部通过。没有 BLOCKED。没有 SKIPPED。

用户批准来自 goal「开始计划实施ms55，要求遵守各种规范」，以及原话「选择通用最优的处理方式就好」。这是批准，不是跳过。设计已闭合为 D4：行为保持现有锁，不改接口和错误语义。RTM 没有 Missing，没有 Simplified。T1 的先行观察是上述 diff 与符号阅读，不另建测试框架。

**Risks and Notes**

`find_key_by_row_id` 的等待用来排除写单元中间态。不要把它当成可拆掉的树等待。目录根同步在闸门之外，不要移动这个边界。I025 的「每次操作 lock 整棵树」与 `search`、`scan_all`、`scan_range` 不一致；这个差别已经记在 Current-State Evidence 里，不构成加锁方案。`arm_write_pause` 的停留由测试放行决定，不是门槛。

行号是规划时读到的位置。Act 不改这个文件时行号不变。行号与符号对不上，而 diff 又是空的，说明本文件的描述有误，返回 Plan。

## Act Response

- Status: reported

实际改动：只填写本 Cycle 的 Act Response。`src/`、`tests/`、`Cargo.toml` 没有改。没有新增测试，没有节点锁，没有耗时比例或等待次数。

`IndexManager.write_gate` 的类型是 `Arc<Mutex<()>>`，`Mutex` 来自 `std::sync`（`src/storage/btree/index_manager.rs` 第 17、165 行）。`IndexManager::new` 第 246 行和 `IndexManager::from_root` 第 265 行各自写 `write_gate: Arc::new(Mutex::new(()))`。文件里没有第三处构造。

取得它的函数是 `insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id`。都是闭包外 `clone`，`spawn_blocking` 闭包开头 `write_gate.lock().unwrap()`，守卫持有到闭包结束。

- `insert`：clone 第 450 行，闭包第 453 行，lock 第 455 行。闭包内顺序是 `BTree::insert`、`pause_at(AfterStructureChangeBeforeRootPublish)`、按需 `root_page_id.store`、`row_to_key.write().insert`。`sync_root_to_catalog` 在 `.await` 之后，第 482 行。
- `delete`：clone 第 497 行，闭包第 500 行，lock 第 501 行。闭包内是查找、`BTree::delete`、同一暂停点、按需发布根、按需删除反向映射。目录同步在闭包外，第 529 行。
- `delete_exact`：clone 第 557 行，闭包第 559 行，lock 第 560 行。`BTree::delete_exact` 之后按需删映射，不发布根。
- `update`：clone 第 729 行，闭包第 731 行，lock 第 732 行。`BTree::update` 之后更新映射，不发布根。
- `find_key_by_row_id`：clone 第 760 行，闭包第 761 行，lock 第 762 行。闭包内读 `row_to_key`。

不取得它的函数是 `search`（第 330 行）、`scan_all`（第 580 行）、`scan_range`（第 643 行）。三处只 `load(Ordering::Acquire)` 根页 id，函数体内没有 `write_gate.lock`。`search_all`（第 338 行）和 `collect_all_pages`（第 784 行）同样不取得它。全文件 `write_gate.lock` 只有上述五处。

与计划的偏差：无。

自检没有发现。Gate 3 通过：符号与 Current behavior 一致，产品代码没有计划外修改。

验证：工作目录 `/home/daivy/projects/RTsql-changes/2026-11-24-ms55-index-node-lock`，分支 `changes/2026-11-24-ms55-index-node-lock`，执行时 HEAD 是 `703eac55e761e472a216ca060378f3b08c100dbc`。`git diff --exit-code 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml` 退出码 0，没有 diff 文本。符号阅读见上。没有跑 `cargo test`，没有跑 `tests/index_write_atomicity_test.rs`。Gate 4 通过。

Experience Candidates: None

未解决问题：无。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读 `src/storage/btree/index_manager.rs`，并阅读 Act 提交 `9858324efd7da59c5fd1c8b03d06b9af360f6772` 的 diff。该提交只填写本 Cycle 的 Act Response，没有改写 Plan Context，也没有改 `src/`、`tests/`、`Cargo.toml`。读到的 `write_gate` 类型、两处构造、五处 `lock` 和三处读路径与 Act Response、T1 的 Current behavior 一致。没有可归入 Plan 遗漏、Plan 错误、Act 偏离、现状变化或新证据的偏差。没有阻塞 Acceptance 的发现，也没有 Minor finding。

**Evidence**

沿用 Act Response 的验证：在 HEAD `703eac55e761e472a216ca060378f3b08c100dbc` 执行 `git diff --exit-code 55efdb0fb872a1d6406891128cf8d4e0f29c8997 -- src tests Cargo.toml`，退出码 0，没有 diff 文本。这条结论仍然覆盖当前树。`703eac55` 到当前 HEAD `9858324` 之间，`src`、`tests`、`Cargo.toml` 的 `git diff --stat` 为空。本分支与 `master` 的 merge-base 仍是 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`。调度器在本次 Review 前对同一条 `git diff --exit-code` 看到退出码 0。本次没有重跑这条命令，也没有跑 `cargo test`。Persisted Evidence 为 none，不因缺少 `evidence/` 提出问题。

本次读到的符号，文件 `src/storage/btree/index_manager.rs`：

- `write_gate` 的类型是 `Arc<Mutex<()>>`。`Mutex` 来自 `std::sync`，导入在第 17 行，字段在第 165 行。
- `IndexManager::new` 第 246 行和 `IndexManager::from_root` 第 265 行各自写 `write_gate: Arc::new(Mutex::new(()))`。文件里没有第三处构造。
- 取得它的函数是 `insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id`。都是闭包外 `clone`，`spawn_blocking` 闭包开头 `write_gate.lock().unwrap()`，守卫持有到闭包结束。全文件 `write_gate.lock` 只有这五处。
  - `insert`：clone 第 450 行，闭包第 453 行，lock 第 455 行。闭包内顺序是 `BTree::insert`、`pause_at(AfterStructureChangeBeforeRootPublish)`、按需 `root_page_id.store`、`row_to_key.write().insert`。`sync_root_to_catalog` 在第 482 行，位于 `.await` 之后。
  - `delete`：clone 第 497 行，闭包第 500 行，lock 第 501 行。闭包内是查找、`BTree::delete`、同一暂停点、按需发布根、按需删除反向映射。目录同步在第 529 行，位于闭包外。
  - `delete_exact`：clone 第 557 行，闭包第 559 行，lock 第 560 行。`BTree::delete_exact` 之后按需删映射，不发布根。
  - `update`：clone 第 729 行，闭包第 731 行，lock 第 732 行。`BTree::update` 之后更新映射，不发布根。
  - `find_key_by_row_id`：clone 第 760 行，闭包第 761 行，lock 第 762 行。闭包内读 `row_to_key`。
- 不取得它的函数是 `search`（第 330 行）、`scan_all`（第 580 行）、`scan_range`（第 643 行）。三处只 `load(Ordering::Acquire)` 根页 id，函数体内没有 `write_gate.lock`。`search_all`（第 338 行）和 `collect_all_pages`（第 784 行）同样不取得它。

这覆盖 Acceptance 1 至 4：产品树相对基点无差异，写路径仍在闭包内持有现有闸门，三条读路径仍不取得它，产品代码里没有新的节点锁、耗时比例、等待次数或为测量而加的暂停夹具。

**Follow-up Decision**

四条既有 Acceptance 都已满足。继续改代码不是达到本 Iteration 原有 Acceptance 的必要条件。Iteration Plan 保持不变，`tasks.md` 没有后续 Iteration。接受本 Cycle。不要求当前 Cycle 修复，不创建 rework 或 replan Cycle。父 Cycle `000-initial.md` 的 Plan Context 仍是 `draft`，本次不改写。本轮范围外没有需要报告的实质缺陷。

**Next Cycle**

None

**Next Iteration**

None
