//! change 2026-10-24-ms34-cancellation-timeout-transaction / Iteration 000（T01）：
//! 被放弃的隐式语句不提交、库属隐式事务回收与 checkpoint 落盘边界。
//!
//! 夹具：独立 `TempDir` 现场；`t(id INT PRIMARY KEY, v INT)` 预置 2000 行；
//! 被放弃的语句是一条 `INSERT INTO t2 SELECT id FROM t`（一条语句、一个隐式
//! 事务、2000 行）。
//!
//! 放弃点由库入口可观察的锚点确定，不靠定时器：活跃事务集合出现该事务 id，
//! 且其版本数 ≥2——执行器逐行推进，第 1 行的索引条目先于第 2 行的
//! `record_version` 写入完成，所以 ≥2 个版本保证「第一个键的索引条目已在位」。
//! 观察循环用 `tokio::task::yield_now()` 让出执行权并带上限（10 秒兜底，
//! 触发即判失败），不用固定 sleep 阈值判定。
//!
//! 第一个键的确定性：`t` 的 id 是 1..2000，被放弃语句按 `SELECT id FROM t`
//! 的顺序插入，`t2` 的第一个键恒为 1 → 回收前 `INSERT INTO t2 VALUES (1)`
//! 必然撞上残留索引条目，回收后必然成功。
//!
//! 并发用例复用同一夹具：A 在克隆出的同一句柄上并发执行，B 在同一句柄上
//! 执行到完成——B 的回收不得中止 A 仍在执行的隐式事务。
//!
//! Iteration 001（T02.1–T02.4）追加调用方可见的三种结果与收尾回归：
//! `committed_statement_survives_undelivered_result`（提交已完成但结果未送达）、
//! `abandoned_statement_in_explicit_tx_keeps_tx_open`（显式事务由调用方决定
//! 继续还是回滚）、`timeout_in_explicit_tx_is_not_a_rollback`（超时不代替回滚）、
//! `session_keeps_working_after_abandoned_statement`（放弃之后会话继续可用）。
//! 显式事务用例以 `ReadCommitted` 打开（见 [`setup_with_isolation`]）。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::integrity::check_integrity;
use rtsql::transaction::{IsolationLevel, Transaction};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// 被放弃的语句（一条语句、一个隐式事务、2000 行）。
const ABANDONED_SQL: &str = "INSERT INTO t2 SELECT id FROM t";
/// 源表预置行数。
const SOURCE_ROWS: i64 = 2000;
/// 锚点观察上限——夹具失控的兜底，触发即判失败（不是产品结论）。
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);
/// 超时取消窗口（Iteration 001 T02.3）——**取消触发源**，不是判据：远大于
/// 语句推进到第 2 个版本所需的时间（毫秒级），远小于该语句完成所需的时间
/// （秒级）。断言全部落在可观察状态上。
const TIMEOUT_CANCEL_WINDOW: Duration = Duration::from_millis(20);

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("statement_cancellation.db")
}

/// 建表 + 预置源数据（`t` 2000 行，`t2` 为写入目标）。
async fn setup(dir: &TempDir) -> Database {
    setup_with_isolation(dir, IsolationLevel::RepeatableRead).await
}

/// 与 [`setup`] 同构的夹具，但以指定隔离级别打开。
///
/// 显式事务的「事务内读得到自己的未提交写、事务外读不到未提交的写」只在
/// `ReadCommitted` 下可观察：`RepeatableRead` 的 `statement_snapshot` 返回
/// `None`（`src/database.rs:477`），扫描路径不做可见性过滤——未提交的行在
/// 事务内外都读得到。
async fn setup_with_isolation(dir: &TempDir, isolation: IsolationLevel) -> Database {
    let db = Database::open_with_isolation(&db_path(dir), isolation)
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    for id in 1..=SOURCE_ROWS {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id}, {id})"))
            .await
            .unwrap();
    }
    db
}

