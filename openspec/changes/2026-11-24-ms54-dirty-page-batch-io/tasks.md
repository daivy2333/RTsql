# MS54 脏页批量写与 I/O 后端

来源：`.agents/docs/tasks.md` 的 MS54，承接 I026 与 I028。约束：M18 的四条不变量保持；M19 要求每条测试命令带 `--test-threads=1`，多次命令顺序执行。历史「5–10 倍」和「30–50%」不是采纳条件。

Gate 1：PASS。需求、场景和范围来自 MS54 已写明的字段，以及 `proposal.md` 里由本次 goal 授权接受的默认假设。没有 SKIPPED。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格在 `specs/storage-io-optimization/spec.md`。
- 000：Plan Context 见 `iterations/000-measure-and-decide/000-initial.md`。Act Response 状态 `reported`。Review Result 为 `accepted`。执行记录是值得交付。
- 001：`iterations/001-apply-batch-decision/000-initial.md`，Plan Context `ready`，Gate 2 PASS。Act 未开始。
- 002、003：只列在下面的 Iteration Plan 里，不建目录。
- Persisted Evidence：none。
- 跳过项：无。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | release 测试打印五次写次数和耗时，非测试写入行为不变 | reported |
| T2 | 000 | T1 | 用 T1 的五行裁定值得交付或保留逐页写 | reported |
| T3 | 001 | T2 记下了值得交付或保留逐页写 | 按裁定交付连续段合并写，或确认仍是逐页写 | 未执行 |
| T4 | 002 | T3 的结论已写入执行记录 | 比较 io_uring，默认 features 不变 | 未执行 |
| T5 | 003 | T4 记下了值得采用 | 增加默认关闭的 `io-uring` feature | 未执行 |

T1 与 T2 的执行依据是 `iterations/000-measure-and-decide/000-initial.md` 的 Task Definition。本文件不另写一套步骤。T5 只在 T4 记下值得采用之后展开。T4 记下保留或平台不适合时，不展开 003。

## Iteration Plan

### Iteration 000: 测量连续脏页写入并裁定批量写去留

- Tasks: T1, T2
- Depends on: None
- Stable result: 五行打印、一次同步打印，以及「值得交付」或「保留逐页写」。非测试构建的 `write_page` 仍是一页一次 `spawn_blocking` 和一次 `write_all_at`。
- Verification boundary: 只跑 T1 的那一条 release 命令。不跑全量。这轮不改检查点、释放提升或页格式，全量读不到这五行。风险：合回前没有全量；diff 若只含 `src/storage/file_storage.rs` 的测试计数和测试函数，排查范围就是这个文件。
- Diagnostic boundary: 失败只看这一个测试的打印、64 页读回、页 64 与页 70，以及计数是否加在 `write_page` / `write_page_blocking` / `sync`。不把 io_uring、目录提交点、分配器或索引页格式纳入排查。
- Non-goals: 不在非测试代码里调用 `pwritev`。不调用 `io_uring_setup`。不改 `flush_all` 的逐页循环。不展开 001。

平衡：打印和裁定是同一次测量的两个结果。只打印而不裁定，下一轮没有可依赖的结论；只裁定而没有这五行，结论没有测量来源。实现合并写的失败模式是脏位、释放和检查点，与这次打印不是同一个排查范围，所以不并进本轮。

### Iteration 001: 按裁定交付批量写或保留逐页写

- Tasks: T3
- Depends on: Iteration 000 的执行记录写了「值得交付」或「保留逐页写」。记录若是测量无效，不展开本轮
- Stable result: 保留时，页写入仍是逐页 `write_page`。值得交付时，`flush_all` 与 `flush_all_except` 把连续页号合成 `pwritev`，失败重试、释放提升和 M18 的四条保持，`flush_pages` 与 `evict_one` 仍逐页。
- Verification boundary: 受影响回归，命令见 001 Cycle 的 Verification，全部 `--test-threads=1`，一次一个 cargo 进程。不在本轮跑全量。合并写触及刷盘失败和释放提升的生命周期，全量留到最后一轮执行的那次收尾，不在这里提前跑。
- Diagnostic boundary: `src/storage/file_storage.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs` 的 `flush_all` 与 `flush_all_except`，以及 T3 列出的测试。`evict_one` 的选页、`src/wal/checkpoint.rs` 的步骤顺序、WAL 组提交不在排查范围。
- Non-goals: 不比较 io_uring。不给数据文件加 `sync_all`。不合并 `flush_pages`。不改 M18 的正文。

