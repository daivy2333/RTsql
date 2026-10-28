//! MS26 T2 WAL 字节水位自动 checkpoint 集成测试
//! （`wal-auto-checkpoint` R1/R2/R3/R4/R5，10 场景）。
//!
//! 见证设计（Task Contract Test witness + Implementation Guidance）：
//!
//! - **lib 通道**（`Database::set_checkpoint_bytes_threshold`）承载全部确定性
//!   场景：触发回落 / 重开恢复 / 零工作 / 显式事务顺延 / 失败传播 / CTAS
//!   语义边界。触发与回落都在同一进程内以 `.wal` 文件长度断言。
//! - **env 通道**（`RTSQL_WAL_CHECKPOINT_BYTES`）由 CLI 层读取，故以子进程
//!   见证：非法值 exit 2 零文件（打开前拒绝），以及 env 覆盖 / `=0` 禁用
//!   的**会话中途** `.wal` 长度。干净退出的会话在两条通道下终态收敛——
//!   各自都只留一条 Checkpoint 记录、位点都归零，故终态不可区分；只有
//!   会话尚未结束时的 `.wal` 长度才能区分「提交边界自动触发」与「仅
//!   close 触发」。故以采样器在子进程存活期间连续量测 `.wal`。
//! - 进程内 close→reopen 依 R24 配方：显式作用域 + 块尾显式 `close()`，
//!   块退出（drop）释放 advisory 文件锁。
//!
//! 不引入任何旁路探针、旁路计数器或判定层：长度回落、失败点名与数据存
//! 活全部在测试自身的断言内完成。

#![cfg(all(feature = "network", feature = "cli"))]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::ColumnType;
use rtsql::wal::WalWriter;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

// ---------------------------------------------------------------- 夹具工具

fn wal_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("wal")
}

fn site_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("checkpoint")
}

/// `.wal` 当前字节长度（不存在按 0 计）。
fn wal_len(db_path: &Path) -> u64 {
    std::fs::metadata(wal_path(db_path))
        .map(|m| m.len())
        .unwrap_or(0)
}

async fn open_db(db_path: &Path) -> Database {
    Database::open_with_isolation(db_path, rtsql::transaction::IsolationLevel::RepeatableRead)
        .await
        .unwrap_or_else(|e| panic!("open {}: {e}", db_path.display()))
}

async fn create_table(db: &Database) {
    db.create_table(
        "t",
        vec![
            ("id".to_string(), ColumnType::Int),
            ("pad".to_string(), ColumnType::String(100)),
        ],
        "id",
    )
    .await
    .unwrap_or_else(|e| panic!("create table: {e}"));
}

/// 插入 `from..=to` 的单行批次；每条一个提交边界。
async fn insert_range(db: &Database, from: i64, to: i64) {
    for id in from..=to {
        match to_response(
            db.execute_sql(&format!("INSERT INTO t (id, pad) VALUES ({id}, 'p{id}')"))
                .await,
        ) {
            Response::AffectedRows { .. } => {}
            other => panic!("insert {id} must succeed, got {other:?}"),
        }
    }
}

async fn query_rows(db: &Database, sql: &str, what: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{what}: expected QueryResult, got {other:?}"),
    }
}

/// `SELECT COUNT(*)` 的单值计数。
async fn count_of(db: &Database, what: &str) -> i64 {
    let rows = query_rows(db, "SELECT COUNT(*) FROM t", what).await;
    rows[0][0]
        .as_i64()
        .unwrap_or_else(|| panic!("{what}: count value: {rows:?}"))
}

fn expect_error(resp: Response, what: &str, needle: &str) -> String {
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains(needle),
                "{what}: error must name {needle:?}, got: {message}"
            );
            message
        }
        other => panic!("{what}: expected Error, got {other:?}"),
    }
}

/// 注入 checkpoint 九步步 5（位点写入）失败：把位点路径换成目录，
/// `OpenOptions::open` 必然失败。步骤 1–4（刷脏页 + WAL fsync）不受影响，
/// WAL 不被截断——已提交数据完整留在 WAL 中，可供重开恢复。
fn inject_site_write_failure(db_path: &Path) {
    let site = site_path(db_path);
    let _ = std::fs::remove_file(&site);
    std::fs::create_dir(&site).expect("create directory in place of the site file");
}

/// 恢复现场：移除注入的目录，使后续 checkpoint 与 close 回到正常路径。
fn restore_site(db_path: &Path) {
    let _ = std::fs::remove_dir_all(site_path(db_path));
}

