//! Value conversion from sqlparser AST to executor Value

use crate::executor::Value;
use crate::parser::error::PlanError;
use sqlparser::ast::Value as SqlValue;

/// 从 sqlparser AST Value 转换为 executor Value
pub fn value_from_sqlparser(v: &SqlValue) -> Result<Value, PlanError> {
    match v {
        // 数字（整数或浮点数）
        SqlValue::Number(n, _) => {
            // Try to parse as integer first
            if let Ok(num) = n.parse::<i64>() {
                Ok(Value::Int(num))
            } else if let Ok(num) = n.parse::<f64>() {
                Ok(Value::Float(num))
            } else {
                Err(PlanError::ParseError(format!("Invalid number: {}", n)))
            }
        }
        // 单引号字符串
        SqlValue::SingleQuotedString(s) => Ok(Value::String(s.clone())),
        // NULL
        SqlValue::Null => Ok(Value::Null),
        // Boolean
        SqlValue::Boolean(b) => Ok(Value::Bool(*b)),
        // 不支持的值类型
        _ => Err(PlanError::UnsupportedValue),
    }
}
