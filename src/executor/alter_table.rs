//! AlterTableExecutor — 执行 `ALTER TABLE`（MS21 Iter003 T3-b/c，design D9/D10）
//!
//! **ADD COLUMN**（D9）：catalog-first（`insert_column_row` + `column_count`
//! 递增，失败零副作用可重试）→ 内存 TableMeta 换入（index-aligned 向量尾部
//! 追加，树与页指针不动）。存量窄行由 T3-a 宽度感知读回层按声明 DEFAULT/NULL
//! 填充——无需数据重写。
//!
//! **DROP COLUMN**（D10）：单隐式事务逐行 DELETE+INSERT 重写（见 T3-c 实施
//! 记录）。
//!
//! 显式事务内调用在 pipeline 执行期拒绝（D13，0A000），不达此执行器。

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::page_format::ColumnType as StorageColumnType;
use crate::storage::Result;
use std::sync::Arc;

/// ALTER TABLE 执行器
pub struct AlterTableExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl AlterTableExecutor {
    /// 创建新的 AlterTableExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for AlterTableExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let node = match &self.plan {
            PhysicalPlan::AlterTable(node) => node,
            _ => panic!("AlterTableExecutor requires AlterTableNode"),
        };

        match &node.operation {
            crate::executor::AlterTableOp::AddColumn {
                column_name,
                column_type,
                not_null,
                default_value,
            } => {
                // planner ColumnType → storage ColumnType（to_schema_column
                // 同款映射：String 无宽度参数 → String(255)）
                let storage_type = match column_type {
                    crate::executor::ColumnType::Int => StorageColumnType::Int,
                    crate::executor::ColumnType::String => StorageColumnType::String(255),
                    crate::executor::ColumnType::Float => StorageColumnType::Float,
                    crate::executor::ColumnType::Bool => StorageColumnType::Bool,
                    crate::executor::ColumnType::Date => StorageColumnType::Date,
                    crate::executor::ColumnType::Timestamp => StorageColumnType::Timestamp,
                };
                self.database
                    .table_manager
                    .alter_add_column(
                        &node.table_name,
                        column_name,
                        storage_type,
                        *not_null,
                        default_value.clone(),
                    )
                    .await?;
                Ok(Some(ExecResult::AffectedRows(0)))
            }
            crate::executor::AlterTableOp::DropColumn { column_name } => {
                // 隐式事务（D10）：重写 DELETE/INSERT 全在一个事务内提交，
                // 崩溃恢复全有或全无。
                let tx = self.database.transaction_manager.begin().await;
                let tx_id = tx.id();
                let result = self
                    .database
                    .table_manager
                    .alter_drop_column(
                        &node.table_name,
                        column_name,
                        tx_id,
                        self.database.transaction_manager.clone(),
                        self.database.wal_buffer.clone(),
                    )
                    .await;
                match result {
                    Ok(()) => {
                        self.database
                            .transaction_manager
                            .commit(tx, &self.database.buffer_pool)
                            .await?;
                        Ok(Some(ExecResult::AffectedRows(0)))
                    }
                    Err(e) => {
                        let _ = self
                            .database
                            .transaction_manager
                            .abort(
                                tx,
                                &self.database.buffer_pool,
                                &std::collections::HashMap::new(),
                            )
                            .await;
                        Err(e)
                    }
                }
            }
        }
    }
}
