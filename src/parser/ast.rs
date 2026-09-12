//! AST helper functions for extracting information from sqlparser AST

use crate::parser::error::PlanError;
use sqlparser::ast::*;

/// 解析 SQL 字符串，返回 Statement 列表
pub fn parse_sql(sql: &str) -> Result<Vec<Statement>, PlanError> {
    sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, sql)
        .map_err(|e| PlanError::ParseError(e.to_string()))
}

/// 从 Query 提取 Select body
pub fn extract_select_body(query: &Query) -> Result<&Select, PlanError> {
    match query.body.as_ref() {
        SetExpr::Select(select) => Ok(select.as_ref()),
        _ => Err(PlanError::UnsupportedStatement),
    }
}

/// 从 FROM 提取表名（仅支持单表）
pub fn extract_table_name(from: &[TableWithJoins]) -> Result<String, PlanError> {
    if from.is_empty() {
        return Err(PlanError::MissingField("FROM clause".into()));
    }
    let table_factor = &from[0].relation;
    match table_factor {
        TableFactor::Table { name, .. } => Ok(name.to_string().to_lowercase()),
        _ => Err(PlanError::UnsupportedStatement),
    }
}

/// 从 projection 提取列名列表（支持 table.column 格式）
/// 对于 CompoundIdentifier，返回 "column" 格式（仅列名）
/// 对于简单 Identifier，返回 "column" 格式
/// 对于聚合函数，返回其结果列名（如 count_star, sum_score 等）
pub fn extract_columns(projection: &[SelectItem]) -> Result<Vec<String>, PlanError> {
    projection
        .iter()
        .map(|item| match item {
            SelectItem::UnnamedExpr(expr) => match expr {
                // Simple column: name
                Expr::Identifier(ident) => Ok(ident.value.to_string().to_lowercase()),
                // Qualified column: table.name -> return just the column name
                Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                    Ok(parts[1].value.to_string().to_lowercase())
                }
                // Aggregate function: return result column name
                Expr::Function(f) => {
                    let name = f.name.to_string().to_uppercase();
                    // MS11-T01 Iter001: COALESCE is a value expression
                    // (projection item), not an aggregate — the item must
                    // reach the planner's SELECT routing instead of being
                    // rejected here.
                    if name == "COALESCE" {
                        return Ok(expr.to_string());
                    }
                    // MS11-T03: registered scalar functions are value
                    // expressions too and take the same pass-through; other
                    // (unregistered) function names stay rejected here.
                    if crate::executor::is_scalar_function(&name) {
                        return Ok(expr.to_string());
                    }
                    match name.as_str() {
                        "COUNT" => {
                            if f.args.is_empty() {
                                return Ok("count_star".to_string());
                            }
                            match &f.args[0] {
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Wildcard,
                                ) => Ok("count_star".to_string()),
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Expr(inner),
                                ) => {
                                    let col = expr_to_column_name_static(inner)?;
                                    Ok(format!("count_{}", col.to_lowercase()))
                                }
                                _ => Ok("count_star".to_string()),
                            }
                        }
                        "SUM" => {
                            let col = extract_single_col_static(&f.args, "SUM")?;
                            Ok(format!("sum_{}", col.to_lowercase()))
                        }
                        "AVG" => {
                            let col = extract_single_col_static(&f.args, "AVG")?;
                            Ok(format!("avg_{}", col.to_lowercase()))
                        }
                        "MIN" => {
                            let col = extract_single_col_static(&f.args, "MIN")?;
                            Ok(format!("min_{}", col.to_lowercase()))
                        }
                        "MAX" => {
                            let col = extract_single_col_static(&f.args, "MAX")?;
                            Ok(format!("max_{}", col.to_lowercase()))
                        }
                        _ => Err(PlanError::UnsupportedStatement),
                    }
                }
                Expr::Value(v) => Ok(format!("_{}", v)),
                // MS11-T01 Iter001: 值表达式项放行至 planner 的 SELECT 路由
                //（聚合检测 / 顶层 Projection 包装），不再在此被拒。返回的
                // Display 文本对投影解析惰性——表达式查询绕过 per-node 投影。
                // MS11-T03: TRIM/CEIL/FLOOR 是独立 sqlparser 变体（非
                // Expr::Function），同门放行，由 planner 臂校验其形态。
                Expr::Case { .. }
                | Expr::Cast { .. }
                | Expr::InList { .. }
                | Expr::Between { .. }
                | Expr::Like { .. }
                | Expr::IsNull { .. }
                | Expr::IsNotNull { .. }
                | Expr::Trim { .. }
                | Expr::Ceil { .. }
                | Expr::Floor { .. } => Ok(expr.to_string()),
                _ => Err(PlanError::UnsupportedStatement),
            },
            SelectItem::ExprWithAlias { alias, .. } => Ok(alias.value.to_string().to_lowercase()),
            SelectItem::Wildcard(_) => Ok("*".into()),
            _ => Err(PlanError::UnsupportedStatement),
        })
        .collect()
}

