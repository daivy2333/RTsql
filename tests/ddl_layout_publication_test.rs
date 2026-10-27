//! change 2026-10-25-ms37-defect-ledger-closure
//! Iteration 006 / Cycle 000（T6 / T7 / T8）：DDL 提交点统一模型——
//!
//! - **T6（ISS25）**：窄化意图随事务持久化 ⇒ 崩溃重开后由恢复发布**已提交**
//!   的窄化；未提交的窄化**不**发布；发布**幂等**；checkpoint **不**丢失未
//!   发布的意图。
//! - **T7（ISS28）**：提交前失败的 DDL 其目录面变更被**补偿**（不留列定义 /
//!   列数变更）；窄化 DDL 在提交前被放弃 ⇒ 不留目录面变更。
//! - **T8（ISS29）**：同表并发的窄化 DDL 被**显式拒绝**；在途状态由带所有者
//!   令牌的每表状态承载 ⇒ 非持有者的清除不发生（读拒绝保持）；跨表并发不受
//!   影响。
//!
//! 夹具形态：独立 `TempDir` 现场（单用例自带，不依赖并行度）；真实文件库 +
//! 真实 `Database::open` + 真实文件字节 + **真实进程崩溃**（不 `close` ⇒ 不刷
//! 盘、不 checkpoint、不截断 WAL）。
//!
//! 两个注入点都是**既有**面：
//!
//! - 崩溃窗口 = `rtsql::executor::ddl_commit_window::arm_ddl_commit_window`
//!   ——真实暂停在「提交完成 → 目录应用」窗口内（不是替身）。
//! - 写盘失败 = `Database::arm_io_failure(IoFailPoint::DataWrite)`——真实
//!   I/O 失败（不是替身断言）。
//!
//! 判定全部落在 `cargo test` 内：目录面读 `Catalog`（列定义 + `column_count`），
//! 数据面读 `SELECT` 的行值与行宽。不引入身份证明机制、不建框架外判定层。
//!
//! 夹具形态的一处收窄（相对 Cycle 的 First test 文字）：源表取**三列**
//! `t(id INT PRIMARY KEY, v TEXT, w TEXT)`，被删的是**第三列** `w`。Cycle 的
//! 夹具文字写的是两列表，而 Cycle 自己的辅助断言要求「窄化后列数 == 2」与
//! 「`__columns` 少一行」——两列表删一列得到的是 1 ⇒ 与该断言矛盾。此处按
//! **断言**取三列表（删第三列 ⇒ 窄化后 2 列 2 行定义），使断言自洽。

use rtsql::database::Database;
use rtsql::executor::ddl_commit_window::{arm_ddl_commit_window, CommitWindowGate};
use rtsql::network::protocol::{to_response, Response};
use rtsql::result::{SqlError, SqlOutcome};
use rtsql::storage::add_column_count_flush_window::{
    arm_add_column_count_flush_window, AddColumnCountFlushGate,
};
use rtsql::storage::add_column_narrowing_wait::{
    arm_add_column_narrowing_wait, disarm_add_column_narrowing_wait, AddColumnNarrowingWaitGate,
};
use rtsql::storage::IoFailPoint;
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use tempfile::TempDir;

/// 被窄化的 DDL（删第三列 `w`）。
const DROP_SQL: &str = "ALTER TABLE t DROP COLUMN w";
/// 窄化前的列数（`t` 的三列）。
const COLUMNS_BEFORE: u32 = 3;
/// 窄化后的列数。
const COLUMNS_AFTER: u32 = 2;
/// 源表预置行数——30 行让「数据面改写」全程留在同一数据页内（与既有
/// `ddl_catalog_transactional_test` 的夹具约定一致），且行数够改写未完成
/// ⇒ 放弃锚点可达。
const SOURCE_ROWS: i64 = 30;
/// 锚点观察上限——夹具失控的兜底，触发即判失败（不是产品结论）。
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("ddl_layout_publication.db")
}

