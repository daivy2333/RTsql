# Iteration 000 / Cycle 000-initial: 只读打开模式（多读单写 + 零写入会话）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS26 三任务路线（本 Iteration 只做 T1/T2）；proposal 11 项默认假设与 design D1/D2/D3/D6/D7；用户四项决策（WAL 未决拒绝 / 锁冲突立即拒绝 / 只读 ATTACH 传递）。
- Excluded scope: `:memory:`（Iteration 001）；WAL 水位（Iteration 002）；dump/schema 生命周期子命令共享锁化；锁等待语义；内存恢复。

**Objective**

只读打开模式端到端可用：lib `Database::open_readonly[_with_key]` + CLI `--readonly` 经 advisory 共享锁实现多读单写（冲突立即 `DatabaseLocked`）；WAL 未决拒绝；只读会话零写入（不恢复、不 checkpoint、无任何文件写）；DML/DDL 计划期 25006 点名拒绝；ATTACH 附加库随会话只读；默认读写路径逐字节零回归。

**Investigation Facts**

- Current Baseline: HEAD 0fe6971 工作区（MS21 收尾批次待入库之上），全量 1541 tests / 0 failures / 2 ignored（2026-09-30 Iter005 Act Response 全量串行，Plan Review 采信）。本 Iteration 改动后的全量验证按 M19 分批纪律执行。
- Current-State Evidence（全部为 Plan 本会话实读，文件行号对应当前工作区）:
  - 锁面：`src/storage/file_storage.rs:60-77`——`open_with_key` 以 `read+write+create` 打开、`file.try_lock()` 独占、锁先于头校验/WAL/恢复、`WouldBlock → StorageError::DatabaseLocked`。头校验段（:87-162）含明文/加密两态分类、KDF 参数读取、`PageSizeMismatch` 校验——只读构造需复用该段（提取共享 helper）。
  - 存储抽象：`AsyncStorage`（`src/storage/async_storage.rs`）5 async 方法 + `page_count`；`Database::open_with_key` 以 `Arc<dyn AsyncStorage>` 装配（`src/database.rs:74`）。
  - Database 构造序：`src/database.rs:68-177`——storage → BufferPool(100) → TableManager::new + open_or_init（只读 catalog 消费）→ views 载入 → TransactionManager → WalWriter::open → WALBuffer + flush loop → set_wal_buffer → RecoveryManager::full_recover → advance_past → attach_index_catalog_contexts → PlanCache → CheckpointManager。`close()` = `checkpoint()`（:300-318）；MS18 零工作守卫在 `CheckpointManager::checkpoint` 顶部（`src/wal/checkpoint.rs:154`）。
  - WAL 可选性：`TransactionManager.wal_buffer: RwLock<Option<Arc<WALBuffer>>>`，begin/commit 均 `if let Some`（`src/transaction/manager.rs:89,112`）；全部写执行器与 `alter_table.rs:93` 以 `Option<Arc<WALBuffer>>` 接收。`Database.wal_buffer`/`wal_writer`/`checkpoint_manager` 非 Option；`database.wal_buffer` 消费方：pipeline :676,:1206,:1221,:1235,:1247（均 `Some(...)` 包裹）、`alter_table.rs:93`、`cli/mod.rs:776`（shutdown）。`database.wal_writer` 无外部消费方。
  - 可见性安全前提：`src/executor/data_scan.rs:335-361`——无快照（RR）时带 `commit_tx_id` 的版本可见、无 commit 不可见、墓碑按活跃集判定；可见性不依赖恢复后的分配器水位。只读者持共享锁期间写者无法获独占锁（flock 互斥），库冻结 → 只读会话不运行恢复可见性安全。
  - checkpoint 干净态：九步（`src/wal/checkpoint.rs:150-201`）后 `.wal` 恰含一条 `WalRecord::Checkpoint` 标记（步骤 6 追加、步骤 7 保留）——只读谓词的放行形态。
  - WAL Reader：`WalReader::open(wal_path)` + `read_all`（`src/wal/reader.rs:19,142`）；`.wal` 缺失 → open 报错，谓词先 `Path::exists` 分支。
  - CLI 面：`execute_main_command`（`src/cli/mod.rs:252`）→ `resolve_db_path` → `execute_command_inner`（:339，open 阶段 :352 `Database::open_with_key(RR, key)`）→ `run_sql` → close；`open_error_status`（:315）：DatabaseLocked → exit 4、InvalidKey → exit 5、其余 General 1。`AttachSession`（:388-434）：`resolve_db_path` + 存在性预检 + `Database::open_with_key(RR, key)`。
  - 写计划形状：`PhysicalPlan` 写变体 13 个——Insert/Upsert/Update/Delete/CreateTable/CreateTableAsSelect/CreateIndex/AlterTable/DropIndex/Truncate/CreateView/DropView/DropTable（`src/executor/plan.rs:25-61`）。plan cache 仅 SELECT。
  - SQLSTATE 通道：`Response::Error` 已有可选 `sqlstate` 载荷（MS19 R2）；CLI SQL 失败 → `sql_failure_status` exit 3。
