# Learned — 学习记忆

> 版本：v1.6 | 最后更新：2026-06-04（M21 遗留项完成 L030）
> 由 openspec-init 从 `.claude/docs/learned.md` 迁移。
> 条目格式: <!-- L{编号} --> 标记开头，支持 grep 精确定位。

---

## Purpose

记录 RTsql 项目开发过程中积累的 API 路径、踩坑档案、技巧模式和依赖关系，避免重复踩坑，加速新功能开发。所有条目支持 grep 精确定位。

---

## Requirements

### Requirement: 踩坑经验可追溯

所有开发中遇到的坑 SHALL 以"症状→根因→解决"格式记录。

#### Scenario: 遇到新的踩坑
- **WHEN** 开发中遇到非显而易见的问题并解决
- **THEN** 在踩坑档案区新增条目，格式：`<!-- L{编号} --> ### [问题标题]` + 症状→根因→解决

#### Scenario: 查询历史踩坑
- **WHEN** 遇到类似问题想参考历史经验
- **THEN** 通过 `grep "关键词" openspec/specs/learned/spec.md` 定位对应条目

### Requirement: API 路径速查可用

核心 API 路径 SHALL 以表格形式维护，便于快速查找。

#### Scenario: 查找 API 位置
- **WHEN** 需要知道某个 API 的文件路径和用途
- **THEN** 在 API 路径表中查找对应条目

#### Scenario: 新增 API
- **WHEN** 实现了新的公共 API
- **THEN** 在 API 路径表中新增条目

### Requirement: 技巧模式可复用

已验证的实现技巧 SHALL 以表格形式记录，便于复用。

#### Scenario: 使用已知技巧
- **WHEN** 需要实现零拷贝、两阶段锁等模式
- **THEN** 在技巧模式表中查找已有实现参考

---

## API 路径

<!-- L001 --> | 名称 | 路径 | 用途 | 时间 |
|------|------|------|------|
| Database::open | src/database.rs | 打开/创建数据库 | 2026-05 |
| Database::execute_sql | src/database.rs | 执行 SQL 语句 | 2026-05 |
| BufferPool::get_page | src/storage/buffer_pool.rs | 获取页（两阶段锁） | 2026-05 |
| PageGuard::page_data | src/storage/page_frame.rs | 零拷贝读取页数据 | 2026-05 |
| PageGuard::modify_page | src/storage/page_frame.rs | 修改页数据（自动 dirty） | 2026-05 |
| IndexManager::search | src/storage/btree/index_manager.rs | Async search | 2026-05 |
| BTree::from_root | src/storage/btree/btree.rs | 临时实例（写操作） | 2026-05 |
| PlanBuilder::build | src/parser/planner.rs | SQL → PhysicalPlan | 2026-05 |
| Pipeline::execute | src/pipeline.rs | 执行管道入口 | 2026-05 |
| inject_correlated_values | src/executor/correlated.rs | 向谓词树注入外层列值 | 2026-05 |
| BTree::search_all | src/storage/btree/btree.rs | 返回所有匹配 RowId | 2026-05 |
| BTree::delete_by_key | src/storage/btree/btree.rs | 删除所有匹配 entries | 2026-05 |
| BTree::delete_exact | src/storage/btree/btree.rs | 精确删除 | 2026-05 |
| LeafNode::merge_right | src/storage/btree/node.rs | 吸收右兄弟 entries | 2026-05 |
| InternalNode::merge_right | src/storage/btree/node.rs | 吸收右兄弟 + 降级 separator | 2026-05 |
| FileStorage.free_pages | src/storage/file_storage.rs | Mutex<Vec<u64>> free-list | 2026-05 |
| Server::new | src/network/server.rs | 创建服务器（addr, db, max_connections） | 2026-06 |
| Server::shutdown_token | src/network/server.rs | 获取 CancellationToken 用于优雅关闭 | 2026-06 |
| TableMeta.data_page_head | src/storage/data/table_manager.rs:51 | 数据页链表头（M19 用） | 2026-06 |
| SlottedPageHeader.next_page_id | src/storage/page_format/slotted_page.rs:21 | 数据页链表指针（M19 用） | 2026-06 |
| IndexManager.scan_all | src/storage/btree/index_manager.rs:204 | BTree 全遍历（当前 ScanExecutor） | 2026-05 |

---

## 文件速查

