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

// ===========================================================================
// MS24 Iteration 001 replan 2.9 — 回滚后墓碑行的索引条目还原
// （`mvcc-tombstone-visibility` ADDED R6 S1-S7 / `sql-constraint-enforcement`
// MODIFIED R3 新增场景）
//
// 删除者事务回滚时，墓碑行在删除前最新存活版本的 PK 与唯一索引条目 SHALL
// 全部还原：修复前条目在删除时已被清理、回滚时因墓碑 slot 无条目而无法定位
// 而整体跳过，产生两类可观察错误结果——PK 等值点查漏行（行在全表扫描可见）
// 与唯一值被释放后可再次插入（`UNIQUE` 静默失效）。
// ===========================================================================

/// R6/S1：DELETE 回滚后原行经 PK 等值点查可达（修复前点查为空）。
#[tokio::test]
async fn rollback_of_delete_restores_pk_point_lookup() {
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
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "in-tx delete",
    );
    db.rollback(tx).await.unwrap();

    let rows = expect_rows(
        db.execute_sql("SELECT * FROM t WHERE id = 1").await,
        "pk point lookup after delete rollback",
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(1), serde_json::json!("a")]],
        "回滚后 PK 等值点查必须可达"
    );
}

/// R6/S2：DELETE 回滚后原唯一值仍被复现行占用（修复后可再次插入同值）。
#[tokio::test]
async fn rollback_of_delete_keeps_unique_value_occupied() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE)")
            .await,
        "create t",
    );
    expect_ok(
        &db.execute_sql("INSERT INTO t (id, code) VALUES (1, 100)")
            .await,
        "seed insert",
    );

    let tx = db.begin().await.unwrap();
    expect_affected(
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "in-tx delete",
    );
    db.rollback(tx).await.unwrap();

    let resp = db
        .execute_sql("INSERT INTO t (id, code) VALUES (2, 100)")
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "回滚后原唯一值必须仍被占用（UNIQUE 不得静默失效），got: {:?}",
        resp
    );
}

/// R6/S3：REPLACE 回滚后原行完整复现（PK 点查可达 + 原唯一值仍占用）。
#[tokio::test]
async fn rollback_of_replace_restores_row_fully() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE)")
            .await,
        "create t",
    );
    expect_ok(
        &db.execute_sql("INSERT INTO t (id, code) VALUES (1, 100)")
            .await,
        "seed insert",
    );

    let tx = db.begin().await.unwrap();
    assert_eq!(
        expect_affected(
            db.execute_in_tx("REPLACE INTO t (id, code) VALUES (1, 300)", &tx)
                .await,
            "in-tx replace",
        ),
        1
    );
    db.rollback(tx).await.unwrap();

    assert_eq!(
        expect_rows(
            db.execute_sql("SELECT * FROM t WHERE id = 1").await,
            "pk point lookup after replace rollback",
        ),
        vec![vec![serde_json::json!(1), serde_json::json!(100)]],
        "回滚后原行必须经点查完整复现"
    );
    let resp = db
        .execute_sql("INSERT INTO t (id, code) VALUES (2, 100)")
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "回滚后原唯一值必须仍被占用，got: {:?}",
        resp
    );
}

/// R6/S4：同事务 update→delete 回滚后点查指向更新前版本（非本事务创建的
/// 首个前驱版本）。
#[tokio::test]
async fn rollback_of_update_then_delete_restores_pre_update_version() {
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
    expect_affected(
        db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
        "in-tx delete",
    );
    db.rollback(tx).await.unwrap();

    assert_eq!(
        expect_rows(
            db.execute_sql("SELECT * FROM t WHERE id = 1").await,
            "pk point lookup after update+delete rollback",
        ),
        vec![vec![serde_json::json!(1), serde_json::json!("a")]],
        "回滚必须跳过本事务的 update 版本、指向更新前的存活版本"
    );
    assert_eq!(
        expect_rows(
            db.execute_sql("SELECT * FROM t").await,
            "full scan after update+delete rollback",
        )
        .len(),
        1,
        "回滚后不得残留重复行"
    );
}

/// R6/S5：单条 auto-commit 的失败 REPLACE（先删后校验）经语句级 abort 后
/// 原行点查可达、扫描单行——修复前扫描可见但点查漏行。
///
/// 输入取非键 VARCHAR 列收 Int：仲裁（PK + 唯一列）通过 → 冲突行已删 →
/// `insert_row` 一般类型门拒绝（design D5 步骤 6 顺序）。唯一列收非法类型
/// 值会在 `arbitrate` 的 F1 守卫处提前拒绝，删除尚未发生，不覆盖本路径。
#[tokio::test]
async fn failed_replace_statement_leaves_no_index_residue() {
    let (db, _dir) = open_db().await;
    expect_ok(
        &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE, note VARCHAR)")
            .await,
        "create t",
    );
    expect_ok(
        &db.execute_sql("INSERT INTO t (id, code, note) VALUES (1, 10, 'keep')")
            .await,
        "seed insert",
    );

    let resp = db
        .execute_sql("REPLACE INTO t (id, code, note) VALUES (1, 10, 123)")
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "note 列收 Int 必须被类型门拒绝，got: {:?}",
        resp
    );

    assert_eq!(
        expect_rows(
            db.execute_sql("SELECT * FROM t WHERE id = 1").await,
            "pk point lookup after failed replace",
        ),
        vec![vec![
            serde_json::json!(1),
            serde_json::json!(10),
            serde_json::json!("keep")
        ]],
        "失败 REPLACE 回滚后原行点查必须可达"
    );
    assert_eq!(
        expect_rows(
            db.execute_sql("SELECT * FROM t").await,
            "full scan after failed replace",
        )
        .len(),
        1,
        "失败 REPLACE 回滚后扫描必须单行"
    );
    let resp = db
        .execute_sql("INSERT INTO t (id, code, note) VALUES (2, 10, 'other')")
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "失败 REPLACE 回滚后原唯一值必须仍被占用，got: {:?}",
        resp
    );
}

/// R6/S6：回滚还原的索引条目经 checkpoint 持久化，干净重开两态一致。
#[tokio::test]
async fn rollback_index_restore_survives_clean_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("explicit_tx.db");

    {
        let db = Database::open(&path).await.unwrap();
        expect_ok(
            &db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE)")
                .await,
            "create t",
        );
        expect_ok(
            &db.execute_sql("INSERT INTO t (id, code) VALUES (1, 100)")
                .await,
            "seed insert",
        );

        let tx = db.begin().await.unwrap();
        expect_affected(
            db.execute_in_tx("DELETE FROM t WHERE id = 1", &tx).await,
            "in-tx delete",
        );
        db.rollback(tx).await.unwrap();
        db.close().await.unwrap();
    }

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        expect_rows(
            db2.execute_sql("SELECT * FROM t WHERE id = 1").await,
            "pk point lookup after clean reopen",
        ),
        vec![vec![serde_json::json!(1), serde_json::json!(100)]],
        "干净重开后 PK 等值点查必须仍可达"
    );
    let resp = db2
        .execute_sql("INSERT INTO t (id, code) VALUES (2, 100)")
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "干净重开后原唯一值必须仍被占用，got: {:?}",
        resp
    );
    db2.wal_buffer.shutdown().await;
}
