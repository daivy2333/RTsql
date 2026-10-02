//! Integration tests for MS20 T2/T3 (`cross-db-namespace-query` R1–R3):
//! two-segment qualified-name routing, read-only cross-database query
//! combinations, and the attached-database write rejection surface.
//!
//! Fixture shape (two databases, one CLI call seeds each):
//!   main `app`      t(id,name) = (1,'a'),(2,'b')   — bare name stays main
//!                   "foo.bar"(id) = 42             — literal dotted name
//!                   "o.u"(id) = 5                  — wrong-target decoy
//!   attached `other`  u(id,tag) = (1,'x'),(9,'y')  — cross-db source
//!                   t(id,tag) = (77,'attached')    — same-name decoy
//! Scenario mapping (requirement → scenario → test):
//!   R1/S1 限定名命中附加库                 qualified_name_routes_to_attached_database
//!   R1/S2 裸名恒主库                      bare_name_always_resolves_to_main_database
//!   R1/S3 未注册别名回落扁平名             unregistered_alias_falls_back_to_flat_name
//!   R1/S4 含点扁平表名零回归               literal_dotted_table_name_zero_regression
//!   R1/S5 三段且首段命中别名拒绝           three_segment_name_with_registered_alias_rejected
//!   R2/S1 跨库 JOIN（INNER/OUTER）         cross_database_join_combines_rows
//!                                           cross_database_outer_join_matches_single_database_semantics
//!   R2/S2 跨库子查询与聚合                 cross_database_subquery_and_aggregate
//!   R2    三段列引用                       three_part_column_reference_resolves_on_routed_table
//!   R2    ORDER BY 别名 / DISTINCT / CTE   distinct_cte_and_order_by_alias_combine_across_databases
//!   R2    集合操作                         set_operation_combines_main_and_attached_rows
//!   R2    多附加库自由组合                  two_attached_databases_combine_with_main
//!   R3/S1 附加库 DML 路由（MS20-T02）        attached_database_dml_routes_to_attached
//!   R3/S2 附加库 DDL 拒绝                  attached_database_ddl_rejected_at_plan_time
//!   R3    零副作用 / 错误目标               write_rejection_leaves_both_databases_untouched
//!   R3    主库写面零回归                    main_database_writes_still_work_with_attached_registry
//!   lib   无注册会话零回归                  lib_path_two_part_name_keeps_flat_semantics
//!   (plan cache 跳过由 src/pipeline.rs 单元测试见证——CLI 无 cache 读路径)

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （夹具模式与 tests/attach_session_test.rs 一致）。
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

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

