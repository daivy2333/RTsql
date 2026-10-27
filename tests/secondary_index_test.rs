//! Integration tests for MS21 Iteration 002 (T2-a…g): 单列 INT 二级索引端到端
//! （delta spec `sql-secondary-indexes` R1–R5）。
//!
//! Scenario mapping (requirement → scenario → test):
//!   R1-S1 建索引与存量行索引化        create_index_backfills_existing_rows_equivalently
//!   R1-S2 干净重开后索引可用          create_index_usable_after_clean_reopen
//!   R1-S3 崩溃恢复两态一致            create_index_consistent_across_crash_recovery
//!   R1-S4 非字符串非 INT 列拒绝       create_index_on_non_int_column_rejected
//!   R1-S5 重复索引名拒绝              create_index_duplicate_name_rejected
//!   R1-S6 UNIQUE 拒绝、两列整数索引可建 create_unique_and_composite_index_rejected
//!   R1-S7 表或列不存在拒绝            create_index_missing_table_or_column_rejected
//!   R1 修饰面拒绝（部分/IF NOT EXISTS）create_index_modifier_forms_rejected
//!   R1 显式事务内拒绝                 create_index_inside_explicit_transaction_rejected
//!   R1 PK 列冗余允许                  create_index_on_pk_column_allowed
//!   R2-S1 等值路由计划形状可观察      explain_shows_named_index_scan_node
//!   R2-S2 建索引前后结果等价          query_results_equivalent_before_and_after_index
//!   R2-S3 不可键控字面量回落          non_keyable_literal_falls_back_without_error
//!   R2-S4 无索引列零回归              unindexed_column_keeps_scan_semantics
//!   R3-S1 INSERT 后点查可达           insert_is_reachable_via_index_query
//!   R3-S2 DELETE 后点查不再命中       delete_stops_matching_index_query
//!   R3-S3 索引列 rekey 新旧键一致     update_rekey_keeps_old_and_new_keys_consistent
//!   R3-S4 事务回滚后条目还原          rollback_restores_index_entries_two_state
//!   R3-S5 UPSERT 冲突行删除后清理     upsert_conflict_actions_maintain_index
//!   R4-S1 删索引后查询回落扫描        drop_index_falls_back_to_scan
//!   R4-S2 索引不存在拒绝              drop_index_missing_or_if_exists_rejected
//!   R4-S3 删索引跨重启持久            drop_index_persists_across_restart

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

async fn open_db() -> (Database, TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str, what: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {sql:?} -> {resp:?}"
    );
}

fn expect_ok_resp(resp: Response, what: &str) {
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {resp:?}"
    );
}

fn query_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("{what}: expected QueryResult, got {other:?}"),
    }
}

fn error_msg(resp: Response, what: &str) -> String {
    match resp {
        Response::Error { message, sqlstate } => {
            format!("{message}|{sqlstate:?}")
        }
        other => panic!("{what}: expected Error, got {other:?}"),
    }
}

/// 断言错误文案携带 `needle`（点名能力/对象）。
fn expect_plan_error(resp: Response, what: &str, needle: &str) {
    let msg = error_msg(resp, what);
    assert!(
        msg.contains(needle),
        "{what}: error must name {needle:?}, got: {msg}"
    );
}

/// 种子：`t(id INT PRIMARY KEY, age INT, name VARCHAR)` 含重复 age 值多行：
/// (1,25,'a') (2,25,'b') (3,30,'c') (4,NULL,'d')。
async fn seed_dup_table(db: &Database) {
    exec_ok(
        db,
        "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
        "create t",
    )
    .await;
    exec_ok(db, "INSERT INTO t VALUES (1, 25, 'a')", "insert 1").await;
    exec_ok(db, "INSERT INTO t VALUES (2, 25, 'b')", "insert 2").await;
    exec_ok(db, "INSERT INTO t VALUES (3, 30, 'c')", "insert 3").await;
    exec_ok(db, "INSERT INTO t VALUES (4, NULL, 'd')", "insert 4").await;
}

fn age_rows(rows: &[Vec<serde_json::Value>]) -> Vec<Vec<serde_json::Value>> {
    let mut out = rows.to_vec();
    out.sort_by_cached_key(|r| serde_json::to_string(r).unwrap());
    out
}

// ===========================================================================
// R1: CREATE INDEX 建立单列 INT 二级索引
// ===========================================================================

