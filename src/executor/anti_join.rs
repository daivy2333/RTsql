//! Anti-Join executor - NOT IN / NOT EXISTS subquery unnesting
//!
//! Outputs only left-table rows that have NO match in the right table.
//! - NOT EXISTS mode: conditions is empty, checks right table has NO rows
//! - NOT IN mode: conditions non-empty, builds hash key from left row and probes right hash map
//! - Correlated subquery: correlated_params non-empty, re-materializes right per left row (placeholder)

use crate::database::Database;
use crate::executor::{
    apply_projection, CorrelatedParam, ExecResult, Executor, JoinCondition, JoinRelatedConfig,
    PhysicalPlan, Value,
};
use crate::storage::Result;
use crate::transaction::Snapshot;
use std::collections::HashMap;
use std::sync::Arc;

/// Anti-Join execution phase
#[derive(Debug, Clone, PartialEq)]
enum AntiJoinPhase {
    /// Build right hash table
    BuildRight,
    /// Scan left and output non-matching rows
    ScanLeft,
    /// All done
    Done,
}

/// Anti-Join executor - outputs left-table rows that do NOT match right table
///
/// Used by Pipeline::create_executor_from_plan for non-correlated subqueries.
/// For correlated subqueries (correlated_params non-empty), the right table
/// re-materialization is a placeholder to be completed in Task 10.
pub struct AntiJoinExecutor {
    left: Box<dyn Executor + Send>,
    right: Box<dyn Executor + Send>,
    conditions: Vec<JoinCondition>,
    /// T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：SELECT
    /// 清单裁剪位（空 = 恒等全宽输出——ORDER BY 存在时裁剪归 Sort 承担）。
    projection: Vec<usize>,
    /// Correlated subquery parameters (empty = independent subquery)
    correlated_params: Vec<CorrelatedParam>,

    /// Saved right plan for per-row rebuild (None = independent subquery)
    right_plan: Option<PhysicalPlan>,
    /// Database reference for per-row rebuild (None = independent subquery)
    database: Option<Arc<Database>>,
    /// MS20 T2: cross-db route table re-applied on every correlated
    /// right-plan rebuild (empty outside an ATTACH session).
    routed_tables: Arc<crate::pipeline::RoutedTables>,
    /// MS09 Iter000 (D4): statement-start Read Committed snapshot re-applied
    /// on every correlated right-plan rebuild (`None` under RepeatableRead).
    snapshot: Option<Snapshot>,

    /// Left column name to index mapping (for finding condition columns)
    left_column_indices: HashMap<String, usize>,
    /// Right column name to index mapping (built by Pipeline from extract_column_indices)
    right_column_indices: HashMap<String, usize>,

    /// Right table hash map: key = right condition column values, value = list of matching rows
    right_hashmap: Option<HashMap<Vec<Value>, Vec<Vec<Value>>>>,
    /// Whether the right table has any rows (for NOT EXISTS mode)
    right_has_rows: Option<bool>,
    /// MS09 Iter002 (D6): statement-level correlated-result cache keyed by the
    /// correlated parameter value sequence. Stores the full right row set
    /// (including NULL-key rows, which count toward `right_has_rows`). Errors
    /// are never cached.
    correlated_rows_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>,

    phase: AntiJoinPhase,
    executed: bool,
}

impl AntiJoinExecutor {
    /// Create a new AntiJoinExecutor
    pub fn new(config: JoinRelatedConfig) -> Self {
        Self {
            left: config.left,
            right: config.right,
            conditions: config.conditions,
            projection: config.projection,
            correlated_params: config.correlated_params,
            right_plan: config.right_plan,
            database: config.database,
            snapshot: config.snapshot,
            routed_tables: config.routed_tables,
            left_column_indices: config.left_column_indices,
            right_column_indices: config.right_column_indices,
            right_hashmap: None,
            right_has_rows: None,
            correlated_rows_cache: HashMap::new(),
            phase: AntiJoinPhase::BuildRight,
            executed: false,
        }
    }

    /// Build hash key from right-table row using join conditions
    fn build_right_key(&self, right_row: &[Value]) -> Option<Vec<Value>> {
        let key: Vec<Value> = self
            .conditions
            .iter()
            .map(|cond| {
                match self
                    .right_column_indices
                    .get(&cond.right_column.column.to_lowercase())
                {
                    Some(&idx) => right_row[idx].clone(),
                    None => Value::Null,
                }
            })
            .collect();

        // SQL semantics: NULL never matches NULL in joins
        if key.iter().any(|v| matches!(v, Value::Null)) {
            return None;
        }
        Some(key)
    }

