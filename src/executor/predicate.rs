//! Predicate and Expression traits for WHERE clause evaluation
//!
//! Task 8: WHERE clause expression evaluator

use crate::executor::Value;
use std::sync::Arc;

/// Predicate trait - evaluates a row against a boolean condition
pub trait Predicate: Send + Sync {
    /// Evaluate the predicate against a row
    /// Returns true if the row satisfies the predicate
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
}

/// Reference to a Predicate (Arc<dyn Predicate>)
pub type PredicateRef = Arc<dyn Predicate>;

/// Expression trait - evaluates to a value
pub trait Expression: Send + Sync {
    /// Evaluate the expression against a row
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
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
}

/// Column expression - evaluates to a column value from the row
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
pub struct ConstantExpression {
    pub value: Value,
}

impl Expression for ConstantExpression {
    fn evaluate(&self, _row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.value.clone())
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
}
