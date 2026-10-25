# Design：ISS20 索引写侧原子性 + ISS21 DDL 目录事务化

本文件记录长期技术选择及其理由，随 change 归档。实施依据在 `iterations/000-index-write-side-atomicity/000-initial.md` 的 Plan Context。

## D1. 索引写单元：一个不可中断的原子单元

**当前行为**（`src/storage/btree/index_manager.rs:239-258`，`insert`）：

```text
capture root_page_id                      // :240  读
spawn_blocking { BTree::insert } → 树变更   // :244-248  结构变更（blocking 任务不可取消）
if split { root_page_id.store(new_root)   // :252  根页 id 发布
           sync_root_to_catalog(..).await }// :253  目录根同步
row_to_key.write().await.insert(..)        // :256  反向映射更新
```

三段写入被两个 await 切开。语句 future 在 `:248` 与 `:256` 之间被丢弃时：**树已变更（或已换根）但根页 id 未发布、反向映射缺失**。这同时是 E1（根页 id 与树内容不一致 → 后续写入沿损坏结构无限下降 → SIGABRT）与 E2（有正向条目、无反向映射 → 回滚分桶误判为墓碑 → 键残留）的共同根因。

**目标行为**：结构变更、根页 id 发布、反向映射更新收进**同一个 `spawn_blocking` 闭包**。tokio 的 blocking 任务在 JoinHandle 被丢弃后仍会执行到结束 ⇒ 语句被取消时该单元**整体落地或整体未开始**，不产生半写状态。

**实现要点**：

- 反向映射容器由 `tokio::sync::RwLock<HashMap<RowId, Vec<u8>>>` 改为 `std::sync::RwLock<HashMap<RowId, Vec<u8>>>`。闭包内同步访问；调用点只在不跨 await 的短临界区内持锁，符合 M13（异步路径不持 std 锁跨 await）。`find_key_by_row_id` 的公开签名保持 `async`（调用点不变），内部改为同步加锁。
- 根页 id 发布的 `AtomicU64::store` 在闭包内同步完成。
- `delete`（`:261-282`）当前**先**删反向映射（`:263`）**再**删正向条目（`:270-274`），取消落在两者之间会产生同形半写状态 ⇒ `delete` 的两段同样收进同一单元，且顺序改为「先删正向条目、后删反向映射」。
- `delete_exact` 与 `update` 的结构变更路径按同一形状收敛。

**目录根同步的归属**：`sync_root_to_catalog` 是 async（要经 buffer pool / storage），留在闭包之外、闭包返回之后执行。理由：它是**持久化**关注点而非结构原子性关注点；把它放进闭包会让 async 锁跨越 `spawn_blocking` 边界。既有语义（内存根先更新、目录根随后同步、失败时返回错误）保持不变。

## D2. 索引写侧串行化

**当前行为**：`insert` 与 `delete` 各自 `spawn_blocking` 一个临时 `BTree` 实例并各自读写 `root_page_id`，两条路径之间没有写者串行化 ⇒ 两个写者可能同时改同一页（其中一个的 `rebuild` 覆盖另一个的修改），也可能用旧根覆盖对方刚发布的根。

**目标行为**：同一 `IndexManager` 上的结构变更串行执行。实现：每个 `IndexManager` 持有一把 `std::sync::Mutex<()>`，在写单元闭包**内部**获取与释放（闭包内不跨 await，符合 M13）。

**为什么用 std Mutex 而不是 tokio Mutex**：写单元整体在 `spawn_blocking` 闭包内执行，锁的生命周期与闭包一致，不需要跨 await 持有。

**读侧同闸（Plan Review 2026-10-24 补记，Iteration 000 / Deviation 3 的裁定）**：**反向映射的读也取同一把闸门**。`find_key_by_row_id` 的公开签名保持 `async` 不变，内部经 `spawn_blocking` 取闸后再读 `row_to_key`。理由见 D3：回滚/回收按「反向映射是否命中」分桶，读若不与写单元串行就可能读到写单元尚未落定的中间态。

- 锁序统一为 **写侧闸门 → 反向映射 `RwLock`**，五处（`insert` / `delete` / `delete_exact` / `update` / `find_key_by_row_id`）无反序；无 std 锁跨 `.await`（M13）；`find_key_by_row_id` 的等待落在 blocking 线程，不占 Tokio worker。
- 边界：闸门是 `std::sync::Mutex`，等待顺序近似 FIFO 而非保证——若被取消语句的写单元尚在「已 spawn 未取到闸门」状态，理论上存在取闸顺序倒置的可能。其后果有界：最多残留一个键条目（E2 形态），不会产生结构损坏（E1 形态），因为写单元本身是原子的。

## D3. 取消/回滚后的键释放

D1 成立后，「有正向条目但反向映射缺失」的半写状态不再产生 ⇒ `src/transaction/manager.rs:339-355` 的 A/B 分桶（只查 `find_key_by_row_id`）即可正确识别这些行并进入 A 趟删除条目。**但 D1 单独还不够——分桶读还必须与写单元串行（见下）。**

