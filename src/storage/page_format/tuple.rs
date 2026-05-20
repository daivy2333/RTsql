//! Tuple serialization/deserialization for data page rows.
//!
//! Binary format per column:
//!   Int    = [0x01][8 bytes i64 LE]
//!   String = [0x02][2 bytes len LE][N bytes UTF-8]
//!   Null   = [0x03]
//!   Float  = [0x04][8 bytes f64 LE]
//!   Bool   = [0x05][1 byte 0/1]

use crate::storage::{Result, StorageError};
use crate::Value;
use std::io;

/// Tag bytes embedded in the serialized tuple stream.
const TAG_INT: u8 = 0x01;
const TAG_STRING: u8 = 0x02;
const TAG_NULL: u8 = 0x03;
const TAG_FLOAT: u8 = 0x04;
const TAG_BOOL: u8 = 0x05;

/// Column type descriptor used as the schema for serialization / deserialization.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnType {
    /// Signed 64-bit integer column.
    Int,
    /// Variable-length UTF-8 string column with a maximum byte length.
    String(u16),
}

/// Return the total number of bytes required to serialize the given values
/// according to `schema`.  Panics when `values.len() != schema.len()`.
pub fn compute_tuple_size(values: &[Value], schema: &[ColumnType]) -> usize {
    assert_eq!(values.len(), schema.len());
    values
        .iter()
        .map(|value| match value {
            Value::Int(_) => 1 + 8,
            Value::String(s) => 1 + 2 + s.len(),
            Value::Null => 1,
            Value::Float(_) => 1 + 8,
            Value::Bool(_) => 1 + 1,
        })
        .sum()
}

/// Serialize `values` into `buf` following the column types declared in `schema`.
///
/// Returns the number of bytes written.  Panics when the lengths of `values`
/// and `schema` differ.
pub fn serialize_tuple(values: &[Value], schema: &[ColumnType], buf: &mut [u8]) -> Result<usize> {
    assert_eq!(values.len(), schema.len());

    fn err_too_small() -> StorageError {
        StorageError::Io(io::Error::new(io::ErrorKind::WriteZero, "buffer too small"))
    }

    let mut pos = 0;
    for value in values {
        match value {
            Value::Int(n) => {
                if pos + 9 > buf.len() {
                    return Err(err_too_small());
                }
                buf[pos] = TAG_INT;
                buf[pos + 1..pos + 9].copy_from_slice(&n.to_le_bytes());
                pos += 9;
            }
            Value::String(s) => {
                let bytes = s.as_bytes();
                let len = bytes.len() as u16;
                if pos + 3 + bytes.len() > buf.len() {
                    return Err(err_too_small());
                }
                buf[pos] = TAG_STRING;
                buf[pos + 1..pos + 3].copy_from_slice(&len.to_le_bytes());
                buf[pos + 3..pos + 3 + bytes.len()].copy_from_slice(bytes);
                pos += 3 + bytes.len();
            }
            Value::Null => {
                if pos + 1 > buf.len() {
                    return Err(err_too_small());
                }
                buf[pos] = TAG_NULL;
                pos += 1;
            }
            Value::Float(f) => {
                if pos + 9 > buf.len() {
                    return Err(err_too_small());
                }
                buf[pos] = TAG_FLOAT;
                buf[pos + 1..pos + 9].copy_from_slice(&f.to_le_bytes());
                pos += 9;
            }
            Value::Bool(b) => {
                if pos + 2 > buf.len() {
                    return Err(err_too_small());
                }
                buf[pos] = TAG_BOOL;
                buf[pos + 1] = if *b { 1 } else { 0 };
                pos += 2;
            }
        }
    }
    Ok(pos)
}

/// Deserialize a tuple from `data` using the column types in `schema`.
///
/// Returns a `Vec<Value>` with one element per column, or a `StorageError`
/// when the buffer is truncated or contains malformed data.
pub fn deserialize_tuple(data: &[u8], schema: &[ColumnType]) -> Result<Vec<Value>> {
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
                    data[pos],
                    data[pos + 1],
                    data[pos + 2],
                    data[pos + 3],
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ];
                values.push(Value::Int(i64::from_le_bytes(bytes)));
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
                let s = String::from_utf8(data[pos..pos + len].to_vec())
                    .map_err(|e| StorageError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
                values.push(Value::String(s));
                pos += len;
            }
            TAG_NULL => {
                values.push(Value::Null);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_single(value: Value, col_type: ColumnType) -> Value {
        let schema = [col_type];
        let size = compute_tuple_size(&[value.clone()], &schema);
        let mut buf = vec![0u8; size];
        let written = serialize_tuple(&[value], &schema, &mut buf).unwrap();
        assert_eq!(written, size, "written bytes must match computed size");
        let result = deserialize_tuple(&buf[..written], &schema).unwrap();
        assert_eq!(result.len(), 1);
        result.into_iter().next().unwrap()
    }

    #[test]
    fn serialize_int_roundtrip() {
        let v = roundtrip_single(Value::Int(42), ColumnType::Int);
        assert_eq!(v, Value::Int(42));
    }

    #[test]
    fn serialize_string_roundtrip() {
        let v = roundtrip_single(Value::String("hello".into()), ColumnType::String(100));
        assert_eq!(v, Value::String("hello".into()));
    }

    #[test]
    fn serialize_null_roundtrip() {
        let v = roundtrip_single(Value::Null, ColumnType::Int);
        assert_eq!(v, Value::Null);
    }

    #[test]
    fn serialize_mixed() {
        let values = [Value::Int(1), Value::String("a".into()), Value::Int(-5)];
        let schema = [ColumnType::Int, ColumnType::String(50), ColumnType::Int];

        let size = compute_tuple_size(&values, &schema);
        let mut buf = vec![0u8; size];
        let written = serialize_tuple(&values, &schema, &mut buf).unwrap();
        assert_eq!(written, size);

        let result = deserialize_tuple(&buf[..written], &schema).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], Value::Int(1));
        assert_eq!(result[1], Value::String("a".into()));
        assert_eq!(result[2], Value::Int(-5));
    }

    #[test]
    fn deserialize_truncated() {
        let schema = [ColumnType::Int];
        let result = deserialize_tuple(&[0x01], &schema);
        assert!(result.is_err(), "truncated buffer must return error");
    }

    #[test]
    fn large_string() {
        let big = "x".repeat(200);
        let v = roundtrip_single(Value::String(big.clone()), ColumnType::String(300));
        assert_eq!(v, Value::String(big));
    }
}
