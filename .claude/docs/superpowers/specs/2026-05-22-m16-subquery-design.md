# M16 子查询支持设计规范

> 日期：2026-05-22
> 里程碑：M16
> 状态：Draft

## 1. 目标

为 RTsql 嵌入式数据库全面支持 SQL 子查询，包括：
- WHERE 子查询：`IN (SELECT ...)`、`EXISTS (SELECT ...)`、`NOT IN`、`NOT EXISTS`
- SELECT 列子查询：标量子查询 `(SELECT avg(salary) FROM emp)`
- FROM 子查询：派生表 `SELECT * FROM (SELECT ...) AS t`
- 相关子查询：子查询引用外层查询列

## 2. 实现策略

**混合策略**（方案 A）：
- WHERE 中 IN/EXISTS → **反嵌套**为 SemiJoin（O(N+M) 性能）
- SELECT 标量子查询 → **保留 Volcano 节点**（SubqueryEval）
- FROM 子查询 → **DerivedScan 节点**（物化为虚拟 Scan）
- 相关子查询 → **CorrelatedParam 机制**传递外层列值

## 3. 新增 Plan 节点

### 3.1 SemiJoinNode

```rust
/// Semi-Join 节点（IN/EXISTS 子查询反嵌套）
#[derive(Debug, Clone)]
pub struct SemiJoinNode {
    /// 左表计划（外层查询）
    pub left: Box<PhysicalPlan>,
    /// 右表计划（子查询物化结果）
    pub right: Box<PhysicalPlan>,
    /// 等值条件（左表列 = 子查询结果列）
    pub conditions: Vec<JoinCondition>,
    /// 输出列（仅左表列）
    pub output_columns: Vec<OutputColumn>,
    /// 相关子查询参数（空 = 独立子查询）
    pub correlated_params: Vec<CorrelatedParam>,
}
```

**执行逻辑**：
1. 物化右表全部行到 HashSet（按条件列值建 hash）
2. 逐行扫描左表，检查左表条件列值是否在 HashSet 中
3. 存在 → 输出该行；不存在 → 跳过
4. 相关子查询：每行重新物化右表（外层列值注入子查询 Filter）

### 3.2 AntiJoinNode

```rust
/// Anti-Join 节点（NOT IN / NOT EXISTS 子查询反嵌套）
#[derive(Debug, Clone)]
pub struct AntiJoinNode {
    pub left: Box<PhysicalPlan>,
    pub right: Box<PhysicalPlan>,
    pub conditions: Vec<JoinCondition>,
    pub output_columns: Vec<OutputColumn>,
    pub correlated_params: Vec<CorrelatedParam>,
}
```

**执行逻辑**：与 SemiJoin 相反——左表条件列值不在右表 HashSet 中时输出该行。

### 3.3 SubqueryEvalNode

```rust
/// 标量子查询求值节点（SELECT 列中的子查询）
#[derive(Debug, Clone)]
pub struct SubqueryEvalNode {
    /// 外层查询输入
    pub input: Box<PhysicalPlan>,
    /// 子查询计划（标量子查询）
    pub subquery: Box<PhysicalPlan>,
    /// 结果列名
    pub output_column: String,
    /// 子查询结果在输出行中的列索引
    pub result_column_index: usize,
    /// 相关子查询参数
    pub correlated_params: Vec<CorrelatedParam>,
}
```

**执行逻辑**：
1. 逐行调用 `input.next()` 获取外层行
2. 执行子查询 Plan（相关时注入外层列值）
3. 取子查询首行首列作为标量结果
4. 将标量结果追加到外层行，输出

### 3.4 DerivedScanNode

```rust
/// FROM 子查询（派生表）节点
#[derive(Debug, Clone)]
pub struct DerivedScanNode {
    /// 子查询计划
    pub subquery: Box<PhysicalPlan>,
    /// 派生表别名
    pub alias: String,
    /// 输出列名列表
    pub columns: Vec<String>,
}
```

**执行逻辑**：
1. 执行子查询 Plan，物化全部行
2. 以物化结果作为 Scan 的数据源（类似内存表）

### 3.5 CorrelatedParam

```rust
/// 相关子查询参数（外层列 → 内层替换位置）
#[derive(Debug, Clone)]
pub struct CorrelatedParam {
    /// 外层列引用
    pub outer_column: ColumnRef,
    /// 子查询 Plan 中需要替换为参数值的列索引
    pub inner_column_index: usize,
}
```

**传递机制**：
- SemiJoin/AntiJoin 相关执行：每行更新右表 Plan 的 Filter 谢词参数值，重新物化右表
- SubqueryEval 相关执行：每行更新子查询 Plan 的 Filter 谢词参数值，重新执行子查询

