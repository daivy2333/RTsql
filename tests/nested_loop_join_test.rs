//! MS09 Iteration 001：非等值 JOIN 经 NestedLoopJoin 可达（I015 NLJ 部分）
//!
//! change: 2026-09-13-ms09-engine-mvcc-closeout
//!
//! 病灶（I015）：`extract_join_conditions`（planner ddl_dml.rs）仅接受 AND
//! 组合的列=列等值腿，任一非等值/字面量/表达式腿在计划期
//! `Plan error: Unsupported expression type`（exit 3）——非等值 JOIN 不可达。
//!
//! 目标语义（delta spec join-executor-selection）：等值 ON 保持既有 Hash
//! Join 路径（计划形状/结果逐字节不变）；含任一非等值腿的 ON 生成
//! NestedLoopJoin——对左输入每行 × 右输入每行的组合行（左 0..n ++ 右
//! n..n+m）求值完整 ON 谓词，三值语义下非 Unknown 非假的组合按
//! output_columns 产出。测试表全部为非 PK 表（规避键路由交互面）。

use rtsql::database::Database;
use rtsql::executor::{
    inject_correlated_values, ColumnExpression, ComparisonOp, ComparisonPredicate, FilterNode,
    NestedLoopJoinNode, ParameterExpression, PhysicalPlan, PredicateRef, ScanNode, Value,
};
use rtsql::network::protocol::Response;
use rtsql::pipeline::{parse_stage, plan_stage};
use serde_json::json;
use std::sync::Arc;
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

/// 按 (首列, 次列) 数值排序——NLJ/Hash 均不约定行序，断言前规范化。
fn sort_rows_i64(mut rows: Vec<Vec<serde_json::Value>>) -> Vec<Vec<serde_json::Value>> {
    rows.sort_by(|a, b| {
        let ka = (a[0].as_i64().unwrap(), a.get(1).and_then(|v| v.as_i64()));
        let kb = (b[0].as_i64().unwrap(), b.get(1).and_then(|v| v.as_i64()));
        ka.cmp(&kb)
    });
    rows
}

async fn plan_of(db: &Database, sql: &str) -> PhysicalPlan {
    let stmts = parse_stage(sql).await.expect("parse should succeed");
    let stmt = stmts.first().expect("one statement");
    plan_stage(db, sql, stmt, false)
        .await
        .expect("plan should succeed")
}

/// R2-S1：不等式 JOIN 产出语义连接结果。
///
/// RED（预期）：计划期 `Unsupported expression type`——非等值腿被
/// `extract_join_conditions` 拒绝。
#[tokio::test]
async fn inequality_join_produces_semantic_result() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1)").await;
    exec_ok(&db, "INSERT INTO r VALUES (2)").await;
    exec_ok(&db, "INSERT INTO s VALUES (2)").await;
    exec_ok(&db, "INSERT INTO s VALUES (3)").await;

    let rows = sort_rows_i64(query_rows(
        db.execute_sql("SELECT r.a, s.b FROM r JOIN s ON r.a < s.b")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(2)],
            vec![json!(1), json!(3)],
            vec![json!(2), json!(3)]
        ],
        "不等式 JOIN 必须产出语义连接结果 {{(1,2),(1,3),(2,3)}}"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S2：混合腿（非等值 + 非等值）多腿 AND 全部评估。
///
/// RED（预期）：计划期 `Unsupported expression type`。
#[tokio::test]
async fn mixed_legs_multi_and_evaluated() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1)").await;
    exec_ok(&db, "INSERT INTO r VALUES (2)").await;
    exec_ok(&db, "INSERT INTO s VALUES (2)").await;
    exec_ok(&db, "INSERT INTO s VALUES (3)").await;

    let rows = query_rows(
        db.execute_sql("SELECT r.a, s.b FROM r JOIN s ON r.a < s.b AND r.a >= 2")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(2), json!(3)]],
        "混合 ON 的全部腿必须逐组合评估，仅产出满足全部腿的组合"
    );

    db.wal_buffer.shutdown().await;
}

