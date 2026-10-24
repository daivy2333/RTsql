# Iteration 001 / Cycle 000: 受控 I/O 失败

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 2.1、2.2、2.3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 场景 8/8b/9；设计 D2（经本 Cycle 修正）；规格 `drop-persistence-boundary` 的「受控 I/O 失败有两种恢复结果」requirement（经修正）；Iteration 000 的 D1 释放语义、两段会话前置与崩溃助手先例
- Excluded scope: 半页写入、真实断电、MS35 空闲页跨重启复用、MS34 取消语义、I031、伴生文件加密、改 checkpoint 九步顺序 / WAL 帧 / 位点布局
- 顺手清理（注释级）：`src/storage/memory_storage.rs:82`「与 FileStorage 回收时写零页同型」在 D1 后过期，改为「与 FileStorage 分配时写零页同型」。纯注释修改，无先行测试要求（公共规则 › TDD）。

**Objective**

类型化 I/O 失败开关落地后：数据文件写入在刷脏页之前失败 → checkpoint 返回携带标记的错误，重开回到 DROP 前状态；WAL fsync 失败或位点路径被目录占据（都发生在脏页写完之后）→ checkpoint 返回错误，重开看到 DROP 已生效、其余已提交内容完好、`check_integrity` 无 error 级发现。未设置开关时字节行为不变。全量分批回归作为本 change 收尾验证通过。

**Background**

Iteration 000 的 Plan Review（2026-10-24）独立核对 checkpoint 九步时发现已批准文本两处与代码事实矛盾，已随本 Cycle 展开修正（`proposal.md` What Changes 与场景 8/8b、`specs/drop-persistence-boundary/spec.md`、`design.md` D2 与顺序节、change `tasks.md` 任务 2.1/2.2/RTM/Iteration 001 Stable result）：

1. WAL fsync 是九步之第 3 步，晚于第 2 步 `flush_all`——fsync 失败时脏页（含 catalog 删除）已写完，重开必然看到 DROP 已生效。原 spec 把「WAL sync 失败」归入「保留 DROP 前状态」一类不可达，除非改九步顺序（proposal「不做」明令禁止）。
2. `FileStorage::sync` 无生产调用点（全库 grep 仅测试与 overflow 测试助手调用），「数据文件 sync」开关位是不可触达的死面，从开关面移除。

修正保持 requirement 标题「两种恢复结果」的两类结构：改字节之前失败 → DROP 前状态；脏页写完之后失败（WAL fsync、位点）→ DROP 已生效。类属改判不改变场景数量与测试面。

**Investigation Facts**

