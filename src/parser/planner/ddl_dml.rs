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

/// MS09-T02: structural column-reference probe for the ON classifier (design
/// D5a) — a bare `Identifier` or 2-part `CompoundIdentifier` only.
fn is_structural_column_ref(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(_) => true,
        Expr::CompoundIdentifier(parts) => parts.len() == 2,
        _ => false,
    }
}

/// MS09-T02 (I015): plan-time ON classification probe (delta spec R4 —
/// structural heuristic, no cost model). AND-recursion mirrors
/// `extract_join_conditions`' shape; a leg qualifies only when it is a bare
/// `Eq` whose both sides are plain column references. Purely structural — no
/// semantic resolution, so the equi forms' existing semantic errors
/// (ColumnNotFound / AmbiguousColumn / …) stay on the Hash path unchanged.
/// The probe-true set is exactly `extract_join_conditions`' acceptance face
/// (`resolve_column_ref` accepts only these two Expr shapes), so the Hash
/// path's input set — and therefore its behavior — is unchanged byte for byte.
pub(crate) fn is_pure_equi_join_on(on_expr: &Expr) -> bool {
    match on_expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => is_pure_equi_join_on(left) && is_pure_equi_join_on(right),
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => is_structural_column_ref(left) && is_structural_column_ref(right),
        _ => false,
    }
}

