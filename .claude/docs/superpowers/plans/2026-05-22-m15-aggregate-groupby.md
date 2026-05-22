# M15 聚合函数与 GROUP BY 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 RTsql 实现 COUNT/SUM/AVG/MIN/MAX 聚合函数、GROUP BY 分组、HAVING 聚合结果过滤，遵循 SQL 标准语义。

**Architecture:** Volcano 迭代器模型 — 新增 AggregateNode/HavingNode 作为 PhysicalPlan 节点，各对应独立的 AggregateExecutor/HavingExecutor。AggregateExecutor 使用 HashMap 按分组键累积聚合状态，HavingExecutor 复用现有 Predicate 体系过滤聚合结果行。

**Tech Stack:** Rust, Tokio, sqlparser-rs 0.44

---

## File Structure

| 文件 | 操作 | 职责 |
|------|------|------|
| `src/executor/aggregate.rs` | 新增 | AggregateFunc 枚举、AggregateState 累积器、AggregateExecutor |
| `src/executor/having.rs` | 新增 | HavingExecutor（结构同 FilterExecutor） |
| `src/executor/plan.rs` | 修改 | 新增 AggregateNode、HavingNode 到 PhysicalPlan 枚举 |
| `src/executor/mod.rs` | 修改 | 导出 aggregate、having 模块和类型 |
| `src/parser/planner.rs` | 修改 | 聚合函数检测、GROUP BY 解析、HAVING 解析、严格模式验证 |
| `src/parser/error.rs` | 修改 | 新增 4 个错误变体 |
| `src/pipeline.rs` | 修改 | create_executor_from_plan 新增 Aggregate/Having 分支 |
| `tests/aggregate_test.rs` | 新增 | 聚合函数 + GROUP BY + HAVING 端到端测试 |

---

## Task 1: AggregateFunc + AggregateState 类型与累积逻辑

**Files:**
- Create: `src/executor/aggregate.rs`
- Modify: `src/executor/mod.rs`

- [ ] **Step 1: 创建 aggregate.rs 基础类型**

在 `src/executor/aggregate.rs` 中创建 AggregateFunc 和 AggregateState：

```rust
use crate::executor::value::Value;

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

/// 聚合累积状态
#[derive(Debug, Clone)]
pub enum AggregateState {
    CountStar(i64),
    Count { count: i64 },
    Sum { sum: Option<Value>, count: i64 },
    Avg { sum: Option<Value>, count: i64 },
    Min(Option<Value>),
    Max(Option<Value>),
}

impl AggregateState {
    /// 根据聚合函数类型创建初始累积器
    pub fn new(func: &AggregateFunc) -> Self {
        match func {
            AggregateFunc::CountStar => AggregateState::CountStar(0),
            AggregateFunc::Count(_) => AggregateState::Count { count: 0 },
            AggregateFunc::Sum(_) => AggregateState::Sum { sum: None, count: 0 },
            AggregateFunc::Avg(_) => AggregateState::Avg { sum: None, count: 0 },
            AggregateFunc::Min(_) => AggregateState::Min(None),
            AggregateFunc::Max(_) => AggregateState::Max(None),
        }
    }

    /// 逐行更新累积器
    pub fn update(&mut self, value: &Value) {
        match self {
            AggregateState::CountStar(count) => {
                *count += 1;
            }
            AggregateState::Count { count } => {
                if !matches!(value, Value::Null) {
                    *count += 1;
                }
            }
            AggregateState::Sum { sum, count } => {
                if !matches!(value, Value::Null) {
                    *count += 1;
                    match sum {
                        None => *sum = Some(value.clone()),
                        Some(current) => {
                            *current = current.add(value);
                        }
                    }
                }
            }
            AggregateState::Avg { sum, count } => {
                if !matches!(value, Value::Null) {
                    *count += 1;
                    match sum {
                        None => *sum = Some(value.clone()),
                        Some(current) => {
                            *current = current.add(value);
                        }
                    }
                }
            }
            AggregateState::Min(opt) => {
                if !matches!(value, Value::Null) {
                    match opt {
                        None => *opt = Some(value.clone()),
                        Some(current) => {
                            if value.lt(current) {
                                *current = value.clone();
                            }
                        }
                    }
                }
            }
            AggregateState::Max(opt) => {
                if !matches!(value, Value::Null) {
                    match opt {
                        None => *opt = Some(value.clone()),
                        Some(current) => {
                            if current.lt(value) {
                                *current = value.clone();
                            }
                        }
                    }
                }
            }
        }
    }

    /// 输出最终聚合结果
    pub fn finalize(&self) -> Value {
        match self {
            AggregateState::CountStar(count) => Value::Int(*count),
            AggregateState::Count { count } => Value::Int(*count),
            AggregateState::Sum { sum, count: _ } => {
                sum.clone().unwrap_or(Value::Null)
            }
            AggregateState::Avg { sum, count } => {
                match (sum, count) {
                    (Some(s), c) if *c > 0 => s.div(&Value::Int(*c)),
                    _ => Value::Null,
                }
            }
            AggregateState::Min(opt) => opt.clone().unwrap_or(Value::Null),
            AggregateState::Max(opt) => opt.clone().unwrap_or(Value::Null),
        }
    }
}

impl AggregateFunc {
    /// 获取聚合结果列名（用于 HAVING 引用和输出列命名）
    pub fn result_column_name(&self) -> String {
        match self {
            AggregateFunc::CountStar => "count_star".to_string(),
            AggregateFunc::Count(col) => format!("count_{}", col),
            AggregateFunc::Sum(col) => format!("sum_{}", col),
            AggregateFunc::Avg(col) => format!("avg_{}", col),
            AggregateFunc::Min(col) => format!("min_{}", col),
            AggregateFunc::Max(col) => format!("max_{}", col),
        }
    }
}
```

