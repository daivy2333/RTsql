# design: ms08-t01-t02-pread-prefetch

## 当前行为（Current-State Evidence，采集自 master @ `4d410ac`，静态代码事实）

### T01 页 I/O 路径

- `FileStorage`（`src/storage/file_storage.rs:12-17`）：`file: Arc<std::fs::File>`（无互斥）、`page_size`、`file_len: AtomicU64`、`free_pages: Mutex<Vec<u64>>`。是唯一生产 `AsyncStorage` 实现（`tests/concurrent_test.rs:201` 的 `CountingStorage` 是测试替身，只包装 trait）。
- `read_page_blocking`（`file_storage.rs:53-64`）：`seek(SeekFrom::Start(offset))` + `read_exact` → 每页 2 syscall（lseek + read）。
- `write_page_blocking`（`file_storage.rs:66-77`）：`seek` + `write_all` → 每页 2 syscall；调用方 `write_page`（`file_storage.rs:88-93`）先 `page.data.clone()` 复制 4KB（本次不动）。
- **共享偏移竞态**：`&File` 的 `Seek` 修改共享 file description 偏移。`BufferPool::get_page` miss 路径（`src/storage/buffer_pool.rs:74-126`）允许 16 个并发加载（`MISS_SEMAPHORE_PERMITS=16`，`buffer_pool.rs:16`），`loading_locks` 只串行化**同页**加载（`buffer_pool.rs:42/97-102`）。两个并发 `read_page`（不同页）交错 seek → 前者读到后者的页（错读）。所有 bench/测试走 `Database::open(文件路径)` 即文件后端，改动可被 bench 观测。
- WAL 对照（不在范围）：`WalWriter` 的 seek 全部在 `Arc<Mutex<File>>` 临界区内（`src/wal/writer.rs:49-66/115-149`），串行无竞态。

### T02 DataScan 页链推进

- `DataScanExecutor::next()`（`src/executor/data_scan.rs:162-324`）主循环：`with_page_data(page_id, closure)` 内解析当前页；页耗尽（`slot_index >= slot_count`，`data_scan.rs:205-212`）时读 `SlottedPageHeader.next_page_id`，返回 `PageAction::JumpToPage(next)`；外层 match 更新 `current_page_id = Some(PageId(next_id))`、`current_slot_index = 0`（`data_scan.rs:313-317`）后 continue 下一轮循环——下一轮 `with_page_data` 才触发 `get_page` miss 加载。**串行"用完再取"**，页间无重叠。
- 链尾哨兵：`next_page_id == 0` → `PageAction::Done`（`data_scan.rs:198/208`）。
- 行内谓词/LIMIT（MS07-T06）：`filter_row`/`yield_capped`（`data_scan.rs:100-120/281-284`），`tests/pushdown_test.rs` 15 测试是等价守卫。
- `BufferPool::get_page` 幂等：缓存命中直接返回，miss 走 load；并发重复加载同页被 loading_locks 串行化——预取与真实读取竞态安全。
- miss 信号量：16 permits，全 miss 路径共享（含驱逐）。

## 目标行为

### T01

- `read_page_blocking`：`file.read_exact_at(&mut buf, offset)`（`FileExt`，Linux 为 pread64）。
- `write_page_blocking`：`file.write_all_at(&*data, offset)`（pwrite64）。
- 删除 `SeekFrom`/`Seek` 导入（若 `WalWriter` 不在本文件则无残留）；错误类型不变（`Result<_, std::io::Error>` 经 `?` 转 `StorageError`）。
- 短读语义：`read_exact_at` 越界短读报 `UnexpectedEof`，与 `read_exact` 一致——S3 测试锁定。
- 并发正确性：位置参数不读写共享偏移，16 并发冷读不同页无错读——S4 测试锁定（概率性 RED，用户已接受）。

### T02

