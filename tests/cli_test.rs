//! MS10-T01 CLI 集成测试 —— 通过真二进制（`CARGO_BIN_EXE_rtsql`）验证
//! one-shot 执行、名称解析、渲染格式、退出码分类与 close 落盘语义。
//!
//! 每个 spawn 都以独立 TempDir 为 CWD，并把 `RTSQL_HOME` 指向该 TempDir
//! （并行安全，同时覆盖 R2 的 RTSQL_HOME 解析场景）；fixture 预建 `db/`
//! 子目录（CLI 不建目录，父目录缺失按契约报错退出 1）。stdout/stderr 始终
//! 为管道（非 TTY），因此默认格式为 JSON。

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::page_format::ColumnType;
use tempfile::TempDir;

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （one-shot CLI 必须自行退出，挂起即为行为错误）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    wait_cli(spawn_cli(dir, args))
}

/// 启动 rtsql 二进制但不等待退出（信号用例需要向存活进程发信号）。
fn spawn_cli(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary")
}

/// 等待子进程退出并收集输出；60s 未退出则 kill 并 panic。
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

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录
fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

/// 建表 + 插入一行种子数据（裸名 `app` → `$RTSQL_HOME/db/app.db`）；
/// 任何一步失败即 panic。
fn seed_users(dir: &Path) {
    let out = run_cli(
        dir,
        &[
            "app",
            "CREATE TABLE users (id INT PRIMARY KEY, name STRING)",
        ],
    );
    assert_eq!(out.code, Some(0), "create users failed: {}", out.stderr);

    let out = run_cli(dir, &["app", "INSERT INTO users VALUES (1, 'Alice')"]);
    assert_eq!(out.code, Some(0), "insert alice failed: {}", out.stderr);
}

/// ① 成功 SELECT：退出 0，table 格式输出含列名表头与数据
#[test]
fn test_select_success_table_header() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &["app", "SELECT id, name FROM users", "--format", "table"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("id") && out.stdout.contains("name"),
        "missing column headers in table output: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Alice"),
        "missing row data in table output: {:?}",
        out.stdout
    );
}

/// ①（RTM R3/A6）别名与聚合表头：别名优先，无别名聚合用引擎 result_column_name 文本
#[test]
fn test_alias_and_aggregate_headers() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &["app", "SELECT COUNT(*) AS cnt, AVG(id) FROM users"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["columns"], serde_json::json!(["cnt", "avg_id"]));
    assert_eq!(parsed["rows"], serde_json::json!([[1, 1.0]]));
}

/// ①（RTM R3）JOIN 查询表头：两个投影列名都出现在 table 输出中
#[test]
fn test_join_select_header() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE orders (user_id INT, total INT)"],
    );
    assert_eq!(out.code, Some(0), "create orders failed: {}", out.stderr);

    let out = run_cli(dir.path(), &["app", "INSERT INTO orders VALUES (1, 42)"]);
    assert_eq!(out.code, Some(0), "insert order failed: {}", out.stderr);

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT users.id, orders.total FROM users JOIN orders ON users.id = orders.user_id",
            "--format",
            "table",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("id") && out.stdout.contains("total"),
        "missing JOIN column headers: {:?}",
        out.stdout
    );
}

/// ② 用法错误：无参数退出 2
#[test]
fn test_usage_error_exit_2() {
    let dir = fixture();
    let out = run_cli(dir.path(), &[]);
    assert_eq!(
        out.code,
        Some(2),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}

/// ③ SQL 错误（parse / plan）：退出 3 且 stderr 有错误信息
#[test]
fn test_sql_error_exit_3() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(dir.path(), &["app", "SELEC id FROM users"]);
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stderr.is_empty(), "parse error must go to stderr");

    let out = run_cli(dir.path(), &["app", "SELECT id FROM missing_table"]);
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stderr.is_empty(), "plan error must go to stderr");
}

