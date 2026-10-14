//! MS18-T01 checkpoint zero-work skip integration tests (R1, 6 scenarios).
//!
//! Skip predicate: `wal_writer.get_write_count() == 0 && !buffer_pool.has_dirty_pages()`.
//!
//! Witness design (design D4): a full checkpoint always rewrites the 24B site
//! with a fresh timestamp and a current tx watermark, appends a Checkpoint
//! marker record and physically shortens the WAL. Site timestamps have
//! one-second granularity, so byte comparison across two checkpoints can be
//! vacuous; [`stamp_sentinel_site`] writes `lsn=0, timestamp=1, watermark=0`
//! (1970) through the product API so that "site rewritten" and "site left
//! alone" are always distinguishable. `lsn=0` is the ordinary
//! post-checkpoint value, so the sentinel is semantically equivalent to a
//! checkpoint-clean database.

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::{BufferPool, ColumnType, FileStorage};
use rtsql::wal::{CheckpointManager, WalWriter};
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

fn site_path(db_path: &Path) -> std::path::PathBuf {
    db_path.with_extension("checkpoint")
}

fn wal_path(db_path: &Path) -> std::path::PathBuf {
    db_path.with_extension("wal")
}

fn read_bytes_opt(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Overwrite the site with an unreachable-in-practice timestamp so that a
/// later full checkpoint is guaranteed to change the file bytes.
/// Must be called while no `Database` holds the file lock.
fn stamp_sentinel_site(db_path: &Path) {
    let wal_writer = Arc::new(WalWriter::open(db_path).unwrap());
    let storage = FileStorage::open(db_path).unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, Arc::new(storage)).unwrap());
    CheckpointManager::new(db_path, wal_writer, buffer_pool, false)
        .write_checkpoint_site(0, 1, 0)
        .unwrap();
}

async fn create_table(db: &Database, name: &str) {
    db.create_table(
        name,
        vec![
            ("id".to_string(), ColumnType::Int),
            ("name".to_string(), ColumnType::String(255)),
        ],
        "id",
    )
    .await
    .unwrap();
}

async fn insert_user(db: &Database, id: i64) {
    match db
        .execute_sql(&format!(
            "INSERT INTO users (id, name) VALUES ({id}, 'u{id}')"
        ))
        .await
    {
        Response::AffectedRows { .. } => {}
        other => panic!("insert must succeed, got {other:?}"),
    }
}

async fn count_users(db: &Database) -> i64 {
    match db.execute_sql("SELECT COUNT(*) FROM users").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "COUNT(*) must yield exactly one row");
            rows[0][0].as_i64().expect("integer count")
        }
        other => panic!("expected QueryResult, got {other:?}"),
    }
}

/// S1 零写入会话 close 跳过 checkpoint: 位点/WAL 字节不变 + 重开数据完整.
#[tokio::test]
async fn zero_work_close_skips_site_and_wal() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    // Session 1: real work, so the database is checkpoint-clean afterwards.
    let db = Database::open(&path).await.unwrap();
    create_table(&db, "users").await;
    insert_user(&db, 1).await;
    insert_user(&db, 2).await;
    db.close().await.unwrap();
    drop(db);

    stamp_sentinel_site(&path);
    let site_before = read_bytes_opt(&site_path(&path)).expect("site present");
    let wal_before = read_bytes_opt(&wal_path(&path)).expect("WAL present");

    // Session 2: read-only statements, then close.
    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 2);
    assert_eq!(
        db.wal_writer.as_ref().unwrap().get_write_count(),
        0,
        "precondition: a read-only session appends no WAL record"
    );
    db.close().await.unwrap();
    drop(db);

    assert_eq!(
        read_bytes_opt(&site_path(&path)),
        Some(site_before),
        "zero-work close must not rewrite the checkpoint site"
    );
    assert_eq!(
        read_bytes_opt(&wal_path(&path)),
        Some(wal_before),
        "zero-work close must not rewrite or truncate the WAL"
    );

    // Reopen equivalence: all data and schema intact.
    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 2);
    assert!(db.get_table("users").await.is_ok());
    db.close().await.unwrap();
}

/// S2 从未 checkpoint 的新库零工作 close 不产生位点文件 (RED witness).
#[tokio::test]
async fn fresh_db_zero_work_close_creates_no_site() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    assert!(
        !site_path(&path).exists(),
        "precondition: a fresh database has no site file"
    );
    assert_eq!(
        db.wal_writer.as_ref().unwrap().get_write_count(),
        0,
        "precondition: no WAL append"
    );
    db.close().await.unwrap();
    drop(db);

    assert!(
        !site_path(&path).exists(),
        "zero-work close must not create a checkpoint site file"
    );

    // Reopen stays clean (existing \"never checkpointed\" path: no site).
    let db = Database::open(&path).await.unwrap();
    match db.execute_sql("SELECT 1").await {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 1),
        other => panic!("expected QueryResult, got {other:?}"),
    }
    db.close().await.unwrap();
}

/// S3 DML 会话 close 保持全量 checkpoint: 位点重写 + WAL 截断 + 重开数据在.
#[tokio::test]
async fn dml_session_close_keeps_full_checkpoint() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    create_table(&db, "users").await;
    db.close().await.unwrap();
    drop(db);

    stamp_sentinel_site(&path);
    let site_before = read_bytes_opt(&site_path(&path)).expect("site present");

    // DML session: a committed INSERT reaches the WAL before close.
    let db = Database::open(&path).await.unwrap();
    insert_user(&db, 1).await;
    assert!(
        db.wal_writer.as_ref().unwrap().get_write_count() > 0,
        "precondition: a committed DML session appends WAL records"
    );
    let wal_before = read_bytes_opt(&wal_path(&path)).expect("WAL present");
    db.close().await.unwrap();
    drop(db);

    let site_after = read_bytes_opt(&site_path(&path)).expect("site still present");
    assert_ne!(
        site_before, site_after,
        "DML close must rewrite the checkpoint site"
    );

    let wal_after = read_bytes_opt(&wal_path(&path)).expect("WAL still present");
    assert!(
        wal_after.len() < wal_before.len(),
        "full checkpoint must physically shorten the WAL ({} -> {})",
        wal_before.len(),
        wal_after.len()
    );

    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 1);
    db.close().await.unwrap();
}