- [ ] **Step 2: 在 mod.rs 中导出**

在 `src/executor/mod.rs` 中添加：

```rust
pub mod aggregate;
pub mod having;
```

并在 pub use 区域添加：

```rust
pub use aggregate::{AggregateFunc, AggregateState};
```

- [ ] **Step 3: 检查 Value 是否已有 add/lt/div 方法，如缺失则添加**

读取 `src/executor/value.rs`，检查 Value 是否有算术运算方法。如没有 `add`、`lt`、`div` 方法，需要添加：

```rust
impl Value {
    pub fn add(&self, other: &Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
            (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 + b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a + *b as f64),
            _ => Value::Null,
        }
    }

    pub fn lt(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a < b,
            (Value::Float(a), Value::Float(b)) => a < b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) < *b,
            (Value::Float(a), Value::Int(b)) => *a < (*b as f64),
            (Value::String(a), Value::String(b)) => a < b,
            _ => false,
        }
    }

    pub fn div(&self, other: &Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) if *b != 0 => Value::Int(a / b),
            (Value::Float(a), Value::Float(b)) if *b != 0.0 => Value::Float(a / b),
            (Value::Int(a), Value::Float(b)) if *b != 0.0 => Value::Float(*a as f64 / b),
            (Value::Float(a), Value::Int(b)) if *b != 0 => Value::Float(a / *b as f64),
            _ => Value::Null,
        }
    }
}
```

- [ ] **Step 4: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过，无错误

- [ ] **Step 5: Commit**

```bash
git add src/executor/aggregate.rs src/executor/value.rs src/executor/mod.rs
git commit -m "feat(M15): add AggregateFunc, AggregateState types and Value arithmetic methods"
```

---

## Task 2: PhysicalPlan 新增 AggregateNode 和 HavingNode

**Files:**
- Modify: `src/executor/plan.rs`

- [ ] **Step 1: 在 PhysicalPlan 枚举中新增两个变体**

在 `src/executor/plan.rs` 中，PhysicalPlan 枚举新增：

```rust
use crate::executor::aggregate::AggregateFunc;
use crate::executor::predicate::PredicateRef;

#[derive(Debug, Clone)]
pub struct AggregateNode {
    pub input: Box<PhysicalPlan>,
    pub group_by: Vec<String>,
    pub aggregates: Vec<AggregateFunc>,
    pub output_columns: Vec<String>,
    pub table_name: String,
}

#[derive(Debug, Clone)]
pub struct HavingNode {
    pub input: Box<PhysicalPlan>,
    pub predicate: PredicateRef,
    pub table_name: String,
}
```

在 PhysicalPlan 枚举中添加：

```rust
pub enum PhysicalPlan {
    // ... 现有变体 ...
    Aggregate(AggregateNode),
    Having(HavingNode),
}
```

- [ ] **Step 2: 更新 PhysicalPlan 的 schema() 和 table_name() 方法**

在 `schema()` 和 `table_name()` 方法中新增 Aggregate 和 Having 分支：

```rust
PhysicalPlan::Aggregate(node) => node.output_columns.clone(),
PhysicalPlan::Having(node) => node.input.schema(),
```

```rust
PhysicalPlan::Aggregate(node) => node.table_name.clone(),
PhysicalPlan::Having(node) => node.table_name.clone(),
```

- [ ] **Step 3: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过（注意 Pipeline 中的 match 可能报 missing variant，暂用 `_ => unimplemented!()` 占位）

- [ ] **Step 4: Commit**

```bash
git add src/executor/plan.rs
git commit -m "feat(M15): add AggregateNode and HavingNode to PhysicalPlan"
```

---

## Task 3: PlanBuilder 聚合函数检测与解析

**Files:**
- Modify: `src/parser/planner.rs`
- Modify: `src/parser/error.rs`

这是最复杂的 Task。Planner 需要从 sqlparser AST 中提取聚合函数和 GROUP BY 信息。

- [ ] **Step 1: 在 PlanError 中新增错误变体**

在 `src/parser/error.rs` 中添加：

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

- [ ] **Step 2: 添加聚合函数检测辅助函数**

在 `src/parser/planner.rs` 中添加：

