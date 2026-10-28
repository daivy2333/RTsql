//! End-to-end tests for full-table UPDATE/DELETE without a WHERE clause
//! (change 2026-10-04-external-review-gaps T6, R6/S22–S26).
//!
//! S22 全表 DELETE：affected_rows=实际行数、重开一致、dump 无行、空表 0
//! S23 全表 UPDATE：全部行生效 + per 行门序照常（FLOAT 升格 / NOT NULL /
//!      键列类型门，任一行失败零副作用）
//! S24 全表 rekey 中途碰撞：语句失败、隐式事务回滚全部已写行；显式事务内可
//!      ROLLBACK
//! S25 附加库目标经路由同语义（每文件独立提交、主库不变、中途失败回滚目标库）
//! S26 TRUNCATE 与带 WHERE 形态零变化（第二重门文案逐字节、子查询拒绝面）

#![cfg(all(feature = "network", feature = "cli"))]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

/// Helper: open a file-backed database in a fresh tempdir（重开见证需要持久文件）。
async fn open_db() -> (Database, TempDir) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("full_table_dml.db");
    let db = Database::open(&path).await.unwrap();
    (db, dir)
}

/// Helper: execute SQL and return the Response.
async fn exec(db: &Database, sql: &str) -> Response {
    to_response(db.execute_sql(sql).await)
}

/// Helper: extract rows from a QueryResult, panicking otherwise.
fn rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::AffectedRows { count } => {
            panic!("expected QueryResult, got AffectedRows({count})")
        }
        Response::Error { message, .. } => panic!("expected QueryResult, got Error: {message}"),
        Response::Pong => panic!("expected QueryResult, got Pong"),
    }
}

/// Helper: extract the affected-row count, panicking otherwise.
fn affected(resp: Response) -> u64 {
    match resp {
        Response::AffectedRows { count } => count,
        Response::Error { message, .. } => panic!("expected AffectedRows, got Error: {message}"),
        other => panic!("expected AffectedRows, got {other:?}"),
    }
}

/// Helper: extract the error message, panicking otherwise.
fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("expected Error, got {other:?}"),
    }
}

/// Helper: assert a `SELECT` over `t(id)` returns exactly the given id values.
async fn assert_ids(db: &Database, sql: &str, expected: &[i64], what: &str) {
    let r = rows(exec(db, sql).await);
    let got: Vec<i64> = r.iter().map(|row| row[0].as_i64().unwrap()).collect();
    assert_eq!(got, expected, "{what}");
}

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出（夹具模式与 tests/cross_db_dml_test.rs 一致）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .env_remove("RTSQL_WAL_CHECKPOINT_BYTES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_string(&mut stdout)
                    .unwrap();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                return CliOutput {
                    code: status.code(),
                    stdout,
                    stderr,
                };
            }
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

// === S22: full-table DELETE ===

