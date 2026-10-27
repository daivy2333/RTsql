//! DropIndexExecutor — 执行 `DROP INDEX`（MS21 Iter002 T2-f，design D4/D14）
//!
//! 登记移除（catalog 段 + 内存 TableMeta，`take_secondary_index` 通道）→
//! 旧树页 best-effort 释放到 free-list（drop_table 同型）。此后等值查询
//! 回落既有扫描（planner 注册通道不再含该索引）。

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::{PageId, Result, StorageError};
use std::sync::Arc;

/// DROP INDEX 执行器
pub struct DropIndexExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl DropIndexExecutor {
    /// 创建新的 DropIndexExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for DropIndexExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let node = match &self.plan {
            PhysicalPlan::DropIndex(node) => node,
            _ => panic!("DropIndexExecutor requires DropIndexNode"),
        };

        let Some((_table, _column, old_tree, anchor)) = self
            .database
            .table_manager
            .take_secondary_index(&node.index_name)
            .await?
        else {
            // 计划期已按登记清单点名拒绝；此臂为防御形态（无 plan cache，
            // 单进程内登记面不会在计划与执行之间变化）。
            return Err(StorageError::ExecutionError(format!(
                "index '{}' does not exist",
                node.index_name
            )));
        };

        // 物理释放（best-effort：collect 失败 warn + 放弃，先例 drop_table）
        let mut released: Vec<PageId> = Vec::new();
        match old_tree.collect_all_pages().await {
            Ok(pages) => {
                for page in pages {
                    match self.database.buffer_pool.free_page(page).await {
                        Ok(()) => released.push(page),
                        Err(e) => {
                            eprintln!("[drop_index] free_page({:?}) failed: {}", page, e)
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[drop_index] collect_all_pages({}) failed: {}",
                    node.index_name, e
                );
            }
        }

        // MS35（`free-page-lifecycle` R3）：释放完成标记——本次释放的锚点写
        // = 目录摘除（`take_secondary_index`），已发出。
        let anchors: Vec<PageId> = anchor.into_iter().collect();
        self.database
            .buffer_pool
            .complete_free_release(&released, &anchors);

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