/// S4 DDL-only 会话：语句边界有序落盘后会话本身零工作，close 跳过；
/// schema 经语句边界落盘（D4）跨 close 存活。
///
/// 校准记录（change 2026-10-13 T3）：本用例原断言「DDL-only close 必须
/// 全量 checkpoint」（站点写入 + 位点重写），其编码的是旧持久化模型
/// 「DDL 只留脏页、靠 close 落盘」。T3/D4 起 DDL 成功返回前即完成有序
/// 落盘，close 时缓冲池无脏页、WAL 无追加——零工作判定谓词
/// （`write_count == 0 && !has_dirty_pages()`）逐字节未变，输入状态按
/// 新模型为真，跳过是正确结果。schema 持久性由语句边界落盘承载，
/// 崩溃面由 tests/ddl_crash_recovery_test.rs 矩阵锁定。
#[tokio::test]
async fn ddl_only_close_is_zero_work_and_schema_survives() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    // Session 1: schema only. DDL writes no WAL record and the statement
    // boundary flush leaves no dirty pages, so close is a legitimate
    // zero-work skip (no site write, no WAL marker).
    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        db.wal_writer.as_ref().unwrap().get_write_count(),
        0,
        "precondition: DDL writes no WAL"
    );
    create_table(&db, "users").await;
    db.close().await.unwrap();
    drop(db);

    assert!(
        read_bytes_opt(&site_path(&path)).is_none(),
        "DDL-only close is zero-work under statement-boundary durability (D4): no site write"
    );

    // Session 2: another DDL-only session is zero-work too — a pre-existing
    // site must be left untouched by the skip.
    stamp_sentinel_site(&path);
    let site_before = read_bytes_opt(&site_path(&path)).expect("site present");

    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        db.wal_writer.as_ref().unwrap().get_write_count(),
        0,
        "precondition: DDL writes no WAL"
    );
    create_table(&db, "orders").await;
    db.close().await.unwrap();
    drop(db);

    assert_eq!(
        Some(site_before),
        read_bytes_opt(&site_path(&path)),
        "DDL-only close must leave the site alone: zero work, nothing pending"
    );

    let db = Database::open(&path).await.unwrap();
    assert!(db.get_table("users").await.is_ok(), "users must survive");
    assert!(db.get_table("orders").await.is_ok(), "orders must survive");
    db.close().await.unwrap();
}

/// S5 crash 脏库: 恢复重放后 close 保持全量 checkpoint, WAL 截断 + 数据完整.
#[tokio::test]
async fn crash_dirty_db_close_keeps_full_checkpoint() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    // Seed the schema so recovery has a catalog to replay onto.
    let db = Database::open(&path).await.unwrap();
    create_table(&db, "users").await;
    db.close().await.unwrap();
    drop(db);

    // Crash session: INSERT then drop without close (no checkpoint, no flush).
    let db = Database::open(&path).await.unwrap();
    insert_user(&db, 1).await;
    insert_user(&db, 2).await;
    drop(db);

    let wal_after_crash = read_bytes_opt(&wal_path(&path)).expect("WAL present");
    assert!(
        !wal_after_crash.is_empty(),
        "precondition: a crashed session leaves a non-empty WAL"
    );

    // Reopen (recovery replays, dirtying pages), read-only, then close.
    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 2, "recovered data must be visible");
    db.close().await.unwrap();
    drop(db);

    let wal_after = read_bytes_opt(&wal_path(&path)).expect("WAL still present");
    assert!(
        wal_after.len() < wal_after_crash.len(),
        "post-recovery close must physically shorten the WAL ({} -> {})",
        wal_after_crash.len(),
        wal_after.len()
    );

    // Reopen again: redo converges, data intact.
    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 2);
    db.close().await.unwrap();
}

/// S6 显式 checkpoint 零工作时无操作: 返回 0 + 位点/WAL 字节不变.
#[tokio::test]
async fn explicit_checkpoint_zero_work_is_noop() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    create_table(&db, "users").await;
    insert_user(&db, 1).await;
    db.close().await.unwrap();
    drop(db);

    stamp_sentinel_site(&path);
    let site_before = read_bytes_opt(&site_path(&path)).expect("site present");
    let wal_before = read_bytes_opt(&wal_path(&path)).expect("WAL present");

    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 1);
    assert_eq!(
        db.wal_writer.as_ref().unwrap().get_write_count(),
        0,
        "precondition: read-only"
    );
    let captured = db
        .checkpoint_manager
        .as_ref()
        .unwrap()
        .checkpoint(|| db.transaction_manager.current_tx_id())
        .await
        .unwrap();
    assert_eq!(
        captured, 0,
        "zero-work checkpoint reports 0 (no LSN captured)"
    );
    drop(db);

    assert_eq!(
        read_bytes_opt(&site_path(&path)),
        Some(site_before),
        "zero-work explicit checkpoint must not write the site"
    );
    assert_eq!(
        read_bytes_opt(&wal_path(&path)),
        Some(wal_before),
        "zero-work explicit checkpoint must not rewrite or truncate the WAL"
    );

    let db = Database::open(&path).await.unwrap();
    assert_eq!(count_users(&db).await, 1);
    db.close().await.unwrap();
}
