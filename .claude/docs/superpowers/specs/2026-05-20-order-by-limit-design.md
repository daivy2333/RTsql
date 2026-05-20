# M9 Phase 2: ORDER BY + LIMIT/OFFSET 设计文档

> 创建日期：2026-05-20
> 状态：Approved

---

## 1. 概述

### 目标

实现 SQL 查询的排序和分页能力：
- **ORDER BY**：多列排序，每列独立 ASC/DESC 方向
- **LIMIT/OFFSET**：分页限制，限制返回行数并跳过前 N 行

### 需求决定

| 需求项 | 决定 | 理由 |
|--------|------|------|
| 排序列数 | 多列排序 | SQL 标准功能，实现复杂度增加不大 |
| 排序方向 | 每列独立 ASC/DESC | SQL 标准行为，灵活 |
| NULL 处理 | 排在末尾（简单默认） | 嵌入式数据库简单默认足够 |
| 分页算子 | LIMIT + OFFSET 合并在一个 LimitExecutor | 经常一起使用，合并更简单 |
| 排序策略 | 内存排序 | 嵌入式场景数据量预期不大，实现简单 |

### 实现方案

采用 **两算子分离方案**（SortExecutor + LimitExecutor），理由：
- 职责单一，符合现有 FilterExecutor 包装器模式
- Plan 节点清晰，可独立测试
- 符合 TDD 原则

---

## 2. PhysicalPlan 节点扩展

### SortNode

```rust
/// 排序节点（ORDER BY）
pub struct SortNode {
    /// 输入计划（通常是 Scan 或 Filter）
    pub input: Box<PhysicalPlan>,
    /// 排序列定义列表
    pub order_by: Vec<OrderByColumn>,
    /// 表名（用于列名解析）
    pub table_name: String,
}
```

### OrderByColumn

```rust
/// 排序列定义
pub struct OrderByColumn {
    /// 列名
    pub column: String,
    /// 是否升序（true = ASC, false = DESC）
    pub asc: bool,
}
```

### LimitNode

```rust
/// 分页节点（LIMIT + OFFSET）
pub struct LimitNode {
    /// 输入计划（通常是 Sort）
    pub input: Box<PhysicalPlan>,
    /// 限制行数（LIMIT）
    pub limit: usize,
    /// 跳过行数（OFFSET）
    pub offset: usize,
}
```

### PhysicalPlan enum 扩展

```rust
pub enum PhysicalPlan {
    // ...现有节点...
    Sort(SortNode),
    Limit(LimitNode),
}
```

---

## 3. Executor 实现

### SortExecutor (`src/executor/sort.rs`)

**数据结构**：

```rust
pub struct SortExecutor {
    input: Box<dyn Executor + Send>,
    order_by: Vec<OrderByColumn>,
    sorted_rows: Vec<Vec<Value>>,  // 排序后缓存
    emitted: bool,                  // 是否已开始输出
}
```

**执行逻辑**：

```
1. 首次调用 next():
   - 循环调用 input.next()，收集所有 ExecResult::Row 到 Vec
   - 调用 Vec::sort_unstable_by，按 order_by 逐列比较
   - 设置 emitted = true

2. 后续调用 next():
   - 从 sorted_rows 逐行弹出（remove(0) 或迭代器）
   - 返回 ExecResult::Row
   - sorted_rows 为空时返回 None
```

**比较函数逻辑**：

```rust
fn compare_rows(a: &[Value], b: &[Value], order_by: &[OrderByColumn], column_index_map: &HashMap<String, usize>) -> Ordering {
    for col in order_by {
        let idx = column_index_map[&col.column];
        let cmp = compare_values(&a[idx], &b[idx]);
        
        // NULL 处理：排在末尾
        if a[idx].is_null() && !b[idx].is_null() {
            return Ordering::Greater;  // NULL 排在后面
        }
        if !a[idx].is_null() && b[idx].is_null() {
            return Ordering::Less;
        }
        if a[idx].is_null() && b[idx].is_null() {
            continue;  // 两个 NULL 相等，比较下一列
        }
        
        // 非空值比较，按方向调整
        let result = if col.asc { cmp } else { cmp.reverse() };
        if result != Ordering::Equal {
            return result;
        }
    }
    Ordering::Equal
}
```

**NULL 排序规则**：
- 无论 ASC 还是 DESC，NULL 都排在末尾
- 实现：NULL vs 非空 → Greater（排在后面）

