//! PlanBuilder — `build_having` / `build_having_expression` and the
//! aggregate helper free functions `is_aggregate_expr`, `extract_aggregate_func`,
//! `extract_single_column_arg`.
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::expression::expr_to_column_name;
use super::PlanBuilder;
use crate::executor::{
    AggregateFunc, ColumnExpression, ComparisonPredicate, ConstantExpression, ExpressionRef,
    LogicalOp, LogicalPredicate, PredicateRef, Value,
};
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::Expr;
use std::sync::Arc;

impl PlanBuilder {
    /// Build ExpressionRef from Expr for HAVING clause
    /// In HAVING context, the row is the aggregate output row with columns:
    ///   [group_col_0, ..., group_col_n, agg_result_0, ..., agg_result_m]
    /// The output_columns list gives the names in order, and the index in that
    /// list is the column index in the output row.
    pub(crate) fn build_having_expression(
        &self,
        expr: &Expr,
        output_columns: &[String],
    ) -> Result<ExpressionRef, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let ident_value = ident.value.to_uppercase();
                // Check for NULL constant
                if ident_value == "NULL" {
                    return Ok(Arc::new(ConstantExpression { value: Value::Null }));
                }
                // Column reference: look up in output_columns
                let column_name = ident.value.to_lowercase();
                let column_index = output_columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                    .ok_or_else(|| PlanError::ColumnNotFound(column_name.clone()))?;
                Ok(Arc::new(ColumnExpression {
                    column_name,
                    column_index,
                }))
            }
            Expr::Function(f) => {
                // Aggregate function reference in HAVING
                // Build the result column name and find its index in output_columns
                let name = f.name.to_string().to_uppercase();
                let result_col_name = match name.as_str() {
                    "COUNT" => {
                        if f.args.is_empty() {
                            "count_star".to_string()
                        } else {
                            match &f.args[0] {
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Wildcard,
                                ) => "count_star".to_string(),
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Expr(inner),
                                ) => {
                                    let col = expr_to_column_name(inner)?;
                                    format!("count_{}", col.to_lowercase())
                                }
                                _ => "count_star".to_string(),
                            }
                        }
                    }
                    "SUM" => {
                        let col = extract_single_column_arg(&f.args, "SUM")?;
                        format!("sum_{}", col.to_lowercase())
                    }
                    "AVG" => {
                        let col = extract_single_column_arg(&f.args, "AVG")?;
                        format!("avg_{}", col.to_lowercase())
                    }
                    "MIN" => {
                        let col = extract_single_column_arg(&f.args, "MIN")?;
                        format!("min_{}", col.to_lowercase())
                    }
                    "MAX" => {
                        let col = extract_single_column_arg(&f.args, "MAX")?;
                        format!("max_{}", col.to_lowercase())
                    }
                    _ => return Err(PlanError::UnsupportedExpression),
                };
                let column_index = output_columns
                    .iter()
                    .position(|c| c.to_lowercase() == result_col_name.to_lowercase())
                    .ok_or_else(|| {
                        PlanError::HavingNonAggregatedReference(result_col_name.clone())
                    })?;
                Ok(Arc::new(ColumnExpression {
                    column_name: result_col_name,
                    column_index,
                }))
            }
            Expr::Value(v) => {
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

    /// Build PredicateRef from HAVING clause expression
    /// Uses build_having_expression which knows about aggregate output columns
    pub(crate) fn build_having(
        &self,
        expr: &Expr,
        output_columns: &[String],
    ) -> Result<PredicateRef, PlanError> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                use sqlparser::ast::BinaryOperator as SqlOp;
                match op {
                    SqlOp::And => {
                        let left_pred = self.build_having(left, output_columns)?;
                        let right_pred = self.build_having(right, output_columns)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::And,
                            right: right_pred,
                        }))
                    }
                    SqlOp::Or => {
                        let left_pred = self.build_having(left, output_columns)?;
                        let right_pred = self.build_having(right, output_columns)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::Or,
                            right: right_pred,
                        }))
                    }
                    _ => {
                        let comp_op = self
                            .convert_comparison_op(op)
                            .ok_or(PlanError::UnsupportedExpression)?;
                        let left_expr = self.build_having_expression(left, output_columns)?;
                        let right_expr = self.build_having_expression(right, output_columns)?;
                        Ok(Arc::new(ComparisonPredicate {
                            left: left_expr,
                            op: comp_op,
                            right: right_expr,
                        }))
                    }
                }
            }
            Expr::Nested(expr) => self.build_having(expr, output_columns),
            _ => Err(PlanError::UnsupportedExpression),
        }
    }
}

/// Check if an Expr is an aggregate function
pub(crate) fn is_aggregate_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Function(f) if {
        let name = f.name.to_string().to_uppercase();
        matches!(name.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
    })
}

/// Extract AggregateFunc from an Expr, returns None if not an aggregate
pub(crate) fn extract_aggregate_func(expr: &Expr) -> Result<Option<AggregateFunc>, PlanError> {
    match expr {
        Expr::Function(f) => {
            let name = f.name.to_string().to_uppercase();
            match name.as_str() {
                "COUNT" => {
                    if f.args.is_empty() {
                        return Err(PlanError::InvalidAggregateArgument(
                            "COUNT requires argument or *".to_string(),
                        ));
                    }
                    match &f.args[0] {
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Wildcard,
                        ) => Ok(Some(AggregateFunc::CountStar)),
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(inner),
                        ) => {
                            let col = expr_to_column_name(inner)?;
                            Ok(Some(AggregateFunc::Count(col)))
                        }
                        _ => Err(PlanError::InvalidAggregateArgument(
                            "COUNT argument must be * or column".to_string(),
                        )),
                    }
                }
                "SUM" => {
                    let col = extract_single_column_arg(&f.args, "SUM")?;
                    Ok(Some(AggregateFunc::Sum(col)))
                }
                "AVG" => {
                    let col = extract_single_column_arg(&f.args, "AVG")?;
                    Ok(Some(AggregateFunc::Avg(col)))
                }
                "MIN" => {
                    let col = extract_single_column_arg(&f.args, "MIN")?;
                    Ok(Some(AggregateFunc::Min(col)))
                }
                "MAX" => {
                    let col = extract_single_column_arg(&f.args, "MAX")?;
                    Ok(Some(AggregateFunc::Max(col)))
                }
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

pub(crate) fn extract_single_column_arg(
    args: &[sqlparser::ast::FunctionArg],
    func_name: &str,
) -> Result<String, PlanError> {
    if args.len() != 1 {
        return Err(PlanError::InvalidAggregateArgument(format!(
            "{} requires exactly one argument",
            func_name
        )));
    }
    match &args[0] {
        sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(expr)) => {
            expr_to_column_name(expr)
        }
        _ => Err(PlanError::InvalidAggregateArgument(format!(
            "{} argument must be a column",
            func_name
        ))),
    }
}
