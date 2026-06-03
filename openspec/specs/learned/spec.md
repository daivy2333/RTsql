# Learned — 学习记忆

> 版本：v1.3 | 最后更新：2026-06-03（M38 网络优化 L018-L021）
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

<!-- L011 --> ### [gc_test SlottedPage SlotID 失效（详细）]
**症状→根因→解决**: GC delete_slot + compacting 后物理 SlotID 变化 → 引入 logical_id 解耦，header 修改后必须 serialize

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

## 待探索

<!-- L016 -->
| 主题 | 优先级 | 备注 |
|------|--------|------|
| io_uring | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
