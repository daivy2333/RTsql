//! Integration tests for MS20-T02 Iter001 (`sql-write-surface` R7, plus
//! R2/R3/R4/R5 witnesses): `INSERT INTO <existing> SELECT ...` against main
//! and attached targets — source materialization, column mapping, conflict
//! actions, failure boundaries and explicit-transaction semantics.
//!
//! Fixture (three CLI-created databases under an isolated `RTSQL_HOME`):
//!   main `app`     t(id,name) = (1,'a'),(2,'b')   — join source / self target
//!                  t2(id,tag)                    — main write target
//!                  conf(id,v) = (1,'old')        — conflict-action target
//!   attached `o`   u(id,tag) = (1,'x'),(9,'y')    — join/pipe source
//!                  dst(id,label)                 — attached write target
//!                  d(id,x DEFAULT 7,s DEFAULT 'z')         — mapping/DEFAULT
//!                  nul(id,v) = (5,NULL)                    — explicit NULL source
//!                  f(id,x FLOAT,dt DATE)                   — type-gate target
//!                  strs(id,s) = (1,'not-a-date')           — runtime source error
//!                  src2(id,n,uniq) = (1,10,100),(2,NULL,101) — late constraint error
//!                  c(id INT PK, n NOT NULL, uniq UNIQUE) + idx_c_n   — rollback target
//!                  conf(id,v) = (1,'old')        — attached conflict target
//!
//! Scenario mapping (requirement → scenario → test):
//!   R7/S1 多库查询写入附加目标   insert_select_cross_db_join_and_main_target
//!   R7/S2 写回主库与同表读写     insert_select_cross_db_join_and_main_target（主库臂）
//!                                   self_source_materializes_once（同表臂）
//!   R7/S3 目标列映射与 DEFAULT   column_mapping_defaults_and_empty_source
//!   R7/S4 空源仍验证列结构       column_mapping_defaults_and_empty_source
//!   R7/S5 源失败与插入失败       source_error_no_target_write
//!                                   insert_select_late_constraint_error_rolls_back_target
//!   R7/S6 SELECT 源冲突动作      select_conflict_actions
//!   R7/S7 主库显式事务读写       insert_select_main_explicit_tx_commit_and_rollback
//!   R5    目标 checkpoint 失败保提交  insert_select_target_auto_checkpoint_failure_preserves_commit

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::pipeline::{AttachRoute, RoutedTables};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic（夹具模式与
/// tests/cross_db_dml_test.rs 一致）。
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

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录。
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

fn run_cli_ok(dir: &Path, db: &str, sql: &str, what: &str) {
    let out = run_cli(dir, &[db, sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
}

fn assert_failure(out: &CliOutput, code: i32, needle: &str, what: &str) {
    assert_eq!(out.code, Some(code), "{what}: stderr was {}", out.stderr);
    assert!(
        out.stderr.contains(needle),
        "{what}: expected stderr containing {needle:?}, got {:?}",
        out.stderr
    );
}

fn assert_rows(out: &CliOutput, rows: &str, what: &str) {
    assert_eq!(out.code, Some(0), "{what}: stderr was {}", out.stderr);
    assert!(
        out.stdout.contains(rows),
        "{what}: expected stdout containing {rows}, got {:?}",
        out.stdout
    );
}

/// 主库 `app` + 附加库 `o`（写入目标/约束/DEFAULT/类型门/错误源表）。
fn is_fixture() -> TempDir {
    let dir = cli_fixture();
    for name in ["app", "o"] {
        let out = run_cli(dir.path(), &["new", name]);
        assert_eq!(out.code, Some(0), "new {name} failed: {}", out.stderr);
    }
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR); \
         INSERT INTO t VALUES (1, 'a'); \
         INSERT INTO t VALUES (2, 'b'); \
         CREATE TABLE t2 (id INT PRIMARY KEY, tag VARCHAR); \
         CREATE TABLE conf (id INT PRIMARY KEY, v VARCHAR); \
         INSERT INTO conf VALUES (1, 'old')",
        "seed main db",
    );
    run_cli_ok(
        dir.path(),
        "o",
        "CREATE TABLE u (id INT PRIMARY KEY, tag VARCHAR); \
         INSERT INTO u VALUES (1, 'x'); \
         INSERT INTO u VALUES (9, 'y'); \
         CREATE TABLE dst (id INT PRIMARY KEY, label VARCHAR); \
         CREATE TABLE d (id INT PRIMARY KEY, x INT DEFAULT 7, s VARCHAR DEFAULT 'z'); \
         CREATE TABLE nul (id INT PRIMARY KEY, v VARCHAR); \
         INSERT INTO nul VALUES (5, NULL); \
         CREATE TABLE f (id INT PRIMARY KEY, x FLOAT, dt DATE); \
         CREATE TABLE strs (id INT PRIMARY KEY, s VARCHAR); \
         INSERT INTO strs VALUES (1, 'not-a-date'); \
         CREATE TABLE src2 (id INT, n INT, uniq INT); \
         INSERT INTO src2 VALUES (1, 10, 100); \
         INSERT INTO src2 VALUES (2, NULL, 101); \
         CREATE TABLE c (id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE); \
         CREATE INDEX idx_c_n ON c (n); \
         CREATE TABLE conf (id INT PRIMARY KEY, v VARCHAR); \
         INSERT INTO conf VALUES (1, 'old')",
        "seed attached o",
    );
    dir
}

