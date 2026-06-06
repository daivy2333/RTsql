# buffer-pool-concurrency Specification

> 版本：v1.0 | 最后更新：2026-06-06
> 归属：M31 (Phase 3 入口) — 来自 `2026-06-06-m31-bufferpool-dashmap-semaphore`

## Purpose

定义 BufferPool 在并发场景下的行为契约：

- **cache hit 路径无锁**：多线程并发 `get_page` 同一已缓存 page 不产生 RwLock/Mutex 争用
- **cache miss 路径有界并发**：in-flight miss load ≤ 16，防止 IO 风暴
- **淘汰正确性**：clock 算法 + ref_count 保护 + dirty 回写
- **内存模型保证**：所有公开 API 的并发行为契约

覆盖 M31 实施后的需求级行为，独立于内部实现细节（具体 DashMap/Semaphore 选型见 `openspec/changes/archive/2026-06-06-m31-bufferpool-dashmap-semaphore/design.md`）。

## Requirements

### Requirement: Concurrent cache hit is lock-free

BufferPool 在 cache hit 路径 SHALL 完全无锁 — 多线程并发 `get_page` 同一已缓存 page SHALL 不产生 RwLock/Mutex 争用，延迟 SHALL 与单线程一致。

#### Scenario: Multiple threads get same cached page

- **GIVEN** BufferPool 容量 ≥ 1 且 page_id=42 已缓存
- **WHEN** 16 个并发 task 各自调用 `get_page(42)`
- **THEN** 全部 16 个 task SHALL 在 ≤ 1µs 内返回相同 `PageGuard`（引用同一 `Arc<Mutex<PageFrame>>`）
- **AND** 任意 task SHALL 持有 PageGuard 时无其他 task 被阻塞

#### Scenario: Cache hit and miss on different pages

- **GIVEN** BufferPool 容量 ≥ 2，page_id=1 已缓存，page_id=2 未缓存
- **WHEN** thread A 调用 `get_page(1)` 同时 thread B 调用 `get_page(2)`
- **THEN** A SHALL 立即返回（cache hit）
- **AND** B SHALL 触发 miss 加载（无 false sharing 阻塞 A）

### Requirement: Bounded in-flight cache miss loading

BufferPool SHALL 限制并发 in-flight cache miss 加载数不超过 16，防止高并发下未命中页同时触发 IO 风暴。

#### Scenario: Concurrent miss requests are throttled

- **GIVEN** BufferPool 容量 = 1，所有 page 未缓存
- **WHEN** 100 个并发 task 同时调用 `get_page(0..100)`（不同 page_id）
- **THEN** 任意时刻 in-flight miss 加载数 SHALL ≤ 16
- **AND** 所有 100 个 task SHALL 最终成功（无 permit 饥饿）
- **AND** 总加载次数 SHALL 恰好 100（无重复加载）

#### Scenario: Semaphore permit is released on error

- **GIVEN** miss semaphore 当前已占用 16/16
- **WHEN** 存储层 `read_page` 返回错误
- **THEN** 对应 permit SHALL 被释放（不在 scope 末尾 leak）
- **AND** 其他等待 task SHALL 立即获得 permit 继续

### Requirement: Double-checked single loading per page

BufferPool SHALL 保证同一 `page_id` 在并发 miss 请求下仅触发 1 次存储加载（double-check 模式）。

#### Scenario: Concurrent miss for same page loads once

- **GIVEN** BufferPool 容量 ≥ 1，page_id=42 未缓存
- **WHEN** 8 个并发 task 同时调用 `get_page(42)`，且 miss semaphore permit 数充足
- **THEN** `storage.read_page(42)` SHALL 被调用恰好 1 次
- **AND** 全部 8 个 task SHALL 返回引用同一 `Arc<Mutex<PageFrame>>` 的 `PageGuard`

#### Scenario: Loading happens after permit acquisition

- **GIVEN** page_id=42 未缓存，miss semaphore 当前已占用 15/16
- **WHEN** thread A 调用 `get_page(42)` 获得最后 1 个 permit
- **THEN** A SHALL 在持有 permit 状态下执行 `storage.read_page(42)`
- **AND** 加载完成后 permit 自动释放

### Requirement: Clock eviction with refcount protection

BufferPool SHALL 在容量满时按 clock 算法淘汰页，受 `ref_count` 和 `clock_bit` 保护；淘汰失败 SHALL 返回 `BufferPoolFull` 错误。