- Starting point: 修订 `acfd274` + Iteration 000 未提交改动（`src/storage/file_storage.rs` D1、`tests/drop_persistence_test.rs` 7 用例）。Iteration 000 已 accepted：定向 7/7、全量 2151/0/2（来源：000 Cycle 000 Act Response 验证表，覆盖未失效）。工作区另有 MS33 规划期产生的 tasks/improvements/references/AGENTS.md 修改与两份 Runbook（R48/R49），本 Cycle 不改它们（memory_storage.rs 注释除外，见 Cycle Scope）。
- Current-State Evidence:
  - `CheckpointManager::checkpoint`（`src/wal/checkpoint.rs:169-231`）：零工作守卫（:177-182，`!pending_wal_at_open && write_count == 0 && !has_dirty_pages()` 时早退 Ok(0)；DROP 留脏 catalog 页，不触发早退）→ 步 1 LSN → 步 2 `buffer_pool.flush_all()`（:191-194，错误映射 `WalError::IoError(e.to_string())`）→ 步 3 `wal_writer.fsync()`（:197）→ 步 5 `write_checkpoint_site`（:206；`OpenOptions::create(true).truncate(true)` 打开位点路径，路径为目录时 `WalError::IoError`）。
  - `BufferPool::flush_all`（`src/storage/buffer_pool.rs:216-233`）：Phase 1 收集并**先清 dirty 位**，Phase 2 逐页 `write_page`。含义：flush 中途失败后，未写页在池内已标记干净，同一会话后续 checkpoint 不再补写——失败用例必须在 checkpoint 失败后立即崩溃，不得继续会话再 close。
  - 注入点：`FileStorage::write_page`（`src/storage/file_storage.rs:372-380`，spawn_blocking 之前检查并消费开关，此时无任何字节修改）；`WalWriter::fsync`（`src/wal/writer.rs:87-88`，sync_all 之前）。`FileStorage::sync`（file_storage.rs:456-465）无生产调用点。
  - WAL fsync 的其他消费面：`WalBuffer` 组提交在 DML commit 时 fsync（`src/wal/buffer.rs:145` 附近 write_batch + fsync）。开关 armed 后若先发生 DML commit 会被其消费——测试必须在 arm 后立即 `checkpoint()`。
  - 构造点：`Database` 打开流程 `FileStorage::open_with_lock`（`src/database.rs:75`）/ `WalWriter::open`（`src/database.rs:309`）。`Database` 为 `#[derive(Clone)]`（`src/database.rs:103`），开关以 `Arc` 字段持有即克隆共享（`schema_version`/`query_memory_limit` 先例）。既有 `FileStorage::open*` / `WalWriter::open` 公开签名被多处测试使用，保持不变；开关经新增内部构造变体或 `pub(crate)` 注入传入。
  - 错误通道：`StorageError::Io(io::Error)` Display `IO error: {0}`；步 2 错误经 :194 字符化进 `WalError::IoError`，`Database::checkpoint`（`src/database.rs:691-702`，返回 `Result<(), StorageError>`）再映射 `StorageError::WalError(String)`。注入错误的 Display 必须在整条链中携带可辨识固定标记，测试按标记断言。
  - 位点目录注入先例：`tests/insert_select_test.rs:723-726`（`std::fs::create_dir(&site)` 占据 `.checkpoint` 路径）。
  - 崩溃助手与两段会话先例：`tests/drop_persistence_test.rs:71-74`（`wal_buffer.shutdown()` + `drop(db)`）、:82-91（第一段 `close` 落盘 + 第二段执行被观察语句）。
- Code and Critical Path: `Database` 公共 arming 方法 → 共享开关单元 → `FileStorage::write_page` / `WalWriter::fsync` 在系统调用前检查-消费-失败 → `Database::checkpoint()` → 九步在失败点返回错误 → 测试崩溃重开经 `Database::open` 恢复并观察行集与 `check_integrity`。

**Implementation Guidance**

开关建议形态：`Arc<Mutex<Option<IoFailPoint>>>`（或等价原子），`IoFailPoint ∈ {DataWrite, WalFsync}`；armed 值在注入点 `take()` 消费（一次性、自清除）。注入错误建议：DataWrite 用 `StorageError::Io(io::Error::new(ErrorKind::Other, "injected io failure (write_page)"))`；WalFsync 用 `WalError::IoError("injected io failure (wal fsync)".into())`——标记字符串固定，经错误链 Display 可辨。两个注入点都必须在任何字节修改（含 `set_len`）之前。`:memory:`（MemoryStorage）与 ATTACH 附加库不接开关；readonly 打开无写路径，接不接均可；开关只传给主库的 `FileStorage` 与 `WalWriter`。

**Task Definitions**

### 2.1: 类型化失败开关

