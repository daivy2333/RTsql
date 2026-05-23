use crate::executor::{Executor, JoinCondition, OutputColumn, Value};
use std::collections::HashMap;

/// Join executor 配置参数集合
///
/// 用于解决 too_many_arguments warning，将 8个参数组织为单一结构体
pub struct JoinConfig {
    pub left_executor: Box<dyn Executor + Send>,
    pub right_executor: Box<dyn Executor + Send>,
    pub conditions: Vec<JoinCondition>,
    pub output_columns: Vec<OutputColumn>,
    pub left_column_indices: HashMap<String, usize>,
    pub right_column_indices: HashMap<String, usize>,
    pub left_table_name: String,
    pub right_table_name: String,
}