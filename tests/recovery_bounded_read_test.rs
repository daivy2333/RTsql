//! 恢复面组的有界读——change 2026-10-25-ms37-defect-ledger-closure
//! Iteration 005 / Cycle 000（T5）。
//!
//! 缺陷（ISS27 恢复面组）：WAL 重放面（`src/wal/recovery.rs`）上的槽读取
//! 无范围检查 ⇒ `&self.data[start..end]` 越界切片 panic ⇒ 进程 abort；页头
//! 声称的槽数超出槽目录容量时，槽目录定位 `PAGE_SIZE - (index + 1) *
//! Slot::SIZE` 整数下溢 ⇒ 同样 abort。⇒ 恢复在损坏数据页上 today 既不能
//! 完成，也不能给出任何诊断。
//!
//! 方向（`tasks.md` Iteration 005 的裁定，按**恢复类别**分面处置）：
//! - **扫描类**（重放前候选扫描 / 重放后索引重建 / 未提交事务标记）⇒ 跳过
//!   该槽（页级形态 ⇒ 跳过该**页**）、**计入恢复告警并报出**、其余可恢复
//!   记录继续重放；
//! - **已提交记录重放类**（更新重放所需的旧行推导 / 重放的幂等探测）⇒
//!   **不跳过该记录** ⇒ 以**明确的错误**结束恢复与打开。
//!
//! 本文件承载先行测试与对照：
//! - 真实文件库 + 真实 `Database::open` + 真实 CLI 二进制；
//! - 损坏注入 = **真实文件字节改写**（槽 offset 字段写成 `0xFFFF`；页头
//!   `slot_count` 写成越界值）——不是替身 / 桩；
//! - 观察对象 = 打开结果、进程是否存活、stderr 文本（断言落在**行为**上，
//!   不断言尚不存在的错误变体类型）。

#![cfg(all(feature = "network", feature = "cli"))]
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::Slot;
use rtsql::storage::{Page, HEADER_SIZE};
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("recovery_read.db")
}

/// 语句执行——断言成功（`Response::Error` 即失败）。
async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement must succeed: {sql:?} -> {resp:?}"
    );
}

/// 槽目录项的字节布局 `[logical_id:2][offset:2][length:2]`；第 `index` 项
/// 落在页尾 `PAGE_SIZE - (index + 1) * Slot::SIZE` 处，offset 字段再 +2。
fn slot_offset_field_offset(slot_index: usize) -> usize {
    Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE + 2
}

fn page_file_offset(page: u64, in_page: usize) -> u64 {
    (HEADER_SIZE + page as usize * Page::PAGE_SIZE + in_page) as u64
}

/// 把某页某槽的 offset 字段改成越界值（`0xFFFF` 超过页长）——真实改坏页字节
/// （**槽级形态**：负载越界）。
fn corrupt_slot_offset(path: &Path, page: u64, slot_index: usize) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(page_file_offset(
        page,
        slot_offset_field_offset(slot_index),
    )))
    .unwrap();
    f.write_all(&0xFFFFu16.to_le_bytes()).unwrap();
    f.flush().unwrap();
}

/// 页头 `slot_count` 字段位于页内偏移 1..3（`SlottedPageHeader::deserialize`
/// 读 `buf[1..3]`）⇒ 文件偏移 = `HEADER_SIZE + page * PAGE_SIZE + 1`。
fn slot_count_field_offset() -> usize {
    1
}

/// 把某页的页头 `slot_count` 改成越界值——真实改坏页字节（**页级形态**：
/// 页头声称的槽数超出槽目录容量 ⇒ 槽目录定位整数下溢）。
fn corrupt_page_slot_count(path: &Path, page: u64, slot_count: u16) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(page_file_offset(
        page,
        slot_count_field_offset(),
    )))
    .unwrap();
    f.write_all(&slot_count.to_le_bytes()).unwrap();
    f.flush().unwrap();
}

/// 页头声称的槽数（夹具前置断言用）。
fn page_slot_count(path: &Path, page: u64) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    let off = page_file_offset(page, slot_count_field_offset()) as usize;
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}

/// 夹具：建库 → 建表 `t` → 插入 2 行 → 对第 2 行执行一次 UPDATE → 脏页落盘
/// → 崩溃模拟（WAL 落盘但不 checkpoint / 不截断）⇒ 下次打开触发全量重放。
///
/// 返回数据页号（`data_page_head`）。
///
/// 落盘用 `buffer_pool.flush_all()`（**不**走 checkpoint）⇒ 磁盘上的数据页
/// 承载真实的 3 个槽（行 1、行 2、行 2 的新版本），而 WAL 仍保有全部记录
/// ⇒ 重放走「幂等命中」路径。UPDATE 触及的是**行 2** ⇒ 槽 0（行 1）**未被
/// UPDATE 触及**。
async fn seed_recovery_fixture(path: &Path) -> u64 {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'b')").await;
    exec_ok(&db, "UPDATE t SET v = 'c' WHERE id = 2").await;

    let meta = db.table_manager.get_table("t").await.unwrap();
    let head = meta.data_page_head.0;

    // WAL 落盘（shutdown ≠ close，不触发 checkpoint / 不截断 WAL）。
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    // 脏页落盘（WAL 不截断 ⇒ 重开触发全量重放）。
    db.buffer_pool.flush_all().await.unwrap();
    drop(db);

    // 前置：该数据页在磁盘上真实承载 3 个槽（行 1、行 2、行 2 的新版本）。
    assert_eq!(
        page_slot_count(path, head),
        3,
        "fixture precondition: the data page must carry 3 slots on disk"
    );
    head
}

