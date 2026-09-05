//! MS07-T05 Checkpoint 位点消费与恢复收敛测试
//!
//! change: ms07-rest-explicit-tx-checkpoint-pushdown / Iteration 001
//! - S2.1: 有效位点只重放位点之后的记录（redo 收敛）
//! - S2.2: 无位点文件 → 全量重放（行为与现状一致）
//! - S2.4: 位点损坏（<16B / L > file_len）→ 安全退化全量重放

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::{AsyncStorage, BufferPool, FileStorage, RowId, TableManager};
use rtsql::wal::{RecoveryManager, WalRecord, WalWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// 位点文件形态
enum SiteMode {
    /// 不写位点文件
    None,
    /// 有效位点：指向 WAL 前缀末尾
    Valid,
    /// 位点文件被截断为 15B（< 16B 视为无效）
    Truncated,
    /// 位点 LSN 超出 WAL 文件长度（代际失效）
    BeyondFileEnd,
}

/// 手写 16B 位点文件（[lsn: u64 LE][timestamp: u64 LE]）
fn write_site_file(path: &Path, lsn: u64, timestamp: u64) {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&lsn.to_le_bytes());
    buf.extend_from_slice(&timestamp.to_le_bytes());
    std::fs::write(path.with_extension("checkpoint"), buf).unwrap();
}

/// 构建场景：CREATE TABLE t + 5 条已提交 INSERT。
///
/// 前 3 条之后刷脏页（catalog + 数据页落盘，非 checkpoint 路径），
/// 按模式写位点文件，再提交后 2 条（WAL 有记录、页未刷）。
/// 不调用 `close()`，避免触发 checkpoint 干扰场景。
async fn build_checkpoint_scenario(dir: &TempDir, mode: SiteMode) -> PathBuf {
    let path = db_path(dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    for i in 0..3u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }

    // 非 checkpoint 路径落盘 catalog + 数据页，使表定义在重启后可见
    db.buffer_pool.flush_all().await.unwrap();

    let prefix_end = db.wal_writer.get_current_lsn().await.unwrap();
    match mode {
        SiteMode::None => {}
        SiteMode::Valid => write_site_file(&path, prefix_end, 42),
        SiteMode::Truncated => {
            std::fs::write(path.with_extension("checkpoint"), vec![0u8; 15]).unwrap();
        }
        SiteMode::BeyondFileEnd => write_site_file(&path, prefix_end + 10_000, 42),
    }

    for i in 3..5u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }

    db.wal_buffer.shutdown().await;
    drop(db);
    path
}

/// 用同一 db 文件构造全新的恢复组件（模拟重启后的 buffer pool / table manager）
async fn fresh_recovery_components(path: &Path) -> (Arc<BufferPool>, Arc<TableManager>) {
    let storage: Arc<dyn AsyncStorage> = Arc::new(FileStorage::open(path).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(100, storage.clone()).unwrap());
    let table_manager = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_manager.open_or_init().await.unwrap();
    (buffer_pool, table_manager)
}

/// S2.1: 有效位点只重放位点之后的 committed 数据记录
#[tokio::test]
async fn valid_site_limits_redo_to_records_after_site() {
    let dir = TempDir::new().unwrap();
    let path = build_checkpoint_scenario(&dir, SiteMode::Valid).await;

    let (buffer_pool, table_manager) = fresh_recovery_components(&path).await;
    let result = RecoveryManager::full_recover(&path, buffer_pool, table_manager)
        .await
        .unwrap();

    assert_eq!(
        result.redo_count, 2,
        "redo 必须只覆盖位点之后的记录（位点前 3 条已由刷脏页覆盖）"
    );
    // 分类仍覆盖全部记录：5 个自动提交事务全部被识别为 committed
    assert!(
        result.committed_tx_ids.len() >= 5,
        "分类不得因位点裁剪: {:?}",
        result.committed_tx_ids
    );
    assert!(result.uncommitted_tx_ids.is_empty());
}

/// S2.2: 无位点文件 → 全量重放（行为与现状一致）
#[tokio::test]
async fn missing_site_falls_back_to_full_redo() {
    let dir = TempDir::new().unwrap();
    let path = build_checkpoint_scenario(&dir, SiteMode::None).await;

    let (buffer_pool, table_manager) = fresh_recovery_components(&path).await;
    let result = RecoveryManager::full_recover(&path, buffer_pool, table_manager)
        .await
        .unwrap();

    assert_eq!(result.redo_count, 5, "无位点时必须全量重放");
}

/// S2.4: 位点文件 < 16B（部分写入）→ 安全退化全量重放
#[tokio::test]
async fn truncated_site_falls_back_to_full_redo() {
    let dir = TempDir::new().unwrap();
    let path = build_checkpoint_scenario(&dir, SiteMode::Truncated).await;

    let (buffer_pool, table_manager) = fresh_recovery_components(&path).await;
    let result = RecoveryManager::full_recover(&path, buffer_pool, table_manager)
        .await
        .unwrap();

    assert_eq!(result.redo_count, 5, "位点损坏时必须退化全量重放");
}

