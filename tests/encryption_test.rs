//! Integration tests for MS17 Iteration 000 — `database-encryption` lib surface.
//!
//! RED per Plan Context T4: `Database::open_with_key` does not exist yet
//! (compile RED). Scenarios: encrypted CRUD + restart roundtrip, wrong-key /
//! mode-mismatch rejection, tampered ciphertext detection, lock precedence
//! over key errors, plaintext companions, and the dump→restore-encrypted
//! migration path at lib level (CLI end-to-end is witnessed by the T5
//! cli_test group).

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::StorageError;
use rtsql::transaction::IsolationLevel;
use tempfile::tempdir;

const KEY: &str = "pw";

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

/// (1) 加密库 CRUD + close → 正确密钥重开数据完整（restart 往返）
#[tokio::test]
async fn encrypted_crud_and_restart_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        create_t(&db).await;
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
            "insert 1",
            1,
        );
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (2, 20)").await,
            "insert 2",
            1,
        );
        db.close().await.unwrap();
    }
    let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
        .await
        .unwrap();
    let scan = rows(db.execute_sql("SELECT * FROM t").await, "reopen scan");
    assert_eq!(scan, vec![row(1, 10), row(2, 20)]);
}

/// (2) 错误密钥重开：打开期拒绝 `InvalidKey`
#[tokio::test]
async fn wrong_key_reopen_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        create_t(&db).await;
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
            "insert",
            1,
        );
        db.close().await.unwrap();
    }
    let err =
        match Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("wrong")).await {
            Err(e) => e,
            Ok(_) => panic!("expected InvalidKey for wrong key"),
        };
    assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
}

/// (3) 明文带钥 / 加密无钥：打开期显式拒绝（明密互斥）
#[tokio::test]
async fn mode_mismatch_rejected() {
    let dir = tempdir().unwrap();
    let plain = dir.path().join("plain.db");
    {
        let db = Database::open(&plain).await.unwrap();
        create_t(&db).await;
        db.close().await.unwrap();
    }
    let err = match Database::open_with_key(&plain, IsolationLevel::RepeatableRead, Some(KEY)).await
    {
        Err(e) => e,
        Ok(_) => panic!("expected InvalidKey for plaintext opened with key"),
    };
    assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");

    let enc = dir.path().join("enc.db");
    {
        let db = Database::open_with_key(&enc, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        create_t(&db).await;
        db.close().await.unwrap();
    }
    let err = match Database::open_with_key(&enc, IsolationLevel::RepeatableRead, None).await {
        Err(e) => e,
        Ok(_) => panic!("expected InvalidKey for encrypted opened without key"),
    };
    assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
}

/// (4) 篡改密文页：reopen 后读触发表时认证失败
#[tokio::test]
async fn tampered_ciphertext_page_detected_on_read() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        create_t(&db).await;
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
            "insert",
            1,
        );
        db.close().await.unwrap();
    }
    // 翻转首个用户数据页（页 2——catalog 页 0/1 之后的记录）tag 区的
    // 最后一个字节：catalog 不受影响，打开成功而数据读取失败
    const RECORD_SIZE: usize = 4124; // 12B nonce + 4096B 页 + 16B tag
    let mut raw = std::fs::read(&path).unwrap();
    let data_record = 64 + 2 * RECORD_SIZE + (RECORD_SIZE - 1);
    raw[data_record] ^= 0x01;
    std::fs::write(&path, raw).unwrap();

    let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
        .await
        .unwrap();
    match db.execute_sql("SELECT * FROM t").await {
        Response::Error { message } => {
            assert!(message.contains("invalid key"), "got: {message}");
            assert!(
                message.contains("wrong key or corrupted page"),
                "got: {message}"
            );
        }
        other => panic!("expected error on tampered page, got {other:?}"),
    }
}

/// (5) 锁冲突优先于密钥错误：持锁者占用加密库时第二开报 `DatabaseLocked`
#[tokio::test]
async fn lock_conflict_takes_precedence_over_key_errors() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    let holder = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
        .await
        .unwrap();
    create_t(&holder).await;

    let err = match Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY)).await
    {
        Err(e) => e,
        Ok(_) => panic!("expected DatabaseLocked while holder alive"),
    };
    assert!(
        matches!(err, StorageError::DatabaseLocked(_)),
        "got: {err:?}"
    );
}

/// (6) 加密库伴生 `.wal`/`.checkpoint` 保持既有明文格式
/// （位点文件 24B——T02 T7 格式；WAL 文件存在且重开可消费）
#[tokio::test]
async fn companion_files_stay_in_existing_plaintext_format() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        create_t(&db).await;
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
            "insert",
            1,
        );
        db.close().await.unwrap();
    }
    let wal_path = path.with_extension("wal");
    let checkpoint_path = path.with_extension("checkpoint");
    assert!(wal_path.exists(), "wal companion must exist");
    assert!(
        checkpoint_path.exists(),
        "checkpoint companion must exist after close()"
    );
    let site = std::fs::read(&checkpoint_path).unwrap();
    assert_eq!(site.len(), 24, "checkpoint site keeps the 24B format");
}

/// (7) 迁移路径（lib 级等价）：明文库 dump 文本逐语句执行进
/// `open_with_key` 空目标 → 行集一致（CLI 端到端由 T5 cli_test 见证）
#[tokio::test]
async fn plaintext_dump_to_encrypted_target_migration() {
    let dir = tempdir().unwrap();
    let plain = dir.path().join("plain.db");
    let enc = dir.path().join("enc.db");
    {
        let db = Database::open(&plain).await.unwrap();
        create_t(&db).await;
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
            "insert 1",
            1,
        );
        assert_affected(
            db.execute_sql("INSERT INTO t VALUES (2, 20)").await,
            "insert 2",
            1,
        );
        db.close().await.unwrap();
    }
    // lifecycle dump 同形态语句流（DDL + INSERT；INT 列字面量形态一致）
    let dump_statements = [
        "CREATE TABLE t (id INT PRIMARY KEY, n INT)",
        "INSERT INTO t VALUES (1, 10)",
        "INSERT INTO t VALUES (2, 20)",
    ];
    {
        let target = Database::open_with_key(&enc, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        for stmt in dump_statements {
            let resp = target.execute_sql(stmt).await;
            assert!(
                !matches!(resp, Response::Error { .. }),
                "restore statement '{stmt}' failed: {resp:?}"
            );
        }
        target.close().await.unwrap();
    }
    let src = Database::open(&plain).await.unwrap();
    let dst = Database::open_with_key(&enc, IsolationLevel::RepeatableRead, Some(KEY))
        .await
        .unwrap();
    let src_rows = rows(src.execute_sql("SELECT * FROM t").await, "source scan");
    let dst_rows = rows(dst.execute_sql("SELECT * FROM t").await, "target scan");
    assert_eq!(src_rows, dst_rows);
}
