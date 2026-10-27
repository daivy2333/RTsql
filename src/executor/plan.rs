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
    /// MS25 T7（design D8）：输出别名键的求值表达式。`Some` = 该键按顶层
    /// 投影输出名解析（decorate-sort-undecorate：`SortExecutor` 在未投影的
    /// 输入行上求值后按键排序）；`None` = 既有列名键（按 `columns` 位置取值）。
    /// 复合语句尾 ORDER BY 与聚合查询的别名键维持列名键。
    pub expr: Option<ExpressionRef>,
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
    /// 插入或冲突消解（MS24 Iter001：`ON CONFLICT DO NOTHING/DO UPDATE`、
    /// `REPLACE INTO`）
    Upsert(UpsertNode),
    /// `INSERT INTO ... SELECT`（MS20-T02 Iter001 design D4）：源 SELECT
    /// 子树在执行面完整物化为原生 Value 行，按位置映射拼装全宽目标行后
    /// 归一为既有 `Insert`/`Upsert` 计划进入写路径。
    InsertSelect(InsertSelectNode),
    /// 更新
    Update(UpdateNode),
    /// 删除
    Delete(DeleteNode),
    /// 创建表
    CreateTable(CreateTableNode),
    /// CTAS：`CREATE TABLE <name> AS <SELECT>`（MS20 T4）
    CreateTableAsSelect(CreateTableAsSelectNode),
    /// 创建二级索引（MS21 Iter002 T2-c）：建空树 → 存量行回填 → catalog 登记
    CreateIndex(CreateIndexNode),
    /// 表结构演进（MS21 Iter003 T3-b/c）：尾部加列 / 删列重写
    AlterTable(AlterTableNode),
    /// 删除二级索引（MS21 Iter002 T2-f）：登记移除 + 树页释放
    DropIndex(DropIndexNode),
    /// TRUNCATE TABLE（MS21 Iter004 T4-a）：物理清空保留定义
    Truncate(TruncateNode),
    /// 创建视图（MS21 Iter005 T5-b）：持久化定义 + 内存注册表登记；
    /// 非物化——无数据页/索引树
    CreateView(CreateViewNode),
    /// 删除视图（MS21 Iter005 T5-d）：catalog 视图行/列行删除 + 注册表移除
    DropView(DropViewNode),
    /// 删除表
    DropTable(DropTableNode),
    /// 排序节点（ORDER BY）
    Sort(SortNode),
    /// 分页节点（LIMIT + OFFSET）
    Limit(LimitNode),
    /// JOIN 节点（INNER JOIN）
    Join(JoinNode),
    /// 排序合并连接（等值内连接）。字段与 `JoinNode` 相同。SQL 计划路径本阶段仍发出 `Join`。
    SortMergeJoin(JoinNode),
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
    /// WHERE 比较腿标量子查询过滤节点（T4 — 不下推的占位谓词载体）
    ScalarSubqueryFilter(ScalarSubqueryFilterNode),
    /// FROM 子查询（派生表）节点
    DerivedScan(DerivedScanNode),
    /// 投影表达式节点（MS11-T01 Iter001 — SELECT 派生列）
    Projection(ProjectionNode),
    /// 去重节点（MS25 T2 — `SELECT DISTINCT`，作用于已就绪的输出行形状）
    Distinct(DistinctNode),
    /// 集合操作节点（MS25 T3 — `UNION [ALL]` / `INTERSECT` / `EXCEPT`）
    SetOperation(SetOperationNode),
    /// 计划树文本输出节点（MS25 T6 — `EXPLAIN <SELECT>`）
    ///
    /// 仅包装被观测的内层计划：pipeline 构造臂**不构造内层执行器**（派生表急切
    /// 物化与任何执行器副作用都不触发），文本由 `plan_format::render_plan_tree`
    /// 纯函数渲染。
    Explain(ExplainNode),
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
    /// MS21 Iter002 (T2-g/D6)：命中的二级索引名。`None` = PK/唯一树上的既有
    /// 全扫语义（构造路径逐字节不变）；`Some` 时执行器消费该登记树
    /// （pipeline 按 `TableMeta.secondary_indexes` 名称解析）。
    pub index_name: Option<String>,
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
    /// 值位参数槽（MS31 Iteration 001）：待插值位上的参数占位——计划期该位
    /// 置 `Value::Null` 占位，执行期由 pipeline 按参数物化真实值。
    pub param_slots: Vec<ValueParamSlot>,
}

