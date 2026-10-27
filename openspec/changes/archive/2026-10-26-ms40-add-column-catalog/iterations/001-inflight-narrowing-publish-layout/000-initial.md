# Iteration 001 / Cycle 000: 在途窄化与同表加列的发布布局

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `ddl-layout-publication`「同表布局窄化 DDL 串行化」的两个新场景，以及既有「窄化令牌不约束 ADD COLUMN」和「同表并发的窄化 DDL 被显式拒绝」。实现规则是 `design.md` D2。D2 的等待包住 Iteration 000 已经落地的加列任务。本文件下面写全，执行时不回读 design，也不回读 Iteration 000 的 Plan Context。
- Excluded scope: 重做 D1 的四行决定表。T3 的全量回归。Join、SMJ、Hash 构建侧、ISS36。`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/` 整目录。其他目录面。Issue 状态。尚未出现的缺陷，以及为它们预建 Iteration。

**Objective**

同表 `ADD COLUMN` 在窄化令牌持有或在途窄化已发布时不写该表目录，等待令牌清除，不被令牌拒绝。正常路径发布安装的活布局，是 `alter_drop_column` 在 `try_acquire` 成功时算出、并在返回前同步过 `data_page_tail` 的那一份 `Arc<TableMeta>`。两条语句都完成后，进程内 `SELECT` 与 `close` 再打开的列集合、列数、行值一致。在途读拒绝和第二条同表删列的立即拒绝保持。

**Investigation Facts**

- Starting point: Iteration 000 Cycle `iterations/000-add-column-flush-abandon/000-initial.md` 的 Act Response 为 `reported`，Plan Review 为 `accepted`。该 Cycle 没有 Issue 候选，没有未关闭的阻塞。本 Iteration 不重跑那三条命令，沿用其结论：
  1. `cargo test --test ddl_layout_publication_test -- --test-threads=1`：退出码 0，`10 passed; 0 failed`。含 `abandoned_add_column_during_count_flush_is_not_durable`、`failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`、`concurrent_same_table_ddl_is_refused`。
  2. `cargo test --test alter_table_test`：退出码 0，`16 passed; 0 failed`。
  3. `cargo test --test ddl_crash_recovery_test`：退出码 0，`6 passed; 0 failed`。含 `alter_add_column_survives_crash`。
  未跑全量。遗留 Minor：表在执行时已经不存在时，`alter_add_column` 用 `?` 返回 `TableNotFound` 且不发送，语句 future 得到 `StorageError::Internal("ADD COLUMN ended before reporting a result")`。计划期会先拒绝不存在的表。本 Cycle 不改这句文案。
  本次只重读了 T2 新碰到的符号。没有读 Join 变更目录。下面的行号是这次阅读时的位置。
- Current-State Evidence:
  - `AlterTableExecutor::next` 的 `AddColumn` 臂（`src/executor/alter_table.rs` 52–96 行）已经把 `alter_add_column` 放进 `tokio::spawn`。语句 future 只等 oneshot。生产代码里只有这一处调用 `alter_add_column`。
  - `TableManager::alter_add_column`（`src/storage/data/table_manager.rs` 961–1063 行）一进函数就 `self.tables.write().await`，持有到决定结束。然后 `insert_column_row`、`update_table_column_count_ddl`、列数刷盘暂停点、`is_closed()` 四行决定。没有看 `NarrowingState`。`self.tables` 的类型是 `tokio::sync::RwLock`（463 行，导入在第 5 行）。同一任务持有读锁再要写锁不会升级。
  - `NarrowingState`（117–191 行）：`try_acquire` 失败返回 `None`；`clear_narrowing` 先把 `published` 置假，再把 `owner` 从令牌 CAS 回 0；`release` 就是 `clear_narrowing`。`ensure_readable` 只看 `published`。`owner` 和 `published` 都是私有字段，现在没有「`owner != 0` 或 `published`」的只读查询。用 `try_acquire` 做探测会在失败时留下已递增的 `next_token`，而且不能用来表示等待。
  - `alter_drop_column`（1109 行）在改写前 `try_acquire`（1162–1168 行）。失败立刻返回点名 `ExecutionError`，文案含表名和 `concurrent narrowing DDL refused`。成功后 `narrowed_meta`（1194 行）算出 `new_meta`。改写结束、返回前把活表的 `data_page_tail` 抄进这份 `new_meta`（1426 行）。返回的 `PendingColumnDrop`（1445–1450 行）只有表名、被删列、新列数和 `NarrowingGuard`，没有这份 `Arc`。
  - `apply_column_drop`（1471–1498 行）调用 `publish_column_narrowing`。成功则 `clear_narrowing` 并 `forget_committed_narrowing`；失败则 `keep_narrowing` 并仍返回 `Ok(())`。`publish_column_narrowing`（1515–1542 行）在目录列数已经等于目标时直接 `Ok(())`，不改页，也不换活布局。否则 `rewrite_column_rows_drop`、`update_table_column_count_ddl`，再 `get_table` + `narrowed_meta` 后写入 `tables`。
  - 恢复期 `src/wal/recovery.rs` 802–804 行，以及 `publish_committed_narrowings`（1592 行），都调用三参数的 `publish_column_narrowing`。这两处没有 `PendingColumnDrop`，也没有同进程的 `ADD COLUMN`。`CommittedNarrowing`（484 行）只有表名、被删列序位和新列数。
  - 删列臂在提交并 `register_committed_narrowing` 之后、`apply_column_drop` 之前调用 `pause_at_ddl_commit_window`（`src/executor/alter_table.rs` 153–162 行）。这个窗口里令牌仍被持有，`published` 已在改写开始处设为真，`tables` 写锁没有被删列持有。
  - `tests/ddl_layout_publication_test.rs` 的 `setup` 建 `t(id, v, w)` 和 `u(id, w)`，`t` 插入 `1..=SOURCE_ROWS`（30）行 `(id, 'v{id}', 'w{id}')`。`park_at_commit_window` 用 `select` 等 `reached()`，语句先结束则失败。`catalog_shape` 不读数据面。`observe` 再跑 `SELECT *`。`read_during_narrowing_in_flight_is_refused` 在 `tests/ddl_rewrite_consistency_test.rs` 820 行，仍用同一个提交窗口断言点名拒绝。
  - 这两条定向命令现在仍能判定 T2：新用例放在 `ddl_layout_publication_test.rs`，在途读拒绝的回归在 `ddl_rewrite_consistency_test.rs`。不需要改命令。
