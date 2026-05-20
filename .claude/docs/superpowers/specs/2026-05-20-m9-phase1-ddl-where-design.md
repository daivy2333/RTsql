# M9 第一阶段设计文档：DDL + WHERE 表达式求值器

> 创建日期：2026-05-20
> 里程碑：M9 - SQL 基础能力完善（第一阶段）
> 状态：设计完成，待用户审查

---

## 一、需求范围

### 1.1 功能范围

| 功能模块 | 需求内容 | 优先级 |
|---------|---------|--------|
| **DDL** | CREATE TABLE + DROP TABLE IF EXISTS | 🔴 高（解决阻塞） |
| **列类型扩展** | INT + VARCHAR/TEXT + FLOAT + BOOL | 🔴 高（完整类型支持） |
| **表约束** | PRIMARY KEY + NOT NULL + UNIQUE + DEFAULT | 🟡 中 |
| **WHERE 表达式** | 比较操作（> / < / >= / <= / =）+ 逻辑操作（AND / OR）+ 不等于（!=） | 🔴 高（核心查询能力） |
| **WHERE 架构** | Predicate trait + 表达式求值器 | 🔴 高 |

### 1.2 推迟功能

| 功能 | 原因 | 推迟到 |
|------|------|--------|
| ORDER BY | 相对独立，WHERE 是查询核心能力 | M9 第二阶段 |
| LIMIT/OFFSET | 相对独立，WHERE 是查询核心能力 | M9 第二阶段 |
| 聚合函数（COUNT/SUM/AVG） | 需要聚合算子设计 | 后续里程碑 |
| 复杂 JOIN | 需要多表计划设计 | M12 |

---

## 二、架构概览

### 2.1 变更模块范围

```
变更模块：
  ┌─────────────────────────────────────────────┐
  │  Parser Layer (src/parser/)                 │  ← DDL 解析 + WHERE 解析
  │    - ast.rs (新增 DDL Statement 解析)       │
  │    - planner.rs (扩展 DDL + WHERE Plan)     │
  ├─────────────────────────────────────────────┤
  │  Executor Layer (src/executor/)             │  ← 新增 DDL + Filter Executor
  │    - plan.rs (新增 CreateTable/DropTable)   │
  │    - predicate.rs (新增表达式求值器 trait)  │
  │    - create_table.rs (新增 DDL Executor)    │
  │    - drop_table.rs (新增 DDL Executor)      │
  │    - filter.rs (新增 WHERE 过滤 Executor)   │
  ├─────────────────────────────────────────────┤
  │  Storage Layer (src/storage/data/)          │  ← 扩展列类型 + 表约束验证
  │    - table_manager.rs (扩展约束检查)        │
  │    - page_format/tuple.rs (新增 Float/Bool) │
  ├─────────────────────────────────────────────┤
  │  Pipeline (src/pipeline.rs)                 │  ← DDL + WHERE 流程集成
  └─────────────────────────────────────────────┘
```

### 2.2 架构设计原则

- **不破坏现有架构**：沿用 PlanBuilder → PhysicalPlan → Executor 流程
- **新增而非修改**：DDL 作为新的 PhysicalPlan 节点，WHERE 作为 Predicate trait
- **保持异步边界清晰**：DDL Executor 调用 TableManager async API，Predicate 在 Executor 内同步执行
- **渐进式实现**：先 DDL → 列类型扩展 → WHERE 表达式，每阶段独立验证

### 2.3 渐进式实现顺序

| 阶段 | 内容 | 验证点 |
|------|------|--------|
| **1.1** | DDL 解析 + PhysicalPlan 扩展 | PlanBuilder 单元测试 |
| **1.2** | DDL Executor 实现 | DDL 集成测试（CREATE/DROP） |
| **1.3** | 列类型扩展（FLOAT/BOOL） | 序列化/反序列化测试 |
| **1.4** | 表约束验证（NOT NULL/UNIQUE） | 约束违反测试 |
| **1.5** | WHERE 表达式求值器（Predicate trait） | Predicate 单元测试 |
| **1.6** | FilterExecutor 实现（WHERE 过滤） | WHERE 集成测试 |

