//! MS25 Iteration 002 / Cycle 000：EXPLAIN 最小计划树（Task T6，design D6）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶：`build_plan` 无 `Statement::Explain` 臂，全形态落入
//! `UnsupportedStatement(unsupported_statement_name(stmt))`——agent 无法观察查询
//! 计划形状。
//!
//! 目标语义（delta spec `explain-plan-output`）：
//! - R1 `EXPLAIN <SELECT>` 输出单列（表头 `plan`）计划树文本（节点名 + 关键
//!   参数，树形缩进），**不执行**内层语句；计划期错误不豁免；经既有 CLI 四格式
//!   渲染面呈现。
//! - R2 非 SELECT 族语句（INSERT / DDL / 事务语句）与 `ANALYZE` / `VERBOSE` /
//!   `FORMAT` 修饰计划期点名拒绝；`EXPLAIN EXPLAIN` 拒绝；`DESC` / `DESCRIBE`
//!   与 EXPLAIN 同语义。
//! - R3 非 EXPLAIN 语句零回归（含 `explain` 作列名）；同语句两次执行输出一致。
//!
//! 计划形状：`PhysicalPlan::Explain(ExplainNode{input})` 仅包装内层计划；
//! pipeline 构造臂**不构造内层执行器**（DerivedScan 急切物化与任何执行器副作用
//! 都不触发）；渲染器 `render_plan_tree` 为逐变体纯函数（节点字段直出）。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use serde_json::json;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
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

/// 计划树文本行集——输出恒单列（I034 纪律：表头与行宽一致）。
fn plan_lines(rows: &[Vec<serde_json::Value>]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            assert_eq!(row.len(), 1, "EXPLAIN 输出恒单列 plan: {row:?}");
            row[0].as_str().expect("plan 行为文本").to_string()
        })
        .collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// R1-S1 种子：`t(id INT PRIMARY KEY, name STRING)` 含 (1,'Alice')、(2,'Bob')。
async fn seed(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, name STRING)").await;
    exec_ok(db, "INSERT INTO t VALUES (1, 'Alice')").await;
    exec_ok(db, "INSERT INTO t VALUES (2, 'Bob')").await;
}

