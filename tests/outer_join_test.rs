//! MS25 Iteration 000 / Cycle 001-replan：OUTER JOIN 三态与 WHERE×JOIN 解禁
//!
//! change: 2026-09-26-ms25-sql-read-surface（Task T1，design D1/D2）
//!
//! 病灶：① `LEFT/RIGHT/FULL [OUTER] JOIN` 与 NATURAL/CROSS APPLY 一律计划期
//! `UnsupportedJoinType`（"Only INNER JOIN is supported"）；② `WHERE` 与任何
//! `JOIN` 组合整体计划期拒绝（`UnsupportedStatement("WHERE CLAUSE ON JOIN")`）。
//!
//! 目标语义（delta spec `sql-outer-join`）：
//! - R1 LEFT：匹配组合按 INNER 语义；左行无匹配 → 左值 + 右列全 NULL；ON 的
//!   NULL 求值按三值语义（Unknown 不匹配 → 走补 NULL 分支）。
//! - R2 RIGHT/FULL：RIGHT 右行无匹配 → 右值 + 左列全 NULL（计划期左右交换
//!   重写，输出列序不变）；FULL 两侧补齐。
//! - R3 组合面：WHERE 过滤作用于补 NULL 后的行流；ORDER BY 解析到 JOIN 输出
//!   形状；外连接形态在 IN 子查询内维持 `InSubqueryJoinUnsupported` 拒绝。
//! - R4 零回归：INNER 等值（Hash）与非等值（NLJ）无 WHERE 路径不变；其余
//!   JOIN 形态维持 `UnsupportedJoinType`；外连接一律经 NLJ（Hash 路径零触碰）。
//!
//! 全部外连接经 NLJ：行序为「左输入行序 × 右输入行序（物化一次）」，补
//! NULL 行紧随其保存侧行；FULL 的未匹配右行在左输入耗尽后按右行序追加。
//! RIGHT 经交换重写为「原右为左」的 LEFT（未匹配的原左行按标准 SQL 语义丢弃），
//! 故行序以原右表行序在前。

use rtsql::database::Database;
use rtsql::executor::{NestedLoopJoinType, PhysicalPlan};
use rtsql::network::protocol::{to_response, Response};
use rtsql::pipeline::{parse_stage, plan_stage};
use serde_json::json;
use std::process::Command;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
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

async fn plan_of(db: &Database, sql: &str) -> PhysicalPlan {
    let stmts = parse_stage(sql).await.expect("parse should succeed");
    let stmt = stmts.first().expect("one statement");
    plan_stage(db, sql, stmt, false)
        .await
        .expect("plan should succeed")
}

/// R1/R2 种子：`l(id INT PRIMARY KEY)` 含 1,2,3；`r(rid INT, val VARCHAR)` 含
/// (1,'a'),(2,'b')。LEFT 场景的共用夹具。
async fn seed_left_joined(db: &Database) {
    exec_ok(db, "CREATE TABLE l (id INT PRIMARY KEY)").await;
    exec_ok(db, "INSERT INTO l VALUES (1)").await;
    exec_ok(db, "INSERT INTO l VALUES (2)").await;
    exec_ok(db, "INSERT INTO l VALUES (3)").await;
    exec_ok(db, "CREATE TABLE r (rid INT, val VARCHAR)").await;
    exec_ok(db, "INSERT INTO r VALUES (1, 'a')").await;
    exec_ok(db, "INSERT INTO r VALUES (2, 'b')").await;
}

/// R1-S1：LEFT JOIN 匹配行组合 + 未匹配左行补 NULL。
///
/// 行序 = 左输入行序（匹配组合紧随其左行，补 NULL 行紧随未匹配左行）。
#[tokio::test]
async fn left_join_matched() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id, r.val FROM l LEFT JOIN r ON l.id = r.rid")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!("a")],
            vec![json!(2), json!("b")],
            vec![json!(3), json!(null)],
        ],
        "LEFT JOIN 必须输出匹配组合并为未匹配左行补 NULL 右列"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S2：NULL 连接键不匹配任何行（左行走补 NULL 分支）。
