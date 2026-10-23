//! MS31 Iteration 002（T9/T10）：schema 版本戳与可复用预编译语句。
//!
//! 见证面（delta spec `sql-parameter-binding` R2 四场景 + T9 版本戳单点）：
//! - T9「schema 版本戳单点」：DDL 执行后版本递增且 plan cache 归零；非 DDL
//!   语句版本不变；版本在 `Database` 克隆间共享。
//! - T10「一次 prepare 多次执行」：同一预编译语句以不同参数多次执行，
//!   写入与读回逐条正确，且与直接执行结果一致。
//! - T10「schema 变化后失效契约」：DDL（ALTER / DROP / 建删索引）后透明
//!   re-plan，结果与直接执行一致。
//! - T10「显式事务内执行」：事务内参数化 DML 的提交 / 回滚语义正确。
//! - T10「并发共享安全」：同一预编译语句经多任务并发带参执行结果正确。
//!
//! 全部走真实入口（`Database::prepare` + `PreparedStatement::execute[_in_tx]`）。

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

/// 取错误 SQLSTATE。
fn error_sqlstate(error: &SqlError) -> Option<String> {
    error.sqlstate.clone()
}

/// 建库（未建表）。
async fn empty_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("prepared.db"))
        .await
        .unwrap();
    (db, dir)
}

/// 建库 + 建表 `t (id INTEGER PRIMARY KEY, k TEXT)`。
async fn seeded_db() -> (Database, tempfile::TempDir) {
    let (db, dir) = empty_db().await;
    db.execute_sql("CREATE TABLE t (id INTEGER PRIMARY KEY, k TEXT)")
        .await
        .unwrap();
    (db, dir)
}

// ===== MS31 Iteration 002 / T9：schema 版本戳单点 =====

/// T9：DDL 执行后版本戳递增、plan cache 归零——版本与缓存失效同源。
#[tokio::test]
async fn t9_ddl_execution_bumps_schema_version_and_clears_plan_cache() {
    let (db, _dir) = empty_db().await;
    assert_eq!(db.schema_version(), 0, "新库版本戳从 0 起");

    db.execute_sql("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    assert_eq!(db.schema_version(), 1, "CREATE TABLE 后版本戳递增");

    // 缓存里放一条 SELECT 计划——随后的 DDL 必须把它清掉。
    db.execute_sql("SELECT * FROM t").await.unwrap();
    assert_eq!(db.plan_cache_len(), 1, "SELECT 计划进入 plan cache");

    db.execute_sql("ALTER TABLE t ADD COLUMN k TEXT")
        .await
        .unwrap();
    assert_eq!(db.schema_version(), 2, "ALTER TABLE 后版本戳递增");
    assert_eq!(db.plan_cache_len(), 0, "DDL 执行后 plan cache 归零");

    db.execute_sql("DROP TABLE t").await.unwrap();
    assert_eq!(db.schema_version(), 3, "DROP TABLE 后版本戳递增");
    assert_eq!(db.plan_cache_len(), 0, "DDL 执行后 plan cache 归零");
}

/// T9：非 DDL 语句（SELECT / INSERT / UPDATE / DELETE）版本戳零变化。
#[tokio::test]
async fn t9_non_ddl_execution_keeps_schema_version() {
    let (db, _dir) = seeded_db().await;
    let before = db.schema_version();

    db.execute_sql("SELECT * FROM t").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 'one')")
        .await
        .unwrap();
    db.execute_sql("UPDATE t SET k = 'uno' WHERE id = 1")
        .await
        .unwrap();
    db.execute_sql("DELETE FROM t WHERE id = 1").await.unwrap();

    assert_eq!(db.schema_version(), before, "非 DDL 语句不递增版本戳");
}

