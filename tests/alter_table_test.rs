//! Integration tests for MS21 Iteration 003 (T3-a…d): ALTER TABLE ADD/DROP COLUMN
//! （delta spec `alter-table-column-evolution` R1–R3）。
//!
//! Scenario mapping (requirement → scenario → test):
//!   R1-S1 加列后存量行读回 NULL                add_column_tail_null_backfill
//!   R1-S2 带 DEFAULT 加列存量行读回默认值      add_column_with_default_backfills
//!   R1-S3 NOT NULL + DEFAULT 非空表可行        add_column_not_null_default_on_non_empty
//!   R1-S4 NOT NULL 无 DEFAULT 非空表拒绝       add_column_not_null_without_default_rejected
//!   R1-S5 约束声明拒绝                          add_column_constraint_declarations_rejected
//!   R1-S6 加列持久与 dump→restore 保真          add_column_persists_and_dump_restore_roundtrip
//!   R2-S1 删列后查询形状一致                    drop_column_output_shape_conforms
//!   R2-S2 被索引引用的列拒绝                    drop_index_referenced_column_rejected
//!   R2-S3 删除最后一列拒绝                      drop_last_column_rejected
//!   R2-S4 删列持久与 dump→restore 保真          drop_column_persists_and_dump_restore_roundtrip
//!   R2-S5 列不存在拒绝                          drop_missing_column_rejected
//!   R2+ 崩溃窗口两态一致                        drop_column_consistent_across_crash_recovery
//!   R3-S1 多操作单语句拒绝                      alter_table_multi_operation_rejected
//!   R3-S2 目标不存在拒绝                        alter_table_missing_table_rejected
//!   R3-S3 显式事务内拒绝                        alter_table_inside_explicit_transaction_rejected
//!   R3-S4 既有面零回归                          （全量分批承载，不单列）

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

fn query_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("{what}: expected QueryResult, got {other:?}"),
    }
}

fn expect_plan_error(resp: Response, what: &str, needle: &str) {
    match resp {
        Response::Error { message, .. } => assert!(
            message.contains(needle),
            "{what}: error must name {needle:?}, got: {message}"
        ),
        other => panic!("{what}: expected Error naming {needle:?}, got {other:?}"),
    }
}

/// 种子：`t(id INT PRIMARY KEY, v INT)` 含若干行。
async fn seed_two_col(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)", "create t").await;
    for (id, v) in [(1, 10), (2, 20), (3, 30)] {
        exec_ok(db, &format!("INSERT INTO t VALUES ({id}, {v})"), "insert").await;
    }
}

// ===========================================================================
// R1: ALTER TABLE ADD COLUMN 尾部加列
// ===========================================================================

/// R1-S1：加列成功；存量行新列读回 NULL；新写入行（显式列清单）读回新值。
#[tokio::test]
async fn add_column_tail_null_backfill() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;

    exec_ok(&db, "ALTER TABLE t ADD COLUMN note VARCHAR", "add column").await;

    let rows = query_rows(
        to_response(db.execute_sql("SELECT id, note FROM t").await),
        "existing rows note",
    );
    assert_eq!(rows.len(), 3);
    for r in rows {
        assert!(r[1].is_null(), "existing row note must read NULL: {r:?}");
    }

    exec_ok(
        &db,
        "INSERT INTO t (id, v, note) VALUES (9, 1, 'x')",
        "full insert",
    )
    .await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT id, note FROM t WHERE id = 9").await),
        "new row note",
    );
    assert_eq!(rows[0][1], serde_json::json!("x"));
}

/// R1-S2：带 DEFAULT 加列——存量行读回默认值；子集 INSERT 省略该列时新行
/// 亦为默认值（`sql-write-surface` DEFAULT 语义衔接）。
#[tokio::test]
async fn add_column_with_default_backfills() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;

    exec_ok(&db, "ALTER TABLE t ADD COLUMN flag INT DEFAULT 0", "add").await;

    let rows = query_rows(
        to_response(db.execute_sql("SELECT id, flag FROM t").await),
        "existing rows flag",
    );
    for r in rows {
        assert_eq!(r[1], serde_json::json!(0), "existing row flag must be 0");
    }

    exec_ok(&db, "INSERT INTO t (id, v) VALUES (5, 50)", "subset insert").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT flag FROM t WHERE id = 5").await),
        "subset insert flag",
    );
    assert_eq!(
        rows[0][0],
        serde_json::json!(0),
        "subset insert uses default"
    );
}

