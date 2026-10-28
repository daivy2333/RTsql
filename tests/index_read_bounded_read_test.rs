//! 索引树读路径：有界槽读 + 损坏页的显式拒绝——change
//! 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
//! Iteration 006 / Cycle 000（承载缺陷 B-5：索引树页的无界槽读）。
//!
//! 缺陷：`src/storage/btree/node.rs` 的 8 个索引树访问器用**未检查**的
//! `get_slot_data` 切页字节（`src/storage/page_format/slotted_page.rs:125` /
//! `:195`）⇒ 损坏索引页的槽偏移越界 ⇒ **越界切片 panic** ⇒ 读 / 写热路径上
//! 进程 abort。
//!
//! 两套互补机制：
//! 1. 访问器族改为**有界槽读** ⇒ 任何路径都不会 panic；
//! 2. 每次据索引树页字节决定下降 / 取条目 / 取子指针的访问都过
//!    **页级校验** ⇒ 损坏页让语句以**点名损坏页与索引树根页**的明确错误
//!    结束，**绝不**退化为「行不存在」。
//!
//! 见证矩阵（真实文件库 + 真实 `Database::open` + 真实 SQL 入口）：
//! - 损坏的索引页上 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 不 panic、不
//!   abort、不静默错读；
//! - 未损坏库上的同样语句行为零变化。
//!
//! 另含**短叶条目**形态（槽负载长度短于条目格式 38 字节而偏移仍落在页内）：
//! 该条目解不出来，未纳入页级判据时会被当作「没有这个条目」静默跳过 ⇒
//! 点查得到「查不到该行」（静默错读）。纳入页级判据后，读 / 写路径以点名
//! 损坏页与索引树根页的明确错误结束。
//!
//! 释放路径（`DROP TABLE` / `TRUNCATE` / `DROP INDEX`）的处置方向相反
//! （计数 + 告警 + 跳过——Iteration 002 已确立），本文件不触碰它。

#![cfg(feature = "network")]
use std::path::{Path, PathBuf};

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::Slot;
use rtsql::storage::{Page, HEADER_SIZE, INTERNAL_NODE};
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("index_read.db")
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

/// 把某页某槽的 offset 字段改成越界值（0xFFFF 超过页长）——真实改坏页字节。
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

/// 槽目录项的 **length 字段**位置——布局 `[logical_id:2][offset:2][length:2]`，
/// 既有 `slot_offset_field_offset` 返回 offset 字段（起点 +2），length 字段再 +2。
fn slot_length_field_offset(slot_index: usize) -> usize {
    Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE + 4
}

/// 把某页某槽的**负载长度**改成给定值——偏移不动 ⇒ 负载仍落在页内
/// （真实改坏页字节，不是替身）。
///
/// 长度短于条目格式（`MAX_KEY_LEN + RowId::SIZE` = 38 字节）时，槽本身**合法**
/// （边界检查通过），但条目解不出来 ⇒ 这是与「越界槽」不同的缺陷形态。
fn corrupt_slot_length(path: &Path, page: u32, slot_index: usize, new_len: u16) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(page_file_offset(
        page,
        slot_length_field_offset(slot_index),
    )))
    .unwrap();
    f.write_all(&new_len.to_le_bytes()).unwrap();
    f.flush().unwrap();
}

/// 夹具：100 行的表 → PK 索引树为「内部根（1 条分隔键）+ 两个叶」。
///
/// 100 行落在「叶满 92 → 分裂一次」的区间内：根页恰有 1 条分隔键 ⇒ 根页
/// 的槽 0 必被每次下降读到（损坏点因此确定命中）；最左叶持最小键。
///
/// 返回 (PK 树根页号, 最左叶页号)。
async fn seed_pk_tree(path: &Path) -> (u32, u32) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    for i in 1..=100i32 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, {i})")).await;
    }
    db.checkpoint().await.unwrap();
    let meta = db.table_manager.get_table("t").await.unwrap();
    let pk_root = meta.index_manager.root_page_id().0 as u32;
    db.close().await.unwrap();
    drop(db);

    // 前置：根页已是内部结点（叶满 → 分裂 → 根上抬）。
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        bytes[page_file_offset(pk_root, 0) as usize] as u8,
        INTERNAL_NODE,
        "fixture precondition: the PK tree root must be an internal node"
    );
    // 最左叶 = 根页头部的 leftmost_child（页内偏移 5..9）。
    let leftmost = u32::from_le_bytes([
        bytes[page_file_offset(pk_root, 5) as usize],
        bytes[page_file_offset(pk_root, 6) as usize],
        bytes[page_file_offset(pk_root, 7) as usize],
        bytes[page_file_offset(pk_root, 8) as usize],
    ]);
    assert_ne!(
        leftmost, pk_root,
        "fixture precondition: the leftmost leaf differs from the root page"
    );
    (pk_root, leftmost)
}

