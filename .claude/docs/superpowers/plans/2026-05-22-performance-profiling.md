# 精确性能参数测试实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 通过分阶段计时量化 execute_sql 各环节耗时，定位性能瓶颈

**Architecture:** 环境变量控制 + task_local! 存储测量数据 + 控制台表格输出，最小改动 pipeline.rs 和 IndexScanExecutor

**Tech Stack:** Rust std::time::Instant + tokio::task_local! + HashMap

---

## 文件结构

| 文件 | 作用 | 改动类型 |
|------|------|----------|
| src/profiling.rs | profiling 模块（task_local! + 输出函数） | Create |
| src/pipeline.rs | execute() 添加计时点 | Modify |
| src/executor/index_scan.rs | IndexScanExecutor::next() 添加计时 | Modify |
| examples/bench_minimal.rs | 设置 RTSQL_PROFILING=1 | Modify |
| src/lib.rs | 导出 profiling 模块 | Modify |

---

### Task 1: 创建 profiling 模块

**Files:**
- Create: `src/profiling.rs`
- Modify: `src/lib.rs:1-5`（添加 mod profiling）

**Goal:** 提供全局 task_local 变量存储测量数据和输出函数

- [ ] **Step 1: 创建 src/profiling.rs 文件**

```rust
//! Profiling module for measuring SQL execution pipeline performance
//!
//! M14 Phase 2 T1: Provides task-local storage for timing measurements

use std::collections::HashMap;
use std::time::Duration;
use tokio::task_local;

task_local! {
    pub static PROFILING_DATA: std::cell::RefCell<HashMap<&'static str, Duration>>;
}

/// Initialize profiling data for current task
pub fn init_profiling() {
    PROFILING_DATA.with(|data| {
        *data.borrow_mut() = HashMap::new();
    });
}

/// Record timing for a specific stage
pub fn record_time(stage: &'static str, duration: Duration) {
    PROFILING_DATA.with(|data| {
        data.borrow_mut().insert(stage, duration);
    });
}

/// Get all recorded timings
pub fn get_timings() -> HashMap<&'static str, Duration> {
    PROFILING_DATA.with(|data| {
        data.borrow().clone()
    })
}

/// Print timings table to stderr
pub fn print_timings(total: Duration) {
    let timings = get_timings();
    let total_us = total.as_micros() as f64;

    eprintln!("Stage                    | Time (µs) | % Total");
    eprintln!("-------------------------|-----------|--------");

    // Sort by time descending for clarity
    let mut sorted: Vec<_> = timings.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));

    for (stage, time) in sorted {
        let time_us = time.as_micros() as f64;
        let percent = (time_us / total_us) * 100.0;
        eprintln!("{:23} | {:9.1} | {:6.1}%", stage, time_us, percent);
    }

    eprintln!("-------------------------|-----------|--------");
    eprintln!("{:23} | {:9.1} | {:6.1}%", "Total", total_us, 100.0);
}

/// Check if profiling is enabled via environment variable
pub fn is_profiling_enabled() -> bool {
    std::env::var("RTSQL_PROFILING").is_ok()
}
```

- [ ] **Step 2: 修改 src/lib.rs 导出模块**

在 `src/lib.rs` 开头添加：

```rust
pub mod profiling;
```

位置：在现有模块声明之前（第 1 行附近）

- [ ] **Step 3: 验证模块编译通过**

Run: `cargo check`
Expected: 无错误，profiling 模块编译成功

- [ ] **Step 4: Commit**

```bash
git add src/profiling.rs src/lib.rs
git commit -m "feat(profiling): add profiling module with task_local storage"
```

---

### Task 2: 修改 pipeline.rs 添加计时点

**Files:**
- Modify: `src/pipeline.rs:19-136`（execute 函数）

**Goal:** 在 execute() 函数各环节前后添加计时点

- [ ] **Step 1: 添加 profiling 导入**

在 `src/pipeline.rs` 开头导入区添加：

```rust
use crate::profiling::{is_profiling_enabled, init_profiling, record_time, print_timings};
use std::time::Instant;
```

