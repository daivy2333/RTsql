//! MS31 Iteration 000 / T2: 类型化公共执行结果（`typed-library-results` R1）。
//!
//! 见证面：
//! - R1「SELECT 返回原生值与列元数据」：查询经类型化入口返回原生 `Value` 行，
//!   逐列元数据的列名与 SELECT 清单一致、`SELECT *` 全列序；
//! - R1「DML 受影响行数」：INSERT/UPDATE/DELETE 返回受影响行数；
//! - R1「错误载荷同源」：未知列 42703、未知表 42P01、语法错 42601 的消息与
//!   SQLSTATE 分类同源既有错误面。
//!
//! T3 追加列类型断言（表达式列元数据 / 声明类型 / 值推导兜底）。

use rtsql::database::Database;
use rtsql::executor::Value;
use rtsql::storage::ColumnType as StorageColumnType;
use rtsql::{ColumnMeta, SqlError, SqlOutcome};
use tempfile::tempdir;

/// 取查询结果的列名序。
fn column_names(outcome: &SqlOutcome) -> Vec<String> {
    match outcome {
        SqlOutcome::Rows { columns, .. } => columns.iter().map(|c| c.name.clone()).collect(),
        other => panic!("expected Rows, got {other:?}"),
    }
}

/// 取查询结果的原生行。
fn rows(outcome: &SqlOutcome) -> Vec<Vec<Value>> {
    match outcome {
        SqlOutcome::Rows { rows, .. } => rows.clone(),
        other => panic!("expected Rows, got {other:?}"),
    }
}

/// 取写面结果的受影响行数。
fn affected(outcome: &SqlOutcome) -> u64 {
    match outcome {
        SqlOutcome::AffectedRows(count) => *count,
        other => panic!("expected AffectedRows, got {other:?}"),
    }
}

/// 取错误载荷。
fn expect_error(outcome: std::result::Result<SqlOutcome, SqlError>) -> SqlError {
    match outcome {
        Ok(other) => panic!("expected an error, got Ok({other:?})"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn select_returns_native_values_and_column_metadata() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER, b TEXT, f FLOAT, flag BOOL, d DATE)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 'hello', 2.5, TRUE, DATE '2024-01-15')")
        .await
        .unwrap();

    let outcome = db
        .execute_sql("SELECT a, b, f, flag, d FROM t")
        .await
        .unwrap();
    // 列名元数据与 SELECT 清单一致（单源：既有计划输出列）。
    assert_eq!(column_names(&outcome), vec!["a", "b", "f", "flag", "d"]);
    // 原生值变体——未经 JSON 往返。
    assert_eq!(
        rows(&outcome),
        vec![vec![
            Value::Int(1),
            Value::String("hello".to_string()),
            Value::Float(2.5),
            Value::Bool(true),
            Value::Date(738_899),
        ]]
    );
}

#[tokio::test]
async fn select_star_returns_full_column_order() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER, b TEXT, c INTEGER)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 'x', 2)")
        .await
        .unwrap();

    let outcome = db.execute_sql("SELECT * FROM t").await.unwrap();
    assert_eq!(column_names(&outcome), vec!["a", "b", "c"]);
    assert_eq!(
        rows(&outcome),
        vec![vec![
            Value::Int(1),
            Value::String("x".to_string()),
            Value::Int(2),
        ]]
    );
}

#[tokio::test]
async fn dml_returns_affected_row_count() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER PRIMARY KEY, b TEXT)")
        .await
        .unwrap();

    let inserted = db
        .execute_sql("INSERT INTO t VALUES (1, 'x')")
        .await
        .unwrap();
    assert_eq!(affected(&inserted), 1);

    let updated = db
        .execute_sql("UPDATE t SET b = 'y' WHERE a = 1")
        .await
        .unwrap();
    assert_eq!(affected(&updated), 1);

    let deleted = db.execute_sql("DELETE FROM t WHERE a = 1").await.unwrap();
    assert_eq!(affected(&deleted), 1);
}

