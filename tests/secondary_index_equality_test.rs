//! Integration tests for change 2026-10-13-e2e-verification-and-defect-closure
//! Iteration 001 (T2 / R2, ISS05 机制 A/B): at duplicate-key scale a secondary
//! index equality query must be bi-directionally strict-equal to the DataScan
//! baseline — no duplicate emission, no missed subtrees — including across
//! boundary UPDATE probes and failed-statement abort (zero index side effects).
//!
//! RED pre-fix: 15000 行 n∈1..=100 确定性分布（每值 150 行）下探重复发射 +
//! 边界子树漏发；失败语句 abort 路径残留幻影条目（COUNT 漂移）。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::{tempdir, TempDir};

const ROWS: i64 = 15000;
const DISTINCT_KEYS: i64 = 100; // n = i % 100 + 1

async fn open_db() -> (Database, TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("secondary_index_equality.db"))
        .await
        .unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str) {
    match to_response(db.execute_sql(sql).await) {
        Response::Error { message, .. } => panic!("{sql:?} failed: {message}"),
        _ => {}
    }
}

async fn query_i64(db: &Database, sql: &str, what: &str) -> i64 {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows[0][0]
            .as_i64()
            .unwrap_or_else(|| panic!("{what}: not an integer: {:?}", rows[0][0])),
        other => panic!("{what}: expected QueryResult, got {other:?}"),
    }
}

/// COUNT(*) / COUNT(DISTINCT id) for every key 1..=100 — the equality matrix.
async fn count_matrix(db: &Database, what: &str) -> Vec<(i64, i64)> {
    let mut out = Vec::with_capacity(DISTINCT_KEYS as usize);
    for k in 1..=DISTINCT_KEYS {
        let c = query_i64(
            db,
            &format!("SELECT COUNT(*) AS c FROM t WHERE n = {k}"),
            what,
        )
        .await;
        let d = query_i64(
            db,
            &format!("SELECT COUNT(DISTINCT id) AS c FROM t WHERE n = {k}"),
            what,
        )
        .await;
        out.push((c, d));
    }
    out
}

/// 播种 ROWS 行（n = i % 100 + 1，每值恰 ROWS/DISTINCT_KEYS 行）。
async fn seed(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    for chunk in 1..=(ROWS / 3000) {
        let values = ((chunk - 1) * 3000 + 1..=chunk * 3000)
            .map(|i| format!("({i}, {})", i % DISTINCT_KEYS + 1))
            .collect::<Vec<_>>()
            .join(", ");
        exec_ok(db, &format!("INSERT INTO t VALUES {values}")).await;
    }
}

/// S1（等值严格相等）：15000 行重复键表，建索引前逐 k 取 DataScan 基线，
/// CREATE INDEX 后逐 k 等值查询（IndexScanAll 面）必须与基线逐项相等，
/// 且 Σ COUNT(*) == 全表 COUNT(*)。
#[tokio::test]
async fn equality_counts_strictly_equal_datascan_baseline() {
    let (db, _dir) = open_db().await;
    seed(&db).await;

    // 基线（DataScan，无索引）
    let baseline = count_matrix(&db, "baseline count").await;

    exec_ok(&db, "CREATE INDEX idx_n ON t (n)").await;

    // 索引面：逐 k 严格相等
    let indexed = count_matrix(&db, "indexed count").await;
    assert_eq!(
        indexed, baseline,
        "IndexScanAll counts must be bi-directionally strict-equal to the DataScan baseline"
    );

    // 全局一致性：Σ COUNT(*) == 全表行数（无幻影、无遗漏）
    let total: i64 = indexed.iter().map(|(c, _)| c).sum();
    assert_eq!(total, ROWS, "per-key counts must partition the full table");
}

/// S2（边界 UPDATE 探针）：改一条边界行的 n，旧键计数恰减一、新键计数
/// 恰增一（索引条目维护与查询面一致）。
#[tokio::test]
async fn boundary_update_probes_shift_counts_by_one() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    exec_ok(&db, "CREATE INDEX idx_n ON t (n)").await;

    // id=1 → n=2；改到 100（两键均存在且满额 150）
    exec_ok(&db, "UPDATE t SET n = 100 WHERE id = 1").await;
    assert_eq!(
        query_i64(&db, "SELECT COUNT(*) AS c FROM t WHERE n = 2", "old key").await,
        149,
        "old-key count must decrease by exactly one"
    );
    assert_eq!(
        query_i64(&db, "SELECT COUNT(*) AS c FROM t WHERE n = 100", "new key").await,
        151,
        "new-key count must increase by exactly one"
    );

    // 改回（rekey 回原值）：计数还原
    exec_ok(&db, "UPDATE t SET n = 2 WHERE id = 1").await;
    assert_eq!(
        query_i64(
            &db,
            "SELECT COUNT(*) AS c FROM t WHERE n = 2",
            "restored key"
        )
        .await,
        150
    );
}

/// S3（失败语句索引零副作用）：制造首行写入成功、次行碰撞的中途失败
/// （DELETE id=2 后 `SET id = id + 1`：1→2 写入成功，3→4 碰撞），语句失败
/// 且隐式事务回滚后，逐 k 索引计数与失败前完全一致（abort 路径无幻影）。
#[tokio::test]
async fn failed_statement_leaves_no_index_side_effects() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    exec_ok(&db, "CREATE INDEX idx_n ON t (n)").await;

    // 先制造键位空缺（DELETE 提交后计数即定），再取失败前矩阵；
    // 中途失败：行 1 → 新键 2（空缺，写入成功）；行 3 → 新键 4 碰撞 → 语句失败。
    exec_ok(&db, "DELETE FROM t WHERE id = 2").await;
    let before = count_matrix(&db, "pre-failure count").await;

    let resp = to_response(db.execute_sql("UPDATE t SET id = id + 1").await);
    match &resp {
        Response::Error { message, .. } => assert!(
            message.to_lowercase().contains("duplicate"),
            "rekey collision must surface DuplicateKey, got: {message}"
        ),
        other => panic!("failing UPDATE must error, got {other:?}"),
    }

    let after = count_matrix(&db, "post-failure count").await;
    assert_eq!(
        after, before,
        "failed statement must leave the secondary index side-effect free"
    );
    assert_eq!(
        query_i64(&db, "SELECT COUNT(*) AS c FROM t", "full count").await,
        ROWS - 1,
        "rollback restored the one written row"
    );
    // 被回滚的行（id=1）内容不变
    assert_eq!(
        query_i64(&db, "SELECT n AS v FROM t WHERE id = 1", "row 1").await,
        2,
        "rolled-back row content intact"
    );
}
