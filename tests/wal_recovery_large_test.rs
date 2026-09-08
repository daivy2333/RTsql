//! MS10-T02 Iteration 000: WAL 恢复逐帧无歧义解析测试
//!
//! change: 2026-09-06-ms10-t02-file-lock-graceful-shutdown / Iteration 000-wal-recovery-fix
//! - S1: 含 ≥19 条记录（>1KB）的 WAL 在崩溃（shutdown + drop 不 close）后
//!   `Database::open` 恢复成功、数据完整。修复前因 `WalReader` 格式嗅探
//!   误判歧义偏移上的新格式帧而 `Incomplete WAL record`。
//! - 混合流: checkpoint 截断后的 WAL 以旧格式 Checkpoint 记录开头，
//!   后续新格式帧与之混排（生产负载形态）。
//! - 守护用例: 小 WAL（<19 条）重开恒成功（守护既有路径）。
//!
//! 夹具说明：CREATE TABLE 后显式 `checkpoint()` 使 catalog 页落盘——DDL 无
//! WAL 记录，未刷页的小库在崩溃重开时 redo 必然 table not found（引擎持久化
//! 模型，与 reader 解析正交）。checkpoint 同时把 WAL 截断为 21B 旧格式
//! Checkpoint 记录，其后的 INSERT 追加新格式帧，构成混合格式流。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// S1: 大 WAL（checkpoint 后 500 条 INSERT 新格式记录，>1KB，混合格式流）
/// 崩溃后恢复成功、数据完整
#[tokio::test]
async fn large_wal_recovers_after_unclean_shutdown() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    db.checkpoint().await.unwrap();

    let values: Vec<String> = (0..500u64).map(|i| format!("({},{})", i, i * 2)).collect();
    let resp = db
        .execute_sql(&format!("INSERT INTO t VALUES {}", values.join(",")))
        .await;
    assert!(
        matches!(resp, Response::AffectedRows { count: 500 }),
        "INSERT 需成功写入 500 行，实际 {:?}",
        resp
    );

    // 全量落 WAL（夹具先例：checkpoint_redo_reduction_test.rs）
    db.wal_buffer.shutdown().await;

    // RED 判据（Plan Context 风险注记 5）：不依赖精确记录数，以 WAL >2KB 保证
    // 越过既有 <1KB 盲区（嗅探误判首例 ≈ 第 19 条记录，offset 1033）
    let wal_len = std::fs::metadata(path.with_extension("wal")).unwrap().len();
    assert!(wal_len > 2048, "夹具要求 WAL >2KB，实际 {}B", wal_len);

    // 崩溃模拟：drop 不 close
    drop(db);

    // 修复前此处 Err(Incomplete WAL record) → unwrap panic（RED）
    let db2 = Database::open(&path).await.unwrap();
    let resp = db2.execute_sql("SELECT COUNT(*) FROM t").await;
    match resp {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "COUNT(*) 必须返回单行");
            assert_eq!(
                rows[0][0],
                serde_json::json!(500),
                "恢复后数据必须完整：checkpoint 后的 500 行全部重放"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    db2.wal_buffer.shutdown().await;
}

/// 守护: 小 WAL（checkpoint 后 5 行，<19 条记录）重开恒成功，数据完整
#[tokio::test]
async fn small_wal_recovers_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.checkpoint().await.unwrap();
    for i in 0..5u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await;
    }
    db.wal_buffer.shutdown().await;
    drop(db);

    let db2 = Database::open(&path).await.unwrap();
    let resp = db2.execute_sql("SELECT COUNT(*) FROM t").await;
    match resp {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(5));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    db2.wal_buffer.shutdown().await;
}

