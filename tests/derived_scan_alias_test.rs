//! MS25 Iteration 004 / Cycle 001：派生表 / CTE 基座的输出别名应用收口（Task T10，design D9）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶（Plan 实测）：`alias_reroutable` 的基座条件是「仅 `Scan(_)`」，派生表与
//! CTE 引用（两者共用 `DerivedScan` 装配路径）不在其内——纯列引用别名项既不进
//! 顶层 Projection 别名面（表头退化为列名），`ORDER BY <别名>` 键又不在 Sort 的
//! 可见列集内而点名列不存在。
//!
//! 目标语义（delta spec `derived-scan-alias`）：
//! - R1 FROM 子查询基座的纯列别名项：表头 = 别名、行值 = 所指列。
//! - R2 CTE 基座同款形态（同一 DerivedScan 装配路径）。
//! - R3 别名排序可达（DESC）；无别名、限定名引用与既有拒绝面零回归。

use rtsql::database::Database;
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, rtsql::network::protocol::Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

async fn seed(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (v STRING, n INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b', 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a', 2)").await;
    db
}

fn rows_of(resp: rtsql::network::protocol::Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        rtsql::network::protocol::Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

// ==== R1：FROM 子查询基座 ====

/// R1-S1：派生表基座的纯列别名项表头 = 别名、行值 = 所指列。
///
/// RED（实施前实测）：表头为列名 `v`（别名被丢弃）。
#[tokio::test]
async fn derived_plain_column_alias_name() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT v AS val FROM (SELECT v FROM t)")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

/// R1-S2：限定名引用的派生表别名项。
#[tokio::test]
async fn derived_qualified_alias_name() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT d.v AS val FROM (SELECT v FROM t) AS d")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

// ==== R2：CTE 基座 ====

/// R2-S1：CTE 基座的纯列别名项（与 FROM 子查询共用 DerivedScan 装配路径）。
#[tokio::test]
async fn cte_plain_column_alias_name() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("WITH c AS (SELECT v FROM t) SELECT v AS val FROM c")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

/// R2-S2：CTE 基座的别名排序（DESC）真实生效。
///
/// RED（实施前实测）：`ORDER BY val` 计划期 `ColumnNotFound` 点名。
#[tokio::test]
async fn cte_alias_sort_desc() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("WITH c AS (SELECT v FROM t) SELECT v AS val FROM c ORDER BY val DESC")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

// ==== R3：既有面零回归 ====

/// R3-S1：无别名的派生表引用表头维持列名。
#[test]
fn derived_no_alias_header_unchanged() {
    let out = query_cli("SELECT v FROM (SELECT v FROM t)");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["v".to_string()],
        "no-alias derived reference must keep the column name header"
    );
}

/// R3-S2：别名项表头 = 别名（真二进制 table 渲染）。
#[test]
fn derived_alias_header_is_alias() {
    let out = query_cli("SELECT v AS val FROM (SELECT v FROM t)");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["val".to_string()],
        "alias item header must be the alias"
    );
}

/// R3-S3：表达式项 × 派生表维持既有可达语义（Iteration 000 值表达式面），
/// 表头 = 表达式文本——别名门放宽不改变该面。
#[tokio::test]
async fn derived_expression_item_unchanged() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT upper(v) FROM (SELECT v FROM t)")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("B")], vec![json!("A")]]);
    let out = query_cli("SELECT upper(v) FROM (SELECT v FROM t)");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["upper(v)".to_string()],
        "expression item header must stay the expression text"
    );
}

// ==== R5：派生基座的外层投影裁剪（MS26 T8，change 2026-09-30-ms26-embedded-runtime）====
//
// 病灶：投影解析的 `base_schema` 仅从 `PhysicalPlan::Scan` 提取，DerivedScan 落入
// identity 臂——纯列清单外层投影静默丢弃（行与表头均为派生全集）。三通道
// （FROM 子查询 / CTE 引用 / 视图展开）共用 `DerivedScan` 装配路径，同病。

