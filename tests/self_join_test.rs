//! Iteration 000 / Cycle 001（replan）：自连接等值 JOIN 侧身份正确性（Task T9，design D9）
//!
//! change: 2026-10-04-external-review-gaps
//!
//! 病灶：JOIN 装配层的侧身份以真实表名承载——① ON 交换判定比较归一真实名
//! （自连接两腿恒真 → 左腿先写形态恒误判交换，行配对错误）；② Hash 与 NLJ
//! 两执行器输出归属比较 `OutputColumn.table_alias`（构造恒真实名）与链首名
//! （自连接两侧同名 → 全部列恒读左侧行）。既有语料唯一自连接用例
//! `ON a.id = b.id`（两侧行值恒同）恰好掩盖两缺陷。
//!
//! 目标语义（R9 / S31–S37，delta spec `sql-table-alias-resolution`）：
//! - ON 条件配对按书写限定符与声明限定符匹配判定侧别，交换只发生在实际反序
//!   书写形态；两腿同侧计划期点名拒绝零副作用；裸名腿按位置解析。
//! - Hash 与 NLJ 两执行器输出行（含外连接补齐行）按计划期显式侧标志归属，
//!   自连接投影各读对应侧行；非自连接全部形态零回归。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
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

fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

/// S31–S34 夹具：`emp2(id, name, manager_id)`——m1/m2 为经理（manager_id
/// NULL），r1/r2 分别汇报给 id=1 / id=2。
async fn seed_emp2(db: &Database) {
    exec_ok(
        db,
        "CREATE TABLE emp2 (id INT PRIMARY KEY, name STRING, manager_id INT)",
    )
    .await;
    exec_ok(db, "INSERT INTO emp2 VALUES (1, 'm1', NULL)").await;
    exec_ok(db, "INSERT INTO emp2 VALUES (2, 'm2', NULL)").await;
    exec_ok(db, "INSERT INTO emp2 VALUES (10, 'r1', 1)").await;
    exec_ok(db, "INSERT INTO emp2 VALUES (11, 'r2', 2)").await;
}

/// S36 夹具：非自连接两表——`dept(id, dname)` 与 `emp3(id, ename, dept_id)`，
/// d 行 dept_id NULL 不命中任何部门。
async fn seed_dept_emp3(db: &Database) {
    exec_ok(db, "CREATE TABLE dept (id INT PRIMARY KEY, dname STRING)").await;
    exec_ok(db, "INSERT INTO dept VALUES (10, 'sales')").await;
    exec_ok(db, "INSERT INTO dept VALUES (20, 'eng')").await;
    exec_ok(
        db,
        "CREATE TABLE emp3 (id INT PRIMARY KEY, ename STRING, dept_id INT)",
    )
    .await;
    exec_ok(db, "INSERT INTO emp3 VALUES (1, 'a', 10)").await;
    exec_ok(db, "INSERT INTO emp3 VALUES (2, 'b', 10)").await;
    exec_ok(db, "INSERT INTO emp3 VALUES (3, 'c', 20)").await;
    exec_ok(db, "INSERT INTO emp3 VALUES (4, 'd', NULL)").await;
}

/// S31：Hash 等值 JOIN 正向书写（左腿属左输入）——行配对经理 × 汇报者。
///
/// RED（实施前实测）：交换判定以归一真实名比较（自连接恒真）误判交换，
/// 得 `[["r1","r1"],["r2","r2"]]`。
#[tokio::test]
async fn hash_forward_self_join_pairs_correctly() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_emp2(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT e1.name, e2.name FROM emp2 e1 JOIN emp2 e2 ON e1.id = e2.manager_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("m1"), json!("r1")],
            vec![json!("m2"), json!("r2")]
        ],
        "rows: {rows:?}"
    );
}

