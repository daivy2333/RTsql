//! 目录页组的有界读——change 2026-10-25-ms37-defect-ledger-closure
//! Iteration 004 / Cycle 000（T4）。
//!
//! 缺陷（ISS27 目录页组）：目录页（`__tables` / `__columns` 链）上的槽负载
//! 经**无范围检查**的 `SlottedPageRef::get_slot_data` 取得 ⇒ 槽偏移 / 长度
//! 越界 ⇒ `&self.data[start..end]` 越界切片 panic ⇒ 进程 abort；页头声称的
//! 槽数超出槽目录容量时，槽目录定位 `PAGE_SIZE - (index + 1) * Slot::SIZE`
//! 整数下溢 ⇒ 同样 abort。⇒ 目录损坏 today 既不能打开，也不给出任何诊断。
//!
//! 方向（Iteration 001 / Cycle 000 裁定）：**目录页损坏 ⇒ 该库 SHALL NOT
//! 打开**——目录是「哪些页仍然存活」的根依据；跳过该目录行会让该表静默
//! 消失（比打不开更糟）。打不开同时严格优于现状（今天是 panic / abort）。
//!
//! 本文件承载先行测试与对照：
//! - 真实文件库 + 真实 `Database::open` + 真实 CLI 二进制；
//! - 损坏注入 = **真实文件字节改写**（槽 0 的 offset 字段写成 `0xFFFF`；
//!   页头 `slot_count` 写成越界值）——不是替身 / 桩；
//! - 观察对象 = 打开 / 命令返回的错误与进程是否存活（panic ⇒ 用例失败）。

#![cfg(all(feature = "network", feature = "cli"))]
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::Slot;
use rtsql::storage::{Page, StorageError, COLUMNS_PAGE_ID, HEADER_SIZE, TABLES_PAGE_ID};
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("catalog_read.db")
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

/// 把某页某槽的 offset 字段改成越界值（`0xFFFF` 超过页长）——真实改坏页字节。
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

/// 把某页的页头 `slot_count` 改成越界值——真实改坏页字节（页级形态）。
///
/// 槽目录自页尾向上生长，每项 `Slot::SIZE`（6）字节：页头声称的槽数超出
/// 槽目录容量时，槽目录定位整数下溢 ⇒ 取槽即越界切片 panic。
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

/// 夹具：建库 → 建表 `t`（2 列）→ 插入 1 行 → checkpoint → 关库。
///
/// `__tables` 页（页 0）与 `__columns` 页（页 1）各至少携带 1 行（前置断言）。
async fn seed_catalog(path: &Path) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1)").await;
    db.checkpoint().await.unwrap();
    db.close().await.unwrap();
    drop(db);

    assert!(
        page_slot_count(path, TABLES_PAGE_ID) >= 1,
        "fixture precondition: the __tables page must carry at least one catalog row"
    );
    assert!(
        page_slot_count(path, COLUMNS_PAGE_ID) >= 1,
        "fixture precondition: the __columns page must carry at least one column row"
    );
}

/// 打开失败必须以**点名该页与该槽**的错误结束。
///
/// 关键：错误必须是 `StorageError::CorruptedCatalogPage` 变体（不是「该表
/// 不存在 / 目录为空」一类语义——那会让该表静默消失），且 detail 点名该槽
/// 序号与越界区间；**不得**臆造表名。
fn assert_corrupted_catalog_page_error(err: &StorageError, page_id: u64) {
    match err {
        StorageError::CorruptedCatalogPage {
            page_id: got_page,
            detail,
        } => {
            assert_eq!(
                *got_page, page_id,
                "the error must name the damaged catalog page {page_id}, got {detail}"
            );
            assert!(
                detail.contains("slot 0"),
                "the error must name the offending slot: {detail}"
            );
            assert!(
                detail.contains('[') && detail.contains(')'),
                "the error must name the offending slot range: {detail}"
            );
        }
        other => panic!(
            "a damaged catalog page must fail the open with a named error, got {other:?} ({other})"
        ),
    }
}

/// 打开一个已损坏目录页的库——必须以点名错误结束（不 panic、不 abort）。
async fn open_corrupted_catalog(path: &Path) -> StorageError {
    match Database::open(path).await {
        Ok(_) => panic!("a damaged catalog page must make the database fail to open"),
        Err(e) => e,
    }
}

// ===========================================================================
// 损坏的目录页：该库 SHALL NOT 打开（点名该页与该槽，不 panic / 不 abort）
// ===========================================================================

