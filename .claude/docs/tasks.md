# 任务与里程碑

> 最后更新：2026-05-25（精简合并，优化顺序按 Phase 重排，新增 M43-M48）

## 当前阶段：全维度性能优化 + 功能完善 + 并发控制

### 执行顺序（5 个 Phase，Phase 内可并行）

```
Phase 1 基础设施（改动小、风险低、后续都依赖）
  M41 → M30 → M38

Phase 2 存储引擎核心（读写路径重构）
  M20 → M19 → M21
  M20 → M36

Phase 3 并发控制（存储引擎稳定后才改锁）
  M31 → M40
  M34 → M32 → M42
  M48（独立）

Phase 4 上层功能（独立，可与 P2/P3 并行）
  M24 | M25 → M26 | M27 → M28 | M29 | M37 | M39 | M44

Phase 5 高级优化（依赖前面完成）
  M19+M31 → M22
  M23 → M33
  M31 → M35
  M43 | M45
```

### 依赖关系图

```
M41 ──→ M40        M20 ──→ M19 ──→ M21 ──→ M22(P5)
  └──→ M48         M20 ──→ M36 ──→ M37(P4)
M30（独立）         M34 ──→ M32 ──→ M42
M38 ──→ M29        M25 ──→ M26
M31 ──→ M35(P5)    M27 ──→ M28
  └──→ M22(P5)     M20 ──→ M39
M23 ──→ M33(P5)
```

---

## 优化路线图

| Phase | M | 优化项 | 预期收益 | 风险 |
|-------|---|--------|---------|------|
| **P1** | **M41** | 事务 ID AtomicU64 | 分配延迟 100ns→10ns | 低 |
| | **M30** | 连接并发 Semaphore | 防连接风暴 | 低 |
| | **M38** | BufWriter + TCP_NODELAY | write 调用 -99% | 低 |
| **P2** | **M20** | 零拷贝 SlottedPageRef | I/O ~20-30% 提速 | 低 |
| | **M19** | DataScan 路径 | 全表扫描 ~2x | 中 |
| | **M21** | 页面级 MVCC | ~10-15% 提速 | 中 |
| | **M36** | 零拷贝 ValueRef | 堆分配 30万→0 | 中 |
| **P3** | **M31** | BufferPool DashMap+Semaphore | 并发读吞吐提升 | 低 |
| | **M40** | RowLockTable DashMap | 行锁争抢 -5-10x | 低 |
| | **M34** | WAL fsync 合并 | TPS 3-10x | 低 |
| | **M32** | WAL 写入背压 | 缓冲区限流 | 低 |
| | **M42** | 消息传递重构 | WAL 锁消除 | 中 |
| | **M48** | pread/pwrite 替代 seek+read | syscall -50% | 低 |
| **P4** | **M24** | 多隔离级别 | SQL 标准支持 | 高 |
| | **M25** | 多 Join 算法 | 小表 join 提速 | 中 |
| | **M26** | 代价模型+Join 重排 | 最优执行计划 | 中 |
| | **M27** | 关联子查询缓存 | 重复参数免执行 | 中 |
| | **M28** | 多层关联子查询 | 嵌套查询支持 | 低 |
| | **M29** | PG Extended Query | 预编译+二进制传输 | 中 |
| | **M37** | clone 消除 Arc/Cow | clone 开销 -90% | 中 |
| | **M39** | INSERT 批量执行 | 1000行 5-10x | 中 |
| | **M44** | 表定义持久化 | 重启不丢 schema | 中 |
| **P5** | **M22** | 预取 Prefetch | 大表 ~15-25% | 低 |
| | **M23** | Varint Key | 索引空间 ~70% 缩减 | 中 |
| | **M33** | B+Tree 节点级锁 | 并发索引访问 | 高 |
| | **M35** | 脏页 writev | Checkpoint 5-10x | 低 |
| | **M43** | 并行扫描 | 多核扫描提速 | 中 |
| | **M45** | io_uring | I/O 延迟 -30-50% | 高 |

---

## 已完成（M1-M18）

| 里程碑 | 内容 | 日期 |
|--------|------|------|
| M1-M6 | 项目初始化+SQL 解析+执行引擎+存储+事务+索引 | 2026-03 |
| M7-M12 | WAL+MVCC+Hash Join+子查询+B+Tree Split/Merge+PG 协议 | 2026-04 |
| M13-M18 | 聚合+关联子查询+B+Tree Redistribute+并发+优化器+基准测试 | 2026-05 |

---

## 详细规划

### Phase 1: 基础设施

#### M41: 事务 ID AtomicU64 无锁分配