- Requirement/Scenario: 受控 I/O 失败有两种恢复结果（开关面本体，由本任务验收行约束）
- Depends on: None（Iteration 000 已 accepted）
- Targets: `src/database.rs`（开关字段 + 公共 arming 方法）；`src/storage/file_storage.rs::write_page`；`src/wal/writer.rs::fsync`；两处构造点传入开关；`tests/drop_persistence_test.rs`
- Current behavior: 无失败开关，写入与同步永不注入失败。
- Required behavior: `Database` 公共测试方法可按类型 arm 一次失败；armed 的下一次 `write_page` 或 `WalWriter::fsync` 在系统调用前返回携带固定标记的错误并自清除；未 armed 时两函数字节行为与 000 交付完全一致。
- Preserve: 既有 `FileStorage::open*` / `WalWriter::open` 公开签名与全部既有调用方；`MemoryStorage`；ATTACH 与 readonly 面零变化；九步顺序。
- Forbidden: 半页写入（先写部分字节再失败）；CLI flag；写入用户文档；数据文件 `sync` 开关位。
- First test: 新增 `io_fail_switch_errors_without_touching_file`（调用尚不存在的 arming API，编译失败即 RED）。序列：建表插行使池有脏页 → 记录 db 文件长度 → arm(DataWrite) → `db.checkpoint()` 断言 Err 且 Display 含标记 → 文件长度与失败前相同 → 再插一行（重新置脏 + WAL 追加，使下一轮 checkpoint 不走零工作早退）→ 不 arm 直接 `checkpoint()` 断言 Ok（开关已自清除、未设置时写入恢复）→ arm(WalFsync) → `checkpoint()` 断言 Err 含标记 → 不 arm 再 `checkpoint()` 断言 Ok。命令：`cargo test --test drop_persistence_test io_fail_switch_errors_without_touching_file -- --test-threads=1`。
- GREEN condition: 该测试通过；`cargo test --test schema_persistence_test --test pipeline_test -- --test-threads=1` 退出码 0。
- Verification: 上述两命令。
- Stop when: 注入点无法在字节修改前拦截；或未 armed 路径出现任何行为差异；或 arm 后到 checkpoint 之间存在无法绕开的 WAL fsync 消费面。

### 2.2: 三种恢复结果

- Requirement/Scenario: 受控 I/O 失败有两种恢复结果 / 写入在改文件之前失败；同步在不改变已写字节的情况下失败；位点文件无法创建
- Depends on: 2.1
- Targets: `tests/drop_persistence_test.rs`
- Current behavior: 三用例不存在。
- Required behavior: 三用例通过，全部沿用两段会话前置（第一段 `close` 落盘截断 WAL）+ 第二段 `DROP TABLE t` 后注入并 `db.checkpoint()` + 失败断言后立即 `crash()` 重开：
  - `checkpoint_write_fault_keeps_dropped_table`：arm(DataWrite) → checkpoint Err 含标记 → 重开 `t` 原行可查询、`other` 行在、check error 级发现为 0。
  - `checkpoint_wal_sync_fault_drop_is_durable`：arm(WalFsync) → checkpoint Err 含标记 → 重开 `t` 不存在、`other` 行在、check error 级发现为 0。
  - `checkpoint_site_occupied_drop_is_durable`：`std::fs::create_dir` 占据 `.checkpoint` 路径 → checkpoint Err → 重开 `t` 不存在、`other` 行在、check error 级发现为 0。
- Preserve: 000 的 7 用例断言零变化；崩溃助手形态；位点注入先例形态（insert_select_test.rs:723-726）。
- Forbidden: 为让用例通过改九步顺序、WAL 帧或位点布局；checkpoint 失败后继续会话再 close（dirty 位已被 flush_all 预清，会得到假象状态）。
- First test: 三用例一次加入（共用两段会话与崩溃助手，000 Cycle 1.3 先例）。命令：`cargo test --test drop_persistence_test -- --test-threads=1`。实施前：2.1 已 GREEN，三新用例因断言的行为未实现而 RED（位点用例在开关缺席时也可能已通过——若如此记「改前即通过」钉住即可，不改产品代码）。
- GREEN condition: 全文件退出码 0（预期 11 passed = 000 的 7 + 2.1 的 1 + 本任务 3）。
- Verification: 同一命令；三用例断言分别对应修正后 spec 的三个 THEN。
- Stop when: WAL sync 用例重开后 `t` 仍存在（九步事实与本计划矛盾，返回 Plan）；位点用例 check 出现 error 级发现；写入用例文件长度变化。

