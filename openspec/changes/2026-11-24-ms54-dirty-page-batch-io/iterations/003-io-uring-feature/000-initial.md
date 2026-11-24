# Iteration 003 / Cycle 000: 默认关闭的 io_uring 写路径

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5
- Inherited scope: `specs/storage-io-optimization/spec.md` 的「默认构建不启用 io_uring」，以及「io_uring 比较不替换默认后端」里“值得采用时只增加默认关闭的 `io-uring` feature”那一段。`design.md` 的 D5。Iteration 002 的执行记录是值得采用，Plan Review 已接受。
- Excluded scope: 不把 `io-uring` 写入 default features。不引入 `tokio-uring` 或其他新依赖。不改 `read_page` 与 `read_page_blocking`。不改 `allocate_page` 里新页的 `set_len` 和直接 `write_all_at`。不改 `flush_claimed`、`flush_pages`、`evict_one`、`async_storage.rs`、`checkpoint.rs`。不改 `io_uring_flush_compare` 及其测试模块里的布局和提交函数。不取消 `contiguous_dirty_page_write_measure` 的 `#[ignore]`，不运行它。不重跑 002 的 release 比较，不用那次之外的耗时更换裁定。不改 M18 的四条正文。不给数据文件加 `sync_all`。

**Objective**

`Cargo.toml` 有 feature `io-uring`，default features 仍是 `cli`、`network`、`crypto`。不启用时，`write_page` 与 `write_page_run` 仍是现在的 `write_all_at` 和 `pwritev`。启用时，这两条路径在现有的 `spawn_blocking` 里提交 `IORING_OP_WRITE` 并等到完成后再返回。注入失败仍在提交前返回，错误文本仍含 `injected io failure (write_page)`。本轮定向测试通过后，按默认 features 跑一次收尾全量。

**Investigation Facts**

