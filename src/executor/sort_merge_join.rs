//! 等值内连接的排序合并执行器。
//!
//! 读完两侧输入后，丢掉键中含 NULL 的行（不匹配、不入账），其余行按连接键排序，
//! 只在键 `PartialEq` 相等时做组间乘积。变体不同的值不是同一组。

use crate::executor::query_budget::{row_bytes, unlimited_budget, SharedBudget};
use crate::executor::{
    ColumnRef, ExecResult, Executor, JoinCondition, JoinConfig, JoinSide, OutputColumn, Value,
};
use crate::storage::Result;
use std::cmp::Ordering;
use std::collections::HashMap;

/// 排序合并连接执行器。
pub struct SortMergeJoinExecutor {
    left_executor: Box<dyn Executor + Send>,
    right_executor: Box<dyn Executor + Send>,
    conditions: Vec<JoinCondition>,
    output_columns: Vec<OutputColumn>,
    left_column_indices: HashMap<String, usize>,
    right_column_indices: HashMap<String, usize>,
    output_sides: Vec<JoinSide>,
    _right_table_name: String,
    budget: SharedBudget,
    outputs: Vec<Vec<Value>>,
    output_index: usize,
    prepared: bool,
}

impl SortMergeJoinExecutor {
    /// 创建执行器。未注入预算时 `charge` 对 limit 0 直接成功。
    pub fn new(config: JoinConfig) -> Self {
        Self {
            left_executor: config.left_executor,
            right_executor: config.right_executor,
            conditions: config.conditions,
            output_columns: config.output_columns,
            left_column_indices: config.left_column_indices,
            right_column_indices: config.right_column_indices,
            output_sides: config.output_sides,
            _right_table_name: config.right_table_name,
            budget: unlimited_budget(),
            outputs: Vec::new(),
            output_index: 0,
            prepared: false,
        }
    }

    /// 注入语句级查询内存预算。
    pub(crate) fn with_budget(mut self, budget: SharedBudget) -> Self {
        self.budget = budget;
        self
    }

    async fn prepare(&mut self) -> Result<()> {
        let left = collect_keyed(
            &mut self.left_executor,
            &self.conditions,
            &self.left_column_indices,
            false,
            &self.budget,
        )
        .await?;
        let right = collect_keyed(
            &mut self.right_executor,
            &self.conditions,
            &self.right_column_indices,
            true,
            &self.budget,
        )
        .await?;
        self.outputs = merge_product(&left, &right, &self.output_columns, &self.output_sides);
        Ok(())
    }
}

/// `(真实表, 列)` 限定键优先，裸名回退。与哈希连接的条件键查表相同。
fn condition_column_index(map: &HashMap<String, usize>, column: &ColumnRef) -> Option<usize> {
    if let Some(table) = &column.table {
        let qualified = format!("{}.{}", table.to_lowercase(), column.column.to_lowercase());
        if let Some(idx) = map.get(&qualified) {
            return Some(*idx);
        }
    }
    map.get(&column.column).copied()
}

fn join_key(
    row: &[Value],
    conditions: &[JoinCondition],
    indices: &HashMap<String, usize>,
    right_side: bool,
) -> Option<Vec<Value>> {
    let key: Vec<Value> = conditions
        .iter()
        .map(|cond| {
            let column = if right_side {
                &cond.right_column
            } else {
                &cond.left_column
            };
            let idx =
                condition_column_index(indices, column).expect("join column index must exist");
            row[idx].clone()
        })
        .collect();
    if key.iter().any(|value| matches!(value, Value::Null)) {
        None
    } else {
        Some(key)
    }
}

async fn collect_keyed(
    executor: &mut Box<dyn Executor + Send>,
    conditions: &[JoinCondition],
    indices: &HashMap<String, usize>,
    right_side: bool,
    budget: &SharedBudget,
) -> Result<Vec<(Vec<Value>, Vec<Value>)>> {
    let mut rows = Vec::new();
    while let Some(result) = executor.next().await? {
        let ExecResult::Row(row) = result else {
            continue;
        };
        let Some(key) = join_key(&row, conditions, indices, right_side) else {
            continue;
        };
        budget.charge(row_bytes(&row))?;
        rows.push((key, row));
    }
    rows.sort_by(|left, right| compare_join_keys(&left.0, &right.0));
    Ok(rows)
}

fn variant_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Int(_) => 2,
        Value::Float(_) => 3,
        Value::String(_) => 4,
        Value::Date(_) => 5,
        Value::Timestamp(_) => 6,
    }
}

/// 先按变体区分。同类型次序与排序执行器的同类型分支一致。
///
/// 浮点里的 NaN 与有序数不相等：`partial_cmp` 的 `None` 若当成 `Equal`，
/// 会破坏全序，排序契约不再成立。NaN 与任何键的 `PartialEq` 也不成立，
/// 因此不会进入合并。
fn compare_join_value(left: &Value, right: &Value) -> Ordering {
    let by_variant = variant_rank(left).cmp(&variant_rank(right));
    if by_variant != Ordering::Equal {
        return by_variant;
    }
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => left.cmp(right),
        (Value::Float(left), Value::Float(right)) => compare_floats(*left, *right),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::Date(left), Value::Date(right)) => left.cmp(right),
        (Value::Timestamp(left), Value::Timestamp(right)) => left.cmp(right),
        (Value::Null, Value::Null) => Ordering::Equal,
        _ => Ordering::Equal,
    }
}