/// S2.4: 位点 LSN 超出 WAL 文件长度（代际失效）→ 安全退化全量重放
#[tokio::test]
async fn site_beyond_wal_length_falls_back_to_full_redo() {
    let dir = TempDir::new().unwrap();
    let path = build_checkpoint_scenario(&dir, SiteMode::BeyondFileEnd).await;

    let (buffer_pool, table_manager) = fresh_recovery_components(&path).await;
    let result = RecoveryManager::full_recover(&path, buffer_pool, table_manager)
        .await
        .unwrap();

    assert_eq!(result.redo_count, 5, "代际失效位点必须退化全量重放");
}

/// S2.3: 恢复期表缺失必须显式报错并传播到 Database::open（K05 显式化）
#[tokio::test]
async fn missing_table_during_redo_fails_explicitly() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    // 手写 WAL：BeginTxn + Insert{table:"ghost"} + CommitTxn（catalog 无 ghost 表）
    let wal_writer = WalWriter::open(&path).unwrap();
    wal_writer
        .write_record(WalRecord::BeginTxn { tx_id: 7 })
        .await
        .unwrap();
    wal_writer
        .write_record(WalRecord::Insert {
            tx_id: 7,
            table_name: "ghost".to_string(),
            row_id: RowId::new(0, 0),
            tuple_data: vec![1, 2, 3],
        })
        .await
        .unwrap();
    wal_writer
        .write_record(WalRecord::CommitTxn {
            tx_id: 7,
            timestamp: 1,
        })
        .await
        .unwrap();
    wal_writer.fsync().await.unwrap();
    drop(wal_writer);

    // 全新组件：catalog 中没有 ghost 表
    let (buffer_pool, table_manager) = fresh_recovery_components(&path).await;
    let err = RecoveryManager::full_recover(&path, buffer_pool, table_manager)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("ghost"),
        "错误信息必须含表名: {}",
        err
    );

    // 同一文件 Database::open 显式失败
    let open_err = match Database::open(&path).await {
        Err(e) => e,
        Ok(_) => panic!("Database::open must fail when redo hits a missing table"),
    };
    assert!(
        open_err.to_string().contains("ghost"),
        "open 错误信息必须含表名: {}",
        open_err
    );
}

/// S2.1(a): checkpoint 后 WAL 文件物理缩短（有界）
#[tokio::test]
async fn checkpoint_truncates_wal_file() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    for i in 0..5u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    let wal = path.with_extension("wal");
    let len_before = std::fs::metadata(&wal).unwrap().len();

    db.checkpoint().await.unwrap();

    let len_after = std::fs::metadata(&wal).unwrap().len();
    assert!(
        len_after < len_before,
        "WAL 必须物理缩短: {} -> {}",
        len_before,
        len_after
    );
    assert!(
        len_after < 128,
        "截断后仅应剩 Checkpoint 记录: {}",
        len_after
    );
}

/// S2.1(b): checkpoint 后崩溃重开，行数精确 N+M（无丢无重：
/// pre-checkpoint 行来自已刷页，post-checkpoint 行来自后缀重放）
#[tokio::test]
async fn crash_after_checkpoint_replays_suffix_exactly() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    for i in 0..5u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    db.checkpoint().await.unwrap();

    // checkpoint 后再写 3 行，然后不 close 直接 drop（崩溃模拟，页未刷）
    for i in 5..8u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    drop(db);

    let db2 = Database::open(&path).await.unwrap();
    let resp = db2.execute_sql("SELECT * FROM t").await;
    match resp {
        Response::QueryResult { rows } => assert_eq!(
            rows.len(),
            8,
            "5 行来自已刷页 + 3 行来自后缀重放，必须无丢无重"
        ),
        other => panic!("Expected QueryResult, got {:?}", other),
    }
}

/// S2.1(c): checkpoint 后崩溃恢复的 redo_count 收敛到后缀记录数
#[tokio::test]
async fn redo_count_converges_after_checkpoint() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    for i in 0..5u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    db.checkpoint().await.unwrap();
    for i in 5..8u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    db.wal_buffer.shutdown().await;
    drop(db);

    let (buffer_pool, table_manager) = fresh_recovery_components(&path).await;
    let result = RecoveryManager::full_recover(&path, buffer_pool, table_manager)
        .await
        .unwrap();

    assert_eq!(
        result.redo_count, 3,
        "redo 必须收敛到 checkpoint 之后的 3 条记录（前 5 条已由刷脏页覆盖）"
    );
}

/// S2.1 补充: close() 自动触发 checkpoint（WAL 有界）且数据完整
#[tokio::test]
async fn close_triggers_checkpoint_and_preserves_data() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    for i in 0..3u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    let wal = path.with_extension("wal");
    let len_before = std::fs::metadata(&wal).unwrap().len();

    db.close().await.unwrap();
    drop(db);

    let len_after = std::fs::metadata(&wal).unwrap().len();
    assert!(
        len_after < len_before,
        "close 必须触发 checkpoint 截断: {} -> {}",
        len_before,
        len_after
    );

    let db2 = Database::open(&path).await.unwrap();
    let resp = db2.execute_sql("SELECT * FROM t").await;
    match resp {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 3),
        other => panic!("Expected QueryResult, got {:?}", other),
    }
}
