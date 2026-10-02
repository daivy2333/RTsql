//! MS10-T02: 跨进程文件锁回归守卫。
//!
//! 场景（change spec database-file-lock R1/R2）：
//! - 同进程二次 `Database::open` 同一路径被拒（`StorageError::DatabaseLocked`）；
//! - 持有者 drop 后锁随 fd 释放，可重新打开；
//! - 两次 `FileStorage::open` 同一路径，第二次被拒；
//! - 错误信息含 `database is locked` 前缀与路径（CLI stderr 断言锚点）。

use std::path::PathBuf;
use std::time::Duration;

use rtsql::database::Database;
use rtsql::storage::{FileStorage, StorageError};
use rtsql::transaction::IsolationLevel;
use tempfile::TempDir;

/// T4（D8 校准）：锁冲突断言用例的显式短预算——`Some(0)` ＝ 仅 ~8ms 可见性
/// 下限窗口即拒绝（MS28 前默认语义的显式表达），断言本身零变化。
const ZERO_WAIT: Option<Duration> = Some(Duration::from_millis(0));

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

    let second = expect_locked(
        Database::open_with_lock_wait(&path, IsolationLevel::RepeatableRead, None, ZERO_WAIT).await,
    );
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
    let second = expect_locked(FileStorage::open_with_lock(&path, None, ZERO_WAIT));
    assert_locked(&second);
    drop(first);
    assert!(FileStorage::open(&path).is_ok(), "reopen after drop");
}

#[tokio::test]
async fn test_locked_error_message_contains_path() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let _holder = Database::open(&path).await.unwrap();
    let err = expect_locked(
        Database::open_with_lock_wait(&path, IsolationLevel::RepeatableRead, None, ZERO_WAIT).await,
    );
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

    let err = expect_locked(FileStorage::open_with_lock(&path, None, ZERO_WAIT));
    assert_locked(&err);
}

/// Cycle 001（replan）T12：重试耗尽拒绝——锁被原生 fd 稳定持有时，
/// `Database::open` 经有界重试（~8ms 窗口）后仍以 `DatabaseLocked` 拒绝。
/// 确定性用例；现实现即绿——语义保持见证（错误面与拒绝语义不变）。
/// T4（D8 校准）：显式短预算 `Some(0)` 承载旧「默认即 ~8ms 窗口」语义，
/// 断言零变化。
#[tokio::test]
async fn test_retry_exhaustion_still_rejects_active_holder() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    // 先建一个有效库文件，再以原生 fd 稳定持有路径锁（模拟活跃持有者）。
    {
        let db = Database::open(&path).await.unwrap();
        drop(db);
    }
    let holder = std::fs::File::open(&path).unwrap();
    holder.try_lock().expect("test process acquires flock");

    let err = expect_locked(
        Database::open_with_lock_wait(&path, IsolationLevel::RepeatableRead, None, ZERO_WAIT).await,
    );
    assert_locked(&err);
}

/// Cycle 001（replan）T12：只读打开同型变体——独占锁被持有时，共享锁
/// 打开经重试耗尽后仍以 `DatabaseLocked` 拒绝（`readonly-open-mode` 锁矩阵
/// 活跃持有者照旧拒绝，仅时延 + 窗口量级）。T4（D8 校准）：显式短预算。
#[tokio::test]
async fn test_retry_exhaustion_still_rejects_readonly_open() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    {
        let db = Database::open(&path).await.unwrap();
        drop(db);
    }
    let holder = std::fs::File::open(&path).unwrap();
    holder.try_lock().expect("test process acquires flock");

    let err = expect_locked(FileStorage::open_readonly_with_lock(&path, None, ZERO_WAIT));
    assert_locked(&err);
}

/// Cycle 001（replan）T12：close→立即重开循环回归见证——flock 释放可见性
/// 窗口（design D10）不再以测试可见频率产生假 `DatabaseLocked`。修复前为
/// 统计性偶发（纯 std 探针 2/20000 量级，父 Cycle Review Finding 9），
/// 本用例作回归见证，不构成 RED 判据；确定性判据是重试机制本身。
#[tokio::test]
async fn test_close_reopen_loop_no_false_locked() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    for round in 0..300 {
        match Database::open(&path).await {
            Ok(db) => drop(db),
            Err(e) => panic!("round {round}: close→reopen hit false DatabaseLocked: {e:?}"),
        }
    }
}