/// R1-S1：简单查询计划树——扫描节点可见表名与谓词/投影参数，语句未执行。
#[tokio::test]
async fn explain_simple_tree() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let lines = plan_lines(&query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT name FROM t WHERE id = 1")
            .await,
    )));
    assert_eq!(lines[0], "EXPLAIN", "根行标识 EXPLAIN: {lines:?}");
    // `WHERE id = 1` 走既有 PK 等值路由：谓词以扫描节点的键参数呈现
    // （非等值谓词的 Filter 节点形态由 explain_compound_nodes 见证）。
    assert!(
        lines
            .iter()
            .any(|l| l.contains("IndexScan t") && l.contains("key=")),
        "计划树含 PK 扫描节点、表名与键参数: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("Alice")),
        "EXPLAIN 不输出数据行: {lines:?}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S2：复合查询节点参数——JOIN 类型、Filter 谓词、Sort 排序键、Limit 值。
#[tokio::test]
async fn explain_compound_nodes() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE l (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE r (rid INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO l VALUES (1), (2)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1), (2)").await;

    let lines = plan_lines(&query_rows(to_response(
        db.execute_sql(
            "EXPLAIN SELECT l.id FROM l JOIN r ON l.id = r.rid \
             WHERE l.id > 1 ORDER BY id LIMIT 5",
        )
        .await,
    )));
    assert_eq!(lines[0], "EXPLAIN", "{lines:?}");
    for (node, needle) in [
        ("Limit", "5"),
        ("Sort", "id"),
        ("Filter", "id"),
        ("Join", "l"),
    ] {
        assert!(
            lines
                .iter()
                .any(|l| l.trim_start().starts_with(node) && l.contains(needle)),
            "计划树缺 {node} 节点参数 {needle:?}: {lines:?}"
        );
    }
    // 树形缩进：EXPLAIN 根最浅，节点行逐层加深
    let root_indent = indent_of(&lines[0]);
    assert!(
        lines[1..].iter().all(|l| indent_of(l) > root_indent),
        "节点行应缩进于根行之下: {lines:?}"
    );

    // MS25 Iteration 000/001 交付的节点臂同样可渲染（外连接 / DISTINCT /
    // 集合操作）
    let outer = plan_lines(&query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT DISTINCT l.id FROM l LEFT JOIN r ON l.id > r.rid")
            .await,
    )));
    for node in ["Distinct", "NestedLoopJoin [Left]", "Scan l", "Scan r"] {
        assert!(
            outer.iter().any(|l| l.trim_start().starts_with(node)),
            "外连接 + DISTINCT 计划树缺 {node}: {outer:?}"
        );
    }

    let compound = plan_lines(&query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT id FROM l UNION SELECT rid FROM r")
            .await,
    )));
    for node in ["SetOperation [UNION]", "DataScan l", "DataScan r"] {
        assert!(
            compound.iter().any(|l| l.trim_start().starts_with(node)),
            "集合操作计划树缺 {node}: {compound:?}"
        );
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S3：EXPLAIN 不执行目标语句——无数据行输出，随后同库查询数据完好。
#[tokio::test]
async fn explain_no_execution() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let lines = plan_lines(&query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT * FROM t").await,
    )));
    assert!(
        lines.iter().any(|l| l.contains("t")),
        "计划树应含被观测的表: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("Alice") || l.contains("Bob") || l.contains('1')),
        "EXPLAIN 不产出数据行: {lines:?}"
    );

    let after = query_rows(to_response(db.execute_sql("SELECT id, name FROM t").await));
    assert_eq!(
        after,
        vec![vec![json!(1), json!("Alice")], vec![json!(2), json!("Bob")]],
        "EXPLAIN 不得改变库内数据"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S4：计划期错误不豁免——`EXPLAIN` 对缺失表按既有错误面拒绝。
#[tokio::test]
async fn explain_plan_error_not_exempt() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let msg = error_msg(to_response(
        db.execute_sql("EXPLAIN SELECT x FROM nosuch_table").await,
    ));
    assert!(
        msg.contains("nosuch_table"),
        "缺失表按既有错误面点名: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S1：DML 拒绝（计划期点名，零副作用）。
#[tokio::test]
async fn explain_dml_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    for sql in [
        "EXPLAIN INSERT INTO t VALUES (3, 'Carol')",
        "EXPLAIN UPDATE t SET name = 'x' WHERE id = 1",
        "EXPLAIN DELETE FROM t WHERE id = 1",
    ] {
        let msg = error_msg(to_response(db.execute_sql(sql).await));
        assert!(
            msg.contains("EXPLAIN of non-SELECT statements"),
            "{sql}: DML 点名拒绝, got {msg}"
        );
    }
    let rows = query_rows(to_response(db.execute_sql("SELECT COUNT(*) FROM t").await));
    assert_eq!(rows, vec![vec![json!(2)]], "拒绝路径零副作用");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S2：DDL 拒绝（计划期点名，零副作用——目标表未被创建）。
#[tokio::test]
async fn explain_ddl_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let msg = error_msg(to_response(
        db.execute_sql("EXPLAIN CREATE TABLE t2 (id INT)").await,
    ));
    assert!(
        msg.contains("EXPLAIN of non-SELECT statements"),
        "DDL 点名拒绝, got {msg}"
    );
    assert!(
        error_msg(to_response(db.execute_sql("SELECT * FROM t2").await)).contains("t2"),
        "被拒 DDL 零副作用（t2 不应存在）"
    );

    let msg = error_msg(to_response(db.execute_sql("EXPLAIN DROP TABLE t2").await));
    assert!(
        msg.contains("EXPLAIN of non-SELECT statements"),
        "DROP 点名拒绝, got {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2 附加：`ANALYZE` / `VERBOSE` / `FORMAT` 修饰与嵌套 EXPLAIN 点名拒绝；
/// `DESC` / `DESCRIBE` 与 EXPLAIN 同语义。
#[tokio::test]
async fn explain_modifiers_rejected_and_desc_accepted() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    for (sql, needle) in [
        ("EXPLAIN ANALYZE SELECT * FROM t", "EXPLAIN ANALYZE"),
        ("EXPLAIN VERBOSE SELECT * FROM t", "EXPLAIN VERBOSE"),
        ("EXPLAIN FORMAT json SELECT * FROM t", "EXPLAIN FORMAT"),
    ] {
        let msg = error_msg(to_response(db.execute_sql(sql).await));
        assert!(msg.contains(needle), "{sql}: 修饰点名拒绝, got {msg}");
    }

    // 嵌套 EXPLAIN 由 sqlparser 在解析期点名拒绝（`Explain must be root of
    // the plan`）；两条路径都以点名错误收敛，不静默接受。
    let msg = error_msg(to_response(
        db.execute_sql("EXPLAIN EXPLAIN SELECT * FROM t").await,
    ));
    assert!(
        msg.to_lowercase().contains("explain"),
        "嵌套 EXPLAIN 点名拒绝, got {msg}"
    );

    let explain = plan_lines(&query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT name FROM t").await,
    )));
    let desc = plan_lines(&query_rows(to_response(
        db.execute_sql("DESC SELECT name FROM t").await,
    )));
    assert_eq!(explain, desc, "DESC 与 EXPLAIN 同语义");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S1：非 EXPLAIN 语句零回归（含 `explain` 作列名的既有解析）。
#[tokio::test]
async fn non_explain_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;
    exec_ok(&db, "CREATE TABLE kw (explain INT, plain INT)").await;
    exec_ok(&db, "INSERT INTO kw VALUES (7, 8)").await;

    assert_eq!(
        query_rows(to_response(
            db.execute_sql("SELECT name FROM t WHERE id = 1").await
        )),
        vec![vec![json!("Alice")]],
        "既有查询结果不变"
    );
    assert_eq!(
        query_rows(to_response(
            db.execute_sql("SELECT explain, plain FROM kw").await
        )),
        vec![vec![json!(7), json!(8)]],
        "`explain` 作列名的既有解析不受影响"
    );
    let affected = to_response(db.execute_sql("INSERT INTO t VALUES (3, 'Carol')").await);
    assert!(
        matches!(affected, Response::AffectedRows { count: 1 }),
        "既有 DML 行为不变: {affected:?}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S2：同一 EXPLAIN 两次执行输出一致（无缓存、确定性渲染）。
#[tokio::test]
async fn explain_repeatable() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let first = query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT name FROM t").await,
    ));
    let second = query_rows(to_response(
        db.execute_sql("EXPLAIN SELECT name FROM t").await,
    ));
    assert_eq!(first, second, "EXPLAIN 输出确定性可重复");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1 输出经既有 CLI 四格式渲染面呈现（表头恒单列 `plan`）。
