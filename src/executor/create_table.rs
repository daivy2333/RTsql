//! CreateTableExecutor - 执行 CREATE TABLE 物理计划

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::{Result, StorageError};
use std::sync::Arc;

/// CREATE TABLE 执行器
pub struct CreateTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl CreateTableExecutor {
    /// 创建新的 CreateTableExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for CreateTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        // 提取 CreateTableNode
        let node = match &self.plan {
            PhysicalPlan::CreateTable(node) => node,
            _ => panic!("CreateTableExecutor requires CreateTableNode"),
        };

        // 检查表是否已存在
        if self.database.table_manager.table_exists(&node.table_name) {
            return Err(StorageError::TableAlreadyExists(node.table_name.clone()));
        }

        // 转换 ColumnDef -> ColumnSchema
        let columns: Vec<(String, crate::storage::page_format::ColumnType)> = node
            .columns
            .iter()
            .map(|col| {
                let schema_col = col.to_schema_column();
                schema_col.to_tuple()
            })
            .collect();

        // 确定主键列
        let pk = match &node.primary_key {
            Some(pk) => pk.clone(),
            None => {
                // 如果没有指定主键，使用第一列作为主键
                columns
                    .first()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| "id".to_string())
            }
        };

        // 调用 TableManager::create_table
        self.database
            .table_manager
            .create_table(&node.table_name, columns, &pk)
            .await?;

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
