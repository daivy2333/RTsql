# M4 SQL 解析与计划 - 设计规范

> 日期：2026-05-20
> 里程碑：M4
> 状态：Draft → Review

---

## 1. 概述

### 1.1 目标

M4 实现 SQL 解析与物理计划生成，为 M5 异步执行引擎提供计划结构。

### 1.2 范围

| 项目 | 决策 |
|------|------|
| SQL 类型 | DML Only（INSERT/UPDATE/DELETE/SELECT） |
| 查询范围 | 单表 + 主键查询 |
| WHERE 支持 | 仅主键等值查询（`WHERE id = 1`） |
| JOIN 支持 | 无（推迟到 M5/M6） |
| 验证方式 | 计划结构验证（不执行） |

### 1.3 架构决策

采用 `sqlparser-rs + 直接物理计划` 方案：

```
SQL → sqlparser-rs AST → PhysicalPlan（直接映射）
```

**优点**：
- 实现简单，跳过逻辑计划中间层
- 与 M2 BTree/M3 Transaction 直接对接
- M5 执行引擎只需遍历计划节点调用 IndexManager

**推迟内容**：
- 逻辑计划层（优化空间，推迟到 M7）
- 复杂 WHERE（表达式计算，推迟到 M5）
- JOIN（多表，推迟到 M5/M6）

---

## 2. 架构图

```
┌─────────────────────────────────────────────────────────────┐
│                      M4 Architecture                         │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   SQL String                                                 │
│       │                                                      │
│       ▼                                                      │
│   ┌─────────────────┐                                        │
│   │ sqlparser-rs    │  ← 同步解析，返回 AST                  │
│   │ parse()         │                                        │
│   └─────────────────┘                                        │
│       │                                                      │
│       ▼ Statement (AST)                                      │
│   ┌─────────────────┐                                        │
│   │ PlanBuilder     │  ← 同步转换，AST → PhysicalPlan        │
│   │ build_plan()    │                                        │
│   └─────────────────┘                                        │
│       │                                                      │
│       ▼ PhysicalPlan                                         │
│   ┌─────────────────┐                                        │
│   │ PhysicalPlan    │  ← 同步结构，M5 异步执行               │
│   │ enum            │                                        │
│   └─────────────────┘                                        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. 模块结构

```
src/parser/
├── mod.rs           # 模块导出：Parser, PlanBuilder, Value, PlanError
├── ast.rs           # sqlparser-rs 类型重导出 + 辅助函数
├── value.rs         # Value enum（Int/String/Null）
├── planner.rs       # PlanBuilder（AST → PhysicalPlan）
└── error.rs         # PlanError 错误类型

src/executor/
├── mod.rs           # 模块导出：PhysicalPlan, 各节点结构
├── plan.rs          # PhysicalPlan enum + 节点结构
└── value.rs         # Value enum（与 parser 共享）
```

---

## 4. PhysicalPlan 结构设计

### 4.1 PhysicalPlan Enum

```rust
// src/executor/plan.rs

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
```

### 4.2 节点结构

#### ScanNode（全表扫描）

```rust
#[derive(Debug, Clone)]
pub struct ScanNode {
    /// 表名
    pub table_name: String,
    /// 输出列名列表
    pub columns: Vec<String>,
}
```

#### IndexScanNode（主键查询）

```rust
#[derive(Debug, Clone)]
pub struct IndexScanNode {
    /// 表名
    pub table_name: String,
    /// 主键值（Key）
    pub key: Key,
    /// 输出列名列表
    pub columns: Vec<String>,
}
```

#### InsertNode

```rust
#[derive(Debug, Clone)]
pub struct InsertNode {
    /// 表名
    pub table_name: String,
    /// 列名列表
    pub columns: Vec<String>,
    /// 值列表（每行一组值）
    pub values: Vec<Vec<Value>>,
}
```

#### UpdateNode

```rust
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
```

#### DeleteNode

```rust
#[derive(Debug, Clone)]
pub struct DeleteNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）
    pub key: Key,
}
```

---

## 5. Value 类型设计

```rust
// src/parser/value.rs

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
    /// 从 sqlparser-rs Value 转换
    pub fn from_sqlparser(v: &sqlparser::ast::Value) -> Result<Self, PlanError> {
        match v {
            sqlparser::ast::Value::Number(n, _) => {
                let num: i64 = n.parse()
                    .map_err(|_| PlanError::ParseError("Invalid number".into()))?;
                Ok(Value::Int(num))
            }
            sqlparser::ast::Value::SingleQuotedString(s) => {
                Ok(Value::String(s.clone()))
            }
            sqlparser::ast::Value::Null => Ok(Value::Null),
            _ => Err(PlanError::UnsupportedValue),
        }
    }
}
```

---

## 6. PlanBuilder 实现

### 6.1 结构

```rust
// src/parser/planner.rs

