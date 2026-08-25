## Purpose

记录已验证、非显然且可复用的项目知识。条目使用 `Kxx` 编号，按踩坑/技巧/性能数据/方法论分类。

## Requirements

### Requirement: 项目知识可复用

项目知识 SHALL 包含结论、证据、适用范围和边界。

#### Scenario: 验证非显然知识

- **WHEN** 问题根因或行为规律已经验证且可能再次使用
- **THEN** 使用递增 K 编号记录结论、证据、范围和边界

---

## 踩坑档案（已验证根因）

## K01: delete_by_key 并发 merge 位置偏移

- **结论**: B-Tree delete_by_key 路径中，merge 改变父节点结构后子节点索引失效
- **根因**: merge 后根节点 parent 关系被重写，迭代器持有的 child index 失效
- **解决**: `&mut self` + root_page_id 现场更新（每次 merge 完成后重新从 root 走）
- **适用范围**: 任何 B-Tree 实现中需要先 redistribute 再 merge 的删除路径
- **证据**: `src/storage/btree/btree.rs:delete_by_key`
- **Legacy**: L003

## K02: merge 容量溢出

- **结论**: 简单 merge 可能超过 leaf 容量上限
- **根因**: min_keys=48, leaf 容量=92, 47+48=95>92（min_keys 来自左叶 + 右叶 entries）
- **解决**: redistribution-first + can_merge_with 拦截（merge 前先判断能否放下，不能则 redistribute）
- **证据**: `src/storage/btree/btree.rs:can_merge_with`
- **Legacy**: L004

## K03: gc_test SlotID 失效

- **结论**: compacting 改变物理 SlotID，版本链引用旧值导致 GC 测试 panic
- **根因**: SlottedPage compact 后 slot 物理位置变化，VersionHeader.next_version 仍指向旧 slot
- **解决**: logical_id 解耦（Slot 4B→6B，logical_id 永不复用，跨 compact 稳定）
- **适用范围**: 所有需要 stable row 标识的 slot-based 存储
- **证据**: `src/storage/page_format/slotted_page.rs`（Slot layout 6B）
- **Legacy**: L005, M04

## K04: delete_slot 不序列化 header 导致连锁 panic

- **结论**: 修改 SlottedPage header 后必须 serialize 回 page.data
- **根因**: slot_count 修改只在内存（page_data_mut），未 serialize → 后续访问看到陈旧 header
- **解决**: header 修改函数必须 serialize 末尾（mod.rs 集中管理）
- **副作用**: 第一个 panic poison BufferPool Mutex → 引入 logical_id 根本修复（K03）
- **证据**: `src/storage/page_format/slotted_page.rs:delete_slot`
- **Legacy**: L006, L007

## K05: RecoveryManager 需要表才能 redo

- **结论**: 完整 redo 恢复要求表定义持久化
- **根因**: `get_table(name)` 失败时 redo 静默跳过（导致数据丢失）
- **解决**: 表定义持久化（M44 计划解决）；当前 WAL 测试策略：直接读 WAL 验证而非重启验证
- **证据**: `tests/wal_recovery_test.rs:recovery_e2e_test`, `src/wal/recovery.rs`
- **Legacy**: L008, L014

## K06: get_subquery_first_column 不支持 SemiJoin/AntiJoin

- **结论**: 嵌套 IN 子查询 SemiJoin 节点未被处理 → 标量提取失败
- **根因**: 原实现只处理 SubqueryEval 节点，遗漏 SemiJoin/AntiJoin 路径
- **解决**: 添加 SemiJoin/AntiJoin 分支 + output_columns 字段
- **证据**: `src/executor/`
- **Legacy**: L009

## K07: inner_column_index 设计失误（关联子查询参数匹配）

- **结论**: 用 usize 索引匹配 CorrelatedParam 容易越界且对列重排不稳健
- **根因**: 列顺序变化导致索引错位
- **解决**: 改为 `param_name: String` 按列名匹配
- **证据**: `src/executor/correlated.rs:ParameterExpression`
- **Legacy**: L010