- Starting point: 产品代码修订是 `5c0a4f67919a1b0615a77f5241b616e5d9ea3ae4`，分支 `changes/2026-11-24-ms54-dirty-page-batch-io`。002 的 Act Response 状态是 `reported`，裁定是值得采用。那次 release 输出是 `setup=ok`，`flush_ns` 升序后的第三个是 157533，`uring_ns` 升序后的第三个是 68438。002 的 Plan Review 结果是 `accepted`。本 Cycle 的计划提交只写审查、本文件和 `tasks.md` 的状态，不改 `src` 与 `Cargo.toml`。Act 开工时相对 `5c0a4f6` 的产品代码应无差异。002 的验证沿用，不重跑。
- Current-State Evidence: `Cargo.toml` 的 `[features]` 是 `default = ["cli", "network", "crypto"]`，以及 `cli`、`network`、`crypto`。没有 `io-uring`。`libc = "0.2"` 在 `[dependencies]`，`Cargo.lock` 里的版本是 0.2.186。002 的测试已经用这个版本调用 `libc::SYS_io_uring_setup` 和 `libc::SYS_io_uring_enter`。该版本没有 io_uring 的结构体。`FileStorage::write_page` 先拒绝只读，再消费 `IoFailPoint::DataWrite`。命中时 `mark_write_failed` 并返回 `StorageError::Io`，文本含 `injected io failure (write_page)`，不进入 `spawn_blocking`。未命中时一次 `spawn_blocking` 调用 `write_page_blocking`。成功则 `mark_write_succeeded`，失败则 `mark_write_failed`。`write_page_run` 先拒绝只读，再用 `contiguous_page_run` 要求长度至少为 2 且相邻页号 `checked_add(1)` 相等；否则返回 `StorageError::Internal`，文本含 `non-contiguous page run`，此时还没有消费 `DataWrite`。连续时在 `spawn_blocking` 之前消费一次 `DataWrite`，命中则为每一页 `mark_write_failed` 并返回同一句注入错误。未命中时一次 `spawn_blocking` 调用 `write_page_run_blocking`。`write_page_blocking` 的偏移是 `HEADER_SIZE + page_id.to_offset(record_size)`。`HEADER_SIZE` 是 64，`Page::PAGE_SIZE` 是 4096，加密记录是 4124。`crypto` feature 打开且 `cipher` 为 `Some` 时先 `encrypt_page`，再 `write_all_at` 这一条记录，然后 `record_page_write` 并返回。明文，以及 `cipher` 为 `None` 时，`write_all_at` 页字节并 `record_page_write`。默认构建包含 `crypto`，明文库的 `cipher` 仍是 `None`，所以会落到后面的 `write_all_at`。`write_page_run_blocking` 把一段拼进一块缓冲：加密且 `cipher` 为 `Some` 时每页先 `encrypt_page`，否则复制 `page.data`。然后 `pwritev_fully`。这条路径不调用 `record_page_write`。`pwritev_fully` 短写则前进偏移；返回负值、未写完的 0，或字节数大于剩余缓冲时返回错误。`page_write_count` 只在 `write_page_blocking` 的 `write_all_at` 成功之后增加。`BufferPool::new` 的签名是 `(usize, Arc<dyn AsyncStorage>)`。`Arc<FileStorage>` 在参数位置可以转成 `Arc<dyn AsyncStorage>`。`PageGuard::modify_page` 把帧的 `dirty` 设为真。`flush_claimed` 按页号升序切连续段，长度大于或等于 2 调用 `write_page_run`，否则 `write_page`。两页连续 `flush_all` 因此是一次 `write_page_run`，`page_write_count` 保持 0。单页 `write_page` 成功则 `page_write_count` 加 1。调用 `write_page` 的产品位置还有 `flush_pages`、`evict_one` 和 `allocate_page` 的空闲页清零。`allocate_page` 分配新页时，明文用 `set_len`，加密用一次直接 `write_all_at`，不经过 `write_page_blocking`。`read_page_blocking` 使用 `read_exact_at`。测试模块有 `use super::*`，并自己声明了 `IoUringSqe`、`IoUringCqe`、`IoSqringOffsets`、`IoCqringOffsets`、`IoUringParams`。002 的测试已经把这些布局跑通：SQE 64 字节，CQE 16 字节，`params` 120 字节。
- Code and Critical Path: 本 Cycle 改 `Cargo.toml` 的 `[features]`，以及 `src/storage/file_storage.rs` 的 `write_page_blocking` 与 `write_page_run_blocking`。启用 feature 时，这两处在现有 `spawn_blocking` 内把 `write_all_at` 或 `pwritev_fully` 换成一次 io_uring 写完。`write_page` 与 `write_page_run` 的只读判断、连续性判断和注入消费留在 `spawn_blocking` 之前。读路径、`flush_claimed` 和 `evict_one` 的函数体不改。启用 feature 后，`flush_all`、`flush_pages`、`evict_one` 和空闲页清零会因为调用 `write_page` 或 `write_page_run` 而走到新的阻塞写。

**Task Definitions**

### T5: 默认关闭的 io_uring feature

