# Iteration 000 / Cycle 000: 测量连续脏页并裁定

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Inherited scope: `proposal.md` 的场景 1 和场景 2，`specs/storage-io-optimization/spec.md` 的「连续脏页写入的测量」与「批量写去留由这五次打印裁定」，`design.md` 的 D1 与 D2
- Excluded scope: T3、T4、T5。不在非测试代码里调用 `pwritev`。不调用 `io_uring_setup`。不改 `flush_all` 的逐页循环，不改 `flush_pages`、`evict_one`、检查点步骤。不跑全量。

**Objective**

测试打印连续 64 个脏页的五次逐页刷盘和五次测试内 `pwritev` 的提交次数、写次数和耗时，并打印一次 `FileStorage::sync`。执行记录根据这五行写下「值得交付」或「保留逐页写」。非测试构建的页写入行为与测量前相同。

**Investigation Facts**

- Starting point: 基点 `2a78e01b44b7275add1f1fe9d539a887e889e4ad`（当时 `master`，工作区干净，相对 `origin/master` ahead 26，`openspec list` 无活跃变更）。本分支为 `changes/2026-11-24-ms54-dirty-page-batch-io`。范围提交是 `91f96362ac25ba9e109ec35e8a6ff3dd1033dad9`。SNAPSHOT 仍写 `04988dc` / ahead 25，本次不改它。没有可沿用的 Explorer 分析；下面的调用链是这次读代码的结果。变更前没有跑过这条测量测试，因为它还不存在。规划时 `cat /proc/self/io` 报文件不存在。`/proc/sys/kernel/io_uring_disabled` 为 0，`io_uring_setup(8)` 返回了可用描述符，内核 `6.18.40.1-microsoft-standard-WSL2`。这不构成 io_uring 的采纳结论，本 Cycle 也不调用它。
- Current-State Evidence: `FileStorage::write_page`（`src/storage/file_storage.rs` 536 行起）在只读拒绝之后消费 `IoFailPoint::DataWrite`。命中则 `mark_write_failed` 并返回含 `injected io failure (write_page)` 的错误，不进入 `spawn_blocking`。未命中则恰好一次 `spawn_blocking`，里面是 `write_page_blocking`（485 行起）。成功 `mark_write_succeeded`，失败 `mark_write_failed`。偏移是 `HEADER_SIZE + page_id.to_offset(record_size)`。`HEADER_SIZE` 为 64，明文 `write_all_at` 4096 字节，加密记录 4124 字节。`sync`（660 行起）另一次 `spawn_blocking` 里 `sync_all`。`BufferPool::flush_all`（`src/storage/buffer_pool.rs` 259 行起）按 DashMap 顺序收集，逐页 `await write_page`。`claim_dirty_for_write`（51 行起）复制页、置 `write_in_flight`、清脏位。失败时 `restore_dirty_from` 从失败下标交还脏位。`flush_pages`（314 行起）按给定顺序逐页写。`flush_all_except`（338 行起）跳过排除集后与 `flush_all` 相同。`evict_one`（222 行起）一次 `write_page`。`CheckpointManager::checkpoint`（`src/wal/checkpoint.rs` 207 行起）取样释放编号、`flush_all`、成功则提升、失败则丢弃，然后 WAL `fsync`。这里没有 `FileStorage::sync`。`Catalog::flush_commit_point`（`src/storage/catalog.rs` 282 行起）先 `flush_all_except` 再 `flush_pages`。`Database::open` 的池容量是 100（`src/database.rs` 359 行）。新文件 `allocate_page` 从 0 递增；明文分配是 `set_len`，不经过 `write_page_blocking`。未伸出的偏移上 `read_page` 会失败。
- Code and Critical Path: 测量入口是新的库测试，调用公开的 `FileStorage::open`、`AsyncStorage::allocate_page`、`BufferPool::new`、`get_page`、`PageGuard::modify_page`、`flush_all`、`read_page`、`sync`。`modify_page` 会把帧标脏（`src/storage/page_frame.rs` 92 行起）。容量 128、只装入 64 页时，`get_page` 的容量检查不会调用 `evict_one`。测试内的 `pwritev` 打开同一路径上的另一个文件描述符，不取 advisory 锁；`FileStorage` 已持有的锁不阻止这次写入。产品写路径在本 Cycle 仍是：`flush_all` → `write_page` → `spawn_blocking` → `write_all_at`。

