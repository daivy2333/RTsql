//! MS42 Iteration 001：crypto 关闭组合的关闭态行为锚定（R3）。
//!
//! 本目标只在 no-crypto 组合编译执行——加密正例由默认组合的
//! `encryption_test` 与 storage 单元测试覆盖。三个用例分别对应：
//! S5 加密头文件被拒绝、S6 携带密钥打开被拒绝（含新建路径不写头字节）、
//! S3 明文库全功能往返。

#![cfg(not(feature = "crypto"))]

use rtsql::database::Database;
use rtsql::storage::StorageError;
use rtsql::transaction::IsolationLevel;
use rtsql::{SqlOutcome, Value};
use tempfile::tempdir;

/// S5：合成 64 字节加密头（flags = 加密位）以 `Database::open` 打开——
/// 本构建 `KNOWN_FLAGS_MASK = 0`，加密位按未知 flag 拒绝，沿用既有
/// 前向防护错误面，不触任何加密代码路径。
#[tokio::test]
async fn encrypted_header_rejected_as_unknown_flag() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    let mut buf = [0u8; 64];
    buf[..8].copy_from_slice(b"RTSQLDB\0");
    buf[8..12].copy_from_slice(&1u32.to_le_bytes()); // version
    buf[12..16].copy_from_slice(&1u32.to_le_bytes()); // FLAG_ENCRYPTED
    buf[16..20].copy_from_slice(&4096u32.to_le_bytes()); // page size
    std::fs::write(&path, buf).unwrap();
    let err = match Database::open(&path).await {
        Err(e) => e,
        Ok(_) => panic!("expected IncompatibleHeader for encrypted header"),
    };
    match err {
        StorageError::IncompatibleHeader(msg) => {
            assert!(msg.contains("unknown feature flags"), "got: {msg}");
        }
        other => panic!("expected IncompatibleHeader, got: {other:?}"),
    }
}

/// S6：携带密钥的打开在两个既有路径都被拒绝——明文库（头校验路径）与
/// 0 字节新建路径；消息点名本构建未包含加密能力，新建路径不写任何
/// 格式头字节（文件保持 0 字节）。
#[tokio::test]
async fn key_carrying_open_rejected_without_crypto() {
    let dir = tempdir().unwrap();
    let plain = dir.path().join("plain.db");
    {
        let db = Database::open(&plain).await.unwrap();
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    let err = match Database::open_with_key(&plain, IsolationLevel::RepeatableRead, Some("k")).await
    {
        Err(e) => e,
        Ok(_) => panic!("expected InvalidKey for plaintext opened with key"),
    };
    match err {
        StorageError::InvalidKey(msg) => {
            assert!(msg.contains("not available in this build"), "got: {msg}");
        }
        other => panic!("expected InvalidKey, got: {other:?}"),
    }

    let fresh = dir.path().join("fresh.db");
    let err = match Database::open_with_key(&fresh, IsolationLevel::RepeatableRead, Some("k")).await
    {
        Err(e) => e,
        Ok(_) => panic!("expected InvalidKey for 0-byte create with key"),
    };
    match err {
        StorageError::InvalidKey(msg) => {
            assert!(msg.contains("not available in this build"), "got: {msg}");
        }
        other => panic!("expected InvalidKey, got: {other:?}"),
    }
    let len = std::fs::metadata(&fresh).map(|m| m.len()).unwrap_or(0);
    assert_eq!(len, 0, "rejected create must not write header bytes");
}

/// S3：明文库全功能——建表/写入/查询/close/重开往返。
#[tokio::test]
async fn plaintext_full_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rt.db");
    {
        let db = Database::open(&path).await.unwrap();
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
            .await
            .unwrap();
        match db
            .execute_sql("INSERT INTO t VALUES (1, 10)")
            .await
            .unwrap()
        {
            SqlOutcome::AffectedRows(1) => {}
            other => panic!("unexpected insert outcome: {other:?}"),
        }
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    match db
        .execute_sql("SELECT n FROM t WHERE id = 1")
        .await
        .unwrap()
    {
        SqlOutcome::Rows { rows, .. } => {
            assert_eq!(rows, vec![vec![Value::Int(10)]]);
        }
        other => panic!("unexpected select outcome: {other:?}"),
    }
    db.close().await.unwrap();
}