fn run_cli_ok(dir: &Path, db: &str, sql: &str, what: &str) {
    let out = run_cli(dir, &[db, sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
}

/// 主库 `app`（t / "foo.bar" / "o.u"）+ 附加库候选 `other`（u / t），
/// 另建 `third`（v）供多附加库场景使用。
fn cross_db_fixture() -> TempDir {
    let dir = cli_fixture();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建三个库（断言语义不变）。
    for name in ["app", "other", "third"] {
        let out = run_cli(dir.path(), &["new", name]);
        assert_eq!(out.code, Some(0), "new {name} failed: {}", out.stderr);
    }
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE t (id INT, name VARCHAR); \
         INSERT INTO t VALUES (1, 'a'); \
         INSERT INTO t VALUES (2, 'b'); \
         CREATE TABLE \"foo.bar\" (id INT); \
         INSERT INTO \"foo.bar\" VALUES (42); \
         CREATE TABLE \"o.u\" (id INT); \
         INSERT INTO \"o.u\" VALUES (5)",
        "seed main db",
    );
    run_cli_ok(
        dir.path(),
        "other",
        "CREATE TABLE u (id INT, tag VARCHAR); \
         INSERT INTO u VALUES (1, 'x'); \
         INSERT INTO u VALUES (9, 'y'); \
         CREATE TABLE t (id INT, tag VARCHAR); \
         INSERT INTO t VALUES (77, 'attached')",
        "seed attached db",
    );
    run_cli_ok(
        dir.path(),
        "third",
        "CREATE TABLE v (id INT, label VARCHAR); \
         INSERT INTO v VALUES (1, 'p1')",
        "seed second attached db",
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

// ---------------------------------------------------------------------------
// R1 — two-segment qualified-name routing
// ---------------------------------------------------------------------------

/// R1/S1（兼 `sql-attach-session` R1/S1 读取可观察量）：`ATTACH 'other' AS o`
/// 后 `SELECT * FROM o.u` 返回附加库 `u` 的行。
#[test]
fn qualified_name_routes_to_attached_database() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; SELECT * FROM o.u"],
    );

    assert_rows(&out, r#""rows":[[1,"x"],[9,"y"]]"#, "qualified name hit");
}

/// R1/S2：主库与附加库各有 `t`，裸名恒解析主库。
#[test]
fn bare_name_always_resolves_to_main_database() {
    let dir = cross_db_fixture();

    let out = run_cli(dir.path(), &["app", "ATTACH 'other' AS o; SELECT * FROM t"]);

    assert_rows(&out, r#""rows":[[1,"a"],[2,"b"]]"#, "bare name stays main");
}

/// R1/S3：未注册别名 `z` 回落扁平名 `z.t` 查主库 catalog → `TableNotFound`
/// 点名（不静默按主库 `t` 处理）。
#[test]
fn unregistered_alias_falls_back_to_flat_name() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; SELECT * FROM z.t"],
    );

    assert_failure(&out, 3, "'z.t' not found", "unregistered alias fallback");
}

/// R1/S4：主库字面表名 `foo.bar` 行为逐字节不变（会话未注册别名 `foo`）。
#[test]
fn literal_dotted_table_name_zero_regression() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; SELECT * FROM foo.bar"],
    );

    assert_rows(&out, r#""rows":[[42]]"#, "literal dotted table name");
}

/// R1/S5：三段限定名且首段命中别名 → 计划期点名拒绝。
#[test]
fn three_segment_name_with_registered_alias_rejected() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; SELECT * FROM o.b.t"],
    );

    assert_failure(&out, 3, "DEEPER THAN alias.table", "three-segment name");
}

// ---------------------------------------------------------------------------
// R2 — read-only cross-database combinations
// ---------------------------------------------------------------------------

/// R2/S1：跨库 INNER JOIN 组合主库与附加库列。
#[test]
fn cross_database_join_combines_rows() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT t.name, o.u.tag FROM t JOIN o.u ON t.id = o.u.id",
        ],
    );

    assert_rows(&out, r#""rows":[["a","x"]]"#, "cross-db inner join");
}

/// R2/S1：OUTER JOIN 跨库形态与单库语义一致（未匹配行补 NULL）。
#[test]
fn cross_database_outer_join_matches_single_database_semantics() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT t.name, o.u.tag FROM t LEFT JOIN o.u ON t.id = o.u.id",
        ],
    );

    assert_rows(
        &out,
        r#""rows":[["a","x"],["b",null]]"#,
        "cross-db left join",
    );
}

/// R2/S2：跨库子查询 + 聚合，按单库同形态语义返回。
#[test]
fn cross_database_subquery_and_aggregate() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT count(*) FROM o.u WHERE id IN (SELECT id FROM t)",
        ],
    );

    assert_rows(&out, r#""rows":[[1]]"#, "cross-db subquery + aggregate");
}

/// R2：三段列引用 `alias.table.col` 在路由命中时可用（投影 + WHERE）。
#[test]
fn three_part_column_reference_resolves_on_routed_table() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT o.u.id FROM o.u WHERE o.u.id = 9",
        ],
    );

    assert_rows(&out, r#""rows":[[9]]"#, "three-part column reference");
}

/// R2：DISTINCT / CTE / ORDER BY 输出别名三者在跨库源上组合。
#[test]
fn distinct_cte_and_order_by_alias_combine_across_databases() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; WITH c AS (SELECT id, tag FROM o.u) SELECT DISTINCT tag AS label FROM c ORDER BY label",
        ],
    );

    assert_rows(
        &out,
        r#""rows":[["x"],["y"]]"#,
        "distinct + cte + order by alias",
    );
}

