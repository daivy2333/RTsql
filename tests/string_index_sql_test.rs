//! SQL `CREATE INDEX` on string columns, and equality queries that use them.

use rtsql::database::Database;
use rtsql::storage::ColumnType;
use rtsql::{ColumnMeta, SqlError, SqlOutcome, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

async fn open_db() -> (Database, TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    (db, dir)
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

fn assert_same_query(before: &Query, after: &Query, what: &str) {
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

fn assert_named_index(plan: &str, index: &str) {
    assert!(
        plan.contains("IndexScanAll") && plan.contains(&format!("index={index}")),
        "plan must name {index} on IndexScanAll, got: {plan}"
    );
}

fn assert_data_scan(plan: &str) {
    assert!(
        plan.contains("DataScan") && !plan.contains("IndexScanAll"),
        "plan must scan without IndexScanAll, got: {plan}"
    );
}

fn sql_quote(text: &str) -> String {
    format!("'{text}'")
}

async fn equality(db: &Database, column: &str, value: &str) -> Query {
    query(
        db,
        &format!(
            "SELECT id, name, note, age FROM t WHERE {column} = {}",
            sql_quote(value)
        ),
    )
    .await
}

async fn outcome(db: &Database, sql: &str) -> Result<Query, SqlError> {
    match db.execute_sql(sql).await {
        Ok(SqlOutcome::Rows { columns, rows }) => Ok(Query { columns, rows }),
        Ok(other) => panic!("{sql}: expected rows, got {other:?}"),
        Err(error) => Err(error),
    }
}

fn assert_same_outcome(
    before: &Result<Query, SqlError>,
    after: &Result<Query, SqlError>,
    what: &str,
) {
    match (before, after) {
        (Ok(left), Ok(right)) => assert_same_query(left, right, what),
        (Err(left), Err(right)) => assert_eq!(left, right, "{what}"),
        _ => panic!("{what}: outcome changed from {before:?} to {after:?}"),
    }
}

async fn explain_outcome(db: &Database, sql: &str) -> Result<String, SqlError> {
    match db.execute_sql(&format!("EXPLAIN {sql}")).await {
        Ok(SqlOutcome::Rows { rows, .. }) => Ok(rows
            .iter()
            .map(|row| match &row[0] {
                Value::String(text) => text.clone(),
                other => panic!("plan cell must be a string, got {other:?}"),
            })
            .collect::<Vec<_>>()
            .join("\n")),
        Ok(other) => panic!("EXPLAIN {sql}: expected rows, got {other:?}"),
        Err(error) => Err(error),
    }
}

async fn stored_id_name(db: &Database, value: &str) -> Vec<Vec<Value>> {
    let all = query(db, "SELECT id, name FROM t").await;
    let mut rows = all
        .rows
        .into_iter()
        .filter(|row| row[1] == Value::String(value.to_string()))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| match (&left[0], &right[0]) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        _ => panic!("id column must be an integer"),
    });
    rows
}

fn unsupported_column_message(column: &str) -> String {
    format!(
        "Plan error: Unsupported statement type: INDEX ON UNSUPPORTED COLUMN '{column}' (only INT and string columns)"
    )
}

fn run_cli(home: &Path, name: &str, sql: &str) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args([name, sql, "--format", "json"])
        .current_dir(home)
        .env("RTSQL_HOME", home)
        .env_remove("RTSQL_KEY")
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|error| panic!("spawn rtsql: {error}"));
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8(output.stdout).expect("stdout utf-8"),
        String::from_utf8(output.stderr).expect("stderr utf-8"),
    )
}

fn cli_ok(home: &Path, name: &str, sql: &str) -> String {
    let (code, stdout, stderr) = run_cli(home, name, sql);
    assert_eq!(code, 0, "{sql} failed\nstdout: {stdout}\nstderr: {stderr}");
    stdout
}

