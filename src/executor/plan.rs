//! Physical plan types for query execution

use crate::executor::aggregate::AggregateFunc;
use crate::executor::predicate::{ExpressionRef, PredicateRef};
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
    /// 全表扫描（via IndexManager.scan_all — M36 保留路径）
    Scan(ScanNode),
    /// 数据页直接扫描（M19 — 跳过索引层，每行 1 次页访问）
    DataScan(DataScanNode),
    /// 主键索引扫描
    IndexScan(IndexScanNode),
    /// 非唯一索引扫描（返回所有匹配行）
    IndexScanAll(IndexScanAllNode),
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
    /// Nested Loop Join 节点（MS09-T02 — 非等值/混合 ON）
    NestedLoopJoin(NestedLoopJoinNode),
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
    /// 投影表达式节点（MS11-T01 Iter001 — SELECT 派生列）
    Projection(ProjectionNode),
    /// 无 FROM 虚拟单行输入节点（MS13 T9 — no-FORM SELECT）
    ///
    /// 恰产出一行空行（`vec![]`），供其上的 ProjectionNode 逐项求值；无负载
    /// 字段（design D13）。列引用在 no-FORM 分支编译期即 ColumnNotFound，
    /// 空行不可达（安全性质）。
    SingleRow,
}

/// 全表扫描节点
#[derive(Debug, Clone)]
pub struct ScanNode {
    /// 表名
    pub table_name: String,
    /// 输出列名列表
    pub columns: Vec<String>,
    /// MS10-T01 Iter001: 输出投影（全 schema 列 index，按 SELECT 列表顺序）。
    /// 空 = 恒等（产出全 schema 行，行为与引入前逐字节一致）。
    pub projection: Vec<usize>,
}

/// 数据页直接扫描节点（M19）
///
/// Planner 在无 WHERE 或非 PK 等值 WHERE 场景下生成此节点，
/// 执行器直接遍历 `data_page_head` → `next_page_id` 链表，
/// 跳过 `IndexManager.scan_all` 索引层。
#[derive(Debug, Clone)]
pub struct DataScanNode {
    /// 表名
    pub table_name: String,
    /// 输出列名列表
    pub columns: Vec<String>,
    /// MS07-T06: 从 WHERE 下推的行内谓词（与 FilterExecutor 相同的全列行求值）。
    /// `None` = 无下推（无 WHERE，或谓词保留在 Filter 节点中）。
    pub predicate: Option<PredicateRef>,
    /// MS07-T06: 从 LIMIT 下推的扫描产出行封顶（`offset + limit`；`limit == 0`
    /// 时为 `Some(0)`）。`None` = 不封顶。
    pub scan_cap: Option<usize>,
    /// MS10-T01 Iter001: 输出投影（全 schema 列 index，按 SELECT 列表顺序）。
    /// 空 = 恒等。在行内谓词求值之后应用。
    pub projection: Vec<usize>,
}

/// 主键索引扫描节点
#[derive(Debug, Clone)]
pub struct IndexScanNode {
    /// 表名
    pub table_name: String,
    /// 主键值（用于 IndexManager.get()）
    pub key: Key,
    /// 输出列名列表（与 `projection` 一致的实际输出形状）
    pub columns: Vec<String>,
    /// MS10-T01 Iter001: 输出投影（空 = 恒等，产出全 schema 行）
    pub projection: Vec<usize>,
}

/// 非唯一索引扫描节点（返回所有匹配行）
#[derive(Debug, Clone)]
pub struct IndexScanAllNode {
    /// 表名
    pub table_name: String,
    /// 索引键值（用于 IndexManager.search_all()）
    pub key: Key,
    /// 输出列名列表（与 `projection` 一致的实际输出形状）
    pub columns: Vec<String>,
    /// MS10-T01 Iter001: 输出投影（空 = 恒等，产出全 schema 行）
    pub projection: Vec<usize>,
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
    /// MS10-T01 Iter001: 输出投影（全 schema 列 index）。空 = 恒等。
    /// 在谓词求值之后应用（谓词 `column_index` 按全 schema 解析）。
    pub projection: Vec<usize>,
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
            // MS13: 日期族直映射（固定宽度负载，无长度参数）
            ColumnType::Date => StorageColumnType::Date,
            ColumnType::Timestamp => StorageColumnType::Timestamp,
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
    /// 排序键查找所用行形状的列名（= 输入计划的真实输出形状）
    pub columns: Vec<String>,
    /// MS10-T01 Iter001: 输出投影（空 = 恒等）。Sort 在输入行形状上比较排序键、
    /// 产出时按投影裁剪（design D10：排序键允许不在投影内）。
    pub projection: Vec<usize>,
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

/// Nested Loop Join 节点（MS09-T02 — 非等值/混合 ON，plan 期启发式路由）
///
/// 对左输入每行 × 右输入每行的组合行（左行 ++ 右行）求值完整 ON 谓词，
/// 谓词为真（三值语义下非 Unknown 非假）的组合按 `output_columns` 产出。
/// 谓词 `column_index` 是组合行绝对索引（左表 0..n、右表 n..n+m）。
#[derive(Debug, Clone)]
pub struct NestedLoopJoinNode {
    /// 左输入计划
    pub left: Box<PhysicalPlan>,
    /// 右输入计划
    pub right: Box<PhysicalPlan>,
    /// 完整 ON 谓词（组合行布局上编译）
    pub predicate: PredicateRef,
    /// 输出列映射（与 JoinNode 同型）
    pub output_columns: Vec<OutputColumn>,
}

/// 聚合节点（GROUP BY + 聚合函数）
///
/// MS13 T8：`group_key_exprs` 为加性字段——分组键的求值表达式（列名键 =
/// `ColumnExpression`，与既有 `extract_group_key` 名字查索引语义逐字节
/// 一致；表达式/别名/位置键 = 对应 SELECT 项的编译表达式）。`group_by`
/// 保留键名清单（执行器 `is_empty` 判定与输出命名）。
#[derive(Debug, Clone)]
pub struct AggregateNode {
    pub input: Box<PhysicalPlan>,
    pub group_by: Vec<String>,
    pub group_key_exprs: Vec<ExpressionRef>,
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

/// 投影项（MS11-T01 Iter001 — SELECT 派生列）
#[derive(Debug, Clone)]
pub struct ProjectionItem {
    /// 逐行求值的值表达式（列引用项 = `ColumnExpression`，表达式项 =
    /// Iteration 000 的 `CaseExpression`/`CoalesceExpression`/`CastExpression` 等）
    pub expr: ExpressionRef,
    /// 输出列名（`AS` 别名或表达式 Display 文本）
    pub name: String,
}

/// 投影表达式节点（MS11-T01 Iter001 — SELECT 派生列）
///
/// SELECT 列表含表达式项时，planner 在最终 plan 最外层（LIMIT 之上）包装
/// 此节点：输入按全形状行流出（per-node 投影裁剪置空），执行器对每行逐项
/// 求值并按 `items` 顺序产出。
#[derive(Debug, Clone)]
pub struct ProjectionNode {
    /// 输入计划（产出全形状行）
    pub input: Box<PhysicalPlan>,
    /// SELECT 列表项（与输出列一一对应）
    pub items: Vec<ProjectionItem>,
    /// 输出列名（CLI 表头来源，`get_plan_output_columns` 直接返回）
    pub columns: Vec<String>,
}
