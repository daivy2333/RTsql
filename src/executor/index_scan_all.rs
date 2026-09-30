//! Index scan executor for non-unique indexes - returns all matching rows

use crate::executor::apply_projection;
use crate::executor::{ExecResult, Executor};
use crate::profiling::{is_profiling_enabled, record_time};
use crate::storage::btree::IndexManager;
use crate::storage::page_format::{deserialize_row, ColumnType, RowId};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;
use std::time::Instant;

pub struct IndexScanAllExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    key: Vec<u8>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    /// MS10-T01 Iter001: output projection (empty = identity).
    projection: Vec<usize>,
    /// MS21 Iter002 (T2-g/D6): the secondary index tree to search. `None`
    /// (the `new()` default) keeps the PK-tree search — every pre-existing
    /// construction path is byte-for-byte unchanged.
    index: Option<Arc<IndexManager>>,
    row_ids: Vec<RowId>,
    current_idx: usize,
    initialized: bool,
}

impl IndexScanAllExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        key: Vec<u8>,
        snapshot: Option<Snapshot>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        Self {
            table_meta,
            buffer_pool,
            key,
            schema,
            snapshot,
            projection: Vec::new(),
            index: None,
            row_ids: Vec::new(),
            current_idx: 0,
            initialized: false,
        }
    }

    /// MS10-T01 Iter001: narrow produced rows to the given full-schema column
    /// indices (empty = identity, the `new()` default).
    pub fn with_projection(mut self, projection: Vec<usize>) -> Self {
        self.projection = projection;
        self
    }

    /// MS21 Iter002 (T2-g/D6): search a registered secondary index tree
    /// instead of the PK tree. The pipeline resolves the plan node's index
    /// name against `TableMeta.secondary_indexes` before calling this.
    pub fn with_index_manager(mut self, index: Arc<IndexManager>) -> Self {
        self.index = Some(index);
        self
    }
}

#[async_trait::async_trait]
impl Executor for IndexScanAllExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // Lazy initialization: execute search-all on first call
        if !self.initialized {
            let search_tree: &Arc<IndexManager> = self
                .index
                .as_ref()
                .unwrap_or(&self.table_meta.index_manager);
            let profiling = is_profiling_enabled();
            if profiling {
                let t0 = Instant::now();
                self.row_ids = search_tree.search_all(&self.key).await?;
                record_time("index_manager_search_all", t0.elapsed());
            } else {
                self.row_ids = search_tree.search_all(&self.key).await?;
            }
            self.initialized = true;
        }

        // Iterate through all RowIds, returning visible versions
        while self.current_idx < self.row_ids.len() {
            let row_id = self.row_ids[self.current_idx];
            self.current_idx += 1;

            // MVCC visibility check (reuse IndexScanExecutor logic)
            if let Some(ref snapshot) = self.snapshot {
                // M36 closure path; MS21 Iter003 (T3-a/D9): width-aware
                // read-back + DEFAULT overlay so ADD COLUMN narrow rows
                // materialize at full schema width.
                let values_opt = self
                    .buffer_pool
                    .find_visible_version(row_id, snapshot, |bytes| {
                        deserialize_row(bytes, &self.schema, &self.table_meta.defaults)
                    })
                    .await?;

                match values_opt {
                    Some(values) => {
                        return Ok(Some(ExecResult::Row(apply_projection(
                            &self.projection,
                            values,
                        ))))
                    }
                    None => continue, // all versions invisible: skip to next RowId
                }
            } else {
                // M36 closure path; MS21 Iter003 (T3-a/D9): width-aware
                // read-back + DEFAULT overlay.
                let values = read_tuple_from_data_page(&self.buffer_pool, row_id, |_vh, bytes| {
                    deserialize_row(bytes, &self.schema, &self.table_meta.defaults)
                })
                .await?;
                return Ok(Some(ExecResult::Row(apply_projection(
                    &self.projection,
                    values,
                ))));
            }
        }

        Ok(None) // All RowIds processed
    }
}
