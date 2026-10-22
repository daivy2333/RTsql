//! T2（change 2026-10-04-external-review-gaps R2）：恢复会话 WAL 收敛
//! （含零 redo 形态）。
//!
//! 病灶：未提交事务 SIGKILL 后，未提交元组只存在于 WAL 不在数据页；纯读
//! 读写打开的恢复全程零页写入零 WAL 追加（redo_count=0）→ close 走 MS18
//! 零工作跳过 → WAL 滞留 → `check`/`--readonly` 永久 exit 1，`checkpoint`
//! 报 "no pending work"，死循环。
//!
//! 目标语义（delta spec `wal-recovery-replay-integrity` R「恢复会话 WAL 收敛」
//! + `ms07-rest-tx-checkpoint-pushdown`「Checkpoint 零工作跳过」MODIFIED）：
//! - 打开时 WAL 曾含事务记录（任一非 Checkpoint 记录）的读写会话，其
//!   close()/checkpoint() SHALL 执行完整 checkpoint 截断 WAL；
//! - WAL 从未含事务记录的形态（纯读会话、干净库）SHALL 保持零工作跳过；
//! - redo > 0 的既有收敛路径零变化。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("conv.db")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement failed: {sql:?} -> {resp:?}"
    );
}

/// 崩溃模拟（同既有 seed_pending_wal 纪律）：WAL 落盘（shutdown ≠ close）
/// 后 drop，不经 close()/checkpoint()。
async fn crash(db: Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

/// S6 夹具：已提交数据完好 + 显式事务未提交写入中途崩溃（WAL 含未提交
/// 记录、数据页无该行）。
async fn seed_uncommitted_crash(path: &Path) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (100, 100)").await;
    db.checkpoint().await.unwrap();
    let tx = db.begin().await.unwrap();
    if let Response::Error { message, .. } =
        to_response(db.execute_in_tx("INSERT INTO t VALUES (1, 1)", &tx).await)
    {
        panic!("tx insert failed: {message:?}");
    }
    crash(db).await;
}

/// S9 夹具：marker-only WAL——空事务（BEGIN/COMMIT 无数据写入）崩溃残留。
async fn seed_marker_only_crash(path: &Path) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    db.checkpoint().await.unwrap();
    let tx = db.begin().await.unwrap();
    db.commit(tx).await.unwrap();
    crash(db).await;
}

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
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

fn wait_cli(mut child: Child) -> CliOutput {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_string(&mut stdout)
                    .unwrap();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                return CliOutput {
                    code: status.code(),
                    stdout,
                    stderr,
                };
            }
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    wait_cli(spawn_cli(dir, args))
}

fn wal_bytes(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path.with_extension("wal")).ok()
}

/// R2-S6：未提交事务崩溃后纯读会话收敛——恢复（数据正确）→ close → check
/// exit 0 单行干净（修复前：永久 exit 1 pending WAL，与 checkpoint 命令
/// 构成死循环）。
#[tokio::test]
async fn uncommitted_crash_pure_read_session_converges() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_uncommitted_crash(&path).await;

    // 纯 SELECT 读写打开：恢复完成（已提交可见、未提交不可见），close。
    let recover = run_cli(dir.path(), &[path.to_str().unwrap(), "SELECT id FROM t"]);
    assert_eq!(recover.code, Some(0), "recover: {:?}", recover.stderr);
    assert!(
        recover.stdout.contains("100"),
        "committed row must survive recovery: {:?}",
        recover.stdout
    );
    assert!(
        !recover.stdout.contains("\"1\""),
        "uncommitted row must be discarded: {:?}",
        recover.stdout
    );

    let check = run_cli(dir.path(), &["check", path.to_str().unwrap()]);
    assert_eq!(
        check.code,
        Some(0),
        "check after pure-read recovery session must converge: {:?} / {:?}",
        check.stderr,
        check.stdout
    );
    assert_eq!(
        check.stdout.lines().count(),
        1,
        "clean check must be exactly one line: {:?}",
        check.stdout
    );
}

