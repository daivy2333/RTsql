//! Integration tests: 子查询内层 FROM **声明限定符**（别名）参与内外层引用分类
//! （delta spec `sql-table-alias-resolution` R1，change
//! `2026-10-15-subquery-alias-binding-and-error-prefix` T1）。
//!
//! 形状覆盖矩阵：算子 {EXISTS, NOT EXISTS, IN, NOT IN, WHERE 标量, 投影位标量}
//! × 内层形态 {单表别名, 单表别名+JOIN 臂别名, 派生表别名}
//! × 谓词列 {首列, 非首列} × 相关性 {非相关, 相关经外层 PK, 相关经外层非 PK}
//! × IN 面 {无索引, 二级索引谓词}。每形状期望按真实语义推导，不取「碰巧正确」。
//!
//! 覆盖缺口归因（change design.md）：既有 57 个子查询用例内层表零别名，全部落在
//! 「碰巧正确」区——本文件是缺口修复的载体。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::tempdir;

async fn open_db() -> Database {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    Database::open(&path).await.unwrap()
}

async fn exec(db: &Database, sql: &str) -> Response {
    to_response(db.execute_sql(sql).await)
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

fn rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("expected QueryResult, got {other:?}"),
    }
}

/// 首列整型值，升序——顺序无关的集合断言。
fn int_ids(resp: Response) -> Vec<i64> {
    let mut ids: Vec<i64> = rows(resp)
        .iter()
        .map(|row| row[0].as_i64().expect("首列为整型"))
        .collect();
    ids.sort();
    ids
}

/// 第二列文本值，按首列升序配对——投影位标量形状的行内容断言。
fn id_text_pairs(resp: Response) -> Vec<(i64, String)> {
    let mut pairs: Vec<(i64, String)> = rows(resp)
        .iter()
        .map(|row| {
            (
                row[0].as_i64().expect("首列为整型"),
                row[1].as_str().expect("次列为文本").to_string(),
            )
        })
        .collect();
    pairs.sort();
    pairs
}

/// 计划树文本行集——输出恒单列。
fn plan_lines(resp: Response) -> Vec<String> {
    rows(resp)
        .iter()
        .map(|row| {
            assert_eq!(row.len(), 1, "EXPLAIN 输出恒单列 plan: {row:?}");
            row[0].as_str().expect("plan 行为文本").to_string()
        })
        .collect()
}

/// 种子 A：外层 `a(id)` 两行；内层 `t(id PK, name)` 含 (1,'Lin')、(2,'Bo')。
async fn seed_pairs(db: &Database) {
    exec_ok(db, "CREATE TABLE a (id INT PRIMARY KEY)").await;
    exec_ok(db, "INSERT INTO a VALUES (1)").await;
    exec_ok(db, "INSERT INTO a VALUES (2)").await;
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, name TEXT)").await;
    exec_ok(db, "INSERT INTO t VALUES (1, 'Lin')").await;
    exec_ok(db, "INSERT INTO t VALUES (2, 'Bo')").await;
}

/// 种子 B：内层 `c(id PK, age INT)` + `age` 二级索引，另有 JOIN 臂表 `u`。
async fn seed_index_and_join(db: &Database) {
    exec_ok(db, "CREATE TABLE a (id INT PRIMARY KEY)").await;
    exec_ok(db, "INSERT INTO a VALUES (1)").await;
    exec_ok(db, "INSERT INTO a VALUES (2)").await;
    exec_ok(db, "CREATE TABLE c (id INT PRIMARY KEY, age INT)").await;
    exec_ok(db, "INSERT INTO c VALUES (1, 41)").await;
    exec_ok(db, "INSERT INTO c VALUES (2, 30)").await;
    exec_ok(db, "CREATE INDEX idx_c_age ON c(age)").await;
    exec_ok(db, "CREATE TABLE u (id INT PRIMARY KEY, tag TEXT)").await;
    exec_ok(db, "INSERT INTO u VALUES (1, 'p')").await;
    exec_ok(db, "INSERT INTO u VALUES (2, 'q')").await;
}

// === 算子 EXISTS：内层别名 × 谓词列位置 ===

/// R1-S1：内层别名谓词落在非首列——谓词按内层行求值，EXISTS 对全部外层行为真。
#[tokio::test]
async fn exists_alias_predicate_on_non_first_column() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "修复前恒空集");
}

/// R1-S2：内层别名谓词落在首列——EXISTS 判定只关乎内层行存在性，不按外层截断。
#[tokio::test]
async fn exists_alias_predicate_on_first_column() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = 1)",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "修复前仅返回外层第一行");
}