---

## 三、DDL 实现设计

### 3.1 PhysicalPlan 扩展

```rust
// src/executor/plan.rs
pub enum PhysicalPlan {
    // ... 现有节点
    CreateTable {
        table_name: String,
        columns: Vec<ColumnDef>,
        primary_key: Option<String>,  // 主键列名
    },
    DropTable {
        table_name: String,
        if_exists: bool,  // 是否带 IF EXISTS
    },
}

pub struct ColumnDef {
    name: String,
    data_type: ColumnType,  // 扩展为 INT/VARCHAR/FLOAT/BOOL
    constraints: Vec<ColumnConstraint>,  // NOT NULL/UNIQUE/DEFAULT
}

pub enum ColumnConstraint {
    NotNull,
    Unique,
    DefaultValue(Value),  // DEFAULT 值
}
```

### 3.2 DDL Executor 实现

**CreateTableExecutor**：
```rust
// src/executor/create_table.rs
pub struct CreateTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
}

impl Executor for CreateTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if let PhysicalPlan::CreateTable { table_name, columns, primary_key } = &self.plan {
            // 1. 检查表是否已存在
            if self.database.table_manager.get_table(table_name).await.is_ok() {
                return Err(StorageError::TableAlreadyExists(table_name.clone()));
            }
            
            // 2. 转换 ColumnDef → TableManager 的列定义格式
            let schema = columns.iter().map(|c| c.to_schema_column()).collect();
            
            // 3. 调用 TableManager::create_table
            self.database.table_manager.create_table(table_name, schema, primary_key).await?;
            
            // 4. 返回成功
            return Ok(Some(ExecResult::AffectedRows(0)));
        }
        Err(ExecutorError::InvalidPlan)
    }
}
```

**DropTableExecutor**：
```rust
// src/executor/drop_table.rs
pub struct DropTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
}

impl Executor for DropTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if let PhysicalPlan::DropTable { table_name, if_exists } = &self.plan {
            // 1. 检查表是否存在
            let exists = self.database.table_manager.get_table(table_name).await.is_ok();
            
            if !exists && !if_exists {
                return Err(StorageError::TableNotFound(table_name.clone()));
            }
            
            if exists {
                // 2. 调用 TableManager::drop_table（需新增）
                self.database.table_manager.drop_table(table_name).await?;
            }
            
            // 3. 返回成功
            return Ok(Some(ExecResult::AffectedRows(0)));
        }
        Err(ExecutorError::InvalidPlan)
    }
}
```

### 3.3 PlanBuilder DDL 解析

```rust
// src/parser/planner.rs
impl PlanBuilder {
    pub fn build_statement(&mut self, stmt: Statement) -> Result<PhysicalPlan> {
        match stmt {
            Statement::CreateTable { name, columns, constraints, .. } => {
                self.build_create_table(name, columns, constraints)
            }
            Statement::Drop { object_type, names, if_exists, .. } => {
                if object_type == ObjectType::Table {
                    self.build_drop_table(names[0], if_exists)
                } else {
                    Err(PlanError::UnsupportedStatement)
                }
            }
            // ... 现有逻辑
        }
    }
    
    fn build_create_table(
        &mut self,
        name: ObjectName,
        columns: &[ColumnDef],
        constraints: &[TableConstraint],
    ) -> Result<PhysicalPlan> {
        // 1. 提取表名
        let table_name = name.to_string();
        
        // 2. 检查空列定义
        if columns.is_empty() {
            return Err(PlanError::EmptyColumnDefinition);
        }
        
        // 3. 提取列定义
        let column_defs: Vec<ColumnDef> = columns.iter().map(|c| {
            ColumnDef {
                name: c.name.to_string(),
                data_type: self.convert_data_type(&c.data_type),
                constraints: self.extract_column_constraints(c),
            }
        }).collect();
        
        // 4. 提取主键（从列约束或表约束）
        let primary_key = self.extract_primary_key(columns, constraints);
        
        // 5. 检查多主键
        if primary_key.is_some() && constraints.iter().any(|c| matches!(c, TableConstraint::PrimaryKey { .. })) {
            return Err(PlanError::MultiplePrimaryKey);
        }
        
        Ok(PhysicalPlan::CreateTable {
            table_name,
            columns: column_defs,
            primary_key,
        })
    }
    
    fn convert_data_type(&self, data_type: &SQLDataType) -> ColumnType {
        match data_type {
            SQLDataType::Int(_) => ColumnType::Int,
            SQLDataType::Varchar(_) | SQLDataType::Text => ColumnType::String,
            SQLDataType::Float(_) | SQLDataType::Double => ColumnType::Float,
            SQLDataType::Boolean => ColumnType::Bool,
            _ => ColumnType::Null,
        }
    }
}
```

