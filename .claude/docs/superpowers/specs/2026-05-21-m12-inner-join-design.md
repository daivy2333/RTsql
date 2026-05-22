# M12: INNER JOIN 多表查询设计文档

> 创建日期：2026-05-21
> 状态：设计完成，待实现

---

## 1. 需求概述

实现 INNER JOIN 多表查询能力，支持链式连接和标准 ON 子句语法。

### 1.1 功能范围

| 功能项 | 选择 |
|--------|------|
| JOIN 类型 | 仅 INNER JOIN |
| JOIN 条件 | ON 子句（显式） |
| ON 复杂度 | AND 组合等值条件 |
| 表数量 | 多表链式 JOIN（A JOIN B JOIN C） |
| 列名格式 | 支持 t.col 格式 |
| JOIN 算法 | 哈希连接（万行级别数据量） |

### 1.2 不支持的功能

- LEFT/RIGHT/FULL OUTER JOIN
- WHERE 隐式连接（逗号分隔表）
- ON 子句中的非等值条件（>、<、LIKE 等）
- 子查询 JOIN
- CROSS JOIN

---

## 2. 架构设计

### 2.1 新增物理计划节点

**PhysicalPlan::Join(JoinNode)**

```rust
pub struct JoinNode {
    pub left: Box<PhysicalPlan>,   // 左表计划（可以是 Scan 或另一个 Join）
    pub right: Box<PhysicalPlan>,  // 右表计划（必须是 Scan）
    pub conditions: Vec<JoinCondition>, // ON 等值条件列表
    pub output_columns: Vec<OutputColumn>, // 输出列映射
}

pub struct JoinCondition {
    pub left_column: ColumnRef,  // 左表列引用
    pub right_column: ColumnRef, // 右表列引用
}

pub struct ColumnRef {
    pub table: Option<String>,   // 表名（可选，t.col 格式时为 Some）
    pub column: String,          // 列名
}

pub struct OutputColumn {
    pub table: Option<String>,   // 表名（可选）
    pub column: String,          // 列名
    pub table_alias: String,     // 实际表名（解析后确定）
    pub column_index: usize,     // 在源表中的列索引
}
```

### 2.2 链式 JOIN 实现

链式 JOIN 通过递归 Join 节点实现：

```
SELECT * FROM A JOIN B ON a1=b1 JOIN C ON b2=c2

PhysicalPlan 结构：
Join(
    left: Join(
        left: Scan(A),
        right: Scan(B),
        conditions: [(A.a1, B.b1)]
    ),
    right: Scan(C),
    conditions: [(B.b2, C.c2)]
)
```

### 2.3 模块文件结构

```
src/executor/
├── join.rs              # 新增：JoinExecutor 实现
├── plan.rs              # 扩展：JoinNode + ColumnRef + OutputColumn
├── mod.rs               # 扩展：导出 JoinExecutor

src/parser/
├── planner.rs           # 扩展：build_from_clause + extract_join_conditions
├── ast.rs               # 扩展：extract_join_table_name 辅助函数
├── error.rs             # 扩展：新增 JOIN 相关错误类型
```

---

## 3. 解析层设计

### 3.1 PlanBuilder 扩展

**build_query 重构**：

```rust
fn build_query(&self, query: &Query) -> Result<PhysicalPlan, PlanError> {
    let select = extract_select_body(query)?;

    // 处理 FROM + JOIN 链
    let base_plan = self.build_from_clause(&select.from)?;

    // 处理 WHERE（如果存在）
    let plan_with_where = if let Some(selection) = &select.selection {
        // 需要知道所有可用表名（从 JOIN 链提取）
        let available_tables = self.extract_available_tables(&select.from)?;
        let predicate = self.build_where_multi_table(&available_tables, selection)?;
        PhysicalPlan::Filter(FilterNode {
            input: Box::new(base_plan),
            predicate,
            table_name: "join_result".to_string(), // 虚拟表名
        })
    } else {
        base_plan
    };

    // 处理 ORDER BY + LIMIT（不变）
    ...
}
```

**build_from_clause 新增**：

