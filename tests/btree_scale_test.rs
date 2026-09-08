// BTree 多页规模正确性测试（Cycle 002-rework）
//
// 覆盖 R-T0b-R2（最小键搜索盲区）、R-T0b-R3（delete Page-full 泄漏）、
// R-T0b-R4（update 内部节点递归）。本文件只对 Database 公开 SQL 接口
// 行为做断言，与 wal_recovery_large_test 的 G1 触发路径一致：
// 50 行/显式事务 × 200 批 → 形成 10k 行 B-Tree。
use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// R-T0b-R2: 10k 树（50/显式事务）→ 重复 INSERT 最小键必须 DuplicateKey
///
/// 触发路径与 `eviction_scale_recovery_row_integrity` 一致：50 行/显式事务
/// × 200 批，构造多层 B-Tree。修复前：search 最小键 0 不可达 → INSERT 0
/// 走非 DuplicateKey 路径被接受（违反 R1-S2 唯一性约束）。
#[tokio::test]
async fn min_key_searchable_at_scale() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;

    // 50 行/显式事务 × 200 批 = 10000 行，触发多层 B-Tree
    for batch in 0..200u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({})", id), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    // 主断言：最小键 0 重复 INSERT 必须 DuplicateKey
    match db.execute_sql("INSERT INTO t VALUES (0)").await {
        Response::Error { .. } => { /* 期望 DuplicateKey */ }
        other => panic!("重复 INSERT 最小键 (id=0) 应被拒，实际 {:?}", other),
    }

    // 对照：次小键 (id=1) 与中部/尾部键也必须被拒
    for i in [1i64, 100, 500, 5_000, 9_999] {
        match db
            .execute_sql(&format!("INSERT INTO t VALUES ({})", i))
            .await
        {
            Response::Error { .. } => { /* 期望 DuplicateKey */ }
            other => panic!("重复 INSERT id={} 应被拒，实际 {:?}", i, other),
        }
    }

    // 守护：新 PK 仍可成功
    match db.execute_sql("INSERT INTO t VALUES (20000)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("新 PK INSERT 应成功，实际 {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R-T0b-R2 恢复侧: 10k 树崩溃重开后，重复 INSERT 最小键仍必须 DuplicateKey
///
/// 流程：建表（DDL 走 buffer_pool.flush_all 落盘，不走 checkpoint）→ 1 万行
/// INSERT（50/显式事务 × 200 批）→ shutdown+drop → 重开 → 重复 INSERT
/// id=0 必须 DuplicateKey。
#[tokio::test]
async fn min_key_searchable_after_recovery() {
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

    let db2 = Database::open(&path).await.unwrap();

    // COUNT 守护：恢复后 10000 行（含新 INSERT 后 10001）
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(10000));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 主断言：重复 INSERT id=0 必须 DuplicateKey
    match db2.execute_sql("INSERT INTO t VALUES (0)").await {
        Response::Error { .. } => { /* 期望 DuplicateKey */ }
        other => panic!("恢复后重复 INSERT id=0 应被拒，实际 {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}

/// R-T0b-R4: 10k 树 UPDATE 内部节点递归（覆盖最小键/中段/尾部）
///
/// 修复前：`BTree::update_in_page` 内部节点分支直接 `Err`，10k 树
/// （高度 ≥2）上 UPDATE 全部失败。修复后：UPDATE 沿内部节点递归下探
/// 至叶更新；键不存在保持 KeyNotFound。
#[tokio::test]
async fn update_works_at_scale() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;

    for batch in 0..200u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({}, {})", id, id * 10), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    // UPDATE 边界/中段/尾部/最小键（最小键 G1 修复后已可达）
    let update_targets = [0i64, 1, 100, 500, 5_000, 9_999];
    for id in update_targets {
        match db
            .execute_sql(&format!("UPDATE t SET v = 9999 WHERE id = {}", id))
            .await
        {
            Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
            other => panic!("UPDATE id={} 应成功，实际 {:?}", id, other),
        }
    }

    // 点查：UPDATE 后 SELECT 应返回 v=9999
    for id in update_targets {
        match db
            .execute_sql(&format!("SELECT v FROM t WHERE id = {}", id))
            .await
        {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 1, "id={} 必须命中", id);
                assert_eq!(
                    rows[0][0],
                    serde_json::json!(9999),
                    "id={} UPDATE 后 v 应为 9999",
                    id
                );
            }
            other => panic!("SELECT id={} 失败: {:?}", id, other),
        }
    }

    // 守护：未 UPDATE 的键值不变
    match db.execute_sql("SELECT v FROM t WHERE id = 2").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(20),
                "id=2 未 UPDATE，v 应为 20"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R-T0b-R3: 10k 树批量 DELETE（跨区段，触发 Page-full 路径）
///
/// 修复前：`BTree::delete` 重平衡路径（`redistribute_*` / `merge_*` /
/// `handle_child_merge`）在某些根因下偶发 `Page full`（Plan 实测
/// id=242）；无中位点 checkpoint 混合 WAL 重开亦因该问题整体打开失败。
/// 修复后：跨区段批量 DELETE 全部成功。
#[tokio::test]
async fn bulk_delete_at_scale() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;

    for batch in 0..200u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({})", id), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }

    // 批量 DELETE ≥200 键（跨区段：低/中/高三段各 70 个，避开端点）
    let mut to_delete: Vec<i64> = Vec::new();
    for i in 100..170 {
        to_delete.push(i);
    }
    for i in 5_000..5_070 {
        to_delete.push(i);
    }
    for i in 9_800..9_860 {
        to_delete.push(i);
    }
    assert!(to_delete.len() >= 200, "跨区段 ≥200 键");

    let mut ok_count = 0u64;
    for id in &to_delete {
        match db
            .execute_sql(&format!("DELETE FROM t WHERE id = {}", id))
            .await
        {
            Response::AffectedRows { count: 1 } => ok_count += 1,
            other => panic!("DELETE id={} 应成功，实际 {:?}", id, other),
        }
    }
    assert_eq!(ok_count, to_delete.len() as u64);

    // 守护：COUNT 精确
    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(10000 - to_delete.len() as i64),
                "删除后 COUNT 必须精确"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 守护：被删行不再可见
    for id in to_delete.iter().take(10) {
        match db
            .execute_sql(&format!("SELECT * FROM t WHERE id = {}", id))
            .await
        {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 0, "已删 id={} 不可见", id);
            }
            other => panic!("SELECT 已删 id={} 失败: {:?}", id, other),
        }
    }

    db.wal_buffer.shutdown().await;
}

