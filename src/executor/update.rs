//! Update executor - MVCC-aware row update

use crate::executor::{ExecResult, Executor, UpdateAssignment, UpdateValue, Value};
use crate::storage::page_format::{
    compute_tuple_size, deserialize_row, serialize_tuple, ColumnType,
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
    /// 定位键；`None` ＝ 无 WHERE 全表形态（change 2026-10-04-external-review-gaps
    /// T6——`scan_all` 枚举 + 逐行复用既有单行逻辑）
    key: Option<Vec<u8>>,
    /// 赋值清单（change 2026-10-03-practical-core-gaps T9：多列化——语句序，
    /// 字面量臂与表达式臂按同一旧行快照消费）
    assignments: Vec<UpdateAssignment>,
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
        key: Option<Vec<u8>>,
        assignments: Vec<UpdateAssignment>,
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
            assignments,
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

        // change 2026-10-04-external-review-gaps T6：单行形态（Some）走既有
        // 定位路径；全表形态（None）以 `scan_all()` 枚举**语句开始时点**的
        // 全部索引键（与单行同源的可见性快照），逐行复用既有单行逻辑，行数
        // 汇总为 affected_rows。语句中途失败沿 DML 根事务包裹整体回滚。
        match self.key.take() {
            Some(key) => {
                let affected = self.update_row(&key).await?;
                Ok(Some(ExecResult::AffectedRows(affected)))
            }
            None => {
                let entries = self.table_meta.index_manager.scan_all().await?;
                let mut affected = 0u64;
                for (key, _rid) in entries {
                    affected += self.update_row(&key).await?;
                }
                Ok(Some(ExecResult::AffectedRows(affected)))
            }
        }
    }
}