/// T9：版本戳在 `Database` 克隆间共享（Arc 语义）——持有克隆的预编译语句
/// 能观测到 bump。
#[tokio::test]
async fn t9_schema_version_is_shared_across_database_clones() {
    let (db, _dir) = empty_db().await;
    let cloned = db.clone();
    assert_eq!(cloned.schema_version(), db.schema_version());

    db.execute_sql("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    assert_eq!(cloned.schema_version(), 1, "克隆观测到同一版本戳递增");
}

// ===== MS31 Iteration 002 / T10：预编译语句 =====

/// T10：一次 prepare、多次不同参数执行——写入与读回逐条正确，且与直接执行
/// 结果一致。
#[tokio::test]
async fn t10_prepare_once_execute_twice_with_different_params() {
    let (db, _dir) = seeded_db().await;
    let prepared = db.prepare("INSERT INTO t VALUES (?, ?)").await.unwrap();

    assert_eq!(
        affected(
            &prepared
                .execute(&[Value::Int(1), Value::String("one".into())])
                .await
                .unwrap()
        ),
        1,
        "首次执行写入一行"
    );
    assert_eq!(
        affected(
            &prepared
                .execute(&[Value::Int(2), Value::String("two".into())])
                .await
                .unwrap()
        ),
        1,
        "二次执行写入一行"
    );

    // 直接执行同一组参数——写入内容与预编译执行逐条一致。
    db.execute_sql("INSERT INTO t VALUES (3, 'three')")
        .await
        .unwrap();

    let read_back = rows(
        &db.execute_sql("SELECT id, k FROM t ORDER BY id")
            .await
            .unwrap(),
    );
    assert_eq!(
        read_back,
        vec![
            vec![Value::Int(1), Value::String("one".into())],
            vec![Value::Int(2), Value::String("two".into())],
            vec![Value::Int(3), Value::String("three".into())],
        ],
        "两次预编译执行与一次直接执行的写入内容逐条一致"
    );
}

/// T10：一次 prepare 多次执行——读面预编译语句的多次执行结果与直接执行
/// 一致，且参数值逐次生效（无跨执行值残留）。
#[tokio::test]
async fn t10_prepared_select_binds_each_execution_params() {
    let (db, _dir) = seeded_db().await;
    db.execute_sql("INSERT INTO t VALUES (1, 'one')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (2, 'two')")
        .await
        .unwrap();

    let prepared = db.prepare("SELECT k FROM t WHERE id = ?").await.unwrap();

    let first = rows(&prepared.execute(&[Value::Int(1)]).await.unwrap());
    let second = rows(&prepared.execute(&[Value::Int(2)]).await.unwrap());
    let direct = rows(
        &db.execute_sql("SELECT k FROM t WHERE id = 2")
            .await
            .unwrap(),
    );

    assert_eq!(first, vec![vec![Value::String("one".into())]]);
    assert_eq!(second, direct, "预编译执行结果与直接执行一致");
    assert_eq!(second, vec![vec![Value::String("two".into())]]);
}

/// T10：schema 变化（ALTER ADD COLUMN）后透明 re-plan——结果与直接执行一致
/// （旧计划缺新列，直接体现 re-plan 是否发生）。
#[tokio::test]
async fn t10_schema_change_triggers_transparent_replan() {
    let (db, _dir) = seeded_db().await;
    db.execute_sql("INSERT INTO t VALUES (1, 'one')")
        .await
        .unwrap();

    let prepared = db.prepare("SELECT * FROM t").await.unwrap();
    let before = rows(&prepared.execute(&[]).await.unwrap());
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].len(), 2, "变更前列数为两列");

    db.execute_sql("ALTER TABLE t ADD COLUMN extra INTEGER")
        .await
        .unwrap();

    let after = rows(&prepared.execute(&[]).await.unwrap());
    let direct = rows(&db.execute_sql("SELECT * FROM t").await.unwrap());
    assert_eq!(after, direct, "re-plan 后结果与直接执行一致");
    assert_eq!(after[0].len(), 3, "re-plan 后列数反映新 schema（三列）");
}

/// T10：schema 变化（DROP TABLE）后透明 re-plan——错误面与直接执行一致。
#[tokio::test]
async fn t10_dropped_table_yields_same_error_as_direct_execution() {
    let (db, _dir) = seeded_db().await;
    let prepared = db.prepare("SELECT * FROM t").await.unwrap();

    db.execute_sql("DROP TABLE t").await.unwrap();

    let prepared_error = prepared.execute(&[]).await.unwrap_err();
    let direct_error = db.execute_sql("SELECT * FROM t").await.unwrap_err();
    assert_eq!(
        error_message(&prepared_error),
        error_message(&direct_error),
        "re-plan 后错误面与直接执行一致"
    );
}