fn json_query(stdout: &str) -> (serde_json::Value, Vec<serde_json::Value>) {
    let doc: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("query json: {error}: {stdout}"));
    let mut rows = doc["rows"].as_array().expect("rows").clone();
    rows.sort_by(|left, right| {
        left[0]
            .as_i64()
            .expect("id")
            .cmp(&right[0].as_i64().expect("id"))
    });
    (doc["columns"].clone(), rows)
}

/// VARCHAR 与 TEXT 建索引前后，等值行、列名和声明类型一致，两侧等值都走索引。
/// 同一张表上的 INT 列 `age = 25` 仍走既有索引。
#[tokio::test]
async fn string_index_equality_matches_scan_multiset_and_columns() {
    let (db, _dir) = open_db().await;
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR, note TEXT, age INT)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 'dup', 'dup', 25)").await;
    exec(&db, "INSERT INTO t VALUES (2, 'dup', 'other', 25)").await;
    exec(&db, "INSERT INTO t VALUES (3, '', '你好', 30)").await;
    exec(&db, "INSERT INTO t VALUES (4, '你好', '', 25)").await;
    exec(&db, "INSERT INTO t VALUES (5, 'ab\0', 'pre-a', 1)").await;
    exec(&db, "INSERT INTO t VALUES (6, 'pre-a', 'pre-b', 25)").await;
    exec(&db, "INSERT INTO t VALUES (7, 'pre-b', 'ab\0', 40)").await;
    exec(&db, "INSERT INTO t VALUES (8, 'solo', 'solo', 25)").await;

    let name_values = ["dup", "", "你好", "ab\0", "pre-a", "pre-b", "solo"];
    let note_values = ["dup", "other", "你好", "", "pre-a", "pre-b", "ab\0", "solo"];
    let mut name_before = Vec::new();
    for value in name_values {
        name_before.push(equality(&db, "name", value).await);
    }
    let mut note_before = Vec::new();
    for value in note_values {
        note_before.push(equality(&db, "note", value).await);
    }
    let age_before = query(&db, "SELECT id, name, note, age FROM t WHERE age = 25").await;

    let declared = &name_before[0];
    assert_eq!(
        declared
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["id", "name", "note", "age"]
    );
    assert!(
        matches!(declared.columns[1].column_type, Some(ColumnType::String(_)))
            && matches!(declared.columns[2].column_type, Some(ColumnType::String(_))),
        "VARCHAR and TEXT must be stored as String(_), got {:?}",
        declared.columns
    );
    assert_eq!(declared.rows.len(), 2, "duplicate name rows");

    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    exec(&db, "CREATE INDEX idx_note ON t(note)").await;

    for (value, before) in name_values.iter().zip(&name_before) {
        let after = equality(&db, "name", value).await;
        assert_same_query(before, &after, value);
        for row in &after.rows {
            assert_eq!(&row[1], &Value::String((*value).to_string()));
        }
        let plan = explain(
            &db,
            &format!(
                "SELECT id, name, note, age FROM t WHERE name = {}",
                sql_quote(value)
            ),
        )
        .await;
        assert_named_index(&plan, "idx_name");
    }
    let reversed = query(&db, "SELECT id, name, note, age FROM t WHERE 'dup' = name").await;
    assert_same_query(&name_before[0], &reversed, "literal = name");
    let reversed_plan = explain(&db, "SELECT id, name, note, age FROM t WHERE 'dup' = name").await;
    assert_named_index(&reversed_plan, "idx_name");

    let pre_a = equality(&db, "name", "pre-a").await;
    assert!(
        pre_a
            .rows
            .iter()
            .all(|row| row[1] != Value::String("pre-b".into())),
        "pre-a must not return pre-b"
    );
    let nul = equality(&db, "name", "ab\0").await;
    assert!(
        nul.rows
            .iter()
            .all(|row| row[1] != Value::String("ab".into())),
        "ab\\0 must not return ab"
    );

    for (value, before) in note_values.iter().zip(&note_before) {
        let after = equality(&db, "note", value).await;
        assert_same_query(before, &after, value);
        for row in &after.rows {
            assert_eq!(&row[2], &Value::String((*value).to_string()));
        }
        let plan = explain(
            &db,
            &format!(
                "SELECT id, name, note, age FROM t WHERE note = {}",
                sql_quote(value)
            ),
        )
        .await;
        assert_named_index(&plan, "idx_note");
    }

    exec(&db, "CREATE INDEX idx_age ON t(age)").await;
    let age_after = query(&db, "SELECT id, name, note, age FROM t WHERE age = 25").await;
    assert_same_query(&age_before, &age_after, "age = 25");
    let age_reversed = query(&db, "SELECT id, name, note, age FROM t WHERE 25 = age").await;
    assert_same_query(&age_before, &age_reversed, "25 = age");
    assert_named_index(
        &explain(&db, "SELECT id, name, note, age FROM t WHERE age = 25").await,
        "idx_age",
    );
    assert_named_index(
        &explain(&db, "SELECT id, name, note, age FROM t WHERE 25 = age").await,
        "idx_age",
    );
}

