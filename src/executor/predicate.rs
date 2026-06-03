//! Predicate and Expression traits for WHERE clause evaluation
//!
//! Task 8: WHERE clause expression evaluator

use crate::executor::{Value, ValueRef};
use std::fmt::Debug;
use std::sync::Arc;

/// Predicate trait - evaluates a row against a boolean condition
pub trait Predicate: Send + Sync + Debug {
    /// Evaluate the predicate against a row
    /// Returns true if the row satisfies the predicate
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;

    /// Recursively inject correlated parameter values into ParameterExpression nodes
    fn inject_parameters(&self, _params: &[(String, Value)]) {}
}

/// Reference to a Predicate (Arc<dyn Predicate>)
pub type PredicateRef = Arc<dyn Predicate>;

/// Expression trait - evaluates to a value
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

/// Reference to an Expression (Arc<dyn Expression>)
pub type ExpressionRef = Arc<dyn Expression>;

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComparisonOp {
    /// Equal (=)
    Eq,
    /// Not equal (!=)
    Ne,
    /// Greater than (>)
    Gt,
    /// Less than (<)
    Lt,
    /// Greater than or equal (>=)
    Ge,
    /// Less than or equal (<=)
    Le,
}

/// Comparison predicate (e.g., id = 5, value > 10)
#[derive(Debug)]
pub struct ComparisonPredicate {
    pub left: ExpressionRef,
    pub op: ComparisonOp,
    pub right: ExpressionRef,
}

impl Predicate for ComparisonPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let left_val = self.left.evaluate(row)?;
        let right_val = self.right.evaluate(row)?;

        // Handle NULL comparisons
        if left_val.is_null() || right_val.is_null() {
            // SQL semantics: NULL comparisons return false (except IS NULL/IS NOT NULL)
            return Ok(false);
        }

        let result = match self.op {
            ComparisonOp::Eq => Ok(left_val.equals(&right_val)),
            ComparisonOp::Ne => Ok(!left_val.equals(&right_val)),
            ComparisonOp::Gt => left_val.gt(&right_val),
            ComparisonOp::Lt => left_val.lt(&right_val),
            ComparisonOp::Ge => left_val.ge(&right_val),
            ComparisonOp::Le => left_val.le(&right_val),
        };

        result.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        for (name, value) in params {
            self.left.set_parameter_value(name, value);
            self.right.set_parameter_value(name, value);
        }
    }
}

/// Logical operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogicalOp {
    /// AND
    And,
    /// OR
    Or,
}

/// Logical predicate (e.g., id > 10 AND value < 100)
#[derive(Debug)]
pub struct LogicalPredicate {
    pub left: PredicateRef,
    pub op: LogicalOp,
    pub right: PredicateRef,
}

impl Predicate for LogicalPredicate {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        match self.op {
            LogicalOp::And => {
                // Short-circuit evaluation for AND
                let left_result = self.left.evaluate(row)?;
                if !left_result {
                    return Ok(false);
                }
                self.right.evaluate(row)
            }
            LogicalOp::Or => {
                // Short-circuit evaluation for OR
                let left_result = self.left.evaluate(row)?;
                if left_result {
                    return Ok(true);
                }
                self.right.evaluate(row)
            }
        }
    }

    fn inject_parameters(&self, params: &[(String, Value)]) {
        self.left.inject_parameters(params);
        self.right.inject_parameters(params);
    }
}

/// Column expression - evaluates to a column value from the row
#[derive(Debug)]
pub struct ColumnExpression {
    pub column_name: String,
    pub column_index: usize,
}

impl Expression for ColumnExpression {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        row.get(self.column_index).cloned().ok_or_else(|| {
            format!(
                "Column index {} out of bounds (row has {} columns)",
                self.column_index,
                row.len()
            )
            .into()
        })
    }
}

/// Constant expression - evaluates to a constant value
#[derive(Debug)]
pub struct ConstantExpression {
    pub value: Value,
}

impl Expression for ConstantExpression {
    fn evaluate(&self, _row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.value.clone())
    }
}

/// Parameter expression - a placeholder for correlated outer column value
/// The value is injected at execution time via set_parameter_value
pub struct ParameterExpression {
    pub param_name: String,
    value: std::sync::Mutex<Value>,
}

impl ParameterExpression {
    pub fn new(param_name: String) -> Self {
        Self {
            param_name,
            value: std::sync::Mutex::new(Value::Null),
        }
    }

    pub fn set_value(&self, value: Value) {
        *self.value.lock().unwrap() = value;
    }
}

impl Debug for ParameterExpression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterExpression")
            .field("param_name", &self.param_name)
            .finish()
    }
}

impl Expression for ParameterExpression {
    fn evaluate(&self, _row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.value.lock().unwrap().clone())
    }

    fn set_parameter_value(&self, param_name: &str, value: &Value) -> bool {
        if self.param_name == param_name {
            *self.value.lock().unwrap() = value.clone();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_expression() {
        let row = vec![Value::Int(42), Value::String("test".to_string())];
        let expr = ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        };
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(42));
    }

    #[test]
    fn test_constant_expression() {
        let row = vec![];
        let expr = ConstantExpression {
            value: Value::Int(999),
        };
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(999));
    }

    #[test]
    fn test_comparison_eq() {
        let row = vec![Value::Int(42)];
        let pred = ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Eq,
            right: Arc::new(ConstantExpression {
                value: Value::Int(42),
            }),
        };
        assert!(pred.evaluate(&row).unwrap());
    }

    #[test]
    fn test_logical_and() {
        let row = vec![Value::Int(50)];
        let pred1 = Arc::new(ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Gt,
            right: Arc::new(ConstantExpression {
                value: Value::Int(10),
            }),
        });
        let pred2 = Arc::new(ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Lt,
            right: Arc::new(ConstantExpression {
                value: Value::Int(100),
            }),
        });
        let logical = LogicalPredicate {
            left: pred1,
            op: LogicalOp::And,
            right: pred2,
        };
        assert!(logical.evaluate(&row).unwrap());
    }

    #[test]
    fn test_parameter_expression() {
        let row = vec![];
        let expr = ParameterExpression::new("emp.dept".to_string());
        expr.set_value(Value::Int(10));
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(10));
        expr.set_value(Value::Int(20));
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(20));
        // name mismatch → no change
        assert!(!expr.set_parameter_value("wrong.name", &Value::Int(99)));
        assert_eq!(expr.evaluate(&row).unwrap(), Value::Int(20));
    }
}