/// T10：schema 变化（建索引 / 删索引）后透明 re-plan——结果与直接执行一致。
#[tokio::test]
async fn t10_index_change_triggers_transparent_replan() {
    let (db, _dir) = empty_db().await;
    db.execute_sql("CREATE TABLE t (id INTEGER PRIMARY KEY, k INTEGER)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 10)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (2, 20)")
        .await
        .unwrap();

    let prepared = db.prepare("SELECT id FROM t WHERE k = ?").await.unwrap();
    let before = rows(&prepared.execute(&[Value::Int(20)]).await.unwrap());
    assert_eq!(before, vec![vec![Value::Int(2)]]);

    db.execute_sql("CREATE INDEX idx_t_k ON t(k)")
        .await
        .unwrap();
    let after_create = rows(&prepared.execute(&[Value::Int(20)]).await.unwrap());
    let direct_create = rows(
        &db.execute_sql("SELECT id FROM t WHERE k = 20")
            .await
            .unwrap(),
    );
    assert_eq!(after_create, direct_create, "建索引后结果与直接执行一致");

    db.execute_sql("DROP INDEX idx_t_k").await.unwrap();
    let after_drop = rows(&prepared.execute(&[Value::Int(20)]).await.unwrap());
    let direct_drop = rows(
        &db.execute_sql("SELECT id FROM t WHERE k = 20")
            .await
            .unwrap(),
    );
    assert_eq!(after_drop, direct_drop, "删索引后结果与直接执行一致");
}

/// T10：显式事务内执行——提交后可见、回滚后不可见。
#[tokio::test]
async fn t10_prepared_execution_inside_explicit_transaction() {
    let (db, _dir) = seeded_db().await;
    let prepared = db.prepare("INSERT INTO t VALUES (?, ?)").await.unwrap();

    // 提交路径。
    let tx = db.begin().await.unwrap();
    affected(
        &prepared
            .execute_in_tx(&[Value::Int(1), Value::String("one".into())], &tx)
            .await
            .unwrap(),
    );
    db.commit(tx).await.unwrap();
    let committed = rows(&db.execute_sql("SELECT * FROM t").await.unwrap());
    assert_eq!(committed.len(), 1, "提交后行可见");

    // 回滚路径。
    let tx = db.begin().await.unwrap();
    affected(
        &prepared
            .execute_in_tx(&[Value::Int(2), Value::String("two".into())], &tx)
            .await
            .unwrap(),
    );
    db.rollback(tx).await.unwrap();
    let rolled_back = rows(&db.execute_sql("SELECT * FROM t").await.unwrap());
    assert_eq!(rolled_back.len(), 1, "回滚后行不可见");
}

/// T10：并发共享安全——同一预编译语句经多任务并发带参执行，各次结果正确。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn t10_prepared_concurrent_execution_with_different_params() {
    let (db, _dir) = seeded_db().await;

    // 写面形态（值位槽位）。
    let insert = db.prepare("INSERT INTO t VALUES (?, ?)").await.unwrap();
    let p1 = [Value::Int(1), Value::String("one".into())];
    let p2 = [Value::Int(2), Value::String("two".into())];
    let p3 = [Value::Int(3), Value::String("three".into())];
    let (a, b, c) = tokio::join!(
        insert.execute(&p1),
        insert.execute(&p2),
        insert.execute(&p3),
    );
    assert_eq!(affected(&a.unwrap()), 1);
    assert_eq!(affected(&b.unwrap()), 1);
    assert_eq!(affected(&c.unwrap()), 1);

    let written = rows(
        &db.execute_sql("SELECT id FROM t ORDER BY id")
            .await
            .unwrap(),
    );
    assert_eq!(
        written,
        vec![
            vec![Value::Int(1)],
            vec![Value::Int(2)],
            vec![Value::Int(3)],
        ],
        "并发写入的行逐条正确"
    );

    // 读面形态（谓词位参数）——并发下各次结果与直接执行一致。
    let select = db.prepare("SELECT k FROM t WHERE id = ?").await.unwrap();
    let q1 = [Value::Int(1)];
    let q3 = [Value::Int(3)];
    let (x, y) = tokio::join!(select.execute(&q1), select.execute(&q3));
    let x_rows = rows(&x.unwrap());
    let y_rows = rows(&y.unwrap());
    let direct_x = rows(
        &db.execute_sql("SELECT k FROM t WHERE id = 1")
            .await
            .unwrap(),
    );
    let direct_y = rows(
        &db.execute_sql("SELECT k FROM t WHERE id = 3")
            .await
            .unwrap(),
    );
    assert_eq!(x_rows, direct_x, "并发读面结果与直接执行一致");
    assert_eq!(y_rows, direct_y, "并发读面结果与直接执行一致");
}