## K08: 闭包零拷贝 API 的 self-referential hang（3 次失败）

- **结论**: safe Rust 中 `MutexGuard` 借用链无法跨越函数调用边界，`Result<PageDataGuard<'_>>` 形式不可达
- **尝试 1**: tuple `(PageGuard, PageDataGuard<'_>)` → E0505 cannot move out of borrowed
- **尝试 2**: unsafe self-referential struct（Arc + raw pointer + 'static transmute）→ 编译通过但 hang
- **尝试 2 hang 根因推测**:
  - eprintln! 内部获取 stderr 锁与 arc.lock() 锁顺序冲突
  - transmute 后 MutexGuard drop 状态损坏
  - current_thread tokio runtime 与 std::sync::Mutex 锁争用
- **解决方案**: 闭包式 API — `pub async fn with_page_data<F, R>(&self, page_id, f: F) -> Result<R> where F: FnOnce(&[u8]) -> R`
- **适用范围**: 所有需要 zero-copy + async + 锁借用的场景（M19/M36/M20/M31 都采用此范式）
- **证据**: `src/storage/buffer_pool.rs:with_page_data`
- **预防**:
  - cargo test 默认吞 stderr，hang 排查必须 `-- --nocapture`
  - 闭包形式是 Rust 异步 + 锁借用的标准范式
- **Legacy**: L022

## K09: M20 闭包方案最终设计

- **决策**: M20 全面采用闭包 API 替代原 get_page_ref / PageDataGuard<'_>
- **核心 API**:
  1. `BufferPool::with_page_data<F, R>(&self, PageId, F) -> Result<R>` — `F: FnOnce(&[u8]) -> Result<R>`
  2. `read_tuple_from_data_page<F, R>(&BufferPool, RowId, F) -> Result<R>`
  3. `find_visible_version<F, R>(&self, RowId, &Snapshot, F) -> Result<Option<R>>`
- **关键设计点**:
  - `VisibilityResult<R>` 辅助枚举解决 find_visible_version 可见/不可见分支返回不同类型
  - `Option<F> + take()` 确保闭包只消费一次（版本链遍历 while 循环中）
  - 不可见版本只读 8B VersionHeader，不拷贝 tuple payload
  - 写路径闭包内 .to_vec()（等价于原实现）
  - 闭包内禁止 .await（FnOnce 非 async 编译期强制）
- **证据**: `src/storage/buffer_pool.rs`
- **Legacy**: L023

## K10: M31 per-page loading lock 设计修正

- **结论**: miss Semaphore 不能替代 per-page lock 保证 R3 double-check 正确性
- **根因**: 原 design 只提 miss Sem，但 DashMap 无全局写锁无法保证同 page 多次 miss 只 1 次 read_page
- **解决**: 加 `loading_locks: DashMap<PageId, Arc<tokio::sync::Mutex<()>>>` 字段；loading_lock 持锁下做 double-check + load + insert
- **锁顺序约定**: `miss_sem → loading_lock → pages → clock_hand → frame`
- **影响**: flush_all 不能再用读锁迭代（DashMap iter 持 per-shard 读锁，但 await 不能持锁）→ collect-then-write 模式
- **证据**: `src/storage/buffer_pool.rs`
- **Legacy**: L031, D12

## K11: M31 锁顺序约定

- **结论**: 多层 DashMap + Semaphore + Mutex 嵌套时锁顺序决定是否会死锁
- **顺序**: `miss_sem.acquire() → loading_lock.lock() → pages.get() → clock_hand.read() → frame.lock()`
- **Why**: 同顺序获取保证无环；不同顺序会导致 cross-task 死锁
- **证据**: `src/storage/buffer_pool.rs`
- **Legacy**: D12, L031

## K12: DELETE mark_deleted 模式

