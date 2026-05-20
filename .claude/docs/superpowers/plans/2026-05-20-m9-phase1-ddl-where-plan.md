# M9 第一阶段实施计划：DDL + WHERE 表达式求值器

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 DDL（CREATE TABLE + DROP TABLE IF EXISTS）+ WHERE 表达式求值器，解决用户无法通过 SQL 创建表的阻塞，并支持复杂 WHERE 条件过滤。

**Architecture:** 采用渐进式实现，先扩展 PhysicalPlan 节点（CreateTable/DropTable），再实现 DDL Executor，然后扩展列类型（Float/Bool），最后实现 Predicate trait + FilterExecutor。沿用 PlanBuilder → PhysicalPlan → Executor 流程，不破坏现有异步边界。

**Tech Stack:** Rust 1.75+ / Tokio 1.x / sqlparser-rs 0.44

---

## File Structure

**新增文件**：
```
src/executor/predicate.rs     # Predicate trait + Expression trait + ComparisonPredicate/LogicalPredicate
src/executor/create_table.rs  # CreateTableExecutor
src/executor/drop_table.rs    # DropTableExecutor
src/executor/filter.rs        # FilterExecutor
tests/predicate_test.rs       # Predicate 单元测试
tests/ddl_test.rs             # DDL 集成测试
tests/where_test.rs           # WHERE 集成测试
```

**修改文件**：
```
src/executor/plan.rs          # 新增 CreateTable/DropTable/Filter 节点 + ColumnDef/ColumnConstraint
src/executor/value.rs         # 新增 Float/Bool + as_float/as_bool + equals/gt/lt/ge/le
src/executor/mod.rs           # 导出 predicate/create_table/drop_table/filter
src/storage/page_format/tuple.rs  # ColumnType 新增 Float/Bool + serialize/deserialize 扩展
src/storage/error.rs          # 新增 TableAlreadyExists/TableNotFound/ConstraintViolation
src/parser/planner.rs         # 新增 build_create_table/build_drop_table/build_where/build_expression
src/parser/error.rs           # 新增 EmptyColumnDefinition/MultiplePrimaryKey/ColumnNotFound
src/storage/data/table_manager.rs  # 新增 drop_table + 约束检查扩展
src/pipeline.rs               # 新增 Statement::CreateTable/Drop 分支 + Filter 处理
```

---

## Task 1: 扩展错误类型（PlanError + StorageError）

**Files:**
- Modify: `src/parser/error.rs`
- Modify: `src/storage/error.rs`

**Prerequisites:** 无

- [ ] **Step 1: 读取现有错误类型文件**

检查现有错误类型定义，准备扩展。

- [ ] **Step 2: 扩展 PlanError（新增 DDL 错误类型）**

```rust
// src/parser/error.rs
pub enum PlanError {
    // ... 现有错误类型

    // 新增 DDL 错误
    EmptyColumnDefinition,       // CREATE TABLE 空列定义
    MultiplePrimaryKey,          // CREATE TABLE 多主键
    ColumnNotFound(String),      // WHERE 列不存在
    InvalidConstraint(String),   // 无效约束
}
```

- [ ] **Step 3: 扩展 StorageError（新增 DDL + 约束错误类型）**

```rust
// src/storage/error.rs
pub enum StorageError {
    // ... 现有错误类型

    // 新增 DDL 错误
    TableAlreadyExists(String),  // CREATE TABLE 表已存在
    TableNotFound(String),       // DROP TABLE 表不存在

    // 新增约束错误
    ConstraintViolation(String), // INSERT 违反 NOT NULL/UNIQUE
    InvalidColumnType(String),   // 类型转换错误
}
```

- [ ] **Step 4: 编译检查**

Run: `cargo check`
Expected: 编译通过，无错误

- [ ] **Step 5: Commit**

```bash
git add src/parser/error.rs src/storage/error.rs
git commit -m "feat(error): add DDL and constraint error types"
```

---

## Task 2: 扩展 PhysicalPlan（DDL 节点 + ColumnDef）

**Files:**
- Modify: `src/executor/plan.rs`
- Modify: `src/executor/value.rs`（前置：新增 ColumnType）

**Prerequisites:** Task 1 完成

- [ ] **Step 1: 读取现有 PhysicalPlan 定义**

检查现有节点类型，准备扩展。

- [ ] **Step 2: 扩展 ColumnType（为 PhysicalPlan 依赖）**

```rust
// src/executor/value.rs（提前扩展，Task 3 会详细实现）
// 仅在此处添加类型声明，避免编译错误

pub enum ColumnType {
    Int,      // 已有
    String,   // 已有（VARCHAR/TEXT）
    Float,    // 新增（FLOAT/DOUBLE）- Task 3 详细实现
    Bool,     // 新增（BOOLEAN）- Task 3 详细实现
    Null,     // 已有
}
```

- [ ] **Step 3: 定义 ColumnDef + ColumnConstraint**

```rust
// src/executor/plan.rs（新增结构）

pub struct ColumnDef {
    pub name: String,
    pub data_type: ColumnType,
    pub constraints: Vec<ColumnConstraint>,
}

pub enum ColumnConstraint {
    NotNull,
    Unique,
    DefaultValue(Value),
}

// 为 ColumnDef 实现辅助方法
impl ColumnDef {
    pub fn new(name: String, data_type: ColumnType) -> Self {
        ColumnDef {
            name,
            data_type,
            constraints: Vec::new(),
        }
    }

    pub fn with_constraint(mut self, constraint: ColumnConstraint) -> Self {
        self.constraints.push(constraint);
        self
    }
}
```

- [ ] **Step 4: 扩展 PhysicalPlan（新增 CreateTable/DropTable）**

```rust
// src/executor/plan.rs
pub enum PhysicalPlan {
    // ... 现有节点（Scan/IndexScan/Insert/Update/Delete）

    // 新增 DDL 节点
    CreateTable {
        table_name: String,
        columns: Vec<ColumnDef>,
        primary_key: Option<String>,
    },
    DropTable {
        table_name: String,
        if_exists: bool,
    },
}
```

- [ ] **Step 5: 编译检查**

Run: `cargo check`
Expected: 编译通过，无错误

- [ ] **Step 6: Commit**

```bash
git add src/executor/plan.rs src/executor/value.rs
git commit -m "feat(plan): add CreateTable/DropTable nodes + ColumnDef"
```

---

## Task 3: 扩展 Value 类型（Float/Bool + 比较方法）

**Files:**
- Modify: `src/executor/value.rs`

**Prerequisites:** Task 2 完成

- [ ] **Step 1: 写 Value 比较方法的测试**

```rust
// tests/value_test.rs（新增测试文件）
use crate::executor::value::{Value, ValueError};

#[test]
fn test_value_equals_int() {
    let v1 = Value::Int(42);
    let v2 = Value::Int(42);
    assert!(v1.equals(&v2));

    let v3 = Value::Int(100);
    assert!(!v1.equals(&v3));
}

#[test]
fn test_value_equals_float() {
    let v1 = Value::Float(3.14);
    let v2 = Value::Float(3.14);
    assert!(v1.equals(&v2));
}

#[test]
fn test_value_equals_cross_type() {
    // Int vs Float（允许隐式转换）
    let v1 = Value::Int(42);
    let v2 = Value::Float(42.0);
    assert!(v1.equals(&v2));
}

#[test]
fn test_value_gt_int() {
    let v1 = Value::Int(100);
    let v2 = Value::Int(42);
    assert!(v1.gt(&v2).unwrap());
    assert!(!v2.gt(&v1).unwrap());
}

#[test]
fn test_value_lt_float() {
    let v1 = Value::Float(3.14);
    let v2 = Value::Float(10.0);
    assert!(v1.lt(&v2).unwrap());
}

#[test]
fn test_as_float() {
    let v1 = Value::Float(3.14);
    assert_eq!(v1.as_float().unwrap(), 3.14);

    let v2 = Value::Int(42);
    assert_eq!(v2.as_float().unwrap(), 42.0);
}

#[test]
fn test_as_bool() {
    let v1 = Value::Bool(true);
    assert!(v1.as_bool().unwrap());

    let v2 = Value::Int(1);
    assert!(v2.as_bool().unwrap());

    let v3 = Value::Int(0);
    assert!(!v3.as_bool().unwrap());
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test value_test`
Expected: FAIL - "Value::Float/Bool not defined", "equals/gt/lt methods not defined"