///
/// 夹具说明：delta spec 该场景的 GIVEN 表 `l(id INT)` 无键位声明，其 NULL 行
/// 属无键行——无键行落库但不进索引（MS10-T05 键位语义），而 JOIN 的输入走
/// `ScanNode` 的索引遍历，故该行在 JOIN 内不可见（既有 INNER JOIN 面同样
/// 如此，语料 `nested_loop_join_test::null_side_combination_excluded` 依赖）。
/// 此处以 `k INT PRIMARY KEY` 为行可见性锚（NULL 落在非键位列 `id`），使场景
/// 的语义要求「NULL 键不匹配任何行 → 补 NULL」可被见证。
#[tokio::test]
async fn left_join_null_key_no_match() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE ln (id INT, k INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO ln VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO ln VALUES (NULL, 2)").await;
    exec_ok(&db, "CREATE TABLE rn (rid INT, val VARCHAR)").await;
    exec_ok(&db, "INSERT INTO rn VALUES (1, 'a')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT ln.id, rn.val FROM ln LEFT JOIN rn ON ln.id = rn.rid")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("a")], vec![json!(null), json!(null)]],
        "NULL 键不匹配任何行，左行自身值保留、右列补 NULL"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S3：非等值 ON 经 NLJ 通道可达（外连接不因 ON 形态改变承载通道）。
#[tokio::test]
async fn left_join_non_equi_on() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE lv (v INT)").await;
    exec_ok(&db, "INSERT INTO lv VALUES (1)").await;
    exec_ok(&db, "INSERT INTO lv VALUES (5)").await;
    exec_ok(&db, "CREATE TABLE rv (rid INT, val VARCHAR)").await;
    exec_ok(&db, "INSERT INTO rv VALUES (3, 'x')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT lv.v, rv.val FROM lv LEFT JOIN rv ON rv.rid > lv.v")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("x")], vec![json!(5), json!(null)]],
        "非等值 ON 的 LEFT JOIN：v=1 匹配 (3,'x')，v=5 无匹配补 NULL"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S1：RIGHT JOIN 未匹配右行补 NULL 左列，未匹配左行丢弃。
///
/// 期望行集：`(1,'x')`（匹配）、`(NULL,'z')`（r 行 9 无匹配）。l 行 2 无匹配
/// 右行，按标准 SQL 语义在 RIGHT JOIN 中被丢弃（保留它是 FULL JOIN 的定义）。
/// 行序以交换重写后的左输入（原右表）行序为准。
#[tokio::test]
async fn right_join_unmatched_padded() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE l (id INT, name VARCHAR)").await;
    exec_ok(&db, "INSERT INTO l VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO l VALUES (2, 'b')").await;
    exec_ok(&db, "CREATE TABLE r (rid INT, val VARCHAR)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1, 'x')").await;
    exec_ok(&db, "INSERT INTO r VALUES (9, 'z')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id, r.val FROM l RIGHT JOIN r ON l.id = r.rid")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("x")], vec![json!(null), json!("z")]],
        "RIGHT JOIN 必须保留全部右行并为未匹配右行补 NULL；未匹配左行丢弃"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S2：FULL JOIN 两侧未匹配补齐（未匹配右行在左输入耗尽后按右行序追加）。
#[tokio::test]
async fn full_join_both_sides_padded() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE fl (id INT)").await;
    exec_ok(&db, "INSERT INTO fl VALUES (1)").await;
    exec_ok(&db, "INSERT INTO fl VALUES (2)").await;
    exec_ok(&db, "CREATE TABLE fr (rid INT, val VARCHAR)").await;
    exec_ok(&db, "INSERT INTO fr VALUES (1, 'x')").await;
    exec_ok(&db, "INSERT INTO fr VALUES (9, 'z')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT fl.id, fr.val FROM fl FULL JOIN fr ON fl.id = fr.rid")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!("x")],
            vec![json!(2), json!(null)],
            vec![json!(null), json!("z")],
        ],
        "FULL JOIN 必须两侧补齐：未匹配左行补 NULL 右列、未匹配右行补 NULL 左列"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S1：外层 WHERE 过滤作用于补 NULL 后的行流（补 NULL 行被滤除）。
