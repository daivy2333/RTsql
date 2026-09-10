# 归档载体：improvements 条目（ARC-202609092322 批次）

> 本文件是 openspec-archivist 清理批次的 Archive 载体，保存被归档 I 条目的完整原文。
> 条目原位置：`openspec/specs/improvements/spec.md`；归档日期：2026-09-09。

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

## I013: M48 pread/pwrite 替代 seek+read

- **分类**: 性能 / 系统调用
- **问题**: 文件读写用 `seek()+read()/write()` 两次 syscall
- **方案**: `FileExt::read_at()` / `write_at()` 单次 syscall
- **预期**: syscall -50%
- **状态**: planned（P3，独立）
- **Legacy**: O013

## I019: M29 PG Extended Query Protocol

- **分类**: 功能 / 协议
- **问题**: 只有 Simple Query
- **方案**: Parse/Bind/Describe/Execute + Prepared Statement
- **依赖**: M38（已完成）
- **状态**: planned（P4）
- **Legacy**: O019

## I022: M44 表定义持久化

- **分类**: 功能 / 持久化
- **问题**: `TableManager` 纯内存，重启丢失
- **方案**: Schema Page（系统表 `__tables` / `__columns`）
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O022, K05

## I023: M22 预取 Prefetch

- **分类**: 性能 / I/O
- **问题**: 顺序扫描逐页读，I/O 延迟未重叠
- **方案**: `Prefetcher` 双缓冲 + 异步预取下一页
- **预期**: 大表 ~15-25%
- **依赖**: M19（已完成）+ M31（已完成）
- **状态**: planned（P5）
- **Legacy**: O023, D12 下游