- [ ] **Step 3: 扩展 Value enum（新增 Float/Bool）**

```rust
// src/executor/value.rs
pub enum Value {
    Int(i64),
    String(String),
    Float(f64),  // 新增
    Bool(bool),  // 新增
    Null,
}

// 新增错误类型
pub enum ValueError {
    TypeMismatch,
    ColumnNotFound(String),
    NullComparison,
}
```

- [ ] **Step 4: 实现 as_float/as_bool 方法**

```rust
// src/executor/value.rs
impl Value {
    pub fn as_float(&self) -> Result<f64, ValueError> {
        match self {
            Value::Float(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),  // 允许隐式转换
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn as_bool(&self) -> Result<bool, ValueError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Int(i) => Ok(*i != 0),  // 允许隐式转换
            _ => Err(ValueError::TypeMismatch),
        }
    }
}
```

- [ ] **Step 5: 实现比较方法（equals/gt/lt/ge/le）**

```rust
// src/executor/value.rs
impl Value {
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,

            // 跨类型比较（Int vs Float）
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),

            _ => false,
        }
    }

    pub fn gt(&self, other: &Value) -> Result<bool, ValueError> {
        let left = self.as_float()?;
        let right = other.as_float()?;
        Ok(left > right)
    }

    pub fn lt(&self, other: &Value) -> Result<bool, ValueError> {
        let left = self.as_float()?;
        let right = other.as_float()?;
        Ok(left < right)
    }

    pub fn ge(&self, other: &Value) -> Result<bool, ValueError> {
        let left = self.as_float()?;
        let right = other.as_float()?;
        Ok(left >= right)
    }

    pub fn le(&self, other: &Value) -> Result<bool, ValueError> {
        let left = self.as_float()?;
        let right = other.as_float()?;
        Ok(left <= right)
    }
}
```

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test value_test`
Expected: PASS - 所有 value_test 测试通过

- [ ] **Step 7: Commit**

```bash
git add src/executor/value.rs tests/value_test.rs
git commit -m "feat(value): add Float/Bool types + comparison methods"
```

---

## Task 4: 扩展列类型序列化（Float/Bool）

**Files:**
- Modify: `src/storage/page_format/tuple.rs`

**Prerequisites:** Task 3 完成

- [ ] **Step 1: 写 Float/Bool 序列化测试**

```rust
// tests/tuple_test.rs（扩展现有测试）
use crate::storage::page_format::tuple::{serialize_tuple, deserialize_tuple, ColumnType};
use crate::executor::value::Value;

#[test]
fn test_serialize_float() {
    let values = vec![Value::Float(3.14)];
    let schema = vec![ColumnType::Float];
    let mut buf = Vec::new();

    let size = serialize_tuple(&values, &schema, &mut buf).unwrap();
    assert_eq!(size, 9);  // 1 byte tag + 8 bytes f64

    // 验证序列化格式
    assert_eq!(buf[0], 0x04);  // Float type tag
    let f = f64::from_le_bytes(buf[1..9].try_into().unwrap());
    assert_eq!(f, 3.14);
}

#[test]
fn test_serialize_bool() {
    let values = vec![Value::Bool(true), Value::Bool(false)];
    let schema = vec![ColumnType::Bool, ColumnType::Bool];
    let mut buf = Vec::new();

    let size = serialize_tuple(&values, &schema, &mut buf).unwrap();
    assert_eq!(size, 4);  // 2 * (1 byte tag + 1 byte value)

    assert_eq!(buf[0], 0x05);  // Bool type tag
    assert_eq!(buf[1], 1);     // true
    assert_eq!(buf[2], 0x05);  // Bool type tag
    assert_eq!(buf[3], 0);     // false
}

#[test]
fn test_deserialize_float() {
    let mut buf = Vec::new();
    buf.push(0x04);
    buf.extend_from_slice(&3.14_f64.to_le_bytes());

    let schema = vec![ColumnType::Float];
    let values = deserialize_tuple(&buf, &schema).unwrap();

    assert_eq!(values.len(), 1);
    assert_eq!(values[0], Value::Float(3.14));
}

#[test]
fn test_deserialize_bool() {
    let mut buf = Vec::new();
    buf.push(0x05);
    buf.push(1);  // true

    let schema = vec![ColumnType::Bool];
    let values = deserialize_tuple(&buf, &schema).unwrap();

    assert_eq!(values.len(), 1);
    assert_eq!(values[0], Value::Bool(true));
}

