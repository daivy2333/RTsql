//! PlanBuilder - Convert AST to PhysicalPlan
//!
//! M4: SQL Parser and Physical Plan
//!
//! MS07-T03: split into `mod.rs` (struct + core + build_plan dispatcher) and
//! five sub-modules by responsibility:
//!   - `query`      — SELECT / FROM / JOIN / projection / PK-equality
//!   - `expression` — `build_expression` / `build_where` / `resolve_column_ref`
//!   - `aggregate`  — `build_having` + aggregate helpers
//!   - `subquery`   — IN/EXISTS subquery, correlated parameters
//!   - `ddl_dml`    — INSERT / UPDATE / DELETE / CREATE TABLE / DROP TABLE

mod aggregate;
mod ddl_dml;
mod expression;
mod query;
mod subquery;

use crate::executor::PhysicalPlan;
use crate::executor::Value;
use crate::parser::error::PlanError;
use crate::storage::page_format::ColumnType;
use sqlparser::ast::ObjectType;
use sqlparser::ast::Statement;
use std::collections::HashMap;

/// Transaction statement kinds recognized on the SQL surface (MS11-T02).
///
/// Shared by `build_plan` (non-session rejection, R4) and the CLI session
/// dispatcher (Iteration 001, R1/R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatementKind {
    Begin,
    Commit,
    Rollback,
}

/// Classify a parsed statement as a transaction statement (design D2).
///
/// `Ok(None)` — not a transaction statement; `Ok(Some(kind))` — a clean
/// `BEGIN`/`COMMIT`/`ROLLBACK` (no modes/modifier/chain/savepoint); `Err` —
/// a boundary clause that is explicitly rejected, carrying the design D3
/// named message. Note sqlparser's `Commit`/`Rollback` `chain: bool` cannot
/// distinguish explicit `AND NO CHAIN` from absence, so both classify as the
/// clean statement.
pub(crate) fn classify_transaction_statement(
    stmt: &Statement,
) -> Result<Option<TxStatementKind>, PlanError> {
    match stmt {
        Statement::StartTransaction {
            modes, modifier, ..
        } => {
            if modes.is_empty() && modifier.is_none() {
                Ok(Some(TxStatementKind::Begin))
            } else {
                Err(PlanError::TransactionStatement(
                    "transaction modes in BEGIN/START TRANSACTION are not supported".to_string(),
                ))
            }
        }
        Statement::Commit { chain } => {
            if *chain {
                Err(PlanError::TransactionStatement(
                    "COMMIT AND CHAIN is not supported".to_string(),
                ))
            } else {
                Ok(Some(TxStatementKind::Commit))
            }
        }
        Statement::Rollback { chain, savepoint } => {
            if savepoint.is_some() {
                Err(PlanError::TransactionStatement(
                    "ROLLBACK TO SAVEPOINT is not supported".to_string(),
                ))
            } else if *chain {
                Err(PlanError::TransactionStatement(
                    "ROLLBACK AND CHAIN is not supported".to_string(),
                ))
            } else {
                Ok(Some(TxStatementKind::Rollback))
            }
        }
        Statement::SetTransaction { .. } => Err(PlanError::TransactionStatement(
            "SET TRANSACTION is not supported".to_string(),
        )),
        Statement::Savepoint { .. } => Err(PlanError::TransactionStatement(
            "SAVEPOINT is not supported".to_string(),
        )),
        Statement::ReleaseSavepoint { .. } => Err(PlanError::TransactionStatement(
            "RELEASE SAVEPOINT is not supported".to_string(),
        )),
        _ => Ok(None),
    }
}

