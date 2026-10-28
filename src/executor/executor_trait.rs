//! Executor trait - async iterator interface

use async_trait::async_trait;

use crate::executor::ExecResult;
use crate::storage::Result;

/// Executor trait - 异步迭代器接口
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Executor {
    /// 执行一次迭代，返回结果
    /// None 表示迭代结束（无更多结果）
    async fn next(&mut self) -> Result<Option<ExecResult>>;

    /// MS32-T01 Iter000：批量取出，最多 `max` 行。
    ///
    /// 默认实现逐行调用 `next`，返回序列与连续调用 `next` 完全同序。
    /// `max == 0` 返回空集合且不调用 `next`，不推进内部游标。
    /// 热点算子可覆盖本方法以减少逐行调用的装箱开销；覆盖实现必须
    /// 保持与逐行路径相同的行序与可见性语义。
    async fn next_batch(&mut self, max: usize) -> Result<Vec<ExecResult>> {
        if max == 0 {
            return Ok(Vec::new());
        }
        // 预分配设上限：`max` 是调用方给的批量大小，不应直接决定分配量。
        let mut batch = Vec::with_capacity(max.min(64));
        while batch.len() < max {
            match self.next().await? {
                None => break,
                Some(result) => batch.push(result),
            }
        }
        Ok(batch)
    }
}
