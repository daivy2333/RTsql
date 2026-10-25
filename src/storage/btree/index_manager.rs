use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::storage::{
    btree::node::{
        ensure_index_page_intact, InternalNodeRef, LeafNodeRef, INTERNAL_NODE, LEAF_NODE,
    },
    catalog::Catalog,
    integrity::{check_tree_page, violation_summary},
    page_format::{Key, RowId},
    BufferPool, PageId, Result, StorageError,
};
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::RwLock;

use super::{AsyncPageLoader, BTree, SyncPageLoader};

/// MS23 Iteration 001 (D5): which `__tables` slot this B-Tree's root
/// persists to on every root change. A table can now own several index
/// trees — the PK index plus one per qualifying UNIQUE column — and each
/// tree's root lives in its own catalog slot.
#[derive(Debug, Clone)]
pub enum CatalogRootSlot {
    /// The table's PK index root (`__tables.index_root_page_id`).
    PrimaryKey { table: String },
    /// The Nth UNIQUE index root (`__tables.unique_roots[ordinal]`).
    /// `ordinal` is the position among the table's qualifying UNIQUE
    /// columns in ascending column order — the same order the roots are
    /// serialized in the catalog row.
    Unique { table: String, ordinal: usize },
    /// MS21 Iteration 002 (D4): the Nth secondary (user CREATE INDEX) root
    /// (`__tables.secondary_indexes[ordinal]`). `ordinal` is the position in
    /// the table's registration list — creation order, the same order the
    /// section is serialized in the catalog row.
    Secondary { table: String, ordinal: usize },
}

/// MS34 Iteration 000（ISS20 / D4）：写侧暂停点位置。
///
/// 语义位置固定为「结构变更已生效、根页 id 尚未发布」——修复前后一致，
/// 因此同一个夹具在修复前后落在同一个窗口上。位置不携带任何运行身份
/// （无 run-id / 指纹 / 时间戳），也不承载任何判定逻辑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePausePosition {
    /// 结构变更已生效、根页 id 尚未发布。
    AfterStructureChangeBeforeRootPublish,
}

/// MS34 Iteration 000（ISS20 / D4）：写侧暂停点槽位。
///
/// `None` = 未装载——写路径零行为差异（写单元只多做一次未命中检查）。
type WritePauseSlot = Arc<Mutex<Option<Arc<WritePauseInner>>>>;

/// MS34 Iteration 000（ISS20 / D4）：写侧暂停点内核（写侧与测试侧共享）。
#[derive(Debug)]
struct WritePauseInner {
    /// 装载的位置——只有匹配的位置才停住。
    position: WritePausePosition,
    /// 到达次数：写单元到达暂停点的累计次数（测试侧轮询的「到达」信号）。
    arrivals: AtomicU64,
    /// 已放行的到达次数。
    released: Mutex<u64>,
    /// 放行通知——写侧（阻塞线程）在此等待。
    condvar: Condvar,
}

impl WritePauseInner {
    /// 写侧命中：登记到达并等待放行。
    ///
    /// 位置不匹配时不等待——语义位置唯一，匹配才停住。
    fn arrive_and_wait(&self, position: WritePausePosition) {
        if self.position != position {
            return;
        }
        let arrival = self.arrivals.fetch_add(1, Ordering::AcqRel) + 1;
        let mut released = self.released.lock().unwrap();
        while *released < arrival {
            released = self.condvar.wait(released).unwrap();
        }
    }
}

/// MS34 Iteration 000（ISS20 / D4）：写侧暂停点句柄（测试支持夹具）。
///
/// 「到达」= 写单元到达暂停点的累计次数（可轮询）；「放行」= 允许当前到达
/// 继续；「卸载」= 之后的写单元不再被停住。
#[derive(Debug, Clone)]
pub struct WritePauseHandle {
    inner: Arc<WritePauseInner>,
    slot: WritePauseSlot,
}

impl WritePauseHandle {
    fn new(position: WritePausePosition, slot: WritePauseSlot) -> Self {
        Self {
            inner: Arc::new(WritePauseInner {
                position,
                arrivals: AtomicU64::new(0),
                released: Mutex::new(0),
                condvar: Condvar::new(),
            }),
            slot,
        }
    }

    /// 写单元到达暂停点的累计次数——测试侧轮询的「到达」信号。
    pub fn arrivals(&self) -> u64 {
        self.inner.arrivals.load(Ordering::Acquire)
    }

    /// 放行：允许当前到达继续（若尚未到达则允许下一次到达）。
    pub fn release(&self) {
        let mut released = self.inner.released.lock().unwrap();
        *released += 1;
        self.inner.condvar.notify_all();
    }

    /// 卸载暂停点——之后的写单元不再被停住。
    pub fn disarm(&self) {
        let mut slot = self.slot.lock().unwrap();
        if slot
            .as_ref()
            .map(|inner| Arc::ptr_eq(inner, &self.inner))
            .unwrap_or(false)
        {
            *slot = None;
        }
    }
}

/// 写侧暂停点命中检查——未装载时立即返回（未装载 = 零行为差异）。
fn pause_at(slot: &WritePauseSlot, position: WritePausePosition) {
    let inner = slot.lock().unwrap().clone();
    if let Some(inner) = inner {
        inner.arrive_and_wait(position);
    }
}

