//! MS10-T01 CLI 集成测试 —— 通过真二进制（`CARGO_BIN_EXE_rtsql`）验证
//! one-shot 执行、名称解析、渲染格式、退出码分类与 close 落盘语义。
//!
//! 每个 spawn 都以独立 TempDir 为 CWD，并把 `RTSQL_HOME` 指向该 TempDir
//! （并行安全，同时覆盖 R2 的 RTSQL_HOME 解析场景）；fixture 预建 `db/`
//! 子目录（CLI 不建目录，父目录缺失按契约报错退出 1）。stdout/stderr 始终
//! 为管道（非 TTY），因此默认格式为 JSON。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::page_format::ColumnType;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
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

/// 启动 rtsql 二进制且 stdin 为管道（`restore <db> -` 用例向其写入输入）。
fn spawn_cli_stdin(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary with piped stdin")
}

/// 向 stdin 管道写入输入后等待退出。子进程若在读 stdin 前退出，写入以
/// EPIPE 失败属预期（退出码断言承载结果），不视为夹具错误。
fn run_cli_stdin(dir: &Path, args: &[&str], input: &str) -> CliOutput {
    let mut child = spawn_cli_stdin(dir, args);
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin); // 关闭写端，子进程 read_to_string 见 EOF
    wait_cli(child)
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

/// ④ 多语句分片执行（MS10-T04 替换原护栏语义）：`;` 分隔的双 INSERT 逐条
/// 生效、顺序输出两段受影响行数、退出 0；重开可见两行（独立事务均已提交）。
#[test]
fn test_multi_statement_executes() {
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
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // 顺序两段受影响行数输出（非 TTY 默认 json）
    let mut docs = out.stdout.trim().lines();
    let first: serde_json::Value = serde_json::from_str(docs.next().expect("first doc")).unwrap();
    let second: serde_json::Value = serde_json::from_str(docs.next().expect("second doc")).unwrap();
    assert_eq!(first["affected_rows"], serde_json::json!(1));
    assert_eq!(second["affected_rows"], serde_json::json!(1));

    // 重开：两条独立事务均已提交（行序无关断言）
    let out = run_cli(dir.path(), &["app", "SELECT name FROM users"]);
    assert_eq!(out.code, Some(0));
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    let mut names: Vec<String> = parsed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["Alice", "Bob", "Carol"],
        "both inserts must be committed"
    );
}

/// ④（S2）顺序渲染：INSERT 的受影响行文档在前，SELECT 的 rows 文档在后，
/// json 格式下为两个独立 JSON 文档（逐行 JSONL 风格）。
/// （SELECT 取两列投影：与既有单语句渲染语义逐字一致——见 test_piped_default_json；
/// Act 校准：全表扫描子集单列投影的表头为全 schema，属既有单语句行为，
/// 不在本 change 契约内，避免在多语句用例中锁死该形状。）
#[test]
fn test_multi_statement_sequential_render() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "INSERT INTO users VALUES (9, 'Zed'); SELECT id, name FROM users",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let lines: Vec<&str> = out.stdout.trim().lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected two independent JSON documents: {:?}",
        out.stdout
    );
    let insert_doc: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let select_doc: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(insert_doc["affected_rows"], serde_json::json!(1));
    assert_eq!(select_doc["columns"], serde_json::json!(["id", "name"]));
    let mut rows: Vec<(i64, String)> = select_doc["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[0].as_i64().unwrap(), r[1].as_str().unwrap().to_string()))
        .collect();
    rows.sort();
    assert_eq!(rows, vec![(1, "Alice".to_string()), (9, "Zed".to_string())]);
}

/// ④（S5，2026-09-08 用户批准修订：no-FROM SELECT 引擎不可达，见 Act Response
/// Blocker Resolution）分号边界：连续分号（空语句跳过）、字符串字面量内分号
/// （不分片）、尾随分号（合法）——按两条语句执行且退出 0。
#[test]
fn test_multi_statement_semicolon_boundaries() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT id FROM users;; SELECT name FROM users WHERE name = 'a;b';",
        ],
    );
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    let lines: Vec<&str> = out.stdout.trim().lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected exactly two statements' outputs: {:?}",
        out.stdout
    );
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first["rows"], serde_json::json!([[1]]));
    assert_eq!(
        second["rows"],
        serde_json::json!([]),
        "string literal 'a;b' matches no row (and must not be split)"
    );
}

