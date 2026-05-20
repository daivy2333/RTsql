//! Value conversion from sqlparser AST to executor Value

use sqlparser::ast::Value as SqlValue;
use crate::executor::Value;
use crate::parser::error::PlanError;

/// 从 sqlparser AST Value 转换为 executor Value
pub fn value_from_sqlparser(v: &SqlValue) -> Result<Value, PlanError> {
    match v {
        // 数字（整数）
        SqlValue::Number(n, _) => {
            let num: i64 = n.parse()
                .map_err(|_| PlanError::ParseError(format!("Invalid number: {}", n)))?;
            Ok(Value::Int(num))
        }
        // 单引号字符串
        SqlValue::SingleQuotedString(s) => {
            Ok(Value::String(s.clone()))
        }
        // NULL
        SqlValue::Null => Ok(Value::Null),
        // 不支持的值类型
        _ => Err(PlanError::UnsupportedValue),
    }
}