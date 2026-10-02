//! MS28 Iteration 000：锁等待预算可配置（busy-timeout 三通道）见证矩阵。
//!
//! lib 臂（`database-file-lock` R1 MODIFIED）：
//! - 无争用 `open_with_lock`/`open_readonly_with_lock` 打开成功；
//! - 预算内持有者释放 → 第二方等待后打开成功（默认 5s 语义核心场景）；
//! - 预算耗尽（持续持有）→ `DatabaseLocked` 拒绝且耗时 ≥ 预算；
//! - `Some(0)` ＝ 仅 ~8ms 可见性下限窗口即拒绝（现状语义的显式表达，快速失败）；
//! - 只读共享锁同款四态。
//!
//! CLI 臂与信号中断用例见本文件后半（T2/T3 阶段追加）。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::storage::{FileStorage, StorageError};
use rtsql::transaction::IsolationLevel;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("busy.db")
}

/// 预建有效库文件后释放（锁矩阵用例的前置）。
fn prepare_db_file(path: &std::path::Path) {
    let storage = FileStorage::open(path).unwrap();
    drop(storage);
}

/// 以原生 fd 稳定持有独占锁，交给调用方控制释放时机。fd 刚关闭后的平台
/// flock 释放可见性窗口（`database-file-lock` R1 既有正确性机制吸收的
/// 同型平台行为）内立即获取可能短暂 `WouldBlock`——helper 以 2ms 间隔
/// 重试吸收（与产品下限窗口同纪律，不构成断言弱化）。
fn hold_exclusive(path: &std::path::Path) -> std::fs::File {
    let holder = std::fs::File::open(path).unwrap();
    let mut attempts = 0;
    loop {
        match holder.try_lock() {
            Ok(()) => return holder,
            Err(std::fs::TryLockError::WouldBlock) if attempts < 50 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => panic!("test process acquires flock: {e:?}"),
        }
    }
}

fn expect_locked<T>(result: Result<T, StorageError>) -> StorageError {
    match result {
        Ok(_) => panic!("expected StorageError::DatabaseLocked, got Ok"),
        Err(e) => e,
    }
}

fn assert_locked(err: &StorageError) {
    assert!(
        matches!(err, StorageError::DatabaseLocked(_)),
        "expected StorageError::DatabaseLocked, got: {err:?}"
    );
}

// ---------- T3 见证：CLI busy-timeout 通道矩阵 ----------

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 启动 rtsql 二进制（RTSQL_HOME 指向临时目录，隔离宿主环境变量；
/// 显式移除 RTSQL_KEY/RTSQL_BUSY_TIMEOUT 防宿主环境污染）。
fn spawn_cli(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.spawn().expect("spawn rtsql binary")
}

/// 等待子进程退出并收集输出；60s 未退出则 kill 并 panic
/// （one-shot CLI 必须自行退出，挂起即为行为错误）。
fn wait_cli(mut child: Child) -> CliOutput {
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    match child.wait_timeout(Duration::from_secs(60)).unwrap() {
        Some(status) => {
            let mut out = String::new();
            let mut err = String::new();
            stdout.read_to_string(&mut out).unwrap();
            stderr.read_to_string(&mut err).unwrap();
            CliOutput {
                code: status.code(),
                stdout: out,
                stderr: err,
            }
        }
        None => {
            let _ = child.kill();
            panic!("rtsql did not exit within 60s (hung)");
        }
    }
}

trait WaitTimeout {
    fn wait_timeout(&mut self, dur: Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl WaitTimeout for Child {
    fn wait_timeout(&mut self, dur: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        // 轮询等待：测试夹具层最小实现（不引新依赖）。
        let start = Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            if start.elapsed() >= dur {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn run_cli(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CliOutput {
    wait_cli(spawn_cli(dir, args, envs))
}

/// 持有者延时 300ms 释放的释放线程（CLI 等待成功用例的确定性锚点）。
fn release_after(holder: Database, millis: u64) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(millis));
        drop(holder);
    })
}

/// flag 通道：持有者延时释放，第二方经 `--busy-timeout` 预算内等待后成功。
#[tokio::test]
async fn test_cli_busy_timeout_flag_waits_and_succeeds() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let first = Database::open(&path).await.unwrap();
    let releaser = release_after(first, 300);

    let start = Instant::now();
    let out = run_cli(
        dir.path(),
        &["--busy-timeout", "5000", path.to_str().unwrap(), "SELECT 1"],
        &[],
    );
    let elapsed = start.elapsed();
    releaser.join().unwrap();

    assert_eq!(out.code, Some(0), "stderr: {:?}", out.stderr);
    assert!(
        elapsed >= Duration::from_millis(250),
        "CLI should have waited for the holder (elapsed: {elapsed:?})"
    );
}

/// env 通道：`RTSQL_BUSY_TIMEOUT` 生效。
#[tokio::test]
async fn test_cli_busy_timeout_env_waits_and_succeeds() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let first = Database::open(&path).await.unwrap();
    let releaser = release_after(first, 300);