/// ④ 多语句护栏：`;` 分隔的双 INSERT 退出 3 且零执行（再查行集只剩种子行）
#[test]
fn test_multi_statement_rejected() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "INSERT INTO users VALUES (2, 'Bob'); INSERT INTO users VALUES (3, 'Carol')",
        ],
    );
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("one statement at a time"),
        "guard message must explain the policy: {:?}",
        out.stderr
    );

    // 零执行：两条 INSERT 都未生效，行集只剩种子数据 Alice。
    // MS10-T01 Iter001（T9 校准）：改回子集投影锁定真投影语义——
    // 返回投影列的行，不再退化为全 schema 行。
    let out = run_cli(dir.path(), &["app", "SELECT name FROM users"]);
    assert_eq!(out.code, Some(0));
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([["Alice"]]),
        "unexpected rows"
    );
}

/// ⑤ 非 TTY（管道）默认 JSON：columns + rows 自描述形状
#[test]
fn test_piped_default_json() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(dir.path(), &["app", "SELECT id, name FROM users"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["columns"], serde_json::json!(["id", "name"]));
    assert_eq!(parsed["rows"], serde_json::json!([[1, "Alice"]]));
}

/// ⑥ `--format csv`：RFC 4180 转义（引号翻倍 + 引号包裹）。
/// MS10-T01 Iter001（T9 校准）：子集投影 + PK 点查（IndexScan 路径）——
/// 正是真投影修复的表头错位场景，行与表头一致地只含投影列。
#[test]
fn test_csv_format_escaping() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &["app", "INSERT INTO users VALUES (2, 'a\"b,c')"],
    );
    assert_eq!(out.code, Some(0), "insert special failed: {}", out.stderr);

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT name FROM users WHERE id = 2",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "name\n\"a\"\"b,c\"\n");
}

/// ⑦ DML 输出 affected_rows，退出 0
#[test]
fn test_insert_affected_rows() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(dir.path(), &["app", "INSERT INTO users VALUES (9, 'Zed')"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["affected_rows"], serde_json::json!(1));
}

