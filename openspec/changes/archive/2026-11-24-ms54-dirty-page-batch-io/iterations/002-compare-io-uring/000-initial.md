# Iteration 002 / Cycle 000: 比较 io_uring 并裁定后端

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Inherited scope: `specs/storage-io-optimization/spec.md` 的「io_uring 比较不替换默认后端」正文，以及场景「建环失败则保留当时的后端」。场景「批量写结论之前不比较 io_uring」已经由 Iteration 000 和 001 满足，本 Cycle 不回头执行。场景「默认构建不启用 io_uring」里「已经增加 `io-uring` feature」的启用行为属于 T5；本 Cycle 只保持默认 features 不含 `io-uring`。`design.md` 的 D5。Iteration 001 的执行记录是已经交付连续段合并写。
- Excluded scope: T5。不增加 Cargo feature `io-uring`。不把 `io-uring` 写进 default features。不引入 `tokio-uring` 或其他新依赖。不改非测试写入路径。不改 `flush_all`、`flush_all_except`、`flush_pages`、`evict_one`、`allocate_page`、`write_page`、`write_page_run`。不改 `Cargo.toml`。不重跑 `contiguous_dirty_page_write_measure`，不去掉它的 `#[ignore]`。不重跑 Iteration 001 的验证命令。不改 M18 的四条正文。不给数据文件加 `sync_all`。

**Objective**

用一条 release 测试对页 0 到 63 各计时五次：一侧是现在的 `flush_all`，另一侧是测试模块里的一次 io_uring 写入，提交后等待完成。执行记录按打印写下「值得采用」「保留」或「平台不适合」。默认 features 仍不含 `io-uring`。本 Cycle 不实现 feature。

**Investigation Facts**

