# M15 聚合函数与 GROUP BY 设计文档

> 创建日期：2026-05-22
> 状态：设计完成，待实现

---

## 1. 需求概要

实现 SQL 聚合查询支持，包括：
- 5 个核心聚合函数：COUNT(*)、COUNT(col)、SUM(col)、AVG(col)、MIN(col)、MAX(col)
- GROUP BY 分组
- HAVING 聚合结果过滤
- SQL 标准 NULL 处理语义
- 严格模式：SELECT 中非聚合列必须出现在 GROUP BY 中

---

## 2. 设计决策

| 决策 | 选择 | 原因 |
|------|------|------|
| 聚合范围 | COUNT/SUM/AVG/MIN/MAX | 覆盖最常见 SQL 查询需求 |
| GROUP BY 语义 | 严格模式 | 与 SQL 标准一致，防止歧义 |
| NULL 处理 | SQL 标准 | COUNT(*) 计所有行，COUNT(col)/SUM/AVG/MIN/MAX 跳过 NULL |
| 空表聚合 | 返回单行 | COUNT→0，其他→NULL，符合 SQL 标准 |
| HAVING | 一起支持 | 用户要求完整聚合查询支持 |
| 实现方案 | Volcano Hash Aggregation | 匹配现有架构，改动最小 |

---

## 3. 新增类型定义

### 3.1 AggregateFunc（聚合函数枚举）

```rust
/// 聚合函数类型
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateFunc {
    /// COUNT(*) — 计算所有行数（包括 NULL）
    CountStar,
    /// COUNT(column) — 计算非 NULL 行数
    Count(String),
    /// SUM(column) — 求和（跳过 NULL）
    Sum(String),
    /// AVG(column) — 求平均（跳过 NULL）
    Avg(String),
    /// MIN(column) — 最小值（跳过 NULL）
    Min(String),
    /// MAX(column) — 最大值（跳过 NULL）
    Max(String),
}
```

### 3.2 AggregateState（聚合累积器）

每个聚合函数维护一个累积器，逐行更新。分组聚合时，每组独立维护一组累积器。

```rust
/// 聚合累积状态
#[derive(Debug, Clone)]
pub enum AggregateState {
    /// COUNT(*) 累积器
    CountStar(i64),
    /// COUNT(col) 累积器
    Count { count: i64 },
    /// SUM 累积器（NULL 表示尚未遇到非 NULL 值）
    Sum { sum: Option<Value>, count: i64 },
    /// AVG 累积器（同时追踪 sum 和 count）
    Avg { sum: Option<Value>, count: i64 },
    /// MIN 累积器
    Min(Option<Value>),
    /// MAX 累积器
    Max(Option<Value>),
}
```

关键方法：
- `update(value: &Value)` — 逐行更新累积器
- `finalize() -> Value` — 输出最终聚合结果
  - CountStar → `Value::Int(count)`
  - Count → `Value::Int(count)`
  - Sum → `sum.unwrap_or(Value::Null)`
  - Avg → `sum / count` 或 `Value::Null`（无非 NULL 值时）
  - Min/Max → `opt.unwrap_or(Value::Null)`（无非 NULL 值时）

### 3.3 PhysicalPlan 新增节点

**AggregateNode**：

```rust
#[derive(Debug, Clone)]
pub struct AggregateNode {
    /// 输入计划（Scan / Filter / Join 等）
    pub input: Box<PhysicalPlan>,
    /// GROUP BY 列名列表
    pub group_by: Vec<String>,
    /// 聚合函数列表
    pub aggregates: Vec<AggregateFunc>,
    /// 输出列名列表（分组列 + 聚合结果列）
    pub output_columns: Vec<String>,
    /// 输出列对应的列索引（用于从输入行提取分组键和聚合值）
    pub column_indices: HashMap<String, usize>,
    /// 表名（用于列查找）
    pub table_name: String,
}
```

**HavingNode**：

