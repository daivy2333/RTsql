# Iteration 001 / Cycle 000: 交付连续段合并写

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Inherited scope: `proposal.md` 场景 3 到场景 8，`specs/storage-io-optimization/spec.md` 里裁定为值得交付之后生效的「页 I/O 使用位置参数调用」「零接口零格式变更」「值得交付时只合并连续脏页段」「失败重试、释放耐久性与恢复约定保持」，`design.md` 的 D3 与 D4。Iteration 000 的执行记录已经是「值得交付」。
- Excluded scope: T3 的「保留逐页写」一节。T4、T5。不调用 `io_uring_setup`。不把 `io-uring` 写进 default features。不改 `flush_pages`、`evict_one`、`allocate_page`。不改 `src/wal/checkpoint.rs`、`src/storage/catalog.rs`、`src/pipeline.rs`、`src/storage/data/table_manager.rs`。不给数据文件加 `sync_all`。不改 M18 的四条正文。不重跑 `contiguous_dirty_page_write_measure`。不跑收尾全量。

**Objective**

`flush_all` 与 `flush_all_except` 在取走脏页后按页号升序切连续段。长度大于或等于 2 的段一次 `pwritev`，长度为 1 的段仍是 `write_page`。失败时当前段和尚未尝试的页重新为脏，更早已经成功的段保持清洁，并且不因这次错误进入失败集合。测量测试改为 `#[ignore]`。

**Investigation Facts**

- Starting point: Iteration 000 的 Act Response（`iterations/000-measure-and-decide/000-initial.md`）已报告，本 Cycle 的 Plan Review 接受该结果。代码修订是 `e67a86cc3776c6bd559689adcd1a7ce8ac65978e`：`src/storage/file_storage.rs` 只增加了 `#[cfg(test)]` 计数和 `contiguous_dirty_page_write_measure`。五个 `baseline_ns` 升序后的第三个是 2500566，五个 `merged_ns` 升序后的第三个是 119665，五次 `merged_writes` 都是 1。裁定是值得交付。`flush_all` 仍对每一脏页调用一次 `write_page`。验证沿用该 Act Response 的那一条 release 命令，退出码 0；本 Cycle 不重跑它。
- Current-State Evidence: `AsyncStorage`（`src/storage/async_storage.rs`）用 `#[async_trait]`，已有带默认实现的方法。`async-trait` 版本是 `0.1`。实现者有 `FileStorage`、`MemoryStorage`、`buffer_pool.rs` 测试里的 `GatedWriteStorage`，以及 `page_format/overflow.rs` 测试里的 `FailAfterNAllocs`。`BufferPool` 持有 `Arc<dyn AsyncStorage>`。`flush_all`（`buffer_pool.rs` 259 行起）按 DashMap 顺序收集，`claim_dirty_for_write`（51 行起）在帧锁内复制页、置 `write_in_flight`、清脏位。随后逐页 `await write_page`。失败时 `restore_dirty_from`（294 行起）从该下标到末尾交还脏位并清除 `write_in_flight`；成功则 `finish_write_in_flight(id, true)`（300 行起）只清除在途标记。`flush_all_except`（338 行起）在认领前跳过排除集，写失败的交还方式与 `flush_all` 相同。`flush_pages`（314 行起）按调用方顺序逐页 `write_page`。`evict_one`（169 行起）一次 `write_page`。`Catalog::flush_commit_point`（`catalog.rs` 282 行起）先 `flush_all_except` 再 `flush_pages`。`CheckpointManager::checkpoint`（`checkpoint.rs` 207 行起）在 `flush_all` 前取样 `release_completion_limit`，成功则 `promote_free_releases`，失败则 `discard_free_releases`，下一步是 WAL `fsync`。这里没有 `FileStorage::sync`。`promote_free_releases`（`file_storage.rs` 728 行起）在任一锚点仍处于 `failed_pages` 时留下该项。`write_page`（540 行起）先拒绝只读，再消费 `IoFailPoint::DataWrite`；命中则 `mark_write_failed` 并返回含 `injected io failure (write_page)` 的 `Io`，不进入 `spawn_blocking`。只读拒绝文本是 `cannot write a page on a read-only storage instance`。`write_page_blocking` 的偏移是 `HEADER_SIZE as u64 + page_id.to_offset(record_size)`。`HEADER_SIZE` 为 64，`Page::PAGE_SIZE` 为 4096，`ENCRYPTED_PAGE_RECORD_SIZE` 为 4124。`PageId::to_offset` 是页号乘 `record_size`。`PageId` 实现 `Ord`。`page_write_count` 只在 `write_all_at` 成功返回后加一。`libc = "0.2"` 只列在 `Cargo.toml` 的 `[dev-dependencies]`，产品代码现在不能引用它。默认 features 含 `crypto`。`FileStorage::open` 是无密钥明文；`open_with_key(path, Some("pw"))` 创建加密库。`failed_flush_then_evict_keeps_the_new_bytes` 只有一个脏页，写路径仍会走到 `write_page`。
- Code and Critical Path: 本 Cycle 的产品入口是 `BufferPool::flush_all` 与 `BufferPool::flush_all_except`。它们在认领之后按 `PageId` 升序切段，长度大于或等于 2 时调用 `AsyncStorage::write_page_run`，长度为 1 时调用 `write_page`。`FileStorage::write_page_run` 覆盖默认实现，在一次 `spawn_blocking` 里 `pwritev`。`MemoryStorage` 和未覆盖该方法的测试存储走默认实现，按页 `write_page`。`flush_pages` 与 `evict_one` 不进入这条路径。

