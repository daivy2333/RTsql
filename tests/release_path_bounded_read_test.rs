//! 释放路径有界读——change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
//! Iteration 002 / Cycle 000（承载缺陷 B-4）。
//!
//! 缺陷：`drop_table` / `truncate_table` / `DROP INDEX` 的释放扫描直接读
//! `SlottedPageRef::get_slot_data`（`src/storage/page_format/slotted_page.rs:125`，
//! 无边界检查）→ 损坏页的槽位偏移越界 → 切片 panic → 进程 abort。
//!
//! 见证矩阵：
//! - 三条释放路径在**损坏页**上不 panic：按释放路径既有的尽力而为语义结束——
//!   损坏页被计数并报出（告警含页号与违规数），不可读的部分被跳过，语句成功；
//! - 损坏页的溢出链 / 索引子树**不被释放**（损坏被检测到，而不是被当作健康页
//!   照常释放）——「检测到损坏」与「忽略损坏」是两回事；
//! - 未损坏库上的释放页集合与既有行为一致（不遗漏、不重复）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::overflow;
use rtsql::storage::page_format::{SlottedPageRef, Slot};
use rtsql::storage::{Page, PageId, HEADER_SIZE, INTERNAL_NODE};
use rtsql::transaction::VersionHeader;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("release.db")
}

/// 语句执行——断言成功（`Response::Error` 即失败）。
async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement must succeed: {sql:?} -> {resp:?}"
    );
}

/// 页内槽目录项的字节布局 `[logical_id:2][offset:2][length:2]`；第 `index` 项
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

/// 读出某页首槽的溢出 locator 并展开整条链。
async fn overflow_chain_of(db: &Database, page: u32) -> Vec<u32> {
    let first = db
        .buffer_pool
        .with_page_data(PageId(page as u64), |data| {
            let slotted = SlottedPageRef::new(data);
            let slot = slotted
                .get_slot(0)
                .expect("fixture: the head page holds the overflow locator slot");
            let start = slot.offset as usize;
            let end = start + slot.length as usize;
            Ok(overflow::decode_locator(&data[start + VersionHeader::SIZE..end])
                .map(|(_, first)| first))
        })
        .await
        .unwrap()
        .expect("fixture: the row must reference an overflow chain");
    overflow::collect_overflow_pages(&db.buffer_pool, first)
        .await
        .unwrap()
}

/// 夹具 A：单行大 value 的表 → 头页唯一槽位即溢出 locator。
/// 返回 (数据页头页号, 溢出链页号, PK 树根页号)。
async fn seed_overflow_table(path: &Path) -> (u32, Vec<u32>, u32) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v STRING(65535))").await;
    exec_ok(
        &db,
        &format!("INSERT INTO t VALUES (1, '{}')", "x".repeat(10_000)),
    )
    .await;
    db.checkpoint().await.unwrap();
    let meta = db.table_manager.get_table("t").await.unwrap();
    let head = meta.data_page_head.0 as u32;
    let pk_root = meta.index_manager.root_page_id().0 as u32;
    let chain = overflow_chain_of(&db, head).await;
    db.close().await.unwrap();
    drop(db);
    (head, chain, pk_root)
}

/// 夹具 B：带二级索引的表（150 行）→ 二级索引树根为内部结点（叶容量 92）。
/// 返回 (二级索引根页号, 数据页头页号, PK 树根页号)。
async fn seed_index_table(path: &Path) -> (u32, u32, u32) {
    let db = Database::open(path).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "CREATE INDEX idx_v ON t (v)").await;
    for i in 0..150i32 {
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, {i})")).await;
    }
    db.checkpoint().await.unwrap();
    let meta = db.table_manager.get_table("t").await.unwrap();
    let head = meta.data_page_head.0 as u32;
    let pk_root = meta.index_manager.root_page_id().0 as u32;
    let sec_root = meta.secondary_indexes[0].index.root_page_id().0 as u32;
    db.close().await.unwrap();
    drop(db);
    (sec_root, head, pk_root)
}