/// ④（S3）fail-fast：中间语句失败 → 退出 3，错误含失败语句序号（第 2 条/共 3 条）
/// 与语句文本，并注明前序语句已生效；失败后语句未执行。
#[test]
fn test_multi_statement_fail_fast() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "INSERT INTO users VALUES (2, 'Bob'); INSERT INTO missing_table VALUES (3); INSERT INTO users VALUES (4, 'Dan')",
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
        out.stderr.contains("statement 2 of 3"),
        "error must locate the failing statement: {:?}",
        out.stderr
    );
    assert!(
        out.stderr.contains("missing_table"),
        "error must contain the failing statement text: {:?}",
        out.stderr
    );
    assert!(
        out.stderr.contains("previous statement(s) were committed"),
        "error must note prior statements took effect: {:?}",
        out.stderr
    );

    // 部分生效：仅第 1 条 INSERT 落库（fail-fast 阻止第 3 条）
    let out = run_cli(dir.path(), &["app", "SELECT id FROM users"]);
    assert_eq!(out.code, Some(0));
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    let mut ids: Vec<i64> = parsed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[0].as_i64().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, vec![1, 2], "only the first statement may take effect");
}

/// ④（S4）语法错误整体拒绝：parse 在任何语句执行前失败（零执行），
/// 错误保留解析器行/列定位文本（不套语句序号模板）。
#[test]
fn test_multi_statement_parse_error_zero_exec() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &["app", "INSERT INTO users VALUES (2, 'Bob'); SELEC typo"],
    );
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("Line:"),
        "parse error must keep parser line/column location: {:?}",
        out.stderr
    );

    // 零执行：parse 失败前的 INSERT 也未生效
    let out = run_cli(dir.path(), &["app", "SELECT id FROM users"]);
    assert_eq!(out.code, Some(0));
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[1]]),
        "no statement may execute when parse fails"
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

// ---- T03：文件格式头（database-file-format-header spec） ----

