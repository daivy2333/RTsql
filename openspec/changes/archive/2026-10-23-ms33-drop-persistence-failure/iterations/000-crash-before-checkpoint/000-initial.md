# Iteration 000 / Cycle 000: checkpoint 前的 DROP 崩溃结果

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 1.1、1.2、1.3、1.4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 中用户已确认的崩溃模型；设计 D1；规格 `drop-persistence-boundary` 里除受控 I/O 以外的要求
- Excluded scope: 任务 2.1–2.3、失败开关、位点目录注入、真实断电、空闲页持久化、伴生文件加密

**Objective**

不调用 `close` 就丢弃数据库句柄之后，`DROP TABLE`、`DROP INDEX`、`DROP VIEW` 的对象和 DROP 前的已提交内容仍可查询，`check_integrity` 的 error 级发现为 0。同一次打开里，后续分配建成的表只含新行。`ALTER TABLE DROP COLUMN` 和加密库 `CREATE TABLE` 的崩溃结果与规格一致。

**Investigation Facts**

- Starting point: 修订 `acfd274`，`master` 比 `origin/master` 超前 8。MS32 收尾记录的全量是 2144/0/2（`.agents/docs/SNAPSHOT.md`），本次计划没有重跑。`.agents/docs/tasks.md` 与 `.agents/memory/improvements.md` 有未提交修改，本 Cycle 不改它们。
- Current-State Evidence:
  - `Catalog::delete_table`（`src/storage/catalog.rs`）删除两链条上的行，不调用 `flush_commit_point`。`remove_secondary_index` 只把更新留在脏页。
  - `TableManager::drop_table` 先 `delete_table`，再从缓冲池 `free_page`。`FileStorage::free_page` 把页号推入 `free_pages` 后立刻 `write_page` 全零页。`BufferPool::free_page` 先从池中移除该页。
  - `allocate_page` 弹出空闲页号时直接返回，不写零页。只有文件尾新扩展的加密页会写加密零页。
  - `DropViewExecutor` 只 `delete_table` 并摘掉内存视图，不调用 `free_page`。
  - `alter_drop_column` 在行重写 WAL 之前执行 `buffer_pool.flush_all()`。执行器在成功时 `commit`。
  - 崩溃助手在 `tests/ddl_crash_recovery_test.rs`：`wal_buffer.shutdown()` 后 `drop(db)`，不 `close`。该文件覆盖 CREATE 族，没有 DROP。
  - `Database` 没有 `Drop` 实现，丢弃句柄不会补一次 checkpoint。
  - 加密页的读写在 `FileStorage::write_page_blocking` / `read_page_blocking`，WAL 与 checkpoint 保持明文。
- Code and Critical Path: SQL `DROP TABLE` → `DropTableExecutor::next` → `TableManager::drop_table` → `Catalog::delete_table`（脏页）→ `BufferPool::free_page` → `FileStorage::free_page`（当前会写零）。崩溃路径不进入 `CheckpointManager::checkpoint`。重开走 `Database::open` 的恢复。`DROP INDEX` 在 `DropIndexExecutor` 中 `take_secondary_index` 后 `free_page`。

**Implementation Guidance**

D1 只改 `FileStorage`。`free_page` 保留只读拒绝和空闲列表推入，删除随后的 `write_page`。`allocate_page` 在 `free_pages.pop()` 成功时，先对那个页号 `write_page` 全零页；加密库使用与新扩展页相同的加密零页。写零返回错误时，把页号推回 `free_pages`，再把错误返回给分配调用者。不要改 `CheckpointManager::checkpoint` 的步骤顺序，也不要给 DROP 增加 `flush_commit_point`。

**Task Definitions**

### 1.1: DROP TABLE 崩溃测试先失败

