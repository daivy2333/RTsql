//! SQL value types for physical plan execution

use super::ValueRef;
use crate::storage::page_format::Key;
use std::fmt;
use std::hash::{Hash, Hasher};

/// SQL 列类型（M9: 支持 Int/String/Null/Float/Bool；MS13: + Date/Timestamp）
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
    /// 日期类型（DATE，MS13：自 0001-01-01 起的天数）
    Date,
    /// 时间戳类型（TIMESTAMP，MS13：Unix epoch 微秒，无时区）
    Timestamp,
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
    /// 日期值（MS13: 新增，自 0001-01-01 起的天数）
    Date(i32),
    /// 时间戳值（MS13: 新增，Unix epoch 微秒，无时区）
    Timestamp(i64),
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
            Value::Null => {}                           // Null 没有额外数据
            Value::Float(f) => f.to_bits().hash(state), // 使用位表示进行哈希
            Value::Bool(b) => b.hash(state),
            Value::Date(d) => d.hash(state),
            Value::Timestamp(t) => t.hash(state),
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
            // MS13: Date/Timestamp 不可键控（DA2——Date 列作 PK 走 MS16 非
            // Int 键列路由回退，B-Tree 零改动）
            Value::Date(_) | Value::Timestamp(_) => None,
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

            // MS13: 日期族同类型比较（跨族由兜底返回 false）
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::Timestamp(a), Value::Timestamp(b)) => a == b,

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

            // MS13: 日期族同类型时间序比较
            (Value::Date(a), Value::Date(b)) => Ok(a > b),
            (Value::Timestamp(a), Value::Timestamp(b)) => Ok(a > b),

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

            // MS13: 日期族同类型时间序比较
            (Value::Date(a), Value::Date(b)) => Ok(a < b),
            (Value::Timestamp(a), Value::Timestamp(b)) => Ok(a < b),

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

            // MS13: 日期族同类型时间序比较
            (Value::Date(a), Value::Date(b)) => Ok(a >= b),
            (Value::Timestamp(a), Value::Timestamp(b)) => Ok(a >= b),

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

            // MS13: 日期族同类型时间序比较
            (Value::Date(a), Value::Date(b)) => Ok(a <= b),
            (Value::Timestamp(a), Value::Timestamp(b)) => Ok(a <= b),

            // 跨类型比较：Int vs Float
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) <= *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a <= (*b as f64)),

            // 不兼容类型
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// Addition (for SUM/AVG aggregation)
    pub fn add(&self, other: &Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
            (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 + b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a + *b as f64),
            _ => Value::Null,
        }
    }

    /// Simple less-than comparison returning bool (for MIN/MAX aggregation)
    /// NOTE: This is different from the existing `lt()` which returns Result<bool, ValueError>
    pub fn lt_agg(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a < b,
            (Value::Float(a), Value::Float(b)) => a < b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) < *b,
            (Value::Float(a), Value::Int(b)) => *a < (*b as f64),
            (Value::String(a), Value::String(b)) => a < b,
            // MS13: 日期族同类型 MIN/MAX 比较（跨族 false 兜底不变）
            (Value::Date(a), Value::Date(b)) => a < b,
            (Value::Timestamp(a), Value::Timestamp(b)) => a < b,
            _ => false,
        }
    }

    /// Division (for AVG aggregation)
    pub fn div(&self, other: &Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) if *b != 0 => Value::Int(a / b),
            (Value::Float(a), Value::Float(b)) if *b != 0.0 => Value::Float(a / b),
            (Value::Int(a), Value::Float(b)) if *b != 0.0 => Value::Float(*a as f64 / b),
            (Value::Float(a), Value::Int(b)) if *b != 0 => Value::Float(a / *b as f64),
            _ => Value::Null,
        }
    }

    /// Borrowed view — `String(s)` → `Text(s.as_str())` borrows s's heap.
    /// Other variants are zero-allocation conversions.
    pub fn as_value_ref(&self) -> ValueRef<'_> {
        match self {
            Value::Int(n) => ValueRef::Int(*n),
            Value::String(s) => ValueRef::Text(s.as_str()),
            Value::Null => ValueRef::Null,
            Value::Float(f) => ValueRef::Float(*f),
            Value::Bool(b) => ValueRef::Bool(*b),
            Value::Date(d) => ValueRef::Date(*d),
            Value::Timestamp(t) => ValueRef::Timestamp(*t),
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
            // MS13: DA5 显示格式（日期族不带引号）
            Value::Date(d) => write!(f, "{}", super::datetime::format_date(*d)),
            Value::Timestamp(t) => write!(f, "{}", super::datetime::format_timestamp(*t)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    #[test]
    fn datetime_equality_is_strict_same_type() {
        assert!(Value::Date(100).equals(&Value::Date(100)));
        assert!(!Value::Date(100).equals(&Value::Date(101)));
        // 跨族：日期 vs 数值/字符串/时间戳一律 false（比较严格）
        assert!(!Value::Date(100).equals(&Value::Int(100)));
        assert!(!Value::Date(100).equals(&Value::String("2024-01-15".into())));
        assert!(!Value::Timestamp(0).equals(&Value::Date(0)));
    }

    #[test]
    fn datetime_ordering_is_time_order() {
        let a = Value::Date(100);
        let b = Value::Date(200);
        assert_eq!(a.lt(&b), Ok(true));
        assert_eq!(a.gt(&b), Ok(false));
        assert_eq!(a.ge(&b), Ok(false));
        assert_eq!(a.le(&b), Ok(true));
        let t1 = Value::Timestamp(1_000);
        let t2 = Value::Timestamp(2_000);
        assert_eq!(t1.lt(&t2), Ok(true));
        assert_eq!(t2.gt(&t1), Ok(true));
    }

    #[test]
    fn datetime_cross_type_ordering_is_type_error() {
        assert_eq!(
            Value::Date(0).lt(&Value::Int(0)),
            Err(ValueError::TypeMismatch)
        );
        assert_eq!(
            Value::Date(0).gt(&Value::String("x".into())),
            Err(ValueError::TypeMismatch)
        );
        assert_eq!(
            Value::Timestamp(0).lt(&Value::Date(0)),
            Err(ValueError::TypeMismatch)
        );
        assert_eq!(
            Value::Date(0).lt(&Value::Null),
            Err(ValueError::NullComparison)
        );
    }

    #[test]
    fn datetime_hash_and_display() {
        // Hash 可用且区分变体
        let mut h1 = DefaultHasher::new();
        Value::Date(5).hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        Value::Date(5).hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
        let mut h3 = DefaultHasher::new();
        Value::Timestamp(5).hash(&mut h3);
        assert_ne!(h1.finish(), h3.finish());
        // Display = DA5 格式（日期族无引号，区别于 String 的引号惯例）
        assert_eq!(Value::Date(0).to_string(), "0001-01-01");
        assert_eq!(
            Value::Timestamp(1_234_567).to_string(),
            "1970-01-01 00:00:01.234567"
        );
    }

    #[test]
    fn datetime_not_keyable_and_ref_roundtrip() {
        assert_eq!(Value::Date(10).to_key(), None);
        assert_eq!(Value::Timestamp(10).to_key(), None);
        assert_eq!(Value::Date(10).as_value_ref(), ValueRef::Date(10));
        assert_eq!(ValueRef::Timestamp(10).to_value(), Value::Timestamp(10));
        assert_eq!(Value::Date(3).as_value_ref().to_value(), Value::Date(3));
    }

    #[test]
    fn datetime_agg_semantics_preserved() {
        // MIN/MAX 经 lt_agg：同型可比，跨族 false 兜底
        assert!(Value::Date(1).lt_agg(&Value::Date(2)));
        assert!(!Value::Date(2).lt_agg(&Value::Date(1)));
        assert!(Value::Timestamp(1).lt_agg(&Value::Timestamp(2)));
        assert!(!Value::Date(1).lt_agg(&Value::Timestamp(1)));
        // SUM/AVG 经 add/div：保持 Null 兜底（不误加）
        assert_eq!(Value::Date(1).add(&Value::Date(1)), Value::Null);
        assert_eq!(Value::Timestamp(1).div(&Value::Int(2)), Value::Null);
    }
}
