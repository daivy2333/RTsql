//! 数据页组的有界槽读——change 2026-10-25-ms37-defect-ledger-closure
//! Iteration 001 / Cycle 000（T3）与 Cycle 001（T17）。
//!
//! 缺陷（T3）：`SlottedPageRef::get_slot_data`
//! （`src/storage/page_format/slotted_page.rs`）对页字节做**无范围检查**的
//! 切片 ⇒ 损坏数据页上的槽偏移 / 长度越界 ⇒ **越界切片 panic** ⇒ 读 / 写
//! 热路径上进程 abort；损坏页上的行还可能被当作「没有这一行」静默跳过。
//!
//! 缺陷（T17）：**页头**声称的槽数超出槽目录容量时，槽目录定位
//! `PAGE_SIZE - (index + 1) * Slot::SIZE` 整数下溢 ⇒ 取槽即越界切片
//! panic ⇒ 进程 abort（ISS35）。
//!
//! 本文件承载先行测试与对照：
//! - 真实文件库 + 真实 `Database::open` + 真实 SQL 入口；
//! - 损坏注入 = **真实文件字节改写**（T3：把槽目录项 0 的 offset 字段写成
//!   `0xFFFF`；T17：把页头的 `slot_count` 字段写成越界值）——不是替身 / 桩；
//! - 观察对象 = 语句返回的错误与进程是否存活（panic ⇒ 用例失败）。

#![cfg(feature = "network")]
use std::path::{Path, PathBuf};

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::{RowId, Slot};
use rtsql::storage::{update_version_header_in_data_page, Page, PageId, HEADER_SIZE};
use rtsql::transaction::{Snapshot, VersionHeader};
use tempfile::TempDir;

/// 数据页的页类型字节（页首）——`0x03` = 数据页。
const DATA_PAGE_TYPE: u8 = 0x03;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("data_page_read.db")
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

fn page_file_offset(page: u32, in_page: usize) -> u64 {
    (HEADER_SIZE + page as usize * Page::PAGE_SIZE + in_page) as u64
}

/// 把某页某槽的 offset 字段改成越界值（`0xFFFF` 超过页长）——真实改坏页字节。
fn corrupt_slot_offset(path: &Path, page: u32, slot_index: usize) {
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

/// 把某页的页头 `slot_count` 改成越界值——真实改坏页字节（T17 / ISS35）。
///
/// 槽目录自页尾向上生长，每项 `Slot::SIZE`（6）字节：页头声称的槽数超出
/// 槽目录容量（`(PAGE_SIZE - HEADER) / Slot::SIZE` = 680）时，槽目录定位
/// `PAGE_SIZE - (index + 1) * Slot::SIZE` 整数下溢 ⇒ 取槽即越界切片 panic。
fn corrupt_page_slot_count(path: &Path, page: u32, slot_count: u16) {
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

/// 损坏的数据页必须让语句以**明确的错误**结束——错误文案点名损坏页与该槽。
///
/// 关键：不得退化为「行不存在 / 表为空」的空结果，也不得 panic。
fn assert_damaged_data_page_error(resp: &Response, page_id: u32) {
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains(&format!("corrupted data page {page_id}")),
                "the error must name the damaged page {page_id}: {message}"
            );
            assert!(
                message.contains("slot 0"),
                "the error must name the offending slot: {message}"
            );
        }
        other => {
            panic!("a damaged data page must fail the statement with a named error, got {other:?}")
        }
    }
}

/// 夹具：建库 → 建表 `t` → 插入 2 行 → checkpoint → 取数据页号 → 关库。
///
/// 返回数据页号（`data_page_head`）。两行落在同一张数据页上（数据页可容纳
/// ~90 行），槽 0 = 第一行（`id = 1`）。
async fn seed_data_page(path: &Path) -> u32 {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
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

// ===========================================================================
// 损坏的数据页：读 / 写路径显式拒绝（不 panic、不 abort、不静默错读）
// ===========================================================================

/// 点查（PK 索引下降 → 取行）：损坏的数据页必须让语句以点名错误结束。
#[tokio::test]
async fn damaged_data_page_makes_point_query_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_damaged_data_page_error(&resp, head);
}

/// 全表扫描（数据页通道）：损坏的数据页必须让语句以点名错误结束。
#[tokio::test]
async fn damaged_data_page_makes_table_scan_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_damaged_data_page_error(&resp, head);
}

/// 写路径：UPDATE / DELETE / CREATE INDEX 回填 / ALTER TABLE 改写前收集，
/// 都必须在损坏的数据页上以点名错误结束（不得静默跳过该行）。
#[tokio::test]
async fn damaged_data_page_makes_writes_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();

    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_damaged_data_page_error(&resp, head);

    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 1").await);
    assert_damaged_data_page_error(&resp, head);

    let resp = to_response(db.execute_sql("CREATE INDEX idx_t_v ON t (v)").await);
    assert_damaged_data_page_error(&resp, head);

    let resp = to_response(db.execute_sql("ALTER TABLE t DROP COLUMN v").await);
    assert_damaged_data_page_error(&resp, head);
}

/// A3：页损坏错误**不得**被 ALTER TABLE 的计划期调用点吞成「表为空」——
/// 失真放行会让 `ADD COLUMN ... NOT NULL`（无 DEFAULT）在非空表上通过计划期
/// 判定。损坏时语句必须以点名错误结束。
#[tokio::test]
async fn damaged_data_page_is_not_swallowed_by_the_alter_table_plan_stage() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();
    let resp = to_response(
        db.execute_sql("ALTER TABLE t ADD COLUMN w INT NOT NULL")
            .await,
    );
    assert_damaged_data_page_error(&resp, head);
}

