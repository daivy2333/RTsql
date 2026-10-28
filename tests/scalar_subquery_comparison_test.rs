//! T4（change 2026-10-04-external-review-gaps）— WHERE 比较腿标量子查询与
//! SELECT 投影位形状（R4，场景 S13–S18）。
//!
//! S13 非相关标量比较（=/>，任一侧）；S14 相关标量比较（逐行参数注入）；
//! S15 空子查询 → NULL 三值语义；S16 多行点名拒绝；S17 JOIN 基座维持点名
//! 拒绝 + 谓词不下推计划形状见证；S18 投影位输出形状与请求一致（未请求列
//! 不泄漏）。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::tempdir;

/// Helper: open an in-memory database.
async fn open_db() -> Database {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    Database::open(&path).await.unwrap()
}

/// Helper: execute SQL and return the Response.
async fn exec(db: &Database, sql: &str) -> Response {
    to_response(db.execute_sql(sql).await)
}

/// Helper: extract rows from a QueryResult, panicking if not QueryResult.
fn rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::AffectedRows { count } => {
            panic!("expected QueryResult, got AffectedRows({count})")
        }
        Response::Error { message, .. } => {
            panic!("expected QueryResult, got Error: {message}")
        }
        Response::Pong => panic!("expected QueryResult, got Pong"),
    }
}

/// Helper: extract error message from an Error response.
fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        Response::Pong => "Pong".to_string(),
        _ => panic!("expected Error, got non-error response"),
    }
}

/// Helper: emp fixture（与 subquery_test 同数据面，便于校准事实互认）。
async fn setup_emp(db: &Database) {
    exec(
        db,
        "CREATE TABLE emp (id INT, name TEXT, dept INT, salary INT)",
    )
    .await;
    exec(db, "INSERT INTO emp VALUES (1, 'Alice', 10, 50000)").await;
    exec(db, "INSERT INTO emp VALUES (2, 'Bob', 20, 60000)").await;
    exec(db, "INSERT INTO emp VALUES (3, 'Carol', 10, 55000)").await;
    exec(db, "INSERT INTO emp VALUES (4, 'Dave', 30, 45000)").await;
    exec(db, "INSERT INTO emp VALUES (5, 'Eve', 20, 65000)").await;
}

/// Helper: sort rows by first value (string) for order-insensitive comparison.
fn sorted_by_first(r: &[Vec<serde_json::Value>]) -> Vec<Vec<serde_json::Value>> {
    let mut v = r.to_vec();
    v.sort_by(|a, b| {
        a[0].as_str()
            .unwrap_or_default()
            .cmp(b[0].as_str().unwrap_or_default())
    });
    v
}

// === S13: 非相关标量比较 ===

/// S13-Eq：最高薪行。修复前：`Unsupported expression type: EXPRESSION` exit 3。
#[tokio::test]
async fn s13_noncorrelated_eq_comparison() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = sorted_by_first(&rows(
        exec(
            &db,
            "SELECT name FROM emp WHERE salary = (SELECT MAX(salary) FROM emp)",
        )
        .await,
    ));
    assert_eq!(r, vec![vec![serde_json::json!("Eve")]]);
}

/// S13-Gt：`>` 变体（高于平均薪 55000 → Bob/Eve）。
#[tokio::test]
async fn s13_noncorrelated_gt_comparison() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = sorted_by_first(&rows(
        exec(
            &db,
            "SELECT name FROM emp WHERE salary > (SELECT AVG(salary) FROM emp)",
        )
        .await,
    ));
    assert_eq!(
        r,
        vec![
            vec![serde_json::json!("Bob")],
            vec![serde_json::json!("Eve")]
        ]
    );
}

/// S13-反侧：标量子查询在比较左腿。
#[tokio::test]
async fn s13_subquery_on_left_leg() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = sorted_by_first(&rows(
        exec(
            &db,
            "SELECT name FROM emp WHERE (SELECT MAX(salary) FROM emp) = salary",
        )
        .await,
    ));
    assert_eq!(r, vec![vec![serde_json::json!("Eve")]]);
}

/// S13-AND 混合：标量腿与普通腿组合（递归检测 + 复合谓词编译）。
#[tokio::test]
async fn s13_scalar_leg_inside_and() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = sorted_by_first(&rows(
        exec(
            &db,
            "SELECT name FROM emp WHERE dept = 10 AND salary = \
             (SELECT MAX(salary) FROM emp WHERE dept = 10)",
        )
        .await,
    ));
    assert_eq!(r, vec![vec![serde_json::json!("Carol")]]);
}

// === S14: 相关标量比较 ===