---

## 四、列类型扩展设计

### 4.1 ColumnType 扩展

```rust
// src/storage/page_format/tuple.rs
pub enum ColumnType {
    Int,      // 已有（INTEGER/BIGINT）
    String,   // 已有（VARCHAR/TEXT）
    Float,    // 新增（FLOAT/DOUBLE）
    Bool,     // 新增（BOOLEAN）
    Null,     // 已有
}
```

### 4.2 Value 扩展

```rust
// src/executor/value.rs
pub enum Value {
    Int(i64),
    String(String),
    Float(f64),  // 新增
    Bool(bool),  // 新增
    Null,
}

impl Value {
    pub fn as_float(&self) -> Result<f64> {
        match self {
            Value::Float(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),  // 允许隐式转换
            _ => Err(ValueError::TypeMismatch),
        }
    }
    
    pub fn as_bool(&self) -> Result<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Int(i) => Ok(*i != 0),  // 允许隐式转换
            _ => Err(ValueError::TypeMismatch),
        }
    }
}
```

### 4.3 序列化/反序列化扩展

**序列化格式**：
```
Float: [0x04][8 bytes f64 LE]  (9 bytes total)
Bool:  [0x05][1 byte (0/1)]    (2 bytes total)

现有格式：
Int:    [0x01][8 bytes i64 LE]  (9 bytes)
String: [0x02][2 bytes len LE][N bytes UTF-8]  (3+N bytes)
Null:   [0x03]                  (1 byte)
```

**serialize_tuple 扩展**：
```rust
pub fn serialize_tuple(values: &[Value], schema: &[ColumnType], buf: &mut Vec<u8>) -> Result<usize> {
    for (value, col_type) in values.iter().zip(schema.iter()) {
        match col_type {
            ColumnType::Float => {
                buf.push(0x04);
                let f = value.as_float()?;
                buf.extend_from_slice(&f.to_le_bytes());
            }
            ColumnType::Bool => {
                buf.push(0x05);
                let b = value.as_bool()?;
                buf.push(if b { 1 } else { 0 });
            }
            // ... 现有逻辑
        }
    }
    Ok(buf.len())
}
```

---

## 五、WHERE 表达式求值器设计

### 5.1 Predicate trait 设计

```rust
// src/executor/predicate.rs
pub trait Predicate: Send + Sync {
    fn evaluate(&self, row: &[Value]) -> Result<bool>;
}

pub type PredicateRef = Arc<dyn Predicate>;
```

### 5.2 基础谓词实现

**ComparisonPredicate（比较操作）**：
```rust
pub struct ComparisonPredicate {
    left: Expression,
    op: ComparisonOp,
    right: Expression,
}

pub enum ComparisonOp {
    Eq,   // =
    Ne,   // !=
    Gt,   // >
    Lt,   // <
    Ge,   // >=
    Le,   // <=
}

impl Predicate for ComparisonPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool> {
        let left_val = self.left.evaluate(row)?;
        let right_val = self.right.evaluate(row)?;
        
        match self.op {
            ComparisonOp::Eq => Ok(left_val.equals(&right_val)),
            ComparisonOp::Ne => Ok(!left_val.equals(&right_val)),
            ComparisonOp::Gt => Ok(left_val.gt(&right_val)?),
            ComparisonOp::Lt => Ok(left_val.lt(&right_val)?),
            ComparisonOp::Ge => Ok(left_val.ge(&right_val)?),
            ComparisonOp::Le => Ok(left_val.le(&right_val)?),
        }
    }
}
```