/// 零回归锚：无别名内层的基名限定形状结果不变。
#[tokio::test]
async fn exists_without_alias_zero_regression() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t WHERE t.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2]);
}

// === 算子 NOT EXISTS：审计新增形状 ===

/// 审计新增形状：NOT EXISTS 内层别名——内层有匹配行时全部外层行被滤除。
#[tokio::test]
async fn not_exists_alias_predicate_matching_row_filters_all() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE NOT EXISTS (SELECT 1 FROM t x WHERE x.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, Vec::<i64>::new(), "修复前全部外层行放行");
}

/// 审计新增形状：NOT EXISTS 内层别名——内层无匹配行时全部外层行保留。
#[tokio::test]
async fn not_exists_alias_predicate_without_match_keeps_all() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE NOT EXISTS (SELECT 1 FROM t x WHERE x.name = 'Zed')",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "修复前静默丢行");
}

// === 算子 IN / NOT IN：内层别名 × 谓词列 × 相关性 ===

/// IN 内层别名谓词落在非首列（无索引）。
#[tokio::test]
async fn in_alias_predicate_on_non_first_column() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE a.id IN (SELECT x.id FROM t x WHERE x.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, vec![1]);
}

/// NOT IN 内层别名——审计新增形状：补集按内层真实结果求差。
#[tokio::test]
async fn not_in_alias_predicate_complements_inner_result() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE a.id NOT IN (SELECT x.id FROM t x WHERE x.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, vec![2], "修复前全部放行");
}

/// R1-S4 / ISS18：IN 内层别名 ∧ 谓词由二级索引服务——别名与索引两个特性的交叉面。
#[tokio::test]
async fn in_alias_with_secondary_index_predicate() {
    let db = open_db().await;
    seed_index_and_join(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE a.id IN (SELECT x.id FROM c x WHERE x.age = 41)",
        )
        .await,
    );
    assert_eq!(got, vec![1], "修复前恒空集");
}

/// 零回归锚：同形状内层无别名时结果不变。
#[tokio::test]
async fn in_without_alias_zero_regression() {
    let db = open_db().await;
    seed_index_and_join(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE a.id IN (SELECT id FROM c WHERE age = 41)",
        )
        .await,
    );
    assert_eq!(got, vec![1]);
}

/// 相关性维度：IN 内层别名经外层主键关联。
#[tokio::test]
async fn in_alias_correlated_via_outer_primary_key() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE a.id IN (SELECT x.id FROM t x WHERE x.id = a.id)",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2]);
}

// === 相关性维度：EXISTS 经外层主键 / 非主键 ===

/// 相关 EXISTS 经外层主键列——真实相关参数正常注入。
#[tokio::test]
async fn exists_alias_correlated_via_outer_primary_key() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT o.id FROM a o WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = o.id)",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2]);
}

/// R1-S3：相关 EXISTS 经外层非主键列——只有 ref 命中内层的行放行。
#[tokio::test]
async fn exists_alias_correlated_via_outer_non_primary_key() {
    let db = open_db().await;
    exec_ok(&db, "CREATE TABLE a6 (id INT PRIMARY KEY, ref INT)").await;
    exec_ok(&db, "INSERT INTO a6 VALUES (1, 9)").await;
    exec_ok(&db, "INSERT INTO a6 VALUES (2, 1)").await;
    exec_ok(&db, "INSERT INTO a6 VALUES (3, 7)").await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, name TEXT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'Lin')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'Bo')").await;

    let got = int_ids(
        exec(
            &db,
            "SELECT o.id FROM a6 o WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = o.ref)",
        )
        .await,
    );
    assert_eq!(got, vec![2], "修复前全部外层行放行");
}

// === 算子 WHERE 标量比较：ISS16 ===

/// R1-S5：WHERE 相关标量比较——内层按外层每行收窄后计数。
#[tokio::test]
async fn where_scalar_subquery_correlated_via_alias() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT a.id FROM a WHERE (SELECT COUNT(*) FROM t x WHERE x.id = a.id) = 1",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "修复前静默空集");
}

/// 同形状零计数分支——内层无匹配时该外层行被滤除。
#[tokio::test]
async fn where_scalar_subquery_correlated_zero_count_filters_row() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT a.id FROM a WHERE (SELECT COUNT(*) FROM t x WHERE x.id = a.id) = 0",
        )
        .await,
    );
    assert_eq!(got, Vec::<i64>::new(), "修复前静默丢行");
}

