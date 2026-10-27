# Iteration 003 / Cycle 000: 故障边界与全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 规格 `sql-secondary-indexes` 中的取消回收、只读、内存库、加密库、附加库和旧库。002 已接受的等值与范围路由继续有效。全量按本文件的 Verification 分批，整个变更只跑这一次。
- Excluded scope: 键编码、哨兵规则和路由规则。ISS36 的修复。表达式索引、部分索引，以及里程碑 41 到 44。

**Objective**

取消后的组合索引只留下已提交行。只读会话能读已有索引并拒绝再建索引。内存库、加密库和附加库上能到达的语句与文件库的行集和计划形状一致。打开旧库样本不改写样本文件。两份 README 的存储段与这些行为一致。全量分批全部 failed 为 0。

**Investigation Facts**

- Starting point: 002 的 Act Response 在 `2fd6b80`，Review Result 为 accepted。等值和范围已经路由。验证沿用该 Review 记录的三条命令，覆盖的代码在该提交之后没有变化。本轮不重跑那三条。写入本文件时 `tests/*.rs` 是 156 个。本轮会新增一个测试文件，全量目标数必须在该文件出现之后重数，不能沿用 156。
- Current-State Evidence:
  - 提交前取消的现成入口在 `tests/string_index_runtime_test.rs`。`abandon_statement` 在目标表出现至少两行版本后丢掉语句 future。下一次写入回收被放弃的隐式事务。`cancelled_string_index_write_is_reclaimed` 随后只看到两条已提交行，`check_integrity` 的 error 数为 0，没有活动事务。组合索引的派生已经走 `derive_key`，回收没有第二条编码路径。
  - 只读拒绝在 `pipeline.rs` 的 `readonly_rejection`。`CreateIndex` 臂的文本是 `cannot execute CREATE INDEX in a read-only session`，SQLSTATE 是 `25006`。它按计划形状拒绝，不区分单列和组合。`readonly_session_reads_string_index` 已覆盖字符串等值。
  - `Database::open_in_memory` 的路径是 `:memory:`，没有 WAL writer、WAL buffer 和 checkpoint manager。`memory_string_index_creates_no_files` 断言当前目录不新增以 `:memory:` 开头的文件。
  - 加密库用 `Database::open_with_key`。错钥返回 `StorageError::InvalidKey`。`encrypted_string_index_keeps_page_records` 断言文件长度减 64 后能被 4124 整除，并且明文字符串不出现在文件里。
  - 附加库写拒绝在 `PlanBuilder::reject_attached_write`。`build_create_index` 在解析列之前调用它，错误文本含 `WRITE TO ATTACHED DATABASE`。`attached_dml_maintains_string_index` 用 CLI 对附加库做 INSERT、UPDATE、DELETE，再直接打开该库核对等值计划和索引名单。`CREATE INDEX` 的 CLI 退出码是 3。
  - 旧库样本是 `tests/fixtures/legacy/legacy_g1.db`。`users` 的列是 `id INT`、`name STRING` 和 `score FLOAT`，没有二级索引。`legacy_sql_string_index_keeps_existing_rows` 打开副本后查询三行，并断言仓库里的样本字节不变。
  - `README.md` 与 `README.zh-CN.md` 的存储段仍只写单列非唯一索引和等值。没有组合索引，没有新建整数索引的范围，也没有旧整数索引的范围仍扫描。
  - `.agents/issues/ISS36-fmt-clippy-gates-unsatisfiable.md` 的 Status 是 open。全局 `cargo fmt --check` 和 `cargo clippy --offline --all-targets -- -D warnings` 在当前工具链下会失败。本轮不修改这个台账。
- Code and Critical Path: 查询仍从 `Database::execute_sql` 进入 002 的路由。只读会话在执行入口调用 `readonly_rejection`。附加库的 `CREATE INDEX` 在 `build_create_index` 里被 `reject_attached_write` 挡住。取消后的下一次写入沿用既有回收，二级键仍由该索引自己的世代和列序派生。旧库打开只读目录；样本文件本身不被写。

**Implementation Guidance**