## 4. PlanBuilder 变更

### 4.1 子查询检测与分类

在 `build_query` 中新增子查询检测逻辑：

```
遍历 select.projection:
  - Expr::Subquery → 标量子查询 → SubqueryEvalNode

遍历 select.selection (WHERE):
  - Expr::InSubquery { expr, subquery, negated }:
    - negated=false → SemiJoinNode
    - negated=true → AntiJoinNode
  - Expr::Exists { subquery }:
    - negated=false → SemiJoinNode (无条件列，仅检查非空)
    - negated=true → AntiJoinNode

遍历 select.from:
  - TableFactor::Derived { subquery, alias } → DerivedScanNode
```

### 4.2 新增方法

- `build_subquery(query: &Query) -> Result<PhysicalPlan>`：递归处理子查询的 Query 对象
- `extract_correlated_params(expr: &Expr, outer_tables: &[String]) -> Result<Vec<CorrelatedParam>>`：检测相关子查询引用的外层列

### 4.3 WHERE 处理变更

当前 `build_where` 仅处理简单比较和逻辑组合。扩展为：
1. 先检测子查询模式（IN/EXISTS）
2. 子查询模式 → 构建 SemiJoin/AntiJoin
3. 非子查询模式 → 保持现有 Filter 逻辑

## 5. Executor 实现

### 5.1 SemiJoinExecutor

基于现有 HashJoinExecutor 模式，关键差异：
- 构建阶段：物化右表到 `HashMap<Vec<Value>, Vec<Vec<Value>>>`
- 探测阶段：左表每行检查是否匹配 → 匹配则输出（仅左表列）
- 不需要右表列输出

**相关子查询处理**：
- 每行左表数据到来时，将 `correlated_params` 的外层列值注入到子查询 Plan 的 Filter
- 重新物化右表
- 探测匹配

### 5.2 AntiJoinExecutor

与 SemiJoinExecutor 相反：不匹配时输出左表行。

### 5.3 SubqueryEvalExecutor

```rust
struct SubqueryEvalExecutor {
    input: Box<dyn Executor + Send>,
    // 子查询通过 Pipeline::create_executor_from_plan 在每行重新创建
    // 因为相关子查询需要注入不同的参数值
    subquery_plan: PhysicalPlan,
    database: Arc<Database>,
    output_column: String,
    result_column_index: usize,
    correlated_params: Vec<CorrelatedParam>,
}
```

**执行**：
- 独立子查询：预执行一次子查询，缓存标量结果
- 相关子查询：每行创建新子查询执行器，注入参数，取首行首列

### 5.4 DerivedScanExecutor

```rust
struct DerivedScanExecutor {
    rows: Vec<Vec<Value>>,  // 物化的子查询结果
    columns: Vec<String>,
    current_row: usize,
}
```

**执行**：从 `rows` 逐行返回，类似 ScanExecutor 但数据来自内存而非 BufferPool。

### 5.5 CorrelatedParamInjector

辅助工具，在 SemiJoin/AntiJoin/SubqueryEval 中使用：

```rust
fn inject_correlated_values(
    plan: &mut PhysicalPlan,
    params: &[CorrelatedParam],
    outer_row: &[Value],
) {
    // 遍历 Plan 树，找到相关子查询的 FilterNode
    // 将 FilterNode.predicate 中的 ColumnExpression 替换为常量值
}
```

**注意**：相关子查询参数注入不修改原始 Plan。每次执行时，通过 `plan.deep_clone()` 创建新的 Plan 副本，在副本上替换相关列为常量值。这样原始 Plan 保持不变，可安全复用。

## 6. Pipeline 变更

`create_executor_from_plan` 新增 3 个分支：

```rust
PhysicalPlan::SemiJoin(node) => {
    let left_executor = create_executor_from_plan(*node.left, database).await?;
    let right_executor = create_executor_from_plan(*node.right, database).await?;
    Ok(Box::new(SemiJoinExecutor::new(...)) as Box<dyn Executor + Send>)
}

PhysicalPlan::AntiJoin(node) => {
    // 同上，AntiJoinExecutor
}

PhysicalPlan::SubqueryEval(node) => {
    let input_executor = create_executor_from_plan(*node.input, database).await?;
    Ok(Box::new(SubqueryEvalExecutor::new(input_executor, node.subquery, ...)) as Box<dyn Executor + Send>)
}

PhysicalPlan::DerivedScan(node) => {
    let subquery_executor = create_executor_from_plan(*node.subquery, database).await?;
    // 物化子查询结果
    let rows = materialize(subquery_executor).await;
    Ok(Box::new(DerivedScanExecutor::new(rows, node.columns)) as Box<dyn Executor + Send>)
}
```

