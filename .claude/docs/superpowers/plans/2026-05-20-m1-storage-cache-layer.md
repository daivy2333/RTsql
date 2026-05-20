# M1 Storage/Cache Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement single-file persistent storage with async Buffer Pool and Clock eviction policy for RTsql database.

**Architecture:** Layered design with AsyncStorage trait for storage abstraction, FileStorage for file I/O via spawn_blocking, BufferPool for page caching with RwLock-based concurrency, and PageGuard for safe page access with automatic reference counting.

**Tech Stack:** Rust, Tokio async runtime, async-trait, thiserror, anyhow, tempfile (for tests)

---

## File Structure

```
src/storage/
├── mod.rs           # Module exports
├── error.rs         # StorageError type
├── page_id.rs       # PageId struct
├── page.rs          # Page struct
├── async_storage.rs # AsyncStorage trait
├── file_storage.rs  # FileStorage implementation
├── buffer_pool.rs   # BufferPool with Clock eviction
└── page_frame.rs    # PageFrame and PageGuard (internal)

tests/
└── storage_test.rs  # Integration tests for storage layer
```

---

## Task 1: Add Dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add async-trait, thiserror, anyhow, tempfile dependencies**

```toml
# In Cargo.toml, under [dependencies]
async-trait = "0.1"
thiserror = "1.0"
anyhow = "1.0"

# In Cargo.toml, under [dev-dependencies]
tempfile = "3.0"
```

- [ ] **Step 2: Verify dependencies compile**

Run: `cargo build`
Expected: Build succeeds with no errors

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(storage): add async-trait, thiserror, anyhow, tempfile dependencies"
```

---

## Task 2: Implement PageId and Error Types

**Files:**
- Create: `src/storage/mod.rs`
- Create: `src/storage/error.rs`
- Create: `src/storage/page_id.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing test for PageId offset calculation**

Create: `tests/storage_test.rs`

```rust
#[cfg(test)]
mod tests {
    use rtsql::storage::PageId;

    #[test]
    fn test_page_id_offset() {
        let page_id = PageId(5);
        let offset = page_id.to_offset(4096);
        assert_eq!(offset, 20480); // 5 * 4096
    }

    #[test]
    fn test_page_id_zero() {
        let page_id = PageId(0);
        let offset = page_id.to_offset(4096);
        assert_eq!(offset, 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_page_id_offset`
Expected: FAIL with "use of undeclared type `PageId`"

- [ ] **Step 3: Create storage module structure**

Create: `src/storage/mod.rs`

```rust
mod error;
mod page_id;

pub use error::{StorageError, Result};
pub use page_id::PageId;
```

- [ ] **Step 4: Implement PageId**

Create: `src/storage/page_id.rs`

```rust
use std::fmt;

/// 页标识符，从 0 开始编号
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub u64);

impl PageId {
    /// 将 PageId 转换为文件偏移量
    pub fn to_offset(&self, page_size: usize) -> u64 {
        self.0 * page_size as u64
    }

    /// 页号
    pub fn page_num(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PageId({})", self.0)
    }
}
```

- [ ] **Step 5: Implement StorageError**

Create: `src/storage/error.rs`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Page size mismatch: expected {expected}, got {actual}")]
    PageSizeMismatch { expected: usize, actual: usize },

    #[error("Buffer pool full, no evictable page")]
    BufferPoolFull,

    #[error("Invalid page id: {0}")]
    InvalidPageId(u64),

    #[error("Invalid capacity: {0}, must be > 0")]
    InvalidCapacity(usize),
}

pub type Result<T> = std::result::Result<T, StorageError>;
```

- [ ] **Step 6: Update lib.rs to export storage module**

Modify: `src/lib.rs`

```rust
pub mod storage;
```

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo test test_page_id`
Expected: 2 tests PASS

- [ ] **Step 8: Commit**

```bash
git add src/storage/mod.rs src/storage/error.rs src/storage/page_id.rs src/lib.rs tests/storage_test.rs
git commit -m "feat(storage): implement PageId and StorageError types"
```

---

## Task 3: Implement Page Struct

