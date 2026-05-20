use crate::storage::{PageId, Result, StorageError};

/// 固定大小的页，4KB
#[derive(Debug, Clone)]
pub struct Page {
    pub id: PageId,
    pub data: Box<[u8; Self::PAGE_SIZE]>,
}

impl Page {
    pub const PAGE_SIZE: usize = 4096;

    pub fn new(id: PageId) -> Self {
        Self {
            id,
            data: Box::new([0u8; Self::PAGE_SIZE]),
        }
    }

    /// 从字节切片创建页（用于文件读取）
    pub fn from_bytes(id: PageId, bytes: &[u8]) -> Result<Self> {
        if bytes.len() != Self::PAGE_SIZE {
            return Err(StorageError::PageSizeMismatch {
                expected: Self::PAGE_SIZE,
                actual: bytes.len(),
            });
        }
        let mut page = Self::new(id);
        page.data.copy_from_slice(bytes);
        Ok(page)
    }
}