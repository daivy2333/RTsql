//! Correlated subquery value injection
//!
//! Walks a PhysicalPlan tree and injects outer row values into
//! ParameterExpression nodes in Filter, Having, and pushed-down DataScan
//! predicate trees.

use crate::executor::{PhysicalPlan, UpdateValue, Value};

/// Inject correlated parameter values into all ParameterExpression nodes
/// in the given PhysicalPlan tree. Walks recursively into sub-plans.
///
/// After injection, any ParameterExpression matching a param_name in `param_values`
/// will return the corresponding Value from `evaluate()`.
pub fn inject_correlated_values(plan: &PhysicalPlan, param_values: &[(String, Value)]) {
    match plan {
        PhysicalPlan::Filter(node) => {
            node.predicate.inject_parameters(param_values);
            inject_correlated_values(&node.input, param_values);
        }
        PhysicalPlan::Having(node) => {
            node.predicate.inject_parameters(param_values);
            inject_correlated_values(&node.input, param_values);
        }
        PhysicalPlan::SemiJoin(node) => {
            inject_correlated_values(&node.left, param_values);
            inject_correlated_values(&node.right, param_values);
        }
        PhysicalPlan::AntiJoin(node) => {
            inject_correlated_values(&node.left, param_values);
            inject_correlated_values(&node.right, param_values);
        }
        PhysicalPlan::SubqueryEval(node) => {
            inject_correlated_values(&node.input, param_values);
            inject_correlated_values(&node.subquery, param_values);
        }
        // T4（change 2026-10-04-external-review-gaps）：标量谓词节点——输入
        // 链与各绑定子查询内的相关参数注入；节点自身谓词的占位参数由执行器
        // 逐行注入，不在此处消费外层值。
        PhysicalPlan::ScalarSubqueryFilter(node) => {
            inject_correlated_values(&node.input, param_values);
            for binding in &node.bindings {
                inject_correlated_values(&binding.subquery, param_values);
            }
        }
        PhysicalPlan::DataScan(node) => {
            // MS07-T06: DataScan can carry a pushed-down WHERE predicate that
            // contains ParameterExpression nodes for correlated subqueries.
            if let Some(predicate) = &node.predicate {
                predicate.inject_parameters(param_values);
            }
        }
        PhysicalPlan::Sort(node) => {
            // MS31 Iteration 001：排序键表达式位（别名键 / 聚合输出行键）可能
            // 携带 ParameterExpression——注入链与谓词、投影同款（未注入时求值
            // 为 Null，排序键退化为常量）。
            for column in &node.order_by {
                if let Some(expr) = &column.expr {
                    for (name, value) in param_values {
                        expr.set_parameter_value(name, value);
                    }
                }
            }
            inject_correlated_values(&node.input, param_values);
        }
        PhysicalPlan::Limit(node) => {
            inject_correlated_values(&node.input, param_values);
        }
        PhysicalPlan::Aggregate(node) => {
            inject_correlated_values(&node.input, param_values);
        }
        PhysicalPlan::Join(node) => {
            inject_correlated_values(&node.left, param_values);
            inject_correlated_values(&node.right, param_values);
        }
        PhysicalPlan::NestedLoopJoin(node) => {
            // MS09-T02: 与 Join 臂同型——ON 谓词可能携带 ParameterExpression
            //（子查询内 NLJ），递归左右输入 + 谓词注入。
            node.predicate.inject_parameters(param_values);
            inject_correlated_values(&node.left, param_values);
            inject_correlated_values(&node.right, param_values);
        }
        PhysicalPlan::DerivedScan(node) => {
            inject_correlated_values(&node.subquery, param_values);
        }
        PhysicalPlan::Projection(node) => {
            // MS11-T01 Iter001: 表达式项内的 ParameterExpression（相关外层
            // 引用）经 Expression::set_parameter_value 注入——与谓词操作数
            // 的传播同链（未注入时 ParameterExpression 求值为 Null，静默错）。
            for item in &node.items {
                for (name, value) in param_values {
                    item.expr.set_parameter_value(name, value);
                }
            }
            inject_correlated_values(&node.input, param_values);
        }
        // MS31 Iteration 001：UPDATE 赋值右值位——值表达式臂携带
        // ParameterExpression（参数位右值），经同款注入链取值（执行器对命中行
        // 旧行快照求值，走 owned evaluate 真值通道）。
        PhysicalPlan::Update(node) => {
            for assignment in &node.assignments {
                if let UpdateValue::Expression(expr) = &assignment.value {
                    for (name, value) in param_values {
                        expr.set_parameter_value(name, value);
                    }
                }
            }
        }
        // Leaf or DML nodes: no sub-plans, no predicates
        PhysicalPlan::Scan(_)
        | PhysicalPlan::IndexScan(_)
        | PhysicalPlan::IndexScanAll(_)
        | PhysicalPlan::Insert(_)
        | PhysicalPlan::Upsert(_)
        | PhysicalPlan::Delete(_)
        | PhysicalPlan::CreateTable(_)
        | PhysicalPlan::CreateIndex(_)
        | PhysicalPlan::DropIndex(_)
        | PhysicalPlan::AlterTable(_)
        | PhysicalPlan::Truncate(_)
        | PhysicalPlan::CreateView(_)
        | PhysicalPlan::DropView(_)
        | PhysicalPlan::DropTable(_) => {}
        PhysicalPlan::CreateTableAsSelect(node) => {
            // MS20 T4: CTAS 包着一条完整 SELECT，递归其下计划（谓词/表达式
            // 参数注入与 `Projection` 语义由源计划自身承担）。
            inject_correlated_values(&node.select, param_values);
        }
        PhysicalPlan::InsertSelect(node) => {
            // MS20-T02 Iter001：INSERT SELECT 的源是完整 SELECT 子树，关联
            // 参数注入与 CTAS 源同型（目标映射无表达式，不参与注入）。
            inject_correlated_values(&node.source, param_values);
        }
        PhysicalPlan::SetOperation(node) => {
            // MS25 T3: 复合两侧的谓词/表达式参数注入（两侧均为完整 SELECT）
            inject_correlated_values(&node.left, param_values);
            inject_correlated_values(&node.right, param_values);
        }
        PhysicalPlan::Distinct(node) => {
            // MS25 T2: 无自有谓词，递归输入（其下的 Projection 臂承担表达式
            // 参数注入）。
            inject_correlated_values(&node.input, param_values);
        }
        // MS13 T9: no-FORM 输入节点无谓词、无子计划
        PhysicalPlan::SingleRow => {}
        // MS25 T6: EXPLAIN 只渲染不执行（子查询内不可达 EXPLAIN 语句），
        // 被观测计划不参与参数注入。
        PhysicalPlan::Explain(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{
        ColumnExpression, ComparisonOp, ComparisonPredicate, FilterNode, ParameterExpression,
        PredicateRef, ScanNode,
    };
    use std::sync::Arc;

    #[test]
    fn test_inject_into_filter() {
        // Create a ParameterExpression that will receive injected value
        let param = Arc::new(ParameterExpression::new("emp.dept".to_string()));

        // Build predicate: id = emp.dept
        // where emp.dept is the ParameterExpression (correlated placeholder)
        let pred: PredicateRef = Arc::new(ComparisonPredicate {
            left: Arc::new(ColumnExpression {
                column_name: "id".to_string(),
                column_index: 0,
            }),
            op: ComparisonOp::Eq,
            right: param.clone(),
        });

        let plan = PhysicalPlan::Filter(FilterNode {
            input: Box::new(PhysicalPlan::Scan(ScanNode {
                table_name: "dept".to_string(),
                columns: vec!["id".to_string(), "name".to_string()],
                projection: Vec::new(),
            })),
            predicate: Arc::clone(&pred),
            table_name: "dept".to_string(),
            projection: Vec::new(),
        });

        // Inject emp.dept = 42
        inject_correlated_values(&plan, &[("emp.dept".to_string(), Value::Int(42))]);

        // Now evaluate: row has id=42, so 42 = 42 should be true
        let row = vec![Value::Int(42), Value::String("test".to_string())];
        assert!(pred.evaluate(&row).unwrap());

        // Test with non-matching value
        inject_correlated_values(&plan, &[("emp.dept".to_string(), Value::Int(99))]);
        assert!(!pred.evaluate(&row).unwrap());
    }
}