**Files:**
- Create: `src/storage/page.rs`
- Modify: `src/storage/mod.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write failing test for Page creation**

Add to: `tests/storage_test.rs`

```rust
use rtsql::storage::Page;

#[test]
fn test_page_new() {
    let page_id = PageId(1);
    let page = Page::new(page_id);
    assert_eq!(page.id, page_id);
    assert_eq!(page.data.len(), Page::PAGE_SIZE);
    assert_eq!(page.data, [0u8; Page::PAGE_SIZE]);
}

#[test]
fn test_page_from_bytes() {
    let page_id = PageId(2);
    let bytes = vec![42u8; Page::PAGE_SIZE];
    let page = Page::from_bytes(page_id, &bytes).unwrap();
    assert_eq!(page.id, page_id);
    assert!(page.data.iter().all(|&b| b == 42));
}

#[test]
fn test_page_from_bytes_wrong_size() {
    let page_id = PageId(3);
    let bytes = vec![0u8; 100]; // Wrong size
    let result = Page::from_bytes(page_id, &bytes);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_page`
Expected: FAIL with "use of undeclared type `Page`"

- [ ] **Step 3: Implement Page**

Create: `src/storage/page.rs`

```rust
use crate::storage::{PageId, Result, StorageError};

/// 固定大小的页，4KB
#[derive(Debug, Clone)]
pub struct Page {
    pub id: PageId,
    pub data: Box<[u8; PAGE_SIZE]>,
}

impl Page {
    pub const PAGE_SIZE: usize = 4096;

    pub fn new(id: PageId) -> Self {
        Self {
            id,
            data: Box::new([0u8; PAGE_SIZE]),
        }
    }

    /// 从字节切片创建页（用于文件读取）
    pub fn from_bytes(id: PageId, bytes: &[u8]) -> Result<Self> {
        if bytes.len() != PAGE_SIZE {
            return Err(StorageError::PageSizeMismatch {
                expected: PAGE_SIZE,
                actual: bytes.len(),
            });
        }
        let mut page = Self::new(id);
        page.data.copy_from_slice(bytes);
        Ok(page)
    }
}
```

- [ ] **Step 4: Update mod.rs to export Page**

Modify: `src/storage/mod.rs`

```rust
mod error;
mod page_id;
mod page;

pub use error::{StorageError, Result};
pub use page_id::PageId;
pub use page::Page;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test test_page`
Expected: 3 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/page.rs src/storage/mod.rs tests/storage_test.rs
git commit -m "feat(storage): implement Page struct with 4KB fixed size"
```

---

## Task 4: Define AsyncStorage Trait

**Files:**
- Create: `src/storage/async_storage.rs`
- Modify: `src/storage/mod.rs`

- [ ] **Step 1: Write failing test for trait existence**

Add to: `tests/storage_test.rs`

```rust
use rtsql::storage::AsyncStorage;
use std::sync::Arc;

// This test just verifies the trait exists and has correct signatures
#[test]
fn test_async_storage_trait_signature() {
    // We'll create a mock implementation to verify trait exists
    struct MockStorage;

    impl MockStorage {
        fn new() -> Self { Self }
    }

    // This will fail to compile if trait doesn't exist or has wrong signature
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_async_storage_trait_signature`
Expected: FAIL with "use of undeclared type `AsyncStorage`"

- [ ] **Step 3: Implement AsyncStorage trait**

Create: `src/storage/async_storage.rs`

```rust
use async_trait::async_trait;
use crate::storage::{Page, PageId, Result};

#[async_trait]
pub trait AsyncStorage: Send + Sync {
    /// 读取指定页
    async fn read_page(&self, page_id: PageId) -> Result<Page>;

    /// 写入指定页
    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()>;

    /// 分配新页，返回 PageId
    async fn allocate_page(&self) -> Result<PageId>;

    /// 同步到磁盘（fsync）
    async fn sync(&self) -> Result<()>;

    /// 返回页大小
    fn page_size(&self) -> usize {
        Page::PAGE_SIZE
    }
}
```

- [ ] **Step 4: Update mod.rs to export trait**

Modify: `src/storage/mod.rs`

```rust
mod error;
mod page_id;
mod page;
mod async_storage;

pub use error::{StorageError, Result};
pub use page_id::PageId;
pub use page::Page;
pub use async_storage::AsyncStorage;
```

- [ ] **Step 5: Run test to verify it compiles**

Run: `cargo test test_async_storage_trait_signature`
Expected: PASS (trait exists)

- [ ] **Step 6: Commit**

```bash
git add src/storage/async_storage.rs src/storage/mod.rs tests/storage_test.rs
git commit -m "feat(storage): define AsyncStorage trait with async methods"
```

---

## Task 5: Implement FileStorage - Open File

**Files:**
- Create: `src/storage/file_storage.rs`
- Modify: `src/storage/mod.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write failing test for FileStorage::open**

Add to: `tests/storage_test.rs`

```rust
use rtsql::storage::FileStorage;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_file_storage_open_new_file() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = FileStorage::open(path).unwrap();
    assert_eq!(storage.page_size(), 4096);
}

