use std::collections::HashSet;
use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rand::RngCore;
use tokio::task::spawn_blocking;

use super::crypto::{
    derive_key, PageCipher, DEFAULT_KDF_M_KIB, DEFAULT_KDF_P, DEFAULT_KDF_T,
    ENCRYPTED_PAGE_RECORD_SIZE,
};
use super::file_header::{FileHeader, HeaderError, FLAG_ENCRYPTED, HEADER_SIZE};
use crate::storage::{AsyncStorage, Page, PageId, Result, StorageError};

/// 一次已释放、尚未提升的页。锚点页号在释放完成时写入；空列表表示这次
/// 释放没有锚点写，提升时不看失败集合。
struct PendingRelease {
    page: u64,
    completion: Option<u64>,
    anchors: Vec<u64>,
}

/// Map a module-private header classification to a `StorageError` with the
/// path attached (MS10-T03 design D5).
fn header_rejection(err: HeaderError, path: &Path) -> StorageError {
    let p = path.display().to_string();
    match err {
        HeaderError::BadMagic | HeaderError::ZeroVersion => StorageError::NotADatabase(p),
        HeaderError::NewerVersion(found) => {
            StorageError::NewerFileVersion(format!("file version {found}: {p}"))
        }
        HeaderError::UnknownFlags(bits) => {
            StorageError::IncompatibleHeader(format!("unknown feature flags: {bits:#x} ({p})"))
        }
        HeaderError::PageMismatch(found) => StorageError::IncompatibleHeader(format!(
            "header page size {found}, expected {} ({p})",
            Page::PAGE_SIZE
        )),
        HeaderError::ReservedNonZero(region) => {
            StorageError::IncompatibleHeader(format!("{region} region must be zero ({p})"))
        }
    }
}

pub struct FileStorage {
    file: Arc<std::fs::File>,
    /// 逻辑页大小（恒 4096，加密不改变页格式）
    page_size: usize,
    /// 单页磁盘记录长度：明文库 = page_size，加密库 = 4124
    record_size: usize,
    /// 加密库持有的页加密器；明文库恒 None（模式在 open 期一次定型）
    cipher: Option<Arc<PageCipher>>,
    file_len: AtomicU64,
    free_pages: Mutex<Vec<u64>>,
    /// MS35（`free-page-lifecycle` R3）：待释放集合——已释放但尚未在崩溃
    /// 边界上成立的页。完成编号为 `None` 时锚点写尚未发出，任何时刻都不得
    /// 被提升。完成时记下这次调用的锚点页号。
    ///
    /// 释放完成编号单调递增（`release_id`）；checkpoint 在刷盘前取样该
    /// 编号，刷盘成功才把完成编号小于样本的项提升进 `free_pages`，刷盘失败
    /// 则丢弃（不提升）。
    pending_frees: Mutex<Vec<PendingRelease>>,
    /// MS35（`free-page-lifecycle` R3）：已完成的释放计数（单调递增）。
    /// checkpoint 在刷盘前取样它。写盘失败不再消耗这个编号。
    release_id: AtomicU64,
    /// 写盘失败的页号。失败的 `write_page` 放入该页，该页其后一次成功写入
    /// 后移出。只在内存，不进页字节，也不进文件头。
    failed_pages: Mutex<HashSet<u64>>,
    /// MS26 T1：只读实例（`open_readonly*`）——写方法显式点名拒绝，
    /// `sync` 为 no-op。只读打开的正常路径不可达这些守卫（写语句在
    /// pipeline 层先被拒，见 `pipeline::readonly_rejection`），此处为
    /// 结构性兜底。
    readonly: bool,
    /// MS33 Iter001（T2.1）：受控 I/O 失败开关单元——`Database` 打开时分发
    /// 同一单元（`open_with_lock_io_failpoint`），未接线时保持空开关。
    io_failpoint: IoFailPointSlot,
}

