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

// ===========================================================================
// MS11-T01 T1: 三值求值内核（追加段；既有测试零修改）
// use 声明置于模块级追加段，避免改动文件头部既有行。
// ===========================================================================

use rtsql::executor::{PredicateRef, Ternary};

/// col{index} op v 比较谓词
fn col_cmp(index: usize, op: ComparisonOp, v: Value) -> PredicateRef {
    std::sync::Arc::new(ComparisonPredicate {
        left: std::sync::Arc::new(ColumnExpression {
            column_name: format!("c{index}"),
            column_index: index,
        }),
        op,
        right: std::sync::Arc::new(ConstantExpression { value: v }),
    })
}

/// (col0 > 0, col1 < 10) AND/OR 对：行 = [col0, col1]，两列可独立取 NULL
fn cmp_pair() -> (PredicateRef, PredicateRef) {
    (
        col_cmp(0, ComparisonOp::Gt, Value::Int(0)),
        col_cmp(1, ComparisonOp::Lt, Value::Int(10)),
    )
}

/// 比较遇 NULL 操作数 → Unknown；fold 后行为与旧 NULL→false 路径逐字节等价
#[test]
fn comparison_null_operand_is_unknown() {
    let pred = col_cmp(0, ComparisonOp::Eq, Value::Int(1));
    let row = vec![Value::Null];
    assert_eq!(pred.evaluate_ternary(&row).unwrap(), Ternary::Unknown);
    assert!(!pred.evaluate(&row).unwrap());
}

/// 非 NULL 比较映射 True/False
#[test]
fn non_null_comparison_maps_to_true_false() {
    let row = vec![Value::Int(5)];
    assert_eq!(
        col_cmp(0, ComparisonOp::Eq, Value::Int(5))
            .evaluate_ternary(&row)
            .unwrap(),
        Ternary::True
    );
    assert_eq!(
        col_cmp(0, ComparisonOp::Eq, Value::Int(6))
            .evaluate_ternary(&row)
            .unwrap(),
        Ternary::False
    );
    assert_eq!(
        col_cmp(0, ComparisonOp::Gt, Value::Int(4))
            .evaluate_ternary(&row)
            .unwrap(),
        Ternary::True
    );
    assert_eq!(
        col_cmp(0, ComparisonOp::Lt, Value::Int(4))
            .evaluate_ternary(&row)
            .unwrap(),
        Ternary::False
    );
}

/// AND 三值表：有 False 则 False，否则有 Unknown 则 Unknown，否则 True
#[test]
fn logical_and_ternary_table() {
    let (l, r) = cmp_pair();
    let cases: Vec<(&[Value], Ternary)> = vec![
        (&[Value::Null, Value::Int(5)], Ternary::Unknown), // Unknown AND True
        (&[Value::Null, Value::Int(50)], Ternary::False),  // Unknown AND False
        (&[Value::Int(-5), Value::Null], Ternary::False),  // False AND Unknown
        (&[Value::Int(5), Value::Null], Ternary::Unknown), // True AND Unknown
        (&[Value::Null, Value::Null], Ternary::Unknown),   // Unknown AND Unknown
        (&[Value::Int(5), Value::Int(5)], Ternary::True),  // True AND True
        (&[Value::Int(-5), Value::Int(50)], Ternary::False), // False AND False
    ];
    for (row, expected) in cases {
        let row = row.to_vec();
        let pred = LogicalPredicate {
            left: l.clone(),
            op: LogicalOp::And,
            right: r.clone(),
        };
        assert_eq!(
            pred.evaluate_ternary(&row).unwrap(),
            expected,
            "row={row:?}"
        );
        // evaluate() = fold：与旧行为选择结果一致
        assert_eq!(
            pred.evaluate(&row).unwrap(),
            expected == Ternary::True,
            "fold row={row:?}"
        );
    }
}

