//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod create_table;
mod delete;
mod drop_table;
mod executor_trait;
mod index_scan;
mod insert;
mod plan;
mod predicate;
mod result;
mod scan;
mod update;
mod value;

pub use create_table::CreateTableExecutor;
pub use delete::DeleteExecutor;
pub use drop_table::DropTableExecutor;
pub use executor_trait::Executor;
pub use index_scan::IndexScanExecutor;
pub use insert::InsertExecutor;
pub use plan::{
    ColumnConstraint, ColumnDef, CreateTableNode, DeleteNode, DropTableNode, FilterNode,
    IndexScanNode, InsertNode, PhysicalPlan, ScanNode, UpdateNode,
};
pub use predicate::{
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, Expression,
    ExpressionRef, LogicalOp, LogicalPredicate, Predicate, PredicateRef,
};
pub use result::ExecResult;
pub use scan::ScanExecutor;
pub use update::UpdateExecutor;
pub use value::{ColumnType, Value, ValueError};