- Code and Critical Path: T1 变更面 = `src/storage/file_storage.rs`（只读构造 + 头校验 helper 提取）、`src/database.rs`（`OpenMode` + 三字段 Option 化 + `open_internal` + `is_readonly` + close/checkpoint 守卫）、`src/pipeline.rs` 5 处 + `src/executor/alter_table.rs:93` + `src/cli/mod.rs:776` 等价改写、`src/storage/error.rs`（新变体 `WalPendingRecovery`）、`src/wal/reader.rs`（谓词消费，只读不改）。T2 变更面 = `src/pipeline.rs`（`readonly_rejection` + 6 入口挂接）、`src/cli/mod.rs`（`--readonly` flag + 主命令分支 + AttachSession 模式参数）。数据流：CLI flag → open_readonly_with_key → FileStorage 只读构造（共享锁 + 头校验 + WAL 谓词）→ Database(mode=ReadOnly, wal/checkpoint 全 None) → run_sql → pipeline 守卫（写计划 → 25006 Error）→ close（no-op）。错误路径：锁冲突 DatabaseLocked → exit 4；WalPendingRecovery → exit 1；25006 → exit 3。

**Implementation Guidance**

- 执行顺序：T1 先行（模式/字段/构造面），T2 依赖 T1 的 `is_readonly` 与 `open_readonly_with_key`。T1 内部先做字段 Option 化与等价改写（编译通过 + 全量 GREEN），再叠只读构造与谓词（新能力 RED→GREEN）。
- 头校验 helper 提取保持明文/加密两态行为逐字节不变；只读构造差异仅三点：OpenOptions（read only、无 create）、`try_lock_shared`、打开后 WAL 谓词。
- pipeline 两处 auto-commit 直呼（:463、:705）的 `database.commit()` 收敛**留给 Iteration 002**（T5）；本 Iteration 不触碰提交路径。
- 25006 消息文案：`cannot execute <STATEMENT KIND> in a read-only session`（语句类取计划变体名）。
- `WalPendingRecovery` Display 文案：`database has pending WAL, open read-write once to recover: <path>`。

**Task Contracts**

### T1: lib 只读打开面（共享锁 + WAL 未决谓词 + 零写入形态）

- Requirement/Scenario: readonly-open-mode R1（入口与共享锁：并发只读/写者拒只读/只读拒写者/缺失文件/加密只读）、R2（WAL 未决拒绝）、R3（零写入）、R6 + database-file-lock R1（默认路径零回归）
- Depends on: None
- Targets: `src/storage/file_storage.rs::FileStorage`（新 `open_readonly`/`open_readonly_with_key` + 头校验 helper 提取 + AsyncStorage 写方法守卫）、`src/database.rs::Database`（`OpenMode`、`mode`/`is_readonly`、三字段 Option 化、`open_readonly`/`open_readonly_with_key`、私有 `open_internal`、`close`/`checkpoint` 非 ReadWrite no-op）、`src/pipeline.rs` 5 处构造点、`src/executor/alter_table.rs:93`、`src/cli/mod.rs:776`、`src/storage/error.rs`（`WalPendingRecovery`）、`src/wal/mod.rs` 导出（如需）
- Current behavior: 打开一律独占锁 + 读写句柄 + 必跑恢复；WAL 无未决判定；只读会话 close 会走 MS18 守卫路径；`Database` 三字段非 Option。
- Required behavior: ① `FileStorage::open_readonly_with_key(path, key)`——只读句柄（无 create，缺失文件报错）、`try_lock_shared`（WouldBlock → `DatabaseLocked`）、头校验/密钥/KDF 行为与读写构造逐字节一致、写方法（allocate/write/free）显式报错点名 read-only；② `Database::open_readonly[_with_key]`——mode=ReadOnly、wal_buffer/wal_writer/checkpoint_manager 为 None、不运行恢复、catalog 与 views 只读载入、打开前 WAL 未决谓词（缺失/空/仅 Checkpoint 标记放行，否则 `WalPendingRecovery`）；③ 既有 ReadWrite 构造行为逐字节不变（字段 Option 化后 6 处消费点等价改写）；④ `close()`/`checkpoint()` 对 ReadOnly 返回 `Ok(())` 且不产生任何文件写入。
- Preserve: 既有 `Database::open`/`open_with_isolation`/`open_with_key` 公有签名与行为逐字节不变；`get_write_count`/`reset_write_count`/MS18 零工作守卫不变；头校验错误分类（NotADatabase/NewerFileVersion/IncompatibleHeader/PageSizeMismatch/InvalidKey）不变；`DatabaseLocked` 语义与 exit 4 映射不变。
- Forbidden: 不触碰 pipeline 提交边界（:463/:705 直呼保留）；不实现水位/字节计数；不修改恢复路径本体；不把 `WalReader` 改为 async 或改帧格式；不动 dump/schema 生命周期子命令的打开方式。
- Test witness: 新 `tests/readonly_session_test.rs`——T1 部分先 RED：双连接并发只读打开成功（flock LOCK_SH ×2）、只读者持锁时读写 open 被拒（`DatabaseLocked`）、写者持锁时只读 open 被拒、只读打开缺失文件不创建、WAL 未决（夹具：写后 kill 模拟——直接手写 WAL 记录或杀进程）拒绝且文件未动、恢复后（读写 open+close）只读可开、只读会话前后三文件 checksum 一致、加密库只读打开 + 错误密钥 `InvalidKey`。既有 `tests/database_file_lock_test.rs` 保持 GREEN（等价改写后立即回归）。
- GREEN condition: 上述新场景全绿 + `database_file_lock_test.rs` 全绿 + `cargo build` 无警告级破坏。
- Verification: `cargo test --test readonly_session_test --test database_file_lock_test`（退出码 0）；字段 Option 化后全量分批（M19 纪律）GREEN——批组按目标名字典序，各批 0 failed。
- Stop when: 字段 Option 化后出现非等价行为差异（如 pipeline 构造点行为变化）；`try_lock_shared` 在目标平台不可用；只读构造与读写构造的头校验行为出现非预期分歧。

