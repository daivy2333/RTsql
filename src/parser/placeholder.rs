//! 占位符索引分配与计数（MS31 Iteration 001 / T5）
//!
//! 裸 `?` 占位符按 **SQL 文本序** 编号（0 基），改写为 `__param_{i}` 标记；
//! 计数与编号由 **同一遍历** 产出（同源同序），参数注入不错位——计划面读标记
//! 里的序号，执行面按序号取参数值，两端不需要各自再遍历一次。
//!
//! 命名空间 `__param_{i}` 与相关外层引用占位（`{table}.{column}`）��标量子
//! 查询保留名（`__scalar_subquery_{i}`）无碰撞。
//!
//! 拒绝面：
//! - 非裸形态（`?N` / `$N` / `:name` / `@name`）——点名拒绝，一套位置式方言。
//! - DDL 语句内的占位符——点名拒绝（CTAS / 视图源 / 索引列位）。
//! - 标识位——维持 sqlparser 既有 parse 拒绝面（解析期即拒绝，不静默）。
//!
//! 已知边界：`Merge` / `Explain` / `Prepare` 等语句形态的占位符位置不在本模块
//! 的遍历面内——它们在计划期即被既有语句分派拒绝（`UnsupportedStatement`），
//! 参数化执行不可达，故不构成可达缺口。

use sqlparser::ast::{
    Expr, Function, FunctionArg, FunctionArgExpr, GroupByExpr, JoinConstraint, OrderByExpr, Query,
    Select, SelectItem, SetExpr, Statement, TableFactor, TableWithJoins,
};

use crate::parser::error::PlanError;

/// 占位符标记前缀。
pub const PARAM_PREFIX: &str = "__param_";

/// 取占位符标记的参数序号（`__param_{i}` → `Some(i)`）。
///
/// 非编号形态（裸 `?` / `?N` / `$N`）返回 `None`——调用方据此点名拒绝，
/// 不把未编号占位符静默按 NULL 处理。
pub fn placeholder_index(text: &str) -> Option<usize> {
    text.strip_prefix(PARAM_PREFIX)
        .and_then(|digits| digits.parse::<usize>().ok())
}

/// 取占位符标记的参数名（`__param_{i}` → `Some("__param_{i}")`）。
pub fn placeholder_param_name(text: &str) -> Option<&str> {
    if placeholder_index(text).is_some() {
        Some(text)
    } else {
        None
    }
}

/// 占位符形态错误文案。
///
/// 未编号的裸 `?`（到达计划面仍未绑定参数入口）与非裸形态分别点名——前者说
/// 明「裸形态只走参数化入口」，后者说明「只支持一套位置式方言」。
pub fn unsupported_placeholder_message(text: &str) -> String {
    if text == "?" {
        "unbound placeholder: bare '?' is only supported through the parameterized \
         execution entry point (execute_with_params)"
            .to_string()
    } else {
        format!("unsupported placeholder form '{text}': only bare '?' is supported")
    }
}

/// 值是否为占位符形态（裸 `?` 或已编号标记 `__param_{i}`）。
///
/// 计划面的「计划期无值」判定消费它——占位符位在计划期不可取字面量值
/// （定位键提取 / 索引路由 / 字面量腿判定）。
pub fn is_placeholder_value(value: &sqlparser::ast::Value) -> bool {
    matches!(value, sqlparser::ast::Value::Placeholder(_))
}

/// 语句内占位符个数（AST 级）。
///
/// 与 [`assign_placeholder_indices`] 同一遍历规则——计数与编号同源同序。
/// 计数含裸 `?` 与已编号标记 `__param_{i}`（任意形态均计入）。
pub fn count_placeholders(stmt: &Statement) -> usize {
    // 遍历需要可变借用以支持编号改写；计数模式下不改写——克隆一次走同一
    // 遍历规则，避免两条路径的顺序定义分叉。
    let mut probe = stmt.clone();
    let mut walker = Walker::new(false);
    // 计数模式不做形态拒绝（`?N` 等形态同样计入占位符总数）。
    let _ = walker.walk_statement(&mut probe);
    walker.count
}

/// 表达式内是否含占位符（裸 `?` 或已编号标记）。
///
/// 供计划面的「不可达位」判定消费（DML WHERE 定位键位）——与计数/编号同
/// 一遍历规则。
pub fn expr_has_placeholder(expr: &Expr) -> bool {
    let mut probe = expr.clone();
    let mut walker = Walker::new(false);
    walker.walk_expr(&mut probe).is_ok() && walker.count > 0
}

