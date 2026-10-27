//! CreateIndexExecutor — 执行 `CREATE INDEX`（MS21 Iter002 T2-c，design D5）
//!
//! 执行序（CTAS「建表→物化→登记」先例同型，失败零副作用可重试）：
//! 1. 建空树（`IndexManager::new`，无 catalog 上下文——登记前根变更不落
//!    catalog，登记后由 `register_secondary_index` 统一挂接）；
//! 2. 全量回填：遍历数据页链收集链尾（不被任何 `next_version` 指向）slot，
//!    对**非墓碑**链尾无条件建条目（含活跃未提交版本——其回滚由 T2-d 扩展
//!    后的 abort_cleanup 二级清理兜底，两向一致）；索引列经
//!    `secondary_index_key` 派生，NULL 与不可容纳的值不入树；
//! 3. 一次性 catalog 登记 + 内存换入（`TableManager::register_secondary_index`）。
//!
//! 回填失败：旧树页 best-effort 释放后传播错误——登记未发生，零副作用。

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::btree::IndexManager;
use crate::storage::page_format::{deserialize_row, overflow, ColumnType, SlottedPageRef};
use crate::storage::{
    corrupted_data_page_slot_count_error, corrupted_data_page_slot_error, BufferPool, PageId,
    Result, RowId,
};
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

/// 数据页链扫描的单页产出：(rid, 链尾标志, tuple 段引用)。
/// MS27 T5 两相装配：页闭包内只做 header 解析与 locator 判定，索引列键在
/// 闭包外装配 tuple 后派生（locator slot 携带 9B 引用而非 tuple 字节）。
type BackfillPage = (u32, Vec<(RowId, bool, overflow::TupleRef)>);

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
            let mut released: Vec<PageId> = Vec::new();
            for page in new_index.collect_all_pages_tolerant().await {
                if self.database.buffer_pool.free_page(page).await.is_ok() {
                    released.push(page);
                }
            }
            // MS35（`free-page-lifecycle` R3）：释放完成标记——该索引从未登记
            // 进目录（无锚点写），按保守方向等下一次成功刷盘。
            self.database
                .buffer_pool
                .complete_free_release(&released, &[]);
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
    let mut page_slots: Vec<(RowId, bool, overflow::TupleRef)> = Vec::new();
    let mut page_id = Some(table_meta.data_page_head);
    while let Some(pid) = page_id {
        let (next_page, slots) = buffer_pool
            .with_page_data(pid, |data| -> Result<BackfillPage> {
                let slotted = SlottedPageRef::new(data);
                // MS37 T17: 页头槽数越界 ⇒ 明确错误（取槽**之前**判；否则
                // 槽目录定位下溢 ⇒ 越界切片 panic ⇒ 进程 abort）。
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
                    // MS37 T3: 越界槽 ⇒ 明确错误（不得跳过——跳过 = 静默错读）。
                    let slot_data = slotted
                        .get_slot_data_checked(&slot)
                        .ok_or_else(|| corrupted_data_page_slot_error(pid.0, index, &slot))?;
                    if slot_data.len() < VersionHeader::SIZE {
                        continue;
                    }
                    let Some(vh) = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                    else {
                        continue;
                    };
                    // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                    // Iteration 001（MTV R1 / ISS24 的 M2）：抑制集合只承认
                    // 「指向者未被回滚」的链接——已回滚（`create_tx_id == 0`）
                    // 的指向者不抑制其前驱版本，与
                    // `DataScanExecutor::superseder_suppresses` 同源。
                    if vh.create_tx_id() != 0 {
                        if let Some(target) = vh.next_version() {
                            pointed.insert(target);
                        }
                    }
                    let tuple_ref =
                        overflow::TupleRef::from_slot_tuple(&slot_data[VersionHeader::SIZE..]);
                    slots.push((
                        RowId::new(pid.0 as u32, slot.logical_id),
                        !vh.is_deleted(),
                        tuple_ref,
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
    for (rid, is_live, tuple_ref) in page_slots {
        if seen.insert(rid, ()).is_some() {
            continue; // 防御：同页链重复 slot id
        }
        // 链尾且非墓碑 → 无条件建条目（含活跃未提交版本）
        if !pointed.contains(&rid) && is_live {
            // MS21 Iter003 (T3-a/D9): width-aware + DEFAULT overlay —— ADD
            // COLUMN 后对尾列建索引时，窄行按声明 DEFAULT 入索引（与扫描读
            // 回同源）。MS27 T5：超页行出页闭包装配后派生（键与运行期写路径
            // 一致）。
            //
            // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
            // Iteration 003（design D7 / ISS25）：在途窄化的表被读 ⇒ **明确
            // 拒绝**——索引回填的旧行读取不得按位置解码出错位的值。
            table_meta.narrowing.ensure_readable()?;
            let tuple_bytes = tuple_ref.resolve(buffer_pool).await?;
            let values = deserialize_row(&tuple_bytes, &schema, &table_meta.defaults)?;
            if let Some(key) = values.get(column_index).and_then(|v| {
                schema
                    .get(column_index)
                    .and_then(|column_type| v.secondary_index_key(column_type))
            }) {
                tree.insert(key.as_bytes(), rid).await?;
            }
        }
    }

    Ok(())
}
