//! PlanBuilder — `build_expression` / `build_where` / `resolve_column_ref`
//! and the free function `expr_to_column_name`.
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::{
    ColumnExpression, ColumnRef, ComparisonOp, ComparisonPredicate, ConstantExpression,
    ExpressionRef, LogicalOp, LogicalPredicate, ParameterExpression, PredicateRef, Value,
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