## 7. PlanError 新增错误类型

```rust
/// 子查询返回多行（标量子查询要求单行）
SubqueryReturnsMultipleRows,
/// 子查询返回多列（IN 子查询要求单列）
SubqueryReturnsMultipleColumns,
/// 标量子查询返回空结果
SubqueryReturnsEmpty,
/// 不支持的子查询位置
UnsupportedSubqueryPosition,
/// 相关子查询参数解析错误
CorrelatedParamError(String),
/// NOT IN 子查询包含 NULL 值
NotInWithNull,
```

## 8. sqlparser-rs AST 映射

sqlparser-rs 已支持以下子查询 AST 类型：

| SQL 语法 | sqlparser AST 类型 | Plan 映射 |
|----------|---------------------|-----------|
| `WHERE x IN (SELECT ...)` | `Expr::InSubquery { expr, subquery, negated: false }` | SemiJoinNode |
| `WHERE x NOT IN (SELECT ...)` | `Expr::InSubquery { expr, subquery, negated: true }` | AntiJoinNode |
| `WHERE EXISTS (SELECT ...)` | `Expr::Exists { subquery, negated: false }` | SemiJoinNode (conditions 为空，仅检测右表非空) |
| `WHERE NOT EXISTS (SELECT ...)` | `Expr::Exists { subquery, negated: true }` | AntiJoinNode (conditions 为空) |

**EXISTS 子查询的特殊处理**：EXISTS 不需要等值条件列，仅需检测子查询结果是否非空。SemiJoinNode 的 `conditions` 为空时，执行逻辑变为：右表有任意行 → 左表行匹配；右表空 → 左表行不匹配。
| `SELECT (SELECT ...)` | `Expr::Subquery` | SubqueryEvalNode |
| `FROM (SELECT ...) AS t` | `TableFactor::Derived { subquery, alias }` | DerivedScanNode |

## 9. 测试覆盖

### 9.1 WHERE IN 子查询（独立）

- `SELECT * FROM emp WHERE dept IN (SELECT dept FROM dept_table WHERE region = 'east')`
- 多值 IN：子查询返回多行
- 空子查询结果：IN 匹配空集 → 无行输出

### 9.2 WHERE NOT IN 子查询

- `SELECT * FROM emp WHERE dept NOT IN (SELECT ...)`
- NOT IN 含 NULL 的三值逻辑

### 9.3 WHERE EXISTS / NOT EXISTS

- `SELECT * FROM emp WHERE EXISTS (SELECT 1 FROM dept WHERE dept.id = emp.dept)`
- `SELECT * FROM emp WHERE NOT EXISTS (SELECT 1 FROM ...)`

### 9.4 相关子查询

- `SELECT * FROM emp WHERE dept IN (SELECT dept FROM dept WHERE dept.region = emp.region)`
- 相关 EXISTS：外层列引用

### 9.5 SELECT 标量子查询

- `SELECT name, (SELECT avg(salary) FROM emp) AS avg_sal FROM dept`
- 相关标量子查询：`(SELECT salary FROM emp WHERE emp.id = dept.manager_id)`

### 9.6 FROM 子查询（派生表）

- `SELECT t.dept, t.avg_sal FROM (SELECT dept, avg(salary) AS avg_sal FROM emp GROUP BY dept) AS t`
- 派生表别名与列引用

### 9.7 嵌套子查询

- 子查询中嵌套子查询
- 多层相关子查询

### 9.8 边界情况

- 子查询返回 NULL 值
- 标量子查询返回多行 → 错误
- IN 子查询返回多列 → 错误
- 空结果子查询
- 子查询与 GROUP BY/HAVING 组合

## 10. 实现顺序

建议按以下顺序实现（每个步骤 TDD）：

1. **T1**: CorrelatedParam 类型 + PlanError 新增错误类型
2. **T2**: SemiJoinNode + SemiJoinExecutor（独立子查询 IN）
3. **T3**: AntiJoinNode + AntiJoinExecutor（NOT IN / NOT EXISTS）
4. **T4**: PlanBuilder 子查询解析（IN/EXISTS → SemiJoin/AntiJoin）
5. **T5**: EXISTS 子查询支持（无条件列的 SemiJoin）
6. **T6**: 相关子查询参数传递机制
7. **T7**: 相关子查询 IN/EXISTS 支持
8. **T8**: SubqueryEvalNode + SubqueryEvalExecutor（标量子查询）
9. **T9**: DerivedScanNode + DerivedScanExecutor（FROM 子查询）
10. **T10**: Pipeline 集成 + PlanCache 子查询处理
11. **T11**: 综合测试（嵌套子查询 + 组合查询）