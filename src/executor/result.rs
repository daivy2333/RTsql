//! Execution result types

use crate::executor::value::Value;
use crate::storage::page_format::RowId;

/// 执行结果类型
#[derive(Debug, Clone, PartialEq)]
pub enum ExecResult {
    /// 查询返回 RowId（IndexScan）
    RowId(RowId),
    /// 写操作返回影响计数（Insert/Update/Delete）
    AffectedRows(u64),
    /// 查询返回行数据（Scan）
    Row(Vec<Value>),
}
