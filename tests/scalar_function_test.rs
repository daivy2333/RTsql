//! MS11-T03: 标量函数库第一批（string 6 + math 4）E2E 见证
//!
//! change: 2026-09-10-ms11-t03-scalar-functions / spec: sql-scalar-functions
//!
//! Iteration 000 见证范围：R1/S1-S4 + R2/S1-S6 + R4/S1-S3（13 场景）+
//! R1 要求文本的命名参数/通配符/零参拒绝锁 + 未知名双位置回归锁。
//! Iteration 001 追加：R3 组（abs/round/floor/ceil）、R4/S1 与 S3 的 abs
//! 腿、R5 组（六个调用面场景，S3/S4/S5 为既有行为锁）与 CEIL/FLOOR TO
//! 形态拒绝锁。R2/S1-S2 的表头文本断言在 cli_test 渲染追加中承载（lib
//! Response 不携带表头，MS11-T01 Iter001 惯例）。
//!
//! 已知 spec 勘误（Act Response 偏差记录）：R1/S1「SELECT nonexistent_fn(id)
//! FROM t」的 THEN 引用 `Unsupported expression type`——实测 change 前该
//! 位置的既有文案为 `Unsupported statement type`（ast.rs extract_columns
//! 门先于 planner 函数臂，二进制探针 2026-09-11）；`Unsupported expression
//! type` 是 WHERE 位置的既有文案。两处均按「维持既有拒绝行为、文案逐字节
//! 不变」断言实测值。
//!
//! RED（实施前实测）：SELECT 位置目标形态 `Plan error: Unsupported
//! statement type`；WHERE/值位置 `Plan error: Unsupported expression type`。
//! R1/S1 与 WHERE 未知名锁为既有行为回归锁（实施前后均绿）。
//!
//! R2/S5 表形偏差（预存缺陷规避，详见测试处注记与 Act Response）：无 PK
//! 表首列被登记为隐式 PK，字符串 Eq 谓词落入 Filter(Scan) 回退返回空结果
//! 集（pristine master 探针实锤，与函数无关）——测试表加声明 PK 列规避。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::TempDir;

async fn open_db() -> (Database, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("fn.db")).await.unwrap();
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
// R1: 函数注册与分派机制
// ---------------------------------------------------------------------------

/// R1/S1（含勘误）：SELECT 位置未知名维持既有 `Unsupported statement type`
/// （extract_columns 门既有文案，逐字节不变）。
#[tokio::test]
async fn unknown_fn_select_keeps_existing_rejection() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    let msg = error_message(db.execute_sql("SELECT nonexistent_fn(id) FROM t").await);
    assert!(
        msg.contains("Unsupported statement type"),
        "unexpected message: {msg}"
    );
}

/// R1 要求文本（WHERE 位置）：未知名维持既有 `Unsupported expression type`
/// （planner 函数臂既有文案，逐字节不变）。
#[tokio::test]
async fn unknown_fn_where_keeps_existing_rejection() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    let msg = error_message(
        db.execute_sql("SELECT id FROM t WHERE nonexistent_fn(id) = 1")
            .await,
    );
    assert!(
        msg.contains("Unsupported expression type"),
        "unexpected message: {msg}"
    );
}

/// R1/S2：OVER 窗口子句 plan 期点名拒绝。
#[tokio::test]
async fn over_clause_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT upper(name) OVER () FROM t").await);
    assert!(msg.contains("OVER"), "unexpected message: {msg}");
}

/// R1/S3：DISTINCT 限定 plan 期点名拒绝。
#[tokio::test]
async fn distinct_qualifier_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT upper(DISTINCT name) FROM t").await);
    assert!(msg.contains("DISTINCT"), "unexpected message: {msg}");
}

/// R1/S4：arity 不符 plan 期拒绝，文案点名函数名与参数个数要求。
#[tokio::test]
async fn arity_mismatch_rejected_at_plan() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT substr(name) FROM t").await);
    assert!(msg.contains("SUBSTR"), "unexpected message: {msg}");
    assert!(msg.contains("argument"), "unexpected message: {msg}");
}

