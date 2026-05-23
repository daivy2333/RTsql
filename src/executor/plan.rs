//! Physical plan types for query execution

use crate::executor::aggregate::AggregateFunc;
use crate::executor::predicate::PredicateRef;
use crate::executor::{ColumnType, Value};
use crate::storage::page_format::Key;
use std::collections::HashMap;

/// 排序列定义
#[derive(Debug, Clone)]
pub struct OrderByColumn {
    pub column: String,
    pub asc: bool,
}

/// 物理计划节点（同步结构，M5 异步执行）
#[derive(Debug, Clone)]
pub enum PhysicalPlan {
    /// 全表扫描
    Scan(ScanNode),
    /// 主键索引扫描
    IndexScan(IndexScanNode),
    /// 过滤节点（WHERE 子句）
    Filter(FilterNode),
    /// 插入
    Insert(InsertNode),
    /// 更新
    Update(UpdateNode),
    /// 删除
    Delete(DeleteNode),
    /// 创建表
    CreateTable(CreateTableNode),
    /// 删除表
    DropTable(DropTableNode),
    /// 排序节点（ORDER BY）
    Sort(SortNode),
    /// 分页节点（LIMIT + OFFSET）
    Limit(LimitNode),
    /// JOIN 节点（INNER JOIN）
    Join(JoinNode),
    /// 聚合节点（GROUP BY + 聚合函数）
    Aggregate(AggregateNode),
    /// HAVING 过滤节点
    Having(HavingNode),
    /// Semi-Join 节点（IN/EXISTS 子查询反嵌套）
    SemiJoin(SemiJoinNode),
    /// Anti-Join 节点（NOT IN / NOT EXISTS 子查询反嵌套）
    AntiJoin(AntiJoinNode),
    /// 标量子查询求值节点
    SubqueryEval(SubqueryEvalNode),
    /// FROM 子查询（派生表）节点
    DerivedScan(DerivedScanNode),
}

/// 全表扫描节点
#[derive(Debug, Clone)]
pub struct ScanNode {
    /// 表名
    pub table_name: String,
    /// 输出列名列表
    pub columns: Vec<String>,
}

/// 主键索引扫描节点
#[derive(Debug, Clone)]
pub struct IndexScanNode {
    /// 表名
    pub table_name: String,
    /// 主键值（用于 IndexManager.get()）
    pub key: Key,
    /// 输出列名列表
    pub columns: Vec<String>,
}

/// 过滤节点（WHERE 子句求值）
#[derive(Debug, Clone)]
pub struct FilterNode {
    /// 输入计划（通常是 Scan）
    pub input: Box<PhysicalPlan>,
    /// 谓词（WHERE 条件）
    pub predicate: PredicateRef,
    /// 表名
    pub table_name: String,
}

/// 插入节点
#[derive(Debug, Clone)]
pub struct InsertNode {
    /// 表名
    pub table_name: String,
    /// 列名列表
    pub columns: Vec<String>,
    /// 值列表（每行一组值，支持批量插入）
    pub values: Vec<Vec<Value>>,
}

/// 更新节点（单行单列更新）
#[derive(Debug, Clone)]
pub struct UpdateNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）
    pub key: Key,
    /// 更新列名
    pub column: String,
    /// 新值
    pub new_value: Value,
}

/// 删除节点
#[derive(Debug, Clone)]
pub struct DeleteNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）
    pub key: Key,
}

/// 列定义
#[derive(Debug, Clone)]
pub struct ColumnDef {
    /// 列名
    pub name: String,
    /// 列类型
    pub data_type: ColumnType,
    /// 列约束
    pub constraints: Vec<ColumnConstraint>,
}

impl ColumnDef {
    /// 创建新的列定义
    pub fn new(name: String, data_type: ColumnType) -> Self {
        Self {
            name,
            data_type,
            constraints: Vec::new(),
        }
    }