```rust
/// 检查 Expr 是否为聚合函数表达式
fn is_aggregate_expr(expr: &sqlparser::ast::Expr) -> bool {
    match expr {
        sqlparser::ast::Expr::Function(f) => {
            let name = f.name.to_string().to_uppercase();
            matches!(name.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
        }
        _ => false,
    }
}

/// 从 Expr 中提取 AggregateFunc
fn extract_aggregate_func(expr: &sqlparser::ast::Expr) -> Result<Option<AggregateFunc>, PlanError> {
    match expr {
        sqlparser::ast::Expr::Function(f) => {
            let name = f.name.to_string().to_uppercase();
            match name.as_str() {
                "COUNT" => {
                    if f.args.is_empty() {
                        return Err(PlanError::InvalidAggregateArgument(
                            "COUNT requires argument or *".to_string(),
                        ));
                    }
                    match &f.args[0] {
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Wildcard,
                        ) => Ok(Some(AggregateFunc::CountStar)),
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(inner),
                        ) => {
                            let col = expr_to_column_name(inner)?;
                            Ok(Some(AggregateFunc::Count(col)))
                        }
                        _ => Err(PlanError::InvalidAggregateArgument(
                            "COUNT argument must be * or column".to_string(),
                        )),
                    }
                }
                "SUM" => {
                    let col = extract_single_column_arg(&f.args, "SUM")?;
                    Ok(Some(AggregateFunc::Sum(col)))
                }
                "AVG" => {
                    let col = extract_single_column_arg(&f.args, "AVG")?;
                    Ok(Some(AggregateFunc::Avg(col)))
                }
                "MIN" => {
                    let col = extract_single_column_arg(&f.args, "MIN")?;
                    Ok(Some(AggregateFunc::Min(col)))
                }
                "MAX" => {
                    let col = extract_single_column_arg(&f.args, "MAX")?;
                    Ok(Some(AggregateFunc::Max(col)))
                }
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// 从函数参数中提取单个列名
fn extract_single_column_arg(
    args: &[sqlparser::ast::FunctionArg],
    func_name: &str,
) -> Result<String, PlanError> {
    if args.len() != 1 {
        return Err(PlanError::InvalidAggregateArgument(format!(
            "{} requires exactly one argument",
            func_name
        )));
    }
    match &args[0] {
        sqlparser::ast::FunctionArg::Unnamed(
            sqlparser::ast::FunctionArgExpr::Expr(expr),
        ) => expr_to_column_name(expr),
        _ => Err(PlanError::InvalidAggregateArgument(format!(
            "{} argument must be a column",
            func_name
        ))),
    }
}

/// 从 Expr 中提取列名（仅支持 Identifier 和 CompoundIdentifier）
fn expr_to_column_name(expr: &sqlparser::ast::Expr) -> Result<String, PlanError> {
    match expr {
        sqlparser::ast::Expr::Identifier(ident) => Ok(ident.value.clone()),
        sqlparser::ast::Expr::CompoundIdentifier(parts) => {
            // 取最后一部分作为列名
            Ok(parts.last().unwrap().value.clone())
        }
        _ => Err(PlanError::InvalidAggregateArgument(
            "Expected column name".to_string(),
        )),
    }
}
```

- [ ] **Step 3: 添加 SELECT 投影列解析方法**

在 Planner 中添加方法，从 SELECT 投影中提取聚合函数和非聚合列：

```rust
/// 解析 SELECT 投影列，分离聚合函数和非聚合列
fn parse_select_projection(
    &self,
    projection: &[sqlparser::ast::SelectItem],
) -> Result<(Vec<AggregateFunc>, Vec<String>, Vec<String>), PlanError> {
    let mut aggregates = Vec::new();
    let mut non_agg_columns = Vec::new();
    let mut output_columns = Vec::new();

    for item in projection {
        match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                if is_aggregate_expr(expr) {
                    let func = extract_aggregate_func(expr)?
                        .ok_or_else(|| PlanError::InvalidAggregateArgument(
                            "Unknown aggregate function".to_string(),
                        ))?;
                    output_columns.push(func.result_column_name());
                    aggregates.push(func);
                } else {
                    let col = expr_to_column_name(expr)?;
                    non_agg_columns.push(col.clone());
                    output_columns.push(col);
                }
            }
            sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => {
                if is_aggregate_expr(expr) {
                    let func = extract_aggregate_func(expr)?
                        .ok_or_else(|| PlanError::InvalidAggregateArgument(
                            "Unknown aggregate function".to_string(),
                        ))?;
                    output_columns.push(alias.value.clone());
                    aggregates.push(func);
                } else {
                    let col = expr_to_column_name(expr)?;
                    non_agg_columns.push(col.clone());
                    output_columns.push(alias.value.clone());
                }
            }
            sqlparser::ast::SelectItem::Wildcard => {
                // SELECT * 不与聚合函数共存
                return Err(PlanError::InvalidAggregateArgument(
                    "SELECT * cannot be used with aggregate functions".to_string(),
                ));
            }
            _ => {
                return Err(PlanError::InvalidAggregateArgument(
                    "Unsupported select item in aggregate query".to_string(),
                ));
            }
        }
    }

    Ok((aggregates, non_agg_columns, output_columns))
}
```