/// R2-S1：8KiB 垃圾文件干净拒绝——exit 1 + stderr 报 not an RTsql database
/// 与路径，文件内容未被修改。（RED 基线：catalog 解析 panic → abort，code=None）
#[test]
fn test_garbage_file_clean_rejection_exit_1() {
    let dir = fixture();
    let junk = dir.path().join("garbage.db");
    let bytes: Vec<u8> = (0..8192usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&junk, &bytes).unwrap();

    let out = run_cli(dir.path(), &[junk.to_str().unwrap(), "SELECT 1"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("not an RTsql database"),
        "stderr must name the format problem: {:?}",
        out.stderr
    );
    assert!(
        out.stderr.contains(junk.to_str().unwrap()),
        "stderr must contain the db path: {:?}",
        out.stderr
    );
    assert_eq!(
        std::fs::read(&junk).unwrap(),
        bytes,
        "rejection must not modify the file"
    );
}

/// R2-S2：文件由新版创建 → exit 1 + "newer version" 文案。
#[test]
fn test_newer_version_file_exit_1() {
    let dir = fixture();
    let db = dir.path().join("future.db");
    let mut bytes = vec![0u8; 64 + 4096];
    bytes[..8].copy_from_slice(b"RTSQLDB\0");
    bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&4096u32.to_le_bytes());
    std::fs::write(&db, &bytes).unwrap();

    let out = run_cli(dir.path(), &[db.to_str().unwrap(), "SELECT 1"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("newer version"),
        "stderr must say the file is from a newer version: {:?}",
        out.stderr
    );
}

/// R3-S2：头拒绝不触碰伴生文件——失败后同目录无 `.wal` / `.checkpoint`。
#[test]
fn test_header_rejection_leaves_no_companion_files() {
    let dir = fixture();
    let db = dir.path().join("bad.db");
    std::fs::write(&db, vec![0u8; 8192]).unwrap();

    let out = run_cli(dir.path(), &[db.to_str().unwrap(), "SELECT 1"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        !dir.path().join("bad.wal").exists(),
        "wal must not be created on rejection"
    );
    assert!(
        !dir.path().join("bad.checkpoint").exists(),
        "checkpoint must not be created on rejection"
    );
}

/// R3-S1：锁先于头校验——垃圾文件被持锁 → exit 4 而非格式错 1。（GREEN 守卫）
#[test]
fn test_lock_precedes_header_validation_exit_4() {
    let dir = fixture();
    let db = dir.path().join("locked-garbage.db");
    std::fs::write(&db, vec![0u8; 8192]).unwrap();

    let holder = std::fs::File::open(&db).unwrap();
    holder.try_lock().expect("test process acquires flock");

    let out = run_cli(dir.path(), &[db.to_str().unwrap(), "SELECT 1"]);
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

/// D5 标定结果（2026-09-08 本机 WSL2 实测，D10 恢复重建落地后重标，见 Act Response）：
/// N 行单会话 WAL 的恢复（open）耗时 T——40k→8.86s、160k→40.7s（驱逐规模下
/// 重放后重建 B-Tree 在 100 页池上的随机访存主导，小库无驱逐档 T 为毫秒级，
/// 200ms ∈ [T/4, T/2] 的窗口恰在耗时悬崖内、不可稳健命中）。T ≥ 500ms 以
/// 17 倍余量满足；D=200ms 低于 T/4 → 信号确定落在打开阶段（两条 e2e 用例
/// 注释均容忍该落点，执行+close 阶段见证由 D5-⑤ 库级结构测试确定性承担）。
/// 40k 档兼顾夹具驱逐阈值（>100 页，catalog 随驱逐落盘）与套件耗时。
const WAL_ROWS: i64 = 40_000;
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
        let held_by_child = matches!(
            std::fs::File::open(&db_file).unwrap().try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        );
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

    // D5-② 观测物：打开阶段信号走无 close 分支——WAL 保持未截断的大文件。
    let wal_len = std::fs::metadata(dir.path().join("db/app.wal"))
        .expect("wal file must exist")
        .len();
    assert!(
        wal_len > 2048,
        "open-phase signal must not run close(): WAL still large, got {wal_len} bytes"
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
                println!("parse error after {count} records at pos {pos}/{wal_len}: {e}");
                let bytes = std::fs::read(&wal_path).unwrap();
                let start = pos.saturating_sub(60) as usize;
                let end = (pos + 80) as usize;
                for (i, b) in bytes[start..end.min(bytes.len())].iter().enumerate() {
                    print!("{:02x} ", b);
                    if (start + i + 1).is_multiple_of(16) {
                        println!();
                    }
                }
                println!();
                break;
            }
        }
    }
}

// ---- T05 Iteration 000：入口子命令分发（MS10-T05） ----

/// 主命令缺 SQL 参数：退出 2（T1 前由 clap 自动承接；T1 后由手动 usage
/// 分支承接同一退出码——行为保持见证，非 RED 项）。
#[test]
fn test_missing_sql_arg_exit_2() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["app"]);
    assert_eq!(
        out.code,
        Some(2),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}

/// 子命令分发：`rtsql list` 分发到 list 子命令执行（T4 实现后收紧为
/// exit 0 + 行集输出），不再落入主命令"缺 SQL"的旧 clap 解析路径。
#[test]
fn test_subcommand_dispatch_list_runs() {
    let dir = fixture();
    std::fs::write(dir.path().join("db/a.db"), vec![0u8; 16]).unwrap();
    let out = run_cli(dir.path(), &["list"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stderr.contains("required arguments were not provided"),
        "must not take the main-command missing-SQL parse path: {:?}",
        out.stderr
    );
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([["a.db", 16]]),
        "list must have executed (not the placeholder): {:?}",
        out.stdout
    );
}

