# tasks — 任务与里程碑路线

> 最后更新：2026-08-25（OpenSpec v1.6.0 初始化 + 路线图迁移）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 路线图结构

5 个 Phase（MS01-MS05），Phase 内可并行；Milestone 编号沿用历史 M19-M48 命名以保持代码 commit 一致性。

## 已完成历史

### MS00: M1-M18 核心开发（2026-05-24 归档）

- 完整 SQL + WAL + Group Commit + 崩溃恢复 + B-Tree Split & Merge + 关联子查询
- 464 tests pass（2026-05-24）
- INSERT 332x faster, PK lookup 5.6x faster than SQLite
- 详见 `openspec/changes/archive/` 历史 change 目录

## Milestone Roadmap

### MS01: Phase 1 基础设施 — ✅ 全部完成

| 里程碑 | 优化项 | 预期收益 | 状态 | 关联 |
|---|---|---|---|---|
| M41 | 事务 ID AtomicU64 | 分配延迟 100ns→10ns | ✅ 完成 (5.1 ns/op) | D09, K16 |
| M30 | 连接并发 Semaphore | 防连接风暴 | ✅ 完成 | K14 |
| M38 | 网络 BufWriter + TCP_NODELAY | write 调用 -99% | ✅ 完成 | D10, K15 |

**稳定基线**: 475 tests pass (2026-06-04)
**验证边界**: 单线程 5.1 ns/op 分配；连接限流 3 压测；网络 N→2 syscalls
**诊断边界**: AtomicU64 性能 → benches/tx_id_bench.rs

### MS02: Phase 2 存储引擎核心 — ✅ 全部完成

| 里程碑 | 优化项 | 预期收益 | 状态 | 关联 |
|---|---|---|---|---|
| M20 | 零拷贝 SlottedPageRef | 读路径 -2.46%~-8.33% | ✅ 完成 | K08, K09, K17 |
| M19 | DataScan 路径 | 全表扫描 1.81x-2.44x | ✅ 完成 | K19, K22 |
| M21 | 页面级 MVCC | 可见性快速路径 | ✅ 完成 | D11, K12, K13 |
| M36 | 零拷贝 ValueRef | 堆分配 30万→0 | ⚠️ 完成（目标未直接验证） | K18 |

**稳定基线**: 481 tests pass (2026-06-06)
**验证边界**: DataScan 1K/10K 实测；visibility bench 3 场景
**诊断边界**: 零拷贝性能 → benches/single, benches/data_scan_bench.rs
**依赖**: M20 → M19/M36（写路径 0 回归），M19/M21 → 未来 M22 预取

### MS03: Phase 3 并发控制 — ⏳ 部分完成

| 里程碑 | 优化项 | 预期收益 | 状态 | 关联 |
|---|---|---|---|---|
| M31 | BufferPool DashMap + Miss Sem + Per-Page Loading Locks | cache hit 100ns→0 | ✅ 完成 (2026-06-06) | D12, K10, K11 |
| M40 | RowLockTable DashMap | 行锁争抢 -5-10x | 📋 planned | I009 |
| M34 | WAL fsync 合并 | TPS 3-10x | 📋 planned | I010 |
| M32 | WAL 写入背压 | 缓冲区限流 | 📋 planned | I011 |
| M42 | 消息传递重构 | WAL 锁消除 | 📋 planned | I012 |
| M48 | pread/pwrite 替代 seek+read | syscall -50% | 📋 planned | I013 |

**稳定基线** (M31 后): 481 tests pass, 6 并发测试新增
**验证边界**: cache hit lock-free 16 tasks ≤ 1µs；miss Sem 1000 tasks 全成功；double-check 8 并发 1 read
**诊断边界**: 锁顺序 miss_sem → loading_lock → pages → clock_hand → frame（K11）

### MS04: Phase 4 上层功能 — 📋 全部 planned

