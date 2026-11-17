//! 合格查询按估计更换左深叶序，EXPLAIN 因此多一行 `JoinOrder`。

use rtsql::database::Database;
use rtsql::{SqlOutcome, Value};

const STAR_FROM: &str = "FROM d1 JOIN fact ON d1.id = fact.d1 JOIN d2 ON fact.d2 = d2.id";

fn star_select() -> String {
    format!("SELECT fact.id, d1.id, d2.id {STAR_FROM} ORDER BY fact.id, d1.id, d2.id")
}

async fn open_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str) {
    db.execute_sql(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql} failed: {error}"));
}

async fn seed_star(db: &Database) {
    exec_ok(db, "CREATE TABLE d1 (id INT)").await;
    exec_ok(db, "CREATE TABLE d2 (id INT)").await;
    exec_ok(db, "CREATE TABLE fact (id INT, d1 INT, d2 INT)").await;
    exec_ok(db, "INSERT INTO d1 VALUES (1)").await;
    exec_ok(db, "INSERT INTO d1 VALUES (2)").await;
    exec_ok(db, "INSERT INTO d2 VALUES (1)").await;
    exec_ok(db, "INSERT INTO d2 VALUES (2)").await;
    for id in 1..=100 {
        let key = (id - 1) % 2 + 1;
        exec_ok(
            db,
            &format!("INSERT INTO fact VALUES ({id}, {key}, {key})"),
        )
        .await;
    }
}

async fn analyze_star(db: &Database) {
    exec_ok(db, "ANALYZE TABLE d1").await;
    exec_ok(db, "ANALYZE TABLE d2").await;
    exec_ok(db, "ANALYZE TABLE fact").await;
}

async fn explain_lines(db: &Database, sql: &str) -> Vec<String> {
    text_rows(db.execute_sql(sql).await.unwrap())
}

