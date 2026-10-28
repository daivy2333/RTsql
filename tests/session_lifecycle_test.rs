//! 读写会话的 checkpoint / close 与刷盘循环。
//!
//! `close` 返回后定时器不再把未达容量的记录写入 WAL。
//! `checkpoint` 返回后定时器继续写。

use rtsql::database::Database;
use rtsql::wal::{WalReader, WalRecord};
use rtsql::SqlOutcome;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn wal_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("wal")
}

fn read_wal(db_path: &Path) -> Vec<WalRecord> {
    let mut reader = WalReader::open(&wal_path(db_path)).unwrap();
    reader.read_all().expect("WAL must be readable to completion")
}

fn contains_begin(records: &[WalRecord], tx_id: u64) -> bool {
    records
        .iter()
        .any(|record| matches!(record, WalRecord::BeginTxn { tx_id: id } if *id == tx_id))
}

fn contains_commit(records: &[WalRecord], tx_id: u64) -> bool {
    records
        .iter()
        .any(|record| matches!(record, WalRecord::CommitTxn { tx_id: id, .. } if *id == tx_id))
}

fn query_rows(outcome: SqlOutcome) -> Vec<Vec<rtsql::Value>> {
    match outcome {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("expected rows, got {other:?}"),
    }
}

async fn open_with_one_committed_row(dir: &tempfile::TempDir) -> (PathBuf, Database) {
    let path = dir.path().join("lifecycle.db");
    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();
    (path, db)
}

#[tokio::test]
async fn close_stops_timer_flush() {
    let dir = tempfile::tempdir().unwrap();
    let (path, db) = open_with_one_committed_row(&dir).await;

    tokio::time::timeout(Duration::from_secs(2), db.close())
        .await
        .expect("close must return within 2 seconds")
        .unwrap();

    let wal = db
        .wal_buffer
        .as_ref()
        .expect("read-write session has a WAL buffer");
    wal.append(WalRecord::BeginTxn { tx_id: 424242 }).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let records = read_wal(&path);
    assert!(
        !contains_begin(&records, 424242),
        "BeginTxn 424242 must stay out of the WAL after close, got {records:?}"
    );
}

#[tokio::test]
async fn checkpoint_resumes_timer_flush() {
    let dir = tempfile::tempdir().unwrap();
    let (path, db) = open_with_one_committed_row(&dir).await;

    db.checkpoint().await.unwrap();

    let wal = db
        .wal_buffer
        .as_ref()
        .expect("read-write session has a WAL buffer");
    wal.append(WalRecord::BeginTxn { tx_id: 424243 }).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let records = read_wal(&path);
    assert!(
        contains_begin(&records, 424243),
        "BeginTxn 424243 must reach the WAL after checkpoint, got {records:?}"
    );

    db.close().await.unwrap();
}

#[tokio::test]
async fn fresh_close_stops_timer_flush() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lifecycle.db");
    let db = Database::open(&path).await.unwrap();

    tokio::time::timeout(Duration::from_secs(2), db.close())
        .await
        .expect("close must return within 2 seconds")
        .unwrap();

    let wal = db
        .wal_buffer
        .as_ref()
        .expect("read-write session has a WAL buffer");
    wal.append(WalRecord::BeginTxn { tx_id: 515151 }).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let records = read_wal(&path);
    assert!(
        !contains_begin(&records, 515151),
        "BeginTxn 515151 must stay out of the WAL after a fresh close, got {records:?}"
    );
}

#[tokio::test]
async fn uncommitted_row_absent_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lifecycle.db");
    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (7)", &tx)
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), db.close())
        .await
        .expect("close must return within 2 seconds")
        .unwrap();
    drop(db);

    let db = Database::open(&path).await.unwrap();
    let rows = query_rows(
        db.execute_sql("SELECT id FROM t WHERE id = 7")
            .await
            .unwrap(),
    );
    assert_eq!(
        rows,
        vec![vec![rtsql::Value::Int(7)]],
        "an uncommitted insert must still be found after close and reopen, got {rows:?}"
    );
    db.close().await.unwrap();
}

#[tokio::test]
async fn commit_after_close_wait_returns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lifecycle.db");
    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), db.close())
        .await
        .expect("close must return within 2 seconds")
        .unwrap();

    let tx_id = tokio::time::timeout(Duration::from_secs(2), async {
        let tx = db.begin().await.unwrap();
        db.execute_in_tx("INSERT INTO t VALUES (3)", &tx)
            .await
            .unwrap();
        let tx_id = tx.id();
        db.commit(tx).await.unwrap();
        tx_id
    })
    .await
    .expect("commit after close must return within 2 seconds");

    let records = read_wal(&path);
    assert!(
        contains_commit(&records, tx_id),
        "CommitTxn {tx_id} must be readable from the WAL after commit-on-closed-session, got {records:?}"
    );
}

#[tokio::test]
async fn commit_and_close_finish_within_2s() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lifecycle.db");
    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (1)", &tx)
        .await
        .unwrap();

    let commit_db = db.clone();
    let close_db = db.clone();
    let (commit_result, close_result) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(2), commit_db.commit(tx)),
        tokio::time::timeout(Duration::from_secs(2), close_db.close()),
    );
    commit_result
        .expect("commit must return within 2 seconds")
        .unwrap();
    close_result
        .expect("close must return within 2 seconds")
        .unwrap();
}

#[tokio::test]
async fn second_shutdown_after_close_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (_path, db) = open_with_one_committed_row(&dir).await;

    tokio::time::timeout(Duration::from_secs(2), db.close())
        .await
        .expect("close must return within 2 seconds")
        .unwrap();

    let wal = db
        .wal_buffer
        .as_ref()
        .expect("read-write session has a WAL buffer");
    tokio::time::timeout(Duration::from_secs(2), wal.shutdown())
        .await
        .expect("the first shutdown after close must return");
    tokio::time::timeout(Duration::from_secs(2), wal.shutdown())
        .await
        .expect("the second shutdown after close must return");
}
