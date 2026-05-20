# M4 SQL 解析与计划 - 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 SQL 解析与物理计划生成，为 M5 执行引擎提供计划结构

**Architecture:** sqlparser-rs 解析 → PlanBuilder 转换 → PhysicalPlan 静态结构（同步）

**Tech Stack:** Rust, sqlparser-rs 0.44, thiserror

---

## File Structure

```
创建文件：
- src/parser/error.rs         → PlanError 错误类型
- src/parser/value.rs         → Value enum + 转换方法
- src/parser/ast.rs           → sqlparser-rs 辅助函数
- src/parser/planner.rs       → PlanBuilder（AST → PhysicalPlan）
- src/executor/plan.rs        → PhysicalPlan + 5 节点结构

修改文件：
- Cargo.toml                  → 添加 sqlparser 依赖
- src/parser/mod.rs           → 导出新模块
- src/executor/mod.rs         → 导出新模块
- src/lib.rs                  → 导出 executor::plan

测试文件：
- tests/parser_test.rs        → SQL 解析测试
- tests/planner_test.rs       → 计划构建测试
```

---

## Task 1: 添加 sqlparser 依赖

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: 添加 sqlparser 依赖到 Cargo.toml**

```toml
[dependencies]
sqlparser = "0.44"
```

- [ ] **Step 2: 验证依赖可下载**

Run: `cargo fetch`
Expected: 依赖下载成功

- [ ] **Step 3: 提交**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(m4): add sqlparser dependency"
```

---

## Task 2: 实现 PlanError 类型

**Files:**
- Create: `src/parser/error.rs`

- [ ] **Step 1: 创建 error.rs 文件**

```rust
//! Plan building error types

use thiserror::Error;

/// 计划构建错误
#[derive(Debug, Error)]
pub enum PlanError {
    /// 不支持的 SQL 语句类型
    #[error("Unsupported SQL statement type")]
    UnsupportedStatement,

    /// 表不存在
    #[error("Table '{0}' does not exist")]
    TableNotFound(String),

    /// 列不存在
    #[error("Column '{0}' does not exist in table '{1}'")]
    ColumnNotFound(String, String),

    /// WHERE 条件不支持（非主键查询）
    #[error("WHERE clause must be primary key equality: {0}")]
    InvalidWhereClause(String),

    /// SQL 解析错误
    #[error("SQL parse error: {0}")]
    ParseError(String),

    /// 缺少必需字段
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// 不支持的值类型
    #[error("Unsupported value type")]
    UnsupportedValue,
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/parser/error.rs
git commit -m "feat(m4): add PlanError error type"
```

---

## Task 3: 实现 Value 类型

**Files:**
- Create: `src/executor/value.rs`

- [ ] **Step 1: 创建 executor/value.rs 文件**

```rust
//! SQL value types for physical plan

use std::fmt;

/// SQL 值类型
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 整数
    Int(i64),
    /// 字符串
    String(String),
    /// NULL
    Null,
}

impl Value {
    /// 转换为 Key（用于索引查找）
    /// 仅 Int 类型支持，其他类型返回 None
    pub fn to_key(&self) -> Option<Key> {
        match self {
            Value::Int(n) => {
                let bytes = n.to_be_bytes();
                Some(Key::new(&bytes))
            }
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "'{}'", s),
            Value::Null => write!(f, "NULL"),
        }
    }
}
```

注意：此文件需要引用 `Key`，添加 import：

```rust
use crate::storage::page_format::Key;
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/executor/value.rs
git commit -m "feat(m4): add Value type for physical plan"
```

---

## Task 4: 实现 PhysicalPlan 和节点结构

**Files:**
- Create: `src/executor/plan.rs`

- [ ] **Step 1: 创建 executor/plan.rs 文件**

```rust
//! Physical plan types for execution engine

use crate::executor::value::Value;
use crate::storage::page_format::Key;

/// 物理计划节点（同步结构）
#[derive(Debug, Clone)]
pub enum PhysicalPlan {
    /// 全表扫描
    Scan(ScanNode),
    /// 主键索引扫描
    IndexScan(IndexScanNode),
    /// 插入
    Insert(InsertNode),
    /// 更新
    Update(UpdateNode),
    /// 删除
    Delete(DeleteNode),
}

/// 全表扫描节点
#[derive(Debug, Clone)]
pub struct ScanNode {
    /// 表名
    pub table_name: String,
    /// 输出列名列表
    pub columns: Vec<String>,
}

