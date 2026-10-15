//! e2e 旅程级验证套件（change 2026-10-13-e2e-verification-and-defect-closure / R1）。
//!
//! 以真实 CLI 入口（`CARGO_BIN_EXE_rtsql` 子进程 + 磁盘 + stdin）覆盖 E1–E13
//! 用户旅程，断言用户实际所见：退出码 / stdout / stderr / 磁盘状态。
//! 已知缺陷（ISS05–ISS14）交互点以 `#[ignore = "ISSxx 待 Tn 修复"]` 标记，
//! 由对应修复 Iteration 解除；P2-10/11 输出形态按用户决策 D-1 锁定现状断言。
//!
//! 夹具模式与 R31 runbook 约束对齐 `tests/cli_test.rs`：独立 TempDir 为 CWD、
//! `RTSQL_HOME` 指向该目录、stdout/stderr 管道（非 TTY → 默认 JSON）、
//! SQL 负载 ≤128 KiB、60s 兜底等待。

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

/// 启动 rtsql 二进制但不等待退出（信号/持锁用例需要向存活进程发信号）。
fn spawn_cli(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary")
}

/// 启动 rtsql 二进制且 stdin 为管道（`restore <db> -` / `import <db> -` 用例）。
fn spawn_cli_stdin(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
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
/// stdout/stderr 由独立读线程并发消费（R31：防管道阻塞——子进程输出超过
/// 管道缓冲（64KB）时若无人并发读取将死锁；E8 大值旅程输出 ~1MB）。
fn wait_cli(mut child: Child) -> CliOutput {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let stdout_reader = std::thread::spawn(move || {
        let mut s = String::new();
        stdout_pipe.read_to_string(&mut s).unwrap();
        s
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut s = String::new();
        stderr_pipe.read_to_string(&mut s).unwrap();
        s
    });
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let stdout = stdout_reader.join().unwrap();
                let stderr = stderr_reader.join().unwrap();
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
/// （CLI 不建目录，父目录缺失按契约报错退出 1）。
fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

/// 解析单语句 JSON 输出（用户实际所见的 stdout 文本）为 serde_json::Value，
/// 仅作断言便利，不构成判定层。
fn json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout.trim()).expect("single-statement JSON output")
}

/// 多语句 JSONL 输出的最后一个对象（ATTACH/SET 类前置语句 + 结果语句形态）。
fn last_json(stdout: &str) -> serde_json::Value {
    let line = stdout
        .trim()
        .lines()
        .last()
        .expect("non-empty JSONL output");
    serde_json::from_str(line).expect("valid JSONL line")
}

/// 便捷断言：单语句 JSON 输出的列名与行内容。
fn assert_rows(out: &CliOutput, columns: serde_json::Value, rows: serde_json::Value) {
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert_eq!(parsed["columns"], columns, "stdout: {}", out.stdout);
    assert_eq!(parsed["rows"], rows, "stdout: {}", out.stdout);
}

