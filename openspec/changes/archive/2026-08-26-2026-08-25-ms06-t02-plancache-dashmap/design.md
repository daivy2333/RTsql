# Design: PlanCache DashMap + SQL 规范化

## 目标

将 `src/plan_cache.rs` 从 `HashMap + &mut self` 重构为 `DashMap + &self`；新增 `normalize_sql_key` 让 cache key 摆脱字符串字面写法的干扰；移除 `Database.plan_cache` 外层 `Mutex`，让 100 并发 SELECT 不再序列化争用。

## 现状（修改前）

### 类型与持有者

```rust
// src/plan_cache.rs:9-12
pub struct PlanCache {
    cache: HashMap<String, PhysicalPlan>,
    max_size: usize,
}
// 全部方法签名 &mut self

// src/database.rs:22
pub plan_cache: Arc<Mutex<PlanCache>>,
// src/database.rs:64
let plan_cache = Arc::new(Mutex::new(PlanCache::new()));
// src/database.rs:95
pub fn plan_cache_len(&self) -> usize {
    self.plan_cache.lock().unwrap().len()
}
```

### 5 个调用点

```rust
// src/pipeline.rs:56
let mut cache = database.plan_cache.lock().unwrap();
cache.get(sql).cloned()
// src/pipeline.rs:62  (no profiling 分支)
let mut cache = database.plan_cache.lock().unwrap();
cache.get(sql).cloned()
// src/pipeline.rs:145, 169  (DDL clear)
database.plan_cache.lock().unwrap().clear();
// src/pipeline.rs:206  (cache put)
let mut cache = database.plan_cache.lock().unwrap();
cache.put(sql.to_string(), plan.clone());
```

### Cache key 问题

`cache.get(sql)` 用原 SQL 字符串做 key。同一逻辑查询的不同写法（大小写、空白、换行）产生不同 key，命中率损失。

`src/parser/ast.rs` 与 `src/parser/planner.rs` 已对 identifier 全部 `to_lowercase()`：

- `parser/ast.rs:27` `TableFactor::Table { name, .. } => Ok(name.to_string().to_lowercase())`
- `parser/ast.rs:42` `Expr::Identifier(ident) => Ok(ident.value.to_string().to_lowercase())`
- `parser/planner.rs:46` `let name_lower = name.to_lowercase();`
- `parser/planner.rs:94` `let name_lower = table_name.to_lowercase();`
- `parser/planner.rs:626, 860, 866, 879, ...`（30+ 处）

**结论**：planner 已经把 identifier 折叠为 lowercase，所以 `FROM Users` 与 `FROM users` 生成的 `PhysicalPlan` 完全相同——只有 cache key 不一致。

## 修改方案

### 1. `src/plan_cache.rs` 重写

```rust
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
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
            max_size: 100,
        }
    }

    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            map: DashMap::new(),
            max_size,
        }
    }

    pub fn get(&self, sql: &str) -> Option<PhysicalPlan> {
        let key = normalize_sql_key(sql);
        self.map.get(&key).map(|entry| entry.value().clone())
    }

    pub fn put(&self, sql: String, plan: PhysicalPlan) {
        let key = normalize_sql_key(&sql);
        // 满则驱逐任意一条
        if self.map.len() >= self.max_size {
            if let Some(entry) = self.map.iter().next() {
                let k = entry.key().clone();
                self.map.remove(&k);
            }
        }
        self.map.insert(key, plan);
    }

    pub fn clear(&self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

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
        // 实际定义见 src/executor/plan.rs:61-66：ScanNode 只有 table_name 与 columns 两个字段
        PhysicalPlan::Scan(ScanNode {
            table_name: "t".to_string(),
            columns: vec![],
        })
    }

    #[test]
    fn normalize_lowercase_folding() {
        assert_eq!(normalize_sql_key("SELECT * FROM T"), "select * from t");
    }

    #[test]
    fn normalize_whitespace_collapse() {
        assert_eq!(
            normalize_sql_key("SELECT   *\nFROM\t t"),
            "select * from t"
        );
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
```