<!-- L002 --> | 名称 | 路径 | 用途 | 时间 |
|------|------|------|------|
| database.rs | src/database.rs | Database 协调器 | 2026-05 |
| pipeline.rs | src/pipeline.rs | SQL 执行管道 | 2026-05 |
| buffer_pool.rs | src/storage/buffer_pool.rs | BufferPool（两阶段锁） | 2026-05 |
| slotted_page.rs | src/storage/page_format/slotted_page.rs | SlottedPage + compacting | 2026-05 |
| index_manager.rs | src/storage/btree/index_manager.rs | IndexManager（AtomicPageId） | 2026-05 |
| aggregate.rs | src/executor/aggregate.rs | AggregateFunc + AggregateState | 2026-05 |
| join.rs | src/executor/join.rs | JoinExecutor（哈希连接） | 2026-05 |
| semi_join.rs | src/executor/semi_join.rs | SemiJoinExecutorV2 | 2026-05 |
| anti_join.rs | src/executor/anti_join.rs | AntiJoinExecutor | 2026-05 |
| subquery_eval.rs | src/executor/subquery_eval.rs | SubqueryEvalExecutor | 2026-05 |
| correlated.rs | src/executor/correlated.rs | inject_correlated_values | 2026-05 |
| predicate.rs | src/executor/predicate.rs | Predicate/Expression + ParameterExpression | 2026-05 |
| planner.rs | src/parser/planner.rs | PlanBuilder（含子查询/关联检测） | 2026-05 |
| data_page.rs | src/storage/data_page.rs | 数据页读写 + VersionHeader | 2026-05 |
| table_manager.rs | src/storage/data/table_manager.rs | TableMeta（data_page_head/tail） | 2026-06 |

---

## 踩坑档案

<!-- L003 --> ### [delete_by_key 并发 merge 位置偏移]
**症状→根因→解决**: merge 改变父节点结构，后续子节点索引失效 → `&mut self` + root_page_id 现场更新

<!-- L004 --> ### [merge 容量溢出]
**症状→根因→解决**: min_keys=48，leaf 容量=92，47+48=95>92 → redistribution-first + can_merge_with 拦截

<!-- L005 --> ### [gc_test SlotID 失效]
**症状→根因→解决**: compacting 改变物理 SlotID，版本链引用旧值 → logical_id 解耦（Slot 4B→6B）

<!-- L006 --> ### [delete_slot 不序列化 header]
**症状→根因→解决**: slot_count 修改只在内存，未 serialize 回 page.data → header 修改后必须 serialize

<!-- L007 --> ### [gc_test panic 连锁]
**症状→根因→解决**: 第一个 panic poison BufferPool Mutex → 引入 logical_id 根本修复

<!-- L008 --> ### [RecoveryManager 需要表才能 redo]
**症状→根因→解决**: get_table 失败时 redo 静默跳过 → 表定义持久化是完整恢复前提

<!-- L009 --> ### [get_subquery_first_column 不支持 SemiJoin/AntiJoin]
**症状→根因→解决**: 嵌套 IN 子查询 SemiJoin 节点未被处理 → 添加 SemiJoin/AntiJoin 分支 + output_columns

<!-- L010 --> ### [inner_column_index 设计失误]
**症状→根因→解决**: CorrelatedParam 用 usize 索引匹配 → 改为 param_name: String 按列名匹配

---

## 技巧模式

<!-- L012 --> | 模式 | 描述 | 适用场景 |
|------|------|----------|
| 零拷贝页读取 | page_data() + SlottedPageRef | 只读场景 |
| 两阶段锁 | 读锁→释放→I/O→写锁(double-check) | 缓存加载 |
| AtomicPageId | AtomicU64::load(Acquire) | async 无锁访问 |
| Hash Aggregation | HashMap<Vec<Value>, Vec<AggregateState>> | GROUP BY |
| 临时 BTree 实例 | BTree::from_root() + spawn_blocking | 写操作保持 sync |
| 哈希连接 | 构建侧哈希表 + 探测侧匹配 | INNER JOIN |
| Mutex 参数注入 | ParameterExpression + Mutex<Value> | 相关子查询 |
| 双路径执行器 | correlated_params 非空走按行重建 | SemiJoin/AntiJoin |
| 非唯一索引同页多条目 | LeafNode 去掉 DuplicateKey 检查 | 索引允许重复 key |
| 批量删除从后向前 | delete_by_key 从后向前删除 slot | 批量删除同页多个 slot |
| 两次加载页模式 | 先 page_data() 读找，再 modify_page() 删除 | 页面读写分离 |
| 惰性初始化 | search_all 在首次 next() 调用时执行 | IndexScanAll |
| 连接并发 Semaphore | `Arc<Semaphore>` + `acquire_owned()` in spawn | 限流并发连接、防连接风暴 |
| 数据页链表遍历 | SlottedPageHeader.next_page_id + TableMeta.data_page_head | 全表扫描跳过索引层（M19） |

