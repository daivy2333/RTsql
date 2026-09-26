//! WAL (Write-Ahead Logging) 模块
//!
//! 提供原子性保障和崩溃恢复能力

mod buffer;
mod checkpoint;
mod reader;
mod record;
// MS24 Iter001 replan (2.9/D9): `pub(crate)` 供事务层回滚通道复用
// `recovery::extract_index_keys` 作为索引键派生单一来源（对外导出不变）。
pub(crate) mod recovery;
mod writer;

pub use buffer::WALBuffer;
pub use checkpoint::{CheckpointManager, CheckpointSite};
pub use reader::WalReader;
pub use record::{WalError, WalRecord, WalRecordType};
pub use recovery::RecoveryManager;
pub use writer::WalWriter;