/// R2-S7：checkpoint 命令在恢复后状态报告实际执行（修复前：no pending
/// work; wal untouched）。
#[tokio::test]
async fn checkpoint_command_reports_actual_execution() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_uncommitted_crash(&path).await;

    // 纯读恢复会话（close 零工作跳过——修复后该形态执行完整 checkpoint，
    // 此会话已收敛；为验证 checkpoint 命令自身的消息契约，另起一个
    // marker 形态：恢复会话收敛后 WAL 只含 checkpoint 标记，checkpoint
    // 命令应报 no pending work。故本用例直接对未收敛库调 checkpoint。）
    let out = run_cli(dir.path(), &["checkpoint", path.to_str().unwrap()]);
    assert_eq!(out.code, Some(0), "checkpoint: {:?}", out.stderr);
    assert!(
        out.stdout
            .contains("wal truncated; checkpoint site advanced"),
        "checkpoint must report actual execution on recovery-pending WAL: {:?}",
        out.stdout
    );

    // 收敛后 check 干净。
    let check = run_cli(dir.path(), &["check", path.to_str().unwrap()]);
    assert_eq!(
        check.code,
        Some(0),
        "check: {:?} / {:?}",
        check.stderr,
        check.stdout
    );
}

/// R2-S8：零工作跳过保持——WAL 从未含事务记录的干净库，纯读会话 close
/// 后位点与 WAL 字节不变（MS18 语义）。
#[tokio::test]
async fn zero_work_skip_preserved_for_never_tx_wal() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        exec_ok(&db, "INSERT INTO t VALUES (1, 1)").await;
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
    }
    let wal_before = wal_bytes(&path).expect("wal exists");
    let site_before = std::fs::read(path.with_extension("checkpoint")).unwrap();

    // 纯读会话 close。
    let out = run_cli(dir.path(), &[path.to_str().unwrap(), "SELECT 1"]);
    assert_eq!(out.code, Some(0), "select: {:?}", out.stderr);

    assert_eq!(
        wal_bytes(&path).unwrap(),
        wal_before,
        "WAL bytes must stay untouched on zero-work close"
    );
    assert_eq!(
        std::fs::read(path.with_extension("checkpoint")).unwrap(),
        site_before,
        "checkpoint site must stay untouched on zero-work close"
    );
}

/// R2-S9：marker-only WAL 收敛——空事务标记残留同样楔死 check，修复后
/// 纯读读写打开即收敛 exit 0。
#[tokio::test]
async fn marker_only_wal_converges() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_marker_only_crash(&path).await;

    let check_before = run_cli(dir.path(), &["check", path.to_str().unwrap()]);
    assert_eq!(
        check_before.code,
        Some(1),
        "pre-recovery check must still reject pending WAL: {:?}",
        check_before.stdout
    );

    let recover = run_cli(dir.path(), &[path.to_str().unwrap(), "SELECT 1"]);
    assert_eq!(recover.code, Some(0), "recover: {:?}", recover.stderr);

    let check = run_cli(dir.path(), &["check", path.to_str().unwrap()]);
    assert_eq!(
        check.code,
        Some(0),
        "marker-only WAL must converge after pure-read session: {:?} / {:?}",
        check.stderr,
        check.stdout
    );
}

/// R2-S9 对照：redo > 0 的既有收敛路径零变化（已提交 INSERT 崩溃，恢复
/// 重放后 close 全量 checkpoint，check exit 0——本就成立）。
#[tokio::test]
async fn redo_path_convergence_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
        db.checkpoint().await.unwrap();
        for i in 0..50 {
            exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, {i})")).await;
        }
        crash(db).await;
    }

    let check_before = run_cli(dir.path(), &["check", path.to_str().unwrap()]);
    assert_eq!(check_before.code, Some(1), "pending WAL must reject");

    let recover = run_cli(
        dir.path(),
        &[path.to_str().unwrap(), "SELECT COUNT(*) FROM t"],
    );
    assert_eq!(recover.code, Some(0), "recover: {:?}", recover.stderr);
    assert!(
        recover.stdout.contains("50"),
        "all committed rows must replay: {:?}",
        recover.stdout
    );

    let check = run_cli(dir.path(), &["check", path.to_str().unwrap()]);
    assert_eq!(
        check.code,
        Some(0),
        "redo path must keep converging: {:?} / {:?}",
        check.stderr,
        check.stdout
    );
}