**Task Definitions**

### T3: 交付连续段合并写

- Requirement/Scenario: 「值得交付时只合并连续脏页段」的全部场景，以及「失败重试、释放耐久性与恢复约定保持」。
- Depends on: Iteration 000 的执行记录是值得交付。
- Targets: `src/storage/async_storage.rs` 的 `write_page_run` 默认实现；`src/storage/file_storage.rs` 的 `FileStorage::write_page_run`；`src/storage/buffer_pool.rs` 的 `flush_all` 与 `flush_all_except`。测试加在这两个文件已有的 `tests` 模块。`Cargo.toml` 只把 `libc = "0.2"` 从 `[dev-dependencies]` 移到 `[dependencies]`。
- Current behavior: `flush_all` 按 DashMap 顺序对每一脏页调用一次 `write_page`。连续 64 页的 `page_write_count` 为 64。没有 `write_page_run`。产品代码不能调用 `libc::pwritev`。
- Required behavior: 完成后的行为如下。落地顺序以 Verification 为准：加密记录的 `pwritev` 晚于加密测试的第一次失败。收集并认领之后按 `PageId` 升序排列，再把相邻页号差为 1 的最大段分开。长度大于或等于 2 的段调用 `write_page_run`，长度为 1 的段调用 `write_page`。`flush_all_except` 使用同一套切段；排除集里的页不认领、不写入。`write_page_run` 的默认实现按页 `await write_page`，中途失败把该错误返回，不继续后面的页。`FileStorage::write_page_run` 接收已经按页号升序排好的 `Vec<Page>`。只读时返回与 `write_page` 相同的拒绝文本。页数小于 2，或相邻页号差不是 1 时，任何字节落盘前返回 `StorageError::Internal`，文本含 `non-contiguous page run`，不消费 `DataWrite`，不调用 `mark_write_failed`。连续时，`DataWrite` 在 `spawn_blocking` 之前消费一次；命中则为这一段每一页 `mark_write_failed`，返回含 `injected io failure (write_page)` 的 `Io`，不调用 `pwritev`。未命中时一次 `spawn_blocking` 写出整段。偏移是第一页的 `HEADER_SIZE + page_id.to_offset(record_size)`。明文每页 4096 字节，加密每页先 `encrypt_page` 再写 4124 字节，顺序与页号升序相同。写出用 `pwritev`。短写则前进偏移继续 `pwritev`。返回负值、未写完就返回 0，或报告的字节数大于剩余缓冲时，这一段每一页 `mark_write_failed` 并返回 `Io`。成功则每一页 `mark_write_succeeded`。`pwritev` 不调用 `write_all_at`，因此不增加 `page_write_count`。调用方收到 `Err` 时，从当前段的第一个下标到已排序向量末尾交还脏位并清除 `write_in_flight`。更早已经成功返回的段保持 `finish_write_in_flight(id, true)` 的结果，不因这次错误进入 `failed_pages`。默认实现里前缀页可能已经 `mark_write_succeeded`，帧脏位仍整段交还。测量测试加上 `#[ignore = "五次打印已记在 Iteration 000 的执行记录"]`。
- Preserve: `flush_pages` 的顺序和逐页 `write_page`。`evict_one` 的选页和单页 `write_page`。`allocate_page`。检查点对 `FileStorage::sync` 的不调用，以及取样、成功提升、失败丢弃的顺序。`injected io failure (write_page)`。M18 的四条。`page_write_count` 只统计成功的 `write_all_at`。
- Forbidden: 修改 `src/wal/checkpoint.rs` 的步骤顺序。修改 `evict_one` 的选页。把提交页并进 `flush_all_except` 的同一次 `pwritev`。给数据文件加 `sync_all`。调用 `io_uring_setup`。修改 default features。把 `flush_pages` 改成 `write_page_run`。在 `BufferPool` 里把存储向下转型成 `FileStorage`。把 `write_page_run` 改成没有默认实现、必须由每个存储手写的方法。重跑测量测试。新增产品侧的脏位查询方法。
- First test: 改写入前先跑 `cargo test --offline --lib storage::file_storage::tests -- --test-threads=1 --skip contiguous_dirty_page_write_measure`，退出码 0。再跑 `cargo test --offline --lib storage::buffer_pool::tests::failed_flush_then_evict_keeps_the_new_bytes -- --test-threads=1`，退出码 0。这是改前的 GREEN。然后只添加 `contiguous_flush_coalesces_a_run_and_keeps_the_gap`，运行 `cargo test --offline --release --lib storage::file_storage::tests::contiguous_flush_coalesces_a_run_and_keeps_the_gap -- --test-threads=1`。实现前产品写次数不小于脏页数，测试失败。这是 RED。
- GREEN condition: Verification 里的每条命令按顺序退出码 0。加密测试在加密记录的 `pwritev` 落地前必须先失败一次。
- Verification: 一次一个 cargo 进程，每条都带 `--test-threads=1`。
  1. 先完成 First test 的两条改前 GREEN，再添加合并测试并观察到 RED。
  2. 实现明文 `pwritev`，并把 `libc` 移入 `[dependencies]`。加密分支在这一步对这一段的每一页调用 `write_page`，成功的 `write_all_at` 继续计入 `page_write_count`。同一条合并测试命令再次运行，退出码 0。同一次修改给测量测试加上上面的 `#[ignore]`。
  3. 明文路径通过后，把失败集合断言加进同一个合并测试再运行。更早一段的页在 `failed_pages` 里，或当前段的页不在里面，命令失败。补齐当前段的 `mark_write_failed` 和更早一段的 `mark_write_succeeded` 后再运行到退出码 0。断言已经通过时，保留这次通过，不为了制造失败而拆掉标记。
  4. 添加 `contiguous_flush_encrypted_pages_roundtrip`，运行 `cargo test --offline --release --lib storage::file_storage::tests::contiguous_flush_encrypted_pages_roundtrip -- --test-threads=1`。此时写次数仍不小于 2，测试失败。
  5. 补上加密记录的 `pwritev`。同一命令退出码 0。
  6. 添加 `failed_run_restores_every_page_in_the_run`，运行 `cargo test --offline --lib storage::buffer_pool::tests::failed_run_restores_every_page_in_the_run -- --test-threads=1`。整段交还或未尝试页的交还尚未接上时测试失败，接上后再跑到退出码 0。已经接上时，这一次通过就是 GREEN。
  7. 顺序跑完这些既有命令，每条退出码 0：`cargo test --offline --lib storage::buffer_pool::tests::failed_flush_then_evict_keeps_the_new_bytes -- --test-threads=1`；`cargo test --offline --test write_failure_state_test -- --test-threads=1`；`cargo test --offline --test release_durability_precondition_test -- --test-threads=1`；`cargo test --offline --test free_page_lifecycle_test -- --test-threads=1`；`cargo test --offline --lib wal::checkpoint::tests -- --test-threads=1`；`cargo test --offline --test checkpoint_test -- --test-threads=1`；`cargo test --offline --test ddl_layout_publication_test -- --test-threads=1`。