- Starting point: 代码修订是 `ab7515e2024e3adf764f25590fa66272db70745c`，分支 `changes/2026-11-24-ms54-dirty-page-batch-io`，审查时工作区干净。Iteration 001 的 Act Response 状态是 `reported`，本轮 Plan Review 接受该结果。合并写已经交付。001 的验证沿用该 Act Response，不重跑：改前两条 GREEN 退出码 0；`contiguous_flush_coalesces_a_run_and_keeps_the_gap` 先以 `flush of four dirty pages wrote 4 times` 失败，明文 `pwritev` 后退出码 0；失败集合断言第一次就是退出码 0；`contiguous_flush_encrypted_pages_roundtrip` 先以 `flush of two encrypted dirty pages wrote 2 times` 失败，加密记录的 `pwritev` 后退出码 0；`failed_run_restores_every_page_in_the_run` 第一次就是退出码 0；随后七条既有命令都是退出码 0。Iteration 000 的裁定是值得交付，五个 `baseline_ns` 的中位数是 2500566，五个 `merged_ns` 的中位数是 119665。那次比较不在本 Cycle 重跑。规划时本机 `io_uring_setup(8)` 曾返回可用描述符，内核是 `6.18.40.1-microsoft-standard-WSL2`。那次探测不代替本测试里的当次 `io_uring_setup`。本 Cycle 的计划提交只增加审查和本文件，不改 `src` 与 `Cargo.toml`。Act 开工时相对 `ab7515e` 的产品代码应无差异。
- Current-State Evidence: `BufferPool::flush_all` 与 `flush_all_except` 在认领后调用 `flush_claimed`。`flush_claimed` 按 `PageId` 升序排列，相邻页号差为 1 的最大段中，长度大于或等于 2 调用 `write_page_run`，长度为 1 调用 `write_page`。连续 64 个脏页因此是一次 `FileStorage::write_page_run`：一次 `spawn_blocking`，明文把 64 个 4096 字节按页号顺序拼进一块缓冲，`pwritev` 的偏移是 `HEADER_SIZE + PageId(0).to_offset(record_size)`。`HEADER_SIZE` 是 64，明文 `record_size` 是 4096，所以偏移是 64，长度是 `64 * 4096`。这条路径不调用 `write_all_at`，不增加 `page_write_count`。`FileStorage::open` 打开无密钥明文库。`FileStorage.file` 是 `Arc<std::fs::File>`，测试模块在 `file_storage.rs` 内部，可以直接克隆它。`Page::PAGE_SIZE` 是 4096。`PageGuard::modify_page` 把帧标脏并改页字节。同文件测试模块已有 `page_pattern`：第 0 字节是页号的低 8 位，其余字节是填充值。`BufferPool::new(128, Arc<FileStorage>)` 可以交给池。`Cargo.toml` 的 default features 是 `cli`、`network`、`crypto`，没有 `io-uring`。`Cargo.lock` 里的 `libc` 是 `0.2.186`，已经在 `[dependencies]`。该版本导出 `libc::SYS_io_uring_setup` 和 `libc::SYS_io_uring_enter`，不导出 `io_uring_params` 或 SQE 结构。本机 `/usr/include/linux/io_uring.h` 的经典布局是 64 字节 SQE、16 字节 CQE、120 字节 `io_uring_params`。`IORING_OP_WRITE` 是 23。`IORING_ENTER_GETEVENTS` 是 1。mmap 偏移是 `IORING_OFF_SQ_RING = 0`、`IORING_OFF_CQ_RING = 0x8000000`、`IORING_OFF_SQES = 0x10000000`。测试模块按这个顺序声明 `#[repr(C)]` 结构，不需要再读头文件。`io_sqring_offsets`：`head`、`tail`、`ring_mask`、`ring_entries`、`flags`、`dropped`、`array`、`resv1` 都是 `u32`，随后 `resv2` 是 `u64`。`io_cqring_offsets`：`head`、`tail`、`ring_mask`、`ring_entries`、`overflow`、`cqes`、`flags`、`resv1` 都是 `u32`，随后 `resv2` 是 `u64`。`io_uring_params`：`sq_entries`、`cq_entries`、`flags`、`sq_thread_cpu`、`sq_thread_idle`、`features`、`wq_fd` 都是 `u32`，随后 `resv: [u32; 3]`、`sq_off`、`cq_off`。完成项是 `user_data: u64`、`res: i32`、`flags: u32`。这些偏移字段是环映射起点的字节偏移，不是下标。
- Code and Critical Path: 本 Cycle 只增加 `src/storage/file_storage.rs` 的 `mod tests` 里的 `io_uring_flush_compare`。产品入口不变。计时的 `flush_all` 走上面的合并写。io_uring 一侧只在这个测试里用 `libc::syscall` 建环、mmap、提交 `IORING_OP_WRITE` 并等待完成，然后 `read_page` 看文件字节。比较不经过 `write_page_run`，也不改它。

**Task Definitions**

### T4: 比较 io_uring

