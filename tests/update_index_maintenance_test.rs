//! MS15-Rest Iteration 001：UPDATE 键位无键值索引条目清理（I037）
//!
//! change: 2026-09-12-ms15-rest-correctness-batch / tasks T4-T6
//!
//! 缺陷（I037）：`UpdateExecutor` Step 7 对新值不问可键控性，无条件
//! `index_manager.update(&self.key, new_row_id)`——`UPDATE SET <键列> = NULL`
//! 后旧键条目指向键位已为 NULL 的版本：对旧键值 INSERT 被 `DuplicateKey`
//! 误拒（无行实际持有该键）、旧键点查经残留条目返回键位为 NULL 的行、
//! 崩溃恢复重建（无键版本不入索引）后自愈——运行期/恢复两态不一致。
//!
//! 修复语义（design D2）：SET 目标列为键列且新值 `to_key()==None` 时，
//! Step 7 改为 `index_manager.delete(&self.key)`；其余形态保持 `update`。
//!
//! - R1 见证：键位置 NULL 后旧键 INSERT 不再误拒；旧键点查空集；
//!   崩溃恢复两态一致（运行期与恢复面同断言）。
//! - R2 见证：非键列 SET 条目保持；键列原值 SET 条目保持（既有语义锚点，
//!   变更前后恒 GREEN）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// R1-S1：键位置 NULL 后旧键 INSERT 不再被残留条目误拒。
///
/// RED（修复前实测）：UPDATE 后索引仍持旧键条目（指向键位 NULL 的新版本），
/// `INSERT INTO t VALUES (5, 200)` 被误拒 `DuplicateKey`。
#[tokio::test]
async fn old_key_insert_accepted_after_key_nulled() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = NULL WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("UPDATE SET id = NULL 应成功，实际 {:?}", other),
    }

    // 修复前：残留索引条目 → DuplicateKey 误拒（RED）
    match db.execute_sql("INSERT INTO t VALUES (5, 200)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("旧键 INSERT 不应被残留条目误拒，实际 {:?}", other),
    }

    // 表含两行：无键行 (NULL, 100) + 新键行 (5, 200)
    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(2), "表必须恰含两行");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db.execute_sql("SELECT COUNT(*) FROM t WHERE v = 100").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(1),
                "无键行 (NULL, 100) 必须可达"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db.execute_sql("SELECT COUNT(*) FROM t WHERE v = 200").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(1), "新键行 (5, 200) 必须可达");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R1-S2：键位置 NULL 后旧键点查返回空集。
///
/// RED（修复前实测）：`SELECT * FROM t WHERE id = 5` 经残留索引条目返回
/// 键位已为 NULL 的行 `(NULL, 100)`（索引信任无残差校验）。
#[tokio::test]
async fn old_key_point_query_empty_after_key_nulled() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = NULL WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("UPDATE SET id = NULL 应成功，实际 {:?}", other),
    }

    // 修复前：经残留条目返回无键行（RED）
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0, "旧键点查必须为空集（残留条目已清理）");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 无键行本身仍可经非键谓词可达
    match db.execute_sql("SELECT COUNT(*) FROM t WHERE v = 100").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(1), "无键行必须经非键谓词可达");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R1-S3：键位置 NULL 后崩溃恢复两态一致。