/// 建表 + 预置源数据。
///
/// `t(id INT PRIMARY KEY, v TEXT, w TEXT)` = 窄化对象（三列）。
/// `u(id INT PRIMARY KEY, w TEXT)` = 跨表并发对照表。
async fn setup(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v TEXT, w TEXT)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE u (id INT PRIMARY KEY, w TEXT)")
        .await
        .unwrap();
    for id in 1..=SOURCE_ROWS {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id}, 'v{id}', 'w{id}')"))
            .await
            .unwrap();
    }
    db.execute_sql("INSERT INTO u VALUES (1, 'uw')")
        .await
        .unwrap();
    db
}

/// 让进程「崩溃」：停掉 WAL 写入器并丢弃库句柄——**不** `close`（不刷盘、
/// 不 checkpoint、不截断 WAL）。
async fn crash(db: Database) {
    if let Some(wal) = db.wal_buffer.as_ref() {
        wal.shutdown().await;
    }
    drop(db);
}

/// 目录面 + 数据面形态的紧凑快照。
#[derive(Debug)]
struct Shape {
    /// 列定义（按 `column_index` 排序的列名）。
    columns: Vec<String>,
    /// 目录行广告的列数。
    column_count: u32,
    /// `SELECT *` 的行值。
    rows: Vec<Vec<serde_json::Value>>,
}

/// 目录面形态（列定义 + 广告列数）——**不读数据面**（窄化在途时读该表会被
/// 明确拒绝）。
async fn catalog_shape(db: &Database, table: &str) -> Shape {
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

    Shape {
        columns,
        column_count,
        rows: Vec::new(),
    }
}

/// 数据面读（`SELECT *`）——窄化在途时该读会被**明确拒绝**（由调用方断言）。
async fn read_rows(db: &Database, table: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(&format!("SELECT * FROM {table}")).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("SELECT * FROM {table} must return rows, got {other:?}"),
    }
}

/// 目录面 + 数据面形态的紧凑快照。
async fn observe(db: &Database, table: &str) -> Shape {
    let mut shape = catalog_shape(db, table).await;
    shape.rows = read_rows(db, table).await;
    shape
}

/// 把一条语句停在「提交完成 → 目录应用」窗口内。
///
/// 可观察锚点（`gate.reached()`）+ `yield_now` 轮询定位窗口。语句在锚点命中前
/// 就跑完 ⇒ 夹具失效，直接失败（不把「跑完」当成窗口）。语句在窗口内**不被
/// 推进**——调用方观测完成后 `gate.release()` 再 await 语句。
async fn park_at_commit_window<F>(mut statement: &mut Pin<Box<F>>, gate: &CommitWindowGate)
where
    F: Future<Output = Result<SqlOutcome, SqlError>>,
{
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if gate.reached() {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the commit-window anchor must be reachable: the fixture statement did not reach it in time")
    });
    tokio::select! {
        biased;
        _ = &mut anchor => {}
        outcome = &mut statement => panic!(
            "statement finished before the commit window was reached: {outcome:?}"
        ),
    }
}

/// 锚点：活跃事务集合中出现事务 id，且其已记录版本数 ≥2 ⇒ 语句已进入数据面
/// 改写（**提交前**）。
async fn anchor_tx(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        if db.transaction_manager.get_tx_versions(tx_id).await.len() >= 2 {
            return Some(tx_id);
        }
    }
    None
}

/// 放弃一条正在执行的语句（**提交前**）：锚点命中即丢弃语句 future。
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

    drop(statement);
    tx_id
}

/// 断言：目录面已窄化（列数 2、列定义少一行、每行 2 个值、值与窄化前一致）。
fn assert_narrowed(shape: &Shape, where_: &str) {
    assert_eq!(
        shape.column_count, COLUMNS_AFTER,
        "{where_}: the catalog column count must be the narrowed count, got {shape:?}"
    );
    assert_eq!(
        shape.columns,
        vec!["id".to_string(), "v".to_string()],
        "{where_}: the dropped column must be gone from the catalog columns, got {shape:?}"
    );
    for row in &shape.rows {
        assert_eq!(
            row.len(),
            COLUMNS_AFTER as usize,
            "{where_}: every row must be narrow-width (no misaligned third field), got {row:?}"
        );
    }
    assert_eq!(
        shape.rows.len(),
        SOURCE_ROWS as usize,
        "{where_}: the narrowing must not lose rows, got {shape:?}"
    );
    for (idx, row) in shape.rows.iter().enumerate() {
        let id = idx as i64 + 1;
        assert_eq!(
            row[0],
            json!(id),
            "{where_}: row {idx} must keep its id, got {row:?}"
        );
        assert_eq!(
            row[1],
            json!(format!("v{id}")),
            "{where_}: row {idx} must keep its v value, got {row:?}"
        );
    }
}