/// 锚点：活跃事务集合中出现事务 id，且其已记录版本数 ≥2。
///
/// ≥2 个版本意味着第一个键的索引条目已在位（插入顺序：`record_version` →
/// 索引写入，逐行推进）。
async fn anchor_tx(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        if db.transaction_manager.get_tx_versions(tx_id).await.len() >= 2 {
            return Some(tx_id);
        }
    }
    None
}

/// 放弃一条正在执行的隐式语句：锚点命中即丢弃语句 future。
///
/// 返回被放弃的事务 id。语句在锚点命中前就跑完 → 夹具规模不对（锚点必须
/// 在语句完成前可达），此时直接失败而不是把「跑完」当成被放弃。
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

/// 取查询结果行集。
async fn expect_rows(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql} must return rows, got {other:?}"),
    }
}

/// 断言一条显式事务内的写语句成功并返回受影响行数。
async fn expect_affected_in_tx(db: &Database, sql: &str, tx: &Transaction) -> u64 {
    match to_response(db.execute_in_tx(sql, tx).await) {
        Response::AffectedRows { count } => count,
        other => panic!("{sql} must succeed with affected rows, got {other:?}"),
    }
}

/// 取显式事务内的查询结果行集。
async fn expect_rows_in_tx(
    db: &Database,
    sql: &str,
    tx: &Transaction,
) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_in_tx(sql, tx).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql} must return rows, got {other:?}"),
    }
}

/// 锚点轮询：活跃事务集合里出现「版本数 ≥2」的事务（复用 [`anchor_tx`] 的
/// 判据）。轮询用 `yield_now` 让出执行权；上限 10 秒只作夹具失控兜底
/// （触发即失败），不是行为判据。
async fn await_anchor_tx(db: &Database) -> u64 {
    tokio::time::timeout(ANCHOR_TIMEOUT, async {
        loop {
            if let Some(tx_id) = anchor_tx(db).await {
                return tx_id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the abandon anchor must be reachable: the statement did not reach it in time")
}

/// T01 / 场景「提交前被取消的语句不提交」+「取消不留下悬空事务」。
///
/// 隐式语句在执行中被放弃后：它不提交，下一条写语句之前完成回收——活跃
/// 事务集合为空、被放弃语句的首个键可以再次插入（键的释放不属本域保证面，
/// 见 spec 已知边界）。
#[tokio::test]
async fn abandoned_implicit_statement_is_not_committed() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    let tx_id = abandon_statement(&db, ABANDONED_SQL).await;
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the abandoned statement's transaction must still be active right after the abandon"
    );

    // 下一条写语句：回收先于它开始，被放弃语句的首个键必须能再次插入
    // （键的释放不属本域保证面，见 spec 已知边界）。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;

    // 被放弃语句写入的第二个键也能再次插入——键的释放由存储域的索引写侧契约
    // 承载（change 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog）：
    // 索引写单元原子化后，回滚/回收释放该语句写入的每一个键。
    expect_affected(&db, "INSERT INTO t2 VALUES (2)").await;

    // 索引里只剩重新插入的两个键——被放弃语句写入的 2000 个键全部已释放。
    let t2 = db.get_table("t2").await.unwrap();
    let entries = t2.index_manager.scan_all().await.unwrap();
    assert_eq!(
        entries.len(),
        2,
        "only the two re-inserted keys may remain in the index, got {} entries",
        entries.len()
    );

    // 取消/回收之后索引结构保持一致：完整性检查无 Error 级发现。
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the reclaimed statement: {:?}",
        report.findings
    );

    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "the abandoned implicit transaction must be reclaimed before the next write statement, \
         active transactions are still {active:?}"
    );
}

