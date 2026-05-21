//! Checkpoint 管理器
//!
//! 负责定期创建检查点，截断 WAL

/// Checkpoint 管理器
pub struct CheckpointManager {
    /// 检查点间隔（事务数）
    interval: usize,
}

impl CheckpointManager {
    /// 创建新的 Checkpoint 管理器
    pub fn new(interval: usize) -> Self {
        Self { interval }
    }
}
