//! change 2026-10-03-recovery-tail-persistence（R1/R3 见证）：
//! 崩溃恢复重放收尾把内存 `data_page_tail` 写回 catalog——恢复 → close →
//! 重开后持久 tail 与数据链 terminal 一致（`rtsql check` 无 info、dump 无
//! 链告警）；重复恢复幂等；`redo_count == 0` 打开路径零变化。
//!
//! 夹具纪律对齐 `check_command_test::seed_pending_wal`：CREATE TABLE 后
//! 显式 `checkpoint()` 固化 catalog（DDL 无 WAL 记录）→ post-checkpoint
//! 写入进 WAL → `wal_buffer.shutdown()` → drop 不 close（崩溃，脏页尽失）。
//!
//! 页几何算术（DELETE-only 与 INSERT 驱动共用）：4096B 页 − 16B header =
//! 4080B 数据区；行 (id INT, v STRING(4030 字符))：tuple = 9 + 4033 =
//! 4042B，slot_data = 22 + 4042 = 4064B ≤ 4074B（内联）；单行页剩余 =
//! 4080 − 4064 − 6 = 10B < 28B（墓碑 = 22B header + 0 tuple + 6B slot 目录）
//! → DELETE 墓碑必然分配新页；每页恰一行，5 行跨 5 页。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("tail.db")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement failed: {sql:?} -> {resp:?}"
    );
}

async fn count_t(db: &Database) -> u64 {
    match db.execute_sql("SELECT COUNT(*) FROM t").await {
        Response::QueryResult { rows } => rows[0][0].as_u64().unwrap(),
        other => panic!("COUNT(*): {other:?}"),
    }
}

async fn assert_chain_complete(db: &Database) {
    let status = db.table_manager.data_chain_status("t").await.unwrap();
    assert!(
        status.is_complete(),
        "chain must terminate at the (persisted) tail: terminal={:?} tail={:?} cyclic={}",
        status.terminal,
        status.tail,
        status.cyclic
    );
}

/// 大行夹具建表 + checkpoint 固化（DDL 无 WAL 记录）。
async fn seed_schema(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, v STRING(4050))").await;
    db.checkpoint().await.unwrap();
}

/// 崩溃模拟（同 seed_pending_wal 纪律）：WAL 落盘（shutdown ≠ close）后 drop。
async fn crash(db: Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

fn spawn_cli(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary")
}

fn wait_cli(mut child: Child) -> (Option<i32>, String, String) {
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(60) {
            let _ = child.kill();
            panic!("rtsql did not exit within 60s (hung)");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut out = String::new();
    let mut err = String::new();
    stdout.read_to_string(&mut out).unwrap();
    stderr.read_to_string(&mut err).unwrap();
    (status.code(), out, err)
}

fn run_cli(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    wait_cli(spawn_cli(dir, args))
}

/// R1/S1（INSERT 驱动跨页增长）：恢复 → close → 重开后持久 tail == 链
/// terminal（is_complete、行数一致、CLI check 干净单行 exit 0、dump 无链告警）。
#[tokio::test]
async fn recovery_persists_tail_two_state_consistent() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    for i in 0..5 {
        exec_ok(
            &db,
            &format!("INSERT INTO t VALUES ({i}, '{}')", "x".repeat(4030)),
        )
        .await;
    }

    // 夹具自检：链至少跨 4 页（head + 3 新页），tail 已前进。
    {
        let meta = db.table_manager.get_table("t").await.unwrap();
        let status = db.table_manager.data_chain_status("t").await.unwrap();
        assert!(
            status.terminal.unwrap().0 - meta.data_page_head.0 >= 3,
            "fixture must span >= 4 data pages: terminal={:?} tail={:?}",
            status.terminal,
            status.tail
        );
    }

    crash(db).await;

    // 恢复 → close（落盘，含 tail 写回）→ 重开（clean）：两态一致。
    // close() 只做 checkpoint；flock 随 Database drop 才释放，先 drop 再重开。
    let recovered = Database::open(&path).await.unwrap();
    recovered.close().await.unwrap();
    drop(recovered);
    let reopened = Database::open(&path).await.unwrap();
    assert_chain_complete(&reopened).await;
    assert_eq!(
        count_t(&reopened).await,
        5,
        "rows survive two-state roundtrip"
    );
    reopened.close().await.unwrap();
    drop(reopened);

    // CLI check：exit 0 且 stdout 恰一行确认（无 info 行）。
    let path_str = path.to_str().unwrap();
    let (code, out, err) = run_cli(dir.path(), &["check", path_str]);
    assert_eq!(code, Some(0), "check stderr: {err}");
    assert_eq!(
        out.lines().count(),
        1,
        "recovered db must check clean: {out:?}"
    );
    assert!(out.contains("check ok"), "confirmation line: {out:?}");

    // CLI dump：exit 0、stderr 无链完整性告警、stdout 为纯 SQL。
    let (code, out, err) = run_cli(dir.path(), &["dump", path_str]);
    assert_eq!(code, Some(0), "dump stderr: {err}");
    assert!(
        !err.contains("damaged") && !err.contains("warning"),
        "dump must not warn on a recovered db: {err:?}"
    );
    assert!(out.contains("CREATE TABLE"), "dump emits SQL: {out:?}");
}

/// R1/S4（crash-during-recovery 幂等）：首次恢复后不 close 再崩溃 → 重开再
/// 恢复收敛（WAL 未截断、重放幂等），close 后两态一致。
#[tokio::test]
async fn repeated_recovery_converges() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    for i in 0..5 {
        exec_ok(
            &db,
            &format!("INSERT INTO t VALUES ({i}, '{}')", "x".repeat(4030)),
        )
        .await;
    }
    crash(db).await;

    // 恢复 #1：会话内内存 tail 已收敛（terminal == 内存 tail）。
    let first = Database::open(&path).await.unwrap();
    assert_chain_complete(&first).await;
    assert_eq!(count_t(&first).await, 5, "recovery #1 count");
    crash(first).await;

    // 恢复 #2：重放幂等，行集一致。
    let second = Database::open(&path).await.unwrap();
    assert_eq!(count_t(&second).await, 5, "recovery #2 count");
    second.close().await.unwrap();
    drop(second);

    // 重开（clean）：持久 tail == terminal。
    let reopened = Database::open(&path).await.unwrap();
    assert_chain_complete(&reopened).await;
    assert_eq!(count_t(&reopened).await, 5, "post-close count");
    reopened.close().await.unwrap();
    drop(reopened);
}