/// 占位符索引分配：裸 `?` 改写为 `__param_{i}`（SQL 文本序），返回占位符总数。
///
/// - 编号与计数同源同序（同一遍历），计划面读标记序号、执行面按序号取参。
/// - 已编号标记（`__param_{i}`）保持原样——重复调用幂等。
/// - 非裸形态点名拒绝；DDL 语句内的占位符点名拒绝。
pub fn assign_placeholder_indices(stmt: &mut Statement) -> Result<usize, PlanError> {
    let mut walker = Walker::new(true);
    walker.walk_statement(stmt)?;
    Ok(walker.count)
}

/// 占位符遍历器。
struct Walker {
    /// 计数模式（不改写、不做形态拒绝）/ 分配模式。
    assign: bool,
    /// 当前语句是 DDL——命中占位符即拒绝。
    ddl: bool,
    /// 下一个待分配序号（分配模式）。
    next_index: usize,
    /// 已见占位符个数。
    count: usize,
}

impl Walker {
    fn new(assign: bool) -> Self {
        Self {
            assign,
            ddl: false,
            next_index: 0,
            count: 0,
        }
    }

    /// 处理一个 sqlparser 值：占位符计数 / 编号 / 形态拒绝。
    fn handle_value(&mut self, value: &mut sqlparser::ast::Value) -> Result<(), PlanError> {
        let sqlparser::ast::Value::Placeholder(text) = value else {
            return Ok(());
        };
        if self.ddl {
            return Err(PlanError::ParseError(
                "placeholders are not supported in DDL statements".to_string(),
            ));
        }
        if text.starts_with(PARAM_PREFIX) {
            // 已编号标记：计入总数，不重新编号（幂等）。
            self.count += 1;
            return Ok(());
        }
        if text != "?" {
            if self.assign {
                return Err(PlanError::ParseError(format!(
                    "unsupported placeholder form '{text}': only bare '?' is supported"
                )));
            }
            // 计数模式：非裸形态同样计入占位符总数（形态拒绝由分配模式承担）。
            self.count += 1;
            return Ok(());
        }
        self.count += 1;
        if self.assign {
            *text = format!("{PARAM_PREFIX}{}", self.next_index);
            self.next_index += 1;
        }
        Ok(())
    }

    fn walk_statement(&mut self, stmt: &mut Statement) -> Result<(), PlanError> {
        match stmt {
            Statement::Query(query) => self.walk_query(query),
            Statement::Insert {
                source,
                partitioned,
                returning,
                on,
                ..
            } => {
                if let Some(source) = source {
                    self.walk_query(source)?;
                }
                for expr in partitioned.iter_mut().flatten() {
                    self.walk_expr(expr)?;
                }
                // ON 冲突子句（`ON CONFLICT ... DO UPDATE SET` / MySQL
                // `ON DUPLICATE KEY UPDATE`）的赋值位与过滤位——文本序在
                // 待插值位之后。
                if let Some(on) = on {
                    self.walk_on_insert(on)?;
                }
                if let Some(returning) = returning {
                    self.walk_select_items(returning)?;
                }
                Ok(())
            }
            Statement::Update {
                table,
                assignments,
                from,
                selection,
                returning,
            } => {
                self.walk_table_with_joins(table)?;
                for assignment in assignments {
                    self.walk_expr(&mut assignment.value)?;
                }
                if let Some(from) = from {
                    self.walk_table_with_joins(from)?;
                }
                if let Some(selection) = selection {
                    self.walk_expr(selection)?;
                }
                if let Some(returning) = returning {
                    self.walk_select_items(returning)?;
                }
                Ok(())
            }
            Statement::Delete {
                from,
                using,
                selection,
                returning,
                order_by,
                limit,
                ..
            } => {
                self.walk_from_table(from)?;
                if let Some(using) = using {
                    for tables in using {
                        self.walk_table_with_joins(tables)?;
                    }
                }
                if let Some(selection) = selection {
                    self.walk_expr(selection)?;
                }
                if let Some(returning) = returning {
                    self.walk_select_items(returning)?;
                }
                self.walk_order_by(order_by)?;
                if let Some(limit) = limit {
                    self.walk_expr(limit)?;
                }
                Ok(())
            }
            // CTAS / 视图：DDL 语句内的占位符点名拒绝（源查询位置仍走遍历，
            // 命中即拒绝——不编号）。
            Statement::CreateTable { query, .. } => {
                self.ddl = true;
                if let Some(query) = query {
                    self.walk_query(query)?;
                }
                Ok(())
            }
            Statement::CreateView { query, .. } => {
                self.ddl = true;
                self.walk_query(query)?;
                Ok(())
            }
            // 索引列位：DDL 内的占位符点名拒绝。
            Statement::CreateIndex { columns, .. } => {
                self.ddl = true;
                for column in columns {
                    self.walk_expr(&mut column.expr)?;
                }
                Ok(())
            }
            // 其余语句形态无可达占位符位置（见模块头「已知边界」）。
            _ => Ok(()),
        }
    }