/// 只读实例写方法的统一拒绝文案（`readonly-open-mode` R3）。
fn read_only_violation(op: &str) -> StorageError {
    StorageError::Internal(format!("cannot {op} on a read-only storage instance"))
}

/// MS28 design D1：锁等待预算常量。
///
/// `LOCK_VISIBILITY_FLOOR` 是平台 flock 释放可见性窗口的吸收机制
/// （Cycle 001 replan design D10 的既有正确性机制，恒存、非用户等待
/// 策略的一部分）；`DEFAULT_LOCK_WAIT` 是 busy-timeout 通道的默认预算
/// （proposal 默认假设 1）。
const LOCK_VISIBILITY_FLOOR: Duration = Duration::from_millis(8);
const DEFAULT_LOCK_WAIT: Duration = Duration::from_millis(5000);

/// MS33 Iteration 001（T2.1）：受控 I/O 失败注入点。armed 值由同型注入点
/// 消费——数据页写入与 WAL fsync 各自独立判定，互不干扰。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoFailPoint {
    /// 下一次数据文件页写入在任何字节落盘之前失败。
    DataWrite,
    /// 下一次 WAL fsync 在 `sync_all` 之前失败。
    WalFsync,
}

/// 共享失败开关单元：`Database` 持有并分发给主库 `FileStorage` 与
/// `WalWriter`，armed 值被同型注入点一次性消费（自清除）。
pub(crate) type IoFailPointSlot = Arc<Mutex<Option<IoFailPoint>>>;

/// 注入点消费：仅当 armed 值与 `point` 同型时取出并返回 true，否则保持
/// 原样返回 false。
pub(crate) fn consume_io_failpoint(slot: &IoFailPointSlot, point: IoFailPoint) -> bool {
    let mut guard = slot.lock().unwrap();
    if *guard == Some(point) {
        *guard = None;
        true
    } else {
        false
    }
}

/// MS28 T1（`database-file-lock` R1 MODIFIED）：advisory 锁获取的 deadline
/// 循环——首次尝试失败后以 2ms 间隔重试，直至 `Instant::now()` 越过
/// `now + max(wait, LOCK_VISIBILITY_FLOOR)`。`wait` 为用户等待预算
/// （`None` 由调用方解析为 `DEFAULT_LOCK_WAIT`）；`Some(0)` 经 `max` 自然
/// 落到 ~8ms 下限窗口（MS28 前语义的显式表达）。预算耗尽后仍按
/// `DatabaseLocked` 拒绝，错误面不变；`Error(e)` 为真实 I/O 失败，不重试
/// 直传。
fn try_lock_with_backoff(
    file: &std::fs::File,
    path: &Path,
    lock: impl Fn(&std::fs::File) -> std::result::Result<(), std::fs::TryLockError>,
    wait: Duration,
) -> Result<()> {
    let deadline = Instant::now() + wait.max(LOCK_VISIBILITY_FLOOR);
    loop {
        match lock(file) {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(StorageError::DatabaseLocked(path.display().to_string()));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(StorageError::Io(e)),
        }
    }
}