- [ ] **Step 4: 修改 build_query 方法，在构建投影后检测聚合**

在 `build_query` 方法中，现有投影构建逻辑之后，添加聚合检测和处理：

在现有 `build_query` 方法中，构建投影（Projection）的逻辑后，新增聚合处理分支。关键伪代码位置：

```
现有流程：build_from → build_where → build_order → build_limit
新流程：build_from → build_where → [聚合检测+AggregateNode+HavingNode] → build_order → build_limit
```

具体修改：在 WHERE 构建之后、ORDER BY 之前，插入聚合检测逻辑：

```rust
// === 聚合函数检测 ===
let (aggregates, non_agg_columns, output_columns) =
    self.parse_select_projection(&select.projection)?;

let has_aggregates = !aggregates.is_empty();

if has_aggregates {
    // 提取 GROUP BY 列名
    let group_by: Vec<String> = select
        .group_by
        .iter()
        .map(|expr| expr_to_column_name(expr))
        .collect::<Result<Vec<_>, _>>()?;

    // 严格模式验证：非聚合列必须出现在 GROUP BY 中
    for col in &non_agg_columns {
        if !group_by.contains(col) {
            return Err(PlanError::NonAggregatedColumn(col.clone()));
        }
    }

    // 构建 AggregateNode
    plan = PhysicalPlan::Aggregate(AggregateNode {
        input: Box::new(plan),
        group_by,
        aggregates,
        output_columns,
        table_name: table_name.clone(),
    });

    // 构建 HAVING
    if let Some(having_expr) = &select.having {
        let having_pred = self.build_expression(having_expr, &table_name)?;
        plan = PhysicalPlan::Having(HavingNode {
            input: Box::new(plan),
            predicate: having_pred,
            table_name: table_name.clone(),
        });
    }
}
```

注意：现有 `build_query` 中的投影逻辑（Projection 节点）在聚合查询中需要跳过，因为 AggregateNode 已包含 output_columns。

- [ ] **Step 5: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过（可能需要调整 build_query 中投影逻辑与聚合逻辑的互斥处理）

- [ ] **Step 6: Commit**

```bash
git add src/parser/planner.rs src/parser/error.rs
git commit -m "feat(M15): add aggregate detection, GROUP BY/HAVING parsing in PlanBuilder"
```

---

## Task 4: AggregateExecutor 实现

**Files:**
- Modify: `src/executor/aggregate.rs`

- [ ] **Step 1: 实现 AggregateExecutor**

在 `src/executor/aggregate.rs` 中添加 Executor 实现：

