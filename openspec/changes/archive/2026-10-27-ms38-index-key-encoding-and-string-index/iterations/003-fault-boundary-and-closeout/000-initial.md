# Iteration 003 / Cycle 000：故障边界、运行形态与收尾

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T7、T8
- Acceptance gaps: None
- Repair items: None
- Inherited scope：R4 的取消、超时、失败和提交前回收；R5 的干净重开、未决 WAL、旧库，以及内存、只读、加密、附加库 DML；R2/R3 的 `DEFAULT` 与溢出页组合；R6 的一次全量回归。fixed32、32 字节键、`secondary_index_key` 和 002 的类型门与等值路由继续有效。
- Excluded scope：字符串主键、字符串 UNIQUE、范围路由、参数路由、限定列路由、组合谓词路由、附加库上的索引 DDL、新的取消 API、`CREATE INDEX` 的回收协议、ordered-variable、prefix-restart、条目宽度、文件头、`Value::to_key` 的类型范围。ISS36 的全库清理。第二轮全量。Criterion。修改 000、001、002 的 Cycle 文件。全局里程碑、SNAPSHOT 和主规格。

**Objective**

字符串二级索引在取消、超时、失败、重开、未决 WAL、旧库、内存、只读、加密和附加库 DML 下，经 `Database::execute_sql` 或 CLI 得到与已提交行一致的等值结果。用户文档写明实际接受的类型，以及超过 32 字节键容量时的完整值扫描。本变更唯一一次全量按分批规则跑完并留下各批计数。全局 fmt/clippy 若仍失败，照实返回。

**Background**

002 的 Review Result 为 accepted，Next Cycle 为 None。两条非阻塞发现留在 `iterations/002-sql-declaration-and-equality/000-initial.md` 的 Plan Review：CLI 测试多了一次 `rtsql new`；类型门消息带有 `Plan error: ` 前缀。本 Cycle 不修改那份审查。

批准来自本次执行 goal，不是 Gate 跳过。goal 要求把已登记的缺陷纳入本变更的新 Iteration。规划时没有新登记的 Issue，所以不另建 Iteration。

**Investigation Facts**

- Starting point：002 Act Response 的定向结果继续有效，覆盖文件在该审查之后没有再改。`string_index_sql_test` 8 passed / 0 failed，`secondary_index_test` 27 passed / 0 failed，`string_secondary_index_test` 6 passed / 0 failed。001 已用 `CreateIndexExecutor` 覆盖重开、未决 WAL 和 `legacy_g1.db`。那些测试不代替本轮的 SQL 入口。001 的 Act Response 明确把取消、超时、内存、只读、加密和附加库留在 T7。全量预算还没有消耗。规划阶段没有跑全量。
- Current-State Evidence：
  - 插入一行的顺序在 `src/executor/insert.rs`：先写数据页和 WAL，再 `record_version`，然后主键，然后唯一索引，最后二级索引。因此活跃事务的版本数达到 2 时，第一行的字符串二级键已经写入。版本数等于 1 还不能证明该键已在树上。
  - 隐式 DML 在 `src/pipeline.rs` 的 `execute_stage_routed` 里 `begin`，并用 `implicit_tx_guard` 登记。future 被丢弃后，下一条隐式写、`checkpoint` 和 `close` 调用 `Database::reclaim_abandoned_implicit_transactions`。回收走 `abort_by_id`。`src/transaction/manager.rs` 还原二级键时调用 `wal::recovery::extract_index_keys`，二级分支使用 `secondary_index_key`。
  - `CREATE INDEX` / `DROP INDEX` 在同一函数里直接执行，不调用 `implicit_tx_guard`。本轮不为 `CREATE INDEX` 增加回收协议。
  - 显式事务里的语句不建这条守卫。`tokio::time::timeout` 到期只是放弃 future。超时之后事务仍由调用方 `rollback` 或 `commit`。`Database::commit` 在事务离开活跃集之后才返回；在离开之后、future 返回之前丢弃 commit future，行保持已提交。既有夹具把 `set_checkpoint_bytes_threshold(1)` 放在 commit 之前，用来拉宽这个窗口。阈值是夹具手段，不是产品判据。
  - `RepeatableRead` 的扫描不做未提交过滤。显式事务「事务内可见、事务外不可见」用 `Database::open_with_isolation(..., IsolationLevel::ReadCommitted)`。
  - 多行 `INSERT` 的 `DuplicateKey` 发生在 `execute_executor` 的 `next` 中，用户可见文本是 `Execution error: Duplicate key`，`sqlstate` 为 `None`。pipeline 随后 `abort` 整条隐式事务。
  - `EXPLAIN` 的结果行第一列是字符串。命中索引时同时含 `IndexScanAll` 和 `index=<索引名>`。扫描时含 `DataScan`，且不含 `IndexScanAll`。
  - 附加库 DML 由 CLI 会话执行 `ATTACH`。`Database` 没有公开的 attach 方法。写执行器经 `routed_db` 使用目标库的 `TableMeta`。`build_create_index` 仍调用 `reject_attached_write`，消息为 `Plan error: Unsupported statement type: WRITE TO ATTACHED DATABASE (attached databases are read-only)`，SQLSTATE `0A000`。CLI 对这条 SQL 错误的退出码是 3。
  - 只读拒绝在 `readonly_rejection`：`25006`，`cannot execute CREATE INDEX in a read-only session` 与 `cannot execute INSERT in a read-only session`。
  - `open_in_memory` 不创建 WAL writer、WAL buffer、checkpoint manager，`path()` 为 `":memory:"`。
  - 加密文件：64 字节明文头，其后每条记录 4124 字节（12 字节 nonce、4096 字节页、16 字节 tag）。错钥打开返回 `StorageError::InvalidKey`。
  - 回填在页闭包外装配 overflow，再用 `deserialize_row` 和 `table_meta.defaults`。溢出页的现成规模是 5000 字节字符串，见 `tests/overflow_page_test.rs`。
  - `tests/fixtures/legacy/legacy_g1.db` 的 `users` 为 `id INT`、`name` 字符串、`score FLOAT`。三行是 `(1, alice, 9.5)`、`(2, bob, 7.25)`、`(3, carol, 8.0)`。样本文件只复制，不修改。
  - 规划时 `tests/*.rs` 有 153 个。T7 新增的文件不在这 153 个里。T8 以当时目录重新点数。