impl FileStorage {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_key(path, None)
    }

    /// 打开数据库文件；`key` 为 `Some` 时按加密库打开（0 字节文件则创建
    /// 加密新库）。打开顺序守卫：锁 → 头校验 → 密钥检查/KDF → 页长度校验
    /// （加密无密钥 / 明文带密钥在页解析与 WAL 触碰前拒绝）。
    pub fn open_with_key(path: &Path, key: Option<&str>) -> Result<Self> {
        Self::open_with_lock(path, key, None)
    }

    /// MS28 T1：锁等待预算可配置的读写打开核心（`database-file-lock` R1
    /// MODIFIED）。`lock_wait` 为等待预算——`None` → `DEFAULT_LOCK_WAIT`
    /// （5000ms，MS28 目标默认语义）；`Some(0)` ＝ 不额外等待，仅
    /// `LOCK_VISIBILITY_FLOOR`（~8ms）下限窗口生效（MS28 前语义的显式
    /// 表达）；预算耗尽维持 `DatabaseLocked` 既有错误面。构造语义与打开
    /// 顺序守卫（锁 → 头校验 → 密钥/KDF → 页长度校验）同 `open_with_key`
    /// 逐字节一致，仅锁窗口参数化；既有 open 家族委托本函数传 `None`。
    pub fn open_with_lock(
        path: &Path,
        key: Option<&str>,
        lock_wait: Option<Duration>,
    ) -> Result<Self> {
        Self::open_with_lock_io_failpoint(path, key, lock_wait, Arc::new(Mutex::new(None)))
    }

    /// MS33 Iter001（T2.1）：内部构造变体——语义与 [`Self::open_with_lock`]
    /// 逐字节一致，仅注入 `Database` 持有的共享失败开关单元。
    pub(crate) fn open_with_lock_io_failpoint(
        path: &Path,
        key: Option<&str>,
        lock_wait: Option<Duration>,
        io_failpoint: IoFailPointSlot,
    ) -> Result<Self> {
        let wait = lock_wait.unwrap_or(DEFAULT_LOCK_WAIT);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // MS10-T02: advisory exclusive lock, held for the fd's lifetime and
        // released on drop/process exit. Taken before WAL open and recovery
        // so a rejected second opener never touches companion files.
        try_lock_with_backoff(&file, path, |f| f.try_lock(), wait)?;

        let file_len = file.metadata()?.len();
        let page_size = Page::PAGE_SIZE;

        // MS10-T03: format header — initialize (new database) or validate
        // before any page is parsed and before WAL or companion files are
        // touched (design D4). No fsync (D7): a torn header on a database
        // that has no pages yet is rejected as NotADatabase on next open.
        let (cipher, record_size, page_count) = if file_len == 0 {
            let (cipher, record_size) = Self::initialize_header(&file, key)?;
            (cipher, record_size, 0)
        } else {
            Self::validate_header(&file, path, key, file_len)?
        };

        Ok(Self {
            file: Arc::new(file),
            page_size,
            record_size,
            cipher,
            file_len: AtomicU64::new(page_count),
            free_pages: Mutex::new(Vec::new()),
            pending_frees: Mutex::new(Vec::new()),
            release_id: AtomicU64::new(0),
            failed_pages: Mutex::new(HashSet::new()),
            readonly: false,
            io_failpoint,
        })
    }

    /// MS26 T1：只读打开（`readonly-open-mode` R1）——advisory 共享锁，
    /// 文件句柄只读且不创建，缺失文件诚实报错。锁冲突与独占构造同型
    /// （`WouldBlock` 经有界重试后仍 `DatabaseLocked`，design D10）。头校验 /
    /// 密钥 / KDF / 页长度语义与读写构造共用同一实现，逐字节一致。
    pub fn open_readonly(path: &Path) -> Result<Self> {
        Self::open_readonly_with_key(path, None)
    }

    /// MS26 T1：只读打开的加密库变体（`key` 为 `Some` 时按加密库解密读取，
    /// 错误密钥沿既有 `InvalidKey` 拒绝）。
    pub fn open_readonly_with_key(path: &Path, key: Option<&str>) -> Result<Self> {
        Self::open_readonly_with_lock(path, key, None)
    }

    /// MS28 T1：锁等待预算可配置的只读打开核心——advisory 共享锁，预算
    /// 语义与错误面同 `open_with_lock` 逐字节一致（`database-file-lock` R1
    /// MODIFIED 共享锁同款；既有 `open_readonly*` 委托本函数传 `None`）。
    pub fn open_readonly_with_lock(
        path: &Path,
        key: Option<&str>,
        lock_wait: Option<Duration>,
    ) -> Result<Self> {
        let wait = lock_wait.unwrap_or(DEFAULT_LOCK_WAIT);
        // 只读句柄：无 `write`、无 `create` —— 缺失文件由 open 直接报错
        // （点名路径，语义同「该路径尚不是 RTsql 数据库」）。
        let file = match OpenOptions::new().read(true).open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StorageError::NotADatabase(path.display().to_string()));
            }
            Err(e) => return Err(StorageError::Io(e)),
        };

        try_lock_with_backoff(&file, path, |f| f.try_lock_shared(), wait)?;

        let file_len = file.metadata()?.len();
        // 0 字节文件尚不是数据库（读写构造此时会写头创建新库，只读构造
        // 无写权限）→ 与截断头同类诚实拒绝。
        let (cipher, record_size, page_count) = if file_len == 0 {
            return Err(StorageError::NotADatabase(path.display().to_string()));
        } else {
            Self::validate_header(&file, path, key, file_len)?
        };

        Ok(Self {
            file: Arc::new(file),
            page_size: Page::PAGE_SIZE,
            record_size,
            cipher,
            file_len: AtomicU64::new(page_count),
            free_pages: Mutex::new(Vec::new()),
            pending_frees: Mutex::new(Vec::new()),
            release_id: AtomicU64::new(0),
            failed_pages: Mutex::new(HashSet::new()),
            readonly: true,
            io_failpoint: Arc::new(Mutex::new(None)),
        })
    }

    /// MS10-T03：0 字节新库的格式头初始化（明文/加密两态），返回
    /// （页加密器, 单页磁盘记录长度）。无 fsync（D7）。
    fn initialize_header(
        file: &std::fs::File,
        key: Option<&str>,
    ) -> Result<(Option<Arc<PageCipher>>, usize)> {
        let page_size = Page::PAGE_SIZE;
        let header = match key {
            Some(_) => {
                let mut salt = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut salt);
                FileHeader::current_encrypted(salt, DEFAULT_KDF_M_KIB, DEFAULT_KDF_T, DEFAULT_KDF_P)
            }
            None => FileHeader::current(),
        };
        file.write_all_at(&header.encode(), 0)?;
        let cipher = key.map(|password| {
            Arc::new(PageCipher::new(derive_key(
                password.as_bytes(),
                &header.salt,
                DEFAULT_KDF_M_KIB,
                DEFAULT_KDF_T,
                DEFAULT_KDF_P,
            )))
        });
        let record_size = if cipher.is_some() {
            ENCRYPTED_PAGE_RECORD_SIZE
        } else {
            page_size
        };
        Ok((cipher, record_size))
    }

    /// MS10-T03：既有库的格式头校验（明文/加密两态分类、KDF 参数读取、
    /// 密钥一致性、页长度整除），返回（页加密器, 单页磁盘记录长度, 页数）。
    /// 读写与只读构造共用（MS26 T1）——两条构造的头校验行为逐字节一致。
    fn validate_header(
        file: &std::fs::File,
        path: &Path,
        key: Option<&str>,
        file_len: u64,
    ) -> Result<(Option<Arc<PageCipher>>, usize, u64)> {
        let page_size = Page::PAGE_SIZE;
        if file_len < HEADER_SIZE as u64 {
            return Err(StorageError::NotADatabase(path.display().to_string()));
        }
        let mut buf = [0u8; HEADER_SIZE];
        file.read_exact_at(&mut buf, 0)?;
        let header = FileHeader::decode(&buf).map_err(|e| header_rejection(e, path))?;
        let encrypted = header.flags & FLAG_ENCRYPTED != 0;
        let cipher = match (encrypted, key) {
            (true, None) => {
                return Err(StorageError::InvalidKey(format!(
                    "database is encrypted, supply --key or RTSQL_KEY: {}",
                    path.display()
                )));
            }
            (false, Some(_)) => {
                return Err(StorageError::InvalidKey(format!(
                    "database is not encrypted: {}",
                    path.display()
                )));
            }
            (true, Some(password)) => {
                let (m_kib, t, p) = header.kdf_params();
                Some(Arc::new(PageCipher::new(derive_key(
                    password.as_bytes(),
                    &header.salt,
                    m_kib,
                    t,
                    p,
                ))))
            }
            (false, None) => None,
        };
        let record_size = if encrypted {
            ENCRYPTED_PAGE_RECORD_SIZE
        } else {
            page_size
        };
        let data_len = file_len - HEADER_SIZE as u64;
        if !data_len.is_multiple_of(record_size as u64) {
            return Err(StorageError::PageSizeMismatch {
                expected: record_size,
                actual: (data_len % record_size as u64) as usize,
            });
        }
        Ok((cipher, record_size, data_len / record_size as u64))
    }

    pub fn page_count(&self) -> u64 {
        self.file_len.load(Ordering::SeqCst)
    }

    fn read_page_blocking(
        file: Arc<std::fs::File>,
        cipher: Option<Arc<PageCipher>>,
        page_id: PageId,
        page_size: usize,
        record_size: usize,
    ) -> Result<Page> {
        let offset = HEADER_SIZE as u64 + page_id.to_offset(record_size);
        match cipher {
            None => {
                let mut buf = vec![0u8; page_size];
                file.as_ref().read_exact_at(&mut buf, offset)?;
                Page::from_bytes(page_id, &buf)
            }
            Some(cipher) => {
                let mut buf = vec![0u8; record_size];
                file.as_ref().read_exact_at(&mut buf, offset)?;
                let record: &[u8; ENCRYPTED_PAGE_RECORD_SIZE] =
                    (&buf[..]).try_into().expect("record length is record_size");
                let plain = cipher.decrypt_page(page_id, record).map_err(|_| {
                    StorageError::InvalidKey(format!(
                        "decryption failed (wrong key or corrupted page): page {}",
                        page_id.0
                    ))
                })?;
                Page::from_bytes(page_id, &plain)
            }
        }
    }

    fn write_page_blocking(
        file: Arc<std::fs::File>,
        cipher: Option<Arc<PageCipher>>,
        page_id: PageId,
        record_size: usize,
        data: Box<[u8; Page::PAGE_SIZE]>,
    ) -> Result<()> {
        let offset = HEADER_SIZE as u64 + page_id.to_offset(record_size);
        match cipher {
            None => file.as_ref().write_all_at(&*data, offset)?,
            Some(cipher) => {
                let record = cipher.encrypt_page(page_id, data.as_ref());
                file.as_ref().write_all_at(&record, offset)?;
            }
        }
        Ok(())
    }

    fn mark_write_failed(&self, page_id: PageId) {
        self.failed_pages.lock().unwrap().insert(page_id.0);
    }

    fn mark_write_succeeded(&self, page_id: PageId) {
        self.failed_pages.lock().unwrap().remove(&page_id.0);
    }
}