#[tokio::test]
async fn error_payload_carries_message_and_sqlstate() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER)").await.unwrap();

    let unknown_column = expect_error(db.execute_sql("SELECT missing_col FROM t").await);
    assert!(
        unknown_column.message.contains("missing_col"),
        "unknown column message must name the column: {}",
        unknown_column.message
    );
    assert_eq!(unknown_column.sqlstate, Some("42703".to_string()));

    let unknown_table = expect_error(db.execute_sql("SELECT * FROM no_such_table").await);
    assert_eq!(unknown_table.sqlstate, Some("42P01".to_string()));

    let syntax_error = expect_error(db.execute_sql("SELECT FROM WHERE").await);
    assert_eq!(syntax_error.sqlstate, Some("42601".to_string()));
}

#[tokio::test]
async fn empty_query_result_keeps_column_metadata() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER, b TEXT)")
        .await
        .unwrap();

    let outcome = db.execute_sql("SELECT a, b FROM t").await.unwrap();
    assert_eq!(column_names(&outcome), vec!["a", "b"]);
    assert!(rows(&outcome).is_empty());
}

#[tokio::test]
async fn column_meta_carries_name_and_declared_type() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER)").await.unwrap();

    let outcome = db.execute_sql("SELECT a FROM t").await.unwrap();
    match outcome {
        SqlOutcome::Rows { columns, .. } => {
            let meta: &ColumnMeta = &columns[0];
            assert_eq!(meta.name, "a");
            // 声明类型经计划 walk 取得——空结果同样报声明类型。
            assert_eq!(meta.column_type, Some(StorageColumnType::Int));
        }
        other => panic!("expected Rows, got {other:?}"),
    }
}

// ---- T3：列类型装配（声明类型 walk + 值推导兜底）----

/// 取逐列类型。
fn column_types(outcome: &SqlOutcome) -> Vec<Option<StorageColumnType>> {
    match outcome {
        SqlOutcome::Rows { columns, .. } => columns.iter().map(|c| c.column_type.clone()).collect(),
        other => panic!("expected Rows, got {other:?}"),
    }
}

/// T3：声明列的类型经计划 walk 取得——INT/TEXT 声明列报声明类型。
#[tokio::test]
async fn declared_column_types_come_from_table_metadata() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER, b TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 'x')")
        .await
        .unwrap();

    let outcome = db.execute_sql("SELECT a, b FROM t").await.unwrap();
    assert_eq!(
        column_types(&outcome),
        vec![
            Some(StorageColumnType::Int),
            Some(StorageColumnType::String(255))
        ]
    );
}

/// T3：日期族声明列报声明类型。
#[tokio::test]
async fn declared_datetime_types_come_from_table_metadata() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (d DATE, ts TIMESTAMP)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (DATE '2024-01-15', TIMESTAMP '2024-01-15 10:00:00')")
        .await
        .unwrap();

    let outcome = db.execute_sql("SELECT d, ts FROM t").await.unwrap();
    assert_eq!(
        column_types(&outcome),
        vec![
            Some(StorageColumnType::Date),
            Some(StorageColumnType::Timestamp)
        ]
    );
}

/// T3：空结果的声明列仍报声明类型（声明来自计划 walk，不依赖结果行）。
#[tokio::test]
async fn empty_result_keeps_declared_column_types() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER, b TEXT)")
        .await
        .unwrap();

    let outcome = db.execute_sql("SELECT a, b FROM t").await.unwrap();
    assert!(rows(&outcome).is_empty());
    assert_eq!(
        column_types(&outcome),
        vec![
            Some(StorageColumnType::Int),
            Some(StorageColumnType::String(255))
        ]
    );
}

/// T3：表达式列的类型经结果行值推导——`a + 1` 报 Int。
#[tokio::test]
async fn expression_column_type_is_derived_from_values() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER)").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let outcome = db.execute_sql("SELECT a + 1 FROM t").await.unwrap();
    assert_eq!(column_types(&outcome), vec![Some(StorageColumnType::Int)]);
}