// ---------------------------------------------------------------------------
// R7/S1 + R7/S2 — cross-db JOIN source, main and attached targets
// ---------------------------------------------------------------------------

/// R7/S1：主库 t JOIN 附加 o.u 的源写入附加目标 o.dst；R7/S2 主库臂——附加
/// o.u 写回主库 t2。行与计数准确、源表不变、重开（新 CLI 调用）结果一致。
#[test]
fn insert_select_cross_db_join_and_main_target() {
    let dir = is_fixture();

    // 附加目标：主库 t JOIN 附加 o.u（等值连接，1 行命中）。列引用用全限定
    // 形态——附加限定表的短名限定引用是既有边界（纯 SELECT 同面），不扩展。
    let joined = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.dst SELECT t.id, o.u.tag FROM t JOIN o.u ON t.id = o.u.id",
        ],
    );
    assert_eq!(joined.code, Some(0), "join insert: {}", joined.stderr);
    assert!(
        joined.stdout.contains(r#""affected_rows":1"#),
        "join source must yield 1 row, got {:?}",
        joined.stdout
    );
    let dst = run_cli(dir.path(), &["o", "SELECT * FROM dst ORDER BY id"]);
    assert_rows(&dst, r#""rows":[[1,"x"]]"#, "attached target rows");
    // 源表不变。
    let src = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(&src, r#""rows":[[1,"a"],[2,"b"]]"#, "main source unchanged");
    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "attached source unchanged",
    );

    // 主库目标：附加 o.u 全量写回主库 t2。
    let main_target = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO t2 SELECT id, tag FROM o.u",
        ],
    );
    assert_eq!(
        main_target.code,
        Some(0),
        "main target: {}",
        main_target.stderr
    );
    assert!(
        main_target.stdout.contains(r#""affected_rows":2"#),
        "main target must take 2 rows, got {:?}",
        main_target.stdout
    );
    let t2 = run_cli(dir.path(), &["app", "SELECT * FROM t2 ORDER BY id"]);
    assert_rows(
        &t2,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "main target rows after reopen",
    );
}

// ---------------------------------------------------------------------------
// R7/S3 + R7/S4 — column mapping, DEFAULT fill, NULL preservation, empty source
// ---------------------------------------------------------------------------