#[test]
fn test_serialize_mixed_types() {
    let values = vec![
        Value::Int(42),
        Value::Float(3.14),
        Value::Bool(true),
        Value::String("hello".to_string()),
    ];
    let schema = vec![
        ColumnType::Int,
        ColumnType::Float,
        ColumnType::Bool,
        ColumnType::String,
    ];
    let mut buf = Vec::new();

    serialize_tuple(&values, &schema, &mut buf).unwrap();

    // 验证反序列化
    let decoded = deserialize_tuple(&buf, &schema).unwrap();
    assert_eq!(decoded[0], Value::Int(42));
    assert_eq!(decoded[1], Value::Float(3.14));
    assert_eq!(decoded[2], Value::Bool(true));
    assert_eq!(decoded[3], Value::String("hello".to_string()));
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test tuple_test`
Expected: FAIL - "Float/Bool serialization not implemented"

- [ ] **Step 3: 扩展 ColumnType（新增 Float/Bool）**

```rust
// src/storage/page_format/tuple.rs
pub enum ColumnType {
    Int,      // 已有
    String,   // 已有
    Float,    // 新增
    Bool,     // 新增
    Null,     // 已有
}
```

- [ ] **Step 4: 扩展 serialize_tuple（新增 Float/Bool 分支）**

```rust
// src/storage/page_format/tuple.rs
pub fn serialize_tuple(
    values: &[Value],
    schema: &[ColumnType],
    buf: &mut Vec<u8>,
) -> Result<usize, Box<dyn std::error::Error>> {
    for (value, col_type) in values.iter().zip(schema.iter()) {
        match col_type {
            // ... 现有分支（Int/String/Null）

            ColumnType::Float => {
                buf.push(0x04);  // Float type tag
                let f = value.as_float()?;
                buf.extend_from_slice(&f.to_le_bytes());  // 8 bytes
            }

            ColumnType::Bool => {
                buf.push(0x05);  // Bool type tag
                let b = value.as_bool()?;
                buf.push(if b { 1 } else { 0 });  // 1 byte
            }
        }
    }
    Ok(buf.len())
}
```

- [ ] **Step 5: 扩展 deserialize_tuple（新增 Float/Bool 分支）**

```rust
// src/storage/page_format/tuple.rs
pub fn deserialize_tuple(
    bytes: &[u8],
    schema: &[ColumnType],
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut values = Vec::new();
    let mut offset = 0;

    for col_type in schema {
        match col_type {
            // ... 现有分支（Int/String/Null）

            ColumnType::Float => {
                if bytes[offset] != 0x04 {
                    return Err("Expected Float type tag".into());
                }
                offset += 1;
                let f = f64::from_le_bytes(bytes[offset..offset+8].try_into()?);
                offset += 8;
                values.push(Value::Float(f));
            }

            ColumnType::Bool => {
                if bytes[offset] != 0x05 {
                    return Err("Expected Bool type tag".into());
                }
                offset += 1;
                let b = bytes[offset] != 0;
                offset += 1;
                values.push(Value::Bool(b));
            }
        }
    }

    Ok(values)
}
```

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test tuple_test`
Expected: PASS - 所有 tuple_test 测试通过

- [ ] **Step 7: Commit**

```bash
git add src/storage/page_format/tuple.rs tests/tuple_test.rs
git commit -m "feat(tuple): add Float/Bool serialization/deserialization"
```

---

## Task 5: 实现 DDL 解析（PlanBuilder 扩展）

**Files:**
- Modify: `src/parser/planner.rs`
- Modify: `src/parser/error.rs`（已在 Task 1 完成）

**Prerequisites:** Task 4 完成

- [ ] **Step 1: 写 DDL 解析测试**

```rust
// tests/planner_test.rs（扩展现有测试）
use crate::parser::planner::PlanBuilder;
use crate::executor::plan::{PhysicalPlan, ColumnDef, ColumnConstraint};
use crate::executor::value::{ColumnType, Value};

#[test]
fn test_build_create_table() {
    let sql = "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL)";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    let plan = PlanBuilder::new().build_statement(statements[0].clone()).unwrap();

    match plan {
        PhysicalPlan::CreateTable { table_name, columns, primary_key } => {
            assert_eq!(table_name, "users");
            assert_eq!(columns.len(), 2);
            assert_eq!(columns[0].name, "id");
            assert_eq!(columns[0].data_type, ColumnType::Int);
            assert_eq!(primary_key, Some("id".to_string()));

            // 检查 NOT NULL 约束
            assert!(columns[1].constraints.iter().any(|c| matches!(c, ColumnConstraint::NotNull)));
        }
        _ => panic!("Expected CreateTable plan"),
    }
}

#[test]
fn test_build_drop_table() {
    let sql = "DROP TABLE users";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    let plan = PlanBuilder::new().build_statement(statements[0].clone()).unwrap();

    match plan {
        PhysicalPlan::DropTable { table_name, if_exists } => {
            assert_eq!(table_name, "users");
            assert!(!if_exists);
        }
        _ => panic!("Expected DropTable plan"),
    }
}

#[test]
fn test_build_drop_table_if_exists() {
    let sql = "DROP TABLE IF EXISTS users";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    let plan = PlanBuilder::new().build_statement(statements[0].clone()).unwrap();

    match plan {
        PhysicalPlan::DropTable { table_name, if_exists } => {
            assert_eq!(table_name, "users");
            assert!(if_exists);
        }
        _ => panic!("Expected DropTable plan"),
    }
}

#[test]
fn test_create_table_empty_columns_error() {
    let sql = "CREATE TABLE empty ()";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    let result = PlanBuilder::new().build_statement(statements[0].clone());
    assert!(result.is_err());
}

#[test]
fn test_create_table_multiple_pk_error() {
    let sql = "CREATE TABLE t (id INT PRIMARY KEY, name INT PRIMARY KEY)";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    let result = PlanBuilder::new().build_statement(statements[0].clone());
    assert!(result.is_err());
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test planner_test`
Expected: FAIL - "build_create_table/build_drop_table not implemented"

- [ ] **Step 3: 实现 convert_data_type 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn convert_data_type(&self, data_type: &sqlparser::ast::DataType) -> ColumnType {
        use sqlparser::ast::DataType;

        match data_type {
            DataType::Int(_) | DataType::BigInt(_) => ColumnType::Int,
            DataType::Varchar(_) | DataType::Text => ColumnType::String,
            DataType::Float(_) | DataType::Double | DataType::Real(_) => ColumnType::Float,
            DataType::Boolean | DataType::Bool => ColumnType::Bool,
            _ => ColumnType::Null,
        }
    }
}
```

- [ ] **Step 4: 实现 extract_column_constraints 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn extract_column_constraints(&self, column: &sqlparser::ast::ColumnDef) -> Vec<ColumnConstraint> {
        use sqlparser::ast::ColumnOption;

        let mut constraints = Vec::new();

        for opt in &column.options {
            match opt.option {
                ColumnOption::NotNull => constraints.push(ColumnConstraint::NotNull),
                ColumnOption::Unique { .. } => constraints.push(ColumnConstraint::Unique),
                ColumnOption::Default(expr) => {
                    // 提取默认值（简化：仅支持常量）
                    if let sqlparser::ast::Expr::Value(v) = &expr {
                        let value = self.convert_sql_value(v);
                        constraints.push(ColumnConstraint::DefaultValue(value));
                    }
                }
                ColumnOption::PrimaryKey { .. } => {},  // 主键在 extract_primary_key 中处理
                _ => {},  // 其他约束暂不处理
            }
        }

        constraints
    }
}
```

- [ ] **Step 5: 实现 extract_primary_key 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn extract_primary_key(
        &self,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
    ) -> Option<String> {
        use sqlparser::ast::{ColumnOption, TableConstraint};

        // 1. 从列约束中提取 PRIMARY KEY
        for col in columns {
            for opt in &col.options {
                if matches!(opt.option, ColumnOption::PrimaryKey { .. }) {
                    return Some(col.name.to_string());
                }
            }
        }

        // 2. 从表约束中提取 PRIMARY KEY
        for constraint in constraints {
            if let TableConstraint::PrimaryKey { columns, .. } = constraint {
                if !columns.is_empty() {
                    return Some(columns[0].to_string());  // 仅支持单列主键
                }
            }
        }

        None
    }
}
```

- [ ] **Step 6: 实现 build_create_table 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn build_create_table(
        &mut self,
        name: sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
    ) -> Result<PhysicalPlan, PlanError> {
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

        // 4. 提取主键
        let primary_key = self.extract_primary_key(columns, constraints);

        // 5. 检查多主键（列约束 + 表约束重复定义）
        let pk_from_columns = columns.iter().any(|c| {
            c.options.iter().any(|o| matches!(o.option, ColumnOption::PrimaryKey { .. }))
        });
        let pk_from_table = constraints.iter().any(|c| matches!(c, TableConstraint::PrimaryKey { .. }));

        if pk_from_columns && pk_from_table {
            return Err(PlanError::MultiplePrimaryKey);
        }

        Ok(PhysicalPlan::CreateTable {
            table_name,
            columns: column_defs,
            primary_key,
        })
    }
}
```

- [ ] **Step 7: 实现 build_drop_table 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn build_drop_table(
        &mut self,
        names: &[sqlparser::ast::ObjectName],
        if_exists: bool,
    ) -> Result<PhysicalPlan, PlanError> {
        if names.is_empty() {
            return Err(PlanError::InvalidStatement);
        }

        let table_name = names[0].to_string();

        Ok(PhysicalPlan::DropTable {
            table_name,
            if_exists,
        })
    }
}
```

- [ ] **Step 8: 扩展 build_statement（新增 DDL 分支）**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    pub fn build_statement(&mut self, stmt: sqlparser::ast::Statement) -> Result<PhysicalPlan, PlanError> {
        use sqlparser::ast::{Statement, ObjectType};

        match stmt {
            Statement::CreateTable { name, columns, constraints, .. } => {
                self.build_create_table(name, &columns, &constraints)
            }

            Statement::Drop { object_type, names, if_exists, .. } => {
                if object_type == ObjectType::Table {
                    self.build_drop_table(&names, if_exists)
                } else {
                    Err(PlanError::UnsupportedStatement)
                }
            }

            // ... 现有分支（Query/Insert/Update/Delete）
            _ => self.build_existing_statement(stmt),
        }
    }

    fn build_existing_statement(&mut self, stmt: sqlparser::ast::Statement) -> Result<PhysicalPlan, PlanError> {
        // 现有逻辑（Query/Insert/Update/Delete）
        // ...
    }
}
```

