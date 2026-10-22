//! 库公共执行结果类型（MS31-T01）。
//!
//! 核心执行路径不再以 `network::protocol::Response` 承载结果——本模块是库
//! 公共入口的类型化结果载体，协议词表退化为 CLI / Server 边界转换。
//!
//! 本模块**不依赖 `network`**（核心/协议解耦的结构性落点）：`serde_json`
//! 依赖保留——`value_to_json` 是 CLI 与协议边界共用的值转换工具，不属于
//! 协议词表。

use crate::executor::Value;
use crate::storage::ColumnType;

/// 逐列元数据：列名 + 声明类型。
///
/// `column_type` 为 `None` 表示类型不可得——表达式列、聚合列、派生表列等
/// 无法由声明类型确定的形态。诚实表示，不虚构类型。
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnMeta {
    pub name: String,
    pub column_type: Option<ColumnType>,
}

/// 类型化执行结果。
#[derive(Debug, Clone, PartialEq)]
pub enum SqlOutcome {
    /// 查询结果：逐列元数据 + 原生 `Value` 行。
    Rows {
        columns: Vec<ColumnMeta>,
        rows: Vec<Vec<Value>>,
    },
    /// 写面结果：受影响行数。
    AffectedRows(u64),
}

/// 语句错误载荷：消息与 SQLSTATE 分类。
///
/// 与既有 `network::protocol::Response::Error` 同源同分类——消息文本逐字节
/// 相同，仅返回形态从协议词表改为 `Result::Err`。
#[derive(Debug, Clone, PartialEq)]
pub struct SqlError {
    pub message: String,
    pub sqlstate: Option<String>,
}

impl std::fmt::Display for SqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SqlError {}

/// 原生值 → JSON 值（CLI 与协议边界共用）。
///
/// 转换逻辑逐字节不变：Float NaN/Inf → Null（JSON 不可表示）、Date 与
/// Timestamp → DA5 字符串、Bool/Int/String/Null 原样。
pub fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Int(n) => serde_json::Value::Number(n.into()),
        Value::String(s) => serde_json::Value::String(s),
        Value::Null => serde_json::Value::Null,
        Value::Float(f) => {
            // serde_json doesn't support special float values (NaN, Inf)
            // so we need to handle them carefully
            if f.is_finite() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                // NaN, Infinity -> Null (not representable in JSON)
                serde_json::Value::Null
            }
        }
        Value::Bool(b) => serde_json::Value::Bool(b),
        // MS13: 日期族按 DA5 字符串形态渲染
        Value::Date(d) => serde_json::Value::String(crate::executor::datetime::format_date(d)),
        Value::Timestamp(t) => {
            serde_json::Value::String(crate::executor::datetime::format_timestamp(t))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Value;

    /// T1：原生值 → JSON 转换逐字节保持——Float NaN/Inf → Null（JSON 不可
    /// 表示）、Bool/Int/String/Null 原样。
    #[test]
    fn value_to_json_preserves_conversion_semantics() {
        assert_eq!(value_to_json(Value::Int(7)), serde_json::json!(7));
        assert_eq!(
            value_to_json(Value::String("a".to_string())),
            serde_json::json!("a")
        );
        assert_eq!(value_to_json(Value::Null), serde_json::Value::Null);
        assert_eq!(value_to_json(Value::Bool(true)), serde_json::json!(true));
        assert_eq!(
            value_to_json(Value::Float(f64::NAN)),
            serde_json::Value::Null
        );
        assert_eq!(
            value_to_json(Value::Float(f64::INFINITY)),
            serde_json::Value::Null
        );
        assert_eq!(value_to_json(Value::Float(1.5)), serde_json::json!(1.5));
    }

    /// T1：日期族按 DA5 字符串形态渲染。
    #[test]
    fn value_to_json_renders_datetime_family_as_da5_string() {
        assert_eq!(
            value_to_json(Value::Date(0)),
            serde_json::json!("0001-01-01")
        );
        assert_eq!(
            value_to_json(Value::Timestamp(0)),
            serde_json::json!("1970-01-01 00:00:00")
        );
    }
}