- **问题**：`next_tx_id()` 用 `Mutex<u64>`，每次事务开始等锁
- **任务**：
  - [ ] T1: `TransactionId::counter` 改为 `AtomicU64`
  - [ ] T2: `next_tx_id()` 改用 `fetch_add(1, SeqCst)`
  - [ ] T3: 事务开始时间戳同理改为 `AtomicU64`
  - [ ] T4: 微基准测试（Mutex vs AtomicU64）

---

#### M30: 连接并发上限

- **问题**：PG 连接无限 `tokio::spawn`，连接风暴压垮系统
- **任务**：
  - [ ] T1: `Server` 新增 `Arc<Semaphore>` 字段，配置 `max_connections`（默认 64）
  - [ ] T2: accept 循环 `acquire_owned().await` 后再 spawn
  - [ ] T3: 连接结束 `drop(permit)` 释放
  - [ ] T4: 并发连接压测 + 超限测试

---

#### M38: 网络 BufWriter + TCP_NODELAY

- **问题**：DataRow 逐行 `write_all()` + `flush()`，每行一次 syscall
- **任务**：
  - [ ] T1: 连接流包裹 `BufWriter`，8KB 缓冲
  - [ ] T2: `TCP_NODELAY` 设置
  - [ ] T3: SELECT 结果累积 `BytesMut`，一次 write+flush
  - [ ] T4: 网络延迟基准测试

---

### Phase 2: 存储引擎核心

#### M20: 零拷贝 SlottedPageRef

- **问题**：`SlottedPage::get()` 返回 `Vec<u8>` 拷贝
- **任务**：
  - [ ] T1: `SlottedPageRef<'_>` 只读视图（借用页缓冲区）
  - [ ] T2: `LeafNodeRef` / `InternalNodeRef` 零拷贝（已有设计）
  - [ ] T3: `BufferPool::get_page_ref()` 返回零拷贝引用
  - [ ] T4: 执行器 Scan 路径改用零拷贝

---

#### M19: DataScan 路径

- **问题**：Index→RowId→Data 每行两次页访问
- **任务**：
  - [ ] T1: `DataScanExecutor` 顺序扫描数据页，跳过索引层
  - [ ] T2: 无 WHERE 条件时 Planner 自动选 DataScan
  - [ ] T3: 有 WHERE 但无索引覆盖时也走 DataScan + 过滤
  - [ ] T4: 全表扫描基准测试对比

---

#### M21: 页面级 MVCC

- **问题**：每行 16B VersionHeader，逐行检查可见性
- **任务**：
  - [ ] T1: `PageVisibilityMap` 每页 4B 摘要（min_tx_id / 全可见标志）
  - [ ] T2: 快照 tx_id < min_tx_id → 跳过整页检查
  - [ ] T3: INSERT/DELETE 时更新页面摘要
  - [ ] T4: 可见性检查基准测试

---

#### M36: 零拷贝 ValueRef

- **问题**：`Expression::evaluate()` 每行每列返回 `Value` 枚举，String/Vec 分配
- **任务**：
  - [ ] T1: `ValueRef<'a>` 枚举（Text→`&'a str`，Bytes→`&'a [u8]`）
  - [ ] T2: `Expression::evaluate_ref()` 返回 `ValueRef<'_>`
  - [ ] T3: 执行器适配 ValueRef 路径
  - [ ] T4: 需所有权时 `ValueRef::to_owned()` 转换
  - [ ] T5: 堆分配计数基准测试

---

### Phase 3: 并发控制

#### M31: BufferPool DashMap + Semaphore

- **问题**：`Arc<Mutex<HashMap>>` 读写都互斥
- **任务**：
  - [ ] T1: `pages` 改为 `DashMap<PageId, Frame>`（分片无锁读）
  - [ ] T2: 引入 `Semaphore(BUFFER_POOL_SIZE)` 限制 pin 页数
  - [ ] T3: PageGuard acquire 语义适配
  - [ ] T4: 并发读写基准测试

---

#### M40: RowLockTable DashMap

- **问题**：`Arc<Mutex<HashMap>>` 行锁获取/释放串行化
- **任务**：
  - [ ] T1: 引入 `dashmap` crate
  - [ ] T2: `locks` 改为 `DashMap<RowId, Arc<Mutex<()>>>`
  - [ ] T3: `get_lock()` 改用 `DashMap::entry()` API
  - [ ] T4: 并发锁获取基准测试

---

#### M34: WAL fsync 合并