位置：第 1-11 行导入区末尾

- [ ] **Step 2: 修改 execute() 函数添加计时逻辑**

替换 `execute()` 函数开头（第 19-36 行），添加 profiling 初始化和计时：

```rust
pub async fn execute(database: &Database, sql: &str) -> Response {
    let profiling = is_profiling_enabled();

    if profiling {
        init_profiling();
    }

    let total_start = if profiling { Some(Instant::now()) } else { None };

    // Check plan cache first
    let cached_plan = {
        if profiling {
            let t0 = Instant::now();
            let result = {
                let mut cache = database.plan_cache.lock().unwrap();
                cache.get(sql).cloned()
            };
            record_time("cache_hit_check", t0.elapsed());
            result
        } else {
            let mut cache = database.plan_cache.lock().unwrap();
            cache.get(sql).cloned()
        }
    };

    if let Some(plan) = cached_plan {
        // Cache hit — skip parse + plan
        if profiling {
            record_time("parse_and_plan", Duration::ZERO);
        }

        let executor_start = if profiling { Some(Instant::now()) } else { None };
        let executor = match create_executor_from_plan(plan, database).await {
            Ok(e) => e,
            Err(e) => {
                return Response::Error {
                    message: e.to_string(),
                }
            }
        };
        if profiling {
            record_time("executor_creation", executor_start.unwrap().elapsed());
        }

        let exec_start = if profiling { Some(Instant::now()) } else { None };
        let response = execute_executor(executor).await;
        if profiling {
            record_time("executor_execution", exec_start.unwrap().elapsed());
            print_timings(total_start.unwrap().elapsed());
        }
        return response;
    }

    // Cache miss — parse and plan
    let parse_start = if profiling { Some(Instant::now()) } else { None };
    let statements = match parse_sql(sql) {
        Ok(s) => s,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            }
        }
    };
    if profiling {
        record_time("parse_and_plan", parse_start.unwrap().elapsed());
    }

    if statements.is_empty() {
        return Response::Error {
            message: "Empty SQL".to_string(),
        };
    }

    // Handle the first statement
    if let Some(stmt) = statements.first() {
        match stmt {
            // DDL: CREATE TABLE
            Statement::CreateTable { .. } => {
                let plan = match PlanBuilder::new().build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                let executor: Box<dyn Executor + Send> =
                    Box::new(CreateTableExecutor::new(plan, Arc::new(database.clone())));
                let response = execute_executor(executor).await;

                database.plan_cache.lock().unwrap().clear();

                if profiling {
                    print_timings(total_start.unwrap().elapsed());
                }

                return response;
            }

            // DDL: DROP TABLE
            Statement::Drop { .. } => {
                let plan = match PlanBuilder::new().build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                let executor: Box<dyn Executor + Send> =
                    Box::new(DropTableExecutor::new(plan, Arc::new(database.clone())));
                let response = execute_executor(executor).await;

                database.plan_cache.lock().unwrap().clear();

                if profiling {
                    print_timings(total_start.unwrap().elapsed());
                }

                return response;
            }

            // Query, Insert, Update, Delete
            _ => {
                let table_lookup_start = if profiling { Some(Instant::now()) } else { None };
                let mut plan_builder = PlanBuilder::new();
                if let Err(e) = register_table(database, &mut plan_builder, stmt).await {
                    return Response::Error { message: e };
                }
                if profiling {
                    record_time("table_metadata_lookup", table_lookup_start.unwrap().elapsed());
                }

                let plan = match plan_builder.build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                if is_cacheable(stmt) {
                    let mut cache = database.plan_cache.lock().unwrap();
                    cache.put(sql.to_string(), plan.clone());
                }

                let executor_start = if profiling { Some(Instant::now()) } else { None };
                let executor = match create_executor_from_plan(plan, database).await {
                    Ok(e) => e,
                    Err(e) => {
                        return Response::Error {
                            message: e.to_string(),
                        }
                    }
                };
                if profiling {
                    record_time("executor_creation", executor_start.unwrap().elapsed());
                }

                let exec_start = if profiling { Some(Instant::now()) } else { None };
                let response = execute_executor(executor).await;
                if profiling {
                    record_time("executor_execution", exec_start.unwrap().elapsed());
                    print_timings(total_start.unwrap().elapsed());
                }
                return response;
            }
        }
    }

    Response::Error {
        message: "No statement executed".to_string(),
    }
}
```

