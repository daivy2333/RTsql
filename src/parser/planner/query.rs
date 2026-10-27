//! PlanBuilder — SELECT / FROM / JOIN / projection / PK-equality
//!
//! MS07-T03: split from single-file `planner.rs` (T2 migration). All method
//! bodies are moved verbatim; only `impl PlanBuilder` block boundary and
//! per-module imports are introduced.

use super::aggregate::{
    contains_aggregate_expr, extract_aggregate_func, is_aggregate_expr, register_aggregate_calls,
    AggRowExprMode,
};
use super::binding::scope_window_layout;
use super::ddl_dml::is_pure_equi_join_on;
use super::expression::expr_to_column_name;
use super::{CteDef, PlanBuilder};
use crate::executor::{
    ColumnExpression, ColumnRef, DataScanNode, DistinctNode, ExpressionRef, FilterNode,
    IndexScanAllNode, IndexScanNode, OrderByColumn, OutputColumn, PhysicalPlan, ProjectionItem,
    ProjectionNode, ScanNode, SetOpKind, SetOperationNode, SortNode,
};
use crate::parser::ast::*;
use crate::parser::error::PlanError;
use crate::parser::placeholder::is_placeholder_value;
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
    /// MS25 T8（design D9）：JOIN 基座上已解析的输入行索引（布局感知解析的
    /// 结果）。`Some` 时编译直取该索引——`column_indices` 的按名折叠不参与
    /// 决策，故同名列不静默末位胜出；单表面为 `None`（既有名字查索引语义）。
    input_index: Option<usize>,
}

/// MS15-Rest (I034): describe a scan node's real output shape. The scan
/// executors trim rows by `projection` after predicate evaluation, so the
/// column metadata must be trimmed the same way (same mapping as the
/// Filter/Sort arms in `get_plan_output_columns`). Empty = identity.
pub(crate) fn projected_columns(columns: &[String], projection: &[usize]) -> Vec<String> {
    if projection.is_empty() {
        columns.to_vec()
    } else {
        projection.iter().map(|&i| columns[i].clone()).collect()
    }
}

/// MS32 Iter002（design D3）：LIMIT 直接约束排序时把驻留上限写进 SortNode。
/// 可设界形状仅两种——`Sort`，或恰好一层 `Projection` 且其输入是 `Sort`
/// （deferred_wrap 表达式项形状，值写进内层 SortNode）。`Distinct(...)` 与
/// 其他形状不设界——DISTINCT、聚合、集合运算保持全量排序语义。
fn mark_sort_bounded(plan: &mut PhysicalPlan, bounded: usize) {
    match plan {
        PhysicalPlan::Sort(node) => node.bounded = Some(bounded),
        PhysicalPlan::Projection(node) => {
            if let PhysicalPlan::Sort(sort) = node.input.as_mut() {
                sort.bounded = Some(bounded);
            }
        }
        _ => {}
    }
}

