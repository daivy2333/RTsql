//! 写路径的槽界限——change 2026-10-25-ms37-defect-ledger-closure
//! Iteration 003 / Cycle 000（T18 = ISS37 / T19 = ISS38）。
//!
//! 缺陷 T18（ISS37）：写路径对**页头声称的槽数**无任何有界检查——
//! `SlottedPage::add_slot_inner` / `free_space` / `delete_slot` 的
//! `PAGE_SIZE - slot_count * Slot::SIZE` 同形减法在页头槽数越界时
//! **整数下溢** ⇒ debug 构建 panic（进程 abort）、release 构建回绕 ⇒
//! 越界写；且 `data_page.rs` / `catalog.rs` 的「页满 ⇒ 换新页继续」分支
//! 把**任何**落位失败都当作页满 ⇒ 损坏页被跳过、数据继续落盘。
//!
//! 缺陷 T19（ISS38）：写路径对**槽负载**无边界检查——
//! `update_version_header_in_data_page` 的版本头原地写
//! `page.data[slot_offset..slot_offset + 22]` 与
//! `SlottedPage::update_slot_in_place` 的 `page.data[start..end]` 均无
//! 边界检查；且**未校验 `slot.length >= 22`** ⇒ 负载短于版本头时
//! 22 字节**越过该行自身负载**、覆盖相邻字节（**静默错写**，不 panic）。
//!
//! 本文件承载先行测试与对照：
//! - 真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + **真实文件
//!   字节改写**（不是替身 / 桩）；
//! - 观察对象 = 语句返回的错误、进程是否存活、以及**会被覆盖的字节区间
//!   是否逐字未变**；
//! - 写侧原语（`update_version_header_in_data_page` / `delete_tuple_from_data_page`）
//!   另以**直接调用**覆盖：SQL 入口在到达写侧之前先被读侧的有界判据拦下
//!   （Iteration 001 / 002 已交付），写侧原语自身才是写侧缺陷的入口。

use std::path::{Path, PathBuf};
use std::process::Command;

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::{RowId, Slot};
use rtsql::storage::{update_version_header_in_data_page, Page, PageId, HEADER_SIZE};
use rtsql::transaction::VersionHeader;
use tempfile::TempDir;

/// 数据页的页类型字节（页首）——`0x03` = 数据页。
const DATA_PAGE_TYPE: u8 = 0x03;

/// 页头声称的槽数越界值（> `MAX_PAGE_SLOTS` = 680）。
const CORRUPT_SLOT_COUNT: u16 = 700;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("write_path_slot_bounds.db")
}

/// 语句执行——断言成功（`Response::Error` 即失败）。
async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement must succeed: {sql:?} -> {resp:?}"
    );
}

/// 查询结果的行数。
fn row_count(resp: &Response) -> usize {
    match resp {
        Response::QueryResult { rows } => rows.len(),
        other => panic!("expected a query result, got {other:?}"),
    }
}

/// 影响行数。
fn affected_rows(resp: &Response) -> u64 {
    match resp {
        Response::AffectedRows { count } => *count,
        other => panic!("expected an affected-rows result, got {other:?}"),
    }
}

/// 槽目录项的字节布局 `[logical_id:2][offset:2][length:2]`；第 `index` 项
/// 落在页尾 `PAGE_SIZE - (index + 1) * Slot::SIZE` 处，offset 字段再 +2。
fn slot_offset_field_offset(slot_index: usize) -> usize {
    Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE + 2
}

/// 槽目录项的 `length` 字段页内偏移（槽项起点再 +4）。
fn slot_length_field_offset(slot_index: usize) -> usize {
    Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE + 4
}

/// 页头 `slot_count` 字段位于页内偏移 1..3（`SlottedPageHeader::deserialize`
/// 读 `buf[1..3]`）。
fn slot_count_field_offset() -> usize {
    1
}

fn page_file_offset(page: u32, in_page: usize) -> u64 {
    (HEADER_SIZE + page as usize * Page::PAGE_SIZE + in_page) as u64
}

fn write_page_bytes(path: &Path, page: u32, in_page: usize, bytes: &[u8]) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(page_file_offset(page, in_page)))
        .unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
}