- Code and Critical Path: 删列是 `execute_sql` → `DropColumn` 臂 → `alter_drop_column`（取令牌、`narrowed_meta`、改写、同步尾指针、返回 pending）→ 提交 → 提交窗口暂停 → `apply_column_drop` → 今天的 `publish_column_narrowing` 再 `get_table`。加列是 `execute_sql` → `AddColumn` 臂 → 任务内 `alter_add_column`，目前一开始就拿写锁并写目录。读路径仍是 `ensure_readable` 只看 `published`。恢复路径仍是打开后的 `publish_column_narrowing`。

**Implementation Guidance**

先加测试可见的等待暂停点，但先不要从 `alter_add_column` 调用它，让新用例在现有调用链上 RED。RED 之后再做两处产品改动：pending 带上捕获的 `Arc` 并由正常路径安装；加列任务在拿写锁和写目录之前按只读条件等待。两处都落地后再跑 GREEN。唤醒方式可以用循环里的 `yield_now`，也可以在 `clear_narrowing` 成功把 `owner` 清回 0 之后 `notify`；两种都要 `.await`，唤醒后都要再读原子量。函数怎么拆开共享目录步骤，不影响下面的可观察结果。

**Task Definitions**

### T2: 在途窄化的发布不使用被加列改过的活布局

- Requirement/Scenario: `ddl-layout-publication`「在途窄化窗口内的同表 ADD COLUMN 不改写发布所用布局」「另一张表的 ADD COLUMN 不等待在途窄化」。保持「窄化令牌不约束 ADD COLUMN」和「同表并发的窄化 DDL 被显式拒绝」。
- Depends on: T1
- Targets:
  - `src/storage/data/table_manager.rs` `PendingColumnDrop`、`TableManager::alter_drop_column`、`TableManager::apply_column_drop`
  - `src/storage/data/table_manager.rs` `TableManager::alter_add_column`，以及 `NarrowingState` 上的一个只读判断
  - `src/storage/data/table_manager.rs` `TableManager::publish_column_narrowing` 保持给恢复期和 `publish_committed_narrowings` 的推导安装
  - 新模块 `src/storage/add_column_narrowing_wait.rs`，由 `src/storage/mod.rs` 导出。按库路径索引，默认不生效。不在第一次暂停时把它从 map 里删掉；测试用 `disarm` 摘掉。不携带 hash、指纹、revision、run-id 或时间证明
  - `tests/ddl_layout_publication_test.rs` 新用例 `add_column_waits_out_inflight_narrowing_and_reopen_matches`
