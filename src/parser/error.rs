//! Parser errors

use std::fmt;

/// SQL 解析/计划错误
#[derive(Debug, Clone)]
pub enum PlanError {
    /// 解析错误
    ParseError(String),
    /// 不支持的值类型
    UnsupportedValue,
    /// 不支持的语句类型
    UnsupportedStatement,
    /// 缺少必要字段
    MissingField(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            PlanError::UnsupportedValue => write!(f, "Unsupported value type"),
            PlanError::UnsupportedStatement => write!(f, "Unsupported statement type"),
            PlanError::MissingField(field) => write!(f, "Missing required field: {}", field),
        }
    }
}

impl std::error::Error for PlanError {}