//! TruncateExecutor — 执行 `TRUNCATE TABLE`（MS21 Iter004 T4-a，design D11）
//!
//! 执行序（checkpoint-first）：
//! 1. **先 `Database::checkpoint()`**——刷脏页 + WAL 截断到位点，此后 WAL 无
//!    任何待重放的 pre-TRUNCATE DML，物理清除后行不可能经重放复活（MS18 零
//!    工作守卫使无未决 WAL/脏页时该调用为无操作，两态皆安全）；
//! 2. `TableManager::truncate_table`——尾页 free-list 释放 + head 原地重置 +
//!    tail 回写 + PK/唯一/二级三类空树换入且根写回 catalog + 旧树释放 +
//!    `flush_all()` 收尾（语句返回即持久）。
//!
//! 崩溃窗口分析（D11，与 `truncate_table` doc 一致）：checkpoint 前崩溃 → 旧
//! 状态完整（语句未生效）；清除开始至 `flush_all()` 前崩溃 → 「部分清除」诚实
//! 化 DDL 状态（数据丢失但 schema/索引结构完整，drop_table 崩溃窗口同类）；
//! 不存在「复活已清行」形态。显式事务内在 pipeline 执行期拒绝（D13，0A000），
//! 不达此执行器。

use crate::database::Database;
use crate::executor::{ExecResult, Executor, PhysicalPlan};
use crate::storage::Result;
use std::sync::Arc;

/// TRUNCATE TABLE 执行器
pub struct TruncateExecutor {
    plan: PhysicalPlan,
    database: Arc<Database>,
    executed: bool,
}

impl TruncateExecutor {
    /// 创建新的 TruncateExecutor
    pub fn new(plan: PhysicalPlan, database: Arc<Database>) -> Self {
        Self {
            plan,
            database,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for TruncateExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let node = match &self.plan {
            PhysicalPlan::Truncate(node) => node,
            _ => panic!("TruncateExecutor requires TruncateNode"),
        };

        // 步骤 1：checkpoint-first——截断 WAL，封死行级复活通道（D11 步骤 2）。
        self.database.checkpoint().await?;

        // 步骤 2：物理清除 + 三类索引根重置 + flush_all 收尾（D11 步骤 3–5）。
        self.database
            .table_manager
            .truncate_table(&node.table_name)
            .await?;

        Ok(Some(ExecResult::AffectedRows(0)))
    }
}