平衡：保留分支没有产品写入改动，交付分支的合并写和它的失败、空洞、加密读回是同一个刷盘故障域。拆成「先改写入、再补失败语义」会让半套 `pwritev` 没有脏位合同。io_uring 是另一个后端，不并进本轮。

### Iteration 002: 比较 io_uring 并裁定后端

- Tasks: T4
- Depends on: Iteration 001 的执行记录已经写明保留逐页写，或已经交付合并写
- Stable result: io_uring 与当时 `flush_all` 的五次耗时，以及「值得采用」「保留」或「平台不适合」。默认 features 仍不含 `io-uring`。
- Verification boundary: T4 的一条 release 测试。001 已交付合并写且本轮结论是保留或平台不适合时，再按下面的收尾全量跑一次，因为本轮成为最后一轮。结论是值得采用时不在本轮跑全量。001 只保留逐页写时不跑全量。
- Diagnostic boundary: T4 的测试输出、`io_uring_setup` 的返回值，以及 `Cargo.toml` 的默认 features。不改索引页格式，不把建环失败算成批量写测量无效。
- Non-goals: 不把 io_uring 放进默认 features。不引入 `tokio-uring`。不在本轮实现 feature。

平衡：比较本身给出后端去留。feature 的缓冲区寿命和注入点是另一轮的故障域。本轮在「不再有下一轮」时兼做收尾全量，避免保留结论之后还空转一轮只为跑测试。

### Iteration 003: 默认关闭的 io_uring 写路径

- Tasks: T5
- Depends on: Iteration 002 的执行记录写了「值得采用」。写的是「保留」或「平台不适合」时不展开本轮
- Stable result: feature `io-uring` 存在且默认关闭。启用时页写在阻塞线程里等待 io_uring 完成；不启用时与 001 留下的写入路径相同。默认 features 不含该 feature。
- Verification boundary: T5 的定向测试，外加一次收尾全量。收尾全量覆盖默认构建。不再为启用 feature 跑第二套全量；启用路径只跑 T5 列出的定向命令。
- Diagnostic boundary: `src/storage/file_storage.rs` 里 feature 门控的写路径，以及 T5 的测试。读路径、检查点步骤、`evict_one` 选页不在排查范围。
- Non-goals: 不修改默认 features。不把读改成 io_uring。不改 M18。

平衡：这是采用结论的实现轮。它和 002 的计时不是同一个故障域。条件不成立时不展开，避免一轮只重复「保留」这两个字。

## 收尾全量

只在 Iteration Plan 写明的那一轮执行，并且只执行一次。

1. `ls tests/*.rs | sort`，条数为 N。文件批数 `B = ceil(N / 12)`。文件按该顺序切片：前 `B × 12 - N` 批各 11 个，其余各 12 个。
2. 每一文件批一次 `cargo test --offline --test <名> ... -- --test-threads=1`。测试名是文件名去掉目录和 `.rs`。
3. 文件批全部结束后，再跑 `cargo test --offline --lib --bins -- --test-threads=1`。
4. 批与批顺序执行。每一批退出码为 0，且输出里匹配 `^test result: ` 的行，其 failed 为 0。缺一批不算通过。

## Requirements Traceability Matrix

