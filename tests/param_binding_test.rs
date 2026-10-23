//! MS31 Iteration 001（T5/T6/T7）：占位符与类型化参数执行。
//!
//! 见证面（delta spec `sql-parameter-binding` R1 七场景）：
//! - T5「AST 占位符计数与不可达位拒绝面」：裸 `?` 计数与编号（SQL 文本序）、
//!   `?N`/`$N` 形态点名拒绝、DDL 内占位符点名拒绝、标识位维持 parse 拒绝面、
//!   DML 定位键位点名拒绝、参数个数不符点名（期望 N 实际 M，零副作用）。
//! - T6「计划面占位符编译」：表达式位编译为参数表达式、DML 值位编译为槽位
//!   （清单→表列映射）、含参语句不入 plan cache、两次不同参数执行无值残留。
//! - T7「参数执行通道与错误/NULL/TEXT 真值矩阵」：等值过滤、值位绑定、类型
//!   门同源、NULL 三值、TEXT 真值（owned evaluate 通道）、显式事务内执行。
//!
//! 全部走真实入口（`Database::execute_with_params` / `execute_in_tx_with_params`）。

use rtsql::database::Database;
use rtsql::executor::Value;
use rtsql::{SqlError, SqlOutcome};
use tempfile::tempdir;

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

/// 取错误消息。
fn error_message(error: &SqlError) -> String {
    error.message.clone()
}

/// 建库 + 建表 + 播种（`id INTEGER PRIMARY KEY, k TEXT`）。
async fn seeded_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("params.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INTEGER PRIMARY KEY, k TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 'one')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (2, 'two')")
        .await
        .unwrap();
    (db, dir)
}

// ===== MS31 Iteration 001 / T5：占位符计数与不可达位拒绝面 =====

use rtsql::parser::ast::parse_sql;
use rtsql::parser::placeholder::{assign_placeholder_indices, count_placeholders};
use sqlparser::ast::Statement;

/// 解析单条语句（占位符面测试用）。
fn parse_one(sql: &str) -> Statement {
    let mut statements = parse_sql(sql).expect("parse");
    assert_eq!(statements.len(), 1, "expected single statement");
    statements.pop().unwrap()
}

/// 语句渲染文本中 `__param_{i}` 标记的出现序（索引序列）。
fn marker_order(stmt: &Statement) -> Vec<usize> {
    let text = stmt.to_string();
    let mut indices = Vec::new();
    let mut rest = text.as_str();
    while let Some(pos) = rest.find("__param_") {
        rest = &rest[pos + "__param_".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        indices.push(digits.parse().unwrap());
    }
    indices
}

#[test]
fn t5_bare_placeholder_count_is_ast_level() {
    assert_eq!(
        count_placeholders(&parse_one("SELECT a FROM t WHERE a = ?")),
        1
    );
    assert_eq!(
        count_placeholders(&parse_one("SELECT a FROM t WHERE a = ? AND b = ?")),
        2
    );
    assert_eq!(
        count_placeholders(&parse_one("INSERT INTO t VALUES (?, ?)")),
        2
    );
    assert_eq!(
        count_placeholders(&parse_one("SELECT a FROM t WHERE a = ? OR b IN (?, ?)")),
        3
    );
    // 无占位符语句计数为 0。
    assert_eq!(count_placeholders(&parse_one("SELECT a FROM t")), 0);
}

#[test]
fn t5_bare_placeholder_is_indexed_in_text_order() {
    // 投影位与谓词位分处两条子句——编号必须按 SQL 文本序（投影在前），
    // 不能按计划序（谓词先建）。
    let mut stmt = parse_one("SELECT ? FROM t WHERE a = ?");
    let count = assign_placeholder_indices(&mut stmt).expect("assign");
    assert_eq!(count, 2);
    assert_eq!(marker_order(&stmt), vec![0, 1]);
    // 渲染文本中投影位拿到 0 号、谓词位拿到 1 号。
    assert!(stmt.to_string().contains("SELECT __param_0"));
    assert!(stmt.to_string().contains("a = __param_1"));
}

#[test]
fn t5_numbered_placeholder_form_is_rejected_by_name() {
    // `?N` / `$N` 点名拒绝——只支持一套位置式方言。
    let mut stmt = parse_one("SELECT a FROM t WHERE a = ?1");
    let error = assign_placeholder_indices(&mut stmt).expect_err("must reject ?1");
    assert!(
        error.to_string().contains("?1"),
        "error must name the rejected form: {error}"
    );

    let mut stmt = parse_one("SELECT a FROM t WHERE a = $1");
    let error = assign_placeholder_indices(&mut stmt).expect_err("must reject $1");
    assert!(
        error.to_string().contains("$1"),
        "error must name the rejected form: {error}"
    );
}

#[test]
fn t5_ddl_placeholder_is_rejected_by_name() {
    // CTAS 源查询内的占位符——DDL 语句不承载参数，点名拒绝。
    let mut stmt = parse_one("CREATE TABLE t2 AS SELECT ?");
    let error = assign_placeholder_indices(&mut stmt).expect_err("must reject");
    assert!(
        error.to_string().contains("DDL"),
        "error must name the DDL boundary: {error}"
    );
}

#[test]
fn t5_identifier_position_placeholder_keeps_parse_rejection() {
    // 标识位占位符维持 sqlparser 既有 parse 拒绝面——解析期即拒绝，不静默。
    let error = parse_sql("SELECT a FROM ?").expect_err("must reject at parse");
    assert!(
        error.to_string().to_lowercase().contains("expected"),
        "unexpected parse error: {error}"
    );
}

// ===== MS31 Iteration 001 / T7：参数执行通道与错误/NULL/TEXT 真值矩阵 =====

#[tokio::test]
async fn t7_where_placeholder_filters_rows() {
    let (db, _dir) = seeded_db().await;
    let outcome = db
        .execute_with_params("SELECT id FROM t WHERE id = ?", &[Value::Int(2)])
        .await
        .unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::Int(2)]]);
}

