# 架构决策记录

> 最后更新：2026-05-23（M17.5 存储架构分析完成）

## 存储层架构决策（M17.5 新增）

### ADR-001: 两层分离索引结构（2026-05-23）

**决策**：RTsql 使用两层分离的索引结构（索引页 + 数据页），而非 SQLite 的聚簇索引。

**原因**：
- ✅ **灵活性**：支持多索引、非唯一索引模式（M17 已验证）
- ✅ **MVCC 友好**：数据页独立管理，版本链实现更简单
- ✅ **实现简洁**：避免聚簇索引的复杂性

**代价**：
- ❌ **空间开销**：文件大小 ~3x larger（索引页额外开销）
- ❌ **点查询路径**：索引页 → 数据页（两次页访问）

**验证结果**（M17.5）：
- PK lookup 仍然 5.6x faster than SQLite（得益于零拷贝 + MVCC 无锁读）
- 非唯一索引功能正常工作（search_all 支持重复键）

**替代方案**：
- SQLite 聚簇索引：更紧凑，但灵活性受限
- PostgreSQL 分离索引：类似 RTsql，验证了架构合理性

---

### ADR-002: 固定长度 Key（32 bytes）（2026-05-23）

**决策**：B-Tree Key 使用固定 32 bytes 存储，而非变长编码。

**原因**：
- ✅ **实现简洁**：避免变长 Key 的复杂性（边界处理、内存管理）
- ✅ **性能稳定**：固定长度减少 CPU 开销
- ✅ **调试友好**：易于追踪和验证

**代价**：
- ❌ **空间浪费**：短 Key（如 INT PRIMARY KEY）浪费 ~28 bytes
- ❌ **长 Key 限制**：无法支持 >32 bytes 的 Key

**影响分析**：
- 10K rows 索引开销：420KB（固定 Key） vs ~40KB（varint）
- 总文件大小影响：~10x per key，但总开销占比合理

**后续优化**（M18+）：
- Varint Key 编码：减少 ~70% Key 开销，但增加实现复杂性

---

### ADR-003: SlottedPage 页格式（2026-05-23）

**决策**：使用标准 SlottedPage 格式（Slot 数组 + Row Data），而非紧凑存储。

**原因**：
- ✅ **标准格式**：主流数据库（PostgreSQL、MySQL）都使用类似格式
- ✅ **MVCC 友好**：Slot 易于管理多个版本（版本链指针）
- ✅ **零拷贝读**：page_data() 直接访问，无需反序列化整个页

**代价**：
- ❌ **Slot overhead**：每个 entry 多 4 bytes（offset + length）
- ❌ **页填充率低**：50-70%（不如 SQLite 的 70-90%）

**影响分析**：
- 每个 tuple 多 4 bytes Slot overhead
- 页填充率低导致文件大小 ~1.3x larger

---

### ADR-004: 自定义二进制序列化（2026-05-23）

**决策**：使用自定义二进制格式（Tag + Value），而非 JSON 或 Protobuf。

**格式定义**：
```
Int    = [Tag 0x01][8 bytes i64 LE]
String = [Tag 0x02][2 bytes len][N bytes UTF-8]
Null   = [Tag 0x03]
Float  = [Tag 0x04][8 bytes f64]
Bool   = [Tag 0x05][1 byte]
```

**原因**：
- ✅ **紧凑高效**：比 JSON 更小更快
- ✅ **类型安全**：Tag byte 明确类型标记
- ✅ **实现简洁**：无需外部依赖（Protobuf）

**代价**：
- ❌ **Tag overhead**：每个值多 1 byte
- ❌ **固定长度**：Int/Float 固定 8 bytes（不如 varint）

**对比**：
- RTsql: Int = 9 bytes (Tag + 8B)
- SQLite varint: Int = 1-9 bytes（平均 ~2-3 bytes）

---

## 架构权衡总结

| 设计决策 | 空间代价 | 性能收益 | 灵活性收益 |
|----------|---------|---------|-----------|
| 两层分离索引 | ~3x larger | PK lookup 5.6x faster ⚡ | 多索引、非唯一索引 ✅ |
| 固定 Key 32B | ~10x per key | CPU 开销低 ✅ | 实现简洁 ✅ |
| SlottedPage | ~1.3x larger | MVCC 无锁读 ⚡ | 版本链管理 ✅ |
| 二进制序列化 | ~1.2x larger | 比 JSON 快 ✅ | 无外部依赖 ✅ |

**总体权衡**：空间效率换取实现简洁性 + 架构灵活性。

**验证结果**：
- ✅ INSERT 332x faster（异步 I/O + 两阶段锁）
- ✅ PK lookup 5.6x faster（零拷贝 + MVCC）
- ✅ 非唯一索引正常（两层分离架构优势）

---

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