**Task Definitions**

### T1: 打印连续脏页的五次写次数和耗时

- Requirement/Scenario: 「连续脏页写入的测量」，含五次打印场景和测量无效场景。
- Depends on: None
- Targets: `src/storage/file_storage.rs` 的测试构建计数，以及同文件 `tests` 模块里的 `contiguous_dirty_page_write_measure`。
- Current behavior: 没有这些计数，也没有这个测试。`flush_all` 对 64 个脏页会调用 64 次 `write_page`。
- Required behavior: 测试按 Verification 的命令退出码为 0，并打印五行 `repeat=` 和一行 `sync_calls=1`。每行 `baseline_submissions=64`、`baseline_writes=64`、`merged_submissions=1`，两侧纳秒大于 0，`merged_writes` 至少为 1。页 0 到 63 与当次字节一致，页 64 全 0，页 70 全是 `0x5A`。输出里没有「值得交付」或「保留逐页写」。非测试构建不包含计数语句。
- Preserve: `write_page`、`write_page_blocking`、`sync` 在非测试构建中的分支、错误文本和写出的字节。`flush_all` 的逐页循环。
- Forbidden: 在非测试代码里调用 `pwritev` 或 `io_uring_setup`。读取 `/proc/self/io`。修改 `buffer_pool.rs`、`checkpoint.rs`、`Cargo.toml`。用 SQL 语句构造这 64 页。
- First test: 先写测试，调用尚不存在的 `reset_page_io_counts`、`page_submit_count`、`page_write_count`、`sync_count`。`cargo test --offline --release --lib storage::file_storage::tests::contiguous_dirty_page_write_measure -- --test-threads=1 --nocapture` 在计数落地前编译失败。这是 RED。
- GREEN condition: 计数只在 `#[cfg(test)]` 下加一之后，同一命令退出码 0，并出现五行走完字段的打印。
- Verification: 只跑这一条命令一次。退出码非 0，或少一行，或读回断言失败，都是测量无效，不改负载再跑。
- Stop when: 不改非测试控制流就无法计数；`allocate_page` 之后 `get_page` 仍短读；不计打印的那次 `flush_all` 写次数不是 64；或必须调用 `io_uring_setup` 才能完成打印。停止并回到计划。

测试按这个顺序做，断言失败即测量无效：

1. `FileStorage::open` 明文路径。`Arc<dyn AsyncStorage>` 上 `allocate_page` 71 次，得到页 0 到 70。
2. `write_page` 把页 70 写成每个字节 `0x5A`。
3. `BufferPool::new(128, storage)`。只 `get_page` 页 0 到 63。
4. 计数函数：`reset_page_io_counts` 把三个计数清零；`page_submit_count` 是 `write_page` 即将 `spawn_blocking` 的次数，注入失败提前返回时不加；`page_write_count` 是 `write_page_blocking` 里 `write_all_at` 成功返回的次数，明文和加密两个分支都加，失败不加；`sync_count` 是 `sync_all` 成功返回的次数。测试模块通过父模块可见性调用它们。
5. 不计打印的一次：把页 0 到 63 改脏，`reset_page_io_counts`，`flush_all`，`page_write_count` 必须为 64。然后 `reset_page_io_counts`。
6. 重复五次，`repeat` 从 0 到 4。逐页填充：`data[0]` 为页号的低 8 位，其余字节为 `repeat + 1`。放下 `PageGuard` 后再计时。计时只包住 `flush_all`。读出 `baseline_submissions`、`baseline_writes`、`baseline_ns`。`read_page` 核对页 0 到 63。合并填充：`data[0]` 仍是页号，其余字节为 `repeat + 101`。计时从复制这 64 页开始，到一次 `spawn_blocking` 里的 `pwritev` 全部写完结束。偏移 64，总长 64 × 4096。短写则前进偏移继续 `pwritev`；返回负值，或未写完返回 0，测试失败。`merged_writes` 是 `pwritev` 的调用次数，`merged_submissions` 为 1。这段期间产品的提交次数和写次数必须仍是 0。核对页 0 到 63，并核对页 64 全 0、页 70 全是 `0x5A`。打印：`repeat={repeat} baseline_submissions={bs} baseline_writes={bw} baseline_ns={bn} merged_submissions={ms} merged_writes={mw} merged_ns={mn}`。
7. 五次之后 `reset_page_io_counts`，计时一次 `sync`，打印 `sync_calls={sync_count} sync_ns={sn}`。`sync_calls` 为 1，`sync_ns` 大于 0。
8. 断言每行 `baseline_submissions` 与 `baseline_writes` 为 64，两侧纳秒大于 0，`merged_submissions` 为 1，`merged_writes` 大于或等于 1。不要断言 `merged_writes < 64`，也不要打印裁定词。

