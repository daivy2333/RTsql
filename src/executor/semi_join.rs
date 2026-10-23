//! Semi-Join executor - IN/EXISTS subquery unnesting
//!
//! Outputs only left-table rows that have a match in the right table.
//! - EXISTS mode: conditions is empty, only checks right table has any rows
//! - IN mode: conditions non-empty, builds hash key from left row and probes right hash map
//! - Correlated subquery: correlated_params non-empty, re-materializes right per left row (placeholder)

use crate::database::Database;
use crate::executor::query_budget::{
    charge_bucket_insert, param_key_bytes, rows_bytes, unlimited_budget, SharedBudget,
};
use crate::executor::{
    apply_projection, ExecResult, Executor, JoinCondition, JoinRelatedConfig, PhysicalPlan, Value,
};
use crate::storage::Result;
use crate::transaction::Snapshot;
use std::collections::HashMap;
use std::sync::Arc;

/// Semi-Join execution phase
#[derive(Debug, Clone, PartialEq)]
enum SemiJoinPhase {
    /// Build right hash table
    BuildRight,
    /// Scan left and output matching rows
    ScanLeft,
    /// All done
    Done,
}

/// Semi-Join executor - outputs left-table rows that match right table
///
/// Used by Pipeline::create_executor_from_plan for non-correlated subqueries.
/// For correlated subqueries (correlated_params non-empty), the right table
/// re-materialization is a placeholder to be completed in Task 10.
pub struct SemiJoinExecutorV2 {
    left: Box<dyn Executor + Send>,
    right: Box<dyn Executor + Send>,
    conditions: Vec<JoinCondition>,
    /// T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：SELECT
    /// 清单裁剪位（空 = 恒等全宽输出——ORDER BY 存在时裁剪归 Sort 承担）。
    projection: Vec<usize>,
    /// Correlated subquery parameters (empty = independent subquery)
    correlated_params: Vec<crate::executor::CorrelatedParam>,

    /// Left column name to index mapping (for finding condition columns)
    left_column_indices: HashMap<String, usize>,
    /// Right column name to index mapping (built by Pipeline from extract_column_indices)
    right_column_indices: HashMap<String, usize>,

    /// Right table hash map: key = right condition column values, value = list of matching rows
    right_hashmap: Option<HashMap<Vec<Value>, Vec<Vec<Value>>>>,
    /// Whether the right table has any rows (for EXISTS mode)
    right_has_rows: Option<bool>,

    /// Right plan for correlated subqueries (cloned per left row)
    right_plan: Option<PhysicalPlan>,
    /// Database for creating executors from plans (correlated path)
    database: Option<Arc<Database>>,
    /// MS20 T2: cross-db route table re-applied on every correlated
    /// right-plan rebuild (empty outside an ATTACH session).
    routed_tables: Arc<crate::pipeline::RoutedTables>,
    /// MS09 Iter000 (D4): statement-start Read Committed snapshot re-applied
    /// on every correlated right-plan rebuild (`None` under RepeatableRead).
    snapshot: Option<Snapshot>,
    /// MS09 Iter002 (D6): statement-level correlated-result cache keyed by the
    /// correlated parameter value sequence. Stores the full right row set
    /// (including NULL-key rows, which count toward `right_has_rows`). Errors
    /// are never cached.
    correlated_rows_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>,
    /// MS32 Iter003（design D5）：语句级查询内存预算——右哈希表（键 + 行
    /// 克隆）与相关缓存（键 + 行集）入账；相关重建换表时旧表按记账字节
    /// 出账。
    budget: SharedBudget,
    /// 当前右哈希表的入账字节数（相关重建路径的换表出账依据）。
    right_residency: u64,

    phase: SemiJoinPhase,
    executed: bool,
}

impl SemiJoinExecutorV2 {
    /// Create a new SemiJoinExecutorV2
    pub fn new(config: JoinRelatedConfig) -> Self {
        Self {
            left: config.left,
            right: config.right,
            conditions: config.conditions,
            projection: config.projection,
            correlated_params: config.correlated_params,
            left_column_indices: config.left_column_indices,
            right_column_indices: config.right_column_indices,
            right_hashmap: None,
            right_has_rows: None,
            right_plan: config.right_plan,
            database: config.database,
            snapshot: config.snapshot,
            routed_tables: config.routed_tables,
            correlated_rows_cache: HashMap::new(),
            budget: unlimited_budget(),
            right_residency: 0,
            phase: SemiJoinPhase::BuildRight,
            executed: false,
        }
    }

