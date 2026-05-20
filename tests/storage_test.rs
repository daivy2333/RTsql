#[cfg(test)]
mod tests {
    use rtsql::storage::{AsyncStorage, BufferPool, FileStorage, Page, PageId};
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    #[test]
    fn test_page_id_offset() {
        let page_id = PageId(5);
        let offset = page_id.to_offset(4096);
        assert_eq!(offset, 20480);
    }

    #[test]
    fn test_page_id_zero() {
        let page_id = PageId(0);
        let offset = page_id.to_offset(4096);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_page_new() {
        let page_id = PageId(1);
        let page = Page::new(page_id);
        assert_eq!(page.id, page_id);
        assert_eq!(page.data.len(), Page::PAGE_SIZE);
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
        let bytes = vec![0u8; 100];
        let result = Page::from_bytes(page_id, &bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_async_storage_trait_signature() {
        struct MockStorage;
        impl MockStorage {
            fn new() -> Self {
                Self
            }
        }
    }

    #[tokio::test]
    async fn test_file_storage_open_new_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = FileStorage::open(temp_file.path()).unwrap();
        assert_eq!(storage.page_size(), 4096);
    }

    #[tokio::test]
    async fn test_file_storage_read_empty_page() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = FileStorage::open(temp_file.path()).unwrap();
        let page_id = storage.allocate_page().await.unwrap();
        let page = storage.read_page(page_id).await.unwrap();
        assert_eq!(page.id, page_id);
        assert_eq!(page.data.len(), 4096);
    }

    #[tokio::test]
    async fn test_file_storage_read_after_write() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = FileStorage::open(temp_file.path()).unwrap();
        let page_id = storage.allocate_page().await.unwrap();
        let mut page = Page::new(page_id);
        page.data[0] = 42;
        page.data[100] = 99;
        storage.write_page(page_id, &page).await.unwrap();
        let read_page = storage.read_page(page_id).await.unwrap();
        assert_eq!(read_page.data[0], 42);
        assert_eq!(read_page.data[100], 99);
    }

    #[tokio::test]
    async fn test_file_storage_write_multiple_pages() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = FileStorage::open(temp_file.path()).unwrap();
        for i in 0..3u8 {
            let page_id = storage.allocate_page().await.unwrap();
            let mut page = Page::new(page_id);
            page.data[0] = i;
            page.data[1] = i * 10;
            storage.write_page(page_id, &page).await.unwrap();
        }
        for i in 0..3u8 {
            let page_id = PageId(i as u64);
            let page = storage.read_page(page_id).await.unwrap();
            assert_eq!(page.data[0], i);
            assert_eq!(page.data[1], i * 10);
        }
    }

    #[tokio::test]
    async fn test_file_storage_allocate_page() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = FileStorage::open(temp_file.path()).unwrap();
        assert_eq!(storage.page_count(), 0);
        let page_id1 = storage.allocate_page().await.unwrap();
        assert_eq!(page_id1.page_num(), 0);
        assert_eq!(storage.page_count(), 1);
        let page_id2 = storage.allocate_page().await.unwrap();
        assert_eq!(page_id2.page_num(), 1);
        assert_eq!(storage.page_count(), 2);
    }

    #[tokio::test]
    async fn test_file_storage_sync() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = FileStorage::open(temp_file.path()).unwrap();
        let page_id = storage.allocate_page().await.unwrap();
        let mut page = Page::new(page_id);
        page.data[0] = 123;
        storage.write_page(page_id, &page).await.unwrap();
        storage.sync().await.unwrap();
        let storage2 = FileStorage::open(temp_file.path()).unwrap();
        let read_page = storage2.read_page(page_id).await.unwrap();
        assert_eq!(read_page.data[0], 123);
    }

    #[tokio::test]
    async fn test_buffer_pool_new() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());
        let pool = BufferPool::new(100, storage).unwrap();
        assert_eq!(pool.capacity(), 100);
    }

    #[tokio::test]
    async fn test_buffer_pool_invalid_capacity() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());
        let result = BufferPool::new(0, storage);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_buffer_pool_get_page_miss() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());
        let page_id = storage.allocate_page().await.unwrap();
        let pool = BufferPool::new(100, storage.clone()).unwrap();
        let guard = pool.get_page(page_id).await.unwrap();
        assert_eq!(guard.page().id, page_id);
        assert_eq!(guard.ref_count(), 1);
    }

    #[tokio::test]
    async fn test_buffer_pool_get_page_hit() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());
        let page_id = storage.allocate_page().await.unwrap();
        let pool = BufferPool::new(100, storage.clone()).unwrap();
        let guard1 = pool.get_page(page_id).await.unwrap();
        drop(guard1);
        let guard2 = pool.get_page(page_id).await.unwrap();
        assert_eq!(guard2.page().id, page_id);
        assert_eq!(guard2.ref_count(), 1);
    }

    #[tokio::test]
    async fn test_buffer_pool_eviction() {
        let temp_file = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::open(temp_file.path()).unwrap());
        for _ in 0..10 {
            storage.allocate_page().await.unwrap();
        }
        let pool = BufferPool::new(5, storage.clone()).unwrap();
        for i in 0..10u64 {
            let guard = pool.get_page(PageId(i)).await.unwrap();
            drop(guard);
        }
        let guard = pool.get_page(PageId(0)).await.unwrap();
        assert_eq!(guard.page().id, PageId(0));
    }
}
