//! change 2026-10-23-ms33-drop-persistence-failure / Iteration 000（T01/I086）：
//! checkpoint 前崩溃时 DROP 类的可观察结果。
//!
//! 崩溃模型沿用 `ddl_crash_recovery_test.rs` 先例：WAL shutdown（不落
//! checkpoint）后 drop 句柄（不 close）→ 脏页丢失 → 重开走恢复。该模型是
//! 进程内丢弃句柄，不是真实断电。
//!
//! 每个用例先开一个会话把「已提交内容」真正写进磁盘文件（close →
//! checkpoint，WAL 被截断），再开第二个会话执行 DROP 类语句并崩溃。若 WAL
//! 仍留着这些行，重开时的 WAL 重放会把行补回来——缺口就被掩盖了；只有行
//! 已经落盘且 WAL 不再持有它们时，释放页被清零造成的数据丢失才可观察。
//!
//! T1.1：DROP TABLE 后崩溃——对象与 DROP 前的已提交内容仍在，
//! `check_integrity` 的 error 级发现为 0。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::integrity::{check_integrity, FindingSeverity};
use rtsql::storage::IoFailPoint;
use rtsql::transaction::IsolationLevel;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const KEY: &str = "pw";

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// 打开加密库（`.wal` 与 `.checkpoint` 保持明文，见 `database-encryption`）。
async fn open_encrypted(path: &Path) -> Database {
    Database::open_with_key(path, IsolationLevel::RepeatableRead, Some(KEY))
        .await
        .unwrap()
}

/// 执行单条 SQL 并断言成功。
async fn ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

/// 断言查询结果行集。
async fn rows(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql} must return rows: {other:?}"),
    }
}

/// 断言重开会话结构干净（`check_integrity` error 级发现为零）。
async fn assert_check_clean(db: &Database) {
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after crash recovery: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
}

/// 崩溃模拟：WAL 落盘后 drop（不 close，脏页不刷）。
async fn crash(db: Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

/// DROP TABLE 成功后不 checkpoint 即崩溃：被删的表与原行都在，另一张表的行
/// 也在，`check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn drop_table_crash_before_checkpoint_keeps_rows() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    // 探针：先让已提交行落盘（WAL 被 checkpoint 截断），再 DROP + 崩溃。
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        ok(&db, "CREATE TABLE other (id INT PRIMARY KEY, v INT)").await;
        ok(&db, "INSERT INTO other VALUES (1, 100)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "DROP TABLE t").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT id, n FROM t ORDER BY id").await,
        vec![
            vec![serde_json::json!(1), serde_json::json!(10)],
            vec![serde_json::json!(2), serde_json::json!(20)],
        ],
        "the dropped table must be intact after a crash before checkpoint"
    );
    assert_eq!(
        rows(&db2, "SELECT v FROM other").await,
        vec![vec![serde_json::json!(100)]],
        "an untouched table must be intact after a crash before checkpoint"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// DROP TABLE 成功后同一会话再分配：新建表只含新插入的行，读不到已释放页
/// 上的旧行。
#[tokio::test]
async fn drop_then_allocate_sees_only_new_rows() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    // 先让被删表的行落到磁盘上——被释放的页因此不是空页，再分配必须清零。
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "DROP TABLE t").await;
    // 同会话再分配：另一张表复用被释放的页。
    ok(&db, "CREATE TABLE u (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO u VALUES (7, 70)").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM u ORDER BY id").await,
        vec![vec![serde_json::json!(7), serde_json::json!(70)]],
        "a table built on a reused page must only see its own rows"
    );
    db.close().await.unwrap();
}