- Stop when: 不把 `flush_pages` 并进同一次 `pwritev` 就不能保持目录提交点；既有测试要求 DashMap 的刷盘顺序；必须修改 `evict_one` 的选页、检查点步骤或 M18 的门控；或 `async_trait` 的默认 `write_page_run` 不能通过 `Arc<dyn AsyncStorage>` 调用。停止并回到计划。不要改成强制每个存储手写该方法，也不要把存储向下转型成 `FileStorage`。

`contiguous_flush_coalesces_a_run_and_keeps_the_gap` 放在 `file_storage.rs` 的测试模块。它持有 `Arc<FileStorage>`，池使用克隆出的 `Arc<dyn AsyncStorage>`。产品写次数是测前 `reset_page_io_counts`、测后 `page_write_count`。断言：

1. 页 3 和页 4 先用 `write_page` 写成可识别原内容，放入容量足够的池后不改脏。页 0、1、2 和页 5 为脏。`flush_all` 后写次数小于 4。页 3 和页 4 的原字节还在。页 0、1、2、5 的新字节可读回。
2. 页 70 用 `write_page` 写成每个字节 `0x5A`，不放入脏集合。刷盘后 `read_page` 仍是 `0x5A`。
3. 容量 128 的池里把页 0 到 63 改脏。`flush_all` 的写次数小于 64，64 页读回一致。
4. 另一次：页 5 先写成与新内容不同的旧字节，再和页 0、1、2 一起改脏。`flush_all_except` 排除页 5。写次数小于 3。页 0、1、2 的新字节可读回。页 5 的磁盘字节仍是旧内容。
5. 失败集合使用另一个新的明文文件，先 `allocate_page` 覆盖页 0 到 4。`write_page_run` 成功写出页 0 和页 1 之后，测试模块执行 `*storage.io_failpoint.lock().unwrap() = Some(IoFailPoint::DataWrite)`，再对页 3 和页 4 调用 `write_page_run`。错误文本含 `injected io failure (write_page)`。`failed_pages` 含 3 和 4，不含 0 和 1。页 0 和页 1 可读回刚写出的字节。页 3 和页 4 仍是调用前的字节。不新增公开的失败集合查询方法。

