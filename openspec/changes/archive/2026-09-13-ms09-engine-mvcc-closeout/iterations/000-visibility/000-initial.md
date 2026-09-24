# Iteration 000 / Cycle 000: 事务可见性域收口（I033 墓碑抑制 + I032 + Read Committed）

## Plan Context

- Status: ready
- Iteration: 000-visibility
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1-T7（tasks.md Iteration 000）
- Depends on: None
- Stable baseline: I033 探针序列扫描空集（运行期 + restart 两态）、未提交删除/回滚扫描语义正确、I032 未提交行重启不复活、RC 可配置且脏读排除；默认 RR 全量零回归
- Verification boundary: T1 测试套件 RED→GREEN + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: 版本链/扫描可见性族（`data_scan.rs`、`version_chain.rs`、`delete.rs`、`manager.rs`、`recovery.rs`）+ isolation 接线面（`database.rs`、`pipeline.rs`、扫描构造点）
- Deferred tasks: T10-T14（Iteration 001 NLJ）、T20-T22（Iteration 002 子查询缓存）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal.md 需求基线（Gate 1 批准 2026-09-13）+ delta specs `mvcc-tombstone-visibility`、`transaction-isolation-levels` + design D0-D4/D7/D9
- Excluded scope: NLJ、子查询缓存、未提交删除点查/回滚索引时序边界（Issue 候选）、RR 真快照化、写写冲突检测、SQL/CLI 隔离级别面

**Objective**

墓碑可见性评估按删除者提交状态正确抑制/回溯（I033 探针序列扫描空集），恢复面未提交行显式中性化不复活（I032 实施），Read Committed 经 lib API 可配且语句级已提交视图可验证；默认 RR 路径行为逐字节不变，892 基线零回归。

**Background**

I033 为 MS15-Rest 收尾实证的「静默错误结果」缺陷（跨进程 INSERT→UPDATE→DELETE 扫描重现 pre-update 版本）；I032 为恢复期 mark-uncommitted-aborted 空转（未提交行重启复活窗口）；RC 为 tasks.md MS09-T01 能力项。三者同属事务可见性域，I033 的墓碑判定是 RC 语义正确性的前提，故同一 Iteration 内先 I033/I032 后 RC。需求来源与用户决策见 proposal.md；技术裁定见 design.md。

**Investigation Facts**

- Current Baseline: HEAD e51c4a3 + MS16 未提交实施改动（工作区与 SNAPSHOT 记录一致，git status 已核对）；892 tests pass / 0 failed / 2 ignored（2026-09-13 MS16 收尾 Plan Review 独立复跑，覆盖范围材料未变化，采信）；基线含 `gc_test`/`version_chain_test`/`plan_exec_test` 的 MS16 校准。
- Current-State Evidence（全部 Plan 直接读码核实，file:line）：
  - `VersionHeader` 22B = create_tx_id(8) + commit_tx_id(8, `UNSET_TX_ID=u64::MAX` 即 None) + next_version(6)（`src/transaction/version_chain.rs:16-20`）；墓碑哨兵 `DELETED_TX_ID=u64::MAX-1`，`mark_deleted` 置哨兵、`is_deleted` 判哨兵（:71-79）；`commit()` 守卫保留哨兵（:56-67）。
  - DELETE 就地墓碑：`src/executor/delete.rs:56-83`——`index_manager.search` → `read_version_header` → `mark_deleted` → `update_version_header_in_data_page(.., &[])` → `clear_all_visible` → `index_manager.delete`（:85，即时移除）→ `record_version(old_rid)` → WAL `WalRecord::Delete{tx_id, table_name, row_id=old_rid}`（:96-103，格式现状）。row_id 为 None 时跳过标记但索引删除仍执行、返回 AffectedRows(1)（:61-85 现状契约）。
  - UPDATE 建独立新 slot：`src/executor/update.rs:143-149`（`VersionHeader::new(tx_id, None).with_next_version(old)`），Step 7 索引三分支（:188-210）。
  - I033 根因：`superseder_suppresses`（`src/executor/data_scan.rs:310-321`）`is_deleted() → false` 恒；墓碑 slot 自身 `:422-424` 跳过；前驱经 `build_superseded_map`（:213-267，target→(superseder_rid, create_tx)，多 superseder 最新创建者胜出 :249-257）+ `slot_is_superseded`（:273-300，沿 superseder 链上溯，`MAX_CHAIN_DEPTH=64`）不被抑制 → 产出 pre-update 版本。非墓碑抑制语义：已提交非墓碑抑制、未提交不抑制（:314-320）。
  - 扫描快照现状：生产全部 `snapshot: None`——`pipeline.rs:457`（Scan）/`:468`（DataScan）/`:483`（IndexScan）/`:496`（IndexScanAll）；`execute_stage_in_tx` 传 tx_id 仅 DML 消费（`pipeline.rs:269-270, 292`）；`Transaction.snapshot` 无消费方；`Snapshot::new` 生产仅 `TransactionManager::begin`（`manager.rs:94`）。
  - Snapshot 语义：`snapshot.rs:28-46` `is_visible`（已提交 ∧ create≤snap.tx_id ∧ 非活跃）+ `is_visible_self`（:50-52）+ `contains_active`（:55-57）。
  - DataScan 可见性：有快照时仅 `is_visible`（`data_scan.rs:428-437`）——**无 is_visible_self**（RC 下自身未提交写需放行，见 T6）；`page_all_visible` 惰性置位仅 `snapshot.is_some()`（:462-476，生产现状恒不触发）；`is_deleted` 检查（:422）先于 all_visible 分支（:428）。
  - Scan/IndexScan/IndexScanAll 有快照时经 `BufferPool::find_visible_version`（`buffer_pool.rs:266-362`，is_visible ∨ is_visible_self 于 :336-337；all_visible 快路径 :291-311；链走 next_version）——传 Some 即得 RC 点查语义，**无需改执行器本体**。
  - 事务管理：`TransactionManager { active_tx_ids: RwLock<HashSet>, tx_versions: RwLock<HashMap<tx, HashMap<table, HashSet<rid>>>> }`（`manager.rs:52-59`）；`commit` = WAL CommitTxn + `append_commit_and_wait` → `commit_mark_versions`（:239-254，逐 rid `write_commit_tx_id` + `clear_all_visible`；墓碑守卫使墓碑 rid 写入无害）→ 移出活跃集；`abort_cleanup_versions`（:262-305）= 索引修复（`find_key_by_row_id` → 有前驱 update / 无前驱 delete）+ `mark_deleted` 墓碑化（:299）；`active_transactions()`（:167-169）/`current_tx_id()`（:223-225）已有。
  - 恢复：`full_recover` 只重放 committed 记录（`recovery.rs:458/472`）；committed/uncommitted 集合自 WAL（:419-446）；Delete redo 臂就地墓碑（:741-792，SlotNotFound 容错 :782-784）；`mark_uncommitted_aborted`（:978-990）→ `BufferPool::mark_tx_aborted` no-op（`buffer_pool.rs:369-373`）；顺序 redo(:458-476) → mark(:478-479) → rebuild（门控 redo_count>0，:484；`rebuild_pk_indexes` 链尾回溯取「已提交 ∧ 非墓碑」版本 :817-912）。
  - Database 面：`Database { pub buffer_pool, table_manager, transaction_manager, wal_writer, wal_buffer, plan_cache, checkpoint_manager }`（`database.rs:17-25`）；`open(path)` 单一构造（:28）；`begin/commit/rollback/execute_in_tx(sql, &Transaction)`（:119-165）。
  - 测试入口：`plan_exec_test.rs` delete 流（索引移除后扫描空集——Option C 下不变）；`explicit_tx_test.rs`（abort 断言 `is_deleted()`——`mark_aborted` 下仍真）；`version_chain_test.rs`/`gc_test.rs`（UpdateExecutor 建链，不触 DELETE）；`tests/join_test.rs` 直连 Hash 执行器；跨进程探针先例 `tests/cli_test.rs`/`database_file_lock_test.rs`（本 change 用库级 close/reopen 等价复刻 + 可选 CLI 探针）。