- **结论**: 索引路径 DELETE 只删 B-tree 索引不更新数据页 → DataScan 返回已删除行
- **解决**:
  - VersionHeader 新增 `DELETED_TX_ID = 0xFFFFFFFFFFFFFFFE` 哨兵值
  - `mark_deleted()` 设置 `commit_tx_id = DELETED_TX_ID`，`is_deleted()` 检查
  - DeleteExecutor 在删索引前标记 version header（容错 slot 不存在）
  - DataScan 跳过 `is_deleted()` 行
- **GC 影响**: 未来 GC 需识别 `DELETED_TX_ID` 哨兵值
- **证据**: `src/storage/data_page.rs:DELETED_TX_ID`, `src/executor/delete.rs`
- **Legacy**: L030

## K13: 惰性 set_all_visible 三条件验证

- **结论**: 页面扫描结束后惰性设置 all_visible 必须验证三条件
- **条件**:
  1. `commit_tx_id().is_some() && !is_deleted()` — 已提交且未删除
  2. `create_tx_id < snapshot.tx_id` — 快照前创建
  3. `!snapshot.contains_active_tx(create_tx_id)` — 非活跃事务
- **调用时机**: DataScan 页面扫描完毕后（JumpToPage/Done + 非 all_invisible）
- **证据**: `src/storage/page_visibility.rs:check_page_all_visible`
- **Legacy**: L030

## K14: 连接并发 Semaphore 模式（M30）

- **结论**: tokio::spawn 内 acquire_owned().await 让外层 accept 循环不阻塞
- **要点**:
  - `_permit` 随 handler 生命周期释放，连接断开自动归还
  - `select! + semaphore` hold：获取 permit 后 spawn 才返回
  - TcpStream connect() 不阻塞；用 `read(&mut [0])` 验证连接仍存活
- **证据**: `src/network/server.rs`, `tests/connection_limit_test.rs`
- **Legacy**: L018, L019

## K15: TCP_NODELAY + write_buf 批写最佳实践（M38）

- **结论**: set_nodelay(true) 在 accept 后、spawn 前调用；write_buf Vec<u8> 优于 bytes::BytesMut
- **证据**: `src/network/pg_protocol.rs`, `src/network/server.rs`
- **Legacy**: L020, L021

## K16: M41 AtomicU64 实测性能数据

- **结论**: 实测 5.1 ns/op（路线图"100ns→10ns"达成），4.5-4.6x 争用加速
- **场景数据**:

  | 场景 | Mutex (ns/op) | Atomic (ns/op) | 加速比 |
  |------|--------------|----------------|--------|
  | 单线程 1M | 10.7 | 5.1 | 2.1x |
  | 10 线程 × 100K | 84.7 | 18.6 | 4.6x |
  | 100 线程 × 10K | 100.8 | 22.5 | 4.5x |
  | 吞吐@1M (单线程) | 90.8 Melem/s | 138.1 Melem/s | 1.52x |

- **关键发现**:
  - 单线程差异假设（< 20%）被推翻：实际 Atomic 在单线程下也 2x 快（锁自身开销不可忽略）
  - 10/100 线程争用下 Atomic 5x 加速假设全部满足
- **基准**: `benches/tx_id_bench.rs`（criterion，4 场景）
- **关联决策**: D09
- **Legacy**: L017

## K17: M20 zero-copy SlottedPageRef 性能实测

- **结论**: 闭包方案下 read 路径 -2.46%~-8.33% 提速，**未达 15% 目标**
- **数据** (`benches/single`):

  | Benchmark | Before-m20 | After-m20 | Change |
  |-----------|------------|-----------|--------|
  | delete/by_pk | 172.38 ms | 158.03 ms | -8.33% |
  | filter/where_value_gt_500 | 33.714 µs | 32.768 µs | -3.53% |
  | sort/order_by_value_desc | 49.123 µs | 46.618 µs | -4.56% |
  | join/inner_join | 53.003 µs | 51.563 µs | -2.46% |
  | scan/full_table | 36.089 µs | 36.665 µs | +2.04% (噪声) |
  | limit/limit_10_offset_5 | 24.063 µs | 23.705 µs | -0.60% |
  | update/single_column | ~75 ms | 78.006 ms | +3.99% |