### 2.3: 全量分批回归（change 收尾验证）

- Requirement/Scenario: 本 change 全部场景 + 既有面零回归
- Depends on: 2.2
- Targets: 无新代码。按 R48 Runbook 配方执行 M19 分批。
- Current behavior: 2.2 后定向全绿；全量自 000 的 1.4 后未再运行。
- Required behavior: `--lib --bins` 一批 + `tests/*.rs` 按文件名字典序每 10 个一批，每批一次独立 `cargo test … -- --test-threads=1`、批间顺序执行，每批 0 failed；计数对账（预期 2151 + 本 Iteration 新增用例数）。
- Preserve: 不因无关失败改其他模块；失败批先停止并把失败测试名写入 Act Response。
- Forbidden: 无过滤全量 `cargo test`；默认并行度跑批次。
- First test: 无新测试。2.2 的命令已是 GREEN。
- GREEN condition: 每批退出码 0。
- Verification: 逐批记录 passed/failed/ignored；缺一批不得声明全量通过。
- Stop when: 某批失败且不属本 Iteration 开关/注入/释放路径 → 停止报告（`e5_concurrency_lock_journey` 已知偶发，按公共规则复核一次后计入）。

**Invariants**

- checkpoint 九步顺序、WAL 帧格式、24 字节位点布局不变。
- 未 armed 时 `write_page` / `WalWriter::fsync` 字节行为与 000 交付完全一致。
- 开关只影响主库 `FileStorage` / `WalWriter`；`:memory:`、ATTACH、readonly 面零变化。
- D1 释放语义（释放不写零、分配写零）不变。
- 失败注入不产生半页：任何注入点失败时目标文件字节与失败前相同（WAL fsync 失败时数据文件可以已合法增长——那是 flush_all 的正常写入，不是注入造成的部分写）。

**Non-goals**

半页写入、真实断电、磁盘满模拟、MS35 空闲页持久化、MS34 取消语义、加密伴生文件、`flush_all` dirty 位预清行为的改造。

**Acceptance**

- 2.1 开关测试通过，`schema_persistence_test` / `pipeline_test` 零回归。
- 2.2 三用例按修正后 spec 的 THEN 通过：写入失败 → DROP 前状态；WAL sync 与位点失败 → DROP 已生效 + check 无 error。
- 2.3 各批 0 failed，计数对账吻合。
- spec delta、design D2、proposal、change tasks 措辞与本 Cycle 交付一致（已随本 Cycle 展开修正）。

**Verification**

三恢复场景全部经库入口 `Database::open` / `execute_sql` / `checkpoint()` / `check_integrity` 观察（真实入口要求满足）；开关本体测试同样经 `Database` 公共方法与文件长度观察。全量按 M19 / R48 分批。本 Iteration 为 change 最后一个 Iteration，2.3 即 change 收尾验证——高开销验证按 change 预算：1.4 为计划批准期的中途执行（已运行并记录于 000 Act Response），2.3 为收尾验证。

**Gate 2 Readiness**

PASS。2026-10-24 用户原话「批准」（含随附的 spec delta / design D2 / proposal / tasks 事实修正）。无 SKIPPED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- `flush_all` 先清 dirty 位：checkpoint 失败后继续会话会得到「池内干净但磁盘缺页」状态；测试契约已定为失败后立即崩溃。该行为是既有边界，不在本 change 修复（如需规定属后续里程碑）。
- armed 与 checkpoint 之间若发生 DML commit，组提交 fsync 会消费 WalFsync 开关；测试在 arm 后立即 checkpoint，不构成风险。
- `e5_concurrency_lock_journey` 已知偶发（000 B04 首跑失败、复核通过），全量遇同类首跑失败按公共规则复核一次。
- memory_storage.rs:82 注释修正是本 Cycle 唯一 000 范围外触碰，纯注释级。

## Act Response

- Status: reported
- Cycle: Iteration 001 / Cycle 000（initial）
- 实施日期：2026-10-24

