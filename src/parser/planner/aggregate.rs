//! PlanBuilder — `build_having` / `build_having_expression` and the
//! aggregate helper free functions `is_aggregate_expr`, `extract_aggregate_func`,
//! `extract_single_column_arg`.
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::expression::expr_to_column_name;
use super::query::aggregate_arg_expr;
use super::PlanBuilder;
use crate::executor::{
    check_scalar_function, is_scalar_function, AggregateFunc, ArithOp, BinaryArithExpression,
    CaseExpression, CastExpression, CoalesceExpression, ColumnExpression, ComparisonPredicate,
    ConstantExpression, ExpressionRef, FunctionExpression, LogicalOp, LogicalPredicate,
    PredicateRef, Value,
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
    ///
    /// change 2026-10-03-practical-core-gaps T5：HAVING 完整表达式编译——
    /// 既有四臂（Identifier / 聚合 Function / Value / 负数字面量）的语义与
    /// 拒绝通道（`ColumnNotFound` / `HavingNonAggregatedReference` /
    /// `UnsupportedValue`）由 `compile_aggregate_row_expr` 的 `Having` 模式
    /// 逐字节承载，`_` 兜底（原 `UnsupportedExpression("EXPRESSION")`）替换
    /// 为完整表达式树臂：算术 / AND-OR 腿 / Nested / CAST / CASE / COALESCE /
    /// TRIM / CEIL / FLOOR / 标量函数 / TypedString 常量。
    pub(crate) fn build_having_expression(
        &self,
        expr: &Expr,
        output_columns: &[String],
        canonical: &[String],
    ) -> Result<ExpressionRef, PlanError> {
        self.compile_aggregate_row_expr(expr, output_columns, canonical, AggRowExprMode::Having)
    }

    /// Build PredicateRef from HAVING clause expression
    /// Uses build_having_expression which knows about aggregate output columns
    pub(crate) fn build_having(
        &self,
        expr: &Expr,
        output_columns: &[String],
        canonical: &[String],
    ) -> Result<PredicateRef, PlanError> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                use sqlparser::ast::BinaryOperator as SqlOp;
                match op {
                    SqlOp::And => {
                        let left_pred = self.build_having(left, output_columns, canonical)?;
                        let right_pred = self.build_having(right, output_columns, canonical)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::And,
                            right: right_pred,
                        }))
                    }
                    SqlOp::Or => {
                        let left_pred = self.build_having(left, output_columns, canonical)?;
                        let right_pred = self.build_having(right, output_columns, canonical)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::Or,
                            right: right_pred,
                        }))
                    }
                    _ => {
                        let comp_op = self.convert_comparison_op(op).ok_or_else(|| {
                            PlanError::UnsupportedExpression(format!("COMPARISON OPERATOR {op}"))
                        })?;
                        let left_expr =
                            self.build_having_expression(left, output_columns, canonical)?;
                        let right_expr =
                            self.build_having_expression(right, output_columns, canonical)?;
                        Ok(Arc::new(ComparisonPredicate {
                            left: left_expr,
                            op: comp_op,
                            right: right_expr,
                        }))
                    }
                }
            }
            Expr::Nested(expr) => self.build_having(expr, output_columns, canonical),
            _ => Err(PlanError::UnsupportedExpression("HAVING PREDICATE".into())),
        }
    }

    /// 聚合输出行上的表达式编译器（change 2026-10-03-practical-core-gaps
    /// T4/T5/T6 共享基座）。
    ///
    /// 行形状 = `[group_cols..., agg_results...]`，`output_columns` 按同序给出
    /// 列名，`canonical` 与 `output_columns` 的聚合段平行给出各注册项的规范名
    /// （`AggregateFunc::result_column_name`，func 身份）。聚合 Function 引用
    /// 解析为输出行上的 `ColumnExpression`（输出列引用，不重求值）；算术 /
    /// COALESCE / 标量函数 / CAST / CASE / TRIM / CEIL / FLOOR / 常量沿
    /// `expression.rs` 既有构造语义递归编译；分支条件经 `build_having` 在聚合
    /// 输出行上编译为谓词。
    ///
    /// T12：聚合引用按规范名解析（显示名可能被别名遮蔽——重名形态下名字 →
    /// 输出列不再是单射，规范名才是调用身份）；未注册引用的既有拒绝通道与
    /// 文案不变。
    ///
    /// 模式差异仅在既有拒绝通道的保持：
    /// - `Having`：标识符缺失 `ColumnNotFound`、聚合引用缺失
    ///   `HavingNonAggregatedReference`（既有 build_having_expression 通道）。
    /// - `OutputRow`（SELECT 表达式项 / ORDER BY 表达式键）：未解析标识符或
    ///   聚合引用 → `NonAggregatedColumn`（无 GROUP BY 非聚合列引用拒绝面）。
    pub(crate) fn compile_aggregate_row_expr(
        &self,
        expr: &Expr,
        output_columns: &[String],
        canonical: &[String],
        mode: AggRowExprMode,
    ) -> Result<ExpressionRef, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                if ident.value.to_uppercase() == "NULL" {
                    return Ok(Arc::new(ConstantExpression { value: Value::Null }));
                }
                let column_name = ident.value.to_lowercase();
                match output_columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                {
                    Some(column_index) => Ok(Arc::new(ColumnExpression {
                        column_name,
                        column_index,
                    })),
                    None => Err(match mode {
                        AggRowExprMode::Having => PlanError::ColumnNotFound(column_name),
                        AggRowExprMode::OutputRow => PlanError::NonAggregatedColumn(column_name),
                    }),
                }
            }
            Expr::Function(f) => {
                if is_aggregate_expr(expr) {
                    // 聚合引用 = 聚合输出行上的列引用（MS25 D7 命名臂原样保持）。
                    // T12：按注册项规范名（func 身份）解析——显示名可能被别名
                    // 遮蔽（重名形态下按显示名首匹配静默取错值，S24）；聚合段
                    // 在 output_columns 尾部、与 canonical 平行。未注册引用的
                    // 既有拒绝通道与文案逐字节不变。
                    let result_col_name = aggregate_ref_result_column(f)?;
                    match canonical
                        .iter()
                        .position(|c| c.to_lowercase() == result_col_name.to_lowercase())
                    {
                        Some(agg_pos) => Ok(Arc::new(ColumnExpression {
                            column_name: result_col_name,
                            column_index: output_columns.len() - canonical.len() + agg_pos,
                        })),
                        None => Err(match mode {
                            AggRowExprMode::Having => {
                                PlanError::HavingNonAggregatedReference(result_col_name)
                            }
                            AggRowExprMode::OutputRow => {
                                PlanError::NonAggregatedColumn(result_col_name)
                            }
                        }),
                    }
                } else {
                    let func_name = f.name.to_string().to_uppercase();
                    if func_name == "COALESCE" {
                        let mut args = Vec::with_capacity(f.args.len());
                        for arg in &f.args {
                            match arg {
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Expr(e),
                                ) => {
                                    args.push(self.compile_aggregate_row_expr(
                                        e,
                                        output_columns,
                                        canonical,
                                        mode,
                                    )?);
                                }
                                _ => {
                                    return Err(PlanError::UnsupportedExpression(
                                        "COALESCE ARGUMENT".into(),
                                    ))
                                }
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
                        if f.over.is_some() {
                            return Err(PlanError::ParseError(format!(
                                "OVER (window function) is not supported for scalar function '{}'",
                                func_name
                            )));
                        }
                        if f.distinct {
                            return Err(PlanError::ParseError(format!(
                                "DISTINCT is not supported for scalar function '{}'",
                                func_name
                            )));
                        }
                        if f.filter.is_some() {
                            return Err(PlanError::ParseError(format!(
                                "FILTER clause is not supported for scalar function '{}'",
                                func_name
                            )));
                        }
                        if f.null_treatment.is_some() {
                            return Err(PlanError::ParseError(format!(
                                "NULL treatment (IGNORE/RESPECT NULLS) is not supported for scalar function '{}'",
                                func_name
                            )));
                        }
                        if !f.order_by.is_empty() {
                            return Err(PlanError::ParseError(format!(
                                "ORDER BY is not supported for scalar function '{}'",
                                func_name
                            )));
                        }
                        let mut args = Vec::with_capacity(f.args.len());
                        for arg in &f.args {
                            match arg {
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Expr(e),
                                ) => {
                                    args.push(self.compile_aggregate_row_expr(
                                        e,
                                        output_columns,
                                        canonical,
                                        mode,
                                    )?);
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
                        check_scalar_function(&func_name, args.len())
                            .map_err(PlanError::ParseError)?;
                        Ok(Arc::new(FunctionExpression {
                            name: func_name,
                            args,
                        }))
                    } else {
                        Err(PlanError::UnsupportedExpression(func_name))
                    }
                }
            }
            Expr::Value(v) => {
                let value = value_from_sqlparser(v)?;
                Ok(Arc::new(ConstantExpression { value }))
            }
            // MS13 T4: `DATE '...'` / `TIMESTAMP '...'` 类型字面量与
            // build_expression 同语义（plan 期解析，解析失败点名）。
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
                        _ => Err(PlanError::UnsupportedExpression(
                            data_type.to_string().to_uppercase(),
                        )),
                    },
                }
            }
            // Handle negative numbers: -42（与 build_expression 同语义）
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
            // MS13 T4: 算术项（AND/OR 与比较操作符属谓词层：HAVING 腿由
            // build_having 拦截，其余位置沿 build_expression 的既有点名通道）。
            Expr::BinaryOp { left, op, right } => {
                use sqlparser::ast::BinaryOperator as SqlOp;
                let arith_op = match op {
                    SqlOp::Plus => ArithOp::Add,
                    SqlOp::Minus => ArithOp::Sub,
                    SqlOp::Multiply => ArithOp::Mul,
                    SqlOp::Divide => ArithOp::Div,
                    _ => {
                        return Err(PlanError::UnsupportedExpression(format!(
                            "ARITHMETIC OPERATOR {op}"
                        )))
                    }
                };
                Ok(Arc::new(BinaryArithExpression {
                    left: self.compile_aggregate_row_expr(left, output_columns, canonical, mode)?,
                    op: arith_op,
                    right: self.compile_aggregate_row_expr(
                        right,
                        output_columns,
                        canonical,
                        mode,
                    )?,
                }))
            }
            Expr::Nested(inner) => {
                self.compile_aggregate_row_expr(inner, output_columns, canonical, mode)
            }
            // MS11-T01: CAST — strict target mapping; the FORMAT clause and
            // unknown types are rejected, not silently degraded.
            Expr::Cast {
                expr: inner,
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
                    expr: self.compile_aggregate_row_expr(
                        inner,
                        output_columns,
                        canonical,
                        mode,
                    )?,
                    target,
                }))
            }
            Expr::TryCast { .. } => Err(PlanError::ParseError(
                "TRY_CAST is not supported".to_string(),
            )),
            // MS11-T01: CASE — searched form built directly; the simple form
            // desugars each WHEN into an Eq comparison. 分支条件是聚合输出行
            // 上的谓词，经 build_having 编译。
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
                    let cond_pred = self.build_having(&cond_expr, output_columns, canonical)?;
                    let result_expr =
                        self.compile_aggregate_row_expr(result, output_columns, canonical, mode)?;
                    whens.push((cond_pred, result_expr));
                }
                let else_ = match else_result {
                    Some(e) => {
                        Some(self.compile_aggregate_row_expr(e, output_columns, canonical, mode)?)
                    }
                    None => None,
                };
                Ok(Arc::new(CaseExpression { whens, else_ }))
            }
            // MS11-T03: TRIM parses as a dedicated sqlparser variant. Only the
            // plain `trim(s)` form is supported.
            Expr::Trim {
                expr: inner,
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
                    args: vec![self.compile_aggregate_row_expr(
                        inner,
                        output_columns,
                        canonical,
                        mode,
                    )?],
                }))
            }
            // MS11-T03: CEIL/FLOOR dedicated variants — plain form only.
            Expr::Ceil { expr: inner, field } => {
                self.compile_ceil_floor("CEIL", inner, field, output_columns, canonical, mode)
            }
            Expr::Floor { expr: inner, field } => {
                self.compile_ceil_floor("FLOOR", inner, field, output_columns, canonical, mode)
            }
            _ => Err(PlanError::UnsupportedExpression("EXPRESSION".into())),
        }
    }

    /// MS11-T03: CEIL/FLOOR 纯形态共享体（`TO DateTimeField` 规格化形态点名拒绝）。
    fn compile_ceil_floor(
        &self,
        name: &str,
        expr: &Expr,
        field: &sqlparser::ast::DateTimeField,
        output_columns: &[String],
        canonical: &[String],
        mode: AggRowExprMode,
    ) -> Result<ExpressionRef, PlanError> {
        if !matches!(field, sqlparser::ast::DateTimeField::NoDateTime) {
            return Err(PlanError::ParseError(format!(
                "{name} with a TO date-time field is not supported; only {name}(x) is supported"
            )));
        }
        Ok(Arc::new(FunctionExpression {
            name: name.to_string(),
            args: vec![self.compile_aggregate_row_expr(expr, output_columns, canonical, mode)?],
        }))
    }
}