/// 夹具：建库 → 建表 `t` → 插入 2 行（已提交）→ 开一个**未提交**的事务并
/// 插入第 3 行 → 脏页落盘 → 崩溃模拟。
///
/// 未提交事务在 WAL 中留下 `BeginTxn`（无 `CommitTxn`）⇒ 恢复把它归入
/// `uncommitted_tx_ids` ⇒「未提交事务标记」扫描点会进入（其既有语义：
/// `uncommitted_tx_ids` 为空即零工作早退）。
async fn seed_uncommitted_fixture(path: &Path) -> u64 {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'b')").await;

    // 未提交事务：WAL 留下 BeginTxn（无 CommitTxn）⇒ 恢复按未提交处理。
    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (3, 'd')", &tx)
        .await
        .expect("the uncommitted insert must succeed");

    let meta = db.table_manager.get_table("t").await.unwrap();
    let head = meta.data_page_head.0;

    db.wal_buffer.as_ref().unwrap().shutdown().await;
    db.buffer_pool.flush_all().await.unwrap();
    drop(db);

    head
}

// ===========================================================================
// 扫描类：越界槽被跳过、计入恢复告警并报出、其余可恢复记录继续重放
// ===========================================================================

/// 用例 A（扫描类 · 槽级形态）：损坏**未被 UPDATE 触及**的行（槽 0）的槽
/// 负载 ⇒ 恢复跳过该槽、报出、其余记录继续重放 ⇒ **打开成功**。
///
/// 变更前（RED）：W1 的候选扫描用无范围检查的 `get_slot_data` 取槽 0 的负载
/// ⇒ `&self.data[65535..]` 越界切片 panic ⇒ 进程 abort。
#[tokio::test]
async fn scanning_class_skips_the_damaged_slot_and_the_replay_continues() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_recovery_fixture(&path).await;

    corrupt_slot_offset(&path, head, 0);

    // 扫描类：越界槽被跳过 ⇒ 恢复完成 ⇒ 打开成功（不 panic / 不 abort）。
    let db = Database::open(&path)
        .await
        .expect("the recovery must skip the damaged slot and complete the open");

    // 其余可恢复记录继续重放：未被损坏的行 2 仍可点查命中。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 2").await);
    match resp {
        Response::QueryResult { rows } => assert_eq!(
            rows.len(),
            1,
            "the undamaged row must survive the replay and the index rebuild"
        ),
        other => panic!("expected a query result, got {other:?}"),
    }

    // 被损坏的行在恢复中被跳过 ⇒ 重建后的主键索引不含它。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    match resp {
        Response::QueryResult { rows } => assert_eq!(
            rows.len(),
            0,
            "the damaged row must have been skipped by the recovery"
        ),
        other => panic!("expected a query result, got {other:?}"),
    }
}

/// 用例 A 的**报出**面：读写模式的真二进制子进程 ⇒ 恢复告警出现在 **stderr**
/// 且点名被跳过的页与槽；进程**存活**（退出码为普通退出码，不是信号终止）。
///
/// 恢复只在读写模式下运行（只读打开不触发恢复）⇒ 断言必须走读写子进程。
#[tokio::test]
async fn scanning_class_reports_the_skipped_slot_on_the_read_write_cli_stderr() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_recovery_fixture(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let child = spawn_cli(dir.path(), &["checkpoint", path.to_str().unwrap()]);
    let out = tokio::task::spawn_blocking(move || wait_cli(child))
        .await
        .expect("the CLI wait task must not panic");

    let code = out
        .code
        .expect("the CLI must exit with an ordinary code, not die from a signal");
    assert_eq!(
        code, 0,
        "the CLI must succeed on a database whose damaged slot was skipped: stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert!(
        out.stderr.contains("[recovery]"),
        "the recovery must report the skipped slot on stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains(&format!("page {head}")),
        "the recovery warning must name the skipped page {head}: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("slot 0"),
        "the recovery warning must name the skipped slot: {}",
        out.stderr
    );
}

// ===========================================================================
// 已提交记录重放类：越界槽**不跳过该记录** ⇒ 以明确的错误结束恢复与打开
// ===========================================================================

