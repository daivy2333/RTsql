//! Integration tests for MS09 Iteration 000 — `transaction-isolation-levels` delta spec.
//!
//! Read Committed (RC) via the lib API: `Database::open_with_isolation` with
//! `IsolationLevel::ReadCommitted`. Every statement evaluates against the
//! committed view at statement start: other transactions' uncommitted writes
//! are invisible, commits between statements become visible to later
//! statements, and a transaction sees its own uncommitted writes. The default
//! (Repeatable Read) path is byte-for-byte unchanged. RED per Plan Context T1:
//! `open_with_isolation` and `IsolationLevel` do not exist yet (compile RED).

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::transaction::IsolationLevel;
use tempfile::tempdir;

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

/// R1-S2: a database opened with Read Committed works for auto-commit and
/// explicit-transaction statements; the write path is unchanged.
#[tokio::test]
async fn rc_open_available() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_isolation(
        &dir.path().join("rc-open.db"),
        IsolationLevel::ReadCommitted,
    )
    .await
    .unwrap();
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "insert",
        1,
    );
    let scan = rows(db.execute_sql("SELECT * FROM t").await, "auto-commit scan");
    assert_eq!(scan, vec![row(1, 10)]);

    let tx = db.begin().await.unwrap();
    let scan = rows(db.execute_in_tx("SELECT * FROM t", &tx).await, "in-tx scan");
    assert_eq!(scan, vec![row(1, 10)]);
    db.commit(tx).await.unwrap();
}

/// R2-S1: another transaction's uncommitted insert is invisible to an
/// auto-commit Read Committed reader.
#[tokio::test]
async fn rc_dirty_read_excluded() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_isolation(
        &dir.path().join("rc-dirty-read.db"),
        IsolationLevel::ReadCommitted,
    )
    .await
    .unwrap();
    create_t(&db).await;

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("INSERT INTO t VALUES (7, 70)", &tx).await,
        "uncommitted insert",
        1,
    );

    let scan = rows(db.execute_sql("SELECT * FROM t").await, "auto-commit scan");
    assert!(
        scan.is_empty(),
        "R2-S1: RC must exclude other transactions' uncommitted writes, got {scan:?}"
    );

    db.rollback(tx).await.unwrap();
}

/// R2-S2: a commit that happens between two statements of an explicit
/// transaction becomes visible to the later statement (statement-level view).
#[tokio::test]
async fn rc_statement_sees_commit_between_statements() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_isolation(
        &dir.path().join("rc-between-statements.db"),
        IsolationLevel::ReadCommitted,
    )
    .await
    .unwrap();
    create_t(&db).await;

    let tx = db.begin().await.unwrap();
    let first = rows(
        db.execute_in_tx("SELECT * FROM t", &tx).await,
        "first select",
    );
    assert!(first.is_empty(), "precondition: row not yet present");

    // Another connection commits an insert between the reader's statements.
    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 50)").await,
        "interleaved insert",
        1,
    );

    let second = rows(
        db.execute_in_tx("SELECT * FROM t", &tx).await,
        "second select",
    );
    assert_eq!(
        second,
        vec![row(5, 50)],
        "R2-S2: a commit between two statements must be visible to the later statement"
    );

    db.rollback(tx).await.unwrap();
}

/// R2-S2 (point-lookup witness): the same between-statement commit is also
/// visible through the PK point-lookup path (`WHERE id = ...`), which consumes
/// the page-level all-invisible fast path in `find_visible_version`.
///
/// Witness mechanism (mvcc-tombstone-visibility R2: the scan path and the
/// index path SHALL be semantically consistent): statement 1 is a full scan,
/// so the DataScan lazy `set_all_visible` establishes the page summary entry
/// (`min_create = u64::MAX`); the interleaved insert then pins `min_create`
/// to the writer's id (commit only clears `all_visible`, not `min_create`),
/// which is what arms the fast path for statement 2. The scan establishes
/// the page summary; the index path consumes it.
#[tokio::test]
async fn rc_point_lookup_sees_commit_between_statements() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_isolation(
        &dir.path().join("rc-point-lookup-between.db"),
        IsolationLevel::ReadCommitted,
    )
    .await
    .unwrap();
    create_t(&db).await;

    let tx = db.begin().await.unwrap();
    let first = rows(db.execute_in_tx("SELECT * FROM t", &tx).await, "first scan");
    assert!(first.is_empty(), "precondition: row not yet present");

    // Another connection commits an insert between the reader's statements.
    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 50)").await,
        "interleaved insert",
        1,
    );

    let second = rows(
        db.execute_in_tx("SELECT * FROM t WHERE id = 5", &tx).await,
        "second point lookup",
    );
    assert_eq!(
        second,
        vec![row(5, 50)],
        "R2-S2: a commit between two statements must be visible to the later point lookup"
    );

    db.rollback(tx).await.unwrap();
}