/// 把某页页头的 `slot_count` 改成越界值——真实改坏页字节（ISS37）。
///
/// 槽目录自页尾向上生长，每项 `Slot::SIZE`（6）字节：页头声称的槽数超出
/// 槽目录容量（`(PAGE_SIZE - HEADER) / Slot::SIZE` = 680）时，槽目录定位
/// `PAGE_SIZE - (index + 1) * Slot::SIZE` 整数下溢 ⇒ 取槽即越界切片 panic。
fn corrupt_page_slot_count(path: &Path, page: u32, slot_count: u16) {
    write_page_bytes(
        path,
        page,
        slot_count_field_offset(),
        &slot_count.to_le_bytes(),
    );
}

/// 把某页某槽的 `offset` 字段改成越界值——真实改坏页字节（ISS38）。
fn corrupt_slot_offset(path: &Path, page: u32, slot_index: usize, offset: u16) {
    write_page_bytes(
        path,
        page,
        slot_offset_field_offset(slot_index),
        &offset.to_le_bytes(),
    );
}

/// 把某页某槽的 `length` 字段改成过短值——真实改坏页字节（ISS38）。
fn corrupt_slot_length(path: &Path, page: u32, slot_index: usize, length: u16) {
    write_page_bytes(
        path,
        page,
        slot_length_field_offset(slot_index),
        &length.to_le_bytes(),
    );
}

/// 从**页字节**解析槽目录项 0 的 `offset` / `length`（真实页解析，非假设值）。
fn slot0_fields(page_data: &[u8]) -> (u16, u16) {
    let base = Page::PAGE_SIZE - Slot::SIZE;
    let offset = u16::from_le_bytes([page_data[base + 2], page_data[base + 3]]);
    let length = u16::from_le_bytes([page_data[base + 4], page_data[base + 5]]);
    (offset, length)
}

/// 夹具：建库 → 建表 `t` 与 `other` → 向 `t` 插入 2 行 → checkpoint →
/// 取 `t` 的尾页页号 → 关库。
///
/// 返回 `t` 的数据页页号（`data_page_head`）。两行落在同一张数据页上
/// （数据页可容纳 ~90 行），槽 0 = 第一行（`id = 1`）。
async fn seed_data_page(path: &Path) -> u32 {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "CREATE TABLE other (id INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 2)").await;
    db.checkpoint().await.unwrap();
    let meta = db.table_manager.get_table("t").await.unwrap();
    let head = meta.data_page_head.0 as u32;
    db.close().await.unwrap();
    drop(db);

    // 前置：该页确实是数据页（页首类型字节）。
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        bytes[page_file_offset(head, 0) as usize] as u8,
        DATA_PAGE_TYPE,
        "fixture precondition: the table's head page must be a data page"
    );
    head
}

/// 损坏的数据页必须让语句以**点名该页**的明确错误结束——错误文案点名
/// 损坏页与该槽的区间 / 长度。
///
/// 关键：不得退化为「行不存在 / 表为空」的空结果，也不得 panic。
fn assert_named_corruption_error(resp: &Response, page_id: u32, needles: &[&str]) {
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains(&format!("corrupted data page {page_id}")),
                "the error must name the damaged page {page_id}: {message}"
            );
            for needle in needles {
                assert!(
                    message.contains(needle),
                    "the error must contain {needle:?}: {message}"
                );
            }
            // 页损坏**不得**被折叠成「行不存在 / 槽不存在」。
            assert!(
                !message.contains("not found"),
                "a damaged page must not be reported as a missing row/slot: {message}"
            );
        }
        other => {
            panic!("a damaged data page must fail the statement with a named error, got {other:?}")
        }
    }
}

/// 读取缓冲池中某页的当前字节（脏页的内存态——写后是否被改写直接可见）。
async fn page_bytes(db: &Database, page: u32) -> Vec<u8> {
    db.buffer_pool
        .with_page_data(PageId(page as u64), |data| Ok(data.to_vec()))
        .await
        .unwrap()
}

// ===========================================================================
// T18（ISS37）：页头槽数越界 —— 写路径不 abort、不下溢、不换页继续
// ===========================================================================

/// 夹具 A：页头声称的槽数超出槽目录容量时，INSERT 必须以**点名该页与页头
/// 声称的槽数**的错误结束——不得 panic / abort，**不得**换新页继续写入。
#[tokio::test]
async fn corrupted_page_slot_count_makes_inserts_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_page_slot_count(&path, head, CORRUPT_SLOT_COUNT);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (3, 3)").await);
    assert_named_corruption_error(&resp, head, &["slot count 700"]);

    // 不得被折叠成「页满 ⇒ 换新页继续」：尾页指针仍指向损坏页（未分配新页）。
    let meta = db.table_manager.get_table("t").await.unwrap();
    assert_eq!(
        *meta.data_page_tail.lock().unwrap(),
        PageId(head as u64),
        "a corrupted tail page must not be swapped for a fresh page (the write must fail, \
         not continue on a new page)"
    );
}