/// T3：字符串表达式列的类型经结果行值推导——`COALESCE(b, 'x')` 报 String。
#[tokio::test]
async fn string_expression_column_type_is_derived_from_values() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (b TEXT)").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (NULL)").await.unwrap();

    let outcome = db
        .execute_sql("SELECT COALESCE(b, 'x') FROM t")
        .await
        .unwrap();
    assert_eq!(
        column_types(&outcome),
        vec![Some(StorageColumnType::String(255))]
    );
}

/// T3：表达式列无结果行时类型不可得——保持 None（不虚构）。
#[tokio::test]
async fn expression_column_without_rows_has_no_type() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER)").await.unwrap();

    let outcome = db.execute_sql("SELECT a + 1 FROM t").await.unwrap();
    assert!(rows(&outcome).is_empty());
    assert_eq!(column_types(&outcome), vec![None]);
}

/// T3：全 NULL 的表达式列类型不可得——保持 None（不虚构）。
#[tokio::test]
async fn all_null_expression_column_has_no_type() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER)").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let outcome = db.execute_sql("SELECT NULL AS x FROM t").await.unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::Null]]);
    assert_eq!(column_types(&outcome), vec![None]);
}

/// T3：聚合列的类型经结果行值推导——`MAX(a)` 报 Int。
#[tokio::test]
async fn aggregate_column_type_is_derived_from_values() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (a INTEGER)").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (7)").await.unwrap();

    let outcome = db.execute_sql("SELECT MAX(a) FROM t").await.unwrap();
    assert_eq!(column_types(&outcome), vec![Some(StorageColumnType::Int)]);
}

/// T3：JOIN 输出列按 (表, 列) 取声明类型。
#[tokio::test]
async fn join_columns_carry_declared_types() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE a (id INTEGER, ax TEXT)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE b (id INTEGER, by TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO a VALUES (1, 'x')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO b VALUES (1, 'y')")
        .await
        .unwrap();

    let outcome = db
        .execute_sql("SELECT a.id, b.by FROM a JOIN b ON a.id = b.id")
        .await
        .unwrap();
    assert_eq!(
        column_types(&outcome),
        vec![
            Some(StorageColumnType::Int),
            Some(StorageColumnType::String(255))
        ]
    );
}

/// T3：表别名形态的 JOIN 输出列仍取声明类型（限定符不可解析时按列名兜底）。
#[tokio::test]
async fn aliased_join_columns_carry_declared_types() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE a (id INTEGER, ax TEXT)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE b (id INTEGER, by TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO a VALUES (1, 'x')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO b VALUES (1, 'y')")
        .await
        .unwrap();

    let outcome = db
        .execute_sql("SELECT x.id, y.by FROM a AS x JOIN b AS y ON x.id = y.id")
        .await
        .unwrap();
    assert_eq!(
        column_types(&outcome),
        vec![
            Some(StorageColumnType::Int),
            Some(StorageColumnType::String(255))
        ]
    );
}

/// R-b：别名 JOIN 的列类型与实际值一致——别名指向后位表时报告**该表**的声明
/// 类型（不取前位候选表的同名列类型），元数据与行值不矛盾。
#[tokio::test]
async fn aliased_join_column_type_matches_the_actual_value() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("typed.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t1 (id INTEGER, k INTEGER)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t2 (id INTEGER, k TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t1 VALUES (1, 7)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t2 VALUES (1, 'seven')")
        .await
        .unwrap();

    let outcome = db
        .execute_sql("SELECT x.k FROM t1 JOIN t2 AS x ON t1.id = x.id")
        .await
        .unwrap();
    // 别名 x 指向 t2 —— 报告 t2.k 的声明类型（TEXT），不是前位候选表 t1.k
    // 的同名列类型（INTEGER）。
    assert_eq!(
        column_types(&outcome),
        vec![Some(StorageColumnType::String(255))]
    );
    // 元数据与实际行值一致。
    assert_eq!(
        rows(&outcome),
        vec![vec![Value::String("seven".to_string())]]
    );
}
