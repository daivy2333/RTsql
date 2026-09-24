//! PlanBuilder — `build_expression` / `build_where` / `resolve_column_ref`
//! and the free function `expr_to_column_name`.
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::datetime::{interval_parts_from_unit, parse_interval_string, IntervalParts};
use crate::executor::{
    check_scalar_function, is_scalar_function, ArithOp, BinaryArithExpression, CaseExpression,
    CastExpression, CastType, CoalesceExpression, ColumnExpression, ColumnRef, ComparisonOp,
    ComparisonPredicate, ConstantExpression, ExpressionRef, FunctionExpression,
    IntervalArithExpression, IsNullPredicate, LikePredicate, LogicalOp, LogicalPredicate,
    NotPredicate, ParameterExpression, PredicateRef, Value,
};
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::Expr;
use std::sync::Arc;

impl PlanBuilder {
    /// 解析列引用（支持 t.col 格式和纯列名）
    pub(crate) fn resolve_column_ref(
        &self,
        expr: &Expr,
        available_tables: &[String],
    ) -> Result<ColumnRef, PlanError> {
        match expr {
            // t.col 格式
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table = parts[0].value.to_lowercase();
                let column = parts[1].value.to_lowercase();

                // 验证表存在
                self.validate_table(&table)?;

                // 验证列存在
                let columns = self
                    .tables
                    .get(&table)
                    .ok_or_else(|| PlanError::TableNotFound(table.clone()))?;
                if !columns.iter().any(|c| c.to_lowercase() == column) {
                    return Err(PlanError::ColumnNotFound(column));
                }

                Ok(ColumnRef {
                    table: Some(table),
                    column,
                })
            }

            // 纯列名格式
            Expr::Identifier(ident) => {
                let column = ident.value.to_lowercase();

                // 查找列来源（检查所有可用表）
                let sources: Vec<String> = available_tables
                    .iter()
                    .filter(|t| {
                        self.tables
                            .get(*t)
                            .map(|cols| cols.iter().any(|c| c.to_lowercase() == column))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();

                match sources.len() {
                    0 => Err(PlanError::ColumnNotFound(column)),
                    1 => Ok(ColumnRef {
                        table: None,
                        column,
                    }),
                    _ => Err(PlanError::AmbiguousColumn(column)),
                }
            }

            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Convert sqlparser BinaryOperator to ComparisonOp
    pub(crate) fn convert_comparison_op(
        &self,
        op: &sqlparser::ast::BinaryOperator,
    ) -> Option<ComparisonOp> {
        use sqlparser::ast::BinaryOperator as SqlOp;
        match op {
            SqlOp::Eq => Some(ComparisonOp::Eq),
            SqlOp::NotEq => Some(ComparisonOp::Ne),
            SqlOp::Gt => Some(ComparisonOp::Gt),
            SqlOp::Lt => Some(ComparisonOp::Lt),
            SqlOp::GtEq => Some(ComparisonOp::Ge),
            SqlOp::LtEq => Some(ComparisonOp::Le),
            _ => None,
        }
    }

    /// MS11-T01: strict CAST target mapping — the four supported families only.
    /// Unlike `convert_data_type` (DDL, unknown → String fallback), an unknown
    /// CAST target type is a planning error.
    pub(crate) fn convert_cast_data_type(
        &self,
        data_type: &sqlparser::ast::DataType,
    ) -> Result<CastType, PlanError> {
        use sqlparser::ast::{DataType, TimezoneInfo};
        Ok(match data_type {
            DataType::Int(_)
            | DataType::Int4(_)
            | DataType::Integer(_)
            | DataType::BigInt(_)
            | DataType::Int8(_)
            | DataType::SmallInt(_)
            | DataType::Int2(_)
            | DataType::TinyInt(_)
            | DataType::MediumInt(_) => CastType::Int,

            DataType::Varchar(_)
            | DataType::Nvarchar(_)
            | DataType::Char(_)
            | DataType::Character(_)
            | DataType::CharacterVarying(_)
            | DataType::CharVarying(_)
            | DataType::String(_)
            | DataType::Text
            | DataType::Clob(_)
            | DataType::CharacterLargeObject(_)
            | DataType::CharLargeObject(_) => CastType::String,

            DataType::Float(_)
            | DataType::Float4
            | DataType::Float64
            | DataType::Real
            | DataType::Double
            | DataType::Float8
            | DataType::DoublePrecision => CastType::Float,

            DataType::Bool | DataType::Boolean => CastType::Bool,

            // MS13 T5: 日期族 CAST 目标（datetime-type-system R5）；带时区
            // 变体落入下方既有未知类型拒绝。
            DataType::Date => CastType::Date,
            DataType::Datetime(_) => CastType::Timestamp,
            DataType::Timestamp(_, TimezoneInfo::None) => CastType::Timestamp,

            other => {
                return Err(PlanError::ParseError(format!(
                    "Unsupported CAST target type: {}",
                    other
                )))
            }
        })
    }

    /// MS13 T7: 解缠绕 sqlparser INTERVAL 吞比较形态。`d + INTERVAL '1 day' > X`
    /// 被 sqlparser 解析为 `Plus(d, Interval{ value: Gt('1 day', X) })`——
    /// 算术腿是谓词顶层而比较被埋进 interval.value。本 helper 仅在该形态
    /// 命中时还原为 `IntervalArith(d, ±, '1 day') <比较> X` 谓词；其余形状
    /// 返回 `None` 交回既有比较臂（interval 自身畸形仍经 parse 点名拒绝）。
    fn unswallow_interval_comparison(
        &self,
        table_name: &str,
        left: &Expr,
        op: &sqlparser::ast::BinaryOperator,
        right: &Expr,
    ) -> Result<Option<PredicateRef>, PlanError> {
        use sqlparser::ast::BinaryOperator as SqlOp;
        let arith_op = match op {
            SqlOp::Plus => ArithOp::Add,
            SqlOp::Minus => ArithOp::Sub,
            _ => return Ok(None),
        };
        let Expr::Interval(interval) = right else {
            return Ok(None);
        };
        let Expr::BinaryOp {
            left: lit,
            op: cmp_op,
            right: rest,
        } = interval.value.as_ref()
        else {
            return Ok(None);
        };
        // 仅比较操作符命中；AND/OR 等更低优先级不会进入 value（解析即终止）
        let Some(comp_op) = self.convert_comparison_op(cmp_op) else {
            return Ok(None);
        };
        // 重建仅含字面量值的 Interval 并按既有解析路径取区间值
        let rebuilt = sqlparser::ast::Interval {
            value: Box::new((**lit).clone()),
            leading_field: interval.leading_field,
            leading_precision: interval.leading_precision,
            last_field: interval.last_field,
            fractional_seconds_precision: interval.fractional_seconds_precision,
        };
        let parts = parse_interval_expr(&rebuilt)?;
        let left_expr = Arc::new(IntervalArithExpression {
            left: self.build_expression(table_name, left)?,
            op: arith_op,
            interval: parts,
        });
        let right_expr = self.build_expression(table_name, rest)?;
        Ok(Some(Arc::new(ComparisonPredicate {
            left: left_expr,
            op: comp_op,
            right: right_expr,
        })))
    }

    /// MS11-T03: shared body of the CEIL/FLOOR dedicated-variant arms. Only    /// the plain `fn(x)` form is supported; `fn(x TO field)` is rejected by
    /// name (mirrors the TRIM specification-form rejection).
    fn build_ceil_floor(
        &self,
        name: &str,
        expr: &Expr,
        field: &sqlparser::ast::DateTimeField,
        table_name: &str,
    ) -> Result<ExpressionRef, PlanError> {
        if !matches!(field, sqlparser::ast::DateTimeField::NoDateTime) {
            return Err(PlanError::ParseError(format!(
                "{name} with a TO date-time field is not supported; only {name}(x) is supported"
            )));
        }
        Ok(Arc::new(FunctionExpression {
            name: name.to_string(),
            args: vec![self.build_expression(table_name, expr)?],
        }))
    }

    /// Build ExpressionRef from Expr
    pub(crate) fn build_expression(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<ExpressionRef, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let ident_value = ident.value.to_uppercase();
                // Check for NULL constant
                if ident_value == "NULL" {
                    return Ok(Arc::new(ConstantExpression { value: Value::Null }));
                }
                // MS09-T02: NLJ combined-row layout override (design D5a).
                // While set, unqualified names resolve against a full-layout
                // search into absolute combined-row indices; `self.tables` is
                // not consulted on this path.
                if let Some(layout) = &self.join_column_layout {
                    let column_name = ident.value.to_lowercase();
                    let mut hit: Option<(usize, usize)> = None;
                    let mut ambiguous = false;
                    let mut offset = 0;
                    for (_table, columns) in layout {
                        if let Some(pos) =
                            columns.iter().position(|c| c.to_lowercase() == column_name)
                        {
                            if hit.is_some() {
                                ambiguous = true;
                                break;
                            }
                            hit = Some((offset, pos));
                        }
                        offset += columns.len();
                    }
                    return match (hit, ambiguous) {
                        (_, true) => Err(PlanError::AmbiguousColumn(column_name)),
                        (Some((table_offset, pos)), false) => Ok(Arc::new(ColumnExpression {
                            column_name,
                            column_index: table_offset + pos,
                        })),
                        (None, false) => Err(PlanError::ColumnNotFound(column_name)),
                    };
                }
                // Column reference
                let column_name = ident.value.to_lowercase();
                let columns = self.tables.get(table_name).ok_or_else(|| {
                    PlanError::ParseError(format!("Table '{}' not found", table_name))
                })?;
                let column_index = columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                    .ok_or_else(|| {
                        PlanError::ParseError(format!(
                            "Column '{}' not found in table '{}'",
                            column_name, table_name
                        ))
                    })?;
                Ok(Arc::new(ColumnExpression {
                    column_name,
                    column_index,
                }))
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table_ref = parts[0].value.to_lowercase();
                let column_name = parts[1].value.to_lowercase();

                // Check if this is an outer (correlated) reference
                if let Some(ref inner_tables) = self.inner_table_names {
                    if !inner_tables
                        .iter()
                        .any(|t| t.eq_ignore_ascii_case(&table_ref))
                    {
                        let param_name = format!("{}.{}", table_ref, column_name);
                        return Ok(Arc::new(ParameterExpression::new(param_name)));
                    }
                }

                // MS09-T02: NLJ combined-row layout override (design D5a) —
                // qualified names resolve to their table's layout offset +
                // in-table position. Correlated outer refs were returned
                // above, so this only sees the join's own tables.
                if let Some(layout) = &self.join_column_layout {
                    let mut offset = 0;
                    for (table, columns) in layout {
                        if *table == table_ref {
                            return match columns
                                .iter()
                                .position(|c| c.to_lowercase() == column_name)
                            {
                                Some(pos) => Ok(Arc::new(ColumnExpression {
                                    column_name,
                                    column_index: offset + pos,
                                })),
                                None => Err(PlanError::ColumnNotFound(column_name)),
                            };
                        }
                        offset += columns.len();
                    }
                    return Err(PlanError::TableNotFound(table_ref));
                }

                // Resolve the table reference
                let columns = self.tables.get(&table_ref).ok_or_else(|| {
                    PlanError::ParseError(format!("Table '{}' not found", table_ref))
                })?;
                let column_index = columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                    .ok_or_else(|| {
                        PlanError::ParseError(format!(
                            "Column '{}' not found in table '{}'",
                            column_name, table_ref
                        ))
                    })?;
                Ok(Arc::new(ColumnExpression {
                    column_name,
                    column_index,
                }))
            }
            Expr::Value(v) => {
                // Constant value
                let value = value_from_sqlparser(v)?;
                Ok(Arc::new(ConstantExpression { value }))
            }
            // MS13 T4: `DATE '...'` / `TIMESTAMP '...'` 类型字面量（sqlparser
            // TypedString）在 plan 期解析为对应值；解析失败点名报错，非日期族
            // 类型名维持既有 Unsupported 拒绝。
            Expr::TypedString { data_type, value } => {
                use sqlparser::ast::{DataType, TimezoneInfo};
                let parsed = match data_type {
                    DataType::Date => crate::executor::datetime::parse_date(value).map(Value::Date),
                    DataType::Datetime(_) | DataType::Timestamp(_, TimezoneInfo::None) => {
                        crate::executor::datetime::parse_timestamp(value).map(Value::Timestamp)
                    }
                    _ => None,
                };
                match parsed {
                    Some(v) => Ok(Arc::new(ConstantExpression { value: v })),
                    None => match data_type {
                        DataType::Date
                        | DataType::Datetime(_)
                        | DataType::Timestamp(_, TimezoneInfo::None) => Err(PlanError::ParseError(
                            format!("invalid DATE/TIMESTAMP literal: '{value}'"),
                        )),
                        _ => Err(PlanError::UnsupportedExpression),
                    },
                }
            }
            // MS13 T4: BinaryOp 算术项（WITH-FORM `SELECT id + 1` 解锁，
            // no-from-select R3 场景；编译路径 design D13）。AND/OR 与比较
            // 操作符由 build_where 处理，此处仅接受四则算术；求值语义：NULL
            // 传播、严格数值面（非数值操作符类型错误）、Int/Int 截断除。
            // MS13 T7: INTERVAL 腿在数值分流之前探测（design D11）——仅
            // `<date/timestamp> ± INTERVAL` 可达；INTERVAL 在左、双侧、
            // 乘除腿点名拒绝（左操作数的 Date/Timestamp 类型在求值期校验，
            // plan 期无法判定列类型）。
            Expr::BinaryOp { left, op, right } => {
                use sqlparser::ast::BinaryOperator as SqlOp;
                let left_interval = try_parse_interval(left)?;
                let right_interval = try_parse_interval(right)?;
                match (left_interval, right_interval) {
                    (Some(_), Some(_)) => Err(PlanError::ParseError(
                        "INTERVAL ± INTERVAL arithmetic is not supported".to_string(),
                    )),
                    (Some(_), None) => Err(PlanError::ParseError(
                        "INTERVAL is only supported on the right side of +/- (e.g. date + INTERVAL '1 day')"
                            .to_string(),
                    )),
                    (None, Some(interval)) => {
                        let arith_op = match op {
                            SqlOp::Plus => ArithOp::Add,
                            SqlOp::Minus => ArithOp::Sub,
                            other => {
                                return Err(PlanError::ParseError(format!(
                                    "INTERVAL arithmetic supports only + and -, not {other}"
                                )))
                            }
                        };
                        Ok(Arc::new(IntervalArithExpression {
                            left: self.build_expression(table_name, left)?,
                            op: arith_op,
                            interval,
                        }))
                    }
                    (None, None) => {
                        let arith_op = match op {
                            SqlOp::Plus => ArithOp::Add,
                            SqlOp::Minus => ArithOp::Sub,
                            SqlOp::Multiply => ArithOp::Mul,
                            SqlOp::Divide => ArithOp::Div,
                            _ => return Err(PlanError::UnsupportedExpression),
                        };
                        Ok(Arc::new(BinaryArithExpression {
                            left: self.build_expression(table_name, left)?,
                            op: arith_op,
                            right: self.build_expression(table_name, right)?,
                        }))
                    }
                }
            }
            // MS13 T7: INTERVAL 不可独立求值（不可存储/不可投影/不可比较）——
            // 仅在 `<date/timestamp> ± INTERVAL` 算术位置可达（BinaryOp 臂
            // 分流到 IntervalArithExpression）；到达此臂即为非法位置，点名拒绝。
            Expr::Interval(_) => Err(PlanError::ParseError(
                "INTERVAL is only supported as `<date/timestamp> +/- INTERVAL` in an arithmetic expression"
                    .to_string(),
            )),
            // MS11-T01: CAST — strict four-family target mapping; the FORMAT
            // clause and unknown types are rejected, not silently degraded.
            Expr::Cast {
                expr,
                data_type,
                format,
            } => {
                if format.is_some() {
                    return Err(PlanError::ParseError(
                        "CAST ... FORMAT clause is not supported".to_string(),
                    ));
                }
                let target = self.convert_cast_data_type(data_type)?;
                Ok(Arc::new(CastExpression {
                    expr: self.build_expression(table_name, expr)?,
                    target,
                }))
            }
            // MS11-T01: TRY_CAST is explicitly rejected (no silent fallthrough).
            Expr::TryCast { .. } => Err(PlanError::ParseError(
                "TRY_CAST is not supported".to_string(),
            )),
            // MS11-T01: CASE — searched form built directly; the simple form
            // (`CASE operand WHEN v ...`) desugars each WHEN into an Eq
            // comparison so a NULL operand yields Unknown and never matches.
            Expr::Case {
                operand,
                conditions,
                results,
                else_result,
            } => {
                let mut whens = Vec::with_capacity(conditions.len());
                for (cond, result) in conditions.iter().zip(results.iter()) {
                    let cond_expr = match operand {
                        Some(op) => Expr::BinaryOp {
                            left: op.clone(),
                            op: sqlparser::ast::BinaryOperator::Eq,
                            right: Box::new(cond.clone()),
                        },
                        None => cond.clone(),
                    };
                    let cond_pred = self.build_where(table_name, &cond_expr)?;
                    let result_expr = self.build_expression(table_name, result)?;
                    whens.push((cond_pred, result_expr));
                }
                let else_ = match else_result {
                    Some(e) => Some(self.build_expression(table_name, e)?),
                    None => None,
                };
                Ok(Arc::new(CaseExpression { whens, else_ }))
            }
            // MS11-T01: COALESCE has no dedicated sqlparser variant — it parses
            // as Expr::Function. MS11-T03: registered scalar functions build a
            // FunctionExpression after plan-time validation; unregistered names
            // keep the existing rejection below.
            Expr::Function(func) => {
                let func_name = func.name.to_string().to_uppercase();
                if func_name == "COALESCE" {
                    let mut args = Vec::with_capacity(func.args.len());
                    for arg in &func.args {
                        match arg {
                            sqlparser::ast::FunctionArg::Unnamed(
                                sqlparser::ast::FunctionArgExpr::Expr(e),
                            ) => {
                                args.push(self.build_expression(table_name, e)?);
                            }
                            _ => return Err(PlanError::UnsupportedExpression),
                        }
                    }
                    if args.is_empty() {
                        return Err(PlanError::ParseError(
                            "COALESCE requires at least one argument".to_string(),
                        ));
                    }
                    Ok(Arc::new(CoalesceExpression { args }))
                } else if is_scalar_function(&func_name) {
                    // MS11-T03: window / qualifier forms are rejected by name,
                    // never silently degraded.
                    if func.over.is_some() {
                        return Err(PlanError::ParseError(format!(
                            "OVER (window function) is not supported for scalar function '{}'",
                            func_name
                        )));
                    }
                    if func.distinct {
                        return Err(PlanError::ParseError(format!(
                            "DISTINCT is not supported for scalar function '{}'",
                            func_name
                        )));
                    }
                    if func.filter.is_some() {
                        return Err(PlanError::ParseError(format!(
                            "FILTER clause is not supported for scalar function '{}'",
                            func_name
                        )));
                    }
                    if func.null_treatment.is_some() {
                        return Err(PlanError::ParseError(format!(
                            "NULL treatment (IGNORE/RESPECT NULLS) is not supported for scalar function '{}'",
                            func_name
                        )));
                    }
                    if !func.order_by.is_empty() {
                        return Err(PlanError::ParseError(format!(
                            "ORDER BY is not supported for scalar function '{}'",
                            func_name
                        )));
                    }
                    let mut args = Vec::with_capacity(func.args.len());
                    for arg in &func.args {
                        match arg {
                            sqlparser::ast::FunctionArg::Unnamed(
                                sqlparser::ast::FunctionArgExpr::Expr(e),
                            ) => {
                                args.push(self.build_expression(table_name, e)?);
                            }
                            sqlparser::ast::FunctionArg::Unnamed(_) => {
                                return Err(PlanError::ParseError(format!(
                                    "'*' wildcard argument is not supported for scalar function '{}'",
                                    func_name
                                )));
                            }
                            sqlparser::ast::FunctionArg::Named { .. } => {
                                return Err(PlanError::ParseError(format!(
                                    "Named arguments are not supported for scalar function '{}'",
                                    func_name
                                )));
                            }
                        }
                    }
                    check_scalar_function(&func_name, args.len()).map_err(PlanError::ParseError)?;
                    Ok(Arc::new(FunctionExpression {
                        name: func_name,
                        args,
                    }))
                } else {
                    Err(PlanError::UnsupportedExpression)
                }
            }
            // MS11-T03: TRIM parses as a dedicated sqlparser variant, not
            // Expr::Function. Only the plain `trim(s)` form is supported
            // (space-only semantics, spec R2); the BOTH/LEADING/TRAILING and
            // custom-character forms are rejected by name.
            Expr::Trim {
                expr,
                trim_where,
                trim_what,
                trim_characters,
            } => {
                if trim_where.is_some()
                    || trim_what.is_some()
                    || trim_characters
                        .as_ref()
                        .is_some_and(|chars| !chars.is_empty())
                {
                    return Err(PlanError::ParseError(
                        "TRIM with a trim specification (BOTH/LEADING/TRAILING or custom characters) is not supported; only trim(s) is supported"
                            .to_string(),
                    ));
                }
                Ok(Arc::new(FunctionExpression {
                    name: "TRIM".to_string(),
                    args: vec![self.build_expression(table_name, expr)?],
                }))
            }
            // MS11-T03: CEIL/FLOOR also parse as dedicated sqlparser variants
            // (like TRIM), not Expr::Function. Only the plain `ceil(x)` /
            // `floor(x)` form is supported; the `TO DateTimeField` scale form
            // is rejected by name.
            Expr::Ceil { expr, field } => self.build_ceil_floor("CEIL", expr, field, table_name),
            Expr::Floor { expr, field } => self.build_ceil_floor("FLOOR", expr, field, table_name),
            // Handle negative numbers: -42
            Expr::UnaryOp {
                op: sqlparser::ast::UnaryOperator::Minus,
                expr: inner,
            } => {
                if let Expr::Value(v) = inner.as_ref() {
                    let value = value_from_sqlparser(v)?;
                    match value {
                        Value::Int(n) => Ok(Arc::new(ConstantExpression {
                            value: Value::Int(-n),
                        })),
                        Value::Float(f) => Ok(Arc::new(ConstantExpression {
                            value: Value::Float(-f),
                        })),
                        _ => Err(PlanError::UnsupportedValue),
                    }
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Build PredicateRef from WHERE clause expression
    pub(crate) fn build_where(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<PredicateRef, PlanError> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                // Check if this is a logical operator (AND/OR)
                use sqlparser::ast::BinaryOperator as SqlOp;
                match op {
                    SqlOp::And => {
                        let left_pred = self.build_where(table_name, left)?;
                        let right_pred = self.build_where(table_name, right)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::And,
                            right: right_pred,
                        }))
                    }
                    SqlOp::Or => {
                        let left_pred = self.build_where(table_name, left)?;
                        let right_pred = self.build_where(table_name, right)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::Or,
                            right: right_pred,
                        }))
                    }
                    _ => {
                        // MS13 T7: sqlparser 0.44 的 INTERVAL 语法把紧随的
                        // 比较操作吞入 interval.value（`d + INTERVAL '1 day' > X`
                        // 解析为 `d + INTERVAL('1 day' > X)`——value 是宽表达式）。
                        // 此处解缠绕：`<expr> ± INTERVAL{比较}` 还原为
                        // 「区间算术 <比较> 右侧表达式」谓词，恢复用户书写意图。
                        if let Some(pred) =
                            self.unswallow_interval_comparison(table_name, left, op, right)?
                        {
                            return Ok(pred);
                        }
                        // Try to convert to comparison operator
                        let comp_op = self
                            .convert_comparison_op(op)
                            .ok_or(PlanError::UnsupportedExpression)?;
                        let left_expr = self.build_expression(table_name, left)?;
                        let right_expr = self.build_expression(table_name, right)?;
                        Ok(Arc::new(ComparisonPredicate {
                            left: left_expr,
                            op: comp_op,
                            right: right_expr,
                        }))
                    }
                }
            }
            // Parenthesized expression - just unwrap
            Expr::Nested(expr) => self.build_where(table_name, expr),
            // MS11-T01: [NOT] IN (值列表) 脱糖为 Eq 比较的 OR 链；三值 NULL
            // 语义由共享比较内核产生（list 项 NULL → Unknown）。
            Expr::InList {
                expr,
                list,
                negated,
            } => {
                if list.is_empty() {
                    return Err(PlanError::ParseError(
                        "IN predicate requires a non-empty value list".to_string(),
                    ));
                }
                let left = self.build_expression(table_name, expr)?;
                let mut acc: PredicateRef = Arc::new(ComparisonPredicate {
                    left: left.clone(),
                    op: ComparisonOp::Eq,
                    right: self.build_expression(table_name, list.last().unwrap())?,
                });
                for item in list.iter().rev().skip(1) {
                    let eq = Arc::new(ComparisonPredicate {
                        left: left.clone(),
                        op: ComparisonOp::Eq,
                        right: self.build_expression(table_name, item)?,
                    });
                    acc = Arc::new(LogicalPredicate {
                        left: eq,
                        op: LogicalOp::Or,
                        right: acc,
                    });
                }
                if *negated {
                    Ok(Arc::new(NotPredicate { inner: acc }))
                } else {
                    Ok(acc)
                }
            }
            // MS11-T01: [NOT] BETWEEN low AND high → AND(Ge, Le)；NOT 经 Not 包装。
            Expr::Between {
                expr,
                negated,
                low,
                high,
            } => {
                let left = self.build_expression(table_name, expr)?;
                let ge = Arc::new(ComparisonPredicate {
                    left: left.clone(),
                    op: ComparisonOp::Ge,
                    right: self.build_expression(table_name, low)?,
                });
                let le = Arc::new(ComparisonPredicate {
                    left,
                    op: ComparisonOp::Le,
                    right: self.build_expression(table_name, high)?,
                });
                let between: PredicateRef = Arc::new(LogicalPredicate {
                    left: ge,
                    op: LogicalOp::And,
                    right: le,
                });
                if *negated {
                    Ok(Arc::new(NotPredicate { inner: between }))
                } else {
                    Ok(between)
                }
            }
            // MS11-T01: [NOT] LIKE；ESCAPE 子句显式拒绝（内核无转义语义）。
            Expr::Like {
                negated,
                expr,
                pattern,
                escape_char,
            } => {
                if escape_char.is_some() {
                    return Err(PlanError::ParseError(
                        "LIKE ESCAPE clause is not supported".to_string(),
                    ));
                }
                let like: PredicateRef = Arc::new(LikePredicate {
                    expr: self.build_expression(table_name, expr)?,
                    pattern: self.build_expression(table_name, pattern)?,
                });
                if *negated {
                    Ok(Arc::new(NotPredicate { inner: like }))
                } else {
                    Ok(like)
                }
            }
            // MS11-T01: IS [NOT] NULL
            Expr::IsNull(inner) => Ok(Arc::new(IsNullPredicate {
                expr: self.build_expression(table_name, inner)?,
            })),
            Expr::IsNotNull(inner) => {
                let is_null: PredicateRef = Arc::new(IsNullPredicate {
                    expr: self.build_expression(table_name, inner)?,
                });
                Ok(Arc::new(NotPredicate { inner: is_null }))
            }
            // MS11-T01: NOT <谓词>（否定单点实现）
            Expr::UnaryOp {
                op: sqlparser::ast::UnaryOperator::Not,
                expr: inner,
            } => Ok(Arc::new(NotPredicate {
                inner: self.build_where(table_name, inner)?,
            })),
            _ => Err(PlanError::UnsupportedExpression),
        }
    }
}

