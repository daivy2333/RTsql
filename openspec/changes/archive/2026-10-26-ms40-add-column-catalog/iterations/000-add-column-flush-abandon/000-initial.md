# Iteration 000 / Cycle 000: 列数刷盘期间放弃加列

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `ddl-catalog-transactional` 新增的「被放弃的 ADD COLUMN 不留下已落盘的列数上调」；`ddl-layout-publication`「失败的 ADD COLUMN 目录面被回退」保持不刷盘。实现规则是 `design.md` D1，本文件下面写全，执行时不回读 design。
- Excluded scope: T2（ISS39）、T3（全量回归）、Join、ISS36、其他目录面改动、Issue 状态、尚未纳入本 Iteration 的新缺陷。

**Objective**

`ALTER TABLE … ADD COLUMN` 在列数刷盘已经成功、成功结果还没交回调用方时被放弃后，目录列数和可见列定义回到语句执行前。不 `close`、不 checkpoint，直接崩溃再打开，新列仍然不可见。刷盘返回错误时仍走现有补偿，补偿本身不刷盘。成功返回的加列跨 `close` 仍在。

**Investigation Facts**

- Starting point: 工作区 HEAD `e9cb9d353c44acacc03be61ff8bf2814978a9f45`，相对该提交有未提交产品改动。本 Cycle 的符号位置按当前工作区源码，不按 Issue 登记时的行号。变更前没有针对本缺陷的失败用例。不跑全量。
- Current-State Evidence:
  - `src/executor/alter_table.rs` 的 `AddColumn` 臂（52–78 行）直接 `await` `TableManager::alter_add_column`。没有隐式事务守卫。成功时返回 `AffectedRows(0)`。`Database::path`（`src/database.rs` 309–311 行）是现有暂停点使用的库路径。`FileStorage` 不保存这份路径。`table_manager` 的类型是 `Arc<TableManager>`（`src/database.rs` 112 行）。
  - `TableManager::alter_add_column`（961–1035 行）持有 `self.tables` 写锁直到函数结束。`insert_column_row` 失败或 `update_table_column_count_ddl` 返回 `Err` 时调用 `compensate_column_add`。成功则换入新 `TableMeta`。没有任何放弃路径上的补偿。
  - `compensate_column_add`（1049–1059 行）先 `restore_table_column_count_no_flush`，再 `remove_column_row_at`。两次调用各自加锁再放开。不刷盘。
  - `Catalog::update_table_column_count_ddl`（`src/storage/catalog.rs` 427–448 行）先改缓冲池中的 `column_count`，再 `flush_commit_point`（270–275 行）：`flush_all_except` 之后 `flush_pages`。它在 428 行持有 `Catalog.lock` 跨过这次刷盘。`FileStorage::write_page`（452–483 行）在 `spawn_blocking`（469 行）里写盘。语句 future 在这次 `await` 上被丢弃时，阻塞写可以继续完成，`write_in_flight` 的收尾和 `compensate_column_add` 都不会跑。
  - `Catalog.lock` 是 `tokio::sync::Mutex`（142 行）。`scan_tables`（307 行）、`scan_columns`（317 行）、`restore_table_column_count_no_flush`（458 行）、`remove_column_row_at`（485 行）各自取得再释放。`update_field_in_chain`（987–993 行）返回 `Result<Option<PageId>>`，即新字节落到的那一页。`delete_from_chain`（904–911 行）返回 `Result<Vec<PageId>>`，即它改过的页；`remove_column_row_at` 在 496 行用 `map(|_| ())` 丢掉这份列表。`BufferPool::flush_pages`（`src/storage/buffer_pool.rs` 316 行）按给定顺序写这些页。
  - `open_or_init`（602–606 行）按目录行的 `column_count` 截断列行。`tests/ddl_layout_publication_test.rs::catalog_shape`（107–133 行）不走这条截断，它直接 `scan_columns` 再 `scan_tables`。
  - `self.tables` 是 `tokio::sync::RwLock`（463 行，导入在文件第 5 行）。
  - 生产代码里 `alter_add_column` 只有 `AlterTableExecutor` 这一处调用。
  - 既有用例：`tests/ddl_layout_publication_test.rs` 的 `failed_add_column_is_compensated`（`arm_io_failure` 后 `close` 再打开，新列不在）、`successful_add_column_still_adds_the_column`（`close` 再打开，新列在）。同文件的 `crash`（87–92 行）是停掉 WAL 并丢掉库句柄，不 `close`。`park_at_commit_window`（155–178 行）用 `select` 等 `reached()`，语句先结束则夹具失败。