/// 主键索引扫描节点
#[derive(Debug, Clone)]
pub struct IndexScanNode {
    /// 表名
    pub table_name: String,
    /// 主键值（Key）
    pub key: Key,
    /// 输出列名列表
    pub columns: Vec<String>,
}

/// 插入节点
#[derive(Debug, Clone)]
pub struct InsertNode {
    /// 表名
    pub table_name: String,
    /// 列名列表
    pub columns: Vec<String>,
    /// 值列表（每行一组值）
    pub values: Vec<Vec<Value>>,
}

/// 更新节点
#[derive(Debug, Clone)]
pub struct UpdateNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）
    pub key: Key,
    /// 更新列名
    pub column: String,
    /// 新值
    pub new_value: Value,
}

/// 删除节点
#[derive(Debug, Clone)]
pub struct DeleteNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）
    pub key: Key,
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/executor/plan.rs
git commit -m "feat(m4): add PhysicalPlan and node structures"
```

---

## Task 5: 更新 executor/mod.rs 导出

**Files:**
- Modify: `src/executor/mod.rs`

- [ ] **Step 1: 更新 executor/mod.rs**

```rust
//! Execution engine - Physical plan execution, async iterator
//!
//! M4: PhysicalPlan structures
//! M5: Implement async fn next() -> Result<Option<Row>>

mod plan;
mod value;

pub use plan::{PhysicalPlan, ScanNode, IndexScanNode, InsertNode, UpdateNode, DeleteNode};
pub use value::Value;
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/executor/mod.rs
git commit -m "feat(m4): export PhysicalPlan and Value from executor module"
```

---

## Task 6: 实现 parser/value.rs（Value 转换方法）

**Files:**
- Create: `src/parser/value.rs`

- [ ] **Step 1: 创建 parser/value.rs 文件**

此文件重导出 executor::Value 并添加 sqlparser 转换方法：

```rust
//! Value conversion from sqlparser AST

use sqlparser::ast::Value as SqlValue;
use crate::executor::Value;
use crate::parser::error::PlanError;

/// 从 sqlparser Value 转换为内部 Value
pub fn value_from_sqlparser(v: &SqlValue) -> Result<Value, PlanError> {
    match v {
        SqlValue::Number(n, _) => {
            let num: i64 = n.parse()
                .map_err(|_| PlanError::ParseError("Invalid number".into()))?;
            Ok(Value::Int(num))
        }
        SqlValue::SingleQuotedString(s) => {
            Ok(Value::String(s.clone()))
        }
        SqlValue::Null => Ok(Value::Null),
        _ => Err(PlanError::UnsupportedValue),
    }
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/parser/value.rs
git commit -m "feat(m4): add Value conversion from sqlparser"
```

---

## Task 7: 实现 parser/ast.rs（辅助解析函数）

**Files:**
- Create: `src/parser/ast.rs`

- [ ] **Step 1: 创建 parser/ast.rs 文件**

```rust
//! AST helper functions for extracting information from sqlparser AST

use sqlparser::ast::*;
use crate::parser::error::PlanError;

/// 解析 SQL 字符串，返回 Statement 列表
pub fn parse_sql(sql: &str) -> Result<Vec<Statement>, PlanError> {
    sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, sql)
        .map_err(|e| PlanError::ParseError(e.to_string()))
}

/// 从 Query 提取 Select body
pub fn extract_select_body(query: &Query) -> Result<&Select, PlanError> {
    match query.body.as_ref() {
        SetExpr::Select(select) => Ok(select.as_ref()),
        _ => Err(PlanError::UnsupportedStatement),
    }
}

/// 从 FROM 提取表名（仅支持单表）
pub fn extract_table_name(from: &[TableWithJoins]) -> Result<String, PlanError> {
    if from.is_empty() {
        return Err(PlanError::MissingField("FROM clause".into()));
    }
    let table_factor = &from[0].relation;
    match table_factor {
        TableFactor::Table { name, .. } => {
            Ok(name.to_string().to_lowercase())
        }
        _ => Err(PlanError::UnsupportedStatement),
    }
}

/// 从 projection 提取列名列表
pub fn extract_columns(projection: &[SelectItem]) -> Result<Vec<String>, PlanError> {
    projection.iter().map(|item| {
        match item {
            SelectItem::UnnamedExpr(expr) => {
                match expr {
                    Expr::Identifier(ident) => Ok(ident.value.to_string().to_lowercase()),
                    _ => Err(PlanError::UnsupportedStatement),
                }
            }
            SelectItem::Wildcard => Ok("*".into()),
            _ => Err(PlanError::UnsupportedStatement),
        }
    }).collect()
}