/// 值位参数槽（MS31 Iteration 001 / design D5）。
///
/// 记录「行位 × 列位 → 参数序号」的映射：`values[row][col]` 在计划期是
/// `Value::Null` 占位，pipeline 在构造执行器前按 `param_index` 取参数值
/// 物化到该位——类型门 / NOT NULL / 键列门 / 索引维护全部沿用执行器既有
/// 语义（在真实值上跑，与字面量语句同源）。
#[derive(Debug, Clone, PartialEq)]
pub struct ValueParamSlot {
    /// 行位（VALUES 行序）。
    pub row: usize,
    /// 列位（全宽行内的表列位）。
    pub col: usize,
    /// 参数序号（`__param_{i}` 的 `i`）。
    pub param_index: usize,
}

/// 冲突仲裁目标（MS24 Iter001 design D4）
#[derive(Debug, Clone)]
pub enum ConflictArbiter {
    /// 省略冲突目标：仲裁 PK + 全部唯一索引（SQLite 语义，PK 先、唯一列按序）
    All,
    /// 显式单列目标：该列必须存在唯一性索引可仲裁（INT 声明 PK 或唯一索引列）
    Column(usize),
}

/// DO UPDATE 赋值的右值形态（MS24 Iter001 design D4）
#[derive(Debug, Clone)]
pub enum UpsertValueExpr {
    /// 字面量（`DEFAULT` 关键字已在计划期字面化为该列声明默认值或 NULL）
    Literal(Value),
    /// `excluded.col`——本行待插值（已 coerce/升格）
    Excluded(usize),
    /// 裸列名——冲突行的旧值
    Old(usize),
    /// 参数位（MS31 Iteration 001）：`DO UPDATE SET` 右值位上的参数占位——
    /// 计划期记录参数序号，执行期由 pipeline 物化为字面量。
    Param(usize),
}

/// DO UPDATE 的单列赋值（MS24 Iter001 design D4）
#[derive(Debug, Clone)]
pub struct UpsertAssignment {
    /// 目标列位
    pub column: usize,
    /// 右值形态
    pub expr: UpsertValueExpr,
}

/// 冲突动作（MS24 Iter001 design D4）
#[derive(Debug, Clone)]
pub enum ConflictAction {
    /// `DO NOTHING`：冲突行跳过，不计入受影响行数
    DoNothing,
    /// `DO UPDATE SET ...`：冲突行原位更新
    DoUpdate(Vec<UpsertAssignment>),
    /// `REPLACE INTO`：冲突行删除后重插
    Replace,
}

/// 插入或冲突消解节点（MS24 Iter001 design D4）
#[derive(Debug, Clone)]
pub struct UpsertNode {
    /// 表名
    pub table_name: String,
    /// 值列表（D2 全宽填充后的行，与 `InsertNode.values` 同形）
    pub values: Vec<Vec<Value>>,
    /// 值位参数槽（MS31 Iteration 001）：待插值位上的参数占位——同
    /// `InsertNode.param_slots`。
    pub param_slots: Vec<ValueParamSlot>,
    /// 冲突仲裁目标
    pub arbiter: ConflictArbiter,
    /// 冲突动作
    pub action: ConflictAction,
}

/// `INSERT ... SELECT` 节点（MS20-T02 Iter001 design D4）
///
/// 源 SELECT 在计划期以**顶层上下文**编译为子树（WITH/DISTINCT/JOIN/视图/
/// 集合操作全量消费，不误用 `building_subquery` 抑制分支）。执行面先把源
/// 完整物化为原生 Value 行，再按 `target_positions`/`fill` 逐行拼装全宽
/// 目标行，归一为既有 `Insert`/`Upsert` 计划——类型/NOT NULL/PK/UNIQUE/
/// 二级索引/冲突动作语义全部沿用既有写面，不经 JSON 或 SQL 文本往返。
#[derive(Debug, Clone)]
pub struct InsertSelectNode {
    /// 目标表名（限定扁平名，与 `InsertNode`/`UpsertNode` 同形）
    pub table_name: String,
    /// 语句列清单（小写；空 = 未写清单，按表列位全量接收）
    pub columns: Vec<String>,
    /// 源 SELECT 计划子树（顶层上下文编译）
    pub source: Box<PhysicalPlan>,
    /// 源输出宽度（计划期判定——空源同样校验，结构验证不依赖数据行）
    pub source_width: usize,
    /// 清单→表列位置映射（`columns[i]` 的目标列位）；未写清单时为恒等映射
    pub target_positions: Vec<usize>,
    /// 全宽填充值（表列宽：声明 DEFAULT 或 NULL）；清单省略位的落点值。
    /// 未写清单时恒等映射覆盖全部位置，本字段不被消费。
    pub fill: Vec<Value>,
    /// 可选冲突仲裁/动作（`ON CONFLICT` / `REPLACE INTO` 复用既有解析）
    pub upsert: Option<(ConflictArbiter, ConflictAction)>,
}

