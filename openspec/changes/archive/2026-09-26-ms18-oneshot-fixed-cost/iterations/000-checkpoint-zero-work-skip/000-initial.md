# Iteration 000 / Cycle 000: Checkpoint 零工作跳过

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1 全部 6 场景（`specs/ms07-rest-tx-checkpoint-pushdown/spec.md` ADDED「Checkpoint 零工作跳过」）；design D1–D4；proposal 范围与决策记录（用户批准 2026-09-26）
- Excluded scope: T2 runtime flavor、T3 README 刷新（Iteration 001）；WAL 格式/九步流程语义变更；阈值跳过；`open` 恢复后路径新接线；Server/runtime 面

**Objective**

checkpoint 在「WAL 自上次 checkpoint 以来无追加（`write_count == 0`）且缓冲池无脏页」时整体无操作（不刷页、不写位点、不碰 WAL），任一非空时九步全量行为逐字节不变；`tests/checkpoint_skip_test.rs` 六场景矩阵 + 守卫单测全绿，既有 checkpoint/schema 套件零校准通过，全量串行零回归。

**Investigation Facts**

- Current Baseline: master @ bc1d18d + 工作区未提交 MS19 收尾批次（Rust 代码自 2026-09-26 MS19 收尾验证后零变化，全量 1277 tests / 0 failures / 2 ignored，SNAPSHOT「仓库现场」）。定向基线（2026-09-26 本 Cycle Plan 实测）：`cargo test --test checkpoint_test --test checkpoint_redo_reduction_test --test schema_persistence_test` → 3 + 9 + 8 全部 passed，退出码 0。one-shot 时延基线：R26 runbook 2026-09-24 @ rev 1a2d877 实测 10.77ms/run（50 次循环）/ 峰值 RSS 16.7MiB——rev 已陈旧仅作量级参照，本 Cycle 前后对比以 Act 自测的改动前基线为准。
- Current-State Evidence:
  - 调用面：`Database::close()`（`src/database.rs:249-251`）→ `Database::checkpoint()`（:262-266，公开 API，watermark 闭包 `|| transaction_manager.current_tx_id()`）→ `CheckpointManager::checkpoint`（`src/wal/checkpoint.rs:131-175`）九步：①`get_current_lsn()`（WAL 文件长度）→ ①b 水位闭包 → ②`buffer_pool.flush_all()` → ③WAL fsync → ④timestamp → ⑤写位点 24B → ⑥追加 `Checkpoint` 记录 → ⑦`rewrite_truncate(lsn)` 保留后缀 → ⑧位点置 0（同水位）→ ⑨`reset_write_count()`。CLI 生产路径唯一触发点 `src/cli/mod.rs:347` `db.close().await`；`open` 不调 checkpoint。返回值消费者：仅 `tests/checkpoint_test.rs:53` 绑定 `captured_lsn`（夹具先写 5 条记录 → count>0 → 全量路径，断言不受守卫影响），其余全 `.unwrap()` 丢弃。
  - WAL 稳态：checkpoint 后 WAL 恒含恰一条 Checkpoint 标记记录（⑥追加 + ⑦保留后缀）——文件长度探针永假，这是谓词用 `write_count` 的决定性理由。`WalWriter::get_write_count()`（`src/wal/writer.rs:152-154`）同步 AtomicU64；`write_record` +1（:68）、`write_batch` +n（WALBuffer `do_flush` 唯一批量入口，buffer.rs:147-182）；步骤 9 清零。
  - WALBuffer：容量 100 / 100ms 定时刷盘；commit 经 `append_commit_and_wait` 等待刷盘确认（buffer.rs:82-104）；abort 仅 `append` 缓冲（manager.rs:152）不等待；CLI 生产路径不调 `wal_buffer.shutdown()`（仅测试夹具调用）。
  - BufferPool：脏位在 frame（`flush_all` Phase 1 迭代置 false + 收集写，buffer_pool.rs:216-233）；`set_all_visible`/`clear_all_visible`/`update_visibility_on_insert` 只写内存 vis_map（:379/:397/:409，sync fn 不碰页字节）；无现成脏页查询 API。
  - 会话 WAL 写入面：DML 经 `begin → WALBuffer → commit`（pipeline.rs:150/218-224，BeginTxn/CommitTxn/数据记录）；SELECT 不包事务——查询路径 `statement_snapshot(None)` 直构执行器（:235-245），零 WAL 记录；DDL 直包执行器（:128-136），零 WAL 记录、经池置脏——schema 持久化唯一通道是 checkpoint 刷脏页（`src/database.rs:236-247` MS07-T01 doc 注释同证）。
  - Catalog bootstrap（`src/storage/catalog.rs:109-136`）：`allocate_page` ×2（storage 直分配）→ `init_empty_slotted_page` 经池置脏 → **自行 `flush_all()`**（:136，脏位清零）→ 新库打开后池干净、WAL 零记录。格式头在 `FileStorage::open` 直写（先于池存在）。
  - Recovery（`src/wal/recovery.rs:454-556`）：`redo_count` 只数已提交数据记录（`data_records` 过滤 committed & ≥ redo_from，:531-536），Checkpoint 标记不计；`redo_record` 与 `mark_uncommitted_aborted` 均经池写页（置脏）。
