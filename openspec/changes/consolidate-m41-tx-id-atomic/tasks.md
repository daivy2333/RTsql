# 任务清单：consolidate-m41-tx-id-atomic

> 关联里程碑：M41（事务 ID AtomicU64 无锁分配）
> 关联 spec：specs/tx-id-allocation-benchmark/spec.md
> 关联 design：design.md

## 1. 校准历史状态（T1-T3 已落地确认）

- [ ] 1.1 确认 `src/transaction/tx_id.rs:4` 字段为 `counter: AtomicU64`
- [ ] 1.2 确认 `src/transaction/tx_id.rs:15` 使用 `fetch_add(1, Ordering::SeqCst)`
- [ ] 1.3 确认 `src/transaction/tx_id.rs:36-47` 单线程测试存在
- [ ] 1.4 确认 `src/transaction/tx_id.rs:50-64` 多线程测试存在（10 线程）
- [ ] 1.5 确认 `src/transaction/manager.rs:78-94 begin()` 复用 `allocate()` 而非独立时间戳计数器
- [ ] 1.6 跑 `cargo test --lib transaction` 验证现有测试全绿

## 2. 建立 criterion 基准骨架

- [ ] 2.1 检查 `Cargo.toml` 确认 criterion 在 `[dev-dependencies]` 且 `[[bench]]` 模式
- [ ] 2.2 在 `benches/` 下复制现有 micro 套件的 import 头（`use criterion::*` 等）
- [ ] 2.3 创建 `benches/tx_id.rs` 包含 `criterion_group!` 和 `criterion_main!`
- [ ] 2.4 跑 `cargo bench --bench tx_id --no-run` 验证骨架编译通过

## 3. 实现 4 个基准场景

- [ ] 3.1 实现 `bench_mutex_single_thread`：1M 次 `Mutex::lock + *c += 1`
- [ ] 3.2 实现 `bench_atomic_single_thread`：1M 次 `fetch_add(1, SeqCst)`
- [ ] 3.3 实现 `bench_mutex_10_threads`：10 thread × 100K 次，std::thread::spawn 汇总
- [ ] 3.4 实现 `bench_atomic_10_threads`：同上
- [ ] 3.5 实现 `bench_mutex_100_threads`：100 thread × 10K 次
- [ ] 3.6 实现 `bench_atomic_100_threads`：同上
- [ ] 3.7 实现 `bench_mutex_throughput` / `bench_atomic_throughput`：单线程 ops/sec

## 4. 验证与报告

- [ ] 4.1 跑 `cargo bench --bench tx_id` 完整运行（输出 criterion HTML 报告）
- [ ] 4.2 收集 4 场景的 ns/op 数据
- [ ] 4.3 对比 Mutex vs AtomicU64：单线程差异、10 线程 5x、100 线程 3x 假设
- [ ] 4.4 在 `openspec/specs/learned/spec.md` 追加 <!-- L 编号 --> 记录实测延迟数据
- [ ] 4.5 若结果推翻路线图预期（"100ns→10ns"），记录原因分析

## 5. 收尾

- [ ] 5.1 跑 `cargo fmt --all` + `cargo clippy --all-targets` 通过
- [ ] 5.2 `git diff src/` 为空（确认不修改源码）
- [ ] 5.3 `git diff Cargo.toml Cargo.lock | grep '^\+[^+]'` 为空（确认无新增依赖）
- [ ] 5.4 提交 PR：`feat(m41): add tx_id allocation micro-benchmark`
- [ ] 5.5 跑 `openspec archive consolidate-m41-tx-id-atomic`

## 验收标准

| 标准 | 命令 | 预期 |
|------|------|------|
| 文件存在 | `ls benches/tx_id.rs` | exit 0，文件非空 |
| 编译通过 | `cargo bench --bench tx_id --no-run` | "Finished" 无 error |
| 4 场景齐全 | `cargo bench --bench tx_id -- --list` | 列出 8 个 bench 函数 |
| 报告生成 | `ls target/criterion/tx_id/*/report/index.html` | 8 份 HTML |
| 不改 src/ | `git diff --stat src/` | 空 |
| 无新增依赖 | `git diff Cargo.toml \| grep '^+[^+]'` | 空（除开 bench 名）|