/// 断言：目录面仍是语句执行前的形态（列数 3、列定义三行、每行 3 个值）。
fn assert_unchanged(shape: &Shape, where_: &str) {
    assert_eq!(
        shape.column_count, COLUMNS_BEFORE,
        "{where_}: the catalog column count must stay at the pre-statement count, got {shape:?}"
    );
    assert_eq!(
        shape.columns,
        vec!["id".to_string(), "v".to_string(), "w".to_string()],
        "{where_}: the catalog columns must stay at the pre-statement columns, got {shape:?}"
    );
    for row in &shape.rows {
        assert_eq!(
            row.len(),
            COLUMNS_BEFORE as usize,
            "{where_}: every row must keep the pre-statement width, got {row:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// T6：窄化意图随事务持久化 ⇒ 崩溃重开后由恢复发布已提交的窄化（ISS25）。
// ---------------------------------------------------------------------------

/// T6 / A20 + A21：崩溃重开后，**已提交**的窄化必须被恢复发布——目录列数 == 2、
/// 列定义少一行、读值不错位。
#[tokio::test]
async fn committed_narrowing_is_published_after_crash() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;
    let before = observe(&db, "t").await;
    assert_unchanged(&before, "before the DDL");

    // 把窄化 DDL 停在「提交完成 → 目录应用」窗口内：数据面改写**已提交**、
    // 目录窄化**未发布**。
    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut statement, &gate).await;

    // 放弃该语句（`PendingColumnDrop` 随语句析构）⇒ 目录窄化仍未发布。
    drop(statement);

    // 进程「崩溃」：不 close（不刷盘、不 checkpoint、不截断 WAL）。
    crash(db).await;

    // 重新打开 ⇒ 触发恢复。
    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_narrowed(&after, "after the crash + reopen");
}

/// T6 / A21：未提交的窄化意图**不被发布**——目录列数仍为 3、读值与语句执行前
/// 一致。
///
/// 构造方式 = 真实的窄化 DDL 路径（`alter_drop_column`）+ 真实事务 + 真实
/// WAL，但**不提交**该事务（语句在提交前被放弃）⇒ 意图记录在 WAL 里、事务未
/// 提交 ⇒ 恢复期不得发布它。
#[tokio::test]
async fn uncommitted_narrowing_is_not_published() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;
    let before = observe(&db, "t").await;
    assert_unchanged(&before, "before the DDL");

    // 提交前放弃窄化 DDL ⇒ 该事务**未提交** ⇒ 意图记录在 WAL 里但未提交。
    let _tx_id = abandon_statement(&db, DROP_SQL).await;

    crash(db).await;

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_unchanged(
        &after,
        "after the crash + reopen of an uncommitted narrowing",
    );
}

/// T6 / A22：发布的**幂等性**——恢复期连续两次发布（两次重开，意图记录仍在
/// WAL 里）⇒ 目录列数与列定义与只发布一次**相同**（列行**不得**被误删）。
#[tokio::test]
async fn publication_is_idempotent_across_repeated_recoveries() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut statement, &gate).await;
    drop(statement);

    // 第一次重开 ⇒ 恢复期发布一次。
    crash(db).await;
    let db = Database::open(&db_path(&dir)).await.unwrap();
    let first = observe(&db, "t").await;
    assert_narrowed(&first, "after the first crash + reopen");

    // 第二次重开（不 close ⇒ WAL 未截断 ⇒ 意图记录仍在）⇒ 恢复期再发布一次。
    crash(db).await;
    let db = Database::open(&db_path(&dir)).await.unwrap();
    let second = observe(&db, "t").await;
    assert_narrowed(&second, "after the second crash + reopen");
    assert_eq!(
        first.columns, second.columns,
        "the repeated publication must not delete a column row by mistake"
    );
}

