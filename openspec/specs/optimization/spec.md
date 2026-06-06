# Optimization — 优化记录

> 版本：v1.5 | 最后更新：2026-06-06（O008 M31 全部完成）
> 由 openspec-init 从 `.claude/docs/optimization.md` 迁移。
> 条目格式: <!-- O{编号} --> - {问题描述}
> 每条含当前影响、建议方案。

---

## Purpose

记录 RTsql 项目的所有优化方向、技术债和性能改进计划，按 Phase 和优先级组织，便于规划迭代和追踪进度。

---

## Requirements

### Requirement: 优化项可追踪

每个优化项 SHALL 包含问题描述、方案、预期收益和状态。

#### Scenario: 新增优化项
- **WHEN** 发现新的性能瓶颈或技术债
- **THEN** 在对应 Phase 区域新增条目，格式：`<!-- O{编号} --> - **M{N}: {标题}**`

#### Scenario: 更新优化状态
- **WHEN** 优化项开始或完成
- **THEN** 更新对应条目的状态标记

#### Scenario: 查询优化计划
- **WHEN** 需要了解下一步优化方向
- **THEN** 按 Phase 顺序查看待优化区域

### Requirement: 依赖关系清晰

优化项之间的依赖关系 SHALL 在 tasks.md 中维护。

#### Scenario: 规划优化顺序
- **WHEN** 需要确定多个优化项的执行顺序
- **THEN** 参考 tasks.md 中的依赖关系图

---

## 待优化（Phase 1: 基础设施）

✅ **Phase 1 全部完成**（M41/30/38），详见"已完成"区域。

---

## 待优化（Phase 2: 存储引擎核心）

✅ **Phase 2 全部完成**（M20/36/19/21），详见"已完成"区域。

---

## 待优化（Phase 3: 并发控制）

<!-- O008 --> - **M31: BufferPool DashMap + Semaphore** ✅ 已完成（2026-06-06）
  - 详情：pages 字段迁移 DashMap + miss Sem(16) + per-page loading_locks，cache hit 100ns→0
  - 详见：ADR-012 + L031 + `openspec/changes/archive/2026-06-06-m31-bufferpool-dashmap-semaphore/`

<!-- O009 --> - **M40: RowLockTable DashMap**
  - 问题：`Arc<Mutex<HashMap>>` 行锁获取/释放串行化
  - 方案：`DashMap<RowId, Arc<Mutex<()>>>`
  - 预期：行锁争抢 -5-10x
  - 状态：📋 P3

<!-- O010 --> - **M34: WAL fsync 合并**
  - 问题：每事务提交单独 fsync，系统调用开销巨大
  - 方案：`tokio::time::interval` 定时器 + 累积多条记录一次 fsync
  - 预期：TPS 3-10x
  - 状态：📋 P3

<!-- O011 --> - **M32: WAL 写入背压**
  - 问题：WAL 无背压，高并发缓冲区膨胀
  - 方案：`Semaphore(WAL_MAX_PENDING)` 限制等待刷盘事务数
  - 状态：📋 P3

<!-- O012 --> - **M42: 消息传递重构**
  - 问题：多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
  - 方案：WAL→mpsc，提交→oneshot，Checkpoint→Notify，BufferPool→watch
  - 状态：📋 P3

<!-- O013 --> - **M48: pread/pwrite 替代 seek+read**
  - 问题：文件读写用 `seek()+read()/write()` 两次 syscall
  - 方案：`FileExt::read_at()` / `write_at()` 单次 syscall
  - 预期：syscall -50%
  - 状态：📋 P3

---

## 待优化（Phase 4: 上层功能）

<!-- O014 --> - **M24: 多隔离级别**
  - 问题：只有 Repeatable Read
  - 方案：Read Committed + Serializable（SSI）
  - 状态：📋 P4

<!-- O015 --> - **M25: 多 Join 算法**
  - 问题：只有 Hash Join
  - 方案：NLJ + SMJ + 启发式选择
  - 状态：📋 P4

<!-- O016 --> - **M26: 代价模型 + Join 重排**
  - 问题：固定 join 顺序，无 cardinality/selectivity
  - 方案：`TableStatistics` + `CostEstimator` + DP/贪心重排
  - 状态：📋 P4

<!-- O017 --> - **M27: 关联子查询缓存**
  - 问题：每行外层重新执行子查询
  - 方案：`SubqueryCache` 参数值→结果集 LRU
  - 状态：📋 P4

<!-- O018 --> - **M28: 多层关联子查询**
  - 问题：显式拒绝多层嵌套
  - 方案：递归遍历 + 多层注入
  - 状态：📋 P4

<!-- O019 --> - **M29: PG Extended Query Protocol**
  - 问题：只有 Simple Query
  - 方案：Parse/Bind/Describe/Execute + Prepared Statement
  - 状态：📋 P4

<!-- O020 --> - **M37: clone 消除 Arc/Cow**
  - 问题：`Value::clone()` 在聚合/排序/JOIN 中反复调用
  - 方案：`Value::Text` 内部 `Arc<str>` + `Cow<'_, str>` 延迟分配
  - 状态：📋 P4