/// R1 要求文本：命名参数 plan 期点名拒绝。
#[tokio::test]
async fn named_argument_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT upper(n => 'x') FROM t").await);
    assert!(msg.contains("Named argument"), "unexpected message: {msg}");
}

/// R1 要求文本：通配符参数 plan 期点名拒绝。
#[tokio::test]
async fn wildcard_argument_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT upper(*) FROM t").await);
    assert!(msg.contains("wildcard"), "unexpected message: {msg}");
}

/// R1 要求文本：零参调用 plan 期拒绝（arity 校验，文案点名函数名与个数）。
#[tokio::test]
async fn zero_arg_call_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT upper() FROM t").await);
    assert!(msg.contains("UPPER"), "unexpected message: {msg}");
    assert!(msg.contains("argument"), "unexpected message: {msg}");
}

// ---------------------------------------------------------------------------
// R2: string 函数六件
// ---------------------------------------------------------------------------

/// R2/S1：upper/lower 派生列值（表头文本断言归 cli_test，见文件头注记）。
#[tokio::test]
async fn upper_lower_derived_columns() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('AbC')").await;
    let rows = query_rows(
        db.execute_sql("SELECT upper(name), lower(name) FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!("ABC"), serde_json::json!("abc")]]
    );
}

/// R2/S2：AS 别名（值断言；列名 `u` 的表头断言归 cli_test）。
#[tokio::test]
async fn upper_alias_values() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let rows = query_rows(db.execute_sql("SELECT upper(name) AS u FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!("ABC")]]);
}

/// R2/S3：length 按 Unicode 字符计数（非字节）+ WHERE 过滤（下推路径）。
#[tokio::test]
async fn length_char_count_and_where() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('你好')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let rows = query_rows(
        db.execute_sql("SELECT name, length(name) FROM t WHERE length(name) > 2")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!("abc"), serde_json::json!(3)]]
    );
}

/// R2/S4：substr 常规与 SQLite 边缘（start=0 少一、负 start 尾部倒数、
/// 负 len 往前取、省略 len 取到尾）。
#[tokio::test]
async fn substr_routine_and_sqlite_edges() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (s STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abcdef')").await;
    let cases = [
        ("SELECT substr(s, 2) FROM t", "bcdef"),
        ("SELECT substr(s, 2, 3) FROM t", "bcd"),
        ("SELECT substr(s, 0, 2) FROM t", "a"),
        ("SELECT substr(s, -2) FROM t", "ef"),
        ("SELECT substr(s, 3, -1) FROM t", "b"),
    ];
    for (sql, expected) in cases {
        let rows = query_rows(db.execute_sql(sql).await);
        assert_eq!(rows, vec![vec![serde_json::json!(expected)]], "sql: {sql}");
    }
}

/// R2/S5：replace 全替换 + trim 仅剥空格（TAB 保留）。
///
/// 表形偏差（Act Response Deviations）：spec GIVEN 为单列无 PK 表
/// `t(s VARCHAR)`——无 PK 表首列被引擎登记为隐式 PK，字符串 Eq 谓词经
/// `to_key()=None` 落入 Filter(Scan) 回退返回空结果集（预存缺陷，与函数
/// 无关，pristine master 探针实锤）。本测试加声明 PK 列 `id` 规避：WHERE
/// 等值走 DataScan 下推路径，THEN 断言与 spec 逐字一致。预存缺陷另立
/// improvement 候选（见 Act Response）。
#[tokio::test]
async fn replace_and_trim_space_only() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, s STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a-b-a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, '  x ')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, '\tx ')").await;

    let rows = query_rows(
        db.execute_sql("SELECT replace(s, '-', '+') FROM t WHERE s = 'a-b-a'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("a+b+a")]]);

    let rows = query_rows(
        db.execute_sql("SELECT trim(s) FROM t WHERE s = '  x '")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("x")]]);

    // TAB 保留：行首 TAB 不剥，行尾空格剥
    let rows = query_rows(
        db.execute_sql("SELECT trim(s) FROM t WHERE s = '\tx '")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!("\tx")]]);
}