**LogicalPredicate（逻辑操作）**：
```rust
pub struct LogicalPredicate {
    left: PredicateRef,
    op: LogicalOp,
    right: PredicateRef,
}

pub enum LogicalOp {
    And,
    Or,
}

impl Predicate for LogicalPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool> {
        let left_result = self.left.evaluate(row)?;
        let right_result = self.right.evaluate(row)?;
        
        match self.op {
            LogicalOp::And => Ok(left_result && right_result),
            LogicalOp::Or => Ok(left_result || right_result),
        }
    }
}
```

### 5.3 Expression 设计

```rust
pub trait Expression: Send + Sync {
    fn evaluate(&self, row: &[Value]) -> Result<Value>;
}

pub enum SimpleExpression {
    Column(String),       // 列引用（通过列名查找值）
    Constant(Value),      // 常量值
}

impl Expression for SimpleExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value> {
        match self {
            SimpleExpression::Constant(v) => Ok(v.clone()),
            SimpleExpression::Column(name) => {
                // 需要列名 → 索引映射（从 Schema 获取）
                // 当前设计：Executor 持有 Schema，传递给 Predicate
                Err(ValueError::ColumnNotFound(name.clone()))
            }
        }
    }
}
```

**列名 → 索引映射问题**：
- 当前设计：Executor 持有 `TableMeta`（包含 Schema）
- Predicate 需要知道列名对应的索引
- **解决方案**：Predicate 持有 `HashMap<String, usize>`（列名 → 索引映射）

**改进设计**：
```rust
pub struct ColumnExpression {
    column_name: String,
    column_index: usize,  // 在构建时解析
}

impl Expression for ColumnExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value> {
        Ok(row[self.column_index].clone())
    }
}
```

### 5.4 PlanBuilder WHERE 解析

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn build_where(&self, expr: &Expr, schema: &[ColumnDef]) -> Result<PredicateRef> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                let op_str = op.to_string();
                
                // 判断是逻辑操作还是比较操作
                if op_str == "AND" || op_str == "OR" {
                    // 逻辑操作
                    let left_pred = self.build_where(left, schema)?;
                    let right_pred = self.build_where(right, schema)?;
                    let logical_op = if op_str == "AND" { LogicalOp::And } else { LogicalOp::Or };
                    
                    Ok(Arc::new(LogicalPredicate {
                        left: left_pred,
                        op: logical_op,
                        right: right_pred,
                    }))
                } else {
                    // 比较操作
                    let left_expr = self.build_expression(left, schema)?;
                    let right_expr = self.build_expression(right, schema)?;
                    let comp_op = self.convert_comparison_op(op);
                    
                    Ok(Arc::new(ComparisonPredicate {
                        left: left_expr,
                        op: comp_op,
                        right: right_expr,
                    }))
                }
            }
            // ... 其他表达式类型
        }
    }
    
    fn build_expression(&self, expr: &Expr, schema: &[ColumnDef]) -> Result<ExpressionRef> {
        match expr {
            Expr::Identifier(name) => {
                // 列引用 → 查找索引
                let column_index = schema.iter().position(|c| c.name == name.to_string())
                    .ok_or(PlanError::ColumnNotFound(name.to_string()))?;
                
                Ok(Arc::new(ColumnExpression {
                    column_name: name.to_string(),
                    column_index,
                }))
            }
            Expr::Value(v) => {
                // 常量 → 转为 Value
                Ok(Arc::new(ConstantExpression(self.convert_value(v)?)))
            }
            // ... 其他表达式类型
        }
    }
}
```

---

## 六、FilterExecutor 设计

### 6.1 WHERE 过滤 Executor

```rust
// src/executor/filter.rs
pub struct FilterExecutor {
    input: ExecutorRef,      // 子 Executor（Scan/IndexScan）
    predicate: PredicateRef,  // WHERE 条件
    schema: Vec<ColumnDef>,   // 列定义（用于读取行数据）
    database: Arc<Database>,  // 用于读取行数据
}