- Current behavior: 同表 `ADD COLUMN` 不看令牌。删列停在提交窗口时，加列拿到 `tables` 写锁，把列数刷上去并换入含新列的活布局。随后 `apply_column_drop` 经 `publish_column_narrowing` 对当时的活布局再做一次 `narrowed_meta`。`PendingColumnDrop` 不携带取得令牌时的那份布局。
- Required behavior:
  - `NarrowingState` 提供只读判断：`owner != 0` 或 `published` 为真时返回真。不修改原子量，不返回令牌值，不调用 `try_acquire`。方法名由实现自定。
  - 等待在 `alter_add_column` 内部、`self.tables.write()` 之前、`insert_column_row` 之前。此时不持有 `tables` 的读锁或写锁。`get_table` 返回后它的读锁已经放下，可以持有返回的 `Arc` 再 `.await`。判断只看本次 `table` 参数那一份 `NarrowingState`。
  - 判断为真时的循环是：只读判断为假就离开；调用 `pause_at_add_column_narrowing_wait`；再判断一次，为假就离开；若这次调用真正停住过，立刻回到循环开头，中间不要做令牌等待。测试的第一次 `release` 之后，任务会马上进入第二次暂停。若这次调用没有停住（未布防或已经 `disarm`），并且判断仍为真，才 `.await` 让出，然后回到循环开头。条件变为假之后，这次让出必须结束。使用 `Notify` 时，在 `clear_narrowing` 把 `published` 清掉且 `owner` 回到 0 之后通知；使用 `yield_now` 时，下一次判断看到条件为假就离开。未布防时不能靠忙等占住运行时。
  - 离开循环后取得 `tables` 写锁，读 map 里该表当前的 `TableMeta`，再判断一次。仍为真：放开写锁，回到循环，这次不写目录。为假：后面的列行序位、列数和换入的活布局都用锁里这份 `TableMeta`，不用等待前克隆的那份。然后进入现在的函数体，包括列数刷盘暂停点和 D1 的四行决定。
  - 等待期间不检查 `is_closed()`，也不新造拒绝错误。接收端已关闭时，仍等到进入现有函数体，由刷盘返回之后的 `is_closed()` 走 D1 已有的行。
  - 表不存在时不报成窄化令牌拒绝。写锁下找不到表仍用现在的 `?`。不改 `StorageError::Internal("ADD COLUMN ended before reporting a result")` 这句文案。
  - `alter_drop_column` 在 1426 行同步 `data_page_tail` 之后，把同一个 `new_meta` `Arc` 放进 `PendingColumnDrop`。这是 `Arc` 克隆，不是再调用 `narrowed_meta`，也不是在发布时从活表再抄尾指针。字段名由实现自定。
  - `apply_column_drop` 在目录列数已经等于 `new_column_count` 时：不改页，不把捕获的 `Arc` 写入 `tables`，然后仍 `clear_narrowing` 并 `forget_committed_narrowing`。列数还不等时：先 `rewrite_column_rows_drop`，再 `update_table_column_count_ddl`，成功后把捕获的那个 `Arc` 插入 `tables`。这条路径不再 `get_table` 后 `narrowed_meta`。目录步骤失败时不插入，`keep_narrowing`，对语句仍返回 `Ok(())`。
  - `publish_column_narrowing` 的三参数签名和推导安装保留给 `src/wal/recovery.rs` 802–804 行和 `publish_committed_narrowings`。这两处调用不改。捕获布局不写入 `CommittedNarrowing`，也不写入 WAL。
  - 发布失败留下的 `owner != 0` 或 `published` 使同表加列继续等待。不设超时，不改成拒绝。本用例不覆盖发布失败。
  - 同表第二条 `DROP COLUMN` 仍在 `try_acquire` 失败时立即返回现有点名错误，不排队。
  - 暂停点模块的对外形状：`arm_add_column_narrowing_wait(path) -> Arc<AddColumnNarrowingWaitGate>`，`disarm_add_column_narrowing_wait(path)`，`pause_at_add_column_narrowing_wait` 为 `pub(crate)` 并返回这次是否真正停住。句柄提供 `reached_count()` 和 `release()`。进入次数在开始等待 `release` 之前加一。`Notify` 在没有等待者时保留一个许可，避免 `release` 早于 `await` 把任务停死。`disarm` 把该路径从 map 移除，之后的 `pause` 立即返回且不再加次数。
- Preserve:
  - D1 四行决定、`compensate_column_add` 不刷盘、`rollback_abandoned_add_column` 的回退顺序、列数刷盘暂停点的位置（刷盘 `Ok` 之后、关闭检查之前）。
  - `ensure_readable` 仍只看 `published`。
  - `failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`、`abandoned_add_column_during_count_flush_is_not_durable`、`concurrent_same_table_ddl_is_refused`。
  - `ddl_rewrite_consistency_test.rs` 的在途读拒绝、完成后的窄化读、另一张表在窗口内仍可读。
