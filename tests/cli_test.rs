//! MS10-T01 CLI 集成测试 —— 通过真二进制（`CARGO_BIN_EXE_rtsql`）验证
//! one-shot 执行、名称解析、渲染格式、退出码分类与 close 落盘语义。
//!
//! 每个 spawn 都以独立 TempDir 为 CWD，并把 `RTSQL_HOME` 指向该 TempDir
//! （并行安全，同时覆盖 R2 的 RTSQL_HOME 解析场景）；fixture 预建 `db/`
//! 子目录（CLI 不建目录，父目录缺失按契约报错退出 1）。stdout/stderr 始终
//! 为管道（非 TTY），因此默认格式为 JSON。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （one-shot CLI 必须自行退出，挂起即为行为错误）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");

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