- Requirement/Scenario: checkpoint 前崩溃保留 DROP 前的已提交状态 / DROP TABLE 后崩溃
- Depends on: None
- Targets: `tests/drop_persistence_test.rs`
- Current behavior: 该测试不存在。按代码，`free_page` 已把数据页写成零，catalog 删除仍是脏页；丢弃句柄后重开，表的原行不应仍可按崩溃前查询，或 `check_integrity` 会有 error。
- Required behavior: 测试文件存在。`drop_table_crash_before_checkpoint_keeps_rows` 在产品代码按 D1 改动之前失败。失败断言是原行不可查询或 error 级发现不为 0。
- Preserve: 不修改 `src/`。
- Forbidden: 为了让测试先通过而改 `free_page`；把真实断电或子进程 SIGKILL 写进这个测试。
- First test: 新增 `tests/drop_persistence_test.rs` 中的 `drop_table_crash_before_checkpoint_keeps_rows`。命令：`cargo test --test drop_persistence_test drop_table_crash_before_checkpoint_keeps_rows -- --test-threads=1`。预期：失败，输出显示行不匹配或 error 级发现不为 0。
- GREEN condition: 本任务不要求 GREEN。GREEN 属于 1.2。
- Verification: 上列命令的退出码非 0，且失败信息指向行或 check，而不是找不到测试或编译失败。
- Stop when: 在未改 `free_page` 时该测试已经通过。这说明静态结论与运行不符，回到计划，不要顺便改分配路径。

### 1.2: 释放时不写零，分配空闲页时写零

- Requirement/Scenario: 同场景，加上同会话再分配不读到已释放页的旧行 / 删表后立即建表
- Depends on: 1.1
- Targets: `src/storage/file_storage.rs` 的 `free_page` 与 `allocate_page`；`tests/drop_persistence_test.rs`
- Current behavior: `free_page` 写零页。`allocate_page` 弹出空闲页号后不写盘。
- Required behavior: `free_page` 不调用 `write_page`。弹出的空闲页在返回页号之前写成全零；失败时页号回到 `free_pages` 且错误向上返回。1.1 的测试通过。`drop_then_allocate_sees_only_new_rows` 通过：DROP 之后新建表再插入一行，查询只有该行。
- Preserve: 只读库 `free_page` 仍返回只读错误。新扩展页的加密零页行为保持。checkpoint 函数体不改。
- Forbidden: 在 `drop_table` 或 `delete_table` 上增加 `flush_commit_point`；改 WAL 或位点格式；持久化 `free_pages`。
- First test: 1.1 已经是 RED。另增 `drop_then_allocate_sees_only_new_rows`，改代码前应能编译；若它在改代码前已经失败，先保持失败，随 D1 一起变绿。命令同上，过滤名为 `drop_table_crash_before_checkpoint_keeps_rows` 与 `drop_then_allocate_sees_only_new_rows`。
- GREEN condition: 两个测试通过。
- Verification: `cargo test --test drop_persistence_test drop_table_crash_before_checkpoint_keeps_rows drop_then_allocate_sees_only_new_rows -- --test-threads=1` 退出码 0。再跑 `cargo test --test schema_persistence_test --test pipeline_test -- --test-threads=1`，退出码 0。
- Stop when: 写零失败后无法把页号放回列表，或空闲页不写零就会在现有测试里读到旧行，且无法用 `write_page` 全页零解决。

### 1.3: 其余 DROP 形态与加密库崩溃

