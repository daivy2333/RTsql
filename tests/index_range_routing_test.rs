//! 等值与范围路由：计划形状和行多重集合。

use rtsql::database::Database;
use rtsql::{ColumnMeta, SqlOutcome, Value};
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

async fn exec(db: &Database, sql: &str) {
    db.execute_sql(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql} failed: {error}"));
}

#[derive(Debug)]
struct Query {
    columns: Vec<ColumnMeta>,
    rows: Vec<Vec<Value>>,
}

async fn query(db: &Database, sql: &str) -> Query {
    match db.execute_sql(sql).await.expect(sql) {
        SqlOutcome::Rows { columns, rows } => Query { columns, rows },
        other => panic!("{sql}: expected rows, got {other:?}"),
    }
}

fn sorted_rows(columns: &[ColumnMeta], mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    if let Some(index) = columns.iter().position(|column| column.name == "id") {
        rows.sort_by(|left, right| match (&left[index], &right[index]) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            _ => panic!("id column must be an integer"),
        });
    } else {
        rows.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    }
    rows
}

fn assert_same_rows(before: &Query, after: &Query, what: &str) {
    assert_eq!(after.columns, before.columns, "{what} columns");
    assert_eq!(
        sorted_rows(&before.columns, before.rows.clone()),
        sorted_rows(&after.columns, after.rows.clone()),
        "{what} rows"
    );
}

async fn explain(db: &Database, sql: &str) -> String {
    let outcome = query(db, &format!("EXPLAIN {sql}")).await;
    outcome
        .rows
        .iter()
        .map(|row| match &row[0] {
            Value::String(text) => text.clone(),
            other => panic!("plan cell must be a string, got {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_int_range(plan: &str, index: &str) {
    assert!(
        plan.contains("IndexRangeScan") && plan.contains(&format!("index={index}")),
        "plan must name {index} on IndexRangeScan, got: {plan}"
    );
    assert!(
        !plan.contains("omitted=scan"),
        "all-integer range must not rescan omitted keys, got: {plan}"
    );
}

/// 新建单列整数索引的范围含负数，且不补扫。
#[tokio::test]
async fn new_int_range_includes_negatives() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "range")).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    for (id, n) in [(1, -5), (2, -1), (3, 0), (4, 1), (5, 3)] {
        exec(&db, &format!("INSERT INTO t VALUES ({id}, {n})")).await;
    }

    let statements = [
        "SELECT id, n FROM t WHERE n > 0",
        "SELECT id, n FROM t WHERE n < 0",
        "SELECT id, n FROM t WHERE n >= -1",
        "SELECT id, n FROM t WHERE n BETWEEN -2 AND 2",
        "SELECT id, n FROM t WHERE 0 < n",
        "SELECT id, n FROM t WHERE 0 > n",
        "SELECT id, n FROM t WHERE -1 <= n",
        "SELECT n FROM t WHERE n > 0",
        "SELECT id, n FROM t WHERE n = -1",
    ];
    let mut before = Vec::new();
    for sql in statements {
        before.push(query(&db, sql).await);
    }

    exec(&db, "CREATE INDEX idx_n ON t(n)").await;

    for (sql, saved) in statements.iter().zip(&before) {
        assert_same_rows(saved, &query(&db, sql).await, sql);
    }

    for sql in &statements[..8] {
        let plan = explain(&db, sql).await;
        assert_int_range(&plan, "idx_n");
    }
    let projected = explain(&db, statements[7]).await;
    assert!(
        projected.contains("columns=[n]"),
        "single-column projection must name that output column, got: {projected}"
    );

    let negatives = query(&db, "SELECT n FROM t WHERE n > 0").await;
    assert!(
        negatives
            .rows
            .iter()
            .all(|row| matches!(row[0], Value::Int(n) if n > 0)),
        "n > 0 must not return negatives, got {:?}",
        negatives.rows
    );
    let below = query(&db, "SELECT n FROM t WHERE n < 0").await;
    assert!(
        below
            .rows
            .iter()
            .all(|row| matches!(row[0], Value::Int(n) if n < 0)),
        "n < 0 must only return negatives, got {:?}",
        below.rows
    );

    let equality = explain(&db, "SELECT id, n FROM t WHERE n = -1").await;
    assert!(
        !equality.contains("IndexScanAll") && !equality.contains("IndexRangeScan"),
        "unary negation on equality stays a scan, got: {equality}"
    );
}

