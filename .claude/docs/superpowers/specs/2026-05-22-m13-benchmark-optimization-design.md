# M13: Performance Benchmark & Critical Optimization

> 日期：2026-05-22 | 状态：Draft | 里程碑：M13

## 目标

1. 建立 criterion.rs 性能基准测试框架，量化当前性能
2. 基于 benchmark 数据修复 Critical 性能问题
3. 对比修复前后性能，验证优化效果

## 方案：并行推进（Phase C）

先跑 benchmark 记录基线 → 修复 Critical 问题 → 再跑 benchmark 对比。

## Phase 1: Benchmark 框架 + 基线测量

### 依赖变更

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
rusqlite = "0.31"

[[bench]]
name = "micro_bench"
harness = false

[[bench]]
name = "concurrent_bench"
harness = false

[[bench]]
name = "scale_bench"
harness = false

[[bench]]
name = "sqlite_compare"
harness = false
```

### 文件结构

```
benches/
├── common/
│   └── mod.rs          # 共享辅助：数据库初始化/清理/数据生成
├── micro_bench.rs      # 单操作延迟
├── concurrent_bench.rs # 并发压力
├── scale_bench.rs      # 规模扩展
└── sqlite_compare.rs   # SQLite 对比
```

### 微基准测试（micro_bench.rs）

| 操作 | 说明 | 测量指标 |
|------|------|----------|
| INSERT | 单行插入延迟 | 均值/中位数/P95/P99 |
| SELECT | 主键查询延迟 | 同上 |
| UPDATE | 主键更新延迟 | 同上 |
| DELETE | 主键删除延迟 | 同上 |
| SCAN | 全表扫描延迟 | 同上 |
| FILTER | WHERE 条件过滤延迟 | 同上 |
| SORT | ORDER BY 排序延迟 | 同上 |
| LIMIT | LIMIT 截断延迟 | 同上 |
| JOIN | INNER JOIN 延迟 | 同上 |

每个操作 100+ 次采样，使用 `criterion::BenchmarkGroup`。

### 并发压力测试（concurrent_bench.rs）

| 场景 | 连接数 | 说明 |
|------|--------|------|
| 并发读 | 1/4/8/16/32 | 多连接同时 SELECT |
| 并发写 | 1/4/8/16/32 | 多连接同时 INSERT |
| 混合读写 | 1/4/8/16/32 | 80% 读 + 20% 写 |
| 事务冲突 | 4/8/16 | 并发更新同一行 |

报告吞吐量（ops/s）和 P99 延迟。

### 规模扩展测试（scale_bench.rs）

| 数据量 | 测试操作 |
|--------|----------|
| 1K 行 | INSERT/SELECT/SCAN/JOIN |
| 10K 行 | 同上 |
| 100K 行 | 同上 |
| 1M 行 | 同上 |

报告延迟和吞吐随规模变化。

### SQLite 对比测试（sqlite_compare.rs）

使用 `rusqlite` 执行相同操作，对比：
- 单操作延迟（INSERT/SELECT/UPDATE/DELETE/SCAN/JOIN）
- 并发吞吐（1/4/8/16 连接）
- 规模扩展（1K/10K/100K 行）

### 共享辅助（common/mod.rs）

```rust
// 数据库初始化
pub fn setup_db() -> (PathBuf, Database) { ... }
// 清理
pub fn cleanup_db(path: &Path) { ... }
// 生成测试数据
pub fn generate_rows(n: usize) -> Vec<RowData> { ... }
// 建表
pub fn create_test_table(db: &Database, name: &str) { ... }
```

## Phase 2: Critical 修复

### 2.1 PageGuard 零拷贝

**问题**：`page()` 返回 `Page` clone，每次 4KB 内存分配。

**修复**：
- 新增 `page_data(&self) -> &[u8]`，返回页内数据切片（零分配）
- 保留 `page()` 兼容接口（标记 deprecated）
- `modify_page()` 保持不变（已通过闭包安全修改）

**影响文件**：
- `src/storage/page_frame.rs` — 新增 `page_data()` 方法
- `src/executor/*.rs` — 逐步迁移到 `page_data()`

### 2.2 BufferPool 两阶段锁

**问题**：`get_page()` 持写锁期间做 I/O，阻塞其他协程。

**修复**：两阶段锁模式：
1. 读锁检查 page cache → 命中返回
2. 未命中 → 释放锁 → 异步 I/O 读取磁盘
3. 获取写锁 → double-check（可能已被其他协程加载）→ 插入 cache

**影响文件**：
- `src/storage/buffer_pool.rs` — 重构 `get_page()` 方法

### 2.3 PageGuard Mutex 安全验证

**问题**：`std::sync::Mutex` 跨 `.await` 会导致 UB。

**现状**：`page()`/`modify_page()`/`mark_dirty()` 均为同步操作，不跨 `.await`。

**修复**：
- 验证所有 Mutex 使用点
- 添加 `// SAFETY: Mutex not held across .await` 注释标记
- 如有跨 `.await` 情况，替换为 `tokio::sync::Mutex`

## Phase 3: Benchmark 对比 + 扩展测试

1. 修复后重新运行全部 benchmark
2. 对比修复前后数据（criterion 自动生成对比报告）
3. 运行并发压力 + 规模扩展 + SQLite 对比
4. 记录结果到 `learned.md`

## 成功标准

| 标准 | 验证方式 |
|------|----------|
| criterion 框架可运行 | `cargo bench` 成功 |
| 微基准覆盖 9 种操作 | 检查 bench 输出 |
| 并发测试覆盖 4 种场景 | 检查 bench 输出 |
| 规模测试覆盖 4 个量级 | 检查 bench 输出 |
| SQLite 对比可运行 | 检查 bench 输出 |
| PageGuard 零拷贝 | `page_data()` 无分配 |
| BufferPool 异步化 | 持锁期间无 I/O |
| 修复后性能提升可量化 | criterion 对比报告 |
| 所有现有测试通过 | `cargo test` |