- Code and Critical Path: 写路径 `delete.rs`（slot 化）→ 页面/索引/WAL；读路径 `data_scan.rs`（抑制判定 + RC 快照检查）与 `buffer_pool.rs::find_visible_version`（点查链走）；事务 `manager.rs`（abort 中性化）与 `transaction/mod.rs`（IsolationLevel）；恢复 `recovery.rs`（Delete redo + mark_uncommitted_aborted）；接线 `pipeline.rs::create_executor_from_plan`（快照参数）与 `database.rs`（open_with_isolation）。

**Implementation Guidance**

实施顺序即任务编号序：T1 RED 建立见证 → T2/T3/T4 运行期墓碑语义（互相咬合，一次 GREEN 观察）→ T5 恢复面 → T6 RC → T7 收尾。关键技术事实：(1) 墓碑 slot 用 `write_tuple_to_data_page(table_meta, VersionHeader::new(tx_id, None).with_next_version(rid).mark_deleted(), &[])`（空 tuple；SENTINEL 由 mark_deleted 置位）；(2) `commit_mark_versions` 对墓碑 rid 的写入被 `commit()` 守卫无害化，无需特判；(3) DataScan 活跃集合在首次 `next()` 时经 `TransactionManager::active_transactions()` 捕获一次存字段（抑制判定用扫描开始时点视图，避免逐 slot 锁读）；(4) `create_executor_from_plan` 签名扩展会强制所有调用点显式处理快照（含 `subquery_eval.rs:121-126`、`semi_join.rs:204-210`、`anti_join.rs:199-203`、`derived_scan` 物化）——全部传语句快照，编译器保证无遗漏；(5) RC 快照的 reader_tx_id：auto-commit 用 `current_tx_id()`（无需 WAL BeginTxn；单调性保证已提交事务 id ≤ 当前值），显式事务用 `tx.id()`；(6) `index_scan.rs:78-97`/`index_scan_all.rs:80-101` 已按 snapshot 分支，传 Some 即生效。

**Behavioral Change**

- 当前：DELETE 就地哨兵覆写最新版本 header；已提交墓碑不抑制前驱（I033：扫描重现 pre-update 版本）；未提交删除扫描回溯越过 pre-delete 版本；恢复期未提交行无标记（重启复活）；查询路径无快照（脏读、RR 名不副实的现状）。
- 目标：DELETE 写独立自描述墓碑 slot；抑制按删除者提交状态（aborted create_tx=0 不抑制 / 活跃不抑制 / 已提交抑制）；abort 全量 `mark_aborted` 中性化；恢复期 Delete redo 写墓碑 slot + uncommitted 行 `mark_aborted`；RC 模式每语句构造 `Snapshot::new(reader_tx_id, active_now)` 穿线全部扫描（含子查询/派生表重建），DataScan 检查 `is_visible ∨ is_visible_self`；默认 RR 行为除墓碑修复本身外逐字节不变。
- 接口/错误/状态语义：`Database::open_with_isolation` 新增（additive）；`DataScanExecutor::new` 增参；`create_executor_from_plan`（pub(crate)）增参；`VersionHeader::mark_aborted` 新增；`BufferPool::mark_tx_aborted` 移除；WAL 记录格式、页格式、公共 SQL 语义不变。

**Task Contracts**

### T1: RED 测试见证建立

- Requirement/Scenario: mvcc-tombstone-visibility R1-S1/S2、R2-S1、R3-S1、R4-S1/S2/S3；transaction-isolation-levels R2-S1
- Depends on: None
- Targets: 新 `tests/mvcc_tombstone_visibility_test.rs`、新 `tests/isolation_level_test.rs`
- Current behavior: 测试不存在；目标行为不可达（探针序列扫描 `[[1,10]]`、未提交删除扫描 `(1,10)`、未提交落盘行重启复活、RC 脏读）
- Required behavior: 测试表达目标行为（I033 序列空集、未提交删除见 pre-delete 版本、回滚恢复、restart 两态、I032 不复活、RC 脏读排除/语句间可见/自身写可见/auto-commit 等价）
- Required changes: 仅新增测试文件；库级探针复刻跨进程序列（各语句独立 auto-commit + close/reopen 变体）
- Preserve: 不修改任何产品代码与既有测试
- Forbidden: 为使测试可跑而预先改动实现；test-only 计数 hooks
- Test witness: `cargo test --test mvcc_tombstone_visibility_test --test isolation_level_test` → 记录 RED 输出（预期多失败；I033 序列断言 observed `[[1,10]]`）
- GREEN condition: T2-T6 完成后全套通过
- Verification: RED 输出记录于 Act Response
- Stop when: RED 形态与调查预测矛盾（如探针序列未复现 `[[1,10]]`）——返回 Plan

### T2: DELETE 墓碑 slot 化

