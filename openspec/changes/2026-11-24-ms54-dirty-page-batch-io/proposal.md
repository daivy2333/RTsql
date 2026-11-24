## Why

里程碑 MS54（`.agents/docs/tasks.md`）的结果是：连续脏页写入有测量；批量写值得做则交付，并保留失败重试、释放耐久性和恢复约定；其后才比较 io_uring；不值得或平台不适合则保留现有后端。

来源是 I026 与 I028。I026 的方向是连续页合并写和 `writev`。它写明历史「Checkpoint 5–10x」未经验证，要分开看任务提交、写和同步。I028 要等这次批量 I/O 有了结论再比较 Linux io_uring，并且不把替换默认跨平台后端当作前提。MS54 依赖 MS52。MS52 已归档，刷盘和驱逐保持当时的合同，M18 的四条不变量没有改写。

本次 goal 批准的是 MS54 已写明的范围。下面列出的默认假设由这次 goal 授权接受。

## What Changes

- 第一轮只测量连续脏页的逐页写入，并据此裁定批量写值得交付还是保留逐页写。不比较 io_uring，也不在非测试代码里合并写。
- 测量用测试构建里的计数，不用 `/proc/self/io`。规划时在本机读取 `/proc/self/io`，该文件不存在。
- 裁定不用「5–10 倍」。五次重复里，合并写的写次数每次都少于逐页刷盘，且合并写耗时的中位数更短，才算值得交付。否则保留逐页写。
- 值得交付时，只把 `flush_all` 和 `flush_all_except` 里页号连续的段合成一次 `pwritev`。`flush_pages`、`evict_one` 和单页 `write_page` 仍是逐页。失败重试、释放耐久性和 M18 的恢复不变量保持原句。
- 批量写的结论写下之后，再用同一负载比较 io_uring。默认构建仍走现有位置写。平台建环失败，或比较不满足同一形态的耗时条件，就保留当时的后端。值得采用时也只加默认关闭的 feature，不放进默认 features。

### 场景草图

1. 连续脏页的五次测量。前置：明文 `FileStorage`，已分配页 0 到 70；缓冲池容量 128，只把页 0 到 63 改脏；页 64 仍是分配后的零页，页 70 是事先写好的 `0x5A`。动作：先做一次不计入结果的 `flush_all`，再重复五次「改脏、计时 `flush_all`、再改脏、计时测试内的一次 `spawn_blocking` + `pwritev`」。结果：五行打印，每行有 `baseline_submissions=64`、`baseline_writes=64`、两边的纳秒，以及 `merged_submissions=1` 和 `merged_writes`；随后一行 `sync_calls=1`。页 0 到 63 的字节与当次写入一致，页 64 仍是零，页 70 仍是 `0x5A`。失败边界：任一次逐页写次数不是 64、任一侧纳秒为 0、读回不一致，或合并写期间产品写计数增加。测试失败，不得记成保留。
2. 保留逐页写。前置：场景 1 的测试通过，且五次合并写里至少有一次写次数不少于 64，或合并写耗时的中位数不小于逐页耗时的中位数。动作：把五行、两个中位数和「保留逐页写」写入执行记录。结果：`write_page` 仍是一页一次 `spawn_blocking` 和一次 `write_all_at`。`flush_all` 仍逐页调用它。失败边界：为了让中位数更小而重跑，或把未出现的「5–10 倍」写成采纳条件。
3. 交付连续段合并写。前置：场景 1 通过，五次 `merged_writes` 都小于 64，且合并写耗时的中位数更短。动作：`flush_all` 把页号连续的脏页合成一次 `pwritev`。结果：64 个连续脏页刷盘后的产品写次数小于 64，读回字节一致；页号有空洞时，空洞页的原字节还在。场景 1 的测量测试改为 `#[ignore]`，忽略理由写明打印已在执行记录里。失败边界：为了减少写次数而改写空洞页，或让 `flush_pages`、`evict_one` 走合并写。
4. 刷盘失败后脏页还在。前置：一次 `DataWrite` 注入已经武装，下一次数据页写入会在落盘前失败。动作：`flush_all`。结果：错误文本仍含 `injected io failure (write_page)`，池里仍有脏页；下一次成功刷盘把这些页写出。失败边界：失败的页被标成不脏，或错误文本改成别的句子。
5. 目录提交点的顺序。前置：`Catalog::flush_commit_point` 先 `flush_all_except` 提交页，再按给定顺序 `flush_pages`。动作：交付合并写。结果：提交页仍由 `flush_pages` 逐页、按调用方给出的顺序写入；它们不和前面的页合成同一次 `pwritev`。失败边界：把提交页排序后并进前一次刷盘。
6. 释放仍看锚点页的失败集合。前置：一次释放的锚点页写失败后还在失败集合里。动作：提升可分配页。结果：这次释放不提升。锚点页之后被成功重写并离开失败集合，随后的 checkpoint 可以提升。失败边界：整段合并写失败时把已经成功的另一段锚点页也留在失败集合里，或失败却提升。
7. 检查点不增加数据文件同步。前置：`CheckpointManager::checkpoint` 在 `flush_all` 之后调用的是 WAL `fsync`，没有 `FileStorage::sync`。动作：交付合并写或保留逐页写。结果：这条调用顺序不变。测量里单独计时的那一次 `sync` 只描述 `FileStorage::sync`，不表示检查点调用了它。失败边界：为了批量写给数据文件加上 `sync_all`。
8. 恢复不变量对照。前置：M18 的四条仍是：驱逐不按树序；`redo_count > 0` 不消费磁盘索引；`redo_count == 0` 信任磁盘树；数据页按 WAL 位置重放。动作：只改多页刷盘的写出方式。结果：`evict_one` 仍是逐页 `write_page`；检查点仍要等 `flush_all` 成功才写位点。失败边界：合并写必须改这四条才能做完。停止，不把计划标成可执行。
9. io_uring 比较。前置：批量写的保留或交付已经写入执行记录。动作：用场景 1 的 64 个连续页，对当时的 `flush_all` 和测试内的一次 io_uring 写入各计时五次。结果：打印两边的中位数；默认 features 仍不含 io_uring。`io_uring_setup` 失败则记下平台不适合并保留当时的后端。建环成功时，只有 io_uring 中位数更短且字节一致才记下值得采用。失败边界：在批量写结论之前做这次比较，或把 io_uring 放进默认 features。
10. 非默认 io_uring。前置：场景 9 记下值得采用。动作：增加默认关闭的 `io-uring` feature。启用时，页写入在阻塞线程里提交 io_uring 并等到完成；不启用时与场景 9 之前的写入路径相同。结果：`Database::open` 的默认构建仍使用位置写。失败边界：只能把 io_uring 放进默认 features 才能做完。停止并回报。

