//! Integration tests for MS09 Iteration 000 — `mvcc-tombstone-visibility` delta spec.
//!
//! I033: committed tombstones (delete markers) must suppress the whole version
//! chain; uncommitted/aborted ones must not, so snapshot-less scans fall
//! through to the pre-delete version. I032: uncommitted rows that reached
//! disk must not resurrect after restart. RED per Plan Context T1: the probe
//! sequences reproduce the pre-fix defects (update→delete scans resurrect the
//! pre-update version; flushed uncommitted rows resurrect after restart).

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::tempdir;

async fn open_db_at(path: &std::path::Path) -> Database {
    Database::open(path).await.unwrap()
}

fn assert_affected(resp: Response, what: &str, expected: u64) {
    match resp {
        Response::AffectedRows { count } => assert_eq!(count, expected, "{what}"),
        Response::Error { message } => panic!("{what} failed: {message}"),
        other => panic!("{what}: unexpected response {other:?}"),
    }
}

fn rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::Error { message } => panic!("{what} failed: {message}"),
        other => panic!("{what}: unexpected response {other:?}"),
    }
}

async fn create_t(db: &Database) {
    let resp = db
        .execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "create table failed: {resp:?}"
    );
}

fn row(id: i64, n: i64) -> Vec<serde_json::Value> {
    vec![serde_json::json!(id), serde_json::json!(n)]
}

/// R2-S1: the I033 probe sequence — INSERT → UPDATE → DELETE, each
/// auto-committed. The scan must not resurrect the pre-update version.
#[tokio::test]
async fn i033_update_delete_scan_is_empty() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("i033.db")).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );
    assert_affected(
        db.execute_sql("DELETE FROM t WHERE id = 1").await,
        "delete",
        1,
    );

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert!(
        scan.is_empty(),
        "R2-S1: committed update→delete chain must scan empty (pre-fix resurrects [[1,10]]), got {scan:?}"
    );
    let point = rows(
        db.execute_sql("SELECT * FROM t WHERE id = 1").await,
        "point query",
    );
    assert!(
        point.is_empty(),
        "R2-S1: point query must be empty, got {point:?}"
    );
}

/// R4-S1: the same sequence stays empty after a clean close and reopen.
#[tokio::test]
async fn i033_update_delete_scan_empty_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("i033-restart.db");
    let db = open_db_at(&path).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );
    assert_affected(
        db.execute_sql("DELETE FROM t WHERE id = 1").await,
        "delete",
        1,
    );

    db.close().await.unwrap();
    drop(db);
    let db = open_db_at(&path).await;

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert!(
        scan.is_empty(),
        "R4-S1: restart must not resurrect the pre-update version, got {scan:?}"
    );
    let point = rows(
        db.execute_sql("SELECT * FROM t WHERE id = 1").await,
        "point query",
    );
    assert!(
        point.is_empty(),
        "R4-S1: point query must be empty, got {point:?}"
    );
}

/// R1-S1: while a delete is uncommitted, a concurrent default-path (no
/// snapshot) scan sees the pre-delete committed version.
#[tokio::test]
async fn uncommitted_delete_scan_sees_pre_delete_version() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("uncommitted-delete.db")).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "uncommitted delete",
        1,
    );

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert_eq!(
        scan,
        vec![row(1, 99)],
        "R1-S1: uncommitted delete must not hide the pre-delete version (pre-fix yields [[1,10]])"
    );

    db.rollback(tx).await.unwrap();
}

/// R1-S2: the deleted version's header (commit info and chain pointer) is not
/// overwritten in place by an uncommitted delete.
#[tokio::test]
async fn uncommitted_delete_leaves_version_header_intact() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("header-intact.db")).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );

    // Locate the current (post-update) version slot before the delete. The
    // index key encoding matches InsertExecutor's `Value::Int.to_key()`.
    let meta = db.get_table("t").await.unwrap();
    let rid = meta
        .index_manager
        .search(&1i64.to_be_bytes())
        .await
        .unwrap()
        .expect("row present in PK index before delete");
    let before = db.buffer_pool.read_version_header(rid).await.unwrap();
    assert!(!before.is_deleted(), "precondition: version not deleted");
    assert!(
        before.commit_tx_id().is_some(),
        "precondition: update committed"
    );
    let before_next = before.next_version();

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "uncommitted delete",
        1,
    );

    let after = db.buffer_pool.read_version_header(rid).await.unwrap();
    assert!(
        !after.is_deleted(),
        "R1-S2: deleted version header must not be tombstoned in place (pre-fix overwrites commit_tx_id with the sentinel)"
    );
    assert_eq!(
        after.commit_tx_id(),
        before.commit_tx_id(),
        "R1-S2: commit info must be preserved"
    );
    assert_eq!(
        after.next_version(),
        before_next,
        "R1-S2: chain pointer must be preserved"
    );

    db.rollback(tx).await.unwrap();
}

/// R3-S1: after the deleting transaction rolls back, the scan produces the
/// latest committed version again (rollback neutralizes the tombstone).
#[tokio::test]
async fn tombstone_rollback_scan_recovers() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("rollback.db")).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "delete",
        1,
    );
    db.rollback(tx).await.unwrap();

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert_eq!(
        scan,
        vec![row(1, 99)],
        "R3-S1: rolled-back delete must restore the latest committed version (pre-fix yields [[1,10]])"
    );
}

