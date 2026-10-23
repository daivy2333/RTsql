//! query-memory-budget 集成测试（MS32 Iteration 003）。
//!
//! 判定面只有成功/失败、错误消息、SQLSTATE 与目标表行数——不采样内存。

use rtsql::database::Database;
use rtsql::SqlOutcome;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// 计费口径常量（design D4）：非字符串值 16 字节。
const VALUE_BYTES: u64 = 16;

fn rows_of(outcome: SqlOutcome) -> Vec<Vec<rtsql::executor::Value>> {
    match outcome {
        SqlOutcome::Rows { rows, .. } => rows,
        SqlOutcome::AffectedRows(n) => panic!("期望行结果，得到 AffectedRows({n})"),
    }
}

/// 标准夹具：t(id, v) 四行——v 序 10/20/30/40 对应 id 2/4/3/1（非平凡排序）。
async fn setup_t(db: &Database) {
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 40), (2, 10), (3, 30), (4, 20)")
        .await
        .unwrap();
}

// ------------------------------------------------------------ CLI 子进程

struct CliOutput {
    code: Option<i32>,
    stderr: String,
}

/// 独立 CWD 运行 one-shot CLI 一次（沿 `wal_auto_checkpoint_test.rs` 的
/// `run_cli` 先例：stdout 丢弃、stderr 落管道；隔离三个字节数类 env 通道）。
fn run_cli(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> CliOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_WAL_CHECKPOINT_BYTES")
        .env_remove("RTSQL_QUERY_MEMORY_BYTES")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn rtsql");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr pipe")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    let status = child.wait().expect("wait rtsql");
    CliOutput {
        code: status.code(),
        stderr,
    }
}

// ------------------------------------------------------------ 场景

/// query-memory-budget R1「默认不限制」：未设置上限时建表/插入/SELECT/
/// JOIN/ORDER BY 全成功——锁定默认面（改前 GREEN）。
#[tokio::test]
async fn default_zero_limit_allows_existing_queries() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    for i in 0..50 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({i}, {i})"))
            .await
            .unwrap();
    }
    db.execute_sql("CREATE TABLE s (id INT PRIMARY KEY, ref INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO s VALUES (1, 1), (2, 2)")
        .await
        .unwrap();

    db.execute_sql("SELECT * FROM t").await.unwrap();
    db.execute_sql("SELECT t.id FROM t JOIN s ON t.id = s.ref")
        .await
        .unwrap();
    db.execute_sql("SELECT id FROM t ORDER BY v DESC")
        .await
        .unwrap();
}

/// query-memory-budget R2「非法配置在打开前拒绝」：非数字 / 负数 / 溢出
/// 三形态各自 exit 2，stderr 点名变量与非法值，零库文件（改前 RED——当前
/// env 被忽略，主命令对缺失库 exit 1）。
#[test]
fn invalid_env_value_rejected_before_open() {
    for raw in ["abc", "-1", "99999999999999999999"] {
        let dir = TempDir::new().unwrap();
        let out = run_cli(
            dir.path(),
            &["illegal.db", "SELECT 1"],
            &[("RTSQL_QUERY_MEMORY_BYTES", raw)],
        );
        assert_eq!(
            out.code,
            Some(2),
            "非法值 {raw:?} 必须 exit 2，stderr: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains("RTSQL_QUERY_MEMORY_BYTES"),
            "非法值 {raw:?} 的报错必须点名变量，stderr: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains(raw),
            "非法值 {raw:?} 的报错必须回显非法值，stderr: {}",
            out.stderr
        );
        // 零副作用：裸名解析路径与 CWD 直开路径都不得出现库文件。
        assert!(
            !dir.path().join("db").join("illegal.db").exists(),
            "非法值 {raw:?} 不得创建库文件"
        );
        assert!(
            !dir.path().join("illegal.db").exists(),
            "非法值 {raw:?} 不得在 CWD 创建库文件"
        );
    }
}

