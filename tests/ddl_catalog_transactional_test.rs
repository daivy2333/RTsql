//! change 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog / Iteration 001（T04）：
//! DDL 目录变更的事务化（承载 ISS21）——被放弃 / 失败的
//! `ALTER TABLE DROP COLUMN` 不窄化目录；目录变更与数据面改写同生共死；放弃后
//! 重开库，目录列数与数据面行宽一致。
//!
//! 夹具形态复用 MS34 的既有夹具约定：独立 `TempDir` 现场（单用例自带，不依赖
//! 并行度）；`t(id INT PRIMARY KEY, v INT, w INT)` 预置 `SOURCE_ROWS` 行。
//!
//! 预置行数取 30：锚点只需「语句已进入数据面改写」（逐行 `record_version`，
//! 每行产出墓碑 + 新行两个版本，第一行完成即达 2），行数只需够改写未完成；
//! 30 行让改写全程留在同一数据页内——更大的表会撞上「改写过程中页写满」的
//! 既有行为（见 Act Response 的 Issue 候选），与本 change 的目录面无关。
//!
//! 放弃手法 = 锚点式手法：活跃事务集合里出现「已记录版本数 ≥2」的事务 ⇒ 该
//! 语句已进入数据面改写。命中锚点即 `drop(statement)`；语句在锚点可观测前跑完
//! ⇒ 夹具不成立，直接失败，不把「跑完」当成被放弃。
//!
//! 锚点观察用 `tokio::task::yield_now()` 让出执行权；10 秒上限只作夹具失控
//! 兜底（触发即失败），不是行为判据。
//!
//! 判定全部落在 `cargo test` 内：目录面读 `Catalog`（列定义 + `column_count`），
//! 数据面读 `SELECT *` 的行宽（每行的值个数）。不引入身份证明机制、不建框架
//! 外判定层。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::integrity::{check_integrity, FindingSeverity};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

/// 被放弃的 DDL 语句。
const ABANDONED_SQL: &str = "ALTER TABLE t DROP COLUMN v";
/// 源表预置行数。
const SOURCE_ROWS: i64 = 30;
/// 语句执行前的列数（`t` 的三列）。
const COLUMNS_BEFORE: u32 = 3;
/// 锚点观察上限——夹具失控的兜底，触发即判失败（不是产品结论）。
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("ddl_catalog_transactional.db")
}

/// 建表 + 预置源数据（`t` 三列 `SOURCE_ROWS` 行，`t2` 为回收触发用的写入目标）。
async fn setup(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT, w INT)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    for id in 1..=SOURCE_ROWS {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id}, {id}, {id})"))
            .await
            .unwrap();
    }
    db
}

/// 锚点：活跃事务集合中出现事务 id，且其已记录版本数 ≥2。
async fn anchor_tx(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        if db.transaction_manager.get_tx_versions(tx_id).await.len() >= 2 {
            return Some(tx_id);
        }
    }
    None
}

/// 放弃一条正在执行的语句：锚点命中即丢弃语句 future。
///
/// 返回被放弃的事务 id。语句在锚点命中前就跑完 ⇒ 夹具规模不对（锚点必须在
/// 语句完成前可达），此时直接失败而不是把「跑完」当成被放弃。
async fn abandon_statement(db: &Database, sql: &str) -> u64 {
    let mut statement = Box::pin(db.execute_sql(sql));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if let Some(tx_id) = anchor_tx(db).await {
                    return tx_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "the abandon anchor must be reachable: the fixture statement did not reach it in time",
        )
    });

    let tx_id = tokio::select! {
        biased;
        tx_id = &mut anchor => tx_id,
        outcome = &mut statement => panic!(
            "statement finished before the abandon anchor was reached: {outcome:?}"
        ),
    };

    // 放弃语句 future——执行器停在执行期，隐式事务留在活跃集里。
    drop(statement);
    tx_id
}