- Requirement/Scenario: 「io_uring 比较不替换默认后端」的比较正文和「建环失败则保留当时的后端」。默认 features 保持不含 `io-uring`。
- Depends on: Iteration 001 的执行记录已经是交付合并写。
- Targets: 只在 `src/storage/file_storage.rs` 的 `mod tests` 增加 `io_uring_flush_compare`。结构体和提交辅助函数也只放在这个测试模块。
- Current behavior: 产品写入是合并写。没有 `io_uring_flush_compare`。非测试代码不调用 `io_uring_setup`。default features 不含 `io-uring`。
- Required behavior: 测试按下面的顺序做完。打印和断言是验收的一部分。裁定词只写进 Act Response，测试源码不打印「值得采用」「保留」或「平台不适合」。
  1. `FileStorage::open` 打开临时明文文件。`allocate_page` 得到页 0 到 70，返回的页号与循环下标相同。页 70 用 `write_page` 写成每个字节 `0x5A`，不放进池的脏集合。页 64 保持分配后的零页，也不放进脏集合。池容量 128，只 `get_page` 页 0 到 63。
  2. 五次循环前先做一次不计时的 `flush_all`：把页 0 到 63 用 `modify_page` 改成可识别内容后刷出。这次不读取 `page_write_count`，也不断言它等于 64。
  3. 建环一次，不是每次循环都建。`libc::syscall(libc::SYS_io_uring_setup, 8u32, params)`。`params` 先全部为零，`flags` 保持 0，不设置 `IORING_SETUP_IOPOLL` 或 `IORING_SETUP_SQPOLL`。返回值小于 0 时，打印一行 `setup=` 接 `std::io::Error::last_os_error().raw_os_error()` 的十进制数，测试返回成功，不进入五次循环，不 mmap，不比较中位数。拿不到 `raw_os_error` 时测试失败。返回值大于或等于 0 时，该值是环的文件描述符，打印一行 `setup=ok`。
  4. `setup=ok` 时，用 `params` 里内核填回的 `sq_entries`、`cq_entries` 和偏移做三次 `mmap`，都是 `PROT_READ | PROT_WRITE` 和 `MAP_SHARED`。SQ 环的长度是 `sq_off.array + sq_entries * 4`，偏移 0。CQ 环的长度是 `cq_off.cqes + cq_entries * 16`，偏移 `0x8000000`。SQE 数组的长度是 `sq_entries * 64`，偏移 `0x10000000`。`mmap` 失败则测试失败，不记成平台不适合。单独映射即可，不要求 `IORING_FEAT_SINGLE_MMAP`。
  5. 然后重复五次，`repeat` 从 0 到 4。每一次先把页 0 到 63 改成 `page_pattern(id, (repeat + 1) as u8)`，只计时 `pool.flush_all().await`，得到 `flush_ns`。再用 `storage.read_page` 核对页 0 到 63 是这一填充，页 64 全是 0，页 70 全是 `0x5A`。接着把页 0 到 63 改成 `page_pattern(id, (repeat + 101) as u8)`。计时从复制这 64 页的字节开始，到 io_uring 把 `64 * 4096` 个字节全部写完结束，得到 `uring_ns`。复制包含在计时里。写入偏移是 `HEADER_SIZE`（64），长度刚好覆盖页 0 到 63，不覆盖页 64 和页 70。缓冲活到对应的完成项被取走。写完后同样核对页 0 到 63、页 64 和页 70。两侧纳秒都大于 0。每一行打印 `repeat=<n> flush_ns=<n> uring_ns=<n>`。
  6. 提交用 `IORING_OP_WRITE`（23），不是 `WRITEV`，也不登记固定缓冲。SQE 是 64 字节：`opcode`、`flags`、`ioprio`、`fd`、`off`、`addr`、`len`、`rw_flags`、`user_data`、`buf_index`、`personality`、`splice_fd_in`、两个 `u64` 填充。除 `opcode`、数据文件 `fd`、`off`、缓冲地址、`len` 和 `user_data` 外都写 0。`user_data` 用非 0 值，完成项必须带回同一个值。下标是 `tail & ring_mask`。填完 SQE 后把 SQ 环的 `array` 槽写成这个下标，再用 `Release` 把 SQ `tail` 加 1。然后 `libc::syscall(libc::SYS_io_uring_enter, ring_fd, 1u32, 1u32, 1u32, 0usize)`。`1u32` 的 flags 是 `IORING_ENTER_GETEVENTS`。返回值小于 0，或没有推进完成队列，测试失败。完成项的 `res` 是这次写出的字节数。`res` 小于或等于 0，或大于剩余缓冲，测试失败。`res` 小于剩余长度时前进偏移和缓冲，仍在同一次 `uring_ns` 里继续提交，直到 64 页全部写完。每取走一个完成项，用 `Release` 把 CQ `head` 加 1。SQ `tail` 的存储和 CQ `head` 的存储用 `Release`，读取对端更新的 `head` 或 `tail` 用 `Acquire`。
  7. 五次结束后 `munmap` 并 `close` 环描述符。不关闭数据文件。
