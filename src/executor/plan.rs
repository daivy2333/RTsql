//! Physical plan types for query execution

use crate::executor::{ColumnType, Value};
use crate::storage::page_format::Key;

/// 物理计划节点（同步结构，M5 异步执行）
#[derive(Debug, Clone)]
pub enum PhysicalPlan {
    /// 全表扫描
    Scan(ScanNode),
    /// 主键索引扫描
    IndexScan(IndexScanNode),
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