/// 断言一次写入成功并返回受影响行数。
async fn expect_affected(db: &Database, sql: &str) -> u64 {
    match to_response(db.execute_sql(sql).await) {
        Response::AffectedRows { count } => count,
        other => panic!("{sql} must succeed with affected rows, got {other:?}"),
    }
}

/// 目录面 + 数据面形态的紧凑快照。
///
/// 目录面 = 目录的列定义（按 `column_index` 排序）与列数；数据面 =
/// `SELECT *` 的行数与行宽（每行的值个数）。
#[derive(Debug)]
struct Shape {
    columns: Vec<String>,
    column_count: u32,
    row_count: usize,
    widths: BTreeSet<usize>,
}

async fn observe(db: &Database, table: &str) -> Shape {
    let mut cols = db
        .table_manager
        .catalog()
        .scan_columns(table)
        .await
        .unwrap();
    cols.sort_by_key(|c| c.column_index);
    let columns: Vec<String> = cols.into_iter().map(|c| c.column_name).collect();

    let column_count = db
        .table_manager
        .catalog()
        .scan_tables()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.table_name == table)
        .expect("the table must have a catalog row")
        .column_count;

    let rows = match to_response(db.execute_sql(&format!("SELECT * FROM {table}")).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("SELECT * FROM {table} must return rows, got {other:?}"),
    };

    Shape {
        columns,
        column_count,
        row_count: rows.len(),
        widths: rows.iter().map(|r| r.len()).collect(),
    }
}

/// 断言目录面与数据面都还处于「语句执行前」的形态。
///
/// 目录列数与数据面行宽都对齐到**语句执行前的列数**——不是对齐到（可能被
/// 窄化过的）目录自身宽度：窄化后的目录 + 窄化后的读回宽度同样「自洽」，但
/// 那正是「被放弃的语句改写了目录」这一缺陷的形态。
///
/// 行数只断言「非空」：放弃的语句会留下一条墓碑，其前驱行在回收后不再可见
/// （回收域的既有行为，不属 DCT 范围），因此这里不把行数钉死。
fn assert_unchanged(shape: &Shape, where_: &str) {
    assert_eq!(
        shape.columns,
        vec!["id", "v", "w"],
        "{where_}: the dropped column must still be in the catalog columns, got {shape:?}"
    );
    assert_eq!(
        shape.column_count, COLUMNS_BEFORE,
        "{where_}: the catalog column count must stay at the pre-statement count, got {shape:?}"
    );
    assert!(
        shape.row_count > 0,
        "{where_}: the data plane must not be empty, got {shape:?}"
    );
    assert_eq!(
        shape.widths,
        BTreeSet::from([COLUMNS_BEFORE as usize]),
        "{where_}: every row must keep the pre-statement width, got {shape:?}"
    );
}

/// T04 / DCT R1 + R3：被放弃的 `ALTER TABLE DROP COLUMN` 不留下目录面变更；
/// 放弃 → 回收 → 重开库后，目录列数与数据面行宽一致。
#[tokio::test]
async fn abandoned_drop_column_leaves_catalog_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    // 语句执行前的基线。
    let before = observe(&db, "t").await;
    assert_unchanged(&before, "before the DDL");

    // 放弃 ALTER TABLE DROP COLUMN v——锚点命中即丢弃语句 future。
    let tx_id = abandon_statement(&db, ABANDONED_SQL).await;
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the abandoned DDL's transaction must still be active right after the abandon"
    );

    // 下一条写语句触发回收：必须成功返回（不报 `Reclaim failed`）。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;

    // 回收之后没有悬空事务。
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "the abandoned DDL transaction must be reclaimed, active transactions are still {active:?}"
    );

    // 目录面与数据面：被放弃的语句不留任何变更。
    let after = observe(&db, "t").await;
    assert_unchanged(&after, "after the abandoned DDL was reclaimed");

    // 重开库后再断言一次（DCT R3：目录列数与数据面行宽一致）。
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let reopened = observe(&db, "t").await;
    assert_unchanged(&reopened, "after close + reopen");
}

