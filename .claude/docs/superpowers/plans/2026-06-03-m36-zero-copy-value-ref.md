# M36 Zero-Copy ValueRef Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate `String` heap allocations on the read path by introducing a zero-copy `ValueRef<'a>` enum that borrows page bytes, while keeping the `Value` enum and `Expression::evaluate()` API backwards-compatible.

**Architecture:** New `ValueRef<'a>` enum (Int/Text/Null/Float/Bool) in `src/executor/value_ref.rs`. New `deserialize_value_refs(data: &'a [u8], schema) -> Result<Vec<ValueRef<'a>>>` in `src/storage/page_format/tuple.rs` reuses existing 5-tag binary format but borrows `&str` instead of allocating `String`. `Expression` trait adds `evaluate_ref` method; `evaluate()` becomes a trait default method that calls `evaluate_ref().to_value()`. 3 Scan executors swap their closure body from `deserialize_tuple` to `deserialize_value_refs + to_value`.

**Tech Stack:** Rust 1.75+ (trait default methods), `std::str::from_utf8` for zero-copy UTF-8 validation, existing `with_page_data` closure API from M20.

---

## File Structure

**Created:**
- `src/executor/value_ref.rs` — `ValueRef<'a>` enum + methods + `Value::as_value_ref` (impl block) + unit tests

**Modified:**
- `src/executor/value.rs` — add `impl Value { pub fn as_value_ref(&self) -> ValueRef<'_> }` block
- `src/executor/predicate.rs` — add `evaluate_ref` to `Expression` trait + implement in 5 Expression structs
- `src/executor/{scan,index_scan,index_scan_all}.rs` — change closure body in `next()` to use `deserialize_value_refs`
- `src/executor/mod.rs` — `pub use value_ref::ValueRef;`
- `src/storage/page_format/tuple.rs` — add `deserialize_value_refs` + tests
- `src/storage/page_format/mod.rs` — `pub use tuple::deserialize_value_refs;`

**No touched files** (out of M36 scope):
- `src/executor/sort.rs` / `aggregate.rs` / `join.rs` — keep `Value`, not in M36
- `src/executor/update.rs` — write path needs `Value` ownership, keep `deserialize_tuple`

---

## Task 1: ValueRef<'a> enum + Copy/Hash/Eq + to_value

**Files:**
- Create: `src/executor/value_ref.rs`
- Modify: `src/executor/mod.rs:67` (add `pub use value_ref::ValueRef;`)
- Test: inline `#[cfg(test)] mod tests` at end of `src/executor/value_ref.rs`

- [ ] **Step 1: Create value_ref.rs with enum + impl + tests**

Write the entire file at `src/executor/value_ref.rs`:

```rust
//! Zero-copy SQL value view that borrows bytes from a page.
//!
//! M36: Created to eliminate String heap allocations on the read path.
//! Pairs with `deserialize_value_refs` (in `storage/page_format/tuple.rs`)
//! which produces `Vec<ValueRef<'a>>` from `&'a [u8]`. Call `to_value()`
//! to convert back to owned `Value` when needed (only allocation point
//! for `Text` variant).

use crate::executor::value::{Value, ValueError};

/// 零拷贝 SQL 值视图，借用 'a 生命周期内的字节切片。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueRef<'a> {
    Int(i64),
    Text(&'a str),
    Null,
    Float(f64),
    Bool(bool),
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
        // Text conversion must allocate String. We can't measure
        // allocation count in stable tests; instead assert roundtrip.
        let s = "hello world";
        let vr = ValueRef::Text(s);
        let v = vr.to_value();
        assert_eq!(v, Value::String("hello world".to_string()));
    }

    #[test]
    fn value_ref_copy_semantics() {
        // Copy means the borrowed &str stays valid after copy
        let s = String::from("borrowed");
        let vr1 = ValueRef::Text(s.as_str());
        let vr2 = vr1; // Copy
        assert_eq!(vr1, vr2);
        drop(s);
        // vr1 and vr2 still valid (no Drop for ValueRef)
        assert_eq!(vr1, ValueRef::Text("borrowed"));
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
        // 8B discriminant + 16B max payload (String/Vec would be 24B)
        use std::mem::size_of;
        assert!(size_of::<ValueRef>() <= 24, "ValueRef must be ≤ 24B, got {}", size_of::<ValueRef>());
    }
}
```