/// S22：`DELETE FROM t` 删除全部行——affected_rows 为实际行数；逐行墓碑后
/// 重开一致（0 行）；对空表再删 affected_rows=0；dump 输出不含数据行。
#[tokio::test]
async fn s22_full_table_delete_affected_reopen_and_empty() {
    let (db, dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    for i in 1..=3 {
        exec(&db, &format!("INSERT INTO t VALUES ({i}, {i}0)")).await;
    }

    // 全表 DELETE：affected_rows = 实际行数。
    let n = affected(exec(&db, "DELETE FROM t").await);
    assert_eq!(n, 3, "full-table DELETE must report the real row count");

    assert_ids(
        &db,
        "SELECT id FROM t",
        &[],
        "table empty after full delete",
    )
    .await;

    // 空表再删：affected_rows = 0，零副作用。
    let n = affected(exec(&db, "DELETE FROM t").await);
    assert_eq!(n, 0, "full-table DELETE on an empty table reports 0");

    // 重开一致：墓碑提交后行不复活。
    let path = dir.path().join("full_table_dml.db");
    db.close().await.unwrap();
    drop(db);
    let reopened = Database::open(&path).await.unwrap();
    assert_ids(
        &reopened,
        "SELECT id FROM t",
        &[],
        "deleted rows stay gone after reopen",
    )
    .await;
    reopened.close().await.unwrap();
    // flock 随 FileStorage drop 释放（close() 仅 checkpoint）——先于 CLI dump。
    drop(reopened);

    // dump 无行：DDL 保留、INSERT 清零。
    let out = run_cli(
        dir.path(),
        &[
            "dump",
            dir.path().join("full_table_dml.db").to_str().unwrap(),
        ],
    );
    assert_eq!(out.code, Some(0), "dump: {}", out.stderr);
    assert!(
        out.stdout.contains("CREATE TABLE"),
        "dump keeps DDL, got: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("INSERT INTO"),
        "dump must not contain data rows after full-table DELETE, got: {}",
        out.stdout
    );
}

// === S23: full-table UPDATE ===

/// S23：`UPDATE t SET v = v + 1` 全部行生效（表达式右值按各自行旧行快照求值）。
#[tokio::test]
async fn s23_full_table_update_all_rows() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 10)").await;
    exec(&db, "INSERT INTO t VALUES (2, 20)").await;
    exec(&db, "INSERT INTO t VALUES (3, 30)").await;

    let n = affected(exec(&db, "UPDATE t SET v = v + 1").await);
    assert_eq!(n, 3, "full-table UPDATE must affect every row");

    let r = rows(exec(&db, "SELECT v FROM t ORDER BY id").await);
    let got: Vec<i64> = r.iter().map(|row| row[0].as_i64().unwrap()).collect();
    assert_eq!(got, vec![11, 21, 31], "every row updated by its own value");
}

/// S23：per 行门序照常——FLOAT 列整数值无损升格。
#[tokio::test]
async fn s23_full_table_update_float_upgrade() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE f (id INT PRIMARY KEY, x FLOAT)").await;
    exec(&db, "INSERT INTO f VALUES (1, 0.5)").await;
    exec(&db, "INSERT INTO f VALUES (2, 2.5)").await;

    let n = affected(exec(&db, "UPDATE f SET x = 1").await);
    assert_eq!(n, 2, "every row affected");

    let r = rows(exec(&db, "SELECT x FROM f ORDER BY id").await);
    let got: Vec<f64> = r.iter().map(|row| row[0].as_f64().unwrap()).collect();
    assert_eq!(
        got,
        vec![1.0, 1.0],
        "Int literal upgraded to Float losslessly"
    );
}

/// S23：per 行门序照常——NOT NULL 门：任一行命中即语句失败且零副作用
/// （隐式事务回滚本语句全部已写行）。
#[tokio::test]
async fn s23_full_table_update_not_null_gate() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE nn (id INT PRIMARY KEY, n INT NOT NULL)").await;
    exec(&db, "INSERT INTO nn VALUES (1, 1)").await;
    exec(&db, "INSERT INTO nn VALUES (2, 2)").await;

    let msg = error_msg(exec(&db, "UPDATE nn SET n = NULL").await);
    assert!(msg.contains("NOT NULL"), "NOT NULL gate must fire: {msg}");
    assert_ids(
        &db,
        "SELECT id FROM nn ORDER BY id",
        &[1, 2],
        "no row changed",
    )
    .await;
    let r = rows(exec(&db, "SELECT n FROM nn ORDER BY id").await);
    assert_eq!(r[0][0].as_i64(), Some(1), "row 1 unchanged");
    assert_eq!(r[1][0].as_i64(), Some(2), "row 2 unchanged");
}

/// S23：per 行门序照常——键列类型门：Int 键列拒绝非 Int/NULL 新值，零副作用。
#[tokio::test]
async fn s23_full_table_update_key_type_gate() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 10)").await;
    exec(&db, "INSERT INTO t VALUES (2, 20)").await;

    let msg = error_msg(exec(&db, "UPDATE t SET id = 1.5").await);
    assert!(
        msg.contains("expects INT"),
        "key type gate must fire: {msg}"
    );
    assert_ids(
        &db,
        "SELECT id FROM t ORDER BY id",
        &[1, 2],
        "no row changed",
    )
    .await;
}

// === S24: rekey mid-statement collision + rollback ===