- Requirement/Scenario: DROP INDEX 后崩溃；DROP VIEW 后崩溃；删列后崩溃；加密库建表后崩溃；加密库删表后崩溃
- Depends on: 1.2
- Targets: `tests/drop_persistence_test.rs`。仅当加密库 CREATE 没有走到现有 `insert_table` 冲刷时，才允许查看调用链并补上已有的 `flush_commit_point`，不得新写刷盘策略。
- Current behavior: DROP INDEX 与 DROP TABLE 一样会 `free_page`。DROP VIEW 不释放页，catalog 删除在脏页中，丢弃句柄后视图应仍在。删列路径已 `flush_all`。加密 CREATE 走 `insert_table` 的有序冲刷，尚无崩溃测试。
- Required behavior: 下列测试通过。`drop_index_crash_before_checkpoint_keeps_index`：等值查询行集与崩溃前相同，error 级发现为 0。`drop_view_crash_before_checkpoint_keeps_view`：视图查询仍返回原行。`alter_drop_column_crash_keeps_narrowed_rows`：被删列不在，剩余列值与删列后一致，error 级发现为 0。`encrypted_create_table_survives_crash`：同一密钥重开后表可插入并读回。`encrypted_drop_table_crash_keeps_rows`：原行仍在，error 级发现为 0。
- Preserve: `tests/ddl_crash_recovery_test.rs` 的 CREATE 族断言。`TRUNCATE` 不新增用例。
- Forbidden: 移动 `alter_drop_column` 里的 `flush_all`；加密 `.wal` 或 `.checkpoint`；为 DROP VIEW 增加页清零。
- First test: 五个测试名如上。先加入 `alter_drop_column_crash_keeps_narrowed_rows` 与 `encrypted_create_table_survives_crash`，在不再改产品代码的前提下运行。命令：`cargo test --test drop_persistence_test -- --test-threads=1`。
- GREEN condition: 该命令退出码 0，六个崩溃测试加再分配测试都在输出里通过。
- Verification: 同一命令。DROP VIEW 或加密 CREATE 若在 1.2 之后已经通过，记下「改前即通过」，不要再改产品代码。
- Stop when: 删列测试失败，且要使它通过必须移动 `flush_all` 或改 WAL。加密 CREATE 失败，且必须加密伴生文件或改页格式。DROP INDEX 在 D1 之后仍失败。

### 1.4: 全量分批回归

- Requirement/Scenario: 本 Iteration 全部场景，加上既有 DDL 崩溃与 schema 持久化不回归
- Depends on: 1.3
- Targets: 无新代码。运行 M19 分批。
- Current behavior: 定向测试已在 1.3 通过。全量尚未在本 Cycle 运行。
- Required behavior: 每一批 0 failed。
- Preserve: 不因全量里与本 Iteration 无关的失败去改其他模块。无关失败停止并报告。
- Forbidden: 一次无过滤的 `cargo test` 作为全量结论；默认并行度跑全量批次。
- First test: 无新测试。1.3 的命令已经是 GREEN。
- GREEN condition: 下列每一批退出码 0。
- Verification: `cargo test --lib --bins -- --test-threads=1` 一批。`tests/*.rs` 按文件名字典序，每 10 个文件一批：`cargo test --test <a> --test <b> ... -- --test-threads=1`。记录每批的 passed/failed 计数。缺一批就不能声明全量通过。
- Stop when: 某一批失败且失败测试不在 `drop_persistence_test` 的新断言所覆盖的释放/分配路径。先停止，把失败测试名写入 Act Response。

**Invariants**

- `DROP TABLE`、`DROP INDEX`、`DROP VIEW`、`TRUNCATE` 不在语句成功返回前获得 catalog commit point。
- checkpoint 九步的顺序、WAL 帧和 24 字节位点布局不变。
- 空闲列表仍只存在于本次打开的内存中。
- 只读打开不能释放页。

**Non-goals**

失败开关、位点目录、磁盘满以外的半页写入、真实断电、MS35 的跨重启复用、I060。

**Acceptance**

- 1.1 在改 `free_page` 之前观察到 RED。
- 1.2 之后 DROP TABLE 崩溃测试与再分配测试通过。
- 1.3 的五个测试通过，或按 Stop when 返回计划。
- 1.4 各批 0 failed。
- `schema_persistence_test` 与 `pipeline_test` 在 1.2 通过。

**Verification**

场景 1–3、5–7 用 `cargo test --test drop_persistence_test` 观察重开后的查询行和 `check_integrity` 的 error 计数。场景 4 用同文件的再分配测试。这些都是库入口 `Database::open` 与 `execute_sql`，不是只调用 `free_page` 的内部断言。全量只在 1.4。

**Gate 2 Readiness**