### 2. `src/database.rs` 持有者类型

```rust
// line 22
pub plan_cache: Arc<PlanCache>,
// line 64
let plan_cache = Arc::new(PlanCache::new());
// line 95
pub fn plan_cache_len(&self) -> usize {
    self.plan_cache.len()
}
```

不再需要 `use std::sync::Mutex;` 用于 plan_cache 字段；但 `std::sync::Mutex` 仍可能用于其他字段（目前 database.rs 没有别处使用，应可整行删）。

### 3. `src/pipeline.rs` 5 处调用点

```rust
// line 56  替换为：
let result = database.plan_cache.get(sql);
record_time("cache_hit_check", t0.elapsed());
result
// 旧版：let mut cache = database.plan_cache.lock().unwrap(); cache.get(sql).cloned()

// line 62  替换为：
let result = database.plan_cache.get(sql);
result
// 旧版：let mut cache = database.plan_cache.lock().unwrap(); cache.get(sql).cloned()

// line 145  替换为：
database.plan_cache.clear();
// 旧版：database.plan_cache.lock().unwrap().clear();

// line 169  替换为：
database.plan_cache.clear();
// 旧版：database.plan_cache.lock().unwrap().clear();

// line 206  替换为：
if is_cacheable(stmt) {
    database.plan_cache.put(sql.to_string(), plan.clone());
}
```

### 4. `tests/executor_test.rs` 5 处 setup

5 处（line 705, 745, 796, 842, 879）`plan_cache: Arc::new(Mutex::new(rtsql::plan_cache::PlanCache::new()))` → `plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new())`。

如需 `use std::sync::Mutex;` 仅用于 plan_cache，则该 import 在 executor_test.rs 中需移除（`grep` 确认仅此 5 处使用 Mutex）。

### 5. `tests/plan_cache_test.rs` 新增（集成）

```rust
//! Integration tests for MS06-T02 PlanCache DashMap + SQL normalization

use rtsql::database::Database;
use rtsql::plan_cache::{normalize_sql_key, PlanCache};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_case_variant_hits_cache() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')").await;

    db.execute_sql("SELECT * FROM t WHERE id = 1").await;
    let n1 = db.plan_cache_len();
    assert!(n1 > 0);

    let r = db.execute_sql("select * from t where id = 1").await;
    assert!(matches!(r, rtsql::network::protocol::Response::QueryResult { .. }));
    assert_eq!(db.plan_cache_len(), n1, "lowercase variant should not grow cache");
}

#[tokio::test]
async fn test_whitespace_variant_hits_cache() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("INSERT INTO t (id) VALUES (1)").await;

    db.execute_sql("SELECT * FROM t").await;
    let n1 = db.plan_cache_len();
    assert!(n1 > 0);

    let r = db.execute_sql("SELECT\n*\nFROM t").await;
    assert!(matches!(r, rtsql::network::protocol::Response::QueryResult { .. }));
    assert_eq!(db.plan_cache_len(), n1);
}

#[tokio::test]
async fn test_string_literal_case_does_not_hit() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')").await;

    db.execute_sql("SELECT * FROM t WHERE name = 'alice'").await;
    let n1 = db.plan_cache_len();
    assert!(n1 > 0);

    let r = db.execute_sql("SELECT * FROM t WHERE name = 'Alice'").await;
    assert!(matches!(r, rtsql::network::protocol::Response::QueryResult { .. }));
    // 'Alice' 与 'alice' 字符串字面量大小写不同 → 规范化后 key 不同 → cache miss → size +1
    assert_eq!(db.plan_cache_len(), n1 + 1);
}

#[tokio::test]
async fn test_concurrent_hits_do_not_block_runtime() {
    // 100 tokio::spawn 同时执行同 SELECT；cache 必须并发无锁命中
    // 验证：测试在 5s 内完成（不挂死），全部返回正确结果
    use std::time::Instant;
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(&dir.path().join("test.db")).await.unwrap());
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    for i in 0..50 {
        db.execute_sql(&format!("INSERT INTO t (id, v) VALUES ({}, {})", i, i))
            .await;
    }
    db.execute_sql("SELECT * FROM t WHERE id = 1").await; // 预热

    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..100 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            db.execute_sql("SELECT * FROM t WHERE id = 1").await
        }));
    }
    for h in handles {
        let r = h.await.unwrap();
        assert!(matches!(r, rtsql::network::protocol::Response::QueryResult { .. }));
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "100 concurrent SELECTs should finish in <5s, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_dml_still_not_cached() {
    // 验证 is_cacheable 行为不变
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("INSERT INTO t (id) VALUES (1)").await;
    assert_eq!(db.plan_cache_len(), 0, "INSERT should not be cached");
}

#[tokio::test]
async fn test_ddl_still_clears_cache() {
    // 验证 DDL clear 行为不变
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("SELECT * FROM t").await;
    assert!(db.plan_cache_len() > 0);
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)").await;
    assert_eq!(db.plan_cache_len(), 0);
}

#[test]
fn normalize_module_function_public() {
    // 验证 normalize_sql_key 是 pub 公开函数
    assert_eq!(normalize_sql_key("SELECT 1"), "select 1");
}
```

