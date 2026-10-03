//! MS27 Iteration 002：`rtsql check <db>` 只读结构完整性扫描见证矩阵
//! （delta spec `data-integrity-verification` R6/R7）。
//!
//! - 干净库：exit 0 + stdout 恰一行确认 + 主库文件全字节不变；
//! - 损坏注入：exit 1 + stdout 逐行报告（页位置 + 损坏语义），文件不变；
//! - pending WAL：readonly 打开被既有拒绝面挡下（exit 1），读写会话恢复后
//!   再 check 通过（SHALL NOT 为校验改用读写打开）；
//! - 加密库对钥 exit 0 / 错钥 exit 5；`--readonly`/`:memory:` 组合 exit 2；
//!   缺库 exit 1 零文件创建；锁冲突经预算 exit 4。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::{Page, HEADER_SIZE};
use rtsql::transaction::IsolationLevel;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("check.db")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement failed: {sql:?} -> {resp:?}"
    );
}

/// 干净落盘种子：建表 + 写行 + checkpoint（WAL 清空，注入字节可在重开后
/// 存活）→ 停 flush loop → drop。
async fn seed_clean(path: &Path, key: Option<&str>) {
    let db = Database::open_with_key(path, IsolationLevel::RepeatableRead, key)
        .await
        .unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v STRING(2000))").await;
    for i in 0..20 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, 'row-{i}')")).await;
    }
    db.checkpoint().await.unwrap();
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

/// pending WAL 种子（同 checkpoint_command_test 夹具纪律）：DDL checkpoint
/// 固化后写 WAL、停 flush loop、drop 不 close——库携带未决 WAL 存活。
async fn seed_pending_wal(path: &Path) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    db.checkpoint().await.unwrap();
    for i in 0..300 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, {i})")).await;
    }
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

/// 数据页头页的文件偏移（注入定位用）：重开只读获取 head page id。
async fn data_head_page(path: &Path) -> u32 {
    let db = Database::open(path).await.unwrap();
    let head = db
        .table_manager
        .get_table("t")
        .await
        .unwrap()
        .data_page_head
        .0 as u32;
    drop(db);
    head
}

fn poke(path: &Path, offset: u64, value: u8) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    f.write_all(&[value]).unwrap();
}

fn page_file_offset(page: u32, in_page: usize) -> u64 {
    (HEADER_SIZE + page as usize * Page::PAGE_SIZE + in_page) as u64
}

fn read_file(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
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

/// 干净库：exit 0 + stdout 恰一行确认；校验前后主库文件全字节不变；
/// `--format` 对本命令忽略（同语义同输出）。
#[tokio::test]
async fn test_check_clean_db_single_line_file_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let new = run_cli(dir.path(), &["new", path.to_str().unwrap()], &[]);
    assert_eq!(new.code, Some(0), "new: {:?}", new.stderr);
    let create = run_cli(
        dir.path(),
        &[
            path.to_str().unwrap(),
            "CREATE TABLE t (id INT PRIMARY KEY, v STRING(2000))",
        ],
        &[],
    );
    assert_eq!(create.code, Some(0), "create: {:?}", create.stderr);
    let insert = run_cli(
        dir.path(),
        &[
            path.to_str().unwrap(),
            "INSERT INTO t VALUES (1, 'a'), (2, 'b')",
        ],
        &[],
    );
    assert_eq!(insert.code, Some(0), "insert: {:?}", insert.stderr);

    let before = read_file(&path);
    let out = run_cli(dir.path(), &["check", path.to_str().unwrap()], &[]);

    assert_eq!(out.code, Some(0), "check: {:?}", out.stderr);
    assert_eq!(
        out.stdout.lines().count(),
        1,
        "clean db must print exactly one confirmation line: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("check ok"),
        "confirmation line: {:?}",
        out.stdout
    );
    assert_eq!(
        read_file(&path),
        before,
        "check must not modify the database file"
    );

    // `--format` 忽略：同语义同输出。
    let with_format = run_cli(
        dir.path(),
        &["--format", "json", "check", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(with_format.code, Some(0));
    assert_eq!(with_format.stdout, out.stdout, "--format must be ignored");
}

/// 损坏注入：数据页页类型字节翻转 → exit 1，stdout 报告含页号与损坏语义；
/// 校验本身不修改文件。
#[tokio::test]
async fn test_check_corrupted_page_exit_1_reports_page_and_semantics() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_clean(&path, None).await;
    let head = data_head_page(&path).await;
    poke(&path, page_file_offset(head, 0), 0x42);

    let before = read_file(&path);
    let out = run_cli(dir.path(), &["check", path.to_str().unwrap()], &[]);

    assert_eq!(out.code, Some(1), "stdout: {:?}", out.stdout);
    assert!(
        out.stdout.contains("error"),
        "stdout must carry the finding lines: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains(&format!("page {head}")),
        "finding must report the page location: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("unknown page type"),
        "finding must name the damage semantics: {:?}",
        out.stdout
    );
    assert_eq!(
        read_file(&path),
        before,
        "check must not modify the corrupted database file"
    );
}

/// pending WAL：readonly 打开被既有拒绝面挡下 exit 1（不触发恢复写）；
/// 读写会话完成恢复后，再 check 通过 exit 0。
#[tokio::test]
async fn test_check_pending_wal_exit_1_then_recovers() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_pending_wal(&path).await;

    let out = run_cli(dir.path(), &["check", path.to_str().unwrap()], &[]);
    assert_eq!(out.code, Some(1), "stdout: {:?}", out.stdout);
    assert!(
        out.stderr.contains("pending WAL"),
        "rejection must name the pending WAL: {:?}",
        out.stderr
    );

    // 读写会话：恢复重放完成（close 落盘），此后 check 干净通过。
    let recover = run_cli(dir.path(), &[path.to_str().unwrap(), "SELECT 1"], &[]);
    assert_eq!(recover.code, Some(0), "recover: {:?}", recover.stderr);

    let after = run_cli(dir.path(), &["check", path.to_str().unwrap()], &[]);
    assert_eq!(
        after.code,
        Some(0),
        "check after recovery: {:?} / stdout: {:?}",
        after.stderr,
        after.stdout
    );
    // change 2026-10-03-recovery-tail-persistence T3（design D3 白名单）：
    // 恢复收尾把重放推进的 tail 写回 catalog → 恢复 + close 后持久 tail ==
    // 链 terminal，check 干净单行 exit 0。原「terminal 领先持久化 tail 以
    // info 报告（benign after crash recovery）」断言锁定的缺陷形态随根因
    // 关闭而终止；info 分级本身保留（存量库指示器，integrity.rs 不动）。
    assert_eq!(
        after.stdout.lines().count(),
        1,
        "recovered db must check clean in exactly one line: {:?}",
        after.stdout
    );
    assert!(
        after.stdout.contains("check ok"),
        "confirmation line: {:?}",
        after.stdout
    );
}