/// T6 / A23（M5）：「已提交但未发布」状态下做过 checkpoint 之后，崩溃重开
/// **仍**发布该窄化。
#[tokio::test]
async fn checkpoint_does_not_lose_unpublished_narrowing() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut statement, &gate).await;

    // 在「已提交但未发布」窗口内做一次 checkpoint——未发布的意图不得随 WAL
    // 截断丢失。
    db.checkpoint().await.unwrap();

    // 直接崩溃（**不放行**语句 ⇒ 语句的正常路径从未发布目录窄化 ⇒ 窄化只可能
    // 由 checkpoint 的发布或恢复期的发布产生）。
    drop(statement);
    crash(db).await;

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_narrowed(&after, "after the checkpoint + crash + reopen");
}

/// T6 对照：同样的库但**不**执行窄化 ⇒ 崩溃重开后行为与既有行为逐点一致
/// （列数 3、读值一致）。
#[tokio::test]
async fn undamaged_control_survives_crash_and_reopen() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;
    let before = observe(&db, "t").await;

    crash(db).await;

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_unchanged(&after, "after the crash + reopen without any DDL");
    assert_eq!(
        before.rows, after.rows,
        "the undamaged rows must read back identically"
    );
}

// ---------------------------------------------------------------------------
// T7：提交前失败的 DDL 其目录面变更被补偿（ISS28）。
// ---------------------------------------------------------------------------

/// T7 / A24：ADD COLUMN 在**发出目录面变更之后**失败（真实写盘失败）⇒ 目录
/// 内容恢复为语句执行前的状态（列 `m` 不存在、列数不变）。
#[tokio::test]
async fn failed_add_column_is_compensated() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;
    let before = observe(&db, "t").await;
    assert_unchanged(&before, "before the DDL");

    // 真实写盘失败（既有注入点）⇒ ADD COLUMN 在提交点失败。
    db.arm_io_failure(IoFailPoint::DataWrite);
    let outcome = db.execute_sql("ALTER TABLE t ADD COLUMN m INT").await;
    assert!(
        outcome.is_err(),
        "the ADD COLUMN must fail when the commit-point write fails: {outcome:?}"
    );

    // 其后一次成功刷盘（close ⇒ checkpoint ⇒ flush）——失败语句留下的目录面
    // 变更若未被补偿，此刻就会落盘。
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_eq!(
        after.column_count, before.column_count,
        "the failed ADD COLUMN must not leave a raised column count: {after:?}"
    );
    assert_eq!(
        after.columns, before.columns,
        "the failed ADD COLUMN must not leave a new column definition: {after:?}"
    );
    assert!(
        !after.columns.contains(&"m".to_string()),
        "the failed ADD COLUMN must not leave column 'm' behind: {after:?}"
    );
}

/// T7 对照：未损坏的 ADD COLUMN 仍然成功（补偿不得误伤成功路径）。
#[tokio::test]
async fn successful_add_column_still_adds_the_column() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    db.execute_sql("ALTER TABLE t ADD COLUMN m INT")
        .await
        .unwrap();
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_eq!(
        after.column_count,
        COLUMNS_BEFORE + 1,
        "the successful ADD COLUMN must advertise the new count: {after:?}"
    );
    assert!(
        after.columns.contains(&"m".to_string()),
        "the successful ADD COLUMN must keep column 'm': {after:?}"
    );
}

/// T7 / A25：窄化 DDL 在**提交前**被放弃 ⇒ 不留任何目录面变更（重开后列数与
/// 列定义 == 语句执行前的值）。
#[tokio::test]
async fn abandoned_narrowing_ddl_leaves_no_catalog_change() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;
    let before = observe(&db, "t").await;
    assert_unchanged(&before, "before the DDL");

    // 提交前放弃窄化 DDL。
    let _tx_id = abandon_statement(&db, DROP_SQL).await;

    crash(db).await;

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let after = observe(&db, "t").await;
    assert_unchanged(&after, "after the abandoned narrowing + crash + reopen");
}