- Preserve: T3 的写入路径。`contiguous_dirty_page_write_measure` 保持 `#[ignore = "五次打印已记在 Iteration 000 的执行记录"]`。`flush_pages`、`evict_one`、`allocate_page`、检查点步骤、M18 的四条。default features 仍是 `cli`、`network`、`crypto`。
- Forbidden: 修改 `Cargo.toml`。修改 `mod tests` 以外的代码。调用 `io_uring_setup` 以外的新依赖。把比较写进 `write_page` 或 `write_page_run`。用「30–50%」或任何百分比作门槛。重跑 T1 或 Iteration 001 的命令。为了让测试编译去改默认 features 或产品写入路径。用 debug 输出替换 release 那次打印来裁定。建环失败时编造五行耗时。
- First test: 函数还不存在时运行 `cargo test --offline --release --lib storage::file_storage::tests::io_uring_flush_compare -- --test-threads=1 --nocapture`。库能编译，输出里这个测试没有被执行，过滤结果是 `0 passed`。这是 RED。不要写一个无法编译的空壳去制造编译错误。`tasks.md` 里的「编译失败」按这次实际的 cargo 过滤行为理解：缺的是测试执行，不是编译。
- GREEN condition: 同一条命令退出码 0，输出有一行 `setup=`。`setup=ok` 时还有五行 `repeat=`，每行都有 `flush_ns=` 和 `uring_ns=`，测试报告 `1 passed`。`setup=` 后面是错误号时没有这五行，测试仍然 `1 passed`。
- Verification: 工作目录是 `/home/daivy/projects/RTsql-changes/2026-11-24-ms54-dirty-page-batch-io`。一次一个 cargo 进程。
  1. 先看到 First test 的 RED，再写入测试，然后跑 GREEN 那一条。只使用这一次 release 输出做裁定，不因为抖动再跑第二次。
  2. `setup` 不是 `ok`：Act Response 写「平台不适合」，抄上 `setup=` 行。不计算中位数。不实现 feature。接着做第 4 步的收尾全量。
  3. `setup=ok` 且测试通过：五个 `uring_ns` 升序后的第三个小于五个 `flush_ns` 升序后的第三个，Act Response 写「值得采用」，并抄上 `setup=ok` 和五行。否则写「保留」，同样抄上这六行。任一侧出现 0 纳秒、字节断言失败、建环成功后 `mmap` 或 `io_uring_enter` 失败，都让测试失败。失败的输出不记成值得采用、保留或平台不适合。停止并回到计划。
  4. 001 已经交付合并写。裁定是「保留」或「平台不适合」时，本轮是最后一轮，按下面的收尾全量跑一次，使用默认 features，不附加 `--features io-uring`，不加 `--ignored` 或 `--include-ignored`。裁定是「值得采用」时不跑收尾全量，也不展开 feature。debug 全量里再次执行这个测试时，不拿那次耗时替换 release 裁定。
  5. 收尾全量：`ls tests/*.rs | sort` 得到 N 个文件。文件批数 `B = ceil(N / 12)`。前 `B × 12 - N` 批各 11 个文件，其余各 12 个。每一批一次 `cargo test --offline --test <名> ... -- --test-threads=1`，测试名是文件名去掉目录和 `.rs`。文件批全部退出码 0 之后，再跑 `cargo test --offline --lib --bins -- --test-threads=1`。批与批顺序执行。每一批退出码为 0，且输出里匹配 `^test result: ` 的行，其 failed 为 0。缺一批不算通过。某一批失败时，不改产品代码来凑通过。Act Response 改为 `blocked`，写上失败命令和那一行 `test result`，回到计划。
  6. `git diff ab7515e2024e3adf764f25590fa66272db70745c -- Cargo.toml src` 中，`Cargo.toml` 无差异；`src` 的差异只在 `src/storage/file_storage.rs` 的 `mod tests` 内。
- Stop when: 测试只有改默认 features、改产品写入路径或增加新依赖才能编译。或字节、耗时、`mmap`、提交、完成队列与上面不一致，却必须改产品路径才能通过。或收尾全量有一批失败。停止并回到计划。不要把 feature 放进默认 features，也不要在本 Cycle 实现 feature。