/// R1-S1：建索引成功；重复 age 值多行全部命中，行集与建索引前一致。
#[tokio::test]
async fn create_index_backfills_existing_rows_equivalently() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;

    let before = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "before index",
    );
    assert_eq!(before.len(), 2, "duplicate age rows must both match");

    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;

    let after = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "after index",
    );
    assert_eq!(
        age_rows(&after),
        age_rows(&before),
        "index creation must not change query results"
    );
}

/// R1-S2：显式 close 后重开，索引仍可用、行集一致。
#[tokio::test]
async fn create_index_usable_after_clean_reopen() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_dup_table(&db).await;
        exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    exec_ok(
        &db,
        "INSERT INTO t VALUES (5, 25, 'e')",
        "insert after reopen",
    )
    .await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "after reopen",
    );
    assert_eq!(rows.len(), 3, "index must stay usable across clean reopen");
}

/// R1-S3：建索引后新写入行，崩溃（shutdown + drop 不 close → WAL 重放 + 索引
/// 重建）后等值行集与崩溃前运行期行为一致。
#[tokio::test]
async fn create_index_consistent_across_crash_recovery() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_dup_table(&db).await;
        exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
        // DDL 无 WAL 记录：catalog（含索引登记段）经 buffer_pool 落盘
        db.buffer_pool.flush_all().await.unwrap();
        exec_ok(
            &db,
            "INSERT INTO t VALUES (5, 25, 'e')",
            "crash-window insert",
        )
        .await;

        let runtime = query_rows(
            to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
            "runtime",
        );
        assert_eq!(runtime.len(), 3, "runtime equality must see all rows");
        db.wal_buffer.as_ref().unwrap().shutdown().await;
        drop(db); // 崩溃模拟：不 close（不 checkpoint、不刷数据页）
    }
    let db2 = Database::open(&path).await.unwrap();
    let recovered = query_rows(
        to_response(db2.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "recovered",
    );
    assert_eq!(
        recovered.len(),
        3,
        "crash recovery must keep index contents consistent with data"
    );
}

/// R1-S4：INT 与字符串以外的列点名拒绝，零副作用。
#[tokio::test]
async fn create_index_on_non_int_column_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(
        &db,
        "CREATE TABLE flags (id INT PRIMARY KEY, flag BOOLEAN)",
        "create flags",
    )
    .await;
    expect_plan_error(
        to_response(db.execute_sql("CREATE INDEX idx_flag ON flags(flag)").await),
        "unsupported column",
        "Unsupported statement type",
    );
    // 零副作用：合法建索引仍可用
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create after reject").await;
    exec_ok(&db, "INSERT INTO t VALUES (5, 25, 'e')", "insert").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "query",
    );
    assert_eq!(rows.len(), 3);
}

/// R1-S5：索引名库级唯一——不同表上的重复名点名拒绝，既有索引零变化。
#[tokio::test]
async fn create_index_duplicate_name_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create first").await;
    exec_ok(
        &db,
        "CREATE TABLE other (id INT PRIMARY KEY, age INT)",
        "create other",
    )
    .await;
    expect_plan_error(
        to_response(db.execute_sql("CREATE INDEX idx_age ON other(age)").await),
        "duplicate index name",
        "already exists",
    );
    // 既有索引零变化：等值查询仍工作
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "query",
    );
    assert_eq!(rows.len(), 2);
}

/// UNIQUE 索引仍被拒绝。两列整数索引可以建立。
#[tokio::test]
async fn create_unique_and_composite_index_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    expect_plan_error(
        to_response(db.execute_sql("CREATE UNIQUE INDEX u ON t(age)").await),
        "unique index",
        "Unsupported statement type",
    );
    exec_ok(&db, "CREATE INDEX c ON t(age, id)", "composite index").await;
}

/// R1 修饰面：部分索引（WHERE）与 IF NOT EXISTS 点名拒绝，不静默忽略成分。
#[tokio::test]
async fn create_index_modifier_forms_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    expect_plan_error(
        to_response(
            db.execute_sql("CREATE INDEX p ON t(age) WHERE age > 20")
                .await,
        ),
        "partial index",
        "Unsupported statement type",
    );
    expect_plan_error(
        to_response(
            db.execute_sql("CREATE INDEX IF NOT EXISTS i ON t(age)")
                .await,
        ),
        "if not exists",
        "Unsupported statement type",
    );
}

/// R1-S7：表不存在 / 列不存在均点名拒绝，零副作用。
#[tokio::test]
async fn create_index_missing_table_or_column_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    expect_plan_error(
        to_response(db.execute_sql("CREATE INDEX i1 ON nope(c)").await),
        "missing table",
        "not found",
    );
    expect_plan_error(
        to_response(db.execute_sql("CREATE INDEX i2 ON t(nope)").await),
        "missing column",
        "not found",
    );
}