/// T0b-1 (契约名 `eviction_scale_recovery_row_integrity`)：驱逐规模
/// 大 WAL 恢复后行数精确 + 索引判重探针。
///
/// 流程：CREATE TABLE t → 不 checkpoint → 1 万行 INSERT（显式事务，
/// 50 行/事务 × 200 批）→ `wal_buffer.shutdown()` + drop（不 close，
/// 不刷 data page）→ 重开 → 断言 COUNT(*) 精确 10000 + 重复 PK
/// INSERT 必须 DuplicateKey + 新 PK INSERT 必须成功。
///
/// RED 判据（修复前实测）：COUNT 虚增至 13190（重复追加）、DuplicateKey
/// 探针失败（缺索引条目）。
#[tokio::test]
async fn eviction_scale_recovery_row_integrity() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    // 契约：create_table 后不 checkpoint（catalog 走 buffer_pool.flush_all
    // 落盘，非 checkpoint 路径 — 夹具先例 checkpoint_redo_reduction_test.rs）
    db.buffer_pool.flush_all().await.unwrap();

    // 1 万行：50 行/显式事务 × 200 批
    for batch in 0..200u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({})", id), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    db.wal_buffer.shutdown().await;
    drop(db);

    // 重开触发 full_recover
    let db2 = Database::open(&path).await.unwrap();

    // 主断言：COUNT 精确 10000
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(10000),
                "恢复后 COUNT 必须精确 10000"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 探针 1：重复 PK → DuplicateKey（索引必须完整覆盖已恢复 1 万条）
    match db2.execute_sql("INSERT INTO t VALUES (42)").await {
        Response::Error { .. } => { /* 期望 DuplicateKey */ }
        other => panic!("重复 PK INSERT 应被拒，实际 {:?}", other),
    }

    // 探针 2：新 PK → 成功（行数变为 10001）
    match db2.execute_sql("INSERT INTO t VALUES (20000)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("新 PK INSERT 应成功，实际 {:?}", other),
    }
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(10001));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}

