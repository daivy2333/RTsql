//! Database coordinator - wires together all RTsql components
//!
//! M7: Single entry point for opening databases, creating tables, and executing SQL.
//! M11: WAL integration for crash recovery.

use crate::executor::{PhysicalPlan, Value};
use crate::plan_cache::PlanCache;
use crate::result::{SqlError, SqlOutcome};
use crate::storage::{
    BufferPool, CatalogKind, ColumnType, FileStorage, IoFailPoint, IoFailPointSlot, MemoryStorage,
    Result, StorageError, TableManager, TableMeta,
};
use crate::transaction::{IsolationLevel, Snapshot, Transaction, TransactionManager};
use crate::wal::{CheckpointManager, RecoveryManager, WALBuffer, WalReader, WalRecord, WalWriter};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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
    io_failpoint: IoFailPointSlot,
) -> Result<FileStorage> {
    let path = path.to_path_buf();
    let key = key.map(str::to_string);
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = match mode {
            OpenMode::ReadWrite => FileStorage::open_with_lock_io_failpoint(
                &path,
                key.as_deref(),
                lock_wait,
                io_failpoint,
            ),
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
    /// MS31 Iteration 002（T9）：schema 版本戳——与 plan cache 失效同源递增
    /// （`bump_schema_version` 单点）。预编译语句据此判定计划是否失效。
    ///
    /// 内存语义：open 时归零，不持久化、不进 WAL / catalog——预编译语句不
    /// 跨 open 存活。
    pub schema_version: Arc<AtomicU64>,
    /// MS26 T1/D1：同上，非 `ReadWrite` 模式为 `None`。
    pub checkpoint_manager: Option<Arc<CheckpointManager>>,
    /// MS09 Iter000 (D4): isolation level fixed at open time. The default
    /// `RepeatableRead` keeps every existing behavior byte for byte.
    pub isolation: IsolationLevel,
    /// MS21 Iter005 (T5-a/D12): persisted view definitions by name.
    pub views: Arc<RwLock<HashMap<String, ViewMeta>>>,
    /// MS26 T1/D1：打开模式（构造期一次定型，运行期不变）。
    mode: OpenMode,
    /// MS32 Iteration 003（design D4）：查询内存上限（字节）——语句入口取
    /// 快照构造语句级预算。`0` = 不限制（默认）。`Arc<AtomicU64>` 沿
    /// `schema_version` 先例（`AtomicU64` 不实现 `Clone`）；克隆共享不改
    /// 变快照语义——上限变更只影响之后开始的语句。
    query_memory_limit: Arc<AtomicU64>,
    /// MS33 Iter001（T2.1）：受控 I/O 失败开关单元——主库 `FileStorage` 与
    /// `WalWriter` 持同一单元，`arm_io_failure` 按型注入一次性失败。只读/
    /// 内存臂无写通道不接线；ATTACH 附加库持有各自独立单元，arm 不波及。
    io_failpoint: IoFailPointSlot,
    /// MS34 Iter000（T01）：被放弃的库属隐式事务登记表——登记项只表示
    /// 「持有它的语句 future 已被放弃」。
    ///
    /// 登记项的唯一来源是语句作用域守卫的 `Drop`（语句被放弃 → 守卫析构
    /// → 记入）；语句正常提交或中止成功后守卫显式解除，不产生登记项。
    /// 回收入口在下一个触发点（下一条隐式写语句、checkpoint、自动 checkpoint
    /// 判定）中止它。登记与摘除本身不做 I/O。
    implicit_transactions: Arc<Mutex<HashSet<u64>>>,
    /// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
    /// Iteration 003（T10）：本会话的数据库文件路径——内存会话为占位值
    /// `":memory:"`。测试支撑的注入点按路径限定（并行测试二进制互不抢对方
    /// 的注入点，先例 `src/storage/free_page_rebuild.rs`）。
    path: PathBuf,
}

/// MS34 Iter000（T01）：库属隐式语句的作用域守卫。
///
/// 登记表语义是「调用方已放弃其语句的事务」——只有守卫析构（语句 future
/// 被放弃）才把 tx id 记入登记表；仍在并发执行中的语句不持有登记项，
/// 因此另一条语句的回收不会中止它。
///
/// 语句正常结束（提交或中止成功）时调用方显式 [`ImplicitTxGuard::disarm`]，
/// `Drop` 不记入；提交/中止失败时保持武装 → 登记项留下，由下一个触发点
/// 重试回收。
pub(crate) struct ImplicitTxGuard {
    registry: Arc<Mutex<HashSet<u64>>>,
    tx_id: u64,
    armed: bool,
}

impl ImplicitTxGuard {
    fn new(registry: Arc<Mutex<HashSet<u64>>>, tx_id: u64) -> Self {
        Self {
            registry,
            tx_id,
            armed: true,
        }
    }

    /// 语句正常结束（提交或中止成功）——解除守卫，`Drop` 不再记入登记表。
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ImplicitTxGuard {
    fn drop(&mut self) {
        if self.armed {
            // 同步插入，无 I/O、不 spawn 任务——回收的时序保证仍由三个库
            // 自有触发点承担（design D2）。
            self.registry.lock().unwrap().insert(self.tx_id);
        }
    }
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

    /// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
    /// Iteration 003（T10）：本会话的数据库文件路径（内存会话为 `":memory:"`）。
    ///
    /// 仅供测试支撑的注入点按路径限定使用（`executor::ddl_commit_window`）；
    /// 产品读写路径不经此字段。
    pub fn path(&self) -> &Path {
        &self.path
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
        // MS33 Iter001（T2.1）：主库受控 I/O 失败开关单元——创建后分发同
        // 一单元给 FileStorage（ReadWrite）与 WalWriter；armed 前不改变任何
        // 写入行为。
        let io_failpoint: IoFailPointSlot = Arc::new(std::sync::Mutex::new(None));
        // 1. Initialize storage
        let storage: Arc<dyn crate::storage::AsyncStorage> = match mode {
            // MS28 T2（design D5）：FileStorage 构造移入专用 std::thread +
            // oneshot 回传——锁等待期间 open future 真实让出，CLI 阶段 1
            // select! 的信号分支可达（等待可被 SIGINT/SIGTERM 即时中断）。
            OpenMode::ReadWrite => Arc::new(
                construct_file_storage(mode, path, key, lock_wait, io_failpoint.clone()).await?,
            ),
            // 只读：共享锁 + 只读句柄。与读写构造同纪律——锁先于伴生文件
            // 判定，被拒的第二打开者不触碰任何文件（MS10-T02 R2）。
            OpenMode::ReadOnly => {
                let storage =
                    construct_file_storage(mode, path, key, lock_wait, io_failpoint.clone())
                        .await?;
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
                WalWriter::open_with_io_failpoint(path, io_failpoint.clone())
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
        // T2（change 2026-10-04-external-review-gaps D2）：恢复会话收敛标志
        // 提升到本作用域——打开时 WAL 曾含事务记录的会话（含零 redo 形态）
        // 的 close()/checkpoint() SHALL 执行完整 checkpoint 截断 WAL。
        let mut pending_wal_at_open = false;
        if mode == OpenMode::ReadWrite {
            let recovery_result =
                RecoveryManager::full_recover(path, buffer_pool.clone(), table_manager.clone())
                    .await
                    .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?;
            pending_wal_at_open = recovery_result.wal_had_tx_records;

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

            // MS35（`free-page-lifecycle` R2）：恢复之后按持久目录重建本次
            // 打开的可分配页集合——恢复会重建索引树并分配新页（页数上界必须
            // 在重建时重取），且恢复期换根会让内存根与目录登记根不同（两棵
            // 树的页都算活页）。重建只读既有页，不写任何数据文件字节。
            //
            // 失败安全（R3）：任一次遍历出错 ⇒ 整次重建放弃（不注入任何页），
            // 打开仍成功，既有表照常可读。
            if let Err(e) = table_manager.rebuild_free_page_set(path).await {
                eprintln!("[free-page-rebuild] rebuild abandoned: {}", e);
            }
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
                pending_wal_at_open,
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
            schema_version: Arc::new(AtomicU64::new(0)),
            checkpoint_manager,
            isolation,
            views,
            mode,
            query_memory_limit: Arc::new(AtomicU64::new(0)),
            io_failpoint,
            implicit_transactions: Arc::new(Mutex::new(HashSet::new())),
            path: path.to_path_buf(),
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

    /// MS31-T01：库公共执行入口——返回类型化结果。
    ///
    /// 查询语句返回原生 `Value` 行与逐列元数据（列名 + 类型），DML 语句返回
    /// 受影响行数；语句错误经 `Err` 携带消息与 SQLSTATE 分类（与既有协议
    /// 错误面渲染同源）。核心层不依赖网络协议词表——协议面由协议模块的
    /// `to_response` 在边界转换。
    pub async fn execute_sql(&self, sql: &str) -> std::result::Result<SqlOutcome, SqlError> {
        crate::pipeline::execute(self, sql).await
    }

    /// MS31 Iteration 001（T7）：参数化执行入口——`sql` 内的裸 `?` 按 SQL 文本
    /// 序绑定 `params` 中的原生值。
    ///
    /// 参数值以原生 `Value` 传入（无字符串解析、无 SQL 文本拼接）：类型门 /
    /// NOT NULL / 键列门与字面量语句同源。参数个数与语句内占位符个数不符时
    /// 在计划前拒绝（点名「期望 N 实际 M」）。参数化语句不读写 plan cache
    /// （cache key 只含 SQL 文本，不含参数值）。
    pub async fn execute_with_params(
        &self,
        sql: &str,
        params: &[Value],
    ) -> std::result::Result<SqlOutcome, SqlError> {
        crate::pipeline::execute_with_params(self, sql, params).await
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

    /// MS34 Iter000（T01）：为本条库属隐式语句创建作用域守卫。
    ///
    /// 语句正常结束（提交或中止成功）时调用方显式解除；语句 future 被放弃
    /// 时守卫随 future 析构 → tx id 记入登记表 → 下一个回收触发点按既有
    /// 中止路径回收它。仍在并发执行中的语句不产生登记项，因此同一句柄上
    /// 另一条语句的回收不会中止它。
    pub(crate) fn implicit_tx_guard(&self, tx_id: u64) -> ImplicitTxGuard {
        ImplicitTxGuard::new(self.implicit_transactions.clone(), tx_id)
    }

    /// MS34 Iter000（T01）：回收被放弃的库属隐式事务。
    ///
    /// 登记项只表示「持有它的语句 future 已被放弃」——仍在执行中的语句不在
    /// 登记表内，回收不会碰它。回收走既有中止路径（写 `AbortTxn`、墓碑化
    /// 未提交版本、还原索引条目、移出活跃集、清版本条目）。表已不存在（被
    /// 放弃之后被 DROP）时先摘掉该事务在这些表上的版本条目——表的数据与
    /// 索引已随 DROP 释放，没有可还原的东西，摘条目让中止能进行到底。
    ///
    /// 中止本身失败（I/O 错误）时保留登记项，由下一个触发点重试；错误点名
    /// 返回给触发方（下一次写语句 / checkpoint）。
    pub(crate) async fn reclaim_abandoned_implicit_transactions(&self) -> Result<()> {
        let pending: Vec<u64> = {
            let mut registry = self.implicit_transactions.lock().unwrap();
            registry.drain().collect()
        };
        if pending.is_empty() {
            return Ok(());
        }
        let mut failed: Vec<u64> = Vec::new();
        let mut first_error: Option<StorageError> = None;
        for tx_id in pending {
            if let Err(e) = self.reclaim_implicit_tx(tx_id).await {
                failed.push(tx_id);
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        if !failed.is_empty() {
            self.implicit_transactions.lock().unwrap().extend(failed);
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 单个被放弃事务的回收（MS34 Iter000，T01）。
    async fn reclaim_implicit_tx(&self, tx_id: u64) -> Result<()> {
        // 已提交 / 已中止的事务不再是「被放弃的那一条」——登记项是残留，
        // 摘掉即可，不写 WAL、不碰版本。
        if !self
            .transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id)
        {
            return Ok(());
        }
        // 解析该事务触及的表（与 `Database::rollback` 同源）。
        let version_tables = self.transaction_manager.tx_version_tables(tx_id).await;
        let mut tables: HashMap<String, Arc<TableMeta>> = HashMap::new();
        let mut dropped_tables: Vec<String> = Vec::new();
        for name in version_tables.keys() {
            match self.table_manager.get_table(name).await {
                Ok(meta) => {
                    tables.insert(name.clone(), meta);
                }
                Err(_) => dropped_tables.push(name.clone()),
            }
        }
        // 表已不存在：先摘掉这些表上的版本条目，让中止能进行到底。
        if !dropped_tables.is_empty() {
            self.transaction_manager
                .discard_tx_version_tables(tx_id, &dropped_tables)
                .await;
        }
        self.transaction_manager
            .abort_by_id(tx_id, &self.buffer_pool, &tables)
            .await
            .map_err(|e| {
                StorageError::ExecutionError(format!(
                    "abandoned implicit transaction {} reclaim failed: {}",
                    tx_id, e
                ))
            })
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
        // MS34 Iter000（T01）：回收先于活跃集判定——被放弃的事务留在活跃集
        // 里会让 R3 顺延条件永远为真（自动 checkpoint 无限顺延、WAL 不再被
        // 截断）。
        self.reclaim_abandoned_implicit_transactions().await?;
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

    /// 设置查询内存上限（MS32 Iteration 003，design D4，`query-memory-budget`
    /// R1）——`0`（默认）不限制。语句入口取快照构造语句级预算，故只影响
    /// 之后开始的语句；语句结束后计数丢弃，不跨语句。
    pub fn set_query_memory_limit(&self, bytes: u64) {
        self.query_memory_limit.store(bytes, Ordering::SeqCst);
    }

    /// 语句入口的上限快照读取（pipeline 预算构造点消费）。
    pub(crate) fn query_memory_limit(&self) -> u64 {
        self.query_memory_limit.load(Ordering::SeqCst)
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
    /// A failed statement returns an error result without terminating the
    /// transaction.
    ///
    /// MS31-T01：返回类型化结果（`Ok(SqlOutcome)`）/ 类型化错误
    /// （`Err(SqlError)`）——形态与 [`Database::execute_sql`] 一致。
    pub async fn execute_in_tx(
        &self,
        sql: &str,
        tx: &Transaction,
    ) -> std::result::Result<SqlOutcome, SqlError> {
        crate::pipeline::execute_in_tx(self, sql, tx.id()).await
    }

    /// MS31 Iteration 001（T7）：显式事务内的参数化执行入口——语义与
    /// [`Database::execute_with_params`] 一致，DML 消费调用方事务的 `tx_id`
    /// （事务语义与既有 `execute_in_tx` 相同）。
    pub async fn execute_in_tx_with_params(
        &self,
        sql: &str,
        params: &[Value],
        tx: &Transaction,
    ) -> std::result::Result<SqlOutcome, SqlError> {
        crate::pipeline::execute_in_tx_with_params(self, sql, params, tx.id()).await
    }

    /// MS31 Iteration 002（T10）：预编译语句——prepare 一次、多次带参执行。
    ///
    /// prepare 期完成 parse、单语句检查、占位符编号与计划构建（计划**不写
    /// plan cache**——预编译语句自持计划），并快照 schema 版本戳。执行期
    /// 版本戳不匹配则透明 re-plan（完整重跑 parse + 编号 + 计划），结果与
    /// 直接执行一致。
    ///
    /// 语句面与 [`Database::execute_sql`] 一致：单语句、多语句拒绝、解析与
    /// 计划错误面同源。
    pub async fn prepare(&self, sql: &str) -> std::result::Result<PreparedStatement, SqlError> {
        // 版本戳取**构建计划之前**的读数（保守）：读数早于计划所见的 schema
        // 时最多多一次 re-plan；读数晚于计划所见的 schema 会漏判失效。
        let version_before_plan = self.schema_version();
        let (plan, param_count) = crate::pipeline::plan_prepared(self, sql).await?;
        Ok(PreparedStatement {
            db: self.clone(),
            sql: sql.to_string(),
            state: std::sync::Mutex::new(PreparedState {
                plan,
                param_count,
                schema_version: version_before_plan,
            }),
            exec_gate: tokio::sync::Mutex::new(()),
        })
    }

    /// Get plan cache size (for testing)
    pub fn plan_cache_len(&self) -> usize {
        self.plan_cache.len()
    }

    /// MS31 Iteration 002（T9）：当前 schema 版本戳。
    ///
    /// 与 plan cache 失效同源递增——每次 schema 变更（DDL / CTAS）执行后
    /// 由 `pipeline::bump_schema_version` 单点递增一次。预编译语句持版本
    /// 快照，执行时与本值比对判定计划是否失效。
    pub fn schema_version(&self) -> u64 {
        self.schema_version.load(Ordering::SeqCst)
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

    /// T2（change 2026-10-04-external-review-gaps D2）：打开时 WAL 是否曾含
    /// 事务记录（任一非 Checkpoint 记录）。CLI checkpoint 命令的零工作取样
    /// 消费该标志——恢复后状态（零 redo 形态）下取样谓词为真但 checkpoint
    /// 实际执行，消息 SHALL 报告实际执行而非 "no pending work"。
    /// 只读/内存形态无 checkpoint 管理器 → `false`（不参与判定）。
    pub fn wal_pending_at_open(&self) -> bool {
        self.checkpoint_manager
            .as_ref()
            .map(|m| m.pending_wal_at_open())
            .unwrap_or(false)
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
        // MS34 Iter000（T01）：回收先于 checkpoint——被放弃事务的未提交页不
        // 应随 checkpoint 落盘（WAL 证据随截断消失后无从判断其归属）。本
        // 点覆盖 `close()`、显式 checkpoint 命令与自动 checkpoint。
        self.reclaim_abandoned_implicit_transactions().await?;
        manager
            .checkpoint(|| self.transaction_manager.current_tx_id())
            .await
            .map(|_captured_lsn| ())
            .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))
    }

    /// MS33 Iteration 001（T2.1）：受控 I/O 失败开关——arm 一次后，主库下一
    /// 次同型数据页写入 / WAL fsync 在系统调用前返回携带固定标记的错误并自
    /// 清除（`injected io failure (write_page)` / `injected io failure (wal
    /// fsync)`）。共享单元经 `Arc` 克隆共享（`schema_version` 先例）；只接线
    /// 主库 `FileStorage` / `WalWriter`——`:memory:` 无文件通道，ATTACH 附加
    /// 库持有各自独立的单元，arm 不波及。
    pub fn arm_io_failure(&self, point: IoFailPoint) {
        *self.io_failpoint.lock().unwrap() = Some(point);
    }
}

/// MS31 Iteration 002（T10）：预编译语句的自持状态。
struct PreparedState {
    /// 自持计划——不经 plan cache（prepare 与执行都不读写缓存）。
    plan: PhysicalPlan,
    /// 语句内占位符个数（prepare 期确定）。
    param_count: usize,
    /// 计划构建时的 schema 版本戳快照。
    schema_version: u64,
}

/// MS31 Iteration 002（T10）：可复用预编译语句。
///
/// prepare 一次、多次带参执行：计划在 prepare 期构建一次并自持，执行时把
/// 参数物化到计划副本上。schema 变化（DDL / CTAS）后版本戳递增，执行时
/// 透明 re-plan——结果与直接执行一致。
///
/// `&self` 共享安全：每次执行持有计划副本；同一条语句的并发执行经闸门
/// 串行化——计划树内的参数节点是共享可变状态（见 [`PreparedStatement::execute`]）。
pub struct PreparedStatement {
    /// 库实例（`Database` 字段全为 `Arc`，克隆与既有 `Arc<Database>` 消费者
    /// 同型）。
    db: Database,
    /// 语句文本——re-plan 时重新解析。
    sql: String,
    /// 自持计划与版本戳快照。
    state: std::sync::Mutex<PreparedState>,
    /// 同一条预编译语句的并发执行闸门。
    exec_gate: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for PreparedStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedStatement")
            .field("sql", &self.sql)
            .finish()
    }
}

impl PreparedStatement {
    /// 带参执行一次预编译语句。
    ///
    /// 参数注入与执行在同一闸门内完成：计划树内的参数节点是**计划树共享的
    /// 可变状态**（`ExpressionRef = Arc<dyn Expression>`，`PhysicalPlan` 的
    /// `Clone` 是浅克隆——表达式子树在副本间共享），并发执行同一条预编译
    /// 语句必须串行化，否则后执行者会覆盖前执行者的参数值。不同预编译语句
    /// 各有闸门，互不阻塞。
    ///
    /// 参数个数与语句占位符个数不符时在执行前点名（文案与 SQLSTATE 与参数
    /// 化执行通道同源）。
    pub async fn execute(&self, params: &[Value]) -> std::result::Result<SqlOutcome, SqlError> {
        let _gate = self.exec_gate.lock().await;
        let (mut plan, param_count) = self.resolve_plan().await?;
        check_param_count(param_count, params)?;
        if !params.is_empty() {
            crate::pipeline::materialize_params(&mut plan, params);
        }
        crate::pipeline::execute_stage(&self.db, plan, crate::profiling::is_profiling_enabled())
            .await
    }

    /// 在既有显式事务内带参执行一次预编译语句。
    ///
    /// 事务语义与 [`Database::execute_in_tx_with_params`] 一致——DML 消费调用
    /// 方事务的 `tx_id`，快照与可见性语义不变。
    pub async fn execute_in_tx(
        &self,
        params: &[Value],
        tx: &Transaction,
    ) -> std::result::Result<SqlOutcome, SqlError> {
        let _gate = self.exec_gate.lock().await;
        let (mut plan, param_count) = self.resolve_plan().await?;
        check_param_count(param_count, params)?;
        if !params.is_empty() {
            crate::pipeline::materialize_params(&mut plan, params);
        }
        crate::pipeline::execute_stage_in_tx(&self.db, plan, tx.id()).await
    }

    /// 取本次执行要用的计划——版本戳匹配则复用自持计划；不匹配则透明
    /// re-plan（完整重跑 parse + 编号 + 计划，不只重跑计划——语句文本对应的
    /// AST 可能因 schema 变化走不同计划路径），并更新自持计划与版本戳。
    async fn resolve_plan(&self) -> std::result::Result<(PhysicalPlan, usize), SqlError> {
        {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.schema_version == self.db.schema_version() {
                return Ok((state.plan.clone(), state.param_count));
            }
        }
        // 版本戳取**重跑计划之前**的读数（保守，理由同 `Database::prepare`）。
        let version_before_replan = self.db.schema_version();
        let (plan, param_count) = crate::pipeline::plan_prepared(&self.db, &self.sql).await?;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.plan = plan.clone();
            state.param_count = param_count;
            state.schema_version = version_before_replan;
        }
        Ok((plan, param_count))
    }
}

/// 参数个数校验——与参数化执行通道同文案同 SQLSTATE。
fn check_param_count(param_count: usize, params: &[Value]) -> std::result::Result<(), SqlError> {
    if param_count != params.len() {
        return Err(crate::pipeline::parameter_count_mismatch(
            param_count,
            params.len(),
        ));
    }
    Ok(())
}
