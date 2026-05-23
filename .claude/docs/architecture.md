# 架构决策记录

> 最后更新：2026-05-23（M17-Phase1 非唯一索引 完成）

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
│ (register +  │     (19 种节点)
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
│  SemiJoin → AntiJoin                │
│  SubqueryEval → DerivedScan         │
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
| 9 | 2026-05 | 子查询混合策略 | WHERE→SemiJoin/AntiJoin O(N+M)，SELECT→SubqueryEval，FROM→DerivedScan | 全部嵌套循环或全部反嵌套 |
| 10 | 2026-05 | CorrelatedParam 机制 | 相关子查询通过参数注入外层值，避免闭包捕获 | 参数化查询/延迟绑定 |
| 11 | 2026-05 | ParameterExpression + Mutex 注入 | 外层列引用在谓词树中以 ParameterExpression 占位，按行 clone+inject+rebuild executor，无需修改 Expression trait 签名 | 深度克隆谓词树 + 类型匹配（复杂且需 as_any） |
| 12 | 2026-05 | 非唯一索引同页多条目方案 | Key 允许重复，同一 key 多个 slot 在同页，利用现有 SlottedPage 结构，最小改动 | 溢出页链表（需新增页类型和管理器） |
| 13 | 2026-05 | LeafNode 去掉 DuplicateKey 检查 | 允许重复 key 插入，非唯一索引基础 | 保持唯一索引限制（需索引类型区分） |
| 14 | 2026-05 | LeafNodeRef::find_all_matches | 非唯一索引查询遍历所有匹配 slot | 二分查找首个匹配（需额外逻辑处理多匹配） |
| 15 | 2026-05 | BTree 批量/精确删除方法 | delete_by_key（删除所有匹配） + delete_exact（key+RowId 精确删除） | 仅支持单 key 删除（非唯一场景受限） |

## PhysicalPlan 节点（19 种）

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
| SemiJoin | 2 | IN/EXISTS 子查询（仅输出左表匹配行） |
| AntiJoin | 2 | NOT IN/NOT EXISTS（仅输出左表不匹配行） |
| SubqueryEval | 1 | SELECT 标量子查询 |
| DerivedScan | 1 | FROM 子查询（派生表） |
| Insert/Update/Delete | - | DML |
| CreateTable/DropTable | - | DDL |

## 数据流（查询执行）

```
SQL → Parser → PlanBuilder(+PlanCache) → PhysicalPlan
  → Pipeline::create_executor_from_plan (递归构建 Executor Tree)
  → Executor::next() 拉取行流
  → Response::QueryResult { rows }
```

### 子查询数据流

```
WHERE IN 子查询:
  SQL: SELECT * FROM emp WHERE dept IN (SELECT dept FROM dept_table WHERE region = 'east')
  Plan: SemiJoin(Scan(emp), Filter(Scan(dept_table)), conditions=[emp.dept = dept_table.dept])
  Exec: BuildRight(hash) → ScanLeft(probe) → Output matching left rows

WHERE EXISTS 子查询:
  Plan: SemiJoin(Scan(emp), Filter(Scan(dept_table)), conditions=[])
  Exec: BuildRight(has_rows?) → ScanLeft → Output left rows if right non-empty

SELECT 标量子查询:
  SQL: SELECT name, (SELECT AVG(salary) FROM emp) AS avg_sal FROM dept
  Plan: SubqueryEval(Scan(dept), Aggregate(Scan(emp)))
  Exec: For each input row → eval subquery once (cached if independent) → insert result

FROM 派生表:
  SQL: SELECT t.dept FROM (SELECT dept, AVG(salary) FROM emp GROUP BY dept) AS t
  Plan: DerivedScan(Aggregate(Scan(emp)))
  Exec: Materialize subquery → iterate as virtual Scan

### 相关子查询数据流（M16-Phase2）

```
SQL: SELECT emp.name FROM emp WHERE emp.dept IN
     (SELECT dept.id FROM dept WHERE dept.id = emp.dept)

Plan 构建:
  1. Planner 检测子查询 WHERE 中 emp.dept 为外层引用
  2. 设置 inner_table_names = ["dept"]，调用 build_expression
  3. build_expression 检查 table_ref "emp" 不在 inner_tables → 创建 ParameterExpression("emp.dept")
  4. 生成 CorrelatedParam { outer_table: "emp", outer_column: "dept", param_name: "emp.dept" }
  5. 创建 SemiJoinNode { correlated_params: [CorrelatedParam(...)] }

Plan 执行（每外层行）:
  1. ScanLeft 获取外层行 [Alice, 10, 50000]
  2. 提取参数: param_values = [("emp.dept", Value::Int(10))]
  3. clone right_plan → inject_correlated_values(clone, param_values)
     → 遍历谓词树，找到 ParameterExpression("emp.dept")，Mutex::set(10)
  4. create_executor_from_plan(clone, database) → 重建右表执行器
  5. 物化右表到 hashmap → probe → 匹配则输出
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