/// 夹具 A'：同一损坏页上的 DELETE 必须以**点名该页**的错误结束——不得
/// panic / abort，**不得**报告「行不存在 / 槽不存在」。
#[tokio::test]
async fn corrupted_page_slot_count_makes_deletes_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_page_slot_count(&path, head, CORRUPT_SLOT_COUNT);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 1").await);
    assert_named_corruption_error(&resp, head, &["slot count 700"]);
}

/// 夹具 A''：同一损坏页上的 SELECT 必须保持**既有**的读侧错误形态
/// （Iteration 002 已交付）——写侧修复不得改变读侧的逐点语义。
#[tokio::test]
async fn corrupted_page_slot_count_keeps_the_read_side_error_shape() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_page_slot_count(&path, head, CORRUPT_SLOT_COUNT);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_named_corruption_error(&resp, head, &["slot count 700"]);
}

/// 删槽面（写侧原语直接入口）：损坏页上的物理删槽必须以**点名该页**的
/// 错误结束——不得 panic / abort（删槽前的槽目录定位会整数下溢）。
#[tokio::test]
async fn corrupted_page_slot_count_makes_the_slot_deletion_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_page_slot_count(&path, head, CORRUPT_SLOT_COUNT);

    let db = Database::open(&path).await.unwrap();
    let err = rtsql::storage::delete_tuple_from_data_page(&db.buffer_pool, RowId::new(head, 0))
        .await
        .expect_err("a corrupted header slot count must be reported as an error, not a panic");
    let message = err.to_string();
    assert!(
        message.contains(&format!("corrupted data page {head}")),
        "the error must name the damaged page {head}: {message}"
    );
    assert!(
        message.contains("slot count 700"),
        "the error must name the slot count the page header claims: {message}"
    );
}

/// 未损坏对照：INSERT / DELETE 的结果与既有行为一致（零变化）。
#[tokio::test]
async fn undamaged_page_inserts_and_deletes_are_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let _head = seed_data_page(&path).await;

    // 对照组不改动任何字节。
    let db = Database::open(&path).await.unwrap();

    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (3, 3)").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 3);

    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 2").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 2);
}

/// A13：写入命中损坏页 ⇒ **语句失败**（非零退出码）⇒ 会话事务回滚 ⇒
/// 该事务内的**先前写入**不生效。
///
/// 真实 CLI 会话（`rtsql <db> "BEGIN; …"`）：第一条语句写入健康表
/// `other`，第二条语句写入损坏表 `t` ⇒ 失败 ⇒ 收尾显式回滚 ⇒ `other`
/// 的那一行不得留存。
#[test]
fn write_failure_rolls_back_the_session_transaction() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = block_on_seed(&path);

    corrupt_page_slot_count(&path, head, CORRUPT_SLOT_COUNT);

    let out = run_cli(
        &path,
        "BEGIN; INSERT INTO other VALUES (1, 1); INSERT INTO t VALUES (3, 3);",
    );
    let combined = format!("{}{}", out.stdout, out.stderr);
    assert!(
        out.code.is_some_and(|c| c != 0),
        "a write hitting a corrupted page must fail the statement (non-zero exit code): \
         stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    assert!(
        combined.contains(&format!("corrupted data page {head}")),
        "the failure must name the damaged page {head}: {combined}"
    );
    assert!(
        combined.contains("rolled back with the transaction"),
        "the failure must report that the session transaction was rolled back: {combined}"
    );

    // 重开库：同一事务内的先前写入随事务回滚（未生效）。
    let rows = block_on(async {
        let db = Database::open(&path).await.unwrap();
        let resp = to_response(db.execute_sql("SELECT * FROM other").await);
        row_count(&resp)
    });
    assert_eq!(
        rows, 0,
        "the write issued before the failure must be rolled back with the transaction"
    );
}

// ===========================================================================
// T19（ISS38）：槽负载越界 / 过短 —— 写路径不越界写、不静默改写相邻字节
// ===========================================================================

/// 夹具 B：槽的 `offset` 越界时，UPDATE 必须以**点名该页与该槽**的明确
/// 错误结束——不得 panic / abort。
#[tokio::test]
async fn corrupted_slot_offset_makes_updates_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0, 65535);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_named_corruption_error(&resp, head, &["slot 0"]);
}