/// IndexManager: Async API wrapper for BTree
/// Uses AtomicPageId for lock-free root page access (read operations)
/// Write operations use spawn_blocking + temporary BTree instance
pub struct IndexManager {
    root_page_id: Arc<AtomicU64>,     // 无锁访问根页
    sync_loader: Arc<SyncPageLoader>, // 写操作仍用 sync
    async_loader: AsyncPageLoader,    // 读操作用 async
    /// MS34 Iteration 000（ISS20 / D4）：反向映射容器改为同步 `RwLock`——
    /// 写单元在阻塞线程内同步访问；公开的 `find_key_by_row_id` 签名保持
    /// `async` 不变，调用点不跨 await 持锁（M13）。
    row_to_key: Arc<RwLock<HashMap<RowId, Vec<u8>>>>,
    /// MS10-T02 Iter000 003-rework (R-T0b-R5): optional catalog context
    /// `(catalog, root slot)` for root-change persistence. Attached by
    /// `TableManager` — at `create_table` directly, and for restored tables
    /// only AFTER crash recovery finished (replay-time root changes must not
    /// be persisted: the recovery load point stays fixed across re-recoveries).
    /// Direct constructions without a catalog (tests) leave it `None`.
    /// MS23 Iteration 001 (D5): the slot generalizes the context from "the
    /// table's PK root" to "PK root or the Nth UNIQUE root".
    catalog_ctx: Mutex<Option<(Arc<Catalog>, CatalogRootSlot)>>,
    /// MS34 Iteration 000（ISS20 / D2）：写侧闸门——同一索引的结构变更串行
    /// 执行，并发写者不会读到彼此的中间结构（不会产生结构分叉）。
    write_gate: Arc<Mutex<()>>,
    /// MS34 Iteration 000（ISS20 / D4）：写侧暂停点槽位——`None` 时写路径
    /// 零行为差异。
    write_pause: WritePauseSlot,
}