    let start = Instant::now();
    let out = run_cli(
        dir.path(),
        &[path.to_str().unwrap(), "SELECT 1"],
        &[("RTSQL_BUSY_TIMEOUT", "5000")],
    );
    let elapsed = start.elapsed();
    releaser.join().unwrap();

    assert_eq!(out.code, Some(0), "stderr: {:?}", out.stderr);
    assert!(
        elapsed >= Duration::from_millis(250),
        "CLI should have waited for the holder (elapsed: {elapsed:?})"
    );
}

/// flag 优先：env=0（立即拒绝）时显式 flag=5000 胜出，等待后成功。
#[tokio::test]
async fn test_cli_busy_timeout_flag_overrides_env() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let first = Database::open(&path).await.unwrap();
    let releaser = release_after(first, 300);

    let out = run_cli(
        dir.path(),
        &["--busy-timeout", "5000", path.to_str().unwrap(), "SELECT 1"],
        &[("RTSQL_BUSY_TIMEOUT", "0")],
    );
    releaser.join().unwrap();

    assert_eq!(
        out.code,
        Some(0),
        "explicit flag must override env: {:?}",
        out.stderr
    );
}

/// 非法值 exit 2：非数字 flag、负数 flag、空 env——均不触库。
#[tokio::test]
async fn test_cli_busy_timeout_invalid_values_exit_2() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let bad_flag = run_cli(
        dir.path(),
        &["--busy-timeout", "abc", path.to_str().unwrap(), "SELECT 1"],
        &[],
    );
    assert_eq!(bad_flag.code, Some(2), "stderr: {:?}", bad_flag.stderr);

    let negative = run_cli(
        dir.path(),
        &["--busy-timeout", "-1", path.to_str().unwrap(), "SELECT 1"],
        &[],
    );
    assert_eq!(negative.code, Some(2), "stderr: {:?}", negative.stderr);

    let empty_env = run_cli(
        dir.path(),
        &[path.to_str().unwrap(), "SELECT 1"],
        &[("RTSQL_BUSY_TIMEOUT", "")],
    );
    assert_eq!(empty_env.code, Some(2), "stderr: {:?}", empty_env.stderr);
}

/// `:memory:` 组合接受并忽略（无文件锁，等待语义无消费点），exit 0。
#[tokio::test]
async fn test_cli_memory_db_ignores_busy_timeout() {
    let dir = TempDir::new().unwrap();

    let with_flag = run_cli(
        dir.path(),
        &["--busy-timeout", "5000", ":memory:", "SELECT 1"],
        &[],
    );
    assert_eq!(with_flag.code, Some(0), "stderr: {:?}", with_flag.stderr);

    let with_env = run_cli(
        dir.path(),
        &[":memory:", "SELECT 1"],
        &[("RTSQL_BUSY_TIMEOUT", "5000")],
    );
    assert_eq!(with_env.code, Some(0), "stderr: {:?}", with_env.stderr);
}

/// 预算耗尽：持有者持续持有，`--busy-timeout 200` 超时后 exit 4，
/// 文案含 "database is locked" 与库路径（既有错误面不变）。
#[tokio::test]
async fn test_cli_busy_timeout_timeout_exit_4() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let first = Database::open(&path).await.unwrap();

    let start = Instant::now();
    let out = run_cli(
        dir.path(),
        &["--busy-timeout", "200", path.to_str().unwrap(), "SELECT 1"],
        &[],
    );
    let elapsed = start.elapsed();
    drop(first);

    assert_eq!(out.code, Some(4), "stdout: {:?}", out.stdout);
    assert!(
        out.stderr.contains("database is locked"),
        "stderr should name the lock failure: {:?}",
        out.stderr
    );
    assert!(
        out.stderr.contains(path.display().to_string().as_str()),
        "stderr should contain the db path: {:?}",
        out.stderr
    );
    assert!(
        elapsed >= Duration::from_millis(200),
        "timeout should wait out the budget (elapsed: {elapsed:?})"
    );
}

