//! Scalar subquery evaluation executor
//!
//! Evaluates a scalar subquery in the SELECT projection list.
//! For each outer row, executes the subquery and takes the first row's first column
//! as the scalar result, inserting it at the specified column index.
//! For independent (non-correlated) subqueries, the result is cached after first evaluation.
//! For correlated subqueries, results are cached per correlated parameter value
//! sequence for the lifetime of the statement (executors are rebuilt per
//! statement, so a local cache never outlives it).

use crate::database::Database;
use crate::executor::{CorrelatedParam, ExecResult, Executor, PhysicalPlan, Value};
use crate::storage::Result;
use crate::transaction::Snapshot;
use std::collections::HashMap;
use std::sync::Arc;

/// Scalar subquery evaluation executor
///
/// Takes an input plan (outer query rows) and a subquery plan, evaluates the
/// subquery for each outer row, and inserts the scalar result at `result_column_index`.
/// For independent subqueries, the result is cached after the first evaluation.
pub struct SubqueryEvalExecutor {
    input: Box<dyn Executor + Send>,
    subquery_plan: PhysicalPlan,
    _output_column: String,
    result_column_index: usize,
    correlated_params: Vec<CorrelatedParam>,
    outer_column_indices: HashMap<String, usize>,
    database: Arc<Database>,
    /// MS09 Iter000 (D4): statement-start Read Committed snapshot re-applied
    /// on every (re-)evaluation of the subquery (`None` under RepeatableRead).
    snapshot: Option<Snapshot>,
    /// MS20 T2: cross-db route table re-applied on every (re-)evaluation, so a
    /// subquery reading an attached table resolves to that database. Empty
    /// outside an ATTACH session (lib path / single-database CLI call).
    pub(crate) routed_tables: Arc<crate::pipeline::RoutedTables>,
    cached_result: Option<Value>,
    /// MS09 Iter002 (D6): statement-level correlated-result cache keyed by the
    /// correlated parameter value sequence. Stores the successful row set (0 or
    /// 1 rows; multi-row evaluations error before reaching the store). Errors
    /// are never cached.
    correlated_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>,
    /// T4（change 2026-10-04-external-review-gaps）：输出收窄宽度（节点
    /// `output_width`）——参数提取后、标量插入前按此宽度截断行；0 = 恒等。
    output_width: usize,
}

