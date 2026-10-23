//! Nested Loop Join executor - INNER/LEFT/FULL JOIN for non-equi / mixed ON conditions
//!
//! MS09-T02 (I015): the Hash Join executor only evaluates column=column
//! equality legs; any other ON shape previously failed at plan time. The NLJ
//! executor evaluates the complete ON predicate over each left × right
//! combination row (`left_row ++ right_row`), so non-equality, literals and
//! mixed AND legs all reach execution. Three-valued semantics come from the
//! shared predicate kernel: `Unknown` folds to no-match (same
//! `evaluate()` source as FilterExecutor).
//!
//! MS25 T1 (design D1): the same loop also carries the outer-join forms. A
//! preserved-side row with no match is emitted once with the opposite side's
//! columns padded to NULL (`Left`), and `Full` additionally keeps a matched
//! bitmap over the materialized right rows so unmatched right rows are emitted
//! in right-row order after the left input is exhausted. `RIGHT JOIN` reaches
//! here as a `Left` node whose inputs the planner already swapped (unmatched
//! original-left rows are dropped), so the padding side follows the preserved
//! side of the rewritten plan.

use crate::executor::query_budget::{row_bytes, unlimited_budget, SharedBudget};
use crate::executor::{
    ExecResult, Executor, JoinSide, NestedLoopJoinType, OutputColumn, PredicateRef, Value,
};
use crate::storage::Result;

/// Nested Loop Join executor - combination-row predicate evaluation
pub struct NestedLoopJoinExecutor {
    left_executor: Box<dyn Executor + Send>,
    right_executor: Box<dyn Executor + Send>,
    predicate: PredicateRef,
    output_columns: Vec<OutputColumn>,
    /// 输出列输入侧归属（T9 replan design D9，与 output_columns 同序同长；
    /// RIGHT 交换由计划期反转）
    output_sides: Vec<JoinSide>,
    /// MS25 T1: 连接语义（Inner = 变更前逐字节一致的既有形态）
    join_type: NestedLoopJoinType,

    // 执行状态：右输入物化一次，左输入流式逐行 × 右行逐组合求值
    right_rows: Vec<Vec<Value>>,
    right_materialized: bool,
    current_left: Option<Vec<Value>>,
    current_right_index: usize,
    /// MS25 T1: 当前左行是否已匹配过任一右行（Left/Full 的补 NULL 判定）
    left_matched: bool,
    /// MS25 T1: 右行匹配位（Full 的左输入耗尽后补齐用；内存上界 = 右行数）
    right_matched: Vec<bool>,
    /// MS25 T1: Full 补齐阶段的右行游标（与匹配阶段的 `current_right_index`
    /// 分离——左输入耗尽时后者已等于右行数）
    drain_index: usize,
    /// MS32 Iter003（design D5）：语句级查询内存预算——右行物化入账。
    budget: SharedBudget,
}

impl NestedLoopJoinExecutor {
    /// 创建新的 NestedLoopJoinExecutor
    pub fn new(
        left_executor: Box<dyn Executor + Send>,
        right_executor: Box<dyn Executor + Send>,
        predicate: PredicateRef,
        output_columns: Vec<OutputColumn>,
        output_sides: Vec<JoinSide>,
        join_type: NestedLoopJoinType,
    ) -> Self {
        Self {
            left_executor,
            right_executor,
            predicate,
            output_columns,
            output_sides,
            join_type,
            right_rows: Vec::new(),
            right_materialized: false,
            current_left: None,
            current_right_index: 0,
            left_matched: false,
            right_matched: Vec::new(),
            drain_index: 0,
            budget: unlimited_budget(),
        }
    }

    /// MS32 Iter003（design D5）：注入语句级预算（pipeline 构造臂）。
    pub(crate) fn with_budget(mut self, budget: SharedBudget) -> Self {
        self.budget = budget;
        self
    }

