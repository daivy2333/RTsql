//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::btree::IndexManager;
use crate::storage::page_format::{deserialize_row, ColumnType};
use crate::storage::{
    read_tuple_from_data_page, update_version_header_in_data_page, write_tuple_to_data_page,
    BufferPool, PageId, Result, RowId, StorageError, TableMeta,
};
use crate::transaction::{TransactionManager, VersionHeader};
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

pub struct DeleteExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    /// 定位键；`None` ＝ 无 WHERE 全表形态（change 2026-10-04-external-review-gaps
    /// T6——`scan_all` 枚举 + 逐行复用既有单行逻辑）
    key: Option<Vec<u8>>,
    tx_id: u64,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl DeleteExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        key: Option<Vec<u8>>,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        Self {
            table_meta,
            buffer_pool,
            tx_manager,
            key,
            tx_id,
            executed: false,
            wal_buffer,
        }
    }

    /// MS23 Iteration 001 (2.7/D7): extract the deleted row's UNIQUE column
    /// keys from the row's data-page slot, before the tombstone write.
    /// Best-effort like the PK path: an unreadable slot (SlotNotFound
    /// tolerance, test fixtures) yields no keys and the unique deletions are
    /// skipped. NULL values never have entries (no key).
    ///
    /// MS21 Iter002 (T2-d/D7): 同一次 tuple 反序列化兼提各二级列键（与唯一
    /// 键同一派生趟；条目清理由调用方以 `delete_exact` 执行——重复键并存下
    /// `delete(key)` 歧义原语禁用）。
    async fn unique_keys_of_row(
        &self,
        rid: RowId,
    ) -> Result<(
        Vec<(Arc<IndexManager>, Vec<u8>)>,
        Vec<(Arc<IndexManager>, Vec<u8>)>,
    )> {
        if self.table_meta.unique_indexes.is_empty() && self.table_meta.secondary_indexes.is_empty()
        {
            return Ok((Vec::new(), Vec::new()));
        }
        let tuple =
            match read_tuple_from_data_page(&self.buffer_pool, rid, |_, bytes| Ok(bytes.to_vec()))
                .await
            {
                Ok(bytes) => bytes,
                Err(StorageError::SlotNotFound(_)) => return Ok((Vec::new(), Vec::new())),
                Err(e) => return Err(e),
            };
        let schema: Vec<ColumnType> = self
            .table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        // MS21 Iter003 (T3-a/D9): width-aware read-back + DEFAULT overlay
        // ——窄行的键派生（含尾列二级索引的 DEFAULT 键）与扫描读回同源。
        //
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 003（design D7 / ISS25）：在途窄化的表被读 ⇒ **明确拒绝**
        // ——DML 的旧行读取不得按位置解码出错位的值。
        self.table_meta.narrowing.ensure_readable()?;
        let values = deserialize_row(&tuple, &schema, &self.table_meta.defaults)?;
        let mut keys = Vec::new();
        for (col_idx, uindex) in &self.table_meta.unique_indexes {
            if let Some(key) = values[*col_idx].to_key() {
                keys.push((uindex.clone(), key.as_bytes().to_vec()));
            }
        }
        let mut secondary_keys = Vec::new();
        for sindex in &self.table_meta.secondary_indexes {
            let column = sindex.column_index;
            if let Some(key) = values[column].secondary_index_key(&schema[column]) {
                secondary_keys.push((sindex.index.clone(), key.as_bytes().to_vec()));
            }
        }
        Ok((keys, secondary_keys))
    }
}

#[async_trait::async_trait]
impl Executor for DeleteExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        // change 2026-10-04-external-review-gaps T6：单行形态（Some）走既有
        // 定位路径；全表形态（None）以 `scan_all()` 枚举**语句开始时点**的
        // 全部索引键（与单行同源的可见性快照），逐行复用既有单行逻辑（含
        // 未命中零副作用臂贡献 0），行数汇总为 affected_rows。语句中途失败
        // 沿 DML 根事务包裹整体回滚。
        match self.key.take() {
            Some(key) => {
                let affected = self.delete_row(&key).await?;
                Ok(Some(ExecResult::AffectedRows(affected)))
            }
            None => {
                let entries = self.table_meta.index_manager.scan_all().await?;
                let mut affected = 0u64;
                for (key, _rid) in entries {
                    affected += self.delete_row(&key).await?;
                }
                Ok(Some(ExecResult::AffectedRows(affected)))
            }
        }
    }
}