```rust
#[derive(Debug, Clone)]
pub struct HavingNode {
    /// 输入计划（通常是 Aggregate）
    pub input: Box<PhysicalPlan>,
    /// HAVING 条件谓词
    pub predicate: PredicateRef,
    /// 表名
    pub table_name: String,
}
```

---

## 4. PlanBuilder 修改

### 4.1 聚合检测

在 `build_query` 中，解析 SELECT 投影列时检测聚合函数：

1. 遍历 `select.projection`，识别 `Expr::Function` 节点
2. 函数名为 COUNT/SUM/AVG/MIN/MAX → 标记为聚合函数
3. 其他列名 → 标记为非聚合列

### 4.2 GROUP BY 解析

从 `select.group_by` 提取分组列名列表。

### 4.3 严格模式验证

如果 SELECT 中存在非聚合列且不在 GROUP BY 中 → 报错 `PlanError::NonAggregatedColumn`。

### 4.4 构建计划链

有聚合函数时，计划构建顺序：

```
Scan/Filter/Join → Aggregate → Having → Sort → Limit
```

无 GROUP BY 时，所有行归入一个组。

### 4.5 HAVING 解析

HAVING 条件从 `select.having` 提取，使用 `build_where` 方法构建 Predicate（复用现有谓词体系）。

HAVING 谓词中的聚合函数引用需要特殊处理：
- `COUNT(*) > 5` → 在 HavingNode 中，聚合结果列名作为列引用
- 聚合结果列名格式：`count_star`, `count_{col}`, `sum_{col}`, `avg_{col}`, `min_{col}`, `max_{col}`

---

## 5. Executor 实现

### 5.1 AggregateExecutor

```rust
pub struct AggregateExecutor {
    input: Box<dyn Executor + Send>,
    group_by: Vec<String>,
    aggregates: Vec<AggregateFunc>,
    column_indices: HashMap<String, usize>,
    /// 分组结果（消耗完所有输入后填充）
    groups: HashMap<Vec<Value>, Vec<AggregateState>>,
    /// 无 GROUP BY 时的单组状态
    single_group: Option<Vec<AggregateState>>,
    /// 输出迭代器状态
    output_iter: Option<Vec<Vec<Value>>>,
    has_consumed_input: bool,
}
```

执行逻辑：
1. 首次 `next()` 调用 → 消耗所有输入行
   - 提取分组键（group_by 列的值）
   - 对每组更新对应 AggregateState
2. 消耗完毕 → 将 groups HashMap 转为结果行列表
3. 后续 `next()` → 逐行输出聚合结果
4. 无 GROUP BY → 所有行归入 single_group
5. 空输入（无 GROUP BY）→ 返回单行：COUNT→0，其他→NULL

输出行格式：`[group_col_1, group_col_2, ..., agg_result_1, agg_result_2, ...]`

### 5.2 HavingExecutor

```rust
pub struct HavingExecutor {
    input: Box<dyn Executor + Send>,
    predicate: PredicateRef,
}
```

与 FilterExecutor 结构完全相同，仅语义不同（过滤聚合结果行而非原始行）。

---

## 6. Pipeline 修改

`create_executor_from_plan` 新增两个分支：

```rust
PhysicalPlan::Aggregate(node) => {
    let input = create_executor_from_plan(*node.input, database).await?;
    Ok(Box::new(AggregateExecutor::new(input, node)) as Box<dyn Executor + Send>)
}

PhysicalPlan::Having(node) => {
    let input = create_executor_from_plan(*node.input, database).await?;
    Ok(Box::new(HavingExecutor::new(input, node.predicate)) as Box<dyn Executor + Send>)
}
```

---

## 7. 错误类型

新增 `PlanError` 变体：

```rust
/// 非聚合列未出现在 GROUP BY 中（严格模式）
NonAggregatedColumn(String),
/// 聚合函数参数错误
InvalidAggregateArgument(String),
/// GROUP BY 列不存在
GroupByColumnNotFound(String),
/// HAVING 中引用非聚合列
HavingNonAggregatedReference(String),
```

---

## 8. SQL 示例与预期行为

### 8.1 基本聚合

