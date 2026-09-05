//! 恢复管理器
//!
//! 负责启动时重放 WAL，恢复未完成事务

use super::{WalError, WalReader, WalRecord};
use crate::storage::{write_tuple_to_data_page, BufferPool, TableManager};
use crate::transaction::VersionHeader;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// 恢复结果
#[derive(Debug, Default)]
pub struct RecoveryResult {
    pub committed_tx_ids: HashSet<u64>,
    pub aborted_tx_ids: HashSet<u64>,
    pub uncommitted_tx_ids: HashSet<u64>,
    pub redo_count: usize,
}

/// 恢复管理器
pub struct RecoveryManager;

impl RecoveryManager {
    /// 基础恢复：仅识别已提交/已回滚事务
    pub fn recover(db_path: &Path) -> Result<(HashSet<u64>, HashSet<u64>), WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok((HashSet::new(), HashSet::new()));
        }

        let mut reader = WalReader::open(&wal_path)?;
        let records = reader.read_all()?;

        let mut committed_tx_ids = HashSet::new();
        let mut aborted_tx_ids = HashSet::new();

        for record in records {
            match record {
                WalRecord::Commit { tx_id, .. } | WalRecord::CommitTxn { tx_id, .. } => {
                    committed_tx_ids.insert(tx_id);
                }
                WalRecord::Abort { tx_id } | WalRecord::AbortTxn { tx_id } => {
                    aborted_tx_ids.insert(tx_id);
                }
                _ => {}
            }
        }

        Ok((committed_tx_ids, aborted_tx_ids))
    }

    /// 完整恢复：Redo committed 事务 + 清理 uncommitted 事务
    ///
    /// 策略：
    /// 1. 扫描 WAL 识别 committed/aborted/uncommitted 事务
    /// 2. Redo committed 事务的 Insert/Update 操作（幂等）
    /// 3. Mark uncommitted 事务的 tuple 为 aborted
    pub async fn full_recover(
        db_path: &Path,
        buffer_pool: Arc<BufferPool>,
        table_manager: Arc<TableManager>,
    ) -> Result<RecoveryResult, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(RecoveryResult::default());
        }

        // 消费 checkpoint 位点（16B 语义与 CheckpointManager 一致）：
        // 位点缺失/损坏（<16B）/ LSN 超出 WAL 文件长度（代际失效）→ 全量重放（0）；
        // 有效位点语义 = 只重放记录偏移 ≥ site 的数据记录（位点前缀已由刷脏页覆盖）
        let site = super::checkpoint::read_site_file(&db_path.with_extension("checkpoint"))?;
        let wal_len = std::fs::metadata(&wal_path)
            .map_err(|e| WalError::IoError(e.to_string()))?
            .len();
        let redo_from = match site {
            Some((lsn, _)) if lsn <= wal_len => lsn,
            _ => 0,
        };

        let mut reader = WalReader::open(&wal_path)?;
        let records = reader.read_all_with_lsn()?;

        if records.is_empty() {
            return Ok(RecoveryResult::default());
        }

        // Step 1: Classify transactions（分类始终覆盖全部记录，不因位点裁剪）
        let mut all_tx_ids = HashSet::new();
        let mut committed_tx_ids = HashSet::new();
        let mut aborted_tx_ids = HashSet::new();
        let mut data_records: Vec<(u64, &WalRecord)> = Vec::new();

        for (lsn, record) in &records {
            match record {
                WalRecord::BeginTxn { tx_id } => {
                    all_tx_ids.insert(*tx_id);
                }
                WalRecord::Commit { tx_id, .. } | WalRecord::CommitTxn { tx_id, .. } => {
                    committed_tx_ids.insert(*tx_id);
                }
                WalRecord::Abort { tx_id } | WalRecord::AbortTxn { tx_id } => {
                    aborted_tx_ids.insert(*tx_id);
                }
                WalRecord::Insert { .. } | WalRecord::Update { .. } | WalRecord::Delete { .. } => {
                    data_records.push((*lsn, record));
                }
                _ => {}
            }
        }

        let uncommitted_tx_ids: HashSet<u64> =
            all_tx_ids.difference(&committed_tx_ids).cloned().collect();
        let uncommitted_tx_ids: HashSet<u64> = uncommitted_tx_ids
            .difference(&aborted_tx_ids)
            .cloned()
            .collect();

        // Step 2: Redo committed transactions after the checkpoint site
        // （K05 显式化：任何 redo 失败立即返回 Err，不再静默吞掉）
        let mut redo_count = 0;
        for (lsn, record) in &data_records {
            let tx_id = record.tx_id();
            if committed_tx_ids.contains(&tx_id) && *lsn >= redo_from {
                Self::redo_record(record, &buffer_pool, &table_manager).await?;
                redo_count += 1;
            }
        }

        // Step 3: Mark uncommitted tuples as aborted
        Self::mark_uncommitted_aborted(&uncommitted_tx_ids, &buffer_pool).await?;

        Ok(RecoveryResult {
            committed_tx_ids,
            aborted_tx_ids,
            uncommitted_tx_ids,
            redo_count,
        })
    }

    /// 重放单条 WAL 记录
    ///
    /// 表缺失或页/索引操作失败时显式报错（K05：恢复不再静默吞错）
    async fn redo_record(
        record: &WalRecord,
        buffer_pool: &Arc<BufferPool>,
        table_manager: &Arc<TableManager>,
    ) -> Result<(), WalError> {
        match record {
            WalRecord::Insert {
                table_name,
                row_id: _,
                tuple_data,
                tx_id,
            } => {
                let table_meta = table_manager.get_table(table_name).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "table '{}' lookup failed during redo: {}",
                        table_name, e
                    ))
                })?;
                let version_header = VersionHeader::new(*tx_id, None);
                write_tuple_to_data_page(buffer_pool, &table_meta, &version_header, tuple_data)
                    .await
                    .map_err(|e| {
                        WalError::RedoFailed(format!(
                            "insert redo into table '{}' failed: {}",
                            table_name, e
                        ))
                    })?;
                Ok(())
            }
            WalRecord::Update {
                table_name,
                row_id: _,
                new_tuple,
                tx_id,
                ..
            } => {
                let table_meta = table_manager.get_table(table_name).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "table '{}' lookup failed during redo: {}",
                        table_name, e
                    ))
                })?;
                let version_header = VersionHeader::new(*tx_id, None);
                write_tuple_to_data_page(buffer_pool, &table_meta, &version_header, new_tuple)
                    .await
                    .map_err(|e| {
                        WalError::RedoFailed(format!(
                            "update redo into table '{}' failed: {}",
                            table_name, e
                        ))
                    })?;
                Ok(())
            }
            WalRecord::Delete {
                table_name, row_id, ..
            } => {
                let table_meta = table_manager.get_table(table_name).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "table '{}' lookup failed during redo: {}",
                        table_name, e
                    ))
                })?;
                if let Some(key) = table_meta.index_manager.find_key_by_row_id(*row_id).await {
                    table_meta.index_manager.delete(&key).await.map_err(|e| {
                        WalError::RedoFailed(format!(
                            "delete redo of table '{}' row {:?} failed: {}",
                            table_name, row_id, e
                        ))
                    })?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Mark all uncommitted tuples as aborted so MVCC skips them
    async fn mark_uncommitted_aborted(
        uncommitted_tx_ids: &HashSet<u64>,
        buffer_pool: &Arc<BufferPool>,
    ) -> Result<(), WalError> {
        for tx_id in uncommitted_tx_ids {
            buffer_pool.mark_tx_aborted(*tx_id).await.map_err(|e| {
                WalError::RedoFailed(format!("mark tx {} aborted failed: {}", tx_id, e))
            })?;
        }
        Ok(())
    }

    /// 检查是否需要恢复
    pub fn needs_recovery(db_path: &Path) -> Result<bool, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(false);
        }

        let metadata =
            std::fs::metadata(&wal_path).map_err(|e| WalError::IoError(e.to_string()))?;

        Ok(metadata.len() > 0)
    }

    /// 读取所有 WAL 记录
    pub fn read_wal(db_path: &Path) -> Result<Vec<WalRecord>, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(Vec::new());
        }

        let mut reader = WalReader::open(&wal_path)?;
        reader.read_all()
    }
}