`contiguous_flush_encrypted_pages_roundtrip` 放在同一测试模块，并带 `#[cfg(feature = "crypto")]`。`open_with_key(..., Some("pw"))`，页 0 和页 1 改脏后 `flush_all`，写次数小于 2，同一密钥读回的明文一致。

`failed_run_restores_every_page_in_the_run` 放在 `buffer_pool.rs` 的测试模块。仿照 `GatedWriteStorage` 写测试存储，直接读 `pages` 里的 `PageFrame`。不要新增产品侧的脏位查询方法。两个案例都在这个测试里：

1. 不覆盖 `write_page_run`。`write_page` 第一次返回成功，第二次返回 `Io` 且不更新该页。池里只有页 0、1、2 为脏。`flush_all` 返回错误后，这三页的 `dirty` 都为真，`write_in_flight` 都为假。
2. 覆盖 `write_page_run`：第一次调用返回成功并记下这些页，第二次返回 `Io` 且不记下。池里页 0 和页 1 为一段，页 3 和页 4 为下一段，页 6 单独为脏。`flush_all` 返回错误后，页 0 和页 1 的 `dirty` 为假、`write_in_flight` 为假；页 3、4、6 的 `dirty` 为真、`write_in_flight` 为假。页 6 的 `write_page` 没有被调用。

**Invariants**

- M18 的四条保持原句：驱逐不按树序；`redo_count > 0` 时恢复不消费磁盘索引，并从数据页重建主键索引；`redo_count == 0` 时磁盘树可信；数据页仍按 WAL 记录中的位置重放。不修改 `evict_one` 的选页，不修改 `rebuild_pk_indexes` 的门控。
- `flush_pages` 不排序，不调用 `write_page_run`。提交页仍由 `flush_commit_point` 在 `flush_all_except` 返回之后按给定顺序逐页写入。
- 检查点仍不调用 `FileStorage::sync`，也不对数据文件 `sync_all`。
- 单页 `write_page` 仍是一次 `spawn_blocking` 和一次 `write_all_at`。读路径仍是 `read_exact_at`。这些路径不调用 `lseek`。
- 测试命令带 `--test-threads=1`。不并行第二条 cargo。