- Requirement/Scenario: mvcc-tombstone-visibility R1（S1/S2）
- Depends on: T1
- Targets: `src/executor/delete.rs::DeleteExecutor::next`
- Current behavior: 就地 `mark_deleted` 覆写被删行 header（delete.rs:62-74）；记录 old_rid
- Required behavior: 写独立墓碑 slot（`VersionHeader::new(tx_id, None).with_next_version(rid).mark_deleted()`，空 tuple，经 `write_tuple_to_data_page`）；两个页面 `clear_all_visible`（墓碑页 + 被删页）；`record_version(tx, table, 墓碑rid)`；被删行 header 不再被改写
- Required changes: 墓碑 slot 写入 + 记录对象切换；其余流程逐行保持
- Preserve: 索引移除即时（:85 原样）；row_id None 分支行为逐字节（跳过标记、索引仍删、AffectedRows(1)）；WAL `WalRecord::Delete{tx_id, table_name, row_id=被删rid}` 格式与字段语义不变；`affected 1` 返回
- Forbidden: WAL 格式变更；索引时序变更；页格式变更；恢复格式变更（记录不变，redo 侧由 T5 适配）
- Test witness: T1 的 R1-S1/S2 用例由 RED 转 GREEN 方向；`plan_exec_test` delete 流保持绿
- GREEN condition: 墓碑 slot 存在且被删行 header 原样（可用 `read_version_header` 断言）
- Verification: `cargo test --test mvcc_tombstone_visibility_test`（部分转绿）+ 既有套件
- Stop when: 墓碑 slot 化破坏 `plan_exec_test`/`keyless_row_test` 既有断言且非校准可解——返回 Plan

### T3: 抑制判定重写 + mark_aborted + DataScan 活跃集

- Requirement/Scenario: mvcc-tombstone-visibility R2（S1-S3）、R1-S1；R3-S2
- Depends on: T2
- Targets: `src/executor/data_scan.rs::superseder_suppresses/slot_is_superseded/DataScanExecutor::new`；`src/transaction/version_chain.rs::VersionHeader::mark_aborted`（新增）；`src/pipeline.rs::create_executor_from_plan` DataScan 臂传参
- Current behavior: `is_deleted → false` 恒（I033）；无活跃集输入
- Required behavior: 抑制判定——`create_tx_id == 0`（aborted 标记）→ 不抑制；无快照：扫描开始时捕获的活跃集含 create_tx → 不抑制，否则抑制；RC 快照：`create_tx == snapshot.tx_id` → 抑制（自身删除）、`snapshot.contains_active(create_tx)` → 不抑制、否则抑制。非墓碑腿语义逐字节保持（已提交非墓碑抑制、未提交不抑制）
- Required changes: 判定函数重写 + 活跃集捕获（首次 `next()` 经 `active_transactions()` 一次存字段）+ 构造参数 `Option<Arc<TransactionManager>>`（生产 `Some`，数据源 `database.transaction_manager`）
- Preserve: `MAX_CHAIN_DEPTH`、`build_superseded_map` 收敛规则、墓碑 slot 自身跳过（:422）、非墓碑抑制、`is_deleted` 判定不面试探序优化
- Forbidden: 页级摘要禁用或全局退化；非墓碑可见性语义变化；快照判定外的 DataScan 行为变化
- Test witness: T1 的 I033 序列（`[[1,10]]`→空集）、未提交删除（`(1,10)`→`(1,99)`）、Z1/Z2 锚点
- GREEN condition: mvcc 套件运行期用例全绿
- Verification: `cargo test --test mvcc_tombstone_visibility_test`
- Stop when: 抑制判定需要非墓碑行为变化才能成立——返回 Plan

### T4: abort 中性化统一

- Requirement/Scenario: mvcc-tombstone-visibility R3（S1/S2）
- Depends on: T3
- Targets: `src/transaction/manager.rs::abort_cleanup_versions`
- Current behavior: 记录 rid 一律 `mark_deleted`（就地，create_tx 保留）
- Required behavior: 一律 `mark_aborted`（create_tx=0 + SENTINEL，next_version 保留）；DELETE 情形（记录对象已切换为墓碑 slot rid）索引修复对墓碑 rid 查不到键而跳过（现状同构），墓碑 slot 中性化后不抑制前驱
- Required changes: 墓碑化调用切换为 `mark_aborted`
- Preserve: `test_abort_cleanup_multi_table` 断言（`is_deleted()` 仍真）；「回滚无残留」契约；索引修复逻辑；abort 错误面（缺表 meta 报错）；`commit` 路径零变化
- Forbidden: DELETE 回滚的索引条目恢复（预存边界，Issue 候选，不做）；commit 期写入逻辑变化
- Test witness: T1 回滚用例 + `manager.rs` 单测 + `explicit_tx_test` 全套
- GREEN condition: 回滚后扫描产出回滚前最新已提交版本
- Verification: `cargo test --lib transaction` + 相关套件
- Stop when: abort 语义需要触及 commit 路径或索引恢复才能满足验收——返回 Plan

### T5: 恢复面（Delete redo slot 化 + I032 实施）

- Requirement/Scenario: mvcc-tombstone-visibility R4（S1/S2/S3）
- Depends on: T2（约定对齐）
- Targets: `src/wal/recovery.rs::redo_record`（Delete 臂）与 `mark_uncommitted_aborted`；`src/storage/buffer_pool.rs::mark_tx_aborted`（移除）
- Current behavior: Delete redo 就地墓碑（:765-781）；`mark_uncommitted_aborted` 空转（经 no-op）
- Required behavior: Delete redo 写独立墓碑 slot（`create_tx = record.tx_id`、SENTINEL、next→record.row_id）；被删 rid SlotNotFound 时维持跳过容错（镜像现状 :782-784）；`mark_uncommitted_aborted` 增加 `table_manager` 参数：迭代各表数据页链，`create_tx ∈ uncommitted ∧ commit == UNSET` 的 slot 改写 `mark_aborted`；`mark_tx_aborted` no-op 连同失实 TODO 注释删除，调用点改指新实现
- Required changes: redo 臂改写 + 未提交标记实施 + no-op 移除
- Preserve: redo committed-only 过滤（:458/:472）；处理顺序（redo → mark → rebuild，rebuild 门控 redo_count>0 不变）；`rebuild_pk_indexes` 的「已提交 ∧ 非墓碑」链尾回溯兼容墓碑 slot；`redo_count == 0` 时索引零重建不变（mark 步对新落盘未提交行的标记属 I032 修复目标行为，非回归）
- Forbidden: WAL 记录格式变更；rebuild 算法变更；checkpoint 流程变更
- Test witness: T1 的 restart 两态用例 + I032 复活用例 + `checkpoint_redo_reduction_test`/`wal_recovery_large_test`/`wal_recovery_replay` 既有套件
- GREEN condition: restart 后 I033 序列空集、未提交 DELETE 不生效、落盘未提交行不复活
- Verification: `cargo test --test mvcc_tombstone_visibility_test --test wal_recovery_large_test --test checkpoint_redo_reduction_test`
- Stop when: mark_uncommitted_aborted 需要 TableManager 之外的页枚举通道或破坏既有恢复套件——返回 Plan