### T2: 只读拒绝面与 CLI（--readonly + 25006 + ATTACH 传递）

- Requirement/Scenario: readonly-open-mode R4（DML/DDL 拒绝 + 只读语句不受限）、R5（ATTACH 只读）、R1（CLI `--readonly` 入口）、R2（CLI WAL 未决错误面）、error-reporting R2（25006 场景）
- Depends on: T1
- Targets: `src/pipeline.rs`（新 `readonly_rejection` 守卫 + `execute`/`execute_stage`/`execute_stage_routed`/`execute_in_tx`/`execute_stage_in_tx`/`execute_stage_in_tx_routed` 6 入口顶部挂接）、`src/cli/mod.rs`（`CliArgs` 增 `--readonly`、`execute_main_command` 分支、`AttachSession::open` 会话模式参数）、`tests/readonly_session_test.rs`（扩展）
- Current behavior: 只读会话（T1 后）执行写语句会真正执行（写路径在只读存储上触发写方法报错或脏页路径——语义未定义）；CLI 无 `--readonly`；ATTACH 一律 `open_with_key` 独占。
- Required behavior: ① 只读会话中 13 写计划变体（Insert/Upsert/Update/Delete/CreateTable/CreateTableAsSelect/CreateIndex/AlterTable/DropIndex/Truncate/CreateView/DropView/DropTable）在计划期（解析后、写入生效前）以 `Response::Error` 拒绝——消息 `cannot execute <KIND> in a read-only session`、`sqlstate: Some("25006")`；CLI 该语句 exit 3，库与 schema 无变化；② SELECT（含 JOIN/聚合/子查询/视图展开）、ATTACH、DETACH、事务语句不受限（显式事务内写语句被拒且事务保持活跃，沿既有语句级失败语义）；③ CLI `--readonly` → `open_readonly_with_key`；WAL 未决 → exit 1 且错误文案点名恢复指引；锁冲突 → exit 4（既有映射）；④ readonly 会话 ATTACH：附加库经 `open_readonly_with_key` 打开（共享锁 + WAL 谓词）；附加库锁冲突/未决 → ATTACH 失败零副作用（MS20 收口语义）；读写/内存会话 ATTACH 行为不变。
- Preserve: 写计划枚举完备性（穷尽 13 变体 + `_` 防御臂返回 None）；非只读会话全部路径行为逐字节不变（守卫 `is_readonly()` 为假时零成本直通）；MS20 ATTACH 既有测试场景（attach_session_test/cross_db_query_test/ctas_test）全绿。
- Forbidden: 不在 planner 层加只读守卫（design D3 已否决）；不改变 ATTACH 失败的既有错误分类与回滚收口；不为 25006 新增 CLI 退出码映射（复用 SQL 失败 exit 3 通道）；不拒绝只读会话内 `CREATE TABLE AS SELECT` 以外的任何读形状计划。
- Test witness: T2 部分先 RED——只读会话 INSERT/UPDATE/DELETE/UPSERT/REPLACE/CREATE TABLE/DROP TABLE/ALTER/TRUNCATE/CREATE+DROP INDEX/CREATE+DROP VIEW/CTAS 各自拒绝且断言 sqlstate 25006 与 exit 3；只读 SELECT/ATTACH/事务包 SELECT 正常；ATTACH 只读传递（附加库被写者持锁 → ATTACH 失败；写者退出后成功 + 跨库查询）；CLI `--readonly` e2e（正常查询 / WAL 未决 exit 1 / 锁冲突 exit 4）。
- GREEN condition: 新场景全绿 + `attach_session_test.rs`/`cross_db_query_test.rs`/`ctas_test.rs`/`tx_statement_test.rs` 零回归 + 全量分批 GREEN。
- Verification: `cargo test --test readonly_session_test --test attach_session_test --test cross_db_query_test --test ctas_test --test tx_statement_test`（退出码 0）；随后全量分批（M19）。
- Stop when: 写计划变体枚举出现遗漏形状（新增计划变体未覆盖）；守卫挂接点存在绕过路径（某入口未挂接导致写可达）；25006 经 PG 协议面分类缺失。