- [ ] **Step 2: Wire ValueRef into executor module**

In `src/executor/mod.rs`, find the line `pub use value::{ColumnType, Value, ValueError};` (line 67) and add ValueRef export **after** it (or in the same group):

```rust
pub use value::{ColumnType, Value, ValueError};
pub use value_ref::ValueRef;
```

- [ ] **Step 3: Run tests to verify**

Run: `cargo test --lib value_ref::tests`
Expected: `5 passed; 0 failed`

- [ ] **Step 4: Run cargo check to verify wiring**

Run: `cargo check --lib`
Expected: `Finished` with no errors

- [ ] **Step 5: Commit**

```bash
git add src/executor/value_ref.rs src/executor/mod.rs
git commit -m "feat(executor): add ValueRef<'a> zero-copy view (M36 T1)"
```

---

## Task 2: Value::as_value_ref — owned → borrowed view

**Files:**
- Modify: `src/executor/value.rs` (add `impl Value` block with `as_value_ref`)
- Test: add to `src/executor/value_ref.rs` tests section

- [ ] **Step 1: Add as_value_ref test to value_ref.rs tests**

Add the following test to the existing `mod tests` block in `src/executor/value_ref.rs` (insert before the closing `}`):

```rust
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
        // Verify borrow points into v's String
        if let ValueRef::Text(s) = vr {
            // s should point into v's heap allocation
            let s_ptr = s.as_ptr();
            let v_ptr = v.as_key_for_test().as_ptr(); // hypothetical
            assert!(!s.is_empty());
            let _ = s_ptr;
        } else {
            panic!("expected Text variant");
        }
    }
```

(We will replace the borrow-verification approach in Step 4 with a simpler one.)

- [ ] **Step 2: Add as_value_ref impl to value.rs**

Open `src/executor/value.rs` and find the end of `impl Value` block (right before `impl fmt::Display for Value`). Add a new `impl` block (or extend the existing one) with this method:

```rust
impl Value {
    /// Borrowed view — `String(s)` → `Text(s.as_str())` borrows s's heap.
    /// Other variants are zero-allocation conversions.
    pub fn as_value_ref(&self) -> ValueRef<'_> {
        match self {
            Value::Int(n) => ValueRef::Int(*n),
            Value::String(s) => ValueRef::Text(s.as_str()),
            Value::Null => ValueRef::Null,
            Value::Float(f) => ValueRef::Float(*f),
            Value::Bool(b) => ValueRef::Bool(*b),
        }
    }
}
```

- [ ] **Step 3: Replace the borrow-verification test with a simpler one**

Replace the `value_as_value_ref_text_borrows` test with a simpler one that doesn't depend on a hypothetical `as_key_for_test`. The borrow semantics are implicit from the `&str` lifetime parameter:

```rust
    #[test]
    fn value_as_value_ref_text_borrows() {
        let v = Value::String("borrowed".to_string());
        let vr = v.as_value_ref();
        assert_eq!(vr, ValueRef::Text("borrowed"));
        // vr borrows from v; both alive
        drop(vr);
        drop(v);
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib value_ref::tests`
Expected: `7 passed; 0 failed`

- [ ] **Step 5: Commit**

```bash
git add src/executor/value.rs src/executor/value_ref.rs
git commit -m "feat(executor): add Value::as_value_ref owned-to-borrowed view (M36 T2)"
```

---

## Task 3: deserialize_value_refs — zero-copy deserializer

