//! Sort executor - ORDER BY clause sorting

use crate::executor::{ExecResult, Executor, OrderByColumn, Value};
use crate::storage::Result;
use std::cmp::Ordering;

/// Sort executor - sorts rows based on ORDER BY clause
pub struct SortExecutor {
    input: Box<dyn Executor + Send>,
    order_by: Vec<OrderByColumn>,
    sorted_rows: Vec<Vec<Value>>,
    position: usize,
    initialized: bool,
}

impl SortExecutor {
    /// Create a new sort executor
    pub fn new(input: Box<dyn Executor + Send>, order_by: Vec<OrderByColumn>) -> Self {
        Self {
            input,
            order_by,
            sorted_rows: Vec::new(),
            position: 0,
            initialized: false,
        }
    }

    /// Initialize: collect all rows and sort them
    async fn initialize(&mut self) -> Result<()> {
        let mut rows = Vec::new();

        // Collect all rows from input
        while let Some(result) = self.input.next().await? {
            match result {
                ExecResult::Row(row) => rows.push(row),
                ExecResult::AffectedRows(_) | ExecResult::RowId(_) => {
                    // Skip non-row results during sorting
                }
            }
        }

        // Sort rows using the order_by specification
        rows.sort_unstable_by(|a, b| self.compare_rows(a, b));

        self.sorted_rows = rows;
        self.initialized = true;
        Ok(())
    }

    /// Compare two rows based on order_by specification
    fn compare_rows(&self, a: &[Value], b: &[Value]) -> Ordering {
        for order_col in &self.order_by {
            // For now, we assume column index matches position in order_by
            // In a more complete implementation, we'd look up column index by name
            let col_idx = self.order_by.iter().position(|c| c.column == order_col.column);

            if let Some(idx) = col_idx {
                if idx < a.len() && idx < b.len() {
                    match compare_values(&a[idx], &b[idx]) {
                        Ordering::Equal => continue,
                        ordering => {
                            if order_col.asc {
                                return ordering;
                            } else {
                                return ordering.reverse();
                            }
                        }
                    }
                }
            }
        }
        Ordering::Equal
    }
}

/// Compare two values with NULL handling
/// NULL values sort to the end regardless of ASC/DESC
fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        // NULL handling: NULL < non-NULL (so NULL sorts to end in ASC)
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Greater, // NULL > non-NULL (to end)
        (_, Value::Null) => Ordering::Less,     // non-NULL < NULL (to end)

        // Int vs Int
        (Value::Int(x), Value::Int(y)) => x.cmp(y),

        // Float vs Float
        (Value::Float(x), Value::Float(y)) => {
            x.partial_cmp(y).unwrap_or(Ordering::Equal)
        }

        // Int vs Float (cross-type comparison)
        (Value::Int(x), Value::Float(y)) => {
            (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (Value::Float(x), Value::Int(y)) => {
            x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
        }

        // String vs String
        (Value::String(x), Value::String(y)) => x.cmp(y),

        // Bool vs Bool
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),

        // Incompatible types - maintain stable order
        _ => Ordering::Equal,
    }
}

#[async_trait::async_trait]
impl Executor for SortExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // Initialize on first call
        if !self.initialized {
            self.initialize().await?;
        }

        // Return sorted rows one by one
        if self.position < self.sorted_rows.len() {
            let row = self.sorted_rows[self.position].clone();
            self.position += 1;
            Ok(Some(ExecResult::Row(row)))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_values_int() {
        assert_eq!(compare_values(&Value::Int(1), &Value::Int(2)), Ordering::Less);
        assert_eq!(compare_values(&Value::Int(2), &Value::Int(1)), Ordering::Greater);
        assert_eq!(compare_values(&Value::Int(1), &Value::Int(1)), Ordering::Equal);
    }

    #[test]
    fn test_compare_values_float() {
        assert_eq!(compare_values(&Value::Float(1.0), &Value::Float(2.0)), Ordering::Less);
        assert_eq!(compare_values(&Value::Float(2.5), &Value::Float(1.5)), Ordering::Greater);
    }

    #[test]
    fn test_compare_values_cross_type() {
        // Int vs Float
        assert_eq!(compare_values(&Value::Int(1), &Value::Float(2.0)), Ordering::Less);
        assert_eq!(compare_values(&Value::Float(2.0), &Value::Int(1)), Ordering::Greater);
        assert_eq!(compare_values(&Value::Int(2), &Value::Float(2.0)), Ordering::Equal);
    }

    #[test]
    fn test_compare_values_null() {
        // NULL < non-NULL (so NULL sorts to end in ASC)
        assert_eq!(compare_values(&Value::Null, &Value::Int(1)), Ordering::Greater);
        assert_eq!(compare_values(&Value::Int(1), &Value::Null), Ordering::Less);
        assert_eq!(compare_values(&Value::Null, &Value::Null), Ordering::Equal);
    }

    #[test]
    fn test_compare_values_string() {
        assert_eq!(compare_values(&Value::String("a".to_string()), &Value::String("b".to_string())), Ordering::Less);
        assert_eq!(compare_values(&Value::String("b".to_string()), &Value::String("a".to_string())), Ordering::Greater);
    }
}