/// T01 / 场景「被放弃语句的未提交数据不随 checkpoint 落盘」。
///
/// 制造一次放弃 → `close()`（checkpoint）→ 重开同一个库：被放弃语句写入的
/// 行不存在，被放弃语句的首个键可以再次插入（键的释放不属本域保证面，见
/// spec 已知边界）。
#[tokio::test]
async fn abandoned_statement_does_not_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    abandon_statement(&db, ABANDONED_SQL).await;

    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&db_path(&dir)).await.unwrap();

    let rows = expect_rows(&db, "SELECT id FROM t2").await;
    assert!(
        rows.is_empty(),
        "rows written by the abandoned statement must not survive the checkpoint, got {rows:?}"
    );

    // 源表内容不受影响——checkpoint 没有把库写坏。
    let source_rows = expect_rows(&db, "SELECT id FROM t").await;
    assert_eq!(
        source_rows.len(),
        SOURCE_ROWS as usize,
        "the source table must be intact after the checkpoint"
    );

    // 被放弃语句的首个键可以再次插入（键的释放不属本域保证面，见 spec 已知边界）。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;

    // 第二个键同样可以再次插入（存储域的索引写侧契约：回滚/回收释放该语句
    // 写入的每一个键）。
    expect_affected(&db, "INSERT INTO t2 VALUES (2)").await;
}

/// T01 / 并发回归：同一句柄上的并发语句互不干扰。
///
/// A = `INSERT INTO t2 SELECT id FROM t`（2000 行）在克隆出的同一句柄上
/// 并发执行（与服务器「多连接共享一个 `Arc<Database>`」同形）；B =
/// `INSERT INTO t3 VALUES (1)` 在同一句柄上执行到完成。
///
/// 断言：B 的回收不得中止 A 仍在执行的隐式事务——A 的事务在 B 之后仍然
/// 活跃，且 A 照常提交成功（未被取消的语句行为不变）。
#[tokio::test]
async fn concurrent_statement_does_not_abort_other_statement_tx() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;
    db.execute_sql("CREATE TABLE t3 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // A：在克隆出的同一句柄上并发执行。
    let a = tokio::spawn({
        let db = db.clone();
        async move { db.execute_sql(ABANDONED_SQL).await }
    });

    // 锚点命中 A 的事务——证明 A 在执行期。
    let a_tx = tokio::time::timeout(ANCHOR_TIMEOUT, async {
        loop {
            if let Some(tx_id) = anchor_tx(&db).await {
                break tx_id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect(
        "the concurrent statement's anchor must be reachable: the statement did not reach it in time",
    );

    // B：同一句柄上的另一条写语句（其回收不得误伤 A）。
    expect_affected(&db, "INSERT INTO t3 VALUES (1)").await;

    // A 的事务在 B 之后仍然活跃。
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.contains(&a_tx),
        "the concurrently running statement's transaction must stay active while the other \
         statement runs, active transactions are {active:?}"
    );

    // A 照常提交成功。
    let a_outcome = a
        .await
        .expect("the concurrent statement task must not panic");
    match to_response(a_outcome) {
        Response::AffectedRows { count } => assert_eq!(
            count, SOURCE_ROWS as u64,
            "the concurrently running statement must commit every row"
        ),
        other => panic!("the concurrently running statement must succeed, got {other:?}"),
    }
}

/// T01 / 第三处库属隐式事务站点（ALTER TABLE DROP COLUMN）的回收契约。
///
/// 放弃 `ALTER TABLE t DROP COLUMN v` → 下一条写语句触发回收 → 回收成功
/// 返回（不报 `Reclaim failed`）→ 回收之后活跃集清空（没有悬空事务）→
/// 下一条写语句照常成功。
///
/// Iteration 001（T04.2 / ISS21）追加目录面与数据面形态断言：被放弃的
/// `ALTER TABLE` 不窄化目录（列定义与列数保持语句执行前的状态），数据面
/// 行宽保持语句执行前的列数。
#[tokio::test]
async fn abandoned_alter_table_drop_column_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    // 放弃 ALTER TABLE DROP COLUMN——锚点命中即丢弃语句 future。
    abandon_statement(&db, "ALTER TABLE t DROP COLUMN v").await;

    // 下一条写语句触发回收：必须成功返回（不报 `Reclaim failed`）。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;

    // 回收之后没有悬空事务。
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "the abandoned ALTER TABLE transaction must be reclaimed, active transactions are still \
         {active:?}"
    );

    // 下一条写语句照常成功。
    expect_affected(&db, "INSERT INTO t2 VALUES (2)").await;

    // 目录面：被放弃的 DDL 不窄化目录——列定义与列数保持语句执行前的状态。
    let mut cols = db.table_manager.catalog().scan_columns("t").await.unwrap();
    cols.sort_by_key(|c| c.column_index);
    assert_eq!(
        cols.iter()
            .map(|c| c.column_name.clone())
            .collect::<Vec<_>>(),
        vec!["id", "v"],
        "the abandoned ALTER TABLE must not narrow the catalog columns: {cols:?}"
    );
    let column_count = db
        .table_manager
        .catalog()
        .scan_tables()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.table_name == "t")
        .expect("table t must have a catalog row")
        .column_count;
    assert_eq!(
        column_count, 2,
        "the abandoned ALTER TABLE must not narrow the catalog column count"
    );

    // 数据面：行宽保持语句执行前的列数（2）。
    let rows = match to_response(db.execute_sql("SELECT * FROM t").await) {
        Response::QueryResult { rows } => rows,
        other => panic!("SELECT * FROM t must return rows, got {other:?}"),
    };
    assert!(!rows.is_empty(), "the data plane must not be empty");
    let widths: std::collections::BTreeSet<usize> = rows.iter().map(|r| r.len()).collect();
    assert_eq!(
        widths,
        std::collections::BTreeSet::from([2]),
        "every row must keep the pre-statement width"
    );
}