```rust
fn build_from_clause(&self, from: &[TableWithJoins]) -> Result<PhysicalPlan, PlanError> {
    if from.is_empty() {
        return Err(PlanError::MissingField("FROM clause".into()));
    }

    // 基础表
    let base_table = extract_table_name(&from[0].relation)?;
    self.validate_table(&base_table)?;
    let base_columns = self.tables.get(&base_table).unwrap().clone();
    let base_plan = PhysicalPlan::Scan(ScanNode {
        table_name: base_table.clone(),
        columns: base_columns,
    });

    // 递归处理 JOIN 链
    let mut current_plan = base_plan;
    let mut current_tables = vec![base_table];

    for join in &from[0].joins {
        // 验证 JOIN 类型（仅支持 INNER）
        if join.join_type != JoinType::Inner {
            return Err(PlanError::UnsupportedJoinType);
        }

        // 解析右表
        let right_table = extract_join_table_name(&join.relation)?;
        self.validate_table(&right_table)?;
        let right_columns = self.tables.get(&right_table).unwrap().clone();
        let right_plan = PhysicalPlan::Scan(ScanNode {
            table_name: right_table.clone(),
            columns: right_columns,
        });

        // 解析 ON 条件
        let on_clause = join.constraint.on.as_ref()
            .ok_or(PlanError::MissingOnClause)?;
        let conditions = self.extract_join_conditions(
            &current_tables,
            &right_table,
            on_clause,
        )?;

        // 解析输出列
        let output_columns = self.build_output_columns_for_join(
            &current_tables,
            &right_table,
        )?;

        // 构建 Join 节点
        current_plan = PhysicalPlan::Join(JoinNode {
            left: Box::new(current_plan),
            right: Box::new(right_plan),
            conditions,
            output_columns,
        });

        current_tables.push(right_table);
    }

    Ok(current_plan)
}
```

### 3.2 ON 条件解析

```rust
fn extract_join_conditions(
    &self,
    left_tables: &[String],
    right_table: &str,
    on_expr: &Expr,
) -> Result<Vec<JoinCondition>, PlanError> {
    // 处理 AND 组合
    if let Expr::BinaryOp { left, op: BinaryOperator::And, right } = on_expr {
        let left_conditions = self.extract_join_conditions(left_tables, right_table, left)?;
        let right_conditions = self.extract_join_conditions(left_tables, right_table, right)?;
        return Ok(left_conditions.into_iter().chain(right_conditions).collect());
    }

    // 处理单一等值条件
    if let Expr::BinaryOp { left, op: BinaryOperator::Eq, right } = on_expr {
        let left_ref = self.resolve_column_ref(left, left_tables)?;
        let right_ref = self.resolve_column_ref(right, &[right_table])?;

        // 验证：左边列必须来自左表，右边列必须来自右表
        //（或反过来，需要调整）
        if left_ref.table.as_ref() == Some(right_table) {
            // 反序：right.col = left.col，需要交换
            return Ok(vec![JoinCondition {
                left_column: right_ref,
                right_column: left_ref,
            }]);
        }

        return Ok(vec![JoinCondition {
            left_column: left_ref,
            right_column: right_ref,
        }]);
    }

    Err(PlanError::UnsupportedExpression)
}
```

### 3.3 列名解析（支持 t.col）

```rust
fn resolve_column_ref(
    &self,
    expr: &Expr,
    available_tables: &[String],
) -> Result<ColumnRef, PlanError> {
    match expr {
        // t.col 格式
        Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
            let table = parts[0].value.to_lowercase();
            let column = parts[1].value.to_lowercase();

            // 验证表存在
            self.validate_table(&table)?;

            // 验证列存在
            let columns = self.tables.get(&table)
                .ok_or(PlanError::TableNotFound(table.clone()))?;
            if !columns.contains(&column) {
                return Err(PlanError::ColumnNotFound(column));
            }

            Ok(ColumnRef { table: Some(table), column })
        }

        // 纯列名格式
        Expr::Identifier(ident) => {
            let column = ident.value.to_lowercase();

            // 查找列来源（检查所有可用表）
            let sources: Vec<String> = available_tables.iter()
                .filter(|t| {
                    self.tables.get(*t)
                        .map(|cols| cols.contains(&column))
                        .unwrap_or(false)
                })
                .collect();

            match sources.len() {
                0 => Err(PlanError::ColumnNotFound(column)),
                1 => Ok(ColumnRef { table: None, column }),
                _ => Err(PlanError::AmbiguousColumn(column)),
            }
        }

        _ => Err(PlanError::UnsupportedExpression),
    }
}
```

---

## 4. 执行层设计

### 4.1 JoinExecutor 实现