impl SubqueryEvalExecutor {
    /// Create a new SubqueryEvalExecutor
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        input: Box<dyn Executor + Send>,
        subquery_plan: PhysicalPlan,
        output_column: String,
        result_column_index: usize,
        correlated_params: Vec<CorrelatedParam>,
        outer_column_indices: HashMap<String, usize>,
        database: Arc<Database>,
        snapshot: Option<Snapshot>,
        routed_tables: Arc<crate::pipeline::RoutedTables>,
    ) -> Self {
        Self::with_output_width(
            input,
            subquery_plan,
            output_column,
            result_column_index,
            correlated_params,
            outer_column_indices,
            database,
            snapshot,
            routed_tables,
            0,
        )
    }

    /// T4（change 2026-10-04-external-review-gaps）：带输出收窄宽度的构造——
    /// 语义同 `new`，另在参数提取后按 `output_width` 截断行（0 = 恒等）。
    #[allow(clippy::too_many_arguments)]
    pub fn with_output_width(
        input: Box<dyn Executor + Send>,
        subquery_plan: PhysicalPlan,
        output_column: String,
        result_column_index: usize,
        correlated_params: Vec<CorrelatedParam>,
        outer_column_indices: HashMap<String, usize>,
        database: Arc<Database>,
        snapshot: Option<Snapshot>,
        routed_tables: Arc<crate::pipeline::RoutedTables>,
        output_width: usize,
    ) -> Self {
        Self {
            input,
            subquery_plan,
            _output_column: output_column,
            result_column_index,
            correlated_params,
            outer_column_indices,
            database,
            snapshot,
            routed_tables,
            cached_result: None,
            correlated_cache: HashMap::new(),
            output_width,
        }
    }

    /// Evaluate the subquery, returning the scalar result (first row, first column).
    /// Returns Null if the subquery produces no rows.
    /// Returns an error if the subquery returns more than one row.
    async fn eval_subquery(&mut self) -> Result<Value> {
        let mut executor = crate::pipeline::create_executor_from_plan(
            self.subquery_plan.clone(),
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

    /// Extract parameter values from the current outer row.
    /// Maps each correlated parameter to its value using `outer_column_indices`.
    ///
    /// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：未命中
    /// SHALL 报错点名该外层列，SHALL NOT 静默回落外层首列。
    fn extract_param_values(&self, row: &[Value]) -> Result<Vec<(String, Value)>> {
        self.correlated_params
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
}

#[async_trait::async_trait]
impl Executor for SubqueryEvalExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(mut row)) => {
                    let is_correlated = !self.correlated_params.is_empty();
                    let scalar_value = if is_correlated {
                        // Statement-level cache: same parameter value sequence
                        // reuses the first evaluation's row set (MS09 Iter002).
                        let param_values = self.extract_param_values(&row)?;
                        if let Some(rows) = self.correlated_cache.get(&param_values) {
                            match rows.first() {
                                Some(inner_row) if !inner_row.is_empty() => inner_row[0].clone(),
                                _ => Value::Null,
                            }
                        } else {
                            let cloned_plan = self.subquery_plan.clone();
                            crate::executor::inject_correlated_values(&cloned_plan, &param_values);
                            let mut executor = crate::pipeline::create_executor_from_plan(
                                cloned_plan,
                                &self.database,
                                None,
                                self.snapshot.clone(),
                                &self.routed_tables,
                            )
                            .await?;
                            let mut result_value: Option<Value> = None;
                            let mut row_count = 0;
                            let mut collected: Vec<Vec<Value>> = Vec::new();
                            while let Some(result) = executor.next().await? {
                                match result {
                                    ExecResult::Row(inner_row) => {
                                        row_count += 1;
                                        if row_count > 1 {
                                            return Err(
                                                crate::storage::StorageError::ExecutionError(
                                                    crate::parser::error::PlanError::SubqueryReturnsMultipleRow
                                                        .to_string(),
                                                ),
                                            );
                                        }
                                        if !inner_row.is_empty() {
                                            result_value = Some(inner_row[0].clone());
                                        }
                                        collected.push(inner_row);
                                    }
                                    ExecResult::AffectedRows(_) | ExecResult::RowId(_) => continue,
                                }
                            }
                            // Store only after a successful drain; errors and
                            // the multi-row early exit above never reach here.
                            self.correlated_cache.insert(param_values, collected);
                            result_value.unwrap_or(Value::Null)
                        }
                    } else {
                        // Independent subquery: cache result (unchanged)
                        if self.cached_result.is_none() {
                            let val = self.eval_subquery().await?;
                            self.cached_result = Some(val);
                        }
                        self.cached_result.as_ref().unwrap().clone()
                    };

                    // T4: 相关参数列已在参数提取时消费——收窄宽度非零时先
                    // 截断补充列，再按 result_column_index 插入标量值。
                    if self.output_width > 0 && self.output_width <= row.len() {
                        row.truncate(self.output_width);
                    }
                    // Insert scalar value at result_column_index
                    if self.result_column_index <= row.len() {
                        row.insert(self.result_column_index, scalar_value);
                    } else {
                        row.push(scalar_value);
                    }

                    return Ok(Some(ExecResult::Row(row)));
                }
                Some(ExecResult::AffectedRows(_)) => continue,
                Some(ExecResult::RowId(_)) => continue,
                None => return Ok(None),
            }
        }
    }
}

/// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：关联参数外层
/// 绑定期望存在而解析失败时显式报错点名列，SHALL NOT 静默回落外层首列。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{CorrelatedParam, StubRowExecutor};
    use std::collections::HashMap;

    fn executor(database: Arc<Database>, outer_column: &str) -> SubqueryEvalExecutor {
        SubqueryEvalExecutor::new(
            Box::new(StubRowExecutor::new(vec![vec![
                Value::Int(7),
                Value::Int(9),
            ]])),
            PhysicalPlan::SingleRow,
            "who".to_string(),
            0,
            vec![CorrelatedParam::new(
                "o".to_string(),
                outer_column.to_string(),
                format!("o.{outer_column}"),
            )],
            HashMap::from([("id".to_string(), 0usize), ("ref".to_string(), 1usize)]),
            database,
            None,
            std::sync::Arc::new(crate::pipeline::RoutedTables::new()),
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