**注意**: 保留了原有逻辑，只在关键环节前后插入计时点，不改变执行路径。

- [ ] **Step 3: 验证编译通过**

Run: `cargo check`
Expected: 无错误，pipeline.rs 编译成功

- [ ] **Step 4: Commit**

```bash
---

### Task 3: 修改 IndexScanExecutor 添加内部计时

**Files:**
- Modify: `src/executor/index_scan.rs`（IndexScanExecutor::next）

**Goal:** 测量 IndexManager.search 时间，区分 executor.next() overhead

- [ ] **Step 1: 添加 profiling 导入**

在 `src/executor/index_scan.rs` 开头添加：

```rust
use crate::profiling::{is_profiling_enabled, record_time};
use std::time::Instant;
```

- [ ] **Step 2: 修改 IndexScanExecutor::next() 添加计时**

找到 `IndexScanExecutor::next()` 方法，在 `self.index_manager.search()` 调用前后添加计时：

```rust
async fn next(&mut self) -> Result<Option<ExecResult>> {
    if self.iteration == 0 {
        // First iteration — perform index search
        let profiling = is_profiling_enabled();

        let search_result = if profiling {
            let t0 = Instant::now();
            let result = self.index_manager.search(&self.key).await;
            record_time("index_manager_search", t0.elapsed());
            result
        } else {
            self.index_manager.search(&self.key).await
        };

        match search_result {
            Some(row_id) => {
                self.row_id = Some(row_id);
                self.iteration = 1;

                // Fetch the actual row from data page
                let page_guard = self.buffer_pool.get_page(row_id.page_id()).await?;
                let page_data = page_guard.page_data();
                let slotted_page = SlottedPageRef::new(page_data);
                let tuple = slotted_page.get_tuple(row_id.slot_id())?;

                // Deserialize into values
                let values = deserialize_tuple(&tuple, &self.table_meta.columns)?;
                self.iteration = 2; // Mark as completed
                Ok(Some(ExecResult::Row(values)))
            }
            None => {
                // Key not found
                Ok(None)
            }
        }
    } else {
        // Already returned result or exhausted
        Ok(None)
    }
}
```

**注意**: 只在第一次 iteration（实际搜索时）添加计时，后续 iteration 直接返回 None。

- [ ] **Step 3: 验证编译通过**

Run: `cargo check`
Expected: 无错误，index_scan.rs 编译成功

- [ ] **Step 4: Commit**

```bash
git add src/executor/index_scan.rs
git commit -m "feat(profiling): add timing to IndexScanExecutor next()"
```

---

### Task 4: 修改 bench_minimal.rs 启用 profiling

**Files:**
- Modify: `examples/bench_minimal.rs:1-31`

**Goal:** 设置环境变量启用 profiling，减少测量次数避免输出过多

- [ ] **Step 1: 添加环境变量设置**

在 `examples/bench_minimal.rs` 开头添加：

```rust
use rtsql::database::Database;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    // Enable profiling
    std::env::set_var("RTSQL_PROFILING", "1");

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let db = Database::open(&db_path).await.unwrap();
    std::mem::forget(dir);

    db.execute_sql("CREATE TABLE bench (id INTEGER PRIMARY KEY, val TEXT)").await;
    for i in 0..100i64 {
        db.execute_sql(&format!("INSERT INTO bench VALUES ({}, 'hello')", i)).await;
    }

    // Warm up (触发 plan cache)
    for _ in 0..100 {
        db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
    }

    // Measure only 10 iterations to avoid excessive output
    let iterations = 10;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
    }
    let elapsed = start.elapsed();
    let per_query = elapsed / iterations;
    println!("\n=== Summary ===");
    println!("PK lookup (avg): {:?}", per_query);
    println!("Total: {:?} for {} iterations", elapsed, iterations);
}
```

**改动**:
- 添加 `std::env::set_var("RTSQL_PROFILING", "1")`
- 减少测量次数从 1000 → 10（避免输出过多）
- 添加汇总输出分隔符

- [ ] **Step 2: 验证编译通过**

Run: `cargo check --examples`
Expected: 无错误，bench_minimal.rs 编译成功

- [ ] **Step 3: Commit**

```bash
git add examples/bench_minimal.rs
git commit -m "feat(profiling): enable profiling in bench_minimal example"
```

---

### Task 5: 运行 profiling 测试并验证结果

**Files:**
- None（运行测试）

**Goal:** Gate 5 验证，确认输出符合预期，能识别瓶颈

- [ ] **Step 1: 运行 profiling 测试**

Run: `cargo run --example bench_minimal`
Expected: 输出包含多次计时表格（每次查询一张表）+ 最后汇总

输出示例：
```
Stage                    | Time (µs) | % Total
-------------------------|-----------|--------
executor_execution       | 30.3      | 89.1%
index_manager_search     | 25.0      | 73.5%
executor_creation        |  2.0      |  5.9%
table_metadata_lookup    |  1.5      |  4.4%
cache_hit_check          |  0.2      |  0.6%
parse_and_plan           |  0.0      |  0.0%
-------------------------|-----------|--------
Total                    | 34.0      | 100%

