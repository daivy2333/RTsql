//! PlanBuilder — DML (INSERT/UPDATE/DELETE) + DDL (CREATE/DROP) + JOIN
//! condition extraction.
//!
//! MS07-T03: split from single-file `planner.rs` (T3 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::datetime::coerce_datetime_write;
use crate::executor::{
    AlterTableNode, AlterTableOp, ColumnConstraint, ColumnDef, ColumnType, ConflictAction,
    ConflictArbiter, CreateIndexNode, CreateTableAsSelectNode, CreateTableNode, CreateViewNode,
    DeleteNode, DropIndexNode, DropTableNode, DropViewNode, InsertNode, InsertSelectNode,
    JoinCondition, PhysicalPlan, TruncateNode, UpdateNode, UpsertAssignment, UpsertNode,
    UpsertValueExpr, Value,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::{
    Assignment, BinaryOperator, ConflictTarget, Expr, OnConflictAction, OnInsert,
};

/// MS21 Iter005 (T5-b/D12)：视图定义 SQL 文本长度上限（字节）。catalog 行须
/// 单 slot 可容（页面 4KB 减去行头/既有尾段预留），超限在计划期前置点名
/// 拒绝（不达 `add_slot` 的执行期 PageFull）。具体边界值为非实质实现细节，
/// 契约是「超长 → 计划期拒绝零副作用」。
const MAX_VIEW_SQL_BYTES: usize = 3000;

/// 列类型大写名（声明期 DEFAULT 错误面用，与执行器 column_type_name 同形态）。
fn column_type_name(ct: &ColumnType) -> &'static str {
    match ct {
        ColumnType::Int => "INT",
        ColumnType::String => "STRING",
        ColumnType::Float => "FLOAT",
        ColumnType::Bool => "BOOL",
        ColumnType::Date => "DATE",
        ColumnType::Timestamp => "TIMESTAMP",
    }
}

/// 值类型大写名（声明期 DEFAULT 错误面「实际值类型」侧）。
fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "INT",
        Value::String(_) => "STRING",
        Value::Float(_) => "FLOAT",
        Value::Bool(_) => "BOOL",
        Value::Date(_) => "DATE",
        Value::Timestamp(_) => "TIMESTAMP",
        Value::Null => "NULL",
    }
}