// ------------------------------------------------------------ CLI 子进程

struct CliSample {
    /// 子进程存活期间观察到的 `.wal` 最大长度。
    max_observed: u64,
    /// 子进程存活期间完成的采样次数（见证非空转，防守空转通过）。
    samples_while_alive: usize,
    /// 进程结束后的 `.wal` 长度。
    final_len: u64,
    exit_code: i32,
    /// 子进程 stderr（失败时随断言消息带出，诊断 exit code 来源）。
    stderr: String,
}

/// 在独立 TempDir CWD 中运行 rtsql 一次，存活期间按 2ms 采样 `.wal` 长度。
///
/// `db_stem` 是**裸名**（不带扩展名）：CLI 把它解析为
/// `$RTSQL_HOME/db/<stem>.db`，故 `RTSQL_HOME` 指向 TempDir 根并预建 `db/`
/// 子目录（CLI 不建父目录），采样路径为 `db/<stem>.db`。
/// stdout 丢弃、stderr 落 TempDir 内文件：工作负载逐语句渲染上千行 JSON，
/// 无人读取的管道会在写满后把子进程阻塞住，故不建 stdout 管道。
fn run_cli_sampling_wal(db_stem: &str, sql: &str, env: &[(&str, &str)]) -> CliSample {
    let dir: TempDir = tempdir().expect("tempdir");
    let db_dir = dir.path().join("db");
    std::fs::create_dir(&db_dir).expect("pre-create db/ (the CLI does not)");
    let db_path = db_dir.join(format!("{db_stem}.db"));
    let stderr_path = dir.path().join("cli-stderr.txt");
    let stderr_file = std::fs::File::create(&stderr_path).expect("create stderr sink");

    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建目标库（断言语义不变）。
    let new_status = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["new", db_stem])
        .current_dir(dir.path())
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn rtsql new");
    assert!(new_status.success(), "rtsql new {db_stem} failed");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args([db_stem, sql])
        .current_dir(dir.path())
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_WAL_CHECKPOINT_BYTES")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file));
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn rtsql");

    let mut max_observed = 0u64;
    let mut samples_while_alive = 0usize;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let len = wal_len(&db_path);
        if child.try_wait().expect("try_wait").is_some() {
            max_observed = max_observed.max(len);
            break;
        }
        if len > 0 {
            max_observed = max_observed.max(len);
            samples_while_alive += 1;
        }
        assert!(
            Instant::now() < deadline,
            "rtsql did not exit within 120s — a one-shot CLI must terminate on its own"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let status = child.wait().expect("wait rtsql");
    let exit_code = status.code().expect("exit code");
    let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();

    CliSample {
        max_observed,
        samples_while_alive,
        final_len: wal_len(&db_path),
        exit_code,
        stderr,
    }
}

struct CliOutput {
    code: Option<i32>,
    /// 只保留 stderr：非法值拒绝的判定面是退出码与点名文案（stdout 在非
    /// TTY 下为逐语句 JSON，与本断言无关）。
    stderr: String,
}