- **问题**：每事务提交单独 fsync，系统调用开销巨大
- **任务**：
  - [ ] T1: `tokio::time::interval` 定时器，与 commit 触发并存
  - [ ] T2: 刷盘窗口累积多条记录，一次 `write_all` + 一次 `fsync`
  - [ ] T3: 可配置 `wal_writer_delay`（默认 2ms）
  - [ ] T4: TPS 基准测试对比

---

#### M32: WAL 写入背压

- **问题**：WAL 无背压，高并发缓冲区膨胀
- **任务**：
  - [ ] T1: `Semaphore(WAL_MAX_PENDING)` 限制等待刷盘事务数
  - [ ] T2: `append_commit_and_wait()` 中 acquire permit
  - [ ] T3: `do_flush()` 完成后释放 permit
  - [ ] T4: 高并发写入压测

---

#### M42: 消息传递重构

- **问题**：多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**：WAL Writer→mpsc，提交等待→oneshot，Checkpoint→Notify，BufferPool→watch
- **任务**：
  - [ ] T1: WALBuffer `buffer: Mutex<Vec>` → `mpsc::channel`
  - [ ] T2: `commit_waiters: Mutex<HashMap>` → `oneshot::channel`
  - [ ] T3: Checkpoint 触发改用 `Notify`
  - [ ] T4: BufferPool eviction 改用 `watch::channel`
  - [ ] T5: 集成测试 + 性能对比

---

#### M48: pread/pwrite 替代 seek+read

- **问题**：文件读写用 `seek()+read()/write()` 两次 syscall，且非原子
- **任务**：
  - [ ] T1: `FileExt::read_at()` / `write_at()` 改用 `pread`/`pwrite`（单次 syscall）
  - [ ] T2: 验证 `tokio::fs::File` 已支持 `read_at`/`write_at`（底层即 pread/pwrite）
  - [ ] T3: 去除 `seek` 调用，所有文件操作基于 offset 参数
  - [ ] T4: syscall 计数对比（strace 验证）

---

### Phase 4: 上层功能

#### M24: 多隔离级别

- **问题**：只有 Repeatable Read
- **方案**：Read Committed（每语句刷新 snapshot）+ Serializable（SSI + 写偏序检测）
- **任务**：
  - [ ] T1: `IsolationLevel` 枚举
  - [ ] T2: `BEGIN TRANSACTION ISOLATION LEVEL ...` 语法
  - [ ] T3: Read Committed 实现
  - [ ] T4: Serializable SSI 实现
  - [ ] T5: ANSI SQL 隔离级别测试

---

#### M25: 多 Join 算法

- **问题**：只有 Hash Join
- **任务**：
  - [ ] T1: `JoinAlgorithm` 枚举 + PhysicalPlan 扩展
  - [ ] T2: `NestedLoopJoinExecutor` 实现
  - [ ] T3: `SortMergeJoinExecutor` 实现
  - [ ] T4: 启发式算法选择（小表 NLJ，有序 SMJ，默认 HJ）
  - [ ] T5: Join 算法基准测试

---

#### M26: 代价模型 + Join 重排

- **问题**：固定 join 顺序，无 cardinality/selectivity
- **任务**：
  - [ ] T1: `TableStatistics` 结构（行数/NDV/min/max/null_count）
  - [ ] T2: `ANALYZE TABLE` 命令 + 持久化
  - [ ] T3: `CostEstimator`（scan/join/filter 代价）
  - [ ] T4: Join 重排序（DP <10 表，贪心 ≥10）
  - [ ] T5: 代价驱动执行计划测试

---

#### M27: 关联子查询缓存

- **问题**：每行外层重新执行子查询
- **任务**：
  - [ ] T1: `SubqueryCache` 结构（参数值→结果集）
  - [ ] T2: `SubqueryEvalExecutor` 集成缓存
  - [ ] T3: LRU 淘汰 / 事务结束清空
  - [ ] T4: 关联子查询性能基准测试

---

#### M28: 多层关联子查询

- **问题**：显式拒绝多层嵌套
- **任务**：
  - [ ] T1: `extract_correlated_params` 改为递归遍历
  - [ ] T2: `inject_correlated_values` 支持多层注入
  - [ ] T3: 移除拒绝逻辑
  - [ ] T4: 多层嵌套测试用例

---

#### M29: PG Extended Query Protocol

- **问题**：只有 Simple Query
- **任务**：
  - [ ] T1: Parse/Bind/Describe/Execute 消息解析与序列化
  - [ ] T2: Prepared Statement 缓存与生命周期
  - [ ] T3: 二进制格式 DataRow 编码
  - [ ] T4: Close/Sync/Flush 消息
  - [ ] T5: psql 预编译语句集成测试