/// T0b-2 (契约名 `mixed_dml_recovery_semantics`)：混合 DML 重放语义
/// （R-T0b-R1 精确版，003-rework）。
///
/// 流程：create→checkpoint→5k（50/显式事务×100）→checkpoint（中位点，
/// spec R2-S1 前提）→5k→UPDATE 100 行（含最小键/中部/尾部，全部须成功）
/// →DELETE 50 行→shutdown+drop 不 close→重开断言全部精确：
/// COUNT(*)==9950、被更新行 v=9999（点查 + 全量口径抽查）、被删行 0 行、
/// 存活区间精确、重复 INSERT 最小键与中位点前键均 DuplicateKey。
///
/// 依赖：R-T0b-R5（catalog root 同步——中位点 checkpoint 后重开的索引
/// 可达性）+ R-T0b-R6（DataScan 被替代版本去重——UPDATE 后计数精确）。
///
/// RED（两修复落地前，Act 实测）：G4 使重开 `update redo: old key not in
/// index` 整体打开失败。
#[tokio::test]
async fn mixed_dml_recovery_semantics() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    db.checkpoint().await.unwrap();

    // 前半段 5k（中位点 checkpoint 前）
    for batch in 0..100u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({}, {})", id, id * 10), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }
    // 中位点 checkpoint（spec R2-S1 前提：site 前已有索引条目落盘）
    db.checkpoint().await.unwrap();

    // 后半段 5k（中位点后，触发 redo）
    for batch in 100..200u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({}, {})", id, id * 10), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    // UPDATE 100 行：最小键 0..49 + 中部 4970..4999 + 尾部 9980..9999
    // （与 DELETE 域 200..250 不相交）；全部须成功（R4 后可达成）
    let mut updated = 0usize;
    let update_ranges: [std::ops::Range<i64>; 3] = [0..50, 4970..5000, 9980..10000];
    for range in update_ranges {
        for id in range.clone() {
            match db
                .execute_sql(&format!("UPDATE t SET v = 9999 WHERE id = {}", id))
                .await
            {
                Response::AffectedRows { count: 1 } => updated += 1,
                other => panic!("UPDATE id={} 应成功，实际 {:?}", id, other),
            }
        }
    }
    assert_eq!(updated, 100, "契约要求恰好 100 行 UPDATE 全部成功");

    // DELETE 50 行（200..250）
    for id in 200..250i64 {
        match db
            .execute_sql(&format!("DELETE FROM t WHERE id = {}", id))
            .await
        {
            Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
            other => panic!("DELETE id={} 应成功，实际 {:?}", id, other),
        }
    }

    db.wal_buffer.shutdown().await;
    drop(db);

    // 重开
    let db2 = Database::open(&path).await.unwrap();

    // ① COUNT 精确 9950 = 10000 − 50（R6 后 UPDATE 不改变行数，零容差）
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(9950),
                "恢复后 COUNT 必须精确 9950"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // ② 被更新行 v=9999：点查（索引路径，覆盖最小键/中部/尾部采样点）
    for id in [0i64, 25, 49, 4970, 4999, 9980, 9999] {
        match db2
            .execute_sql(&format!("SELECT v FROM t WHERE id = {}", id))
            .await
        {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 1, "id={} 必须命中", id);
                assert_eq!(rows[0][0], serde_json::json!(9999), "id={} v 应为 9999", id);
            }
            other => panic!("SELECT id={} 失败: {:?}", id, other),
        }
    }

    // ③ 全量口径抽查：v=9999 恰好 100 行（v 无索引 → DataScan + 谓词路径；
    // 值正确性守护——判别双计的主断言是 ① 的 COUNT(*)==9950）
    match db2
        .execute_sql("SELECT COUNT(*) FROM t WHERE v = 9999")
        .await
    {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(100),
                "全量口径 v=9999 必须恰好 100 行（被更新行的新版本）"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // ④ 被删行 0 行
    for id in [200i64, 220, 249] {
        match db2
            .execute_sql(&format!("SELECT * FROM t WHERE id = {}", id))
            .await
        {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 0, "Delete 后 id={} 不可见", id);
            }
            other => panic!("Expected QueryResult, got {:?}", other),
        }
    }

    // ⑤ 存活区间精确：id ∈ [100, 200) 共 100 行（既有 loose ≥90 精确化）
    match db2
        .execute_sql("SELECT COUNT(*) FROM t WHERE id >= 100 AND id < 200")
        .await
    {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(100),
                "未删区间必须精确 100 行"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // ⑥ 唯一性：重复 INSERT 最小键与中位点前键均 DuplicateKey
    for id in [0i64, 300] {
        match db2
            .execute_sql(&format!("INSERT INTO t VALUES ({}, 0)", id))
            .await
        {
            Response::Error { .. } => { /* 期望 DuplicateKey */ }
            other => panic!("恢复后重复 INSERT id={} 应被拒，实际 {:?}", id, other),
        }
    }

    db2.wal_buffer.shutdown().await;
}

