//! AST helper functions for extracting information from sqlparser AST

use crate::parser::error::PlanError;
use sqlparser::ast::*;

/// 解析 SQL 字符串，返回 Statement 列表
pub fn parse_sql(sql: &str) -> Result<Vec<Statement>, PlanError> {
    sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, sql)
        .map_err(|e| PlanError::ParseError(e.to_string()))
}

/// 顶层语句 catch-all 的能力名（MS19-T02）：取 sqlparser Display 的首词
/// （关键字以大写开头，如 `MERGE INTO ...` → `MERGE`）；空文本回落
/// `"STATEMENT"`。
pub fn unsupported_statement_name(stmt: &Statement) -> String {
    stmt.to_string()
        .split_whitespace()
        .next()
        .filter(|word| !word.is_empty())
        .unwrap_or("STATEMENT")
        .to_string()
}

/// 从 Query 提取 Select body
pub fn extract_select_body(query: &Query) -> Result<&Select, PlanError> {
    match query.body.as_ref() {
        SetExpr::Select(select) => Ok(select.as_ref()),
        _ => Err(PlanError::UnsupportedStatement("SET OPERATION".into())),
    }
}

/// 从 FROM 提取表名（仅支持单表）
pub fn extract_table_name(from: &[TableWithJoins]) -> Result<String, PlanError> {
    if from.is_empty() {
        return Err(PlanError::MissingField("FROM clause".into()));
    }
    let table_factor = &from[0].relation;
    match table_factor {
        TableFactor::Table { name, .. } => Ok(object_name_to_table_name(name)),
        _ => Err(PlanError::UnsupportedStatement("DERIVED TABLE".into())),
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
                // MS20 T2: alias.table.name 三段列引用——输出列名同两段形态
                // （只取末段），表名侧由 `extract_qualified_columns` 承载。
                Expr::CompoundIdentifier(parts) if parts.len() == 3 => {
                    Ok(parts[2].value.to_string().to_lowercase())
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
                    // MS25 D7: `DISTINCT` 量化词进入输出列名（`count_distinct_v`
                    // 等），与执行器 `AggregateFunc::result_column_name` 同源。
                    let distinct = f.distinct;
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
                                    Ok(if distinct {
                                        format!("count_distinct_{}", col.to_lowercase())
                                    } else {
                                        format!("count_{}", col.to_lowercase())
                                    })
                                }
                                _ => Ok("count_star".to_string()),
                            }
                        }
                        "SUM" => {
                            let col = extract_single_col_static(&f.args, "SUM")?;
                            Ok(if distinct {
                                format!("sum_distinct_{}", col.to_lowercase())
                            } else {
                                format!("sum_{}", col.to_lowercase())
                            })
                        }
                        "AVG" => {
                            let col = extract_single_col_static(&f.args, "AVG")?;
                            Ok(if distinct {
                                format!("avg_distinct_{}", col.to_lowercase())
                            } else {
                                format!("avg_{}", col.to_lowercase())
                            })
                        }
                        "MIN" => {
                            let col = extract_single_col_static(&f.args, "MIN")?;
                            Ok(format!("min_{}", col.to_lowercase()))
                        }
                        "MAX" => {
                            let col = extract_single_col_static(&f.args, "MAX")?;
                            Ok(format!("max_{}", col.to_lowercase()))
                        }
                        // MS25 D7: GROUP_CONCAT 输出名只取首参列（分隔符不入名）
                        "GROUP_CONCAT" => {
                            let col = extract_first_col_static(&f.args, "GROUP_CONCAT")?;
                            Ok(format!("group_concat_{}", col.to_lowercase()))
                        }
                        _ => Err(PlanError::UnsupportedStatement(name)),
                    }
                }
                Expr::Value(v) => Ok(format!("_{}", v)),
                // MS11-T01 Iter001: 值表达式项放行至 planner 的 SELECT 路由
                //（聚合检测 / 顶层 Projection 包装），不再在此被拒。返回的
                // Display 文本对投影解析惰性——表达式查询绕过 per-node 投影。
                // MS11-T03: TRIM/CEIL/FLOOR 是独立 sqlparser 变体（非
                // Expr::Function），同门放行，由 planner 臂校验其形态。
                // MS13 T4: TypedString（类型字面量）与 BinaryOp（算术项，
                // WITH-FORM `SELECT id + 1` 解锁）同门放行；JOIN 形态的
                // qualified 提取仍拒绝（既有表达式项 + JOIN 拒绝面）。
                Expr::Case { .. }
                | Expr::Cast { .. }
                | Expr::InList { .. }
                | Expr::Between { .. }
                | Expr::Like { .. }
                | Expr::IsNull { .. }
                | Expr::IsNotNull { .. }
                | Expr::Trim { .. }
                | Expr::Ceil { .. }
                | Expr::Floor { .. }
                | Expr::TypedString { .. }
                | Expr::BinaryOp { .. }
                // MS13 T7: INTERVAL 腿随算术项放行至 planner（独立/畸形
                // 形态由 planner 点名拒绝，非此处既有兜底文案）。
                | Expr::Interval { .. } => Ok(expr.to_string()),
                _ => Err(PlanError::UnsupportedStatement(
                    "UNSUPPORTED SELECT ITEM EXPRESSION".into(),
                )),
            },
            SelectItem::ExprWithAlias { alias, .. } => Ok(alias.value.to_string().to_lowercase()),
            SelectItem::Wildcard(_) => Ok("*".into()),
            // MS25 T13（design D12）：`SelectItem` 四变体中前三个已显式匹配，
            // 兜底臂只承接 `QualifiedWildcard`（`t.*`）——点名实际能力，与
            // no-FROM 路径 "Qualified wildcard is not supported without FROM"
            // 的指称一致。拒绝语义与覆盖面零变化。
            _ => Err(PlanError::UnsupportedStatement("QUALIFIED WILDCARD".into())),
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
                // MS20 T2: alias.table.name 三段列引用——表名侧为已注册的限定
                // 扁平名（`"o.u"`），列名取末段。
                Expr::CompoundIdentifier(parts) if parts.len() == 3 => Ok((
                    Some(
                        format!(
                            "{}.{}",
                            parts[0].value,
                            parts[1].value
                        )
                        .to_lowercase(),
                    ),
                    parts[2].value.to_string().to_lowercase(),
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
                    // MS25 D7: 与 `extract_columns` 同源的 DISTINCT 感知命名
                    let distinct = f.distinct;
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
                                    Ok((
                                        None,
                                        if distinct {
                                            format!("count_distinct_{}", col.to_lowercase())
                                        } else {
                                            format!("count_{}", col.to_lowercase())
                                        },
                                    ))
                                }
                                _ => Ok((None, "count_star".to_string())),
                            }
                        }
                        "SUM" => {
                            let col = extract_single_col_static(&f.args, "SUM")?;
                            Ok((
                                None,
                                if distinct {
                                    format!("sum_distinct_{}", col.to_lowercase())
                                } else {
                                    format!("sum_{}", col.to_lowercase())
                                },
                            ))
                        }
                        "AVG" => {
                            let col = extract_single_col_static(&f.args, "AVG")?;
                            Ok((
                                None,
                                if distinct {
                                    format!("avg_distinct_{}", col.to_lowercase())
                                } else {
                                    format!("avg_{}", col.to_lowercase())
                                },
                            ))
                        }
                        "MIN" => {
                            let col = extract_single_col_static(&f.args, "MIN")?;
                            Ok((None, format!("min_{}", col.to_lowercase())))
                        }
                        "MAX" => {
                            let col = extract_single_col_static(&f.args, "MAX")?;
                            Ok((None, format!("max_{}", col.to_lowercase())))
                        }
                        // MS25 D7: GROUP_CONCAT 输出名只取首参列
                        "GROUP_CONCAT" => {
                            let col = extract_first_col_static(&f.args, "GROUP_CONCAT")?;
                            Ok((None, format!("group_concat_{}", col.to_lowercase())))
                        }
                        _ => Err(PlanError::UnsupportedStatement(name)),
                    }
                }
                Expr::Value(v) => Ok((None, format!("_{}", v))),
                // MS11-T01 Iter001: 值表达式项放行至 planner 的 SELECT 路由
                //（Display 文本对 JOIN 输出过滤惰性——表达式项 + JOIN 显式拒绝）
                // MS11-T03: TRIM/CEIL/FLOOR 独立变体同门放行。
                // MS13 T4: TypedString 与 BinaryOp（算术项）同门放行——本提取
                // 对非 JOIN 查询也执行（query.rs 无条件调用），表达式项 + JOIN
                // 的显式拒绝由 planner 路由层统一裁决（与 CASE/CAST 同型）。
                Expr::Case { .. }
                | Expr::Cast { .. }
                | Expr::InList { .. }
                | Expr::Between { .. }
                | Expr::Like { .. }
                | Expr::IsNull { .. }
                | Expr::IsNotNull { .. }
                | Expr::Trim { .. }
                | Expr::Ceil { .. }
                | Expr::Floor { .. }
                | Expr::TypedString { .. }
                | Expr::BinaryOp { .. }
                // MS13 T7: INTERVAL 腿同门放行（拒绝面由 planner 统一裁决）。
                | Expr::Interval { .. } => Ok((None, expr.to_string())),
                _ => Err(PlanError::UnsupportedStatement(
                    "UNSUPPORTED SELECT ITEM EXPRESSION".into(),
                )),
            },
            SelectItem::ExprWithAlias { expr, alias } => {
                // MS25 T8 / T10（design D9）：别名项引用普通列时按**被引用列**
                // 参与组合行形状承载——折叠为别名名会让 JOIN / 派生表基座的
                // 过滤式解析得到零宽行（`SELECT t1.v AS val FROM t1 JOIN t2 …`）。
                // 输出名由顶层 Projection 别名面给出（非聚合）或聚合输出装配
                // 给出（聚合）；其余表达式项维持别名名既有语义。
                if let Some(reference) = aliased_plain_column_ref(expr) {
                    return Ok(reference);
                }
                Ok((None, alias.value.to_string().to_lowercase()))
            }
            SelectItem::Wildcard(_) => Ok((None, "*".into())),
            // MS25 T13（design D12）：同 extract_columns——兜底臂只承接
            // `QualifiedWildcard`，点名能力而非泛化 `SELECT ITEM`。
            _ => Err(PlanError::UnsupportedStatement("QUALIFIED WILDCARD".into())),
        })
        .collect()
}