- Requirement/Scenario: 「默认构建不启用 io_uring」；「io_uring 比较不替换默认后端」中启用 feature 的正文。
- Depends on: Iteration 002 的执行记录是值得采用。
- Targets: `Cargo.toml` 的 `[features]`。`src/storage/file_storage.rs` 的 `write_page_blocking`、`write_page_run_blocking`，以及同文件里 `#[cfg(feature = "io-uring")]` 的私有子模块。测试加在该文件已有的 `mod tests`：`io_uring_feature_off_keeps_the_decided_write_path`、`io_uring_feature_on_write_roundtrip`。计数函数放在现有的 `page_write_count` 旁边。
- Current behavior: 没有 `io-uring` feature。两页连续脏页的 `flush_all` 走一次 `write_page_run` 的 `pwritev`，`page_write_count` 不增加。单页 `write_page` 走 `write_all_at`，成功后 `page_write_count` 加 1。`DataWrite` 在 `spawn_blocking` 之前消费。读是 `read_exact_at`。
- Required behavior: 按下面的顺序做完。
  1. 在 `[features]` 增加 `io-uring = []`。`default` 数组保持 `cli`、`network`、`crypto`。不增加依赖，不改 `Cargo.lock` 的包列表。`cargo test --features io-uring` 会在默认 features 之外再启用它。本轮的启用命令不要加 `--no-default-features`，这样 `crypto` 仍会编进来。
  2. 私有子模块 `io_uring_write` 放在 `#[cfg(feature = "io-uring")]` 下。布局类型和提交函数只放在这个子模块里，不放到 `file_storage` 模块本身。测试模块的 `use super::*` 因此不会导入这些类型名。结构按 002 已跑通的宽度声明，`#[repr(C)]`。SQE：`opcode: u8`、`flags: u8`、`ioprio: u16`、`fd: i32`、`off: u64`、`addr: u64`、`len: u32`、`rw_flags: u32`、`user_data: u64`、`buf_index: u16`、`personality: u16`、`splice_fd_in: i32`、`pad: [u64; 2]`，共 64 字节。CQE：`user_data: u64`、`res: i32`、`flags: u32`，共 16 字节。`io_sqring_offsets` 的 `head`、`tail`、`ring_mask`、`ring_entries`、`flags`、`dropped`、`array`、`resv1` 都是 `u32`，随后 `resv2: u64`。`io_cqring_offsets` 的 `head`、`tail`、`ring_mask`、`ring_entries`、`overflow`、`cqes`、`flags`、`resv1` 都是 `u32`，随后 `resv2: u64`。`io_uring_params`：`sq_entries`、`cq_entries`、`flags`、`sq_thread_cpu`、`sq_thread_idle`、`features`、`wq_fd` 都是 `u32`，随后 `resv: [u32; 3]`、`sq_off`、`cq_off`，共 120 字节。
  3. 子模块提供一个写完全部字节的函数。它接收数据文件、缓冲和起始偏移。`params` 先全部为零，`flags` 保持 0，`io_uring_setup` 的 entries 是 `8u32`。不设置 `IORING_SETUP_IOPOLL` 或 `IORING_SETUP_SQPOLL`。建环失败返回 `std::io::Error::last_os_error()`，不 `mmap`，不回退到 `write_all_at` 或 `pwritev`。成功后按内核填回的 entries 和偏移做三次 `mmap`，保护是 `PROT_READ | PROT_WRITE`，标志是 `MAP_SHARED`。SQ 环长度是 `sq_off.array + sq_entries * 4`，偏移 0。CQ 环长度是 `cq_off.cqes + cq_entries * 16`，偏移 `0x8000000`。SQE 数组长度是 `sq_entries * 64`，偏移 `0x10000000`。某次 `mmap` 失败时，解除已经成功的映射，`close` 环描述符，返回 `last_os_error()`。此后任何返回路径都 `munmap` 并 `close` 环描述符。不关闭数据文件。
  4. 提交使用 `IORING_OP_WRITE`（23），一次一个 SQE，不登记固定缓冲。除 `opcode`、数据文件 `fd`、偏移、缓冲地址、长度和 `user_data` 外，SQE 字段写 0。`user_data` 从 1 起，每次提交加 1，绕回 0 时改回 1。下标是 `tail & ring_mask`。写完 SQE 后把 SQ 环的 `array` 槽写成这个下标，再用 `Release` 把 SQ `tail` 加 1。然后 `libc::syscall(libc::SYS_io_uring_enter, ring_fd, 1u32, 1u32, 1u32, 0usize)`。最后一个 `1u32` 是 `IORING_ENTER_GETEVENTS`。返回值小于 0 时返回 `last_os_error()`。完成队列没有前进时返回错误。完成项的 `user_data` 必须等于这次提交的值，否则返回错误。`res` 小于 0 时，能安全取负就用 `std::io::Error::from_raw_os_error` 返回该错误码，否则返回 `Error::other`。`res` 等于 0 时返回 `WriteZero`。`res` 大于剩余长度时返回错误。`res` 小于剩余长度时前进偏移和缓冲，仍在同一次调用里继续提交，直到缓冲写完。每取走一个完成项，用 `Release` 把 CQ `head` 加 1。读取对端更新的 `head` 或 `tail` 用 `Acquire`。缓冲活到对应完成项被取走。写完整个缓冲后，在 `#[cfg(test)]` 下调用父模块的 `record_uring_write` 一次。短写重试不加多次。非测试构建不调用它，否则 `cargo build --features io-uring` 会找不到这个函数。子模块返回 `std::io::Result<()>`。两个 blocking 函数用 `?` 把错误变成 `StorageError::Io`，现有的 `Err` 分支继续 `mark_write_failed`。SQE 字节和 array 槽的写入都发生在 SQ `tail` 那次 `Release` 存储之前。
  5. `#[cfg(not(feature = "io-uring"))]` 时，`write_page_blocking` 与 `write_page_run_blocking` 保持现在的 `write_all_at`、`pwritev_fully` 和 `record_page_write`。`#[cfg(feature = "io-uring")]` 时，两处都改为调用子模块。加密分支仍先 `encrypt_page`，再把记录交给 io_uring。明文分支把原来交给 `write_all_at` 或 `pwritev_fully` 的字节交给 io_uring。启用时这两处都不调用 `record_page_write`。偏移、记录长度和拼接顺序不变。`write_page` 与 `write_page_run` 里 `spawn_blocking` 之前的只读判断、连续性判断和 `DataWrite` 消费不改，注入文本不改。每次阻塞调用自己建环并在返回前拆掉，不把环存在 `FileStorage` 上。
  6. `#[cfg(all(test, feature = "io-uring"))]` 增加 `reset_uring_write_count`、`uring_write_count` 和 `record_uring_write`，放在现有页计数旁边。测试模块像调用 `super::page_write_count` 一样调用 `super::reset_uring_write_count` 和 `super::uring_write_count`。默认构建不编译这三个函数。
  7. `io_uring_feature_off_keeps_the_decided_write_path` 在默认构建运行。`FileStorage::open` 打开临时明文文件，`allocate_page` 得到页 0 和页 1，页号与下标相同。`storage` 保持 `Arc<FileStorage>`，用 `BufferPool::new(8, Arc::clone(&storage) as Arc<dyn AsyncStorage>)` 建池。两页 `get_page` 后 `modify_page` 写成 `page_pattern(id, 0x21)` 与 `page_pattern(id, 0x22)`。`reset_page_io_counts()` 之后 `flush_all` 成功。`page_write_count()` 小于 2。两页 `read_page` 与写出的字节一致。`assert!(!cfg!(feature = "io-uring"))`。
  8. `io_uring_feature_on_write_roundtrip` 标成 `#[cfg(feature = "io-uring")]`。每个案例在待测写入之前 `reset_uring_write_count()` 和 `reset_page_io_counts()`。明文两页连续 `flush_all` 之后，`uring_write_count()` 为 1，`page_write_count()` 为 0，两页读回一致。明文一次 `write_page` 之后，两个计数同样是 1 和 0，该页读回一致。加密库用 `open_with_key(..., Some("pw"))`，两页 `flush_all` 和一次 `write_page` 也各满足这两个计数，并用同一密钥读回明文。明文先成功写出可识别字节，再武装 `DataWrite`，然后 `write_page_run` 写出不同字节：错误文本含 `injected io failure (write_page)`，`uring_write_count()` 为 0，读回仍是旧字节，`failed_pages` 含这两页。`write_page` 的注入同样检查错误文本、计数为 0、旧字节还在，以及 `failed_pages` 含该页。`flush_all` 返回成功时，字节已经能被 `read_page` 读到。
