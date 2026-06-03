//! Zero-copy SQL value view that borrows bytes from a page.
//!
//! M36: Created to eliminate String heap allocations on the read path.
//! Pairs with `deserialize_value_refs` (in `storage/page_format/tuple.rs`)
//! which produces `Vec<ValueRef<'a>>` from `&'a [u8]`. Call `to_value()`
//! to convert back to owned `Value` when needed (only allocation point
//! for `Text` variant).

use crate::executor::value::{Value, ValueError};
use std::hash::{Hash, Hasher};

/// 零拷贝 SQL 值视图，借用 'a 生命周期内的字节切片。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueRef<'a> {
    Int(i64),
    Text(&'a str),
    Null,
    Float(f64),
    Bool(bool),
}

impl<'a> Eq for ValueRef<'a> {}

impl<'a> Hash for ValueRef<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Int(n) => n.hash(state),
            Self::Text(s) => s.hash(state),
            Self::Null => {}
            Self::Float(f) => f.to_bits().hash(state),
            Self::Bool(b) => b.hash(state),
        }
    }
}

impl<'a> ValueRef<'a> {
    /// Convert to owned `Value`. The only String allocation point.
    pub fn to_value(&self) -> Value {
        match self {
            Self::Int(n) => Value::Int(*n),
            Self::Text(s) => Value::String((*s).to_string()),
            Self::Null => Value::Null,
            Self::Float(f) => Value::Float(*f),
            Self::Bool(b) => Value::Bool(*b),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_float(&self) -> Result<f64, ValueError> {
        match self {
            Self::Float(f) => Ok(*f),
            Self::Int(i) => Ok(*i as f64),
            Self::Null => Err(ValueError::NullComparison),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// Convert to bool (Int -> Bool, 0 = false, non-0 = true)
    pub fn as_bool(&self) -> Result<bool, ValueError> {
        match self {
            Self::Bool(b) => Ok(*b),
            Self::Int(i) => Ok(*i != 0),
            Self::Null => Err(ValueError::NullComparison),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// Equality comparison (cross-type: Int vs Float)
    pub fn equals(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Null, _) | (_, Self::Null) => false,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Float(b)) => (*a as f64) == *b,
            (Self::Float(a), Self::Int(b)) => *a == (*b as f64),
            _ => false,
        }
    }

    pub fn gt(&self, other: &Self) -> Result<bool, ValueError> {
        match (self, other) {
            (Self::Null, _) | (_, Self::Null) => Err(ValueError::NullComparison),
            (Self::Int(a), Self::Int(b)) => Ok(a > b),
            (Self::Text(a), Self::Text(b)) => Ok(a > b),
            (Self::Float(a), Self::Float(b)) => Ok(a > b),
            (Self::Bool(a), Self::Bool(b)) => Ok(a > b),
            (Self::Int(a), Self::Float(b)) => Ok((*a as f64) > *b),
            (Self::Float(a), Self::Int(b)) => Ok(*a > *b as f64),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn lt(&self, other: &Self) -> Result<bool, ValueError> {
        match (self, other) {
            (Self::Null, _) | (_, Self::Null) => Err(ValueError::NullComparison),
            (Self::Int(a), Self::Int(b)) => Ok(a < b),
            (Self::Text(a), Self::Text(b)) => Ok(a < b),
            (Self::Float(a), Self::Float(b)) => Ok(a < b),
            (Self::Bool(a), Self::Bool(b)) => Ok(a < b),
            (Self::Int(a), Self::Float(b)) => Ok((*a as f64) < *b),
            (Self::Float(a), Self::Int(b)) => Ok(*a < *b as f64),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn ge(&self, other: &Self) -> Result<bool, ValueError> {
        match (self, other) {
            (Self::Null, _) | (_, Self::Null) => Err(ValueError::NullComparison),
            (Self::Int(a), Self::Int(b)) => Ok(a >= b),
            (Self::Text(a), Self::Text(b)) => Ok(a >= b),
            (Self::Float(a), Self::Float(b)) => Ok(a >= b),
            (Self::Bool(a), Self::Bool(b)) => Ok(a >= b),
            (Self::Int(a), Self::Float(b)) => Ok((*a as f64) >= *b),
            (Self::Float(a), Self::Int(b)) => Ok(*a >= *b as f64),
            _ => Err(ValueError::TypeMismatch),
        }
    }

    pub fn le(&self, other: &Self) -> Result<bool, ValueError> {
        match (self, other) {
            (Self::Null, _) | (_, Self::Null) => Err(ValueError::NullComparison),
            (Self::Int(a), Self::Int(b)) => Ok(a <= b),
            (Self::Text(a), Self::Text(b)) => Ok(a <= b),
            (Self::Float(a), Self::Float(b)) => Ok(a <= b),
            (Self::Bool(a), Self::Bool(b)) => Ok(a <= b),
            (Self::Int(a), Self::Float(b)) => Ok((*a as f64) <= *b),
            (Self::Float(a), Self::Int(b)) => Ok(*a <= *b as f64),
            _ => Err(ValueError::TypeMismatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_ref_int_to_value_zero_alloc() {
        let vr = ValueRef::Int(42);
        let v = vr.to_value();
        assert_eq!(v, Value::Int(42));
    }

    #[test]
    fn value_ref_text_to_value_allocates() {
        let s = "hello world";
        let vr = ValueRef::Text(s);
        let v = vr.to_value();
        assert_eq!(v, Value::String("hello world".to_string()));
    }

    #[test]
    fn value_ref_copy_semantics() {
        let s = String::from("borrowed");
        // Borrow of s captured into Copy enum. After this, vr1 owns the
        // borrow; further immutable uses of s are fine, but we must not
        // move/drop s before vr1's last use.
        let vr1 = ValueRef::Text(s.as_str());
        let vr2 = vr1; // Copy
        assert_eq!(vr1, vr2);
        assert_eq!(vr1, ValueRef::Text("borrowed"));
        // s still alive here; no drop needed.
        let _ = s.len();
    }

    #[test]
    fn value_ref_hash_eq() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        ValueRef::Int(42).hash(&mut h1);
        ValueRef::Int(42).hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn value_ref_size_is_small() {
        use std::mem::size_of;
        assert!(
            size_of::<ValueRef>() <= 24,
            "ValueRef must be ≤ 24B, got {}",
            size_of::<ValueRef>()
        );
    }

    #[test]
    fn value_as_value_ref_int() {
        let v = Value::Int(42);
        let vr = v.as_value_ref();
        assert_eq!(vr, ValueRef::Int(42));
    }

    #[test]
    fn value_as_value_ref_text_borrows() {
        let v = Value::String("borrowed".to_string());
        let vr = v.as_value_ref();
        assert_eq!(vr, ValueRef::Text("borrowed"));
        drop(vr);
        drop(v);
    }

    #[test]
    fn value_ref_comparison_int() {
        let a = ValueRef::Int(5);
        let b = ValueRef::Int(10);
        assert!(a.lt(&b).unwrap());
        assert!(!a.gt(&b).unwrap());
        assert!(a.le(&b).unwrap());
        assert!(!a.ge(&b).unwrap());
        assert!(!a.equals(&b));
    }

    #[test]
    fn value_ref_comparison_text() {
        let a = ValueRef::Text("apple");
        let b = ValueRef::Text("banana");
        assert!(a.lt(&b).unwrap());
        assert_eq!(a, ValueRef::Text("apple"));
    }

    #[test]
    fn value_ref_comparison_null_errors() {
        let a = ValueRef::Null;
        let b = ValueRef::Int(1);
        assert!(a.gt(&b).is_err());
        assert!(!b.equals(&a)); // Null != anything, returns false not error
    }
}
