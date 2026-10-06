use crate::executor::{Executor, JoinCondition, JoinSide, OutputColumn};
use std::collections::HashMap;

/// Join executor 配置参数集合
///
/// 用于解决 too_many_arguments warning，将 8个参数组织为单一结构体
pub struct JoinConfig {
    pub left_executor: Box<dyn Executor + Send>,
    pub right_executor: Box<dyn Executor + Send>,
    pub conditions: Vec<JoinCondition>,
    pub output_columns: Vec<OutputColumn>,
    /// 输出列输入侧归属（T9 replan design D9，与 output_columns 同序同长）
    pub output_sides: Vec<JoinSide>,
    pub left_column_indices: HashMap<String, usize>,
    pub right_column_indices: HashMap<String, usize>,
    pub right_table_name: String,
}