/// MS16 Iteration 000 replan (design D7): shared count-mismatch message for
/// INSERT column-list / row-length rejections (expected vs actual, pointable).
fn insert_count_error(table_name: &str, expected: usize, got: usize) -> PlanError {
    PlanError::ParseError(format!(
        "INSERT INTO '{}' expects {} values, got {}",
        table_name, expected, got
    ))
}

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

        // MS16 Iteration 000 replan (BH-2, design D7): apply the column list —
        // validate it as a permutation of the table's columns and reorder each
        // row through the list→table mapping (key-position semantics act on
        // the reordered value).
        let values = self.map_insert_values(&table_name_str, &columns, values)?;

        Ok(PhysicalPlan::Insert(InsertNode {
            table_name: table_name_str,
            columns,
            values,
        }))
    }

    /// MS16 Iteration 000 replan (BH-2, design D7): `InsertNode.columns` had no
    /// downstream consumer — values were interpreted positionally in table
    /// column order, so an out-of-order list silently misplaced values, a
    /// partial list panicked in `compute_tuple_size`, and unknown columns were
    /// silently dropped. The list must be a permutation of the table's
    /// columns; rows are reordered through the list→table mapping. A missing
    /// list keeps the existing positional semantics with only a row-length
    /// check (restore/import produce list-less INSERTs and are unaffected).
    fn map_insert_values(
        &self,
        table_name: &str,
        columns: &[String],
        values: Vec<Vec<Value>>,
    ) -> Result<Vec<Vec<Value>>, PlanError> {
        let table_columns = self.tables.get(&table_name.to_lowercase()).ok_or_else(|| {
            PlanError::ParseError(format!("Table '{}' does not exist", table_name))
        })?;

        if columns.is_empty() {
            return values
                .into_iter()
                .map(|row| {
                    if row.len() != table_columns.len() {
                        Err(insert_count_error(
                            table_name,
                            table_columns.len(),
                            row.len(),
                        ))
                    } else {
                        Ok(row)
                    }
                })
                .collect();
        }

        // The list must be exactly a permutation of the table's columns:
        // every entry resolves to a distinct known column and the list is
        // as long as the table is wide.
        let mut table_pos: Vec<usize> = Vec::with_capacity(columns.len());
        for name in columns {
            let pos = table_columns
                .iter()
                .position(|c| c.to_lowercase() == *name)
                .ok_or_else(|| PlanError::ColumnNotFound(name.clone()))?;
            if table_pos.contains(&pos) {
                return Err(PlanError::ParseError(format!(
                    "Duplicate column '{}' in INSERT column list",
                    name
                )));
            }
            table_pos.push(pos);
        }
        if columns.len() != table_columns.len() {
            return Err(insert_count_error(
                table_name,
                table_columns.len(),
                columns.len(),
            ));
        }

        values
            .into_iter()
            .map(|row| {
                if row.len() != columns.len() {
                    return Err(insert_count_error(table_name, columns.len(), row.len()));
                }
                let mut ordered = vec![Value::Null; table_columns.len()];
                for (requested, target) in table_pos.iter().enumerate() {
                    ordered[*target] = row[requested].clone();
                }
                Ok(ordered)
            })
            .collect()
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
                                    // MS11-T01 (I040): `-<number>` folds into the
                                    // negative literal; non-numeric operands keep
                                    // the existing rejection (same shape as
                                    // `build_expression`'s UnaryOp::Minus arm).
                                    Expr::UnaryOp {
                                        op: sqlparser::ast::UnaryOperator::Minus,
                                        expr: inner,
                                    } => {
                                        if let Expr::Value(v) = inner.as_ref() {
                                            match value_from_sqlparser(v)? {
                                                Value::Int(n) => Ok(Value::Int(-n)),
                                                Value::Float(f) => Ok(Value::Float(-f)),
                                                _ => Err(PlanError::UnsupportedValue),
                                            }
                                        } else {
                                            Err(PlanError::UnsupportedValue)
                                        }
                                    }
                                    // MS13 T4: 类型字面量 plan 期解析（决策 2）；
                                    // 解析失败点名报错，非日期族类型名维持既有拒绝。
                                    Expr::TypedString { data_type, value } => {
                                        use sqlparser::ast::{DataType, TimezoneInfo};
                                        match data_type {
                                            DataType::Date => crate::executor::datetime::parse_date(value)
                                                .map(Value::Date)
                                                .ok_or_else(|| {
                                                    PlanError::ParseError(format!(
                                                        "invalid DATE/TIMESTAMP literal: '{value}'"
                                                    ))
                                                }),
                                            DataType::Datetime(_)
                                            | DataType::Timestamp(_, TimezoneInfo::None) => {
                                                crate::executor::datetime::parse_timestamp(value)
                                                    .map(Value::Timestamp)
                                                    .ok_or_else(|| {
                                                        PlanError::ParseError(format!(
                                                            "invalid DATE/TIMESTAMP literal: '{value}'"
                                                        ))
                                                    })
                                            }
                                            _ => Err(PlanError::UnsupportedValue),
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

    /// Convert sqlparser DataType to ColumnType. MS13: DATE/TIMESTAMP map
    /// explicitly (decision 1); timezone-aware timestamp, TIME and INTERVAL
    /// are rejected by name — INTERVAL is expression-only and never a stored
    /// column type — while other unknown types keep the String fallback.
    pub(crate) fn convert_data_type(
        &self,
        data_type: &sqlparser::ast::DataType,
    ) -> Result<ColumnType, PlanError> {
        use sqlparser::ast::{DataType, TimezoneInfo};
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
            | DataType::MediumInt(_) => Ok(ColumnType::Int),

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
            | DataType::CharLargeObject(_) => Ok(ColumnType::String),

            // Float types -> Float
            DataType::Float(_)
            | DataType::Float4
            | DataType::Float64
            | DataType::Real
            | DataType::Double
            | DataType::Float8
            | DataType::DoublePrecision => Ok(ColumnType::Float),

            // Boolean types -> Bool
            DataType::Bool | DataType::Boolean => Ok(ColumnType::Bool),

            // MS13: 日期族显式映射（决策 1）——DATETIME 与无时区信息/无时区
            // TIMESTAMP 落 Timestamp；带时区变体与 TIME 点名拒绝。
            DataType::Date => Ok(ColumnType::Date),
            DataType::Datetime(_) => Ok(ColumnType::Timestamp),
            DataType::Timestamp(_, TimezoneInfo::None) => Ok(ColumnType::Timestamp),
            DataType::Timestamp(_, _) => Err(PlanError::ParseError(format!(
                "Unsupported column type: {data_type} (time-zone-aware timestamps are not supported)"
            ))),
            DataType::Time(_, _) => Err(PlanError::ParseError(format!(
                "Unsupported column type: {data_type}"
            ))),

            // MS13: INTERVAL 仅作表达式构造，不可作列类型存储（决策 1）。
            DataType::Interval => Err(PlanError::ParseError(
                "Unsupported column type: INTERVAL (interval is expression-only and cannot be a stored column type)"
                    .to_string(),
            )),

            // Unknown/unsupported types -> String (existing fallback)
            _ => Ok(ColumnType::String), // Default to String for unknown types
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
        let table_name = object_name_to_table_name(name);

        // Check for empty columns
        if columns.is_empty() {
            return Err(PlanError::EmptyColumnDefinition);
        }

        // Extract column definitions
        let column_defs: Vec<ColumnDef> = columns
            .iter()
            .map(|col| {
                let col_name = col.name.value.to_lowercase();
                let col_type = self.convert_data_type(&col.data_type)?;
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

        let table_name = object_name_to_table_name(&names[0]);

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
            // MS13 T4: 类型字面量 plan 期解析（决策 2）；裸字符串的强制解析
            // 在 UpdateExecutor 按目标列类型进行。
            Expr::TypedString { data_type, value } => {
                use sqlparser::ast::{DataType, TimezoneInfo};
                match data_type {
                    DataType::Date => crate::executor::datetime::parse_date(value)
                        .map(Value::Date)
                        .ok_or_else(|| {
                            PlanError::ParseError(format!(
                                "invalid DATE/TIMESTAMP literal: '{value}'"
                            ))
                        })?,
                    DataType::Datetime(_) | DataType::Timestamp(_, TimezoneInfo::None) => {
                        crate::executor::datetime::parse_timestamp(value)
                            .map(Value::Timestamp)
                            .ok_or_else(|| {
                                PlanError::ParseError(format!(
                                    "invalid DATE/TIMESTAMP literal: '{value}'"
                                ))
                            })?
                    }
                    _ => return Err(PlanError::UnsupportedValue),
                }
            }
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
