//! 会话内基表统计。
//!
//! 计数只留在打开的会话里，不写目录和 WAL。不同值按 `Value` 的相等比较累计，
//! 负零和 DECIMAL 规范形各算一个值。

use crate::executor::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ColumnStat {
    pub name: String,
    pub row_count: u64,
    pub null_count: u64,
    pub ndv: u64,
}

/// 按列统计可见行。`column_names` 的顺序就是结果行序。NULL 计入 `null_count`，不计入 `ndv`。
pub(crate) fn summarize(column_names: &[String], rows: &[Vec<Value>]) -> Vec<ColumnStat> {
    let mut columns: Vec<ColumnStat> = column_names
        .iter()
        .map(|name| ColumnStat {
            name: name.clone(),
            row_count: rows.len() as u64,
            null_count: 0,
            ndv: 0,
        })
        .collect();
    let mut distinct = vec![Vec::new(); column_names.len()];
    for row in rows {
        for (index, column) in columns.iter_mut().enumerate() {
            let value = match row.get(index) {
                Some(value) => value.clone(),
                None => Value::Null,
            };
            if value == Value::Null {
                column.null_count += 1;
                continue;
            }
            if distinct[index].iter().all(|seen| seen != &value) {
                distinct[index].push(value);
            }
        }
    }
    for (column, values) in columns.iter_mut().zip(distinct) {
        column.ndv = values.len() as u64;
    }
    columns
}

/// 本会话已保存的基表统计。克隆的 `Database` 共享同一份。
#[derive(Debug, Default)]
pub(crate) struct SessionStats {
    tables: Mutex<BTreeMap<String, Vec<ColumnStat>>>,
}

impl SessionStats {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<ColumnStat>>> {
        self.tables.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// 本轮只有箱内测试经 `Database::table_stats_snapshot` 读取。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn snapshot(&self) -> BTreeMap<String, Vec<ColumnStat>> {
        self.lock().clone()
    }
}

/// 左深代价。本轮计划器不调用，只由本文件的测试读取。
#[cfg_attr(not(test), allow(dead_code))]
mod join_order {
    /// 一次左深枚举能接受的叶数。再多就不再挑顺序。
    const LEFT_DEEP_LIMIT: usize = 8;

    #[derive(Clone, Debug)]
    pub(crate) struct LeafCard {
        pub qualifier: String,
        pub rows: u64,
        /// 列名和该列的不同值个数。
        pub ndv: Vec<(String, u64)>,
    }

    #[derive(Clone, Debug)]
    pub(crate) struct EqEdge {
        pub left_qualifier: String,
        pub left_column: String,
        pub right_qualifier: String,
        pub right_column: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum LeftDeepChoice {
        /// 用估计选出的叶序。代价与书写顺序相同时，叶序保持书写顺序。
        Estimated { order: Vec<String>, cost: u64 },
        /// 叶数超过上限，或书写顺序的代价溢出。不另给一条叶序。
        Insufficient,
    }

    enum OrderCost {
        Value(u64),
        Illegal,
        Overflow,
    }

    /// 某个叶序的代价。不合法或溢出时为 `None`。
    pub(crate) fn order_cost(
        leaves: &[LeafCard],
        edges: &[EqEdge],
        order: &[&str],
        sort_merge_first: impl Fn(&str, &str) -> bool,
    ) -> Option<u64> {
        match cost_of(leaves, edges, order, &sort_merge_first) {
            OrderCost::Value(cost) => Some(cost),
            OrderCost::Illegal | OrderCost::Overflow => None,
        }
    }

    /// 在合法左深顺序里取最小代价。平手时保留书写限定符序。
    pub(crate) fn choose_left_deep_order(
        leaves: &[LeafCard],
        edges: &[EqEdge],
        written: &[&str],
        sort_merge_first: impl Fn(&str, &str) -> bool,
    ) -> LeftDeepChoice {
        let sort_merge_first = &sort_merge_first;
        if written.len() > LEFT_DEEP_LIMIT {
            return LeftDeepChoice::Insufficient;
        }
        if matches!(
            cost_of(leaves, edges, written, sort_merge_first),
            OrderCost::Overflow
        ) {
            return LeftDeepChoice::Insufficient;
        }
        let mut best: Option<(u64, Vec<usize>)> = None;
        let mut chosen = Vec::with_capacity(written.len());
        let mut used = vec![false; written.len()];
        search(
            leaves,
            edges,
            written,
            sort_merge_first,
            &mut chosen,
            &mut used,
            &mut best,
        );
        match best {
            Some((cost, indexes)) => LeftDeepChoice::Estimated {
                order: indexes
                    .into_iter()
                    .map(|index| written[index].to_string())
                    .collect(),
                cost,
            },
            None => LeftDeepChoice::Insufficient,
        }
    }