| 里程碑 | 优化项 | 风险 | 关联 |
|---|---|---|---|
| M24 | 多隔离级别 | 高 | I014 |
| M25 | 多 Join 算法 | 中 | I015 |
| M26 | 代价模型 + Join 重排 | 中 | I016 |
| M27 | 关联子查询缓存 | 中 | I017 |
| M28 | 多层关联子查询 | 低 | I018 |
| M29 | PG Extended Query | 中 | I019 |
| M37 | clone 消除 Arc/Cow | 中 | I020 |
| M39 | INSERT 批量执行 | 中 | I021 |
| M44 | 表定义持久化 | 中 | I022 |

**依赖**: M37 ← M20; M25 → M26; M27 → M28; M44 独立
**并行性**: 可与 MS03 部分并行

### MS05: Phase 5 高级优化 — 📋 全部 planned

| 里程碑 | 优化项 | 预期收益 | 风险 | 关联 |
|---|---|---|---|---|
| M22 | 预取 Prefetch | 大表 ~15-25% | 低 | I023 |
| M23 | Varint Key 编码 | 索引空间 ~70% 缩减 | 中 | I024 |
| M33 | B+Tree 节点级锁 | 并发索引访问 | 高 | I025 |
| M35 | 脏页 writev | Checkpoint 5-10x | 低 | I026 |
| M43 | 并行扫描 | 多核扫描提速 | 中 | I027 |
| M45 | io_uring | I/O 延迟 -30-50% | 高 | I028 |
| M46 | 瘦内部节点 | 内部节点空间优化 | 高 | I029 |
| M47 | 合并 Tag byte | 序列化 1 byte/slot | 低 | I030 |

**依赖**:
- M22 ← M19 (完成) + M31 (完成)
- M23 → M33
- M35 ← M31 (完成) + M48 (I013)
- M43 ← M19 (完成) + M22
- M46 ← M23

## 长期方向

- **io_uring 集成 (K36)**: Linux 5.1+ tokio-uring 批量提交
- **jemalloc/mimalloc 优化 (K37)**: 减少 String/Vec 分配开销

## 依赖关系图

```
M41 ──→ M40        M20 ──→ M19 ──→ M21 ──→ M22(MS05)
  └──→ M48         M20 ──→ M36 ──→ M37(MS04)
M30 (独立)         M34 ──→ M32 ──→ M42
M38 ──→ M29        M25 ──→ M26
M31 ──→ M35(MS05)  M27 ──→ M28
  └──→ M22(MS05)   M20 ──→ M39
M23 ──→ M33(MS05)
```

## 进行中

- （无 — M31 完成后等待下一个 MS03 milestone 启动）

## 已承诺待办

- M40 (RowLockTable DashMap) — 下一步 MS03 推荐
- M34 (WAL fsync 合并) — MS03 备选

## 阻塞

- （无）

## 最近完成

| 里程碑 | 完成日期 | commit |
|---|---|---|
| M31 | 2026-06-06 | f64c874, b55a9a1, 5fc5494, fcaeb7c, faa87a4, ad90379 |
| M21 延后项 | 2026-06-04 | 78a3b01 |
| M19 | 2026-06-04 | 6f1d00f, b9b9a08, 602f8fe |
| M36 | 2026-06-03 | 73076ac, 95bb3f9, b75d307, bf4cbc1 |
| M20 | 2026-06-03 | （多个） |
| M41 | 2026-06-03 | 634764d, ee9ceee |
| M38 | 2026-06-03 | （多个） |
| M30 | 2026-06-03 | （多个） |

## 与 OpenSpec Changes 同步

- 每个 MSxx 内的 M 编号 milestone 实施时通过 `openspec/changes/<date>-<m-tag>/` 创建 change
- 完成的 change 通过 `openspec archive` 归档
- 归档的 change carrier 保持不可变
- 新发现的问题写 `openspec/specs/improvements/spec.md` (Ixx)
- 完整迁移的旧版 entry 记录在 `.claude/legacy/2026-08-25-openspec-init-migration/COVERAGE.md`
