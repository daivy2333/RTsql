//! MS28 Iteration 001：`rtsql checkpoint <db>` 显式落盘子命令见证矩阵
//! （`cli-noninteractive-shell` ADDED「生命周期子命令 checkpoint」）。
//!
//! - 有未决 WAL：exit 0 确认行 + dump 可见全部已提交数据 + WAL 截断/缩减；
//! - 干净库：exit 0 + 零工作早退明示（`no pending work`）；
//! - 拒绝面：缺库 exit 1（点名库名、零文件创建）、锁冲突经预算 exit 4、
//!   `--readonly`/`:memory:` 组合 exit 2、加密库错钥 exit 5。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::transaction::IsolationLevel;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("cp.db")
}

/// 夹具：建表落盘（DDL 无 WAL，先 checkpoint 固化 schema）→ 一批写入只进
/// WAL → 停 flush loop 后 drop 不 close——库携带未决 WAL 存活（恢复重放
/// 保证数据不丢，与 cli/mod.rs 信号结构测试同一纪律）。
async fn seed_pending_wal(path: &Path, key: Option<&str>) {
    let db = Database::open_with_key(path, IsolationLevel::RepeatableRead, key)
        .await
        .unwrap();
    match to_response(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
            .await,
    ) {
        Response::AffectedRows { .. } => {}
        other => panic!("create table failed: {other:?}"),
    }
    db.checkpoint().await.unwrap();
    for i in 0..300 {
        match to_response(
            db.execute_sql(&format!("INSERT INTO t VALUES ({}, {})", i, i * 10))
                .await,
        ) {
            Response::AffectedRows { .. } => {}
            other => panic!("insert {i} failed: {other:?}"),
        }
    }
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

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

/// RED 主体：子命令存在且端到端完成——修复前 clap unknown subcommand exit 2。
#[tokio::test]
async fn test_checkpoint_command_completes_with_pending_wal() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_pending_wal(&path, None).await;
    let wal_before = std::fs::metadata(path.with_extension("wal")).unwrap().len();
    assert!(
        wal_before >= 1024,
        "precondition: WAL should carry pending records, got {wal_before}"
    );

    let out = run_cli(dir.path(), &["checkpoint", path.to_str().unwrap()], &[]);

    assert_eq!(
        out.code,
        Some(0),
        "checkpoint must succeed: {:?}",
        out.stderr
    );
    assert!(
        out.stdout.contains("checkpoint complete"),
        "stdout must carry the one-line confirmation: {:?}",
        out.stdout
    );
    // 落盘可见性：dump 经重开恢复后仍见全部已提交数据。
    let dump = run_cli(dir.path(), &["dump", path.to_str().unwrap()], &[]);
    assert_eq!(
        dump.code,
        Some(0),
        "dump after checkpoint: {:?}",
        dump.stderr
    );
    for needle in ["(0, 0)", "(42, 420)", "(299, 2990)"] {
        assert!(
            dump.stdout.contains(needle),
            "dump must contain committed row {needle}: {:?}",
            dump.stdout
        );
    }
    // WAL 截断/缩减：checkpoint 后未决记录落盘，WAL 应显著收缩。
    let wal_after = std::fs::metadata(path.with_extension("wal")).unwrap().len();
    assert!(
        wal_after < wal_before,
        "WAL must shrink after checkpoint: before={wal_before} after={wal_after}"
    );
}

/// 零工作早退明示：干净库（new 后无写入）checkpoint exit 0 且 stdout 点名
/// 无未决工作（`Database::checkpoint()` 零工作跳过语义不变）。
#[tokio::test]
async fn test_checkpoint_zero_work_explicit() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let new = run_cli(dir.path(), &["new", path.to_str().unwrap()], &[]);
    assert_eq!(new.code, Some(0), "new: {:?}", new.stderr);

    let out = run_cli(dir.path(), &["checkpoint", path.to_str().unwrap()], &[]);

    assert_eq!(out.code, Some(0), "checkpoint: {:?}", out.stderr);
    assert!(
        out.stdout.contains("no pending work"),
        "zero-work confirmation must be explicit: {:?}",
        out.stdout
    );
}

