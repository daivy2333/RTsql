//! Tests for Predicate trait + Expression system
//!
//! Task 8: WHERE clause expression evaluator

use rtsql::executor::{
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, Expression, LogicalOp,
    LogicalPredicate, Predicate, Value,
};

/// Test 1: ColumnExpression evaluates to column value
#[test]
fn test_column_expression_evaluates_column_value() {
    let row = vec![Value::Int(42), Value::String("hello".to_string())];
    let expr = ColumnExpression {
        column_name: "id".to_string(),
        column_index: 0,
    };

    let result = expr.evaluate(&row).unwrap();
    assert_eq!(result, Value::Int(42));
}

/// Test 2: ConstantExpression evaluates to constant value
#[test]
fn test_constant_expression_evaluates_constant_value() {
    let row = vec![Value::Int(100)];
    let expr = ConstantExpression {
        value: Value::Int(999),
    };

    let result = expr.evaluate(&row).unwrap();
    assert_eq!(result, Value::Int(999));
}

/// Test 3: ComparisonPredicate Eq (equal) works
#[test]
fn test_comparison_predicate_eq() {
    let row = vec![Value::Int(42), Value::String("test".to_string())];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "id".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::Int(42),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Eq,
        right,
    };

    assert!(pred.evaluate(&row).unwrap());
}

/// Test 4: ComparisonPredicate Ne (not equal) works
#[test]
fn test_comparison_predicate_ne() {
    let row = vec![Value::Int(42)];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "id".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::Int(999),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Ne,
        right,
    };

    assert!(pred.evaluate(&row).unwrap());
}

/// Test 5: ComparisonPredicate Gt (greater than) works
#[test]
fn test_comparison_predicate_gt() {
    let row = vec![Value::Int(100)];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "value".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::Int(50),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Gt,
        right,
    };

    assert!(pred.evaluate(&row).unwrap());
}

/// Test 6: ComparisonPredicate Lt (less than) works
#[test]
fn test_comparison_predicate_lt() {
    let row = vec![Value::Int(10)];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "value".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::Int(50),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Lt,
        right,
    };

    assert!(pred.evaluate(&row).unwrap());
}

/// Test 7: ComparisonPredicate Ge (greater than or equal) works
#[test]
fn test_comparison_predicate_ge() {
    let row = vec![Value::Int(100)];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "value".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::Int(100),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Ge,
        right,
    };

    assert!(pred.evaluate(&row).unwrap());
}

/// Test 8: ComparisonPredicate Le (less than or equal) works
#[test]
fn test_comparison_predicate_le() {
    let row = vec![Value::Int(50)];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "value".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::Int(50),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Le,
        right,
    };

    assert!(pred.evaluate(&row).unwrap());
}

/// Test 9: LogicalPredicate And works
#[test]
fn test_logical_predicate_and() {
    let row = vec![Value::Int(50), Value::Int(100)];

    // id > 10 AND value < 200
    let pred1 = std::sync::Arc::new(ComparisonPredicate {
        left: std::sync::Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Gt,
        right: std::sync::Arc::new(ConstantExpression {
            value: Value::Int(10),
        }),
    });

    let pred2 = std::sync::Arc::new(ComparisonPredicate {
        left: std::sync::Arc::new(ColumnExpression {
            column_name: "value".to_string(),
            column_index: 1,
        }),
        op: ComparisonOp::Lt,
        right: std::sync::Arc::new(ConstantExpression {
            value: Value::Int(200),
        }),
    });

    let logical = LogicalPredicate {
        left: pred1,
        op: LogicalOp::And,
        right: pred2,
    };

    assert!(logical.evaluate(&row).unwrap());
}

/// Test 10: LogicalPredicate Or works
#[test]
fn test_logical_predicate_or() {
    let row = vec![Value::Int(5)];

    // id = 5 OR id = 10
    let pred1 = std::sync::Arc::new(ComparisonPredicate {
        left: std::sync::Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Eq,
        right: std::sync::Arc::new(ConstantExpression {
            value: Value::Int(5),
        }),
    });

    let pred2 = std::sync::Arc::new(ComparisonPredicate {
        left: std::sync::Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Eq,
        right: std::sync::Arc::new(ConstantExpression {
            value: Value::Int(10),
        }),
    });

    let logical = LogicalPredicate {
        left: pred1,
        op: LogicalOp::Or,
        right: pred2,
    };

    assert!(logical.evaluate(&row).unwrap());
}

/// Test 11: Complex nested predicate (id > 10 AND value < 100) OR (id = 5)
#[test]
fn test_complex_nested_predicate() {
    let row = vec![Value::Int(5), Value::Int(200)];

    // id > 10 AND value < 100
    let inner_and = std::sync::Arc::new(LogicalPredicate {
        left: std::sync::Arc::new(ComparisonPredicate {
            left: std::sync::Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Gt,
            right: std::sync::Arc::new(ConstantExpression {
                value: Value::Int(10),
            }),
        }),
        op: LogicalOp::And,
        right: std::sync::Arc::new(ComparisonPredicate {
            left: std::sync::Arc::new(ColumnExpression {
                column_name: "value".to_string(),
                column_index: 1,
            }),
            op: ComparisonOp::Lt,
            right: std::sync::Arc::new(ConstantExpression {
                value: Value::Int(100),
            }),
        }),
    });

    // id = 5
    let id_eq_5 = std::sync::Arc::new(ComparisonPredicate {
        left: std::sync::Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Eq,
        right: std::sync::Arc::new(ConstantExpression {
            value: Value::Int(5),
        }),
    });

    // (inner_and) OR (id = 5)
    let outer_or = LogicalPredicate {
        left: inner_and,
        op: LogicalOp::Or,
        right: id_eq_5,
    };

    // id=5 satisfies, so should be true
    assert!(outer_or.evaluate(&row).unwrap());
}

/// Test 12: String comparison works
#[test]
fn test_string_comparison() {
    let row = vec![Value::String("hello".to_string())];

    let left = std::sync::Arc::new(ColumnExpression {
        column_name: "name".to_string(),
        column_index: 0,
    });
    let right = std::sync::Arc::new(ConstantExpression {
        value: Value::String("world".to_string()),
    });

    let pred = ComparisonPredicate {
        left,
        op: ComparisonOp::Lt,
        right,
    };

    assert!(pred.evaluate(&row).unwrap()); // "hello" < "world"
}