/// 投影、聚合、省略 DEFAULT 列和派生查询的结果与建索引前一致，计划点名索引。
#[tokio::test]
async fn string_index_projection_aggregate_default_and_derived() {
    let (db, _dir) = open_db().await;
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR, age INT)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 'dup', 25)").await;
    exec(&db, "INSERT INTO t VALUES (2, 'dup', 30)").await;
    exec(&db, "INSERT INTO t VALUES (3, 'other', 25)").await;
    exec(
        &db,
        "CREATE TABLE defs (id INT PRIMARY KEY, name VARCHAR DEFAULT 'def')",
    )
    .await;
    exec(&db, "INSERT INTO defs (id) VALUES (1)").await;

    let statements = [
        "SELECT name, id FROM t WHERE name = 'dup'",
        "SELECT name FROM t WHERE name = 'dup'",
        "SELECT * FROM t WHERE name = 'dup'",
        "SELECT COUNT(*) FROM t WHERE name = 'dup'",
    ];
    let mut before = Vec::new();
    for sql in statements {
        before.push(query(&db, sql).await);
    }
    let default_before = query(&db, "SELECT id, name FROM defs WHERE name = 'def'").await;
    let direct_before = query(&db, "SELECT id, name FROM t WHERE name = 'dup'").await;
    let derived_sql = "SELECT id, name FROM (SELECT id, name FROM t WHERE name = 'dup') d";
    let derived_before = query(&db, derived_sql).await;
    assert_same_query(&direct_before, &derived_before, "derived before index");
    assert_eq!(before[3].rows, vec![vec![Value::Int(2)]]);

    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    exec(&db, "CREATE INDEX idx_def ON defs(name)").await;

    for (sql, saved) in statements.iter().zip(&before) {
        assert_same_query(saved, &query(&db, sql).await, sql);
    }
    assert_named_index(
        &explain(&db, "SELECT COUNT(*) FROM t WHERE name = 'dup'").await,
        "idx_name",
    );
    assert_same_query(
        &default_before,
        &query(&db, "SELECT id, name FROM defs WHERE name = 'def'").await,
        "default",
    );
    assert_named_index(
        &explain(&db, "SELECT id, name FROM defs WHERE name = 'def'").await,
        "idx_def",
    );
    let derived_after = query(&db, derived_sql).await;
    assert_same_query(&derived_before, &derived_after, "derived");
    assert_same_query(
        &direct_before,
        &query(&db, "SELECT id, name FROM t WHERE name = 'dup'").await,
        "direct",
    );
    assert_named_index(&explain(&db, derived_sql).await, "idx_name");
}

