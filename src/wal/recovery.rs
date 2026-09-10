//! 恢复管理器
//!
//! 负责启动时重放 WAL，恢复未完成事务

use super::{WalError, WalReader, WalRecord};
use crate::storage::data::TableMeta;
use crate::storage::page_format::{deserialize_tuple, RowId, SlottedPage, SlottedPageRef};
use crate::storage::{
    update_version_header_in_data_page, BufferPool, IndexManager, PageId, TableManager,
};
use crate::transaction::VersionHeader;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// 恢复结果
#[derive(Debug, Default)]
pub struct RecoveryResult {
    pub committed_tx_ids: HashSet<u64>,
    pub aborted_tx_ids: HashSet<u64>,
    pub uncommitted_tx_ids: HashSet<u64>,
    pub redo_count: usize,
}

/// R-T0b-R7 (D10) + T8-R2：重放期的磁盘版本多映射，按表分桶。
///
/// - `keyed`：table → PK key → 升序 row ids（键位可键控的版本，键 =
///   `Value::to_key` 字节）。
/// - `keyless`：table → tuple 原始字节 → 升序 row ids（键位不可键控的
///   版本——T8-R1 起落库不入索引的行；桶键即 tuple 字节，Update 推导按
///   old_tuple 字节取候选集）。
///
/// Update 臂 `old_row_id` 推导读取对应桶；判重与索引维护由重放后的重建
/// 统一负责（R-T0b-R8）。
#[derive(Default)]
struct PkVersionMaps {
    keyed: HashMap<String, HashMap<Vec<u8>, Vec<RowId>>>,
    keyless: HashMap<String, HashMap<Vec<u8>, Vec<RowId>>>,
}

/// 恢复上下文：跨 `redo_record` 调用追踪每张表的链/tail 状态。
///
/// T0b 设计：原 `redo_record` 三臂都走 `write_tuple_to_data_page` 追加到
/// 当前 tail，丢失了 WAL 记录的 (page_id, slot_id)，导致恢复后 B-Tree 指
/// 针与数据页分叉。位置寻址重放修复后，需要在跨调用的状态中追踪每张表
/// 已写入的「上一页」——当当前重放记录的目标页与之不同（页间切换，即
/// 运行期 `write_tuple_to_data_page` 的溢出新页序）时，把 prev.next 置
/// 为 current 重建链，并把内存 `data_page_tail` 跟着挪到最后一页。
struct RedoContext {
    /// table_name → 该表在本次重放中最近一次写入的目标页。
    /// 重放开始时为空；每条 Insert/Update 完成后更新。
    table_last_page: HashMap<String, PageId>,
    /// R-T0b-R7 (D10)：`Some` 表示本次打开有已提交 data 记录重放
    /// （redo_count > 0），三臂全部 B-Tree 自由，Update 经多映射派生
    /// `old_row_id`，PK 索引由重放后的重建统一负责（R-T0b-R8）；
    /// `None` 保持既有 redo_count == 0 路径（消费磁盘树）逐字节不变。
    pk_versions: Option<PkVersionMaps>,
}

impl RedoContext {
    fn new() -> Self {
        Self {
            table_last_page: HashMap::new(),
            pk_versions: None,
        }
    }

    fn is_deindexed(&self) -> bool {
        self.pk_versions.is_some()
    }
}

/// 向升序候选集中插入一个 rid（按 (page_id, slot_id) 排序）。
fn insert_row_id_sorted(candidates: &mut Vec<RowId>, rid: RowId) {
    let pos = candidates.partition_point(|r| (r.page_id, r.slot_id) < (rid.page_id, rid.slot_id));
    candidates.insert(pos, rid);
}

/// 页扫描闭包产出：`data_page.rs` 页遍历共用形态（build_pk_version_maps）。
/// slot 键：可键控 slot 携带 PK 键；无键 slot 携带 tuple 原始字节（T8-R2
/// keyless 桶的桶键）。
type PreScanPage = crate::storage::Result<(u32, Vec<(PrescanSlotKey, RowId)>)>;
/// 页扫描闭包产出（rebuild_pk_indexes）：(rid, header, PK key)。
type RebuildScanPage = crate::storage::Result<(u32, Vec<(RowId, VersionHeader, Option<Vec<u8>>)>)>;

/// 预扫描 slot 键（T8-R2）：`Keyed` = PK 键字节；`Keyless` = tuple 原始字节。
enum PrescanSlotKey {
    Keyed(Vec<u8>),
    Keyless(Vec<u8>),
}