- **未达 15% 根因**:
  - micro_bench 数据 1K 行 ~100B/行，每次 to_vec() 分配 100KB
  - jemalloc/Rust 全局分配器对 100KB Vec 分配已非常快
  - 关键收益是消除 N×4KB 页内 slice 拷贝（隐式），1K 行规模下也有限
- **写路径 +3.99% 回归**: design.md 决策 5 — 写路径闭包内 .to_vec()，微小回归来自闭包调用栈额外开销
- **How to apply**:
  - M19/M36 应进一步消除分配，可能达 15%+
  - 真实场景（更大行/批量分配）零拷贝收益更明显
- **证据**: `benches/single`, commit `f64c874`
- **Legacy**: L024

## K18: M36 zero-copy ValueRef 性能与局限

- **结论**: M36 实施后实测数据仅供回归参考，**5% 提速 + 30万→0 分配 目标未直接验证**
- **未验证根因**:
  - M36 实施前未保存 before-m36 baseline（plan T9 步骤 1 设计缺陷 — 应在 M36 改动前保存）
  - micro_bench 当前用 Int 列，**M36 主要消除 String 分配，对 Int-only 场景收益不直接可测**
  - git stash 与 master 上其它改动冲突，无法干净 stash M36 改动
- **after-m36 数据**:

  | Benchmark | After-m36 | 场景 |
  |-----------|-----------|------|
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

- **实际收益场景**: String 列 ≥ 100B 的 Scan 路径，1K 行可减 30万次 String 分配
- **How to apply**:
  - 未来 zero-copy 改进（M19/M37）实施前应先 `cargo bench --save-baseline before-X`
  - micro_bench 需扩展 String 列场景
  - M36 设计本身正确（闭包内 deserialize_value_refs 借用 + .to_value() 消费）
- **关联**: K09 (M20 经验对照)
- **Legacy**: L025

## K19: M19 DataScan 1.81x-2.44x 实测提速

- **结论**: DataScanExecutor 实测 1.81x-2.44x 提速（vs ScanExecutor），10K 行场景 2.44x
- **测试条件** (`benches/data_scan_bench.rs`):
  - 表 schema: `(id INT PK, name STRING, value INT)`
  - 数据规模: 1K 行（100B/行）、10K 行
  - criterion sample-size=20, warm-up=1s, measurement=2s
  - 1次构建 dataset，每次 iter 只测 scan 阶段
- **实测**:

  | Rows | ScanExecutor (index) | DataScanExecutor | Speedup |
  |------|----------------------|-------------------|---------|
  | 1K   | 257.79 µs            | 142.39 µs         | 1.81x   |
  | 10K  | 17.719 ms            | 7.273 ms          | 2.44x   |

- **提速根因**:
  - IndexManager.scan_all 需遍历 BTree 全部页 + 累积 (key, RowId) 到 Vec
  - DataScan 沿 next_page_id 链表直接读数据页，每行 1 次页访问（旧 2 次）
  - 流式 next() 无 results: Vec<Vec<Value>> 预加载
- **踩坑教训**: 第一次写 bench 时把 setup 放在 iter 闭包里（setup 用 db.execute_sql("INSERT...") 串行 1K 次，每 iter 1.5s；100 samples × 3 档 = 450s+）
- **修复**:
  - 一次构建 dataset（在 c.bench_with_input 之前 block_on 执行 setup_table_with_rows）
  - 用 batch INSERT 1000 行/次 而非单行 INSERT
  - iter 闭包只做 executor 构造 + drain
- **证据**: `benches/data_scan_bench.rs`
- **Legacy**: L026

## K20: Rust 测试运行过长的诊断框架