/// T10：prepare 不读写 plan cache——准备与执行后缓存长度不变。
#[tokio::test]
async fn t10_prepare_does_not_touch_plan_cache() {
    let (db, _dir) = seeded_db().await;

    let before = db.plan_cache_len();
    let prepared = db.prepare("SELECT * FROM t").await.unwrap();
    assert_eq!(db.plan_cache_len(), before, "prepare 不写 plan cache");

    prepared.execute(&[]).await.unwrap();
    assert_eq!(db.plan_cache_len(), before, "预编译执行不写 plan cache");
}

/// T10：参数个数错误在执行期点名——文案与 SQLSTATE 与参数化执行通道同源。
#[tokio::test]
async fn t10_prepared_parameter_count_mismatch_reports_same_error() {
    let (db, _dir) = seeded_db().await;
    let prepared = db.prepare("SELECT k FROM t WHERE id = ?").await.unwrap();

    let prepared_error = prepared.execute(&[]).await.unwrap_err();
    let direct_error = db
        .execute_with_params("SELECT k FROM t WHERE id = ?", &[])
        .await
        .unwrap_err();

    assert_eq!(
        error_message(&prepared_error),
        error_message(&direct_error),
        "参数个数错误文案与参数化通道一致"
    );
    assert_eq!(
        error_sqlstate(&prepared_error),
        error_sqlstate(&direct_error),
        "参数个数错误 SQLSTATE 与参数化通道一致"
    );
}

/// T10：prepare 语句面与 `execute_sql` 一致——多语句拒绝、解析错误同面。
#[tokio::test]
async fn t10_prepare_rejects_multi_statement_and_parse_errors() {
    let (db, _dir) = seeded_db().await;

    let multi = db.prepare("SELECT 1; SELECT 2").await.unwrap_err();
    let direct_multi = db.execute_sql("SELECT 1; SELECT 2").await.unwrap_err();
    assert_eq!(error_message(&multi), error_message(&direct_multi));

    let parse_error = db.prepare("SELEC 1").await.unwrap_err();
    let direct_parse_error = db.execute_sql("SELEC 1").await.unwrap_err();
    assert_eq!(
        error_message(&parse_error),
        error_message(&direct_parse_error)
    );
    assert_eq!(
        error_sqlstate(&parse_error),
        error_sqlstate(&direct_parse_error)
    );
}

/// T10：DDL 可预编译——执行触发版本 bump，后续执行透明 re-plan（幂等）。
#[tokio::test]
async fn t10_ddl_can_be_prepared_and_executed() {
    let (db, _dir) = empty_db().await;
    let prepared = db
        .prepare("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();

    prepared.execute(&[]).await.unwrap();
    assert_eq!(db.schema_version(), 1, "预编译 DDL 执行触发版本 bump");

    // 表已存在——再次执行与直接执行同样点名拒绝。
    let prepared_error = prepared.execute(&[]).await.unwrap_err();
    let direct_error = db
        .execute_sql("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .await
        .unwrap_err();
    assert_eq!(
        error_message(&prepared_error),
        error_message(&direct_error),
        "重复建表错误面与直接执行一致"
    );
}
