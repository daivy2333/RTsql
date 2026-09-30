//! CreateViewExecutor — 执行 `CREATE VIEW`（MS21 Iter005 T5-b，design D12）
//!
//! 视图非物化：执行器只做「catalog 视图行 + 列名行持久化 + 内存注册表
//! 登记」，不分配任何数据页/索引树。重名/定义验证在计划期完成（TOCTOU
//! 窗口由执行器侧注册表复检闭合，create_table 先例）；catalog 持久化失败
//! 则登记未发生，零副作用可重试。

use crate::database::{Database, ViewMeta};
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::catalog::CatalogColumnRow;
use crate::storage::{ColumnType, Result, StorageError};
use std::sync::Arc;

/// CREATE VIEW 执行器
pub struct CreateViewExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl CreateViewExecutor {
    /// 创建新的 CreateViewExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for CreateViewExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let node = match &self.plan {
            PhysicalPlan::CreateView(node) => node,
            _ => panic!("CreateViewExecutor requires CreateViewNode"),
        };

        // TOCTOU 复检（create_table 先例）：plan 后、执行前注册表/表集合
        // 可能变化。跨基表与视图的名称唯一性在此双向闭合。
        if self
            .database
            .table_manager
            .get_table(&node.view_name)
            .await
            .is_ok()
            || self
                .database
                .views
                .read()
                .await
                .contains_key(&node.view_name)
        {
            return Err(StorageError::DuplicateTable(node.view_name.clone()));
        }

        // 列名行：视图列无声明类型/约束——`String(255)` 为占位（列名-only
        // 行；所有类型消费面——open_or_init / recovery / lifecycle——均已
        // 跳过 kind=View 行）。
        let column_rows: Vec<CatalogColumnRow> = node
            .columns
            .iter()
            .enumerate()
            .map(|(idx, name)| CatalogColumnRow {
                table_name: node.view_name.clone(),
                column_index: idx as u32,
                column_name: name.clone(),
                column_type: ColumnType::String(255),
                not_null: false,
                unique: false,
                default_value: None,
            })
            .collect();

        // 持久化（视图行 + 列名行；失败零副作用——注册表登记在其后）
        self.database
            .table_manager
            .catalog()
            .insert_view_row(&node.view_name, &node.sql, &column_rows)
            .await?;

        // 内存注册表登记
        self.database.views.write().await.insert(
            node.view_name.clone(),
            ViewMeta {
                sql: node.sql.clone(),
                columns: node.columns.clone(),
            },
        );

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