**Invariants**

- 默认读写打开（lib 三入口 + CLI 无 flag 全命令）加锁、恢复、checkpoint、行为逐字节不变（readonly-open-mode R6、database-file-lock R1 MODIFIED、proposal 假设 11）。
- 只读会话生命周期零文件写入（主文件、`.wal`、`.checkpoint`）——结构性保证（wal/checkpoint 通道不存在），不以 MS18 守卫巧合替代。
- flock 语义：只读 = `LOCK_SH`、读写 = `LOCK_EX`；冲突立即拒绝，无等待。
- 可见性：只读会话查询走既有无快照（RR）路径，库冻结态下「最新已提交」恒定（design 调查事实 6）。
- 错误面：`DatabaseLocked` → exit 4、`WalPendingRecovery` → exit 1、只读写拒绝 → 25006 + exit 3、`InvalidKey` → exit 5——既有映射不重排。

**Non-goals**

- `:memory:`（Iteration 001）；WAL 水位（Iteration 002）；dump/schema 共享锁化；锁等待/超时；只读会话内存恢复；RC 隔离级别的只读组合（只读打开固定 RR 语义——无快照「最新已提交」在冻结库上恒正确，如未来需要 RC 只读另议）。

**Acceptance**

- readonly-open-mode R1：双连接并发只读打开并查询一致；锁矩阵三形态（读读并行 / 写者拒只读者 / 只读者拒写者）行为正确且冲突为 `DatabaseLocked`；缺失文件不创建；加密库只读可查、错误密钥拒绝。（T1/T2 测试）
- readonly-open-mode R2：checkpoint-clean 放行；WAL 未决点名拒绝且文件未动；读写恢复一次后只读可开。（T1 测试）
- readonly-open-mode R3：只读会话（含显式 checkpoint/close 调用）前后主文件 + `.wal` + `.checkpoint` 字节一致。（T1 测试）
- readonly-open-mode R4：13 写变体拒绝矩阵（25006 + exit 3、库与 schema 无变化）；SELECT/ATTACH/DETACH/事务语句不受限。（T2 测试）
- readonly-open-mode R5：附加库随会话只读打开；附加库锁冲突/未决 ATTACH 失败零副作用。（T2 测试）
- readonly-open-mode R6 + database-file-lock R1：`database_file_lock_test.rs` 全矩阵 + 全量分批零回归。（T1/T2 验证）
- error-reporting R2：25006 分类经 `sqlstate` 载荷可达（T2 测试断言）。

**Verification**

- 场景 → 判定映射（全部自动命令，退出码 0 为通过；M19 分批纪律适用于全量）：
  - R1/R2/R3/R5/R6 → `cargo test --test readonly_session_test --test database_file_lock_test`；断言失败即对应场景失败。
  - R4 + 25006 → `cargo test --test readonly_session_test`（拒绝矩阵用例）。
  - 受影响边界 → `cargo test --test attach_session_test --test cross_db_query_test --test ctas_test --test tx_statement_test`。
  - 全量 → 按目标名字典序分批 `cargo test --test <targets> -- --test-threads=1`，各批 0 failed；结论注明分批方式与各批计数。

**Gate 2 Readiness**

PASS——2026-09-30 用户批准计划（需求/范围 Gate 1 与计划 Gate 2 均经 AskUserQuestion 明确批准；四项场景缺口决策与 11 项默认假设记录于 proposal）。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（验证结论以 Act Response 命令 + 决定性输出 + 退出码承载；checksum 见证直接在测试断言内完成，无需独立 Evidence。）

**Risks and Notes**