### T6: Read Committed 配置与每语句快照

- Requirement/Scenario: transaction-isolation-levels R1（S1/S2）、R2（S1-S5）
- Depends on: T3（墓碑判定就绪）
- Targets: `src/transaction/mod.rs` 或新文件（`IsolationLevel`）；`src/database.rs`（`open_with_isolation` + `isolation` 字段，`open` 委托）；`src/pipeline.rs::execute_stage`（查询臂）/`execute_stage_in_tx`/`create_executor_from_plan`（签名 + 4 扫描臂 + 递归穿线）；`src/executor/data_scan.rs`（快照检查加 `is_visible_self`）；`src/executor/subquery_eval.rs`/`semi_join.rs`/`anti_join.rs`（语句快照字段与重建传参）
- Current behavior: 全扫描 `snapshot: None`；无隔离级别概念
- Required behavior: RC 模式——每条查询语句构造 `Snapshot::new(reader_tx_id, active_now)`（auto-commit：`current_tx_id()` + `active_transactions()`；显式事务：`tx.id()` + `active_transactions()`）并穿线 `create_executor_from_plan`（新参数 `Option<Snapshot>`）至全部扫描构造与子查询/派生表重建；DataScan 快照检查为 `is_visible ∨ is_visible_self`；RR（默认）两条路径维持 `None` 逐字节不变
- Required changes: 枚举 + open 变体 + 签名扩展 + 快照构造 + DataScan 一行检查扩展；Scan/IndexScan/IndexScanAll 本体零改动（已按 snapshot 分支）
- Preserve: 默认 RR 全部行为；DML 臂（tx_id 语义）；`Database::open` 与 `execute_in_tx` 公共签名；plan cache 语义（快照不进 plan）；`Snapshot`/`find_visible_version` 本体不变
- Forbidden: SQL/CLI 隔离面；运行中切换；写写冲突检测；RR 快照化
- Test witness: T1 的 isolation 套件（脏读排除/语句间可见·消失/自身写可见/auto-commit 等价）
- GREEN condition: isolation_level_test 全绿 + 全量 RR 基线零回归
- Verification: `cargo test --test isolation_level_test` + 全量
- Stop when: 快照穿线需要改变 plan cache 键或 DML 语义——返回 Plan

### T7: Iteration 收尾

- Requirement/Scenario: 全部（R3/R5 零回归类）
- Depends on: T1-T6
- Targets: 全局
- Current behavior: —
- Required behavior: mvcc + isolation 套件全绿；全量回归零修改通过；`cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` 0、`openspec validate --specs --changes` PASS
- Required changes: 如有既有测试依赖旧缺陷行为，按 design D7 在 delta spec 记录校准后实施（当前调查未预见）；否则仅验证
- Preserve: 校准不放宽断言语义
- Forbidden: 静默放宽既有断言
- Test witness: 全量输出（决定性片段）
- GREEN condition: 892 + 新增全绿 / 0 failed / 2 ignored
- Verification: `cargo test` 全量 + clippy/fmt/validate
- Stop when: 全量出现非校准可解失败——返回 Plan

**Invariants**

- 页格式 22B VersionHeader 与 WAL 记录格式不变；文件格式版本不变。
- 默认 RR 路径除墓碑修复本身外行为逐字节不变（892 基线零修改）。
- 公共 API 仅 additive（`open_with_isolation`、`mark_aborted`、构造参数）；`Database::open`/`execute_in_tx`/`begin/commit/rollback` 签名不变。
- 索引移除时序、`find_key_by_row_id` 修复逻辑、plan cache 键语义、DML 事务包裹不变。
- 身份型证据工程禁令（不引入计数 hooks、hash 链、run-id 等）。

**Non-goals**

- NLJ（Iteration 001）、子查询缓存（Iteration 002）。
- 未提交删除期间点查不可达 / DELETE 回滚后 PK 点查不可达（预存索引时序边界——Issue 候选，Act Response 报告不落账）。
- RR 真快照化、Serializable/SSI、写写冲突检测、SQL `SET TRANSACTION`、CLI 隔离面。
- GC 对墓碑链的回收策略（I038 域）。

**Acceptance**

1. `tests/mvcc_tombstone_visibility_test.rs`：I033 探针序列（运行期 + restart）扫描空集；未提交删除并发扫描见 pre-delete 版本；回滚后恢复；被删行 header 不被改写；落盘未提交行重启不复活——映射 R1-R4。
2. `tests/isolation_level_test.rs`：RC 脏读排除、语句间提交可见/消失、自身未提交写可见、auto-commit 等价、RC 打开可用——映射 transaction-isolation-levels R1/R2。
3. 全量回归零修改通过（≥892 + 新增，0 failed / 2 ignored）——映射 R3/R5 零回归类。
4. clippy/fmt/validate 全 0/PASS。
Acceptance 与 requirement/scenario/design/task/代码/测试映射见 tasks.md RTM（全部 Covered）。

**Verification**

- `cargo test --test mvcc_tombstone_visibility_test --test isolation_level_test`（目标套件，RED→GREEN）
- `cargo test`（全量，零回归门）
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- 决定性输出各取 ≤20 行写入 Act Response；Persisted Evidence 默认足以承载（见下）。

**Gate 2 Readiness**

- 无 Missing requirement（RTM 全 Covered）：PASS（tasks.md RTM）
- 无 Simplified 未批准：PASS
- 调查完整（当前实现/调用链/状态/测试/影响面均有 file:line 证据）：PASS（本 Plan Context Investigation Facts + design D0）
- 设计闭合（行为差异/接口/错误语义/关键选择已明确，D1-D4 有替代案否决理由）：PASS（design.md）
- 任务可执行（每任务有位置/行为变化/测试见证/停止条件）：PASS（Task Contracts T1-T7）
- 分轮合理（Iteration Plan 覆盖全部任务、依赖有序、平衡审计完成）：PASS（tasks.md）
- 追踪完整（RTM 无 Missing）：PASS
- 验证充分（RED→GREEN + 全量零回归 + 工具链门）：PASS（Verification）
- 无身份型证据工程：PASS（结果面测试，无计数/指纹机制）
- 无实质未知项留 Act：PASS（调查闭合全部实质问题；非实质项见 Risks）
- tasks/specs/design/Iteration/Cycle 一致：PASS
- Persisted Evidence 模式明确：PASS（none，见下）
- 用户批准计划：PASS（2026-09-13 Gate 2 用户批准「批准,交接 Act」）

**Persisted Evidence**

- Mode: none

理由：全部验收由可重复运行的测试与工具链命令承载，命令、决定性输出、退出码写入 Act Response 即满足公共规则 › 验证；不存在不可低成本重跑的决定。