/// UPDATE SET 右值形态（change 2026-10-03-practical-core-gaps T8）
#[derive(Debug, Clone)]
pub enum UpdateValue {
    /// 字面量（既有语义逐字节：含 TypedString 日期解析与 NULL 标识符）
    Literal(Value),
    /// 值表达式（列引用 / 算术 / CAST / CASE / COALESCE / 标量函数——执行器
    /// 对命中行旧行快照求值，见 `UpdateExecutor`）
    Expression(ExpressionRef),
}

/// 字面量按值相等；表达式臂不可比较（计划期形状断言用 `matches!`）。
impl PartialEq for UpdateValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UpdateValue::Literal(a), UpdateValue::Literal(b)) => a == b,
            _ => false,
        }
    }
}

/// UPDATE 的单列赋值（change 2026-10-03-practical-core-gaps T8）
#[derive(Debug, Clone)]
pub struct UpdateAssignment {
    /// 目标列名（小写归一）
    pub column: String,
    /// 右值形态
    pub value: UpdateValue,
}

/// 更新节点（change 2026-10-03-practical-core-gaps T8：多列赋值 + 值表达式
/// 右值；定位行语义不变——主键等值 WHERE。change 2026-10-04-external-review-gaps
/// T6：`key` 为 `None` 时为无 WHERE 全表形态——执行器枚举全部索引键逐行应用）。
#[derive(Debug, Clone)]
pub struct UpdateNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）；`None` ＝ 无 WHERE 全表形态
    pub key: Option<Key>,
    /// 赋值清单（语句序）
    pub assignments: Vec<UpdateAssignment>,
}

