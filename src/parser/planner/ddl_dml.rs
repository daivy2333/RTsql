//! PlanBuilder — DML (INSERT/UPDATE/DELETE) + DDL (CREATE/DROP) + JOIN
//! condition extraction.
//!
//! MS07-T03: split from single-file `planner.rs` (T3 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::{
    ColumnConstraint, ColumnDef, ColumnType, CreateTableNode, DeleteNode, DropTableNode,
    InsertNode, JoinCondition, PhysicalPlan, UpdateNode, Value,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::{BinaryOperator, Expr};

impl PlanBuilder {
    /// 提取 JOIN ON 条件（支持 AND 组合等值条件）
    pub(crate) fn extract_join_conditions(
        &self,
        left_tables: &[String],
        right_table: &str,
        on_expr: &Expr,
    ) -> Result<Vec<JoinCondition>, PlanError> {
        // 处理 AND 组合
        if let Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } = on_expr
        {
            let left_conditions = self.extract_join_conditions(left_tables, right_table, left)?;
            let right_conditions = self.extract_join_conditions(left_tables, right_table, right)?;
            return Ok(left_conditions
                .into_iter()
                .chain(right_conditions)
                .collect());
        }

        // 处理单一等值条件
        if let Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } = on_expr
        {
            let left_ref = self.resolve_column_ref(left, left_tables)?;
            let right_ref = self.resolve_column_ref(right, &[right_table.to_string()])?;

            // 验证：左边列来自左表，右边列来自右表（或反序）
            if left_ref.table.as_deref() == Some(right_table) {
                // 反序：right.col = left.col，交换
                return Ok(vec![JoinCondition {
                    left_column: right_ref,
                    right_column: left_ref,
                }]);
            }

            Ok(vec![JoinCondition {
                left_column: left_ref,
                right_column: right_ref,
            }])
        } else {
            Err(PlanError::UnsupportedExpression)
        }
    }

    /// Build PhysicalPlan for INSERT statement
    pub(crate) fn build_insert(
        &self,
        table_name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::Ident],
        source: &Option<Box<sqlparser::ast::Query>>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name_str = extract_name_from_object(table_name);
        self.validate_table(&table_name_str)?;

        // Extract column names
        let columns: Vec<String> = columns.iter().map(|c| c.value.to_lowercase()).collect();

        // Extract values from source
        let values = self.extract_insert_values(source)?;

        Ok(PhysicalPlan::Insert(InsertNode {
            table_name: table_name_str,
            columns,
            values,
        }))
    }

    /// Extract values from INSERT source (VALUES clause)
    pub(crate) fn extract_insert_values(
        &self,
        source: &Option<Box<sqlparser::ast::Query>>,
    ) -> Result<Vec<Vec<Value>>, PlanError> {
        let source = source
            .as_ref()
            .ok_or_else(|| PlanError::MissingField("VALUES".into()))?;

        // Expect SetExpr::Values
        match source.body.as_ref() {
            sqlparser::ast::SetExpr::Values(values) => {
                values
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|expr| {
                                match expr {
                                    Expr::Value(v) => value_from_sqlparser(v),
                                    Expr::Identifier(ident) => {
                                        // Handle NULL identifier
                                        if ident.value.to_uppercase() == "NULL" {
                                            Ok(Value::Null)
                                        } else {
                                            Err(PlanError::UnsupportedValue)
                                        }
                                    }
                                    _ => Err(PlanError::UnsupportedValue),
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()
            }
            _ => Err(PlanError::UnsupportedStatement),
        }
    }

    /// Convert sqlparser DataType to ColumnType
    pub(crate) fn convert_data_type(&self, data_type: &sqlparser::ast::DataType) -> ColumnType {
        use sqlparser::ast::DataType;
        match data_type {
            // Integer types -> Int
            DataType::Int(_)
            | DataType::Int4(_)
            | DataType::Integer(_)
            | DataType::BigInt(_)
            | DataType::Int8(_)
            | DataType::SmallInt(_)
            | DataType::Int2(_)
            | DataType::TinyInt(_)
            | DataType::MediumInt(_) => ColumnType::Int,

            // String types -> String
            DataType::Varchar(_)
            | DataType::Nvarchar(_)
            | DataType::Char(_)
            | DataType::Character(_)
            | DataType::CharacterVarying(_)
            | DataType::CharVarying(_)
            | DataType::Text
            | DataType::Clob(_)
            | DataType::CharacterLargeObject(_)
            | DataType::CharLargeObject(_) => ColumnType::String,

            // Float types -> Float
            DataType::Float(_)
            | DataType::Float4
            | DataType::Float64
            | DataType::Real
            | DataType::Double
            | DataType::Float8
            | DataType::DoublePrecision => ColumnType::Float,

            // Boolean types -> Bool
            DataType::Bool | DataType::Boolean => ColumnType::Bool,

            // Unknown/unsupported types -> Null (placeholder)
            _ => ColumnType::String, // Default to String for unknown types
        }
    }

    /// Extract column constraints from sqlparser ColumnDef
    pub(crate) fn extract_column_constraints(
        &self,
        column: &sqlparser::ast::ColumnDef,
    ) -> Result<Vec<ColumnConstraint>, PlanError> {
        let mut constraints = Vec::new();

        for option in &column.options {
            match &option.option {
                sqlparser::ast::ColumnOption::NotNull => {
                    constraints.push(ColumnConstraint::NotNull);
                }
                sqlparser::ast::ColumnOption::Unique {
                    is_primary: false, ..
                } => {
                    constraints.push(ColumnConstraint::Unique);
                }
                sqlparser::ast::ColumnOption::Default(expr) => {
                    let value = self.extract_default_value(expr)?;
                    constraints.push(ColumnConstraint::DefaultValue(value));
                }
                // PrimaryKey (is_primary: true) is handled separately by extract_primary_key
                // Null, ForeignKey, Check, DialectSpecific, etc. are ignored
                _ => {}
            }
        }

        Ok(constraints)
    }

    /// Extract default value from expression
    pub(crate) fn extract_default_value(&self, expr: &Expr) -> Result<Value, PlanError> {
        match expr {
            Expr::Value(v) => value_from_sqlparser(v),
            Expr::Identifier(ident) => {
                if ident.value.to_uppercase() == "NULL" {
                    Ok(Value::Null)
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            // Handle negative numbers: -42
            Expr::UnaryOp {
                op: sqlparser::ast::UnaryOperator::Minus,
                expr,
            } => {
                if let Expr::Value(v) = expr.as_ref() {
                    let value = value_from_sqlparser(v)?;
                    match value {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err(PlanError::UnsupportedValue),
                    }
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            _ => Err(PlanError::UnsupportedValue),
        }
    }

    /// Extract primary key from column constraints and table constraints
    pub(crate) fn extract_primary_key(
        &self,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
    ) -> Result<Option<String>, PlanError> {
        let mut pk_candidates = Vec::new();

        // Check column-level PRIMARY KEY constraints
        for column in columns {
            for option in &column.options {
                if let sqlparser::ast::ColumnOption::Unique {
                    is_primary: true, ..
                } = &option.option
                {
                    pk_candidates.push(column.name.value.to_lowercase());
                }
            }
        }

        // Check table-level PRIMARY KEY constraints (Unique { is_primary: true })
        for constraint in constraints {
            if let sqlparser::ast::TableConstraint::Unique {
                is_primary: true,
                columns: pk_columns,
                ..
            } = constraint
            {
                for pk_col in pk_columns {
                    pk_candidates.push(pk_col.value.to_lowercase());
                }
            }
        }

        // Validate: only single-column PK supported
        match pk_candidates.len() {
            0 => Ok(None),
            1 => Ok(Some(pk_candidates[0].clone())),
            _ => Err(PlanError::MultiplePrimaryKey),
        }
    }

    /// Build PhysicalPlan for CREATE TABLE statement
    pub(crate) fn build_create_table(
        &self,
        name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name = name.to_string().to_lowercase();

        // Check for empty columns
        if columns.is_empty() {
            return Err(PlanError::EmptyColumnDefinition);
        }

        // Extract column definitions
        let column_defs: Vec<ColumnDef> = columns
            .iter()
            .map(|col| {
                let col_name = col.name.value.to_lowercase();
                let col_type = self.convert_data_type(&col.data_type);
                let col_constraints = self.extract_column_constraints(col)?;
                Ok(ColumnDef {
                    name: col_name,
                    data_type: col_type,
                    constraints: col_constraints,
                })
            })
            .collect::<Result<Vec<_>, PlanError>>()?;

        // Extract primary key
        let primary_key = self.extract_primary_key(columns, constraints)?;

        Ok(PhysicalPlan::CreateTable(CreateTableNode {
            table_name,
            columns: column_defs,
            primary_key,
        }))
    }

    /// Build PhysicalPlan for DROP TABLE statement
    pub(crate) fn build_drop_table(
        &self,
        names: &[sqlparser::ast::ObjectName],
        if_exists: &bool,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name (only single table supported)
        if names.is_empty() {
            return Err(PlanError::MissingField("table name".into()));
        }

        let table_name = names[0].to_string().to_lowercase();

        Ok(PhysicalPlan::DropTable(DropTableNode {
            table_name,
            if_exists: *if_exists,
        }))
    }

    /// Build PhysicalPlan for UPDATE statement
    ///
    /// Only supports single column update with primary key WHERE clause
    pub(crate) fn build_update(
        &self,
        table: &sqlparser::ast::TableWithJoins,
        assignments: &[sqlparser::ast::Assignment],
        selection: &Option<Expr>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name = extract_table_name(std::slice::from_ref(table))?;
        self.validate_table(&table_name)?;

        // Extract primary key from WHERE clause
        let where_expr = selection
            .as_ref()
            .ok_or_else(|| PlanError::MissingField("WHERE clause for UPDATE".into()))?;
        let key = self
            .extract_pk_from_where(&table_name, where_expr)?
            .ok_or_else(|| {
                PlanError::ParseError("UPDATE requires primary key equality in WHERE clause".into())
            })?;

        // Only support single column update
        if assignments.len() != 1 {
            return Err(PlanError::UnsupportedStatement);
        }

        let assignment = &assignments[0];

        // Extract column name
        if assignment.id.len() != 1 {
            return Err(PlanError::UnsupportedStatement);
        }
        let column = assignment.id[0].value.to_lowercase();

        // Extract new value
        let new_value = match &assignment.value {
            Expr::Value(v) => value_from_sqlparser(v)?,
            Expr::Identifier(ident) => {
                if ident.value.to_uppercase() == "NULL" {
                    Value::Null
                } else {
                    return Err(PlanError::UnsupportedValue);
                }
            }
            _ => return Err(PlanError::UnsupportedValue),
        };

        Ok(PhysicalPlan::Update(UpdateNode {
            table_name,
            key,
            column,
            new_value,
        }))
    }

    /// Build PhysicalPlan for DELETE statement
    ///
    /// Only supports primary key WHERE clause
    pub(crate) fn build_delete(
        &self,
        from: &sqlparser::ast::FromTable,
        selection: &Option<Expr>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name from FromTable
        let table_with_joins = match from {
            sqlparser::ast::FromTable::WithFromKeyword(tables) => tables,
            sqlparser::ast::FromTable::WithoutKeyword(tables) => tables,
        };
        let table_name = extract_table_name(table_with_joins)?;
        self.validate_table(&table_name)?;

        // Extract primary key from WHERE clause
        let where_expr = selection
            .as_ref()
            .ok_or_else(|| PlanError::MissingField("WHERE clause for DELETE".into()))?;
        let key = self
            .extract_pk_from_where(&table_name, where_expr)?
            .ok_or_else(|| {
                PlanError::ParseError("DELETE requires primary key equality in WHERE clause".into())
            })?;

        Ok(PhysicalPlan::Delete(DeleteNode { table_name, key }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::PlanBuilder;
    use crate::executor::{PhysicalPlan, Value};
    use crate::parser::ast::parse_sql;

    #[test]
    fn test_build_insert() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Insert(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.columns, vec!["id", "name"]);
                assert_eq!(node.values.len(), 1);
                assert_eq!(node.values[0].len(), 2);
                assert_eq!(node.values[0][0], Value::Int(1));
                assert_eq!(node.values[0][1], Value::String("Alice".to_string()));
            }
            _ => panic!("Expected Insert plan"),
        }
    }

    #[test]
    fn test_build_update() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "UPDATE users SET name = 'Bob' WHERE id = 1";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Update(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.column, "name");
                assert_eq!(node.new_value, Value::String("Bob".to_string()));
                let expected_key = Value::Int(1).to_key().unwrap();
                assert_eq!(node.key, expected_key);
            }
            _ => panic!("Expected Update plan"),
        }
    }

    #[test]
    fn test_build_delete() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "DELETE FROM users WHERE id = 1";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Delete(node) => {
                assert_eq!(node.table_name, "users");
                let expected_key = Value::Int(1).to_key().unwrap();
                assert_eq!(node.key, expected_key);
            }
            _ => panic!("Expected Delete plan"),
        }
    }

    #[test]
    fn test_insert_multiple_rows() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob')";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Insert(node) => {
                assert_eq!(node.values.len(), 2);
                assert_eq!(
                    node.values[0],
                    vec![Value::Int(1), Value::String("Alice".to_string())]
                );
                assert_eq!(
                    node.values[1],
                    vec![Value::Int(2), Value::String("Bob".to_string())]
                );
            }
            _ => panic!("Expected Insert plan"),
        }
    }
}