- Forbidden:
  - 修改 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/`，或改 Join、SMJ、Hash 构建侧。
  - 改 D1 的四行决定，或把列数刷盘暂停点挪到等待之前。
  - 让 `ADD COLUMN` 调用 `try_acquire`、保存令牌，或返回同表第二条删列那种拒绝。
  - 等待时持有 `tables` 写锁或读锁。
  - 在 `apply_column_drop` 里用发布当时的 `get_table` + `narrowed_meta` 安装活布局，包括先装上再覆盖。
  - 列数已等于目标时仍替换活布局。
  - 修改 `src/wal/recovery.rs` 的发布调用，或把捕获布局塞进 WAL / `CommittedNarrowing`。
  - 给等待加超时，或把发布失败的 `keep_narrowing` 改成放行加列。
  - 改 `write_page`、`flush_pages`、`flush_all_except` 的取消行为。
  - 新增公共 API、错误变体或磁盘格式字段。
  - 用 hash、指纹、revision、run-id、session id 或时间证明标识这次运行。
  - 跑全量，或一次无目标过滤的 `cargo test`。
  - 修本 Iteration 未纳入的新缺陷。与本变更故障域相同的留给后续 Iteration；其他故障域仍留在 MS40，不新建里程碑。不预建那些 Iteration。
- First test: `tests/ddl_layout_publication_test.rs::add_column_waits_out_inflight_narrowing_and_reopen_matches`。自带 `setup` 的 `TempDir`。在布防删列窗口之前再 `CREATE TABLE s (id INT PRIMARY KEY)`。`s` 只用于第 8 步，确认等待暂停点摘掉之后同表加列没有占着写锁。
  1. 记下 `catalog_shape(&db, "t")`。`setup` 之后列名是 `id`、`v`、`w`，列数是 3。
  2. `arm_ddl_commit_window`，把 `ALTER TABLE t DROP COLUMN w` 停在 `park_at_commit_window`。
  3. `arm_add_column_narrowing_wait`。把 `ALTER TABLE t ADD COLUMN m INT` 做成 pinned future。用同文件 `park_at_commit_window` 的 `select` 等到 `reached_count() >= 1`。语句先结束则失败。
  4. 两个暂停都还没放行时：`t` 的 `catalog_shape` 仍是第 1 步；`SELECT * FROM t` 的错误文本包含 `'t'`；`ALTER TABLE u ADD COLUMN extra INT` 返回成功；`t` 的 `catalog_shape` 仍是第 1 步。
  5. `release` 一次，让加列任务离开第一次暂停。再用 `select` 等到 `reached_count() >= 2`，上限是本文件 `ANCHOR_TIMEOUT`。等待期间如果加列 future 已经完成，或 `t` 的目录列数或列名变了，失败。到点计数仍小于 2，也是失败。正确实现会马上进入第二次暂停，不会用满这个上限。
  6. `disarm_add_column_narrowing_wait`，再 `release` 一次，让任务离开第二次暂停。随后 `yield_now` 一次。
  7. `select` 加列 future 和一次 `yield_now`：future 已完成则失败。
  8. `ALTER TABLE s ADD COLUMN extra INT` 在删列放行之前返回成功。然后再做一次第 7 步的 `select`，并确认 `t` 的 `catalog_shape` 仍是第 1 步。
  9. 放行删列窗口。删列 future 与加列 future 都返回成功。加列的成功结果仍是受影响行数 0 的那条 SQL 成功路径。
  10. 进程内 `observe(&db, "t")`。然后 `db.close().await`，再 `Database::open` 同一路径，再 `observe`。两次的列名都是 `id`、`v`、`m`，列数都是 3，按 `id` 排序后的行相同。每一行是该 `id`、`v{id}`、JSON null。列名里没有 `w`。行数是 `SOURCE_ROWS`。不要用同文件的 `crash`。
  - 变更前命令：`cargo test --test ddl_layout_publication_test add_column_waits_out_inflight_narrowing_and_reopen_matches -- --test-threads=1`
  - RED：先把暂停点模块和导出加上，使用例能编译。先不要从 `alter_add_column` 调用它，也不要改发布安装。当前加列不等待，语句会在 `reached_count() >= 1` 之前结束，`select` 失败。编译失败不算这次 RED。没看到这个失败，不得开始 GREEN 的产品修改。
- GREEN condition: 同一命令退出码 0。用例走到第 10 步，进程内观察与 `close` 再打开一致。此时捕获 `Arc` 的安装和令牌等待都已经在代码里。只做了等待、发布仍 `get_table` 后重算，不得把该命令的通过当成 GREEN。
- Verification:
  1. `cargo test --test ddl_layout_publication_test -- --test-threads=1` 退出码 0，failed 为 0。其中包含新用例、`concurrent_same_table_ddl_is_refused`，以及 Iteration 000 已通过的加列用例。
  2. `cargo test --test ddl_rewrite_consistency_test` 退出码 0，failed 为 0。其中包含 `read_during_narrowing_in_flight_is_refused`。
  两条顺序执行，同一时刻只有一条。第 1 条的 `--test-threads=1` 只稳住这个文件里的暂停点。第 2 条保持 `tasks.md` 所写的默认并行度。任一条 failed 非 0，T2 未完成。不跑全量。
- Stop when: 等待必须持有 `tables` 写锁才能做完；不改 D1 四行决定就无法让加列等在目录写入之前；恢复期必须改成使用捕获布局；同表第二条删列必须改成排队；必须改 Join 执行器。上述任一条都停止并回到计划。捕获 `Arc` 是否安装，由 Acceptance 第 4 条看代码，不由第 10 步单独证明。

**Invariants**

- 磁盘格式不变。`ColumnNarrowing` WAL 记录的字段不变。
- 删列从取得令牌到发布结束，仍不全程持有 `tables` 写锁。读者仍能在 `published` 为真时被点名拒绝。
- 令牌清除发生在捕获布局已经安装之后。加列在清除之后、写锁复查通过之后，才按锁里的活布局追加。
- 新用例不依赖其他用例留下的目录、文件或暂停点。`u` 的加列和 `s` 的加列都不读取 `t` 的令牌。
- 未布防的等待暂停点不改变成功加列、失败补偿、放弃回退和删列。

**Non-goals**

- 重做 D1 的放弃决定表，以及 Iteration 000 那个表不存在时的 Internal 文案。
- 发布失败后同表加列一直等到进程结束的路径。行为按上面的等待条件保持，本用例不断言它。
- 检查点在 `apply_column_drop` 之前抢先发布。那条仍走 `publish_column_narrowing` 的推导；列数已经相等时正常路径不替换活布局。
- 全量回归。Join。为还不存在的缺陷预建 Iteration。

**Acceptance**

1. 新用例 RED 是行为失败：暂停点还没被加列调用时，同表 `ADD COLUMN` 在 `reached_count() >= 1` 之前结束。
2. GREEN 后同一用例：删列停在提交窗口时，同表加列不因令牌被拒绝；`t` 的目录列数和列名保持发布前；读 `t` 的错误点名 `t`；`u` 的加列在删列放行前完成，且不改 `t` 的目录。放行后两条都成功。进程内 `SELECT` 与 `close` 再打开的列名、列数、行值一致：`w` 不在，`m` 在且为 JSON null，原来的 `id` 和 `v` 还在。
3. 第一次 `release` 之后、删列仍停着时，加列再次进入等待暂停点（`reached_count() >= 2`）。`disarm` 之后、删列放行之前，`s` 的加列能完成，`t` 的目录仍不变，同表加列 future 仍未完成。
4. `apply_column_drop` 在列数尚未等于目标时安装的是 pending 里那个 `Arc`，不是发布当时 `get_table` 再 `narrowed_meta` 的结果。列数已等于目标时不替换活布局。恢复期和 `publish_committed_narrowings` 仍推导。
5. `concurrent_same_table_ddl_is_refused` 通过。`ddl_rewrite_consistency_test` 的在途读拒绝通过。
6. Verification 的两条命令按顺序执行，结果如上。未跑全量，不声明全量通过。

**Verification**

新用例的删列、加列、另一张表的加列和 `SELECT` 都走 `Database::execute_sql`。重开走 `Database::close` 然后 `Database::open`。目录断言用 `catalog_shape`。行值用 `observe` 的 `SELECT *` 结果。不另写判定程序。

第 4 条 Acceptance 里「安装的是哪一个 `Arc`」不能由这个 SQL 用例单独区分：等待生效时，发布当时的活布局还没被加列改过，重算和捕获 `Arc` 的列清单相同。这条以代码为准，Review 时看 `apply_column_drop` 的插入点和 `publish_column_narrowing` 的两个保留调用点。命令与通过条件见 T2 的 Verification。

**Gate 2 Readiness**

PASS。计划审查 11 项通过；RTM 无 Missing、无 Simplified；等待条件、捕获 `Arc` 的安装、幂等返回不替换活布局、恢复期仍推导、第二条删列仍立即拒绝，都写在本 Task Definition；本 Cycle 与 `tasks.md` 的 T2、delta spec、`design.md` D2 一致；Persisted Evidence 为 `none`；用户在本次 goal 原文中批准计划和 Gate 2，该项按批准通过，不是跳过。没有 BLOCKED。没有 SKIPPED。

**Persisted Evidence**

- Mode: none

命令、结果、改动文件和符号写入 Act Response 即可。定向测试可以重跑，不需要单独留下日志或页面副本。

**Risks and Notes**

- 发布失败走 `keep_narrowing` 时，令牌不清除。同表加列会一直停在等待上，直到进程结束。这是 D2 的等待条件，不是本用例的通过条件。不要加超时把它变成错误。
- 调用方在等待期间丢弃语句 future 时，任务仍会等到令牌清除，再进入 D1 的函数体。刷盘成功且接收端已关闭的那一行会回退。本用例不丢弃加列 future。
- `s` 的加列只用来观察：等待暂停点摘掉之后，同表加列没有占着 `tables` 写锁。它不是一条新的产品场景。
- 列数刷盘暂停点如果在本用例里被同时布防，正确实现会在发布之后停在那里。本用例不要布防它。
- Iteration 000 的三条命令覆盖范围没有被这次计划改写。本 Cycle 不重跑它们。

## Act Response

- Status: reported

### Implemented

同表 `ADD COLUMN` 在拿 `tables` 写锁和写目录之前，按该表 `NarrowingState` 的只读条件等待。正常路径发布安装的是删列在同步 `data_page_tail` 之后放进 pending 的那一份 `Arc<TableMeta>`。

- `src/storage/data/table_manager.rs`：`NarrowingState::add_column_must_wait` 在 `owner != 0` 或 `published` 为真时返回真。只读原子量，不调用 `try_acquire`。
- `TableManager::alter_add_column`：`get_table` 返回后放下读锁，持有该表的 `NarrowingState`，调用 `wait_out_same_table_narrowing`。条件为假才离开。离开后取得写锁，再读 map 里当前的 `TableMeta`。仍要等就放开写锁回到循环，这次不写目录。为假时，列行序位、列数和换入的活布局都用锁里这份。列数刷盘暂停点仍在 `update_table_column_count_ddl` 返回 `Ok` 之后、`is_closed()` 之前。等待期间不看接收端是否已关闭。表不存在时写锁下仍用 `?` 返回 `TableNotFound`。
- `wait_out_same_table_narrowing`：判断为假就离开；调用 `pause_at_add_column_narrowing_wait`；再判断，为假就离开。这次调用真正停住过就立刻回到循环开头。没有停住且条件仍为真时 `yield_now` 一次，再回到开头。
- `PendingColumnDrop::narrowed_layout`：`alter_drop_column` 在同步 `data_page_tail` 之后 `Arc::clone` 同一份 `new_meta`。不是再调用 `narrowed_meta`。
- `TableManager::apply_column_drop`：目录列数已经等于 `new_column_count` 时不改页，也不把这份 `Arc` 写入 `tables`，然后 `clear_narrowing` 并 `forget_committed_narrowing`。列数还不等时先 `rewrite_column_rows_drop`，再 `update_table_column_count_ddl`，成功后插入 `pending.narrowed_layout`。这条路径不再 `get_table` 后 `narrowed_meta`。目录步骤失败时不插入，`keep_narrowing`，语句仍返回 `Ok(())`。令牌清除在需要安装的布局已经写入之后。
- `TableManager::publish_column_narrowing` 仍是三参数，内部仍 `get_table` 后 `narrowed_meta`。`src/wal/recovery.rs` 的调用和 `publish_committed_narrowings` 没有改。捕获布局没有写入 `CommittedNarrowing` 或 WAL。
- `src/storage/add_column_narrowing_wait.rs`：按库路径索引，默认不生效。`arm_add_column_narrowing_wait`、`disarm_add_column_narrowing_wait` 对测试可见。`pause_at_add_column_narrowing_wait` 为 `pub(crate)`，返回这次是否真正停住。第一次暂停不从 map 删除。`disarm` 摘掉路径后，后续暂停立即返回 `false` 且不加次数。进入次数在等待 `release` 之前加一。`Notify` 在没有等待者时保留一个许可。
- `src/storage/mod.rs`：导出该模块。
- `tests/ddl_layout_publication_test.rs`：`add_column_waits_out_inflight_narrowing_and_reopen_matches`。自带 `setup` 的 `TempDir`。布防删列窗口之前 `CREATE TABLE s`。删列、加列、另一张表的加列和 `SELECT` 走 `Database::execute_sql`。重开走 `close`、放下句柄、再 `Database::open`。目录用 `catalog_shape`，行值用 `observe`。

同表第二条 `DROP COLUMN` 仍在 `try_acquire` 失败时立即返回原来的点名错误。`ensure_readable` 仍只看 `published`。`Value` 的 `PartialEq`/`Hash` 和排序合并执行器没有改。

### Deviations from Plan

1. 重开前在 `close` 之后 `drop(db)`。文件锁要等句柄释放才放开。不放下时第二次 `Database::open` 返回 `DatabaseLocked("/tmp/.../ddl_layout_publication.db")`。同文件 `failed_add_column_is_compensated` 也是 `close` 之后 `drop` 再打开。
2. 第二次暂停的 `select` future 在观察结束后 `drop`。它借用 `db`，留到函数结束会让 `drop(db)` 无法编译（`E0505`）。
3. 未布防且条件仍为真时的让出是 `tokio::task::yield_now`。没有在 `clear_narrowing` 上增加 `Notify`。计划两种都允许。让出结束后回到循环开头，再读 `owner` 和 `published`。

### Self-Review

Gate 3 通过。新用例走到第 10 步：删列停在提交窗口时同表加列不被拒绝，`t` 的目录保持发布前，读 `t` 的错误点名 `t`，`u` 的加列在删列放行前完成。第一次 `release` 之后 `reached_count` 到 2。`disarm` 之后、删列放行之前，`s` 的加列完成，`t` 的目录不变，同表加列 future 仍未完成。放行后两条都成功，加列结果是 `AffectedRows(0)`。进程内 `SELECT` 与关闭再打开的列名是 `id`、`v`、`m`，列数是 3，`w` 不在，`m` 为 JSON null，行数是 `SOURCE_ROWS`。

`apply_column_drop` 列数尚未等于目标时插入的是 `pending.narrowed_layout`，不是发布当时 `get_table` 再 `narrowed_meta` 的结果。列数已等于目标时不替换活布局。恢复期和 `publish_committed_narrowings` 仍推导。第 10 步单独不能区分这两份列清单；安装点以这段代码为准。

先行测试是行为失败：暂停点还没被加列调用时，语句在 `reached_count() >= 1` 之前以 `Ok(AffectedRows(0))` 结束。编译成功。没有 hash、指纹、revision、run-id 或时间证明。`ANCHOR_TIMEOUT` 仍是到点即失败的夹具上限。没有计划外的产品文件。

Gate 5 未命中。没有遗留 Minor。

### Verification

先行测试命令：

`cargo test --test ddl_layout_publication_test add_column_waits_out_inflight_narrowing_and_reopen_matches -- --test-threads=1`

RED：编译成功。退出码 101。`0 passed; 1 failed`。失败点是 `statement finished before the add-column narrowing wait was reached: Ok(AffectedRows(0))`。当时 `pause_at_add_column_narrowing_wait` 还没有被调用，编译带有该函数未使用的警告。产品修改接上调用后，这条警告不再出现。

GREEN 后同一命令：退出码 0。`1 passed; 0 failed`。耗时 0.14 秒。此时等待和捕获 `Arc` 的安装都已经在代码里。

随后两条命令按顺序各跑一次，第二条在第一条退出码为 0 之后才启动：

1. `cargo test --test ddl_layout_publication_test -- --test-threads=1`：退出码 0。`11 passed; 0 failed`。含新用例、`concurrent_same_table_ddl_is_refused`、`abandoned_add_column_during_count_flush_is_not_durable`、`failed_add_column_is_compensated`、`successful_add_column_still_adds_the_column`。
2. `cargo test --test ddl_rewrite_consistency_test`：退出码 0。`9 passed; 0 failed`。含 `read_during_narrowing_in_flight_is_refused`。

未跑无目标过滤的 `cargo test`，未跑全量。

### Persisted Evidence

None required。模式是 `none`。上述命令可以重跑，未创建 evidence 目录。

### Experience Candidates

None。

Issue 候选：None。

### 未解决问题

无阻塞项。T3 不在本 Cycle，未做。Review Result 仍是 pending。未填写 Plan Review，未创建后继 Cycle 或后继 Iteration，未改全局 tasks、SNAPSHOT、M/R/I，未关闭 Issue，未归档。

## Plan Review

- Review Result: accepted

**Findings**

无阻塞 Acceptance 的发现。无 Minor。偏差分类里没有 Plan 遗漏、Plan 错误、Act 偏离、现状变化或新证据。

Acceptance 4 以当前代码为准。`apply_column_drop` 在目录列数尚未等于 `new_column_count` 时，目录步骤成功后插入的是 `pending.narrowed_layout`。该 `Arc` 来自 `alter_drop_column` 在同步 `data_page_tail` 之后的 `Arc::clone(&new_meta)`。这条路径没有 `get_table`，也没有 `narrowed_meta`。列数已经等于目标时不进入插入，随后仍 `clear_narrowing` 并 `forget_committed_narrowing`。目录步骤失败时 `keep_narrowing` 并返回 `Ok(())`，不插入。`publish_column_narrowing` 仍是三参数，内部仍 `get_table` 后 `narrowed_meta`。`src/wal/recovery.rs` 对已提交 `ColumnNarrowing` 的调用，以及 `publish_committed_narrowings`，仍走这个三参数入口。`CommittedNarrowing` 和 WAL 记录没有捕获布局。

等待在 `alter_add_column` 里、`self.tables.write()` 之前。`get_table` 的读锁在函数返回时放下。`add_column_must_wait` 只读 `owner != 0` 或 `published`，不调用 `try_acquire`。真正停住过的暂停回到循环开头，中间没有 `yield_now`。未停住且条件仍为真时才 `yield_now`。写锁下复查仍为真时放开写锁并回到外层循环，这次不写目录。为假时，列序位、列数和换入的活布局用锁里那份 `TableMeta`。列数刷盘暂停点仍在 `update_table_column_count_ddl` 返回 `Ok` 之后、`is_closed()` 之前。同表第二条删列仍在 `try_acquire` 失败时返回点名 `ExecutionError`。`ensure_readable` 仍只看 `published`。

新用例的步骤覆盖 Task Definition 的 1–10 步：布防删列窗口之前 `CREATE TABLE s`；两个暂停都未放行时目录保持 `id, v, w` 且列数为 3，`SELECT * FROM t` 的错误文本包含 `'t'`，`u` 的加列成功且不改 `t`；第一次 `release` 之后等到 `reached_count >= 2`；`disarm` 后再 `release` 一次并 `yield_now`；删列放行前 `s` 的加列成功，`t` 的目录不变，同表加列 future 未完成；放行后删列成功，加列结果是 `AffectedRows(0)`；进程内 `observe` 与 `close` 后 `Database::open` 同一路径的列名是 `id, v, m`，列数是 3，`w` 不在，`m` 为 JSON null，行数是 `SOURCE_ROWS`，按 `id` 排序后的行相同。暂停点模块按库路径索引，默认不生效；`disarm` 摘掉路径；进入次数在等待 `release` 之前加一；没有 hash、指纹、revision、run-id 或时间证明。

Act Response 的三处局部差异不构成缺口。`close` 之后 `drop(db)` 才放开文件锁，同文件 `failed_add_column_is_compensated` 也是这个顺序；重开仍是同一路径上的 `Database::open`。第二次暂停的 `select` 在观察结束后 `drop`，否则它借用 `db`，`drop(db)` 无法编译。未布防时的让出是 `tokio::task::yield_now`，Task Definition 允许这种做法，也允许在 `clear_narrowing` 上 `notify`。

`tests/ddl_rewrite_consistency_test.rs` 相对索引的未暂存差异只有换行。`read_during_narrowing_in_flight_is_refused` 仍要求错误文本包含 `'t'`，并包含 `ALTER TABLE` 或 `being modified`。`concurrent_same_table_ddl_is_refused` 仍要求同表第二条删列被拒绝。

**Evidence**

沿用 Act Response 的验证，本轮没有重跑。当前代码与该 Response 的实现描述一致，两个测试文件的 `#[tokio::test]` 个数分别是 11 和 9，与通过计数一致。

