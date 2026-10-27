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
                // I080 一期（T02-C1）：作用域快照/恢复——外层累计段 = 界前，
                // 子查询自身 FROM 段退出截断（嵌套构建不外泄）。
                let saved_scopes_len = self.binding_scopes.len();
                let saved_scope_boundary = self.binding_scope_boundary;
                self.binding_scope_boundary = saved_scopes_len;
                self.building_subquery = true;
                self.building_from_derived = false;
                let right_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.building_subquery = saved_subquery_ctx;
                        self.building_from_derived = saved_from_derived;
                        self.inner_table_names = None;
                        self.binding_scope_boundary = saved_scope_boundary;
                        self.restore_binding_scopes(saved_scopes_len);
                        return Err(e);
                    }
                };
                // 001-replan Guidance ④：段名快照在恢复前捕获——界后 = 子查询
                // 自身（内层）段，界前 = 外层累计段；错误路径不收集、不需快照。
                let inner_scope: Vec<String> = self.binding_scopes[saved_scopes_len..]
                    .iter()
                    .map(|e| e.qualifier.clone())
                    .collect();
                let outer_scope: Vec<String> = self.binding_scopes[..saved_scopes_len]
                    .iter()
                    .map(|e| e.qualifier.clone())
                    .collect();
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.inner_table_names = None;
                self.binding_scope_boundary = saved_scope_boundary;
                self.restore_binding_scopes(saved_scopes_len);

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
                let correlated_params = self.extract_correlated_params(
                    subquery,
                    &inner_tables,
                    &inner_scope,
                    &outer_scope,
                )?;

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
                // I080 一期（T02-C1）：作用域快照/恢复——同 IN 臂纪律。
                let saved_scopes_len = self.binding_scopes.len();
                let saved_scope_boundary = self.binding_scope_boundary;
                self.binding_scope_boundary = saved_scopes_len;
                self.building_subquery = true;
                self.building_from_derived = false;
                let right_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.building_subquery = saved_subquery_ctx;
                        self.building_from_derived = saved_from_derived;
                        self.inner_table_names = None;
                        self.binding_scope_boundary = saved_scope_boundary;
                        self.restore_binding_scopes(saved_scopes_len);
                        return Err(e);
                    }
                };
                // 001-replan Guidance ④：段名快照在恢复前捕获——同 IN 臂。
                let inner_scope: Vec<String> = self.binding_scopes[saved_scopes_len..]
                    .iter()
                    .map(|e| e.qualifier.clone())
                    .collect();
                let outer_scope: Vec<String> = self.binding_scopes[..saved_scopes_len]
                    .iter()
                    .map(|e| e.qualifier.clone())
                    .collect();
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.inner_table_names = None;
                self.binding_scope_boundary = saved_scope_boundary;
                self.restore_binding_scopes(saved_scopes_len);

                // T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：
                // 同 IN 臂——基座承载左表全宽行。
                let output_columns = self.build_full_output_columns_for_table(table_name);

                let conditions = vec![]; // EXISTS does not need equality conditions

                // Detect correlated parameters
                let correlated_params = self.extract_correlated_params(
                    subquery,
                    &inner_tables,
                    &inner_scope,
                    &outer_scope,
                )?;

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
        // I080 一期（T02-C1）：作用域快照/恢复——同 IN 臂纪律。
        let saved_scopes_len = self.binding_scopes.len();
        let saved_scope_boundary = self.binding_scope_boundary;
        self.binding_scope_boundary = saved_scopes_len;
        self.building_subquery = true;
        self.building_from_derived = false;
        let plan = match self.build_query(subquery) {
            Ok(p) => p,
            Err(e) => {
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.inner_table_names = None;
                self.binding_scope_boundary = saved_scope_boundary;
                self.restore_binding_scopes(saved_scopes_len);
                return Err(e);
            }
        };
        // 001-replan Guidance ④：段名快照在恢复前捕获——同 IN 臂。
        let inner_scope: Vec<String> = self.binding_scopes[saved_scopes_len..]
            .iter()
            .map(|e| e.qualifier.clone())
            .collect();
        let outer_scope: Vec<String> = self.binding_scopes[..saved_scopes_len]
            .iter()
            .map(|e| e.qualifier.clone())
            .collect();
        self.building_subquery = saved_subquery_ctx;
        self.building_from_derived = saved_from_derived;
        self.inner_table_names = None;
        self.binding_scope_boundary = saved_scope_boundary;
        self.restore_binding_scopes(saved_scopes_len);
        let correlated_params =
            self.extract_correlated_params(subquery, &inner_tables, &inner_scope, &outer_scope)?;
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
    ///
    /// T1（change 2026-10-15-subquery-alias-binding-and-error-prefix）：输出**声明
    /// 限定符**而非仅基表名。修复前丢弃 `alias` 字段使内层别名限定引用
    /// （`FROM c x ... x.age`）被 `qualified_column_expression` 与
    /// `collect_outer_column_refs` 双重误判为相关外层引用（ISS16–ISS18）。
    pub(crate) fn extract_subquery_table_names(subquery: &Query) -> Vec<String> {
        match subquery.body.as_ref() {
            SetExpr::Select(select) => Self::declared_from_qualifiers(&select.from),
            _ => Vec::new(),
        }
    }

    /// FROM 子句全部声明限定符（基表/派生表/JOIN 右臂；别名优先，基名并列
    /// 保留——与 `extract_subquery_table_names` 同一提取规则）。001-replan
    /// 投影臂补强消费：投影位循环先于外层 FROM 构建，外层段按本提取规则
    /// 临时登记进作用域表。
    pub(crate) fn declared_from_qualifiers(from: &[sqlparser::ast::TableWithJoins]) -> Vec<String> {
        from.iter()
            .flat_map(|twj| {
                let mut names = Self::declared_qualifiers(&twj.relation);
                for join in &twj.joins {
                    names.extend(Self::declared_qualifiers(&join.relation));
                }
                names
            })
            .collect()
    }

    /// FROM 项的可解析限定符集——显式别名（`AS` 或隐式）优先，无别名取基表名；
    /// 派生表取别名，无别名取其隐式注册名 `derived`；其余 TableFactor 变体不产出。
    ///
    /// A1（2026-10-15 用户裁定豁免，见 Act Response Blocker Resolution）：别名与基
    /// 表名**并列入集**——别名入集消除幻影相关参数，基表名保留使「别名 + 基名
    /// 限定」子查询形状维持既有可达能力（本引擎顶层对同一写法本就宽松）。
    ///
    /// T03-D4：无别名派生的 `derived` 与该臂实际注册名同源（`query.rs` 派生臂
    /// `unwrap_or_else(|| "derived".to_string())`）——两处字面量互指，避免登记
    /// 段名与实际注册名分叉。
    fn declared_qualifiers(factor: &TableFactor) -> Vec<String> {
        match factor {
            TableFactor::Table { name, alias, .. } => {
                let base = crate::parser::ast::object_name_to_table_name(name);
                match alias.as_ref().map(|a| a.name.value.to_lowercase()) {
                    Some(alias_name) if alias_name != base => vec![alias_name, base],
                    _ => vec![base],
                }
            }
            TableFactor::Derived { alias, .. } => {
                match alias.as_ref().map(|a| a.name.value.to_lowercase()) {
                    Some(alias_name) => vec![alias_name],
                    // T03-D4 无别名派生的隐式注册名 = `derived`（派生臂注册名同
                    // 源，见 `query.rs` 派生臂 `unwrap_or_else(|| "derived"
                    // .to_string())`）——登记段名与实际注册名一致，作用域身份判定
                    // 不再落空。
                    None => vec!["derived".to_string()],
                }
            }
            _ => Vec::new(),
        }
    }

    /// Extract correlated parameters from a subquery by scanning its WHERE clause
    /// for column references that resolve to the outer scope.
    ///
    /// For each outer column reference (e.g., `emp.region` bound to the outer
    /// scope), creates a CorrelatedParam that maps the outer table/column
    /// to the inner column index where the value will be injected at execution time.
    ///
    /// 001-replan：外层判定经 `is_outer_reference` 单源（与分类同公式）；
    /// `inner_scope`/`outer_scope` 为构建退出后、恢复前的段名小写快照
    /// （调用点在 `restore_binding_scopes` 前捕获）——碰撞形状（内层别名
    /// 遮蔽外层基名）由此与分类同侧解析为外层相关参数。
    pub(crate) fn extract_correlated_params(
        &self,
        subquery: &Query,
        inner_tables: &[String],
        inner_scope: &[String],
        outer_scope: &[String],
    ) -> Result<Vec<CorrelatedParam>, PlanError> {
        let where_expr = match subquery.body.as_ref() {
            SetExpr::Select(select) => select.selection.as_ref(),
            _ => return Ok(Vec::new()),
        };
        let Some(where_expr) = where_expr else {
            return Ok(Vec::new());
        };

        let mut params = Vec::new();
        self.collect_outer_column_refs(
            where_expr,
            inner_tables,
            inner_scope,
            outer_scope,
            &mut params,
        )?;
        Ok(params)
    }

    /// Recursively walk an expression tree to find outer column references.
    /// An outer column reference is a CompoundIdentifier (table.column) bound
    /// to the outer scope (001-replan：`is_outer_reference` 单源判定).
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn collect_outer_column_refs(
        &self,
        expr: &Expr,
        inner_tables: &[String],
        inner_scope: &[String],
        outer_scope: &[String],
        params: &mut Vec<CorrelatedParam>,
    ) -> Result<(), PlanError> {
        match expr {
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let table = parts[0].value.to_lowercase();
                let column = parts[1].value.to_lowercase();

                let inner_hit = inner_scope.iter().any(|t| t.eq_ignore_ascii_case(&table));
                let outer_hit = outer_scope.iter().any(|t| t.eq_ignore_ascii_case(&table));
                let in_declared_list = inner_tables.iter().any(|t| t.eq_ignore_ascii_case(&table));
                if super::binding::is_outer_reference(inner_hit, outer_hit, in_declared_list) {
                    // Build qualified param name (e.g. "emp.dept") for matching
                    // ParameterExpression nodes at execution time
                    let param_name = format!("{}.{}", table, column);
                    params.push(CorrelatedParam::new(table, column, param_name));
                }
                Ok(())
            }
            // Recurse into binary operations
            Expr::BinaryOp { left, right, .. } => {
                self.collect_outer_column_refs(
                    left,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                self.collect_outer_column_refs(
                    right,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                Ok(())
            }
            // Recurse into unary operations
            Expr::UnaryOp { expr, .. } => {
                self.collect_outer_column_refs(
                    expr,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                Ok(())
            }
            // Recurse into nested expressions
            Expr::Nested(expr) => {
                self.collect_outer_column_refs(
                    expr,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                Ok(())
            }
            // Recurse into BETWEEN
            Expr::Between {
                expr, low, high, ..
            } => {
                self.collect_outer_column_refs(
                    expr,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                self.collect_outer_column_refs(
                    low,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                self.collect_outer_column_refs(
                    high,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
                Ok(())
            }
            // Recurse into IN list
            Expr::InList { expr, .. } => {
                self.collect_outer_column_refs(
                    expr,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
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
                self.collect_outer_column_refs(
                    expr,
                    inner_tables,
                    inner_scope,
                    outer_scope,
                    params,
                )?;
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
                    self.collect_outer_column_refs(
                        op,
                        inner_tables,
                        inner_scope,
                        outer_scope,
                        params,
                    )?;
                }
                for cond in conditions {
                    self.collect_outer_column_refs(
                        cond,
                        inner_tables,
                        inner_scope,
                        outer_scope,
                        params,
                    )?;
                }
                for res in results {
                    self.collect_outer_column_refs(
                        res,
                        inner_tables,
                        inner_scope,
                        outer_scope,
                        params,
                    )?;
                }
                if let Some(else_expr) = else_result {
                    self.collect_outer_column_refs(
                        else_expr,
                        inner_tables,
                        inner_scope,
                        outer_scope,
                        params,
                    )?;
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
            PhysicalPlan::Join(_)
            | PhysicalPlan::SortMergeJoin(_)
            | PhysicalPlan::NestedLoopJoin(_) => Err(PlanError::InSubqueryJoinUnsupported),
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
            PhysicalPlan::IndexRangeScan(node) => {
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
