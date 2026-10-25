//! Upsert executor — `INSERT ... ON CONFLICT DO NOTHING/DO UPDATE` 与
//! `REPLACE INTO`（MS24 Iteration 001，design D5）
//!
//! 三个动作共享同一条逐行路径：前置门（coerce → FLOAT 升格 → NOT NULL →
//! PK 键位门，与 `InsertExecutor` 同序同文本）→ 冲突仲裁搜索 → 动作分派。
//! 无冲突行走既有 INSERT 序列、`src/executor/insert.rs` 文件不动（零回归
//! 锚点）；DO UPDATE 镜像 `update.rs` 的写形状；REPLACE 镜像 `delete.rs` 的
//! 删除语义后接插入序列。

use super::datetime::coerce_datetime_write;
use crate::executor::{
    ConflictAction, ConflictArbiter, ExecResult, Executor, UpsertAssignment, UpsertValueExpr, Value,
};
use crate::storage::data::TableManager;
use crate::storage::page_format::{
    compute_tuple_size, deserialize_row, serialize_tuple, ColumnType,
};
use crate::storage::{
    read_tuple_from_data_page, update_version_header_in_data_page, write_tuple_to_data_page,
    BufferPool, PageId, Result, RowId, StorageError, TableMeta,
};
use crate::transaction::{TransactionManager, VersionHeader};
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

/// 键位越界值的类型名（`KeyTypeMismatch` 文案用，镜像 `insert.rs`/`update.rs`）。
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

/// 列声明类型的点名名（`ColumnTypeMismatch` 文案用，镜像 `insert.rs`）。
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

