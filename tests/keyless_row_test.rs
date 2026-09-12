//! MS10-T05 Iteration 001 / 001-rework：无键行落库语义与崩溃恢复
//!
//! change: 2026-09-09-ms10-t05-lifecycle-subcommands / repair items T8-R1、T8-R2
//!
//! 引擎现状（001-rework Plan Context 证据链）：键位（PK 列，未声明时为第一列）
//! 值不可键控（NULL / 非 Int）的行在 `InsertExecutor` 中被整行静默丢弃
//! （`to_key()` None → continue，无页写入、无 WAL、affected 少计）；恢复侧
//! Update 重放对无键 old_tuple 硬性 `RedoFailed`。
//!
//! 修复语义（用户裁定方向 A）：可键控行为逐字节不变；不可键控行落库但不入
//! 索引（无唯一性检查——SQLite NULL-PK 先例，文档化语义），恢复按 tuple
//! 原始字节建立 keyless 候选集推导 old_row_id。
//!
//! - R1 见证：String 首列隐式 PK 表整表可插；NULL 键位行落库可见；
//!   可键控重复插入守卫（DuplicateKey 保持）。
//! - R2 见证：无键行 INSERT + UPDATE 链 + 崩溃（drop 不 close）重开——
//!   MS10-T05 修复前 `RedoFailed` 使 `Database::open` 失败；修复后恢复
//!   成功、行数精确、键位转无键行可见、可键控行不受影响（MS15-Rest
//!   按 I037 修复语义校准：第二次键位等值 UPDATE 改断言 KeyNotFound）。
//!
//! UPDATE 可达性注记：`build_update` 要求 WHERE 为 `pk = 可键控值` 且经索引
//! 定位，INSERT 的无键行无法被 UPDATE 直接命中；产生「new_tuple 无键」的
//! Update WAL 记录的 SQL 可达路径是 SET 键列为 NULL 的更新
//! （keyed → keyless）。I037 修复后旧键索引条目随之删除，对无键版本的
//! 再次键位等值 UPDATE 在执行器 Step 1 即 KeyNotFound——「old_tuple 无键」
//! 的 old-lookup 重放子路径自此无 SQL 运行期生产者（legacy WAL 兼容面，
//! 恢复侧语义保持）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// T8-R1：String 首列（隐式 PK）表 INSERT 2 行必须落库可见。
///
/// RED（修复前实测）：两行均在 InsertExecutor 键位 `to_key()` None 分支被
/// 静默丢弃——affected 0、SELECT 0 行。
#[tokio::test]
async fn string_first_column_table_accepts_inserts() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (s STRING, v INT)").await;

    match db.execute_sql("INSERT INTO t VALUES ('x', 1)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("String 键位 INSERT 应 affected 1，实际 {:?}", other),
    }
    match db.execute_sql("INSERT INTO t VALUES ('y', 2)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("String 键位 INSERT 应 affected 1，实际 {:?}", other),
    }

    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(2),
                "String 隐式 PK 表的两行必须都落库"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 非 PK 谓词走 DataScan 下推（M19 pushdown），可达无键行
    match db.execute_sql("SELECT s FROM t WHERE v = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "WHERE v = 1 必须命中无键行");
            assert_eq!(rows[0][0], serde_json::json!("x"));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// T8-R1：NULL 键位行 INSERT → affected 1 且 SELECT 可见。
///
/// RED（修复前实测）：整行静默丢弃——affected 0、SELECT 空行集。
#[tokio::test]
async fn null_key_position_row_insert_visible() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (a INT, b INT)").await;

    match db.execute_sql("INSERT INTO t VALUES (NULL, 7)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("NULL 键位 INSERT 应 affected 1，实际 {:?}", other),
    }

    match db.execute_sql("SELECT a, b FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(null), serde_json::json!(7)]],
                "NULL 键位行必须落库可见"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// T8-R1 行为保持守卫：可键控（Int 键位）重复插入仍被 DuplicateKey 拒绝。
/// 变更前后恒 GREEN。
#[tokio::test]
async fn keyed_duplicate_still_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT)").await;

    match db.execute_sql("INSERT INTO t VALUES (1)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("首次 INSERT 应成功，实际 {:?}", other),
    }
    match db.execute_sql("INSERT INTO t VALUES (1)").await {
        Response::Error { .. } => { /* 期望 DuplicateKey */ }
        other => panic!("重复键位 INSERT 应被拒，实际 {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// T8-R2：无键行 INSERT + UPDATE 链 + 崩溃重开（MS15-Rest 001-replan 按
/// I037 修复语义校准）。
///
/// 流程：CREATE TABLE t (a INT, v INT)（隐式 PK = a）→ flush_all 持久化
/// catalog（DDL 无 WAL 记录）→ INSERT 无键行 (NULL,1) + 可键控行 (5,0)、
/// (7,100) → UPDATE 键位转无键（SET a = NULL：old_tuple 键控、new_tuple
/// 无键，旧键条目删除——恢复侧 keyless 桶 NEW 版本重放见证）→ 对无键版本
/// 的再次键位等值 UPDATE 被拒（KeyNotFound：残留条目已消除，键位等值
/// 不可达）→ shutdown + drop 不 close → 重开。
///
/// GREEN：恢复成功；COUNT 精确 3（无键插入行 + 键位转无键行 + 可键控行）；
/// 键位转无键行 (NULL,0) 经非键谓词可见（keyless NEW_tuple 重放见证）；
/// 无键插入行 v=1 可见；可键控行点查与重复拒绝守卫保持；重开后旧键 5
/// INSERT 成功（恢复侧索引无残留条目，与运行期 delete 分支一致）。
#[tokio::test]
async fn keyless_row_update_recovery_after_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    db.execute_sql("CREATE TABLE t (a INT, v INT)").await;
    // DDL 无 WAL 记录：catalog 经 buffer_pool 落盘（夹具先例
    // wal_recovery_large_test.rs），否则重开 redo 必 table not found
    db.buffer_pool.flush_all().await.unwrap();

    match db.execute_sql("INSERT INTO t VALUES (NULL, 1)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库（T8-R1） */ }
        other => panic!("无键行 INSERT 应 affected 1，实际 {:?}", other),
    }
    match db.execute_sql("INSERT INTO t VALUES (5, 0)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("可键控行 INSERT 应 affected 1，实际 {:?}", other),
    }
    match db.execute_sql("INSERT INTO t VALUES (7, 100)").await {
        Response::AffectedRows { count: 1 } => { /* 期望落库 */ }
        other => panic!("可键控行 INSERT 应 affected 1，实际 {:?}", other),
    }

    // 键行 → 无键：v2 (NULL, 0)；I037 修复后旧键 5 条目删除（new_tuple
    // 无键 → 恢复侧 keyless 桶 NEW 版本重放见证）
    match db.execute_sql("UPDATE t SET a = NULL WHERE a = 5").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("UPDATE SET a = NULL 应成功，实际 {:?}", other),
    }
    // 对无键版本的再次键位等值 UPDATE：残留条目已消除，执行器 Step 1
    // search(5) → None → KeyNotFound（I037 修复语义——键位无键行对键位
    // 等值不可达，R1 新场景直接见证）
    match db.execute_sql("UPDATE t SET v = 42 WHERE a = 5").await {
        Response::Error { message } => {
            assert!(
                message.contains("Key not found"),
                "对无键版本的键位等值 UPDATE 应 KeyNotFound，实际 {message}"
            );
        }
        other => panic!("对无键版本的键位等值 UPDATE 应被拒，实际 {:?}", other),
    }

    db.wal_buffer.shutdown().await;
    drop(db); // 崩溃模拟：不 close（不 checkpoint、不刷数据页）

    // 恢复重放 keyed→keyless Update 记录（keyless 桶 NEW 版本追踪）
    let db2 = Database::open(&path).await.unwrap();

    // 行数精确：无键插入行 + 更新后的无键行 + 可键控行
    match db2.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(3), "恢复后 COUNT 必须精确 3");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 键位转无键行 (NULL,0) 可见（v 无索引 → DataScan + 谓词全扫描，
    // keyless NEW_tuple 重放见证；第二次 UPDATE 已 KeyNotFound，无 v=42 行）
    match db2.execute_sql("SELECT COUNT(*) FROM t WHERE v = 0").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows[0][0],
                serde_json::json!(1),
                "键位转无键行 (NULL,0) 必须可见"
            );
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 无键插入行可见（v=1 唯一，对照行 v=100）
    match db2.execute_sql("SELECT COUNT(*) FROM t WHERE v = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(1), "无键插入行必须可见");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // 可键控行恢复不受影响：点查 + 索引判重守卫
    match db2.execute_sql("SELECT v FROM t WHERE a = 7").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "可键控行点查必须命中");
            assert_eq!(rows[0][0], serde_json::json!(100));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    match db2.execute_sql("INSERT INTO t VALUES (7, 0)").await {
        Response::Error { .. } => { /* 期望 DuplicateKey（重建索引完整性） */ }
        other => panic!("恢复后重复键 INSERT 应被拒，实际 {:?}", other),
    }
    match db2.execute_sql("INSERT INTO t VALUES (8, 0)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("恢复后新键 INSERT 应成功，实际 {:?}", other),
    }
    // 旧键 5 重开 INSERT 成功：恢复侧索引无残留条目（与运行期 delete
    // 分支一致——R1-S3 一致性收口）
    match db2.execute_sql("INSERT INTO t VALUES (5, 200)").await {
        Response::AffectedRows { count: 1 } => { /* 期望成功 */ }
        other => panic!("重开后旧键 5 INSERT 应成功，实际 {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}