/// R7/S3：子集重排清单按位置映射；省略位落声明 DEFAULT；显式源 NULL 保持
/// NULL；类型门与 VALUES 同面。R7/S4：空源 affected_rows=0 且结构校验不
/// 依赖行。
#[test]
fn column_mapping_defaults_and_empty_source() {
    let dir = is_fixture();

    // 重排子集：(s, id) ← (tag, id)；省略位 x 落 DEFAULT 7。
    let mapped = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.d (s, id) SELECT tag, id FROM o.u",
        ],
    );
    assert_eq!(mapped.code, Some(0), "mapped insert: {}", mapped.stderr);
    let d = run_cli(dir.path(), &["o", "SELECT * FROM d ORDER BY id"]);
    assert_rows(
        &d,
        r#""rows":[[1,7,"x"],[9,7,"y"]]"#,
        "reordered subset fills declared DEFAULT",
    );

    // 显式源 NULL 保持 NULL（s 列显式 NULL，不被 DEFAULT 'z' 替换）。
    let nulls = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.d (id, s) SELECT id, v FROM o.nul",
        ],
    );
    assert_eq!(nulls.code, Some(0), "null insert: {}", nulls.stderr);
    let d2 = run_cli(dir.path(), &["o", "SELECT id, x, s FROM d WHERE id = 5"]);
    assert_rows(
        &d2,
        r#""rows":[[5,7,null]]"#,
        "explicit source NULL stays NULL",
    );

    // 类型门与 VALUES 同面：STRING 源值写入 FLOAT 列点名拒绝。
    let typed = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.f (id, x) SELECT id, tag FROM o.u",
        ],
    );
    assert_failure(&typed, 3, "expects FLOAT", "type gate matches VALUES face");

    // 空源：affected_rows=0，目标不变（src2 三列对齐 d 全列接收）。
    let empty = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.d SELECT id, n, uniq FROM o.src2 WHERE id > 100",
        ],
    );
    assert_eq!(empty.code, Some(0), "empty source: {}", empty.stderr);
    assert!(
        empty.stdout.contains(r#""affected_rows":0"#),
        "empty source must report 0, got {:?}",
        empty.stdout
    );
    let d3 = run_cli(dir.path(), &["o", "SELECT COUNT(*) FROM d"]);
    assert_rows(&d3, r#""rows":[[3]]"#, "empty source writes nothing");

    // 空源仍做计划期结构校验：宽度不符在源求值前拒绝。
    let malformed = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.d (id) SELECT id, tag FROM o.u WHERE id > 100",
        ],
    );
    assert_failure(
        &malformed,
        3,
        "expects 1 values, got 2",
        "structural validation with empty source",
    );
}

// ---------------------------------------------------------------------------
// R7/S2 — self-source write-back
// ---------------------------------------------------------------------------

