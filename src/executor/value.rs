//! SQL value types for physical plan execution

use crate::storage::page_format::Key;
use std::fmt;
use std::hash::{Hash, Hasher};

/// SQL 列类型（M9: 支持 Int/String/Null/Float/Bool）
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnType {
    /// 整数类型
    Int,
    /// 字符串类型
    String,
    /// 浮点类型（FLOAT/DOUBLE）
    Float,
    /// 布尔类型（BOOLEAN）
    Bool,
}

/// 值类型错误
#[derive(Debug, Clone, PartialEq)]
pub enum ValueError {
    /// 类型不匹配
    TypeMismatch,
    /// 列不存在
    ColumnNotFound(String),
    /// NULL 比较错误
    NullComparison,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::TypeMismatch => write!(f, "Type mismatch"),
            ValueError::ColumnNotFound(name) => write!(f, "Column not found: {}", name),
            ValueError::NullComparison => write!(f, "Cannot compare NULL values"),
        }
    }
}

impl std::error::Error for ValueError {}

/// SQL 值类型（M4: 仅支持 Int/String/Null）
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 整数值
    Int(i64),
    /// 字符串值
    String(String),
    /// NULL 值
    Null,
    /// 浮点值（M9: 新增）
    Float(f64),
    /// 布尔值（M9: 新增）
    Bool(bool),
}

// 手动实现 Eq，因为 f64 不实现 Eq
// 对于 Float，使用 to_bits() 进行相等比较
impl Eq for Value {}

// 手动实现 Hash，因为 f64 不实现 Hash
// 使用 to_bits() 将 f64 转换为 u64 进行哈希
impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // 使用 discriminant 区分不同变体
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Int(n) => n.hash(state),
            Value::String(s) => s.hash(state),
            Value::Null => {} // Null 没有额外数据
            Value::Float(f) => f.to_bits().hash(state), // 使用位表示进行哈希
            Value::Bool(b) => b.hash(state),
        }
    }
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
            Value::String(_) | Value::Null | Value::Float(_) | Value::Bool(_) => None,
        }
    }

    /// 检查是否为 NULL
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// 转换为浮点数（支持隐式转换：Int -> Float）
    pub fn as_float(&self) -> Result<f64, ValueError> {
        match self {
            Value::Float(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),
            Value::Null => Err(ValueError::NullComparison),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// 转换为布尔值（支持隐式转换：Int -> Bool，0 为 false，非 0 为 true）
    pub fn as_bool(&self) -> Result<bool, ValueError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Int(i) => Ok(*i != 0),
            Value::Null => Err(ValueError::NullComparison),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// 相等比较（支持跨类型：Int vs Float）
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            // NULL 比较：只有 NULL == NULL
            (Value::Null, Value::Null) => true,
            (Value::Null, _) | (_, Value::Null) => false,

            // 同类型比较
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,

            // 跨类型比较：Int vs Float（隐式转换）
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),

            // 不兼容类型比较
            _ => false,
        }
    }

    /// 大于比较（支持跨类型：Int vs Float）
    pub fn gt(&self, other: &Value) -> Result<bool, ValueError> {
        match (self, other) {
            (Value::Null, _) | (_, Value::Null) => Err(ValueError::NullComparison),

            // 同类型比较
            (Value::Int(a), Value::Int(b)) => Ok(a > b),
            (Value::String(a), Value::String(b)) => Ok(a > b),
            (Value::Float(a), Value::Float(b)) => Ok(a > b),
            (Value::Bool(a), Value::Bool(b)) => Ok(a > b),

            // 跨类型比较：Int vs Float
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) > *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a > (*b as f64)),

            // 不兼容类型
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// 小于比较（支持跨类型：Int vs Float）
    pub fn lt(&self, other: &Value) -> Result<bool, ValueError> {
        match (self, other) {
            (Value::Null, _) | (_, Value::Null) => Err(ValueError::NullComparison),

            // 同类型比较
            (Value::Int(a), Value::Int(b)) => Ok(a < b),
            (Value::String(a), Value::String(b)) => Ok(a < b),
            (Value::Float(a), Value::Float(b)) => Ok(a < b),
            (Value::Bool(a), Value::Bool(b)) => Ok(a < b),

            // 跨类型比较：Int vs Float
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) < *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a < (*b as f64)),

            // 不兼容类型
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// 大于等于比较（支持跨类型：Int vs Float）
    pub fn ge(&self, other: &Value) -> Result<bool, ValueError> {
        match (self, other) {
            (Value::Null, _) | (_, Value::Null) => Err(ValueError::NullComparison),

            // 同类型比较
            (Value::Int(a), Value::Int(b)) => Ok(a >= b),
            (Value::String(a), Value::String(b)) => Ok(a >= b),
            (Value::Float(a), Value::Float(b)) => Ok(a >= b),
            (Value::Bool(a), Value::Bool(b)) => Ok(a >= b),

            // 跨类型比较：Int vs Float
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) >= *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a >= (*b as f64)),

            // 不兼容类型
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// 小于等于比较（支持跨类型：Int vs Float）
    pub fn le(&self, other: &Value) -> Result<bool, ValueError> {
        match (self, other) {
            (Value::Null, _) | (_, Value::Null) => Err(ValueError::NullComparison),

            // 同类型比较
            (Value::Int(a), Value::Int(b)) => Ok(a <= b),
            (Value::String(a), Value::String(b)) => Ok(a <= b),
            (Value::Float(a), Value::Float(b)) => Ok(a <= b),
            (Value::Bool(a), Value::Bool(b)) => Ok(a <= b),

            // 跨类型比较：Int vs Float
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) <= *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a <= (*b as f64)),

            // 不兼容类型
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "'{}'", s),
            Value::Null => write!(f, "NULL"),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
        }
    }
}