    fn search(
        leaves: &[LeafCard],
        edges: &[EqEdge],
        written: &[&str],
        sort_merge_first: &dyn Fn(&str, &str) -> bool,
        chosen: &mut Vec<usize>,
        used: &mut [bool],
        best: &mut Option<(u64, Vec<usize>)>,
    ) {
        if chosen.len() == written.len() {
            let order: Vec<&str> = chosen.iter().map(|&index| written[index]).collect();
            if let OrderCost::Value(cost) = cost_of(leaves, edges, &order, sort_merge_first) {
                let replace = match best {
                    None => true,
                    Some((best_cost, best_order)) => {
                        cost < *best_cost
                            || (cost == *best_cost && chosen.as_slice() < best_order.as_slice())
                    }
                };
                if replace {
                    *best = Some((cost, chosen.clone()));
                }
            }
            return;
        }
        for index in 0..written.len() {
            if used[index] {
                continue;
            }
            used[index] = true;
            chosen.push(index);
            search(leaves, edges, written, sort_merge_first, chosen, used, best);
            chosen.pop();
            used[index] = false;
        }
    }

    fn cost_of(
        leaves: &[LeafCard],
        edges: &[EqEdge],
        order: &[&str],
        sort_merge_first: &dyn Fn(&str, &str) -> bool,
    ) -> OrderCost {
        if order.len() < 2 {
            return OrderCost::Value(0);
        }
        let Some(first) = leaf_by_name(leaves, order[0]) else {
            return OrderCost::Illegal;
        };
        let mut left_rows = first.rows;
        let mut placed = vec![order[0].to_string()];
        let mut total = 0u64;
        for (index, qualifier) in order.iter().copied().enumerate().skip(1) {
            let Some(edge) = connecting_edge(edges, qualifier, &placed) else {
                return OrderCost::Illegal;
            };
            let Some(right) = leaf_by_name(leaves, qualifier) else {
                return OrderCost::Illegal;
            };
            let divisor = column_ndv(leaves, &edge.left_qualifier, &edge.left_column)
                .max(column_ndv(
                    leaves,
                    &edge.right_qualifier,
                    &edge.right_column,
                ))
                .max(1);
            let Some(product) = left_rows.checked_mul(right.rows) else {
                return OrderCost::Overflow;
            };
            let output = product / divisor;
            // 只有两张基表的第一步才可能用排序合并权。中间结果当左输入时始终是探测 1、构建 2、输出 1。
            let sort_merge = index == 1 && sort_merge_first(order[0], qualifier);
            let left_weight = if sort_merge { 2 } else { 1 };
            let Some(step) = left_rows
                .checked_mul(left_weight)
                .and_then(|left_term| {
                    right
                        .rows
                        .checked_mul(2)
                        .and_then(|right_term| left_term.checked_add(right_term))
                })
                .and_then(|sum| sum.checked_add(output))
            else {
                return OrderCost::Overflow;
            };
            let Some(next_total) = total.checked_add(step) else {
                return OrderCost::Overflow;
            };
            total = next_total;
            left_rows = output;
            placed.push(qualifier.to_string());
        }
        OrderCost::Value(total)
    }

    fn leaf_by_name<'a>(leaves: &'a [LeafCard], name: &str) -> Option<&'a LeafCard> {
        leaves.iter().find(|leaf| leaf.qualifier == name)
    }

    fn column_ndv(leaves: &[LeafCard], qualifier: &str, column: &str) -> u64 {
        leaf_by_name(leaves, qualifier)
            .and_then(|leaf| {
                leaf.ndv
                    .iter()
                    .find(|(name, _)| name == column)
                    .map(|(_, count)| *count)
            })
            .unwrap_or(0)
    }

    fn connecting_edge<'a>(
        edges: &'a [EqEdge],
        qualifier: &str,
        placed: &[String],
    ) -> Option<&'a EqEdge> {
        edges.iter().find(|edge| {
            let left_placed = placed.iter().any(|name| name == &edge.left_qualifier);
            let right_placed = placed.iter().any(|name| name == &edge.right_qualifier);
            (edge.right_qualifier == qualifier && left_placed)
                || (edge.left_qualifier == qualifier && right_placed)
        })
    }
}