/// R2：集合操作跨主库与附加库组合。
#[test]
fn set_operation_combines_main_and_attached_rows() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT id FROM t UNION ALL SELECT id FROM o.u",
        ],
    );

    assert_rows(
        &out,
        r#""rows":[[1],[2],[1],[9]]"#,
        "union all across databases",
    );
}

/// R2：主库 + 两个附加库自由组合（两个两表 JOIN 语句；链式三表 JOIN 在单库
/// 形态下同样受限，故不在此处断言）。
#[test]
fn two_attached_databases_combine_with_main() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; ATTACH 'third' AS p; \
             SELECT t.name, o.u.tag FROM t JOIN o.u ON t.id = o.u.id",
        ],
    );
    assert_rows(&out, r#""rows":[["a","x"]]"#, "main + attached join");

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; ATTACH 'third' AS p; \
             SELECT o.u.tag, p.v.label FROM o.u JOIN p.v ON o.u.id = p.v.id",
        ],
    );
    assert_rows(&out, r#""rows":[["x","p1"]]"#, "attached + attached join");
}

// ---------------------------------------------------------------------------
// R3 — attached-database read-only boundary
// ---------------------------------------------------------------------------

/// R3/S1（MS20-T02 迁移）：INSERT / UPDATE / DELETE 指向附加库 → 路由成功，
/// 仅修改附加库；主库同名 decoy 不变（原一期「计划期拒绝」断言随能力交付
/// 改为成功路由见证，见 `cross-db-dml` R1）。
#[test]
fn attached_database_dml_routes_to_attached() {
    let dir = cross_db_fixture();

    for sql in [
        "ATTACH 'other' AS o; INSERT INTO o.u VALUES (5, 'z')",
        "ATTACH 'other' AS o; UPDATE o.u SET tag = 'q' WHERE id = 1",
        "ATTACH 'other' AS o; DELETE FROM o.u WHERE id = 9",
    ] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_eq!(
            out.code,
            Some(0),
            "attached DML must route successfully: {}",
            out.stderr
        );
    }

    let attached = run_cli(dir.path(), &["other", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &attached,
        r#""rows":[[1,"q"],[5,"z"]]"#,
        "attached rows after routed DML",
    );
    let main_decoy = run_cli(dir.path(), &["app", "SELECT * FROM \"o.u\""]);
    assert_rows(&main_decoy, r#""rows":[[5]]"#, "main decoy untouched");
}

/// R3/S2：`CREATE TABLE o.newt` / `DROP TABLE o.u` 计划期点名拒绝。
#[test]
fn attached_database_ddl_rejected_at_plan_time() {
    let dir = cross_db_fixture();

    for sql in [
        "ATTACH 'other' AS o; CREATE TABLE o.newt (a INT)",
        "ATTACH 'other' AS o; DROP TABLE o.u",
    ] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_failure(
            &out,
            3,
            "attached databases are read-only",
            "attached DDL rejection",
        );
    }
}

/// R3：拒绝零副作用——附加库内容与 schema 不变，且主库同名 `"o.u"` 表未被
/// 静默写入（错误目标面）。MS20-T02 起附加 DML 已可写，此见证改用仍拒绝的
/// 附加 DDL 目标（`cross-db-dml` R4）。
#[test]
fn write_rejection_leaves_both_databases_untouched() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; CREATE TABLE o.newt (a INT)"],
    );
    assert_failure(
        &out,
        3,
        "attached databases are read-only",
        "rejected DDL write",
    );

    let attached = run_cli(dir.path(), &["other", "SELECT * FROM u"]);
    assert_rows(
        &attached,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "attached rows intact",
    );

    let main_decoy = run_cli(dir.path(), &["app", "SELECT * FROM \"o.u\""]);
    assert_rows(&main_decoy, r#""rows":[[5]]"#, "main decoy table untouched");

    let created = run_cli(dir.path(), &["other", "SELECT * FROM newt"]);
    assert_failure(&created, 3, "not found", "no table created in attached db");
}

/// R3 Preserve：主库写面在会话持有注册表时行为零变化。
#[test]
fn main_database_writes_still_work_with_attached_registry() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; INSERT INTO t VALUES (3, 'c')"],
    );
    assert_eq!(out.code, Some(0), "main write failed: {}", out.stderr);

    let read = run_cli(dir.path(), &["app", "SELECT * FROM t"]);
    assert_rows(
        &read,
        r#""rows":[[1,"a"],[2,"b"],[3,"c"]]"#,
        "main write applied",
    );
}