/// R2-S3: a committed delete that happens between two statements of an
/// explicit transaction makes the row disappear for the later statement.
#[tokio::test]
async fn rc_statement_sees_delete_between_statements() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_isolation(
        &dir.path().join("rc-delete-between.db"),
        IsolationLevel::ReadCommitted,
    )
    .await
    .unwrap();
    create_t(&db).await;

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (9, 90)").await,
        "insert",
        1,
    );

    let tx = db.begin().await.unwrap();
    let first = rows(
        db.execute_in_tx("SELECT * FROM t", &tx).await,
        "first select",
    );
    assert_eq!(first, vec![row(9, 90)], "precondition: row present");

    // Another connection commits a delete between the reader's statements.
    assert_affected(
        db.execute_sql("DELETE FROM t WHERE id = 9").await,
        "interleaved delete",
        1,
    );

    let second = rows(
        db.execute_in_tx("SELECT * FROM t", &tx).await,
        "second select",
    );
    assert!(
        second.is_empty(),
        "R2-S3: a commit between two statements must make the deleted row disappear, got {second:?}"
    );

    db.rollback(tx).await.unwrap();
}

/// R2-S4: within an explicit RC transaction, the transaction's own
/// uncommitted writes are visible to its own reads.
#[tokio::test]
async fn rc_self_uncommitted_write_visible() {
    let dir = tempdir().unwrap();
    let db = Database::open_with_isolation(
        &dir.path().join("rc-self-write.db"),
        IsolationLevel::ReadCommitted,
    )
    .await
    .unwrap();
    create_t(&db).await;

    let tx = db.begin().await.unwrap();
    assert_affected(
        db.execute_in_tx("INSERT INTO t VALUES (3, 30)", &tx).await,
        "own insert",
        1,
    );
    let scan = rows(db.execute_in_tx("SELECT * FROM t", &tx).await, "own read");
    assert_eq!(
        scan,
        vec![row(3, 30)],
        "R2-S4: a transaction must see its own uncommitted writes"
    );

    db.rollback(tx).await.unwrap();
}

/// R2-S5: for fully committed data, an auto-commit query sequence produces
/// identical results under Read Committed and under the default mode.
#[tokio::test]
async fn rc_autocommit_matches_default_path() {
    async fn run_sequence(path: &std::path::Path) -> Vec<Vec<serde_json::Value>> {
        let db = Database::open(path).await.unwrap();
        let resp = db
            .execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
            .await;
        assert!(!matches!(resp, Response::Error { .. }));
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
            "insert",
            1,
        );
        assert_affected(
            db.execute_sql("UPDATE t SET n = 20 WHERE id = 1").await,
            "update",
            1,
        );
        rows(db.execute_sql("SELECT * FROM t").await, "scan")
    }

    let dir = tempdir().unwrap();
    let rc_rows = run_sequence(&dir.path().join("rc-equiv.db")).await;
    let default_rows = run_sequence(&dir.path().join("default-equiv.db")).await;
    assert_eq!(
        rc_rows, default_rows,
        "R2-S5: auto-commit results must match"
    );
    assert_eq!(rc_rows, vec![row(1, 20)]);
}

