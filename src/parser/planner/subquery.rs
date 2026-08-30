//! PlanBuilder — IN/EXISTS subquery, correlated parameters, multi-level
//! correlation detection.
//!
//! MS07-T03: split from single-file `planner.rs` (T3 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::PlanBuilder;
use crate::executor::{
    AntiJoinNode, ColumnRef, CorrelatedParam, JoinCondition, PhysicalPlan, SemiJoinNode,
};
use crate::parser::error::PlanError;
use sqlparser::ast::{Expr, Query, SetExpr, TableFactor};

impl PlanBuilder {
    /// Try to build a SemiJoin/AntiJoin plan from WHERE subquery expressions.
    /// Returns Ok(Some(plan)) if the expression is an IN subquery or EXISTS subquery,
    /// returns Ok(None) if the expression does not match any subquery pattern.
    pub(crate) fn try_build_where_subquery(
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
    pub(crate) fn extract_subquery_table_names(subquery: &Query) -> Vec<String> {
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
    pub(crate) fn extract_correlated_params(
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
    pub(crate) fn collect_outer_column_refs(
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
    pub(crate) fn has_outer_refs_outside(expr: &Expr, allowed_tables: &[String]) -> bool {
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
    pub(crate) fn resolve_column_in_plan(
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
    pub(crate) fn get_subquery_first_column(
        &self,
        plan: &PhysicalPlan,
    ) -> Result<ColumnRef, PlanError> {
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
            PhysicalPlan::DataScan(node) => {
                // M19: subquery's no-WHERE plan is DataScan — same column layout.
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
}