/// S24：全表 rekey 中途碰撞——`SET id = id + 1` 处理首行即与新键碰撞，
/// 语句失败（DuplicateKey），隐式事务回滚全部已写行（本形态首行写入前
/// 即失败，恒等见证：两行形态首行写入成功、次行碰撞，全部已写行回滚
/// 由下一用例承载）。
#[tokio::test]
async fn s24_full_table_rekey_collision_fails() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 100)").await;
    exec(&db, "INSERT INTO t VALUES (2, 200)").await;
    exec(&db, "INSERT INTO t VALUES (3, 300)").await;

    let msg = error_msg(exec(&db, "UPDATE t SET id = id + 1").await);
    assert!(
        msg.to_lowercase().contains("duplicate"),
        "rekey collision must be rejected: {msg}"
    );
    assert_ids(
        &db,
        "SELECT id FROM t ORDER BY id",
        &[1, 2, 3],
        "rows intact",
    )
    .await;
    let r = rows(exec(&db, "SELECT v FROM t ORDER BY id").await);
    let got: Vec<i64> = r.iter().map(|row| row[0].as_i64().unwrap()).collect();
    assert_eq!(got, vec![100, 200, 300], "values untouched");
}

/// S24：首行写入成功、次行碰撞的三行形态——**全部已写行**（含首行）回滚
/// 零残留；显式事务内可 ROLLBACK 同型见证。
#[tokio::test]
async fn s24_full_table_rekey_mid_statement_rollback_all_written() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 100)").await;
    exec(&db, "INSERT INTO t VALUES (3, 300)").await;
    exec(&db, "INSERT INTO t VALUES (4, 400)").await;

    // 键升序处理：key=1 → 新键 2 无碰撞、写入成功（索引条目 1→2）；
    // key=3 → 新键 4 与未处理行碰撞 → 语句失败、隐式事务 abort。
    let msg = error_msg(exec(&db, "UPDATE t SET id = id + 1").await);
    assert!(
        msg.to_lowercase().contains("duplicate"),
        "second-row rekey collision must be rejected: {msg}"
    );
    // 首行已写（1→2 的新版本）必须随隐式事务回滚：三行原值仍在。
    assert_ids(
        &db,
        "SELECT id FROM t ORDER BY id",
        &[1, 3, 4],
        "rollback restored",
    )
    .await;
    let r = rows(exec(&db, "SELECT v FROM t ORDER BY id").await);
    let got: Vec<i64> = r.iter().map(|row| row[0].as_i64().unwrap()).collect();
    assert_eq!(
        got,
        vec![100, 300, 400],
        "no residue from the first written row"
    );

    // 显式事务：全表 UPDATE 成功后 ROLLBACK，全部行还原。
    let tx = db.begin().await.unwrap();
    let resp = to_response(db.execute_in_tx("UPDATE t SET v = v + 1", &tx).await);
    assert_eq!(
        affected(resp),
        3,
        "in-tx full-table update affects all rows"
    );
    db.rollback(tx).await.unwrap();
    let r = rows(exec(&db, "SELECT v FROM t ORDER BY id").await);
    let got: Vec<i64> = r.iter().map(|row| row[0].as_i64().unwrap()).collect();
    assert_eq!(
        got,
        vec![100, 300, 400],
        "explicit rollback restores all rows"
    );
}

// === S25: attached-database target ===