/// R1/S3（redo_count == 0 零变化）：无崩溃库的正常写入往返不变，打开路径
/// 不触碰 tail 持久化。
#[tokio::test]
async fn clean_open_untouched() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    for i in 0..3 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, 'row-{i}')")).await;
    }
    db.close().await.unwrap();
    drop(db);

    let reopened = Database::open(&path).await.unwrap();
    assert_chain_complete(&reopened).await;
    assert_eq!(count_t(&reopened).await, 3, "clean roundtrip count");
    exec_ok(&reopened, "INSERT INTO t VALUES (9, 'row-9')").await;
    reopened.close().await.unwrap();
    drop(reopened);

    let final_db = Database::open(&path).await.unwrap();
    assert_chain_complete(&final_db).await;
    assert_eq!(count_t(&final_db).await, 4, "write after clean reopen");
    final_db.close().await.unwrap();
    drop(final_db);
}

/// R1（T1-R2 DELETE-only 见证）：仅 post-checkpoint 已提交 DELETE 的表——
/// 墓碑写入触发新页分配、内存 tail 前进而**不**经 `redo_tuple_at_row_id`
/// （Delete 臂绕行）——恢复收尾同样写回 tail；恢复 → close → 重开后
/// `is_complete()` 成立、删后行数一致、`rtsql check` exit 0（容忍 info 行：
/// 运行期墓碑页成为 unreferenced 页属 MS27 已知边界，不断言「无 info」）。
#[tokio::test]
async fn delete_only_replay_allocating_page_persists_tail() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    seed_schema(&db).await;
    for i in 0..3 {
        exec_ok(
            &db,
            &format!("INSERT INTO t VALUES ({i}, '{}')", "x".repeat(4030)),
        )
        .await;
    }
    // checkpoint 固化：tail 页（单行 4064B）空闲 10B < 28B 墓碑 footprint。
    db.checkpoint().await.unwrap();

    // 夹具自检：DELETE 后链必须增长（墓碑分配新页），否则几何失准。
    let before = db.table_manager.data_chain_status("t").await.unwrap();
    exec_ok(&db, "DELETE FROM t WHERE id = 2").await;
    let after = db.table_manager.data_chain_status("t").await.unwrap();
    assert!(
        after.terminal.unwrap().0 > before.tail.0,
        "fixture geometry: tombstone must allocate a new page \
         (before tail: {:?}, after terminal: {:?})",
        before.tail,
        after.terminal
    );
    crash(db).await;

    // 恢复（墓碑重放分配新页、内存 tail 前进）→ close（tail 写回落盘）→
    // 重开（clean）：持久 tail == terminal。
    let recovered = Database::open(&path).await.unwrap();
    recovered.close().await.unwrap();
    drop(recovered);
    let reopened = Database::open(&path).await.unwrap();
    assert_chain_complete(&reopened).await;
    assert_eq!(count_t(&reopened).await, 2, "deleted row stays deleted");
    reopened.close().await.unwrap();
    drop(reopened);

    let path_str = path.to_str().unwrap();
    let (code, _out, err) = run_cli(dir.path(), &["check", path_str]);
    assert_eq!(code, Some(0), "check stderr: {err}");
}