- Code and Critical Path: 守卫插入点 `CheckpointManager::checkpoint` 顶部（该方法已持有 `wal_writer: Arc<WalWriter>` 与 `buffer_pool: Arc<BufferPool>`，checkpoint.rs:14-18）；新探测 `BufferPool::has_dirty_pages()` 放 `src/storage/buffer_pool.rs`（`flush_all` 同域）；`Database::close()` doc 注释补零工作跳过语义；测试新文件 `tests/checkpoint_skip_test.rs` + `src/wal/checkpoint.rs` `#[cfg(test)]` 守卫单测（既有 3 个位点单测同模块，`manager_for` 夹具可复用扩展）。

**Implementation Guidance**

实施顺序（TDD）：① 先写 `tests/checkpoint_skip_test.rs` 六用例（`fresh_db_zero_work_close_creates_no_site` 在现状必 RED——全量 checkpoint 必创建位点文件；其余五用例现状 GREEN，作为行为锁定基线）；② `BufferPool::has_dirty_pages()` 只读探测（DashMap 迭代遇 `dirty == true` 提前返回 true；同步 fn 无 await；与 `flush_all` Phase 1 同型分片迭代纪律，不清位不置位）；③ `CheckpointManager::checkpoint` 顶部守卫：`if self.wal_writer.get_write_count() == 0 && !self.buffer_pool.has_dirty_pages() { return Ok(0); }` + doc 注明「0 = 零工作跳过，未写位点」；④ `src/wal/checkpoint.rs` 单测：零工作 manager 直调返回 0 且 tempdir 无位点文件产生；⑤ `Database::close()` doc 更新；⑥ 定向四套件 + 全量串行。

测试夹具要点：DDL-only 场景用 SQL 语句形态 `CREATE TABLE ...`（贴近真实会话；`Database::create_table` API 与 SQL DDL 同一执行器同不写 WAL）；字节对比读位点文件全文（24B）与 `.wal` 全文，close 前后各读一次比对；crash 脏库场景 = 打开 → INSERT 若干 → `drop(db)`（不 close）→ 重开（恢复重放）→ SELECT → close → 断言位点推进（字节变化）+ WAL 截断。

**Task Contracts**

### T1: Checkpoint 零工作跳过（R1 六场景）