### T2: 用五行裁定批量写去留

- Requirement/Scenario: 「批量写去留由这五次打印裁定」的两个场景。
- Depends on: T1
- Targets: 执行记录。不改文件，除非 T1 的测试源码在 T2 之前还没有提交；本任务本身不改 `src/`。
- Current behavior: 还没有裁定。
- Required behavior: 从 T1 的输出取出五行走完的数字。五个 `baseline_ns` 升序后的第三个，和五个 `merged_ns` 升序后的第三个，写入执行记录。五次 `merged_writes` 都小于 64，且合并写中位数小于逐页中位数时，记录「值得交付」。否则记录「保留逐页写」。同时抄上五行和 `sync_calls` 行。`sync_ns` 不参加比较。
- Preserve: T1 的测试源码和计数位置。非测试写入路径。
- Forbidden: 重跑 T1。把阈值改成「5–10 倍」或任何打印里没有的倍数。调用 `io_uring_setup`。因为中位数不更短就去改 `flush_all`。
- First test: T1 已经通过的 `contiguous_dirty_page_write_measure`。本任务不新写测试，也不再运行它。
- GREEN condition: 执行记录里有五行原文、两个中位数，以及「值得交付」或「保留逐页写」二者之一。
- Verification: 对照 T1 命令的已有输出做算术。输出缺少任一字段时，不补跑，按 T1 的停止条件回到计划。
- Stop when: T1 的进程不是退出码 0；或五行无法解析；或需要重跑才能得到严格小于。停止，不记成保留，也不记成值得交付。

**Invariants**

- 非测试构建的 `write_page` 仍是一页一次 `spawn_blocking` 和一次 `write_all_at`。注入失败仍在落盘前返回 `injected io failure (write_page)`。
- `flush_all`、`flush_pages`、`evict_one` 的控制流在本 Cycle 不改。检查点仍不调用 `FileStorage::sync`。
- M18 的四条不在本 Cycle 改写：驱逐不按树序；`redo_count > 0` 不消费磁盘索引；`redo_count == 0` 信任磁盘树；数据页按 WAL 位置重放。
- 测试命令带 `--test-threads=1`。不并行第二条 cargo。

**Acceptance**

- T1 的命令退出码 0，打印满足规格里的字段和读回条件，且没有裁定词。
- T2 的执行记录用中位数规则写下「值得交付」或「保留逐页写」，并抄有五行。
- `git diff` 相对范围提交，产品写入行为的差异只有 `src/storage/file_storage.rs` 里 `#[cfg(test)]` 计数和这个测试。`Cargo.toml` 无差异。

**Verification**

场景 1 的判定就是 T1 那一条命令的退出码、打印和测试内断言。场景 2 的判定是 T2 对已有打印做的中位数比较，不另起进程。本 Cycle 没有跨过 SQL 或检查点入口的场景。不使用哈希、运行编号或额外判定程序。

**Gate 2 Readiness**

PASS。审查计划 11 项通过；RTM 无 Missing、无 Simplified；接口、错误文本和采纳条件已写明；Persisted Evidence 为 none。无 BLOCKED，无 SKIPPED。

**Risks and Notes**

