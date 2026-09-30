//! DropViewExecutor — 执行 `DROP VIEW`（MS21 Iter005 T5-d，design D12）
//!
//! 视图非物化：删除 = 「catalog 视图行 + 列名行移除（复用既有
//! `delete_table` 双链删除机制）+ 内存注册表移除」，不涉及数据页/索引树
//! 释放。计划期已完成存在性检查（TOCTOU 窗口由执行器侧注册表复检闭合）；
//! 基表数据不受影响。

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::{Result, StorageError};
use std::sync::Arc;

/// DROP VIEW 执行器
pub struct DropViewExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl DropViewExecutor {
    /// 创建新的 DropViewExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for DropViewExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let node = match &self.plan {
            PhysicalPlan::DropView(node) => node,
            _ => panic!("DropViewExecutor requires DropViewNode"),
        };

        // TOCTOU 复检（create_table 先例）：plan 后、执行前注册表可能变化。
        // 基表名不是视图操作目标（计划臂按视图注册表判定，此处同面）。
        if !self
            .database
            .views
            .read()
            .await
            .contains_key(&node.view_name)
        {
            return Err(StorageError::TableNotFound(node.view_name.clone()));
        }

        // catalog 删除：`delete_table` 复用既有双链删除机制（__tables 行 +
        // __columns 行）；idempotent。
        self.database
            .table_manager
            .catalog()
            .delete_table(&node.view_name)
            .await?;

        // 内存注册表移除
        self.database.views.write().await.remove(&node.view_name);

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
