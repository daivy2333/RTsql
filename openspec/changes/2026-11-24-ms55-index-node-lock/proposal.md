## Why

MS55 要求索引页的空间和锁粒度有测量。格式那一支已经结束：`2026-11-24-ms55-index-page-space` 的 Iteration 000 Review Result 为 `accepted`，Next Iteration 为 None。执行记录是 `index_pages=4 separator_len=36 leaf_len=38 removable=0`，以及 `header_len=22 tuple_tag=1 gap=0 slot_len=31`。内部节点槽保持 36 字节分隔条目，数据槽保持 22 字节版本头紧贴 tuple。生产逻辑不改格式。

本变更因此做自己的锁计划。I025 只剩下这一问：写争用被测量证实之后才加节点锁，并保留写单元原子性、根发布、取消和回滚。未过门槛则保持现有锁。

`000-initial.md` 写不出一条既能走向加锁、也能走向保持的判据，该 Cycle 的 Plan Context 保持 draft。后继 `001-replan.md` 按已批准的选择保持现有 `write_gate`。两次落在不同叶子上的插入若在时间上重叠，后一次会停在 `write_gate.lock()`。这次等待是这把锁正在把同一索引的结构写串起来。规格 `index-write-side-atomicity` 要求这种串行。因此不加节点锁，不改 `write_gate`，不新写耗时比例或等待次数。

## 依赖