/// R-T0b-R5: 根分裂后 catalog root 同步——中位点 checkpoint 混合负载
/// 崩溃重开后，site 前条目经恢复的索引可达。
///
/// 流程：create→checkpoint→5k（50/显式事务×100）→checkpoint（中位点）→
/// 5k→shutdown+drop 不 close→重开→重复 INSERT 中位点前键必须
/// DuplicateKey。
///
/// 探针选点依据：根分裂把原始叶保留为最左孩子且升序插入后其内容冻结在
/// 首批 ~46 键（叶容量 92 的一半），故中位点前键取 id=300（越过 stale
/// root 叶容量，且 <5000 不在 redo 窗口）；id=0（stale root 内）与
/// id=6000（redo 窗口内）为对照。
///
/// RED 形态（G4，Act 实测）：catalog `index_root_page_id` 停留在建表时
/// 初始根页，重开从 stale root 重建索引——id=300 不可达，重复 INSERT
/// 被接受（预期 DuplicateKey 实际 AffectedRows）。open 本身成功（纯
/// INSERT redo 无 old-key 查找）。
#[tokio::test]
async fn root_sync_survives_midpoint_checkpoint() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.checkpoint().await.unwrap();

    // 前半段 5k（中位点 checkpoint 前）
    for batch in 0..100u64 {
        let tx = db.begin().await.unwrap();
        for i in 0..50u64 {
            let id = (batch * 50 + i) as i64;
            db.execute_in_tx(&format!("INSERT INTO t VALUES ({})", id), &tx)
                .await;
        }
        db.commit(tx).await.unwrap();
    }
    // 中位点 checkpoint：catalog 页（含 root 镜像）+ 数据页 + B-Tree 落盘
    db.checkpoint().await.unwrap();

    // 后半段 5k（中位点后，触发 redo）
    for batch in 100..200u64 {
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

    let db2 = Database::open(&path).await.unwrap();

    // 数据页守卫：COUNT 精确 10000（数据页由 T0b 位置寻址保证，与 G4 正交）
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(10000));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 主断言：中位点前键 id=300 重复 INSERT 必须 DuplicateKey
    // （site 前条目必须经恢复后的索引可达）
    match db2.execute_sql("INSERT INTO t VALUES (300)").await {
        Response::Error { .. } => { /* 期望 DuplicateKey */ }
        other => panic!(
            "恢复后重复 INSERT 中位点前键 id=300 应被拒，实际 {:?}",
            other
        ),
    }

    // 对照：最小键（stale root 内）与中位点后键（redo 窗口内）同样必须判重
    for id in [0i64, 6_000] {
        match db2
            .execute_sql(&format!("INSERT INTO t VALUES ({})", id))
            .await
        {
            Response::Error { .. } => { /* 期望 DuplicateKey */ }
            other => panic!("恢复后重复 INSERT id={} 应被拒，实际 {:?}", id, other),
        }
    }

    db2.wal_buffer.shutdown().await;
}
