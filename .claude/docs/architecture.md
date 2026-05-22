# 架构决策记录

> 最后更新：2026-05-22（M15 聚合函数与 GROUP BY）

## 系统架构

```
┌──────────────┐
│   SQL Text   │
└──────┬───────┘
       ▼
┌──────────────┐     ┌──────────┐
│   Parser     │────▶│ PlanCache│ (LRU, SELECT only)
│ (sqlparser)  │     └──────────┘
└──────┬───────┘
       ▼
┌──────────────┐
│  PlanBuilder │───▶ PhysicalPlan
│ (register +  │     (15 种节点)
│  build_plan) │
└──────┬───────┘
       ▼
┌──────────────┐
│   Pipeline   │───▶ create_executor_from_plan (递归)
│              │
└──────┬───────┘
       ▼
┌──────────────────────────────────────┐
│         Volcano Executor Tree        │
│                                      │
│  Scan → Filter → Join → Aggregate   │
│       → Having → Sort → Limit       │
│  IndexScan → Insert/Update/Delete   │
└──────────────────────────────────────┘
       │
       ▼
┌──────────────┐
│  Storage     │
│  BufferPool  │───▶ PageGuard (零拷贝/修改)
│  BTree       │───▶ AtomicPageId (async) + from_root (sync)
│  SlottedPage │───▶ 读: SlottedPageRef / 写: SlottedPage + compacting
└──────────────┘
```

## 核心架构决策

| # | 日期 | 决策 | 原因 | 替代方案 |
|---|------|------|------|----------|
| 1 | 2026-05 | Volcano 迭代器模型 | 算子可自由组合，扩展方便 | 物化模型（内存占用高） |
| 2 | 2026-05 | Tokio async 协程 | 无栈协程轻量，适合 I/O 密集 | 同步 I/O（吞吐低） |
| 3 | 2026-05 | 两阶段锁 BufferPool | I/O 期间不持锁，避免阻塞 | 单阶段锁（I/O 阻塞） |
| 4 | 2026-05 | AtomicPageId 无锁读 | async 路径避免 std::sync::RwLock | RwLock<BTree>（跨 .await 死锁） |
| 5 | 2026-05 | 哈希连接 | 等值连接 O(N+M)，最常见场景 | 嵌套循环（O(N×M)） |
| 6 | 2026-05 | Volcano Hash Aggregation | 匹配现有架构，改动最小 | 排序聚合（需 SortExecutor 依赖） |
| 7 | 2026-05 | 严格 GROUP BY 模式 | SQL 标准一致，防歧义 | 宽松模式（结果不确定） |
| 8 | 2026-05 | HAVING 复用 Predicate 体系 | HavingExecutor 结构同 FilterExecutor | 独立谓词体系（重复代码） |

## PhysicalPlan 节点（15 种）

| 节点 | 输入 | 用途 |
|------|------|------|
| Scan | - | 全表扫描 |
| IndexScan | - | 主键索引扫描 |
| Filter | 1 | WHERE 过滤 |
| Join | 2 | 哈希连接（INNER JOIN） |
| Aggregate | 1 | 聚合 + GROUP BY |
| Having | 1 | HAVING 过滤（聚合后） |
| Sort | 1 | ORDER BY |
| Limit | 1 | LIMIT/OFFSET |
| Insert/Update/Delete | - | DML |
| CreateTable/DropTable | - | DDL |

## 数据流（查询执行）

```
SQL → Parser → PlanBuilder(+PlanCache) → PhysicalPlan
  → Pipeline::create_executor_from_plan (递归构建 Executor Tree)
  → Executor::next() 拉取行流
  → Response::QueryResult { rows }
```

### 聚合查询数据流

```
SQL: SELECT dept, COUNT(*) FROM emp GROUP BY dept HAVING COUNT(*) > 3

Parser → PlanBuilder:
  1. 检测聚合函数 (COUNT/SUM/AVG/MIN/MAX)
  2. 验证严格模式（非聚合列必须在 GROUP BY）
  3. 构建: Scan → Aggregate → Having → Sort → Limit

Pipeline → Executor Tree:
  ScanExecutor → AggregateExecutor → HavingExecutor

执行:
  1. AggregateExecutor.consume_input() 消耗全部输入
     - 提取分组键 → HashMap<Vec<Value>, Vec<AggregateState>>
     - 逐行更新 AggregateState
  2. build_output_rows() 物化结果
  3. HavingExecutor 过滤聚合结果行
```

## 存储层

### BufferPool（Clock 淘汰 + 两阶段锁）
- 读: `get_page()` → PageGuard
- 写: `get_page_for_write()` → PageGuard + mark_dirty
- 零拷贝: `PageGuard::page_data()` → &[u8]
- 修改: `PageGuard::modify_page(f)` → 自动 dirty

### BTree 索引
- 读路径: `AtomicPageId` + `search_async` (无 spawn_blocking)
- 写路径: `BTree::from_root()` + `spawn_blocking`
- 页格式: `LeafNodeRef/InternalNodeRef` (零拷贝读取)

### 事务（MVCC）
- 版本链: VersionChain + Snapshot
- 行锁: RowLock（写写冲突检测）
- WAL: WalWriter + Recovery + Checkpoint