/// R1：显式事务内 CREATE INDEX 计划期点名拒绝（0A000，CTAS 先例同型），
/// 事务保持可用。
#[tokio::test]
async fn create_index_inside_explicit_transaction_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    let tx = db.begin().await.unwrap();
    match to_response(
        db.execute_in_tx("CREATE INDEX idx_age ON t(age)", &tx)
            .await,
    ) {
        Response::Error { sqlstate, .. } => {
            assert_eq!(
                sqlstate.as_deref(),
                Some("0A000"),
                "in-tx CREATE INDEX must be rejected with 0A000"
            );
        }
        other => panic!("expected in-tx rejection, got {other:?}"),
    }
    // 事务保持可用：回滚干净
    db.rollback(tx).await.unwrap();
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create after tx").await;
}

/// R1：对 PK 列建二级索引允许（冗余但行为等价，不做专门检测）。
#[tokio::test]
async fn create_index_on_pk_column_allowed() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_id ON t(id)", "create on pk").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE id = 3").await),
        "query",
    );
    assert_eq!(rows.len(), 1);
}

// ===========================================================================
// R2: 二级索引等值查询路由可达
// ===========================================================================

/// R2-S1：EXPLAIN 计划树含点名该索引的索引扫描节点。
#[tokio::test]
async fn explain_shows_named_index_scan_node() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let rows = query_rows(
        to_response(
            db.execute_sql("EXPLAIN SELECT * FROM t WHERE age = 25")
                .await,
        ),
        "explain",
    );
    let text: Vec<String> = rows
        .iter()
        .map(|r| r[0].as_str().expect("plan line").to_string())
        .collect();
    assert!(
        text.iter()
            .any(|l| l.contains("IndexScanAll") && l.contains("idx_age")),
        "plan tree must name the secondary index scan node, got: {text:?}"
    );
}

/// R2-S2：建索引前后同一等值查询行集一致（仅计划形状不同）。
#[tokio::test]
async fn query_results_equivalent_before_and_after_index() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    let before = query_rows(
        to_response(
            db.execute_sql("SELECT id, name FROM t WHERE age = 30")
                .await,
        ),
        "before",
    );
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let after = query_rows(
        to_response(
            db.execute_sql("SELECT id, name FROM t WHERE age = 30")
                .await,
        ),
        "after",
    );
    assert_eq!(before, after);
    assert_eq!(before.len(), 1);
}

/// R2-S3：不可键控字面量（NULL / 非 Int）回落既有扫描语义，无路由错误。
#[tokio::test]
async fn non_keyable_literal_falls_back_without_error() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = NULL").await),
        "null literal",
    );
    assert!(
        rows.is_empty(),
        "age = NULL must fall back to row evaluation"
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = '25'").await),
        "string literal",
    );
    assert!(rows.is_empty(), "string literal must fall back (type gate)");
}

/// R2-S4：未建索引的列与无索引的表保持既有扫描语义零回归。
#[tokio::test]
async fn unindexed_column_keeps_scan_semantics() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE name = 'a'").await),
        "unindexed column",
    );
    assert_eq!(rows.len(), 1, "unindexed equality must keep scan semantics");
    let lines: Vec<String> = query_rows(
        to_response(
            db.execute_sql("EXPLAIN SELECT * FROM t WHERE name = 'a'")
                .await,
        ),
        "explain",
    )
    .iter()
    .map(|r| r[0].as_str().unwrap().to_string())
    .collect();
    assert!(
        !lines.iter().any(|l| l.contains("IndexScanAll")),
        "unindexed column must not route to an index node: {lines:?}"
    );
}

// ===========================================================================
// R3: 二级索引写路径条目维护
// ===========================================================================

/// R3-S1：INSERT 新行经索引点查可达（含重复键多行并存）。
#[tokio::test]
async fn insert_is_reachable_via_index_query() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    exec_ok(&db, "INSERT INTO t VALUES (10, 25, 'x')", "insert").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "query",
    );
    assert_eq!(rows.len(), 3, "new row must enter the index");
}

/// R3-S2：DELETE 后点查不再命中。
#[tokio::test]
async fn delete_stops_matching_index_query() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    exec_ok(&db, "DELETE FROM t WHERE id = 1", "delete").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "query",
    );
    assert_eq!(rows.len(), 1, "deleted row must leave the index");
    assert_eq!(rows[0][0], serde_json::json!(2));
}

