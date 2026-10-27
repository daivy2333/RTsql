//! 组合索引在取消、只读、内存、加密、附加库和旧库上的 SQL / CLI 证据。

use rtsql::database::Database;
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::StorageError;
use rtsql::transaction::IsolationLevel;
use rtsql::{SqlError, SqlOutcome, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

const SOURCE_ROWS: i64 = 2000;
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);
const ABANDON_SQL: &str = "INSERT INTO t2 SELECT id, id, id FROM src";

fn db_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

async fn exec(db: &Database, sql: &str) {
    db.execute_sql(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql} failed: {error}"));
}

async fn query(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.expect(sql) {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("{sql}: expected rows, got {other:?}"),
    }
}

async fn explain(db: &Database, sql: &str) -> String {
    query(db, &format!("EXPLAIN {sql}"))
        .await
        .into_iter()
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

fn sort_by_id(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort_by(|left, right| match (&left[0], &right[0]) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        _ => panic!("id must be an integer"),
    });
    rows
}

async fn assert_clean(db: &Database) {
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(report.error_count(), 0, "{report:?}");
}

async fn assert_no_active(db: &Database) {
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "active transactions are still {active:?}"
    );
}

async fn index_names(db: &Database, table: &str) -> Vec<String> {
    let meta = db.get_table(table).await.expect(table);
    meta.secondary_indexes
        .iter()
        .map(|index| index.name.clone())
        .collect()
}

async fn anchor_table(db: &Database, table: &str) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        let tables = db.transaction_manager.tx_version_tables(tx_id).await;
        if tables.get(table).map(|rows| rows.len()).unwrap_or(0) >= 2 {
            return Some(tx_id);
        }
    }
    None
}

async fn load_source(db: &Database) {
    exec(db, "CREATE TABLE src (id INT PRIMARY KEY)").await;
    for id in 1..=SOURCE_ROWS {
        exec(db, &format!("INSERT INTO src VALUES ({id})")).await;
    }
}

async fn abandon_statement(db: &Database, sql: &str, table: &str) -> u64 {
    let mut statement = Box::pin(db.execute_sql(sql));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if let Some(tx_id) = anchor_table(db, table).await {
                    return tx_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fixture failure: the abandon anchor was not reached within 10s; return to Plan")
    });

    let tx_id = tokio::select! {
        biased;
        tx_id = &mut anchor => tx_id,
        outcome = &mut statement => panic!(
            "fixture failure: the statement finished before the abandon anchor: {outcome:?}; return to Plan"
        ),
    };
    drop(statement);
    tx_id
}

async fn expect_sql_error(db: &Database, sql: &str) -> SqlError {
    db.execute_sql(sql)
        .await
        .expect_err(&format!("{sql} must fail"))
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

fn memory_prefixed_entries() -> BTreeSet<String> {
    std::fs::read_dir(".")
        .expect("read current directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| name.starts_with(":memory:"))
        .collect()
}

#[tokio::test]
async fn cancelled_composite_index_write_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        Database::open_with_isolation(&db_path(&dir, "cancel.db"), IsolationLevel::RepeatableRead)
            .await
            .unwrap();
    load_source(&db).await;
    exec(&db, "CREATE TABLE t2 (id INT PRIMARY KEY, a INT, b INT)").await;
    exec(&db, "CREATE INDEX idx_ab ON t2(a, b)").await;

    let tx_id = abandon_statement(&db, ABANDON_SQL, "t2").await;
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the abandoned transaction must still be active before the next write"
    );

    exec(&db, "INSERT INTO t2 VALUES (1, 1, 1)").await;
    exec(&db, "INSERT INTO t2 VALUES (2, 2, 2)").await;
    assert_eq!(
        query(&db, "SELECT id, a, b FROM t2 WHERE a = 1").await,
        vec![vec![Value::Int(1), Value::Int(1), Value::Int(1)]]
    );
    assert!(query(&db, "SELECT id, a, b FROM t2 WHERE a = 3")
        .await
        .is_empty());
    assert_eq!(
        query(&db, "SELECT COUNT(*) FROM t2").await,
        vec![vec![Value::Int(2)]]
    );
    assert_clean(&db).await;
    assert_no_active(&db).await;
}

