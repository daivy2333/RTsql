//! DropTableExecutor - 执行 DROP TABLE 物理计划

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::{Result, StorageError};
use std::sync::Arc;

/// DROP TABLE 执行器
pub struct DropTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl DropTableExecutor {
    /// 创建新的 DropTableExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for DropTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        // 提取 DropTableNode
        let node = match &self.plan {
            PhysicalPlan::DropTable(node) => node,
            _ => panic!("DropTableExecutor requires DropTableNode"),
        };

        // 检查表是否存在
        let table_exists = self.database.table_manager.table_exists(&node.table_name);

        // 根据 if_exists 决定行为
        if !table_exists {
            if node.if_exists {
                // 表不存在但 IF EXISTS=true，返回成功（不报错）
                return Ok(Some(ExecResult::AffectedRows(0)));
            } else {
                // 表不存在且 IF EXISTS=false，返回错误
                return Err(StorageError::TableNotFound(node.table_name.clone()));
            }
        }

        // 表存在，调用 drop_table
        self.database
            .table_manager
            .drop_table(&node.table_name)
            .await?;

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