#[test]
fn explain_cli_four_formats() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    let run = |args: &[&str]| -> (Option<i32>, String, String) {
        let child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
            .args(args)
            .current_dir(dir.path())
            .env("RTSQL_HOME", dir.path())
            .env_remove("RTSQL_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rtsql binary");
        let out = child.wait_with_output().expect("wait rtsql");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    let seed_args: &[&str] = &["app", "CREATE TABLE t (id INT PRIMARY KEY, name STRING)"];

    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let (new_code, _, new_err) = run(&["new", "app"]);
    assert_eq!(new_code, Some(0), "new failed: {new_err}");

    let (code, _, err) = run(seed_args);
    assert_eq!(code, Some(0), "seed DDL failed: {err}");

    let (code, stdout, err) = run(&["app", "EXPLAIN SELECT name FROM t", "--format", "json"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json output");
    assert_eq!(
        parsed["columns"],
        serde_json::json!(["plan"]),
        "表头恒为 plan"
    );
    let rows = parsed["rows"].as_array().expect("rows array");
    assert!(
        rows.iter()
            .all(|r| r.as_array().map(|c| c.len()) == Some(1)),
        "行宽恒 1: {rows:?}"
    );

    for (format, needle) in [("table", "plan"), ("csv", "plan"), ("tsv", "plan")] {
        let (code, stdout, err) = run(&["app", "EXPLAIN SELECT name FROM t", "--format", format]);
        assert_eq!(code, Some(0), "{format} stderr: {err}");
        assert!(
            stdout.contains(needle),
            "{format} 输出含表头 plan: {stdout:?}"
        );
    }

    // 既有错误面零回归：EXPLAIN 的计划期错误走既有退出码通道（3 = SQL 错误）
    let (code, _, err) = run(&["app", "EXPLAIN INSERT INTO t VALUES (1, 'x')"]);
    assert_eq!(code, Some(3), "计划期错误退出 3: {err}");
}