/// R-list-S1：枚举集中区 `*.db` 常规文件（名称排序、字节数），非 `.db` 不出现。
#[test]
fn test_list_enumerates_db_files() {
    let dir = fixture();
    std::fs::write(dir.path().join("db/a.db"), vec![0u8; 100]).unwrap();
    std::fs::write(dir.path().join("db/b.db"), vec![0u8; 200]).unwrap();
    std::fs::write(dir.path().join("db/notes.txt"), b"notes").unwrap();

    let out = run_cli(dir.path(), &["list"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["columns"], serde_json::json!(["name", "size_bytes"]));
    assert_eq!(
        parsed["rows"],
        serde_json::json!([["a.db", 100], ["b.db", 200]]),
        "sorted by name; non-.db files excluded: {:?}",
        out.stdout
    );
}

/// R-list-S2：`db/` 目录不存在或不含任何 `*.db` → 空行集，exit 0。
#[test]
fn test_list_empty_or_missing_dir() {
    let dir = TempDir::new().unwrap(); // 无 db/ 目录
    let out = run_cli(dir.path(), &["list"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([]));

    // 空目录变体
    let dir = fixture();
    let out = run_cli(dir.path(), &["list"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([]));
}

/// R-schema-S1：表结构 DDL 输出（列、类型、PRIMARY KEY、NOT NULL）。
#[test]
fn test_schema_outputs_ddl() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE items (id INT PRIMARY KEY, label STRING NOT NULL)",
        ],
    );
    assert_eq!(out.code, Some(0), "create table failed: {}", out.stderr);

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("CREATE TABLE \"items\""),
        "table ident must be quoted: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"id\" INT PRIMARY KEY"),
        "PK column must be rendered: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"label\" STRING NOT NULL"),
        "NOT NULL constraint must be rendered: {:?}",
        out.stdout
    );
}

/// R-schema-S1 补充：UNIQUE 约束经 SQL 建库 → schema 输出往返（catalog 真实值）。
#[test]
fn test_schema_unique_roundtrip() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE tags (id INT PRIMARY KEY, name STRING UNIQUE)",
        ],
    );
    assert_eq!(out.code, Some(0), "create table failed: {}", out.stderr);

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("\"name\" STRING UNIQUE"),
        "UNIQUE constraint must be rendered: {:?}",
        out.stdout
    );
}

/// R-schema-S3：库文件不存在 → exit 1 + stderr 含 `does not exist`，不创建文件。
#[test]
fn test_schema_missing_db_errors() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["schema", "missing"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("does not exist"),
        "stderr must say the db does not exist: {:?}",
        out.stderr
    );
    assert!(
        !dir.path().join("db/missing.db").exists(),
        "schema must not create any file"
    );
}

/// R-schema-S2：空库（无用户表）→ 无输出，exit 0。
#[test]
fn test_schema_empty_db_no_output() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.is_empty(),
        "empty db must produce no output: {:?}",
        out.stdout
    );
}

/// R-new-S1：裸名新建 + 缺失 `db/` 目录自动创建；静默 exit 0；
/// 建库结果紧接主命令立即可用。
#[test]
fn test_new_creates_db_and_dirs() {
    let dir = TempDir::new().unwrap(); // 不预建 db/ 子目录
    assert!(!dir.path().join("db").exists());

    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(out.stdout.is_empty(), "new must be silent on success");
    assert!(
        dir.path().join("db/app.db").exists(),
        "db/ directory and app.db must be created"
    );

    let out = run_cli(dir.path(), &["app", "CREATE TABLE t (id INT)"]);
    assert_eq!(
        out.code,
        Some(0),
        "newly created db must be immediately usable: {}",
        out.stderr
    );
}

/// R-new-S2：含 `/` 路径新建 + 缺失父目录创建。
#[test]
fn test_new_path_creates_parents() {
    let dir = TempDir::new().unwrap();
    let out = run_cli(dir.path(), &["new", "x/y/data.db"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        dir.path().join("x/y/data.db").exists(),
        "parent directories and target file must be created"
    );
}

/// R-new-S3：目标已存在（含 0 字节）→ exit 1 + stderr 含 `already exists`，
/// 文件内容不变。
#[test]
fn test_new_existing_file_rejected() {
    let dir = fixture();
    let target = dir.path().join("db/app.db");
    std::fs::write(&target, b"payload").unwrap();

    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("already exists"),
        "stderr must say the target already exists: {:?}",
        out.stderr
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"payload",
        "existing file must not be modified"
    );

    // 0 字节文件变体：同样拒绝且保持 0 字节
    std::fs::write(&target, b"").unwrap();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(1), "0-byte file must also be rejected");
    assert!(
        out.stderr.contains("already exists"),
        "stderr must say the target already exists: {:?}",
        out.stderr
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().len(),
        0,
        "0-byte file must stay untouched"
    );
}