    /// Build hash key from left-table row using join conditions
    fn build_left_key(&self, left_row: &[Value]) -> Option<Vec<Value>> {
        let key: Vec<Value> = self
            .conditions
            .iter()
            .map(|cond| {
                // Use left_column_indices to find condition column (includes all table columns)
                match self
                    .left_column_indices
                    .get(&cond.left_column.column.to_lowercase())
                {
                    Some(&idx) => left_row[idx].clone(),
                    None => Value::Null,
                }
            })
            .collect();

        // SQL semantics: NULL never matches NULL in joins
        if key.iter().any(|v| matches!(v, Value::Null)) {
            return None;
        }
        Some(key)
    }

    /// Build output row from left-table row (T5 D8：基座全宽承载——行内容 = 左行
    /// 直通；SELECT 清单裁剪由本节点 `projection` 在直通后应用，ORDER BY 存在时
    /// projection 为空、裁剪归 Sort)。
    fn build_output_row(&self, left_row: &[Value]) -> Vec<Value> {
        apply_projection(&self.projection, left_row.to_vec())
    }

    /// Extract correlated parameter values from a left-table row.
    ///
    /// T1（change 2026-10-15-antijoin-correlated-param-hardening）：未命中 SHALL
    /// 报错点名该外层列——修复前 `filter_map` 静默丢弃该参数，使右计划对应参数
    /// 未注入求值 NULL，且 `correlated_rows_cache` 键短项化可致跨行误共享
    /// （同族前 change T2 三处为 `unwrap_or(0)` 静默回落外层首列）。
    fn extract_param_values(&self, left_row: &[Value]) -> Result<Vec<(String, Value)>> {
        self.correlated_params
            .iter()
            .map(|cp| {
                let idx = self
                    .left_column_indices
                    .get(&cp.outer_column.to_lowercase())
                    .copied()
                    .ok_or_else(|| {
                        crate::storage::StorageError::ExecutionError(format!(
                            "correlated parameter column not found in outer row: {}",
                            cp.outer_column
                        ))
                    })?;
                Ok((cp.param_name.clone(), left_row[idx].clone()))
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Executor for AntiJoinExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if !self.executed {
            self.executed = true;
            self.phase = AntiJoinPhase::BuildRight;
        }

        loop {
            match self.phase {
                AntiJoinPhase::BuildRight => {
                    if self.correlated_params.is_empty() {
                        // Independent subquery: materialize right table once
                        let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> = HashMap::new();
                        let mut has_rows = false;

                        while let Some(result) = self.right.next().await? {
                            if let ExecResult::Row(row) = result {
                                has_rows = true;
                                if let Some(hash_key) = self.build_right_key(&row) {
                                    hashmap.entry(hash_key).or_default().push(row);
                                }
                            }
                        }

                        self.right_hashmap = Some(hashmap);
                        self.right_has_rows = Some(has_rows);
                    } else {
                        // Correlated subquery: defer materialization to per-row ScanLeft
                        self.right_hashmap = Some(HashMap::new());
                        self.right_has_rows = Some(false);
                    }
                    self.phase = AntiJoinPhase::ScanLeft;
                }

                AntiJoinPhase::ScanLeft => {
                    // Scan left table row by row
                    loop {
                        match self.left.next().await? {
                            Some(ExecResult::Row(left_row)) => {
                                // Correlated subquery: rebuild right hashmap per left row
                                if !self.correlated_params.is_empty() {
                                    if let (Some(ref right_plan), Some(ref database)) =
                                        (&self.right_plan, &self.database)
                                    {
                                        let param_values = self.extract_param_values(&left_row)?;
                                        if let Some(rows) =
                                            self.correlated_rows_cache.get(&param_values)
                                        {
                                            // Cache hit: rebuild the hash map and row
                                            // presence exactly as the miss path does.
                                            let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> =
                                                HashMap::new();
                                            for row in rows {
                                                if let Some(hash_key) = self.build_right_key(row) {
                                                    hashmap
                                                        .entry(hash_key)
                                                        .or_default()
                                                        .push(row.clone());
                                                }
                                            }
                                            let has_rows = !rows.is_empty();
                                            self.right_hashmap = Some(hashmap);
                                            self.right_has_rows = Some(has_rows);
                                        } else {
                                            let plan = right_plan.clone();
                                            crate::executor::inject_correlated_values(
                                                &plan,
                                                &param_values,
                                            );
                                            let mut right_exec =
                                                crate::pipeline::create_executor_from_plan(
                                                    plan,
                                                    database,
                                                    None,
                                                    self.snapshot.clone(),
                                                    &self.routed_tables,
                                                )
                                                .await?;
                                            let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> =
                                                HashMap::new();
                                            let mut has_rows = false;
                                            let mut collected: Vec<Vec<Value>> = Vec::new();
                                            while let Some(result) = right_exec.next().await? {
                                                if let ExecResult::Row(row) = result {
                                                    has_rows = true;
                                                    if let Some(hash_key) =
                                                        self.build_right_key(&row)
                                                    {
                                                        hashmap
                                                            .entry(hash_key)
                                                            .or_default()
                                                            .push(row.clone());
                                                    }
                                                    collected.push(row);
                                                }
                                            }
                                            // Store only after a successful drain.
                                            self.correlated_rows_cache
                                                .insert(param_values, collected);
                                            self.right_hashmap = Some(hashmap);
                                            self.right_has_rows = Some(has_rows);
                                        }
                                    }
                                }

                                let has_rows = self.right_has_rows.unwrap();

                                // NOT EXISTS mode: conditions empty, check if right has NO rows
                                if self.conditions.is_empty() {
                                    if !has_rows {
                                        return Ok(Some(ExecResult::Row(
                                            self.build_output_row(&left_row),
                                        )));
                                    }
                                    continue;
                                }

                                // NOT IN mode: build left key and probe right hash map
                                let hashmap = self.right_hashmap.as_ref().unwrap();
                                match self.build_left_key(&left_row) {
                                    Some(key) => {
                                        // Anti-Join: output when NOT matched
                                        if !hashmap.contains_key(&key) {
                                            return Ok(Some(ExecResult::Row(
                                                self.build_output_row(&left_row),
                                            )));
                                        }
                                        continue;
                                    }
                                    None => {
                                        // NULL in key, no match possible
                                        // In anti-join, NULL key means no match, so output the row
                                        return Ok(Some(ExecResult::Row(
                                            self.build_output_row(&left_row),
                                        )));
                                    }
                                }
                            }
                            Some(ExecResult::AffectedRows(_)) => continue,
                            Some(ExecResult::RowId(_)) => continue,
                            None => {
                                self.phase = AntiJoinPhase::Done;
                                break;
                            }
                        }
                    }
                }

                AntiJoinPhase::Done => {
                    return Ok(None);
                }
            }
        }
    }
}

/// T1（change 2026-10-15-antijoin-correlated-param-hardening）：关联参数外层
/// 绑定期望存在而解析失败时显式报错点名列，SHALL NOT 静默丢弃该参数——丢弃会
/// 使右计划对应参数未注入求值 NULL，且缓存键短项化致跨行误共享。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::JoinRelatedConfig;
    use crate::executor::{CorrelatedParam, StubRowExecutor};

    /// 外层行只有 id / ref 两列，`missing_col` 不在其中。`right_plan` 与
    /// `database` 必须给 `Some`：`ScanLeft` 的相关分支整体位于
    /// `if let (Some(right_plan), Some(database))` 之内，二者缺一时绑定期根本
    /// 不被经过，断言会因错误原因通过。
    async fn config(outer_column: &str) -> JoinRelatedConfig {
        JoinRelatedConfig {
            left: Box::new(StubRowExecutor::new(vec![vec![
                Value::Int(7),
                Value::Int(9),
            ]])),
            right: Box::new(StubRowExecutor::new(Vec::new())),
            conditions: Vec::new(),
            projection: Vec::new(),
            correlated_params: vec![CorrelatedParam::new(
                "o".to_string(),
                outer_column.to_string(),
                format!("o.{outer_column}"),
            )],
            left_column_indices: HashMap::from([
                ("id".to_string(), 0usize),
                ("ref".to_string(), 1usize),
            ]),
            right_column_indices: HashMap::new(),
            right_plan: Some(PhysicalPlan::SingleRow),
            database: Some(Arc::new(Database::open_in_memory().await.unwrap())),
            snapshot: None,
            routed_tables: Arc::new(crate::pipeline::RoutedTables::new()),
        }
    }

    #[tokio::test]
    async fn missing_outer_column_reports_error_naming_column() {
        let mut exec = AntiJoinExecutor::new(config("missing_col").await);
        let err = exec
            .next()
            .await
            .expect_err("绑参未命中应报错而非静默丢弃该参数");
        let msg = err.to_string();
        assert!(
            msg.contains("missing_col"),
            "错误应点名未命中的外层列: {msg}"
        );
    }
}
