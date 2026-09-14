//! Nested Loop Join executor - INNER JOIN for non-equi / mixed ON conditions
//!
//! MS09-T02 (I015): the Hash Join executor only evaluates column=column
//! equality legs; any other ON shape previously failed at plan time. The NLJ
//! executor evaluates the complete ON predicate over each left × right
//! combination row (`left_row ++ right_row`), so non-equality, literals and
//! mixed AND legs all reach execution. Three-valued semantics come from the
//! shared predicate kernel: `Unknown` folds to no-match (same
//! `evaluate()` source as FilterExecutor).

use crate::executor::{ExecResult, Executor, OutputColumn, PredicateRef, Value};
use crate::storage::Result;

/// Nested Loop Join executor - combination-row predicate evaluation
pub struct NestedLoopJoinExecutor {
    left_executor: Box<dyn Executor + Send>,
    right_executor: Box<dyn Executor + Send>,
    predicate: PredicateRef,
    output_columns: Vec<OutputColumn>,
    /// 左输入链首表名（output_columns 归属判定，与 JoinExecutor 同型）
    left_table_name: String,

    // 执行状态：右输入物化一次，左输入流式逐行 × 右行逐组合求值
    right_rows: Vec<Vec<Value>>,
    right_materialized: bool,
    current_left: Option<Vec<Value>>,
    current_right_index: usize,
}

impl NestedLoopJoinExecutor {
    /// 创建新的 NestedLoopJoinExecutor
    pub fn new(
        left_executor: Box<dyn Executor + Send>,
        right_executor: Box<dyn Executor + Send>,
        predicate: PredicateRef,
        output_columns: Vec<OutputColumn>,
        left_table_name: String,
    ) -> Self {
        Self {
            left_executor,
            right_executor,
            predicate,
            output_columns,
            left_table_name,
            right_rows: Vec::new(),
            right_materialized: false,
            current_left: None,
            current_right_index: 0,
        }
    }

    /// 构建输出行（根据 output_columns 从左/右组合侧提取列，
    /// 与 executor/join.rs `build_output_row` 同型）
    fn build_output_row(&self, left_row: &[Value], right_row: &[Value]) -> Vec<Value> {
        self.output_columns
            .iter()
            .map(|col| {
                if col.table_alias == self.left_table_name {
                    left_row[col.column_index].clone()
                } else {
                    right_row[col.column_index].clone()
                }
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Executor for NestedLoopJoinExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // Phase 1: 物化右输入行集（一次）
        if !self.right_materialized {
            self.right_materialized = true;
            while let Some(result) = self.right_executor.next().await? {
                if let ExecResult::Row(row) = result {
                    self.right_rows.push(row);
                }
            }
        }

        loop {
            // Phase 2: 载入当前左行（流式）
            if self.current_left.is_none() {
                match self.left_executor.next().await? {
                    None => return Ok(None),
                    Some(ExecResult::Row(row)) => {
                        self.current_left = Some(row);
                        self.current_right_index = 0;
                    }
                    // 左输入为扫描链，非行结果按 JoinExecutor 先例丢弃
                    Some(_) => continue,
                }
            }
            let left_row = self.current_left.as_ref().expect("current left row loaded");

            // Phase 3: 当前左行 × 右行逐组合求值谓词
            while self.current_right_index < self.right_rows.len() {
                let right_row = &self.right_rows[self.current_right_index];
                let combined: Vec<Value> = left_row
                    .iter()
                    .cloned()
                    .chain(right_row.iter().cloned())
                    .collect();
                match self.predicate.evaluate(&combined) {
                    Ok(true) => {
                        self.current_right_index += 1;
                        return Ok(Some(ExecResult::Row(
                            self.build_output_row(left_row, right_row),
                        )));
                    }
                    Ok(false) => self.current_right_index += 1,
                    Err(e) => {
                        return Err(crate::storage::StorageError::ExecutionError(format!(
                            "Predicate evaluation error: {}",
                            e
                        )))
                    }
                }
            }

            // 当前左行组合耗尽 → 拉取下一左行
            self.current_left = None;
        }
    }
}