/// 观测释放路径实际释放的页集合：提升 → 取页 → 收集被复用的页号
/// （与 `tests/index_page_ownership_test.rs` 的库级释放集合观测同法）。
async fn observe_released_pages(db: &Database) -> BTreeSet<u64> {
    let storage = db.buffer_pool.storage().clone();
    let markers = storage.release_completion_limit();
    storage.promote_free_releases(markers);
    let hwm = storage.page_count();
    let mut released = BTreeSet::new();
    for _ in 0..4096 {
        let id = storage.allocate_page().await.unwrap().0;
        if id >= hwm {
            break;
        }
        released.insert(id);
    }
    released
}

/// 走真实 CLI 入口执行一条 DDL，返回 (退出码, stderr)。
fn run_cli_ddl(dir: &TempDir, path: &Path, sql: &str) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .arg(path)
        .arg(sql)
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .output()
        .expect("rtsql CLI must run");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

// ===========================================================================
// 损坏页上的三条释放路径：不 panic、不 abort、损坏被计数并报出
// ===========================================================================

#[tokio::test]
async fn drop_table_on_damaged_data_page_does_not_panic() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (head, chain, pk_root) = seed_overflow_table(&path).await;
    assert!(
        chain.len() >= 2,
        "fixture precondition: the 10KB row spans several chain pages, got {chain:?}"
    );
    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();
    let meta = db.table_manager.get_table("t").await.unwrap();
    assert_eq!(
        meta.data_page_head.0 as u32,
        head,
        "precondition: the damaged page is still the table's head page"
    );

    exec_ok(&db, "DROP TABLE t").await;

    let released = observe_released_pages(&db).await;
    let mut expected = BTreeSet::new();
    expected.insert(head as u64);
    expected.insert(pk_root as u64);
    assert_eq!(
        released, expected,
        "the damaged page's locators must be skipped: its overflow chain is not \
         released (chain pages {chain:?}), everything else is released as before"
    );
}

#[tokio::test]
async fn truncate_on_damaged_data_page_does_not_panic() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (head, chain, pk_root) = seed_overflow_table(&path).await;
    assert!(
        chain.len() >= 2,
        "fixture precondition: the 10KB row spans several chain pages, got {chain:?}"
    );
    corrupt_slot_offset(&path, head, 0);

    let db = Database::open(&path).await.unwrap();

    exec_ok(&db, "TRUNCATE TABLE t").await;

    let released = observe_released_pages(&db).await;
    let expected: BTreeSet<u64> = [pk_root as u64].into_iter().collect();
    assert_eq!(
        released, expected,
        "the damaged page's locators must be skipped: its overflow chain is not \
         released (chain pages {chain:?}); the old PK tree is released as before"
    );
}

#[tokio::test]
async fn drop_index_on_damaged_index_page_does_not_panic() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (sec_root, _head, _pk_root) = seed_index_table(&path).await;

    // 前置：二级索引树根必须是内部结点（释放扫描才读它的槽位）。
    let page_type = std::fs::read(&path).unwrap()
        [page_file_offset(sec_root, 0) as usize] as u8;
    assert_eq!(
        page_type, INTERNAL_NODE,
        "fixture precondition: the secondary index root must be an internal node"
    );
    corrupt_slot_offset(&path, sec_root, 0);

    let db = Database::open(&path).await.unwrap();
    let meta = db.table_manager.get_table("t").await.unwrap();
    assert_eq!(
        meta.secondary_indexes[0].index.root_page_id().0 as u32,
        sec_root,
        "precondition: the damaged page is still the secondary index root"
    );

    exec_ok(&db, "DROP INDEX idx_v").await;

    let released = observe_released_pages(&db).await;
    let expected: BTreeSet<u64> = [sec_root as u64].into_iter().collect();
    assert_eq!(
        released, expected,
        "the damaged internal node's subtree must be skipped: only the damaged \
         root itself is released"
    );
}