**Acceptance**

- 页 0 到 63 一次 `flush_all` 的 `page_write_count` 小于 64，读回与刷出前一致。空洞页原字节不变。排除集里的页不被同一次 `pwritev` 写到。
- 加密库页 0 和页 1 的 `flush_all` 写次数小于 2，同一密钥读回明文一致。
- `DataWrite` 在一段合并写落盘前消费一次，错误文本含 `injected io failure (write_page)`。失败段的每一页进入 `failed_pages`，更早已经成功的页不进入。
- 一次 `flush_all` 中当前段失败时，该段每一页和尚未尝试的页为脏，`write_in_flight` 为假。更早已经成功的段保持不脏。默认实现在段内第二页 `write_page` 失败时，整段三页都重新为脏。
- `failed_flush_then_evict_keeps_the_new_bytes`、`write_failure_state_test`、`release_durability_precondition_test`、`free_page_lifecycle_test`、`wal::checkpoint::tests`、`checkpoint_test`、`ddl_layout_publication_test` 都退出码 0。
- 测量测试带有规定的 `#[ignore]`。`git diff` 不含 `io_uring_setup`，default features 不含 `io-uring`。`checkpoint.rs` 的检查点步骤没有被修改。

**Verification**

场景判定就是 T3 Verification 里那些 cargo 命令的退出码，以及测试内对读回字节、写次数、`dirty`、`write_in_flight` 和 `failed_pages` 的断言。`ddl_layout_publication_test` 走目录提交点的真实入口。`wal::checkpoint::tests` 与 `checkpoint_test` 走检查点入口。这些命令一次一个进程。不使用哈希、运行编号或额外判定程序。本 Iteration 不跑收尾全量；全量留在 Iteration Plan 写明的最后一轮。

**Gate 2 Readiness**

PASS。审查计划 11 项通过；RTM 无 Missing、无 Simplified；本 Cycle 只执行值得交付一节，保留逐页写一节不执行；`write_page_run` 的签名、只读文本、不连续文本、注入文本和失败交还已经写明；Persisted Evidence 为 none。无 BLOCKED，无 SKIPPED。goal 指令批准本计划。

**Risks and Notes**

- `pwritev` 使用一组向量还是一块连续缓冲，只要字节分组、偏移和短写重试与 Required behavior 相同，就留给实现选择。这个选择不改变读回字节、写次数或错误文本。
- `page_write_count` 是进程内静态值。每条命令只跑一个测试，并带 `--test-threads=1`。
- 既有 `write_failure_state_test` 只检查注入文本、池里仍有脏页，以及锚点是否挡住提升。它不断言 DashMap 遍历下的某一个页号。若某条既有测试断言了那个顺序里的特定页号，按 Stop when 回到计划。
- 加密分支在 Verification 第 4 步观察到失败之前保持逐页 `write_page`。提前改成加密记录的 `pwritev`，该测试就不会先失败。
- 001 不比较 io_uring。002 仍留在 `tasks.md`，本 Cycle 不建它的目录。

## Act Response

- Status: reported

### 实际改动

