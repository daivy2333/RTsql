//! SQL value types for physical plan execution

use super::ValueRef;
use crate::storage::page_format::{Key, MAX_KEY_LEN};
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

    /// 二级索引键。
    ///
    /// `Int` 与 `to_key` 相同。`String` 在完整转义长度不超过 32 字节时入树：
    /// 非零字节原样写入，输入 `00` 写成 `00 FF`，结果末尾追加 `00 00`。
    /// 声明长度不参与判断。其余列类型和 `Null` 返回 `None`。
    pub fn secondary_index_key(
        &self,
        column_type: &crate::storage::page_format::ColumnType,
    ) -> Option<Key> {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        match column_type {
            SchemaColumnType::Int => self.to_key(),
            SchemaColumnType::String(_) => match self {
                Value::String(text) => fixed32_string_key(text),
                _ => None,
            },
            SchemaColumnType::Float
            | SchemaColumnType::Bool
            | SchemaColumnType::Date
            | SchemaColumnType::Timestamp => None,
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

    /// 按世代派生二级索引键。
    ///
    /// 世代 0 只接受恰好一列，字节与 [`Self::secondary_index_key`] 相同。
    /// 世代 1 按列序拼接：整数是首字节异或 `0x80` 的大端 8 字节，字符串是
    /// 现有转义且不含键级填零。总长超过 32 字节、NULL、列数不配对或不支持的类型返回 `None`。
    pub fn derive_secondary_index_key(
        columns: &[crate::storage::page_format::ColumnType],
        values: &[Value],
        generation: u8,
    ) -> Option<Key> {
        if generation == 0 {
            if columns.len() != 1 || values.len() != 1 {
                return None;
            }
            return values[0].secondary_index_key(&columns[0]);
        }
        if generation != 1 || columns.len() != values.len() || columns.is_empty() {
            return None;
        }
        let mut encoded = Vec::new();
        for (column, value) in columns.iter().zip(values.iter()) {
            append_ordered_component(&mut encoded, column, value)?;
        }
        Some(Key::new(&encoded))
    }

    /// 由前缀约束计算世代 1 的范围哨兵。
    ///
    /// 除最后一列外必须是等值。最后一列可以是等值，或带包含标记的上下界。
    /// 前缀长于索引、类型不匹配、NULL、空范围或编码放不下时返回 `None`。
    pub fn secondary_range_bounds(
        index_columns: &[crate::storage::page_format::ColumnType],
        prefix_bounds: &[IndexColumnBound],
    ) -> Option<SecondaryRangeBounds> {
        if prefix_bounds.is_empty() || prefix_bounds.len() > index_columns.len() {
            return None;
        }
        let last = prefix_bounds.len() - 1;
        for bound in &prefix_bounds[..last] {
            if !matches!(bound, IndexColumnBound::Eq(_)) {
                return None;
            }
        }

        let trailing_range = match &prefix_bounds[last] {
            IndexColumnBound::Eq(_) => None,
            IndexColumnBound::Range { lower, upper } => {
                if lower.is_none() && upper.is_none() {
                    return None;
                }
                Some((lower.as_ref(), upper.as_ref()))
            }
        };

        let equality_len = if trailing_range.is_some() {
            last
        } else {
            prefix_bounds.len()
        };
        let mut equality_prefix = Vec::new();
        for (bound, column) in prefix_bounds[..equality_len]
            .iter()
            .zip(index_columns.iter())
        {
            let IndexColumnBound::Eq(value) = bound else {
                return None;
            };
            append_ordered_component(&mut equality_prefix, column, value)?;
        }

        if let Some((lower, upper)) = trailing_range {
            let column = &index_columns[last];
            let lower_sentinel = if let Some(end) = lower {
                let mut prefix = equality_prefix.clone();
                append_ordered_component(&mut prefix, column, &end.value)?;
                Some(range_sentinel(&prefix, end.inclusive, true))
            } else if !equality_prefix.is_empty() {
                Some(range_sentinel(&equality_prefix, true, true))
            } else {
                None
            };
            let upper_sentinel = if let Some(end) = upper {
                let mut prefix = equality_prefix.clone();
                append_ordered_component(&mut prefix, column, &end.value)?;
                Some(range_sentinel(&prefix, end.inclusive, false))
            } else if !equality_prefix.is_empty() {
                Some(range_sentinel(&equality_prefix, true, false))
            } else {
                None
            };
            if lower_sentinel.is_none() && upper_sentinel.is_none() {
                return None;
            }
            return Some(SecondaryRangeBounds {
                lower: lower_sentinel,
                upper: upper_sentinel,
            });
        }

        if equality_prefix.is_empty() {
            return None;
        }
        Some(SecondaryRangeBounds {
            lower: Some(range_sentinel(&equality_prefix, true, true)),
            upper: Some(range_sentinel(&equality_prefix, true, false)),
        })
    }
}

/// 一列范围端点。`inclusive` 为真时端点包含该值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundMark {
    pub value: Value,
    pub inclusive: bool,
}