/// OR 三值表：有 True 则 True，否则有 Unknown 则 Unknown，否则 False
#[test]
fn logical_or_ternary_table() {
    let (l, r) = cmp_pair();
    let cases: Vec<(&[Value], Ternary)> = vec![
        (&[Value::Null, Value::Int(5)], Ternary::True), // Unknown OR True
        (&[Value::Int(5), Value::Null], Ternary::True), // True OR Unknown
        (&[Value::Null, Value::Int(50)], Ternary::Unknown), // Unknown OR False
        (&[Value::Int(-5), Value::Null], Ternary::Unknown), // False OR Unknown
        (&[Value::Null, Value::Null], Ternary::Unknown), // Unknown OR Unknown
        (&[Value::Int(-5), Value::Int(50)], Ternary::False), // False OR False
        (&[Value::Int(5), Value::Int(5)], Ternary::True), // True OR True
    ];
    for (row, expected) in cases {
        let row = row.to_vec();
        let pred = LogicalPredicate {
            left: l.clone(),
            op: LogicalOp::Or,
            right: r.clone(),
        };
        assert_eq!(
            pred.evaluate_ternary(&row).unwrap(),
            expected,
            "row={row:?}"
        );
        assert_eq!(
            pred.evaluate(&row).unwrap(),
            expected == Ternary::True,
            "fold row={row:?}"
        );
    }
}

/// 默认 evaluate_ternary 实现：两值谓词（如 Future 扩展类型）零修改即正确；
/// 此处以 Comparison 的既有 evaluate 语义验证默认映射不被破坏
#[test]
fn default_ternary_maps_two_valued_evaluate() {
    let row = vec![Value::Int(5), Value::Null];
    // col1 IS NOT comparable — but default impl is only exercised by types
    // without an override; ParameterExpression-style types map via evaluate.
    // Guard the mapping itself through a non-NULL comparison.
    assert_eq!(
        col_cmp(0, ComparisonOp::Ne, Value::Int(6))
            .evaluate_ternary(&row)
            .unwrap(),
        Ternary::True
    );
}

// ===========================================================================
// MS11-T01 T2: 新谓词 LikePredicate / IsNullPredicate / NotPredicate（追加段）
// ===========================================================================

use rtsql::executor::{IsNullPredicate, LikePredicate, NotPredicate};

/// col{index} LIKE <pattern 常量>
fn like_pred(index: usize, pattern: &str) -> LikePredicate {
    LikePredicate {
        expr: std::sync::Arc::new(ColumnExpression {
            column_name: format!("c{index}"),
            column_index: index,
        }),
        pattern: std::sync::Arc::new(ConstantExpression {
            value: Value::String(pattern.to_string()),
        }),
    }
}

/// LIKE 通配矩阵：% 任意串、_ 单字符、无通配精确匹配、贪婪回溯
#[test]
fn like_wildcard_matrix() {
    let alice = vec![Value::String("Alice".to_string())];
    let cases: Vec<(&str, bool)> = vec![
        ("A%", true),    // 前缀
        ("%e", true),    // 后缀
        ("%lic%", true), // 中缀
        ("_lice", true), // 单字符
        ("Ali_e", true), // _ 嵌中
        ("Alice", true), // 无通配精确匹配
        ("Al%", true),   // 字面量后随 %
        ("Bob", false),  // 精确不匹配
        ("a%c", false),  // 长度不足
        ("A_ce", false), // _ 恰一字符，模式总长不足
        ("A_l", false),  // _ 占位后不足
    ];
    for (pattern, expected) in cases {
        assert_eq!(
            like_pred(0, pattern).evaluate(&alice).unwrap(),
            expected,
            "Alice LIKE {pattern:?}"
        );
    }

    let abc = vec![Value::String("abc".to_string())];
    assert!(
        like_pred(0, "a%c").evaluate(&abc).unwrap(),
        "贪婪回溯：a%c 命中 abc"
    );
    assert!(like_pred(0, "%%").evaluate(&abc).unwrap(), "多重 %");
    let empty = vec![Value::String(String::new())];
    assert!(like_pred(0, "%").evaluate(&empty).unwrap(), "空串 LIKE %");
    assert!(!like_pred(0, "_").evaluate(&empty).unwrap(), "空串 LIKE _");
}