先写 `tests/composite_index_runtime_test.rs` 并运行。通过表示这些形态已经走现有路径，不要为了制造失败去改产品代码。失败时，失败的测试就是先行测试，再修运行路径。两份 README 在定向测试通过后改。全量放在定向测试、README、格式和 OpenSpec 检查之后，并且只跑一次。

**Task Definitions**

### T5: 运行形态、README 和一次分批全量

- Requirement/Scenario: 规格里组合索引「回滚、失败和提交前取消经既有回收后恢复条目」。proposal 场景草图 S10。既有语义零回归里的全局格式门场景。002 的路由场景不在本轮重测。
- Depends on: 002 accepted
- Targets:
  - 新建 `tests/composite_index_runtime_test.rs`
  - `README.md` 与 `README.zh-CN.md` 的存储段
  - 定向测试失败时，维持组合索引条目的回收、只读拒绝、附加库拒绝或打开路径。不改 `Value::to_key`、`Key` 的比较、`derive_secondary_index_key`、`secondary_range_bounds` 和 002 的路由
- Current behavior: 字符串索引已有取消、只读、内存、加密、附加库和旧库测试。组合索引和新建整数范围还没有同一组运行形态测试。两份 README 的存储段只描述单列等值。全量还没有在本变更上跑过。
- Required behavior:
  - `cancelled_composite_index_write_is_reclaimed`：表 `t2(id INT PRIMARY KEY, a INT, b INT)` 上有 `CREATE INDEX idx_ab ON t2(a, b)`。源表有 2000 行。`INSERT INTO t2 SELECT id, id, id FROM src` 在 `t2` 出现至少两行版本后被丢掉。随后插入 `(1, 1, 1)` 和 `(2, 2, 2)`。`WHERE a = 1` 只返回 id 1，`WHERE a = 3` 没有行，`COUNT(*)` 是 2。`check_integrity` 的 error 数是 0，没有活动事务。
  - `readonly_session_reads_composite_and_new_int_range`：读写真会话建好组合索引和新的单列 INT 索引，关闭后再 `open_readonly`。组合等值的 EXPLAIN 含 `IndexScanAll` 和该索引名。新建整数索引上 `列 > 0` 的 EXPLAIN 含 `IndexRangeScan` 和该索引名，不含 `omitted=scan`，行集与关闭前相同。再建索引失败，SQLSTATE 是 `25006`，消息含 `cannot execute CREATE INDEX in a read-only session`。重新用读写打开后，索引名单与关闭前相同。
  - `memory_composite_range_creates_no_files`：`open_in_memory` 上的全整数组合索引，`a > 0` 的 EXPLAIN 含 `IndexRangeScan` 和索引名，不含 `omitted=scan`，行集与建索引前相同。路径是 `:memory:`。没有 WAL writer、WAL buffer 和 checkpoint manager。当前目录不新增以 `:memory:` 开头的文件。
  - `encrypted_composite_index_keeps_page_records`：`open_with_key` 建含字符串列的组合索引。关闭后文件长度减 64 能被 4124 整除，明文字符串不出现在文件里。错钥打开得到 `StorageError::InvalidKey`。对钥打开后等值查询返回该行，EXPLAIN 含 `IndexScanAll` 和索引名。
  - `attached_dml_maintains_composite_index`：CLI 对附加库上的组合索引执行 INSERT、UPDATE 和 DELETE。直接打开该库时，新组合、旧组合和被删组合的等值结果与这些语句一致，EXPLAIN 点名该索引。`CREATE INDEX` 打到附加库的 CLI 退出码是 3，stderr 含 `WRITE TO ATTACHED DATABASE`，索引名单不变。
  - `legacy_open_does_not_rewrite_sample`：复制 `tests/fixtures/legacy/legacy_g1.db` 后打开副本。`SELECT * FROM users` 仍是 alice、bob、carol 三行。在副本上 `CREATE INDEX idx_id_name ON users(id, name)`，`WHERE id = 1 AND name = 'alice'` 返回 alice 那一行，EXPLAIN 含 `IndexScanAll` 和 `idx_id_name`。仓库里的样本文件字节与打开前相同。
  - 两份 README 的存储段保留现有单列字符串等值、超长回扫、不截断、NULL 不入索引、重复值允许，以及 PRIMARY KEY 和 UNIQUE 仍只接受整数。同时写明：INT 与字符串列可以建多列非唯一索引；键仍是 32 字节；新建单列 INT 索引按有符号数值序服务范围；升级前的单列 INT 索引保持原字节，范围仍扫描，等值不变；可能漏掉非 NULL 行的范围结果还包含未入树且谓词成立的行，结果与全表扫描相同。
  - 全量、OpenSpec 检查和格式门见 Verification。缺一批不得写成全量通过。ISS36 保持 open。