impl UpdateExecutor {
    /// 既有单行更新路径逐字节承载（T6 提取共享——禁止复制门序）：定位、
    /// 旧行快照求值、per 赋值门序、版本写与唯一/二级索引维护、键位三分支。
    /// 成功恒影响 1 行；定位未命中维持 `KeyNotFound`。
    async fn update_row(&mut self, key: &[u8]) -> Result<u64> {
        // Step 1: Search index for key → get old RowId
        let old_row_id = match self.table_meta.index_manager.search(key).await? {
            Some(id) => id,
            None => return Err(StorageError::KeyNotFound),
        };

        // Step 2 (T9 前移): Read old tuple from data page (M20 closure form,
        // .to_vec() for WAL ownership)。表达式右值对旧行快照求值的输入——
        // 读取前移到求值之前；既有门序的相对顺序（NOT NULL → 键列门 →
        // coerce → 唯一碰撞预检 → 类型门）逐字节保持。
        // MS21 Iter003 (T3-a/D9): width-aware read-back + DEFAULT overlay —
        // ADD COLUMN 前的窄行读回全宽后自愈（写新版本恒全宽）。
        let (_version_header, old_tuple_bytes) =
            read_tuple_from_data_page(&self.buffer_pool, old_row_id, |vh, bytes| {
                Ok((vh, bytes.to_vec()))
            })
            .await?;
        let mut values =
            deserialize_row(&old_tuple_bytes, &self.schema, &self.table_meta.defaults)?;

        // Step 3 (T9): 解析赋值列位并求值全部右值（语句序，同一旧行快照——
        // S14 标准 SQL 语义）。字面量臂直取；表达式臂逐行求值（求值错误
        // 沿 ProjectionExecutor 同款 `ExecutionError` 映射）。
        let mut resolved: Vec<(usize, Value)> = Vec::with_capacity(self.assignments.len());
        for assignment in &self.assignments {
            let col_idx = self
                .table_meta
                .columns
                .iter()
                .position(|(name, _)| name == &assignment.column)
                .ok_or_else(|| StorageError::ColumnNotFound(assignment.column.clone()))?;
            let raw = match &assignment.value {
                UpdateValue::Literal(v) => v.clone(),
                UpdateValue::Expression(e) => e.evaluate(&values).map_err(|err| {
                    StorageError::ExecutionError(format!("Expression evaluation error: {err}"))
                })?,
            };
            resolved.push((col_idx, raw));
        }

        // Step 4 (T9): per-assignment 门序（语句序）——NOT NULL → 键列类型门
        // （含 rekey 碰撞预检）→ 日期族 coerce。既有单列门序与错误面文本
        // 逐字节承载；任何写入前（预检类失败零副作用）。
        for (col_idx, raw) in resolved.iter_mut() {
            let column_name = self.table_meta.columns[*col_idx].0.clone();

            // MS23-T02: SET 目标列为 NOT NULL 时拒绝置 NULL——先于键列门
            // （既有顺序）；表达式臂对求值结果强制。
            if matches!(*raw, Value::Null) && self.table_meta.not_null[*col_idx] {
                return Err(StorageError::NullConstraintViolation {
                    column: column_name,
                });
            }

            // MS16 Iteration 000 (design D3): SET 目标列为键列时，新值类型必须
            // 与键列声明类型同族——Int 键列只接受 Int/NULL。NULL 放行走既有
            // I037 删旧键分支。
            if column_name == self.table_meta.pk_column {
                let pk_declared_int =
                    matches!(self.table_meta.columns[*col_idx].1, ColumnType::Int);
                if pk_declared_int && !matches!(*raw, Value::Int(_) | Value::Null) {
                    return Err(StorageError::KeyTypeMismatch {
                        column: self.table_meta.pk_column.clone(),
                        expected: "INT".to_string(),
                        actual: key_value_type_name(raw).to_string(),
                    });
                }

                // MS16 Iteration 001 (design D4): rekey 碰撞预检——新值可键控且
                // 新键与旧键字节不同时，任何写入之前以新键查索引，命中即拒绝
                // （与 INSERT 先查后写同一模式，拒绝零副作用）。同键字节相等
                // 走既有 update 路径，不做预检。
                if let Some(new_key) = raw.to_key() {
                    if new_key.as_bytes() != key
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

            // MS13 T4（决策 2）：SET 目标列为日期族时强制解析/拒绝（同族值/
            // Null 原样、String 强制解析、其余 InvalidDateTime）。既有顺序：
            // 键列门之后、唯一碰撞预检之前。
            *raw = crate::executor::datetime::coerce_datetime_write(
                raw,
                &self.table_meta.columns[*col_idx].1,
            )?;
        }

        // MS23 Iteration 001 (2.6/D7): 唯一列旧值快照——改值前捕获，供碰撞
        // 预检与四分支维护使用（无唯一列的表零开销，不快照）。
        let unique_old_values: Vec<Value> = self
            .table_meta
            .unique_indexes
            .iter()
            .map(|(col_idx, _)| values[*col_idx].clone())
            .collect();

        // MS21 Iter002 (T2-d/D7)：二级列旧值快照——同区同型（无二级索引的
        // 表零开销，不快照）。
        let secondary_old_values: Vec<Value> = self
            .table_meta
            .secondary_indexes
            .iter()
            .map(|s| values[s.column_index].clone())
            .collect();

        // Step 5 (T9): 赋值落位（语句序；同列重复赋值后写胜出）。
        let mut assigned_cols: Vec<usize> = Vec::with_capacity(resolved.len());
        for (col_idx, raw) in resolved {
            values[col_idx] = raw;
            if !assigned_cols.contains(&col_idx) {
                assigned_cols.push(col_idx);
            }
        }

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
        // 改写赋值行，Step 7 键位维护派生自升格后值）；其余跨类型组合零副
        // 作用点名拒绝（原行保持）。T9：多列按语句序逐列消费同一门序。
        for &col_idx in &assigned_cols {
            match (&values[col_idx], &self.schema[col_idx]) {
                (Value::Int(n), ColumnType::Float) => {
                    let upgraded = Value::Float(*n as f64);
                    values[col_idx] = upgraded;
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
                        column: self.table_meta.columns[col_idx].0.clone(),
                        expected: column_type_name(ct).to_string(),
                        actual: key_value_type_name(v).to_string(),
                    });
                }
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
        // branches; T9: 新键派生自落位后值数组——PK 列未被赋值时条目随行指向
        // 新版本，被赋值时按升格/coerce 后终值走三分支):
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
        let pk_col_idx = self
            .table_meta
            .columns
            .iter()
            .position(|(name, _)| name == &self.table_meta.pk_column);
        let pk_assigned = pk_col_idx.is_some_and(|idx| assigned_cols.contains(&idx));
        if !pk_assigned {
            self.table_meta
                .index_manager
                .update(key, new_row_id)
                .await?;
        } else {
            let new_key = values[pk_col_idx.unwrap()].to_key();
            match new_key {
                None => self.table_meta.index_manager.delete(key).await?,
                Some(k) if k.as_bytes() == key => {
                    self.table_meta
                        .index_manager
                        .update(key, new_row_id)
                        .await?
                }
                Some(k) => {
                    self.table_meta.index_manager.delete(key).await?;
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

        // MS21 Iter002 (T2-d/D7)：二级索引条目维护——UPDATE 为该行写入新
        // 版本（新 rid），条目必须从 (旧键, 旧 rid) 迁移到 (新键, 新 rid)。
        // 写入一律 `delete_exact` + `insert`（D3：二级索引重复键并存，
        // `update`/`delete` 歧义原语禁用）。三支形态：
        // 1. 值不变：条目按同键换指新 rid；
        // 2. 置 NULL：删旧条目（NULL 不入索引）；
        // 3. 改值：删旧键旧条目、插新键新条目（NULL 旧值无条目自然跳过；
        //    INT 声明列经写入类型门新值必为 Int/NULL，to_key 防御形态同唯一列）。
        for (pos, sindex) in self.table_meta.secondary_indexes.iter().enumerate() {
            let col = sindex.column_index;
            let old_v = &secondary_old_values[pos];
            let new_v = &values[col];
            let old_key = old_v.to_key();
            let new_key = new_v.to_key();
            if old_v == new_v {
                if let Some(k) = new_key {
                    sindex.index.delete_exact(k.as_bytes(), old_row_id).await?;
                    sindex.index.insert(k.as_bytes(), new_row_id).await?;
                }
            } else {
                if let Some(k) = old_key {
                    sindex.index.delete_exact(k.as_bytes(), old_row_id).await?;
                }
                if let Some(k) = new_key {
                    sindex.index.insert(k.as_bytes(), new_row_id).await?;
                }
            }
        }

        Ok(1)
    }
}
