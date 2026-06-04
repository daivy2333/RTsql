## Why

当前 MVCC 可见性检查对每行都要解析 22 字节的 `VersionHeader`（create_tx_id + commit_tx_id + next_version），然后调用 `snapshot.is_visible()` 逐行判断。在全表扫描场景中，瓶颈在页内每行的 header 解析和可见性比较，而非 I/O。引入页面级可见性摘要（每页 9 字节内存开销），在大部分页"全可见"或"全不可见"时跳过逐行检查，预期全表扫描提速 10-15%。

## What Changes

- **新增** `PageVisibilityInfo` 结构（内存），每页追踪 `min_create_tx_id` + `all_visible` 标志
- **新增** `BufferPool` 内 `DashMap<PageId, PageVisibilityInfo>` 维护可见性摘要
- **修改** `find_visible_version` 和 `DataScanExecutor` 扫描路径：逐行检查前先查可见性摘要快速判断
- **修改** INSERT / DELETE / UPDATE 写路径：操作后更新对应页的可见性摘要
- **新增** criterion 基准测试，对比优化前后全表扫描延迟

## Capabilities

### New Capabilities
- `page-visibility-map`: 页面级 MVCC 可见性快速路径 — 用每页 9 字节内存摘要跳过逐行 VersionHeader 解析，在全可见/全不可见页场景下消除 O(slot_count) 的逐行比较开销

### Modified Capabilities
<!-- 不修改现有 spec 行为 — 这是纯实现优化，可见性判断结果不变 -->

## Impact

- **代码层**：`src/storage/buffer_pool.rs`（新增 DashMap + 快速路径）、`src/executor/data_scan.rs`（快速路径集成）、`src/storage/data_page.rs`（INSERT/DELETE 时更新摘要）、`src/executor/update.rs`（UPDATE 时更新）、`benches/` （新增可见性基准）
- **运行时**：纯内存优化，崩溃后自动降级为逐行检查（功能正确性不受影响）
- **内存**：每数据页 +9 字节 DashMap 开销（~10K 页 ≈ 90KB）
- **回滚方案**：`DashMap` 为空时行为等同于无优化（`all_visible=false` + `min_create_tx_id=0`），清空 map 即回滚