///
/// 流程：建表 → flush_all 持久化 catalog（DDL 无 WAL 记录）→ INSERT (5,100)
/// → UPDATE SET id = NULL → 崩溃前先断言运行期点查空集（两态比较面）→
/// shutdown + drop 不 close → 重开（WAL 重放 + 索引重建）。
///
/// RED（修复前实测）：崩溃前运行期点查经残留条目返回无键行——运行期与
/// 恢复面（重建索引无键 5 条目）不一致。
/// GREEN：恢复后行 (NULL, 100) 经非键谓词可达、`WHERE id = 5` 空集、
/// `INSERT (5, 200)` 成功——与运行期行为一致。
#[tokio::test]
async fn recovery_matches_runtime_after_key_nulled() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    // DDL 无 WAL 记录：catalog 经 buffer_pool 落盘（夹具先例
    // keyless_row_test.rs），否则重开 redo 必 table not found
    db.buffer_pool.flush_all().await.unwrap();

    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = NULL WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("UPDATE SET id = NULL 应成功，实际 {:?}", other),
    }

    // 运行期两态比较面：修复前经残留条目返回无键行（RED）
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0, "运行期旧键点查必须为空集");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
    drop(db); // 崩溃模拟：不 close（不 checkpoint、不刷数据页）

    let db2 = Database::open(&path).await.unwrap();

    // 恢复面：无键行 (NULL, 100) 可达
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(1), "恢复后必须恰含无键行");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db2
        .execute_sql("SELECT COUNT(*) FROM t WHERE v = 100")
        .await
    {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(1), "无键行必须经非键谓词可达");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 恢复面：旧键点查空集（重建索引无键 5 条目）
    match db2.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0, "恢复后旧键点查必须为空集");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 恢复面：旧键 INSERT 成功——与运行期行为一致（两态一致）
    match db2.execute_sql("INSERT INTO t VALUES (5, 200)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("恢复后旧键 INSERT 不应被误拒，实际 {:?}", other),
    }
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(2), "INSERT 后表必须恰含两行");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}

/// R1-S4：非键列更新不影响索引条目。
///
/// 行为保持锚点：变更前后恒 GREEN（Step 7 else 臂既有 `update` 路径）。
#[tokio::test]
async fn non_key_column_update_keeps_index_entry() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET v = 200 WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("非键列 UPDATE 应成功，实际 {:?}", other),
    }

    // 索引条目 5 仍指向该行：点查返回更新后的行
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5), serde_json::json!(200)]],
                "非键列 UPDATE 后旧键条目必须保持"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R2-S1：键列原值更新条目保持。
///
/// 行为保持锚点：SET 键列为原可键控值（`to_key()==Some`）走既有 `update`
/// 路径，变更前后恒 GREEN。
#[tokio::test]
async fn key_column_same_value_update_keeps_entry() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = 5 WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("键列原值 UPDATE 应成功，实际 {:?}", other),
    }

    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5), serde_json::json!(100)]],
                "键列原值 UPDATE 后条目必须保持"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

// ============================================================================
// MS16 Iteration 001（I047）：UPDATE rekey 索引一致性
// ============================================================================
//
// change: 2026-09-12-ms16-correctness-batch / tasks T5-T6
//
// 缺陷（I047）：Step 7 else 臂对键列 SET 为另一可键控值（rekey）无条件
// `index_manager.update(&self.key, new_row_id)`——旧键条目残留指向键位已改的
// 新版本（旧键点查返回 (7, 100)、旧键 INSERT 被 DuplicateKey 误拒）、新键无
// 索引条目（点查静默空集）、碰撞 rekey 无预检静默改写已有行、崩溃恢复重建
// 后与运行期不一致。
//
// 修复语义（design D4）：写入前碰撞预检（新键命中即 DuplicateKey 拒绝、零
// 副作用）+ Step 7 三分支（NULL → delete（I037 原样）/ 同键 → update（原样）/
// rekey → 先 delete 旧键后 insert 新键，顺序固定）。

/// R4-S1：rekey 后新键点查可达。
///
/// RED（修复前推演）：else 臂只把旧键 5 条目的 RowId 改指新版本，新键 7 无
/// 索引条目——`WHERE id = 7` IndexScan 静默空集。
#[tokio::test]
async fn rekey_new_key_point_query_reachable() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = 7 WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("rekey UPDATE 应成功，实际 {:?}", other),
    }

    // 修复前：新键 7 无索引条目 → 空集（RED）
    match db.execute_sql("SELECT * FROM t WHERE id = 7").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(7), serde_json::json!(100)]],
                "rekey 后新键点查必须返回 rekeyed 行"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R4-S2：rekey 后旧键条目清理——旧键点查空集、旧键 INSERT 可用。
