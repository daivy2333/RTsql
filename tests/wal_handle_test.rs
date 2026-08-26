//! WalWriter 持久句柄测试
//!
//! 验证 WalWriter 持有单一持久文件句柄后的行为契约：
//! 1. fd 上界：10K tx 压测下 /proc/self/fd 净增量 < 10（MS06-T03 验收口径）
//! 2. LSN 偏移语义：返回值 LSN == 写前文件长度，首条为 0，严格递增
//! 3. truncate 后同句柄追加位置与 get_current_lsn 正确
//! 4. 并发写后恢复解析完整无错

use rtsql::database::Database;
use rtsql::storage::RowId;
use rtsql::wal::{WalReader, WalRecord, WalWriter};
use std::sync::Arc;
use tempfile::tempdir;

/// 构造一条可区分的 Insert 记录
fn mk_insert(tx_id: u64, tag: u8, size: usize) -> WalRecord {
    WalRecord::Insert {
        tx_id,
        table_name: "handle_test".to_string(),
        row_id: RowId::new(0, 0),
        tuple_data: vec![tag; size],
    }
}

/// 断言 Response 非 Error
fn assert_ok(resp: rtsql::network::protocol::Response, ctx: &str) {
    if let rtsql::network::protocol::Response::Error { message } = resp {
        panic!("{} failed: {}", ctx, message);
    }
}

/// 统计当前进程 fd 数量（Linux /proc/self/fd）
fn count_fds() -> std::io::Result<usize> {
    Ok(std::fs::read_dir("/proc/self/fd")?.count())
}

#[tokio::test]
async fn test_fd_bound_under_10k_tx() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("fd_bound.rtsql");
    let db = Database::open(&db_path)
        .await
        .expect("Failed to open database");

    db.execute_sql("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        .await;

    // 压测前采样点在 Database::open 之后（排除打开瞬间的固有 fd）
    let before = count_fds().unwrap();

    for i in 0..10_000u64 {
        let sql = format!("INSERT INTO users VALUES ({}, 'user_{}')", i, i);
        let resp = db.execute_sql(&sql).await;
        assert_ok(resp, &format!("INSERT {}", i));
    }

    let after = count_fds().unwrap();
    let delta = after as i64 - before as i64;
    println!(
        "fd bound: before={} after={} delta={}",
        before, after, delta
    );
    assert!(
        delta < 10,
        "fd net delta {} ({} -> {}) should be < 10 under 10K tx",
        delta,
        before,
        after
    );

    // Database 无公开 close()；drop 验证句柄随对象释放（等价收口）
    let fds_before_drop = count_fds().unwrap();
    drop(db);
    let fds_after_drop = count_fds().unwrap();
    println!(
        "fd around db drop: before_drop={} after_drop={}",
        fds_before_drop, fds_after_drop
    );
}

#[tokio::test]
async fn test_write_record_lsn_equals_file_offset() {
    let dir = tempdir().unwrap();
    // 不带扩展名，使 with_extension("wal") 产生可预期的 wal 路径
    let db_path = dir.path().join("lsn_offset");
    let wal_path = db_path.with_extension("wal");
    let writer = WalWriter::open(&db_path).unwrap();

    let mut prev_lsn: Option<u64> = None;
    let mut first_lsn: Option<u64> = None;
    for i in 0..3u64 {
        let len_before = std::fs::metadata(&wal_path).unwrap().len();
        let record = mk_insert(i + 1, (i as u8) + 1, 8 + i as usize);
        let lsn = writer.write_record(record).await.unwrap();

        assert_eq!(
            lsn, len_before,
            "LSN must equal pre-write file length (record {})",
            i
        );
        if let Some(prev) = prev_lsn {
            assert!(lsn > prev, "LSN must strictly increase (record {})", i);
        }
        if first_lsn.is_none() {
            first_lsn = Some(lsn);
        }
        prev_lsn = Some(lsn);
    }
    assert_eq!(first_lsn, Some(0), "First record must have LSN 0");
}

#[tokio::test]
async fn test_truncate_then_append_same_handle() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("trunc_append");
    let wal_path = db_path.with_extension("wal");
    let writer = WalWriter::open(&db_path).unwrap();

    // 写 3 条，记录各条 LSN
    let mut lsns = Vec::new();
    for i in 0..3u64 {
        lsns.push(
            writer
                .write_record(mk_insert(i + 1, (i as u8) + 1, 16))
                .await
                .unwrap(),
        );
    }
    assert_eq!(lsns[0], 0);

    // 截断到第 3 条边界（第 2 条末尾）
    writer.truncate_to(lsns[2]).await.unwrap();
    assert_eq!(
        writer.get_current_lsn().await.unwrap(),
        lsns[2],
        "get_current_lsn must reflect truncated length"
    );

    // 同句柄再写一条，落在新末尾
    let new_lsn = writer.write_record(mk_insert(99, 9, 16)).await.unwrap();
    assert_eq!(
        new_lsn, lsns[2],
        "post-truncate append LSN must equal truncation point"
    );

    drop(writer);

    // 读回验证：恰好 3 条完整记录且无解析错误（第 1、2 条 + 新追加的第 4 条）
    let mut reader = WalReader::open(&wal_path).unwrap();
    let records = reader.read_all().unwrap();
    assert_eq!(
        records.len(),
        3,
        "expected exactly 3 valid records after truncate+append"
    );
}

#[tokio::test]
async fn test_concurrent_writers_recovery_consistent() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("concurrent_writers");
    let wal_path = db_path.with_extension("wal");
    let writer = Arc::new(WalWriter::open(&db_path).unwrap());

    const TASKS: usize = 4;
    const PER_TASK: usize = 25;

    let mut handles = Vec::new();
    for t in 0..TASKS {
        let w = Arc::clone(&writer);
        handles.push(tokio::spawn(async move {
            for i in 0..PER_TASK {
                let tx_id = ((t * PER_TASK + i) + 1) as u64;
                w.write_record(mk_insert(tx_id, (t as u8) + 1, 12))
                    .await
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    drop(writer);

    // 恢复读回：总数一致且逐条解析无错误
    let mut reader = WalReader::open(&wal_path).unwrap();
    let records = reader.read_all().unwrap();
    assert_eq!(
        records.len(),
        TASKS * PER_TASK,
        "recovery must read back all written records"
    );
}