/// 世代 0 的单列整数索引不做范围路由，等值仍然点名该索引。
#[tokio::test]
async fn legacy_int_range_stays_scan() {
    use rtsql::executor::{CreateIndexExecutor, CreateIndexNode, Executor, PhysicalPlan};
    use std::sync::Arc;

    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(&db_path(&dir, "legacy")).await.unwrap());
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    for (id, n) in [(1, -4), (2, 0), (3, 2), (4, 5)] {
        exec(&db, &format!("INSERT INTO t VALUES ({id}, {n})")).await;
    }
    let before_range = query(&db, "SELECT id, n FROM t WHERE n > 0").await;
    let before_eq = query(&db, "SELECT id, n FROM t WHERE n = 2").await;

    let meta = db.get_table("t").await.unwrap();
    let column_index = meta
        .columns
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case("n"))
        .expect("n");
    let plan = PhysicalPlan::CreateIndex(CreateIndexNode {
        index_name: "idx_old".into(),
        table_name: "t".into(),
        column_index,
        column_name: "n".into(),
        generation: 0,
        extra_column_indexes: Vec::new(),
    });
    let mut creator = CreateIndexExecutor::new(plan, db.clone());
    assert!(creator.next().await.unwrap().is_some());
    assert!(creator.next().await.unwrap().is_none());

    assert_same_rows(
        &before_range,
        &query(&db, "SELECT id, n FROM t WHERE n > 0").await,
        "legacy n > 0",
    );
    let range_plan = explain(&db, "SELECT id, n FROM t WHERE n > 0").await;
    assert!(
        !range_plan.contains("IndexRangeScan"),
        "generation 0 integer range stays a scan, got: {range_plan}"
    );
    assert_same_rows(
        &before_eq,
        &query(&db, "SELECT id, n FROM t WHERE n = 2").await,
        "legacy n = 2",
    );
    let equality = explain(&db, "SELECT id, n FROM t WHERE n = 2").await;
    assert!(
        equality.contains("IndexScanAll") && equality.contains("index=idx_old"),
        "generation 0 integer equality still names the index, got: {equality}"
    );
}

/// 全整数组合前缀的端点含 `(2, i64::MIN)`、不含 `(1, i64::MAX)` 的越界行。
#[tokio::test]
async fn composite_prefix_bounds() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "prefix")).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 1, 9223372036854775807)").await;
    exec(&db, "INSERT INTO t VALUES (2, 2, 0)").await;
    exec(
        &db,
        "UPDATE t SET b = 0 - 9223372036854775807 - 1 WHERE id = 2",
    )
    .await;
    let stored = query(&db, "SELECT b FROM t WHERE id = 2").await;
    assert_eq!(stored.rows, vec![vec![Value::Int(i64::MIN)]]);

    let statements = [
        "SELECT id, a, b FROM t WHERE a > 1",
        "SELECT id, a, b FROM t WHERE a = 1",
        "SELECT id, a, b FROM t WHERE a = 1 AND b > 2",
        "SELECT id, a, b FROM t WHERE b > 2 AND a = 1",
    ];
    let mut before = Vec::new();
    for sql in statements {
        before.push(query(&db, sql).await);
    }
    exec(&db, "CREATE INDEX idx_ab ON t(a, b)").await;
    for (sql, saved) in statements.iter().zip(&before) {
        assert_same_rows(saved, &query(&db, sql).await, sql);
        let plan = explain(&db, sql).await;
        assert_int_range(&plan, "idx_ab");
    }

    let above = query(&db, "SELECT id FROM t WHERE a > 1").await;
    assert_eq!(above.rows, vec![vec![Value::Int(2)]]);
    let equal = query(&db, "SELECT id FROM t WHERE a = 1").await;
    assert_eq!(equal.rows, vec![vec![Value::Int(1)]]);
    let tighter = query(&db, "SELECT id FROM t WHERE a = 1 AND b > 2").await;
    assert_eq!(tighter.rows, vec![vec![Value::Int(1)]]);
}

