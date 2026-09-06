//! PlanBuilder — SELECT / FROM / JOIN / projection / PK-equality
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::aggregate::{extract_aggregate_func, is_aggregate_expr};
use super::expression::expr_to_column_name;
use super::PlanBuilder;
use crate::executor::{
    DataScanNode, FilterNode, IndexScanNode, OrderByColumn, OutputColumn, PhysicalPlan, ScanNode,
    SortNode,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::{Expr, Query, TableFactor};
use std::collections::HashMap;

impl PlanBuilder {
    /// 从 PhysicalPlan 中提取输出列名（用于派生表的列注册）
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn get_plan_output_columns(&self, plan: &PhysicalPlan) -> Vec<String> {
        match plan {
            PhysicalPlan::Scan(node) => node.columns.clone(),
            PhysicalPlan::DataScan(node) => node.columns.clone(),
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
            PhysicalPlan::IndexScan(node) => node.columns.clone(),
            PhysicalPlan::IndexScanAll(node) => node.columns.clone(),
            PhysicalPlan::Join(node) => {
                // JOIN 行组装严格按 output_columns 顺序（见 executor/join.rs），
                // 列名直接取自节点，不递归合并左右子计划。
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
            PhysicalPlan::SubqueryEval(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::Insert(_) | PhysicalPlan::Update(_) | PhysicalPlan::Delete(_) => {
                Vec::new()
            }
            PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => Vec::new(),
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
                let table_name = name.to_string().to_lowercase();
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
                let subquery_plan = self.build_query(subquery)?;
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
            let conditions =
                self.extract_join_conditions(&current_tables, &right_table, on_clause)?;

            // 构建输出列（根据 qualified_columns 过滤）
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

            // 构建 Join 节点
            current_plan = PhysicalPlan::Join(crate::executor::JoinNode {
                left: Box::new(current_plan),
                right: Box::new(right_plan),
                conditions,
                output_columns,
            });

            current_tables.push(right_table);
        }

        Ok(current_plan)
    }

    /// Build PhysicalPlan for SELECT query
    pub(crate) fn build_query(&mut self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        // Extract Select body
        let select = extract_select_body(query)?;

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
            let subquery_plan = match self.build_query(expr) {
                Ok(p) => p,
                Err(e) => {
                    self.inner_table_names = None;
                    return Err(e);
                }
            };
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
            PhysicalPlan::Join(_) => "join_result".to_string(), // 虚拟表名用于 JOIN 结果
            _ => "unknown".to_string(),
        };

        // === Aggregate function detection ===
        // Check if SELECT projection contains aggregate functions.
        // Runs before WHERE handling: `has_aggregates` gates the projection
        // resolution below (aggregate plans keep the full-schema input).
        let mut aggregates = Vec::new();
        let mut non_agg_columns = Vec::new();
        let mut agg_output_columns = Vec::new();

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
                        agg_output_columns.push(func.result_column_name());
                        aggregates.push(func);
                    } else {
                        let col = expr_to_column_name(expr)?;
                        non_agg_columns.push(col.clone());
                        agg_output_columns.push(col);
                    }
                }
                sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => {
                    if is_aggregate_expr(expr) {
                        let func = extract_aggregate_func(expr)?.ok_or_else(|| {
                            PlanError::InvalidAggregateArgument(
                                "Unknown aggregate function".to_string(),
                            )
                        })?;
                        agg_output_columns.push(alias.value.clone());
                        aggregates.push(func);
                    } else {
                        let col = expr_to_column_name(expr)?;
                        non_agg_columns.push(col.clone());
                        agg_output_columns.push(alias.value.clone());
                    }
                }
                _ => {} // Wildcard etc. — not relevant for aggregate queries
            }
        }

        let has_aggregates = !aggregates.is_empty();

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
            if matches!(base_plan, PhysicalPlan::Join(_)) {
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
            } else if let Some(key) = self.extract_pk_from_where(&table_name, where_expr)? {
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
                if has_pk_eq {
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
            // Extract GROUP BY columns
            let group_by: Vec<String> = match &select.group_by {
                sqlparser::ast::GroupByExpr::Expressions(exprs) => exprs
                    .iter()
                    .map(expr_to_column_name)
                    .collect::<Result<Vec<_>, _>>()?,
                sqlparser::ast::GroupByExpr::All => {
                    // GROUP BY ALL: all non-aggregate columns
                    non_agg_columns.clone()
                }
            };

            // Strict mode: non-aggregate columns must appear in GROUP BY
            for col in &non_agg_columns {
                if !group_by.contains(col) {
                    return Err(PlanError::NonAggregatedColumn(col.clone()));
                }
            }

            // Build column index mapping from input plan.
            // MS10-T01 Iter001: unified through get_plan_output_columns, which
            // describes the input plan's real output shape on every form —
            // IndexScan/IndexScanAll inputs previously fell into the empty
            // fallback and silently NULL-ed aggregates (and mis-mapped GROUP
            // BY keys).
            let input_schema = self.get_plan_output_columns(&plan_with_where);
            let column_indices: HashMap<String, usize> = input_schema
                .iter()
                .enumerate()
                .map(|(i, col)| (col.to_lowercase(), i))
                .collect();

            // Build HAVING predicate BEFORE consuming agg_output_columns
            let having_pred = if let Some(having_expr) = &select.having {
                Some(self.build_having(having_expr, &agg_output_columns)?)
            } else {
                None
            };

            let agg_plan = PhysicalPlan::Aggregate(crate::executor::AggregateNode {
                input: Box::new(plan_with_where),
                group_by,
                aggregates,
                output_columns: agg_output_columns,
                table_name: table_name.clone(),
                column_indices,
            });

            // Wrap with HAVING if predicate was built
            if let Some(having_pred) = having_pred {
                PhysicalPlan::Having(crate::executor::HavingNode {
                    input: Box::new(agg_plan),
                    predicate: having_pred,
                    table_name: table_name.clone(),
                })
            } else {
                agg_plan
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

        Ok(plan)
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