/// Check if an Expr is an aggregate function
pub(crate) fn is_aggregate_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Function(f) if {
        let name = f.name.to_string().to_uppercase();
        matches!(name.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "GROUP_CONCAT")
    })
}

/// change 2026-10-03-practical-core-gaps T4：聚合输出行表达式的编译模式
/// （拒绝通道随消费面不同，见 `PlanBuilder::compile_aggregate_row_expr`）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AggRowExprMode {
    /// HAVING 腿：既有通道（`ColumnNotFound` / `HavingNonAggregatedReference`）。
    Having,
    /// SELECT 表达式项 / ORDER BY 表达式键：未解析引用 → `NonAggregatedColumn`
    /// （无 GROUP BY 非聚合列引用拒绝面）。
    OutputRow,
}

/// change 2026-10-03-practical-core-gaps T4：聚合调用在聚合输出行上的结果
/// 列名（既有 build_having_expression 命名逻辑原样提取，HAVING 与编译器共享）。
/// MS25 D7：新聚合形态的输出名先行解析（DISTINCT 感知 + GROUP_CONCAT）；
/// 非 DISTINCT 的既有五聚合命名逐字节不变。
fn aggregate_ref_result_column(f: &sqlparser::ast::Function) -> Result<String, PlanError> {
    let name = f.name.to_string().to_uppercase();
    let ms25_name = match name.as_str() {
        "COUNT" | "SUM" | "AVG" if has_distinct_quantifier(f) => {
            let col = match f.args.first() {
                Some(sqlparser::ast::FunctionArg::Unnamed(
                    sqlparser::ast::FunctionArgExpr::Expr(inner),
                )) => expr_to_column_name(inner)?,
                _ => {
                    return Err(PlanError::InvalidAggregateArgument(
                        "COUNT/SUM/AVG(DISTINCT ...) requires a column".to_string(),
                    ))
                }
            };
            Some(format!(
                "{}_distinct_{}",
                name.to_lowercase(),
                col.to_lowercase()
            ))
        }
        "GROUP_CONCAT" => {
            let (col, _) = extract_group_concat_args(&f.args)?;
            Some(format!("group_concat_{}", col.to_lowercase()))
        }
        _ => None,
    };
    match ms25_name {
        Some(name) => Ok(name),
        None => match name.as_str() {
            "COUNT" => {
                if f.args.is_empty() {
                    Ok("count_star".to_string())
                } else {
                    match &f.args[0] {
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Wildcard,
                        ) => Ok("count_star".to_string()),
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(inner),
                        ) => {
                            let col = expr_to_column_name(inner)?;
                            Ok(format!("count_{}", col.to_lowercase()))
                        }
                        _ => Ok("count_star".to_string()),
                    }
                }
            }
            "SUM" => {
                let col = extract_single_column_arg(&f.args, "SUM")?;
                Ok(format!("sum_{}", col.to_lowercase()))
            }
            "AVG" => {
                let col = extract_single_column_arg(&f.args, "AVG")?;
                Ok(format!("avg_{}", col.to_lowercase()))
            }
            "MIN" => {
                let col = extract_single_column_arg(&f.args, "MIN")?;
                Ok(format!("min_{}", col.to_lowercase()))
            }
            "MAX" => {
                let col = extract_single_column_arg(&f.args, "MAX")?;
                Ok(format!("max_{}", col.to_lowercase()))
            }
            _ => Err(PlanError::UnsupportedExpression(name)),
        },
    }
}