/// PlanBuilder - Convert AST to PhysicalPlan
///
/// Stores table metadata (columns, primary keys) for validation and plan generation.
#[derive(Debug, Clone)]
pub struct PlanBuilder {
    /// Table name -> column names
    pub(crate) tables: HashMap<String, Vec<String>>,
    /// Table name -> primary key column name
    pub(crate) primary_keys: HashMap<String, String>,
    /// MS16 Iteration 000 (I046): Table name -> key column declared type (design D2 additive
    /// channel). Real tables are passed through at the pipeline registration point; derived table aliases
    /// (pk="") are not registered — unknown type = falls back to existing routing (only real
    /// tables can reach key-position equality, downgrade is unreachable).
    pub(crate) primary_key_types: HashMap<String, ColumnType>,
    /// MS24 Iteration 000 (D1): Table name -> per-column declared DEFAULT
    /// literals (index-aligned with the registered column list; `None` = no
    /// default). Additive channel mirroring `set_pk_column_type`; consumed by
    /// `map_insert_values` for subset-INSERT / DEFAULT-keyword fill.
    pub(crate) table_defaults: HashMap<String, Vec<Option<Value>>>,
    /// MS24 Iteration 001 (D4): Table name -> columns carrying a UNIQUE
    /// index (ascending column position, the same order the catalog persists
    /// unique roots in). Additive channel consumed by the `ON CONFLICT`
    /// conflict-target resolution — a target column is arbitrable only if it
    /// is an INT declared PK or listed here.
    pub(crate) table_unique_columns: HashMap<String, Vec<usize>>,
    /// Set of inner table names when building a subquery (for detecting outer references).
    /// None when building a top-level query.
    pub(crate) inner_table_names: Option<Vec<String>>,
    /// MS11-T01 Iter001: 正在构建子查询计划（WHERE IN/EXISTS、标量子查询、
    /// 派生表）。子查询计划形状是 SemiJoin/SubqueryEval/DerivedScan 机制的
    /// 消费面——SELECT 表达式项的顶层 Projection 路由在子查询上下文抑制，
    /// 子查询计划保持既有行为。
    pub(crate) building_subquery: bool,
    /// MS09-T02 (I015): NLJ 组合行列布局（design D5a 加性通道）。Some 时
    /// `build_expression` 两个列解析臂优先消费——列引用解析为组合行绝对
    /// 索引（前序表偏移累计 + 表内位置）。仅在 NLJ 分支编译 ON 期间置位
    ///（save/restore 严格配对），None 时既有路径行为逐字节不变。
    pub(crate) join_column_layout: Option<Vec<(String, Vec<String>)>>,
}

impl PlanBuilder {
    /// Create empty PlanBuilder
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            primary_keys: HashMap::new(),
            primary_key_types: HashMap::new(),
            table_defaults: HashMap::new(),
            table_unique_columns: HashMap::new(),
            inner_table_names: None,
            building_subquery: false,
            join_column_layout: None,
        }
    }

    /// Register table metadata
    pub fn register_table(&mut self, name: &str, columns: Vec<String>, pk: &str) {
        let name_lower = name.to_lowercase();
        self.tables.insert(name_lower.clone(), columns);
        self.primary_keys.insert(name_lower, pk.to_lowercase());
    }

    /// MS16 Iteration 000 (I046): Register the key column's declared type (design D2 additive channel —
    /// `register_table` signature unchanged; the key is lowercase, consistent with `register_table`).
    pub fn set_pk_column_type(&mut self, name: &str, ct: ColumnType) {
        self.primary_key_types.insert(name.to_lowercase(), ct);
    }

    /// MS24 Iteration 000 (D1): Register the table's per-column declared
    /// DEFAULT literals (additive channel — `register_table` signature
    /// unchanged; the key is lowercase, consistent with `register_table`).
    /// The vector is index-aligned with the registered column list.
    pub fn set_table_defaults(&mut self, name: &str, defaults: Vec<Option<Value>>) {
        self.table_defaults.insert(name.to_lowercase(), defaults);
    }

    /// MS24 Iteration 001 (D4): Register the columns carrying a UNIQUE index
    /// (additive channel — `register_table` signature unchanged; the key is
    /// lowercase, consistent with `register_table`). The vector holds column
    /// positions in ascending order.
    pub fn set_table_unique_columns(&mut self, name: &str, columns: Vec<usize>) {
        self.table_unique_columns
            .insert(name.to_lowercase(), columns);
    }

    /// Build PhysicalPlan from Statement
    pub fn build_plan(&mut self, stmt: &Statement) -> Result<PhysicalPlan, PlanError> {
        // MS11-T02: transaction statements have no executor semantics on any
        // statement path — reject precisely before planning (boundary clauses
        // propagate their D3 message; clean statements get the session-only
        // message). They never reach the plan cache either way.
        if classify_transaction_statement(stmt)?.is_some() {
            return Err(PlanError::TransactionStatement(
                "transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session".to_string(),
            ));
        }
        match stmt {
            Statement::Query(query) => self.build_query(query),
            Statement::Insert {
                table_name,
                columns,
                source,
                on,
                replace_into,
                ..
            } => self.build_insert(table_name, columns, source, on, *replace_into),
            Statement::Update {
                table,
                assignments,
                selection,
                ..
            } => self.build_update(table, assignments, selection),
            Statement::Delete {
                from, selection, ..
            } => self.build_delete(from, selection),
            Statement::CreateTable {
                name,
                columns,
                constraints,
                ..
            } => self.build_create_table(name, columns, constraints),
            Statement::Drop {
                object_type,
                if_exists,
                names,
                ..
            } => {
                if *object_type == ObjectType::Table {
                    self.build_drop_table(names, if_exists)
                } else {
                    Err(PlanError::UnsupportedStatement)
                }
            }
            _ => Err(PlanError::UnsupportedStatement),
        }
    }

    /// Validate table exists
    fn validate_table(&self, table_name: &str) -> Result<(), PlanError> {
        let name_lower = table_name.to_lowercase();
        if self.tables.contains_key(&name_lower) {
            Ok(())
        } else {
            Err(PlanError::ParseError(format!(
                "Table '{}' does not exist",
                table_name
            )))
        }
    }
}

