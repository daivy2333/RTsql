//! CreateIndexExecutor — 执行 `CREATE INDEX`（MS21 Iter002 T2-c，design D5）
//!
//! 执行序（CTAS「建表→物化→登记」先例同型，失败零副作用可重试）：
//! 1. 建空树（`IndexManager::new`，无 catalog 上下文——登记前根变更不落
//!    catalog，登记后由 `register_secondary_index` 统一挂接）；
//! 2. 全量回填：遍历数据页链收集链尾（不被任何 `next_version` 指向）slot，
//!    对**非墓碑**链尾无条件建条目（含活跃未提交版本——其回滚由 T2-d 扩展
//!    后的 abort_cleanup 二级清理兜底，两向一致）；索引列 NULL 跳过
//!    （`to_key` None，同 PK/唯一语义；写入类型门保证 INT 列无非 Int 值）；
//! 3. 一次性 catalog 登记 + 内存换入（`TableManager::register_secondary_index`）。
//!
//! 回填失败：旧树页 best-effort 释放后传播错误——登记未发生，零副作用。

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::btree::IndexManager;
use crate::storage::page_format::{deserialize_row, ColumnType, SlottedPageRef};
use crate::storage::{BufferPool, Result, RowId};
use crate::transaction::VersionHeader;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// CREATE INDEX 执行器
pub struct CreateIndexExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl CreateIndexExecutor {
    /// 创建新的 CreateIndexExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

/// 数据页链扫描的单页产出：(rid, 链尾标志, 索引列键)。
type BackfillPage = (u32, Vec<(RowId, bool, Option<Vec<u8>>)>);

#[async_trait::async_trait]
impl Executor for CreateIndexExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let node = match &self.plan {
            PhysicalPlan::CreateIndex(node) => node,
            _ => panic!("CreateIndexExecutor requires CreateIndexNode"),
        };

        let table_meta = self
            .database
            .table_manager
            .get_table(&node.table_name)
            .await?;

        // 1. 建空树（spawn_blocking：IndexManager::new 内部 block_on）
        let bp = self.database.buffer_pool.clone();
        let new_index =
            Arc::new(tokio::task::spawn_blocking(move || IndexManager::new(bp)).await??);

        // 2. 回填（rebuild 同型链尾扫描；schema 反序列化取索引列值）
        let backfill = backfill_tree(
            &self.database.buffer_pool,
            &table_meta,
            node.column_index,
            &new_index,
        )
        .await;

        if let Err(e) = backfill {
            // 失败零副作用：树页 best-effort 释放（先例 drop_table），登记未发生
            for page in new_index.collect_all_pages_tolerant().await {
                let _ = self.database.buffer_pool.free_page(page).await;
            }
            return Err(e);
        }

        // 3. 登记（catalog 段追加 + TableMeta 换入 + catalog 上下文挂接）
        self.database
            .table_manager
            .register_secondary_index(
                &node.table_name,
                &node.index_name,
                node.column_index,
                new_index,
            )
            .await?;

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}

/// 遍历表数据页链，对每个「非墓碑链尾」版本按索引列值建条目。
/// 链尾 = 不被任何 `next_version` 指向的 slot（该行版本的最新版）。
async fn backfill_tree(
    buffer_pool: &Arc<BufferPool>,
    table_meta: &Arc<crate::storage::TableMeta>,
    column_index: usize,
    tree: &Arc<IndexManager>,
) -> Result<()> {
    let schema: Vec<ColumnType> = table_meta
        .columns
        .iter()
        .map(|(_, ct)| ct.clone())
        .collect();

    let mut pointed: HashSet<RowId> = HashSet::new();
    let mut page_slots: Vec<(RowId, bool, Option<Vec<u8>>)> = Vec::new();
    let mut page_id = Some(table_meta.data_page_head);
    while let Some(pid) = page_id {
        let (next_page, slots) = buffer_pool
            .with_page_data(pid, |data| -> Result<BackfillPage> {
                let slotted = SlottedPageRef::new(data);
                let mut slots = Vec::new();
                for index in 0..slotted.slot_count() {
                    let Some(slot) = slotted.get_slot(index) else {
                        continue;
                    };
                    let slot_data = slotted.get_slot_data(&slot);
                    if slot_data.len() < VersionHeader::SIZE {
                        continue;
                    }
                    let Some(vh) = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                    else {
                        continue;
                    };
                    if let Some(target) = vh.next_version() {
                        pointed.insert(target);
                    }
                    // 索引列键在此单趟页读内派生（tuple 字节随 header 同页）。
                    let key = if vh.is_deleted() {
                        None
                    } else {
                        // MS21 Iter003 (T3-a/D9): width-aware + DEFAULT
                        // overlay —— ADD COLUMN 后对尾列建索引时，窄行按
                        // 声明 DEFAULT 入索引（与扫描读回同源）。
                        let values = deserialize_row(
                            &slot_data[VersionHeader::SIZE..],
                            &schema,
                            &table_meta.defaults,
                        )?;
                        values
                            .get(column_index)
                            .and_then(|v| v.to_key())
                            .map(|k| k.as_bytes().to_vec())
                    };
                    slots.push((
                        RowId::new(pid.0 as u32, slot.logical_id),
                        !vh.is_deleted(),
                        key,
                    ));
                }
                Ok((slotted.header().next_page_id, slots))
            })
            .await?;
        page_slots.extend(slots);
        page_id = if next_page == 0 {
            None
        } else {
            Some(crate::storage::PageId(next_page as u64))
        };
    }

    let mut seen: HashMap<RowId, ()> = HashMap::new();
    for (rid, is_live, key) in page_slots {
        if seen.insert(rid, ()).is_some() {
            continue; // 防御：同页链重复 slot id
        }
        // 链尾且非墓碑 → 无条件建条目（含活跃未提交版本）
        if !pointed.contains(&rid) && is_live {
            if let Some(key) = key {
                tree.insert(&key, rid).await?;
            }
        }
    }

    Ok(())
}