/// R3-S1：NULL 侧组合不进入结果（三值语义 Unknown 折叠为不匹配）。
///
/// RED（预期）：计划期 `Unsupported expression type`。
#[tokio::test]
async fn null_side_combination_excluded() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1)").await;
    exec_ok(&db, "INSERT INTO r VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO s VALUES (2)").await;
    exec_ok(&db, "INSERT INTO s VALUES (NULL)").await;

    let rows = sort_rows_i64(query_rows(
        db.execute_sql("SELECT r.a, s.b FROM r JOIN s ON r.a < s.b")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(2)]],
        "谓词涉及 NULL 侧的组合必须被排除（仅 (1,2) 可达）"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S3：任一侧空表 → 空集，不报错。
///
/// RED（预期）：计划期 `Unsupported expression type`。
#[tokio::test]
async fn empty_input_yields_empty_set() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;
    exec_ok(&db, "INSERT INTO s VALUES (2)").await;

    let rows = query_rows(
        db.execute_sql("SELECT r.a, s.b FROM r JOIN s ON r.a < s.b")
            .await,
    );
    assert_eq!(
        rows,
        Vec::<Vec<serde_json::Value>>::new(),
        "左侧空表必须产出空集"
    );

    let rows = query_rows(
        db.execute_sql("SELECT s.b, r.a FROM s JOIN r ON s.b > r.a")
            .await,
    );
    assert_eq!(
        rows,
        Vec::<Vec<serde_json::Value>>::new(),
        "右侧空表必须产出空集"
    );

    db.wal_buffer.shutdown().await;
}