- `JumpToPage` 分支：更新游标后，对**新当前页的下一页**（即再下一页）发起预取；或在首次进入某页时预取其 next。实现口径：进入新页（`current_page_id` 变化）后从该页 header 读 `next_page_id`，若非 0 则 `tokio::spawn` 一个丢弃结果的 `buffer_pool.get_page(PageId(next))`。
- 预取在途约束：扫描器持有 `Option<JoinHandle>` 或等价状态；发起前 join/丢弃旧 handle，保证 ≤1 在途。
- 错误丢弃：spawn 任务内 `let _ = ...get_page(...).await;`，不 panic（get_page 返回 Result，无 unwrap）。
- `snapshot`/可见性/谓词/LIMIT 逻辑零改动：预取不触碰行处理路径。
- **默认关闭（2026-09-05 replan 修订，NEW-EVIDENCE）**：实测默认路径预取净回退（data_scan/1000 +40~47%、/10000 +17~18%，均 p<0.05；同套件未改动路径 scan_via_index 对照组无变化；机制归因：默认池容量 100 页下 1000 行集全暖缓存零 miss 可隐藏，spawn/wake 为纯开销；10000 行集部分冷路径收益仅部分抵消）。修订：`new` 默认 `prefetch_enabled=false`，`with_prefetch(true)` 显式启用；预取能力保留（代码/测试/开关完整），供慢存储或受限容量场景后续评估。触发点实现口径：closure 顶部捕获后继页 id，await 返回后触发（重叠窗口 = 当前页剩余行处理时间）。

## 关键技术选择与理由

1. **`FileExt::{read_exact_at, write_all_at}` 而非 libc/pread crate**：std 原生、无新依赖（符合"依次优先平台原生能力"约束）；语义恰好是"位置参数 + exact"。
2. **预取用 `tokio::spawn` 丢弃结果而非双向缓冲数据结构**：预取目标是 BufferPool 缓存（已有正确性保证），不需要第二个用户态缓冲；真实读取路径零变化，行为等价性可由 pushdown/prefetch 测试守卫。"双缓冲"语义由"当前页处理 ∥ 下一页装载"实现。替代方案（独立预取队列 + wakeup）复杂度高且改动 BufferPool 接口——拒绝。
3. **预取深度 1**：单游标扫描，深度 1 已覆盖"当前页处理时间"窗口；深度 >1 需要预取队列与 miss 预算再平衡（Explorer 调查明确的风险点），不做。
4. **Iteration 划分 000（T01）→ 001（T02）**：T02 复用 T01 稳定的页 I/O 底层与 bench 基线体系；各自独立验收（平衡审计：T01 是"底层正确性+syscall 证据"，T02 是"行为等价+重叠收益"，不同验证域，不合并）。

## 责任边界

- **修改**：`src/storage/file_storage.rs`（仅 2 个私有 blocking 函数 + 导入）、`src/executor/data_scan.rs`（`next()` 换页分支 + 预取状态字段 + helper）、新增 2 个测试文件。
- **保持**：`AsyncStorage` trait、`FileStorage` 公开方法、`BufferPool` 全部行为、磁盘格式、所有执行器语义、WAL 子系统。
- **禁止**：改 `write_page` 的 clone 路径、改 BufferPool/淘汰策略、引入新依赖、改 `WalWriter`、动 `tests/` 既有文件（只新增）。

## 实现顺序

Iteration 000：baseline（T1）→ RED 测试（T2.1-2.2）→ 改造（T2.3）→ GREEN + 回归 + after 证据（T2.4-3.4）。
Iteration 001：before 基线 → 预取测试（T1）→ 实现（T2）→ 回归 + after（T3）。

## 并发、安全、性能、多平台风险

- 并发：T01 消除读竞态；T02 预取与真实读取并发走 get_page（loading_locks 保证单次加载），无新竞态。
- 平台：`FileExt` 是 Unix trait，项目当前仅支持 Linux x86_64（SNAPSHOT），无 Windows 兼容要求。
- 性能：改善可能不显著（页缓存命中时 read 极快）——MS08 验证边界允许"明确记录未达预期"，syscall 计数是主要量化证据。
- 数据安全：零格式变更；S4 守卫防回归。
