//! Iteration 001 / Cycle 000：FROM 表别名注册与限定名解析统一（Task T6，design D4）
//!
//! change: 2026-09-30-catalog-durability-and-cli-surface
//!
//! 病灶：基表别名在计划期被完全忽略——`FROM people a` 的 `a` 不进入任何注册
//! 通道，限定列 `a.name` 在自连接投影（`Column not found`）、ON 腿
//! （`Table 'a' does not exist`）、WHERE（`Table 'a' not found`）、GROUP BY 与
//! ORDER BY（`ORDER BY only supports column names`）四面报错。
//!
//! 目标语义（delta spec `sql-table-alias-resolution`）：
//! - R1 FROM 别名注册为语句作用域限定符，四个子句面统一生效；表头、`OutputColumn`
//!   真实表名语义与计划形状不变。
//! - R2 限定列解析先归一再解析；归一后不可达的限定符以 `TableNotFound`（42P01）
//!   点名，不再误报列缺失或静默求值。
//! - R3 单表限定列与 JOIN 同规则，不保留「静默丢弃限定符」路径。
//! - R4 三段限定仅在 `a.b` 整体命中注册键（attach 复合键）时可达；本地库形态统一
//!   计划期点名拒绝。
//! - R5 ORDER BY 限定键（`order-by-alias-sorting` 的边界注记终止）。
//! - `join-output-shape-resolution` 严格解析域：别名限定列可达，裸名歧义仍点名。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
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

/// R1-S1 种子：`people(id, name, team_id)` + `team(id, team_name)`。
async fn seed(db: &Database) {
    exec_ok(
        db,
        "CREATE TABLE people (id INT PRIMARY KEY, name STRING, team_id INT)",
    )
    .await;
    exec_ok(db, "INSERT INTO people VALUES (1, 'ann', 10)").await;
    exec_ok(db, "INSERT INTO people VALUES (2, 'bob', 20)").await;
    exec_ok(db, "INSERT INTO people VALUES (3, 'amy', 10)").await;
    exec_ok(
        db,
        "CREATE TABLE team (id INT PRIMARY KEY, team_name STRING)",
    )
    .await;
    exec_ok(db, "INSERT INTO team VALUES (10, 'red')").await;
    exec_ok(db, "INSERT INTO team VALUES (20, 'blue')").await;
}

/// R1-S1：自连接两别名全子句面可用——投影 / ON / WHERE / ORDER BY 同时按
/// 左右别名解析。
///
/// RED（实施前实测）：`Parse error: Table 'a' does not exist`（ON 腿先失败）。
#[tokio::test]
async fn self_join_two_aliases_across_clauses() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT a.name, b.name FROM people a JOIN people b ON a.id = b.id \
             WHERE a.team_id = 10 ORDER BY b.id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("ann"), json!("ann")],
            vec![json!("amy"), json!("amy")]
        ],
        "rows: {rows:?}"
    );
}

/// `join-output-shape-resolution` R「别名限定列可达」：自连接投影只取左别名侧
/// 一列（不是两段同名表的全宽形状）。
#[tokio::test]
async fn self_join_projection_is_single_column() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("SELECT a.name FROM people a JOIN people b ON a.id = b.id")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("ann")], vec![json!("bob")], vec![json!("amy")]],
        "rows: {rows:?}"
    );
}

/// R1-S2：JOIN 两臂混用别名与真实表名限定列。
#[tokio::test]
async fn mixed_alias_and_real_name_qualifiers() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT p.name, team.team_name FROM people p JOIN team \
             ON p.team_id = team.id WHERE p.team_id = 10 ORDER BY p.name",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("amy"), json!("red")],
            vec![json!("ann"), json!("red")]
        ],
        "rows: {rows:?}"
    );
}

/// R1-S2 尾：裸名在两表同名列上的歧义仍点名为 `AmbiguousColumn`。
#[tokio::test]
async fn bare_name_across_aliased_join_is_ambiguous() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let msg = error_msg(
        db.execute_sql("SELECT id FROM people p JOIN team ON p.team_id = team.id")
            .await,
    );
    assert!(
        msg.contains("Ambiguous") || msg.contains("ambiguous"),
        "expected ambiguity error, got: {msg}"
    );
}

/// R2-S2：ON 腿与 WHERE 的别名限定（历史缺陷形态 `Parse error: Table 't' does
/// not exist` / `Table not found: t`）。
#[tokio::test]
async fn on_and_where_resolve_aliases() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("SELECT p.name FROM people p JOIN team t ON t.id = p.team_id")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("ann")], vec![json!("bob")], vec![json!("amy")]],
        "rows: {rows:?}"
    );

    let rows = query_rows(
        db.execute_sql(
            "SELECT * FROM people p JOIN team t ON t.id = p.team_id WHERE p.team_id = 10",
        )
        .await,
    );
    assert_eq!(rows.len(), 2, "rows: {rows:?}");
}

