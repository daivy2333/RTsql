//! MS26 T3（`in-memory-database` R1/R2，design D5）：`:memory:` 内存存储后端。
//!
//! 页数据的全部生命周期都在进程内存内：读为页拷贝、写为页拷贝、分配优先消费
//! free list、同步为 no-op。**无格式头**——新实例 `page_count() == 0`，
//! `TableManager::new` 据此走 catalog bootstrap（分配页 0/1）。
//!
//! `page_count()` 语义镜像 `FileStorage` 的分配计数（单调已分配数，free 不收缩）：
//! 唯一生产消费方是 bootstrap 判定 `== 0`，单调计数保证「曾分配过页」的存储不会
//! 被误判为空库重新 bootstrap。
//!
//! free 页的零化语义与 `FileStorage::free_page` 同型——回收即写零页，使
//! 「allocate → 首读先于首写」读到全零空页的既有依赖在内存后端同样成立；
//! 未分配页的读按 `InvalidPageId` 点名拒绝（镜像文件库的越界读报错面，
//! 正常路径不可达：BufferPool 只驱逐已分配页）。

use crate::storage::{AsyncStorage, Page, PageId, Result, StorageError};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// 内存页存储。两个 `Mutex` 的锁作用域互不嵌套（先 free list 后页表，各自
/// 独立临界区），故无锁序约束。
pub struct MemoryStorage {
    /// 已分配页：页号 → 页内容。
    pages: Mutex<HashMap<u64, Box<[u8; Page::PAGE_SIZE]>>>,
    /// 已回收待复用的页号（LIFO，与 `FileStorage` free list 同型）。
    free_pages: Mutex<Vec<u64>>,
    /// 单调页号分配器（`FileStorage::file_len` 的内存等价物）。
    next_page_id: AtomicU64,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            pages: Mutex::new(HashMap::new()),
            free_pages: Mutex::new(Vec::new()),
            next_page_id: AtomicU64::new(0),
        }
    }

    /// 页内容拷贝：读出为 owned 页（与文件读同型，页大小固定 4KB）。
    fn read_page_sync(&self, page_id: PageId) -> Result<Page> {
        let pages = self.pages.lock().unwrap();
        let data = pages
            .get(&page_id.0)
            .ok_or(StorageError::InvalidPageId(page_id.0))?;
        Ok(Page {
            id: page_id,
            data: Box::new(**data),
        })
    }

    /// 页内容拷贝：写入覆盖既有页内容（分配后首写即建页）。
    fn write_page_sync(&self, page_id: PageId, page: &Page) {
        let mut pages = self.pages.lock().unwrap();
        pages.insert(page_id.0, Box::new(*page.data));
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsyncStorage for MemoryStorage {
    async fn read_page(&self, page_id: PageId) -> Result<Page> {
        self.read_page_sync(page_id)
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
        self.write_page_sync(page_id, page);
        Ok(())
    }

    async fn allocate_page(&self) -> Result<PageId> {
        // free list 优先（锁域独立，不与页表锁嵌套）
        let reused = self.free_pages.lock().unwrap().pop();
        let page_id = match reused {
            Some(id) => PageId(id),
            None => PageId(self.next_page_id.fetch_add(1, Ordering::SeqCst)),
        };
        // 复用页必须读到全零空页：与 FileStorage 回收时写零页同型
        self.write_page_sync(page_id, &Page::new(page_id));
        Ok(page_id)
    }

    async fn free_page(&self, page_id: PageId) -> Result<()> {
        self.write_page_sync(page_id, &Page::new(page_id));
        self.free_pages.lock().unwrap().push(page_id.0);
        Ok(())
    }

    async fn sync(&self) -> Result<()> {
        // 内存页即最新值，无落盘同步面
        Ok(())
    }

    fn page_count(&self) -> u64 {
        self.next_page_id.load(Ordering::SeqCst)
    }
}