- `File::try_lock_shared` 与 `try_lock` 同批 stable（Rust 1.88）；本机工具链若低于该版本需先升级（Act 第一步编译即可暴露，非实质未知）。
- flock 在同一进程内按 open-file-description 语义：同进程双 `LOCK_SH` 成功、同进程 `LOCK_SH`+`LOCK_EX` 互斥——与既有 `database_file_lock_test` 的同进程双开拒绝测试同型，测试可在单进程内完成锁矩阵。
- WAL 未决夹具：优先「读写会话写入后不 close 直接 drop + 手工构造」或杀子进程；若恢复侧夹具不稳定，允许退化为手写 WAL 帧（复用 `record.rs` 序列化），谓词测试不依赖崩溃时序。
- 非 ReadWrite 模式的 `wal_writer` 为 None 后，`Database` 不再持有任何 WAL 句柄——若 Act 发现隐藏消费方（调查未覆盖的 `database.wal_writer` 引用），按「该消费点在 ReadOnly/Memory 不可达」或返回 Plan 二选一，不得静默给ReadOnly 模式挂 WAL 句柄。

## Act Response

- Status: reported

**Implemented**

T1（lib 只读打开面）——

- `src/storage/error.rs`：新 `StorageError::WalPendingRecovery(String)`，Display `database has pending WAL, open read-write once to recover: {path}`。
- `src/storage/file_storage.rs`：
  - 头校验逻辑提取为共享 `validate_header(file, path, key, file_len) -> (cipher, record_size, page_count)`，0 字节新库头初始化提取为 `initialize_header`；读写构造改调这两处（行为逐字节不变——MS10-T03 既有两态分类、KDF 参数读取、密钥一致性、`PageSizeMismatch` 判定全部原样搬移）。
  - 新 `FileStorage::open_readonly[_with_key]`：只读句柄（无 `write`/`create`，缺失文件 → `NotADatabase` 点名路径，0 字节文件同类拒绝）→ `try_lock_shared`（`WouldBlock` → `DatabaseLocked`，不等待）→ 复用 `validate_header`（加密库按密钥解密，错误密钥 `InvalidKey`）。
  - 新 `readonly` 字段：`write_page`/`allocate_page`/`free_page` 点名拒绝（`cannot <op> on a read-only storage instance`），`sync` no-op。
- `src/database.rs`：
  - 新 `pub(crate) enum OpenMode { ReadWrite, ReadOnly }` + `Database::mode` 私有字段 + `pub fn is_readonly()`。
  - `wal_writer`/`wal_buffer`/`checkpoint_manager` 三字段 Option 化（`ReadWrite` 全 `Some`，`ReadOnly` 全 `None`）。
  - 构造链收敛为私有 `open_internal(mode, path, isolation, key)`；`open_with_key` 成为 `ReadWrite` 委托壳（公有签名不变）。`ReadOnly` 跳过 `WalWriter`/`WALBuffer`/`RecoveryManager::full_recover`/`advance_past`/`attach_index_catalog_contexts`/`CheckpointManager` 构造，仍走 `TableManager::open_or_init` 与视图载入（只读 catalog）。
  - 新 `open_readonly[_with_key]`（固定 `RepeatableRead`）；WAL 未决谓词 `assert_no_pending_wal`（缺失/空/仅 `Checkpoint` 放行，否则 `WalPendingRecovery`）。
  - `close()`/`checkpoint()` 对非 `ReadWrite` 直接 `Ok(())`（无 checkpoint manager 即无写路径，结构性零写入，不依赖 MS18 零工作守卫的巧合）。
- 等价改写消费点：`src/pipeline.rs` 5 处、`src/executor/alter_table.rs:93`（`Some(...)` 包裹去除）、`src/cli/mod.rs:776`（测试模块 `shutdown`）。

T2（只读拒绝面与 CLI）——

- `src/pipeline.rs`：新 `pub(crate) fn readonly_rejection(db, plan) -> Option<Response>` + `READ_ONLY_SQLSTATE = "25006"`；13 个写计划变体映射到 SQL 语句类名，消息 `cannot execute <KIND> in a read-only session`；防御臂 `_ => None`。挂在 `execute_stage_routed` 与 `execute_stage_in_tx_routed` 两个叶子入口顶部（`execute_stage_in_tx_routed` 侧优先级高于既有 0A000「事务内不支持」拒绝）。
- `src/cli/mod.rs`：`CliArgs` 新增 `--readonly`；`execute_command` 对 `--readonly` + 生命周期子命令组合显式 `Usage`（exit 2，不静默忽略）；`execute_command_inner` 拆出 `execute_command_inner_mode(..., readonly, ...)` 承载只读打开（既有 8 处生命周期调用零改动）；`attach_database` 新增 `readonly` 参数，调用点传 `db.is_readonly()`（模式单一来源，会话内不另存状态）。

测试——

