//! 排序合并连接：执行器多重集合，以及整数主键基表扫描的计划选择。
//!
//! 执行器用例自己提供输入。语句用例各自打开数据库。

use rtsql::database::Database;
use rtsql::executor::{
    ColumnRef, ExecResult, Executor, JoinCondition, JoinConfig, JoinExecutor, JoinSide,
    OutputColumn, SortMergeJoinExecutor, Value,
};
use rtsql::SqlOutcome;
use std::collections::HashMap;
use tempfile::TempDir;

/// 按给定行序逐行产出后结束。
struct RowSource {
    rows: Vec<Vec<Value>>,
    index: usize,
}

impl RowSource {
    fn new(rows: Vec<Vec<Value>>) -> Self {
        Self { rows, index: 0 }
    }
}

#[async_trait::async_trait]
impl Executor for RowSource {
    async fn next(&mut self) -> rtsql::storage::Result<Option<ExecResult>> {
        if self.index >= self.rows.len() {
            return Ok(None);
        }
        let row = self.rows[self.index].clone();
        self.index += 1;
        Ok(Some(ExecResult::Row(row)))
    }
}

fn column_ref(table: &str, column: &str) -> ColumnRef {
    ColumnRef {
        table: Some(table.to_string()),
        column: column.to_string(),
    }
}

fn output_column(table: &str, column: &str, column_index: usize) -> OutputColumn {
    OutputColumn {
        table: Some(table.to_string()),
        column: column.to_string(),
        table_alias: table.to_string(),
        column_index,
    }
}

fn indices(names: &[&str]) -> HashMap<String, usize> {
    names
        .iter()
        .enumerate()
        .map(|(idx, name)| ((*name).to_string(), idx))
        .collect()
}

/// 单列等值：左行 `(键, 标记)`，右行 `(键, 标记)`，输出两侧标记。
fn join_config(left_rows: Vec<Vec<Value>>, right_rows: Vec<Vec<Value>>) -> JoinConfig {
    JoinConfig {
        left_executor: Box::new(RowSource::new(left_rows)),
        right_executor: Box::new(RowSource::new(right_rows)),
        conditions: vec![JoinCondition {
            left_column: column_ref("l", "k"),
            right_column: column_ref("r", "k"),
        }],
        output_columns: vec![output_column("l", "mark", 1), output_column("r", "mark", 1)],
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        left_column_indices: indices(&["k", "mark"]),
        right_column_indices: indices(&["k", "mark"]),
        right_table_name: "r".to_string(),
    }
}

fn duplicate_and_null_rows() -> (Vec<Vec<Value>>, Vec<Vec<Value>>) {
    let left = vec![
        vec![Value::Int(1), Value::String("a".to_string())],
        vec![Value::Int(1), Value::String("b".to_string())],
        vec![Value::Int(2), Value::String("c".to_string())],
        vec![Value::Null, Value::String("n".to_string())],
    ];
    let right = vec![
        vec![Value::Int(1), Value::String("x".to_string())],
        vec![Value::Int(1), Value::String("y".to_string())],
        vec![Value::Null, Value::String("m".to_string())],
        vec![Value::Int(3), Value::String("z".to_string())],
    ];
    (left, right)
}

async fn collect_rows(executor: &mut dyn Executor) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    while let Some(result) = executor.next().await.unwrap() {
        if let ExecResult::Row(row) = result {
            rows.push(row);
        }
    }
    rows
}

fn multiset(rows: Vec<Vec<Value>>) -> HashMap<Vec<Value>, usize> {
    let mut counts = HashMap::new();
    for row in rows {
        *counts.entry(row).or_insert(0) += 1;
    }
    counts
}