/// change 2026-10-03-practical-core-gaps T4：表达式树内是否嵌套聚合调用。
/// 自顶向下递归；聚合调用节点即真（不进入其参数——嵌套聚合由聚合参数校验
/// 既有通道点名拒绝）；子查询是独立作用域，不下探。
pub(crate) fn contains_aggregate_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Function(f) => {
            if is_aggregate_expr(expr) {
                return true;
            }
            f.args.iter().any(|arg| {
                matches!(
                    arg,
                    sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(e))
                        if contains_aggregate_expr(e)
                )
            })
        }
        Expr::BinaryOp { left, right, .. } => {
            contains_aggregate_expr(left) || contains_aggregate_expr(right)
        }
        Expr::UnaryOp { expr: inner, .. } => contains_aggregate_expr(inner),
        Expr::Nested(inner) => contains_aggregate_expr(inner),
        Expr::Cast { expr: inner, .. } => contains_aggregate_expr(inner),
        Expr::Case {
            operand,
            conditions,
            results,
            else_result,
        } => {
            operand.as_deref().is_some_and(contains_aggregate_expr)
                || conditions.iter().any(contains_aggregate_expr)
                || results.iter().any(contains_aggregate_expr)
                || else_result.as_deref().is_some_and(contains_aggregate_expr)
        }
        Expr::Trim {
            expr: inner,
            trim_what,
            ..
        } => {
            contains_aggregate_expr(inner)
                || trim_what.as_deref().is_some_and(contains_aggregate_expr)
        }
        Expr::Ceil { expr: inner, .. } | Expr::Floor { expr: inner, .. } => {
            contains_aggregate_expr(inner)
        }
        _ => false,
    }
}