- Preserve: default features。`injected io failure (write_page)`。`mark_write_failed` 与 `mark_write_succeeded` 的调用点。`non-contiguous page run` 与只读拒绝。`read_exact_at`。`allocate_page` 新页的 `set_len` 和直接 `write_all_at`。`flush_pages` 的顺序。`evict_one` 的选页。检查点步骤。M18 的四条。测量测试的 `#[ignore]`。`io_uring_flush_compare`。
- Forbidden: 把 `io-uring` 放进 `default` 数组。依赖 `tokio-uring`。把读改成 io_uring。把环放进 `FileStorage` 或进程级静态变量。建环失败时回退到 `write_all_at` 或 `pwritev`。把注入移到 `spawn_blocking` 里面或改掉错误文本。修改 `buffer_pool.rs`、`async_storage.rs`、`checkpoint.rs`。修改比较测试来消除同名冲突。用「30–50%」或重跑 002 的 release 作为本轮通过条件。给收尾全量加上 `--features io-uring`、`--ignored` 或 `--include-ignored`。
- First test: 工作目录是 `/home/daivy/projects/RTsql-changes/2026-11-24-ms54-dirty-page-batch-io`。一次一个 cargo 进程，命令带 `--test-threads=1`。
  1. 测试还不存在时运行 `cargo test --offline --lib storage::file_storage::tests::io_uring_feature_off_keeps_the_decided_write_path -- --test-threads=1`。退出码 0，过滤结果是 `0 passed`。这只说明测试还没执行。
  2. 按上面第 7 点写关闭 feature 的测试，先不改 `Cargo.toml`，也不改写入路径。再跑同一条命令。退出码 0，`1 passed`。这是改写路径之前的 GREEN。两页合并写的 `page_write_count() < 2` 现在已经成立。断言保持小于 2。被替代的做法是写成等于 2 来制造失败，那和已经交付的合并写相反。若这次不是 `1 passed`，停止并回到计划。
