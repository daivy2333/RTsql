//! MS25 T6：EXPLAIN 计划树渲染（`render_plan_tree` 纯函数）
//!
//! 逐变体一行「节点名 + 关键参数」，根在前、缩进表达深度（每层两个空格）。
//! 参数全部为节点字段的确定性直出（表名、键字节的十六进制、谓词的 `Debug`
//! 投影、列清单、JOIN 类型、排序键、LIMIT 值）——同一计划两次渲染逐字节一致，
//! 不引入任何身份型机制。
//!
//! 覆盖 `PhysicalPlan` 全部变体（含 DML/DDL 变体——它们在计划期已被 EXPLAIN
//! 点名拒绝，本渲染面对不可达变体同样给出确定文本，保证新增变体时编译器
//! 强制补齐渲染臂）。

use crate::executor::aggregate::AggregateFunc;
use crate::executor::plan::{
    ConflictAction, JoinCondition, NestedLoopJoinType, OrderByColumn, PhysicalPlan, SetOpKind,
};
use crate::executor::{AlterTableOp, UpdateValue, UpsertValueExpr};
use crate::storage::page_format::Key;

/// 渲染整棵计划树：首行恒为 `EXPLAIN` 根标识，其后每行对应一个计划节点。
pub fn render_plan_tree(plan: &PhysicalPlan) -> Vec<String> {
    let mut lines = vec!["EXPLAIN".to_string()];
    render_node(plan, 1, &mut lines);
    lines
}