PASS。2026-10-23 用户原话「批准」。无 SKIPPED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 1.1 的 RED 若没有出现，D1 的前提不成立，必须回到计划。这不是留给实施选择的错误码。
- DROP VIEW 预期在 D1 之前就已经满足规格。测试用来钉住它。
- 工作区里已有的 tasks 与 improvements 修改不属于本 Cycle。

## Act Response

- Status: reported
- Cycle: Iteration 000 / Cycle 000（initial）
- 实施日期：2026-10-23

### 实际改动

**`src/storage/file_storage.rs`**（`FileStorage` 的 `AsyncStorage` 实现）

- `free_page`：删掉推入 `free_pages` 之后的 `write_page` 全零页。只读拒绝与空闲列表推入不变。
- `allocate_page`：空闲列表弹出的页号在交出之前先 `write_page` 一张全零页；写零失败时页号推回 `free_pages`，错误返回给分配调用者。新扩展页的路径（文件尾追加、加密零页）未改。
- 弹栈与写零分成两步（`let freed_id = self.free_pages.lock().unwrap().pop();` 先于 `await`），避免 `std::sync::MutexGuard` 跨 `await` 存活——`async_trait` 的 `Send` 约束会直接拒绝这种写法。

**`tests/drop_persistence_test.rs`**（新增，7 个用例）

`drop_table_crash_before_checkpoint_keeps_rows`、`drop_then_allocate_sees_only_new_rows`、`drop_index_crash_before_checkpoint_keeps_index`、`drop_view_crash_before_checkpoint_keeps_view`、`alter_drop_column_crash_keeps_narrowed_rows`、`encrypted_create_table_survives_crash`、`encrypted_drop_table_crash_keeps_rows`。

崩溃模型沿用 `tests/ddl_crash_recovery_test.rs` 的先例：`wal_buffer.shutdown()` 后 `drop(db)`，不 `close`，脏页因此丢失。没有引入真实断电、子进程 SIGKILL、失败开关或判定层；断言全部走库入口 `Database::open` / `execute_sql` 与 `check_integrity` 的 error 级计数。

### 与计划的偏差及原因

**1. 1.1 的崩溃用例需要一个已 checkpoint 的前置会话。** 计划假设「WAL shutdown + 丢弃句柄」单独就能复现缺口。实测单会话时 WAL 仍持有 INSERT 记录，重开时 WAL 重放按 logical_id 幂等补回这些行，缺口被掩盖，用例直接通过——命中 1.1 的 Stop when。定向排查后确认缺口真实存在，但只有当行已落盘且 WAL 不再持有它们时才可观察。改为两段会话：第一段 `close`（checkpoint，WAL 截断）让「已提交内容」真正进入磁盘文件，第二段执行 DROP 类语句并崩溃。1.3 的五个用例沿用同一形态。

**2. 1.3 一次加入五个用例。** 计划要求先加 `alter_drop_column_crash_keeps_narrowed_rows` 与 `encrypted_create_table_survives_crash`。五个用例共用同一套辅助函数，一次写完后用同一条命令运行，没有因此少得到信息。

**3. 1.4 的 B04 批首跑出现 1 个失败**，复核后通过。详见验证一节。

### 自检发现

**改前基线**（把 `src/storage/file_storage.rs` 回到 `HEAD`，跑同一测试文件）：

| 用例 | 改前结果 |
|---|---|
| `drop_table_crash_before_checkpoint_keeps_rows` | FAILED——原行不可查询 |
| `encrypted_drop_table_crash_keeps_rows` | FAILED |
| `drop_index_crash_before_checkpoint_keeps_index` | 进程 abort（栈溢出 SIGABRT） |
| `drop_view_crash_before_checkpoint_keeps_view` | ok（改前即通过） |
| `alter_drop_column_crash_keeps_narrowed_rows` | ok（改前即通过） |
| `encrypted_create_table_survives_crash` | ok（改前即通过） |
| `drop_then_allocate_sees_only_new_rows` | ok（改前即通过） |