- Preserve: 002 的计划形状和行集。字符串运行形态测试的断言。旧库样本文件。主键和唯一索引的 `to_key`。页类型和 32 字节键。
- Forbidden: 修改 `Key` 比较、`derive_secondary_index_key`、`secondary_range_bounds` 和 002 的谓词路由。不要为了单列 `1` 与 `(1, i64::MIN)` 改编码。不要格式化或 lint 修复本轮没有编辑的文件。不要把 ISS36 标成关闭。不要跑一次不带目标的 `cargo test`。不要让两批全量同时运行。
- First test: 新增 `tests/composite_index_runtime_test.rs` 的 `cancelled_composite_index_write_is_reclaimed`。实现或改文档前运行 `cargo test --offline --test composite_index_runtime_test cancelled_composite_index_write_is_reclaimed -- --test-threads=1`。现有回收若已覆盖组合索引，这一条会直接通过。通过就不要改产品代码来制造失败。失败则先保持这条失败，再修回收。
- GREEN condition: Verification 里的定向测试 failed 为 0。全量每一批 failed 为 0。OpenSpec 检查退出码为 0。本轮编辑过的 Rust 文件能被 `cargo fmt` 接受。
- Verification: 见本 Cycle 的 Verification。定向测试失败表示该运行形态的行集、计划、拒绝文本或文件副作用与本任务不一致。全量里某一批失败表示既有测试或本轮测试与当前代码不一致。锚点在 10 秒内没有出现，按夹具失败返回计划，不要放宽等待来换取通过。
- Stop when: 不修改键编码、哨兵或 002 的路由就无法让某个运行形态的结果与文件库一致；或者让本轮编辑过的文件通过格式检查必须改动逻辑而不是空白。这些情况都返回计划。

**Invariants**

- 键仍是 32 字节。超长和 NULL 不入树，不截断。
- 只读会话不写页。附加库不接受 `CREATE INDEX`。
- 打开旧库不改写仓库里的样本文件。
- 行比较按多重集合。测试可以在比较前排序。

**Non-goals**

超时取消、显式事务超时、未送达提交和语句中途失败。这些仍由 `tests/string_index_runtime_test.rs` 覆盖，并进入本轮全量。ISS36 本身。表达式索引和部分索引。

**Acceptance**

- 丢掉进行中的组合索引写入后，后续已提交查询只返回回收之后的行。完整性检查没有 error，没有活动事务。
- 只读会话能读组合等值和新整数范围，计划形状与读写会话一致。再建索引被 `25006` 拒绝。重新读写打开后索引名单不变。
- 内存库上的全整数组合范围是 `IndexRangeScan`，不含 `omitted=scan`，不产生 `:memory:` 文件。
- 加密库文件不含明文字符串。错钥打不开。对钥等值点名该组合索引。
- 附加库上的 INSERT、UPDATE、DELETE 之后，组合等值与这些语句一致。对附加库 `CREATE INDEX` 的 CLI 退出码是 3，索引名单不变。
- 打开并查询旧库副本不改变仓库样本。副本上新建的组合等值点名新索引。
- 两份 README 的存储段同时写出组合索引、新建整数范围、旧整数范围仍扫描，以及超长行不漏。
- 全量按 Verification 的分批规则跑完，每一批 failed 为 0。OpenSpec 严格检查通过。本轮编辑过的 Rust 文件格式检查通过。全局格式和 deny clippy 的结果已记录。只落在未编辑文件上、且与 ISS36 一致的失败不使本任务失败。

