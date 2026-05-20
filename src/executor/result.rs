//! Execution result types

use crate::storage::page_format::RowId;

/// 执行结果类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecResult {
    /// 查询返回 RowId（IndexScan）
    RowId(RowId),
    /// 写操作返回影响计数（Insert/Update/Delete）
    AffectedRows(u64),
    /// Scan 暂不实现（M6 补数据层）
    NotImplemented,
}
