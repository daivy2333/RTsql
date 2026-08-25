## Purpose

记录有证据但尚未承诺实施的改进机会。条目使用 `Ixx` 编号，按 Phase 或主题分类。批准实施后创建 OpenSpec change 并标记 `promoted`。

## Requirements

### Requirement: 改进项可评估

改进项 SHALL 包含分类、问题、证据、影响、建议和状态。

#### Scenario: 发现未排期问题

- **WHEN** 已有证据表明存在改进机会但尚未批准实施
- **THEN** 使用递增 I 编号记录

#### Scenario: 批准实施

- **WHEN** 用户批准实施改进项
- **THEN** 创建 OpenSpec change 并把原条目标记 promoted

---

## Phase 1 基础设施（已完成）

**I001-I003 全部完成**（M41/M30/M38，详见 R08-013 + K14-16 + D09-10）

## Phase 2 存储引擎核心（已完成）

**I004-I008 全部完成**（M20/M19/M21/M36/M19，详见 K17-19, K22, D11-12, R10-013）

## Phase 3 并发控制

## I009: M40 RowLockTable DashMap

- **分类**: 性能 / 并发
- **问题**: `Arc<Mutex<HashMap>>` 行锁获取/释放串行化
- **方案**: `DashMap<RowId, Arc<Mutex<()>>>`
- **预期**: 行锁争抢 -5-10x
- **依赖**: M31（已完成）
- **状态**: planned（P3）
- **Legacy**: O009

## I010: M34 WAL fsync 合并

- **分类**: 性能 / WAL
- **问题**: 每事务提交单独 fsync，系统调用开销巨大
- **方案**: `tokio::time::interval` 定时器 + 累积多条记录一次 fsync
- **预期**: TPS 3-10x
- **依赖**: 无（M30 完成后可立即开始）
- **状态**: planned（P3）
- **Legacy**: O010

## I011: M32 WAL 写入背压

- **分类**: 性能 / WAL
- **问题**: WAL 无背压，高并发缓冲区膨胀
- **方案**: `Semaphore(WAL_MAX_PENDING)` 限制等待刷盘事务数
- **依赖**: M34（I010）
- **状态**: planned（P3）
- **Legacy**: O011

## I012: M42 消息传递重构

- **分类**: 架构 / 可维护性
- **问题**: 多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**: WAL→mpsc，提交→oneshot，Checkpoint→Notify，BufferPool→watch
- **依赖**: M32（I011）
- **状态**: planned（P3）
- **Legacy**: O012

## I013: M48 pread/pwrite 替代 seek+read

- **分类**: 性能 / 系统调用
- **问题**: 文件读写用 `seek()+read()/write()` 两次 syscall
- **方案**: `FileExt::read_at()` / `write_at()` 单次 syscall
- **预期**: syscall -50%
- **状态**: planned（P3，独立）
- **Legacy**: O013

## Phase 4 上层功能

## I014: M24 多隔离级别

- **分类**: 功能 / SQL 标准
- **问题**: 只有 Repeatable Read
- **方案**: Read Committed + Serializable（SSI）
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O014

## I015: M25 多 Join 算法

- **分类**: 功能 / 查询优化
- **问题**: 只有 Hash Join
- **方案**: NLJ + SMJ + 启发式选择
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O015

## I016: M26 代价模型 + Join 重排

- **分类**: 功能 / 优化器
- **问题**: 固定 join 顺序，无 cardinality/selectivity
- **方案**: `TableStatistics` + `CostEstimator` + DP/贪心重排
- **依赖**: M25（I015）
- **状态**: planned（P4）
- **Legacy**: O016

## I017: M27 关联子查询缓存

- **分类**: 性能 / 子查询
- **问题**: 每行外层重新执行子查询
- **方案**: `SubqueryCache` 参数值→结果集 LRU
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O017

## I018: M28 多层关联子查询

- **分类**: 功能 / 子查询
- **问题**: 显式拒绝多层嵌套
- **方案**: 递归遍历 + 多层注入
- **依赖**: M27（I017）
- **状态**: planned（P4）
- **Legacy**: O018

