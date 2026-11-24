## MODIFIED Requirements

### Requirement: 页 I/O 使用位置参数调用

`FileStorage` 的页读 SHALL 继续使用 `FileExt::read_exact_at`。单页 `write_page`、`evict_one` 和 `flush_pages` SHALL 继续使用 `FileExt::write_all_at`。这些路径每次页 I/O 仍是一次位置读或一次位置写，SHALL NOT 调用 `lseek`。

测量裁定为保留逐页写时，`flush_all` 与 `flush_all_except` SHALL 继续对每一脏页调用一次 `write_page`。裁定为值得交付时，这两个入口里一段页号连续的脏页 SHALL 用一次 `pwritev` 写出，短写时继续调用直到这一段的全部字节写完或返回错误；页号不连续的页 SHALL NOT 被同一次 `pwritev` 覆盖。读路径的错误类型与语义（含短读报 `UnexpectedEof`）SHALL 与改造前一致。

#### Scenario: 页内容读写往返等价

- **GIVEN** 已打开的 `FileStorage`，已分配至少 3 页并各自写入可识别的不同内容
- **WHEN** 依次 `read_page` 读回每页
- **THEN** 每页内容与写入逐字节一致，返回的 `Page.id` 正确

#### Scenario: 越界读取报错语义不变

- **GIVEN** 文件长度为 N 页
- **WHEN** `read_page` 请求页 id 大于或等于 N
- **THEN** 返回 `Err`，不 panic，不死循环

#### Scenario: 并发冷读不同页无错读

- **GIVEN** 已写入至少 16 页可识别内容的 `FileStorage`，读取直接走存储层
- **WHEN** 16 个任务并发多次 `read_page` 各自不同的页并校验内容
- **THEN** 每任务每次读到的内容都与该页写入的内容一致

#### Scenario: 并发读写混合不串页

- **GIVEN** 多页 `FileStorage`，页 X 与页 Y 各自写入可识别内容
- **WHEN** 任务 A 反复 `write_page` 页 X，同时任务 B 并发多次 `read_page` 页 Y
- **THEN** B 每次读到的都是页 Y 的内容

#### Scenario: 保留逐页写时刷盘仍是一页一次位置写

- **GIVEN** 测量裁定为保留逐页写，缓冲池里有一段连续脏页
- **WHEN** `flush_all`
- **THEN** 每一脏页仍经过一次 `write_page` 的 `write_all_at`
- **AND** 读回字节与刷出前的页内容一致

#### Scenario: 交付合并写时连续段不覆盖段外页

- **GIVEN** 测量裁定为值得交付，脏页含页号连续的一段，且相邻页号之间有未纳入该段的页
- **WHEN** `flush_all` 或 `flush_all_except`
- **THEN** 连续段的字节被写出
- **AND** 未纳入该段的页，其原字节保持不变

#### Scenario: syscall 序列位置参数化

- **GIVEN** 一次触发若干页读的运行，以及一次连续脏页刷盘
- **WHEN** 查看页读与页写使用的调用
- **THEN** 页读仍是 `read_exact_at`，不调用 `lseek`
- **AND** 单页写仍是 `write_all_at`；连续段在值得交付时是 `pwritev`，同样不调用 `lseek`

### Requirement: 零接口零格式变更

页布局、文件头和单页磁盘记录长度 SHALL NOT 改变。明文记录 4096 字节，加密记录 4124 字节，文件头 64 字节。查询结果 SHALL NOT 因本次写路径变化而改变。

`AsyncStorage` 既有方法的签名 SHALL NOT 改变。测量裁定为保留逐页写时，SHALL NOT 新增必须实现的写入方法。裁定为值得交付时，可以增加带默认实现的 `write_page_run`：默认实现按页调用 `write_page`；`FileStorage` 对连续段改为 `pwritev`。`MemoryStorage` 与测试用的存储替身未覆盖该方法时，行为与逐页 `write_page` 相同。

#### Scenario: 既有单页写入签名仍可调用

- **GIVEN** 调用方持有 `AsyncStorage`
- **WHEN** 调用既有的 `read_page`、`write_page`、`allocate_page`、`free_page` 或 `sync`
- **THEN** 这些方法的参数和返回类型与变更前一致

#### Scenario: 磁盘格式不变

- **GIVEN** 变更前写成的明文库或加密库
- **WHEN** 用变更后的代码打开并读回已提交的页
- **THEN** 页字节按原记录长度解析，不需要迁移