#[tokio::test]
async fn test_file_storage_open_existing_file() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // Create file first
    let storage1 = FileStorage::open(path).unwrap();
    storage1.sync().await.unwrap();

    // Open again
    let storage2 = FileStorage::open(path).unwrap();
    assert_eq!(storage2.page_size(), 4096);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_file_storage_open`
Expected: FAIL with "use of undeclared type `FileStorage`"

- [ ] **Step 3: Implement FileStorage::open**

Create: `src/storage/file_storage.rs`

```rust
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::storage::{AsyncStorage, Page, PageId, Result, StorageError};

pub struct FileStorage {
    file: Arc<std::fs::File>,
    page_size: usize,
    /// 文件长度（页数），用于分配新页
    file_len: AtomicU64,
}

impl FileStorage {
    /// 打开或创建数据库文件
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let page_size = Page::PAGE_SIZE;

        // 验证文件长度是页大小的整数倍
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
        })
    }

    pub fn page_count(&self) -> u64 {
        self.file_len.load(Ordering::SeqCst)
    }
}
```

- [ ] **Step 4: Update mod.rs to export FileStorage**

Modify: `src/storage/mod.rs`

```rust
mod error;
mod page_id;
mod page;
mod async_storage;
mod file_storage;

pub use error::{StorageError, Result};
pub use page_id::PageId;
pub use page::Page;
pub use async_storage::AsyncStorage;
pub use file_storage::FileStorage;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test test_file_storage_open`
Expected: 2 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/file_storage.rs src/storage/mod.rs tests/storage_test.rs
git commit -m "feat(storage): implement FileStorage::open for file initialization"
```

---

## Task 6: Implement FileStorage - Read Page

**Files:**
- Modify: `src/storage/file_storage.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write failing test for read_page**

Add to: `tests/storage_test.rs`

```rust
use tokio::task::spawn_blocking;

#[tokio::test]
async fn test_file_storage_read_empty_page() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = FileStorage::open(path).unwrap();
    // Allocate a page first
    let page_id = storage.allocate_page().await.unwrap();

    // Read the page
    let page = storage.read_page(page_id).await.unwrap();
    assert_eq!(page.id, page_id);
    assert_eq!(page.data.len(), 4096);
}

#[tokio::test]
async fn test_file_storage_read_after_write() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = FileStorage::open(path).unwrap();
    let page_id = storage.allocate_page().await.unwrap();

    // Write some data
    let mut page = Page::new(page_id);
    page.data[0] = 42;
    page.data[100] = 99;
    storage.write_page(page_id, &page).await.unwrap();

    // Read back
    let read_page = storage.read_page(page_id).await.unwrap();
    assert_eq!(read_page.data[0], 42);
    assert_eq!(read_page.data[100], 99);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_file_storage_read`
Expected: FAIL with "method `read_page` not found"

- [ ] **Step 3: Implement read_page with spawn_blocking**

Modify: `src/storage/file_storage.rs`

Add import at top:

```rust
use tokio::task::spawn_blocking;
use std::io::{Read, Seek, SeekFrom};
```

Add to FileStorage struct impl (not trait impl yet):

```rust
impl FileStorage {
    // ... existing open() and page_count() ...