impl DeleteExecutor {
    /// 既有单行删除路径逐字节承载（T6 提取共享——禁止复制墓碑/索引清理序）：
    /// 定位、未命中零副作用早退、墓碑写、版本登记、链指针补写、PK/唯一/二级条目
    /// 清理、WAL。返回实际删除行数（命中 1 / 未命中 0）。
    async fn delete_row(&mut self, key: &[u8]) -> Result<u64> {
        // Search for row_id before deleting
        let row_id = self.table_meta.index_manager.search(key).await?;

        // 未命中键零副作用早退（MS21 Iter001 D16）：search 未命中时返回
        // affected 0，不触碰墓碑/唯一/PK 条目/WAL。此前无条件
        // `index_manager.delete` 会以「首个 ≥」插入位误删邻近条目；键大于
        // 全部键时上抛 KeyNotFound 误报。命中路径逐字节不变。
        if row_id.is_none() {
            return Ok(0);
        }

        // MS23 Iteration 001 (2.7/D7): 唯一列键值提取——写墓碑前从 rid 处
        // slot 数据反序列化行元组（SlotNotFound 容忍路径无元组可读，与 PK
        // 同型跳过唯一删除）。MS21 Iter002 (T2-d/D7)：同趟兼提二级列键。
        let (unique_keys, secondary_keys) = match row_id.as_ref() {
            Some(rid) => self.unique_keys_of_row(*rid).await?,
            None => (Vec::new(), Vec::new()),
        };

        // MS09 Iter000 (D1, I033): express the delete as an independent
        // tombstone version slot (create_tx = deleter, commit = delete
        // sentinel, chain pointer -> deleted row). The deleted version's own
        // header is no longer overwritten: while the deleter is uncommitted,
        // snapshot-less scans fall through to the pre-delete version, and the
        // tombstone stays self-describing for the deleter-commit-state
        // suppression decision. The header read doubles as the existence
        // probe for the pre-existing SlotNotFound tolerance (test fixtures).
        //
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 005 (ISS24, DML 同形面)：写序与 DDL 改写路径
        // （`src/storage/data/table_manager.rs` 的 `alter_drop_column` DELETE
        // 腿）**同形**——不带链指针写 → 登记 → 补链指针 → 再清理索引条目。
        // 放弃（语句 future 被丢弃）可能落在「写出版本槽」与「登记版本」之间：
        // 带链指针的墓碑按删除者的事务状态压制其前驱版本，而回收的覆盖面由
        // 登记的时点决定 ⇒ 看不到未登记的墓碑 ⇒ 被放弃的 `DELETE` 隐藏了它
        // 已删除的那一行。不带链指针的墓碑不压制任何前驱版本 ⇒ 该窗口无害。
        // 槽的终态内容（create_tx / 删除哨兵 / 链指针）与修复前逐字节相同。
        let mut tombstone_rid: Option<RowId> = None;
        if let Some(rid) = &row_id {
            match self.buffer_pool.read_version_header(*rid).await {
                Ok(_) => {
                    let tombstone = VersionHeader::new(self.tx_id, None).mark_deleted();
                    let written = write_tuple_to_data_page(
                        &self.buffer_pool,
                        &self.table_meta,
                        &tombstone,
                        &[],
                    )
                    .await?;
                    self.buffer_pool
                        .clear_all_visible(PageId(written.page_id as u64));
                    self.buffer_pool
                        .clear_all_visible(PageId(rid.page_id as u64));
                    tombstone_rid = Some(written);
                }
                Err(StorageError::SlotNotFound(_)) => {
                    // Data page/slot doesn't exist — skip the tombstone write.
                    // The index entry will still be removed, which is sufficient
                    // for PK lookup correctness.
                }
                Err(e) => return Err(e),
            }
        }

        // 登记在链指针之前——登记之后回收才看得见这个版本（放弃窗口内的墓碑
        // 必须能被中和）。登记对象与既有形态逐字节相同：写了墓碑就登记墓碑槽，
        // 否则登记被删行的 rid（slot-missing 容忍臂的既有形状）。
        if let Some(rid) = row_id {
            let recorded = tombstone_rid.unwrap_or(rid);
            self.tx_manager
                .record_version(self.tx_id, &self.table_meta.name, recorded)
                .await;
        }

        // 补链指针：墓碑指向被删的旧行（终态与修复前逐字节相同）。
        if let (Some(tombstone), Some(rid)) = (tombstone_rid, row_id) {
            update_version_header_in_data_page(
                &self.buffer_pool,
                tombstone,
                VersionHeader::new(self.tx_id, None)
                    .with_next_version(rid)
                    .mark_deleted(),
                &[],
            )
            .await?;
        }

        // 索引条目清理在链指针之后——放弃窗口落在链指针之前时，条目仍在位。
        self.table_meta.index_manager.delete(key).await?;

        // MS23 Iteration 001 (2.7/D7): 删 PK 条目同区逐唯一列移除条目——
        // 不移除则残留条目使后续同值插入假阳性 DuplicateKey。
        for (uindex, key) in &unique_keys {
            uindex.delete(key).await?;
        }

        // MS21 Iter002 (T2-d/D7)：逐二级索引以 `delete_exact` 清理被删行的
        // 条目（按 tuple 派生键精确定位；重复键并存下其余条目不受扰）。
        for (sindex, key) in &secondary_keys {
            if let Some(rid) = row_id {
                sindex.delete_exact(key, rid).await?;
            }
        }

        // M10: record the version object for commit/abort bookkeeping — the
        // tombstone slot when one was written, the deleted row's rid
        // otherwise (slot-missing tolerance keeps its pre-change shape).
        // commit() keeps the tombstone sentinel (version_chain commit guard);
        // abort neutralizes the tombstone slot (manager abort cleanup).
        //
        // WAL: Delete record carrying the deleted row's rid (record
        // format and field semantics unchanged; recovery rebuilds the
        // tombstone slot from it). BeginTxn/CommitTxn are written by
        // TransactionManager::begin()/commit() (the single source of
        // truth).
        if let Some(rid) = row_id {
            if let Some(wal) = &self.wal_buffer {
                wal.append(WalRecord::Delete {
                    tx_id: self.tx_id,
                    table_name: self.table_meta.name.clone(),
                    row_id: rid,
                })
                .await;
            }
        }

        Ok(1)
    }
}
