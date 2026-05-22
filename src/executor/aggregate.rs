//! Aggregate function types and state accumulators (M15: GROUP BY / aggregation)
//!
//! Supports SQL standard aggregate functions: COUNT(*), COUNT(col), SUM, AVG, MIN, MAX.

use crate::executor::executor_trait::Executor;
use crate::executor::plan::AggregateNode;
use crate::executor::result::ExecResult;
use crate::executor::Value;
use crate::storage;
use async_trait::async_trait;
use std::collections::HashMap;

/// Aggregate function descriptor (from SQL parse)
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateFunc {
    /// COUNT(*) — counts all rows including NULLs
    CountStar,
    /// COUNT(col) — counts non-NULL values in column
    Count(String),
    /// SUM(col) — sum of non-NULL values
    Sum(String),
    /// AVG(col) — average of non-NULL values
    Avg(String),
    /// MIN(col) — minimum non-NULL value
    Min(String),
    /// MAX(col) — maximum non-NULL value
    Max(String),
}

/// Running accumulator state for an aggregate function
#[derive(Debug, Clone)]
pub enum AggregateState {
    /// COUNT(*) accumulator: counts all rows
    CountStar(i64),
    /// COUNT(col) accumulator: counts non-NULL values
    Count { count: i64 },
    /// SUM accumulator: running sum and count of non-NULL values
    Sum { sum: Option<Value>, count: i64 },
    /// AVG accumulator: running sum and count of non-NULL values
    Avg { sum: Option<Value>, count: i64 },
    /// MIN accumulator: current minimum
    Min(Option<Value>),
    /// MAX accumulator: current maximum
    Max(Option<Value>),
}

impl AggregateFunc {
    /// Returns the output column name for this aggregate function
    pub fn result_column_name(&self) -> String {
        match self {
            AggregateFunc::CountStar => "count_star".to_string(),
            AggregateFunc::Count(col) => format!("count_{}", col),
            AggregateFunc::Sum(col) => format!("sum_{}", col),
            AggregateFunc::Avg(col) => format!("avg_{}", col),
            AggregateFunc::Min(col) => format!("min_{}", col),
            AggregateFunc::Max(col) => format!("max_{}", col),
        }
    }
}

impl AggregateState {
    /// Creates a new accumulator for the given aggregate function
    pub fn new(func: &AggregateFunc) -> Self {
        match func {
            AggregateFunc::CountStar => AggregateState::CountStar(0),
            AggregateFunc::Count(_) => AggregateState::Count { count: 0 },
            AggregateFunc::Sum(_) => AggregateState::Sum {
                sum: None,
                count: 0,
            },
            AggregateFunc::Avg(_) => AggregateState::Avg {
                sum: None,
                count: 0,
            },
            AggregateFunc::Min(_) => AggregateState::Min(None),
            AggregateFunc::Max(_) => AggregateState::Max(None),
        }
    }

    /// Updates the accumulator with a value from the current row.
    ///
    /// SQL standard NULL handling:
    /// - CountStar: counts all rows (ignores value entirely)
    /// - Count/Sum/Avg/Min/Max: skip NULL values
    pub fn update(&mut self, value: &Value) {
        match self {
            AggregateState::CountStar(count) => {
                *count += 1;
            }
            AggregateState::Count { count } => {
                if !value.is_null() {
                    *count += 1;
                }
            }
            AggregateState::Sum { sum, count } => {
                if !value.is_null() {
                    *sum = Some(match sum.take() {
                        Some(prev) => prev.add(value),
                        None => value.clone(),
                    });
                    *count += 1;
                }
            }
            AggregateState::Avg { sum, count } => {
                if !value.is_null() {
                    *sum = Some(match sum.take() {
                        Some(prev) => prev.add(value),
                        None => value.clone(),
                    });
                    *count += 1;
                }
            }
            AggregateState::Min(current) => {
                if !value.is_null() {
                    *current = Some(match current.take() {
                        Some(cur) => {
                            if value.lt_agg(&cur) {
                                value.clone()
                            } else {
                                cur
                            }
                        }
                        None => value.clone(),
                    });
                }
            }
            AggregateState::Max(current) => {
                if !value.is_null() {
                    *current = Some(match current.take() {
                        Some(cur) => {
                            if cur.lt_agg(value) {
                                value.clone()
                            } else {
                                cur
                            }
                        }
                        None => value.clone(),
                    });
                }
            }
        }
    }

    /// Returns the final aggregate result.
    ///
    /// Empty set semantics:
    /// - CountStar -> 0, Count -> 0
    /// - Sum -> Null, Avg -> Null
    /// - Min -> Null, Max -> Null
    pub fn finalize(&self) -> Value {
        match self {
            AggregateState::CountStar(count) => Value::Int(*count),
            AggregateState::Count { count } => Value::Int(*count),
            AggregateState::Sum { sum, count: _ } => match sum {
                Some(v) => v.clone(),
                None => Value::Null,
            },
            AggregateState::Avg { sum, count } => match sum {
                Some(v) if *count > 0 => v.div(&Value::Int(*count)),
                _ => Value::Null,
            },
            AggregateState::Min(opt) => match opt {
                Some(v) => v.clone(),
                None => Value::Null,
            },
            AggregateState::Max(opt) => match opt {
                Some(v) => v.clone(),
                None => Value::Null,
            },
        }
    }
}

