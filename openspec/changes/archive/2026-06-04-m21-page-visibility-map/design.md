## Context

当前 MVCC 可见性检查流程：每次读取一行数据时，从 SlottedPage 的 slot 中解析 22 字节 `VersionHeader`，提取 `create_tx_id` 和 `commit_tx_id`，调用 `Snapshot::is_visible()` 判断。全表扫描（DataScan + Scan + IndexScan）逐行执行此流程。每页通常容纳 30-100 行，每行 22 字节 header 解析 + 2 次 u64 比较 + 1 次 HashSet 查找（active_tx_ids），合计约 50-100ns/行。

引入页面级摘要（`PageVisibilityInfo`），在逐行检查前做 O(1) 页面级判断：
- **全可见**：页上所有行已提交 → 跳过逐行解析
- **全不可见**：页上所有行在快照之后创建 → 跳过整页

## Goals / Non-Goals

**Goals:**
- 减少全表扫描中每行的 `VersionHeader` 解析和 `is_visible` 调用
- INSERT/DELETE/UPDATE 时自动维护可见性摘要，无额外 scan 开销
- 纯内存优化，崩溃安全（丢失后自动降级）

**Non-Goals:**
- 不修改 `Snapshot::is_visible` 逻辑（语义不变）
- 不持久化可见性摘要（不入 WAL，不写盘）
- 不改变跨页版本链查找行为（`find_visible_in_chain` 仍逐版本检查）
- 不处理 GC 后的摘要重建（GC 频率低，代价可接受）

## Decisions

### 决策 1：PageVisibilityInfo 存储位置

**选择**：`BufferPool` 内 `DashMap<PageId, PageVisibilityInfo>`（内存）

**备选方案**：
| 方案 | 优点 | 缺点 | 结果 |
|------|------|------|------|
| A. DashMap in BufferPool | 无页面格式改动；并发读无额外锁；崩溃自动清零 | 需额外内存（~9B/page） | **采用** |
| B. SlottedPageHeader 内 | 随页持久化；无外部分配 | 改 4KB 页面格式影响所有读写；崩溃后数据可能过期 | 拒绝 |
| C. PageFrame 字段 | 和页生命周期绑定 | 需要 `Mutex<PageFrame>` 才能更新，和读路径零拷贝冲突 | 拒绝 |

### 决策 2：min_create_tx_id 语义

**选择**：追踪页上所有行的最小 `create_tx_id`（不含已 GC 删除的旧版本）

**推理**：
- 若 `snapshot.tx_id < min_create_tx_id`，则该页所有行均在该快照**之后**创建 → 整页不可见（乐观跳过）
- 若 `snapshot.tx_id >= min_create_tx_id`，部分行可能可见 → 逐行检查
- 备选 `min_commit_tx_id` 不适用：commit_tx_id 在事务提交时才写入，期间无法用于优化

### 决策 3：all_visible 语义

**选择**：`all_visible` 为 true 当且仅当页上所有行的 `commit_tx_id != UNSET`（无未提交行）

**推理**：
- INSERT 插入未提交行 → `all_visible = false`（clear）
- 所有行提交后 → 不会自动设 `all_visible = true`（等下一读周期遇全可见时顺便设）
- 简化写路径：INSERT/DELETE 只需清标志，不需扫描全页

### 决策 4：all_visible 的"惰性设置"

**选择**：写路径只清 `all_visible`，不设置它；读路径在发现整页可见时**顺便**设 `all_visible = true`

**推理**：
- 避免 COMMIT 时扫描全页（COMMIT 发生在事务提交，需要遍历 `tx_versions` 逐行标记）
- 第一个读到全可见页的扫描器设标志，后续扫描受益
- 线程安全：DashMap `entry` API 的 compare-and-swap 语义保证不丢失更新

### 决策 5：并发安全

**选择**：`DashMap<PageId, PageVisibilityInfo>` + 读取路径无额外锁

**推理**：
- DashMap 默认 64 分片，读操作几乎无争用
- 写路径（INSERT/DELETE/UPDATE）通过 `modify_page` 已有 `Mutex<PageFrame>` 保护
- 可见性摘要的更新在页写入后同步执行，不需要额外同步

## Risks / Trade-offs

| 风险 | 缓解 |
|------|------|
| **INSERT 频繁的页** `all_visible` 始终为 false，优化无效 | 预期收益来自稳态数据（读多写少场景）；写入密集场景退化到逐行检查，无惩罚 |
| **可见性摘要在并发下短暂不一致** | 非关键：错误地将全可见页判为不可见 → 多几次逐行检查（安全）；错误地将不可见页判为全可见 → 仅当 `all_visible` 惰性设置时可能，且条件严格（所有行 commit_tx_id != UNSET），不会导致可见性错误 |
| **崩溃后摘要丢失** | DashMap 纯内存，崩溃后为空。首轮扫描逐行检查，自动重建摘要 |
| **内存开销** | 每页 ~50 字节（DashMap entry 开销 + 9 字节数据），10K 页 ≈ 500KB，可接受 |
