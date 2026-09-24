//! Insert executor - MVCC-aware row insert

use super::datetime::coerce_datetime_write;
use crate::executor::{ExecResult, Executor, Value};
use crate::storage::data::TableManager;
use crate::storage::page_format::{compute_tuple_size, serialize_tuple, ColumnType};
use crate::storage::{
    write_tuple_to_data_page, BufferPool, PageId, Result, StorageError, TableMeta,
};
use crate::transaction::{TransactionManager, VersionHeader};
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

/// MS16 Iteration 000 (design D3): 键位越界值的类型名（`KeyTypeMismatch`
/// 错误文案用；调用点已保证值非 Int/Null，Int/Null 臂仅为穷尽性）。
fn key_value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::String(_) => "String",
        Value::Null => "Null",
        Value::Float(_) => "Float",
        Value::Bool(_) => "Bool",
        Value::Date(_) => "Date",
        Value::Timestamp(_) => "Timestamp",
    }
}

pub struct InsertExecutor {
    table_meta: Arc<TableMeta>,
    /// MS07-T01: When set, the executor routes writes through
    /// `TableManager::write_tuple` so the catalog's `data_page_tail`
    /// is kept in sync on cross-page allocation. When `None` (legacy
    /// test paths), falls back to the standalone `write_tuple_to_data_page`
    /// function which only updates the in-memory `TableMeta`.
    table_manager: Option<Arc<TableManager>>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    values: Vec<Vec<Value>>,
    schema: Vec<ColumnType>,
    pk_index: usize,
    tx_id: u64,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl InsertExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        values: Vec<Vec<Value>>,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        Self::with_table_manager(
            table_meta,
            None,
            buffer_pool,
            tx_manager,
            values,
            tx_id,
            wal_buffer,
        )
    }

    /// MS07-T01: constructor variant that wires the executor to a
    /// `TableManager` so writes can keep the catalog in sync.
    pub fn with_table_manager(
        table_meta: Arc<TableMeta>,
        table_manager: Option<Arc<TableManager>>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        values: Vec<Vec<Value>>,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        let pk_index = table_meta.pk_index;
        Self {
            table_meta,
            table_manager,
            buffer_pool,
            tx_manager,
            values,
            schema,
            pk_index,
            tx_id,
            executed: false,
            wal_buffer,
        }
    }
}

#[async_trait::async_trait]
impl Executor for InsertExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let mut count = 0u64;
        for row_values in &self.values {
            // MS13 T4（决策 2）：日期族目标列写入强制解析——逐列先于 MS16
            // 键位预检与任何索引访问，非法值零副作用拒绝（同族值/Null 原样，
            // String 强制解析，其余 InvalidDateTime）。
            let coerced: Vec<Value> = row_values
                .iter()
                .zip(self.schema.iter())
                .map(|(v, ct)| coerce_datetime_write(v, ct))
                .collect::<Result<Vec<_>>>()?;
            let row_values = &coerced;
            let pk_value = &row_values[self.pk_index];

            // MS16 Iteration 000 (design D3): Int 键列只接受 Int 或 NULL 键位
            // 值——越界类型在此拒绝（先于 DuplicateKey 预检，非法类型无需
            // 访问索引），否则按值打 tag 落库为无键行、键位等值点查静默漏行
            // （I046 同族）。非 Int 键列不新增拒绝；NULL 保持无键行语义。
            if matches!(self.schema[self.pk_index], ColumnType::Int)
                && !matches!(pk_value, Value::Int(_) | Value::Null)
            {
                return Err(StorageError::KeyTypeMismatch {
                    column: self.table_meta.pk_column.clone(),
                    expected: "INT".to_string(),
                    actual: key_value_type_name(pk_value).to_string(),
                });
            }

            // MS10-T05 001-rework (T8-R1): rows whose key-position value has
            // no B-Tree key (NULL / non-Int) are stored but not indexed —
            // no duplicate check and no index entry (documented semantics,
            // SQLite NULL-PK precedent). Keyed rows keep the exact path below.
            let key = pk_value.to_key();

            if let Some(key) = key.as_ref() {
                if self
                    .table_meta
                    .index_manager
                    .search(key.as_bytes())
                    .await?
                    .is_some()
                {
                    return Err(StorageError::DuplicateKey);
                }
            }

            let size = compute_tuple_size(row_values, &self.schema);
            let mut buf = vec![0u8; size];
            serialize_tuple(row_values, &self.schema, &mut buf)?;

            let version_header = VersionHeader::new(self.tx_id, None);

            // MS07-T01: write via TableManager when available so the
            // catalog's `data_page_tail` is updated on cross-page
            // allocation. Falls back to the standalone function for
            // legacy test paths that don't construct a TableManager.
            let row_id = if let Some(tm) = &self.table_manager {
                tm.write_tuple(&self.table_meta, &version_header, &buf)
                    .await?
            } else {
                write_tuple_to_data_page(&self.buffer_pool, &self.table_meta, &version_header, &buf)
                    .await?
            };

            // M21: Update page visibility summary after INSERT
            let page_id = PageId(row_id.page_id as u64);
            self.buffer_pool.clear_all_visible(page_id);
            self.buffer_pool
                .update_visibility_on_insert(page_id, self.tx_id);

            // WAL: Insert record. BeginTxn/CommitTxn are written by
            // TransactionManager::begin()/commit() (the single source of truth).
            if let Some(wal) = &self.wal_buffer {
                wal.append(WalRecord::Insert {
                    tx_id: self.tx_id,
                    table_name: self.table_meta.name.clone(),
                    row_id,
                    tuple_data: buf.clone(),
                })
                .await;
            }

            // Record version in tx_versions (M10)
            self.tx_manager
                .record_version(self.tx_id, &self.table_meta.name, row_id)
                .await;

            // T8-R1: keyless rows skip the index insert (stored, not indexed)
            if let Some(key) = key.as_ref() {
                self.table_meta
                    .index_manager
                    .insert(key.as_bytes(), row_id)
                    .await?;
            }

            count += 1;
        }

        Ok(Some(ExecResult::AffectedRows(count)))
    }
}