// ---------------------------------------------------------------------------
// T8：同表并发的窄化 DDL 被显式拒绝（ISS29）。
// ---------------------------------------------------------------------------

/// T8 / A26 + A27：同表并发的窄化 DDL 被**显式拒绝**（错误点名该表）；拒绝
/// **不**清掉第一条的在途读拒绝；跨表并发 DDL **不受影响**。
#[tokio::test]
async fn concurrent_same_table_ddl_is_refused() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    // 第一条窄化 DDL 停在「提交完成 → 目录应用」窗口内（持有该表的在途令牌）。
    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql("ALTER TABLE t DROP COLUMN v"));
    park_at_commit_window(&mut statement, &gate).await;

    // 同表的第二条窄化 DDL ⇒ 必须被显式拒绝。
    match db.execute_sql("ALTER TABLE t DROP COLUMN w").await {
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("'t'"),
                "the refusal must name the table, got: {message}"
            );
            assert!(
                message.contains("refused") || message.contains("in-flight"),
                "the refusal must say the concurrent narrowing DDL is refused, got: {message}"
            );
        }
        Ok(other) => panic!(
            "the concurrent same-table narrowing DDL must be refused; it returned: {other:?}"
        ),
    }

    // 拒绝**不**得清掉第一条的在途窄化状态 ⇒ 读该表仍被明确拒绝。
    match db.execute_sql("SELECT * FROM t").await {
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("'t'"),
                "the read refusal must name the table being modified, got: {message}"
            );
        }
        Ok(SqlOutcome::Rows { rows, .. }) => panic!(
            "reading a table whose DROP COLUMN rewrite is in flight must still be refused; \
             the read returned misaligned rows instead: {rows:?}"
        ),
        Ok(other) => panic!(
            "reading a table whose DROP COLUMN rewrite is in flight must still be refused; got {other:?}"
        ),
    }

    // 被拒绝的第二条 DDL **不留目录面变更**。
    let during = catalog_shape(&db, "t").await;
    assert_unchanged(&during, "while the first narrowing is in flight");

    // 跨表并发 DDL 不受影响（令牌按表隔离）。
    db.execute_sql("ALTER TABLE u DROP COLUMN w")
        .await
        .expect("the cross-table narrowing DDL must not be refused");

    // 放行 ⇒ 第一条正常走完（目录窄化生效）。
    gate.release();
    let outcome = statement.await;
    assert!(
        outcome.is_ok(),
        "the first DROP COLUMN must succeed after the window is released: {outcome:?}"
    );

    let after = observe(&db, "t").await;
    assert_eq!(
        after.column_count, COLUMNS_AFTER,
        "the completed narrowing must advertise the narrowed count: {after:?}"
    );
    assert_eq!(
        after.columns,
        vec!["id".to_string(), "w".to_string()],
        "the completed narrowing must drop column 'v': {after:?}"
    );
}

/// 把加列停在列数刷盘成功之后、提交决定之前。
///
/// 锚点是暂停点的 `reached`。语句先结束则夹具失败。语句停住后不继续推进，
/// 调用方丢弃或放行。
async fn park_at_add_column_count_flush<F>(
    statement: &mut Pin<Box<F>>,
    gate: &AddColumnCountFlushGate,
) where
    F: Future<Output = Result<SqlOutcome, SqlError>>,
{
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if gate.reached() {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "the add-column flush anchor must be reachable: the fixture statement did not reach it in time",
        )
    });
    tokio::select! {
        biased;
        _ = &mut anchor => {}
        outcome = &mut *statement => panic!(
            "statement finished before the add-column flush window was reached: {outcome:?}"
        ),
    }
}