/// R4：非等值 ON 计划形状 = NestedLoopJoin（计划期启发式可观测）。
///
/// RED（预期）：编译失败——`PhysicalPlan::NestedLoopJoin` 变体尚不存在
///（实现前测试文件整体编译失败计入 RED 观察）。
#[tokio::test]
async fn non_equi_on_routes_to_nested_loop_join() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;

    let plan = plan_of(&db, "SELECT r.a FROM r JOIN s ON r.a < s.b").await;
    match plan {
        PhysicalPlan::NestedLoopJoin(_) => {}
        other => panic!("非等值 ON 应路由 NestedLoopJoin，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

/// R1/R4：纯等值 ON 保持既有 Hash Join 计划形状。
#[tokio::test]
async fn equi_on_keeps_hash_join_shape() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;

    let plan = plan_of(&db, "SELECT r.a FROM r JOIN s ON r.a = s.b").await;
    match plan {
        PhysicalPlan::Join(_) => {}
        other => panic!("纯等值 ON 应保持既有 Hash Join 形状，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

/// R5：非等值 JOIN + ORDER BY 排序正确（Sort 消费 NLJ 输出形状）。
///
/// 排序列用裸列名——`extract_column_name` 只接受 Identifier，限定名
/// ORDER BY 在既有 Hash Join 上同样被拒（预存共享拒绝面，CLI 探针实证
/// `ORDER BY users.id` over equi JOIN → exit 3），NLJ 对齐同一接受面
///（delta spec R5 保持，不扩大）。
///
/// RED（预期）：计划期 `Unsupported expression type`。
#[tokio::test]
async fn order_by_over_non_equi_join() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;
    exec_ok(&db, "INSERT INTO r VALUES (2)").await;
    exec_ok(&db, "INSERT INTO r VALUES (1)").await;
    exec_ok(&db, "INSERT INTO s VALUES (3)").await;
    exec_ok(&db, "INSERT INTO s VALUES (2)").await;

    let rows = query_rows(
        db.execute_sql("SELECT r.a, s.b FROM r JOIN s ON r.a < s.b ORDER BY a")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(2)],
            vec![json!(1), json!(3)],
            vec![json!(2), json!(3)]
        ],
        "ORDER BY 必须对 NLJ 输出正确排序"
    );

    db.wal_buffer.shutdown().await;
}

/// R5：关联子查询注入路径保持——`inject_correlated_values` NLJ 臂行为见证。
///
/// e2e 关联 ON × JOIN 形态（IN 子查询内 JOIN、ON 含外层引用）在批准范围
/// 内不可达：`get_subquery_first_column` 无 Join 形态臂（纯等值形态今天即
/// `Subquery returns multiple columns` 拒绝，Hash 同源预存边界）、
/// `extract_correlated_params` 仅扫描子查询 WHERE 不扫 ON、WHERE + JOIN
/// 维持计划期拒绝（父 Cycle 001-initial Blocker Handoff / Plan Review F1
/// 三段机理链）。本用例按 001-rework 裁定（用户选项 A「见证改形」）改形
/// 为可达见证：直接构造 NestedLoopJoin 计划，锁定注入臂机制本身——
/// (a) NLJ 谓词在组合行布局上真值随注入值翻转；(b) 左子树 Filter 谓词经
/// 臂递归同样收到注入（R5「注入路径保持」的可达见证面）。
#[test]
fn correlated_on_injection() {
    // join 谓词：a < o.y —— o.y 为关联参数占位；组合行 = left_row ++ right_row，
    // 左表首列在组合行索引 0。
    let join_pred: PredicateRef = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "a".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Lt,
        right: Arc::new(ParameterExpression::new("o.y".to_string())),
    });

    // 左子树 = Filter(第二关联参数谓词) 包 Scan：见证臂对子树的递归注入；
    // 右子树 = Scan。
    let filter_pred: PredicateRef = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "a".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Gt,
        right: Arc::new(ParameterExpression::new("o.x".to_string())),
    });
    let plan = PhysicalPlan::NestedLoopJoin(NestedLoopJoinNode {
        left: Box::new(PhysicalPlan::Filter(FilterNode {
            input: Box::new(PhysicalPlan::Scan(ScanNode {
                table_name: "r".to_string(),
                columns: vec!["a".to_string()],
                projection: Vec::new(),
            })),
            predicate: Arc::clone(&filter_pred),
            table_name: "r".to_string(),
            projection: Vec::new(),
        })),
        right: Box::new(PhysicalPlan::Scan(ScanNode {
            table_name: "s".to_string(),
            columns: vec!["b".to_string()],
            projection: Vec::new(),
        })),
        predicate: Arc::clone(&join_pred),
        output_columns: Vec::new(),
    });

    // 单次注入覆盖两条通路：o.y 经 NLJ 臂的谓词注入；o.x 经臂递归进入左
    // 子树 Filter 谓词。
    inject_correlated_values(
        &plan,
        &[
            ("o.y".to_string(), Value::Int(2)),
            ("o.x".to_string(), Value::Int(0)),
        ],
    );

    let row = vec![Value::Int(1)]; // 组合行布局：索引 0 = 左表列 a
    assert!(
        join_pred.evaluate(&row).unwrap(),
        "注入 o.y=2 后 join 谓词 a < o.y 对 a=1 必须为真"
    );
    assert!(
        filter_pred.evaluate(&row).unwrap(),
        "注入 o.x=0 后左子树 Filter 谓词 a > o.x 对 a=1 必须为真（递归注入）"
    );

    // 换值重注入：两个谓词的真值都必须随新注入值翻转。
    inject_correlated_values(
        &plan,
        &[
            ("o.y".to_string(), Value::Int(1)),
            ("o.x".to_string(), Value::Int(2)),
        ],
    );
    assert!(
        !join_pred.evaluate(&row).unwrap(),
        "注入 o.y=1 后 join 谓词 a < o.y 对 a=1 必须翻转为假"
    );
    assert!(
        !filter_pred.evaluate(&row).unwrap(),
        "注入 o.x=2 后 Filter 谓词 a > o.x 对 a=1 必须翻转为假（递归注入）"
    );
}

/// T13 可选见证：WHERE + 非等值 JOIN 维持计划期拒绝（两种 join 节点同语义）。
///
/// `query.rs` WHERE 处理的 `matches!` 扩展前，NLJ 基计划会漏接为
/// table_name "unknown" 的单表 WHERE 路径（错误行为而非既有拒绝）；
/// 本用例锁定扩展后的拒绝面。
#[tokio::test]
async fn where_over_non_equi_join_stays_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE r (a INT)").await;
    exec_ok(&db, "CREATE TABLE s (b INT)").await;

    let resp = db
        .execute_sql("SELECT r.a, s.b FROM r JOIN s ON r.a < s.b WHERE r.a > 0")
        .await;
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("Unsupported statement type"),
                "WHERE + 非等值 JOIN 必须维持计划期拒绝，实际 {message:?}"
            );
        }
        other => panic!("WHERE + 非等值 JOIN 应被拒绝，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}