///
/// RED（修复前推演）：残留旧键 5 条目指向新版本——`WHERE id = 5` 返回键位
/// 已为 7 的行 `(7, 100)`；`INSERT (5, 200)` 被 DuplicateKey 误拒。
#[tokio::test]
async fn rekey_old_key_cleaned_insert_available() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = 7 WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("rekey UPDATE 应成功，实际 {:?}", other),
    }

    // 修复前：经残留条目返回 (7, 100)（RED）
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0, "rekey 后旧键点查必须为空集");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    // 修复前：DuplicateKey 误拒（RED）
    match db.execute_sql("INSERT INTO t VALUES (5, 200)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("rekey 后旧键 INSERT 不应被残留条目误拒，实际 {:?}", other),
    }
    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(2), "表必须恰含两行");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R4-S3：碰撞 rekey 在任何写入前以 DuplicateKey 拒绝，零副作用。
///
/// 行 (5, 100)、(7, 200)，`SET id = 7 WHERE id = 5`。
/// RED（修复前推演）：无碰撞预检——UPDATE 成功 affected 1，行 (5, 100) 被
/// 静默改写为 (7, 100)。
#[tokio::test]
async fn rekey_collision_rejected_before_write_zero_side_effects() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("INSERT INTO t VALUES (7, 200)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("第二次 INSERT 应成功，实际 {:?}", other),
    }

    // 修复前：无预检 → 成功 affected 1，行被静默改写（RED）
    match db.execute_sql("UPDATE t SET id = 7 WHERE id = 5").await {
        Response::Error { message } => {
            assert!(
                message.to_lowercase().contains("duplicate"),
                "碰撞 rekey 必须报 DuplicateKey，实际: {message}"
            );
        }
        other => panic!("碰撞 rekey 必须被拒绝，实际 {:?}", other),
    }

    // 零副作用：两行原样
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5), serde_json::json!(100)]],
                "被拒绝的 rekey 不得改写行 (5, 100)"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db.execute_sql("SELECT * FROM t WHERE id = 7").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(7), serde_json::json!(200)]],
                "被拒绝的 rekey 不得改写行 (7, 200)"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(2), "表必须恰含两行");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// R4-S4：rekey 后崩溃恢复两态一致。
///
/// 流程：建表 → flush_all 持久化 catalog（DDL 无 WAL 记录）→ INSERT (5,100)
/// → UPDATE SET id = 7 → 运行期断言新键可达/旧键空集（两态比较面）→
/// shutdown + drop 不 close → 重开（WAL 重放 + 索引重建）→ 同断言。
///
/// RED（修复前推演）：运行期新键 7 无条目点查空集、旧键 5 经残留条目返回
/// (7, 100)——恢复面（重建索引以数据页为准）却正确，两态不一致。
#[tokio::test]
async fn recovery_matches_runtime_after_rekey() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;
    // DDL 无 WAL 记录：catalog 经 buffer_pool 落盘（夹具先例），否则重开
    // redo 必 table not found
    db.buffer_pool.flush_all().await.unwrap();

    match db.execute_sql("INSERT INTO t VALUES (5, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("UPDATE t SET id = 7 WHERE id = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("rekey UPDATE 应成功，实际 {:?}", other),
    }

    // 运行期两态比较面（修复前：新键空集 + 旧键返回 (7, 100)，RED）
    match db.execute_sql("SELECT * FROM t WHERE id = 7").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(7), serde_json::json!(100)]],
                "运行期新键点查必须可达"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0, "运行期旧键点查必须为空集");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
    drop(db); // 崩溃模拟：不 close（不 checkpoint、不刷数据页）

    let db2 = Database::open(&path).await.unwrap();

    // 恢复面同断言（两态一致）
    match db2.execute_sql("SELECT * FROM t WHERE id = 7").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(7), serde_json::json!(100)]],
                "恢复后新键点查必须可达"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db2.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0, "恢复后旧键点查必须为空集");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    // 恢复面：旧键 INSERT 成功（与运行期行为一致）
    match db2.execute_sql("INSERT INTO t VALUES (5, 200)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("恢复后旧键 INSERT 不应被误拒，实际 {:?}", other),
    }
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(2), "INSERT 后表必须恰含两行");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}