#### Scenario: 既有行为全量回归

- **GIVEN** 非测试代码的页写入路径已经改动
- **WHEN** 按 M19 做一次分批全量，每批 `cargo test … -- --test-threads=1`，批间顺序执行
- **THEN** 每一批的 failed 计数为 0
- **AND** 非测试代码的页写入路径没有改动时，不把这次全量当作测量轮的通过条件

## ADDED Requirements

### Requirement: 连续脏页写入的测量

系统 SHALL 在测试构建中对连续脏页刷盘打印写次数和耗时。打印本身 SHALL NOT 裁定值得交付或保留。

负载 SHALL 为：无密钥打开的明文文件；`allocate_page` 得到页 0 到 70；缓冲池容量 128；只将页 0 到 63 放入池中并改脏。页 70 在计时前用 `write_page` 写成每个字节都是 `0x5A`，并且不放入这次刷盘的脏页集合。页 64 保持分配后的零页，也不放入脏页集合。

计时前 SHALL 先执行一次不打印的 `flush_all`，并确认这次产品写次数为 64。随后重复五次。每一次先把页 0 到 63 改脏并计时 `flush_all`，再把这 64 页改脏，并计时一次 `spawn_blocking` 中的 `pwritev`。`pwritev` 的偏移是 `HEADER_SIZE`，长度是 64 乘 4096，向量只覆盖页 0 到 63。五次的产品写计数在合并写期间 SHALL 保持不变。

每次 `flush_all` 的产品提交次数和产品写次数都 SHALL 为 64。产品提交次数是 `write_page` 进入 `spawn_blocking` 的次数。产品写次数是 `write_page_blocking` 里 `write_all_at` 成功返回的次数。合并写的提交次数 SHALL 为 1。合并写的写次数是该次 `pwritev` 的调用次数，短写继续调用时按实际次数打印。两侧耗时都 SHALL 大于 0。读回的页 0 到 63 SHALL 与当次写出的字节一致。页 64 SHALL 仍全是 0。页 70 SHALL 仍全是 `0x5A`。

五次都打印一行，字段名为 `repeat`、`baseline_submissions`、`baseline_writes`、`baseline_ns`、`merged_submissions`、`merged_writes`、`merged_ns`。五次之后再调用一次 `FileStorage::sync`，打印 `sync_calls` 与 `sync_ns`。`sync_calls` SHALL 为 1，`sync_ns` SHALL 大于 0。这次 `sync` SHALL NOT 进入值得与否的比较。

计数只在测试构建中存在。非测试构建的 `write_page`、`write_page_blocking` 和 `sync` SHALL NOT 因计数而改变成功、失败或写出的字节。

#### Scenario: 五次重复打印字段且字节一致

- **GIVEN** 上述负载，命令为 `cargo test --offline --release --lib storage::file_storage::tests::contiguous_dirty_page_write_measure -- --test-threads=1 --nocapture`
- **WHEN** 该测试运行一次
- **THEN** 输出有五行走完上列字段，以及一行 `sync_calls=1`
- **AND** 页 0 到 63 与当次写入一致，页 64 全是 0，页 70 全是 `0x5A`
- **AND** 输出中没有「值得交付」或「保留逐页写」

#### Scenario: 写次数或耗时不成立时测量无效

- **GIVEN** 同一次测试
- **WHEN** 任一次逐页产品写次数或提交次数不是 64，任一侧耗时为 0，读回不一致，或合并写期间产品写次数增加
- **THEN** 测试失败
- **AND** 该次输出不得被记成保留逐页写，也不得被记成值得交付

### Requirement: 批量写去留由这五次打印裁定

去留 SHALL 只使用上述五次打印。中位数是五个纳秒值升序后的第三个。值得交付，当且仅当五次 `merged_writes` 都小于 64，且 `merged_ns` 的中位数小于 `baseline_ns` 的中位数。否则保留逐页写。

SHALL NOT 使用「5–10 倍」或任何未在这五次打印中出现的倍数。SHALL NOT 重跑这五次来更换结果。`sync_ns` SHALL NOT 单独构成值得交付或保留的理由。

库 SHALL NOT 在 `flush_all` 时读取这些打印来选择路径。裁定是这次变更的实施决定，不是运行时分支。

#### Scenario: 五次写次数都减少且合并写中位数更短