/// change 2026-10-03-practical-core-gaps T4：提取-注册遍历——自顶向下遇聚合
/// 调用即经 `extract_aggregate_func` 规范化注册进共享清单（不进入聚合参数），
/// 按结果列名在 `registered_names` 去重（同一调用共享一个输出列，不重复注册）。
/// `registered_names` 与 `aggregates` 同序平行（聚合输出行显示名）。
pub(crate) fn register_aggregate_calls(
    expr: &Expr,
    aggregates: &mut Vec<AggregateFunc>,
    aggregate_args: &mut Vec<Option<Expr>>,
    registered_names: &mut Vec<String>,
) -> Result<(), PlanError> {
    match expr {
        Expr::Function(f) => {
            if is_aggregate_expr(expr) {
                let func = extract_aggregate_func(expr)?.ok_or_else(|| {
                    PlanError::InvalidAggregateArgument("Unknown aggregate function".to_string())
                })?;
                let result = func.result_column_name();
                // T12：去重键 = 注册项规范名（func 身份）。显示名可能被别名
                // 遮蔽（`MAX(a) AS max_b` 的显示名撞 `MAX(b)` 的规范名），按
                // 显示名去重会把未注册的不同调用误判为已注册——该调用不进
                // 共享清单、执行器不计算，引用随后静默落到别的输出列（S24
                // 形态④取错值的根源之一）。
                if !aggregates
                    .iter()
                    .any(|f| f.result_column_name().eq_ignore_ascii_case(&result))
                {
                    registered_names.push(result);
                    aggregate_args.push(aggregate_arg_expr(expr).cloned());
                    aggregates.push(func);
                }
                return Ok(());
            }
            for arg in &f.args {
                if let sqlparser::ast::FunctionArg::Unnamed(
                    sqlparser::ast::FunctionArgExpr::Expr(e),
                ) = arg
                {
                    register_aggregate_calls(e, aggregates, aggregate_args, registered_names)?;
                }
            }
            Ok(())
        }
        Expr::BinaryOp { left, right, .. } => {
            register_aggregate_calls(left, aggregates, aggregate_args, registered_names)?;
            register_aggregate_calls(right, aggregates, aggregate_args, registered_names)
        }
        Expr::UnaryOp { expr: inner, .. } => {
            register_aggregate_calls(inner, aggregates, aggregate_args, registered_names)
        }
        Expr::Nested(inner) => {
            register_aggregate_calls(inner, aggregates, aggregate_args, registered_names)
        }
        Expr::Cast { expr: inner, .. } => {
            register_aggregate_calls(inner, aggregates, aggregate_args, registered_names)
        }
        Expr::Case {
            operand,
            conditions,
            results,
            else_result,
        } => {
            if let Some(op) = operand {
                register_aggregate_calls(op, aggregates, aggregate_args, registered_names)?;
            }
            for e in conditions.iter().chain(results.iter()) {
                register_aggregate_calls(e, aggregates, aggregate_args, registered_names)?;
            }
            if let Some(e) = else_result {
                register_aggregate_calls(e, aggregates, aggregate_args, registered_names)?;
            }
            Ok(())
        }
        Expr::Trim {
            expr: inner,
            trim_what,
            ..
        } => {
            register_aggregate_calls(inner, aggregates, aggregate_args, registered_names)?;
            if let Some(e) = trim_what {
                register_aggregate_calls(e, aggregates, aggregate_args, registered_names)?;
            }
            Ok(())
        }
        Expr::Ceil { expr: inner, .. } | Expr::Floor { expr: inner, .. } => {
            register_aggregate_calls(inner, aggregates, aggregate_args, registered_names)
        }
        _ => Ok(()),
    }
}

