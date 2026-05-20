//! SQL value types for physical plan execution

use crate::storage::page_format::Key;
use std::fmt;

/// SQL 值类型（M4: 仅支持 Int/String/Null）
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 整数值
    Int(i64),
    /// 字符串值
    String(String),
    /// NULL 值
    Null,
}

impl Value {
    /// 转换为 Key（用于索引查找）
    /// 仅 Int 类型支持，返回其 big-endian 字节表示
    pub fn to_key(&self) -> Option<Key> {
        match self {
            Value::Int(n) => {
                let bytes = n.to_be_bytes();
                Some(Key::new(&bytes))
            }
            Value::String(_) | Value::Null => None,
        }
    }

    /// 检查是否为 NULL
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "'{}'", s),
            Value::Null => write!(f, "NULL"),
        }
    }
}