- Budget: 不适用（none）

**Risks and Notes**

- `find_visible_version` 的 all_visible 快路径（`buffer_pool.rs:291-311`）在 RC 传快照后首次对生产查询生效：其正确性依赖「commit 清除 all_visible」（`manager.rs:250`）与「mark 步清除」（I032 新增标记处需同样 `clear_all_visible`——T5 契约内非实质细节，Act 按既有模式处理）；若发现既有 `check_page_all_visible` 语义与墓碑/未提交标记冲突，属非实质局部修正（记录于 Act Response），不改变契约语义。
- 既有测试如依赖旧缺陷行为（调查预判为零，D7），按 MS16 BH 先例在 delta spec 记录校准。
- 非实质留白（Act 自行决定，不改变契约语义）：墓碑 slot 的 `add_slot` 失败页分配路径复用 `write_tuple_to_data_page` 既有行为；活跃集捕获的存储形态（Vec/HashSet）；`IsolationLevel` 定义文件位置与 derive 集；错误文案措辞（新增错误面时对齐既有风格）。
- RC 快照读 `active_transactions()` 与语句执行间的并发窗口（事务恰在窗口内提交）：语义等价于语句开始时点视图，与既有无快照路径的并发语义同级，不做更强保证。

## Act Response

- Status: blocked

**Implemented**

T1-T5 全部完成并 GREEN；T6 按契约实施完毕、4/6 隔离验收场景通过，R2-S1/R2-S2 实证失败（Gate 6 阻塞，见 Blocker Handoff）。T7 的全量/工具链门已运行（RR 路径零回归 907 passed），但 Iteration 收尾被 T6 验收缺口阻塞，不声明完成。

- T1：新增 `tests/mvcc_tombstone_visibility_test.rs`（10 用例：R2-S1 运行期/restart、R1-S1/S2、R3-S1、R2-S2 Z1、R2-S3 Z2、R4-S2、R4-S3 I032、RR 基线守卫）与 `tests/isolation_level_test.rs`（6 用例，R1-S2/R2-S1-S5）。RED 观察到位：mvcc 套件 8 failed / 2 passed，失败形态与调查预测逐项一致（I033 序列/Z1 复现 `[[1,10]]`，未提交删除 `[[1,10]]`，header 被覆写，未提交落盘行复活 `[[7,70]]`）；isolation 套件编译 RED（`IsolationLevel`/`open_with_isolation` 不存在，6 errors）。
- T2：DELETE 墓碑 slot 化——`DeleteExecutor::next` 改为写独立墓碑 slot（`VersionHeader::new(tx_id, None).with_next_version(rid).mark_deleted()`，空 tuple，经 `write_tuple_to_data_page`），墓碑页与被删页 `clear_all_visible`，`record_version` 记录对象切换为墓碑 rid，被删行 header 不再改写；索引即时移除、row_id None 分支、`WalRecord::Delete{row_id=被删rid}` 格式逐字节保持；SlotNotFound 容错以存在性探测保留。GREEN：mvcc R1-S1/S2、R3-S1、R4-S2 转绿，`executor_test`/`plan_exec_test` delete 流保持绿。
- T3：`superseder_suppresses` 按 D2 重写（create_tx=0 不抑制；墓碑按删除者提交状态：无快照查扫描开始时活跃集、有快照按 self/active/committed 三态；非墓碑语义逐字节保持）+ `VersionHeader::mark_aborted`（新增，含单测）+ `DataScanExecutor` 新增 `Option<Arc<TransactionManager>>` 构造参数（生产 Some）与首次 `next()` 一次性活跃集捕获。GREEN：I033 运行期/restart、Z1、Z2 全部转绿。
- T4：`abort_cleanup_versions` 墓碑化由 `mark_deleted` 改 `mark_aborted`（create_tx=0 + SENTINEL，next_version 保留）；DELETE 情形经墓碑 rid 查键不中跳过索引修复（现状同构）。GREEN：回滚恢复用例转绿；`manager.rs` 单测（含 `test_abort_cleanup_multi_table` 的 `is_deleted()` 断言）全绿。
- T5：`redo_record` Delete 臂改为写独立墓碑 slot（create_tx = record.tx_id，SENTINEL，next→record.row_id；被删 rid SlotNotFound 容错镜像运行期）；`mark_uncommitted_aborted` 实施为经 `TableManager` 迭代各表数据页链，对 `create_tx ∈ uncommitted ∧ (commit UNSET ∨ 墓碑哨兵)` 的 slot 写 `mark_aborted`（I032 实施）；`BufferPool::mark_tx_aborted` no-op 及失实 TODO 注释删除。GREEN：mvcc 套件 10/10 全绿（restart 两态 + I032 不复活）。
- T6：`IsolationLevel`（transaction/mod.rs，默认 RR）+ `Database::open_with_isolation`（`open` 委托 RR）+ `Database::isolation` pub 字段 + `statement_snapshot` helper；`create_executor_from_plan` 增加 `Option<Snapshot>` 参数并穿线 4 扫描臂、全部递归臂、Semi/Anti/SubqueryEval 重建与 DerivedScan 物化；`execute_stage` 查询臂（reader=`current_tx_id()`）/`execute_stage_in_tx`（reader=`tx.id()`）按契约构造 RC 语句快照；DataScan 逐 slot 检查扩为 `is_visible ∨ is_visible_self`。结果：`rc_open_available`/`rc_statement_sees_delete_between_statements`/`rc_self_uncommitted_write_visible`/`rc_autocommit_matches_default_path` 4 场景通过；R2-S1/R2-S2 失败（结构性根因，见 Blocker Handoff）。

**Changed Files and Symbols**

产品代码（本次 change）：
- `src/executor/delete.rs` — `DeleteExecutor`（index_manager/table_name 字段 → `table_meta: Arc<TableMeta>`；`next` 墓碑 slot 化）
- `src/executor/data_scan.rs` — `DataScanExecutor`（+`tx_manager`/`active_tx_ids` 字段、`new` 增参、`superseder_suppresses` 重写、逐 slot `is_visible_self` 扩展）
- `src/transaction/version_chain.rs` — `VersionHeader::mark_aborted`（新增）+ 单测
- `src/transaction/manager.rs` — `abort_cleanup_versions`（mark_aborted）
- `src/wal/recovery.rs` — `redo_record` Delete 臂（墓碑 slot 化）、`mark_uncommitted_aborted`（实施 + `table_manager` 参数）
- `src/storage/buffer_pool.rs` — `mark_tx_aborted` no-op 移除
- `src/transaction/mod.rs` — `IsolationLevel`（新增）
- `src/transaction/snapshot.rs` — `#[derive(Clone)]`（加性，见偏差 1）
- `src/database.rs` — `Database::isolation` 字段、`open_with_isolation`（`open` 委托）、`statement_snapshot`
- `src/pipeline.rs` — `create_executor_from_plan`（+snapshot 参数，全部臂穿线）、`execute_stage` 查询臂/DML 臂、`execute_stage_in_tx`
- `src/executor/join_related_config.rs` — `JoinRelatedConfig.snapshot` 字段
- `src/executor/semi_join.rs` / `src/executor/anti_join.rs` — `snapshot` 字段 + 关联重建传参
- `src/executor/subquery_eval.rs` — `snapshot` 字段 + 两个求值路径传参