=== Summary ===
PK lookup (avg): ~34µs
Total: ~340µs for 10 iterations
```

- [ ] **Step 2: 验证瓶颈识别**

检查输出中：
- `index_manager_search` 占比是否 > 70%（预期是主要瓶颈）
- `executor_execution` 是否包含 `index_manager_search` 时间
- `cache_hit_check` 和 `parse_and_plan` 是否接近 0µs（cache hit 场景）

- [ ] **Step 3: 运行完整测试确保无破坏**

Run: `cargo test --lib`
Expected: 全量测试通过，profiling 代码不破坏原有功能

- [ ] **Step 4: Gate 5 验证记录**

记录验证结果到 `.claude/docs/snapshot.md`：

```markdown
## M14 Phase 2 T1 验证结果

运行 `cargo run --example bench_minimal`（RTSQL_PROFILING=1）：

[粘贴实际输出]

瓶颈定位：
- IndexManager.search: XXµs (XX%)
- Executor creation: XXµs (XX%)
- Table metadata lookup: XXµs (XX%)

结论：[根据结果判断下一步优化方向]
```

- [ ] **Step 5: Commit 最终状态**

```bash
git add .claude/docs/snapshot.md .claude/docs/tasks.md
git commit -m "feat(M14-T1): complete profiling implementation and validation"
```

---

## 自检清单

**1. Spec 覆盖检查**:
- ✅ 测量环节：cache_hit_check, table_metadata_lookup, executor_creation, executor_execution, index_manager_search
- ✅ 输出格式：控制台表格 + 百分比
- ✅ 测试场景：Cache hit + 单叶节点
- ✅ 环境变量控制：RTSQL_PROFILING=1
- ✅ task_local! 存储方案

**2. Placeholder 扫描**:
- ✅ 无 TBD/TODO
- ✅ 所有代码步骤包含完整代码块
- ✅ 所有命令包含具体命令和预期输出

**3. 类型一致性检查**:
- ✅ profiling 模块函数签名一致
- ✅ task_local! 使用 std::cell::RefCell<HashMap>
- ✅ Instant::now() 和 elapsed() 类型匹配

---

## 后续优化决策（基于 profiling 结果）

根据 Task 5 的输出结果：

**如果 index_manager_search > 70%**:
- 优先启用 IndexManager async search 路径（修改 index_manager.rs）
- 或消除 spawn_blocking 调度瓶颈（专用线程池）

**如果 executor_creation > 20%**:
- 优化 create_executor_from_plan（executor cache）

**如果 table_metadata_lookup > 20%**:
- 优化 table_manager.get_table（元数据缓存）

**如果 cache_hit_check > 5%**:
- 优化 plan_cache.lock()（减少锁争用）