/// 组合整键等值点名索引；放不进 32 字节的整键改走扫描，仍返回真实相等的行。
#[tokio::test]
async fn composite_equality_names_index() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "eq")).await.unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, a INT, name VARCHAR)",
    )
    .await;
    let long = "x".repeat(40);
    exec(&db, "INSERT INTO t VALUES (1, 1, 'ok')").await;
    exec(&db, "INSERT INTO t VALUES (2, 1, 'ok')").await;
    exec(&db, &format!("INSERT INTO t VALUES (3, 1, '{}')", long)).await;
    exec(&db, "INSERT INTO t VALUES (4, 2, 'ok')").await;

    let fitting = [
        "SELECT id, a, name FROM t WHERE a = 1 AND name = 'ok'",
        "SELECT id, a, name FROM t WHERE name = 'ok' AND a = 1",
    ];
    let overlong = format!("SELECT id, a, name FROM t WHERE a = 1 AND name = '{long}'");
    let overlong_swapped = format!("SELECT id, a, name FROM t WHERE name = '{long}' AND a = 1");
    let mut before_fitting = Vec::new();
    for sql in fitting {
        before_fitting.push(query(&db, sql).await);
    }
    let before_overlong = query(&db, &overlong).await;
    let before_swapped = query(&db, &overlong_swapped).await;

    exec(&db, "CREATE INDEX idx_a_name ON t(a, name)").await;
    for (sql, saved) in fitting.iter().zip(&before_fitting) {
        assert_same_rows(saved, &query(&db, sql).await, sql);
        let plan = explain(&db, sql).await;
        assert!(
            plan.contains("IndexScanAll") && plan.contains("index=idx_a_name"),
            "full-key equality must name idx_a_name, got: {plan}"
        );
    }
    for sql in [&overlong, &overlong_swapped] {
        let plan = explain(&db, sql).await;
        assert!(
            !plan.contains("IndexScanAll"),
            "an overlong composite literal is not IndexScanAll, got: {plan}"
        );
    }
    assert_same_rows(
        &before_overlong,
        &query(&db, &overlong).await,
        "overlong equality",
    );
    assert_same_rows(
        &before_swapped,
        &query(&db, &overlong_swapped).await,
        "swapped overlong equality",
    );
    assert_eq!(
        query(&db, &overlong).await.rows.len(),
        1,
        "the overlong row is still returned"
    );
}

/// 字符串范围补上超长且谓词成立的行，NULL 不凭补扫返回。
#[tokio::test]
async fn string_range_includes_overlong() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "str")).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    let long = format!("abc{}", "d".repeat(40));
    exec(&db, "INSERT INTO t VALUES (1, 'abc')").await;
    exec(&db, &format!("INSERT INTO t VALUES (2, '{long}')")).await;
    exec(&db, "INSERT INTO t VALUES (3, 'a')").await;
    exec(&db, "INSERT INTO t VALUES (4, NULL)").await;

    let sql = "SELECT id, name FROM t WHERE name > 'ab'";
    let before = query(&db, sql).await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    let after = query(&db, sql).await;
    assert_same_rows(&before, &after, sql);
    let ids: Vec<i64> = after
        .rows
        .iter()
        .map(|row| match row[0] {
            Value::Int(id) => id,
            _ => panic!("id"),
        })
        .collect();
    assert!(
        ids.contains(&1) && ids.contains(&2),
        "short and overlong rows, got {ids:?}"
    );
    assert!(
        !ids.contains(&3) && !ids.contains(&4),
        "excluded rows, got {ids:?}"
    );
    let plan = explain(&db, sql).await;
    assert!(
        plan.contains("IndexRangeScan")
            && plan.contains("index=idx_name")
            && plan.contains("omitted=scan"),
        "string range rescans omitted keys, got: {plan}"
    );
}