/// MS29 T4 (D6)：声明期 DEFAULT 字面量与列类型的校验/归一单点——CREATE
/// TABLE 与 ALTER ADD COLUMN 共用，规则与写入值类型一致门对齐。`Ok(None)`
/// = 原样接受；`Ok(Some(v))` = 归一后载荷（FLOAT 列整数字面量无损升格）；
/// `Err` = 计划期点名拒绝（列名/期望/实际），零副作用。
pub(crate) fn validate_default_value(
    col_name: &str,
    col_type: &ColumnType,
    value: &Value,
) -> Result<Option<Value>, PlanError> {
    use crate::storage::page_format::ColumnType as StorageColumnType;
    match (col_type, value) {
        // NULL 豁免——NULL 性由 NOT NULL 门裁决
        (_, Value::Null) => Ok(None),
        // FLOAT 列整数字面量无损升格归一持久化
        (ColumnType::Float, Value::Int(n)) => Ok(Some(Value::Float(*n as f64))),
        // 日期族：String（含 TypedString 归一同态）经写入边界同源强制解析验证
        (ColumnType::Date | ColumnType::Timestamp, _) => {
            let storage_type = match col_type {
                ColumnType::Date => StorageColumnType::Date,
                _ => StorageColumnType::Timestamp,
            };
            match coerce_datetime_write(value, &storage_type) {
                Ok(_) => Ok(None),
                Err(_) => Err(PlanError::DefaultTypeMismatch(format!(
                    "column '{}': DEFAULT {} is not a valid {} value",
                    col_name,
                    value,
                    column_type_name(col_type)
                ))),
            }
        }
        // 同型放行
        (ColumnType::Int, Value::Int(_))
        | (ColumnType::String, Value::String(_))
        | (ColumnType::Float, Value::Float(_))
        | (ColumnType::Bool, Value::Bool(_)) => Ok(None),
        // 其余异型点名拒绝
        (col_type, value) => Err(PlanError::DefaultTypeMismatch(format!(
            "column '{}': expected {}, got {}",
            col_name,
            column_type_name(col_type),
            value_type_name(value)
        ))),
    }
}

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
            Err(PlanError::UnsupportedExpression(
                "JOIN ON CONSTRAINT".into(),
            ))
        }
    }

    /// Build PhysicalPlan for INSERT statement
    ///
    /// MS24 Iteration 001 (D4): `on` / `replace_into` are consumed explicitly
    /// (the dispatcher no longer drops them with `..`) — an upsert clause is
    /// either planned as `PhysicalPlan::Upsert` or named-rejected, closing the
    /// "clause written but never in effect" silent surface. A statement with
    /// neither keeps the existing `PhysicalPlan::Insert` path byte for byte.
    ///
    /// MS20-T02 Iter001 (D4): a SELECT source takes the `InsertSelect` arm —
    /// the source compiles as a top-level query and its rows are materialized
    /// at execution time (T5). A VALUES source (or a missing source, which
    /// keeps the `MissingField("VALUES")` face) runs the unchanged path below.
    pub(crate) fn build_insert(
        &mut self,
        table_name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::Ident],
        source: &Option<Box<sqlparser::ast::Query>>,
        on: &Option<OnInsert>,
        replace_into: bool,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name_str = extract_name_from_object(table_name);
        // MS21 Iter005 (T5-c/D12)：视图只读——DML 目标为视图名计划期点名
        // 拒绝（先于 validate_table：视图名不在基表注册表，后者只会给出
        // 误导性的 42P01）。
        self.reject_view_write(&table_name_str)?;
        self.validate_table(&table_name_str)?;
        // MS20-T02: 附加库引用不再在计划期拒绝——DML 目标由 pipeline 的
        // 执行根按路由表选定目标库（design D1）；DDL/CTAS/view 的附加拒绝面
        // 保持（`build_create_table`/`build_alter_table` 等仍调用
        // `reject_attached_write`）。

        // Extract column names
        let columns: Vec<String> = columns.iter().map(|c| c.value.to_lowercase()).collect();

        let select_source = match source.as_deref() {
            // Missing source → existing `MissingField("VALUES")` face below.
            None => None,
            Some(q) => match q.body.as_ref() {
                sqlparser::ast::SetExpr::Values(_) => None,
                _ => Some(q),
            },
        };
        if let Some(source_query) = select_source {
            return self.build_insert_select(
                &table_name_str,
                columns,
                source_query,
                on,
                replace_into,
            );
        }

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

    /// MS20-T02 Iter001 (design D4): plan `INSERT INTO <existing> SELECT ...`.
    ///
    /// The source compiles as a **top-level** query (`build_query` — WITH /
    /// DISTINCT / JOIN / views / set operations fully consumed, never the
    /// `building_subquery` suppression). Column-list validation shares the
    /// VALUES semantics: unknown names → `ColumnNotFound`, duplicates named,
    /// and the source width must match the list (or the table width when no
    /// list is given) — an empty source is still structurally validated here
    /// because the width comes from the plan, not from rows. Conflict clauses
    /// reuse `build_upsert_action` (`REPLACE INTO ... SELECT` included).
    fn build_insert_select(
        &mut self,
        table_name: &str,
        columns: Vec<String>,
        source: &sqlparser::ast::Query,
        on: &Option<OnInsert>,
        replace_into: bool,
    ) -> Result<PhysicalPlan, PlanError> {
        let source_plan = self.build_query(source)?;
        let source_width = self.get_plan_output_columns(&source_plan).len();

        let table_name_lower = table_name.to_lowercase();
        let table_columns = self.tables.get(&table_name_lower).ok_or_else(|| {
            PlanError::ParseError(format!("Table '{}' does not exist", table_name))
        })?;
        let defaults = self
            .table_defaults
            .get(&table_name_lower)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // 清单→表列位置映射与 VALUES 通道共享语义（未知 / 重复列同文案）。
        // 未写清单 = 恒等映射（全列按位接收）。
        let target_positions: Vec<usize> = if columns.is_empty() {
            (0..table_columns.len()).collect()
        } else {
            let mut positions = Vec::with_capacity(columns.len());
            for name in &columns {
                let pos = table_columns
                    .iter()
                    .position(|c| c.to_lowercase() == *name)
                    .ok_or_else(|| PlanError::ColumnNotFound(name.clone()))?;
                if positions.contains(&pos) {
                    return Err(PlanError::ParseError(format!(
                        "Duplicate column '{}' in INSERT column list",
                        name
                    )));
                }
                positions.push(pos);
            }
            positions
        };

        // 源宽度必须在计划期对齐接收面（空源同样成立——结构验证不依赖行）。
        let expected = if columns.is_empty() {
            table_columns.len()
        } else {
            columns.len()
        };
        if source_width != expected {
            return Err(insert_count_error(table_name, expected, source_width));
        }

        // 全宽填充：省略位落声明 DEFAULT 或 NULL（与 map_insert_values 的
        // fill 语义一致）；未写清单时恒等映射覆盖全部位置，fill 不被消费。
        let fill = (0..table_columns.len())
            .map(|i| defaults.get(i).cloned().flatten().unwrap_or(Value::Null))
            .collect();

        let upsert = self.build_upsert_action(table_name, on, replace_into)?;
        Ok(PhysicalPlan::InsertSelect(InsertSelectNode {
            table_name: table_name.to_string(),
            columns,
            source: Box::new(source_plan),
            source_width,
            target_positions,
            fill,
            upsert,
        }))
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
            _ => Err(PlanError::UnsupportedStatement("INSERT ... SELECT".into())),
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
            // MS29 T5: 类型字面量（TypedString，如 `DATE '2026-01-02'`）——
            // 日期族经写入边界同源强制解析验证后以 String 目录形态返回（与
            // 手写 String 声明同态，dump→restore 往返修复）；其余类型维持
            // 既有 UnsupportedValue。
            Expr::TypedString { data_type, value } => {
                use crate::storage::page_format::ColumnType as StorageColumnType;
                use sqlparser::ast::{DataType, TimezoneInfo};
                let storage_type = match data_type {
                    DataType::Date => StorageColumnType::Date,
                    DataType::Timestamp(_, TimezoneInfo::None) => StorageColumnType::Timestamp,
                    _ => return Err(PlanError::UnsupportedValue),
                };
                match coerce_datetime_write(&Value::String(value.clone()), &storage_type) {
                    Ok(_) => Ok(Value::String(value.clone())),
                    Err(_) => Err(PlanError::DefaultTypeMismatch(format!(
                        "invalid {data_type} literal '{value}'"
                    ))),
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
    ///
    /// MS20 T4：`query` 成分消费为 `CreateTableAsSelect` 计划；`LIKE` / `CLONE`
    /// 与「列清单 + query」混合形态计划期点名拒绝（不再静默建空表或静默忽略
    /// 成分）。
    pub(crate) fn build_create_table(
        &mut self,
        name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
        query: Option<&sqlparser::ast::Query>,
        like: Option<&sqlparser::ast::ObjectName>,
        clone: Option<&sqlparser::ast::ObjectName>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name = object_name_to_table_name(name);
        // MS20 T3: 附加库一期只读（`CREATE TABLE o.newt` 同样拒绝，CTAS 目标
        // 沿用同一条写库面检查）。
        self.reject_attached_write(&table_name)?;

        // MS21 Iter005 (T5-c/D12)：名称唯一性跨基表与视图——对既有视图名
        // 建表计划期点名拒绝（执行期的基表重名检查不覆盖 kind=View 行）。
        if self.views.contains_key(&table_name) {
            return Err(PlanError::InvalidQuery(format!(
                "view '{table_name}' already exists"
            )));
        }

        // MS20 T4: 表形态诚实面。`LIKE other` / `CLONE other` 在解析层可表达，
        // 但引擎无对应能力——点名拒绝而不是静默建空表。
        if like.is_some() {
            return Err(PlanError::UnsupportedStatement(
                "CREATE TABLE ... LIKE".into(),
            ));
        }
        if clone.is_some() {
            return Err(PlanError::UnsupportedStatement(
                "CREATE TABLE ... CLONE".into(),
            ));
        }

        if let Some(query) = query {
            // 混合形态：列清单与 query 并存，语义（以谁为准）未定 —— 点名拒绝。
            if !columns.is_empty() {
                return Err(PlanError::UnsupportedStatement(
                    "CREATE TABLE ... AS WITH COLUMN DEFINITIONS".into(),
                ));
            }
            if !constraints.is_empty() {
                return Err(PlanError::UnsupportedStatement(
                    "CREATE TABLE ... AS WITH TABLE CONSTRAINTS".into(),
                ));
            }
            return self.build_create_table_as_select(table_name, query);
        }

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

        // MS29 T8: 重复列名计划期拒绝（列名已小写归一，同名与大小写变体同面）。
        // 该形态静默建成会使 dump→restore 必触发 MultiplePrimaryKey。
        for (index, col) in column_defs.iter().enumerate() {
            if column_defs[..index].iter().any(|c| c.name == col.name) {
                return Err(PlanError::InvalidQuery(format!(
                    "duplicate column name '{}'",
                    col.name
                )));
            }
        }

        // MS29 T4: 声明期 DEFAULT 类型校验/归一——NULL 豁免、FLOAT 列整数
        // 升格、日期族强制解析，其余跨类型点名拒绝（与写入值类型一致门同规
        // 则，消除「建表成功但应用默认值的 INSERT 必然失败」分裂）。
        for col in column_defs.iter_mut() {
            for constraint in col.constraints.iter_mut() {
                if let ColumnConstraint::DefaultValue(value) = constraint {
                    if let Some(normalized) =
                        validate_default_value(&col.name, &col.data_type, value)?
                    {
                        *value = normalized;
                    }
                }
            }
        }

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

    /// MS20 T4：CTAS 计划构造。
    ///
    /// 物化表列名取 SELECT 输出表头（既有 `get_plan_output_columns` 投影面），
    /// 序同输出形状。输出列为空（`SELECT` 无表头可达形态）点名拒绝——物化
    /// 零列表无首列主键语义。源计划构造在同一次 `PlanBuilder` 内完成，沿用既有
    /// 子查询/CTE 抑制与列提取校验面。
    fn build_create_table_as_select(
        &mut self,
        table_name: String,
        query: &sqlparser::ast::Query,
    ) -> Result<PhysicalPlan, PlanError> {
        let select = self.build_query(query)?;
        let columns = self.get_plan_output_columns(&select);
        if columns.is_empty() {
            return Err(PlanError::EmptyColumnDefinition);
        }
        // 输出列名重复 → 建表前点名拒绝（物化表列名必须唯一，否则 catalog 列行
        // 语义与列引用消歧都失去依据）。
        for (index, name) in columns.iter().enumerate() {
            if columns[..index].contains(name) {
                return Err(PlanError::UnsupportedStatement(format!(
                    "DUPLICATE OUTPUT COLUMN '{name}'"
                )));
            }
        }
        Ok(PhysicalPlan::CreateTableAsSelect(CreateTableAsSelectNode {
            table_name,
            select: Box::new(select),
            columns,
        }))
    }

    /// Build PhysicalPlan for DROP TABLE statement
    /// MS21 Iter003 (T3-b/c/D9/D10/D13)：`ALTER TABLE` 计划臂——单操作白名单
    /// （AddColumn / DropColumn），其余操作形态与多操作点名拒绝（D14 复用既有
    /// 变体）。目标表存在性经 builder 注册表（pipeline `register_table` 通道
    /// 先行——缺失即 42P01）；附加库目标点名拒绝；ADD 的约束声明 / NOT NULL
    /// × 非空 × 无 DEFAULT 计划期校验，列重复名拒绝。
    pub(crate) fn build_alter_table(
        &self,
        stmt: &sqlparser::ast::Statement,
    ) -> Result<PhysicalPlan, PlanError> {
        let sqlparser::ast::Statement::AlterTable {
            name,
            if_exists,
            only,
            operations,
            ..
        } = stmt
        else {
            return Err(PlanError::UnsupportedStatement("ALTER TABLE".into()));
        };
        // IF EXISTS / ONLY 修饰诚实化拒绝（假设 10 同型）
        if *if_exists {
            return Err(PlanError::UnsupportedStatement(
                "ALTER TABLE IF EXISTS".into(),
            ));
        }
        if *only {
            return Err(PlanError::UnsupportedStatement("ALTER TABLE ONLY".into()));
        }
        // 多操作点名拒绝（首版单操作）
        if operations.len() != 1 {
            return Err(PlanError::UnsupportedStatement(format!(
                "MULTI-OPERATION ALTER TABLE ({}, first version supports single-operation statements)",
                operations.len()
            )));
        }

        let table_name = object_name_to_table_name(name);
        // MS20 T3: 附加库一期只读。
        self.reject_attached_write(&table_name)?;

        // 表存在性（pipeline 通道下缺失在 register_table 即被点名拒绝，
        // 此处为直呼构造的防御臂）
        let column_list = self
            .tables
            .get(&table_name)
            .ok_or_else(|| PlanError::TableNotFound(table_name.clone()))?;

        let op = match &operations[0] {
            sqlparser::ast::AlterTableOperation::AddColumn {
                if_not_exists,
                column_def,
                ..
            } => {
                if *if_not_exists {
                    return Err(PlanError::UnsupportedStatement(
                        "ALTER TABLE ADD COLUMN IF NOT EXISTS".into(),
                    ));
                }
                let column_name = column_def.name.value.to_lowercase();
                if column_list.contains(&column_name) {
                    return Err(PlanError::InvalidQuery(format!(
                        "column '{}' already exists in table '{}'",
                        column_name, table_name
                    )));
                }
                // 约束声明矩阵：UNIQUE/CHECK/FK/方言项点名拒绝（MS23 复用）；
                // PRIMARY KEY（is_primary Unique）单独点名
                for option in &column_def.options {
                    match &option.option {
                        sqlparser::ast::ColumnOption::Unique {
                            is_primary: true, ..
                        } => {
                            return Err(PlanError::UnsupportedConstraint(
                                "PRIMARY KEY in ADD COLUMN",
                            ));
                        }
                        sqlparser::ast::ColumnOption::Unique {
                            is_primary: false, ..
                        } => {
                            return Err(PlanError::UnsupportedConstraint("UNIQUE in ADD COLUMN"));
                        }
                        sqlparser::ast::ColumnOption::Check(_) => {
                            return Err(PlanError::UnsupportedConstraint("CHECK in ADD COLUMN"));
                        }
                        sqlparser::ast::ColumnOption::ForeignKey { .. } => {
                            return Err(PlanError::UnsupportedConstraint(
                                "FOREIGN KEY in ADD COLUMN",
                            ));
                        }
                        sqlparser::ast::ColumnOption::DialectSpecific(_) => {
                            return Err(PlanError::UnsupportedConstraint(
                                "dialect-specific options in ADD COLUMN",
                            ));
                        }
                        _ => {}
                    }
                }
                let column_type = self.convert_data_type(&column_def.data_type)?;
                // NOT NULL / DEFAULT 提取（多项 NOT NULL/DEFAULT 视首项）
                let mut not_null = false;
                let mut default_value: Option<Value> = None;
                for option in &column_def.options {
                    match &option.option {
                        sqlparser::ast::ColumnOption::NotNull => not_null = true,
                        sqlparser::ast::ColumnOption::Default(expr) => {
                            if default_value.is_none() {
                                default_value = Some(self.extract_default_value(expr)?);
                            }
                        }
                        _ => {}
                    }
                }
                // MS29 T6: ADD COLUMN 声明 DEFAULT 适用同款校验/归一（与
                // CREATE TABLE 同一单点，D6 规则一致）。
                if let Some(v) = &default_value {
                    if let Some(normalized) = validate_default_value(&column_name, &column_type, v)?
                    {
                        default_value = Some(normalized);
                    }
                }
                // NOT NULL 无 DEFAULT 且表非空 → 计划期点名拒绝（存量行无法
                // 满足）；表非空判定为 plan_stage 注入的通道输入
                if not_null && default_value.is_none() {
                    let has_rows = self
                        .table_has_live_rows
                        .get(&table_name)
                        .copied()
                        .unwrap_or(false);
                    if has_rows {
                        return Err(PlanError::InvalidQuery(format!(
                            "NOT NULL column '{}' without DEFAULT cannot be added to non-empty table '{}' (existing rows cannot satisfy the constraint)",
                            column_name, table_name
                        )));
                    }
                }
                AlterTableOp::AddColumn {
                    column_name,
                    column_type,
                    not_null,
                    default_value,
                }
            }
            sqlparser::ast::AlterTableOperation::DropColumn {
                column_name,
                if_exists,
                cascade,
                ..
            } => {
                if *if_exists {
                    return Err(PlanError::UnsupportedStatement(
                        "ALTER TABLE DROP COLUMN IF EXISTS".into(),
                    ));
                }
                if *cascade {
                    return Err(PlanError::UnsupportedStatement(
                        "ALTER TABLE DROP COLUMN CASCADE".into(),
                    ));
                }
                let column_name = column_name.value.to_lowercase();
                let col_index = column_list
                    .iter()
                    .position(|c| *c == column_name)
                    .ok_or_else(|| PlanError::ColumnNotFound(column_name.clone()))?;
                // 守卫矩阵：最后一列 / PK / 唯一声明 / 二级索引引用
                if column_list.len() == 1 {
                    return Err(PlanError::InvalidQuery(
                        "cannot drop the last column of a table (tables must keep at least one column)"
                            .into(),
                    ));
                }
                // PK 列：经 builder 注册的 PK 类型通道在 T3-c 消费；此处仅列
                // 存在性校验，守卫臂由 build 层的独立通道判定（见 T3-c）。
                let _ = col_index;
                AlterTableOp::DropColumn { column_name }
            }
            other => {
                return Err(PlanError::UnsupportedStatement(format!(
                    "ALTER TABLE {} (first version supports ADD COLUMN / DROP COLUMN only)",
                    alter_operation_name(other)
                )));
            }
        };

        Ok(PhysicalPlan::AlterTable(AlterTableNode {
            table_name,
            operation: op,
        }))
    }

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
        // MS20 T3: 附加库一期只读。
        self.reject_attached_write(&table_name)?;

        Ok(PhysicalPlan::DropTable(DropTableNode {
            table_name,
            if_exists: *if_exists,
        }))
    }

    /// MS21 Iter004 (T4-a)：`TRUNCATE TABLE` 计划臂——物理清空保留定义
    /// （design D11）。白名单：单表目标、无 Hive `PARTITION` 清单；修饰面
    /// 与目标边界全部点名拒绝（D14：复用既有变体，不新增 `PlanError`）。
    /// 表存在性经 builder 注册表（plan_stage 的 register_table 通道先行——
    /// 缺失表/系统保留名/附加库引用即被点名拒绝，此处为直呼构造的防御臂）。
    pub(crate) fn build_truncate(
        &self,
        stmt: &sqlparser::ast::Statement,
    ) -> Result<PhysicalPlan, PlanError> {
        let sqlparser::ast::Statement::Truncate {
            table_name,
            partitions,
            ..
        } = stmt
        else {
            return Err(PlanError::UnsupportedStatement("TRUNCATE".into()));
        };
        // Hive 分区清单点名拒绝（本库无分区概念）
        if partitions.is_some() {
            return Err(PlanError::UnsupportedStatement(
                "TRUNCATE PARTITION (partition lists are not supported)".into(),
            ));
        }
        let table_name = object_name_to_table_name(table_name);
        // MS20 T3: 附加库一期只读。
        self.reject_attached_write(&table_name)?;

        // 表存在性（pipeline 通道下缺失在 register_table 即被点名拒绝，
        // 此处为直呼构造的防御臂）
        self.tables
            .get(&table_name)
            .ok_or_else(|| PlanError::TableNotFound(table_name.clone()))?;

        Ok(PhysicalPlan::Truncate(TruncateNode { table_name }))
    }

    /// MS21 Iter005 (T5-b/D12)：`CREATE VIEW` 计划臂——非物化只读视图。
    ///
    /// 持久化三要素在计划期定形：视图名（小写归一、非限定）、定义 SQL 文本
    /// （`Query::to_string()` 规范化序列化——展开期经 `parse_sql` 重解析）、
    /// 列名清单（显式清单覆盖，缺省取定义输出表头）。创建期对定义
    /// `build_query` 验证（CTAS 先例）——引用必须是已存在的基表或已存在
    /// 视图（缺失 42P01 / 42703 自然上抛），「引用尚不存在的视图」在创建期
    /// 即拒 → 循环引用结构性不可能，运行期无需环检测。修饰面 / 限定名 /
    /// 列选项 / 重名 / 超长定义全部计划期点名拒绝（D14：复用既有变体）。
    pub(crate) fn build_create_view(
        &mut self,
        stmt: &sqlparser::ast::Statement,
    ) -> Result<PhysicalPlan, PlanError> {
        let sqlparser::ast::Statement::CreateView {
            or_replace,
            materialized,
            name,
            columns,
            query,
            options,
            cluster_by,
            with_no_schema_binding,
            if_not_exists,
            temporary,
        } = stmt
        else {
            return Err(PlanError::UnsupportedStatement("CREATE VIEW".into()));
        };

        // 修饰面拒绝矩阵（能力点名，MS19 错误面先例；不静默忽略成分）
        if *or_replace {
            return Err(PlanError::UnsupportedStatement(
                "CREATE OR REPLACE VIEW".into(),
            ));
        }
        if *materialized {
            return Err(PlanError::UnsupportedStatement("MATERIALIZED VIEW".into()));
        }
        if *if_not_exists {
            return Err(PlanError::UnsupportedStatement(
                "CREATE VIEW IF NOT EXISTS".into(),
            ));
        }
        if *temporary {
            return Err(PlanError::UnsupportedStatement("TEMPORARY VIEW".into()));
        }
        if !cluster_by.is_empty() {
            return Err(PlanError::UnsupportedStatement(
                "CREATE VIEW CLUSTER BY".into(),
            ));
        }
        if *with_no_schema_binding {
            return Err(PlanError::UnsupportedStatement(
                "CREATE VIEW WITH NO SCHEMA BINDING".into(),
            ));
        }
        if !matches!(options, sqlparser::ast::CreateTableOptions::None) {
            return Err(PlanError::UnsupportedStatement(
                "CREATE VIEW WITH/OPTIONS (storage options)".into(),
            ));
        }

        // 视图名：单段、非限定（无 schema 层级）；系统保留名拒绝（与
        // CREATE TABLE 同面——视图名与表名共享名称空间纪律）
        let parts = object_name_parts(name);
        if parts.len() != 1 {
            return Err(PlanError::UnsupportedStatement(
                "QUALIFIED VIEW NAME (no schema hierarchy)".into(),
            ));
        }
        let view_name = parts[0].to_lowercase();
        if view_name == crate::storage::TABLES_SYSTEM_NAME
            || view_name == crate::storage::COLUMNS_SYSTEM_NAME
        {
            return Err(PlanError::InvalidQuery(format!(
                "'{view_name}' is a reserved system table name"
            )));
        }

        // 重名：跨基表与视图（名称唯一性不变量；点名拒绝零副作用）
        if self.tables.contains_key(&view_name) {
            return Err(PlanError::InvalidQuery(format!(
                "table '{view_name}' already exists"
            )));
        }
        if self.views.contains_key(&view_name) {
            return Err(PlanError::InvalidQuery(format!(
                "view '{view_name}' already exists"
            )));
        }

        // 列名清单形态：仅裸标识符（列级选项点名拒绝）
        for col in columns {
            if col.options.is_some() {
                return Err(PlanError::UnsupportedStatement(
                    "VIEW COLUMN OPTIONS".into(),
                ));
            }
        }

        // 创建期定义验证（CTAS 先例）：引用缺失 42P01 / 列缺失 42703 自然
        // 上抛；对尚不存在视图的引用也在此 42P01（T5-c 展开钩子未命中的
        // 名字走基表 miss 通道）。
        let definition_plan = self.build_query(query)?;
        let output_columns = self.get_plan_output_columns(&definition_plan);
        if output_columns.is_empty() {
            return Err(PlanError::InvalidQuery(
                "view definition outputs no columns".into(),
            ));
        }
        // 输出列名重复 → 建视图前点名拒绝（CTAS 同型——catalog 列行语义与
        // 列引用消歧都失去依据）
        for (index, col) in output_columns.iter().enumerate() {
            if output_columns[..index].contains(col) {
                return Err(PlanError::UnsupportedStatement(format!(
                    "DUPLICATE OUTPUT COLUMN '{col}'"
                )));
            }
        }

        // 列名清单 = 显式清单（长度须匹配定义输出宽度、无重复名）或定义
        // 输出表头
        let final_columns: Vec<String> = if columns.is_empty() {
            output_columns
        } else {
            if columns.len() != output_columns.len() {
                return Err(PlanError::InvalidQuery(format!(
                    "view column list has {} names but the definition outputs {} columns",
                    columns.len(),
                    output_columns.len()
                )));
            }
            let names: Vec<String> = columns
                .iter()
                .map(|c| c.name.value.to_lowercase())
                .collect();
            for (index, col) in names.iter().enumerate() {
                if names[..index].contains(col) {
                    return Err(PlanError::InvalidQuery(format!(
                        "duplicate column name '{col}' in view column list"
                    )));
                }
            }
            names
        };

        // 定义 SQL 文本 = `Query::to_string()` 规范化序列化；超长前置点名
        // 拒绝（catalog 行单 slot 容量上限，不达执行期 PageFull）
        let sql_text = query.to_string();
        if sql_text.len() > MAX_VIEW_SQL_BYTES {
            return Err(PlanError::InvalidQuery(format!(
                "view definition SQL text too long ({} > {} bytes)",
                sql_text.len(),
                MAX_VIEW_SQL_BYTES
            )));
        }

        Ok(PhysicalPlan::CreateView(CreateViewNode {
            view_name,
            sql: sql_text,
            columns: final_columns,
        }))
    }

    /// MS21 Iter005 (T5-d/D12)：`DROP VIEW` 计划臂——单名、非限定、
    /// `IF EXISTS` 点名拒绝（Forbidden）；视图不存在点名拒绝（plan_stage
    /// 视图通道注入的名单判定，直呼构造同面；D14：视图不存在 →
    /// `TableNotFound` 42P01）。基表名不是视图操作目标——按视图不存在点名。
    pub(crate) fn build_drop_view(
        &self,
        names: &[sqlparser::ast::ObjectName],
        if_exists: &bool,
    ) -> Result<PhysicalPlan, PlanError> {
        if *if_exists {
            return Err(PlanError::UnsupportedStatement(
                "DROP VIEW IF EXISTS".into(),
            ));
        }
        if names.len() != 1 {
            return Err(PlanError::UnsupportedStatement(format!(
                "MULTI-OBJECT DROP VIEW ({}, first version supports single-object statements)",
                names.len()
            )));
        }
        let parts = object_name_parts(&names[0]);
        if parts.len() != 1 {
            return Err(PlanError::UnsupportedStatement(
                "QUALIFIED VIEW NAME (no schema hierarchy)".into(),
            ));
        }
        let view_name = parts[0].to_lowercase();
        if !self.views.contains_key(&view_name) {
            return Err(PlanError::TableNotFound(view_name));
        }
        Ok(PhysicalPlan::DropView(DropViewNode { view_name }))
    }

    /// MS21 Iter002 (T2-c)：`CREATE INDEX` 计划臂——白名单（单列、非唯一、
    /// 声明 INT 列、显式索引名）+ 全形态修饰点名拒绝（D14：复用既有变体，
    /// 不新增 `PlanError`）。表存在性经 builder 注册表（plan_stage 的
    /// register_table 通道先行——缺失即 `TableNotFound` 42P01，此处为直呼
    /// 构造的防御臂）；列存在性 / INT 声明 / 库级重名计划期点名拒绝零副作用。
    /// 对 PK 列建索引允许（冗余但行为等价，不做专门检测）。
    pub(crate) fn build_create_index(
        &self,
        stmt: &sqlparser::ast::Statement,
    ) -> Result<PhysicalPlan, PlanError> {
        let sqlparser::ast::Statement::CreateIndex {
            name,
            table_name,
            using,
            columns,
            unique,
            concurrently,
            if_not_exists,
            include,
            nulls_distinct,
            predicate,
        } = stmt
        else {
            return Err(PlanError::UnsupportedStatement("CREATE INDEX".into()));
        };

        // 修饰面拒绝矩阵（能力点名，MS19 错误面先例；不静默忽略成分）
        if *unique {
            return Err(PlanError::UnsupportedStatement(
                "CREATE UNIQUE INDEX".into(),
            ));
        }
        if *concurrently {
            return Err(PlanError::UnsupportedStatement(
                "CREATE INDEX CONCURRENTLY".into(),
            ));
        }
        if *if_not_exists {
            return Err(PlanError::UnsupportedStatement(
                "CREATE INDEX IF NOT EXISTS".into(),
            ));
        }
        if using.is_some() {
            return Err(PlanError::UnsupportedStatement(
                "CREATE INDEX USING (access method)".into(),
            ));
        }
        if !include.is_empty() {
            return Err(PlanError::UnsupportedStatement(
                "CREATE INDEX INCLUDE".into(),
            ));
        }
        if nulls_distinct.is_some() {
            return Err(PlanError::UnsupportedStatement(
                "CREATE INDEX NULLS DISTINCT".into(),
            ));
        }
        if predicate.is_some() {
            return Err(PlanError::UnsupportedStatement(
                "PARTIAL INDEX (WHERE clause)".into(),
            ));
        }
        if columns.len() != 1 {
            return Err(PlanError::UnsupportedStatement("MULTI-COLUMN INDEX".into()));
        }

        // 显式索引名（sqlparser 的 `CREATE INDEX ON t(c)` 自动命名形态不
        // 支持——MissingField 点名）；限定名（schema.idx）拒绝（无 schema 层级）
        let Some(name) = name else {
            return Err(PlanError::MissingField("index name".into()));
        };
        let parts = object_name_parts(name);
        if parts.len() != 1 {
            return Err(PlanError::UnsupportedStatement(
                "QUALIFIED INDEX NAME (no schema hierarchy)".into(),
            ));
        }
        let index_name = parts[0].to_lowercase();

        let table_name = object_name_to_table_name(table_name);
        // MS20 T3: 附加库一期只读。
        self.reject_attached_write(&table_name)?;

        // 表存在性（直呼构造的防御臂；pipeline 通道下缺失在 register_table
        // 即被点名拒绝，不达此处）
        if !self.tables.contains_key(&table_name) {
            return Err(PlanError::TableNotFound(table_name));
        }

        // 单列表达式形态：仅裸列名（表达式索引点名拒绝）
        let column_expr = &columns[0].expr;
        let Expr::Identifier(ident) = column_expr else {
            return Err(PlanError::UnsupportedStatement("EXPRESSION INDEX".into()));
        };
        let column_name = ident.value.to_lowercase();
        let column_list = &self.tables[&table_name];
        let Some(column_index) = column_list
            .iter()
            .position(|c| c.to_lowercase() == column_name)
        else {
            return Err(PlanError::ColumnNotFound(column_name));
        };

        // 声明类型门：首版仅 INT 列（写入类型门保证 INT 列无非 Int 值）
        let declared_int = self
            .table_column_types
            .get(&table_name)
            .and_then(|types| types.get(column_index))
            .is_some_and(|ct| matches!(ct, crate::storage::page_format::ColumnType::Int));
        if !declared_int {
            return Err(PlanError::UnsupportedStatement(format!(
                "INDEX ON NON-INT COLUMN '{}' (first version supports INT columns only)",
                column_name
            )));
        }

        // 库级重名（plan_stage 注入的全库名单；点名拒绝零副作用）
        if self.existing_index_names.contains(&index_name) {
            return Err(PlanError::InvalidQuery(format!(
                "index '{}' already exists",
                index_name
            )));
        }

        Ok(PhysicalPlan::CreateIndex(CreateIndexNode {
            index_name,
            table_name,
            column_index,
            column_name,
        }))
    }

    /// MS21 Iter002 (T2-f)：`DROP INDEX` 计划臂——单名、非限定、`IF EXISTS`
    /// 点名拒绝（假设 10 诚实化）；索引不存在按计划期名单点名拒绝。
    pub(crate) fn build_drop_index(
        &self,
        names: &[sqlparser::ast::ObjectName],
        if_exists: &bool,
    ) -> Result<PhysicalPlan, PlanError> {
        if names.is_empty() {
            return Err(PlanError::MissingField("index name".into()));
        }
        if names.len() > 1 {
            return Err(PlanError::UnsupportedStatement(
                "MULTIPLE INDEX NAMES".into(),
            ));
        }
        let parts = object_name_parts(&names[0]);
        if parts.len() != 1 {
            return Err(PlanError::UnsupportedStatement(
                "QUALIFIED INDEX NAME (no schema hierarchy)".into(),
            ));
        }
        if *if_exists {
            return Err(PlanError::UnsupportedStatement(
                "DROP INDEX IF EXISTS".into(),
            ));
        }
        let index_name = parts[0].to_lowercase();
        if !self.existing_index_names.contains(&index_name) {
            return Err(PlanError::InvalidQuery(format!(
                "index '{}' does not exist",
                index_name
            )));
        }
        Ok(PhysicalPlan::DropIndex(DropIndexNode { index_name }))
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
        // MS21 Iter005 (T5-c/D12)：视图只读（先于 validate_table，同 INSERT）。
        self.reject_view_write(&table_name)?;
        self.validate_table(&table_name)?;
        // MS20-T02: 附加库目标不再计划期拒绝（见 `build_insert`）。

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
            return Err(PlanError::UnsupportedStatement(
                "MULTI-COLUMN UPDATE".into(),
            ));
        }

        let assignment = &assignments[0];

        // Extract column name
        if assignment.id.len() != 1 {
            return Err(PlanError::UnsupportedStatement(
                "COMPOUND ASSIGNMENT TARGET".into(),
            ));
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
        // MS21 Iter005 (T5-c/D12)：视图只读（先于 validate_table，同 INSERT）。
        self.reject_view_write(&table_name)?;
        self.validate_table(&table_name)?;
        // MS20-T02: 附加库目标不再计划期拒绝（见 `build_insert`）。

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

/// MS21 Iter003 (T3-b)：拒绝面文案的 ALTER 操作能力名（D14 点名形态）。
fn alter_operation_name(op: &sqlparser::ast::AlterTableOperation) -> String {
    match op {
        sqlparser::ast::AlterTableOperation::AddConstraint(_) => "ADD CONSTRAINT".into(),
        sqlparser::ast::AlterTableOperation::AddColumn { .. } => "ADD COLUMN".into(),
        sqlparser::ast::AlterTableOperation::DisableRowLevelSecurity => {
            "DISABLE ROW LEVEL SECURITY".into()
        }
        sqlparser::ast::AlterTableOperation::DisableRule { .. } => "DISABLE RULE".into(),
        sqlparser::ast::AlterTableOperation::DisableTrigger { .. } => "DISABLE TRIGGER".into(),
        sqlparser::ast::AlterTableOperation::DropConstraint { .. } => "DROP CONSTRAINT".into(),
        sqlparser::ast::AlterTableOperation::DropColumn { .. } => "DROP COLUMN".into(),
        sqlparser::ast::AlterTableOperation::DropPrimaryKey => "DROP PRIMARY KEY".into(),
        sqlparser::ast::AlterTableOperation::EnableAlwaysRule { .. } => "ENABLE ALWAYS RULE".into(),
        sqlparser::ast::AlterTableOperation::EnableAlwaysTrigger { .. } => {
            "ENABLE ALWAYS TRIGGER".into()
        }
        sqlparser::ast::AlterTableOperation::EnableReplicaRule { .. } => {
            "ENABLE REPLICA RULE".into()
        }
        sqlparser::ast::AlterTableOperation::EnableReplicaTrigger { .. } => {
            "ENABLE REPLICA TRIGGER".into()
        }
        sqlparser::ast::AlterTableOperation::EnableRowLevelSecurity => {
            "ENABLE ROW LEVEL SECURITY".into()
        }
        sqlparser::ast::AlterTableOperation::RenameTable { .. } => "RENAME".into(),
        _ => format!("{}", op),
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

    // ---- MS20-T02 Iter001 (T4): INSERT ... SELECT 计划面 ----

    /// 无清单 INSERT SELECT：源以顶层上下文编译为子树，节点携带源宽度、
    /// 恒等位置映射与全宽 NULL 填充（无声明 DEFAULT）。
    #[test]
    fn test_insert_select_plan_shape() {
        use crate::executor::InsertSelectNode;
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into(), "b".into()], "a");

        let sql = "INSERT INTO users SELECT a, b FROM src";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::InsertSelect(node) => {
                let InsertSelectNode {
                    table_name,
                    columns,
                    source,
                    source_width,
                    target_positions,
                    fill,
                    upsert,
                } = node;
                assert_eq!(table_name, "users");
                assert!(columns.is_empty(), "no list → identity mapping");
                assert_eq!(source_width, 2);
                assert_eq!(target_positions, vec![0, 1]);
                assert_eq!(fill, vec![Value::Null, Value::Null]);
                assert!(upsert.is_none());
                // 源子树是完整 SELECT 计划（DataScan/Scan 形态），非空。
                assert!(matches!(
                    source.as_ref(),
                    PhysicalPlan::DataScan(_) | PhysicalPlan::Scan(_) | PhysicalPlan::Filter(_)
                ));
            }
            other => panic!("Expected InsertSelect plan, got {other:?}"),
        }
    }

    /// 子集重排列清单：位置映射按清单→表列（name→1，id→0）；省略位填充
    /// 落声明 DEFAULT。
    #[test]
    fn test_insert_select_column_mapping_subset_reorder() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into(), "b".into()], "a");

        let sql = "INSERT INTO users (name, id) SELECT b, a FROM src";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::InsertSelect(node) => {
                assert_eq!(node.columns, vec!["name".to_string(), "id".to_string()]);
                assert_eq!(node.target_positions, vec![1, 0]);
                assert_eq!(node.source_width, 2);
                // 省略位无（全列清单）——fill 仍为表宽全 NULL。
                assert_eq!(node.fill, vec![Value::Null, Value::Null]);
            }
            other => panic!("Expected InsertSelect plan, got {other:?}"),
        }
    }

    /// 带声明 DEFAULT 的省略位：fill 按表列声明（x DEFAULT 7 → Int(7)）。
    #[test]
    fn test_insert_select_fill_uses_declared_defaults() {
        let mut builder = PlanBuilder::new();
        builder.register_table("d", vec!["id".into(), "x".into()], "id");
        builder.set_table_defaults("d", vec![None, Some(Value::Int(7))]);
        builder.register_table("src", vec!["a".into()], "a");

        let sql = "INSERT INTO d (id) SELECT a FROM src";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::InsertSelect(node) => {
                assert_eq!(node.target_positions, vec![0]);
                assert_eq!(node.source_width, 1);
                assert_eq!(node.fill, vec![Value::Null, Value::Int(7)]);
            }
            other => panic!("Expected InsertSelect plan, got {other:?}"),
        }
    }

    /// 宽度不符计划期拒绝：清单数与源宽度、表宽与源宽度两形态（空源同样
    /// 成立——宽度来自计划而非行）。
    #[test]
    fn test_insert_select_width_mismatch_rejected() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into(), "b".into()], "a");

        // 清单 1 列 vs 源 2 列。
        let stmts = parse_sql("INSERT INTO users (id) SELECT a, b FROM src").unwrap();
        let err = builder.build_plan(&stmts[0]).unwrap_err().to_string();
        assert!(
            err.contains("expects 1 values, got 2"),
            "list-vs-source width mismatch must be named, got: {err}"
        );

        // 无清单：表 2 列 vs 源 1 列。
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into()], "a");
        let stmts = parse_sql("INSERT INTO users SELECT a FROM src").unwrap();
        let err = builder.build_plan(&stmts[0]).unwrap_err().to_string();
        assert!(
            err.contains("expects 2 values, got 1"),
            "table-vs-source width mismatch must be named, got: {err}"
        );
    }

    /// 未知目标列计划期拒绝（ColumnNotFound，与 VALUES 通道同面）。
    #[test]
    fn test_insert_select_unknown_column_rejected() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into()], "a");

        let stmts = parse_sql("INSERT INTO users (nope) SELECT a FROM src").unwrap();
        let err = builder.build_plan(&stmts[0]).unwrap_err().to_string();
        assert!(
            err.contains("Column not found: nope"),
            "unknown list column must be named, got: {err}"
        );
    }

    /// 重复目标列计划期拒绝（与 VALUES 通道同文案）。
    #[test]
    fn test_insert_select_duplicate_column_rejected() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into(), "b".into()], "a");

        let stmts = parse_sql("INSERT INTO users (id, id) SELECT a, b FROM src").unwrap();
        let err = builder.build_plan(&stmts[0]).unwrap_err().to_string();
        assert!(
            err.contains("Duplicate column 'id' in INSERT column list"),
            "duplicate list column must be named, got: {err}"
        );
    }

    /// 空源结构合法：LIMIT 0 源计划期通过（宽度来自计划，零行是运行期形态）。
    #[test]
    fn test_insert_select_empty_source_structurally_valid() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("src", vec!["a".into(), "b".into()], "a");

        let sql = "INSERT INTO users SELECT a, b FROM src LIMIT 0";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();
        match plan {
            PhysicalPlan::InsertSelect(node) => {
                assert_eq!(node.source_width, 2);
                assert_eq!(node.target_positions, vec![0, 1]);
            }
            other => panic!("Expected InsertSelect plan, got {other:?}"),
        }
    }

    /// REPLACE INTO ... SELECT 与 ON CONFLICT ... SELECT 复用既有仲裁/动作
    /// 解析（build_upsert_action）。
    #[test]
    fn test_insert_select_replace_and_conflict_arbiter() {
        use crate::executor::{ConflictAction, ConflictArbiter, InsertSelectNode};
        use crate::storage::page_format::ColumnType;
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.set_pk_column_type("users", ColumnType::Int);
        builder.register_table("src", vec!["a".into(), "b".into()], "a");

        let sql = "REPLACE INTO users SELECT a, b FROM src";
        let stmts = parse_sql(sql).unwrap();
        match builder.build_plan(&stmts[0]).unwrap() {
            PhysicalPlan::InsertSelect(InsertSelectNode {
                upsert: Some((arbiter, action)),
                ..
            }) => {
                assert!(matches!(arbiter, ConflictArbiter::All));
                assert!(matches!(action, ConflictAction::Replace));
            }
            other => panic!("Expected InsertSelect with Replace, got {other:?}"),
        }

        let sql = "INSERT INTO users SELECT a, b FROM src ON CONFLICT (id) DO NOTHING";
        let stmts = parse_sql(sql).unwrap();
        match builder.build_plan(&stmts[0]).unwrap() {
            PhysicalPlan::InsertSelect(InsertSelectNode {
                upsert: Some((arbiter, action)),
                ..
            }) => {
                assert!(matches!(arbiter, ConflictArbiter::Column(0)));
                assert!(matches!(action, ConflictAction::DoNothing));
            }
            other => panic!("Expected InsertSelect with DO NOTHING, got {other:?}"),
        }
    }
}