/// R2-S2 (Z1 anchor): rekey update→delete — UPDATE moves the key 1→2, then
/// DELETE removes key 2. The scan must not resurrect the pre-update version
/// under the old key.
#[tokio::test]
async fn rekey_update_delete_scan_is_empty() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("rekey-z1.db")).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET id = 2 WHERE id = 1").await,
        "rekey update",
        1,
    );
    assert_affected(
        db.execute_sql("DELETE FROM t WHERE id = 2").await,
        "delete",
        1,
    );

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert!(
        scan.is_empty(),
        "R2-S2 Z1: rekey update→delete chain must scan empty, got {scan:?}"
    );
}

/// R2-S3 (Z2 anchor): delete → re-insert → update — the existing correct
/// chain form keeps yielding only the latest version.
#[tokio::test]
async fn delete_then_reinsert_update_yields_latest_version() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("reinsert-z2.db")).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("DELETE FROM t WHERE id = 1").await,
        "delete",
        1,
    );
    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 20)").await,
        "re-insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert_eq!(
        scan,
        vec![row(1, 99)],
        "R2-S3 Z2: delete→re-insert→update chain must yield only the latest version"
    );
}

/// R4-S2: an uncommitted delete does not take effect after the process
/// terminates — the WAL record of the deleting transaction is not replayed.
#[tokio::test]
async fn uncommitted_delete_not_applied_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("uncommitted-delete-restart.db");
    let db = open_db_at(&path).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    assert_affected(
        db.execute_sql("UPDATE t SET n = 99 WHERE id = 1").await,
        "update",
        1,
    );

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "uncommitted delete",
        1,
    );

    // Persist schema and committed rows (the flush equivalent of eviction)
    // without commit, rollback, or checkpoint — the uncommitted delete's WAL
    // records stay untruncated but must not be replayed on the next open.
    db.wal_buffer.do_flush().await;
    db.buffer_pool.flush_all().await.unwrap();
    drop(db);
    let db = open_db_at(&path).await;

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert_eq!(
        scan,
        vec![row(1, 99)],
        "R4-S2: uncommitted delete must not take effect after restart"
    );
}

/// R4-S3 (I032): an uncommitted row that reached disk (page flush without
/// commit or checkpoint) must not resurrect after restart — recovery marks
/// uncommitted versions aborted.
#[tokio::test]
async fn flushed_uncommitted_insert_does_not_resurrect() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("i032-resurrect.db");
    let db = open_db_at(&path).await;
    create_t(&db).await;

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("INSERT INTO t VALUES (7, 70)", &tx).await,
        "uncommitted insert",
        1,
    );

    // Persist the uncommitted row's pages and WAL records without committing
    // or checkpointing (deterministic equivalent of eviction-driven flush),
    // then terminate the process.
    db.wal_buffer.do_flush().await;
    db.buffer_pool.flush_all().await.unwrap();
    drop(db);

    let db = open_db_at(&path).await;
    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert!(
        scan.is_empty(),
        "R4-S3: uncommitted row that reached disk must not resurrect after restart (pre-fix yields [[7,70]]), got {scan:?}"
    );
}

/// Default-path baseline guard: an uncommitted insert from an explicit
/// transaction stays visible to a concurrent default-path scan (current
/// no-snapshot semantics, unchanged by this change).
#[tokio::test]
async fn default_path_uncommitted_insert_visible_baseline() {
    let dir = tempdir().unwrap();
    let db = open_db_at(&dir.path().join("rr-baseline.db")).await;
    create_t(&db).await;

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("INSERT INTO t VALUES (7, 70)", &tx).await,
        "uncommitted insert",
        1,
    );

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "scan");
    assert_eq!(
        scan,
        vec![row(7, 70)],
        "default path: uncommitted insert stays visible to snapshot-less scans (current semantics)"
    );

    db.rollback(tx).await.unwrap();
}

/// D10 (001-replan T6-R3): after a restart, the tx id allocator must hand
/// out ids strictly above every recovered id — the Read Committed
/// high-water argument ("every id ≤ the allocator's current value is
/// committed, aborted, or active") breaks if ids are reused.
#[tokio::test]
async fn tx_id_allocator_advances_past_recovered_ids_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("tx-id-advance.db");
    let db = open_db_at(&path).await;
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "committed insert",
        1,
    );

    // Leave an uncommitted insert on disk (flush without commit or
    // checkpoint), so recovery observes both a committed and an uncommitted
    // tx id on the next open.
    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("INSERT INTO t VALUES (7, 70)", &tx).await,
        "uncommitted insert",
        1,
    );
    let max_before_restart = db.transaction_manager.current_tx_id();
    assert!(
        max_before_restart >= 2,
        "precondition: at least two tx ids allocated, got {max_before_restart}"
    );

    db.wal_buffer.do_flush().await;
    db.buffer_pool.flush_all().await.unwrap();
    drop(db);

    let db = open_db_at(&path).await;
    let tx = db.begin().await.unwrap();
    assert!(
        tx.id() > max_before_restart,
        "D10: post-restart tx id must exceed every recovered id (pre-fix reuses id 1), \
         got {} with max recovered {max_before_restart}",
        tx.id()
    );
}
