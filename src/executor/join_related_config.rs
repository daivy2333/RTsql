use crate::database::Database;
use crate::executor::{CorrelatedParam, Executor, JoinCondition, PhysicalPlan};
use crate::transaction::Snapshot;
use std::collections::HashMap;
use std::sync::Arc;

/// Anti/Semi-Join executor 配置参数集合
///
/// 用于解决 too_many_arguments warning，将 9个参数组织为单一结构体
pub struct JoinRelatedConfig {
    pub left: Box<dyn Executor + Send>,
    pub right: Box<dyn Executor + Send>,
    pub conditions: Vec<JoinCondition>,
    /// T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：SELECT
    /// 清单裁剪位（空 = 恒等全宽输出）。
    pub projection: Vec<usize>,
    pub correlated_params: Vec<CorrelatedParam>,
    pub left_column_indices: HashMap<String, usize>,
    pub right_column_indices: HashMap<String, usize>,
    pub right_plan: Option<PhysicalPlan>,
    pub database: Option<Arc<Database>>,
    /// MS09 Iter000 (D4): statement-start Read Committed snapshot, re-applied
    /// on every correlated right-plan rebuild (`None` under RepeatableRead).
    pub snapshot: Option<Snapshot>,
    /// MS20 T2: statement-level cross-db route table (qualified flat name ->
    /// attached `Database`), re-applied on every correlated right-plan rebuild.
    /// Empty for the lib path and for single-database sessions, which keeps
    /// the rebuild path byte-for-byte unchanged.
    pub routed_tables: Arc<crate::pipeline::RoutedTables>,
}