- Code and Critical Path: 调用链是 `Database::execute_sql` → `AlterTableExecutor::next` 的 `AddColumn` 臂 → `alter_add_column` → `insert_column_row` → `update_table_column_count_ddl` → `flush_commit_point` → `write_page`。放弃窗口是列数刷盘返回 `Ok` 之后、活布局换入之前。错误窗口是 `update_table_column_count_ddl` 返回 `Err`。删列发布走同一个 `update_table_column_count_ddl`，本 Cycle 不改那次调用的刷盘顺序，也不在那次调用上放暂停点。

**Implementation Guidance**

先让新用例在现有调用链上观察到 RED，再把尝试移进不会随语句 future 丢弃的任务。暂停点在 RED 和 GREEN 都位于「刷盘返回 `Ok` 之后、决定提交之前」。未布防时立即返回，现有用例不武装它。

**Task Definitions**

### T1: 列数刷盘期间被放弃的加列不留下已落盘的列数

- Requirement/Scenario: `ddl-catalog-transactional`「被放弃的 ADD COLUMN 不留下已落盘的列数上调」/「列数刷盘期间被放弃的 ADD COLUMN 重开后不可见」。同时保持 `ddl-layout-publication`「失败的 ADD COLUMN 目录面被回退」。
- Depends on: None
- Targets:
  - `src/executor/alter_table.rs` `AlterTableExecutor::next` 的 `AddColumn` 臂
  - `src/storage/data/table_manager.rs` `TableManager::alter_add_column`
  - `src/storage/data/table_manager.rs` `TableManager::compensate_column_add`
  - `src/storage/catalog.rs`：与 `restore_table_column_count_no_flush` 分开的回退入口。它在同一次 `Catalog.lock` 里恢复列数、移除列行，并按 Required behavior 的顺序调用 `flush_pages`。不改变 `restore_table_column_count_no_flush` 和 `remove_column_row_at` 的不刷盘行为。
  - 新的默认不生效暂停点，只从加列路径、在 `update_table_column_count_ddl` 返回 `Ok` 之后调用。模块放在 `src/storage/`，形态与 `src/executor/ddl_commit_window.rs` 相同：按库路径一次消费，默认不生效，布防函数对集成测试可见。不携带 hash、指纹、revision、run-id 或时间证明。
  - `tests/ddl_layout_publication_test.rs` 新用例 `abandoned_add_column_during_count_flush_is_not_durable`