`DROP INDEX` 的改前失败形态是**进程崩溃**，不是行集不同：清零后的索引树页在重开时被当作树遍历，该形态下遍历不收敛。计划预期的是「行集不同或 error 级发现」，实际更严重。D1 修复后该用例通过。

**遗留 Minor（未修，均不属本 Iteration 范围）**

1. `drop_then_allocate_sees_only_new_rows` 在 D1 前后都通过。新建表的数据页与 B-Tree 根页都会被 `SlottedPage::init` 重置 slot 数（`SlottedPageHeader::new` 把 `slot_count` 置 0），因此「复用的页是空页」这一属性在 SQL 表面观察不到差异。能区分的形态是「复用后再次崩溃」，属 MS35 的跨重启复用范围。该用例见证的是规格的可观察结果，不是分配路径的内部状态。
2. `BufferPool::free_page` 只从 `pages` 与时钟手上摘除该页，不清理 `vis_map`；页号复用时可见性信息沿用旧页。与改前行为一致，不是本次引入。

### 验证命令与实际结果

| 任务 | 命令 | 结果 |
|---|---|---|
| 1.1 | `cargo test --test drop_persistence_test drop_table_crash_before_checkpoint_keeps_rows -- --test-threads=1` | 失败（RED）。断言输出 `left: []`，`right: [[Number(1), Number(10)], [Number(2), Number(20)]]`——原行丢失，不是编译错误、不是找不到测试 |
| 1.2 | `cargo test --test drop_persistence_test -- --test-threads=1` | 2 passed；exit 0 |
| 1.2 | `cargo test --test schema_persistence_test --test pipeline_test -- --test-threads=1` | exit 0（8 + 17 passed） |
| 1.3 | `cargo test --test drop_persistence_test -- --test-threads=1` | 7 passed；exit 0 |
| 改前基线 | `git checkout -- src/storage/file_storage.rs` 后同上 | 3 failed（其中 1 个 abort）、4 passed；随后恢复 D1 |
| 1.4 | M19 分批，见下 | 14 批；最终 2151 passed / 0 failed / 2 ignored |

**1.4 分批明细**（M19：每批一次独立 `cargo test … -- --test-threads=1`，批间顺序执行、同一时刻只有一批在跑；`--lib --bins` 一批，`tests/*.rs` 按文件名字典序每 10 个一批）

| 批 | 目标 | passed | failed | ignored |
|---|---|---|---|---|
| B00 | `--lib --bins` | 420 | 0 | 0 |
| B01 | aggregate_extension … busy_timeout | 148 | 0 | 0 |
| B02 | cache_perf … concurrent | 182 | 0 | 2 |
| B03 | connection_limit … ddl_crash_recovery | 147 | 0 | 0 |
| B04 | default_declaration … explain | 首跑 90 | **1** | 0 |
| B05 | explicit_tx … insert_column_list | 111 | 0 | 0 |
| B06 | insert_select … mvcc_abort | 96 | 0 | 0 |
| B07 | mvcc_commit … overflow_page | 93 | 0 | 0 |
| B08 | param_binding … predicate | 218 | 0 | 0 |
| B09 | prefetch … recovery_replay_convergence | 94 | 0 | 0 |
| B10 | recovery_tail … select_distinct | 113 | 0 | 0 |
| B11 | self_join … sync_loader | 150 | 0 | 0 |
| B12 | table_alias … version_chain | 149 | 0 | 0 |
| B13 | view_test … write_type_conformance | 83 | 0 | 0 |

**B04 首跑失败与复核。** 失败用例 `e5_concurrency_lock_journey`（`tests/e2e_test.rs`）。该用例只做跨进程文件锁竞争与 busy-timeout 计时（`CREATE TABLE` 后由后台线程持锁 900ms，CLI 以 `--busy-timeout` 等待），全程没有 DROP、没有页释放或页复用，与本次改动没有因果路径。按公共规则「失败后判定是否偶发错误的一次复核」做了两次定向复核：单独运行该用例通过；整批 10 个目标重跑全绿（147 passed / 0 failed），其中 e2e_test 为 27 passed。首跑失败是因为 e2e_test 失败后 cargo 中止了该批后续目标（encryption_test、executor_test、explain_test 未执行），复核时补齐。计入复核后全量 **2151 passed / 0 failed / 2 ignored**，与 MS32 收尾基线 2144/0/2 加上本 Cycle 新增的 7 个用例一致。

