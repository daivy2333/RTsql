//! Semi-Join executor - IN/EXISTS subquery unnesting
//!
//! Outputs only left-table rows that have a match in the right table.
//! - EXISTS mode: conditions is empty, only checks right table has any rows
//! - IN mode: conditions non-empty, builds hash key from left row and probes right hash map
//! - Correlated subquery: correlated_params non-empty, re-materializes right per left row (placeholder)

use crate::database::Database;
use crate::executor::{ExecResult, Executor, JoinCondition, OutputColumn, PhysicalPlan, Value};
use crate::storage::Result;
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
    output_columns: Vec<OutputColumn>,
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

    phase: SemiJoinPhase,
    executed: bool,
}

impl SemiJoinExecutorV2 {
    /// Create a new SemiJoinExecutorV2
    pub fn new(
        left: Box<dyn Executor + Send>,
        right: Box<dyn Executor + Send>,
        conditions: Vec<JoinCondition>,
        output_columns: Vec<OutputColumn>,
        correlated_params: Vec<crate::executor::CorrelatedParam>,
        left_column_indices: HashMap<String, usize>,
        right_column_indices: HashMap<String, usize>,
        right_plan: Option<PhysicalPlan>,
        database: Option<Arc<Database>>,
    ) -> Self {
        Self {
            left,
            right,
            conditions,
            output_columns,
            correlated_params,
            left_column_indices,
            right_column_indices,
            right_hashmap: None,
            right_has_rows: None,
            right_plan,
            database,
            phase: SemiJoinPhase::BuildRight,
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

    /// Build output row from left-table row (semi-join only outputs left columns)
    fn build_output_row(&self, left_row: &[Value]) -> Vec<Value> {
        self.output_columns
            .iter()
            .map(|oc| left_row[oc.column_index].clone())
            .collect()
    }

    /// Extract outer row values for each correlated parameter
    fn extract_param_values(&self, left_row: &[Value]) -> Vec<(String, Value)> {
        self.correlated_params
            .iter()
            .map(|cp| {
                let idx = self
                    .left_column_indices
                    .get(&cp.outer_column.to_lowercase())
                    .copied()
                    .unwrap_or(0);
                (cp.param_name.clone(), left_row.get(idx).cloned().unwrap_or(Value::Null))
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
                                    hashmap.entry(hash_key).or_default().push(row);
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
                                    let plan = self.right_plan.as_ref().unwrap();
                                    let db = self.database.as_ref().unwrap();
                                    let cloned_plan = plan.clone();
                                    let param_values = self.extract_param_values(&left_row);
                                    crate::executor::inject_correlated_values(&cloned_plan, &param_values);
                                    let mut right_exec = crate::pipeline::create_executor_from_plan(
                                        cloned_plan, db,
                                    ).await?;
                                    let mut hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>> = HashMap::new();
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
