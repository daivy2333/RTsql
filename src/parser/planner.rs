//! PlanBuilder - Convert AST to PhysicalPlan
//!
//! M4: SQL Parser and Physical Plan

use crate::executor::{
    AggregateFunc, AggregateNode, AntiJoinNode, ColumnConstraint, ColumnDef, ColumnRef, ColumnType,
    ComparisonOp, ComparisonPredicate, ConstantExpression, CorrelatedParam, CreateTableNode,
    DeleteNode, DerivedScanNode, DropTableNode, ExpressionRef, FilterNode, HavingNode,
    IndexScanNode, InsertNode, JoinCondition, LimitNode, LogicalOp, LogicalPredicate,
    OrderByColumn, OutputColumn, ParameterExpression, PhysicalPlan, PredicateRef, ScanNode,
    SemiJoinNode, SortNode, SubqueryEvalNode, UpdateNode, Value,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::value::value_from_sqlparser;
use sqlparser::ast::{Expr, ObjectType, Query, SetExpr, Statement, TableFactor};
use std::collections::HashMap;
use std::sync::Arc;

/// PlanBuilder - Convert AST to PhysicalPlan
///
/// Stores table metadata (columns, primary keys) for validation and plan generation.
#[derive(Debug, Clone)]
pub struct PlanBuilder {
    /// Table name -> column names
    tables: HashMap<String, Vec<String>>,
    /// Table name -> primary key column name
    primary_keys: HashMap<String, String>,
    /// Set of inner table names when building a subquery (for detecting outer references).
    /// None when building a top-level query.
    pub(crate) inner_table_names: Option<Vec<String>>,
}

impl PlanBuilder {
    /// Create empty PlanBuilder
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            primary_keys: HashMap::new(),
            inner_table_names: None,
        }
    }

    /// Register table metadata
    pub fn register_table(&mut self, name: &str, columns: Vec<String>, pk: &str) {
        let name_lower = name.to_lowercase();
        self.tables.insert(name_lower.clone(), columns);
        self.primary_keys.insert(name_lower, pk.to_lowercase());
    }

    /// Build PhysicalPlan from Statement
    pub fn build_plan(&mut self, stmt: &Statement) -> Result<PhysicalPlan, PlanError> {
        match stmt {
            Statement::Query(query) => self.build_query(query),
            Statement::Insert {
                table_name,
                columns,
                source,
                ..
            } => self.build_insert(table_name, columns, source),
            Statement::Update {
                table,
                assignments,
                selection,
                ..
            } => self.build_update(table, assignments, selection),
            Statement::Delete {
                from, selection, ..
            } => self.build_delete(from, selection),
            Statement::CreateTable {
                name,
                columns,
                constraints,
                ..
            } => self.build_create_table(name, columns, constraints),
            Statement::Drop {
                object_type,
                if_exists,
                names,
                ..
            } => {
                if *object_type == ObjectType::Table {
                    self.build_drop_table(names, if_exists)
                } else {
                    Err(PlanError::UnsupportedStatement)
                }
            }
            _ => Err(PlanError::UnsupportedStatement),
        }
    }

    /// Validate table exists
    fn validate_table(&self, table_name: &str) -> Result<(), PlanError> {
        let name_lower = table_name.to_lowercase();
        if self.tables.contains_key(&name_lower) {
            Ok(())
        } else {
            Err(PlanError::ParseError(format!(
                "Table '{}' does not exist",
                table_name
            )))
        }
    }

    /// 解析列引用（支持 t.col 格式和纯列名）
    fn resolve_column_ref(
        &self,
        expr: &Expr,
        available_tables: &[String],
    ) -> Result<crate::executor::ColumnRef, PlanError> {
        match expr {
            // t.col 格式
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table = parts[0].value.to_lowercase();
                let column = parts[1].value.to_lowercase();

                // 验证表存在
                self.validate_table(&table)?;

                // 验证列存在
                let columns = self
                    .tables
                    .get(&table)
                    .ok_or_else(|| PlanError::TableNotFound(table.clone()))?;
                if !columns.iter().any(|c| c.to_lowercase() == column) {
                    return Err(PlanError::ColumnNotFound(column));
                }

                Ok(crate::executor::ColumnRef {
                    table: Some(table),
                    column,
                })
            }

            // 纯列名格式
            Expr::Identifier(ident) => {
                let column = ident.value.to_lowercase();

                // 查找列来源（检查所有可用表）
                let sources: Vec<String> = available_tables
                    .iter()
                    .filter(|t| {
                        self.tables
                            .get(*t)
                            .map(|cols| cols.iter().any(|c| c.to_lowercase() == column))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();

                match sources.len() {
                    0 => Err(PlanError::ColumnNotFound(column)),
                    1 => Ok(crate::executor::ColumnRef {
                        table: None,
                        column,
                    }),
                    _ => Err(PlanError::AmbiguousColumn(column)),
                }
            }

            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// 提取 JOIN ON 条件（支持 AND 组合等值条件）
    fn extract_join_conditions(
        &self,
        left_tables: &[String],
        right_table: &str,
        on_expr: &Expr,
    ) -> Result<Vec<crate::executor::JoinCondition>, PlanError> {
        use sqlparser::ast::BinaryOperator;

        // 处理 AND 组合
        if let Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } = on_expr
        {
            let left_conditions = self.extract_join_conditions(left_tables, right_table, left)?;
            let right_conditions = self.extract_join_conditions(left_tables, right_table, right)?;
            return Ok(left_conditions
                .into_iter()
                .chain(right_conditions)
                .collect());
        }

        // 处理单一等值条件
        if let Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } = on_expr
        {
            let left_ref = self.resolve_column_ref(left, left_tables)?;
            let right_ref = self.resolve_column_ref(right, &[right_table.to_string()])?;

            // 验证：左边列来自左表，右边列来自右表（或反序）
            if left_ref.table.as_deref() == Some(right_table) {
                // 反序：right.col = left.col，交换
                return Ok(vec![crate::executor::JoinCondition {
                    left_column: right_ref,
                    right_column: left_ref,
                }]);
            }

            Ok(vec![crate::executor::JoinCondition {
                left_column: left_ref,
                right_column: right_ref,
            }])
        } else {
            Err(PlanError::UnsupportedExpression)
        }
    }

    /// 构建 FROM + JOIN 链计划（支持列投影）
    /// 从 PhysicalPlan 中提取输出列名（用于派生表的列注册）
    #[allow(clippy::only_used_in_recursion)]
    fn get_plan_output_columns(&self, plan: &PhysicalPlan) -> Vec<String> {
        match plan {
            PhysicalPlan::Scan(node) => node.columns.clone(),
            PhysicalPlan::DerivedScan(node) => node.columns.clone(),
            PhysicalPlan::Filter(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::Sort(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::Limit(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::Aggregate(node) => node.output_columns.clone(),
            PhysicalPlan::Having(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::IndexScan(node) => node.columns.clone(),
            PhysicalPlan::Join(_) | PhysicalPlan::SemiJoin(_) | PhysicalPlan::AntiJoin(_) => {
                // JOIN 的列来自左右子计划的合并
                Vec::new()
            }
            PhysicalPlan::SubqueryEval(node) => self.get_plan_output_columns(&node.input),
            PhysicalPlan::Insert(_) | PhysicalPlan::Update(_) | PhysicalPlan::Delete(_) => {
                Vec::new()
            }
            PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => Vec::new(),
        }
    }

    fn build_from_clause_with_projection(
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
                let plan = PhysicalPlan::DerivedScan(DerivedScanNode {
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
            });

            // 解析 ON 条件
            let on_clause = on_clause.ok_or(PlanError::MissingOnClause)?;
            let conditions =
                self.extract_join_conditions(&current_tables, &right_table, on_clause)?;

            // 构建输出列（根据 qualified_columns 过滤）
            let all_columns: Vec<crate::executor::OutputColumn> = current_tables
                .iter()
                .flat_map(|t| {
                    let columns = self
                        .tables
                        .get(t)
                        .expect("validated table must exist in metadata");
                    columns
                        .iter()
                        .enumerate()
                        .map(|(idx, col)| crate::executor::OutputColumn {
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
                        .map(|(idx, col)| crate::executor::OutputColumn {
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
    fn build_query(&mut self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        // Extract Select body
        let select = extract_select_body(query)?;

        // === Scalar subquery detection in SELECT projection ===
        // Scan projection for Expr::Subquery items and build subquery plans
        // Also detect correlated parameters (outer table column references)
        let mut subquery_evals: Vec<(usize, PhysicalPlan, String, Vec<CorrelatedParam>)> =
            Vec::new();
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
                        columns: extract_columns(&select.projection)?,
                    })
                } else {
                    // Complex WHERE with PK - use Filter over Scan
                    let predicate = self.build_where(&table_name, where_expr)?;
                    PhysicalPlan::Filter(FilterNode {
                        input: Box::new(base_plan),
                        predicate,
                        table_name: table_name.clone(),
                    })
                }
            } else {
                // Non-PK WHERE - use Filter over Scan
                let predicate = self.build_where(&table_name, where_expr)?;
                PhysicalPlan::Filter(FilterNode {
                    input: Box::new(base_plan),
                    predicate,
                    table_name: table_name.clone(),
                })
            }
        } else {
            // No WHERE clause - full table scan
            base_plan
        };

        // === Aggregate function detection ===
        // Check if SELECT projection contains aggregate functions
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

            // Build column index mapping from input plan
            let input_schema = match &plan_with_where {
                PhysicalPlan::Scan(node) => node.columns.clone(),
                PhysicalPlan::Filter(node) => {
                    // Get schema from input of filter
                    match node.input.as_ref() {
                        PhysicalPlan::Scan(scan) => scan.columns.clone(),
                        _ => vec![],
                    }
                }
                _ => vec![],
            };
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

            let agg_plan = PhysicalPlan::Aggregate(AggregateNode {
                input: Box::new(plan_with_where),
                group_by,
                aggregates,
                output_columns: agg_output_columns,
                table_name: table_name.clone(),
                column_indices,
            });

            // Wrap with HAVING if predicate was built
            if let Some(having_pred) = having_pred {
                PhysicalPlan::Having(HavingNode {
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

            PhysicalPlan::Sort(SortNode {
                input: Box::new(plan_with_aggregate),
                order_by,
                table_name: table_name.clone(),
                columns: projection_columns.clone(),
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

            PhysicalPlan::Limit(LimitNode {
                input: Box::new(plan_with_order),
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
            plan = PhysicalPlan::SubqueryEval(SubqueryEvalNode {
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
    fn is_simple_pk_equality(&self, table_name: &str, expr: &Expr) -> Result<bool, PlanError> {
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

    /// Extract primary key from WHERE clause
    ///
    /// Only supports: pk_column = value
    fn extract_pk_from_where(
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

    /// Convert sqlparser BinaryOperator to ComparisonOp
    fn convert_comparison_op(&self, op: &sqlparser::ast::BinaryOperator) -> Option<ComparisonOp> {
        use sqlparser::ast::BinaryOperator as SqlOp;
        match op {
            SqlOp::Eq => Some(ComparisonOp::Eq),
            SqlOp::NotEq => Some(ComparisonOp::Ne),
            SqlOp::Gt => Some(ComparisonOp::Gt),
            SqlOp::Lt => Some(ComparisonOp::Lt),
            SqlOp::GtEq => Some(ComparisonOp::Ge),
            SqlOp::LtEq => Some(ComparisonOp::Le),
            _ => None,
        }
    }

    /// Build ExpressionRef from Expr for HAVING clause
    /// In HAVING context, the row is the aggregate output row with columns:
    ///   [group_col_0, ..., group_col_n, agg_result_0, ..., agg_result_m]
    /// The output_columns list gives the names in order, and the index in that
    /// list is the column index in the output row.
    fn build_having_expression(
        &self,
        expr: &Expr,
        output_columns: &[String],
    ) -> Result<ExpressionRef, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let ident_value = ident.value.to_uppercase();
                // Check for NULL constant
                if ident_value == "NULL" {
                    return Ok(Arc::new(ConstantExpression { value: Value::Null }));
                }
                // Column reference: look up in output_columns
                let column_name = ident.value.to_lowercase();
                let column_index = output_columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                    .ok_or_else(|| PlanError::ColumnNotFound(column_name.clone()))?;
                Ok(Arc::new(crate::executor::ColumnExpression {
                    column_name,
                    column_index,
                }))
            }
            Expr::Function(f) => {
                // Aggregate function reference in HAVING
                // Build the result column name and find its index in output_columns
                let name = f.name.to_string().to_uppercase();
                let result_col_name = match name.as_str() {
                    "COUNT" => {
                        if f.args.is_empty() {
                            "count_star".to_string()
                        } else {
                            match &f.args[0] {
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Wildcard,
                                ) => "count_star".to_string(),
                                sqlparser::ast::FunctionArg::Unnamed(
                                    sqlparser::ast::FunctionArgExpr::Expr(inner),
                                ) => {
                                    let col = expr_to_column_name(inner)?;
                                    format!("count_{}", col.to_lowercase())
                                }
                                _ => "count_star".to_string(),
                            }
                        }
                    }
                    "SUM" => {
                        let col = extract_single_column_arg(&f.args, "SUM")?;
                        format!("sum_{}", col.to_lowercase())
                    }
                    "AVG" => {
                        let col = extract_single_column_arg(&f.args, "AVG")?;
                        format!("avg_{}", col.to_lowercase())
                    }
                    "MIN" => {
                        let col = extract_single_column_arg(&f.args, "MIN")?;
                        format!("min_{}", col.to_lowercase())
                    }
                    "MAX" => {
                        let col = extract_single_column_arg(&f.args, "MAX")?;
                        format!("max_{}", col.to_lowercase())
                    }
                    _ => return Err(PlanError::UnsupportedExpression),
                };
                let column_index = output_columns
                    .iter()
                    .position(|c| c.to_lowercase() == result_col_name.to_lowercase())
                    .ok_or_else(|| {
                        PlanError::HavingNonAggregatedReference(result_col_name.clone())
                    })?;
                Ok(Arc::new(crate::executor::ColumnExpression {
                    column_name: result_col_name,
                    column_index,
                }))
            }
            Expr::Value(v) => {
                let value = value_from_sqlparser(v)?;
                Ok(Arc::new(ConstantExpression { value }))
            }
            // Handle negative numbers: -42
            Expr::UnaryOp {
                op: sqlparser::ast::UnaryOperator::Minus,
                expr: inner,
            } => {
                if let Expr::Value(v) = inner.as_ref() {
                    let value = value_from_sqlparser(v)?;
                    match value {
                        Value::Int(n) => Ok(Arc::new(ConstantExpression {
                            value: Value::Int(-n),
                        })),
                        Value::Float(f) => Ok(Arc::new(ConstantExpression {
                            value: Value::Float(-f),
                        })),
                        _ => Err(PlanError::UnsupportedValue),
                    }
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Build PredicateRef from HAVING clause expression
    /// Uses build_having_expression which knows about aggregate output columns
    fn build_having(
        &self,
        expr: &Expr,
        output_columns: &[String],
    ) -> Result<PredicateRef, PlanError> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                use sqlparser::ast::BinaryOperator as SqlOp;
                match op {
                    SqlOp::And => {
                        let left_pred = self.build_having(left, output_columns)?;
                        let right_pred = self.build_having(right, output_columns)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::And,
                            right: right_pred,
                        }))
                    }
                    SqlOp::Or => {
                        let left_pred = self.build_having(left, output_columns)?;
                        let right_pred = self.build_having(right, output_columns)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::Or,
                            right: right_pred,
                        }))
                    }
                    _ => {
                        let comp_op = self
                            .convert_comparison_op(op)
                            .ok_or(PlanError::UnsupportedExpression)?;
                        let left_expr = self.build_having_expression(left, output_columns)?;
                        let right_expr = self.build_having_expression(right, output_columns)?;
                        Ok(Arc::new(ComparisonPredicate {
                            left: left_expr,
                            op: comp_op,
                            right: right_expr,
                        }))
                    }
                }
            }
            Expr::Nested(expr) => self.build_having(expr, output_columns),
            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Build ExpressionRef from Expr
    fn build_expression(&self, table_name: &str, expr: &Expr) -> Result<ExpressionRef, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let ident_value = ident.value.to_uppercase();
                // Check for NULL constant
                if ident_value == "NULL" {
                    return Ok(Arc::new(ConstantExpression { value: Value::Null }));
                }
                // Column reference
                let column_name = ident.value.to_lowercase();
                let columns = self.tables.get(table_name).ok_or_else(|| {
                    PlanError::ParseError(format!("Table '{}' not found", table_name))
                })?;
                let column_index = columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                    .ok_or_else(|| {
                        PlanError::ParseError(format!(
                            "Column '{}' not found in table '{}'",
                            column_name, table_name
                        ))
                    })?;
                Ok(Arc::new(crate::executor::ColumnExpression {
                    column_name,
                    column_index,
                }))
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table_ref = parts[0].value.to_lowercase();
                let column_name = parts[1].value.to_lowercase();

                // Check if this is an outer (correlated) reference
                if let Some(ref inner_tables) = self.inner_table_names {
                    if !inner_tables
                        .iter()
                        .any(|t| t.eq_ignore_ascii_case(&table_ref))
                    {
                        let param_name = format!("{}.{}", table_ref, column_name);
                        return Ok(Arc::new(ParameterExpression::new(param_name)));
                    }
                }

                // Resolve the table reference
                let columns = self.tables.get(&table_ref).ok_or_else(|| {
                    PlanError::ParseError(format!("Table '{}' not found", table_ref))
                })?;
                let column_index = columns
                    .iter()
                    .position(|c| c.to_lowercase() == column_name)
                    .ok_or_else(|| {
                        PlanError::ParseError(format!(
                            "Column '{}' not found in table '{}'",
                            column_name, table_ref
                        ))
                    })?;
                Ok(Arc::new(crate::executor::ColumnExpression {
                    column_name,
                    column_index,
                }))
            }
            Expr::Value(v) => {
                // Constant value
                let value = value_from_sqlparser(v)?;
                Ok(Arc::new(ConstantExpression { value }))
            }
            // Handle negative numbers: -42
            Expr::UnaryOp {
                op: sqlparser::ast::UnaryOperator::Minus,
                expr: inner,
            } => {
                if let Expr::Value(v) = inner.as_ref() {
                    let value = value_from_sqlparser(v)?;
                    match value {
                        Value::Int(n) => Ok(Arc::new(ConstantExpression {
                            value: Value::Int(-n),
                        })),
                        Value::Float(f) => Ok(Arc::new(ConstantExpression {
                            value: Value::Float(-f),
                        })),
                        _ => Err(PlanError::UnsupportedValue),
                    }
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Build PredicateRef from WHERE clause expression
    fn build_where(&self, table_name: &str, expr: &Expr) -> Result<PredicateRef, PlanError> {
        match expr {
            Expr::BinaryOp { left, op, right } => {
                // Check if this is a logical operator (AND/OR)
                use sqlparser::ast::BinaryOperator as SqlOp;
                match op {
                    SqlOp::And => {
                        let left_pred = self.build_where(table_name, left)?;
                        let right_pred = self.build_where(table_name, right)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::And,
                            right: right_pred,
                        }))
                    }
                    SqlOp::Or => {
                        let left_pred = self.build_where(table_name, left)?;
                        let right_pred = self.build_where(table_name, right)?;
                        Ok(Arc::new(LogicalPredicate {
                            left: left_pred,
                            op: LogicalOp::Or,
                            right: right_pred,
                        }))
                    }
                    _ => {
                        // Try to convert to comparison operator
                        let comp_op = self
                            .convert_comparison_op(op)
                            .ok_or(PlanError::UnsupportedExpression)?;
                        let left_expr = self.build_expression(table_name, left)?;
                        let right_expr = self.build_expression(table_name, right)?;
                        Ok(Arc::new(ComparisonPredicate {
                            left: left_expr,
                            op: comp_op,
                            right: right_expr,
                        }))
                    }
                }
            }
            // Parenthesized expression - just unwrap
            Expr::Nested(expr) => self.build_where(table_name, expr),
            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Try to build a SemiJoin/AntiJoin plan from WHERE subquery expressions.
    /// Returns Ok(Some(plan)) if the expression is an IN subquery or EXISTS subquery,
    /// returns Ok(None) if the expression does not match any subquery pattern.
    fn try_build_where_subquery(
        &mut self,
        expr: &Expr,
        base_plan: &PhysicalPlan,
        table_name: &str,
        projection_columns: &[String],
    ) -> Result<Option<PhysicalPlan>, PlanError> {
        match expr {
            Expr::InSubquery {
                expr: left_expr,
                subquery,
                negated,
            } => {
                let inner_tables = Self::extract_subquery_table_names(subquery);

                self.inner_table_names = Some(inner_tables.clone());
                let right_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.inner_table_names = None;
                        return Err(e);
                    }
                };
                self.inner_table_names = None;

                let left_column = self.resolve_column_in_plan(left_expr, table_name)?;
                let right_column = self.get_subquery_first_column(&right_plan)?;

                let conditions = vec![JoinCondition {
                    left_column,
                    right_column,
                }];

                let output_columns =
                    self.build_output_columns_for_table(table_name, projection_columns);

                // Detect correlated parameters
                let correlated_params = self.extract_correlated_params(subquery, &inner_tables)?;

                if *negated {
                    Ok(Some(PhysicalPlan::AntiJoin(AntiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        correlated_params,
                    })))
                } else {
                    Ok(Some(PhysicalPlan::SemiJoin(SemiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        correlated_params,
                    })))
                }
            }
            Expr::Exists { subquery, negated } => {
                let inner_tables = Self::extract_subquery_table_names(subquery);

                self.inner_table_names = Some(inner_tables.clone());
                let right_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.inner_table_names = None;
                        return Err(e);
                    }
                };
                self.inner_table_names = None;

                let output_columns =
                    self.build_output_columns_for_table(table_name, projection_columns);

                let conditions = vec![]; // EXISTS does not need equality conditions

                // Detect correlated parameters
                let correlated_params = self.extract_correlated_params(subquery, &inner_tables)?;

                if *negated {
                    Ok(Some(PhysicalPlan::AntiJoin(AntiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        correlated_params,
                    })))
                } else {
                    Ok(Some(PhysicalPlan::SemiJoin(SemiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        correlated_params,
                    })))
                }
            }
            _ => Ok(None),
        }
    }

    /// Extract table names from a subquery's FROM clause
    fn extract_subquery_table_names(subquery: &Query) -> Vec<String> {
        match subquery.body.as_ref() {
            SetExpr::Select(select) => select
                .from
                .iter()
                .flat_map(|twj| {
                    let mut names = Vec::new();
                    if let TableFactor::Table { name, .. } = &twj.relation {
                        names.push(name.to_string().to_lowercase());
                    }
                    for join in &twj.joins {
                        if let TableFactor::Table { name, .. } = &join.relation {
                            names.push(name.to_string().to_lowercase());
                        }
                    }
                    names
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Extract correlated parameters from a subquery by scanning its WHERE clause
    /// for column references to tables NOT in the subquery's own FROM clause.
    ///
    /// For each outer column reference (e.g., `emp.region` where `emp` is not in the
    /// subquery's FROM), creates a CorrelatedParam that maps the outer table/column
    /// to the inner column index where the value will be injected at execution time.
    fn extract_correlated_params(
        &self,
        subquery: &Query,
        inner_tables: &[String],
    ) -> Result<Vec<CorrelatedParam>, PlanError> {
        let where_expr = match subquery.body.as_ref() {
            SetExpr::Select(select) => select.selection.as_ref(),
            _ => return Ok(Vec::new()),
        };
        let Some(where_expr) = where_expr else {
            return Ok(Vec::new());
        };

        let mut params = Vec::new();
        self.collect_outer_column_refs(where_expr, inner_tables, &mut params)?;
        Ok(params)
    }

    /// Recursively walk an expression tree to find outer column references.
    /// An outer column reference is a CompoundIdentifier (table.column) where
    /// the table is NOT in the inner_tables list.
    #[allow(clippy::only_used_in_recursion)]
    fn collect_outer_column_refs(
        &self,
        expr: &Expr,
        inner_tables: &[String],
        params: &mut Vec<CorrelatedParam>,
    ) -> Result<(), PlanError> {
        match expr {
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table = parts[0].value.to_lowercase();
                let column = parts[1].value.to_lowercase();

                // If this table is NOT in the inner tables, it's an outer reference
                if !inner_tables.iter().any(|t| t.eq_ignore_ascii_case(&table)) {
                    // Build qualified param name (e.g. "emp.dept") for matching
                    // ParameterExpression nodes at execution time
                    let param_name = format!("{}.{}", table, column);
                    params.push(CorrelatedParam::new(table, column, param_name));
                }
                Ok(())
            }
            // Recurse into binary operations
            Expr::BinaryOp { left, right, .. } => {
                self.collect_outer_column_refs(left, inner_tables, params)?;
                self.collect_outer_column_refs(right, inner_tables, params)?;
                Ok(())
            }
            // Recurse into unary operations
            Expr::UnaryOp { expr, .. } => {
                self.collect_outer_column_refs(expr, inner_tables, params)?;
                Ok(())
            }
            // Recurse into nested expressions
            Expr::Nested(expr) => {
                self.collect_outer_column_refs(expr, inner_tables, params)?;
                Ok(())
            }
            // Recurse into BETWEEN
            Expr::Between {
                expr, low, high, ..
            } => {
                self.collect_outer_column_refs(expr, inner_tables, params)?;
                self.collect_outer_column_refs(low, inner_tables, params)?;
                self.collect_outer_column_refs(high, inner_tables, params)?;
                Ok(())
            }
            // Recurse into IN list
            Expr::InList { expr, .. } => {
                self.collect_outer_column_refs(expr, inner_tables, params)?;
                Ok(())
            }
            // Recurse into IN subquery
            Expr::InSubquery { expr, subquery, .. } => {
                // Multi-level correlated: check for nested outer refs beyond inner_tables
                let nested_inner_tables = Self::extract_subquery_table_names(subquery);
                if let SetExpr::Select(select) = subquery.body.as_ref() {
                    if let Some(ref where_expr) = select.selection {
                        let all_allowed: Vec<String> = inner_tables
                            .iter()
                            .chain(nested_inner_tables.iter())
                            .cloned()
                            .collect();
                        if Self::has_outer_refs_outside(where_expr, &all_allowed) {
                            return Err(PlanError::CorrelatedParamError(
                                "Multi-level correlated subqueries are not supported".to_string(),
                            ));
                        }
                    }
                }
                self.collect_outer_column_refs(expr, inner_tables, params)?;
                Ok(())
            }
            // Recurse into EXISTS / NOT EXISTS (subquery itself may have outer refs)
            Expr::Exists { subquery, .. } => {
                // Multi-level correlated: check for nested outer refs beyond inner_tables
                let nested_inner_tables = Self::extract_subquery_table_names(subquery);
                if let SetExpr::Select(select) = subquery.body.as_ref() {
                    if let Some(ref where_expr) = select.selection {
                        let all_allowed: Vec<String> = inner_tables
                            .iter()
                            .chain(nested_inner_tables.iter())
                            .cloned()
                            .collect();
                        if Self::has_outer_refs_outside(where_expr, &all_allowed) {
                            return Err(PlanError::CorrelatedParamError(
                                "Multi-level correlated subqueries are not supported".to_string(),
                            ));
                        }
                    }
                }
                Ok(())
            }
            // Recurse into CASE
            Expr::Case {
                operand,
                conditions,
                results,
                else_result,
                ..
            } => {
                if let Some(op) = operand {
                    self.collect_outer_column_refs(op, inner_tables, params)?;
                }
                for cond in conditions {
                    self.collect_outer_column_refs(cond, inner_tables, params)?;
                }
                for res in results {
                    self.collect_outer_column_refs(res, inner_tables, params)?;
                }
                if let Some(else_expr) = else_result {
                    self.collect_outer_column_refs(else_expr, inner_tables, params)?;
                }
                Ok(())
            }
            // Simple identifiers, values, functions, etc. — no outer refs to collect
            _ => Ok(()),
        }
    }

    /// Check if an expression tree contains column references to tables outside the allowed set
    fn has_outer_refs_outside(expr: &Expr, allowed_tables: &[String]) -> bool {
        match expr {
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table = parts[0].value.to_lowercase();
                !allowed_tables
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(&table))
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::has_outer_refs_outside(left, allowed_tables)
                    || Self::has_outer_refs_outside(right, allowed_tables)
            }
            Expr::Nested(inner) => Self::has_outer_refs_outside(inner, allowed_tables),
            Expr::UnaryOp { expr, .. } => Self::has_outer_refs_outside(expr, allowed_tables),
            Expr::Between {
                expr, low, high, ..
            } => {
                Self::has_outer_refs_outside(expr, allowed_tables)
                    || Self::has_outer_refs_outside(low, allowed_tables)
                    || Self::has_outer_refs_outside(high, allowed_tables)
            }
            Expr::InList { expr, .. } => Self::has_outer_refs_outside(expr, allowed_tables),
            Expr::InSubquery { expr, subquery, .. } => {
                if Self::has_outer_refs_outside(expr, allowed_tables) {
                    return true;
                }
                let nested_tables = Self::extract_subquery_table_names(subquery);
                let all_allowed: Vec<String> = allowed_tables
                    .iter()
                    .chain(nested_tables.iter())
                    .cloned()
                    .collect();
                if let SetExpr::Select(select) = subquery.body.as_ref() {
                    select
                        .selection
                        .as_ref()
                        .is_some_and(|w| Self::has_outer_refs_outside(w, &all_allowed))
                } else {
                    false
                }
            }
            Expr::Exists { subquery, .. } => {
                let nested_tables = Self::extract_subquery_table_names(subquery);
                let all_allowed: Vec<String> = allowed_tables
                    .iter()
                    .chain(nested_tables.iter())
                    .cloned()
                    .collect();
                if let SetExpr::Select(select) = subquery.body.as_ref() {
                    select
                        .selection
                        .as_ref()
                        .is_some_and(|w| Self::has_outer_refs_outside(w, &all_allowed))
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Resolve column reference from an expression (for IN subquery left-table column)
    fn resolve_column_in_plan(
        &self,
        expr: &Expr,
        table_name: &str,
    ) -> Result<ColumnRef, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let column = ident.value.to_lowercase();
                Ok(ColumnRef {
                    table: Some(table_name.to_string()),
                    column,
                })
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table = parts[0].value.to_lowercase();
                let column = parts[1].value.to_lowercase();
                Ok(ColumnRef {
                    table: Some(table),
                    column,
                })
            }
            _ => Err(PlanError::UnsupportedExpression),
        }
    }

    /// Get the first output column name from a subquery plan (IN subquery requires single column)
    #[allow(clippy::only_used_in_recursion)]
    fn get_subquery_first_column(&self, plan: &PhysicalPlan) -> Result<ColumnRef, PlanError> {
        match plan {
            PhysicalPlan::Scan(node) => {
                if node.columns.is_empty() {
                    return Err(PlanError::SubqueryReturnsMultipleColumns);
                }
                Ok(ColumnRef {
                    table: Some(node.table_name.clone()),
                    column: node.columns[0].clone(),
                })
            }
            PhysicalPlan::Filter(node) => self.get_subquery_first_column(&node.input),
            PhysicalPlan::Aggregate(node) => {
                if node.output_columns.is_empty() {
                    return Err(PlanError::SubqueryReturnsMultipleColumns);
                }
                Ok(ColumnRef {
                    table: Some(node.table_name.clone()),
                    column: node.output_columns[0].clone(),
                })
            }
            PhysicalPlan::SemiJoin(node) => {
                if node.output_columns.is_empty() {
                    return Err(PlanError::SubqueryReturnsMultipleColumns);
                }
                Ok(ColumnRef {
                    table: Some(node.output_columns[0].table.clone().unwrap_or_default()),
                    column: node.output_columns[0].column.clone(),
                })
            }
            PhysicalPlan::AntiJoin(node) => {
                if node.output_columns.is_empty() {
                    return Err(PlanError::SubqueryReturnsMultipleColumns);
                }
                Ok(ColumnRef {
                    table: Some(node.output_columns[0].table.clone().unwrap_or_default()),
                    column: node.output_columns[0].column.clone(),
                })
            }
            _ => Err(PlanError::SubqueryReturnsMultipleColumns),
        }
    }

    /// Build output_columns for a single-table query
    fn build_output_columns_for_table(
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

    /// Build PhysicalPlan for INSERT statement
    fn build_insert(
        &self,
        table_name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::Ident],
        source: &Option<Box<sqlparser::ast::Query>>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name_str = extract_name_from_object(table_name);
        self.validate_table(&table_name_str)?;

        // Extract column names
        let columns: Vec<String> = columns.iter().map(|c| c.value.to_lowercase()).collect();

        // Extract values from source
        let values = self.extract_insert_values(source)?;

        Ok(PhysicalPlan::Insert(InsertNode {
            table_name: table_name_str,
            columns,
            values,
        }))
    }

    /// Extract values from INSERT source (VALUES clause)
    fn extract_insert_values(
        &self,
        source: &Option<Box<sqlparser::ast::Query>>,
    ) -> Result<Vec<Vec<Value>>, PlanError> {
        let source = source
            .as_ref()
            .ok_or_else(|| PlanError::MissingField("VALUES".into()))?;

        // Expect SetExpr::Values
        match source.body.as_ref() {
            SetExpr::Values(values) => {
                values
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|expr| {
                                match expr {
                                    Expr::Value(v) => value_from_sqlparser(v),
                                    Expr::Identifier(ident) => {
                                        // Handle NULL identifier
                                        if ident.value.to_uppercase() == "NULL" {
                                            Ok(Value::Null)
                                        } else {
                                            Err(PlanError::UnsupportedValue)
                                        }
                                    }
                                    _ => Err(PlanError::UnsupportedValue),
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()
            }
            _ => Err(PlanError::UnsupportedStatement),
        }
    }

    /// Convert sqlparser DataType to ColumnType
    fn convert_data_type(&self, data_type: &sqlparser::ast::DataType) -> ColumnType {
        use sqlparser::ast::DataType;
        match data_type {
            // Integer types -> Int
            DataType::Int(_)
            | DataType::Int4(_)
            | DataType::Integer(_)
            | DataType::BigInt(_)
            | DataType::Int8(_)
            | DataType::SmallInt(_)
            | DataType::Int2(_)
            | DataType::TinyInt(_)
            | DataType::MediumInt(_) => ColumnType::Int,

            // String types -> String
            DataType::Varchar(_)
            | DataType::Nvarchar(_)
            | DataType::Char(_)
            | DataType::Character(_)
            | DataType::CharacterVarying(_)
            | DataType::CharVarying(_)
            | DataType::Text
            | DataType::Clob(_)
            | DataType::CharacterLargeObject(_)
            | DataType::CharLargeObject(_) => ColumnType::String,

            // Float types -> Float
            DataType::Float(_)
            | DataType::Float4
            | DataType::Float64
            | DataType::Real
            | DataType::Double
            | DataType::Float8
            | DataType::DoublePrecision => ColumnType::Float,

            // Boolean types -> Bool
            DataType::Bool | DataType::Boolean => ColumnType::Bool,

            // Unknown/unsupported types -> Null (placeholder)
            _ => ColumnType::String, // Default to String for unknown types
        }
    }

    /// Extract column constraints from sqlparser ColumnDef
    fn extract_column_constraints(
        &self,
        column: &sqlparser::ast::ColumnDef,
    ) -> Result<Vec<ColumnConstraint>, PlanError> {
        let mut constraints = Vec::new();

        for option in &column.options {
            match &option.option {
                sqlparser::ast::ColumnOption::NotNull => {
                    constraints.push(ColumnConstraint::NotNull);
                }
                sqlparser::ast::ColumnOption::Unique {
                    is_primary: false, ..
                } => {
                    constraints.push(ColumnConstraint::Unique);
                }
                sqlparser::ast::ColumnOption::Default(expr) => {
                    let value = self.extract_default_value(expr)?;
                    constraints.push(ColumnConstraint::DefaultValue(value));
                }
                // PrimaryKey (is_primary: true) is handled separately by extract_primary_key
                // Null, ForeignKey, Check, DialectSpecific, etc. are ignored
                _ => {}
            }
        }

        Ok(constraints)
    }

    /// Extract default value from expression
    fn extract_default_value(&self, expr: &Expr) -> Result<Value, PlanError> {
        match expr {
            Expr::Value(v) => value_from_sqlparser(v),
            Expr::Identifier(ident) => {
                if ident.value.to_uppercase() == "NULL" {
                    Ok(Value::Null)
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            // Handle negative numbers: -42
            Expr::UnaryOp {
                op: sqlparser::ast::UnaryOperator::Minus,
                expr,
            } => {
                if let Expr::Value(v) = expr.as_ref() {
                    let value = value_from_sqlparser(v)?;
                    match value {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err(PlanError::UnsupportedValue),
                    }
                } else {
                    Err(PlanError::UnsupportedValue)
                }
            }
            _ => Err(PlanError::UnsupportedValue),
        }
    }

    /// Extract primary key from column constraints and table constraints
    fn extract_primary_key(
        &self,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
    ) -> Result<Option<String>, PlanError> {
        let mut pk_candidates = Vec::new();

        // Check column-level PRIMARY KEY constraints
        for column in columns {
            for option in &column.options {
                if let sqlparser::ast::ColumnOption::Unique {
                    is_primary: true, ..
                } = &option.option
                {
                    pk_candidates.push(column.name.value.to_lowercase());
                }
            }
        }

        // Check table-level PRIMARY KEY constraints (Unique { is_primary: true })
        for constraint in constraints {
            if let sqlparser::ast::TableConstraint::Unique {
                is_primary: true,
                columns: pk_columns,
                ..
            } = constraint
            {
                for pk_col in pk_columns {
                    pk_candidates.push(pk_col.value.to_lowercase());
                }
            }
        }

        // Validate: only single-column PK supported
        match pk_candidates.len() {
            0 => Ok(None),
            1 => Ok(Some(pk_candidates[0].clone())),
            _ => Err(PlanError::MultiplePrimaryKey),
        }
    }

    /// Build PhysicalPlan for CREATE TABLE statement
    fn build_create_table(
        &self,
        name: &sqlparser::ast::ObjectName,
        columns: &[sqlparser::ast::ColumnDef],
        constraints: &[sqlparser::ast::TableConstraint],
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name = name.to_string().to_lowercase();

        // Check for empty columns
        if columns.is_empty() {
            return Err(PlanError::EmptyColumnDefinition);
        }

        // Extract column definitions
        let column_defs: Vec<ColumnDef> = columns
            .iter()
            .map(|col| {
                let col_name = col.name.value.to_lowercase();
                let col_type = self.convert_data_type(&col.data_type);
                let col_constraints = self.extract_column_constraints(col)?;
                Ok(ColumnDef {
                    name: col_name,
                    data_type: col_type,
                    constraints: col_constraints,
                })
            })
            .collect::<Result<Vec<_>, PlanError>>()?;

        // Extract primary key
        let primary_key = self.extract_primary_key(columns, constraints)?;

        Ok(PhysicalPlan::CreateTable(CreateTableNode {
            table_name,
            columns: column_defs,
            primary_key,
        }))
    }

    /// Build PhysicalPlan for DROP TABLE statement
    fn build_drop_table(
        &self,
        names: &[sqlparser::ast::ObjectName],
        if_exists: &bool,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name (only single table supported)
        if names.is_empty() {
            return Err(PlanError::MissingField("table name".into()));
        }

        let table_name = names[0].to_string().to_lowercase();

        Ok(PhysicalPlan::DropTable(DropTableNode {
            table_name,
            if_exists: *if_exists,
        }))
    }

    /// Build PhysicalPlan for UPDATE statement
    ///
    /// Only supports single column update with primary key WHERE clause
    fn build_update(
        &self,
        table: &sqlparser::ast::TableWithJoins,
        assignments: &[sqlparser::ast::Assignment],
        selection: &Option<Expr>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name
        let table_name = extract_table_name(std::slice::from_ref(table))?;
        self.validate_table(&table_name)?;

        // Extract primary key from WHERE clause
        let where_expr = selection
            .as_ref()
            .ok_or_else(|| PlanError::MissingField("WHERE clause for UPDATE".into()))?;
        let key = self
            .extract_pk_from_where(&table_name, where_expr)?
            .ok_or_else(|| {
                PlanError::ParseError("UPDATE requires primary key equality in WHERE clause".into())
            })?;

        // Only support single column update
        if assignments.len() != 1 {
            return Err(PlanError::UnsupportedStatement);
        }

        let assignment = &assignments[0];

        // Extract column name
        if assignment.id.len() != 1 {
            return Err(PlanError::UnsupportedStatement);
        }
        let column = assignment.id[0].value.to_lowercase();

        // Extract new value
        let new_value = match &assignment.value {
            Expr::Value(v) => value_from_sqlparser(v)?,
            Expr::Identifier(ident) => {
                if ident.value.to_uppercase() == "NULL" {
                    Value::Null
                } else {
                    return Err(PlanError::UnsupportedValue);
                }
            }
            _ => return Err(PlanError::UnsupportedValue),
        };

        Ok(PhysicalPlan::Update(UpdateNode {
            table_name,
            key,
            column,
            new_value,
        }))
    }

    /// Build PhysicalPlan for DELETE statement
    ///
    /// Only supports primary key WHERE clause
    fn build_delete(
        &self,
        from: &sqlparser::ast::FromTable,
        selection: &Option<Expr>,
    ) -> Result<PhysicalPlan, PlanError> {
        // Extract table name from FromTable
        let table_with_joins = match from {
            sqlparser::ast::FromTable::WithFromKeyword(tables) => tables,
            sqlparser::ast::FromTable::WithoutKeyword(tables) => tables,
        };
        let table_name = extract_table_name(table_with_joins)?;
        self.validate_table(&table_name)?;

        // Extract primary key from WHERE clause
        let where_expr = selection
            .as_ref()
            .ok_or_else(|| PlanError::MissingField("WHERE clause for DELETE".into()))?;
        let key = self
            .extract_pk_from_where(&table_name, where_expr)?
            .ok_or_else(|| {
                PlanError::ParseError("DELETE requires primary key equality in WHERE clause".into())
            })?;

        Ok(PhysicalPlan::Delete(DeleteNode { table_name, key }))
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

/// Check if an Expr is an aggregate function
fn is_aggregate_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Function(f) if {
        let name = f.name.to_string().to_uppercase();
        matches!(name.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
    })
}

/// Extract AggregateFunc from an Expr, returns None if not an aggregate
fn extract_aggregate_func(expr: &Expr) -> Result<Option<AggregateFunc>, PlanError> {
    match expr {
        Expr::Function(f) => {
            let name = f.name.to_string().to_uppercase();
            match name.as_str() {
                "COUNT" => {
                    if f.args.is_empty() {
                        return Err(PlanError::InvalidAggregateArgument(
                            "COUNT requires argument or *".to_string(),
                        ));
                    }
                    match &f.args[0] {
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Wildcard,
                        ) => Ok(Some(AggregateFunc::CountStar)),
                        sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(inner),
                        ) => {
                            let col = expr_to_column_name(inner)?;
                            Ok(Some(AggregateFunc::Count(col)))
                        }
                        _ => Err(PlanError::InvalidAggregateArgument(
                            "COUNT argument must be * or column".to_string(),
                        )),
                    }
                }
                "SUM" => {
                    let col = extract_single_column_arg(&f.args, "SUM")?;
                    Ok(Some(AggregateFunc::Sum(col)))
                }
                "AVG" => {
                    let col = extract_single_column_arg(&f.args, "AVG")?;
                    Ok(Some(AggregateFunc::Avg(col)))
                }
                "MIN" => {
                    let col = extract_single_column_arg(&f.args, "MIN")?;
                    Ok(Some(AggregateFunc::Min(col)))
                }
                "MAX" => {
                    let col = extract_single_column_arg(&f.args, "MAX")?;
                    Ok(Some(AggregateFunc::Max(col)))
                }
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

fn extract_single_column_arg(
    args: &[sqlparser::ast::FunctionArg],
    func_name: &str,
) -> Result<String, PlanError> {
    if args.len() != 1 {
        return Err(PlanError::InvalidAggregateArgument(format!(
            "{} requires exactly one argument",
            func_name
        )));
    }
    match &args[0] {
        sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(expr)) => {
            expr_to_column_name(expr)
        }
        _ => Err(PlanError::InvalidAggregateArgument(format!(
            "{} argument must be a column",
            func_name
        ))),
    }
}

/// Extract column name from Expr (Identifier, CompoundIdentifier, or Value literal)
fn expr_to_column_name(expr: &Expr) -> Result<String, PlanError> {
    match expr {
        Expr::Identifier(ident) => Ok(ident.value.clone()),
        Expr::CompoundIdentifier(parts) if !parts.is_empty() => {
            Ok(parts.last().unwrap().value.clone())
        }
        Expr::Value(v) => Ok(format!("_{}", v)),
        _ => Err(PlanError::InvalidAggregateArgument(
            "Expected column name".to_string(),
        )),
    }
}

impl Default for PlanBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_builder_new() {
        let builder = PlanBuilder::new();
        assert!(builder.tables.is_empty());
        assert!(builder.primary_keys.is_empty());
    }

    #[test]
    fn test_register_table() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        assert!(builder.tables.contains_key("users"));
        assert!(builder.primary_keys.contains_key("users"));
        assert_eq!(builder.primary_keys.get("users"), Some(&"id".to_string()));
    }

    #[test]
    fn test_validate_table() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into()], "id");

        assert!(builder.validate_table("users").is_ok());
        assert!(builder.validate_table("nonexistent").is_err());
    }

    #[test]
    fn test_build_query_scan() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "SELECT id, name FROM users";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Scan(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.columns, vec!["id", "name"]);
            }
            _ => panic!("Expected Scan plan"),
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
    fn test_build_insert() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Insert(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.columns, vec!["id", "name"]);
                assert_eq!(node.values.len(), 1);
                assert_eq!(node.values[0].len(), 2);
                assert_eq!(node.values[0][0], Value::Int(1));
                assert_eq!(node.values[0][1], Value::String("Alice".to_string()));
            }
            _ => panic!("Expected Insert plan"),
        }
    }

    #[test]
    fn test_build_update() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "UPDATE users SET name = 'Bob' WHERE id = 1";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Update(node) => {
                assert_eq!(node.table_name, "users");
                assert_eq!(node.column, "name");
                assert_eq!(node.new_value, Value::String("Bob".to_string()));
                let expected_key = Value::Int(1).to_key().unwrap();
                assert_eq!(node.key, expected_key);
            }
            _ => panic!("Expected Update plan"),
        }
    }

    #[test]
    fn test_build_delete() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "DELETE FROM users WHERE id = 1";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Delete(node) => {
                assert_eq!(node.table_name, "users");
                let expected_key = Value::Int(1).to_key().unwrap();
                assert_eq!(node.key, expected_key);
            }
            _ => panic!("Expected Delete plan"),
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
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        // Non-PK WHERE clause - should generate Filter plan
        let sql = "SELECT * FROM users WHERE name = 'Alice'";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Filter(_) => {} // Expected - Filter for non-PK WHERE
            _ => panic!("Expected Filter plan for non-PK WHERE"),
        }
    }

    #[test]
    fn test_insert_multiple_rows() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob')";
        let stmts = parse_sql(sql).unwrap();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Insert(node) => {
                assert_eq!(node.values.len(), 2);
                assert_eq!(
                    node.values[0],
                    vec![Value::Int(1), Value::String("Alice".to_string())]
                );
                assert_eq!(
                    node.values[1],
                    vec![Value::Int(2), Value::String("Bob".to_string())]
                );
            }
            _ => panic!("Expected Insert plan"),
        }
    }
}