/// 删除节点（T6：`key` 为 `None` 时为无 WHERE 全表形态）
#[derive(Debug, Clone)]
pub struct DeleteNode {
    /// 表名
    pub table_name: String,
    /// 主键（定位行）；`None` ＝ 无 WHERE 全表形态
    pub key: Option<Key>,
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

/// CTAS 节点（MS20 T4）：`CREATE TABLE <name> AS <SELECT>` 物化到主库
#[derive(Debug, Clone)]
pub struct CreateTableAsSelectNode {
    /// 主库目标表名（扁平名；附加库目标已在计划期拒绝）
    pub table_name: String,
    /// 物化源 SELECT 计划
    pub select: Box<PhysicalPlan>,
    /// SELECT 输出列名（= 物化表列名，序同输出形状）
    pub columns: Vec<String>,
}

/// 删除表节点
#[derive(Debug, Clone)]
pub struct DropTableNode {
    /// 表名
    pub table_name: String,
    /// 是否使用 IF EXISTS
    pub if_exists: bool,
}

/// 创建二级索引节点（MS21 Iter002 T2-c）
#[derive(Debug, Clone)]
pub struct CreateIndexNode {
    /// 索引名（小写归一，库级唯一）
    pub index_name: String,
    /// 目标表名
    pub table_name: String,
    /// 索引第一列（表列序内位置）
    pub column_index: usize,
    /// 索引第一列名（EXPLAIN 参数直出）
    pub column_name: String,
    /// 键编码世代。单列字符串为 0，新建单列整数和多列为 1。
    pub generation: u8,
    /// 第一列之后的列号，顺序与索引定义一致。
    pub extra_column_indexes: Vec<usize>,
}

/// ALTER TABLE 操作（MS21 Iter003 T3-b/c）——计划期白名单后的两种形态
#[derive(Debug, Clone)]
pub enum AlterTableOp {
    /// `ADD COLUMN <col> <type> [NOT NULL] [DEFAULT <literal>]`（尾部加列）；
    /// 类型为 planner 层 ColumnType（CREATE TABLE 先例——storage 转换在
    /// 执行器侧，String → String(255)）
    AddColumn {
        column_name: String,
        column_type: ColumnType,
        not_null: bool,
        default_value: Option<Value>,
    },
    /// `DROP COLUMN <col>`（单隐式事务逐行重写）
    DropColumn { column_name: String },
}

/// ALTER TABLE 节点（MS21 Iter003 T3-b/c）
#[derive(Debug, Clone)]
pub struct AlterTableNode {
    /// 目标表名（小写归一）
    pub table_name: String,
    /// 已校验的单操作
    pub operation: AlterTableOp,
}

/// 删除二级索引节点（MS21 Iter002 T2-f）
#[derive(Debug, Clone)]
pub struct DropIndexNode {
    /// 索引名（小写归一）
    pub index_name: String,
}

/// TRUNCATE TABLE 节点（MS21 Iter004 T4-a）：物理清空保留定义——数据页经
/// free-list 释放、head 原地重置、PK/唯一/二级三类索引重建为空树，表与
/// 索引定义完整保留（design D11 checkpoint-first 序，物理语义无行级痕迹）。
#[derive(Debug, Clone)]
pub struct TruncateNode {
    /// 目标表名（小写归一；缺失/附加库/系统保留名已在计划期拒绝）
    pub table_name: String,
}

/// 创建视图节点（MS21 Iter005 T5-b/D12）
#[derive(Debug, Clone)]
pub struct CreateViewNode {
    /// 视图名（小写归一；库级唯一——跨基表与视图）
    pub view_name: String,
    /// 定义 SQL 文本（`Query::to_string()` 规范化形态；展开期 `parse_sql`
    /// 重解析）
    pub sql: String,
    /// 视图列名清单（显式清单或定义输出表头，计划期定形）
    pub columns: Vec<String>,
}

/// 删除视图节点（MS21 Iter005 T5-d/D12）
#[derive(Debug, Clone)]
pub struct DropViewNode {
    /// 视图名（小写归一；存在性已在计划期拒绝）
    pub view_name: String,
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
    /// MS32 Iter002（design D3）：LIMIT 直接约束排序时的驻留上限
    /// （`offset + limit`）。`None` = 全量排序；仅计划期两处 Limit 包装点
    /// 在 `Sort` / `Projection(Sort)` 形状上赋值，其余形状维持 `None`
    /// （Distinct、聚合、集合运算等全量排序语义不变）。渲染不输出本字段。
    pub bounded: Option<usize>,
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

/// 输出列的输入侧归属（T9 replan，design D9）——执行器输入项语境：`Left`
/// = 从执行器左输入行取值，`Right` = 右输入行。与 `output_columns` 同序同
/// 长；计划期装配（Hash 直用、NLJ RIGHT 交换反转），执行器不再按表名猜测
/// 归属——自连接两侧真实名相同使名字比较恒读一侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinSide {
    /// 取执行器左输入行
    Left,
    /// 取执行器右输入行
    Right,
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
    /// 输出列输入侧归属（与 `output_columns` 同序同长）
    pub output_sides: Vec<JoinSide>,
}

/// Nested Loop Join 语义类型（MS25 T1 — design D1 加性字段）
///
/// `Inner` 为既有形态（仅产出匹配组合）。`Left` 为左外连接：左行无匹配时
/// 产出该左行并以 NULL 补齐右侧列。`Full` 为全外连接：两侧都补齐。`RIGHT
/// JOIN` 由 planner 交换左右输入后按 `Left` 承载（左输入 = 原右表，未匹配的
/// 原左行丢弃），故本枚举不设 `Right` 变体——计划期的左右交换在节点的
/// `left`/`right` 上可见。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NestedLoopJoinType {
    /// INNER JOIN（既有形态）
    #[default]
    Inner,
    /// LEFT OUTER JOIN
    Left,
    /// FULL OUTER JOIN（两侧都补齐）
    Full,
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
    /// 输出列输入侧归属（与 `output_columns` 同序同长；RIGHT 交换由计划期
    /// 反转，执行器侧只见输入项语境）
    pub output_sides: Vec<JoinSide>,
    /// MS25 T1：连接语义（`Inner` = 变更前逐字节一致的既有形态）
    pub join_type: NestedLoopJoinType,
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
    /// MS25 T8（design D9）：与 `aggregates` 同序的参数列输入行索引。`Some`
    /// 由计划期在 JOIN 基座的组合布局上解析（`column_indices` 的按名折叠不
    /// 参与决策）；`None` = 无参数（`COUNT(*)`）或单表面——后者沿既有按名
    /// 查找语义逐字节不变。
    pub aggregate_arg_indices: Vec<Option<usize>>,
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
    /// T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：SELECT
    /// 清单裁剪位（基表全宽行上的索引；空 = 恒等全宽输出——ORDER BY 存在时
    /// 裁剪归 Sort 承担）。R5：基座全宽承载，排序键可见非输出列。
    pub projection: Vec<usize>,
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
    /// T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：SELECT
    /// 清单裁剪位（同 SemiJoinNode.projection）。
    pub projection: Vec<usize>,
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
    /// T4（change 2026-10-04-external-review-gaps）：输出收窄宽度——相关
    /// 参数取值列以尾部附加形式保留在输入行中（SELECT 投影之外的补充列），
    /// 本链最外层节点在参数提取后按此宽度截断，使补充列不泄漏过节点。
    /// 0 = 不截断（恒等，引入前行为）。
    pub output_width: usize,
}