- `AsyncStorage::write_page_run`：默认实现按页 `await write_page`，中途失败立即返回。
- `FileStorage::write_page_run`：只读拒绝文本与 `write_page` 相同。页数小于 2 或相邻页号差不是 1 时，在任何字节落盘前返回 `StorageError::Internal`，文本含 `non-contiguous page run`，不消费 `DataWrite`，不调用 `mark_write_failed`。连续时在 `spawn_blocking` 之前消费一次 `DataWrite`；命中则为这一段每一页 `mark_write_failed`，返回含 `injected io failure (write_page)` 的 `Io`，不调用 `pwritev`。未命中时一次 `spawn_blocking` 写出整段。偏移是第一页的 `HEADER_SIZE + page_id.to_offset(record_size)`。明文按页号顺序拼接 4096 字节，加密先 `encrypt_page` 再按同样顺序拼接 4124 字节。写出用一块连续缓冲上的 `pwritev`，短写则前进偏移继续。返回负值、未写完就返回 0，或报告的字节数大于剩余缓冲时，这一段每一页 `mark_write_failed` 并返回 `Io`。成功则每一页 `mark_write_succeeded`。这条路径不调用 `write_all_at`，不增加 `page_write_count`。
- `BufferPool::flush_claimed`：`flush_all` 与 `flush_all_except` 在认领之后按 `PageId` 升序切出相邻页号差为 1 的最大段。长度大于或等于 2 调用 `write_page_run`，长度为 1 调用 `write_page`。调用方收到 `Err` 时，从当前段第一个下标到已排序向量末尾交还脏位并清除 `write_in_flight`。更早已经成功的段保持 `finish_write_in_flight(id, true)`。
- `Cargo.toml`：`libc = "0.2"` 从 `[dev-dependencies]` 移到 `[dependencies]`。default features 仍是 `cli`、`network`、`crypto`。
- 测试：`contiguous_flush_coalesces_a_run_and_keeps_the_gap`、`contiguous_flush_encrypted_pages_roundtrip`、`failed_run_restores_every_page_in_the_run`。`contiguous_dirty_page_write_measure` 加上 `#[ignore = "五次打印已记在 Iteration 000 的执行记录"]`。
- 未改 `flush_pages`、`evict_one`、`allocate_page`、`src/wal/checkpoint.rs`。diff 不含 `io_uring_setup`。

### 与计划的偏差

- 失败集合断言加进合并测试后的第一次运行就是退出码 0。明文 `write_page_run` 在接上 `pwritev` 时已经对成功段调用 `mark_write_succeeded`，对注入失败段的每一页调用 `mark_write_failed`。按 Verification 第 3 步，断言已经通过时保留这次通过，没有拆掉标记再制造失败。
- `failed_run_restores_every_page_in_the_run` 的第一次运行就是退出码 0。`flush_claimed` 在接上合并写时已经从当前段起点交还脏位。按 Verification 第 6 步，已经接上时这一次通过就是 GREEN。
- `pwritev` 使用一块连续缓冲和单个 `iovec`。计划把一组向量或一块连续缓冲留给实现，字节分组、偏移和短写重试与 Required behavior 相同。

### 自检

Gate 3 通过。连续段、单页、排除页、加密记录、注入失败和失败交还都有对应断言。`flush_pages` 仍按调用方顺序逐页 `write_page`。检查点文件和 default features 没有被修改。测试编译没有新增警告。没有身份证明机制或判定层。

Gate 4 通过。下面每条命令都是单独的 cargo 进程，都带 `--test-threads=1`，退出码来自这次实际运行。

### 验证

工作目录是 `/home/daivy/projects/RTsql-changes/2026-11-24-ms54-dirty-page-batch-io`。

改前 GREEN：

1. `cargo test --offline --lib storage::file_storage::tests -- --test-threads=1 --skip contiguous_dirty_page_write_measure`：退出码 0，8 passed。
2. `cargo test --offline --lib storage::buffer_pool::tests::failed_flush_then_evict_keeps_the_new_bytes -- --test-threads=1`：退出码 0，1 passed。

合并测试 RED，然后明文 `pwritev`：

3. 只加入 `contiguous_flush_coalesces_a_run_and_keeps_the_gap` 后，`cargo test --offline --release --lib storage::file_storage::tests::contiguous_flush_coalesces_a_run_and_keeps_the_gap -- --test-threads=1`。第一次编译失败，原因是 `Arc::clone` 被目标类型收成 `Arc<dyn AsyncStorage>`。测试改为先克隆 `Arc<FileStorage>` 再交给 `BufferPool::new`。重跑退出码 101，断言是 `flush of four dirty pages wrote 4 times`。
4. 接上明文 `pwritev`、移动 `libc`、给测量测试加上规定的 `#[ignore]` 后，同一条命令退出码 0，1 passed。加密分支在这一步仍对这一段的每一页调用 `write_page`。
5. 把失败集合断言加进同一个测试后再跑同一条命令：退出码 0，1 passed。这就是上面记下的第一次即通过。