- **核心断言**: Rust 测试或 bench 在 RTsql 项目**正常应在秒级完成**。超过 30s 几乎都是配置或 bug 引起
- **症状分类 + 诊断路径**:

  | 症状 | 诊断命令 | 根因 | 修复 |
  |------|----------|------|------|
  | 跑很久无输出 stdout 0 字节 | `ps aux \| grep <test_or_bench>` 看 CPU% | (a) 无输出空转 (b) 真的死锁 | 加 eprintln! + `cargo test -- --nocapture` |
  | 单个 test 跑 > 30s | `cargo test <name> -- --nocapture` | 死锁/无限循环/setup 卡死 | 拆测试 + 检查锁 + 检查 for 条件 |
  | bench criterion 报 "Unable to complete 100 samples in Xs" | 看 X（target time）| setup 混进 iter 闭包 | 一次构建 dataset 在 bench_with_input 之前 |
  | bench 警告 "increase target time to 1456s" | criterion 自动算的目标时间 | 100 samples × 1.5s/iter = 150s/规模 | --sample-size 10 --measurement-time 2 快速验证 |
  | cargo test 全绿但慢 | cargo test --release 看加速比 | debug 模式无优化 | 日常 debug 跑，生产 release 验证 |

- **三类空转的区分**:
  1. **死锁（CPU ~0%）**: 锁循环等待，进程几乎不耗 CPU。`ps aux`: CPU% < 5% 持续
  2. **无限循环（CPU 100%）**: 条件永不满足。`ps aux`: CPU% ~100% 持续
  3. **Setup 过重（CPU 中高但每步慢）**: 每个测试都在重建
- **关键 cargo 命令**:
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
- **预防检查清单** (提交新 test/bench 前):
  - [ ] 单个 test 跑 < 5s（无 I/O 的纯逻辑 < 100ms）
  - [ ] 大数据集 bench 先用 --sample-size 10 --measurement-time 2 验证 setup
  - [ ] 怀疑死锁时 ps aux 看 CPU%，加 eprintln! + -- --nocapture
  - [ ] 数据规模从 1K 起步，不要 100K 起跑
  - [ ] 复用 tempfile 不持锁到测试结束
- **Legacy**: L027

## K21: 性能 baseline 一定要实施前留（M36 教训）

- **结论**: 性能优化前必须 `cargo bench --save-baseline before-X`，否则无 before 参照无法验证目标
- **反例**: M36 实施时 T9 步骤 1 跑 baseline 时已是 M36 实施后状态（git stash 与 master 上其它改动冲突，无法干净 stash M36 改动）
- **正确流程**:
  1. 实施前：`cargo bench --save-baseline before-X`
  2. 实施
  3. 实施后：`cargo bench -- --baseline before-X` 对比
- **适用范围**: 任何性能优化里程碑（M19/M37/M22/M23/M24...）
- **Legacy**: K18, L025

## K22: 数据页链表遍历是 M19 提速关键

- **结论**: SlottedPageHeader.next_page_id 链表遍历是全表扫描跳过索引层的关键
- **机制**:
  - TableMeta 记录 data_page_head
  - 每页 next_page_id 链向下页
  - 末尾 next_page_id = 0 返回 Ok(None) 终止
- **数据流**: DataScan → TableMeta.data_page_head → SlottedPageHeader.next_page_id → ... → 0
- **证据**: `src/storage/page_format/slotted_page.rs:21`（SlottedPageHeader）, `src/storage/data/table_manager.rs:51`（TableMeta.data_page_head）
- **Legacy**: L001, D11, M19 spec

## 技巧模式

## K23: 零拷贝页读取

- **模式**: `with_page_data(&self, PageId, FnOnce(&[u8]) -> R) -> Result<R>`
- **适用**: 只读页访问场景（scan, MVCC check, etc.）
- **证据**: `src/storage/buffer_pool.rs`
- **Legacy**: L012

## K24: 两阶段锁加载缺失页

- **模式**: 读锁→释放→I/O→写锁(double-check)
- **适用**: 缓存加载（避免持锁 await 阻塞其他 reader）
- **M31 演进**: 被 DashMap + per-page loading_locks 替代（K10, K11）
- **证据**: `src/storage/buffer_pool.rs`
- **Legacy**: L012, M07

## K25: AtomicPageId 无锁读

