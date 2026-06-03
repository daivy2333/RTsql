# 规格：事务 ID 分配微基准

> 版本：v1.0
> 最后更新：2026-06-03
> 来源 change：consolidate-m41-tx-id-atomic

## ADDED Requirements

### Requirement: 基准文件存在

系统 MUST 在 `benches/tx_id.rs` 提供 criterion 微基准，文件可被 `cargo bench --bench tx_id` 识别并执行。

#### Scenario: 文件存在
- **WHEN** 运行 `ls benches/tx_id.rs`
- **THEN** 文件存在且非空

#### Scenario: cargo bench 识别
- **WHEN** 运行 `cargo bench --bench tx_id --no-run`
- **THEN** 编译通过，criterion 列出 `tx_id::bench_*` 函数

### Requirement: 单线程延迟对比

系统 MUST 对比 `Mutex<u64>` 与 `AtomicU64` 在单线程 1M 次分配下的平均延迟。

#### Scenario: Mutex 单线程
- **WHEN** `bench_mutex_single_thread` 执行 1,000,000 次 `lock + *c += 1`
- **THEN** criterion 输出 `tx_id/mutex_single_thread` 报告，ns/op

#### Scenario: Atomic 单线程
- **WHEN** `bench_atomic_single_thread` 执行 1,000,000 次 `fetch_add(1, SeqCst)`
- **THEN** criterion 输出 `tx_id/atomic_single_thread` 报告，ns/op

#### Scenario: 单线程差异可对比
- **WHEN** 两个报告生成
- **THEN** 在单线程下 Atomic 与 Mutex 差异 < 20%（作为对照基线）

### Requirement: 10 线程争用对比

系统 MUST 对比两实现在 10 线程 × 100K 次分配（每线程）下的总耗时。

#### Scenario: Mutex 10 线程
- **WHEN** `bench_mutex_10_threads` 启动 10 个 `std::thread`，每线程 100K 次 `lock + *c += 1`
- **THEN** criterion 输出总耗时（ns）

#### Scenario: Atomic 10 线程
- **WHEN** `bench_atomic_10_threads` 启动 10 个 `std::thread`，每线程 100K 次 `fetch_add(1, SeqCst)`
- **THEN** criterion 输出总耗时（ns）

#### Scenario: 10 线程下 Atomic 显著更快
- **WHEN** 两个报告对比
- **THEN** Atomic 总耗时 <= Mutex 总耗时的 50%（5x 加速，验证路线图"~5x 争用收益"）

### Requirement: 100 线程高争用对比

系统 MUST 对比两实现在 100 线程 × 10K 次分配下的总耗时。

#### Scenario: Mutex 100 线程
- **WHEN** `bench_mutex_100_threads` 启动 100 个 `std::thread`，每线程 10K 次分配
- **THEN** criterion 输出总耗时（ns）

#### Scenario: Atomic 100 线程
- **WHEN** `bench_atomic_100_threads` 启动 100 个 `std::thread`，每线程 10K 次分配
- **THEN** criterion 输出总耗时（ns）

#### Scenario: 100 线程下 Atomic 优势扩大
- **WHEN** 两个报告对比
- **THEN** Atomic 总耗时 <= Mutex 总耗时的 30%（≥3x 加速，验证"高争用下锁开销指数级恶化"假设）

### Requirement: 稳态吞吐

系统 MUST 测量单线程极限分配速率（ops/sec）。

#### Scenario: Atomic 稳态吞吐
- **WHEN** `bench_atomic_throughput` 单线程 1M 次分配
- **THEN** criterion 报告 ops/sec

#### Scenario: Mutex 稳态吞吐
- **WHEN** `bench_mutex_throughput` 单线程 1M 次分配
- **THEN** criterion 报告 ops/sec

### Requirement: 不修改 src/

本次变更 MUST NOT 修改 `src/transaction/tx_id.rs`、`src/transaction/manager.rs` 或任何 `src/` 下的文件。

#### Scenario: git diff 干净
- **WHEN** 运行 `git diff src/`
- **THEN** 无输出（仅 `benches/tx_id.rs` 新增 + `Cargo.toml` 不动）

### Requirement: 无新增依赖

本次变更 MUST NOT 在 `Cargo.toml` 的 `[dependencies]` 或 `[dev-dependencies]` 新增 crate。

#### Scenario: cargo tree 干净
- **WHEN** 运行 `git diff Cargo.toml Cargo.lock | grep '^\+[^+]'`
- **THEN** 无新增 crate 行（criterion 已有）