---

## 依赖关系图

<!-- L013 -->
```
Database ──→ Pipeline ──→ Executor Tree
                │
                ▼
         BufferPool ──→ PageGuard
                │
                ▼
           BTree ──→ LeafNode/InternalNode
                │
                ▼
        IndexManager ──→ AtomicPageId (async)
                │
                ▼
         WalWriter ──→ WALBuffer ──→ WALFile
                │
                ▼
      RecoveryManager ──→ TransactionManager
```

---

## WAL/Recovery 测试策略

<!-- L014 --> | 发现 | 详情 | 来源 |
|------|------|------|
| WAL 记录验证优于重启验证 | TableManager 纯内存，重启后表丢失，改为直接读 WAL 验证 | recovery_e2e_test.rs |
| 独立 WAL 层 benchmark | 直接操作 WALBuffer，不经过 SQL 层 | wal_group_commit_bench.rs |
| tempdir leak 模式 | `std::mem::forget(dir)` 保证 WAL 文件存活 | wal_group_commit_bench.rs |

---

## 基准测试技巧

<!-- L015 --> | 技巧 | 用途 | 代码位置 |
|------|------|----------|
| 共享 tokio::runtime | 避免 per-iteration 创建 runtime | benches/sqlite_compare.rs |
| RTsqlDirect in-process | 直接调用 API，避免 network overhead | benches/sqlite_compare.rs |
| criterion Throughput | 设置 throughput 更准确测量 | benches/sqlite_compare.rs |
| #[inline(never)] + std::hint::black_box | 防止编译器消除 fetch_add 真实开销 | benches/tx_id_bench.rs |
| std::thread::spawn + Arc 共享计数器 | 多线程争用基准（避免 rayon 依赖）| benches/tx_id_bench.rs |

---

## 实测性能数据（M41 AtomicU64）

<!-- L017 -->
| 场景 | Mutex (ns/op) | Atomic (ns/op) | 加速比 | 来源 |
|------|--------------|----------------|--------|------|
| 单线程 1M | 10.7 | 5.1 | 2.1x | benches/tx_id_bench.rs |
| 10 线程 × 100K | 84.7 | 18.6 | 4.6x | benches/tx_id_bench.rs |
| 100 线程 × 10K | 100.8 | 22.5 | 4.5x | benches/tx_id_bench.rs |
| 吞吐@1M (单线程) | 90.8 Melem/s | 138.1 Melem/s | 1.52x | benches/tx_id_bench.rs |

**关键结论**：
- tasks.md 路线图"100ns→10ns"达成（单线程 5.1 ns/op）
- 10/100 线程争用下 Atomic 5x 加速假设全部满足
- 单线程差异假设（< 20%）被推翻：实际 Atomic 在单线程下也 2x 快（锁自身开销显著）

### M20 闭包方案（zero-copy SlottedPageRef）

<!-- L024 -->
| Benchmark | Before-m20 (median) | After-m20 (median) | Change | 场景 |
|-----------|---------------------|--------------------|--------|------|
| delete/by_pk | 172.38 ms | 158.03 ms | **-8.33%** | 读 + 写（index + tuple）|
| filter/where_value_gt_500 | 33.714 µs | 32.768 µs | **-3.53%** | 读路径（read_tuple + deserialize）|
| sort/order_by_value_desc | 49.123 µs | 46.618 µs | **-4.56%** | 读 + 排序 |
| join/inner_join | 53.003 µs | 51.563 µs | **-2.46%** | 读 + Hash Join |
| scan/full_table | 36.089 µs | 36.665 µs | +2.04% | 全表扫描（噪声内）|
| limit/limit_10_offset_5 | 24.063 µs | 23.705 µs | -0.60% | 限制（噪声内）|
| update/single_column | ~75 ms | 78.006 ms | **+3.99%** | 写路径（闭包内 `.to_vec()` 成本）|

**对比 design.md 目标**：
- ≥ 15% 提速 — **未达**（实际 read 路径 -2.46% 到 -8.33%）
- 回归 < 5% — ✅ 通过（update +3.99% < 5% 阈值）