| Requirement | Task | First Test |
|---|---|---|
| 连续脏页写入的测量 | T1 | `contiguous_dirty_page_write_measure` |
| 测量无效时不裁定 | T1 | `contiguous_dirty_page_write_measure` |
| 批量写去留由这五次打印裁定 | T2 | T1 打印的 `repeat=` 行 |
| 保留逐页写 | T3 | T1 的打印；`git diff` 中非测试写入路径为空 |
| 值得交付时只合并连续脏页段 | T3 | `contiguous_flush_coalesces_a_run_and_keeps_the_gap` |
| 刷盘失败后脏页仍在 | T3 | `failed_write_keeps_page_dirty` |
| 合并写失败时整段仍为脏 | T3 | `failed_run_restores_every_page_in_the_run` |
| 释放仍按锚点页的失败集合提升 | T3 | `anchor_stays_unpromoted_while_its_page_is_failed` |
| 检查点不调用数据文件 sync，恢复门控不改 | T3 | `wal::checkpoint::tests` |
| io_uring 在批量写结论之后比较，且不进入默认 features | T4 | `io_uring_flush_compare` |
| 值得采用时才增加默认关闭的 feature | T5 | `io_uring_feature_off_keeps_the_decided_write_path` |

没有 Simplified。没有 Missing。T3 的交付类测试只在裁定为值得交付时编写。T4、T5 不在 000 执行。保留逐页写时，T3 不编写合并写测试，对应行由「保留逐页写」那一行的 diff 检查覆盖，不把未编写记成 Missing。

## T3: 按裁定交付或保留

本任务的执行依据是 `iterations/001-apply-batch-decision/000-initial.md` 的 Task Definition。000 的 Cycle 不执行。T2 的记录是值得交付，所以只执行下面的「值得交付」一节。

### 保留逐页写

- Requirement/Scenario: 「批量写去留」中写次数未减少或中位数不更短的场景，以及「页 I/O 使用位置参数调用」的保留场景。
- Depends on: T2 记录的是保留逐页写。
- Targets: 无产品写入改动。`src/storage/file_storage.rs` 保持 T1 留下的测试计数和测量测试。
- Current behavior: `flush_all` 对每一脏页调用一次 `write_page`。
- Required behavior: 执行记录写明保留逐页写，并抄上 T1 的五行、两个中位数和 `sync_calls` 行。非测试代码相对 T1 完成时没有新的差异。
- Preserve: 测量测试不加重 `#[ignore]`。错误文本、检查点步骤、`evict_one`。
- Forbidden: 调用 `pwritev` 或 `io_uring_setup`。修改 `flush_all`、`Cargo.toml`。重跑 T1。
- First test: 不新写测试。沿用 T1 已通过的 `contiguous_dirty_page_write_measure`。
- GREEN condition: `git diff <T1 完成提交> -- src Cargo.toml` 为空。
- Verification: 只做上述 diff。不跑测试。diff 非空表示这节不该改代码。
- Stop when: T2 的记录不是保留逐页写，或必须改 `flush_all` 才能把「保留」写进执行记录。停止并回到计划。

### 值得交付