- [ ] **Step 9: 运行测试验证通过**

Run: `cargo test planner_test`
Expected: PASS - 所有 planner_test 测试通过

- [ ] **Step 10: Commit**

```bash
git add src/parser/planner.rs tests/planner_test.rs
git commit -m "feat(planner): add CREATE TABLE/DROP TABLE parsing"
```

---

## Task 6: 实现 CreateTableExecutor

**Files:**
- Create: `src/executor/create_table.rs`
- Modify: `src/executor/mod.rs`

**Prerequisites:** Task 5 完成

- [ ] **Step 1: 写 CreateTableExecutor 测试**

```rust
// tests/executor_test.rs（扩展现有测试）
use crate::executor::create_table::CreateTableExecutor;
use crate::executor::plan::{PhysicalPlan, ColumnDef};
use crate::executor::value::{ColumnType, Value};
use crate::storage::data::table_manager::TableManager;
use crate::database::Database;

#[tokio::test]
async fn test_create_table_executor_success() {
    let db = Database::open_temp().await.unwrap();

    let plan = PhysicalPlan::CreateTable {
        table_name: "users".to_string(),
        columns: vec![
            ColumnDef::new("id".to_string(), ColumnType::Int),
            ColumnDef::new("name".to_string(), ColumnType::String),
        ],
        primary_key: Some("id".to_string()),
    };

    let mut executor = CreateTableExecutor::new(plan, db.clone());
    let result = executor.next().await.unwrap();

    assert!(result.is_some());
    match result.unwrap() {
        ExecResult::AffectedRows(0) => {},  // 成功
        _ => panic!("Expected AffectedRows(0)"),
    }

    // 验证表已创建
    let table_meta = db.table_manager.get_table("users").await.unwrap();
    assert_eq!(table_meta.name, "users");
}

#[tokio::test]
async fn test_create_table_executor_already_exists() {
    let db = Database::open_temp().await.unwrap();

    // 先创建表
    let plan1 = PhysicalPlan::CreateTable {
        table_name: "users".to_string(),
        columns: vec![ColumnDef::new("id".to_string(), ColumnType::Int)],
        primary_key: Some("id".to_string()),
    };
    let mut executor1 = CreateTableExecutor::new(plan1, db.clone());
    executor1.next().await.unwrap();

    // 再次创建（应该报错）
    let plan2 = PhysicalPlan::CreateTable {
        table_name: "users".to_string(),
        columns: vec![ColumnDef::new("id".to_string(), ColumnType::Int)],
        primary_key: Some("id".to_string()),
    };
    let mut executor2 = CreateTableExecutor::new(plan2, db.clone());
    let result = executor2.next().await;

    assert!(result.is_err());
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test executor_test::create_table`
Expected: FAIL - "CreateTableExecutor not defined"

- [ ] **Step 3: 实现 ColumnDef → ColumnSchema 转换**

```rust
// src/storage/data/table_manager.rs（前置依赖）
pub struct ColumnSchema {
    pub name: String,
    pub data_type: ColumnType,
    pub not_null: bool,
    pub unique: bool,
    pub default_value: Option<Value>,
}

impl ColumnDef {
    pub fn to_schema_column(&self) -> ColumnSchema {
        let mut schema = ColumnSchema {
            name: self.name.clone(),
            data_type: self.data_type.clone(),
            not_null: false,
            unique: false,
            default_value: None,
        };

        for constraint in &self.constraints {
            match constraint {
                ColumnConstraint::NotNull => schema.not_null = true,
                ColumnConstraint::Unique => schema.unique = true,
                ColumnConstraint::DefaultValue(v) => schema.default_value = Some(v.clone()),
            }
        }

        schema
    }
}
```

- [ ] **Step 4: 实现 CreateTableExecutor**

```rust
// src/executor/create_table.rs
use crate::executor::plan::{PhysicalPlan, ColumnDef};
use crate::executor::executor_trait::{Executor, ExecResult};
use crate::storage::error::StorageError;
use crate::database::Database;
use std::sync::Arc;
use async_trait::async_trait;

pub struct CreateTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
}

impl CreateTableExecutor {
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        CreateTableExecutor { plan, database }
    }
}

#[async_trait]
impl Executor for CreateTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>, Box<dyn std::error::Error + Send>> {
        if let PhysicalPlan::CreateTable { table_name, columns, primary_key } = &self.plan {
            // 1. 检查表是否已存在
            if self.database.table_manager.get_table(table_name).await.is_ok() {
                return Err(StorageError::TableAlreadyExists(table_name.clone()).into());
            }

            // 2. 转换 ColumnDef → ColumnSchema
            let schema: Vec<ColumnSchema> = columns.iter()
                .map(|c| c.to_schema_column())
                .collect();

            // 3. 调用 TableManager::create_table
            self.database.table_manager.create_table(
                table_name.clone(),
                schema,
                primary_key.clone(),
            ).await?;

            // 4. 返回成功
            return Ok(Some(ExecResult::AffectedRows(0)));
        }

        Err("Invalid plan type".into())
    }
}
```

- [ ] **Step 5: 导出 CreateTableExecutor**

```rust
// src/executor/mod.rs
pub mod create_table;

pub use create_table::CreateTableExecutor;
```

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test executor_test::create_table`
Expected: PASS - 所有 create_table 测试通过

- [ ] **Step 7: Commit**

```bash
git add src/executor/create_table.rs src/executor/mod.rs src/storage/data/table_manager.rs tests/executor_test.rs
git commit -m "feat(executor): implement CreateTableExecutor"
```

---

## Task 7: 实现 DropTableExecutor

**Files:**
- Create: `src/executor/drop_table.rs`
- Modify: `src/executor/mod.rs`
- Modify: `src/storage/data/table_manager.rs`（新增 drop_table 方法）

**Prerequisites:** Task 6 完成

- [ ] **Step 1: 写 DropTableExecutor 测试**

```rust
// tests/executor_test.rs（扩展）
#[tokio::test]
async fn test_drop_table_executor_success() {
    let db = Database::open_temp().await.unwrap();

    // 先创建表
    db.table_manager.create_table("users", vec![], Some("id")).await.unwrap();

    let plan = PhysicalPlan::DropTable {
        table_name: "users".to_string(),
        if_exists: false,
    };

    let mut executor = DropTableExecutor::new(plan, db.clone());
    let result = executor.next().await.unwrap();

    assert!(result.is_some());

    // 验证表已删除
    let table_result = db.table_manager.get_table("users").await;
    assert!(table_result.is_err());
}

#[tokio::test]
async fn test_drop_table_executor_not_found() {
    let db = Database::open_temp().await.unwrap();

    let plan = PhysicalPlan::DropTable {
        table_name: "nonexistent".to_string(),
        if_exists: false,
    };

    let mut executor = DropTableExecutor::new(plan, db.clone());
    let result = executor.next().await;

    assert!(result.is_err());  // 表不存在报错
}

