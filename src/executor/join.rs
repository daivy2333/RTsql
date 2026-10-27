//! Join executor - INNER JOIN using hash join algorithm

use crate::executor::query_budget::{charge_bucket_insert, unlimited_budget, SharedBudget};
use crate::executor::{
    ColumnRef, ExecResult, Executor, JoinCondition, JoinConfig, JoinSide, OutputColumn, Value,
};
use crate::storage::Result;
use std::collections::HashMap;

/// JOIN 执行阶段
#[derive(Debug, Clone, PartialEq)]
enum JoinPhase {
    /// 构建右表哈希表
    BuildRight,
    /// 左表逐行探测并输出匹配结果
    Output,
}

/// Join executor - 哈希连接实现
pub struct JoinExecutor {
    left_executor: Box<dyn Executor + Send>,
    right_executor: Box<dyn Executor + Send>,
    conditions: Vec<JoinCondition>,
    output_columns: Vec<OutputColumn>,

    // 列索引映射
    /// 左表列名到索引的映射
    left_column_indices: HashMap<String, usize>,
    /// 右表列名到索引的映射
    right_column_indices: HashMap<String, usize>,
    /// 输出列输入侧归属（T9 replan design D9，与 output_columns 同序同长）
    output_sides: Vec<JoinSide>,
    /// 右表名称
    _right_table_name: String,

    // 哈希连接状态
    right_hashmap: HashMap<Vec<Value>, Vec<Vec<Value>>>,
    /// 当前探测中的左表行（None = 需从左执行器取下一行，左侧行不整表驻留）
    current_left: Option<Vec<Value>>,
    /// 当前行命中桶的键，探测时缓存；桶按需经键取回，不整桶克隆
    current_match_key: Option<Vec<Value>>,
    current_right_index: usize,
    /// MS32 Iter003（design D5）：语句级查询内存预算——右行 + 哈希键入账；
    /// 左行只作当前探测行不计。
    budget: SharedBudget,

    phase: JoinPhase,
    executed: bool,
}

impl JoinExecutor {
    /// 创建新的 JoinExecutor
    pub fn new(config: JoinConfig) -> Self {
        Self {
            left_executor: config.left_executor,
            right_executor: config.right_executor,
            conditions: config.conditions,
            output_columns: config.output_columns,
            left_column_indices: config.left_column_indices,
            right_column_indices: config.right_column_indices,
            output_sides: config.output_sides,
            _right_table_name: config.right_table_name,
            right_hashmap: HashMap::new(),
            current_left: None,
            current_match_key: None,
            current_right_index: 0,
            budget: unlimited_budget(),
            phase: JoinPhase::BuildRight,
            executed: false,
        }
    }

    /// MS32 Iter003（design D5）：注入语句级预算（pipeline 构造臂）。
    pub(crate) fn with_budget(mut self, budget: SharedBudget) -> Self {
        self.budget = budget;
        self
    }

    /// T11（change 2026-10-04-external-review-gaps）：条件键列位查表——
    /// `(真实表, 列)` 限定键优先、裸名回退。链式 JOIN 的左输入含多表同名
    /// 列，裸名查表静默串列；pipeline 侧已注册限定键（唯一命中才注册），
    /// 测试直构的裸名 map / 未注册形态回落既有语义。查无 → `None`，由
    /// 调用方既有 `.expect` 防御臂处理。
    fn condition_column_index(map: &HashMap<String, usize>, column: &ColumnRef) -> Option<usize> {
        if let Some(table) = &column.table {
            let qualified = format!("{}.{}", table.to_lowercase(), column.column.to_lowercase());
            if let Some(idx) = map.get(&qualified) {
                return Some(*idx);
            }
        }
        map.get(&column.column).copied()
    }

    /// 计算右表行的哈希键（ON 条件右表列值组合）
    /// 返回 None 表示键包含 NULL，在 INNER JOIN 中不会匹配任何行
    fn build_hash_key_right(&self, row: &[Value]) -> Option<Vec<Value>> {
        let key: Vec<Value> = self
            .conditions
            .iter()
            .map(|cond| {
                let idx =
                    Self::condition_column_index(&self.right_column_indices, &cond.right_column)
                        .expect("right column index must exist");
                row[idx].clone()
            })
            .collect();

        // SQL semantics: NULL never matches NULL in joins
        if key.iter().any(|v| matches!(v, Value::Null)) {
            return None;
        }
        Some(Self::normalize_signed_zero_key(key))
    }