/// T04 / DCT R1：被守卫拒绝的 DROP COLUMN 报错后，目录面与数据面均不变。
#[tokio::test]
async fn failed_drop_column_leaves_catalog_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;
    // 一张单列表（最后一列的守卫面）与一个引用 `v` 的二级索引（被索引引用的
    // 列的守卫面）。
    db.execute_sql("CREATE TABLE t3 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("CREATE INDEX idx_t_v ON t (v)")
        .await
        .unwrap();

    let before = observe(&db, "t").await;
    assert_unchanged(&before, "before the rejected DDL");
    let before_t3 = observe(&db, "t3").await;

    // 主键列不能被删除。
    assert!(
        db.execute_sql("ALTER TABLE t DROP COLUMN id")
            .await
            .is_err(),
        "dropping the primary key column must be rejected"
    );
    // 最后一列不能被删除。
    assert!(
        db.execute_sql("ALTER TABLE t3 DROP COLUMN id")
            .await
            .is_err(),
        "dropping the last column of a table must be rejected"
    );
    // 被二级索引引用的列不能被删除。
    assert!(
        db.execute_sql("ALTER TABLE t DROP COLUMN v").await.is_err(),
        "dropping a secondary-indexed column must be rejected"
    );

    // 目录面与数据面均未变更。
    let after = observe(&db, "t").await;
    assert_unchanged(&after, "after the rejected DDL");
    let after_t3 = observe(&db, "t3").await;
    assert_eq!(
        after_t3.columns, before_t3.columns,
        "the rejected DDL must not change the catalog columns, got {after_t3:?}"
    );
    assert_eq!(
        after_t3.column_count, before_t3.column_count,
        "the rejected DDL must not change the catalog column count, got {after_t3:?}"
    );
    assert_eq!(
        after_t3.row_count, before_t3.row_count,
        "the rejected DDL must not change the data plane, got {after_t3:?}"
    );
}

/// T04 / DCT R2（对照基线）：正常完成的 DROP COLUMN 使目录与数据面一致。
///
/// T06 追加：close → reopen 后目录与数据面仍然一致（DCT R3 的完成路径一侧），
/// 且 `check_integrity` 的 error 级发现为 0。
#[tokio::test]
async fn completed_drop_column_aligns_catalog_and_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    expect_affected(&db, ABANDONED_SQL).await;

    let shape = observe(&db, "t").await;
    assert_eq!(
        shape.columns,
        vec!["id", "w"],
        "the dropped column must be gone from the catalog columns, got {shape:?}"
    );
    assert_eq!(
        shape.column_count,
        COLUMNS_BEFORE - 1,
        "the catalog column count must drop by one, got {shape:?}"
    );
    assert_eq!(
        shape.row_count, SOURCE_ROWS as usize,
        "the data plane must keep every row, got {shape:?}"
    );
    assert_eq!(
        shape.widths,
        BTreeSet::from([(COLUMNS_BEFORE - 1) as usize]),
        "every row must be stored with the narrowed width, got {shape:?}"
    );

    // 完整性：目录与数据面一致 ⇒ 没有 error 级发现。
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the completed DROP COLUMN: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Error)
            .collect::<Vec<_>>()
    );

    // close → reopen：目录与数据面仍然一致。
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let reopened = observe(&db, "t").await;
    assert_eq!(
        reopened.columns,
        vec!["id", "w"],
        "the dropped column must stay gone after the reopen, got {reopened:?}"
    );
    assert_eq!(
        reopened.column_count,
        COLUMNS_BEFORE - 1,
        "the catalog column count must stay narrowed after the reopen, got {reopened:?}"
    );
    assert_eq!(
        reopened.row_count, SOURCE_ROWS as usize,
        "the data plane must keep every row after the reopen, got {reopened:?}"
    );
    assert_eq!(
        reopened.widths,
        BTreeSet::from([(COLUMNS_BEFORE - 1) as usize]),
        "every row must stay stored with the narrowed width after the reopen, got {reopened:?}"
    );
}
