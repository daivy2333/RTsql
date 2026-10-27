//! `index-page-ownership`（IPO）R1/R2/R3——索引页归属唯一性。
//!
//! change: `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence`
//! （Iteration 000）
//!
//! 覆盖面：
//! - IPO R1（被合并掉的树页只释放一次）：一次兄弟合并只发出一次释放完成标记；
//!   被合并掉的页在待释放集合中只出现一次。
//! - IPO R2（被释放的页不同时属于两个对象）：提升后的可分配集合中同一页号不
//!   出现两次；被释放的页不再被树引用。
//! - IPO R3（索引插入路径的下降必然终止）：删列改写在多层索引规模下不耗尽栈；
//!   下降成环时插入返回错误而不是崩溃。
//!
//! 观察约定沿用既有先例：真实文件库 + 真实 `Database::open` / DML
//! （`free_page_lifecycle_test.rs` 的库级夹具形态）；释放的持久化判定沿用
//! 同一文件的存储层入口（`complete_free_release` + `promote_free_releases`，
//! 与 `src/wal/checkpoint.rs` 第 2 步同一条）；树形读面沿用
//! `InternalNodeRef` 的公开读面。

use std::collections::BTreeSet;
use std::sync::Arc;

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::btree::{BTree, InternalNodeRef, SyncPageLoader};
use rtsql::storage::page_format::RowId;
use rtsql::storage::{
    AsyncStorage, BufferPool, FileStorage, InternalNode, PageId, StorageError, INTERNAL_NODE,
    LEAF_NODE,
};
use tempfile::{tempdir, TempDir};

/// 物理页空间的高水位（`FileStorage::page_count`）：已分配过的页号上界。
/// 释放的页号恒 `< hwm`，新分配的页号恒 `>= hwm`。
type HighWaterMark = u64;

// ===========================================================================
// 夹具
// ===========================================================================

/// 真实文件库 + 缓冲池（`btree_merge_test.rs` 的既有形态）。
fn create_test_pool() -> Arc<BufferPool> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    Arc::new(BufferPool::new(1000, storage).unwrap())
}

/// 8 字节整数键（与主键索引的 `Value::Int` 键形态一致）。
fn make_key(i: u32) -> Vec<u8> {
    i.to_be_bytes().to_vec()
}

/// 执行单条 SQL 并断言成功。
async fn ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