- 新 `tests/readonly_session_test.rs`（13 例）：R1 五例（双只读并发 + 一致查询、写者拒只读者、只读者拒写者、缺失文件不创建 + 目录仍空、加密只读 + 错误密钥）、R2 三例（checkpoint-clean 放行、未决拒绝 + 三文件字节未变、恢复后只读可开）、R3 一例（显式 `checkpoint()`/`close()` 前后主文件/`.wal`/`.checkpoint` 字节与目录项一致）、R4 两例（15 条语句覆盖 13 变体逐条 25006 + 库与 schema 无变化；只读语句不受限 + 事务内写被拒而事务仍活跃）、R5 一例（ATTACH 只读传递：锁冲突 exit 4 / 成功跨库查询 / 附加库 WAL 未决 exit 3）、CLI 一例（`--readonly` 查询 exit 0、锁冲突 exit 4、WAL 未决 exit 1 点名恢复指引、无 flag 读写路径不受影响）。

**Deviations from Plan**

1. **只读打开的 WAL 谓词改到取锁之后**（契约「判定在任何页写入与恢复重放之前完成」未变）：初版在 `FileStorage::open_readonly_with_key` 之前判定，测试暴露该顺序使只读打开在「写者持锁 + WAL 未决」时先报 `WalPendingRecovery`（exit 1）而非 `DatabaseLocked`（exit 4），与 MS10-T02 既有纪律（锁先于伴生文件与内容检查，`test_locked_bad_file_reports_lock_not_format` 见证）相反。改为共享锁取得后再判定，两条构造顺序同型。
2. **只读拒绝面挂在 2 个叶子入口而非 6 个入口逐个挂接**：`execute`/`execute_stage` → `execute_stage_routed`，`execute_in_tx`/`execute_stage_in_tx` → `execute_stage_in_tx_routed`（委托链已确认覆盖全部 6 个公开入口），等价控制流。
3. **T1 契约未列的调用方适配**（Risks and Notes 第 4 条按「消费点在 ReadOnly/Memory 不可达」处理）：字段 Option 化后，既有测试与 `src/cli/mod.rs` 测试模块中 264 处 `db.wal_buffer.shutdown()/do_flush()`（256）、`db.wal_writer.get_write_count()/get_current_lsn()`（7）、`db.checkpoint_manager`（1）直接访问需适配为 `.as_ref().unwrap()`（只读写会话恒 `Some`，语义逐字节不变）；`tests/executor_test.rs` 5 处 `Database { .. }` 结构体字面量因新增私有 `mode` 字段无法在 crate 外构造，改为经 `Database::open` 构造并从 `db.table_manager` 取 manager（顺带消除原夹具 `WalWriter::open(":memory:")` 在 CWD 落 `:memory:` 文件的副作用）。
4. **只读实例写方法错误分类**：契约允许「`StorageError::Io` 类或新变体随实现定」，采用既有 `StorageError::Internal` + 点名文案（不新增错误面变体，`error-reporting` 分类表不受影响）。
5. **25006 消息中的语句类名用 SQL 拼写**（`INSERT`/`CREATE TABLE`/…）而非 Rust 变体名——`<STATEMENT KIND>` 的用户可读形式；`PhysicalPlan::Upsert` 统一映射为 `UPSERT`（`ON CONFLICT` 与 `REPLACE INTO` 同一计划变体）。
6. **`--readonly` 与生命周期子命令组合显式拒绝**（exit 2）：契约未列，但 flag 落在顶层 `CliArgs` 时 `rtsql --readonly schema x` 会被 clap 接受并静默忽略——按 proposal 假设 10（子命令维持读写打开）与诚实化纪律补显式拒绝。
7. **`OpenMode` 本轮只含两变体**（`ReadWrite`/`ReadOnly`）：`Memory` 属 Iteration 001 T3，本轮加入会触发 never-constructed dead_code 警告。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

- **Critical/Important：0 项未解决。**
- 自检发现并已修复 2 项：① 上述 WAL 谓词与取锁的顺序缺陷（由 `cli_readonly_open_modes` 断言 exit 4 失败暴露，修正后重跑受影响验证）；② `--readonly` + 子命令臂的 `needless_return` clippy 警告（改为表达式臂）。
- 测试夹具修正 2 处（均在首轮 RED 后的验证阶段，非产品代码）：只读缺失文件的错误需点名路径（实现侧改为 `NotADatabase`）；WAL 未决夹具由手写帧改为经库自身 `WalWriter` 序列化器写 `BeginTxn`+`CommitTxn`（Plan Risks 第 3 条授权的退化路径，帧为可被恢复路径消费的真实形态，使 R2-S3 走真实恢复而非断言夹具不可重放）。
- 遗留 Minor：0。
- diff 范围核对：产品代码仅触及契约列出的 6 个文件（`storage/error.rs`、`storage/file_storage.rs`、`database.rs`、`pipeline.rs`、`executor/alter_table.rs`、`cli/mod.rs`）+ 新测试文件；`wal/reader.rs` 未改（只读消费）、`network/protocol.rs` 未改（25006 经既有 `sqlstate` 载荷通道，JSON 序列化不变）。无新增依赖、无身份型证据机制（checksum 见证在测试断言内完成）、无判定层脚本。