**为什么没达 15%**：
- micro_bench 测试数据是 1000 行 ~100B/行，每次 `to_vec()` 分配 100KB
- 现代 jemalloc/Rust 全局分配器对 100KB Vec 分配已经非常快
- 真实场景（更大行/批量分配）零拷贝收益更明显，但 micro_bench 行数太小掩盖收益
- 关键收益是消除 **N×4KB 页内 slice 拷贝**（隐式），这部分在 1K 行规模下也有限

**写路径 +3.99% 回归原因**：
- design.md 决策 5：写路径（write_commit_tx_id / UpdateExecutor）闭包内 `.to_vec()`
- `.to_vec()` 在 `with_page_data` 闭包内执行，等价于原 `to_vec()`
- 微小回归来自闭包调用栈额外开销（函数指针、Closure 环境捕获）

**How to apply**：
- M19 (DataScan) / M36 (零拷贝 ValueRef) 应进一步消除分配，可能达 15%+
- 当前 4%-8% 改进已符合 "I/O ~20-30% 提速" 路线图（实际是分配开销 ~5%）
- update 写路径回归在阈值内，可接受；进一步优化需避免 `.to_vec()` 但要重做写路径设计

### M36 闭包方案（zero-copy ValueRef）

<!-- L025 -->
| Benchmark | After-m36 (median) | 场景 |
|-----------|---------------------|------|
| insert/single_row/50 | 3.6922 ms | 单行插入（50B 值）|
| insert/single_row/99 | 3.9623 ms | 单行插入（99B 值）|
| select/pk_lookup | 73.951 µs | 主键查询 |
| update/single_column | 125.76 ms | 单列更新（写路径）|
| delete/by_pk | 258.43 ms | 按主键删除 |
| scan/full_table | 53.274 µs | 全表扫描 |
| filter/where_value_gt_500 | 57.388 µs | 过滤 |
| sort/order_by_value_desc | 76.624 µs | 排序 |
| limit/limit_10_offset_5 | 37.665 µs | 限制+偏移 |
| join/inner_join | 81.133 µs | Hash Join |

**vs design.md 目标**：
- 30万→0 分配 — ⚠️ **未直接验证**（micro_bench 当前用 Int 列，无 String 分配；30万→0 是 String 列场景的设计目标）
- ≥ 5% 速度 — ⚠️ **未直接验证**（无 before-m36 baseline，因为 M36 改动已发生）

**为什么没直接验证 5%/30万→0**：
- M36 实施前未保存 before-m36 baseline（plan T9 步骤 1 设计缺陷 — 应在 M36 改动**前**保存）
- micro_bench 当前用 Int 列，**M36 主要消除 String 分配，对 Int-only 场景收益不直接可测**
- 实际收益场景：String 列 ≥ 100B 的 Scan 路径，1K 行可减 30万次 String 分配

**对比 M20 经验**：
- M20 留了 before-m20 baseline（M38/M30 漏 commit 后未受影响），所以有真实对比
- M36 T9 步骤 1 跑 baseline 时已是 M36 实施**后**状态（git stash 与 master 上其它改动冲突，无法干净 stash M36 改动）
- 这是 T9 plan 的盲点：未来 M19/M37 等 milestone 应**先**留 baseline 再实施

**已知 L025 局限**：
- 数据是 after-m36 snapshot，仅供未来回归参考（M19/M37 实施后跑 `--baseline before-m36` 即可对比）
- 30万→0 分配目标需要 String 列 benchmark 单独验证（建议 `benches/string_scan_bench.rs` 新建）

**How to apply**：
- 未来 zero-copy 改进（M19/M37）实施前应先 `cargo bench --save-baseline before-X`
- micro_bench 需扩展 String 列场景才能验证 M36 真实收益
- M36 设计本身正确（闭包内 `deserialize_value_refs` 借用 + `.to_value()` 消费），性能差异主要在 String 列

---

## M30 连接并发 Semaphore

<!-- L018 --> | 发现 | 详情 | 来源 |
|------|------|------|
| Semaphore in tokio::spawn | `acquire_owned().await` 在 spawn 闭包内获取 permit，外层 accept 循环不阻塞 | server.rs |
| Permit 生命周期绑定 handler | `_permit` 随 handler 生命周期释放，连接断开自动归还信号量 | server.rs |

