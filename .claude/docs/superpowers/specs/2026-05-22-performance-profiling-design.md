# 精确性能参数测试设计

> 2026-05-22 | M14 Phase 2 T1

## 目标

通过分阶段计时量化 `execute_sql` 各环节的耗时占比，定位真实性能瓶颈，为后续优化提供精确数据支撑。

## 测试场景

**Cache hit + 单叶节点**（理想情况）：
- 100 行数据，单叶 BTree
- 预热 100 次触发 plan cache
- BufferPool 全命中
- 目标：测量最理想执行路径，定位剩余瓶颈

预期结果：
- Cache hit 后跳过 parse/plan（~0µs）
- Executor execution 是主要瓶颈（预期 ~30µs）
- IndexManager.search 占 executor execution 大头（预期 ~25µs）

## 测量环节

### 外层环节（pipeline.rs）

| 环节 | 计时点 | 预期耗时 |
|------|--------|----------|
| Cache hit check | plan_cache.lock().unwrap().get(sql) | ~0.2µs |
| Table metadata lookup | table_manager.get_table(&table_name).await | ~1.5µs |
| Executor creation | create_executor_from_plan().await | ~2µs |
| Executor execution | execute_executor().await 循环 | ~30µs |

### 内层环节（executor）

**IndexScanExecutor**（PK lookup）需要在 `next()` 内部测量：
- IndexManager.search（包含 spawn_blocking + BTree search）
- 其他开销（executor.next() 循环 overhead）

**测量方式**：在 IndexScanExecutor::next() 中插入计时点，区分：
- IndexManager.search 时间
- 其他开销（结果构造、loop overhead）

## 技术方案

### 方案 A：修改 pipeline.rs 添加可选计时（已选择）

**实现要点**：
1. 在 `execute()` 函数开头检查环境变量 `RTSQL_PROFILING=1`
2. 启用时在各环节前后调用 `std::time::Instant::now()`
3. 累积时间，在函数结束时输出表格到 stderr
4. 不修改默认行为（RTSQL_PROFILING 未设置时无开销）

**代码结构**：
```rust
pub async fn execute(database: &Database, sql: &str) -> Response {
    let profiling = std::env::var("RTSQL_PROFILING").is_ok();

    if profiling {
        let start = Instant::now();
        let mut timings = HashMap::new();

        // Cache hit check
        let t0 = Instant::now();
        let cached_plan = { ... };
        timings.insert("cache_hit_check", t0.elapsed());

        // ... 其他环节类似

        // 输出表格
        print_timings(&timings, start.elapsed());
    }

    // 正常执行逻辑
    ...
}
```

**性能开销**：
- Instant::now() 约 ~50ns/次，测量期间可接受
- 未启用时无额外开销（环境变量检查 ~1µs）

### IndexScanExecutor 内部测量

**修改 IndexScanExecutor::next()**：
```rust
async fn next(&mut self) -> Result<Option<ExecResult>> {
    let profiling = std::env::var("RTSQL_PROFILING").is_ok();

    if profiling {
        let t0 = Instant::now();
        let result = self.index_manager.search(&self.key).await;
        let search_time = t0.elapsed();

        // 输出或累积到全局状态
        eprintln!("IndexManager.search: {:?}", search_time);

        // 构造结果
        ...
    } else {
        // 正常逻辑
        ...
    }
}
```

**传递测量数据**：
- 方案 1：全局静态变量（ thread_local! + Cell<HashMap>）
- 方案 2：修改 Executor trait，添加 profiling context 参数（影响面大）

**选择方案 1**：全局 thread_local 变量，最小改动。

## 输出格式

控制台表格（stderr）：
```
Stage                    | Time (µs) | % Total
-------------------------|-----------|--------
Cache hit check          |  0.2      |  0.6%
Table metadata lookup    |  1.5      |  4.4%
Executor creation        |  2.0      |  5.9%
Executor execution       | 30.3      | 89.1%
  - IndexManager.search  | 25.0      | 73.5%
  - executor.next loop   |  5.3      | 15.6%
-------------------------|-----------|--------
Total                    | 34.0      | 100%
```

**实现函数**：
```rust
fn print_timings(timings: &HashMap<&str, Duration>, total: Duration) {
    let total_us = total.as_micros() as f64;
    eprintln!("Stage                    | Time (µs) | % Total");
    eprintln!("-------------------------|-----------|--------");
    for (stage, time) in timings {
        let time_us = time.as_micros() as f64;
        let percent = (time_us / total_us) * 100.0;
        eprintln!("{:23} | {:9.1} | {:6.1}%", stage, time_us, percent);
    }
    eprintln!("-------------------------|-----------|--------");
    eprintln!("{:23} | {:9.1} | {:6.1}%", "Total", total_us, 100.0);
}
```

## 测试工具

**修改 examples/bench_minimal.rs**：
- 设置环境变量 `RTSQL_PROFILING=1`
- 保持预热 + 测量逻辑
- 输出会包含每次查询的详细计时（可只测量单次或少量次数以避免输出过多）

**或创建新 examples/profiling.rs**：
- 更精确控制测量次数（如只测量 10 次）
- 汇总平均耗时

选择：先修改 bench_minimal.rs，验证可行后再考虑独立工具。

## 成功标准

**Gate 5 验证**：
1. 运行 `RTSQL_PROFILING=1 cargo run --example bench_minimal`
2. 输出包含完整计时表格
3. 能清晰识别瓶颈环节（预期 IndexManager.search > 70%）

**后续优化决策**：
- 如果 IndexManager.search > 70%：优先启用 async search 路径或消除 spawn_blocking
- 如果 Executor creation > 20%：优化 create_executor_from_plan（如 executor cache）
- 如果 Table metadata lookup > 20%：优化 table_manager.get_table（如元数据缓存）

## 约束与风险

**Karpathy 原则遵守**：
- ✅ Think Before Coding：已完成设计
- ✅ Implementation Simplicity：环境变量控制 + thread_local 变量，最小改动
- ✅ Surgical Changes：只修改 pipeline.rs 和 IndexScanExecutor，不重构其他代码
- ✅ Requirements Integrity：测量环节覆盖所有关键路径，无裁剪

**风险**：
- thread_local 变量在 async 上下文中可能跨线程迁移（Tokio multi-thread scheduler）
  - 解决：使用 tokio::task_local! 替代 thread_local!
- 测量输出可能干扰 benchmark（stderr 输出）
  - 解决：只在 profiling 模式下输出，benchmark 时禁用

## 后续步骤

1. 实现测量代码（pipeline.rs + IndexScanExecutor）
2. 运行 profiling 测试
3. 根据结果定位瓶颈
4. 进入 T8: 全量 benchmark 验证
5. 根据瓶颈定向优化（启用 async search 或其他）