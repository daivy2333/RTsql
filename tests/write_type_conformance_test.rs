//! MS24 Iteration 000：写入值类型一致门（design D3，ISS04 / R29）
//!
//! change: 2026-09-25-ms24-write-surface-completion / tasks 1.1-1.3
//!
//! 缺陷（ISS04，2026-09-25 MS23 Review 登记）：MS16/MS23 分别收口 PK 键列与
//! INT 唯一列的类型边缘后，非键列写入面仍无类型校验——String/Bool 值写入
//! INT 列、Int 值写入 STRING 列静默落库（序列化按值打 tag），读回类型与列
//! 声明不符（「静默写坏」残余面）。
//!
//! 强制语义（design D3）：INSERT/UPDATE 在任何写入（数据页/WAL/版本链/索引）
//! 之前逐列校验值变体与列声明类型一致——NULL 豁免（NULL 性由 NOT NULL 门
//! 裁决）；日期族列的 String 经既有 coerce 强制解析后视为一致；FLOAT 列接受
//! 整数值并无损升格（就地改写 `Value::Float(n as f64)`）；其余跨类型组合以
//! `StorageError::ColumnTypeMismatch { column, expected, actual }` 点名拒绝
//! （exit 3，零副作用）。既有 NOT NULL / PK 键位门 / UNIQUE F1 守卫 /
//! DuplicateKey 预检的触发优先级与错误文本逐字节不变（一般类型门在其后，
//! 见 R2-S5 优先级锚点）。dump/restore/import 通道的类型化产出不误报
//! （R2-S6）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

fn expect_error(resp: Response, what: &str) -> String {
    match resp {
        Response::Error { message } => message,
        other => panic!("{what} 应被拒绝为 Error，实际 {other:?}"),
    }
}

fn expect_affected(resp: Response, what: &str) {
    match resp {
        Response::AffectedRows { .. } => {}
        other => panic!("{what} 应成功，实际 {other:?}"),
    }
}

fn query_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

async fn assert_count(db: &Database, sql: &str, expected: serde_json::Value, what: &str) {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], expected, "{what}");
        }
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

// ===========================================================================
// R2-S1：非键 INT 列拒绝 String 值（INSERT 与 UPDATE 双向，零副作用）
// ===========================================================================

/// R2-S1 INSERT：`v INT` 收 String 以 ColumnTypeMismatch 点名拒绝（列名、
/// 期望类型、实际类型三要素），零副作用。
///
/// RED（修复前实测）：String 值按 tag 静默落库为 `v='abc'`。
#[tokio::test]
async fn insert_int_column_rejects_string_value_zero_side_effects() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (1, 'abc')").await,
        "非键 INT 列 INSERT String 值",
    );
    assert!(
        message.contains("column 'v'") && message.contains("INT") && message.contains("String"),
        "错误文案必须点名列名、期望 INT 与实际 String，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R2-S1 UPDATE：`SET v = 'abc'` 以 ColumnTypeMismatch 点名拒绝，原行
/// 逐字节保持（零副作用）。
///
/// RED（修复前实测）：String 值静默落库覆盖原行。
#[tokio::test]
async fn update_int_column_rejects_string_value_zero_side_effects() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 100)").await,
        "首次 INSERT",
    );

    let message = expect_error(
        db.execute_sql("UPDATE t SET v = 'abc' WHERE id = 5").await,
        "非键 INT 列 UPDATE String 值",
    );
    assert!(
        message.contains("column 'v'") && message.contains("INT") && message.contains("String"),
        "错误文案必须点名列名、期望 INT 与实际 String，实际: {message}"
    );

    match db.execute_sql("SELECT id, v FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5), serde_json::json!(100)]],
                "被拒绝的 UPDATE 不得改动原行"
            );
        }
        other => panic!("Expected QueryResult，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// R2-S2：非键 STRING 列拒绝整数值
// ===========================================================================

/// R2-S2：`s STRING` 收 Int/Float 值以 ColumnTypeMismatch 点名拒绝，零副作用。
///
/// RED（修复前实测）：整数值按 tag 静默落库。
#[tokio::test]
async fn insert_string_column_rejects_int_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, s STRING)")
            .await,
        "建表",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (1, 42)").await,
        "非键 STRING 列 INSERT Int 值",
    );
    assert!(
        message.contains("column 's'") && message.contains("STRING") && message.contains("Int"),
        "错误文案必须点名列名、期望 STRING 与实际 Int，实际: {message}"
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (2, 4.5)").await,
        "非键 STRING 列 INSERT Float 值",
    );
    assert!(
        message.contains("column 's'") && message.contains("Float"),
        "错误文案必须点名实际 Float，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// R2-S3：FLOAT 列接受整数值并无损升格（INSERT 与 UPDATE 双向）
// ===========================================================================

/// R2-S3 INSERT：`f FLOAT` 收 Int 值升格为 Float 落库，读回 5.0（非 Int 5）。
///
/// RED（修复前实测）：Int 值按 tag 落库，读回 json!(5)。
#[tokio::test]
async fn insert_float_column_upgrades_int_value_losslessly() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, f FLOAT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 5)").await,
        "FLOAT 列 INSERT Int 值",
    );

    let rows = query_rows(
        db.execute_sql("SELECT f FROM t WHERE id = 1").await,
        "升格读回",
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(5.0)]],
        "Int 值必须无损升格为 Float 落库（读回 5.0），实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S3 UPDATE：`SET f = 5` 升格为 Float，读回 5.0。