- GREEN condition: 启用测试在写路径完成前失败，完成后退出码 0。关闭 feature 的测试在写路径完成后再跑一次，默认构建仍是退出码 0、`1 passed`。
- Verification: 继续上面的顺序。
  3. 先只增加 `io-uring = []` 和计数函数。这一步不把 `write_page_blocking` 或 `write_page_run_blocking` 改成 io_uring，`record_uring_write` 还不会被调用。写上 `io_uring_feature_on_write_roundtrip`，断言仍按第 8 点。运行 `cargo test --offline --features io-uring --lib storage::file_storage::tests::io_uring_feature_on_write_roundtrip -- --test-threads=1`。此时完成次数为 0，测试失败。这是 RED。先看到这次失败，再做第 4 步。失败输出不记成功能完成。
  4. 再把两个 blocking 函数的启用分支接到子模块。同一条启用命令退出码 0，`1 passed`。然后用默认构建再跑一次关闭 feature 的命令，退出码 0，`1 passed`。
  5. `git diff 5c0a4f67919a1b0615a77f5241b616e5d9ea3ae4 -- Cargo.toml src`。`Cargo.toml` 的 `default` 数组仍是 `cli`、`network`、`crypto`，并新增 `io-uring = []`，依赖列表没有新包。`src` 只有 `src/storage/file_storage.rs`。若 `io-uring` 出现在 `default` 数组里，先移出去，再重跑受影响的那一条定向测试。移出之后启用构建仍不能通过，才属于 Stop when。
  6. 然后跑一次收尾全量。使用默认 features，不附加 `--features io-uring`，不加 `--ignored` 或 `--include-ignored`。`ls tests/*.rs | sort` 得到 N。规划时 N 是 171。文件批数 `B = ceil(N / 12)`。前 `B × 12 - N` 批各 11 个文件，其余各 12 个。N 仍是 171 时，B 是 15，前 9 批各 11 个，后 6 批各 12 个。文件按排序结果切，测试名是文件名去掉目录和 `.rs`。每一批一次 `cargo test --offline --test <名> ... -- --test-threads=1`。文件批全部退出码 0 之后，再跑 `cargo test --offline --lib --bins -- --test-threads=1`。批与批顺序执行。每一批退出码为 0，且输出里每一行匹配 `^test result: ` 的 failed 都是 0。缺一批不算通过。库测试批会再次运行 `io_uring_flush_compare`。那次输出不替换 002 的裁定。
  7. 某一批失败时，若失败来自本 Cycle 改过的默认构建路径，在本 Task Definition 里修好，只重跑失败的那一批。已经退出码 0 的批次不重跑。失败与这些改动无关，或同一批修完仍失败，Act Response 改为 `blocked`，写上失败命令和那一行 `test result`，回到计划。不把 `io-uring` 放进默认 features 来换通过。
- Stop when: 启用构建只有把 `io-uring` 放进 default features 才能通过。或等待完成必须改掉 `injected io failure (write_page)` 的文本。或建环在启用测试里失败，而通过方式是回退到 `pwritev` 或 `write_all_at`。或必须修改 `read_page_blocking`、`evict_one` 的选页、检查点步骤才能通过。停止并回到计划。同一批收尾失败连续三次也停止，不开始第四次同类修改。

**Invariants**