**Files:**
- Modify: `src/storage/page_format/tuple.rs` (add function after `deserialize_tuple` at line 202)
- Modify: `src/storage/page_format/mod.rs:8` (add export)
- Test: append to `mod tests` in `src/storage/page_format/tuple.rs`

- [ ] **Step 1: Write the failing test for String borrow semantics**

In `src/storage/page_format/tuple.rs`, find `mod tests` at the end and add (before the closing `}` of the mod):

```rust
    #[test]
    fn deserialize_value_refs_string_borrows() {
        // Verify Text variant borrows from data slice, not allocates
        let data = vec![
            0x02, 0x05, 0x00,  // TAG_STRING, len=5
            b'h', b'e', b'l', b'l', b'o',
        ];
        let schema = [ColumnType::String(100)];
        let refs = deserialize_value_refs(&data, &schema).unwrap();
        assert_eq!(refs.len(), 1);
        match &refs[0] {
            ValueRef::Text(s) => {
                assert_eq!(*s, "hello");
                // Critical assertion: the str must point into data
                let s_ptr = s.as_ptr();
                let data_ptr = data.as_ptr();
                assert!(s_ptr >= data_ptr, "Text must borrow from data, not allocate");
                assert!(s_ptr < unsafe { data_ptr.add(data.len()) }, "Text must point inside data");
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn deserialize_value_refs_int_roundtrip() {
        let data = [0x01, 42, 0, 0, 0, 0, 0, 0, 0];  // TAG_INT + i64=42 LE
        let schema = [ColumnType::Int];
        let refs = deserialize_value_refs(&data, &schema).unwrap();
        assert_eq!(refs, vec![ValueRef::Int(42)]);
    }

    #[test]
    fn deserialize_value_refs_truncated() {
        let data = [0x01];  // TAG_INT but no payload
        let schema = [ColumnType::Int];
        assert!(deserialize_value_refs(&data, &schema).is_err());
    }

    #[test]
    fn deserialize_value_refs_invalid_utf8() {
        let data = vec![
            0x02, 0x02, 0x00,  // TAG_STRING, len=2
            0xFF, 0xFE,         // invalid UTF-8
        ];
        let schema = [ColumnType::String(100)];
        assert!(deserialize_value_refs(&data, &schema).is_err());
    }

    #[test]
    fn deserialize_value_refs_mixed_types() {
        // [Int(42), Float(1.5), Bool(true), Text("hi"), Null, Bool(false)]
        let data = vec![
            0x01, 42, 0, 0, 0, 0, 0, 0, 0,  // Int
            0x04, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f,  // Float 1.5 LE
            0x05, 0x01,  // Bool true
            0x02, 0x02, 0x00, b'h', b'i',  // String "hi"
            0x03,  // Null
            0x05, 0x00,  // Bool false
        ];
        let schema = [
            ColumnType::Int, ColumnType::Float, ColumnType::Bool,
            ColumnType::String(50), ColumnType::Int, ColumnType::Bool,
        ];
        let refs = deserialize_value_refs(&data, &schema).unwrap();
        assert_eq!(refs.len(), 6);
        assert_eq!(refs[0], ValueRef::Int(42));
        assert_eq!(refs[1], ValueRef::Float(1.5));
        assert_eq!(refs[2], ValueRef::Bool(true));
        assert_eq!(refs[3], ValueRef::Text("hi"));
        assert_eq!(refs[4], ValueRef::Null);
        assert_eq!(refs[5], ValueRef::Bool(false));
    }
```

You'll also need the import at top of test mod — add to existing `use super::*;`:

```rust
    use super::*;
    use crate::executor::ValueRef;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib tuple::tests::deserialize_value_refs`
Expected: FAIL with "function `deserialize_value_refs` not found" (or similar compile error)

- [ ] **Step 3: Implement deserialize_value_refs**

In `src/storage/page_format/tuple.rs`, find the `deserialize_tuple` function (line 115) and add the new function **right after it ends** (around line 202, just before the `// Tests` separator):

