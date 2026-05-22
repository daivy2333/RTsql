//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod aggregate;
mod create_table;
mod delete;
mod drop_table;
mod executor_trait;
mod filter;
mod having;
mod index_scan;
mod insert;
mod join;
mod limit;
mod plan;
mod predicate;
mod result;
mod scan;
mod sort;
mod update;
mod value;

pub use aggregate::{AggregateFunc, AggregateState};
pub use create_table::CreateTableExecutor;
pub use delete::DeleteExecutor;
pub use drop_table::DropTableExecutor;
pub use executor_trait::Executor;
pub use filter::FilterExecutor;
pub use index_scan::IndexScanExecutor;
pub use insert::InsertExecutor;
pub use join::JoinExecutor;
pub use limit::LimitExecutor;
pub use plan::{
    AggregateNode, ColumnConstraint, ColumnDef, ColumnRef, CreateTableNode, DeleteNode,
    DropTableNode, FilterNode, HavingNode, IndexScanNode, InsertNode, JoinCondition, JoinNode,
    LimitNode, OrderByColumn, OutputColumn, PhysicalPlan, ScanNode, SortNode, UpdateNode,
};
pub use predicate::{
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, Expression,
    ExpressionRef, LogicalOp, LogicalPredicate, Predicate, PredicateRef,
};
pub use result::ExecResult;
pub use scan::ScanExecutor;
pub use sort::SortExecutor;
pub use update::UpdateExecutor;
pub use value::{ColumnType, Value, ValueError};