    /// 计算左表行的哈希键（ON 条件左表列值组合）
    /// 返回 None 表示键包含 NULL，在 INNER JOIN 中不会匹配任何行
    fn build_hash_key_left(&self, row: &[Value]) -> Option<Vec<Value>> {
        let key: Vec<Value> = self
            .conditions
            .iter()
            .map(|cond| {
                let idx =
                    Self::condition_column_index(&self.left_column_indices, &cond.left_column)
                        .expect("left column index must exist");
                row[idx].clone()
            })
            .collect();

        // SQL semantics: NULL never matches NULL in joins
        if key.iter().any(|v| matches!(v, Value::Null)) {
            return None;
        }
        Some(Self::normalize_signed_zero_key(key))
    }

    /// 交给哈希表的键把每个等于零的浮点收成正零。
    /// `Hash` 使用 `to_bits()`，负零和正零会进不同的桶；`PartialEq` 却把它们看成相等。
    /// 桶里的行和输出行不经过这里。
    fn normalize_signed_zero_key(mut key: Vec<Value>) -> Vec<Value> {
        for value in &mut key {
            if let Value::Float(f) = value {
                if *f == 0.0 {
                    *value = Value::Float(0.0);
                }
            }
        }
        key
    }

    /// 构建输出行（根据 output_columns 从左/右表提取列）
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
}

#[async_trait::async_trait]
impl Executor for JoinExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if !self.executed {
            self.executed = true;
            self.phase = JoinPhase::BuildRight;
        }

        loop {
            match self.phase {
                JoinPhase::BuildRight => {
                    // Phase 1: 执行右表，构建哈希表
                    while let Some(result) = self.right_executor.next().await? {
                        if let ExecResult::Row(row) = result {
                            if let Some(hash_key) = self.build_hash_key_right(&row) {
                                // MS32 Iter003（design D5）：右行逐行入账、哈希
                                // 键按桶入账（单源助手）；驻留持续到语句结束，
                                // 左行只作当前探测行不计。
                                charge_bucket_insert(
                                    &self.budget,
                                    &mut self.right_hashmap,
                                    hash_key,
                                    row,
                                )?;
                            }
                        }
                    }
                    self.phase = JoinPhase::Output;
                }

                JoinPhase::Output => {
                    // Phase 2: 左表逐行探测，左侧行不缓存
                    if self.current_left.is_none() {
                        match self.left_executor.next().await? {
                            Some(ExecResult::Row(row)) => {
                                self.current_left = Some(row);
                                self.current_right_index = 0;
                            }
                            Some(_) => continue,
                            // 左表已耗尽
                            None => return Ok(None),
                        }
                    }

                    // 首次探测当前左表行
                    if self.current_right_index == 0 {
                        let left_row = self.current_left.as_ref().unwrap();
                        match self.build_hash_key_left(left_row) {
                            // 键含 NULL 不会匹配任何右表行
                            None => {
                                self.current_left = None;
                                continue;
                            }
                            Some(hash_key) => self.current_match_key = Some(hash_key),
                        }
                    }

                    // 查找匹配的右表行，按下标取行，不克隆整桶
                    let hash_key = self
                        .current_match_key
                        .as_ref()
                        .expect("match key cached while probing");
                    if let Some(bucket) = self.right_hashmap.get(hash_key) {
                        if self.current_right_index < bucket.len() {
                            let right_row = &bucket[self.current_right_index];
                            let left_row = self.current_left.as_ref().unwrap();
                            let output_row = self.build_output_row(left_row, right_row);
                            self.current_right_index += 1;
                            return Ok(Some(ExecResult::Row(output_row)));
                        }
                    }

                    // 当前左表行所有匹配已输出（或无匹配），移到下一行
                    self.current_left = None;
                }
            }
        }
    }
}