---

#### M37: clone 消除 Arc/Cow

- **问题**：`Value::clone()` 在聚合/排序/JOIN 中反复调用
- **任务**：
  - [ ] T1: `Value::Text` 内部 `Arc<str>` 替代 `String`
  - [ ] T2: `Value::Bytes` 内部 `bytes::Bytes` 替代 `Vec<u8>`
  - [ ] T3: SQL 解析阶段 `Cow<'_, str>` 延迟分配
  - [ ] T4: clone 计数基准测试

---

#### M39: INSERT 批量执行

- **问题**：多值 INSERT 逐行执行
- **任务**：
  - [ ] T1: `InsertExecutor` 检测多值 INSERT，收集后批量执行
  - [ ] T2: B+Tree `bulk_insert(keys: &[Key])`
  - [ ] T3: WAL `append_batch(records: &[WalRecord])`
  - [ ] T4: 批量 INSERT 基准测试

---

#### M44: 表定义持久化

- **问题**：`TableManager` 纯内存，重启丢失
- **任务**：
  - [ ] T1: Schema Page 设计（系统表 `__tables` / `__columns`）
  - [ ] T2: `CREATE TABLE` 时写入系统表
  - [ ] T3: 启动时从系统表加载 schema
  - [ ] T4: `DROP TABLE` / `ALTER TABLE` 同步更新
  - [ ] T5: 重启持久化测试

---

### Phase 5: 高级优化

#### M22: 预取 Prefetch

- **问题**：顺序扫描逐页读，I/O 延迟未重叠
- **任务**：
  - [ ] T1: `Prefetcher` 双缓冲结构（当前页+预取页交替）
  - [ ] T2: DataScan 中 `spawn` 异步预取下一页
  - [ ] T3: 预取深度可配置（默认 2 页）
  - [ ] T4: 大表扫描基准测试

---

#### M23: Varint Key 编码

- **问题**：固定 32B Key，INT PK 浪费 ~28B
- **任务**：
  - [ ] T1: `Key` 内部 `Vec<u8>` 变长编码（Varint prefix + payload）
  - [ ] T2: B+Tree 比较逻辑适配变长 Key
  - [ ] T3: 插入时动态计算节点 split 点（基于实际字节占用）
  - [ ] T4: 索引空间基准测试

---

#### M33: B+Tree 节点级锁

- **问题**：每次操作 lock 整棵树
- **方案**：短期 Semaphore 限流 + 长期 latch coupling（crabbing protocol）
- **任务**：
  - [ ] T1: `Semaphore(MAX_TREE_OPS)` 限流
  - [ ] T2: 节点级 `RwLock` per node
  - [ ] T3: crabbing protocol（读：自顶向下释放父锁；写：持有全部路径锁到叶）
  - [ ] T4: 死锁检测 / 无死锁验证
  - [ ] T5: 并发索引访问基准测试

---

#### M35: 脏页 writev 批量写回

- **问题**：逐页 `write_at()` + 单独 `fsync()`
- **任务**：
  - [ ] T1: `flush_all()` 先收集脏页列表再批量写
  - [ ] T2: 连续 page_id 合并为单次 `write_at`
  - [ ] T3: 非连续脏页 `writev()`
  - [ ] T4: Checkpoint 性能基准测试

---

#### M43: 并行扫描

- **问题**：全表扫描单线程
- **任务**：
  - [ ] T1: 按页范围分区，每个分区 spawn 一个 scan task
  - [ ] T2: `mpsc` channel 汇聚分区结果
  - [ ] T3: 并行度可配置（默认 = CPU 核数）
  - [ ] T4: 聚合/排序场景的并行归约
  - [ ] T5: 大表扫描基准测试

---

#### M45: io_uring 批量提交

- **问题**：`tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
- **任务**：
  - [ ] T1: 引入 `tokio-uring` 或 `io-uring` crate
  - [ ] T2: `BufferPool` 读写改用 io_uring 提交
  - [ ] T3: WAL 刷盘改用 io_uring
  - [ ] T4: 批量提交（IOSQE_IO_LINK）合并 fsync
  - [ ] T5: I/O 延迟基准测试

---

### 长期方向（未规划具体里程碑）

| 方向 | 说明 | 难度 |
|------|------|------|
| M46 瘦内部节点 | B+Tree 内部节点只存 separator keys，不存完整 Key | 高，需重构 B+Tree 分裂/合并逻辑 |
| M47 合并 Tag byte | Slot Tag byte 合并进 VersionHeader，省 1 byte/slot | 低，但影响序列化格式兼容性 |