fn error_msg(resp: rtsql::network::protocol::Response) -> String {
    match resp {
        rtsql::network::protocol::Response::Error { message, .. } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

/// R5-S1：派生表基座的纯列清单按外层清单裁剪行。
///
/// RED（实施前实测）：返回派生全集两列（`[["b",1],["a",2]]`）。
#[tokio::test]
async fn derived_pure_list_trims_rows() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT n FROM (SELECT v, n FROM t) AS d")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(1)], vec![json!(2)]]);
}

/// R5-S1：表头与行形状一致（真二进制 CLI 渲染）。
#[test]
fn derived_pure_list_trims_header() {
    let out = query_cli("SELECT n FROM (SELECT v, n FROM t) AS d");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["n".to_string()],
        "derived base pure list must trim the header to the outer list"
    );
}

/// R5-S2：CTE 通道与派生表形态同型。
#[tokio::test]
async fn cte_pure_list_trims_rows() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("WITH c AS (SELECT v, n FROM t) SELECT n FROM c")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(1)], vec![json!(2)]]);
}

/// R5-S2：视图通道与派生表形态同型（零通道特判）。
#[tokio::test]
async fn view_pure_list_trims_rows() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;
    exec_ok(&db, "CREATE VIEW vw AS SELECT v, n FROM t").await;

    let rows = rows_of(db.execute_sql("SELECT n FROM vw").await);
    assert_eq!(rows, vec![vec![json!(1)], vec![json!(2)]]);
}

/// R5-S3：外层未知列计划期 `ColumnNotFound` 点名（不静默返回派生全集）。
#[tokio::test]
async fn derived_unknown_column_named_rejected() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;
    exec_ok(&db, "CREATE VIEW vw AS SELECT v, n FROM t").await;

    for sql in [
        "SELECT zzz FROM (SELECT v, n FROM t) AS d",
        "WITH c AS (SELECT v, n FROM t) SELECT zzz FROM c",
        "SELECT zzz FROM vw",
    ] {
        let msg = error_msg(db.execute_sql(sql).await);
        assert!(
            msg.contains("Column not found") && msg.contains("zzz"),
            "unknown outer column over derived base must be named: {sql:?} -> {msg}"
        );
    }
}

/// R5-S4：WHERE 组合面——Filter 承载的裁剪（过滤后单列输出）。
#[tokio::test]
async fn derived_where_combination_trims() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT n FROM (SELECT v, n FROM t) AS d WHERE n = 2")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(2)]]);
}

/// R5-S4：ORDER BY 组合面——Sort 承载的裁剪（排序后单列输出）。
#[tokio::test]
async fn derived_order_by_combination_trims() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT v FROM (SELECT v, n FROM t) AS d ORDER BY v")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("a")], vec![json!("b")]]);

    let out = query_cli("SELECT v FROM (SELECT v, n FROM t) AS d ORDER BY v");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["v".to_string()],
        "Sort-owned trim must keep the header consistent with the row shape"
    );
}

/// R5-S4：CTAS 列名取裁剪后表头并物化对应值。
#[tokio::test]
async fn ctas_from_derived_trims_columns() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    exec_ok(
        &db,
        "CREATE TABLE cn AS SELECT n FROM (SELECT v, n FROM t) AS d",
    )
    .await;
    let rows = rows_of(db.execute_sql("SELECT n FROM cn").await);
    assert_eq!(rows, vec![vec![json!(1)], vec![json!(2)]]);

    let dir = fixture();
    let created = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE cn2 AS SELECT n FROM (SELECT v, n FROM t) AS d",
        ],
    );
    assert_eq!(created.code, Some(0), "ctas failed: {}", created.stderr);
    let listed = run_cli(dir.path(), &["app", "SELECT * FROM cn2"]);
    assert_eq!(listed.code, Some(0), "query failed: {}", listed.stderr);
    assert_eq!(
        listed.columns(),
        vec!["n".to_string()],
        "CTAS over derived base must materialize the trimmed header"
    );
}

/// R5-S5：`SELECT *` 恒等面不回归（两列）。
#[tokio::test]
async fn derived_wildcard_identity() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT * FROM (SELECT v, n FROM t) AS d")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("b"), json!(1)], vec![json!("a"), json!(2)]]
    );
}

/// R5-S5：聚合恒等面不回归（输入保持全形状）。
#[tokio::test]
async fn derived_count_identity() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT COUNT(*) FROM (SELECT v, n FROM t) AS d")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(2)]]);
}

