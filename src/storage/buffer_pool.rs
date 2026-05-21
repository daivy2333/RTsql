use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::storage::{
    page_frame::{PageFrame, PageGuard},
    AsyncStorage, Page, PageId, Result, RowId, StorageError,
};
use crate::transaction::Snapshot;

pub struct BufferPool {
    pages: RwLock<HashMap<PageId, Arc<std::sync::Mutex<PageFrame>>>>,
    clock_hand: RwLock<Vec<PageId>>,
    capacity: usize,
    storage: Arc<dyn AsyncStorage>,
}

impl BufferPool {
    pub fn new(capacity: usize, storage: Arc<dyn AsyncStorage>) -> Result<Self> {
        if capacity == 0 {
            return Err(StorageError::InvalidCapacity(capacity));
        }

        Ok(Self {
            pages: RwLock::new(HashMap::new()),
            clock_hand: RwLock::new(Vec::new()),
            capacity,
            storage,
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get storage reference for page allocation
    pub fn storage(&self) -> &Arc<dyn AsyncStorage> {
        &self.storage
    }

    pub async fn get_page(&self, page_id: PageId) -> Result<PageGuard> {
        // 1. 读锁检查缓存
        {
            let pages = self.pages.read().await;
            if let Some(frame) = pages.get(&page_id) {
                return Ok(PageGuard::new(frame.clone()));
            }
        }

        // 2. 写锁加载页
        let mut pages = self.pages.write().await;

        // Double check
        if let Some(frame) = pages.get(&page_id) {
            return Ok(PageGuard::new(frame.clone()));
        }

        // 3. 缓存满则淘汰
        if pages.len() >= self.capacity {
            self.evict_one(&mut pages).await?;
        }

        // 4. 从存储加载页
        let page = self.storage.read_page(page_id).await?;
        let frame = Arc::new(std::sync::Mutex::new(PageFrame::new(page)));

        pages.insert(page_id, frame.clone());
        self.clock_hand.write().await.push(page_id);

        Ok(PageGuard::new(frame))
    }

    async fn evict_one(
        &self,
        pages: &mut HashMap<PageId, Arc<std::sync::Mutex<PageFrame>>>,
    ) -> Result<()> {
        let mut clock_hand = self.clock_hand.write().await;
        let mut attempts = 0;
        let max_attempts = clock_hand.len() * 2;

        while attempts < max_attempts {
            if clock_hand.is_empty() {
                return Err(StorageError::BufferPoolFull);
            }

            let candidate_id = clock_hand.remove(0);
            attempts += 1;

            let frame = match pages.get(&candidate_id) {
                Some(f) => f.clone(),
                None => continue,
            };

            let (dirty, page_copy): (bool, Option<Page>) = {
                let mut frame_guard = frame.lock().unwrap();

                if frame_guard.ref_count > 0 {
                    clock_hand.push(candidate_id);
                    (false, None)
                } else if frame_guard.clock_bit {
                    frame_guard.clock_bit = false;
                    clock_hand.push(candidate_id);
                    (false, None)
                } else {
                    let is_dirty = frame_guard.dirty;
                    let page = frame_guard.page.clone();
                    (is_dirty, Some(page))
                }
            };

            let Some(page_copy) = page_copy else {
                continue;
            };

            if dirty {
                self.storage.write_page(candidate_id, &page_copy).await?;
            }

            pages.remove(&candidate_id);
            return Ok(());
        }

        Err(StorageError::BufferPoolFull)
    }

    /// Flush all dirty pages to storage
    pub async fn flush_all(&self) -> Result<()> {
        let pages = self.pages.read().await;

        for (page_id, frame) in pages.iter() {
            let mut frame_guard = frame.lock().unwrap();

            if frame_guard.dirty {
                let page = frame_guard.page.clone();
                frame_guard.dirty = false;
                drop(frame_guard);
                self.storage.write_page(*page_id, &page).await?;
            }
        }

        Ok(())
    }

    /// Traverse version chain to find first visible version (M10)
    ///
    /// Returns: visible tuple bytes, or None if all versions invisible
    pub async fn find_visible_version(
        &self,
        row_id: RowId,
        snapshot: &Snapshot,
    ) -> Result<Option<Vec<u8>>> {
        let mut current_row_id = Some(row_id);

        while let Some(current) = current_row_id {
            // Read current version from data page
            let (version_header, tuple_bytes) =
                crate::storage::read_tuple_from_data_page(self, current).await?;

            // Check visibility
            let create_tx = version_header.create_tx_id();
            let commit_tx = version_header.commit_tx_id();

            let visible = snapshot.is_visible(create_tx, commit_tx)
                || snapshot.is_visible_self(create_tx, commit_tx);

            if visible {
                return Ok(Some(tuple_bytes));
            }

            // Not visible, follow next_version pointer
            current_row_id = version_header.next_version();
        }

        // All versions invisible
        Ok(None)
    }
}