/// 主键索引树的**子指针**遍历（真实页读 + `InternalNodeRef` 的公开读面）。
///
/// 返回 (叶数, 树深层数, 被两个节点同时引用的页号)。子指针图必须是一棵树：
/// 同一页号被两个节点引用 ⇒ 页归属被破坏（IPO R2）。
async fn walk_index_tree(pool: &BufferPool, root: PageId) -> (usize, usize, Vec<PageId>) {
    let mut leaves = 0usize;
    let mut depth = 0usize;
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    let mut duplicates: Vec<PageId> = Vec::new();
    let mut current = vec![root];
    loop {
        let mut next = Vec::new();
        for page_id in &current {
            if !seen.insert(page_id.0) {
                duplicates.push(*page_id);
            }
            let guard = pool.get_page(*page_id).await.unwrap();
            let data = guard.page_data();
            if data[0] == LEAF_NODE {
                leaves += 1;
            } else {
                assert_eq!(
                    data[0], INTERNAL_NODE,
                    "index page {} has an unexpected page type {:#04x}",
                    page_id.0, data[0]
                );
                let iref = InternalNodeRef::new(&data);
                next.push(PageId(iref.leftmost_child() as u64));
                for i in 0..iref.key_count() {
                    if let Some(child) = iref.get_child_page_id(i) {
                        next.push(PageId(child as u64));
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        current = next;
        depth += 1;
    }
    (leaves, depth, duplicates)
}

/// 库级夹具：建一张带主键索引的表并写入 `n` 行（一个事务内的多行 INSERT）。
async fn seed_table(db: &Database, table: &str, n: i64) {
    ok(
        db,
        &format!("CREATE TABLE {table} (id INT PRIMARY KEY, n INT)"),
    )
    .await;
    for start in (0..n).step_by(50) {
        let end = (start + 50).min(n);
        let values: Vec<String> = (start..end).map(|i| format!("({i}, {i})")).collect();
        ok(
            db,
            &format!("INSERT INTO {table} VALUES {}", values.join(",")),
        )
        .await;
    }
}

/// 释放集合的断言：可分配集合里同一页号不得出现两次，且每个被释放的页只带
/// 一次释放完成标记。
///
/// 观测不新增指标——可分配集合的成员就是它交给调用方的页号，重复的页号会
/// **两次**被交给两个对象；被释放的页号恒小于提升前的高水位。返回被释放的
/// 页号集合（供「被释放的页不再被树引用」断言使用）。
fn assert_allocatable_set(
    allocated: &[u64],
    hwm: HighWaterMark,
    markers_before: u64,
    markers_after: u64,
) -> BTreeSet<u64> {
    // IPO R2：可分配集合中同一页号不出现两次——同一页号不得被交给两个对象。
    let distinct: BTreeSet<u64> = allocated.iter().copied().collect();
    let repeated: Vec<u64> = allocated
        .iter()
        .copied()
        .filter(|id| allocated.iter().filter(|other| *other == id).count() > 1)
        .collect::<BTreeSet<u64>>()
        .into_iter()
        .collect();
    assert_eq!(
        distinct.len(),
        allocated.len(),
        "the allocatable set must not hand the same page id to two owners \
         ({} allocations, {} distinct, repeated page ids: {repeated:?})",
        allocated.len(),
        distinct.len()
    );

    // IPO R1：每个被释放的页只带一次释放完成标记（标记数 == 被释放的页数）。
    let released: BTreeSet<u64> = allocated.iter().copied().filter(|id| *id < hwm).collect();
    assert_eq!(
        markers_after - markers_before,
        released.len() as u64,
        "each released page must carry exactly one release-completion marker \
         (markers {} for {} released pages)",
        markers_after - markers_before,
        released.len()
    );
    released
}

/// 库级释放集合观测：提升 → 取页 → 断言（异步入口）。
///
/// 可分配集合被取空（取到第一个新页号为止）——否则「被释放的页数」只是
/// 集合的一部分，标记数与页数的对账就不成立。
async fn assert_allocatable_set_has_no_duplicate(
    storage: &Arc<dyn AsyncStorage>,
    markers_before: u64,
    markers_after: u64,
) -> BTreeSet<u64> {
    // 释放完成的页进入可分配集合（`promote_free_releases` 与 checkpoint
    // 第 2 步同一条入口，见 `src/wal/checkpoint.rs`）。
    storage.promote_free_releases(markers_after);

    // 提升前的高水位：释放的页号恒小于它，新分配的页号恒不小于它。
    let hwm: HighWaterMark = storage.page_count();

    let mut allocated: Vec<u64> = Vec::new();
    for _ in 0..4096 {
        let id = storage.allocate_page().await.unwrap().0;
        let reused = id < hwm;
        allocated.push(id);
        if !reused {
            break;
        }
    }

    assert_allocatable_set(&allocated, hwm, markers_before, markers_after)
}

/// 树级释放集合观测：提升 → 取页 → 断言（同步入口，`SyncPageLoader` 在
/// blocking 线程内使用）。
fn assert_allocatable_set_has_no_duplicate_sync(
    loader: &SyncPageLoader,
    storage: &Arc<dyn AsyncStorage>,
    markers_before: u64,
    markers_after: u64,
) -> BTreeSet<u64> {
    storage.promote_free_releases(markers_after);
    let hwm: HighWaterMark = storage.page_count();

    let mut allocated: Vec<u64> = Vec::new();
    for _ in 0..4096 {
        let id = loader.allocate_page().unwrap().0;
        let reused = id < hwm;
        allocated.push(id);
        if !reused {
            break;
        }
    }

    assert_allocatable_set(&allocated, hwm, markers_before, markers_after)
}

/// 从根下降收集叶子页号、内部节点页号与树高（真实页读 + `InternalNodeRef`
/// 的公开读面）。
fn collect_tree(loader: &SyncPageLoader, root: PageId) -> (Vec<PageId>, Vec<PageId>, usize) {
    let mut leaves = Vec::new();
    let mut internals = Vec::new();
    let mut level = 0;
    let mut current = vec![root];
    loop {
        let mut next = Vec::new();
        for page_id in &current {
            let guard = loader.load_page(*page_id).unwrap();
            let data = guard.page_data();
            if data[0] == LEAF_NODE {
                leaves.push(*page_id);
            } else {
                internals.push(*page_id);
                let iref = InternalNodeRef::new(&data);
                next.push(PageId(iref.leftmost_child() as u64));
                for i in 0..iref.key_count() {
                    if let Some(child) = iref.get_child_page_id(i) {
                        next.push(PageId(child as u64));
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        current = next;
        level += 1;
    }
    (leaves, internals, level)
}

// ===========================================================================
// IPO R1 / R2：被合并掉的页只释放一次
// ===========================================================================

/// 库级形态（真实入口）：删除使主键索引的叶发生兄弟合并——被合并掉的页只
/// 进入待释放集合一次，提升后的可分配集合中同一页号不出现两次。
#[tokio::test]
async fn merged_index_page_enters_the_allocatable_set_once() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let db = Database::open(&path).await.unwrap();
    seed_table(&db, "t", 200).await;

    let storage = db.buffer_pool.storage().clone();
    let markers_before = storage.release_completion_limit();
    assert_eq!(
        markers_before, 0,
        "precondition: no page was released before the delete"
    );

    // 主键索引的删除路径（一个事务内的逐行删除，避免中途的提交边界）。
    let tx = db.begin().await.unwrap();
    for id in 0..150i64 {
        let resp = to_response(
            db.execute_in_tx(&format!("DELETE FROM t WHERE id = {id}"), &tx)
                .await,
        );
        assert!(
            !matches!(resp, Response::Error { .. }),
            "DELETE must succeed: {resp:?}"
        );
    }
    db.commit(tx).await.unwrap();

    let markers_after = storage.release_completion_limit();
    assert!(
        markers_after > markers_before,
        "the delete must have released at least one merged page"
    );

    assert_allocatable_set_has_no_duplicate(&storage, markers_before, markers_after).await;
}

/// 树级形态（真实文件库 + 真实 `BTree`）：一次批量删除使多片叶发生兄弟合并
/// ——每个被合并掉的页只带一次释放完成标记，可分配集合中同一页号不出现两次。
#[tokio::test]
async fn sibling_merge_releases_the_merged_page_once() {
    tokio::task::spawn_blocking(move || {
        let pool = create_test_pool();
        let loader = Arc::new(SyncPageLoader::new(pool.clone()));
        let mut btree = BTree::new(loader.clone()).unwrap();

        // 200 条 → 叶分裂成多片（叶容量 92 条）。
        for i in 0..200u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }
        let (leaves_before, _, _) = collect_tree(&loader, btree.root_page_id());
        assert!(
            leaves_before.len() >= 2,
            "the tree must have more than one leaf before the delete"
        );

        let markers_before = pool.storage().release_completion_limit();
        assert_eq!(
            markers_before, 0,
            "precondition: no page was released before the delete"
        );

        // 逐条删除使叶欠载 → 兄弟合并。
        for i in 0..170u32 {
            btree.delete(&make_key(i)).unwrap();
        }

        let markers_after = pool.storage().release_completion_limit();
        assert!(
            markers_after > markers_before,
            "the delete must have released at least one merged page"
        );

        // 被合并掉的页不再被树引用（它不再属于树这个对象）。
        let (leaves_after, _, _) = collect_tree(&loader, btree.root_page_id());
        let released = assert_allocatable_set_has_no_duplicate_sync(
            &loader,
            pool.storage(),
            markers_before,
            markers_after,
        );
        for page in &released {
            assert!(
                !leaves_after.contains(&PageId(*page)),
                "a released page must not stay reachable from the tree"
            );
        }
    })
    .await
    .unwrap();
}

/// 树级形态：内部节点合并（`merge_internal_nodes`）释放的页同样只释放一次。
///
/// 内部节点的兄弟合并只在树高 ≥3（根分裂出内部节点子节点）时可达：根需
/// 98 片叶才分裂 ⇒ 约 9k 条键。本用例在树级（非表级）构造该形态——它不
/// 触及 ISS23 的「删列改写后插入」路径，不受本 Cycle 的行数规模约束。
#[tokio::test]
async fn internal_merge_releases_the_merged_page_once() {
    tokio::task::spawn_blocking(move || {
        let pool = create_test_pool();
        let loader = Arc::new(SyncPageLoader::new(pool.clone()));
        let mut btree = BTree::new(loader.clone()).unwrap();

        // 足够多的键 ⇒ 根分裂出内部节点子节点（树高 3）。
        let total: u32 = 20_000;
        for i in 0..total {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }
        let (_, internals_before, height) = collect_tree(&loader, btree.root_page_id());
        assert!(height >= 2, "the tree must have at least three levels");
        assert!(
            internals_before.len() >= 2,
            "the tree must have internal nodes below the root"
        );

        let markers_before = pool.storage().release_completion_limit();

        // 中段整段删除：两片内部节点的子节点数同时降到阈值以下 ⇒ 兄弟合并
        // （而非再分配）。
        for i in 2_000..18_000u32 {
            btree.delete(&make_key(i)).unwrap();
        }

        let markers_after = pool.storage().release_completion_limit();
        assert!(
            markers_after > markers_before,
            "the delete must have released at least one merged page"
        );

        let released = assert_allocatable_set_has_no_duplicate_sync(
            &loader,
            pool.storage(),
            markers_before,
            markers_after,
        );

        // 覆盖内部节点合并形态：被释放的页里至少有一片是删除前的内部节点
        // （`merge_internal_nodes` 释放的是内部节点页）。
        let internal_released = released
            .iter()
            .filter(|id| internals_before.contains(&PageId(**id)))
            .count();
        assert!(
            internal_released > 0,
            "the delete must exercise the internal-node merge path"
        );
    })
    .await
    .unwrap();
}

// ===========================================================================
// IPO R3：索引插入路径的下降必然终止
// ===========================================================================

/// 库级形态（真实入口）：`ALTER TABLE DROP COLUMN` 的改写路径在多层索引规模
/// 下正常返回，进程不 abort；改写后索引树结构保持自洽（子指针遍历无重复引用）。
///
/// 规模约束：本 Cycle 的行数上限为 300 行以下（ISS23 的触发面在 300 行以上，
/// 本用例在修复前不得用更大的规模构造——否则用例在修复前无法被观察）。
///
/// 边界：**数据链**不在本用例的判据内——改写路径的双尾游标（ISS22）会丢行，
/// 属 Iteration 001 的修复面；本用例判的是**索引树**结构（子指针遍历）。
#[tokio::test]
async fn alter_table_drop_column_completes_without_exhausting_the_stack() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT, extra INT)").await;
    for start in (0..200i64).step_by(50) {
        let end = (start + 50).min(200);
        let values: Vec<String> = (start..end).map(|i| format!("({i}, {i}, {i})")).collect();
        ok(&db, &format!("INSERT INTO t VALUES {}", values.join(","))).await;
    }

    // 改写前：主键索引已达多层（下降经过内部节点）。
    let meta = db.table_manager.get_table("t").await.unwrap();
    let root = meta.index_manager.root_page_id();
    let (leaves, depth, duplicates) = walk_index_tree(&db.buffer_pool, root).await;
    assert!(
        leaves >= 2,
        "the primary key index must have several leaves"
    );
    assert!(depth >= 1, "the primary key index must have several levels");
    assert!(
        duplicates.is_empty(),
        "a page must not be referenced by two nodes: {duplicates:?}"
    );

    // 改写语句必须正常返回（成功或明确错误）——进程不得 abort。
    let resp = to_response(db.execute_sql("ALTER TABLE t DROP COLUMN extra").await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "ALTER TABLE DROP COLUMN must complete: {resp:?}"
    );

    // 改写后：索引树仍自洽——子指针遍历无重复引用、无环。
    let meta = db.table_manager.get_table("t").await.unwrap();
    let root = meta.index_manager.root_page_id();
    let (_, _, duplicates) = walk_index_tree(&db.buffer_pool, root).await;
    assert!(
        duplicates.is_empty(),
        "a page must not be referenced by two nodes after the rewrite: {duplicates:?}"
    );
    db.close().await.unwrap();
}

/// 树级形态：结构被破坏到某节点可达自身时，插入返回错误而不是耗尽栈。
#[tokio::test]
async fn insert_into_a_cyclic_structure_returns_an_error() {
    tokio::task::spawn_blocking(move || {
        let pool = create_test_pool();
        let loader = Arc::new(SyncPageLoader::new(pool.clone()));
        let mut btree = BTree::new(loader.clone()).unwrap();

        // 150 条 ⇒ 根是内部节点。
        for i in 1000..1150u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }
        let root = btree.root_page_id();

        // 破坏结构：根的最左子节点指向根自身 ⇒ 下降成环。
        {
            let guard = loader.load_page(root).unwrap();
            guard
                .modify_page(|page| {
                    let mut internal = InternalNode::from_page(page)?;
                    internal.set_leftmost_child(root.0 as u32);
                    Ok::<(), StorageError>(())
                })
                .unwrap();
        }

        // 比所有已有键都小的键 ⇒ 下降进入根的最左子节点 ⇒ 成环。
        let result = btree.insert(&make_key(1), RowId::new(1, 0));
        assert!(
            result.is_err(),
            "an insert into a cyclic structure must fail with an error, not exhaust the stack"
        );
    })
    .await
    .unwrap();
}