/// ⑧ close 语义：进程退出后数据可见（checkpoint 落盘），WAL 被截断为最小长度
#[test]
fn test_close_persists_and_truncates_wal() {
    let dir = fixture();
    seed_users(dir.path());

    // 新进程重开：上一进程 close() 后数据可见
    let out = run_cli(dir.path(), &["app", "SELECT id, name FROM users"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([[1, "Alice"]]));

    // WAL 截断的等价可观察断言：checkpoint 后 WAL 不再累积
    let wal = dir.path().join("db/app.wal");
    let wal_len = std::fs::metadata(&wal).expect("wal file must exist").len();
    assert!(
        wal_len < 1024,
        "WAL should be truncated by close(), got {} bytes",
        wal_len
    );
}

/// T5: 不存在的裸名库沿用 open 语义静默创建，建表后往返可查
#[test]
fn test_new_database_created_silently() {
    let dir = fixture();
    assert!(!dir.path().join("db/fresh.db").exists());

    let out = run_cli(dir.path(), &["fresh", "CREATE TABLE t (id INT)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        dir.path().join("db/fresh.db").exists(),
        "db file must be created by open"
    );

    let out = run_cli(dir.path(), &["fresh", "SELECT id FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([]));
}

/// T5: 页不对齐的垃圾文件打开失败 → stderr 报错，退出 1
#[test]
fn test_corrupt_file_open_fails_exit_1() {
    let dir = fixture();
    let junk = dir.path().join("junk.db");
    std::fs::write(&junk, vec![0u8; 100]).unwrap();

    let out = run_cli(dir.path(), &[junk.to_str().unwrap(), "SELECT 1"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stderr.is_empty(), "open failure must go to stderr");
}

/// T02（file-lock R1-S2 / cli R1 锁冲突场景）：测试进程持有目标库的
/// advisory 独占锁 → rtsql 打开被拒 → 退出 4 + stderr `database is locked`
/// 前缀 + stdout 为空（SQL 未执行）；释放锁后同一命令退出 0。
/// （SQL 取 `SELECT id FROM users`：契约草拟的 `SELECT 1` 无 FROM 子句
/// 在 plan 阶段即报错，无法作为"释放后可正常执行"的见证。）
#[test]
fn test_lock_conflict_exit_4() {
    let dir = fixture();
    seed_users(dir.path());
    let db_file = dir.path().join("db/app.db");

    let holder = std::fs::File::open(&db_file).unwrap();
    holder.try_lock().expect("test process acquires flock");

    let out = run_cli(dir.path(), &["app", "SELECT id FROM users"]);
    assert_eq!(
        out.code,
        Some(4),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.starts_with("database is locked"),
        "stderr must start with 'database is locked', got: {:?}",
        out.stderr
    );
    assert!(out.stdout.is_empty(), "no SQL output on lock conflict");

    drop(holder);
    let out = run_cli(dir.path(), &["app", "SELECT id FROM users"]);
    assert_eq!(out.code, Some(0), "after release: {}", out.stderr);
}

// ---- T3：优雅停机（信号接线 close，D4/D5） ----

/// D5 标定结果（2026-09-06 本机 WSL2 实测，见 Act Response）：N 行单事务
/// WAL 的恢复（open）耗时 T；要求 T ≥ 500ms，信号延迟 D ∈ [T/4, T/2]。
const WAL_ROWS: i64 = 20_000;
const SIGNAL_DELAY_MS: u64 = 200;

/// D5 构造：单会话分块事务批量插入 N 行（50 行/事务，缓冲记录数恒低于
/// capacity=100 阈值，避开 WALBuffer appender-threshold 与 flush loop 的
/// 并发 do_flush 偏移竞争——见 Act Response Remaining Issues）→
/// drop 不 close（无 checkpoint，`.wal` 保留全部 redo，重开需完整恢复）。
async fn build_big_wal(dir: &Path, n: i64) {
    const CHUNK: i64 = 50;
    let path = dir.join("db/app.db");
    let db = Database::open(&path).await.unwrap();
    db.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let mut done = 0;
    while done < n {
        let tx = db.begin().await.unwrap();
        for i in done..(done + CHUNK).min(n) {
            let resp = db
                .execute_in_tx(&format!("INSERT INTO t VALUES ({})", i), &tx)
                .await;
            if let Response::Error { message } = resp {
                panic!("insert {} failed: {}", i, message);
            }
        }
        db.commit(tx).await.unwrap();
        done += CHUNK;
    }
    drop(db);
}

/// 信号落地后重开，行数必须完整。
fn assert_row_count_intact(dir: &Path, n: i64) {
    let out = run_cli(dir, &["app", "SELECT COUNT(*) FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[n]]),
        "committed rows must survive the signal shutdown"
    );
}

/// ① 执行阶段 SIGINT：D ∈ [T/4, T/2] 处发信号 → 130，重开数据完整
/// （信号落在打开或执行阶段均可：两条路径的退出码与数据完整性一致）。
#[tokio::test]
async fn test_sigint_during_run_graceful_130() {
    let dir = fixture();
    build_big_wal(dir.path(), WAL_ROWS).await;

    let child = spawn_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t"]);
    std::thread::sleep(Duration::from_millis(SIGNAL_DELAY_MS));
    let rc = unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    assert_eq!(rc, 0, "kill(SIGINT) failed");

    let out = wait_cli(child);
    assert_eq!(
        out.code,
        Some(130),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    assert_row_count_intact(dir.path(), WAL_ROWS);
}

/// ② 打开阶段 SIGINT：以「子进程已持有主文件锁」为确定性锚点（信号
/// 处理器已安装、仍在恢复中）立即发信号 → 130，无挂起。
#[tokio::test]
async fn test_sigint_during_open_130() {
    let dir = fixture();
    build_big_wal(dir.path(), WAL_ROWS).await;
    let db_file = dir.path().join("db/app.db");

    let child = spawn_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let held_by_child = match std::fs::File::open(&db_file).unwrap().try_lock() {
            Err(std::fs::TryLockError::WouldBlock) => true,
            _ => false,
        };
        if held_by_child {
            break;
        }
        if Instant::now() > deadline {
            panic!("child never acquired the db file lock within 10s");
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let rc = unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    assert_eq!(rc, 0, "kill(SIGINT) failed");
    let out = wait_cli(child);
    assert_eq!(
        out.code,
        Some(130),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}

/// ③ SIGTERM 同语义：143。
#[tokio::test]
async fn test_sigterm_during_run_143() {
    let dir = fixture();
    build_big_wal(dir.path(), WAL_ROWS).await;

    let child = spawn_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t"]);
    std::thread::sleep(Duration::from_millis(SIGNAL_DELAY_MS));
    let rc = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) failed");

    let out = wait_cli(child);
    assert_eq!(
        out.code,
        Some(143),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}

/// ④ kill -9 守护（file-lock R2 强杀后无死锁 + MS10-T02 稳定基线
/// "kill 后 WAL 恢复 e2e"）：SIGKILL 无优雅路径，重开必须成功且数据完整。
#[tokio::test]
async fn test_sigkill_leaves_recoverable_db() {
    let dir = fixture();
    build_big_wal(dir.path(), WAL_ROWS).await;

    let child = spawn_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t"]);
    std::thread::sleep(Duration::from_millis(SIGNAL_DELAY_MS));
    let rc = unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
    assert_eq!(rc, 0, "kill(SIGKILL) failed");

    let out = wait_cli(child);
    assert_eq!(
        out.code, None,
        "SIGKILL death must be signal-death (no exit code)"
    );

    assert_row_count_intact(dir.path(), WAL_ROWS);
}

/// D5 标定助手（默认忽略，不进常规套件）：
/// `RTSQL_CALIBRATION_ROWS=20000 cargo test --test cli_test \
///   calibration_recovery_time -- --ignored --nocapture`
/// 输出 build 耗时与恢复耗时 T；T < 500ms 时倍增 N。
#[tokio::test]
#[ignore]
async fn calibration_recovery_time() {
    let n: i64 = std::env::var("RTSQL_CALIBRATION_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let dir = fixture();

    let build = Instant::now();
    build_big_wal(dir.path(), n).await;
    println!("build N={} wal: {:?}", n, build.elapsed());

    let t0 = Instant::now();
    let db = Database::open(&dir.path().join("db/app.db")).await.unwrap();
    println!("recovery T for N={}: {:?}", n, t0.elapsed());
    drop(db);
}

/// WAL 解析诊断（默认忽略）：构建后用 Reader 逐条解析 `.wal`，
/// 打印解析成功的记录数、出错位置与文件长度，用于定位截断/交叠。
#[tokio::test]
#[ignore]
async fn diagnostic_wal_parse() {
    let n: i64 = std::env::var("RTSQL_CALIBRATION_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let dir = fixture();
    build_big_wal(dir.path(), n).await;

    let wal_path = dir.path().join("db/app.wal");
    let wal_len = std::fs::metadata(&wal_path).unwrap().len();
    let mut reader = rtsql::wal::WalReader::open(&wal_path).unwrap();
    let mut count: u64 = 0;
    loop {
        match reader.read_next_with_lsn() {
            Ok(Some((_lsn, _rec))) => count += 1,
            Ok(None) => {
                println!("clean EOF after {count} records, wal_len={wal_len}");
                break;
            }
            Err(e) => {
                let pos = reader.current_position().unwrap();
                println!(
                    "parse error after {count} records at pos {pos}/{wal_len}: {e}"
                );
                let bytes = std::fs::read(&wal_path).unwrap();
                let start = pos.saturating_sub(60) as usize;
                let end = (pos + 80) as usize;
                for (i, b) in bytes[start..end.min(bytes.len())].iter().enumerate() {
                    print!("{:02x} ", b);
                    if (start + i + 1) % 16 == 0 {
                        println!();
                    }
                }
                println!();
                break;
            }
        }
    }
}
