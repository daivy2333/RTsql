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

        // 转换 ColumnDef -> (name, type, not_null, unique, default)。约束经
        // to_schema_column 既有解析取得；PRIMARY KEY 不在此通道（planner 对
        // PK 列不产出 Unique 约束，pk 独立持久化于 catalog pk_column）。
        // MS24 Iteration 000 (D1)：DEFAULT 字面量经第 5 元素透传
        // create_table_with_constraints，persist 进 catalog 列行尾段。
        let columns: Vec<(
            String,
            crate::storage::page_format::ColumnType,
            bool,
            bool,
            Option<crate::executor::Value>,
        )> = node
            .columns
            .iter()
            .map(|col| {
                let schema_col = col.to_schema_column();
                let (name, col_type) = schema_col.to_tuple();
                (
                    name,
                    col_type,
                    schema_col.not_null,
                    schema_col.unique,
                    schema_col.default_value,
                )
            })
            .collect();

        // 确定主键列
        let pk = match &node.primary_key {
            Some(pk) => pk.clone(),
            None => {
                // 如果没有指定主键，使用第一列作为主键
                columns
                    .first()
                    .map(|(name, _, _, _, _)| name.clone())
                    .unwrap_or_else(|| "id".to_string())
            }
        };

        // 调用 TableManager::create_table_with_constraints
        self.database
            .table_manager
            .create_table_with_constraints(&node.table_name, columns, &pk)
            .await?;

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
