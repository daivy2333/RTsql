# 任务追踪

> 最后更新：2026-05-24（M24-M29 规划阶段新增）

## 当前阶段：全维度性能优化（M19-M23）+ 功能完善（M24-M29）

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
| **M24** | 多隔离级别 | Read Committed + Serializable | 高 | 待开始 |
| **M25** | 多 Join 算法 | NLJ + SMJ + 代价选择 | 中 | 待开始 |
| **M26** | 代价模型 + Join 重排 | 统计信息 + 代价估算 + 重排序 | 中 | 待开始 |
| **M27** | 关联子查询缓存 | 参数化缓存 + 物化 | 中 | 待开始 |
| **M28** | 多层关联子查询 | 递归参数注入 | 低 | 待开始 |
| **M29** | PG Extended Query | Parse/Bind/Describe/Execute | 中 | 待开始 |

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

### M24: 多隔离级别

**问题**：只实现了 Repeatable Read（快照隔离），无 Read Committed / Serializable。

**方案**：
- Read Committed：每条语句重新获取 snapshot（而非事务开始时）
- Serializable：SSI（Serializable Snapshot Isolation）+ 写偏序检测

**任务分解**：
- [ ] T1: `IsolationLevel` 枚举定义（ReadCommitted / RepeatableRead / Serializable）
- [ ] T2: `BEGIN TRANSACTION ISOLATION LEVEL ...` SQL 语法支持
- [ ] T3: Read Committed 实现（每语句刷新 snapshot）
- [ ] T4: Serializable SSI 实现（写偏序检测 + predicate locking）
- [ ] T5: 隔离级别测试（ANSI SQL 隔离级别标准测试用例）

**默认假设**：
- 默认隔离级别保持 Repeatable Read（向后兼容）
- Read Committed 实现相对简单，优先完成
- Serializable SSI 复杂度高，可分阶段交付

**预期**：支持标准 SQL 隔离级别，满足不同业务场景需求

---

### M25: 多 Join 算法

**问题**：只有 Hash Join，无 Nested Loop Join / Sort-Merge Join。小表 join 或有序数据场景效率低。

**方案**：
- Nested Loop Join：小表驱动大表，无需 build hash table
- Sort-Merge Join：已排序数据直接归并
- 代价模型自动选择 Join 算法（与 M26 协同）

**任务分解**：
- [ ] T1: `JoinAlgorithm` 枚举 + `PhysicalPlan::Join` 扩展算法字段
- [ ] T2: `NestedLoopJoinExecutor` 实现
- [ ] T3: `SortMergeJoinExecutor` 实现
- [ ] T4: Planner 简单启发式选择（小表 NLJ，有序 SMJ，默认 HJ）
- [ ] T5: Join 算法基准测试对比

**预期**：小表 join 场景显著提速，有序数据避免额外排序

---

### M26: 代价模型 + Join 重排序

**问题**：Planner 固定 join 顺序，无 cardinality/selectivity 估算，无代价模型。

**方案**：
- 统计信息收集（行数、NDV、直方图）
- 代价估算模型（CPU + I/O 代价）
- Join 重排序（动态规划 / 贪心）

**任务分解**：
- [ ] T1: `TableStatistics` 结构（行数、NDV、min/max、null_count）
- [ ] T2: `ANALYZE TABLE` 命令 + 统计信息持久化
- [ ] T3: `CostEstimator` 代价估算（scan cost / join cost / filter selectivity）
- [ ] T4: Join 重排序算法（DP for <10 tables, greedy for ≥10）
- [ ] T5: 代价模型驱动的执行计划选择测试

**预期**：多表 join 自动选最优顺序，避免最差执行计划

---

### M27: 关联子查询缓存

**问题**：关联子查询每行外层都重新执行，无物化缓存。N 行外层 = N 次子查询执行。

**方案**：
- 参数化缓存：相同关联参数值命中缓存
- 子查询物化：将子查询结果物化为临时表

**任务分解**：
- [ ] T1: `SubqueryCache` 结构（关联参数值 → 结果集映射）
- [ ] T2: `SubqueryEvalExecutor` 集成缓存逻辑
- [ ] T3: 缓存淘汰策略（LRU / 事务结束清空）
- [ ] T4: 关联子查询性能基准测试

**预期**：重复关联参数场景避免重复执行，性能提升与参数重复率正相关

---

### M28: 多层关联子查询

**问题**：代码显式拒绝多层嵌套关联子查询，复杂查询直接报错。

**方案**：
- 递归注入外层参数到多层子查询
- 逐层解析关联列引用

**任务分解**：
- [ ] T1: `extract_correlated_params` 改为递归遍历子查询嵌套
- [ ] T2: `inject_correlated_values` 支持多层参数注入
- [ ] T3: 移除多层嵌套拒绝逻辑
- [ ] T4: 多层关联子查询测试用例

**预期**：支持 `WHERE EXISTS (SELECT ... WHERE col = (SELECT ...))` 等嵌套结构

---

### M29: PG Extended Query Protocol

**问题**：只支持 Simple Query Protocol，无 Parse/Bind/Describe/Execute。

**方案**：
- 实现 Extended Query Protocol 消息流
- 支持 prepared statement 缓存
- 二进制格式 DataRow 传输

**任务分解**：
- [ ] T1: Parse / Bind / Describe / Execute 消息解析与序列化
- [ ] T2: Prepared Statement 缓存与生命周期管理
- [ ] T3: 二进制格式 DataRow 编码（Int/Text/Float/Bool）
- [ ] T4: Close / Sync / Flush 消息支持
- [ ] T5: psql 预编译语句集成测试

**预期**：支持预编译语句，减少重复解析开销；二进制传输提升效率

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
- **M24**: 📋 多隔离级别（Read Committed + Serializable）
- **M25**: 📋 多 Join 算法（NLJ + SMJ + 代价选择）
- **M26**: 📋 代价模型 + Join 重排序
- **M27**: 📋 关联子查询缓存
- **M28**: 📋 多层关联子查询
- **M29**: 📋 PG Extended Query Protocol
