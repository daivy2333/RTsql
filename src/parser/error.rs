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
    /// 不支持的表达式类型
    UnsupportedExpression,
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
    /// 列名歧义（多表存在同名列）
    AmbiguousColumn(String),
    /// 表不存在
    TableNotFound(String),
    /// JOIN 缺少 ON 子句
    MissingOnClause,
    /// 不支持的 JOIN 类型（非 INNER）
    UnsupportedJoinType,
    /// 非聚合列未出现在 GROUP BY 中（严格模式）
    NonAggregatedColumn(String),
    /// 聚合函数参数错误
    InvalidAggregateArgument(String),
    /// GROUP BY 列不存在
    GroupByColumnNotFound(String),
    /// HAVING 中引用非聚合列
    HavingNonAggregatedReference(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            PlanError::UnsupportedValue => write!(f, "Unsupported value type"),
            PlanError::UnsupportedStatement => write!(f, "Unsupported statement type"),
            PlanError::UnsupportedExpression => write!(f, "Unsupported expression type"),
            PlanError::MissingField(field) => write!(f, "Missing required field: {}", field),
            PlanError::EmptyColumnDefinition => {
                write!(f, "Empty column definition in CREATE TABLE")
            }
            PlanError::MultiplePrimaryKey => write!(f, "Multiple primary keys in CREATE TABLE"),
            PlanError::ColumnNotFound(col) => write!(f, "Column not found: {}", col),
            PlanError::InvalidConstraint(msg) => write!(f, "Invalid constraint: {}", msg),
            PlanError::AmbiguousColumn(col) => {
                write!(f, "Ambiguous column: '{}' exists in multiple tables", col)
            }
            PlanError::TableNotFound(table) => write!(f, "Table not found: {}", table),
            PlanError::MissingOnClause => write!(f, "INNER JOIN requires ON clause"),
            PlanError::UnsupportedJoinType => write!(f, "Only INNER JOIN is supported"),
            PlanError::NonAggregatedColumn(col) => write!(f, "Non-aggregated column '{}' must appear in GROUP BY clause", col),
            PlanError::InvalidAggregateArgument(msg) => write!(f, "Invalid aggregate argument: {}", msg),
            PlanError::GroupByColumnNotFound(col) => write!(f, "GROUP BY column not found: {}", col),
            PlanError::HavingNonAggregatedReference(col) => write!(f, "HAVING references non-aggregated column: {}", col),
        }
    }
}

impl std::error::Error for PlanError {}