/// Extract column name from Expr (Identifier, CompoundIdentifier, or Value literal)
pub(crate) fn expr_to_column_name(expr: &Expr) -> Result<String, PlanError> {
    match expr {
        Expr::Identifier(ident) => Ok(ident.value.clone()),
        Expr::CompoundIdentifier(parts) if !parts.is_empty() => {
            Ok(parts.last().unwrap().value.clone())
        }
        Expr::Value(v) => Ok(format!("_{}", v)),
        _ => Err(PlanError::InvalidAggregateArgument(
            "Expected column name".to_string(),
        )),
    }
}

/// MS13 T7（design D11）：`Expr::Interval` → 内部区间值；非 Interval 表达式
/// 返回 `None`，畸形 INTERVAL（多字段/精度语法/未知单位/多段字符串）返回
/// 点名 `ParseError`。
fn try_parse_interval(e: &Expr) -> Result<Option<IntervalParts>, PlanError> {
    match e {
        Expr::Interval(interval) => Ok(Some(parse_interval_expr(interval)?)),
        _ => Ok(None),
    }
}

/// 解析 sqlparser `Interval`：支持 `INTERVAL 'N unit'`（字符串内嵌单位，
/// leading_field=None）与 `INTERVAL N UNIT` / `INTERVAL 'N' UNIT`
///（leading_field=Some，六主力单位）。多字段（`YEAR TO MONTH` 的
/// last_field 形态）、精度语法、六主力单位之外的字段（WEEK 等）点名拒绝。
fn parse_interval_expr(interval: &sqlparser::ast::Interval) -> Result<IntervalParts, PlanError> {
    use sqlparser::ast::{DateTimeField, Value as SqlValue};
    if interval.last_field.is_some() {
        return Err(PlanError::ParseError(
            "INTERVAL with multiple fields (e.g. YEAR TO MONTH) is not supported".to_string(),
        ));
    }
    if interval.leading_precision.is_some() || interval.fractional_seconds_precision.is_some() {
        return Err(PlanError::ParseError(
            "INTERVAL precision syntax is not supported".to_string(),
        ));
    }
    match (&*interval.value, interval.leading_field) {
        // 字符串形态：单位内嵌于字符串
        (Expr::Value(SqlValue::SingleQuotedString(s)), None) => {
            parse_interval_string(s).map_err(PlanError::ParseError)
        }
        // 数值/纯整数字符串 + 单位字段
        (Expr::Value(v), Some(field)) => {
            let unit = match field {
                DateTimeField::Year => "year",
                DateTimeField::Month => "month",
                DateTimeField::Day => "day",
                DateTimeField::Hour => "hour",
                DateTimeField::Minute => "minute",
                DateTimeField::Second => "second",
                other => {
                    return Err(PlanError::ParseError(format!(
                        "INTERVAL unit {other:?} is not supported (supported: year, month, day, hour, minute, second)"
                    )))
                }
            };
            let n: i64 = match v {
                SqlValue::Number(n, _) => n
                    .parse()
                    .map_err(|_| PlanError::ParseError(format!("invalid INTERVAL value '{n}'")))?,
                SqlValue::SingleQuotedString(s) => s.trim().parse().map_err(|_| {
                    PlanError::ParseError(format!(
                        "invalid INTERVAL value '{s}' for a typed unit (expected an integer)"
                    ))
                })?,
                other => {
                    return Err(PlanError::ParseError(format!(
                        "invalid INTERVAL value {other:?} (expected a number)"
                    )))
                }
            };
            interval_parts_from_unit(unit, n)
                .ok_or_else(|| PlanError::ParseError("INTERVAL value out of range".to_string()))
        }
        (other, _) => Err(PlanError::ParseError(format!(
            "invalid INTERVAL value {other:?} (expected `INTERVAL '<n> <unit>'` or `INTERVAL <n> <unit>`)"
        ))),
    }
}