/// R7/S2：同表写回只消费物化前行——主库与附加目标各一条，调用有限结束，
/// 结果行 = 原行 ∪ 变换行（无递归自读自写）。
#[test]
fn self_source_materializes_once() {
    let dir = is_fixture();

    let main_self = run_cli(
        dir.path(),
        &["app", "INSERT INTO t SELECT id + 10, name FROM t"],
    );
    assert_eq!(
        main_self.code,
        Some(0),
        "main self insert: {}",
        main_self.stderr
    );
    assert!(
        main_self.stdout.contains(r#""affected_rows":2"#),
        "self source must consume only pre-write rows (2), got {:?}",
        main_self.stdout
    );
    let t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(
        &t,
        r#""rows":[[1,"a"],[2,"b"],[11,"a"],[12,"b"]]"#,
        "main self write-back is finite",
    );

    let attached_self = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.u SELECT id + 100, tag FROM o.u",
        ],
    );
    assert_eq!(
        attached_self.code,
        Some(0),
        "attached self insert: {}",
        attached_self.stderr
    );
    assert!(
        attached_self.stdout.contains(r#""affected_rows":2"#),
        "attached self source must consume only pre-write rows (2), got {:?}",
        attached_self.stdout
    );
    let u = run_cli(dir.path(), &["o", "SELECT COUNT(*) FROM u"]);
    assert_rows(&u, r#""rows":[[4]]"#, "attached self write-back is finite");
}

// ---------------------------------------------------------------------------
// R7/S6 — conflict actions on SELECT source
// ---------------------------------------------------------------------------

/// R7/S6：DO NOTHING / DO UPDATE（excluded 形态）/ REPLACE INTO ... SELECT 在
/// 主库与附加目标复用既有仲裁与计数语义；非法冲突目标继续拒绝。
#[test]
fn select_conflict_actions() {
    let dir = is_fixture();

    // 附加目标 DO NOTHING：冲突行跳过不计、新行插入。
    let nothing = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.conf (id, v) SELECT id, tag FROM o.u ON CONFLICT (id) DO NOTHING",
        ],
    );
    assert_eq!(nothing.code, Some(0), "DO NOTHING: {}", nothing.stderr);
    assert!(
        nothing.stdout.contains(r#""affected_rows":1"#),
        "DO NOTHING counts inserted rows only, got {:?}",
        nothing.stdout
    );
    let conf = run_cli(dir.path(), &["o", "SELECT * FROM conf ORDER BY id"]);
    assert_rows(
        &conf,
        r#""rows":[[1,"old"],[9,"y"]]"#,
        "DO NOTHING keeps the conflicting old row",
    );

    // 附加目标 DO UPDATE（excluded 形态）：冲突行原位更新。
    let upd = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.conf (id, v) SELECT id, tag FROM o.u \
             ON CONFLICT (id) DO UPDATE SET v = excluded.v",
        ],
    );
    assert_eq!(upd.code, Some(0), "DO UPDATE: {}", upd.stderr);
    let conf2 = run_cli(dir.path(), &["o", "SELECT * FROM conf ORDER BY id"]);
    assert_rows(
        &conf2,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "DO UPDATE rewrites the conflicting row in place",
    );

    // 主库目标 REPLACE INTO ... SELECT：冲突行删除后重插。
    let replace = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; REPLACE INTO conf SELECT id, tag FROM o.u",
        ],
    );
    assert_eq!(replace.code, Some(0), "REPLACE: {}", replace.stderr);
    let conf3 = run_cli(dir.path(), &["app", "SELECT * FROM conf ORDER BY id"]);
    assert_rows(
        &conf3,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "REPLACE INTO ... SELECT deletes then reinserts",
    );

    // 非法冲突目标继续拒绝（v 无唯一性约束）。
    let illegal = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.conf SELECT id, tag FROM o.u ON CONFLICT (v) DO NOTHING",
        ],
    );
    assert_failure(
        &illegal,
        3,
        "PRIMARY KEY or UNIQUE",
        "illegal conflict target on SELECT source",
    );
}

// ---------------------------------------------------------------------------
// R7/S5 — source failure / late constraint failure
// ---------------------------------------------------------------------------

/// R7/S5 源失败臂：源求值运行期失败（CAST 解析错误）不写目标；计划期源
/// 错误（未知列）同样零写入。
#[test]
fn source_error_no_target_write() {
    let dir = is_fixture();

    // 运行期源失败：CAST(s AS DATE) 逐行解析失败 → 目标 t2 零写入
    //（清单 (id) 与源单列宽度匹配，拒绝只能来自源求值）。
    let runtime = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO t2 (id) SELECT CAST(s AS DATE) FROM o.strs",
        ],
    );
    assert_failure(&runtime, 3, "Type mismatch", "runtime source failure");
    let t2 = run_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t2"]);
    assert_rows(
        &t2,
        r#""rows":[[0]]"#,
        "target untouched after source failure",
    );

    // 计划期源错误：未知列在源求值前点名。
    let planned = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO t2 SELECT nope FROM o.u",
        ],
    );
    assert_failure(&planned, 3, "not found", "plan-time source error");
    let t2b = run_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t2"]);
    assert_rows(&t2b, r#""rows":[[0]]"#, "target untouched after plan error");
}