- Code and Critical Path：
  - `Database::execute_sql`、`execute_in_tx`、`begin`、`commit`、`rollback`、`close`、`checkpoint`、`open`、`open_with_isolation`、`open_readonly`、`open_in_memory`、`open_with_key`、`reclaim_abandoned_implicit_transactions`、`implicit_tx_guard`。
  - `pipeline::execute_stage_routed` 的 DML 臂与 `readonly_rejection`。
  - `InsertExecutor` 的二级循环，`TransactionManager::abort` / `abort_by_id`，`recovery::extract_index_keys`。
  - `CreateIndexExecutor::backfill_tree` 的 overflow 装配和 DEFAULT overlay。
  - CLI `rtsql`，裸名对应 `$RTSQL_HOME/db/<name>.db`。`ATTACH '<名>' AS <别名>` 只在 CLI 进程内有效。
  - `storage::integrity::check_integrity` 提供 error 计数。它是完整性观察，不代替 SQL 结果。

**Implementation Guidance**

T7 先写 `tests/string_index_runtime_test.rs` 并运行。编译通过后的失败才是产品差距。第一次编译通过且全部断言通过时，不改产品代码。某个断言失败时，只修该断言指向的现有路径。

T8 在 T7 的定向结果为 0 failed 之后开始。先完成本变更文件的格式与新 lint，再改两份 README，然后跑唯一一次全量，最后跑全局 fmt/clippy 和 OpenSpec 检查。

取消夹具沿用 `tests/statement_cancellation_test.rs`：`yield_now` 轮询版本数，10 秒只是夹具失败上限。20 毫秒只用于触发超时。断言看行、等值结果、错误文本和 `error_count`。

**Task Definitions**

### T7: 故障、恢复与运行形态有 SQL 或 CLI 证据

- Requirement/Scenario: R4 的取消、超时和失败；R5 的干净重开、未决 WAL、旧库，以及 S14 的内存、只读、加密、附加库 DML；R2/R3 的 DEFAULT 与溢出页组合。S09、S10、S11、S14。
- Depends on: T6 accepted
- Targets:
  - `tests/string_index_runtime_test.rs`（新文件）
  - 仅当对应断言失败时：`src/executor/insert.rs`、`src/executor/update.rs`、`src/executor/delete.rs`、`src/executor/upsert.rs`、`src/executor/create_index.rs`、`src/wal/recovery.rs`、`src/transaction/manager.rs`、`src/pipeline.rs` 中已经调用 `secondary_index_key` 或 `reclaim_abandoned_implicit_transactions` 的现有臂