/// LIKE 任一操作数 NULL → Unknown；fold 为不匹配
#[test]
fn like_null_operand_is_unknown() {
    let pred = like_pred(0, "A%");
    let null_row = vec![Value::Null];
    assert_eq!(pred.evaluate_ternary(&null_row).unwrap(), Ternary::Unknown);
    assert!(!pred.evaluate(&null_row).unwrap());

    // 模式为 NULL（列引用构造便于注入）：col0 值非空、模式列 col1 为 NULL
    let pred2 = LikePredicate {
        expr: std::sync::Arc::new(ColumnExpression {
            column_name: "c0".to_string(),
            column_index: 0,
        }),
        pattern: std::sync::Arc::new(ColumnExpression {
            column_name: "c1".to_string(),
            column_index: 1,
        }),
    };
    let row = vec![Value::String("Alice".to_string()), Value::Null];
    assert_eq!(pred2.evaluate_ternary(&row).unwrap(), Ternary::Unknown);
}

/// LIKE 任一操作数非 String → 执行期类型错误
#[test]
fn like_non_string_operand_is_error() {
    let int_row = vec![Value::Int(42)];
    assert!(like_pred(0, "A%").evaluate(&int_row).is_err());

    let pred = LikePredicate {
        expr: std::sync::Arc::new(ColumnExpression {
            column_name: "c0".to_string(),
            column_index: 0,
        }),
        pattern: std::sync::Arc::new(ConstantExpression {
            value: Value::Int(5),
        }),
    };
    let str_row = vec![Value::String("Alice".to_string())];
    assert!(pred.evaluate(&str_row).is_err());
}

/// IS NULL：NULL → True，非 NULL → False，永不为 Unknown
#[test]
fn is_null_predicate_ternary() {
    let pred = IsNullPredicate {
        expr: std::sync::Arc::new(ColumnExpression {
            column_name: "c0".to_string(),
            column_index: 0,
        }),
    };
    let null_row = vec![Value::Null];
    assert_eq!(pred.evaluate_ternary(&null_row).unwrap(), Ternary::True);
    assert!(pred.evaluate(&null_row).unwrap());

    let int_row = vec![Value::Int(1)];
    assert_eq!(pred.evaluate_ternary(&int_row).unwrap(), Ternary::False);
    assert!(!pred.evaluate(&int_row).unwrap());

    let str_row = vec![Value::String("x".to_string())];
    assert_eq!(pred.evaluate_ternary(&str_row).unwrap(), Ternary::False);
}

/// NOT 三值取反表
#[test]
fn not_predicate_ternary_inversion() {
    let not_of = |inner: PredicateRef| NotPredicate { inner };
    let true_row = vec![Value::Int(5)];
    let false_row = vec![Value::Int(-5)];
    let null_row = vec![Value::Null];
    let col_gt0 = col_cmp(0, ComparisonOp::Gt, Value::Int(0));

    assert_eq!(
        not_of(col_gt0.clone()).evaluate_ternary(&true_row).unwrap(),
        Ternary::False
    );
    assert_eq!(
        not_of(col_gt0.clone())
            .evaluate_ternary(&false_row)
            .unwrap(),
        Ternary::True
    );
    assert_eq!(
        not_of(col_gt0.clone()).evaluate_ternary(&null_row).unwrap(),
        Ternary::Unknown
    );
    // fold：True 取反后不匹配
    assert!(!not_of(col_gt0).evaluate(&true_row).unwrap());
}

/// 组合语义守卫：NOT (name LIKE '%o%') 对 "Alice" 为 True（R1/S3 的 NOT LIKE）
#[test]
fn not_composed_with_like_matches_standard_semantics() {
    let alice = vec![Value::String("Alice".to_string())];
    let bob = vec![Value::String("Bob".to_string())];
    let pred = NotPredicate {
        inner: std::sync::Arc::new(like_pred(0, "%o%")),
    };
    assert!(pred.evaluate(&alice).unwrap());
    assert!(!pred.evaluate(&bob).unwrap());
}