impl Default for PlanBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_builder_new() {
        let builder = PlanBuilder::new();
        assert!(builder.tables.is_empty());
        assert!(builder.primary_keys.is_empty());
    }

    #[test]
    fn test_register_table() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        assert!(builder.tables.contains_key("users"));
        assert!(builder.primary_keys.contains_key("users"));
        assert_eq!(builder.primary_keys.get("users"), Some(&"id".to_string()));
    }

    #[test]
    fn test_validate_table() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into()], "id");

        assert!(builder.validate_table("users").is_ok());
        assert!(builder.validate_table("nonexistent").is_err());
    }
}

#[cfg(test)]
mod tx_statement_tests {
    use super::*;
    use crate::parser::parse_sql;

    fn classify(sql: &str) -> Result<Option<TxStatementKind>, PlanError> {
        let statements = parse_sql(sql).expect("parse must succeed");
        assert_eq!(statements.len(), 1);
        classify_transaction_statement(&statements[0])
    }

    #[test]
    fn clean_transaction_statements_classify_to_kind() {
        for (sql, expected) in [
            ("BEGIN", TxStatementKind::Begin),
            ("BEGIN TRANSACTION", TxStatementKind::Begin),
            ("START TRANSACTION", TxStatementKind::Begin),
            ("COMMIT", TxStatementKind::Commit),
            ("ROLLBACK", TxStatementKind::Rollback),
        ] {
            assert_eq!(classify(sql).unwrap(), Some(expected), "{sql}");
        }
    }

    #[test]
    fn boundary_clauses_rejected_with_named_message() {
        for (sql, message) in [
            (
                "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
                "SET TRANSACTION is not supported",
            ),
            ("SAVEPOINT sp1", "SAVEPOINT is not supported"),
            (
                "RELEASE SAVEPOINT sp1",
                "RELEASE SAVEPOINT is not supported",
            ),
            (
                "BEGIN ISOLATION LEVEL SERIALIZABLE",
                "transaction modes in BEGIN/START TRANSACTION are not supported",
            ),
            (
                "START TRANSACTION READ ONLY",
                "transaction modes in BEGIN/START TRANSACTION are not supported",
            ),
            ("COMMIT AND CHAIN", "COMMIT AND CHAIN is not supported"),
            ("ROLLBACK AND CHAIN", "ROLLBACK AND CHAIN is not supported"),
            (
                "ROLLBACK TO SAVEPOINT sp1",
                "ROLLBACK TO SAVEPOINT is not supported",
            ),
        ] {
            match classify(sql) {
                Err(PlanError::TransactionStatement(msg)) => {
                    assert_eq!(msg, message, "{sql}");
                }
                other => panic!("{sql}: expected TransactionStatement, got {:?}", other),
            }
        }
    }

    #[test]
    fn non_transaction_statements_classify_to_none() {
        for sql in [
            "SELECT 1",
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET id = 1",
            "DELETE FROM t",
            "CREATE TABLE t (id INT)",
            "DROP TABLE t",
        ] {
            assert_eq!(classify(sql).unwrap(), None, "{sql}");
        }
    }

    #[test]
    fn build_plan_rejects_transaction_statements_with_session_message() {
        for sql in ["BEGIN", "COMMIT", "ROLLBACK"] {
            let statements = parse_sql(sql).unwrap();
            let mut builder = PlanBuilder::new();
            match builder.build_plan(&statements[0]) {
                Err(PlanError::TransactionStatement(msg)) => assert_eq!(
                    msg,
                    "transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session",
                    "{sql}"
                ),
                other => panic!("{sql}: expected TransactionStatement, got {:?}", other),
            }
        }
    }
}