/// S25：附加库目标全表 DML 经 `routed_db` 同语义——写入/删除落在附加库
/// 文件（每文件独立提交），主库不变；目标库 per 行门失败回滚目标库已写行。
#[test]
fn s25_full_table_dml_on_attached_target() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    for name in ["app", "o"] {
        let out = run_cli(dir.path(), &["new", name]);
        assert_eq!(out.code, Some(0), "new {name}: {}", out.stderr);
    }

    // 主库 t(1) 不变见证；附加库 u 三行 + nn NOT NULL 门表。
    let seed_main = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE t (id INT PRIMARY KEY); INSERT INTO t VALUES (7)",
        ],
    );
    assert_eq!(seed_main.code, Some(0), "seed main: {}", seed_main.stderr);
    let seed_o = run_cli(
        dir.path(),
        &[
            "o",
            "CREATE TABLE u (id INT PRIMARY KEY, tag VARCHAR); \
             INSERT INTO u VALUES (1, 'x'); \
             INSERT INTO u VALUES (2, 'y'); \
             INSERT INTO u VALUES (3, 'z'); \
             CREATE TABLE nn (id INT PRIMARY KEY, n INT NOT NULL); \
             INSERT INTO nn VALUES (1, 1); \
             INSERT INTO nn VALUES (2, 2)",
        ],
    );
    assert_eq!(seed_o.code, Some(0), "seed attached: {}", seed_o.stderr);

    // 全表 UPDATE 附加目标：affected 3；主库不变。
    let upd = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; UPDATE o.u SET tag = 'q'"],
    );
    assert_eq!(upd.code, Some(0), "attached full update: {}", upd.stderr);
    assert!(
        upd.stdout.contains("\"affected_rows\":3"),
        "attached full update affected 3, got: {}",
        upd.stdout
    );

    // per 行门失败：目标库已写行回滚（语句中途失败 → 目标库隐式事务 abort）。
    let gate = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; UPDATE o.nn SET n = NULL"],
    );
    assert_eq!(gate.code, Some(3), "attached gate failure: {}", gate.stderr);
    assert!(
        gate.stderr.contains("NOT NULL"),
        "gate message surfaced: {}",
        gate.stderr
    );

    // 全表 DELETE 附加目标：affected 3；重开附加库两态一致。
    let del = run_cli(dir.path(), &["app", "ATTACH 'o' AS o; DELETE FROM o.u"]);
    assert_eq!(del.code, Some(0), "attached full delete: {}", del.stderr);
    assert!(
        del.stdout.contains("\"affected_rows\":3"),
        "attached full delete affected 3, got: {}",
        del.stdout
    );

    // 重开见证：附加库 u 空、nn 原值未动；主库 t 不变。
    let u = run_cli(dir.path(), &["o", "SELECT * FROM u"]);
    assert_eq!(u.code, Some(0), "reopen attached u: {}", u.stderr);
    assert!(
        u.stdout.contains("\"rows\":[]"),
        "attached u must be empty after full delete, got: {}",
        u.stdout
    );
    let nn = run_cli(dir.path(), &["o", "SELECT n FROM nn ORDER BY id"]);
    assert!(
        nn.stdout.contains("[[1],[2]]"),
        "attached nn untouched by rolled-back statement, got: {}",
        nn.stdout
    );
    let main_t = run_cli(dir.path(), &["app", "SELECT id FROM t"]);
    assert!(
        main_t.stdout.contains("[[7]]"),
        "main db untouched, got: {}",
        main_t.stdout
    );
}

// === S26: TRUNCATE and WHERE forms unchanged ===

/// S26：TRUNCATE 语义零变化；带 WHERE PK 等值单行形态逐字节；带 WHERE 非
/// PK 谓词维持第二重门点名拒绝（文案逐字节）；子查询拒绝面维持。
#[tokio::test]
async fn s26_truncate_and_where_forms_zero_change() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec(&db, "INSERT INTO t VALUES (1, 10)").await;
    exec(&db, "INSERT INTO t VALUES (2, 20)").await;

    // 带 WHERE PK 等值：单行形态照常。
    let n = affected(exec(&db, "UPDATE t SET v = 99 WHERE id = 1").await);
    assert_eq!(n, 1, "single-row PK-equality update unchanged");
    let n = affected(exec(&db, "DELETE FROM t WHERE id = 2").await);
    assert_eq!(n, 1, "single-row PK-equality delete unchanged");

    // 第二重门：带 WHERE 非 PK 等值维持点名拒绝（文案逐字节；ISS11 起
    // 解析类错误恰一层 `Parse error:`，PlanFailure 不再叠加 `Plan error:`）。
    let msg = error_msg(exec(&db, "UPDATE t SET v = 1 WHERE v = 99").await);
    assert_eq!(
        msg, "Parse error: UPDATE requires primary key equality in WHERE clause",
        "second gate text byte-exact"
    );
    let msg = error_msg(exec(&db, "DELETE FROM t WHERE v = 99").await);
    assert_eq!(
        msg, "Parse error: DELETE requires primary key equality in WHERE clause",
        "second gate text byte-exact"
    );

    // 子查询拒绝面维持（右值 / IN 条件）。
    let msg = error_msg(exec(&db, "UPDATE t SET v = (SELECT 1)").await);
    assert!(
        msg.contains("UPDATE SUBQUERY"),
        "subquery right-value rejection kept: {msg}"
    );

    // TRUNCATE 语义零变化：checkpoint-first 物理清空、定义保留。
    exec(&db, "INSERT INTO t VALUES (5, 50)").await;
    let resp = exec(&db, "TRUNCATE TABLE t").await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "TRUNCATE must keep working, got {resp:?}"
    );
    assert_ids(&db, "SELECT id FROM t", &[], "truncate emptied the table").await;
    let r = rows(exec(&db, "SELECT id FROM t").await);
    assert!(r.is_empty(), "truncate keeps no rows");
}