### 实际改动

**`src/storage/file_storage.rs`**

- 新增 `IoFailPoint` 枚举（`DataWrite` / `WalFsync`）、`pub(crate) type IoFailPointSlot = Arc<Mutex<Option<IoFailPoint>>>` 与消费函数 `consume_io_failpoint(slot, point)`——仅当 armed 值与注入点同型时 `take()` 并返回 true，不同型的 armed 值保持原样。
- `FileStorage` 新增私有字段 `io_failpoint: IoFailPointSlot`（两个构造点的结构体字面量各补一处；只读构造持空开关）。
- `open_with_lock` 委托新增 `pub(crate) open_with_lock_io_failpoint(path, key, lock_wait, io_failpoint)`，既有公开签名不变。
- `AsyncStorage::write_page` 注入点：readonly 拒绝之后、`spawn_blocking`（任何字节落盘，含 `set_len` 与 `write_all_at`）之前消费 `DataWrite`，命中返回 `StorageError::Io(io::Error::other("injected io failure (write_page)"))`。未 armed 时该检查返回 false，函数体其余路径逐字节与 000 交付一致。
- `free_page` / `allocate_page`（D1 语义）零改动。

**`src/wal/writer.rs`**

- `WalWriter` 新增私有字段 `io_failpoint`；`open` 委托新增 `pub(crate) open_with_io_failpoint(db_path, io_failpoint)`，公开签名不变。
- `fsync` 注入点：方法入口、`spawn_blocking` 的 `sync_all` 之前消费 `WalFsync`，命中返回 `WalError::IoError("injected io failure (wal fsync)")`。

**`src/storage/mod.rs`**

- `pub use file_storage::IoFailPoint`（测试经 `rtsql::storage::IoFailPoint` 消费）；`pub(crate) use` slot 别名与消费函数（`wal` 模块跨模块消费）。

**`src/database.rs`**

- `open_internal` 在存储构造前创建共享开关单元；ReadWrite 与 ReadOnly 两臂的 `construct_file_storage` 传入同一单元（新增参数，std::thread 闭包穿线；readonly 写路径本被前置守卫拒绝，接线无行为差异），`WalWriter::open_with_io_failpoint(path, io_failpoint.clone())` 分发。
- `Database` 新增私有字段 `io_failpoint`（`#[derive(Clone)]` 下 Arc 克隆共享，`schema_version` 先例）。
- 新增公共 arming 方法 `arm_io_failure(&self, point: IoFailPoint)`——置一次 armed 值。`:memory:`（MemoryStorage 无此通道）与 ATTACH 附加库（各自 Database 持独立单元）不波及。

**`src/storage/memory_storage.rs`**（顺手清理，纯注释）

- :85「与 FileStorage 回收时写零页同型」→「与 FileStorage 分配时写零页同型」（D1 后过期注释，Plan Review finding #2）。

**`tests/drop_persistence_test.rs`**（新增 4 用例）

- `io_fail_switch_errors_without_touching_file`（2.1）：开关本体——arm(DataWrite) → checkpoint Err 含标记且库文件长度不变 → 重插一行后不 arm checkpoint Ok（自清除）→ 再插一行 arm(WalFsync) → checkpoint Err 含标记 → 不 arm checkpoint Ok。
- `checkpoint_write_fault_keeps_dropped_table`、`checkpoint_wal_sync_fault_drop_is_durable`、`checkpoint_site_occupied_drop_is_durable`（2.2）：全部沿用两段会话前置（第一段 close 落盘截断 WAL）+ 第二段 `DROP TABLE t` 后注入并 `checkpoint()` + 失败断言后立即 `crash()` 重开；断言分别对应修正后 spec 的三个 THEN（写入失败 → t 原行可查询；fsync/位点失败 → t 不存在），三例均断言 `other` 行在、`check_integrity` error 级发现为 0。位点用例在重开前 `remove_dir` 清除注入目录（`read_site_file` 对目录路径 `File::open` 后 read 返回 EISDIR，不清除则重开失败）。