/// 从 projection 提取完整的列信息（table.column 格式）
/// 返回 Vec<(Option<String>, String)> - (table_name, column_name)
/// 对于聚合函数，返回 (None, result_column_name)
pub fn extract_qualified_columns(
    projection: &[SelectItem],
) -> Result<Vec<(Option<String>, String)>, PlanError> {
    projection
        .iter()
        .map(|item| match item {
            SelectItem::UnnamedExpr(expr) => match expr {
                // Simple column: name
                Expr::Identifier(ident) => Ok((None, ident.value.to_string().to_lowercase())),
                // Qualified column: table.name
                Expr::CompoundIdentifier(parts) if parts.len() == 2 => Ok((
                    Some(parts[0].value.to_string().to_lowercase()),
                    parts[1].value.to_string().to_lowercase(),
                )),
                // Aggregate function: return result column name
                Expr::Function(f) => {
                    let name = f.name.to_string().to_uppercase();
                    // MS11-T01 Iter001: COALESCE 放行（值表达式投影项，非聚合）
                    if name == "COALESCE" {
                        return Ok((None, expr.to_string()));
                    }
                    // MS11-T03: 注册标量函数同门放行；未知名维持此处既有拒绝
                    if crate::executor::is_scalar_function(&name) {
                        return Ok((None, expr.to_string()));
                    }
                    match name.as_str() {
                        "COUNT" => {
                            if f.args.is_empty() {
                                return Ok((None, "count_star".to_string()));
                            }
                            match &f.args[0] {
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Wildcard,
                                ) => Ok((None, "count_star".to_string())),
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Expr(inner),
                                ) => {
                                    let col = expr_to_column_name_static(inner)?;
                                    Ok((None, format!("count_{}", col.to_lowercase())))
                                }
                                _ => Ok((None, "count_star".to_string())),
                            }
                        }
                        "SUM" => {
                            let col = extract_single_col_static(&f.args, "SUM")?;
                            Ok((None, format!("sum_{}", col.to_lowercase())))
                        }
                        "AVG" => {
                            let col = extract_single_col_static(&f.args, "AVG")?;
                            Ok((None, format!("avg_{}", col.to_lowercase())))
                        }
                        "MIN" => {
                            let col = extract_single_col_static(&f.args, "MIN")?;
                            Ok((None, format!("min_{}", col.to_lowercase())))
                        }
                        "MAX" => {
                            let col = extract_single_col_static(&f.args, "MAX")?;
                            Ok((None, format!("max_{}", col.to_lowercase())))
                        }
                        _ => Err(PlanError::UnsupportedStatement),
                    }
                }
                Expr::Value(v) => Ok((None, format!("_{}", v))),
                // MS11-T01 Iter001: 值表达式项放行至 planner 的 SELECT 路由
                //（Display 文本对 JOIN 输出过滤惰性——表达式项 + JOIN 显式拒绝）
                // MS11-T03: TRIM/CEIL/FLOOR 独立变体同门放行。
                Expr::Case { .. }
                | Expr::Cast { .. }
                | Expr::InList { .. }
                | Expr::Between { .. }
                | Expr::Like { .. }
                | Expr::IsNull { .. }
                | Expr::IsNotNull { .. }
                | Expr::Trim { .. }
                | Expr::Ceil { .. }
                | Expr::Floor { .. } => Ok((None, expr.to_string())),
                _ => Err(PlanError::UnsupportedStatement),
            },
            SelectItem::ExprWithAlias { alias, .. } => {
                Ok((None, alias.value.to_string().to_lowercase()))
            }
            SelectItem::Wildcard(_) => Ok((None, "*".into())),
            _ => Err(PlanError::UnsupportedStatement),
        })
        .collect()
}

/// 从 ObjectName 提取表名（lowercase）
pub fn extract_name_from_object(obj: &ObjectName) -> String {
    obj.to_string().to_lowercase()
}

/// 从 JOIN 关系的 TableFactor 提取表名
pub fn extract_join_table_name(relation: &TableFactor) -> Result<String, PlanError> {
    match relation {
        TableFactor::Table { name, .. } => Ok(name.to_string().to_lowercase()),
        _ => Err(PlanError::UnsupportedStatement),
    }
}

/// Extract column name from Expr (Identifier or CompoundIdentifier)
/// Used by extract_columns / extract_qualified_columns for aggregate function argument parsing.
fn expr_to_column_name_static(expr: &Expr) -> Result<String, PlanError> {
    match expr {
        Expr::Identifier(ident) => Ok(ident.value.clone()),
        Expr::CompoundIdentifier(parts) if !parts.is_empty() => {
            Ok(parts.last().unwrap().value.clone())
        }
        _ => Err(PlanError::InvalidAggregateArgument(
            "Expected column name".to_string(),
        )),
    }
}

/// Extract a single column argument from function args
fn extract_single_col_static(
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
            expr_to_column_name_static(expr)
        }
        _ => Err(PlanError::InvalidAggregateArgument(format!(
            "{} argument must be a column",
            func_name
        ))),
    }
}