#[async_trait]
impl AsyncStorage for FileStorage {
    async fn read_page(&self, page_id: PageId) -> Result<Page> {
        let file = self.file.clone();
        let cipher = self.cipher.clone();
        let page_size = self.page_size;
        let record_size = self.record_size;
        spawn_blocking(move || {
            Self::read_page_blocking(file, cipher, page_id, page_size, record_size)
        })
        .await?
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
        if self.readonly {
            return Err(read_only_violation("write a page"));
        }
        // MS33 Iter001（T2.1）：armed 的 DataWrite 在任何字节落盘（含
        // spawn_blocking 内的 write_all_at）之前消费——注入失败不产生半页、
        // 不触碰文件。
        if consume_io_failpoint(&self.io_failpoint, IoFailPoint::DataWrite) {
            self.mark_write_failed(page_id);
            return Err(StorageError::Io(std::io::Error::other(
                "injected io failure (write_page)",
            )));
        }
        let file = self.file.clone();
        let cipher = self.cipher.clone();
        let record_size = self.record_size;
        let data = page.data.clone();
        match spawn_blocking(move || {
            Self::write_page_blocking(file, cipher, page_id, record_size, data)
        })
        .await?
        {
            Ok(()) => {
                self.mark_write_succeeded(page_id);
                Ok(())
            }
            Err(e) => {
                self.mark_write_failed(page_id);
                Err(e)
            }
        }
    }