- 耗时中位数来自固定的五次，不因为抖动追加运行。
- 计数是进程内静态值。这条命令只过滤一个测试。
- `pwritev` 的短写次数现在不知道。测试打印实际调用次数，T2 用打印值比较，不把次数改成假定的 1。
- 规划时 io_uring 能建环。本 Cycle 不因此比较它。
- 后续任务 T3、T4、T5 的步骤在 `tasks.md`。本 Cycle 不执行它们。001 只在本 Cycle 的执行记录已经是「值得交付」或「保留逐页写」之后才能展开。

## Act Response

- Status: reported

### 实际改动

- `src/storage/file_storage.rs`
  - `write_page`：注入失败返回之后、`spawn_blocking` 之前，`#[cfg(test)]` 调用 `record_page_submit`。加密与非加密两个分支各一处。
  - `write_page_blocking`：明文和加密两条 `write_all_at` 成功返回后，`#[cfg(test)]` 调用 `record_page_write`。失败走 `?`，不加。
  - `sync`：`sync_all` 成功返回后，`#[cfg(test)]` 调用 `record_page_sync`。
  - 测试构建的计数：`reset_page_io_counts`、`page_submit_count`、`page_write_count`、`sync_count`，以及只供上述加一使用的 `record_page_submit`、`record_page_write`、`record_page_sync`。
  - 测试 `storage::file_storage::tests::contiguous_dirty_page_write_measure`：通过 `super::` 调用上述四个计数函数。
- `openspec/changes/2026-11-24-ms54-dirty-page-batch-io/tasks.md`：T1、T2 状态改为 `reported`。001 未展开。
- 未改 `flush_all`、`buffer_pool.rs`、`checkpoint.rs`、`Cargo.toml`。非测试代码没有 `pwritev` 或 `io_uring_setup`。

### 与计划的偏差

- 合并段计时前多调用一次 `reset_page_io_counts`。逐页刷盘的提交次数和写次数已经读入局部变量。清零后，这段从复制 64 页到 `pwritev` 结束，产品提交次数和写次数保持为 0。
- `pwritev` 若报告的字节数大于剩余缓冲，测试返回错误。计划要求负值和未写完的 0 失败。这次五次都是一次写完，该分支没有执行。

### 验证

先行测试 RED：同一条命令在计数落地前退出码 101。`reset_page_io_counts`、`page_submit_count`、`page_write_count`、`sync_count` 均报 `E0425`。

验证命令只跑了一次，T2 没有重跑：

```
cargo test --offline --release --lib storage::file_storage::tests::contiguous_dirty_page_write_measure -- --test-threads=1 --nocapture
```

退出码 0。`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 460 filtered out`。输出没有「值得交付」或「保留逐页写」。`rtsql` 没有 rustc 警告。

```
repeat=0 baseline_submissions=64 baseline_writes=64 baseline_ns=2584264 merged_submissions=1 merged_writes=1 merged_ns=280418
repeat=1 baseline_submissions=64 baseline_writes=64 baseline_ns=2500566 merged_submissions=1 merged_writes=1 merged_ns=419135
repeat=2 baseline_submissions=64 baseline_writes=64 baseline_ns=2168840 merged_submissions=1 merged_writes=1 merged_ns=108542
repeat=3 baseline_submissions=64 baseline_writes=64 baseline_ns=2952992 merged_submissions=1 merged_writes=1 merged_ns=110777
repeat=4 baseline_submissions=64 baseline_writes=64 baseline_ns=2375862 merged_submissions=1 merged_writes=1 merged_ns=119665
sync_calls=1 sync_ns=2139393
```

### T2 裁定

五个 `baseline_ns` 升序：2168840、2375862、2500566、2584264、2952992。第三个是 2500566。

五个 `merged_ns` 升序：108542、110777、119665、280418、419135。第三个是 119665。

五次 `merged_writes` 都是 1，都小于 64。合并写中位数 119665 小于逐页中位数 2500566。`sync_ns` 不参加比较。

值得交付

### Gate

- Gate 3：T1、T2 通过。打印字段、读回断言、计数位置和禁止项与 diff 一致。无 Critical 或 Important。
- Gate 4：T1 的验证命令退出码 0，五行和 `sync_calls=1` 已打印。T2 只对这次输出做了中位数比较。

### Experience Candidates

None

### 未解决问题

无

## Plan Review

- Review Result: pending