use std::collections::HashMap;
use sqlparser::ast::{Statement, Query, Select, Insert, Update, Delete};
use crate::executor::plan::{PhysicalPlan, ScanNode, IndexScanNode, InsertNode, UpdateNode, DeleteNode};
use crate::parser::error::PlanError;
use crate::storage::page_format::Key;

/// 计划构建器
pub struct PlanBuilder {
    /// 表名 → 列名列表（元数据）
    tables: HashMap<String, Vec<String>>,
    /// 主键列名映射
    primary_keys: HashMap<String, String>,
}
```

### 6.2 核心方法

```rust
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
}
```

### 6.3 SELECT 处理

```rust
impl PlanBuilder {
    fn build_query(&self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        let select = Self::extract_select_body(query)?;

        // 提取表名
        let table_name = Self::extract_table_name(&select.from)?;
        self.validate_table(&table_name)?;

        // 提取列
        let columns = Self::extract_columns(&select.projection)?;

        // 处理 WHERE
        if let Some(selection) = &select.selection {
            // 仅支持主键等值查询
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
}
```

### 6.4 INSERT 处理

```rust
impl PlanBuilder {
    fn build_insert(&self, insert: &Insert) -> Result<PhysicalPlan, PlanError> {
        let table_name = Self::normalize_table_name(&insert.table_name)?;
        self.validate_table(&table_name)?;

        let columns = insert.columns.iter()
            .map(|c| c.value.to_string().to_lowercase())
            .collect();

        let values = Self::extract_insert_values(&insert.source)?;

        Ok(PhysicalPlan::Insert(InsertNode {
            table_name,
            columns,
            values,
        }))
    }
}
```

---

## 7. 错误类型设计

```rust
// src/parser/error.rs

/// 计划构建错误
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("Unsupported SQL statement type")]
    UnsupportedStatement,

    #[error("Table '{0}' does not exist")]
    TableNotFound(String),

    #[error("Column '{0}' does not exist in table '{1}'")]
    ColumnNotFound(String, String),

    #[error("WHERE clause must be primary key equality: {0} = value")]
    InvalidWhereClause(String),

    #[error("SQL parse error: {0}")]
    ParseError(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Unsupported value type")]
    UnsupportedValue,
}
```

---

## 8. 辅助解析函数

```rust
// src/parser/ast.rs

use sqlparser::ast::*;
use crate::parser::error::PlanError;

/// 解析 SQL 字符串
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

/// 从 FROM 提取表名
pub fn extract_table_name(from: &[TableWithJoins]) -> Result<String, PlanError> {
    if from.is_empty() {
        return Err(PlanError::MissingField("FROM clause"));
    }
    let table_factor = &from[0].relation;
    match table_factor {
        TableFactor::Table { name, .. } => {
            Ok(name.to_string().to_lowercase())
        }
        _ => Err(PlanError::UnsupportedStatement),
    }
}

/// 从 projection 提取列名
pub fn extract_columns(projection: &[SelectItem]) -> Result<Vec<String>, PlanError> {
    projection.iter().map(|item| {
        match item {
            SelectItem::UnnamedExpr(expr) => {
                match expr {
                    Expr::Identifier(ident) => Ok(ident.value.to_string().to_lowercase()),
                    _ => Err(PlanError::UnsupportedStatement),
                }
            }
            SelectItem::Wildcard => Ok("*".to_string()), // 特殊处理
            _ => Err(PlanError::UnsupportedStatement),
        }
    }).collect()
}
```

---

## 9. 测试设计

### 9.1 测试文件结构

```
tests/
├── parser_test.rs      # SQL 解析测试
├── planner_test.rs     # 计划构建测试
└── value_test.rs       # Value 类型测试
```

### 9.2 测试用例

```rust
// tests/planner_test.rs

use rtsql::parser::{parse_sql, PlanBuilder};
use rtsql::executor::plan::PhysicalPlan;

#[test]
fn test_select_by_pk() {
    let sql = "SELECT id, name FROM users WHERE id = 1";
    let stmt = parse_sql(sql).unwrap()[0];
    
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id", "name"], "id");
    
    let plan = builder.build_plan(&stmt).unwrap();
    
    match plan {
        PhysicalPlan::IndexScan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected IndexScan"),
    }
}

#[test]
fn test_select_scan() {
    let sql = "SELECT id, name FROM users";
    let stmt = parse_sql(sql).unwrap()[0];
    
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id", "name"], "id");
    
    let plan = builder.build_plan(&stmt).unwrap();
    
    match plan {
        PhysicalPlan::Scan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected Scan"),
    }
}

#[test]
fn test_insert() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
    let stmt = parse_sql(sql).unwrap()[0];
    
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id", "name"], "id");
    