    async fn allocate_page(&self) -> Result<PageId> {
        if self.readonly {
            return Err(read_only_violation("allocate a page"));
        }
        // Try free list first.
        //
        // MS33 D1：清零发生在「弹出空闲页」这一刻，不在释放时。释放后若
        // 进程在 checkpoint 前结束，磁盘页仍留着原内容——catalog 仍登记
        // 该对象时重开可读回；同会话再次分配到该页号时先写零页，读不到
        // 已释放页上的旧行。
        //
        // 弹栈与写零分两步：空闲列表的锁在 await 之前释放，不跨 await
        // 持有 std MutexGuard。
        let freed_id = self.free_pages.lock().unwrap().pop();
        if let Some(freed_id) = freed_id {
            let page_id = PageId(freed_id);
            let zero_page = Page::new(page_id);
            if let Err(e) = self.write_page(page_id, &zero_page).await {
                // 写零失败：页号回到空闲列表，不把未清零的页交给调用者。
                self.free_pages.lock().unwrap().push(freed_id);
                return Err(e);
            }
            return Ok(page_id);
        }
        // Otherwise allocate new
        let page_id = self.file_len.fetch_add(1, Ordering::SeqCst);
        let offset = HEADER_SIZE as u64 + PageId(page_id).to_offset(self.record_size);
        let file = self.file.clone();
        let record_size = self.record_size;
        match self.cipher.clone() {
            Some(cipher) => {
                // 加密库：直接写加密零页。set_len 只留裸零字节，无法通过
                // GCM 认证——明文语义"新分配页读到全零空页"由密文零页承载
                // （allocate→首读先于首写的路径依赖该语义）。
                spawn_blocking(move || {
                    let zero_page = [0u8; Page::PAGE_SIZE];
                    let record = cipher.encrypt_page(PageId(page_id), &zero_page);
                    file.as_ref().write_all_at(&record, offset)?;
                    Ok::<(), std::io::Error>(())
                })
                .await??;
            }
            None => {
                let offset = PageId(page_id).to_offset(record_size);
                spawn_blocking(move || {
                    file.as_ref()
                        .set_len(HEADER_SIZE as u64 + offset + record_size as u64)?;
                    Ok::<(), std::io::Error>(())
                })
                .await??;
            }
        }
        Ok(PageId(page_id))
    }