/// WHERE 比较腿标量子查询绑定（T4，change 2026-10-04-external-review-gaps）
///
/// 一条标量子查询与其谓词占位的绑定：`param_name` 即谓词树中
/// `ParameterExpression` 的名字（保留名 `__scalar_subquery_<n>`），执行器
/// 逐行求值子查询后按名注入标量结果。
#[derive(Debug, Clone)]
pub struct ScalarSubqueryBinding {
    /// 谓词占位参数名（`__scalar_subquery_<n>`）
    pub param_name: String,
    /// 子查询计划（标量输出）
    pub subquery: Box<PhysicalPlan>,
    /// 相关子查询参数（子查询 WHERE 引用外层列）
    pub correlated_params: Vec<CorrelatedParam>,
}

/// WHERE 标量子查询过滤节点（T4，change 2026-10-04-external-review-gaps）
///
/// WHERE 含标量子查询比较腿时的谓词载体：不下推进扫描节点（占位求值需
/// 逐行注入子查询结果），由执行器对全形状输入行逐行求值各绑定子查询 →
/// 按名注入谓词 → 求值，再按 `projection` 裁剪输出。`columns` 为裁剪后的
/// 实际输出列名（IndexScan「已收窄」契约同型——表头与行形状的唯一真源）。
#[derive(Debug, Clone)]
pub struct ScalarSubqueryFilterNode {
    /// 输入计划（产出全形状行；谓词 `column_index` 按其全 schema 解析）
    pub input: Box<PhysicalPlan>,
    /// 谓词（含 `__scalar_subquery_<n>` 占位）
    pub predicate: PredicateRef,
    /// 标量子查询绑定（占位名 → 子查询计划 + 相关参数）
    pub bindings: Vec<ScalarSubqueryBinding>,
    /// 裁剪后的实际输出列名（`projection` 应用后的行形状）
    pub columns: Vec<String>,
    /// 表名（输入链的表名，EXPLAIN/诊断用）
    pub table_name: String,
    /// 输出投影（全 schema 列 index，空 = 恒等），谓词求值后应用
    pub projection: Vec<usize>,
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
    /// MS26 T8：外层 SELECT 清单解析到的派生列索引（投影裁剪）。
    /// 语义与 `ScanNode`/`DataScanNode.projection` 同型——对物化行按索引
    /// 裁剪；空 = 恒等（`SELECT *` / 聚合输入 / 表达式项 / ORDER BY 下
    /// 由 Sort 承担裁剪的形态）。与 `columns` 共同描述输出形状。
    pub projection: Vec<usize>,
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

/// 去重节点（MS25 T2 — `SELECT DISTINCT`）
///
/// 插入位置在「输出行形状已就绪」处：纯列清单在 `Sort` 之下即输出形状，
/// 表达式项在顶层 `Projection` 之上。行形状与输出列名与输入一致（本节点不
/// 改列）。
#[derive(Debug, Clone)]
pub struct DistinctNode {
    /// 输入计划（产出待去重的输出行）
    pub input: Box<PhysicalPlan>,
}

/// 集合操作种类（MS25 T3）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOpKind {
    /// `UNION` / `UNION ALL`（去重由节点的 `all` 标志切换）
    Union,
    /// `INTERSECT`（去重交集）
    Intersect,
    /// `EXCEPT`（去重差集）
    Except,
}

/// 集合操作节点（MS25 T3 — `UNION [ALL]` / `INTERSECT` / `EXCEPT`）
///
/// 两侧计划各自产出其分支的输出行（列数已由 planner 校验相等）；复合输出列名
/// 取左侧列名。`all` 仅对 `Union` 有意义（`UNION ALL` 保留重复与序），
/// `INTERSECT`/`EXCEPT` 的 `ALL` 量化词在计划期点名拒绝（SQLite 对齐）。
#[derive(Debug, Clone)]
pub struct SetOperationNode {
    /// 左分支计划
    pub left: Box<PhysicalPlan>,
    /// 右分支计划
    pub right: Box<PhysicalPlan>,
    /// 操作种类
    pub op: SetOpKind,
    /// `UNION ALL` = 保留重复行（其他形态恒去重）
    pub all: bool,
    /// 复合输出列名（= 左侧列名）
    pub columns: Vec<String>,
}

/// 计划树包装节点（MS25 T6 — `EXPLAIN <SELECT>`）
#[derive(Debug, Clone)]
pub struct ExplainNode {
    /// 被观测的内层计划（渲染对象，从不执行）
    pub input: Box<PhysicalPlan>,
}