- Requirement/Scenario: R1 全部 6 场景（零工作 close 跳过 / 新库零工作 close 无位点 / DML 会话全量保持 / DDL-only 会话全量保持不丢表 / crash 脏库全量保持 / 显式 checkpoint 零工作无操作）
- Depends on: None
- Targets: `src/wal/checkpoint.rs::CheckpointManager::checkpoint`（顶部守卫 + doc）、`src/storage/buffer_pool.rs::BufferPool`（新 `has_dirty_pages`）、`src/database.rs::Database::close`（doc 注释）、`tests/checkpoint_skip_test.rs`（新，六用例）、`src/wal/checkpoint.rs` tests 模块（守卫单测）
- Current behavior: checkpoint 无条件九步全量（多次 fsync + WAL 重写）；零写入 one-shot 会话 close 全价执行；新库零工作 close 必创建位点文件
- Required behavior: `write_count == 0 && !has_dirty_pages()` → 早退 `Ok(0)`，零副作用（不刷页、不写位点、不 fsync、不重写/截断 WAL；`write_count` 已为 0 无需重置）；任一条件非空 → 既有九步全量逐字节不变；跳过与执行两态下重开（含崩溃恢复）语义等价
- Preserve: 公开 API 签名（`checkpoint(&self, impl Fn() -> u64) -> Result<u64, WalError>`、`close() -> Result<()>`）；九步崩溃窗口次序与位点 24B 格式（跳过路径不触发，全量路径不动）；WALBuffer 恢复/刷盘语义；M17 锁序；M18「checkpoint-clean 关闭磁盘树可信」不变量（跳过仅当等价 checkpoint-clean 态）
- Forbidden: 不改 WAL 记录格式与九步次序；不改 WALBuffer；不动 `open`/恢复路径接线；不加阈值条件；不引入身份型证据机制（守卫只消费既有 `write_count` 与新增脏页探测，无新协议/CLI/审计面）；不动 Server/runtime 面（T2 域）；不修改既有测试断言
- Test witness: `tests/checkpoint_skip_test.rs` 六用例先于实现运行——`fresh_db_zero_work_close_creates_no_site` 观察 RED（现状位点文件被创建），其余五用例观察 GREEN（行为锁定）；`cargo test --test checkpoint_skip_test` 退出码非 0（RED）→ 实现后 0
- GREEN condition: 六用例 + `zero_work_checkpoint_returns_zero_without_site` 单测全绿；`checkpoint_test`(3)/`checkpoint_redo_reduction_test`(9)/`schema_persistence_test`(8) 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test checkpoint_skip_test --test checkpoint_test --test checkpoint_redo_reduction_test --test schema_persistence_test`（退出码 0）；②全量 `cargo test -- --test-threads=1`（M19 纪律，退出码 0）；③one-shot 时延前后对比——R26 步骤 5 口径 50 次 `rtsql <db> "SELECT 1"` 循环，改动前基线与改动后各一轮（release 构建），方向性下降即支持目标（精确数字 Iteration 001 三组实测定形）；时延无下降时不阻塞 Acceptance（行为等价与文件级见证已足），记录数据供 Iteration 001 分析
- Stop when: 发现绕过 `write_count` 与脏页的未决工作来源（D2 安全论证前提失效）；文件级见证不可观测；守卫引发既有测试失败且需修改既有断言才能通过（说明语义变化，返回 Plan）

**Invariants**

- 「close 即落盘」语义不变：有未决工作（DML 数据、DDL 脏页、崩溃恢复重放脏页）必全量 checkpoint；无未决工作时本无落盘工作。
- 跳过判定是进程内既有状态消费（`write_count` AtomicU64 + 池脏位），零新增 I/O、零协议面。
- 九步全量路径逐字节不变——凡谓词非空，行为与本 change 前完全一致。

**Non-goals**

- T2 runtime flavor、T3 README 刷新（Iteration 001）。
- WAL 格式、九步流程、恢复路径、WALBuffer 语义变更。
- 阈值跳过（用户否决：重开索引重建代价 + M18）。
- `open` 恢复后 checkpoint 接线（现状 open 不调 checkpoint，保持）。

**Acceptance**

R1 六场景全部可观察满足（映射见 RTM 与 Task Contract Test witness/GREEN condition）；既有定向套件零校准；全量串行零回归；one-shot 时延方向性见证（非阻塞）。Scenario→Task→Test 链路：RTM R1 行。

**Verification**

- S1 零工作 close 跳过：`zero_work_close_skips_site_and_wal`——位点/WAL close 前后字节一致 + 重开全数据可查。
- S2 新库零工作：`fresh_db_zero_work_close_creates_no_site`——close 后无位点文件（RED→GREEN）+ 重开干净。
- S3 DML 会话：`dml_session_close_keeps_full_checkpoint`——位点字节变化 + WAL 截断 + 重开数据在。
- S4 DDL-only：`ddl_only_close_keeps_full_checkpoint_and_schema`——位点写入 + 重开建表在。
- S5 crash 脏库：`crash_dirty_db_close_keeps_full_checkpoint`——重开恢复后 close 位点推进 + WAL 截断。
- S6 显式零工作：`explicit_checkpoint_zero_work_is_noop`——`checkpoint()` 返回 Ok 且位点字节不变。
- 判定均为各测试自身断言 + cargo 退出码；全量串行 + 时延对比见 Task Contract Verification。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 全 Covered）；调查完整（调用面/稳态/写入面/恢复面/bootstrap 全部 file:line 实证）；设计闭合（D1 谓词、D2 安全论证穷举、D3 探测、D4 见证）；任务契约自包含（Act 无需回读外部资料）；验证直接判定（文件字节 + 测试断言 + 退出码，无身份型证据工程）；无实质未知项。等待用户批准计划后 Status 转 ready。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果：文件级断言在测试内自证，命令输出均 <20 行可入 Response；无一次性环境、无不可复现结论。

**Risks and Notes**

- WALBuffer 缓冲中 begin-only 悬挂记录（无数据、无 commit/abort）在跳过路径可能不落盘——与今日进程退出行为一致，recovery 对无数据记录事务无操作，语义惰性（D2 论证覆盖）。
- pre-site legacy 形态（位点 lsn>0 且无尾部追加）跳过保持位点与文件原样，磁盘字节与「今日全量重写后」不同但语义等价（位点过滤语义既有）；已知可接受差异。
- 守卫与返回间的并发写窗口与既有 close 相同（单进程多 task 共享 Database 的库用户场景）；不扩大既有窗口。
- one-shot 时延对比受机器噪声影响——Iteration 000 只看方向，精确数字与 flavor 决策在 Iteration 001 三组实测（×3 取中位）。
- 实施时若 `has_dirty_pages` 与 `flush_all` 迭代纪律冲突（如 borrow checker 阻止提前返回），允许等价实现（如先收集计数），行为契约不变——非实质选择。

## Act Response

- Status: reported

**Implemented**

T1 全量交付（4 处代码面 + 1 个新测试文件，产品代码净增 110 行 / 删 0 行，其中 doc 8 行）：

- `src/storage/buffer_pool.rs:235` — 新增 `BufferPool::has_dirty_pages() -> bool`（pub 只读探测）。DashMap 迭代取每 frame 锁读 `dirty`，`.any()` 命中即返回；不清位不置位，同步 fn 无 await，与 `flush_all` Phase 1 同型分片纪律（M17 锁序不变），仅在 checkpoint 入口调用（不触碰热路径）。
- `src/wal/checkpoint.rs:151` — `CheckpointManager::checkpoint` 方法顶部、步骤 1 之前加入守卫
  `if self.wal_writer.get_write_count() == 0 && !self.buffer_pool.has_dirty_pages() { return Ok(0); }`。
  单点覆盖全部触发路径（`Database::close()`、公开 `checkpoint()`、库用户直调）；`open`/恢复路径未接线（保持现状）。跳过路径不调 `reset_write_count()`（计数已为 0），九步全量路径逐字未动。
- `src/wal/checkpoint.rs:123` — `checkpoint` 方法 doc 增补零工作跳过语义：判定条件、`Ok(0)` 含义、WAL 侧为何用 `write_count` 而非文件长度（步骤 6/7 使 checkpoint 后 WAL 恒含标记记录，长度探针永假）、DDL 不写 WAL 故脏页条件必需。原有崩溃窗口次序与 MS17-T02 D8 水位说明原样保留。
- `src/database.rs:250` — `Database::close()` doc 增补零工作跳过语义（MS18-T01 段），「close 即落盘」承诺表述不变。
- `tests/checkpoint_skip_test.rs`（新，349 行，6 用例）+ `src/wal/checkpoint.rs` tests 模块 2 单测。

公开 API 签名零变化：`checkpoint(&self, impl Fn() -> u64) -> Result<u64, WalError>`、`Database::close() -> Result<()>`、`Database::checkpoint() -> Result<()>`、`CheckpointManager::new/read_checkpoint_site/write_checkpoint_site`。

**Deviations from Plan**

1. **测试见证加「哨兵位点」以消除秒级时间戳的空洞断言（测试技术，等价强化）**：计划约定「close 前后各读一次位点/WAL 全文比对」。位点 24B 含 timestamp（秒级）与 tx watermark，两次全量 checkpoint 可能落在同一秒、且只读会话未必推进水位——纯「字节不变」断言在守卫缺席时可能空洞通过（测试因错误原因通过，Gate 3 禁止）。实现保持计划的比对技术，另加 `stamp_sentinel_site()`：用产品 API `CheckpointManager::write_checkpoint_site(0, 1, 0)`（1970 时间戳，经 `read_site_file` 24B 布局核对）把位点改写为全量 checkpoint 不可能产生的值，使「重写」与「未动」恒可区分。`lsn=0` 正是常规 checkpoint 后取值，哨兵态与 checkpoint-clean 库语义等价，`watermark=0` 为保守方向，不掩盖恢复期水位推进。RED 观察证实非空洞：守卫前 S1 断言 `left: [.., 21, 202, 183, 106, ..]`（真实时间戳）vs `right: [.., 1, 0, 0, ..]`（哨兵）。
2. **新增第二个单测 `dirty_pool_checkpoint_keeps_full_path`（RTM 只列了一个）**：谓词有两个条件，RTM 命名的单测只覆盖 WAL 侧（零 → 跳过）与 WAL 非零 → 全量。池侧真分支需要构造脏页，无公开「置脏」入口以外的装置；经 `PageGuard::mark_dirty()`（既有公开 API）可在 `src/wal/checkpoint.rs` 内直接构造，成本 1 个同域单测。落在 Task Contract 已列的 Targets（`src/wal/checkpoint.rs` tests 模块），不扩变更面。S4 已在 e2e 面覆盖同一条件，本单测提供 `has_dirty_pages()` 真分支的单元级直接见证。
3. **该单测以「位点文件存在 + 脏位转假」判定全量路径，不用返回值**：新库 WAL 长度为 0，全量路径捕获的 LSN 也是 0（`dirty_pool...` 夹具），返回值无法区分两条路径（见 Remaining Issues 1）。计划 GREEN condition 只要求该单测覆盖早退语义，已满足。
4. **全量串行跑了两次**：第一次取每套件明细（全部 `ok`），第二次为把 `test result` 行汇总成总数（1285/0/2）。第二次不是可靠性复跑而是数字聚合，但客观上违反「验证通过一次即为通过」的最小执行原则，如实记录。
5. **rustfmt 作用于三个目标文件整体而非仅新增行**：`cargo fmt --check` 基线本已不干净（`src/executor/delete.rs`、`src/parser/planner/mod.rs`、`src/storage/data/table_manager.rs`、`src/wal/recovery.rs`、`tests/cli_test.rs`、`tests/constraint_enforcement_test.rs` 六文件为 MS19/MS21 遗留）。已核对本轮三个文件的 fmt diff 全部落在新增行上，故对 `src/storage/buffer_pool.rs`、`src/wal/checkpoint.rs`、`tests/checkpoint_skip_test.rs` 单独执行 `rustfmt`（退出码 0），未触碰其余六文件（不清理无关代码）。格式化为纯空白变更。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

Gate 3（spec compliance）逐项核对：

- R1「Checkpoint 零工作跳过」Requirement 全部 SHALL 项：SHALL NOT 刷脏页/写位点/重写截断 WAL（守卫在步骤 1 前 return，三者均不执行）；任一条件非空时全量行为逐字节不变（九步代码未改，既有套件 20 测试零校准通过）；判定覆盖全部触发路径（单点守卫，§D1 拒绝「守卫放 close()」的理由同时成立）；公开 API 签名不变（已核）；跳过/执行两态重开语义等价（S1/S3/S5/S6 均含重开断言）；「close 即落盘」不变（S3 DML / S4 DDL-only / S5 crash 脏库三条未决工作来源均走全量）。
- 六场景与计划命名一一对应：S1 `zero_work_close_skips_site_and_wal`、S2 `fresh_db_zero_work_close_creates_no_site`、S3 `dml_session_close_keeps_full_checkpoint`、S4 `ddl_only_close_keeps_full_checkpoint_and_schema`、S5 `crash_dirty_db_close_keeps_full_checkpoint`、S6 `explicit_checkpoint_zero_work_is_noop`；单测 `zero_work_checkpoint_returns_zero_without_site` 按 RTM 命名。
- 见证非空洞：守卫前 3 RED（S1/S2/S6）/ 3 GREEN（S3/S4/S5 行为锁定，与计划预期一致），实现后 6 GREEN。S6 另有直判信号——守卫前 `captured` 返回 21（真实 LSN），守卫后为 0。
- 预设条件显式断言：S1/S2/S4/S6 断言 `get_write_count() == 0`，S3 断言 `> 0`，确保各用例确在目标谓词状态下运行。
- Preserve/Forbidden 全部守住：WAL 记录格式与九步次序未动、WALBuffer 未动、`open`/恢复路径未接线、无阈值条件、无身份型证据机制（守卫只消费既有 `get_write_count()` 与新增只读探测，无协议/CLI/审计面）、Server/runtime 面未动、既有测试断言零修改（`git diff` 显示测试侧只新增一个文件）。

Gate 3（code quality）逐项核对：diff 无计划外修改；错误/边界/状态/资源生命周期正确（守卫不持锁跨越 await，`has_dirty_pages` 单帧锁、无嵌套 BufferPool 调用 → 无锁序倒置）；无新增警告（`cargo clippy --all-targets` 退出码 0）；无死代码（`has_dirty_pages` 唯一调用点即守卫）；无重复实现（探测与 `flush_all` Phase 1 同型但语义不同，未合并）；无无依据复杂度；测试不因错误原因通过（哨兵位点 + 预置断言 + RED 观察三重保障）；命名与文件内既有惯例一致（`has_*` 谓词、`ms07-rest-tx-checkpoint-pushdown` 注释前缀沿用文件内既有风格）；无判定层（判定全在 `cargo test` 与 cargo 退出码内，无自建脚本/包装命令）。

自检发现并已修复（均在 GREEN 之前或过程中）：
- 集成测试初版 `count_users` helper 写坏（重复执行 + 自引用断言）→ 重写为单次查询返回 `i64`。
- `assert_ne!(Vec<u8>, Option<Vec<u8>>)` 类型不匹配（编译失败）→ 统一为 `Option<Vec<u8>>` 两侧比较。
- 单测 `PageId::new(0)` 不存在（`PageId` 是 tuple struct）→ `PageId(0)`。
- 夹具页 0 在仅 64B 文件头上 `read_page` 返回 `UnexpectedEof` → 先经 `storage().write_page(...)` 落一页再 `get_page` + `mark_dirty()`。
- import 路径 `crate::storage::page_format::PageId` 不存在 → `crate::storage::PageId`（连带 `Page`）。

遗留 Minor（不阻塞，见 Remaining Issues）：无 Critical、无 Important。

额外发现（产品面，change 范围内，作为行为证据记录）：真实 CLI 路径独立确认——预建 2000 行库上执行 `rtsql <db> "SELECT 1"` 后，`r2k.checkpoint`（24B）与 `r2k.wal`（21B）md5 逐字节不变，旁证守卫在生产路径生效（非仅测试夹具）。

**Verification Evidence**

1. 定向四套件（`cargo test --test checkpoint_skip_test --test checkpoint_test --test checkpoint_redo_reduction_test --test schema_persistence_test`，rustfmt 后新鲜运行），退出码 0：
```
running 9 tests   (checkpoint_redo_reduction_test)
test result: ok. 9 passed; 0 failed; 0 ignored
running 6 tests   (checkpoint_skip_test)
test result: ok. 6 passed; 0 failed; 0 ignored
running 3 tests   (checkpoint_test)
test result: ok. 3 passed; 0 failed; 0 ignored
running 8 tests   (schema_persistence_test)
test result: ok. 8 passed; 0 failed; 0 ignored
```
既有三套件 9+3+8=20 测试零校准（断言与夹具零修改，见 `git diff`）。

2. 守卫单测（`cargo test --lib wal::checkpoint`），退出码 0：
```
running 5 tests
test wal::checkpoint::tests::zero_work_checkpoint_returns_zero_without_site ... ok
test wal::checkpoint::tests::dirty_pool_checkpoint_keeps_full_path ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