/// DROP INDEX 成功后不 checkpoint 即崩溃：索引仍在，按索引列的等值查询行集
/// 与崩溃前相同，`check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn drop_index_crash_before_checkpoint_keeps_index() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    // 索引树页先落盘——被释放的树页因此不是空页。
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        for i in 1..=200 {
            ok(&db, &format!("INSERT INTO t VALUES ({i}, {})", i % 10)).await;
        }
        ok(&db, "CREATE INDEX idx_n ON t (n)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    let before = rows(&db, "SELECT id FROM t WHERE n = 7 ORDER BY id").await;
    ok(&db, "DROP INDEX idx_n").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    let meta = db2.get_table("t").await.unwrap();
    assert!(
        meta.secondary_indexes.iter().any(|i| i.name == "idx_n"),
        "the secondary index must survive a crash before checkpoint"
    );
    assert_eq!(
        rows(&db2, "SELECT id FROM t WHERE n = 7 ORDER BY id").await,
        before,
        "the equality query must return the same row set after the crash"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// DROP VIEW 成功后不 checkpoint 即崩溃：视图仍可查询出基表的已提交行。
#[tokio::test]
async fn drop_view_crash_before_checkpoint_keeps_view() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10)").await;
        ok(&db, "CREATE VIEW v AS SELECT id, n FROM t").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "DROP VIEW v").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert!(
        db2.views.read().await.contains_key("v"),
        "the view must survive a crash before checkpoint"
    );
    assert_eq!(
        rows(&db2, "SELECT n FROM v WHERE id = 1").await,
        vec![vec![serde_json::json!(10)]],
        "the view must still return the base table rows after the crash"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// ALTER TABLE DROP COLUMN 成功后不 checkpoint 即崩溃：被删列不再存在，其余列
/// 的已提交值与删列后一致，`check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn alter_drop_column_crash_keeps_narrowed_rows() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        ok(
            &db,
            "CREATE TABLE t (id INT PRIMARY KEY, n INT, pad STRING)",
        )
        .await;
        ok(&db, "INSERT INTO t VALUES (1, 10, 'x')").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "ALTER TABLE t DROP COLUMN pad").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT id, n FROM t").await,
        vec![vec![serde_json::json!(1), serde_json::json!(10)]],
        "the remaining columns must keep their values after the crash"
    );
    match to_response(db2.execute_sql("SELECT pad FROM t").await) {
        Response::Error { .. } => {}
        other => panic!("the dropped column must not exist after the crash: {other:?}"),
    }
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// 加密库 CREATE TABLE 后不 checkpoint 即崩溃：同一密钥重开后表存在，插入的
/// 行可以读回。
#[tokio::test]
async fn encrypted_create_table_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = open_encrypted(&path).await;
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    crash(db).await;

    let db2 = open_encrypted(&path).await;
    ok(&db2, "INSERT INTO t VALUES (2, 20)").await;
    assert_eq!(
        rows(&db2, "SELECT id, n FROM t ORDER BY id").await,
        vec![
            vec![serde_json::json!(1), serde_json::json!(10)],
            vec![serde_json::json!(2), serde_json::json!(20)],
        ],
        "the encrypted table must survive a crash before checkpoint"
    );
    db2.close().await.unwrap();
}

/// 加密库 DROP TABLE 后不 checkpoint 即崩溃：结果与明文库同类——原行仍可
/// 查询，`check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn encrypted_drop_table_crash_keeps_rows() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = open_encrypted(&path).await;
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = open_encrypted(&path).await;
    ok(&db, "DROP TABLE t").await;
    crash(db).await;

    let db2 = open_encrypted(&path).await;
    assert_eq!(
        rows(&db2, "SELECT id, n FROM t ORDER BY id").await,
        vec![vec![serde_json::json!(1), serde_json::json!(10)]],
        "the encrypted dropped table must be intact after a crash before checkpoint"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// 受控 I/O 失败开关本体：arm 后下一次同型写入 / WAL fsync 在系统调用前
/// 返回固定标记错误并自清除；注入失败不触碰库文件（长度不变），未设置时
/// checkpoint 正常完成。
#[tokio::test]
async fn io_fail_switch_errors_without_touching_file() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;

    let len_before = std::fs::metadata(&path).unwrap().len();

    // DataWrite：下一次数据页写入在系统调用前失败。
    db.arm_io_failure(IoFailPoint::DataWrite);
    let err = db.checkpoint().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (write_page)"),
        "checkpoint must fail with the injected write marker: {err}"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        len_before,
        "an injected write failure must not touch the database file"
    );

    // 开关一次性、自清除：不 arm 的 checkpoint 正常完成（先重新置脏 +
    // 追加 WAL，避开零工作早退）。
    ok(&db, "INSERT INTO t VALUES (2, 20)").await;
    db.checkpoint().await.unwrap();

    // WalFsync：下一次 WAL fsync 在 sync_all 之前失败。此前需再追加 WAL
    // 记录，否则 checkpoint 走零工作早退、触达不了 fsync。
    ok(&db, "INSERT INTO t VALUES (3, 30)").await;
    db.arm_io_failure(IoFailPoint::WalFsync);
    let err = db.checkpoint().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (wal fsync)"),
        "checkpoint must fail with the injected fsync marker: {err}"
    );

    // 自清除：fsync 恢复正常，checkpoint 完整走完。
    db.checkpoint().await.unwrap();
    db.close().await.unwrap();
}

