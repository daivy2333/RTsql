# 任务追踪

> 最后更新：2026-05-24（M19-M23 全维度性能优化规划）

## 当前阶段：全维度性能优化（M19-M23）

### 背景

M1-M18 项目核心开发完成。当前性能短板：

| 操作 | RTsql | SQLite | 差距 | 根因 |
|------|-------|--------|------|------|
| **Full Scan 1K rows** | 327µs | 80µs | **4x slower** | Index-to-Data 双读 + 每行堆分配 + MVCC 逐行检查 |
| **文件大小 10K rows** | 1.4MB | 217KB | **6.5x larger** | 固定 Key 32B + 两层索引 + Tag byte |

### 优化路线图

| 里程碑 | 优化项 | 预期收益 | 风险 | 状态 |
|--------|--------|---------|------|------|
| **M19** | DataScan 路径 | 全表扫描 ~2x 提速 | 中 | 待开始 |
| **M20** | 零拷贝读取 | I/O ~20-30% 提速 | 低 | 待开始 |
| **M21** | 页面级 MVCC | ~10-15% 提速 | 中 | 待开始 |
| **M22** | 预取 Prefetch | 大表 ~15-25% 提速 | 低 | 待开始 |
| **M23** | Varint Key 编码 | 索引空间 ~70% 缩减 | 中 | 待开始 |

---

### M19: DataScan 路径（全表扫描直读数据页）

**问题**：当前全表扫描走 B+tree → RowId → 数据页，每行双读（索引页 + 数据页）。

**方案**：新增 `DataScanExecutor`，直接按 page_id 顺序遍历数据页，跳过 B+tree。

**任务分解**：
- [ ] T1: `PhysicalPlan::DataScan` 节点 + Planner 自动选择（无 WHERE 时用 DataScan）
- [ ] T2: `DataScanExecutor` 实现（按 page 范围顺序读数据页 + 遍历 slots）
- [ ] T3: `SlottedPage::iter_slots()` 接口（遍历所有有效 slot）
- [ ] T4: Pipeline 层集成（DataScan → DataScanExecutor 创建）
- [ ] T5: 基准测试 + 对比验证

**默认假设**：
- DataScan 只读已提交数据（MVCC 可见性仍逐行检查）
- 并发写入时 DataScan 可运行（读快照可见行）
- 空表返回空结果
- 有 WHERE 条件仍走 B+tree Scan

**预期**：Full Scan 从 327µs → ~160µs（消除 B+tree 页读取开销）

---

### M20: 零拷贝读取

**问题**：`SlottedPage::get` 每行 `Vec::from` 堆分配，全表扫描 N 行 = N 次堆分配。

**方案**：返回 `&[u8]` 切片引用直接指向页缓冲区，消除每行堆分配。

**任务分解**：
- [ ] T1: `SlottedPage::get_slice()` 返回 `&[u8]`（零拷贝接口）
- [ ] T2: `DataPage::get_row_slice()` 零拷贝接口
- [ ] T3: Scan/DataScan executor 适配零拷贝路径
- [ ] T4: 基准测试对比（堆分配 vs 零拷贝）

**预期**：I/O 密集场景 ~20-30% 提速，减少内存分配压力

---

### M21: 页面级 MVCC

**问题**：每行 16 字节 VersionHeader，全表扫描逐行检查可见性，纯开销。

**方案**：页面级可见性标记（page header 记录 min_commit_ts/max_txn_id），整页可跳过时跳过。

**任务分解**：
- [ ] T1: Page header 扩展（min_commit_ts/max_txn_id 字段）
- [ ] T2: 写入时更新页面级 MVCC 元数据
- [ ] T3: DataScan 页面级可见性快速判断（整页跳过 or 逐行检查）
- [ ] T4: 基准测试 + 正确性验证

**默认假设**：
- 页面级判断保守：min_commit_ts > snapshot_ts → 整页可见；max_txn_id 活跃 → 退回逐行检查
- 误判时退回逐行检查，保证正确性

**预期**：读多写少场景 ~10-15% 提速

---

### M22: 预取 Prefetch

**问题**：顺序扫描无 read-ahead，每次只读一页，I/O 延迟未重叠。

**方案**：DataScan 顺序扫描时异步预读下一页（tokio::spawn 预取），处理当前页时下一页已在缓冲池。

**任务分解**：
- [ ] T1: `BufferPool::prefetch_page()` 异步预读接口
- [ ] T2: DataScanExecutor 双缓冲预取逻辑
- [ ] T3: 基准测试（小表/大表对比）

**预期**：大表（>1K rows）~15-25% 提速，小表效果不明显

---

### M23: Varint Key 编码

**问题**：B-Tree Key 固定 32 bytes，INT PRIMARY KEY 浪费 ~28 bytes，索引空间 ~10x 膨胀。

**方案**：Key 改用 varint 编码（1-9 bytes），大幅缩减索引页占用。

**任务分解**：
- [ ] T1: Varint 编解码实现（u64 ↔ 1-9 bytes）
- [ ] T2: LeafNode/InternalNode Key 存储改用 varint
- [ ] T3: B-Tree 查找/插入/删除适配变长 Key
- [ ] T4: 基准测试 + 文件大小对比

**预期**：索引空间 ~70% 缩减，文件大小从 6.5x → ~2-3x SQLite

---

## 已完成（M1-M18）

> 详细子任务已归档至 archive.md §tasks。

- **M18**: WAL + Group Commit + 崩溃恢复 + B-Tree Merge ✅
- **M17**: 非唯一索引 + B-Tree Split ✅
- **M16**: 子查询支持 ✅
- **M15**: SQLite 基础性能对比 ✅
- **M1-M14**: 核心功能（SQL解析/执行器/存储/B-Tree/MVCC/事务/索引） ✅

---

## 里程碑路线图

- M1-M15: ✅ 核心功能 + 性能验证
- M16: ✅ 子查询支持
- M17: ✅ 非唯一索引 + B-Tree Split
- M17.5: ✅ 代码清理 + 全面对比
- M18: ✅ WAL + Group Commit + B-Tree Merge
- **M19**: 📋 DataScan 路径（全表扫描 ~2x 提速）
- **M20**: 📋 零拷贝读取（I/O ~20-30% 提速）
- **M21**: 📋 页面级 MVCC（~10-15% 提速）
- **M22**: 📋 预取 Prefetch（大表 ~15-25% 提速）
- **M23**: 📋 Varint Key 编码（索引空间 ~70% 缩减）