### 6. T0 前置：基线 clippy 归零（2026-08-25 用户批准并入本 change）

Plan 审计实测：当前基线（HEAD `56869ba`）上 `cargo clippy --all-targets -- -D warnings`
失败，共 7 处既有错误，其中 version_chain.rs 的 unused import 由 MS06-T01 提交引入。
V2 门禁要求 clippy 全绿，因此增加前置任务 T0，机械修复以下位置（不改变任何运行时行为）：

| # | 位置 | Lint | 最小修复 |
|---|---|---|---|
| L1 | `src/transaction/version_chain.rs:114` | unused_imports | 删除 cfg(test) 内 `use crate::storage::PageId;` |
| L2 | `src/executor/data_scan.rs:37,63` | dead_code（字段构造后从未读取） | 删除 `DataScanExecutor.table_meta` 字段声明与构造存储；`new()` 参数与签名不变（参数仍用于提取 schema 与 data_page_head） |
| L3 | `src/storage/buffer_pool.rs:139` | doc_list_item_without_indentation | 按 rustdoc 规范重排 SAFETY doc 列表缩进 |
| L4 | `src/transaction/manager.rs:380` | unused_variables | `tx2_id` → `_tx2_id`（保留意图标注） |
| L5 | `src/storage/page_format/tuple.rs:315` | redundant clone 建议 | `&[value.clone()]` → `std::slice::from_ref(&value)`（cfg(test) helper） |
| L6 | `src/storage/page_visibility.rs:69` | clippy::clone_on_copy | 该测试的存在意义就是验证 `Clone` impl：对测试函数加 `#[allow(clippy::clone_on_copy)]` 并注释说明，保留 `.clone()` 断言 |
| L7 | `src/executor/value_ref.rs:207` | dropping_copy_types | 删除 `drop(vr);` 行（NLL 下 vr 最后使用在 line 205，borrow 已自然结束） |

另：`tests/executor_test.rs` 内部模块 line 903 / 975 / 1062 的函数局部
`use std::sync::{Arc, Mutex};` 中 `Mutex` 当前即未被使用（全文件无其他 `Mutex` 引用），
属潜在 unused_imports，T0 一并处理：三处均改为 `use std::sync::Arc;`。

设计取舍：

