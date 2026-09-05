//! Correlated subquery value injection
//!
//! Walks a PhysicalPlan tree and injects outer row values into
//! ParameterExpression nodes in Filter, Having, and pushed-down DataScan
//! predicate trees.

use crate::executor::{PhysicalPlan, Value};

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
        PhysicalPlan::DataScan(node) => {
            // MS07-T06: DataScan can carry a pushed-down WHERE predicate that
            // contains ParameterExpression nodes for correlated subqueries.
            if let Some(predicate) = &node.predicate {
                predicate.inject_parameters(param_values);
            }
        }
        PhysicalPlan::Sort(node) => {
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
        PhysicalPlan::DerivedScan(node) => {
            inject_correlated_values(&node.subquery, param_values);
        }
        // Leaf or DML nodes: no sub-plans, no predicates
        PhysicalPlan::Scan(_)
        | PhysicalPlan::IndexScan(_)
        | PhysicalPlan::IndexScanAll(_)
        | PhysicalPlan::Insert(_)
        | PhysicalPlan::Update(_)
        | PhysicalPlan::Delete(_)
        | PhysicalPlan::CreateTable(_)
        | PhysicalPlan::DropTable(_) => {}
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
            })),
            predicate: Arc::clone(&pred),
            table_name: "dept".to_string(),
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