```rust
pub struct JoinExecutor {
    left_executor: Box<dyn Executor>,
    right_executor: Box<dyn Executor>,
    conditions: Vec<JoinCondition>,
    output_columns: Vec<OutputColumn>,

    // 哈希连接状态
    right_hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>>,
    left_rows: Vec<Vec<Value>>,
    current_left_index: usize,
    current_right_matches: Vec<Vec<Value>>,
    current_right_index: usize,

    phase: JoinPhase,
    executed: bool,
}

enum JoinPhase {
    BuildRight,  // 构建右表哈希表
    ScanLeft,    // 扫描左表并缓存
    Output,      // 输出匹配结果
}
```

### 4.2 哈希连接流程

```rust
#[async_trait::async_trait]
impl Executor for JoinExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if !self.executed {
            self.executed = true;
            self.phase = JoinPhase::BuildRight;
        }

        loop {
            match self.phase {
                JoinPhase::BuildRight => {
                    // 执行右表，构建哈希表
                    while let Some(result) = self.right_executor.next().await? {
                        if let ExecResult::Row(row) = result {
                            // 计算哈希键（ON 条件右表列）
                            let hash_key = self.build_hash_key_right(&row)?;
                            self.right_hashmap.entry(hash_key)
                                .or_insert_with(Vec::new)
                                .push(row);
                        }
                    }
                    self.phase = JoinPhase::ScanLeft;
                }

                JoinPhase::ScanLeft => {
                    // 执行左表，缓存所有行
                    while let Some(result) = self.left_executor.next().await? {
                        if let ExecResult::Row(row) = result {
                            self.left_rows.push(row);
                        }
                    }
                    self.phase = JoinPhase::Output;
                }

                JoinPhase::Output => {
                    // 逐行匹配输出
                    while self.current_left_index < self.left_rows.len() {
                        let left_row = &self.left_rows[self.current_left_index];
                        let hash_key = self.build_hash_key_left(left_row)?;

                        // 查找匹配的右表行
                        if self.current_right_index == 0 {
                            self.current_right_matches = self.right_hashmap
                                .get(&hash_key)
                                .cloned()
                                .unwrap_or_default();
                        }

                        // 输出当前匹配
                        if self.current_right_index < self.current_right_matches.len() {
                            let right_row = &self.current_right_matches[self.current_right_index];
                            let output_row = self.build_output_row(left_row, right_row);
                            self.current_right_index += 1;
                            return Ok(Some(ExecResult::Row(output_row)));
                        }

                        // 当前左表行所有匹配已输出，移到下一行
                        self.current_left_index += 1;
                        self.current_right_index = 0;
                    }

                    // 所有行已处理完毕
                    return Ok(None);
                }
            }
        }
    }
}
```

### 4.3 哈希键计算

```rust
fn build_hash_key_right(&self, row: &[Value]) -> Result<Vec<Value>> {
    // ON 条件中所有右表列的值组合
    self.conditions.iter()
        .map(|cond| {
            let idx = self.find_column_index_right(&cond.right_column)?;
            Ok(row[idx].clone())
        })
        .collect::<Result<Vec<_>>>()
}

fn build_hash_key_left(&self, row: &[Value]) -> Result<Vec<Value>> {
    // ON 条件中所有左表列的值组合
    self.conditions.iter()
        .map(|cond| {
            let idx = self.find_column_index_left(&cond.left_column)?;
            Ok(row[idx].clone())
        })
        .collect::<Result<Vec<_>>>()
}
```

### 4.4 输出行构建

```rust
fn build_output_row(&self, left_row: &[Value], right_row: &[Value]) -> Vec<Value> {
    self.output_columns.iter()
        .map(|col| {
            // 根据列所属表选择左/右行中的值
            if col.table_alias == self.left_table_name {
                left_row[col.column_index]
            } else {
                right_row[col.column_index]
            }
        })
        .collect()
}
```

---

## 5. 列名歧义处理

### 5.1歧义检测逻辑

纯列名格式（不带表名）时，检查所有可用表：
- 0 个匹配 → ColumnNotFound 错误
- 1 个匹配 → 自动确定来源
- ≥2 个匹配 → AmbiguousColumn 错误，要求使用 t.col 格式

### 5.2 列索引映射

JoinExecutor 需要维护列索引映射表：