// ---- T05 Iteration 001：数据面子命令（dump/restore/import） ----

/// R-dump-restore-S2：空库 dump 无输出，exit 0。
#[test]
fn test_dump_empty_db_no_output() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);

    let out = run_cli(dir.path(), &["dump", "app"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.is_empty(),
        "empty db dump must produce no output: {:?}",
        out.stdout
    );
}

/// R-dump-restore-S1：dump-restore 往返等价。dump 产物为 SQL 文本
/// （DDL + 逐行 INSERT，字符串单引号加倍、NULL/TRUE/FALSE 字面量）。
/// （T6 交付 dump 产物断言；restore 执行与 SELECT 比对随 T7 扩展转绿。）
#[test]
fn test_dump_restore_roundtrip() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE items (id INT PRIMARY KEY, label STRING, price FLOAT, in_stock BOOL)",
        ],
    );
    assert_eq!(out.code, Some(0), "create table failed: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "app",
            // 种子取引擎 INSERT 通道可达值：planner extract_insert_values 只接受
            // Expr::Value/裸 NULL（负数字面量为 UnaryOp → UnsupportedValue，既有限制）
            "INSERT INTO items VALUES (1, 'it''s ok', 3.5, TRUE); INSERT INTO items VALUES (2, NULL, 0.25, FALSE)",
        ],
    );
    assert_eq!(out.code, Some(0), "seed failed: {}", out.stderr);

    let dump_out = run_cli(dir.path(), &["dump", "app"]);
    assert_eq!(
        dump_out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        dump_out.stdout,
        dump_out.stderr
    );
    assert!(
        dump_out.stdout.contains("CREATE TABLE \"items\""),
        "dump must contain DDL: {:?}",
        dump_out.stdout
    );
    assert!(
        dump_out.stdout.contains("INSERT INTO \"items\" VALUES"),
        "dump must contain INSERT statements: {:?}",
        dump_out.stdout
    );
    assert!(
        dump_out.stdout.contains("'it''s ok'"),
        "string literals must escape single quotes: {:?}",
        dump_out.stdout
    );
    assert!(
        dump_out.stdout.contains("NULL"),
        "NULL must render as SQL literal: {:?}",
        dump_out.stdout
    );
    assert!(
        dump_out.stdout.contains("TRUE") && dump_out.stdout.contains("FALSE"),
        "booleans must render as SQL literals: {:?}",
        dump_out.stdout
    );

    // restore 侧（T7）：dump 文本 → new b → restore b → SELECT 比对
    let dump_file = dir.path().join("dump.sql");
    std::fs::write(&dump_file, &dump_out.stdout).unwrap();

    let out = run_cli(dir.path(), &["new", "b"]);
    assert_eq!(out.code, Some(0), "new b failed: {}", out.stderr);
    let out = run_cli(dir.path(), &["restore", "b", dump_file.to_str().unwrap()]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(out.stdout.is_empty(), "restore must be silent on success");

    // 库 b 数据比对。表名经 dump 的带引号 DDL 重建后为 Display 形式（引擎以
    // ObjectName to_string 为表名，见 lifecycle.rs select_all_rows 注记），
    // 因此比对 SELECT 用带引号形式。
    let out = run_cli(
        dir.path(),
        &[
            "b",
            "SELECT id, label, price, in_stock FROM \"items\"",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[1, "it's ok", 3.5, true], [2, null, 0.25, false]])
    );
}

