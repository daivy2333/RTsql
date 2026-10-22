//! Integration tests for MS20 T4 (`create-table-as-select` R1):
//! `CREATE TABLE <name> AS <SELECT>` materialization, output-shape inference,
//! cross-database sources, and the zero-side-effect rejection surface.
//!
//! Fixture shape (one CLI call seeds each database):
//!   main `app`      t(k INT, name VARCHAR)   = (1,'a'),(2,'b'),(3,'c')
//!                   tn(k INT, v VARCHAR)     = (1,NULL),(2,NULL)   — all-NULL column
//!                   ts2(k INT, s VARCHAR)    = (1,'s')             — String source
//!                   tf(k INT, v FLOAT)       = (1,2.5)             — Float source
//!                   td(k INT, d DATE, ts TIMESTAMP)                — typed source
//!                   dup(k INT, id INT)       = (1,1),(2,1)         — duplicate non-key value
//!   attached `other`  u(id INT, tag VARCHAR) = (1,'x'),(9,'y')     — cross-db source
//!
//! Scenario mapping (requirement → scenario → test):
//!   S1 主库物化 + 重启读回                  ctas_materializes_main_database_rows
//!   S2 跨库源（数据库视作表）                ctas_from_attached_database_source
//!   S3 目标已存在拒绝                        ctas_target_already_exists_rejected
//!   S4 附加库目标拒绝                        ctas_target_in_attached_database_rejected
//!   S5 表达式与别名输出形状                  ctas_expression_alias_defines_output_shape
//!   S6 类型推导矩阵                          ctas_infers_column_types_from_output_values
//!   S6 不可行混合拒绝                        ctas_incompatible_type_mix_rejected
//!   S6 输出列名重复拒绝                      ctas_duplicate_output_column_names_rejected
//!   S7 隐式主键冲突零副作用                  ctas_implicit_primary_key_conflict_leaves_no_table
//!   S8 源失败零副作用                        ctas_source_failure_leaves_no_table
//!   S9 显式事务内拒绝                        ctas_inside_explicit_transaction_rejected
//!   S10 混合形态 / LIKE / CLONE 拒绝         ctas_mixed_definition_and_query_rejected
//!                                          ctas_like_and_clone_rejected

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （夹具模式与 tests/cross_db_query_test.rs 一致）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
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

fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

fn run_cli_ok(dir: &Path, db: &str, sql: &str, what: &str) {
    let out = run_cli(dir, &[db, sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
}

fn ctas_fixture() -> TempDir {
    let dir = cli_fixture();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建两个库（断言语义不变）。
    for name in ["app", "other"] {
        let out = run_cli(dir.path(), &["new", name]);
        assert_eq!(out.code, Some(0), "new {name} failed: {}", out.stderr);
    }
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE t (k INT, name VARCHAR); \
         INSERT INTO t VALUES (1, 'a'); \
         INSERT INTO t VALUES (2, 'b'); \
         INSERT INTO t VALUES (3, 'c'); \
         CREATE TABLE tn (k INT, v VARCHAR); \
         INSERT INTO tn VALUES (1, NULL); \
         INSERT INTO tn VALUES (2, NULL); \
         CREATE TABLE ts2 (k INT, s VARCHAR); \
         INSERT INTO ts2 VALUES (1, 's'); \
         CREATE TABLE tf (k INT, v FLOAT); \
         INSERT INTO tf VALUES (1, 2.5); \
         CREATE TABLE td (k INT, d DATE, ts TIMESTAMP); \
         INSERT INTO td VALUES (1, DATE '2026-01-02', TIMESTAMP '2026-01-02 03:04:05'); \
         CREATE TABLE dup (k INT, id INT); \
         INSERT INTO dup VALUES (1, 1); \
         INSERT INTO dup VALUES (2, 1)",
        "seed main db",
    );
    run_cli_ok(
        dir.path(),
        "other",
        "CREATE TABLE u (id INT, tag VARCHAR); \
         INSERT INTO u VALUES (1, 'x'); \
         INSERT INTO u VALUES (9, 'y')",
        "seed attached db",
    );
    dir
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

/// `rtsql schema <db>` 的 CREATE TABLE DDL —— 物化列类型的观察面。
fn schema_ddl(dir: &Path, db: &str) -> String {
    let out = run_cli(dir, &["schema", db]);
    assert_eq!(out.code, Some(0), "schema {db} failed: {}", out.stderr);
    out.stdout
}

fn assert_ddl_contains(ddl: &str, needle: &str, what: &str) {
    assert!(
        ddl.contains(needle),
        "{what}: expected schema DDL containing {needle:?}, got {:?}",
        ddl
    );
}

/// 目标表不存在 —— 零副作用观察面（`schema` 逐用户表输出 DDL）。
fn assert_table_absent(dir: &Path, db: &str, table: &str, what: &str) {
    let ddl = schema_ddl(dir, db);
    assert!(
        !ddl.contains(&format!("\"{table}\"")),
        "{what}: table {table} should not exist, schema was {:?}",
        ddl
    );
}

// ---------------------------------------------------------------------------
// S1 — main-database materialization + restart read-back
// ---------------------------------------------------------------------------

/// S1：`CREATE TABLE m AS SELECT ... WHERE ...` 在主库物化；第二个 CLI 调用
/// 重开库后 `SELECT * FROM m` 一致（catalog + 数据页落盘常轨）。
#[test]
fn ctas_materializes_main_database_rows() {
    let dir = ctas_fixture();

    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k, name FROM t WHERE k > 1",
        "ctas materialize",
    );

    // 重开库读回（独立进程 → 独立 open/recovery 路径）。
    let out = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(
        &out,
        r#""rows":[[2,"b"],[3,"c"]]"#,
        "ctas restart read-back",
    );
}

/// S1 附：物化结果的输出列名取 SELECT 输出表头（裸列名），与源表列序一致。
#[test]
fn ctas_output_columns_follow_select_header() {
    let dir = ctas_fixture();

    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k, name FROM t",
        "ctas materialize all",
    );

    let ddl = schema_ddl(dir.path(), "app");
    assert_ddl_contains(
        &ddl,
        r#"CREATE TABLE "m" ("k" INTEGER PRIMARY KEY, "name" TEXT)"#,
        "ctas header",
    );
}