/// R7/S5 插入失败臂：SELECT 源多行写入中后行违反 NOT NULL → 本语句全部行
/// 与索引回滚，已有数据保持；键/唯一值可重用，二级索引路由在重试后可达。
#[test]
fn insert_select_late_constraint_error_rolls_back_target() {
    let dir = is_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.c SELECT id, n, uniq FROM o.src2",
        ],
    );
    assert_failure(
        &out,
        3,
        "NOT NULL",
        "late constraint error fails the statement",
    );

    // 回滚见证：目标 c 零残留（首行 (1,10,100) 也被还原）。
    let c = run_cli(dir.path(), &["o", "SELECT * FROM c ORDER BY id"]);
    assert_rows(&c, r#""rows":[]"#, "failed statement fully rolled back");

    // 首行的键与唯一值可重用；二级索引路由到重建后的条目。
    let retry = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.c (id, n, uniq) VALUES (1, 10, 100)",
        ],
    );
    assert_eq!(retry.code, Some(0), "keys reusable: {}", retry.stderr);
    let idx = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; SELECT * FROM o.c WHERE n = 10"],
    );
    assert_rows(
        &idx,
        r#""rows":[[1,10,100]]"#,
        "secondary index after rollback and retry",
    );
}

// ---------------------------------------------------------------------------
// R4/R3 — write-guard coverage for the InsertSelect shape
// ---------------------------------------------------------------------------

/// R4：只读会话的 INSERT SELECT 按 25006 面拒绝（`cannot execute INSERT in
/// a read-only session`），先于源求值——目标与源表零变化。
#[test]
fn readonly_insert_select_rejected_before_source_evaluation() {
    let dir = is_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "--readonly",
            "app",
            "ATTACH 'o' AS o; INSERT INTO t2 SELECT id, tag FROM o.u",
        ],
    );
    assert_failure(&out, 3, "read-only session", "readonly INSERT SELECT");

    let t2 = run_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t2"]);
    assert_rows(&t2, r#""rows":[[0]]"#, "readonly target untouched");
    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "readonly source intact",
    );
}

/// R3：显式事务内附加目标 INSERT SELECT 点名拒绝（0A000 面），先于源求值
/// 与目标 begin——会话事务沿既有失败收口回滚，主库未提交修改撤销。
#[test]
fn attached_target_insert_select_in_tx_rejected() {
    let dir = is_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; BEGIN; INSERT INTO t VALUES (3, 'c'); \
             INSERT INTO o.dst SELECT id, tag FROM o.u",
        ],
    );
    assert_failure(&out, 3, "inside a transaction", "attached target in tx");

    // 主库事务已回滚、附加库目标未被写入。
    let main_t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(
        &main_t,
        r#""rows":[[1,"a"],[2,"b"]]"#,
        "main tx rolled back",
    );
    let dst = run_cli(dir.path(), &["o", "SELECT COUNT(*) FROM dst"]);
    assert_rows(&dst, r#""rows":[[0]]"#, "attached target untouched in tx");
}

// ---------------------------------------------------------------------------
// R7/S7 — main-target explicit transaction (COMMIT / ROLLBACK)
// ---------------------------------------------------------------------------

/// R7/S7：BEGIN 内主库目标 INSERT SELECT（附加库源）复用会话事务——COMMIT
/// 保留、ROLLBACK 撤销，附加源不变。
#[test]
fn insert_select_main_explicit_tx_commit_and_rollback() {
    let dir = is_fixture();

    let commit = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; BEGIN; INSERT INTO t2 SELECT id, tag FROM o.u; COMMIT",
        ],
    );
    assert_eq!(commit.code, Some(0), "commit case: {}", commit.stderr);
    let t2 = run_cli(dir.path(), &["app", "SELECT * FROM t2 ORDER BY id"]);
    assert_rows(
        &t2,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "COMMIT keeps the target insert",
    );
    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "attached source unchanged by tx",
    );

    let rollback = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; BEGIN; INSERT INTO t2 SELECT id + 50, tag FROM o.u; ROLLBACK",
        ],
    );
    assert_eq!(rollback.code, Some(0), "rollback case: {}", rollback.stderr);
    let t2b = run_cli(dir.path(), &["app", "SELECT * FROM t2 ORDER BY id"]);
    assert_rows(
        &t2b,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "ROLLBACK undoes the target insert",
    );
}

// ---------------------------------------------------------------------------
// R5 — target auto-checkpoint failure after commit
// ---------------------------------------------------------------------------

