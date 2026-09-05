//! Integration tests for MS07-T04: explicit transactions (R1/S1.1–S1.6)
//!
//! Covers the `Database::begin/commit/rollback/execute_in_tx` public API:
//! atomic multi-statement commit, rollback residue-freeness (including
//! snapshot-less DataScan), tx usability after failed statements, observable
//! double-commit/rollback errors, implicit auto-commit compatibility, and
//! tx-id reuse across every statement inside one transaction.

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::transaction::{Snapshot, Transaction};
use tempfile::tempdir;

async fn open_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("explicit_tx.db"))
        .await
        .unwrap();
    (db, dir)
}

fn expect_ok(resp: &Response, what: &str) {
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {:?}",
        resp
    );
}

fn expect_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("{what}: expected QueryResult, got {:?}", other),
    }
}

fn expect_affected(resp: Response, what: &str) -> u64 {
    match resp {
        Response::AffectedRows { count } => count,
        other => panic!("{what}: expected AffectedRows, got {:?}", other),
    }
}

/// R1/S1.1: two in-tx INSERTs (different tables) become visible only through
/// one commit; no implicit begin/commit happens in between.
#[tokio::test]
async fn explicit_tx_commit_makes_multi_table_writes_visible() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t1 (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t1",
    );
    expect_ok(
        &db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t2",
    );

    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();

    expect_affected(
        db.execute_in_tx("INSERT INTO t1 (id, name) VALUES (1, 'a')", &tx)
            .await,
        "in-tx insert t1",
    );
    expect_affected(
        db.execute_in_tx("INSERT INTO t2 (id, name) VALUES (2, 'b')", &tx)
            .await,
        "in-tx insert t2",
    );

    // No implicit begin between statements: the allocator never advanced.
    assert_eq!(
        db.transaction_manager.current_tx_id(),
        tx_id,
        "in-tx DML must not allocate new transaction ids"
    );

    // No implicit commit: versions still carry UNSET commit_tx_id.
    let t1 = db.get_table("t1").await.unwrap();
    let rid = t1
        .index_manager
        .search(&1i64.to_be_bytes())
        .await
        .unwrap()
        .expect("uncommitted row must be in the index");
    let vh = db.buffer_pool.read_version_header(rid).await.unwrap();
    assert_eq!(vh.create_tx_id(), tx_id);
    assert!(
        vh.commit_tx_id().is_none(),
        "in-tx DML must not auto-commit"
    );

    db.commit(tx).await.unwrap();

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t1").await,
        "select t1 after commit",
    );
    assert_eq!(rows.len(), 1);
    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t2").await,
        "select t2 after commit",
    );
    assert_eq!(rows.len(), 1);
}

/// R1/S1.1: DDL and DML may share one explicit transaction (DDL executes
/// immediately; its artifacts are visible after the tx commits).
#[tokio::test]
async fn explicit_tx_allows_ddl_and_dml_together() {
    let (db, _dir) = open_db().await;

    let tx = db.begin().await.unwrap();
    expect_ok(
        &db.execute_in_tx("CREATE TABLE t2 (id INT PRIMARY KEY, name VARCHAR)", &tx)
            .await,
        "in-tx create table",
    );
    expect_affected(
        db.execute_in_tx("INSERT INTO t2 (id, name) VALUES (1, 'a')", &tx)
            .await,
        "in-tx insert into fresh table",
    );
    db.commit(tx).await.unwrap();

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t2").await,
        "select t2 after commit",
    );
    assert_eq!(rows.len(), 1);
}

/// R1/S1.2: rollback removes every uncommitted write — no residue for
/// snapshot-less DataScan (`SELECT *`), PK lookup, or the index.
#[tokio::test]
async fn explicit_tx_rollback_leaves_no_insert_residue() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t",
    );

    let tx = db.begin().await.unwrap();
    expect_affected(
        db.execute_in_tx("INSERT INTO t (id, name) VALUES (1, 'a')", &tx)
            .await,
        "in-tx insert",
    );
    db.rollback(tx).await.unwrap();

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t").await,
        "select after rollback",
    );
    assert!(
        rows.is_empty(),
        "rolled-back insert must not be visible, got {:?}",
        rows
    );

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t WHERE id = 1").await,
        "pk select after rollback",
    );
    assert!(rows.is_empty(), "rolled-back PK must not resolve");

    let t = db.get_table("t").await.unwrap();
    assert_eq!(
        t.index_manager.search(&1i64.to_be_bytes()).await.unwrap(),
        None,
        "index must not retain the rolled-back row"
    );
}