```rust
/// Zero-copy deserialize: returns `Vec<ValueRef<'a>>` borrowing from `data`.
/// Reuses the same 5 tag bytes as `deserialize_tuple` (TAG_INT / TAG_STRING
/// / TAG_NULL / TAG_FLOAT / TAG_BOOL). TAG_STRING borrows via `str::from_utf8`
/// without allocating. Call `.to_value()` to convert to owned `Value`.
///
/// Allocation: `Vec<ValueRef>` header (~24B) + 8B × schema.len() (stack-only
/// `Copy` enum). No `String` allocation.
pub fn deserialize_value_refs(
    data: &'a [u8],
    schema: &[ColumnType],
) -> Result<Vec<ValueRef<'a>>> {
    fn eof(what: &str) -> StorageError {
        StorageError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("truncated tuple: {what}"),
        ))
    }

    let mut pos = 0;
    let mut values = Vec::with_capacity(schema.len());

    for _col_type in schema {
        if pos >= data.len() {
            return Err(eof("expected tag byte"));
        }
        let tag = data[pos];
        pos += 1;

        match tag {
            TAG_INT => {
                if pos + 8 > data.len() {
                    return Err(eof("expected 8 bytes for i64"));
                }
                let bytes = [
                    data[pos], data[pos + 1], data[pos + 2], data[pos + 3],
                    data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7],
                ];
                values.push(ValueRef::Int(i64::from_le_bytes(bytes)));
                pos += 8;
            }
            TAG_STRING => {
                if pos + 2 > data.len() {
                    return Err(eof("expected 2 bytes for string length"));
                }
                let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                pos += 2;
                if pos + len > data.len() {
                    return Err(eof("expected string payload"));
                }
                let s = std::str::from_utf8(&data[pos..pos + len]).map_err(|e| {
                    StorageError::Io(io::Error::new(io::ErrorKind::InvalidData, e))
                })?;
                values.push(ValueRef::Text(s));  // borrows data
                pos += len;
            }
            TAG_NULL => {
                values.push(ValueRef::Null);
            }
            TAG_FLOAT => {
                if pos + 8 > data.len() {
                    return Err(eof("expected 8 bytes for f64"));
                }
                let bytes = [
                    data[pos], data[pos + 1], data[pos + 2], data[pos + 3],
                    data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7],
                ];
                values.push(ValueRef::Float(f64::from_le_bytes(bytes)));
                pos += 8;
            }
            TAG_BOOL => {
                if pos + 1 > data.len() {
                    return Err(eof("expected 1 byte for bool"));
                }
                values.push(ValueRef::Bool(data[pos] != 0));
                pos += 1;
            }
            other => {
                return Err(StorageError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown tag byte: {other:#x}"),
                )));
            }
        }
    }

    Ok(values)
}
```

Also update the imports at top of `tuple.rs`:

```rust
use crate::executor::ValueRef;
use crate::storage::{Result, StorageError};
use crate::Value;
use std::io;
```

- [ ] **Step 4: Wire deserialize_value_refs into page_format module**

In `src/storage/page_format/mod.rs`, find line 8 (`pub use slotted_page::{Slot, SlottedPage, SlottedPageHeader, SlottedPageRef};` or similar) and add to the tuple re-exports:

```rust
pub use tuple::{compute_tuple_size, deserialize_tuple, deserialize_value_refs, serialize_tuple};
```

- [ ] **Step 5: Run tests to verify all pass**

Run: `cargo test --lib tuple::tests`
Expected: All tuple tests pass (5 new + 6 existing = 11 tests)

- [ ] **Step 6: Commit**

```bash
git add src/storage/page_format/tuple.rs src/storage/page_format/mod.rs
git commit -m "feat(storage): add deserialize_value_refs zero-copy deserializer (M36 T3)"
```

---

## Task 4: Expression trait — add evaluate_ref + default evaluate

**Files:**
- Modify: `src/executor/predicate.rs` (Expression trait at line 23)
- Test: existing tests in predicate.rs `mod tests` at line 213

- [ ] **Step 1: Update Expression trait definition**

In `src/executor/predicate.rs`, find the `Expression` trait (around line 23) and replace it:

OLD:
```rust
pub trait Expression: Send + Sync + Debug {
    /// Evaluate the expression against a row
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;