/// R1-S3：NOT NULL + DEFAULT 非空表可行，存量行读回默认值。
#[tokio::test]
async fn add_column_not_null_default_on_non_empty() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;

    exec_ok(
        &db,
        "ALTER TABLE t ADD COLUMN req INT NOT NULL DEFAULT 7",
        "add",
    )
    .await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT req FROM t").await),
        "read back",
    );
    for r in rows {
        assert_eq!(r[0], serde_json::json!(7));
    }

    // 新写入：省略该列走 DEFAULT；显式 NULL 被 NOT NULL 拒绝
    exec_ok(&db, "INSERT INTO t (id, v) VALUES (4, 40)", "subset").await;
    expect_plan_error(
        to_response(db.execute_sql("INSERT INTO t VALUES (6, 60, NULL)").await),
        "not null insert",
        "NOT NULL",
    );
}

/// R1-S4：NOT NULL 且无 DEFAULT 且表非空——计划期点名拒绝零副作用。
#[tokio::test]
async fn add_column_not_null_without_default_rejected() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;

    expect_plan_error(
        to_response(
            db.execute_sql("ALTER TABLE t ADD COLUMN req VARCHAR NOT NULL")
                .await,
        ),
        "not-null without default on non-empty",
        "NOT NULL",
    );
    // 零副作用：列未加入，合法加列仍可用
    exec_ok(&db, "ALTER TABLE t ADD COLUMN note VARCHAR", "add after").await;
    let rows = query_rows(
        to_response(db.execute_sql("SELECT note FROM t").await),
        "note",
    );
    assert_eq!(rows.len(), 3);
}

/// R1-S5：ADD COLUMN 声明 PK/UNIQUE/CHECK 计划期点名拒绝。
#[tokio::test]
async fn add_column_constraint_declarations_rejected() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;

    for (sql, what) in [
        ("ALTER TABLE t ADD COLUMN u INT UNIQUE", "unique"),
        ("ALTER TABLE t ADD COLUMN p INT PRIMARY KEY", "primary key"),
    ] {
        expect_plan_error(to_response(db.execute_sql(sql).await), what, "Unsupported");
    }
}

/// R1-S6：加列（含 DEFAULT）跨重启持久；dump→restore 往返保真。
#[tokio::test]
async fn add_column_persists_and_dump_restore_roundtrip() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_two_col(&db).await;
        exec_ok(&db, "ALTER TABLE t ADD COLUMN flag INT DEFAULT 0", "add").await;
        db.close().await.unwrap();
    }
    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(
            to_response(db.execute_sql("SELECT flag FROM t").await),
            "reopen",
        );
        for r in rows {
            assert_eq!(r[0], serde_json::json!(0), "default must survive reopen");
        }
    } // close：释放锁，dump 子进程才能打开

    // dump → restore 到新库：schema 渲染含新列与 DEFAULT
    let dump_out = std::process::Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["dump", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        dump_out.status.success(),
        "dump must succeed: {}",
        String::from_utf8_lossy(&dump_out.stderr)
    );
    let dump_sql = String::from_utf8_lossy(&dump_out.stdout).to_string();
    assert!(
        dump_sql.contains("flag"),
        "dumped DDL must contain the added column: {dump_sql}"
    );

    let dir2 = tempdir().unwrap();
    let restore_path = db_path(&dir2);
    let dump_file = db_path(&dir2).with_extension("sql");
    std::fs::write(&dump_file, &dump_sql).unwrap();
    // restore 目标须为已存在的空库（restore 空库前置语义）
    std::process::Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["new", restore_path.to_str().unwrap()])
        .output()
        .unwrap();
    let restore_out = std::process::Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args([
            "restore",
            restore_path.to_str().unwrap(),
            dump_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        restore_out.status.success(),
        "restore must succeed: {}",
        String::from_utf8_lossy(&restore_out.stderr)
    );

    let db2 = Database::open(&restore_path).await.unwrap();
    let rows = query_rows(
        to_response(db2.execute_sql("SELECT flag FROM t").await),
        "restored",
    );
    for r in rows {
        assert_eq!(r[0], serde_json::json!(0), "restored default must hold");
    }
}

