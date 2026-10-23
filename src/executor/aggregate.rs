//! Aggregate function types and state accumulators (M15: GROUP BY / aggregation)
//!
//! Supports SQL standard aggregate functions: COUNT(*), COUNT(col), SUM, AVG, MIN, MAX,
//! plus the MS25 D7 extensions: COUNT/SUM/AVG(DISTINCT col) (real de-duplicating
//! accumulators) and GROUP_CONCAT(col [, sep]).

use crate::executor::executor_trait::Executor;
use crate::executor::query_budget::{
    row_bytes, string_bytes, unlimited_budget, value_bytes, SharedBudget,
};
use crate::executor::result::ExecResult;
use crate::executor::{ExpressionRef, Value};
use crate::storage;
use async_trait::async_trait;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

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
    /// COUNT(DISTINCT col) — MS25 D7: counts distinct non-NULL values
    CountDistinct(String),
    /// SUM(DISTINCT col) — MS25 D7: sums distinct non-NULL values
    SumDistinct(String),
    /// AVG(DISTINCT col) — MS25 D7: averages distinct non-NULL values
    AvgDistinct(String),
    /// GROUP_CONCAT(col [, sep]) — MS25 D7: joins non-NULL values in input row
    /// order; `separator` is the literal separator (defaults to `,`).
    GroupConcat {
        col: String,
        separator: Option<String>,
    },
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
    /// COUNT(DISTINCT col) accumulator: distinct non-NULL values seen
    CountDistinct(HashSet<Value>),
    /// SUM(DISTINCT col) accumulator: distinct values + running sum
    SumDistinct {
        seen: HashSet<Value>,
        sum: Option<Value>,
    },
    /// AVG(DISTINCT col) accumulator: distinct values + running sum and count
    AvgDistinct {
        seen: HashSet<Value>,
        sum: Option<Value>,
        count: i64,
    },
    /// GROUP_CONCAT accumulator: joined parts in input row order
    GroupConcat {
        parts: Vec<String>,
        separator: String,
    },
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
            AggregateFunc::CountDistinct(col) => format!("count_distinct_{}", col),
            AggregateFunc::SumDistinct(col) => format!("sum_distinct_{}", col),
            AggregateFunc::AvgDistinct(col) => format!("avg_distinct_{}", col),
            AggregateFunc::GroupConcat { col, .. } => format!("group_concat_{}", col),
        }
    }

    /// The column this aggregate reads, or `None` for COUNT(*).
    pub(crate) fn column(&self) -> Option<&str> {
        match self {
            AggregateFunc::CountStar => None,
            AggregateFunc::Count(col)
            | AggregateFunc::Sum(col)
            | AggregateFunc::Avg(col)
            | AggregateFunc::Min(col)
            | AggregateFunc::Max(col)
            | AggregateFunc::CountDistinct(col)
            | AggregateFunc::SumDistinct(col)
            | AggregateFunc::AvgDistinct(col)
            | AggregateFunc::GroupConcat { col, .. } => Some(col),
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
            AggregateFunc::CountDistinct(_) => AggregateState::CountDistinct(HashSet::new()),
            AggregateFunc::SumDistinct(_) => AggregateState::SumDistinct {
                seen: HashSet::new(),
                sum: None,
            },
            AggregateFunc::AvgDistinct(_) => AggregateState::AvgDistinct {
                seen: HashSet::new(),
                sum: None,
                count: 0,
            },
            AggregateFunc::GroupConcat { separator, .. } => AggregateState::GroupConcat {
                parts: Vec::new(),
                separator: separator.clone().unwrap_or_else(|| ",".to_string()),
            },
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
            // MS25 D7: DISTINCT accumulators skip NULL and fold duplicates via
            // `Value` Eq/Hash (NULL would never enter the set anyway).
            AggregateState::CountDistinct(seen) => {
                if !value.is_null() {
                    seen.insert(value.clone());
                }
            }
            AggregateState::SumDistinct { seen, sum } => {
                if !value.is_null() && seen.insert(value.clone()) {
                    *sum = Some(match sum.take() {
                        Some(prev) => prev.add(value),
                        None => value.clone(),
                    });
                }
            }
            AggregateState::AvgDistinct { seen, sum, count } => {
                if !value.is_null() && seen.insert(value.clone()) {
                    *sum = Some(match sum.take() {
                        Some(prev) => prev.add(value),
                        None => value.clone(),
                    });
                    *count += 1;
                }
            }
            AggregateState::GroupConcat { parts, .. } => {
                if !value.is_null() {
                    parts.push(concat_text(value));
                }
            }
        }
    }

    /// Returns the final aggregate result.
    ///
    /// Empty set semantics:
    /// - CountStar -> 0, Count -> 0, CountDistinct -> 0
    /// - Sum -> Null, Avg -> Null, SumDistinct -> Null, AvgDistinct -> Null
    /// - Min -> Null, Max -> Null
    /// - GroupConcat -> Null (SQLite alignment: no non-NULL part)
    pub fn finalize(&self) -> Value {
        match self {
            AggregateState::CountStar(count) => Value::Int(*count),
            AggregateState::Count { count } => Value::Int(*count),
            AggregateState::Sum { sum, count: _ } => match sum {
                Some(v) => v.clone(),
                None => Value::Null,
            },
            AggregateState::Avg { sum, count } => match sum {
                Some(v) if *count > 0 => {
                    // AVG should always produce a float result
                    let sum_float = v.as_float().unwrap_or(0.0);
                    Value::Float(sum_float / *count as f64)
                }
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
            AggregateState::CountDistinct(seen) => Value::Int(seen.len() as i64),
            AggregateState::SumDistinct { sum, .. } => match sum {
                Some(v) => v.clone(),
                None => Value::Null,
            },
            AggregateState::AvgDistinct { sum, count, .. } => match sum {
                Some(v) if *count > 0 => {
                    let sum_float = v.as_float().unwrap_or(0.0);
                    Value::Float(sum_float / *count as f64)
                }
                _ => Value::Null,
            },
            AggregateState::GroupConcat { parts, separator } => {
                if parts.is_empty() {
                    Value::Null
                } else {
                    Value::String(parts.join(separator))
                }
            }
        }
    }
}

