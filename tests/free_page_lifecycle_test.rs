//! MS35 `free-page-lifecycle` R2/R3：重开时按持久目录重建空闲页集合。
//!
//! change: 2026-10-24-ms35-free-page-reuse-across-restart（Iteration 000）
//!
//! 覆盖面：
//! - A1（R2 场景 1）：`DROP TABLE` 释放且被 checkpoint 持久化的页，在重开后
//!   再次被分配——库文件体积不因新表而增长。
//! - A2（R2 场景 2）：重建不把活表的页（数据页链 / 溢出链 / 索引树）判成
//!   空闲页：活表已提交行仍可查询，`check_integrity` 的 error 级发现为 0。
//! - A3（R2 场景 3）：恢复期重建过索引树的库，重开后内存索引树页与持久
//!   目录登记的树页都不进入可分配页集合。
//!
//! 观察约定沿用既有先例：体积按文件长度换算（`drop_table_free_test.rs` 的
//! 本地 helper）、崩溃模拟沿用进程内丢弃句柄（`.agents/runbooks/
//! in-process-crash-simulation.md` R49）、完整性断言沿用
//! `check_integrity(...).error_count()`。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::free_page_rebuild::arm_free_page_rebuild_fault;
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::page_format::ColumnType;
use rtsql::storage::{AsyncStorage, FileStorage, IoFailPoint, Page, HEADER_SIZE};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// 物理页数（按文件长度换算）：格式头在页空间之外，先扣除；释放的页进入
/// 空闲列表而不截断，故长度即高水位。
fn page_count(path: &Path) -> u64 {
    (std::fs::metadata(path).unwrap().len() - HEADER_SIZE as u64) / Page::PAGE_SIZE as u64
}

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test.db")
}

/// 执行单条 SQL 并断言成功。
async fn ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

/// 断言查询结果行集。
async fn rows(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql} must return rows: {other:?}"),
    }
}

/// 崩溃模拟：WAL 落盘后丢弃句柄（不 close → 脏页不刷、不 checkpoint）。
async fn crash(db: Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

/// 建一张两列的表（`id INT PRIMARY KEY` + `n INT`）。
async fn create_small(db: &Database, name: &str) {
    ok(
        db,
        &format!("CREATE TABLE {name} (id INT PRIMARY KEY, n INT)"),
    )
    .await;
}

// ===========================================================================
// A1（R2 场景 1）：重开后释放的页可再次分配
// ===========================================================================

/// 会话一「建表 `t` → 写入 → 删表 → checkpoint → 关闭」，会话二重开并建表
/// `u`：`u` 占用 `t` 释放的页号，库文件体积不因 `u` 增长。
#[tokio::test]
async fn reopened_database_reuses_released_pages() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let (t_data_page, t_tree_root) = {
        let db = Database::open(&path).await.unwrap();
        create_small(&db, "t").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        let meta = db.get_table("t").await.unwrap();
        let data_page = meta.data_page_head;
        let tree_root = meta.index_manager.root_page_id();
        ok(&db, "DROP TABLE t").await;
        // 释放的持久化判定：目录删除被一次成功完成的 checkpoint 刷盘。
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
        (data_page, tree_root)
    };

    let pages_after_drop = page_count(&path);

    // 会话二：重开 → 建表 `u`。
    let db = Database::open(&path).await.unwrap();
    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();

    assert_eq!(
        page_count(&path),
        pages_after_drop,
        "the reopened database must allocate `u` from the released pages instead of growing the file"
    );
    let released = [t_data_page.0, t_tree_root.0];
    assert!(
        released.contains(&u_meta.data_page_head.0),
        "`u`'s data page {} must come from the released pages {released:?}",
        u_meta.data_page_head.0
    );
    assert!(
        released.contains(&u_meta.index_manager.root_page_id().0),
        "`u`'s index root {} must come from the released pages {released:?}",
        u_meta.index_manager.root_page_id().0
    );

    // 新对象只含新写入的行（分配时清零，不读到已释放页上的旧行）。
    ok(&db, "INSERT INTO u VALUES (7, 70)").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM u").await,
        vec![vec![serde_json::json!(7), serde_json::json!(70)]]
    );
    db.close().await.unwrap();
}

// ===========================================================================
// A2（R2 场景 2）：重建不把活页判成空闲页
// ===========================================================================

