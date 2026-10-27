//! 字符串二级索引在取消、超时、失败、重开和几种运行形态下的 SQL / CLI 证据。

use rtsql::database::Database;
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::StorageError;
use rtsql::transaction::{IsolationLevel, Transaction};
use rtsql::{SqlError, SqlOutcome, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const SOURCE_ROWS: i64 = 2000;
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);
const TIMEOUT_CANCEL_WINDOW: Duration = Duration::from_millis(20);
const ABANDON_SQL: &str = "INSERT INTO t2 SELECT id, name FROM src";

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

async fn query_in_tx(db: &Database, sql: &str, tx: &Transaction) -> Vec<Vec<Value>> {
    match db.execute_in_tx(sql, tx).await.expect(sql) {
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

fn assert_data_scan(plan: &str) {
    assert!(
        plan.contains("DataScan") && !plan.contains("IndexScanAll"),
        "plan must scan without IndexScanAll, got: {plan}"
    );
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

fn assert_id_name(rows: &[Vec<Value>], id: i64, name: &str) {
    assert_eq!(
        rows,
        vec![vec![Value::Int(id), Value::String(name.to_string())]]
    );
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
    exec(db, "CREATE TABLE src (id INT PRIMARY KEY, name VARCHAR)").await;
    for id in 1..=SOURCE_ROWS {
        exec(db, &format!("INSERT INTO src VALUES ({id}, '{id}')")).await;
    }
}

async fn open_copy_target(dir: &TempDir, name: &str, isolation: IsolationLevel) -> Database {
    let db = Database::open_with_isolation(&db_path(dir, name), isolation)
        .await
        .unwrap();
    load_source(&db).await;
    exec(&db, "CREATE TABLE t2 (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "CREATE INDEX idx_name ON t2(name)").await;
    db
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

fn watch_anchor(db: Database, table: &'static str) -> tokio::task::JoinHandle<bool> {
    tokio::spawn(async move {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if anchor_table(&db, table).await.is_some() {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok()
    })
}

async fn assert_reclaimed_pair(db: &Database) {
    assert_id_name(
        &query(db, "SELECT id, name FROM t2 WHERE name = '1'").await,
        1,
        "1",
    );
    assert_id_name(
        &query(db, "SELECT id, name FROM t2 WHERE name = '2'").await,
        2,
        "2",
    );
    assert!(query(db, "SELECT id, name FROM t2 WHERE name = '3'")
        .await
        .is_empty());
    assert_eq!(
        query(db, "SELECT COUNT(*) FROM t2").await,
        vec![vec![Value::Int(2)]]
    );
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

fn cli_rows(stdout: &str) -> Vec<Vec<serde_json::Value>> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(found) = value.get("rows").and_then(|item| item.as_array()) else {
            continue;
        };
        for row in found {
            if let Some(cells) = row.as_array() {
                rows.push(cells.clone());
            }
        }
    }
    rows
}

fn memory_prefixed_entries() -> BTreeSet<String> {
    std::fs::read_dir(".")
        .expect("read current directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| name.starts_with(":memory:"))
        .collect()
}

fn assert_repeated(value: &Value, byte: u8, len: usize) {
    match value {
        Value::String(text) => {
            assert_eq!(text.len(), len);
            assert!(text.as_bytes().iter().all(|item| *item == byte));
        }
        other => panic!("expected a string, got {other:?}"),
    }
}

#[tokio::test]
async fn cancelled_string_index_write_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_copy_target(&dir, "cancel.db", IsolationLevel::RepeatableRead).await;

    let tx_id = abandon_statement(&db, ABANDON_SQL, "t2").await;
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the abandoned transaction must still be active before the next write"
    );

    exec(&db, "INSERT INTO t2 VALUES (1, '1')").await;
    exec(&db, "INSERT INTO t2 VALUES (2, '2')").await;
    assert_reclaimed_pair(&db).await;
    assert_clean(&db).await;
    assert_no_active(&db).await;
}

#[tokio::test]
async fn timed_out_implicit_string_index_write_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_copy_target(&dir, "timeout.db", IsolationLevel::RepeatableRead).await;
    let observer = watch_anchor(db.clone(), "t2");

    let outcome = tokio::time::timeout(TIMEOUT_CANCEL_WINDOW, db.execute_sql(ABANDON_SQL)).await;
    assert!(
        outcome.is_err(),
        "fixture failure: the statement finished within 20ms; return to Plan. outcome={outcome:?}"
    );
    assert!(
        observer.await.unwrap(),
        "fixture failure: the anchor observer did not see t2 reach 2 versions; return to Plan"
    );

    exec(&db, "INSERT INTO t2 VALUES (1, '1')").await;
    exec(&db, "INSERT INTO t2 VALUES (2, '2')").await;
    assert_reclaimed_pair(&db).await;
    assert_clean(&db).await;
}

#[tokio::test]
async fn explicit_tx_timeout_keeps_string_index_until_caller_ends_tx() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "explicit.db");
    let db = open_copy_target(&dir, "explicit.db", IsolationLevel::ReadCommitted).await;

    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();
    let observer = watch_anchor(db.clone(), "t2");
    let outcome =
        tokio::time::timeout(TIMEOUT_CANCEL_WINDOW, db.execute_in_tx(ABANDON_SQL, &tx)).await;
    assert!(
        outcome.is_err(),
        "fixture failure: the statement finished within 20ms; return to Plan. outcome={outcome:?}"
    );
    assert!(
        observer.await.unwrap(),
        "fixture failure: the anchor observer did not see t2 reach 2 versions; return to Plan"
    );
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the explicit transaction must stay active after the timeout"
    );
    assert!(
        !query_in_tx(&db, "SELECT id FROM t2", &tx).await.is_empty(),
        "the timed-out writes must stay visible inside the transaction"
    );
    assert!(
        query(&db, "SELECT id FROM t2").await.is_empty(),
        "the timed-out writes must stay invisible outside the transaction"
    );

    db.rollback(tx).await.unwrap();
    assert!(
        query(&db, "SELECT id FROM t2").await.is_empty(),
        "rollback must leave the abandoned rows invisible"
    );
    exec(&db, "INSERT INTO t2 VALUES (1, '1')").await;
    exec(&db, "INSERT INTO t2 VALUES (2, '2')").await;

    // t2 已经重新占用 1 和 2。提交半程改写空表 t3，同一条 INSERT SELECT 才能在锚点前继续推进。
    exec(&db, "CREATE TABLE t3 (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "CREATE INDEX idx_t3_name ON t3(name)").await;
    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();
    let observer = watch_anchor(db.clone(), "t3");
    let outcome = tokio::time::timeout(
        TIMEOUT_CANCEL_WINDOW,
        db.execute_in_tx("INSERT INTO t3 SELECT id, name FROM src", &tx),
    )
    .await;
    assert!(
        outcome.is_err(),
        "fixture failure: the commit-half statement finished within 20ms; return to Plan. outcome={outcome:?}"
    );
    assert!(
        observer.await.unwrap(),
        "fixture failure: the anchor observer did not see t3 reach 2 versions; return to Plan"
    );
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the second explicit transaction must stay active after the timeout"
    );
    db.commit(tx).await.unwrap();
    assert!(
        !query(&db, "SELECT id FROM t3 WHERE name = '1'")
            .await
            .is_empty(),
        "the caller's commit must keep the string equality row"
    );

    db.close().await.unwrap();
    drop(db);
    let db = Database::open(&path).await.unwrap();
    assert!(
        !query(&db, "SELECT id FROM t3 WHERE name = '1'")
            .await
            .is_empty(),
        "the committed string equality row must survive reopen"
    );
}