/// 位置寻址写入 helper：把 (vh + tuple_bytes) 落到 WAL 记录的 (page, slot)。
///
/// 行为：
///   1. `get_page(row_id.page_id)`：若文件长度未到（首次恢复时）则隐式延展
///      ——由后续 `get_page` 的实现保证；不再显式 `allocate_page`。
///   2. 页类型字节 0 则 init(0x03) 数据页。
///   3. `get_slot_by_logical_id(row_id.slot_id)` 若 Some → 已存在，
///      幂等跳据（crash-during-recovery re-entry）。
///   4. `add_slot` 并校验落位 logical_id == row_id.slot_id（稠密性），
///      不等 → `WalError::RedoFailed`（K05：序列不一致时显式失败）。
///   5. 页链更新：若 ctx 中上次目标页 != 当前目标页，置 prev.next = current。
///   6. 内存 `data_page_tail` 跟随到当前目标页。
///   7. M21 镜像：`clear_all_visible` + `update_visibility_on_insert`。
async fn redo_tuple_at_row_id(
    buffer_pool: &Arc<BufferPool>,
    table_meta: &Arc<TableMeta>,
    row_id: RowId,
    version_header: &VersionHeader,
    tuple_bytes: &[u8],
    ctx: &mut RedoContext,
) -> Result<(), WalError> {
    let page_id = PageId(row_id.page_id as u64);
    let table_name = table_meta.name.clone();

    // 1. 拼装 slot_data
    let mut slot_data = version_header.to_bytes();
    slot_data.extend_from_slice(tuple_bytes);

    // 2. 获取页 + 写入
    let guard = buffer_pool.get_page(page_id).await.map_err(|e| {
        WalError::RedoFailed(format!("get_page for row_id {:?} failed: {}", row_id, e))
    })?;

    let write_result: std::result::Result<(), String> = guard.modify_page(|page| {
        // 2a. 未初始化则 init 为数据页
        if page.data[0] == 0 {
            let _ = SlottedPage::init(page, 0x03);
        }

        let mut slotted = SlottedPage::new(page);

        // 2b. 幂等：slot 已存在则直接成功返回
        if slotted.get_slot_by_logical_id(row_id.slot_id).is_some() {
            return Ok(());
        }

        // 2c. add_slot
        let (new_logical_id, _slot_index) = slotted
            .add_slot(&slot_data)
            .map_err(|e| format!("add_slot failed: {}", e))?;

        // 2d. 稠密落位校验：写入的 logical_id 必须等于 WAL 记录的 slot_id
        if new_logical_id != row_id.slot_id {
            return Err(format!(
                "dense-assignment violation: WAL slot_id={} but page assigned logical_id={}",
                row_id.slot_id, new_logical_id
            ));
        }
        Ok(())
    });

    write_result.map_err(|e| {
        WalError::RedoFailed(format!(
            "redo_tuple_at_row_id for row_id {:?} failed: {}",
            row_id, e
        ))
    })?;

    // 3. 链更新：跨页时把 prev.next 置为 current
    if let Some(&prev) = ctx.table_last_page.get(&table_name) {
        if prev != page_id {
            let prev_guard = buffer_pool.get_page(prev).await.map_err(|e| {
                WalError::RedoFailed(format!(
                    "chain-link get_page(prev={:?}) failed: {}",
                    prev, e
                ))
            })?;
            prev_guard.modify_page(|page| {
                let next_u32 = page_id.0 as u32;
                page.data[5..9].copy_from_slice(&next_u32.to_le_bytes());
            });
        }
    }
    ctx.table_last_page.insert(table_name.clone(), page_id);

    // 4. 内存 data_page_tail 跟随
    *table_meta.data_page_tail.lock().unwrap() = page_id;

    // 5. M21 镜像：clear + insert hint
    buffer_pool.clear_all_visible(page_id);
    buffer_pool.update_visibility_on_insert(page_id, version_header.create_tx_id());

    Ok(())
}

/// 从已序列化的 tuple bytes 提取 PK 的 B-Tree 键。
///
/// 复用 executor 的 `deserialize_tuple`，按 `TableMeta.pk_index` 选列，
/// 调用 `Value::to_key()` 得到 `Option<Vec<u8>>`。None 表示该 PK 类型
/// 不支持索引（String/Float/Bool/Null）——T8-R2 起无键版本按 tuple 原始
/// 字节入 keyless 桶追踪（重放路径）；仅 `redo_count == 0` 的既有路径
/// （对重放记录不可达）保留 RedoFailed 语义。
fn extract_pk_key(table_meta: &Arc<TableMeta>, tuple_data: &[u8]) -> Option<Vec<u8>> {
    let schema: Vec<_> = table_meta
        .columns
        .iter()
        .map(|(_, ct)| ct.clone())
        .collect();
    let values = deserialize_tuple(tuple_data, &schema).ok()?;
    let pk_value = values.get(table_meta.pk_index)?;
    pk_value.to_key().map(|k| k.as_bytes().to_vec())
}

