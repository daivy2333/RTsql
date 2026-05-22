//! Aggregate function types and state accumulators (M15: GROUP BY / aggregation)
//!
//! Supports SQL standard aggregate functions: COUNT(*), COUNT(col), SUM, AVG, MIN, MAX.

use crate::executor::Value;

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