/// 零回归锚：无别名内层的相关标量比较形状结果不变。
#[tokio::test]
async fn where_scalar_subquery_without_alias_zero_regression() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = int_ids(
        exec(
            &db,
            "SELECT a.id FROM a WHERE (SELECT COUNT(*) FROM t WHERE t.id = a.id) = 1",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2]);
}

// === 算子 SELECT 投影位标量：ISS16 ===

/// R1-S6：投影位相关标量——输出行数与外层一致，每行取本行匹配值。
#[tokio::test]
async fn select_list_scalar_subquery_correlated_via_alias() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let got = id_text_pairs(
        exec(
            &db,
            "SELECT a.id, (SELECT x.name FROM t x WHERE x.id = a.id) AS who FROM a",
        )
        .await,
    );
    assert_eq!(
        got,
        vec![(1, "Lin".to_string()), (2, "Bo".to_string())],
        "修复前塌缩为基列或报多行错"
    );
}

// === 内层形态维度：JOIN 臂别名 / 派生表别名 ===

/// 审计新增形状：EXISTS 内层 JOIN 两臂都带别名——内层自身先按真实语义求值。
#[tokio::test]
async fn exists_inner_join_both_arms_aliased() {
    let db = open_db().await;
    seed_index_and_join(&db).await;
    exec_ok(&db, "UPDATE c SET age = 41 WHERE id = 1").await;
    // c 复用为内层首臂：取 name 形状另建内层表。
    exec_ok(&db, "CREATE TABLE j (id INT PRIMARY KEY, name TEXT)").await;
    exec_ok(&db, "INSERT INTO j VALUES (1, 'Lin')").await;
    exec_ok(&db, "INSERT INTO j VALUES (2, 'Bo')").await;

    // 内层直执行：两臂别名解析正确，命中 {1}。
    let inner = int_ids(
        exec(
            &db,
            "SELECT x.id FROM j x JOIN u y ON x.id = y.id WHERE x.name = 'Lin'",
        )
        .await,
    );
    assert_eq!(inner, vec![1], "内层 JOIN 双臂别名直执行结果");

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE EXISTS \
         (SELECT 1 FROM j x JOIN u y ON x.id = y.id WHERE x.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "修复前恒空集");
}

/// 审计新增形状：EXISTS 内层派生表带别名——内层结果 {1}，外层全部放行。
#[tokio::test]
async fn exists_inner_derived_table_alias() {
    let db = open_db().await;
    seed_index_and_join(&db).await;
    exec_ok(&db, "CREATE TABLE j (id INT PRIMARY KEY, name TEXT)").await;
    exec_ok(&db, "INSERT INTO j VALUES (1, 'Lin')").await;
    exec_ok(&db, "INSERT INTO j VALUES (2, 'Bo')").await;

    let inner = int_ids(
        exec(
            &db,
            "SELECT d.id FROM (SELECT id, name FROM j) d WHERE d.name = 'Lin'",
        )
        .await,
    );
    assert_eq!(inner, vec![1], "派生表内层直执行结果");

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE EXISTS \
         (SELECT 1 FROM (SELECT id, name FROM j) d WHERE d.name = 'Lin')",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "修复前按内层存活行数截断外层");
}

// === A1 现状锁定：别名内层 + 基名限定引用 ===

/// A1（change proposal 默认假设）：子查询内层声明别名后，基名限定引用
/// （`FROM c1 y ... WHERE c1.age = 41`）保持引擎现状语义——按内层列求值，
/// EXISTS 对有匹配行的外层行成立。
#[tokio::test]
async fn a1_base_name_qualifier_inside_aliased_inner() {
    let db = open_db().await;
    seed_index_and_join(&db).await;
    exec_ok(&db, "CREATE TABLE c1 (id INT PRIMARY KEY, age INT)").await;
    exec_ok(&db, "INSERT INTO c1 VALUES (1, 41)").await;
    exec_ok(&db, "INSERT INTO c1 VALUES (2, 30)").await;

    let got = int_ids(
        exec(
            &db,
            "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM c1 y WHERE c1.age = 41)",
        )
        .await,
    );
    assert_eq!(got, vec![1, 2], "A1：别名内层 + 基名限定");
}

// === 计划可观测面 ===