- Current behavior：SQL 可以创建字符串索引并做等值查询。存储层测试已经覆盖执行器建索引后的重开、未决 WAL、旧库、回滚和失败主键。取消、超时、内存、只读查询、加密和附加库 DML 还没有字符串索引的 SQL/CLI 证据。`CREATE INDEX` 不进入隐式事务回收表。
- Required behavior：新文件里的每个测试都通过。观察点如下。
  - `cancelled_string_index_write_is_reclaimed`：`Database::open`。`src(id INT PRIMARY KEY, name VARCHAR)` 插入 `1..=2000`，`name` 为该整数的十进制文本。`t2` 同结构，`CREATE INDEX idx_name ON t2(name)`。放弃 `INSERT INTO t2 SELECT id, name FROM src` 的 future。放弃点是 `t2` 上该事务的版本数不少于 2，并且语句尚未结束。丢弃 future 后，下一条 `INSERT INTO t2 VALUES (1, '1')` 成功，接着 `INSERT INTO t2 VALUES (2, '2')` 成功。`WHERE name = '1'` 与 `WHERE name = '2'` 各返回这一行。`WHERE name = '3'` 返回 0 行。`SELECT COUNT(*) FROM t2` 为 2。`check_integrity` 的 `error_count()` 为 0。活跃事务集合为空。
  - `timed_out_implicit_string_index_write_is_reclaimed`：同一规模。另起观察任务，在 10 秒内看到版本数不少于 2。主 future 用 `tokio::time::timeout(Duration::from_millis(20), execute_sql(...))`。结果必须是超时。观察任务必须在超时前看到锚点。随后同样能插入 `(1,'1')` 和 `(2,'2')`，`name = '3'` 为 0 行，`COUNT(*)` 为 2，完整性 error 数为 0。语句若在 20 毫秒内跑完，这是夹具失败，返回 Plan。
  - `explicit_tx_timeout_keeps_string_index_until_caller_ends_tx`：`open_with_isolation(..., ReadCommitted)`。同样的 `src` 与已建索引的 `t2`。`begin` 后对 `execute_in_tx` 使用 20 毫秒超时，观察任务确认超时前版本数不少于 2。超时后该事务 id 仍在活跃集。事务内 `SELECT id FROM t2` 非空，事务外为空。`rollback` 后事务外仍为空，再插入 `(1,'1')` 与 `(2,'2')` 成功。另一次超时后改为 `commit`：`WHERE name = '1'` 至少返回一行，`close` 后重开仍能返回该行。
  - `undelivered_commit_keeps_string_index_row`：普通 `open`。表上已有字符串索引。显式事务插入 `(1, 'kept')`。`set_checkpoint_bytes_threshold(1)` 在 `commit` 之前。commit future 在事务 id 离开活跃集时被丢弃，且丢弃时 commit future 尚未返回。丢弃后 `WHERE name = 'kept'` 返回该行，活跃集为空，下一条插入成功。`close` 后重开，该行仍在。commit future 若先返回，这是夹具失败，返回 Plan。
  - `failed_mid_insert_restores_string_index`：`t(id INT PRIMARY KEY, name VARCHAR)` 已有 `(1, 'keep')` 和 `idx_name`。`src(id INT, name VARCHAR)` 有 `(2, 'new')` 与 `(1, 'dup')`。`INSERT INTO t SELECT id, name FROM src ORDER BY id DESC` 返回 `Err`，`message` 为 `Execution error: Duplicate key`，`sqlstate` 为 `None`。之后 `SELECT * FROM t` 只有 `(1, 'keep')`。`WHERE name = 'new'` 为 0 行。`WHERE name = 'keep'` 返回原行，EXPLAIN 含 `IndexScanAll` 和 `index=idx_name`。完整性 error 数为 0。随后 `INSERT INTO t VALUES (2, 'new')` 成功，`WHERE name = 'new'` 返回该行。`INSERT SELECT` 若拒绝 `ORDER BY`，返回 Plan。
  - `sql_string_index_clean_reopen_and_pending_wal`：`execute_sql` 建表并插入 `(1, 'alpha')`、`(2, '你好')`，`CREATE INDEX idx_name ON t(name)`。`close` 后重新 `open`。两个等值各返回一行，EXPLAIN 含 `IndexScanAll` 和 `index=idx_name`，完整性 error 数为 0。再插入 `(3, 'late')`，调用 `wal_buffer.shutdown()`，丢弃 `Database` 且不 `close`。再次 `open` 后三个等值各返回一行，完整性 error 数为 0。
  - `legacy_sql_string_index_keeps_existing_rows`：把 `tests/fixtures/legacy/legacy_g1.db` 复制到临时目录后 `open`。`SELECT * FROM users` 仍是 alice、bob、carol 三行。`CREATE INDEX idx_name ON users(name)` 成功。`WHERE name = 'alice'` 返回 id 1 和 score 9.5，EXPLAIN 含 `IndexScanAll`。`WHERE id = 1` 返回同一行。完整性 error 数为 0。仓库里的样本文件保持原样。
  - `memory_string_index_creates_no_files`：`open_in_memory`。建表、插入 `(1, 'mem')`、`CREATE INDEX idx_name ON t(name)`。`WHERE name = 'mem'` 返回该行，EXPLAIN 含 `IndexScanAll`。`wal_writer`、`wal_buffer`、`checkpoint_manager` 都是 `None`，`path()` 为 `":memory:"`。进程当前目录里以 `:memory:` 开头的条目，在会话前后的集合相同。
  - `readonly_session_reads_string_index`：读写会话建好索引和 `(1, 'ro')` 后 `close`。`open_readonly` 上 `WHERE name = 'ro'` 返回该行，EXPLAIN 含 `IndexScanAll` 和 `index=idx_name`。再建索引返回 `25006`，消息含 `cannot execute CREATE INDEX in a read-only session`。`INSERT` 返回 `25006`，消息含 `cannot execute INSERT in a read-only session`。重新读写打开后，原行仍在，`idx_name` 仍是该表唯一的二级索引。
  - `encrypted_string_index_keeps_page_records`：`open_with_key(..., RepeatableRead, Some("sidx-key"))`。插入 `(1, 'sidx-secret')` 并建 `idx_name`。`close` 后文件长度大于 64，且 `(长度 - 64) % 4124 == 0`。文件字节中没有连续的 `sidx-secret`。用 `"wrong"` 打开返回 `StorageError::InvalidKey`。再用 `"sidx-key"` 打开，`WHERE name = 'sidx-secret'` 返回该行，EXPLAIN 含 `IndexScanAll`。
  - `attached_dml_maintains_string_index`：临时目录作为 `RTSQL_HOME`，其下 `db/app.db` 与 `db/o.db` 由 `Database::open` 创建。`o.db` 中 `u(id INT PRIMARY KEY, name VARCHAR)` 有 `(1, 'alpha')`、`(2, 'beta')` 和 `idx_name`。两个会话都先 `close`。之后每条 CLI 使用 `CARGO_BIN_EXE_rtsql`，工作目录和 `RTSQL_HOME` 都是该临时目录，去掉 `RTSQL_KEY`。参数顺序是库名、SQL、`--format`、`json`。`app "ATTACH 'o' AS o; INSERT INTO o.u VALUES (5, 'routed')" --format json` 退出码 0。同样方式把 id 1 的 name 改为 `moved`，并删除 id 2。查询 stdout 里含有结果行的那一段包含 `5` 和 `routed`。多语句时 stdout 可以有多行，不要求整段 stdout 是一个 JSON 值。`name = 'beta'` 的查询结果不含 beta 行。`name = 'moved'` 的结果含 id 1。然后 `Database::open` 这个 `o.db`，三条等值与 CLI 一致，`moved` 与 `routed` 的 EXPLAIN 含 `IndexScanAll` 和 `index=idx_name`。CLI `ATTACH 'o' AS o; CREATE INDEX idx_new ON o.u(name)` 退出码 3，stderr 含 `WRITE TO ATTACHED DATABASE`。再次打开 `o.db`，二级索引仍只有 `idx_name`。
  - `default_and_overflow_row_hits_string_index`：`t(id INT PRIMARY KEY, name VARCHAR DEFAULT 'def', body TEXT)`。插入时省略 `name`，`body` 为 5000 个 `x`。然后 `CREATE INDEX idx_name ON t(name)`。`WHERE name = 'def'` 返回该行，`body` 长度 5000，EXPLAIN 含 `IndexScanAll` 和 `index=idx_name`。建索引后再省略 `name` 插入 5000 个 `y`，`WHERE name = 'def'` 返回两行。另插入 `(3, '<5000 个 z>', 'short')` 到被索引列。该行 `SELECT *` 可读。`WHERE name = '<这 5000 个 z>'` 的 EXPLAIN 含 `DataScan` 且不含 `IndexScanAll`，并只返回这一行。`WHERE name = 'def'` 仍不返回它。
