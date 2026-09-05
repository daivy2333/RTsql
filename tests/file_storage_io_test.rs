//! MS08-T01: FileStorage 页 I/O 位置参数化（pread/pwrite）回归守卫。
//!
//! 场景（change spec R1）：
//! - 多页读写往返等价；
//! - 越界读取报错语义不变（UnexpectedEof → StorageError::Io）；
//! - 16 任务并发冷读不同页无错读（共享文件偏移竞态的结构性守卫）；
//! - 并发读写混合不串页。

use std::sync::Arc;

use rtsql::storage::{AsyncStorage, FileStorage, Page, PageId, StorageError};
use tempfile::NamedTempFile;

/// 页内容模式：每字节由 (page_num, offset) 唯一决定；31 与 256 互质，
/// 不同页在任意偏移上的模式字节必然不同，任何串页/错读都会立即暴露。
fn pattern_byte(page_num: u64, offset: usize) -> u8 {
    (page_num as u8).wrapping_mul(31).wrapping_add(offset as u8)
}

fn fill_page(page: &mut Page) {
    let page_num = page.id.0;
    for (i, b) in page.data.iter_mut().enumerate() {
        *b = pattern_byte(page_num, i);
    }
}

fn verify_page(page: &Page) -> Result<(), String> {
    let page_num = page.id.0;
    for (i, &b) in page.data.iter().enumerate() {
        if b != pattern_byte(page_num, i) {
            return Err(format!(
                "page {} offset {} corrupted: got {}, expected {}",
                page_num,
                i,
                b,
                pattern_byte(page_num, i)
            ));
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_file_storage_roundtrip_multi_page() {
    let temp_file = NamedTempFile::new().unwrap();
    let storage = FileStorage::open(temp_file.path()).unwrap();

    let mut ids = Vec::new();
    for _ in 0..4 {
        let id = storage.allocate_page().await.unwrap();
        let mut page = Page::new(id);
        fill_page(&mut page);
        storage.write_page(id, &page).await.unwrap();
        ids.push(id);
    }

    for id in ids {
        let page = storage.read_page(id).await.unwrap();
        assert_eq!(page.id, id);
        verify_page(&page).unwrap_or_else(|e| panic!("roundtrip mismatch: {e}"));
    }
}

#[tokio::test]
async fn test_file_storage_read_out_of_bounds_errors() {
    let temp_file = NamedTempFile::new().unwrap();
    let storage = FileStorage::open(temp_file.path()).unwrap();
    storage.allocate_page().await.unwrap(); // 文件长度 = 1 页

    // PageId(3) 偏移 12KiB 超出文件长度：短读必须以 UnexpectedEof 报错
    let err = storage.read_page(PageId(3)).await.unwrap_err();
    assert!(
        matches!(err, StorageError::Io(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
        "expected Io(UnexpectedEof), got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_file_storage_concurrent_cold_read_no_cross_page() {
    let temp_file = NamedTempFile::new().unwrap();
    let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());

    const TASKS: u64 = 16;
    const ROUNDS: usize = 250;
    for id in 0..TASKS {
        let mut page = Page::new(PageId(id));
        fill_page(&mut page);
        storage.write_page(PageId(id), &page).await.unwrap();
    }

    let mut handles = Vec::new();
    for task in 0..TASKS {
        let storage = storage.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..ROUNDS {
                let page = storage
                    .read_page(PageId(task))
                    .await
                    .expect("cold read failed");
                assert_eq!(page.id, PageId(task));
                verify_page(&page).unwrap_or_else(|e| panic!("cross-page read detected: {e}"));
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_file_storage_concurrent_mixed_read_write_no_crosstalk() {
    // 8 页：每页 1 个写者（反复重写自身模式）+ 1 个读者（校验自身模式）。
    // 写者写入的内容恒等于模式，读者读到的合法内容只有模式本身，
    // 因此校验失败只可能来自跨页串写/串读。
    let temp_file = NamedTempFile::new().unwrap();
    let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());

    const PAGES: u64 = 8;
    const ROUNDS: usize = 150;
    for id in 0..PAGES {
        let mut page = Page::new(PageId(id));
        fill_page(&mut page);
        storage.write_page(PageId(id), &page).await.unwrap();
    }

    let mut handles = Vec::new();
    for page_num in 0..PAGES {
        let writer_storage = storage.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..ROUNDS {
                let mut page = Page::new(PageId(page_num));
                fill_page(&mut page);
                writer_storage
                    .write_page(PageId(page_num), &page)
                    .await
                    .expect("writer failed");
            }
        }));
        let reader_storage = storage.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..ROUNDS {
                let page = reader_storage
                    .read_page(PageId(page_num))
                    .await
                    .expect("reader failed");
                assert_eq!(page.id, PageId(page_num));
                verify_page(&page).unwrap_or_else(|e| panic!("cross-talk detected: {e}"));
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
}