/// S32：反序书写形态（右表腿先写）——与 S31 同结果。
///
/// RED（实施前实测）：恒误判交换后配对虽翻转回正确组合，但归属恒读左侧，
/// 得 `[["m1","m1"],["m2","m2"]]`。
#[tokio::test]
async fn hash_reversed_self_join_same_pairing() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_emp2(&db).await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT e1.name, e2.name FROM emp2 e1 JOIN emp2 e2 ON e2.manager_id = e1.id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("m1"), json!("r1")],
            vec![json!("m2"), json!("r2")]
        ],
        "rows: {rows:?}"
    );
}

/// S33：投影归属——各列读其限定符对应侧行。
///
/// RED（实施前实测）：Hash 归属按真实表名比较（自连接恒读左侧行），
/// 同形态得 `[[10,10],[11,11]]`（左侧行 id 读两遍）。
///
/// 校准注记（Act 2026-10-04）：JOIN 基座输出列序恒为表序（SELECT 序重排在
/// 该基座既不生效，自连接与非自连接、Hash 与 NLJ 同语义——既有共享行为，
/// 范围外），故归属见证用与表序一致的 SELECT 形态。
///
/// 校准注记（Act 2026-10-05，T10/change 2026-10-04-external-review-gaps）：
/// JOIN 基座输出列序已 SELECT 化（join-output-shape-resolution R10），第二
/// 形态断言由表序 `[m1, r1, 1]` 翻转为 SELECT 序 `[m1, 1, r1]`；归属语义
/// 不变（各列仍读其限定符对应侧行）。
#[tokio::test]
async fn self_join_projection_reads_declared_side() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_emp2(&db).await;

    // 同名双列：左 id 读左行、右 id 读右行（修复前两列同读左行）。
    let rows = query_rows(
        db.execute_sql("SELECT e1.id, e2.id FROM emp2 e1 JOIN emp2 e2 ON e1.id = e2.manager_id")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(10)], vec![json!(2), json!(11)]],
        "rows: {rows:?}"
    );

    // 三列混排：e1.name / e2.manager_id / e2.name——输出按 SELECT 序
    //（T10 校准：表序 [name, name, manager_id] → SELECT 序 [name,
    // manager_id, name]），归属各读对应侧。
    let rows = query_rows(
        db.execute_sql(
            "SELECT e1.name, e2.manager_id, e2.name FROM emp2 e1 JOIN emp2 e2 \
             ON e1.id = e2.manager_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("m1"), json!(1), json!("r1")],
            vec![json!("m2"), json!(2), json!("r2")]
        ],
        "rows: {rows:?}"
    );
}

/// S34：NLJ 外连接与非等值形态——匹配行与补齐行均按侧标志归属。
///
/// RED（实施前实测）：NLJ 归属恒读左侧，LEFT 形态得全部 `[x, x]`，
/// FULL 补齐行右侧列错读左侧。
#[tokio::test]
async fn self_join_outer_and_nonequi_attribution() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_emp2(&db).await;

    // LEFT JOIN：未匹配的 r1/r2 行右侧补 NULL。
    let rows = query_rows(
        db.execute_sql(
            "SELECT e1.name, e2.name FROM emp2 e1 LEFT JOIN emp2 e2 \
             ON e1.id = e2.manager_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("m1"), json!("r1")],
            vec![json!("m2"), json!("r2")],
            vec![json!("r1"), json!(null)],
            vec![json!("r2"), json!(null)]
        ],
        "LEFT JOIN rows: {rows:?}"
    );

    // FULL JOIN：追加右侧未匹配的 m1/m2 行左侧补 NULL。
    let rows = query_rows(
        db.execute_sql(
            "SELECT e1.name, e2.name FROM emp2 e1 FULL JOIN emp2 e2 \
             ON e1.id = e2.manager_id",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("m1"), json!("r1")],
            vec![json!("m2"), json!("r2")],
            vec![json!("r1"), json!(null)],
            vec![json!("r2"), json!(null)],
            vec![json!(null), json!("m1")],
            vec![json!(null), json!("m2")]
        ],
        "FULL JOIN rows: {rows:?}"
    );

    // 非等值自连接（NLJ 谓词路径）：id 小于 id 的组合，两列各读对应侧。
    let rows = query_rows(
        db.execute_sql("SELECT e1.id, e2.id FROM emp2 e1 JOIN emp2 e2 ON e1.id < e2.id")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(2)],
            vec![json!(1), json!(10)],
            vec![json!(1), json!(11)],
            vec![json!(2), json!(10)],
            vec![json!(2), json!(11)],
            vec![json!(10), json!(11)]
        ],
        "non-equi rows: {rows:?}"
    );
}