- Preserve：`25006`、`0A000`、`InvalidKey`、`Execution error: Duplicate key` 的现有文本。整数二级索引、主键和 UNIQUE 的整数限制。附加库索引 DDL 拒绝。内存库无文件。加密记录 4124 字节。32 字节键和长值扫描。002 的八个 SQL 测试和 001 的六个存储层测试仍然能通过。
- Forbidden：新增取消或 deadline API。给 `CREATE INDEX` 加隐式事务回收。放行附加库 `CREATE INDEX`。修改 `tests/fixtures/legacy/` 里的样本。把评估原型编进 `src/`。改 `Key` 宽度、`node.rs` 条目宽度、`file_header.rs`。清理无关工作区差异。在 T7 运行全量、全局 `cargo fmt --check` 或全局 clippy。处理 ISS36。修改 002 的 Cycle 文件。
- First test：`cancelled_string_index_write_is_reclaimed`。先添加文件并运行下面的命令。预期是新测试被编译并执行。产品路径已经接上 `secondary_index_key` 时，该测试可以第一次运行就通过。断言失败时，失败点应是被放弃插入留下的字符串键或可见行。
- GREEN condition：`cargo test --offline --test string_index_runtime_test` 的 failed 为 0。若 T7 改了产品代码，再运行 `string_secondary_index_test` 与 `string_index_sql_test`，两者 failed 也为 0。
- Verification：定向测试用默认并行度。命令是 `cargo test --offline --test string_index_runtime_test`。产品代码有改动时加上上述两个既有文件。不传 `--test-threads=1`，除非第一次失败被怀疑为并行冲突；那种情况只复核一次，并同时记下两次结果。不跑无目标过滤的全量。
- Stop when：要通过测试就必须新增取消 API、改变 `25006` / `0A000` / `InvalidKey`、放行附加库索引 DDL、截断长字符串，或让 `CREATE INDEX` 进入回收表。`INSERT SELECT` 不接受 `ORDER BY`。20 毫秒超时窗口里 2000 行语句已经结束。返回 Plan。

### T8: 一次全量、文档与收尾检查

- Requirement/Scenario: R5、R6。用户可见的类型与长值回落。S02 的测量不重跑。
- Depends on: T7
- Targets:
  - `README.md` 与 `README.zh-CN.md` 的二级索引能力句
  - 本变更新增或修改过的文件：`benches/index_key_encoding_bench.rs`，`tests/index_key_encoding_evaluation_test.rs`，`tests/support/index_key_encoding.rs`，`tests/string_secondary_index_test.rs`，`tests/string_index_sql_test.rs`，`tests/string_index_runtime_test.rs`，`tests/secondary_index_test.rs`，`src/executor/value.rs`，`src/executor/create_index.rs`，`src/executor/insert.rs`，`src/executor/update.rs`，`src/executor/delete.rs`，`src/executor/upsert.rs`，`src/storage/data/table_manager.rs`，`src/wal/recovery.rs`，`src/storage/btree/index_manager.rs`，`src/parser/planner/ddl_dml.rs`，`src/parser/planner/query.rs`
- Current behavior：两份 README 的存储条只写 `CREATE INDEX` 服务等值且结果与全表扫描一致，没有写字符串声明，也没有写超过键容量时扫描完整字面量。ISS36 仍 open：全局 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 在基线上失败。`table_manager.rs` 属于 ISS36 的既有 fmt 文件，`ddl_dml.rs` 属于 ISS36 的既有 clippy 文件；这两个文件同时也被本变更改过。
- Required behavior：
  - 两份 README 的同一条能力写明：单列非唯一 `CREATE INDEX` 接受 INTEGER 和字符串声明（TEXT、VARCHAR、CHAR、CLOB）。等值结果与全表扫描一致。字符串在完整键编码放得进 32 字节时走索引。更长的字符串整值写入，等值查询扫描该完整字面量。即使该行因为其他列进入溢出页，这条回落也不变。不截断，不给列加长度上限。NULL 不入索引。重复字符串允许。PRIMARY KEY 与 UNIQUE 仍只接受整数。
  - 全量只跑一次。先执行 `ls tests/*.rs`，去掉目录和 `.rs`，得到集成测试名。再加入目标名 `lib` 和 `bins`。`LC_ALL=C sort` 得到唯一顺序列表。`string_index_runtime_test` 必须在列表里。`N` 为列表长度，`B = ceil(N / 12)`。前 `B * 12 - N` 批各 11 个目标，其余各 12 个。每一批目标数都在 8 到 12 之间。按顺序每一批一条命令，等该批退出后再开始下一批。命令形如 `cargo test --offline`，后接该批里的 `--lib`、`--bins` 和每个 `--test <名>`，最后是 `-- --test-threads=1`。不使用 `--tests`。不使用没有目标过滤的 `cargo test`。不使用默认并行度。每一批记录退出码，以及每个目标的 `test result:` 汇总中的 passed 与 failed。`network_server_test` 或 bin 出现 0 passed 时，只要该目标跑过且 failed 为 0，该批仍可通过。任一批 failed 不为 0 时，把该批原命令复核一次。复核通过则记下首跑失败。复核仍失败则不再启动后续批次。已完成批次和未跑批次都写明。范围外的失败作为 Issue 候选写入 Act Response，不在本 Cycle 改那些产品代码，也不登记 Issue。
  - 全量之前，对本变更文件做格式和 lint。除 `table_manager.rs` 外，`cargo fmt --check -- <那些文件>` 必须通过。失败时只格式化这些文件并再查一次。`table_manager.rs` 若被 `cargo fmt --check` 点名，只处理与本变更的 `secondary_index_key` 编辑重叠的差异。文件其余部分的既有差异保持不动，不对这个文件执行整文件 `cargo fmt`。`cargo clippy --offline --all-targets` 不加 `-D warnings`，完成全部目标后，本变更新文件中的 warning 必须为 0。`ddl_dml.rs` 里落在本变更编辑上的 warning 必须消除。落在该文件其余位置、且属于 ISS36 已记录范围的 warning 保留。
  - 全量之后各跑一次 `cargo fmt --check` 和 `cargo clippy --offline --all-targets -- -D warnings`。记下退出码和输出点名的文件。这两个命令失败时，Response 改为 `blocked`，原样保留失败，不写全局门通过，不修改 ISS36 基线文件，不再跑全量。若失败点落在本变更的新文件或新编辑上，同样 `blocked` 并返回 Plan，不在全局失败之后继续改代码。
  - `openspec validate 2026-10-27-ms38-index-key-encoding-and-string-index --strict --no-interactive` 的输出含 `is valid`。`openspec status --change 2026-10-27-ms38-index-key-encoding-and-string-index` 显示规划产物齐全。这两条在 README 修改之后运行。