/// MS25 D7: 是否带 `DISTINCT` 量化词（sqlparser 0.44 `Function::distinct` 为 bool）。
fn has_distinct_quantifier(f: &sqlparser::ast::Function) -> bool {
    f.distinct
}

/// MS25 D7: `GROUP_CONCAT(<col> [, '<sep>'])` 的参数解析。第一参数为列引用；
/// 第二参数必须是字面量字符串（分隔符不做表达式求值）。参数数量越界或
/// 分隔符非字面量 → `InvalidAggregateArgument` 点名。
fn extract_group_concat_args(
    args: &[sqlparser::ast::FunctionArg],
) -> Result<(String, Option<String>), PlanError> {
    if args.is_empty() || args.len() > 2 {
        return Err(PlanError::InvalidAggregateArgument(
            "GROUP_CONCAT requires a column and an optional literal separator".to_string(),
        ));
    }
    let col = match &args[0] {
        sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(expr)) => {
            expr_to_column_name(expr)?
        }
        _ => {
            return Err(PlanError::InvalidAggregateArgument(
                "GROUP_CONCAT argument must be a column".to_string(),
            ))
        }
    };
    let separator = match args.get(1) {
        None => None,
        Some(sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
            sqlparser::ast::Expr::Value(value),
        ))) => Some(match value_from_sqlparser(value)? {
            Value::String(sep) => sep,
            _ => {
                return Err(PlanError::InvalidAggregateArgument(
                    "GROUP_CONCAT separator must be a string literal".to_string(),
                ))
            }
        }),
        Some(_) => {
            return Err(PlanError::InvalidAggregateArgument(
                "GROUP_CONCAT separator must be a string literal".to_string(),
            ))
        }
    };
    Ok((col, separator))
}