/// 列数刷盘已经成功、结果还没交回时放弃加列：不 `close`，崩溃再打开后新列不可见，
/// 列数回到语句前。
#[tokio::test]
async fn abandoned_add_column_during_count_flush_is_not_durable() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("add_column_flush_abandon.db");
    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 1)").await.unwrap();
    let before = catalog_shape(&db, "t").await;

    let gate = arm_add_column_count_flush_window(&path);
    let mut statement = Box::pin(db.execute_sql("ALTER TABLE t ADD COLUMN m INT"));
    park_at_add_column_count_flush(&mut statement, &gate).await;

    drop(statement);
    gate.release();

    // 10 秒是这次等待的上限。到点后仍崩溃重开，到点本身不是夹具失败。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let shape = catalog_shape(&db, "t").await;
        if shape.column_count == before.column_count
            && !shape.columns.iter().any(|name| name == "m")
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::task::yield_now().await;
    }

    crash(db).await;

    let db = Database::open(&path).await.unwrap();
    let after = catalog_shape(&db, "t").await;
    assert_eq!(
        after.column_count, before.column_count,
        "reopen after an abandoned ADD COLUMN must keep the pre-statement column count, got {after:?}"
    );
    assert!(
        !after.columns.iter().any(|name| name == "m"),
        "reopen after an abandoned ADD COLUMN must not keep column 'm', got {after:?}"
    );
}

/// 把同表加列停在写目录之前的等待暂停点。语句先结束则夹具失败。
async fn park_until_add_column_narrowing_wait<F>(
    statement: &mut Pin<Box<F>>,
    gate: &AddColumnNarrowingWaitGate,
) where
    F: Future<Output = Result<SqlOutcome, SqlError>>,
{
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if gate.reached_count() >= 1 {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the add-column narrowing-wait anchor must be reachable: the fixture statement did not reach it in time")
    });
    tokio::select! {
        biased;
        _ = &mut anchor => {}
        outcome = &mut *statement => panic!(
            "statement finished before the add-column narrowing wait was reached: {outcome:?}"
        ),
    }
}

/// 加列 future 与一次 `yield_now` 竞争。future 已经完成则失败。
async fn assert_add_column_still_waiting<F>(statement: &mut Pin<Box<F>>)
where
    F: Future<Output = Result<SqlOutcome, SqlError>>,
{
    tokio::select! {
        biased;
        outcome = &mut *statement => panic!(
            "ADD COLUMN finished while the same-table narrowing is still in flight: {outcome:?}"
        ),
        _ = tokio::task::yield_now() => {}
    }
}

fn sort_rows_by_id(mut rows: Vec<Vec<serde_json::Value>>) -> Vec<Vec<serde_json::Value>> {
    rows.sort_by(|left, right| {
        left.first()
            .and_then(serde_json::Value::as_i64)
            .cmp(&right.first().and_then(serde_json::Value::as_i64))
    });
    rows
}

fn assert_column_m_after_drop(shape: &Shape, where_: &str) {
    assert_eq!(
        shape.column_count, COLUMNS_BEFORE,
        "{where_}: the column count must stay 3, got {shape:?}"
    );
    assert_eq!(
        shape.columns,
        vec!["id".to_string(), "v".to_string(), "m".to_string()],
        "{where_}: columns must be id, v, m, got {shape:?}"
    );
    assert!(
        !shape.columns.iter().any(|name| name == "w"),
        "{where_}: column w must be gone, got {shape:?}"
    );
    let rows = sort_rows_by_id(shape.rows.clone());
    assert_eq!(
        rows.len(),
        SOURCE_ROWS as usize,
        "{where_}: row count must stay {SOURCE_ROWS}, got {shape:?}"
    );
    for row in &rows {
        let id = row[0]
            .as_i64()
            .unwrap_or_else(|| panic!("{where_}: id must be an integer, got {row:?}"));
        assert_eq!(
            row,
            &vec![json!(id), json!(format!("v{id}")), serde_json::Value::Null,],
            "{where_}: row {id} must be id, v{{id}}, null, got {row:?}"
        );
    }
}