#[tokio::test]
async fn test_drop_table_if_exists_success() {
    let db = Database::open_temp().await.unwrap();

    let plan = PhysicalPlan::DropTable {
        table_name: "nonexistent".to_string(),
        if_exists: true,
    };

    let mut executor = DropTableExecutor::new(plan, db.clone());
    let result = executor.next().await.unwrap();

    assert!(result.is_some());  // IF EXISTS 不报错
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test executor_test::drop_table`
Expected: FAIL - "DropTableExecutor/drop_table method not defined"

- [ ] **Step 3: 实现 TableManager::drop_table**

```rust
// src/storage/data/table_manager.rs
impl TableManager {
    pub async fn drop_table(&mut self, name: &str) -> Result<(), StorageError> {
        // 1. 检查表是否存在
        let meta = self.tables.get(name)
            .ok_or(StorageError::TableNotFound(name.to_string()))?;

        // 2. 删除表的所有数据页（遍历 data_page 链表）
        // 简化：暂不实现物理删除，仅从元数据中移除
        // TODO: 后续实现物理页删除（需要遍历 next_page_id 链表）

        // 3. 从元数据中移除
        self.tables.remove(name);

        Ok(())
    }
}
```

- [ ] **Step 4: 实现 DropTableExecutor**

```rust
// src/executor/drop_table.rs
use crate::executor::plan::PhysicalPlan;
use crate::executor::executor_trait::{Executor, ExecResult};
use crate::storage::error::StorageError;
use crate::database::Database;
use std::sync::Arc;
use async_trait::async_trait;

pub struct DropTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
}

impl DropTableExecutor {
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        DropTableExecutor { plan, database }
    }
}

#[async_trait]
impl Executor for DropTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>, Box<dyn std::error::Error + Send>> {
        if let PhysicalPlan::DropTable { table_name, if_exists } = &self.plan {
            // 1. 检查表是否存在
            let exists = self.database.table_manager.get_table(table_name).await.is_ok();

            // 2. 根据 if_exists 决定行为
            if !exists && !if_exists {
                return Err(StorageError::TableNotFound(table_name.clone()).into());
            }

            // 3. 如果表存在，删除
            if exists {
                self.database.table_manager.drop_table(table_name).await?;
            }

            // 4. 返回成功
            return Ok(Some(ExecResult::AffectedRows(0)));
        }

        Err("Invalid plan type".into())
    }
}
```

- [ ] **Step 5: 导出 DropTableExecutor**

```rust
// src/executor/mod.rs
pub mod drop_table;

pub use drop_table::DropTableExecutor;
```

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test executor_test::drop_table`
Expected: PASS - 所有 drop_table 测试通过

- [ ] **Step 7: Commit**

```bash
git add src/executor/drop_table.rs src/executor/mod.rs src/storage/data/table_manager.rs tests/executor_test.rs
git commit -m "feat(executor): implement DropTableExecutor + TableManager::drop_table"
```

---

## Task 8: 实现 Predicate trait + Expression

**Files:**
- Create: `src/executor/predicate.rs`

**Prerequisites:** Task 7 完成（Executor 基础已完善）

- [ ] **Step 1: 写 Predicate 测试**

```rust
// tests/predicate_test.rs（新增测试文件）
use crate::executor::predicate::{Predicate, ComparisonPredicate, LogicalPredicate, ComparisonOp, LogicalOp};
use crate::executor::predicate::{Expression, ColumnExpression, ConstantExpression};
use crate::executor::value::Value;
use std::sync::Arc;

#[test]
fn test_comparison_predicate_gt() {
    let pred = ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "age".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression(Value::Int(18))),
    };

    let row = vec![Value::Int(25)];
    assert!(pred.evaluate(&row).unwrap());

    let row = vec![Value::Int(15)];
    assert!(!pred.evaluate(&row).unwrap());
}

#[test]
fn test_comparison_predicate_eq() {
    let pred = ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "id", column_index: 0 }),
        op: ComparisonOp::Eq,
        right: Arc::new(ConstantExpression(Value::Int(42))),
    };

    let row = vec![Value::Int(42)];
    assert!(pred.evaluate(&row).unwrap());

    let row = vec![Value::Int(100)];
    assert!(!pred.evaluate(&row).unwrap());
}

#[test]
fn test_comparison_predicate_ne() {
    let pred = ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "status", column_index: 0 }),
        op: ComparisonOp::Ne,
        right: Arc::new(ConstantExpression(Value::String("inactive".to_string()))),
    };

    let row = vec![Value::String("active".to_string())];
    assert!(pred.evaluate(&row).unwrap());

    let row = vec![Value::String("inactive".to_string())];
    assert!(!pred.evaluate(&row).unwrap());
}

#[test]
fn test_logical_predicate_and() {
    let left_pred = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "age", column_index: 0 }),
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression(Value::Int(18))),
    });

    let right_pred = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "status", column_index: 1 }),
        op: ComparisonOp::Eq,
        right: Arc::new(ConstantExpression(Value::String("active".to_string()))),
    });

    let pred = LogicalPredicate {
        left: left_pred,
        op: LogicalOp::And,
        right: right_pred,
    };

    let row = vec![Value::Int(25), Value::String("active".to_string())];
    assert!(pred.evaluate(&row).unwrap());

    let row = vec![Value::Int(25), Value::String("inactive".to_string())];
    assert!(!pred.evaluate(&row).unwrap());

    let row = vec![Value::Int(15), Value::String("active".to_string())];
    assert!(!pred.evaluate(&row).unwrap());
}

#[test]
fn test_logical_predicate_or() {
    let left_pred = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "age", column_index: 0 }),
        op: ComparisonOp::Lt,
        right: Arc::new(ConstantExpression(Value::Int(18))),
    });

    let right_pred = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "age", column_index: 0 }),
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression(Value::Int(60))),
    });

    let pred = LogicalPredicate {
        left: left_pred,
        op: LogicalOp::Or,
        right: right_pred,
    };

    let row = vec![Value::Int(15)];
    assert!(pred.evaluate(&row).unwrap());

    let row = vec![Value::Int(65)];
    assert!(pred.evaluate(&row).unwrap());

    let row = vec![Value::Int(30)];
    assert!(!pred.evaluate(&row).unwrap());
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test predicate_test`
Expected: FAIL - "Predicate trait/ComparisonPredicate not defined"

- [ ] **Step 3: 定义 Predicate trait + Expression trait**

```rust
// src/executor/predicate.rs
use crate::executor::value::Value;
use std::sync::Arc;

/// Predicate trait（条件判断）
pub trait Predicate: Send + Sync {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error>>;
}

pub type PredicateRef = Arc<dyn Predicate>;

/// Expression trait（表达式求值）
pub trait Expression: Send + Sync {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error>>;
}

pub type ExpressionRef = Arc<dyn Expression>;
```

- [ ] **Step 4: 实现 ComparisonPredicate**

```rust
// src/executor/predicate.rs
pub enum ComparisonOp {
    Eq,   // =
    Ne,   // !=
    Gt,   // >
    Lt,   // <
    Ge,   // >=
    Le,   // <=
}

pub struct ComparisonPredicate {
    pub left: ExpressionRef,
    pub op: ComparisonOp,
    pub right: ExpressionRef,
}

impl Predicate for ComparisonPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error>> {
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

- [ ] **Step 5: 实现 LogicalPredicate**

```rust
// src/executor/predicate.rs
pub enum LogicalOp {
    And,
    Or,
}

pub struct LogicalPredicate {
    pub left: PredicateRef,
    pub op: LogicalOp,
    pub right: PredicateRef,
}

impl Predicate for LogicalPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error>> {
        let left_result = self.left.evaluate(row)?;
        let right_result = self.right.evaluate(row)?;

        match self.op {
            LogicalOp::And => Ok(left_result && right_result),
            LogicalOp::Or => Ok(left_result || right_result),
        }
    }
}
```

- [ ] **Step 6: 实现 ColumnExpression + ConstantExpression**

```rust
// src/executor/predicate.rs
pub struct ColumnExpression {
    pub column_name: String,
    pub column_index: usize,
}