/// R-dump-restore-S1 全形状 + R-import-S3/S4 上游数据面（001-rework T8-R3）：
/// 含无键行（键位 NULL / String 首列隐式 PK）的库 dump → new → restore →
/// SELECT 比对等价。T8-R1 前：无键行在 INSERT 通道被静默丢弃，restore 后
/// 丢失；T8-R2 前：无键行版本链使重开 RedoFailed。
#[test]
fn test_dump_restore_roundtrip_full_shape() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "CREATE TABLE mixed (a INT, b FLOAT, c BOOL, d STRING); CREATE TABLE notes (s STRING, v INT)",
        ],
    );
    assert_eq!(out.code, Some(0), "create tables failed: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "app",
            // mixed：(NULL,…) 键位 NULL 行 + 键位 Int 行（含 NULL 非键字段、
            // String 空串）；notes：String 首列隐式 PK，全部行无键
            "INSERT INTO mixed VALUES (NULL, 2.5, TRUE, 'keep'); INSERT INTO mixed VALUES (7, NULL, FALSE, ''); INSERT INTO notes VALUES ('x', 1); INSERT INTO notes VALUES ('y', 2)",
        ],
    );
    assert_eq!(out.code, Some(0), "seed failed: {}", out.stderr);

    let dump_out = run_cli(dir.path(), &["dump", "app"]);
    assert_eq!(
        dump_out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        dump_out.stdout,
        dump_out.stderr
    );
    assert!(
        dump_out
            .stdout
            .contains("INSERT INTO \"mixed\" VALUES (NULL, 2.5, TRUE, 'keep')"),
        "keyless row must survive in dump: {:?}",
        dump_out.stdout
    );

    let dump_file = dir.path().join("dump_full.sql");
    std::fs::write(&dump_file, &dump_out.stdout).unwrap();

    let out = run_cli(dir.path(), &["new", "b"]);
    assert_eq!(out.code, Some(0), "new b failed: {}", out.stderr);
    let out = run_cli(dir.path(), &["restore", "b", dump_file.to_str().unwrap()]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(out.stdout.is_empty(), "restore must be silent on success");

    // 比对 1：mixed 两行全在（无键行 + 键位 Int 行），空字段语义保持
    let out = run_cli(
        dir.path(),
        &["b", "SELECT a, b, c, d FROM \"mixed\"", "--format", "json"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[null, 2.5, true, "keep"], [7, null, false, ""]]),
        "keyless row and keyed row must round-trip"
    );

    // 比对 2：String 首列表全部行（无键）往返等价
    let out = run_cli(
        dir.path(),
        &["b", "SELECT s, v FROM \"notes\"", "--format", "json"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([["x", 1], ["y", 2]]),
        "String-first-column rows must round-trip"
    );
}

/// R-dump-restore-S4：目标库已含用户表 → exit 1 拒绝，库内容不变。
#[test]
fn test_restore_rejects_nonempty_target() {
    let dir = fixture();
    seed_users(dir.path());

    let dump_file = dir.path().join("dump.sql");
    std::fs::write(&dump_file, "CREATE TABLE t (id INT);\n").unwrap();

    let out = run_cli(dir.path(), &["restore", "app", dump_file.to_str().unwrap()]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stderr.is_empty(), "rejection must go to stderr");

    // 库内容不变：users 仍只有种子一行
    let out = run_cli(dir.path(), &["app", "SELECT COUNT(*) FROM users"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([[1]]));
}

/// R-dump-restore-S5：执行期语句失败 fail-fast → exit 3 + stderr 含失败语句
/// 序号与前序已生效注记；失败前语句重开可见，其后语句未执行。
#[test]
fn test_restore_fail_fast() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "b"]);
    assert_eq!(out.code, Some(0), "new b failed: {}", out.stderr);

    let sql_file = dir.path().join("dup.sql");
    std::fs::write(
        &sql_file,
        "CREATE TABLE t (id INT PRIMARY KEY);\nINSERT INTO t VALUES (1);\nINSERT INTO t VALUES (1);\nINSERT INTO t VALUES (3);\n",
    )
    .unwrap();

    let out = run_cli(dir.path(), &["restore", "b", sql_file.to_str().unwrap()]);
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("statement 3 of 4"),
        "error must locate the failing statement: {:?}",
        out.stderr
    );
    assert!(
        out.stderr.contains("previous statement(s) were committed"),
        "error must note prior statements took effect: {:?}",
        out.stderr
    );

    // 第 1、2 条已生效（表 + 1 行），第 4 条未执行。
    // SQL 文件用裸名 CREATE，故表名无引号（与 dump 带引号 DDL 重建的场景相反）
    let out = run_cli(dir.path(), &["b", "SELECT COUNT(*) FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[1]]),
        "only the first two statements may take effect"
    );
}