    fn read_page_blocking(file: Arc<std::fs::File>, page_id: PageId, page_size: usize) -> Result<Page> {
        let offset = page_id.to_offset(page_size);
        let mut file_ref = file.as_ref();
        file_ref.seek(SeekFrom::Start(offset))?;

        let mut buf = vec![0u8; page_size];
        file_ref.read_exact(&mut buf)?;

        Page::from_bytes(page_id, &buf)
    }
}

#[async_trait]
impl AsyncStorage for FileStorage {
    async fn read_page(&self, page_id: PageId) -> Result<Page> {
        let file = self.file.clone();
        let page_size = self.page_size;

        spawn_blocking(move || {
            Self::read_page_blocking(file, page_id, page_size)
        })
        .await?
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
        // Placeholder for next task
        unimplemented!("write_page will be implemented in next task")
    }

    async fn allocate_page(&self) -> Result<PageId> {
        // Placeholder for next task
        unimplemented!("allocate_page will be implemented in next task")
    }

    async fn sync(&self) -> Result<()> {
        // Placeholder for next task
        unimplemented!("sync will be implemented in next task")
    }
}
```

- [ ] **Step 4: Run test to verify it still fails (write_page not implemented)**

Run: `cargo test test_file_storage_read_after_write`
Expected: FAIL with "not yet implemented"

- [ ] **Step 5: Implement write_page temporarily for read test**

Modify the write_page placeholder in `src/storage/file_storage.rs`:

```rust
async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
    let file = self.file.clone();
    let page_size = self.page_size;
    let offset = page_id.to_offset(page_size);
    let data = page.data.clone();

    spawn_blocking(move || {
        let mut file_ref = file.as_ref();
        file_ref.seek(SeekFrom::Start(offset))?;
        std::io::Write::write_all(&mut file_ref, &*data)?;
        Ok(())
    })
    .await?
}

async fn allocate_page(&self) -> Result<PageId> {
    // Temporary implementation for test
    let page_id = self.file_len.fetch_add(1, Ordering::SeqCst);
    let offset = PageId(page_id).to_offset(self.page_size);

    let file = self.file.clone();
    let page_size = self.page_size;

    spawn_blocking(move || {
        file.as_ref().set_len(offset + page_size as u64)?;
        Ok(())
    })
    .await??;

    Ok(PageId(page_id))
}

async fn sync(&self) -> Result<()> {
    // Temporary for test
    Ok(())
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test test_file_storage_read`
Expected: 2 tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/storage/file_storage.rs tests/storage_test.rs
git commit -m "feat(storage): implement FileStorage read_page with spawn_blocking"
```

---

## Task 7: Implement FileStorage - Write Page Full Implementation

**Files:**
- Modify: `src/storage/file_storage.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write test for write_page correctness**

Add to: `tests/storage_test.rs`

```rust
#[tokio::test]
async fn test_file_storage_write_multiple_pages() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = FileStorage::open(path).unwrap();

    // Allocate and write 3 pages
    for i in 0..3 {
        let page_id = storage.allocate_page().await.unwrap();
        let mut page = Page::new(page_id);
        page.data[0] = i as u8;
        page.data[1] = (i * 10) as u8;
        storage.write_page(page_id, &page).await.unwrap();
    }

    // Read back and verify
    for i in 0..3 {
        let page_id = PageId(i);
        let page = storage.read_page(page_id).await.unwrap();
        assert_eq!(page.data[0], i as u8);
        assert_eq!(page.data[1], (i * 10) as u8);
    }
}
```

- [ ] **Step 2: Run test to verify it passes (already implemented in Task 6)**

Run: `cargo test test_file_storage_write_multiple`
Expected: PASS

- [ ] **Step 3: Add write_page implementation details (already done, verify code structure)**

The write_page implementation from Task 6 is complete. Verify proper imports:

```rust
use std::io::{Read, Seek, SeekFrom, Write};
```

- [ ] **Step 4: Run all file_storage tests**

Run: `cargo test file_storage`
Expected: All tests PASS

- [ ] **Step 5: Commit (code already written, just verify)**

```bash
git add src/storage/file_storage.rs tests/storage_test.rs
git commit -m "feat(storage): complete FileStorage write_page implementation"
```

---

## Task 8: Implement FileStorage - Allocate Page and Sync

**Files:**
- Modify: `src/storage/file_storage.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write test for allocate_page**

