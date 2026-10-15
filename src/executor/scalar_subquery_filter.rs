//! WHERE scalar-subquery filter executor (T4, change
//! 2026-10-04-external-review-gaps).
//!
//! Evaluates a WHERE predicate whose comparison legs contain scalar
//! subqueries. The planner compiled each `Expr::Subquery` leg into a
//! reserved-name `ParameterExpression` placeholder and recorded the subquery
//! plan as a binding. Per outer row, this executor evaluates each binding's
//! subquery, injects the scalar result into the predicate by parameter name,
//! evaluates the predicate on the full-width row, and trims survivors by
//! `projection`.
//!
//! Caching mirrors `SubqueryExecutor`/`SubqueryEvalExecutor`: non-correlated
//! bindings evaluate once per statement; correlated bindings cache per
//! correlated-parameter value sequence for the statement's lifetime
//! (executors are rebuilt per statement). Errors are never cached.

use crate::database::Database;
use crate::executor::{
    apply_projection, inject_correlated_values, ExecResult, Executor, PredicateRef, Value,
};
use crate::storage::Result;
use crate::transaction::Snapshot;
use std::collections::HashMap;
use std::sync::Arc;

/// Per-binding evaluation state: cached scalar for non-correlated bindings,
/// per-parameter-value cache for correlated ones.
struct BindingCache {
    scalar: Option<Value>,
    correlated: HashMap<Vec<(String, Value)>, Value>,
}

/// WHERE scalar-subquery filter executor (see [`crate::executor::plan`]
/// `ScalarSubqueryFilterNode` for the plan-side contract).
pub struct ScalarSubqueryFilterExecutor {
    input: Box<dyn Executor + Send>,
    predicate: PredicateRef,
    bindings: Vec<crate::executor::ScalarSubqueryBinding>,
    outer_column_indices: HashMap<String, usize>,
    database: Arc<Database>,
    /// Statement-start snapshot re-applied on every (re-)evaluation of a
    /// binding subquery (`None` under RepeatableRead) — same rule as
    /// SubqueryEvalExecutor.
    snapshot: Option<Snapshot>,
    /// Cross-db route re-applied on every (re-)evaluation (ATTACH sessions).
    routed_tables: Arc<crate::pipeline::RoutedTables>,
    projection: Vec<usize>,
    caches: Vec<BindingCache>,
}

impl ScalarSubqueryFilterExecutor {
    /// Create a new ScalarSubqueryFilterExecutor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        input: Box<dyn Executor + Send>,
        predicate: PredicateRef,
        bindings: Vec<crate::executor::ScalarSubqueryBinding>,
        outer_column_indices: HashMap<String, usize>,
        database: Arc<Database>,
        snapshot: Option<Snapshot>,
        routed_tables: Arc<crate::pipeline::RoutedTables>,
        projection: Vec<usize>,
    ) -> Self {
        let caches = bindings
            .iter()
            .map(|_| BindingCache {
                scalar: None,
                correlated: HashMap::new(),
            })
            .collect();
        Self {
            input,
            predicate,
            bindings,
            outer_column_indices,
            database,
            snapshot,
            routed_tables,
            projection,
            caches,
        }
    }

    /// Extract correlated parameter values for one binding from the outer row
    /// (same by-column-name lookup as SubqueryEvalExecutor).
    ///
    /// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：未命中
    /// SHALL 报错点名该外层列，SHALL NOT 静默回落外层首列。
    fn extract_param_values(
        &self,
        binding_idx: usize,
        row: &[Value],
    ) -> Result<Vec<(String, Value)>> {
        self.bindings[binding_idx]
            .correlated_params
            .iter()
            .map(|cp| {
                let idx = self
                    .outer_column_indices
                    .get(&cp.outer_column.to_lowercase())
                    .copied()
                    .ok_or_else(|| {
                        crate::storage::StorageError::ExecutionError(format!(
                            "correlated parameter column not found in outer row: {}",
                            cp.outer_column
                        ))
                    })?;
                Ok((
                    cp.param_name.clone(),
                    row.get(idx).cloned().unwrap_or(Value::Null),
                ))
            })
            .collect()
    }

    /// Evaluate one binding's subquery, returning the scalar (first row, first
    /// column; empty → NULL; multi-row → the established error face). Same
    /// drain semantics as SubqueryEvalExecutor.
    async fn eval_subquery_scalar(
        &mut self,
        plan: &crate::executor::PhysicalPlan,
    ) -> Result<Value> {
        let mut executor = crate::pipeline::create_executor_from_plan(
            plan.clone(),
            &self.database,
            None,
            self.snapshot.clone(),
            &self.routed_tables,
        )
        .await?;

        let mut result_value: Option<Value> = None;
        let mut row_count = 0;
        while let Some(result) = executor.next().await? {
            match result {
                ExecResult::Row(row) => {
                    row_count += 1;
                    if row_count > 1 {
                        return Err(crate::storage::StorageError::ExecutionError(
                            crate::parser::error::PlanError::SubqueryReturnsMultipleRow.to_string(),
                        ));
                    }
                    if !row.is_empty() {
                        result_value = Some(row[0].clone());
                    }
                }
                ExecResult::AffectedRows(_) | ExecResult::RowId(_) => continue,
            }
        }
        Ok(result_value.unwrap_or(Value::Null))
    }

    /// Resolve one binding's scalar for the current row (cache-aware). Errors
    /// propagate before any cache insert, so failed evaluations are re-run.
    async fn eval_binding(&mut self, binding_idx: usize, row: &[Value]) -> Result<Value> {
        let plan = self.bindings[binding_idx].subquery.as_ref().clone();
        if self.bindings[binding_idx].correlated_params.is_empty() {
            if let Some(cached) = &self.caches[binding_idx].scalar {
                return Ok(cached.clone());
            }
            let value = self.eval_subquery_scalar(&plan).await?;
            self.caches[binding_idx].scalar = Some(value.clone());
            Ok(value)
        } else {
            let param_values = self.extract_param_values(binding_idx, row)?;
            if let Some(cached) = self.caches[binding_idx].correlated.get(&param_values) {
                return Ok(cached.clone());
            }
            let injected = plan;
            inject_correlated_values(&injected, &param_values);
            let value = self.eval_subquery_scalar(&injected).await?;
            self.caches[binding_idx]
                .correlated
                .insert(param_values, value.clone());
            Ok(value)
        }
    }
}