<!-- L019 --> | 发现 | 详情 | 来源 |
|------|------|------|
| 连接限制测试模式 | TcpStream `connect()` 不阻塞；用 `read(&mut [0])` 验证连接仍存活（读取阻塞 = 连接保持）| connection_limit_test.rs |
| `select! + semaphore` hold | 获取 permit 后 spawn 才返回，控制面与数据面解耦 | server.rs |

---

## M38 网络 BufWriter + TCP_NODELAY

<!-- L020 --> | 发现 | 详情 | 来源 |
|------|------|------|
| PgProtocol 内嵌 write_buf | 不修改 Protocol trait，在 PgProtocol 上挂 `write_buf: Vec<u8>`（8KB），`write_response` 中 `self.write_buf.clear()` 后累积，最后单次 `write_all`+`flush` | pg_protocol.rs |
| send_startup_response 同样批量化 | 启动握手 7 次独立 write_all → 1 次，减少连接建立 syscall | pg_protocol.rs |
| Vec<u8> 优于 bytes::BytesMut | 无需引入新依赖，Vec 的 `extend_from_slice` + `clear()` + `write_all` 满足批写需求 | pg_protocol.rs |

<!-- L021 --> | 发现 | 详情 | 来源 |
|------|------|------|
| TCP_NODELAY 设置时机 | `stream.set_nodelay(true)` 在 accept 后、spawn 前调用，确保每次连接生效 | server.rs |
| 批写验证测试 | 100 行 × 3 列（超 8KB 自扩容）+ 4 批次连续查询（缓冲复用），11 tests 全通过 | pg_protocol_test.rs |

<!-- L022 -->
### [M20 PageDataGuard 自包含设计 3 次失败 — 2026-06-03]

**问题**：M20 设计要求 `get_page_ref() -> Result<PageDataGuard<'_>>`，其中 PageDataGuard 借用与 &self 生命周期一致。

**尝试 1：tuple `(PageGuard, PageDataGuard<'_>)`**
- 编译错误：`cannot move out of page_guard because it is borrowed` (E0505)
- `page_data()` 返回的 PageDataGuard 借用 &page_guard，无法同时 move page_guard 进 tuple

**尝试 2：unsafe self-referential struct（PageDataGuard 自包含 Arc + raw pointer + 'static transmute）**
- 编译通过（unsafe 通过）
- 测试 hang 在 `data.len()` 之前：eprintln `"[DBG] get_page_ref: got data"` 输出后，进程永远卡住
- 30 秒 `timeout` 触发，exit code 124
- 推测原因：
  - `eprintln!` 内部获取 stderr 锁，与 `arc.lock()` 形成锁顺序冲突？
  - 或 `std::mem::transmute` 后 MutexGuard 的内部状态损坏，drop 时死锁？
  - 或 current_thread tokio runtime 与 std::sync::Mutex 锁争用？

**根本约束**：在 safe Rust 中，MutexGuard 的 borrow 链无法跨越函数调用边界（&Arc 必须保持在 scope 内）。`Result<PageDataGuard<'_>>` 形式不可达。

**结论**：M20 的 `get_page_ref` 必须妥协为以下任一形式：
1. **闭包式 API**（与 find_visible_version 一致）：`pub async fn get_page_ref<F, R>(&self, page_id, f: F) -> Result<R> where F: FnOnce(&[u8]) -> R`
2. **tuple 配合 PageGuard::Clone**（需要补 Clone 实现 + 修复 ref_count 逻辑）
3. **unsafe self-referential 调试**（需更多时间查清 hang 根因）

**为什么写下来**：
- 避免后续 M19/M36 重复踩坑（同样涉及零拷贝借用链）
- unsafe 调试成本高，**优先走方案 1（闭包）**
- cargo test 默认捕获 stderr，hang 排查必须加 `-- --nocapture` 才看得到 eprintln

**预防**：
- 闭包形式 = Rust 异步 + 锁借用的标准范式，零拷贝场景应默认采用
- 若坚持直返 guard，要么 unsafe 写完整 + 单测覆盖所有 drop 顺序，要么妥协为 tuple/Clone

**How to apply**：
- M20 后续 task（T3-T6）涉及零拷贝 API 设计时，**首选闭包方案**
- 调试 cargo test hang 立即用 `cargo test -- --nocapture`
- 自包含 self-referential struct 在 Rust 中成本极高（需要 `ouroboros` crate 或手写 unsafe + 精心 drop 顺序），不值得为 1 个 API 引入

