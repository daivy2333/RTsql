# 任务清单：ms08-t01-t02-pread-prefetch

> 关联里程碑：**MS08**（性能压测 / T01 pread-pwrite + T02 prefetch 双缓冲）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 spec：`specs/storage-io-optimization/spec.md`

## Iteration Plan

### Iteration 000: T01 页 I/O 位置参数化（pread/pwrite + 并发正确性）

- Tasks: T1, T2, T3
- Depends on: None（无前置；基线 HEAD = `4d410ac`，577 tests 全绿）
- Stable baseline: 页读写经 pread64/pwrite64 完成（strace lseek 页路径计数 0）；并发冷读不同页正确性有永久回归守卫；577+ 既有测试零修改全绿；before-MS08-T01 bench 基线已落盘
- Verification boundary: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff；`cargo test --all` 0 failures（≥577 + 新增）；strace 验证 syscall 序列；bench 对比结论（改善或明确未达预期）写入 Act Response
- Diagnostic boundary: `src/storage/file_storage.rs`、`tests/file_storage_io_test.rs`
- Deferred tasks: Iteration 001（T02 预取）

### Iteration 001: T02 DataScan 链预取（可选能力，默认关闭）

- Tasks: T1, T2, T3, T4, T5
- Depends on: Iteration 000（页 I/O 底层稳定；bench 基线体系复用）
- Stable baseline: 全表扫描行序/结果与无预取逐行一致（含谓词+LIMIT 组合，开关两态）；链尾无无效预取；预取在途 ≤1；预取默认关闭、`with_prefetch(true)` 显式启用；默认路径 bench 恢复"无变化"（p>0.05）
- Verification boundary: 4 项质量命令全绿；`tests/prefetch_test.rs` 全绿（含默认关闭断言）；`tests/pushdown_test.rs` 15 测试零修改全绿（等价守卫）；`cargo test --all` 0 failures；默认路径 bench 对 before-MS08-T02 无显著变化
- Diagnostic boundary: `src/executor/data_scan.rs`（`next()` 换页路径 + 预取 helper + 构造器开关）、`tests/prefetch_test.rs`
- Deferred tasks: 无（T03 writev 依赖本 change pwrite 底层，另开 change）

## Iteration 000: T01 页 I/O 位置参数化（当前展开 Iteration）

### Task 1: baseline 采集（MS08 前置纪律）

- [x] 1.1 确认 `cargo test --all` 当前全绿（基线健康）
- [x] 1.2 `cargo bench --bench micro_bench --bench data_scan_bench --bench buffer_pool_concurrency_bench --save-baseline before-MS08-T01`（记录总耗时与关键项数值）
- [x] 1.3 strace syscall 计数：对一个代表性测试二进制（如 `--test storage_test` 或最小页读写测试）统计 lseek/read/pread64 计数，作为 before 证据
- [x] 1.4 baseline 数值记入 Act Response（criterion baseline 已由 criterion 落盘 `target/criterion`，Act Response 记录摘要）

### Task 2: TDD—FileStorage 位置参数化改造

- [x] 2.1 新建 `tests/file_storage_io_test.rs`，先写测试：多页往返等价、越界读报错、16 任务并发冷读内容校验（预期 RED）、并发读写混合不串页
- [x] 2.2 运行新测试确认 RED 状态（S1/S3 可能因既有实现已满足而直接 GREEN——记录实际状态；S4 依赖时序，按用户决策接受概率性 RED）
- [x] 2.3 `read_page_blocking` 改 `read_exact_at`、`write_page_blocking` 改 `write_all_at`（`std::os::unix::fs::FileExt`），删除 seek 与 `SeekFrom` 导入
- [x] 2.4 新测试全 GREEN；`cargo build` 0 warning
- [x] 2.5 `cargo clippy -D warnings` / `cargo fmt --check` 通过

### Task 3: 全量回归 + after 证据 + 对比结论