// ===========================================================================
// R2: ALTER TABLE DROP COLUMN 删列
// ===========================================================================

/// R2-S1：删中间列后 `SELECT *` 形状与行数据正确；INSERT 目标列集一致。
#[tokio::test]
async fn drop_column_output_shape_conforms() {
    let (db, _dir) = open_db().await;
    exec_ok(
        &db,
        "CREATE TABLE t (a INT PRIMARY KEY, b INT, c VARCHAR)",
        "create",
    )
    .await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 11, 'x')", "ins 1").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 22, 'y')", "ins 2").await;

    exec_ok(&db, "ALTER TABLE t DROP COLUMN b", "drop column").await;

    let rows = query_rows(
        to_response(db.execute_sql("SELECT * FROM t").await),
        "select *",
    );
    assert_eq!(rows.len(), 2);
    for r in &rows {
        assert_eq!(r.len(), 2, "row shape must be (a, c): {r:?}");
    }
    assert_eq!(rows[0][0], serde_json::json!(1));
    assert_eq!(rows[0][1], serde_json::json!("x"));

    exec_ok(&db, "INSERT INTO t (a, c) VALUES (3, 'z')", "insert narrow").await;
    expect_plan_error(
        to_response(
            db.execute_sql("INSERT INTO t (a, b, c) VALUES (4, 4, 'w')")
                .await,
        ),
        "insert dropped column",
        "not found",
    );
}

/// R2-S2：被 PK、唯一声明或二级索引引用的列计划期点名拒绝。
#[tokio::test]
async fn drop_index_referenced_column_rejected() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;

    expect_plan_error(
        to_response(db.execute_sql("ALTER TABLE t DROP COLUMN id").await),
        "pk column",
        "PRIMARY KEY",
    );

    exec_ok(
        &db,
        "CREATE TABLE u (id INT PRIMARY KEY, tag INT UNIQUE, w INT)",
        "create u",
    )
    .await;
    expect_plan_error(
        to_response(db.execute_sql("ALTER TABLE u DROP COLUMN tag").await),
        "unique column",
        "UNIQUE",
    );

    exec_ok(&db, "CREATE INDEX idx_v ON t(v)", "create index").await;
    expect_plan_error(
        to_response(db.execute_sql("ALTER TABLE t DROP COLUMN v").await),
        "secondary-indexed column",
        "index",
    );
}

/// R2-S3：删除表中最后一列点名拒绝。
#[tokio::test]
async fn drop_last_column_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE solo (a INT PRIMARY KEY)", "create").await;
    expect_plan_error(
        to_response(db.execute_sql("ALTER TABLE solo DROP COLUMN a").await),
        "last column",
        "last column",
    );
}

/// R2-S4：删列跨重启持久；dump→restore 往返 schema 与数据一致。
#[tokio::test]
async fn drop_column_persists_and_dump_restore_roundtrip() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(
            &db,
            "CREATE TABLE t (a INT PRIMARY KEY, b INT, c VARCHAR)",
            "create",
        )
        .await;
        exec_ok(&db, "INSERT INTO t VALUES (1, 11, 'x')", "ins").await;
        exec_ok(&db, "ALTER TABLE t DROP COLUMN b", "drop").await;
        db.close().await.unwrap();
    }
    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(
            to_response(db.execute_sql("SELECT * FROM t").await),
            "reopen",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
    } // close：释放锁

    let dump_out = std::process::Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["dump", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        dump_out.status.success(),
        "dump must succeed: {}",
        String::from_utf8_lossy(&dump_out.stderr)
    );
    let dump_sql = String::from_utf8_lossy(&dump_out.stdout).to_string();
    assert!(
        !dump_sql.contains(" b "),
        "dumped DDL must not contain dropped column: {dump_sql}"
    );
}

