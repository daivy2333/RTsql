//! change 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog /
//! Iteration 000（T01–T03）：索引写侧原子性（承载 ISS20）。
//!
//! 夹具：`IndexManager::arm_write_pause` 装载的写侧暂停点，语义位置固定为
//! 「结构变更已生效、根页 id 尚未发布」——修复前后一致，因此同一个夹具在
//! 修复前后落在同一个窗口上。暂停点的「到达」是写单元到达该位置的累计
//! 次数（可轮询），「放行」允许当前到达继续，「卸载」让之后的写单元不再
//! 被停住。
//!
//! 锚点：到达计数 + `tokio::task::yield_now()` 让出执行权；10 秒上限只作
//! 夹具失控兜底（触发即判失败），不是行为判据。取消 = 丢弃语句 future——
//! tokio 的 blocking 任务在 JoinHandle 被丢弃后仍会跑完，这正是修复后
//! 「写单元整体落地」得以成立的基础。
//!
//! 断言全部落在可观察状态上（索引查找、SQL 执行结果、完整性报告），
//! 判定逻辑在 `cargo test` 集合内，无框架外判定层。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::WritePauseHandle;
use rtsql::storage::WritePausePosition;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use tempfile::TempDir;

/// 单叶恰好装满的行数：页 4096 字节、叶条目 38+6 字节、
/// `free_space = 4080 - 44n` ⇒ n ≤ 92；第 93 个键触发叶分裂与换根
/// （`MIN_KEYS = 46` ⇒ 分裂为 46/47）。
const LEAF_FULL_ROWS: i64 = 92;
/// 锚点观察上限——夹具失控的兜底，触发即判失败（不是产品结论）。
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("index_write_atomicity.db")
}

/// 建库 + 建表。
async fn setup(dir: &TempDir, tables: &[&str]) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    for table in tables {
        db.execute_sql(&format!("CREATE TABLE {table} (id INT PRIMARY KEY)"))
            .await
            .unwrap();
    }
    db
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

/// 断言完整性检查无 Error 级发现。
async fn expect_no_integrity_error(db: &Database, context: &str) {
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level integrity findings {context}: {:?}",
        report.findings
    );
}

/// 等到写单元累计到达暂停点 `n` 次。
///
/// 轮询用 `yield_now` 让出执行权；上限只作夹具失控兜底（触发即失败），
/// 不是行为判据。
async fn await_arrivals(pause: &WritePauseHandle, n: u64) {
    tokio::time::timeout(ANCHOR_TIMEOUT, async {
        loop {
            if pause.arrivals() >= n {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the write-pause arrival must be reachable: the writer did not reach it in time");
}

/// 取消一条正在写入的语句：等到写单元第 `arrival` 次到达暂停点即丢弃语句
/// future（取消），随后放行被停住的写单元并卸载暂停点。
///
/// 语句在到达之前跑完 = 夹具规模不对（暂停点必须可达），此时直接失败，
/// 而不是把「跑完」当成被取消。
async fn cancel_at_arrival<S>(pause: &WritePauseHandle, mut statement: Pin<Box<S>>, arrival: u64)
where
    S: Future,
    S::Output: std::fmt::Debug,
{
    let mut anchor = Box::pin(await_arrivals(pause, arrival));
    tokio::select! {
        biased;
        _ = &mut anchor => {},
        outcome = &mut statement => panic!(
            "the statement finished before write-pause arrival {arrival} was reached: {outcome:?}"
        ),
    }

    // 取消语句 future——写单元停在「结构变更已生效、根页 id 尚未发布」。
    drop(statement);
    pause.release();
    pause.disarm();
}

/// 夹具自检：单叶恰好装满 92 行，第 93 个键触发叶分裂与换根。
///
/// 换根事实用根页 id 变化观察（根页 id 是索引结构的公开可观测量）。
#[tokio::test]
async fn leaf_fills_at_92_rows_and_the_93rd_key_splits_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir, &["t"]).await;

    for id in 1..=LEAF_FULL_ROWS {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id})"))
            .await
            .unwrap();
    }

    let table = db.get_table("t").await.unwrap();
    let root_before = table.index_manager.root_page_id();

    db.execute_sql("INSERT INTO t VALUES (93)").await.unwrap();

    let root_after = table.index_manager.root_page_id();
    assert_ne!(
        root_before, root_after,
        "the 93rd key must split the leaf and change the root page id \
         (92 rows must exactly fill one leaf)"
    );
}

/// T01 / 场景「取消的写不留下半写的索引」。
///
/// 建单列主键表 → 插 92 行（单叶恰好装满）→ 装载暂停点 → 后台插第 93 个
/// 键（触发叶分裂与换根）→ 轮询到达 → 丢弃语句 future（取消）→ 放行 →
/// 下一条写语句触发回收（隐式事务回收）→ 断言：键 1..92 全部可查、键 93
/// 可再次插入、完整性检查无 Error 级发现、进程未 abort。
#[tokio::test]
async fn cancelled_write_keeps_index_self_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir, &["t", "t2"]).await;

    for id in 1..=LEAF_FULL_ROWS {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id})"))
            .await
            .unwrap();
    }

    let table = db.get_table("t").await.unwrap();
    let pause = table
        .index_manager
        .arm_write_pause(WritePausePosition::AfterStructureChangeBeforeRootPublish);

    // 后台语句：第 93 个键触发叶分裂与换根；到达暂停点即取消。
    cancel_at_arrival(
        &pause,
        Box::pin(db.execute_sql("INSERT INTO t VALUES (93)")),
        1,
    )
    .await;

    // 下一条写语句触发隐式事务回收——回收先于本语句的隐式事务。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;

    // 键 1..92 全部可查：索引结构自洽（换根没有丢失）。
    for id in 1..=LEAF_FULL_ROWS {
        let key = id.to_be_bytes();
        assert!(
            table.index_manager.search(&key).await.unwrap().is_some(),
            "key {id} must stay reachable in the index after the cancelled write"
        );
        let rows = expect_rows(&db, &format!("SELECT id FROM t WHERE id = {id}")).await;
        assert_eq!(
            rows.len(),
            1,
            "key {id} must stay reachable through the SQL entry point after the cancelled write"
        );
    }

    // 键 93 可再次插入（不报重复键）。
    expect_affected(&db, "INSERT INTO t VALUES (93)").await;

    // 索引结构自洽：完整性检查无 Error 级发现。
    expect_no_integrity_error(&db, "after the cancelled write").await;
}