/// 删列停在提交窗口时，同表加列等待而不改目录；放行后发布的是取得令牌时的布局，
/// 加列再追加。进程内 `SELECT` 与关闭再打开一致。
#[tokio::test]
async fn add_column_waits_out_inflight_narrowing_and_reopen_matches() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;
    let before = catalog_shape(&db, "t").await;
    assert_eq!(
        before.columns,
        vec!["id".to_string(), "v".to_string(), "w".to_string()],
        "setup must leave columns id, v, w, got {before:?}"
    );
    assert_eq!(
        before.column_count, COLUMNS_BEFORE,
        "setup column count, got {before:?}"
    );

    db.execute_sql("CREATE TABLE s (id INT PRIMARY KEY)")
        .await
        .unwrap();

    let drop_gate = arm_ddl_commit_window(&db_path(&dir));
    let mut drop_statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut drop_statement, &drop_gate).await;

    let wait_gate = arm_add_column_narrowing_wait(&db_path(&dir));
    let mut add_statement = Box::pin(db.execute_sql("ALTER TABLE t ADD COLUMN m INT"));
    park_until_add_column_narrowing_wait(&mut add_statement, &wait_gate).await;

    let during = catalog_shape(&db, "t").await;
    assert_eq!(
        during.column_count, before.column_count,
        "catalog count changed before either pause was released: {during:?}"
    );
    assert_eq!(
        during.columns, before.columns,
        "catalog columns changed before either pause was released: {during:?}"
    );

    match db.execute_sql("SELECT * FROM t").await {
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("'t'"),
                "the in-flight read refusal must name table t, got: {message}"
            );
        }
        Ok(other) => {
            panic!("SELECT * FROM t must be refused while the drop is in flight, got {other:?}")
        }
    }

    db.execute_sql("ALTER TABLE u ADD COLUMN extra INT")
        .await
        .expect("ADD COLUMN on table u must succeed while t is narrowing");
    let after_other = catalog_shape(&db, "t").await;
    assert_eq!(
        after_other.column_count, before.column_count,
        "u's ADD COLUMN changed t's column count: {after_other:?}"
    );
    assert_eq!(
        after_other.columns, before.columns,
        "u's ADD COLUMN changed t's columns: {after_other:?}"
    );

    wait_gate.release();
    let mut second = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if wait_gate.reached_count() >= 2 {
                    return;
                }
                let shape = catalog_shape(&db, "t").await;
                if shape.column_count != before.column_count || shape.columns != before.columns {
                    panic!(
                        "t's catalog changed while waiting for the second add-column pause: {shape:?}"
                    );
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the second add-column narrowing wait must be reached before the anchor timeout")
    });
    tokio::select! {
        biased;
        _ = &mut second => {}
        outcome = &mut add_statement => panic!(
            "ADD COLUMN finished before the second narrowing wait: {outcome:?}"
        ),
    }
    drop(second);

    disarm_add_column_narrowing_wait(&db_path(&dir));
    wait_gate.release();
    tokio::task::yield_now().await;

    assert_add_column_still_waiting(&mut add_statement).await;

    db.execute_sql("ALTER TABLE s ADD COLUMN extra INT")
        .await
        .expect("ADD COLUMN on table s must succeed before the drop is released");
    assert_add_column_still_waiting(&mut add_statement).await;
    let still = catalog_shape(&db, "t").await;
    assert_eq!(
        still.column_count, before.column_count,
        "t's catalog count changed before the drop was released: {still:?}"
    );
    assert_eq!(
        still.columns, before.columns,
        "t's catalog columns changed before the drop was released: {still:?}"
    );

    drop_gate.release();
    let drop_outcome = drop_statement.await;
    assert!(
        drop_outcome.is_ok(),
        "DROP COLUMN must succeed after the commit window is released: {drop_outcome:?}"
    );
    match add_statement.await {
        Ok(SqlOutcome::AffectedRows(0)) => {}
        other => panic!(
            "ADD COLUMN must succeed with affected rows 0 after the narrowing clears: {other:?}"
        ),
    }

    let live = observe(&db, "t").await;
    assert_column_m_after_drop(&live, "in-process read");
    db.close().await.unwrap();
    drop(db);
    let reopened = Database::open(&db_path(&dir)).await.unwrap();
    let restored = observe(&reopened, "t").await;
    assert_column_m_after_drop(&restored, "reopen");
    assert_eq!(
        sort_rows_by_id(live.rows.clone()),
        sort_rows_by_id(restored.rows),
        "in-process rows and reopened rows must match"
    );
}