/// S14：逐行按部门最高薪匹配（Carol/Eve/Dave 各为本部门最高薪）。
#[tokio::test]
async fn s14_correlated_comparison() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = sorted_by_first(&rows(
        exec(
            &db,
            "SELECT name FROM emp e WHERE e.salary = \
             (SELECT MAX(salary) FROM emp WHERE dept = e.dept)",
        )
        .await,
    ));
    assert_eq!(
        r,
        vec![
            vec![serde_json::json!("Carol")],
            vec![serde_json::json!("Dave")],
            vec![serde_json::json!("Eve")],
        ]
    );
}

/// S14-缓存等价：同参数值序列重复行结果一致（镜像 subquery_test 缓存见证
/// 风格——求值次数不可观察，行为一致性即见证）。
#[tokio::test]
async fn s14_duplicate_param_values_consistent() {
    let db = open_db().await;
    setup_emp(&db).await;
    let sql = "SELECT name FROM emp e WHERE e.salary = \
               (SELECT MAX(salary) FROM emp WHERE dept = e.dept)";
    let r1 = sorted_by_first(&rows(exec(&db, sql).await));
    let r2 = sorted_by_first(&rows(exec(&db, sql).await));
    assert_eq!(r1, r2);
    assert_eq!(r1.len(), 3);
}

// === S15: 空子查询 → NULL 三值语义 ===

/// S15：空子查询 → NULL，`v = NULL` 为 Unknown → 零行。
#[tokio::test]
async fn s15_empty_subquery_null_three_valued() {
    let db = open_db().await;
    exec(&db, "CREATE TABLE vt (v INT)").await;
    exec(&db, "INSERT INTO vt VALUES (1)").await;
    exec(&db, "INSERT INTO vt VALUES (2)").await;
    let r = rows(
        exec(
            &db,
            "SELECT v FROM vt WHERE v = (SELECT v FROM vt WHERE 1 = 0)",
        )
        .await,
    );
    assert_eq!(r.len(), 0);
}

// === S16: 多行点名拒绝 ===

/// S16：多行子查询 → `SubqueryReturnsMultipleRow` 既有错误面。
#[tokio::test]
async fn s16_multiple_rows_rejected() {
    let db = open_db().await;
    setup_emp(&db).await;
    let msg = error_msg(
        exec(
            &db,
            "SELECT name FROM emp WHERE salary = (SELECT salary FROM emp)",
        )
        .await,
    );
    assert!(
        msg.contains("Subquery returns multiple rows"),
        "unexpected error: {msg}"
    );
}

// === S17: JOIN 基座拒绝面 + 不下推计划形状 ===

/// S17：JOIN 基座 WHERE 内标量子查询维持点名拒绝（镜像 InSubquery/Exists）。
#[tokio::test]
async fn s17_join_where_scalar_subquery_rejected() {
    let db = open_db().await;
    setup_emp(&db).await;
    exec(&db, "CREATE TABLE dept (id INT, name TEXT, region TEXT)").await;
    exec(&db, "INSERT INTO dept VALUES (10, 'Engineering', 'East')").await;
    let msg = error_msg(
        exec(
            &db,
            "SELECT emp.name FROM emp JOIN dept ON emp.dept = dept.id \
             WHERE emp.salary = (SELECT MAX(salary) FROM emp)",
        )
        .await,
    );
    assert!(
        msg.contains("WHERE CLAUSE ON JOIN"),
        "unexpected error: {msg}"
    );
}

/// S17-不下推见证：单表标量谓词的计划为 ScalarSubqueryFilter 包 Scan，
/// 不是谓词下推的 DataScan。
#[tokio::test]
async fn s17_predicate_not_pushed_down() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = rows(
        exec(
            &db,
            "EXPLAIN SELECT name FROM emp WHERE salary = (SELECT MAX(salary) FROM emp)",
        )
        .await,
    );
    // 计划树每节点一行——拼全文断言（根 ScalarSubqueryFilter + 输入 Scan +
    // 绑定子查询子树）。
    let plan: Vec<&str> = r
        .iter()
        .map(|row| row[0].as_str().unwrap())
        .collect::<Vec<_>>();
    let plan_text = plan.join("\n");
    assert!(
        plan_text.contains("ScalarSubqueryFilter"),
        "expected ScalarSubqueryFilter in plan:\n{plan_text}"
    );
    // 谓词不下推见证：外层谓词不得进入任何 DataScan（绑定子查询自身的
    // DataScan 不携带谓词参数——`predicate=` 是下推标记）。
    assert!(
        !plan_text.contains("predicate="),
        "outer predicate must not be pushed into a DataScan:\n{plan_text}"
    );
}

// === S18: 投影位形状与请求一致 ===