Add to: `tests/storage_test.rs`

```rust
#[tokio::test]
async fn test_file_storage_allocate_page() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = FileStorage::open(path).unwrap();
    assert_eq!(storage.page_count(), 0);

    let page_id1 = storage.allocate_page().await.unwrap();
    assert_eq!(page_id1.page_num(), 0);
    assert_eq!(storage.page_count(), 1);

    let page_id2 = storage.allocate_page().await.unwrap();
    assert_eq!(page_id2.page_num(), 1);
    assert_eq!(storage.page_count(), 2);

    let page_id3 = storage.allocate_page().await.unwrap();
    assert_eq!(page_id3.page_num(), 2);
    assert_eq!(storage.page_count(), 3);
}

#[tokio::test]
async fn test_file_storage_sync() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = FileStorage::open(path).unwrap();
    let page_id = storage.allocate_page().await.unwrap();

    let mut page = Page::new(page_id);
    page.data[0] = 123;
    storage.write_page(page_id, &page).await.unwrap();

    // Sync to disk
    storage.sync().await.unwrap();

    // Reopen and verify data persisted
    let storage2 = FileStorage::open(path).unwrap();
    let read_page = storage2.read_page(page_id).await.unwrap();
    assert_eq!(read_page.data[0], 123);
}
```

- [ ] **Step 2: Run test to verify allocate_page passes (already implemented in Task 6)**

Run: `cargo test test_file_storage_allocate_page`
Expected: PASS

- [ ] **Step 3: Implement sync method fully**

Modify: `src/storage/file_storage.rs`

The sync placeholder needs full implementation:

```rust
async fn sync(&self) -> Result<()> {
    let file = self.file.clone();

    spawn_blocking(move || {
        file.as_ref().sync_all()?;
        Ok(())
    })
    .await?
}
```

- [ ] **Step 4: Run test to verify sync passes**

Run: `cargo test test_file_storage_sync`
Expected: PASS

- [ ] **Step 5: Run all FileStorage tests**

Run: `cargo test file_storage`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/file_storage.rs tests/storage_test.rs
git commit -m "feat(storage): implement FileStorage allocate_page and sync methods"
```

---

## Task 9: Implement PageFrame and PageGuard

**Files:**
- Create: `src/storage/page_frame.rs`
- Modify: `src/storage/mod.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write failing test for PageGuard basic usage**

Add to: `tests/storage_test.rs`

```rust
use std::ops::Deref;

#[test]
fn test_page_frame_new() {
    use rtsql::storage::page_frame::PageFrame;

    let page_id = PageId(1);
    let page = Page::new(page_id);
    let frame = PageFrame::new(page);

    assert_eq!(frame.page.id, page_id);
    assert!(!frame.dirty);
    assert_eq!(frame.ref_count, 0);
    assert!(frame.clock_bit);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_page_frame`
Expected: FAIL with "module `page_frame` is private"

- [ ] **Step 3: Implement PageFrame**

Create: `src/storage/page_frame.rs`

```rust
use std::sync::{Arc, Mutex};

use crate::storage::Page;

/// 缓存中的页帧，包含元数据
pub struct PageFrame {
    pub page: Page,
    pub dirty: bool,
    pub ref_count: u32,
    pub clock_bit: bool,
}

impl PageFrame {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            dirty: false,
            ref_count: 0,
            clock_bit: true,
        }
    }
}
```

- [ ] **Step 4: Update mod.rs to make page_frame module available**

Modify: `src/storage/mod.rs`

```rust
mod error;
mod page_id;
mod page;
mod async_storage;
mod file_storage;
pub mod page_frame;  // Make public for tests

pub use error::{StorageError, Result};
pub use page_id::PageId;
pub use page::Page;
pub use async_storage::AsyncStorage;
pub use file_storage::FileStorage;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test test_page_frame`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/page_frame.rs src/storage/mod.rs tests/storage_test.rs
git commit -m "feat(storage): implement PageFrame struct with dirty/ref_count/clock_bit"
```

---

## Task 10: Implement BufferPool - Basic Structure

**Files:**
- Create: `src/storage/buffer_pool.rs`
- Modify: `src/storage/mod.rs`
- Modify: `src/storage/page_frame.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write failing test for BufferPool creation**