/// 从 ObjectName 提取表名（lowercase）
pub fn extract_name_from_object(obj: &ObjectName) -> String {
    object_name_to_table_name(obj)
}

/// MS15-Rest (I039): resolve a table name from its ObjectName by the
/// identifiers' unquoted values (quote_style-insensitive), lowercased —
/// quoted and bare spellings resolve to the same table. Multi-part names
/// join with '.', matching the previous Display shape.
///
/// MS20 T2: 拍平只在「确为多段 ObjectName」时有意义——`"foo.bar"` 是单个
/// Ident（字面表名），`foo.bar` 是两个 Ident。跨库路由判定必须区分二者，故
/// 分段信息由 [`object_name_parts`] 单独提供，本函数仍是既有消费点的键。
pub fn object_name_to_table_name(name: &ObjectName) -> String {
    object_name_parts(name).join(".")
}

/// MS20 T2: `ObjectName` 的分段形态（去引号 + 小写，逐段）——拍平前的原始
/// 结构。路由判定在分段尚存处进行（`alias.table` 首段查注册表），不得对
/// [`object_name_to_table_name`] 的扁平串再 split：单个 Ident 的字面表名
/// `"foo.bar"` 拍平后与之不可区分。
pub fn object_name_parts(name: &ObjectName) -> Vec<String> {
    name.0.iter().map(|id| id.value.to_lowercase()).collect()
}