```rust
use crate::executor::executor_trait::Executor;
use crate::executor::plan::AggregateNode;
use crate::executor::result::ExecutionResult;
use crate::executor::value::Value;
use async_trait::async_trait;
use std::collections::HashMap;

pub struct AggregateExecutor {
    input: Box<dyn Executor + Send>,
    group_by: Vec<String>,
    aggregates: Vec<AggregateFunc>,
    output_columns: Vec<String>,
    /// 分组结果：分组键 → 聚合状态列表
    groups: HashMap<Vec<Value>, Vec<AggregateState>>,
    /// 无 GROUP BY 时的单组状态
    single_group: Option<Vec<AggregateState>>,
    /// 是否已消耗所有输入
    has_consumed_input: bool,
    /// 输出迭代位置
    output_rows: Option<Vec<Vec<Value>>>,
    output_index: usize,
}

impl AggregateExecutor {
    pub fn new(input: Box<dyn Executor + Send>, node: AggregateNode) -> Self {
        let aggregate_count = node.aggregates.len();
        Self {
            input,
            group_by: node.group_by,
            aggregates: node.aggregates,
            output_columns: node.output_columns,
            groups: HashMap::new(),
            single_group: None,
            has_consumed_input: false,
            output_rows: None,
            output_index: 0,
        }
    }

    /// 消耗所有输入行，按分组键累积聚合状态
    async fn consume_input(&mut self) -> Result<(), ExecutionResult> {
        let is_no_group_by = self.group_by.is_empty();

        if is_no_group_by {
            let mut states: Vec<AggregateState> = self
                .aggregates
                .iter()
                .map(|f| AggregateState::new(f))
                .collect();
            self.single_group = Some(states);
        }

        loop {
            match self.input.next().await {
                Ok(row) => {
                    if is_no_group_by {
                        // 无 GROUP BY：所有行归入单组
                        let states = self.single_group.as_mut().unwrap();
                        for (i, func) in self.aggregates.iter().enumerate() {
                            let value = Self::extract_value(&row, func);
                            states[i].update(&value);
                        }
                    } else {
                        // 有 GROUP BY：提取分组键
                        // 从行中提取 group_by 列的值作为分组键
                        // 需要列索引映射 — 暂用列名匹配
                        let group_key = self.extract_group_key(&row);

                        let states = self.groups.entry(group_key).or_insert_with(|| {
                            self.aggregates.iter().map(|f| AggregateState::new(f)).collect()
                        });

                        for (i, func) in self.aggregates.iter().enumerate() {
                            let value = Self::extract_value(&row, func);
                            states[i].update(&value);
                        }
                    }
                }
                Err(ExecutionResult::EndOfRows) => break,
                Err(e) => return Err(e),
            }
        }

        self.has_consumed_input = true;
        Ok(())
    }

    /// 从行中提取聚合函数所需的值
    fn extract_value(row: &[Value], func: &AggregateFunc) -> Value {
        match func {
            AggregateFunc::CountStar => Value::Int(1), // COUNT(*) 不需要具体值，只需触发 update
            AggregateFunc::Count(col)
            | AggregateFunc::Sum(col)
            | AggregateFunc::Avg(col)
            | AggregateFunc::Min(col)
            | AggregateFunc::Max(col) => {
                // 从行中按列名查找 — 但当前行是 Vec<Value>，没有列名映射
                // 解决方案：在 AggregateNode 中存储列索引映射
                // 暂时返回 Null 占位，在 Task 5 集成时解决
                Value::Null
            }
        }
    }

    /// 从行中提取分组键
    fn extract_group_key(&self, row: &[Value]) -> Vec<Value> {
        // 需要列索引映射 — 在 Task 5 集成时实现
        Vec::new()
    }

    /// 将分组结果转换为输出行
    fn build_output_rows(&mut self) {
        let mut rows = Vec::new();

        if self.group_by.is_empty() {
            // 无 GROUP BY：返回单行
            if let Some(states) = &self.single_group {
                let mut row = Vec::new();
                for state in states {
                    row.push(state.finalize());
                }
                rows.push(row);
            } else {
                // 空输入：返回单行（COUNT→0，其他→NULL）
                let row: Vec<Value> = self
                    .aggregates
                    .iter()
                    .map(|f| {
                        let state = AggregateState::new(f);
                        state.finalize()
                    })
                    .collect();
                rows.push(row);
            }
        } else {
            // 有 GROUP BY：每组一行
            for (group_key, states) in &self.groups {
                let mut row = group_key.clone();
                for state in states {
                    row.push(state.finalize());
                }
                rows.push(row);
            }
        }

        self.output_rows = Some(rows);
    }
}

#[async_trait]
impl Executor for AggregateExecutor {
    async fn next(&mut self) -> Result<Vec<Value>, ExecutionResult> {
        if !self.has_consumed_input {
            self.consume_input().await?;
            self.build_output_rows();
        }

        match &self.output_rows {
            Some(rows) => {
                if self.output_index < rows.len() {
                    let row = rows[self.output_index].clone();
                    self.output_index += 1;
                    Ok(row)
                } else {
                    Err(ExecutionResult::EndOfRows)
                }
            }
            None => Err(ExecutionResult::EndOfRows),
        }
    }
}
```

注意：AggregateExecutor 和 HavingExecutor 必须遵循现有 Executor trait 签名：

```rust
#[async_trait]
impl Executor for AggregateExecutor {
    async fn next(&mut self) -> crate::storage::Result<Option<ExecResult>> {
        if !self.has_consumed_input {
            self.consume_input().await?;
            self.build_output_rows();
        }

        match &self.output_rows {
            Some(rows) => {
                if self.output_index < rows.len() {
                    let row = rows[self.output_index].clone();
                    self.output_index += 1;
                    Ok(Some(ExecResult::Row(row)))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}
```

`consume_input` 方法签名应返回 `crate::storage::Result<()>`，内部循环使用 `self.input.next().await` 并匹配 `Ok(Some(ExecResult::Row(row)))` 累积、`Ok(None)` 结束、`Err(e)` 透传。

- [ ] **Step 2: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
git add src/executor/aggregate.rs
git commit -m "feat(M15): implement AggregateExecutor with hash aggregation"
```

---

## Task 5: 列索引映射 — 解决聚合值和分组键提取

**Files:**
- Modify: `src/executor/aggregate.rs`
- Modify: `src/executor/plan.rs`
- Modify: `src/parser/planner.rs`

这个 Task 解决 Task 4 中遗留的列索引问题。AggregateNode 需要知道每列在输入行中的位置。

- [ ] **Step 1: 在 AggregateNode 中添加列索引映射**

修改 `src/executor/plan.rs` 中 AggregateNode：

```rust
#[derive(Debug, Clone)]
pub struct AggregateNode {
    pub input: Box<PhysicalPlan>,
    pub group_by: Vec<String>,
    pub aggregates: Vec<AggregateFunc>,
    pub output_columns: Vec<String>,
    pub table_name: String,
    /// 列名 → 在输入行中的索引
    pub column_indices: std::collections::HashMap<String, usize>,
}
```

- [ ] **Step 2: 在 Planner 中构建列索引映射**

在 `src/parser/planner.rs` 的聚合处理逻辑中，构建 AggregateNode 时，需要从输入计划的 schema 中建立列索引映射：

```rust
// 构建列索引映射：从输入计划的 schema 中获取列名到索引的映射
let input_schema = plan.schema();
let column_indices: HashMap<String, usize> = input_schema
    .iter()
    .enumerate()
    .map(|(i, col)| (col.clone(), i))
    .collect();