// ------------------------------------------------- R3/R5/R6 内存形状（4.2）

/// R3「直接全表扫描不因结果大小被拒绝」：上限 16 字节、多行整数表、无
/// WHERE 的 `SELECT *` 全部返回（单行在途与最终结果行不计费）。
#[tokio::test]
async fn direct_scan_succeeds_under_16_bytes() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(VALUE_BYTES);
    let rows = rows_of(db.execute_sql("SELECT * FROM t").await.unwrap());
    assert_eq!(rows.len(), 4, "直接扫描不得因结果大小被拒绝");
}

/// R3「派生表按源行计费」：上限小于子查询全部源行（4 行 × 32B = 128B）→
/// 失败（改前 RED——当前不失败）。
#[tokio::test]
async fn derived_table_fails_under_small_limit() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(100);
    let result = db.execute_sql("SELECT * FROM (SELECT * FROM t)").await;
    assert!(result.is_err(), "派生表源行驻留超限应整句失败: {result:?}");
}

/// R5「右表本身超限」：右表 4 行（每行 32B 行 + 16B 哈希键）在 64B 上限下
/// 构建失败（改前 RED）。
#[tokio::test]
async fn join_right_side_fails_under_small_limit() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    db.execute_sql("CREATE TABLE l (id INT PRIMARY KEY, pad INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO l VALUES (1, 0), (2, 0)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE r (id INT PRIMARY KEY, ref INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO r VALUES (101, 1), (102, 2), (103, 1), (104, 2)")
        .await
        .unwrap();
    db.set_query_memory_limit(64);
    let result = db
        .execute_sql("SELECT l.id, r.id FROM l JOIN r ON l.id = r.ref")
        .await;
    assert!(result.is_err(), "右表驻留超限应整句失败: {result:?}");
}

/// R5「小右表加大左表」：右表 1 行（48B 驻留）+ 左表 20 行逐行探测，64B
/// 上限下成功且匹配行与不设上限相同（消费 Iteration 001 左表不驻留）。
#[tokio::test]
async fn small_right_large_left_join_succeeds_under_small_limit() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    db.execute_sql("CREATE TABLE l (id INT PRIMARY KEY, pad INT)")
        .await
        .unwrap();
    for i in 1..=20 {
        db.execute_sql(&format!("INSERT INTO l VALUES ({i}, 0)"))
            .await
            .unwrap();
    }
    db.execute_sql("CREATE TABLE r (id INT PRIMARY KEY, ref INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO r VALUES (100, 3)")
        .await
        .unwrap();

    let sql = "SELECT l.id, r.id FROM l JOIN r ON l.id = r.ref";
    db.set_query_memory_limit(64);
    let limited = rows_of(db.execute_sql(sql).await.unwrap());
    assert_eq!(
        limited,
        vec![vec![
            rtsql::executor::Value::Int(3),
            rtsql::executor::Value::Int(100)
        ]]
    );
}

/// R4「排序超过上限」：无 LIMIT 排序驻留 4 行（128B）在 100B 上限下失败
/// （改前 RED）。
#[tokio::test]
async fn unbounded_sort_fails_under_small_limit() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(100);
    let result = db.execute_sql("SELECT id, v FROM t ORDER BY v").await;
    assert!(result.is_err(), "无界排序驻留超限应整句失败: {result:?}");
}

/// R6「有界排序能通过小上限」：`LIMIT 1` 只驻留 1 行（32B），64B 上限下
/// 成功且为排序后第一行（消费 Iteration 002 有界排序）。
#[tokio::test]
async fn bounded_sort_limit_succeeds_under_small_limit() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(64);
    let rows = rows_of(
        db.execute_sql("SELECT id, v FROM t ORDER BY v LIMIT 1")
            .await
            .unwrap(),
    );
    assert_eq!(
        rows,
        vec![vec![
            rtsql::executor::Value::Int(2),
            rtsql::executor::Value::Int(10)
        ]]
    );
}

