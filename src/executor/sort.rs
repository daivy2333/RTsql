//! Sort executor - ORDER BY clause sorting

use crate::executor::apply_projection;
use crate::executor::{ExecResult, Executor, OrderByColumn, Value};
use crate::storage::Result;
use std::cmp::Ordering;

/// Sort executor - sorts rows based on ORDER BY clause
pub struct SortExecutor {
    input: Box<dyn Executor + Send>,
    order_by: Vec<OrderByColumn>,
    columns: Vec<String>,
    /// MS10-T01 Iter001: output projection (empty = identity), applied when
    /// materializing the sorted buffer so comparisons still see the input
    /// row shape (design D10: sort keys may live outside the projection).
    projection: Vec<usize>,
    sorted_rows: Vec<Vec<Value>>,
    position: usize,
    initialized: bool,
}

impl SortExecutor {
    /// Create a new sort executor
    pub fn new(
        input: Box<dyn Executor + Send>,
        order_by: Vec<OrderByColumn>,
        columns: Vec<String>,
    ) -> Self {
        Self {
            input,
            order_by,
            columns,
            projection: Vec::new(),
            sorted_rows: Vec::new(),
            position: 0,
            initialized: false,
        }
    }

    /// MS10-T01 Iter001: narrow output rows to the given input-shape column
    /// indices (empty = identity, the `new()` default).
    pub fn with_projection(mut self, projection: Vec<usize>) -> Self {
        self.projection = projection;
        self
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

        // Sort rows using the order_by specification (on the input row shape)
        rows.sort_unstable_by(|a, b| self.compare_rows(a, b));

        // MS10-T01 Iter001: apply the output projection after sorting.
        self.sorted_rows = rows
            .into_iter()
            .map(|row| apply_projection(&self.projection, row))
            .collect();
        self.initialized = true;
        Ok(())
    }

    /// Compare two rows based on order_by specification
    fn compare_rows(&self, a: &[Value], b: &[Value]) -> Ordering {
        for order_col in &self.order_by {
            // Find the index of the column in the result columns
            let col_idx = self
                .columns
                .iter()
                .position(|c| c.to_lowercase() == order_col.column.to_lowercase());

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
        (_, Value::Null) => Ordering::Less,    // non-NULL < NULL (to end)

        // Int vs Int
        (Value::Int(x), Value::Int(y)) => x.cmp(y),

        // Float vs Float
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),

        // Int vs Float (cross-type comparison)
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal),

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
        assert_eq!(
            compare_values(&Value::Int(1), &Value::Int(2)),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&Value::Int(2), &Value::Int(1)),
            Ordering::Greater
        );
        assert_eq!(
            compare_values(&Value::Int(1), &Value::Int(1)),
            Ordering::Equal
        );
    }

    #[test]
    fn test_compare_values_float() {
        assert_eq!(
            compare_values(&Value::Float(1.0), &Value::Float(2.0)),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&Value::Float(2.5), &Value::Float(1.5)),
            Ordering::Greater
        );
    }

    #[test]
    fn test_compare_values_cross_type() {
        // Int vs Float
        assert_eq!(
            compare_values(&Value::Int(1), &Value::Float(2.0)),
            Ordering::Less
        );
        assert_eq!(
            compare_values(&Value::Float(2.0), &Value::Int(1)),
            Ordering::Greater
        );
        assert_eq!(
            compare_values(&Value::Int(2), &Value::Float(2.0)),
            Ordering::Equal
        );
    }

    #[test]
    fn test_compare_values_null() {
        // NULL < non-NULL (so NULL sorts to end in ASC)
        assert_eq!(
            compare_values(&Value::Null, &Value::Int(1)),
            Ordering::Greater
        );
        assert_eq!(compare_values(&Value::Int(1), &Value::Null), Ordering::Less);
        assert_eq!(compare_values(&Value::Null, &Value::Null), Ordering::Equal);
    }

    #[test]
    fn test_compare_values_string() {
        assert_eq!(
            compare_values(
                &Value::String("a".to_string()),
                &Value::String("b".to_string())
            ),
            Ordering::Less
        );
        assert_eq!(
            compare_values(
                &Value::String("b".to_string()),
                &Value::String("a".to_string())
            ),
            Ordering::Greater
        );
    }
}