- Preserve：README 里主键、UNIQUE、加密记录布局、溢出页和退出码的既有句子。ISS36 的处置不在本变更里改变。000 的 Criterion 结果不重跑。
- Forbidden：一次无过滤全量。默认并行度的全量批次。缺批仍声明全量通过。为了全局门通过去格式化或修改 ISS36 基线文件。把本变更新文件的 fmt/clippy 问题记成 ISS36。第二轮全量。修改 `openspec/specs/` 主规格、SNAPSHOT 或 `.agents/docs/tasks.md`。
- First test：T8 没有新的产品行为测试。前置状态是 T7 的 `string_index_runtime_test` 已经 0 failed。文档和命令本身就是验证。
- GREEN condition：全量每一批都已运行且 failed 为 0。两份 README 含上述类型和长值句子。本变更新文件没有新的 fmt/clippy 问题。OpenSpec 两条命令成功。全局 fmt/clippy 要么通过，要么已按上面的规则写成 `blocked`。
- Verification：全量命令和计数写入 Act Response。只保留每批的退出码、目标名和 passed/failed。不保存完整测试输出。OpenSpec 两条命令的结论各留一行。
- Stop when：批次大小落在 8 到 12 之外，或列表缺 `string_index_runtime_test`。全量复核后仍有 failed。全局门失败。这些情况都停止并返回，不另开 Iteration。

**Invariants**

- 键仍是 32 字节。叶条目 38 字节，内部分隔条目 36 字节。页类型仍是 `0x01` / `0x02`。
- `Value::to_key` 仍只接受整数。二级字符串只经 `secondary_index_key`。
- 非零原文超过 30 字节，或转义后超过 32 字节，不入树。查询走完整字面量扫描。
- 取消只来自 future 不再被驱动。隐式 DML、CTAS 和 `ALTER TABLE DROP COLUMN` 的回收范围不变。显式事务由调用方收尾。
- 附加库 DML 使用目标库自己的表和事务管理器。附加库索引 DDL 继续拒绝。
- 只读会话零写入。内存会话零文件。加密页记录步长 4124 字节。
- 全量遵守 M19：分批、批间串行、每批 `--test-threads=1`。定向测试可以用默认并行度。
- 工作区里 000 的原型、001/002 的产品改动和更早的未提交差异都保留。

**Non-goals**

字符串主键和唯一约束。范围、参数、限定列和组合谓词的新路由。性能优化。Criterion。评估原型的删除。ISS36 的全库重排或工具链固定。新的 Issue 登记。主规格合并。

**Acceptance**

- A1 / R4 / S09 / T7：隐式语句被丢弃或超时后，下一次写回收该语句。被放弃的字符串键可以再次插入，未再插入的值不可见，完整性 error 数为 0。
- A2 / R4 / S09 / T7：显式事务里的超时不提交也不回滚。调用方回滚后键释放。调用方提交后，重开仍能按字符串等值读到该行。
- A3 / R4 / S09 / T7：commit 已经离开活跃集但结果未送达时，字符串等值仍返回该行，重开后还在。
- A4 / R4 / S08 / T7：后行主键冲突使整条插入失败。已提交的 `keep` 仍在，`new` 不可见。冲突文本为 `Execution error: Duplicate key`。之后可以插入 `new`。
- A5 / R5 / S10 / S11 / T7：`execute_sql` 建好的索引在 `close` 重开后可用。不 `close`、只关闭 WAL buffer 再打开，已提交的 `late` 可用。两条路径的完整性 error 数都为 0。
- A6 / R5 / T7：旧库副本接受 SQL `CREATE INDEX`。alice、bob、carol 和原来的整数主键查询都在。样本文件不变。
- A7 / S14 / T7：内存会话能建字符串索引并等值查询。没有 WAL 和 checkpoint 通道，当前目录不新增 `:memory:` 文件。
- A8 / S14 / T7：只读会话能查询已有字符串索引。建索引和插入仍是 `25006`。
- A9 / S14 / T7：加密库的字符串等值在同一密钥下重开可用。记录步长仍是 4124 字节。页字节不含 `sidx-secret`。错钥为 `InvalidKey`。
- A10 / S14 / T7：CLI 附加库 INSERT、UPDATE、DELETE 后，等值结果与直接打开该文件一致。对附加表建索引退出码 3，原索引仍是唯一二级索引。
- A11 / R2 / R3 / T7：默认值 `def` 与 5000 字节溢出列同时存在时，回填和后续插入都走 `idx_name`。被索引列本身为 5000 字节时走 `DataScan`，行仍返回。
- A12 / R6 / T8：全量每一批 failed 为 0，批数和目标覆盖与现场列表一致。缺批则 A12 未满足。
- A13 / T8：两份 README 写明字符串类型和超容量扫描。OpenSpec 检查成功。本变更新文件没有新的格式或 lint 问题。全局门的实际结果已记录。全局门失败时本 Cycle 为 `blocked`，A13 的文档和 OpenSpec 部分仍可已完成。

**Verification**

T7 的每个测试都调用 `Database::execute_sql`，附加库场景再调用 CLI。通过条件是上列可观察结果，不是内部计数器。

T8 的全量以每批命令的退出码和 `test result:` 汇总为准。格式、lint 和 OpenSpec 以对应命令的退出码和点名文件为准。

**Gate 2 Readiness**

PASS。检查项没有 BLOCKED 或 SKIPPED。计划批准来自本次执行 goal，不是跳过。

**Persisted Evidence**

- Mode: none

各批 passed/failed、README 改动和命令结论写入 Act Response 即可。定向测试和全量都可以按同一命令重跑。不创建 evidence 目录，不保存完整测试输出。

**Risks and Notes**

- 20 毫秒和 2000 行沿用既有取消夹具。它们只决定放弃发生在语句结束前。机器若让语句提前结束，返回 Plan，不把行数加到通过为止。
- `table_manager.rs` 与 `ddl_dml.rs` 同时属于本变更和 ISS36。整文件格式化或整文件清 lint 会把旧问题卷进来。
- 全量失败若落在本变更没有修改的测试上，只作为 Issue 候选报告。当前没有新登记的 Issue，不为此建 Iteration。
- 002 的两条非阻塞发现不在本 Cycle 修复。

## Act Response

- Status: blocked

**Implemented**