- [x] 3.1 `cargo test --all` 0 failures（≥577 + 新增 file_storage_io_test）
- [x] 3.2 strace after 计数：同 1.3 口径，验证页读路径 lseek = 0、pread64 > 0
- [x] 3.3 `cargo bench --bench micro_bench --bench data_scan_bench --bench buffer_pool_concurrency_bench --baseline before-MS08-T01`，记录量化对比结论（改善 / 未达预期，附环境标注 WSL2）
- [x] 3.4 结论与偏差写入 Act Response；`openspec validate` 本 change 通过

## Iteration 001: T02 DataScan 链预取（展开见目录）

### Task 1: TDD—预取行为测试

- [x] 1.1 新建 `tests/prefetch_test.rs`：多页数据集上全表扫描行序等价（与临时禁用预取对照）、谓词+LIMIT 组合等价、链尾不预取 PageId(0)
- [x] 1.2 确认对照路径（禁用预取）可测试注入——预取开关或对照执行器的最小实现方式由 Act 在契约内决定
- [x] 1.3 运行确认初始状态（预取未实现时等价测试可能全 GREEN——此时测试意义为回归守卫，记录之）

### Task 2: DataScanExecutor 预取实现

- [x] 2.1 `next()` 的 `JumpToPage` 分支后增加预取触发：读取新当前页的 `next_page_id`（需在页数据 closure 内捕获），对下一页 `tokio::spawn` 发起 `buffer_pool.get_page`，结果与错误丢弃
- [x] 2.2 保证同一时刻至多 1 个预取在途（跟踪在途 JoinHandle 或游标状态）
- [x] 2.3 `PageId(0)`（链尾哨兵）不触发预取
- [x] 2.4 新测试 GREEN；既有测试零修改全绿（重点 `tests/pushdown_test.rs`、`tests/executor_test.rs`）

### Task 3: 全量回归 + after 证据

- [x] 3.1 `cargo test --all` 0 failures；`cargo build`/`clippy -D warnings`/`fmt --check` 全 0
- [x] 3.2 `cargo bench --bench data_scan_bench --baseline before-MS08-T02`（Iteration 001 开始时先落 before 基线）
- [x] 3.3 对比结论写入 Act Response；`openspec validate` 通过

### Task 5: 预取默认改关（replan 2026-09-05，NEW-EVIDENCE：默认路径实测回退 +40~47%/+17~18%）

- [x] 5.1 `DataScanExecutor::new` 默认 `prefetch_enabled = false`（一行翻转）；`with_prefetch(true)` 显式启用路径保持不变
- [x] 5.2 `tests/prefetch_test.rs` ON 路径改为 `with_prefetch(true)`；新增默认关闭断言（非空洞性验证：`src/executor/data_scan.rs` 内 `#[cfg(test)]` 单测断言 `new` 默认关闭，或等价可观察手段——形态非实质，须能区分默认与显式开启）
- [x] 5.3 `cargo test --all` 0 failures；clippy/fmt/validate 全 0
- [x] 5.4 `cargo bench --bench data_scan_bench -- --baseline before-MS08-T02`：默认路径（data_scan 两档）恢复"无变化"（p>0.05，对照组 scan_via_index 维持无变化）——1000 档两轮 No change（p=0.66/0.61）；10000 档两轮 p<0.05 但均为改善方向（-3.8%/-3.4%），判读为会话环境漂移（对照组同幅漂移且跨轮不复现），判读细节见 replan Act Response Deviations，待 Plan Review 裁定
- [x] 5.5 对比结论写入 replan Cycle Act Response；修正 docs：proposal What Changes 段补默认关闭说明

## 验收

- R1（页 I/O 位置参数化）：Iteration 000 T2/T3 — 测试四场景 + strace 证据
- R2（零接口零格式变更）：Iteration 000 T3.1 — 577 零修改全绿
- R3（DataScan 预取，默认关闭可选能力）：Iteration 001 T1/T2/T3（开关两态等价 + 在途 ≤1 + 回退实测）+ T5（默认关闭 + 默认路径恢复基线）
