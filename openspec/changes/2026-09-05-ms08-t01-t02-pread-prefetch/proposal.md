# proposal: MS08-T01+T02 页 I/O 位置参数化与扫描预取

## Why

MS08 验证边界要求"前置 baseline 留档 AND 实施后量化改善 OR 明确记录未达预期"。本 change 覆盖 MS08 前两个任务：T01（pread/pwrite，无前置）与 T02（prefetch 双缓冲，前置 MS02-T02/T03 已完成）。两者都作用于同一条顺序页访问路径，合并规划（用户 2026-09-05 决策）。

- **T01 syscall 冗余**：`read_page_blocking` / `write_page_blocking`（`src/storage/file_storage.rs:53-77`）每页执行 `seek(SeekFrom::Start) + read_exact/write_all`，即每页 2 次 syscall（lseek + read/write）。`FileExt::read_exact_at` / `write_all_at`（pread64/pwrite64）每次页 I/O 只需 1 次 syscall，且无需维护文件偏移。
- **T01 共享偏移竞态（Explorer 2026-09-05 会话发现，未实测复现）**：`file: Arc<std::fs::File>` 无互斥保护，`seek` 作用于共享 file description 偏移。`BufferPool` 允许最多 16 个并发 miss 加载不同页（`src/storage/buffer_pool.rs:16` `MISS_SEMAPHORE_PERMITS=16`，per-page loading lock 只串行化同页加载），两个并发 `read_page` 交错 seek 后存在错读窗口（A seek 到页 X 后 B seek 到页 Y，A 实际读到页 Y 的内容）。现有 577 tests 无"多任务并发冷读不同页并校验内容"场景。pread 是位置参数调用，结构性消除该竞态。
- **T02 扫描停顿**：`DataScanExecutor::next()`（`src/executor/data_scan.rs:162-324`）沿 `SlottedPageHeader.next_page_id` 链逐页推进：每页消耗完（`slot_index >= slot_count`，`data_scan.rs:205-212`）才通过 `JumpToPage` 发起下一页的 `with_page_data` → miss 时同步 `get_page` 加载——页与页之间是串行"用完再取"。双缓冲预取让下一页的加载与当前页的行处理重叠，隐藏 miss 延迟。

## What Changes

- **Iteration 000（T01）**：`FileStorage::read_page_blocking` / `write_page_blocking` 改用 `std::os::unix::fs::FileExt::{read_exact_at, write_all_at}`，删除 seek 与 `SeekFrom`；错误语义保持（`read_exact_at` 短读报 `UnexpectedEof`，与 `read_exact` 一致）。新增 `tests/file_storage_io_test.rs`（往返等价 + 越界报错 + 并发冷读正确性）。
- **Iteration 001（T02）**：`DataScanExecutor` 在页耗尽跳转时预取下一页：`JumpToPage` 分支只更新游标，随后在 `current_page_id` 换页时对"新当前页的 next_page_id 对应页"发起 `tokio::spawn` 预取。预取仅写入 BufferPool 缓存，结果与错误都被丢弃（真实读取仍走 `with_page_data` 的正常路径，错误在真实读取时显式报告）。miss 信号量预算被预取占用不构成饥饿：预取与普通 miss 共用 16 permits，同一时刻至多 1 个预取在途（单游标扫描），占用不超过 6.25%。
- **预取默认关闭（replan 2026-09-05，用户决策）**：默认路径（`new`）`prefetch_enabled = false`，预取经 `with_prefetch(true)` 显式启用。依据：默认路径实测回退 `data_scan/1000` +40~47%、`data_scan/10000` +17~18%（p<0.05，对照组无变化）——暖缓存环境下页加载延迟不存在，spawn 任务生命周期开销无处可隐藏；按 MS08 实测驱动纪律由测量结果决定默认值，预取能力完整保留供慢存储/冷读场景后续评估。
- 按 MS08 纪律采集对比证据：各 Iteration 实施前 `cargo bench --save-baseline` + strace syscall 计数（T01：micro/data_scan/buffer_pool 三套；T02：data_scan_bench 前后对比），量化结论写入 Act Response。

## Capabilities

### New Capabilities

- `storage-io-optimization`：页 I/O 位置参数化（pread/pwrite）+ 数据页链扫描预取。改前：每页 2 syscall + 并发冷读不同页存在错读窗口 + 扫描逐页串行取页。改后：每页 1 syscall + 并发读结构性正确 + 扫描时下一页加载与当前页行处理重叠。
  - 关联：MS08-T01/T02（`.claude/docs/tasks.md`）；Explorer 2026-09-05 MS08 调查（本会话即时回答）；m19-datascan-path.md（DataScan 链遍历结构）。

### Out of Scope（本 change 不做）

- **T03 脏页 writev**：依赖本 change T01 的 pwrite 底层，单独 change（还需先按 page_id 排序分连续段）。
- **T04/T05/T06**：RowLock DashMap、Varint Key、WAL fsync 合并——各自独立 change。
- **WalWriter seek 路径**：全部在 `Arc<Mutex<File>>` 临界区内（串行、append 语义），无竞态，不属本 change。
- **write_page 的 `page.data.clone()` 复制消除**：非 syscall 项，待 profiling 数据。
- **LRU/淘汰策略改造、BufferPool 缓存容量调整**：预取依赖现有 clock 淘汰处理容量压力。
- **`AsyncStorage` trait / 公开 API / 磁盘格式变化**：本 change 全部为零接口/零格式变更。

## Impact

- **影响模块**：`src/storage/file_storage.rs`（2 个私有函数）、`src/executor/data_scan.rs`（`next()` 换页分支 + 新增预取 helper）、`tests/file_storage_io_test.rs`（新增）、`tests/prefetch_test.rs`（新增，覆盖预取下的行序等价与LIMIT路径）。`BufferPool` 不改（`get_page` 幂等，预取复用其现有路径）。
- **影响接口**：无公开接口变化。预取通过现有 `BufferPool::get_page` 完成。
- **影响行为**：页 I/O syscall 序列（lseek+read → pread64；lseek+write → pwrite64）；扫描行序与查询结果零变化（预取只提前装载缓存）；错误语义：并发场景下预取遇到的 IO 错误被丢弃，真实读取时同页错误仍显式报告——行为与改造前一致（改造前该页在真实读取时才遇到错误）。
- **兼容性**：577 既有测试零修改全绿；`tests/concurrent_test.rs` 的 `CountingStorage` 替身只包装 trait 不受影响。
- **风险**：
  - T01 S4 竞态测试单次运行可能侥幸 GREEN（低）：已获用户决策接受——16 任务高频交错使失败概率趋近 1，若侥幸 GREEN 以 strace 结构证据收尾并记录，不阻塞。
  - T02 预取改动可能扰动 MVCC 快照语义（中）：预取只写缓存不改数据，`with_page_data` 读时页内容与无预取时一致；`pushdown_test.rs` 15 测试作等价守卫。
  - T02 bench 改善可能不显著（中）：容量足够时全部命中缓存，无 miss 可隐藏——MS08 纪律允许"明确记录未达预期"。
  - WSL2 bench 噪声（低）：同会话采集对比，标注环境。

## 关联

- 关联里程碑：**MS08**（性能压测 / T01+T02）。完成后 T03 获得依赖的 pwrite 底层。
- 实施顺序：Iteration 000（T01）先行，Iteration 001（T02）复用同一 bench 基线体系。