/// R2 契约边界：TRIM 规格化变体（BOTH/LEADING/TRAILING、自定义字符）plan 期
/// 点名拒绝——trim 只支持 `trim(s)` 单参形态（独立 sqlparser 变体，见 Act
/// Response Deviations）。
#[tokio::test]
async fn trim_specification_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, s STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, '  x ')").await;
    let msg = error_message(db.execute_sql("SELECT trim(BOTH ' ' FROM s) FROM t").await);
    assert!(msg.contains("TRIM"), "unexpected message: {msg}");
}

/// R2/S6：非字符串入参执行期类型错误，无隐式转换。
#[tokio::test]
async fn strict_type_error_on_int() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (123)").await;
    let msg = error_message(db.execute_sql("SELECT upper(id) FROM t").await);
    assert!(msg.contains("Type mismatch"), "unexpected message: {msg}");
}

// ---------------------------------------------------------------------------
// R3: math 函数四件
// ---------------------------------------------------------------------------

/// R3/S1：abs 保持入参类型（Int→Int、Float→Float）。
#[tokio::test]
async fn abs_keeps_input_type() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (i INT, f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (-5, -5.5)").await;
    let rows = query_rows(db.execute_sql("SELECT abs(i), abs(f) FROM t").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(5), serde_json::json!(5.5)]]
    );
}

/// R3/S2：round 半数远离零 + digits 正负整数位（Float digits 向零截断在单测覆盖）。
#[allow(clippy::approx_constant)] // 3.14 is a spec-locked rounding value, not PI
#[tokio::test]
async fn round_direction_and_digits() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3.14159)").await;
    let cases = [
        ("SELECT round(3.7) FROM t", 4.0),
        ("SELECT round(2.5) FROM t", 3.0),
        ("SELECT round(-2.5) FROM t", -3.0),
        ("SELECT round(f, 2) FROM t", 3.14),
        ("SELECT round(123.4, -1) FROM t", 120.0),
    ];
    for (sql, expected) in cases {
        let rows = query_rows(db.execute_sql(sql).await);
        assert_eq!(rows, vec![vec![serde_json::json!(expected)]], "sql: {sql}");
    }
}

/// R3/S3：floor/ceil 返回 Float 形态（含负数输入）。
#[tokio::test]
async fn floor_ceil_return_float() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3.2)").await;
    exec_ok(&db, "INSERT INTO t VALUES (-3.7)").await;
    let rows = query_rows(db.execute_sql("SELECT floor(f), ceil(f) FROM t").await);
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(3.0), serde_json::json!(4.0)],
            vec![serde_json::json!(-4.0), serde_json::json!(-3.0)],
        ]
    );
}

/// R3 契约边界：CEIL/FLOOR 的 `TO DateTimeField` 形态 plan 期点名拒绝——
/// ceil/floor 与 trim 同为独立 sqlparser 变体，只支持 `fn(x)` 单参形态
/// （镜像 trim_specification_rejected，见 Act Response Deviations）。
#[tokio::test]
async fn ceil_floor_to_datetime_field_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3.2)").await;
    let msg = error_message(db.execute_sql("SELECT ceil(f TO DAY) FROM t").await);
    assert!(msg.contains("CEIL"), "unexpected message: {msg}");
}

// ---------------------------------------------------------------------------
// R4: NULL 语义与嵌套参数
// ---------------------------------------------------------------------------