impl Executor for FilterExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // 循环直到找到满足条件的行
        loop {
            match self.input.next().await? {
                None => return Ok(None),  // 子 Executor 结束
                
                Some(ExecResult::Row(row_id)) => {
                    // 读取行数据
                    let (version_header, tuple_bytes) = read_tuple_from_data_page(
                        &self.database.buffer_pool,
                        row_id,
                    ).await?;
                    
                    // 反序列化
                    let table_meta = self.database.table_manager.get_table(&self.table_name).await?;
                    let row = deserialize_tuple(&tuple_bytes, &table_meta.schema)?;
                    
                    // MVCC 可见性检查（沿用现有逻辑）
                    if !self.snapshot.is_visible(
                        version_header.create_tx_id(),
                        version_header.commit_tx_id(),
                    ) {
                        continue;  // 不可见，继续循环
                    }
                    
                    // 应用 Predicate
                    if self.predicate.evaluate(&row)? {
                        return Ok(Some(ExecResult::Row(row_id)));
                    }
                    // 不满足 WHERE 条件，继续循环
                }
                
                Some(ExecResult::AffectedRows(n)) => {
                    // 写操作直接返回（WHERE 用于 Update/Delete）
                    return Ok(Some(ExecResult::AffectedRows(n)));
                }
                
                Some(ExecResult::Rows(rows)) => {
                    // 批量行结果（未来支持）
                    return Ok(Some(ExecResult::Rows(rows)));
                }
            }
        }
    }
}
```

### 6.2 PhysicalPlan 扩展（WHERE）

```rust
// src/executor/plan.rs
pub enum PhysicalPlan {
    // ... 现有节点
    Filter {
        input: PhysicalPlan,       // 子计划
        predicate: PredicateRef,   // WHERE 条件（已构建）
        table_name: String,        // 表名（用于读取 Schema）
    },
}
```

---

## 七、错误处理设计

### 7.1 错误类型扩展

**StorageError 扩展**：
```rust
// src/storage/error.rs
pub enum StorageError {
    // ... 现有错误
    TableAlreadyExists(String),  // CREATE TABLE 表已存在
    TableNotFound(String),       // DROP TABLE 表不存在
    ColumnNotFound(String),      // WHERE 列不存在
    ConstraintViolation(String), // INSERT 违反 NOT NULL/UNIQUE
    InvalidColumnType(String),   // 类型转换错误
}
```

**PlanError 扩展**：
```rust
// src/parser/error.rs
pub enum PlanError {
    // ... 现有错误
    EmptyColumnDefinition,       // CREATE TABLE 空列定义
    MultiplePrimaryKey,          // CREATE TABLE 多主键
    InvalidConstraint(String),   // 无效约束
}
```

**ValueError 扩展**：
```rust
// src/executor/value.rs
pub enum ValueError {
    TypeMismatch,                // 类型不匹配
    ColumnNotFound(String),      // 列不存在
    NullComparison,              // NULL 比较（需用 IS NULL）
}
```

### 7.2 错误返回流程

```
错误流向：
  PlanBuilder（解析错误） → PlanError → Pipeline → Response::Error
  Executor（执行错误） → StorageError/ValueError → Pipeline → Response::Error
  Predicate（求值错误） → ValueError → Executor → Pipeline → Response::Error
```

---

## 八、表约束验证设计

### 8.1 TableManager 约束检查扩展

```rust
// src/storage/data/table_manager.rs
impl TableManager {
    pub async fn create_table(
        &mut self,
        name: String,
        columns: Vec<ColumnSchema>,
        primary_key: Option<String>,
    ) -> Result<()> {
        // ... 现有逻辑
        
        // 新增：检查约束定义
        if let Some(pk) = &primary_key {
            // 检查主键列是否存在
            if !columns.iter().any(|c| c.name == *pk) {
                return Err(StorageError::ColumnNotFound(pk.clone()));
            }
        }
        
        // 检查 NOT NULL + UNIQUE 列定义
        for col in &columns {
            if col.not_null && col.unique {
                // 需要额外处理（UNIQUE 约束需创建额外索引）
            }
        }
        
        // ... 创建表逻辑
    }
    
