mod error;
mod page_id;
mod page;
mod async_storage;
mod file_storage;
pub mod page_frame;

pub use error::{StorageError, Result};
pub use page_id::PageId;
pub use page::Page;
pub use async_storage::AsyncStorage;
pub use file_storage::FileStorage;
pub use page_frame::PageGuard;