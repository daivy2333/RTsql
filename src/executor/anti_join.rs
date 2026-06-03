//! Anti-Join executor - NOT IN / NOT EXISTS subquery unnesting
//!
//! Outputs only left-table rows that have NO match in the right table.
//! - NOT EXISTS mode: conditions is empty, checks right table has NO rows
//! - NOT IN mode: conditions non-empty, builds hash key from left row and probes right hash map
//! - Correlated subquery: correlated_params non-empty, re-materializes right per left row (placeholder)

use crate::database::Database;
use crate::executor::{
    CorrelatedParam, ExecResult, Executor, JoinCondition, JoinRelatedConfig, OutputColumn,
    PhysicalPlan, Value,
};
use crate::storage::Result;
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
    output_columns: Vec<OutputColumn>,
    /// Correlated subquery parameters (empty = independent subquery)
    correlated_params: Vec<CorrelatedParam>,

    /// Saved right plan for per-row rebuild (None = independent subquery)
    right_plan: Option<PhysicalPlan>,
    /// Database reference for per-row rebuild (None = independent subquery)
    database: Option<Arc<Database>>,

    /// Left column name to index mapping (for finding condition columns)
    left_column_indices: HashMap<String, usize>,
    /// Right column name to index mapping (built by Pipeline from extract_column_indices)
    right_column_indices: HashMap<String, usize>,

    /// Right table hash map: key = right condition column values, value = list of matching rows
    right_hashmap: Option<HashMap<Vec<Value>, Vec<Vec<Value>>>>,
    /// Whether the right table has any rows (for NOT EXISTS mode)
    right_has_rows: Option<bool>,

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
            output_columns: config.output_columns,
            correlated_params: config.correlated_params,
            right_plan: config.right_plan,
            database: config.database,
            left_column_indices: config.left_column_indices,
            right_column_indices: config.right_column_indices,
            right_hashmap: None,
            right_has_rows: None,
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

    /// Build output row from left-table row (anti-join only outputs left columns)
    fn build_output_row(&self, left_row: &[Value]) -> Vec<Value> {
        self.output_columns
            .iter()
            .map(|oc| left_row[oc.column_index].clone())
            .collect()
    }

    /// Extract correlated parameter values from a left-table row.
    fn extract_param_values(&self, left_row: &[Value]) -> Vec<(String, Value)> {
        self.correlated_params
            .iter()
            .filter_map(|cp| {
                self.left_column_indices
                    .get(&cp.outer_column.to_lowercase())
                    .map(|&idx| (cp.param_name.clone(), left_row[idx].clone()))
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
                                        let param_values = self.extract_param_values(&left_row);
                                        let plan = right_plan.clone();
                                        crate::executor::inject_correlated_values(
                                            &plan,
                                            &param_values,
                                        );
                                        let mut right_exec =
                                            crate::pipeline::create_executor_from_plan(
                                                plan, database,
                                            )
                                            .await?;
                                        let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> =
                                            HashMap::new();
                                        let mut has_rows = false;
                                        while let Some(result) = right_exec.next().await? {
                                            if let ExecResult::Row(row) = result {
                                                has_rows = true;
                                                if let Some(hash_key) = self.build_right_key(&row) {
                                                    hashmap.entry(hash_key).or_default().push(row);
                                                }
                                            }
                                        }
                                        self.right_hashmap = Some(hashmap);
                                        self.right_has_rows = Some(has_rows);
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