// ---------------------------------------------------------------------------
// S2 — attached-database source
// ---------------------------------------------------------------------------

/// S2：`CREATE TABLE m AS SELECT * FROM o.u` 物化附加库全部行到主库。
#[test]
fn ctas_from_attached_database_source() {
    let dir = ctas_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; CREATE TABLE m AS SELECT * FROM o.u",
        ],
    );
    assert_eq!(out.code, Some(0), "cross-db ctas failed: {}", out.stderr);

    let read = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(
        &read,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "cross-db materialized rows",
    );
}

/// S2 附：一次性物化 —— 附加库后续变化不影响主库 `m`。
#[test]
fn ctas_from_attached_database_is_one_time_snapshot() {
    let dir = ctas_fixture();

    run_cli_ok(
        dir.path(),
        "app",
        "ATTACH 'other' AS o; CREATE TABLE m AS SELECT * FROM o.u",
        "cross-db ctas",
    );
    run_cli_ok(
        dir.path(),
        "other",
        "INSERT INTO u VALUES (20, 'later')",
        "extend attached db",
    );

    let read = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(&read, r#""rows":[[1,"x"],[9,"y"]]"#, "snapshot unaffected");
}

// ---------------------------------------------------------------------------
// S3 — target already exists
// ---------------------------------------------------------------------------

/// S3：目标表已存在 → 点名拒绝且既有表零变化。
#[test]
fn ctas_target_already_exists_rejected() {
    let dir = ctas_fixture();
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m (k INT, name VARCHAR); INSERT INTO m VALUES (9, 'existing')",
        "seed existing target",
    );

    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE m AS SELECT k, name FROM t"],
    );
    assert_failure(&out, 3, "duplicate table", "ctas target exists");

    let read = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(&read, r#""rows":[[9,"existing"]]"#, "existing m unchanged");
}

// ---------------------------------------------------------------------------
// S4 — attached-database target rejected
// ---------------------------------------------------------------------------

/// S4：`CREATE TABLE o.m AS SELECT ...` 计划期点名拒绝（CTAS 仅写主库）。
#[test]
fn ctas_target_in_attached_database_rejected() {
    let dir = ctas_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; CREATE TABLE o.m AS SELECT id, tag FROM o.u",
        ],
    );
    assert_failure(
        &out,
        3,
        "WRITE TO ATTACHED DATABASE",
        "ctas target in attached db",
    );
    assert_table_absent(dir.path(), "other", "m", "attached target");
}

// ---------------------------------------------------------------------------
// S5 — expression and alias output shape
// ---------------------------------------------------------------------------