T7 没有改产品路径。`tests/string_index_runtime_test.rs` 的 12 个测试第一次完整运行就是 12 passed / 0 failed。取消、超时、显式事务、未送达提交、中途主键冲突、干净重开、未决 WAL、旧库、内存、只读、加密、附加库 DML，以及默认值加溢出页，都走 `Database::execute_sql`；附加库另走 `rtsql`。

本轮改动的文件：

- `tests/string_index_runtime_test.rs`（新文件）
- `src/parser/planner/query.rs`：只把 `PhysicalPlan` 与 `ProjectionItem` 的 import 收成 rustfmt 要求的换行
- `tests/support/index_key_encoding.rs`：两处取模改成 `is_multiple_of`
- `benches/index_key_encoding_bench.rs`：相邻页判断改成 `matches!`
- `tests/index_key_encoding_evaluation_test.rs`：大端序改用 `sort_by_key`，超宽记录改用 `std::slice::from_ref`，去掉恒真的 `assert!(-1 < 1)`
- `README.md` 与 `README.zh-CN.md`：存储条里的二级索引句写明字符串声明、32 字节内走索引、超长整值扫描、溢出页回落不变、不截断、NULL 不入索引、重复字符串允许，以及 PRIMARY KEY / UNIQUE 仍只接受整数

未改 `insert.rs`、`update.rs`、`delete.rs`、`upsert.rs`、`create_index.rs`、`recovery.rs`、`manager.rs`、`pipeline.rs`、`value.rs`、`index_manager.rs`、`ddl_dml.rs`、`table_manager.rs`。未创建下一 Cycle 或下一 Iteration。

**Deviations from Plan**

- `explicit_tx_timeout_keeps_string_index_until_caller_ends_tx` 的提交半程写在空表 `t3` 和 `idx_t3_name` 上。回滚半程已经把 `(1,'1')` 和 `(2,'2')` 重新插入 `t2`，索引名又是库级唯一；同一条 `INSERT SELECT` 若再写 `t2`，会在版本锚点前撞上主键。超时、事务仍活跃、事务内可见、事务外为空、回滚后再插入成功、提交后 `name = '1'` 至少一行、重开后仍在，这些观察都在。
- `table_manager.rs` 的 rustfmt 差异只有第 729 行 `create_table_with_constraints(...).await` 的换行，和 `secondary_index_key` 的编辑不重叠，文件保持原样。
- `ddl_dml.rs:1482` 的 collapsible-if 不在本变更编辑（约 1953 行）上，保留。`table_manager.rs:978` 的 `too_many_arguments` 在 `alter_add_column`，不在 `secondary_index_key` 编辑上，保留。

**Self-review**

Gate 3：12 个测试覆盖 T7 列出的观察点。先行测试是新文件本身；产品路径已接上时第一次运行即为 GREEN，没有为了通过去改存储或执行器。没有遗留的 Critical 或 Important 问题。

T8 的文档句和 OpenSpec 检查已完成。全局 fmt/clippy 按任务停在失败结果上，T8 的 Gate 4 因此不通过。

**Verification**

T7，格式化新测试之后，默认并行度：

```sh
cargo test --offline --test string_index_runtime_test
```

12 passed / 0 failed。未改产品代码，所以没有加跑 `string_secondary_index_test` 和 `string_index_sql_test`。未在 T7 跑全量、全局 fmt 或全局 clippy。

本变更文件的 rustfmt：除 `table_manager.rs` 外，列出的文件 `rustfmt --edition 2021 --check` 通过。`table_manager.rs` 仍被点名，差异不重叠 `secondary_index_key`，未整文件格式化。

`cargo clippy --offline --all-targets`（不加 `-D warnings`）退出码 0。本变更新文件的 warning 为 0。`ddl_dml.rs` 只剩 `1482` 那一条，不在本变更编辑上。

评估文件的 lint 修改之后：

```sh
cargo test --offline --test index_key_encoding_evaluation_test
```

7 passed / 0 failed。

T8 全量。`ls tests/*.rs` 去掉目录和 `.rs`，加上 `lib` 和 `bins`，`LC_ALL=C` 字典序。`N=156`，`B=13`，`B*12-N=0`，13 批各 12 个目标。`string_index_runtime_test` 在第 11 批。每批 `cargo test --offline` 按该批顺序接 `--lib` / `--bins` / `--test <名>`，最后 `-- --test-threads=1`。批间串行。没有无过滤全量，没有默认并行度。没有复核，因为每批第一次 failed 就是 0。13 批都已运行。计数是 passed/failed。