/// 从 ObjectName 提取表名
pub fn extract_name_from_object(obj: &ObjectName) -> String {
    obj.to_string().to_lowercase()
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/parser/ast.rs
git commit -m "feat(m4): add AST helper functions for parsing"
```

---

## Task 8: 实现 PlanBuilder（核心逻辑）

**Files:**
- Create: `src/parser/planner.rs`

由于 PlanBuilder 较长，分多个步骤实现。

- [ ] **Step 1: 创建 planner.rs 文件框架**

```rust
//! Plan builder: AST → PhysicalPlan

use std::collections::HashMap;
use sqlparser::ast::*;
use crate::executor::plan::*;
use crate::executor::Value;
use crate::parser::error::PlanError;
use crate::parser::ast::*;
use crate::parser::value::value_from_sqlparser;
use crate::storage::page_format::Key;

/// 计划构建器
pub struct PlanBuilder {
    /// 表名 → 列名列表（元数据）
    tables: HashMap<String, Vec<String>>,
    /// 表名 → 主键列名
    primary_keys: HashMap<String, String>,
}

impl PlanBuilder {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            primary_keys: HashMap::new(),
        }
    }

    /// 注册表元数据
    pub fn register_table(&mut self, name: &str, columns: Vec<String>, pk: &str) {
        let table_name = name.to_lowercase();
        self.tables.insert(table_name.clone(), columns);
        self.primary_keys.insert(table_name, pk.to_lowercase());
    }

    /// 构建物理计划
    pub fn build_plan(&self, stmt: &Statement) -> Result<PhysicalPlan, PlanError> {
        match stmt {
            Statement::Query(query) => self.build_query(query),
            Statement::Insert(insert) => self.build_insert(insert),
            Statement::Update(update) => self.build_update(update),
            Statement::Delete(delete) => self.build_delete(delete),
            _ => Err(PlanError::UnsupportedStatement),
        }
    }

    /// 验证表存在
    fn validate_table(&self, table_name: &str) -> Result<(), PlanError> {
        if self.tables.contains_key(table_name) {
            Ok(())
        } else {
            Err(PlanError::TableNotFound(table_name.into()))
        }
    }
}
```

- [ ] **Step 2: 实现 SELECT 查询构建**

在 impl PlanBuilder 中添加：

```rust
    /// 构建 SELECT 计划
    fn build_query(&self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        let select = extract_select_body(query)?;

        let table_name = extract_table_name(&select.from)?;
        self.validate_table(&table_name)?;

        let columns = extract_columns(&select.projection)?;

        // 处理 WHERE
        if let Some(selection) = &select.selection {
            let key = self.extract_pk_from_where(&table_name, selection)?;
            Ok(PhysicalPlan::IndexScan(IndexScanNode {
                table_name,
                key,
                columns,
            }))
        } else {
            Ok(PhysicalPlan::Scan(ScanNode {
                table_name,
                columns,
            }))
        }
    }

    /// 从 WHERE 提取主键值（仅支持 pk = value）
    fn extract_pk_from_where(&self, table_name: &str, expr: &Expr) -> Result<Key, PlanError> {
        let pk_column = self.primary_keys.get(table_name)
            .ok_or_else(|| PlanError::MissingField("Primary key".into()))?;

        match expr {
            Expr::BinaryOp { left, op, right } => {
                // 仅支持等值比较
                if !matches!(op, BinaryOperator::Eq) {
                    return Err(PlanError::InvalidWhereClause(expr.to_string()));
                }

                // 检查左侧是否是主键列
                let column_name = match left.as_ref() {
                    Expr::Identifier(ident) => ident.value.to_string().to_lowercase(),
                    _ => return Err(PlanError::InvalidWhereClause(expr.to_string())),
                };

                if column_name != *pk_column {
                    return Err(PlanError::InvalidWhereClause(format!(
                        "Expected primary key '{}', got '{}'",
                        pk_column, column_name
                    )));
                }

                // 提取右侧值
                let value = match right.as_ref() {
                    Expr::Value(v) => value_from_sqlparser(v)?,
                    _ => return Err(PlanError::InvalidWhereClause(expr.to_string())),
                };

                // 转换为 Key
                value.to_key()
                    .ok_or_else(|| PlanError::InvalidWhereClause("Value cannot be used as key".into()))
            }
            _ => Err(PlanError::InvalidWhereClause(expr.to_string())),
        }
    }