/// Iteration 001（T02.1）/ 场景「提交已完成但调用方未收到结果时语句已提交」。
///
/// 显式事务不经登记表、从不被回收；`Database::commit` 的原子效果（提交帧
/// fsync → 版本标 committed → 移出活跃集）先于它返回。因此「提交已完成但
/// 结果未送达」时，那一行必须已经持久且可见。
///
/// 窗口工程（三件，缺一不可）：
/// 1. **拉宽尾段**：提交前 `set_checkpoint_bytes_threshold(1)`——提交帧本身会
///    `bytes_appended.fetch_add`，阈值 1 必然达阈；提交后活跃集为空（R3 不
///    顺延）→ `Database::commit` 尾段变成一次真 checkpoint（多个 await），
///    「提交已原子完成、future 尚未返回」的窗口才有 await 点可达。
/// 2. **锚点**：轮询 `active_transactions()` 直到**不含**该 tx id——移出活跃集
///    是提交原子完成的判据。轮询用 `yield_now` 让出执行权，上限 10 秒只作
///    夹具失控兜底（触发即失败），不是行为判据。
/// 3. **判别位**：`commit_returned` 记录 commit future 是否已返回。锚点命中时
///    该位为真 → 放弃点落在提交完成**之后**、场景未被复现 → 用例显式失败。
#[tokio::test]
async fn committed_statement_survives_undelivered_result() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();
    db.execute_in_tx("INSERT INTO t2 VALUES (1)", &tx)
        .await
        .unwrap();

    // 拉宽提交尾段——阈值必须在提交之前设置（准备阶段的追加字节会被
    // checkpoint 归零）。
    db.set_checkpoint_bytes_threshold(1);

    // 判别位：commit future 是否已经返回。
    let commit_returned = Arc::new(AtomicBool::new(false));
    let mut commit = Box::pin({
        let db = db.clone();
        let commit_returned = commit_returned.clone();
        async move {
            let outcome = db.commit(tx).await;
            commit_returned.store(true, Ordering::SeqCst);
            outcome
        }
    });

    // 锚点：tx id 离开活跃集 = 提交已原子完成。
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if !db
                    .transaction_manager
                    .active_transactions()
                    .await
                    .contains(&tx_id)
                {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "the commit anchor must be reachable: the commit did not leave the active set in time",
        )
    });

    tokio::select! {
        biased;
        _ = &mut anchor => {}
        outcome = &mut commit => panic!(
            "the commit future returned before the undelivered-result window was reached: {outcome:?}"
        ),
    }

    assert!(
        !commit_returned.load(Ordering::SeqCst),
        "the commit future already returned: the undelivered-result scenario was not reproduced"
    );

    // 锚点任务结束（它借用 `db`，先于会话收尾释放）。
    drop(anchor);

    // 放弃 commit future：提交已原子完成，结果未送达。
    drop(commit);

    // (a) 该行在同一会话读得到。
    let rows = expect_rows(&db, "SELECT id FROM t2").await;
    assert_eq!(
        rows.len(),
        1,
        "the committed row must be readable in the same session, got {rows:?}"
    );

    // (c) 活跃事务集合为空（没有悬空事务）。
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "no dangling transaction may remain after the undelivered commit, active transactions are {active:?}"
    );

    // (d) 放弃之后的后续写语句正常成功（不报 `Reclaim failed`）。
    expect_affected(&db, "INSERT INTO t2 VALUES (2)").await;

    // (b) close() 后重开该行仍在。
    db.close().await.unwrap();
    drop(db);
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let rows = expect_rows(&db, "SELECT id FROM t2").await;
    assert_eq!(
        rows.len(),
        2,
        "the committed row must survive the reopen, got {rows:?}"
    );
}