/// R-T0b-R7 (D10)：重放前对每张目录表扫描数据页链，构建磁盘版本多映射。
///
/// 页链遍历与 slot 遍历同 `build_superseded_map` 形态（buffer pool 读路径 +
/// `SlottedPageRef`）；每个 slot 按 tuple PK 归入对应键的候选集，rid 升序。
/// 槽形不完整（长度不足 / header 解析失败）的 slot 跳过。墓碑 slot 保留在
/// 映射中——它们仍是磁盘版本，max-rid 派生 + tuple 校验可正确越过。
async fn build_pk_version_maps(
    buffer_pool: &Arc<BufferPool>,
    table_manager: &Arc<TableManager>,
) -> Result<PkVersionMaps, WalError> {
    let mut maps = PkVersionMaps::default();
    let tables =
        table_manager.catalog().scan_tables().await.map_err(|e| {
            WalError::RedoFailed(format!("recovery pre-scan catalog failed: {}", e))
        })?;

    for row in tables {
        let table_name = row.table_name.clone();
        let meta = table_manager.get_table(&table_name).await.map_err(|e| {
            WalError::RedoFailed(format!(
                "recovery pre-scan: table '{}' lookup failed: {}",
                table_name, e
            ))
        })?;
        let keyed_map = maps.keyed.entry(table_name.clone()).or_default();
        let keyless_map = maps.keyless.entry(table_name).or_default();

        let mut page_id = Some(meta.data_page_head);
        while let Some(pid) = page_id {
            let (next_page, entries) = buffer_pool
                .with_page_data(pid, |data| -> PreScanPage {
                    let slotted = SlottedPageRef::new(data);
                    let mut entries = Vec::new();
                    for index in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(index) else {
                            continue;
                        };
                        let slot_data = slotted.get_slot_data(&slot);
                        if slot_data.len() < VersionHeader::SIZE {
                            continue;
                        }
                        if VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]).is_none() {
                            continue;
                        }
                        let slot_tuple = &slot_data[VersionHeader::SIZE..];
                        // T8-R2: 无键 slot 按 tuple 原始字节入 keyless 桶
                        let slot_key = match extract_pk_key(&meta, slot_tuple) {
                            Some(key) => PrescanSlotKey::Keyed(key),
                            None => PrescanSlotKey::Keyless(slot_tuple.to_vec()),
                        };
                        entries.push((slot_key, RowId::new(pid.0 as u32, slot.logical_id)));
                    }
                    Ok((slotted.header().next_page_id, entries))
                })
                .await
                .map_err(|e| {
                    WalError::RedoFailed(format!(
                        "recovery pre-scan of table '{}' page {:?} failed: {}",
                        meta.name, pid, e
                    ))
                })?;

            for (slot_key, rid) in entries {
                let bucket = match slot_key {
                    PrescanSlotKey::Keyed(key) => keyed_map.entry(key).or_default(),
                    PrescanSlotKey::Keyless(tuple) => keyless_map.entry(tuple).or_default(),
                };
                bucket.push(rid);
            }
            page_id = if next_page == 0 {
                None
            } else {
                Some(PageId(next_page as u64))
            };
        }

        for candidates in keyed_map.values_mut().chain(keyless_map.values_mut()) {
            candidates.sort_by_key(|rid| (rid.page_id, rid.slot_id));
        }
    }

    Ok(maps)
}

/// R-T0b-R7 (D10)：从多映射派生 Update 的 `old_row_id`。
///
/// 候选集升序，取 `< record.row_id` 的最大 rid（同键版本链 rid 序 ==
/// LSN 序：同行写者被行锁串行化，提交序即执行序，WAL 按提交序写）；
/// 派生槽的 tuple 必须与 WAL `old_tuple` 逐字节一致，否则显式报错（K05）。
async fn derive_old_row_id(
    buffer_pool: &Arc<BufferPool>,
    table_name: &str,
    candidates: &[RowId],
    record_row_id: RowId,
    old_tuple: &[u8],
) -> Result<RowId, WalError> {
    let pos = candidates.partition_point(|r| {
        (r.page_id, r.slot_id) < (record_row_id.page_id, record_row_id.slot_id)
    });
    if pos == 0 {
        return Err(WalError::RedoFailed(format!(
            "update redo: table '{}' old key has no version before row {:?} on disk",
            table_name, record_row_id
        )));
    }
    let old_row_id = candidates[pos - 1];

    let slot_tuple = buffer_pool
        .with_page_data(
            PageId(old_row_id.page_id as u64),
            |data| -> crate::storage::Result<Vec<u8>> {
                let slotted = SlottedPageRef::new(data);
                let Some((slot, _)) = slotted.get_slot_by_logical_id(old_row_id.slot_id) else {
                    return Err(crate::storage::StorageError::SlotNotFound(old_row_id));
                };
                let slot_data = slotted.get_slot_data(&slot);
                if slot_data.len() < VersionHeader::SIZE {
                    return Err(crate::storage::StorageError::Internal(format!(
                        "derived slot {:?} shorter than version header",
                        old_row_id
                    )));
                }
                Ok(slot_data[VersionHeader::SIZE..].to_vec())
            },
        )
        .await
        .map_err(|e| {
            WalError::RedoFailed(format!(
                "update redo: table '{}' derived slot {:?} read failed: {}",
                table_name, old_row_id, e
            ))
        })?;

    if slot_tuple != old_tuple {
        return Err(WalError::RedoFailed(format!(
            "update redo: table '{}' derived slot {:?} tuple mismatch with WAL old_tuple",
            table_name, old_row_id
        )));
    }

    Ok(old_row_id)
}