测试：新增 `tests/mvcc_tombstone_visibility_test.rs`、`tests/isolation_level_test.rs`；适配 `tests/executor_test.rs`（`test_delete_executor` 夹具升级为真实表 + T2 GREEN 断言；5 处 `Database` 字面量补 `isolation`）、`tests/plan_exec_test.rs`（2 处 `DeleteExecutor::new` 调用点）、`tests/prefetch_test.rs`（调用点）。
Bench 适配：`benches/visibility_bench.rs`、`benches/data_scan_bench.rs`。

**Deviations from Plan**

1. `Snapshot` 增加 `#[derive(Clone)]`（T6 Preserve 字面为「`Snapshot`/`find_visible_version` 本体不变」）：契约自身要求快照按值穿线并被重建型执行器存储（owned），不可 Clone 无法实现；derive 为纯加性、可见性判定语义零变化，`find_visible_version` 本体未动。非实质实现面偏差，记录供 Plan Review 追认。
2. T5 中性化条件由契约字面「`commit == UNSET`」扩为「`commit UNSET ∨ 墓碑哨兵`」：T2 起未提交删除以独立墓碑 slot 表达（commit=SENTINEL），按契约字面则落盘未提交墓碑重启后仍抑制整链、R4-S2「未提交 DELETE 不生效」不可能转绿；delta spec R4「未提交事务遗留版本 SHALL 显式中性化标记」的正文涵盖墓碑。属契约条件与 spec 正文的不一致，按 spec 正文实施。
3. T2 SlotNotFound 容错形态：契约未列明 slot 缺失时是否写墓碑；实施为保留既有容错（存在性探测，缺失则跳过墓碑写入，仅索引清理）——镜像修复前 :76-80 语义，错误面逐字节不变。
4. `DeleteExecutor::new` 签名以 `table_meta: Arc<TableMeta>` 替换 `index_manager`+`table_name` 两参（墓碑写入需 data page tail；二者均可从 TableMeta 派生）；调用点 pipeline.rs + 3 处测试同步。
5. T1 夹具修正：R4-S2 用例 drop 前增加 `wal_buffer.do_flush()` + `buffer_pool.flush_all()`（纯 drop 丢失 catalog 页导致 reopen 时 redo 报 `table not found`，RED 落在 open 失败而非断言行为；flush 后 RED 正确落在 `[[1,10]]` 断言）。
6. `executor_test::test_delete_executor` 夹具从「独立 IndexManager + 幽灵 rid」升级为「真实 TableManager 表 + 真实 tuple + 索引条目」并补 T2 GREEN 断言（被删行 header 原样）——`DeleteExecutor` 不再接受独立 IndexManager。
7. `DataScanExecutor::new` 新参数追加在参数表末尾（契约未定位置）；21 处调用点机械同步（测试/bench 传 None，生产传 Some）。
8. `SubqueryEvalExecutor::new` 加 `#[allow(clippy::too_many_arguments)]`（8 参，先例 update.rs:41）。
9. R2-S3「Z2 delete→update 既有形态」按「INSERT→DELETE→INSERT→UPDATE」建链实施（被删行经索引不可再 UPDATE，字面 delete→update 不可达）；该用例修复前后均绿（守卫）。
10. `cargo fmt` 对本次触碰文件做了机械重排（clippy bool-literal 修复后）。

**Blocker Handoff**

- **命中 Gate**：T6 GREEN condition / Gate 5（Task Contract 无法覆盖达到既有 Acceptance 所需的工作）。
- **task/step**：T6「Read Committed 配置与每语句快照」；验收场景 transaction-isolation-levels R2-S1、R2-S2。
- **Plan 预期**：按 Plan Context T6 契约构造快照（auto-commit reader=`current_tx_id()`、显式事务 reader=`tx.id()`，DataScan 检查 `is_visible ∨ is_visible_self`）即可满足 delta spec R1/R2（RTM 全 Covered）；`isolation_level_test` 全绿。
- **实际情况**：4/6 通过；两个场景实证失败——
  - **R2-S1（脏读未排除）**：`rc_dirty_read_excluded` 实测 RC auto-commit 扫描产出未提交行 `[[7,70]]`。根因：auto-commit reader_tx_id 取 `current_tx_id()`（最近分配值），与仍活跃的写事务 id 相同（其间无新事务分配），`Snapshot::is_visible_self(create==reader ∧ commit=None)` 误判为「自身写」放行。
  - **R2-S2（语句间提交不可见）**：`rc_statement_sees_commit_between_statements` 实测事务内第二条语句返回 `[]`（期望 `[[5,50]]`）。根因：显式事务 reader_tx_id 固定为 `tx.id()`（begin 时点），语句间才提交的事务若在 reader 事务 begin 之后开始，其 `create_tx_id > reader_tx_id` 被 `Snapshot::is_visible` 规则 2 拒绝。
- **结构性根因**：`Snapshot` 单一 `tx_id` 字段同时承担两个不相容角色——(a) 可见性高水位（`is_visible` 规则 2：create > tx_id → 不可见）与 (b) 自身身份（`is_visible_self`：create == tx_id）。对 RC 语句快照不存在同时满足 (a)(b) 的单值：auto-commit 取 `current_tx_id()` 满足 (a) 但在 (b) 上与活跃事务 id 撞车（脏读）；显式事务取 `tx.id()` 满足 (b) 但在 (a) 上排除 begin 晚于 reader 的事务的提交。T6 Preserve 明文禁止改 `Snapshot`/`find_visible_version` 本体 → 契约无法达到既有 Acceptance。注：`find_visible_version`（buffer_pool.rs:336-337）已内建 `is_visible ∨ is_visible_self`，同样的撞车/高水位问题对 RC 点查同样成立，修复方案须一并覆盖。
- **影响**：T6 验收未达成；T7 收尾门（要求 isolation 套件全绿）不满足，Iteration 不声明完成。T1-T5（I033 + I032 核心）完整收口且 RR 默认路径全量零回归，不受阻塞影响。
- **修复方向供 Plan 裁定**（均超出当前契约授权，未实施）：
  1. **Snapshot 自身身份与高水位分离**（推荐评估）：为 Snapshot 增加独立高水位（或自身 id）字段/构造器——`is_visible` 按高水位判定、`is_visible_self` 按自身事务 id 判定；`find_visible_version` 仅消费 Snapshot 方法、本体不动；RR 路径仍传 None 零回归。需 Plan 显式豁免/修订 T6 Preserve 的「Snapshot 本体不变」。
  2. 自身 id 与快照分离穿线（Snapshot 不动，另穿 reader self-id）：触碰面更大，同样实质修改 Preserve 约束的接线方式。
  3. 缩窄验收语义（放弃「begin 晚于 reader 的事务提交可见」）：属 spec 裁定，不建议静默采纳。