| 批 | 退出码 | 目标 passed/failed |
|---|---:|---|
| 1 | 0 | aggregate_extension_test 39/0；aggregate_output_shape_test 8/0；aggregate_test 19/0；alter_table_test 16/0；attach_session_test 11/0；bins 0/0；btree_merge_test 10/0；btree_scale_test 5/0；btree_split_test 12/0；btree_test 10/0；busy_timeout_test 18/0；cache_perf_test 1/0 |
| 2 | 0 | catalog_bounded_read_test 5/0；catalog_durability_test 4/0；chain_join_test 9/0；check_command_test 11/0；checkpoint_command_test 6/0；checkpoint_redo_reduction_test 9/0；checkpoint_skip_test 6/0；checkpoint_test 2/0；cli_test 126/0；concurrent_test 10/0；connection_limit_test 3/0；constraint_enforcement_test 26/0 |
| 3 | 0 | cross_db_dml_test 17/0；cross_db_query_test 21/0；ctas_test 17/0；cte_test 14/0；data_page_bounded_read_test 8/0；database_file_lock_test 8/0；datetime_function_test 19/0；datetime_type_test 16/0；ddl_catalog_transactional_test 3/0；ddl_crash_recovery_test 6/0；ddl_layout_publication_test 11/0；ddl_rewrite_consistency_test 9/0 |
| 4 | 0 | default_declaration_test 14/0；default_precheck_test 5/0；derived_scan_alias_test 26/0；dml_tombstone_abandon_window_test 3/0；dml_tx_id_test 6/0；drop_persistence_test 11/0；drop_table_free_test 6/0；e2e_test 27/0；encryption_test 7/0；executor_test 39/0；explain_test 10/0；explicit_tx_test 21/0 |
| 5 | 0 | expression_e2e_test 24/0；file_header_test 14/0；file_storage_io_test 4/0；free_page_lifecycle_test 12/0；full_table_dml_test 11/0；gc_test 3/0；group_by_expr_test 12/0；hash_join_build_side_test 3/0；import_input_surface_test 12/0；index_key_encoding_evaluation_test 7/0；index_manager_test 3/0；index_page_ownership_stack_overflow_test 1/0 |
| 6 | 0 | index_page_ownership_test 5/0；index_read_bounded_read_test 7/0；index_write_atomicity_test 5/0；insert_column_list_test 7/0；insert_select_test 10/0；isolation_level_test 9/0；join_output_resolution_test 19/0；join_test 9/0；key_type_conformance_test 8/0；keyless_eq_routing_test 14/0；keyless_row_test 4/0；legacy_compat_test 16/0 |
| 7 | 0 | lib 428/0；limit_test 5/0；memory_db_test 15/0；mvcc_abort_test 3/0；mvcc_commit_test 4/0；mvcc_record_test 5/0；mvcc_tombstone_visibility_test 11/0；nested_loop_join_test 9/0；network_protocol_test 5/0；network_server_test 0/0；no_from_select_test 9/0；order_by_alias_sort_test 7/0 |
| 8 | 0 | outer_join_test 16/0；overflow_page_test 27/0；param_binding_test 28/0；parser_test 6/0；pg_integration_test 6/0；pg_messages_test 9/0；pg_protocol_test 11/0；pipeline_test 17/0；plan_cache_test 7/0；plan_exec_test 4/0；planner_test 107/0；predicate_test 23/0 |
| 9 | 0 | prefetch_test 3/0；prepared_statement_test 14/0；projection_expression_test 16/0；projection_test 6/0；pushdown_test 16/0；query_memory_test 12/0；readonly_session_test 15/0；recovery_bounded_read_test 6/0；recovery_e2e_test 6/0；recovery_error_surface_test 3/0；recovery_replay_convergence_test 3/0；recovery_tail_persistence_test 4/0 |
| 10 | 0 | recovery_test 3/0；recovery_wal_convergence_test 5/0；release_durability_precondition_test 2/0；release_path_bounded_read_test 9/0；runtime_test 3/0；scalar_function_test 32/0；scalar_subquery_comparison_test 16/0；schema_persistence_test 8/0；secondary_index_equality_test 3/0；secondary_index_test 27/0；select_distinct_test 12/0；self_join_test 6/0 |
| 11 | 0 | set_operations_test 21/0；sort_merge_join_test 15/0；sort_test 8/0；sql_differential_test 10/0；statement_cancellation_test 8/0；storage_test 21/0；string_index_runtime_test 12/0；string_index_sql_test 8/0；string_secondary_index_test 6/0；structural_property_test 5/0；subquery_alias_binding_test 25/0；subquery_test 46/0 |
| 12 | 0 | subset_insert_test 6/0；sync_loader_test 2/0；table_alias_resolution_test 19/0；table_manager_test 6/0；truncate_test 7/0；tx_statement_test 21/0；typed_api_test 17/0；update_expression_test 14/0；update_index_maintenance_test 11/0；upsert_test 32/0；value_test 19/0；version_chain_test 3/0 |
| 13 | 0 | view_test 19/0；visibility_test 5/0；wal_auto_checkpoint_test 11/0；wal_buffer_test 4/0；wal_handle_test 4/0；wal_integration_test 2/0；wal_record_test 14/0；wal_recovery_large_test 7/0；wal_writer_test 4/0；write_failure_state_test 3/0；write_path_slot_bounds_test 11/0；write_type_conformance_test 13/0 |

`bins` 和 `network_server_test` 是 0 passed / 0 failed，目标都跑过。156 个目标各有一条 `test result:`，没有缺批。

全局门：

```sh
cargo fmt --check
```

退出码 1。点名文件：`src/storage/data/table_manager.rs`、`tests/hash_join_build_side_test.rs`、`tests/sort_merge_join_test.rs`。

```sh
cargo clippy --offline --all-targets -- -D warnings
```

退出码 101。`could not compile rtsql (lib) due to 20 previous errors`，lib test 同样 20 条。点名文件：`src/pipeline.rs`、`src/cli/lifecycle.rs`、`src/executor/alter_table.rs`、`src/executor/executor_trait.rs`、`src/executor/sort_merge_join.rs`、`src/network/protocol.rs`、`src/parser/planner/ddl_dml.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`、`src/storage/data/table_manager.rs`、`src/transaction/manager.rs`。deny 在 lib 停止，测试目标没有被这条命令 lint。

OpenSpec：

```sh
openspec validate 2026-10-27-ms38-index-key-encoding-and-string-index --strict --no-interactive
```

`Change '2026-10-27-ms38-index-key-encoding-and-string-index' is valid`。退出码 0。

```sh
openspec status --change 2026-10-27-ms38-index-key-encoding-and-string-index
```

`Progress: 4/4 artifacts complete`，proposal、specs、design、tasks 都是完成。`All planning artifacts complete!`。退出码 0。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Unresolved**

全局 fmt 与 deny clippy 仍失败。本变更新文件没有被这两条命令点名。`ddl_dml.rs:1482` 与 `table_manager.rs:978` 留在原处。没有当前 Cycle 范围外的新实质缺陷要作为 Issue 候选。

**Blocker Handoff**

- 位置：T8，全量之后的全局门。任务定义的停止条件命中。T8 的 Gate 4 不通过。
- 计划预期：这两条全局命令要么通过，要么失败后把 Response 写成 blocked，保留失败，不改 ISS36 基线，不再跑全量。
- 实际情况：fmt 退出码 1，clippy deny 退出码 101。失败点不在 `string_index_runtime_test.rs`，也不在 `secondary_index_key` 编辑上。
- 影响：A12 已满足。A13 的文档和 OpenSpec 已完成。全局门没有通过，T8 不能标完成。本 Cycle 不另开 Iteration。
- 已完成：T7。T8 里的变更文件格式与新文件 lint、两份 README、13 批全量、OpenSpec。
- 部分完成：T8 的全局门，结果已记录，门未通过。
- 未开始：无。没有后继 Cycle。
- 已通过的 Gate：T7 的 Gate 3 和 Gate 4。T8 在全局门之前的检查已完成。
- Evidence：None required。
- 恢复条件：由 `openspec-plan` Review 本 Cycle。本 Cycle 不再跑第二轮全量，也不清理上面点名的全局门文件。