/// 恢复管理器
pub struct RecoveryManager;

impl RecoveryManager {
    /// 基础恢复：仅识别已提交/已回滚事务
    pub fn recover(db_path: &Path) -> Result<(HashSet<u64>, HashSet<u64>), WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok((HashSet::new(), HashSet::new()));
        }

        let mut reader = WalReader::open(&wal_path)?;
        let records = reader.read_all()?;

        let mut committed_tx_ids = HashSet::new();
        let mut aborted_tx_ids = HashSet::new();

        for record in records {
            match record {
                WalRecord::Commit { tx_id, .. } | WalRecord::CommitTxn { tx_id, .. } => {
                    committed_tx_ids.insert(tx_id);
                }
                WalRecord::Abort { tx_id } | WalRecord::AbortTxn { tx_id } => {
                    aborted_tx_ids.insert(tx_id);
                }
                _ => {}
            }
        }

        Ok((committed_tx_ids, aborted_tx_ids))
    }

    /// 完整恢复：Redo committed 事务 + 清理 uncommitted 事务
    ///
    /// 策略：
    /// 1. 扫描 WAL 识别 committed/aborted/uncommitted 事务
    /// 2. Redo committed 事务的 Insert/Update 操作（幂等）
    /// 3. Mark uncommitted 事务的 tuple 为 aborted
    pub async fn full_recover(
        db_path: &Path,
        buffer_pool: Arc<BufferPool>,
        table_manager: Arc<TableManager>,
    ) -> Result<RecoveryResult, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(RecoveryResult::default());
        }

        // 消费 checkpoint 位点（16B 语义与 CheckpointManager 一致）：
        // 位点缺失/损坏（<16B）/ LSN 超出 WAL 文件长度（代际失效）→ 全量重放（0）；
        // 有效位点语义 = 只重放记录偏移 ≥ site 的数据记录（位点前缀已由刷脏页覆盖）
        let site = super::checkpoint::read_site_file(&db_path.with_extension("checkpoint"))?;
        let wal_len = std::fs::metadata(&wal_path)
            .map_err(|e| WalError::IoError(e.to_string()))?
            .len();
        let redo_from = match site {
            Some((lsn, _)) if lsn <= wal_len => lsn,
            _ => 0,
        };

        let mut reader = WalReader::open(&wal_path)?;
        let records = reader.read_all_with_lsn()?;

        if records.is_empty() {
            return Ok(RecoveryResult::default());
        }

        // Step 1: Classify transactions（分类始终覆盖全部记录，不因位点裁剪）
        let mut all_tx_ids = HashSet::new();
        let mut committed_tx_ids = HashSet::new();
        let mut aborted_tx_ids = HashSet::new();
        let mut data_records: Vec<(u64, &WalRecord)> = Vec::new();

        for (lsn, record) in &records {
            match record {
                WalRecord::BeginTxn { tx_id } => {
                    all_tx_ids.insert(*tx_id);
                }
                WalRecord::Commit { tx_id, .. } | WalRecord::CommitTxn { tx_id, .. } => {
                    committed_tx_ids.insert(*tx_id);
                }
                WalRecord::Abort { tx_id } | WalRecord::AbortTxn { tx_id } => {
                    aborted_tx_ids.insert(*tx_id);
                }
                WalRecord::Insert { .. } | WalRecord::Update { .. } | WalRecord::Delete { .. } => {
                    data_records.push((*lsn, record));
                }
                _ => {}
            }
        }

        let uncommitted_tx_ids: HashSet<u64> =
            all_tx_ids.difference(&committed_tx_ids).cloned().collect();
        let uncommitted_tx_ids: HashSet<u64> = uncommitted_tx_ids
            .difference(&aborted_tx_ids)
            .cloned()
            .collect();

        // Step 2: Redo committed transactions after the checkpoint site
        // （K05 显式化：任何 redo 失败立即返回 Err，不再静默吞掉）
        //
        // D10 (R-T0b-R7) 门控：存在 lsn >= redo_from 的已提交 data 记录
        // ⟺ redo_count > 0。该形态下重放完全不消费磁盘 B-Tree（撕裂树
        // 基座上任何读都不健全）：Update 的 old_row_id 由磁盘版本多映射
        // 派生，PK 索引由重放后的重建统一负责（R-T0b-R8）。无重放时
        // （redo_count == 0）保持既有路径逐字节不变。
        let will_redo = data_records
            .iter()
            .any(|(lsn, record)| committed_tx_ids.contains(&record.tx_id()) && *lsn >= redo_from);

        let mut ctx = if will_redo {
            let pk_versions = build_pk_version_maps(&buffer_pool, &table_manager).await?;
            RedoContext {
                table_last_page: HashMap::new(),
                pk_versions: Some(pk_versions),
            }
        } else {
            RedoContext::new()
        };
        let mut redo_count = 0;
        for (lsn, record) in &data_records {
            let tx_id = record.tx_id();
            if committed_tx_ids.contains(&tx_id) && *lsn >= redo_from {
                Self::redo_record(record, &buffer_pool, &table_manager, &mut ctx).await?;
                redo_count += 1;
            }
        }

        // Step 3: Mark uncommitted tuples as aborted
        Self::mark_uncommitted_aborted(&uncommitted_tx_ids, &buffer_pool).await?;

        // Step 4 (R-T0b-R8, D10)：有重放时从最终数据页重建各表 PK 索引并
        // 换入（redo_count == 0 的 clean 打开零变化——不进入重建路径）
        if redo_count > 0 {
            Self::rebuild_pk_indexes(&buffer_pool, &table_manager, &committed_tx_ids).await?;
        }

        Ok(RecoveryResult {
            committed_tx_ids,
            aborted_tx_ids,
            uncommitted_tx_ids,
            redo_count,
        })
    }

    /// 重放单条 WAL 记录
    ///
    /// 表缺失或页/索引操作失败时显式报错（K05：恢复不再静默吞错）
    ///
    /// T0b 位置寻址重放：Insert/Update 不再走 `write_tuple_to_data_page`
    /// 追加到当前 tail，而是按 WAL 记录的 `row_id` 写入（已存在跳过、
    /// 稠密落位校验、未初始化页 init）。Update 重建 `next_version` 链
    /// （`old_row_id` 派生）。Delete 双步：索引清理 + 墓碑。页间切换由
    /// `RedoContext::table_last_page` 重建 next 链。
    ///
    /// D10 (R-T0b-R7)：存在重放（`ctx.pk_versions` 在位）时三臂 B-Tree
    /// 自由——Update 的 `old_row_id` 由磁盘版本多映射派生，判重与索引
    /// 维护移至重放后的 PK 索引重建（R-T0b-R8）；`redo_count == 0` 的
    /// 打开维持既有磁盘树路径不变。
    async fn redo_record(
        record: &WalRecord,
        buffer_pool: &Arc<BufferPool>,
        table_manager: &Arc<TableManager>,
        ctx: &mut RedoContext,
    ) -> Result<(), WalError> {
        match record {
            WalRecord::Insert {
                table_name,
                row_id,
                tuple_data,
                tx_id,
            } => {
                let table_meta = table_manager.get_table(table_name).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "table '{}' lookup failed during redo: {}",
                        table_name, e
                    ))
                })?;

                // 1. 位置寻址写入。header 用已提交编码（create == commit ==
                // tx）：redo 只重放已提交事务，与运行期 commit 传播语义一致；
                // R6 扫描去重（superseder 需 commit_tx_id = Some）与 R8 重建
                // 谓词都依赖该编码。
                let version_header = VersionHeader::new(*tx_id, Some(*tx_id));
                redo_tuple_at_row_id(
                    buffer_pool,
                    &table_meta,
                    *row_id,
                    &version_header,
                    tuple_data,
                    ctx,
                )
                .await?;

                // 2. 索引维护——D10 (R-T0b-R7)：重放形态下 B-Tree 自由，
                // 位置写入后向多映射追加版本；判重职责移至重放后的重建
                // （R-T0b-R8，K05 保持）。redo_count == 0 维持磁盘树路径。
                // T8-R2：无键版本按 tuple 原始字节入 keyless 桶。
                if ctx.is_deindexed() {
                    let maps = ctx.pk_versions.as_mut().expect("deindexed mode");
                    match extract_pk_key(&table_meta, tuple_data) {
                        Some(key) => {
                            let candidates = maps
                                .keyed
                                .entry(table_name.clone())
                                .or_default()
                                .entry(key)
                                .or_default();
                            insert_row_id_sorted(candidates, *row_id);
                        }
                        None => {
                            let candidates = maps
                                .keyless
                                .entry(table_name.clone())
                                .or_default()
                                .entry(tuple_data.clone())
                                .or_default();
                            insert_row_id_sorted(candidates, *row_id);
                        }
                    }
                    return Ok(());
                }
                let key = match extract_pk_key(&table_meta, tuple_data) {
                    Some(k) => k,
                    None => {
                        return Err(WalError::RedoFailed(format!(
                            "insert redo: table '{}' PK extraction failed",
                            table_name
                        )))
                    }
                };
                match table_meta.index_manager.search(&key).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "insert redo index search in '{}' failed: {}",
                        table_name, e
                    ))
                })? {
                    Some(existing_rid) if existing_rid == *row_id => {
                        // 同位：幂等跳据（crash-during-recovery re-entry）
                    }
                    Some(other) => {
                        return Err(WalError::RedoFailed(format!(
                            "insert redo: table '{}' key already mapped to {:?}, \
                             WAL row_id {:?}",
                            table_name, other, row_id
                        )));
                    }
                    None => {
                        table_meta
                            .index_manager
                            .insert(&key, *row_id)
                            .await
                            .map_err(|e| {
                                WalError::RedoFailed(format!(
                                    "insert redo index insert in '{}' failed: {}",
                                    table_name, e
                                ))
                            })?;
                    }
                }
                Ok(())
            }
            WalRecord::Update {
                table_name,
                row_id,
                old_tuple,
                new_tuple,
                tx_id,
                ..
            } => {
                let table_meta = table_manager.get_table(table_name).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "table '{}' lookup failed during redo: {}",
                        table_name, e
                    ))
                })?;

                // 1. 从 old_tuple 解 PK → 派生 old_row_id。
                //    D10 (R-T0b-R7)：重放形态下经多映射派生（max rid <
                //    record.row_id + old_tuple 逐字节校验，K05）；T8-R2：
                //    old_tuple 无键时按 tuple 原始字节从 keyless 桶取候选集；
                //    否则磁盘树 search（redo_count == 0 既有路径，无键仍
                //    RedoFailed——该分支对重放记录不可达）。
                let old_key = extract_pk_key(&table_meta, old_tuple);
                let old_row_id = if ctx.is_deindexed() {
                    let maps = ctx.pk_versions.as_ref().expect("deindexed mode");
                    let candidates: &[RowId] = match &old_key {
                        Some(key) => maps
                            .keyed
                            .get(table_name.as_str())
                            .and_then(|m| m.get(key))
                            .map(|v| v.as_slice())
                            .unwrap_or(&[]),
                        None => maps
                            .keyless
                            .get(table_name.as_str())
                            .and_then(|m| m.get(old_tuple))
                            .map(|v| v.as_slice())
                            .unwrap_or(&[]),
                    };
                    derive_old_row_id(buffer_pool, table_name, candidates, *row_id, old_tuple)
                        .await?
                } else {
                    let old_key = old_key.ok_or_else(|| {
                        WalError::RedoFailed(format!(
                            "update redo: table '{}' old PK extraction failed",
                            table_name
                        ))
                    })?;
                    table_meta
                        .index_manager
                        .search(&old_key)
                        .await
                        .map_err(|e| {
                            WalError::RedoFailed(format!(
                                "update redo index search in '{}' failed: {}",
                                table_name, e
                            ))
                        })?
                        .ok_or_else(|| {
                            WalError::RedoFailed(format!(
                                "update redo: table '{}' old key not in index",
                                table_name
                            ))
                        })?
                };

                // 2. 新版本头 = 已提交编码 + next_version(old_row_id)
                //    （commit 语义同 Insert 臂——R6 压制与 R8 谓词依赖）
                let version_header =
                    VersionHeader::new(*tx_id, Some(*tx_id)).with_next_version(old_row_id);

                // 3. 位置寻址写入新版本
                redo_tuple_at_row_id(
                    buffer_pool,
                    &table_meta,
                    *row_id,
                    &version_header,
                    new_tuple,
                    ctx,
                )
                .await?;

                // 4. 索引维护——D10 (R-T0b-R7)：重放形态下向多映射追加新
                // 版本，索引由重建统一负责；redo_count == 0 维持磁盘树 update。
                // T8-R2：无键新版本按 tuple 原始字节入 keyless 桶。
                if ctx.is_deindexed() {
                    let maps = ctx.pk_versions.as_mut().expect("deindexed mode");
                    match extract_pk_key(&table_meta, new_tuple) {
                        Some(new_key) => {
                            let candidates = maps
                                .keyed
                                .entry(table_name.clone())
                                .or_default()
                                .entry(new_key)
                                .or_default();
                            insert_row_id_sorted(candidates, *row_id);
                        }
                        None => {
                            let candidates = maps
                                .keyless
                                .entry(table_name.clone())
                                .or_default()
                                .entry(new_tuple.clone())
                                .or_default();
                            insert_row_id_sorted(candidates, *row_id);
                        }
                    }
                } else {
                    let new_key = match extract_pk_key(&table_meta, new_tuple) {
                        Some(k) => k,
                        None => {
                            return Err(WalError::RedoFailed(format!(
                                "update redo: table '{}' new PK extraction failed",
                                table_name
                            )))
                        }
                    };
                    table_meta
                        .index_manager
                        .update(&new_key, *row_id)
                        .await
                        .map_err(|e| {
                            WalError::RedoFailed(format!(
                                "update redo index update in '{}' failed: {}",
                                table_name, e
                            ))
                        })?;
                }
                Ok(())
            }
            WalRecord::Delete {
                table_name, row_id, ..
            } => {
                let table_meta = table_manager.get_table(table_name).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "table '{}' lookup failed during redo: {}",
                        table_name, e
                    ))
                })?;

                // 1. 索引清理：find_key_by_row_id → delete。
                //    D10 (R-T0b-R7)：重放形态下 B-Tree 自由（索引由重建统一
                //    负责）；redo_count == 0 维持既有路径。
                if !ctx.is_deindexed() {
                    if let Some(key) = table_meta.index_manager.find_key_by_row_id(*row_id).await {
                        table_meta.index_manager.delete(&key).await.map_err(|e| {
                            WalError::RedoFailed(format!(
                                "delete redo of table '{}' row {:?} failed: {}",
                                table_name, row_id, e
                            ))
                        })?;
                    }
                }

                // 2. 墓碑：read_version_header → mark_deleted → update_version_header_in_data_page
                //    SlotNotFound 按 delete.rs:76-80 语义跳过（PK 索引已清即足够保证查询正确）
                let page_id = PageId(row_id.page_id as u64);
                match buffer_pool.read_version_header(*row_id).await {
                    Ok(vh) => {
                        let deleted_vh = vh.mark_deleted();
                        update_version_header_in_data_page(buffer_pool, *row_id, deleted_vh, &[])
                            .await
                            .map_err(|e| {
                                WalError::RedoFailed(format!(
                                    "delete redo tombstone update in '{}' row {:?} failed: {}",
                                    table_name, row_id, e
                                ))
                            })?;
                        // M21: clear page visibility after marking deleted
                        buffer_pool.clear_all_visible(page_id);
                    }
                    Err(crate::storage::StorageError::SlotNotFound(_)) => {
                        // SlotNotFound 跳过（delete.rs:76-80 对齐）
                    }
                    Err(e) => {
                        return Err(WalError::RedoFailed(format!(
                            "delete redo read_version_header in '{}' row {:?} failed: {}",
                            table_name, row_id, e
                        )));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// MS10-T02 Iter000 004-rework (R-T0b-R8, D10)：重放完成后从最终数据页
    /// 重建各表 PK 索引并换入（门控 `redo_count > 0`）。
    ///
    /// 撕裂树教训（Plan Review 3 独立复现）：中位点 checkpoint 后页驱逐按
    /// LRU 而非树拓扑刷盘，磁盘树含洞/孤儿——恢复对其任何消费都不健全。
    /// 重建只信任数据页：
    ///
    /// 1. 新建空 `IndexManager`（先于旧树释放分配——新树永不复用本表旧树页）；
    /// 2. 一遍扫描数据页链：slot → (rid, header, PK key)，收集所有被
    ///    `next_version` 指向的 rid（链内部）；
    /// 3. 对每个链尾沿 next_version 回溯，取第一个「已提交 ∧ 非墓碑」版本：
    ///    运行期已提交落盘编码 commit_tx_id = Some(tx)；崩溃前被驱逐的未
    ///    提交副本 commit_tx_id = None，以 create_tx ∈ committed_tx_ids 判定
    ///    （重放只写已提交编码，R7）。墓碑：创作者 ∈ committed_tx_ids →
    ///    提交删除，整行不建条目；否则视为未提交删除，继续回溯。
    ///    同键第二条目 → 显式报错（K05 判重职责自 redo 移至此处）；
    /// 4. 批量 insert 新索引 → 换入 → `update_table_root` 持久化新根
    ///    （重建根与数据页自洽；后续运行期 split 经 R5 上下文可达）；
    /// 5. 旧树洞容忍收集 + free（失败 warn + 放弃，先例 drop_table）。
    async fn rebuild_pk_indexes(
        buffer_pool: &Arc<BufferPool>,
        table_manager: &Arc<TableManager>,
        committed_tx_ids: &HashSet<u64>,
    ) -> Result<(), WalError> {
        let tables = table_manager.catalog().scan_tables().await.map_err(|e| {
            WalError::RedoFailed(format!("index rebuild catalog scan failed: {}", e))
        })?;

        for row in tables {
            let table_name = row.table_name.clone();
            let meta = table_manager.get_table(&table_name).await.map_err(|e| {
                WalError::RedoFailed(format!(
                    "index rebuild: table '{}' lookup failed: {}",
                    table_name, e
                ))
            })?;

            // 1. 新索引实例（先分配，后释放旧树）
            let bp = buffer_pool.clone();
            let new_index = Arc::new(
                tokio::task::spawn_blocking(move || IndexManager::new(bp))
                    .await
                    .map_err(|e| WalError::RedoFailed(format!("index rebuild join error: {}", e)))?
                    .map_err(|e| {
                        WalError::RedoFailed(format!(
                            "index rebuild init for '{}' failed: {}",
                            table_name, e
                        ))
                    })?,
            );

            // 2. 扫描最终数据页
            let mut slots: HashMap<RowId, (VersionHeader, Option<Vec<u8>>)> = HashMap::new();
            let mut pointed: HashSet<RowId> = HashSet::new();
            let mut page_id = Some(meta.data_page_head);
            while let Some(pid) = page_id {
                let (next_page, page_slots) = buffer_pool
                    .with_page_data(pid, |data| -> RebuildScanPage {
                        let slotted = SlottedPageRef::new(data);
                        let mut page_slots = Vec::new();
                        for index in 0..slotted.slot_count() {
                            let Some(slot) = slotted.get_slot(index) else {
                                continue;
                            };
                            let slot_data = slotted.get_slot_data(&slot);
                            if slot_data.len() < VersionHeader::SIZE {
                                continue;
                            }
                            let Some(vh) =
                                VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                            else {
                                continue;
                            };
                            let key = extract_pk_key(&meta, &slot_data[VersionHeader::SIZE..]);
                            if let Some(target) = vh.next_version() {
                                pointed.insert(target);
                            }
                            page_slots.push((RowId::new(pid.0 as u32, slot.logical_id), vh, key));
                        }
                        Ok((slotted.header().next_page_id, page_slots))
                    })
                    .await
                    .map_err(|e| {
                        WalError::RedoFailed(format!(
                            "index rebuild scan of '{}' page {:?} failed: {}",
                            table_name, pid, e
                        ))
                    })?;

                for (rid, vh, key) in page_slots {
                    slots.insert(rid, (vh, key));
                }
                page_id = if next_page == 0 {
                    None
                } else {
                    Some(PageId(next_page as u64))
                };
            }

            // 3. 链尾回溯：不被指向的 slot 为链尾，new→old 取首个存活版本
            let mut entries: HashMap<Vec<u8>, RowId> = HashMap::new();
            for tail_rid in slots.keys().copied().collect::<Vec<_>>() {
                if pointed.contains(&tail_rid) {
                    continue;
                }
                let mut rid = tail_rid;
                loop {
                    let (vh, key) = &slots[&rid];
                    if vh.is_deleted() {
                        if committed_tx_ids.contains(&vh.create_tx_id()) {
                            break; // 提交删除：整行不建条目
                        }
                        // 未提交删除：继续回溯（既有未提交删除崩溃语义）
                    } else if vh.commit_tx_id().is_some()
                        || committed_tx_ids.contains(&vh.create_tx_id())
                    {
                        // 首个「已提交 ∧ 非墓碑」版本 = 该键的存活版本
                        if let Some(key) = key {
                            if entries.contains_key(key) {
                                return Err(WalError::RedoFailed(format!(
                                    "index rebuild: table '{}' duplicate PK across chains",
                                    table_name
                                )));
                            }
                            entries.insert(key.clone(), rid);
                        }
                        break;
                    }
                    // 未提交版本 / 未提交删除：回溯旧版本
                    match vh.next_version() {
                        Some(target) if slots.contains_key(&target) => rid = target,
                        _ => break,
                    }
                }
            }

            // 4. 写入新索引 → 换入 → 持久化新根
            for (key, rid) in &entries {
                new_index.insert(key, *rid).await.map_err(|e| {
                    WalError::RedoFailed(format!(
                        "index rebuild insert into '{}' failed: {}",
                        table_name, e
                    ))
                })?;
            }
            let old_index = table_manager
                .replace_index_manager(&table_name, new_index.clone())
                .await
                .map_err(|e| {
                    WalError::RedoFailed(format!(
                        "index rebuild swap-in for '{}' failed: {}",
                        table_name, e
                    ))
                })?;
            table_manager
                .catalog()
                .update_table_root(&table_name, new_index.root_page_id().0 as u32)
                .await
                .map_err(|e| {
                    WalError::RedoFailed(format!(
                        "index rebuild catalog root update for '{}' failed: {}",
                        table_name, e
                    ))
                })?;

            // 5. 旧树释放（洞容忍；失败 warn + 泄漏，先例 drop_table）
            for page in old_index.collect_all_pages_tolerant().await {
                if let Err(e) = buffer_pool.free_page(page).await {
                    eprintln!(
                        "[recovery] free old index page {:?} of '{}' failed: {}",
                        page, table_name, e
                    );
                }
            }
        }

        Ok(())
    }

    /// Mark all uncommitted tuples as aborted so MVCC skips them
    async fn mark_uncommitted_aborted(
        uncommitted_tx_ids: &HashSet<u64>,
        buffer_pool: &Arc<BufferPool>,
    ) -> Result<(), WalError> {
        for tx_id in uncommitted_tx_ids {
            buffer_pool.mark_tx_aborted(*tx_id).await.map_err(|e| {
                WalError::RedoFailed(format!("mark tx {} aborted failed: {}", tx_id, e))
            })?;
        }
        Ok(())
    }

    /// 检查是否需要恢复
    pub fn needs_recovery(db_path: &Path) -> Result<bool, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(false);
        }

        let metadata =
            std::fs::metadata(&wal_path).map_err(|e| WalError::IoError(e.to_string()))?;

        Ok(metadata.len() > 0)
    }

    /// 读取所有 WAL 记录
    pub fn read_wal(db_path: &Path) -> Result<Vec<WalRecord>, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(Vec::new());
        }

        let mut reader = WalReader::open(&wal_path)?;
        reader.read_all()
    }
}