- **任务状态**：T1-T5 completed；T6 implemented-per-contract / acceptance unmet；T7 not completed（全量与工具链门已运行并记录，但 Iteration 收尾被阻塞）。
- **工作区状态**：MS16 既有待提交改动之上叠加本 Iteration T1-T6 改动（文件清单见 Changed Files）；无 plan 外修改；clippy 0 / fmt 0 / openspec validate 28 PASS。
- **已通过 Gate**：Gate 3（T1 RED 见证 + 各任务修改前见证）；Gate 4（逐任务 spec/code review + Response 前完整 diff review，结论见 Self-Review）。

**Blocker Resolution**

None（尚未恢复）

**Self-Review**

- Plan compliance: T1-T5 逐项符合 Task Contract；T6 按契约实施、验收缺口为契约与 spec 冲突（非 Act 偏离）
- Full diff reviewed: yes（Phase 4 完整 diff 审查：不变量——页格式 22B/WAL 记录格式/文件格式版本未动；公共 API 仅 additive；plan cache 键语义未动；索引时序未动；无身份型证据机制；无 plan 外修改；测试不因错误原因通过——2 个失败为真实验收缺口而非测试缺陷）
- Critical findings unresolved: 0（R2-S1/S2 缺陷属 Plan 层设计冲突，已按 Gate 6 交接，非 Act 范围内可修复项）
- Important findings unresolved: 0
- Minor findings unresolved: 2
  - M1：`mark_uncommitted_aborted` 对 `redo_count == 0` 的干净打开也遍历各表页链（uncommitted 为空时 O(catalog scan) 早退，行为无变化；大库冷打开新增一次页链扫描的 I/O 代价未量化——如需可后续加 catalog 级未提交标记）。
  - M2：`statement_snapshot` 的 `active_transactions()` 与语句执行间的并发窗口沿用 Plan Context Risks 的既有裁定（语句开始时点视图，同级语义），未新增保证。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 RED（mvcc） | `cargo test --test mvcc_tombstone_visibility_test`（T2-T5 前） | `8 failed; 2 passed`；I033/Z1 断言 observed `[[Number(1), Number(10)]]`、I032 observed `[[7,70]]` | 目标缺陷全部复现 | RED 与调查预测一致 PASS |
| T1 RED（isolation） | `cargo test --test isolation_level_test`（T6 前） | `E0432/E0599` ×6（`IsolationLevel`/`open_with_isolation` 不存在） | RC API 缺失 | 编译 RED PASS |
| T2-T5 GREEN | `cargo test --test mvcc_tombstone_visibility_test`（T5 后） | `10 passed; 0 failed`（exit 0） | mvcc-tombstone-visibility R1-R4 全场景 | GREEN PASS |
| 邻接回归 | `cargo test --test wal_recovery_large_test … --test version_chain_test`（T5 后，两批） | 全部 `ok`（9+5+6+8+4+8+7 / 39+24+3+7+8+14+4+6+3+9+3+4+4+2+14+5） | 恢复/GC/版本链/MS16 面 | 零回归 PASS |
| 全量回归 | `cargo test --no-fail-fast`（收尾前） | `TOTAL passed: 907 failed: 2 ignored: 2`；仅 `rc_dirty_read_excluded`、`rc_statement_sees_commit_between_statements` FAILED | 892 基线 + mvcc 10 + isolation 6 + 新单测 | RR 零回归 PASS；RC 缺口即阻塞项 |
| 隔离套件（T6 后） | `cargo test --test isolation_level_test` | `4 passed; 2 failed`（失败输出见 Blocker Handoff） | transaction-isolation-levels R1/R2 | R2-S1/S2 FAIL（阻塞） |
| clippy | `cargo clippy --all-targets -- -D warnings` | 0 error | 全目标静态分析 | PASS |
| fmt | `cargo fmt --check` | 0 diff | 全仓格式 | PASS |
| validate | `openspec validate --specs --changes` | `Totals: 28 passed, 0 failed` | change/specs 结构 | PASS |

**Persisted Evidence**

None required（Persisted Evidence 模式 none；全部验收由可重复命令承载，输出已摘录于上；阻塞证据即两条失败用例本身，`cargo test --test isolation_level_test` 可随时复现）

**Experience Candidates**

- Runbook 候选（可重复、经端到端验证的崩溃态模拟手法）：「无 close/checkpoint 的进程终止 + 确定性页/WAL 落盘」——`wal_buffer.do_flush()` + `buffer_pool.flush_all().await` 后 drop `Database`，可在库级测试中确定性地构造「未提交数据已落盘、WAL 未截断」的崩溃等价态（避免 checkpoint 截断 WAL 造成的状态歧义与真崩溃注入）。证据：`tests/mvcc_tombstone_visibility_test.rs` `flushed_uncommitted_insert_does_not_resurrect`（RED→GREEN 全程）。普通测试夹具与一次性命令按记录边界不构成候选的判断已被本手法的可复用性（任何 MVCC/恢复测试可直接复用）超越，登记与否由 Recorder/用户裁定。

**Remaining Issues**

- 阻塞项本体：RC 快照构造的 Snapshot 单 id 结构性缺口（见 Blocker Handoff——属当前 change 范围内，非范围外 Issue 候选，不落账）。
- 预存边界（Plan Context 已列，Act 未触碰、维持 Issue 候选不落账）：未提交删除期间与 DELETE 回滚后的 PK 点查不可达。
- 新增观察（预存、本 change 未修复、建议 Plan Review 记录）：`Database::open` 恢复后事务 id 分配器未越过已恢复 id（open 中 `_max_tx_id` 计算后被丢弃），重启后 tx id 复用；当前仅快照路径的 `is_visible` 规则 2 受其潜在影响，生产 RR 无快照路径无消费方——与阻塞修复方向 1 的字段语义设计相关，届时一并裁定。

**Commit or Diff Reference**