- **GIVEN** 五次 `merged_writes` 都小于 64，且 `merged_ns` 的中位数小于 `baseline_ns` 的中位数
- **WHEN** 按本 Requirement 裁定
- **THEN** 记录为值得交付
- **AND** 下一轮只交付连续段合并写

#### Scenario: 写次数未减少或合并写中位数不更短

- **GIVEN** 至少一次 `merged_writes` 大于或等于 64，或 `merged_ns` 的中位数大于或等于 `baseline_ns` 的中位数
- **WHEN** 按本 Requirement 裁定
- **THEN** 记录为保留逐页写
- **AND** 非测试构建的页写入仍是一页一次 `write_all_at`

### Requirement: 值得交付时只合并连续脏页段

本 Requirement 只在裁定为值得交付时实施。裁定为保留逐页写时，本 Requirement 的场景不实施。

`flush_all` 与 `flush_all_except` SHALL 在取走脏页之后按 `PageId` 升序排列，再把页号差为 1 的最大段分开。长度为 1 的段 SHALL 调用 `write_page`。长度大于或等于 2 的段 SHALL 调用 `write_page_run`。`FileStorage::write_page_run` SHALL 在一次 `spawn_blocking` 里对这一段做 `pwritev`：明文按 4096 字节一组，加密按 4124 字节一组，组内顺序与页号升序相同，文件偏移为第一页的现有偏移。短写 SHALL 继续 `pwritev` 直到这一段全部写完。`pwritev` 返回错误，或在尚未写完时返回 0，SHALL 使这次调用返回错误，并且这一段的每一页都进入失败集合。

页号不连续时，`FileStorage::write_page_run` SHALL 在任何字节落盘前返回 `StorageError::Internal`，错误文本含 `non-contiguous page run`。调用方 SHALL 把这次调用涉及的页和尚未尝试的页交还脏位。

`flush_pages` SHALL 保持调用方给出的顺序，逐页 `write_page`，SHALL NOT 排序，SHALL NOT 调用 `write_page_run`。`evict_one` SHALL 保持逐页 `write_page`。`allocate_page` 的清零写入 SHALL NOT 改成 `write_page_run`。

`DataWrite` 注入点在 `write_page_run` 中 SHALL 于任何字节落盘前消费一次。命中时返回的错误文本 SHALL 含 `injected io failure (write_page)`，这一段的每一页进入失败集合，并且不调用 `pwritev`。

只读存储上的 `write_page_run` SHALL 返回与 `write_page` 相同的只读拒绝文本。

检查点 SHALL NOT 因此调用 `FileStorage::sync` 或对数据文件 `sync_all`。

#### Scenario: 一段连续脏页的写次数少于页数

- **GIVEN** 裁定为值得交付，缓冲池容量足以容纳 64 个脏页，页 0 到 63 都为脏，且没有空洞
- **WHEN** `flush_all`
- **THEN** 产品写次数小于 64
- **AND** 读回的 64 页与刷出前逐字节一致

#### Scenario: 空洞页保持原字节

- **GIVEN** 页 0、1、2 和页 5 为脏，页 3 与页 4 有可识别的原内容且不为脏
- **WHEN** `flush_all`
- **THEN** 页 0、1、2 和页 5 的新内容可读回
- **AND** 页 3 与页 4 的原内容不变

#### Scenario: 注入失败不写出这一段

- **GIVEN** `DataWrite` 已武装，下一次合并写或逐页写会在落盘前失败
- **WHEN** `flush_all`
- **THEN** 返回的错误文本含 `injected io failure (write_page)`
- **AND** 缓冲池仍有脏页
- **AND** 随后一次未武装的 `flush_all` 成功后，这些页不再为脏，内容已写入存储

#### Scenario: 提交页仍按给定顺序逐页写

- **GIVEN** 调用方先 `flush_all_except` 排除一组提交页，再 `flush_pages` 按给定顺序写入这组页
- **WHEN** 两次调用都返回
- **THEN** 提交页的写入发生在排除集合的刷盘返回之后
- **AND** `flush_pages` 对每一提交页各调用一次 `write_page`

#### Scenario: 加密连续页按记录长度读回

- **GIVEN** `FileStorage::open_with_key` 使用 `Some("pw")` 创建的加密库，页 0 和页 1 在缓冲池中为脏
- **WHEN** `flush_all` 后用同一密钥读回
- **THEN** 两页明文与刷出前一致
- **AND** 这次产品写次数小于 2

### Requirement: 失败重试、释放耐久性与恢复约定保持