/// 库中存有活表 `t`（数据页链 + 溢出链 + 索引树页）：重开后新建表不占用
/// `t` 的任何页，`t` 的已提交行仍可查询，`check_integrity` 的 error 级
/// 发现数为 0。
#[tokio::test]
async fn rebuild_never_marks_live_pages_free() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let (live_data_page, live_tree_root) = {
        let db = Database::open(&path).await.unwrap();
        // A released table first: its pages become the allocatable set after
        // the reopen, so "the live table's pages stay out of that set" is an
        // observable (the new table would otherwise take them).
        create_small(&db, "gone").await;
        ok(&db, "INSERT INTO gone VALUES (1, 20)").await;
        ok(&db, "DROP TABLE gone").await;
        ok(&db, "CREATE TABLE live (id INT PRIMARY KEY, s STRING)").await;
        // A row beyond the inline capacity → overflow chain pages; a short
        // tuple → inline slot.
        let big = "x".repeat(5000);
        ok(
            &db,
            &format!("INSERT INTO live VALUES (1, '{big}'), (2, 'small')"),
        )
        .await;
        let meta = db.get_table("live").await.unwrap();
        let data_page = meta.data_page_head;
        let tree_root = meta.index_manager.root_page_id();
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
        (data_page, tree_root)
    };

    let db = Database::open(&path).await.unwrap();

    // 活表的页不被交给新对象。
    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();
    assert_ne!(
        u_meta.data_page_head.0, live_data_page.0,
        "the new table must not take the live table's data page"
    );
    assert_ne!(
        u_meta.index_manager.root_page_id().0,
        live_tree_root.0,
        "the new table must not take the live table's index tree root"
    );

    // 活表的已提交行完整可读（含溢出链上的长元组）。
    let live_rows = rows(&db, "SELECT id, s FROM live ORDER BY id").await;
    assert_eq!(live_rows.len(), 2, "both committed rows must survive");
    assert_eq!(live_rows[0][0], serde_json::json!(1));
    assert_eq!(
        live_rows[0][1].as_str().map(|s| s.len()),
        Some(5000),
        "the overflow-backed row must read back in full"
    );
    assert_eq!(live_rows[1][1], serde_json::json!("small"));

    // 重建没有破坏任何结构：error 级发现为 0。
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the rebuild: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
    db.close().await.unwrap();
}

// ===========================================================================
// A3（R2 场景 3）：重建包含恢复期重建的索引树页
// ===========================================================================

/// 打开时 WAL 未决、恢复流程重建了某表的索引树：内存索引树占用的页与持久
/// 目录登记的索引树占用的页都计入活页集合，两者都不进入可分配页集合。
#[tokio::test]
async fn recovery_rebuilt_index_pages_are_not_reallocated() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    // 会话一：目录变更被 checkpoint 持久化，行留在 WAL 里（崩溃前未落盘）。
    let persisted_tree_root = {
        let db = Database::open(&path).await.unwrap();
        create_small(&db, "t").await;
        let meta = db.get_table("t").await.unwrap();
        let tree_root = meta.index_manager.root_page_id();
        db.checkpoint().await.unwrap();
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        crash(db).await;
        tree_root
    };

    // 会话二：打开时 WAL 未决 → 恢复重放并重建索引树（内存根 ≠ 目录登记根）。
    let db = Database::open(&path).await.unwrap();
    let meta = db.get_table("t").await.unwrap();
    let memory_tree_root = meta.index_manager.root_page_id();
    assert_ne!(
        memory_tree_root.0, persisted_tree_root.0,
        "the recovery must have rebuilt the index tree under a new root"
    );
    assert_eq!(
        rows(&db, "SELECT id, n FROM t ORDER BY id").await,
        vec![
            vec![serde_json::json!(1), serde_json::json!(10)],
            vec![serde_json::json!(2), serde_json::json!(20)],
        ],
        "the replayed rows must be readable through the rebuilt tree"
    );

    // 新表不得占用任一棵树的页。
    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();
    for (role, page) in [
        ("memory tree root", memory_tree_root.0),
        ("persisted tree root", persisted_tree_root.0),
    ] {
        assert_ne!(
            u_meta.data_page_head.0, page,
            "the new table must not take the {role} page"
        );
        assert_ne!(
            u_meta.index_manager.root_page_id().0,
            page,
            "the new table must not take the {role} page"
        );
    }

    // 重建后的树仍可写入与读回。
    ok(&db, "INSERT INTO t VALUES (3, 30)").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM t ORDER BY id").await,
        vec![
            vec![serde_json::json!(1), serde_json::json!(10)],
            vec![serde_json::json!(2), serde_json::json!(20)],
            vec![serde_json::json!(3), serde_json::json!(30)],
        ]
    );
    db.close().await.unwrap();
}