    /// MS32 Iter003（design D5）：注入语句级预算（pipeline 构造臂）。
    pub(crate) fn with_budget(mut self, budget: SharedBudget) -> Self {
        self.budget = budget;
        self
    }

    /// MS32 Iter003（design D5）：相关重建换表——旧表先按记账字节出账。
    /// 按字段传递（调用点同时持有 `right_plan` / `database` 的字段借用）。
    fn release_right_residency(budget: &SharedBudget, residency: &mut u64) {
        budget.release(std::mem::take(residency));
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

    /// Build output row from left-table row (T5 D8：output_columns 为基表全列，
    /// 行内容 = 左行直通；SELECT 清单裁剪由本节点 `projection` 在直通后应用——
    /// ORDER BY 存在时 projection 为空，裁剪归 Sort）。
    fn build_output_row(&self, left_row: &[Value]) -> Vec<Value> {
        apply_projection(&self.projection, left_row.to_vec())
    }

    /// Extract outer row values for each correlated parameter
    ///
    /// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：未命中
    /// SHALL 报错点名该外层列——修复前 `unwrap_or(0)` 静默取外层首列，是别名
    /// 幻影参数静默错结果的执行器侧放大器（ISS16/ISS17/ISS18）。
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
                Ok((
                    cp.param_name.clone(),
                    left_row.get(idx).cloned().unwrap_or(Value::Null),
                ))
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Executor for SemiJoinExecutorV2 {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if !self.executed {
            self.executed = true;
            self.phase = SemiJoinPhase::BuildRight;
        }

        loop {
            match self.phase {
                SemiJoinPhase::BuildRight => {
                    let is_correlated = !self.correlated_params.is_empty();
                    if !is_correlated {
                        // Independent subquery: materialize right once (fast path)
                        let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> = HashMap::new();
                        let mut has_rows = false;

                        while let Some(result) = self.right.next().await? {
                            if let ExecResult::Row(row) = result {
                                has_rows = true;
                                if let Some(hash_key) = self.build_right_key(&row) {
                                    // MS32 Iter003（design D5）：独立路径单次
                                    // 建表，驻留到语句结束（单源计费助手）。
                                    charge_bucket_insert(
                                        &self.budget,
                                        &mut hashmap,
                                        hash_key,
                                        row,
                                    )?;
                                }
                            }
                        }

                        self.right_hashmap = Some(hashmap);
                        self.right_has_rows = Some(has_rows);
                    } else {
                        // Correlated: right will be rebuilt per left row in ScanLeft
                        self.right_hashmap = Some(HashMap::new());
                        self.right_has_rows = Some(false);
                    }
                    self.phase = SemiJoinPhase::ScanLeft;
                }

                SemiJoinPhase::ScanLeft => {
                    // Scan left table row by row
                    loop {
                        match self.left.next().await? {
                            Some(ExecResult::Row(left_row)) => {
                                // CORRELATED PATH: rebuild right for this left row
                                if !self.correlated_params.is_empty() {
                                    let param_values = self.extract_param_values(&left_row)?;
                                    if let Some(rows) =
                                        self.correlated_rows_cache.get(&param_values)
                                    {
                                        // Cache hit: rebuild the hash map and row
                                        // presence exactly as the miss path does.
                                        Self::release_right_residency(
                                            &self.budget,
                                            &mut self.right_residency,
                                        );
                                        let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> =
                                            HashMap::new();
                                        let mut residency = 0u64;
                                        for row in rows {
                                            if let Some(hash_key) = self.build_right_key(row) {
                                                residency += charge_bucket_insert(
                                                    &self.budget,
                                                    &mut hashmap,
                                                    hash_key,
                                                    row.clone(),
                                                )?;
                                            }
                                        }
                                        let has_rows = !rows.is_empty();
                                        self.right_residency = residency;
                                        self.right_hashmap = Some(hashmap);
                                        self.right_has_rows = Some(has_rows);
                                    } else {
                                        let plan = self.right_plan.as_ref().unwrap();
                                        let db = self.database.as_ref().unwrap();
                                        let cloned_plan = plan.clone();
                                        crate::executor::inject_correlated_values(
                                            &cloned_plan,
                                            &param_values,
                                        );
                                        let mut right_exec =
                                            crate::pipeline::create_executor_from_plan(
                                                cloned_plan,
                                                db,
                                                None,
                                                self.snapshot.clone(),
                                                &self.routed_tables,
                                                &self.budget,
                                            )
                                            .await?;
                                        Self::release_right_residency(
                                            &self.budget,
                                            &mut self.right_residency,
                                        );
                                        let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> =
                                            HashMap::new();
                                        let mut has_rows = false;
                                        let mut collected: Vec<Vec<Value>> = Vec::new();
                                        let mut residency = 0u64;
                                        while let Some(result) = right_exec.next().await? {
                                            if let ExecResult::Row(row) = result {
                                                has_rows = true;
                                                if let Some(hash_key) = self.build_right_key(&row) {
                                                    residency += charge_bucket_insert(
                                                        &self.budget,
                                                        &mut hashmap,
                                                        hash_key,
                                                        row.clone(),
                                                    )?;
                                                }
                                                collected.push(row);
                                            }
                                        }
                                        // Store only after a successful drain.
                                        // MS32 Iter003（design D5）：缓存驻留 =
                                        // 相关参数键 + 行集，入账先于插入。
                                        self.budget.charge(
                                            param_key_bytes(&param_values) + rows_bytes(&collected),
                                        )?;
                                        self.right_residency = residency;
                                        self.correlated_rows_cache.insert(param_values, collected);
                                        self.right_hashmap = Some(hashmap);
                                        self.right_has_rows = Some(has_rows);
                                    }
                                }

                                // Re-read hashmap after potential correlated rebuild
                                let hashmap = self.right_hashmap.as_ref().unwrap();
                                let has_rows = self.right_has_rows.unwrap();

                                // EXISTS mode: conditions empty, check if right has any rows
                                if self.conditions.is_empty() {
                                    if has_rows {
                                        return Ok(Some(ExecResult::Row(
                                            self.build_output_row(&left_row),
                                        )));
                                    }
                                    continue;
                                }

                                // IN mode: build left key and probe right hash map
                                match self.build_left_key(&left_row) {
                                    Some(key) => {
                                        if hashmap.contains_key(&key) {
                                            return Ok(Some(ExecResult::Row(
                                                self.build_output_row(&left_row),
                                            )));
                                        }
                                        continue;
                                    }
                                    None => {
                                        // NULL in key, no match possible
                                        continue;
                                    }
                                }
                            }
                            Some(ExecResult::AffectedRows(_)) => continue,
                            Some(ExecResult::RowId(_)) => continue,
                            None => {
                                self.phase = SemiJoinPhase::Done;
                                break;
                            }
                        }
                    }
                }

                SemiJoinPhase::Done => {
                    return Ok(None);
                }
            }
        }
    }
}

/// T2（change 2026-10-15-subquery-alias-binding-and-error-prefix）：关联参数外层
/// 绑定期望存在而解析失败时显式报错点名列，SHALL NOT 静默回落外层首列。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::JoinRelatedConfig;
    use crate::executor::{CorrelatedParam, StubRowExecutor};

    fn config(outer_column: &str) -> JoinRelatedConfig {
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
            // 外层行只有 id / ref 两列，`missing_col` 不在其中。
            left_column_indices: HashMap::from([
                ("id".to_string(), 0usize),
                ("ref".to_string(), 1usize),
            ]),
            right_column_indices: HashMap::new(),
            right_plan: None,
            database: None,
            snapshot: None,
            routed_tables: Arc::new(crate::pipeline::RoutedTables::new()),
        }
    }

    #[tokio::test]
    async fn missing_outer_column_reports_error_naming_column() {
        let mut exec = SemiJoinExecutorV2::new(config("missing_col"));
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
