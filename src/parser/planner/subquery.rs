//! PlanBuilder — IN/EXISTS subquery, correlated parameters, multi-level
//! correlation detection.
//!
//! MS07-T03: split from single-file `planner.rs` (T3 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::query::projected_columns;
use super::PlanBuilder;
use crate::executor::{
    AntiJoinNode, ColumnRef, CorrelatedParam, JoinCondition, PhysicalPlan, ScalarSubqueryBinding,
    SemiJoinNode,
};
use crate::parser::error::PlanError;
use sqlparser::ast::{Expr, FunctionArg, FunctionArgExpr, Query, SetExpr, TableFactor};

impl PlanBuilder {
    /// Try to build a SemiJoin/AntiJoin plan from WHERE subquery expressions.
    /// Returns Ok(Some(plan)) if the expression is an IN subquery or EXISTS subquery,
    /// returns Ok(None) if the expression does not match any subquery pattern.
    ///
    /// T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：`projection_columns`
    /// 不再消费——基座 `output_columns` 恒为左表全列（R5），调用方签名保持
    /// 不变（外层按基表 schema 装配裁剪位）。
    pub(crate) fn try_build_where_subquery(
        &mut self,
        expr: &Expr,
        base_plan: &PhysicalPlan,
        table_name: &str,
        projection_columns: &[String],
    ) -> Result<Option<PhysicalPlan>, PlanError> {
        let _ = projection_columns;
        match expr {
            Expr::InSubquery {
                expr: left_expr,
                subquery,
                negated,
            } => {
                let inner_tables = Self::extract_subquery_table_names(subquery);

                self.inner_table_names = Some(inner_tables.clone());
                let saved_subquery_ctx = self.building_subquery;
                // T5：表达式型子查询上下文——FROM 派生标志复位（IN 子查询内
                // 集合操作维持拒绝）。
                let saved_from_derived = self.building_from_derived;
                self.building_subquery = true;
                self.building_from_derived = false;
                let right_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.building_subquery = saved_subquery_ctx;
                        self.building_from_derived = saved_from_derived;
                        self.inner_table_names = None;
                        return Err(e);
                    }
                };
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.inner_table_names = None;

                let left_column = self.resolve_column_in_plan(left_expr, table_name)?;
                let right_column = self.get_subquery_first_column(&right_plan)?;

                let conditions = vec![JoinCondition {
                    left_column,
                    right_column,
                }];

                // T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：
                // 基座承载左表全宽行——`output_columns` 为基表全列（行内容 =
                // 左行直通），SELECT 清单裁剪由下游承担（无 ORDER BY 时本节点
                // `projection`，ORDER BY 时 Sort）。R5：排序键可见非输出列。
                let output_columns = self.build_full_output_columns_for_table(table_name);

                // Detect correlated parameters
                let correlated_params = self.extract_correlated_params(subquery, &inner_tables)?;

                if *negated {
                    Ok(Some(PhysicalPlan::AntiJoin(AntiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        projection: Vec::new(),
                        correlated_params,
                    })))
                } else {
                    Ok(Some(PhysicalPlan::SemiJoin(SemiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        projection: Vec::new(),
                        correlated_params,
                    })))
                }
            }
            Expr::Exists { subquery, negated } => {
                let inner_tables = Self::extract_subquery_table_names(subquery);

                self.inner_table_names = Some(inner_tables.clone());
                let saved_subquery_ctx = self.building_subquery;
                // T5：表达式型子查询上下文——FROM 派生标志复位（EXISTS 子查询
                // 内集合操作维持拒绝）。
                let saved_from_derived = self.building_from_derived;
                self.building_subquery = true;
                self.building_from_derived = false;
                let right_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.building_subquery = saved_subquery_ctx;
                        self.building_from_derived = saved_from_derived;
                        self.inner_table_names = None;
                        return Err(e);
                    }
                };
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.inner_table_names = None;

                // T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：
                // 同 IN 臂——基座承载左表全宽行。
                let output_columns = self.build_full_output_columns_for_table(table_name);

                let conditions = vec![]; // EXISTS does not need equality conditions

                // Detect correlated parameters
                let correlated_params = self.extract_correlated_params(subquery, &inner_tables)?;

                if *negated {
                    Ok(Some(PhysicalPlan::AntiJoin(AntiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        projection: Vec::new(),
                        correlated_params,
                    })))
                } else {
                    Ok(Some(PhysicalPlan::SemiJoin(SemiJoinNode {
                        left: Box::new(base_plan.clone()),
                        right: Box::new(right_plan),
                        conditions,
                        output_columns,
                        projection: Vec::new(),
                        correlated_params,
                    })))
                }
            }
            _ => Ok(None),
        }
    }

    /// T4（change 2026-10-04-external-review-gaps）：WHERE 比较腿标量子查询
    /// 的绑定与占位改写——深度优先（左→右）遍历表达式树，每个 `Expr::Subquery`
    /// 构建子查询计划（`building_subquery` 上下文 + 相关参数提取，IN/EXISTS
    /// 同款机制）并替换为保留名占位 `Expr::Function`（`__scalar_subquery_<n>`，
    /// 编译期经 `build_expression` 保留名臂映射为 `ParameterExpression`）。
    /// 返回改写后表达式与按占位序排列的绑定清单。遍历变体集与
    /// `contains_scalar_subquery`（query.rs）同面。
    pub(crate) fn bind_where_scalar_subqueries(
        &mut self,
        expr: &Expr,
    ) -> Result<(Expr, Vec<ScalarSubqueryBinding>), PlanError> {
        let mut bindings = Vec::new();
        let rewritten = self.rewrite_scalar_subqueries(expr, &mut bindings)?;
        Ok((rewritten, bindings))
    }

    /// 构建单个标量子查询绑定（子查询计划 + 相关参数 + 占位参数名）。
    fn build_scalar_subquery_binding(
        &mut self,
        subquery: &Query,
        index: usize,
    ) -> Result<ScalarSubqueryBinding, PlanError> {
        let inner_tables = Self::extract_subquery_table_names(subquery);
        self.inner_table_names = Some(inner_tables.clone());
        let saved_subquery_ctx = self.building_subquery;
        // T5：表达式型子查询上下文——FROM 派生标志复位（标量子查询内集合
        // 操作维持拒绝）。
        let saved_from_derived = self.building_from_derived;
        self.building_subquery = true;
        self.building_from_derived = false;
        let plan = match self.build_query(subquery) {
            Ok(p) => p,
            Err(e) => {
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.inner_table_names = None;
                return Err(e);
            }
        };
        self.building_subquery = saved_subquery_ctx;
        self.building_from_derived = saved_from_derived;
        self.inner_table_names = None;
        let correlated_params = self.extract_correlated_params(subquery, &inner_tables)?;
        Ok(ScalarSubqueryBinding {
            param_name: format!("__scalar_subquery_{index}"),
            subquery: Box::new(plan),
            correlated_params,
        })
    }

    /// 表达式树改写体——`Expr::Subquery` 腿替换为占位，其余表达式载体逐变体
    /// 递归重建；遍历集与 `contains_scalar_subquery` 检测一致（InSubquery/
    /// Exists 腿与 Query 体内不进入，维持既有拒绝面）。
    fn rewrite_scalar_subqueries(
        &mut self,
        expr: &Expr,
        bindings: &mut Vec<ScalarSubqueryBinding>,
    ) -> Result<Expr, PlanError> {
        match expr {
            Expr::Subquery(subquery) => {
                let index = bindings.len();
                bindings.push(self.build_scalar_subquery_binding(subquery, index)?);
                Ok(Self::scalar_subquery_placeholder(index))
            }
            Expr::BinaryOp { left, op, right } => Ok(Expr::BinaryOp {
                left: Box::new(self.rewrite_scalar_subqueries(left, bindings)?),
                op: op.clone(),
                right: Box::new(self.rewrite_scalar_subqueries(right, bindings)?),
            }),
            Expr::UnaryOp { op, expr: inner } => Ok(Expr::UnaryOp {
                op: *op,
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
            }),
            Expr::Nested(inner) => Ok(Expr::Nested(Box::new(
                self.rewrite_scalar_subqueries(inner, bindings)?,
            ))),
            Expr::Between {
                expr: inner,
                negated,
                low,
                high,
            } => Ok(Expr::Between {
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                negated: *negated,
                low: Box::new(self.rewrite_scalar_subqueries(low, bindings)?),
                high: Box::new(self.rewrite_scalar_subqueries(high, bindings)?),
            }),
            Expr::InList {
                expr: inner,
                list,
                negated,
            } => Ok(Expr::InList {
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                list: list
                    .iter()
                    .map(|e| self.rewrite_scalar_subqueries(e, bindings))
                    .collect::<Result<Vec<_>, _>>()?,
                negated: *negated,
            }),
            Expr::Like {
                negated,
                expr: inner,
                pattern,
                escape_char,
            } => Ok(Expr::Like {
                negated: *negated,
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                pattern: Box::new(self.rewrite_scalar_subqueries(pattern, bindings)?),
                escape_char: *escape_char,
            }),
            Expr::IsNull(inner) => Ok(Expr::IsNull(Box::new(
                self.rewrite_scalar_subqueries(inner, bindings)?,
            ))),
            Expr::IsNotNull(inner) => Ok(Expr::IsNotNull(Box::new(
                self.rewrite_scalar_subqueries(inner, bindings)?,
            ))),
            Expr::Cast {
                expr: inner,
                data_type,
                format,
            } => Ok(Expr::Cast {
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                data_type: data_type.clone(),
                format: format.clone(),
            }),
            Expr::Case {
                operand,
                conditions,
                results,
                else_result,
            } => Ok(Expr::Case {
                operand: match operand {
                    Some(e) => Some(Box::new(self.rewrite_scalar_subqueries(e, bindings)?)),
                    None => None,
                },
                conditions: conditions
                    .iter()
                    .map(|e| self.rewrite_scalar_subqueries(e, bindings))
                    .collect::<Result<Vec<_>, _>>()?,
                results: results
                    .iter()
                    .map(|e| self.rewrite_scalar_subqueries(e, bindings))
                    .collect::<Result<Vec<_>, _>>()?,
                else_result: match else_result {
                    Some(e) => Some(Box::new(self.rewrite_scalar_subqueries(e, bindings)?)),
                    None => None,
                },
            }),
            Expr::Function(func) => Ok(Expr::Function(sqlparser::ast::Function {
                name: func.name.clone(),
                args: func
                    .args
                    .iter()
                    .map(|arg| match arg {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Ok(FunctionArg::Unnamed(
                            FunctionArgExpr::Expr(self.rewrite_scalar_subqueries(e, bindings)?),
                        )),
                        other => Ok(other.clone()),
                    })
                    .collect::<Result<Vec<_>, PlanError>>()?,
                filter: func.filter.clone(),
                null_treatment: func.null_treatment,
                over: func.over.clone(),
                distinct: func.distinct,
                special: func.special,
                order_by: func.order_by.clone(),
            })),
            Expr::Trim {
                expr: inner,
                trim_where,
                trim_what,
                trim_characters,
            } => Ok(Expr::Trim {
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                trim_where: *trim_where,
                trim_what: match trim_what {
                    Some(e) => Some(Box::new(self.rewrite_scalar_subqueries(e, bindings)?)),
                    None => None,
                },
                trim_characters: match trim_characters {
                    Some(chars) => Some(
                        chars
                            .iter()
                            .map(|e| self.rewrite_scalar_subqueries(e, bindings))
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    None => None,
                },
            }),
            Expr::Ceil { expr: inner, field } => Ok(Expr::Ceil {
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                field: *field,
            }),
            Expr::Floor { expr: inner, field } => Ok(Expr::Floor {
                expr: Box::new(self.rewrite_scalar_subqueries(inner, bindings)?),
                field: *field,
            }),
            // 其余变体原样保留：InSubquery/Exists 腿与 Query 体内不进入
            //（维持既有拒绝面）；值/类型字面量等无子表达式载体不可携带
            // 标量子查询腿。
            other => Ok(other.clone()),
        }
    }

    /// 保留名占位表达式（`__scalar_subquery_<n>` 零参函数形态）——
    /// `build_expression` 按前缀映射为 `ParameterExpression`。
    fn scalar_subquery_placeholder(index: usize) -> Expr {
        Expr::Function(sqlparser::ast::Function {
            name: sqlparser::ast::ObjectName(vec![sqlparser::ast::Ident::new(format!(
                "__scalar_subquery_{index}"
            ))]),
            args: Vec::new(),
            filter: None,
            null_treatment: None,
            over: None,
            distinct: false,
            special: false,
            order_by: Vec::new(),
        })
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
                        names.push(crate::parser::ast::object_name_to_table_name(name));
                    }
                    for join in &twj.joins {
                        if let TableFactor::Table { name, .. } = &join.relation {
                            names.push(crate::parser::ast::object_name_to_table_name(name));
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
            _ => Err(PlanError::UnsupportedExpression("COLUMN REFERENCE".into())),
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
            // MS17-T02/ISS02: a JOIN-shaped IN-subquery plan used to fall into
            // the `_` arm below and misreport "multiple columns" even for a
            // single-column select list — reject with a truthful message.
            PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_) => {
                Err(PlanError::InSubqueryJoinUnsupported)
            }
            // MS21 Iter005 (T5-c/D12)：视图/CTE 展开基座（DerivedScan）——
            // 返回首列名（此前兜底误报多列；CTE 基座同形随之收口，D12）。
            PhysicalPlan::DerivedScan(node) => {
                if node.columns.is_empty() {
                    return Err(PlanError::SubqueryReturnsMultipleColumns);
                }
                Ok(ColumnRef {
                    table: Some(node.alias.clone()),
                    column: node.columns[0].clone(),
                })
            }
            // T8（change 2026-10-13-e2e-verification-and-defect-closure D11）：
            // 裸名主键等值右支是 IndexScan——`columns` 构造期已收窄为
            // SELECT 清单形状（契约同 `get_plan_output_columns` 的
            // IndexScan 臂），直取首列不再误报多列。空清单维持多列错误。
            PhysicalPlan::IndexScan(node) => {
                if node.columns.is_empty() {
                    return Err(PlanError::SubqueryReturnsMultipleColumns);
                }
                Ok(ColumnRef {
                    table: Some(node.table_name.clone()),
                    column: node.columns[0].clone(),
                })
            }
            // T8（同上）：二级索引等值右支是 IndexScanAll——`columns` 恒为
            // 基表全列、输出形状由 `projection` 唯一决定（按
            // `get_plan_output_columns` 同款 `projected_columns` 规则先裁剪
            // 再取首列；投影非空时不得回退基表第一列）。空清单维持多列错误。
            PhysicalPlan::IndexScanAll(node) => {
                let output_columns = projected_columns(&node.columns, &node.projection);
                match output_columns.first() {
                    Some(column) => Ok(ColumnRef {
                        table: Some(node.table_name.clone()),
                        column: column.clone(),
                    }),
                    None => Err(PlanError::SubqueryReturnsMultipleColumns),
                }
            }
            _ => Err(PlanError::SubqueryReturnsMultipleColumns),
        }
    }
}