```

- [ ] **Step 3: 实现 INSERT 构建**

在 impl PlanBuilder 中添加：

```rust
    /// 构建 INSERT 计划
    fn build_insert(&self, insert: &Insert) -> Result<PhysicalPlan, PlanError> {
        let table_name = extract_name_from_object(&insert.table_name);
        self.validate_table(&table_name)?;

        let columns: Vec<String> = insert.columns.iter()
            .map(|c| c.value.to_string().to_lowercase())
            .collect();

        let values = self.extract_insert_values(&insert.source)?;

        Ok(PhysicalPlan::Insert(InsertNode {
            table_name,
            columns,
            values,
        }))
    }

    /// 从 Insert source 提取值列表
    fn extract_insert_values(&self, source: &Query) -> Result<Vec<Vec<Value>>, PlanError> {
        let select = extract_select_body(source)?;

        // 提取 VALUES (...) 格式
        match &select.body {
            SetExpr::Values(values) => {
                values.rows.iter().map(|row| {
                    row.iter().map(|expr| {
                        match expr {
                            Expr::Value(v) => value_from_sqlparser(v),
                            _ => Err(PlanError::UnsupportedValue),
                        }
                    }).collect::<Result<Vec<_>, _>>()
                }).collect()
            }
            _ => Err(PlanError::UnsupportedStatement),
        }
    }
```

- [ ] **Step 4: 实现 UPDATE 构建**

在 impl PlanBuilder 中添加：

```rust
    /// 构建 UPDATE 计划
    fn build_update(&self, update: &Update) -> Result<PhysicalPlan, PlanError> {
        let table_name = extract_name_from_object(&update.table_name);
        self.validate_table(&table_name)?;

        // 仅支持单列更新
        if update.assignments.len() != 1 {
            return Err(PlanError::UnsupportedStatement);
        }

        let assignment = &update.assignments[0];
        let column = assignment.id.value.to_string().to_lowercase();

        let new_value = match &assignment.value {
            Expr::Value(v) => value_from_sqlparser(v)?,
            _ => return Err(PlanError::UnsupportedValue),
        };

        // WHERE 必须是主键查询
        let key = self.extract_pk_from_where(&table_name, &update.selection)?;

        Ok(PhysicalPlan::Update(UpdateNode {
            table_name,
            key,
            column,
            new_value,
        }))
    }
```

- [ ] **Step 5: 实现 DELETE 构建**

在 impl PlanBuilder 中添加：

```rust
    /// 构建 DELETE 计划
    fn build_delete(&self, delete: &Delete) -> Result<PhysicalPlan, PlanError> {
        let table_name = extract_name_from_object(&delete.table_name);
        self.validate_table(&table_name)?;

        // WHERE 必须是主键查询
        let key = self.extract_pk_from_where(&table_name, &delete.selection)?;

        Ok(PhysicalPlan::Delete(DeleteNode {
            table_name,
            key,
        }))
    }
```

- [ ] **Step 6: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 7: 提交**

```bash
git add src/parser/planner.rs
git commit -m "feat(m4): implement PlanBuilder for AST to PhysicalPlan conversion"
```

---

## Task 9: 更新 parser/mod.rs 导出

**Files:**
- Modify: `src/parser/mod.rs`

- [ ] **Step 1: 更新 parser/mod.rs**

```rust
//! SQL parser - Parse SQL to internal representation
//!
//! M4: Integrate sqlparser-rs, build PhysicalPlan

mod ast;
mod error;
mod planner;
mod value;

pub use ast::parse_sql;
pub use error::PlanError;
pub use planner::PlanBuilder;
pub use value::value_from_sqlparser;
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/parser/mod.rs
git commit -m "feat(m4): export parser module components"
```

---

## Task 10: 更新 lib.rs 导出

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: 更新 lib.rs**

```rust
//! RTsql library - Async embedded database components

pub mod executor;
pub mod network;
pub mod parser;
pub mod storage;
pub mod transaction;

// Re-export common types for convenience
pub use executor::{PhysicalPlan, Value};
pub use parser::{parse_sql, PlanBuilder, PlanError};
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: ✅ 编译成功

- [ ] **Step 3: 提交**

```bash
git add src/lib.rs
git commit -m "feat(m4): re-export parser and executor types from lib.rs"
```

---

## Task 11: 编写 parser_test.rs（SQL 解析测试）

**Files:**
- Create: `tests/parser_test.rs`

- [ ] **Step 1: 创建 tests/parser_test.rs**