## Plan Review

- Review Result: replan-required

**Findings**

A1–A12 成立。A13 里的两份 README、OpenSpec 检查，以及新编辑上的格式和 lint 也成立。按本 Cycle 原来的 A13，全局门失败仍使 Act Response 保持 blocked，T8 不能标完成，本 Cycle 不能 accepted。

本审查没有重跑测试。沿用本 Cycle Act Response 的命令结果。覆盖这些结果的文件，修改时间都早于本文件在 17:24 的审查记录：`string_index_runtime_test.rs`、两份 README、`query.rs` 和评估文件不晚于 16:57，`ddl_dml.rs` 是 16:03，`table_manager.rs` 是 15:26。

本次另读了工作区。`tests/*.rs` 仍是 154 个，加上 `lib` 和 `bins` 为 156。未暂存 diff 只有两份 README、`query.rs`、`index_key_encoding_bench.rs`、`index_key_encoding_evaluation_test.rs` 和 `tests/support/index_key_encoding.rs`。`string_index_runtime_test.rs` 是未跟踪新文件，里面有 T7 点名的 12 个测试。`insert.rs`、`update.rs`、`delete.rs`、`upsert.rs`、`create_index.rs`、`recovery.rs`、`transaction/manager.rs`、`pipeline.rs`、`value.rs`、`index_manager.rs`、`ddl_dml.rs` 和 `table_manager.rs` 没有未暂存 diff。

`query.rs` 的未暂存 diff 只调整 `PhysicalPlan` 和 `ProjectionItem` 的 import 换行。评估文件是 `matches!`、`sort_by_key`、`std::slice::from_ref`、`is_multiple_of`，并删除 `assert!(-1 < 1)`。两份 README 的第 263 行写明字符串声明、32 字节内走索引、超长整值扫描、溢出页回落不变、不截断、NULL 不入索引、重复字符串允许，以及 PRIMARY KEY 与 UNIQUE 仍只接受整数。

`table_manager.rs` 第 732 行是 `create_table_with_constraints(...).await`，第 1358 行和第 1464 行才是 `secondary_index_key`。本 Cycle 早先的审查跑过 `rustfmt --edition 2021 --check src/storage/data/table_manager.rs`，退出码 1，只报这一处换行，行号写成 729。该文件此后没有改，本次不重跑 rustfmt。`ddl_dml.rs` 第 1482 行是 ADD COLUMN 默认值分支的 `if default_value.is_none()`。类型门在 `build_create_index` 第 1953–1969 行。失败点不在本变更的新编辑上。`.agents/issues/ISS36-fmt-clippy-gates-unsatisfiable.md` 的 Status 仍是 open。

`explicit_tx_timeout_keeps_string_index_until_caller_ends_tx` 的提交半程使用空表 `t3`。A2 的观察都在。非阻塞。Unresolved 说新文件没有被全局命令点名；命令确实点了 `table_manager.rs` 和 `ddl_dml.rs`，但不是新编辑。Blocker Handoff 写的是失败点不在新编辑上。表述更宽，非阻塞。

偏差原因是验收边界变化。本 Cycle 的执行依据在用户决定之前成立：全局门失败就保持 blocked，并且禁止为了通过去改 ISS36 基线。这不是 Act 偏离，也没有 Plan 遗漏到需要在原依据里补产品修复。用户决定之后，全局门不再是本变更的完成条件。

**Acceptance Gaps**

按本 Cycle 的 A13，全局 `cargo fmt --check` 退出码 1，`cargo clippy --offline --all-targets -- -D warnings` 退出码 101，所以本 Cycle 不能 accepted，T8 也不能在这个执行依据里标完成。这个缺口不在当前 Cycle 修复。当前依据禁止改 ISS36 基线和清全库格式。用户已经改变验收边界，所以用 replan 关闭，不用 rework。

**Evidence**

- 沿用本 Cycle Act Response，本审查不重跑：`string_index_runtime_test` 为 12 passed / 0 failed；`index_key_encoding_evaluation_test` 为 7 passed / 0 failed；13 批全量每批退出码 0、failed 为 0；不加 `-D warnings` 的 clippy 退出码 0，变更文件 warning 为 0；OpenSpec validate 含 `is valid`，status 为 4/4。
- 本审查另读：未暂存 diff、154 个 `tests/*.rs`、上述行号、两份 README 第 263 行、ISS36 的 Status。本 Cycle 早先的 `rustfmt --edition 2021 --check src/storage/data/table_manager.rs` 退出码 1，只报第 729 行一处换行。该文件的修改时间早于那次命令，本次不重跑。源文件里对应的 `.await` 调用在第 732 行。
- 失败原文仍在本 Cycle 的 Blocker Handoff。本审查不改那段。

**Follow-up Decision**

用户原话：「这个后续解决，直接收尾吧」。ISS36 保持 open。不在本变更修那两条全局命令，不把 ISS36 收成新 Iteration。失败原文留在本 Cycle 的 Blocker Handoff，不再阻塞本变更验收。Act Response 保持 blocked。不在本 Cycle 把 Response 改成 reported。旧执行依据约束不了这项新的完成条件，因此要 replan，不留在当前 Cycle 修复，也不建 rework。

**Iteration Plan Update**

Iteration 003 仍是 T7 和 T8，仍是最后一轮。不新增 Iteration。验证方式改为：已通过的 T7 定向结果、13 批全量、两份 README 和 OpenSpec 检查保留并且不重跑。全局门失败留在本 Cycle，不修、不清 ISS36 基线、不把 ISS36 标成已关闭，也不再作为完成条件。平衡审计：剩下的只有把这项决定写入状态，没有新的产品结果，不拆出去。delta specs 不改，因为它们没有把全局门写成行为要求。R1–R6 仍是 Covered，没有 Simplified。

**Next Cycle**

`iterations/003-fault-boundary-and-closeout/001-replan.md`

**Next Iteration**

None