/// 共享前缀、尾部零字节、30 字节索引和 31 字节扫描，以及跨过容量边界的更新。
#[tokio::test]
async fn string_index_prefix_nul_capacity_and_cross_boundary_update() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    let inside = "a".repeat(30);
    let outside_b = format!("{}b", "a".repeat(30));
    let outside_c = format!("{}c", "a".repeat(30));
    exec(&db, "INSERT INTO t VALUES (1, 'pre-a')").await;
    exec(&db, "INSERT INTO t VALUES (2, 'pre-b')").await;
    exec(&db, "INSERT INTO t VALUES (3, 'ab\0')").await;
    exec(&db, &format!("INSERT INTO t VALUES (4, '{inside}')")).await;
    exec(&db, &format!("INSERT INTO t VALUES (5, '{outside_b}')")).await;
    exec(&db, &format!("INSERT INTO t VALUES (6, '{outside_c}')")).await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;

    for value in ["pre-a", "pre-b", "ab\0", inside.as_str()] {
        let sql = format!("SELECT id, name FROM t WHERE name = {}", sql_quote(value));
        let found = query(&db, &sql).await;
        assert_eq!(
            sorted_rows(&found.columns, found.rows),
            stored_id_name(&db, value).await,
            "{value:?}"
        );
        assert_named_index(&explain(&db, &sql).await, "idx_name");
    }
    let pre_a = query(&db, "SELECT id, name FROM t WHERE name = 'pre-a'").await;
    assert!(pre_a
        .rows
        .iter()
        .all(|row| row[1] != Value::String("pre-b".into())));
    let nul = query(&db, "SELECT id, name FROM t WHERE name = 'ab\0'").await;
    assert!(nul
        .rows
        .iter()
        .all(|row| row[1] != Value::String("ab".into())));

    for value in [&outside_b, &outside_c] {
        let sql = format!("SELECT id, name FROM t WHERE name = {}", sql_quote(value));
        let found = query(&db, &sql).await;
        assert_eq!(
            sorted_rows(&found.columns, found.rows.clone()),
            stored_id_name(&db, value).await,
            "{value:?}"
        );
        assert_eq!(
            found.rows.len(),
            1,
            "{value:?} must not mix with the other long string"
        );
        assert_data_scan(&explain(&db, &sql).await);
    }

    exec(
        &db,
        &format!("UPDATE t SET name = '{outside_b}' WHERE id = 4"),
    )
    .await;
    for value in [&inside, &outside_b, &outside_c] {
        let sql = format!("SELECT id, name FROM t WHERE name = {}", sql_quote(value));
        let found = query(&db, &sql).await;
        assert_eq!(
            sorted_rows(&found.columns, found.rows),
            stored_id_name(&db, value).await,
            "after widening {value:?}"
        );
    }
    assert_named_index(
        &explain(
            &db,
            &format!("SELECT id, name FROM t WHERE name = {}", sql_quote(&inside)),
        )
        .await,
        "idx_name",
    );
    assert_data_scan(
        &explain(
            &db,
            &format!(
                "SELECT id, name FROM t WHERE name = {}",
                sql_quote(&outside_b)
            ),
        )
        .await,
    );

    exec(&db, &format!("UPDATE t SET name = '{inside}' WHERE id = 4")).await;
    for value in [&inside, &outside_b, &outside_c] {
        let sql = format!("SELECT id, name FROM t WHERE name = {}", sql_quote(value));
        let found = query(&db, &sql).await;
        assert_eq!(
            sorted_rows(&found.columns, found.rows),
            stored_id_name(&db, value).await,
            "after restoring {value:?}"
        );
    }
    assert_named_index(
        &explain(
            &db,
            &format!("SELECT id, name FROM t WHERE name = {}", sql_quote(&inside)),
        )
        .await,
        "idx_name",
    );
    assert_data_scan(
        &explain(
            &db,
            &format!(
                "SELECT id, name FROM t WHERE name = {}",
                sql_quote(&outside_c)
            ),
        )
        .await,
    );
}

