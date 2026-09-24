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
pub(crate) mod datetime;
mod delete;
mod derived_scan;
mod drop_table;
mod executor_trait;
mod filter;
mod function;
mod having;
mod index_scan;
mod index_scan_all;
mod insert;
mod join;
mod limit;
mod nested_loop_join;
mod plan;
mod predicate;
mod projection;
mod result;
mod scan;
mod semi_join;
mod single_row;
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
pub use function::{check_scalar_function, is_scalar_function, FunctionExpression};
pub use having::HavingExecutor;
pub use index_scan::IndexScanExecutor;
pub use index_scan_all::IndexScanAllExecutor;
pub use insert::InsertExecutor;
pub use join::JoinExecutor;
pub use limit::LimitExecutor;
pub use nested_loop_join::NestedLoopJoinExecutor;
pub use plan::{
    AggregateNode, AntiJoinNode, ColumnConstraint, ColumnDef, ColumnRef, CorrelatedParam,
    CreateTableNode, DataScanNode, DeleteNode, DerivedScanNode, DropTableNode, FilterNode,
    HavingNode, IndexScanAllNode, IndexScanNode, InsertNode, JoinCondition, JoinNode, LimitNode,
    NestedLoopJoinNode, OrderByColumn, OutputColumn, PhysicalPlan, ProjectionItem, ProjectionNode,
    ScanNode, SemiJoinNode, SortNode, SubqueryEvalNode, UpdateNode,
};
pub use predicate::{
    ArithOp, BinaryArithExpression, CaseExpression, CastExpression, CastType, CoalesceExpression,
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, Expression,
    ExpressionRef, IntervalArithExpression, IsNullPredicate, LikePredicate, LogicalOp,
    LogicalPredicate, NotPredicate, ParameterExpression, Predicate, PredicateRef, Ternary,
};
pub use projection::ProjectionExecutor;
pub use result::ExecResult;
pub use scan::ScanExecutor;
pub use semi_join::SemiJoinExecutorV2;
pub use single_row::SingleRowExecutor;
pub use sort::SortExecutor;
pub use subquery_eval::SubqueryEvalExecutor;
pub use update::UpdateExecutor;
pub use value::{ColumnType, Value, ValueError};
pub use value_ref::ValueRef;

/// MS10-T01 Iter001: narrow a full-schema row to the selected column indices
/// (projection order). An empty projection is the identity, returning the row
/// unchanged. Indices are planner-resolved against the full schema, so a row
/// reaching this function is always at least `projection.len()` long.
pub(crate) fn apply_projection(projection: &[usize], values: Vec<Value>) -> Vec<Value> {
    if projection.is_empty() {
        return values;
    }
    projection.iter().map(|&i| values[i].clone()).collect()
}
