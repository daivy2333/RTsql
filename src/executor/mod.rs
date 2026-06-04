//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod join_config;
mod join_related_config;

mod aggregate;
mod anti_join;
mod correlated;
mod create_table;
mod data_scan;
mod delete;
mod derived_scan;
mod drop_table;
mod executor_trait;
mod filter;
mod having;
mod index_scan;
mod index_scan_all;
mod insert;
mod join;
mod limit;
mod plan;
mod predicate;
mod result;
mod scan;
mod semi_join;
mod sort;
mod subquery_eval;
mod update;
mod value;
mod value_ref;

pub use join_config::JoinConfig;
pub use join_related_config::JoinRelatedConfig;

pub use aggregate::{AggregateExecutor, AggregateFunc, AggregateState};
pub use anti_join::AntiJoinExecutor;
pub use correlated::inject_correlated_values;
pub use create_table::CreateTableExecutor;
pub use data_scan::DataScanExecutor;
pub use delete::DeleteExecutor;
pub use derived_scan::DerivedScanExecutor;
pub use drop_table::DropTableExecutor;
pub use executor_trait::Executor;
pub use filter::FilterExecutor;
pub use having::HavingExecutor;
pub use index_scan::IndexScanExecutor;
pub use index_scan_all::IndexScanAllExecutor;
pub use insert::InsertExecutor;
pub use join::JoinExecutor;
pub use limit::LimitExecutor;
pub use plan::{
    AggregateNode, AntiJoinNode, ColumnConstraint, ColumnDef, ColumnRef, CorrelatedParam,
    CreateTableNode, DataScanNode, DeleteNode, DerivedScanNode, DropTableNode, FilterNode,
    HavingNode, IndexScanAllNode, IndexScanNode, InsertNode, JoinCondition, JoinNode, LimitNode,
    OrderByColumn, OutputColumn, PhysicalPlan, ScanNode, SemiJoinNode, SortNode, SubqueryEvalNode,
    UpdateNode,
};
pub use predicate::{
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, Expression,
    ExpressionRef, LogicalOp, LogicalPredicate, ParameterExpression, Predicate, PredicateRef,
};
pub use result::ExecResult;
pub use scan::ScanExecutor;
pub use semi_join::SemiJoinExecutorV2;
pub use sort::SortExecutor;
pub use subquery_eval::SubqueryEvalExecutor;
pub use update::UpdateExecutor;
pub use value::{ColumnType, Value, ValueError};
pub use value_ref::ValueRef;