impl IndexManager {
    /// Get the root page-id of this BTree (zero if uninitialized).
    pub fn root_page_id(&self) -> PageId {
        PageId(self.root_page_id.load(Ordering::Acquire))
    }

    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());

        // 创建 BTree 并获取 root_page_id
        let btree = BTree::new(sync_loader.clone())?;
        let root_page_id = btree.root_page_id().0;

        Ok(Self {
            root_page_id: Arc::new(AtomicU64::new(root_page_id)),
            sync_loader,
            async_loader,
            row_to_key: Arc::new(RwLock::new(HashMap::new())),
            catalog_ctx: Mutex::new(None),
            write_gate: Arc::new(Mutex::new(())),
            write_pause: Arc::new(Mutex::new(None)),
        })
    }

    /// MS07-T01: bind an existing BTree root page without allocating a new
    /// one. Used by `Catalog::recover` to reattach `IndexManager` instances
    /// to index roots that were allocated in a previous process and are
    /// recorded in the `__tables` SlottedPage.
    pub fn from_root(buffer_pool: Arc<BufferPool>, root_page_id: PageId) -> Result<Self> {
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());

        Ok(Self {
            root_page_id: Arc::new(AtomicU64::new(root_page_id.0)),
            sync_loader,
            async_loader,
            row_to_key: Arc::new(RwLock::new(HashMap::new())),
            catalog_ctx: Mutex::new(None),
            write_gate: Arc::new(Mutex::new(())),
            write_pause: Arc::new(Mutex::new(None)),
        })
    }

    /// MS10-T02 Iter000 003-rework (R-T0b-R5): attach the catalog context so
    /// every B-Tree root change (split on insert, shrink on delete merge) is
    /// persisted to the table's `__tables` row. Builder style keeps the
    /// `new` / `from_root` signatures (and every catalog-less caller)
    /// unchanged. MS23 Iteration 001 (D5): the caller names the slot this
    /// tree's root persists to (PK root or Nth UNIQUE root).
    pub fn with_catalog_context(self, catalog: Arc<Catalog>, slot: CatalogRootSlot) -> Self {
        *self.catalog_ctx.lock().unwrap() = Some((catalog, slot));
        self
    }

    /// R-T0b-R5: attach the catalog context post-construction (used by
    /// `TableManager` for tables restored by `open_or_init`, after crash
    /// recovery has completed — see the field docs for why replay must run
    /// without a context).
    pub fn set_catalog_context(&self, catalog: Arc<Catalog>, slot: CatalogRootSlot) {
        *self.catalog_ctx.lock().unwrap() = Some((catalog, slot));
    }

    /// MS34 Iteration 000（ISS20 / D4）：装载写侧暂停点。
    ///
    /// 装载后，到达该位置的写单元被确定性地停住，直到
    /// [`WritePauseHandle::release`]；[`WritePauseHandle::disarm`] 卸载。
    /// 未装载时写路径零行为差异——写单元只多做一次未命中检查。
    pub fn arm_write_pause(&self, position: WritePausePosition) -> WritePauseHandle {
        let slot = self.write_pause.clone();
        let handle = WritePauseHandle::new(position, slot.clone());
        *slot.lock().unwrap() = Some(handle.inner.clone());
        handle
    }

    /// Persist a root change to the catalog row (R-T0b-R5). No-op without a
    /// catalog context. Errors propagate strictly — a failed catalog write
    /// must not silently desynchronize the recoverable root. MS23 Iteration
    /// 001 (D5): dispatch by slot — PK root to `index_root_page_id`, UNIQUE
    /// root to `unique_roots[ordinal]`. MS21 Iteration 002 (D4): secondary
    /// root to `secondary_indexes[ordinal]`.
    async fn sync_root_to_catalog(&self, new_root: PageId) -> Result<()> {
        let ctx = self.catalog_ctx.lock().unwrap().clone();
        if let Some((catalog, slot)) = ctx {
            match slot {
                CatalogRootSlot::PrimaryKey { table } => {
                    catalog.update_table_root(&table, new_root.0 as u32).await?;
                }
                CatalogRootSlot::Unique { table, ordinal } => {
                    catalog
                        .update_unique_index_root(&table, ordinal, new_root.0 as u32)
                        .await?;
                }
                CatalogRootSlot::Secondary { table, ordinal } => {
                    catalog
                        .update_secondary_index_root(&table, ordinal, new_root.0 as u32)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Async search — direct async path without spawn_blocking
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        let key_obj = Key::new(key);

        self.search_from_page_async(root_page_id, &key_obj).await
    }

    /// Async search_all — find all RowIds matching a key (for non-unique indexes)
    pub async fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        let key_obj = Key::new(key);

        self.search_all_from_page_async(root_page_id, &key_obj)
            .await
    }

    /// Recursive async search from a page
    fn search_from_page_async<'a>(
        &'a self,
        page_id: PageId,
        key: &'a Key,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Option<RowId>>> + Send + 'a>> {
        Box::pin(async move {
            let child_page_id = {
                let guard = self.async_loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                ensure_index_page_intact(
                    &data_guard,
                    page_id.0,
                    self.root_page_id.load(Ordering::Acquire),
                )?;

                if data_guard[0] == LEAF_NODE {
                    let leaf = LeafNodeRef::new(&data_guard);
                    let (found, pos) = leaf.find_key_position_binary(key);
                    if found {
                        return Ok(leaf.get_row_id(pos));
                    } else {
                        return Ok(None);
                    }
                } else {
                    let internal = InternalNodeRef::new(&data_guard);
                    internal.find_child_page_id_binary(key)
                }
            }; // guard and data_guard dropped here

            self.search_from_page_async(PageId(child_page_id as u64), key)
                .await
        })
    }

    /// Recursive async search_all from a page
    #[allow(clippy::only_used_in_recursion)]
    fn search_all_from_page_async<'a>(
        &'a self,
        page_id: PageId,
        key: &'a Key,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<RowId>>> + Send + 'a>> {
        Box::pin(async move {
            let child_page_ids = {
                let guard = self.async_loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                ensure_index_page_intact(
                    &data_guard,
                    page_id.0,
                    self.root_page_id.load(Ordering::Acquire),
                )?;

                if data_guard[0] == LEAF_NODE {
                    // Leaf: collect all matching RowIds
                    let leaf = LeafNodeRef::new(&data_guard);
                    let matches = leaf.find_all_matches(key);
                    let mut row_ids = Vec::new();
                    for idx in matches {
                        if let Some(rid) = leaf.get_row_id(idx) {
                            row_ids.push(rid);
                        }
                    }
                    return Ok(row_ids);
                } else {
                    // Internal: 等值查找的候选子树集合（ISS05 机制 B 修复，
                    // 公式见 `InternalNodeRef::equality_candidate_children`——
                    // 闭区间键域 ⇒ 每子树恰访一次；等值分隔键连占时覆盖全部
                    // 相邻子树，无「非空即跳过常规路由」分支）。
                    let internal = InternalNodeRef::new(&data_guard);
                    internal
                        .equality_candidate_children(key)
                        .into_iter()
                        .map(|c| PageId(c as u64))
                        .collect::<Vec<PageId>>()
                }
            }; // guard and data_guard dropped

            // Recursively search all child subtrees
            let mut results = Vec::new();
            for child_page_id in child_page_ids {
                let child_results = self.search_all_from_page_async(child_page_id, key).await?;
                results.extend(child_results);
            }
            Ok(results)
        })
    }

    /// Insert a key-value pair — spawn_blocking with temporary BTree instance
    /// Updates root_page_id atomically if a root split occurred
    ///
    /// MS34 Iteration 000（ISS20 / D1–D2）：结构变更、根页 id 发布与反向映射
    /// 更新构成**同一个写单元**——三者都在同一个 blocking 闭包内完成。tokio 的
    /// blocking 任务在 JoinHandle 被丢弃后仍会跑完，因此语句被取消时该写单元
    /// 整体落地或整体未开始，不会留下「结构已变、根未发布」或「条目已写、
    /// 反向映射缺失」的半写状态。同一索引的写单元经写侧闸门串行化。
    ///
    /// 目录根同步留在写单元之外（既有持久化语义不变）。
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();
        let root_page_id = self.root_page_id.clone();
        let row_to_key = self.row_to_key.clone();
        let write_gate = self.write_gate.clone();
        let write_pause = self.write_pause.clone();

        let new_root = tokio::task::spawn_blocking(move || {
            // 写侧串行化（D2）：同一索引的结构变更串行执行。
            let _gate = write_gate.lock().unwrap();

            let mut btree =
                BTree::from_root(PageId(root_page_id.load(Ordering::Acquire)), sync_loader);
            let new_root = btree.insert(&key_vec, row_id)?;

            // MS34 Iteration 000（ISS20 / D4）：写侧暂停点——结构变更已生效、
            // 根页 id 尚未发布。修复前后语义位置一致。
            pause_at(
                &write_pause,
                WritePausePosition::AfterStructureChangeBeforeRootPublish,
            );

            // 根页 id 发布（D1）：与结构变更同单元。
            if let Some(new_root_id) = new_root {
                root_page_id.store(new_root_id.0, Ordering::Release);
            }

            // 反向映射更新（D1）：与结构变更同单元。
            row_to_key.write().unwrap().insert(row_id, key_vec.clone());

            Ok::<Option<PageId>, StorageError>(new_root)
        })
        .await??;

        // If root split occurred, persist the new root to the catalog.
        if let Some(new_root_id) = new_root {
            self.sync_root_to_catalog(new_root_id).await?;
        }
        Ok(())
    }

    /// Delete a key — spawn_blocking with temporary BTree instance
    ///
    /// MS34 Iteration 000（ISS20 / D1–D3）：正向条目删除与反向映射清理构成
    /// **同一个写单元**，顺序为「先删正向条目、后删反向映射」——取消落在两者
    /// 之间不再产生同形半写状态。
    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();
        let root_page_id = self.root_page_id.clone();
        let row_to_key = self.row_to_key.clone();
        let write_gate = self.write_gate.clone();
        let write_pause = self.write_pause.clone();

        let (new_root, old_root) = tokio::task::spawn_blocking(move || {
            let _gate = write_gate.lock().unwrap();

            let old_root = PageId(root_page_id.load(Ordering::Acquire));
            let mut btree = BTree::from_root(old_root, sync_loader);
            // 正向条目定位——与既有 `search` 同语义（精确匹配）。
            let found = btree.search(&key_vec)?;
            let new_root = btree.delete(&key_vec)?;

            pause_at(
                &write_pause,
                WritePausePosition::AfterStructureChangeBeforeRootPublish,
            );

            // 根页 id 发布（D1/D3）：与结构变更同单元。
            if let Some(new_root_id) = new_root {
                root_page_id.store(new_root_id.0, Ordering::Release);
            }

            // 反向映射清理（D3）：正向条目已删后才清理。
            if let Some(row_id) = found {
                row_to_key.write().unwrap().remove(&row_id);
            }

            Ok::<(Option<PageId>, PageId), StorageError>((new_root, old_root))
        })
        .await??;

        if let Some(new_root_id) = new_root {
            self.sync_root_to_catalog(new_root_id).await?;
            // MS35（`free-page-lifecycle` R3）：缩根释放的锚点写 = 目录 root
            // 更新，此刻已发出 ⇒ 释放完成标记（旧根页）。
            self.sync_loader.complete_free_release(&[old_root]);
        }

        Ok(())
    }

    /// MS21 Iteration 002 (D3/T2-b): exact single-entry delete — removes only
    /// the `(key, row_id)` entry, leaving sibling same-key entries and their
    /// reverse mappings untouched. Secondary indexes allow duplicate keys
    /// (multiple rows sharing one value), so the ambiguous primitives
    /// `delete(key)` (first-≥ position) / `update(key, rid)` (first match)
    /// must never be used against them — this is their only cleanup channel.
    ///
    /// `BTree::delete_exact` never rebalances and never changes the root
    /// (returns `Ok(None)` by contract), so no root or catalog sync is needed.
    /// The reverse mapping entry is removed only when it still maps the rid
    /// to this exact key. Key present but rid mismatched → `KeyNotFound`
    /// (same error surface as the leaf-level exact delete).
    pub async fn delete_exact(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();
        let root_page_id = self.root_page_id.clone();
        let row_to_key = self.row_to_key.clone();
        let write_gate = self.write_gate.clone();

        tokio::task::spawn_blocking(move || {
            let _gate = write_gate.lock().unwrap();

            let mut btree =
                BTree::from_root(PageId(root_page_id.load(Ordering::Acquire)), sync_loader);
            let _ = btree.delete_exact(&key_vec, row_id)?;

            // Reverse mapping (D5): clear the rid's entry only when it still
            // names this key (an entry re-pointed by a later writer must
            // survive) — in the same write unit as the structural change.
            let mut map = row_to_key.write().unwrap();
            if map.get(&row_id).map(|k| k.as_slice()) == Some(key_vec.as_slice()) {
                map.remove(&row_id);
            }
            Ok::<(), StorageError>(())
        })
        .await??;
        Ok(())
    }

    /// Async scan all entries — direct async path
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        self.scan_all_async_from_root(root_page_id).await
    }

    /// 树遍历枚举全部条目（B2）。显式栈、内部页感知、visited 去重；
    /// 子页按从左到右压栈，弹出顺序即叶序，结果保持键升序。
    /// 不沿叶链走，也不以 page 0 作为终止哨兵——栈空即结束。
    async fn scan_all_async_from_root(
        &self,
        root_page_id: PageId,
    ) -> Result<Vec<(Vec<u8>, RowId)>> {
        let mut results = Vec::new();
        let mut stack = vec![root_page_id];
        let mut visited = HashSet::new();

        while let Some(page_id) = stack.pop() {
            if !visited.insert(page_id.0) {
                continue;
            }
            let guard = self.async_loader.load_page(page_id).await?;
            let data_guard = guard.page_data();
            ensure_index_page_intact(
                &data_guard,
                page_id.0,
                self.root_page_id.load(Ordering::Acquire),
            )?;
            let page_type = data_guard[0];

            if page_type == LEAF_NODE {
                let leaf = LeafNodeRef::new(&data_guard);
                for i in 0..leaf.key_count() {
                    if let (Some(key), Some(row_id)) = (leaf.get_key(i), leaf.get_row_id(i)) {
                        results.push((key.as_bytes().to_vec(), row_id));
                    }
                }
            } else if page_type == INTERNAL_NODE {
                let internal = InternalNodeRef::new(&data_guard);
                let mut children = Vec::with_capacity(internal.key_count() + 1);
                children.push(internal.leftmost_child());
                for i in 0..internal.key_count() {
                    if let Some(child) = internal.get_child_page_id(i) {
                        children.push(child);
                    }
                }
                // 右到左入栈，弹出时从左到右，叶序即键序。
                // 页号 0 是合法页（根分裂后旧根常为 page 0），不能当哨兵丢弃。
                for child in children.into_iter().rev() {
                    let child_id = PageId(child as u64);
                    if !visited.contains(&child_id.0) {
                        stack.push(child_id);
                    }
                }
            }
        }

        Ok(results)
    }

    /// Update the RowId for an existing key — spawn_blocking with temporary BTree instance
    ///
    /// MS34 Iteration 000（ISS20 / D5）：反向映射更新与结构变更同单元。
    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();
        let root_page_id = self.root_page_id.clone();
        let row_to_key = self.row_to_key.clone();
        let write_gate = self.write_gate.clone();

        tokio::task::spawn_blocking(move || {
            let _gate = write_gate.lock().unwrap();

            let btree = BTree::from_root(PageId(root_page_id.load(Ordering::Acquire)), sync_loader);
            btree.update(&key_vec, new_row_id)?;

            // 反向映射更新（D5）：与结构变更同单元。
            row_to_key
                .write()
                .unwrap()
                .insert(new_row_id, key_vec.clone());
            Ok::<(), StorageError>(())
        })
        .await??;
        Ok(())
    }

    /// Find key by RowId (M10 reverse mapping)
    ///
    /// MS34 Iteration 000（ISS20 / D4）：容器为同步 `RwLock`，本方法内部同步
    /// 加锁——不跨 await 持锁（M13）；公开签名保持 `async` 不变。
    ///
    /// MS34 Iteration 000（ISS20 / D2）：本次读与写单元串行——读到的反向映射
    /// 是**写单元落定后**的状态（写单元持写侧闸门期间读不到它的中间态）。
    /// 理由：回滚/回收按「反向映射是否命中」分桶，若读到写单元尚未落定的中间
    /// 态，被取消语句写入的键会被误判为墓碑而不被释放。等待发生在 blocking
    /// 线程上：异步运行时线程不被阻塞，也不跨 await 持锁（M13）。
    pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>> {
        let row_to_key = self.row_to_key.clone();
        let write_gate = self.write_gate.clone();
        tokio::task::spawn_blocking(move || {
            let _gate = write_gate.lock().unwrap();
            row_to_key.read().unwrap().get(&row_id).cloned()
        })
        .await
        .unwrap()
    }

    /// Collect every `PageId` occupied by this BTree (root, internal nodes,
    /// all leaves), using a stack-based DFS from the root with a `visited`
    /// set to guard against cycles. Read-only — does not modify any page.
    ///
    /// MS07-T02: used by `TableManager::drop_table` to return the table's
    /// index pages to the storage free-list.
    ///
    /// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
    /// Iteration 002 (B-4): the traversal reads the tree pages through the
    /// integrity module's bounded reader (`check_tree_page`) — the previous
    /// `InternalNodeRef::get_child_page_id` path sliced the slot payload
    /// unchecked and panicked on an out-of-bounds slot offset (release path
    /// ⇒ process abort). A damaged page is counted, reported and skipped:
    /// its subtree stays unreleased (the hole-page policy), the pages
    /// collected so far are kept.
    pub async fn collect_all_pages(&self) -> Result<Vec<PageId>> {
        let root = PageId(self.root_page_id.load(Ordering::Acquire));
        let mut result = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack = vec![root];

        while let Some(page_id) = stack.pop() {
            if !visited.insert(page_id.0) {
                continue;
            }
            result.push(page_id);

            let children = {
                let guard = self.async_loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                let step = check_tree_page(&data_guard);
                if let Some(actual) = step.invalid_type {
                    return Err(StorageError::InvalidPageType {
                        expected: LEAF_NODE,
                        actual,
                    });
                }
                if !step.violations.is_empty() {
                    // Damaged page — count it, report it, skip its subtree.
                    eprintln!(
                        "[index-release] index tree page {} is damaged: {}",
                        page_id.0,
                        violation_summary(&step.violations)
                    );
                    continue;
                }
                step.children
                    .into_iter()
                    .map(|child| PageId(child as u64))
                    .collect::<Vec<_>>()
            };

            for child in children {
                if child.0 != 0 {
                    stack.push(child);
                }
            }
        }

        Ok(result)
    }

    /// MS10-T02 Iter000 004-rework (R-T0b-R8, D10): hole-tolerant variant of
    /// [`Self::collect_all_pages`] for releasing the PREVIOUS tree after
    /// recovery rebuilt the index from data pages. A torn tree can contain
    /// hole pages (`InvalidPageType`) or unreadable pages mid-tree; every
    /// page id reached from the root — including a discovered hole itself,
    /// whose id the parent pointer still names — is returned so the caller
    /// can free it, but a bad page never fails the walk and its subtree is
    /// simply unrecoverable (leaked by design, warned). Never returns `Err`.
    pub async fn collect_all_pages_tolerant(&self) -> Vec<PageId> {
        let root = PageId(self.root_page_id.load(Ordering::Acquire));
        let mut result = Vec::new();
        if root.0 == 0 {
            return result;
        }

        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack = vec![root];

        while let Some(page_id) = stack.pop() {
            if !visited.insert(page_id.0) {
                continue;
            }
            result.push(page_id);

            let children = {
                let guard = match self.async_loader.load_page(page_id).await {
                    Ok(guard) => guard,
                    Err(e) => {
                        eprintln!(
                            "[recovery] old index page {:?} unreadable, skipping subtree: {}",
                            page_id, e
                        );
                        continue;
                    }
                };
                let data_guard = guard.page_data();

                let step = check_tree_page(&data_guard);
                if let Some(other) = step.invalid_type {
                    // Torn/hole page reached via its parent pointer: free
                    // it, but its subtree is unnameable — leak with a
                    // warning (same policy as drop_table free failures).
                    eprintln!(
                        "[recovery] old index page {:?} has invalid type {:#04x}, skipping subtree",
                        page_id, other
                    );
                    Vec::new()
                } else if !step.violations.is_empty() {
                    // Damaged page — count it, report it, skip its subtree
                    // (change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                    // Iteration 002 / B-4: the previous unchecked slot slice
                    // panicked here).
                    eprintln!(
                        "[recovery] old index page {:?} is damaged: {}, skipping subtree",
                        page_id,
                        violation_summary(&step.violations)
                    );
                    Vec::new()
                } else {
                    step.children
                        .into_iter()
                        .map(|child| PageId(child as u64))
                        .collect::<Vec<_>>()
                }
            };

            for child in children {
                if child.0 != 0 {
                    stack.push(child);
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FileStorage;
    use tempfile::tempdir;

    /// Build an in-memory IndexManager backed by a temp-file storage.
    async fn make_manager() -> (tempfile::TempDir, IndexManager) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("btree_test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage).unwrap());
        // IndexManager::new is sync but internally block_on (BTree::new), so
        // offload to spawn_blocking to avoid blocking the tokio runtime.
        let im = tokio::task::spawn_blocking(move || IndexManager::new(bp))
            .await
            .unwrap()
            .unwrap();
        (dir, im)
    }

    #[tokio::test]
    async fn collect_all_pages_single_leaf_returns_root_only() {
        let (_dir, im) = make_manager().await;
        let root = im.root_page_id();

        let pages = im.collect_all_pages().await.unwrap();
        assert_eq!(pages, vec![root]);
    }

    #[tokio::test]
    async fn collect_all_pages_returns_all_internal_and_leaves() {
        let (_dir, im) = make_manager().await;

        // Insert enough entries to force at least one leaf split (height >= 2).
        for i in 0..300u64 {
            let key = format!("key{:07}", i);
            im.insert(key.as_bytes(), RowId::new((i % 100) as u32, i as u16))
                .await
                .unwrap();
        }

        let pages = im.collect_all_pages().await.unwrap();
        assert!(
            pages.len() >= 3,
            "height>=2 BTree should have >= 3 pages (root + leaves), got {}",
            pages.len()
        );
        assert!(
            pages.contains(&im.root_page_id()),
            "result must contain the current root {:?}, got {:?}",
            im.root_page_id(),
            pages
        );

        // No duplicates — visited set must prevent cycles / re-visits.
        let mut seen = std::collections::HashSet::new();
        for p in &pages {
            assert!(seen.insert(p.0), "duplicate page id in result: {:?}", p);
        }
    }

    // ===========================================================================
    // MS21 Iteration 002 (T2-b/D3): delete_exact 精确删除原语
    // ===========================================================================

    /// 同 key 双 rid 并存时 `delete_exact` 只删指定条目：兄弟条目、反向映射
    /// 与其余 key 不受扰；键存在但 rid 不匹配 → KeyNotFound。
    #[tokio::test]
    async fn delete_exact_removes_only_named_entry_among_duplicates() {
        let (_dir, im) = make_manager().await;
        let rid_a = RowId::new(3, 1);
        let rid_b = RowId::new(4, 2);
        let rid_c = RowId::new(5, 3);

        // 同 key 两个条目 + 邻近另一 key 条目
        im.insert(b"dup", rid_a).await.unwrap();
        im.insert(b"dup", rid_b).await.unwrap();
        im.insert(b"other", rid_c).await.unwrap();

        // 键在树中但 rid 不匹配 → KeyNotFound（两 rid 均不误删）
        assert!(im.delete_exact(b"dup", RowId::new(9, 9)).await.is_err());
        assert_eq!(im.search_all(b"dup").await.unwrap().len(), 2);

        // 精确删除 rid_a：rid_b 条目与反向映射保留
        im.delete_exact(b"dup", rid_a).await.unwrap();
        assert_eq!(im.search_all(b"dup").await.unwrap(), vec![rid_b]);
        assert_eq!(
            im.find_key_by_row_id(rid_b).await.as_deref(),
            Some(b"dup".as_slice()),
            "sibling reverse mapping must survive"
        );
        assert_eq!(im.find_key_by_row_id(rid_a).await, None);

        // 精确删除最后一个条目后键清空；邻近 key 不受扰
        im.delete_exact(b"dup", rid_b).await.unwrap();
        assert!(im.search_all(b"dup").await.unwrap().is_empty());
        assert_eq!(im.search_all(b"other").await.unwrap(), vec![rid_c]);

        // 缺失键 KeyNotFound（leaf 层精确语义，不误删「首个 ≥」邻近条目）
        assert!(im.delete_exact(b"zzz", rid_c).await.is_err());
        assert_eq!(im.search_all(b"other").await.unwrap(), vec![rid_c]);
    }

    // ===========================================================================
    // change 2026-10-13-e2e-verification-and-defect-closure Iteration 001 (T2):
    // B1（叶链保真）/ B2（scan_all 树遍历）/ 机制 B（delete_exact 候选子树集合）
    // ===========================================================================

    /// Deterministic shuffle: multiplicative hash sort (no RNG state).
    fn shuffled(n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        v.sort_by_key(|i| i.wrapping_mul(2654435761) % 1000003);
        v
    }

    /// Descend to the leftmost leaf via internal `leftmost_child` links, then
    /// walk the sibling chain through `next_leaf_page_id`, returning every
    /// entry in chain order plus the number of leaves visited.
    async fn walk_leaf_chain(im: &IndexManager) -> (Vec<(Vec<u8>, RowId)>, usize) {
        let mut page_id = im.root_page_id();
        loop {
            let (is_leaf, next) = {
                let guard = im.async_loader.load_page(page_id).await.unwrap();
                let data = guard.page_data();
                if data[0] == LEAF_NODE {
                    (true, 0)
                } else {
                    let internal = InternalNodeRef::new(&data);
                    (false, internal.leftmost_child())
                }
            };
            if is_leaf {
                break;
            }
            page_id = PageId(next as u64);
        }

        let mut out = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut leaves = 0usize;
        loop {
            assert!(
                visited.insert(page_id.0),
                "cycle in leaf chain at {:?}",
                page_id
            );
            let next = {
                let guard = im.async_loader.load_page(page_id).await.unwrap();
                let data = guard.page_data();
                assert_eq!(data[0], LEAF_NODE, "chain must only visit leaves");
                let leaf = LeafNodeRef::new(&data);
                leaves += 1;
                for i in 0..leaf.key_count() {
                    out.push((
                        leaf.get_key(i).unwrap().as_bytes().to_vec(),
                        leaf.get_row_id(i).unwrap(),
                    ));
                }
                leaf.next_leaf_page_id()
            };
            if next == 0 {
                break;
            }
            page_id = PageId(next as u64);
        }
        (out, leaves)
    }

    /// B1 回归：`shift_slots_right` 重建页时必须保留 `next_leaf_page_id`。
    /// 乱序插入使大量插入落位叶中（触发 shift），修复前 ~90% 叶子链指针
    /// 被清零，链walk只能触达首叶。
    #[tokio::test]
    async fn shift_slots_right_preserves_leaf_chain_on_mid_inserts() {
        let (_dir, im) = make_manager().await;
        for n in shuffled(300) {
            let key = format!("key{:07}", n);
            im.insert(key.as_bytes(), RowId::new(n as u32, n as u16))
                .await
                .unwrap();
        }

        let (chain, leaves) = walk_leaf_chain(&im).await;
        assert!(
            leaves >= 3,
            "300 shuffled keys must span several leaves, walked {leaves}"
        );
        assert_eq!(
            chain.len(),
            300,
            "leaf chain must reach every entry (B1: chain pointer zeroed pre-fix)"
        );
        assert!(
            chain.windows(2).all(|w| w[0].0 <= w[1].0),
            "leaf chain order must be non-decreasing"
        );
    }

    /// B2 回归：`scan_all` 对多层树返回全部条目（树遍历，不依赖叶链，
    /// 不把内部页当叶读），并保持升序。
    #[tokio::test]
    async fn scan_all_returns_full_set_on_multi_level_tree() {
        let (_dir, im) = make_manager().await;
        let mut expected = Vec::with_capacity(3000);
        for n in shuffled(3000) {
            let key = format!("key{:07}", n);
            let rid = RowId::new(n as u32, n as u16);
            im.insert(key.as_bytes(), rid).await.unwrap();
            expected.push((key.into_bytes(), rid));
        }

        let scanned = im.scan_all().await.unwrap();
        assert_eq!(
            scanned.len(),
            3000,
            "scan_all must return every entry of a multi-level tree (B2: dropped pre-fix)"
        );
        assert!(
            scanned.windows(2).all(|w| w[0].0 <= w[1].0),
            "scan_all must preserve ascending key order"
        );
        let scanned_set: HashSet<(Vec<u8>, RowId)> = scanned.iter().cloned().collect();
        let expected_set: HashSet<(Vec<u8>, RowId)> = expected.into_iter().collect();
        assert_eq!(
            scanned_set, expected_set,
            "scan_all must return exactly the inserted entry set"
        );
    }

    /// 机制 B 回归：`delete_exact` 必须经由等值候选子树集合下探（同值键可
    /// 散布在多个相邻子树、分隔键可等于查找键），叶级候选叶穷尽后才
    /// KeyNotFound——不依赖叶链回退。5 键 × 400 rid 乱序插入使每个键的
    /// 400 个条目跨 4~5 片叶子散布，根部分隔键出现等值长连——修复前
    /// 「首个等值分隔键左后右」双径下探对第三子树即 KeyNotFound。
    #[tokio::test]
    async fn delete_exact_reaches_every_duplicate_entry() {
        let (_dir, im) = make_manager().await;
        let mut entries: Vec<(String, RowId)> = Vec::with_capacity(2000);
        for d in 0..5u64 {
            for r in 0..400u64 {
                entries.push((
                    format!("dup{:02}", d),
                    RowId::new((1000 + d * 400 + r) as u32, r as u16),
                ));
            }
        }
        for idx in shuffled(entries.len()) {
            let (key, rid) = &entries[idx];
            im.insert(key.as_bytes(), *rid).await.unwrap();
        }

        for (key, rid) in &entries {
            im.delete_exact(key.as_bytes(), *rid)
                .await
                .unwrap_or_else(|e| {
                    panic!("delete_exact({key}, {rid:?}) must reach the entry: {e}")
                });
        }
        for d in 0..5u64 {
            let key = format!("dup{:02}", d);
            assert!(
                im.search_all(key.as_bytes()).await.unwrap().is_empty(),
                "all entries of {key} must be gone"
            );
        }
    }

    /// 树高至少 3 的重复键：每个键的 `search_all` 条数等于插入条数，
    /// 随后逐条 `delete_exact` 成功，再 `search_all` 为空。
    /// 与 `delete_exact_reaches_every_duplicate_entry` 同一洗牌序。
    #[tokio::test]
    async fn delete_exact_reaches_every_entry_on_height_at_least_3() {
        let (_dir, im) = make_manager().await;
        let keys = 8u64;
        let rids = 800u64;
        let mut entries: Vec<(String, RowId)> = Vec::with_capacity((keys * rids) as usize);
        for d in 0..keys {
            for r in 0..rids {
                entries.push((
                    format!("dup{:02}", d),
                    RowId::new((d * rids + r) as u32, r as u16),
                ));
            }
        }
        for idx in shuffled(entries.len()) {
            let (key, rid) = &entries[idx];
            im.insert(key.as_bytes(), *rid).await.unwrap();
        }

        let height = index_height(&im).await;
        assert!(
            height >= 3,
            "8 keys x 800 rids must reach an internal child of the root, height={height}"
        );

        for d in 0..keys {
            let key = format!("dup{:02}", d);
            let found = im.search_all(key.as_bytes()).await.unwrap();
            assert_eq!(
                found.len(),
                rids as usize,
                "search_all({key}) must return every inserted rid"
            );
        }

        for (key, rid) in &entries {
            im.delete_exact(key.as_bytes(), *rid)
                .await
                .unwrap_or_else(|e| {
                    panic!("delete_exact({key}, {rid:?}) must reach the entry: {e}")
                });
        }
        for d in 0..keys {
            let key = format!("dup{:02}", d);
            assert!(
                im.search_all(key.as_bytes()).await.unwrap().is_empty(),
                "all entries of {key} must be gone"
            );
        }
    }

    async fn index_height(im: &IndexManager) -> usize {
        let mut page_id = im.root_page_id();
        let mut height = 1usize;
        loop {
            let (is_leaf, child) = {
                let guard = im.async_loader.load_page(page_id).await.unwrap();
                let data = guard.page_data();
                if data[0] == LEAF_NODE {
                    (true, 0u32)
                } else {
                    let internal = InternalNodeRef::new(&data);
                    (false, internal.leftmost_child())
                }
            };
            if is_leaf {
                return height;
            }
            height += 1;
            page_id = PageId(child as u64);
        }
    }
}