未 commit（待用户触发；对照基线 d8a244f + 工作区既有 MS16 未提交改动）。本 Iteration 改动可按 `git status` 中 src 13 文件 + tests 3 适配 + tests 2 新增 + benches 2 适配识别。

## Plan Review

- Review Result: replan-required

**Findings**

- **F1（阻塞，裁定依据）**：T6 验收缺口确认为 Plan 层设计冲突。Act 的结构性根因分析经独立核实成立：`Snapshot` 单一 `tx_id` 字段双角色（`is_visible` 规则 2 高水位 `snapshot.rs:41-44` ∨ `is_visible_self` 自身身份 `snapshot.rs:56-58`），对 RC 语句视图不存在同时满足二者的单值；两失败场景由 Plan 本会话复现（`isolation_level_test` 4 passed / 2 failed，`rc_dirty_read_excluded` observed `[[7,70]]`、`rc_statement_sees_commit_between_statements` observed `[]`，形态与 Blocker Handoff 逐字一致）。delta spec R2-S1/S2 场景表达忠实，测试本身无缺陷。
- **F2（PLAN-INVALID 本体）**：T6 Preserve「`Snapshot`/`find_visible_version` 本体不变」与 D4 的语句视图语义互相矛盾——高水位与自身身份分离必须触碰 `Snapshot` 结构；契约在既有 Acceptance 下不可满足，非 Act 可修复项。修复方向 1（结构分离）经代码核实可行：消费面全部经方法（`find_visible_version` `buffer_pool.rs:336-337`、DataScan 抑制臂），`Snapshot::new` 语义保留则 RR 面与全部既有单测/bench 零改动。
- **F3（新增并入项）**：Act 遗留观察核实属实——`open_with_isolation` 计算 `_max_tx_id` 后丢弃（`database.rs:74-81`），重启后分配器从 0 重来（`tx_id.rs:8-20`）。它不仅影响 RC 重启后视图（高水位覆盖不了已恢复 committed id），还是高水位论证「≤ current 即 committed/aborted/active」的健全性前提（复用 id 使新事务 abort 的 `mark_aborted` 可误伤历史同 id 版本）；并入修订设计 D10。
- **F4（非阻塞，追认）**：T1-T5 全部按契约实施——diff 逐项核对（delete.rs 墓碑 slot 化、superseder_suppresses 重写、mark_aborted、abort 中性化、恢复面 redo slot 化 + mark_uncommitted_aborted 实施 + no-op 移除）与 Act Response 一致；`mvcc_tombstone_visibility_test` 10/10 复跑绿；I033/I032 核心收口有效。偏差 1（Clone derive）与 10 项偏差记录完整、均非实质。
- **F5（非阻塞 Minor）**：偏差 2（T5 中性化条件扩为 `commit UNSET ∨ 墓碑哨兵`）分类 PLAN-OMISSION——契约未列墓碑形态，spec 正文覆盖，按正文实施正确；偏差 9（Z2 守卫用例建链形态适配）前后均绿，非实质。Act M1（`mark_uncommitted_aborted` 干净打开 I/O 成本）实际优于描述：uncommitted 为空时在 catalog scan 前早退（`recovery.rs:1004-1006`），无需动作。Act Risks 的 all_visible 清旗关切已由既有 helper 覆盖（`data_page.rs:144`）。

**Deviation Classification**

- 阻塞本体：**PLAN-INVALID**（D4 快照设计 + T6 Preserve 约束无法达到既有 Acceptance R2-S1/S2；修复需修订设计与执行契约）
- 偏差 2：**PLAN-OMISSION**（T5 契约条件未列墓碑哨兵形态；spec 正文涵盖，实施正确）
- 偏差 1、3、4、5、6、7、8、10：非实质实现面偏差，追认（Clone derive 为契约自身要求的必要后果；SlotNotFound 容错镜像既有语义；签名重构/夹具升级/参数位置/fmt 均局部）
- 偏差 9：非实质（Z2 守卫用例形态适配，修复前后均绿）
- 无 ACT-DEVIATION、BASELINE-CHANGED、NEW-EVIDENCE

**Acceptance Gaps**

- transaction-isolation-levels R2-S1（他事务未提交写不可见）：FAIL——实测扫描产出 `[[7,70]]`
- transaction-isolation-levels R2-S2（语句间提交立即可见）：FAIL——实测 `[]` ≠ `[[5,50]]`
- T7 收尾门（isolation 套件全绿）未满足；T1-T5 对应场景与全量 RR 零回归（907 passed）已满足

**Convergence**

N/A（首次 Review；相比父 Cycle 无更早 Review 版本）

**Evidence**

- 复现：`cargo test --test isolation_level_test --test mvcc_tombstone_visibility_test` → isolation `4 passed; 2 failed`（失败输出与 Act Blocker Handoff 一致）、mvcc `10 passed; 0 failed`（本会话退出码 1/0）
- 代码核实：`snapshot.rs:10-58`（单 id 双角色）、`database.rs:74-81`（`_max_tx_id` 丢弃）、`database.rs:119-127`（statement_snapshot 构造）、`buffer_pool.rs:336-337`（方法消费面）、`data_page.rs:144`（清旗 helper）、`tx_id.rs:8-20`（分配器）
- diff 核对：`git status`/`git diff --stat` 与 Act Changed Files 一致（工作区为 MS16 + 本 Iteration 叠加态，无 plan 外修改）；Act 全量 907/2/2 结论按公共规则 › 验证采信（材料未变化 + 只读基线检查通过 + 决定性套件已独立复跑）

**Follow-up Decision**

必须重新规划（replan-required）：修复 R2-S1/S2 需要修订设计（Snapshot 自身身份/高水位分离 + 分配器水位推进，design D10 已记录）并解除 T6 Preserve 对 `Snapshot` 本体的禁令——超出 000-initial 契约授权，不属于可留在当前 Cycle 的有限修复（当前契约明文禁止触碰 `Snapshot` 本体），也不属 rework（不是既有契约内的新 repair item，而是执行契约本身失效）。已创建 `iterations/000-visibility/001-replan.md`（T6-R1/R2/R3/R4 修订契约，Plan Context 置 draft 待 Gate 2 用户批准）；design.md 增 D10、tasks.md T6 行与 RTM 已同步。父 Cycle 的 T1-T5 实施与验证结论由 001-replan Inherited scope 采信继承，不重做。

**Iteration Plan Update**

Iteration 000 的目标、验收边界与任务归属不变（T1-T7）；T6 内容按 D10 修订（Snapshot 结构修订 + 分配器水位推进，tasks.md 已同步标注）。Iteration 001/002 不受影响。

**Next Cycle**

`iterations/000-visibility/001-replan.md`

**Next Iteration**

<None>