    /// ON 冲突子句遍历——`DO UPDATE SET` 赋值位与过滤位。
    fn walk_on_insert(&mut self, on: &mut sqlparser::ast::OnInsert) -> Result<(), PlanError> {
        match on {
            sqlparser::ast::OnInsert::DuplicateKeyUpdate(assignments) => {
                for assignment in assignments.iter_mut() {
                    self.walk_expr(&mut assignment.value)?;
                }
                Ok(())
            }
            sqlparser::ast::OnInsert::OnConflict(conflict) => {
                if let sqlparser::ast::OnConflictAction::DoUpdate(do_update) = &mut conflict.action
                {
                    for assignment in do_update.assignments.iter_mut() {
                        self.walk_expr(&mut assignment.value)?;
                    }
                    if let Some(selection) = do_update.selection.as_mut() {
                        self.walk_expr(selection)?;
                    }
                }
                Ok(())
            }
            // `OnInsert` 是 non_exhaustive 枚举——未来新增形态按「无可达占位符
            // 位置」处理。
            _ => Ok(()),
        }
    }

    fn walk_from_table(&mut self, from: &mut sqlparser::ast::FromTable) -> Result<(), PlanError> {
        match from {
            sqlparser::ast::FromTable::WithFromKeyword(tables)
            | sqlparser::ast::FromTable::WithoutKeyword(tables) => {
                for table in tables {
                    self.walk_table_with_joins(table)?;
                }
                Ok(())
            }
        }
    }

    fn walk_query(&mut self, query: &mut Query) -> Result<(), PlanError> {
        // 文本序：WITH（CTE 定义）→ 主体 → ORDER BY → LIMIT → LIMIT BY →
        // OFFSET → FETCH。
        if let Some(with) = query.with.as_mut() {
            for cte in with.cte_tables.iter_mut() {
                self.walk_query(&mut cte.query)?;
            }
        }
        self.walk_set_expr(&mut query.body)?;
        self.walk_order_by(&mut query.order_by)?;
        if let Some(limit) = query.limit.as_mut() {
            self.walk_expr(limit)?;
        }
        for expr in query.limit_by.iter_mut() {
            self.walk_expr(expr)?;
        }
        if let Some(offset) = query.offset.as_mut() {
            self.walk_expr(&mut offset.value)?;
        }
        if let Some(fetch) = query.fetch.as_mut() {
            if let Some(quantity) = fetch.quantity.as_mut() {
                self.walk_expr(quantity)?;
            }
        }
        Ok(())
    }

    fn walk_set_expr(&mut self, body: &mut SetExpr) -> Result<(), PlanError> {
        match body {
            SetExpr::Select(select) => self.walk_select(select),
            SetExpr::Query(query) => self.walk_query(query),
            SetExpr::SetOperation { left, right, .. } => {
                self.walk_set_expr(left)?;
                self.walk_set_expr(right)?;
                Ok(())
            }
            SetExpr::Values(values) => {
                for row in values.rows.iter_mut() {
                    for expr in row.iter_mut() {
                        self.walk_expr(expr)?;
                    }
                }
                Ok(())
            }
            SetExpr::Insert(stmt) => self.walk_statement(stmt),
            SetExpr::Update(stmt) => self.walk_statement(stmt),
            SetExpr::Table(_) => Ok(()),
        }
    }