fn compare_floats(left: f64, right: f64) -> Ordering {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}

fn compare_join_keys(left: &[Value], right: &[Value]) -> Ordering {
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        let ordering = compare_join_value(left_value, right_value);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

fn merge_product(
    left_rows: &[(Vec<Value>, Vec<Value>)],
    right_rows: &[(Vec<Value>, Vec<Value>)],
    output_columns: &[OutputColumn],
    output_sides: &[JoinSide],
) -> Vec<Vec<Value>> {
    let mut outputs = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left_rows.len() && right_index < right_rows.len() {
        let ordering = compare_join_keys(&left_rows[left_index].0, &right_rows[right_index].0);
        let same_key = left_rows[left_index].0 == right_rows[right_index].0;
        if ordering == Ordering::Equal && same_key {
            let left_end = group_end(left_rows, left_index);
            let right_end = group_end(right_rows, right_index);
            for left in left_index..left_end {
                for right in right_index..right_end {
                    outputs.push(build_output_row(
                        output_columns,
                        output_sides,
                        &left_rows[left].1,
                        &right_rows[right].1,
                    ));
                }
            }
            left_index = left_end;
            right_index = right_end;
        } else if ordering == Ordering::Less {
            left_index += 1;
        } else {
            right_index += 1;
        }
    }
    outputs
}

fn group_end(rows: &[(Vec<Value>, Vec<Value>)], start: usize) -> usize {
    let mut end = start + 1;
    while end < rows.len() && rows[end].0 == rows[start].0 {
        end += 1;
    }
    end
}

fn build_output_row(
    output_columns: &[OutputColumn],
    output_sides: &[JoinSide],
    left_row: &[Value],
    right_row: &[Value],
) -> Vec<Value> {
    output_columns
        .iter()
        .zip(output_sides)
        .map(|(column, side)| match side {
            JoinSide::Left => left_row[column.column_index].clone(),
            JoinSide::Right => right_row[column.column_index].clone(),
        })
        .collect()
}

#[async_trait::async_trait]
impl Executor for SortMergeJoinExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if !self.prepared {
            self.prepare().await?;
            self.prepared = true;
        }
        if self.output_index >= self.outputs.len() {
            return Ok(None);
        }
        let row = self.outputs[self.output_index].clone();
        self.output_index += 1;
        Ok(Some(ExecResult::Row(row)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::query_budget::QueryBudget;
    use crate::storage::StorageError;
    use std::sync::Arc;

    struct RowSource {
        rows: Vec<Vec<Value>>,
        index: usize,
    }

    #[async_trait::async_trait]
    impl Executor for RowSource {
        async fn next(&mut self) -> Result<Option<ExecResult>> {
            if self.index >= self.rows.len() {
                return Ok(None);
            }
            let row = self.rows[self.index].clone();
            self.index += 1;
            Ok(Some(ExecResult::Row(row)))
        }
    }

    fn config(left: Vec<Vec<Value>>, right: Vec<Vec<Value>>) -> JoinConfig {
        let mut left_column_indices = HashMap::new();
        left_column_indices.insert("k".to_string(), 0);
        let mut right_column_indices = HashMap::new();
        right_column_indices.insert("k".to_string(), 0);
        JoinConfig {
            left_executor: Box::new(RowSource {
                rows: left,
                index: 0,
            }),
            right_executor: Box::new(RowSource {
                rows: right,
                index: 0,
            }),
            conditions: vec![JoinCondition {
                left_column: ColumnRef {
                    table: None,
                    column: "k".to_string(),
                },
                right_column: ColumnRef {
                    table: None,
                    column: "k".to_string(),
                },
            }],
            output_columns: vec![OutputColumn {
                table: None,
                column: "k".to_string(),
                table_alias: "l".to_string(),
                column_index: 0,
            }],
            output_sides: vec![JoinSide::Left],
            left_column_indices,
            right_column_indices,
            right_table_name: "r".to_string(),
        }
    }

    #[tokio::test]
    async fn sort_merge_join_memory_limit_rejects() {
        let budget = Arc::new(QueryBudget::new(16));
        let mut executor = SortMergeJoinExecutor::new(config(
            vec![vec![Value::Int(1)]],
            vec![vec![Value::Int(1)]],
        ))
        .with_budget(budget);
        let err = executor
            .next()
            .await
            .expect_err("两侧各一行整数，预算 16 字节小于两侧保留行");
        match err {
            StorageError::QueryMemoryExceeded { limit } => assert_eq!(limit, 16),
            other => panic!("应为 QueryMemoryExceeded，实际 {other}"),
        }
    }
}
