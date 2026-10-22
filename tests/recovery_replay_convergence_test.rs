//! Iteration 002 T8（R10）：崩溃恢复 redo 按 WAL logical_id 强制收敛。
//!
//! change: 2026-10-02-ms20-cross-db-dml / iterations/002-defect-closeout
//!
//! 探针序列（Iteration 000 Act Response 记载的失配形态）：语句部分写入回滚
//! （abort 原位中性化、页逻辑 id 计数器照常消耗）→ 重插同键 → 崩溃（WAL
//! shutdown + drop 不 close）→ 重开。修复前重放按自然分配落位，「重放分配
//! 序列 == 运行期分配序列」假设失配 → dense-assignment violation →
//! `RedoFailed` → open 失败；修复后按 WAL 记录的 logical_id 强制落位收敛。
//!
//! 夹具说明：CREATE TABLE / CREATE INDEX 后显式 `checkpoint()` 使 catalog 页
//! 落盘（DDL 无 WAL 记录，引擎持久化模型——wal_recovery_large_test.rs 先例）。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// 探针表的建表 + 二级索引 + checkpoint（DDL 无 WAL 记录，需显式落盘）。
async fn seed_schema(db: &Database) {
    for sql in [
        "CREATE TABLE c (id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE)",
        "CREATE INDEX idx_c_n ON c (n)",
    ] {
        let resp = to_response(db.execute_sql(sql).await);
        assert!(
            !matches!(resp, Response::Error { .. }),
            "schema setup failed for {sql}: {resp:?}"
        );
    }
    db.checkpoint().await.unwrap();
}

/// 提交面写入：UPSERT / rekey UPDATE / INSERT / DELETE 全部成功。
async fn committed_writes(db: &Database) {
    let steps = [
        ("INSERT INTO c VALUES (1, 10, 100)", true),
        // UPSERT DO UPDATE：命中 (id) 仲裁后原位更新 n（uniq 保持 100）。
        (
            "INSERT INTO c VALUES (1, 10, 100) ON CONFLICT (id) DO UPDATE SET n = 12",
            true,
        ),
        // rekey UPDATE：键位 1 → 5（索引条目 rekey 语义）。
        ("UPDATE c SET id = 5 WHERE id = 1", true),
        ("INSERT INTO c VALUES (8, 9, 101)", true),
        ("DELETE FROM c WHERE id = 8", true),
    ];
    for (sql, should_commit) in steps {
        let resp = to_response(db.execute_sql(sql).await);
        assert!(
            matches!(resp, Response::Error { .. }) != should_commit,
            "write step {sql:?} unexpected response: {resp:?}"
        );
    }
}

/// 多行 INSERT：前行 (2,11,101) 已写、后行 (3,12,100) 唯一值 100 与键位 5 的
/// 行冲突 → 语句失败（abort，前行原位中性化、页逻辑 id 已消耗）。
async fn aborting_multi_row_insert(db: &Database) {
    let resp = to_response(
        db.execute_sql("INSERT INTO c VALUES (2, 11, 101), (3, 12, 100)")
            .await,
    );
    assert!(
        matches!(&resp, Response::Error { message, .. } if message.contains("Duplicate key")),
        "multi-row INSERT must fail on the unique conflict, got {resp:?}"
    );
}