    fn walk_select(&mut self, select: &mut Select) -> Result<(), PlanError> {
        // 文本序：投影 → FROM（JOIN ON）→ LATERAL VIEW → WHERE → GROUP BY →
        // HAVING → WINDOW → QUALIFY → CLUSTER/DISTRIBUTE/SORT BY。
        self.walk_select_items(&mut select.projection)?;
        for table in select.from.iter_mut() {
            self.walk_table_with_joins(table)?;
        }
        for lateral in select.lateral_views.iter_mut() {
            self.walk_expr(&mut lateral.lateral_view)?;
        }
        if let Some(selection) = select.selection.as_mut() {
            self.walk_expr(selection)?;
        }
        if let GroupByExpr::Expressions(exprs) = &mut select.group_by {
            for expr in exprs.iter_mut() {
                self.walk_expr(expr)?;
            }
        }
        if let Some(having) = select.having.as_mut() {
            self.walk_expr(having)?;
        }
        for window in select.named_window.iter_mut() {
            for expr in window.1.partition_by.iter_mut() {
                self.walk_expr(expr)?;
            }
            self.walk_order_by(&mut window.1.order_by)?;
        }
        if let Some(qualify) = select.qualify.as_mut() {
            self.walk_expr(qualify)?;
        }
        for expr in select.cluster_by.iter_mut() {
            self.walk_expr(expr)?;
        }
        for expr in select.distribute_by.iter_mut() {
            self.walk_expr(expr)?;
        }
        for expr in select.sort_by.iter_mut() {
            self.walk_expr(expr)?;
        }
        Ok(())
    }

    fn walk_select_items(&mut self, items: &mut [SelectItem]) -> Result<(), PlanError> {
        for item in items.iter_mut() {
            match item {
                SelectItem::UnnamedExpr(expr) => self.walk_expr(expr)?,
                SelectItem::ExprWithAlias { expr, .. } => self.walk_expr(expr)?,
                SelectItem::QualifiedWildcard(_, _) | SelectItem::Wildcard(_) => {}
            }
        }
        Ok(())
    }

    fn walk_order_by(&mut self, items: &mut [OrderByExpr]) -> Result<(), PlanError> {
        for item in items.iter_mut() {
            self.walk_expr(&mut item.expr)?;
        }
        Ok(())
    }

    fn walk_table_with_joins(&mut self, table: &mut TableWithJoins) -> Result<(), PlanError> {
        self.walk_table_factor(&mut table.relation)?;
        for join in table.joins.iter_mut() {
            self.walk_table_factor(&mut join.relation)?;
            self.walk_join_operator(&mut join.join_operator)?;
        }
        Ok(())
    }

    /// JOIN 运算符携带的约束（`ON` 表达式位）。
    fn walk_join_operator(
        &mut self,
        operator: &mut sqlparser::ast::JoinOperator,
    ) -> Result<(), PlanError> {
        use sqlparser::ast::JoinOperator as Op;
        match operator {
            Op::Inner(constraint)
            | Op::LeftOuter(constraint)
            | Op::RightOuter(constraint)
            | Op::FullOuter(constraint)
            | Op::LeftSemi(constraint)
            | Op::RightSemi(constraint)
            | Op::LeftAnti(constraint)
            | Op::RightAnti(constraint) => self.walk_join_constraint(constraint),
            _ => Ok(()),
        }
    }

    fn walk_join_constraint(&mut self, constraint: &mut JoinConstraint) -> Result<(), PlanError> {
        match constraint {
            JoinConstraint::On(expr) => self.walk_expr(expr),
            JoinConstraint::Using(_) | JoinConstraint::Natural | JoinConstraint::None => Ok(()),
        }
    }

    fn walk_table_factor(&mut self, factor: &mut TableFactor) -> Result<(), PlanError> {
        match factor {
            TableFactor::Table {
                args, with_hints, ..
            } => {
                if let Some(args) = args {
                    self.walk_function_args(args)?;
                }
                for hint in with_hints.iter_mut() {
                    self.walk_expr(hint)?;
                }
                Ok(())
            }
            TableFactor::Derived { subquery, .. } => self.walk_query(subquery),
            TableFactor::TableFunction { expr, .. } => self.walk_expr(expr),
            TableFactor::Function { args, .. } => self.walk_function_args(args),
            TableFactor::UNNEST { array_exprs, .. } => {
                for expr in array_exprs.iter_mut() {
                    self.walk_expr(expr)?;
                }
                Ok(())
            }
            TableFactor::NestedJoin {
                table_with_joins, ..
            } => self.walk_table_with_joins(table_with_joins),
            TableFactor::Pivot {
                table,
                aggregate_function,
                pivot_values,
                ..
            } => {
                self.walk_table_factor(table)?;
                self.walk_expr(aggregate_function)?;
                for value in pivot_values.iter_mut() {
                    self.handle_value(value)?;
                }
                Ok(())
            }
            TableFactor::Unpivot { table, .. } => self.walk_table_factor(table),
            TableFactor::JsonTable {
                json_expr,
                json_path,
                ..
            } => {
                self.walk_expr(json_expr)?;
                self.handle_value(json_path)?;
                Ok(())
            }
        }
    }