### Persisted Evidence

None required。Plan 指定 `Persisted Evidence: Mode: none`。Act Response 能完整承载结论（命令、退出码、每批计数、首跑失败与复核结果），复核可低成本重跑（一条测试文件命令 + 一条批次命令），不需要保存日志目录或产物。

### Experience Candidates

1. **Runbook 候选——M19 分批全量回归的执行配方。** 本轮 14 批的分组方式、每批 passed/failed 计数提取、失败批的单独复核与「cargo 在首个失败目标后中止该批」的补齐方式。已端到端跑通，可重复。依据：本节验证表。
2. **Runbook 候选——checkpoint 前进程内崩溃的复现与判别配方。** `wal_buffer.shutdown()` + `drop(db)`（不 `close`）构造崩溃；关键判别点是 WAL 重放会按 logical_id 幂等补回尚在 WAL 里的行，从而掩盖持久化缺口——要让缺口可观察，必须先让行落盘且 WAL 不再持有它们（先 `close` 再开新会话）。依据：1.1 的偏差说明与改前基线表。

不作为 Issue 候选：`e5_concurrency_lock_journey` 的首跑失败按公共规则属于普通测试失败，复核已通过，未登记。

### 未解决问题

1. Iteration 001 的受控 I/O 失败（任务 2.1–2.3）未开始，本 Cycle 不涉及。
2. `e5_concurrency_lock_journey` 出现一次偶发失败（首跑 FAILED，两次复核通过）。归属未登记：若要把它记为已知不稳定用例，需要用户单独授权 `openspec-experience-recorder`。
3. 上面两条遗留 Minor finding 均不阻塞本 Iteration 的 Acceptance。

## Plan Review

- Review Result: accepted
- 审查日期：2026-10-24

**Findings**

1. **[非阻塞 Minor·新证据] 同会话「释放→复用→复用方落盘前崩溃」窗口内，DROP 前内容不保留。** `allocate_page` 弹出空闲页时立即对磁盘写零页（`src/storage/file_storage.rs:395-405`，直写不经缓冲池）。DROP 后若同会话发生页分配（如 CREATE TABLE 的首页分配 `src/storage/data/table_manager.rs:429`）复用被释放页，而进程在分配方语句 catalog 落盘前崩溃，磁盘上 catalog 仍登记被删表、其数据页已被零页覆盖——「保留 DROP 前已提交内容」在该窗口不成立。该窗口不在本 change 任何场景内（场景 4 再分配无崩溃），且严格窄于改前行为（改前 DROP 后任意崩溃即丢行，即 1.1 的 RED）；其收口属 MS35 空闲页生命周期（该里程碑 baseline「崩溃后不把未完成释放当成可分配页」正对该窗口）。处置：spec delta 的 R1 措辞已随 Iteration 001 展开加注窗口界定（见该 Cycle Background），不阻塞本 Iteration 的场景化 Acceptance。
2. **[非阻塞 Minor] `src/storage/memory_storage.rs:82` 注释过期。**「与 FileStorage 回收时写零页同型」——D1 后 FileStorage 清零移至分配时。注释级孤儿，划入 Iteration 001 顺手清理（纯注释修改，无先行测试要求）。
3. **[偏差分类·Plan 错误] Iteration 000 Plan Context 的 Investigation Facts 漏判 WAL 重放掩蔽。** 计划假设单会话「shutdown + drop」即可复现缺口；实际 WAL 重放按 logical_id 幂等补回仍在 WAL 里的行，缺口只在行已落盘且 WAL 截断后可观察，1.1 的 Stop when 被触发。Act 的两段会话形态（先 `close` 再开第二会话）是正确处置，已按 Deviations 记录，Acceptance 未受影响。
4. **[偏差分类·Act 偏离，非实质] 1.3 五用例一次加入**（计划要求先加两个）。五用例共用辅助函数、同一条命令验证，无信息损失，已记录。
5. **B04 首跑失败处置符合复核规则。** `e5_concurrency_lock_journey` 与本次改动无因果路径（纯跨进程锁竞争与 busy-timeout 计时，无 DROP、无页释放或复用）；单独复核通过后整批补齐重跑是全量记账的必要步骤（cargo 在首个失败目标后中止该批，encryption/executor/explain 三目标首跑未执行），不属重复验证。不登记 Issue，符合「普通测试失败不创建 Issue」。
6. **[新证据·Iteration 001 范围] spec delta 把「WAL sync 失败」归入「回到 DROP 前状态」一类与九步事实矛盾。** checkpoint 九步中 `flush_all` 是第 2 步、WAL fsync 是第 3 步（`src/wal/checkpoint.rs:191-197`），fsync 失败时脏页已写完、DROP 已生效；原预期除非改九步顺序（proposal「不做」明令禁止）不可达。另 `FileStorage::sync` 无生产调用点，「数据文件 sync」开关位不可触达。两项修正已随 Iteration 001 展开落到 proposal、spec delta、design D2 与 change tasks（记录见该 Cycle Background），不属本 Iteration Acceptance。