/// R5-S6：嵌套派生的列序与注册一致——内层派生先裁剪，外层按清单重排裁剪。
#[tokio::test]
async fn nested_derived_column_order() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT n, v FROM (SELECT v, n FROM (SELECT v, n FROM t) AS d) AS e")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("b")], vec![json!(2), json!("a")]]
    );
}

// ==== R4：派生表/视图内层输出列外层可见（change 2026-10-13-e2e-verification-and-defect-closure T4，delta spec `join-output-shape-resolution` ADDED R4）====
//
// 病灶（pre-fix 实测）：派生表内层的 `col AS alias`、表达式列与 DISTINCT 被
// `building_subquery` 分类门抑制——外层按别名引用收 42703，DISTINCT 静默
// 跳过；CTE 通道（顶层构建）不受影响。修复把抑制条件统一为
// `building_subquery && !building_from_derived`（视图展开与 FROM 派生同型
// 置位，见 view_test.rs R4 组）。e2e 见证：`e11_inner_columns_outer_visible`
// 解除 ignore 由 RED 转 GREEN。

/// R4-S1：派生表内层别名列外层可引用。
#[tokio::test]
async fn r4_derived_inner_alias_outer_visible() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT val FROM (SELECT v AS val FROM t) d ORDER BY val")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("a")], vec![json!("b")]]);
}

/// R4-S2：派生表内层表达式（计算）列外层可引用。
#[tokio::test]
async fn r4_derived_inner_expression_outer_visible() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT up FROM (SELECT upper(v) AS up FROM t) d ORDER BY up")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("A")], vec![json!("B")]]);
}

/// R4-S3：派生表内层 DISTINCT 真实去重（修复前静默跳过）。
#[tokio::test]
async fn r4_derived_inner_distinct_dedups() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;
    exec_ok(&db, "CREATE TABLE t2 (v STRING)").await;
    for v in ["x", "y", "x"] {
        exec_ok(&db, &format!("INSERT INTO t2 VALUES ('{v}')")).await;
    }

    let rows = rows_of(
        db.execute_sql("SELECT d FROM (SELECT DISTINCT v AS d FROM t2) s ORDER BY d")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("x")], vec![json!("y")]]);
}

/// R4-S4：三通道一致——同一内层定义经派生表 / 视图 / CTE 输出逐字节一致。
#[tokio::test]
async fn r4_three_channels_identical() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;
    exec_ok(
        &db,
        "CREATE VIEW vw AS SELECT v AS val, upper(v) AS up FROM t",
    )
    .await;

    let expected = vec![vec![json!("a"), json!("A")], vec![json!("b"), json!("B")]];
    let derived = rows_of(
        db.execute_sql(
            "SELECT val, up FROM (SELECT v AS val, upper(v) AS up FROM t) d ORDER BY val",
        )
        .await,
    );
    let view = rows_of(db.execute_sql("SELECT val, up FROM vw ORDER BY val").await);
    let cte = rows_of(
        db.execute_sql(
            "WITH c AS (SELECT v AS val, upper(v) AS up FROM t) SELECT val, up FROM c ORDER BY val",
        )
        .await,
    );
    assert_eq!(derived, expected);
    assert_eq!(
        view, expected,
        "view channel must match derived byte-for-byte"
    );
    assert_eq!(
        cte, expected,
        "cte channel must match derived byte-for-byte"
    );
}

/// R4-S5：IN/EXISTS/标量上下文内层表达式项维持既有拒绝（文案逐字节不变）。
#[tokio::test]
async fn r4_expression_items_still_rejected_in_expression_subqueries() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    for sql in [
        "SELECT n FROM t WHERE n IN (SELECT upper(v) FROM t)",
        "SELECT n FROM t WHERE n IN (SELECT upper(v) AS u FROM t)",
        "SELECT n FROM t WHERE EXISTS (SELECT upper(v) FROM t)",
        "SELECT n FROM t WHERE n = (SELECT upper(v) FROM t)",
    ] {
        let msg = error_msg(db.execute_sql(sql).await);
        assert!(
            msg.contains("Invalid aggregate argument: Expected column name"),
            "expression subquery rejection surface must be unchanged: {sql:?} -> {msg}"
        );
    }
}

// ==== CLI 夹具（表头断言需要真二进制 JSON 列名）====

fn query_cli(sql: &str) -> CliOutput {
    let dir = fixture();
    run_cli(dir.path(), &["app", sql])
}

fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(new_out.code, Some(0), "new app failed: {}", new_out.stderr);
    for sql in [
        "CREATE TABLE t (v STRING, n INT)",
        "INSERT INTO t VALUES ('b', 1)",
        "INSERT INTO t VALUES ('a', 2)",
    ] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_eq!(out.code, Some(0), "setup {sql:?} failed: {}", out.stderr);
    }
    dir
}

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CliOutput {
    /// JSON 渲染的列名清单（CLI 默认格式）。
    fn columns(&self) -> Vec<String> {
        let parsed: serde_json::Value = serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {:?}", self.stdout));
        parsed["columns"]
            .as_array()
            .expect("columns array")
            .iter()
            .map(|c| c.as_str().expect("column name").to_string())
            .collect()
    }
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
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

// ==== T03-D3：聚合派生表的外层列引用 ====

/// T03-D3（change 2026-10-20-ms30-semantic-binding-differential-net）：
/// 聚合派生表的 direct 形态（无顶层 Projection 包装）下，注册列取自
/// `AggregateNode::output_columns`——分组键位是**规范名**（`tag`）而非
/// SELECT 清单的别名（`n`），别名在派生表列源丢失 → 外层引用 `d.n` 计划期
/// 拒绝（`Column not found: n`）。
///
/// RED（实施前实测）：计划期拒绝。
#[tokio::test]
async fn t03_d3_aggregate_derived_alias_key_visible() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE orders (id INT, tag STRING)").await;
    exec_ok(&db, "INSERT INTO orders VALUES (1, 'alpha')").await;
    exec_ok(&db, "INSERT INTO orders VALUES (2, 'alpha')").await;
    exec_ok(&db, "INSERT INTO orders VALUES (3, 'beta')").await;

    let mut rows = rows_of(
        db.execute_sql(
            "SELECT d.n, d.c FROM (SELECT tag AS n, COUNT(*) AS c FROM orders GROUP BY tag) d",
        )
        .await,
    );
    rows.sort_by_key(|row| row[0].as_str().unwrap().to_string());
    assert_eq!(
        rows,
        vec![
            vec![json!("alpha"), json!(2)],
            vec![json!("beta"), json!(1)]
        ],
        "聚合派生表的键位应按 SELECT 别名可见"
    );
}

/// T03-D3 对照：无别名的键位维持规范名（`tag`），聚合位维持显示名。
#[tokio::test]
async fn t03_d3_aggregate_derived_plain_key_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE orders (id INT, tag STRING)").await;
    exec_ok(&db, "INSERT INTO orders VALUES (1, 'alpha')").await;
    exec_ok(&db, "INSERT INTO orders VALUES (2, 'alpha')").await;
    exec_ok(&db, "INSERT INTO orders VALUES (3, 'beta')").await;

    let mut rows = rows_of(
        db.execute_sql(
            "SELECT d.tag, d.c FROM (SELECT tag, COUNT(*) AS c FROM orders GROUP BY tag) d",
        )
        .await,
    );
    rows.sort_by_key(|row| row[0].as_str().unwrap().to_string());
    assert_eq!(
        rows,
        vec![
            vec![json!("alpha"), json!(2)],
            vec![json!("beta"), json!(1)]
        ],
        "无别名键位维持规范名"
    );
}

/// T03-D3 对照：同内容 CTE 形态维持既有行为（键位别名可见）。
#[tokio::test]
async fn t03_d3_aggregate_cte_alias_key_visible() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE orders (id INT, tag STRING)").await;
    exec_ok(&db, "INSERT INTO orders VALUES (1, 'alpha')").await;
    exec_ok(&db, "INSERT INTO orders VALUES (2, 'alpha')").await;
    exec_ok(&db, "INSERT INTO orders VALUES (3, 'beta')").await;

    let mut rows = rows_of(
        db.execute_sql(
            "WITH g AS (SELECT tag AS n, COUNT(*) AS c FROM orders GROUP BY tag) \
             SELECT g.n, g.c FROM g",
        )
        .await,
    );
    rows.sort_by_key(|row| row[0].as_str().unwrap().to_string());
    assert_eq!(
        rows,
        vec![
            vec![json!("alpha"), json!(2)],
            vec![json!("beta"), json!(1)]
        ],
        "CTE 形态的聚合键位别名可见"
    );
}
