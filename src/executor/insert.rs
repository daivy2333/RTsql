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
            let mut coerced: Vec<Value> = row_values
                .iter()
                .zip(self.schema.iter())
                .map(|(v, ct)| coerce_datetime_write(v, ct))
                .collect::<Result<Vec<_>>>()?;

            // MS24 Iteration 000 (1.2/D3)：FLOAT 列整数值无损升格——静默
            // 就地改写（无错误面），先于键位派生，保证 Float 键列收 Int 值
            // 时键位与索引语义派生自升格后值（Float 无 B-Tree 键 → 无键行，
            // 与恢复期自 tuple 重建的键位两态一致）。键位门/唯一门覆盖的
            // INT 声明列不受升格影响，既有错误面输入不变。
            for (v, ct) in coerced.iter_mut().zip(self.schema.iter()) {
                if matches!(ct, ColumnType::Float) {
                    if let Value::Int(n) = *v {
                        *v = Value::Float(n as f64);
                    }
                }
            }

            // MS23-T02: NOT NULL 强制——coerce 之后、键位预检/索引访问/任何
            // 写入之前逐列校验，违反零副作用拒绝。
            for (i, v) in coerced.iter().enumerate() {
                if self.table_meta.not_null[i] && v.is_null() {
                    return Err(StorageError::NullConstraintViolation {
                        column: self.table_meta.columns[i].0.clone(),
                    });
                }
            }

            let pk_value = &coerced[self.pk_index];

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

            // MS23 Iteration 001 (2.5/D7)：UNIQUE 预检——PK 预检之后、
            // serialize 之前，逐唯一列以 Int 键查专属索引，命中即
            // DuplicateKey（零副作用拒绝，未触任何写入）。NULL 豁免（不入
            // 唯一索引、互不冲突）。非 NULL 且 to_key 为 None 的值（2.4
            // INT 门只约束列声明类型、不约束值运行时类型——Plan Review F1）
            // 以 KeyTypeMismatch 点名拒绝，替代原静默跳过。
            for (col_idx, uindex) in &self.table_meta.unique_indexes {
                let value = &coerced[*col_idx];
                if value.is_null() {
                    continue;
                }
                let Some(key) = value.to_key() else {
                    return Err(StorageError::KeyTypeMismatch {
                        column: self.table_meta.columns[*col_idx].0.clone(),
                        expected: "INT".to_string(),
                        actual: key_value_type_name(value).to_string(),
                    });
                };
                if uindex.search(key.as_bytes()).await?.is_some() {
                    return Err(StorageError::DuplicateKey);
                }
            }

            // MS24 Iteration 000 (1.2/D3)：一般写入类型门（拒绝趟）——既有
            // NOT NULL / PK 键位 / UNIQUE F1 / DuplicateKey 门之后、serialize
            // 之前逐列校验值变体与列声明类型一致：NULL 豁免（NOT NULL 门已
            // 裁决 NULL 性）、日期族经 coerce 已同族、FLOAT 列 Int 值已在升
            // 格趟改写；其余跨类型组合零副作用点名拒绝（未触任何写入）。
            // 既有门的触发优先级与文本由此结构性保持（其在本门之前）。
            for (i, v) in coerced.iter().enumerate() {
                let ok = matches!(
                    (v, &self.schema[i]),
                    (Value::Null, _)
                        | (Value::Int(_), ColumnType::Int)
                        | (Value::String(_), ColumnType::String(_))
                        | (Value::Float(_), ColumnType::Float)
                        | (Value::Bool(_), ColumnType::Bool)
                        | (Value::Date(_), ColumnType::Date)
                        | (Value::Timestamp(_), ColumnType::Timestamp)
                );
                if !ok {
                    return Err(StorageError::ColumnTypeMismatch {
                        column: self.table_meta.columns[i].0.clone(),
                        expected: column_type_name(&self.schema[i]).to_string(),
                        actual: key_value_type_name(v).to_string(),
                    });
                }
            }

            let row_values = &coerced;
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

            // MS23 Iteration 001 (2.5/D7): UNIQUE 条目在数据落位与 PK 索引
            // 插入之后写入（镜像 PK 顺序——数据写失败不留索引条目）。NULL
            // 不入索引（to_key None 自然跳过）。
            for (col_idx, uindex) in &self.table_meta.unique_indexes {
                if let Some(key) = row_values[*col_idx].to_key() {
                    uindex.insert(key.as_bytes(), row_id).await?;
                }
            }

            count += 1;
        }

        Ok(Some(ExecResult::AffectedRows(count)))
    }
}