- 默认构建的 `flush_all` 与 `flush_all_except` 仍把连续段交给 `pwritev`，单页仍是 `write_page` 的 `write_all_at`。
- default features 不含 `io-uring`。
- `DataWrite` 仍在任何字节落盘之前消费，错误文本不变。失败页进入 `failed_pages`，成功页离开。
- 读路径仍是 `read_exact_at`，不调用 `lseek`。
- `flush_pages` 仍按调用方顺序逐页 `write_page`。`evict_one` 的选页规则不改。检查点仍不调用数据文件 `sync_all`。
- 测试命令带 `--test-threads=1`。不并行第二条 cargo。

**Acceptance**

- `io-uring` 出现在 `Cargo.toml` 的 feature 表里，`default` 数组仍是 `cli`、`network`、`crypto`。没有 `tokio-uring`，依赖列表没有新包。
- 默认构建的 `io_uring_feature_off_keeps_the_decided_write_path` 退出码 0。两页连续 `flush_all` 的 `page_write_count()` 小于 2，读回一致，并且该构建里 `cfg!(feature = "io-uring")` 为假。
- `cargo test --offline --features io-uring --lib storage::file_storage::tests::io_uring_feature_on_write_roundtrip -- --test-threads=1` 退出码 0。明文和加密的两页 `flush_all`、明文和加密的单页 `write_page`，完成后 `uring_write_count()` 为 1、`page_write_count()` 为 0，读回一致。两次注入的错误文本含 `injected io failure (write_page)`，完成次数为 0，旧字节还在，失败集合含对应页。
- `read_page_blocking` 仍调用 `read_exact_at`。`write_page` 与 `write_page_run` 的注入仍在 `spawn_blocking` 之前。启用时的等待发生在这两个 blocking 函数里，函数返回前环描述符已关闭。
- 收尾全量每一批退出码 0，且每一行 `^test result: ` 的 failed 为 0。这是本 change 的一次收尾全量。
- `git diff 5c0a4f67919a1b0615a77f5241b616e5d9ea3ae4 -- src` 只有 `src/storage/file_storage.rs`。

**Verification**

关闭 feature 的判定是默认构建那条库测试的退出码、`page_write_count()` 和读回字节。启用路径的判定是 `--features io-uring` 那条库测试的退出码、完成次数、`page_write_count()`、读回字节和注入错误文本。两条都走 `FileStorage` 与 `BufferPool::flush_all` 或 `write_page`，不另写判定程序。收尾全量的判定是每一批的退出码和 `^test result: ` 行的 failed 计数。完成次数只用来区分 io_uring 和现有的 `pwritev`：`page_write_count` 在 `pwritev` 上保持 0，不能单独证明启用路径。不使用哈希、运行编号或额外判定程序。

**Gate 2 Readiness**

PASS。审查计划 11 项通过；RTM 无 Missing、无 Simplified；feature 的位置、启用后的错误返回、注入时点和收尾全量都已写明；Persisted Evidence 为 none。无 BLOCKED，无 SKIPPED。goal 指令批准本计划。

**Risks and Notes**

- 每次写入在当次阻塞调用里建环、写完、拆环，不把环存在 `FileStorage` 上。重叠写入各自持有自己的环。被替代的做法是一个环加一把锁。验收没有要求分摊建环成本，002 的中位数也不在本轮重跑。
- feature 使用空的 `io-uring = []`。被替代的做法是 `tokio-uring`，规格不允许。
- 启用测试用完成次数区分 io_uring 和 `pwritev`。被替代的做法是只看 `page_write_count() == 0`。那在今天的合并写上已经成立，写路径改完之前启用测试就会通过。
- 关闭 feature 的合同断言先观察 GREEN。被替代的做法是把 `page_write_count()` 断言写成等于 2。那和已经交付的合并写相反。
- 启用测试里的 `io_uring_setup` 若返回错误，测试失败。停止并回到计划。不回退到 `pwritev`，也不把 feature 放进默认 features。002 当时的 `setup=ok` 不代替这次调用。
- 收尾全量里的 `io_uring_flush_compare` 可能再打印五行。那次耗时不参加任何裁定。
- 短写在同一次阻塞调用里继续提交。本轮测试不制造短写。

## Act Response

- Status: 尚未执行

## Plan Review

- Review Result: pending