#[tokio::test]
async fn undelivered_commit_keeps_string_index_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "undelivered.db");
    let db = Database::open(&path).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;

    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();
    db.execute_in_tx("INSERT INTO t VALUES (1, 'kept')", &tx)
        .await
        .unwrap();
    db.set_checkpoint_bytes_threshold(1);

    let commit_returned = Arc::new(AtomicBool::new(false));
    let mut commit = Box::pin({
        let db = db.clone();
        let commit_returned = commit_returned.clone();
        async move {
            let outcome = db.commit(tx).await;
            commit_returned.store(true, Ordering::SeqCst);
            outcome
        }
    });
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if !db
                    .transaction_manager
                    .active_transactions()
                    .await
                    .contains(&tx_id)
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "fixture failure: the commit did not leave the active set within 10s; return to Plan",
        )
    });

    tokio::select! {
        biased;
        _ = &mut anchor => {}
        outcome = &mut commit => panic!(
            "fixture failure: the commit future returned before the undelivered window: {outcome:?}; return to Plan"
        ),
    }
    assert!(
        !commit_returned.load(Ordering::SeqCst),
        "fixture failure: the commit future already returned; return to Plan"
    );
    drop(anchor);
    drop(commit);

    assert_id_name(
        &query(&db, "SELECT id, name FROM t WHERE name = 'kept'").await,
        1,
        "kept",
    );
    assert_no_active(&db).await;
    exec(&db, "INSERT INTO t VALUES (2, 'next')").await;

    db.close().await.unwrap();
    drop(db);
    let db = Database::open(&path).await.unwrap();
    assert_id_name(
        &query(&db, "SELECT id, name FROM t WHERE name = 'kept'").await,
        1,
        "kept",
    );
}

