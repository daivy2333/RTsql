//! PlanBuilder — DML (INSERT/UPDATE/DELETE) + DDL (CREATE/DROP) + JOIN
//! condition extraction.
//!
//! MS07-T03: split from single-file `planner.rs` (T3 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::{
    ColumnConstraint, ColumnDef, ColumnType, ConflictAction, ConflictArbiter, CreateTableNode,
    DeleteNode, DropTableNode, InsertNode, JoinCondition, PhysicalPlan, UpdateNode,
    UpsertAssignment, UpsertNode, UpsertValueExpr, Value,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::{
    Assignment, BinaryOperator, ConflictTarget, Expr, OnConflictAction, OnInsert,
};

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

/// MS24 Iteration 001 (D4): shared message for conflict targets that match no
/// arbiterable constraint (composite target, non-unique column, non-INT
/// declared PK, unknown column). SQLite's own wording — the engine has no
/// composite unique constraint, so the message is exact.
fn conflict_target_mismatch() -> PlanError {
    PlanError::ParseError(
        "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint".to_string(),
    )
}

/// MS24 Iteration 001 (D4): named rejection for `DO UPDATE SET` shapes outside
/// the three supported right-value forms, pointing at the assigned column.
fn upsert_assignment_unsupported(column: &str) -> PlanError {
    PlanError::ParseError(format!(
        "DO UPDATE SET only supports literals, DEFAULT, excluded.<column> and old-row column references; unsupported assignment for column '{}'",
        column
    ))
}

/// MS24 Iteration 000 (D2): VALUES 行值的中间形态——具体值位与 `DEFAULT`
/// 关键字位（sqlparser 0.44 无 `Expr::Default` 变体，VALUES 中的 DEFAULT
/// 关键字落入既有 Identifier 臂）。仅 build_insert 内部存在；
/// `map_insert_values` 填充后输出恒为全宽 `Vec<Vec<Value>>`。
enum InsertValue {
    Val(Value),
    DefaultKeyword,
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
    ///
    /// MS24 Iteration 001 (D4): `on` / `replace_into` are consumed explicitly
    /// (the dispatcher no longer drops them with `..`) — an upsert clause is
    /// either planned as `PhysicalPlan::Upsert` or named-rejected, closing the
    /// "clause written but never in effect" silent surface. A statement with
    /// neither keeps the existing `PhysicalPlan::Insert` path byte for byte.
    pub(crate) fn build_insert(
        &self,
        table_name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::Ident],
        source: &Option<Box<sqlparser::ast::Query>>,
        on: &Option<OnInsert>,
        replace_into: bool,
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

