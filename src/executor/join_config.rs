use crate::executor::Executor;

/// Join executor 配置参数集合
///
/// 用于解决 too_many_arguments warning，将 8-9 个参数组织为单一结构体
pub struct JoinConfig {
    pub left_source: Box<dyn Executor>,
    pub right_source: Box<dyn Executor>,
    pub left_key_column: usize,
    pub right_key_column: usize,
    pub output_columns: Vec<usize>,
    pub left_alias: Option<String>,
    pub right_alias: Option<String>,
}