/// 前缀中的一列约束。范围只允许出现在前缀最后一列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexColumnBound {
    Eq(Value),
    Range {
        lower: Option<BoundMark>,
        upper: Option<BoundMark>,
    },
}

/// 比较用的范围端点。`inclusive` 为真时下界是大于等于、上界是小于等于。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSentinel {
    pub key: Key,
    pub inclusive: bool,
}

/// 世代 1 范围扫描的下界和上界。缺席的一侧表示该方向没有约束。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecondaryRangeBounds {
    pub lower: Option<RangeSentinel>,
    pub upper: Option<RangeSentinel>,
}

/// 把字符串编成现有转义字节，不含键级填零。超长时返回 `None`，不截断。
fn encode_string_bytes(text: &str) -> Option<Vec<u8>> {
    let raw = text.as_bytes();
    // 每个输入字节至少占 1 字节，再加结尾 `00 00`。超过 30 字节的原文不可能入树。
    if raw.len() > MAX_KEY_LEN - 2 {
        return None;
    }
    let mut encoded = Vec::with_capacity(raw.len().saturating_add(2));
    for byte in raw {
        if *byte == 0 {
            encoded.push(0x00);
            encoded.push(0xFF);
        } else {
            encoded.push(*byte);
        }
        if encoded.len() > MAX_KEY_LEN {
            return None;
        }
    }
    encoded.push(0x00);
    encoded.push(0x00);
    if encoded.len() > MAX_KEY_LEN {
        None
    } else {
        Some(encoded)
    }
}

/// 把字符串编成可放进现有 32 字节键的字节。超长时返回 `None`，不截断。
fn fixed32_string_key(text: &str) -> Option<Key> {
    encode_string_bytes(text).map(|encoded| Key::new(&encoded))
}

fn encode_ordered_component(
    column: &crate::storage::page_format::ColumnType,
    value: &Value,
) -> Option<Vec<u8>> {
    use crate::storage::page_format::ColumnType as SchemaColumnType;
    match (column, value) {
        (SchemaColumnType::Int, Value::Int(n)) => {
            let mut bytes = n.to_be_bytes();
            bytes[0] ^= 0x80;
            Some(bytes.to_vec())
        }
        (SchemaColumnType::String(_), Value::String(text)) => encode_string_bytes(text),
        _ => None,
    }
}

fn append_ordered_component(
    buf: &mut Vec<u8>,
    column: &crate::storage::page_format::ColumnType,
    value: &Value,
) -> Option<()> {
    let part = encode_ordered_component(column, value)?;
    if buf.len() + part.len() > MAX_KEY_LEN {
        return None;
    }
    buf.extend(part);
    Some(())
}

