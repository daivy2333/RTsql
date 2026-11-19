//! 打印四个执行器的排空耗时、对应克隆循环耗时和行数。
//! 本测试不断言 10% 比较的真假。

use rtsql::executor::{
    ColumnRef, ConstantPredicate, ExecResult, Executor, JoinCondition, JoinConfig, JoinExecutor,
    JoinSide, NestedLoopJoinExecutor, NestedLoopJoinType, OrderByColumn, OutputColumn,
    PredicateRef, SortExecutor, SortMergeJoinExecutor, Value,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

struct RowSource {
    rows: std::vec::IntoIter<Vec<Value>>,
}

impl RowSource {
    fn new(rows: Vec<Vec<Value>>) -> Self {
        Self {
            rows: rows.into_iter(),
        }
    }
}

#[async_trait::async_trait]
impl Executor for RowSource {
    async fn next(&mut self) -> rtsql::storage::Result<Option<ExecResult>> {
        Ok(self.rows.next().map(ExecResult::Row))
    }
}

fn key_column() -> ColumnRef {
    ColumnRef {
        table: None,
        column: "k".to_string(),
    }
}

fn output_column(column: &str, column_index: usize) -> OutputColumn {
    OutputColumn {
        table: None,
        column: column.to_string(),
        table_alias: String::new(),
        column_index,
    }
}

fn output_columns() -> Vec<OutputColumn> {
    vec![
        output_column("k", 0),
        output_column("name", 1),
        output_column("k", 0),
        output_column("v", 1),
    ]
}

fn output_sides() -> Vec<JoinSide> {
    vec![
        JoinSide::Left,
        JoinSide::Left,
        JoinSide::Right,
        JoinSide::Right,
    ]
}

fn left_indices() -> HashMap<String, usize> {
    HashMap::from([("k".to_string(), 0), ("name".to_string(), 1)])
}

fn right_indices() -> HashMap<String, usize> {
    HashMap::from([("k".to_string(), 0), ("v".to_string(), 1)])
}

fn join_config(left: Vec<Vec<Value>>, right: Vec<Vec<Value>>) -> JoinConfig {
    JoinConfig {
        left_executor: Box::new(RowSource::new(left)),
        right_executor: Box::new(RowSource::new(right)),
        conditions: vec![JoinCondition {
            left_column: key_column(),
            right_column: key_column(),
        }],
        output_columns: output_columns(),
        output_sides: output_sides(),
        left_column_indices: left_indices(),
        right_column_indices: right_indices(),
        right_table_name: "r".to_string(),
    }
}

fn customer_rows() -> Vec<Vec<Value>> {
    (0..50i64)
        .map(|i| vec![Value::Int(i), Value::String(format!("customer_{i}"))])
        .collect()
}

fn hash_right_rows() -> Vec<Vec<Value>> {
    (0..50i64)
        .map(|i| vec![Value::Int(i), Value::Int(i * 100)])
        .collect()
}

fn sort_input_rows() -> Vec<Vec<Value>> {
    (0..100i64)
        .rev()
        .map(|i| vec![Value::Int(i), Value::String(format!("user_{i}"))])
        .collect()
}

async fn drain(executor: &mut dyn Executor) -> (Vec<Vec<Value>>, u128) {
    let start = Instant::now();
    let mut rows = Vec::new();
    while let Some(result) = executor.next().await.expect("executor next") {
        if let ExecResult::Row(row) = result {
            rows.push(row);
        }
    }
    let elapsed = Instant::now().duration_since(start).as_nanos();
    (rows, elapsed)
}

fn nlj_clone_ns(left_rows: &[Vec<Value>], right_rows: &[Vec<Value>]) -> u128 {
    let start = Instant::now();
    for left_row in left_rows {
        for right_row in right_rows {
            let combined: Vec<Value> = left_row
                .iter()
                .cloned()
                .chain(right_row.iter().cloned())
                .collect();
            std::hint::black_box(combined);
        }
    }
    Instant::now().duration_since(start).as_nanos()
}

fn handed_off_clone_ns(rows: &[Vec<Value>]) -> u128 {
    let start = Instant::now();
    for row in rows {
        std::hint::black_box(row.clone());
    }
    Instant::now().duration_since(start).as_nanos()
}

fn time_string_clones() -> u128 {
    let start = Instant::now();
    for i in 0..50 {
        let value = Value::String(format!("customer_{i}"));
        std::hint::black_box(value.clone());
    }
    Instant::now().duration_since(start).as_nanos()
}

fn print_field(name: &str, value: u128) {
    println!("{name}={value}");
}

#[tokio::test]
async fn avoidable_copy_times_and_row_counts() {
    let mut hash_join = JoinExecutor::new(join_config(customer_rows(), hash_right_rows()));
    let (hash_rows, hash_executor_ns) = drain(&mut hash_join).await;
    let hash_bucket_clone_ns = 0;

    let nlj_left = customer_rows();
    let nlj_right = customer_rows();
    let predicate: PredicateRef = Arc::new(ConstantPredicate { value: true });
    let mut nested_loop = NestedLoopJoinExecutor::new(
        Box::new(RowSource::new(nlj_left.clone())),
        Box::new(RowSource::new(nlj_right.clone())),
        predicate,
        output_columns(),
        output_sides(),
        NestedLoopJoinType::Inner,
    );
    let (nlj_rows, nlj_executor_ns) = drain(&mut nested_loop).await;
    let nlj_clone_ns = nlj_clone_ns(&nlj_left, &nlj_right);

    let mut sort = SortExecutor::new(
        Box::new(RowSource::new(sort_input_rows())),
        vec![OrderByColumn {
            column: "k".to_string(),
            asc: true,
            expr: None,
        }],
        vec!["k".to_string(), "name".to_string()],
    );
    let (sort_rows, sort_executor_ns) = drain(&mut sort).await;
    let sort_clone_ns = handed_off_clone_ns(&sort_rows);

    let mut sort_merge =
        SortMergeJoinExecutor::new(join_config(customer_rows(), hash_right_rows()));
    let (smj_rows, smj_executor_ns) = drain(&mut sort_merge).await;
    let smj_clone_ns = handed_off_clone_ns(&smj_rows);

    let string_clone_ns = time_string_clones();

    print_field("hash_executor_ns", hash_executor_ns);
    print_field("hash_bucket_clone_ns", hash_bucket_clone_ns);
    print_field("nlj_executor_ns", nlj_executor_ns);
    print_field("nlj_clone_ns", nlj_clone_ns);
    print_field("sort_executor_ns", sort_executor_ns);
    print_field("sort_clone_ns", sort_clone_ns);
    print_field("smj_executor_ns", smj_executor_ns);
    print_field("smj_clone_ns", smj_clone_ns);
    print_field("string_clone_ns", string_clone_ns);
    print_field("hash_rows", hash_rows.len() as u128);
    print_field("nlj_rows", nlj_rows.len() as u128);
    print_field("sort_rows", sort_rows.len() as u128);
    print_field("smj_rows", smj_rows.len() as u128);

    assert_eq!(hash_rows.len(), 50);
    assert_eq!(nlj_rows.len(), 2500);
    assert_eq!(sort_rows.len(), 100);
    assert_eq!(smj_rows.len(), 50);
    assert_eq!(sort_rows[0][0], Value::Int(0));
    assert_eq!(sort_rows[99][0], Value::Int(99));
    assert!(hash_executor_ns > 0);
    assert!(nlj_executor_ns > 0);
    assert!(sort_executor_ns > 0);
    assert!(smj_executor_ns > 0);
    assert_eq!(hash_bucket_clone_ns, 0);
}