/// 等待中 SIGINT：预算内仍等待时收到 SIGINT → 130（无 close）。
#[tokio::test]
async fn test_cli_sigint_during_lock_wait_exit_130() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let first = Database::open(&path).await.unwrap();

    let child = spawn_cli(
        dir.path(),
        &[path.to_str().unwrap(), "SELECT 1"],
        &[("RTSQL_BUSY_TIMEOUT", "10000")],
    );
    // 等待确定性锚点：子进程已进入锁等待（短时 sleep 即达，锁已被测试持有）。
    std::thread::sleep(Duration::from_millis(500));
    let rc = unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    assert_eq!(rc, 0, "kill(SIGINT) failed");
    let out = wait_cli(child);
    drop(first);

    assert_eq!(
        out.code,
        Some(130),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}

/// ATTACH 附加库打开点穿线同值：附加库被持锁时经预算等待后成功。
#[tokio::test]
async fn test_cli_attach_waits_within_budget() {
    let dir = TempDir::new().unwrap();
    let main_path = dir.path().join("main.db");
    let att_path = dir.path().join("att.db");
    {
        let main_db = Database::open(&main_path).await.unwrap();
        let att_db = Database::open(&att_path).await.unwrap();
        att_db
            .execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
            .await;
        att_db.execute_sql("INSERT INTO t VALUES (1)").await;
        // lib 契约：DDL 无 WAL，drop 不 close——显式落盘 schema 与数据后再释放。
        att_db.close().await.unwrap();
        drop(att_db);
        drop(main_db);
    }
    // 持锁者占用附加库文件锁，延时释放。
    let holder = Database::open(&att_path).await.unwrap();
    let releaser = release_after(holder, 300);

    let sql = format!("ATTACH '{}' AS a; SELECT * FROM a.t", att_path.display());
    let start = Instant::now();
    let out = run_cli(
        dir.path(),
        &["--busy-timeout", "5000", main_path.to_str().unwrap(), &sql],
        &[],
    );
    let elapsed = start.elapsed();
    releaser.join().unwrap();

    assert_eq!(out.code, Some(0), "stderr: {:?}", out.stderr);
    assert!(
        elapsed >= Duration::from_millis(250),
        "ATTACH should have waited for the attached db holder (elapsed: {elapsed:?})"
    );
    assert!(
        out.stdout.contains('1'),
        "query result should contain the attached row: {:?}",
        out.stdout
    );
}

// ---------- T2 见证：Database 打开面锁等待预算 ----------

/// 显式 `Some(0)` 预算：同进程双开立即拒绝（仅 ~8ms 下限窗口）。
/// RED 主体：`open_with_lock_wait` API 缺失（编译期见证）。
#[tokio::test]
async fn test_database_open_with_lock_wait_zero_budget_rejects() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let first = Database::open(&path).await.unwrap();

    let start = Instant::now();
    let err = expect_locked(
        Database::open_with_lock_wait(
            &path,
            IsolationLevel::RepeatableRead,
            None,
            Some(Duration::from_millis(0)),
        )
        .await,
    );
    let elapsed = start.elapsed();
    assert_locked(&err);
    assert!(
        elapsed < Duration::from_millis(100),
        "zero budget should reject within the floor window only (elapsed: {elapsed:?})"
    );
    drop(first);
}

/// 默认预算（`None` → 5000ms）：持有者延时 300ms 释放后第二方打开成功。
#[tokio::test]
async fn test_database_open_with_lock_wait_default_budget_waits_release() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let first = Database::open(&path).await.unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(first);
    });

    let start = Instant::now();
    let second = Database::open_with_lock_wait(&path, IsolationLevel::RepeatableRead, None, None)
        .await
        .expect("second open within default budget after holder release");
    let elapsed = start.elapsed();
    releaser.join().unwrap();

    assert!(
        elapsed >= Duration::from_millis(250),
        "second open should have waited for the holder (elapsed: {elapsed:?})"
    );
    drop(second);
}

// ---------- T1 见证：FileStorage 锁等待预算 ----------

#[tokio::test]
async fn test_open_with_lock_no_contention() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let storage = FileStorage::open_with_lock(&path, None, Some(Duration::from_secs(5)))
        .expect("open_with_lock on fresh path succeeds");
    assert_eq!(storage.page_count(), 0);
}

