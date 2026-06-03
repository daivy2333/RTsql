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
        assert!(size_of::<ValueRef>() <= 24, "ValueRef must be ≤ 24B, got {}", size_of::<ValueRef>());
    }
}
