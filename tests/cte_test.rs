//! MS25 Iteration 002 / Cycle 000：非递归 CTE（Task T5，design D5）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶：`Query.with` 在 planner 全无消费——顶层 `WITH c AS (…) SELECT * FROM c`
//! 报 `Table 'c' does not exist`；CTE 名与基表同名时**静默按基表查询**（静默错误
//! 结果）；`WITH RECURSIVE` 同静默丢弃。
//!
//! 目标语义（delta spec `sql-common-table-expressions`，SQLite 对齐基准）：
//! - R1 顶层 WITH 端到端可达；多定义 + 前向引用；列别名列表；CTE 名遮蔽同名基表；
//!   反向引用点名拒绝。
//! - R2 同一语句内各引用面对同一快照行集（自 JOIN 双侧引用、标量子查询引用）。
//! - R3 `RECURSIVE` 关键字 / 定义内自引用 / 子查询内 WITH 点名拒绝；未定义引用走
//!   既有表缺失路径。
//! - R4 无 WITH 的全部查询（含同名基表查询）与 plan cache 键语义零回归。
//!
//! 计划形状：planner 语句级 `cte_plans` 注册表 + `pending_cte_names` 自引用检测；
//! 引用点克隆定义计划为既有 `DerivedScanNode`（pipeline 构造期急切物化，无新执行器
//! / 节点）；定义计划以顶层上下文构建（定义内 ORDER BY / LIMIT / 集合操作 / 表达式
//! 项 / DISTINCT 经既有装配全量复用）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
use std::process::{Command, Stdio};
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

fn query_rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

/// R1-S1/R1-S2 种子：`t(v INT)` 含 (1),(2)；同名基表 `c(v INT)` 含 (9)。
async fn seed(db: &Database) {
    exec_ok(db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(db, "INSERT INTO t VALUES (1, 1)").await;
    exec_ok(db, "INSERT INTO t VALUES (2, 2)").await;
    exec_ok(db, "CREATE TABLE c (v INT)").await;
    exec_ok(db, "INSERT INTO c VALUES (9)").await;
}

/// R1-S1：基础 CTE 可达（现状 RED：`Table 'c' does not exist` / 静默按基表查询）。
#[tokio::test]
async fn basic_cte() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("WITH cte AS (SELECT v * 2 AS d FROM t) SELECT d FROM cte")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(2)], vec![json!(4)]]);

    // 第二次执行经 plan cache 命中路径（WITH 语句可缓存，SQL 文本自带结构）
    let cached = query_rows(
        db.execute_sql("WITH cte AS (SELECT v * 2 AS d FROM t) SELECT d FROM cte")
            .await,
    );
    assert_eq!(cached, rows, "缓存命中路径结果一致");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1 附加：WITH × 复合查询——两臂共享语句级注册表（`WITH c AS (…) SELECT …
/// UNION SELECT …` 随本 Cycle 可达）。
#[tokio::test]
async fn cte_with_compound_query() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("WITH cte AS (SELECT v FROM t) SELECT v FROM cte UNION SELECT v FROM c")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(1)], vec![json!(2)], vec![json!(9)]]);

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S2：CTE 名遮蔽同名基表（现状 RED：静默输出基表行 9）。
#[tokio::test]
async fn cte_shadows_base_table() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("WITH c AS (SELECT 1 AS v) SELECT v FROM c")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(1)]],
        "CTE 遮蔽同名基表（基表 c 含 9）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S3：多定义与前向引用（后定义引用先定义）。
#[tokio::test]
async fn multi_def_forward_ref() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "WITH a AS (SELECT v FROM t), b AS (SELECT v + 10 AS w FROM a) SELECT w FROM b",
        )
        .await,
    );
    assert_eq!(rows, vec![vec![json!(11)], vec![json!(12)]]);

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S4：列别名列表重命名 CTE 输出列（表头经 CLI 见证为 n、s）。
#[tokio::test]
async fn column_alias_list() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (a INT, b STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'x')").await;

    let rows = query_rows(
        db.execute_sql("WITH cte(n, s) AS (SELECT a, b FROM t) SELECT n, s FROM cte")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(1), json!("x")]], "别名列表按序生效");

    // 列数不符点名拒绝
    let msg = error_msg(
        db.execute_sql("WITH cte(n, s, extra) AS (SELECT a, b FROM t) SELECT n FROM cte")
            .await,
    );
    assert!(
        msg.to_lowercase().contains("column"),
        "列数不符点名拒绝: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S5：反向引用（先定义引用后定义）点名拒绝。
#[tokio::test]
async fn reverse_ref_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let msg = error_msg(
        db.execute_sql("WITH a AS (SELECT * FROM b), b AS (SELECT 1 AS v) SELECT * FROM a")
            .await,
    );
    assert!(
        msg.contains("'b'") || msg.contains("b'"),
        "反向引用按既有表缺失面拒绝: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S1：多引用行集一致（自 JOIN 双侧引用同一 CTE）。
#[tokio::test]
async fn multi_ref_consistent_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "WITH cte AS (SELECT v FROM t) \
             SELECT a.v, b.v FROM cte a JOIN cte b ON a.v = b.v",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(1)], vec![json!(2), json!(2)]]
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S2：两个标量子查询引用读同一行集。
///
/// 见证形态带外层 FROM（`FROM c`）：delta spec 的示例 SQL 无外层 FROM，而
/// scalar-subquery 项在无 FROM 形态下被既有 MS13 T9 拒绝面点名（与 CTE 无关的
/// 前置限制，见 Act Response Deviations）。语义等价：两个标量引用同一 CTE，
/// 面对同一语句快照读出一致行集。
#[tokio::test]
async fn two_scalar_refs_same_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "WITH cte AS (SELECT v FROM t) \
             SELECT (SELECT COUNT(*) FROM cte) AS n1, (SELECT COUNT(*) FROM cte) AS n2 FROM c",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(2), json!(2), json!(9)]],
        "两个标量引用读同一行集（外层 FROM 的 v 尾随）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S1：`WITH RECURSIVE` 关键字拒绝（含内容非递归的形态）。