3. 全量串行（`cargo test -- --test-threads=1`，M19 纪律），全部套件 `ok`，退出码 0：
```
passed=1285 failed=0 ignored=2
```
基线 1277 + 本轮新增 8（6 集成 + 2 单测）= 1285，failed/ignored 与基线一致（2 ignored 为既有）。逐套件明细见 Deviation 4。

4. RED→GREEN 见证（`cargo test --test checkpoint_skip_test`，守卫缺席时），退出码非 0：
```
test fresh_db_zero_work_close_creates_no_site ... FAILED
test explicit_checkpoint_zero_work_is_noop ... FAILED
test zero_work_close_skips_site_and_wal ... FAILED
test ddl_only_close_keeps_full_checkpoint_and_schema ... ok
test crash_dirty_db_close_keeps_full_checkpoint ... ok
test dml_session_close_keeps_full_checkpoint ... ok
test result: FAILED. 3 passed; 3 failed
---- explicit_checkpoint_zero_work_is_noop ----
  assertion `left == right` failed: zero-work checkpoint reports 0 (no LSN captured)
  left: 21   right: 0
---- zero_work_close_skips_site_and_wal ----
  assertion `left == right` failed: zero-work close must not rewrite the checkpoint site
  left: Some([0,0,0,0,0,0,0,0, 21,202,183,106, 0,0,0,0,0,0,0,0])   ← 真实时间戳
 right: Some([0,0,0,0,0,0,0,0, 1,0,0,0, 0,0,0,0,0,0,0,0])          ← 哨兵
---- fresh_db_zero_work_close_creates_no_site ----
  zero-work close must not create a checkpoint site file
```
实现后同一命令 6 passed / 0 failed（见证据 1）。