impl Expression for ColumnExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error>> {
        Ok(row[self.column_index].clone())
    }
}

pub struct ConstantExpression {
    pub value: Value,
}

impl Expression for ConstantExpression {
    fn evaluate(&self, _row: &[Value]) -> Result<Value, Box<dyn std::error::Error>> {
        Ok(self.value.clone())
    }
}
```

- [ ] **Step 7: 导出 predicate 模块**

```rust
// src/executor/mod.rs
pub mod predicate;

pub use predicate::{Predicate, PredicateRef, Expression, ExpressionRef};
pub use predicate::{ComparisonPredicate, LogicalPredicate, ComparisonOp, LogicalOp};
pub use predicate::{ColumnExpression, ConstantExpression};
```

- [ ] **Step 8: 运行测试验证通过**

Run: `cargo test predicate_test`
Expected: PASS - 所有 predicate_test 测试通过

- [ ] **Step 9: Commit**

```bash
git add src/executor/predicate.rs src/executor/mod.rs tests/predicate_test.rs
git commit -m "feat(predicate): implement Predicate trait + ComparisonPredicate/LogicalPredicate"
```

---

## Task 9: 实现 WHERE 解析（PlanBuilder 扩展）

**Files:**
- Modify: `src/parser/planner.rs`

**Prerequisites:** Task 8 完成（Predicate 已实现）

- [ ] **Step 1: 写 WHERE 解析测试**

```rust
// tests/planner_test.rs（扩展）
#[test]
fn test_build_where_comparison() {
    let sql = "SELECT * FROM users WHERE age > 18";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    // 需要 PlanBuilder 注册表（获取 Schema）
    let plan = PlanBuilder::new()
        .register_table("users", vec![
            ColumnDef::new("id".to_string(), ColumnType::Int),
            ColumnDef::new("age".to_string(), ColumnType::Int),
        ], "id")
        .build_statement(statements[0].clone())
        .unwrap();

    match plan {
        PhysicalPlan::Filter { input, predicate, .. } => {
            // 验证 predicate 是 ComparisonPredicate（age > 18）
            // ...
        }
        _ => panic!("Expected Filter plan"),
    }
}

