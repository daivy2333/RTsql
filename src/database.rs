//! Database coordinator - wires together all RTsql components
//!
//! M7: Single entry point for opening databases, creating tables, and executing SQL.
//! M11: WAL integration for crash recovery.

use crate::network::protocol::Response;
use crate::plan_cache::PlanCache;
use crate::storage::{
    BufferPool, CatalogKind, ColumnType, FileStorage, MemoryStorage, Result, StorageError,
    TableManager, TableMeta,
};
use crate::transaction::{IsolationLevel, Snapshot, Transaction, TransactionManager};
use crate::wal::{CheckpointManager, RecoveryManager, WALBuffer, WalReader, WalRecord, WalWriter};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, RwLock};

/// MS26 T1/D1：打开模式。`ReadWrite` 是既有全部入口的默认形态（行为逐字节
/// 不变）；`ReadOnly` 经 advisory 共享锁多读单写，WAL / checkpoint 通道
/// 结构性不存在（`readonly-open-mode` R1/R3）；`Memory` 是 MS26 T3 新增的
/// `:memory:` 形态——可写、无文件（`in-memory-database` R1/R2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    ReadWrite,
    ReadOnly,
    Memory,
}

/// MS26 T1：只读打开的 WAL 未决谓词（`readonly-open-mode` R2）。
///
/// 判定在任何页写入与恢复重放之前完成：`.wal` 不存在、为空、或仅含
/// `Checkpoint` 标记记录（干净关闭后步骤 6/7 的残留形态）放行；任何其他
/// 记录（数据重放、未提交事务）点名拒绝。只读会话不执行重放——恢复经
/// BufferPool 驱逐刷脏页落盘，只读会话无法保证不写盘。
fn assert_no_pending_wal(path: &Path) -> Result<()> {
    let wal_path = path.with_extension("wal");
    if !wal_path.exists() {
        return Ok(());
    }
    let to_storage = |e: crate::wal::WalError| StorageError::WalError(e.to_string());
    let mut reader = WalReader::open(&wal_path).map_err(to_storage)?;
    let records = reader.read_all().map_err(to_storage)?;
    if records
        .iter()
        .all(|record| matches!(record, WalRecord::Checkpoint { .. }))
    {
        return Ok(());
    }
    Err(StorageError::WalPendingRecovery(
        wal_path.display().to_string(),
    ))
}

/// MS28 T2（design D5）：FileStorage 的同步构造（锁等待 + 头校验 + KDF 均为
/// 阻塞段）在专用 `std::thread` 中执行，经 oneshot 回传——open future 在锁
/// 等待期间真实让出，CLI 阶段 1 `tokio::select!` 的信号分支可达（等待可被
/// SIGINT/SIGTERM 即时中断，分支代码零改动）。`Memory` 臂无锁无阻塞，保持
/// 内联不走本通道。`RecvError` 仅 detached 线程 panic 形态可达（线程不被
/// runtime drop 等待，取消安全随进程退出终止），映射 `StorageError::Internal`。
async fn construct_file_storage(
    mode: OpenMode,
    path: &Path,
    key: Option<&str>,
    lock_wait: Option<Duration>,
) -> Result<FileStorage> {
    let path = path.to_path_buf();
    let key = key.map(str::to_string);
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = match mode {
            OpenMode::ReadWrite => FileStorage::open_with_lock(&path, key.as_deref(), lock_wait),
            OpenMode::ReadOnly => {
                FileStorage::open_readonly_with_lock(&path, key.as_deref(), lock_wait)
            }
            // 内存臂在 open_internal 内联构造，永不达本通道。
            OpenMode::Memory => unreachable!("memory mode constructs MemoryStorage inline"),
        };
        let _ = tx.send(result);
    });
    match rx.await {
        Ok(result) => result,
        Err(e) => Err(StorageError::Internal(format!("storage init failed: {e}"))),
    }
}

/// MS21 Iter005 (T5-a/D12): in-memory view registry entry — the persisted
/// definition SQL plus the persisted column-name list (the explicit column
/// list, or the definition's output header when none was given). Loaded
/// from the catalog at open time; kept in sync by the CREATE/DROP VIEW
/// executors. Views are non-materialized: a `ViewMeta` is the whole
/// in-memory footprint (no data pages, no index trees).
#[derive(Debug, Clone)]
pub struct ViewMeta {
    pub sql: String,
    pub columns: Vec<String>,
}

