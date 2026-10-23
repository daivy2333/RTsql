//! 每语句查询内存预算（MS32 Iteration 003，design D4/D5）。
//!
//! 语句入口按 `Database` 上限快照构造一个预算对象，随语句结束丢弃——计数
//! 不跨语句。`limit == 0`（默认，未配置）时 charge/release 全部短路，零
//! 行为差异。计费口径单源：非字符串值 16 字节，字符串 16 加字节长度。

use crate::executor::Value;
use crate::storage::StorageError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 单个非字符串值的固定计费字节数（design D4 口径）。
const VALUE_BYTES: u64 = 16;

/// 驻留字节口径（design D4）：非字符串值（含 NULL）恒 16 字节；字符串
/// 16 加字节长度。同一份值的每个驻留副本分别计费。
pub(crate) fn value_bytes(value: &Value) -> u64 {
    match value {
        Value::String(s) => string_bytes(s),
        _ => VALUE_BYTES,
    }
}

/// 字符串口径：16 + 字节长度。
pub(crate) fn string_bytes(s: &str) -> u64 {
    VALUE_BYTES + s.len() as u64
}

/// 行计费 = 各值之和。
pub(crate) fn row_bytes(row: &[Value]) -> u64 {
    row.iter().map(value_bytes).sum()
}

/// 行集计费 = 各行之和。
pub(crate) fn rows_bytes(rows: &[Vec<Value>]) -> u64 {
    rows.iter().map(|row| row_bytes(row)).sum()
}

/// 相关参数键计费：`(参数名, 值)` 序列按名字符串 + 值之和（语句级缓存键）。
pub(crate) fn param_key_bytes(params: &[(String, Value)]) -> u64 {
    params
        .iter()
        .map(|(name, value)| string_bytes(name) + value_bytes(value))
        .sum()
}

/// MS32 Iter003（design D5）：哈希桶插入 + 计费单源——新桶入账键字节，行
/// 字节恒入账（同一份值的每个驻留副本分别计费）。返回本次新增驻留字节，
/// 供替换式哈希表（相关重建）在换表时按同字节出账。
pub(crate) fn charge_bucket_insert(
    budget: &QueryBudget,
    map: &mut std::collections::HashMap<Vec<Value>, Vec<Vec<Value>>>,
    hash_key: Vec<Value>,
    row: Vec<Value>,
) -> crate::storage::Result<u64> {
    use std::collections::hash_map::Entry;
    match map.entry(hash_key) {
        Entry::Vacant(vacant) => {
            let key_bytes = row_bytes(vacant.key());
            let bytes = key_bytes + row_bytes(&row);
            budget.charge(bytes)?;
            vacant.insert(vec![row]);
            Ok(bytes)
        }
        Entry::Occupied(mut occupied) => {
            let bytes = row_bytes(&row);
            budget.charge(bytes)?;
            occupied.get_mut().push(row);
            Ok(bytes)
        }
    }
}

/// 语句级预算对象。
pub struct QueryBudget {
    limit: u64,
    used: AtomicU64,
}

impl QueryBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            used: AtomicU64::new(0),
        }
    }

    /// 不限制形态——执行器直构（单测）与未接线路径的默认值。
    pub fn unlimited() -> Self {
        Self::new(0)
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// 驻留字节入账。超限时回退本次入账并返回超限错误——语句整句失败，
    /// 预算随语句丢弃，无需逐点释放。
    pub fn charge(&self, bytes: u64) -> crate::storage::Result<()> {
        if self.limit == 0 || bytes == 0 {
            return Ok(());
        }
        let prev = self.used.fetch_add(bytes, Ordering::Relaxed);
        if prev.saturating_add(bytes) > self.limit {
            self.used.fetch_sub(bytes, Ordering::Relaxed);
            return Err(StorageError::QueryMemoryExceeded { limit: self.limit });
        }
        Ok(())
    }

    /// 驻留字节出账：按入账时的同一字节数回退，不重算口径。
    pub fn release(&self, bytes: u64) {
        if self.limit == 0 || bytes == 0 {
            return;
        }
        self.used.fetch_sub(bytes, Ordering::Relaxed);
    }
}

/// 执行器持有的共享预算（语句内同一实例；`unlimited` 为直构默认）。
pub(crate) type SharedBudget = Arc<QueryBudget>;

/// 直构默认预算（`QueryBudget::unlimited`）。
pub(crate) fn unlimited_budget() -> SharedBudget {
    Arc::new(QueryBudget::unlimited())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R1 默认不限制：limit 0 时任意字节数 charge 成功、release 短路。
    #[test]
    fn zero_limit_short_circuits_charge_and_release() {
        let budget = QueryBudget::new(0);
        assert!(budget.charge(u64::MAX).is_ok());
        budget.release(u64::MAX);
        assert_eq!(budget.limit(), 0);
    }

    /// R4 超限：超限 charge 返回 QueryMemoryExceeded，计数回退——后续更小
    /// 的入账仍按真实驻留判定。
    #[test]
    fn over_limit_charge_returns_error_and_rolls_back() {
        let budget = QueryBudget::new(32);
        assert!(budget.charge(32).is_ok());
        let err = budget.charge(1).expect_err("33 > 32 应超限");
        match err {
            StorageError::QueryMemoryExceeded { limit } => assert_eq!(limit, 32),
            other => panic!("应为超限变体: {other}"),
        }
        assert!(budget.charge(0).is_ok());
    }

    /// release 与 charge 同字节回退：入账-出账-再入账不误超。
    #[test]
    fn release_restores_capacity() {
        let budget = QueryBudget::new(16);
        assert!(budget.charge(16).is_ok());
        budget.release(16);
        assert!(budget.charge(16).is_ok());
    }

    /// D4 计费口径：非字符串 16，字符串 16 + len，行 = 各值之和。
    #[test]
    fn value_bytes_uses_fixed_measure() {
        assert_eq!(value_bytes(&Value::Int(1)), 16);
        assert_eq!(value_bytes(&Value::Null), 16);
        assert_eq!(value_bytes(&Value::Float(1.5)), 16);
        assert_eq!(value_bytes(&Value::String("abc".to_string())), 19);
        assert_eq!(
            row_bytes(&[Value::Int(1), Value::String("ab".to_string())]),
            34
        );
    }
}
