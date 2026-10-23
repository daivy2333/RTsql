//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod join_config;
mod join_related_config;

mod aggregate;
mod alter_table;
mod anti_join;
mod correlated;
mod create_index;
mod create_table;
mod create_view;
mod data_scan;
pub(crate) mod datetime;
mod delete;
mod derived_scan;
mod distinct;
mod drop_index;
mod drop_table;
mod drop_view;
mod executor_trait;
mod explain;
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
mod plan_format;
mod predicate;
mod projection;
pub(crate) mod query_budget;
mod result;
mod scalar_subquery_filter;
mod scan;
mod semi_join;
mod set_operation;
mod single_row;
mod sort;
mod subquery_eval;
mod truncate;
mod update;
mod upsert;
mod value;
mod value_ref;

pub use join_config::JoinConfig;
pub use join_related_config::JoinRelatedConfig;

pub use aggregate::{AggregateExecutor, AggregateFunc, AggregateState};
pub use alter_table::AlterTableExecutor;
pub use anti_join::AntiJoinExecutor;
pub use correlated::inject_correlated_values;
pub use create_index::CreateIndexExecutor;
pub use create_table::CreateTableExecutor;
pub use create_view::CreateViewExecutor;
pub use data_scan::DataScanExecutor;
pub use delete::DeleteExecutor;
pub use derived_scan::DerivedScanExecutor;
pub use distinct::DistinctExecutor;
pub use drop_index::DropIndexExecutor;
pub use drop_table::DropTableExecutor;
pub use drop_view::DropViewExecutor;
pub use executor_trait::Executor;
pub use explain::ExplainExecutor;
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
    AggregateNode, AlterTableNode, AlterTableOp, AntiJoinNode, ColumnConstraint, ColumnDef,
    ColumnRef, ConflictAction, ConflictArbiter, CorrelatedParam, CreateIndexNode,
    CreateTableAsSelectNode, CreateTableNode, CreateViewNode, DataScanNode, DeleteNode,
    DerivedScanNode, DistinctNode, DropIndexNode, DropTableNode, DropViewNode, ExplainNode,
    FilterNode, HavingNode, IndexScanAllNode, IndexScanNode, InsertNode, InsertSelectNode,
    JoinCondition, JoinNode, JoinSide, LimitNode, NestedLoopJoinNode, NestedLoopJoinType,
    OrderByColumn, OutputColumn, PhysicalPlan, ProjectionItem, ProjectionNode,
    ScalarSubqueryBinding, ScalarSubqueryFilterNode, ScanNode, SemiJoinNode, SetOpKind,
    SetOperationNode, SortNode, SubqueryEvalNode, TruncateNode, UpdateAssignment, UpdateNode,
    UpdateValue, UpsertAssignment, UpsertNode, UpsertValueExpr, ValueParamSlot,
};
pub use predicate::{
    ArithOp, BinaryArithExpression, CaseExpression, CastExpression, CastType, CoalesceExpression,
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, ConstantPredicate,
    Expression, ExpressionRef, IntervalArithExpression, IsNullPredicate, LikePredicate, LogicalOp,
    LogicalPredicate, NotPredicate, ParameterExpression, Predicate, PredicateRef, Ternary,
};
pub use projection::ProjectionExecutor;
pub use query_budget::QueryBudget;
pub use result::ExecResult;
pub use scalar_subquery_filter::ScalarSubqueryFilterExecutor;
pub use scan::ScanExecutor;
pub use semi_join::SemiJoinExecutorV2;
pub use set_operation::SetOperationExecutor;
pub use single_row::SingleRowExecutor;
pub use sort::SortExecutor;
pub use subquery_eval::SubqueryEvalExecutor;
pub use truncate::TruncateExecutor;
pub use update::UpdateExecutor;
pub use upsert::UpsertExecutor;
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

/// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：单元测试用
/// 行序列执行器——按给定行集逐行产出后结束，供执行器构造面直接实例化被测
/// 执行器（生产路径不消费）。
#[cfg(test)]
pub(crate) struct StubRowExecutor {
    rows: std::vec::IntoIter<Vec<Value>>,
}

#[cfg(test)]
impl StubRowExecutor {
    pub(crate) fn new(rows: Vec<Vec<Value>>) -> Self {
        Self {
            rows: rows.into_iter(),
        }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl Executor for StubRowExecutor {
    async fn next(&mut self) -> crate::storage::Result<Option<ExecResult>> {
        Ok(self.rows.next().map(ExecResult::Row))
    }
}

/// MS32-T01 Iter000（Task 1.1）：`next_batch` 默认方法的契约——批量取出与
/// 逐行取出同序，`max = 0` 不消费输入。
#[cfg(test)]
mod batch_interface_tests {
    use super::*;

    fn rows(n: i64) -> Vec<Vec<Value>> {
        (0..n).map(|i| vec![Value::Int(i)]).collect()
    }

    #[tokio::test]
    async fn next_batch_matches_rows() {
        // 5 行按 2 分批：2、2、1，然后空批。
        let mut executor = StubRowExecutor::new(rows(5));
        let first = executor.next_batch(2).await.unwrap();
        let second = executor.next_batch(2).await.unwrap();
        let third = executor.next_batch(2).await.unwrap();
        let fourth = executor.next_batch(2).await.unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        assert_eq!(third.len(), 1);
        assert!(fourth.is_empty());

        // 分批序列与逐行序列逐项相同。
        let mut rowwise = StubRowExecutor::new(rows(5));
        let mut expected = Vec::new();
        while let Some(result) = rowwise.next().await.unwrap() {
            expected.push(result);
        }
        let mut actual = Vec::new();
        actual.extend(first);
        actual.extend(second);
        actual.extend(third);
        assert_eq!(actual, expected);

        // `max = 0` 返回空批且不推进游标——下一次 `next` 仍是第一行。
        let mut zero = StubRowExecutor::new(rows(5));
        assert!(zero.next_batch(0).await.unwrap().is_empty());
        assert_eq!(
            zero.next().await.unwrap(),
            Some(ExecResult::Row(vec![Value::Int(0)]))
        );
    }
}