// ===========================================================================
// A4（R3 场景 1）：重建的失败安全
// ===========================================================================

/// 重建过程中出现页读失败：打开仍成功、活表仍可读、`check_integrity` 的
/// error 级发现为 0，且本次打开没有可分配页（不复用任何释放空间）。
#[tokio::test]
async fn rebuild_failure_keeps_live_pages_intact() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    // 会话一：一张活表 + 一张被删的表（后者的页在重建成功时可被复用）。
    {
        let db = Database::open(&path).await.unwrap();
        create_small(&db, "live").await;
        ok(&db, "INSERT INTO live VALUES (1, 10)").await;
        create_small(&db, "gone").await;
        ok(&db, "INSERT INTO gone VALUES (1, 20)").await;
        ok(&db, "DROP TABLE gone").await;
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
    }
    let pages_before = page_count(&path);

    // 武装一次页读失败：本次打开的重建在首次页读处告终。
    arm_free_page_rebuild_fault(&path);

    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db, "SELECT id, n FROM live").await,
        vec![vec![serde_json::json!(1), serde_json::json!(10)]],
        "the open must succeed and the live table must stay readable"
    );
    create_small(&db, "u").await;
    assert!(
        page_count(&path) > pages_before,
        "an abandoned rebuild must not reuse any released space"
    );
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "the abandoned rebuild must not damage the database: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
    db.close().await.unwrap();
    drop(db);

    // 注入点被本次打开的重建消费（一次性）：下一次打开重建成功，释放的空间
    // 重新可用。
    {
        let db = Database::open(&path).await.unwrap();
        let pages_before_reuse = page_count(&path);
        create_small(&db, "v").await;
        assert_eq!(
            page_count(&path),
            pages_before_reuse,
            "the next open must rebuild successfully and reuse the released space"
        );
        assert!(
            pages_before_reuse > pages_before,
            "the first open must have grown the file (no reuse while the rebuild was abandoned)"
        );
        db.close().await.unwrap();
    }
}

// ===========================================================================
// R1 / R4（Iteration 001）：释放的持久化判定
// ===========================================================================

/// R1 场景 1：`DROP TABLE` 后**未 checkpoint**——同一会话新建表**不得**占用
/// 被释放的页（可观察：新建表的数据页号 ≠ 被释放的页号，且库文件页数上界
/// 增长）。
#[tokio::test]
async fn released_page_not_allocatable_before_release_durable() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    create_small(&db, "t").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    let meta = db.get_table("t").await.unwrap();
    let t_data_page = meta.data_page_head.0;
    let t_tree_root = meta.index_manager.root_page_id().0;
    let pages_before_drop = page_count(&path);

    ok(&db, "DROP TABLE t").await;

    // 未 checkpoint ⇒ 释放在崩溃边界上尚未成立 ⇒ 被释放的页不得可分配。
    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();
    assert_ne!(
        u_meta.data_page_head.0, t_data_page,
        "the new table must not take the released data page {t_data_page}"
    );
    assert_ne!(
        u_meta.index_manager.root_page_id().0,
        t_tree_root,
        "the new table must not take the released index tree root {t_tree_root}"
    );
    assert!(
        page_count(&path) > pages_before_drop,
        "the file must grow while the release is not persisted yet"
    );
    db.close().await.unwrap();
}

/// R1 场景 2：`DROP TABLE` 后执行一次**成功** checkpoint——同一会话新建表
/// **必须**占用被释放的页（可观察：库文件页数上界不增长，且新建表的数据页号
/// ∈ 被释放的页集合）。
#[tokio::test]
async fn released_page_allocatable_after_release_durable() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    create_small(&db, "t").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    let meta = db.get_table("t").await.unwrap();
    let released = [meta.data_page_head.0, meta.index_manager.root_page_id().0];
    let pages_before_drop = page_count(&path);

    ok(&db, "DROP TABLE t").await;
    // 释放的持久化判定：目录删除被一次成功完成的 checkpoint 刷盘。
    db.checkpoint().await.unwrap();

    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();
    assert_eq!(
        page_count(&path),
        pages_before_drop,
        "the new table must reuse the released pages instead of growing the file"
    );
    assert!(
        released.contains(&u_meta.data_page_head.0),
        "the new table's data page {} must come from the released pages {released:?}",
        u_meta.data_page_head.0
    );
    assert!(
        released.contains(&u_meta.index_manager.root_page_id().0),
        "the new table's index root {} must come from the released pages {released:?}",
        u_meta.index_manager.root_page_id().0
    );
    db.close().await.unwrap();
}

