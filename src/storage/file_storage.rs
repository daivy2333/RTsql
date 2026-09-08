use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::task::spawn_blocking;

use super::file_header::{FileHeader, HeaderError, HEADER_SIZE};
use crate::storage::{AsyncStorage, Page, PageId, Result, StorageError};

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
    page_size: usize,
    file_len: AtomicU64,
    free_pages: Mutex<Vec<u64>>,
}

impl FileStorage {
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // MS10-T02: advisory exclusive lock, held for the fd's lifetime and
        // released on drop/process exit. Taken before WAL open and recovery
        // so a rejected second opener never touches companion files.
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(StorageError::DatabaseLocked(path.display().to_string()));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(StorageError::Io(e)),
        }

        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let page_size = Page::PAGE_SIZE;

        // MS10-T03: format header — initialize (new database) or validate
        // before any page is parsed and before WAL or companion files are
        // touched (design D4). No fsync (D7): a torn header on a database
        // that has no pages yet is rejected as NotADatabase on next open.
        let page_count = if file_len == 0 {
            file.write_all_at(&FileHeader::current().encode(), 0)?;
            0
        } else if file_len < HEADER_SIZE as u64 {
            return Err(StorageError::NotADatabase(path.display().to_string()));
        } else {
            let mut buf = [0u8; HEADER_SIZE];
            file.read_exact_at(&mut buf, 0)?;
            FileHeader::decode(&buf).map_err(|e| header_rejection(e, path))?;
            let data_len = file_len - HEADER_SIZE as u64;
            if !data_len.is_multiple_of(page_size as u64) {
                return Err(StorageError::PageSizeMismatch {
                    expected: page_size,
                    actual: (data_len % page_size as u64) as usize,
                });
            }
            data_len / page_size as u64
        };

        Ok(Self {
            file: Arc::new(file),
            page_size,
            file_len: AtomicU64::new(page_count),
            free_pages: Mutex::new(Vec::new()),
        })
    }

    pub fn page_count(&self) -> u64 {
        self.file_len.load(Ordering::SeqCst)
    }

    fn read_page_blocking(
        file: Arc<std::fs::File>,
        page_id: PageId,
        page_size: usize,
    ) -> Result<Page> {
        let offset = HEADER_SIZE as u64 + page_id.to_offset(page_size);
        let mut buf = vec![0u8; page_size];
        file.as_ref().read_exact_at(&mut buf, offset)?;
        Page::from_bytes(page_id, &buf)
    }

    fn write_page_blocking(
        file: Arc<std::fs::File>,
        page_id: PageId,
        page_size: usize,
        data: Box<[u8; Page::PAGE_SIZE]>,
    ) -> Result<()> {
        let offset = HEADER_SIZE as u64 + page_id.to_offset(page_size);
        file.as_ref().write_all_at(&*data, offset)?;
        Ok(())
    }
}

#[async_trait]
impl AsyncStorage for FileStorage {
    async fn read_page(&self, page_id: PageId) -> Result<Page> {
        let file = self.file.clone();
        let page_size = self.page_size;
        spawn_blocking(move || Self::read_page_blocking(file, page_id, page_size)).await?
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
        let file = self.file.clone();
        let page_size = self.page_size;
        let data = page.data.clone();
        spawn_blocking(move || Self::write_page_blocking(file, page_id, page_size, data)).await?
    }

    async fn allocate_page(&self) -> Result<PageId> {
        // Try free list first
        if let Some(freed_id) = self.free_pages.lock().unwrap().pop() {
            return Ok(PageId(freed_id));
        }
        // Otherwise allocate new
        let page_id = self.file_len.fetch_add(1, Ordering::SeqCst);
        let offset = PageId(page_id).to_offset(self.page_size);
        let file = self.file.clone();
        let page_size = self.page_size;
        spawn_blocking(move || {
            file.as_ref()
                .set_len(HEADER_SIZE as u64 + offset + page_size as u64)?;
            Ok::<(), std::io::Error>(())
        })
        .await??;
        Ok(PageId(page_id))
    }

    async fn free_page(&self, page_id: PageId) -> Result<()> {
        self.free_pages.lock().unwrap().push(page_id.0);
        // Zero the page on disk
        let zero_page = Page::new(page_id);
        self.write_page(page_id, &zero_page).await?;
        Ok(())
    }

    async fn sync(&self) -> Result<()> {
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
}
