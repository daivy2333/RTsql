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
                _ => Err(PlanError::UnsupportedStatement),
            },
            SelectItem::Wildcard(_) => Ok("*".into()),
            _ => Err(PlanError::UnsupportedStatement),
        })
        .collect()
}

/// 从 projection 提取完整的列信息（table.column 格式）
/// 返回 Vec<(Option<String>, String)> - (table_name, column_name)
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
                Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                    Ok((
                        Some(parts[0].value.to_string().to_lowercase()),
                        parts[1].value.to_string().to_lowercase(),
                    ))
                }
                _ => Err(PlanError::UnsupportedStatement),
            },
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
