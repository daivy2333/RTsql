//! MS11-T01: SQL 表达式四件套与值表达式 E2E 矩阵
//!
//! change: 2026-09-10-ms11-t01-sql-expressions / spec: sql-expression-evaluation
//!
//! Iteration 000 见证范围：WHERE 侧全部形态（R1/R2）+ 值表达式的谓词操作数
//! 语义（R3）+ INSERT 负数字面量（R5）。spec R3/S1-S4 场景以 SELECT 投影
//! 形态书写，其输出形状依赖 Iteration 001 的 ProjectionNode 机制（T7-T9）；
//! 本文件以 WHERE 操作数等价形式见证同一值表达式语义（searched/simple
//! CASE、缺省 ELSE→NULL、COALESCE 首个非 NULL、CAST 转换矩阵与错误面）。
//!
//! 已知 spec 勘误（Act Response 偏差记录）：R1/S3「NOT LIKE '%o%'」THEN
//! 第三项写 Bob，按标准 LIKE 语义 '%o%' 命中 Bob（含 o），NOT LIKE 应返回
//! Alice——本文件按标准语义断言。
//!
//! RED（实施前实测）：目标形态全部 `Plan error: Unsupported expression
//! type`（或 INSERT 负数 `Unsupported value type`）。
//!
//! MS16 T8 校准（change 2026-09-12-ms16-correctness-batch，BH-1 裁定
//! 2026-09-12，spec key-column-type-conformance 校准段）：
//! `negative_number_literal_persists` 的负 Float 行由 Int 隐式键列表 `t`
//! 移入 Float 键列表 `tf`——R3 键列写入类型强制（design D3）后 Int 键列
//! 拒绝 Float 值；负 Int 行断言逐字节保留，I040 负 Int/负 Float 字面量
//! 折叠覆盖完整。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::TempDir;

async fn open_db() -> (Database, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("expr.db")).await.unwrap();
    (db, dir)
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

fn error_message(resp: Response) -> String {
    match resp {
        Response::Error { message } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// R1: 谓词表达式四件套
// ---------------------------------------------------------------------------

/// R1/S1：IN 匹配与否定
#[tokio::test]
async fn in_match_and_negation() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    for id in 1..=3 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({id})")).await;
    }

    let rows = query_rows(db.execute_sql("SELECT id FROM t WHERE id IN (1, 3)").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(1)], vec![serde_json::json!(3)]]
    );

    let rows = query_rows(
        db.execute_sql("SELECT id FROM t WHERE id NOT IN (1, 3)")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(2)]]);
}

/// R1/S2：BETWEEN 含端点
#[tokio::test]
async fn between_inclusive_endpoints() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    for id in 1..=4 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({id})")).await;
    }

    let rows = query_rows(
        db.execute_sql("SELECT id FROM t WHERE id BETWEEN 2 AND 3")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(2)], vec![serde_json::json!(3)]]
    );

    let rows = query_rows(
        db.execute_sql("SELECT id FROM t WHERE id NOT BETWEEN 2 AND 3")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(1)], vec![serde_json::json!(4)]]
    );
}

/// R1/S3：LIKE 通配符与否定
///
/// 第三问按标准语义断言 Alice（'%o%' 命中 Bob/Carol，不含 o 的仅 Alice）；
/// spec THEN 第三项「Bob」为笔误，见文件头勘误注记。
#[tokio::test]
async fn like_wildcards_and_negation() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    for name in ["Alice", "Bob", "Carol"] {
        exec_ok(&db, &format!("INSERT INTO t VALUES ('{name}')")).await;
    }

    let rows = query_rows(
        db.execute_sql("SELECT name FROM t WHERE name LIKE 'A%'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("Alice")]]);

    let rows = query_rows(
        db.execute_sql("SELECT name FROM t WHERE name LIKE '_ob'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("Bob")]]);

    let rows = query_rows(
        db.execute_sql("SELECT name FROM t WHERE name NOT LIKE '%o%'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("Alice")]]);
}

/// R1/S4：IS NULL 与 IS NOT NULL
#[tokio::test]
async fn is_null_and_is_not_null() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3)").await;

    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE v IS NULL").await);
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);

    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE v IS NOT NULL").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(1)], vec![serde_json::json!(3)]]
    );
}

/// R1/S5：NOT 复合谓词
#[tokio::test]
async fn not_compound_predicate() {
    let (db, _dir) = open_db().await;
    // 显式 id 主键：避免首列 a 成为隐式键位而拒绝 a=1 重复行
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT)").await;
    for (id, a, b) in [(1, 1, 2), (2, 1, 5), (3, 3, 2), (4, 3, 9)] {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({id}, {a}, {b})")).await;
    }

    // 既非 a=1 也非 b=2 → 仅 (3,9)
    let rows = query_rows(
        db.execute_sql("SELECT a, b FROM t WHERE NOT (a = 1 OR b = 2)")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(3), serde_json::json!(9)]]);
}