    /// Try to set a parameter value by name. Returns true if matched and set.
    fn set_parameter_value(&self, _param_name: &str, _value: &Value) -> bool {
        false
    }
}
```

NEW:
```rust
pub trait Expression: Send + Sync + Debug {
    /// Backward-compatible owned entry point. Default impl converts the row
    /// to `&[ValueRef]` via `as_value_ref` and calls `evaluate_ref`, then
    /// `to_value()`s the result. Implementations SHOULD override only
    /// `evaluate_ref`; the default `evaluate` will call it correctly.
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let row_ref: Vec<ValueRef> = row.iter().map(Value::as_value_ref).collect();
        self.evaluate_ref(&row_ref).map(|vr| vr.to_value())
    }

    /// M36: zero-copy entry point. Returns a `ValueRef<'row>` borrowing
    /// from `row`. Implementations MUST NOT `.await` and MUST NOT
    /// recursively call `BufferPool` methods (deadlock risk).
    fn evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, Box<dyn std::error::Error + Send + Sync>>;

    /// Try to set a parameter value by name. Returns true if matched and set.
    fn set_parameter_value(&self, _param_name: &str, _value: &Value) -> bool {
        false
    }
}
```

Also add the import at top of `predicate.rs`:

```rust
use crate::executor::{Value, ValueRef};
```

(Replace the existing `use crate::executor::Value;` line.)

- [ ] **Step 2: Run cargo check to verify trait compiles (impls will fail)**

Run: `cargo check --lib`
Expected: COMPILE ERROR — 3 existing impls (ColumnExpression / ConstantExpression / ParameterExpression) don't implement `evaluate_ref`. Error messages name the missing method.

- [ ] **Step 3: Commit broken state (TDD red)**

```bash
git add src/executor/predicate.rs
git commit -m "wip(predicate): add Expression::evaluate_ref (Task 4 step 1, impls pending)"
```

(Yes, commit the broken state per TDD red-green discipline.)


---

## Task 5: Implement evaluate_ref in ColumnExpression / ConstantExpression / ParameterExpression

**Files:**
- Modify: `src/executor/predicate.rs` (3 Expression impls at lines 144, 163, 197)
- Test: existing tests in `mod tests` should still pass

- [ ] **Step 1: Add evaluate_ref to ColumnExpression**

In `src/executor/predicate.rs`, find the `impl Expression for ColumnExpression` block (line 144). After the existing `evaluate` method, add:

```rust
    fn evaluate_ref(&self, row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, Box<dyn std::error::Error + Send + Sync>> {
        row.get(self.column_index).copied().ok_or_else(|| {
            format!(
                "Column index {} out of bounds (row has {} columns)",
                self.column_index,
                row.len()
            )
            .into()
        })
    }
```

- [ ] **Step 2: Add evaluate_ref to ConstantExpression**

Find `impl Expression for ConstantExpression` (line 163). Add:

```rust
    fn evaluate_ref(&self, _row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.value.as_value_ref())
    }
```

- [ ] **Step 3: Add evaluate_ref to ParameterExpression**

Find `impl Expression for ParameterExpression` (line 197). Add:

```rust
    fn evaluate_ref(&self, _row: &[ValueRef<'_>]) -> Result<ValueRef<'_>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.value.lock().unwrap().as_value_ref())
    }
