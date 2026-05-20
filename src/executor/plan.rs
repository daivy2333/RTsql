//! Physical plan types for query execution

use crate::executor::Value;
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