/// R1/S7：ESCAPE 子句显式拒绝
#[tokio::test]
async fn like_escape_clause_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('Alice')").await;

    let msg = error_message(
        db.execute_sql("SELECT name FROM t WHERE name LIKE 'A%' ESCAPE '\\'")
            .await,
    );
    assert!(
        msg.contains("LIKE ESCAPE clause is not supported"),
        "ESCAPE 子句必须显式拒绝，实际: {msg}"
    );
}

/// R1/S7：ILIKE 维持既有拒绝（落默认臂）
#[tokio::test]
async fn ilike_remains_unsupported() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('Alice')").await;

    let msg = error_message(
        db.execute_sql("SELECT name FROM t WHERE name ILIKE 'a%'")
            .await,
    );
    assert!(
        msg.contains("Unsupported expression type"),
        "ILIKE 必须维持拒绝，实际: {msg}"
    );
}

// ---------------------------------------------------------------------------
// R2: NULL 三值语义
// ---------------------------------------------------------------------------

/// R2/S1：NOT IN 含 NULL 排除全部
#[tokio::test]
async fn not_in_with_null_excludes_all() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2)").await;

    let rows = query_rows(
        db.execute_sql("SELECT v FROM t WHERE v NOT IN (2, NULL)")
            .await,
    );
    assert_eq!(
        rows,
        Vec::<Vec<serde_json::Value>>::new(),
        "NOT Unknown 仍 Unknown，必须排除全部行"
    );
}

/// R2/S2：既有比较 NULL 行为不变
#[tokio::test]
async fn existing_comparison_null_behavior_unchanged() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2)").await;

    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE v = 2").await);
    assert_eq!(rows, vec![vec![serde_json::json!(2)]]);

    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE v > 0").await);
    assert_eq!(rows, vec![vec![serde_json::json!(2)]]);
}

/// R2/S3：OR 组合中 True 胜出（Unknown OR True = True）
#[tokio::test]
async fn or_true_beats_unknown() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (a INT, b INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL, 1)").await;

    let rows = query_rows(db.execute_sql("SELECT b FROM t WHERE a = 1 OR b = 1").await);
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
}

/// R2/S4：IS NULL 不受三值影响（NOT (v IS NULL) 正常取反）
#[tokio::test]
async fn not_is_null_inverts_normally() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let rows = query_rows(
        db.execute_sql("SELECT v FROM t WHERE NOT (v IS NULL)")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
}

// ---------------------------------------------------------------------------
// R3: 值表达式（WHERE 谓词操作数形态；SELECT 投影形态待 Iteration 001）
// ---------------------------------------------------------------------------

/// R3/S1 语义（谓词形态）：searched CASE 条件分支 + ELSE 缺省 NULL
#[tokio::test]
async fn searched_case_branches() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (score INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (95)").await;
    exec_ok(&db, "INSERT INTO t VALUES (40)").await;

    // 命中分支：score >= 60 → 1
    let rows = query_rows(
        db.execute_sql("SELECT score FROM t WHERE CASE WHEN score >= 60 THEN 1 ELSE 0 END = 1")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(95)]]);

    // 缺省 ELSE → NULL：未命中行产出 NULL，经 IS NULL 捕获
    let rows = query_rows(
        db.execute_sql("SELECT score FROM t WHERE CASE WHEN score >= 60 THEN 1 END IS NULL")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(40)]]);
}

