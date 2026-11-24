## 背景

起点是 `55efdb0fb872a1d6406891128cf8d4e0f29c8997`。分支 `changes/2026-11-24-ms55-index-node-lock` 的计划提交之前，HEAD 是 `97b648ae069a274c40b54399db0647d0ecdc8782`，上面只有本变更的提案。没有合并 master。`write_gate` 在本分支的 `src/storage/btree/index_manager.rs` 里可读。

格式变更的执行记录在 master 的 `openspec/changes/archive/2026-11-24-ms55-index-page-space/iterations/000-measure-and-keep/000-initial.md`。Review Result 为 `accepted`，Next Iteration 为 None。本分支槽宽与该结论一致，所以不把归档目录合并进来。

## 目标与非目标

目标：为 I025 写下写争用的采纳或保持判据。判据必须来自 MS55 原文、既有规格，或能从当前代码和可观察等待证明的条件。写不出来就保持计划为 draft，不加锁。

非目标：改索引页格式，缩短 Key，合并 Tag byte，脏页批量写，io_uring，代价模型，变长键。也不在本文件里选择 latch coupling、crabbing 或 Semaphore。

## 当前行为

`IndexManager.write_gate` 的类型是 `Arc<Mutex<()>>`。`new` 和 `from_root` 各自执行 `Arc::new(Mutex::new(()))`。主键、唯一索引和二级索引是不同的 `IndexManager`，`TableMeta.index_manager` 是 `Arc<IndexManager>`。`Database` 可克隆，内部是 `Arc`。`Server` 把 `Arc<Database>` 交给每个连接任务。`pipeline.rs` 没有再包一层语句互斥。两个连接可以同时进入同一索引的写路径，后一个等的是这一棵树自己的 `write_gate`。

取得 `write_gate` 的函数，都在 `tokio::task::spawn_blocking` 里 `lock()`，并持有到闭包结束：

| 函数 | 持锁期间还做什么 |
|---|---|
| `insert` | `BTree::insert`，暂停点，按需发布根页 id，写入 `row_to_key` |
| `delete` | 查找、`BTree::delete`，暂停点，按需发布根页 id，删除反向映射 |
| `delete_exact` | `BTree::delete_exact`，按需删除反向映射。该函数的合同是不改根 |
| `update` | `BTree::update`，更新反向映射。不发布根 |
| `find_key_by_row_id` | 读 `row_to_key` |

`insert` 和 `delete` 的目录根同步在闭包之外，通过 `sync_root_to_catalog`。闸门不覆盖这一步。

`search`、`search_all`、`scan_all`、`scan_range`、`collect_all_pages` 只对 `root_page_id` 做 `Ordering::Acquire` 的加载，不调用 `write_gate.lock`。

不分裂的叶插入在 `BTree::insert_into_page_step` 里对那一片叶子 `modify_page` 成功后返回 `Ok(None)`。内部节点在这次下降里只被读过。根页 id 只有根分裂时才换成新页。因此两片不同叶子上的不分裂插入不修改同一索引页，也不发布新根，但仍然全程持有同一把 `write_gate`。

页帧本身是 `Arc<Mutex<PageFrame>>`（M17）。这把帧锁只覆盖单次读页或改页。它不代替 `write_gate` 把整次写单元串起来。

`find_key_by_row_id` 取得闸门，是为了不看见「结构已改、反向映射还没写」的中间态。这段等待属于写单元的读者排除，不属于节点锁要拆开的树等待。

`index_manager.rs` 不调用诊断接口。诊断阶段里的等待合计没有 `write_gate` 的计时。`arm_write_pause` 的位置是结构已改、根页 id 尚未发布。未装载时写路径只做一次未命中检查。装载后的停留时长由 `WritePauseHandle::release` 决定。既有原子性测试把 10 秒写成夹具失控兜底，并写明它不是行为判据。

M08：根页 id 用 `AtomicU64`，异步读路径不用 `RwLock` 保护根指针。M13：异步路径不持 `std::sync::Mutex` 跨 `.await`；B-Tree 写在 `spawn_blocking` 里完成。这两条在本计划里保持。本计划不设计另一套持锁方式。

## D1 格式不再测量

槽宽沿用已接受的格式结论，并用本分支源码核对过宽度公式。本变更不新增槽字节测试，不改 `node.rs`、`key.rs`、`version_chain.rs`、`tuple.rs`、`file_header.rs`。

被替代的做法：在本分支重跑格式变更的两条打印测试。结论已经接受，槽公式也还在，重跑不产生锁的判据。

## D2 不设采纳或保持判据

写争用的可观察判据写不出来。正文与 `proposal.md` 同节一致，这里记录选择和被替代方案。

选择：Plan Context 保持 `draft`。不创建测量测试，不修改 `write_gate`，不在 `tasks.md` 里预写加锁 Iteration。

理由：MS55 要求未过门槛时保持现有锁。判据必须能走向保持，也能走向加锁。下面每一条被替代方案都做不到这一点，或者使用了 MS55 和现有规格都没有的数字。

| 被替代方案 | 不采用的原因 |
|---|---|
| 两次不相交的不分裂插入一旦重叠就加节点锁 | 重叠由测试排出来。`Mutex::lock` 在持有期间必然挡住下一次 `lock`。保持分支到不了，等于预先批准加锁 |
| 等待次数大于 0 才加锁 | 上一种方案的次数写法。次数没有出现在 MS55 或原子性规格里。MS53 分配器变更已经拒绝把「次数大于 0」当作换实现的条件，这里同样没有这个条件 |
| 等待时间占写入时间的某个比例，包括 10% | MS55 没有比例。MS47 的 10% 比的是逐行插入在语句里的占比，不是索引闸门 |
| 仿照 MS54，做一种更细锁的试验实现，中位数更短就加锁 | 试验实现的代价就是协议本身。零代价的假锁在不相交键上会更短，结果仍是预先加锁。MS54 的「写次数小于 64」对应的是合并写少发了写调用，节点锁没有这样的现成计数 |
| 规格写了结构变更必须串行，所以永远不加节点锁 | 这取消了 MS55 里「证实之后可以加」的半句。本计划没有得到这条范围削减 |
| 用 `arm_write_pause` 的停留或诊断阶段的等待合计当门槛 | 停留由放行决定。诊断不读取 `write_gate`。原子性测试已写明 10 秒不是行为判据 |
| I025 的 Semaphore 加 latch coupling 直接作为实现 | 那是方向，不是已批准需求。读路径并不取得 `write_gate`，不能用「每次操作锁整棵树」证明争用已经成立 |

## D3 以后若要离开 draft

需要新的判据来源：MS55 的补充原文、既有规格里的句子，或一种不靠自选比例和次数、并且保持分支仍然可达的可观察等待。那是重新计划，不是本 Cycle 的 Act。加锁若进入那时的后继 Iteration，写单元、根发布、取消和回滚仍要保持，M08 与 M13 仍要保持。

## 风险

有人把场景 4 的必然等待读成测量已经证实。D2 的第一行就是为了挡住这个读法。

本分支没有格式变更的归档目录。以后核对执行记录用 master 上的那份 Cycle，不合并。

## 回滚

见 `proposal.md`。无产品改动，无格式迁移。
