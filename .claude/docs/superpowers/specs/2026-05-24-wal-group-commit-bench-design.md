# WAL Group Commit 性能基准测试设计

> 日期：2026-05-24 | 阶段：M18 Phase3 T7

## 目标

验证 WAL Group Commit 相比逐条 fsync 的性能提升，目标 5-10x throughput 加速。

## 方案

独立 WAL 层 benchmark，直接操作 WALBuffer，不经过 SQL 层。

## 文件结构

- 新建 `benches/wal_group_commit_bench.rs`
- `Cargo.toml` 新增 `[[bench]] name = "wal_group_commit_bench" harness = false`

## Benchmark Groups

### Group 1: wal_baseline — 逐条 fsync 基线

- 参数：capacity=1, flush_interval_ms=0
- 模式：单线程顺序 append 1000 条 Insert 记录 + append_commit_and_wait
- 度量：throughput（records/sec）

### Group 2: wal_group_commit — Group Commit 并发吞吐

- 参数：capacity=100, flush_interval_ms=100ms
- 并发：[1, 4, 8, 16, 32] 线程
- 每线程写 200 条记录 + commit（总 200-6400 条）
- 度量：throughput（records/sec），与 baseline 对比计算加速比

### Group 3: wal_capacity_impact — capacity 参数影响

- 参数：capacity [1, 10, 100], 并发=8, flush_interval_ms=100ms
- 每线程写 200 条记录 + commit
- 度量：不同 capacity 下的吞吐变化

## 辅助函数

- `create_wal_buffer(capacity, flush_interval_ms) -> Arc<WALBuffer>` — 构造临时 WAL 文件 + WALBuffer
- `make_insert_record(tx_id, i) -> WalRecord` — 生成测试用 Insert 记录
- `bench_cleanup(wal_buffer)` — shutdown WALBuffer + 清理临时文件

## 测试参数

- 每组写入量：1000 条记录（baseline）/ 每线程 200 条（并发组）
- criterion 配置：sample_size=50, measurement_time=10s

## 预期结果

- Group Commit (capacity=100, 8+ threads) vs Baseline (capacity=1) → 5-10x throughput 提升
- capacity 从 1→10→100 吞吐递增

## 场景覆盖

| 场景 | Group | 覆盖 |
|------|-------|------|
| 逐条 fsync 基线 | wal_baseline | ✅ |
| Group Commit 并发吞吐 | wal_group_commit | ✅ |
| capacity 参数影响 | wal_capacity_impact | ✅ |
| 不同并发级别 | wal_group_commit (1-32) | ✅ |
| 单条 commit 延迟 | wal_baseline (per-iter) | ✅ |
