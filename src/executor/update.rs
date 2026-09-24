//! Update executor - MVCC-aware row update

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::page_format::{
    compute_tuple_size, deserialize_tuple, serialize_tuple, ColumnType,
};
use crate::storage::{
    read_tuple_from_data_page, write_tuple_to_data_page, BufferPool, PageId, Result, StorageError,
    TableMeta,
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

pub struct UpdateExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    key: Vec<u8>,
    column_name: String,
    new_value: Value,
    tx_id: u64,
    schema: Vec<ColumnType>,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl UpdateExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        key: Vec<u8>,
        column_name: String,
        new_value: Value,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        Self {
            table_meta,
            buffer_pool,
            tx_manager,
            key,
            column_name,
            new_value,
            tx_id,
            schema,
            executed: false,
            wal_buffer,
        }
    }
}

#[async_trait::async_trait]
impl Executor for UpdateExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        // Step 1: Search index for key → get old RowId
        let old_row_id = match self.table_meta.index_manager.search(&self.key).await? {
            Some(id) => id,
            None => return Err(StorageError::KeyNotFound),
        };

        // MS16 Iteration 000 (design D3): SET 目标列为键列时，新值类型必须与
        // 键列声明类型同族——Int 键列只接受 Int/NULL。校验位于 Step 1 之后
        // （目标行不存在仍报 KeyNotFound）且先于任何写入（Step 6 数据页/
        // WAL/版本链）；NULL 放行走既有 I037 删旧键分支。
        if self.column_name == self.table_meta.pk_column {
            let pk_declared_int = self.table_meta.columns.iter().any(|(name, ct)| {
                name == &self.table_meta.pk_column && matches!(ct, ColumnType::Int)
            });
            if pk_declared_int && !matches!(self.new_value, Value::Int(_) | Value::Null) {
                return Err(StorageError::KeyTypeMismatch {
                    column: self.table_meta.pk_column.clone(),
                    expected: "INT".to_string(),
                    actual: key_value_type_name(&self.new_value).to_string(),
                });
            }

            // MS16 Iteration 001 (design D4): rekey 碰撞预检——新值可键控且
            // 新键与旧键字节不同时，任何写入之前以新键查索引，命中即拒绝
            // （与 INSERT 先查后写同一模式，拒绝零副作用）。同键字节相等走
            // 既有 update 路径，不做预检。
            if let Some(new_key) = self.new_value.to_key() {
                if new_key.as_bytes() != self.key.as_slice()
                    && self
                        .table_meta
                        .index_manager
                        .search(new_key.as_bytes())
                        .await?
                        .is_some()
                {
                    return Err(StorageError::DuplicateKey);
                }
            }
        }

        // MS13 T4（决策 2）：SET 目标列为日期族时强制解析/拒绝（同族值/Null
        // 原样、String 强制解析、其余 InvalidDateTime）。先于 Step 2 读与
        // Step 6 写，非法值零副作用；未知列维持 Step 3 既有 ColumnNotFound。
        let new_value = match self
            .table_meta
            .columns
            .iter()
            .find(|(name, _)| name == &self.column_name)
            .map(|(_, ct)| ct.clone())
        {
            Some(ct) => crate::executor::datetime::coerce_datetime_write(&self.new_value, &ct)?,
            None => self.new_value.clone(),
        };

        // Step 2: Read old tuple from data page (M20 closure form, .to_vec() for WAL ownership)
        let (_version_header, old_tuple_bytes) =
            read_tuple_from_data_page(&self.buffer_pool, old_row_id, |vh, bytes| {
                Ok((vh, bytes.to_vec()))
            })
            .await?;
        let mut values = deserialize_tuple(&old_tuple_bytes, &self.schema)?;

        // Step 3: Find column index and modify the target column
        let col_idx = self
            .table_meta
            .columns
            .iter()
            .position(|(name, _)| name == &self.column_name)
            .ok_or_else(|| StorageError::ColumnNotFound(self.column_name.clone()))?;
        values[col_idx] = new_value.clone();

        // Step 4: Serialize new tuple
        let size = compute_tuple_size(&values, &self.schema);
        let mut buf = vec![0u8; size];
        serialize_tuple(&values, &self.schema, &mut buf)?;

        // Step 5: Create new VersionHeader with next_version → old RowId
        let version_header = VersionHeader::new(self.tx_id, None).with_next_version(old_row_id);

        // Step 6: Write new tuple to data page
        let new_row_id =
            write_tuple_to_data_page(&self.buffer_pool, &self.table_meta, &version_header, &buf)
                .await?;

        // M21: Clear page visibility summary after UPDATE (new version page + old version page)
        let new_page_id = PageId(new_row_id.page_id as u64);
        self.buffer_pool.clear_all_visible(new_page_id);
        let old_page_id = PageId(old_row_id.page_id as u64);
        self.buffer_pool.clear_all_visible(old_page_id);

        // WAL: Update record only. BeginTxn/CommitTxn are written by
        // TransactionManager::begin()/commit() (the single source of truth).
        if let Some(wal) = &self.wal_buffer {
            wal.append(WalRecord::Update {
                tx_id: self.tx_id,
                table_name: self.table_meta.name.clone(),
                row_id: new_row_id,
                old_tuple: old_tuple_bytes.clone(),
                new_tuple: buf.clone(),
            })
            .await;
        }

        // Step 6.1: Record version in tx_versions (M10)
        self.tx_manager
            .record_version(self.tx_id, &self.table_meta.name, new_row_id)
            .await;

        // Step 7: Maintain the PK index (MS16 Iteration 001, design D4, three
        // branches):
        // - non-key column: the entry stays under the old key (unchanged);
        // - key column set to a value with no B-Tree key (NULL / non-Int):
        //   delete the old entry so runtime state matches the recovery-side
        //   rebuild (keyless versions are never indexed) — I037, unchanged;
        // - key column set to the same key: update the entry in place
        //   (unchanged);
        // - key column rekeyed to a different keyable value: delete the old
        //   entry first, then insert the new key. delete() resolves the
        //   row_to_key reverse mapping via search, so updating the old entry
        //   first would make that delete clear the new row's mapping.
        let new_key = new_value.to_key();
        if self.column_name != self.table_meta.pk_column {
            self.table_meta
                .index_manager
                .update(&self.key, new_row_id)
                .await?;
        } else {
            match new_key {
                None => self.table_meta.index_manager.delete(&self.key).await?,
                Some(k) if k.as_bytes() == self.key.as_slice() => {
                    self.table_meta
                        .index_manager
                        .update(&self.key, new_row_id)
                        .await?
                }
                Some(k) => {
                    self.table_meta.index_manager.delete(&self.key).await?;
                    self.table_meta
                        .index_manager
                        .insert(k.as_bytes(), new_row_id)
                        .await?;
                }
            }
        }

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