### 与计划的偏差及原因

1. **2.1 测试序列在 arm(WalFsync) 前补了一次 INSERT。** 计划的序列里，arm(WalFsync) 前的 checkpoint 已成功走完（步骤 9 归零 write_count、池无脏页），紧接的 checkpoint 会命中零工作早退返回 `Ok(0)`，触达不了 fsync——计划 First test 的「使下一轮 checkpoint 不走零工作早退」只覆盖了它前面的那一次。补插一行重置脏页与 WAL 追加，计划的全部断言保留。
2. **2.2 三用例首跑即通过（改前即通过），未观察到计划预期的 RED。** 2.2 无产品代码改动（Targets 仅测试文件）：两种恢复类由 checkpoint 九步顺序（flush 在 fsync/位点之前）与 2.1 注入点组合涌现，三用例在 2.1 完成后即满足。按计划对位点用例的同类预设记「改前即通过」钉住；计划的 RED 预测属 Plan 错误（非阻塞，Acceptance 是 GREEN 条件）。
3. **实现形态的等价局部选择**：注入错误用 `io::Error::other`（与计划的 `Error::new(ErrorKind::Other, ..)` Display 输出相同）；开关类型与消费函数落在 `file_storage.rs`（数据侧注入点），经 storage 模块再导出供 wal 与测试消费；构造变体而非 setter（计划两种形态中的前者）。另：实施证实 `WalWriter::fsync` 全库唯一生产调用点是 checkpoint 步骤 3，组提交走 `write_batch` 内部的 `sync_all`、不经过 `fsync()`——计划 Risks 中「arm 后 DML commit 组提交会消费 WalFsync 开关」的前提实际不成立，测试确定性优于计划假设（计划前提修正，方向有利）。

### 自检发现

- Gate 3（spec compliance）：修正后 spec 三场景与 RTM 三行各有用例钉住；Invariants 逐项核对——checkpoint.rs 零改动（九步顺序/WAL 帧/位点布局不变）、未 armed 时两注入点只是多一次空检查、D1 释放/分配语义零改动、`:memory:`/ATTACH/readonly 面零变化；既有 `FileStorage::open*` / `WalWriter::open` 公开签名与全部调用方保持。
- Gate 3（code quality）：diff 无计划外文件；`cargo fmt` 通过（格式化后复跑 11/11 GREEN）；`cargo clippy --lib --bins` 14 条警告全部位于既有文件（pipeline/lifecycle/executor_trait/protocol/ddl_dml/async_storage/manager），与 MS32 收尾基线持平，无新增。
- 已知边界沿用计划 Risks：`flush_all` 先清 dirty 位——测试契约定为 checkpoint 失败后立即崩溃，三用例遵守（失败断言后第一条语句就是 `crash()`）。
- 遗留 Minor：无新增。

### 验证命令与实际结果

| 任务 | 命令 | 结果 |
|---|---|---|
| 2.1 先行 | `cargo test --test drop_persistence_test io_fail_switch_errors_without_touching_file -- --test-threads=1`（实施前） | 失败（RED）：E0599 `arm_io_failure` method not found ×2 + E0432 `rtsql::storage::IoFailPoint` unresolved——编译失败，计划定义的 RED 形态 |
| 2.1 | 同命令（实施后） | 1 passed；exit 0 |
| 2.1 | `cargo test --test schema_persistence_test --test pipeline_test -- --test-threads=1` | 8 + 17 passed；exit 0 |
| 2.2 | `cargo test --test drop_persistence_test -- --test-threads=1` | 11 passed（000 的 7 + 2.1 的 1 + 本任务 3）；exit 0；fmt 后复跑 11 passed |
| 2.3 | M19 分批（R48 配方），见下表 | 14 批全部 0 failed |

**2.3 分批明细**（每批一次独立 `cargo test … -- --test-threads=1`，批间顺序执行、同一时刻只有一批在跑；`--lib --bins` 一批，`tests/*.rs` 按文件名字典序每 10 个文件一批）