/// 同一可容纳字符串的 200 行等值结果与建索引前相同，计划走索引，计数为 200。
#[tokio::test]
async fn string_index_multileaf_duplicates_match_scan() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    for id in 1..=200 {
        exec(&db, &format!("INSERT INTO t VALUES ({id}, 'dup')")).await;
    }
    let before = query(&db, "SELECT id, name FROM t WHERE name = 'dup'").await;
    let count_before = query(&db, "SELECT COUNT(*) FROM t WHERE name = 'dup'").await;
    assert_eq!(count_before.rows, vec![vec![Value::Int(200)]]);
    assert_eq!(before.rows.len(), 200);

    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    assert_same_query(
        &before,
        &query(&db, "SELECT id, name FROM t WHERE name = 'dup'").await,
        "200 duplicates",
    );
    assert_same_query(
        &count_before,
        &query(&db, "SELECT COUNT(*) FROM t WHERE name = 'dup'").await,
        "count",
    );
    assert_named_index(
        &explain(&db, "SELECT COUNT(*) FROM t WHERE name = 'dup'").await,
        "idx_name",
    );
    assert_named_index(
        &explain(&db, "SELECT id, name FROM t WHERE name = 'dup'").await,
        "idx_name",
    );
}

/// 删索引后等值结果和列元数据回到建索引前，EXPLAIN 回到 DataScan。
#[tokio::test]
async fn string_index_drop_restores_scan_metadata() {
    let (db, _dir) = open_db().await;
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR, note TEXT)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 'dup', 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (2, 'dup', 'other')").await;
    exec(&db, "INSERT INTO t VALUES (3, '', '你好')").await;
    let before = query(&db, "SELECT id, name, note FROM t WHERE name = 'dup'").await;
    assert!(matches!(
        before.columns[1].column_type,
        Some(ColumnType::String(_))
    ));

    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    assert_named_index(
        &explain(&db, "SELECT id, name, note FROM t WHERE name = 'dup'").await,
        "idx_name",
    );
    exec(&db, "DROP INDEX idx_name").await;

    assert_same_query(
        &before,
        &query(&db, "SELECT id, name, note FROM t WHERE name = 'dup'").await,
        "after drop",
    );
    assert_data_scan(&explain(&db, "SELECT id, name, note FROM t WHERE name = 'dup'").await);
}

/// NULL、类型不匹配、范围、AND、OR、限定列和参数不走索引，结果与建索引前相同。
#[tokio::test]
async fn string_index_unrouted_predicates_stay_scan() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "INSERT INTO t VALUES (1, 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (2, 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (3, 'pre-a')").await;
    exec(&db, "INSERT INTO t VALUES (4, '')").await;

    let statements = [
        "SELECT id, name FROM t WHERE name = NULL",
        "SELECT id, name FROM t WHERE name = 1",
        "SELECT id, name FROM t WHERE name > 'a'",
        "SELECT id, name FROM t WHERE name = 'dup' AND id = 1",
        "SELECT id, name FROM t WHERE name = 'dup' OR name = 'pre-a'",
        "SELECT id, name FROM t WHERE t.name = 'dup'",
        "SELECT id, name FROM t WHERE name = ?",
    ];
    let mut before = Vec::new();
    let mut plans_before = Vec::new();
    for sql in statements {
        before.push(outcome(&db, sql).await);
        plans_before.push(explain_outcome(&db, sql).await);
    }
    let params = [Value::String("dup".into())];
    let param_before = db
        .execute_with_params("SELECT id, name FROM t WHERE name = ?", &params)
        .await;

    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    for (sql, (saved, saved_plan)) in statements.iter().zip(before.iter().zip(&plans_before)) {
        assert_same_outcome(saved, &outcome(&db, sql).await, sql);
        let plan = explain_outcome(&db, sql).await;
        match (saved_plan, &plan) {
            (Ok(_), Ok(text)) if sql.contains("name > 'a'") => assert!(
                text.contains("IndexRangeScan")
                    && text.contains("index=idx_name")
                    && text.contains("omitted=scan"),
                "name > 'a' must range-scan idx_name and rescan omitted keys, got: {text}"
            ),
            (Ok(_), Ok(text)) => assert!(
                !text.contains("IndexScanAll") && !text.contains("IndexRangeScan"),
                "plan must not route, got: {text}"
            ),
            (Err(left), Err(right)) => assert_eq!(left, right, "{sql}"),
            _ => panic!("{sql}: explain outcome changed from {saved_plan:?} to {plan:?}"),
        }
    }
    let param_after = db
        .execute_with_params("SELECT id, name FROM t WHERE name = ?", &params)
        .await;
    match (&param_before, &param_after) {
        (Ok(left), Ok(right)) => match (left, right) {
            (
                SqlOutcome::Rows {
                    columns: left_columns,
                    rows: left_rows,
                },
                SqlOutcome::Rows {
                    columns: right_columns,
                    rows: right_rows,
                },
            ) => assert_same_query(
                &Query {
                    columns: left_columns.clone(),
                    rows: left_rows.clone(),
                },
                &Query {
                    columns: right_columns.clone(),
                    rows: right_rows.clone(),
                },
                "parameter",
            ),
            _ => panic!("parameter query must return rows: {left:?} {right:?}"),
        },
        _ => panic!("parameter query changed: {param_before:?} -> {param_after:?}"),
    }
}