#[tokio::test]
async fn t7_text_parameter_keeps_true_value() {
    let (db, _dir) = seeded_db().await;
    // TEXT 参数真值断言——借用通道降级会把 String 打成 NULL（静默空结果）。
    let outcome = db
        .execute_with_params(
            "SELECT id FROM t WHERE k = ?",
            &[Value::String("one".into())],
        )
        .await
        .unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn t7_projection_placeholder_returns_parameter_value() {
    let (db, _dir) = seeded_db().await;
    let outcome = db
        .execute_with_params(
            "SELECT ? FROM t WHERE id = ?",
            &[Value::Int(7), Value::Int(1)],
        )
        .await
        .unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::Int(7)]]);
}

#[tokio::test]
async fn t7_insert_values_bind_parameters() {
    let (db, _dir) = seeded_db().await;
    let outcome = db
        .execute_with_params(
            "INSERT INTO t VALUES (?, ?)",
            &[Value::Int(3), Value::String("three".into())],
        )
        .await
        .unwrap();
    assert_eq!(affected(&outcome), 1);
    let rows_out = rows(
        &db.execute_sql("SELECT id, k FROM t WHERE id = 3")
            .await
            .unwrap(),
    );
    assert_eq!(
        rows_out,
        vec![vec![Value::Int(3), Value::String("three".to_string())]]
    );
}

#[tokio::test]
async fn t7_update_set_value_binds_parameter() {
    let (db, _dir) = seeded_db().await;
    let outcome = db
        .execute_with_params(
            "UPDATE t SET k = ? WHERE id = 1",
            &[Value::String("uno".into())],
        )
        .await
        .unwrap();
    assert_eq!(affected(&outcome), 1);
    let rows_out = rows(
        &db.execute_sql("SELECT k FROM t WHERE id = 1")
            .await
            .unwrap(),
    );
    assert_eq!(rows_out, vec![vec![Value::String("uno".to_string())]]);
}