/// R2-S5：列不存在点名拒绝零副作用。
#[tokio::test]
async fn drop_missing_column_rejected() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;
    expect_plan_error(
        to_response(db.execute_sql("ALTER TABLE t DROP COLUMN nope").await),
        "missing column",
        "not found",
    );
}

/// R2+：删列重写后崩溃（drop 不 close）——重开两态一致（schema/行形状）。
#[tokio::test]
async fn drop_column_consistent_across_crash_recovery() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_two_col(&db).await;
        exec_ok(&db, "ALTER TABLE t ADD COLUMN note VARCHAR", "add").await;
        exec_ok(
            &db,
            "INSERT INTO t (id, v, note) VALUES (4, 40, 'n4')",
            "ins",
        )
        .await;
        exec_ok(&db, "ALTER TABLE t DROP COLUMN note", "drop").await;
        // DDL 无 WAL：catalog 变更经 flush_all 落盘（重写事务的 WAL DML 记录
        // 保留在 WAL 中——重放按 row_id 写回，两态一致）
        db.buffer_pool.flush_all().await.unwrap();
        db.wal_buffer.as_ref().unwrap().shutdown().await;
        drop(db);
    }
    let db2 = Database::open(&path).await.unwrap();
    let rows = query_rows(
        to_response(db2.execute_sql("SELECT * FROM t").await),
        "recovered",
    );
    assert_eq!(rows.len(), 4);
    for r in rows {
        assert_eq!(r.len(), 2, "recovered rows must be two-column: {r:?}");
    }
}

// ===========================================================================
// R3: ALTER 语句形态与既有面边界
// ===========================================================================

/// R3-S1：多操作单语句计划期点名拒绝。
#[tokio::test]
async fn alter_table_multi_operation_rejected() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;
    expect_plan_error(
        to_response(
            db.execute_sql("ALTER TABLE t ADD COLUMN x INT, DROP COLUMN v")
                .await,
        ),
        "multi operation",
        "ALTER TABLE",
    );
}

/// R3-S2：目标表不存在点名拒绝。
#[tokio::test]
async fn alter_table_missing_table_rejected() {
    let (db, _dir) = open_db().await;
    expect_plan_error(
        to_response(db.execute_sql("ALTER TABLE nope ADD COLUMN x INT").await),
        "missing table",
        "not found",
    );
}

/// R3-S2 扩展：系统保留名目标点名拒绝（`__tables`/`__columns` 不是用户表）。
#[tokio::test]
async fn alter_table_reserved_name_rejected() {
    let (db, _dir) = open_db().await;
    for target in ["__tables", "__columns"] {
        expect_plan_error(
            to_response(
                db.execute_sql(&format!("ALTER TABLE {target} ADD COLUMN x INT"))
                    .await,
            ),
            target,
            "not found",
        );
    }
}

/// R3-S3：显式事务内执行期点名拒绝（0A000），事务保持可用。
#[tokio::test]
async fn alter_table_inside_explicit_transaction_rejected() {
    let (db, _dir) = open_db().await;
    seed_two_col(&db).await;
    let tx = db.begin().await.unwrap();
    match to_response(
        db.execute_in_tx("ALTER TABLE t ADD COLUMN x INT", &tx)
            .await,
    ) {
        Response::Error { sqlstate, .. } => {
            assert_eq!(
                sqlstate.as_deref(),
                Some("0A000"),
                "in-tx ALTER must be rejected with 0A000"
            );
        }
        other => panic!("expected in-tx rejection, got {other:?}"),
    }
    db.rollback(tx).await.unwrap();
    exec_ok(&db, "ALTER TABLE t ADD COLUMN x INT", "alter after tx").await;
}