/// R3-S3：索引列 rekey 后新旧键可达性一致（经索引路由与非索引路径一致）。
#[tokio::test]
async fn update_rekey_keeps_old_and_new_keys_consistent() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    exec_ok(&db, "UPDATE t SET age = 40 WHERE id = 3", "rekey update").await;
    let new_key = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 40").await),
        "new key",
    );
    assert_eq!(new_key.len(), 1, "new key must reach the row");
    let old_key = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 30").await),
        "old key",
    );
    assert!(old_key.is_empty(), "old key must no longer match");
}

/// R3-S4：显式事务中 INSERT/UPDATE/DELETE 后回滚——等值查询行集与事务前
/// 一致，重开两态一致。
#[tokio::test]
async fn rollback_restores_index_entries_two_state() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_dup_table(&db).await;
        exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
        // DDL 无 WAL 记录：catalog（含索引登记段）经 buffer_pool 落盘，
        // 否则崩溃重开的 redo 找不到表
        db.buffer_pool.flush_all().await.unwrap();

        let tx = db.begin().await.unwrap();
        expect_ok_resp(
            to_response(
                db.execute_in_tx("INSERT INTO t VALUES (10, 25, 'x')", &tx)
                    .await,
            ),
            "tx insert",
        );
        expect_ok_resp(
            to_response(
                db.execute_in_tx("UPDATE t SET age = 99 WHERE id = 3", &tx)
                    .await,
            ),
            "tx update",
        );
        expect_ok_resp(
            to_response(db.execute_in_tx("DELETE FROM t WHERE id = 2", &tx).await),
            "tx delete",
        );
        db.rollback(tx).await.unwrap();

        let rows = query_rows(
            to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
            "after rollback",
        );
        assert_eq!(
            rows.len(),
            2,
            "rollback must restore index entries to pre-transaction state"
        );
        let rows = query_rows(
            to_response(db.execute_sql("SELECT * FROM t WHERE age = 30").await),
            "after rollback",
        );
        assert_eq!(rows.len(), 1, "rolled-back rekey must leave old key intact");
        let rows = query_rows(
            to_response(db.execute_sql("SELECT * FROM t WHERE age = 99").await),
            "after rollback",
        );
        assert!(rows.is_empty(), "rolled-back new key must have no entry");

        db.wal_buffer.as_ref().unwrap().shutdown().await;
        drop(db);
    }
    let db2 = Database::open(&path).await.unwrap();
    let rows = query_rows(
        to_response(db2.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "reopened",
    );
    assert_eq!(rows.len(), 2, "reopen two-state must agree with runtime");
}

/// R3-S5：UPSERT 冲突动作维护索引——DO UPDATE 改索引列后新旧键一致，
/// REPLACE 删冲突行后旧键条目清理。
#[tokio::test]
async fn upsert_conflict_actions_maintain_index() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;

    exec_ok(
        &db,
        "INSERT INTO t VALUES (1, 99, 'b') ON CONFLICT (id) DO UPDATE SET age = 99",
        "do update",
    )
    .await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 99").await),
        "do update new key",
    );
    assert_eq!(rows.len(), 1, "DO UPDATE must enter the new key");
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "do update old key",
    );
    assert_eq!(rows.len(), 1, "only the untouched duplicate remains");

    exec_ok(&db, "REPLACE INTO t VALUES (2, 77, 'c')", "replace").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 77").await),
        "replace new key",
    );
    assert_eq!(rows.len(), 1);
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "replace old key",
    );
    assert!(rows.is_empty(), "replaced row's old key entry must be gone");
}

// ===========================================================================
// R4: DROP INDEX 移除二级索引
// ===========================================================================

/// R4-S1：删索引后等值查询回落扫描且行集一致；EXPLAIN 不再含索引节点。
#[tokio::test]
async fn drop_index_falls_back_to_scan() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let before = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "with index",
    );
    exec_ok(&db, "DROP INDEX idx_age", "drop index").await;
    let after = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "after drop",
    );
    assert_eq!(
        age_rows(&after),
        age_rows(&before),
        "rows must be untouched"
    );

    let lines: Vec<String> = query_rows(
        to_response(
            db.execute_sql("EXPLAIN SELECT * FROM t WHERE age = 25")
                .await,
        ),
        "explain",
    )
    .iter()
    .map(|r| r[0].as_str().unwrap().to_string())
    .collect();
    assert!(
        !lines.iter().any(|l| l.contains("idx_age")),
        "dropped index must not appear in plan: {lines:?}"
    );
}