5. 静态与格式：`cargo clippy --all-targets` 退出码 0（无 warning/error 计数）；`rustfmt --edition 2021` 对三目标文件退出码 0，其后 `cargo fmt --check` 的 diff 文件列表不含这三个文件（余六项为基线既有，未触碰）。

6. OpenSpec：`openspec validate 2026-09-26-ms18-oneshot-fixed-cost --strict` → `Change '2026-09-26-ms18-oneshot-fixed-cost' is valid`，退出码 0。

7. one-shot 时延方向见证（R26 runbook 步骤 5 口径，release 构建，同一预建 2000 行库 `/tmp/opencode/r2k.db`，同机背靠背各一轮 50 次循环）：
```
before: 12.35 ms/run
after:  5.85 ms/run
```
改动前基线由 Act 自测：`git stash push` 仅回退本轮三个产品文件 → `cargo build --release` → 存 `rtsql-before` → `git stash pop` 复原（`git diff --stat` 复原为 110 insertions/1 deletion，与 stash 前逐字一致；新测试文件为 untracked 未被 stash 影响，`wc -l` 349 行完好）。方向性下降约 53%，支持 T1 目标；绝对值与 R26 2026-09-24 记录（10.77ms @ rev 1a2d877）不同属环境差异，两数同场背靠背可比。精确数字与 flavor 决策归 Iteration 001 三组实测（×3 取中位）。