#[tokio::test]
async fn sort_merge_join_matches_hash_multiset() {
    let (left, right) = duplicate_and_null_rows();
    let mut hash_join = JoinExecutor::new(join_config(left.clone(), right.clone()));
    let mut sort_merge = SortMergeJoinExecutor::new(join_config(left, right));

    let hash_rows = collect_rows(&mut hash_join).await;
    let sort_rows = collect_rows(&mut sort_merge).await;

    let expected = multiset(vec![
        vec![
            Value::String("a".to_string()),
            Value::String("x".to_string()),
        ],
        vec![
            Value::String("a".to_string()),
            Value::String("y".to_string()),
        ],
        vec![
            Value::String("b".to_string()),
            Value::String("x".to_string()),
        ],
        vec![
            Value::String("b".to_string()),
            Value::String("y".to_string()),
        ],
    ]);
    assert_eq!(multiset(hash_rows), expected);
    assert_eq!(multiset(sort_rows), expected);
}

#[tokio::test]
async fn sort_merge_join_empty_input_matches_hash() {
    assert_same_multiset(Vec::new(), Vec::new()).await;

    let right_only = vec![vec![Value::Int(1), Value::String("x".to_string())]];
    assert_same_multiset(Vec::new(), right_only).await;

    let left_only = vec![vec![Value::Int(1), Value::String("a".to_string())]];
    let rows = assert_same_multiset(left_only, Vec::new()).await;
    assert!(rows.is_empty());
}

#[tokio::test]
async fn sort_merge_join_int_does_not_match_float() {
    let left = vec![
        vec![Value::Int(2), Value::String("li".to_string())],
        vec![Value::Float(2.0), Value::String("lf".to_string())],
    ];
    let right = vec![
        vec![Value::Float(2.0), Value::String("rf".to_string())],
        vec![Value::Int(2), Value::String("ri".to_string())],
    ];
    let rows = assert_same_multiset(left, right).await;
    let expected = multiset(vec![
        vec![
            Value::String("li".to_string()),
            Value::String("ri".to_string()),
        ],
        vec![
            Value::String("lf".to_string()),
            Value::String("rf".to_string()),
        ],
    ]);
    assert_eq!(rows, expected);
}

const NEG_ZERO_BITS: u64 = 0x8000_0000_0000_0000;

/// 唯一列是键，输出列是左键和右键。
fn signed_zero_config(left_rows: Vec<Vec<Value>>, right_rows: Vec<Vec<Value>>) -> JoinConfig {
    JoinConfig {
        left_executor: Box::new(RowSource::new(left_rows)),
        right_executor: Box::new(RowSource::new(right_rows)),
        conditions: vec![JoinCondition {
            left_column: column_ref("l", "k"),
            right_column: column_ref("r", "k"),
        }],
        output_columns: vec![output_column("l", "k", 0), output_column("r", "k", 0)],
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        left_column_indices: indices(&["k"]),
        right_column_indices: indices(&["k"]),
        right_table_name: "r".to_string(),
    }
}

/// `Value` 的相等把负零和正零看成同一个值，位模式用 `to_bits` 读取。
fn signed_zero_bits(rows: &[Vec<Value>]) -> Vec<(u64, u64)> {
    let mut pairs: Vec<(u64, u64)> = rows
        .iter()
        .map(|row| {
            let left = match &row[0] {
                Value::Float(value) => value.to_bits(),
                other => panic!("left cell is not a float: {other:?}"),
            };
            let right = match &row[1] {
                Value::Float(value) => value.to_bits(),
                other => panic!("right cell is not a float: {other:?}"),
            };
            (left, right)
        })
        .collect();
    pairs.sort();
    pairs
}

#[derive(Debug, PartialEq, Eq)]
struct SignedZeroMatch {
    sort_merge: Vec<(u64, u64)>,
    hash_join: Vec<(u64, u64)>,
}