/// T03 / 场景「并发写入不产生结构分叉」。
///
/// 两个写者并发写同一个索引：全部键可查、完整性检查无 Error 级发现。
#[tokio::test]
async fn concurrent_writers_do_not_fork_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir, &["t", "src1", "src2"]).await;

    // 两个源表各 300 行，键区间不重叠。
    for id in 1..=300 {
        db.execute_sql(&format!("INSERT INTO src1 VALUES ({id})"))
            .await
            .unwrap();
        db.execute_sql(&format!("INSERT INTO src2 VALUES ({})", id + 1000))
            .await
            .unwrap();
    }

    // 同一个库句柄上的两个并发写者（与服务器「多连接共享一个 Arc<Database>」同形）。
    let (a, b) = tokio::join!(
        db.execute_sql("INSERT INTO t SELECT id FROM src1"),
        db.execute_sql("INSERT INTO t SELECT id FROM src2"),
    );
    assert!(a.is_ok(), "the first concurrent writer must succeed: {a:?}");
    assert!(
        b.is_ok(),
        "the second concurrent writer must succeed: {b:?}"
    );

    // 全部 600 个键都在索引里（每个写入的行一个条目）。
    let table = db.get_table("t").await.unwrap();
    let entries = table.index_manager.scan_all().await.unwrap();
    assert_eq!(
        entries.len(),
        600,
        "the index must hold exactly one entry per row written by the concurrent writers"
    );
    for id in (1..=300).chain(1001..=1300) {
        let key = (id as i64).to_be_bytes();
        assert!(
            table.index_manager.search(&key).await.unwrap().is_some(),
            "key {id} must be in the index after the concurrent writers"
        );
    }

    // 索引结构自洽：完整性检查无 Error 级发现。
    expect_no_integrity_error(&db, "after the concurrent writers").await;
}

/// T03 / 场景「显式事务内取消的多行 INSERT 回滚后释放全部键」。
///
/// 显式事务内的多行 INSERT 写入两个键，在第二个键的写单元上被取消 → 调用方
/// 回滚 → 被取消语句写入的每一个键都能再次插入。
#[tokio::test]
async fn cancelled_multi_row_insert_in_explicit_tx_releases_every_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir, &["t"]).await;

    let table = db.get_table("t").await.unwrap();
    let pause = table
        .index_manager
        .arm_write_pause(WritePausePosition::AfterStructureChangeBeforeRootPublish);

    let tx = db.transaction_manager.begin().await;

    // 第 1 次到达放行（第一个键的写单元走完），第 2 次到达取消（第二个键）。
    pause.release();
    cancel_at_arrival(
        &pause,
        Box::pin(db.execute_in_tx("INSERT INTO t VALUES (1), (2)", &tx)),
        2,
    )
    .await;

    // 显式事务由调用方回滚。
    db.rollback(tx).await.unwrap();

    // 被取消语句写入的每一个键都能再次插入。
    expect_affected(&db, "INSERT INTO t VALUES (1)").await;
    expect_affected(&db, "INSERT INTO t VALUES (2)").await;

    // 索引结构自洽：完整性检查无 Error 级发现。
    expect_no_integrity_error(&db, "after the cancelled multi-row insert").await;
}

/// T03 / 场景「隐式路径回收后释放全部键」。
///
/// 隐式事务内的多行 INSERT 写入两个键，在第二个键的写单元上被取消 → 下一条
/// 写语句触发回收 → 被取消语句写入的每一个键都能再次插入。
#[tokio::test]
async fn cancelled_multi_row_insert_in_implicit_tx_releases_every_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = setup(&dir, &["t", "t2"]).await;

    let table = db.get_table("t").await.unwrap();
    let pause = table
        .index_manager
        .arm_write_pause(WritePausePosition::AfterStructureChangeBeforeRootPublish);

    // 第 1 次到达放行（第一个键的写单元走完），第 2 次到达取消（第二个键）。
    pause.release();
    cancel_at_arrival(
        &pause,
        Box::pin(db.execute_sql("INSERT INTO t VALUES (1), (2)")),
        2,
    )
    .await;

    // 下一条写语句触发隐式事务回收——回收先于本语句的隐式事务。
    expect_affected(&db, "INSERT INTO t2 VALUES (1)").await;

    // 被取消语句写入的每一个键都能再次插入。
    expect_affected(&db, "INSERT INTO t VALUES (1)").await;
    expect_affected(&db, "INSERT INTO t VALUES (2)").await;

    // 索引结构自洽：完整性检查无 Error 级发现。
    expect_no_integrity_error(&db, "after the reclaimed multi-row insert").await;
}