#### Scenario: Eviction triggered when full

- **GIVEN** BufferPool 容量 = 1，page_id=1 已缓存
- **WHEN** `get_page(2)` 被调用
- **THEN** `evict_one` SHALL 被触发
- **AND** page_id=1 SHALL 被从 `pages` 和 `clock_hand` 中移除
- **AND** page_id=2 SHALL 被加载并插入 `pages` 和 `clock_hand`

#### Scenario: All pages pinned returns BufferPoolFull

- **GIVEN** BufferPool 容量 = 1，page_id=1 已缓存且被某 PageGuard 持有（ref_count > 0）
- **WHEN** `get_page(2)` 被调用触发 `evict_one`
- **THEN** `evict_one` SHALL 跳过 page_id=1（ref_count > 0）放回 clock_hand
- **AND** `evict_one` SHALL 在 `max_attempts = clock_hand.len() * 2` 次尝试后放弃
- **AND** `get_page(2)` SHALL 返回 `Err(BufferPoolFull)`

#### Scenario: Dirty page is flushed before eviction

- **GIVEN** BufferPool 容量 = 1，page_id=1 已缓存且 `dirty=true`
- **WHEN** `evict_one` 选中 page_id=1 淘汰
- **THEN** `storage.write_page(1, &page)` SHALL 在 `pages.remove` 之前被调用
- **AND** 写盘失败 SHALL 传播错误，`pages.remove` 不执行

### Requirement: Storage errors propagate to caller

BufferPool SHALL 将底层 `AsyncStorage` 错误无修改地传播给调用方，不吞错。

#### Scenario: read_page error returns to caller

- **GIVEN** page_id=42 未缓存
- **WHEN** `storage.read_page(42)` 返回 `Err(IoError)`
- **THEN** `get_page(42)` SHALL 返回相同 `Err`
- **AND** `pages` SHALL 不会被插入新 entry
- **AND** `clock_hand` SHALL 不会被追加 page_id=42

### Requirement: Invalid capacity returns error

BufferPool `new(0, _)` SHALL 返回 `StorageError::InvalidCapacity(0)`，与 M31 改造前行为一致。

#### Scenario: Zero capacity is rejected

- **GIVEN** 任何 `Arc<dyn AsyncStorage>` 实例
- **WHEN** `BufferPool::new(0, storage)` 被调用
- **THEN** SHALL 返回 `Err(StorageError::InvalidCapacity(0))`
- **AND** 不分配任何内部状态

### Requirement: free_page is safe under concurrent get_page

BufferPool `free_page` SHALL 与并发 `get_page` 安全共存，不破坏 DashMap 不变量或导致悬垂引用。

#### Scenario: free_page then immediate get_page reloads

- **GIVEN** page_id=42 已缓存
- **WHEN** thread A 调用 `free_page(42)`，thread B 紧接着调用 `get_page(42)`
- **THEN** A SHALL 完成后 B 的 `get_page` SHALL 触发新一次 `storage.read_page(42)`
- **AND** B SHALL 返回有效 `PageGuard`（不悬垂）

#### Scenario: Concurrent free_page and get_page on different pages

- **GIVEN** page_id=1 和 page_id=2 都已缓存
- **WHEN** thread A 调用 `free_page(1)`，thread B 调用 `get_page(2)`
- **THEN** A 和 B SHALL 各自独立完成
- **AND** `pages` DashMap 不变量 SHALL 保持（无悬垂 entry）

### Requirement: Miss semaphore backpressure is non-fatal

BufferPool miss semaphore 满 SHALL 表现为 task 等待而非失败；permit 永远 SHALL 在合理时间内可获得。

#### Scenario: Burst of 1000 miss requests all complete

- **GIVEN** BufferPool 容量 = 100，page_id=0..1000 都未缓存
- **WHEN** 1000 个并发 task 各自调用 `get_page(0..1000)` 不同 page_id
- **THEN** 全部 1000 个 task SHALL 在合理时间（≤ 10s 测试环境）内成功返回
- **AND** 任意 task SHALL 永远不因 permit 不足而永久阻塞

#### Scenario: Hit path never blocks on miss semaphore

- **GIVEN** miss semaphore 已占用 16/16（满）
- **WHEN** thread A 调用 `get_page(已缓存_page_id)`
- **THEN** A SHALL 立即返回（cache hit 路径不 acquire permit）
- **AND** A SHALL 永远不等待 permit 释放