- Current behavior: 补偿只在 `insert_column_row` 或 `update_table_column_count_ddl` 返回 `Err` 时运行。语句 future 在列数刷盘期间被丢弃时不运行补偿。刷盘使用 `spawn_blocking`，丢弃 future 之后写入仍可能落盘。重开后目录列数是上调后的值，新列可见。
- Required behavior:
  - `AddColumn` 臂把这次 `alter_add_column` 放进 `tokio::spawn` 的任务。任务持有克隆的 `Arc<TableManager>`、语句参数的拥有副本，以及 `database.path()` 的拥有副本。语句 future 只等待 oneshot 的 `Result`。丢弃语句 future 会关闭接收端，任务继续执行，不随该 future 取消。
  - 写锁在任务内取得，直到下面的决定做完才释放。丢弃语句 future 不得在决定之前放开这把锁。锁顺序保持今天的顺序：先 `tables` 写锁，再 `Catalog.lock`。
  - `update_table_column_count_ddl` 返回 `Ok` 之后、决定之前，调用暂停点（未布防则立即返回）。暂停点用任务持有的库路径，按路径一次消费。然后看 oneshot 发送端的 `is_closed()`：
    - 接收端未关闭且刷盘 `Ok`：换入活布局，再发送 `Ok`。这是提交。发送若因接收端此时已关闭而失败，不再回退；语句保持已提交。此后再丢弃 future，同样不回退。
    - 接收端未关闭且刷盘 `Err`：调用现有 `compensate_column_add`（不刷盘），把原始错误交回。不得把补偿失败变成新的错误。
    - 接收端已关闭且刷盘 `Ok`：不换入活布局，不交回 `Ok`。在同一次 `Catalog.lock` 持有中恢复列数、移除本次列行，然后 `flush_pages` 只写列数页，再 `flush_pages` 写列行被改过的页。两次 `flush_pages` 都返回之后才释放这把锁。列数页是 `update_field_in_chain` 返回的 `PageId`。列行页是 `delete_from_chain` 返回的列表。不写死 `TABLES_PAGE_ID` 或 `COLUMNS_PAGE_ID`。禁止用 `flush_commit_point`。禁止调用 `compensate_column_add` 再补一次刷盘来代替这次回退。回退的 `flush_pages` 返回 `Err` 时：不换入活布局，不发送 `Ok`，不重试，不再调用 `compensate_column_add`。
    - 接收端已关闭且刷盘 `Err`：现有 `compensate_column_add`（不刷盘）。
  - 超时到期只是丢弃该 future，不另做一条分支。
  - 成功路径的 SQL 结果仍是 `AffectedRows(0)`。错误路径的错误仍是刷盘返回的那个错误。
  - 删列对 `update_table_column_count_ddl` 的调用、`publish_column_narrowing`、窄化令牌、WAL 记录保持现状。
- Preserve:
  - `failed_add_column_is_compensated`：注入写失败后语句失败，补偿不刷盘，`close` 再打开后新列不在、列数是语句前的值。
  - `successful_add_column_still_adds_the_column`：成功加列在 `close` 再打开后仍在，列数增加。
  - `alter_add_column_survives_crash`（`tests/ddl_crash_recovery_test.rs`）：成功加列后立即崩溃，重开可见。
  - 同表第二条删列仍被点名拒绝。本 Cycle 不改该路径。
- Forbidden:
  - 修改 `write_page`、`flush_pages`、`flush_all_except` 的通用取消行为。
  - 失败路径的补偿改为自己刷盘。
  - 在删列的列数刷盘上武装或调用本暂停点。
  - 把 `ADD COLUMN` 放进窄化令牌，或让它在令牌期间返回拒绝。那是 T2 的等待，不是本 Cycle。
  - 改 `publish_column_narrowing` 的布局来源。
  - 新增公共 API、错误变体、磁盘格式字段。
  - 用 hash、指纹、revision、run-id、session id 或时间证明标识这次运行。
  - 跑全量，或一次无目标过滤的 `cargo test`。
  - 修本 Iteration 未纳入的新缺陷。与本变更故障域相同的留给后续 Iteration；其他故障域仍留在 MS40，不新建里程碑。不预建那些 Iteration。
- First test: `tests/ddl_layout_publication_test.rs::abandoned_add_column_during_count_flush_is_not_durable`。用例自带 `TempDir`。自己建两列表并插入一行，不复用同文件 `setup` 的三列表，也不用 `COLUMNS_BEFORE`。布防暂停点，用同文件 `park_at_commit_window` 的 `select` 方式把 `ALTER TABLE t ADD COLUMN m INT` 停在暂停点（锚点是暂停点的 `reached`，语句提前结束则夹具失败）。然后丢弃语句 future，放行暂停点。放行后用同文件 `catalog_shape` 轮询，直到 `column_count` 回到语句前且列名不含 `m`，或满 10 秒。10 秒是这次产品等待的上限：到点也继续往下崩溃重开，不把到点本身当成夹具失败。然后用同文件 `crash`（停 WAL、丢句柄、不 `close`）再 `Database::open`。断言重新打开后的 `catalog_shape` 列数等于语句前，列名不含 `m`。轮询必须走 `catalog_shape`。不要用 `SELECT` 或 `get_table` 当完成信号：放弃路径不换入活布局，活布局在回退刷盘之前就已经是语句前的列。`catalog_shape` 与回退抢同一把 `Catalog.lock`，所以它看见恢复后的列数时，两次 `flush_pages` 已经返回。
  - 变更前命令：`cargo test --test ddl_layout_publication_test abandoned_add_column_during_count_flush_is_not_durable -- --test-threads=1`
  - RED：暂停点先加在当前调用链上（仍在调用方 future 里，位于刷盘返回 `Ok` 之后、换入活布局之前），不改变未布防行为。此时丢弃 future 不会补偿。崩溃重开后列 `m` 可见或列数比语句前大 1，断言失败。编译失败不算这次 RED。未观察到这个行为失败，不得开始下面的 GREEN 修改。