fn render_node(plan: &PhysicalPlan, depth: usize, out: &mut Vec<String>) {
    out.push(format!("{}{}", "  ".repeat(depth), describe(plan)));
    // 递归下探（每变体至多一个子输入；JOIN/SemiJoin/AntiJoin/SetOperation 两臂）
    match plan {
        PhysicalPlan::Filter(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Sort(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Limit(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Aggregate(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Having(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Projection(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Distinct(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::SubqueryEval(node) => {
            render_node(&node.input, depth + 1, out);
            render_node(&node.subquery, depth + 1, out);
        }
        // T4（change 2026-10-04-external-review-gaps）：标量谓词过滤节点——
        // 输入链与各绑定子查询逐一下探（绑定计划是可执行子树）。
        PhysicalPlan::ScalarSubqueryFilter(node) => {
            render_node(&node.input, depth + 1, out);
            for binding in &node.bindings {
                render_node(&binding.subquery, depth + 1, out);
            }
        }
        PhysicalPlan::DerivedScan(node) => render_node(&node.subquery, depth + 1, out),
        // MS20 T4: CTAS 的物化源作为子树下探（表头与行宽描述在 CTAS 行本身）
        PhysicalPlan::CreateTableAsSelect(node) => render_node(&node.select, depth + 1, out),
        // MS20-T02 Iter001：INSERT SELECT 的源 SELECT 子树下探（目标映射在
        // InsertSelect 行本身）。
        PhysicalPlan::InsertSelect(node) => render_node(&node.source, depth + 1, out),
        PhysicalPlan::Explain(node) => render_node(&node.input, depth + 1, out),
        PhysicalPlan::Join(node) => {
            render_node(&node.left, depth + 1, out);
            render_node(&node.right, depth + 1, out);
        }
        PhysicalPlan::NestedLoopJoin(node) => {
            render_node(&node.left, depth + 1, out);
            render_node(&node.right, depth + 1, out);
        }
        PhysicalPlan::SemiJoin(node) => {
            render_node(&node.left, depth + 1, out);
            render_node(&node.right, depth + 1, out);
        }
        PhysicalPlan::AntiJoin(node) => {
            render_node(&node.left, depth + 1, out);
            render_node(&node.right, depth + 1, out);
        }
        PhysicalPlan::SetOperation(node) => {
            render_node(&node.left, depth + 1, out);
            render_node(&node.right, depth + 1, out);
        }
        PhysicalPlan::Scan(_)
        | PhysicalPlan::DataScan(_)
        | PhysicalPlan::IndexScan(_)
        | PhysicalPlan::IndexScanAll(_)
        | PhysicalPlan::Insert(_)
        | PhysicalPlan::Upsert(_)
        | PhysicalPlan::Update(_)
        | PhysicalPlan::Delete(_)
        | PhysicalPlan::CreateTable(_)
        | PhysicalPlan::CreateIndex(_)
        | PhysicalPlan::DropIndex(_)
        | PhysicalPlan::AlterTable(_)
        | PhysicalPlan::Truncate(_)
        | PhysicalPlan::CreateView(_)
        | PhysicalPlan::DropView(_)
        | PhysicalPlan::DropTable(_)
        | PhysicalPlan::SingleRow => {}
    }
}

/// 单个节点的一行描述：节点名 + 关键参数（括号内为参数列表）。
fn describe(plan: &PhysicalPlan) -> String {
    match plan {
        PhysicalPlan::Scan(node) => {
            format!(
                "Scan {} ({})",
                node.table_name,
                columns(&node.columns, &node.projection)
            )
        }
        PhysicalPlan::DataScan(node) => {
            let mut params = vec![format!(
                "columns={}",
                columns(&node.columns, &node.projection)
            )];
            if let Some(predicate) = &node.predicate {
                params.push(format!("predicate={:?}", predicate));
            }
            if let Some(cap) = node.scan_cap {
                params.push(format!("scan_cap={}", cap));
            }
            format!("DataScan {} ({})", node.table_name, params.join(", "))
        }
        // MS15-Rest (I034): IndexScan 的 `columns` 在构造期已收窄为投影形状，
        // 其 `projection` 索引指向基表 schema——此处按既有渲染口径直出
        // `columns`（与 `get_plan_output_columns` 的 IndexScan 臂同款）。
        PhysicalPlan::IndexScan(node) => format!(
            "IndexScan {} (key={}, columns=[{}])",
            node.table_name,
            key_hex(&node.key),
            node.columns.join(", ")
        ),
        // IndexScanAll 相反：`columns` 是基表列清单，真实输出形状由 `projection`
        // 决定（与 `get_plan_output_columns` 的 IndexScanAll 臂同款裁剪）。
        // MS21 Iter002 (T2-g)：二级索引路由命中时点名索引（EXPLAIN 可观察）；
        // None（既有路径）保持渲染逐字节不变。
        PhysicalPlan::IndexScanAll(node) => {
            let mut params = vec![format!("key={}", key_hex(&node.key))];
            if let Some(index_name) = &node.index_name {
                params.push(format!("index={}", index_name));
            }
            params.push(format!(
                "columns={}",
                columns(&node.columns, &node.projection)
            ));
            format!("IndexScanAll {} ({})", node.table_name, params.join(", "))
        }
        PhysicalPlan::Filter(node) => format!(
            "Filter {} (predicate={:?})",
            node.table_name, node.predicate
        ),
        PhysicalPlan::Insert(node) => format!(
            "Insert {} (rows={}, columns=[{}])",
            node.table_name,
            node.values.len(),
            node.columns.join(", ")
        ),
        PhysicalPlan::Upsert(node) => {
            let mut params = vec![format!("rows={}", node.values.len())];
            params.push(match &node.action {
                ConflictAction::DoNothing => "action=DO NOTHING".to_string(),
                ConflictAction::DoUpdate(assignments) => {
                    let rendered: Vec<String> = assignments
                        .iter()
                        .map(|a| {
                            let right = match &a.expr {
                                UpsertValueExpr::Literal(v) => format!("{:?}", v),
                                UpsertValueExpr::Excluded(i) => format!("excluded[{}]", i),
                                UpsertValueExpr::Old(i) => format!("old[{}]", i),
                            };
                            format!("col[{}]={}", a.column, right)
                        })
                        .collect();
                    format!("action=DO UPDATE ({})", rendered.join(", "))
                }
                ConflictAction::Replace => "action=REPLACE".to_string(),
            });
            format!("Upsert {} ({})", node.table_name, params.join(", "))
        }
        // MS20-T02 Iter001：INSERT SELECT——确定性直出列清单/源宽度/冲突动作
        //（DO UPDATE 只报赋值个数；仲裁目标与 Upsert 臂同口径不展开）。
        PhysicalPlan::InsertSelect(node) => {
            let mut params = vec![format!("columns=[{}]", node.columns.join(", "))];
            params.push(format!("source_width={}", node.source_width));
            if let Some((_, action)) = &node.upsert {
                params.push(match action {
                    ConflictAction::DoNothing => "action=DO NOTHING".to_string(),
                    ConflictAction::DoUpdate(assignments) => {
                        format!("action=DO UPDATE ({} assignments)", assignments.len())
                    }
                    ConflictAction::Replace => "action=REPLACE".to_string(),
                });
            }
            format!("InsertSelect {} ({})", node.table_name, params.join(", "))
        }
        PhysicalPlan::Update(node) => {
            // change 2026-10-03-practical-core-gaps T8：多列赋值渲染——字面量
            // 显示 SQL 形态，表达式显示其 Debug 形态。
            let sets = node
                .assignments
                .iter()
                .map(|a| match &a.value {
                    UpdateValue::Literal(v) => format!("{}={:?}", a.column, v),
                    UpdateValue::Expression(e) => format!("{}={:?}", a.column, e),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Update {} (key={}, set {})",
                node.table_name,
                // T6：全表形态（无 WHERE）渲染 `key=-`。
                node.key.as_ref().map(key_hex).as_deref().unwrap_or("-"),
                sets
            )
        }
        PhysicalPlan::Delete(node) => {
            format!(
                "Delete {} (key={})",
                node.table_name,
                // T6：全表形态（无 WHERE）渲染 `key=-`。
                node.key.as_ref().map(key_hex).as_deref().unwrap_or("-")
            )
        }
        PhysicalPlan::CreateTable(node) => format!(
            "CreateTable {} (columns={}, primary_key={})",
            node.table_name,
            node.columns.len(),
            node.primary_key.as_deref().unwrap_or("-")
        ),
        PhysicalPlan::CreateTableAsSelect(node) => format!(
            "CreateTableAsSelect {} (columns=[{}])",
            node.table_name,
            node.columns.join(", ")
        ),
        PhysicalPlan::CreateIndex(node) => format!(
            "CreateIndex {} (on {}({}))",
            node.index_name, node.table_name, node.column_name
        ),
        PhysicalPlan::DropIndex(node) => {
            format!("DropIndex {}", node.index_name)
        }
        PhysicalPlan::CreateView(node) => format!(
            "CreateView {} (columns=[{}])",
            node.view_name,
            node.columns.join(", ")
        ),
        PhysicalPlan::DropView(node) => {
            format!("DropView {}", node.view_name)
        }
        PhysicalPlan::AlterTable(node) => match &node.operation {
            AlterTableOp::AddColumn {
                column_name,
                column_type,
                ..
            } => format!(
                "AlterTable {} (ADD COLUMN {} {:?})",
                node.table_name, column_name, column_type
            ),
            AlterTableOp::DropColumn { column_name } => {
                format!(
                    "AlterTable {} (DROP COLUMN {})",
                    node.table_name, column_name
                )
            }
        },
        PhysicalPlan::DropTable(node) => format!(
            "DropTable {} (if_exists={})",
            node.table_name, node.if_exists
        ),
        PhysicalPlan::Truncate(node) => {
            format!("Truncate {}", node.table_name)
        }
        PhysicalPlan::Sort(node) => {
            let keys: Vec<String> = node
                .order_by
                .iter()
                .map(|OrderByColumn { column, asc, .. }| {
                    format!("{} {}", column, if *asc { "ASC" } else { "DESC" })
                })
                .collect();
            format!(
                "Sort (order_by=[{}], columns={})",
                keys.join(", "),
                columns(&node.columns, &node.projection)
            )
        }
        PhysicalPlan::Limit(node) => {
            format!("Limit (limit={}, offset={})", node.limit, node.offset)
        }
        PhysicalPlan::Join(node) => {
            let conditions: Vec<String> = node.conditions.iter().map(join_condition_text).collect();
            format!("Join [Inner] (on=[{}])", conditions.join(", "))
        }
        PhysicalPlan::NestedLoopJoin(node) => format!(
            "NestedLoopJoin [{}] (on={:?})",
            match node.join_type {
                NestedLoopJoinType::Inner => "Inner",
                NestedLoopJoinType::Left => "Left",
                NestedLoopJoinType::Full => "Full",
            },
            node.predicate
        ),
        PhysicalPlan::Aggregate(node) => {
            let aggregates: Vec<String> = node.aggregates.iter().map(aggregate_text).collect();
            format!(
                "Aggregate {} (group_by=[{}], aggregates=[{}])",
                node.table_name,
                node.group_by.join(", "),
                aggregates.join(", ")
            )
        }
        PhysicalPlan::Having(node) => format!(
            "Having {} (predicate={:?})",
            node.table_name, node.predicate
        ),
        PhysicalPlan::SemiJoin(node) => {
            let conditions: Vec<String> = node.conditions.iter().map(join_condition_text).collect();
            format!(
                "SemiJoin (on=[{}], correlated_params={})",
                conditions.join(", "),
                node.correlated_params.len()
            )
        }
        PhysicalPlan::AntiJoin(node) => {
            let conditions: Vec<String> = node.conditions.iter().map(join_condition_text).collect();
            format!(
                "AntiJoin (on=[{}], correlated_params={})",
                conditions.join(", "),
                node.correlated_params.len()
            )
        }
        PhysicalPlan::SubqueryEval(node) => format!(
            "SubqueryEval (column={}, correlated_params={})",
            node.output_column,
            node.correlated_params.len()
        ),
        PhysicalPlan::ScalarSubqueryFilter(node) => format!(
            "ScalarSubqueryFilter {} (bindings={}, correlated_params={}, columns={})",
            node.table_name,
            node.bindings.len(),
            node.bindings
                .iter()
                .map(|b| b.correlated_params.len())
                .sum::<usize>(),
            columns(&node.columns, &[])
        ),
        PhysicalPlan::DerivedScan(node) => format!(
            "DerivedScan {} (columns=[{}])",
            node.alias,
            node.columns.join(", ")
        ),
        PhysicalPlan::Projection(node) => {
            let names: Vec<String> = node.items.iter().map(|i| i.name.clone()).collect();
            format!("Projection (columns=[{}])", names.join(", "))
        }
        PhysicalPlan::Distinct(_) => "Distinct".to_string(),
        PhysicalPlan::SetOperation(node) => format!(
            "SetOperation [{}{}] (columns=[{}])",
            match node.op {
                SetOpKind::Union => "UNION",
                SetOpKind::Intersect => "INTERSECT",
                SetOpKind::Except => "EXCEPT",
            },
            if node.all { " ALL" } else { "" },
            node.columns.join(", ")
        ),
        PhysicalPlan::SingleRow => "SingleRow".to_string(),
        PhysicalPlan::Explain(_) => "Explain".to_string(),
    }
}

/// 扫描类节点的输出列形状（`projection` 非空时按投影裁剪，空 = 全列）。
fn columns(all: &[String], projection: &[usize]) -> String {
    if projection.is_empty() {
        format!("[{}]", all.join(", "))
    } else {
        let trimmed: Vec<&str> = projection.iter().map(|&i| all[i].as_str()).collect();
        format!("[{}]", trimmed.join(", "))
    }
}

/// 键的确定性文本（有效字节的十六进制）。
fn key_hex(key: &Key) -> String {
    key.as_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// 等值 JOIN 条件文本（`l.id = r.rid`）。
fn join_condition_text(condition: &JoinCondition) -> String {
    let left = match &condition.left_column.table {
        Some(table) => format!("{}.{}", table, condition.left_column.column),
        None => condition.left_column.column.clone(),
    };
    let right = match &condition.right_column.table {
        Some(table) => format!("{}.{}", table, condition.right_column.column),
        None => condition.right_column.column.clone(),
    };
    format!("{} = {}", left, right)
}

/// 聚合函数文本（含 MS25 T4 的 DISTINCT 量化词与 group_concat 分隔符）。
fn aggregate_text(func: &AggregateFunc) -> String {
    match func {
        AggregateFunc::CountStar => "count(*)".to_string(),
        AggregateFunc::Count(col) => format!("count({})", col),
        AggregateFunc::Sum(col) => format!("sum({})", col),
        AggregateFunc::Avg(col) => format!("avg({})", col),
        AggregateFunc::Min(col) => format!("min({})", col),
        AggregateFunc::Max(col) => format!("max({})", col),
        AggregateFunc::CountDistinct(col) => format!("count(DISTINCT {})", col),
        AggregateFunc::SumDistinct(col) => format!("sum(DISTINCT {})", col),
        AggregateFunc::AvgDistinct(col) => format!("avg(DISTINCT {})", col),
        AggregateFunc::GroupConcat { col, separator } => match separator {
            Some(sep) => format!("group_concat({}, sep={})", col, sep),
            None => format!("group_concat({})", col),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::plan::{DistinctNode, LimitNode, ProjectionNode, SortNode};
    use crate::executor::{OrderByColumn, ProjectionItem};

    #[test]
    fn renders_nested_tree_top_down_with_indentation() {
        let plan = PhysicalPlan::Limit(LimitNode {
            input: Box::new(PhysicalPlan::Distinct(DistinctNode {
                input: Box::new(PhysicalPlan::Sort(SortNode {
                    input: Box::new(PhysicalPlan::SingleRow),
                    order_by: vec![OrderByColumn {
                        column: "v".to_string(),
                        asc: true,
                        expr: None,
                    }],
                    table_name: "t".to_string(),
                    columns: vec!["v".to_string()],
                    projection: Vec::new(),
                })),
            })),
            limit: 5,
            offset: 0,
        });

        assert_eq!(
            render_plan_tree(&plan),
            vec![
                "EXPLAIN".to_string(),
                "  Limit (limit=5, offset=0)".to_string(),
                "    Distinct".to_string(),
                "      Sort (order_by=[v ASC], columns=[v])".to_string(),
                "        SingleRow".to_string(),
            ]
        );
    }

    #[test]
    fn renders_projection_item_names() {
        let plan = PhysicalPlan::Projection(ProjectionNode {
            input: Box::new(PhysicalPlan::SingleRow),
            items: vec![ProjectionItem {
                expr: std::sync::Arc::new(crate::executor::ConstantExpression {
                    value: crate::executor::Value::Int(1),
                }),
                name: "one".to_string(),
            }],
            columns: vec!["one".to_string()],
        });
        assert_eq!(
            render_plan_tree(&plan),
            vec![
                "EXPLAIN".to_string(),
                "  Projection (columns=[one])".to_string(),
                "    SingleRow".to_string(),
            ]
        );
    }
}