#[tokio::test]
async fn failed_mid_insert_restores_string_index() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "dup.db")).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "INSERT INTO t VALUES (1, 'keep')").await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    exec(&db, "CREATE TABLE src (id INT, name VARCHAR)").await;
    exec(&db, "INSERT INTO src VALUES (2, 'new')").await;
    exec(&db, "INSERT INTO src VALUES (1, 'dup')").await;

    let error = db
        .execute_sql("INSERT INTO t SELECT id, name FROM src ORDER BY id DESC")
        .await
        .expect_err("the duplicate primary key must fail the insert");
    assert_eq!(
        error,
        SqlError {
            message: "Execution error: Duplicate key".to_string(),
            sqlstate: None,
        },
        "ORDER BY rejection or a different duplicate text returns to Plan"
    );

    assert_id_name(&query(&db, "SELECT * FROM t").await, 1, "keep");
    assert!(query(&db, "SELECT id, name FROM t WHERE name = 'new'")
        .await
        .is_empty());
    let keep_sql = "SELECT id, name FROM t WHERE name = 'keep'";
    assert_id_name(&query(&db, keep_sql).await, 1, "keep");
    assert_named_index(&explain(&db, keep_sql).await, "idx_name");
    assert_clean(&db).await;

    exec(&db, "INSERT INTO t VALUES (2, 'new')").await;
    assert_id_name(
        &query(&db, "SELECT id, name FROM t WHERE name = 'new'").await,
        2,
        "new",
    );
}

#[tokio::test]
async fn sql_string_index_clean_reopen_and_pending_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "reopen.db");
    let db = Database::open(&path).await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "INSERT INTO t VALUES (1, 'alpha')").await;
    exec(&db, "INSERT INTO t VALUES (2, '你好')").await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&path).await.unwrap();
    for (id, name) in [(1, "alpha"), (2, "你好")] {
        let sql = format!("SELECT id, name FROM t WHERE name = '{name}'");
        assert_id_name(&query(&db, &sql).await, id, name);
        assert_named_index(&explain(&db, &sql).await, "idx_name");
    }
    assert_clean(&db).await;

    exec(&db, "INSERT INTO t VALUES (3, 'late')").await;
    db.wal_buffer.as_ref().expect("wal buffer").shutdown().await;
    drop(db);

    let db = Database::open(&path).await.unwrap();
    for (id, name) in [(1, "alpha"), (2, "你好"), (3, "late")] {
        let sql = format!("SELECT id, name FROM t WHERE name = '{name}'");
        assert_id_name(&query(&db, &sql).await, id, name);
    }
    assert_clean(&db).await;
}

#[tokio::test]
async fn legacy_sql_string_index_keeps_existing_rows() {
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
    exec(&db, "CREATE INDEX idx_name ON users(name)").await;
    let alice = "SELECT * FROM users WHERE name = 'alice'";
    assert_eq!(
        query(&db, alice).await,
        vec![vec![
            Value::Int(1),
            Value::String("alice".into()),
            Value::Float(9.5)
        ]]
    );
    assert!(
        explain(&db, alice).await.contains("IndexScanAll"),
        "alice equality must use IndexScanAll"
    );
    assert_eq!(
        query(&db, "SELECT * FROM users WHERE id = 1").await,
        query(&db, alice).await
    );
    assert_clean(&db).await;
    drop(db);
    assert_eq!(
        std::fs::read(&source).expect("reread legacy fixture"),
        before,
        "the repository sample must stay unchanged"
    );
}

#[tokio::test]
async fn memory_string_index_creates_no_files() {
    let before = memory_prefixed_entries();
    let db = Database::open_in_memory().await.unwrap();
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "INSERT INTO t VALUES (1, 'mem')").await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    let sql = "SELECT id, name FROM t WHERE name = 'mem'";
    assert_id_name(&query(&db, sql).await, 1, "mem");
    assert!(
        explain(&db, sql).await.contains("IndexScanAll"),
        "memory equality must use IndexScanAll"
    );
    assert!(db.wal_writer.is_none());
    assert!(db.wal_buffer.is_none());
    assert!(db.checkpoint_manager.is_none());
    assert_eq!(db.path(), Path::new(":memory:"));
    assert_eq!(memory_prefixed_entries(), before);
}