- GREEN condition: 同一命令退出码 0。崩溃重开后的列数等于语句前，列定义不含 `m`。处理结束前的目录轮询也能看到列数回到语句前。
- Verification:
  1. `cargo test --test ddl_layout_publication_test -- --test-threads=1` 退出码 0，failed 为 0。其中包含新用例、`failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`、`concurrent_same_table_ddl_is_refused`。
  2. `cargo test --test alter_table_test` 退出码 0。
  3. `cargo test --test ddl_crash_recovery_test` 退出码 0。
  三条顺序执行，同一时刻只有一条。第 1 条的 `--test-threads=1` 是单目标命令，用来避免同文件暂停点用例互相调度，不是 M19 的全量批次。后两条用默认并行度。任一条 failed 非 0，T1 未完成。
- Stop when: 放弃路径必须改 `write_page` 的通用取消语义才能做完；失败路径不得不自己刷盘；成功结果已经交回之后仍要回退；必须把加列放进令牌或改发布布局来源；出现本 Iteration 范围外的实质缺陷。上述任一条都停止并回到计划，不在本 Cycle 里改验收。

**Invariants**

- 磁盘格式不变。`ADD COLUMN` 仍不写 WAL 记录。
- 失败补偿的顺序仍是先恢复列数、再移除列行，且该路径不刷盘。
- 检查时接收端仍开着并已换入活布局之后，语句保持已提交。发送失败不再回退。
- 新用例不依赖其他用例留下的目录、文件或暂停点。
- `catalog_shape` 看见恢复后的列数时，列数页和列行页的 `flush_pages` 都已经返回。

**Non-goals**

- ISS39 的等待和发布布局来源（T2）。
- 全量回归（T3）。
- 列行插入期间被放弃、但列数刷盘尚未开始的窗口。本 Cycle 不把它写成新的验收。
- 删列提交前放弃、在途读拒绝、检查点与未发布意图。

**Acceptance**

1. 新用例 RED 是行为失败：暂停点在当前调用链上时，崩溃重开仍看见上调后的列数或列 `m`。
2. GREEN 后同一用例：处理结束时列数已回到语句前；不 `close` 的崩溃重开，列数等于语句前，列定义不含 `m`。
3. `failed_add_column_is_compensated` 通过：失败后 `close` 再打开，新列不在。
4. `successful_add_column_still_adds_the_column` 与 `alter_add_column_survives_crash` 通过。
5. `concurrent_same_table_ddl_is_refused` 通过。
6. 上面三条验证命令按 Verification 的顺序执行，结果如上。未跑全量，不声明全量通过。

**Verification**

新用例走 `Database::execute_sql`，崩溃重开走 `Database::open`。目录断言读 catalog 的列行与 `column_count`。不另写判定程序。命令与通过条件见 T1 的 Verification。

**Gate 2 Readiness**

PASS。计划审查 11 项通过；RTM 无 Missing、无 Simplified；D1 的四行决定、目录锁跨过两次回退刷盘、页号来源和发送失败后不再回退，都写在本 Task Definition；本 Cycle 与 `tasks.md`、delta spec、`design.md` D1 一致；Persisted Evidence 为 `none`；用户在本次 goal 原文中批准 Gate 1 与 Gate 2，该项按批准通过，不是跳过。

**Persisted Evidence**