/// Iteration 001（T02.2）/ 场景「显式事务内的语句被放弃后事务仍然活跃」。
///
/// 显式事务的语句路径不 abort、不 commit、不建守卫、不触发回收——语句
/// future 被放弃时事务状态不变，已写入的部分留在事务内。库在任何取消路径上
/// 都不代替调用方提交或回滚：继续还是回滚由调用方决定。
///
/// 夹具以 `ReadCommitted` 打开：「事务内读得到自己的未提交写、事务外读不到
/// 未提交的写」只在 RC 下可观察（`RepeatableRead` 的 `statement_snapshot` 返回
/// `None`，扫描路径不做可见性过滤）。
#[tokio::test]
async fn abandoned_statement_in_explicit_tx_keeps_tx_open() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup_with_isolation(&dir, IsolationLevel::ReadCommitted).await;

    // --- 场景 1：事务仍然活跃且可用 ---
    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();

    let mut statement = Box::pin(db.execute_in_tx(ABANDONED_SQL, &tx));
    let mut anchor = Box::pin(await_anchor_tx(&db));
    tokio::select! {
        biased;
        anchored = &mut anchor => {
            assert_eq!(
                anchored, tx_id,
                "the anchored transaction must be the explicit transaction under test"
            );
        }
        outcome = &mut statement => panic!(
            "the statement finished before the abandon anchor was reached: {outcome:?}"
        ),
    }
    // 放弃语句 future——执行器停在执行期，事务状态不变。
    drop(anchor);
    drop(statement);

    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.contains(&tx_id),
        "the explicit transaction must stay active after the abandon, active transactions are {active:?}"
    );

    // 事务仍可用于后续语句。
    expect_affected_in_tx(&db, "INSERT INTO t2 VALUES (9999)", &tx).await;

    // 库没有代替调用方提交或回滚：事务外读不到该语句已写入的行。
    let outside = expect_rows(&db, "SELECT id FROM t2").await;
    assert!(
        outside.is_empty(),
        "the abandoned statement's writes must not be visible outside the transaction, got {outside:?}"
    );
    // 事务内读得到——该行仍在事务里。
    let inside = expect_rows_in_tx(&db, "SELECT id FROM t2", &tx).await;
    assert!(
        !inside.is_empty(),
        "the abandoned statement's writes must still be visible inside the transaction"
    );

    // --- 场景 2：调用方回滚撤销被取消语句的效果 ---
    db.rollback(tx).await.unwrap();
    let after_rollback = expect_rows(&db, "SELECT id FROM t2").await;
    assert!(
        after_rollback.is_empty(),
        "the caller's rollback must undo the abandoned statement's writes, got {after_rollback:?}"
    );
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "the rolled-back transaction must be gone, active transactions are {active:?}"
    );

    // 回滚之后，被取消语句写入的键可以再次插入（存储域的索引写侧契约：回滚
    // 释放该语句写入的每一个键）——第一个键与第二个键都不再占用索引。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;
    expect_affected(&db, "INSERT INTO t2 VALUES (2)").await;

    // 取消/回滚之后索引结构保持一致：完整性检查无 Error 级发现。
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the rolled-back statement: {:?}",
        report.findings
    );

    // --- 场景 3：调用方提交保留被取消语句的效果 ---
    //
    // 场景 3 用第二个写入目标 `t3`：场景 2 的回滚之后，被取消语句写入的键已
    // 被释放（存储域的索引写侧契约），同表同键的再次写入不再撞 DuplicateKey；
    // 本场景要证明的是「提交保留被取消语句的效果」，与键的释放无关，故仍用
    // 第二个写入目标保持场景形状。
    db.execute_sql("CREATE TABLE t3 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();

    let mut statement = Box::pin(db.execute_in_tx("INSERT INTO t3 SELECT id FROM t", &tx));
    let mut anchor = Box::pin(await_anchor_tx(&db));
    tokio::select! {
        biased;
        anchored = &mut anchor => {
            assert_eq!(
                anchored, tx_id,
                "the anchored transaction must be the explicit transaction under test"
            );
        }
        outcome = &mut statement => panic!(
            "the statement finished before the abandon anchor was reached: {outcome:?}"
        ),
    }
    drop(anchor);
    drop(statement);

    db.commit(tx).await.unwrap();
    let after_commit = expect_rows(&db, "SELECT id FROM t3").await;
    assert!(
        !after_commit.is_empty(),
        "the caller's commit must keep the abandoned statement's writes"
    );

    // close() 后重开仍在。
    db.close().await.unwrap();
    drop(db);
    let db = setup_open_only(&dir, IsolationLevel::ReadCommitted).await;

    let rows = expect_rows(&db, "SELECT id FROM t3").await;
    assert!(
        !rows.is_empty(),
        "the committed rows must survive the reopen, got {rows:?}"
    );
}