/// R-T0b-R6 (契约名 `count_after_update_exact`)：DataScan 对被更新行版本链
/// 只产出最新可见版本——运行期计数精确（与恢复后同源，同一 executor）。
///
/// 100 行 + 10 UPDATE（覆盖最小键/中部/尾部）→ COUNT 精确 100 + 被更新行
/// 点查新值。
///
/// RED（G5，Plan 独立复现）：100 行 + 10 UPDATE → COUNT=110（新旧版本
/// 双计——被新版本 `next_version` 指向的旧 slot 自身 header 可见即被照常
/// 产出）；全量口径 v=9999 计 20。
#[tokio::test]
async fn count_after_update_exact() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;

    for i in 0..100i64 {
        match db
            .execute_sql(&format!("INSERT INTO t VALUES ({}, {})", i, i))
            .await
        {
            Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
            other => panic!("INSERT id={} 应成功，实际 {:?}", i, other),
        }
    }

    // 10 行 UPDATE（覆盖最小键/中部/尾部）
    let update_targets = [0i64, 5, 10, 25, 50, 60, 75, 88, 95, 99];
    for id in update_targets {
        match db
            .execute_sql(&format!("UPDATE t SET v = 9999 WHERE id = {}", id))
            .await
        {
            Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
            other => panic!("UPDATE id={} 应成功，实际 {:?}", id, other),
        }
    }

    // 主断言：COUNT 精确 100（RED：110——被更新行新旧版本双计）
    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(100),
                "UPDATE 后 COUNT 必须精确 100（版本链只产出最新可见版本）"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 点查新值（索引路径）
    for id in [0i64, 50, 99] {
        match db
            .execute_sql(&format!("SELECT v FROM t WHERE id = {}", id))
            .await
        {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 1, "id={} 必须命中", id);
                assert_eq!(rows[0][0], serde_json::json!(9999), "id={} v 应为 9999", id);
            }
            other => panic!("SELECT id={} 失败: {:?}", id, other),
        }
    }

    // 全量口径抽查：v=9999 恰好 10 行（v 无索引 → DataScan + 谓词路径；
    // 值正确性守护——判别双计的主断言是上方 COUNT(*)）
    match db
        .execute_sql("SELECT COUNT(*) FROM t WHERE v = 9999")
        .await
    {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(10));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 未更新行守护：v 保持原值且只出现一次（id=1 → v=1）
    match db.execute_sql("SELECT v FROM t WHERE id = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], serde_json::json!(1));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R-T0b-R6 跨页场景：更新行分属 ≥3 数据页（实测 ~87 行/页，300 行 ≈ 4 页），
/// UPDATE 后计数仍精确。
///
/// RED：3 行跨页 UPDATE → COUNT=303。
#[tokio::test]
async fn count_after_cross_page_update_exact() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;

    // 300 行：50/显式事务 × 6 批（页容量 ~87 行 → 分属 4 页）
    for batch in 0..6u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({}, {})", id, id), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    // 更新行分属 ≥3 数据页：id=0（第 1 页）、id=100（第 2 页）、id=250（第 3+ 页）
    for id in [0i64, 100, 250] {
        match db
            .execute_sql(&format!("UPDATE t SET v = 9999 WHERE id = {}", id))
            .await
        {
            Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
            other => panic!("UPDATE id={} 应成功，实际 {:?}", id, other),
        }
    }

    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(300),
                "跨页 UPDATE 后 COUNT 必须精确 300"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 跨页更新行的点查新值
    for id in [0i64, 100, 250] {
        match db
            .execute_sql(&format!("SELECT v FROM t WHERE id = {}", id))
            .await
        {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0][0], serde_json::json!(9999));
            }
            other => panic!("SELECT id={} 失败: {:?}", id, other),
        }
    }

    db.wal_buffer.shutdown().await;
}

/// T0b-3 (契约名 `recovery_rerun_is_idempotent`)：重放重跑幂等。
///
/// 流程同 ① 至重开（恢复后 COUNT=10000）→ 不写入、不 checkpoint、
/// `wal_buffer.shutdown()` + drop → 再次重开 → COUNT 仍 10000。
/// 修复前：第二次 open 再次 replay → 重复追加，COUNT 虚增。
#[tokio::test]
async fn recovery_rerun_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.buffer_pool.flush_all().await.unwrap();

    for batch in 0..200u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({})", id), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    db.wal_buffer.shutdown().await;
    drop(db);

    // 第一次重开
    let db1 = Database::open(&path).await.unwrap();
    match db1.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(10000),
                "第一次重开 COUNT 10000"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    db1.wal_buffer.shutdown().await;
    drop(db1);

    // 第二次重开（WAL 仍在，无 checkpoint 截断）— 修复前会再 append 1 万
    let db2 = Database::open(&path).await.unwrap();
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(10000),
                "第二次重开 COUNT 必须仍 10000（幂等）"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}