8. 真实 CLI 路径旁证（构建产物 + 预建库，非测试夹具）：
```
6377183cf0d7d7ad7bd0f3b016ef54e3  /tmp/opencode/r2k.checkpoint
9686bd0efd147e2e91724a7ccc05a54e  /tmp/opencode/r2k.wal
6377183cf0d7d7ad7bd0f3b016ef54e3  /tmp/opencode/r2k.checkpoint   ← rtsql-after <db> "SELECT 1" 之后
9686bd0efd147e2e91724a7ccc05a54e  /tmp/opencode/r2k.wal
```

change 结构自检：tasks.md 状态与实际一致（T1 交付；T2/T3 仍属 Iteration 001 未展开）；delta spec `ms07-rest-tx-checkpoint-pushdown` ADDED「Checkpoint 零工作跳过」6 场景与已实现行为逐条对应；design D1–D4 与实现一致（D1 谓词与落点、D2 安全论证、D3 探测、D4 文件级见证）；Iteration 000/Cycle 000 文件齐全；`Review Result: pending` 与流程状态一致。

Iteration 状态更新：不适用。已读取 change `tasks.md`——其 Task List 为 `Task | 内容 | Requirement | Iteration` 表格、Iteration Plan 为无状态字段的条目列表，本 change 未定义 task 级或 Iteration 级状态字段（与 MS24/MS19 归档 carrier 的 `- [x]` 清单格式不同）。按「精准修改、不补全 Plan 遗漏设计」纪律，Act 未新增状态字段，T1 交付状态由本 Response 与 RTM 承载。