#[tokio::test]
async fn readonly_session_reads_composite_and_new_int_range() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "ro.db");
    let equality = "SELECT id, a, b, n FROM t WHERE a = 1 AND b = 1";
    let range = "SELECT id, a, b, n FROM t WHERE n > 0";
    let (equality_rows, range_rows, equality_plan, range_plan, names) = {
        let db = Database::open(&path).await.unwrap();
        exec(
            &db,
            "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT, n INT)",
        )
        .await;
        exec(&db, "INSERT INTO t VALUES (1, 1, 1, -1)").await;
        exec(&db, "INSERT INTO t VALUES (2, 2, 2, 0)").await;
        exec(&db, "INSERT INTO t VALUES (3, 3, 3, 4)").await;
        exec(&db, "CREATE INDEX idx_ab ON t(a, b)").await;
        exec(&db, "CREATE INDEX idx_n ON t(n)").await;
        let equality_rows = query(&db, equality).await;
        let range_rows = sort_by_id(query(&db, range).await);
        let equality_plan = explain(&db, equality).await;
        let range_plan = explain(&db, range).await;
        let names = index_names(&db, "t").await;
        db.close().await.unwrap();
        (equality_rows, range_rows, equality_plan, range_plan, names)
    };
    assert_eq!(names, vec!["idx_ab".to_string(), "idx_n".to_string()]);
    assert_named_index(&equality_plan, "idx_ab");
    assert_int_range(&range_plan, "idx_n");

    let db = Database::open_readonly(&path).await.unwrap();
    assert_eq!(query(&db, equality).await, equality_rows);
    assert_eq!(explain(&db, equality).await, equality_plan);
    assert_named_index(&explain(&db, equality).await, "idx_ab");
    assert_eq!(sort_by_id(query(&db, range).await), range_rows);
    let readonly_range = explain(&db, range).await;
    assert_eq!(readonly_range, range_plan);
    assert_int_range(&readonly_range, "idx_n");

    let create_index = expect_sql_error(&db, "CREATE INDEX idx_other ON t(n)").await;
    assert_eq!(create_index.sqlstate.as_deref(), Some("25006"));
    assert!(
        create_index
            .message
            .contains("cannot execute CREATE INDEX in a read-only session"),
        "{}",
        create_index.message
    );
    drop(db);

    let db = Database::open(&path).await.unwrap();
    assert_eq!(index_names(&db, "t").await, names);
}

#[tokio::test]
async fn memory_composite_range_creates_no_files() {
    let before = memory_prefixed_entries();
    let db = Database::open_in_memory().await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, -1, 0)").await;
    exec(&db, "INSERT INTO t VALUES (2, 1, 2)").await;
    exec(&db, "INSERT INTO t VALUES (3, 5, 1)").await;
    let sql = "SELECT id, a, b FROM t WHERE a > 0";
    let saved = sort_by_id(query(&db, sql).await);
    assert_eq!(
        saved,
        vec![
            vec![Value::Int(2), Value::Int(1), Value::Int(2)],
            vec![Value::Int(3), Value::Int(5), Value::Int(1)],
        ]
    );

    exec(&db, "CREATE INDEX idx_ab ON t(a, b)").await;
    assert_eq!(sort_by_id(query(&db, sql).await), saved);
    assert_int_range(&explain(&db, sql).await, "idx_ab");
    assert!(db.wal_writer.is_none());
    assert!(db.wal_buffer.is_none());
    assert!(db.checkpoint_manager.is_none());
    assert_eq!(db.path(), Path::new(":memory:"));
    assert_eq!(memory_prefixed_entries(), before);
}

#[tokio::test]
async fn encrypted_composite_index_keeps_page_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("cidx-key"))
            .await
            .unwrap();
        exec(
            &db,
            "CREATE TABLE t (id INT PRIMARY KEY, a INT, name VARCHAR)",
        )
        .await;
        exec(&db, "INSERT INTO t VALUES (1, 7, 'cidx-secret')").await;
        exec(&db, "CREATE INDEX idx_an ON t(a, name)").await;
        db.close().await.unwrap();
    }

    let bytes = std::fs::read(&path).expect("read encrypted database");
    assert!(bytes.len() > 64, "encrypted file length is {}", bytes.len());
    assert_eq!((bytes.len() - 64) % 4124, 0);
    let secret = b"cidx-secret";
    assert!(
        !bytes.windows(secret.len()).any(|window| window == secret),
        "the plaintext secret must not appear in the database file"
    );

    match Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("wrong")).await {
        Err(StorageError::InvalidKey(_)) => {}
        Err(other) => panic!("expected InvalidKey, got {other}"),
        Ok(_) => panic!("expected InvalidKey, but the database opened"),
    }

    let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("cidx-key"))
        .await
        .unwrap();
    let sql = "SELECT id, a, name FROM t WHERE a = 7 AND name = 'cidx-secret'";
    assert_eq!(
        query(&db, sql).await,
        vec![vec![
            Value::Int(1),
            Value::Int(7),
            Value::String("cidx-secret".into())
        ]]
    );
    assert_named_index(&explain(&db, sql).await, "idx_an");
}