**Invariants**

- 非测试代码的 `flush_all` 仍是连续段 `pwritev`，单页仍是 `write_page` 的 `write_all_at`。读路径仍是 `read_exact_at`。
- default features 不含 `io-uring`。本 Cycle 不新增这个 feature。
- 测量测试保持规定的 `#[ignore]`。它里面 `page_write_count() == 64` 的断言已经和现在的 `flush_all` 不一致，因为合并写不调用 `write_all_at`。本 Cycle 不修改、不取消忽略、不运行它。
- M18 的四条、检查点步骤、`flush_pages` 和 `evict_one` 不改。
- 测试命令带 `--test-threads=1`。不并行第二条 cargo。

**Acceptance**

- `io_uring_flush_compare` 只存在于 `file_storage.rs` 的测试模块。release 命令退出码 0。
- `setup=ok` 时，输出有这一行和五行 `repeat=` / `flush_ns=` / `uring_ns=`。每次 `flush_all` 和每次 io_uring 写完后，页 0 到 63 与当次写出的字节一致，页 64 全是 0，页 70 全是 `0x5A`，两侧纳秒都大于 0。
- `setup=` 为错误号时，输出有这一行，没有五行耗时，测试仍通过。
- Act Response 只写下一个裁定：`setup` 不是 ok 时是「平台不适合」；`setup=ok` 且 uring 中位数更短时是「值得采用」；`setup=ok` 且 uring 中位数大于或等于 flush 中位数时是「保留」。中位数是五个纳秒值升序后的第三个。不用百分比。
- 裁定不是「值得采用」时，收尾全量每一批退出码 0 且 `test result` 的 failed 为 0。裁定是「值得采用」时，没有跑收尾全量，也没有 feature 差异。
- `Cargo.toml` 无差异。default features 仍是 `cli`、`network`、`crypto`。`src` 的差异只在测试模块。

**Verification**

场景判定就是上面那一条 release 命令的退出码、`setup=` 行、五行里的纳秒，以及测试内对页 0 到 63、页 64、页 70 的读回。裁定由 Act 对这一次输出做中位数比较，不另写判定程序。收尾全量只在裁定不是「值得采用」时执行，判定是每一批的退出码和 `^test result: ` 行的 failed 计数。不使用哈希、运行编号或额外判定程序。

**Gate 2 Readiness**

PASS。审查计划 11 项通过；RTM 里 T4 映射到 `io_uring_flush_compare`，无 Missing、无 Simplified；建环失败、中位数和收尾全量的分支已经写明；不实现 feature，不改默认 features；Persisted Evidence 为 none。无 BLOCKED，无 SKIPPED。goal 指令批准本计划。

**Risks and Notes**

- 五次耗时在 WSL 上会抖动。采纳只看这一次 release 的中位数，不追加第六次，也不重跑替换结果。
- `io_uring_setup` 的当次返回值决定「平台不适合」。规划时的建环成功不保证这次仍成功。
- libc 0.2.186 没有 io_uring 结构体。测试模块按上面的字段宽度自己声明。`size_of` SQE 不是 64、CQE 不是 16 或 `params` 不是 120 时，测试在建环前失败并回到计划。
- 被忽略的测量测试不能当作本轮的基线。它仍断言 64 次 `write_all_at`。
- 收尾全量只证明默认构建的既有测试仍然通过。它不参与中位数。

## Act Response

- Status: reported

实际改动：

- `src/storage/file_storage.rs` 的 `mod tests` 增加 `io_uring_flush_compare`。同模块还有布局结构 `IoUringSqe`、`IoUringCqe`、`IoSqringOffsets`、`IoCqringOffsets`、`IoUringParams`，以及 `UringMaps`、`map_uring`、`uring_write_all`、`assert_uring_compare_bytes`。
- 非测试代码没有改。`Cargo.toml` 没有改。默认 features 仍是 `cli`、`network`、`crypto`。没有增加 `io-uring` feature。
- 五次循环结束后，`UringMaps` 析构时 `munmap` 三次并 `close` 环描述符。数据文件没有关闭。