/// BOOLEAN、FLOAT、UNIQUE 和重名被拒绝。整数与字符串组成的索引可以建立。
/// 只读与显式事务拒绝后仍能建索引。整数等值仍走索引。
#[tokio::test]
async fn string_index_rejection_readonly_and_transaction() {
    let (db, _dir) = open_db().await;
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR, age INT, flag BOOLEAN, score FLOAT)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 'a', 25, TRUE, 1.5)").await;
    exec(&db, "INSERT INTO t VALUES (2, 'b', 25, FALSE, 2.5)").await;
    exec(&db, "INSERT INTO t VALUES (3, 'a', 30, TRUE, 1.5)").await;
    let age_before = query(&db, "SELECT id, name, age FROM t WHERE age = 25").await;

    for (sql, column) in [
        ("CREATE INDEX idx_flag ON t(flag)", "flag"),
        ("CREATE INDEX idx_score ON t(score)", "score"),
    ] {
        let error = db.execute_sql(sql).await.expect_err(sql);
        assert_eq!(error.message, unsupported_column_message(column));
        assert_eq!(error.sqlstate.as_deref(), Some("0A000"));
    }

    exec(&db, "CREATE INDEX idx_age ON t(age)").await;
    assert_same_query(
        &age_before,
        &query(&db, "SELECT id, name, age FROM t WHERE age = 25").await,
        "age = 25",
    );
    assert_same_query(
        &age_before,
        &query(&db, "SELECT id, name, age FROM t WHERE 25 = age").await,
        "25 = age",
    );
    assert_named_index(
        &explain(&db, "SELECT id, name, age FROM t WHERE age = 25").await,
        "idx_age",
    );
    assert_named_index(
        &explain(&db, "SELECT id, name, age FROM t WHERE 25 = age").await,
        "idx_age",
    );

    let unique = db
        .execute_sql("CREATE UNIQUE INDEX idx_unique ON t(name)")
        .await
        .expect_err("unique");
    assert!(
        unique.message.contains("Unsupported statement type")
            && unique.message.contains("CREATE UNIQUE INDEX"),
        "{}",
        unique.message
    );
    exec(&db, "CREATE INDEX idx_multi ON t(name, age)").await;

    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    let duplicate = db
        .execute_sql("CREATE INDEX idx_name ON t(name)")
        .await
        .expect_err("duplicate");
    assert!(
        duplicate.message.contains("already exists"),
        "{}",
        duplicate.message
    );

    let ro_dir = tempdir().unwrap();
    let path = ro_dir.path().join("ro.db");
    {
        let writable = Database::open(&path).await.unwrap();
        exec(
            &writable,
            "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)",
        )
        .await;
        exec(&writable, "INSERT INTO t VALUES (1, 'dup')").await;
        writable.close().await.unwrap();
    }
    {
        let readonly = Database::open_readonly(&path).await.unwrap();
        let error = readonly
            .execute_sql("CREATE INDEX idx_name ON t(name)")
            .await
            .expect_err("readonly");
        assert_eq!(error.sqlstate.as_deref(), Some("25006"));
        assert!(
            error.message.contains("read-only session"),
            "{}",
            error.message
        );
        let meta = readonly.get_table("t").await.unwrap();
        assert!(meta.secondary_indexes.is_empty());
    }
    let reopened = Database::open(&path).await.unwrap();
    exec(&reopened, "CREATE INDEX idx_name ON t(name)").await;
    assert_named_index(
        &explain(&reopened, "SELECT id, name FROM t WHERE name = 'dup'").await,
        "idx_name",
    );

    exec(&db, "CREATE TABLE s (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "INSERT INTO s VALUES (1, 'dup')").await;
    let tx = db.begin().await.unwrap();
    let error = db
        .execute_in_tx("CREATE INDEX idx_s ON s(name)", &tx)
        .await
        .expect_err("transaction");
    assert_eq!(error.sqlstate.as_deref(), Some("0A000"));
    assert!(
        error.message.contains("inside a transaction"),
        "{}",
        error.message
    );
    db.rollback(tx).await.unwrap();
    exec(&db, "CREATE INDEX idx_s ON s(name)").await;
    assert_named_index(
        &explain(&db, "SELECT id, name FROM s WHERE name = 'dup'").await,
        "idx_s",
    );
}