#[tokio::test]
async fn attached_dml_maintains_composite_index() {
    let home = tempfile::tempdir().unwrap();
    let db_dir = home.path().join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    {
        let app = Database::open(&db_dir.join("app.db")).await.unwrap();
        app.close().await.unwrap();
    }
    {
        let other = Database::open(&db_dir.join("o.db")).await.unwrap();
        exec(&other, "CREATE TABLE u (id INT PRIMARY KEY, a INT, b INT)").await;
        exec(&other, "INSERT INTO u VALUES (1, 10, 10)").await;
        exec(&other, "INSERT INTO u VALUES (2, 20, 20)").await;
        exec(&other, "CREATE INDEX idx_ab ON u(a, b)").await;
        other.close().await.unwrap();
    }

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; INSERT INTO o.u VALUES (5, 50, 50)",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; UPDATE o.u SET a = 11, b = 11 WHERE id = 1",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; DELETE FROM o.u WHERE id = 2",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");

    let inserted = "SELECT id, a, b FROM u WHERE a = 50 AND b = 50";
    let updated = "SELECT id, a, b FROM u WHERE a = 11 AND b = 11";
    let old = "SELECT id, a, b FROM u WHERE a = 10 AND b = 10";
    let deleted = "SELECT id, a, b FROM u WHERE a = 20 AND b = 20";
    let names = {
        let other = Database::open(&db_dir.join("o.db")).await.unwrap();
        assert_eq!(
            query(&other, inserted).await,
            vec![vec![Value::Int(5), Value::Int(50), Value::Int(50)]]
        );
        assert_eq!(
            query(&other, updated).await,
            vec![vec![Value::Int(1), Value::Int(11), Value::Int(11)]]
        );
        assert!(query(&other, old).await.is_empty());
        assert!(query(&other, deleted).await.is_empty());
        for sql in [inserted, updated, old, deleted] {
            assert_named_index(&explain(&other, sql).await, "idx_ab");
        }
        let names = index_names(&other, "u").await;
        other.close().await.unwrap();
        names
    };
    assert_eq!(names, vec!["idx_ab".to_string()]);

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; CREATE INDEX idx_new ON o.u(a, b)",
    );
    assert_eq!(code, 3, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("WRITE TO ATTACHED DATABASE"),
        "stderr: {stderr}"
    );

    let other = Database::open(&db_dir.join("o.db")).await.unwrap();
    assert_eq!(index_names(&other, "u").await, names);
}

#[tokio::test]
async fn legacy_open_does_not_rewrite_sample() {
    let dir = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("legacy")
        .join("legacy_g1.db");
    let before = std::fs::read(&source).expect("read legacy fixture");
    let path = db_path(&dir, "legacy_g1.db");
    std::fs::copy(&source, &path).expect("copy legacy fixture");

    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        query(&db, "SELECT * FROM users").await,
        vec![
            vec![
                Value::Int(1),
                Value::String("alice".into()),
                Value::Float(9.5)
            ],
            vec![
                Value::Int(2),
                Value::String("bob".into()),
                Value::Float(7.25)
            ],
            vec![
                Value::Int(3),
                Value::String("carol".into()),
                Value::Float(8.0)
            ],
        ]
    );
    exec(&db, "CREATE INDEX idx_id_name ON users(id, name)").await;
    let alice = "SELECT * FROM users WHERE id = 1 AND name = 'alice'";
    assert_eq!(
        query(&db, alice).await,
        vec![vec![
            Value::Int(1),
            Value::String("alice".into()),
            Value::Float(9.5)
        ]]
    );
    assert_named_index(&explain(&db, alice).await, "idx_id_name");
    drop(db);
    assert_eq!(
        std::fs::read(&source).expect("reread legacy fixture"),
        before,
        "the repository sample must stay unchanged"
    );
}
