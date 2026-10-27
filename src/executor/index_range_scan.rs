//! 二级索引范围扫描。树内条目按哨兵过滤；需要时再补上派生为空且谓词成立的可见行。

use crate::executor::apply_projection;
use crate::executor::predicate::PredicateRef;
use crate::executor::{ExecResult, Executor, Value};
use crate::storage::btree::IndexManager;
use crate::storage::page_format::{deserialize_row, ColumnType, Key, RowId, SlottedPageRef};
use crate::storage::{
    corrupted_data_page_slot_count_error, corrupted_data_page_slot_error,
    read_tuple_from_data_page, BufferPool, PageId, Result, TableMeta,
};
use crate::transaction::{Snapshot, VersionHeader};
use std::collections::HashSet;
use std::sync::Arc;

pub struct IndexRangeScanExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    index: Arc<IndexManager>,
    lower: Option<(Key, bool)>,
    upper: Option<(Key, bool)>,
    omitted_scan: bool,
    predicate: Option<PredicateRef>,
    index_column_indexes: Vec<usize>,
    index_column_types: Vec<ColumnType>,
    generation: u8,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    projection: Vec<usize>,
    rows: Vec<Vec<Value>>,
    cursor: usize,
    initialized: bool,
}

impl IndexRangeScanExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        index: Arc<IndexManager>,
        lower: Option<(Key, bool)>,
        upper: Option<(Key, bool)>,
        omitted_scan: bool,
        predicate: Option<PredicateRef>,
        index_column_indexes: Vec<usize>,
        index_column_types: Vec<ColumnType>,
        generation: u8,
        snapshot: Option<Snapshot>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, column_type)| column_type.clone())
            .collect();
        Self {
            table_meta,
            buffer_pool,
            index,
            lower,
            upper,
            omitted_scan,
            predicate,
            index_column_indexes,
            index_column_types,
            generation,
            schema,
            snapshot,
            projection: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            initialized: false,
        }
    }

    pub fn with_projection(mut self, projection: Vec<usize>) -> Self {
        self.projection = projection;
        self
    }

    async fn visible_row(&self, row_id: RowId) -> Result<Option<Vec<Value>>> {
        if let Some(ref snapshot) = self.snapshot {
            self.buffer_pool
                .find_visible_version(row_id, snapshot, |bytes| {
                    deserialize_row(bytes, &self.schema, &self.table_meta.defaults)
                })
                .await
        } else {
            let values = read_tuple_from_data_page(&self.buffer_pool, row_id, |_header, bytes| {
                deserialize_row(bytes, &self.schema, &self.table_meta.defaults)
            })
            .await?;
            Ok(Some(values))
        }
    }

    fn derived_key(&self, row: &[Value]) -> Option<Key> {
        let mut values = Vec::with_capacity(self.index_column_indexes.len());
        for &index in &self.index_column_indexes {
            values.push(row.get(index)?.clone());
        }
        if values.len() != self.index_column_types.len() {
            return None;
        }
        Value::derive_secondary_index_key(&self.index_column_types, &values, self.generation)
    }

    async fn omitted_rows(&self) -> Result<Vec<Vec<Value>>> {
        let Some(predicate) = &self.predicate else {
            return Ok(Vec::new());
        };
        let mut pointed = HashSet::new();
        let mut tails = Vec::new();
        let mut page_id = Some(self.table_meta.data_page_head);
        while let Some(pid) = page_id {
            let (next_page, slots) = self
                .buffer_pool
                .with_page_data(pid, |data| -> Result<(u32, Vec<(RowId, bool)>)> {
                    let slotted = SlottedPageRef::new(data);
                    if slotted.slot_count_exceeds_capacity() {
                        return Err(corrupted_data_page_slot_count_error(
                            pid.0,
                            slotted.slot_count(),
                        ));
                    }
                    let mut slots = Vec::new();
                    for index in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(index) else {
                            continue;
                        };
                        let slot_data = slotted
                            .get_slot_data_checked(&slot)
                            .ok_or_else(|| corrupted_data_page_slot_error(pid.0, index, &slot))?;
                        if slot_data.len() < VersionHeader::SIZE {
                            continue;
                        }
                        let Some(header) =
                            VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                        else {
                            continue;
                        };
                        if header.create_tx_id() != 0 {
                            if let Some(target) = header.next_version() {
                                pointed.insert(target);
                            }
                        }
                        slots.push((
                            RowId::new(pid.0 as u32, slot.logical_id),
                            header.is_deleted(),
                        ));
                    }
                    Ok((slotted.header().next_page_id, slots))
                })
                .await?;
            tails.extend(slots);
            page_id = if next_page == 0 {
                None
            } else {
                Some(PageId(next_page as u64))
            };
        }

        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        for (row_id, deleted) in tails {
            if !seen.insert(row_id) || pointed.contains(&row_id) {
                continue;
            }
            if self.snapshot.is_none() && deleted {
                continue;
            }
            let Some(values) = self.visible_row(row_id).await? else {
                continue;
            };
            if self.derived_key(&values).is_some() {
                continue;
            }
            match predicate.evaluate(&values) {
                Ok(true) => rows.push(apply_projection(&self.projection, values)),
                Ok(false) => {}
                Err(error) => {
                    return Err(crate::storage::StorageError::ExecutionError(format!(
                        "Predicate evaluation error: {error}"
                    )))
                }
            }
        }
        Ok(rows)
    }
}

#[async_trait::async_trait]
impl Executor for IndexRangeScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        self.table_meta.narrowing.ensure_readable()?;
        if !self.initialized {
            let row_ids = self
                .index
                .scan_range(self.lower.clone(), self.upper.clone())
                .await?;
            for row_id in row_ids {
                if let Some(values) = self.visible_row(row_id).await? {
                    self.rows.push(apply_projection(&self.projection, values));
                }
            }
            if self.omitted_scan {
                self.rows.extend(self.omitted_rows().await?);
            }
            self.initialized = true;
        }
        if self.cursor < self.rows.len() {
            let row = self.rows[self.cursor].clone();
            self.cursor += 1;
            return Ok(Some(ExecResult::Row(row)));
        }
        Ok(None)
    }
}