#[tokio::test]
async fn where_filters_padded_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id FROM l LEFT JOIN r ON l.id = r.rid WHERE r.val = 'a'")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)]],
        "WHERE 引用补 NULL 列过滤时，未匹配左行（r.val 为 NULL，判定 Unknown）必须被滤除"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S2：ORDER BY 作用于外连接结果（裸列名键解析到 JOIN 输出形状）。
///
/// 行序按既有比较语义：`compare_values` 把 NULL 排在非 NULL 之后，DESC 反转
/// 比较结果后 NULL 居首（既有语义，本能力不改排序 NULL 行为）。
#[tokio::test]
async fn order_by_on_outer_result() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id, r.val FROM l LEFT JOIN r ON l.id = r.rid ORDER BY val DESC")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(3), json!(null)],
            vec![json!(2), json!("b")],
            vec![json!(1), json!("a")],
        ],
        "ORDER BY 裸列名键必须解析到 JOIN 输出形状并对补 NULL 行参与排序"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S3：IN 子查询中的外连接形态维持计划期点名拒绝（`ON true` 形态可达，
/// 拒绝由子查询 JOIN 诚实化臂承担——与无 WHERE 的 JOIN 形态文案统一）。
#[tokio::test]
async fn in_subquery_join_still_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    let resp = to_response(
        db.execute_sql("SELECT id FROM l WHERE id IN (SELECT rid FROM r LEFT JOIN l ON true)")
            .await,
    );
    let msg = error_msg(resp);
    assert!(
        msg.contains("IN subquery with JOIN is not supported"),
        "IN 子查询内 JOIN 形态必须点名拒绝，实际 {msg:?}"
    );
    assert!(
        !msg.contains("multiple columns"),
        "拒绝信息不得误报为子查询返回多列，实际 {msg:?}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R4：其余 JOIN 形态维持既有点名拒绝。
///
/// 文案敏感行：`Inner(Using/Natural/None)` 维持既有 `MissingOnClause`（契约
/// 「`Inner(Using/None)` 维持 `MissingOnClause`」+ 既有语义不触碰）；外连接的
/// Using/Natural/None 与其余 `JoinOperator` 维持 `UnsupportedJoinType`。后者
/// 的 Display 文本（"Only INNER JOIN is supported"）在本能力后对 NATURAL
/// 外连接已不精确；该措辞修订属 Plan 的 spec 面裁定，本用例随文本修订同步
/// 更新。
#[tokio::test]
async fn natural_join_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    for sql in [
        "SELECT l.id FROM l NATURAL JOIN r",
        "SELECT l.id FROM l JOIN r USING (id)",
    ] {
        let msg = error_msg(to_response(db.execute_sql(sql).await));
        assert!(
            msg.contains("INNER JOIN requires ON clause"),
            "{sql}: INNER 的 Using/Natural 形态必须维持 MissingOnClause 拒绝，实际 {msg:?}"
        );
    }

    for sql in [
        "SELECT l.id FROM l NATURAL LEFT JOIN r",
        "SELECT l.id FROM l NATURAL FULL JOIN r",
        "SELECT l.id FROM l LEFT JOIN r USING (id)",
    ] {
        let msg = error_msg(to_response(db.execute_sql(sql).await));
        assert!(
            msg.contains("Only INNER JOIN is supported"),
            "{sql}: 外连接的 Using/Natural 形态必须维持 UnsupportedJoinType 点名拒绝，实际 {msg:?}"
        );
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R4：外连接一律经 NLJ（等值 ON 不走 Hash），RIGHT 计划期交换左右输入。
#[tokio::test]
async fn outer_join_routes_to_nested_loop_join() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    for (sql, expected) in [
        (
            "SELECT l.id, r.val FROM l LEFT JOIN r ON l.id = r.rid",
            NestedLoopJoinType::Left,
        ),
        (
            "SELECT l.id, r.val FROM l FULL JOIN r ON l.id = r.rid",
            NestedLoopJoinType::Full,
        ),
        (
            "SELECT l.id, r.val FROM l RIGHT JOIN r ON l.id = r.rid",
            NestedLoopJoinType::Left,
        ),
    ] {
        match plan_of(&db, sql).await {
            PhysicalPlan::NestedLoopJoin(node) => {
                assert_eq!(node.join_type, expected, "{sql}: 外连接承载类型");
            }
            other => panic!("{sql}: 外连接必须路由 NestedLoopJoin，实际 {other:?}"),
        }
    }

    // RIGHT 的交换重写：左输入为原右表、右输入为原左表（输出列序不变由
    // output_columns 保证）。
    match plan_of(
        &db,
        "SELECT l.id, r.val FROM l RIGHT JOIN r ON l.id = r.rid",
    )
    .await
    {
        PhysicalPlan::NestedLoopJoin(node) => {
            assert!(
                matches!(node.left.as_ref(), PhysicalPlan::Scan(s) if s.table_name == "r"),
                "RIGHT 重写后左输入必须是原右表"
            );
            assert!(
                matches!(node.right.as_ref(), PhysicalPlan::Scan(s) if s.table_name == "l"),
                "RIGHT 重写后右输入必须是原左表"
            );
        }
        other => panic!("RIGHT JOIN 必须路由 NestedLoopJoin，实际 {other:?}"),
    }

    // R4 零回归：INNER 等值 ON 保持 Hash Join 计划形状。
    match plan_of(&db, "SELECT l.id, r.val FROM l JOIN r ON l.id = r.rid").await {
        PhysicalPlan::Join(_) => {}
        other => panic!("INNER 等值 ON 必须保持 Hash Join 形状，实际 {other:?}"),
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// D2 边界：`IN (SELECT …)` / `EXISTS (SELECT …)` 谓词在 JOIN 基座上维持既有
/// 计划期拒绝（逐字节不变）。
///
/// SemiJoin / AntiJoin 的左输入是单表计划（`build_output_columns_for_table`
/// 按单表 schema 解析输出列），与 JOIN 组合形状不同构——不在本能力解禁范围。
/// 该断言锁定边界：若将来解禁，必须先让 SemiJoin/AntiJoin 消费组合形状，否则
/// 会静默产出错误行形状。
#[tokio::test]
async fn where_subquery_over_join_stays_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    for sql in [
        "SELECT l.id FROM l LEFT JOIN r ON l.id = r.rid WHERE l.id IN (SELECT rid FROM r)",
        "SELECT l.id FROM l LEFT JOIN r ON l.id = r.rid WHERE EXISTS (SELECT rid FROM r)",
    ] {
        let msg = error_msg(to_response(db.execute_sql(sql).await));
        assert!(
            msg.contains("Unsupported statement type: WHERE CLAUSE ON JOIN"),
            "{sql}: JOIN 基座上的子查询谓词必须维持既有计划期拒绝，实际 {msg:?}"
        );
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// D2：WHERE × JOIN 在 INNER（等值 Hash 路径与 NLJ 路径）与 OUTER 上均可达。
#[tokio::test]
async fn where_over_inner_join_reachable() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id, r.val FROM l JOIN r ON l.id = r.rid WHERE r.val <> 'b'")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("a")]],
        "INNER 等值 JOIN + WHERE 必须可达并按组合行谓词过滤"
    );

    // NLJ 路径：`l.id < r.rid` 下 l.id=1 只匹配 rid=2（rid=1 不满足严格小于）
    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id, r.val FROM l JOIN r ON l.id < r.rid WHERE l.id = 1")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("b")]],
        "INNER 非等值（NLJ 路径）JOIN + WHERE 必须可达"
    );

    let rows = query_rows(to_response(
        db.execute_sql("SELECT l.id, r.val FROM l LEFT JOIN r ON l.id = r.rid WHERE l.id > 2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(3), json!(null)]],
        "OUTER JOIN + WHERE 必须作用于补 NULL 后的行流"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// D2：WHERE 谓词在组合行布局上解析——限定名 / 唯一非限定名可达，多表同名
/// `AmbiguousColumn`、缺失列 `ColumnNotFound`（既有错误面，无新文案）。
#[tokio::test]
async fn where_over_join_column_resolution() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE jl (id INT, lval INT)").await;
    exec_ok(&db, "INSERT INTO jl VALUES (1, 10)").await;
    exec_ok(&db, "CREATE TABLE jr (id INT, rval INT)").await;
    exec_ok(&db, "INSERT INTO jr VALUES (1, 20)").await;
    exec_ok(&db, "INSERT INTO jr VALUES (2, 30)").await;

    // 限定名 → 表内位置
    let rows = query_rows(to_response(
        db.execute_sql("SELECT jl.lval FROM jl JOIN jr ON jl.id = jr.id WHERE jr.rval = 20")
            .await,
    ));
    assert_eq!(rows, vec![vec![json!(10)]]);

    // 非限定名在组合列中唯一命中
    let rows = query_rows(to_response(
        db.execute_sql("SELECT jl.lval FROM jl JOIN jr ON jl.id = jr.id WHERE lval = 10")
            .await,
    ));
    assert_eq!(rows, vec![vec![json!(10)]]);

    // 两表同名 → AmbiguousColumn
    let msg = error_msg(to_response(
        db.execute_sql("SELECT jl.lval FROM jl JOIN jr ON jl.id = jr.id WHERE id = 1")
            .await,
    ));
    assert!(
        msg.contains("Ambiguous column: 'id'"),
        "组合布局下多表同名列必须点名歧义，实际 {msg:?}"
    );

    // 组合列中不存在 → ColumnNotFound
    let msg = error_msg(to_response(
        db.execute_sql("SELECT jl.lval FROM jl JOIN jr ON jl.id = jr.id WHERE nosuch = 1")
            .await,
    ));
    assert!(
        msg.contains("Column not found: nosuch"),
        "组合布局下缺失列必须点名列名，实际 {msg:?}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// D2：WHERE × JOIN × ORDER BY 的行形状与表头 = SELECT 清单（Sort 承担裁剪），
/// LIMIT 作用于该结果。
#[tokio::test]
async fn where_over_join_projection_shape() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_left_joined(&db).await;

    let rows = query_rows(
        to_response(db.execute_sql(
            "SELECT l.id FROM l LEFT JOIN r ON l.id = r.rid WHERE l.id > 0 ORDER BY id DESC LIMIT 2",
        )
        .await),
    );
    assert_eq!(
        rows,
        vec![vec![json!(3)], vec![json!(2)]],
        "WHERE×JOIN×ORDER BY×LIMIT 必须按 SELECT 清单的行形状排序并截断"
    );

    // 计划形状：Sort 持组合形状上的 SELECT 投影（Filter 不裁剪）。
    match plan_of(
        &db,
        "SELECT l.id FROM l LEFT JOIN r ON l.id = r.rid WHERE l.id > 0 ORDER BY id DESC",
    )
    .await
    {
        PhysicalPlan::Sort(node) => {
            assert_eq!(node.projection, vec![0], "Sort 必须持 SELECT 投影 [0]");
            assert!(
                matches!(node.input.as_ref(), PhysicalPlan::Filter(f) if f.projection.is_empty()),
                "Filter 必须是恒等投影（组合行全列承载）"
            );
        }
        other => panic!("ORDER BY 查询必须以 Sort 为顶层节点，实际 {other:?}"),
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// D2：WHERE × JOIN × 聚合分组可达（Filter 恒等承载，分组键在组合形状上解析）。
#[tokio::test]
async fn where_over_join_aggregate() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE ac (id INT PRIMARY KEY, k INT, amt INT)").await;
    exec_ok(&db, "INSERT INTO ac VALUES (1, 1, 10)").await;
    exec_ok(&db, "INSERT INTO ac VALUES (2, 1, 20)").await;
    exec_ok(&db, "INSERT INTO ac VALUES (3, 2, 30)").await;
    exec_ok(&db, "CREATE TABLE bc (k2 INT, tag VARCHAR)").await;
    exec_ok(&db, "INSERT INTO bc VALUES (1, 'x')").await;
    exec_ok(&db, "INSERT INTO bc VALUES (2, 'y')").await;

    let rows = query_rows(
        to_response(db.execute_sql(
            "SELECT ac.k, SUM(ac.amt) FROM ac JOIN bc ON ac.k = bc.k2 WHERE bc.tag = 'x' GROUP BY ac.k",
        )
        .await),
    );
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(30)]],
        "WHERE×JOIN×GROUP BY 必须在组合形状上解析分组键并聚合"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// D2：表头 = SELECT 清单的端到端见证（真二进制 JSON 渲染；ORDER BY 存在时
/// 由 Sort 承担裁剪，WHERE 存在时由 Filter 承担裁剪）。
#[test]
fn where_over_join_cli_header() {
    let dir = tempdir().unwrap();
    let db = db_path(&dir);
    let db_str = db.to_str().unwrap().to_string();

    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["new", &db_str])
        .current_dir(&dir)
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql new");
    assert!(
        new_out.status.success(),
        "rtsql new failed: {}",
        String::from_utf8_lossy(&new_out.stderr)
    );

    for sql in [
        "CREATE TABLE hl (id INT PRIMARY KEY)",
        "INSERT INTO hl VALUES (1)",
        "INSERT INTO hl VALUES (2)",
        "CREATE TABLE hr (rid INT, val VARCHAR)",
        "INSERT INTO hr VALUES (1, 'a')",
        "INSERT INTO hr VALUES (2, 'b')",
    ] {
        let out = run_cli(&dir, &db_str, sql);
        assert_eq!(out.0, Some(0), "setup 失败 {}: {}", sql, out.2);
    }

    for (sql, columns) in [
        (
            "SELECT hl.id FROM hl LEFT JOIN hr ON hl.id = hr.rid WHERE hr.rid > 0",
            json!(["id"]),
        ),
        (
            "SELECT hl.id FROM hl LEFT JOIN hr ON hl.id = hr.rid WHERE hr.rid > 0 ORDER BY id DESC",
            json!(["id"]),
        ),
        (
            "SELECT hl.id, hr.val FROM hl JOIN hr ON hl.id = hr.rid WHERE hr.rid > 0 ORDER BY val DESC",
            json!(["id", "val"]),
        ),
    ] {
        let out = run_cli(&dir, &db_str, sql);
        assert_eq!(out.0, Some(0), "{} 失败: {}", sql, out.2);
        let parsed: serde_json::Value = serde_json::from_str(out.1.trim())
            .unwrap_or_else(|e| panic!("{}: stdout 非 JSON（{e}）: {}", sql, out.1));
        assert_eq!(
            parsed["columns"], columns,
            "{}: 表头必须等于 SELECT 清单",
            sql
        );
    }
}

/// 运行真二进制（one-shot，JSON 渲染）——(退出码, stdout, stderr)。
fn run_cli(dir: &TempDir, db: &str, sql: &str) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .arg(db)
        .arg(sql)
        .args(["--format", "json"])
        .current_dir(dir.path())
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql binary");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}