    async fn free_page(&self, page_id: PageId) -> Result<()> {
        if self.readonly {
            return Err(read_only_violation("free a page"));
        }
        // MS35（`free-page-lifecycle` R3）：释放先进入待释放集合，**不**直接
        // 进可分配集合——该页在「释放在崩溃边界上成立」之前不得被发放。
        // 释放完成的标记（`complete_free_release`）只覆盖本次释放的页。
        //
        // MS33 D1：释放时不写零页。checkpoint 前崩溃时磁盘页保留原内容，
        // 清零移到 `allocate_page` 弹出该页号时。
        self.pending_frees.lock().unwrap().push(PendingRelease {
            page: page_id.0,
            completion: None,
            anchors: Vec::new(),
        });
        Ok(())
    }

    async fn sync(&self) -> Result<()> {
        if self.readonly {
            return Ok(());
        }
        let file = self.file.clone();
        spawn_blocking(move || {
            file.as_ref().sync_all()?;
            Ok::<(), StorageError>(())
        })
        .await?
    }

    fn page_count(&self) -> u64 {
        self.file_len.load(Ordering::SeqCst)
    }

    /// MS35 (`free-page-lifecycle` R2): install the rebuilt allocatable page
    /// set. The set replaces the current free list (rebuild semantics): the
    /// session's allocatable pages are derived from the persisted directory,
    /// so the pages released while opening (e.g. the old index tree pages a
    /// recovery re-rooted away) do not become allocatable while the
    /// persisted directory still registers them.
    fn inject_free_pages(&self, pages: Vec<u64>) {
        let mut free = self.free_pages.lock().unwrap();
        free.clear();
        free.extend(pages);
    }