/// 缺库拒绝：exit 1、错误点名库名、零文件创建（创建通道收敛 `rtsql new`）。
#[tokio::test]
async fn test_checkpoint_missing_db_exit_1() {
    let dir = TempDir::new().unwrap();

    let out = run_cli(dir.path(), &["checkpoint", "nope"], &[]);

    assert_eq!(out.code, Some(1), "stdout: {:?}", out.stdout);
    assert!(
        out.stderr.contains("nope"),
        "error must name the missing db: {:?}",
        out.stderr
    );
    assert!(
        !dir.path().join("nope.db").exists(),
        "rejection must not create any file"
    );
}

/// 锁冲突经等待预算后拒绝：持有者持续持锁 + `RTSQL_BUSY_TIMEOUT=0`
/// （仅 ~8ms 下限窗口）→ exit 4，错误面维持 `database is locked` 含库名。
#[tokio::test]
async fn test_checkpoint_lock_conflict_exit_4() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let holder = Database::open(&path).await.unwrap();

    let out = run_cli(
        dir.path(),
        &["checkpoint", path.to_str().unwrap()],
        &[("RTSQL_BUSY_TIMEOUT", "0")],
    );
    drop(holder);

    assert_eq!(out.code, Some(4), "stdout: {:?}", out.stdout);
    assert!(
        out.stderr.contains("database is locked"),
        "stderr must name the lock failure: {:?}",
        out.stderr
    );
    assert!(
        out.stderr.contains(path.display().to_string().as_str()),
        "stderr must contain the db path: {:?}",
        out.stderr
    );
}

/// 组合拒绝：`--readonly checkpoint` 与 `checkpoint :memory:` 均 exit 2
/// （既有全局守卫自动生效，零新代码）。
#[tokio::test]
async fn test_checkpoint_readonly_and_memory_combinations_exit_2() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let new = run_cli(dir.path(), &["new", path.to_str().unwrap()], &[]);
    assert_eq!(new.code, Some(0), "new: {:?}", new.stderr);

    let readonly = run_cli(
        dir.path(),
        &["--readonly", "checkpoint", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(
        readonly.code,
        Some(2),
        "--readonly checkpoint must be a Usage rejection: {:?}",
        readonly.stderr
    );
    assert!(
        readonly.stderr.contains("--readonly"),
        "rejection must name the flag: {:?}",
        readonly.stderr
    );

    let memory = run_cli(dir.path(), &["checkpoint", ":memory:"], &[]);
    assert_eq!(
        memory.code,
        Some(2),
        "checkpoint :memory: must be a Usage rejection: {:?}",
        memory.stderr
    );
    assert!(
        memory.stderr.contains(":memory:"),
        "rejection must name the memory target: {:?}",
        memory.stderr
    );
}

/// 加密库：正确密钥 exit 0 完成落盘；错误密钥沿用 exit 5 解密失败面。
#[tokio::test]
async fn test_checkpoint_encrypted_db() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("enc.db");
    seed_pending_wal(&path, Some("secret")).await;

    let ok = run_cli(
        dir.path(),
        &["--key", "secret", "checkpoint", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(
        ok.code,
        Some(0),
        "encrypted checkpoint with correct key: {:?}",
        ok.stderr
    );
    assert!(
        ok.stdout.contains("checkpoint complete"),
        "stdout must carry the confirmation: {:?}",
        ok.stdout
    );

    let bad = run_cli(
        dir.path(),
        &["--key", "wrong", "checkpoint", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(
        bad.code,
        Some(5),
        "wrong key must keep the invalid-key face: {:?}",
        bad.stderr
    );
}