/// R3/S2 语义（谓词形态）：simple CASE 的 operand 为 NULL 时不命中任何 WHEN
#[tokio::test]
async fn simple_case_null_operand_falls_to_else() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    // operand NULL 与 1 比较为 Unknown → 不命中 → ELSE
    let rows = query_rows(
        db.execute_sql("SELECT v FROM t WHERE CASE v WHEN 1 THEN 'one' ELSE 'other' END = 'other'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);

    let rows = query_rows(
        db.execute_sql("SELECT v FROM t WHERE CASE v WHEN 1 THEN 'one' ELSE 'other' END = 'one'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
}

/// R3/S3 语义（谓词形态）：COALESCE 逐参数取首个非 NULL，全 NULL 落兜底
#[tokio::test]
async fn coalesce_first_non_null_argument() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (a STRING, b STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL, 'x')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('y', 'z')").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL, NULL)").await;

    // 行 1：a NULL → 取 b='x'
    let rows = query_rows(
        db.execute_sql("SELECT b FROM t WHERE COALESCE(a, b, 'fallback') = 'x'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("x")]]);

    // 行 2：a='y' 非 NULL → 直接取 a
    let rows = query_rows(
        db.execute_sql("SELECT b FROM t WHERE COALESCE(a, b, 'fallback') = 'y'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("z")]]);

    // 行 3：全 NULL → 兜底参数
    let rows = query_rows(
        db.execute_sql("SELECT b FROM t WHERE COALESCE(a, b, 'fallback') = 'fallback'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);
}

/// R3/S4 语义（谓词形态）：CAST 字符串→Int 解析成功与失败两面
#[tokio::test]
async fn cast_string_to_int_both_faces() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (s STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('42')").await;

    let rows = query_rows(
        db.execute_sql("SELECT s FROM t WHERE CAST(s AS INT) = 42")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("42")]]);

    db.wal_buffer.shutdown().await;
    drop(db);

    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (s STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(
        db.execute_sql("SELECT s FROM t WHERE CAST(s AS INT) = 42")
            .await,
    );
    assert!(
        msg.contains("Type mismatch"),
        "'abc' 不可解析为 Int 必须执行期报错，实际: {msg}"
    );
}

/// R3/S4：Float→Int 截断向零
#[tokio::test]
async fn cast_float_to_int_truncates_toward_zero() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1.7)").await;

    let rows = query_rows(
        db.execute_sql("SELECT f FROM t WHERE CAST(f AS INT) = 1")
            .await,
    );
    assert_eq!(rows.len(), 1, "CAST(1.7 AS INT) = 1（截断向零）");
}

/// R3/S5：谓词操作数中使用值表达式
#[tokio::test]
async fn value_expression_as_predicate_operand() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (59.5)").await;
    exec_ok(&db, "INSERT INTO t VALUES (60.5)").await;

    // CAST(59.5) = 59 不命中；CAST(60.5) = 60 命中
    let rows = query_rows(
        db.execute_sql("SELECT f FROM t WHERE CAST(f AS INT) >= 60")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(60.5)]]);
}

/// R3/S4：数值→String 按值格式化（非 Value::Display，无引号）
#[tokio::test]
async fn cast_int_to_string_value_formatting() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (42)").await;

    let rows = query_rows(
        db.execute_sql("SELECT v FROM t WHERE CAST(v AS STRING) = '42'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(42)]]);
}

/// R3/S4：CAST NULL → NULL 短路
#[tokio::test]
async fn cast_null_short_circuits() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (7)").await;

    let rows = query_rows(
        db.execute_sql("SELECT v FROM t WHERE CAST(NULL AS INT) IS NULL")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(7)]]);
}