依赖 `2026-11-24-ms55-index-page-space` 的格式结论，作为计划顺序。基点仍是 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`。本分支不合并 master，也不把归档文件并进来。本分支上的槽宽与那份结论一致：`MAX_KEY_LEN` 为 32，叶条目是 `MAX_KEY_LEN + RowId::SIZE`，分隔条目是 `MAX_KEY_LEN + 4`，`VersionHeader::SIZE` 为 22。

## What Changes

- 保持现有 `write_gate`。每个 `IndexManager` 仍是一把 `Arc<Mutex<()>>`。写路径仍在 `spawn_blocking` 里取得它。`search`、`scan_all`、`scan_range` 仍不取得它。
- 不加节点锁。不新写耗时比例或等待次数。不新增测量测试、暂停夹具或身份证明。
- 不改索引页格式，不缩短 Key，不合并 Tag byte，不做脏页批量写、io_uring、代价模型。不沿用变长键假设。
- 不把「一定加节点锁」写成需求。不展开加锁 Iteration。
- Act 确认 `src`、`tests`、`Cargo.toml` 相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 无差异，并阅读上述符号。判定用 `git diff` 与符号阅读。

### 场景草图

1. 格式保持。前置：上述执行记录已被接受，本分支代码的条目宽度与它一致。动作：本变更不改 `node.rs` 的条目宽度、`VersionHeader`、tuple tag 和 `FORMAT_VERSION`。结果：槽宽保持。失败边界：为了测量锁而去改槽。停止。
2. 写单元保持。前置：`openspec/specs/index-write-side-atomicity/spec.md` 仍在本分支。动作：本变更不改 `insert`、`delete`、`delete_exact`、`update` 里结构变更、根页 id 发布和反向映射的同闭包顺序。结果：取消仍不能切开这个单元；同一索引的结构写仍串行；取消或回滚仍释放该语句占用的全部键；之后的结构检查仍要求没有 Error。失败边界：把根发布或反向映射移出 `spawn_blocking` 闭包。
3. 读路径与闸门。前置：当前 `IndexManager`。动作：核对 `search`、`scan_all`、`scan_range` 是否取得 `write_gate`。结果：这三处不取得它。I025 背景里的「每次操作 lock 整棵树」不描述这三条读路径。失败边界：把这句背景当成已经复核的加锁需求。
4. 同一索引上的写者。前置：两个连接可以同时进入同一个 `IndexManager` 的 `insert`。动作：观察后进入者与 `write_gate` 的关系。结果：后进入者要等先进入者释放这把 `Mutex<()>`。这是闸门的行为，规格要求的串行就由它完成。失败边界：因为能构出一次重叠，就批准节点锁。
5. 不同索引。前置：主键、唯一索引、二级索引各自一个 `IndexManager`。动作：核对 `write_gate` 的创建点。结果：`new` 和 `from_root` 各自 `Arc::new(Mutex::new(()))`。另一棵树的写者不在这把锁上等待。失败边界：按「整库一把锁」设计测量。
6. 反向映射读。前置：写单元持有 `write_gate` 期间，反向映射还没写完。动作：`find_key_by_row_id` 也取得 `write_gate` 再读映射。结果：它读到的是写单元落定后的映射。这段等待要留下来。失败边界：把这次查找的等待当成加节点锁的证据。
7. 保持现有锁。前置：场景 1 至 6 已核对。两次落在不同叶子上的插入若在时间上重叠，后一次停在 `write_gate.lock()`。动作：按已批准的选择，把这次等待当作同一索引结构写的串行。结果：不加节点锁，不改 `write_gate`，不新写耗时比例或次数。写单元原子性、根发布、取消和回滚保持。失败边界：把这次必然等待当成争用已证实并加节点锁，或新写一个耗时比例或等待次数。

### 本次 goal 授权接受的选择

1. 格式与节点锁仍然是两个变更。格式结论只作为本变更的前提，不在这里重测槽字节。
2. 同样满足 MS55 的方案里，复用已有 `write_gate`。用户原话：「选择通用最优的处理方式就好」。这是批准，不是跳过。所选方案、理由和被替代方案在 `design.md` 的 D4。
3. Semaphore、latch coupling 和 crabbing 仍是 I025 的历史方向，不是本计划的决定。

## 保持现有锁

`000-initial.md` 已经记录：一条能走向加锁、也能走向保持的耗时比例或等待次数，在 MS55、`index-write-side-atomicity` 和现有代码里都写不出来。该 Cycle 的 Plan Context 保持 draft，不改写。

本 replan 保持现有锁。每个 `IndexManager` 的 `write_gate` 是 `Arc<Mutex<()>>`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 里取得它，并持有到该闭包结束。`search`、`scan_all`、`scan_range` 不取得它。

不分裂的叶插入只修改那一片叶子。两次这种插入若落在不同叶子上，而且调用在时间上重叠，后一次会停在 `write_gate.lock()`。这次等待是这把锁正在把同一索引的结构写串起来。规格「同一索引的结构写侧串行化」要求这种串行。MS55 允许未过门槛时保持现有锁。

因此不加节点锁，不改 `write_gate`，不新写耗时比例或等待次数。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- 无。锁的行为和索引格式都不改。`.openspec.yaml` 使用 `skip_specs: true`。

## Impact

- 代码：无。Act 确认 `src`、`tests`、`Cargo.toml` 相对 `55efdb0fb872a1d6406891128cf8d4e0f29c8997` 仍无差异。
- 引用：MS55、I025。I029 与 I030 已由格式变更消耗。M03、M08、M13、M17 保持。
- 测试：不新增测试，不跑全量。判定是 `git diff` 与 `index_manager.rs` 的符号阅读。

## 回滚

没有产品改动。去掉本变更在本分支写入的计划文件，就没有这份保持决定。已有库文件不用迁移。

## Gate 1

PASS。需求、场景和范围来自 MS55 的 Outcome、Scope、Non-goals、Split signals，以及本次 goal。这是批准，不是跳过。`000-initial` 曾把场景 7 记为「不发明数字、计划停在 draft」。本次 replan 用用户原话「选择通用最优的处理方式就好」把场景 7 定为保持现有 `write_gate`。没有 SKIPPED 项。没有需求裁剪。

## Gate 2

`iterations/000-contention-criterion/000-initial.md` 的 Plan Context 仍是 draft，其 Gate 2 仍是 BLOCKED。该 Plan Context 不改写，不交给 Act。

`iterations/000-contention-criterion/001-replan.md` 的 Gate 2 为 PASS。保持现有 `write_gate` 的任务和直接观察写在该 Cycle。这是批准，不是跳过。没有 SKIPPED 项。
