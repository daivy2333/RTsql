## Context

**背景**：`RTsql` 事务子系统使用 `TransactionId` 结构分配全局递增事务 ID。高并发事务压力下，旧 `Mutex<u64>` 实现的锁争用会限制吞吐。当前 main 分支已落地 AtomicU64 重构（`src/transaction/tx_id.rs:4-15`），但缺少量化证据：Mutex vs AtomicU64 的性能差异在不同线程数下呈何种曲线。

**现状**：
- `tx_id.rs`：`AtomicU64` + `fetch_add(1, Ordering::SeqCst)`，单线程 + 10 线程并发单元测试
- `manager.rs:78-94 begin()`：复用 `allocate()` 生成 `tx_id` 并作为 Snapshot 时间戳
- `benches/`：6 套现有基准（micro/concurrent/scale/sqlite_compare/single/precise_compare），**无** tx_id 相关

**约束**：
- 只能在 `benches/` 下加新文件，不修改 `src/` 任何代码
- criterion 已在 `[dev-dependencies]`，无新增依赖
- 黑盒函数必须 `pub`，避免内部调用被优化掉

**Stakeholders**：
- 性能优化路线图（tasks.md M41）：需要量化收益
- 后续优化（M40 RowLock、M31 BufferPool）：参考基线

## Goals / Non-Goals

**Goals**：
- 建立 `benches/tx_id.rs`，4 个场景：单线程延迟 / 10 线程争用 / 100 线程高争用 / 稳态吞吐
- 量化 `Mutex<u64>` vs `AtomicU64` 的分配延迟（ns/op）和吞吐量（ops/sec）
- 输出可对比的 criterion 报告（HTML/CSV），可被 CI 读取
- 确认/推翻 tasks.md 路线图"100ns → 10ns"的预期

**Non-Goals**：
- 不修改 `TransactionId` API
- 不修改 `TransactionManager::begin`
- 不引入新依赖（criterion 已存在）
- 不做 CI 性能门禁（留给后续 phase）
- 不重做 Snapshot 时间戳设计（确认当前 `tx_id` 即时间戳，无独立 counter 需改造）

## Decisions

### 决策 1：基准文件位置 → `benches/tx_id.rs`

**理由**：
- 项目已用 criterion 6 套基准，新文件遵循相同惯例
- 单一文件聚焦"事务 ID 分配"主题，避免污染 micro 套件
- Cargo.toml 已有 `[[bench]]` 模式可参考

**替代方案**：
- 加到 `benches/micro.rs`：与现有 micro 套件混杂，主题不清
- 加到 `benches/concurrent.rs`：偏向 IO 路径争用，与本基准（计数器争用）语义不符
- 新建 `benches/atomic.rs`：未来若引入更多无锁基元可扩展，但当前无必要

### 决策 2：场景设计 → 4 场景覆盖单线程→高争用全曲线

**理由**：
- 单线程：建立基线（无争用状态下 Atomic 优势消失）
- 10 线程：模拟中等并发（PG 默认 max_connections=64 之下的子集）
- 100 线程：高争用上限（暴露 Mutex 排队代价）
- 稳态吞吐：单线程极限速率（对比指标更稳定）

**替代方案**：
- 只跑单线程：无法暴露争用收益，价值低
- 跑 1k 线程：超出项目实际负载（DB 不会同时有 1k 事务分配），无意义
- 跑真实事务混合：变量过多，CPU cache/IO 干扰 Atomic vs Mutex 差异

### 决策 3：黑盒函数 → `pub fn` + 隔离分配器实例

**理由**：
- 防止编译器内联 `fetch_add` 消除真实开销
- 每次 `c.bench_function` 重新构造 `Arc<Mutex<u64>>` / `Arc<AtomicU64>`，状态独立
- 多线程场景用 `std::thread::spawn`（criterion 标准并发模式），不依赖 rayon

**替代方案**：
- 用 rayon：引入新依赖（项目无 rayon），不必要
- 共享全局状态：会引入跨 benchmark 的状态污染
- 用 tokio::spawn：async runtime 开销会淹没 Atomic vs Mutex 差异

### 决策 4：测量单位 → `c.bench_function` 默认 wall-clock

**理由**：
- criterion 默认配置合理（采样 + 统计 + 离群值检测）
- 显式 `Measurement::WallTime` 与 `Measurement::ProcessTime` 对照（前者含调度，后者纯 CPU），本基准关心实际事务延迟，选 wall-clock
- 单次采样迭代次数 `throughput(Throughput::Elements(N))` 让 criterion 自动算 ops/sec

## Risks / Trade-offs

- [基准数据易受环境干扰] → 在 CI 中固定 `--features release`，注明"结果仅作相对对比"
- [Criterion 输出非确定] → 每个场景独立运行 3 次取中位数（criterion 默认），对比报告手动取 delta
- [Mutex vs AtomicU64 在单线程下差异可能 < 5%] → 单线程场景作为对照基线，差异由高争用场景体现
- [未覆盖真实事务负载] → 明确 Non-Goals，不替代 e2e 压测
- [黑盒函数可能被 inlining 优化] → 用 `#[inline(never)]` 标注 + 显式黑盒参数 `Bencher`

## Migration Plan

无（新增文件，不修改现有代码）。回滚：删除 `benches/tx_id.rs` 即可。