#[tokio::test]
async fn t7_dml_key_placeholder_is_rejected() {
    let (db, _dir) = seeded_db().await;
    // UPDATE 定位键位——点名不可达原因（定位键为计划期物化）。
    let error = db
        .execute_with_params("UPDATE t SET k = 'x' WHERE id = ?", &[Value::Int(1)])
        .await
        .expect_err("must reject");
    let message = error_message(&error);
    assert!(
        message.contains("parameterized primary key"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("plan time"),
        "error must name the unreachable reason: {message}"
    );

    // DELETE 定位键位——同因点名。
    let error = db
        .execute_with_params("DELETE FROM t WHERE id = ?", &[Value::Int(1)])
        .await
        .expect_err("must reject");
    let message = error_message(&error);
    assert!(
        message.contains("parameterized primary key"),
        "unexpected error: {message}"
    );
}

#[tokio::test]
async fn t7_parameter_count_mismatch_is_named() {
    let (db, _dir) = seeded_db().await;
    // 期望 2 实际 1。
    let error = db
        .execute_with_params("SELECT id FROM t WHERE id = ? OR k = ?", &[Value::Int(1)])
        .await
        .expect_err("must reject");
    let message = error_message(&error);
    assert!(
        message.contains("expected 2") && message.contains("got 1"),
        "error must name expected/actual counts: {message}"
    );

    // 无参路径等价 `params = &[]`——同一个数错误（期望 2 实际 0）。
    let error = db
        .execute_sql("SELECT id FROM t WHERE id = ? OR k = ?")
        .await
        .expect_err("must reject");
    let message = error_message(&error);
    assert!(
        message.contains("expected 2") && message.contains("got 0"),
        "error must name expected/actual counts: {message}"
    );
}

#[tokio::test]
async fn t7_parameter_type_mismatch_matches_literal_gate() {
    let (db, _dir) = seeded_db().await;
    // 字面量写入的类型门文案（基准）。
    let literal = db
        .execute_sql("INSERT INTO t(id, k) VALUES (9, 1)")
        .await
        .expect_err("literal must be rejected");
    // 参数写入走同一道门——文案逐字节一致。
    let parameterized = db
        .execute_with_params("INSERT INTO t(id, k) VALUES (9, ?)", &[Value::Int(1)])
        .await
        .expect_err("parameter must be rejected");
    assert_eq!(error_message(&literal), error_message(&parameterized));
    assert!(
        error_message(&parameterized).contains("expects STRING"),
        "unexpected error: {}",
        error_message(&parameterized)
    );
}

#[tokio::test]
async fn t7_null_parameter_keeps_three_value_semantics() {
    let (db, _dir) = seeded_db().await;
    // `= NULL` 是 UNKNOWN——不匹配任何行（三值语义，不退化为「等于 NULL」）。
    let outcome = db
        .execute_with_params("SELECT id FROM t WHERE id = ?", &[Value::Null])
        .await
        .unwrap();
    assert!(rows(&outcome).is_empty());
    // IS NULL 形态不受影响（NULL 参数不改变 IS NULL 的判定）。
    let outcome = db
        .execute_with_params(
            "SELECT id FROM t WHERE k IS NULL AND id <> ?",
            &[Value::Int(1)],
        )
        .await
        .unwrap();
    assert!(rows(&outcome).is_empty());
}

#[tokio::test]
async fn t7_unreachable_placeholder_positions_are_rejected() {
    let (db, _dir) = seeded_db().await;
    // DDL 语句内占位符——点名拒绝。
    let error = db
        .execute_with_params("CREATE TABLE t2 AS SELECT ?", &[Value::Int(1)])
        .await
        .expect_err("must reject");
    assert!(
        error_message(&error).contains("DDL"),
        "error must name the DDL boundary: {}",
        error_message(&error)
    );

    // 非裸占位符形态——点名拒绝。
    let error = db
        .execute_with_params("SELECT id FROM t WHERE id = ?1", &[Value::Int(1)])
        .await
        .expect_err("must reject");
    assert!(
        error_message(&error).contains("?1"),
        "error must name the rejected form: {}",
        error_message(&error)
    );
}

#[tokio::test]
async fn t7_parameterized_statement_does_not_enter_plan_cache() {
    let (db, _dir) = seeded_db().await;
    db.execute_with_params("SELECT id FROM t WHERE id = ?", &[Value::Int(1)])
        .await
        .unwrap();
    assert_eq!(
        db.plan_cache_len(),
        0,
        "parameterized plan must not be cached"
    );
    // 无占位符语句照常进缓存（既有纪律不变）。
    db.execute_sql("SELECT id FROM t").await.unwrap();
    assert_eq!(db.plan_cache_len(), 1);
}

#[tokio::test]
async fn t7_in_tx_with_params_binds_values() {
    let (db, _dir) = seeded_db().await;
    let tx = db.begin().await.unwrap();
    let outcome = db
        .execute_in_tx_with_params(
            "INSERT INTO t VALUES (?, ?)",
            &[Value::Int(4), Value::String("four".into())],
            &tx,
        )
        .await
        .unwrap();
    assert_eq!(affected(&outcome), 1);
    db.commit(tx).await.unwrap();
    let rows_out = rows(
        &db.execute_sql("SELECT k FROM t WHERE id = 4")
            .await
            .unwrap(),
    );
    assert_eq!(rows_out, vec![vec![Value::String("four".to_string())]]);
}

#[tokio::test]
async fn t7_upsert_value_and_do_update_bind_parameters() {
    let (db, _dir) = seeded_db().await;
    // 待插值位参数绑定。
    let outcome = db
        .execute_with_params(
            "INSERT INTO t VALUES (?, ?) ON CONFLICT (id) DO NOTHING",
            &[Value::Int(5), Value::String("five".into())],
        )
        .await
        .unwrap();
    assert_eq!(affected(&outcome), 1);

    // DO UPDATE SET 右值位参数绑定（冲突行原位更新）。
    let outcome = db
        .execute_with_params(
            "INSERT INTO t VALUES (?, ?) ON CONFLICT (id) DO UPDATE SET k = ?",
            &[
                Value::Int(5),
                Value::String("five".into()),
                Value::String("FIVE".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(affected(&outcome), 1);
    let rows_out = rows(
        &db.execute_sql("SELECT k FROM t WHERE id = 5")
            .await
            .unwrap(),
    );
    assert_eq!(rows_out, vec![vec![Value::String("FIVE".to_string())]]);
}

#[tokio::test]
async fn t7_insert_column_list_maps_parameter_slots_to_table_columns() {
    let (db, _dir) = seeded_db().await;
    // 清单逆序——参数按清单位绑定，不错位到表列位。
    db.execute_with_params(
        "INSERT INTO t(k, id) VALUES (?, ?)",
        &[Value::String("six".into()), Value::Int(6)],
    )
    .await
    .unwrap();
    let rows_out = rows(
        &db.execute_sql("SELECT id, k FROM t WHERE id = 6")
            .await
            .unwrap(),
    );
    assert_eq!(
        rows_out,
        vec![vec![Value::Int(6), Value::String("six".to_string())]]
    );
}

#[tokio::test]
async fn t7_insert_select_source_placeholder_binds() {
    let (db, _dir) = seeded_db().await;
    // INSERT SELECT 的源谓词位参数绑定。
    let outcome = db
        .execute_with_params(
            "INSERT INTO t SELECT id + 10, k FROM t WHERE id = ?",
            &[Value::Int(2)],
        )
        .await
        .unwrap();
    assert_eq!(affected(&outcome), 1);
    let rows_out = rows(
        &db.execute_sql("SELECT id FROM t WHERE id = 12")
            .await
            .unwrap(),
    );
    assert_eq!(rows_out, vec![vec![Value::Int(12)]]);
}

#[tokio::test]
async fn t7_join_on_placeholder_is_not_silently_ignored() {
    let (db, _dir) = seeded_db().await;
    db.execute_sql("CREATE TABLE u (id INTEGER, k TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO u VALUES (1, 'x')")
        .await
        .unwrap();
    // JOIN ON 的非列等值形态（含占位符）不被静默忽略——要么按等值条件路由，
    // 要么点名拒绝。
    let outcome = db
        .execute_with_params(
            "SELECT t.id FROM t JOIN u ON t.id = u.id WHERE t.id = ?",
            &[Value::Int(1)],
        )
        .await
        .unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn t7_order_by_alias_key_with_parameter_keeps_value() {
    let (db, _dir) = seeded_db().await;
    // 排序键别名指向含参数的表达式项——排序键求值不得退化为常量 NULL。
    let outcome = db
        .execute_with_params(
            "SELECT id, ? AS k FROM t ORDER BY k",
            &[Value::String("one".into())],
        )
        .await
        .expect("alias key over a parameter must be reachable");
    let values: Vec<Value> = rows(&outcome).into_iter().flatten().collect();
    assert!(
        values.contains(&Value::String("one".to_string())),
        "projection must carry the parameter value: {values:?}"
    );
}

#[tokio::test]
async fn t5_parameter_count_mismatch_has_zero_side_effect() {
    let (db, _dir) = seeded_db().await;
    // 个数不符在计划前拒绝——不产生任何写入副作用。
    let error = db
        .execute_with_params("INSERT INTO t VALUES (?, ?)", &[Value::Int(3)])
        .await
        .expect_err("must reject");
    assert!(
        error_message(&error).contains("expected 2") && error_message(&error).contains("got 1"),
        "error must name expected/actual counts: {}",
        error_message(&error)
    );
    // 表内容零变化（无插入行）。
    let outcome = db.execute_sql("SELECT id FROM t").await.unwrap();
    assert_eq!(rows(&outcome).len(), 2);
}

#[tokio::test]
async fn t6_same_sql_different_params_leave_no_residue() {
    let (db, _dir) = seeded_db().await;
    // 同一 SQL 文本、不同参数值——两次执行结果各自正确（前次参数值不残留）。
    let first = db
        .execute_with_params("SELECT id FROM t WHERE id = ?", &[Value::Int(1)])
        .await
        .unwrap();
    assert_eq!(rows(&first), vec![vec![Value::Int(1)]]);
    let second = db
        .execute_with_params("SELECT id FROM t WHERE id = ?", &[Value::Int(2)])
        .await
        .unwrap();
    assert_eq!(rows(&second), vec![vec![Value::Int(2)]]);
    let third = db
        .execute_with_params("SELECT id FROM t WHERE id = ?", &[Value::Int(1)])
        .await
        .unwrap();
    assert_eq!(rows(&third), vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn t7_not_null_violation_via_parameter_matches_literal_gate() {
    let (db, _dir) = seeded_db().await;
    db.execute_sql("CREATE TABLE n (id INTEGER PRIMARY KEY, k TEXT NOT NULL)")
        .await
        .unwrap();
    // 字面量 NULL 写入（基准文案）。
    let literal = db
        .execute_sql("INSERT INTO n VALUES (1, NULL)")
        .await
        .expect_err("literal must be rejected");
    // 参数 NULL 写入——同一道门（NOT NULL 约束在真实值上跑）。
    let parameterized = db
        .execute_with_params("INSERT INTO n VALUES (?, ?)", &[Value::Int(1), Value::Null])
        .await
        .expect_err("parameter must be rejected");
    assert_eq!(error_message(&literal), error_message(&parameterized));
    assert!(
        error_message(&parameterized).contains("NOT NULL"),
        "unexpected error: {}",
        error_message(&parameterized)
    );
}

#[tokio::test]
async fn t7_top_level_projection_keeps_text_parameter_truth() {
    let (db, _dir) = seeded_db().await;
    // 顶层投影位的 TEXT 参数真值——借用通道降级会把 String 打成 NULL。
    let outcome = db
        .execute_with_params("SELECT ?", &[Value::String("hello".into())])
        .await
        .unwrap();
    assert_eq!(
        rows(&outcome),
        vec![vec![Value::String("hello".to_string())]]
    );
}

#[tokio::test]
async fn t7_parameter_write_maintains_unique_and_secondary_indexes() {
    let (db, _dir) = seeded_db().await;
    // 唯一约束面为 INT 列（本项目既有约束纪律）。
    db.execute_sql("CREATE TABLE ix (id INTEGER PRIMARY KEY, k INTEGER UNIQUE, j INTEGER)")
        .await
        .unwrap();
    db.execute_sql("CREATE INDEX ix_j ON ix (j)").await.unwrap();
    // 带参写入——唯一索引与次级索引维护照常（与字面量语句同源）。
    db.execute_with_params(
        "INSERT INTO ix VALUES (?, ?, ?)",
        &[Value::Int(1), Value::Int(10), Value::Int(7)],
    )
    .await
    .unwrap();
    // 唯一索引可见：同键再写被拒。
    let error = db
        .execute_with_params(
            "INSERT INTO ix VALUES (?, ?, ?)",
            &[Value::Int(2), Value::Int(10), Value::Int(8)],
        )
        .await
        .expect_err("unique key must be enforced");
    assert!(
        error_message(&error).contains("UNIQUE") || error_message(&error).contains("Duplicate"),
        "unexpected error: {}",
        error_message(&error)
    );
    // 次级索引可见：按次级键回查命中参数写入的行。
    let outcome = db
        .execute_with_params("SELECT id FROM ix WHERE j = ?", &[Value::Int(7)])
        .await
        .unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn t7_rejections_have_zero_side_effect() {
    let (db, _dir) = seeded_db().await;
    // DML 定位键位拒绝——行零变化。
    db.execute_with_params("UPDATE t SET k = 'x' WHERE id = ?", &[Value::Int(1)])
        .await
        .expect_err("must reject");
    let outcome = db
        .execute_sql("SELECT k FROM t WHERE id = 1")
        .await
        .unwrap();
    assert_eq!(rows(&outcome), vec![vec![Value::String("one".to_string())]]);

    // 类型门拒绝——行零落库。
    db.execute_with_params(
        "INSERT INTO t(id, k) VALUES (?, ?)",
        &[Value::Int(9), Value::Int(1)],
    )
    .await
    .expect_err("must reject");
    let outcome = db
        .execute_sql("SELECT id FROM t WHERE id = 9")
        .await
        .unwrap();
    assert!(rows(&outcome).is_empty());
}