plan = PhysicalPlan::Aggregate(AggregateNode {
    input: Box::new(plan),
    group_by,
    aggregates,
    output_columns,
    table_name: table_name.clone(),
    column_indices,
});
```

- [ ] **Step 3: 更新 AggregateExecutor 使用列索引**

在 `src/executor/aggregate.rs` 中：

更新 AggregateExecutor 结构体，添加 `column_indices` 字段：

```rust
pub struct AggregateExecutor {
    // ... 现有字段 ...
    column_indices: HashMap<String, usize>,
}
```

更新 `new()` 方法接收 column_indices。

替换 `extract_value` 和 `extract_group_key`：

```rust
fn extract_value(&self, row: &[Value], func: &AggregateFunc) -> Value {
    match func {
        AggregateFunc::CountStar => Value::Int(1),
        AggregateFunc::Count(col)
        | AggregateFunc::Sum(col)
        | AggregateFunc::Avg(col)
        | AggregateFunc::Min(col)
        | AggregateFunc::Max(col) => {
            match self.column_indices.get(col) {
                Some(&idx) => row.get(idx).cloned().unwrap_or(Value::Null),
                None => Value::Null,
            }
        }
    }
}

fn extract_group_key(&self, row: &[Value]) -> Vec<Value> {
    self.group_by
        .iter()
        .map(|col| {
            match self.column_indices.get(col) {
                Some(&idx) => row.get(idx).cloned().unwrap_or(Value::Null),
                None => Value::Null,
            }
        })
        .collect()
}
```

- [ ] **Step 4: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过

- [ ] **Step 5: Commit**

```bash
git add src/executor/aggregate.rs src/executor/plan.rs src/parser/planner.rs
git commit -m "feat(M15): add column index mapping for aggregate value and group key extraction"
```

---

## Task 6: HavingExecutor 实现

**Files:**
- Create: `src/executor/having.rs`

- [ ] **Step 1: 实现 HavingExecutor**

```rust
use crate::executor::executor_trait::Executor;
use crate::executor::predicate::PredicateRef;
use crate::executor::result::ExecResult;
use crate::executor::value::Value;
use crate::storage;

pub struct HavingExecutor {
    input: Box<dyn Executor + Send>,
    predicate: PredicateRef,
}

impl HavingExecutor {
    pub fn new(input: Box<dyn Executor + Send>, predicate: PredicateRef) -> Self {
        Self { input, predicate }
    }
}

#[async_trait]
impl Executor for HavingExecutor {
    async fn next(&mut self) -> storage::Result<Option<ExecResult>> {
        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(row)) => {
                    if self.predicate.evaluate(&row) {
                        return Ok(Some(ExecResult::Row(row)));
                    }
                    // 不满足 HAVING 条件的行，跳过
                }
                Some(other) => return Ok(Some(other)),
                None => return Ok(None),
            }
        }
    }
}
```

- [ ] **Step 2: 在 mod.rs 中导出**

确认 `src/executor/mod.rs` 已导出 having 模块：

```rust
pub mod having;
```

- [ ] **Step 3: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add src/executor/having.rs src/executor/mod.rs
git commit -m "feat(M15): implement HavingExecutor"
```

---

## Task 7: Pipeline 整合

**Files:**
- Modify: `src/pipeline.rs`

- [ ] **Step 1: 在 create_executor_from_plan 中添加 Aggregate 和 Having 分支**

在 `src/pipeline.rs` 的 `create_executor_from_plan` 函数中，PhysicalPlan 的 match 新增：

```rust
PhysicalPlan::Aggregate(node) => {
    let column_indices = node.column_indices.clone();
    let input = create_executor_from_plan(*node.input, database).await?;
    Ok(Box::new(
        AggregateExecutor::new_with_indices(input, node)
    ) as Box<dyn Executor + Send>)
}

PhysicalPlan::Having(node) => {
    let input = create_executor_from_plan(*node.input, database).await?;
    Ok(Box::new(HavingExecutor::new(input, node.predicate)) as Box<dyn Executor + Send>)
}
```

- [ ] **Step 2: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译通过

- [ ] **Step 3: 运行现有测试确保无回归**

Run: `cargo test 2>&1 | tail -20`
Expected: 所有现有测试通过

- [ ] **Step 4: Commit**

```bash
git add src/pipeline.rs
git commit -m "feat(M15): integrate Aggregate and Having executors in Pipeline"
```

---

## Task 8: 聚合函数端到端测试 — 基本聚合（无 GROUP BY）

**Files:**
- Create: `tests/aggregate_test.rs`

- [ ] **Step 1: 编写基本聚合测试**