/// R1/S1.2: rollback of an UPDATE restores the previous version without
/// duplicating rows for snapshot-less scans.
#[tokio::test]
async fn explicit_tx_rollback_restores_updated_value() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t",
    );
    expect_ok(
        &db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'a')")
            .await,
        "seed insert",
    );

    let tx = db.begin().await.unwrap();
    expect_affected(
        db.execute_in_tx("UPDATE t SET name = 'b' WHERE id = 1", &tx)
            .await,
        "in-tx update",
    );
    db.rollback(tx).await.unwrap();

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t").await,
        "select after update rollback",
    );
    assert_eq!(
        rows.len(),
        1,
        "aborted update version must not duplicate rows"
    );
    assert_eq!(
        rows[0][1],
        serde_json::json!("a"),
        "value must revert to previous version"
    );
}

/// R1/S1.3: a failed statement returns an error but keeps the transaction
/// Active and usable (no auto-rollback).
#[tokio::test]
async fn explicit_tx_survives_failed_statement() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t",
    );

    let tx = db.begin().await.unwrap();

    // Parse error inside the tx.
    let resp = db.execute_in_tx("INSERT INTO no such table", &tx).await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "bad SQL must error: {:?}",
        resp
    );

    // Constraint failure inside the tx (duplicate PK).
    expect_affected(
        db.execute_in_tx("INSERT INTO t (id, name) VALUES (1, 'a')", &tx)
            .await,
        "first insert",
    );
    let resp = db
        .execute_in_tx("INSERT INTO t (id, name) VALUES (1, 'dup')", &tx)
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "dup PK must error: {:?}",
        resp
    );

    // The tx still accepts and commits further work.
    expect_affected(
        db.execute_in_tx("INSERT INTO t (id, name) VALUES (2, 'b')", &tx)
            .await,
        "post-error insert",
    );
    db.commit(tx).await.unwrap();

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t").await,
        "select after commit",
    );
    assert_eq!(rows.len(), 2, "only the two successful inserts commit");
}

/// R1/S1.4: re-committing (or rolling back) an already-terminal tx id
/// returns an explicit error and leaves the database usable.
#[tokio::test]
async fn explicit_tx_double_commit_and_rollback_error() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await,
        "create t",
    );

    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();
    db.commit(tx).await.unwrap();

    // The owned handle was consumed; re-entering the same tx id needs a
    // reconstructed handle.
    let again = Transaction::new(tx_id, Snapshot::new(tx_id, Vec::new()));
    let err = db.commit(again).await.unwrap_err();
    assert!(
        err.to_string().contains("already committed"),
        "expected AlreadyCommitted, got: {}",
        err
    );

    let again = Transaction::new(tx_id, Snapshot::new(tx_id, Vec::new()));
    let err = db.rollback(again).await.unwrap_err();
    assert!(
        err.to_string().contains("already aborted"),
        "expected AlreadyAborted, got: {}",
        err
    );

    // Database still usable after both errors.
    expect_ok(
        &db.execute_sql("INSERT INTO t (id) VALUES (1)").await,
        "insert after errors",
    );
}

/// R1/S1.5: without an explicit transaction, `execute_sql` still auto-commits.
#[tokio::test]
async fn implicit_execute_sql_autocommits_unchanged() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t",
    );

    expect_affected(
        db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'a')")
            .await,
        "implicit insert",
    );

    // Auto-committed: visible immediately through a fresh statement.
    let rows = expect_rows(db.execute_sql("SELECT * FROM t").await, "select");
    assert_eq!(rows.len(), 1);

    // And no transaction lingers in the active set.
    assert!(db
        .transaction_manager
        .active_transactions()
        .await
        .is_empty());
}

/// R1/S1.6: every version written inside one explicit tx carries
/// `create_tx_id == tx.id()` (never 0, never another tx), before and after
/// the commit.
#[tokio::test]
async fn explicit_tx_reuses_tx_id_across_statements() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
            .await,
        "create t",
    );

    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();

    for (id, name) in [(1, "a"), (2, "b"), (3, "c")] {
        expect_affected(
            db.execute_in_tx(
                &format!("INSERT INTO t (id, name) VALUES ({}, '{}')", id, name),
                &tx,
            )
            .await,
            &format!("in-tx insert {}", id),
        );
    }

    let t = db.get_table("t").await.unwrap();
    for id in 1i64..=3 {
        let rid = t
            .index_manager
            .search(&id.to_be_bytes())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("row {} missing before commit", id));
        let vh = db.buffer_pool.read_version_header(rid).await.unwrap();
        assert_eq!(
            vh.create_tx_id(),
            tx_id,
            "row {} must reuse the explicit tx id",
            id
        );
        assert_ne!(vh.create_tx_id(), 0);
    }

    db.commit(tx).await.unwrap();

    for id in 1i64..=3 {
        let rid = t
            .index_manager
            .search(&id.to_be_bytes())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("row {} missing after commit", id));
        let vh = db.buffer_pool.read_version_header(rid).await.unwrap();
        assert_eq!(
            vh.create_tx_id(),
            tx_id,
            "commit must not rewrite create_tx_id"
        );
        assert_eq!(
            vh.commit_tx_id(),
            Some(tx_id),
            "commit must stamp commit_tx_id with the explicit tx id"
        );
    }
}