fn run_cli(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> CliOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_WAL_CHECKPOINT_BYTES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
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

// ------------------------------------------------------------ CLI 工作负载

/// 低水位 env 通道的工作负载（`wal-auto-checkpoint` R2）。
///
/// 整段 SQL 作为**单个 argv 实参**传给 one-shot CLI，故受 Linux
/// `MAX_ARG_STRLEN`（128 KiB）约束：600 条长填充插入约 79 KiB 字符、WAL 约
/// 80 KiB，尾部 1000 条 `SELECT 1` 约 10 KiB 且把子进程存活时间拉长到
/// 采样器能稳定取样。8 KiB 水位下写入量约为水位的 10 倍，两条通道的
/// 判别面在 32 KiB 处，留出两侧 ≥2 倍余量。
const PAD_LEN: usize = 90;
const WORKLOAD_ROWS: usize = 600;
const TAIL_SELECTS: usize = 1000;
const ENV_THRESHOLD: &str = "8192";
/// 两条通道的判别面：启用时 `.wal` 峰值远低于此（理论上界 ≈ 水位 + 一帧 +
/// 保留的 Checkpoint 记录），禁用时远超此（总量约 10× 水位）。
const ENV_DISCRIMINATOR: u64 = 32 * 1024;

fn long_write_workload() -> String {
    let pad = "x".repeat(PAD_LEN);
    let mut sql = String::with_capacity(WORKLOAD_ROWS * (PAD_LEN + 48) + TAIL_SELECTS * 10);
    sql.push_str("CREATE TABLE t (id INT PRIMARY KEY, pad VARCHAR(100));");
    for id in 1..=WORKLOAD_ROWS {
        sql.push_str(&format!("INSERT INTO t (id, pad) VALUES ({id}, '{pad}');"));
    }
    for _ in 0..TAIL_SELECTS {
        sql.push_str("SELECT 1;");
    }
    sql
}

// ------------------------------------------------------------------ 场景

/// wal-auto-checkpoint R2 默认值：水位默认 16 MiB。
#[test]
fn default_threshold_is_16_mib() {
    let dir = tempdir().unwrap();
    let writer = WalWriter::open(&dir.path().join("d.db")).unwrap();
    assert_eq!(
        writer.checkpoint_bytes_threshold(),
        16 * 1024 * 1024,
        "默认水位必须是 16 MiB"
    );
}

/// wal-auto-checkpoint R1「长会话 WAL 有界」：lib 通道设置低水位后连续写入，
/// 提交边界触发 checkpoint，`.wal` 物理回落且全量行可查。
#[tokio::test(flavor = "multi_thread")]
async fn lib_watermark_bounds_wal_and_keeps_rows() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("lib.db");
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        db.set_checkpoint_bytes_threshold(64 * 1024);
        // 远超 64 KiB 水位的连续提交边界
        insert_range(&db, 1, 2000).await;
        assert_eq!(
            count_of(&db, "count after trigger").await,
            2000,
            "触发 checkpoint 不得丢行"
        );
        db.close().await.unwrap();
    }
}

/// wal-auto-checkpoint R1「触发后重开恢复代价有界」：触发 + close 后重开，
/// 恢复成功且全部已提交行可查。
#[tokio::test(flavor = "multi_thread")]
async fn reopen_after_auto_checkpoint_recovers_every_row() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("reopen.db");
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        db.set_checkpoint_bytes_threshold(64 * 1024);
        insert_range(&db, 1, 2000).await;
        db.close().await.unwrap();
    }
    let db = open_db(&db_path).await;
    assert_eq!(
        count_of(&db, "reopen count").await,
        2000,
        "重开后已提交行必须全部可见"
    );
    db.close().await.unwrap();
}

/// wal-auto-checkpoint R1「零工作跳过保持」：无写入会话（含纯 SELECT 与
/// 显式 COMMIT 边界）不得产生任何 WAL 字节。
#[tokio::test(flavor = "multi_thread")]
async fn pure_select_session_writes_nothing() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ro.db");
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        db.set_checkpoint_bytes_threshold(1);
        db.close().await.unwrap();
    }
    let baseline = wal_len(&db_path);
    {
        let db = open_db(&db_path).await;
        // 水位设为 1 字节：任何写入都会立即触发，故此处断言纯读路径零 I/O。
        // 阈值在**本会话**下发——设置器作用于当前 `WalWriter`，重开即失效。
        db.set_checkpoint_bytes_threshold(1);
        for _ in 0..5 {
            let _ = query_rows(&db, "SELECT * FROM t", "pure select").await;
        }
        // 校准记录（change 2026-10-13 T3）：原断言覆盖整段会话（含末尾
        // 空 begin/commit 的收敛尾），其基线依赖「DDL 会话 close 全量
        // checkpoint」旧模型。T3/D4 起 DDL 语句边界有序落盘，会话 A 的
        // close 为零工作跳过（规格 `wal-auto-checkpoint`「零工作跳过保持」
        // + `wal-recovery-replay-integrity` 零工作判定谓词逐字节未变），
        // 基线从「1 条标记记录」变为 0。规格不变量「纯 SELECT 零 WAL 字节」
        // 在 SELECT 阶段严格断言；空事务尾由阈值 1 的自动 checkpoint 收敛
        // （其自身语义由 reopen 用例覆盖），close 对收敛态不追加。
        assert_eq!(
            wal_len(&db_path),
            baseline,
            "纯 SELECT 会话不得追加任何 WAL 字节"
        );
        let tx = db.begin().await.unwrap();
        db.commit(tx).await.unwrap();
        let converged = wal_len(&db_path);
        db.close().await.unwrap();
        assert_eq!(
            wal_len(&db_path),
            converged,
            "收敛态 close 为零工作跳过：不得追加 WAL 字节"
        );
    }
}