/// S26：空表全表 UPDATE affected_rows=0；对不存在键的单行 DELETE 既有
/// 未命中零副作用语义不变（对照 S22 的全表形态）。
#[tokio::test]
async fn s26_full_table_update_empty_table_and_miss_forms() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;

    let n = affected(exec(&db, "UPDATE t SET v = 1").await);
    assert_eq!(n, 0, "full-table UPDATE on an empty table reports 0");

    // 单行未命中（既有语义对照）：affected 0。
    exec(&db, "INSERT INTO t VALUES (1, 10)").await;
    let n = affected(exec(&db, "DELETE FROM t WHERE id = 99").await);
    assert_eq!(n, 0, "miss-keyed single-row delete stays affected 0");
}

// === S27: multi-level PK tree full-table DML（本 change T2 / B2） ===

/// S27：≥3 层 PK 树（乱序 20000 唯一键）上无 WHERE 全表 UPDATE/DELETE 的
/// `scan_all` 枚举必须触达全部行——affected_rows = 行数、逐行生效。
/// RED 见证（B2）：修复前多层树 scan_all 返回空/碎片，UPDATE 中途
/// `Abort failed: Key not found` 或 affected 严重偏少。
#[tokio::test]
async fn s27_full_table_dml_multi_level_pk_tree() {
    let (db, _dir) = open_db().await;
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;

    // 20000 个乱序唯一键（确定性乘法哈希序）→ PK 树 ≥3 层。
    let mut ids: Vec<i64> = (1..=20000).collect();
    ids.sort_by_key(|id| id.wrapping_mul(2654435761) % 1000003);
    for chunk in ids.chunks(1000) {
        let values = chunk
            .iter()
            .map(|id| format!("({id}, {})", id * 10))
            .collect::<Vec<_>>()
            .join(", ");
        exec(&db, &format!("INSERT INTO t VALUES {values}")).await;
    }

    // 全表 UPDATE：affected = 行数，SUM 证明逐行恰生效一次（v = id*10 + 1）。
    let n = affected(exec(&db, "UPDATE t SET v = v + 1").await);
    assert_eq!(
        n, 20000,
        "full-table UPDATE must touch every row of a multi-level PK tree"
    );
    let r = rows(exec(&db, "SELECT SUM(v) AS s FROM t").await);
    assert_eq!(
        r[0][0].as_i64(),
        Some(20000 * 20001 / 2 * 10 + 20000),
        "every row updated exactly once"
    );
    // 抽查边界行
    let r = rows(exec(&db, "SELECT v AS x FROM t WHERE id = 1").await);
    assert_eq!(r[0][0].as_i64(), Some(11));
    let r = rows(exec(&db, "SELECT v AS x FROM t WHERE id = 20000").await);
    assert_eq!(r[0][0].as_i64(), Some(200001));

    // 全表 DELETE：affected = 行数，表空。
    let n = affected(exec(&db, "DELETE FROM t").await);
    assert_eq!(
        n, 20000,
        "full-table DELETE must touch every row of a multi-level PK tree"
    );
    assert_ids(&db, "SELECT id FROM t", &[], "empty after full delete").await;
}