/// checkpoint 步骤 2（刷脏页）中的页写入在改文件之前失败：DROP 未生效，
/// 重开看到 DROP 前状态——被删表与原行可查询，另一张表不受影响，
/// `check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn checkpoint_write_fault_keeps_dropped_table() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        ok(&db, "CREATE TABLE other (id INT PRIMARY KEY, v INT)").await;
        ok(&db, "INSERT INTO other VALUES (1, 100)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "DROP TABLE t").await;
    db.arm_io_failure(IoFailPoint::DataWrite);
    let err = db.checkpoint().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (write_page)"),
        "checkpoint must fail with the injected write marker: {err}"
    );
    // flush_all 预清 dirty 位：失败后必须立即崩溃，不得继续会话再 close。
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT id, n FROM t ORDER BY id").await,
        vec![
            vec![serde_json::json!(1), serde_json::json!(10)],
            vec![serde_json::json!(2), serde_json::json!(20)],
        ],
        "a write fault before any byte change must keep the pre-DROP rows"
    );
    assert_eq!(
        rows(&db2, "SELECT v FROM other").await,
        vec![vec![serde_json::json!(100)]],
        "an untouched table must be intact after the write fault"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// checkpoint 步骤 3（WAL fsync）失败：脏页（含 catalog 删除）已写完，
/// DROP 已生效——重开看不到被删表，另一张表不受影响，`check_integrity`
/// 的 error 级发现为 0。
#[tokio::test]
async fn checkpoint_wal_sync_fault_drop_is_durable() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        ok(&db, "CREATE TABLE other (id INT PRIMARY KEY, v INT)").await;
        ok(&db, "INSERT INTO other VALUES (1, 100)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "DROP TABLE t").await;
    db.arm_io_failure(IoFailPoint::WalFsync);
    let err = db.checkpoint().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (wal fsync)"),
        "checkpoint must fail with the injected fsync marker: {err}"
    );
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    match to_response(db2.execute_sql("SELECT id, n FROM t").await) {
        Response::Error { .. } => {}
        other => panic!("the dropped table must stay dropped after the fsync fault: {other:?}"),
    }
    assert_eq!(
        rows(&db2, "SELECT v FROM other").await,
        vec![vec![serde_json::json!(100)]],
        "an untouched table must be intact after the fsync fault"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// checkpoint 步骤 5（位点写入）失败——`.checkpoint` 路径被目录占据：
/// 脏页已写完、WAL 已 fsync，DROP 已生效——重开看不到被删表，另一张表
/// 不受影响，`check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn checkpoint_site_occupied_drop_is_durable() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        ok(&db, "CREATE TABLE other (id INT PRIMARY KEY, v INT)").await;
        ok(&db, "INSERT INTO other VALUES (1, 100)").await;
        db.close().await.unwrap();
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    ok(&db, "DROP TABLE t").await;
    // 注入：位点路径被目录占据（insert_select_test 先例）。
    let site = path.with_extension("checkpoint");
    let _ = std::fs::remove_file(&site);
    std::fs::create_dir(&site).unwrap();
    assert!(db.checkpoint().await.is_err(), "site write must fail");
    crash(db).await;
    // 位点占据是测试注入的环境失败：重开前清除，恢复位点通道。
    std::fs::remove_dir(&site).unwrap();

    let db2 = Database::open(&path).await.unwrap();
    match to_response(db2.execute_sql("SELECT id, n FROM t").await) {
        Response::Error { .. } => {}
        other => panic!("the dropped table must stay dropped after the site fault: {other:?}"),
    }
    assert_eq!(
        rows(&db2, "SELECT v FROM other").await,
        vec![vec![serde_json::json!(100)]],
        "an untouched table must be intact after the site fault"
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}