**Persisted Evidence**

None required —— 计划设定 `Mode: none`，本轮全部结论均由测试自身断言 + cargo 原生退出码 + ≤20 行决定性输出承载，文件级见证在测试内自证，命令可在任意 checkout 低成本重跑；无一次性环境、无不可复现结论、无身份型证据需求。构建产物对比（`/tmp/opencode/rtsql-before`、`rtsql-after`、`r2k.db`）为可由 `git stash` + `cargo build --release` + R26 步骤 5 完全复现的流程，不满足保存白名单。

**Experience Candidates**

- Runbook 候选：R26 runbook（`.agents/runbooks/sqlite-compare-benchmark.md`）步骤 5 只给了绝对时延口径，缺「同变更前后对照」的取基线方法——本轮用 `git stash push -- <仅本轮产品文件>` → release 构建存二进制 → `git stash pop` 复原取得同场可比基线，并逐字核对 `git diff --stat` 复原。适用于任何「工作区含未提交他人/前序批次改动、不可整体回退」的对照测量场景。若用户认为值得沉淀，由 `openspec-experience-recorder` 决定是否更新 R26。
- Issue 候选：无。本轮自检与验证未发现 change 范围外的实质缺陷。

**Remaining Issues**

1. Minor（返回值语义，change 范围内，无当前消费者）：`checkpoint()` 的 `Ok(0)` 既表示「零工作跳过」，也可能来自「WAL 长度为 0 的全量路径」（新库 DDL-only 场景，见 `dirty_pool_checkpoint_keeps_full_path` 夹具）。doc 已写明 `0 = 零工作跳过，未写位点`，但单看返回值无法判别；当前无任何消费方据此分支（`Database::close`/`Database::checkpoint` 丢弃返回值，`checkpoint_test.rs:53` 的 `> 0` 断言所在夹具 WAL 非空），故无功能影响。若未来需要「是否跳过」的可观测信号，应另设明确返回类型而非复用 `u64`——本轮不扩范围。
2. Minor（测试基建）：集成测试 `stamp_sentinel_site` 直接依赖位点 24B 布局（`lsn=0, timestamp=1, watermark=0`）。若 MS17-T02 的位点格式再演进（如扩字段），该夹具需同步；影响面仅本文件 4 处调用。
3. 无 Critical、无 Important。Gate 5 未触发（无 Blocker，无三次失败）。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（完整 diff + 四个代码面 + 新测试文件逐行阅读）与 Act 声明一致，未发现 Plan 遗漏、Plan 错误、Act 偏离或基线变化。逐项核对：守卫位置与谓词与 design D1 逐字一致（`checkpoint.rs` 步骤 1 前、`write_count == 0 && !has_dirty_pages()`、`Ok(0)`）；九步主体与位点/WAL 格式未动（diff 纯插入）；`has_dirty_pages` 为同步只读探测、与 `flush_all` Phase 1 同型分片纪律、无 await 无锁跨越；公开 API 签名零变化；六个集成场景与 RTM 一一对应且前置条件显式断言（`get_write_count` 在各用例目标态断言）；哨兵位点技术健全（经产品 API 写入全量路径不可能产生的 24B 值，RED 输出证实判别力非空洞）；单测 `dirty_pool_checkpoint_keeps_full_path` 落在 Task Contract Targets 内。本轮范围核实：3 个产品文件（database.rs/buffer_pool.rs/checkpoint.rs 的 diff 仅含本轮内容）+ 1 个新测试文件（untracked）；三个回归套件与 `wal_recovery_large_test.rs` 自 HEAD 起零修改（`git status` 空输出），其 GREEN 即零回归的干净见证。