/// 计划可观测性：内层别名不再产生幻影相关参数，`correlated_params` 回落为真实值。
#[tokio::test]
async fn explain_correlated_params_not_inflated_for_aliased_inner() {
    let db = open_db().await;
    seed_pairs(&db).await;

    let lines = plan_lines(
        exec(
            &db,
            "EXPLAIN SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.name = 'Lin')",
        )
        .await,
    );
    let semi = lines
        .iter()
        .find(|l| l.contains("SemiJoin"))
        .unwrap_or_else(|| panic!("计划树缺 SemiJoin 节点: {lines:?}"));
    assert!(
        semi.contains("correlated_params=0"),
        "内层别名被误判为相关外层引用: {semi:?}"
    );

    let lines = plan_lines(
        exec(
            &db,
            "EXPLAIN SELECT o.id FROM a o WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = o.id)",
        )
        .await,
    );
    let semi = lines
        .iter()
        .find(|l| l.contains("SemiJoin"))
        .unwrap_or_else(|| panic!("计划树缺 SemiJoin 节点: {lines:?}"));
    assert!(
        semi.contains("correlated_params=1"),
        "真实相关参数应保留 1 个: {semi:?}"
    );
}

// === 拒绝面零回归 ===

/// Non-goals：JOIN 形 IN 子查询维持计划期诚实拒绝（0A000），不因本次修复改变拒绝面。
#[tokio::test]
async fn join_form_in_subquery_still_rejected() {
    let db = open_db().await;
    seed_index_and_join(&db).await;

    let resp = exec(
        &db,
        "SELECT id FROM a WHERE a.id IN (SELECT x.id FROM c x JOIN u y ON x.id = y.id)",
    )
    .await;
    match resp {
        Response::Error { message, sqlstate } => {
            assert_eq!(
                message,
                "Plan error: IN subquery with JOIN is not supported"
            );
            assert_eq!(sqlstate.as_deref(), Some("0A000"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

/// 同款拒绝面的别名内层变体——别名不得成为绕开拒绝面的通道。
#[tokio::test]
async fn join_form_in_subquery_rejection_unaffected_by_alias() {
    let db = open_db().await;
    seed_index_and_join(&db).await;

    let resp = exec(
        &db,
        "SELECT id FROM a WHERE a.id IN \
         (SELECT x.id FROM c x JOIN u y ON x.id = y.id WHERE x.age = 41)",
    )
    .await;
    match resp {
        Response::Error { message, .. } => {
            assert_eq!(
                message,
                "Plan error: IN subquery with JOIN is not supported"
            )
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

/// 碰撞形状（change 2026-10-20-ms30-semantic-binding-differential-net
/// 001-replan，2026-10-20 用户裁定）：外层未别名声明 + 子查询内层别名声明同表
/// + 子查询内基表名限定引用——别名遮蔽基表名，解析为**外层相关参数**并注入
/// 求值（对齐 SQLite 标准语义）。控制组：无外层同名时维持 A1 宽松内层解析
/// （2026-10-15 豁免的非碰撞子集）。
#[tokio::test]
async fn collision_shape_alias_shadowing_resolves_outer() {
    let db = open_db().await;
    exec_ok(&db, "CREATE TABLE users (id INTEGER PRIMARY KEY, v INT)").await;
    exec_ok(&db, "CREATE TABLE orders (k INT, m INT)").await;
    for (i, v) in [(1, 10), (2, 20), (3, 30)] {
        exec_ok(&db, &format!("INSERT INTO users VALUES ({i}, {v})")).await;
    }
    exec_ok(&db, "INSERT INTO orders VALUES (2, 5)").await;

    // EXISTS 自相等：u.v = users.v → 外层相关（每行匹配自身）→ 全体。
    assert_eq!(
        int_ids(
            exec(
                &db,
                "SELECT id FROM users WHERE EXISTS (SELECT 1 FROM users u WHERE u.v = users.v)"
            )
            .await
        ),
        vec![1, 2, 3]
    );
    // EXISTS +10：u.v = users.v + 10 → 仅 v=10、20 行存在匹配。
    assert_eq!(
        int_ids(
            exec(
                &db,
                "SELECT id FROM users WHERE EXISTS \
                 (SELECT 1 FROM users u WHERE u.v = users.v + 10)"
            )
            .await
        ),
        vec![1, 2]
    );
    // 投影位标量子查询碰撞：每行取自身 v（未绑定参数现状为 NULL）。
    let mut proj: Vec<(i64, i64)> = rows(
        exec(
            &db,
            "SELECT id, (SELECT MAX(v) FROM users u WHERE u.id = users.id) FROM users",
        )
        .await,
    )
    .iter()
    .map(|r| {
        (
            r[0].as_i64().expect("id 整型"),
            r[1].as_i64().expect("碰撞参数注入后 MAX(v) 为整型"),
        )
    })
    .collect();
    proj.sort();
    assert_eq!(proj, vec![(1, 10), (2, 20), (3, 30)]);
    // IN 面：u.v = users.v 相关 → 每行自身命中 IN 集。
    assert_eq!(
        int_ids(
            exec(
                &db,
                "SELECT id FROM users WHERE id IN \
                 (SELECT u.id FROM users u WHERE u.v = users.v)"
            )
            .await
        ),
        vec![1, 2, 3]
    );
    // 控制组：无外层同名（orders）→ A1 宽松内层解析维持 → 2 = 5 假 → 空。
    assert_eq!(
        int_ids(
            exec(
                &db,
                "SELECT id FROM users WHERE EXISTS (SELECT 1 FROM orders o WHERE o.k = orders.m)"
            )
            .await
        ),
        Vec::<i64>::new()
    );
}

// === T04：投影位外层作用域段登记（无别名派生基座） ===

/// T04（change 2026-10-20-ms30-semantic-binding-differential-net）：无别名派生
/// 基座的隐式注册名 `derived` 必须在作用域登记里可达——登记段名与该臂实际
/// 注册名单源一致。
///
/// 形态 A（契约形状）：外层 FROM 为无别名派生表，投影位标量子查询以
/// `derived.k` 限定引用外层列 → 解析为外层相关参数并正确注入求值。
#[tokio::test]
async fn t04_projection_scalar_subquery_correlated_via_unnamed_derived() {
    let db = open_db().await;
    exec_ok(&db, "CREATE TABLE t (k INT, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 20)").await;
    exec_ok(&db, "CREATE TABLE tt (x INT, y INT)").await;
    exec_ok(&db, "INSERT INTO tt VALUES (100, 1)").await;
    exec_ok(&db, "INSERT INTO tt VALUES (200, 1)").await;
    exec_ok(&db, "INSERT INTO tt VALUES (300, 2)").await;

    let got = rows(
        exec(
            &db,
            "SELECT derived.v, \
             (SELECT MAX(x) FROM tt WHERE tt.y = derived.k) \
             FROM (SELECT k, v FROM t)",
        )
        .await,
    );
    assert_eq!(
        got,
        vec![
            vec![serde_json::json!(10), serde_json::json!(200)],
            vec![serde_json::json!(20), serde_json::json!(300)],
        ],
        "无别名派生基座的投影位相关引用应解析为外层相关参数并正确求值"
    );
}

/// T04 形态 B：无别名派生表作为**子查询自身 FROM** 的基座时，其隐式注册名
/// `derived` 同样必须可达——否则该限定引用被误判为外层相关参数，执行期报
/// `correlated parameter column not found in outer row`。
///
/// 修复前：EXISTS 臂报执行期错误（登记缺口）；修复后按内层派生表求值。
#[tokio::test]
async fn t04_subquery_own_from_unnamed_derived_qualifier_resolves_inner() {
    let db = open_db().await;
    exec_ok(&db, "CREATE TABLE t (k INT, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 20)").await;
    exec_ok(&db, "CREATE TABLE tt (x INT, y INT)").await;
    exec_ok(&db, "INSERT INTO tt VALUES (100, 1)").await;
    exec_ok(&db, "INSERT INTO tt VALUES (200, 1)").await;
    exec_ok(&db, "INSERT INTO tt VALUES (300, 2)").await;

    // 内层无别名派生表 (x, y)：`derived.y = 1` 命中两行 → EXISTS 对全部外层行为真
    let got = rows(
        exec(
            &db,
            "SELECT k FROM t WHERE EXISTS \
             (SELECT 1 FROM (SELECT x, y FROM tt) WHERE derived.y = 1)",
        )
        .await,
    );
    assert_eq!(
        got,
        vec![vec![serde_json::json!(1)], vec![serde_json::json!(2)]],
        "无别名派生的隐式名应解析为子查询自身 FROM 的内层基座"
    );

    // 同一形态的标量子查询臂：内层 MAX(x)（y=1 的两行）= 200 → 不等于 100 → 空集
    let got = rows(
        exec(
            &db,
            "SELECT k FROM t WHERE \
             (SELECT MAX(x) FROM (SELECT x, y FROM tt) WHERE derived.y = 1) = 100",
        )
        .await,
    );
    assert_eq!(
        got,
        Vec::<Vec<serde_json::Value>>::new(),
        "同一形态的标量臂按内层派生表求值 → 200 ≠ 100 → 空集"
    );
}
