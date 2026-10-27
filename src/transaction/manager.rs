use crate::storage::page_format::SlottedPageRef;
use crate::storage::{
    corrupted_data_page_slot_count_error, corrupted_data_page_slot_error,
    read_tuple_from_data_page, update_version_header_in_data_page, BufferPool, PageId, Result,
    RowId, StorageError, TableMeta,
};
use crate::transaction::{Snapshot, TransactionError, TransactionId, VersionHeader};
use crate::wal::{WALBuffer, WalRecord};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 墓碑行索引还原时的版本链回溯上限（MS24 Iter001 replan 2.9/D9）——仅防御
/// 损坏库的无环性；正常链长受版本链深度限制，远小于该值。
const MAX_TOMBSTONE_CHAIN_WALK: usize = 64;

/// Transaction state
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionState {
    Active,
    Committed,
    Aborted,
}

/// Transaction represents an active database transaction
pub struct Transaction {
    id: u64,
    snapshot: Snapshot,
    state: TransactionState,
}

impl Transaction {
    pub fn new(id: u64, snapshot: Snapshot) -> Self {
        Self {
            id,
            snapshot,
            state: TransactionState::Active,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn state(&self) -> TransactionState {
        self.state.clone()
    }
}

/// TransactionManager manages transaction lifecycle
///
/// - begin: allocate TxId, create snapshot, record in active list
/// - commit: remove from active list, mark committed
/// - abort: remove from active list, mark aborted
pub struct TransactionManager {
    tx_id_allocator: TransactionId,
    active_tx_ids: RwLock<HashSet<u64>>,
    // M10: 跟踪每个事务的未提交版本，按表聚合以支持多表事务回滚（MS07-T04）
    tx_versions: RwLock<HashMap<u64, HashMap<String, HashSet<RowId>>>>,
    // WAL buffer for writing transaction lifecycle records
    wal_buffer: RwLock<Option<Arc<WALBuffer>>>,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            tx_id_allocator: TransactionId::new(),
            active_tx_ids: RwLock::new(HashSet::new()),
            tx_versions: RwLock::new(HashMap::new()),
            wal_buffer: RwLock::new(None),
        }
    }

    pub async fn set_wal_buffer(&self, wal_buffer: Arc<WALBuffer>) {
        *self.wal_buffer.write().await = Some(wal_buffer);
    }

    /// Begin a new transaction
    ///
    /// - Allocates unique TxId
    /// - Records active transactions for snapshot
    /// - Creates snapshot
    pub async fn begin(&self) -> Transaction {
        let tx_id = self.tx_id_allocator.allocate();

        // WAL: write BeginTxn record
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            wal.append(WalRecord::BeginTxn { tx_id }).await;
        }

        // Get current active transactions for snapshot
        let active_ids: Vec<u64> = self.active_tx_ids.read().await.iter().copied().collect();

        // Add this transaction to active list
        self.active_tx_ids.write().await.insert(tx_id);