/// R-dump-restore-S3：`dump a | restore b -` stdin 管道往返，数据等价。
#[test]
fn test_restore_stdin_pipe() {
    let dir = fixture();
    seed_users(dir.path());

    let dump_out = run_cli(dir.path(), &["dump", "app"]);
    assert_eq!(dump_out.code, Some(0), "dump failed: {}", dump_out.stderr);

    let out = run_cli(dir.path(), &["new", "b"]);
    assert_eq!(out.code, Some(0), "new b failed: {}", out.stderr);

    let out = run_cli_stdin(dir.path(), &["restore", "b", "-"], &dump_out.stdout);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    let out = run_cli(
        dir.path(),
        &["b", "SELECT id, name FROM \"users\"", "--format", "json"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[1, "Alice"]]),
        "restored db must equal source"
    );
}

// ---- T05 Iteration 001：import --csv（R-import） ----

/// R-import-S1/S2：基本导入 + 乱序表头按列名映射；成功输出 affected_rows；
/// 缺 `--csv` flag → Usage exit 2（先于一切 IO）。
#[test]
fn test_import_basic_and_header_order() {
    let dir = fixture();
    seed_users(dir.path());

    // 缺 --csv：Usage exit 2
    let out = run_cli(dir.path(), &["import", "app", "users", "data.csv"]);
    assert_eq!(
        out.code,
        Some(2),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // 乱序表头：name,id 与表定义 (id, name) 相反，按列名映射
    std::fs::write(dir.path().join("data.csv"), "name,id\nBob,2\nCarol,3\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "users", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(2),
        "import must report affected rows: {:?}",
        out.stdout
    );

    // 按列名正确映射入库
    let out = run_cli(dir.path(), &["app", "SELECT id, name FROM users"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[1, "Alice"], [2, "Bob"], [3, "Carol"]]),
        "values must map by column name, not position: {:?}",
        out.stdout
    );
}

/// R-import-S3：类型转换与空字段语义（Int 空→NULL、Float/Bool 按类型、
/// String 空→空串），SELECT 比对验证。
#[test]
fn test_import_types_and_empty_fields() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE t (a INT, b FLOAT, c BOOL, d STRING)"],
    );
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    std::fs::write(
        dir.path().join("data.csv"),
        "a,b,c,d\n,2.5,TRUE,keep\n7,,false,\n",
    )
    .unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    let out = run_cli(dir.path(), &["app", "SELECT a, b, c, d FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[null, 2.5, true, "keep"], [7, null, false, ""]]),
        "empty fields must map to NULL (non-String) / empty string (String): {:?}",
        out.stdout
    );
}

/// R-import-S5：转换失败 fail-fast → exit 1 + stderr 含数据行定位与原值；
/// 失败前已提交的行保留（重开可见）。
#[test]
fn test_import_conversion_fail_fast() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);
    let out = run_cli(dir.path(), &["app", "CREATE TABLE t (a INT)"]);
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    std::fs::write(dir.path().join("data.csv"), "a\n1\nabc\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("row 2") && out.stderr.contains("abc"),
        "error must locate the failing data row and original value: {:?}",
        out.stderr
    );

    // 第 1 行已入库（逐条 auto-commit）
    let out = run_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([[1]]));
}