/// `__tables` 目录页上的越界槽 ⇒ 打开以点名错误结束（不 panic / 不 abort）。
///
/// 变更前（RED）：`scan_chain` 用无范围检查的 `get_slot_data` 取槽 0 的负载
/// ⇒ `&self.data[65535..]` 越界切片 panic ⇒ 进程 abort。
#[tokio::test]
async fn damaged_tables_catalog_page_makes_the_database_fail_to_open() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_catalog(&path).await;

    corrupt_slot_offset(&path, TABLES_PAGE_ID, 0);

    let err = open_corrupted_catalog(&path).await;
    assert_corrupted_catalog_page_error(&err, TABLES_PAGE_ID);
}

/// `__columns` 目录页上的越界槽 ⇒ 打开以点名错误结束（不 panic / 不 abort）。
#[tokio::test]
async fn damaged_columns_catalog_page_makes_the_database_fail_to_open() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_catalog(&path).await;

    corrupt_slot_offset(&path, COLUMNS_PAGE_ID, 0);

    let err = open_corrupted_catalog(&path).await;
    assert_corrupted_catalog_page_error(&err, COLUMNS_PAGE_ID);
}

/// 页头声称的槽数超出槽目录容量（页级形态）⇒ 打开以点名该页、页头声称的
/// 槽数与容量上限的错误结束（不 panic / 不 abort）。
#[tokio::test]
async fn damaged_catalog_page_slot_count_makes_the_database_fail_to_open() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_catalog(&path).await;

    corrupt_page_slot_count(&path, TABLES_PAGE_ID, 700);

    let err = open_corrupted_catalog(&path).await;
    match &err {
        StorageError::CorruptedCatalogPage { page_id, detail } => {
            assert_eq!(*page_id, TABLES_PAGE_ID, "must name the damaged page");
            assert!(
                detail.contains("slot count 700"),
                "the error must name the slot count the page header claims: {detail}"
            );
            assert!(
                detail.contains("680 slots"),
                "the error must name the slot directory capacity: {detail}"
            );
        }
        other => panic!(
            "a corrupted catalog header must fail the open with a named error, got {other:?}"
        ),
    }
}

// ===========================================================================
// 使用者面：后果可见（`rtsql check` 报出打开错误，而非基于残缺目录的结论）
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
                    panic!("rtsql check did not exit within 60s");
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

/// `rtsql check <db>` 必须报出**打开错误**（退出码普通非零、非信号终止），
/// 而不是基于残缺目录的结论，也不是进程 abort。
#[tokio::test]
async fn damaged_catalog_page_makes_the_cli_report_the_open_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_catalog(&path).await;

    corrupt_slot_offset(&path, TABLES_PAGE_ID, 0);

    let child = spawn_cli(dir.path(), &["check", path.to_str().unwrap()]);
    let out = tokio::task::spawn_blocking(move || wait_cli(child))
        .await
        .expect("the CLI wait task must not panic");

    let code = out
        .code
        .expect("the CLI must exit with an ordinary non-zero code, not die from a signal");
    assert_ne!(code, 0, "the CLI must fail on a damaged catalog page");
    let combined = format!("{}{}", out.stdout, out.stderr);
    assert!(
        combined.contains("failed to open database"),
        "the CLI must report the open failure: {combined}"
    );
    assert!(
        combined.contains(&format!("corrupted catalog page {TABLES_PAGE_ID}")),
        "the CLI must name the damaged catalog page: {combined}"
    );
}

// ===========================================================================
// 未损坏对照：打开 / 目录内容 / DML 行为零变化
// ===========================================================================

/// 未损坏路径上：库能打开、表与列目录可见、DML 结果与既有行为一致。
#[tokio::test]
async fn undamaged_catalog_reads_are_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_catalog(&path).await;

    // 对照组不改动任何字节。
    let db = Database::open(&path).await.unwrap();

    // 表与列目录可见。
    let meta = db.table_manager.get_table("t").await.unwrap();
    assert_eq!(meta.columns.len(), 2, "the column catalog must be intact");
    assert_eq!(meta.columns[0].0, "id");
    assert_eq!(meta.columns[1].0, "v");

    // 全表扫描：夹具写入的 1 行可见。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    match resp {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 1),
        other => panic!("expected a query result, got {other:?}"),
    }

    // DML 结果与既有行为一致。
    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (2, 2)").await);
    match resp {
        Response::AffectedRows { count } => assert_eq!(count, 1),
        other => panic!("expected an affected-rows result, got {other:?}"),
    }
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    match resp {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 2),
        other => panic!("expected a query result, got {other:?}"),
    }
    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 2").await);
    match resp {
        Response::AffectedRows { count } => assert_eq!(count, 1),
        other => panic!("expected an affected-rows result, got {other:?}"),
    }
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    match resp {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 1),
        other => panic!("expected a query result, got {other:?}"),
    }
}