        let snapshot = Snapshot::new(tx_id, active_ids);
        Transaction::new(tx_id, snapshot)
    }

    /// Commit a transaction
    ///
    /// - Marks all versions as committed
    /// - Removes from active list
    /// - Clears tx_versions
    pub async fn commit(&self, tx: Transaction, buffer_pool: &BufferPool) -> Result<()> {
        let tx_id = tx.id();

        // WAL: write CommitTxn record and wait for persistence (Group Commit)
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            wal.append(WalRecord::CommitTxn { tx_id, timestamp }).await;
            wal.append_commit_and_wait(tx_id).await?;
        }

        // M10: Mark all versions as committed
        self.commit_mark_versions(tx_id, buffer_pool).await?;

        // Remove from active list
        let mut active = self.active_tx_ids.write().await;
        if !active.remove(&tx_id) {
            return Err(TransactionError::AlreadyCommitted(tx_id).into());
        }

        // Clear tx_versions
        self.tx_versions.write().await.remove(&tx_id);

        Ok(())
    }

    /// Abort a transaction
    ///
    /// - Cleans up uncommitted versions (M10), rolling back each table's
    ///   index entries via the resolved `TableMeta` map (MS07-T04)
    /// - Removes from active list
    /// - Clears tx_versions
    pub async fn abort(
        &self,
        tx: Transaction,
        buffer_pool: &BufferPool,
        tables: &HashMap<String, Arc<TableMeta>>,
    ) -> Result<()> {
        let tx_id = tx.id();
        self.abort_by_id(tx_id, buffer_pool, tables).await
    }

    /// Abort a transaction addressed by id (MS34 Iter000, T01).
    ///
    /// Same path as [`TransactionManager::abort`] — WAL `AbortTxn` record,
    /// uncommitted version cleanup (index entries restored, versions
    /// tombstoned), removal from the active list, `tx_versions` cleared —
    /// but the transaction is addressed by id instead of by an owned handle.
    /// The abandoned-implicit-transaction reclaim holds only the id (the
    /// statement future that owned the handle is gone).
    pub async fn abort_by_id(
        &self,
        tx_id: u64,
        buffer_pool: &BufferPool,
        tables: &HashMap<String, Arc<TableMeta>>,
    ) -> Result<()> {
        // WAL: write AbortTxn record (no need to wait for persistence)
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            wal.append(WalRecord::AbortTxn { tx_id }).await;
        }

        // M10: Cleanup uncommitted versions
        self.abort_cleanup_versions(tx_id, buffer_pool, tables)
            .await?;

        // Remove from active list
        let mut active = self.active_tx_ids.write().await;
        if !active.remove(&tx_id) {
            return Err(TransactionError::AlreadyAborted(tx_id).into());
        }

        // Clear tx_versions
        self.tx_versions.write().await.remove(&tx_id);

        Ok(())
    }

    /// Drop the version entries a transaction holds on the named tables
    /// (MS34 Iter000, T01).
    ///
    /// Used by the abandoned-implicit-transaction reclaim: a table that no
    /// longer exists (dropped after the statement was abandoned) has no data
    /// and no index entry left to restore — the drop already released them —
    /// so its version entries are removed from the manager before the abort
    /// runs. Otherwise the abort stops at the missing table meta and the
    /// transaction stays active.
    pub async fn discard_tx_version_tables(&self, tx_id: u64, table_names: &[String]) {
        let mut versions = self.tx_versions.write().await;
        let mut drop_tx = false;
        if let Some(entry) = versions.get_mut(&tx_id) {
            for name in table_names {
                entry.remove(name);
            }
            drop_tx = entry.is_empty();
        }
        if drop_tx {
            versions.remove(&tx_id);
        }
    }

    /// Get current active transactions
    pub async fn active_transactions(&self) -> Vec<u64> {
        self.active_tx_ids.read().await.iter().copied().collect()
    }

    /// Record a version created by this transaction (M10)
    ///
    /// Called by InsertExecutor/UpdateExecutor/DeleteExecutor when creating
    /// new versions. Versions are aggregated per table so an abort can roll
    /// back every table the transaction touched (MS07-T04).
    pub async fn record_version(&self, tx_id: u64, table_name: &str, row_id: RowId) {
        let mut versions = self.tx_versions.write().await;
        versions
            .entry(tx_id)
            .or_default()
            .entry(table_name.to_string())
            .or_default()
            .insert(row_id);
    }

    /// Get all versions recorded for a transaction, across all its tables
    /// (for testing)
    pub async fn get_tx_versions(&self, tx_id: u64) -> HashSet<RowId> {
        self.tx_versions
            .read()
            .await
            .get(&tx_id)
            .map(|tables| tables.values().flatten().copied().collect())
            .unwrap_or_default()
    }

    /// Get the per-table versions recorded for a transaction (for rollback)
    pub async fn tx_version_tables(&self, tx_id: u64) -> HashMap<String, HashSet<RowId>> {
        self.tx_versions
            .read()
            .await
            .get(&tx_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get tx_versions flattened across tables (for testing)
    pub async fn tx_versions(&self) -> HashMap<u64, HashSet<RowId>> {
        self.tx_versions
            .read()
            .await
            .iter()
            .map(|(tx_id, tables)| {
                (
                    *tx_id,
                    tables.values().flatten().copied().collect::<HashSet<_>>(),
                )
            })
            .collect()
    }

    /// Get current max TxId (for testing)
    pub fn current_tx_id(&self) -> u64 {
        self.tx_id_allocator.current()
    }

    /// Advance the tx id allocator past every id observed by recovery
    /// (MS09 Iter000 001-replan, D10): the next allocated id is strictly
    /// greater than `max_used`, so ids are never reused after a restart and
    /// the Read Committed high-water argument ("every id ≤ the allocator's
    /// current value is committed, aborted, or active") stays sound.
    pub fn advance_past(&self, max_used: u64) {
        self.tx_id_allocator.advance_past(max_used);
    }

    /// Commit by ID (for testing error cases)
    pub async fn commit_by_id(&self, tx_id: u64) -> Result<()> {
        let mut active = self.active_tx_ids.write().await;

        if !active.remove(&tx_id) {
            return Err(TransactionError::NotFound(tx_id).into());
        }

        Ok(())
    }

    /// Mark all versions as committed (M10)
    pub async fn commit_mark_versions(&self, tx_id: u64, buffer_pool: &BufferPool) -> Result<()> {
        let versions = self.tx_versions.read().await;
        let tx_versions: HashSet<RowId> = versions
            .get(&tx_id)
            .map(|tables| tables.values().flatten().copied().collect())
            .unwrap_or_default();

        for row_id in tx_versions {
            buffer_pool.write_commit_tx_id(row_id, tx_id).await?;
            // M21: Clear page visibility summary after COMMIT mark
            let page_id = PageId(row_id.page_id as u64);
            buffer_pool.clear_all_visible(page_id);
        }

        Ok(())
    }

    /// Cleanup uncommitted versions on abort (M10)
    ///
    /// For each uncommitted version created by this transaction, grouped by
    /// table (MS07-T04):
    /// - If it has a previous version, update index to point to previous
    /// - If it has no previous version, delete from index
    ///
    /// MS24 Iter001 replan (2.9/D9) 增加 B 趟：DELETE / REPLACE 的墓碑 slot
    /// 从不进入索引（条目在删除时已清理），`find_key_by_row_id` 恒为 `None`，
    /// 整段还原被跳过 → 回滚后被删行的 PK 与唯一条目不恢复（PK 等值点查漏行、
    /// 唯一值被释放致 `UNIQUE` 静默失效）。两趟划分在任何索引写入之前一次性
    /// 完成——这是处理顺序与 `record_version` 集合迭代顺序无关的唯一保证点。
    pub async fn abort_cleanup_versions(
        &self,
        tx_id: u64,
        buffer_pool: &BufferPool,
        tables: &HashMap<String, Arc<TableMeta>>,
    ) -> Result<()> {
        let versions = self.tx_versions.read().await;
        let tx_versions = versions.get(&tx_id).cloned().unwrap_or_default();
        drop(versions);

        for (table_name, row_ids) in tx_versions {
            let table_meta = tables.get(&table_name).ok_or_else(|| {
                StorageError::ExecutionError(format!(
                    "abort cleanup for tx {}: no table meta resolved for table '{}'",
                    tx_id, table_name
                ))
            })?;

            // A 趟 / B 趟划分（MS24 replan 2.9）：按「当前是否持有索引条目」
            // 分桶——A 趟为 INSERT / UPDATE 形态（条目在位，现状回退或移除），
            // B 趟为条目已随删除清理的墓碑（需从前驱存活版本还原）。
            //
            // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
            // Iteration 001（ISS24）：登记到 `tx_versions` 的版本之外，还要
            // 收回本事务在数据链上创建、但**未被登记**的版本——放弃窗口可能
            // 落在「写出版本槽」与「登记版本」之间，墓碑已在链上（按其删除者
            // 的事务状态压制前驱版本），而事务管理器看不到它 ⇒ 中止清理无法
            // 中和它 ⇒ 被放弃的改写隐藏了它已开始改写的行。
            let mut row_ids = row_ids;
            row_ids.extend(versions_created_by(buffer_pool, table_meta, tx_id).await?);

            let mut indexed = Vec::new();
            let mut tombstones = Vec::new();
            for row_id in row_ids {
                if table_meta
                    .index_manager
                    .find_key_by_row_id(row_id)
                    .await
                    .is_some()
                {
                    indexed.push(row_id);
                } else {
                    tombstones.push(row_id);
                }
            }

            for row_id in indexed {
                let header = buffer_pool.read_version_header(row_id).await?;
                let prev_row_id = header.next_version();

                // MS21 Iter000 (D1): 有前驱时从 prev 版本 tuple 派生真实
                // PK/唯一键——rekey UPDATE 回滚时反查键是新键而 prev tuple
                // 携带旧键，必须按派生键区分 rekey（删新键 + 按旧键回插）
                // 与非 rekey（既有 update 快路径）。派生不可得时按既有形态
                // 回退（见 `read_prev_index_keys`）。
                let derived = match prev_row_id {
                    Some(prev) => read_prev_index_keys(buffer_pool, table_meta, prev).await?,
                    None => None,
                };
                let derived_pk: Option<Option<&[u8]>> =
                    derived.as_ref().map(|(pk, _, _)| pk.as_deref());
                let derived_uniques: Vec<Option<Option<&[u8]>>> = match &derived {
                    Some((_, uniques, _)) => uniques.iter().map(|k| Some(k.as_deref())).collect(),
                    None => vec![None; table_meta.unique_indexes.len()],
                };
                let derived_secondaries: Vec<Option<Option<&[u8]>>> = match &derived {
                    Some((_, _, secondaries)) => {
                        secondaries.iter().map(|k| Some(k.as_deref())).collect()
                    }
                    None => vec![None; table_meta.secondary_indexes.len()],
                };

                let key = table_meta.index_manager.find_key_by_row_id(row_id).await;

                if let Some(key) = key {
                    match (derived_pk, prev_row_id) {
                        (Some(Some(dk)), Some(prev)) if dk == key.as_slice() => {
                            // 非 rekey 快路径：派生键与反查键逐字节相等——
                            // 既有 update 回退，行为逐字节不变。
                            table_meta.index_manager.update(&key, prev).await?;
                        }
                        (Some(Some(dk)), Some(prev)) => {
                            // rekey：反查键是新键而 prev tuple 携带旧键——
                            // 删新键条目后按旧键回插（delete 先于 insert：
                            // `row_to_key` 反向映射经 delete 清理）。
                            table_meta.index_manager.delete(&key).await?;
                            table_meta.index_manager.insert(dk, prev).await?;
                        }
                        (Some(None), Some(_)) => {
                            // prev 键位 NULL / 不可键控：仅删除不回插。
                            table_meta.index_manager.delete(&key).await?;
                        }
                        (None, Some(prev)) => {
                            // prev tuple 不可读（派生不可得）：既有形态回退。
                            table_meta.index_manager.update(&key, prev).await?;
                        }
                        (_, None) => {
                            table_meta.index_manager.delete(&key).await?;
                        }
                    }
                }

                // MS23 Iteration 001 (2.7/D8): 唯一索引同型修复——不修复则
                // 回滚的 INSERT 残留唯一条目，后续同值插入假阳性
                // DuplicateKey。MS21 Iter000 (D1): 与 PK 臂同型按派生键
                // 区分 rekey 回退。
                for ((_, uindex), derived_key) in table_meta
                    .unique_indexes
                    .iter()
                    .zip(derived_uniques.into_iter())
                {
                    if let Some(key) = uindex.find_key_by_row_id(row_id).await {
                        match (derived_key, prev_row_id) {
                            (Some(Some(dk)), Some(prev)) if dk == key.as_slice() => {
                                uindex.update(&key, prev).await?;
                            }
                            (Some(Some(dk)), Some(prev)) => {
                                uindex.delete(&key).await?;
                                uindex.insert(dk, prev).await?;
                            }
                            (Some(None), Some(_)) => {
                                uindex.delete(&key).await?;
                            }
                            (None, Some(prev)) => {
                                uindex.update(&key, prev).await?;
                            }
                            (_, None) => {
                                uindex.delete(&key).await?;
                            }
                        }
                    } else if let Some(prev) = prev_row_id {
                        // MS21 Iter001 (D17)：值→NULL rekey 的对称还原——
                        // 写入期已删除旧唯一条目且 NULL 不入索引，反查 None
                        // 落此处。prev 唯一键可派生且非 NULL 时纯插入还原旧
                        // 值占用（无 delete-先于-insert 次序约束）；键位
                        // NULL 或派生不可得时无条目可还原，跳过。
                        if let Some(Some(dk)) = derived_key {
                            uindex.insert(dk, prev).await?;
                        }
                    }
                }

                // MS21 Iter002 (T2-d/D7)：二级索引同型回退——分支形状与唯一
                // 臂一致，但写入一律 `delete_exact` + `insert`（D3：二级索引
                // 禁用歧义原语 `delete`/`update`；重复键并存时二者均不可定位
                // 唯一条目）。三支形态：
                // 1. 派生键 Some（含 ==反查键的非 rekey 与 ≠的 rekey）：按
                //    派生键换指 prev（delete_exact 删反查键条目 + insert 派
                //    生键指向 prev——delete 先于 insert 的反向映射次序约束
                //    同样适用）；
                // 2. prev 键位 NULL：仅删反查键条目；
                // 3. 派生不可得：按反查键换指 prev（与唯一臂 `update` 回退
                //    的终态等价——反查键条目确定在位）。
                // 反查 None 的对称还原分支（值→NULL 形态，镜像 D17）：prev
                // 键可派生且非 NULL 时纯插入还原。
                for (sindex, derived_key) in table_meta
                    .secondary_indexes
                    .iter()
                    .zip(derived_secondaries.into_iter())
                {
                    if let Some(key) = sindex.index.find_key_by_row_id(row_id).await {
                        match (derived_key, prev_row_id) {
                            (Some(Some(dk)), Some(prev)) => {
                                sindex.index.delete_exact(&key, row_id).await?;
                                sindex.index.insert(dk, prev).await?;
                            }
                            (Some(None), Some(_)) => {
                                sindex.index.delete_exact(&key, row_id).await?;
                            }
                            (None, Some(prev)) => {
                                sindex.index.delete_exact(&key, row_id).await?;
                                sindex.index.insert(&key, prev).await?;
                            }
                            (_, None) => {
                                sindex.index.delete_exact(&key, row_id).await?;
                            }
                        }
                    } else if let (Some(prev), Some(Some(dk))) = (prev_row_id, derived_key) {
                        sindex.index.insert(dk, prev).await?;
                    }
                }

                // MS07-T04: tombstone the aborted version. Index fixup alone
                // leaves the tuple in its data-page slot, and snapshot-less
                // scans (DataScan with `snapshot: None`) yield every slot
                // that is not deleted — the rolled-back row would stay
                // visible to `SELECT *`. Marking it aborted (MS09 Iter000
                // T4/D2: create_tx_id = 0 + delete sentinel) makes scans skip
                // it and — unlike the plain delete sentinel — marks it as
                // belonging to no transaction, so the tombstone never
                // suppresses the surviving predecessor versions.
                update_version_header_in_data_page(buffer_pool, row_id, header.mark_aborted(), &[])
                    .await?;
            }

            for row_id in tombstones {
                restore_tombstone_index_entries(buffer_pool, table_meta, tx_id, row_id).await?;

                // 与 A 趟同型的中性化：墓碑 slot 转为 aborted（create_tx_id = 0
                // + delete 哨兵），扫描跳过且不抑制前驱存活版本——回滚后
                // 「该删除未发生」。
                let header = buffer_pool.read_version_header(row_id).await?;
                update_version_header_in_data_page(buffer_pool, row_id, header.mark_aborted(), &[])
                    .await?;
            }
        }

        Ok(())
    }
}

/// 数据链上由 `tx_id` 创建的版本（墓碑与其新版本）。
///
/// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
/// Iteration 001（ISS24）：放弃（语句 future 被丢弃）可能落在「写出版本槽」
/// 与「登记版本到事务管理器」之间——版本槽已经在链上（墓碑按其删除者的事务
/// 状态压制前驱版本），而事务管理器看不到它 ⇒ 中止清理无法中和它 ⇒ 被放弃
/// 的改写隐藏了它已开始改写的行、且其主键条目不再被还原。
///
/// 按**创建者事务 id**在链上找回这些版本，使中止清理覆盖完整。已登记的版本
/// 同样在链上（重复登记由调用方的集合去重），因此本函数只增不减。
///
/// 终止条件：沿 `next_page_id` 走链至 terminal（`next_page_id == 0`）为止；
/// 损坏页 / 越界页按既有页读错误面显式传播，不静默。链成环时按既有
/// `data_chain_status` / `heal_stale_tail` 的形态显式报错，不无限走查。
async fn versions_created_by(
    buffer_pool: &BufferPool,
    table_meta: &Arc<TableMeta>,
    tx_id: u64,
) -> Result<Vec<RowId>> {
    let mut found = Vec::new();
    let mut visited: HashSet<PageId> = HashSet::new();
    let mut page_id = Some(table_meta.data_page_head);
    while let Some(pid) = page_id {
        if !visited.insert(pid) {
            return Err(StorageError::Internal(format!(
                "data page chain cycle while scanning versions of tx {} for table '{}'",
                tx_id, table_meta.name
            )));
        }
        let (next_page, rids) = buffer_pool
            .with_page_data(pid, |data| -> Result<(u32, Vec<RowId>)> {
                let slotted = SlottedPageRef::new(data);
                // MS37 T17: 页头槽数越界 ⇒ 明确错误（取槽**之前**判；否则
                // 槽目录定位下溢 ⇒ 越界切片 panic ⇒ 进程 abort）。
                if slotted.slot_count_exceeds_capacity() {
                    return Err(corrupted_data_page_slot_count_error(
                        pid.0,
                        slotted.slot_count(),
                    ));
                }
                let mut rids = Vec::new();
                for index in 0..slotted.slot_count() {
                    let Some(slot) = slotted.get_slot(index) else {
                        continue;
                    };
                    // MS37 T3: 越界槽 ⇒ 明确错误（不得跳过——跳过 = 静默错读）。
                    let slot_data = slotted
                        .get_slot_data_checked(&slot)
                        .ok_or_else(|| corrupted_data_page_slot_error(pid.0, index, &slot))?;
                    if slot_data.len() < VersionHeader::SIZE {
                        continue;
                    }
                    let Some(vh) = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                    else {
                        continue;
                    };
                    if vh.create_tx_id() == tx_id {
                        rids.push(RowId::new(pid.0 as u32, slot.logical_id));
                    }
                }
                Ok((slotted.header().next_page_id, rids))
            })
            .await?;
        found.extend(rids);
        page_id = if next_page == 0 {
            None
        } else {
            Some(PageId(next_page as u64))
        };
    }
    Ok(found)
}

/// 墓碑行索引条目还原（MS24 Iter001 replan 2.9 / design D9）：从墓碑的
/// `next_version()` 出发回溯版本链，跳过本事务创建的版本，取首个非本事务
/// 版本为还原目标，经 `wal::recovery::extract_index_keys` 派生 PK 键与各
/// 唯一列键后逐项 `insert` 还原（条目已在删除时移除）。
///
/// 终止与跳过条件（均不报错，与既有 `SlotNotFound` 容忍同型）：
/// - 墓碑无前驱（`next_version()` 为 `None`）——本事务插入后同事务删除的
///   新行形态，无存活版本可还原；
/// - 链上全部版本均由本事务创建——同上；
/// - 链回溯超出 `MAX_TOMBSTONE_CHAIN_WALK` 上限（仅防御损坏库的无环性）；
/// - slot 缺失 / 目标版本本身是墓碑 / 键不可键控（无键行、NULL 唯一值）——
///   对应位 `None`，`extract_index_keys` 已按此语义返回。
async fn restore_tombstone_index_entries(
    buffer_pool: &BufferPool,
    table_meta: &Arc<TableMeta>,
    tx_id: u64,
    tombstone_row_id: RowId,
) -> Result<()> {
    let unique_cols: Vec<usize> = table_meta
        .unique_indexes
        .iter()
        .map(|(col_idx, _)| *col_idx)
        .collect();

    let mut cursor = buffer_pool
        .read_version_header(tombstone_row_id)
        .await?
        .next_version();
    let mut steps = 0usize;

    while let Some(row_id) = cursor {
        if steps >= MAX_TOMBSTONE_CHAIN_WALK {
            break;
        }
        steps += 1;

        // 单次页读同时取出版本头与（命中还原目标时的）元组字节。
        let read = read_tuple_from_data_page(buffer_pool, row_id, |header, bytes| {
            let is_target = header.create_tx_id() != tx_id && !header.is_deleted();
            Ok((header, is_target.then(|| bytes.to_vec())))
        })
        .await;

        let Ok((header, tuple)) = read else {
            break;
        };

        if header.create_tx_id() == tx_id {
            // 本事务创建的版本（update→delete 形态）——继续回溯。
            cursor = header.next_version();
            continue;
        }

        if let Some(tuple) = tuple {
            let (pk_key, unique_keys, secondary_keys) =
                crate::wal::recovery::extract_index_keys(table_meta, &unique_cols, &tuple);
            if let Some(pk_key) = pk_key {
                table_meta.index_manager.insert(&pk_key, row_id).await?;
            }
            for ((_, uindex), key) in table_meta.unique_indexes.iter().zip(unique_keys) {
                if let Some(key) = key {
                    uindex.insert(&key, row_id).await?;
                }
            }
            // MS21 Iter002 (T2-d/D7)：二级条目同型回插（条目已在删除时以
            // `delete_exact` 清理；NULL 值无条目自然跳过）。
            for (sindex, key) in table_meta.secondary_indexes.iter().zip(secondary_keys) {
                if let Some(key) = key {
                    sindex.index.insert(&key, row_id).await?;
                }
            }
        }
        break;
    }

    Ok(())
}

/// MS21 Iter000 (design D1)：单次页读 prev 版本 tuple 并派生其真实 PK 键
/// 与各唯一列键（B 趟 `restore_tombstone_index_entries` 同型单次页读 +
/// `wal::recovery::extract_index_keys` 单一派生源，键派生规则零变化）。
///
/// MS21 Iter002 (T2-d/D7)：签名兼提各二级列键（与唯一键同一派生调用，
/// 无第二派生源）。
///
/// `Ok(None)`（派生不可得，调用方按既有形态回退——`update` 反查键）：
/// - slot 不可读（`SlotNotFound`，夹具/损坏态可达）；
/// - tuple 按表 schema 不可反序列化——变更前 A 趟从不读 prev tuple，且
///   直构夹具（`mvcc_abort_test` 等）的条目键为手写字节、与 tuple 内容
///   无关，该角落行为必须与变更前逐字节一致。
///
/// 反序列化成功但键位 NULL / 不可键控 → `Ok(Some((None, …)))`：调用方仅
/// 删除不回插（真实键位 NULL 形态）。其余错误传播（与既有错误面同型）。
async fn read_prev_index_keys(
    buffer_pool: &BufferPool,
    table_meta: &Arc<TableMeta>,
    prev_row_id: RowId,
) -> Result<Option<(Option<Vec<u8>>, Vec<Option<Vec<u8>>>, Vec<Option<Vec<u8>>>)>> {
    let unique_cols: Vec<usize> = table_meta
        .unique_indexes
        .iter()
        .map(|(col_idx, _)| *col_idx)
        .collect();

    let tuple =
        match read_tuple_from_data_page(buffer_pool, prev_row_id, |_, bytes| Ok(bytes.to_vec()))
            .await
        {
            Ok(tuple) => tuple,
            Err(StorageError::SlotNotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };

    let schema: Vec<crate::storage::page_format::ColumnType> = table_meta
        .columns
        .iter()
        .map(|(_, ct)| ct.clone())
        .collect();
    // MS21 Iter003 (T3-a/D9)：宽度感知校验——窄行（ADD COLUMN 前写入）是
    // 合法形态不再判失败；撕裂/错位行仍 None 回退。
    if crate::storage::page_format::deserialize_tuple_width(&tuple, &schema).is_err() {
        return Ok(None);
    }

    Ok(Some(crate::wal::recovery::extract_index_keys(
        table_meta,
        &unique_cols,
        &tuple,
    )))
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{write_tuple_to_data_page, BufferPool, FileStorage, TableManager};
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Create a test buffer pool for tests that need it
    fn create_test_buffer_pool() -> Arc<BufferPool> {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        Arc::new(BufferPool::new(10, storage).unwrap())
    }

    /// Create a test table for abort tests
    async fn create_test_table(buffer_pool: Arc<BufferPool>) -> Arc<TableMeta> {
        let storage = buffer_pool.storage().clone();
        let table_manager = TableManager::new(buffer_pool.clone(), storage)
            .await
            .unwrap();
        table_manager
            .create_table(
                "test_table",
                vec![("id".to_string(), crate::storage::ColumnType::Int)],
                "id",
            )
            .await
            .unwrap();
        table_manager.get_table("test_table").await.unwrap()
    }

    #[tokio::test]
    async fn test_transaction_begin() {
        let manager = TransactionManager::new();

        let tx = manager.begin().await;

        assert!(tx.id() > 0);
        assert_eq!(tx.state(), TransactionState::Active);
        assert!(tx.snapshot().tx_id() > 0);
    }

    #[tokio::test]
    async fn test_transaction_commit() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();

        let tx = manager.begin().await;
        let tx_id = tx.id();

        manager.commit(tx, &buffer_pool).await.unwrap();

        // Verify transaction not in active list
        let active = manager.active_transactions().await;
        assert!(!active.contains(&tx_id));
    }

    #[tokio::test]
    async fn test_transaction_abort() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        let tx = manager.begin().await;
        let tx_id = tx.id();

        let tables = HashMap::from([("test_table".to_string(), table_meta)]);
        manager.abort(tx, &buffer_pool, &tables).await.unwrap();

        // Verify transaction not in active list
        let active = manager.active_transactions().await;
        assert!(!active.contains(&tx_id));
    }

    #[tokio::test]
    async fn test_transaction_multiple() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        let tx1 = manager.begin().await;
        let tx2 = manager.begin().await;
        let tx3 = manager.begin().await;

        // IDs should be unique and increasing
        assert!(tx1.id() < tx2.id());
        assert!(tx2.id() < tx3.id());

        // All three should be in active list
        let active = manager.active_transactions().await;
        assert!(active.contains(&tx1.id()));
        assert!(active.contains(&tx2.id()));
        assert!(active.contains(&tx3.id()));
        assert_eq!(active.len(), 3);

        // Commit tx1 and tx3, abort tx2
        manager.commit(tx1, &buffer_pool).await.unwrap();
        let tables = HashMap::from([("test_table".to_string(), table_meta)]);
        manager.abort(tx2, &buffer_pool, &tables).await.unwrap();
        manager.commit(tx3, &buffer_pool).await.unwrap();

        // Active list should be empty
        let active = manager.active_transactions().await;
        assert_eq!(active.len(), 0);
    }

    #[tokio::test]
    async fn test_transaction_snapshot_active_list() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        // Start two transactions
        let tx1 = manager.begin().await;
        let tx1_id = tx1.id();

        // tx1 should see empty active list (no prior active transactions)
        let snap1_active = tx1.snapshot().tx_id();
        assert_eq!(snap1_active, tx1_id);

        // Start tx2 after tx1
        let tx2 = manager.begin().await;
        let _tx2_id = tx2.id();

        // tx2's snapshot should include tx1 in active list
        // But our current snapshot implementation doesn't track active_ids separately from tx_id
        // The key property: tx2.is_visible(tx1_id, None) should be false (tx1 not committed)

        // Start tx3
        let tx3 = manager.begin().await;

        // Commit tx1
        manager.commit(tx1, &buffer_pool).await.unwrap();

        // tx2 and tx3 snapshots were taken before tx1 committed
        // They should still see tx1 as not visible (based on snapshot rules)

        let tables = HashMap::from([("test_table".to_string(), table_meta)]);
        manager.abort(tx2, &buffer_pool, &tables).await.unwrap();
        manager.commit(tx3, &buffer_pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_double_commit_error() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();

        let tx = manager.begin().await;
        manager.commit(tx, &buffer_pool).await.unwrap();

        // Second commit on same tx_id should fail
        // But we can't reuse the same tx object after commit (it was moved)
        // Test with a non-existent tx_id
        let result = manager.commit_by_id(999).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tx_versions_initialization() {
        let manager = TransactionManager::new();
        // tx_versions should be empty initially
        assert!(manager.tx_versions().await.is_empty());
    }

    #[tokio::test]
    async fn test_record_version_single() {
        let manager = TransactionManager::new();
        let row_id = RowId::new(1, 0);
        manager.record_version(1, "t1", row_id).await;
        let versions = manager.get_tx_versions(1).await;
        assert!(versions.contains(&row_id));
        assert_eq!(versions.len(), 1);
    }

    #[tokio::test]
    async fn test_record_version_multiple() {
        let manager = TransactionManager::new();
        let row_id1 = RowId::new(1, 0);
        let row_id2 = RowId::new(2, 0);
        manager.record_version(1, "t1", row_id1).await;
        manager.record_version(1, "t1", row_id2).await;
        let versions = manager.get_tx_versions(1).await;
        assert!(versions.contains(&row_id1));
        assert!(versions.contains(&row_id2));
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn test_record_version_multiple_tables() {
        let manager = TransactionManager::new();
        manager.record_version(1, "t1", RowId::new(1, 0)).await;
        manager.record_version(1, "t2", RowId::new(2, 0)).await;
        manager.record_version(1, "t1", RowId::new(3, 0)).await;

        // Union view keeps the legacy flat per-tx shape.
        let versions = manager.get_tx_versions(1).await;
        assert_eq!(versions.len(), 3);

        // Per-table view separates the two tables.
        let tables = manager.tx_version_tables(1).await;
        assert_eq!(tables.get("t1").unwrap().len(), 2);
        assert_eq!(tables.get("t2").unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_abort_cleanup_multi_table() {
        use crate::transaction::VersionHeader;

        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let storage = buffer_pool.storage().clone();
        let table_manager = TableManager::new(buffer_pool.clone(), storage)
            .await
            .unwrap();
        for name in ["t1", "t2"] {
            table_manager
                .create_table(
                    name,
                    vec![("id".to_string(), crate::storage::ColumnType::Int)],
                    "id",
                )
                .await
                .unwrap();
        }
        let t1 = table_manager.get_table("t1").await.unwrap();
        let t2 = table_manager.get_table("t2").await.unwrap();

        let tx = manager.begin().await;
        let tx_id = tx.id();

        // One uncommitted insert per table, each registered in its index.
        let tuple = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let rid1 =
            write_tuple_to_data_page(&buffer_pool, &t1, &VersionHeader::new(tx_id, None), &tuple)
                .await
                .unwrap();
        let rid2 =
            write_tuple_to_data_page(&buffer_pool, &t2, &VersionHeader::new(tx_id, None), &tuple)
                .await
                .unwrap();
        manager.record_version(tx_id, "t1", rid1).await;
        manager.record_version(tx_id, "t2", rid2).await;
        t1.index_manager.insert(b"1", rid1).await.unwrap();
        t2.index_manager.insert(b"2", rid2).await.unwrap();

        let metas = HashMap::from([
            ("t1".to_string(), t1.clone()),
            ("t2".to_string(), t2.clone()),
        ]);
        manager.abort(tx, &buffer_pool, &metas).await.unwrap();

        // Both index entries are rolled back; version bookkeeping is cleared.
        assert_eq!(t1.index_manager.search(b"1").await.unwrap(), None);
        assert_eq!(t2.index_manager.search(b"2").await.unwrap(), None);
        assert!(manager.get_tx_versions(tx_id).await.is_empty());

        // Both aborted versions are tombstoned so snapshot-less scans skip
        // them (no residue after rollback).
        assert!(buffer_pool
            .read_version_header(rid1)
            .await
            .unwrap()
            .is_deleted());
        assert!(buffer_pool
            .read_version_header(rid2)
            .await
            .unwrap()
            .is_deleted());
    }

    #[tokio::test]
    async fn test_abort_cleanup_missing_table_meta_errors() {
        use crate::transaction::VersionHeader;

        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        let tx = manager.begin().await;
        let tx_id = tx.id();
        let tuple = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let rid = write_tuple_to_data_page(
            &buffer_pool,
            &table_meta,
            &VersionHeader::new(tx_id, None),
            &tuple,
        )
        .await
        .unwrap();
        manager.record_version(tx_id, "test_table", rid).await;

        // No meta resolved for the recorded table: abort must surface an
        // error instead of silently leaving a dangling index entry.
        let empty = HashMap::new();
        let err = manager.abort(tx, &buffer_pool, &empty).await.unwrap_err();
        assert!(err.to_string().contains("test_table"), "got: {}", err);
    }

    #[tokio::test]
    async fn test_get_tx_versions_empty() {
        let manager = TransactionManager::new();
        let versions = manager.get_tx_versions(999).await;
        assert!(versions.is_empty());
    }

    // MS09 Iter000 001-replan (T6-R3/D10): after recovery hands the max used
    // tx id to the allocator, `current_tx_id` reflects the watermark and the
    // next begun transaction gets an id strictly above every recovered id.

    #[tokio::test]
    async fn test_advance_past_reflected_in_current_and_next_begin() {
        let manager = TransactionManager::new();
        assert_eq!(manager.current_tx_id(), 0);

        manager.advance_past(42);
        assert_eq!(manager.current_tx_id(), 42);

        let tx = manager.begin().await;
        assert_eq!(tx.id(), 43);
    }
}