/// A2：缓冲池的**布尔型页辅助**（`check_page_all_visible`）在损坏槽上必须
/// 降级为安全值（非「全可见」）⇒ 走慢路径 ⇒ 错误由实际取行处抛出。
#[tokio::test]
async fn damaged_data_page_degrades_the_boolean_page_helper_to_the_safe_value() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();
    // 高水位远高于任何事务 id ⇒ 健康页上该辅助返回「全可见」。
    let snapshot = Snapshot::new(1_000_000, vec![]);
    let all_visible = db
        .buffer_pool
        .check_page_all_visible(PageId(head as u64), &snapshot)
        .await;
    assert!(
        !all_visible,
        "the boolean page helper must degrade to the safe value (not all-visible) on a damaged slot"
    );

    // 随后的实际取行以点名错误结束（错误由取行处抛出，不在辅助处）。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_damaged_data_page_error(&resp, head);
}

// ===========================================================================
// 页头槽数越界（页级形态，T17 / ISS35）：取槽之前 ⇒ 明确错误
// ===========================================================================

/// 页头槽数越界的损坏页必须让语句以**明确的错误**结束——错误文案点名该页、
/// 页头声称的槽数与槽目录容量上限。
///
/// 关键：不得退化为「行不存在 / 表为空」的空结果，也不得 panic / abort。
fn assert_slot_count_corruption_error(resp: &Response, page_id: u32, slot_count: u16) {
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains(&format!("corrupted data page {page_id}")),
                "the error must name the damaged page {page_id}: {message}"
            );
            assert!(
                message.contains(&format!("slot count {slot_count}")),
                "the error must name the slot count the page header claims: {message}"
            );
            assert!(
                message.contains("680 slots"),
                "the error must name the slot directory capacity: {message}"
            );
        }
        other => panic!(
            "a page whose header claims too many slots must fail the statement with a named error, got {other:?}"
        ),
    }
}

/// 页头声称的槽数超出槽目录容量 ⇒ 全表扫描与点查都必须以**点名该页与页头
/// 声称槽数**的错误结束（不 panic、不 abort、不静默跳过）。
#[tokio::test]
async fn corrupted_page_slot_count_makes_reads_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_page_slot_count(&path, head, 700);

    let db = Database::open(&path).await.unwrap();

    // 布尔型页辅助在页头损坏时必须降级为安全值（非「全可见」）⇒ 走慢路径，
    // 错误由随后的实际取行处抛出（布尔辅助自身不报页损坏）。
    let snapshot = Snapshot::new(1_000_000, vec![]);
    assert!(
        !db.buffer_pool
            .check_page_all_visible(PageId(head as u64), &snapshot)
            .await,
        "the boolean page helper must degrade to the safe value (not all-visible) on a corrupted header"
    );

    // 全表扫描。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_slot_count_corruption_error(&resp, head, 700);

    // 点查。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_slot_count_corruption_error(&resp, head, 700);
}

/// 可写变体的取槽点（`update_version_header_in_data_page`）：指向该页上
/// **不存在**的 `slot_id` 时，取槽会遍历到页头声称的槽数末端 ⇒ 槽目录
/// 定位下溢 ⇒ abort。损坏页上必须返回点名该页的错误（不 abort）。
#[tokio::test]
async fn corrupted_page_slot_count_makes_the_writable_slot_lookup_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    corrupt_page_slot_count(&path, head, 700);

    let db = Database::open(&path).await.unwrap();
    // 该页上不存在的 logical id ⇒ 取槽遍历到页头声称的槽数末端。
    let row_id = RowId::new(head, 999);
    let err = update_version_header_in_data_page(
        &db.buffer_pool,
        row_id,
        VersionHeader::new(1, None),
        &[],
    )
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

// ===========================================================================
// 未损坏对照：读 / 写行为零变化
// ===========================================================================

#[tokio::test]
async fn undamaged_data_page_reads_and_writes_are_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let head = seed_data_page(&path).await;

    // 对照组不改动任何字节。
    let db = Database::open(&path).await.unwrap();

    // 布尔辅助在健康页上仍返回「全可见」（对照组证明上面的降级断言非空洞）。
    let snapshot = Snapshot::new(1_000_000, vec![]);
    assert!(
        db.buffer_pool
            .check_page_all_visible(PageId(head as u64), &snapshot)
            .await,
        "an undamaged committed page must stay all-visible"
    );

    // 点查。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_eq!(row_count(&resp), 1);

    // 全表扫描。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 2);

    // UPDATE：命中，影响 1 行；旧值不再命中、新值命中。
    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_eq!(row_count(&resp), 1);

    // INSERT：影响 1 行；表多一行。
    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (3, 3)").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 3);

    // DELETE：命中，影响 1 行；表少一行。
    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 2").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 2);

    // ALTER TABLE 的计划期判定在健康表上照旧（非空表拒绝 NOT NULL 无 DEFAULT）。
    let resp = to_response(
        db.execute_sql("ALTER TABLE t ADD COLUMN w INT NOT NULL")
            .await,
    );
    match resp {
        Response::Error { message, .. } => assert!(
            message.contains("without DEFAULT"),
            "the plan-stage rejection must be unchanged on an undamaged table: {message}"
        ),
        other => panic!("expected the plan-stage rejection, got {other:?}"),
    }
}