/// R3/D5「DISTINCT 全量去重驻留」：4 个互异 id 各留输出行 + seen 副本
/// （4 × 16B × 2 = 128B）在 100B 上限下失败——`LIMIT 1` 不能让去重提前止
/// （改前 RED）。
#[tokio::test]
async fn distinct_limit_fails_under_small_limit() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(100);
    let result = db.execute_sql("SELECT DISTINCT id FROM t LIMIT 1").await;
    assert!(
        result.is_err(),
        "DISTINCT 去重驻留超限应整句失败: {result:?}"
    );
}

// ------------------------------------------------- R4 超限错误面（4.3）

/// 无界排序（4 行 × 32B = 128B）在 100B 上限下超限——三入口通用形状。
const OVER_LIMIT_SORT: &str = "SELECT id, v FROM t ORDER BY v";

fn assert_over_limit_face(result: std::result::Result<SqlOutcome, rtsql::SqlError>) {
    let err = result.expect_err("超限应整句失败");
    assert_eq!(
        err.message, "Execution error: query memory limit exceeded (limit 100 bytes)",
        "三入口超限消息 SHALL 固定"
    );
    assert_eq!(
        err.sqlstate.as_deref(),
        Some("53200"),
        "SQLSTATE SHALL 53200"
    );
}

/// R4「超限整句失败」：直接执行 / 显式事务内执行 / 预编译执行三入口同消
/// 息同 53200（改前 RED——消息已对、sqlstate 缺失）。
#[tokio::test]
async fn three_entries_report_over_limit_with_53200() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(100);

    // 1. 直接执行
    assert_over_limit_face(db.execute_sql(OVER_LIMIT_SORT).await);

    // 2. 显式事务内执行（失败语句不终止事务，收尾回滚）
    let tx = db.begin().await.unwrap();
    assert_over_limit_face(db.execute_in_tx(OVER_LIMIT_SORT, &tx).await);
    db.rollback(tx).await.unwrap();

    // 3. 预编译执行
    let stmt = db.prepare(OVER_LIMIT_SORT).await.unwrap();
    assert_over_limit_face(stmt.execute(&[]).await);
}

/// R3「下一条语句不继承计数」：超限语句的中途入账不残留——同上限下后续
/// 语句从 0 计；同上限连续两次小排序都成功。
#[tokio::test]
async fn next_statement_does_not_inherit_count() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.set_query_memory_limit(100);

    // 超限失败（失败时 used 已到 96——若残留，下一条 32B 语句会误超）
    assert!(db
        .execute_sql("SELECT * FROM (SELECT * FROM t)")
        .await
        .is_err());
    // 同上限下 32B 有界排序成功 = 计数未跨语句
    db.execute_sql("SELECT id, v FROM t ORDER BY v LIMIT 1")
        .await
        .unwrap();

    // R3 场景：上限足以容纳一次小排序，连续两次相同排序都成功
    db.set_query_memory_limit(64);
    db.execute_sql("SELECT id, v FROM t ORDER BY v LIMIT 1")
        .await
        .unwrap();
    db.execute_sql("SELECT id, v FROM t ORDER BY v LIMIT 1")
        .await
        .unwrap();
}

/// R4「INSERT SELECT 零写入」：源行（128B）在 100B 上限下物化失败，dst
/// 零行且错误面同三入口（sqlstate 部分改前 RED——String 通道丢失分类）。
#[tokio::test]
async fn insert_select_over_limit_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    setup_t(&db).await;
    db.execute_sql("CREATE TABLE dst (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    db.set_query_memory_limit(100);
    assert_over_limit_face(db.execute_sql("INSERT INTO dst SELECT * FROM t").await);
    let rows = rows_of(db.execute_sql("SELECT * FROM dst").await.unwrap());
    assert!(
        rows.is_empty(),
        "INSERT SELECT 超限 SHALL 零写入，实得 {rows:?}"
    );
}