**D1 单独不足（Plan Review 2026-10-24 补记）**：实施期实测证明，仅有 D1 的写单元原子化**不足以**兑现「被取消语句写入的**全部**键在回滚后释放」——回滚/回收的分桶读仍可能读到写单元尚未落定的中间态（写单元在 blocking 线程上跑完前，读已发出），被取消语句写入的键因而被误判为墓碑而不释放（显式事务路径稳定报 `Duplicate key`）。因此分桶读必须与写单元**串行**：读到的反向映射是**写单元落定后**的状态。⇒ D2 的闸门被读侧共用（D2「读侧同闸」）；这是兑现已批准 requirement `index-write-side-atomicity` R3 的最小机制，不改公共 API 的签名与返回值、不改分桶逻辑、不新增公共 API。

**不做的事**：不额外加固分桶逻辑（例如按 tuple 派生 PK 键反查条目）。理由：根因已在源头消除；再加一条反查路径会扩大改动面并引入新的错误语义。既有损坏索引的修复不在本 change 范围。

## D4. 确定性触发 ISS20 窗口（关键设计产出）

ISS20 的复现是竞态依赖（登记时 6 次尝试未命中），**「用例通过但缺陷仍在」的风险必须消除**。本 change 采用「写侧暂停点（test-support 夹具）」在项目内制造可复现的同步窗口。

### D4.1 夹具形态

在 `IndexManager` 上增加一个**测试专用**的写侧暂停点（先例：`Database::arm_io_failure`，MS33 已在产品代码里提供故障注入夹具）：

- 语义位置固定为「**结构变更已应用、根页 id 尚未发布**」。修复前后该语义位置不变（修复后根页 id 发布被移入同一闭包，位置即「闭包内结构变更之后、发布之前」）。
- 装载方式：`IndexManager::arm_write_pause(position) -> handle`；`handle` 提供「到达」信号（可 await / 可轮询）与「放行」操作。
- 未装载时不产生任何行为差异（既有测试保持 GREEN）。
- 实现要点：写单元在闭包内调用「到达并等待」；「到达」信号用可在阻塞线程上触发的通知，「放行」用 `Mutex<bool>` + `Condvar`（阻塞线程等待）。

**约束**：这不是身份证明机制，也不是判定层——判定逻辑全部落在 `cargo test` 内；暂停点只制造时序，不参与判定。

### D4.2 夹具几何（已按实际代码推导）

- 页大小 4096（`Page::PAGE_SIZE`）；叶节点条目 = `MAX_KEY_LEN(32) + RowId::SIZE(6) = 38` 字节 + `Slot::SIZE(6)`；`SlottedPageHeader::SIZE = 16` ⇒ **单叶最多 92 条**（`16 + 93×44 > 4096`）。
- `MIN_KEYS = 46`（`src/storage/btree/btree.rs:20`）。
- ⇒ 向一棵**单叶**索引插入第 **93** 个键时，叶分裂（46 / 47）并**换根**：`BTree::insert` 在闭包内创建新根页并返回 `Ok(Some(new_root))`，而 `IndexManager::insert` 的根页 id 发布在闭包**之外**（`:252`）。

### D4.3 用例形状（同时覆盖 E1 与 E2）

```text
1. 建表 t（单列主键），插入 92 行（键 1..92）——索引为单叶，恰好装满。
2. 在 t 的 IndexManager 上装载写侧暂停点（位置：结构变更之后、根发布之前）。
3. 后台任务执行 INSERT INTO t VALUES (93, ...)：
   写入第 93 个键 → 叶分裂 → 新根页已建 → 命中暂停点。
4. 测试轮询「到达」信号（yield_now 轮询，超时只作夹具失控兜底，不是行为判据）。
5. 丢弃语句 future（取消）。
6. 放行暂停点：写单元跑完。
7. 执行下一条写语句 → 触发回滚回收。
8. 断言：
   a) 键 1..92 全部仍可查到（换根被丢失时，键 47..92 落在新根的右半，不可达）；
   b) 键 93 可再次插入（无 DuplicateKey）；
   c) check_integrity 无 Error 级发现；
   d) 进程未 abort。
```

**为什么这稳定命中缺陷**：修复前，结构变更（叶分裂 + 新根页）在闭包内完成，根页 id 发布在闭包外；取消落在两者之间时**必然**出现「树已换根、已发布根仍是旧根」⇒ 断言 (a) 必然失败（键 47..92 不可达），断言 (b) 也必然失败（反向映射缺失 ⇒ 回滚分桶把该行当墓碑 ⇒ 键 93 残留）。修复后，写单元整体落地 ⇒ 断言全绿。

**与「竞态依赖」的区别**：本用例不依赖两个写者的交错时序——单写者在暂停点被取消即可确定性地产生半写状态。它命中的是 E1/E2 的**同一个根因窗口**（结构变更与发布/反向映射之间），并且是 E1 现场「根页 id 指向非 B 树节点页」的上游成因（换根丢失 ⇒ 根页 id 与树内容分叉 ⇒ 后续写入沿错误结构下降）。