/// mvcc-tombstone-visibility (MS17-T02 T5): the page-level visibility summary
/// must not poison visibility. After a restart the in-memory summary is empty;
/// the first full scan lazily establishes it (`set_all_visible` first-builds
/// `min_create = u64::MAX`), and the committed delete then clears
/// `all_visible`. The stale `{u64::MAX, false}` entry must fall back to
/// per-row checks — the remaining committed rows stay reachable through both
/// the PK point-lookup and the full-scan path. RED before the fix: the stale
/// entry reports the whole page all-invisible, so both reads return empty.
///
/// Fixture note: the checkpoint site carries the allocator watermark
/// (MS17-T02 Iter002), so the reopened RC database's high-water mark already
/// covers the pre-restart tx ids — no scratch DML is needed to raise it, and
/// table t keeps no summary entry until the scan below arms it via the
/// entry-less `set_all_visible` first-build.
#[tokio::test]
async fn rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rc-summary-poison.db");

    {
        let db = Database::open_with_isolation(&path, IsolationLevel::ReadCommitted)
            .await
            .unwrap();
        create_t(&db).await;
        for i in 1..=3 {
            assert_affected(
                db.execute_sql(&format!("INSERT INTO t VALUES ({i}, {i}0)"))
                    .await,
                "committed insert",
                1,
            );
        }
        db.close().await.unwrap();
    } // dropped: releases the advisory file lock before reopening

    // Reopen: the process-local vis_map starts empty, so the scan below arms
    // the summary via set_all_visible's entry-less first-build.
    let db = Database::open_with_isolation(&path, IsolationLevel::ReadCommitted)
        .await
        .unwrap();

    let scan = rows(
        db.execute_sql("SELECT * FROM t").await,
        "scan after restart",
    );
    assert_eq!(
        scan.len(),
        3,
        "precondition: three committed rows survive the restart"
    );

    assert_affected(
        db.execute_sql("DELETE FROM t WHERE id = 1").await,
        "committed delete",
        1,
    );

    let lookup = rows(
        db.execute_sql("SELECT * FROM t WHERE id = 2").await,
        "point lookup after delete",
    );
    assert_eq!(
        lookup,
        vec![row(2, 20)],
        "stale page summary must not hide a committed row from the point-lookup path"
    );
    let scan = rows(
        db.execute_sql("SELECT * FROM t").await,
        "full scan after delete",
    );
    assert_eq!(
        scan.len(),
        2,
        "stale page summary must not hide committed rows from the scan path"
    );
}

/// transaction-isolation-levels S1/S3 (MS17-T02 Iter002): after a clean close
/// (checkpoint truncates the WAL), the checkpoint site carries the allocator
/// watermark, so a reopened Read Committed database sees all pre-restart
/// committed rows immediately — without any new DML raising the high-water
/// mark. RED before the fix: the truncated WAL leaves no Begin/Commit frames,
/// recovery observes no ids, the allocator restarts at zero, and the first
/// SELECT returns zero rows. The trailing insert is the S3 witness: the
/// post-restart tx id exceeds the site watermark, so the new row coexists
/// with the pre-restart rows in one RC statement.
#[tokio::test]
async fn rc_reopen_sees_committed_rows_without_new_dml() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rc-reopen-watermark.db");

    {
        let db = Database::open_with_isolation(&path, IsolationLevel::ReadCommitted)
            .await
            .unwrap();
        create_t(&db).await;
        for i in 1..=3 {
            assert_affected(
                db.execute_sql(&format!("INSERT INTO t VALUES ({i}, {i}0)"))
                    .await,
                "committed insert",
                1,
            );
        }
        db.close().await.unwrap();
    } // dropped: releases the advisory file lock before reopening

    let db = Database::open_with_isolation(&path, IsolationLevel::ReadCommitted)
        .await
        .unwrap();

    let scan = rows(
        db.execute_sql("SELECT * FROM t").await,
        "immediate scan after reopen",
    );
    assert_eq!(
        scan,
        vec![row(1, 10), row(2, 20), row(3, 30)],
        "S1: committed rows must be visible immediately after reopen, got {scan:?}"
    );

    assert_affected(
        db.execute_sql("INSERT INTO t VALUES (4, 40)").await,
        "post-restart insert",
        1,
    );
    let scan = rows(
        db.execute_sql("SELECT * FROM t").await,
        "scan after new insert",
    );
    assert_eq!(
        scan,
        vec![row(1, 10), row(2, 20), row(3, 30), row(4, 40)],
        "S3: the new row's tx id exceeds the site watermark; old and new rows coexist in one statement, got {scan:?}"
    );
}