**Verification Evidence**

定向（M19 允许的单目标/少目标运行，退出码为判定）：

1. `cargo test --test readonly_session_test --test database_file_lock_test` → 9 + 5 passed / 0 failed，exit 0（T1 GREEN）。
2. `cargo test --test readonly_session_test` → 13 passed / 0 failed，exit 0（T2 GREEN）。
3. T2 受影响边界：`cargo test --test readonly_session_test --test database_file_lock_test --test attach_session_test --test cross_db_query_test --test ctas_test --test tx_statement_test -- --test-threads=1` → 11 / 17 / 17 / 5 / 13 / 21 passed，0 failed，exit 0。
4. `cargo build` → exit 0；`cargo clippy --lib --bins` → 0 warning；`cargo fmt --check` → clean。
5. CLI 实跑（`RTSQL_HOME` 指向独立 TempDir，裸名 `db`）：`--readonly` 查询 exit 0 输出 `{"columns":["n"],"rows":[[10]]}`；`--readonly` 写入 exit 3 输出 `statement 1 of 1 failed: cannot execute INSERT in a read-only session`；`--readonly schema db` exit 2；`completions bash` exit 0。

全量（M19 分批纪律：目标名字典序、每批 10 个、批内 `--test-threads=1`、批间顺序执行）：

- 批 0 `cargo test --lib --bins -- --test-threads=1` → 340 passed / 0 failed，exit 0。
- 批 1–10（`cargo test --test <10 targets> -- --test-threads=1`）→ 各批 exit 0；累计 **1214 passed / 0 failed / 2 ignored**。
- 合计 **1554 passed / 0 failed / 2 ignored**（基线 1541 + 本 change 新增 13）。
- `openspec validate 2026-09-30-ms26-embedded-runtime --strict` → `Change '2026-09-30-ms26-embedded-runtime' is valid`，exit 0。

**Persisted Evidence**

None required（Plan 指定 Mode: none；checksum 见证在测试断言内完成，无需独立证据文件）。

**Experience Candidates**

None.

**Remaining Issues**

- 非阻塞：CLI 新 flag `--readonly` 尚未进入 `README.md` / `README.zh-CN.md` / `rtsql-docs/SKILL.md` 的命令面文档（本 Iteration 契约未列文档面，收尾同步时处理）。
- 跨 Iteration 依赖提示：Iteration 001 T3 的 `OpenMode::Memory` 变体与 `open_in_memory` 待建；Iteration 002 T5 依赖本轮落定的 `Database` 字段 Option 化基线。


## Plan Review

- Review Result: accepted

**Findings**

独立检查（强制项：独立阅读完整 diff，核对 Act Response 声明与代码一致；不重跑已采信验证）：