/// R4-S2：索引不存在点名拒绝；IF EXISTS 子句点名拒绝（假设 10）。
#[tokio::test]
async fn drop_index_missing_or_if_exists_rejected() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    expect_plan_error(
        to_response(db.execute_sql("DROP INDEX nope").await),
        "missing index",
        "nope",
    );
    expect_plan_error(
        to_response(db.execute_sql("DROP INDEX IF EXISTS nope").await),
        "if exists",
        "Unsupported statement type",
    );
}

/// R4-S3：删除跨重启持久（catalog 登记移除）；重开后等值查询为扫描语义。
#[tokio::test]
async fn drop_index_persists_across_restart() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_dup_table(&db).await;
        exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
        exec_ok(&db, "DROP INDEX idx_age", "drop index").await;
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    let lines: Vec<String> = query_rows(
        to_response(
            db.execute_sql("EXPLAIN SELECT * FROM t WHERE age = 25")
                .await,
        ),
        "explain",
    )
    .iter()
    .map(|r| r[0].as_str().unwrap().to_string())
    .collect();
    assert!(
        !lines.iter().any(|l| l.contains("idx_age")),
        "dropped index must stay gone across restart: {lines:?}"
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 25").await),
        "query",
    );
    assert_eq!(rows.len(), 2);
}

// ===========================================================================
// T1（change 2026-10-04-external-review-gaps R1）：IndexScanAll 列元数据契约
// ——`columns` 恒为基表全列，输出形状由 `projection` 唯一决定；任意投影组合
// 表头与行逐位对齐、不 panic；EXPLAIN 同契约；聚合输入映射正确。
// ===========================================================================

/// R1-S1：后位列单列投影不 panic（修复前：index out of bounds panic，exit 101）。
#[tokio::test]
async fn secondary_index_scan_late_column_projection_no_panic() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT name FROM t WHERE age = 30").await),
        "late column projection",
    );
    assert_eq!(rows, vec![vec![serde_json::json!("c")]]);
}

/// R1-S2：乱序多列投影行序与请求一致（lib 层行形状回归守卫；表头对齐由
/// cli_test `test_secondary_index_scan_projection_headers` 承载）。
#[tokio::test]
async fn secondary_index_scan_out_of_order_projection_rows() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let rows = query_rows(
        to_response(
            db.execute_sql("SELECT name, id FROM t WHERE age = 30")
                .await,
        ),
        "out-of-order projection",
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!("c"), serde_json::json!(3)]]
    );
}

/// R1-S3：EXPLAIN 渲染遵循同一契约，不 panic 且显示真实输出形状
/// （修复前：plan_format 渲染点 panic）。
#[tokio::test]
async fn explain_secondary_index_projection_shape_no_panic() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let lines: Vec<String> = query_rows(
        to_response(
            db.execute_sql("EXPLAIN SELECT name FROM t WHERE age = 30")
                .await,
        ),
        "explain",
    )
    .iter()
    .map(|r| r[0].as_str().expect("plan line").to_string())
    .collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("IndexScanAll") && l.contains("columns=[name]")),
        "EXPLAIN must render the projected output shape, got: {lines:?}"
    );
}

/// R1-S4：PK 路由、范围谓词回落与通配投影零回归。
#[tokio::test]
async fn secondary_index_scan_pk_range_wildcard_zero_regression() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    // PK 等值路由（IndexScan，「已收窄」契约）零变化
    let rows = query_rows(
        to_response(db.execute_sql("SELECT name FROM t WHERE id = 3").await),
        "pk route",
    );
    assert_eq!(rows, vec![vec![serde_json::json!("c")]]);
    // 范围谓词回落扫描
    let rows = query_rows(
        to_response(db.execute_sql("SELECT name FROM t WHERE age > 24").await),
        "range fallback",
    );
    assert_eq!(rows.len(), 3);
    // 通配投影经二级索引扫描
    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t WHERE age = 30").await),
        "wildcard",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(3),
            serde_json::json!(30),
            serde_json::json!("c")
        ]]
    );
}

/// R1-S5：二级索引扫描上的聚合输入按基表列号正确映射。
#[tokio::test]
async fn aggregate_over_secondary_index_scan_maps_correctly() {
    let (db, _dir) = open_db().await;
    seed_dup_table(&db).await;
    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;
    let rows = query_rows(
        to_response(
            db.execute_sql("SELECT COUNT(*), MAX(id) FROM t WHERE age = 25")
                .await,
        ),
        "aggregate over index scan",
    );
    assert_eq!(rows, vec![vec![serde_json::json!(2), serde_json::json!(2)]]);
}
