//! Parser errors

use std::fmt;

/// SQL 解析/计划错误
#[derive(Debug, Clone)]
pub enum PlanError {
    /// 解析错误
    ParseError(String),
    /// 不支持的值类型
    UnsupportedValue,
    /// 不支持的语句类型（载荷为被拒能力名，MS19-T02）
    UnsupportedStatement(String),
    /// 无效查询
    InvalidQuery(String),
    /// 不支持的表达式类型（载荷为被拒能力名，MS19-T02）
    UnsupportedExpression(String),
    /// 缺少必要字段
    MissingField(String),
    /// CREATE TABLE 空列定义
    EmptyColumnDefinition,
    /// CTAS 输出列类型冲突（载荷为列名与参与类型，MS20-T04）
    CtasColumnTypeConflict(String),
    /// 声明期 DEFAULT 类型不一致（载荷为列名/期望/实际，MS29-T04：建表与
    /// 加列的 DEFAULT 字面量计划期校验失败，规则与写入值类型一致门对齐）
    DefaultTypeMismatch(String),
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
    /// 子查询返回多行（标量子查询要求单行）
    SubqueryReturnsMultipleRow,
    /// 子查询返回多列（IN 子查询要求单列）
    SubqueryReturnsMultipleColumns,
    /// IN 子查询计划含 JOIN 节点（MS17-T02/ISS02：点名 JOIN 的诚实拒绝）
    InSubqueryJoinUnsupported,
    /// 建表约束点名拒绝（MS23：CHECK/FOREIGN KEY/方言项不再静默忽略）
    UnsupportedConstraint(&'static str),
    /// 标量子查询返回空结果
    SubqueryReturnsEmpty,
    /// 不支持的子查询位置
    UnsupportedSubqueryPosition,
    /// 相关子查询参数解析错误
    CorrelatedParamError(String),
    /// NOT IN 子查询包含 NULL 值
    NotInWithNull,
    /// 事务语句/边界子句拒绝（MS11-T02；消息即全文）
    TransactionStatement(String),
    /// ATTACH 语句拒绝（MS20 T1；消息即全文）
    AttachStatement(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            PlanError::UnsupportedValue => write!(f, "Unsupported value type"),
            PlanError::UnsupportedStatement(name) => {
                write!(f, "Unsupported statement type: {}", name)
            }
            PlanError::InvalidQuery(msg) => write!(f, "Invalid query: {}", msg),
            PlanError::UnsupportedExpression(name) => {
                write!(f, "Unsupported expression type: {}", name)
            }
            PlanError::MissingField(field) => write!(f, "Missing required field: {}", field),
            PlanError::EmptyColumnDefinition => {
                write!(f, "Empty column definition in CREATE TABLE")
            }
            PlanError::CtasColumnTypeConflict(detail) => {
                write!(f, "CTAS column type conflict: {}", detail)
            }
            PlanError::DefaultTypeMismatch(detail) => {
                write!(f, "DEFAULT type mismatch: {}", detail)
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
            PlanError::NonAggregatedColumn(col) => write!(
                f,
                "Non-aggregated column '{}' must appear in GROUP BY clause",
                col
            ),
            PlanError::InvalidAggregateArgument(msg) => {
                write!(f, "Invalid aggregate argument: {}", msg)
            }
            PlanError::GroupByColumnNotFound(col) => {
                write!(f, "GROUP BY column not found: {}", col)
            }
            PlanError::HavingNonAggregatedReference(col) => {
                write!(f, "HAVING references non-aggregated column: {}", col)
            }
            PlanError::SubqueryReturnsMultipleRow => write!(
                f,
                "Subquery returns multiple rows (scalar subquery requires single row)"
            ),
            PlanError::SubqueryReturnsMultipleColumns => write!(
                f,
                "Subquery returns multiple columns (IN subquery requires single column)"
            ),
            PlanError::InSubqueryJoinUnsupported => {
                write!(f, "IN subquery with JOIN is not supported")
            }
            PlanError::UnsupportedConstraint(what) => write!(f, "Unsupported constraint: {}", what),
            PlanError::SubqueryReturnsEmpty => write!(f, "Scalar subquery returns empty result"),
            PlanError::UnsupportedSubqueryPosition => write!(f, "Unsupported subquery position"),
            PlanError::CorrelatedParamError(msg) => {
                write!(f, "Correlated subquery parameter error: {}", msg)
            }
            PlanError::NotInWithNull => write!(f, "NOT IN subquery contains NULL values"),
            PlanError::TransactionStatement(msg) => write!(f, "{}", msg),
            PlanError::AttachStatement(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for PlanError {}

impl PlanError {
    /// PG 协议 SQLSTATE 分类（MS19-T02：能力面拒绝 0A000、解析 42601、
    /// 表 42P01、列 42703、列名歧义 42702，其余未分类 58000 兜底）。
    pub fn sqlstate(&self) -> &'static str {
        match self {
            PlanError::ParseError(_) => "42601",
            PlanError::UnsupportedValue
            | PlanError::UnsupportedStatement(_)
            | PlanError::UnsupportedExpression(_)
            | PlanError::UnsupportedJoinType
            | PlanError::UnsupportedConstraint(_)
            | PlanError::UnsupportedSubqueryPosition
            | PlanError::InSubqueryJoinUnsupported
            | PlanError::TransactionStatement(_)
            | PlanError::AttachStatement(_) => "0A000",
            PlanError::TableNotFound(_) => "42P01",
            PlanError::ColumnNotFound(_) | PlanError::GroupByColumnNotFound(_) => "42703",
            PlanError::AmbiguousColumn(_) => "42702",
            _ => "58000",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R3：SQLSTATE 分类表全变体可枚举（MS19-T02）。
    #[test]
    fn planerror_sqlstate_mapping() {
        let cases: Vec<(PlanError, &str)> = vec![
            (PlanError::ParseError("boom".into()), "42601"),
            (PlanError::UnsupportedValue, "0A000"),
            (PlanError::UnsupportedStatement("MERGE".into()), "0A000"),
            (
                PlanError::UnsupportedExpression("PREDICATE".into()),
                "0A000",
            ),
            (PlanError::UnsupportedJoinType, "0A000"),
            (PlanError::UnsupportedConstraint("CHECK"), "0A000"),
            (PlanError::UnsupportedSubqueryPosition, "0A000"),
            (PlanError::InSubqueryJoinUnsupported, "0A000"),
            (PlanError::TransactionStatement("SAVEPOINT".into()), "0A000"),
            (PlanError::AttachStatement("ATTACH".into()), "0A000"),
            (PlanError::TableNotFound("t".into()), "42P01"),
            (PlanError::ColumnNotFound("c".into()), "42703"),
            (PlanError::GroupByColumnNotFound("c".into()), "42703"),
            (PlanError::AmbiguousColumn("c".into()), "42702"),
            // 未分类错误维持 58000 兜底
            (PlanError::InvalidQuery("q".into()), "58000"),
            (PlanError::MissingField("f".into()), "58000"),
            (PlanError::EmptyColumnDefinition, "58000"),
            (PlanError::CtasColumnTypeConflict("c".into()), "58000"),
            (PlanError::DefaultTypeMismatch("d".into()), "58000"),
            (PlanError::MultiplePrimaryKey, "58000"),
            (PlanError::InvalidConstraint("c".into()), "58000"),
            (PlanError::MissingOnClause, "58000"),
            (PlanError::NonAggregatedColumn("c".into()), "58000"),
            (PlanError::InvalidAggregateArgument("a".into()), "58000"),
            (PlanError::HavingNonAggregatedReference("c".into()), "58000"),
            (PlanError::SubqueryReturnsMultipleRow, "58000"),
            (PlanError::SubqueryReturnsMultipleColumns, "58000"),
            (PlanError::SubqueryReturnsEmpty, "58000"),
            (PlanError::CorrelatedParamError("e".into()), "58000"),
            (PlanError::NotInWithNull, "58000"),
        ];

        for (err, expected) in cases {
            assert_eq!(err.sqlstate(), expected, "{:?}", err);
        }
    }

    /// R2：两变体 Display 追加能力名，既有文案框架保持。
    #[test]
    fn unsupported_variants_display_capability_name() {
        assert_eq!(
            PlanError::UnsupportedStatement("MERGE".into()).to_string(),
            "Unsupported statement type: MERGE"
        );
        assert_eq!(
            PlanError::UnsupportedExpression("ARITHMETIC OPERATOR %".into()).to_string(),
            "Unsupported expression type: ARITHMETIC OPERATOR %"
        );
    }
}