// ===========================================================================
// 未损坏对照：释放页集合与既有行为一致（不遗漏、不重复）
// ===========================================================================

#[tokio::test]
async fn undamaged_release_paths_free_the_expected_pages() {
    // DROP TABLE：数据页头 + PK 树根 + 溢出链页。
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (head, chain, pk_root) = seed_overflow_table(&path).await;
    let db = Database::open(&path).await.unwrap();
    exec_ok(&db, "DROP TABLE t").await;
    let released = observe_released_pages(&db).await;
    let mut expected: BTreeSet<u64> = chain.iter().map(|p| *p as u64).collect();
    expected.insert(head as u64);
    expected.insert(pk_root as u64);
    assert_eq!(
        released, expected,
        "the undamaged DROP TABLE must free exactly the table's pages"
    );

    // TRUNCATE：旧 PK 树根 + 溢出链页（头页原地重置，不进释放集）。
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (head, chain, pk_root) = seed_overflow_table(&path).await;
    let db = Database::open(&path).await.unwrap();
    exec_ok(&db, "TRUNCATE TABLE t").await;
    let released = observe_released_pages(&db).await;
    let mut expected: BTreeSet<u64> = chain.iter().map(|p| *p as u64).collect();
    expected.insert(pk_root as u64);
    assert_eq!(
        released, expected,
        "the undamaged TRUNCATE must free exactly the old tree + chain pages \
         (head page {head} is reset in place)"
    );

    // DROP INDEX：二级索引树的全部页。
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (sec_root, _head, _pk_root) = seed_index_table(&path).await;
    let db = Database::open(&path).await.unwrap();
    exec_ok(&db, "DROP INDEX idx_v").await;
    let released = observe_released_pages(&db).await;
    assert!(
        released.contains(&(sec_root as u64)),
        "the undamaged DROP INDEX must free the secondary index root, got {released:?}"
    );
    assert!(
        released.len() >= 2,
        "the undamaged DROP INDEX must free the whole index tree, got {released:?}"
    );
}

// ===========================================================================
// 真实 CLI 入口：损坏页被计数并报出（告警含页号与违规数）
// ===========================================================================

#[tokio::test]
async fn drop_table_cli_reports_the_damaged_page() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (head, chain, _pk_root) = seed_overflow_table(&path).await;
    assert!(!chain.is_empty(), "fixture precondition: an overflow chain exists");
    corrupt_slot_offset(&path, head, 0);

    let (code, stderr) = run_cli_ddl(&dir, &path, "DROP TABLE t");
    assert_eq!(code, 0, "the DDL must not abort: stderr={stderr}");
    assert!(
        stderr.contains(&format!("data page {head} is damaged: 1 violation")),
        "the damaged page must be counted and reported: stderr={stderr}"
    );
}

#[tokio::test]
async fn truncate_cli_reports_the_damaged_page() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (head, chain, _pk_root) = seed_overflow_table(&path).await;
    assert!(!chain.is_empty(), "fixture precondition: an overflow chain exists");
    corrupt_slot_offset(&path, head, 0);

    let (code, stderr) = run_cli_ddl(&dir, &path, "TRUNCATE TABLE t");
    assert_eq!(code, 0, "the DDL must not abort: stderr={stderr}");
    assert!(
        stderr.contains(&format!("data page {head} is damaged: 1 violation")),
        "the damaged page must be counted and reported: stderr={stderr}"
    );
}

#[tokio::test]
async fn drop_index_cli_reports_the_damaged_page() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let (sec_root, _head, _pk_root) = seed_index_table(&path).await;
    corrupt_slot_offset(&path, sec_root, 0);

    let (code, stderr) = run_cli_ddl(&dir, &path, "DROP INDEX idx_v");
    assert_eq!(code, 0, "the DDL must not abort: stderr={stderr}");
    assert!(
        stderr.contains(&format!("index tree page {sec_root} is damaged: 1 violation")),
        "the damaged page must be counted and reported: stderr={stderr}"
    );
}