    /// 构建输出行（根据 output_columns 从左/右组合侧提取列，
    /// 与 executor/join.rs `build_output_row` 同型）
    ///
    /// T9 replan（design D9）：归属按计划期 `output_sides` 侧标志——自连接
    /// 两侧真实名相同使表名比较恒读一侧。
    fn build_output_row(&self, left_row: &[Value], right_row: &[Value]) -> Vec<Value> {
        self.output_columns
            .iter()
            .zip(&self.output_sides)
            .map(|(col, side)| match side {
                JoinSide::Left => left_row[col.column_index].clone(),
                JoinSide::Right => right_row[col.column_index].clone(),
            })
            .collect()
    }

    /// MS25 T1: 外连接的补齐行——`output_sides` 归属侧取原值，缺席侧
    /// （`None`）填 NULL。只发生在产出层，不改写任何存储行或谓词语义。
    fn build_padded_row(
        &self,
        left_row: Option<&[Value]>,
        right_row: Option<&[Value]>,
    ) -> Vec<Value> {
        self.output_columns
            .iter()
            .zip(&self.output_sides)
            .map(|(col, side)| {
                let source = match side {
                    JoinSide::Left => left_row,
                    JoinSide::Right => right_row,
                };
                match source {
                    Some(row) => row[col.column_index].clone(),
                    None => Value::Null,
                }
            })
            .collect()
    }

    /// MS25 T1: Full 语义下取下一个未匹配的右行（左侧补 NULL）。非 Full 或
    /// 右行全部匹配时返回 `None`（结束流）。
    fn next_unmatched_right_row(&mut self) -> Option<Vec<Value>> {
        if self.join_type != NestedLoopJoinType::Full {
            return None;
        }
        while self.drain_index < self.right_rows.len() {
            let index = self.drain_index;
            self.drain_index += 1;
            if self.right_matched[index] {
                continue;
            }
            let right_row = self.right_rows[index].clone();
            return Some(self.build_padded_row(None, Some(&right_row)));
        }
        None
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
                    // MS32 Iter003（design D5）：右行物化逐行入账，驻留持续
                    // 到语句结束；左行流式不计。
                    self.budget.charge(row_bytes(&row))?;
                    self.right_rows.push(row);
                }
            }
            if self.join_type == NestedLoopJoinType::Full {
                self.right_matched = vec![false; self.right_rows.len()];
            }
        }

        loop {
            // Phase 2: 载入当前左行（流式）
            if self.current_left.is_none() {
                match self.left_executor.next().await? {
                    // 左输入耗尽：Full 语义在此补齐未匹配右行后结束
                    None => {
                        return Ok(self.next_unmatched_right_row().map(ExecResult::Row));
                    }
                    Some(ExecResult::Row(row)) => {
                        self.current_left = Some(row);
                        self.current_right_index = 0;
                        self.left_matched = false;
                    }
                    // 左输入为扫描链，非行结果按 JoinExecutor 先例丢弃
                    Some(_) => continue,
                }
            }
            let left_row = self.current_left.as_ref().expect("current left row loaded");

            // Phase 3: 当前左行 × 右行逐组合求值谓词
            while self.current_right_index < self.right_rows.len() {
                let right_index = self.current_right_index;
                let right_row = &self.right_rows[right_index];
                let combined: Vec<Value> = left_row
                    .iter()
                    .cloned()
                    .chain(right_row.iter().cloned())
                    .collect();
                match self.predicate.evaluate(&combined) {
                    Ok(true) => {
                        self.current_right_index += 1;
                        self.left_matched = true;
                        if self.join_type == NestedLoopJoinType::Full {
                            self.right_matched[right_index] = true;
                        }
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

            // MS25 T1: Left/Full 语义下当前左行无匹配 → 补 NULL 右列产出一行。
            // 置位匹配标志：本保存侧行的唯一输出已发出（否则下次 `next()` 会在
            // 右组合耗尽态上重复产出同一行）。
            if self.join_type != NestedLoopJoinType::Inner && !self.left_matched {
                self.left_matched = true;
                return Ok(Some(ExecResult::Row(
                    self.build_padded_row(Some(left_row), None),
                )));
            }

            // 当前左行组合耗尽 → 拉取下一左行
            self.current_left = None;
        }
    }
}