/// Database is the central coordinator that owns all major RTsql subsystems.
#[derive(Clone)]
pub struct Database {
    pub buffer_pool: Arc<BufferPool>,
    pub table_manager: Arc<TableManager>,
    pub transaction_manager: Arc<TransactionManager>,
    /// MS26 T1/D1：仅 `ReadWrite` 模式为 `Some`——只读会话不持有任何 WAL
    /// 句柄（零写入的结构性保证，`readonly-open-mode` R3）。
    pub wal_writer: Option<Arc<WalWriter>>,
    pub wal_buffer: Option<Arc<WALBuffer>>,
    pub plan_cache: Arc<PlanCache>,
    /// MS26 T1/D1：同上，非 `ReadWrite` 模式为 `None`。
    pub checkpoint_manager: Option<Arc<CheckpointManager>>,
    /// MS09 Iter000 (D4): isolation level fixed at open time. The default
    /// `RepeatableRead` keeps every existing behavior byte for byte.
    pub isolation: IsolationLevel,
    /// MS21 Iter005 (T5-a/D12): persisted view definitions by name.
    pub views: Arc<RwLock<HashMap<String, ViewMeta>>>,
    /// MS26 T1/D1：打开模式（构造期一次定型，运行期不变）。
    mode: OpenMode,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        Self::open_with_isolation(path, IsolationLevel::RepeatableRead).await
    }

    /// Open a database with an explicit isolation level (MS09 Iter000, D4).
    ///
    /// `RepeatableRead` is equivalent to [`Database::open`]. Under
    /// `ReadCommitted`, every query statement constructs a snapshot at
    /// statement start (see [`Database::statement_snapshot`]) and threads it
    /// through all scan constructions, so only transactions already
    /// committed at that point are visible.
    pub async fn open_with_isolation(path: &Path, isolation: IsolationLevel) -> Result<Self> {
        Self::open_with_key(path, isolation, None).await
    }

    /// Open a database with an optional encryption key (MS17 Iter000).
    ///
    /// `key` is `Some` for encrypted databases (or to create one from a
    /// zero-byte file); `None` opens or creates a plaintext database.
    /// Plaintext behavior is identical to [`Database::open_with_isolation`].
    pub async fn open_with_key(
        path: &Path,
        isolation: IsolationLevel,
        key: Option<&str>,
    ) -> Result<Self> {
        Self::open_internal(OpenMode::ReadWrite, path, isolation, key, None).await
    }

    /// MS28 T2：锁等待预算可配置的打开核心（`database-file-lock` R1 MODIFIED
    /// 的 lib 面通道）。`lock_wait` 为 advisory 锁等待预算——`None` → 默认
    /// 5000ms；`Some(0)` ＝ 仅 ~8ms 可见性下限窗口（MS28 前语义的显式表达）；
    /// 预算耗尽维持 `DatabaseLocked` 既有错误面。FileStorage 构造（锁等待 +
    /// 头校验 + KDF 均为阻塞段）在专用 `std::thread` 中执行，open future 在
    /// 等待期间让出、可被信号路径取消（design D5）。既有 open 家族委托
    /// `open_internal` 传 `None`。
    pub async fn open_with_lock_wait(
        path: &Path,
        isolation: IsolationLevel,
        key: Option<&str>,
        lock_wait: Option<Duration>,
    ) -> Result<Self> {
        Self::open_internal(OpenMode::ReadWrite, path, isolation, key, lock_wait).await
    }

    /// MS26 T1：只读打开（`readonly-open-mode` R1/R2）——advisory 共享锁，
    /// 多个只读会话可同时打开同一数据库，任一读写持有者的独占锁都排除
    /// 只读打开（冲突立即 `DatabaseLocked`，不等待）。文件必须已存在（不
    /// 创建），`.wal` 有未决记录时点名拒绝，不运行恢复、不写任何文件。
    /// 隔离级别固定为 `RepeatableRead`（冻结库上「最新已提交」恒定）。
    pub async fn open_readonly(path: &Path) -> Result<Self> {
        Self::open_readonly_with_key(path, None).await
    }

    /// MS26 T1：只读打开的加密库变体（`key` 为 `Some` 时按加密库解密读取，
    /// 错误密钥沿既有 `InvalidKey` 拒绝）。
    pub async fn open_readonly_with_key(path: &Path, key: Option<&str>) -> Result<Self> {
        Self::open_readonly_with_lock_wait(path, key, None).await
    }

    /// MS28 T3：只读打开的锁等待预算通道（`readonly-open-mode` R1 MODIFIED
    /// 共享锁同款等待）——预算语义与错误面同 `open_with_lock_wait` 逐字节
    /// 一致；既有 `open_readonly*` 委托本函数传 `None`。
    pub async fn open_readonly_with_lock_wait(
        path: &Path,
        key: Option<&str>,
        lock_wait: Option<Duration>,
    ) -> Result<Self> {
        Self::open_internal(
            OpenMode::ReadOnly,
            path,
            IsolationLevel::RepeatableRead,
            key,
            lock_wait,
        )
        .await
    }

    /// MS26 T3：`:memory:` 内存库（`in-memory-database` R1/R2）——全部页数据
    /// 由 `MemoryStorage` 承载，不创建也不触碰任何文件（`.db` / `.wal` /
    /// `.checkpoint` 均不产生），无恢复语义（进程退出即弃）。构造链按
    /// `OpenMode::Memory` 裁剪：WAL writer/buffer、恢复与 checkpoint manager
    /// 三段全部跳过，写路径经 `Option<Arc<WALBuffer>>` 天然降级，事务提交、
    /// 回滚还原与版本可见性全部由页与事务管理器承载。内存库可写
    /// （`is_readonly()` 为假）；隔离级别固定 `RepeatableRead`。
    ///
    /// `path` 占位值为 `":memory:"`——`Memory` 模式下不被任何构造步骤消费
    /// （WAL / 恢复 / checkpoint 路径均由 `mode == ReadWrite` 门控）。
    pub async fn open_in_memory() -> Result<Self> {
        Self::open_internal(
            OpenMode::Memory,
            Path::new(":memory:"),
            IsolationLevel::RepeatableRead,
            None,
            None,
        )
        .await
    }

    /// True when this session was opened read-only — the pipeline rejects
    /// every write plan shape in that mode (`readonly-open-mode` R4).
    /// `Memory` sessions are writable, so they report `false`.
    pub fn is_readonly(&self) -> bool {
        self.mode == OpenMode::ReadOnly
    }

    /// 各打开模式共用的构造链。`ReadWrite` 走既有全部步骤（存储 → 缓冲池
    /// → catalog → 视图 → 事务管理器 → WAL writer/buffer → 恢复 → 分配器
    /// 推进 → catalog root 上下文 → plan cache → checkpoint manager）；
    /// `ReadOnly` 裁剪 WAL / 恢复 / checkpoint 构造（写路径经
    /// `Option<Arc<WALBuffer>>` 天然降级，`readonly-open-mode` R3）；`Memory`
    /// 同样裁剪这三段并改用 `MemoryStorage`（`in-memory-database` R2——
    /// 新实例 `page_count() == 0` 使 catalog 走 bootstrap，视图注册表空载入）。
    async fn open_internal(
        mode: OpenMode,
        path: &Path,
        isolation: IsolationLevel,
        key: Option<&str>,
        lock_wait: Option<Duration>,
    ) -> Result<Self> {
        // 1. Initialize storage
        let storage: Arc<dyn crate::storage::AsyncStorage> = match mode {
            // MS28 T2（design D5）：FileStorage 构造移入专用 std::thread +
            // oneshot 回传——锁等待期间 open future 真实让出，CLI 阶段 1
            // select! 的信号分支可达（等待可被 SIGINT/SIGTERM 即时中断）。
            OpenMode::ReadWrite => {
                Arc::new(construct_file_storage(mode, path, key, lock_wait).await?)
            }
            // 只读：共享锁 + 只读句柄。与读写构造同纪律——锁先于伴生文件
            // 判定，被拒的第二打开者不触碰任何文件（MS10-T02 R2）。
            OpenMode::ReadOnly => {
                let storage = construct_file_storage(mode, path, key, lock_wait).await?;
                // WAL 未决谓词先于任何页写入与重放（`readonly-open-mode` R2）。
                assert_no_pending_wal(path)?;
                Arc::new(storage)
            }
            // 内存：无文件路径、无锁、无格式头（`in-memory-database` R1）。
            OpenMode::Memory => Arc::new(MemoryStorage::new()),
        };
        let buffer_pool = Arc::new(BufferPool::new(100, storage.clone())?);
        // MS07-T01: TableManager is async + takes storage. It bootstraps
        // or opens the catalog, then `open_or_init` rebuilds the in-memory
        // cache from the catalog for an existing file.
        let table_manager = TableManager::new(buffer_pool.clone(), storage).await?;
        table_manager.open_or_init().await?;

        // MS21 Iter005 (T5-a/D12): load the persisted view registry —
        // kind=View catalog rows plus their column-name rows (schema order
        // = persisted `column_index` order, same hardening as
        // `open_or_init`; `open_or_init` itself skips view rows).
        let mut views = HashMap::new();
        for row in table_manager.catalog().scan_tables().await? {
            if row.kind != CatalogKind::View {
                continue;
            }
            let mut cols = table_manager
                .catalog()
                .scan_columns(&row.table_name)
                .await?;
            cols.sort_by_key(|c| c.column_index);
            views.insert(
                row.table_name.clone(),
                ViewMeta {
                    sql: row.view_sql.unwrap_or_default(),
                    columns: cols.into_iter().map(|c| c.column_name).collect(),
                },
            );
        }
        let views = Arc::new(RwLock::new(views));
        let transaction_manager = Arc::new(TransactionManager::new());

        // 2. Initialize WAL（只读模式不持有 WAL 句柄——R3 零写入）
        let (wal_writer, wal_buffer) = if mode == OpenMode::ReadWrite {
            let wal_writer = Arc::new(
                WalWriter::open(path)
                    .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?,
            );

            // 2b. Initialize WAL buffer with Group Commit
            let wal_buffer = Arc::new(WALBuffer::new(wal_writer.clone(), 100, 100));
            wal_buffer.start_flush_loop();

            // 2c. Connect WAL buffer to TransactionManager
            transaction_manager.set_wal_buffer(wal_buffer.clone()).await;
            (Some(wal_writer), Some(wal_buffer))
        } else {
            (None, None)
        };

        // 3. Full recovery: Redo committed + cleanup uncommitted（只读不运行）
        if mode == OpenMode::ReadWrite {
            let recovery_result =
                RecoveryManager::full_recover(path, buffer_pool.clone(), table_manager.clone())
                    .await
                    .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?;

            // MS09 Iter000 001-replan (D10/T6-R3): advance the tx id allocator
            // past every id observed in recovery so ids are never reused after a
            // restart — the next allocation is strictly greater than the max
            // committed/aborted/uncommitted id. This is also the soundness
            // premise of the Read Committed high-water mark ("every id ≤ the
            // allocator's current value is committed, aborted, or active").
            //
            // MS17-T02 Iter002 (D8): a checkpointed restart loses the WAL id
            // history (a clean close leaves no Begin/Commit frames), so the
            // observed max alone would reset the allocator to zero. The
            // checkpoint site carries the allocator watermark captured right
            // after the LSN; advancing past the max of both keeps the premise
            // sound on every restart path.
            let wal_observed_max = recovery_result
                .committed_tx_ids
                .iter()
                .chain(recovery_result.aborted_tx_ids.iter())
                .chain(recovery_result.uncommitted_tx_ids.iter())
                .max()
                .copied()
                .unwrap_or(0);
            let site_watermark = recovery_result.checkpoint_tx_watermark.unwrap_or(0);
            transaction_manager.advance_past(std::cmp::max(wal_observed_max, site_watermark));

            // R-T0b-R5: attach catalog root-sync contexts only AFTER recovery —
            // replay-time root changes must not be persisted (the recovery load
            // point stays fixed across re-recoveries).
            table_manager.attach_index_catalog_contexts().await;
        }

        // 4. Initialize plan cache
        let plan_cache = Arc::new(PlanCache::new());

        // 5. Initialize checkpoint manager (MS07-T05)（只读模式为 None——R3）
        let checkpoint_manager = if mode == OpenMode::ReadWrite {
            Some(Arc::new(CheckpointManager::new(
                path,
                wal_writer
                    .clone()
                    .expect("ReadWrite mode holds a WAL writer"),
                buffer_pool.clone(),
            )))
        } else {
            None
        };

        Ok(Self {
            buffer_pool,
            table_manager,
            transaction_manager,
            wal_writer,
            wal_buffer,
            plan_cache,
            checkpoint_manager,
            isolation,
            views,
            mode,
        })
    }

    /// Statement-start snapshot for the current isolation level (MS09
    /// Iter000, D4; statement view per 001-replan D10).
    ///
    /// `RepeatableRead` returns `None` — the historical no-snapshot scan
    /// semantics stay byte for byte. `ReadCommitted` returns a statement
    /// view: the visibility high-water mark is the allocator's current
    /// value (every transaction committed before this statement has an id
    /// ≤ it; auto-commit reads need no WAL BeginTxn), and `reader_tx_id`
    /// carries only self identity — the explicit transaction's id inside
    /// one, otherwise 0 (an auto-commit statement owns no in-flight
    /// versions, and id 0 is the aborted marker so `is_visible_self` never
    /// matches a real version).
    pub(crate) async fn statement_snapshot(&self, reader_tx_id: Option<u64>) -> Option<Snapshot> {
        match self.isolation {
            IsolationLevel::RepeatableRead => None,
            IsolationLevel::ReadCommitted => {
                let active = self.transaction_manager.active_transactions().await;
                Some(Snapshot::statement_view(
                    self.transaction_manager.current_tx_id(),
                    reader_tx_id.unwrap_or(0),
                    active,
                ))
            }
        }
    }

    pub async fn create_table(
        &self,
        name: &str,
        columns: Vec<(String, ColumnType)>,
        pk: &str,
    ) -> Result<()> {
        self.table_manager.create_table(name, columns, pk).await
    }

    pub async fn get_table(&self, name: &str) -> Result<Arc<TableMeta>> {
        self.table_manager.get_table(name).await
    }

    pub async fn execute_sql(&self, sql: &str) -> Response {
        crate::pipeline::execute(self, sql).await
    }

    /// Begin a new explicit transaction (MS07-T04).
    ///
    /// Returns an owned handle; pass it to [`Database::execute_in_tx`] for
    /// every statement that belongs to the transaction, then terminate the
    /// transaction with [`Database::commit`] or [`Database::rollback`].
    /// Statements issued through [`Database::execute_sql`] keep their
    /// implicit auto-commit behavior and do not interact with this handle.
    pub async fn begin(&self) -> Result<Transaction> {
        Ok(self.transaction_manager.begin().await)
    }

    /// Commit an explicit transaction, making all of its writes durable and
    /// visible (WAL commit record + version marking).
    ///
    /// MS26 T5：提交成功后到达水位检查（`wal-auto-checkpoint` R1）——累计
    /// 追加字节达阈值且无其他活跃事务时，同步执行一次完整 checkpoint。
    /// 顺序不可颠倒：事务提交（WAL fsync + 版本标 committed + 出活跃集）
    /// 恒先于水位检查，checkpoint 失败不回滚已提交事务（R4）。
    pub async fn commit(&self, tx: Transaction) -> Result<()> {
        self.transaction_manager
            .commit(tx, &self.buffer_pool)
            .await?;
        self.maybe_auto_checkpoint().await
    }

    /// 提交边界字节水位检查（MS26 T5，`wal-auto-checkpoint` R1/R3/R4）。
    ///
    /// 触发条件（四项合取）：本会话持有 WAL 通道（`ReadWrite`）∧ 阈值非 0
    /// （`0` 禁用）∧ 累计追加字节达阈值 ∧ 无其他活跃事务（`R3` 顺延）。
    /// 计数器为内存原子量，不引入每语句 `metadata()` 调用。
    ///
    /// 单一判定点：[`Database::commit`] 与 CTAS 隐式提交的第二段共用本方法，
    /// pipeline 站点零复制（契约 Invariants「水位逻辑单点」）。
    ///
    /// 错误语义：触发后的 checkpoint 失败以点名 `checkpoint failed` 的错误
    /// 返回给触发该边界的语句；此时事务早已提交，已提交数据不受影响
    /// （`R4`），后续提交边界自然重试（字节计数未归零）。
    pub async fn maybe_auto_checkpoint(&self) -> Result<()> {
        // 只读 / 内存会话结构性无 WAL 通道（R3 零写入），直接返回。
        let Some(writer) = &self.wal_writer else {
            return Ok(());
        };
        let threshold = writer.checkpoint_bytes_threshold();
        if threshold == 0 || writer.bytes_appended() < threshold {
            return Ok(());
        }
        // R3 顺延：其他活跃事务存在时等下一个无活跃事务的提交边界
        // （本事务已由 `commit` 移出活跃集，无需排除自身）。
        if !self
            .transaction_manager
            .active_transactions()
            .await
            .is_empty()
        {
            return Ok(());
        }
        self.checkpoint().await.map_err(|e| {
            crate::storage::StorageError::WalError(format!("checkpoint failed after commit: {e}"))
        })
    }

    /// 设置 WAL 字节水位阈值（MS26 T5，`wal-auto-checkpoint` R2）——打开后
    /// 下发，`0` 禁用自动 checkpoint。只读 / 内存会话无 WAL 通道，为 no-op。
    pub fn set_checkpoint_bytes_threshold(&self, bytes: u64) {
        if let Some(writer) = &self.wal_writer {
            writer.set_checkpoint_bytes_threshold(bytes);
        }
    }

    /// Roll back an explicit transaction: every version the transaction
    /// recorded is cleaned up per table (index restored to the previous
    /// version or deleted, aborted versions tombstoned).
    pub async fn rollback(&self, tx: Transaction) -> Result<()> {
        let tx_id = tx.id();
        // Resolve the TableMeta of every table this transaction touched so
        // abort can roll back index entries across all of them. A table that
        // can no longer be resolved (e.g. dropped afterwards) surfaces as an
        // explicit error from `abort` instead of a silent skip.
        let mut tables = HashMap::new();
        for name in self
            .transaction_manager
            .tx_version_tables(tx_id)
            .await
            .keys()
        {
            if let Ok(meta) = self.table_manager.get_table(name).await {
                tables.insert(name.clone(), meta);
            }
        }
        self.transaction_manager
            .abort(tx, &self.buffer_pool, &tables)
            .await
    }

    /// Execute one SQL statement inside an existing explicit transaction
    /// (MS07-T04).
    ///
    /// DML statements reuse `tx.id()` and skip the implicit
    /// begin/commit/abort wrapping of [`Database::execute_sql`]; SELECT and
    /// DDL behave as in the implicit path (visibility semantics unchanged).
    /// A failed statement returns an error response without terminating the
    /// transaction.
    pub async fn execute_in_tx(&self, sql: &str, tx: &Transaction) -> Response {
        crate::pipeline::execute_in_tx(self, sql, tx.id()).await
    }

    /// Get plan cache size (for testing)
    pub fn plan_cache_len(&self) -> usize {
        self.plan_cache.len()
    }

    /// Flush all dirty buffer-pool pages and the WAL to disk, then record a
    /// checkpoint site and physically truncate the WAL (MS07-T05).
    ///
    /// MS07-T01: callers that drop the `Database` and immediately re-open
    /// the file must call `close()` first, otherwise the in-memory
    /// catalog pages (or any other dirty pages) never reach the on-disk
    /// file and the re-opened database sees an empty schema.
    ///
    /// MS07-T05: close() now performs a full checkpoint — dirty pages are
    /// flushed, the checkpoint site is written and the WAL is rewritten
    /// truncated, so the next open replays (almost) nothing.
    ///
    /// MS18-T01: the checkpoint is skipped entirely when there is nothing
    /// pending — no WAL record appended since the last checkpoint and no
    /// dirty buffer-pool page (a read-only session on an already
    /// checkpoint-clean database). Such a session has nothing to persist, so
    /// the site and WAL are left byte for byte untouched. Pending work of
    /// any kind — DML records, DDL dirty pages, crash-recovery replayed
    /// pages — still takes the full checkpoint path.
    ///
    /// MS26 T1: a read-only session holds no checkpoint manager at all
    /// (`readonly-open-mode` R3) — `close()` is a structural no-op there, not
    /// a zero-work skip that happens to match.
    pub async fn close(&self) -> Result<()> {
        self.checkpoint().await
    }

    /// Run a checkpoint: flush dirty pages, write the checkpoint site and
    /// rewrite-truncate the WAL so the file stays bounded (MS07-T05).
    ///
    /// MS17-T02 Iter002 (D8): the site carries the tx id allocator watermark,
    /// read inside `CheckpointManager::checkpoint` right after the WAL LSN
    /// capture, so a restart after WAL truncation still advances the
    /// allocator past every pre-restart id (Read Committed high-water
    /// soundness).
    ///
    /// MS26 T1: non-read-write sessions own no checkpoint manager and take
    /// the zero-write path without touching the WAL, the site file, or the
    /// buffer pool (`readonly-open-mode` R3).
    pub async fn checkpoint(&self) -> Result<()> {
        let Some(manager) = &self.checkpoint_manager else {
            return Ok(());
        };
        manager
            .checkpoint(|| self.transaction_manager.current_tx_id())
            .await
            .map(|_captured_lsn| ())
            .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))
    }
}