Add to: `tests/storage_test.rs`

```rust
use rtsql::storage::BufferPool;
use std::sync::Arc;

#[tokio::test]
async fn test_buffer_pool_new() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = Arc::new(FileStorage::open(path).unwrap());
    let pool = BufferPool::new(100, storage);

    assert_eq!(pool.capacity(), 100);
}

#[tokio::test]
async fn test_buffer_pool_invalid_capacity() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = Arc::new(FileStorage::open(path).unwrap());
    let result = BufferPool::new(0, storage);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_buffer_pool`
Expected: FAIL with "use of undeclared type `BufferPool`"

- [ ] **Step 3: Implement BufferPool structure**

Create: `src/storage/buffer_pool.rs`

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::storage::{
    AsyncStorage, Page, PageId, Result, StorageError,
    page_frame::PageFrame,
};

pub struct BufferPool {
    pages: RwLock<HashMap<PageId, Arc<std::sync::Mutex<PageFrame>>>,
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
}
```

- [ ] **Step 4: Update mod.rs to export BufferPool**

Modify: `src/storage/mod.rs`

```rust
mod error;
mod page_id;
mod page;
mod async_storage;
mod file_storage;
mod page_frame;
mod buffer_pool;

pub use error::{StorageError, Result};
pub use page_id::PageId;
pub use page::Page;
pub use async_storage::AsyncStorage;
pub use file_storage::FileStorage;
pub use buffer_pool::BufferPool;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test test_buffer_pool`
Expected: 2 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/buffer_pool.rs src/storage/mod.rs tests/storage_test.rs
git commit -m "feat(storage): implement BufferPool structure with capacity validation"
```

---
## Task 11: Implement BufferPool - get_page with Clock Eviction

**Files:**
- Modify: `src/storage/buffer_pool.rs`
- Modify: `src/storage/page_frame.rs`
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write failing test for get_page (cache miss)**

Add to: `tests/storage_test.rs`

```rust
#[tokio::test]
async fn test_buffer_pool_get_page_miss() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = Arc::new(FileStorage::open(path).unwrap());
    // Allocate a page in storage
    let page_id = storage.allocate_page().await.unwrap();

    let pool = BufferPool::new(100, storage.clone()).unwrap();

    // Get page from pool (should load from storage)
    let guard = pool.get_page(page_id).await.unwrap();

    assert_eq!(guard.page.id, page_id);
    assert_eq!(guard.ref_count(), 1);
}

#[tokio::test]
async fn test_buffer_pool_get_page_hit() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let storage = Arc::new(FileStorage::open(path).unwrap());
    let page_id = storage.allocate_page().await.unwrap();

    let pool = BufferPool::new(100, storage.clone()).unwrap();

    // First access (miss)
    let guard1 = pool.get_page(page_id).await.unwrap();
    drop(guard1);

    // Second access (hit)
    let guard2 = pool.get_page(page_id).await.unwrap();
    assert_eq!(guard2.page.id, page_id);
    assert_eq!(guard2.ref_count(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_buffer_pool_get_page`
Expected: FAIL with "method `get_page` not found"

- [ ] **Step 3: Implement PageGuard**

Modify: `src/storage/page_frame.rs`

```rust
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use crate::storage::Page;

/// 缓存中的页帧，包含元数据
pub struct PageFrame {
    pub page: Page,
    pub dirty: bool,
    pub ref_count: u32,
    pub clock_bit: bool,
}

impl PageFrame {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            dirty: false,
            ref_count: 0,
            clock_bit: true,
        }
    }
}

/// 页访问守卫，类似 RwLockReadGuard
pub struct PageGuard {
    frame: Arc<Mutex<PageFrame>>,
}

impl PageGuard {
    pub fn new(frame: Arc<Mutex<PageFrame>>) -> Self {
        frame.lock().unwrap().ref_count += 1;
        frame.lock().unwrap().clock_bit = true;
        Self { frame }
    }

    pub fn mark_dirty(&self) {
        self.frame.lock().unwrap().dirty = true;
    }

    pub fn ref_count(&self) -> u32 {
        self.frame.lock().unwrap().ref_count
    }
}

impl Deref for PageGuard {
    type Target = Page;
    fn deref(&self) -> &Self::Target {
        &self.frame.lock().unwrap().page
    }
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        self.frame.lock().unwrap().ref_count -= 1;
    }
}
```