/// 重开既有库（不重建夹具数据）。
async fn setup_open_only(dir: &TempDir, isolation: IsolationLevel) -> Database {
    Database::open_with_isolation(&db_path(dir), isolation)
        .await
        .unwrap()
}

/// Iteration 001（T02.3）/ 场景「超时不代替回滚」。
///
/// 超时是语句 future 被放弃的另一种来源（丢弃与超时在库入口是同一类事件；
/// 连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达
/// 的结果）。显式事务路径不因语句结束而收尾事务——超时到期后事务仍然活跃、
/// 行仍在事务内，只有调用方显式回滚才撤销它。
///
/// 超时时长只是**取消触发源**，不是判据：断言全部落在可观察状态上（超时返回
/// `Elapsed`、事务仍活跃、行在事务内、回滚才撤销、提交则持久）。另起一个
/// 锚点观察任务（只克隆 `Database`——`Transaction` 不是 `Clone`）确认「超时
/// 到期前语句已推进」，避免把「语句根本没跑起来就超时了」误判为通过。
#[tokio::test]
async fn timeout_in_explicit_tx_is_not_a_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup_with_isolation(&dir, IsolationLevel::ReadCommitted).await;

    // --- (a) 超时放弃执行中的语句 ---
    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();

    // 锚点观察任务：活跃事务集合里出现「版本数 ≥2」的事务。
    let advanced = Arc::new(AtomicBool::new(false));
    let observer_db = db.clone();
    let observer = tokio::spawn({
        let advanced = advanced.clone();
        async move {
            tokio::time::timeout(ANCHOR_TIMEOUT, async {
                loop {
                    if anchor_tx(&observer_db).await.is_some() {
                        advanced.store(true, Ordering::SeqCst);
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_ok()
        }
    });

    // 超时时长：远大于语句推进到第 2 个版本所需的时间（毫秒级），远小于该
    // 语句完成所需的时间（秒级）。它只决定取消何时发生，不参与任何断言。
    let outcome =
        tokio::time::timeout(TIMEOUT_CANCEL_WINDOW, db.execute_in_tx(ABANDONED_SQL, &tx)).await;
    assert!(
        outcome.is_err(),
        "the timeout must cancel the in-flight statement, got {outcome:?}"
    );

    // 锚点观察任务：超时到期前语句已推进（夹具规模不对，不是产品结论）。
    assert!(
        observer.await.unwrap(),
        "the anchor observer must have seen the statement advance before the timeout fired"
    );
    assert!(
        advanced.load(Ordering::SeqCst),
        "the statement must have advanced before the timeout fired"
    );

    // --- (b) 事务仍然活跃 ---
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.contains(&tx_id),
        "the explicit transaction must stay active after the timeout, active transactions are {active:?}"
    );

    // --- (c) 该行仍在事务内：事务内读得到、事务外读不到 ---
    let inside = expect_rows_in_tx(&db, "SELECT id FROM t2", &tx).await;
    assert!(
        !inside.is_empty(),
        "the timed-out statement's writes must still be visible inside the transaction"
    );
    let outside = expect_rows(&db, "SELECT id FROM t2").await;
    assert!(
        outside.is_empty(),
        "the timed-out statement's writes must not be visible outside the transaction, got {outside:?}"
    );

    // --- (d) 只有调用方显式回滚才撤销它 ---
    db.rollback(tx).await.unwrap();
    let after_rollback = expect_rows(&db, "SELECT id FROM t2").await;
    assert!(
        after_rollback.is_empty(),
        "the caller's rollback must undo the timed-out statement's writes, got {after_rollback:?}"
    );
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.is_empty(),
        "the rolled-back transaction must be gone, active transactions are {active:?}"
    );

    // 回滚之后，被超时放弃的语句写入的键可以再次插入（存储域的索引写侧契约：
    // 回滚释放该语句写入的每一个键）——第一个键与第二个键都不再占用索引。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;
    expect_affected(&db, "INSERT INTO t2 VALUES (2)").await;

    // --- (e) 调用方显式提交则该效果持久 ---
    //
    // 用第二个写入目标 `t3`：本场景要证明的是「提交则持久」，与键的释放无关；
    // 换目标表保持场景形状（(d) 的回滚已释放 `t2` 的键，同表同键的再次写入
    // 不再撞 DuplicateKey，但本场景不依赖该事实）。
    db.execute_sql("CREATE TABLE t3 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    let tx_id = tx.id();

    let observer_db = db.clone();
    let observer = tokio::spawn({
        let advanced = advanced.clone();
        async move {
            tokio::time::timeout(ANCHOR_TIMEOUT, async {
                loop {
                    if anchor_tx(&observer_db).await.is_some() {
                        advanced.store(true, Ordering::SeqCst);
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_ok()
        }
    });
    let outcome = tokio::time::timeout(
        TIMEOUT_CANCEL_WINDOW,
        db.execute_in_tx("INSERT INTO t3 SELECT id FROM t", &tx),
    )
    .await;
    assert!(
        outcome.is_err(),
        "the timeout must cancel the in-flight statement, got {outcome:?}"
    );
    assert!(
        observer.await.unwrap(),
        "the anchor observer must have seen the statement advance before the timeout fired"
    );

    // 超时后事务仍然活跃（与 (b) 同型）。
    let active = db.transaction_manager.active_transactions().await;
    assert!(
        active.contains(&tx_id),
        "the explicit transaction must stay active after the timeout, active transactions are {active:?}"
    );

    db.commit(tx).await.unwrap();
    let after_commit = expect_rows(&db, "SELECT id FROM t3").await;
    assert!(
        !after_commit.is_empty(),
        "the caller's commit must keep the timed-out statement's writes"
    );

    // close() 后重开仍在。
    db.close().await.unwrap();
    drop(db);
    let db = setup_open_only(&dir, IsolationLevel::ReadCommitted).await;

    let rows = expect_rows(&db, "SELECT id FROM t3").await;
    assert!(
        !rows.is_empty(),
        "the committed rows must survive the reopen, got {rows:?}"
    );
}

/// Iteration 001（T02.4）/ 场景「被放弃的语句之后会话继续可用」。
///
/// 会话级对象（WAL writer、WAL buffer、缓冲池）由 `open_internal` 构造，不随
/// 语句生死；语句级只读预取任务自然终止。放弃一条语句后，同一会话应继续正常
/// 写入并提交，`close()` 后重开数据完整。
///
/// 写入键取 9001/9002（落在被放弃语句写入的 1..2000 之外）：被放弃语句的键在
/// 回收后仍有一个残留的主键索引条目（见 Act Response 的 Experience
/// Candidates），同键再插会撞 DuplicateKey——那与本用例要证明的「会话继续
/// 可用」无关。
///
/// 同一会话形态的断言放在 `close()` 之前：回收完成后，同一会话读不到被放弃
/// 语句写入的行（键 1..2000）——这是规格场景 1 保留的「行不可见」在同会话
/// 形态上的直接断言（此前只有重开后的形态被断言）。断言对象是行不可见，键
/// 是否释放不属本域的保证面。
#[tokio::test]
async fn session_keeps_working_after_abandoned_statement() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir).await;

    db.execute_sql("CREATE TABLE t3 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // 放弃一条隐式语句。
    abandon_statement(&db, ABANDONED_SQL).await;

    // 同一会话继续写入若干行——每条都提交成功。
    expect_affected(&db, "INSERT INTO t2 VALUES (9001)").await;
    expect_affected(&db, "INSERT INTO t2 VALUES (9002)").await;
    expect_affected(&db, "INSERT INTO t3 VALUES (9003)").await;

    // 同一会话形态（规格场景 1 的 WHEN 是「随后调用方在同一个 `Database` 句柄
    // 上执行下一条写语句」）：回收完成后，同一会话读不到被放弃语句写入的行
    // （键 1..2000）。
    //
    // 断言对象是「行不可见」——键是否释放不属本域的保证面（已知边界），这里
    // 既不断言键一定可复用，也不断言键一定被占用。读回的行集是判定依据，
    // 不用内部状态、哈希、指纹或时间顺序。
    let same_session_rows = expect_rows(&db, "SELECT id FROM t2").await;
    assert!(
        !same_session_rows.is_empty(),
        "the rows written after the abandon must be readable in the same session, got {same_session_rows:?}"
    );
    assert!(
        same_session_rows
            .iter()
            .all(|row| row.first().and_then(|value| value.as_i64()).is_some_and(|id| id > SOURCE_ROWS)),
        "the abandoned statement's rows must not be visible in the same session, got {same_session_rows:?}"
    );

    // close()（checkpoint）→ 重开同一个库。
    db.close().await.unwrap();
    drop(db);
    let db = Database::open(&db_path(&dir)).await.unwrap();

    // 后续写入的行完整可读。
    let rows = expect_rows(&db, "SELECT id FROM t2").await;
    assert_eq!(
        rows.len(),
        2,
        "the rows written after the abandon must survive the reopen, got {rows:?}"
    );
    let rows = expect_rows(&db, "SELECT id FROM t3").await;
    assert_eq!(
        rows.len(),
        1,
        "the rows written after the abandon must survive the reopen, got {rows:?}"
    );

    // 被放弃语句写入的行不存在。
    let rows = expect_rows(&db, "SELECT id FROM t2 WHERE id <= 2000").await;
    assert!(
        rows.is_empty(),
        "the abandoned statement's rows must not survive the checkpoint, got {rows:?}"
    );

    // 完整性检查无 Error 级发现。
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the abandoned statement: {:?}",
        report.findings
    );
}