与计划的偏差：

无行为偏差。`BufferPool::new` 接收 `Arc<dyn AsyncStorage>`。测试写完后的第一次 release 命令在 `Arc::clone(&storage)` 处类型不匹配，退出码 101，测试没有执行，也没有耗时。改为先得到 `Arc<FileStorage>` 再转换之后，才有下面这一次裁定输出。

自检：

类型不匹配已修复。通过的那次编译没有新增 rustc 警告。页 0 到 63、页 64、页 70、建环、`mmap` 和完成队列都由这次通过的测试断言。没有遗留 Minor。

验证：

工作目录是 `/home/daivy/projects/RTsql-changes/2026-11-24-ms54-dirty-page-batch-io`。命令带 `--test-threads=1`，一次一个 cargo。

1. RED，测试函数还不存在。命令：

`cargo test --offline --release --lib storage::file_storage::tests::io_uring_flush_compare -- --test-threads=1 --nocapture`

退出码 0。`running 0 tests`。`test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 464 filtered out`。

2. 写入测试后的第一次同命令：退出码 101。`error[E0308]`，`Arc::clone(&storage)` 期望 `&Arc<dyn AsyncStorage>`，实际是 `&Arc<FileStorage>`。没有运行测试。

3. 修正后的同命令。这一次的输出用于裁定。退出码 0。

```
setup=ok
repeat=0 flush_ns=425476 uring_ns=299838
repeat=1 flush_ns=120110 uring_ns=68438
repeat=2 flush_ns=87114 uring_ns=63427
repeat=3 flush_ns=157533 uring_ns=62194
repeat=4 flush_ns=266090 uring_ns=144907
```

`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 464 filtered out`。

`flush_ns` 升序是 87114、120110、157533、266090、425476，第三个是 157533。`uring_ns` 升序是 62194、63427、68438、144907、299838，第三个是 68438。68438 < 157533。

裁定：值得采用

收尾全量没有跑。裁定是值得采用，所以本 Cycle 不实现 feature，也不把 `io-uring` 放进默认 features。

4. `git diff ab7515e2024e3adf764f25590fa66272db70745c -- Cargo.toml src`：`Cargo.toml` 无差异。`src` 只有 `src/storage/file_storage.rs` 的一个 hunk，`@@ -1458,4 +1458,394 @@ mod tests`。

Experience Candidates: None

未解决问题：无。

## Plan Review

- Review Result: accepted

**Findings**

- 非阻塞 Minor。测试模块里的 `IoUringSqe`、`IoUringCqe`、`IoSqringOffsets`、`IoCqringOffsets`、`IoUringParams` 带 `#[allow(dead_code)]`。这些字段构成内核读写的 `repr(C)` 布局，测试只填写或读取其中一部分。去掉未读字段会改变 SQE 64 字节、CQE 16 字节或 `params` 120 字节。这一项留下，不作为返工。

**Evidence**