/// R-import-S6：表头不匹配（缺表列 / 含表外列）→ exit 1，表内无新行。
#[test]
fn test_import_header_mismatch_rejected() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);
    let out = run_cli(dir.path(), &["app", "CREATE TABLE t (a INT, b INT)"]);
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    // 表头缺 b 列
    std::fs::write(dir.path().join("missing.csv"), "a\n1\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "missing.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains('b'),
        "error must name the missing column: {:?}",
        out.stderr
    );

    // 表头含表外列
    std::fs::write(dir.path().join("extra.csv"), "a,b,c\n1,2,3\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "extra.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains('c'),
        "error must name the unknown column: {:?}",
        out.stderr
    );

    // 表内无新行
    let out = run_cli(dir.path(), &["app", "SELECT COUNT(*) FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([[0]]));
}

/// R-import-S7：目标表不存在 / 目标库不存在 → exit 1（库缺失文案含
/// `does not exist`）。
#[test]
fn test_import_missing_table_or_db() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);

    std::fs::write(dir.path().join("data.csv"), "id\n1\n").unwrap();

    // 表不存在
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(!out.stderr.is_empty(), "missing table must go to stderr");

    // 库不存在
    let out = run_cli(dir.path(), &["import", "missing", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(1),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("does not exist"),
        "missing db error must follow the shared wording: {:?}",
        out.stderr
    );
}

/// R-import-S8：RFC4180 引号转义——字段含逗号、双引号、跨行文本完整入库。
#[test]
fn test_import_quoted_fields() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);
    let out = run_cli(dir.path(), &["app", "CREATE TABLE t (id INT, note STRING)"]);
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    std::fs::write(
        dir.path().join("data.csv"),
        "id,note\n1,\"has,comma\"\n2,\"has\"\"quote\"\n3,\"line1\nline2\"\n",
    )
    .unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    let out = run_cli(dir.path(), &["app", "SELECT id, note FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["rows"],
        serde_json::json!([[1, "has,comma"], [2, "has\"quote"], [3, "line1\nline2"]]),
        "quoted fields must survive RFC4180 unescaping: {:?}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// MS11-T01 Iteration 001（R4/S1-S4）：SELECT 派生列表头与渲染兼容。
// 值语义见 tests/projection_expression_test.rs；此处断言 lib Response 不
// 携带的表头（Display 文本命名）与四格式渲染。
// ---------------------------------------------------------------------------

/// R4/S1：AS 别名表头 + 逐行求值（默认非 TTY JSON）
#[test]
fn test_projection_derived_column_alias_header() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT name, CASE WHEN id >= 1 THEN 'Y' ELSE 'N' END AS passed FROM users",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["columns"], serde_json::json!(["name", "passed"]));
    assert_eq!(parsed["rows"], serde_json::json!([["Alice", "Y"]]));
}

/// R4/S1（后半）：无别名派生列表头 = CASE 表达式 Display 文本
#[test]
fn test_projection_derived_column_display_header() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT name, CASE WHEN id >= 1 THEN 'Y' ELSE 'N' END FROM users",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["columns"],
        serde_json::json!(["name", "CASE WHEN id >= 1 THEN 'Y' ELSE 'N' END"])
    );
}

/// R4/S2：混合列 + COALESCE 表头（Display 原文）与行值
#[test]
fn test_projection_coalesce_header() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &["app", "SELECT id, COALESCE(NULL, id) FROM users"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["columns"],
        serde_json::json!(["id", "COALESCE(NULL, id)"])
    );
    assert_eq!(parsed["rows"], serde_json::json!([[1, 1]]));
}

/// 怪癖修正：`SELECT 42 FROM t` 单列常量（表头 `42`），不再恒等回退全 schema
#[test]
fn test_projection_constant_single_column() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(dir.path(), &["app", "SELECT 42 FROM users"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["columns"], serde_json::json!(["42"]));
    assert_eq!(parsed["rows"], serde_json::json!([[42]]));
}

/// R4/S4：csv 渲染兼容（派生列值均为既有 Value 变体）
#[test]
fn test_projection_csv_render() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT name, CASE WHEN id >= 1 THEN 'Y' ELSE 'N' END AS passed FROM users",
            "--format",
            "csv",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "name,passed\nAlice,Y\n");
}

/// R4/S4：table 渲染兼容（表头 + 行值）
#[test]
fn test_projection_table_render() {
    let dir = fixture();
    seed_users(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "SELECT name, CASE WHEN id >= 1 THEN 'Y' ELSE 'N' END AS passed FROM users",
            "--format",
            "table",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("name") && out.stdout.contains("passed"),
        "missing derived column headers in table output: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Alice") && out.stdout.contains("Y"),
        "missing derived column values in table output: {:?}",
        out.stdout
    );
}
