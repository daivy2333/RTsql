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
/// MS24 Iteration 000 (D3)：一般写入类型门的实际类型名复用同一命名。
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

/// MS24 Iteration 000 (D3): 列声明类型的点名名（`ColumnTypeMismatch` 期望
/// 类型文案用，与 `KeyTypeMismatch` 的 `INT` 大写形态一致）。
fn column_type_name(ct: &ColumnType) -> &'static str {
    match ct {
        ColumnType::Int => "INT",
        ColumnType::String(_) => "STRING",
        ColumnType::Float => "FLOAT",
        ColumnType::Bool => "BOOL",
        ColumnType::Date => "DATE",
        ColumnType::Timestamp => "TIMESTAMP",
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

        // MS23-T02: SET 目标列为 NOT NULL 时拒绝置 NULL——Step 1 之后
        // （KeyNotFound 保持优先）、任何写入之前，原行保持。未知列不在此
        // 拦截，维持 Step 3 既有 ColumnNotFound。
        if matches!(self.new_value, Value::Null) {
            if let Some(idx) = self
                .table_meta
                .columns
                .iter()
                .position(|(name, _)| name == &self.column_name)
            {
                if self.table_meta.not_null[idx] {
                    return Err(StorageError::NullConstraintViolation {
                        column: self.column_name.clone(),
                    });
                }
            }
        }

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
        // mut：MS24 一般类型门对 FLOAT 列 Int 值就地升格改写。
        let mut new_value = match self
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

        // MS23 Iteration 001 (2.6/D7): 唯一列旧值快照——Step 3 改值前捕获，
        // 供碰撞预检与四分支维护使用（无唯一列的表零开销，不快照）。
        let unique_old_values: Vec<Value> = self
            .table_meta
            .unique_indexes
            .iter()
            .map(|(col_idx, _)| values[*col_idx].clone())
            .collect();

        // Step 3: Find column index and modify the target column
        let col_idx = self
            .table_meta
            .columns
            .iter()
            .position(|(name, _)| name == &self.column_name)
            .ok_or_else(|| StorageError::ColumnNotFound(self.column_name.clone()))?;
        values[col_idx] = new_value.clone();

        // MS23 Iteration 001 (2.6/D7): UNIQUE 碰撞预检——改值分支（新值非
        // NULL 且异于旧值）以新键查专属索引，命中即 DuplicateKey。位于任何
        // 写入（Step 6 数据页/WAL/版本链/索引）之前，原行保持零副作用；
        // 旧值读取是本预检的输入，故校验点在 Step 2 之后（仍满足契约
        // 「任何写入前」）。同值分支不预检——条目在本行名下，search 会
        // 误报自碰撞。非 NULL 新值无 B-Tree 键（声明 INT 唯一列的运行时
        // 非 Int 值——2.4 INT 门只约束声明面，Plan Review F1）以
        // KeyTypeMismatch 点名拒绝，先于 Step 4-6 写入（修复前该形态
        // 写入损坏值后维护区 unwrap panic）。
        for (pos, (u_col_idx, uindex)) in self.table_meta.unique_indexes.iter().enumerate() {
            let old_v = &unique_old_values[pos];
            let new_v = &values[*u_col_idx];
            if old_v == new_v || new_v.is_null() {
                continue;
            }
            let Some(new_key) = new_v.to_key() else {
                return Err(StorageError::KeyTypeMismatch {
                    column: self.table_meta.columns[*u_col_idx].0.clone(),
                    expected: "INT".to_string(),
                    actual: key_value_type_name(new_v).to_string(),
                });
            };
            if uindex.search(new_key.as_bytes()).await?.is_some() {
                return Err(StorageError::DuplicateKey);
            }
        }

        // MS24 Iteration 000 (1.3/D3)：SET 赋值列一般类型门——UNIQUE 碰撞
        // 预检之后、serialize 之前（既有 NOT NULL / PK 键位 / F1 守卫 /
        // 碰撞预检门先行，文本与优先级保持）。FLOAT 列整数值无损升格（就地
        // 改写赋值行与 `new_value`，Step 7 键位维护派生自升格后值）；其余
        // 跨类型组合零副作用点名拒绝（原行保持）。
        match (&values[col_idx], &self.schema[col_idx]) {
            (Value::Int(n), ColumnType::Float) => {
                let upgraded = Value::Float(*n as f64);
                values[col_idx] = upgraded.clone();
                new_value = upgraded;
            }
            (Value::Null, _)
            | (Value::Int(_), ColumnType::Int)
            | (Value::String(_), ColumnType::String(_))
            | (Value::Float(_), ColumnType::Float)
            | (Value::Bool(_), ColumnType::Bool)
            | (Value::Date(_), ColumnType::Date)
            | (Value::Timestamp(_), ColumnType::Timestamp) => {}
            (v, ct) => {
                return Err(StorageError::ColumnTypeMismatch {
                    column: self.column_name.clone(),
                    expected: column_type_name(ct).to_string(),
                    actual: key_value_type_name(v).to_string(),
                });
            }
        }

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

        // MS23 Iteration 001 (2.6/D7): 唯一索引四分支维护（数据写入后，与
        // PK 维护同区）——
        // 1/2. SET 非该列或同值：条目随行指向新版本（NULL 无条目自然跳过）；
        // 3.   SET 该列为非 NULL 新值：delete 旧条目 + insert 新条目
        //      （delete 先于 insert——row_to_key 反向映射次序约束与 PK
        //      rekey 同型；碰撞与键控性已在写入前预检拒绝，新值此处必有
        //      键，if-let 为防御形态——Plan Review F1 修复移除 unwrap）；
        // 4.   SET 该列 NULL：删旧条目（NULL 不入索引，镜像 I037 PK 分支）。
        for (pos, (u_col_idx, uindex)) in self.table_meta.unique_indexes.iter().enumerate() {
            let old_v = &unique_old_values[pos];
            let new_v = &values[*u_col_idx];
            let old_key = old_v.to_key();
            let new_key = new_v.to_key();
            if old_v == new_v {
                if let Some(k) = new_key {
                    uindex.update(k.as_bytes(), new_row_id).await?;
                }
            } else if new_v.is_null() {
                if let Some(k) = old_key {
                    uindex.delete(k.as_bytes()).await?;
                }
            } else {
                if let Some(k) = old_key {
                    uindex.delete(k.as_bytes()).await?;
                }
                if let Some(k) = new_key {
                    uindex.insert(k.as_bytes(), new_row_id).await?;
                }
            }
        }

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