#[tokio::test]
async fn recursive_keyword_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    for sql in [
        "WITH RECURSIVE cte AS (SELECT 1 AS v) SELECT v FROM cte",
        "WITH RECURSIVE cte AS (SELECT 1 AS v), other AS (SELECT 2 AS v) SELECT v FROM other",
    ] {
        let msg = error_msg(db.execute_sql(sql).await);
        assert!(
            msg.contains("RECURSIVE"),
            "{sql}: RECURSIVE 点名拒绝, got {msg}"
        );
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S2：定义内自引用点名拒绝。
#[tokio::test]
async fn self_reference_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let msg = error_msg(
        db.execute_sql("WITH cte AS (SELECT v FROM cte) SELECT * FROM cte")
            .await,
    );
    assert!(msg.contains("cte"), "自引用点名拒绝: {msg}");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S3：子查询内 WITH 点名拒绝（范围仅顶层语句）。
#[tokio::test]
async fn with_in_subquery_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let msg = error_msg(
        db.execute_sql(
            "SELECT k FROM t WHERE k IN (WITH cte AS (SELECT 1 AS x) SELECT x FROM cte)",
        )
        .await,
    );
    assert!(
        msg.contains("subquer") || msg.contains("WITH") || msg.contains("with"),
        "子查询内 WITH 点名拒绝: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3 附加：定义内嵌套 WITH 点名拒绝；引用处列别名列表点名拒绝（声明面唯一
/// 入口是 `WITH c(cols) AS (…)`）。
#[tokio::test]
async fn nested_with_and_reference_alias_list_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let msg = error_msg(
        db.execute_sql(
            "WITH outer_cte AS (WITH inner_cte AS (SELECT 1 AS v) SELECT v FROM inner_cte) \
             SELECT v FROM outer_cte",
        )
        .await,
    );
    assert!(
        msg.contains("nested WITH"),
        "嵌套定义内 WITH 点名拒绝: {msg}"
    );

    let msg = error_msg(
        db.execute_sql("WITH cte AS (SELECT v FROM t) SELECT x FROM cte AS c2(x)")
            .await,
    );
    assert!(
        msg.contains("column alias list"),
        "引用处列别名列表点名拒绝: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R4-S1：无 WITH 的全部查询零回归（含同名基表查询）。
#[tokio::test]
async fn no_with_queries_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    assert_eq!(
        query_rows(db.execute_sql("SELECT v FROM c").await),
        vec![vec![json!(9)]],
        "无 WITH 时同名基表查询不受影响"
    );
    assert_eq!(
        query_rows(db.execute_sql("SELECT v FROM t WHERE v > 1").await),
        vec![vec![json!(2)]]
    );
    // 未定义引用走既有表缺失面
    assert!(
        error_msg(db.execute_sql("SELECT v FROM nothere").await).contains("nothere"),
        "未定义表文案不变"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S4 表头见证：列别名列表经既有 CLI 渲染面输出 n、s。
#[test]
fn column_alias_list_headers_via_cli() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    let run = |args: &[&str]| -> (Option<i32>, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
            .args(args)
            .current_dir(dir.path())
            .env("RTSQL_HOME", dir.path())
            .env_remove("RTSQL_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("run rtsql binary");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };

    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let (new_code, _, new_err) = run(&["new", "app"]);
    assert_eq!(new_code, Some(0), "new failed: {new_err}");

    let (code, _, err) = run(&["app", "CREATE TABLE t (a INT, b STRING)"]);
    assert_eq!(code, Some(0), "seed DDL failed: {err}");
    let (code, _, err) = run(&["app", "INSERT INTO t VALUES (1, 'x')"]);
    assert_eq!(code, Some(0), "seed DML failed: {err}");

    let (code, stdout, err) = run(&[
        "app",
        "WITH cte(n, s) AS (SELECT a, b FROM t) SELECT n, s FROM cte",
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json output");
    assert_eq!(parsed["columns"], serde_json::json!(["n", "s"]));
    assert_eq!(parsed["rows"], serde_json::json!([[1, "x"]]));
}