| 批 | 目标 | passed | failed | ignored |
|---|---|---|---|---|
| B00 | `--lib --bins` | 420 | 0 | 0 |
| B01 | aggregate_extension … busy_timeout | 148 | 0 | 0 |
| B02 | cache_perf … concurrent | 182 | 0 | 2 |
| B03 | connection_limit … ddl_crash_recovery | 147 | 0 | 0 |
| B04 | default_declaration … explain | 151 | 0 | 0 |
| B05 | explicit_tx … insert_column_list | 111 | 0 | 0 |
| B06 | insert_select … mvcc_abort | 96 | 0 | 0 |
| B07 | mvcc_commit … overflow_page | 93 | 0 | 0 |
| B08 | param_binding … predicate | 218 | 0 | 0 |
| B09 | prefetch … recovery_replay_convergence | 94 | 0 | 0 |
| B10 | recovery_tail … select_distinct | 113 | 0 | 0 |
| B11 | self_join … sync_loader | 150 | 0 | 0 |
| B12 | table_alias … version_chain | 149 | 0 | 0 |
| B13 | view_test … write_type_conformance | 83 | 0 | 0 |

合计 **2155 passed / 0 failed / 2 ignored**（2 个 ignored 为 `cli_test.rs` 长期 `#[ignore]` 的校准/诊断用例）。对账：000 收尾全量 2151 + 本 Iteration 新增 4 = 2155；批级对账 B00–B03、B05–B13 与 000 明细逐批相等，B04 147 → 151 恰为本文件新增 4 用例。首跑无任何失败（000 B04 的 `e5_concurrency_lock_journey` 本次首跑即通过），无复核轮。

### Persisted Evidence

None required。Plan 指定 `Mode: none`；Act Response 完整承载命令、退出码与逐批计数，复核可低成本重跑。

### Experience Candidates

1. **Runbook 候选——受控 I/O 失败注入的测试配方。** 开关 arm 后必须立即 checkpoint（本实现下 WalFsync 只被 checkpoint 步骤 3 消费，组提交走 `write_batch` 内部 sync_all 不消费）；arm WalFsync 前须保证 write_count > 0 或池有脏页，否则 checkpoint 零工作早退触达不了 fsync；checkpoint 失败后必须立即崩溃不得再 close（`flush_all` 预清 dirty 位）；位点目录注入在重开前必须清除（`read_site_file` 对目录 EISDIR）。依据：本 Response「实际改动」与「验证命令与实际结果」节（`io_fail_switch_errors_without_touching_file` 及三恢复用例端到端跑通，可重复）。

不作为 Issue 候选：本次无 change 范围外的实质缺陷发现。

### 未解决问题

1. 本 Iteration 为 change 最后一个 Iteration，2.3 即 change 收尾验证；全部任务完成，等待 Plan Review。
2. 无遗留 Minor、无阻塞项。

## Plan Review

- Review Result: accepted
- 审查日期：2026-10-24

**Findings**

