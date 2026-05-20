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
    /// CREATE TABLE 空列定义
    EmptyColumnDefinition,
    /// CREATE TABLE 多主键
    MultiplePrimaryKey,
    /// WHERE 列不存在
    ColumnNotFound(String),
    /// 无效约束
    InvalidConstraint(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            PlanError::UnsupportedValue => write!(f, "Unsupported value type"),
            PlanError::UnsupportedStatement => write!(f, "Unsupported statement type"),
            PlanError::MissingField(field) => write!(f, "Missing required field: {}", field),
            PlanError::EmptyColumnDefinition => write!(f, "Empty column definition in CREATE TABLE"),
            PlanError::MultiplePrimaryKey => write!(f, "Multiple primary keys in CREATE TABLE"),
            PlanError::ColumnNotFound(col) => write!(f, "Column not found: {}", col),
            PlanError::InvalidConstraint(msg) => write!(f, "Invalid constraint: {}", msg),
        }
    }
}

impl std::error::Error for PlanError {}