**Verification**

定向测试从 `Database::execute_sql` 或 CLI 进入。附加库那一条使用 CLI，因为现有附加库写入拒绝的见证就是 CLI。

1. `cargo test --offline --test composite_index_runtime_test -- --test-threads=1`
   - 上面 Required behavior 列出的六个测试都 passed，failed 为 0。
2. 本轮保存之后，用 `git diff --name-only $(git merge-base HEAD master) HEAD` 加上仍未跟踪的 `*.rs`，得到本变更编辑过的 Rust 文件。对这些路径执行 `cargo fmt -- --check <paths>`，退出码为 0。
3. `openspec validate 2026-10-27-ms39-composite-index-and-range-access --strict --no-interactive` 退出码为 0，输出含 `is valid`。
4. 全量只跑一次。目标是 `lib`、`bins` 和每个 `tests/*.rs` 的文件名（不含扩展名）。`LC_ALL=C` 排序。设 N 为当时的目标数，`B = ceil(N / 12)`。前 `B * 12 - N` 批各 11 个目标，其余各 12 个。批间串行，同一时刻只有一批。每批命令是 `cargo test --offline` 加上该批的 `--lib`、`--bins` 或 `--test <名>`，最后是 `-- --test-threads=1`。每一批都要实际运行。缺一批不得声明全量通过。每批记录退出码、passed 和 failed。failed 为 0 才算该批通过。
5. `cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings` 各跑一次并记录退出码和点名文件。失败只位于第 2 步名单之外的文件，且与 ISS36 的「全局门在当前工具链下不可满足」一致时，不修改那些文件，本任务不因此失败。第 2 步名单里的文件若被这两条命令点名，必须在本轮修好后再记录一次该文件已经不在失败名单里。

定向测试可以用这一条里的 `--test-threads=1`。不要用默认并行度跑第 4 步。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过。运行形态的可观察结果由规格、S10 和现有拒绝文本唯一确定。批准来自本次目标对已写 Iteration Plan 的授权，不是跳过。

**Persisted Evidence**

- Mode: none

定向测试和分批计数可以重跑。命令、每批的 passed/failed 和改动的文件写入 Act Response 即可。不保存完整测试输出。

**Risks and Notes**

- 取消夹具沿用字符串测试的锚点：10 秒内目标表没有出现至少两行版本，就是夹具失败，返回计划。不要把等待加长后当成产品通过。
- 写入本文件时测试文件是 156 个。加上本轮新文件后，`lib`、`bins` 和测试文件名要重新数。批表以运行时的名单为准。
- 全局 fmt 和 deny clippy 预期会失败。这不是本任务的新缺陷，也不要顺手重排未编辑文件。

## Act Response

- Status: reported

**Implemented**

T5。新建 `tests/composite_index_runtime_test.rs`，六个测试都从 `Database::execute_sql` 或 CLI 进入。取消回收、只读会话、内存库、加密库、附加库和旧库副本在实现前就已经沿现有路径通过，没有为了制造失败去改产品代码。`README.md` 与 `README.zh-CN.md` 的存储段保留原有单列等值描述，并写明多列非唯一索引、32 字节键、新建单列 INT 的有符号范围、升级前单列 INT 的范围仍扫描，以及可能漏行的范围仍包含未入树且谓词成立的行。

Clippy 点名了本变更已经改过的五个 Rust 文件。只清了这五处：`pipeline.rs` 里一段悬空文档注释改为普通注释；`IndexRangeScanExecutor::new` 和 `TableManager::alter_add_column` 按现有执行器的做法加上 `too_many_arguments` 允许；`ddl_dml.rs` 的 DEFAULT 分支收成等价的 match guard；`transaction/manager.rs` 两处 `zip` 去掉多余的 `into_iter`。键比较、`derive_secondary_index_key`、`secondary_range_bounds` 和 002 的谓词路由没有改。

**Deviations from Plan**

`cancelled_composite_index_write_is_reclaimed` 在下一次写入前断言被丢掉的事务仍活动，与字符串取消夹具相同。附加库测试除了新键、旧键和被删键，还断言 UPDATE 后的 `(11, 11)` 返回 id 1。