/// 夹具 B2：槽的 `length` 过短（< 版本头长度）时，UPDATE 必须以**点名该页
/// 与该槽**的明确错误结束——**相邻字节逐字未变**。
#[tokio::test]
async fn short_slot_payload_makes_updates_fail_without_touching_neighbours() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_length(&path, head, 0, 4);

    let db = Database::open(&path).await.unwrap();
    let before = page_bytes(&db, head).await;
    let (offset, length) = slot0_fields(&before);
    assert_eq!(
        length, 4,
        "the fixture must have shortened slot 0's payload"
    );

    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_named_corruption_error(&resp, head, &["slot 0"]);

    let after = page_bytes(&db, head).await;
    let end = offset as usize + VersionHeader::SIZE;
    assert_eq!(
        &before[offset as usize..end],
        &after[offset as usize..end],
        "the version header write must not touch the bytes beyond the slot's own payload"
    );
}

/// 写侧原语（版本头原地写）直接入口：槽 `offset` 越界 ⇒ 点名该页与该槽
/// 的明确错误，**不得**越界切片 panic。
#[tokio::test]
async fn corrupted_slot_offset_makes_the_version_header_write_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0, 65535);

    let db = Database::open(&path).await.unwrap();
    let err = update_version_header_in_data_page(
        &db.buffer_pool,
        RowId::new(head, 0),
        VersionHeader::new(1, None),
        &[],
    )
    .await
    .expect_err("an out-of-bounds slot offset must be reported as an error, not a panic");
    let message = err.to_string();
    assert!(
        message.contains(&format!("corrupted data page {head}")),
        "the error must name the damaged page {head}: {message}"
    );
    assert!(
        message.contains("slot 0"),
        "the error must name the offending slot: {message}"
    );
    assert!(
        message.contains("65535"),
        "the error must name the slot's range: {message}"
    );
}

/// 写侧原语（版本头原地写）直接入口：槽负载过短 ⇒ 点名该页与该槽的
/// 「负载不足」错误，且**被覆盖范围内的字节逐字未变**（不得静默改写
/// 相邻字节）。
#[tokio::test]
async fn short_slot_payload_makes_the_version_header_write_fail_without_touching_neighbours() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_length(&path, head, 0, 4);

    let db = Database::open(&path).await.unwrap();
    let before = page_bytes(&db, head).await;
    let (offset, length) = slot0_fields(&before);
    assert_eq!(
        length, 4,
        "the fixture must have shortened slot 0's payload"
    );

    let err = update_version_header_in_data_page(
        &db.buffer_pool,
        RowId::new(head, 0),
        VersionHeader::new(1, None),
        &[],
    )
    .await
    .expect_err("a payload shorter than the version header must be reported as an error");
    let message = err.to_string();
    assert!(
        message.contains(&format!("corrupted data page {head}")),
        "the error must name the damaged page {head}: {message}"
    );
    assert!(
        message.contains("slot 0"),
        "the error must name the offending slot: {message}"
    );
    assert!(
        message.contains("shorter"),
        "the error must report the short payload: {message}"
    );

    let after = page_bytes(&db, head).await;
    let end = offset as usize + VersionHeader::SIZE;
    assert_eq!(
        &before[offset as usize..end],
        &after[offset as usize..end],
        "the version header write must not touch the bytes beyond the slot's own payload"
    );
}

/// 未损坏对照：UPDATE 的结果与既有行为一致（零变化）。
#[tokio::test]
async fn undamaged_page_updates_are_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let _head = seed_data_page(&path).await;

    let db = Database::open(&path).await.unwrap();

    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_eq!(row_count(&resp), 1);
}

// ===========================================================================
// CLI 会话夹具（同步测试内驱动真实二进制）
// ===========================================================================

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行真实 `rtsql` 二进制（一条 one-shot 会话）并收集退出码与输出。
fn run_cli(db_file: &Path, sql: &str) -> CliOutput {
    let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .arg(db_file)
        .arg(sql)
        .env("RTSQL_HOME", db_file.parent().unwrap())
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql binary");
    CliOutput {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// 同步测试内的异步夹具（tokio 当前线程 runtime）。
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
        .block_on(fut)
}

/// 建库 + 种子数据 + 关库（同步测试内的异步夹具）。
fn block_on_seed(path: &Path) -> u32 {
    block_on(seed_data_page(path))
}