/// S5：`SELECT k + 1 AS inc` → 列名 `inc`，值为求值结果。
#[test]
fn ctas_expression_alias_defines_output_shape() {
    let dir = ctas_fixture();

    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k + 1 AS inc FROM t",
        "ctas expression alias",
    );

    let ddl = schema_ddl(dir.path(), "app");
    assert_ddl_contains(&ddl, r#""inc" INTEGER"#, "expression alias header");

    let out = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(&out, r#""rows":[[2],[3],[4]]"#, "expression alias values");
}

// ---------------------------------------------------------------------------
// S6 — type inference matrix
// ---------------------------------------------------------------------------

/// S6 类型推导矩阵：全 NULL → STRING；Int∪Float → FLOAT；同型保留（INT /
/// DATE / TIMESTAMP 不降级）。
#[test]
fn ctas_infers_column_types_from_output_values() {
    let dir = ctas_fixture();

    // 全 NULL 列 → STRING(255)
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m_null AS SELECT k, v FROM tn",
        "ctas all-null column",
    );
    // Int∪Float 混合列（UNION ALL 分支不校验列类型，是真实的混合来源）→ FLOAT
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m_mix AS SELECT k AS v FROM t UNION ALL SELECT v FROM tf",
        "ctas int/float mix",
    );
    // 同型保留：DATE / TIMESTAMP / INT
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m_typed AS SELECT k, d, ts FROM td",
        "ctas typed columns",
    );

    let ddl = schema_ddl(dir.path(), "app");
    assert_ddl_contains(
        &ddl,
        r#"CREATE TABLE "m_null" ("k" INTEGER PRIMARY KEY, "v" TEXT)"#,
        "all-null → STRING",
    );
    // change 2026-10-03-practical-core-gaps T3：非 INT 隐式首列不渲染
    // PRIMARY KEY（声明从未被强制，输出与语义一致）。
    assert_ddl_contains(
        &ddl,
        r#"CREATE TABLE "m_mix" ("v" FLOAT)"#,
        "int/float mix → FLOAT",
    );
    assert_ddl_contains(
        &ddl,
        r#"CREATE TABLE "m_typed" ("k" INTEGER PRIMARY KEY, "d" DATE, "ts" TIMESTAMP)"#,
        "typed preserved",
    );
}

/// S6：不可行混合（Int∪String）→ 建表前点名拒绝，零副作用。
#[test]
fn ctas_incompatible_type_mix_rejected() {
    let dir = ctas_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE m AS SELECT k AS v FROM t UNION ALL SELECT s FROM ts2",
        ],
    );
    assert_failure(&out, 3, "CTAS column type conflict", "incompatible mix");
    assert_table_absent(dir.path(), "app", "m", "incompatible mix");
}

/// S6：`SELECT k, k` 输出列名重复 → 建表前点名拒绝，零副作用。
#[test]
fn ctas_duplicate_output_column_names_rejected() {
    let dir = ctas_fixture();

    let out = run_cli(dir.path(), &["app", "CREATE TABLE m AS SELECT k, k FROM t"]);
    assert_failure(&out, 3, "DUPLICATE OUTPUT COLUMN", "duplicate output names");
    assert_table_absent(dir.path(), "app", "m", "duplicate output names");
}

/// S6 已知边界：空结果集没有可观测值 → 全部输出列落入「全 NULL → STRING(255)」
/// 规则（零行是「全 NULL」的退化形态），建表与空表形态正常。
#[test]
fn ctas_empty_source_creates_empty_string_typed_table() {
    let dir = ctas_fixture();

    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k, name FROM t WHERE k > 99",
        "ctas empty source",
    );

    let ddl = schema_ddl(dir.path(), "app");
    // change 2026-10-03-practical-core-gaps T3：非 INT 隐式首列不渲染
    // PRIMARY KEY（声明从未被强制，输出与语义一致）。
    assert_ddl_contains(
        &ddl,
        r#"CREATE TABLE "m" ("k" TEXT, "name" TEXT)"#,
        "empty source shape",
    );
    let out = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(&out, r#""rows":[]"#, "empty source rows");
}

// ---------------------------------------------------------------------------
// S7 — implicit primary key conflict
// ---------------------------------------------------------------------------

