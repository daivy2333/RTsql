## Why

`next_tx_id()`（实际命名为 `TransactionId::allocate()`）历史上使用 `Mutex<u64>` 保护共享计数器，每次事务开始需获取锁。在高并发场景下，锁争用会显著拖累事务启动延迟（tasks.md 路线图记录：分配延迟 ~100ns，AtomicU64 可降至 ~10ns）。

目前核心改造已完成：`src/transaction/tx_id.rs` 已落地 `AtomicU64` + `fetch_add(1, Ordering::SeqCst)` + 单线程/多线程单元测试。`TransactionManager::begin()` 复用同一计数器作为"事务开始时间戳"（Snapshot 不持独立时间戳字段，tx_id 即时间戳）。**仅缺一项：Mutex vs AtomicU64 的性能对比基准，无法在 CI 中量化"无锁分配"的收益承诺**。

本次变更以"校准 + 补完"为定位，承认 3/4 任务已在 main 上落地，1/4（T4 微基准）为新增工作。

## What Changes

- **新建** `benches/tx_id.rs`：criterion 微基准，对比 `Mutex<u64>` 与 `AtomicU64` 在 4 个场景下的 fetch_add 性能
- **校准** M41 任务清单：T1/T2/T3 标记为已完成（附代码位置 + 测试证据），T4 拆为 4 个子任务
- **不修改** `TransactionId` 的 API、不修改 `begin()`、不引入新依赖（criterion 已在 `Cargo.toml`）
- **不重新设计** Snapshot 时间戳策略（确认当前 `tx_id` 即时间戳的设计无需变更）

## Capabilities

### New Capabilities

- `tx-id-allocation-benchmark`: 事务 ID 分配微基准能力
  - 覆盖 REQUIREMENTS：基准文件路径、对比维度、运行命令、断言门槛
  - 文件位置：`benches/tx_id.rs`

### Modified Capabilities

<!-- 不修改任何现有 spec 的 REQUIREMENTS；T1-T3 已是历史实现，无 spec 层面行为变更 -->
- 无

## Impact

- **新增文件**：`benches/tx_id.rs`（criterion 黑盒函数，4 场景）
- **Cargo.toml**：确认 `criterion` 在 dev-dependencies（已存在，6 套基准已用）
- **CI**：可选集成 `cargo bench --bench tx_id` 性能门禁（不强制，留给后续优化路线图）
- **依赖**：无新增 crate
- **风险**：低；T1-T3 不变，仅追加文件
- **回滚方案**：删除 `benches/tx_id.rs` 即可（不影响主代码）
- **相关 ADR**：无（未涉及架构决策；M41 路线图已在 `tasks.md` 记录）