#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub(crate) use join_order::{choose_left_deep_order, order_cost, EqEdge, LeafCard, LeftDeepChoice};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_negative_zero_ndv_is_one() {
        let columns = vec!["f".to_string()];
        let rows = vec![vec![Value::Float(-0.0)], vec![Value::Float(0.0)]];
        let stats = summarize(&columns, &rows);
        assert_eq!(stats[0].row_count, 2);
        assert_eq!(stats[0].null_count, 0);
        assert_eq!(stats[0].ndv, 1);
    }

    #[test]
    fn summarize_canonical_decimal_ndv_is_one() {
        let left = Value::decimal(150, 2);
        let right = Value::decimal(15, 1);
        assert_eq!(left, right);
        assert_ne!(left.decimal_parts(), right.decimal_parts());
        let columns = vec!["d".to_string()];
        let stats = summarize(&columns, &[vec![left], vec![right]]);
        assert_eq!(stats[0].ndv, 1);
    }

    fn leaf(name: &str, rows: u64, ndv: &[(&str, u64)]) -> LeafCard {
        LeafCard {
            qualifier: name.to_string(),
            rows,
            ndv: ndv
                .iter()
                .map(|(column, count)| (column.to_string(), *count))
                .collect(),
        }
    }

    fn edge(left: &str, left_column: &str, right: &str, right_column: &str) -> EqEdge {
        EqEdge {
            left_qualifier: left.to_string(),
            left_column: left_column.to_string(),
            right_qualifier: right.to_string(),
            right_column: right_column.to_string(),
        }
    }

    fn chain(names: &[&str], rows: u64, ndv: u64) -> (Vec<LeafCard>, Vec<EqEdge>) {
        let leaves = names
            .iter()
            .map(|name| leaf(name, rows, &[("id", ndv)]))
            .collect();
        let edges = names
            .windows(2)
            .map(|pair| edge(pair[0], "id", pair[1], "id"))
            .collect();
        (leaves, edges)
    }

    #[test]
    fn star_join_prefers_fact_on_the_left() {
        let leaves = vec![
            leaf("d1", 2, &[("id", 2)]),
            leaf("d2", 2, &[("id", 2)]),
            leaf("fact", 100, &[("d1", 2), ("d2", 2)]),
        ];
        let edges = vec![
            edge("d1", "id", "fact", "d1"),
            edge("fact", "d2", "d2", "id"),
        ];
        let written = ["d1", "fact", "d2"];
        let hash = |_: &str, _: &str| false;
        assert_eq!(order_cost(&leaves, &edges, &written, &hash), Some(506));
        assert_eq!(
            order_cost(&leaves, &edges, &["fact", "d1", "d2"], &hash),
            Some(408)
        );
        assert_eq!(
            choose_left_deep_order(&leaves, &edges, &written, &hash),
            LeftDeepChoice::Estimated {
                order: vec!["fact".into(), "d1".into(), "d2".into()],
                cost: 408,
            }
        );
    }

    #[test]
    fn sort_merge_first_step_changes_the_chosen_order() {
        let (leaves, edges) = chain(&["a", "b", "c"], 10, 10);
        let written = ["a", "b", "c"];
        let sort_merge = |left: &str, right: &str| {
            let mut pair = [left, right];
            pair.sort_unstable();
            pair == ["a", "b"]
        };
        assert_eq!(order_cost(&leaves, &edges, &written, &sort_merge), Some(90));
        assert_eq!(
            order_cost(&leaves, &edges, &["b", "c", "a"], &sort_merge),
            Some(80)
        );
        assert_eq!(
            choose_left_deep_order(&leaves, &edges, &written, &sort_merge),
            LeftDeepChoice::Estimated {
                order: vec!["b".into(), "c".into(), "a".into()],
                cost: 80,
            }
        );
    }

    #[test]
    fn equal_hash_costs_keep_the_written_order() {
        let (leaves, edges) = chain(&["a", "b", "c"], 10, 10);
        let written = ["a", "b", "c"];
        let hash = |_: &str, _: &str| false;
        for order in [
            ["a", "b", "c"],
            ["b", "a", "c"],
            ["b", "c", "a"],
            ["c", "b", "a"],
        ] {
            assert_eq!(
                order_cost(&leaves, &edges, &order, &hash),
                Some(80),
                "{order:?}"
            );
        }
        assert_eq!(
            choose_left_deep_order(&leaves, &edges, &written, &hash),
            LeftDeepChoice::Estimated {
                order: vec!["a".into(), "b".into(), "c".into()],
                cost: 80,
            }
        );
    }

    #[test]
    fn nine_table_chain_is_insufficient() {
        let names: Vec<String> = (1..=9).map(|index| format!("t{index}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (leaves, edges) = chain(&refs, 10, 10);
        assert_eq!(
            choose_left_deep_order(&leaves, &edges, &refs, |_, _| false),
            LeftDeepChoice::Insufficient
        );
    }

    #[test]
    fn written_order_overflow_is_insufficient() {
        let (leaves, edges) = chain(&["a", "b", "c"], u64::MAX, 1);
        let written = ["a", "b", "c"];
        assert_eq!(
            order_cost(&leaves, &edges, &written, |_: &str, _: &str| false),
            None
        );
        assert_eq!(
            choose_left_deep_order(&leaves, &edges, &written, |_, _| false),
            LeftDeepChoice::Insufficient
        );
    }

    #[test]
    fn unconnected_adjacent_leaves_are_not_chosen() {
        let leaves = vec![
            leaf("a", 1000, &[("id", 1)]),
            leaf("b", 1000, &[("id", 1)]),
            leaf("c", 1, &[("id", 1)]),
        ];
        let edges = vec![edge("a", "id", "b", "id"), edge("b", "id", "c", "id")];
        let written = ["a", "b", "c"];
        assert_eq!(
            order_cost(&leaves, &edges, &["a", "c", "b"], |_: &str, _: &str| false),
            None
        );
        assert_eq!(
            choose_left_deep_order(&leaves, &edges, &written, |_, _| false),
            LeftDeepChoice::Estimated {
                order: vec!["b".into(), "c".into(), "a".into()],
                cost: 1_005_002,
            }
        );
    }
}