```rust
use rtsql::database::Database;
use tempfile::NamedTempFile;

async fn setup_db() -> Database {
    let file = NamedTempFile::new().unwrap();
    let db = Database::open(file.path().to_str().unwrap()).await.unwrap();
    db.execute("CREATE TABLE scores (name TEXT, score INT)", &[]).await.unwrap();
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', 90)", &[]).await.unwrap();
    db.execute("INSERT INTO scores (name, score) VALUES ('Bob', 80)", &[]).await.unwrap();
    db.execute("INSERT INTO scores (name, score) VALUES ('Charlie', 70)", &[]).await.unwrap();
    db
}

#[tokio::test]
async fn test_count_star() {
    let db = setup_db().await;
    let result = db.query("SELECT COUNT(*) FROM scores", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(3));
}

#[tokio::test]
async fn test_count_column() {
    let db = setup_db().await;
    let result = db.query("SELECT COUNT(score) FROM scores", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(3));
}

#[tokio::test]
async fn test_count_column_with_null() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Dave', NULL)", &[]).await.unwrap();
    let result = db.query("SELECT COUNT(score) FROM scores", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(3)); // 跳过 NULL
}

#[tokio::test]
async fn test_sum() {
    let db = setup_db().await;
    let result = db.query("SELECT SUM(score) FROM scores", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(240));
}

#[tokio::test]
async fn test_avg() {
    let db = setup_db().await;
    let result = db.query("SELECT AVG(score) FROM scores", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    // AVG(90, 80, 70) = 80
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(80));
}

#[tokio::test]
async fn test_min_max() {
    let db = setup_db().await;
    let result = db.query("SELECT MIN(score), MAX(score) FROM scores", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(70));
    assert_eq!(result[0][1], rtsql::executor::value::Value::Int(90));
}

#[tokio::test]
async fn test_empty_table_count() {
    let db = setup_db().await;
    db.execute("CREATE TABLE empty (val INT)", &[]).await.unwrap();
    let result = db.query("SELECT COUNT(*) FROM empty", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(0));
}

#[tokio::test]
async fn test_empty_table_sum() {
    let db = setup_db().await;
    db.execute("CREATE TABLE empty (val INT)", &[]).await.unwrap();
    let result = db.query("SELECT SUM(val) FROM empty", &[]).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Null);
}
```

- [ ] **Step 2: 运行测试验证失败（RED）**

Run: `cargo test --test aggregate_test 2>&1 | tail -20`
Expected: 测试编译失败或运行失败（聚合查询尚不可用）

- [ ] **Step 3: 调试修复直到测试通过（GREEN）**

根据实际错误调整代码，可能需要修复：
- Planner 中 SELECT 投影与聚合逻辑的交互
- AggregateExecutor 中行格式与查询接口的匹配
- 列名解析和索引映射的正确性

Run: `cargo test --test aggregate_test 2>&1 | tail -20`
Expected: 所有基本聚合测试通过

- [ ] **Step 4: Commit**

```bash
git add tests/aggregate_test.rs
git commit -m "test(M15): add basic aggregate function tests (no GROUP BY)"
```

---

## Task 9: GROUP BY 测试

**Files:**
- Modify: `tests/aggregate_test.rs`

- [ ] **Step 1: 编写 GROUP BY 测试**

在 `tests/aggregate_test.rs` 中追加：

```rust
#[tokio::test]
async fn test_group_by_single_column() {
    let db = setup_db().await;
    let result = db.query(
        "SELECT name, COUNT(*) FROM scores GROUP BY name",
        &[],
    ).await.unwrap();
    // 3 个不同名字，每组 1 行
    assert_eq!(result.len(), 3);
}

#[tokio::test]
async fn test_group_by_with_sum() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', 95)", &[]).await.unwrap();
    let result = db.query(
        "SELECT name, SUM(score) FROM scores GROUP BY name",
        &[],
    ).await.unwrap();
    // Alice 组 SUM = 90 + 95 = 185
    let alice_row = result.iter().find(|r| {
        matches!(&r[0], rtsql::executor::value::Value::String(s) if s == "Alice")
    });
    assert!(alice_row.is_some());
    assert_eq!(alice_row.unwrap()[1], rtsql::executor::value::Value::Int(185));
}

#[tokio::test]
async fn test_group_by_with_avg() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', 95)", &[]).await.unwrap();
    let result = db.query(
        "SELECT name, AVG(score) FROM scores GROUP BY name",
        &[],
    ).await.unwrap();
    // Alice 组 AVG = (90 + 95) / 2 = 92
    let alice_row = result.iter().find(|r| {
        matches!(&r[0], rtsql::executor::value::Value::String(s) if s == "Alice")
    });
    assert!(alice_row.is_some());
    assert_eq!(alice_row.unwrap()[1], rtsql::executor::value::Value::Int(92));
}

#[tokio::test]
async fn test_strict_mode_non_aggregated_column() {
    let db = setup_db().await;
    let result = db.query(
        "SELECT name, score FROM scores GROUP BY name",
        &[],
    ).await;
    // score 不在 GROUP BY 中，应报错
    assert!(result.is_err());
}

#[tokio::test]
async fn test_group_by_with_null() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', NULL)", &[]).await.unwrap();
    let result = db.query(
        "SELECT name, COUNT(score), SUM(score) FROM scores GROUP BY name",
        &[],
    ).await.unwrap();
    let alice_row = result.iter().find(|r| {
        matches!(&r[0], rtsql::executor::value::Value::String(s) if s == "Alice")
    });
    assert!(alice_row.is_some());
    // Alice 有 1 个非 NULL score (90) 和 1 个 NULL
    assert_eq!(alice_row.unwrap()[1], rtsql::executor::value::Value::Int(1)); // COUNT(score)
    assert_eq!(alice_row.unwrap()[2], rtsql::executor::value::Value::Int(90)); // SUM(score)
}
```