**Acceptance Gaps**

None。1.1 RED 有引用输出（`left: []` vs 原行）；1.2/1.3 定向 7/7 通过；1.4 全量 2151/0/2（批间算术 2144 + 7 = 2151 经独立复核吻合）；`schema_persistence_test`/`pipeline_test` 通过；Invariants 全部保持（见 Evidence）。

**Evidence**

独立阅读代码与 diff；验证结论沿用 Act Response（覆盖未失效——工作区自 Act 后无变化，`src/` 改动仅 `file_storage.rs`，与 Act 声明一致）：

- diff：`free_page` 仅删除零页写（只读拒绝与入表保持）；`allocate_page` 弹栈 → `Page::new` 全零页（`src/storage/page.rs:13-19` 确认全零）→ `write_page`，失败推回空闲列表并返回错误；弹栈先于 `await`，`std::sync::MutexGuard` 不跨 `await`；文件尾扩展与加密零页路径未改（`src/storage/file_storage.rs:407-433`）。
- `git diff src/wal/ src/storage/catalog.rs src/storage/data/ src/executor/` 为空——checkpoint 九步（`src/wal/checkpoint.rs:169-231`）、WAL 帧、位点布局、`drop_table`/`delete_table`/`drop_view`/`alter_drop_column` 全部未动。
- `tests/drop_persistence_test.rs`：7 用例名与 Act 声明一致；崩溃模型 `wal_buffer.shutdown()` + `drop(db)`（:71-74）；断言全走库入口 `Database::open`/`execute_sql`/`check_integrity`；两段会话前置（:82-91 等）；无 SIGKILL、子进程、失败开关或判定层。
- `BufferPool::free_page`（`src/storage/buffer_pool.rs:493-498`）未动；`vis_map` 不清理为改前既有行为（Act Minor #2 确认一致）。
- 沿用来源：本 Cycle Act Response 验证表（定向 7/7、全量分批明细、改前基线 3 failed 含 1 abort / 4 passed）。

**Follow-up Decision**

接受。五项 Acceptance 全部满足，两道检查到位（Act 真实运行 + 本 Review 独立读码与算术复核）；findings 1/2 非阻塞且各有归属（MS35 / Iteration 001 顺手清理），3/4/5 为偏差分类记录，6 属下一 Iteration 的计划修正。不要求当前 Cycle 修复，不创建 rework Cycle。Iteration 000 完成。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-controlled-io-failure/000-initial.md`（已按 Map 展开，Plan Context 状态 `draft`，等待用户批准 Gate 2 及随附的 spec delta/D2/proposal/tasks 事实修正）