/// S18：标量按 SELECT 位插入、未请求列不泄漏（修复前：全表列泄漏 + 位次
/// 插列 `__subquery`）。带 WHERE 的裁剪路径。
#[tokio::test]
async fn s18_projection_shape_with_where() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT name, (SELECT MAX(salary) FROM emp) FROM emp WHERE dept = 20",
        )
        .await,
    );
    assert_eq!(r.len(), 2);
    for row in &r {
        assert_eq!(row.len(), 2, "row must match SELECT shape: {row:?}");
        assert_eq!(row[1], serde_json::json!(65000));
    }
    let names: Vec<&str> = r.iter().map(|row| row[0].as_str().unwrap()).collect();
    assert!(names.contains(&"Bob") && names.contains(&"Eve"));
}

/// S18-别名：`expr AS alias` 输出名。
#[tokio::test]
async fn s18_projection_shape_aliased() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT name, (SELECT MAX(salary) FROM emp) AS top FROM emp WHERE dept = 30",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![vec![serde_json::json!("Dave"), serde_json::json!(65000)]]
    );
}

/// S18-无 WHERE + ORDER BY：Sort 承担裁剪后 SubqueryEval 插列（无 WHERE
/// 裸路径 + 排序键在裁剪之外仍可达）。
#[tokio::test]
async fn s18_projection_shape_order_by_trim() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT name, (SELECT COUNT(*) FROM emp) FROM emp ORDER BY id LIMIT 2",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![
            vec![serde_json::json!("Alice"), serde_json::json!(5)],
            vec![serde_json::json!("Bob"), serde_json::json!(5)],
        ]
    );
}

/// S18-混合投影既有面零回归：两个标量项按 SELECT 位插入。
#[tokio::test]
async fn s18_two_scalars_positioned() {
    let db = open_db().await;
    setup_emp(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT name, (SELECT COUNT(*) FROM emp) AS cnt, \
             (SELECT MAX(salary) FROM emp) AS max_sal FROM emp WHERE dept = 30",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![vec![
            serde_json::json!("Dave"),
            serde_json::json!(5),
            serde_json::json!(65000)
        ]]
    );
}

// === T03-D2: TEXT 相关参数的 owned 求值 ===

/// T03-D2（change 2026-10-20-ms30-semantic-binding-differential-net）：
/// TEXT 相关参数在 owned 求值通道上取真实值——`ParameterExpression::evaluate`
/// 未被覆盖，参数真值经 `evaluate_ref` 的 `ValueRef::Null` 丢失，相关子查询
/// 谓词恒 Unknown → 外层零行。
///
/// 差分白名单 Q5-05 最小复现形态：`s2.carrier = s.carrier`（TEXT 相关参数）。
/// ups 承运商两行（days 5 与 2）→ AVG 3.5；只有 days=2 的行满足 `days < 3.5`。
#[tokio::test]
async fn t03_d2_text_correlated_parameter_uses_real_value() {
    let db = open_db().await;
    exec(&db, "CREATE TABLE ship (id INT, carrier TEXT, days INT)").await;
    exec(&db, "INSERT INTO ship VALUES (1, 'ups', 5)").await;
    exec(&db, "INSERT INTO ship VALUES (2, 'dhl', 3)").await;
    exec(&db, "INSERT INTO ship VALUES (3, 'ups', 2)").await;

    let r = rows(
        exec(
            &db,
            "SELECT s.id FROM ship s WHERE s.days < \
             (SELECT AVG(s2.days) FROM ship s2 WHERE s2.carrier = s.carrier)",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![vec![serde_json::json!(3)]],
        "TEXT 相关参数应取真实值：ups 承运商 AVG=3.5，只有 days=2 的行满足条件"
    );
}

/// T03-D2 对照：TEXT 相关参数在计数型相关子查询中同样取真实值。
#[tokio::test]
async fn t03_d2_text_correlated_parameter_in_count_subquery() {
    let db = open_db().await;
    exec(&db, "CREATE TABLE ship (id INT, carrier TEXT, days INT)").await;
    exec(&db, "INSERT INTO ship VALUES (1, 'ups', 5)").await;
    exec(&db, "INSERT INTO ship VALUES (2, 'dhl', 3)").await;
    exec(&db, "INSERT INTO ship VALUES (3, 'ups', 2)").await;

    let mut r = rows(
        exec(
            &db,
            "SELECT s.id FROM ship s WHERE \
             (SELECT COUNT(*) FROM ship s2 WHERE s2.carrier = s.carrier) = 2",
        )
        .await,
    );
    r.sort_by_key(|row| row[0].as_i64().unwrap());
    assert_eq!(
        r,
        vec![vec![serde_json::json!(1)], vec![serde_json::json!(3)]],
        "两个 ups 行的同承运商计数为 2，dhl 行为 1"
    );
}