## I019: M29 PG Extended Query Protocol

- **分类**: 功能 / 协议
- **问题**: 只有 Simple Query
- **方案**: Parse/Bind/Describe/Execute + Prepared Statement
- **依赖**: M38（已完成）
- **状态**: planned（P4）
- **Legacy**: O019

## I020: M37 clone 消除 Arc/Cow

- **分类**: 性能 / 分配
- **问题**: `Value::clone()` 在聚合/排序/JOIN 中反复调用
- **方案**: `Value::Text` 内部 `Arc<str>` + `Cow<'_, str>` 延迟分配
- **依赖**: M20（已完成）
- **状态**: planned（P4）
- **Legacy**: O020

## I021: M39 INSERT 批量执行

- **分类**: 性能 / 写入
- **问题**: 多值 INSERT 逐行执行
- **方案**: `bulk_insert(keys)` + `append_batch(records)`
- **依赖**: M20（已完成）
- **状态**: planned（P4）
- **Legacy**: O021

## I022: M44 表定义持久化

- **分类**: 功能 / 持久化
- **问题**: `TableManager` 纯内存，重启丢失
- **方案**: Schema Page（系统表 `__tables` / `__columns`）
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O022, K05

## Phase 5 高级优化

## I023: M22 预取 Prefetch

- **分类**: 性能 / I/O
- **问题**: 顺序扫描逐页读，I/O 延迟未重叠
- **方案**: `Prefetcher` 双缓冲 + 异步预取下一页
- **预期**: 大表 ~15-25%
- **依赖**: M19（已完成）+ M31（已完成）
- **状态**: planned（P5）
- **Legacy**: O023, D12 下游

## I024: M23 Varint Key 编码

- **分类**: 性能 / 存储
- **问题**: 固定 32B Key，INT PK 浪费 ~28B
- **方案**: `Key` 内部 `Vec<u8>` 变长编码
- **预期**: 索引空间 ~70% 缩减
- **依赖**: 无
- **状态**: planned（P5）
- **Legacy**: O024, D02 successor

## I025: M33 B+Tree 节点级锁

- **分类**: 并发 / 锁
- **问题**: 每次操作 lock 整棵树
- **方案**: Semaphore 限流 + latch coupling（crabbing protocol）
- **依赖**: M23（I024）
- **状态**: planned（P5）
- **Legacy**: O025

## I026: M35 脏页 writev 批量写回

- **分类**: 性能 / Checkpoint
- **问题**: 逐页 `write_at()` + 单独 `fsync()`
- **方案**: 连续页合并写 + `writev()` 向量化
- **预期**: Checkpoint 5-10x
- **依赖**: M31（已完成）+ M48（I013）
- **状态**: planned（P5）
- **Legacy**: O026, D12 下游

## I027: M43 并行扫描

- **分类**: 性能 / 并行
- **问题**: 全表扫描单线程
- **方案**: 按页范围分区 + `mpsc` 汇聚
- **依赖**: M19（已完成）+ M22（I023）
- **状态**: planned（P5）
- **Legacy**: O027

## I028: M45 io_uring 批量提交

- **分类**: 性能 / I/O
- **问题**: `tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
- **方案**: `tokio-uring` 批量提交（IOSQE_IO_LINK）
- **预期**: I/O 延迟 -30-50%
- **依赖**: Linux 5.1+
- **状态**: planned（P5）
- **Legacy**: O028, K36

## 长期方向（未规划具体里程碑）

## I029: M46 瘦内部节点

- **分类**: 性能 / 存储
- **问题**: B+Tree 内部节点只存 separator keys，不存完整 Key
- **难度**: 高，需重构 B+Tree 分裂/合并逻辑
- **依赖**: M23（I024）
- **状态**: long-term
- **Legacy**: O029

## I030: M47 合并 Tag byte

- **分类**: 性能 / 序列化
- **问题**: Slot Tag byte 合并进 VersionHeader，省 1 byte/slot
- **难度**: 低，但影响序列化格式兼容性
- **状态**: long-term
- **Legacy**: O030