1. **[非实质·Act 偏离已记录] 2.1 测试序列补一次 INSERT。** 计划 First test 的「避开零工作早退」只修正了未 arm 的那次 checkpoint；arm(WalFsync) 紧随一次成功 checkpoint（步骤 9 归零 write_count、池已干净）会命中零工作早退，触达不了 fsync。Act 的补插修正了计划序列的真实空洞（偏差分类：Plan 错误，Act 处置正确），计划的全部断言保留。
2. **[偏差分类·Plan 错误] 2.2 的 RED 预测不成立。** 三用例在 2.1 落地后即满足（两种恢复类由九步顺序与注入点组合涌现，2.2 无产品代码改动）；位点用例的「改前即通过」形态计划已预设。Acceptance 以 GREEN 为条件，RED 缺席不构成缺口。
3. **[Plan 前提修正，方向有利] 组提交不消费 WalFsync 开关。** 计划 Risks 假设 DML commit 的组提交会消费开关；实际组提交走 `write_batch` 内部的 `sync_all`（`src/wal/writer.rs:239-254`），`WalWriter::fsync` 全库唯一生产调用点是 checkpoint 步骤 3——独立读码证实。测试确定性优于计划假设；Act 的 Runbook 候选已按此事实书写。
4. **[状态注记，非本 Cycle 缺陷] 工作区存在 change 范围外的未提交清理。** README×2 / `openspec/config.yaml` / decisions、knowledge spec / 两份 analysis / `test-bench-diagnosis.md` / references 微调——内容为 `.claude→.agents`、`CLAUDE.md→AGENTS.md` 类规则迁移扫尾，Act Response 未认领、不属本 change；Forbidden「不写入用户文档」成立（README 改动是无关的路径修正，开关未进用户文档）。
5. memory_storage.rs:82 注释修正与 000 Review finding #2 的处置一致（纯注释级）。

**Acceptance Gaps**

None。2.1 开关用例 GREEN + `schema_persistence_test`/`pipeline_test` 8+17 通过；2.2 全文件 11 passed（000 的 7 + 2.1 的 1 + 本任务 3），三用例断言逐条对照修正后 spec 的三个 THEN 成立；2.3 十四批全部 0 failed，合计 2155 = 2151 + 4，批级对账 B00–B03、B05–B13 与 000 明细逐批相等、B04 147→151 恰为本文件新增 4 用例。

**Evidence**

独立阅读代码与 diff；验证结论沿用 Act Response（覆盖未失效——`src/` 改动与 Act 声明的文件清单一致，无声明外产品代码改动）：

- 开关实现：`IoFailPoint` 类型化枚举与 `consume_io_failpoint` 仅同型消费（`src/storage/file_storage.rs:75-99`）；`write_page` 注入点位于 readonly 拒绝后、`spawn_blocking` 之前（:416-425），任何字节落盘前拦截；`WalWriter::fsync` 入口注入（`src/wal/writer.rs:100-108`），`sync_all` 之前；两处标记字符串与测试断言一致。未 armed 时仅多一次空检查。
- 接线：`open_internal` 单点创建共享单元，ReadWrite 臂经 `open_with_lock_io_failpoint` 传 `FileStorage`、`WalWriter::open_with_io_failpoint` 传 WAL（`src/database.rs:265-330`）；`arm_io_failure` 公共方法、无 CLI flag；readonly 构造持空开关；`:memory:` 与 ATTACH 不接线。
- 公开签名与不变量：`FileStorage::open_with_lock` / `WalWriter::open` 委托空开关保持原签名与全部既有调用方；`git diff src/cli/ src/wal/checkpoint.rs src/storage/buffer_pool.rs` 为空——九步、CLI、flush_all 全部未动；D1 的 `free_page`/`allocate_page` 与 000 交付逐字节一致。
- 测试：4 新用例断言全走库入口（`Database::open`/`execute_sql`/`checkpoint()`/`check_integrity` 与文件长度）；失败断言后第一条语句即 `crash()`（dirty 位预清契约）；位点用例重开前 `remove_dir` 恢复位点通道。
- 沿用来源：本 Cycle Act Response 验证表（定向 11/11、全量分批明细、clippy 14 条与 MS32 收尾基线持平）。

**Follow-up Decision**

接受。三项 Acceptance 全部满足且两道检查到位（Act 真实运行 + 本 Review 独立读码与逐批对账）；findings 1–3 为计划侧错误的 Act 正确处置与有利前提修正，4 为范围外状态注记。不要求修复，不创建后继 Cycle。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None——本 Iteration 为 change 最后一个逻辑 Iteration，全部 7 任务完成、两个 Iteration 均 `accepted`。change 待收尾：`openspec-docs-maintainer` 合并行为规格（`drop-persistence-boundary` 增量含本轮修正、`wal-recovery-replay-integrity` 修改）、同步 tasks 与 M/R/I、归档。