- [ ] **Step 2: 运行测试验证（GREEN）**

Run: `cargo test --test aggregate_test 2>&1 | tail -20`
Expected: 所有 GROUP BY 测试通过

- [ ] **Step 3: Commit**

```bash
git add tests/aggregate_test.rs
git commit -m "test(M15): add GROUP BY tests with strict mode and NULL handling"
```

---

## Task 10: HAVING 测试

**Files:**
- Modify: `tests/aggregate_test.rs`

- [ ] **Step 1: 编写 HAVING 测试**

```rust
#[tokio::test]
async fn test_having_basic() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', 95)", &[]).await.unwrap();
    let result = db.query(
        "SELECT name, COUNT(*) FROM scores GROUP BY name HAVING COUNT(*) > 1",
        &[],
    ).await.unwrap();
    // 只有 Alice 有 2 行，其他只有 1 行
    assert_eq!(result.len(), 1);
}

#[tokio::test]
async fn test_having_with_sum() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', 95)", &[]).await.unwrap();
    let result = db.query(
        "SELECT name, SUM(score) FROM scores GROUP BY name HAVING SUM(score) > 100",
        &[],
    ).await.unwrap();
    // Alice SUM = 185 > 100，其他 < 100
    assert_eq!(result.len(), 1);
}

#[tokio::test]
async fn test_having_filters_all() {
    let db = setup_db().await;
    let result = db.query(
        "SELECT name, COUNT(*) FROM scores GROUP BY name HAVING COUNT(*) > 100",
        &[],
    ).await.unwrap();
    // 没有组满足条件
    assert_eq!(result.len(), 0);
}
```

- [ ] **Step 2: 运行测试验证（GREEN）**

Run: `cargo test --test aggregate_test 2>&1 | tail -20`
Expected: 所有 HAVING 测试通过

- [ ] **Step 3: Commit**

```bash
git add tests/aggregate_test.rs
git commit -m "test(M15): add HAVING filter tests"
```

---

## Task 11: 组合查询测试 + 回归验证

**Files:**
- Modify: `tests/aggregate_test.rs`

- [ ] **Step 1: 编写组合查询测试**

```rust
#[tokio::test]
async fn test_aggregate_with_where() {
    let db = setup_db().await;
    let result = db.query(
        "SELECT COUNT(*) FROM scores WHERE score > 75",
        &[],
    ).await.unwrap();
    // Alice(90) 和 Bob(80) 满足条件
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(2));
}

#[tokio::test]
async fn test_group_by_with_where() {
    let db = setup_db().await;
    db.execute("INSERT INTO scores (name, score) VALUES ('Alice', 95)", &[]).await.unwrap();
    let result = db.query(
        "SELECT name, COUNT(*) FROM scores WHERE score > 75 GROUP BY name",
        &[],
    ).await.unwrap();
    // Alice(90,95) 和 Bob(80) 满足条件
    assert_eq!(result.len(), 2);
}

#[tokio::test]
async fn test_multiple_aggregates() {
    let db = setup_db().await;
    let result = db.query(
        "SELECT COUNT(*), SUM(score), AVG(score), MIN(score), MAX(score) FROM scores",
        &[],
    ).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0][0], rtsql::executor::value::Value::Int(3));
    assert_eq!(result[0][1], rtsql::executor::value::Value::Int(240));
    assert_eq!(result[0][2], rtsql::executor::value::Value::Int(80));
    assert_eq!(result[0][3], rtsql::executor::value::Value::Int(70));
    assert_eq!(result[0][4], rtsql::executor::value::Value::Int(90));
}
```

- [ ] **Step 2: 运行全部测试确保无回归**

Run: `cargo test 2>&1 | tail -30`
Expected: 所有测试通过（包括现有测试和新增聚合测试）

- [ ] **Step 3: 运行 clippy**

Run: `cargo clippy 2>&1 | tail -20`
Expected: 无新 warning

- [ ] **Step 4: Commit**

```bash
git add tests/aggregate_test.rs
git commit -m "test(M15): add combination query tests and verify no regressions"
```

---

## Task 12: 项目文档更新

**Files:**
- Modify: `.claude/docs/tasks.md`
- Modify: `.claude/docs/snapshot.md`
- Modify: `.claude/docs/learned.md`

- [ ] **Step 1: 更新 tasks.md**

将 M15 标记为完成，更新当前状态。

- [ ] **Step 2: 更新 snapshot.md**

更新项目状态、最近修改、测试数量。

- [ ] **Step 3: 更新 learned.md**

记录实现过程中发现的关键知识（API 路径、sqlparser 聚合函数解析技巧、踩坑经验等）。

- [ ] **Step 4: Commit**

```bash
git add .claude/docs/
git commit -m "docs(M15): update project docs with aggregate/GROUP BY completion"
```