- Mode: none

命令、结果、改动文件和符号写入 Act Response 即可。本缺陷的重跑就是上述定向测试，不需要单独留下日志或页面副本。

**Risks and Notes**

- 回退的两次写盘之间如果崩溃，磁盘列数已回到语句前，列行可能还在。`open_or_init` 按列数截断，`SELECT` 看不见新列。`catalog_shape` 的 `scan_columns` 不截断，这个中间状态仍可能扫到该列行。验收不等待这个中间状态，只等待两次 `flush_pages` 都返回之后的崩溃重开。
- 语句 future 丢弃之后，写锁会留到回退结束。这比「future 一丢锁就放」多持有一段，读者在这段里等到回退完成，而不是读到上调后的列数。
- 暂停点未布防时不得改变成功、失败和删列路径。
- 回退若在 `scan_tables` 能看见恢复后的列数之前放开 `Catalog.lock`，`catalog_shape` 会在刷盘返回前放行。随后的不 `close` 崩溃仍会看见上调后的列数。完成信号就是这把锁跨过两次 `flush_pages`。

## Act Response

- Status: reported

### Implemented

加列在列数刷盘返回成功之后不再留在语句 future 里。语句 future 只等 oneshot 的 `Result`。丢弃它会关掉接收端，任务继续做提交或回退。

- `src/executor/alter_table.rs`：`AlterTableExecutor::next` 的 `AddColumn` 臂把 `TableManager::alter_add_column` 放进 `tokio::spawn`。任务持有 `Arc<TableManager>`、表名、列名、存储列类型、`not_null`、默认值，以及 `database.path()` 的 `PathBuf`。成功时语句结果仍是 `AffectedRows(0)`。刷盘错误原样交回。
- `src/storage/data/table_manager.rs`：`TableManager::alter_add_column` 在任务内取得 `tables` 写锁，持有到决定完成。锁顺序仍是先这把写锁，再 `Catalog.lock`。`update_table_column_count_ddl` 返回 `Ok` 之后调用暂停点，然后看发送端 `is_closed()`：
  - 接收端未关且刷盘成功：换入活布局，再发送 `Ok`。发送失败不再回退。
  - 接收端未关且刷盘失败：`compensate_column_add`（不刷盘），发送原来的错误。
  - 接收端已关且刷盘成功：不换入活布局，不发送 `Ok`。调用 `Catalog::rollback_abandoned_add_column`。该调用返回错误时不重试，也不再调用 `compensate_column_add`。
  - 接收端已关且刷盘失败：`compensate_column_add`（不刷盘），不发送成功。
- `TableManager::compensate_column_add` 未改：仍先 `restore_table_column_count_no_flush`，再 `remove_column_row_at`，两次都不刷盘。
- `src/storage/catalog.rs`：新增 `Catalog::rollback_abandoned_add_column`（`pub(crate)`）。同一次 `Catalog.lock` 里恢复列数、移除本次列行，再 `flush_pages` 列数页，然后 `flush_pages` 列行改过的页。列数页来自 `update_field_in_chain` 的返回值，列行页来自 `delete_from_chain` 的返回值。不用 `flush_commit_point`。`restore_table_column_count_no_flush` 和 `remove_column_row_at` 仍不刷盘。
- `src/storage/add_column_count_flush_window.rs`：按库路径一次消费的暂停点，默认不生效。`arm_add_column_count_flush_window` 对集成测试可见。未布防时 `pause_at_add_column_count_flush_window` 立即返回。只从加列路径、在列数刷盘返回 `Ok` 之后调用。
- `src/storage/mod.rs`：导出上述模块。
- `tests/ddl_layout_publication_test.rs`：`abandoned_add_column_during_count_flush_is_not_durable`。自带 `TempDir`，自建两列表并插入一行。暂停、丢弃语句、放行后用 `catalog_shape` 轮询，最多 10 秒，到点仍崩溃重开。崩溃用同文件的 `crash`（停 WAL、丢句柄、不 `close`）。

删列对 `update_table_column_count_ddl` 的调用、`publish_column_narrowing`、窄化令牌和 WAL 记录没有改。`write_page`、`flush_pages`、`flush_all_except` 的取消行为没有改。

