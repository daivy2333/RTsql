//! Plan cache for query optimization
//!
//! M14 Phase 1: Simple plan cache implementation

use crate::executor::PhysicalPlan;
use std::collections::HashMap;

/// Simple plan cache using HashMap with size limit
pub struct PlanCache {
    cache: HashMap<String, PhysicalPlan>,
    max_size: usize,
}

impl PlanCache {
    /// Create a new plan cache with default size limit
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            max_size: 100,
        }
    }

    /// Create a new plan cache with custom size limit
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            cache: HashMap::new(),
            max_size,
        }
    }

    /// Get a cached plan by SQL string
    pub fn get(&mut self, sql: &str) -> Option<&PhysicalPlan> {
        self.cache.get(sql)
    }

    /// Put a plan into cache
    pub fn put(&mut self, sql: String, plan: PhysicalPlan) {
        // Simple eviction: remove random entries if at capacity
        if self.cache.len() >= self.max_size {
            // Remove first entry (arbitrary)
            if let Some(key) = self.cache.keys().next().cloned() {
                self.cache.remove(&key);
            }
        }
        self.cache.insert(sql, plan);
    }

    /// Clear the cache
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Get cache size
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for PlanCache {
    fn default() -> Self {
        Self::new()
    }
}