        match self.build_upsert_action(&table_name_str, on, replace_into)? {
            Some((arbiter, action)) => Ok(PhysicalPlan::Upsert(UpsertNode {
                table_name: table_name_str,
                values,
                arbiter,
                action,
            })),
            None => Ok(PhysicalPlan::Insert(InsertNode {
                table_name: table_name_str,
                columns,
                values,
            })),
        }
    }

    /// MS24 Iteration 001 (D4): resolve the upsert clause into
    /// `(arbiter, action)`, or `None` when the statement is a plain INSERT.
    fn build_upsert_action(
        &self,
        table_name: &str,
        on: &Option<OnInsert>,
        replace_into: bool,
    ) -> Result<Option<(ConflictArbiter, ConflictAction)>, PlanError> {
        let on = match (on, replace_into) {
            (None, false) => return Ok(None),
            // REPLACE INTO arbitrates every constraint (SQLite: no target
            // concept for the replace form).
            (None, true) => return Ok(Some((ConflictArbiter::All, ConflictAction::Replace))),
            (Some(_), true) => return Err(PlanError::ParseError(
                "REPLACE INTO cannot be combined with an ON CONFLICT or ON DUPLICATE KEY clause"
                    .to_string(),
            )),
            (Some(on), false) => on,
        };

        let conflict = match on {
            OnInsert::DuplicateKeyUpdate(_) => {
                return Err(PlanError::ParseError(
                    "ON DUPLICATE KEY UPDATE is not supported".to_string(),
                ))
            }
            OnInsert::OnConflict(conflict) => conflict,
            // `OnInsert` 非穷尽枚举：未来变体不静默忽略，点名拒绝。
            _ => {
                return Err(PlanError::ParseError(
                    "unsupported INSERT conflict clause".to_string(),
                ))
            }
        };

        let arbiter = match &conflict.conflict_target {
            None => ConflictArbiter::All,
            Some(ConflictTarget::OnConstraint(_)) => {
                return Err(PlanError::ParseError(
                    "ON CONFLICT ON CONSTRAINT is not supported".to_string(),
                ))
            }
            Some(ConflictTarget::Columns(idents)) => {
                if idents.len() != 1 {
                    return Err(conflict_target_mismatch());
                }
                ConflictArbiter::Column(self.resolve_conflict_target(table_name, &idents[0].value)?)
            }
        };

        let action = match &conflict.action {
            OnConflictAction::DoNothing => ConflictAction::DoNothing,
            OnConflictAction::DoUpdate(update) => {
                if update.selection.is_some() {
                    return Err(PlanError::ParseError(
                        "DO UPDATE WHERE is not supported".to_string(),
                    ));
                }
                let mut assignments = Vec::with_capacity(update.assignments.len());
                for assignment in &update.assignments {
                    assignments.push(self.build_upsert_assignment(table_name, assignment)?);
                }
                ConflictAction::DoUpdate(assignments)
            }
        };

        Ok(Some((arbiter, action)))
    }

    /// MS24 Iteration 001 (D4): resolve an explicit single-column conflict
    /// target. Only an INT declared PK column or a UNIQUE index column is
    /// arbitrable — `to_key` yields a key for Int values only, so a non-INT
    /// declared PK has no index entry to arbitrate against.
    fn resolve_conflict_target(&self, table_name: &str, column: &str) -> Result<usize, PlanError> {
        let table_name_lower = table_name.to_lowercase();
        let table_columns = self.tables.get(&table_name_lower).ok_or_else(|| {
            PlanError::ParseError(format!("Table '{}' does not exist", table_name))
        })?;
        let pos = table_columns
            .iter()
            .position(|c| c.to_lowercase() == column.to_lowercase())
            .ok_or_else(conflict_target_mismatch)?;

        let pk = self
            .primary_keys
            .get(&table_name_lower)
            .map(|s| s.as_str())
            .unwrap_or("");
        let is_int_pk = !pk.is_empty()
            && pk == column.to_lowercase()
            && self.primary_key_types.get(&table_name_lower)
                == Some(&crate::storage::page_format::ColumnType::Int);
        let is_unique = self
            .table_unique_columns
            .get(&table_name_lower)
            .is_some_and(|cols| cols.contains(&pos));

        if is_int_pk || is_unique {
            Ok(pos)
        } else {
            Err(conflict_target_mismatch())
        }
    }

    /// MS24 Iteration 001 (D4): one `DO UPDATE SET` assignment — target column
    /// position plus the three supported right-value forms. A compound left
    /// value (`t.col = ...`) is named-rejected.
    fn build_upsert_assignment(
        &self,
        table_name: &str,
        assignment: &Assignment,
    ) -> Result<UpsertAssignment, PlanError> {
        let name = assignment
            .id
            .last()
            .map(|ident| ident.value.to_lowercase())
            .unwrap_or_default();
        if assignment.id.len() != 1 {
            return Err(upsert_assignment_unsupported(&name));
        }
        let column = self.resolve_column_pos(table_name, &name)?;
        Ok(UpsertAssignment {
            column,
            expr: self.build_upsert_value(table_name, &name, column, &assignment.value)?,
        })
    }

    /// MS24 Iteration 001 (D4): `DO UPDATE SET` right value — literal (with
    /// `DEFAULT` literalized from the declared default), `excluded.col` (the
    /// row being inserted) or a bare column name (the conflicting old row).
    /// Arithmetic / function / nested expressions are named-rejected.
    fn build_upsert_value(
        &self,
        table_name: &str,
        target_name: &str,
        target: usize,
        expr: &Expr,
    ) -> Result<UpsertValueExpr, PlanError> {
        match expr {
            Expr::Value(v) => Ok(UpsertValueExpr::Literal(value_from_sqlparser(v)?)),
            // 类型字面量 plan 期解析（与 `build_update` 同规则）；裸字符串的强制
            // 解析在执行期按目标列类型进行。
            Expr::TypedString { data_type, value } => {
                use sqlparser::ast::{DataType, TimezoneInfo};
                match data_type {
                    DataType::Date => crate::executor::datetime::parse_date(value)
                        .map(Value::Date)
                        .ok_or_else(|| {
                            PlanError::ParseError(format!(
                                "invalid DATE/TIMESTAMP literal: '{value}'"
                            ))
                        })
                        .map(UpsertValueExpr::Literal),
                    DataType::Datetime(_) | DataType::Timestamp(_, TimezoneInfo::None) => {
                        crate::executor::datetime::parse_timestamp(value)
                            .map(Value::Timestamp)
                            .ok_or_else(|| {
                                PlanError::ParseError(format!(
                                    "invalid DATE/TIMESTAMP literal: '{value}'"
                                ))
                            })
                            .map(UpsertValueExpr::Literal)
                    }
                    _ => Err(PlanError::UnsupportedValue),
                }
            }
            Expr::Identifier(ident) => {
                let upper = ident.value.to_uppercase();
                if upper == "NULL" {
                    return Ok(UpsertValueExpr::Literal(Value::Null));
                }
                if upper == "DEFAULT" {
                    let default = self
                        .table_defaults
                        .get(&table_name.to_lowercase())
                        .and_then(|d| d.get(target))
                        .cloned()
                        .flatten()
                        .unwrap_or(Value::Null);
                    return Ok(UpsertValueExpr::Literal(default));
                }
                Ok(UpsertValueExpr::Old(
                    self.resolve_column_pos(table_name, &ident.value)?,
                ))
            }
            Expr::CompoundIdentifier(parts)
                if parts.len() == 2 && parts[0].value.eq_ignore_ascii_case("excluded") =>
            {
                Ok(UpsertValueExpr::Excluded(
                    self.resolve_column_pos(table_name, &parts[1].value)?,
                ))
            }
            _ => Err(upsert_assignment_unsupported(target_name)),
        }
    }

    /// MS24 Iteration 001 (D4): resolve a column name to its position, keeping
    /// the existing `ColumnNotFound` face.
    fn resolve_column_pos(&self, table_name: &str, column: &str) -> Result<usize, PlanError> {
        let table_columns = self.tables.get(&table_name.to_lowercase()).ok_or_else(|| {
            PlanError::ParseError(format!("Table '{}' does not exist", table_name))
        })?;
        table_columns
            .iter()
            .position(|c| c.to_lowercase() == column.to_lowercase())
            .ok_or_else(|| PlanError::ColumnNotFound(column.to_string()))
    }

    /// MS16 Iteration 000 replan (BH-2, design D7): `InsertNode.columns` had no
    /// downstream consumer — values were interpreted positionally in table
    /// column order, so an out-of-order list silently misplaced values, a
    /// partial list panicked in `compute_tuple_size`, and unknown columns were
    /// silently dropped. The list must be a permutation of the table's
    /// columns; rows are reordered through the list→table mapping. A missing
    /// list keeps the existing positional semantics with only a row-length
    /// check (restore/import produce list-less INSERTs and are unaffected).
    ///
    /// MS24 Iteration 000 (D2): the list is relaxed to a SUBSET of the table's
    /// columns (each entry resolves to a distinct known column). Omitted
    /// positions and `DEFAULT`-keyword positions are filled from the table's
    /// declared defaults (`table_defaults`; no default → NULL). Unknown /
    /// duplicate columns and list-less row-length mismatches keep their
    /// existing plan-time rejections; a full list and a list-less INSERT
    /// without DEFAULT keywords behave byte-identically to the previous
    /// semantics. NOT NULL enforcement for filled NULLs stays at the
    /// executor's single gate (no duplicate plan-time path).
    fn map_insert_values(
        &self,
        table_name: &str,
        columns: &[String],
        values: Vec<Vec<InsertValue>>,
    ) -> Result<Vec<Vec<Value>>, PlanError> {
        let table_name_lower = table_name.to_lowercase();
        let table_columns = self.tables.get(&table_name_lower).ok_or_else(|| {
            PlanError::ParseError(format!("Table '{}' does not exist", table_name))
        })?;
        let defaults = self
            .table_defaults
            .get(&table_name_lower)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // 省略位/DEFAULT 关键字位的填充值：声明 DEFAULT → 克隆；无 → NULL。
        let fill =
            |i: usize| -> Value { defaults.get(i).cloned().flatten().unwrap_or(Value::Null) };

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
                        Ok(row
                            .into_iter()
                            .enumerate()
                            .map(|(i, iv)| match iv {
                                InsertValue::Val(v) => v,
                                InsertValue::DefaultKeyword => fill(i),
                            })
                            .collect())
                    }
                })
                .collect();
        }

        // The list entries must resolve to distinct known columns (a subset
        // of the table's columns is legal since MS24).
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

        values
            .into_iter()
            .map(|row| {
                if row.len() != columns.len() {
                    return Err(insert_count_error(table_name, columns.len(), row.len()));
                }
                // 全宽输出：省略位与 DEFAULT 关键字位预填 defaults/NULL，
                // 显式值位经清单→表列映射放置。
                let mut ordered: Vec<Value> = (0..table_columns.len()).map(fill).collect();
                for (requested, target) in table_pos.iter().enumerate() {
                    if let InsertValue::Val(v) = &row[requested] {
                        ordered[*target] = v.clone();
                    }
                }
                Ok(ordered)
            })
            .collect()
    }

    /// Extract values from INSERT source (VALUES clause)
    ///
    /// MS24 Iteration 000 (D2): row values are the internal `InsertValue`
    /// shape — `DEFAULT` keywords map to `DefaultKeyword` (sqlparser 0.44
    /// yields `Expr::Identifier("DEFAULT")`), everything else keeps its
    /// existing value resolution wrapped in `Val`. Private: the only caller
    /// is `build_insert` in this module (the `InsertValue` intermediate stays
    /// module-internal).
    fn extract_insert_values(
        &self,
        source: &Option<Box<sqlparser::ast::Query>>,
    ) -> Result<Vec<Vec<InsertValue>>, PlanError> {
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
                                    Expr::Value(v) => Ok(InsertValue::Val(value_from_sqlparser(v)?)),
                                    Expr::Identifier(ident) => {
                                        // Handle NULL identifier
                                        if ident.value.to_uppercase() == "NULL" {
                                            Ok(InsertValue::Val(Value::Null))
                                        } else if ident.value.to_uppercase() == "DEFAULT" {
                                            // MS24 Iteration 000 (D2): DEFAULT
                                            // 关键字等价省略（填充在
                                            // map_insert_values 消费 defaults）。
                                            Ok(InsertValue::DefaultKeyword)
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
                                                Value::Int(n) => {
                                                    Ok(InsertValue::Val(Value::Int(-n)))
                                                }
                                                Value::Float(f) => {
                                                    Ok(InsertValue::Val(Value::Float(-f)))
                                                }
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
                                                .map(|v| InsertValue::Val(Value::Date(v)))
                                                .ok_or_else(|| {
                                                    PlanError::ParseError(format!(
                                                        "invalid DATE/TIMESTAMP literal: '{value}'"
                                                    ))
                                                }),
                                            DataType::Datetime(_)
                                            | DataType::Timestamp(_, TimezoneInfo::None) => {
                                                crate::executor::datetime::parse_timestamp(value)
                                                    .map(|v| InsertValue::Val(Value::Timestamp(v)))
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
                // MS23-T02: CHECK/FOREIGN KEY/方言项点名拒绝——执行器不消费这些
                // 约束，静默接受即「建表成功但约束永不生效」。
                sqlparser::ast::ColumnOption::Check(_) => {
                    return Err(PlanError::UnsupportedConstraint("CHECK"));
                }
                sqlparser::ast::ColumnOption::ForeignKey { .. } => {
                    return Err(PlanError::UnsupportedConstraint("FOREIGN KEY"));
                }
                sqlparser::ast::ColumnOption::DialectSpecific(_) => {
                    return Err(PlanError::UnsupportedConstraint(
                        "dialect-specific column options",
                    ));
                }
                // PrimaryKey (is_primary: true) is handled separately by extract_primary_key
                // Null, Comment 等无语义期望选项维持忽略
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
        let mut column_defs: Vec<ColumnDef> = columns
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

        // MS23-T02: 表级 CHECK/FOREIGN KEY/Index 类点名拒绝——PRIMARY KEY 继续
        // 由 extract_primary_key 消费，表级 UNIQUE 属 MS23 Iteration 001。
        for constraint in constraints {
            match constraint {
                sqlparser::ast::TableConstraint::Check { .. } => {
                    return Err(PlanError::UnsupportedConstraint("CHECK"));
                }
                sqlparser::ast::TableConstraint::ForeignKey { .. } => {
                    return Err(PlanError::UnsupportedConstraint("FOREIGN KEY"));
                }
                sqlparser::ast::TableConstraint::Index { .. } => {
                    return Err(PlanError::UnsupportedConstraint("INDEX/KEY"));
                }
                sqlparser::ast::TableConstraint::FulltextOrSpatial { .. } => {
                    return Err(PlanError::UnsupportedConstraint("FULLTEXT/SPATIAL"));
                }
                _ => {}
            }
        }

        // MS23 Iteration 001 (2.4/D6): UNIQUE 策略面——仅 INT 非 PK 列强制。
        // (a) 列级 UNIQUE：非 PK 列且声明类型非 INT 点名拒绝；PK 列声明的
        //     UNIQUE 消费为 PK 索引既有唯一性（承载面不建第二索引），不拒绝。
        // (b) 表级 UNIQUE{is_primary: false}：多列（组合）点名拒绝；单列按
        //     列级同规则映射为该列 Unique 标志（to_schema_column 折叠后
        //     catalog 持久化与 dump/schema 渲染自动正确）。
        for col in &column_defs {
            let declared_unique = col
                .constraints
                .iter()
                .any(|c| matches!(c, ColumnConstraint::Unique));
            if declared_unique
                && primary_key.as_deref() != Some(col.name.as_str())
                && col.data_type != ColumnType::Int
            {
                return Err(PlanError::UnsupportedConstraint(
                    "UNIQUE (INT columns only)",
                ));
            }
        }
        for constraint in constraints {
            if let sqlparser::ast::TableConstraint::Unique {
                is_primary: false,
                columns: unique_cols,
                ..
            } = constraint
            {
                if unique_cols.len() > 1 {
                    return Err(PlanError::UnsupportedConstraint(
                        "composite UNIQUE (single-column UNIQUE only)",
                    ));
                }
                let target = unique_cols[0].value.to_lowercase();
                let is_pk = primary_key.as_deref() == Some(target.as_str());
                let col_def = column_defs
                    .iter_mut()
                    .find(|c| c.name == target)
                    .ok_or_else(|| PlanError::ColumnNotFound(target.clone()))?;
                if !is_pk {
                    if col_def.data_type != ColumnType::Int {
                        return Err(PlanError::UnsupportedConstraint(
                            "UNIQUE (INT columns only)",
                        ));
                    }
                    if !col_def
                        .constraints
                        .iter()
                        .any(|c| matches!(c, ColumnConstraint::Unique))
                    {
                        col_def.constraints.push(ColumnConstraint::Unique);
                    }
                }
            }
        }

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
