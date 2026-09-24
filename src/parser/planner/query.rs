//! PlanBuilder — SELECT / FROM / JOIN / projection / PK-equality
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::aggregate::{extract_aggregate_func, is_aggregate_expr};
use super::ddl_dml::is_pure_equi_join_on;
use super::expression::expr_to_column_name;
use super::PlanBuilder;
use crate::executor::{
    ColumnExpression, DataScanNode, ExpressionRef, FilterNode, IndexScanNode, OrderByColumn,
    OutputColumn, PhysicalPlan, ProjectionItem, ProjectionNode, ScanNode, SortNode,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::{Expr, FunctionArg, FunctionArgExpr, Query, TableFactor};
use std::collections::HashMap;
use std::sync::Arc;

/// MS13 T8: 解析后的 GROUP BY 键——列名键（`column`）或表达式键（匹配的
/// SELECT 项索引 `expr_item`）。`name` 为键名（列名 / SELECT 项名），用于
/// 输出装配、严格检查与 HAVING 绑定。
pub(crate) struct GroupKey {
    name: String,
    column: Option<String>,
    expr_item: Option<usize>,
}

/// MS15-Rest (I034): describe a scan node's real output shape. The scan
/// executors trim rows by `projection` after predicate evaluation, so the
/// column metadata must be trimmed the same way (same mapping as the
/// Filter/Sort arms in `get_plan_output_columns`). Empty = identity.
fn projected_columns(columns: &[String], projection: &[usize]) -> Vec<String> {
    if projection.is_empty() {
        columns.to_vec()
    } else {
        projection.iter().map(|&i| columns[i].clone()).collect()
    }
}

impl PlanBuilder {
    /// 从 PhysicalPlan 中提取输出列名（用于派生表的列注册）
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn get_plan_output_columns(&self, plan: &PhysicalPlan) -> Vec<String> {
        match plan {
            PhysicalPlan::Scan(node) => projected_columns(&node.columns, &node.projection),
            PhysicalPlan::DataScan(node) => projected_columns(&node.columns, &node.projection),
            PhysicalPlan::DerivedScan(node) => node.columns.clone(),
            PhysicalPlan::Filter(node) => {
                let mut columns = self.get_plan_output_columns(&node.input);
                if !node.projection.is_empty() {
                    // MS10-T01 Iter001: the Filter owns the projection trim —
                    // describe the narrowed output shape.
                    columns = node
                        .projection
                        .iter()
                        .map(|&i| columns[i].clone())
                        .collect();
                }
                columns
            }
            PhysicalPlan::Sort(node) => {
                let mut columns = self.get_plan_output_columns(&node.input);
                if !node.projection.is_empty() {
                    // MS10-T01 Iter001: the Sort owns the projection trim —
                    // describe the narrowed output shape.
                    columns = node
                        .projection
                        .iter()
                        .map(|&i| columns[i].clone())
                        .collect();
                }
                columns
            }
            PhysicalPlan::Limit(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::Aggregate(node) => node.output_columns.clone(),
            PhysicalPlan::Having(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::IndexScan(node) => {
                // MS15-Rest (I034): IndexScan columns are already narrowed to
                // the projected shape at construction (WHERE routing below)
                // and its `projection` indexes point into the base schema —
                // applying them here would double-trim / go out of bounds.
                node.columns.clone()
            }
            PhysicalPlan::IndexScanAll(node) => projected_columns(&node.columns, &node.projection),
            PhysicalPlan::Join(node) => {
                // JOIN 行组装严格按 output_columns 顺序（见 executor/join.rs），
                // 列名直接取自节点，不递归合并左右子计划。
                node.output_columns
                    .iter()
                    .map(|c| c.column.clone())
                    .collect()
            }
            PhysicalPlan::NestedLoopJoin(node) => {
                // MS09-T02: 与 Join 臂同型——行组装严格按 output_columns 顺序
                //（见 executor/nested_loop_join.rs），列名直接取自节点。
                node.output_columns
                    .iter()
                    .map(|c| c.column.clone())
                    .collect()
            }
            PhysicalPlan::SemiJoin(node) => node
                .output_columns
                .iter()
                .map(|c| c.column.clone())
                .collect(),
            PhysicalPlan::AntiJoin(node) => node
                .output_columns
                .iter()
                .map(|c| c.column.clone())
                .collect(),
            PhysicalPlan::SubqueryEval(node) => {
                // MS17-T02/ISS03: 执行器在 result_column_index 插入标量值
                //（越界时 push，见 executor/subquery_eval.rs）——表头镜像该
                // 语义插入标量列名，保证列数与行宽一致。
                let mut columns = self.get_plan_output_columns(&node.input);
                let idx = node.result_column_index.min(columns.len());
                columns.insert(idx, node.output_column.clone());
                columns
            }
            PhysicalPlan::Projection(node) => node.columns.clone(),
            // MS13 T9: SingleRow 无输出列（no-FORM 输入节点，表头由其上的
            // ProjectionNode 承载）。
            PhysicalPlan::SingleRow => Vec::new(),
            PhysicalPlan::Insert(_) | PhysicalPlan::Update(_) | PhysicalPlan::Delete(_) => {
                Vec::new()
            }
            PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => Vec::new(),
        }
    }

    /// MS13 T8（design D12）：解析单个 GROUP BY 项——列名（既有语义优先）
    /// → SELECT 别名（大小写不敏感）→ SELECT 项表达式文本（双侧
    /// `Expr::to_string()` 归一比较）→ 1-based 位置引用；全部不匹配 →
    /// `NonAggregatedColumn` 点名。命中聚合项的别名/文本/位置键显式拒绝
    ///（键不得为聚合）。
    pub(crate) fn resolve_group_by_item(
        &self,
        expr: &Expr,
        projection: &[sqlparser::ast::SelectItem],
        column_indices: &HashMap<String, usize>,
    ) -> Result<GroupKey, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let name = ident.value.clone();
                // 列名（既有语义优先）
                if column_indices.contains_key(&name.to_lowercase()) {
                    return Ok(GroupKey {
                        name: name.clone(),
                        column: Some(name),
                        expr_item: None,
                    });
                }
                // SELECT 别名（大小写不敏感）
                for (i, item) in projection.iter().enumerate() {
                    if let sqlparser::ast::SelectItem::ExprWithAlias { expr: e, alias } = item {
                        if alias.value.eq_ignore_ascii_case(&name) {
                            if is_aggregate_expr(e) {
                                return Err(PlanError::NonAggregatedColumn(name));
                            }
                            return Ok(GroupKey {
                                name: alias.value.clone(),
                                column: None,
                                expr_item: Some(i),
                            });
                        }
                    }
                }
                Err(PlanError::NonAggregatedColumn(name))
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let name = parts[1].value.clone();
                if column_indices.contains_key(&name.to_lowercase()) {
                    Ok(GroupKey {
                        name: name.clone(),
                        column: Some(name),
                        expr_item: None,
                    })
                } else {
                    Err(PlanError::NonAggregatedColumn(name))
                }
            }
            Expr::Value(sqlparser::ast::Value::Number(n, _)) => {
                // 位置引用（1-based，≤ 投影项数）
                let pos: usize = n
                    .parse()
                    .map_err(|_| PlanError::NonAggregatedColumn(n.clone()))?;
                let idx = pos
                    .checked_sub(1)
                    .filter(|i| *i < projection.len())
                    .ok_or_else(|| PlanError::NonAggregatedColumn(n.clone()))?;
                match &projection[idx] {
                    sqlparser::ast::SelectItem::UnnamedExpr(e) => {
                        if is_aggregate_expr(e) {
                            return Err(PlanError::NonAggregatedColumn(e.to_string()));
                        }
                        Ok(GroupKey {
                            name: e.to_string(),
                            column: None,
                            expr_item: Some(idx),
                        })
                    }
                    sqlparser::ast::SelectItem::ExprWithAlias { expr: e, alias } => {
                        if is_aggregate_expr(e) {
                            return Err(PlanError::NonAggregatedColumn(alias.value.clone()));
                        }
                        Ok(GroupKey {
                            name: alias.value.clone(),
                            column: None,
                            expr_item: Some(idx),
                        })
                    }
                    other => Err(PlanError::NonAggregatedColumn(other.to_string())),
                }
            }
            other => {
                // 表达式文本匹配（双侧 to_string 归一比较；聚合项不参与）
                let text = other.to_string();
                for (i, item) in projection.iter().enumerate() {
                    let matched = match item {
                        sqlparser::ast::SelectItem::UnnamedExpr(e) => {
                            !is_aggregate_expr(e) && e.to_string() == text
                        }
                        sqlparser::ast::SelectItem::ExprWithAlias { expr: e, .. } => {
                            !is_aggregate_expr(e) && e.to_string() == text
                        }
                        _ => false,
                    };
                    if matched {
                        let name = match item {
                            sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } => {
                                alias.value.clone()
                            }
                            _ => text.clone(),
                        };
                        return Ok(GroupKey {
                            name,
                            column: None,
                            expr_item: Some(i),
                        });
                    }
                }
                Err(PlanError::NonAggregatedColumn(text))
            }
        }
    }

    /// 构建 FROM + JOIN 链计划（支持列投影）
    pub(crate) fn build_from_clause_with_projection(
        &mut self,
        from: &[sqlparser::ast::TableWithJoins],
        qualified_columns: &[(Option<String>, String)],
    ) -> Result<PhysicalPlan, PlanError> {
        use crate::parser::ast::extract_join_table_name;
        use sqlparser::ast::JoinOperator;

        if from.is_empty() {
            return Err(PlanError::MissingField("FROM clause".into()));
        }

        // 基础表 — 支持 TableFactor::Table（普通表）和 TableFactor::Derived（派生表）
        let (base_plan, base_table) = match &from[0].relation {
            TableFactor::Table { name, .. } => {
                let table_name = object_name_to_table_name(name);
                self.validate_table(&table_name)?;
                let base_columns = self.tables.get(&table_name).cloned().unwrap_or_default();
                let plan = PhysicalPlan::Scan(ScanNode {
                    table_name: table_name.clone(),
                    columns: base_columns.clone(),
                    projection: Vec::new(),
                });
                (plan, table_name)
            }
            TableFactor::Derived {
                subquery, alias, ..
            } => {
                let saved_subquery_ctx = self.building_subquery;
                self.building_subquery = true;
                let subquery_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.building_subquery = saved_subquery_ctx;
                        return Err(e);
                    }
                };
                self.building_subquery = saved_subquery_ctx;
                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.value.to_lowercase())
                    .unwrap_or_else(|| "derived".to_string());
                // 提取子查询输出列名
                let columns = self.get_plan_output_columns(&subquery_plan);
                // 注册派生表列信息（供后续 WHERE/ORDER BY 引用）
                self.register_table(&alias_name, columns.clone(), "");
                let plan = PhysicalPlan::DerivedScan(crate::executor::DerivedScanNode {
                    subquery: Box::new(subquery_plan),
                    alias: alias_name.clone(),
                    columns,
                });
                (plan, alias_name)
            }
            _ => {
                return Err(PlanError::InvalidQuery(
                    "unsupported table factor in FROM clause".into(),
                ))
            }
        };

        // 递归处理 JOIN 链
        let mut current_plan = base_plan;
        let mut current_tables = vec![base_table.clone()];

        for join in &from[0].joins {
            // 验证 JOIN 类型（仅支持 INNER）
            let on_clause = match &join.join_operator {
                JoinOperator::Inner(sqlparser::ast::JoinConstraint::On(expr)) => Some(expr),
                JoinOperator::Inner(_) => None, // USING or None constraint
                _ => return Err(PlanError::UnsupportedJoinType),
            };

            // 解析右表
            let right_table = extract_join_table_name(&join.relation)?;
            self.validate_table(&right_table)?;
            let right_columns = self.tables.get(&right_table).cloned().unwrap_or_default();
            let right_plan = PhysicalPlan::Scan(ScanNode {
                table_name: right_table.clone(),
                columns: right_columns.clone(),
                projection: Vec::new(),
            });

            // 解析 ON 条件
            let on_clause = on_clause.ok_or(PlanError::MissingOnClause)?;

            // 构建输出列（根据 qualified_columns 过滤）—— Hash 与 NLJ 两路由共享，
            // SELECT * 与列过滤行为对两种 join 节点一致。
            let all_columns: Vec<OutputColumn> = current_tables
                .iter()
                .flat_map(|t| {
                    let columns = self
                        .tables
                        .get(t)
                        .expect("validated table must exist in metadata");
                    columns.iter().enumerate().map(|(idx, col)| OutputColumn {
                        table: Some(t.clone()),
                        column: col.clone(),
                        table_alias: t.clone(),
                        column_index: idx,
                    })
                })
                .chain(
                    self.tables
                        .get(&right_table)
                        .expect("validated right_table must exist")
                        .iter()
                        .enumerate()
                        .map(|(idx, col)| OutputColumn {
                            table: Some(right_table.clone()),
                            column: col.clone(),
                            table_alias: right_table.clone(),
                            column_index: idx,
                        }),
                )
                .collect();

            // 根据 qualified_columns 过滤输出列
            let output_columns = if qualified_columns.iter().any(|(_, c)| c == "*") {
                // SELECT *: 输出所有列
                all_columns
            } else {
                // SELECT col1, col2... 或 SELECT t.col1, t.col2...
                all_columns
                    .into_iter()
                    .filter(|col| {
                        qualified_columns.iter().any(|(qual_table, qual_col)| {
                            match qual_table {
                                Some(table) => {
                                    // Qualified column: table.column
                                    col.table.as_deref() == Some(table.as_str())
                                        && col.column.to_lowercase() == qual_col.to_lowercase()
                                }
                                None => {
                                    // Unqualified column: column
                                    col.column.to_lowercase() == qual_col.to_lowercase()
                                }
                            }
                        })
                    })
                    .collect()
            };

            // MS09-T02 (I015): 计划期启发式路由（delta spec R4）。结构探测的
            // 真集恰为 `extract_join_conditions` 的接受面（resolve_column_ref
            // 只接受 Identifier / 2 段 CompoundIdentifier），纯等值形态的 Hash
            // 路径输入集与行为逐字节不变；其余形态（非等值/字面量/表达式腿）
            // 路由 NLJ——原计划期 `Unsupported expression type` 拒绝面被该
            // 能力取代。
            if is_pure_equi_join_on(on_clause) {
                let conditions =
                    self.extract_join_conditions(&current_tables, &right_table, on_clause)?;

                // 构建 Join 节点
                current_plan = PhysicalPlan::Join(crate::executor::JoinNode {
                    left: Box::new(current_plan),
                    right: Box::new(right_plan),
                    conditions,
                    output_columns,
                });
            } else {
                // 非纯等值 ON → NLJ：组合行布局（左表偏移 0..n、右表
                // n..n+m）上编译完整 ON 谓词。布局覆盖 save/restore 严格配对
                //（`inner_table_names` 同型先例）；谓词 `column_index` 为
                // 组合行绝对索引。
                let mut layout: Vec<(String, Vec<String>)> = current_tables
                    .iter()
                    .map(|t| {
                        (
                            t.clone(),
                            self.tables
                                .get(t)
                                .expect("validated table must exist in metadata")
                                .clone(),
                        )
                    })
                    .collect();
                layout.push((
                    right_table.clone(),
                    self.tables
                        .get(&right_table)
                        .expect("validated right_table must exist")
                        .clone(),
                ));
                self.join_column_layout = Some(layout);
                let predicate = match self.build_where(&current_tables[0], on_clause) {
                    Ok(p) => p,
                    Err(e) => {
                        self.join_column_layout = None;
                        return Err(e);
                    }
                };
                self.join_column_layout = None;

                current_plan = PhysicalPlan::NestedLoopJoin(crate::executor::NestedLoopJoinNode {
                    left: Box::new(current_plan),
                    right: Box::new(right_plan),
                    predicate,
                    output_columns,
                });
            }

            current_tables.push(right_table);
        }

        Ok(current_plan)
    }

    /// Build PhysicalPlan for SELECT query
    pub(crate) fn build_query(&mut self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        // MS11-T01 Iter001: 子查询上下文（WHERE IN/EXISTS、标量子查询、派生表）
        // 抑制 SELECT 表达式项路由——子查询计划形状是 SemiJoin/SubqueryEval/
        // DerivedScan 机制的消费面，保持既有行为（R6 零回归）。
        let building_subquery = self.building_subquery;
        // Extract Select body
        let select = extract_select_body(query)?;

        // === MS13 T9: no-FROM SELECT（I035）===
        // 无 FROM 的 SELECT 经虚拟单行输入（SingleRow）产出恰一行：先拒绝面
        // 逐项点名（通配符 / WHERE / GROUP BY / HAVING / ORDER BY / LIMIT /
        // 聚合项），再逐项 `build_expression` 编译并包 `Projection(SingleRow)`。
        // 置于子查询检测之前——no-FORM 形态不进入聚合/SubqueryEval 装配；
        // 列引用经既有 `ColumnNotFound`、子查询项经既有 `UnsupportedExpression`
        // 兜底拒绝。`building_subquery` 上下文同样生效（内层 no-FORM 子查询
        // 经既有 SubqueryEval/SemiJoin 消费面自然可达）。
        if select.from.is_empty() {
            return self.build_no_from_select(select, query);
        }

        // === Scalar subquery detection in SELECT projection ===
        // Scan projection for Expr::Subquery items and build subquery plans
        // Also detect correlated parameters (outer table column references)
        let mut subquery_evals: Vec<(
            usize,
            PhysicalPlan,
            String,
            Vec<crate::executor::CorrelatedParam>,
        )> = Vec::new();
        for (idx, item) in select.projection.iter().enumerate() {
            let (expr, col_name) = match item {
                sqlparser::ast::SelectItem::UnnamedExpr(Expr::Subquery(subquery)) => {
                    (subquery, "__subquery".to_string())
                }
                sqlparser::ast::SelectItem::ExprWithAlias {
                    expr: Expr::Subquery(subquery),
                    alias,
                } => (subquery, alias.value.to_lowercase()),
                _ => continue,
            };
            // Detect correlated parameters before building the plan
            let inner_tables = Self::extract_subquery_table_names(expr);
            self.inner_table_names = Some(inner_tables.clone());
            let correlated_params = self.extract_correlated_params(expr, &inner_tables)?;
            let saved_subquery_ctx = self.building_subquery;
            self.building_subquery = true;
            let subquery_plan = match self.build_query(expr) {
                Ok(p) => p,
                Err(e) => {
                    self.building_subquery = saved_subquery_ctx;
                    self.inner_table_names = None;
                    return Err(e);
                }
            };
            self.building_subquery = saved_subquery_ctx;
            self.inner_table_names = None;
            subquery_evals.push((idx, subquery_plan, col_name, correlated_params));
        }

        // Build a filtered projection that excludes subquery items
        // (SubqueryEval will insert the scalar results at the correct positions later)
        let filtered_projection: Vec<sqlparser::ast::SelectItem> = select
            .projection
            .iter()
            .enumerate()
            .filter(|(idx, _)| {
                !subquery_evals
                    .iter()
                    .any(|(sq_idx, _, _, _)| *sq_idx == *idx)
            })
            .map(|(_, item)| item.clone())
            .collect();

        // Extract columns from filtered projection (for filtering JOIN output)
        let projection_columns = if filtered_projection.is_empty() {
            extract_columns(&select.projection)?
        } else {
            extract_columns(&filtered_projection)?
        };

        // Extract qualified columns from filtered projection for JOIN filtering
        let qualified_columns = if filtered_projection.is_empty() {
            extract_qualified_columns(&select.projection)?
        } else {
            extract_qualified_columns(&filtered_projection)?
        };

        // Build FROM + JOIN chain (with projection columns for filtering)
        let base_plan = self.build_from_clause_with_projection(&select.from, &qualified_columns)?;

        // Extract table name from base plan for single-table queries (for WHERE/ORDER BY processing)
        let table_name = match &base_plan {
            PhysicalPlan::Scan(scan_node) => scan_node.table_name.clone(),
            PhysicalPlan::DerivedScan(derived_node) => derived_node.alias.clone(),
            PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_) => {
                "join_result".to_string() // 虚拟表名用于 JOIN 结果
            }
            _ => "unknown".to_string(),
        };

        // === Aggregate function detection ===
        // Check if SELECT projection contains aggregate functions.
        // Runs before WHERE handling: `has_aggregates` gates the projection
        // resolution below (aggregate plans keep the full-schema input).
        let mut aggregates = Vec::new();
        let mut non_agg_columns = Vec::new();
        let mut agg_output_columns = Vec::new();
        // MS11-T01 Iter001: SELECT 表达式项（非普通列引用的项）。此处只标记
        // 不报错——路由在下文统一裁决：非聚合 → 顶层 Projection；聚合 →
        // 保持聚合路径报错（design D4：检测循环裁决顺序不变）。
        let mut has_expression_items = false;
        // MS13 T8: 每个 SELECT 项的聚合装配角色（None = 子查询/通配等不经
        // 聚合装配的项），供混合投影校验、分组键归属与条件包装消费。
        #[derive(Clone)]
        enum SelectItemRole {
            /// 聚合项（输出名 = result_column_name / 别名）
            Aggregate(String),
            /// 纯列名项（列名，大小写保留）
            Column(String),
            /// 表达式项（输出名 = 别名 / 表达式文本）
            Expression(String),
        }
        let mut item_roles: Vec<Option<SelectItemRole>> = vec![None; select.projection.len()];

        for (item_idx, item) in select.projection.iter().enumerate() {
            // Skip subquery items (handled by SubqueryEval plan node later)
            if subquery_evals.iter().any(|(idx, _, _, _)| *idx == item_idx) {
                continue;
            }
            match item {
                sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                    if is_aggregate_expr(expr) {
                        let func = extract_aggregate_func(expr)?.ok_or_else(|| {
                            PlanError::InvalidAggregateArgument(
                                "Unknown aggregate function".to_string(),
                            )
                        })?;
                        let name = func.result_column_name();
                        agg_output_columns.push(name.clone());
                        item_roles[item_idx] = Some(SelectItemRole::Aggregate(name));
                        aggregates.push(func);
                    } else if !building_subquery && !is_plain_column_expr(expr) {
                        has_expression_items = true;
                        item_roles[item_idx] = Some(SelectItemRole::Expression(expr.to_string()));
                    } else {
                        let col = expr_to_column_name(expr)?;
                        non_agg_columns.push(col.clone());
                        agg_output_columns.push(col.clone());
                        item_roles[item_idx] = Some(SelectItemRole::Column(col));
                    }
                }
                sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => {
                    if is_aggregate_expr(expr) {
                        let func = extract_aggregate_func(expr)?.ok_or_else(|| {
                            PlanError::InvalidAggregateArgument(
                                "Unknown aggregate function".to_string(),
                            )
                        })?;
                        let name = alias.value.clone();
                        agg_output_columns.push(name.clone());
                        item_roles[item_idx] = Some(SelectItemRole::Aggregate(name));
                        aggregates.push(func);
                    } else if !building_subquery && !is_plain_column_expr(expr) {
                        has_expression_items = true;
                        item_roles[item_idx] =
                            Some(SelectItemRole::Expression(alias.value.clone()));
                    } else {
                        let col = expr_to_column_name(expr)?;
                        non_agg_columns.push(col.clone());
                        agg_output_columns.push(alias.value.clone());
                        item_roles[item_idx] = Some(SelectItemRole::Column(col));
                    }
                }
                _ => {} // Wildcard etc. — not relevant for aggregate queries
            }
        }

        let has_aggregates = !aggregates.is_empty();

        // === MS13 T8: SELECT 表达式项路由 ===
        // 混合投影（表达式项 + 聚合）解锁：输出装配由聚合分支的条件包装
        // 承担（每个表达式项 SHALL 解析到某 GROUP BY 键，校验在 GROUP BY
        // 解析后）；范围外混用形态维持既有显式拒绝。纯表达式查询（无聚合）
        // 走既有顶层 Projection 通路，裁决顺序不变。
        let mixed_aggregate = has_expression_items && has_aggregates;
        if mixed_aggregate {
            // 标量子查询项会追加一列（SubqueryEval 移位输出形状），与混合
            // 投影显式拒绝；子查询单独出现维持现状
            if !subquery_evals.is_empty() {
                return Err(PlanError::ParseError(
                    "Expression projection items cannot be mixed with scalar subquery items"
                        .to_string(),
                ));
            }
            // JOIN 输出形状是列过滤而非逐项求值，表达式项 + JOIN 显式拒绝
            //（MS09-T02: 两种 join 节点同语义，拒绝面不因新节点形状漏接）
            if matches!(
                base_plan,
                PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_)
            ) {
                return Err(PlanError::ParseError(
                    "Expression projection items are not supported with JOIN queries".to_string(),
                ));
            }
            // `SELECT *, expr` 通配混用拒绝（通配单独出现维持现状）
            if select
                .projection
                .iter()
                .any(|item| matches!(item, sqlparser::ast::SelectItem::Wildcard(_)))
            {
                return Err(PlanError::ParseError(
                    "SELECT * cannot be mixed with expression projection items".to_string(),
                ));
            }
        }
        let projection_items = if has_expression_items && !mixed_aggregate {
            // 标量子查询项会追加一列（SubqueryEval 移位输出形状），与表达式
            // 项混用显式拒绝；子查询单独出现维持现状
            if !subquery_evals.is_empty() {
                return Err(PlanError::ParseError(
                    "Expression projection items cannot be mixed with scalar subquery items"
                        .to_string(),
                ));
            }
            // JOIN 输出形状是列过滤而非逐项求值，表达式项 + JOIN 显式拒绝
            //（MS09-T02: 两种 join 节点同语义，拒绝面不因新节点形状漏接）
            if matches!(
                base_plan,
                PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_)
            ) {
                return Err(PlanError::ParseError(
                    "Expression projection items are not supported with JOIN queries".to_string(),
                ));
            }
            // `SELECT *, expr` 通配混用拒绝（通配单独出现维持现状）
            if select
                .projection
                .iter()
                .any(|item| matches!(item, sqlparser::ast::SelectItem::Wildcard(_)))
            {
                return Err(PlanError::ParseError(
                    "SELECT * cannot be mixed with expression projection items".to_string(),
                ));
            }
            // 顶层 Projection 项：列引用经 build_expression 解析为全 schema
            // 的 ColumnExpression（与谓词同源索引）；表达式项复用 Iteration
            // 000 值表达式。输入行全形状流出，索引稳定。
            let mut items = Vec::with_capacity(select.projection.len());
            for item in &select.projection {
                match item {
                    sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                        let built = self.build_expression(&table_name, expr)?;
                        items.push(ProjectionItem {
                            expr: built,
                            name: expr.to_string(),
                        });
                    }
                    sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => {
                        let built = self.build_expression(&table_name, expr)?;
                        items.push(ProjectionItem {
                            expr: built,
                            name: alias.value.clone(),
                        });
                    }
                    _ => {
                        return Err(PlanError::ParseError(
                            "Unsupported projection item".to_string(),
                        ))
                    }
                }
            }
            Some(items)
        } else {
            None
        };

        // === Projection resolution (MS10-T01 Iter001) ===
        // Resolve the SELECT list to base-schema column indices (projection
        // order). `None` = identity projection: aggregates present (the
        // aggregate consumes full-schema rows), scalar subqueries in the
        // SELECT list (SubqueryEval owns those shapes), a wildcard, or a name
        // that is not a base column (alias / expression).
        let base_schema = match &base_plan {
            PhysicalPlan::Scan(node) => Some(node.columns.clone()),
            _ => None,
        };
        let sort_due = !query.order_by.is_empty();
        let projection_indices = if has_aggregates || !subquery_evals.is_empty() {
            None
        } else if has_expression_items {
            // MS11-T01 Iter001: 表达式查询禁用 per-node 裁剪——输入全形状
            // 流出，顶层 ProjectionNode 统一求值 + 裁剪（含 ORDER BY 时
            // Sort 的排序键基础列始终可达，design D10 精神）
            None
        } else {
            base_schema
                .as_ref()
                .and_then(|schema| resolve_projection_indices(&projection_columns, schema))
        };
        // With ORDER BY the Sort node owns the trim (design D10): the chain
        // below it must emit full-schema rows so sort keys outside the
        // projection stay reachable. Otherwise the scan (or the Filter
        // wrapper) applies the projection after its predicate evaluates.
        let proj_or_empty = if sort_due {
            Vec::new()
        } else {
            projection_indices.clone().unwrap_or_default()
        };

        // Handle WHERE clause
        let plan_with_where = if let Some(where_expr) = &select.selection {
            // Skip WHERE processing for JOIN queries (will be handled in future tasks)
            // MS09-T02: 两种 join 节点同语义——漏接时 NLJ 会以 table_name
            // "unknown" 走单表 WHERE 路径（错误行为而非既有拒绝）。
            if matches!(
                base_plan,
                PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_)
            ) {
                return Err(PlanError::UnsupportedStatement);
            }

            // Try subquery patterns first (IN subquery / EXISTS)
            if let Some(subquery_plan) = self.try_build_where_subquery(
                where_expr,
                &base_plan,
                &table_name,
                &projection_columns,
            )? {
                subquery_plan
            } else if let Some(key) = self.extract_pk_from_where_gated(&table_name, where_expr)? {
                // Try to extract primary key from WHERE clause for index scan
                // Simple PK equality check - use index scan
                // Note: This is a simplification. A more sophisticated optimizer would
                // check if the WHERE clause is ONLY pk = value, not part of a complex expression
                if self.is_simple_pk_equality(&table_name, where_expr)? {
                    PhysicalPlan::IndexScan(IndexScanNode {
                        table_name: table_name.clone(),
                        key,
                        columns: if !sort_due && projection_indices.is_some() {
                            projection_columns.clone()
                        } else {
                            base_schema.clone().unwrap_or_default()
                        },
                        projection: proj_or_empty.clone(),
                    })
                } else {
                    // Complex WHERE with PK - use Filter over Scan
                    let predicate = self.build_where(&table_name, where_expr)?;
                    PhysicalPlan::Filter(FilterNode {
                        input: Box::new(base_plan),
                        predicate,
                        table_name: table_name.clone(),
                        projection: proj_or_empty.clone(),
                    })
                }
            } else {
                // Non-PK WHERE — M19 routing + MS07-T06 pushdown:
                // - PK equality in a non-simple form (e.g. AND-combined):
                //   Filter over the original Scan, unchanged.
                // - OR anywhere in the predicate: not pushdown-eligible;
                //   keep the FilterExecutor wrapper (semantics baseline).
                // - Otherwise: the predicate moves into the DataScan node
                //   (row-level filtering) and no Filter node is generated.
                let predicate = self.build_where(&table_name, where_expr)?;
                let has_pk_eq = self.has_pk_equality(&table_name, where_expr)?;
                // MS15-T01 (I036): a PK-equality leg with a non-keyable literal
                // (String/Float/Bool/NULL) must not route to the index — keyless
                // rows (stored, not indexed) are invisible to Scan's index
                // traversal and would be silently dropped. Fall through to the
                // OR / pushdown arms for data-page evaluation instead.
                // MS16 Iteration 000 (I046, design D1 门 2)：键列已知声明非
                // Int 时同样不得进入 Filter(Scan) 索引遍历（该表全行无键），
                // 键位等值全形态分流到数据页臂。
                if has_pk_eq
                    && !self.pk_type_known_non_int(&table_name)
                    && !self.has_non_keyable_pk_literal_leg(&table_name, where_expr)?
                {
                    // PK equality present but in a non-simple form (e.g. AND-combined
                    // with another predicate). Keep base_plan as-is.
                    PhysicalPlan::Filter(FilterNode {
                        input: Box::new(base_plan),
                        predicate,
                        table_name: table_name.clone(),
                        projection: proj_or_empty.clone(),
                    })
                } else if contains_or(where_expr) {
                    let input = match base_plan {
                        PhysicalPlan::Scan(scan_node) => PhysicalPlan::DataScan(DataScanNode {
                            table_name: scan_node.table_name,
                            columns: scan_node.columns,
                            predicate: None,
                            scan_cap: None,
                            // The Filter wrapper above owns the projection trim
                            // (or the Sort node when ORDER BY is present).
                            projection: Vec::new(),
                        }),
                        other => other,
                    };
                    PhysicalPlan::Filter(FilterNode {
                        input: Box::new(input),
                        predicate,
                        table_name: table_name.clone(),
                        projection: proj_or_empty.clone(),
                    })
                } else {
                    // Pushdown-eligible: swap the Scan for a DataScan carrying
                    // the predicate. DerivedScan and other sources keep the
                    // Filter wrapper (they are not the single-table scan the
                    // predicate was built against).
                    match base_plan {
                        PhysicalPlan::Scan(scan_node) => PhysicalPlan::DataScan(DataScanNode {
                            table_name: scan_node.table_name,
                            columns: scan_node.columns,
                            predicate: Some(predicate),
                            scan_cap: None,
                            projection: proj_or_empty.clone(),
                        }),
                        other => PhysicalPlan::Filter(FilterNode {
                            input: Box::new(other),
                            predicate,
                            table_name: table_name.clone(),
                            projection: proj_or_empty.clone(),
                        }),
                    }
                }
            }
        } else {
            // No WHERE clause — M19: route to DataScan (skip index layer).
            // Subqueries / derived scans keep their original plan.
            match base_plan {
                PhysicalPlan::Scan(scan_node) => PhysicalPlan::DataScan(DataScanNode {
                    table_name: scan_node.table_name,
                    columns: scan_node.columns,
                    predicate: None,
                    scan_cap: None,
                    projection: proj_or_empty.clone(),
                }),
                _ => base_plan,
            }
        };

        // Build aggregate plan if needed
        let plan_with_aggregate = if has_aggregates {
            // Build column index mapping from input plan.
            // MS10-T01 Iter001: unified through get_plan_output_columns, which
            // describes the input plan's real output shape on every form —
            // IndexScan/IndexScanAll inputs previously fell into the empty
            // fallback and silently NULL-ed aggregates (and mis-mapped GROUP
            // BY keys).
            // MS13 T8: 前移——分组键解析与编译消费 column_indices。
            let input_schema = self.get_plan_output_columns(&plan_with_where);
            let column_indices: HashMap<String, usize> = input_schema
                .iter()
                .enumerate()
                .map(|(i, col)| (col.to_lowercase(), i))
                .collect();

            // MS13 T8: GROUP BY 项解析（design D12 顺序：列名（既有）→
            // SELECT 别名 → SELECT 项表达式文本 → 1-based 位置；全部不匹配
            // → NonAggregatedColumn 点名）。
            let group_keys: Vec<GroupKey> = match &select.group_by {
                sqlparser::ast::GroupByExpr::Expressions(exprs) => exprs
                    .iter()
                    .map(|e| self.resolve_group_by_item(e, &select.projection, &column_indices))
                    .collect::<Result<Vec<_>, _>>()?,
                sqlparser::ast::GroupByExpr::All => {
                    // GROUP BY ALL: all non-aggregate columns（列名键，既有语义）
                    non_agg_columns
                        .iter()
                        .map(|col| GroupKey {
                            name: col.clone(),
                            column: Some(col.clone()),
                            expr_item: None,
                        })
                        .collect()
                }
            };
            let group_by: Vec<String> = group_keys.iter().map(|k| k.name.clone()).collect();

            // Strict mode: non-aggregate columns must appear in GROUP BY
            //（名字包含检查与既有语义一致——大小写敏感，列名键名 = 既有
            // expr_to_column_name 产物）
            for col in &non_agg_columns {
                if !group_by.contains(col) {
                    return Err(PlanError::NonAggregatedColumn(col.clone()));
                }
            }

            // MS13 T8 混合投影校验：每个表达式 SELECT 项必须被某分组键匹配
            //（别名/表达式文本/位置），否则 NonAggregatedColumn 点名。
            for (i, role) in item_roles.iter().enumerate() {
                if let Some(SelectItemRole::Expression(name)) = role {
                    if !group_keys.iter().any(|k| k.expr_item == Some(i)) {
                        return Err(PlanError::NonAggregatedColumn(name.clone()));
                    }
                }
            }

            // MS13 T8: 分组键编译为求值表达式。列名键 = ColumnExpression
            //（column_indices 已知索引，与既有 extract_group_key 名字查索引
            // 语义逐字节一致）；表达式键 = 匹配 SELECT 项的编译表达式。
            let group_key_exprs: Vec<ExpressionRef> = group_keys
                .iter()
                .map(|k| match &k.column {
                    Some(col) => {
                        let idx = *column_indices
                            .get(&col.to_lowercase())
                            .ok_or_else(|| PlanError::NonAggregatedColumn(col.clone()))?;
                        Ok(Arc::new(ColumnExpression {
                            column_name: col.to_lowercase(),
                            column_index: idx,
                        }) as ExpressionRef)
                    }
                    None => {
                        let item = &select.projection[k.expr_item.expect("validated at resolve")];
                        let e = match item {
                            sqlparser::ast::SelectItem::UnnamedExpr(e) => e,
                            sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => expr,
                            _ => unreachable!("aggregate/wildcard keys rejected at resolve"),
                        };
                        self.build_expression(&table_name, e)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;

            // MS13 T8 输出装配（design D12）：直出路径零变化判定——无表达式
            // 项、全部键为纯列名、且 SELECT 序为「键项在前、聚合项在后」；
            // 否则（表达式键或交错序）在 Aggregate/Having 之上包一层
            // ProjectionNode（键项/聚合项按聚合行位置重排，列名 = SELECT 名）。
            let keys_all_column = group_keys.iter().all(|k| k.column.is_some());
            let mut seen_aggregate = false;
            let mut select_keys_first = true;
            for role in item_roles.iter().flatten() {
                match role {
                    SelectItemRole::Column(_) => {
                        if seen_aggregate {
                            select_keys_first = false;
                        }
                    }
                    SelectItemRole::Aggregate(_) => seen_aggregate = true,
                    SelectItemRole::Expression(_) => select_keys_first = false,
                }
            }
            let direct = !has_expression_items && keys_all_column && select_keys_first;

            let key_count = group_keys.len();
            let agg_names: Vec<String> = item_roles
                .iter()
                .flatten()
                .filter_map(|r| match r {
                    SelectItemRole::Aggregate(name) => Some(name.clone()),
                    _ => None,
                })
                .collect();
            // 输出列名：直出 = 既有 agg_output_columns（SELECT 序，逐字节
            // 保持）；包装 = 行序（GROUP BY 键序 ++ 聚合序，供 HAVING 绑定
            // 与聚合行形状一致）。
            let node_output_columns = if direct {
                agg_output_columns
            } else {
                group_keys
                    .iter()
                    .map(|k| k.name.clone())
                    .chain(agg_names)
                    .collect()
            };

            // 条件包装项（仅非直出形态）：SELECT 项 → 聚合行位置的
            // ColumnExpression 重排，列名 = SELECT 名。
            let wrap_projection = if direct {
                None
            } else {
                let mut items = Vec::with_capacity(select.projection.len());
                let mut agg_seen = 0usize;
                for (i, role) in item_roles.iter().enumerate() {
                    match role {
                        None => {}
                        Some(SelectItemRole::Column(col)) => {
                            let gpos = group_keys
                                .iter()
                                .position(|k| {
                                    k.column
                                        .as_deref()
                                        .is_some_and(|c| c.eq_ignore_ascii_case(col))
                                })
                                .ok_or_else(|| PlanError::NonAggregatedColumn(col.clone()))?;
                            items.push(ProjectionItem {
                                expr: Arc::new(ColumnExpression {
                                    column_name: col.to_lowercase(),
                                    column_index: gpos,
                                }),
                                name: col.clone(),
                            });
                        }
                        Some(SelectItemRole::Expression(name)) => {
                            let gpos = group_keys
                                .iter()
                                .position(|k| k.expr_item == Some(i))
                                .expect("mixed projection validated above");
                            items.push(ProjectionItem {
                                expr: Arc::new(ColumnExpression {
                                    column_name: name.to_lowercase(),
                                    column_index: gpos,
                                }),
                                name: name.clone(),
                            });
                        }
                        Some(SelectItemRole::Aggregate(name)) => {
                            items.push(ProjectionItem {
                                expr: Arc::new(ColumnExpression {
                                    column_name: name.to_lowercase(),
                                    column_index: key_count + agg_seen,
                                }),
                                name: name.clone(),
                            });
                            agg_seen += 1;
                        }
                    }
                }
                let columns = items.iter().map(|it| it.name.clone()).collect();
                Some((items, columns))
            };

            // Build HAVING predicate BEFORE consuming node_output_columns
            let having_pred = if let Some(having_expr) = &select.having {
                Some(self.build_having(having_expr, &node_output_columns)?)
            } else {
                None
            };

            let agg_plan = PhysicalPlan::Aggregate(crate::executor::AggregateNode {
                input: Box::new(plan_with_where),
                group_by,
                group_key_exprs,
                aggregates,
                output_columns: node_output_columns,
                table_name: table_name.clone(),
                column_indices,
            });

            // Wrap with HAVING if predicate was built
            let with_having = if let Some(having_pred) = having_pred {
                PhysicalPlan::Having(crate::executor::HavingNode {
                    input: Box::new(agg_plan),
                    predicate: having_pred,
                    table_name: table_name.clone(),
                })
            } else {
                agg_plan
            };

            match wrap_projection {
                Some((items, columns)) => PhysicalPlan::Projection(ProjectionNode {
                    input: Box::new(with_having),
                    items,
                    columns,
                }),
                None => with_having,
            }
        } else {
            plan_with_where
        };

        // Parse ORDER BY
        let plan_with_order = if !query.order_by.is_empty() {
            let order_by: Vec<OrderByColumn> = query
                .order_by
                .iter()
                .map(|o| {
                    let column = extract_column_name(&o.expr)?;
                    // sqlparser: asc field is Option<bool>
                    // None or Some(true) = ASC, Some(false) = DESC
                    let asc = o.asc.unwrap_or(true);
                    Ok(OrderByColumn { column, asc })
                })
                .collect::<Result<Vec<_>, PlanError>>()?;

            // MS10-T01 Iter001: sort-key lookup uses the input plan's real
            // output shape. With no aggregate in play the Sort node also owns
            // the projection trim (the scan chain below emits full-schema
            // rows, so keys outside the projection stay reachable — design
            // D10). Aggregate inputs keep their own output shape and are not
            // re-projected.
            let sort_columns = if has_aggregates {
                projection_columns.clone()
            } else {
                self.get_plan_output_columns(&plan_with_aggregate)
            };
            let sort_projection = if has_aggregates || !is_base_scan_chain(&plan_with_aggregate) {
                Vec::new()
            } else {
                projection_indices.clone().unwrap_or_default()
            };

            PhysicalPlan::Sort(SortNode {
                input: Box::new(plan_with_aggregate),
                order_by,
                table_name: table_name.clone(),
                columns: sort_columns,
                projection: sort_projection,
            })
        } else {
            plan_with_aggregate
        };

        // Parse LIMIT/OFFSET
        let plan_with_limit = if let Some(limit_expr) = &query.limit {
            let limit = parse_limit_value(limit_expr)?;
            let offset = query
                .offset
                .as_ref()
                .map(|o| parse_offset_value(&o.value))
                .transpose()?
                .unwrap_or(0);

            // MS07-T06: push the row cap into a directly-wrapped DataScan so
            // the scan can stop early. The eligible chain is exactly
            // `DataScan`: pushable Filter(DataScan) shapes were already merged
            // into DataScan by the WHERE pushdown above, and every remaining
            // wrapper (Filter with a non-pushable predicate, Sort, Aggregate,
            // DerivedScan, …) is not row-transparent, so capping below it
            // would truncate its input. The top-level Limit node is always
            // kept (safe cap + offset skipping for non-pushed shapes).
            let input = match plan_with_order {
                PhysicalPlan::DataScan(mut node) => {
                    node.scan_cap = Some(if limit == 0 {
                        0
                    } else {
                        offset.saturating_add(limit)
                    });
                    PhysicalPlan::DataScan(node)
                }
                other => other,
            };

            PhysicalPlan::Limit(crate::executor::LimitNode {
                input: Box::new(input),
                limit,
                offset,
            })
        } else {
            plan_with_order
        };

        // === Wrap with SubqueryEval nodes for scalar subqueries in SELECT ===
        // Process from right to left so that result_column_index calculations remain stable
        // result_column_index = projection_index - (number of subqueries at indices < projection_index)
        let mut plan = plan_with_limit;
        for (proj_idx, subquery_plan, col_name, correlated_params) in subquery_evals.iter().rev() {
            let subqueries_before = subquery_evals
                .iter()
                .filter(|(idx, _, _, _)| idx < proj_idx)
                .count();
            let result_column_index = proj_idx - subqueries_before;
            plan = PhysicalPlan::SubqueryEval(crate::executor::SubqueryEvalNode {
                input: Box::new(plan),
                subquery: Box::new(subquery_plan.clone()),
                output_column: col_name.clone(),
                result_column_index,
                correlated_params: correlated_params.clone(),
            });
        }

        // === MS11-T01 Iter001: 顶层 Projection 包装（LIMIT 与 SubqueryEval
        // 之上）——SELECT 列表表达式项逐行求值并按项输出 ===
        if let Some(items) = projection_items {
            let columns = items.iter().map(|i| i.name.clone()).collect();
            plan = PhysicalPlan::Projection(ProjectionNode {
                input: Box::new(plan),
                items,
                columns,
            });
        }

        Ok(plan)
    }

    /// MS13 T9（design D13）：no-FROM SELECT——拒绝面先行，逐项编译表达式，
    /// 包 `Projection(SingleRow)`（恰产出一行）。表头 = 别名 / 表达式文本
    /// （既有表达式项语义）；行数恰 1。
    fn build_no_from_select(
        &mut self,
        select: &sqlparser::ast::Select,
        query: &Query,
    ) -> Result<PhysicalPlan, PlanError> {
        // 拒绝面：通配符与聚合项（逐项点名）
        for item in &select.projection {
            match item {
                sqlparser::ast::SelectItem::Wildcard(_) => {
                    return Err(PlanError::ParseError(
                        "SELECT * is not supported without FROM".to_string(),
                    ));
                }
                sqlparser::ast::SelectItem::QualifiedWildcard(_, _) => {
                    return Err(PlanError::ParseError(
                        "Qualified wildcard is not supported without FROM".to_string(),
                    ));
                }
                sqlparser::ast::SelectItem::UnnamedExpr(expr)
                | sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => {
                    if is_aggregate_expr(expr) {
                        return Err(PlanError::ParseError(format!(
                            "Aggregate function {} is not supported without FROM",
                            expr
                        )));
                    }
                }
            }
        }
        // 拒绝面：各子句逐一点名（LIMIT 与 OFFSET 各自报）
        if select.selection.is_some() {
            return Err(PlanError::ParseError(
                "WHERE clause is not supported without FROM".to_string(),
            ));
        }
        match &select.group_by {
            sqlparser::ast::GroupByExpr::Expressions(exprs) if exprs.is_empty() => {}
            _ => {
                return Err(PlanError::ParseError(
                    "GROUP BY clause is not supported without FROM".to_string(),
                ))
            }
        }
        if select.having.is_some() {
            return Err(PlanError::ParseError(
                "HAVING clause is not supported without FROM".to_string(),
            ));
        }
        if !query.order_by.is_empty() {
            return Err(PlanError::ParseError(
                "ORDER BY clause is not supported without FROM".to_string(),
            ));
        }
        if query.limit.is_some() {
            return Err(PlanError::ParseError(
                "LIMIT clause is not supported without FROM".to_string(),
            ));
        }
        if query.offset.is_some() {
            return Err(PlanError::ParseError(
                "OFFSET clause is not supported without FROM".to_string(),
            ));
        }

        // 可达面：逐项编译（无表注册）。空布局覆盖（MS09-T02 NLJ layout
        // 机制的空表形态）使列引用走既有布局搜索臂——空布局无命中 → 既有
        // `ColumnNotFound`（spec R2/S3 列不存在类错误，零新文案）；限定名
        // → 既有 `TableNotFound`。save/restore 与 NLJ 配对同型。
        let saved_layout = self.join_column_layout.take();
        self.join_column_layout = Some(Vec::new());
        let compiled = (|| {
            let mut items = Vec::with_capacity(select.projection.len());
            for item in &select.projection {
                match item {
                    sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                        let built = self.build_expression("", expr)?;
                        items.push(ProjectionItem {
                            expr: built,
                            name: expr.to_string(),
                        });
                    }
                    sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => {
                        let built = self.build_expression("", expr)?;
                        items.push(ProjectionItem {
                            expr: built,
                            name: alias.value.clone(),
                        });
                    }
                    _ => unreachable!("wildcards rejected above"),
                }
            }
            Ok(items)
        })();
        self.join_column_layout = saved_layout;
        let items = compiled?;

        let columns = items.iter().map(|i| i.name.clone()).collect();
        Ok(PhysicalPlan::Projection(ProjectionNode {
            input: Box::new(PhysicalPlan::SingleRow),
            items,
            columns,
        }))
    }

    /// Check if WHERE clause is a simple PK equality (pk = value)
    pub(crate) fn is_simple_pk_equality(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<bool, PlanError> {
        let pk_column = match self.primary_keys.get(table_name) {
            Some(pk) => pk.clone(),
            None => return Ok(false),
        };

        match expr {
            Expr::BinaryOp {
                left,
                op: sqlparser::ast::BinaryOperator::Eq,
                right,
            } => {
                // Check: column = value
                if let Expr::Identifier(ident) = left.as_ref() {
                    if ident.value.to_lowercase() == pk_column {
                        return Ok(matches!(right.as_ref(), Expr::Value(_)));
                    }
                }
                // Check: value = column
                if let Expr::Identifier(ident) = right.as_ref() {
                    if ident.value.to_lowercase() == pk_column {
                        return Ok(matches!(left.as_ref(), Expr::Value(_)));
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// M19: Check if WHERE clause contains a PK equality somewhere in the tree
    /// (recursively through AND combinations). Used to decide whether the
    /// query should still go to `IndexScan` (M19 does not change that path)
    /// or can be served by `Filter(DataScan)`.
    ///
    /// Conservative: OR-branches return `false` (we don't optimize OR→IndexScan
    /// here — that is M21 / Phase 5 work).
    pub(crate) fn has_pk_equality(&self, table_name: &str, expr: &Expr) -> Result<bool, PlanError> {
        let pk_column = match self.primary_keys.get(table_name) {
            Some(pk) => pk.clone(),
            None => return Ok(false),
        };

        match expr {
            Expr::BinaryOp {
                left,
                op: sqlparser::ast::BinaryOperator::Eq,
                right,
            } => {
                let left_is_pk = matches!(left.as_ref(), Expr::Identifier(i) if i.value.to_lowercase() == pk_column);
                let right_is_pk = matches!(right.as_ref(), Expr::Identifier(i) if i.value.to_lowercase() == pk_column);
                Ok(left_is_pk || right_is_pk)
            }
            Expr::BinaryOp {
                left,
                op: sqlparser::ast::BinaryOperator::And,
                right,
            } => Ok(self.has_pk_equality(table_name, left)?
                || self.has_pk_equality(table_name, right)?),
            _ => Ok(false),
        }
    }

    /// MS15-T01 (I036): Check whether any PK-equality leg compares the key
    /// column against a non-keyable literal (String/Float/Bool/NULL —
    /// `Value::to_key()` is `None`). Those legs make index routing unsound:
    /// keyless rows never enter the index, so `Filter(Scan)` traversal
    /// silently drops them and the predicate must be evaluated on the data
    /// page instead.
    ///
    /// Traversal mirrors `has_pk_equality` (Eq legs + AND recursion; OR is
    /// conservative `false`). A leg whose non-key side is not an
    /// `Expr::Value` (column-column, unary negation) is not a literal leg.
    /// Conversion failures propagate like `extract_pk_from_where`.
    fn has_non_keyable_pk_literal_leg(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<bool, PlanError> {
        let pk_column = match self.primary_keys.get(table_name) {
            Some(pk) => pk.clone(),
            None => return Ok(false),
        };

        match expr {
            Expr::BinaryOp {
                left,
                op: sqlparser::ast::BinaryOperator::Eq,
                right,
            } => {
                let literal = if matches!(
                    left.as_ref(),
                    Expr::Identifier(i) if i.value.to_lowercase() == pk_column
                ) {
                    right.as_ref()
                } else if matches!(
                    right.as_ref(),
                    Expr::Identifier(i) if i.value.to_lowercase() == pk_column
                ) {
                    left.as_ref()
                } else {
                    return Ok(false);
                };
                if let Expr::Value(v) = literal {
                    return Ok(value_from_sqlparser(v)?.to_key().is_none());
                }
                Ok(false)
            }
            Expr::BinaryOp {
                left,
                op: sqlparser::ast::BinaryOperator::And,
                right,
            } => Ok(self.has_non_keyable_pk_literal_leg(table_name, left)?
                || self.has_non_keyable_pk_literal_leg(table_name, right)?),
            _ => Ok(false),
        }
    }

    /// MS16 Iteration 000 (I046): 键列声明类型是否已知且非 Int
    /// （Float/String/Bool）。此时全部存储行必为无键行（`Value::to_key()`
    /// 仅 Int 有值），键位等值任何形态都不可路由索引遍历——统一分流到
    /// 数据页臂（design D1）。类型未注册（派生表别名）视同未知，不分流、
    /// 回退既有路由。
    fn pk_type_known_non_int(&self, table_name: &str) -> bool {
        match self.primary_key_types.get(table_name) {
            Some(ct) => !matches!(ct, crate::storage::page_format::ColumnType::Int),
            None => false,
        }
    }

    /// MS16 Iteration 000 (I046): `extract_pk_from_where` 判定门（design D1
    /// 门 1）——键列已知非 Int 时跳过索引键提取（视同 None），键位等值
    /// 流入下方非 PK 臂；类型未知或 Int 时保持原提取行为。
    fn extract_pk_from_where_gated(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<Option<crate::storage::page_format::Key>, PlanError> {
        if self.pk_type_known_non_int(table_name) {
            return Ok(None);
        }
        self.extract_pk_from_where(table_name, expr)
    }

    /// Extract primary key from WHERE clause
    ///
    /// Only supports: pk_column = value
    pub(crate) fn extract_pk_from_where(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<Option<crate::storage::page_format::Key>, PlanError> {
        // Get primary key column name
        let pk_column = match self.primary_keys.get(table_name) {
            Some(pk) => pk.clone(),
            None => return Ok(None),
        };

        // Check for binary operation: column = value or value = column
        if let Expr::BinaryOp {
            left,
            op: sqlparser::ast::BinaryOperator::Eq,
            right,
        } = expr
        {
            // Case 1: column = value
            if let Expr::Identifier(ident) = left.as_ref() {
                if ident.value.to_lowercase() == pk_column {
                    if let Expr::Value(v) = right.as_ref() {
                        let value = value_from_sqlparser(v)?;
                        return Ok(value.to_key());
                    }
                }
            }

            // Case 2: value = column
            if let Expr::Identifier(ident) = right.as_ref() {
                if ident.value.to_lowercase() == pk_column {
                    if let Expr::Value(v) = left.as_ref() {
                        let value = value_from_sqlparser(v)?;
                        return Ok(value.to_key());
                    }
                }
            }
        }

        // Unsupported WHERE clause
        Ok(None)
    }

    /// Build output_columns for a single-table query
    pub(crate) fn build_output_columns_for_table(
        &self,
        table_name: &str,
        projection_columns: &[String],
    ) -> Vec<OutputColumn> {
        let columns = self.tables.get(table_name).cloned().unwrap_or_default();
        projection_columns
            .iter()
            .map(|col| {
                let column_index = columns
                    .iter()
                    .position(|c| c.to_lowercase() == col.to_lowercase())
                    .unwrap_or(0);
                OutputColumn {
                    table: Some(table_name.to_string()),
                    column: col.clone(),
                    table_alias: table_name.to_string(),
                    column_index,
                }
            })
            .collect()
    }
}

/// Whether the plan is the single-table scan chain (optionally Filter-wrapped)
/// whose nodes emit full-schema rows when their projections are empty.
/// Gates Sort-owned projection: the projection indices are resolved against
/// the base schema and are only valid for that chain.
fn is_base_scan_chain(plan: &PhysicalPlan) -> bool {
    match plan {
        PhysicalPlan::Scan(_)
        | PhysicalPlan::DataScan(_)
        | PhysicalPlan::IndexScan(_)
        | PhysicalPlan::IndexScanAll(_) => true,
        PhysicalPlan::Filter(node) => is_base_scan_chain(&node.input),
        _ => false,
    }
}

/// MS11-T01 Iter001: SELECT 列表表达式是否为普通列引用（裸标识符或两段限定
/// 标识符）。裸 `NULL` 是字面量（`build_expression` 映射为常量），不是列；
/// 两段以外的 CompoundIdentifier 在 extract_columns 处已被拒，到不了这里。
fn is_plain_column_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(ident) => !ident.value.eq_ignore_ascii_case("NULL"),
        Expr::CompoundIdentifier(parts) => parts.len() == 2,
        _ => false,
    }
}

/// Resolve select-list column names to base-schema indices (projection order).
///
/// Returns `None` for the identity projection: an empty list, a wildcard
/// item, or any name that is not a base column (alias, expression, aggregate
/// result). Identity keeps the pre-projection row shape byte-for-byte.
fn resolve_projection_indices(projection: &[String], schema: &[String]) -> Option<Vec<usize>> {
    if projection.is_empty() {
        return None;
    }
    let mut indices = Vec::with_capacity(projection.len());
    for col in projection {
        if col == "*" {
            return None;
        }
        match schema
            .iter()
            .position(|c| c.to_lowercase() == col.to_lowercase())
        {
            Some(idx) => indices.push(idx),
            None => return None,
        }
    }
    Some(indices)
}

/// Check whether a WHERE expression contains a logical `OR` at any depth.
///
/// MS07-T06 pushdown eligibility: only the planner-buildable surface matters
/// (`build_where` accepts BinaryOp / Nested; comparisons host no OR). Other
/// variants either cannot carry an OR into `build_where` or fail planning
/// before pushdown is decided.
fn contains_or(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp {
            op: sqlparser::ast::BinaryOperator::Or,
            ..
        } => true,
        Expr::BinaryOp { left, right, .. } => contains_or(left) || contains_or(right),
        Expr::UnaryOp { expr, .. } => contains_or(expr),
        Expr::Nested(expr) => contains_or(expr),
        // MS11-T01: the new predicate forms only carry an OR when a
        // sub-expression does — except InList, whose desugaring IS an OR
        // chain, so it is conservatively true (IN keeps the Filter path).
        Expr::InList { .. } => true,
        Expr::Between {
            expr, low, high, ..
        } => contains_or(expr) || contains_or(low) || contains_or(high),
        Expr::Like { expr, pattern, .. } => contains_or(expr) || contains_or(pattern),
        Expr::IsNull(expr) | Expr::IsNotNull(expr) => contains_or(expr),
        Expr::Cast { expr, .. } => contains_or(expr),
        Expr::Case {
            operand,
            conditions,
            results,
            else_result,
        } => {
            operand.as_ref().map(|e| contains_or(e)).unwrap_or(false)
                || conditions.iter().any(contains_or)
                || results.iter().any(contains_or)
                || else_result
                    .as_ref()
                    .map(|e| contains_or(e))
                    .unwrap_or(false)
        }
        Expr::Function(func) => func.args.iter().any(|arg| match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => contains_or(e),
            FunctionArg::Named {
                arg: FunctionArgExpr::Expr(e),
                ..
            } => contains_or(e),
            _ => false,
        }),
        _ => false,
    }
}

/// Extract column name from ORDER BY expression
fn extract_column_name(expr: &Expr) -> Result<String, PlanError> {
    match expr {
        Expr::Identifier(ident) => Ok(ident.value.clone()),
        _ => Err(PlanError::ParseError(
            "ORDER BY only supports column names".to_string(),
        )),
    }
}

/// Parse LIMIT value from expression
fn parse_limit_value(expr: &Expr) -> Result<usize, PlanError> {
    match expr {
        Expr::Value(sqlparser::ast::Value::Number(n, _)) => n
            .parse::<usize>()
            .map_err(|_| PlanError::ParseError("Invalid LIMIT value".to_string())),
        _ => Err(PlanError::ParseError("LIMIT must be a number".to_string())),
    }
}

/// Parse OFFSET value from expression
fn parse_offset_value(expr: &Expr) -> Result<usize, PlanError> {
    match expr {
        Expr::Value(sqlparser::ast::Value::Number(n, _)) => n
            .parse::<usize>()
            .map_err(|_| PlanError::ParseError("Invalid OFFSET value".to_string())),
        _ => Err(PlanError::ParseError("OFFSET must be a number".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{PhysicalPlan, Value};
    use crate::parser::ast::parse_sql;

    #[test]
    fn test_build_query_scan() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "SELECT id, name FROM users";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        // M19: no-WHERE SELECT now routes to DataScan (skip index layer).
        match plan {
            PhysicalPlan::DataScan(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.columns, vec!["id", "name"]);
            }
            _ => panic!("Expected DataScan plan (M19 default for no-WHERE)"),
        }
    }

    #[test]
    fn test_build_query_index_scan() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "SELECT id, name FROM users WHERE id = 42";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::IndexScan(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.columns, vec!["id", "name"]);
                // key should be 42 as big-endian bytes
                let expected_key = Value::Int(42).to_key().unwrap();
                assert_eq!(node.key, expected_key);
            }
            _ => panic!("Expected IndexScan plan"),
        }
    }

    #[test]
    fn test_extract_pk_from_where_reversed() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into()], "id");

        // Test: value = column (reversed order)
        let sql = "SELECT * FROM users WHERE 42 = id";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::IndexScan(node) => {
                let expected_key = Value::Int(42).to_key().unwrap();
                assert_eq!(node.key, expected_key);
            }
            _ => panic!("Expected IndexScan plan"),
        }
    }

    #[test]
    fn test_nonexistent_table() {
        let mut builder = PlanBuilder::new();

        let sql = "SELECT * FROM nonexistent";
        let stmts = parse_sql(sql).unwrap();
        let result = builder.build_plan(&stmts[0]);

        assert!(result.is_err());
    }

    #[test]
    fn test_unsupported_where() {
        // MS07-T06: non-PK WHERE without OR is pushed into DataScan
        // (row-level predicate, no Filter node).
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "SELECT * FROM users WHERE name = 'Alice'";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::DataScan(node) => {
                assert!(
                    node.predicate.is_some(),
                    "non-PK WHERE must carry its predicate inside DataScan"
                );
            }
            _ => panic!("Expected DataScan with pushed predicate, got {:?}", plan),
        }
    }

    #[test]
    fn test_get_plan_output_columns_join() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");
        builder.register_table("orders", vec!["user_id".into(), "total".into()], "");

        let sql =
            "SELECT users.id, orders.total FROM users JOIN orders ON users.id = orders.user_id";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        assert!(
            matches!(plan, PhysicalPlan::Join(_)),
            "expected Join plan, got {:?}",
            plan
        );
        let columns = builder.get_plan_output_columns(&plan);
        assert_eq!(columns, vec!["id", "total"]);
    }
}