async fn assert_signed_zero_match(
    left: Vec<Vec<Value>>,
    right: Vec<Vec<Value>>,
    mut expected: Vec<(u64, u64)>,
) {
    expected.sort();
    let mut hash_join = JoinExecutor::new(signed_zero_config(left.clone(), right.clone()));
    let mut sort_merge = SortMergeJoinExecutor::new(signed_zero_config(left, right));
    let hash_join = signed_zero_bits(&collect_rows(&mut hash_join).await);
    let sort_merge = signed_zero_bits(&collect_rows(&mut sort_merge).await);
    assert_eq!(
        SignedZeroMatch {
            sort_merge,
            hash_join,
        },
        SignedZeroMatch {
            sort_merge: expected.clone(),
            hash_join: expected,
        }
    );
}

#[tokio::test]
async fn signed_zero_matches_across_hash_and_sort_merge() {
    let neg = Value::Float(-0.0);
    let pos = Value::Float(0.0);

    assert_signed_zero_match(
        vec![vec![neg.clone()]],
        vec![vec![pos.clone()]],
        vec![(NEG_ZERO_BITS, 0)],
    )
    .await;

    assert_signed_zero_match(
        vec![vec![pos.clone()]],
        vec![vec![neg.clone()]],
        vec![(0, NEG_ZERO_BITS)],
    )
    .await;

    assert_signed_zero_match(
        vec![vec![neg.clone()]],
        vec![vec![neg], vec![pos]],
        vec![(NEG_ZERO_BITS, NEG_ZERO_BITS), (NEG_ZERO_BITS, 0)],
    )
    .await;
}

async fn assert_same_multiset(
    left: Vec<Vec<Value>>,
    right: Vec<Vec<Value>>,
) -> HashMap<Vec<Value>, usize> {
    let mut hash_join = JoinExecutor::new(join_config(left.clone(), right.clone()));
    let mut sort_merge = SortMergeJoinExecutor::new(join_config(left, right));
    let hash_rows = multiset(collect_rows(&mut hash_join).await);
    let sort_rows = multiset(collect_rows(&mut sort_merge).await);
    assert_eq!(sort_rows, hash_rows);
    sort_rows
}

async fn open_db() -> (Database, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str) {
    db.execute_sql(sql)
        .await
        .unwrap_or_else(|err| panic!("{sql} failed: {err}"));
}

async fn query_rows(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        SqlOutcome::AffectedRows(n) => panic!("expected rows from {sql}, got {n} affected"),
    }
}

fn cell_text(row: &[Value]) -> String {
    match &row[0] {
        Value::String(text) => text.clone(),
        other => panic!("expected text cell, got {other:?}"),
    }
}

async fn explain_lines(db: &Database, sql: &str) -> Vec<String> {
    query_rows(db, sql)
        .await
        .into_iter()
        .map(|row| cell_text(&row))
        .collect()
}

/// 没有写出 PRIMARY KEY 的整数首列，关闭再打开后仍走哈希。
#[tokio::test]
async fn implicit_first_column_integer_key_stays_hash_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE r (a INT)").await;
        exec_ok(&db, "CREATE TABLE s (b INT)").await;
        db.close().await.unwrap();
    }

    let db = Database::open(&path).await.unwrap();
    let lines = explain_lines(&db, "EXPLAIN SELECT r.a FROM r JOIN s ON r.a = s.b").await;
    assert_line_starts_with(&lines, "Join [");
}

/// 两侧都是整数主键基表扫描时，等值内连接发出排序合并，结果多重集合是 `(2, 2)`。
#[tokio::test]
async fn integer_pk_base_scans_select_sort_merge() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE customers (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE orders (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO customers VALUES (1), (2)").await;
    exec_ok(&db, "INSERT INTO orders VALUES (2), (3)").await;

    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT customers.id, orders.id FROM customers JOIN orders ON customers.id = orders.id",
    )
    .await;
    assert!(
        lines
            .iter()
            .any(|line| line.trim_start().starts_with("SortMergeJoin")),
        "计划行应以 SortMergeJoin 开头: {lines:?}"
    );

    let rows = query_rows(
        &db,
        "SELECT customers.id, orders.id FROM customers JOIN orders ON customers.id = orders.id",
    )
    .await;
    assert_eq!(
        multiset(rows),
        multiset(vec![vec![Value::Int(2), Value::Int(2)]])
    );
}

