//! PlanBuilder — `build_expression` / `build_where` / `resolve_column_ref`
//! and the free function `expr_to_column_name`.
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::{
    CaseExpression, CastExpression, CastType, CoalesceExpression, ColumnExpression, ColumnRef,
    ComparisonOp, ComparisonPredicate, ConstantExpression, ExpressionRef, IsNullPredicate,
    LikePredicate, LogicalOp, LogicalPredicate, NotPredicate, ParameterExpression, PredicateRef,
    Value,
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
        use sqlparser::ast::DataType;
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

            other => {
                return Err(PlanError::ParseError(format!(
                    "Unsupported CAST target type: {}",
                    other
                )))
            }
        })
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
            // as Expr::Function. Other function names stay unsupported
            // (scalar function library is MS11-T03 surface).
            Expr::Function(func) => {
                if func.name.to_string().to_uppercase() == "COALESCE" {
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
                } else {
                    Err(PlanError::UnsupportedExpression)
                }
            }
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