    /// MS35（`free-page-lifecycle` R3）：取样释放完成编号——checkpoint 在
    /// 刷盘**之前**调用它，随后按样本提升或丢弃待释放项。
    fn release_completion_limit(&self) -> u64 {
        self.release_id.load(Ordering::SeqCst)
    }

    /// MS35（`free-page-lifecycle` R3）：释放操作完成——本次释放的锚点写
    /// 已经发出。只有 `pages` 里的页被标记为完成：未完成的释放任何时刻都
    /// 不得被提升（并发释放互不覆盖）。
    fn complete_free_release(&self, pages: &[PageId], anchors: &[PageId]) {
        if pages.is_empty() {
            return;
        }
        let id = self.release_id.fetch_add(1, Ordering::SeqCst);
        let anchor_ids: Vec<u64> = anchors.iter().map(|page| page.0).collect();
        let mut pending = self.pending_frees.lock().unwrap();
        for entry in pending.iter_mut() {
            if entry.completion.is_none() && pages.iter().any(|page| page.0 == entry.page) {
                entry.completion = Some(id);
                entry.anchors = anchor_ids.clone();
            }
        }
    }

    /// MS35（`free-page-lifecycle` R3）：覆盖样本的刷盘成功 ⇒ 该批释放在
    /// 崩溃边界上成立 ⇒ 其页进入可分配集合。
    ///
    /// 释放的可分配判定以锚点页是否仍在失败集合内为准。完成编号仍须
    /// `id < limit`。锚点列表为空时，不因失败集合拒绝。任一锚点仍失败则
    /// 该项留在待释放集合，等该页成功写入后再判。
    fn promote_free_releases(&self, limit: u64) {
        let failed = self.failed_pages.lock().unwrap();
        let mut pending = self.pending_frees.lock().unwrap();
        let mut free = self.free_pages.lock().unwrap();
        let mut remaining = Vec::with_capacity(pending.len());
        for entry in pending.drain(..) {
            match entry.completion {
                Some(id) if id < limit => {
                    let anchor_failed = entry.anchors.iter().any(|page| failed.contains(page));
                    if anchor_failed {
                        remaining.push(entry);
                    } else {
                        free.push(entry.page);
                    }
                }
                _ => remaining.push(entry),
            }
        }
        *pending = remaining;
    }

    /// MS35（`free-page-lifecycle` R3）：覆盖样本的刷盘失败 ⇒ 该批释放被
    /// 丢弃（不提升）——锚点写可能永远没有落盘，磁盘上这些页仍归旧主人，
    /// 重开时按持久目录重建会把它们判成空闲。
    fn discard_free_releases(&self, limit: u64) {
        let mut pending = self.pending_frees.lock().unwrap();
        pending.retain(|entry| match entry.completion {
            Some(id) => id >= limit,
            None => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FLAG_ENCRYPTED;
    use tempfile::tempdir;

    fn page_with(fill: u8) -> Page {
        let mut page = Page::new(PageId(0));
        page.data.fill(fill);
        page
    }

    /// (7) 明文库 `open` 与 `open_with_key(None)` 行为等价：字节级一致 + 往返
    #[tokio::test]
    async fn open_none_equivalent_to_open_for_plaintext() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.db");
        let b = dir.path().join("b.db");
        for path in [&a, &b] {
            let storage = if path == &a {
                FileStorage::open(path).unwrap()
            } else {
                FileStorage::open_with_key(path, None).unwrap()
            };
            let page = page_with(0xAB);
            storage.write_page(PageId(0), &page).await.unwrap();
            let read_back = storage.read_page(PageId(0)).await.unwrap();
            assert_eq!(read_back.data.as_ref(), page.data.as_ref());
            drop(storage);
        }
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    }

    /// (1) 加密库落盘形态：记录步长 4124、首记录非明文、头 flags 为加密位
    #[tokio::test]
    async fn encrypted_db_on_disk_shape() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
            let page = page_with(0xA5);
            storage.write_page(PageId(0), &page).await.unwrap();
            storage.sync().await.unwrap();
        }
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(
            raw.len() as u64,
            HEADER_SIZE as u64 + ENCRYPTED_PAGE_RECORD_SIZE as u64
        );
        assert_ne!(
            &raw[HEADER_SIZE..HEADER_SIZE + Page::PAGE_SIZE],
            page_with(0xA5).data.as_ref()
        );
        assert_eq!(
            u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            FLAG_ENCRYPTED
        );
    }