/// 写出整数主键的两表关闭再打开后，等值内连接仍是排序合并。
#[tokio::test]
async fn written_integer_primary_key_stays_sort_merge_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE customers (id INT PRIMARY KEY)").await;
        exec_ok(&db, "CREATE TABLE orders (id INT PRIMARY KEY)").await;
        db.close().await.unwrap();
    }

    let db = Database::open(&path).await.unwrap();
    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT customers.id, orders.id FROM customers JOIN orders ON customers.id = orders.id",
    )
    .await;
    assert_line_starts_with(&lines, "SortMergeJoin");
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn assert_line_starts_with(lines: &[String], prefix: &str) {
    assert!(
        lines.iter().any(|line| line.trim_start().starts_with(prefix)),
        "计划行应以 {prefix} 开头: {lines:?}"
    );
}

/// 自连接的两侧扫描是同一张整数主键表，条件文本使用真实表名。
#[tokio::test]
async fn self_join_integer_pk_selects_sort_merge() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE customers (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO customers VALUES (1), (2)").await;

    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT a.id FROM customers a JOIN customers b ON a.id = b.id",
    )
    .await;
    assert_line_starts_with(&lines, "SortMergeJoin");
    assert!(
        lines.iter().any(|line| line.contains("customers.id = customers.id")),
        "条件文本应是 customers.id = customers.id: {lines:?}"
    );

    let rows = query_rows(
        &db,
        "SELECT a.id FROM customers a JOIN customers b ON a.id = b.id",
    )
    .await;
    assert_eq!(
        multiset(rows),
        multiset(vec![vec![Value::Int(1)], vec![Value::Int(2)]])
    );
}

/// `ORDER BY` 包在排序合并之外，结果次序由 Sort 决定。
#[tokio::test]
async fn order_by_keeps_sort_merge_and_sort() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE customers (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE orders (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO customers VALUES (1), (2)").await;
    exec_ok(&db, "INSERT INTO orders VALUES (2), (3)").await;

    let sql = "SELECT customers.id, orders.id FROM customers JOIN orders \
               ON customers.id = orders.id ORDER BY customers.id";
    let lines = explain_lines(&db, &format!("EXPLAIN {sql}")).await;
    let sort_at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Sort ("))
        .unwrap_or_else(|| panic!("缺少 Sort (: {lines:?}"));
    let join_at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("SortMergeJoin"))
        .unwrap_or_else(|| panic!("缺少 SortMergeJoin: {lines:?}"));
    assert!(sort_at < join_at, "Sort 应出现在 SortMergeJoin 之前: {lines:?}");
    assert!(
        indent_of(&lines[sort_at]) < indent_of(&lines[join_at]),
        "Sort 的缩进应浅于 SortMergeJoin: {lines:?}"
    );

    let rows = query_rows(&db, sql).await;
    assert_eq!(rows, vec![vec![Value::Int(2), Value::Int(2)]]);
}

/// 计划文本点名排序合并节点和等值条件。
#[tokio::test]
async fn explain_integer_pk_join_names_sort_merge() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE customers (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE orders (id INT PRIMARY KEY)").await;

    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT customers.id FROM customers JOIN orders ON customers.id = orders.id",
    )
    .await;
    assert_line_starts_with(&lines, "SortMergeJoin");
    assert!(
        lines.iter().any(|line| line.contains("customers.id = orders.id")),
        "计划文本应含等值条件: {lines:?}"
    );
}