///
/// RED（修复前实测）：Int 值按 tag 落库，读回 json!(5)。
#[tokio::test]
async fn update_float_column_upgrades_int_value_losslessly() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, f FLOAT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 1.5)").await,
        "首次 INSERT",
    );
    expect_affected(
        db.execute_sql("UPDATE t SET f = 5 WHERE id = 1").await,
        "FLOAT 列 UPDATE Int 值",
    );

    let rows = query_rows(
        db.execute_sql("SELECT f FROM t WHERE id = 1").await,
        "升格读回",
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(5.0)]],
        "Int 值必须无损升格为 Float 落库（读回 5.0），实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// R2-S4：日期族 String 经既有强制解析保持一致；非法日期维持既有拒绝
// ===========================================================================

/// R2-S4：`d DATE` 收合法 String 经 coerce 落 DATE（门不拒绝）；非法日期
/// 仍被既有 InvalidDateTime 通道拒绝（非 ColumnTypeMismatch）。
///
/// 锚点（合法分支变更前后恒 GREEN——coerce 先于类型门）。
#[tokio::test]
async fn insert_date_column_string_coerces_and_illegal_date_still_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, d DATE)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (1, '2026-09-25')")
            .await,
        "DATE 列 INSERT 合法 String",
    );

    let rows = query_rows(
        db.execute_sql("SELECT d FROM t WHERE id = 1").await,
        "coerce 读回",
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!("2026-09-25")]],
        "String 必须经 coerce 落 DATE 类型，实际: {rows:?}"
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (2, '2023-02-29')")
            .await,
        "DATE 列 INSERT 非法日期 String",
    );
    assert!(
        message.contains("invalid DATE value"),
        "非法日期必须维持既有 InvalidDateTime 文案，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(1),
        "非法日期行不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// R2-S5：既有错误面优先级与文本逐字节不变（一般类型门不提前触发）+ NULL 豁免
// ===========================================================================

/// R2-S5 锚点：Int 键列收 String 仍走既有 PK 键位门（KeyTypeMismatch 文本，
/// 非 ColumnTypeMismatch）。
#[tokio::test]
async fn int_pk_gate_takes_precedence_over_general_type_gate() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES ('abc', 1)").await,
        "Int 键列 INSERT String 值",
    );
    assert!(
        message.contains("key column 'id'"),
        "键列越界必须维持既有 KeyTypeMismatch 文案，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R2-S5 锚点：UPDATE SET 键列为 String 仍走既有 PK 键位门文案。
#[tokio::test]
async fn update_pk_gate_takes_precedence_over_general_type_gate() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 100)").await,
        "首次 INSERT",
    );

    let message = expect_error(
        db.execute_sql("UPDATE t SET id = 'x' WHERE id = 5").await,
        "UPDATE SET 键列为 String",
    );
    assert!(
        message.contains("key column 'id'"),
        "键列越界必须维持既有 KeyTypeMismatch 文案，实际: {message}"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S5 锚点：INT 唯一列收 String 仍走既有 F1 守卫（KeyTypeMismatch 文本，
/// 先于一般类型门）；唯一列收合法 Int 而非键列收越界值时一般类型门触发。
#[tokio::test]
async fn unique_f1_guard_takes_precedence_over_general_type_gate() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, u INT UNIQUE, v INT)")
            .await,
        "建表",
    );

    // 唯一列越界：F1 守卫先行（既有文本）
    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (1, 'abc', 2)").await,
        "INT 唯一列 INSERT String 值",
    );
    assert!(
        message.contains("key column 'u'"),
        "唯一列越界必须维持既有 F1 KeyTypeMismatch 文案，实际: {message}"
    );

    // 唯一列合法、非键列越界：一般类型门触发
    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (1, 5, 'xyz')").await,
        "非键列 INSERT String 值（唯一列合法）",
    );
    assert!(
        message.contains("column 'v'") && message.contains("INT"),
        "非键列越界必须由一般类型门点名，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R2-S5 锚点：NOT NULL 门文本与优先级不变——NULL 写入 NOT NULL 列仍由既有
/// 门拒绝（先于类型门）；可空列 NULL 豁免（INSERT 与 UPDATE）。
#[tokio::test]
async fn not_null_gate_unchanged_and_null_exempt() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT, s STRING NOT NULL)")
            .await,
        "建表",
    );

    // NOT NULL 既有门：文本不变
    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (1, NULL, NULL)").await,
        "NOT NULL 列 INSERT NULL",
    );
    assert!(
        message.contains("NOT NULL constraint violation: column 's'"),
        "NOT NULL 拒绝必须维持既有文案，实际: {message}"
    );

    // NULL 豁免：可空列收 NULL 成功
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (1, NULL, 'x')").await,
        "可空列 INSERT NULL",
    );
    // UPDATE SET 可空列 = NULL 既有语义保持
    expect_affected(
        db.execute_sql("UPDATE t SET v = NULL WHERE id = 1").await,
        "可空列 UPDATE NULL",
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(1),
        "NULL 豁免行落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R2-S5 锚点：重复键预检先于一般类型门——重复 PK + 跨类型值的组合仍报
/// 既有 Duplicate key（与既有行为一致）。
#[tokio::test]
async fn duplicate_key_precedes_general_type_gate() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (1, 10)").await,
        "首次 INSERT",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (1, 'abc')").await,
        "重复 PK + 跨类型值 INSERT",
    );
    assert!(
        message.contains("Duplicate key"),
        "重复键预检必须先于一般类型门（既有行为），实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(1),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// R2-S6：全通道覆盖——dump/restore 与 import 通道的类型化产出不误报
// ===========================================================================

/// R2-S6：dump 通道形态——dump 按列类型生成类型化字面量（日期族 typed
/// 字面量、其余 SQL 字面量），经 restore 通路（普通 INSERT 管道）执行时
/// 类型门不误报，全类型族值逐列保真。
#[tokio::test]
async fn dump_shaped_typed_literals_insert_without_false_positive() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql(
            "CREATE TABLE t (id INT PRIMARY KEY, v INT, f FLOAT, b BOOL, d DATE, ts TIMESTAMP, s STRING)",
        )
        .await,
        "建表",
    );

    // dump 生成的 INSERT 文本形态：Int→`2`、Float→`3.5`、Bool→`TRUE`、
    // Date/Timestamp→typed 字面量、String→单引号字面量
    expect_affected(
        db.execute_sql(
            "INSERT INTO t VALUES (1, 2, 3.5, TRUE, DATE '2026-09-25', TIMESTAMP '2026-09-25 10:30:00.000000', 'hi')",
        )
        .await,
        "dump 形态 INSERT",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, v, f, b, d, ts, s FROM t").await,
        "全列读回",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(1),
            serde_json::json!(2),
            serde_json::json!(3.5),
            serde_json::json!(true),
            serde_json::json!("2026-09-25"),
            serde_json::json!("2026-09-25 10:30:00"),
            serde_json::json!("hi"),
        ]],
        "dump 形态值必须逐列保真，实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S6：import 通道形态——`csv_value` 按列类型产出（日期族裸 String 透传
/// 经 coerce、空字段 NULL），类型门不误报。
#[tokio::test]
async fn import_shaped_values_insert_without_false_positive() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql(
            "CREATE TABLE t (id INT PRIMARY KEY, v INT, f FLOAT, b BOOL, d DATE, ts TIMESTAMP, s STRING)",
        )
        .await,
        "建表",
    );

    // import 生成的 INSERT 文本形态：日期族裸 String（写入时 coerce）、
    // 空字段 → NULL（Bool 位）
    expect_affected(
        db.execute_sql(
            "INSERT INTO t VALUES (2, 7, 2.5, NULL, '2026-09-25', '2026-09-25 10:30:00.000000', 'txt')",
        )
        .await,
        "import 形态 INSERT",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, v, f, b, d, ts, s FROM t").await,
        "全列读回",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(2),
            serde_json::json!(7),
            serde_json::json!(2.5),
            serde_json::json!(null),
            serde_json::json!("2026-09-25"),
            serde_json::json!("2026-09-25 10:30:00"),
            serde_json::json!("txt"),
        ]],
        "import 形态值必须逐列保真，实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}