#[tokio::test]
async fn test_open_readonly_with_lock_no_contention() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let storage = FileStorage::open_readonly_with_lock(&path, None, Some(Duration::from_secs(5)))
        .expect("open_readonly_with_lock succeeds");
    assert_eq!(storage.page_count(), 0);
}

/// 预算内获得锁：持有者延时 300ms 释放，第二方 5s 预算内等待后成功。
/// RED 主体：`open_with_lock` API 缺失（编译期见证）。
#[tokio::test]
async fn test_open_with_lock_waits_within_budget() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let holder = hold_exclusive(&path);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(holder);
    });

    let start = Instant::now();
    let storage = FileStorage::open_with_lock(&path, None, Some(Duration::from_secs(5))).unwrap();
    let elapsed = start.elapsed();
    releaser.join().unwrap();

    assert_eq!(storage.page_count(), 0);
    assert!(
        elapsed >= Duration::from_millis(250),
        "second open should have waited for the holder (elapsed: {elapsed:?})"
    );
}

/// 预算耗尽：持有者持续持有，100ms 预算耗尽后 `DatabaseLocked`，
/// 且耗时 ≥ 预算（证明真实等待而非立即拒绝）。
#[tokio::test]
async fn test_open_with_lock_budget_exhaustion_rejects() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let _holder = hold_exclusive(&path);

    let start = Instant::now();
    let err = expect_locked(FileStorage::open_with_lock(
        &path,
        None,
        Some(Duration::from_millis(100)),
    ));
    let elapsed = start.elapsed();
    assert_locked(&err);
    assert!(
        elapsed >= Duration::from_millis(100),
        "exhaustion should wait out the full budget (elapsed: {elapsed:?})"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "exhaustion should return promptly after budget (elapsed: {elapsed:?})"
    );
}

/// `Some(0)` ＝ 不额外等待，仅 ~8ms 可见性下限窗口即拒绝（快速失败）。
#[tokio::test]
async fn test_open_with_lock_zero_budget_floor_only() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let _holder = hold_exclusive(&path);

    let start = Instant::now();
    let err = expect_locked(FileStorage::open_with_lock(
        &path,
        None,
        Some(Duration::from_millis(0)),
    ));
    let elapsed = start.elapsed();
    assert_locked(&err);
    assert!(
        elapsed < Duration::from_millis(100),
        "zero budget should reject within the floor window only (elapsed: {elapsed:?})"
    );
}

/// 只读共享锁同款——预算内持有者释放后等待成功。
#[tokio::test]
async fn test_open_readonly_with_lock_waits_within_budget() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let holder = hold_exclusive(&path);
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(holder);
    });

    let start = Instant::now();
    let storage =
        FileStorage::open_readonly_with_lock(&path, None, Some(Duration::from_secs(5))).unwrap();
    let elapsed = start.elapsed();
    releaser.join().unwrap();

    assert_eq!(storage.page_count(), 0);
    assert!(
        elapsed >= Duration::from_millis(250),
        "readonly second open should have waited for the holder (elapsed: {elapsed:?})"
    );
}

/// 只读共享锁同款——预算耗尽拒绝。
#[tokio::test]
async fn test_open_readonly_with_lock_budget_exhaustion_rejects() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let _holder = hold_exclusive(&path);

    let start = Instant::now();
    let err = expect_locked(FileStorage::open_readonly_with_lock(
        &path,
        None,
        Some(Duration::from_millis(100)),
    ));
    let elapsed = start.elapsed();
    assert_locked(&err);
    assert!(
        elapsed >= Duration::from_millis(100),
        "readonly exhaustion should wait out the full budget (elapsed: {elapsed:?})"
    );
}

/// 只读共享锁同款——`Some(0)` 仅下限窗口即拒绝。
#[tokio::test]
async fn test_open_readonly_with_lock_zero_budget_floor_only() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    prepare_db_file(&path);

    let _holder = hold_exclusive(&path);

    let start = Instant::now();
    let err = expect_locked(FileStorage::open_readonly_with_lock(
        &path,
        None,
        Some(Duration::from_millis(0)),
    ));
    let elapsed = start.elapsed();
    assert_locked(&err);
    assert!(
        elapsed < Duration::from_millis(100),
        "zero budget should reject within the floor window only (elapsed: {elapsed:?})"
    );
}