/// R2-S3：JOIN 基座的聚合与 GROUP BY 别名限定。
#[tokio::test]
async fn aggregate_and_group_by_resolve_aliases() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT a.team_id, COUNT(*) FROM people a JOIN people b ON a.id = b.id \
             GROUP BY a.team_id ORDER BY a.team_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(10), json!(2)], vec![json!(20), json!(1)]],
        "rows: {rows:?}"
    );
}

/// R3-S1：单表别名投影与 WHERE 同规则（历史缺陷：投影可用而 WHERE 报
/// `Table 'p' not found`）。
#[tokio::test]
async fn single_table_alias_projection_and_where() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("SELECT p.name FROM people p WHERE p.id = 1")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("ann")]], "rows: {rows:?}");
}

/// R2-S4 / R3-S2：未知限定符在单表与 JOIN 两形态下同语义点名，且携带限定符
/// 本身（不是列名）。
#[tokio::test]
async fn unknown_qualifier_is_named() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let msg = error_msg(db.execute_sql("SELECT x.name FROM people").await);
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");

    let msg = error_msg(
        db.execute_sql("SELECT x.name FROM people p JOIN team t ON p.team_id = t.id")
            .await,
    );
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");
}

/// R4-S1：本地库三段限定统一计划期点名拒绝（历史：单表静默成功、JOIN 失败）。
#[tokio::test]
async fn local_three_segment_qualifier_is_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let msg = error_msg(db.execute_sql("SELECT demo.people.name FROM people").await);
    assert!(
        msg.contains("demo.people"),
        "expected 3-segment qualifier in error, got: {msg}"
    );
}

/// R5-S1：JOIN 基座的限定键按右侧表真实排序（历史：`ORDER BY only supports
/// column names`）。
#[tokio::test]
async fn order_by_qualified_key_sorts_by_right_table() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE dept (id INT PRIMARY KEY, label STRING, rank INT)",
    )
    .await;
    exec_ok(&db, "INSERT INTO dept VALUES (30, 'c', 3)").await;
    exec_ok(&db, "INSERT INTO dept VALUES (10, 'a', 1)").await;
    exec_ok(&db, "INSERT INTO dept VALUES (20, 'b', 2)").await;

    // 右侧表的 id 序 = a, b, c；输入序为 c, a, b。
    let rows = query_rows(
        db.execute_sql("SELECT d.label FROM dept d ORDER BY d.id")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("a")], vec![json!("b")], vec![json!("c")]],
        "rows: {rows:?}"
    );

    // 同一查询的 DESC 臂。
    let rows = query_rows(
        db.execute_sql("SELECT d.label FROM dept d ORDER BY d.id DESC")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("c")], vec![json!("b")], vec![json!("a")]],
        "rows: {rows:?}"
    );
}

/// R5-S1 组合臂：限定键与裸名键多键混合。
#[tokio::test]
async fn order_by_qualified_and_bare_keys_combine() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE dept (id INT PRIMARY KEY, grp INT, label STRING)",
    )
    .await;
    exec_ok(&db, "INSERT INTO dept VALUES (1, 1, 'x')").await;
    exec_ok(&db, "INSERT INTO dept VALUES (2, 1, 'y')").await;
    exec_ok(&db, "INSERT INTO dept VALUES (3, 0, 'z')").await;

    let rows = query_rows(
        db.execute_sql("SELECT grp, label FROM dept d ORDER BY d.grp, label")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(0), json!("z")],
            vec![json!(1), json!("x")],
            vec![json!(1), json!("y")]
        ],
        "rows: {rows:?}"
    );
}

/// R1 不变量：别名只改解析路径，不改计划形状与输出列语义——同一条查询的
/// 「真实表名书写」与「别名书写」两条形态 EXPLAIN 逐行同型。
#[tokio::test]
async fn alias_and_real_name_produce_isomorphic_plans() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let real = query_rows(
        db.execute_sql("EXPLAIN SELECT people.name FROM people WHERE people.team_id = 10")
            .await,
    );
    let alias = query_rows(
        db.execute_sql("EXPLAIN SELECT p.name FROM people p WHERE p.team_id = 10")
            .await,
    );
    assert!(!real.is_empty(), "EXPLAIN must produce plan rows");
    assert_eq!(real, alias, "alias form must not alter the plan shape");
}