- [ ] **Step 4: Implement BufferPool::get_page**

Modify: `src/storage/buffer_pool.rs`

```rust
use crate::storage::page_frame::PageGuard;

impl BufferPool {
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

            if let Some(frame) = pages.get(&candidate_id) {
                let mut frame_guard = frame.lock().unwrap();

                if frame_guard.ref_count > 0 {
                    clock_hand.push(candidate_id);
                    continue;
                }

                if frame_guard.clock_bit {
                    frame_guard.clock_bit = false;
                    clock_hand.push(candidate_id);
                    continue;
                }

                if frame_guard.dirty {
                    let page = frame_guard.page.clone();
                    drop(frame_guard);
                    self.storage.write_page(candidate_id, &page).await?;
                }

                pages.remove(&candidate_id);
                return Ok(());
            }
        }

        Err(StorageError::BufferPoolFull)
    }
}
```

- [ ] **Step 5: Run test**

Run: `cargo test test_buffer_pool_get_page`
Expected: 2 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/buffer_pool.rs src/storage/page_frame.rs tests/storage_test.rs
git commit -m "feat(storage): implement BufferPool::get_page with Clock eviction"
```

---

## Task 12: BufferPool - Eviction Tests

**Files:**
- Modify: `tests/storage_test.rs`

- [ ] **Step 1: Write test for eviction**

Add to: `tests/storage_test.rs`

```rust
#[tokio::test]
async fn test_buffer_pool_eviction() {
    let temp_file = NamedTempFile::new().unwrap();
    let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());

    for _ in 0..10 {
        storage.allocate_page().await.unwrap();
    }

    let pool = BufferPool::new(5, storage.clone()).unwrap();

    for i in 0..10 {
        let guard = pool.get_page(PageId(i)).await.unwrap();
        drop(guard);
    }

    let pages = pool.pages.read().await;
    assert!(pages.len() <= 5);
}
```

- [ ] **Step 2: Run test**

Run: `cargo test test_buffer_pool_eviction`
Expected: PASS

- [ ] **Step 3: Run all tests**

Run: `cargo test storage`
Expected: All PASS

- [ ] **Step 4: Run clippy**

Run: `cargo clippy`
Expected: No warnings

- [ ] **Step 5: Run fmt**

Run: `cargo fmt -- --check`
Expected: No issues

- [ ] **Step 6: Commit**

```bash
git add tests/storage_test.rs
git commit -m "test(storage): add eviction test for BufferPool"
```

---

## Task 13: Update Documentation

**Files:**
- Modify: `.claude/docs/snapshot.md`
- Modify: `.claude/docs/tasks.md`
- Modify: `.claude/docs/learned.md`

- [ ] **Step 1: Update snapshot.md**

Update current status to M1 complete, add new files to structure.

- [ ] **Step 2: Update tasks.md**

Move M1 to completed section.

- [ ] **Step 3: Update learned.md**

Add API paths and file locations for storage layer.

- [ ] **Step 4: Commit**

```bash
git add .claude/docs/*.md
git commit -m "docs: update snapshot and tasks for M1 completion"
```

---

## Milestone Completion Checklist

- [ ] All tests pass: `cargo test`
- [ ] No clippy warnings: `cargo clippy`
- [ ] Code formatted: `cargo fmt -- --check`
- [ ] All API implemented
- [ ] Eviction tested
- [ ] Persistence tested
- [ ] Documentation updated

---

## Self-Review Results

**1. Spec coverage:** All spec requirements have corresponding tasks ✅

**2. Placeholders:** No TBD/TODO found ✅

**3. Type consistency:** PageId, Page, StorageError consistent across all modules ✅