/// R3/S4：Bool↔数值转换拒绝（执行期）
#[tokio::test]
async fn cast_bool_numeric_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let msg = error_message(
        db.execute_sql("SELECT v FROM t WHERE CAST(v AS BOOL) = true")
            .await,
    );
    assert!(
        msg.contains("Type mismatch"),
        "Int→Bool 必须拒绝，实际: {msg}"
    );

    let msg = error_message(
        db.execute_sql("SELECT v FROM t WHERE CAST(true AS INT) = 1")
            .await,
    );
    assert!(
        msg.contains("Type mismatch"),
        "Bool→Int 必须拒绝，实际: {msg}"
    );
}

/// R3/S4：TRY_CAST 显式拒绝（计划期）
#[tokio::test]
async fn try_cast_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (s STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('x')").await;

    let msg = error_message(
        db.execute_sql("SELECT s FROM t WHERE TRY_CAST(s AS INT) = 1")
            .await,
    );
    assert!(
        msg.contains("TRY_CAST"),
        "TRY_CAST 必须显式拒绝，实际: {msg}"
    );
}

/// R3/S4：未知目标类型显式拒绝（计划期，不兜底 String）
#[tokio::test]
async fn cast_unknown_target_type_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let msg = error_message(
        db.execute_sql("SELECT v FROM t WHERE CAST(v AS DATE) = v")
            .await,
    );
    assert!(
        msg.contains("CAST"),
        "未知 CAST 目标类型必须计划期拒绝，实际: {msg}"
    );
}

// ---------------------------------------------------------------------------
// R5: INSERT 负数字面量（I040）
// ---------------------------------------------------------------------------

/// R5/S1：负数入库、查询与重开库持久
///
/// MS16 T8 校准（BH-1 裁定，spec key-column-type-conformance 校准段）：
/// 负 Float 行原落在 Int 隐式键列表 `t`，R3 键列写入类型强制（MS16
/// design D3）后 Int 键列拒绝 Float 值，移入 Float 键列表 `tf`；负 Int
/// 行与断言逐字节保留，I040 覆盖完整。
#[tokio::test]
async fn negative_number_literal_persists() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("neg.db");
    let db = Database::open(&path).await.unwrap();

    exec_ok(&db, "CREATE TABLE t (v INT, s STRING)").await;
    // DDL 无 WAL 记录：catalog 需先落盘（keyless_row_test 夹具先例）
    db.buffer_pool.flush_all().await.unwrap();

    match db.execute_sql("INSERT INTO t VALUES (-1, 'x')").await {
        Response::AffectedRows { count: 1 } => {}
        other => panic!("负数 INSERT 应 affected 1，实际 {:?}", other),
    }

    // MS16 T8 校准：负 Float 行移入 Float 键列表（R3 强制后 Int 键列不接受）
    exec_ok(&db, "CREATE TABLE tf (f FLOAT, s STRING)").await;
    db.buffer_pool.flush_all().await.unwrap();
    match db.execute_sql("INSERT INTO tf VALUES (-1.5, 'y')").await {
        Response::AffectedRows { count: 1 } => {}
        other => panic!("负浮点 INSERT 应 affected 1，实际 {:?}", other),
    }

    let rows = query_rows(db.execute_sql("SELECT v, s FROM t WHERE s = 'x'").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(-1), serde_json::json!("x")]]
    );

    db.wal_buffer.shutdown().await;
    drop(db);

    let db2 = Database::open(&path).await.unwrap();
    let rows = query_rows(db2.execute_sql("SELECT v, s FROM t ORDER BY v").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(-1), serde_json::json!("x")]],
        "负数行必须持久化并在重开后可见"
    );
    let rows = query_rows(db2.execute_sql("SELECT f, s FROM tf ORDER BY f").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(-1.5), serde_json::json!("y")]],
        "负浮点行（Float 键列表）必须持久化并在重开后可见"
    );
    db2.wal_buffer.shutdown().await;
}

/// R5/S2：非字面量取负保持拒绝
#[tokio::test]
async fn non_literal_negation_still_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let msg = error_message(db.execute_sql("INSERT INTO t VALUES (-v)").await);
    assert!(
        msg.contains("Unsupported value type"),
        "列引用取负必须维持拒绝，实际: {msg}"
    );
}