    /// (2) 错误密钥：open 结构成功，首读页解密认证失败 `InvalidKey`
    #[tokio::test]
    async fn wrong_key_fails_on_first_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("right")).unwrap();
            storage.write_page(PageId(0), &page_with(1)).await.unwrap();
        }
        let storage = FileStorage::open_with_key(&path, Some("wrong")).unwrap();
        let err = storage.read_page(PageId(0)).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (3) 明文库带钥：open 期拒绝
    #[tokio::test]
    async fn plaintext_with_key_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plain.db");
        {
            let storage = FileStorage::open(&path).unwrap();
            storage.write_page(PageId(0), &page_with(2)).await.unwrap();
        }
        let err = match FileStorage::open_with_key(&path, Some("pw")) {
            Err(e) => e,
            Ok(_) => panic!("expected InvalidKey for plaintext opened with key"),
        };
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (4) 加密库无钥：open 期拒绝
    #[tokio::test]
    async fn encrypted_without_key_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
            storage.write_page(PageId(0), &page_with(3)).await.unwrap();
        }
        let err = match FileStorage::open_with_key(&path, None) {
            Err(e) => e,
            Ok(_) => panic!("expected InvalidKey for encrypted opened without key"),
        };
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (5) 密文单字节篡改：首读认证失败
    #[tokio::test]
    async fn tampered_ciphertext_detected_on_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
            storage.write_page(PageId(0), &page_with(4)).await.unwrap();
        }
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        std::fs::write(&path, raw).unwrap();
        let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
        let err = storage.read_page(PageId(0)).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (6) 加密库 allocate/页数按 4124 步长
    #[tokio::test]
    async fn encrypted_allocate_uses_4124_stride() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
        let id = storage.allocate_page().await.unwrap();
        assert_eq!(id, PageId(0));
        assert_eq!(storage.page_count(), 1);
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(len, HEADER_SIZE as u64 + ENCRYPTED_PAGE_RECORD_SIZE as u64);
    }

    /// (8) 加密库页长度校验：数据区长度非 4124 整除 → PageSizeMismatch
    /// （expected 携带 4124；weak KDF 参数仅为测试速度，语义不受影响）
    #[test]
    fn encrypted_db_rejects_non_record_aligned_length() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        let header = FileHeader::current_encrypted([7u8; 32], 1024, 1, 1);
        let mut raw = header.encode().to_vec();
        raw.extend_from_slice(&vec![0u8; ENCRYPTED_PAGE_RECORD_SIZE + 100]);
        std::fs::write(&path, raw).unwrap();
        let err = match FileStorage::open_with_key(&path, Some("pw")) {
            Err(e) => e,
            Ok(_) => panic!("expected PageSizeMismatch for non-4124-aligned data region"),
        };
        assert!(
            matches!(
                err,
                StorageError::PageSizeMismatch {
                    expected: ENCRYPTED_PAGE_RECORD_SIZE,
                    ..
                }
            ),
            "got: {err:?}"
        );
    }
}