/// S35：双腿同侧计划期点名拒绝零副作用。
///
/// RED（实施前实测）：同侧腿当前静默误判执行（恒误判交换后产出错误配对）。
#[tokio::test]
async fn self_join_same_side_on_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_emp2(&db).await;

    // 双腿同左侧。
    let resp = db
        .execute_sql("SELECT e1.name FROM emp2 e1 JOIN emp2 e2 ON e1.id = e1.manager_id")
        .await;
    let msg = error_msg(resp);
    assert!(
        msg.contains("both sides"),
        "same-side left legs must be rejected, got: {msg}"
    );

    // 双腿同右侧。
    let resp = db
        .execute_sql("SELECT e1.name FROM emp2 e1 JOIN emp2 e2 ON e2.id = e2.manager_id")
        .await;
    let msg = error_msg(resp);
    assert!(
        msg.contains("both sides"),
        "same-side right legs must be rejected, got: {msg}"
    );

    // 零副作用：表数据不受拒绝语句影响。
    let rows = query_rows(db.execute_sql("SELECT COUNT(*) FROM emp2").await);
    assert_eq!(rows, vec![vec![json!(4)]], "rows: {rows:?}");
}

/// S36：非自连接全形态对照——正向 / 反向 / 裸名 / 无别名 / LEFT 补齐逐字节
/// 与既有行为一致。
#[tokio::test]
async fn non_self_join_forms_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_dept_emp3(&db).await;

    // 正向限定。
    let forward = query_rows(
        db.execute_sql("SELECT e.ename, d.dname FROM emp3 e JOIN dept d ON e.dept_id = d.id")
            .await,
    );
    assert_eq!(
        forward,
        vec![
            vec![json!("a"), json!("sales")],
            vec![json!("b"), json!("sales")],
            vec![json!("c"), json!("eng")]
        ],
        "forward rows: {forward:?}"
    );

    // 反向限定：同结果。
    let reversed = query_rows(
        db.execute_sql("SELECT e.ename, d.dname FROM emp3 e JOIN dept d ON d.id = e.dept_id")
            .await,
    );
    assert_eq!(reversed, forward, "reversed rows: {reversed:?}");

    // 裸名左腿 + 限定右腿：按位置解析（裸名 dept_id 属左输入 emp3）。
    let bare = query_rows(
        db.execute_sql("SELECT e.ename, d.dname FROM emp3 e JOIN dept d ON dept_id = d.id")
            .await,
    );
    assert_eq!(bare, forward, "bare rows: {bare:?}");

    // 无别名（声明限定符即真实表名）。
    let no_alias = query_rows(
        db.execute_sql(
            "SELECT emp3.ename, dept.dname FROM emp3 JOIN dept \
             ON emp3.dept_id = dept.id",
        )
        .await,
    );
    assert_eq!(no_alias, forward, "no-alias rows: {no_alias:?}");

    // LEFT JOIN 补齐行归属（d 行 dept_id NULL 不命中，右侧补 NULL）。
    let left = query_rows(
        db.execute_sql("SELECT e.ename, d.dname FROM emp3 e LEFT JOIN dept d ON e.dept_id = d.id")
            .await,
    );
    assert_eq!(
        left,
        vec![
            vec![json!("a"), json!("sales")],
            vec![json!("b"), json!("sales")],
            vec![json!("c"), json!("eng")],
            vec![json!("d"), json!(null)]
        ],
        "LEFT JOIN rows: {left:?}"
    );
}
