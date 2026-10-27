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
                // 加列放进独立任务。语句 future 只等结果通道；丢弃它会关掉接收端，
                // 任务继续做提交或回退，不随这次丢弃取消。
                let table_manager = std::sync::Arc::clone(&self.database.table_manager);
                let table_name = node.table_name.clone();
                let column_name = column_name.clone();
                let not_null = *not_null;
                let default_value = default_value.clone();
                let path = self.database.path().to_path_buf();
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tokio::spawn(async move {
                    let _ = table_manager
                        .alter_add_column(
                            &table_name,
                            &column_name,
                            storage_type,
                            not_null,
                            default_value,
                            &path,
                            tx,
                        )
                        .await;
                });
                match rx.await {
                    Ok(Ok(())) => Ok(Some(ExecResult::AffectedRows(0))),
                    Ok(Err(error)) => Err(error),
                    Err(_closed) => Err(crate::storage::StorageError::Internal(
                        "ADD COLUMN ended before reporting a result".to_string(),
                    )),
                }
            }
            crate::executor::AlterTableOp::DropColumn { column_name } => {
                // 隐式事务（D10）：重写 DELETE/INSERT 全在一个事务内提交，
                // 崩溃恢复全有或全无。
                //
                // MS34 Iter000（T01）：回收先于本语句的隐式事务——被放弃的
                // 上一条隐式语句必须在下一条隐式写语句开始之前完成回滚。
                self.database
                    .reclaim_abandoned_implicit_transactions()
                    .await
                    .map_err(|e| {
                        crate::storage::StorageError::ExecutionError(format!(
                            "Reclaim failed: {}",
                            e
                        ))
                    })?;
                let tx = self.database.transaction_manager.begin().await;
                let tx_id = tx.id();
                // MS34 Iter000（T01）：语句作用域守卫——语句 future 被放弃时
                // 守卫析构，tx id 记入登记表，由下一个触发点回收；正常提交或
                // 中止成功后显式解除。
                let mut guard = self.database.implicit_tx_guard(tx_id);
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
                    Ok(pending) => {
                        self.database
                            .transaction_manager
                            .commit(tx, &self.database.buffer_pool)
                            .await?;
                        // 库属隐式事务已提交 → 解除语句作用域守卫。
                        guard.disarm();
                        // change 2026-10-25-ms37-defect-ledger-closure
                        // Iteration 006 / Cycle 000（T6 / M5）：登记「已提交但
                        // 未发布」的窄化意图——checkpoint 据此**不得**让该意图
                        // 随 WAL 截断丢失（截断前先发布在途窄化）。
                        //
                        // 登记是**同步**的：与提交之间没有让出点 ⇒ 不出现「已
                        // 提交但未登记」的窗口。
                        self.database
                            .table_manager
                            .register_committed_narrowing(&pending);
                        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                        // Iteration 003（T10）：测试支撑注入点——布防时把语句停在
                        // 「提交完成 → 目录应用」窗口内（未布防 ⇒ 立即返回，零
                        // 行为变化）。窄化行此时已提交可见而目录仍是旧布局，
                        // ISS25 的错读窗口由此可被测试真实观测。
                        crate::executor::ddl_commit_window::pause_at_ddl_commit_window(
                            self.database.path(),
                        )
                        .await;
                        // ISS21 / D5：目录窄化在提交点之后生效——提交完成前
                        // 目录仍是旧布局，被放弃/失败的语句不留目录面变更。
                        self.database
                            .table_manager
                            .apply_column_drop(&pending)
                            .await?;
                        Ok(Some(ExecResult::AffectedRows(0)))
                    }
                    Err(e) => {
                        let aborted = self
                            .database
                            .transaction_manager
                            .abort(
                                tx,
                                &self.database.buffer_pool,
                                &std::collections::HashMap::new(),
                            )
                            .await
                            .is_ok();
                        // 中止失败时保留登记项——下一个触发点重试回收。
                        if aborted {
                            guard.disarm();
                        }
                        Err(e)
                    }
                }
            }
        }
    }
}