- Requirement/Scenario: 「值得交付时只合并连续脏页段」的全部场景，以及「失败重试、释放耐久性与恢复约定保持」。
- Depends on: T2 记录的是值得交付。
- Targets: `src/storage/async_storage.rs` 的 `write_page_run` 默认实现；`src/storage/file_storage.rs` 的 `FileStorage::write_page_run`；`src/storage/buffer_pool.rs` 的 `flush_all` 与 `flush_all_except`。测试加在这两个文件已有的 `tests` 模块。`Cargo.toml` 只把 `libc = "0.2"` 从 `[dev-dependencies]` 移到 `[dependencies]`。
- Current behavior: `flush_all` 按 DashMap 顺序逐页 `write_page`。连续 64 页的产品写次数为 64。
- Required behavior: 收集后按 `PageId` 升序切连续段。长度大于或等于 2 的段一次 `pwritev`，长度为 1 的段仍是 `write_page`。空洞页字节不变。失败时整段和未尝试的页重新为脏，更早的成功段不进入这次失败集合。加密两页按 4124 字节记录读回。测量测试改为 `#[ignore]`。
- Preserve: `flush_pages` 的顺序、`evict_one`、`allocate_page`、检查点对 `FileStorage::sync` 的不调用、`injected io failure (write_page)`、M18 的四条。
- Forbidden: 修改 `src/wal/checkpoint.rs` 的步骤顺序。修改 `evict_one` 的选页。把提交页并进 `flush_all_except` 的同一次 `pwritev`。给数据文件加 `sync_all`。调用 `io_uring_setup`。
- First test: 改写入前，先跑 `cargo test --offline --lib storage::file_storage::tests -- --test-threads=1 --skip contiguous_dirty_page_write_measure`，退出码 0。再跑 `cargo test --offline --lib storage::buffer_pool::tests::failed_flush_then_evict_keeps_the_new_bytes -- --test-threads=1`，退出码 0。这是改前的 GREEN，不重跑测量测试。
- GREEN condition: 见 Verification 的顺序。新测试在实现前必须先观察到失败。
- Verification: 与 001 Cycle 的 Verification 同序，一次一个进程。第一条合并测试先只断言写次数和读回，不调用 `write_page_run`。明文 `pwritev` 通过后，再把失败集合断言加进同一个测试。加密记录的 `pwritev` 晚于加密测试的第一次失败。命令和通过条件以该 Cycle 为准。
- Stop when: 不把 `flush_pages` 并进同一次 `pwritev` 就不能保持目录提交点；既有测试要求 DashMap 的刷盘顺序；必须修改 `evict_one` 的选页、检查点步骤或 M18 的门控；或 `async_trait` 的默认 `write_page_run` 不能通过 `Arc<dyn AsyncStorage>` 调用。停止并回到计划。不要改成强制每个存储手写该方法，也不要把存储向下转型成 `FileStorage`。

`contiguous_flush_coalesces_a_run_and_keeps_the_gap` 的断言：页 3 和页 4 有可识别原内容且不为脏，页 0、1、2 和页 5 为脏；`flush_all` 后产品写次数小于 4；页 3 和页 4 的原字节还在；页 0、1、2、5 的新字节可读回。接着在容量 128 的池里把页 0 到 63 改脏，`flush_all` 的产品写次数小于 64，64 页读回一致。页 70 写成 `0x5A` 且不在脏集合里，刷盘后仍是 `0x5A`。另一次把页 0、1、2 和页 5 改脏，`flush_all_except` 排除页 5，写次数小于 3，页 0、1、2 的新字节可读回，页 5 的磁盘字节仍是旧内容。明文 `write_page_run` 成功写出页 0 和页 1 之后，武装 `DataWrite` 再写页 3 和页 4，错误文本含 `injected io failure (write_page)`，`failed_pages` 含 3 和 4、不含 0 和 1。字段访问和顺序见 001 的 Task Definition。

`contiguous_flush_encrypted_pages_roundtrip`：`open_with_key(..., Some("pw"))`，页 0 和页 1 改脏后 `flush_all`，产品写次数小于 2，同一密钥读回的明文一致。

`failed_run_restores_every_page_in_the_run`：仿照同文件的 `GatedWriteStorage` 写测试存储，测试模块直接读 `pages` 里的 `PageFrame`。不要新增产品侧的脏位查询方法。第一个案例不覆盖 `write_page_run`，`write_page` 第二次调用返回 `Io` 且不更新该页，池里只有页 0、1、2 为脏；`flush_all` 返回错误后这三页的 `dirty` 都为真，`write_in_flight` 都为假。第二个案例覆盖 `write_page_run`，第一次成功，第二次返回 `Io`；页 0 和页 1 为一段，页 3 和页 4 为下一段，页 6 单独为脏。错误返回后页 0 和页 1 不脏，页 3、4、6 为脏，这些页的 `write_in_flight` 都为假，页 6 的 `write_page` 没有被调用。

## T4: 比较 io_uring

