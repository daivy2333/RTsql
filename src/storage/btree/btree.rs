// BTree 核心逻辑（Task 5 实现）
use std::sync::Arc;

use crate::storage::{
    page_format::RowId,
    PageId, Result, StorageError,
};

use super::SyncPageLoader;

pub struct BTree {
    loader: Arc<SyncPageLoader>,
    root_page_id: PageId,
}

impl BTree {
    pub fn new(loader: Arc<SyncPageLoader>) -> Result<Self> {
        let root_page_id = loader.allocate_page()?;
        Ok(Self {
            loader,
            root_page_id,
        })
    }

    pub fn search(&self, _key: &[u8]) -> Result<Option<RowId>> {
        // Task 5 实现
        Err(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, "BTree::search not implemented yet")))
    }

    pub fn insert(&self, _key: &[u8], _row_id: RowId) -> Result<()> {
        // Task 5 实现
        Err(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, "BTree::insert not implemented yet")))
    }

    pub fn delete(&self, _key: &[u8]) -> Result<()> {
        // Task 5 实现
        Err(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, "BTree::delete not implemented yet")))
    }

    pub fn update(&self, _key: &[u8], _new_row_id: RowId) -> Result<()> {
        // Task 5 实现
        Err(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, "BTree::update not implemented yet")))
    }
}