/// 语句必须以**明确的错误**结束——错误文案同时点名损坏页与索引树根页。
///
/// 关键：不得退化为「行不存在」的空结果，也不得返回别的行。
fn assert_damaged_index_error(resp: &Response, page_id: u32, index_root_page_id: u32) {
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains(&format!("corrupted index page {page_id}")),
                "the error must name the damaged page {page_id}: {message}"
            );
            assert!(
                message.contains(&format!("(index root page {index_root_page_id})")),
                "the error must name the index tree root page {index_root_page_id}: {message}"
            );
        }
        other => {
            panic!("a damaged index page must fail the statement with a named error, got {other:?}")
        }
    }
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

// ===========================================================================
// 损坏的索引页：读 / 写路径显式拒绝（不 panic、不 abort、不静默错读）
// ===========================================================================

#[tokio::test]
async fn damaged_index_page_makes_select_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (pk_root, leftmost_leaf) = seed_pk_tree(&path).await;

    // 损坏点落在点查与全索引扫描真正扫描到的**叶页**上（不是根页）⇒ 错误
    // 文案同时点名两个不同的页号（损坏页 + 索引树根页）。
    corrupt_slot_offset(&path, leftmost_leaf, 0);

    let db = Database::open(&path).await.unwrap();

    // 点查：根页健康 → 下降到最左叶 → 损坏页 ⇒ 明确错误（不是空结果）。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_damaged_index_error(&resp, leftmost_leaf, pk_root);

    // 全索引扫描（复合 PK 谓词 → Filter(Scan) → scan_all 遍历全部叶）⇒
    // 同样以明确错误结束。
    let resp = to_response(
        db.execute_sql("SELECT * FROM t WHERE id = 1 AND v = 1")
            .await,
    );
    assert_damaged_index_error(&resp, leftmost_leaf, pk_root);

    // 无 WHERE 的 SELECT 走数据页通道（M19 路由），本就不读索引树页 ⇒
    // 既有行为未变（返回全部行）。该形态**不可能**命中索引页损坏，不据此
    // 改判据。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(
        row_count(&resp),
        100,
        "a no-WHERE SELECT never reads the index tree: its existing behaviour must be unchanged"
    );
}

#[tokio::test]
async fn damaged_index_page_makes_insert_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (pk_root, _leftmost_leaf) = seed_pk_tree(&path).await;

    // 损坏点落在根页：写路径的下降必经根页 ⇒ 损坏必被命中。
    corrupt_slot_offset(&path, pk_root, 0);

    let db = Database::open(&path).await.unwrap();

    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (101, 101)").await);
    assert_damaged_index_error(&resp, pk_root, pk_root);

    // 拒绝发生在任何写入之前 ⇒ 零副作用（行数不变）。
    // 观测走不读索引树的通道（M19：无 WHERE 的 SELECT 走数据页通道）——
    // 用读索引的查询观测会撞上同一个损坏页而测不到副作用。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(
        row_count(&resp),
        100,
        "a rejected INSERT must not have written the row"
    );
}

#[tokio::test]
async fn damaged_index_page_makes_delete_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (pk_root, _leftmost_leaf) = seed_pk_tree(&path).await;

    corrupt_slot_offset(&path, pk_root, 0);

    let db = Database::open(&path).await.unwrap();

    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 1").await);
    assert_damaged_index_error(&resp, pk_root, pk_root);

    // 拒绝发生在墓碑写之前 ⇒ 零副作用（行仍在）。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(
        row_count(&resp),
        100,
        "a rejected DELETE must not have removed the row"
    );
}