impl PlanBuilder {
    /// T03-D3：派生表 / CTE 基座的列源——direct 形态聚合的注册列按 SELECT 清单
    /// 逐位取名。
    ///
    /// 聚合节点的 `output_columns` 是「分组键**规范名** ++ 聚合显示名」：分组
    /// 键位在 SELECT 清单带别名时（`SELECT tag AS n, COUNT(*) AS c … GROUP BY
    /// tag`），别名不在该公式内 → 基座只登记 `tag`，外层引用 `d.n` 计划期
    /// 拒绝（`Column not found: n`）。
    ///
    /// 这里按 SELECT 清单逐位取名：键位取别名（无别名取列名），聚合位维持
    /// `output_columns` 的显示名。位序与聚合输出行一致——direct 形态下 SELECT
    /// 的键段序 = 分组键序、聚合段序 = 聚合注册序。
    ///
    /// 仅当解包 Limit/Sort/Having 后顶层为 Aggregate（direct 形态）时改写；
    /// 包装形态（顶层为 Projection，其列已是 SELECT 清单名）与其余形态维持
    /// `get_plan_output_columns`。
    pub(crate) fn derived_output_columns(&self, query: &Query, plan: &PhysicalPlan) -> Vec<String> {
        let base = self.get_plan_output_columns(plan);
        let mut node = plan;
        loop {
            node = match node {
                PhysicalPlan::Limit(n) => &n.input,
                PhysicalPlan::Sort(n) => &n.input,
                PhysicalPlan::Having(n) => &n.input,
                _ => break,
            };
        }
        let PhysicalPlan::Aggregate(agg) = node else {
            return base;
        };
        let Ok(select) = extract_select_body(query) else {
            return base;
        };
        // 键位数 = 分组键数；聚合位显示名取自聚合节点的输出列（同序）。
        let key_count = agg.group_by.len();
        let mut names: Vec<String> = Vec::with_capacity(select.projection.len());
        let mut agg_index = 0usize;
        for item in &select.projection {
            let expr = match item {
                sqlparser::ast::SelectItem::UnnamedExpr(e) => e,
                sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => expr,
                sqlparser::ast::SelectItem::Wildcard(_) => return base,
                sqlparser::ast::SelectItem::QualifiedWildcard(_, _) => return base,
            };
            if is_aggregate_expr(expr) {
                let Some(display) = agg.output_columns.get(key_count + agg_index) else {
                    return base;
                };
                names.push(display.clone());
                agg_index += 1;
            } else {
                // 键位：别名优先，否则列名（direct 形态下键项恒为纯列）。
                let name = match item {
                    sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } => alias.value.clone(),
                    _ => match expr_to_column_name(expr) {
                        Ok(col) => col,
                        Err(_) => return base,
                    },
                };
                names.push(name);
            }
        }
        // 位数与聚合输出列数不一致 → 维持既有列源（不得产生错位映射）。
        if names.len() != agg.output_columns.len() {
            return base;
        }
        names
    }

    /// 从 PhysicalPlan 中提取输出列名（用于派生表的列注册）
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn get_plan_output_columns(&self, plan: &PhysicalPlan) -> Vec<String> {
        match plan {
            PhysicalPlan::Scan(node) => projected_columns(&node.columns, &node.projection),
            PhysicalPlan::DataScan(node) => projected_columns(&node.columns, &node.projection),
            PhysicalPlan::DerivedScan(node) => projected_columns(&node.columns, &node.projection),
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
            PhysicalPlan::ScalarSubqueryFilter(node) => {
                // T4（change 2026-10-04-external-review-gaps）：`columns` 即
                // 裁剪后实际输出形状（IndexScan「已收窄」契约同型——构造期
                // 已按基表 schema × projection 收窄，此处直出不再二次裁剪）。
                node.columns.clone()
            }
            PhysicalPlan::Join(node) | PhysicalPlan::SortMergeJoin(node) => {
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
            PhysicalPlan::SemiJoin(node) => {
                // T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：
                // `output_columns` 恒为基表全列；节点 `projection` 非空时行已
                // 按其收窄（无 ORDER BY 的 SELECT 清单裁剪），列名随投影裁剪
                // ——与 Scan/Sort 臂同款，表头与行形状一致（I034 纪律）。
                if node.projection.is_empty() {
                    node.output_columns
                        .iter()
                        .map(|c| c.column.clone())
                        .collect()
                } else {
                    node.projection
                        .iter()
                        .map(|&i| node.output_columns[i].column.clone())
                        .collect()
                }
            }
            PhysicalPlan::AntiJoin(node) => {
                // T5：同 SemiJoin 臂。
                if node.projection.is_empty() {
                    node.output_columns
                        .iter()
                        .map(|c| c.column.clone())
                        .collect()
                } else {
                    node.projection
                        .iter()
                        .map(|&i| node.output_columns[i].column.clone())
                        .collect()
                }
            }
            PhysicalPlan::SubqueryEval(node) => {
                // MS17-T02/ISS03: 执行器在 result_column_index 插入标量值
                //（越界时 push，见 executor/subquery_eval.rs）——表头镜像该
                // 语义插入标量列名，保证列数与行宽一致。
                // T4：相关参数补充列以尾部附加形式在输入行中，本节点按
                // output_width 截断后再插列——表头同步截断。
                let mut columns = self.get_plan_output_columns(&node.input);
                if node.output_width > 0 {
                    columns.truncate(node.output_width);
                }
                let idx = node.result_column_index.min(columns.len());
                columns.insert(idx, node.output_column.clone());
                columns
            }
            PhysicalPlan::Projection(node) => node.columns.clone(),
            // MS25 T2: Distinct 不改列——透传输入的输出形状
            PhysicalPlan::Distinct(node) => self.get_plan_output_columns(&node.input),
            // MS25 T3: 复合输出列名 = 左侧列名（planner 已校验两侧列数相等）
            PhysicalPlan::SetOperation(node) => node.columns.clone(),
            // MS25 T6: EXPLAIN 输出恒单列 `plan`（表头与行宽一致，I034 纪律）
            PhysicalPlan::Explain(_) => vec!["plan".to_string()],
            // MS20 T4: CTAS 输出列名 = 物化表列名（= SELECT 输出表头，构造时
            // 已由 build_create_table_as_select 记录）。
            PhysicalPlan::CreateTableAsSelect(node) => node.columns.clone(),
            // MS13 T9: SingleRow 无输出列（no-FORM 输入节点，表头由其上的
            // ProjectionNode 承载）。
            PhysicalPlan::SingleRow => Vec::new(),
            PhysicalPlan::Insert(_)
            | PhysicalPlan::Upsert(_)
            | PhysicalPlan::InsertSelect(_)
            | PhysicalPlan::Update(_)
            | PhysicalPlan::Delete(_) => Vec::new(),
            PhysicalPlan::CreateTable(_)
            | PhysicalPlan::CreateIndex(_)
            | PhysicalPlan::DropIndex(_)
            | PhysicalPlan::AlterTable(_)
            | PhysicalPlan::Truncate(_)
            | PhysicalPlan::CreateView(_)
            | PhysicalPlan::DropView(_)
            | PhysicalPlan::DropTable(_) => Vec::new(),
        }
    }

    /// MS13 T8（design D12）：解析单个 GROUP BY 项——列名（既有语义优先）
    /// → SELECT 别名（大小写不敏感）→ SELECT 项表达式文本（双侧
    /// `Expr::to_string()` 归一比较）→ 1-based 位置引用；全部不匹配 →
    /// `NonAggregatedColumn` 点名。命中聚合项的别名/文本/位置键显式拒绝
    ///（键不得为聚合）。
    ///
    /// `join_layout`（MS25 T8 design D9）——`Some` 为 JOIN 基座的行布局：列名键
    /// 在布局上精确解析（限定名按表定位、裸名唯一命中，歧义点名为
    /// `AmbiguousColumn`），命中的输入行索引随 `GroupKey.input_index` 下传，
    /// 编译阶段不再经按名折叠的 `column_indices`；`None` 为单表面既有语义。
    pub(crate) fn resolve_group_by_item(
        &self,
        expr: &Expr,
        projection: &[sqlparser::ast::SelectItem],
        column_indices: &HashMap<String, usize>,
        join_layout: Option<&[(String, Vec<String>)]>,
    ) -> Result<GroupKey, PlanError> {
        match expr {
            Expr::Identifier(ident) => {
                let name = ident.value.clone();
                // 列名（既有语义优先）
                if let Some(layout) = join_layout {
                    // MS25 T8：布局上唯一命中才认定为列名键；多命中点名为
                    // AmbiguousColumn；无命中交回下方别名/位置既有语义。
                    if let Some(idx) = resolve_layout_index(self, layout, None, &name)? {
                        return Ok(GroupKey {
                            name: name.clone(),
                            column: Some(name),
                            expr_item: None,
                            input_index: Some(idx),
                        });
                    }
                } else if column_indices.contains_key(&name.to_lowercase()) {
                    return Ok(GroupKey {
                        name: name.clone(),
                        column: Some(name),
                        expr_item: None,
                        input_index: None,
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
                                input_index: None,
                            });
                        }
                    }
                }
                Err(PlanError::NonAggregatedColumn(name))
            }
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                let name = parts[1].value.clone();
                // Cycle 001（replan）design D4：两段限定符先在 FROM 作用域内
                // 点名（镜像 ORDER BY 路径）——归一后不可达即 42P01 携带限定符
                // 原文，单表与 JOIN 两基座同语义，不再经末段静默命中或误报列
                // 缺失；in-scope 限定符 + 列缺失维持既有错误面。
                if !self.qualifier_in_scope(&parts[0].value) {
                    return Err(PlanError::TableNotFound(parts[0].value.clone()));
                }
                if let Some(layout) = join_layout {
                    // MS25 T8：限定名在布局上精确解析；in-scope 限定符下列
                    // 不在该表沿用既有 NonAggregatedColumn 点名。
                    if let Some(idx) =
                        resolve_layout_index(self, layout, Some(&parts[0].value), &name)?
                    {
                        return Ok(GroupKey {
                            name: name.clone(),
                            column: Some(name),
                            expr_item: None,
                            input_index: Some(idx),
                        });
                    }
                    Err(PlanError::NonAggregatedColumn(name))
                } else if column_indices.contains_key(&name.to_lowercase()) {
                    Ok(GroupKey {
                        name: name.clone(),
                        column: Some(name),
                        expr_item: None,
                        input_index: None,
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
                            input_index: None,
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
                            input_index: None,
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
                            input_index: None,
                        });
                    }
                }
                Err(PlanError::NonAggregatedColumn(text))
            }
        }
    }

    /// 构建 FROM + JOIN 链计划（支持列投影）
    ///
    /// `carry_all_columns`（MS25 T1 design D2）——查询含 WHERE 时置位：JOIN
    /// 节点的 `output_columns` 承载**全部组合列**（跳过 SELECT 清单过滤），
    /// 由其上的 Filter 节点承担 SELECT 收窄。谓词在补 NULL 后的组合行上求值，
    /// ORDER BY 键与分组键也得以在组合形状上解析。未置位时（无 WHERE）维持
    /// 既有过滤语义，INNER 路径逐字节不变。
    ///
    /// `pure_column_list`（MS25 T8 design D9）——SELECT 清单为纯列引用时，
    /// 过滤式解析前逐项严格校验（未知列 `ColumnNotFound`、裸名多命中
    /// `AmbiguousColumn`）；`false`（表达式/聚合/子查询项）维持既有承载语义。
    ///
    /// design D4：返回 `(计划, 组合列声明限定符)`——第二元组与最终 JOIN 节点的
    /// `output_columns` 同序同长（无 JOIN 臂时为空），供调用方在组合行布局与
    /// 投影索引解析上按声明限定符匹配（真实表名无法区分自连接的两段同表）。
    pub(crate) fn build_from_clause_with_projection(
        &mut self,
        from: &[sqlparser::ast::TableWithJoins],
        qualified_columns: &[(Option<String>, String)],
        carry_all_columns: bool,
        pure_column_list: bool,
    ) -> Result<(PhysicalPlan, Vec<String>), PlanError> {
        use crate::parser::ast::extract_join_table_name;
        use sqlparser::ast::{JoinConstraint, JoinOperator};

        if from.is_empty() {
            return Err(PlanError::MissingField("FROM clause".into()));
        }

        // I080 一期（T02-C2）：布局作用域窗口入口——本 FROM 构造自此起的绑定
        // 作用域段即本 FROM 的各臂段（声明序）。`entry` 之前的段不在窗口内：
        // 外层作用域段、CTE 定义内部残留段、投影臂临时登记段。
        let layout_scope_entry = self.binding_scopes.len();

        // 基础表 — 支持 TableFactor::Table（普通表）和 TableFactor::Derived（派生表）
        //
        // design D4：第二元组是本 FROM 臂的**真实表键**；**声明限定符**（声明
        // 别名即别名，否则 effective 表名）由绑定作用域表登记——JOIN 组合形状
        // 按它分段，自连接同表两别名各占一段。
        let (base_plan, base_table) = match &from[0].relation {
            TableFactor::Table { name, alias, .. } => {
                let table_name = object_name_to_table_name(name);
                // MS25 T5: CTE 查找钩子置于 `validate_table` 之前（CTE 名遮蔽
                // 同名基表）；未命中时既有基表路径逐字节不变（含既有忽略
                // 表别名的行为）。
                // MS21 Iter005 (T5-c)：视图查找钩子置于 CTE 未命中之后、基表
                // `validate_table` 之前（CTE > 视图 > 基表优先级不变量）；
                // 视图未命中时既有基表路径逐字节不变。
                match self.resolve_cte_reference(&table_name, alias.as_ref())? {
                    // design D4：CTE / 视图引用点的别名**就是** effective 键
                    // （`resolve_cte_reference` / `resolve_view_reference` 已按
                    // 别名注册 `self.tables`），无别名条目可登——登记恒等映射
                    // 不改变任何查找结果。
                    Some((plan, effective)) => (plan, effective.clone()),
                    None => match self.resolve_view_reference(&table_name, alias.as_ref())? {
                        Some((plan, effective)) => (plan, effective.clone()),
                        None => {
                            self.validate_table(&table_name)?;
                            // design D4：基表别名登记——`FROM people p` 的
                            // `p` 映射到 `self.tables` 的真实键 `people`。
                            let qualifier = match alias {
                                Some(a) => {
                                    self.register_table_alias(&a.name.value, &table_name);
                                    a.name.value.to_lowercase()
                                }
                                None => {
                                    // Iteration 002 T7（R9）：两段附加 FROM 项无显式
                                    // 别名 → 尾段隐式关联名（附加标记为条件）。
                                    self.register_implicit_attached_short_name(&table_name);
                                    table_name.clone()
                                }
                            };
                            let base_columns =
                                self.tables.get(&table_name).cloned().unwrap_or_default();
                            // I080 一期（T02-C1）：基表段按声明限定符登记绑定
                            // 作用域（类型/可空自既有加性通道取——键经小写归一；
                            // 未注册即 None，诚实记录）。
                            let (scope_types, scope_not_null) =
                                self.table_binding_meta(&table_name);
                            self.register_binding_scope(
                                qualifier.clone(),
                                base_columns.clone(),
                                scope_types,
                                scope_not_null,
                            );
                            let plan = PhysicalPlan::Scan(ScanNode {
                                table_name: table_name.clone(),
                                columns: base_columns,
                                projection: Vec::new(),
                            });
                            (plan, table_name)
                        }
                    },
                }
            }
            TableFactor::Derived {
                subquery, alias, ..
            } => {
                // T5（change 2026-10-04-external-review-gaps）：FROM 派生臂是
                // 唯一置位 `building_from_derived` 的入口——集合操作守卫据此
                // 放行派生表源；save/restore 与 `building_subquery` 严格配对。
                let saved_subquery_ctx = self.building_subquery;
                let saved_from_derived = self.building_from_derived;
                // I080 一期（T02-C1）：作用域快照/恢复——派生定义内层 FROM 段
                // 在退出时截断（定义内部形状不外泄到外层分类面），界同步恢复。
                let saved_scopes_len = self.binding_scopes.len();
                let saved_scope_boundary = self.binding_scope_boundary;
                self.binding_scope_boundary = saved_scopes_len;
                self.building_subquery = true;
                self.building_from_derived = true;
                let subquery_plan = match self.build_query(subquery) {
                    Ok(p) => p,
                    Err(e) => {
                        self.building_subquery = saved_subquery_ctx;
                        self.building_from_derived = saved_from_derived;
                        self.binding_scope_boundary = saved_scope_boundary;
                        self.restore_binding_scopes(saved_scopes_len);
                        return Err(e);
                    }
                };
                self.building_subquery = saved_subquery_ctx;
                self.building_from_derived = saved_from_derived;
                self.binding_scope_boundary = saved_scope_boundary;
                self.restore_binding_scopes(saved_scopes_len);
                let alias_name = alias
                    .as_ref()
                    .map(|a| a.name.value.to_lowercase())
                    // T03-D4：无别名派生的隐式注册名 = `derived`——作用域登记侧
                    // 的同一字面量见 `subquery.rs::declared_qualifiers`（两处互指，
                    // 来源单源）。
                    .unwrap_or_else(|| "derived".to_string());
                // 提取子查询输出列名
                // T03-D3: 聚合直出形态的键位按 SELECT 清单别名取名（聚合节点的
                // 输出列只带分组键规范名）。
                let columns = self.derived_output_columns(subquery, &subquery_plan);
                // 注册派生表列信息（供后续 WHERE/ORDER BY 引用）
                self.register_table(&alias_name, columns.clone(), "");
                // I080 一期（T02-C1）：派生段按 effective 名登记（类型/可空
                // 计划期未知 = None）。
                self.register_binding_scope(alias_name.clone(), columns.clone(), None, None);
                let plan = PhysicalPlan::DerivedScan(crate::executor::DerivedScanNode {
                    subquery: Box::new(subquery_plan),
                    alias: alias_name.clone(),
                    columns,
                    // MS26 T8：外层纯列清单的裁剪由下方投影解析阶段回填
                    // （构造点先落恒等，与三构造点同型）。
                    projection: Vec::new(),
                });
                // design D4：派生表的别名即注册名，无需别名条目。
                (plan, alias_name.clone())
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
        // design D4：与 `current_tables` 同序的**声明限定符**（每臂一个）改由
        // 绑定作用域表承载（I080 一期 T02-C2）——本处的平行记账移除，段键与
        // 声明序的唯一事实源 = 作用域表窗口。
        // design D4：末次迭代的 `output_qualifiers`（与最终节点 `output_columns`
        // 同序同长）；无 JOIN 臂时保持空——调用方只在组合形状存在时消费。
        let mut combined_qualifiers: Vec<String> = Vec::new();
        // T11（change 2026-10-04-external-review-gaps D11）：链装配三修复——
        // ① 严格校验与 SELECT 收窄（kept）只在**末步**执行（非末步强制全
        // 组合承载，链前缀列对后续 ON/谓词/投影恒可达）；② 链前缀列
        // `column_index` 重定基为**左输入行绝对偏移**（累计宽度，链长 1 时
        // 数值与 within-table 相同，两表形态逐字节不变）；③ 条件键限定解析
        // 在执行器查表侧（pipeline 注册 `(真实表, 列)` 键——见 join.rs）。
        let total_joins = from[0].joins.len();

        for (step, join) in from[0].joins.iter().enumerate() {
            let is_last = step + 1 == total_joins;
            // MS25 T1（design D1）：JOIN 形态分类。`Inner(On)` 与
            // `Inner(Using/Natural/None)` 维持既有两臂（编译 / `MissingOnClause`
            // 拒绝）；`LeftOuter(On)` 经 NLJ 承载，`FullOuter(On)` 经 NLJ 全侧
            // 补齐，`RightOuter(On)` 交换左右输入后按 LEFT 承载（左侧 =
            // 原右表，未匹配的原左行按标准 SQL 语义丢弃）；其余
            // `JoinOperator`（含外连接的 Using/Natural/None、CROSS APPLY 等）
            // 维持 `UnsupportedJoinType` 点名拒绝。
            let (on_clause, outer) = match &join.join_operator {
                JoinOperator::Inner(JoinConstraint::On(expr)) => (Some(expr), OuterJoinKind::None),
                JoinOperator::Inner(_) => (None, OuterJoinKind::None),
                JoinOperator::LeftOuter(JoinConstraint::On(expr)) => {
                    (Some(expr), OuterJoinKind::Left)
                }
                JoinOperator::FullOuter(JoinConstraint::On(expr)) => {
                    (Some(expr), OuterJoinKind::Full)
                }
                JoinOperator::RightOuter(JoinConstraint::On(expr)) => {
                    (Some(expr), OuterJoinKind::Right)
                }
                _ => return Err(PlanError::UnsupportedJoinType),
            };

            // 解析右表
            let right_table = extract_join_table_name(&join.relation)?;
            // design D4：右臂声明限定符（别名优先，否则真实表名）——组合形状
            // 的分段键。CTE / 视图臂的 effective 键即其别名（无别名条目可登）。
            let right_qualifier = table_factor_alias(&join.relation)
                .map(|a| a.name.value.to_lowercase())
                .unwrap_or_else(|| right_table.clone());
            // MS25 T5: JOIN 右臂同款 CTE 钩子（置于 `validate_table` 之前）。
            // MS21 Iter005 (T5-c)：JOIN 右臂同款视图钩子（CTE 未命中后、
            // 基表前）。
            let (right_table, right_plan) = match self
                .resolve_cte_reference(&right_table, table_factor_alias(&join.relation))?
            {
                Some((plan, effective)) => (effective, plan),
                None => match self
                    .resolve_view_reference(&right_table, table_factor_alias(&join.relation))?
                {
                    Some((plan, effective)) => (effective, plan),
                    None => {
                        self.validate_table(&right_table)?;
                        // design D4：JOIN 右臂基表别名登记，与 FROM 基表臂
                        // 同款（CTE / 视图右臂同上，别名即 effective 键）。
                        match table_factor_alias(&join.relation) {
                            Some(a) => self.register_table_alias(&a.name.value, &right_table),
                            None => {
                                // Iteration 002 T7（R9）：右臂两段附加项无显式
                                // 别名 → 尾段隐式关联名（附加标记为条件）。
                                self.register_implicit_attached_short_name(&right_table);
                            }
                        }
                        let columns = self.tables.get(&right_table).cloned().unwrap_or_default();
                        // I080 一期（T02-C1）：JOIN 右臂基表段按声明限定符登记
                        // 绑定作用域（类型/可空同 FROM 基表臂）。
                        let (scope_types, scope_not_null) = self.table_binding_meta(&right_table);
                        self.register_binding_scope(
                            right_qualifier.clone(),
                            columns.clone(),
                            scope_types,
                            scope_not_null,
                        );
                        let plan = PhysicalPlan::Scan(ScanNode {
                            table_name: right_table.clone(),
                            columns: columns.clone(),
                            projection: Vec::new(),
                        });
                        (right_table, plan)
                    }
                },
            };

            // 解析 ON 条件
            let on_clause = on_clause.ok_or(PlanError::MissingOnClause)?;

            // 构建输出列（根据 qualified_columns 过滤）—— Hash 与 NLJ 两路由共享，
            // SELECT * 与列过滤行为对两种 join 节点一致。D2：WHERE 存在时承载
            // 全部组合列（SELECT 收窄交由其上的 Filter / Sort 承担）。
            //
            // design D4：`all_qualifiers` 与 `all_columns` 同序同长，记录每列
            // 所属 FROM 臂的声明限定符。限定列按它匹配（自连接同表两别名各占
            // 一段，真实表名无法区分归属）；`OutputColumn` 的真实表名字段不变。
            //
            // T11：`column_index` 为该列在**本步输出行**中的绝对偏移——左链
            // 各臂列按累计宽度重定基（左输入行 = 前序步全组合输出），右臂列
            // = 左链总宽 + 表内位置。链长 1（两表）时左链恰一表，数值与
            // within-table 相同，行为逐字节不变。
            let mut left_width = 0usize;
            let mut all_columns: Vec<OutputColumn> = Vec::new();
            for t in &current_tables {
                let columns = self
                    .tables
                    .get(t)
                    .expect("validated table must exist in metadata");
                let base = left_width;
                left_width += columns.len();
                for (idx, col) in columns.iter().enumerate() {
                    all_columns.push(OutputColumn {
                        table: Some(t.clone()),
                        column: col.clone(),
                        table_alias: t.clone(),
                        column_index: base + idx,
                    });
                }
            }
            for (idx, col) in self
                .tables
                .get(&right_table)
                .expect("validated right_table must exist")
                .iter()
                .enumerate()
            {
                all_columns.push(OutputColumn {
                    table: Some(right_table.clone()),
                    column: col.clone(),
                    table_alias: right_table.clone(),
                    // 右臂列保持 within-table 语义——执行器按右输入行寻址
                    //（左链列的绝对偏移重定基只针对左输入行）。
                    column_index: idx,
                });
            }
            // I080 一期（T02-C2）：本步的作用域窗口——末段为本步右臂段（JOIN
            // 装配点：右臂段在 ON 编译前登记），其余段为左链各臂段（声明序）。
            // 段键与列清单的唯一事实源 = 绑定作用域表（`current_qualifiers`
            // 平行记账移除）。窗口在此一次性物化为布局同形的段列表——与作用域
            // 表的后续增删解耦（子查询臂构建期的作用域截断不影响已物化副本）。
            let scope_window = scope_window_layout(self.binding_scope_window(layout_scope_entry));
            let (right_segment, left_segments) = scope_window
                .split_last()
                .expect("JOIN scope window must contain the right arm segment");
            let all_qualifiers: Vec<String> = left_segments
                .iter()
                .flat_map(|(qualifier, columns)| {
                    std::iter::repeat_n(qualifier.clone(), columns.len())
                })
                .chain(std::iter::repeat_n(
                    right_segment.0.clone(),
                    right_segment.1.len(),
                ))
                .collect();
            // design D9：输出列输入侧标志（与 `all_columns`/`all_qualifiers`
            // 同序同长，原始表序语境——左链各臂列 Left、右臂列 Right）。执行器
            // 据此取行，自连接两侧真实名相同不再使归属失效。
            let all_sides: Vec<crate::executor::JoinSide> = current_tables
                .iter()
                .flat_map(|t| {
                    let width = self
                        .tables
                        .get(t)
                        .expect("validated table must exist in metadata")
                        .len();
                    std::iter::repeat_n(crate::executor::JoinSide::Left, width)
                })
                .chain(std::iter::repeat_n(
                    crate::executor::JoinSide::Right,
                    self.tables
                        .get(&right_table)
                        .expect("validated right_table must exist")
                        .len(),
                ))
                .collect();

            let select_star = qualified_columns.iter().any(|(_, c)| c == "*");
            // MS25 T8（design D9）：过滤式解析前的严格校验——纯列清单里既不
            // 存在也不唯一存在的列必须计划期点名（既有「逐列过滤」对未知列
            // 静默产出零宽行、对裸名多命中静默产出重复列行）。校验在组合
            // 列形状上按限定名精确 / 裸名唯一命中判定，与 `build_expression`
            // 的布局解析臂同款错误变体。
            // design D4：限定名按声明限定符匹配——限定符不属本 FROM 作用域
            // 交由链末的 `validate_projection_qualifiers` 点名（42P01 携带限定符），
            // 此处只判列存在性（未知列 42703 / 裸名多命中 42702）。
            // T11：严格校验只在**末步**执行——中间步的当前组合形状不含后续
            // 表列（第三表列在第一步即误点名 `ColumnNotFound` 的链式失败
            // 根因），末步作用域 = 全链组合形状，真未知列仍在此点名。
            if is_last && !carry_all_columns && !select_star && pure_column_list {
                for (qual_table, qual_col) in qualified_columns {
                    // design D4：不在 FROM 作用域内的限定符交由链末的
                    // `validate_projection_qualifiers` 点名（42P01 携带限定符），
                    // 此处只判列存在性，避免把「关系不存在」误报为列缺失。
                    if qual_table
                        .as_deref()
                        .is_some_and(|table| !self.qualifier_in_scope(table))
                    {
                        continue;
                    }
                    let hits = all_columns
                        .iter()
                        .zip(&all_qualifiers)
                        .filter(|(col, qualifier)| {
                            col.column.to_lowercase() == qual_col.to_lowercase()
                                && match qual_table {
                                    // T7（R9）：原文相等或归一化命中（隐式短名）。
                                    Some(table) => {
                                        self.qualifier_matches_declared(table, qualifier)
                                    }
                                    None => true,
                                }
                        })
                        .count();
                    if hits == 0 {
                        return Err(PlanError::ColumnNotFound(qual_col.clone()));
                    }
                    if hits > 1 && qual_table.is_none() {
                        return Err(PlanError::AmbiguousColumn(qual_col.clone()));
                    }
                }
            }
            let (output_columns, output_qualifiers, output_sides) = if carry_all_columns
                || select_star
                // T11：非末步强制全组合承载——中间步收窄会使左输入行形状 ≠
                // 下一步的全宽假设（链上 ON 条件列缺失 → 执行器键查 panic /
                // 错位），末步按查询标志现行逻辑。
                || !is_last
            {
                // SELECT *（或 D2 的组合行全列承载）：输出所有列
                (all_columns, all_qualifiers, all_sides)
            } else {
                // SELECT col1, col2... 或 SELECT t.col1, t.col2...
                //
                // T10（change 2026-10-04-external-review-gaps D10）：kept 按
                // SELECT 清单序重建（对齐 Scan 基座 `resolve_projection_indices`
                // 的按清单序语义）——逐 SELECT 项定位命中列（限定名按声明限定
                // 符精确匹配、裸名任意命中），重复项重复产出。纯列清单的 0
                // 命中/裸名多命中已由上方严格校验点名；非纯清单维持既有静默
                // 跳过语义（`*` 项同样跳过）。三清单（列/限定符/侧标志）同步
                // 产出，行组装与归属不变。
                let mut kept: Vec<OutputColumn> = Vec::new();
                let mut kept_qualifiers: Vec<String> = Vec::new();
                let mut kept_sides: Vec<crate::executor::JoinSide> = Vec::new();
                for (qual_table, qual_col) in qualified_columns {
                    if qual_col == "*" {
                        continue;
                    }
                    for ((col, qualifier), side) in all_columns
                        .iter()
                        .zip(all_qualifiers.iter())
                        .zip(all_sides.iter())
                    {
                        let matched = col.column.to_lowercase() == qual_col.to_lowercase()
                            && match qual_table {
                                // Qualified column: qualifier.col（T7：原文相等
                                // 或归一化命中——隐式短名 `u` ↔ 声明 `o.u`）。
                                Some(table) => self.qualifier_matches_declared(table, qualifier),
                                None => true,
                            };
                        if matched {
                            kept.push(col.clone());
                            kept_qualifiers.push(qualifier.clone());
                            kept_sides.push(*side);
                        }
                    }
                }
                (kept, kept_qualifiers, kept_sides)
            };

            // MS09-T02 (I015): 计划期启发式路由（delta spec R4）。结构探测的
            // 真集恰为 `extract_join_conditions` 的接受面（resolve_column_ref
            // 只接受 Identifier / 2 段 CompoundIdentifier）。纯等值内连接里，
            // 两侧基表扫描且每条键都是写出的整数 PRIMARY KEY 时发排序合并，其余仍发哈希
            // 连接。非等值、字面量和表达式腿路由 NLJ。MS25 T1：外连接三态一律
            // 经 NLJ。
            if matches!(outer, OuterJoinKind::None) && is_pure_equi_join_on(on_clause) {
                let conditions = self.extract_join_conditions(
                    &current_tables,
                    &scope_window,
                    &right_table,
                    on_clause,
                )?;

                let join = crate::executor::JoinNode {
                    left: Box::new(current_plan),
                    right: Box::new(right_plan),
                    conditions,
                    output_columns,
                    output_sides,
                };
                // 两侧都是基表扫描，且每条等值键都是该表写出的整数 PRIMARY KEY 时走排序合并。
                // 任一条件不成立仍发哈希连接。
                current_plan = if self.integer_pk_base_scan_join(&join) {
                    PhysicalPlan::SortMergeJoin(join)
                } else {
                    PhysicalPlan::Join(join)
                };
            } else {
                // 非纯等值 ON → NLJ：组合行布局（左表偏移 0..n、右表
                // n..n+m）上编译完整 ON 谓词。布局覆盖 save/restore 严格配对
                // （`inner_table_names` 同型先例）；谓词 `column_index` 为
                // 组合行绝对索引。RIGHT 交换重写时布局按交换后的输入序
                // （原右表在前）——ON 谓词在交换后的组合行上求值。
                // I080 一期（T02-C2）：布局段自作用域窗口派生——段键与列清单
                // 同源（窗口除末段 = 左链各臂段，末段 = 本步右臂段）。RIGHT
                // 交换变体在派生后重排（原右臂段在前），ON 谓词在交换后的
                // 组合行上求值——同既有语义。
                let layout: Vec<(String, Vec<String>)> = if outer == OuterJoinKind::Right {
                    // design D4：段键为声明限定符（自连接两别名各占一段）
                    let mut swapped = vec![right_segment.clone()];
                    swapped.extend(left_segments.iter().cloned());
                    swapped
                } else {
                    let mut ordered: Vec<(String, Vec<String>)> = left_segments.to_vec();
                    ordered.push(right_segment.clone());
                    ordered
                };
                self.join_column_layout = Some(layout);
                // MS25 T1: `ON <布尔字面量>`（如 `ON true`）是与行无关的常量
                // 谓词——既有 `build_where` 无字面量臂，此处显式常量编译；
                // 其余 ON 形态经既有编译路径。
                let compiled: Result<crate::executor::PredicateRef, PlanError> = match on_clause {
                    Expr::Value(sqlparser::ast::Value::Boolean(value)) => {
                        Ok(Arc::new(crate::executor::ConstantPredicate {
                            value: *value,
                        }))
                    }
                    other => self.build_where(&current_tables[0], other),
                };
                let predicate = match compiled {
                    Ok(p) => p,
                    Err(e) => {
                        self.join_column_layout = None;
                        return Err(e);
                    }
                };
                self.join_column_layout = None;

                // RIGHT = 交换左右输入的 LEFT：保存侧（原右表）成为左输入，
                // 未匹配的原左行丢弃；FULL 不做交换（两侧都补齐）。
                // `output_columns` 按原表序保持不变；输出归属由 `output_sides`
                // 侧标志承载（design D9）——RIGHT 交换后执行器输入侧对调，
                // 侧标志随输入交换反转（原右臂列改读左输入行，原左链列反之）。
                let output_sides = if outer == OuterJoinKind::Right {
                    output_sides
                        .into_iter()
                        .map(|side| match side {
                            crate::executor::JoinSide::Left => crate::executor::JoinSide::Right,
                            crate::executor::JoinSide::Right => crate::executor::JoinSide::Left,
                        })
                        .collect()
                } else {
                    output_sides
                };
                let join_type = match outer {
                    OuterJoinKind::None => crate::executor::NestedLoopJoinType::Inner,
                    OuterJoinKind::Left | OuterJoinKind::Right => {
                        crate::executor::NestedLoopJoinType::Left
                    }
                    OuterJoinKind::Full => crate::executor::NestedLoopJoinType::Full,
                };
                current_plan = if outer == OuterJoinKind::Right {
                    PhysicalPlan::NestedLoopJoin(crate::executor::NestedLoopJoinNode {
                        left: Box::new(right_plan),
                        right: Box::new(current_plan),
                        predicate,
                        output_columns,
                        output_sides,
                        join_type,
                    })
                } else {
                    PhysicalPlan::NestedLoopJoin(crate::executor::NestedLoopJoinNode {
                        left: Box::new(current_plan),
                        right: Box::new(right_plan),
                        predicate,
                        output_columns,
                        output_sides,
                        join_type,
                    })
                };
            }

            current_tables.push(right_table);
            combined_qualifiers = output_qualifiers;
        }

        // design D4：纯列清单的限定符在计划期点名——归一后无对应注册表即
        // `TableNotFound`（42P01），不静默丢弃限定符按裸名解析（单表基座的
        // 「静默成功」面与本地三段 `db.table.col` 的分裂面同源于此）。置于
        // JOIN 链装配之后——各臂别名此时才全部登记，自连接两别名均在作用域内。
        if pure_column_list {
            self.validate_projection_qualifiers(qualified_columns)?;
        }

        // design D4：末次迭代的 `output_qualifiers` 与最终节点的 `output_columns`
        // 同序同长，供调用方的组合行布局与投影索引解析按声明限定符匹配。
        Ok((current_plan, combined_qualifiers))
    }

    /// design D4：限定符在本语句 FROM 作用域内可达？——别名归一后命中
    /// `self.tables`，或限定符本身是注册键（真实表名 / CTE·视图 effective 名 /
    /// 派生表别名 / attach 复合键）。
    fn qualifier_in_scope(&self, qualifier: &str) -> bool {
        let key = self.resolve_table_qualifier(qualifier).unwrap_or(qualifier);
        self.tables.contains_key(key)
    }

    /// T3（change 2026-10-04-external-review-gaps D3）：JOIN 基座 ORDER BY 限定
    /// 键的组合行布局解析——限定符命中声明段键时产出指向组合行绝对索引的
    /// `ColumnExpression`（SortExecutor decorate 通路消费）。
    ///
    /// 匹配序：① 书写限定符与声明段键直接匹配（别名原文相等 / 隐式短名归一，
    /// `qualifier_matches_declared` 同款）；② 全表名书写（FROM 臂声明了别名）：
    /// 段键归一（别名 → 真实键）后与书写限定符归一结果比较——单表面全表名
    /// 恒可达（`qualifier_in_scope` 真实名分支），JOIN 基座同规则；多段归一
    /// 同键（自连接 + 全表名）首个命中，与既有裸名首匹配语义同源。两步都未
    /// 命中返回 `None` 交回调用方回落既有裸名键路径。命中段缺列 →
    /// `ColumnNotFound` 点名（限定符精确归属，镜像 `qualified_column_expression`
    /// 布局臂）。
    fn resolve_order_by_layout_key(
        &self,
        layout: &[(String, Vec<String>)],
        written: &str,
        column: &str,
    ) -> Result<Option<ExpressionRef>, PlanError> {
        let col = column.to_lowercase();
        let mut offset = 0usize;
        for (declared, seg) in layout {
            if self.qualifier_matches_declared(written, declared) {
                return match seg.iter().position(|c| c.to_lowercase() == col) {
                    Some(pos) => Ok(Some(Arc::new(ColumnExpression {
                        column_name: col,
                        column_index: offset + pos,
                    }))),
                    None => Err(PlanError::ColumnNotFound(column.to_string())),
                };
            }
            offset += seg.len();
        }
        let written_key = self.resolve_table_qualifier(written).unwrap_or(written);
        let mut offset = 0usize;
        for (declared, seg) in layout {
            let declared_key = self.resolve_table_qualifier(declared).unwrap_or(declared);
            if declared_key.eq_ignore_ascii_case(written_key) {
                return match seg.iter().position(|c| c.to_lowercase() == col) {
                    Some(pos) => Ok(Some(Arc::new(ColumnExpression {
                        column_name: col,
                        column_index: offset + pos,
                    }))),
                    None => Err(PlanError::ColumnNotFound(column.to_string())),
                };
            }
            offset += seg.len();
        }
        Ok(None)
    }

    /// design D4：纯列清单的限定符计划期点名——归一（别名 → 真实键）后无对应
    /// 注册表即 `TableNotFound`（42P01 = FROM 中无该关系），错误携带限定符原文。
    ///
    /// 覆盖单表基座此前「静默丢弃限定符按裸名解析」的路径（`SELECT x.name FROM
    /// people` 与本地三段 `db.table.col` 静默成功即由此而来）。非纯列清单不在此
    /// 校验——表达式 / 聚合 / 子查询项经 `build_expression` 与子查询通道各自
    /// 点名既有错误面。`*`（含 `t.*`）不参与。
    fn validate_projection_qualifiers(
        &self,
        qualified_columns: &[(Option<String>, String)],
    ) -> Result<(), PlanError> {
        for (qual_table, qual_col) in qualified_columns {
            if qual_col == "*" {
                continue;
            }
            let Some(qualifier) = qual_table else {
                continue;
            };
            if !self.qualifier_in_scope(qualifier) {
                return Err(PlanError::TableNotFound(qualifier.clone()));
            }
        }
        Ok(())
    }

    /// Cycle 001（replan）design D4：无聚合查询的 GROUP BY 项限定符可达性校验。
    /// 有聚合路径由 `resolve_group_by_item` 两段臂点名；无聚合时 GROUP BY 不经
    /// 分组解析（既有静默语义保持），仅对两段限定名做同款作用域检查——归一后
    /// 不可达即 42P01 携限定符原文，单表与 JOIN 同语义。
    fn validate_group_by_qualifiers(
        &self,
        group_by: &sqlparser::ast::GroupByExpr,
    ) -> Result<(), PlanError> {
        if let sqlparser::ast::GroupByExpr::Expressions(exprs) = group_by {
            for e in exprs {
                if let Expr::CompoundIdentifier(parts) = e {
                    if parts.len() == 2 && !self.qualifier_in_scope(&parts[0].value) {
                        return Err(PlanError::TableNotFound(parts[0].value.clone()));
                    }
                }
            }
        }
        Ok(())
    }

    /// MS25 T3（design D4）：复合查询构建——`UNION [ALL]` / `INTERSECT` /
    /// `EXCEPT` 的分派入口。侧臂经合成 `Query` 复用 `build_query`（JOIN /
    /// 外连接 / 聚合 / 表达式项 / WHERE 全量复用），复合节点列名取左侧列名，
    /// 语句尾 ORDER BY / LIMIT 作用于复合输出。
    fn build_set_operation(&mut self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        // 分支上的 `DISTINCT` 量化词（含嵌套分支）归属歧义，点名拒绝
        // （design D3）——复合语句的去重由各操作自身的语义决定（`UNION`
        // 去重 / `UNION ALL` 保留）。
        if set_operand_has_distinct(&query.body) {
            return Err(PlanError::ParseError(
                "DISTINCT cannot be combined with set operations".to_string(),
            ));
        }
        // 子查询上下文内的复合：范围外（子查询计划形状由 SemiJoin / SubqueryEval
        // / DerivedScan 机制消费），点名拒绝而非静默。
        // T5（change 2026-10-04-external-review-gaps）：FROM 派生上下文放行
        // （`building_from_derived` 置位）——评测点名形态；视图展开与表达式
        // 子查询（IN/EXISTS/标量）上下文维持拒绝，错误文案逐字节不变。
        if self.building_subquery && !self.building_from_derived {
            return Err(PlanError::ParseError(
                "Set operations are not supported inside subqueries".to_string(),
            ));
        }
        let (op, quantifier, left, right) = match query.body.as_ref() {
            sqlparser::ast::SetExpr::SetOperation {
                op,
                set_quantifier,
                left,
                right,
            } => (*op, set_quantifier, left, right),
            _ => return Err(PlanError::UnsupportedStatement("SET OPERATION".into())),
        };

        let node =
            PhysicalPlan::SetOperation(self.build_set_operation_node(op, quantifier, left, right)?);
        let columns = self.get_plan_output_columns(&node);

        // 复合语句尾 ORDER BY（键 ∈ 复合输出列名；裸名限制与既有 ORDER BY 一致）
        let mut plan_with_order = if !query.order_by.is_empty() {
            let order_by: Vec<OrderByColumn> = query
                .order_by
                .iter()
                .map(|o| {
                    let (qualifier, column) = extract_column_name(&o.expr)?;
                    // design D4：复合查询无 FROM 作用域，任何限定键都不可达——
                    // 点名限定符（42P01），维持裸名键既有面。
                    if let Some(qualifier) = qualifier {
                        return Err(PlanError::TableNotFound(qualifier));
                    }
                    let asc = o.asc.unwrap_or(true);
                    Ok(OrderByColumn {
                        column,
                        asc,
                        expr: None,
                    })
                })
                .collect::<Result<Vec<_>, PlanError>>()?;
            for ob in &order_by {
                if !columns.iter().any(|c| c.eq_ignore_ascii_case(&ob.column)) {
                    return Err(PlanError::ColumnNotFound(ob.column.clone()));
                }
            }
            PhysicalPlan::Sort(SortNode {
                input: Box::new(node),
                order_by,
                table_name: "set_operation".to_string(),
                columns: columns.clone(),
                // The compound output already is the SELECT shape — the Sort
                // node is the identity here (no projection owner below it).
                projection: Vec::new(),
                // 有界与否由复合语句尾 LIMIT 包装点按需写入（design D3）。
                bounded: None,
            })
        } else {
            node
        };

        // 复合语句尾 LIMIT（作用于复合输出；无扫描封顶可下推——输入是复合节点）
        if let Some(limit_expr) = &query.limit {
            let limit = parse_limit_value(limit_expr)?;
            let offset = query
                .offset
                .as_ref()
                .map(|o| parse_offset_value(&o.value))
                .transpose()?
                .unwrap_or(0);
            // MS32 Iter002（design D3）：此处 `plan_with_order` 为 identity
            // 投影的裸 `Sort`（无 ORDER BY 时是复合节点，helper 不动作）。
            mark_sort_bounded(&mut plan_with_order, offset.saturating_add(limit));
            return Ok(PhysicalPlan::Limit(crate::executor::LimitNode {
                input: Box::new(plan_with_order),
                limit,
                offset,
            }));
        }
        Ok(plan_with_order)
    }

    /// MS25 T3：单个集合操作节点（左右分支各自递归；sqlparser 的嵌套形状
    /// 天然承载 INTERSECT 更深 = 更高优先级，`a UNION b INTERSECT c` 的
    /// 右侧即嵌套的 INTERSECT 节点）。
    fn build_set_operation_node(
        &mut self,
        op: sqlparser::ast::SetOperator,
        quantifier: &sqlparser::ast::SetQuantifier,
        left: &sqlparser::ast::SetExpr,
        right: &sqlparser::ast::SetExpr,
    ) -> Result<SetOperationNode, PlanError> {
        use sqlparser::ast::{SetOperator, SetQuantifier};

        let (kind, all) = match (op, quantifier) {
            (SetOperator::Union, SetQuantifier::All) => (SetOpKind::Union, true),
            (SetOperator::Union, SetQuantifier::None | SetQuantifier::Distinct) => {
                (SetOpKind::Union, false)
            }
            (SetOperator::Intersect, SetQuantifier::None | SetQuantifier::Distinct) => {
                (SetOpKind::Intersect, false)
            }
            (SetOperator::Except, SetQuantifier::None | SetQuantifier::Distinct) => {
                (SetOpKind::Except, false)
            }
            (SetOperator::Intersect | SetOperator::Except, SetQuantifier::All) => {
                return Err(PlanError::ParseError(format!(
                    "{} ALL is not supported",
                    if matches!(op, SetOperator::Intersect) {
                        "INTERSECT"
                    } else {
                        "EXCEPT"
                    }
                )))
            }
            (
                _,
                SetQuantifier::ByName | SetQuantifier::AllByName | SetQuantifier::DistinctByName,
            ) => {
                return Err(PlanError::ParseError(
                    "BY NAME set quantifier is not supported".to_string(),
                ))
            }
        };

        let left_plan = self.build_set_operand(left)?;
        let right_plan = self.build_set_operand(right)?;
        let left_columns = self.get_plan_output_columns(&left_plan);
        let right_columns = self.get_plan_output_columns(&right_plan);
        if left_columns.len() != right_columns.len() {
            return Err(PlanError::ParseError(format!(
                "Set operation branches must have the same column count ({} vs {})",
                left_columns.len(),
                right_columns.len()
            )));
        }

        Ok(SetOperationNode {
            left: Box::new(left_plan),
            right: Box::new(right_plan),
            op: kind,
            all,
            columns: left_columns,
        })
    }

    /// MS25 T3：复合分支构建——`Select` 分支合成零 ORDER BY / LIMIT 的
    /// `Query` 复用既有 `build_query`；嵌套复合递归；括号化分支点名拒绝。
    fn build_set_operand(
        &mut self,
        expr: &sqlparser::ast::SetExpr,
    ) -> Result<PhysicalPlan, PlanError> {
        match expr {
            sqlparser::ast::SetExpr::Select(select) => {
                let arm_query = Query {
                    with: None,
                    body: Box::new(sqlparser::ast::SetExpr::Select(select.clone())),
                    order_by: Vec::new(),
                    limit: None,
                    limit_by: Vec::new(),
                    offset: None,
                    fetch: None,
                    locks: Vec::new(),
                    for_clause: None,
                };
                self.build_query(&arm_query)
            }
            sqlparser::ast::SetExpr::SetOperation {
                op,
                set_quantifier,
                left,
                right,
            } => {
                let node = self.build_set_operation_node(*op, set_quantifier, left, right)?;
                Ok(PhysicalPlan::SetOperation(node))
            }
            sqlparser::ast::SetExpr::Query(_) => Err(PlanError::ParseError(
                "Parenthesized set operation operands are not supported".to_string(),
            )),
            _ => Err(PlanError::UnsupportedStatement("SET OPERATION".into())),
        }
    }

    /// MS25 T5（design D5）：消费语句级 `WITH` 子句——逐定义以**顶层上下文**
    /// 构建计划并注册到 `cte_plans`（后定义可见先定义 = 前向引用）。定义计划
    /// 克隆 `with: None` 的合成 Query，`building_subquery` 不变（保持 false），
    /// 因此定义内 ORDER BY / LIMIT / 集合操作 / 表达式项 / DISTINCT 经既有装配
    /// 全量复用；`pending_cte_names` 在定义构建期间置位（自引用 / 成环拒绝），
    /// push/pop 严格配对。
    ///
    /// 拒绝面（fail-fast，不静默降级）：子查询内 WITH、`WITH RECURSIVE`（含内容
    /// 非递归的形态）、嵌套定义内 WITH、列别名列表数量与定义输出列数不符。
    fn consume_with(&mut self, with: &sqlparser::ast::With) -> Result<(), PlanError> {
        if self.building_subquery {
            return Err(PlanError::ParseError(
                "WITH inside a subquery is not supported".to_string(),
            ));
        }
        if with.recursive {
            return Err(PlanError::UnsupportedStatement(
                "WITH RECURSIVE (recursive common table expressions)".to_string(),
            ));
        }
        for cte in &with.cte_tables {
            if cte.query.with.is_some() {
                return Err(PlanError::ParseError(
                    "nested WITH inside a CTE definition is not supported".to_string(),
                ));
            }
            let name = cte.alias.name.value.to_lowercase();
            self.pending_cte_names.push(name.clone());
            let mut definition = (*cte.query).clone();
            definition.with = None;
            let plan = self.build_query(&definition);
            self.pending_cte_names.pop();
            let plan = plan?;

            // T03-D3: 聚合直出形态的键位按 SELECT 清单别名取名（与派生表基座
            // 同一列源——CTE 与派生表共用 DerivedScan 装配路径）。
            let mut columns = self.derived_output_columns(&definition, &plan);
            if !cte.alias.columns.is_empty() {
                let aliases: Vec<String> = cte
                    .alias
                    .columns
                    .iter()
                    .map(|ident| ident.value.to_lowercase())
                    .collect();
                if aliases.len() != columns.len() {
                    return Err(PlanError::InvalidQuery(format!(
                        "CTE '{}' declares {} column aliases but its query produces {} columns",
                        name,
                        aliases.len(),
                        columns.len()
                    )));
                }
                columns = aliases;
            }
            self.cte_plans.push(CteDef {
                name,
                columns,
                plan,
            });
        }
        Ok(())
    }

    /// MS25 T5：FROM 基座 / JOIN 右臂的 CTE 查找钩子（置于 `validate_table`
    /// 之前，故 CTE 名遮蔽同名基表）。命中则克隆定义计划为 `DerivedScanNode`
    /// 并按引用处别名（或 CTE 名）注册列信息，供 WHERE / ORDER BY / JOIN 条件
    /// 引用；未命中返回 `None`（既有基表路径逐字节不变）。
    fn resolve_cte_reference(
        &mut self,
        name: &str,
        alias: Option<&sqlparser::ast::TableAlias>,
    ) -> Result<Option<(PhysicalPlan, String)>, PlanError> {
        let normalized = name.to_lowercase();
        if self.pending_cte_names.iter().any(|n| n == &normalized) {
            return Err(PlanError::InvalidQuery(format!(
                "CTE '{}' cannot reference itself",
                name
            )));
        }
        let definition = self
            .cte_plans
            .iter()
            .find(|def| def.name == normalized)
            .map(|def| (def.columns.clone(), def.plan.clone()));
        let Some((columns, plan)) = definition else {
            return Ok(None);
        };
        if let Some(alias) = alias {
            if !alias.columns.is_empty() {
                return Err(PlanError::InvalidQuery(format!(
                    "column alias list on a reference to CTE '{}' is not supported; declare it as WITH {}(...) AS (...)",
                    name, name
                )));
            }
        }
        let effective = alias
            .map(|a| a.name.value.to_lowercase())
            .unwrap_or(normalized);
        self.register_table(&effective, columns.clone(), "");
        // I080 一期（T02-C1）：CTE 引用段按 effective 名登记（类型/可空 = None）。
        self.register_binding_scope(effective.clone(), columns.clone(), None, None);
        Ok(Some((
            PhysicalPlan::DerivedScan(crate::executor::DerivedScanNode {
                subquery: Box::new(plan),
                alias: effective.clone(),
                columns,
                projection: Vec::new(), // MS26 T8：同 FROM 派生表构造点
            }),
            effective,
        )))
    }

    /// MS21 Iter005 (T5-c/D12)：视图引用展开——命中 builder 视图通道时把
    /// 定义文本重解析并以派生表同型上下文（`building_subquery` save/restore，
    /// 内联 `FROM (<定义>) AS <视图名>` 逐字节等价的唯一裁判）`build_query`
    /// 递归展开（定义内 ORDER BY/LIMIT/聚合/JOIN/嵌套视图全可达），包
    /// `DerivedScanNode`（alias=引用别名或视图名、columns=持久化列名清单
    /// ——显式清单已覆盖）。未命中返回 None（基表路径逐字节不变）。CTE
    /// 查找先于此（同名 CTE 遮蔽视图，优先级不变量）；引用别名列清单与
    /// 派生表同型忽略（零特殊化）。
    fn resolve_view_reference(
        &mut self,
        name: &str,
        alias: Option<&sqlparser::ast::TableAlias>,
    ) -> Result<Option<(PhysicalPlan, String)>, PlanError> {
        let normalized = name.to_lowercase();
        let Some(def) = self.views.get(&normalized) else {
            return Ok(None);
        };
        let sql = def.sql.clone();
        let columns = def.columns.clone();
        // 定义文本重解析（`Query::to_string()` → parse 往返稳定）；防御臂：
        // 持久化文本非查询（结构性不可达）
        let statements = crate::parser::ast::parse_sql(&sql)?;
        let sqlparser::ast::Statement::Query(inner_query) = &statements[0] else {
            return Err(PlanError::InvalidQuery(format!(
                "view '{normalized}' definition did not re-parse as a query"
            )));
        };
        let saved_subquery_ctx = self.building_subquery;
        // change 2026-10-13-e2e-verification-and-defect-closure T4：视图展开
        // 与 FROM 派生表同型——置位 `building_from_derived`，内层别名/表达式列
        // 与 DISTINCT 经分类门全量放行（R4 三通道一致）；IN/EXISTS/标量
        // 入口的复位不受影响，save/restore 严格配对。
        let saved_from_derived = self.building_from_derived;
        // I080 一期（T02-C1）：作用域快照/恢复——视图定义内层 FROM 段在
        // 退出时截断（定义内部形状不外泄到外层分类面），界同步恢复。
        let saved_scopes_len = self.binding_scopes.len();
        let saved_scope_boundary = self.binding_scope_boundary;
        self.binding_scope_boundary = saved_scopes_len;
        self.building_subquery = true;
        self.building_from_derived = true;
        let inner_result = self.build_query(inner_query);
        self.building_subquery = saved_subquery_ctx;
        self.building_from_derived = saved_from_derived;
        self.binding_scope_boundary = saved_scope_boundary;
        self.restore_binding_scopes(saved_scopes_len);
        let inner_plan = inner_result?;
        let effective = alias
            .map(|a| a.name.value.to_lowercase())
            .unwrap_or(normalized);
        self.register_table(&effective, columns.clone(), "");
        // I080 一期（T02-C1）：视图引用段按 effective 名登记（类型/可空 = None）。
        self.register_binding_scope(effective.clone(), columns.clone(), None, None);
        Ok(Some((
            PhysicalPlan::DerivedScan(crate::executor::DerivedScanNode {
                subquery: Box::new(inner_plan),
                alias: effective.clone(),
                columns,
                projection: Vec::new(), // MS26 T8：同 FROM 派生表构造点
            }),
            effective,
        )))
    }

    /// Build PhysicalPlan for SELECT query
    pub(crate) fn build_query(&mut self, query: &Query) -> Result<PhysicalPlan, PlanError> {
        // MS11-T01 Iter001: 子查询上下文（WHERE IN/EXISTS、标量子查询、派生表）
        // 抑制 SELECT 表达式项路由——子查询计划形状是 SemiJoin/SubqueryEval/
        // DerivedScan 机制的消费面，保持既有行为（R6 零回归）。
        let building_subquery = self.building_subquery;
        // change 2026-10-13-e2e-verification-and-defect-closure T4：分类门与
        // DISTINCT 门只在「真子查询（IN/EXISTS/标量）且非派生」时抑制——
        // FROM 派生表/视图内层与顶层同权（R4），CTE 通道对照零变化。
        let inner_suppressed = building_subquery && !self.building_from_derived;
        // === MS25 T5: WITH 消费（design D5）===
        // 位于 body 分派之前：语句级 `WITH` 先于侧臂构建消费（定义内集合操作
        // 可达，两臂引用同一注册表）。子查询上下文 / `RECURSIVE` / 嵌套定义内
        // WITH 点名拒绝；定义以顶层上下文构建（`building_subquery` 不变）。
        if let Some(with) = &query.with {
            self.consume_with(with)?;
        }
        // === MS25 T3: body 分派 ===
        // `SetExpr::Select` 走既有单查询装配（下方全部逻辑逐字节不变）；
        // `SetExpr::SetOperation` 走复合构建（design D4）；括号化侧臂点名拒绝；
        // 其余形态（`Values` 等）维持 `extract_select_body` 的既有
        // `UnsupportedStatement("SET OPERATION")` 拒绝。
        match query.body.as_ref() {
            sqlparser::ast::SetExpr::Select(_) => {}
            sqlparser::ast::SetExpr::SetOperation { .. } => {
                return self.build_set_operation(query);
            }
            sqlparser::ast::SetExpr::Query(_) => {
                return Err(PlanError::ParseError(
                    "Parenthesized set operation operands are not supported".to_string(),
                ))
            }
            _ => {}
        }
        // Extract Select body
        let select = extract_select_body(query)?;

        // === MS25 T2: DISTINCT 量化词 ===
        // 修复静默面——`query.distinct` 此前在 planner 全无消费，`SELECT
        // DISTINCT` 照常返回重复行。`SELECT ALL` 解析为 `None`，与无量化词
        // 形态同路径（显式无操作，SQLite 对齐）。`DISTINCT ON(...)` 点名拒绝。
        //
        // 仅顶层与派生上下文消费（R4）：FROM 派生表/视图内层 DISTINCT 真实
        // 去重，与 CTE 通道一致；真子查询（IN/EXISTS/标量）内 DISTINCT 维持
        // 零消费（SemiJoin/SubqueryEval/ScalarSubqueryFilter 按既有节点形状
        // 消费的契约 Forbidden 项不变）。
        let distinct_due = match &select.distinct {
            None => false,
            Some(sqlparser::ast::Distinct::Distinct) => !inner_suppressed,
            Some(sqlparser::ast::Distinct::On(_)) => {
                return Err(PlanError::ParseError(
                    "DISTINCT ON(...) is not supported".to_string(),
                ))
            }
        };

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
            let saved_subquery_ctx = self.building_subquery;
            // T5：SELECT 投影位子查询是表达式型上下文——FROM 派生标志复位。
            let saved_from_derived = self.building_from_derived;
            // I080 一期（T02-C1）：作用域快照/恢复——与 IN/EXISTS/标量绑定臂
            // 同款纪律（外层累计段 = 界前，子查询自身 FROM 段退出截断）。
            let saved_scopes_len = self.binding_scopes.len();
            // 001-replan 投影臂补强：投影位循环先于外层 FROM 构建（本函数
            // 后文才调 `build_from_clause_with_projection`），外层段尚不在作用
            // 域表——按外层 FROM 声明限定符临时登记外层段（仅限定符参与
            // 命中判定，列清单留空；001-replan 判定需要外层段存在性），与
            // 内层段在退出时一并截断，界恢复进臂前原值。
            let saved_outer_boundary = self.binding_scope_boundary;
            for qualifier in Self::declared_from_qualifiers(&select.from) {
                self.register_binding_scope(qualifier, Vec::new(), None, None);
            }
            let saved_scope_boundary = self.binding_scopes.len();
            self.binding_scope_boundary = saved_scope_boundary;
            self.building_subquery = true;
            self.building_from_derived = false;
            let subquery_plan = match self.build_query(expr) {
                Ok(p) => p,
                Err(e) => {
                    self.building_subquery = saved_subquery_ctx;
                    self.building_from_derived = saved_from_derived;
                    self.inner_table_names = None;
                    self.binding_scope_boundary = saved_outer_boundary;
                    self.restore_binding_scopes(saved_scopes_len);
                    return Err(e);
                }
            };
            // 001-replan Guidance ⑤：收集自构建前移至恢复前——快照可得性
            // 要求。已接受边界：构建失败与预收集拒绝（多级相关）双无效输入
            // 时构建错误先行。
            let inner_scope: Vec<String> = self.binding_scopes[saved_scope_boundary..]
                .iter()
                .map(|e| e.qualifier.clone())
                .collect();
            let outer_scope: Vec<String> = self.binding_scopes
                [saved_scopes_len..saved_scope_boundary]
                .iter()
                .map(|e| e.qualifier.clone())
                .collect();
            self.building_subquery = saved_subquery_ctx;
            self.building_from_derived = saved_from_derived;
            self.inner_table_names = None;
            self.binding_scope_boundary = saved_outer_boundary;
            self.restore_binding_scopes(saved_scopes_len);
            let correlated_params =
                self.extract_correlated_params(expr, &inner_tables, &inner_scope, &outer_scope)?;
            subquery_evals.push((idx, subquery_plan, col_name, correlated_params));
        }

        // MS25 T2: 标量子查询项 × DISTINCT 点名拒绝——SubqueryEval 在 Limit
        // 之上插列，与 DISTINCT 的「输出形状已就绪」位置冲突（诚实拒绝优于
        // 错误语义）。
        if distinct_due && !subquery_evals.is_empty() {
            return Err(PlanError::ParseError(
                "DISTINCT cannot be combined with scalar subquery items".to_string(),
            ));
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

        // Build FROM + JOIN chain (with projection columns for filtering).
        // MS25 T1 (design D2): a WHERE clause makes the JOIN nodes carry the
        // full combination columns — the Filter above owns the SELECT trim.
        // MS25 T8 (design D9): an aggregate projection carries the full
        // combination columns the same way, so aggregate parameters and
        // qualified GROUP BY keys resolve against the whole shape.
        let subquery_indices: Vec<usize> = subquery_evals.iter().map(|(idx, ..)| *idx).collect();
        let projection_has_aggregates = {
            let items: &[sqlparser::ast::SelectItem] = if filtered_projection.is_empty() {
                &select.projection
            } else {
                &filtered_projection
            };
            items.iter().any(|item| match item {
                sqlparser::ast::SelectItem::UnnamedExpr(e)
                | sqlparser::ast::SelectItem::ExprWithAlias { expr: e, .. } => is_aggregate_expr(e),
                _ => false,
            })
        };
        let (base_plan, combined_qualifiers) = self.build_from_clause_with_projection(
            &select.from,
            &qualified_columns,
            // T3（change 2026-10-04-external-review-gaps D3）：ORDER BY 存在时
            // JOIN 节点同样承载全组合列（镜像 WHERE 规则）——排序键在组合形状
            // 上解析，SELECT 裁剪归其上的 Sort（sort_projection）。
            select.selection.is_some() || projection_has_aggregates || !query.order_by.is_empty(),
            select_list_is_pure_column_list(select, &subquery_indices),
        )?;

        // MS25 T8（design D9）：JOIN 基座的行布局——`base_plan` 的 `output_columns`
        // 就是 Sort/Aggregate 输入行的真实列序（无 WHERE 且无聚合 = 过滤后布局；
        // 有 WHERE 或有聚合 = 全组合布局）。名字解析（投影项、ORDER BY 别名键、
        // 分组键、聚合参数）与组合形状严格同基座。
        // design D4：段键取 `build_from_clause_with_projection` 返回的声明
        // 限定符（与 `output_columns` 同序同长），自连接同表两别名各占一段。
        let join_layout: Option<Vec<(String, Vec<String>)>> = join_combined_columns(&base_plan)
            .map(|combined| join_row_layout(combined, &combined_qualifiers));

        // Extract table name from base plan for single-table queries (for WHERE/ORDER BY processing)
        let table_name = match &base_plan {
            PhysicalPlan::Scan(scan_node) => scan_node.table_name.clone(),
            PhysicalPlan::DerivedScan(derived_node) => derived_node.alias.clone(),
            PhysicalPlan::Join(_)
            | PhysicalPlan::SortMergeJoin(_)
            | PhysicalPlan::NestedLoopJoin(_) => {
                "join_result".to_string() // 虚拟表名用于 JOIN 结果
            }
            _ => "unknown".to_string(),
        };

        // === Aggregate function detection ===
        // Check if SELECT projection contains aggregate functions.
        // Runs before WHERE handling: `has_aggregates` gates the projection
        // resolution below (aggregate plans keep the full-schema input).
        let mut aggregates = Vec::new();
        // MS25 T8（design D9）：与 `aggregates` 同序的参数列表达式（`COUNT(*)`
        // 无参 → `None`）。`AggregateFunc` 只保留列名（限定名在此丢失），布局
        // 感知解析需要原始表达式才能按 (表, 列) 精确定位。
        let mut aggregate_args: Vec<Option<Expr>> = Vec::new();
        let mut non_agg_columns = Vec::new();
        // change 2026-10-03-practical-core-gaps T4/T6：聚合输出行显示名（与
        // `aggregates` 注册序平行）——SELECT 聚合项 = 别名优先否则
        // result_column_name；表达式嵌入与 ORDER BY 键引入的额外聚合 =
        // result_column_name。聚合节点输出列名 = 分组键名 ++ 本清单，与执行器
        // 行布局（group keys ++ aggregates 注册序）严格对齐。
        let mut aggregate_display_names: Vec<String> = Vec::new();
        // T4：聚合嵌入表达式项与纯常量项的 SELECT 项索引——在聚合输出行上
        // 求值（不经分组键绑定校验，wrap 阶段按本清单编译）。
        let mut agg_row_expr_items: Vec<usize> = Vec::new();
        // MS11-T01 Iter001: SELECT 表达式项（非普通列引用的项）。此处只标记
        // 不报错——路由在下文统一裁决：非聚合 → 顶层 Projection；聚合 →
        // 保持聚合路径报错（design D4：检测循环裁决顺序不变）。
        let mut has_expression_items = false;
        // MS13 T8: 每个 SELECT 项的聚合装配角色（None = 子查询/通配等不经
        // 聚合装配的项），供混合投影校验、分组键归属与条件包装消费。
        #[derive(Clone)]
        enum SelectItemRole {
            /// 聚合项（输出名 = result_column_name / 别名）+ 其注册序号
            /// （T12：重名显示名下名字 → 输出列不再是单射，注册序号才是
            /// 该项到共享清单条目的精确映射）
            Aggregate(String, usize),
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
                        let reg_idx = aggregates.len();
                        item_roles[item_idx] =
                            Some(SelectItemRole::Aggregate(name.clone(), reg_idx));
                        aggregate_args.push(aggregate_arg_expr(expr).cloned());
                        aggregates.push(func);
                        aggregate_display_names.push(name);
                    } else if !inner_suppressed && contains_aggregate_expr(expr) {
                        // change 2026-10-03-practical-core-gaps T4: 聚合嵌入
                        // 表达式项——嵌套聚合调用注册进共享清单，项本体在聚合
                        // 输出行上求值（wrap 阶段编译）。
                        register_aggregate_calls(
                            expr,
                            &mut aggregates,
                            &mut aggregate_args,
                            &mut aggregate_display_names,
                        )?;
                        has_expression_items = true;
                        agg_row_expr_items.push(item_idx);
                        item_roles[item_idx] = Some(SelectItemRole::Expression(expr.to_string()));
                    } else if !inner_suppressed && !is_plain_column_expr(expr) {
                        has_expression_items = true;
                        // T4: 无列引用的纯常量项在聚合查询中按常量混排合法
                        // （聚合输出行求值）；非聚合查询经既有顶层 Projection
                        // 通道，本标记不被消费、行为零变化。
                        if is_pure_constant_expr(expr) {
                            agg_row_expr_items.push(item_idx);
                        }
                        item_roles[item_idx] = Some(SelectItemRole::Expression(expr.to_string()));
                    } else {
                        let col = expr_to_column_name(expr)?;
                        non_agg_columns.push(col.clone());
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
                        let reg_idx = aggregates.len();
                        item_roles[item_idx] =
                            Some(SelectItemRole::Aggregate(name.clone(), reg_idx));
                        aggregate_args.push(aggregate_arg_expr(expr).cloned());
                        aggregates.push(func);
                        aggregate_display_names.push(name);
                    } else if !inner_suppressed && contains_aggregate_expr(expr) {
                        // T4: 聚合嵌入表达式项（同 UnnamedExpr 臂；输出名 = 别名）。
                        register_aggregate_calls(
                            expr,
                            &mut aggregates,
                            &mut aggregate_args,
                            &mut aggregate_display_names,
                        )?;
                        has_expression_items = true;
                        agg_row_expr_items.push(item_idx);
                        item_roles[item_idx] =
                            Some(SelectItemRole::Expression(alias.value.clone()));
                    } else if !inner_suppressed && !is_plain_column_expr(expr) {
                        has_expression_items = true;
                        if is_pure_constant_expr(expr) {
                            agg_row_expr_items.push(item_idx);
                        }
                        item_roles[item_idx] =
                            Some(SelectItemRole::Expression(alias.value.clone()));
                    } else {
                        let col = expr_to_column_name(expr)?;
                        non_agg_columns.push(col.clone());
                        item_roles[item_idx] = Some(SelectItemRole::Column(col));
                    }
                }
                _ => {} // Wildcard etc. — not relevant for aggregate queries
            }
        }

        let has_aggregates = !aggregates.is_empty();

        // change 2026-10-03-practical-core-gaps T6：聚合查询的 ORDER BY 键
        // 预扫描——键中的聚合调用（含 SELECT 未出现的新调用）须在 agg_plan
        // 装配前进入共享清单，否则聚合输出行不含该结果列（装配顺序约束，
        // design D4）。非聚合查询不预扫描（ORDER BY 行为零变化）。
        if has_aggregates {
            for ob in &query.order_by {
                if contains_aggregate_expr(&ob.expr) {
                    register_aggregate_calls(
                        &ob.expr,
                        &mut aggregates,
                        &mut aggregate_args,
                        &mut aggregate_display_names,
                    )?;
                }
            }
        }

        // === MS19 Iteration 002 (T4b): 纯非聚合查询的别名项路由 ===
        // `extract_columns` 把 `ExprWithAlias` 折叠为别名名，而角色分类把
        // 普通列别名的项标为 `Column`——两者叠加使别名项在投影解析阶段以别名
        // 去基 schema 查找，必然落到 identity 桶（表头退化为全 schema）。此处
        // 把「普通列引用 + 别名」的项改标 `Expression` 并置
        // `has_expression_items`，交既有顶层 Projection 通路求值（表头 = 别名，
        // 行 = 该列）；`build_expression` 顺带把未知列点名为 ColumnNotFound。
        //
        // 范围：非聚合、非抑制上下文（顶层 + FROM 派生表/视图内层——R4 三
        // 通道一致；IN/EXISTS/标量内仍抑制）、清单不含通配符；基座为 Scan、
        // 派生表（DerivedScan：FROM 子查询与 CTE 引用共用此装配）与 JOIN/NLJ。
        // MS25 T8：JOIN 基座的纯列别名项改标后不再撞上「表达式项 + JOIN」
        // 拒绝臂（该臂只拦真正的表达式项，见下方 `expression_items_plain`）；
        // 组合行承载与名字解析按 `join_layout` 同基座，输出名 = 别名。
        // 含通配符的清单仍维持改标前行为（会撞上「`*` 不能与表达式项混用」）。
        let alias_reroutable = !has_aggregates
            && !inner_suppressed
            && matches!(
                &base_plan,
                PhysicalPlan::Scan(_)
                    | PhysicalPlan::DerivedScan(_)
                    | PhysicalPlan::Join(_)
                    | PhysicalPlan::SortMergeJoin(_)
                    | PhysicalPlan::NestedLoopJoin(_)
            )
            && !select
                .projection
                .iter()
                .any(|item| matches!(item, sqlparser::ast::SelectItem::Wildcard(_)));
        if alias_reroutable {
            let mut has_aliased_plain_column = false;
            for (item_idx, item) in select.projection.iter().enumerate() {
                if let sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } = item {
                    if matches!(item_roles[item_idx], Some(SelectItemRole::Column(_)))
                        && is_plain_column_expr(expr)
                    {
                        item_roles[item_idx] =
                            Some(SelectItemRole::Expression(alias.value.clone()));
                        has_aliased_plain_column = true;
                    }
                }
            }
            if has_aliased_plain_column {
                has_expression_items = true;
            }
        }

        // MS25 T8（design D9）：表达式项是否全部为纯列引用（含别名重路由的
        // 纯列别名项）。纯列项按组合行形状承载并经布局解析，不属「表达式项 ×
        // JOIN」拒绝面；任一真表达式项（函数 / 算术 / CASE / CAST …）维持既有
        // 显式拒绝。
        let expression_items_plain =
            select.projection.iter().enumerate().all(|(idx, item)| {
                match (&item_roles[idx], item) {
                    (
                        Some(SelectItemRole::Expression(_)),
                        sqlparser::ast::SelectItem::UnnamedExpr(e),
                    )
                    | (
                        Some(SelectItemRole::Expression(_)),
                        sqlparser::ast::SelectItem::ExprWithAlias { expr: e, .. },
                    ) => is_plain_column_expr(e),
                    _ => true,
                }
            });

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
            // JOIN 输出形状是列过滤而非逐项求值，真表达式项 + JOIN 显式拒绝
            //（MS09-T02: 两种 join 节点同语义，拒绝面不因新节点形状漏接；
            // MS25 T8: 纯列引用项走组合行承载 + 布局解析，不属本拒绝面）
            if matches!(
                base_plan,
                PhysicalPlan::Join(_)
                    | PhysicalPlan::SortMergeJoin(_)
                    | PhysicalPlan::NestedLoopJoin(_)
            ) && !expression_items_plain
            {
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
        let mut projection_items = if has_expression_items && !mixed_aggregate {
            // 标量子查询项会追加一列（SubqueryEval 移位输出形状），与表达式
            // 项混用显式拒绝；子查询单独出现维持现状
            if !subquery_evals.is_empty() {
                return Err(PlanError::ParseError(
                    "Expression projection items cannot be mixed with scalar subquery items"
                        .to_string(),
                ));
            }
            // JOIN 输出形状是列过滤而非逐项求值，真表达式项 + JOIN 显式拒绝
            //（MS09-T02: 两种 join 节点同语义，拒绝面不因新节点形状漏接；
            // MS25 T8: 纯列引用项走组合行承载 + 布局解析，不属本拒绝面）
            if matches!(
                base_plan,
                PhysicalPlan::Join(_)
                    | PhysicalPlan::SortMergeJoin(_)
                    | PhysicalPlan::NestedLoopJoin(_)
            ) && !expression_items_plain
            {
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
            //
            // MS25 T8（design D9）：JOIN 基座的纯列项按 `join_layout`（组合行
            // 布局）解析——布局由 `base_plan.output_columns` 构造，与本 Projection
            // 输入行的真实列序同源（顶层 Projection 存在时其下无节点裁剪）。
            if let Some(layout) = &join_layout {
                self.join_column_layout = Some(layout.clone());
            }
            let built_items: Result<Vec<ProjectionItem>, PlanError> = (|| {
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
                Ok(items)
            })();
            self.join_column_layout = None;
            Some(built_items?)
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
            // MS26 T8: the derived base (FROM subquery / CTE / view — one
            // shared DerivedScan assembly path) resolves its pure column list
            // against the derived column set, exactly like the Scan base.
            // Unknown outer names now surface as ColumnNotFound instead of
            // silently widening to the full derived row.
            PhysicalPlan::DerivedScan(node) => Some(node.columns.clone()),
            _ => None,
        };
        let sort_due = !query.order_by.is_empty();
        // T4（change 2026-10-04-external-review-gaps）：标量子查询项不再整体
        // 恒等化投影——混排形态（纯列项 + 子查询项）按过滤后清单解析裁剪，
        // SubqueryEval 的插列位置公式在 SELECT 序行上成立；清单全为子查询项
        // 时无基列可解析，维持恒等（既有行为：全行 + 插列）。
        //
        // 相关参数取值列补充：子查询项携带相关参数时，参数按列名从外层行取
        // 值——SELECT 投影之外的取值列追加到输入投影尾部（不裁掉），由
        // SubqueryEval 链最外层节点按 output_width 截断收尾。任一取值列在
        // 基表 schema 上不可解析（派生表别名等未知形状）→ 整体回落恒等
        // （引入前行为：全行承载，参数恒可达）。
        let subquery_param_columns: Vec<String> = subquery_evals
            .iter()
            .flat_map(|(.., params)| params.iter().map(|p| p.outer_column.to_lowercase()))
            .collect();
        let mut subquery_param_extras: Option<usize> = if subquery_param_columns.is_empty() {
            Some(0)
        } else {
            None
        };
        let projection_indices = if has_aggregates
            // T4：清单全为标量子查询项——无基列可解析，维持恒等（既有行为：
            // 全行 + 插列）。
            || (!subquery_evals.is_empty() && filtered_projection.is_empty())
        {
            None
        } else if has_expression_items {
            // MS11-T01 Iter001: 表达式查询禁用 per-node 裁剪——输入全形状
            // 流出，顶层 ProjectionNode 统一求值 + 裁剪（含 ORDER BY 时
            // Sort 的排序键基础列始终可达，design D10 精神）
            None
        } else {
            match base_schema.as_ref() {
                Some(schema) => {
                    // MS19 Iteration 002 (T4a): 严格解析只作用于纯列清单——
                    // 每个非子查询项都是普通列引用（通配项放行）。非列项不是
                    // 「投影名」：子查询内的字面量投影（`SELECT 1 FROM t`）经
                    // `extract_columns` 产出 `_N` 伪名（ast.rs），顶层字面量
                    // 投影已由 `has_expression_items` 分流到 Projection 通路。
                    // 二者都维持恒等，与变更前一致。
                    if select_list_is_pure_column_list(select, &subquery_indices) {
                        let mut indices = resolve_projection_indices(&projection_columns, schema)?;
                        if let Some(list) = indices.as_mut() {
                            let mut extras = 0usize;
                            let mut resolvable = true;
                            for name in &subquery_param_columns {
                                if projection_columns.iter().any(|c| c.to_lowercase() == *name) {
                                    continue;
                                }
                                match schema.iter().position(|c| c.to_lowercase() == *name) {
                                    Some(i) => {
                                        if !list.contains(&i) {
                                            list.push(i);
                                            extras += 1;
                                        }
                                    }
                                    None => {
                                        resolvable = false;
                                        break;
                                    }
                                }
                            }
                            if resolvable {
                                subquery_param_extras = Some(extras);
                            } else {
                                // 相关取值列不可解析 → 恒等（引入前行为）
                                indices = None;
                            }
                        }
                        indices
                    } else {
                        None
                    }
                }
                // Non-Scan bases (DerivedScan, …) carry no base schema to
                // resolve against — keep the identity projection (design:
                // conservative, do not widen the change surface).
                None => None,
            }
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

        // MS25 T1 (design D2): SELECT 清单在 JOIN 组合形状上的投影索引
        // （`None` = 恒等，如 `SELECT *`）。WHERE×JOIN 路径产出，ORDER BY
        // 存在时交由 Sort 承担同一裁剪。
        let mut where_join_projection: Option<Vec<usize>> = None;

        // Handle WHERE clause
        let plan_with_where = if let Some(where_expr) = &select.selection {
            // MS25 T1（design D2）：JOIN 基座的 WHERE 按**组合行布局**编译——
            // 谓词经 `join_column_layout`（NLJ ON 同款通道）解析为组合行绝对
            // 索引，Filter 在补 NULL 后的行流上求值并承担 SELECT 裁剪。
            // 组合面自然成立：Filter→Aggregate、Filter→Sort、Filter→Limit。
            if let Some(combined) = join_combined_columns(&base_plan) {
                // IN / EXISTS 子查询谓词由 SemiJoin / AntiJoin 机制消费，其左
                // 输入是单表计划——保持变更前的计划期拒绝（同名信息），不与本
                // 能力同批解禁。子查询形态嵌套在复合谓词内时，既有
                // `build_where` 无该臂 → `UnsupportedExpression` 点名拒绝。
                // T4（change 2026-10-04-external-review-gaps）：含标量子查询
                // 比较腿的谓词同面点名拒绝（镜像 InSubquery/Exists）。
                if matches!(
                    where_expr,
                    sqlparser::ast::Expr::InSubquery { .. } | sqlparser::ast::Expr::Exists { .. }
                ) || contains_scalar_subquery(where_expr)
                {
                    return Err(PlanError::UnsupportedStatement(
                        "WHERE CLAUSE ON JOIN".into(),
                    ));
                }
                // 布局 = 组合行形状（按 FROM 臂声明限定符分连续段——限定名
                // 解析臂在首个同名段内查找，非限定名解析臂全表搜索；非限定名
                // 在组合列中唯一命中才解析，多表命中 → AmbiguousColumn，缺失
                // → ColumnNotFound——既有布局解析臂的行为，零新文案）。
                let layout = join_row_layout(combined, &combined_qualifiers);
                self.join_column_layout = Some(layout);
                let compiled = self.build_where(&table_name, where_expr);
                self.join_column_layout = None;
                let predicate = compiled?;

                let select_indices = resolve_join_projection_indices(
                    self,
                    combined,
                    &combined_qualifiers,
                    &qualified_columns,
                    select_list_is_pure_column_list(
                        select,
                        &subquery_evals
                            .iter()
                            .map(|(idx, ..)| *idx)
                            .collect::<Vec<_>>(),
                    ),
                )?;
                // Filter 恒等承载（有 ORDER BY 时裁剪归 Sort；聚合、标量子
                // 查询项与顶层 Projection（MS25 T8：表达式/别名项按全形状求值）
                // 需要完整输入形状，与单表路径的 `proj_or_empty` 同规则）。
                let filter_projection = if sort_due
                    || has_aggregates
                    || has_expression_items
                    || !subquery_evals.is_empty()
                {
                    Vec::new()
                } else {
                    select_indices.clone().unwrap_or_default()
                };
                where_join_projection = select_indices;

                PhysicalPlan::Filter(FilterNode {
                    input: Box::new(base_plan),
                    predicate,
                    table_name: table_name.clone(),
                    projection: filter_projection,
                })
            // Try subquery patterns first (IN subquery / EXISTS)
            } else if let Some(mut subquery_plan) = self.try_build_where_subquery(
                where_expr,
                &base_plan,
                &table_name,
                &projection_columns,
            )? {
                // T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：
                // 基座全宽承载——无 ORDER BY 时 SELECT 清单裁剪由本节点
                // `projection` 承担（ORDER BY 存在时留空恒等，裁剪归 Sort，
                // 排序键可见非输出列）。`projection_indices` 为 None（聚合
                // 输入/表达式项/恒等面）时留空恒等，与 Scan 基座
                // `proj_or_empty` 同规则。
                if !sort_due {
                    let trim = projection_indices.clone().unwrap_or_default();
                    match &mut subquery_plan {
                        PhysicalPlan::SemiJoin(node) => node.projection = trim,
                        PhysicalPlan::AntiJoin(node) => node.projection = trim,
                        _ => {}
                    }
                }
                subquery_plan
            } else if contains_scalar_subquery(where_expr) {
                // T4（change 2026-10-04-external-review-gaps D4）：WHERE 比较
                // 腿标量子查询——子查询绑定预构建 + 占位改写 + 编译，谓词由
                // ScalarSubqueryFilter 逐行承载（不下推进扫描节点、不路由
                // 索引：占位求值需逐行注入子查询结果）。裁剪在谓词求值之后
                // 由节点自身 `projection` 应用（Order BY 存在时为空，Sort
                // 承担裁剪——`is_base_scan_chain` 契约同 Filter 链）。
                let (rewritten, bindings) = self.bind_where_scalar_subqueries(where_expr)?;
                let predicate = self.build_where(&table_name, &rewritten)?;
                let base_columns = base_schema.clone().unwrap_or_default();
                let columns = if proj_or_empty.is_empty() {
                    base_columns
                } else {
                    proj_or_empty
                        .iter()
                        .map(|&i| base_columns[i].clone())
                        .collect()
                };
                PhysicalPlan::ScalarSubqueryFilter(crate::executor::ScalarSubqueryFilterNode {
                    input: Box::new(base_plan),
                    predicate,
                    bindings,
                    columns,
                    table_name: table_name.clone(),
                    projection: proj_or_empty.clone(),
                })
            } else if let Some(key) = self.extract_pk_from_where_gated(&table_name, where_expr)? {
                // Try to extract primary key from WHERE clause for index scan
                // Simple PK equality check - use index scan
                // Note: This is a simplification. A more sophisticated optimizer would
                // check if the WHERE clause is ONLY pk = value, not part of a complex expression
                if self.is_simple_pk_equality(&table_name, where_expr)? {
                    // T4: 相关参数补充列在输入投影尾部——IndexScan「已收窄」
                    // 列名随投影同步附加（行形状 = SELECT ∪ 补充列）。
                    let mut narrowed = projection_columns.clone();
                    for name in &subquery_param_columns {
                        let n = name.to_lowercase();
                        if !narrowed.iter().any(|c| c.to_lowercase() == n) {
                            narrowed.push(name.clone());
                        }
                    }
                    PhysicalPlan::IndexScan(IndexScanNode {
                        table_name: table_name.clone(),
                        key,
                        columns: if !sort_due && projection_indices.is_some() {
                            narrowed
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
            } else if let Some((index_name, key)) =
                self.extract_secondary_index_equality(&table_name, where_expr)?
            {
                // 二级索引等值路由——PK 判定链未命中后、非 PK 臂前插入。
                // 单条件裸列等值且 `secondary_index_key` 返回键时走
                // `IndexScanAll`（执行器消费该登记树）。键原样写入节点。
                // 其余形态一律既有路径。
                // T1（change 2026-10-04-external-review-gaps D1）：`columns`
                // 恒为基表全列，输出形状由 `projection` 唯一决定——消费方
                // （get_plan_output_columns / EXPLAIN 渲染 / 聚合输入 schema）
                // 按「基表全列 × 投影号」契约裁剪。PK IndexScan 臂保持
                // 「已收窄」契约不变，两契约并存是消费方现状。
                PhysicalPlan::IndexScanAll(IndexScanAllNode {
                    table_name: table_name.clone(),
                    key,
                    columns: base_schema.clone().unwrap_or_default(),
                    projection: proj_or_empty.clone(),
                    index_name: Some(index_name),
                })
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
                // MS26 T8: the derived base has no WHERE wrapper to own the
                // trim, so the node itself carries the projection (mirrors the
                // DataScan swap above; empty when Sort owns it).
                PhysicalPlan::DerivedScan(mut derived_node) => {
                    derived_node.projection = proj_or_empty.clone();
                    PhysicalPlan::DerivedScan(derived_node)
                }
                _ => base_plan,
            }
        };

        // change 2026-10-03-practical-core-gaps T6：聚合查询存在表达式排序键
        // 时，Sort 须在聚合输出行上求值（键中的聚合调用不必出现在 SELECT
        // 清单）——条件包装（若存在）移到 Sort 之上装配（deferred）；
        // `agg_row_output_columns` 携带聚合输出行名集供表达式键编译。
        let mut deferred_wrap: Option<(Vec<ProjectionItem>, Vec<String>)> = None;
        let mut agg_row_output_columns: Option<Vec<String>> = None;
        // T12：聚合输出行编译器（嵌入项 / HAVING / ORDER BY 表达式键）的规范名
        // 快照——聚合分支装配前从 `aggregates` 采集（该清单随后移入
        // AggregateNode），ORDER BY 编译点在分支之后仍可消费。
        let mut aggregate_canonical_names: Vec<String> = Vec::new();
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
                    .map(|e| {
                        self.resolve_group_by_item(
                            e,
                            &select.projection,
                            &column_indices,
                            join_layout.as_deref(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                sqlparser::ast::GroupByExpr::All => {
                    // GROUP BY ALL: all non-aggregate columns（列名键，既有语义）
                    non_agg_columns
                        .iter()
                        .map(|col| GroupKey {
                            name: col.clone(),
                            column: Some(col.clone()),
                            expr_item: None,
                            input_index: None,
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
            // change 2026-10-03-practical-core-gaps T4: 聚合嵌入表达式项与纯
            // 常量项在聚合输出行上求值，不受分组键绑定校验约束（嵌套非聚合
            // 列引用在 wrap 编译期点名）。
            for (i, role) in item_roles.iter().enumerate() {
                if let Some(SelectItemRole::Expression(name)) = role {
                    if agg_row_expr_items.contains(&i) {
                        continue;
                    }
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
                        // MS25 T8：JOIN 基座的布局感知键直取已解析索引；单表面
                        // 沿用既有按名查索引语义（`column_indices` 已知索引）。
                        let idx = match k.input_index {
                            Some(idx) => idx,
                            None => *column_indices
                                .get(&col.to_lowercase())
                                .ok_or_else(|| PlanError::NonAggregatedColumn(col.clone()))?,
                        };
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
            let mut column_items: Vec<&str> = Vec::new();
            for role in item_roles.iter().flatten() {
                match role {
                    SelectItemRole::Column(name) => {
                        column_items.push(name.as_str());
                        if seen_aggregate {
                            select_keys_first = false;
                        }
                    }
                    SelectItemRole::Aggregate(..) => seen_aggregate = true,
                    SelectItemRole::Expression(_) => select_keys_first = false,
                }
            }
            // MS25 T9（design D9）：形状安全的直出判定——分组键集合必须是 SELECT
            // 纯列项的子集（未入选键不进输出），且键序与 SELECT 键序一致（直出行的
            // 值序是 GROUP BY 序、表头序是 SELECT 序）。任一条不成立即改走既有
            // 条件包装路径（`wrap_projection` 按 SELECT 清单出值），输出行与表头
            // 恒为 SELECT 清单形状；未入选分组键仅经 `node_output_columns` 供
            // HAVING 绑定。反向（非键列项）由上方既有严格检查点名
            // （NonAggregatedColumn），此处不加重复校验。
            let group_key_names: Vec<&str> = group_keys.iter().map(|k| k.name.as_str()).collect();
            let keys_selected_in_select_order = column_items.len() == group_key_names.len()
                && column_items
                    .iter()
                    .zip(group_key_names.iter())
                    .all(|(c, k)| c.eq_ignore_ascii_case(k));
            // change 2026-10-03-practical-core-gaps T6：聚合注册序显示名数
            // ≠ SELECT 聚合角色数（ORDER BY 键引入的额外聚合，或同一聚合
            // 调用去重共享）时强制走条件包装路径——执行器行含全部注册聚合，
            // 直出形状会泄漏 SELECT 未选列，包装按 SELECT 清单裁剪。
            let role_agg_count = item_roles
                .iter()
                .flatten()
                .filter(|r| matches!(r, SelectItemRole::Aggregate(..)))
                .count();
            let direct = !has_expression_items
                && aggregate_display_names.len() == role_agg_count
                && keys_all_column
                && select_keys_first
                && keys_selected_in_select_order;

            let key_count = group_keys.len();
            // 输出列名（change 2026-10-03-practical-core-gaps T4/T6 统一公式）：
            // 分组键名 ++ 注册序聚合显示名——与执行器行布局（group keys ++
            // aggregates 注册序）严格对齐。无 T4/T6 项的直出形态与既有
            // agg_output_columns（SELECT 序）逐字节一致（直出判定强制分组键 =
            // SELECT 纯列项且键序一致，SELECT 聚合项注册序 = SELECT 序）；非
            // 直出形态与既有「键名 ++ 聚合角色名」一致（无 T4 项时角色序 =
            // 注册序）。
            let node_output_columns = group_keys
                .iter()
                .map(|k| k.name.clone())
                .chain(aggregate_display_names.iter().cloned())
                .collect::<Vec<String>>();
            // T6：表达式排序键在聚合输出行上编译所需的行名集。
            agg_row_output_columns = Some(node_output_columns.clone());
            // T12：各注册项的规范名（func 身份，与 display names 平行）——
            // 聚合输出行编译器（嵌入项 / HAVING / ORDER BY 键）按规范名解析
            // 聚合引用，显示名被别名遮蔽时不再首匹配取错列。
            aggregate_canonical_names = aggregates.iter().map(|f| f.result_column_name()).collect();

            // 条件包装项（仅非直出形态）：SELECT 项 → 聚合行位置的
            // ColumnExpression 重排，列名 = SELECT 名；聚合嵌入表达式项与纯
            // 常量项（T4）按编译后的聚合行表达式求值。
            // T6：存在表达式排序键（非裸列键）时，包装移到 Sort 之上装配——
            // Sort 直接落在（Having 包裹的）聚合输出行上。
            let mut wrap_projection = if direct {
                None
            } else {
                let mut items = Vec::with_capacity(select.projection.len());
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
                            if agg_row_expr_items.contains(&i) {
                                // T4: 聚合嵌入表达式项 / 纯常量项——在聚合输出
                                // 行上编译求值（嵌套非聚合列引用在此点名）。
                                let e = match &select.projection[i] {
                                    sqlparser::ast::SelectItem::UnnamedExpr(e) => e,
                                    sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => expr,
                                    _ => {
                                        unreachable!(
                                            "agg-row expression items are expression items"
                                        )
                                    }
                                };
                                let compiled = self.compile_aggregate_row_expr(
                                    e,
                                    &node_output_columns,
                                    &aggregate_canonical_names,
                                    AggRowExprMode::OutputRow,
                                )?;
                                items.push(ProjectionItem {
                                    expr: compiled,
                                    name: name.clone(),
                                });
                            } else {
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
                        }
                        Some(SelectItemRole::Aggregate(name, reg_idx)) => {
                            // T4/T12: 聚合输出行索引 = 分类期记录的注册序号——
                            // T4 嵌入聚合使角色序与注册序不再一致，重名显示名
                            // 使名字 → 输出列不再是单射（按名首匹配静默取错
                            // 列）；注册序号是该角色的精确映射，无 T4 项时与
                            // 既有 key_count + 角色序逐值一致。
                            items.push(ProjectionItem {
                                expr: Arc::new(ColumnExpression {
                                    column_name: name.to_lowercase(),
                                    column_index: key_count + reg_idx,
                                }),
                                name: name.clone(),
                            });
                        }
                    }
                }
                let columns = items.iter().map(|it| it.name.clone()).collect();
                Some((items, columns))
            };

            // Build HAVING predicate BEFORE consuming node_output_columns
            let having_pred = if let Some(having_expr) = &select.having {
                Some(self.build_having(
                    having_expr,
                    &node_output_columns,
                    &aggregate_canonical_names,
                )?)
            } else {
                None
            };

            // MS25 T12（design D12）：聚合参数的输入行索引——两个基座统一在计划期
            // 解析。JOIN 基座按组合布局逐个解析（限定名按表定位、裸名唯一命中，
            // 歧义 `AmbiguousColumn`、查无 `ColumnNotFound` 点名）；单表面按
            // `column_indices`（输入行真实列序）解析，`Identifier` 取小写名、
            // `CompoundIdentifier` 沿既有末段名语义。查无一律点名——执行器的
            // 名字查找回退对列参数不再可达（保留为防御）。`COUNT(*)` 无参
            // 参数列为 `None`。
            let aggregate_arg_indices: Vec<Option<usize>> = aggregate_args
                .iter()
                .map(|arg| match arg {
                    None => Ok(None),
                    Some(Expr::Identifier(ident)) => match &join_layout {
                        Some(layout) => resolve_layout_index(self, layout, None, &ident.value)
                            .and_then(|idx| {
                                idx.map(Some)
                                    .ok_or_else(|| PlanError::ColumnNotFound(ident.value.clone()))
                            }),
                        None => column_indices
                            .get(&ident.value.to_lowercase())
                            .copied()
                            .map(Some)
                            .ok_or_else(|| PlanError::ColumnNotFound(ident.value.clone())),
                    },
                    Some(Expr::CompoundIdentifier(parts)) if parts.len() == 2 => {
                        // Cycle 001（replan）design D4：两段限定符先在 FROM 作用域
                        // 内点名（42P01 携限定符原文，单表与 JOIN 同语义），再按
                        // 基座既有路径解析；in-scope 限定符 + 列缺失维持既有错误面。
                        if !self.qualifier_in_scope(&parts[0].value) {
                            Err(PlanError::TableNotFound(parts[0].value.clone()))
                        } else {
                            match &join_layout {
                                Some(layout) => resolve_layout_index(
                                    self,
                                    layout,
                                    Some(&parts[0].value),
                                    &parts[1].value,
                                )
                                .and_then(|idx| {
                                    idx.map(Some).ok_or_else(|| {
                                        PlanError::ColumnNotFound(parts[1].value.clone())
                                    })
                                }),
                                None => column_indices
                                    .get(&parts[1].value.to_lowercase())
                                    .copied()
                                    .map(Some)
                                    .ok_or_else(|| {
                                        PlanError::ColumnNotFound(parts[1].value.clone())
                                    }),
                            }
                        }
                    }
                    // 非列引用参数（字面量等）沿既有名字查找语义
                    _ => Ok(None),
                })
                .collect::<Result<Vec<_>, PlanError>>()?;

            let agg_plan = PhysicalPlan::Aggregate(crate::executor::AggregateNode {
                input: Box::new(plan_with_where),
                group_by,
                group_key_exprs,
                aggregates,
                aggregate_arg_indices,
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

            if wrap_projection.is_some()
                && query
                    .order_by
                    .iter()
                    .any(|o| extract_column_name(&o.expr).is_err())
            {
                deferred_wrap = wrap_projection.take();
            }

            match wrap_projection {
                Some((items, columns)) => PhysicalPlan::Projection(ProjectionNode {
                    input: Box::new(with_having),
                    items,
                    columns,
                }),
                None => with_having,
            }
        } else {
            // Cycle 001（replan）design D4：无聚合路径的 GROUP BY 限定符点名
            // （有聚合路径由 resolve_group_by_item 承担）。
            self.validate_group_by_qualifiers(&select.group_by)?;
            plan_with_where
        };

        // Parse ORDER BY
        let plan_with_order = if !query.order_by.is_empty() {
            let mut order_by: Vec<OrderByColumn> = query
                .order_by
                .iter()
                .map(|o| {
                    // change 2026-10-03-practical-core-gaps T6：聚合查询的非
                    // 裸列键经聚合输出行编译器产出表达式键（键中聚合调用与
                    // 常量表达式在聚合输出行上求值）；裸列/限定列键维持既有
                    // 路径。非聚合查询零变化。
                    if let Some(agg_columns) = &agg_row_output_columns {
                        if extract_column_name(&o.expr).is_err() {
                            let compiled = self.compile_aggregate_row_expr(
                                &o.expr,
                                agg_columns,
                                &aggregate_canonical_names,
                                AggRowExprMode::OutputRow,
                            )?;
                            // sqlparser: asc field is Option<bool>
                            // None or Some(true) = ASC, Some(false) = DESC
                            return Ok(OrderByColumn {
                                column: o.expr.to_string(),
                                asc: o.asc.unwrap_or(true),
                                expr: Some(compiled),
                            });
                        }
                    }
                    let (qualifier, column) = extract_column_name(&o.expr)?;
                    // design D4：限定键先在 FROM 作用域内点名（别名归一后仍
                    // 不可达 → 42P01 携带限定符），再按末段列名进入既有键
                    // 校验集（`sort_columns` ∪ 输出别名）——排序键语义与裸名
                    // 键同源，SortExecutor 无需感知限定符。
                    if let Some(qualifier) = &qualifier {
                        if !self.qualifier_in_scope(qualifier) {
                            return Err(PlanError::TableNotFound(qualifier.clone()));
                        }
                        // T3（change 2026-10-04-external-review-gaps D3）：JOIN
                        // 基座（组合行布局存在）的限定键经布局解析产出表达式
                        // 键（decorate-sort-undecorate，别名键先例同型）——
                        // 自连接同名列按限定符精确解析，不再受裸名首匹配局限。
                        // 仅限 Sort 输入行 = 组合行的形态（无聚合、无标量
                        // 子查询项）：聚合查询的 Sort 在聚合输出行上，限定键
                        // 维持既有裸名键路径（按聚合输出列集解析）。布局未命中
                        // （非 JOIN 基座等）同回落。
                        if !has_aggregates && subquery_evals.is_empty() {
                            if let Some(layout) = &join_layout {
                                if let Some(expr) =
                                    self.resolve_order_by_layout_key(layout, qualifier, &column)?
                                {
                                    // sqlparser: asc field is Option<bool>
                                    let asc = o.asc.unwrap_or(true);
                                    return Ok(OrderByColumn {
                                        column,
                                        asc,
                                        expr: Some(expr),
                                    });
                                }
                            }
                        }
                    }
                    // sqlparser: asc field is Option<bool>
                    // None or Some(true) = ASC, Some(false) = DESC
                    let asc = o.asc.unwrap_or(true);
                    Ok(OrderByColumn {
                        column,
                        asc,
                        expr: None,
                    })
                })
                .collect::<Result<Vec<_>, PlanError>>()?;

            // MS10-T01 Iter001: sort-key lookup uses the input plan's real
            // output shape. With no aggregate in play the Sort node also owns
            // the projection trim (the scan chain below emits full-schema
            // rows, so keys outside the projection stay reachable — design
            // D10). Aggregate inputs keep their own output shape and are not
            // re-projected.
            let sort_columns = if has_aggregates {
                if deferred_wrap.is_some() {
                    // change 2026-10-03-practical-core-gaps T6：Sort 落在聚合
                    // 输出行上（条件包装已移至 Sort 之上），列键按聚合行名集
                    // 解析（该形态在旧路由下计划期报错，无既有行为）。
                    agg_row_output_columns.clone().unwrap_or_default()
                } else {
                    projection_columns.clone()
                }
            } else {
                self.get_plan_output_columns(&plan_with_aggregate)
            };
            // MS25 T1 (design D2): the `Filter(JOIN combination)` shape is the
            // second owner of the projection trim — the Filter stays identity
            // (its predicate runs on padded combination rows) and the Sort
            // narrows to the SELECT list, resolved against the combination
            // shape. Aggregate inputs keep their own output shape and are not
            // re-projected.
            let sort_projection = if has_aggregates || has_expression_items {
                // MS25 T8：顶层 Projection 在 Sort 之上按未裁剪行求值——此时
                // 裁剪交由 Projection 承担（与单表路径 `projection_indices` 在
                // 表达式项下为 None 的既有规则同型）。
                Vec::new()
            } else if is_base_scan_chain(&plan_with_aggregate) {
                projection_indices.clone().unwrap_or_default()
            } else if is_identity_filter_over_join(&plan_with_aggregate) {
                where_join_projection.clone().unwrap_or_default()
            } else if matches!(
                plan_with_aggregate,
                PhysicalPlan::SemiJoin(_) | PhysicalPlan::AntiJoin(_)
            ) {
                // T5（change 2026-10-13-e2e-verification-and-defect-closure D8）：
                // SemiJoin/AntiJoin 基座全宽承载（行 = 左表全宽行），Sort 在其上
                // 按全宽列集排序（ORDER BY 非输出列可达），裁剪归本节点——
                // 索引按基表 schema 序解析（与 Scan 基座臂同规则；`None` 恒等
                // 面 unwrap_or_default 为空 = 全宽输出）。
                projection_indices.clone().unwrap_or_default()
            } else if join_combined_columns(&plan_with_aggregate).is_some() {
                // T3（change 2026-10-04-external-review-gaps D3）：无 WHERE 的
                // 裸 JOIN 基座在 ORDER BY 存在时承载全组合列（carry_all 加序），
                // SELECT 裁剪归 Sort——与 WHERE×JOIN 的 `where_join_projection`
                // 同规则解析。标量子查询项维持既有恒等承载（与 WHERE 路径
                // `filter_projection` 的排序让位条件同款）。
                if subquery_evals.is_empty() {
                    resolve_join_projection_indices(
                        self,
                        join_combined_columns(&plan_with_aggregate)
                            .expect("JOIN base checked above"),
                        &combined_qualifiers,
                        &qualified_columns,
                        select_list_is_pure_column_list(select, &subquery_indices),
                    )?
                    .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // MS19 Iteration 002 (T6): 排序键 SHALL 在计划期对 Sort 节点的真实
            // 可见列集（`sort_columns`）校验。`SortExecutor::compare_rows` 对列集
            // 里不存在的键 `position()` 得 None 后静默跳过该键——产出未排序结果
            // 而不报错。匹配规则与 `compare_rows` 同源（大小写不敏感）。
            //
            // 输出别名键为例外，不属拒绝面：顶层投影项的别名（`ExprWithAlias`）在
            // Sort 之后即该列的输出名，`ORDER BY <别名>` 引用它（MS25 T7 起按
            // 输出列真实排序，解析见下方表达式键装配）。
            let alias_columns: Vec<String> = select
                .projection
                .iter()
                .filter_map(|item| match item {
                    sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } => {
                        Some(alias.value.to_lowercase())
                    }
                    _ => None,
                })
                .collect();
            for ob in &order_by {
                // change 2026-10-03-practical-core-gaps T6：表达式键豁免列集
                // 校验（别名键先例同型——键值由 SortExecutor 的 decorate 表达
                // 式求值承载，不按列名查位）。
                if ob.expr.is_some() {
                    continue;
                }
                let key = ob.column.to_lowercase();
                let mut resolvable = sort_columns.iter().any(|c| c.to_lowercase() == key)
                    || alias_columns.contains(&key);
                // MS25 T2: DISTINCT 查询的排序键 SHALL ∈ 输出列集（SQLite 的
                // DISTINCT 约束）。Sort 位于 Distinct 之下时键解析于宽形状，
                // 此处显式收窄到输出列集，保证「先去重后排序」等价。
                if distinct_due {
                    let in_output = projection_columns.iter().any(|c| c.to_lowercase() == key)
                        || alias_columns.contains(&key);
                    resolvable = resolvable && in_output;
                }
                if !resolvable {
                    return Err(if distinct_due {
                        PlanError::ParseError(format!(
                            "ORDER BY key must appear in the SELECT list of a DISTINCT query: {}",
                            ob.column
                        ))
                    } else {
                        PlanError::ColumnNotFound(ob.column.clone())
                    });
                }
            }

            // MS25 T7（design D8）：输出别名键按**输出列**真实排序——键命中顶层
            // 投影项的 `ExprWithAlias` 别名时，按别名回查该输出项的底层 `Expr`
            // 并经 `build_expression(&table_name, …)` 编译（与顶层 ProjectionItem
            // 同一编译基座），置入 `OrderByColumn.expr` 交 SortExecutor
            // decorate-sort-undecorate 在未投影的输入行上求值。
            //
            // 输出名优先：别名键即输出列，别名与同名输入列都可匹配时按输出列排序
            // （别名 = 源列名、别名遮蔽源列两形态）。
            //
            // 范围：`projection_items` 存在即纯非聚合表达式项通道（聚合查询的
            // 别名键 `sort_columns = projection_columns` 命中列名键，已真实排序，
            // 零变化；JOIN 基座表达式项与子查询路径在既有拒绝/抑制臂内，无输出
            // 别名面）。未命中别名项的键维持既有列名键。
            if projection_items.is_some() {
                if let Some(layout) = &join_layout {
                    self.join_column_layout = Some(layout.clone());
                }
                let compiled: Result<(), PlanError> = (|| {
                    for ob in order_by.iter_mut() {
                        let key = ob.column.to_lowercase();
                        let Some(expr) = select.projection.iter().find_map(|item| match item {
                            sqlparser::ast::SelectItem::ExprWithAlias { expr, alias }
                                if alias.value.to_lowercase() == key =>
                            {
                                Some(expr)
                            }
                            _ => None,
                        }) else {
                            continue;
                        };
                        ob.expr = Some(self.build_expression(&table_name, expr)?);
                    }
                    Ok(())
                })();
                self.join_column_layout = None;
                compiled?;
            }

            let sort_plan = PhysicalPlan::Sort(SortNode {
                input: Box::new(plan_with_aggregate),
                order_by,
                table_name: table_name.clone(),
                columns: sort_columns,
                projection: sort_projection,
                // 有界与否由顶层 LIMIT 包装点按需写入（design D3）。
                bounded: None,
            });
            // change 2026-10-03-practical-core-gaps T6：条件包装在 Sort 之上
            // 装配（Sort 在聚合输出行上求值后再按 SELECT 清单出值）；无表达式
            // 排序键时包装位置与形状逐字节不变（deferred 恒为 None）。
            match deferred_wrap {
                Some((items, columns)) => PhysicalPlan::Projection(ProjectionNode {
                    input: Box::new(sort_plan),
                    items,
                    columns,
                }),
                None => sort_plan,
            }
        } else {
            plan_with_aggregate
        };

        // === MS25 T2（design D3）：DISTINCT 插在「输出形状已就绪」处 ===
        // 纯列清单：Sort 之下即输出形状 → Distinct 直接在 Sort 之上。
        // 表达式项（含别名重路由）：顶层 Projection 求值后才成输出形状 →
        // Projection 先于 Distinct 装配（其下移只在本量化词存在时发生，
        // 无量化词路径的 Projection 位置逐字节不变）。聚合查询的混合投影
        // 包装 Projection 已在其内（`plan_with_aggregate`），Distinct 直接
        // 落在聚合输出形状之上。
        let plan_pre_distinct = if distinct_due {
            match projection_items.take() {
                Some(items) => {
                    let columns = items.iter().map(|i| i.name.clone()).collect();
                    PhysicalPlan::Projection(ProjectionNode {
                        input: Box::new(plan_with_order),
                        items,
                        columns,
                    })
                }
                None => plan_with_order,
            }
        } else {
            plan_with_order
        };
        let plan_with_distinct = if distinct_due {
            PhysicalPlan::Distinct(DistinctNode {
                input: Box::new(plan_pre_distinct),
            })
        } else {
            plan_pre_distinct
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
            let mut input = match plan_with_distinct {
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

            // MS32 Iter002（design D3）：LIMIT 直接约束排序时只驻留
            // `offset + limit` 行——形状为 `Sort` 或 `Projection(Sort)`
            // （deferred_wrap）；`Distinct(...)` 与其他形状不设界，全量排序。
            mark_sort_bounded(&mut input, offset.saturating_add(limit));

            PhysicalPlan::Limit(crate::executor::LimitNode {
                input: Box::new(input),
                limit,
                offset,
            })
        } else {
            plan_with_distinct
        };

        // === Wrap with SubqueryEval nodes for scalar subqueries in SELECT ===
        // Process from right to left so that result_column_index calculations remain stable
        // result_column_index = projection_index - (number of subqueries at indices < projection_index)
        //
        // T4：相关参数补充列（`subquery_param_extras`）只在输入行中存活到
        // 本链**最外层**节点（k+1 == total）——该节点按 output_width（SELECT
        // 纯列宽 + 更深节点的标量数）截断收尾，补充列不泄漏进输出。
        let mut plan = plan_with_limit;
        let total_subquery_items = subquery_evals.len();
        for (k, (proj_idx, subquery_plan, col_name, correlated_params)) in
            subquery_evals.iter().rev().enumerate()
        {
            let subqueries_before = subquery_evals
                .iter()
                .filter(|(idx, _, _, _)| idx < proj_idx)
                .count();
            let result_column_index = proj_idx - subqueries_before;
            let output_width = match subquery_param_extras {
                Some(extras) if extras > 0 && k + 1 == total_subquery_items => {
                    projection_indices
                        .as_ref()
                        .map(|l| l.len() - extras)
                        .unwrap_or(0)
                        + total_subquery_items
                        - 1
                }
                _ => 0,
            };
            plan = PhysicalPlan::SubqueryEval(crate::executor::SubqueryEvalNode {
                input: Box::new(plan),
                subquery: Box::new(subquery_plan.clone()),
                output_column: col_name.clone(),
                result_column_index,
                correlated_params: correlated_params.clone(),
                output_width,
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
                    // MS31 Iteration 001：参数占位位不是字面量腿。
                    if is_placeholder_value(v) {
                        return Ok(false);
                    }
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

    /// 纯等值内连接在五个条件同时成立时改走排序合并：左右都是基表 `Scan`，
    /// 条件非空，每一条左列是左表主键、右列是右表主键，两侧主键类型都是已登记的整数，
    /// 且两侧扫描表都在 `explicit_primary_keys` 里。
    fn integer_pk_base_scan_join(&self, join: &crate::executor::JoinNode) -> bool {
        if join.conditions.is_empty() {
            return false;
        }
        let (PhysicalPlan::Scan(left), PhysicalPlan::Scan(right)) =
            (join.left.as_ref(), join.right.as_ref())
        else {
            return false;
        };
        self.registered_integer_primary_key(&left.table_name)
            && self.registered_integer_primary_key(&right.table_name)
            && self.explicit_primary_keys.contains(&left.table_name)
            && self.explicit_primary_keys.contains(&right.table_name)
            && join.conditions.iter().all(|condition| {
                self.column_is_scan_primary_key(&condition.left_column, &left.table_name)
                    && self.column_is_scan_primary_key(&condition.right_column, &right.table_name)
            })
    }

    fn registered_integer_primary_key(&self, table_name: &str) -> bool {
        matches!(
            self.primary_key_types.get(table_name),
            Some(crate::storage::page_format::ColumnType::Int)
        )
    }

    fn column_is_scan_primary_key(&self, column: &ColumnRef, scan_table: &str) -> bool {
        let Some(pk) = self.primary_keys.get(scan_table) else {
            return false;
        };
        if pk.is_empty() || column.column != *pk {
            return false;
        }
        match &column.table {
            Some(table) => table == scan_table,
            None => true,
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

    /// 二级索引等值路由键提取。WHERE 为单个裸列等值（字面量可在任一侧）且该列
    /// 命中没有额外列的二级索引时，用该索引的世代调用
    /// `Value::derive_secondary_index_key`，返回 `(索引名, 键)`。限定名、AND、OR、
    /// 范围和参数占位返回 `None`。类型向量缺项时也返回 `None`，
    /// 不回退到 `to_key`。字面量转换失败与主键提取同型传播。
    fn extract_secondary_index_equality(
        &self,
        table_name: &str,
        expr: &Expr,
    ) -> Result<Option<(String, crate::storage::page_format::Key)>, PlanError> {
        let Some(indexes) = self.table_secondary_indexes.get(table_name) else {
            return Ok(None);
        };
        let Expr::BinaryOp {
            left,
            op: sqlparser::ast::BinaryOperator::Eq,
            right,
        } = expr
        else {
            return Ok(None);
        };
        // column = literal / literal = column（裸列名；限定名不路由——与
        // PK 简单等值同口径）
        let (column, literal) = match (left.as_ref(), right.as_ref()) {
            (Expr::Identifier(c), Expr::Value(v)) => (c, v),
            (Expr::Value(v), Expr::Identifier(c)) => (c, v),
            _ => return Ok(None),
        };
        let column_name = column.value.to_lowercase();
        let Some((index_name, _, generation)) =
            indexes.iter().find(|(_, col, _)| *col == column_name)
        else {
            return Ok(None);
        };
        // MS31 Iteration 001：参数占位位在计划期无值——不路由索引键。
        if is_placeholder_value(literal) {
            return Ok(None);
        }
        let value = value_from_sqlparser(literal)?;
        let Some(column_index) = self.tables.get(table_name).and_then(|columns| {
            columns
                .iter()
                .position(|column| column.to_lowercase() == column_name)
        }) else {
            return Ok(None);
        };
        let Some(column_type) = self
            .table_column_types
            .get(table_name)
            .and_then(|types| types.get(column_index))
        else {
            return Ok(None);
        };
        match crate::executor::Value::derive_secondary_index_key(
            std::slice::from_ref(column_type),
            std::slice::from_ref(&value),
            *generation,
        ) {
            Some(key) => Ok(Some((index_name.clone(), key))),
            None => Ok(None),
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
                        // MS31 Iteration 001：参数占位位在计划期无值——不提取
                        // 定位键（回落既有扫描路径，谓词在执行期按参数值求值）。
                        if is_placeholder_value(v) {
                            return Ok(None);
                        }
                        let value = value_from_sqlparser(v)?;
                        return Ok(value.to_key());
                    }
                }
            }

            // Case 2: value = column
            if let Expr::Identifier(ident) = right.as_ref() {
                if ident.value.to_lowercase() == pk_column {
                    if let Expr::Value(v) = left.as_ref() {
                        if is_placeholder_value(v) {
                            return Ok(None);
                        }
                        let value = value_from_sqlparser(v)?;
                        return Ok(value.to_key());
                    }
                }
            }
        }

        // Unsupported WHERE clause
        Ok(None)
    }

    /// Build output_columns for a subquery-base node (T5, change
    /// 2026-10-13-e2e-verification-and-defect-closure D8): the full left-table
    /// column list at schema ordinals — the node carries full-width rows
    /// (`build_output_row` passes the left row through) and the SELECT-list
    /// trim is owned downstream (node `projection` / Sort `sort_projection`).
    pub(crate) fn build_full_output_columns_for_table(
        &self,
        table_name: &str,
    ) -> Vec<OutputColumn> {
        let columns = self.tables.get(table_name).cloned().unwrap_or_default();
        columns
            .iter()
            .enumerate()
            .map(|(column_index, col)| OutputColumn {
                table: Some(table_name.to_string()),
                column: col.clone(),
                table_alias: table_name.to_string(),
                column_index,
            })
            .collect()
    }
}

/// MS25 T1（design D1）：JOIN 链单步的外连接形态。`None` = INNER（既有语义：
/// 等值 ON 走 Hash，其余走 NLJ）；`Right` 经左右交换重写后按 `Left` 承载。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OuterJoinKind {
    None,
    Left,
    Right,
    Full,
}

/// MS25 T3：分支树上的 `SELECT DISTINCT` 探测。复合语句内任一分支（含嵌套
/// 复合的分支）带 DISTINCT 量化词即计划期点名拒绝——否则该分支会被静默去重，
/// 与复合操作自身的去重语义叠加成无法解释的结果。
fn set_operand_has_distinct(expr: &sqlparser::ast::SetExpr) -> bool {
    match expr {
        sqlparser::ast::SetExpr::Select(select) => select.distinct.is_some(),
        sqlparser::ast::SetExpr::SetOperation { left, right, .. } => {
            set_operand_has_distinct(left) || set_operand_has_distinct(right)
        }
        sqlparser::ast::SetExpr::Query(inner) => set_operand_has_distinct(&inner.body),
        _ => false,
    }
}

/// MS25 T1（design D2）：JOIN 基座的输出列 = 组合行形状——Filter 谓词的列索引、
/// `get_plan_output_columns` 与 Sort 投影都以它为基准。
fn join_combined_columns(plan: &PhysicalPlan) -> Option<&[OutputColumn]> {
    match plan {
        PhysicalPlan::Join(node) | PhysicalPlan::SortMergeJoin(node) => Some(&node.output_columns),
        PhysicalPlan::NestedLoopJoin(node) => Some(&node.output_columns),
        _ => None,
    }
}

/// MS25 T1（design D2）：`Filter(JOIN)` 恒等投影形态——谓词在补 NULL 后的
/// 组合行上求值，SELECT 裁剪由其上的 Sort 承担。
fn is_identity_filter_over_join(plan: &PhysicalPlan) -> bool {
    match plan {
        PhysicalPlan::Filter(node) => {
            node.projection.is_empty() && join_combined_columns(&node.input).is_some()
        }
        _ => false,
    }
}

/// MS25 T8（design D9）：由 JOIN 节点的实际输出列构造行布局（FROM 臂段 → 列名，
/// 按行序连续）。与 `build_from_clause_with_projection` 内 WHERE 路径的布局装配
/// 同型，但以 `output_columns` 为唯一真源——该列表就是 Sort/Aggregate 输入行的
/// 真实列序（无 WHERE 的 JOIN 承载过滤后布局，有 WHERE/聚合的承载全组合布局）。
/// design D4：段键取 `qualifiers`（声明限定符，与 `columns` 同序同长，来自
/// FROM 构建器）——`columns` 的 `table_alias` 是真实表名，自连接同表两别名
/// 无法据此分段。
fn join_row_layout(columns: &[OutputColumn], qualifiers: &[String]) -> Vec<(String, Vec<String>)> {
    let mut layout: Vec<(String, Vec<String>)> = Vec::new();
    for (idx, col) in columns.iter().enumerate() {
        let qualifier = qualifiers.get(idx).cloned().unwrap_or_default();
        match layout.last_mut() {
            Some((alias, seg)) if *alias == qualifier => seg.push(col.column.clone()),
            _ => layout.push((qualifier, vec![col.column.clone()])),
        }
    }
    layout
}

/// MS25 T8（design D9）：在 JOIN 基座行布局上解析列引用到行内索引。限定名按
/// （段, 列）精确匹配并取首个命中；裸名唯一命中返回其索引，多表命中 → `AmbiguousColumn`，
/// 无命中返回 `None`（交回调用方的既有名字语义）。与 `build_expression` 的布局解析臂
/// 同款判定，零新文案。
fn resolve_layout_index(
    builder: &PlanBuilder,
    layout: &[(String, Vec<String>)],
    table: Option<&str>,
    column: &str,
) -> Result<Option<usize>, PlanError> {
    let col = column.to_lowercase();
    let mut offset = 0usize;
    let mut bare_hits: Vec<usize> = Vec::new();
    for (alias, seg) in layout {
        // T7（R9）：限定名与布局段键的匹配——原文相等或书写限定符归一化
        // 命中（隐式短名 `u` ↔ 声明段键 `o.u`）；显式别名段键即别名文本，
        // 原文相等分支先行命中，既有行为零变化。
        let alias_matches = table.map(|t| builder.qualifier_matches_declared(t, alias));
        for (pos, seg_col) in seg.iter().enumerate() {
            if seg_col.to_lowercase() == col && alias_matches.unwrap_or(true) {
                if alias_matches.is_some() {
                    // 限定名：首个同表同名命中即精确匹配
                    return Ok(Some(offset + pos));
                }
                bare_hits.push(offset + pos);
            }
        }
        offset += seg.len();
    }
    match bare_hits.len() {
        0 => Ok(None),
        1 => Ok(Some(bare_hits[0])),
        _ => Err(PlanError::AmbiguousColumn(column.to_string())),
    }
}

/// MS25 T8（design D9）：聚合函数读取的参数列表达式。`COUNT(*)` 无参 → `None`；
/// 其余聚合（`SUM/AVG/MIN/MAX/GROUP_CONCAT/COUNT`）取首个普通实参表达式，与
/// `extract_aggregate_func` 的同族语义一致。（change 2026-10-03-practical-core-gaps
/// T4：`pub(crate)` 供 `aggregate.rs` 提取-注册遍历共享。）
pub(crate) fn aggregate_arg_expr(func_expr: &Expr) -> Option<&Expr> {
    match func_expr {
        Expr::Function(f) => match f.args.first() {
            Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(e))) => Some(e),
            _ => None,
        },
        _ => None,
    }
}

/// MS19 T4a / MS25 T1（design D2）：SELECT 清单是否纯列引用（每个非子查询项都
/// 是裸标识符或两段限定标识符；通配项放行）。非列项的 `qualified_columns` 携带
/// 的是**输出名**（聚合 `sum_x`、函数表达式文本），不是基列——投影解析不得对
/// 它们做基形状列存在性校验。
fn select_list_is_pure_column_list(
    select: &sqlparser::ast::Select,
    subquery_item_indices: &[usize],
) -> bool {
    select.projection.iter().enumerate().all(|(idx, item)| {
        if subquery_item_indices.contains(&idx) {
            return true;
        }
        match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expr)
            | sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => is_plain_column_expr(expr),
            _ => true,
        }
    })
}

/// MS25 T1（design D2）：把 SELECT 清单解析为 JOIN 组合行形状上的投影索引。
///
/// 命中规则与 `build_from_clause_with_projection` 的 `output_columns` 过滤同
/// 谓词（限定名按 FROM 臂声明限定符匹配；非限定名命中任一表即入选——两表同名列按
/// 既有语义两列都入选），索引按组合形状的列序。`None` = 恒等（`SELECT *` / 空清单 /
/// 非纯列清单）。纯列清单中的列在组合形状上不存在时点名为 `ColumnNotFound`
/// （MS19 T5 同款模型：不静默产出与 SELECT 清单不符的行形状）。
/// design D4：`qualifiers` 与 `combined` 同序同长（声明限定符）。
fn resolve_join_projection_indices(
    builder: &PlanBuilder,
    combined: &[OutputColumn],
    qualifiers: &[String],
    qualified_columns: &[(Option<String>, String)],
    pure_column_list: bool,
) -> Result<Option<Vec<usize>>, PlanError> {
    if qualified_columns.is_empty() || qualified_columns.iter().any(|(_, c)| c == "*") {
        return Ok(None);
    }
    if pure_column_list {
        for (qual_table, qual_col) in qualified_columns {
            let hits = combined
                .iter()
                .enumerate()
                .filter(|(idx, col)| {
                    col.column.to_lowercase() == qual_col.to_lowercase()
                        && match qual_table {
                            Some(table) => qualifiers
                                .get(*idx)
                                .is_some_and(|q| builder.qualifier_matches_declared(table, q)),
                            None => true,
                        }
                })
                .count();
            if hits == 0 {
                return Err(PlanError::ColumnNotFound(qual_col.clone()));
            }
            // MS25 T8（design D9）：裸名在组合形状上多命中时，既有「两列都入选」
            // 语义产出与 SELECT 清单不符的重复列行——改为计划期点名歧义
            // （零宽行/重复列静默面收口），限定名逐字精确不受影响。
            if hits > 1 && qual_table.is_none() {
                return Err(PlanError::AmbiguousColumn(qual_col.clone()));
            }
        }
    }
    // T10（change 2026-10-04-external-review-gaps D10）：索引按 SELECT 清单序
    // 收集（对齐 Scan 基座语义）——逐 SELECT 项定位其命中组合列（限定名按
    // 声明限定符精确、裸名任意命中），重复项重复产出；纯列清单的命中/歧义
    // 已由上方校验保证，非纯清单的未命中项维持既有静默跳过。
    let mut indices = Vec::with_capacity(qualified_columns.len());
    for (qual_table, qual_col) in qualified_columns {
        if qual_col == "*" {
            continue;
        }
        if let Some((idx, _)) = combined.iter().enumerate().find(|(idx, col)| {
            col.column.to_lowercase() == qual_col.to_lowercase()
                && match qual_table {
                    Some(table) => qualifiers
                        .get(*idx)
                        .is_some_and(|q| builder.qualifier_matches_declared(table, q)),
                    None => true,
                }
        }) {
            indices.push(idx);
        }
    }
    Ok(Some(indices))
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
        | PhysicalPlan::IndexScanAll(_)
        // MS26 T8: the derived base emits full derived rows when its
        // projection is empty (the Sort node then owns the trim), and its
        // projection indices are resolved against the same derived column
        // set the Select list was resolved from.
        | PhysicalPlan::DerivedScan(_)
        // T4（change 2026-10-04-external-review-gaps）：标量谓词过滤节点——
        // 谓词在全形状行上求值，`projection` 空 = 全 schema 行流出，Sort
        // 承担裁剪（与 Filter 包裹链同契约）。
        | PhysicalPlan::ScalarSubqueryFilter(_) => true,
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
        Expr::CompoundIdentifier(parts) => matches!(parts.len(), 2 | 3),
        _ => false,
    }
}

/// change 2026-10-03-practical-core-gaps T4：纯常量项判定——不含列引用
/// （Identifier/CompoundIdentifier 出现即非纯常量）、子查询与 INTERVAL 的
/// 表达式（聚合调用由分类循环先行分流，不可达本判定）。聚合查询中的纯常量
/// 项与聚合混排合法（`Expr::TypedString` 日期字面量同为常量）；非聚合查询
/// 中本判定不被消费，顶层 Projection 行为零变化。
fn is_pure_constant_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) => false,
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => false,
        Expr::Interval(_) => false,
        Expr::BinaryOp { left, right, .. } => {
            is_pure_constant_expr(left) && is_pure_constant_expr(right)
        }
        Expr::UnaryOp { expr: inner, .. } => is_pure_constant_expr(inner),
        Expr::Nested(inner) => is_pure_constant_expr(inner),
        Expr::Cast { expr: inner, .. } => is_pure_constant_expr(inner),
        Expr::Case {
            operand,
            conditions,
            results,
            else_result,
        } => {
            operand.as_deref().is_none_or(is_pure_constant_expr)
                && conditions.iter().all(is_pure_constant_expr)
                && results.iter().all(is_pure_constant_expr)
                && else_result.as_deref().is_none_or(is_pure_constant_expr)
        }
        Expr::Function(f) => f.args.iter().all(|arg| match arg {
            sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(e)) => {
                is_pure_constant_expr(e)
            }
            _ => false,
        }),
        Expr::Trim {
            expr: inner,
            trim_what,
            ..
        } => {
            is_pure_constant_expr(inner)
                && trim_what
                    .as_deref()
                    .map(is_pure_constant_expr)
                    .unwrap_or(true)
        }
        Expr::Ceil { expr: inner, .. } | Expr::Floor { expr: inner, .. } => {
            is_pure_constant_expr(inner)
        }
        Expr::Value(_) | Expr::TypedString { .. } => true,
        _ => false,
    }
}

/// Resolve select-list column names to base-schema indices (projection order).
///
/// Returns `None` for the identity projection: an empty list or a wildcard
/// item. Identity keeps the pre-projection row shape byte-for-byte.
///
/// MS19 Iteration 002 (T4a): a name that is not a base column is no longer
/// folded into the identity bucket — it is a plan-time `ColumnNotFound`.
/// The bucket previously also served aliases and expression items, so an
/// unknown column silently widened the output to the full schema instead of
/// naming the offending column. Non-column items no longer reach this call:
/// the caller only resolves pure-column lists (see `pure_column_list`), and
/// alias items are routed to the top-level Projection beforehand.
fn resolve_projection_indices(
    projection: &[String],
    schema: &[String],
) -> Result<Option<Vec<usize>>, PlanError> {
    if projection.is_empty() {
        return Ok(None);
    }
    let mut indices = Vec::with_capacity(projection.len());
    for col in projection {
        if col == "*" {
            return Ok(None);
        }
        match schema
            .iter()
            .position(|c| c.to_lowercase() == col.to_lowercase())
        {
            Some(idx) => indices.push(idx),
            None => return Err(PlanError::ColumnNotFound(col.clone())),
        }
    }
    Ok(Some(indices))
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

/// T4（change 2026-10-04-external-review-gaps）：WHERE 表达式是否含标量
/// 子查询（`Expr::Subquery`）腿。遍历变体集与
/// `PlanBuilder::rewrite_scalar_subqueries`（subquery.rs）的改写面一致——
/// InSubquery/Exists 腿与 Query 体内不进入（既有机制/拒绝面承载）。
fn contains_scalar_subquery(expr: &Expr) -> bool {
    match expr {
        Expr::Subquery(_) => true,
        Expr::BinaryOp { left, right, .. } => {
            contains_scalar_subquery(left) || contains_scalar_subquery(right)
        }
        Expr::UnaryOp { expr, .. } => contains_scalar_subquery(expr),
        Expr::Nested(expr) => contains_scalar_subquery(expr),
        Expr::InList { expr, list, .. } => {
            contains_scalar_subquery(expr) || list.iter().any(contains_scalar_subquery)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            contains_scalar_subquery(expr)
                || contains_scalar_subquery(low)
                || contains_scalar_subquery(high)
        }
        Expr::Like { expr, pattern, .. } => {
            contains_scalar_subquery(expr) || contains_scalar_subquery(pattern)
        }
        Expr::IsNull(expr) | Expr::IsNotNull(expr) => contains_scalar_subquery(expr),
        Expr::Cast { expr, .. } => contains_scalar_subquery(expr),
        Expr::Case {
            operand,
            conditions,
            results,
            else_result,
        } => {
            operand
                .as_ref()
                .is_some_and(|e| contains_scalar_subquery(e))
                || conditions.iter().any(contains_scalar_subquery)
                || results.iter().any(contains_scalar_subquery)
                || else_result
                    .as_ref()
                    .is_some_and(|e| contains_scalar_subquery(e))
        }
        Expr::Function(func) => func.args.iter().any(|arg| match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => contains_scalar_subquery(e),
            FunctionArg::Named {
                arg: FunctionArgExpr::Expr(e),
                ..
            } => contains_scalar_subquery(e),
            _ => false,
        }),
        Expr::Trim {
            expr, trim_what, ..
        } => {
            contains_scalar_subquery(expr)
                || trim_what.as_deref().is_some_and(contains_scalar_subquery)
        }
        Expr::Ceil { expr, .. } | Expr::Floor { expr, .. } => contains_scalar_subquery(expr),
        _ => false,
    }
}

/// Extract column name from ORDER BY expression.
///
/// design D4：两段形态（`t.col`）解出限定符与列名——限定符由调用方按本语句的
/// FROM 作用域归一 / 点名，排序键取末段列名（与 `compare_rows` 的按名查找同型）。
/// 三段及以上与表达式项维持既有 `ORDER BY only supports column names` 拒绝。
fn extract_column_name(expr: &Expr) -> Result<(Option<String>, String), PlanError> {
    match expr {
        Expr::Identifier(ident) => Ok((None, ident.value.clone())),
        Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
            Ok((Some(parts[0].value.to_lowercase()), parts[1].value.clone()))
        }
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