```sql
-- 无 GROUP BY，对全表聚合
SELECT COUNT(*) FROM users;
-- 空表 → [{count_star: 0}]

SELECT AVG(age) FROM users;
-- 空表 → [{avg_age: NULL}]

SELECT SUM(salary), COUNT(*) FROM employees;
-- → [{sum_salary: 150000, count_star: 3}]
```

### 8.2 GROUP BY

```sql
SELECT department, COUNT(*) FROM employees GROUP BY department;
-- → [{department: "Engineering", count_star: 5}, {department: "Sales", count_star: 3}]

SELECT department, AVG(salary) FROM employees GROUP BY department;
-- → [{department: "Engineering", avg_salary: 80000}, {department: "Sales", avg_salary: 50000}]
```

### 8.3 HAVING

```sql
SELECT department, COUNT(*) FROM employees
GROUP BY department
HAVING COUNT(*) > 3;
-- → 只输出部门人数 > 3 的组
```

### 8.4 NULL 处理

```sql
SELECT COUNT(*), COUNT(age), SUM(age) FROM users;
-- COUNT(*) 计所有行（包括 age=NULL 的行）
-- COUNT(age) 跳过 age=NULL 的行
-- SUM(age) 跳过 age=NULL 的行
```

### 8.5 严格模式报错

```sql
-- ❌ name 不在 GROUP BY 中
SELECT name, COUNT(*) FROM users GROUP BY department;
-- → Error: Non-aggregated column 'name' must appear in GROUP BY clause
```

---

## 9. 测试覆盖

### 9.1 单元测试

| 测试 | 覆盖 |
|------|------|
| test_count_star_empty | 空表 COUNT(*) → 0 |
| test_count_star_basic | 基本行计数 |
| test_count_column_null | COUNT(col) 跳过 NULL |
| test_sum_basic | SUM 基本求和 |
| test_sum_null | SUM 跳过 NULL |
| test_sum_empty | 空表 SUM → NULL |
| test_avg_basic | AVG 基本平均值 |
| test_avg_null | AVG 跳过 NULL |
| test_avg_empty | 空表 AVG → NULL |
| test_min_max_basic | MIN/MAX 基本功能 |
| test_min_max_null | MIN/MAX 跳过 NULL |
| test_min_max_empty | 空表 MIN/MAX → NULL |
| test_group_by_single | 单列分组 |
| test_group_by_multi | 多列分组 |
| test_having_basic | HAVING 过滤 |
| test_having_with_aggregate_ref | HAVING 引用聚合函数 |
| test_strict_mode_error | 非聚合列报错 |
| test_aggregate_with_where | 聚合 + WHERE |
| test_aggregate_with_order_limit | 聚合 + ORDER BY + LIMIT |

### 9.2 集成测试

通过 pipeline 执行完整 SQL 语句，验证端到端行为。

---

## 10. 文件变更清单

| 文件 | 操作 | 内容 |
|------|------|------|
| `src/executor/aggregate.rs` | 新增 | AggregateFunc, AggregateState, AggregateExecutor |
| `src/executor/having.rs` | 新增 | HavingExecutor |
| `src/executor/plan.rs` | 修改 | 新增 AggregateNode, HavingNode |
| `src/executor/mod.rs` | 修改 | 导出新模块和类型 |
| `src/parser/planner.rs` | 修改 | 解析聚合函数 + GROUP BY + HAVING |
| `src/parser/error.rs` | 修改 | 新增错误类型 |
| `src/pipeline.rs` | 修改 | create_executor_from_plan 新增分支 |
| `tests/aggregate_test.rs` | 新增 | 聚合函数单元测试 |
| `tests/group_by_test.rs` | 新增 | GROUP BY + HAVING 测试 |

---

## 11. 实现顺序建议

1. AggregateFunc + AggregateState（类型定义 + 累积逻辑）
2. AggregateNode + HavingNode（Plan 节点）
3. PlanBuilder 聚合解析
4. AggregateExecutor
5. HavingExecutor
6. Pipeline 整合
7. 测试（逐步 TDD）
8. 项目文档更新