/// E1 全生命周期：new → 建表（约束+DEFAULT+重复列名拒绝）→ 写矩阵
/// （子集 INSERT/UPSERT 三形态/UPDATE 表达式多列/全表 UPDATE+DELETE/带 WHERE）
/// → 读矩阵（JOIN 三型/聚合/CTE/视图/集合/DISTINCT/别名排序/派生表/IN 子查询）
/// → checkpoint → dump → restore 新库 → 数据逐行对照 → delete 逐文件报告。
#[test]
fn e1_full_lifecycle_journey() {
    let dir = fixture();

    // new：创建通道收敛，主命令不再隐式建库
    let out = run_cli(dir.path(), &["new", "shop"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(dir.path().join("db").join("shop.db").exists());

    // 建表：约束（NOT NULL + UNIQUE）+ DEFAULT 声明
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "CREATE TABLE users (id INT PRIMARY KEY, name STRING NOT NULL, age INT DEFAULT 18)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":0}"#); // D-1：DDL affected_rows:0

    // 重复列名：计划期点名拒绝（exit 3）零副作用
    let out = run_cli(dir.path(), &["shop", "CREATE TABLE bad (id INT, id INT)"]);
    assert_eq!(out.code, Some(3), "stdout: {:?}", out.stdout);
    let schema = run_cli(dir.path(), &["schema", "shop"]);
    assert_eq!(schema.code, Some(0));
    assert!(
        !schema.stdout.contains("bad"),
        "duplicate-column table must not be created: {}",
        schema.stdout
    );

    // 写矩阵：子集 INSERT + DEFAULT 应用
    let out = run_cli(
        dir.path(),
        &["shop", "INSERT INTO users (id, name) VALUES (1, 'Alice')"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":1}"#);

    // 完整列 INSERT
    let out = run_cli(
        dir.path(),
        &["shop", "INSERT INTO users VALUES (2, 'Bob', 25)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // UPSERT 三形态：ON CONFLICT DO NOTHING（已有 id=1 → 不插入）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "INSERT INTO users VALUES (1, 'Ignored', 99) ON CONFLICT DO NOTHING",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":0}"#);

    // UPSERT 三形态：ON CONFLICT DO UPDATE SET（字面量 + excluded 引用）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "INSERT INTO users VALUES (1, 'AliceUpdated', 30) ON CONFLICT DO UPDATE SET name = excluded.name",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":1}"#);

    // UPSERT 三形态：REPLACE INTO
    let out = run_cli(
        dir.path(),
        &["shop", "REPLACE INTO users VALUES (2, 'BobReplaced', 26)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":1}"#);

    // UPDATE 表达式多列（同旧行快照语义）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "UPDATE users SET age = age + 1, name = name WHERE id = 2",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":1}"#);

    // 读矩阵数据基线
    let out = run_cli(dir.path(), &["shop", "SELECT * FROM users ORDER BY id"]);
    assert_rows(
        &out,
        serde_json::json!(["id", "name", "age"]),
        serde_json::json!([[1, "AliceUpdated", 18], [2, "BobReplaced", 27]]),
    );

    // 带 WHERE 的 UPDATE
    let out = run_cli(
        dir.path(),
        &["shop", "UPDATE users SET age = 31 WHERE id = 1"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 全表 UPDATE（无 WHERE）+ 全表 DELETE（独立小表，避免污染主数据）
    let out = run_cli(dir.path(), &["shop", "CREATE TABLE scratch (k INT, v INT)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "INSERT INTO scratch VALUES (1, 10), (2, 20), (3, 30)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["shop", "UPDATE scratch SET v = v * 2"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":3}"#);
    let out = run_cli(dir.path(), &["shop", "DELETE FROM scratch"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":3}"#);
    let out = run_cli(dir.path(), &["shop", "SELECT COUNT(*) AS c FROM scratch"]);
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[0]]));

    // 读矩阵：JOIN 三型（LEFT/INNER/FULL 形态各自可达）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "CREATE TABLE orders (oid INT PRIMARY KEY, user_id INT, total INT)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "INSERT INTO orders VALUES (10, 1, 100), (11, 1, 200), (12, 3, 300)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT users.name, orders.total FROM users JOIN orders ON users.id = orders.user_id ORDER BY orders.total",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert_eq!(parsed["columns"], serde_json::json!(["name", "total"]));
    assert_eq!(
        parsed["rows"],
        serde_json::json!([["AliceUpdated", 100], ["AliceUpdated", 200]])
    );

    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT users.name, orders.total FROM users LEFT JOIN orders ON users.id = orders.user_id ORDER BY users.id, orders.total",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert_eq!(
        parsed["rows"],
        serde_json::json!([
            ["AliceUpdated", 100],
            ["AliceUpdated", 200],
            ["BobReplaced", null]
        ])
    );

    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT users.name, orders.total FROM users FULL JOIN orders ON users.id = orders.user_id ORDER BY orders.total",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert_eq!(
        parsed["rows"],
        serde_json::json!([
            ["AliceUpdated", 100],
            ["AliceUpdated", 200],
            [null, 300],
            ["BobReplaced", null]
        ])
    );

    // 读矩阵：聚合 + GROUP BY
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT user_id, COUNT(*) AS cnt, SUM(total) AS s FROM orders GROUP BY user_id ORDER BY user_id",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["user_id", "cnt", "s"]),
        serde_json::json!([[1, 2, 300], [3, 1, 300]]),
    );

    // 读矩阵：CTE
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "WITH big AS (SELECT user_id, total FROM orders WHERE total > 150) SELECT user_id, total FROM big ORDER BY total",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["user_id", "total"]),
        serde_json::json!([[1, 200], [3, 300]]),
    );

    // 读矩阵：视图（普通列视图，非缺陷面）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "CREATE VIEW v_orders AS SELECT oid, user_id, total FROM orders WHERE total >= 200",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["shop", "SELECT oid, total FROM v_orders ORDER BY oid"],
    );
    assert_rows(
        &out,
        serde_json::json!(["oid", "total"]),
        serde_json::json!([[11, 200], [12, 300]]),
    );

    // 读矩阵：集合操作（UNION 去重）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT user_id FROM orders UNION SELECT id FROM users ORDER BY user_id",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["user_id"]),
        serde_json::json!([[1], [2], [3]]),
    );

    // 读矩阵：DISTINCT
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT DISTINCT user_id FROM orders ORDER BY user_id",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["user_id"]),
        serde_json::json!([[1], [3]]),
    );

    // 读矩阵：ORDER BY 输出别名排序
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT name AS n, age AS a FROM users ORDER BY a DESC, n",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["n", "a"]),
        serde_json::json!([["AliceUpdated", 31], ["BobReplaced", 27]]),
    );

    // 读矩阵：派生表（内层普通列，非缺陷面）
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT t.name FROM (SELECT name, age FROM users) t WHERE t.age > 26 ORDER BY t.name",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["name"]),
        serde_json::json!([["AliceUpdated"], ["BobReplaced"]]),
    );

    // 读矩阵：IN 子查询
    let out = run_cli(
        dir.path(),
        &[
            "shop",
            "SELECT name FROM users WHERE id IN (SELECT user_id FROM orders) ORDER BY name",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["name"]),
        serde_json::json!([["AliceUpdated"]]),
    );

    // checkpoint：one-shot 写入经 close 已收敛 → 零工作早退明示
    //（"wal truncated" 截断路径由 E3 崩溃旅程承载——pending WAL 现场）。
    let out = run_cli(dir.path(), &["checkpoint", "shop"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("no pending work"),
        "zero-work checkpoint message: {}",
        out.stdout
    );

    // dump → restore 新库 → 数据逐行对照
    let dump = run_cli(dir.path(), &["dump", "shop"]);
    assert_eq!(dump.code, Some(0), "stderr: {}", dump.stderr);
    assert!(
        dump.stdout.contains("CREATE TABLE") && dump.stdout.contains("INSERT INTO"),
        "dump must emit DDL+DML SQL: {}",
        dump.stdout
    );
    let out = run_cli(dir.path(), &["new", "shop2"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let restore = run_cli_stdin(dir.path(), &["restore", "shop2", "-"], &dump.stdout);
    assert_eq!(
        restore.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        restore.stdout,
        restore.stderr
    );
    let left = run_cli(dir.path(), &["shop", "SELECT * FROM users ORDER BY id"]);
    let right = run_cli(dir.path(), &["shop2", "SELECT * FROM users ORDER BY id"]);
    assert_eq!(left.stdout, right.stdout, "dump→restore 数据逐行对照");
    let left = run_cli(dir.path(), &["shop", "SELECT * FROM orders ORDER BY oid"]);
    let right = run_cli(dir.path(), &["shop2", "SELECT * FROM orders ORDER BY oid"]);
    assert_eq!(left.stdout, right.stdout, "dump→restore orders 对照");

    // delete：逐文件报告 + 物理移除
    let out = run_cli(dir.path(), &["delete", "shop"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        !dir.path().join("db").join("shop.db").exists(),
        "shop.db must be removed"
    );
    assert!(
        !dir.path().join("db").join("shop.wal").exists(),
        "shop.wal must be removed"
    );
}

/// E2 导入：CSV 文件 + stdin `-` + 目标表缺失自动建表 + `:memory:` import
/// + 表头冲突拒绝（零副作用）。
#[test]
fn e2_import_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "imp"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["imp", "CREATE TABLE people (id INT, name STRING, age INT)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 文件导入（既有表，表头按列名匹配）
    let csv_path = dir.path().join("people.csv");
    std::fs::write(&csv_path, "id,name,age\n1,Alice,30\n2,Bob,25\n").unwrap();
    let out = run_cli(
        dir.path(),
        &[
            "import",
            "imp",
            "people",
            csv_path.to_str().unwrap(),
            "--csv",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":2}"#);
    let out = run_cli(dir.path(), &["imp", "SELECT * FROM people ORDER BY id"]);
    assert_rows(
        &out,
        serde_json::json!(["id", "name", "age"]),
        serde_json::json!([[1, "Alice", 30], [2, "Bob", 25]]),
    );

    // stdin `-` 导入
    let out = run_cli_stdin(
        dir.path(),
        &["import", "imp", "people", "-", "--csv"],
        "id,name,age\n3,Carol,22\n",
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":1}"#);

    // 目标表缺失：按 CSV 表头自动建表（首列隐式主键；类型按字段猜测）
    let out = run_cli_stdin(
        dir.path(),
        &["import", "imp", "gamers", "-", "--csv"],
        "name,score\nDan,91.5\nEve,88\n",
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":2}"#);
    let out = run_cli(
        dir.path(),
        &["imp", "SELECT name, score FROM gamers ORDER BY name"],
    );
    assert_rows(
        &out,
        serde_json::json!(["name", "score"]),
        serde_json::json!([["Dan", 91.5], ["Eve", 88.0]]),
    );

    // `:memory:` import 豁免面：内存库导入可达
    let out = run_cli_stdin(
        dir.path(),
        &["import", ":memory:", "m", "-", "--csv"],
        "a,b\n1,x\n2,y\n",
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":2}"#);

    // 表头冲突：既有表与 CSV 表头不匹配 → exit 1 点名拒绝零副作用
    let out = run_cli_stdin(
        dir.path(),
        &["import", "imp", "people", "-", "--csv"],
        "id,fullname\n9,Zed\n",
    );
    assert_eq!(out.code, Some(1), "stdout: {:?}", out.stdout);
    let out = run_cli(dir.path(), &["imp", "SELECT COUNT(*) AS c FROM people"]);
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[3]]));
}

/// E8 大值：~1MB 字符串写读 + dump/restore 往返 + check 干净
/// （超页溢出通道；大 SQL 经 restore stdin 承载，绕开 MAX_ARG_STRLEN）。
#[test]
fn e8_large_value_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "big"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 大 SQL（含建表）经 restore stdin 承载，绕开 MAX_ARG_STRLEN；
    // restore 要求空库前置 → 建表语句一并由 SQL 文本携带。
    let payload = "x".repeat(1_000_000);
    let sql = format!(
        "CREATE TABLE docs (id INT, body STRING); INSERT INTO docs VALUES (1, '{}');",
        payload
    );
    let out = run_cli_stdin(dir.path(), &["restore", "big", "-"], &sql);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // 读回：长度与内容逐字节一致
    let out = run_cli(dir.path(), &["big", "SELECT id, body FROM docs"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert_eq!(parsed["columns"], serde_json::json!(["id", "body"]));
    let body = parsed["rows"][0][1].as_str().expect("body is a string");
    assert_eq!(body.len(), 1_000_000, "round-tripped length");
    assert!(body.chars().all(|c| c == 'x'), "round-tripped content");

    // UPDATE 增长通道：REPLACE 使溢出链增长一倍（1MB → 2MB），内容逐字节正确
    let out = run_cli(
        dir.path(),
        &[
            "big",
            "UPDATE docs SET body = REPLACE(body, 'x', 'xy') WHERE id = 1",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":1}"#);
    let out = run_cli(dir.path(), &["big", "SELECT LENGTH(body) FROM docs"]);
    assert_rows(
        &out,
        serde_json::json!(["LENGTH(body)"]),
        serde_json::json!([[2000000]]),
    );

    // dump → restore 往返逐字节一致
    let dump = run_cli(dir.path(), &["dump", "big"]);
    assert_eq!(dump.code, Some(0), "stderr: {}", dump.stderr);
    let out = run_cli(dir.path(), &["new", "big2"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli_stdin(dir.path(), &["restore", "big2", "-"], &dump.stdout);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let left = run_cli(dir.path(), &["big", "SELECT body FROM docs"]);
    let right = run_cli(dir.path(), &["big2", "SELECT body FROM docs"]);
    assert_eq!(left.stdout, right.stdout, "dump→restore 大值往返");

    // check：含溢出页库干净单行
    let out = run_cli(dir.path(), &["check", "big"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert_eq!(
        out.stdout.lines().count(),
        1,
        "clean check one line: {:?}",
        out.stdout
    );

    // 大值 × 四格式矩阵（ISS15 组合空格）：csv/tsv 全值保真、table 64 字符截断 exit 0
    let out = run_cli(
        dir.path(),
        &["big", "SELECT body FROM docs", "--format", "csv"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let data_line = out.stdout.lines().nth(1).expect("csv data row");
    assert_eq!(data_line.len(), 2_000_000, "csv full fidelity");

    let out = run_cli(
        dir.path(),
        &["big", "SELECT body FROM docs", "--format", "tsv"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let data_line = out.stdout.lines().nth(1).expect("tsv data row");
    assert_eq!(data_line.len(), 2_000_000, "tsv full fidelity");

    let out = run_cli(
        dir.path(),
        &["big", "SELECT body FROM docs", "--format", "table"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "table header/separator/data: {:?}",
        out.stdout
    );
    let cell = lines[2].trim_end();
    assert_eq!(cell.chars().count(), 64, "table cell truncated");
    assert!(cell.ends_with("..."));
}

/// E9 分析子命令：stats/sample/profile × 四格式渲染。
#[test]
fn e9_analytics_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "ana"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "ana",
            "CREATE TABLE m (id INT PRIMARY KEY, g STRING, v INT)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let mut values = String::new();
    for i in 1..=20 {
        let g = if i % 2 == 0 { "even" } else { "odd" };
        values.push_str(&format!(
            "INSERT INTO m VALUES ({}, '{}', {});",
            i,
            g,
            i * 10
        ));
    }
    let out = run_cli(dir.path(), &["ana", &values]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // stats × 四格式
    for fmt in ["json", "table", "csv", "tsv"] {
        let out = run_cli(dir.path(), &["stats", "ana", "m", "--format", fmt]);
        assert_eq!(out.code, Some(0), "fmt {} stderr: {}", fmt, out.stderr);
        assert!(
            out.stdout.contains("column"),
            "stats header fmt {}: {}",
            fmt,
            out.stdout
        );
        assert!(
            out.stdout.contains("v"),
            "stats column v fmt {}: {}",
            fmt,
            out.stdout
        );
    }
    let out = run_cli(dir.path(), &["stats", "ana", "m"]);
    let parsed = json(&out.stdout);
    assert_eq!(
        parsed["columns"],
        serde_json::json!([
            "column",
            "type",
            "row_count",
            "null_rate",
            "distinct",
            "min",
            "max",
            "p50",
            "p90",
            "p99"
        ])
    );

    // sample：N ≤ 行数全行；JSON 行数正确
    let out = run_cli(dir.path(), &["sample", "ana", "m", "5", "--format", "json"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 5);

    // profile：String 列 top-k（频次降序）
    let out = run_cli(dir.path(), &["profile", "ana", "m", "--format", "json"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("g"),
        "profile string column: {}",
        out.stdout
    );

    // 大值 × 分析子命令（ISS15 用户实测 panic 路径）：70,000 字符 > 65,536 阈值
    let out = run_cli(
        dir.path(),
        &[
            "ana",
            "CREATE TABLE big_stats (id INT PRIMARY KEY, s STRING)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let big: String = "y".repeat(70_000);
    let out = run_cli(
        dir.path(),
        &[
            "ana",
            &format!("INSERT INTO big_stats VALUES (1, '{}')", big),
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // stats：table 截断 exit 0（String 列 min/max 携带原值 = panic 触发路径）
    let out = run_cli(
        dir.path(),
        &["stats", "ana", "big_stats", "--format", "table"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // stats json：min/max 全值保真（index 5/6 = min/max）
    let out = run_cli(
        dir.path(),
        &["stats", "ana", "big_stats", "--format", "json"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    let s_row = parsed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[0] == "s")
        .expect("s column row");
    assert_eq!(
        s_row[5].as_str().map(|v| v.len()),
        Some(70_000),
        "stats min full value"
    );
    assert_eq!(
        s_row[6].as_str().map(|v| v.len()),
        Some(70_000),
        "stats max full value"
    );

    // sample / profile：table 截断 exit 0
    let out = run_cli(
        dir.path(),
        &["sample", "ana", "big_stats", "1", "--format", "table"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["profile", "ana", "big_stats", "--format", "table"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
}

/// E10 输出面锁定（D-1）：多语句 JSONL、DDL affected_rows:0、四格式渲染形态。
#[test]
fn e10_output_surface_lockdown() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "o"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 多语句 `;` 分片：逐语句 JSONL（每行一个 JSON 对象）
    let out = run_cli(
        dir.path(),
        &[
            "o",
            "CREATE TABLE t (id INT); INSERT INTO t VALUES (1); INSERT INTO t VALUES (2); SELECT * FROM t",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let lines: Vec<&str> = out.stdout.trim().lines().collect();
    assert_eq!(lines.len(), 4, "multi-statement JSONL: {:?}", out.stdout);
    assert_eq!(
        lines[0].trim(),
        r#"{"affected_rows":0}"#,
        "DDL affected_rows:0 (D-1)"
    );
    assert_eq!(lines[1].trim(), r#"{"affected_rows":1}"#);
    assert_eq!(lines[2].trim(), r#"{"affected_rows":1}"#);
    let parsed: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(parsed["rows"], serde_json::json!([[1], [2]]));

    // DDL affected_rows:0 单语句形态（D-1 锁定）
    let out = run_cli(dir.path(), &["o", "CREATE TABLE d2 (x INT)"]);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":0}"#);

    // table 形态：两列以上才有 -+- 分隔行
    let out = run_cli(
        dir.path(),
        &[
            "o",
            "SELECT id, id + 1 AS next FROM t ORDER BY id",
            "--format",
            "table",
        ],
    );
    assert!(out.stdout.contains("id"), "table header: {}", out.stdout);
    assert!(
        out.stdout.contains("-+-"),
        "table separator: {}",
        out.stdout
    );
    let out = run_cli(
        dir.path(),
        &["o", "SELECT id FROM t ORDER BY id", "--format", "csv"],
    );
    assert_eq!(out.stdout.trim(), "id\n1\n2", "csv shape: {:?}", out.stdout);
    let out = run_cli(
        dir.path(),
        &["o", "SELECT id FROM t ORDER BY id", "--format", "tsv"],
    );
    assert_eq!(out.stdout.trim(), "id\n1\n2", "tsv shape: {:?}", out.stdout);
    let out = run_cli(
        dir.path(),
        &["o", "SELECT id FROM t ORDER BY id", "--format", "json"],
    );
    let parsed = json(&out.stdout);
    assert_eq!(parsed["columns"], serde_json::json!(["id"]));
    assert_eq!(parsed["rows"], serde_json::json!([[1], [2]]));
}

/// 带环境变量覆盖的 one-shot 运行（RTSQL_KEY 通道等；未覆盖的键保持夹具默认）。
fn run_cli_env(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> CliOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if !envs.iter().any(|(k, _)| k == &"RTSQL_KEY") {
        cmd.env_remove("RTSQL_KEY");
    }
    wait_cli(cmd.spawn().expect("spawn rtsql binary"))
}

/// 后台持锁者：独立运行时打开库、持有 `hold` 时长后关闭释放
/// （busy_timeout_test「待命写会话」先例；flock 跨进程生效）。
fn hold_lock(path: &Path, hold: Duration) -> std::thread::JoinHandle<()> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(rtsql::database::Database::open(&path)).unwrap();
        std::thread::sleep(hold);
        rt.block_on(db.close()).unwrap();
    })
}

/// E5 并发锁：持锁窗口内 等待-成功 / 超时 exit 4 / 只读冲突 exit 4 /
/// delete 占用拒绝 exit 4；释放后库可用。
#[test]
fn e5_concurrency_lock_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "locked"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["locked", "CREATE TABLE t (id INT)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let db_path = dir.path().join("db").join("locked.db");

    // 预算内等待 → 持锁者释放后成功
    let holder = hold_lock(&db_path, Duration::from_millis(900));
    let start = Instant::now();
    let out = run_cli(
        dir.path(),
        &[
            "locked",
            "SELECT COUNT(*) AS c FROM t",
            "--busy-timeout",
            "5000",
        ],
    );
    let elapsed = start.elapsed();
    holder.join().unwrap();
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        elapsed >= Duration::from_millis(800),
        "waiter should have blocked until release (elapsed: {elapsed:?})"
    );

    // 预算耗尽 → exit 4 点名锁错误
    let holder = hold_lock(&db_path, Duration::from_millis(900));
    let out = run_cli(dir.path(), &["locked", "SELECT 1", "--busy-timeout", "200"]);
    holder.join().unwrap();
    assert_eq!(
        out.code,
        Some(4),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("database is locked"),
        "stderr should name the lock failure: {:?}",
        out.stderr
    );

    // 只读共享锁与写独占锁互斥 → exit 4
    let holder = hold_lock(&db_path, Duration::from_millis(900));
    let out = run_cli(
        dir.path(),
        &["--readonly", "--busy-timeout", "200", "locked", "SELECT 1"],
    );
    holder.join().unwrap();
    assert_eq!(
        out.code,
        Some(4),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // delete 探测永不等待 → 占用即 exit 4
    let holder = hold_lock(&db_path, Duration::from_millis(900));
    let out = run_cli(dir.path(), &["delete", "locked"]);
    holder.join().unwrap();
    assert_eq!(
        out.code,
        Some(4),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(db_path.exists(), "rejected delete must not remove files");

    // 释放后库可用
    let out = run_cli(dir.path(), &["locked", "SELECT 1"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
}

/// E6 跨库：ATTACH → 跨查 → 跨写 → CTAS 主库物化 → 两库独立可用。
#[test]
fn e6_cross_db_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "maindb"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["new", "auxdb"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "auxdb",
            "CREATE TABLE items (id INT PRIMARY KEY, label STRING); INSERT INTO items VALUES (1, 'one'), (2, 'two')",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let aux_path = dir.path().join("db").join("auxdb.db");
    let aux_path_str = aux_path.to_str().unwrap().to_string();

    // ATTACH + 跨库只读查询（多语句 JSONL，取结果语句行）
    let sql = format!(
        "ATTACH '{}' AS a; SELECT id, label FROM a.items ORDER BY id",
        aux_path_str
    );
    let out = run_cli(dir.path(), &["maindb", &sql]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = last_json(&out.stdout);
    assert_eq!(parsed["columns"], serde_json::json!(["id", "label"]));
    assert_eq!(parsed["rows"], serde_json::json!([[1, "one"], [2, "two"]]));

    // 跨库写入（附加库目标，独立提交）
    let sql = format!(
        "ATTACH '{}' AS a; INSERT INTO a.items VALUES (3, 'three')",
        aux_path_str
    );
    let out = run_cli(dir.path(), &["maindb", &sql]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(
        last_json(&out.stdout),
        serde_json::json!({"affected_rows": 1})
    );

    // CTAS 主库物化（快照落主库）
    let sql = format!(
        "ATTACH '{}' AS a; CREATE TABLE snap AS SELECT * FROM a.items",
        aux_path_str
    );
    let out = run_cli(dir.path(), &["maindb", &sql]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["maindb", "SELECT COUNT(*) AS c FROM snap"]);
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[3]]));

    // 两库独立可用：跨写对附加库直接打开可见；主库快照可见
    let out = run_cli(dir.path(), &["auxdb", "SELECT COUNT(*) AS c FROM items"]);
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[3]]));
    let out = run_cli(
        dir.path(),
        &["maindb", "SELECT label FROM snap ORDER BY id"],
    );
    assert_rows(
        &out,
        serde_json::json!(["label"]),
        serde_json::json!([["one"], ["two"], ["three"]]),
    );
}

/// E7 加密：`--key` 建库全流程 + 错钥 exit 5 + check + dump/restore 带钥 +
/// 落盘密文不泄明文 + `RTSQL_KEY` env 通道等效。
#[test]
fn e7_encryption_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "--key", "pw", "enc"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let out = run_cli(
        dir.path(),
        &[
            "--key",
            "pw",
            "enc",
            "CREATE TABLE s (id INT PRIMARY KEY, secret STRING)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "--key",
            "pw",
            "enc",
            "INSERT INTO s VALUES (1, 'topsecret')",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["--key", "pw", "enc", "SELECT secret FROM s"]);
    assert_rows(
        &out,
        serde_json::json!(["secret"]),
        serde_json::json!([["topsecret"]]),
    );

    // 落盘字节不携带明文
    let raw = std::fs::read(dir.path().join("db").join("enc.db")).unwrap();
    assert!(
        !raw.windows(b"topsecret".len()).any(|w| w == b"topsecret"),
        "encrypted pages must not leak plaintext"
    );

    // 错钥 exit 5
    let out = run_cli(
        dir.path(),
        &["--key", "wrong", "enc", "SELECT secret FROM s"],
    );
    assert_eq!(
        out.code,
        Some(5),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // check 带钥干净单行
    let out = run_cli(dir.path(), &["--key", "pw", "check", "enc"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert_eq!(
        out.stdout.lines().count(),
        1,
        "clean check one line: {:?}",
        out.stdout
    );

    // dump/restore 带钥往返
    let dump = run_cli(dir.path(), &["--key", "pw", "dump", "enc"]);
    assert_eq!(dump.code, Some(0), "stderr: {}", dump.stderr);
    let out = run_cli(dir.path(), &["new", "--key", "pw", "enc2"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli_stdin(
        dir.path(),
        &["--key", "pw", "restore", "enc2", "-"],
        &dump.stdout,
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["--key", "pw", "enc2", "SELECT secret FROM s"]);
    assert_rows(
        &out,
        serde_json::json!(["secret"]),
        serde_json::json!([["topsecret"]]),
    );

    // RTSQL_KEY env 通道等效（flag 优先矩阵归 busy_timeout_test 语料）
    let out = run_cli_env(
        dir.path(),
        &[("RTSQL_KEY", "pw")],
        &["enc", "INSERT INTO s VALUES (2, 'envkey')"],
    );
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let out = run_cli_env(
        dir.path(),
        &[("RTSQL_KEY", "pw")],
        &["enc", "SELECT COUNT(*) AS c FROM s"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[2]]));

    // import × 加密库（组合空格，此前零见证）：对钥成功数据可查、错钥 exit 5
    std::fs::write(dir.path().join("f.csv"), "id,secret\n7,a1\n8,a2\n").unwrap();
    let out = run_cli(
        dir.path(),
        &[
            "--key",
            "pw",
            "enc",
            "CREATE TABLE s2 (id INT PRIMARY KEY, secret STRING)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["--key", "pw", "import", "enc", "s2", "f.csv", "--csv"],
    );
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let out = run_cli(
        dir.path(),
        &["--key", "pw", "enc", "SELECT COUNT(*) AS c FROM s2"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[2]]));

    // 错钥 import exit 5（密钥错误打开面与主命令一致）
    let out = run_cli(
        dir.path(),
        &["--key", "wrong", "import", "enc", "s2", "f.csv", "--csv"],
    );
    assert_eq!(
        out.code,
        Some(5),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // stats × 加密库（此前零见证）
    let out = run_cli(
        dir.path(),
        &["--key", "pw", "stats", "enc", "s", "--format", "json"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed = json(&out.stdout);
    assert!(
        parsed["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[0] == "secret"),
        "stats encrypted db rows: {}",
        out.stdout
    );
}

/// E3 崩溃恢复对照（非缺陷面）：已存在表 + 写入中途 SIGKILL → 读写重开
/// 已提交数据完好 → 重开幂等 → check 干净 → 收敛后 checkpoint 零工作。
#[test]
fn e3_crash_recovery_control() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "crash"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["crash", "CREATE TABLE t (id INT PRIMARY KEY, n INT)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["crash", "INSERT INTO t VALUES (1, 1), (2, 2), (3, 3)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 写入中途 SIGKILL：多语句 INSERT 序列（debug 逐条 auto-commit），
    // 600ms 后 kill——首条必已提交，千条量级在 600ms 内必未完成。
    let mut sql = String::new();
    for i in 4..1004 {
        sql.push_str(&format!("INSERT INTO t VALUES ({}, {});", i, i));
    }
    let mut child = spawn_cli(dir.path(), &["crash", &sql]);
    std::thread::sleep(Duration::from_millis(600));
    child.kill().expect("SIGKILL writer");
    let killed = wait_cli(child);
    assert!(
        killed.code.is_none(),
        "SIGKILL terminates by signal: {:?}",
        killed.code
    );

    // 读写重开：恢复可达、已提交行完好（数量在已证区间内、行唯一）
    let out = run_cli(dir.path(), &["crash", "SELECT COUNT(*) AS c FROM t"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let recovered = json(&out.stdout)["rows"][0][0].as_i64().unwrap();
    assert!(
        (4..=1003).contains(&recovered),
        "recovered rows must be within the committed window: {recovered}"
    );
    let out = run_cli(
        dir.path(),
        &["crash", "SELECT COUNT(DISTINCT id) AS c FROM t"],
    );
    assert_eq!(
        json(&out.stdout)["rows"][0][0].as_i64().unwrap(),
        recovered,
        "recovered rows must be distinct (no dup/loss)"
    );

    // 重开幂等：再开一次 count 不变
    let out = run_cli(dir.path(), &["crash", "SELECT COUNT(*) AS c FROM t"]);
    assert_eq!(
        json(&out.stdout)["rows"][0][0].as_i64().unwrap(),
        recovered,
        "reopen must be idempotent"
    );

    // check 干净单行
    let out = run_cli(dir.path(), &["check", "crash"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert_eq!(
        out.stdout.lines().count(),
        1,
        "clean check one line: {:?}",
        out.stdout
    );

    // 恢复会话收敛后 checkpoint 零工作
    let out = run_cli(dir.path(), &["checkpoint", "crash"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("no pending work"),
        "converged: {}",
        out.stdout
    );
}

/// E3b 自动建表即崩溃（ISS06，pre-fix 整库砖化）。
/// post-fix 期望：读写重开恢复可达（自动建的表 + 已提交行可恢复）、check 干净。
#[test]
fn e3b_auto_create_crash_recovery() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "ib"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // import 自动建表：EOF 后开始逐行提交；2s 后 SIGKILL（DDL 未落盘窗口）
    let mut csv = String::from("id,name\n");
    for i in 1..=10000 {
        csv.push_str(&format!("{},row{}\n", i, i));
    }
    let mut child = spawn_cli_stdin(dir.path(), &["import", "ib", "autot", "-", "--csv"]);
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(csv.as_bytes()).unwrap();
    drop(stdin); // EOF → import 开始提交
    std::thread::sleep(Duration::from_millis(2000));
    child.kill().expect("SIGKILL import");
    let killed = wait_cli(child);
    assert!(
        killed.code.is_none(),
        "SIGKILL terminates by signal: {:?}",
        killed.code
    );

    // post-fix：读写重开恢复可达，已提交行可查询（≥1 行committed）
    let out = run_cli(dir.path(), &["ib", "SELECT COUNT(*) AS c FROM autot"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let recovered = json(&out.stdout)["rows"][0][0].as_i64().unwrap();
    assert!(
        recovered >= 1,
        "at least one committed row recoverable: {recovered}"
    );

    // check 干净单行
    let out = run_cli(dir.path(), &["check", "ib"]);
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert_eq!(
        out.stdout.lines().count(),
        1,
        "clean check one line: {:?}",
        out.stdout
    );
}

/// E4 索引旅程（绿面）：≥30000 行重复键表 + CREATE INDEX → 建索引前
/// DataScan 基线 → 全量 COUNT → DROP INDEX 回落 DataScan 基线恢复。
#[test]
fn e4_index_journey() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "idxdb"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 播种：30000 行（n ∈ 1..100，每值恰 300 行）经 restore stdin 单语句
    let mut values = String::new();
    for i in 1..=30000 {
        values.push_str(&format!("({}, {}),", i, i % 100 + 1));
    }
    values.pop();
    let sql = format!(
        "CREATE TABLE t (id INT PRIMARY KEY, n INT); INSERT INTO t VALUES {};",
        values
    );
    let out = run_cli_stdin(dir.path(), &["restore", "idxdb", "-"], &sql);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 建索引前基线（DataScan）：n=55 恰 300 行
    let out = run_cli(
        dir.path(),
        &["idxdb", "SELECT COUNT(*) AS c FROM t WHERE n = 55"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[300]]));

    // CREATE INDEX（存量回填，catalog 登记根重开保持）
    let out = run_cli(dir.path(), &["idxdb", "CREATE INDEX idx_n ON t (n)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 全量 COUNT（不涉等值路由面）
    let out = run_cli(dir.path(), &["idxdb", "SELECT COUNT(*) AS c FROM t"]);
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[30000]]));

    // DROP INDEX → 删除页回落扫描 → 基线恢复
    let out = run_cli(dir.path(), &["idxdb", "DROP INDEX idx_n"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["idxdb", "SELECT COUNT(*) AS c FROM t WHERE n = 55"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[300]]));
}

/// E4 索引正确性面（ISS05/B1/B2）：二级索引等值计数与 DataScan 基线
/// 双向严格相等（不重复、不遗漏）+ 无 WHERE 全表 UPDATE affected=行数。
#[test]
fn e4_index_correctness_surface() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "idxc"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 15000 行（n ∈ 1..100，每值恰 150 行）——已达等值跨边界散布规模
    let mut values = String::new();
    for i in 1..=15000 {
        values.push_str(&format!("({}, {}),", i, i % 100 + 1));
    }
    values.pop();
    let sql = format!(
        "CREATE TABLE t (id INT PRIMARY KEY, n INT); INSERT INTO t VALUES {};",
        values
    );
    let out = run_cli_stdin(dir.path(), &["restore", "idxc", "-"], &sql);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 基线（DataScan）
    let out = run_cli(
        dir.path(),
        &["idxc", "SELECT COUNT(*) AS c FROM t WHERE n = 55"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[150]]));

    let out = run_cli(dir.path(), &["idxc", "CREATE INDEX idx_n ON t (n)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 等值计数严格相等（重开后经 catalog 根索引面）：COUNT(*) 无重复虚发
    let out = run_cli(
        dir.path(),
        &["idxc", "SELECT COUNT(*) AS c FROM t WHERE n = 55"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[150]]));
    // COUNT(DISTINCT id) 无遗漏
    let out = run_cli(
        dir.path(),
        &["idxc", "SELECT COUNT(DISTINCT id) AS c FROM t WHERE n = 55"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[150]]));

    // 无 WHERE 全表 UPDATE：affected_rows = 实际行数，全部行触达
    let out = run_cli(dir.path(), &["idxc", "UPDATE t SET n = n"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), r#"{"affected_rows":15000}"#);
    let out = run_cli(
        dir.path(),
        &["idxc", "SELECT COUNT(*) AS c FROM t WHERE n = 55"],
    );
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[150]]));
}

/// people/dept 种子现场（E11 旅程共用形态）。
fn seed_people_dept(dir: &Path) {
    let out = run_cli(dir, &["new", "hr"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir,
        &[
            "hr",
            "CREATE TABLE people (id INT PRIMARY KEY, name STRING, age INT); CREATE TABLE dept (did INT PRIMARY KEY, name STRING); INSERT INTO people VALUES (1, 'a', 30), (2, 'b', 25), (3, 'c', 35); INSERT INTO dept VALUES (1, 'eng'), (2, 'ops')",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
}

/// E11 派生/视图读面（绿对照面）：CTE 通道内层别名外层可引用、IN 子查询、
/// 普通视图/派生表读面正常。
#[test]
fn e11_derived_view_read_surface() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // CTE 通道：内层别名外层可引用（绿——三通道一致的参照面）
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "WITH x AS (SELECT age AS v FROM people) SELECT v FROM x ORDER BY v",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["v"]),
        serde_json::json!([[25], [30], [35]]),
    );

    // 普通派生表（内层普通列）绿
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT t.name FROM (SELECT name, age FROM people) t WHERE t.age > 26 ORDER BY t.name",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["name"]),
        serde_json::json!([["a"], ["c"]]),
    );

    // 普通视图（普通列定义）绿
    let out = run_cli(
        dir.path(),
        &["hr", "CREATE VIEW v_people AS SELECT id, name FROM people"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["hr", "SELECT name FROM v_people WHERE id = 2"],
    );
    assert_rows(
        &out,
        serde_json::json!(["name"]),
        serde_json::json!([["b"]]),
    );
}

/// E11 内层输出列外层可见（ISS07）：派生表/视图通道的内层
/// 别名列与表达式列外层可引用（CTE 通道绿对照），内层 DISTINCT 同族放行。
#[test]
fn e11_inner_columns_outer_visible() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // 派生表：内层别名列
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT v FROM (SELECT age AS v FROM people) t ORDER BY v",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["v"]),
        serde_json::json!([[25], [30], [35]]),
    );

    // 派生表：内层表达式列
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT tier FROM (SELECT CASE WHEN age >= 30 THEN 'senior' ELSE 'junior' END AS tier FROM people) t ORDER BY tier",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["tier"]),
        serde_json::json!([["junior"], ["senior"], ["senior"]]),
    );

    // 视图：内层别名列
    let out = run_cli(
        dir.path(),
        &["hr", "CREATE VIEW pv AS SELECT age AS v FROM people"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["hr", "SELECT v FROM pv ORDER BY v"]);
    assert_rows(
        &out,
        serde_json::json!(["v"]),
        serde_json::json!([[25], [30], [35]]),
    );

    // 派生表：内层 DISTINCT 别名列（同族放行）
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT v FROM (SELECT DISTINCT age AS v FROM people) t ORDER BY v",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["v"]),
        serde_json::json!([[25], [30], [35]]),
    );
}

/// E11 SemiJoin 排序键（ISS09）：ORDER BY 非输出列在
/// IN 子查询基座上按语义排序可用。
#[test]
fn e11_semijoin_order_by_non_output_column() {
    let dir = fixture();
    seed_people_dept(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT name FROM people WHERE id IN (SELECT did FROM dept) ORDER BY id",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["name"]),
        serde_json::json!([["a"], ["b"]]),
    );

    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT name FROM people WHERE id IN (SELECT did FROM dept) ORDER BY id DESC",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["name"]),
        serde_json::json!([["b"], ["a"]]),
    );
}

/// E12 文案面（绿面）：错误面 SQLSTATE 分类与零副作用（未知列/表/不支持语句）。
#[test]
fn e12_error_surface() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // 未知列：exit 3 + 42703 + 表无副作用
    let out = run_cli(dir.path(), &["hr", "SELECT nosuch FROM people"]);
    assert_eq!(out.code, Some(3));
    assert!(
        out.stderr.contains("42703") && out.stderr.contains("Column not found"),
        "stderr: {:?}",
        out.stderr
    );

    // 未知表：exit 3 + 42P01
    let out = run_cli(dir.path(), &["hr", "SELECT * FROM nosuch"]);
    assert_eq!(out.code, Some(3));
    assert!(
        out.stderr.contains("42P01") && out.stderr.contains("nosuch"),
        "stderr: {:?}",
        out.stderr
    );

    // 不支持语句：exit 3 + 0A000 点名能力
    let out = run_cli(
        dir.path(),
        &["hr", "SELECT ROW_NUMBER() OVER () FROM people"],
    );
    assert_eq!(out.code, Some(3));
    assert!(
        out.stderr.contains("0A000") && out.stderr.contains("ROW_NUMBER"),
        "stderr: {:?}",
        out.stderr
    );

    // 零副作用：people 三行完好
    let out = run_cli(dir.path(), &["hr", "SELECT COUNT(*) AS c FROM people"]);
    assert_rows(&out, serde_json::json!(["c"]), serde_json::json!([[3]]));
}

/// E12 --help 用户视角（ISS10，pre-fix RED）：无内部任务编号/提案标记。
#[test]
fn e12_help_without_internal_markers() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["--help"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    for marker in ["MS28", "MS26", "proposal", "ISS", "T1：", "Iteration"] {
        assert!(
            !out.stdout.contains(marker),
            "--help must not leak internal marker {marker:?}: {}",
            out.stdout
        );
    }
}

/// E12 错误前缀单层（ISS11，pre-fix RED）：Parse error 不再双层嵌套。
#[test]
fn e12_error_prefix_single_layer() {
    let dir = fixture();
    seed_people_dept(dir.path());
    let out = run_cli(dir.path(), &["hr", "SELEC id FROM people"]);
    assert_eq!(out.code, Some(3));
    let parsed = json(&out.stderr);
    let msg = parsed["error"].as_str().unwrap_or("");
    assert!(
        !msg.contains("Parse error: Parse error:"),
        "double prefix: {msg}"
    );
    assert!(msg.contains("Parse error"), "single prefix kept: {msg}");
}

/// E12 dump 类型名标准长名（ISS12，pre-fix RED）：INTEGER/TEXT 等标准名
/// 渲染（dump→restore 往返保持）。
#[test]
fn e12_dump_standard_type_names() {
    let dir = fixture();
    seed_people_dept(dir.path());
    let out = run_cli(dir.path(), &["dump", "hr"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("INTEGER") && out.stdout.contains("TEXT"),
        "standard long type names: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains(" INT ") && !out.stdout.contains(" STRING"),
        "short type names retired: {}",
        out.stdout
    );
}

/// E12 busy-timeout env 报错点名（ISS13，pre-fix RED）：非法
/// RTSQL_BUSY_TIMEOUT 报错点名环境变量（flag 优先级与退出码不变）。
#[test]
fn e12_busy_timeout_env_named() {
    let dir = fixture();
    seed_people_dept(dir.path());
    let out = run_cli_env(
        dir.path(),
        &[("RTSQL_BUSY_TIMEOUT", "abc")],
        &["hr", "SELECT 1"],
    );
    assert_eq!(
        out.code,
        Some(2),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    // clap 用法错误为纯文本 stderr（非 JSON 载荷）：原始文本点名环境变量。
    assert!(
        out.stderr.contains("RTSQL_BUSY_TIMEOUT"),
        "error must name the env var: {}",
        out.stderr
    );
}

/// E13 DEFAULT 超限点名拒绝（ISS14，pre-fix RED）：DEFAULT 字面量超出
/// catalog 页内容量 → 计划期点名拒绝（DEFAULT 语义）、零副作用。
#[test]
fn e13_oversized_default_named_rejection() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "e13"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let big_default = "d".repeat(5000);
    let sql = format!(
        "CREATE TABLE t (id INT, pad STRING DEFAULT '{}')",
        big_default
    );
    let out = run_cli(dir.path(), &["e13", &sql]);
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    let parsed = json(&out.stderr);
    let error = parsed["error"].as_str().unwrap_or("");
    // 语义段（statement 回声之前的消息本体）必须点名 DEFAULT——
    // 防止「语句回显含 DEFAULT 字样」造成假阳性。
    let message = error.split("; statement:").next().unwrap_or(error);
    assert!(
        message.to_uppercase().contains("DEFAULT"),
        "error message must name the DEFAULT constraint: {message}"
    );
}

/// E13 建表失败零残留（ISS08，pre-fix RED）：失败 DDL 无半表残留、
/// 同名可重试。
#[test]
fn e13_create_failure_zero_residual() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "e13r"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let big_default = "d".repeat(5000);
    let sql = format!(
        "CREATE TABLE t (id INT, pad STRING DEFAULT '{}')",
        big_default
    );
    let out = run_cli(dir.path(), &["e13r", &sql]);
    assert!(
        out.code != Some(0),
        "oversized CREATE must fail: {:?}",
        out.stdout
    );

    // schema 无半表残留
    let out = run_cli(dir.path(), &["schema", "e13r"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        !out.stdout.contains("\"t\""),
        "no residual half-table: {}",
        out.stdout
    );

    // INSERT 拒绝（表不存在）
    let out = run_cli(dir.path(), &["e13r", "INSERT INTO t VALUES (1, 'x')"]);
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // 同名重建成功
    let out = run_cli(dir.path(), &["e13r", "CREATE TABLE t (id INT)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["e13r", "INSERT INTO t VALUES (1)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
}

/// E14 子查询别名限定符绑定（ISS16/ISS17/ISS18 + 同类面审计新增形状）——真实
/// CLI 子进程见证，change 2026-10-15-subquery-alias-binding-and-error-prefix T4。
///
/// 修复前（pre-fix RED）：内层 FROM 项的别名被丢弃，内层别名限定引用被误判为
/// 相关外层引用，各形状退化为静默空集 / 按位截断 / 全放行。修复后各形状按
/// 真实语义出结果，每个形状另带无别名对照。
#[test]
fn e14_subquery_alias_binding_where_scalar() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // ISS16：WHERE 相关标量比较——内层别名限定引用按外层每行收窄。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT p.id FROM people p WHERE (SELECT COUNT(*) FROM dept x WHERE x.did = p.id) = 1",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2]]),
    );

    // 无别名对照：同语义。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT p.id FROM people p WHERE (SELECT COUNT(*) FROM dept d WHERE d.did = p.id) = 1",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2]]),
    );

    // 零计数分支：内层无匹配的外层行（people id=3）按 COUNT=0 保留。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT p.id FROM people p WHERE (SELECT COUNT(*) FROM dept x WHERE x.did = p.id) = 0",
        ],
    );
    assert_rows(&out, serde_json::json!(["id"]), serde_json::json!([[3]]));
}

/// ISS16 投影位形态：输出行数与外层一致，每行取本行匹配值（修复前塌缩或报多行错）。
#[test]
fn e14_subquery_alias_binding_select_list() {
    let dir = fixture();
    seed_people_dept(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT p.id, (SELECT x.name FROM dept x WHERE x.did = p.id) AS dn FROM people p",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id", "dn"]),
        serde_json::json!([[1, "eng"], [2, "ops"], [3, null]]),
    );
}

/// ISS17 EXISTS 内层别名四形态 + 审计新增 NOT EXISTS 内层别名形状。
#[test]
fn e14_subquery_alias_binding_exists() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // 形态一：谓词落在内层非首列（修复前恒空集）。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people WHERE EXISTS (SELECT 1 FROM dept x WHERE x.name = 'eng')",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2], [3]]),
    );

    // 形态二：谓词落在内层首列（修复前仅返回外层第一行）。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people WHERE EXISTS (SELECT 1 FROM dept x WHERE x.did = 1)",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2], [3]]),
    );

    // 形态三：相关经外层非主键列——只有 ref 命中的行放行（修复前全部放行）。
    let out = run_cli(
        dir.path(),
        &["hr", "CREATE TABLE link (id INT PRIMARY KEY, ref INT)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["hr", "INSERT INTO link VALUES (1, 9), (2, 1), (3, 7)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT o.id FROM link o WHERE EXISTS (SELECT 1 FROM dept x WHERE x.did = o.ref)",
        ],
    );
    assert_rows(&out, serde_json::json!(["id"]), serde_json::json!([[2]]));

    // 形态四：无别名对照。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people WHERE EXISTS (SELECT 1 FROM dept WHERE dept.name = 'eng')",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2], [3]]),
    );

    // 审计新增形状：NOT EXISTS 内层别名——内层无匹配时全部外层行保留。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people WHERE NOT EXISTS (SELECT 1 FROM dept x WHERE x.name = 'nobody')",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2], [3]]),
    );

    // 审计新增形状：NOT EXISTS 内层别名——内层有匹配时全部外层行被滤除。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people WHERE NOT EXISTS (SELECT 1 FROM dept x WHERE x.name = 'eng')",
        ],
    );
    assert_rows(&out, serde_json::json!(["id"]), serde_json::json!([]));

    // 审计新增形状：EXISTS 内层 JOIN 两臂都带别名（修复前恒空集）。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people WHERE EXISTS \
             (SELECT 1 FROM dept x JOIN people y ON x.did = y.id WHERE x.name = 'eng')",
        ],
    );
    assert_rows(
        &out,
        serde_json::json!(["id"]),
        serde_json::json!([[1], [2], [3]]),
    );
}

/// ISS18：IN 内层别名 ∧ 谓词由二级索引服务——恒空集缺陷的端到端见证。
#[test]
fn e14_subquery_alias_binding_in_with_index() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // CREATE INDEX 仅支持单列 INT —— 取独立内层表，索引列非主键整数列。
    let out = run_cli(
        dir.path(),
        &["hr", "CREATE TABLE c (id INT PRIMARY KEY, age INT)"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["hr", "INSERT INTO c VALUES (1, 41), (2, 30)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["hr", "CREATE INDEX idx_c_age ON c(age)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    // 内层带别名 ∧ 索引谓词（修复前恒空集）。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people p WHERE p.id IN (SELECT x.id FROM c x WHERE x.age = 41)",
        ],
    );
    assert_rows(&out, serde_json::json!(["id"]), serde_json::json!([[1]]));

    // 无别名对照：同形状同结果。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT id FROM people p WHERE p.id IN (SELECT c.id FROM c WHERE c.age = 41)",
        ],
    );
    assert_rows(&out, serde_json::json!(["id"]), serde_json::json!([[1]]));
}

/// ISS19（R3 端到端）：执行器路径错误输出恰一层 `Execution error:` 前缀。
#[test]
fn e14_execution_error_single_prefix() {
    let dir = fixture();
    seed_people_dept(dir.path());

    // 按非键列关联构造必然多行的标量子查询（`x.did >= 1` 命中内层两行）。
    let out = run_cli(
        dir.path(),
        &[
            "hr",
            "SELECT p.name, (SELECT x.name FROM dept x WHERE x.did >= 1) AS dn FROM people p",
        ],
    );
    assert_eq!(
        out.code,
        Some(3),
        "stdout: {} stderr: {}",
        out.stdout,
        out.stderr
    );
    let msg = json(&out.stderr)["error"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        !msg.contains("Execution error: execution error:"),
        "double prefix: {msg}"
    );
    assert!(
        msg.contains("Execution error:"),
        "single prefix kept: {msg}"
    );
    assert!(
        msg.contains("Subquery returns multiple rows"),
        "error body kept: {msg}"
    );
}