加密 RED，然后加密记录的 `pwritev`：

6. 加入 `contiguous_flush_encrypted_pages_roundtrip` 后，`cargo test --offline --release --lib storage::file_storage::tests::contiguous_flush_encrypted_pages_roundtrip -- --test-threads=1`：退出码 101，断言是 `flush of two encrypted dirty pages wrote 2 times`。
7. 补上加密记录的 `pwritev` 后，同一条命令退出码 0，1 passed。

失败交还：

8. 加入 `failed_run_restores_every_page_in_the_run` 后，`cargo test --offline --lib storage::buffer_pool::tests::failed_run_restores_every_page_in_the_run -- --test-threads=1`：退出码 0，1 passed。这就是上面记下的第一次即通过。

既有命令，按这个顺序，每条退出码 0：

9. `cargo test --offline --lib storage::buffer_pool::tests::failed_flush_then_evict_keeps_the_new_bytes -- --test-threads=1`：1 passed。
10. `cargo test --offline --test write_failure_state_test -- --test-threads=1`：3 passed。
11. `cargo test --offline --test release_durability_precondition_test -- --test-threads=1`：2 passed。
12. `cargo test --offline --test free_page_lifecycle_test -- --test-threads=1`：12 passed。
13. `cargo test --offline --lib wal::checkpoint::tests -- --test-threads=1`：5 passed。
14. `cargo test --offline --test checkpoint_test -- --test-threads=1`：2 passed。
15. `cargo test --offline --test ddl_layout_publication_test -- --test-threads=1`：11 passed。

### Experience Candidates

None

### 未解决问题

无。

## Plan Review

- Review Result: accepted

**Findings**

- 非阻塞 Minor。`contiguous_dirty_page_write_measure` 仍断言不计打印的那次 `flush_all` 使 `page_write_count` 为 64，五次基线的 `baseline_writes` 也仍为 64。合并写之后这条路径走 `pwritev`，不调用 `write_all_at`，计数保持 0。Task Definition 只要求加上规定的 `#[ignore]`，并禁止重跑这个测试。忽略属性已经是 `#[ignore = "五次打印已记在 Iteration 000 的执行记录"]`。默认测试不会运行它。不在本 Cycle 改测试正文。
- 非阻塞 Minor，等价实现。`failed_run_restores_every_page_in_the_run` 用 `PrefixThenFail` 和 `RunThenFail` 两个测试存储，而不是改现有的 `GatedWriteStorage`。两个案例的脏位、`write_in_flight` 和页 6 未被 `write_page` 调用，与 Task Definition 的断言一致。

**Evidence**