/// S7：首列隐式主键遇重复 Int → `Duplicate key` 点名失败；失败后 `m` 不存在，
/// 同名可重试。
#[test]
fn ctas_implicit_primary_key_conflict_leaves_no_table() {
    let dir = ctas_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE m AS SELECT id, k FROM dup"],
    );
    assert_failure(&out, 3, "Duplicate key", "implicit pk conflict");
    assert_table_absent(dir.path(), "app", "m", "pk conflict cleanup");

    // 同名可重试 —— 证明失败点没有留下不可回收的 catalog / 索引残留。
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k, name FROM t WHERE k = 1",
        "retry after pk conflict",
    );
    let read = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(&read, r#""rows":[[1,"a"]]"#, "retry materialized");
}

// ---------------------------------------------------------------------------
// S8 — source failure
// ---------------------------------------------------------------------------

/// S8：源表不存在 → 点名失败且主库无 `m`；同名可重试。
#[test]
fn ctas_source_failure_leaves_no_table() {
    let dir = ctas_fixture();

    let out = run_cli(dir.path(), &["app", "CREATE TABLE m AS SELECT * FROM nope"]);
    assert_failure(&out, 3, "'nope' not found", "ctas source failure");
    assert_table_absent(dir.path(), "app", "m", "source failure");

    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k, name FROM t WHERE k = 2",
        "retry after source failure",
    );
    let read = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(&read, r#""rows":[[2,"b"]]"#, "retry after source failure");
}

// ---------------------------------------------------------------------------
// S9 — explicit transaction
// ---------------------------------------------------------------------------

/// S9：会话事务活跃时 CTAS → 点名拒绝；事务收口后库可用、无残留 `m`。
#[test]
fn ctas_inside_explicit_transaction_rejected() {
    let dir = ctas_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "BEGIN; CREATE TABLE m AS SELECT k, name FROM t; SELECT 1",
        ],
    );
    assert_failure(&out, 3, "not supported inside a transaction", "ctas in tx");
    assert_table_absent(dir.path(), "app", "m", "in-tx rejection");

    // 库仍可用（拒绝未损坏会话/库状态）。
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE m AS SELECT k, name FROM t WHERE k = 3",
        "usable after in-tx rejection",
    );
    let read = run_cli(dir.path(), &["app", "SELECT * FROM m"]);
    assert_rows(
        &read,
        r#""rows":[[3,"c"]]"#,
        "materialized after in-tx rejection",
    );
}

// ---------------------------------------------------------------------------
// S10 — mixed definition / LIKE / CLONE
// ---------------------------------------------------------------------------

/// S10：列清单与 query 并存 → 计划期点名拒绝（不静默建空表）。
#[test]
fn ctas_mixed_definition_and_query_rejected() {
    let dir = ctas_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE m (a INT) AS SELECT k, name FROM t"],
    );
    assert_failure(&out, 3, "WITH COLUMN DEFINITIONS", "mixed definition form");
    assert_table_absent(dir.path(), "app", "m", "mixed definition form");
}

/// S10：`LIKE other` / `CLONE other` → 计划期点名拒绝（不静默忽略成分）。
#[test]
fn ctas_like_and_clone_rejected() {
    let dir = ctas_fixture();

    let like_out = run_cli(dir.path(), &["app", "CREATE TABLE m LIKE t"]);
    assert_failure(&like_out, 3, "CREATE TABLE ... LIKE", "ctas like form");
    assert_table_absent(dir.path(), "app", "m", "like form");

    let clone_out = run_cli(dir.path(), &["app", "CREATE TABLE m CLONE t"]);
    assert_failure(&clone_out, 3, "CREATE TABLE ... CLONE", "ctas clone form");
    assert_table_absent(dir.path(), "app", "m", "clone form");
}

// ---------------------------------------------------------------------------
// lib path — no attach registry
// ---------------------------------------------------------------------------

/// lib 无注册会话：两段名维持既有扁平名语义，CTAS 源不存在时点名失败。
#[tokio::test]
async fn lib_path_ctas_two_part_name_keeps_flat_semantics() {
    let dir = ctas_fixture();
    let db = Database::open(&dir.path().join("db").join("app.db"))
        .await
        .unwrap();

    let miss = to_response(db.execute_sql("CREATE TABLE m AS SELECT * FROM o.u").await);
    assert!(
        matches!(&miss, Response::Error { message, .. } if message.contains("o.u") && message.contains("not found")),
        "lib path must keep flat-name lookup for the CTAS source, got {miss:?}"
    );

    // 同库可用（失败未留下 catalog 残留）。
    let ok = to_response(
        db.execute_sql("CREATE TABLE m AS SELECT k, name FROM t WHERE k = 1")
            .await,
    );
    assert!(
        !matches!(ok, Response::Error { .. }),
        "lib retry after source failure: {ok:?}"
    );
}
