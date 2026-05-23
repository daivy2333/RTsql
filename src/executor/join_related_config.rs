use crate::database::Database;
use crate::executor::{CorrelatedParam, Executor, JoinCondition, OutputColumn, PhysicalPlan, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Anti/Semi-Join executor 配置参数集合
///
/// 用于解决 too_many_arguments warning，将 9个参数组织为单一结构体
pub struct JoinRelatedConfig {
    pub left: Box<dyn Executor + Send>,
    pub right: Box<dyn Executor + Send>,
    pub conditions: Vec<JoinCondition>,
    pub output_columns: Vec<OutputColumn>,
    pub correlated_params: Vec<CorrelatedParam>,
    pub left_column_indices: HashMap<String, usize>,
    pub right_column_indices: HashMap<String, usize>,
    pub right_plan: Option<PhysicalPlan>,
    pub database: Option<Arc<Database>>,
}