/// Extract AggregateFunc from an Expr, returns None if not an aggregate
pub(crate) fn extract_aggregate_func(expr: &Expr) -> Result<Option<AggregateFunc>, PlanError> {
    match expr {
        Expr::Function(f) => {
            let name = f.name.to_string().to_uppercase();
            // MS25 D7: `GROUP_CONCAT(DISTINCT …)` 不在支持面（重复值不做去重
            // 折叠），落入既有的「未知名聚合」点名拒绝通道。
            if name == "GROUP_CONCAT" && has_distinct_quantifier(f) {
                return Ok(None);
            }
            match name.as_str() {
                "COUNT" => {
                    if f.args.is_empty() {
                        return Err(PlanError::InvalidAggregateArgument(
                            "COUNT requires argument or *".to_string(),
                        ));
                    }
                    // MS25 D7: `COUNT(DISTINCT col)` 真实去重计数；`*` 与多参数
                    // 形态在 DISTINCT 下点名拒绝（去重对象必须是单个列）。
                    let distinct = has_distinct_quantifier(f);
                    if distinct && f.args.len() > 1 {
                        return Err(PlanError::InvalidAggregateArgument(
                            "COUNT(DISTINCT ...) requires exactly one column".to_string(),
                        ));
                    }
                    match &f.args[0] {
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Wildcard,
                        ) => {
                            if distinct {
                                return Err(PlanError::InvalidAggregateArgument(
                                    "COUNT(DISTINCT ...) requires a column, not *".to_string(),
                                ));
                            }
                            Ok(Some(AggregateFunc::CountStar))
                        }
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(inner),
                        ) => {
                            let col = expr_to_column_name(inner)?;
                            Ok(Some(if distinct {
                                AggregateFunc::CountDistinct(col)
                            } else {
                                AggregateFunc::Count(col)
                            }))
                        }
                        _ => Err(PlanError::InvalidAggregateArgument(
                            "COUNT argument must be * or column".to_string(),
                        )),
                    }
                }
                "SUM" => {
                    let col = extract_single_column_arg(&f.args, "SUM")?;
                    Ok(Some(if has_distinct_quantifier(f) {
                        AggregateFunc::SumDistinct(col)
                    } else {
                        AggregateFunc::Sum(col)
                    }))
                }
                "AVG" => {
                    let col = extract_single_column_arg(&f.args, "AVG")?;
                    Ok(Some(if has_distinct_quantifier(f) {
                        AggregateFunc::AvgDistinct(col)
                    } else {
                        AggregateFunc::Avg(col)
                    }))
                }
                // MS25 D7: `MIN/MAX(DISTINCT x)` 等价普通形态（SQLite 对齐），
                // DISTINCT 量化词被丢弃。
                "MIN" => {
                    let col = extract_single_column_arg(&f.args, "MIN")?;
                    Ok(Some(AggregateFunc::Min(col)))
                }
                "MAX" => {
                    let col = extract_single_column_arg(&f.args, "MAX")?;
                    Ok(Some(AggregateFunc::Max(col)))
                }
                "GROUP_CONCAT" => {
                    let (col, separator) = extract_group_concat_args(&f.args)?;
                    Ok(Some(AggregateFunc::GroupConcat { col, separator }))
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