<!-- O021 --> - **M39: INSERT 批量执行**
  - 问题：多值 INSERT 逐行执行
  - 方案：`bulk_insert(keys)` + `append_batch(records)`
  - 状态：📋 P4

<!-- O022 --> - **M44: 表定义持久化**
  - 问题：`TableManager` 纯内存，重启丢失
  - 方案：Schema Page（系统表 `__tables` / `__columns`）
  - 状态：📋 P4

---

## 待优化（Phase 5: 高级优化）

<!-- O023 --> - **M22: 预取 Prefetch**
  - 问题：顺序扫描逐页读，I/O 延迟未重叠
  - 方案：`Prefetcher` 双缓冲 + 异步预取下一页
  - 预期：大表 ~15-25%
  - 状态：📋 P5

<!-- O024 --> - **M23: Varint Key 编码**
  - 问题：固定 32B Key，INT PK 浪费 ~28B
  - 方案：`Key` 内部 `Vec<u8>` 变长编码
  - 预期：索引空间 ~70% 缩减
  - 状态：📋 P5

<!-- O025 --> - **M33: B+Tree 节点级锁**
  - 问题：每次操作 lock 整棵树
  - 方案：Semaphore 限流 + latch coupling（crabbing protocol）
  - 状态：📋 P5

<!-- O026 --> - **M35: 脏页 writev 批量写回**
  - 问题：逐页 `write_at()` + 单独 `fsync()`
  - 方案：连续页合并写 + `writev()` 向量化
  - 预期：Checkpoint 5-10x
  - 状态：📋 P5

<!-- O027 --> - **M43: 并行扫描**
  - 问题：全表扫描单线程
  - 方案：按页范围分区 + `mpsc` 汇聚
  - 状态：📋 P5

<!-- O028 --> - **M45: io_uring 批量提交**
  - 问题：`tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
  - 方案：`tokio-uring` 批量提交（IOSQE_IO_LINK）
  - 预期：I/O 延迟 -30-50%
  - 状态：📋 P5

---

## 长期方向（未规划具体里程碑）

<!-- O029 --> - **M46: 瘦内部节点**
  - B+Tree 内部节点只存 separator keys，不存完整 Key
  - 难度：高，需重构 B+Tree 分裂/合并逻辑

<!-- O030 --> - **M47: 合并 Tag byte**
  - Slot Tag byte 合并进 VersionHeader，省 1 byte/slot
  - 难度：低，但影响序列化格式兼容性

---

## 已完成

<!-- 完成后移到此处，标注完成日期 -->
> M1-M18 核心开发已完成（2026-05-24 归档）
> 481 tests pass (2026-06-06), INSERT 332x faster, PK lookup 5.6x faster than SQLite

<!-- O003 已完成（2026-06-03）-->
**M38: 网络 BufWriter + TCP_NODELAY** — PgProtocol `write_buf` 累积响应，N→2 syscalls + `set_nodelay`

<!-- O002 已完成（2026-06-03）-->
**M30: 连接并发上限** — `Server::new(addr, db, max_connections)` + `Arc<Semaphore>` + 3 并发压测通过

<!-- O007 已完成（2026-06-03）-->
**M36: 零拷贝 ValueRef** — `ValueRef<'a>` 枚举 + `deserialize_value_refs` 零拷贝，堆分配 30万→0

<!-- O008 已完成（2026-06-06）-->
**M31: BufferPool DashMap + Miss Semaphore** — pages 字段迁 DashMap（lock-free hit）+ miss Sem(16) bound IO + per-page loading_locks（double-check 正确性），481 tests pass

<!-- O006 已完成（2026-06-04）-->
**M21: 页面级 MVCC** — `PageVisibilityInfo` + `DashMap` 快速路径 + DELETE mark_deleted + 惰性 set_all_visible + benchmark

<!-- O005 已完成（2026-06-04）-->
**M19: DataScan 路径** — `DataScanExecutor` 数据页链表遍历，1K 1.81x / 10K 2.44x 提速

<!-- O004 已完成（2026-06-03）-->
**M20: 零拷贝 SlottedPageRef** — 纯闭包 API `with_page_data`，read 路径 -2.46%~-8.33%

<!-- O001 已完成（2026-06-03）-->
**M41: 事务 ID AtomicU64 无锁分配**（commit `634764d` + `ee9ceee`）

**实测性能数据**（criterion 4 场景，ns/op）：

| 场景 | Mutex | Atomic | 加速比 |
|------|-------|--------|--------|
| 单线程 1M | 10.7 | 5.1 | 2.1x |
| 10 线程争用 | 84.7 | 18.6 | 4.6x |
| 100 线程高争用 | 100.8 | 22.5 | 4.5x |
| 吞吐@1M (单线程) | 90.8 Melem/s | 138.1 Melem/s | 1.52x |

**关键结论**：
- 路线图"100ns→10ns"达成（实际 5.1 ns/op）
- 10/100 线程争用下 Atomic 5x 加速假设全部满足
- 单线程差异假设（< 20%）被推翻：实际 Atomic 在单线程下也 2x 快（锁自身开销不可忽略）

**微基准**：`benches/tx_id_bench.rs`（criterion，黑盒函数 + 4 场景）
**ADR**：`architecture/spec.md` ADR-009
**数据记录**：`learned/spec.md` L017