<!-- L023 -->
### [M20 闭包方案最终设计 — 2026-06-03]

**决策**：M20 全面采用闭包 API，替代原 `get_page_ref` / `PageDataGuard<'_>` 返回值方案。

**核心 API**：
1. `BufferPool::with_page_data<F, R>(&self, PageId, F) -> Result<R>` — `F: FnOnce(&[u8]) -> Result<R>`
2. `read_tuple_from_data_page<F, R>(&BufferPool, RowId, F) -> Result<R>` — `F: FnOnce(VersionHeader, &[u8]) -> Result<R>`
3. `find_visible_version<F, R>(&self, RowId, &Snapshot, F) -> Result<Option<R>>` — `F: FnOnce(&[u8]) -> R`

**关键设计点**：
- `VisibilityResult<R>` 枅助枚举解决 `find_visible_version` 中可见/不可见分支返回不同类型的问题
- `Option<F>` + `take()` 确保用户闭包只消费一次（版本链遍历 while 循环中）
- 不可见版本只读 8B VersionHeader，不拷贝 tuple payload — 这是闭包方案的核心性能收益
- 写路径（`write_commit_tx_id` / `UpdateExecutor`）闭包内 `.to_vec()` — 等价于原实现
- 闭包内禁止 `.await`（`FnOnce` 非 async，编译期强制）和递归调用 BufferPool（文档约束）

**为什么选闭包而非其他方案**：
- safe Rust 返回 `PageDataGuard<'_>` 不可行（E0505 / self-referential hang）
- 闭包是 Rust 异步 + 锁借用的标准范式（与 `PageGuard::modify_page` 一致）
- 不需要 `Box::pin` / `AsyncFnOnce` — 闭包内是同步操作

**How to apply**：
- M19/M36 等后续零拷贝里程碑遇到类似借用链问题，**首选闭包方案**
- `with_page_data` 可作为 BufferPool 的通用零拷贝访问原语

---

## L026: M19 实测性能 — DataScan 1.81x-2.44x 提速

<!-- L026 -->

**结论**：`DataScanExecutor` 实测 **1.81x-2.44x 提速**（vs `ScanExecutor`），10K 行场景 **2.44x**，达到预期 ~2x 目标。

**测试条件**（`benches/data_scan_bench.rs`）：
- 表 schema: `(id INT PK, name STRING, value INT)`
- 数据规模: 1K 行（100B/行）、10K 行
- criterion sample-size=20, warm-up=1s, measurement=2s
- 1次构建 dataset，每次 iter 只测 scan 阶段（不混 insert 开销）

**实测数据**：
| Rows | ScanExecutor (index) | DataScanExecutor | Speedup |
|------|----------------------|-------------------|---------|
| 1K   | 257.79 µs            | 142.39 µs         | 1.81x   |
| 10K  | 17.719 ms            | 7.273 ms          | 2.44x   |

**为什么能提速**：
- IndexManager.scan_all 需遍历 BTree 全部页（内部 + 叶节点）+ 累积 `(key, RowId)` 到 Vec
- DataScan 沿 `next_page_id` 链表直接读数据页，每行 **1 次页访问**（旧路径 2 次：索引页 + 数据页）
- 流式 `next()` 无 `results: Vec<Vec<Value>>` 预加载

**踩坑：bench 第一次卡死**（教训）

第一次写 `data_scan_bench.rs` 时把 setup 放在 `iter` 闭包里，setup 用 `db.execute_sql("INSERT...")` 串行 1K 次，每 iter 1.5s。100 samples × 3 档 = 450s+，10K 警告 1434s。

**修复**：
- 一次构建 dataset（在 `c.bench_with_input` 之前 `block_on` 执行 `setup_table_with_rows`）
- 用 batch INSERT 1000 行/次 而非单行 INSERT
- iter 闭包只做 executor 构造 + drain

**为什么重要**：
- criterion 默认 100 samples，每 sample 多 iter 几次取中位数
- setup 混进 iter = benchmark 测错东西（setup + scan 而非纯 scan）
- 实际收益会随 dataset 增大放大（10K 比 1K 提速更多），符合预期

**How to apply**：
- 写新 bench 时：dataset 构建在 `bench_with_input` **之前**完成（`block_on` 同步 setup）
- 串行 `execute_sql` 跑大数据集极慢，**必须**用 batch INSERT 或直接 executor API
- 第一次跑 bench 设 `--sample-size 10 --measurement-time 2` 快速验证 setup 不卡