/// 恢复后的提交终态断言：键位 5（n=12, uniq=100）与键位 2（n=11, uniq=101）。
async fn assert_committed_terminal_state(db: &Database) {
    let count = to_response(db.execute_sql("SELECT COUNT(*) FROM c").await);
    match count {
        Response::QueryResult { rows } => {
            assert_eq!(rows, vec![vec![serde_json::json!(2)]], "recovered count");
        }
        other => panic!("COUNT(*) after recovery: {other:?}"),
    }

    // PK 点查（含 rekey 前旧键 1 与被删键 8 的不可达）。
    for (sql, expected) in [
        (
            "SELECT * FROM c WHERE id = 2",
            vec![vec![
                serde_json::json!(2),
                serde_json::json!(11),
                serde_json::json!(101),
            ]],
        ),
        (
            "SELECT * FROM c WHERE id = 5",
            vec![vec![
                serde_json::json!(5),
                serde_json::json!(12),
                serde_json::json!(100),
            ]],
        ),
        ("SELECT * FROM c WHERE id = 1", vec![]),
        ("SELECT * FROM c WHERE id = 8", vec![]),
    ] {
        let resp = to_response(db.execute_sql(sql).await);
        match resp {
            Response::QueryResult { rows } => {
                assert_eq!(rows, expected, "PK probe {sql:?}");
            }
            other => panic!("PK probe {sql:?}: {other:?}"),
        }
    }

    // 唯一占用：uniq 100/101 各归其主，冲突写入零副作用拒绝。
    for (sql, expected) in [
        (
            "SELECT id FROM c WHERE uniq = 100",
            vec![vec![serde_json::json!(5)]],
        ),
        (
            "SELECT id FROM c WHERE uniq = 101",
            vec![vec![serde_json::json!(2)]],
        ),
    ] {
        let resp = to_response(db.execute_sql(sql).await);
        match resp {
            Response::QueryResult { rows } => {
                assert_eq!(rows, expected, "unique probe {sql:?}");
            }
            other => panic!("unique probe {sql:?}: {other:?}"),
        }
    }
    let dup = to_response(db.execute_sql("INSERT INTO c VALUES (9, 9, 100)").await);
    assert!(
        matches!(&dup, Response::Error { message, .. } if message.contains("Duplicate key")),
        "unique enforcement must survive recovery, got {dup:?}"
    );

    // 二级索引查询（idx_c_n）：n=12 仅键位 5 的行。
    let sec = to_response(db.execute_sql("SELECT id FROM c WHERE n = 12").await);
    match sec {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5)]],
                "secondary index probe"
            );
        }
        other => panic!("secondary index probe: {other:?}"),
    }
}

/// R10/S1 探针：abort → 重插同键 → 崩溃 → 重开恢复成功且终态一致。
#[tokio::test]
async fn aborted_insert_reinsert_crash_recovery_succeeds() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    committed_writes(&db).await;
    aborting_multi_row_insert(&db).await;

    // 重插同键提交（被中性化的版本不占用键位）。
    let resp = to_response(db.execute_sql("INSERT INTO c VALUES (2, 11, 101)").await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "reinsert after abort must commit, got {resp:?}"
    );

    // 崩溃模拟：WAL 落盘（shutdown ≠ close，不触发 checkpoint）后 drop。
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);

    // 修复前：dense-assignment violation → RedoFailed → open 失败（RED）。
    let db2 = Database::open(&path).await.unwrap();
    assert_committed_terminal_state(&db2).await;

    // 重跑幂等：再次崩溃重开（WAL 未截断），重放收敛结果一致。
    db2.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db2);
    let db3 = Database::open(&path).await.unwrap();
    let count = to_response(db3.execute_sql("SELECT COUNT(*) FROM c").await);
    match count {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(2)]],
                "rerun idempotent count"
            );
        }
        other => panic!("rerun COUNT(*): {other:?}"),
    }
    db3.close().await.unwrap();
}

/// 对照组 1：同一提交面、无 abort、无重插——恢复干净（既有行为守护）。
#[tokio::test]
async fn committed_writes_without_abort_recover_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    committed_writes(&db).await;

    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);

    let db2 = Database::open(&path).await.unwrap();
    let count = to_response(db2.execute_sql("SELECT COUNT(*) FROM c").await);
    match count {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(1)]],
                "no-abort control count"
            );
        }
        other => panic!("control COUNT(*): {other:?}"),
    }
    let row = to_response(db2.execute_sql("SELECT * FROM c WHERE id = 5").await);
    match row {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![
                    serde_json::json!(5),
                    serde_json::json!(12),
                    serde_json::json!(100)
                ]],
                "no-abort control rekeyed row"
            );
        }
        other => panic!("control PK probe: {other:?}"),
    }
    db2.close().await.unwrap();
}

/// 对照组 2：含 abort、不重插——恢复干净（中性化版本不复活，既有行为守护）。
#[tokio::test]
async fn abort_without_reinsert_recovers_cleanly() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    committed_writes(&db).await;
    aborting_multi_row_insert(&db).await;

    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);

    let db2 = Database::open(&path).await.unwrap();
    let count = to_response(db2.execute_sql("SELECT COUNT(*) FROM c").await);
    match count {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(1)]],
                "abort-only control count"
            );
        }
        other => panic!("control COUNT(*): {other:?}"),
    }
    // 中止行 (2,11,101) 不得复活。
    let probe = to_response(db2.execute_sql("SELECT * FROM c WHERE id = 2").await);
    match probe {
        Response::QueryResult { rows } => {
            assert!(
                rows.is_empty(),
                "aborted row must stay neutralized, got {rows:?}"
            );
        }
        other => panic!("aborted-row probe: {other:?}"),
    }
    db2.close().await.unwrap();
}