`cargo fmt -- --check <paths>` 在当前 Cargo 里仍会检查包内其余文件。路径本身的格式结果来自 `rustfmt --edition 2021 --check` 加这 27 个 Rust 路径，退出码为 0。

**Self-review**

Gate 3 没有未解决的 Critical 或 Important 问题。六个运行形态的行集、计划、拒绝文本和文件副作用都由对应测试观察。允许属性和注释不改变签名或执行。match guard 与去掉 `into_iter` 之后，`add_column_with_default_backfills` 和 `cancelled_composite_index_write_is_reclaimed` 重新运行，failed 都是 0。全量是在这两处修改之前跑的，没有再跑第二次。

**Verification Evidence**

先行测试在改产品和改文档之前运行：

- `cargo test --offline --test composite_index_runtime_test cancelled_composite_index_write_is_reclaimed -- --test-threads=1`：1 passed，0 failed，5 filtered out。

定向测试：

- `cargo test --offline --test composite_index_runtime_test -- --test-threads=1`：6 passed，0 failed。六个测试分别是 `cancelled_composite_index_write_is_reclaimed`、`readonly_session_reads_composite_and_new_int_range`、`memory_composite_range_creates_no_files`、`encrypted_composite_index_keeps_page_records`、`attached_dml_maintains_composite_index`、`legacy_open_does_not_rewrite_sample`。

本变更 27 个 Rust 路径的 `rustfmt --edition 2021 --check` 退出码为 0。

`openspec validate 2026-10-27-ms39-composite-index-and-range-access --strict --no-interactive` 退出码为 0，输出含 `is valid`。

全量只跑了一次。新测试文件出现后目标数 N 为 159，包括 `lib`、`bins` 和 157 个测试文件名。`LC_ALL=C` 排序。B 为 14。前 9 批各 11 个目标，后 5 批各 12 个。批间串行。每批都是 `cargo test --offline` 加上该批的 `--lib`、`--bins` 或 `--test`，最后是 `-- --test-threads=1`。

| 批 | 退出码 | passed | failed |
|---|---|---|---|
| 01 | 0 | 148 | 0 |
| 02 | 0 | 186 | 0 |
| 03 | 0 | 149 | 0 |
| 04 | 0 | 110 | 0 |
| 05 | 0 | 175 | 0 |
| 06 | 0 | 65 | 0 |
| 07 | 0 | 537 | 0 |
| 08 | 0 | 84 | 0 |
| 09 | 0 | 245 | 0 |
| 10 | 0 | 104 | 0 |
| 11 | 0 | 126 | 0 |
| 12 | 0 | 185 | 0 |
| 13 | 0 | 157 | 0 |
| 14 | 0 | 97 | 0 |

`cargo fmt --check` 退出码为 1。点名文件是 `tests/hash_join_build_side_test.rs` 和 `tests/sort_merge_join_test.rs`。二者不在本变更的 Rust 路径里，没有修改。

`cargo clippy --offline --all-targets -- -D warnings` 在修复前退出码为 101，点名了 `src/pipeline.rs`、`src/executor/index_range_scan.rs`、`src/parser/planner/ddl_dml.rs`、`src/storage/data/table_manager.rs`、`src/transaction/manager.rs`，以及变更路径之外的文件。修复后同一命令退出码仍为 101。上述五个文件不再出现。仍被点名的项目文件是 `src/cli/lifecycle.rs`、`src/executor/alter_table.rs`、`src/executor/executor_trait.rs`、`src/executor/sort_merge_join.rs`、`src/network/protocol.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`。这些文件不在本变更的 Rust 路径里，没有修改。ISS36 仍是 open。

受 lint 修改影响的定向复核：

- `cargo test --offline --test alter_table_test add_column_with_default_backfills -- --test-threads=1`：1 passed，0 failed，15 filtered out。
- `cargo test --offline --test composite_index_runtime_test cancelled_composite_index_write_is_reclaimed -- --test-threads=1`：1 passed，0 failed，5 filtered out。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Unresolved**

None。