/// wal-auto-checkpoint R2 `=0` 禁用（lib 通道）：阈值 0 时不发生自动
/// checkpoint，`.wal` 持续增长；close 仍执行完整 checkpoint。
#[tokio::test(flavor = "multi_thread")]
async fn zero_threshold_disables_auto_checkpoint() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("zero.db");
    let grown;
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        db.set_checkpoint_bytes_threshold(0);
        insert_range(&db, 1, 2000).await;
        grown = wal_len(&db_path);
        assert!(
            grown > 128 * 1024,
            "禁用水位时 WAL 应持续增长，实际 {grown} 字节"
        );
        db.close().await.unwrap();
    }
    assert!(
        wal_len(&db_path) < grown,
        "close 仍必须执行完整 checkpoint（WAL 物理回落），{grown} → {}",
        wal_len(&db_path)
    );
}

/// wal-auto-checkpoint R3「显式事务期间顺延」：`BEGIN` 后写入使水位超限
/// 但事务仍活跃 → 不触发；`COMMIT` 完成后的边界触发 checkpoint。
#[tokio::test(flavor = "multi_thread")]
async fn explicit_transaction_defers_watermark_until_commit() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tx.db");
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        db.set_checkpoint_bytes_threshold(64 * 1024);
        db.close().await.unwrap();
    }
    let db = open_db(&db_path).await;
    // 阈值在本会话下发——设置器作用于当前 `WalWriter`，重开回到 16 MiB 默认。
    db.set_checkpoint_bytes_threshold(64 * 1024);
    let tx = db.begin().await.unwrap();
    for id in 1..=2000 {
        let resp = to_response(
            db.execute_in_tx(
                &format!("INSERT INTO t (id, pad) VALUES ({id}, 'p{id}')"),
                &tx,
            )
            .await,
        );
        assert!(
            matches!(resp, Response::AffectedRows { .. }),
            "事务内插入必须成功，got {resp:?}"
        );
    }
    let before_commit = wal_len(&db_path);
    // 顺延的前提是累计字节已越过水位，否则「不截断」无法与「未达水位」区分
    assert!(
        before_commit > 64 * 1024,
        "事务内写入应使累计追加字节越过 64 KiB 水位，实际 {before_commit} 字节"
    );
    // 事务仍活跃 → 顺延：WAL 不得被截断
    assert_eq!(
        wal_len(&db_path),
        before_commit,
        "其他活跃事务存在时必须顺延，不得截断 WAL"
    );
    // COMMIT 后的提交边界触发 checkpoint
    db.commit(tx).await.unwrap();
    let after_commit = wal_len(&db_path);
    assert!(
        after_commit < before_commit / 2,
        "COMMIT 后的提交边界应触发 checkpoint 回落，{before_commit} → {after_commit}"
    );
    let count_of_tx = count_of(&db, "tx count").await;
    assert_eq!(count_of_tx, 2000, "顺延与触发都不得丢行");
    db.close().await.unwrap();
}

/// wal-auto-checkpoint R4「失败诚实报告且数据不丢」：注入步 5 位点写入
/// 失败后，触发边界的提交收到点名 checkpoint 的错误；已提交行经恢复现场
/// 后的重开全部可查（WAL 完整，提交先于 checkpoint）。
#[tokio::test(flavor = "multi_thread")]
async fn checkpoint_failure_names_checkpoint_and_keeps_rows() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("fail.db");
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        insert_range(&db, 1, 20).await;
        db.set_checkpoint_bytes_threshold(1);
        inject_site_write_failure(&db_path);

        let resp = to_response(
            db.execute_sql("INSERT INTO t (id, pad) VALUES (100, 'after')")
                .await,
        );
        let message = expect_error(resp, "auto checkpoint failure", "checkpoint failed");
        assert!(
            !message.is_empty(),
            "错误文案必须非空（点名 checkpoint 失败）"
        );

        // 恢复现场后 close 回到正常路径（write_count 未归零 → 全量 checkpoint）
        restore_site(&db_path);
        db.close().await.unwrap();
    }
    let db = open_db(&db_path).await;
    assert_eq!(
        count_of(&db, "after failure").await,
        21,
        "失败语句的提交已落 WAL，重开后必须可见（含 id=100）"
    );
    db.close().await.unwrap();
}