// lib routed helpers（模式与 cross_db_dml_test 一致）。

fn assert_ok(resp: Response, what: &str) {
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {resp:?}"
    );
}

async fn routed_exec(main: &Database, route: &AttachRoute, sql: &str) -> Response {
    use rtsql::parser::parse_sql;
    use rtsql::pipeline::{execute_stage_routed, plan_stage_routed};
    let stmts = parse_sql(sql).expect("parse routed sql");
    let stmt = stmts.first().expect("one statement");
    let mut routed = RoutedTables::new();
    let plan = plan_stage_routed(main, sql, stmt, false, Some(route), &mut routed)
        .await
        .unwrap_or_else(|e| panic!("plan {sql}: {e}"));
    to_response(execute_stage_routed(main, plan, false, &routed).await)
}

async fn query(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql}: expected QueryResult, got {other:?}"),
    }
}

/// R5（design D4）：目标库提交后的自动 checkpoint 失败——事务已提交（错误
/// 消息点名 `Commit failed` / `checkpoint failed`），SELECT 源写入的行与
/// 索引保持；修复现场后重开数据可读。
#[tokio::test]
async fn insert_select_target_auto_checkpoint_failure_preserves_commit() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("main.db");
    let other_path = dir.path().join("other.db");

    let other = Arc::new(Database::open(&other_path).await.unwrap());
    assert_ok(
        to_response(
            other
                .execute_sql("CREATE TABLE dst (id INT PRIMARY KEY, label VARCHAR)")
                .await,
        ),
        "create attached dst",
    );
    other.checkpoint().await.unwrap();
    // 1 字节水位：下一次提交边界必然触发自动 checkpoint（注入后必失败）。
    other.set_checkpoint_bytes_threshold(1);

    let main = Database::open(&main_path).await.unwrap();
    assert_ok(
        to_response(
            main.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, tag VARCHAR)")
                .await,
        ),
        "create main t",
    );
    assert_ok(
        to_response(main.execute_sql("INSERT INTO t VALUES (1, 'a')").await),
        "seed main row 1",
    );
    assert_ok(
        to_response(main.execute_sql("INSERT INTO t VALUES (2, 'b')").await),
        "seed main row 2",
    );

    let mut dbs = HashMap::new();
    dbs.insert("o".to_string(), other.clone());
    let route = AttachRoute::new(dbs);

    // 注入 checkpoint 步骤 5（位点写入）失败：位点路径被目录占据。
    let site = other_path.with_extension("checkpoint");
    let _ = std::fs::remove_file(&site);
    std::fs::create_dir(&site).unwrap();

    let resp = routed_exec(&main, &route, "INSERT INTO o.dst SELECT id, tag FROM t").await;
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains("Commit failed") && message.contains("checkpoint failed"),
                "commit-boundary checkpoint failure must be named, got: {message}"
            );
        }
        other => panic!("expected Error from failed checkpoint, got {other:?}"),
    }

    // 已提交数据保持（R4：checkpoint 失败不回滚已提交事务）——同实例行在。
    let rows = query(&other, "SELECT * FROM dst ORDER BY id").await;
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(1), serde_json::json!("a")],
            vec![serde_json::json!(2), serde_json::json!("b")]
        ],
        "committed SELECT-source rows must survive checkpoint failure"
    );
    // 主库（源）不变。
    let main_rows = query(&main, "SELECT * FROM t ORDER BY id").await;
    assert_eq!(
        main_rows.len(),
        2,
        "source rows must be untouched: {main_rows:?}"
    );

    // 崩溃丢弃 + 修复现场后重开：数据经 WAL 恢复。
    other.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(other);
    drop(route);
    drop(main);
    let _ = std::fs::remove_dir_all(&site);
    let reopened = Database::open(&other_path).await.unwrap();
    let rows = query(&reopened, "SELECT * FROM dst ORDER BY id").await;
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(1), serde_json::json!("a")],
            vec![serde_json::json!(2), serde_json::json!("b")]
        ],
        "committed rows must be recoverable after checkpoint failure"
    );
    reopened.close().await.unwrap();
}