- **模式**: `AtomicU64::load(Acquire)` 异步无锁读根页 ID
- **适用**: 任何 async 路径中需要访问 B-Tree 根节点
- **证据**: `src/storage/btree/index_manager.rs`
- **Legacy**: L012, M08

## K26: 临时 BTree 实例 + spawn_blocking 写操作

- **模式**: `BTree::from_root(root_id) + spawn_blocking(|| btree.insert(...))`
- **适用**: B-Tree 写操作保持 sync 接口（避免 async lock 跨越）
- **证据**: `src/storage/btree/btree.rs:from_root`
- **Legacy**: L012

## K27: Hash Aggregation

- **模式**: `HashMap<Vec<Value>, Vec<AggregateState>>`
- **适用**: GROUP BY 聚合
- **证据**: `src/executor/aggregate.rs`
- **Legacy**: L012

## K28: Mutex 参数注入实现相关子查询

- **模式**: `ParameterExpression + Mutex<Value>` — 外层行值注入到子查询谓词
- **适用**: 标量/Semi/Anti 关联子查询
- **证据**: `src/executor/predicate.rs:ParameterExpression`, `src/executor/correlated.rs`
- **Legacy**: L012, K07

## K29: 双路径执行器（普通 vs 关联子查询）

- **模式**: `correlated_params` 非空走按行重建
- **适用**: SemiJoin/AntiJoin/SubqueryEval 等支持关联子查询的执行器
- **证据**: `src/executor/semi_join.rs`, `src/executor/anti_join.rs`
- **Legacy**: L012

## K30: 非唯一索引同页多条目

- **模式**: LeafNode 去掉 DuplicateKey 检查
- **适用**: 允许重复 key 的二级索引（M17）
- **证据**: `src/storage/btree/node.rs:LeafNode`
- **Legacy**: L012

## K31: 批量删除从后向前

- **模式**: `delete_by_key` 从后向前删除 slot
- **适用**: 批量删除同页多个 slot（避免 compacting 中间抖动）
- **证据**: `src/storage/btree/btree.rs:delete_by_key`
- **Legacy**: L012

## K32: 两次加载页模式

- **模式**: 先 `page_data()` 读找，再 `modify_page()` 删除
- **适用**: 页面读写分离（避免 modify 后才能读的场景）
- **证据**: `src/storage/buffer_pool.rs`
- **Legacy**: L012

## K33: 惰性初始化执行器

- **模式**: `search_all` 在首次 `next()` 调用时执行
- **适用**: IndexScanAll（延迟到真正消费时才查询）
- **证据**: `src/executor/index_scan_all.rs`
- **Legacy**: L012

## K34: tempdir leak 模式（独立 WAL benchmark）

- **模式**: `std::mem::forget(dir)` 保证 WAL 文件存活
- **适用**: 独立 WAL 层 benchmark（不经过 SQL 层）
- **证据**: `benches/wal_group_commit_bench.rs`
- **Legacy**: L014

## K35: 性能 baseline 技巧集

- **技巧 1**: 共享 tokio::runtime — 避免 per-iteration 创建 runtime
- **技巧 2**: RTsqlDirect in-process — 直接调用 API，避免 network overhead
- **技巧 3**: criterion Throughput — 设置 throughput 更准确测量
- **技巧 4**: `#[inline(never)] + std::hint::black_box` — 防止编译器消除 fetch_add 真实开销
- **技巧 5**: `std::thread::spawn + Arc` 共享计数器 — 多线程争用基准（避免 rayon 依赖）
- **证据**: `benches/sqlite_compare.rs`, `benches/tx_id_bench.rs`
- **Legacy**: L015

## 待探索

## K36: io_uring 集成可能性

- **优先级**: 低
- **备注**: Linux 5.1+，需 tokio-uring；M45 改进项
- **关联改进**: I028

## K37: jemalloc/mimalloc 内存分配器优化

- **优先级**: 低
- **备注**: 减少 String/Vec 分配开销；M36 收益在 String 列场景可能放大
- **关联改进**: 隐含于 I020 (clone 消除)