    /// 添加约束
    pub fn with_constraint(mut self, constraint: ColumnConstraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// 转换为 storage::ColumnSchema
    pub fn to_schema_column(&self) -> crate::storage::data::ColumnSchema {
        use crate::storage::page_format::ColumnType as StorageColumnType;

        // 转换 executor::ColumnType -> storage::ColumnType
        let storage_type = match &self.data_type {
            ColumnType::Int => StorageColumnType::Int,
            ColumnType::String => StorageColumnType::String(255), // 默认长度 255
            ColumnType::Float => StorageColumnType::Float,
            ColumnType::Bool => StorageColumnType::Bool,
        };

        // 解析约束
        let mut not_null = false;
        let mut unique = false;
        let mut default_value = None;

        for constraint in &self.constraints {
            match constraint {
                ColumnConstraint::NotNull => not_null = true,
                ColumnConstraint::Unique => unique = true,
                ColumnConstraint::DefaultValue(v) => default_value = Some(v.clone()),
            }
        }

        crate::storage::data::ColumnSchema {
            name: self.name.clone(),
            data_type: storage_type,
            not_null,
            unique,
            default_value,
        }
    }
}

/// 列约束
#[derive(Debug, Clone)]
pub enum ColumnConstraint {
    /// 非空约束
    NotNull,
    /// 唯一约束
    Unique,
    /// 默认值
    DefaultValue(Value),
}

/// 创建表节点
#[derive(Debug, Clone)]
pub struct CreateTableNode {
    /// 表名
    pub table_name: String,
    /// 列定义列表
    pub columns: Vec<ColumnDef>,
    /// 主键列名
    pub primary_key: Option<String>,
}

/// 删除表节点
#[derive(Debug, Clone)]
pub struct DropTableNode {
    /// 表名
    pub table_name: String,
    /// 是否使用 IF EXISTS
    pub if_exists: bool,
}

/// 排序节点（ORDER BY）
#[derive(Debug, Clone)]
pub struct SortNode {
    pub input: Box<PhysicalPlan>,
    pub order_by: Vec<OrderByColumn>,
    pub table_name: String,
    pub columns: Vec<String>,
}

/// 分页节点（LIMIT + OFFSET）
#[derive(Debug, Clone)]
pub struct LimitNode {
    pub input: Box<PhysicalPlan>,
    pub limit: usize,
    pub offset: usize,
}

/// JOIN 条件（等值连接）
#[derive(Debug, Clone)]
pub struct JoinCondition {
    /// 左表列引用
    pub left_column: ColumnRef,
    /// 右表列引用
    pub right_column: ColumnRef,
}

/// 列引用（支持 t.col 格式）
#[derive(Debug, Clone)]
pub struct ColumnRef {
    /// 表名（可选，t.col 格式时为 Some）
    pub table: Option<String>,
    /// 列名
    pub column: String,
}

/// 输出列定义
#[derive(Debug, Clone)]
pub struct OutputColumn {
    /// 表名（可选）
    pub table: Option<String>,
    /// 列名
    pub column: String,
    /// 实际表名（解析后确定）
    pub table_alias: String,
    /// 在源表中的列索引
    pub column_index: usize,
}

/// JOIN 节点（INNER JOIN）
#[derive(Debug, Clone)]
pub struct JoinNode {
    /// 左表计划（可以是 Scan 或另一个 Join）
    pub left: Box<PhysicalPlan>,
    /// 右表计划（必须是 Scan）
    pub right: Box<PhysicalPlan>,
    /// ON 等值条件列表（AND 组合）
    pub conditions: Vec<JoinCondition>,
    /// 输出列映射
    pub output_columns: Vec<OutputColumn>,
}

/// 聚合节点（GROUP BY + 聚合函数）
#[derive(Debug, Clone)]
pub struct AggregateNode {
    pub input: Box<PhysicalPlan>,
    pub group_by: Vec<String>,
    pub aggregates: Vec<AggregateFunc>,
    pub output_columns: Vec<String>,
    pub table_name: String,
    pub column_indices: HashMap<String, usize>,
}

/// HAVING 过滤节点
#[derive(Debug, Clone)]
pub struct HavingNode {
    pub input: Box<PhysicalPlan>,
    pub predicate: PredicateRef,
    pub table_name: String,
}

/// 相关子查询参数（外层列 → 内层替换位置）
#[derive(Debug, Clone)]
pub struct CorrelatedParam {
    /// 外层表名
    pub outer_table: String,
    /// 外层列名
    pub outer_column: String,
    /// 限定参数名（如 "emp.dept"），匹配 ParameterExpression::param_name
    pub param_name: String,
}

impl CorrelatedParam {
    pub fn new(outer_table: String, outer_column: String, param_name: String) -> Self {
        Self {
            outer_table,
            outer_column,
            param_name,
        }
    }
}

/// Semi-Join 节点（IN/EXISTS 子查询反嵌套）
/// 输出仅包含左表行，当左表行在右表中有匹配时输出
#[derive(Debug, Clone)]
pub struct SemiJoinNode {
    /// 左表计划（外层查询）
    pub left: Box<PhysicalPlan>,
    /// 右表计划（子查询物化结果）
    pub right: Box<PhysicalPlan>,
    /// 等值条件（左表列 = 子查询结果列）
    /// EXISTS 子查询时 conditions 为空，仅检测右表非空
    pub conditions: Vec<JoinCondition>,
    /// 输出列（仅左表列）
    pub output_columns: Vec<OutputColumn>,
    /// 相关子查询参数（空 = 独立子查询）
    pub correlated_params: Vec<CorrelatedParam>,
}

/// Anti-Join 节点（NOT IN / NOT EXISTS 子查询反嵌套）
/// 输出仅包含左表行，当左表行在右表中无匹配时输出
#[derive(Debug, Clone)]
pub struct AntiJoinNode {
    /// 左表计划（外层查询）
    pub left: Box<PhysicalPlan>,
    /// 右表计划（子查询物化结果）
    pub right: Box<PhysicalPlan>,
    /// 等值条件
    pub conditions: Vec<JoinCondition>,
    /// 输出列（仅左表列）
    pub output_columns: Vec<OutputColumn>,
    /// 相关子查询参数
    pub correlated_params: Vec<CorrelatedParam>,
}

/// 标量子查询求值节点（SELECT 列中的子查询）
/// 逐行对外层行执行子查询，取首行首列作为标量结果追加到输出
#[derive(Debug, Clone)]
pub struct SubqueryEvalNode {
    /// 外层查询输入
    pub input: Box<PhysicalPlan>,
    /// 子查询计划（标量子查询）
    pub subquery: Box<PhysicalPlan>,
    /// 结果列名
    pub output_column: String,
    /// 子查询结果在输出行中的列索引
    pub result_column_index: usize,
    /// 相关子查询参数
    pub correlated_params: Vec<CorrelatedParam>,
}

/// FROM 子查询（派生表）节点
/// 将子查询结果物化为内存表，作为 Scan 数据源
#[derive(Debug, Clone)]
pub struct DerivedScanNode {
    /// 子查询计划
    pub subquery: Box<PhysicalPlan>,
    /// 派生表别名
    pub alias: String,
    /// 输出列名列表
    pub columns: Vec<String>,
}