    pub async fn drop_table(&mut self, name: &str) -> Result<()> {
        // 1. 检查表是否存在
        let meta = self.tables.get(name).ok_or(StorageError::TableNotFound(name.to_string()))?;
        
        // 2. 删除表的所有数据页（遍历 data_page 链表）
        // 3. 删除表的索引页（BTree 所有页）
        // 4. 从元数据中移除
        
        // 5. 删除成功
        Ok(())
    }
}
```

### 8.2 INSERT 约束检查（InsertExecutor）

```rust
// src/executor/insert.rs
impl Executor for InsertExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // ... 现有逻辑
        
        // 新增：约束检查
        let table_meta = self.database.table_manager.get_table(&self.table_name).await?;
        
        for (value, col) in values.iter().zip(table_meta.schema.iter()) {
            // NOT NULL 检查
            if col.not_null && value == Value::Null {
                return Err(StorageError::ConstraintViolation(
                    format!("Column '{}' cannot be NULL", col.name)
                ));
            }
            
            // UNIQUE 检查（需要额外索引查询）
            // 暂时推迟（需要 UNIQUE 索引设计）
        }
        
        // ... 插入逻辑
    }
}
```

---

## 九、数据流与集成

### 9.1 DDL 数据流

```
CREATE TABLE 流程：
  SQL → Parser::parse_sql
      → Statement::CreateTable
      → PlanBuilder::build_create_table
      → PhysicalPlan::CreateTable
      → CreateTableExecutor::next
      → TableManager::create_table
      → Response::Success

DROP TABLE 流程：
  SQL → Parser::parse_sql
      → Statement::Drop
      → PlanBuilder::build_drop_table
      → PhysicalPlan::DropTable
      → DropTableExecutor::next
      → TableManager::drop_table
      → Response::Success/Error
```

### 9.2 WHERE 数据流

```
SELECT with WHERE 流程：
  SQL → Parser::parse_sql
      → Statement::Query(Select { where: Some(expr) })
      → PlanBuilder::build_query
          → build_where(expr) → PredicateRef
          → PhysicalPlan::Filter {
              input: PhysicalPlan::Scan/IndexScan,
              predicate: PredicateRef,
          }
      → FilterExecutor::next
          → input.next() → RowId
          → read_tuple → Row
          → predicate.evaluate(Row) → bool
          → 返回满足条件的 RowId
      → Response::Rows
```

### 9.3 Pipeline 集成

```rust
// src/pipeline.rs
pub async fn execute(database: Arc<Database>, sql: &str) -> Result<Response> {
    // 1. 解析 SQL
    let statements = parse_sql(sql)?;
    
    // 2. 遍历 Statement
    for stmt in statements {
        match stmt {
            Statement::CreateTable { .. } => {
                // DDL 流程
                let plan = PlanBuilder::new().build_statement(stmt)?;
                let executor = CreateTableExecutor::new(plan, database.clone());
                let result = executor.next().await?;
                // 转为 Response
            }
            Statement::Query { .. } => {
                // SELECT 流程（WHERE 处理）
                let plan = PlanBuilder::new()
                    .register_table_from_sql(&stmt, &database.table_manager)
                    .build_statement(stmt)?;
                
                // 根据 plan 类型创建 Executor
                let executor = create_executor(plan, database.clone());
                
                // 执行 + 收集结果
                let rows = collect_rows(executor)?;
                
                return Ok(Response::QueryResult { rows });
            }
            // ... 其他 Statement
        }
    }
}
```

---

## 十、测试策略

### 10.1 单元测试

| 测试模块 | 测试内容 | 文件位置 |
|---------|---------|---------|
| **DDL 解析** | CREATE TABLE/DROP TABLE 语法解析 | tests/parser_test.rs |
| **Predicate** | ComparisonPredicate/LogicalPredicate evaluate | tests/predicate_test.rs |
| **列类型序列化** | Float/Bool serialize/deserialize | tests/tuple_test.rs |

**示例测试**：
```rust
// tests/predicate_test.rs
#[test]
fn test_comparison_predicate_gt() {
    let pred = ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "age", column_index: 0 }),
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression(Value::Int(18))),
    };
    
    let row = vec![Value::Int(25)];
    assert!(pred.evaluate(&row).unwrap());
    
    let row = vec![Value::Int(15)];
    assert!(!pred.evaluate(&row).unwrap());
}