/// 用例 B（重放类 · 页级形态）：损坏**页头 `slot_count`** ⇒ 打开以**明确的
/// 错误**结束（进程不 abort、不 panic）。
///
/// 变更前（RED）：W1 / W5 的取槽按页头槽数迭代槽目录 ⇒ 槽目录定位整数下溢
/// ⇒ 越界切片 panic ⇒ 进程 abort。
#[tokio::test]
async fn replay_class_fails_the_open_with_a_named_error_on_a_damaged_page_header() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_recovery_fixture(&path).await;

    corrupt_page_slot_count(&path, head, 700);

    let err = match Database::open(&path).await {
        Ok(_) => panic!("a damaged page header must make the recovery fail with an error"),
        Err(e) => e,
    };
    let message = err.to_string();
    assert!(
        message.contains("update redo") || message.contains("redo_tuple_at_row_id"),
        "the failure must come from the replay of a committed record: {message}"
    );
    assert!(
        message.contains(&format!("corrupted data page {head}")),
        "the failure must name the damaged page {head}: {message}"
    );
    assert!(
        message.contains("slot count 700"),
        "the failure must name the slot count the page header claims: {message}"
    );
}

/// 用例 C（重放类 · 槽级形态）：损坏 **UPDATE 旧行所在**的槽 ⇒ 打开以**明确
/// 的错误**结束（进程不 abort、不 panic）。
///
/// 变更前（RED）：W1 先命中该槽 ⇒ 越界切片 panic ⇒ 进程 abort。
/// GREEN 后：W1 跳过该槽 ⇒ 该行不在更新推导的候选集内 ⇒ 既有
/// `WalError::RedoFailed` 语义（文案含 `update redo`）给出明确错误。
#[tokio::test]
async fn replay_class_fails_the_open_with_the_existing_redo_error_on_a_damaged_old_row() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_recovery_fixture(&path).await;

    // 槽 1 = 行 2 的旧版本（UPDATE 触及的旧行）。
    corrupt_slot_offset(&path, head, 1);

    let err = match Database::open(&path).await {
        Ok(_) => panic!("a damaged replayed row must make the recovery fail with an error"),
        Err(e) => e,
    };
    let message = err.to_string();
    assert!(
        message.contains("update redo"),
        "the failure must come from the committed update replay: {message}"
    );
}

// ===========================================================================
// 未损坏对照：恢复与打开行为零变化
// ===========================================================================

/// 用例 D（扫描类 · **未提交事务标记点**）：损坏的槽在「未提交事务标记」
/// 扫描点上也必须被跳过并报出，其余记录继续被标记（恢复完成 ⇒ 打开成功）。
///
/// 该扫描点只在存在**未提交事务**时进入（既有语义：`uncommitted_tx_ids`
/// 为空即零工作早退）⇒ 夹具额外开一个**未提交**事务（WAL 留下 `BeginTxn`
/// 而无 `CommitTxn`）。
///
/// 变更前（RED）：该扫描点用无范围检查的 `get_slot_data` 取槽负载 ⇒ 越界
/// 切片 panic ⇒ 进程 abort。
#[tokio::test]
async fn scanning_class_reports_the_skipped_slot_while_marking_uncommitted_rows() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_uncommitted_fixture(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let child = spawn_cli(dir.path(), &["checkpoint", path.to_str().unwrap()]);
    let out = tokio::task::spawn_blocking(move || wait_cli(child))
        .await
        .expect("the CLI wait task must not panic");

    let code = out
        .code
        .expect("the CLI must exit with an ordinary code, not die from a signal");
    assert_eq!(
        code, 0,
        "the CLI must succeed on a database whose damaged slot was skipped: stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert!(
        out.stderr.contains("mark uncommitted aborted of table 't'"),
        "the uncommitted-row marking scan must report the skipped slot: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains(&format!("slot 0 of page {head}")),
        "the recovery warning must name the skipped slot and its page: {}",
        out.stderr
    );
}

// ===========================================================================
// 未损坏对照：恢复与打开行为零变化
// ===========================================================================
/// 未损坏路径上：恢复完成、打开成功、两行都可见。
#[tokio::test]
async fn undamaged_recovery_is_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_recovery_fixture(&path).await;

    let db = Database::open(&path)
        .await
        .expect("an undamaged database must open after the replay");

    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    match resp {
        Response::QueryResult { rows } => assert_eq!(
            rows.len(),
            2,
            "both rows must survive the replay on an undamaged database"
        ),
        other => panic!("expected a query result, got {other:?}"),
    }
}

// ===========================================================================
// CLI 子进程辅助（读写模式 ⇒ 触发恢复）
// ===========================================================================

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 启动 rtsql 二进制（RTSQL_HOME 指向临时目录，隔离宿主环境变量；显式移除
/// RTSQL_KEY / RTSQL_BUSY_TIMEOUT 防宿主环境污染）。
fn spawn_cli(dir: &Path, args: &[&str]) -> Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().expect("spawn rtsql binary")
}

/// 等待子进程退出并收集输出；60s 未退出则 kill 并 panic（one-shot CLI 必须
/// 自行退出，挂起即为行为错误）。
fn wait_cli(mut child: Child) -> CliOutput {
    use std::io::Read;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx_out.send(buf);
    });
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let _ = tx_err.send(buf);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("rtsql checkpoint did not exit within 60s");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    CliOutput {
        code: status.code(),
        stdout: String::from_utf8_lossy(&rx_out.recv().unwrap()).to_string(),
        stderr: String::from_utf8_lossy(&rx_err.recv().unwrap()).to_string(),
    }
}