```rust
// 在 JoinExecutor::new 中构建
fn build_column_index_maps(
    left_meta: &TableMeta,
    right_meta: &TableMeta,
    output_columns: &[OutputColumn],
) -> Vec<OutputColumn> {
    output_columns.iter()
        .map(|col| {
            let (table_alias, column_index) = if col.table.as_ref() == Some(&left_meta.name)
                || col.table.is_none() && left_meta.columns.contains_key(&col.column)
            {
                (left_meta.name.clone(), left_meta.column_index(&col.column))
            } else {
                (right_meta.name.clone(), right_meta.column_index(&col.column))
            };

            OutputColumn {
                table: col.table.clone(),
                column: col.column.clone(),
                table_alias,
                column_index,
            }
        })
        .collect()
}
```

---

## 6. 错误处理

### 6.1 新增错误类型

```rust
pub enum PlanError {
    // 现有错误...

    // JOIN 相关新增
    AmbiguousColumn(String),      // "Column 'id' exists in multiple tables: orders, users"
    ColumnNotFound(String),       // "Column 'foo' not found in any table"
    InvalidJoinColumn(String),    // "ON condition references column 'bar' which does not exist"
    MissingOnClause,              // "INNER JOIN requires ON clause"
    UnsupportedJoinType,          // "Only INNER JOIN is supported"
    TableNotFound(String),        // "Table 'xxx' not found"
}
```

---

## 7. 测试策略

### 7.1 测试分层

| 层级 | 文件 | 测试数量 | 内容 |
|------|------|----------|------|
| Planner | planner_test.rs | +10 | JOIN AST → PhysicalPlan |
| Executor | join_test.rs | +8 | JoinExecutor 单元测试 |
| Pipeline | pipeline_test.rs | +5 | JOIN 执行管道集成 |
| E2E | e2e_test.rs | +4 | TCP 端到端 JOIN |

### 7.2 测试场景

**基础测试**:

1. 两表单条件 JOIN
   ```sql
   SELECT orders.id, users.name FROM orders JOIN users ON orders.user_id = users.id
   ```

2. 两表 AND 组合条件
   ```sql
   SELECT * FROM orders JOIN users ON orders.user_id = users.id AND orders.status = users.status
   ```

3. 三表链式 JOIN
   ```sql
   SELECT * FROM orders JOIN users ON orders.user_id = users.id
   JOIN products ON orders.product_id = products.id
   ```

**边界测试**:

4. 列名歧义报错
   ```sql
   SELECT id FROM orders JOIN users ON orders.user_id = users.id -- 应报错
   ```

5. 不存在的列报错
   ```sql
   SELECT orders.foo FROM orders JOIN users ON orders.user_id = users.id -- 应报错
   ```

6. ON 条件列不存在报错
   ```sql
   SELECT * FROM orders JOIN users ON orders.foo = users.bar -- 应报错
   ```

**组合测试**:

7. JOIN + WHERE
   ```sql
   SELECT * FROM orders JOIN users ON orders.user_id = users.id WHERE users.status = 'active'
   ```

8. JOIN + ORDER BY
   ```sql
   SELECT * FROM orders JOIN users ON orders.user_id = users.id ORDER BY users.name
   ```

9. JOIN + LIMIT
   ```sql
   SELECT * FROM orders JOIN users ON orders.user_id = users.id LIMIT 10
   ```

---

## 8. 实现顺序

### Phase 1: 基础结构
1. 新增 plan.rs 结构体（JoinNode, ColumnRef, OutputColumn）
2. 新增 error.rs 错误类型
3. 新增 join.rs JoinExecutor 框架（空的 next()）

### Phase 2: 解析层
4. 扩展 ast.rs（extract_join_table_name）
5. 扩展 planner.rs（build_from_clause, extract_join_conditions）
6. planner_test.rs 解析测试

### Phase 3: 执行层
7. 实现 JoinExecutor 哈希连接逻辑
8. join_test.rs 单元测试

### Phase 4: 集成
9. 扩展 pipeline.rs（JoinExecutor 创建）
10. pipeline_test.rs 集成测试
11. e2e_test.rs 端到端测试

---

## 9. 性能考虑

### 9.1 哈希连接复杂度

- 时间复杂度：O(M + N)（构建哈希表 + 扫描匹配）
- 空间复杂度：O(N)（右表哈希表）

### 9.2 内存优化（未来可选）

当前实现将右表全部加载到内存。对于超大表可考虑：
- 分区哈希连接（grace hash join）
- 外部哈希连接（disk-based）

M12 不实现这些优化，保持简单。

---

## 10. 验收标准

- 所有测试通过（279 + 27 ≈ 306 tests）
- cargo clippy 无警告
- cargo fmt 格式正确
- E2E JOIN 查询可执行
- 三表链式 JOIN 正确输出
- 列名歧义正确报错