- 先行测试 `cargo test --test ddl_layout_publication_test add_column_waits_out_inflight_narrowing_and_reopen_matches -- --test-threads=1`：退出码 101，`0 passed; 1 failed`。失败点是语句在进入等待前以 `Ok(AffectedRows(0))` 结束。来源：本 Cycle Act Response。
- GREEN 后同一命令：退出码 0，`1 passed; 0 failed`。来源：本 Cycle Act Response。
- `cargo test --test ddl_layout_publication_test -- --test-threads=1`：退出码 0，`11 passed; 0 failed`。来源：本 Cycle Act Response。
- `cargo test --test ddl_rewrite_consistency_test`：退出码 0，`9 passed; 0 failed`。第二条在第一条退出码为 0 之后执行。来源：本 Cycle Act Response。
- 未跑全量。Persisted Evidence 模式是 `none`，没有 evidence 目录。

安装点：`src/storage/data/table_manager.rs` 的 `PendingColumnDrop::narrowed_layout`、`alter_drop_column` 返回处的 `Arc::clone`、`apply_column_drop` 的插入、`publish_column_narrowing`。等待：`NarrowingState::add_column_must_wait`、`wait_out_same_table_narrowing`、`src/storage/add_column_narrowing_wait.rs`。用例：`tests/ddl_layout_publication_test.rs` 的 `add_column_waits_out_inflight_narrowing_and_reopen_matches`。

**Follow-up Decision**

六条 Acceptance 都已满足。Iteration Plan 保持不变。不需要当前 Cycle 修复，不创建后继 Cycle。本次审查不展开 Iteration 002；`tasks.md` 的 Iteration Map 里 002 仍是未展开，任务仍是 T3。

**Next Cycle**

None

**Next Iteration**

未创建。`tasks.md` Iteration Map 的 002（T3，本变更的全量回归）保持未展开。本次审查不建立该目录。