/// R4 场景「释放未持久时崩溃后旧表内容完整」（`free-page-lifecycle`）与
/// `drop-persistence-boundary` R1（MODIFIED）场景「释放未持久时的分配不摧毁旧
/// 表内容」：`DROP TABLE` 成功返回之后、**任何后续 DDL 语句成功返回之前**崩溃
/// （进程内丢弃句柄，不 `close()`）——重开后旧表 `t` 仍存在且原行可查询，未被
/// 该语句触及的活表 `other` 的行可查询；未完成的释放**不是**可分配页（新表不
/// 占用旧表的页）；`check_integrity` 的 error 级发现数为 0。
///
/// 崩溃点为何取在 DDL 边界之前：DDL 语句边界的有序刷盘（`src/storage/catalog.rs`
/// 的 `flush_commit_point` → `flush_all_except`）会把该语句的提交点与其它脏页
/// 一并落盘，先前的 `DROP TABLE` 目录删除因而随之落盘——该形态下重开后旧表不
/// 再存在，不属于本场景的崩溃点。此处以「未完成的释放不是可分配页」这一可观
/// 察量承载 R4。
#[tokio::test]
async fn crash_with_pending_release_keeps_old_table_rows() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    // 会话一：把「已提交内容」真正写进磁盘文件（WAL 被 checkpoint 截断）——
    // 否则重开时的 WAL 重放会把行补回来，缺口会被掩盖。
    let (t_data_page, t_tree_root) = {
        let db = Database::open(&path).await.unwrap();
        create_small(&db, "t").await;
        ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)").await;
        // 另一张未被该 DROP 触及的活表：修正后的场景要求它的行在崩溃后仍可查询。
        create_small(&db, "other").await;
        ok(&db, "INSERT INTO other VALUES (1, 100)").await;
        let meta = db.get_table("t").await.unwrap();
        let pages = (meta.data_page_head.0, meta.index_manager.root_page_id().0);
        db.close().await.unwrap();
        drop(db);
        pages
    };

    // 会话二：释放已发生但未持久化（无 checkpoint）→ 崩溃。
    {
        let db = Database::open(&path).await.unwrap();
        ok(&db, "DROP TABLE t").await;
        crash(db).await;
    }

    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db, "SELECT id, n FROM t ORDER BY id").await,
        vec![
            vec![serde_json::json!(1), serde_json::json!(10)],
            vec![serde_json::json!(2), serde_json::json!(20)],
        ],
        "the dropped table must stay intact after a crash with a pending release"
    );
    // 未被该 DROP 触及的活表：其已提交行同样可查询。
    assert_eq!(
        rows(&db, "SELECT id, n FROM other").await,
        vec![vec![serde_json::json!(1), serde_json::json!(100)]],
        "an untouched table must stay intact after a crash with a pending release"
    );
    // 未完成的释放不是可分配页：重开后的新表拿不到旧表的页。
    create_small(&db, "v").await;
    let v_meta = db.get_table("v").await.unwrap();
    assert_ne!(
        v_meta.data_page_head.0, t_data_page,
        "the new table must not take the old table's data page {t_data_page}"
    );
    assert_ne!(
        v_meta.index_manager.root_page_id().0,
        t_tree_root,
        "the new table must not take the old table's index tree root {t_tree_root}"
    );
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the crash: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
    db.close().await.unwrap();
}

/// R1 场景 3（失败安全）：checkpoint 的刷盘失败（`IoFailPoint::DataWrite`）
/// ——被释放的页**不得**被提升为可分配（可观察：失败后同一会话新建表不占用
/// 被释放的页），且数据库仍可正常使用、`check_integrity` 的 error 级发现数
/// 为 0。
#[tokio::test]
async fn flush_failure_does_not_promote_pending_release() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    create_small(&db, "t").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    let meta = db.get_table("t").await.unwrap();
    let t_data_page = meta.data_page_head.0;
    let t_tree_root = meta.index_manager.root_page_id().0;

    ok(&db, "DROP TABLE t").await;
    db.arm_io_failure(IoFailPoint::DataWrite);
    let err = db.checkpoint().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (write_page)"),
        "the checkpoint must fail with the injected write marker: {err}"
    );

    // 刷盘失败 ⇒ 待释放项被丢弃（不提升）⇒ 新建表拿不到被释放的页。
    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();
    assert_ne!(
        u_meta.data_page_head.0, t_data_page,
        "the new table must not take the released data page {t_data_page}"
    );
    assert_ne!(
        u_meta.index_manager.root_page_id().0,
        t_tree_root,
        "the new table must not take the released index tree root {t_tree_root}"
    );

    // 数据库仍可正常使用。
    ok(&db, "INSERT INTO u VALUES (1, 1)").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM u").await,
        vec![vec![serde_json::json!(1), serde_json::json!(1)]]
    );

    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the flush failure: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
    db.close().await.unwrap();
}