### Deviations from Plan

1. 刷盘成功、刷盘失败和放弃回退这几条路径里，`alter_add_column` 向调用方返回 `Ok(())`，语句结果只放在 oneshot 上。spawn 忽略这个返回值。接收端开着时，成功仍是 `AffectedRows(0)`，失败仍是刷盘返回的那个错误。表在执行时已经不存在时，函数用 `?` 返回 `TableNotFound`，发送端被丢弃，语句 future 走第 2 条。计划期会先拒绝不存在的表。
2. 接收端还在等、发送端却没发送就结束时，语句 future 得到 `StorageError::Internal("ADD COLUMN ended before reporting a result")`。四条决定里，接收端开着都会发送；这条只覆盖任务没把结果送出的情况。
3. 新模块在 `src/storage/mod.rs` 里按名称排在文件首位。这是 rustfmt 的模块顺序，不改变调用关系。

### Self-Review

Gate 3 通过。四条决定、失败补偿不刷盘、成功加列仍交回 `AffectedRows(0)`、删列路径不调用本暂停点，都落在上面的符号里。新用例走 `Database::execute_sql` 和 `Database::open`，目录断言走 `catalog_shape`。没有 hash、指纹、revision、run-id 或时间证明。没有计划外的产品文件。

遗留 Minor：表在执行时已经不存在时，`?` 返回 `TableNotFound` 且不往通道发送，语句 future 得到 `StorageError::Internal("ADD COLUMN ended before reporting a result")`。计划期会先拒绝不存在的表，`alter_table_missing_table_rejected` 仍通过。调用点只有这一处 spawn。

Gate 5 未命中。

### Verification

先行测试先停在当时的调用链上（刷盘返回 `Ok` 之后、换入活布局之前），再跑：

`cargo test --test ddl_layout_publication_test abandoned_add_column_during_count_flush_is_not_durable -- --test-threads=1`

编译成功。退出码 101。`0 passed; 1 failed`。断言失败：重开后 `column_count` 为 3，语句前为 2，列名是 `id`、`v`、`m`。耗时 10.02 秒，是轮询到点后的行为失败，不是编译失败。

GREEN 后同一命令：退出码 0。`1 passed; 0 failed`。耗时 0.03 秒，轮询在 10 秒上限之前看到列数回到语句前。

随后三条命令按顺序各跑一次，同一时刻只有一条：

1. `cargo test --test ddl_layout_publication_test -- --test-threads=1`：退出码 0。`10 passed; 0 failed`。含新用例、`failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`、`concurrent_same_table_ddl_is_refused`。
2. `cargo test --test alter_table_test`：退出码 0。`16 passed; 0 failed`。
3. `cargo test --test ddl_crash_recovery_test`：退出码 0。`6 passed; 0 failed`。含 `alter_add_column_survives_crash`。

未跑无目标过滤的 `cargo test`，未跑全量。

### Persisted Evidence

None required。模式是 `none`。上述命令可以重跑，未创建 evidence 目录。

### Experience Candidates

None。

Issue 候选：None。

### 未解决问题

无阻塞项。T2、T3 不在本 Cycle，未做。Iteration 000 的 Review Result 仍是 pending。未填写 Plan Review，未创建后继 Cycle 或后继 Iteration，未改全局 tasks、SNAPSHOT、M/R/I，未关闭 Issue，未归档。

## Plan Review

- Review Result: accepted

**Findings**

`AlterTableExecutor::next` 的 `AddColumn` 臂把 `alter_add_column` 放进 `tokio::spawn`。语句 future 只等 oneshot。任务持有 `Arc<TableManager>`、语句参数的拥有副本，以及 `database.path()` 的 `PathBuf`。`tables` 写锁在任务内取得，持有到决定结束，之后才进入 `Catalog.lock`。

`update_table_column_count_ddl` 返回 `Ok` 之后调用 `pause_at_add_column_count_flush_window`。未布防时该函数立即返回。四条决定与 Task Definition 一致：

