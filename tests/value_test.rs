//! Tests for Value type extensions (Float/Bool + comparison methods)

use rtsql::executor::Value;
use rtsql::executor::ValueError;

#[test]
fn test_value_equals_int() {
    let v1 = Value::Int(42);
    let v2 = Value::Int(42);
    assert!(v1.equals(&v2));

    let v3 = Value::Int(100);
    assert!(!v1.equals(&v3));
}

#[test]
fn test_value_equals_string() {
    let v1 = Value::String("hello".to_string());
    let v2 = Value::String("hello".to_string());
    assert!(v1.equals(&v2));

    let v3 = Value::String("world".to_string());
    assert!(!v1.equals(&v3));
}

#[test]
fn test_value_equals_float() {
    let v1 = Value::Float(3.14);
    let v2 = Value::Float(3.14);
    assert!(v1.equals(&v2));

    let v3 = Value::Float(2.71);
    assert!(!v1.equals(&v3));
}

#[test]
fn test_value_equals_bool() {
    let v1 = Value::Bool(true);
    let v2 = Value::Bool(true);
    assert!(v1.equals(&v2));

    let v3 = Value::Bool(false);
    assert!(!v1.equals(&v3));
}

#[test]
fn test_value_equals_null() {
    let v1 = Value::Null;
    let v2 = Value::Null;
    assert!(v1.equals(&v2));

    let v3 = Value::Int(42);
    assert!(!v1.equals(&v3));
}

#[test]
fn test_value_equals_cross_type_int_float() {
    // Int vs Float: 允许隐式转换
    let v1 = Value::Int(42);
    let v2 = Value::Float(42.0);
    assert!(v1.equals(&v2));
    assert!(v2.equals(&v1));

    let v3 = Value::Int(100);
    let v4 = Value::Float(100.0);
    assert!(v3.equals(&v4));
}

#[test]
fn test_value_gt_int() {
    let v1 = Value::Int(100);
    let v2 = Value::Int(42);
    assert!(v1.gt(&v2).unwrap());
    assert!(!v2.gt(&v1).unwrap());
}

#[test]
fn test_value_lt_float() {
    let v1 = Value::Float(2.71);
    let v2 = Value::Float(3.14);
    assert!(v1.lt(&v2).unwrap());
    assert!(!v2.lt(&v1).unwrap());
}

#[test]
fn test_value_ge() {
    let v1 = Value::Int(42);
    let v2 = Value::Int(42);
    assert!(v1.ge(&v2).unwrap());

    let v3 = Value::Int(100);
    assert!(v3.ge(&v1).unwrap());
    assert!(!v1.ge(&v3).unwrap());
}

#[test]
fn test_value_le() {
    let v1 = Value::Float(3.14);
    let v2 = Value::Float(3.14);
    assert!(v1.le(&v2).unwrap());

    let v3 = Value::Float(2.71);
    assert!(v3.le(&v1).unwrap());
    assert!(!v1.le(&v3).unwrap());
}

#[test]
fn test_as_float() {
    // Float 转 Float
    let v1 = Value::Float(3.14);
    assert_eq!(v1.as_float().unwrap(), 3.14);

    // Int 转 Float（隐式转换）
    let v2 = Value::Int(42);
    assert_eq!(v2.as_float().unwrap(), 42.0);
}

#[test]
fn test_as_float_error() {
    // String 不能转 Float
    let v1 = Value::String("hello".to_string());
    assert!(matches!(v1.as_float(), Err(ValueError::TypeMismatch)));

    // Null 不能转 Float
    let v2 = Value::Null;
    assert!(matches!(v2.as_float(), Err(ValueError::NullComparison)));
}

#[test]
fn test_as_bool() {
    // Bool 转 Bool
    let v1 = Value::Bool(true);
    assert!(v1.as_bool().unwrap());

    let v2 = Value::Bool(false);
    assert!(!v2.as_bool().unwrap());

    // Int 转 Bool（隐式转换）
    let v3 = Value::Int(1);
    assert!(v3.as_bool().unwrap());

    let v4 = Value::Int(0);
    assert!(!v4.as_bool().unwrap());
}

#[test]
fn test_as_bool_error() {
    // String 不能转 Bool
    let v1 = Value::String("hello".to_string());
    assert!(matches!(v1.as_bool(), Err(ValueError::TypeMismatch)));

    // Float 不能转 Bool
    let v2 = Value::Float(3.14);
    assert!(matches!(v2.as_bool(), Err(ValueError::TypeMismatch)));

    // Null 不能转 Bool
    let v3 = Value::Null;
    assert!(matches!(v3.as_bool(), Err(ValueError::NullComparison)));
}

#[test]
fn test_comparison_cross_type_int_float() {
    // Int vs Float 比较
    let v1 = Value::Int(42);
    let v2 = Value::Float(42.0);
    assert!(v1.equals(&v2));

    let v3 = Value::Int(100);
    let v4 = Value::Float(50.0);
    assert!(v3.gt(&v4).unwrap());
    assert!(v4.lt(&v3).unwrap());
}

#[test]
fn test_null_comparison_error() {
    // Null 不能参与比较操作
    let v1 = Value::Null;
    let v2 = Value::Int(42);

    assert!(matches!(v1.gt(&v2), Err(ValueError::NullComparison)));
    assert!(matches!(v1.lt(&v2), Err(ValueError::NullComparison)));
    assert!(matches!(v1.ge(&v2), Err(ValueError::NullComparison)));
    assert!(matches!(v1.le(&v2), Err(ValueError::NullComparison)));
}

#[test]
fn test_type_mismatch_comparison() {
    // 不兼容类型比较
    let v1 = Value::String("hello".to_string());
    let v2 = Value::Int(42);

    assert!(matches!(v1.gt(&v2), Err(ValueError::TypeMismatch)));
    assert!(matches!(v1.lt(&v2), Err(ValueError::TypeMismatch)));
}

#[test]
fn test_display_float() {
    let v = Value::Float(3.14);
    assert_eq!(format!("{}", v), "3.14");
}

#[test]
fn test_display_bool() {
    let v1 = Value::Bool(true);
    assert_eq!(format!("{}", v1), "true");

    let v2 = Value::Bool(false);
    assert_eq!(format!("{}", v2), "false");
}