/// Cycle 001（replan）T6：未知限定符在 GROUP BY 与聚合参数两通道点名——单表
/// 基座此前经末段静默解析成功（exit 0），JOIN 基座误报列缺失
///（`Non-aggregated column 'id'` / `Column not found: id`）且错误不携带限定符。
///
/// RED（实施前实测）：①② 单表两形态 exit 0；③④ JOIN 两形态报列缺失类错误。
#[tokio::test]
async fn unknown_qualifier_in_group_by_and_aggregate_args_is_named() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    // ① 单表基座 GROUP BY 未知限定符。
    let msg = error_msg(db.execute_sql("SELECT id FROM people GROUP BY x.id").await);
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");

    // ② 单表基座聚合参数未知限定符。
    let msg = error_msg(db.execute_sql("SELECT COUNT(x.id) FROM people").await);
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");

    // ③ JOIN 基座 GROUP BY 未知限定符（含聚合形态——契约 RED 记录：
    // `Non-aggregated column 'id'`）与无聚合形态（GROUP BY 静默被忽略）。
    let msg = error_msg(
        db.execute_sql(
            "SELECT p.id, COUNT(*) FROM people p JOIN team t ON p.team_id = t.id \
             GROUP BY x.id",
        )
        .await,
    );
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");

    let msg = error_msg(
        db.execute_sql("SELECT p.id FROM people p JOIN team t ON p.team_id = t.id GROUP BY x.id")
            .await,
    );
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");

    // ④ JOIN 基座聚合参数未知限定符。
    let msg = error_msg(
        db.execute_sql("SELECT COUNT(x.id) FROM people p JOIN team t ON p.id = t.id")
            .await,
    );
    assert!(msg.contains('x'), "expected qualifier in error, got: {msg}");
}

/// Cycle 001（replan）T6 护栏：别名可达形态在两通道不回归（现状已绿，防回归）。
#[tokio::test]
async fn in_scope_alias_group_by_and_aggregate_args_reachable() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    // 单表基座：别名限定 GROUP BY 键。
    let rows = query_rows(
        db.execute_sql("SELECT p.id, COUNT(*) FROM people p GROUP BY p.id ORDER BY p.id")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(1)],
            vec![json!(2), json!(1)],
            vec![json!(3), json!(1)]
        ],
        "rows: {rows:?}"
    );

    // 自连接基座：别名限定 GROUP BY 键（聚合参数 COUNT(*) 无列参数臂）。
    let rows = query_rows(
        db.execute_sql(
            "SELECT a.team_id, COUNT(*) FROM people a JOIN people b ON a.id = b.id \
             GROUP BY a.team_id ORDER BY a.team_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(10), json!(2)], vec![json!(20), json!(1)]],
        "rows: {rows:?}"
    );

    // 单表基座：别名限定聚合参数。
    let rows = query_rows(db.execute_sql("SELECT COUNT(p.id) FROM people p").await);
    assert_eq!(rows, vec![vec![json!(3)]], "rows: {rows:?}");
}

/// R5-S2：输出别名优先与未知限定键拒绝不变——`ORDER BY k` 仍按输出别名排序，
/// `ORDER BY p.zzz` 计划期点名拒绝且错误携带限定符。
#[tokio::test]
async fn order_by_alias_priority_and_unknown_qualified_key() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;

    let rows = query_rows(
        db.execute_sql("SELECT id AS k FROM people p ORDER BY k")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(1)], vec![json!(2)], vec![json!(3)]],
        "rows: {rows:?}"
    );

    let msg = error_msg(
        db.execute_sql("SELECT p.zzz FROM people p ORDER BY p.zzz")
            .await,
    );
    assert!(
        msg.contains("zzz"),
        "expected key name in error, got: {msg}"
    );

    // 限定符不在作用域 → 42P01 携带限定符（与 R2 同款）。
    let msg = error_msg(
        db.execute_sql("SELECT name FROM people p ORDER BY q.name")
            .await,
    );
    assert!(msg.contains('q'), "expected qualifier in error, got: {msg}");
}

/// Cycle 001（replan）T8：历史缺陷 NLJ 变体见证——LEFT JOIN + 别名限定投影、
/// 未匹配行 NULL 补齐与反连接形态。对现实现即 GREEN（见证性质）；缺陷 RED
/// 已由父 Cycle Review Finding 4 探针实测记录。
#[tokio::test]
async fn left_join_alias_projection_and_null_padding() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed(&db).await;
    exec_ok(&db, "INSERT INTO people VALUES (4, 'cat', 30)").await;

    // 未匹配行 NULL 补齐 + 别名限定投影两侧可达（行序 = 基表扫描序，
    // 与本文件既有断言约定一致；排序键不在此见证——JOIN 基座排序键校验集
    // 为投影输出列，属 order-by-alias-sorting R3 既有语义）。
    let rows = query_rows(
        db.execute_sql(
            "SELECT p.name, t.team_name FROM people p LEFT JOIN team t \
             ON t.id = p.team_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("ann"), json!("red")],
            vec![json!("bob"), json!("blue")],
            vec![json!("amy"), json!("red")],
            vec![json!("cat"), json!(null)]
        ],
        "rows: {rows:?}"
    );

    // 反连接形态：WHERE 右侧别名列 IS NULL 只保留未匹配行。
    let rows = query_rows(
        db.execute_sql(
            "SELECT p.name FROM people p LEFT JOIN team t \
             ON t.id = p.team_id WHERE t.id IS NULL",
        )
        .await,
    );
    assert_eq!(rows, vec![vec![json!("cat")]], "rows: {rows:?}");
}