- 接收端未关且刷盘成功：换入活布局，再 `send(Ok(()))`。发送失败不回退。
- 接收端未关且刷盘失败：`compensate_column_add`，再把原来的错误送出。
- 接收端已关且刷盘成功：不换入活布局，不发送 `Ok`。`rollback_abandoned_add_column` 在同一次 `Catalog.lock` 里恢复列数、移除本次列行，然后 `flush_pages` 列数页，再 `flush_pages` 列行页。列数页来自 `update_field_in_chain` 的返回值，列行页来自 `delete_from_chain` 的返回值。该调用的错误被忽略：不重试，不再调用 `compensate_column_add`。
- 接收端已关且刷盘失败：`compensate_column_add`，不发送成功。

`compensate_column_add` 仍先 `restore_table_column_count_no_flush`，再 `remove_column_row_at`，两条都不刷盘。删列臂仍停在 `ddl_commit_window`，不调用本暂停点。`publish_column_narrowing` 仍直接调用 `update_table_column_count_ddl`。新用例自带 `TempDir`，建两列表并插入一行，用 `catalog_shape` 轮询，再用同文件 `crash` 后 `Database::open`。没有 hash、指纹、revision、run-id 或时间证明。

非阻塞 Minor，偏差原因是 Act 偏离。表在执行时已经不存在时，`alter_add_column` 取得 `tables` 写锁后用 `?` 返回 `TableNotFound`，发送端被丢弃。语句 future 得到 `StorageError::Internal("ADD COLUMN ended before reporting a result")`。`tests/alter_table_test.rs` 的 `alter_table_missing_table_rejected` 在计划期拒绝不存在的表，断言错误包含 `not found`。这条路径不在本 Iteration 的六条 Acceptance 里。

**Evidence**

- `src/executor/alter_table.rs` 的 `AddColumn` 臂（52–97 行）只等待 oneshot，成功结果是 `AffectedRows(0)`，通道上的错误原样返回。
- `src/storage/data/table_manager.rs` 的 `alter_add_column`（961–1063 行）与 `compensate_column_add`（1077–1087 行）。
- `src/storage/catalog.rs` 的 `rollback_abandoned_add_column`（504–540 行）。`restore_table_column_count_no_flush`（457–475 行）和 `remove_column_row_at`（484–497 行）没有刷盘调用。
- `src/storage/add_column_count_flush_window.rs`：按路径一次消费，默认不生效。
- `tests/ddl_layout_publication_test.rs` 的 `abandoned_add_column_during_count_flush_is_not_durable`（615–659 行）。

上述产品文件在本次阅读时没有未暂存改动，Act Response 的符号声明与当前代码一致。三条定向测试沿用 Act Response，本轮未重跑：

1. 先行 RED：`cargo test --test ddl_layout_publication_test abandoned_add_column_during_count_flush_is_not_durable -- --test-threads=1`。退出码 101，`0 passed; 1 failed`。重开后 `column_count` 为 3，语句前为 2，列名是 `id`、`v`、`m`。耗时 10.02 秒。
2. GREEN：同一命令退出码 0，`1 passed; 0 failed`，耗时 0.03 秒。
3. `cargo test --test ddl_layout_publication_test -- --test-threads=1`：退出码 0，`10 passed; 0 failed`。含新用例、`failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`、`concurrent_same_table_ddl_is_refused`。
4. `cargo test --test alter_table_test`：退出码 0，`16 passed; 0 failed`。
5. `cargo test --test ddl_crash_recovery_test`：退出码 0，`6 passed; 0 failed`。含 `alter_add_column_survives_crash`。

未跑全量。Persisted Evidence 为 `none`，没有证据目录。

**Follow-up Decision**

六条 Acceptance 都有对应代码，测试结论沿用 Act Response。上面的 Minor 不阻塞。不需要当前 Cycle 修复，不创建 rework 或 replan。Iteration Plan 保持不变。

**Next Cycle**

None

**Next Iteration**

目录未创建。`tasks.md` Iteration Map 的下一项是 Iteration 001「在途窄化与同表加列的发布布局」（T2）。本轮不展开。