#[tokio::test]
async fn readonly_session_reads_string_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "ro.db");
    {
        let db = Database::open(&path).await.unwrap();
        exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        exec(&db, "INSERT INTO t VALUES (1, 'ro')").await;
        exec(&db, "CREATE INDEX idx_name ON t(name)").await;
        db.close().await.unwrap();
    }

    let db = Database::open_readonly(&path).await.unwrap();
    let sql = "SELECT id, name FROM t WHERE name = 'ro'";
    assert_id_name(&query(&db, sql).await, 1, "ro");
    assert_named_index(&explain(&db, sql).await, "idx_name");

    let create_index = expect_sql_error(&db, "CREATE INDEX idx_other ON t(name)").await;
    assert_eq!(create_index.sqlstate.as_deref(), Some("25006"));
    assert!(
        create_index
            .message
            .contains("cannot execute CREATE INDEX in a read-only session"),
        "{}",
        create_index.message
    );
    let insert = expect_sql_error(&db, "INSERT INTO t VALUES (2, 'no')").await;
    assert_eq!(insert.sqlstate.as_deref(), Some("25006"));
    assert!(
        insert
            .message
            .contains("cannot execute INSERT in a read-only session"),
        "{}",
        insert.message
    );
    drop(db);

    let db = Database::open(&path).await.unwrap();
    assert_id_name(
        &query(&db, "SELECT id, name FROM t WHERE name = 'ro'").await,
        1,
        "ro",
    );
    let meta = db.get_table("t").await.unwrap();
    let names: Vec<&str> = meta
        .secondary_indexes
        .iter()
        .map(|index| index.name.as_str())
        .collect();
    assert_eq!(names, vec!["idx_name"]);
}

async fn expect_sql_error(db: &Database, sql: &str) -> SqlError {
    db.execute_sql(sql)
        .await
        .expect_err(&format!("{sql} must fail"))
}

#[tokio::test]
async fn encrypted_string_index_keeps_page_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = db_path(&dir, "enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("sidx-key"))
            .await
            .unwrap();
        exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        exec(&db, "INSERT INTO t VALUES (1, 'sidx-secret')").await;
        exec(&db, "CREATE INDEX idx_name ON t(name)").await;
        db.close().await.unwrap();
    }

    let bytes = std::fs::read(&path).expect("read encrypted database");
    assert!(bytes.len() > 64, "encrypted file length is {}", bytes.len());
    assert_eq!((bytes.len() - 64) % 4124, 0);
    let secret = b"sidx-secret";
    assert!(
        !bytes.windows(secret.len()).any(|window| window == secret),
        "the plaintext secret must not appear in the database file"
    );

    match Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("wrong")).await {
        Err(StorageError::InvalidKey(_)) => {}
        Err(other) => panic!("expected InvalidKey, got {other}"),
        Ok(_) => panic!("expected InvalidKey, but the database opened"),
    }

    let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("sidx-key"))
        .await
        .unwrap();
    let sql = "SELECT id, name FROM t WHERE name = 'sidx-secret'";
    assert_id_name(&query(&db, sql).await, 1, "sidx-secret");
    assert!(
        explain(&db, sql).await.contains("IndexScanAll"),
        "encrypted equality must use IndexScanAll"
    );
}

