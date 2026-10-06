//! Join executor - INNER JOIN using hash join algorithm

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
    /// 扫描左表并缓存
    ScanLeft,
    /// 输出匹配结果
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
    left_rows: Vec<Vec<Value>>,
    current_left_index: usize,
    current_right_matches: Vec<Vec<Value>>,
    current_right_index: usize,

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
            left_rows: Vec::new(),
            current_left_index: 0,
            current_right_matches: Vec::new(),
            current_right_index: 0,
            phase: JoinPhase::BuildRight,
            executed: false,
        }
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
        Some(key)
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
        Some(key)
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
                                self.right_hashmap.entry(hash_key).or_default().push(row);
                            }
                        }
                    }
                    self.phase = JoinPhase::ScanLeft;
                }

                JoinPhase::ScanLeft => {
                    // Phase 2: 执行左表，缓存所有行
                    while let Some(result) = self.left_executor.next().await? {
                        if let ExecResult::Row(row) = result {
                            self.left_rows.push(row);
                        }
                    }
                    self.phase = JoinPhase::Output;
                }

                JoinPhase::Output => {
                    // Phase 3: 逐行匹配输出
                    while self.current_left_index < self.left_rows.len() {
                        let left_row = &self.left_rows[self.current_left_index];

                        // Skip NULL keys (no match possible)
                        let hash_key = match self.build_hash_key_left(left_row) {
                            Some(key) => key,
                            None => {
                                self.current_left_index += 1;
                                continue;
                            }
                        };

                        // 查找匹配的右表行
                        if self.current_right_index == 0 {
                            self.current_right_matches = self
                                .right_hashmap
                                .get(&hash_key)
                                .cloned()
                                .unwrap_or_default();
                        }

                        // 输出当前匹配
                        if self.current_right_index < self.current_right_matches.len() {
                            let right_row = &self.current_right_matches[self.current_right_index];
                            let output_row = self.build_output_row(left_row, right_row);
                            self.current_right_index += 1;
                            return Ok(Some(ExecResult::Row(output_row)));
                        }

                        // 当前左表行所有匹配已输出，移到下一行
                        self.current_left_index += 1;
                        self.current_right_index = 0;
                    }

                    // 所有行已处理完毕
                    return Ok(None);
                }
            }
        }
    }
}
