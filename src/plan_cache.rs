//! Plan cache for query optimization
//!
//! MS06-T02: DashMap-backed lock-free cache with normalized SQL keys.

use crate::executor::PhysicalPlan;
use dashmap::DashMap;

/// PlanCache: lock-free per-shard reads via DashMap. All methods take &self.
pub struct PlanCache {
    map: DashMap<String, PhysicalPlan>,
    max_size: usize,
}

impl PlanCache {
    /// Create a new plan cache with default size limit
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
            max_size: 100,
        }
    }

    /// Create a new plan cache with custom size limit
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            map: DashMap::new(),
            max_size,
        }
    }

    /// Get a cached plan by SQL string. The key is normalized so case/whitespace
    /// variants of the same logical SQL share a single entry.
    pub fn get(&self, sql: &str) -> Option<PhysicalPlan> {
        let key = normalize_sql_key(sql);
        self.map.get(&key).map(|entry| entry.value().clone())
    }

    /// Put a plan into cache. The SQL string is normalized before being used as
    /// the key. When the cache is at capacity, a single arbitrary entry is
    /// evicted (simple "first writer wins" strategy, not LRU).
    pub fn put(&self, sql: String, plan: PhysicalPlan) {
        let key = normalize_sql_key(&sql);
        if self.map.len() >= self.max_size {
            // Collect the key first, then drop the RefMulti guard before
            // calling `remove` — avoids holding a shard read lock while
            // requesting a write lock on the same shard.
            let key_to_remove = self.map.iter().next().map(|e| e.key().clone());
            if let Some(k) = key_to_remove {
                self.map.remove(&k);
            }
        }
        self.map.insert(key, plan);
    }

    /// Clear the cache
    pub fn clear(&self) {
        self.map.clear();
    }

    /// Get cache size
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for PlanCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize SQL text to a canonical cache key.
///
/// 规则：
/// 1. ASCII 折叠：所有非字符串字面量区域的字符 `.to_ascii_lowercase()`
/// 2. 空白折叠：连续空白字符折叠为单个 ASCII space
/// 3. Trim：去除首尾空白
/// 4. 字符串字面量：单引号 toggle 状态机；内部的字符保留原样（含大小写）
///
/// 已知限制：未处理 SQL 标准的转义引号 `''`（`WHERE name = 'O''Brien'` 中
/// 的 `O''Brien` 会被误判为离开字符串字面量区）。本 change 接受此限制，
/// 实际工作负载中此类 case 极罕见；如未来需要再扩展为 quote-aware scanner。
pub fn normalize_sql_key(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut in_string = false;
    let mut prev_was_space = false;
    let mut started = false;

    for c in sql.chars() {
        if c == '\'' {
            in_string = !in_string;
            out.push(c);
            prev_was_space = false;
            started = true;
            continue;
        }
        if in_string {
            out.push(c);
            prev_was_space = false;
            started = true;
            continue;
        }
        if c.is_whitespace() {
            if started && !prev_was_space {
                out.push(' ');
                prev_was_space = true;
            }
            continue;
        }
        out.push(c.to_ascii_lowercase());
        prev_was_space = false;
        started = true;
    }

    // Trim trailing space (added by whitespace collapse at the tail)
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{PhysicalPlan, ScanNode};

    fn dummy_plan() -> PhysicalPlan {
        // ScanNode fields: table_name, columns, projection (see src/executor/plan.rs).
        PhysicalPlan::Scan(ScanNode {
            table_name: "t".to_string(),
            columns: vec![],
            projection: Vec::new(),
        })
    }

    // ---- normalize_sql_key tests ----

    #[test]
    fn normalize_lowercase_folding() {
        assert_eq!(normalize_sql_key("SELECT * FROM T"), "select * from t");
    }

    #[test]
    fn normalize_whitespace_collapse() {
        assert_eq!(normalize_sql_key("SELECT   *\nFROM\t t"), "select * from t");
    }

    #[test]
    fn normalize_trim() {
        assert_eq!(normalize_sql_key("  SELECT * FROM t  "), "select * from t");
    }

    #[test]
    fn normalize_preserves_string_literal() {
        assert_eq!(
            normalize_sql_key("SELECT * FROM t WHERE name = 'SELECT'"),
            "select * from t where name = 'SELECT'"
        );
    }

    #[test]
    fn normalize_variants_share_key() {
        let s1 = "SELECT * FROM t WHERE id = 1";
        let s2 = "select * from t where id = 1";
        let s3 = "SELECT\n*\nFROM t\nWHERE id = 1";
        assert_eq!(normalize_sql_key(s1), normalize_sql_key(s2));
        assert_eq!(normalize_sql_key(s2), normalize_sql_key(s3));
    }

    // ---- PlanCache behavior tests ----

    #[test]
    fn case_variants_hit_same_entry() {
        let cache = PlanCache::new();
        cache.put("SELECT * FROM t".to_string(), dummy_plan());
        let hit = cache.get("select * from T");
        assert!(hit.is_some(), "lowercase variant should hit");
    }

    #[test]
    fn whitespace_variants_hit_same_entry() {
        let cache = PlanCache::new();
        cache.put("SELECT * FROM t".to_string(), dummy_plan());
        let hit = cache.get("SELECT\n*\nFROM t");
        assert!(hit.is_some(), "whitespace variant should hit");
    }

    #[test]
    fn string_literal_case_distinguishes() {
        let cache = PlanCache::new();
        cache.put("WHERE name = 'select'".to_string(), dummy_plan());
        let hit = cache.get("WHERE name = 'SELECT'");
        assert!(hit.is_none(), "string literal case difference should miss");
    }

    #[test]
    fn put_evicts_when_full() {
        let cache = PlanCache::with_capacity(2);
        cache.put("a".to_string(), dummy_plan());
        cache.put("b".to_string(), dummy_plan());
        cache.put("c".to_string(), dummy_plan());
        assert_eq!(cache.len(), 2, "eviction should bound size to max");
    }

    #[test]
    fn clear_empties_cache() {
        let cache = PlanCache::new();
        cache.put("a".to_string(), dummy_plan());
        cache.clear();
        assert_eq!(cache.len(), 0);
    }
}