    let plan = builder.build_plan(&stmt).unwrap();
    
    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
            assert_eq!(node.values.len(), 1);
        }
        _ => panic!("Expected Insert"),
    }
}

#[test]
fn test_table_not_found() {
    let sql = "SELECT * FROM unknown";
    let stmt = parse_sql(sql).unwrap()[0];
    
    let builder = PlanBuilder::new();
    let result = builder.build_plan(&stmt);
    
    assert!(result.is_err());
}

#[test]
fn test_invalid_where() {
    let sql = "SELECT id FROM users WHERE name = 'Alice'";
    let stmt = parse_sql(sql).unwrap()[0];
    
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id", "name"], "id");
    
    let result = builder.build_plan(&stmt);
    
    // name 不是主键，应返回错误
    assert!(result.is_err());
}
```

---

## 10. 依赖更新

### 10.1 Cargo.toml

```toml
[dependencies]
sqlparser = "0.44"
```

---

## 11. 与 M2/M3 集成

### 11.1 计划结构预留接口

PhysicalPlan 节点结构已预留 M2/M3 集成所需字段：

| 节点 | M2/M3 集成 |
|------|-----------|
| IndexScanNode | `key: Key` → `IndexManager.get()` |
| InsertNode | `values: Vec<Value>` → `IndexManager.insert()` + `TransactionManager.begin()` |
| UpdateNode | `key: Key` → `IndexManager.get()` + `TransactionManager` |
| DeleteNode | `key: Key` → `IndexManager.delete()` + `TransactionManager` |

### 11.2 M5 执行引擎接口（预览）

```rust
// M5 将实现：
async fn execute_plan(
    plan: &PhysicalPlan,
    index_manager: &IndexManager,
    tx_manager: &TransactionManager,
) -> Result<ExecutionResult, ExecutionError> {
    match plan {
        PhysicalPlan::IndexScan(node) => {
            // 调用 IndexManager.get(&node.key)
        }
        PhysicalPlan::Insert(node) => {
            // 调用 tx_manager.begin()
            // 调用 index_manager.insert()
            // 调用 tx_manager.commit()
        }
        // ...
    }
}
```

---

## 12. 验证标准

| 验证项 | 命令 | 期望 |
|--------|------|------|
| 编译 | `cargo build` | ✅ 0 errors |
| 测试 | `cargo test` | ✅ 所有测试通过 |
| Clippy | `cargo clippy` | ✅ 无 Critical warnings |
| Format | `cargo fmt --check` | ✅ 无格式问题 |

---

## 13. 推迟内容

| 推迟项 | 原因 | 目标里程碑 |
|--------|------|-----------|
| 逻辑计划层 | 单表主键无需优化 | M7 |
| 复杂 WHERE | 需表达式计算 | M5 |
| JOIN | 需多表计划和执行逻辑 | M5/M6 |
| DDL (CREATE/DROP) | 需元数据管理 | 后续里程碑 |
| 聚合函数 | 需聚合算子 | M5 |

---

## 14. 成功标准

1. ✅ sqlparser-rs 成功解析 DML SQL
2. ✅ PlanBuilder 正确构建 PhysicalPlan
3. ✅ 语义验证（表名/列名/主键）生效
4. ✅ 所有测试通过
5. ✅ PhysicalPlan 结构可供 M5 执行引擎使用