#[async_trait::async_trait]
impl Executor for ScalarSubqueryFilterExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        loop {
            match self.input.next().await? {
                None => return Ok(None),
                Some(ExecResult::Row(values)) => {
                    let mut scalar_error: Option<crate::storage::StorageError> = None;
                    for binding_idx in 0..self.bindings.len() {
                        match self.eval_binding(binding_idx, &values).await {
                            Ok(scalar) => {
                                let name = self.bindings[binding_idx].param_name.clone();
                                self.predicate.inject_parameters(&[(name, scalar)]);
                            }
                            Err(e) => {
                                scalar_error = Some(e);
                                break;
                            }
                        }
                    }
                    if let Some(e) = scalar_error {
                        return Err(e);
                    }
                    match self.predicate.evaluate(&values) {
                        Ok(true) => {
                            return Ok(Some(ExecResult::Row(apply_projection(
                                &self.projection,
                                values,
                            ))))
                        }
                        Ok(false) => continue,
                        Err(e) => {
                            return Err(crate::storage::StorageError::ExecutionError(format!(
                                "Predicate evaluation error: {e}"
                            )))
                        }
                    }
                }
                Some(ExecResult::AffectedRows(_)) => continue,
                Some(ExecResult::RowId(_)) => continue,
            }
        }
    }
}

/// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：关联参数外层
/// 绑定期望存在而解析失败时显式报错点名列，SHALL NOT 静默回落外层首列。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{
        ConstantPredicate, CorrelatedParam, PhysicalPlan, ScalarSubqueryBinding, StubRowExecutor,
    };
    use std::collections::HashMap;

    fn executor(database: Arc<Database>, outer_column: &str) -> ScalarSubqueryFilterExecutor {
        ScalarSubqueryFilterExecutor::new(
            Box::new(StubRowExecutor::new(vec![vec![
                Value::Int(7),
                Value::Int(9),
            ]])),
            std::sync::Arc::new(ConstantPredicate { value: true }),
            vec![ScalarSubqueryBinding {
                param_name: "__scalar_subquery_0".to_string(),
                subquery: Box::new(PhysicalPlan::SingleRow),
                correlated_params: vec![CorrelatedParam::new(
                    "o".to_string(),
                    outer_column.to_string(),
                    format!("o.{outer_column}"),
                )],
            }],
            HashMap::from([("id".to_string(), 0usize), ("ref".to_string(), 1usize)]),
            database,
            None,
            std::sync::Arc::new(crate::pipeline::RoutedTables::new()),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn missing_outer_column_reports_error_naming_column() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let mut exec = executor(db, "missing_col");
        let err = exec
            .next()
            .await
            .expect_err("绑参未命中应报错而非静默取外层首列");
        let msg = err.to_string();
        assert!(
            msg.contains("missing_col"),
            "错误应点名未命中的外层列: {msg}"
        );
    }
}