### 本次 goal 授权接受的默认假设

1. 连续脏页负载是测试构造的页 0 到 63，缓冲池容量 128，因此测量时 `evict_one` 不运行。不测量一条 SQL 检查点里实际脏页的页号混合。有空洞的页不会比一整段连续页更适合合并写；最好情况不值得，就保留逐页写。
2. 写次数是测试构建里的计数：逐页路径计 `write_page_blocking` 里成功的 `write_all_at`，合并路径计测试里 `pwritev` 的调用次数。不读 `/proc/self/io`。任务提交数：逐页路径是 64 次 `spawn_blocking`，合并路径是测试里的 1 次。
3. 重复五次，事先再做一次不计时的 `flush_all`。耗时取中位数，也就是排序后的第三个数。不重跑，不丢掉某一次。
4. 同步只单独调用一次 `FileStorage::sync` 并打印。它不进入值得与否的比较。检查点当前不调用它，交付时也不加。
5. 页 64 和页 70 用来确认合并写的长度和偏移没有盖到段外。页 70 的哨兵字节是 `0x5A`。
6. 加密库的磁盘记录长 4124 字节。测量用无密钥的明文打开。交付时的合并写按当时的 `record_size` 组向量，加密页先逐页加密再写入。
7. io_uring 的比较与采纳使用和批量写相同的五次中位数，不使用历史「30–50%」。采用方式是默认关闭的 Cargo feature，写入在阻塞线程里等完成。不引入 `tokio-uring`。

### 已经选定的行为

- 第一轮的稳定结果就是批量写的去留。io_uring 不在这一轮。
- 产品对外的 SQL 结果、错误文本 `injected io failure (write_page)`、检查点九步的调用顺序和页格式保持现状。变更的是多页刷盘的写次数，而且只有裁定为值得交付时才改。
- 行为规格增量写在 `specs/storage-io-optimization/spec.md`。`.openspec.yaml` 不使用 `skip_specs`。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `storage-io-optimization`：连续脏页的测量和去留；值得交付时，`flush_all` 与 `flush_all_except` 的连续段可以一次 `pwritev`。读路径、`flush_pages`、`evict_one` 和单页 `write_page` 仍是一次位置读写。默认构建的后端不被 io_uring 替换。

## Impact

- 代码：第一轮只在 `src/storage/file_storage.rs` 增加测试构建计数和测量测试。值得交付时再改 `src/storage/async_storage.rs`、`src/storage/file_storage.rs` 和 `src/storage/buffer_pool.rs` 的多页刷盘。`evict_one`、`flush_pages`、`src/wal/checkpoint.rs` 的步骤顺序不改。
- 引用：MS54、I026、I028、M18、M19。M18 在设计里对照，不改 `project-model.md`。M19 要求测试命令带 `--test-threads=1`。全量分批只在后文的收尾条件成立时跑一次，不作为第一轮的义务。
- 测试：第一轮一条 release 库测试。交付轮再跑失败重试、释放耐久性、检查点和目录提交点相关的既有测试。
- 数据：无格式变更，无迁移。明文页 4096 字节，加密记录 4124 字节，文件头 64 字节，均保持。

## 回滚

没有格式迁移。去掉测试构建计数和测量测试，就没有这次打印。若已经交付合并写，恢复 `flush_all` 与 `flush_all_except` 的逐页 `write_page` 即回到现在的写路径；已有库文件不用迁移。若增加了默认关闭的 `io-uring` feature，去掉该 feature 后默认构建与去掉之前的写入路径相同。

## Gate 1

PASS。需求、场景和范围来自 MS54 的 Outcome、Scope、Non-goals、Split signals 和 Consumes，以及上面由本次 goal 授权接受的默认假设。这是批准，不是跳过。没有 SKIPPED 项。