- L6 用 allow 而非删断言——删除 `.clone()` 会让"验证 Clone 实现"的测试失去语义。
- 其余 6 处直接修复而非 allow——均为真正的机械残留，无表达价值。
- lib / test 目标编译此前被前序 clippy 错误截断，T0 后可能暴露少量表外既有 lint：
  机械项就地修复并在 Act Response 记录；涉及行为或语义的项停止并返回 Plan。

## 行为差异表

| 场景 | 改前 | 改后 |
|---|---|---|
| 同一字符串重复 SELECT | hit | hit |
| `SELECT * FROM T` vs `select * from t` | miss（key 不同）| hit（同 key） |
| `SELECT\n*\nFROM t` vs `SELECT * FROM t` | miss | hit |
| `WHERE name = 'alice'` vs `WHERE name = 'Alice'` | miss | miss（保留语义不同）|
| 100 并发同 SELECT | `Mutex` 串行化争用 | DashMap 无锁并发 |
| DDL clear | `Mutex.lock().clear()` | `DashMap.clear()` |
| DML put | 不进入（`is_cacheable` 拦截）| 不进入（行为不变）|
| 内存中 entry 数 | `HashMap.len()` | `DashMap.len()` |
| `Database.plan_cache` 字段类型 | `Arc<Mutex<PlanCache>>` | `Arc<PlanCache>` |
| `PlanCache::get` 签名 | `&mut self` | `&self` |

## 兼容性

- **现有 `tests/pipeline_test.rs`**：3 个测试（test_plan_cache_hit / test_ddl_clears_cache / test_dml_not_cached）走 `db.plan_cache_len()` 公共 API — 行为不变，预期通过
- **现有 `tests/executor_test.rs`**：5 个 setup 机械调整（删 `Mutex::new` 包裹），预期通过
- **T0（§6）**：7 处 lint 修复 + 3 处 import 清理均不改变运行时行为与公开签名；全部既有测试数量与结果不变
- **现有 487 tests** 基线：保持
- **`Database::execute_sql` 公开 API**：0 变化
- **WAL / 事务 / MVCC**：完全解耦

## 风险与缓解

| 风险 | 严重度 | 缓解 |
|---|---|---|
| `normalize_sql_key` 误折叠字符串字面量内字符 | 高 | 单测 SP3 显式失败则必须修复；toggle 状态机已隔离字符串区 |
| 100 并发测试在 CI 环境卡死 | 中 | 测试设 5s 超时；如有问题可降为 `--test-threads=1` 串行化 |
| DashMap 替换 HashMap 后 plan cache 命中语义差异（get 返回 `Option<PhysicalPlan>` 而非 `Option<&PhysicalPlan>`） | 低 | 调用点 `.cloned()` 去掉即可，API 兼容 |
| `use std::sync::Mutex;` import 残留 | 低 | 已核实：database.rs 的 Mutex 仅用于 plan_cache（line 22/64），T2 后整组改 `use std::sync::{Arc};`；executor_test.rs 顶层 line 14 由 T4 处理、内部三处（903/975/1062）由 T0 处理 |
| plan_cache.rs 旧 `&mut self` 公开方法被外部 crate 直接调用 | 低 | 仅 `pub` 暴露在 crate 内；外部 crate 不依赖（无 lib 发布） |
| T0 修复后暴露表外既有 lint | 低 | lib/test 编译此前被截断；处置规则见 §6 设计取舍第 3 条 |

## 不做（Non-goals）

- 不实现 LRU 精确淘汰（仅简单"满则驱逐一条"）
- 不参数化查询的 plan 缓存（不重写 parser）
- 不持久化 cache
- 不跨 Database 实例共享 cache
- 不解决字符串字面量内含转义引号 `O''Brien` 的 edge case
- 不修改 `Pipeline` 中其他模块（保持 MS06-T04 范围独立）
- 不优化 cache key 哈希（DashMap 默认 SipHash 已足够）
- T0 仅限 §6 表列出的 L1-L7 与 executor_test.rs 三处 import，不做表外清理或重构