    fn walk_function_args(&mut self, args: &mut [FunctionArg]) -> Result<(), PlanError> {
        for arg in args.iter_mut() {
            match arg {
                FunctionArg::Named { arg, .. } => {
                    if let FunctionArgExpr::Expr(expr) = arg {
                        self.walk_expr(expr)?;
                    }
                }
                FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self.walk_expr(expr)?,
                FunctionArg::Unnamed(_) => {}
            }
        }
        Ok(())
    }

    fn walk_function(&mut self, function: &mut Function) -> Result<(), PlanError> {
        self.walk_function_args(&mut function.args)?;
        if let Some(filter) = function.filter.as_mut() {
            self.walk_expr(filter)?;
        }
        self.walk_order_by(&mut function.order_by)?;
        Ok(())
    }

    /// 表达式遍历——覆盖 sqlparser `Expr` 的全部形态（编译器保证穷尽）。
    fn walk_expr(&mut self, expr: &mut Expr) -> Result<(), PlanError> {
        match expr {
            // 叶子形态：无可达子表达式（`Value` 单独处理）。
            Expr::Identifier(_)
            | Expr::CompoundIdentifier(_)
            | Expr::Wildcard
            | Expr::QualifiedWildcard(_)
            | Expr::TypedString { .. }
            | Expr::Interval(_) => Ok(()),
            Expr::Value(value) => self.handle_value(value),
            Expr::IntroducedString { value, .. } => self.handle_value(value),
            Expr::MatchAgainst { match_value, .. } => self.handle_value(match_value),
            Expr::JsonAccess { left, right, .. } => {
                self.walk_expr(left)?;
                self.walk_expr(right)?;
                Ok(())
            }
            Expr::CompositeAccess { expr, .. } => self.walk_expr(expr),
            Expr::IsFalse(inner)
            | Expr::IsNotFalse(inner)
            | Expr::IsTrue(inner)
            | Expr::IsNotTrue(inner)
            | Expr::IsNull(inner)
            | Expr::IsNotNull(inner)
            | Expr::IsUnknown(inner)
            | Expr::IsNotUnknown(inner)
            | Expr::OuterJoin(inner)
            | Expr::Nested(inner) => self.walk_expr(inner),
            Expr::IsDistinctFrom(left, right) | Expr::IsNotDistinctFrom(left, right) => {
                self.walk_expr(left)?;
                self.walk_expr(right)?;
                Ok(())
            }
            Expr::InList { expr, list, .. } => {
                self.walk_expr(expr)?;
                for item in list.iter_mut() {
                    self.walk_expr(item)?;
                }
                Ok(())
            }
            Expr::InSubquery { expr, subquery, .. } => {
                self.walk_expr(expr)?;
                self.walk_query(subquery)?;
                Ok(())
            }
            Expr::InUnnest {
                expr, array_expr, ..
            } => {
                self.walk_expr(expr)?;
                self.walk_expr(array_expr)?;
                Ok(())
            }
            Expr::Between {
                expr, low, high, ..
            } => {
                self.walk_expr(expr)?;
                self.walk_expr(low)?;
                self.walk_expr(high)?;
                Ok(())
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk_expr(left)?;
                self.walk_expr(right)?;
                Ok(())
            }
            Expr::Like { expr, pattern, .. }
            | Expr::ILike { expr, pattern, .. }
            | Expr::SimilarTo { expr, pattern, .. } => {
                self.walk_expr(expr)?;
                self.walk_expr(pattern)?;
                Ok(())
            }
            Expr::RLike { expr, .. } => self.walk_expr(expr),
            Expr::AnyOp { left, right, .. } | Expr::AllOp { left, right, .. } => {
                self.walk_expr(left)?;
                self.walk_expr(right)?;
                Ok(())
            }
            Expr::UnaryOp { expr, .. } => self.walk_expr(expr),
            Expr::Convert { expr, .. } => self.walk_expr(expr),
            Expr::Cast { expr, .. } | Expr::TryCast { expr, .. } | Expr::SafeCast { expr, .. } => {
                self.walk_expr(expr)?;
                Ok(())
            }
            Expr::AtTimeZone { timestamp, .. } => self.walk_expr(timestamp),
            Expr::Extract { expr, .. } => self.walk_expr(expr),
            Expr::Ceil { expr, .. } | Expr::Floor { expr, .. } => self.walk_expr(expr),
            Expr::Position { expr, r#in } => {
                self.walk_expr(expr)?;
                self.walk_expr(r#in)?;
                Ok(())
            }
            Expr::Substring {
                expr,
                substring_from,
                substring_for,
                ..
            } => {
                self.walk_expr(expr)?;
                if let Some(from) = substring_from.as_mut() {
                    self.walk_expr(from)?;
                }
                if let Some(for_) = substring_for.as_mut() {
                    self.walk_expr(for_)?;
                }
                Ok(())
            }
            Expr::Trim {
                expr,
                trim_what,
                trim_characters,
                ..
            } => {
                self.walk_expr(expr)?;
                if let Some(what) = trim_what.as_mut() {
                    self.walk_expr(what)?;
                }
                if let Some(chars) = trim_characters.as_mut() {
                    for char in chars.iter_mut() {
                        self.walk_expr(char)?;
                    }
                }
                Ok(())
            }
            Expr::Overlay {
                expr,
                overlay_what,
                overlay_from,
                overlay_for,
                ..
            } => {
                self.walk_expr(expr)?;
                self.walk_expr(overlay_what)?;
                self.walk_expr(overlay_from)?;
                if let Some(for_) = overlay_for.as_mut() {
                    self.walk_expr(for_)?;
                }
                Ok(())
            }
            Expr::Collate { expr, .. } => self.walk_expr(expr),
            Expr::MapAccess { column, keys } => {
                self.walk_expr(column)?;
                for key in keys.iter_mut() {
                    self.walk_expr(key)?;
                }
                Ok(())
            }
            Expr::Function(function) => self.walk_function(function),
            Expr::AggregateExpressionWithFilter { expr, filter } => {
                self.walk_expr(expr)?;
                self.walk_expr(filter)?;
                Ok(())
            }
            Expr::Case {
                operand,
                conditions,
                results,
                else_result,
            } => {
                // 文本序：operand → (condition_i, result_i) 逐对 → else_result。
                if let Some(operand) = operand.as_mut() {
                    self.walk_expr(operand)?;
                }
                for (index, condition) in conditions.iter_mut().enumerate() {
                    self.walk_expr(condition)?;
                    if let Some(result) = results.get_mut(index) {
                        self.walk_expr(result)?;
                    }
                }
                if let Some(else_result) = else_result.as_mut() {
                    self.walk_expr(else_result)?;
                }
                Ok(())
            }
            Expr::Exists { subquery, .. } => self.walk_query(subquery),
            Expr::Subquery(query) | Expr::ArraySubquery(query) => self.walk_query(query),
            Expr::ListAgg(agg) => {
                self.walk_expr(&mut agg.expr)?;
                if let Some(separator) = agg.separator.as_mut() {
                    self.walk_expr(separator)?;
                }
                self.walk_order_by(&mut agg.within_group)?;
                Ok(())
            }
            Expr::ArrayAgg(agg) => {
                self.walk_expr(&mut agg.expr)?;
                if let Some(order_by) = agg.order_by.as_mut() {
                    self.walk_order_by(order_by)?;
                }
                if let Some(limit) = agg.limit.as_mut() {
                    self.walk_expr(limit)?;
                }
                Ok(())
            }
            Expr::GroupingSets(groups) => {
                for group in groups.iter_mut() {
                    for expr in group.iter_mut() {
                        self.walk_expr(expr)?;
                    }
                }
                Ok(())
            }
            Expr::Cube(groups) | Expr::Rollup(groups) => {
                for group in groups.iter_mut() {
                    for expr in group.iter_mut() {
                        self.walk_expr(expr)?;
                    }
                }
                Ok(())
            }
            Expr::Tuple(exprs) => {
                for expr in exprs.iter_mut() {
                    self.walk_expr(expr)?;
                }
                Ok(())
            }
            Expr::ArrayIndex { obj, indexes } => {
                self.walk_expr(obj)?;
                for index in indexes.iter_mut() {
                    self.walk_expr(index)?;
                }
                Ok(())
            }
            Expr::Struct { values, .. } => {
                for value in values.iter_mut() {
                    self.walk_expr(value)?;
                }
                Ok(())
            }
            Expr::Named { expr, .. } => self.walk_expr(expr),
            Expr::Array(array) => {
                for elem in array.elem.iter_mut() {
                    self.walk_expr(elem)?;
                }
                Ok(())
            }
        }
    }
}