- 产品代码 6 文件与新测试文件全部独立读毕；34 文件 diff 的测试适配抽样（upsert/explicit_tx/executor/checkpoint_skip 等）确认为机械 `.as_ref().unwrap()` 改写（全仓 262 处，与 Act 声明 264 处同量级——计数口径差异，非实质）+ `executor_test.rs` 5 处 `Database { .. }` 结构体字面量改 `Database::open` 构造（顺带消除 `WalWriter::open(":memory:")` CWD 文件副作用，与偏差 3 声明一致）。
- 委托链验证（偏差 2 的等价性）：`execute`→`execute_inner`→`execute_stage`→`execute_stage_routed`（pipeline.rs :95/:293/:299）；`execute_in_tx`→`execute_stage_in_tx`→`execute_stage_in_tx_routed`（:830/:861/:873/:879）——两个叶子挂点覆盖全部 6 个公开入口，等价控制流成立。
- 13 写变体枚举完备性（Act 防御臂注释指定由 Plan Review 核对）：对照 `PhysicalPlan` 全部变体（`src/executor/plan.rs`），写形状恰为 13 个，`readonly_rejection` 匹配臂一一对应无遗漏；其余（Scan/DataScan/IndexScan/IndexScanAll/Filter/Sort/Limit/Join/NestedLoopJoin/Aggregate/Having/SemiJoin/AntiJoin/SubqueryEval/DerivedScan/Projection/Distinct/SetOperation/Explain/SingleRow）均为读形状。
- WAL 路径推导一致性：`assert_no_pending_wal` 的 `path.with_extension("wal")` 与 `WalWriter::open`（writer.rs:27）、recovery.rs 4 处完全同型，无路径漂移；谓词位于 storage 构造臂内，先于 BufferPool/TableManager 与任何页操作。
- 偏差 1（WAL 谓词移到取锁之后）核实为正确修复：`open_internal` ReadOnly 臂先 `open_readonly_with_key`（持共享锁）后 `assert_no_pending_wal`，与 MS10-T02「锁先于伴生文件检查」纪律同型；`cli_readonly_open_modes` 断言「写者持锁 + WAL 未决 → exit 4」锁定该顺序。
- 25006 经既有 `sqlstate` 载荷通道直通：`network/protocol.rs` 未改，`Response::Error.sqlstate` 逐字进 JSON/PG 错误面——error-reporting R2（MODIFIED）可达，Act 声明成立。
- Forbidden 项核对：pipeline 提交边界（原 :463/:705 `transaction_manager.commit` 直呼）未触碰；恢复路径本体逐字节不变（仅包入 `if mode == ReadWrite`）；`wal/reader.rs`、`network/protocol.rs` 零改动；生命周期子命令经 `execute_command_inner` 委托壳（readonly=false）零改动；无 planner 层守卫。
- Preserve 项核对：`validate_header`/`initialize_header` 提取与被删内联代码逐行比对一致（明文/加密两态分类、KDF 参数、`PageSizeMismatch` 判定原样搬移）；`Database` 三入口公有签名不变；`DatabaseLocked`→exit 4 映射不变；MS18 零工作守卫未动。
- 测试见证核对：`tests/readonly_session_test.rs` 13 例与契约 witness 一一对应（R1 五例/R2 三例/R3 一例/R4 两例/R5 一例/CLI 一例）；R4 用例 15 条语句覆盖 13 计划变体并断言 25006 与消息点名，且含后置状态验证（行/视图/索引未动）；R2 未决夹具经库自身 `WalWriter` 写 `BeginTxn`+`CommitTxn`（Plan Risks 第 3 条授权的退化路径），帧为恢复路径可消费的真实形态。
- 偏差分类：偏差 1 = Act 自检修复（正确）；偏差 2/4/5/7 = 等价控制流与非实质选择（契约留给实现面）；偏差 3 = Plan 遗漏（契约未列字段 Option 化的既有测试调用方适配面——影响为机械改写，无行为变化，不阻塞）；偏差 6 = Act 偏离（契约外新增诚实化行为，方向正确，见 Minor 1）。

非阻塞 Minor finding：

1. 偏差 6（`--readonly` + 生命周期子命令 → exit 2）为契约外新增行为，仅有 Act Response 人工 CLI 实跑见证（Verification Evidence 第 5 项 `--readonly schema db` exit 2），无自动化测试用例。不阻塞（Acceptance 不含该行为，spec 无对应 SHALL）；作为回归耐久性改进候选记录，不强制当前 Cycle 修复——后续触碰 CLI 守卫面时顺带补自动化用例即可。
2. `--readonly` 文档面（README/README.zh-CN/rtsql-docs）未更新——Act 已登记 Remaining Issues，随 change 收尾文档同步处理，维持该处置。

观察项（非缺陷，备查）：只读打开缺失文件映射为 `NotADatabase`（点名路径）——spec 只要求「报错点名路径」，满足；WAL 撕裂帧时 `read_all` 错误映射为 `WalError`（exit 1）而非 `WalPendingRecovery`——fail-closed 且不改文件，超出 spec 场景（spec 场景均为完整帧）。

**Acceptance Gaps**

None——readonly-open-mode R1–R6 与 database-file-lock R1（MODIFIED）、error-reporting R2（MODIFIED）的全部场景有代码与测试见证对应，无未满足项。

**Evidence**

- 本 Review 新增检查：上述独立 diff 审读（产品 6 文件 + 新测试文件全文 + 34 文件 diff 抽样 + `PhysicalPlan` 枚举、pipeline 委托链、WAL 路径推导、protocol.rs sqlstate 通道的定点核对）。
- 采信 Act Response 验证结论（2026-09-30）：定向 `readonly_session_test` 13 passed、`database_file_lock_test` 5 passed；受影响边界 6 文件 84 例 0 failed；`cargo build`/`clippy`/`fmt --check` clean；全量 M19 分批 1554 passed / 0 failed / 2 ignored；`openspec validate --strict` 通过。覆盖面未失效（工作区 diff 与 Act 声明变更面一致），按公共规则 › 验证 不重跑。

**Follow-up Decision**

既有 Acceptance 全部满足、无阻塞 finding、两条 Minor 不构成返工理由（非阻塞 Minor finding 不创建修复）→ `accepted`。偏差 3 属 Plan 遗漏但仅为机械适配，已核实无行为影响；偏差 6 保留为改进候选。

**Iteration Plan Update**

None——Iteration Map 与平衡审计不变。

**Next Cycle**

None。

**Next Iteration**

`iterations/001-memory-db/000-initial.md`（`:memory:` 库形态，T3/T4）——已按既有 Map 展开，Plan Context 状态 `ready`（Gate 2 PASS 引用 2026-09-30 change 计划批准 + 本 Iteration 新表面补查）。