/// 从 JOIN 关系的 TableFactor 提取表名
pub fn extract_join_table_name(relation: &TableFactor) -> Result<String, PlanError> {
    match relation {
        TableFactor::Table { name, .. } => Ok(object_name_to_table_name(name)),
        _ => Err(PlanError::UnsupportedStatement("DERIVED TABLE".into())),
    }
}

/// 从 TableFactor 提取表别名（MS25 T5：CTE 引用按引用处别名注册列信息）
pub fn table_factor_alias(relation: &TableFactor) -> Option<&TableAlias> {
    match relation {
        TableFactor::Table { alias, .. } => alias.as_ref(),
        _ => None,
    }
}

/// Extract column name from Expr (Identifier or CompoundIdentifier)
/// MS25 T8 / T10：别名项的被引用列——`SELECT t1.v AS val` 的承载列是
/// `(Some("t1"), "v")` 而非别名 `val`。裸 `NULL` 是字面量不是列，与
/// `is_plain_column_expr` 同口径。
fn aliased_plain_column_ref(expr: &Expr) -> Option<(Option<String>, String)> {
    match expr {
        Expr::Identifier(ident) if !ident.value.eq_ignore_ascii_case("NULL") => {
            Some((None, ident.value.to_lowercase()))
        }
        Expr::CompoundIdentifier(parts) if parts.len() == 2 => Some((
            Some(parts[0].value.to_lowercase()),
            parts[1].value.to_lowercase(),
        )),
        _ => None,
    }
}

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

/// MS25 D7: 提取首参列（`GROUP_CONCAT(col [, sep])`——分隔符不参与输出名，
/// 故不能用要求恰好一参的 `extract_single_col_static`）。
fn extract_first_col_static(
    args: &[sqlparser::ast::FunctionArg],
    func_name: &str,
) -> Result<String, PlanError> {
    match args.first() {
        Some(sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(expr))) => {
            expr_to_column_name_static(expr)
        }
        _ => Err(PlanError::InvalidAggregateArgument(format!(
            "{} argument must be a column",
            func_name
        ))),
    }
}