#[test]
fn test_logical_predicate_and() {
    let pred = LogicalPredicate {
        left: Arc::new(ComparisonPredicate { /* age > 18 */ }),
        op: LogicalOp::And,
        right: Arc::new(ComparisonPredicate { /* status = 'active' */ }),
    };
    
    let row = vec![Value::Int(25), Value::String("active".to_string())];
    assert!(pred.evaluate(&row).unwrap());
}
```

### 10.2 集成测试

| 测试场景 | 测试内容 | 文件位置 |
|---------|---------|---------|
| **DDL 流程** | CREATE TABLE → INSERT → SELECT → DROP | tests/ddl_test.rs |
| **WHERE 过滤** | 各种操作组合（> / < / AND / OR） | tests/where_test.rs |

**示例测试**：
```rust
// tests/ddl_test.rs
#[tokio::test]
async fn test_create_table_success() {
    let db = Database::open(temp_path()).await?;
    
    // CREATE TABLE
    let sql = "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL)";
    let response = db.execute_sql(sql).await?;
    assert!(matches!(response, Response::Success));
    
    // INSERT
    let sql = "INSERT INTO users VALUES (1, 'Alice')";
    let response = db.execute_sql(sql).await?;
    assert!(matches!(response, Response::AffectedRows(1)));
    
    // SELECT
    let sql = "SELECT * FROM users WHERE id = 1";
    let response = db.execute_sql(sql).await?;
    assert!(matches!(response, Response::QueryResult { .. }));
}