/// MS32 Iter003（design D5）：单个累加器状态的驻留字节——携带 `Value` /
/// 字符串的状态计（SUM/AVG 的和值、MIN/MAX 当前值、DISTINCT 集合、
/// GROUP_CONCAT 拼接段与分隔符）；纯数值累加器恒 0 不计。
fn state_value_bytes(state: &AggregateState) -> u64 {
    match state {
        AggregateState::Sum { sum, .. } | AggregateState::Avg { sum, .. } => {
            sum.as_ref().map_or(0, value_bytes)
        }
        AggregateState::Min(current) | AggregateState::Max(current) => {
            current.as_ref().map_or(0, value_bytes)
        }
        AggregateState::CountDistinct(seen) => seen.iter().map(value_bytes).sum(),
        AggregateState::SumDistinct { seen, sum }
        | AggregateState::AvgDistinct { seen, sum, .. } => {
            seen.iter().map(value_bytes).sum::<u64>() + sum.as_ref().map_or(0, value_bytes)
        }
        AggregateState::GroupConcat { parts, separator } => {
            parts.iter().map(|p| string_bytes(p)).sum::<u64>() + string_bytes(separator)
        }
        AggregateState::CountStar(_) | AggregateState::Count { .. } => 0,
    }
}

/// MS25 D7: render a value as its GROUP_CONCAT text part. Date/Timestamp use
/// the same DA5 string form as the CLI renderer; other scalars use their
/// `Display`/`to_string` form.
fn concat_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Date(d) => crate::executor::datetime::format_date(*d),
        Value::Timestamp(t) => crate::executor::datetime::format_timestamp(*t),
        Value::Null => String::new(),
    }
}

/// Hash-based aggregate executor for GROUP BY + aggregate functions
pub struct AggregateExecutor {
    input: Box<dyn Executor + Send>,
    group_by: Vec<String>,
    /// MS13 T8: 分组键求值表达式（与 `group_by` 名单一一对应）。列名键 =
    /// `ColumnExpression`（与既有名字查索引语义逐字节一致），表达式/别名/
    /// 位置键 = SELECT 项编译表达式。
    group_key_exprs: Vec<ExpressionRef>,
    aggregates: Vec<AggregateFunc>,
    /// MS25 T8 (D9): per-aggregate parameter index into the input row, parallel
    /// to `aggregates`. `Some` = plan-time layout-resolved index (JOIN base);
    /// `None` = no parameter (`COUNT(*)`) or single-table base, which keeps the
    /// existing by-name lookup semantics byte-identically.
    aggregate_arg_indices: Vec<Option<usize>>,
    /// Output column names for projection optimization (future use)
    #[allow(dead_code)]
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
    /// MS32 Iter003（design D5）：语句级查询内存预算——分组键与状态值入账
    /// （纯数值累加器不计）。
    budget: SharedBudget,
}

