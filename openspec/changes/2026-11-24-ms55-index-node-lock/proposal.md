## Why

MS55 要求索引页的空间和锁粒度有测量。格式那一支已经结束：`2026-11-24-ms55-index-page-space` 的 Iteration 000 Review Result 为 `accepted`，Next Iteration 为 None。执行记录是 `index_pages=4 separator_len=36 leaf_len=38 removable=0`，以及 `header_len=22 tuple_tag=1 gap=0 slot_len=31`。内部节点槽保持 36 字节分隔条目，数据槽保持 22 字节版本头紧贴 tuple。生产逻辑不改格式。

本变更因此开始自己的锁计划。I025 只剩下这一问：写争用被测量证实之后才加节点锁，并保留写单元原子性、根发布、取消和回滚。未过门槛则保持现有锁。

这一问的判据写不出来。计划停在这里。不加节点锁，也不改 `write_gate`。

## 依赖

依赖 `2026-11-24-ms55-index-page-space` 的格式结论，作为计划顺序。基点仍是 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`。本分支不合并 master，也不把归档文件并进来。本分支上的槽宽与那份结论一致：`MAX_KEY_LEN` 为 32，叶条目是 `MAX_KEY_LEN + RowId::SIZE`，分隔条目是 `MAX_KEY_LEN + 4`，`VersionHeader::SIZE` 为 22。

## What Changes

- 重读当前 `write_gate` 的取得者和读路径，并把结果写入 `design.md` 与当前 Cycle。
- 写争用的采纳或保持判据：见下方同名一节。该节的结论是写不出来。
- 不改索引页格式，不缩短 Key，不合并 Tag byte，不做脏页批量写、io_uring、代价模型。不沿用变长键假设。
- 不把「一定加节点锁」写成需求。不展开加锁 Iteration。

### 场景草图

1. 格式保持。前置：上述执行记录已被接受，本分支代码的条目宽度与它一致。动作：本变更不改 `node.rs` 的条目宽度、`VersionHeader`、tuple tag 和 `FORMAT_VERSION`。结果：槽宽保持。失败边界：为了测量锁而去改槽。停止。
2. 写单元保持。前置：`openspec/specs/index-write-side-atomicity/spec.md` 仍在本分支。动作：本变更不改 `insert`、`delete`、`delete_exact`、`update` 里结构变更、根页 id 发布和反向映射的同闭包顺序。结果：取消仍不能切开这个单元；同一索引的结构写仍串行；取消或回滚仍释放该语句占用的全部键；之后的结构检查仍要求没有 Error。失败边界：把根发布或反向映射移出 `spawn_blocking` 闭包。
3. 读路径与闸门。前置：当前 `IndexManager`。动作：核对 `search`、`scan_all`、`scan_range` 是否取得 `write_gate`。结果：这三处不取得它。I025 背景里的「每次操作 lock 整棵树」不描述这三条读路径。失败边界：把这句背景当成已经复核的加锁需求。
4. 同一索引上的写者。前置：两个连接可以同时进入同一个 `IndexManager` 的 `insert`。动作：观察后进入者与 `write_gate` 的关系。结果：后进入者要等先进入者释放这把 `Mutex<()>`。这是闸门的行为，不是加锁门槛。失败边界：因为能构出一次重叠，就批准节点锁。
5. 不同索引。前置：主键、唯一索引、二级索引各自一个 `IndexManager`。动作：核对 `write_gate` 的创建点。结果：`new` 和 `from_root` 各自 `Arc::new(Mutex::new(()))`。另一棵树的写者不在这把锁上等待。失败边界：按「整库一把锁」设计测量。
6. 反向映射读。前置：写单元持有 `write_gate` 期间，反向映射还没写完。动作：`find_key_by_row_id` 也取得 `write_gate` 再读映射。结果：它读到的是写单元落定后的映射。这段等待要留下来，它不是「可以改成节点锁就去掉」的等待。失败边界：把这次查找的等待当成加节点锁的证据。
7. 判据缺失。前置：场景 1 至 6 已核对。动作：为「写争用被测量证实」写一条能走向加锁、也能走向保持现有锁的判据。结果：写不出来。本 Cycle 保持 draft。失败边界：补一个耗时比例或次数，使 Gate 2 通过。

### 本次 goal 授权接受的选择

1. 格式与节点锁仍然是两个变更。格式结论只作为本变更的前提，不在这里重测槽字节。
2. 全局最优是停在判据上，而不是选一种锁协议。Semaphore、latch coupling 和 crabbing 仍是 I025 的历史方向，不是本计划的决定。
3. 被替代的做法见 `design.md` 的 D2。它们要么预先批准加锁，要么预先取消 MS55 里「证实之后可以加」的那半句，要么使用 MS55 没有写过的数字。

## 写争用的可观察判据

写不出来。

MS55 的原文是：节点锁只有在写争用被测量证实后才加，并保留写单元原子性、根发布、取消和回滚。未过门槛的保持原格式和现有锁。

`index-write-side-atomicity` 要求同一索引的结构变更串行执行，两个写者不得交错修改同一索引页，不得用过期根覆盖刚发布的根。规格没有写等待多久、等待多少次、或者哪一种页面不相交才改锁。

代码能证明的等待是：每个 `IndexManager` 有自己的 `write_gate: Arc<Mutex<()>>`。`insert`、`delete`、`delete_exact`、`update`、`find_key_by_row_id` 在 `spawn_blocking` 里取得它，并持有到该闭包结束。不分裂的叶插入只修改那一片叶子。两次这种插入若落在不同叶子上，而且调用在时间上重叠，后一次会停在 `write_gate.lock()`，即使两次都不修改同一页，也不发布新根。`search`、`scan_all`、`scan_range` 不取得这把锁。

把两次调用排成重叠之后，这次等待一定出现。用「观察到一次就加锁」做门槛，保持现有锁的那一支到不了。里程碑写的是未过门槛则保持现有锁，所以判据必须能走向保持。耗时比例、等待次数，以及别的里程碑里「中位数更短」那种比较，都没有出现在 MS55、这份规格或上述等待的证明里。

`arm_write_pause` 可以把已经持有闸门的写单元停住。停住的时长由测试放行决定。诊断摘要里的阶段等待也不读取 `write_gate`。这两处都不是这道门槛。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- 无。锁的行为和索引格式都不改。`.openspec.yaml` 使用 `skip_specs: true`。

## Impact

- 代码：无。
- 引用：MS55、I025。I029 与 I030 已由格式变更消耗。M03、M08、M13、M17 保持。
- 测试：本批不跑。不新增测量测试，因为测试要断言的采纳或保持条件还不存在。

## 回滚

没有产品改动。去掉本变更在本批写入的计划文件，就没有这份判据阻塞记录。已有库文件不用迁移。

## Gate 1

PASS。需求、场景和范围来自 MS55 的 Outcome、Scope、Non-goals、Split signals，以及本次 goal。这是批准，不是跳过。场景 7 的缺口用「不发明数字、计划停在 draft」补上，并写入上一节。没有 SKIPPED 项。

## Gate 2

BLOCKED。判据见「写争用的可观察判据」。`iterations/000-contention-criterion/000-initial.md` 的 Plan Context 保持 `draft`。不得交给 Act。