// ---------------------------------------------------------------------------
// R9（MS20-T02 Iteration 002 / T7）—— 两段附加 FROM 项的隐式短名限定引用
// ---------------------------------------------------------------------------

/// R9/S1：JOIN 右臂两段附加项无显式别名时，尾段短名限定列在 SELECT/ON/WHERE/
/// GROUP BY/ORDER BY 全子句面可达（计划形状与全限定 `o.u.tag` 等价）。
#[test]
fn attached_join_short_name_reaches_all_clauses() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; \
             SELECT u.tag FROM t JOIN o.u ON t.id = u.id \
             WHERE u.tag = 'x' ORDER BY u.tag",
        ],
    );
    assert_rows(
        &out,
        r#""rows":[["x"]]"#,
        "join short name select/where/order",
    );

    let grouped = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; \
             SELECT u.tag, COUNT(*) FROM t JOIN o.u ON t.id = u.id GROUP BY u.tag",
        ],
    );
    assert_rows(
        &grouped,
        r#""rows":[["x",1]]"#,
        "join short name group/aggregate",
    );
}

/// R9/S2：单表两段附加项的短名限定列可达（`SELECT u.tag FROM o.u`）。
#[test]
fn attached_single_table_short_name_qualifier() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT u.tag FROM o.u WHERE u.tag = 'y'",
        ],
    );
    assert_rows(&out, r#""rows":[["y"]]"#, "single-table short name");
}

/// R9/S3：显式别名优先——`JOIN o.u x` 后 `x.tag` 可达，短名 `u.tag` 不登记、
/// 维持既有 `TableNotFound` 点名面。
#[test]
fn attached_short_name_explicit_alias_priority() {
    let dir = cross_db_fixture();

    let aliased = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT x.tag FROM t JOIN o.u x ON t.id = x.id",
        ],
    );
    assert_rows(
        &aliased,
        r#""rows":[["x"]]"#,
        "explicit alias keeps working",
    );

    let short = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'other' AS o; SELECT u.tag FROM t JOIN o.u x ON t.id = x.id",
        ],
    );
    assert_failure(
        &short,
        3,
        "Table not found: u",
        "short name stays unregistered with explicit alias",
    );
}

/// R9/S4：字面含点主库表名不带附加标记——`FROM "o.u"`（单段引号名）的短名
/// 形态仍点名，不误登记隐式关联名。
#[test]
fn attached_short_name_literal_dotted_not_registered() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; SELECT u.id FROM \"o.u\""],
    );
    assert_failure(
        &out,
        3,
        "Table not found: u",
        "literal dotted main-db table must not register the short name",
    );
}

// ---------------------------------------------------------------------------
// lib 路径零回归（无注册会话）
// ---------------------------------------------------------------------------

/// lib 执行路径没有 attach 注册表：两段名按既有扁平名语义处理，含点字面表名
/// 可达、未注册别名点名 `TableNotFound`。
#[tokio::test]
async fn lib_path_two_part_name_keeps_flat_semantics() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();

    for sql in [
        "CREATE TABLE \"foo.bar\" (id INT)",
        "INSERT INTO \"foo.bar\" VALUES (42)",
    ] {
        let resp = db.execute_sql(sql).await;
        assert!(
            !matches!(resp, Response::Error { .. }),
            "setup failed for {sql}: {resp:?}"
        );
    }

    let hit = db.execute_sql("SELECT * FROM foo.bar").await;
    match hit {
        Response::QueryResult { rows } => {
            assert_eq!(rows, vec![vec![serde_json::json!(42)]]);
        }
        other => panic!("expected rows from the literal dotted table, got {other:?}"),
    }

    let miss = db.execute_sql("SELECT * FROM z.t").await;
    assert!(
        matches!(&miss, Response::Error { message, .. } if message.contains("z.t") && message.contains("not found")),
        "unregistered two-part name must stay TableNotFound, got {miss:?}"
    );
}