/// 语句入口的查询内存上限仍是 SQLSTATE 53200，没有成功结果行。
#[tokio::test]
async fn sort_merge_join_statement_memory_limit_53200() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE l (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE r (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO l VALUES (1)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1)").await;
    db.set_query_memory_limit(16);

    let err = db
        .execute_sql("SELECT l.id, r.id FROM l JOIN r ON l.id = r.id")
        .await
        .expect_err("上限 16 时语句必须失败");
    assert_eq!(err.sqlstate.as_deref(), Some("53200"));
    assert_eq!(
        err.message,
        "Execution error: query memory limit exceeded (limit 16 bytes)"
    );
}

/// 非主键列，以及多一条非主键等值腿，整步仍是哈希。
#[tokio::test]
async fn non_pk_equijoin_stays_hash() {
    let (db, _dir) = open_db().await;
    exec_ok(
        &db,
        "CREATE TABLE orders (id INT PRIMARY KEY, customer_id INT, amount INT)",
    )
    .await;
    exec_ok(
        &db,
        "CREATE TABLE customers (id INT PRIMARY KEY, name STRING)",
    )
    .await;

    let foreign_key = explain_lines(
        &db,
        "EXPLAIN SELECT orders.id FROM orders JOIN customers ON orders.customer_id = customers.id",
    )
    .await;
    assert_line_starts_with(&foreign_key, "Join [");

    let mixed = explain_lines(
        &db,
        "EXPLAIN SELECT customers.id FROM customers JOIN orders \
         ON customers.id = orders.id AND customers.id = orders.customer_id",
    )
    .await;
    assert_line_starts_with(&mixed, "Join [");
}

/// 字符串主键的等值内连接仍是哈希。
///
/// 新的 `CREATE TABLE ... TEXT PRIMARY KEY` 会在计划期被拒绝，所以这两张表按
/// 已有存量建表入口写入目录，再经 `execute_sql` 观察连接计划。
#[tokio::test]
async fn string_pk_equijoin_stays_hash() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let db = Database::open(&path).await.unwrap();
        for table in ["customers", "orders"] {
            db.table_manager
                .create_table_with_constraints(
                    table,
                    vec![(
                        "id".into(),
                        rtsql::storage::ColumnType::String(20),
                        false,
                        false,
                        None,
                    )],
                    "id",
                    false,
                )
                .await
                .unwrap();
        }
        db.close().await.unwrap();
    }

    let db = Database::open(&path).await.unwrap();
    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT customers.id FROM customers JOIN orders ON customers.id = orders.id",
    )
    .await;
    assert_line_starts_with(&lines, "Join [");
}

/// 三表链的第一步可以是排序合并，第二步的左输入已是连接，仍是哈希。
#[tokio::test]
async fn third_table_join_stays_hash() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE a (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE b (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE c (id INT PRIMARY KEY)").await;

    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT a.id FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id",
    )
    .await;
    let hash_at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Join ["))
        .unwrap_or_else(|| panic!("缺少 Join [: {lines:?}"));
    let merge_at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("SortMergeJoin"))
        .unwrap_or_else(|| panic!("缺少 SortMergeJoin: {lines:?}"));
    assert!(
        hash_at < merge_at && indent_of(&lines[hash_at]) < indent_of(&lines[merge_at]),
        "第二步应是哈希，第一步是排序合并: {lines:?}"
    );
}

/// 整数主键连接作为 IN 子查询仍按连接形状拒绝，不误报多列。
#[tokio::test]
async fn in_subquery_integer_pk_join_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE customers (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE orders (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE probe (id INT)").await;

    let err = db
        .execute_sql(
            "SELECT id FROM probe WHERE id IN \
             (SELECT customers.id FROM customers JOIN orders ON customers.id = orders.id)",
        )
        .await
        .expect_err("IN 子查询中的连接必须被拒绝");
    assert!(
        err.message.contains("IN subquery with JOIN is not supported"),
        "错误应点名连接形状: {}",
        err.message
    );
    assert!(
        !err.message.contains("Subquery returns multiple columns"),
        "不得误报多列: {}",
        err.message
    );
}