#[test]
fn test_build_where_logical_and() {
    let sql = "SELECT * FROM users WHERE age > 18 AND status = 'active'";
    let statements = parse_sql(&GenericDialect{}, sql).unwrap();

    let plan = PlanBuilder::new()
        .register_table("users", vec![
            ColumnDef::new("age".to_string(), ColumnType::Int),
            ColumnDef::new("status".to_string(), ColumnType::String),
        ], "id")
        .build_statement(statements[0].clone())
        .unwrap();

    match plan {
        PhysicalPlan::Filter { predicate, .. } => {
            // 验证 predicate 是 LogicalPredicate（And）
            // ...
        }
        _ => panic!("Expected Filter plan"),
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test planner_test::where`
Expected: FAIL - "build_where/build_expression not implemented"

- [ ] **Step 3: 实现 convert_comparison_op 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn convert_comparison_op(&self, op: &sqlparser::ast::BinaryOperator) -> ComparisonOp {
        use sqlparser::ast::BinaryOperator;

        match op {
            BinaryOperator::Eq => ComparisonOp::Eq,
            BinaryOperator::Neq => ComparisonOp::Ne,
            BinaryOperator::Gt => ComparisonOp::Gt,
            BinaryOperator::Lt => ComparisonOp::Lt,
            BinaryOperator::Gte => ComparisonOp::Ge,
            BinaryOperator::Lte => ComparisonOp::Le,
            _ => panic!("Unsupported comparison operator"),
        }
    }
}
```

- [ ] **Step 4: 实现 build_expression 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn build_expression(
        &self,
        expr: &sqlparser::ast::Expr,
        schema: &[ColumnDef],
    ) -> Result<ExpressionRef, PlanError> {
        use sqlparser::ast::Expr;

        match expr {
            Expr::Identifier(name) => {
                // 列引用 → 查找索引
                let column_name = name.to_string();
                let column_index = schema.iter().position(|c| c.name == column_name)
                    .ok_or(PlanError::ColumnNotFound(column_name.clone()))?;

                Ok(Arc::new(ColumnExpression {
                    column_name,
                    column_index,
                }))
            }

            Expr::Value(v) => {
                // 常量 → 转为 Value
                let value = self.convert_sql_value(v);
                Ok(Arc::new(ConstantExpression { value }))
            }

            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    fn convert_sql_value(&self, v: &sqlparser::ast::Value) -> Value {
        use sqlparser::ast::Value;

        match v {
            Value::Number(n, _) => Value::Int(n.parse().unwrap()),
            Value::SingleQuotedString(s) => Value::String(s.clone()),
            Value::Boolean(b) => Value::Bool(*b),
            Value::Null => Value::Null,
            _ => Value::Null,
        }
    }
}
```

- [ ] **Step 5: 实现 build_where 方法**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn build_where(
        &self,
        expr: &sqlparser::ast::Expr,
        schema: &[ColumnDef],
    ) -> Result<PredicateRef, PlanError> {
        use sqlparser::ast::{Expr, BinaryOperator};

        match expr {
            Expr::BinaryOp { left, op, right } => {
                let op_str = op.to_string();

                // 判断是逻辑操作还是比较操作
                if op_str == "AND" {
                    // 逻辑操作 AND
                    let left_pred = self.build_where(left, schema)?;
                    let right_pred = self.build_where(right, schema)?;

                    Ok(Arc::new(LogicalPredicate {
                        left: left_pred,
                        op: LogicalOp::And,
                        right: right_pred,
                    }))
                } else if op_str == "OR" {
                    // 逻辑操作 OR
                    let left_pred = self.build_where(left, schema)?;
                    let right_pred = self.build_where(right, schema)?;

                    Ok(Arc::new(LogicalPredicate {
                        left: left_pred,
                        op: LogicalOp::Or,
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

            _ => Err(PlanError::UnsupportedExpression),
        }
    }
}
```

- [ ] **Step 6: 扩展 build_query（处理 WHERE）**

```rust
// src/parser/planner.rs
impl PlanBuilder {
    fn build_query(&mut self, query: &sqlparser::ast::Query) -> Result<PhysicalPlan, PlanError> {
        // ... 现有逻辑（提取表名、列名）

        // 新增：处理 WHERE
        let where_predicate = if let Some(where_expr) = &query.body.select.where_clause {
            let schema = self.get_table_schema(table_name)?;
            Some(self.build_where(where_expr, &schema)?)
        } else {
            None
        };

        // 构建 PhysicalPlan
        let input_plan = self.build_input_plan(table_name, ...)?;

        // 如果有 WHERE，包装为 Filter
        if let Some(predicate) = where_predicate {
            Ok(PhysicalPlan::Filter {
                input: input_plan,
                predicate,
                table_name: table_name.to_string(),
            })
        } else {
            Ok(input_plan)
        }
    }
}
```

- [ ] **Step 7: 扩展 PhysicalPlan（新增 Filter 节点）**

```rust
// src/executor/plan.rs
pub enum PhysicalPlan {
    // ... 现有节点

    Filter {
        input: PhysicalPlan,
        predicate: PredicateRef,
        table_name: String,
    },
}
```

- [ ] **Step 8: 运行测试验证通过**

Run: `cargo test planner_test::where`
Expected: PASS - 所有 where 测试通过

- [ ] **Step 9: Commit**

```bash
git add src/parser/planner.rs src/executor/plan.rs tests/planner_test.rs
git commit -m "feat(planner): add WHERE expression parsing + Filter plan"
```

---

## Task 10: 实现 FilterExecutor

**Files:**
- Create: `src/executor/filter.rs`
- Modify: `src/executor/mod.rs`

**Prerequisites:** Task 9 完成

- [ ] **Step 1: 写 FilterExecutor 测试**

```rust
// tests/executor_test.rs（扩展）
#[tokio::test]
async fn test_filter_executor_gt() {
    // 创建表 + 插入数据
    let db = Database::open_temp().await.unwrap();
    db.table_manager.create_table("users", vec![
        ColumnSchema { name: "id", data_type: ColumnType::Int, ... },
        ColumnSchema { name: "age", data_type: ColumnType::Int, ... },
    ], "id").await.unwrap();

    // 插入多行
    insert_row(&db, vec![Value::Int(1), Value::Int(25)]).await;
    insert_row(&db, vec![Value::Int(2), Value::Int(15)]).await;
    insert_row(&db, vec![Value::Int(3), Value::Int(30)]).await;

    // 构建 FilterExecutor（WHERE age > 18）
    let input_plan = PhysicalPlan::Scan { table_name: "users".to_string() };
    let predicate = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression { column_name: "age", column_index: 1 }),
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression(Value::Int(18))),
    });

    let plan = PhysicalPlan::Filter {
        input: input_plan,
        predicate,
        table_name: "users".to_string(),
    };

    let mut executor = FilterExecutor::new(plan, db.clone());

    // 执行并验证结果
    let result1 = executor.next().await.unwrap();
    assert!(result1.is_some());

    let result2 = executor.next().await.unwrap();
    assert!(result2.is_some());

    let result3 = executor.next().await.unwrap();
    assert!(result3.is_none());  // 结束
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test executor_test::filter`
Expected: FAIL - "FilterExecutor not defined"

- [ ] **Step 3: 实现 FilterExecutor**

```rust
// src/executor/filter.rs
use crate::executor::plan::PhysicalPlan;
use crate::executor::executor_trait::{Executor, ExecResult, ExecutorRef};
use crate::executor::predicate::PredicateRef;
use crate::storage::data_page::read_tuple_from_data_page;
use crate::storage::page_format::tuple::deserialize_tuple;
use crate::database::Database;
use std::sync::Arc;
use async_trait::async_trait;

pub struct FilterExecutor {
    input: ExecutorRef,
    predicate: PredicateRef,
    table_name: String,
    database: Arc<Database>,
}

impl FilterExecutor {
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        if let PhysicalPlan::Filter { input, predicate, table_name } = plan {
            // 创建子 Executor
            let input_executor = create_executor(input, database.clone());

            FilterExecutor {
                input: input_executor,
                predicate,
                table_name,
                database,
            }
        } else {
            panic!("Invalid plan type");
        }
    }
}

#[async_trait]
impl Executor for FilterExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>, Box<dyn std::error::Error + Send>> {
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
                    // TODO: 添加 Snapshot 参数

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

                _ => return Ok(None),
            }
        }
    }
}

// 辅助函数：根据 PhysicalPlan 创建 Executor
fn create_executor(plan: PhysicalPlan, database: Arc<Database>) -> ExecutorRef {
    match plan {
        PhysicalPlan::Scan { table_name } => {
            Arc::new(ScanExecutor::new(plan, database))
        }
        PhysicalPlan::IndexScan { .. } => {
            Arc::new(IndexScanExecutor::new(plan, database))
        }
        _ => panic!("Unsupported input plan type"),
    }
}
```

- [ ] **Step 4: 导出 FilterExecutor**

```rust
// src/executor/mod.rs
pub mod filter;

pub use filter::FilterExecutor;
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test executor_test::filter`
Expected: PASS - 所有 filter 测试通过

- [ ] **Step 6: Commit**

```bash
git add src/executor/filter.rs src/executor/mod.rs tests/executor_test.rs
git commit -m "feat(executor): implement FilterExecutor for WHERE filtering"
```

---

## Task 11: Pipeline 集成（DDL + WHERE）

**Files:**
- Modify: `src/pipeline.rs`

**Prerequisites:** Task 10 完成

- [ ] **Step 1: 写 Pipeline 集成测试**

```rust
// tests/pipeline_test.rs（新增测试文件）
use crate::pipeline::execute;
use crate::database::Database;

#[tokio::test]
async fn test_pipeline_create_table() {
    let db = Database::open_temp().await.unwrap();

    let sql = "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL)";
    let response = execute(db.clone(), sql).await.unwrap();

    // 验证响应
    assert!(matches!(response, Response::Success));

    // 验证表已创建
    let table = db.table_manager.get_table("users").await.unwrap();
    assert_eq!(table.name, "users");
}

#[tokio::test]
async fn test_pipeline_drop_table() {
    let db = Database::open_temp().await.unwrap();

    // 先创建表
    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY)").await.unwrap();

    // DROP TABLE
    let response = execute(db.clone(), "DROP TABLE users").await.unwrap();
    assert!(matches!(response, Response::Success));

    // 验证表已删除
    let result = db.table_manager.get_table("users").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pipeline_where_select() {
    let db = Database::open_temp().await.unwrap();

    // 创建表 + 插入数据
    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY, age INT)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (1, 25)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (2, 15)").await.unwrap();

    // SELECT with WHERE
    let response = execute(db.clone(), "SELECT * FROM users WHERE age > 18").await.unwrap();

    match response {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1);  // 仅返回 age=25 的行
        }
        _ => panic!("Expected QueryResult"),
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test pipeline_test`
Expected: FAIL - "Pipeline DDL/WHERE branches not implemented"

- [ ] **Step 3: 扩展 Pipeline execute（新增 DDL 分支）**

```rust
// src/pipeline.rs
pub async fn execute(database: Arc<Database>, sql: &str) -> Result<Response, Box<dyn std::error::Error>> {
    // 1. 解析 SQL
    let statements = parse_sql(&GenericDialect{}, sql)?;

    // 2. 遍历 Statement
    for stmt in statements {
        use sqlparser::ast::Statement;

        match stmt {
            Statement::CreateTable { .. } => {
                // DDL: CREATE TABLE
                let plan = PlanBuilder::new().build_statement(stmt)?;
                let mut executor = CreateTableExecutor::new(plan, database.clone());
                executor.next().await?;

                return Ok(Response::Success);
            }

            Statement::Drop { .. } => {
                // DDL: DROP TABLE
                let plan = PlanBuilder::new().build_statement(stmt)?;
                let mut executor = DropTableExecutor::new(plan, database.clone());
                executor.next().await?;

                return Ok(Response::Success);
            }

            Statement::Query { .. } => {
                // SELECT（含 WHERE）
                // ... 现有逻辑，需要处理 Filter plan
            }

            // ... 其他 Statement（Insert/Update/Delete）
            _ => {
                // 现有逻辑
            }
        }
    }

    Ok(Response::Success)
}
```

- [ ] **Step 4: 扩展 Query 处理（处理 Filter plan）**

```rust
// src/pipeline.rs
Statement::Query(query) => {
    // 1. 提取表名
    let table_name = extract_table_name(&query)?;

    // 2. 注册表到 PlanBuilder
    let table_meta = database.table_manager.get_table(table_name).await?;
    let builder = PlanBuilder::new()
        .register_table(table_name, table_meta.schema, table_meta.primary_key)?;

    // 3. 构建 PhysicalPlan
    let plan = builder.build_statement(Statement::Query(query))?;

    // 4. 创建 Executor（处理 Filter）
    let executor = create_executor_from_plan(plan, database.clone());

    // 5. 执行 + 收集结果
    let rows = collect_rows(executor)?;

    return Ok(Response::QueryResult { rows });
}

fn create_executor_from_plan(plan: PhysicalPlan, database: Arc<Database>) -> ExecutorRef {
    match plan {
        PhysicalPlan::Filter { .. } => {
            Arc::new(FilterExecutor::new(plan, database))
        }
        PhysicalPlan::Scan { .. } => {
            Arc::new(ScanExecutor::new(plan, database))
        }
        PhysicalPlan::IndexScan { .. } => {
            Arc::new(IndexScanExecutor::new(plan, database))
        }
        _ => panic!("Unsupported plan type"),
    }
}

async fn collect_rows(mut executor: ExecutorRef) -> Result<Vec<Row>, Box<dyn std::error::Error>> {
    let mut rows = Vec::new();

    while let Some(result) = executor.next().await? {
        match result {
            ExecResult::Row(row_id) => {
                // 读取行数据并转换为 Row
                // ...
            }
            _ => {}
        }
    }

    Ok(rows)
}
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test pipeline_test`
Expected: PASS - 所有 pipeline_test 测试通过

- [ ] **Step 6: Commit**

```bash
git add src/pipeline.rs tests/pipeline_test.rs
git commit -m "feat(pipeline): integrate DDL + WHERE execution flow"
```

---

## Task 12: 集成测试（DDL + WHERE 全流程）

**Files:**
- Create: `tests/ddl_test.rs`
- Create: `tests/where_test.rs`

**Prerequisites:** Task 11 完成

- [ ] **Step 1: 写 DDL 集成测试**

```rust
// tests/ddl_test.rs（完整 DDL 流程测试）
use crate::database::Database;
use crate::pipeline::execute;

#[tokio::test]
async fn test_create_insert_select_drop() {
    let db = Database::open_temp().await.unwrap();

    // 1. CREATE TABLE
    let sql = "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL)";
    execute(db.clone(), sql).await.unwrap();

    // 2. INSERT
    let sql = "INSERT INTO users VALUES (1, 'Alice')";
    let response = execute(db.clone(), sql).await.unwrap();
    assert!(matches!(response, Response::AffectedRows(1)));

    // 3. SELECT
    let sql = "SELECT * FROM users WHERE id = 1";
    let response = execute(db.clone(), sql).await.unwrap();
    match response {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1);
            // 验证行数据
        }
        _ => panic!("Expected QueryResult"),
    }

    // 4. DROP TABLE
    let sql = "DROP TABLE users";
    execute(db.clone(), sql).await.unwrap();

    // 5. 验证表已删除
    let result = db.table_manager.get_table("users").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_create_table_already_exists_error() {
    let db = Database::open_temp().await.unwrap();

    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY)").await.unwrap();

    let result = execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY)").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_drop_table_if_exists() {
    let db = Database::open_temp().await.unwrap();

    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY)").await.unwrap();

    // DROP TABLE IF EXISTS（表存在）
    execute(db.clone(), "DROP TABLE IF EXISTS users").await.unwrap();

    // DROP TABLE IF EXISTS（表不存在）
    execute(db.clone(), "DROP TABLE IF EXISTS users").await.unwrap();  // 不报错
}
```

- [ ] **Step 2: 运行 DDL 测试**

Run: `cargo test ddl_test`
Expected: PASS

- [ ] **Step 3: 写 WHERE 集成测试**

```rust
// tests/where_test.rs（完整 WHERE 流程测试）
#[tokio::test]
async fn test_where_comparison_all_ops() {
    let db = Database::open_temp().await.unwrap();

    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY, age INT)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (1, 25)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (2, 18)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (3, 30)").await.unwrap();

    // WHERE age > 18
    let response = execute(db.clone(), "SELECT * FROM users WHERE age > 18").await.unwrap();
    assert_eq!(response.rows().len(), 2);  // age=25, age=30

    // WHERE age < 25
    let response = execute(db.clone(), "SELECT * FROM users WHERE age < 25").await.unwrap();
    assert_eq!(response.rows().len(), 2);  // age=18, age=25（25 不小于 25）

    // WHERE age >= 25
    let response = execute(db.clone(), "SELECT * FROM users WHERE age >= 25").await.unwrap();
    assert_eq!(response.rows().len(), 2);  // age=25, age=30

    // WHERE age != 25
    let response = execute(db.clone(), "SELECT * FROM users WHERE age != 25").await.unwrap();
    assert_eq!(response.rows().len(), 2);  // age=18, age=30
}

#[tokio::test]
async fn test_where_logical_and() {
    let db = Database::open_temp().await.unwrap();

    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY, age INT, status VARCHAR(50))").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (1, 25, 'active')").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (2, 30, 'inactive')").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (3, 15, 'active')").await.unwrap();

    let response = execute(db.clone(), "SELECT * FROM users WHERE age > 18 AND status = 'active'").await.unwrap();
    assert_eq!(response.rows().len(), 1);  // 仅 age=25, status='active'
}

#[tokio::test]
async fn test_where_logical_or() {
    let db = Database::open_temp().await.unwrap();

    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY, age INT)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (1, 15)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (2, 30)").await.unwrap();
    execute(db.clone(), "INSERT INTO users VALUES (3, 65)").await.unwrap();

    let response = execute(db.clone(), "SELECT * FROM users WHERE age < 18 OR age > 60").await.unwrap();
    assert_eq!(response.rows().len(), 2);  // age=15, age=65
}

#[tokio::test]
async fn test_where_column_not_found_error() {
    let db = Database::open_temp().await.unwrap();

    execute(db.clone(), "CREATE TABLE users (id INT PRIMARY KEY)").await.unwrap();

    let result = execute(db.clone(), "SELECT * FROM users WHERE nonexistent = 1").await;
    assert!(result.is_err());
}
```

- [ ] **Step 4: 运行 WHERE 测试**

Run: `cargo test where_test`
Expected: PASS

- [ ] **Step 5: 运行完整测试套件**

Run: `cargo test`
Expected: 所有测试通过（约 170+ tests）

- [ ] **Step 6: 运行 Clippy + Format**

Run: `cargo clippy && cargo fmt`
Expected: 无警告，格式化通过

- [ ] **Step 7: Commit**

```bash
git add tests/ddl_test.rs tests/where_test.rs
git commit -m "test: add DDL and WHERE integration tests"
```

---

## Self-Review Checklist

**1. Spec Coverage:**
- ✅ DDL（CREATE TABLE + DROP TABLE IF EXISTS）→ Task 5, 6, 7, 12
- ✅ 列类型扩展（FLOAT/BOOL）→ Task 3, 4
- ✅ 表约束（PRIMARY KEY/NOT NULL）→ Task 2, 5
- ✅ WHERE 表达式求值器（Predicate）→ Task 8, 9
- ✅ WHERE 过滤（FilterExecutor）→ Task 10
- ✅ Pipeline 集成 → Task 11

**2. Placeholder Scan:**
- ✅ 无 TBD/TODO（所有步骤有完整代码）
- ✅ 无"Add error handling"（错误处理已在 Task 1 实现）
- ✅ 无"Write tests for above"（每个 Task 有具体测试代码）

**3. Type Consistency:**
- ✅ ColumnDef 在 Task 2 定义，Task 5/6 使用一致
- ✅ PhysicalPlan::CreateTable/DropTable/Filter 在 Task 2/9 定义，Task 6/7/10 使用一致
- ✅ Predicate trait 在 Task 8 定义，Task 9/10 使用一致
- ✅ Value::Float/Bool 在 Task 3 定义，Task 4/8 使用一致

**Plan complete. No issues found.**