#[tokio::test]
async fn attached_dml_maintains_string_index() {
    let home = tempfile::tempdir().unwrap();
    let db_dir = home.path().join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    {
        let app = Database::open(&db_dir.join("app.db")).await.unwrap();
        app.close().await.unwrap();
    }
    {
        let other = Database::open(&db_dir.join("o.db")).await.unwrap();
        exec(&other, "CREATE TABLE u (id INT PRIMARY KEY, name VARCHAR)").await;
        exec(&other, "INSERT INTO u VALUES (1, 'alpha')").await;
        exec(&other, "INSERT INTO u VALUES (2, 'beta')").await;
        exec(&other, "CREATE INDEX idx_name ON u(name)").await;
        other.close().await.unwrap();
    }

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; INSERT INTO o.u VALUES (5, 'routed')",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; UPDATE o.u SET name = 'moved' WHERE id = 1",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; DELETE FROM o.u WHERE id = 2",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; SELECT id, name FROM o.u WHERE name = 'routed'",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let routed = cli_rows(&stdout);
    assert!(
        routed.iter().any(|row| {
            row.first().and_then(serde_json::Value::as_i64) == Some(5)
                && row.get(1).and_then(serde_json::Value::as_str) == Some("routed")
        }),
        "routed result segment: {stdout}"
    );

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; SELECT id, name FROM o.u WHERE name = 'beta'",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        cli_rows(&stdout)
            .iter()
            .all(|row| { row.get(1).and_then(serde_json::Value::as_str) != Some("beta") }),
        "beta query must not return a beta row: {stdout}"
    );

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; SELECT id, name FROM o.u WHERE name = 'moved'",
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        cli_rows(&stdout)
            .iter()
            .any(|row| row.first().and_then(serde_json::Value::as_i64) == Some(1)),
        "moved result must contain id 1: {stdout}"
    );

    {
        let other = Database::open(&db_dir.join("o.db")).await.unwrap();
        assert_id_name(
            &query(&other, "SELECT id, name FROM u WHERE name = 'moved'").await,
            1,
            "moved",
        );
        assert!(query(&other, "SELECT id, name FROM u WHERE name = 'beta'")
            .await
            .is_empty());
        assert_id_name(
            &query(&other, "SELECT id, name FROM u WHERE name = 'routed'").await,
            5,
            "routed",
        );
        for name in ["moved", "routed"] {
            let sql = format!("SELECT id, name FROM u WHERE name = '{name}'");
            assert_named_index(&explain(&other, &sql).await, "idx_name");
        }
        other.close().await.unwrap();
    }

    let (code, stdout, stderr) = run_cli(
        home.path(),
        "app",
        "ATTACH 'o' AS o; CREATE INDEX idx_new ON o.u(name)",
    );
    assert_eq!(code, 3, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("WRITE TO ATTACHED DATABASE"),
        "stderr: {stderr}"
    );

    let other = Database::open(&db_dir.join("o.db")).await.unwrap();
    let meta = other.get_table("u").await.unwrap();
    let names: Vec<&str> = meta
        .secondary_indexes
        .iter()
        .map(|index| index.name.as_str())
        .collect();
    assert_eq!(names, vec!["idx_name"]);
}

#[tokio::test]
async fn default_and_overflow_row_hits_string_index() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&db_path(&dir, "overflow.db")).await.unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR DEFAULT 'def', body TEXT)",
    )
    .await;
    let xs = "x".repeat(5000);
    let ys = "y".repeat(5000);
    let zs = "z".repeat(5000);
    exec(&db, &format!("INSERT INTO t (id, body) VALUES (1, '{xs}')")).await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;

    let def_sql = "SELECT id, name, body FROM t WHERE name = 'def'";
    let first = query(&db, def_sql).await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0][0], Value::Int(1));
    assert_eq!(first[0][1], Value::String("def".into()));
    assert_repeated(&first[0][2], b'x', 5000);
    assert_named_index(&explain(&db, def_sql).await, "idx_name");

    exec(&db, &format!("INSERT INTO t (id, body) VALUES (2, '{ys}')")).await;
    let mut defs = query(&db, def_sql).await;
    defs.sort_by(|left, right| match (&left[0], &right[0]) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        _ => panic!("id must be an integer"),
    });
    assert_eq!(defs.len(), 2);
    assert_eq!(defs[0][0], Value::Int(1));
    assert_eq!(defs[1][0], Value::Int(2));
    assert_eq!(defs[0][1], Value::String("def".into()));
    assert_eq!(defs[1][1], Value::String("def".into()));

    exec(&db, &format!("INSERT INTO t VALUES (3, '{zs}', 'short')")).await;
    let mut all = query(&db, "SELECT * FROM t").await;
    all.sort_by(|left, right| match (&left[0], &right[0]) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        _ => panic!("id must be an integer"),
    });
    let long = all
        .iter()
        .find(|row| row[0] == Value::Int(3))
        .expect("the long indexed value must be readable from SELECT *");
    assert_repeated(&long[1], b'z', 5000);
    assert_eq!(long[2], Value::String("short".into()));

    let long_sql = format!("SELECT id, name, body FROM t WHERE name = '{zs}'");
    assert_data_scan(&explain(&db, &long_sql).await);
    let long_rows = query(&db, &long_sql).await;
    assert_eq!(long_rows.len(), 1);
    assert_eq!(long_rows[0][0], Value::Int(3));
    assert_repeated(&long_rows[0][1], b'z', 5000);

    let defs = query(&db, def_sql).await;
    assert!(defs.iter().all(|row| row[0] != Value::Int(3)));
    assert_eq!(defs.len(), 2);
}