/// 非前缀、析取、额外谓词和不可路由形态保持扫描。矛盾区间仍是空的范围扫描。
#[tokio::test]
async fn unrouted_predicates_stay_scan() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "unrouted")).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 1, 3)").await;
    exec(&db, "INSERT INTO t VALUES (2, 4, 1)").await;
    exec(&db, "INSERT INTO t VALUES (3, NULL, 1)").await;

    let scans = [
        "SELECT id, a, b FROM t WHERE b > 2",
        "SELECT id, a, b FROM t WHERE a > 1 OR b > 2",
        "SELECT id, a, b FROM t WHERE a > 1 AND id = 1",
        "SELECT id, a, b FROM t WHERE a NOT BETWEEN 0 AND 9",
        "SELECT id, a, b FROM t WHERE t.a > 1",
        "SELECT id, a, b FROM t WHERE a = NULL",
        "SELECT id, a, b FROM t WHERE a = 'x'",
        "SELECT id, a, b FROM t WHERE a = ?",
    ];
    let mut before = Vec::new();
    let mut plans_before = Vec::new();
    for sql in scans {
        before.push(db.execute_sql(sql).await);
        plans_before.push(db.execute_sql(&format!("EXPLAIN {sql}")).await);
    }
    let empty_before = query(&db, "SELECT id FROM t WHERE a > 5 AND a < 1").await;

    exec(&db, "CREATE INDEX idx_ab ON t(a, b)").await;
    for (sql, (saved, saved_plan)) in scans.iter().zip(before.iter().zip(&plans_before)) {
        let after = db.execute_sql(sql).await;
        match (saved, &after) {
            (
                Ok(SqlOutcome::Rows { columns, rows }),
                Ok(SqlOutcome::Rows {
                    columns: columns_after,
                    rows: rows_after,
                }),
            ) => {
                assert_same_rows(
                    &Query {
                        columns: columns.clone(),
                        rows: rows.clone(),
                    },
                    &Query {
                        columns: columns_after.clone(),
                        rows: rows_after.clone(),
                    },
                    sql,
                );
            }
            (Err(left), Err(right)) => assert_eq!(left, right, "{sql}"),
            _ => panic!("{sql}: outcome changed from {saved:?} to {after:?}"),
        }
        let plan = db.execute_sql(&format!("EXPLAIN {sql}")).await;
        match (saved_plan, &plan) {
            (
                Ok(SqlOutcome::Rows { rows, .. }),
                Ok(SqlOutcome::Rows {
                    rows: rows_after, ..
                }),
            ) => {
                let text = rows
                    .iter()
                    .chain(rows_after.iter())
                    .map(|row| match &row[0] {
                        Value::String(text) => text.clone(),
                        other => panic!("plan cell must be a string, got {other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    !text.contains("IndexScanAll") && !text.contains("IndexRangeScan"),
                    "{sql} must stay a scan, got: {text}"
                );
            }
            (Err(left), Err(right)) => assert_eq!(left, right, "EXPLAIN {sql}"),
            _ => panic!("{sql}: explain outcome changed from {saved_plan:?} to {plan:?}"),
        }
    }

    let empty_sql = "SELECT id FROM t WHERE a > 5 AND a < 1";
    assert_same_rows(&empty_before, &query(&db, empty_sql).await, empty_sql);
    assert!(query(&db, empty_sql).await.rows.is_empty());
    let empty_plan = explain(&db, empty_sql).await;
    assert_int_range(&empty_plan, "idx_ab");
}

/// 前缀更长者优先；同样长则不补扫者优先；再同样则创建更早者优先。
#[tokio::test]
async fn index_selection_prefers_longer_then_exact_then_earlier() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "pick")).await.unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT, name VARCHAR)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 1, 2, 'n')").await;
    exec(&db, "INSERT INTO t VALUES (2, 1, 9, 'n')").await;
    exec(&db, "INSERT INTO t VALUES (3, 4, 2, 'n')").await;

    let statements = [
        "SELECT id FROM t WHERE a = 1 AND b = 2 AND name = 'n'",
        "SELECT id FROM t WHERE a = 1 AND b = 2",
        "SELECT id FROM t WHERE a = 1",
        "SELECT id FROM t WHERE a > 0",
    ];
    let mut before = Vec::new();
    for sql in statements {
        before.push(query(&db, sql).await);
    }
    exec(&db, "CREATE INDEX idx_wide ON t(a, b, name)").await;
    exec(&db, "CREATE INDEX idx_named ON t(a, name)").await;
    exec(&db, "CREATE INDEX idx_early ON t(a)").await;
    exec(&db, "CREATE INDEX idx_late ON t(a)").await;
    exec(&db, "CREATE INDEX idx_pair ON t(a, b)").await;
    for (sql, saved) in statements.iter().zip(&before) {
        assert_same_rows(saved, &query(&db, sql).await, sql);
    }

    let wide = explain(&db, statements[0]).await;
    assert!(
        wide.contains("IndexScanAll") && wide.contains("index=idx_wide"),
        "the longer prefix wins, got: {wide}"
    );
    let pair = explain(&db, statements[1]).await;
    assert!(
        pair.contains("IndexScanAll") && pair.contains("index=idx_pair"),
        "same prefix length prefers the index that does not rescan, got: {pair}"
    );
    assert!(
        !pair.contains("omitted=scan") && !pair.contains("index=idx_wide"),
        "the earlier string index must not win, got: {pair}"
    );
    let early = explain(&db, statements[2]).await;
    assert!(
        early.contains("IndexScanAll") && early.contains("index=idx_early"),
        "same length and no rescan prefers the earlier index, got: {early}"
    );
    assert!(
        !early.contains("index=idx_late"),
        "the later duplicate must not win, got: {early}"
    );
    let range = explain(&db, statements[3]).await;
    assert_int_range(&range, "idx_early");
}