---

### LimitExecutor (`src/executor/limit.rs`)

**数据结构**：

```rust
pub struct LimitExecutor {
    input: Box<dyn Executor + Send>,
    limit: usize,
    offset: usize,
    skipped: usize,   // 已跳过计数
    taken: usize,     // 已取计数
}
```

**执行逻辑**：

```
1. OFFSET 处理：
   - while skipped < offset:
     - input.next()
     - skipped += 1
     - 若返回 None → OFFSET 超过总行数 → 返回 None

2. LIMIT 处理：
   - if taken >= limit → 返回 None
   - input.next() → taken += 1 → 返回 Row
```

**边界情况**：
- `OFFSET > 总行数` → 返回空结果
- `LIMIT = 0` → 返回空结果
- `LIMIT > 剩余行数` → 返回所有剩余行

---

## 4. Parser 扩展

### build_order_by 方法

在 `src/parser/planner.rs` 中新增：

```rust
impl PlanBuilder {
    fn build_order_by(&mut self, base_plan: PhysicalPlan, stmt: &Select, table_name: String) -> Result<PhysicalPlan> {
        // 解析 ORDER BY 子句
        let order_by: Vec<OrderByColumn> = stmt.order_by.iter()
            .map(|o| {
                OrderByColumn {
                    column: extract_column_name(&o.expr),
                    asc: o.asc,  // sqlparser: asc=true for ASC, false for DESC
                }
            })
            .collect();
        
        if order_by.is_empty() {
            return Ok(base_plan);  // 无 ORDER BY，直接返回
        }
        
        // 构建 SortNode
        let sort_plan = PhysicalPlan::Sort(SortNode {
            input: Box::new(base_plan),
            order_by,
            table_name,
        });
        
        // 解析 LIMIT/OFFSET
        let limit = stmt.limit.map(|v| parse_limit_value(v));
        let offset = stmt.offset.map(|v| parse_offset_value(v));
        
        if let Some(limit_val) = limit {
            // 构建 LimitNode
            Ok(PhysicalPlan::Limit(LimitNode {
                input: Box::new(sort_plan),
                limit: limit_val,
                offset: offset.unwrap_or(0),
            }))
        } else {
            Ok(sort_plan)
        }
    }
    
    fn parse_limit_value(expr: &Expr) -> usize {
        // 解析 LIMIT 常量值
        match expr {
            Expr::Value(Value::Number(n, _)) => n.parse().unwrap_or(0),
            _ => 0,
        }
    }
    
    fn parse_offset_value(expr: &Offset) -> usize {
        // 解析 OFFSET 常量值
        match &expr.value {
            Expr::Value(Value::Number(n, _)) => n.parse().unwrap_or(0),
            _ => 0,
        }
    }
}
```

### extract_column_name 辅助函数

```rust
fn extract_column_name(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(ident) => ident.value.clone(),
        // 其他情况（表达式排序）暂不支持，返回空字符串或报错
        _ => String::new(),
    }
}
```

**注意**：当前版本仅支持**单列名排序**，不支持表达式排序（如 `ORDER BY a + b`）。

---

## 5. Pipeline 集成

在 `src/pipeline.rs` 的 `execute_plan` match 中新增：

```rust
match plan {
    PhysicalPlan::Sort(node) => {
        let input_executor = build_executor(node.input, db)?;
        let executor = SortExecutor::new(input_executor, node.order_by);
        collect_results(executor)
    }
    
    PhysicalPlan::Limit(node) => {
        let input_executor = build_executor(node.input, db)?;
        let executor = LimitExecutor::new(input_executor, node.limit, node.offset);
        collect_results(executor)
    }
    
    // ...现有节点...
}
```

---

## 6. 测试策略

### 单元测试

| 测试类别 | 测试内容 | 文件 |
|----------|----------|------|
| **SortExecutor** | 基础排序（单列 ASC/DESC） | `sort_test.rs` |
| | 多列排序（优先级） | |
| | NULL 排序（排在末尾） | |
| | 空输入排序 | |
| | 全 NULL 排序 | |
| **LimitExecutor** | 基础分页（LIMIT） | `limit_test.rs` |
| | OFFSET 跳过 | |
| | LIMIT + OFFSET 组合 | |
| | OFFSET 超过总行数 | |
| | LIMIT = 0 | |

### 集成测试