pub struct UpsertExecutor {
    table_meta: Arc<TableMeta>,
    /// 与 `InsertExecutor` 同型：`Some` 时写入经 `TableManager::write_tuple`
    /// 保持 catalog `data_page_tail` 同步。
    table_manager: Option<Arc<TableManager>>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    arbiter: ConflictArbiter,
    action: ConflictAction,
    values: Vec<Vec<Value>>,
    schema: Vec<ColumnType>,
    pk_index: usize,
    tx_id: u64,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl UpsertExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn with_table_manager(
        table_meta: Arc<TableMeta>,
        table_manager: Option<Arc<TableManager>>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        arbiter: ConflictArbiter,
        action: ConflictAction,
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
            arbiter,
            action,
            values,
            schema,
            pk_index,
            tx_id,
            executed: false,
            wal_buffer,
        }
    }

    /// 前置门：日期族强制解析 → FLOAT 列整数值无损升格 → NOT NULL → PK 键位
    /// 类型门。顺序与文本与 `InsertExecutor` 逐项一致（既有错误面优先级由此
    /// 结构性保持）。
    fn prepare_row(&self, row_values: &[Value]) -> Result<Vec<Value>> {
        let mut coerced: Vec<Value> = row_values
            .iter()
            .zip(self.schema.iter())
            .map(|(v, ct)| coerce_datetime_write(v, ct))
            .collect::<Result<Vec<_>>>()?;

        for (v, ct) in coerced.iter_mut().zip(self.schema.iter()) {
            if matches!(ct, ColumnType::Float) {
                if let Value::Int(n) = *v {
                    *v = Value::Float(n as f64);
                }
            }
        }

        for (i, v) in coerced.iter().enumerate() {
            if self.table_meta.not_null[i] && v.is_null() {
                return Err(StorageError::NullConstraintViolation {
                    column: self.table_meta.columns[i].0.clone(),
                });
            }
        }

        if matches!(self.schema[self.pk_index], ColumnType::Int)
            && !matches!(coerced[self.pk_index], Value::Int(_) | Value::Null)
        {
            return Err(StorageError::KeyTypeMismatch {
                column: self.table_meta.pk_column.clone(),
                expected: "INT".to_string(),
                actual: key_value_type_name(&coerced[self.pk_index]).to_string(),
            });
        }

        Ok(coerced)
    }

    /// 冲突仲裁搜索（design D5 步骤 2）：`All` 仲裁 PK（先）与逐唯一索引列
    /// （按序），`Column(idx)` 只仲裁该列。无键值（NULL / 非 Int）不参与仲裁，
    /// 与 INSERT 的无键行语义一致；唯一列的非 NULL 无键值以 `KeyTypeMismatch`
    /// 点名拒绝（MS23 F1 守卫同型）。
    async fn arbitrate(&self, row: &[Value]) -> Result<Vec<RowId>> {
        match &self.arbiter {
            ConflictArbiter::All => {
                let mut hits = Vec::new();
                if let Some(key) = row[self.pk_index].to_key() {
                    if let Some(rid) = self.table_meta.index_manager.search(key.as_bytes()).await? {
                        hits.push(rid);
                    }
                }
                for (col_idx, uindex) in &self.table_meta.unique_indexes {
                    let value = &row[*col_idx];
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
                    if let Some(rid) = uindex.search(key.as_bytes()).await? {
                        hits.push(rid);
                    }
                }
                Ok(hits)
            }
            ConflictArbiter::Column(idx) => {
                let idx = *idx;
                if idx == self.pk_index {
                    return Ok(match row[idx].to_key() {
                        Some(key) => self
                            .table_meta
                            .index_manager
                            .search(key.as_bytes())
                            .await?
                            .into_iter()
                            .collect(),
                        None => Vec::new(),
                    });
                }
                let uindex = self
                    .table_meta
                    .unique_indexes
                    .iter()
                    .find(|(col_idx, _)| *col_idx == idx)
                    .map(|(_, uindex)| uindex.clone())
                    .ok_or_else(|| {
                        StorageError::Internal(format!(
                            "ON CONFLICT target column '{}' carries no UNIQUE index",
                            self.table_meta.columns[idx].0
                        ))
                    })?;
                let value = &row[idx];
                if value.is_null() {
                    return Ok(Vec::new());
                }
                let Some(key) = value.to_key() else {
                    return Err(StorageError::KeyTypeMismatch {
                        column: self.table_meta.columns[idx].0.clone(),
                        expected: "INT".to_string(),
                        actual: key_value_type_name(value).to_string(),
                    });
                };
                Ok(uindex.search(key.as_bytes()).await?.into_iter().collect())
            }
        }
    }

    /// 无冲突路径 / REPLACE 冲突行删除后的插入序列——镜像
    /// `InsertExecutor` 的 PK 重复预检 → UNIQUE 预检 → 类型门 → serialize
    /// → 数据页 → visibility → WAL Insert → record_version → PK 条目 →
    /// 唯一条目。
    ///
    /// PK 预检（MS24 Iteration 001 replan 2.7）在此而非仅在 `arbitrate`：
    /// 显式唯一列目标下 `arbitrate` 不查 PK，无冲突路径若缺该预检会静默
    /// 写入重复主键行并覆盖既有 PK 条目（delta spec R3「仲裁外约束 SHALL
    /// 以既有 DuplicateKey 拒绝」）。REPLACE 恒 `All` 仲裁，冲突行已删、
    /// 条目已清，预检自然放行。无键行（`to_key()` 为 `None`）跳过。
    async fn insert_row(&self, row_values: &[Value]) -> Result<()> {
        if let Some(key) = row_values[self.pk_index].to_key() {
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

        for (col_idx, uindex) in &self.table_meta.unique_indexes {
            let value = &row_values[*col_idx];
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

        for (i, v) in row_values.iter().enumerate() {
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

        let size = compute_tuple_size(row_values, &self.schema);
        let mut buf = vec![0u8; size];
        serialize_tuple(row_values, &self.schema, &mut buf)?;

        let version_header = VersionHeader::new(self.tx_id, None);
        let row_id = if let Some(tm) = &self.table_manager {
            tm.write_tuple(&self.table_meta, &version_header, &buf)
                .await?
        } else {
            write_tuple_to_data_page(&self.buffer_pool, &self.table_meta, &version_header, &buf)
                .await?
        };

        let page_id = PageId(row_id.page_id as u64);
        self.buffer_pool.clear_all_visible(page_id);
        self.buffer_pool
            .update_visibility_on_insert(page_id, self.tx_id);

        if let Some(wal) = &self.wal_buffer {
            wal.append(WalRecord::Insert {
                tx_id: self.tx_id,
                table_name: self.table_meta.name.clone(),
                row_id,
                tuple_data: buf.clone(),
            })
            .await;
        }

        self.tx_manager
            .record_version(self.tx_id, &self.table_meta.name, row_id)
            .await;

        if let Some(key) = row_values[self.pk_index].to_key() {
            self.table_meta
                .index_manager
                .insert(key.as_bytes(), row_id)
                .await?;
        }

        for (col_idx, uindex) in &self.table_meta.unique_indexes {
            if let Some(key) = row_values[*col_idx].to_key() {
                uindex.insert(key.as_bytes(), row_id).await?;
            }
        }

        // MS21 Iter002 (T2-d/D7)：二级索引条目——镜像唯一条目（NULL 跳过）。
        for sindex in &self.table_meta.secondary_indexes {
            if let Some(key) = row_values[sindex.column_index].to_key() {
                sindex.index.insert(key.as_bytes(), row_id).await?;
            }
        }

        Ok(())
    }

    /// DO UPDATE 臂（design D5 步骤 5）——写形状镜像 `UpdateExecutor`：
    /// 读冲突行旧值 → 逐赋值求值（`Old` 取自赋值前的旧行快照）→ 日期族
    /// 强制解析 → 赋值列 NOT NULL → 键位门 + rekey 碰撞预检 → 唯一碰撞
    /// 预检 → 赋值列类型门（含 FLOAT 升格）→ serialize → 新版本
    /// `with_next_version(冲突行)` → visibility → WAL Update → record_version
    /// → PK 四分支 + 唯一四分支维护。
    async fn apply_do_update(
        &self,
        inserted: &[Value],
        assignments: &[UpsertAssignment],
        conflict_rid: RowId,
    ) -> Result<()> {
        let (_version_header, old_tuple_bytes) =
            read_tuple_from_data_page(&self.buffer_pool, conflict_rid, |vh, bytes| {
                Ok((vh, bytes.to_vec()))
            })
            .await?;
        // MS21 Iter003 (T3-a/D9): width-aware read-back + DEFAULT overlay（窄行
        // 读回全宽；写新版本恒全宽，行自愈）。
        //
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 003（design D7 / ISS25）：在途窄化的表被读 ⇒ **明确拒绝**
        // ——DML 的旧行读取不得按位置解码出错位的值。
        self.table_meta.narrowing.ensure_readable()?;
        let mut values =
            deserialize_row(&old_tuple_bytes, &self.schema, &self.table_meta.defaults)?;

        // 旧行快照：`Old` 右值与唯一列旧值均取自赋值前状态（SQLite 同型——
        // 全部右值对原行求值）。
        let old_values = values.clone();
        let unique_old_values: Vec<Value> = self
            .table_meta
            .unique_indexes
            .iter()
            .map(|(col_idx, _)| values[*col_idx].clone())
            .collect();
        // MS21 Iter002 (T2-d/D7)：二级列旧值快照（同区同型）。
        let secondary_old_values: Vec<Value> = self
            .table_meta
            .secondary_indexes
            .iter()
            .map(|s| values[s.column_index].clone())
            .collect();

        let mut touched_columns: Vec<usize> = Vec::with_capacity(assignments.len());
        for assignment in assignments {
            let col_idx = assignment.column;
            let raw = match &assignment.expr {
                UpsertValueExpr::Literal(v) => v.clone(),
                UpsertValueExpr::Excluded(idx) => inserted[*idx].clone(),
                UpsertValueExpr::Old(idx) => old_values[*idx].clone(),
                // MS31 Iteration 001：参数槽位在 pipeline 物化（改写为
                // Literal）后才构造执行器——执行器见到该形态说明物化漏做。
                UpsertValueExpr::Param(index) => {
                    return Err(StorageError::Internal(format!(
                        "unmaterialized parameter slot in DO UPDATE assignment: param {index}"
                    )))
                }
            };
            let coerced = coerce_datetime_write(&raw, &self.schema[col_idx])?;
            values[col_idx] = coerced;
            if !touched_columns.contains(&col_idx) {
                touched_columns.push(col_idx);
            }
        }

        // 赋值列 NOT NULL（仅赋值列可能新增违反；原行既有违反不由 upsert 放大）。
        for col_idx in &touched_columns {
            if self.table_meta.not_null[*col_idx] && values[*col_idx].is_null() {
                return Err(StorageError::NullConstraintViolation {
                    column: self.table_meta.columns[*col_idx].0.clone(),
                });
            }
        }

        // 键位门 + rekey 碰撞预检（赋值触及键列时；旧键派生自冲突行自身）。
        let old_key = old_values[self.pk_index].to_key();
        if touched_columns.contains(&self.pk_index) {
            if matches!(self.schema[self.pk_index], ColumnType::Int)
                && !matches!(values[self.pk_index], Value::Int(_) | Value::Null)
            {
                return Err(StorageError::KeyTypeMismatch {
                    column: self.table_meta.pk_column.clone(),
                    expected: "INT".to_string(),
                    actual: key_value_type_name(&values[self.pk_index]).to_string(),
                });
            }
            if let Some(new_key) = values[self.pk_index].to_key() {
                let unchanged = old_key
                    .as_ref()
                    .is_some_and(|k| k.as_bytes() == new_key.as_bytes());
                if !unchanged
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

        // 唯一列碰撞预检（改值分支；镜像 `update.rs`）。
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

        // 赋值列一般类型门（FLOAT 列整数值无损升格，就地改写；镜像 `update.rs`）。
        for col_idx in &touched_columns {
            match (&values[*col_idx], &self.schema[*col_idx]) {
                (Value::Int(n), ColumnType::Float) => {
                    values[*col_idx] = Value::Float(*n as f64);
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
                        column: self.table_meta.columns[*col_idx].0.clone(),
                        expected: column_type_name(ct).to_string(),
                        actual: key_value_type_name(v).to_string(),
                    });
                }
            }
        }

        let size = compute_tuple_size(&values, &self.schema);
        let mut buf = vec![0u8; size];
        serialize_tuple(&values, &self.schema, &mut buf)?;

        let version_header = VersionHeader::new(self.tx_id, None).with_next_version(conflict_rid);
        let new_row_id =
            write_tuple_to_data_page(&self.buffer_pool, &self.table_meta, &version_header, &buf)
                .await?;

        self.buffer_pool
            .clear_all_visible(PageId(new_row_id.page_id as u64));
        self.buffer_pool
            .clear_all_visible(PageId(conflict_rid.page_id as u64));

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

        self.tx_manager
            .record_version(self.tx_id, &self.table_meta.name, new_row_id)
            .await;

        // PK 索引维护（镜像 `update.rs` Step 7 四分支）。
        if !touched_columns.contains(&self.pk_index) {
            if let Some(old_key) = old_key.as_ref() {
                self.table_meta
                    .index_manager
                    .update(old_key.as_bytes(), new_row_id)
                    .await?;
            }
        } else {
            match values[self.pk_index].to_key() {
                None => {
                    if let Some(old_key) = old_key.as_ref() {
                        self.table_meta
                            .index_manager
                            .delete(old_key.as_bytes())
                            .await?;
                    }
                }
                Some(new_key) => {
                    let unchanged = old_key
                        .as_ref()
                        .is_some_and(|k| k.as_bytes() == new_key.as_bytes());
                    if unchanged {
                        self.table_meta
                            .index_manager
                            .update(new_key.as_bytes(), new_row_id)
                            .await?;
                    } else {
                        if let Some(old_key) = old_key.as_ref() {
                            self.table_meta
                                .index_manager
                                .delete(old_key.as_bytes())
                                .await?;
                        }
                        self.table_meta
                            .index_manager
                            .insert(new_key.as_bytes(), new_row_id)
                            .await?;
                    }
                }
            }
        }

        // 唯一索引四分支维护（镜像 `update.rs`）。
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

        // MS21 Iter002 (T2-d/D7)：二级索引条目维护——镜像 `update.rs` 三支
        // 形态（同值换指 / 置 NULL 仅删 / 改值删旧插新），写入一律
        // `delete_exact` + `insert`（D3 禁用歧义原语）。
        for (pos, sindex) in self.table_meta.secondary_indexes.iter().enumerate() {
            let col = sindex.column_index;
            let old_v = &secondary_old_values[pos];
            let new_v = &values[col];
            let old_key = old_v.to_key();
            let new_key = new_v.to_key();
            if old_v == new_v {
                if let Some(k) = new_key {
                    sindex
                        .index
                        .delete_exact(k.as_bytes(), conflict_rid)
                        .await?;
                    sindex.index.insert(k.as_bytes(), new_row_id).await?;
                }
            } else {
                if let Some(k) = old_key {
                    sindex
                        .index
                        .delete_exact(k.as_bytes(), conflict_rid)
                        .await?;
                }
                if let Some(k) = new_key {
                    sindex.index.insert(k.as_bytes(), new_row_id).await?;
                }
            }
        }

        Ok(())
    }

    /// REPLACE 冲突行删除（design D5 步骤 6）——镜像 `DeleteExecutor`：
    /// 唯一列键值提取 → 墓碑版本（`mark_deleted`，链指针后补）→ visibility →
    /// record_version（墓碑槽位）→ 链指针补写 → PK 条目删除 → 唯一条目删除
    /// → WAL Delete。
    ///
    /// MS21 Iter000 (design D2)：`hit_pk_key` 为仲裁命中的 PK 搜索键——slot
    /// 不可读容忍臂下 PK 条目删除按该键执行（`DeleteExecutor` 同形态，不留
    /// 悬空条目），且悬空 rid 不入 `record_version`（不可读 slot 入
    /// `tx_versions` 会让事务终结路径对它读 slot 报错）。
    async fn delete_conflict_row(&self, rid: RowId, hit_pk_key: Option<Vec<u8>>) -> Result<()> {
        let (old_values, unique_keys, secondary_keys) =
            match read_tuple_from_data_page(&self.buffer_pool, rid, |_, bytes| Ok(bytes.to_vec()))
                .await
            {
                Ok(tuple) => {
                    // MS21 Iter003 (T3-a/D9): width-aware + DEFAULT overlay。
                    //
                    // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                    // Iteration 003（design D7 / ISS25）：在途窄化的表被读 ⇒
                    // **明确拒绝**——DML 的旧行读取不得按位置解码出错位的值。
                    self.table_meta.narrowing.ensure_readable()?;
                    let values = deserialize_row(&tuple, &self.schema, &self.table_meta.defaults)?;
                    let mut keys = Vec::new();
                    for (col_idx, uindex) in &self.table_meta.unique_indexes {
                        if let Some(key) = values[*col_idx].to_key() {
                            keys.push((uindex.clone(), key.as_bytes().to_vec()));
                        }
                    }
                    // MS21 Iter002 (T2-d/D7)：同趟兼提二级列键（调用方以
                    // `delete_exact` 清理）。
                    let mut secondary = Vec::new();
                    for sindex in &self.table_meta.secondary_indexes {
                        if let Some(key) = values[sindex.column_index].to_key() {
                            secondary.push((sindex.index.clone(), key.as_bytes().to_vec()));
                        }
                    }
                    (Some(values), keys, secondary)
                }
                // 与 `DeleteExecutor` 的 SlotNotFound 容忍同型：无元组可读则跳过
                // 唯一条目删除，仍移除索引条目。
                Err(StorageError::SlotNotFound(_)) => (None, Vec::new(), Vec::new()),
                Err(e) => return Err(e),
            };

        let mut tombstone_rid: Option<RowId> = None;
        match self.buffer_pool.read_version_header(rid).await {
            Ok(_) => {
                // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                // Iteration 005（ISS24，DML 同形面）：墓碑**不带链指针**写入
                // ——与 `DeleteExecutor` 及 DDL 改写路径同形。放弃窗口落在
                // 「写出版本槽」与「登记版本」之间时，带链指针的墓碑按删除者的
                // 事务状态压制其前驱版本，而回收看不到未登记的版本 ⇒ 被放弃的
                // REPLACE 隐藏了它的冲突行。不带链指针的墓碑不压制任何前驱版本。
                let tombstone = VersionHeader::new(self.tx_id, None).mark_deleted();
                let written =
                    write_tuple_to_data_page(&self.buffer_pool, &self.table_meta, &tombstone, &[])
                        .await?;
                self.buffer_pool
                    .clear_all_visible(PageId(written.page_id as u64));
                self.buffer_pool
                    .clear_all_visible(PageId(rid.page_id as u64));
                tombstone_rid = Some(written);
            }
            Err(StorageError::SlotNotFound(_)) => {}
            Err(e) => return Err(e),
        }

        // MS21 Iter000 (design D2): 容忍臂（墓碑写入被 SlotNotFound 跳过且
        // 无元组可读）不记录悬空 rid——不可读 slot 入 `tx_versions` 会让
        // `commit_mark_versions` / abort 对它读 slot 报错，事务无法终结。
        // WAL Delete 记录仍携带原 rid（形态不变）。
        //
        // 登记在链指针之前——登记之后回收才看得见这个版本（放弃窗口内的墓碑
        // 必须能被中和）。
        let recorded = tombstone_rid.or_else(|| old_values.is_some().then_some(rid));
        if let Some(recorded) = recorded {
            self.tx_manager
                .record_version(self.tx_id, &self.table_meta.name, recorded)
                .await;
        }

        // 补链指针：墓碑指向被删的冲突行（终态与修复前逐字节相同）。
        if let Some(tombstone) = tombstone_rid {
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
        if let Some(values) = &old_values {
            if let Some(key) = values[self.pk_index].to_key() {
                self.table_meta.index_manager.delete(key.as_bytes()).await?;
            }
        } else if let Some(key) = &hit_pk_key {
            // MS21 Iter000 (design D2)：容忍臂对齐 `DeleteExecutor`——slot
            // 不可读无元组可派生键，仍按仲裁命中的搜索键清理 PK 条目。删除
            // 前按在位守卫确认该键在索引中确实指向本冲突行：`delete` 对缺
            // 失键会误删首个 ≥ 的邻近条目（歧义删除原语），且经唯一索引命
            // 中的冲突行其 PK 键与仲裁键无关。
            if self.table_meta.index_manager.search(key.as_slice()).await? == Some(rid) {
                self.table_meta.index_manager.delete(key.as_slice()).await?;
            }
        }

        for (uindex, key) in &unique_keys {
            uindex.delete(key).await?;
        }

        // MS21 Iter002 (T2-d/D7)：逐二级索引 `delete_exact` 清理冲突行条目
        // （tuple 派生键精确定位；容忍臂无元组可派生，与唯一键同型跳过）。
        for (sindex, key) in &secondary_keys {
            sindex.delete_exact(key, rid).await?;
        }

        if let Some(wal) = &self.wal_buffer {
            wal.append(WalRecord::Delete {
                tx_id: self.tx_id,
                table_name: self.table_meta.name.clone(),
                row_id: rid,
            })
            .await;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Executor for UpsertExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let mut count = 0u64;
        for row_values in &self.values {
            let prepared = self.prepare_row(row_values)?;
            let conflicts = self.arbitrate(&prepared).await?;

            if !conflicts.is_empty() {
                match &self.action {
                    ConflictAction::DoNothing => continue,
                    ConflictAction::DoUpdate(assignments) => {
                        self.apply_do_update(&prepared, assignments, conflicts[0])
                            .await?;
                        count += 1;
                        continue;
                    }
                    ConflictAction::Replace => {
                        // 全部约束上的冲突行去重后逐行删除（同一行可能同时被
                        // PK 与唯一索引命中）。
                        let mut deleted: Vec<RowId> = Vec::new();
                        for rid in conflicts {
                            if !deleted.contains(&rid) {
                                deleted.push(rid);
                            }
                        }
                        // MS21 Iter000 (design D2)：仲裁命中的 PK 键穿入冲突
                        // 删除——slot 不可读容忍臂据此按键清理条目。
                        let hit_pk_key = prepared[self.pk_index]
                            .to_key()
                            .map(|k| k.as_bytes().to_vec());
                        for rid in deleted {
                            self.delete_conflict_row(rid, hit_pk_key.clone()).await?;
                        }
                    }
                }
            }

            self.insert_row(&prepared).await?;
            count += 1;
        }

        Ok(Some(ExecResult::AffectedRows(count)))
    }
}
