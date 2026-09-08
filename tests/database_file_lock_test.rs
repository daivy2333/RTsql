//! MS10-T02: 跨进程文件锁回归守卫。
//!
//! 场景（change spec database-file-lock R1/R2）：
//! - 同进程二次 `Database::open` 同一路径被拒（`StorageError::DatabaseLocked`）；
//! - 持有者 drop 后锁随 fd 释放，可重新打开；
//! - 两次 `FileStorage::open` 同一路径，第二次被拒；
//! - 错误信息含 `database is locked` 前缀与路径（CLI stderr 断言锚点）。

use std::path::PathBuf;

use rtsql::database::Database;
use rtsql::storage::{FileStorage, StorageError};
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("lock_test.db")
}

/// unwrap_err 的等价形式：不要求 Ok 侧类型实现 Debug。
fn expect_locked<T>(result: Result<T, StorageError>) -> StorageError {
    match result {
        Ok(_) => panic!("expected StorageError::DatabaseLocked, got Ok"),
        Err(e) => e,
    }
}

fn assert_locked(err: &StorageError) {
    assert!(
        matches!(err, StorageError::DatabaseLocked(_)),
        "expected StorageError::DatabaseLocked, got: {err:?}"
    );
}

#[tokio::test]
async fn test_same_process_double_open_database_rejected() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let first = Database::open(&path).await;
    assert!(
        first.is_ok(),
        "first open should succeed: {:?}",
        first.as_ref().err()
    );

    let second = expect_locked(Database::open(&path).await);
    assert_locked(&second);
}

#[tokio::test]
async fn test_reopen_after_drop_succeeds() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let first = Database::open(&path).await.unwrap();
    drop(first);

    let reopened = Database::open(&path).await;
    assert!(
        reopened.is_ok(),
        "reopen after drop should succeed: {:?}",
        reopened.as_ref().err()
    );
}

#[tokio::test]
async fn test_file_storage_double_open_rejected() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let first = FileStorage::open(&path).unwrap();
    let second = expect_locked(FileStorage::open(&path));
    assert_locked(&second);
    drop(first);
    assert!(FileStorage::open(&path).is_ok(), "reopen after drop");
}

#[tokio::test]
async fn test_locked_error_message_contains_path() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let _holder = Database::open(&path).await.unwrap();
    let err = expect_locked(Database::open(&path).await);
    let msg = err.to_string();
    assert!(
        msg.contains("database is locked"),
        "message should contain 'database is locked', got: {msg}"
    );
    assert!(
        msg.contains(path.display().to_string().as_str()),
        "message should contain the db path, got: {msg}"
    );
}

/// MS10-T03（file-format-header R3-S1）：锁先于头校验——非 RTsql 文件被
/// 持锁时，第二打开者得 `DatabaseLocked` 而非格式错误。
/// GREEN 守卫（锁本就先于一切，MS10-T02 语义）。
#[tokio::test]
async fn test_locked_bad_file_reports_lock_not_format() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("garbage.db");
    std::fs::write(&path, vec![0xABu8; 8192]).unwrap();

    let holder = std::fs::File::open(&path).unwrap();
    holder.try_lock().expect("test process acquires flock");

    let err = expect_locked(FileStorage::open(&path));
    assert_locked(&err);
}