| 测试类别 | 测试内容 | 文件 |
|----------|----------|------|
| **Parser** | ORDER BY 单列解析 | `planner_test.rs` |
| | ORDER BY 多列解析 | |
| | LIMIT 解析 | |
| | OFFSET 解析 | |
| | ORDER BY + LIMIT 组合 | |
| **Pipeline** | SELECT ORDER BY 端到端 | `pipeline_test.rs` |
| | SELECT ORDER BY LIMIT 端到端 | |
| | SELECT WHERE ORDER BY LIMIT 端到端 | |

### E2E 测试

在 `tests/e2e_test.rs` 中新增：
- TCP 连接 + INSERT + SELECT ORDER BY LIMIT
- 验证返回行顺序和数量

---

## 7. 文件结构

新增文件：

| 文件 | 内容 |
|------|------|
| `src/executor/sort.rs` | SortExecutor 实现 |
| `src/executor/limit.rs` | LimitExecutor 实现 |
| `tests/sort_test.rs` | SortExecutor 单元测试 |
| `tests/limit_test.rs` | LimitExecutor 单元测试 |

修改文件：

| 文件 | 修改内容 |
|------|----------|
| `src/executor/plan.rs` | 新增 SortNode、LimitNode、OrderByColumn |
| `src/executor/mod.rs` | 导出 SortExecutor、LimitExecutor |
| `src/parser/planner.rs` | 新增 build_order_by 方法 |
| `src/pipeline.rs` | 新增 Sort/Limit execute_plan 分支 |
| `tests/planner_test.rs` | 新增 ORDER BY + LIMIT 解析测试 |
| `tests/pipeline_test.rs` | 新增 ORDER BY + LIMIT 端到端测试 |

---

## 8. 实现顺序

按照 TDD 原则，实现顺序：

1. **PhysicalPlan 节点**（SortNode、LimitNode、OrderByColumn）
2. **SortExecutor 单元测试**（先写测试）
3. **SortExecutor 实现**（让测试通过）
4. **LimitExecutor 单元测试**（先写测试）
5. **LimitExecutor 实现**（让测试通过）
6. **Parser 扩展**（build_order_by）
7. **Parser 测试**（planner_test.rs）
8. **Pipeline 集成**（execute_plan 分支）
9. **Pipeline 测试**（pipeline_test.rs）
10. **E2E 测试**（e2e_test.rs）

---

## 9. 约束与限制

### 当前版本约束

1. **仅支持列名排序**：不支持表达式排序（如 `ORDER BY a + b`）
2. **内存排序**：不支持外部排序（数据量大时可能内存不足）
3. **NULL 固定末尾**：不支持 NULLS FIRST/LAST 语法
4. **单表排序**：不支持 JOIN 场景的排序（M12 实现后支持）

### 符合 Surgical Changes 原则

- 只修改必要文件，不"顺便"重构
- 保持现有 Executor 模式一致
- 不添加未要求的"灵活性"

---

## 10. 里程碑路线图

| 里程碑 | 优先级 | 描述 |
|--------|--------|------|
| **M9 Phase 2** | 🔴 高 | ORDER BY + LIMIT/OFFSET（本文档） |
| M10 | 🟡 中 | 完整版本链遍历 + 版本链 GC |
| M11 | 🔴 高 | WAL 持久化 + 崩溃恢复 |
| M12 | 🟢 低 | JOIN 多表支持 |

---

## 附录 A: sqlparser-rs API 参考

**Select 结构**（`src/parser/planner.rs` 使用）：

```rust
pub struct Select {
    pub order_by: Vec<OrderByExpr>,  // ORDER BY 子句
    pub limit: Option<Expr>,         // LIMIT 子句
    pub offset: Option<Offset>,      // OFFSET 子句
}

pub struct OrderByExpr {
    pub expr: Expr,                  // 排序表达式（列名或表达式）
    pub asc: bool,                   // true = ASC, false = DESC
}

pub struct Offset {
    pub value: Expr,                 // OFFSET 常量值
}
```

**解析示例**：

```sql
SELECT * FROM users ORDER BY age DESC, name ASC LIMIT 10 OFFSET 5
```

解析为：
```rust
Select {
    order_by: [
        OrderByExpr { expr: Identifier("age"), asc: false },  // DESC
        OrderByExpr { expr: Identifier("name"), asc: true },  // ASC
    ],
    limit: Some(Value::Number("10")),
    offset: Some(Offset { value: Value::Number("5") }),
}
```