**并发写者场景的覆盖边界（诚实声明）**：两个写者对同一索引页交错修改需要真正的时间重叠，本 change **不**为它提供确定性触发；该风险由 D2 的写侧串行化从设计上消除，其行为面断言落在 D4.3 的用例与 T3 的端到端回归上。

## D5. DDL 目录变更的事务化（ISS21）

**当前行为**（`src/storage/data/table_manager.rs:792-802`，`alter_drop_column`）：先 `rewrite_column_rows_drop` + `update_table_column_count` 改目录，再 `flush_all`，**之后**才在事务内逐行 DELETE + INSERT 改写数据面；执行器（`src/executor/alter_table.rs`）随后提交或中止。⇒ 语句被放弃/失败时：目录已窄化且已落盘，数据面被回滚 ⇒ 「目录已窄化 + 数据面仍混合宽度」。

**目标行为**：目录窄化延后到**语句确定完成之后**（ISS21 处置给出的两个方向之一：「窄化延后到语句确定完成之后」）。

- `alter_drop_column` 不再改目录、不再 `flush_all`；它只做守卫检查、计算新布局、在事务内完成数据面改写，并返回**待应用的目录变更**（表名、被删列的完整列定义与原序号）。
- 执行器：`alter_drop_column(...)` → 成功 → `commit(tx)` → **提交完成后**应用目录变更（`rewrite_column_rows_drop` + `update_table_column_count_ddl` + flush，先列行后计数，与 ADD COLUMN 路径同序）→ 切换内存 `TableMeta` 到新布局。
- 失败路径：执行器 `abort(tx)`，目录与内存布局均未变更，数据面回滚 ⇒ 完全一致。

**为什么选「延后」而不是「纳入回滚」**：「纳入回滚」需要在中止路径重建目录（重插列行 + 后续序号回退 + 恢复计数）并回滚内存 `TableMeta`；中止路径当前只拿到 `&HashMap<String, Arc<TableMeta>>` 快照，无法写回活表映射，改动面显著更大。「延后」复用既有 API（`update_table_column_count_ddl` 已是 ADD COLUMN 的提交点写法），改动面小且与 ISS21 自身建议一致。

**崩溃窗口（明确记录）**：提交完成与目录应用之间崩溃 ⇒ 目录仍是旧布局、数据已是新布局。**读侧不会报错，但会位置性错读**：宽度感知解码（`deserialize_tuple_width`，`src/storage/page_format/tuple.rs:189-222`）按位置取值、尾部补 `Value::Null`，不校验列序；全部读路径（`src/executor/scan.rs`、`data_scan.rs`、`index_scan*.rs` 等）都走它 ⇒ 窄 tuple 被无声接受。重开库时活布局取自目录列行并按宣告列数截断（`src/storage/data/table_manager.rs:284-299`）⇒ 该窗口内活布局仍是 3 列而数据已是 2 宽 ⇒ 2 宽的 tuple 读作 `[id, w, NULL]`（`v` 列读到 `w` 的值、`w` 列读 NULL），影响面是全部已改写的行。代价是该 DDL 的**目录效果丢失**（目录仍描述旧布局 ⇒ 该 DDL 可重做）。与现状对比：现状（目录已窄化 + 数据仍宽）是**镜像方向**的同一类位置性错读（`w` 列读到 `v` 的值、宽行尾部被忽略），两者影响行集相当 ⇒「比现状更安全」的比较不成立，可靠的差别只在丢的是目录效果（可重做）还是数据改写（需重做改写）。

MS21 D10 的既有崩溃边界不在本 change 内调整；该窗口的承载见本 change Act Response 的 Issue 候选 D。**本段表述于 Iteration 001 Cycle 000 的 Plan Review 就地修正**：原文写「读侧……不会静默错读」，与上述读码事实矛盾（修正依据 = 读码，未构造崩溃点实跑——窗口的可达性本就作为已知边界记录）。

**并发读窗口（风险记录）**：数据面改写期间，活 `TableMeta` 仍是旧布局而部分行已是新宽度，存在与现状同类的读侧窗口（现状是「新布局 + 旧宽度数据」）。本 change 不改变这一窗口的性质。

## D6. 实现顺序与依赖

1. 先落 D4.1 的写侧暂停点夹具（行为中性，既有测试保持 GREEN）→ 再写 D4.3 的用例并**观察 RED**。
2. 再落 D1 + D2 ⇒ 用例转 GREEN。
3. 再补端到端回归（T3）。
4. ISS21 独立成 Iteration：先落 RED 用例（放弃 DDL → 回收 → 重开 → 目录列数与行宽一致），再落 D5 ⇒ GREEN。

## D7. 明确禁止的实现方式

- 不得用「加 run-id / 指纹 / 时间戳确认这是同一次运行」「加判定层识别损坏结构」等方式解决复现问题（AGENTS.md 行为约束）。
- 不得用 timer 阈值作行为判据；暂停点「到达」用可观察状态 + `yield_now` 轮询，timer 只作夹具失控兜底。
- 不得在 `cargo test` 集合之外建判定脚本、封装命令或专用参数。
- 不得改动 MS34 已验收的取消语义面。