impl AggregateExecutor {
    pub fn new(
        input: Box<dyn Executor + Send>,
        group_by: Vec<String>,
        group_key_exprs: Vec<ExpressionRef>,
        aggregates: Vec<AggregateFunc>,
        aggregate_arg_indices: Vec<Option<usize>>,
        output_columns: Vec<String>,
        column_indices: HashMap<String, usize>,
    ) -> Self {
        Self {
            input,
            group_by,
            group_key_exprs,
            aggregates,
            aggregate_arg_indices,
            output_columns,
            column_indices,
            groups: HashMap::new(),
            single_group: None,
            has_consumed_input: false,
            output_rows: None,
            output_index: 0,
            budget: unlimited_budget(),
        }
    }

    /// MS32 Iter003（design D5）：注入语句级预算（pipeline 构造臂）。
    pub(crate) fn with_budget(mut self, budget: SharedBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Consume all input rows and accumulate aggregate states
    async fn consume_input(&mut self) -> storage::Result<()> {
        let is_no_group_by = self.group_by.is_empty();

        if is_no_group_by {
            let states: Vec<AggregateState> =
                self.aggregates.iter().map(AggregateState::new).collect();
            self.single_group = Some(states);
        }

        let budget = self.budget.clone();
        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(row)) => {
                    if is_no_group_by {
                        let states = self.single_group.as_mut().unwrap();
                        for (i, func) in self.aggregates.iter().enumerate() {
                            let value = Self::extract_value(
                                &row,
                                func,
                                self.aggregate_arg_indices.get(i).copied().flatten(),
                                &self.column_indices,
                            );
                            Self::update_state(&budget, &mut states[i], &value)?;
                        }
                    } else {
                        let group_key = self.extract_group_key(&row)?;
                        let states = match self.groups.entry(group_key) {
                            // MS32 Iter003（design D5）：新组分组键入账（驻留
                            // 到语句结束）；既有组无键计费。
                            Entry::Vacant(vacant) => {
                                budget.charge(row_bytes(vacant.key()))?;
                                vacant.insert(
                                    self.aggregates.iter().map(AggregateState::new).collect(),
                                )
                            }
                            Entry::Occupied(occupied) => occupied.into_mut(),
                        };
                        for (i, func) in self.aggregates.iter().enumerate() {
                            let value = Self::extract_value(
                                &row,
                                func,
                                self.aggregate_arg_indices.get(i).copied().flatten(),
                                &self.column_indices,
                            );
                            Self::update_state(&budget, &mut states[i], &value)?;
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

    /// MS32 Iter003（design D5）：状态值驻留增量入账——按更新前后状态字节
    /// 数差入账/出账；纯数值累加器（COUNT 族、数值 count）恒 0 不计。默认
    /// 不限制（limit 0）时跳过前后度量，维持零开销。
    fn update_state(
        budget: &SharedBudget,
        state: &mut AggregateState,
        value: &Value,
    ) -> storage::Result<()> {
        if budget.limit() == 0 {
            state.update(value);
            return Ok(());
        }
        let before = state_value_bytes(state);
        state.update(value);
        let after = state_value_bytes(state);
        if after > before {
            budget.charge(after - before)?;
        } else if after < before {
            budget.release(before - after);
        }
        Ok(())
    }

    /// Extract value from row for aggregate function
    ///
    /// MS25 T8 (D9): `arg_index` is the plan-time layout-resolved parameter
    /// index (JOIN base) and is used when present; otherwise the existing
    /// by-name lookup runs unchanged.
    fn extract_value(
        row: &[Value],
        func: &AggregateFunc,
        arg_index: Option<usize>,
        column_indices: &HashMap<String, usize>,
    ) -> Value {
        match func {
            AggregateFunc::CountStar => Value::Int(1),
            // MS25 D7: every column-reading variant shares the name lookup.
            _ => match arg_index.or_else(|| {
                column_indices
                    .get(&func.column().unwrap_or("").to_lowercase())
                    .copied()
            }) {
                Some(idx) => row.get(idx).cloned().unwrap_or(Value::Null),
                None => Value::Null,
            },
        }
    }

    /// Extract group key from row by evaluating the group key expressions
    /// (MS13 T8). NULL 求值结果作为键值参与分桶（HashMap 归并单组）。
    fn extract_group_key(&self, row: &[Value]) -> Result<Vec<Value>, storage::StorageError> {
        let mut key = Vec::with_capacity(self.group_key_exprs.len());
        for expr in &self.group_key_exprs {
            match expr.evaluate(row) {
                Ok(v) => key.push(v),
                Err(e) => {
                    return Err(storage::StorageError::ExecutionError(format!(
                        "GROUP BY key evaluation error: {}",
                        e
                    )))
                }
            }
        }
        Ok(key)
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