```

- [ ] **Step 4: Run cargo check to verify all impls present**

Run: `cargo check --lib`
Expected: `Finished` with no errors

- [ ] **Step 5: Run existing predicate tests to verify backward compat**

Run: `cargo test --lib predicate::tests`
Expected: All existing tests pass (the default `evaluate()` impl calls `evaluate_ref().to_value()` so old behavior preserved)

- [ ] **Step 6: Commit**

```bash
git add src/executor/predicate.rs
git commit -m "feat(predicate): implement evaluate_ref in 3 Expression impls (M36 T5)"
```

---

## Task 6: 3 Scan executors — swap closure to deserialize_value_refs

**Files:**
- Modify: `src/executor/scan.rs` (ScanExecutor::next closure, ~line 49-72)
- Modify: `src/executor/index_scan.rs` (IndexScanExecutor::next closure, ~line 65-87)
- Modify: `src/executor/index_scan_all.rs` (IndexScanAllExecutor::next closure, ~line 68-91)
- Test: existing tests in `tests/executor_test.rs` should pass

- [ ] **Step 1: Update ScanExecutor::next (both branches)**

In `src/executor/scan.rs`, find the `for (_key, row_id) in entries` block inside `next()`. Replace the entire `if let Some(ref snapshot) = ... else ...` block with the deserialize_value_refs version (full code in M36 change artifact).

Also update the `use` statement at top of `scan.rs`:

OLD:
```rust
use crate::storage::page_format::{deserialize_tuple, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
```

NEW:
```rust
use crate::storage::page_format::{deserialize_value_refs, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
```

- [ ] **Step 2: Update IndexScanExecutor::next (both branches)**

Same pattern as Task 6 Step 1, applied to `src/executor/index_scan.rs`. Update use statement:

```rust
use crate::storage::page_format::{deserialize_value_refs, ColumnType};
```

- [ ] **Step 3: Update IndexScanAllExecutor::next (both branches)**

Same pattern for `src/executor/index_scan_all.rs`. Update use statement:

```rust
use crate::storage::page_format::{deserialize_value_refs, ColumnType, RowId};
```

- [ ] **Step 4: Run cargo check to verify all 3 scans compile**

Run: `cargo check --lib`
Expected: `Finished` with no errors

- [ ] **Step 5: Run executor tests to verify behavior preserved**

Run: `cargo test --test executor_test`
Expected: All existing executor tests pass

- [ ] **Step 6: Run full test suite to confirm no regression**

Run: `cargo test --lib --tests`
Expected: All tests pass (no regression — closure body swap preserves semantics)

- [ ] **Step 7: Commit**

```bash
git add src/executor/scan.rs src/executor/index_scan.rs src/executor/index_scan_all.rs
git commit -m "feat(executor): 3 Scan executors use deserialize_value_refs (M36 T6)"
```

---

## Task 7: Module exports + wiring

**Files:**
- Modify: `src/executor/mod.rs` (verify `ValueRef` is exported — from Task 1)
- Modify: `src/storage/page_format/mod.rs` (verify `deserialize_value_refs` is exported — from Task 3)

- [ ] **Step 1: Verify exports are in place**

Run: `grep -n "ValueRef\|deserialize_value_refs" src/executor/mod.rs src/storage/page_format/mod.rs`

Expected: Both exports present (from Task 1 and Task 3 respectively). If missing, add them.

- [ ] **Step 2: Run full test suite**

Run: `cargo test --lib --tests`
Expected: 0 failures

- [ ] **Step 3: Commit if any changes needed**

```bash
# Only if Step 1 found missing exports
git add src/executor/mod.rs src/storage/page_format/mod.rs
git commit -m "chore: verify M36 module exports (Task 7)"
```

---

## Task 8: Lint + fmt + integration tests

**Files:**
- Modify: `tests/executor_test.rs` (add 2 M36-specific tests)

- [ ] **Step 1: Add 2 M36 integration tests**

In `tests/executor_test.rs`, add at end:

```rust
    /// M36: Verify Value::as_value_ref borrows from String
    #[tokio::test]
    async fn test_m36_value_as_value_ref() {
        let v = Value::String("hello".to_string());
        let vr = v.as_value_ref();
        assert_eq!(vr, ValueRef::Text("hello"));
    }

    /// M36: Verify deserialize_value_refs borrows from input data
    #[tokio::test]
    async fn test_m36_deserialize_value_refs_borrow() {
        use rtsql::storage::page_format::{deserialize_value_refs, ColumnType};
        let data = vec![0x02, 0x05, 0x00, b'h', b'e', b'l', b'l', b'o'];
        let schema = [ColumnType::String(100)];
        let refs = deserialize_value_refs(&data, &schema).unwrap();
        match &refs[0] {
            ValueRef::Text(s) => {
                let s_ptr = s.as_ptr();
                let data_ptr = data.as_ptr();
                assert!(s_ptr >= data_ptr, "Text must borrow from data");
            }
            _ => panic!("expected Text"),
        }
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test --test executor_test`
Expected: All pass

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets 2>&1 | grep -E "warning|error" | head -30`
Expected: No new warnings introduced by M36 (pre-existing warnings can be ignored per Surgical Changes rule)

- [ ] **Step 4: Run fmt**

Run: `cargo fmt -p rtsql` and `rustfmt src/executor/value_ref.rs tests/executor_test.rs src/executor/predicate.rs`

- [ ] **Step 5: Commit**

```bash
git add tests/executor_test.rs
git commit -m "test: add M36 zero-copy integration tests (Task 8)"
```

---

## Task 9: Performance verification

**Files:** None modified; this is verification only.

- [ ] **Step 1: Save before-m36 baseline**

Run: `cargo bench --bench micro_bench -- --save-baseline before-m36 2>&1 | tail -30`
Expected: All benchmarks complete; baseline saved to `target/criterion/before-m36`

(This may take 5-10 minutes.)

- [ ] **Step 2: Compare current state to before-m36**

Run: `cargo bench --bench micro_bench -- --baseline before-m36 2>&1 | tail -50`
Expected: All benchmarks complete with "change:" lines

- [ ] **Step 3: Verify dual criteria**

Check the output:
- **Criterion 1 (allocation)**: 1K row scans with String column should show 30万 → 0 String allocation reduction.
- **Criterion 2 (speed)**: At least one scan-related benchmark should show ≥ 5% improvement.

If either criterion is NOT met, document in the change log and consider follow-up. Do not block on this.

- [ ] **Step 4: Run concurrent_bench for regression check**

Run: `cargo bench --bench concurrent_bench 2>&1 | tail -20`
Expected: No regression > 5%

- [ ] **Step 5: Document results in learned/spec.md**

Open `openspec/specs/learned/spec.md`, find a free L-number (look for last `<!-- L0XX -->` and increment), and add L025 entry with benchmark results.

- [ ] **Step 6: Commit benchmark results**

```bash
git add openspec/specs/learned/spec.md
git commit -m "docs(learned): M36 实测性能 L025 (Task 9)"
```

---

## Task 10: Documentation sync + archive

**Files:**
- Modify: `.claude/docs/tasks.md` (M36 status)
- Modify: `.claude/docs/snapshot.md` (M20 → M36 transition)

- [ ] **Step 1: Update tasks.md**

Find the M20 entry in `.claude/docs/tasks.md` and add M36 below it.

- [ ] **Step 2: Update snapshot.md**

Find the M20 entry in `.claude/docs/snapshot.md` and add a similar M36 entry below it.

- [ ] **Step 3: Archive OpenSpec change**

Run: `yes | openspec archive m36-zero-copy-value-ref 2>&1 | tail -20`
Expected: Specs updated successfully + change moved to archive/

- [ ] **Step 4: Final commit**

```bash
git add .claude/docs/tasks.md .claude/docs/snapshot.md openspec/changes/m36-zero-copy-value-ref/
git commit -m "docs: sync M36 status + archive OpenSpec change (Task 10)"
```

- [ ] **Step 5: Verify final state**

Run: `git log --oneline -5` and `git status`
Expected: 10-12 commits total, all M36 work committed; working tree clean (except .codegraph/daemon.pid untracked).