本任务在 Iteration 002 展开之后执行。000 和 001 不执行。

- Requirement/Scenario: 「io_uring 比较不替换默认后端」的前三个场景。
- Depends on: T3 的执行记录已经是保留逐页写或值得交付。
- Targets: 只加测试 `storage::file_storage::tests::io_uring_flush_compare`。不改 `Cargo.toml` 的默认 features，不改非测试写入路径。
- Current behavior: 产品写入是 T3 留下的路径。规划时本机 `io_uring_setup(8)` 曾返回可用描述符；实施时以当次调用为准。
- Required behavior: 打印五次 `flush_ns` 与 `uring_ns`，以及 `setup=<ok 或错误号>`。页 0 到 63 的字节一致，页 64 为 0，页 70 为 `0x5A`。执行记录写明值得采用、保留或平台不适合。默认 features 不含 `io-uring`。
- Preserve: T3 的写入路径。测量测试在保留分支仍不忽略，在交付分支保持 `#[ignore]`。
- Forbidden: 修改默认 features。把比较放在没有 T3 记录的时候做。用「30–50%」作门槛。重跑 T1。
- First test: `io_uring_flush_compare`。实现测试前该名称不存在，编译失败，这是 RED。测试写完后命令退出码 0。
- GREEN condition: `cargo test --offline --release --lib storage::file_storage::tests::io_uring_flush_compare -- --test-threads=1 --nocapture` 退出码 0，且输出有五行和一个 `setup=`。
- Verification: 只跑这一条。`setup` 不是 ok 时，记录平台不适合，不比较中位数。`setup` 为 ok 时，五个 `uring_ns` 的中位数小于五个 `flush_ns` 的中位数，且字节断言已由测试通过，才记录值得采用；否则记录保留。001 已交付合并写且本结论不是值得采用时，再执行一次收尾全量。
- Stop when: 测试只有改默认 features 或改产品写入路径才能编译。停止并回到计划。

## T5: 默认关闭的 io_uring feature

本任务在 Iteration 003 展开之后执行。前几轮不执行。

- Requirement/Scenario: 「默认构建不启用 io_uring」。
- Depends on: T4 记录的是值得采用。
- Targets: `Cargo.toml` 的 feature `io-uring`，默认 features 不增加它。`src/storage/file_storage.rs` 的 `write_page` 与 `write_page_run` 在该 feature 启用时走 io_uring。
- Current behavior: 没有这个 feature。写入是 T3 的路径。
- Required behavior: 默认构建的 `flush_all` 与 T3 完成后相同。`cargo test --features io-uring` 下，连续两页 `flush_all` 后读回一致，武装 `DataWrite` 时错误文本仍含 `injected io failure (write_page)` 且没有字节落盘。提交在阻塞线程里等待完成。读仍是 `read_exact_at`。
- Preserve: 默认 features、注入文本、失败集合、检查点步骤、`evict_one`。
- Forbidden: 把 `io-uring` 写入 default features。依赖 `tokio-uring`。把读路径改成 io_uring。
- First test: 默认构建下的 `io_uring_feature_off_keeps_the_decided_write_path`。它调用 `flush_all` 并断言与 T3 的写次数合同一致：T3 是保留时 2 个连续脏页的产品写次数为 2；T3 是交付时小于 2。测试先写成会失败的断言，观察到 RED，再加 feature 门控，使默认构建通过。
- GREEN condition: 默认构建的该测试退出码 0。然后 `cargo test --offline --features io-uring --lib storage::file_storage::tests::io_uring_feature_on_write_roundtrip -- --test-threads=1` 在 feature 写路径完成前失败，完成后退出码 0。
- Verification: 上面两条按顺序各跑一次。然后跑一次收尾全量，使用默认 features，不附加 `--features io-uring`。
- Stop when: 不把 feature 放进默认 features 就不能让启用构建通过，或等待完成会改变 `injected io failure (write_page)` 的文本。停止并回到计划，不改默认 features。