/// R4/S1（string 腿）：任一参数 NULL → NULL；`abs(NULL)` 腿归属 Iteration 001。
#[tokio::test]
async fn null_propagation_string_functions() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;

    let rows = query_rows(db.execute_sql("SELECT upper(name) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);

    // abs 腿（Iteration 001，R4/S1 完整三腿）：math NULL 同样传播
    let rows = query_rows(db.execute_sql("SELECT abs(NULL) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);

    let rows = query_rows(db.execute_sql("SELECT substr(name, 1, 1) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);
}

/// R4/S2：NULL 短路优先于类型校验（Int 型 NULL 不触发 string 函数类型错误）。
#[tokio::test]
async fn null_short_circuit_skips_type_check() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    let rows = query_rows(
        db.execute_sql("SELECT upper(CAST(NULL AS INT)) FROM t")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);
}

/// R4/S3（string 腿）：嵌套 COALESCE 先求值；`abs(-5)` 腿归属 Iteration 001。
#[tokio::test]
async fn nested_coalesce_argument() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('x')").await;
    let rows = query_rows(
        db.execute_sql("SELECT upper(COALESCE(name, 'empty')) FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("EMPTY")],
            vec![serde_json::json!("X")],
        ]
    );

    // abs 腿（Iteration 001，R4/S3 AND 子句）：负数字面量常量参数；
    // 常量函数项逐行产出，两行表 → 两行 5
    let rows = query_rows(db.execute_sql("SELECT abs(-5) FROM t").await);
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(5)], vec![serde_json::json!(5)]]
    );
}

// ---------------------------------------------------------------------------
// R5: 调用面与边界（Iteration 001；S3/S4/S5 为既有行为锁）
// ---------------------------------------------------------------------------

/// R5/S1：WHERE 函数谓词无 OR、非简单 PK 等值 → DataScan 下推路径，结果正确。
#[tokio::test]
async fn where_function_predicate_pushdown() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'AbC')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'xyz')").await;
    let rows = query_rows(
        db.execute_sql("SELECT id FROM t WHERE upper(name) = 'ABC'")
            .await,
    );
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
}

/// R5/S2：WHERE 函数谓词与 OR 组合 → Filter 包装路径，结果一致。
#[tokio::test]
async fn where_function_predicate_with_or() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'AbC')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'xyz')").await;
    let rows = query_rows(
        db.execute_sql("SELECT id FROM t WHERE upper(name) = 'ABC' OR id = 2")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(1)], vec![serde_json::json!(2)]]
    );
}

/// R5/S3：SELECT 列表聚合混用保持既有拒绝（聚合检测先于表达式项路由）。
#[tokio::test]
async fn aggregate_mixed_with_scalar_function_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('abc')").await;
    let msg = error_message(db.execute_sql("SELECT count(*), upper(name) FROM t").await);
    assert!(
        msg.contains("Invalid aggregate argument"),
        "unexpected message: {msg}"
    );
}

/// R5/S4：HAVING 中标量函数保持既有不支持拒绝（build_having_expression
/// 只认聚合名，注册标量名不改变该路径）。
#[tokio::test]
async fn having_scalar_function_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    let msg = error_message(
        db.execute_sql("SELECT id, count(*) FROM t GROUP BY id HAVING upper(id) = 'X'")
            .await,
    );
    assert!(
        msg.contains("Unsupported expression type"),
        "unexpected message: {msg}"
    );
}

/// R5/S5：ORDER BY 引用表达式项别名静默保持输入序（既有语义锁定，SHALL NOT
/// 报错、SHALL NOT 按别名列排序）。
#[tokio::test]
async fn order_by_expression_alias_keeps_input_order() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    let rows = query_rows(
        db.execute_sql("SELECT upper(name) AS u FROM t ORDER BY u")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!("B")], vec![serde_json::json!("A")]]
    );
}

/// R5/S6：主键列与函数比较走普通扫描路径（SHALL NOT 误入索引点查），
/// 结果正确。
#[tokio::test]
async fn pk_compared_with_function_uses_plain_scan() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (5)").await;
    let rows = query_rows(db.execute_sql("SELECT id FROM t WHERE id = abs(5)").await);
    assert_eq!(rows, vec![vec![serde_json::json!(5)]]);
}