#[tokio::test]
async fn damaged_index_page_makes_update_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (pk_root, _leftmost_leaf) = seed_pk_tree(&path).await;

    corrupt_slot_offset(&path, pk_root, 0);

    let db = Database::open(&path).await.unwrap();

    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_damaged_index_error(&resp, pk_root, pk_root);

    // 拒绝发生在版本写之前 ⇒ 零副作用（旧值仍在、新值未写）。
    // 观测走不读索引树的通道（非 PK 列谓词 → 数据页通道）。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE v = 1").await);
    assert_eq!(
        row_count(&resp),
        1,
        "a rejected UPDATE must not have changed the row"
    );
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE v = 999").await);
    assert_eq!(
        row_count(&resp),
        0,
        "a rejected UPDATE must not have written the new value"
    );
}

// ===========================================================================
// 未损坏对照：读 / 写行为零变化
// ===========================================================================

#[tokio::test]
async fn undamaged_index_reads_and_writes_are_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_pk_tree(&path).await;

    let db = Database::open(&path).await.unwrap();

    // 点查。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_eq!(row_count(&resp), 1);

    // 全索引扫描（复合 PK 谓词 → Filter(Scan)）。
    let resp = to_response(
        db.execute_sql("SELECT * FROM t WHERE id = 1 AND v = 1")
            .await,
    );
    assert_eq!(row_count(&resp), 1);

    // 无 WHERE 的全表扫描。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 100);

    // UPDATE：单行命中，影响 1 行；旧值不再命中、新值命中。
    let resp = to_response(db.execute_sql("UPDATE t SET v = 999 WHERE id = 1").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(
        db.execute_sql("SELECT * FROM t WHERE id = 1 AND v = 1")
            .await,
    );
    assert_eq!(row_count(&resp), 0);
    let resp = to_response(
        db.execute_sql("SELECT * FROM t WHERE id = 1 AND v = 999")
            .await,
    );
    assert_eq!(row_count(&resp), 1);

    // INSERT：新键落位，影响 1 行；表多一行。
    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (101, 101)").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 101);

    // DELETE：命中，影响 1 行；表少一行；被删键不再命中。
    let resp = to_response(db.execute_sql("DELETE FROM t WHERE id = 1").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 100);
    let resp = to_response(
        db.execute_sql("SELECT * FROM t WHERE id = 1 AND v = 999")
            .await,
    );
    assert_eq!(row_count(&resp), 0);
}

// ===========================================================================
// 短叶条目：负载长度短于条目格式（38 字节）而偏移仍落在页内
// ===========================================================================

/// 叶页槽的负载长度短于条目格式时，该条目**解不出来**（`get_key` /
/// `get_row_id` 因长度不足返回 `None`）⇒ 未纳入页级判据时它被当作「没有这个
/// 条目」静默跳过 ⇒ 点查得到「查不到该行」（静默错读）。
///
/// 判据纳入后：读 / 写路径以**点名损坏页与索引树根页**的明确错误结束。
#[tokio::test]
async fn short_leaf_entry_makes_select_fail_with_a_named_error() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (pk_root, leftmost_leaf) = seed_pk_tree(&path).await;

    // 损坏点落在点查真正下降到的**最左叶页**上：槽 0 的负载长度 38 → 10
    // 字节，偏移不动 ⇒ 槽仍落在页内（不是越界槽）。
    corrupt_slot_length(&path, leftmost_leaf, 0, 10);

    let db = Database::open(&path).await.unwrap();

    // 点查：根页健康 → 下降到最左叶 → 短叶条目 ⇒ 明确错误（不是空结果）。
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_damaged_index_error(&resp, leftmost_leaf, pk_root);
}

/// 未损坏库上的同样语句**零变化**（判据扩展不误报健康页）。
#[tokio::test]
async fn short_leaf_entry_does_not_change_undamaged_reads() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    seed_pk_tree(&path).await;

    let db = Database::open(&path).await.unwrap();

    // 读：行集合不变。
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 100);
    let resp = to_response(db.execute_sql("SELECT * FROM t WHERE id = 1").await);
    assert_eq!(row_count(&resp), 1);

    // 写：INSERT 仍成功。
    let resp = to_response(db.execute_sql("INSERT INTO t VALUES (101, 101)").await);
    assert_eq!(affected_rows(&resp), 1);
    let resp = to_response(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(row_count(&resp), 101);
}