无论裁定为保留还是值得交付，下列约定 SHALL 保持。

一次刷盘中某次页写失败时，该次调用涉及且尚未成功结束的页，以及这次刷盘尚未尝试的页，SHALL 重新标为脏。已经成功结束的更早一段 SHALL 保持已写出的结果，SHALL NOT 被这次失败重新放进失败集合。`write_in_flight` SHALL 在该次调用返回后清除。写盘进行中的页 SHALL NOT 被 `evict_one` 当作干净页移出池。

释放提升 SHALL 继续以锚点页是否仍在失败集合中为准。检查点 SHALL 继续在 `flush_all` 之前取样释放完成编号，成功则提升，失败则丢弃该样本。

M18 的四条 SHALL 保持：驱逐不按树序；`redo_count > 0` 时恢复不消费磁盘索引，并从数据页重建主键索引；`redo_count == 0` 时磁盘树可信；数据页仍按 WAL 记录中的位置重放。本变更 SHALL NOT 修改 `evict_one` 的选页规则，SHALL NOT 修改 `rebuild_pk_indexes` 的门控。

#### Scenario: 合并写失败时整段和未尝试的页仍为脏

- **GIVEN** 裁定为值得交付，一次 `flush_all` 已经成功写出更早的一段，当前段的写入返回错误
- **WHEN** 错误返回到调用方
- **THEN** 当前段的每一页和尚未尝试的页都为脏
- **AND** 更早一段里已经成功的页不因这次错误进入失败集合

#### Scenario: 锚点页仍失败时释放不提升

- **GIVEN** 一次释放的锚点页仍在失败集合中
- **WHEN** 触发可分配提升
- **THEN** 该释放不提升

#### Scenario: 不洁关闭仍不消费磁盘索引

- **GIVEN** WAL 尚有未检查点的已提交记录，进程未调用 `close` 即丢弃句柄
- **WHEN** 再次 `Database::open`
- **THEN** 恢复仍在 `redo_count > 0` 时重建主键索引，而不是把磁盘索引当作查询依据

### Requirement: io_uring 比较不替换默认后端

本 Requirement 在批量写去留已经写入执行记录之后才实施。第一轮测量 SHALL NOT 调用 `io_uring_setup`。

比较负载与连续脏页测量相同：页 0 到 63，五次。一侧是当时的 `flush_all`，另一侧是测试内一次 io_uring 写入，写入在提交后等待完成，字节覆盖页 0 到 63，不覆盖页 64 和页 70。中位数仍是五个纳秒值升序后的第三个。

`io_uring_setup` 失败时，SHALL 记录平台不适合，并保留当时的后端。建环成功时，只有 io_uring 一侧的中位数更短，且页 0 到 63、页 64、页 70 的字节条件与测量场景相同，才记录为值得采用。否则保留当时的后端。SHALL NOT 使用「30–50%」。

默认 Cargo features SHALL NOT 因此加入 io_uring。值得采用时，SHALL 只增加默认关闭的 `io-uring` feature。启用该 feature 时，`write_page` 与连续段写入在阻塞线程中提交 io_uring 并等待完成后再返回；失败点仍在提交前消费，错误文本仍含 `injected io failure (write_page)`。不启用时，写入路径与比较之前的裁定结果相同。读路径 SHALL NOT 改为 io_uring。SHALL NOT 依赖 `tokio-uring`。

若只能把 io_uring 放入默认 features 才能完成采用，SHALL 停止并回报，SHALL NOT 修改默认 features。

#### Scenario: 批量写结论之前不比较 io_uring

- **GIVEN** 连续脏页测量测试尚未运行，或去留尚未写入执行记录
- **WHEN** 执行第一轮
- **THEN** 该轮不调用 `io_uring_setup`，也不修改默认 features

#### Scenario: 建环失败则保留当时的后端

- **GIVEN** 批量写去留已经记录，且 `io_uring_setup` 返回错误
- **WHEN** 按本 Requirement 比较
- **THEN** 记录平台不适合
- **AND** `FileStorage` 的默认写入路径保持比较前的实现

#### Scenario: 默认构建不启用 io_uring

- **GIVEN** 比较记录为值得采用，并且已经增加 `io-uring` feature
- **WHEN** 使用未启用该 feature 的默认构建打开数据库并 `flush_all`
- **THEN** 页写入仍走比较前的位置写或 `pwritev`
- **AND** 默认 features 列表不含 `io-uring`