/// 包含下界和不包含上界补零；不包含下界和包含上界补 `FF`。
/// 前缀已经是 32 字节时补不进去，哨兵就是这 32 字节本身。
fn range_sentinel(prefix: &[u8], inclusive: bool, is_lower: bool) -> RangeSentinel {
    let fill_ff = if is_lower { !inclusive } else { inclusive };
    let key = if fill_ff {
        let mut bytes = [0u8; MAX_KEY_LEN];
        bytes[..prefix.len()].copy_from_slice(prefix);
        for byte in &mut bytes[prefix.len()..] {
            *byte = 0xFF;
        }
        Key::new(&bytes)
    } else {
        Key::new(prefix)
    };
    RangeSentinel { key, inclusive }
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

    #[test]
    fn secondary_index_key_preserves_int_bytes() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        let int_col = SchemaColumnType::Int;
        for n in [0i64, -1, i64::MIN, i64::MAX] {
            let value = Value::Int(n);
            let derived = value
                .secondary_index_key(&int_col)
                .unwrap_or_else(|| panic!("{n} must produce a key"));
            let direct = value
                .to_key()
                .unwrap_or_else(|| panic!("{n} must be keyable"));
            assert_eq!(derived.as_bytes(), direct.as_bytes());
        }
        assert!(Value::Null.secondary_index_key(&int_col).is_none());
        assert!(Value::String("1".into())
            .secondary_index_key(&int_col)
            .is_none());
    }

    #[test]
    fn string_secondary_key_preserves_full_value_within_32() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;

        fn key_of(text: &str, declared: u16) -> Option<crate::storage::page_format::Key> {
            Value::String(text.to_string()).secondary_index_key(&SchemaColumnType::String(declared))
        }

        let empty = key_of("", 8).expect("empty string fits");
        assert_eq!(empty.as_bytes(), &[0x00, 0x00]);

        let letter = key_of("a", 1).expect("declared length does not reject the key");
        assert_eq!(letter.as_bytes(), &[0x61, 0x00, 0x00]);

        let nul = key_of("\0", 8).expect("embedded NUL fits");
        assert_eq!(nul.as_bytes(), &[0x00, 0xFF, 0x00, 0x00]);

        let prefix_a = key_of("prefix-a", 64).expect("shared prefix a");
        let prefix_b = key_of("prefix-b", 64).expect("shared prefix b");
        assert!(prefix_a.full_data() < prefix_b.full_data());

        let utf8 = key_of("你好", 64).expect("multibyte UTF-8 fits");
        assert_eq!(
            utf8.as_bytes(),
            &[0xE4, 0xBD, 0xA0, 0xE5, 0xA5, 0xBD, 0x00, 0x00]
        );

        let trailing = key_of("ab\0", 64).expect("trailing zero byte fits");
        assert_eq!(trailing.as_bytes(), &[0x61, 0x62, 0x00, 0xFF, 0x00, 0x00]);

        let thirty = "c".repeat(30);
        let fitted = key_of(&thirty, 1).expect("30 non-zero bytes fit");
        assert_eq!(fitted.as_bytes().len(), 32);
        assert!(fitted.as_bytes()[..30].iter().all(|byte| *byte == b'c'));
        assert_eq!(&fitted.as_bytes()[30..], &[0x00, 0x00]);

        let distinct = [
            empty.full_data(),
            letter.full_data(),
            nul.full_data(),
            prefix_a.full_data(),
            prefix_b.full_data(),
            utf8.full_data(),
            trailing.full_data(),
            fitted.full_data(),
        ];
        for (i, left) in distinct.iter().enumerate() {
            for right in distinct.iter().skip(i + 1) {
                assert_ne!(left, right);
            }
        }

        assert!(key_of(&"d".repeat(31), 255).is_none());
        assert!(Value::Null
            .secondary_index_key(&SchemaColumnType::String(8))
            .is_none());
        assert!(Value::Int(1)
            .secondary_index_key(&SchemaColumnType::String(8))
            .is_none());
        assert!(Value::String("a".into())
            .secondary_index_key(&SchemaColumnType::Float)
            .is_none());
    }

    fn derive_ints(values: &[i64], generation: u8) -> Option<Key> {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        let columns = vec![SchemaColumnType::Int; values.len()];
        let stored: Vec<Value> = values.iter().copied().map(Value::Int).collect();
        Value::derive_secondary_index_key(&columns, &stored, generation)
    }

    fn eq_int(value: i64) -> IndexColumnBound {
        IndexColumnBound::Eq(Value::Int(value))
    }

    fn lower_int(value: i64, inclusive: bool) -> IndexColumnBound {
        IndexColumnBound::Range {
            lower: Some(BoundMark {
                value: Value::Int(value),
                inclusive,
            }),
            upper: None,
        }
    }

    fn upper_int(value: i64, inclusive: bool) -> IndexColumnBound {
        IndexColumnBound::Range {
            lower: None,
            upper: Some(BoundMark {
                value: Value::Int(value),
                inclusive,
            }),
        }
    }

    fn key_satisfies(key: &Key, bounds: &SecondaryRangeBounds) -> bool {
        if let Some(lower) = &bounds.lower {
            let ordering = key.cmp(&lower.key);
            let matches = if lower.inclusive {
                ordering != std::cmp::Ordering::Less
            } else {
                ordering == std::cmp::Ordering::Greater
            };
            if !matches {
                return false;
            }
        }
        if let Some(upper) = &bounds.upper {
            let ordering = key.cmp(&upper.key);
            let matches = if upper.inclusive {
                ordering != std::cmp::Ordering::Greater
            } else {
                ordering == std::cmp::Ordering::Less
            };
            if !matches {
                return false;
            }
        }
        true
    }

    #[test]
    fn ordered_int_key_matches_signed_order() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        let keys: Vec<Key> = [i64::MIN, -1, 0, 1, i64::MAX]
            .into_iter()
            .map(|n| {
                derive_ints(&[n], 1).unwrap_or_else(|| panic!("{n} must produce an ordered key"))
            })
            .collect();
        for pair in keys.windows(2) {
            assert!(
                pair[0] < pair[1],
                "signed order must be MIN < -1 < 0 < 1 < MAX"
            );
        }
        let column = SchemaColumnType::Int;
        for n in [i64::MIN, -1, 0, 1, i64::MAX] {
            let legacy = Value::Int(n)
                .secondary_index_key(&column)
                .unwrap_or_else(|| panic!("{n} legacy key"));
            let ordered = derive_ints(&[n], 1).unwrap();
            assert_eq!(ordered.as_bytes().len(), 8);
            assert_eq!(ordered.as_bytes()[0], legacy.as_bytes()[0] ^ 0x80);
            assert_eq!(&ordered.as_bytes()[1..], &legacy.as_bytes()[1..]);
        }
        assert!(derive_ints_typed(&[SchemaColumnType::Int], &[Value::Null], 1).is_none());
        assert!(derive_ints_typed(&[SchemaColumnType::Float], &[Value::Float(1.0)], 1).is_none());
        assert!(derive_ints_typed(&[SchemaColumnType::Bool], &[Value::Bool(true)], 1).is_none());
        assert!(derive_ints_typed(&[SchemaColumnType::Date], &[Value::Date(1)], 1).is_none());
        assert!(
            derive_ints_typed(&[SchemaColumnType::Timestamp], &[Value::Timestamp(1)], 1).is_none()
        );
    }

    fn derive_ints_typed(
        columns: &[crate::storage::page_format::ColumnType],
        values: &[Value],
        generation: u8,
    ) -> Option<Key> {
        Value::derive_secondary_index_key(columns, values, generation)
    }

    #[test]
    fn composite_key_orders_prefix_before_next_int() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        let low = derive_ints(&[1, i64::MAX], 1).expect("(1, MAX)");
        let high = derive_ints(&[2, i64::MIN], 1).expect("(2, MIN)");
        assert!(low < high);
        assert_ne!(low, high);
        assert_ne!(
            derive_ints(&[1, 0], 1).expect("(1, 0)"),
            derive_ints(&[1], 1).expect("(1)")
        );
        let mixed = derive_ints_typed(
            &[SchemaColumnType::Int, SchemaColumnType::String(8)],
            &[Value::Int(1), Value::String("a".into())],
            1,
        )
        .expect("int + string fits");
        assert_eq!(
            mixed.as_bytes(),
            &[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x61, 0x00, 0x00]
        );
    }

    #[test]
    fn composite_key_is_none_when_encoding_exceeds_32() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        assert!(derive_ints(&[1, 2, 3, 4, 5], 1).is_none());
        let too_long = "x".repeat(23);
        assert!(derive_ints_typed(
            &[SchemaColumnType::Int, SchemaColumnType::String(64)],
            &[Value::Int(1), Value::String(too_long)],
            1,
        )
        .is_none());
        let fitted = "y".repeat(22);
        let key = derive_ints_typed(
            &[SchemaColumnType::Int, SchemaColumnType::String(64)],
            &[Value::Int(1), Value::String(fitted)],
            1,
        )
        .expect("8 + 22 + terminator fits in 32");
        assert_eq!(key.len(), 32);
        assert!(derive_ints_typed(
            &[SchemaColumnType::Int, SchemaColumnType::Int],
            &[Value::Int(1)],
            1,
        )
        .is_none());
        assert!(
            derive_ints_typed(&[SchemaColumnType::Int], &[Value::String("1".into())], 1).is_none()
        );
    }

    #[test]
    fn legacy_single_column_derivation_matches_secondary_index_key() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        for n in [0i64, -1, 1, i64::MIN, i64::MAX] {
            let derived = derive_ints(&[n], 0).unwrap_or_else(|| panic!("{n} generation 0"));
            let legacy = Value::Int(n)
                .secondary_index_key(&SchemaColumnType::Int)
                .unwrap();
            assert_eq!(derived.as_bytes(), legacy.as_bytes());
        }
        let text = "a\0b";
        let derived = derive_ints_typed(
            &[SchemaColumnType::String(8)],
            &[Value::String(text.into())],
            0,
        )
        .expect("generation 0 string");
        let legacy = Value::String(text.into())
            .secondary_index_key(&SchemaColumnType::String(8))
            .unwrap();
        assert_eq!(derived.as_bytes(), legacy.as_bytes());
        assert!(derive_ints(&[1, 2], 0).is_none());
        assert!(derive_ints(&[], 0).is_none());
        assert!(derive_ints_typed(&[SchemaColumnType::Int], &[Value::Null], 0).is_none());
    }

    #[test]
    fn range_sentinel_composite_prefix_excludes_next_leading_value() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        let int_col = [SchemaColumnType::Int];
        let above_zero =
            Value::secondary_range_bounds(&int_col, &[lower_int(0, false)]).expect("> 0");
        assert!(!key_satisfies(&derive_ints(&[0], 1).unwrap(), &above_zero));
        assert!(!key_satisfies(
            &derive_ints(&[i64::MIN], 1).unwrap(),
            &above_zero
        ));
        assert!(!key_satisfies(&derive_ints(&[-1], 1).unwrap(), &above_zero));
        assert!(key_satisfies(&derive_ints(&[1], 1).unwrap(), &above_zero));

        let below_zero =
            Value::secondary_range_bounds(&int_col, &[upper_int(0, false)]).expect("< 0");
        assert!(key_satisfies(&derive_ints(&[-1], 1).unwrap(), &below_zero));
        assert!(key_satisfies(
            &derive_ints(&[i64::MIN], 1).unwrap(),
            &below_zero
        ));
        assert!(!key_satisfies(&derive_ints(&[0], 1).unwrap(), &below_zero));
        assert!(!key_satisfies(&derive_ints(&[1], 1).unwrap(), &below_zero));

        let pair = [SchemaColumnType::Int, SchemaColumnType::Int];
        let at_one = derive_ints(&[1, i64::MAX], 1).unwrap();
        let next_leading = derive_ints(&[2, i64::MIN], 1).unwrap();
        let equal_one = Value::secondary_range_bounds(&pair, &[eq_int(1)]).expect("a = 1");
        let above_one =
            Value::secondary_range_bounds(&pair, &[lower_int(1, false)]).expect("a > 1");
        let prefix_and_tail =
            Value::secondary_range_bounds(&pair, &[eq_int(1), lower_int(2, false)])
                .expect("a = 1 AND b > 2");
        assert!(key_satisfies(&at_one, &equal_one));
        assert!(!key_satisfies(&at_one, &above_one));
        assert!(key_satisfies(&next_leading, &above_one));
        assert!(!key_satisfies(&next_leading, &prefix_and_tail));
        assert!(key_satisfies(&at_one, &prefix_and_tail));

        let both = Value::secondary_range_bounds(
            &int_col,
            &[IndexColumnBound::Range {
                lower: Some(BoundMark {
                    value: Value::Int(0),
                    inclusive: false,
                }),
                upper: Some(BoundMark {
                    value: Value::Int(1),
                    inclusive: true,
                }),
            }],
        )
        .expect("0 < x <= 1");
        assert!(!key_satisfies(&derive_ints(&[0], 1).unwrap(), &both));
        assert!(key_satisfies(&derive_ints(&[1], 1).unwrap(), &both));
        assert!(!key_satisfies(&derive_ints(&[2], 1).unwrap(), &both));
        assert!(!key_satisfies(&derive_ints(&[-1], 1).unwrap(), &both));

        assert!(Value::secondary_range_bounds(&int_col, &[eq_int(1), eq_int(2)],).is_none());
        assert!(Value::secondary_range_bounds(&int_col, &[eq_int_null()]).is_none());
        assert!(Value::secondary_range_bounds(
            &int_col,
            &[IndexColumnBound::Eq(Value::String("1".into()))],
        )
        .is_none());
        assert!(Value::secondary_range_bounds(&pair, &[lower_int(1, false), eq_int(2)],).is_none());
        assert!(Value::secondary_range_bounds(
            &[SchemaColumnType::String(64)],
            &[IndexColumnBound::Eq(Value::String("z".repeat(31)))],
        )
        .is_none());
        assert!(Value::secondary_range_bounds(
            &int_col,
            &[IndexColumnBound::Range {
                lower: None,
                upper: None,
            }],
        )
        .is_none());
    }

    fn eq_int_null() -> IndexColumnBound {
        IndexColumnBound::Eq(Value::Null)
    }

    #[test]
    fn range_sentinel_four_int_columns_use_exact_key_when_full() {
        use crate::storage::page_format::ColumnType as SchemaColumnType;
        let columns = [
            SchemaColumnType::Int,
            SchemaColumnType::Int,
            SchemaColumnType::Int,
            SchemaColumnType::Int,
        ];
        let exact = derive_ints(&[1, 2, 3, 4], 1).expect("four ints");
        assert_eq!(exact.len(), 32);
        let bounds = Value::secondary_range_bounds(
            &columns,
            &[eq_int(1), eq_int(2), eq_int(3), lower_int(4, false)],
        )
        .expect("d > 4");
        let lower = bounds.lower.as_ref().expect("exclusive lower");
        assert!(!lower.inclusive);
        assert_eq!(lower.key.full_data(), exact.full_data());
        assert!(!key_satisfies(&exact, &bounds));
        let next = derive_ints(&[1, 2, 3, 5], 1).expect("next integer");
        assert!(key_satisfies(&next, &bounds));
        let other_prefix = derive_ints(&[1, 2, 4, 5], 1).expect("different third column");
        assert!(!key_satisfies(&other_prefix, &bounds));
        let top = derive_ints(&[1, 2, 3, i64::MAX], 1).expect("last column max");
        assert!(key_satisfies(&top, &bounds));
    }
}