fn text_rows(outcome: SqlOutcome) -> Vec<String> {
    match outcome {
        SqlOutcome::Rows { rows, .. } => rows
            .into_iter()
            .map(|row| match &row[0] {
                Value::String(text) => text.clone(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect(),
        other => panic!("expected explain rows, got {other:?}"),
    }
}

fn rows_of(outcome: SqlOutcome) -> (Vec<String>, Vec<Vec<Value>>) {
    match outcome {
        SqlOutcome::Rows { columns, rows } => (
            columns.into_iter().map(|column| column.name).collect(),
            rows,
        ),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn trimmed<'a>(lines: &'a [String]) -> Vec<&'a str> {
    lines.iter().map(|line| line.trim_start()).collect()
}

fn first_scan(lines: &[String]) -> String {
    trimmed(lines)
        .into_iter()
        .find(|line| line.starts_with("Scan "))
        .expect("explain has a scan")
        .to_string()
}

fn assert_join_order(lines: &[String], label: &str) {
    assert!(
        trimmed(lines).iter().any(|line| *line == label),
        "missing {label} in {lines:?}"
    );
}

fn assert_written_left(lines: &[String], scan_prefix: &str) {
    assert!(
        !lines.iter().any(|line| line.contains("JoinOrder")),
        "unexpected JoinOrder in {lines:?}"
    );
    let scan = first_scan(lines);
    assert!(
        scan.starts_with(scan_prefix),
        "first scan should start with {scan_prefix}, got {scan}"
    );
}

#[tokio::test]
async fn star_join_explain_puts_fact_first() {
    let (db, _dir) = open_db().await;
    seed_star(&db).await;
    analyze_star(&db).await;
    let lines = explain_lines(&db, &format!("EXPLAIN {}", star_select())).await;
    let scan = first_scan(&lines);
    assert!(
        scan.starts_with("Scan fact"),
        "first scan should be fact, got {scan}"
    );
    assert_join_order(&lines, "JoinOrder estimated");
}

#[tokio::test]
async fn star_results_match_the_unanalyzed_session() {
    let (db, _dir) = open_db().await;
    seed_star(&db).await;
    let select_star = format!("SELECT * {STAR_FROM} ORDER BY fact.id");
    let filtered = format!(
        "SELECT fact.id, d1.id, d2.id {STAR_FROM} WHERE fact.id = 1 ORDER BY fact.id"
    );
    let grouped = format!(
        "SELECT fact.d1, COUNT(*) {STAR_FROM} GROUP BY fact.d1 ORDER BY fact.d1"
    );
    let limited = format!("SELECT fact.id, d1.id, d2.id {STAR_FROM} ORDER BY fact.id LIMIT 1");
    let before_star = rows_of(db.execute_sql(&select_star).await.unwrap());
    let before_where = rows_of(db.execute_sql(&filtered).await.unwrap());
    let before_group = rows_of(db.execute_sql(&grouped).await.unwrap());
    let before_limit = rows_of(db.execute_sql(&limited).await.unwrap());
    analyze_star(&db).await;
    let after_star = rows_of(db.execute_sql(&select_star).await.unwrap());
    let after_where = rows_of(db.execute_sql(&filtered).await.unwrap());
    let after_group = rows_of(db.execute_sql(&grouped).await.unwrap());
    let after_limit = rows_of(db.execute_sql(&limited).await.unwrap());
    assert_eq!(after_star, before_star);
    assert_eq!(after_where.1.len(), 1);
    assert_eq!(after_where, before_where);
    assert_eq!(after_group, before_group);
    assert_eq!(after_limit, before_limit);
    let lines = explain_lines(&db, &format!("EXPLAIN {limited}")).await;
    assert_written_left(&lines, "Scan d1");
}

#[tokio::test]
async fn ineligible_queries_keep_the_written_leaf() {
    let (db, _dir) = open_db().await;
    seed_star(&db).await;
    analyze_star(&db).await;

    let no_order = format!("EXPLAIN SELECT fact.id, d1.id, d2.id {STAR_FROM}");
    assert_written_left(&explain_lines(&db, &no_order).await, "Scan d1");

    let two_tables =
        "EXPLAIN SELECT fact.id, d1.id FROM d1 JOIN fact ON d1.id = fact.d1 ORDER BY fact.id";
    assert_written_left(&explain_lines(&db, two_tables).await, "Scan d1");

    let outer = format!(
        "EXPLAIN SELECT fact.id FROM d1 LEFT JOIN fact ON d1.id = fact.d1 \
         JOIN d2 ON fact.d2 = d2.id ORDER BY fact.id"
    );
    let lines = explain_lines(&db, &outer).await;
    assert_written_left(&lines, "Scan d1");
    assert!(
        lines.iter().any(|line| line.contains("NestedLoopJoin")),
        "outer join should keep NestedLoopJoin, got {lines:?}"
    );
}

#[tokio::test]
async fn equal_estimated_cost_keeps_the_written_leaf() {
    let (db, _dir) = open_db().await;
    for name in ["a", "b", "c"] {
        exec_ok(&db, &format!("CREATE TABLE {name} (id INT)")).await;
        for id in 1..=10 {
            exec_ok(&db, &format!("INSERT INTO {name} VALUES ({id})")).await;
        }
    }
    for name in ["a", "b", "c"] {
        exec_ok(&db, &format!("ANALYZE TABLE {name}")).await;
    }
    let lines = explain_lines(
        &db,
        "EXPLAIN SELECT * FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id ORDER BY a.id",
    )
    .await;
    assert_join_order(&lines, "JoinOrder estimated");
    let scan = first_scan(&lines);
    assert!(
        scan.starts_with("Scan a"),
        "first scan should stay a, got {scan}"
    );
}

#[tokio::test]
async fn nine_table_chain_reports_written_order() {
    let (db, _dir) = open_db().await;
    for index in 1..=9 {
        exec_ok(&db, &format!("CREATE TABLE t{index} (id INT)")).await;
        exec_ok(&db, &format!("INSERT INTO t{index} VALUES (1)")).await;
    }
    for index in 1..=9 {
        exec_ok(&db, &format!("ANALYZE TABLE t{index}")).await;
    }
    let mut sql = String::from("EXPLAIN SELECT * FROM t1");
    for index in 2..=9 {
        let prev = index - 1;
        sql.push_str(&format!(" JOIN t{index} ON t{prev}.id = t{index}.id"));
    }
    sql.push_str(" ORDER BY t1.id");
    let lines = explain_lines(&db, &sql).await;
    assert_join_order(&lines, "JoinOrder written");
    let scan = first_scan(&lines);
    assert!(
        scan.starts_with("Scan t1 ("),
        "first scan should stay t1, got {scan}"
    );
}

#[tokio::test]
async fn prepared_star_matches_direct_execution() {
    let (db, _dir) = open_db().await;
    seed_star(&db).await;
    let sql = star_select();
    let prepared = db.prepare(&sql).await.unwrap();
    let before = db.schema_version();
    analyze_star(&db).await;
    assert!(db.schema_version() > before);
    let prepared_rows = rows_of(prepared.execute(&[]).await.unwrap());
    let direct_rows = rows_of(db.execute_sql(&sql).await.unwrap());
    assert_eq!(prepared_rows.1.len(), 100);
    assert_eq!(prepared_rows, direct_rows);
}

#[tokio::test]
async fn create_index_removes_the_join_order_line() {
    let (db, _dir) = open_db().await;
    seed_star(&db).await;
    analyze_star(&db).await;
    let sql = format!("EXPLAIN {}", star_select());
    let before = explain_lines(&db, &sql).await;
    assert_join_order(&before, "JoinOrder estimated");
    exec_ok(&db, "CREATE INDEX fact_id ON fact(id)").await;
    let after = explain_lines(&db, &sql).await;
    assert_written_left(&after, "Scan d1");
}

#[tokio::test]
async fn reopen_removes_the_join_order_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let sql = format!("EXPLAIN {}", star_select());
    {
        let db = Database::open(&path).await.unwrap();
        seed_star(&db).await;
        analyze_star(&db).await;
        let before = explain_lines(&db, &sql).await;
        assert_join_order(&before, "JoinOrder estimated");
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    let after = explain_lines(&db, &sql).await;
    assert_written_left(&after, "Scan d1");
}