/// Hash-based aggregate executor for GROUP BY + aggregate functions
pub struct AggregateExecutor {
    input: Box<dyn Executor + Send>,
    group_by: Vec<String>,
    aggregates: Vec<AggregateFunc>,
    output_columns: Vec<String>,
    column_indices: HashMap<String, usize>,
    /// Grouped results: group key → aggregate states
    groups: HashMap<Vec<Value>, Vec<AggregateState>>,
    /// Single group state (when no GROUP BY)
    single_group: Option<Vec<AggregateState>>,
    /// Whether all input has been consumed
    has_consumed_input: bool,
    /// Output rows (materialized after consuming input)
    output_rows: Option<Vec<Vec<Value>>>,
    /// Current position in output rows
    output_index: usize,
}

impl AggregateExecutor {
    pub fn new(input: Box<dyn Executor + Send>, node: AggregateNode) -> Self {
        Self {
            input,
            group_by: node.group_by,
            aggregates: node.aggregates,
            output_columns: node.output_columns,
            column_indices: node.column_indices,
            groups: HashMap::new(),
            single_group: None,
            has_consumed_input: false,
            output_rows: None,
            output_index: 0,
        }
    }

    /// Consume all input rows and accumulate aggregate states
    async fn consume_input(&mut self) -> storage::Result<()> {
        let is_no_group_by = self.group_by.is_empty();

        if is_no_group_by {
            let states: Vec<AggregateState> = self
                .aggregates
                .iter()
                .map(|f| AggregateState::new(f))
                .collect();
            self.single_group = Some(states);
        }

        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(row)) => {
                    if is_no_group_by {
                        let states = self.single_group.as_mut().unwrap();
                        for (i, func) in self.aggregates.iter().enumerate() {
                            let value = Self::extract_value(&row, func, &self.column_indices);
                            states[i].update(&value);
                        }
                    } else {
                        let group_key = self.extract_group_key(&row);
                        let states = self.groups.entry(group_key).or_insert_with(|| {
                            self.aggregates.iter().map(|f| AggregateState::new(f)).collect()
                        });
                        for (i, func) in self.aggregates.iter().enumerate() {
                            let value = Self::extract_value(&row, func, &self.column_indices);
                            states[i].update(&value);
                        }
                    }
                }
                Some(_) => {} // Skip non-row results
                None => break,
            }
        }

        self.has_consumed_input = true;
        Ok(())
    }

    /// Extract value from row for aggregate function
    fn extract_value(row: &[Value], func: &AggregateFunc, column_indices: &HashMap<String, usize>) -> Value {
        match func {
            AggregateFunc::CountStar => Value::Int(1),
            AggregateFunc::Count(col)
            | AggregateFunc::Sum(col)
            | AggregateFunc::Avg(col)
            | AggregateFunc::Min(col)
            | AggregateFunc::Max(col) => {
                match column_indices.get(&col.to_lowercase()) {
                    Some(&idx) => row.get(idx).cloned().unwrap_or(Value::Null),
                    None => Value::Null,
                }
            }
        }
    }

    /// Extract group key from row
    fn extract_group_key(&self, row: &[Value]) -> Vec<Value> {
        self.group_by
            .iter()
            .map(|col| {
                match self.column_indices.get(&col.to_lowercase()) {
                    Some(&idx) => row.get(idx).cloned().unwrap_or(Value::Null),
                    None => Value::Null,
                }
            })
            .collect()
    }

    /// Build output rows from accumulated aggregate states
    fn build_output_rows(&mut self) {
        let mut rows = Vec::new();

        if self.group_by.is_empty() {
            // No GROUP BY: return single row
            if let Some(states) = &self.single_group {
                let mut row = Vec::new();
                for state in states {
                    row.push(state.finalize());
                }
                rows.push(row);
            } else {
                // Empty input: return single row (COUNT→0, others→NULL)
                let row: Vec<Value> = self
                    .aggregates
                    .iter()
                    .map(|f| {
                        let state = AggregateState::new(f);
                        state.finalize()
                    })
                    .collect();
                rows.push(row);
            }
        } else {
            // With GROUP BY: one row per group
            for (group_key, states) in &self.groups {
                let mut row = group_key.clone();
                for state in states {
                    row.push(state.finalize());
                }
                rows.push(row);
            }
        }

        self.output_rows = Some(rows);
    }
}

#[async_trait]
impl Executor for AggregateExecutor {
    async fn next(&mut self) -> storage::Result<Option<ExecResult>> {
        if !self.has_consumed_input {
            self.consume_input().await?;
            self.build_output_rows();
        }

        match &self.output_rows {
            Some(rows) => {
                if self.output_index < rows.len() {
                    let row = rows[self.output_index].clone();
                    self.output_index += 1;
                    Ok(Some(ExecResult::Row(row)))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}