- 沿用 Act Response 的验证，不重跑。工作目录是 `/home/daivy/projects/RTsql-changes/2026-11-24-ms54-dirty-page-batch-io`。改前 `storage::file_storage::tests`（跳过测量）8 passed，退出码 0；`failed_flush_then_evict_keeps_the_new_bytes` 1 passed，退出码 0。合并测试先以 `flush of four dirty pages wrote 4 times` 失败，明文 `pwritev` 后 1 passed。失败集合断言第一次即 1 passed。加密测试先以 `flush of two encrypted dirty pages wrote 2 times` 失败，加密 `pwritev` 后 1 passed。`failed_run_restores_every_page_in_the_run` 第一次即 1 passed。随后 `failed_flush_then_evict_keeps_the_new_bytes`、`write_failure_state_test`（3 passed）、`release_durability_precondition_test`（2 passed）、`free_page_lifecycle_test`（12 passed）、`wal::checkpoint::tests`（5 passed）、`checkpoint_test`（2 passed）、`ddl_layout_publication_test`（11 passed）都是退出码 0。这些命令都带 `--test-threads=1`，一次一个 cargo 进程。代码与这些结论同在 `ab7515e2024e3adf764f25590fa66272db70745c`，其后产品文件没有新提交。结论与下面读到的代码一致，不属于公共规则「验证」里的重跑情形。
- `AsyncStorage::write_page_run` 的默认实现按页 `await write_page`，`?` 在第一处错误返回。`FileStorage::write_page_run` 先用与 `write_page` 相同的 `read_only_violation("write a page")` 拒绝只读。`contiguous_page_run` 要求长度至少为 2 且相邻 `id.0` 的 `checked_add(1)` 相等；否则返回 `StorageError::Internal`，文本含 `non-contiguous page run`，此时还没有消费 `DataWrite`，也没有 `mark_write_failed`。连续时在 `spawn_blocking` 之前消费一次 `DataWrite`，命中则为每一页 `mark_write_failed` 并返回含 `injected io failure (write_page)` 的 `StorageError::Io`。未命中时一次 `spawn_blocking` 调用 `write_page_run_blocking`。偏移是 `HEADER_SIZE + pages[0].id.to_offset(record_size)`。明文拼接 `page.data`，加密在 `cipher` 为 `Some` 时先 `encrypt_page` 再拼接。`pwritev_fully` 用一块缓冲和一个 `iovec`，短写则前进偏移。返回负值、未写完的 0，或字节数大于剩余缓冲时，错误经 `From<std::io::Error>` 成为 `StorageError::Io`，调用方对这一段每一页 `mark_write_failed`。成功则 `mark_write_succeeded`。这条路径不调用 `write_all_at`，也不调用 `record_page_write`。
- `flush_claimed` 先按 `PageId` 排序，再按 `checked_add(1)` 切最大连续段。长度大于或等于 2 调用 `write_page_run`，否则 `write_page`。`Err` 时 `restore_dirty_from(&claimed, start)`，从当前段起点到已排序向量末尾把 `dirty` 设回真并清除 `write_in_flight`。成功段调用 `finish_write_in_flight(id, true)`，只清除在途标记。`flush_all` 和 `flush_all_except` 都进入 `flush_claimed`。`flush_all_except` 在认领前跳过排除集。`flush_pages` 仍按调用方顺序逐页 `write_page`。`evict_one` 仍是一次 `write_page`。
- `contiguous_flush_coalesces_a_run_and_keeps_the_gap` 覆盖四页写次数小于 4、页 3 和页 4 的原字节、页 70 的 `0x5A`、64 页写次数小于 64 且读回一致、`flush_all_except` 排除页 5 后写次数小于 3，以及页 0 和页 1 成功后对页 3 和页 4 注入 `DataWrite`。错误文本含 `injected io failure (write_page)`，`failed_pages` 含 3 和 4、不含 0 和 1，页 3 和页 4 的字节不变。`contiguous_flush_encrypted_pages_roundtrip` 带 `#[cfg(feature = "crypto")]`，两页写次数小于 2，同一密钥读回的填充字节一致。`failed_run_restores_every_page_in_the_run` 覆盖默认实现的三页重新变脏，以及两段加单页时更早一段保持不脏、页 3、4、6 重新变脏、页 6 的 `write_page` 未被调用。
- Act Response 记录的三处偏差与计划允许的范围一致。失败集合断言和失败交还测试第一次即通过，Verification 第 3 步和第 6 步允许保留这次通过。`pwritev` 使用一块连续缓冲，Plan Context 把一组向量或一块缓冲留给实现。
- `git diff 2a78e01b44b7275add1f1fe9d539a887e889e4ad..ab7515e2024e3adf764f25590fa66272db70745c` 的产品文件是 `Cargo.toml`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`、`src/storage/file_storage.rs`。`Cargo.toml` 只把 `libc = "0.2"` 从 `[dev-dependencies]` 移到 `[dependencies]`。default features 仍是 `cli`、`network`、`crypto`。`src` 与 change 文档里都没有 `io_uring_setup`。`src/wal/checkpoint.rs` 不在 diff 里。Persisted Evidence 为 none。不因缺少 Evidence 目录记问题。

**Follow-up Decision**

Acceptance 已满足。上面两项不阻塞，不要求当前 Cycle 修复，也不创建 rework 或 replan。Iteration Plan 保持不变。001 已交付合并写，按 `tasks.md` 的既有 Map 展开 Iteration 002，只比较 io_uring，不实现 feature，不把 `io-uring` 放进默认 features。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-11-24-ms54-dirty-page-batch-io/iterations/002-compare-io-uring/000-initial.md`