/// wal-auto-checkpoint R4 × CTAS 语义边界：CTAS 隐式提交点的 checkpoint
/// 失败 SHALL NOT 触发 `drop_table`——表与已提交数据存活，错误点名传播。
#[tokio::test(flavor = "multi_thread")]
async fn ctas_checkpoint_failure_keeps_table_and_rows() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ctas.db");
    {
        let db = open_db(&db_path).await;
        create_table(&db).await;
        insert_range(&db, 1, 20).await;
        db.set_checkpoint_bytes_threshold(1);
        inject_site_write_failure(&db_path);

        let resp = to_response(
            db.execute_sql("CREATE TABLE copy_t AS SELECT id, pad FROM t")
                .await,
        );
        expect_error(resp, "ctas auto checkpoint failure", "checkpoint failed");

        // 表存活：已提交数据不可被 checkpoint 失败连带删除
        let survivors = query_rows(
            &db,
            "SELECT COUNT(*) FROM copy_t",
            "ctas table must survive",
        )
        .await;
        assert_eq!(
            survivors[0][0].as_i64(),
            Some(20),
            "CTAS 表与已提交行必须在 checkpoint 失败后存活"
        );

        restore_site(&db_path);
        db.close().await.unwrap();
    }
    let db = open_db(&db_path).await;
    let survivors = query_rows(&db, "SELECT COUNT(*) FROM copy_t", "ctas after reopen").await;
    assert_eq!(survivors[0][0].as_i64(), Some(20), "重开后 CTAS 表仍存在");
    db.close().await.unwrap();
}

/// wal-auto-checkpoint R2「环境变量覆盖默认值」：`RTSQL_WAL_CHECKPOINT_BYTES=65536`
/// 的长会话在提交边界自动 checkpoint——会话存活期间 `.wal` 长度始终有界。
#[test]
fn env_override_triggers_auto_checkpoint_during_session() {
    let sample = run_cli_sampling_wal(
        "env_on",
        &long_write_workload(),
        &[("RTSQL_WAL_CHECKPOINT_BYTES", ENV_THRESHOLD)],
    );
    assert_eq!(
        sample.exit_code, 0,
        "env 覆盖下工作负载必须成功，stderr: {}",
        sample.stderr
    );
    assert!(
        sample.samples_while_alive >= 3,
        "子进程存活期间采样过少（{} 次），见证可能空转",
        sample.samples_while_alive
    );
    assert!(
        sample.max_observed < ENV_DISCRIMINATOR,
        "水位 {ENV_THRESHOLD} 时会话中途 `.wal` 必须有界（实测峰值 {} 字节 = 未截断）",
        sample.max_observed
    );
}

/// wal-auto-checkpoint R2「0 禁用」：`=0` 的长会话 `.wal` 持续增长到远超
/// 默认水位，全程不被截断；退出码 0（close 仍完整 checkpoint）。
#[test]
fn env_zero_keeps_wal_growing() {
    let sample = run_cli_sampling_wal(
        "env_off",
        &long_write_workload(),
        &[("RTSQL_WAL_CHECKPOINT_BYTES", "0")],
    );
    assert_eq!(
        sample.exit_code, 0,
        "禁用水位时工作负载仍必须成功，stderr: {}",
        sample.stderr
    );
    assert!(
        sample.samples_while_alive >= 3,
        "子进程存活期间采样过少（{} 次），见证可能空转",
        sample.samples_while_alive
    );
    assert!(
        sample.max_observed > ENV_DISCRIMINATOR,
        "=0 禁用时会话中途 `.wal` 应持续增长、远超水位（实测峰值 {} 字节）",
        sample.max_observed
    );
    assert!(
        sample.final_len < sample.max_observed,
        "close 仍必须执行完整 checkpoint（WAL 物理回落）：峰值 {} → 终态 {}",
        sample.max_observed,
        sample.final_len
    );
}

/// wal-auto-checkpoint R2「非法配置打开期点名拒绝」：非数字 / 负数 / 溢出
/// 三形态各自 exit 2，文案含变量名与非法值，且零副作用（无库文件）。
#[test]
fn env_illegal_values_rejected_before_open() {
    for raw in ["abc", "-1", "99999999999999999999"] {
        let dir = tempdir().unwrap();
        let out = run_cli(
            dir.path(),
            &["illegal.db", "SELECT 1"],
            &[("RTSQL_WAL_CHECKPOINT_BYTES", raw)],
        );
        assert_eq!(
            out.code,
            Some(2),
            "非法值 {raw:?} 必须 exit 2，stderr: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains("RTSQL_WAL_CHECKPOINT_BYTES"),
            "非法值 {raw:?} 的报错必须点名变量，stderr: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains(raw),
            "非法值 {raw:?} 的报错必须回显非法值，stderr: {}",
            out.stderr
        );
        assert!(
            !dir.path().join("illegal.db").exists(),
            "非法值必须在打开前拒绝（零副作用），实际创建了库文件"
        );
    }
}