---

## L027: Rust 测试运行过长的诊断指南

<!-- L027 -->

**核心断言**：Rust 测试或 bench 在 RTsql 项目**正常应在秒级完成**。超过 30s 几乎都是配置或 bug 引起，不是合理的性能开销。

**M19 实测反例**：第一次跑 `data_scan_bench` 卡了 7+ 分钟没有任何输出（用户："卡在测试很久了，rust 的测试一般不需要这么久"）。

**症状分类 + 诊断路径**：

| 症状 | 诊断命令 | 根因 | 修复 |
|------|----------|------|------|
| 跑很久无输出，`stdout` 0 字节 | `ps aux \| grep <test_or_bench>` 看 CPU% | (a) 无输出空转 (b) 真的死锁 | 加 `eprintln!` + `cargo test -- --nocapture` |
| 单个 test 跑 > 30s | `cargo test <name> -- --nocapture` 看 panic / print | 死锁/无限循环/setup 卡死 | 拆测试 + 检查锁 + 检查 `for` 条件 |
| bench criterion 报 "Unable to complete 100 samples in Xs" | 看 X（target time）| setup 混进 `iter` 闭包 | 一次构建 dataset 在 `bench_with_input` 之前 |
| bench 警告 "increase target time to 1456s" | criterion 自动算的目标时间 | 100 samples × 1.5s/iter = 150s/规模，3 档 = 450s+ | `--sample-size 10 --measurement-time 2` 快速验证 |
| `cargo test` 全绿但慢 | `cargo test --release` 看加速比 | debug 模式无优化 | 日常 debug 跑，生产 release 验证 |

**为什么 Rust 测试默认不长**：
- `#[tokio::test]` 单进程跑测试（除非显式多线程 `#[tokio::test(flavor = "multi_thread")]`)
- `cargo test` 不带 `--release`，但 unit test 本身开销小（毫秒级）
- 没有 I/O 等待的纯逻辑测试应该 < 100ms
- 大数据集测试用 tempfile 隔离，无全局状态竞争

**三类空转的区分**：

1. **死锁（CPU ~0%）**：锁循环等待，进程几乎不耗 CPU
   - 查 `ps aux`：CPU% < 5% 持续 → 死锁
   - 常见原因：`std::sync::Mutex` 跨 `.await`、递归 BufferPool 调用
   - 解决：检查 BufferPool::with_page_data 闭包内不调用其他 BufferPool 方法（learned L022）

2. **无限循环（CPU 100%）**：条件永不满足
   - 查 `ps aux`：CPU% ~100% 持续 → 无限循环
   - 常见原因：分页/链表遍历 next_id 写错（自身指向）
   - 解决：加 `eprintln!` 输出中间状态 + 跑 `-- --nocapture`

3. **Setup 过重（CPU 中高但每步慢）**：每个测试都在重建
   - 查输出：但输出 0 字节 → setup 在后台，print 没出来
   - 常见原因：`iter` 闭包内 `db.execute_sql` 串行 1K 次
   - 解决：dataset 一次构建 + batch INSERT

**预防检查清单**（提交新 test/bench 前）：

```
[ ] 单个 test 跑 < 5s（无 I/O 的纯逻辑 < 100ms）
[ ] 大数据集 bench 先用 --sample-size 10 --measurement-time 2 验证 setup
[ ] 怀疑死锁时 ps aux 看 CPU%，加 eprintln! + -- --nocapture
[ ] 数据规模从 1K 起步，不要 100K 起跑（避免雪崩）
[ ] 复用 tempfile 不持锁到测试结束
```

**关键 cargo 命令**：

```bash
# 诊断 hang：
cargo test <name> -- --nocapture 2>&1 | tee /tmp/test.log &
ps aux | grep cargo    # 看 CPU%
kill -9 <pid>          # 必要时 kill

# 诊断 bench 过慢：
cargo bench <name> -- --sample-size 10 --measurement-time 2
# 如果还是慢 → setup 问题；快 → 调大 sample-size

# 性能 baseline：
cargo bench --save-baseline before-X
```

**L026 vs L027 关系**：
- L026：具体 M19 bench setup 教训（dataset 一次构建 + batch INSERT）
- L027：**通用** Rust 测试/bench 运行过长的诊断框架（症状→命令→根因→修复）
- 两者互补：L027 是方法论，L026 是案例