// ===========================================================================
// 存储接缝：注入的可分配页集合经既有弹出路径生效
// ===========================================================================

/// 存储层接缝：注入的页集合成为可分配页集合，`allocate_page` 依次弹出它们
///（既有弹出路径不变）。
#[tokio::test]
async fn injected_free_pages_become_the_allocatable_set() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("seam.db");
    let storage = FileStorage::open(&path).unwrap();

    let allocated: Vec<u64> = {
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(storage.allocate_page().await.unwrap().0);
        }
        ids
    };

    storage.inject_free_pages(allocated.clone());

    let mut taken = Vec::new();
    for _ in 0..3 {
        taken.push(storage.allocate_page().await.unwrap().0);
    }
    taken.sort_unstable();
    let mut expected = allocated;
    expected.sort_unstable();
    assert_eq!(
        taken, expected,
        "the injected pages must be the allocatable ones"
    );
}

// ===========================================================================
// 会话内释放时机（Iteration 001 改动）：释放持久化之后才可复用
// ===========================================================================

/// 同会话内 `DROP TABLE` 释放的页：**释放未持久化前**不得可分配；**一次成功
/// checkpoint 之后**可被新表复用（MS35 `free-page-lifecycle` R1）。
#[tokio::test]
async fn same_session_release_reuses_pages_after_release_durable() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    create_small(&db, "t").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    let count_after_create = page_count(&path);
    let meta = db.get_table("t").await.unwrap();
    let released = [meta.data_page_head.0, meta.index_manager.root_page_id().0];

    db.table_manager.drop_table("t").await.unwrap();
    assert_eq!(page_count(&path), count_after_create);

    // 未持久化：新建表拿不到被释放的页（文件页数上界增长）。
    create_small(&db, "u").await;
    let u_meta = db.get_table("u").await.unwrap();
    assert!(
        u_meta.data_page_head.0 >= count_after_create,
        "the new table must not take the released data page {} before the release is durable",
        released[0]
    );
    assert!(
        page_count(&path) > count_after_create,
        "the file must grow while the release is not persisted yet"
    );

    // 释放持久化（一次成功 checkpoint）之后：新建表复用被释放的页。
    db.checkpoint().await.unwrap();
    let count_before_reuse = page_count(&path);
    create_small(&db, "v").await;
    let v_meta = db.get_table("v").await.unwrap();
    assert!(
        released.contains(&v_meta.data_page_head.0),
        "the new table's data page {} must come from the released pages {released:?}",
        v_meta.data_page_head.0
    );
    assert_eq!(
        page_count(&path),
        count_before_reuse,
        "the same session must reuse the released pages once the release is durable"
    );
    db.close().await.unwrap();
}

/// 只读打开零写入：只读会话不做重建（无 WAL / 恢复 / 空闲集合注入）。
#[tokio::test]
async fn readonly_open_does_not_rebuild_free_pages() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        create_small(&db, "t").await;
        ok(&db, "INSERT INTO t VALUES (1, 10)").await;
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
    }
    let before = std::fs::metadata(&path).unwrap().len();
    let db = Database::open_readonly(&path).await.unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        before,
        "a read-only open must not write to the database file"
    );
    assert_eq!(
        rows(&db, "SELECT id, n FROM t").await,
        vec![vec![serde_json::json!(1), serde_json::json!(10)]]
    );
}

/// 内存库行为不变：`:memory:` 无文件、无重建。
#[tokio::test]
async fn in_memory_database_is_unaffected() {
    let db = Database::open_in_memory().await.unwrap();
    create_small(&db, "t").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM t").await,
        vec![vec![serde_json::json!(1), serde_json::json!(10)]]
    );
    let meta = db.get_table("t").await.unwrap();
    assert_eq!(meta.columns[1].1, ColumnType::Int);
}