#[tokio::test]
async fn test_drop_table_if_exists() {
    let db = Database::open(temp_path()).await?;
    
    // CREATE TABLE
    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY)").await?;
    
    // DROP TABLE IF EXISTS
    let response = db.execute_sql("DROP TABLE IF EXISTS users").await?;
    assert!(matches!(response, Response::Success));
    
    // 再次 DROP（不报错）
    let response = db.execute_sql("DROP TABLE IF EXISTS users").await?;
    assert!(matches!(response, Response::Success));
}
```

### 10.3 端到端测试

| 测试场景 | 测试内容 | 文件位置 |
|---------|---------|---------|
| **PostgreSQL 协议 + DDL** | 通过 psql 发送 CREATE/DROP + SELECT | tests/e2e_test.rs（扩展） |

---

## 十一、场景草图（BDD）

### 11.1 DDL 场景

| 场景 | 输入 | 输出 | 状态 |
|------|------|------|------|
| **Happy Path**: CREATE TABLE 成功 | `CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100))` | 成功 | ✅ |
| **Sad Path**: CREATE TABLE 表已存在 | 重复执行 CREATE TABLE | 错误 "Table 'users' already exists" | ✅ |
| **Sad Path**: DROP TABLE 表不存在（无 IF EXISTS） | `DROP TABLE nonexistent` | 错误 "Table 'nonexistent' not found" | ✅ |
| **Happy Path**: DROP TABLE IF EXISTS | `DROP TABLE IF EXISTS users` | 成功 | ✅ |
| **Edge Case**: CREATE TABLE 空列定义 | `CREATE TABLE empty ()` | 错误 "Table must have at least one column" | ✅ |
| **Edge Case**: CREATE TABLE 多主键 | `CREATE TABLE t (id INT PRIMARY KEY, name INT PRIMARY KEY)` | 错误 "Multiple primary keys defined" | ✅ |

### 11.2 WHERE 场景

| 场景 | 输入 | 输出 | 状态 |
|------|------|------|------|
| **Happy Path**: WHERE 比较操作 | `SELECT * FROM users WHERE age > 18` | 返回符合条件的行 | ✅ |
| **Happy Path**: WHERE 逻辑操作（AND） | `SELECT * FROM users WHERE age > 18 AND status = 'active'` | 返回同时满足的行 | ✅ |
| **Happy Path**: WHERE 逻辑操作（OR） | `SELECT * FROM users WHERE age < 18 OR age > 60` | 返回满足任一的行 | ✅ |
| **Happy Path**: WHERE 不等于 | `SELECT * FROM users WHERE status != 'inactive'` | 返回不等于的行 | ✅ |
| **Sad Path**: WHERE 列不存在 | `SELECT * FROM users WHERE nonexistent = 1` | 错误 "Column 'nonexistent' not found" | ✅ |
| **Edge Case**: WHERE NULL 比较 | `SELECT * FROM users WHERE name = NULL` | 返回空（需用 IS NULL） | ⚠️ |
| **Edge Case**: WHERE 类型不匹配 | `SELECT * FROM users WHERE age = 'string'` | 类型错误或空结果 | ⚠️ |

### 11.3 表约束场景

| 场景 | 输入 | 输出 | 状态 |
|------|------|------|------|
| **Sad Path**: INSERT 违反 NOT NULL | `INSERT INTO users (id) VALUES (1)`（name 有 NOT NULL） | 错误 "Column 'name' cannot be NULL" | ⚠️ |
| **Sad Path**: INSERT 违反 UNIQUE | INSERT 重复主键值 | 错误 "Duplicate key value" | ⚠️ |

**注**：⚠️ 标记的场景为推迟功能（表约束验证在阶段 1.4 实现）。

---

## 十二、风险与约束

### 12.1 技术风险

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| **Predicate 列名映射复杂度** | 列名 → 索引映射需要 Schema 信息 | Predicate 持有 HashMap 或在 Executor 层传递 Schema |
| **类型比较复杂度** | 不同类型比较（Int vs Float）需要转换 | Value 实现跨类型比较方法 |
| **UNIQUE 约束实现复杂度** | 需要额外索引设计 | 推迟到阶段 1.4，先实现 NOT NULL |

### 12.2 架构约束

- **不破坏现有 MVCC 流程**：WHERE 过滤在 Executor 层，不影响 VersionHeader/Snapshot
- **不破坏现有异步边界**：Predicate 同步执行，不引入 async overhead
- **保持 PlanBuilder 纯同步**：WHERE 解析在 PlanBuilder，不依赖 async 上下文

---

## 十三、实现顺序（渐进式）

| 阶段 | 任务 | 验证点 | 预估工作量 |
|------|------|--------|-----------|
| **1.1** | DDL 解析 + PhysicalPlan 扩展 | parser_test + plan.rs 编译通过 | 2-3h |
| **1.2** | DDL Executor 实现 | ddl_test 通过 | 3-4h |
| **1.3** | 列类型扩展（FLOAT/BOOL） | tuple_test 通过 | 2-3h |
| **1.4** | 表约束验证（NOT NULL） | 约束测试通过 | 2-3h |
| **1.5** | WHERE 表达式求值器（Predicate） | predicate_test 通过 | 4-5h |
| **1.6** | FilterExecutor 实现 | where_test 通过 | 3-4h |

**总预估工作量**：16-20h

---

## 十四、下一步行动

完成本设计审查后，将进入 **Phase 2: PLAN**，调用 writing-plans skill 生成详细实施计划。

---

**设计文档状态**：✅ 完成待审查

**审查要点**：
1. 架构是否合理（DDL + WHERE 设计）
2. Predicate 设计是否清晰（列名映射、类型比较）
3. 渐进式实现顺序是否合理
4. 测试策略是否完整

**请用户审查后确认，然后进入 Phase 2（writing-plans）。**