/// CLI 在同一个库上建表、插入、建索引、查询、删索引，JSON 结果保持一致。
#[test]
fn cli_string_index_create_query_explain_and_drop() {
    let home = tempdir().unwrap();
    std::fs::create_dir(home.path().join("db")).unwrap();
    let name = "sidx";
    let created_db = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["new", name])
        .current_dir(home.path())
        .env("RTSQL_HOME", home.path())
        .env_remove("RTSQL_KEY")
        .stdin(Stdio::null())
        .output()
        .expect("spawn rtsql new");
    assert!(
        created_db.status.success(),
        "rtsql new failed: {}",
        String::from_utf8_lossy(&created_db.stderr)
    );
    let query_sql = "SELECT id, name FROM t WHERE name = 'dup'";

    cli_ok(
        home.path(),
        name,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)",
    );
    cli_ok(home.path(), name, "INSERT INTO t VALUES (1, 'dup')");
    cli_ok(home.path(), name, "INSERT INTO t VALUES (2, 'other')");
    cli_ok(home.path(), name, "INSERT INTO t VALUES (3, 'dup')");
    let before = cli_ok(home.path(), name, query_sql);

    let created = cli_ok(home.path(), name, "CREATE INDEX idx_name ON t(name)");
    assert_eq!(created.trim(), r#"{"affected_rows":0}"#);
    let explained = cli_ok(home.path(), name, &format!("EXPLAIN {query_sql}"));
    assert!(
        explained.contains("IndexScanAll") && explained.contains("index=idx_name"),
        "{explained}"
    );
    let after = cli_ok(home.path(), name, query_sql);
    assert_eq!(json_query(&before), json_query(&after));

    let dropped = cli_ok(home.path(), name, "DROP INDEX idx_name");
    assert_eq!(dropped.trim(), r#"{"affected_rows":0}"#);
    let scanned = cli_ok(home.path(), name, &format!("EXPLAIN {query_sql}"));
    assert!(
        scanned.contains("DataScan") && !scanned.contains("IndexScanAll"),
        "{scanned}"
    );
    let restored = cli_ok(home.path(), name, query_sql);
    assert_eq!(json_query(&before), json_query(&restored));
}