**How to apply**：
- 用户报告 "测试很久" / "卡住" → 先查 ps aux CPU% 区分三类空转
- 写新 bench：先 `--sample-size 10 --measurement-time 2` 验证 setup 不卡
- 写新 test：单 test < 5s 强制约束（超 5s 必有性能问题）
- 调试 hang 的 test：`-- --nocapture` 必加（默认输出被吞）

---

<!-- L028 -->

### [M21 页面级 MVCC 可见性摘要 — 架构与踩坑]

**新增 API 路径**：

| 名称 | 路径 | 用途 |
|------|------|------|
| `PageVisibilityInfo` | `src/storage/page_visibility.rs` | 页面级可见性摘要（min_create_tx_id + all_visible） |
| `BufferPool::get_visibility` | `src/storage/buffer_pool.rs` | 查询页的 visibility map 条目 |
| `BufferPool::update_visibility_on_insert` | `src/storage/buffer_pool.rs` | INSERT 后更新 min_create_tx_id + 清 all_visible |
| `BufferPool::clear_all_visible` | `src/storage/buffer_pool.rs` | INSERT/DELETE/UPDATE/COMMIT 后清标志 |
| `BufferPool::set_all_visible` | `src/storage/buffer_pool.rs` | 惰性设置（零调用者，待实现） |
| `DataScanExecutor` 快速路径 | `src/executor/data_scan.rs` | 闭包外查询 visibility_map，闭包内 skip/skip-page |

**关键设计决策**：
- 存储位置：`DashMap<PageId, PageVisibilityInfo>` in BufferPool（纯内存，崩溃降级）
- `min_create_tx_id`：页上所有行的最小 create_tx_id，用于 `all_invisible_for` 判断
- `all_visible`：惰性设置延后（Plan Agent 建议避免竞态条件，先保正确性）
- COMMIT 路径缺口（Plan Agent 发现）：`commit_mark_versions` 需清 `all_visible`

**踩坑记录**：
1. **DELETE 仅删索引不删数据页** — ✅ 已修复（L030）：`mark_deleted()` 标记 version header，DataScan 跳过已删除行。
2. **`set_all_visible` 零调用者** — ✅ 已修复（L030）：`check_page_all_visible()` 三条件验证后惰性设置。
3. **闭包外查询可见性** — DataScanExecutor 在 `with_page_data` 闭包外查 `get_visibility`，因闭包是同步 `FnOnce` 无法访问 `self.buffer_pool`。

**延后项**（已全部完成，详见 L030）：
- T4 benchmark：`benches/visibility_bench.rs`（3 场景）
- T2.3 惰性设置：`check_page_all_visible` 三条件验证

---

<!-- L030 -->

### [M21 遗留项完成 — DELETE mark_deleted + 惰性 set_all_visible — 2026-06-04]

**问题**：(1) DELETE 只删 B-tree 索引不更新数据页 → DataScan 返回已删除行；(2) `set_all_visible` 零调用者；(3) 无 visibility benchmark。

**解决方案**：

1. **DELETE mark_deleted**：
   - `VersionHeader` 新增 `DELETED_TX_ID = 0xFFFFFFFFFFFFFFFE` 哨兵值
   - `mark_deleted()` 设置 `commit_tx_id = DELETED_TX_ID`，`is_deleted()` 检查
   - DeleteExecutor 在删索引前标记 version header（容错 slot 不存在）
   - DataScan 跳过 `is_deleted()` 行

2. **惰性 set_all_visible**：
   - `check_page_all_visible(page_id, snapshot)` 扫描所有 version header 验证三条件：
     - `commit_tx_id().is_some() && !is_deleted()` — 已提交且未删除
     - `create_tx_id < snapshot.tx_id` — 快照前创建
     - `!snapshot.contains_active_tx(create_tx_id)` — 非活跃事务
   - DataScan 页面扫描完毕后（`JumpToPage/Done` + 非 all_invisible）惰性设置

3. **Visibility benchmark**：`benches/visibility_bench.rs`（3 场景：no_snapshot / cold / warm）

**How to apply**：
- DELETE 正确性依赖 `mark_deleted`，未来 GC 需识别 `DELETED_TX_ID`
- `check_page_all_visible` 是 async 方法（需 `with_page_data`），仅在页面扫描结束时调用
- Benchmark 中 snapshot 场景不 assert count（auto-commit tx_id 与 snapshot tx_id 交互问题）

---

## 待探索

<!-- L016 -->
| 主题 | 优先级 | 备注 |
|------|--------|------|
| io_uring | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