/// 加密库：对钥 exit 0；错钥沿用 exit 5。
#[tokio::test]
async fn test_check_encrypted_db_key_matrix() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("enc.db");
    seed_clean(&path, Some("secret")).await;

    let ok = run_cli(
        dir.path(),
        &["--key", "secret", "check", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(
        ok.code,
        Some(0),
        "encrypted check with correct key: {:?}",
        ok.stderr
    );

    let bad = run_cli(
        dir.path(),
        &["--key", "wrong", "check", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(
        bad.code,
        Some(5),
        "wrong key must keep the invalid-key face: {:?}",
        bad.stderr
    );
}

/// 组合拒绝：`--readonly check` 与 `check :memory:` 均 exit 2
/// （既有全局守卫自动生效，零新代码）。
#[tokio::test]
async fn test_check_readonly_and_memory_combinations_exit_2() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let new = run_cli(dir.path(), &["new", path.to_str().unwrap()], &[]);
    assert_eq!(new.code, Some(0), "new: {:?}", new.stderr);

    let readonly = run_cli(
        dir.path(),
        &["--readonly", "check", path.to_str().unwrap()],
        &[],
    );
    assert_eq!(
        readonly.code,
        Some(2),
        "--readonly check must be a Usage rejection: {:?}",
        readonly.stderr
    );

    let memory = run_cli(dir.path(), &["check", ":memory:"], &[]);
    assert_eq!(
        memory.code,
        Some(2),
        "check :memory: must be a Usage rejection: {:?}",
        memory.stderr
    );
}

/// 缺库拒绝：exit 1、错误点名库名、零文件创建。
#[tokio::test]
async fn test_check_missing_db_exit_1() {
    let dir = TempDir::new().unwrap();

    let out = run_cli(dir.path(), &["check", "nope"], &[]);

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
/// （仅 ~8ms 下限窗口）→ exit 4，错误面维持 `database is locked`。
#[tokio::test]
async fn test_check_lock_conflict_exit_4() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let holder = Database::open(&path).await.unwrap();

    let out = run_cli(
        dir.path(),
        &["check", path.to_str().unwrap()],
        &[("RTSQL_BUSY_TIMEOUT", "0")],
    );
    drop(holder);

    assert_eq!(out.code, Some(4), "stdout: {:?}", out.stdout);
    assert!(
        out.stderr.contains("database is locked"),
        "stderr must name the lock failure: {:?}",
        out.stderr
    );
}

/// 未引用溢出链以 info 级报告且不改退出码（error/info 分级交接口径）。
#[tokio::test]
async fn test_check_unreferenced_overflow_is_info_exit_0() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    // 10KB 行：跨 3 个溢出页的链（chunk 4080 ×2 + 尾段）。
    let db = Database::open(&path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v STRING(65535))").await;
    exec_ok(
        &db,
        &format!("INSERT INTO t VALUES (1, '{}')", "x".repeat(10_000)),
    )
    .await;
    db.checkpoint().await.unwrap();
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    // 在页锁外抹掉 locator 标记字节：行不再引用链，链页成为未引用空间。
    let head = db
        .table_manager
        .get_table("t")
        .await
        .unwrap()
        .data_page_head
        .0 as u32;
    drop(db);
    poke(&path, page_file_offset(head, 16 + 22), 0x01);

    let out = run_cli(dir.path(), &["check", path.to_str().unwrap()], &[]);

    assert_eq!(
        out.code,
        Some(0),
        "info-level findings must not change the exit code: {:?}",
        out.stderr
    );
    assert!(
        out.stdout.contains("info"),
        "stdout must carry the info finding: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("not referenced"),
        "info finding must name the semantics: {:?}",
        out.stdout
    );
}