- 沿用 Act Response 的验证，不重跑。工作目录是 `/home/daivy/projects/RTsql-changes/2026-11-24-ms54-dirty-page-batch-io`。代码与该结论同在 `5c0a4f67919a1b0615a77f5241b616e5d9ea3ae4`，工作区干净，其后没有新提交。公共规则「验证」列出的重跑情形没有出现。
- RED：测试还不存在时，`cargo test --offline --release --lib storage::file_storage::tests::io_uring_flush_compare -- --test-threads=1 --nocapture` 退出码 0，`running 0 tests`，`test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 464 filtered out`。
- 写入测试后的第一次同命令退出码 101，`error[E0308]`，`Arc::clone(&storage)` 的类型对不上。测试没有运行，没有耗时。修正留在测试模块：`storage` 保持 `Arc<FileStorage>`，克隆后再转成 `Arc<dyn AsyncStorage>` 交给 `BufferPool::new`。裁定只使用修正后那一次的输出。
- 裁定用的那一次退出码 0。`setup=ok`。五行是 `repeat=0 flush_ns=425476 uring_ns=299838`、`repeat=1 flush_ns=120110 uring_ns=68438`、`repeat=2 flush_ns=87114 uring_ns=63427`、`repeat=3 flush_ns=157533 uring_ns=62194`、`repeat=4 flush_ns=266090 uring_ns=144907`。`flush_ns` 升序后的第三个是 157533，`uring_ns` 升序后的第三个是 68438。68438 < 157533。执行记录是值得采用。收尾全量没有跑，`Cargo.toml` 没有 feature 差异。
- 独立阅读 `io_uring_flush_compare`、`uring_write_all`、`map_uring` 和 `UringMaps`。`FileStorage::open` 打开明文临时文件。`allocate_page` 得到页 0 到 70。页 70 经 `write_page` 写成 `0x5A`。池容量 128，只装入页 0 到 63。五次循环前有一次不计时的 `flush_all`。建环一次，`entries` 为 8，`params` 先清零，`flags` 保持 0。失败时打印 `setup=` 加十进制错误号并返回，不进入循环，不 `mmap`。成功时打印 `setup=ok`。三次 `mmap` 的保护是 `PROT_READ | PROT_WRITE`，标志是 `MAP_SHARED`，长度分别由 `sq_off.array + sq_entries * 4`、`cq_off.cqes + cq_entries * 16`、`sq_entries * 64` 算出，偏移是 0、`0x8000000`、`0x10000000`。提交 opcode 是 23。`io_uring_enter` 的参数是环描述符、`1u32`、`1u32`、`IORING_ENTER_GETEVENTS`、`0usize`。SQ `tail` 与 CQ `head` 的存储是 `Release`，对端 `head` 或 `tail` 的读取是 `Acquire`。`user_data` 从 1 起，完成项必须带回同一值。`res` 小于等于 0 或大于剩余长度时断言失败。短写留在同一次 `uring_ns` 里继续提交。偏移是 `HEADER_SIZE`。计时包住从池中复制 64 页到全部字节写完。写完后 `read_page` 核对页 0 到 63、页 64 和页 70。两侧纳秒都要求大于 0。测试源码不打印「值得采用」「保留」或「平台不适合」。五次结束后由 `UringMaps` 析构 `munmap` 三次并 `close` 环描述符，不关闭数据文件。进入建环前断言 SQE 64 字节、CQE 16 字节、`params` 120 字节。
- `git diff ab7515e2024e3adf764f25590fa66272db70745c..HEAD -- Cargo.toml src`：`Cargo.toml` 无差异。`src` 只有 `src/storage/file_storage.rs` 的 `@@ -1458,4 +1458,394 @@ mod tests`。`async_storage.rs` 与 `buffer_pool.rs` 无差异。default features 仍是 `cli`、`network`、`crypto`。`src` 里的 `SYS_io_uring_setup` 只出现在这个测试模块。`contiguous_dirty_page_write_measure` 的忽略属性仍是 `#[ignore = "五次打印已记在 Iteration 000 的执行记录"]`。
- merge-base `2a78e01b44b7275add1f1fe9d539a887e889e4ad` 到 HEAD 的其余产品差异是 001 已接受的 `libc` 移动、`write_page_run` 和 `flush_claimed`。002 没有改这些路径。本轮 Persisted Evidence 为 none。不因缺少 Evidence 目录记问题。

**Follow-up Decision**

Acceptance 已满足。上面一项不阻塞，不要求当前 Cycle 修复，也不创建 rework 或 replan。Iteration Plan 保持不变。执行记录是值得采用。按 `tasks.md` 的既有 Map 展开 Iteration 003：增加默认关闭的 `io-uring` feature，不放进默认 features，并在该轮跑一次收尾全量。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-11-24-ms54-dirty-page-batch-io/iterations/003-io-uring-feature/000-initial.md`