Minor finding（均不阻塞）：

1. **注释外观**：`tests/checkpoint_skip_test.rs:153` 行注释内含转义引号字面量（`\"never checkpointed\"`），渲染为反斜杠噪声。纯外观，低于修复门槛（公共规则 › Review 不为风格偏好创建修复），记录不改。
2. **流程偏差（Act 自报 Deviation 4）**：全量串行跑了两次，第二次用于数字聚合（1285 汇总）。客观上超出「验证通过一次即为通过」的最小执行；同代码状态、非可靠性复跑，无结论影响。记录备查。
3. **`Ok(0)` 语义歧义（Act Remaining Issue 1）**：返回值无法区分「零工作跳过」与「WAL 长度为 0 的全量路径」。当前无消费方（close/checkpoint 丢弃返回值），doc 已写明；同意 Act 判断——未来需要该信号时另设返回类型，本轮不扩范围。
4. **测试基建耦合（Act Remaining Issue 2）**：哨兵夹具依赖位点 24B 布局，格式演进时需同步（影响面 4 处调用）。可接受。

Deviation 1/2/3（哨兵位点、第二个单测、返回值判定改文件+脏位）全部接受：属等价强化或 Target 内补强，理由成立且经 RED 输出与代码核实。Deviation 5（rustfmt 仅三目标文件）核实无范围外触碰。

**Acceptance Gaps**

无。R1 六场景全部由 RED→GREEN 见证（守卫前 3 RED/3 GREEN 行为锁定 → 实现后 6 GREEN）；既有 20 测试零校准；全量串行 1285/0/2（= 基线 1277 + 新增 8）；one-shot 时延方向性下降（12.35 → 5.85 ms，同场背靠背）；真实 CLI 路径 md5 旁证守卫在生产路径生效。

**Evidence**

- 采信 Act Response Verification Evidence 1–8（本会话新鲜运行，覆盖未失效）：定向四套件 6+3+9+8、`--lib wal::checkpoint` 5、全量串行 1285/0/2、clippy 退出码 0、`openspec validate --strict` 通过、时延前后对比、CLI md5 旁证。
- Plan 独立补充：四文件 diff 逐行阅读（本 Review Findings）；`git status` 核实三个回归套件与 `wal_recovery_large_test.rs` 零修改；修改面边界与 Act 声明一致（15 个修改 src 文件中 12 个为会话前 MS19 批次既有面）。

**Follow-up Decision**

接受（accepted）：Acceptance 全部满足，无阻塞 finding。Minor 1–4 记录不修复。无 Issue 候选落账需求（Act Experience Candidates 的 R26 前后对照测量方法为 Runbook 候选，按流程由用户决定是否调用 `openspec-experience-recorder`，本 Review 不落账）。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-09-26-ms18-oneshot-fixed-cost/iterations/001-runtime-flavor-readme/000-initial.md`（已按 Iteration Map 展开，Status: ready——change 级计划含 D5/D6 判据已于 2026-09-26 Gate 2 获用户批准）。