```rust
//! SQL parsing tests

use rtsql::parse_sql;
use sqlparser::ast::Statement;

#[test]
fn test_parse_select() {
    let sql = "SELECT id, name FROM users";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Query(_)));
}

#[test]
fn test_parse_select_with_where() {
    let sql = "SELECT id, name FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Query(_)));
}

#[test]
fn test_parse_insert() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Insert(_)));
}

#[test]
fn test_parse_update() {
    let sql = "UPDATE users SET name = 'Bob' WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Update(_)));
}

#[test]
fn test_parse_delete() {
    let sql = "DELETE FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Delete(_)));
}

#[test]
fn test_parse_error() {
    let sql = "INVALID SQL";
    let result = parse_sql(sql);
    assert!(result.is_err());
}
```

- [ ] **Step 2: 运行测试验证**

Run: `cargo test parser_test`
Expected: ✅ 6 tests passed

- [ ] **Step 3: 提交**

```bash
git add tests/parser_test.rs
git commit -m "test(m4): add SQL parsing tests"
```

---

## Task 12: 编写 planner_test.rs（计划构建测试）

**Files:**
- Create: `tests/planner_test.rs`

- [ ] **Step 1: 创建 tests/planner_test.rs（SELECT 测试）**

```rust
//! Plan builder tests

use rtsql::{parse_sql, PlanBuilder, PhysicalPlan};

fn setup_builder() -> PlanBuilder {
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id", "name"], "id");
    builder
}

#[test]
fn test_select_by_pk() {
    let sql = "SELECT id, name FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::IndexScan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected IndexScan, got {:?}", plan),
    }
}

#[test]
fn test_select_scan() {
    let sql = "SELECT id, name FROM users";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Scan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected Scan, got {:?}", plan),
    }
}
```

- [ ] **Step 2: 添加 INSERT/UPDATE/DELETE 测试**

追加到 tests/planner_test.rs：

```rust
#[test]
fn test_insert() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
            assert_eq!(node.values.len(), 1);
            assert_eq!(node.values[0].len(), 2);
        }
        _ => panic!("Expected Insert, got {:?}", plan),
    }
}

#[test]
fn test_update() {
    let sql = "UPDATE users SET name = 'Bob' WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Update(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.column, "name");
        }
        _ => panic!("Expected Update, got {:?}", plan),
    }
}

#[test]
fn test_delete() {
    let sql = "DELETE FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Delete(node) => {
            assert_eq!(node.table_name, "users");
        }
        _ => panic!("Expected Delete, got {:?}", plan),
    }
}
```

- [ ] **Step 3: 添加错误场景测试**

追加到 tests/planner_test.rs：

```rust
#[test]
fn test_table_not_found() {
    let sql = "SELECT id FROM unknown_table";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let result = builder.build_plan(&stmts[0]);
    assert!(result.is_err());
}

#[test]
fn test_invalid_where_not_pk() {
    let sql = "SELECT id FROM users WHERE name = 'Alice'";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let result = builder.build_plan(&stmts[0]);
    assert!(result.is_err());
}

#[test]
fn test_unsupported_statement() {
    let sql = "CREATE TABLE test (id INT)";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();

    let result = builder.build_plan(&stmts[0]);
    assert!(result.is_err());
}
```

- [ ] **Step 4: 运行测试验证**

Run: `cargo test planner_test`
Expected: ✅ 8 tests passed

- [ ] **Step 5: 提交**

```bash
git add tests/planner_test.rs
git commit -m "test(m4): add plan builder tests"
```

---

## Task 13: 运行完整测试并验证

- [ ] **Step 1: 运行所有测试**

Run: `cargo test`
Expected: ✅ 所有测试通过（包括之前的 78 个 M3 测试）

- [ ] **Step 2: 运行 clippy**

Run: `cargo clippy`
Expected: ✅ 无 Critical warnings（可能有 Minor warnings）

- [ ] **Step 3: 运行格式检查**

Run: `cargo fmt --check`
Expected: ✅ 无格式问题（如有则运行 `cargo fmt`）

- [ ] **Step 4: 最终提交（如有未提交的改动）**

```bash
git status
# 如有未提交改动，提交
git add -A
git commit -m "feat(m4): complete SQL parser and plan implementation"
```

---

## Verification Summary

| 验证项 | 命令 | 期望 |
|--------|------|------|
| 编译 | `cargo build` | ✅ 0 errors |
| 测试 | `cargo test` | ✅ 78 + 14 新测试通过 |
| Clippy | `cargo clippy` | ✅ 无 Critical warnings |
| Format | `cargo fmt --check` | ✅ 无格式问题 |

---

## Success Criteria

1. ✅ sqlparser-rs 成功解析 DML SQL
2. ✅ PlanBuilder 正确构建 PhysicalPlan
3. ✅ 语义验证（表名/列名/主键）生效
4. ✅ 所有测试通过
5. ✅ PhysicalPlan 结构可供 M5 执行引擎使用