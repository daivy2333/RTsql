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
use crate::parser::error::PlanError;
use sqlparser::ast::ObjectType;
use sqlparser::ast::Statement;
use std::collections::HashMap;

/// PlanBuilder - Convert AST to PhysicalPlan
///
/// Stores table metadata (columns, primary keys) for validation and plan generation.
#[derive(Debug, Clone)]
pub struct PlanBuilder {
    /// Table name -> column names
    pub(crate) tables: HashMap<String, Vec<String>>,
    /// Table name -> primary key column name
    pub(crate) primary_keys: HashMap<String, String>,
    /// Set of inner table names when building a subquery (for detecting outer references).
    /// None when building a top-level query.
    pub(crate) inner_table_names: Option<Vec<String>>,
}

impl PlanBuilder {
    /// Create empty PlanBuilder
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            primary_keys: HashMap::new(),
            inner_table_names: None,
        }
    }

    /// Register table metadata
    pub fn register_table(&mut self, name: &str, columns: Vec<String>, pk: &str) {
        let name_lower = name.to_lowercase();
        self.tables.insert(name_lower.clone(), columns);
        self.primary_keys.insert(name_lower, pk.to_lowercase());
    }

    /// Build PhysicalPlan from Statement
    pub fn build_plan(&mut self, stmt: &Statement) -> Result<PhysicalPlan, PlanError> {
        match stmt {
            Statement::Query(query) => self.build_query(query),
            Statement::Insert {
                table_name,
                columns,
                source,
                ..
            } => self.build_insert(table_name, columns, source),
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
