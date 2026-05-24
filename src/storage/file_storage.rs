use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::task::spawn_blocking;

use crate::storage::{AsyncStorage, Page, PageId, Result, StorageError};

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

        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let page_size = Page::PAGE_SIZE;

        if file_len % page_size as u64 != 0 {
            return Err(StorageError::PageSizeMismatch {
                expected: page_size,
                actual: file_len as usize % page_size,
            });
        }

        let page_count = file_len / page_size as u64;

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
        let offset = page_id.to_offset(page_size);
        let mut file_ref = file.as_ref();
        file_ref.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; page_size];
        file_ref.read_exact(&mut buf)?;
        Page::from_bytes(page_id, &buf)
    }

    fn write_page_blocking(
        file: Arc<std::fs::File>,
        page_id: PageId,
        page_size: usize,
        data: Box<[u8; Page::PAGE_SIZE]>,
    ) -> Result<()> {
        let offset = page_id.to_offset(page_size);
        let mut file_ref = file.as_ref();
        file_ref.seek(SeekFrom::Start(offset))?;
        file_ref.write_all(&*data)?;
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
            file.as_ref().set_len(offset + page_size as u64)?;
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
}
