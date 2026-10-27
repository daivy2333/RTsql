# Iteration 006 / Cycle 000: DDL 提交点统一模型（窄化意图随事务持久化 + 恢复期发布 + 提交前失败的目录面补偿 + 每表布局 DDL 令牌）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T6（ISS25）、T7（ISS28）、T8（ISS29）
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - change `design.md` **D2**（方向裁定一：DDL 提交点统一模型）的 M1–M5 与 **D4**（ISS29 的方向裁定）；**不得另选方向**。
  - change `specs/ddl-layout-publication/spec.md`（本 change 自己的 delta 域，**扩展，不新建**）——本 Cycle 已按 M5 与 T7 / T8 的范围裁定补充 Requirement 与 Scenario（见「Acceptance」节的 RTM 映射）。
  - Iteration 005（恢复面组）的既有行为：恢复面的有界读与「扫描类跳过 + 报出 / 重放类明确错误」的处置**零改动**。
  - Iteration 003（D7）已交付的写路径槽界限、Iteration 002 冻结的页级判据、Iteration 001 的读路径严格报错语义：**零改动**。
  - 既有合同：`ddl-catalog-transactional`、`ddl-rewrite-consistency`、`legacy-format-compat`（G1 / G2 / G3）、`mvcc-tombstone-visibility`、`drop-persistence-boundary`、`release-durability-precondition`、`schema-persistence`、`ddl-layout-publication`（delta）、`database-file-format-header`。
- Excluded scope:
  - T9 / T10（ISS30 / ISS31，写盘失败状态面）→ Iteration 007。
  - T11 / T12（ISS33 / ISS32，dump / restore）→ Iteration 008。
  - T13–T16（语料对齐与收尾全量回归）→ Iteration 009。
  - ISS25 / ISS28 / ISS29 台账条目的**编辑**（只读；「处置指针」归 Recorder）。
  - 全局 `.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references：**不动**。

**Objective**

DDL 的**提交点统一为一个模型**，并用真实入口验证：

1. `ALTER TABLE t DROP COLUMN` 的**窄化意图随事务持久化**（事务作用域的 WAL 记录，与该事务的提交记录同一次持久化落地），**崩溃重开时由恢复发布**该已提交的窄化 ⇒ 重开后的目录列数与数据面行宽一致，读**不再**按旧列序错位取值（ISS25 关闭）。
2. **提交点即事务提交**；目录窄化的发布是提交后的**幂等**步骤；重复发布得到相同结果。
3. **提交前失败的 DDL，其已发出的目录面变更被补偿** ⇒ 失败的 DDL 不留下目录面变更，且其后任一次成功刷盘持久化的是**回退后的内容**（ISS28 关闭）。
4. **同表并发的窄化 DDL 被显式拒绝**（错误点名该表），被拒绝者**不启动数据面改写**；在途状态由**带所有者令牌的每表状态**承载，只有持有者能清除 ⇒ 重叠清除不再丢失读拒绝（ISS29 关闭）。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`；**未提交**的工作区改动 = Iteration 000–005 的产品代码与测试改动 + `.agents/docs/tasks.md`（MS37–MS40 路线的既有现场）+ ISS35–ISS38 台账 + 本 change 产物目录。**本 Cycle 从零实施**，无既有实现可复用、无他人改动需回退。
  - 前序 Iteration 结论（**沿用**，覆盖范围未变化）：Iteration 002（页级判据冻结）、Iteration 003（写路径槽界限）、Iteration 004（目录页组）、Iteration 005（恢复面组，**本 Plan 亲自 Review 判定 `accepted`**）⇒ 既有结论**不失效**，可直接沿用。
  - **已知限制**（本 Cycle 的起点状态，逐条见下）：
    - ISS25：`ALTER TABLE t DROP COLUMN` 的**窄化意图不在任何持久载体中**（只存在于内存的 `NarrowingState`）⇒ 崩溃重开时该意图**永久丢失**；目录仍是旧列数（3），数据面已是窄行（2 列宽）⇒ 读按**位置**取到错位的值（`[id, v, NULL]` 形态）。
    - ISS28：`ALTER TABLE t ADD COLUMN m INT` 的目录面变更（`__columns` 追加列行 + `__tables` 的 `column_count` 上调）在语句边界的刷盘失败后**仍留在缓冲池的脏页里** ⇒ 其后任一次成功刷盘把它落盘 ⇒ 重开库可见该列（失败的语句留下了目录面变更）。
    - ISS29：`NarrowingState` 的在途状态是**「单表一个布尔量」**（`AtomicBool`）⇒ 同表并发窄化 DDL 存在两个交互缺陷：重叠清除（后完成者清掉仍在窗口内的读拒绝）与豁免读按旧 schema 解码（第二条 DDL 的改写自身读）。
- Current-State Evidence（**本 Plan 逐行核对当前工作区**，行号 = 当前工作区；符号名是定位依据，行号会随后续改动漂移）：
  - **WAL 记录与持久化**
    - `src/wal/record.rs`：`WalRecordType` 的取值 `0x01`–`0x09`（`Insert` / `Update` / `Delete` / `Commit` / `Abort` / `Checkpoint` / `BeginTxn` / `CommitTxn` / `AbortTxn`）⇒ **下一个可用类型字节 = `0x0A`**。帧格式 = `[lsn:8B][type:1B][len:4B][body][crc32:4B]`；`WalRecord` 是**枚举**（非结构体），每个变体自带序列化 / 反序列化 ⇒ 新增一个变体即新增一种记录。
    - `src/wal/buffer.rs:62`：`pub async fn append(&self, record: WalRecord) -> u64`（入缓冲，返回 LSN；缓冲满即刷）；`:82`：`pub async fn append_commit_and_wait(&self, tx_id: u64) -> Result<(), WalError>`（Group Commit：等该 tx 的持久化确认）。
    - `src/transaction/manager.rs:87` `begin()` → `wal.append(WalRecord::BeginTxn { tx_id })`；`:110` `commit()` → `wal.append(WalRecord::CommitTxn { tx_id, timestamp })` **紧接** `wal.append_commit_and_wait(tx_id)` ⇒ **事务的意图记录与提交记录在同一次 fsync 落地**（这正是 M1 要求的原子绑定，载体已就位，缺的是**窄化意图这条记录**）。
    - `src/wal/checkpoint.rs`：`CheckpointManager::checkpoint(tx_watermark)` = 刷盘 → 写检查点站点 → **截断 WAL** ⇒ WAL 是窄化意图的**唯一**持久载体时，截断会丢意图（M5 的落点）。
  - **目录面写入**
    - `src/storage/catalog.rs:365` `insert_column_row()`：`__columns` 链尾追加一行（**只改缓冲池页，不刷盘**）。
    - `src/storage/catalog.rs:425` `update_table_column_count_ddl(name, new_count)`：`__tables` 行 `column_count` 上调 + **刷盘**（`flush_commit_point`，`catalog.rs:270`）⇒ 提交点顺序 = 列行先、列数后 ⇒ 崩溃不会留下「列数已持久但列行未持久」。
    - `src/storage/catalog.rs:1697` `rewrite_column_rows_drop(table, removed_index)`：Pass 1 删除 `removed_index` 处的列行；Pass 2 把 `column_index > removed_index` 的行序号下调 ⇒ **非幂等**（第二次执行会删掉已被上移的行）⇒ 幂等性**必须**由调用方在发布入口判定。
    - `src/storage/catalog.rs` 的列数上限规则：`__tables` 行的 `column_count` 是**唯一**的列数权威（`open_or_init` 按它截断列定义，孤儿列行不可见）⇒ 目录面的「列数」与「列定义」是**两个**独立事实，必须**同时**回退。
  - **DDL 语句路径**
    - `src/storage/data/table_manager.rs:760` `alter_add_column()`：**catalog-first**（无事务、无 WAL）：`insert_column_row` → `update_table_column_count_ddl` → 内存活布局切换 ⇒ 失败时目录面已变而语句失败 ⇒ ISS28。
    - `src/storage/data/table_manager.rs:838` `alter_drop_column()`：读旧布局 → 构造 `new_meta` → **`:984` `old.narrowing.publish()`（改写开始处发布在途窄化）** → `:987` `NarrowingGuard::new(...)` → 改写（每行 DELETE + INSERT，走 WAL：`:1137` `WalRecord::Delete` / `:1187` `WalRecord::Insert`）⇒ 返回 `PendingColumnDrop`。
    - `src/storage/data/table_manager.rs:1228` `apply_column_drop(&pending)`：`rewrite_column_rows_drop` → `update_table_column_count_ddl` → 活布局切换（`:1237`）→ `:1245` `pending.new_meta.narrowing.clear()`。
    - `src/executor/alter_table.rs:80-155`（`DropColumn` 臂）：`reclaim_abandoned_implicit_transactions` → `transaction_manager.begin()` → `implicit_tx_guard(tx_id)` → `alter_drop_column(...)` → **Ok 分支**：`:116` `commit(tx)` → `:119` `guard.disarm()` → `:125` `ddl_commit_window::pause_at_ddl_commit_window(path)` → `:131` `apply_column_drop(&pending)` ⇒ **提交点 = 事务提交**（M2 的现状已成立），**「提交完成 → 目录应用」窗口**由 `src/executor/ddl_commit_window.rs` 的注入点精确暴露（ISS25 的可复现入口）。
    - `src/storage/data/table_manager.rs:105` `NarrowingState { table: String, in_flight: AtomicBool }`；`:127` `publish()` / `:132` `clear()` / `:139` `ensure_readable()`（读拒绝文案点名该表）；`:158` `NarrowingGuard`（`Drop` ⇒ `clear()`，兜底异常 / 放弃路径）。
  - **恢复路径**
    - `src/database.rs` 的打开顺序：`TableManager::new` + **`open_or_init`（目录先打开，活布局 = 目录当时的旧布局）** → **其后**才 `full_recover(path, buffer_pool, table_manager)` ⇒ 恢复期发布目录窄化时，**活布局必须同步切换**，否则恢复期的读仍按旧布局解码。
    - `src/wal/recovery.rs` `full_recover` 的阶段序：**W1 预扫描（`build_pk_version_maps`）→ 重放（`redo_record`）→ W4（`mark_uncommitted_aborted`）→ W3（`rebuild_pk_indexes`）**；`will_redo` 门控未变（Iteration 005 已核对）。
    - `src/wal/recovery.rs` 的 `extract_pk_key(table_meta, tuple_data)` 按 **`table_meta.columns`**（活布局的列定义）解码元组 ⇒ 重放与索引重建**都依赖活布局的列数** ⇒ **恢复期发布的时点必须早于重放**（否则窄行按旧列序解码 ⇒ 与 ISS25 的错读同形）。
    - `redo_record` 的 **Insert 臂在 deindexed 模式下**把 WAL 记录的 `row_id` **再次** `insert_row_id_sorted` 进**同一份** `ctx.pk_versions` 映射（Iteration 005 的 F5 已独立核实）⇒ 恢复期判定「意图所属事务是否已提交」时，**必须按事务 id 判定**，不得依赖候选集的来源。
  - **既有合同的逐条核对结论**（读码核对，见「Acceptance」节的兼容性核对表）
    - `legacy-format-compat` 的 G1 / G2 / G3 = **目录行布局代际**（`__tables` / `__columns` 行的字段布局），与 **WAL 记录类型**是**两个独立的格式面** ⇒ 新增一种 WAL 记录类型**不落在** G1 / G2 / G3 的支持范围内 ⇒ **保持**。G1 的恢复要求（旧库带未决写入记录打开后恢复已提交行）**不受影响**：旧 WAL 不含新记录类型 ⇒ 旧库的恢复行为逐点不变 ⇒ **保持**。
    - `database-file-format-header`：**文件头与页格式不变**（不新增页、不改页头、不改目录行布局）⇒ **保持**。
    - `schema-persistence`（「DDL 操作不写 WAL 记录」）：**必须修订**——本 Cycle 为窄化 DDL **新增一种事务作用域的 WAL 记录**。修订点只限「`ALTER TABLE … DROP COLUMN` 的窄化意图随事务持久化」这一条；`CREATE TABLE` / `DROP TABLE` / `ADD COLUMN` / `CREATE INDEX` **仍不写 WAL 记录** ⇒ 修订面**最小且明确**（见「Compatibility Cross-Check」）。
    - `ddl-catalog-transactional`（「DDL 目录变更随事务提交或中止」）：**修订**——窄化 DDL 的**数据面**仍随事务提交或中止（不变），但**目录面的发布**改为提交**之后**的幂等步骤（M2）⇒ 该域的原条款（「目录变更与数据面在同一事务内原子可见」）对窄化 DDL **不再逐字成立**，须显式修订；`ADD COLUMN` 的既有条款（catalog-first，失败不留变更）由 T7 的**补偿**兑现 ⇒ 语义方向不变、实现形态变化。
    - `ddl-rewrite-consistency`（窄行补 NULL / 改写一致性）：**保持**——改写的行级语义、墓碑可见性、窄行读取语义**零改动**。
    - `mvcc-tombstone-visibility`：**保持**——墓碑可见性判定（`VersionHeader::is_deleted` 等）**零改动**；本 Cycle 只改**目录面的发布时点**与**恢复期的发布**，不改版本链与可见性。
    - `drop-persistence-boundary` / `release-durability-precondition`：**保持**——本 Cycle **不改**释放路径与「刷盘失败不丢弃脏页写」的语义；T7 的补偿**依赖**该语义（恢复后的内容保持脏 ⇒ 其后成功刷盘持久化回退后的内容）⇒ 依赖方向是「使用既有语义」，不是「修改」。
    - `ddl-layout-publication`（本 change 的 delta 域）：**扩展**（不新建）——本 Cycle 已补 M5 的 Requirement 与 Scenario，并把 T7 / T8 的范围裁定写细。
- Code and Critical Path:
  - **写路径（T6 / T7 / T8）**：`src/executor/alter_table.rs` → `src/storage/data/table_manager.rs::alter_drop_column` / `alter_add_column` → `src/wal/buffer.rs::append` → `src/transaction/manager.rs::commit` → （提交后）`src/storage/data/table_manager.rs::apply_column_drop` → `src/storage/catalog.rs::rewrite_column_rows_drop` / `update_table_column_count_ddl` → `src/storage/catalog.rs::flush_commit_point`。
  - **恢复路径（T6）**：`src/database.rs::open` → `full_recover` → WAL 读出记录 → 事务分组（提交 / 中止判定）→ **窄化意图收集（新增）** → **已提交意图的发布（新增，排在重放之前）** → 重放（`redo_record`）→ `mark_uncommitted_aborted` → `rebuild_pk_indexes`。
  - **状态所有权**：窄化意图的**持久**所有权 = WAL（新增记录类型）；窄化意图的**在途**所有权 = 每表 `NarrowingState`（带所有者令牌）；目录窄化**已发布**的所有权 = `__tables` 行的 `column_count` + `__columns` 的列行。
  - **外部影响面**：`ALTER TABLE` 的成功 / 失败语义（提交点）、崩溃重开的可观察状态、同表并发 DDL 的错误面、WAL 文件的记录类型集合、checkpoint 的截断时机。**不触及**：dump / restore 合同面、`rtsql check` 的诊断分级、页格式、目录行布局。

**Implementation Guidance**

- **建议顺序**（T6 → T7 → T8 有硬依赖：T8 的令牌是 T6 的发布入口与 T7 的回退路径都要用的所有权机制 ⇒ T8 的**状态单元改造**应与 T6 的发布入口一起落地）：
  1. **T8 的状态单元改造先行**（`NarrowingState` 的令牌化 + `try_acquire` / `release` / 所有者门控的清除）——它改变 `NarrowingState` 的字段与语义，是 T6 / T7 的所有权基础。
  2. **T6 的 WAL 记录**（`src/wal/record.rs` 新增变体 + 序列化 / 反序列化 + `tx_id()`）⇒ 纯新增，**无既有行为变化**（可先行落地，**不构成** TDD 偏差：新记录类型在写入方使用前**不可被产生**）。
  3. **T6 的写路径**（`alter_drop_column` 写意图记录）+ **T6 的发布入口幂等化**（`publish_column_narrowing`）+ **T6 的恢复期发布**。
  4. **T7 的补偿**（`alter_add_column` 的失败回退）。
  5. **T6 / T7 / T8 的测试**（先行测试在**每个行为变更之前**落地并观察 RED）。
- **关键取舍**：
  - **发布入口的幂等判据用 `__tables` 行的 `column_count`**（而非列行内容）：`column_count == new_column_count` ⇒ 已发布 ⇒ 直接返回（**必须**在改写列行**之前**判定，否则第二次发布会删错行——`rewrite_column_rows_drop` 非幂等）。
  - **恢复期发布的时点**：必须在**重放之前**（活布局的列定义决定重放与索引重建的解码宽度）；发布后**必须**同步切换内存活布局（否则 `Database::open` 返回后仍是旧布局）。
  - **意图记录的写入时点**：在**数据面改写成功之后、事务提交之前**写入 ⇒ 改写失败 / 语句中止 ⇒ 不产生意图 ⇒ 恢复期不会发布未提交的窄化。
  - **发布失败不把已提交的语句改判为失败**（M2）：发布失败时**保留**该表的在途窄化状态（读继续被拒绝），使读**不会**退化成按旧布局错读；意图记录留在 WAL 中 ⇒ 下次恢复期发布。
  - **补偿不做存储 IO**：只改缓冲池页（脏）⇒ 在存储正在失败时也能完成回退；**不刷盘** ⇒ 其后任一次成功刷盘持久化的是回退后的内容。
  - **令牌的取值**：每表单调递增计数器（`AtomicU64`），**仅作互斥所有权标记**，**不承载任何身份语义**（不是 run-id / 指纹 / revision / 时间证明）。
  - **T8 的适用范围裁定**：令牌只覆盖**窄化 DDL**（`ALTER TABLE … DROP COLUMN`）。`ADD COLUMN` 不纳入令牌：它全程持有 `self.tables.write()` ⇒ 同表的两条 `ADD COLUMN` **已被串行化**（不会同时在途）；把 `ADD COLUMN` 纳入令牌会把今天**能成功**的并发 `ADD COLUMN` 改成被拒绝 ⇒ 属行为回退。**`ADD COLUMN` 与窄化 DDL 并发的竞态**（`ADD COLUMN` 读到旧列数而窄化 DDL 正在改写）**不在本 Cycle 范围** ⇒ 作为 Issue 候选**只报告不登记**（见「Risks and Notes」）。

**Task Definitions**

### T6: 窄化意图随事务持久化，崩溃重开后由恢复发布已提交的窄化（ISS25）

- Requirement/Scenario: DLP「布局窄化的意图与数据面改写原子绑定」（+ 新增「未发布的窄化意图不因检查点而丢失」）；`ddl-catalog-transactional`（修订）；`schema-persistence`（修订）
- Depends on: T8（令牌化的在途状态是发布入口的所有权基础）
- Targets:
  - `src/wal/record.rs::WalRecordType`（新增类型字节 `0x0A`）、`src/wal/record.rs::WalRecord`（新增变体 + 序列化 / 反序列化 + `tx_id()`）
  - `src/storage/data/table_manager.rs::alter_drop_column`（写意图记录）
  - `src/storage/data/table_manager.rs::apply_column_drop`、`src/storage/data/table_manager.rs::publish_column_narrowing`（新增：幂等发布入口）
  - `src/storage/data/table_manager.rs::narrowed_meta`（新增：从旧 `TableMeta` 推出窄化后的 `TableMeta` 的**单一**实现，供正常路径与恢复期发布共用）
  - `src/wal/recovery.rs::full_recover`（收集意图 + 已提交意图的发布，排在重放之前）
  - `src/wal/checkpoint.rs::CheckpointManager::checkpoint`（M5：不丢失未发布的意图）
  - `src/storage/data/table_manager.rs`（未发布意图的登记面：供 checkpoint 判定）
- Current behavior:
  - 窄化意图**只存在于内存**（`NarrowingState` 的布尔量）⇒ 崩溃重开后意图**永久丢失**；目录仍是旧列数、数据面已是窄行 ⇒ 读按位置取到错位的值。
  - `apply_column_drop` 的发布**非幂等**（`rewrite_column_rows_drop` 第二次执行会删错行）。
  - 恢复期**没有**任何目录窄化的发布路径 ⇒ 恢复期的重放与索引重建按**旧**活布局解码窄行。
  - checkpoint 会**无条件截断** WAL ⇒ 若在「意图已提交但未发布」的窗口内发生截断，意图**永久丢失**。
- Required behavior:
  1. **窄化意图记录**：`ALTER TABLE t DROP COLUMN` 的意图（表名 + 被删列序号 + 新列数）作为**事务作用域的 WAL 记录**写入该语句的事务记录序列，与该事务的 `CommitTxn` **同一次持久化**落地（载体：`src/wal/buffer.rs::append` + `append_commit_and_wait` 的既有 Group Commit）⇒ 不出现「意图已持久但数据面未提交」或「数据面已提交但意图不存在」。
  2. **写意图的时点**：在**数据面改写成功之后、事务提交之前**；改写失败 / 语句中止 ⇒ **不产生**意图记录。
  3. **幂等发布入口**：`publish_column_narrowing(table, removed_column_index, new_column_count)` —— 判据 = `__tables` 行的 `column_count`；已等于 `new_column_count` ⇒ **直接返回成功且不改任何页**（**必须**在改写列行之前判定）；否则执行「改写列行 → 下调列数 → 切换内存活布局 → 清除该表在途窄化状态」。重复调用得到**相同结果**。
  4. **正常路径**：提交完成后调用该发布入口（既有 `apply_column_drop` 改为它的薄封装）。
  5. **恢复期发布**：`full_recover` 收集全部窄化意图；对**事务已提交**的意图执行发布，**排在重放之前**（活布局的列定义决定重放与索引重建的解码宽度）；对**事务未提交**（中止 / 未提交）的意图**不发布**。发布后**同步切换内存活布局**。
  6. **发布失败**：**不回滚**已提交的数据面，**不把已提交的语句改判为失败**；**保留**该表的在途窄化状态（读继续被拒绝），使读**不**退化为按旧布局错读；意图记录留在 WAL 中 ⇒ 下次恢复期发布。
  7. **M5**：当存在**已提交但未发布**的窄化意图时，checkpoint **不得**使该意图丢失。允许的两种实现（D2 的原话「须先发布在途窄化，或推迟截断」）：(a) 截断前先发布在途窄化；或 (b) 推迟截断。**可观察要求**：在该状态下做过 checkpoint 之后，崩溃重开**仍**发布该窄化。
  8. **磁盘格式**：**不变**（不新增页、不改页头、不改 `__tables` / `__columns` 的行布局）。
- Preserve:
  - 窄化 DDL 的**数据面**改写语义（每行 DELETE + 墓碑 + 索引清理 + 重新 INSERT + WAL 记录）、改写的**事务性**（全有或全无）、墓碑可见性判定**零改动**。
  - `rewrite_column_rows_drop` 的**既有**语义与调用点**零改动**（幂等性由**调用方**保证，不改它本身）。
  - `WalError::RedoFailed` 与既有错误文案形态；恢复的阶段序（W1 → 重放 → W4 → W3）**不变**（发布作为**新增的前置阶段**插入，不改动既有阶段的相对顺序）。
  - 既有 WAL 记录类型 `0x01`–`0x09` 的语义与编码**零改动**（新类型追加，不改旧类型）。
  - `will_redo` 门控、`redo_count == 0` 的 clean 打开路径**零改动**。
  - `src/wal/recovery.rs` 内 Iteration 005 交付的有界读与处置分立**零改动**。
- Forbidden:
  - 改 `__tables` / `__columns` 的**行布局**、改页格式、改文件头 ⇒ 磁盘格式变化。
  - 新增**非事务作用域**的意图载体（不得把意图只写进目录行；不得引入独立的意图表 / 意图文件）。
  - 把窄化意图做成**独立于事务**的记录（必须随该事务提交 / 中止一起落地）。
  - 改既有 WAL 记录类型 `0x01`–`0x09` 的编码或语义；引入 WAL **文件格式版本**。
  - 新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制（Hash / 指纹 / revision pin / run-id / manifest / 时间证明）。
  - 改 dump / restore 合同面、改 `rtsql check` 的诊断分级。
  - 跑全量回归。
- First test:
  - 位置：`tests/ddl_layout_publication_test.rs`（**新增测试文件**；真实文件库 + 真实 `Database::open` + **真实文件字节 / 真实进程崩溃**，无替身 / 桩）。
  - **夹具**（崩溃重开，真实入口）：
    1. `Database::open(path)` → `CREATE TABLE t (id INT PRIMARY KEY, v TEXT)` → 插入 ≥ 2 行 → 提交。
    2. 用**既有**注入点 `src/executor/ddl_commit_window.rs`（`arm_ddl_commit_window(path)`）把 `ALTER TABLE t DROP COLUMN v` 停在**「提交完成 → 目录应用」窗口内**（该窗口正是 ISS25 的错读窗口）⇒ 该语句的数据面改写**已提交**、目录窄化**未发布**。
    3. **放弃该语句并让进程「崩溃」**：丢弃语句 future（`PendingColumnDrop` 随语句析构）⇒ 目录窄化**未发布**；随后**不 close**该库（不刷盘、不 checkpoint、不截断 WAL）⇒ 磁盘上的目录仍是旧列数。
    4. **重新打开**该库（`Database::open(path)`）⇒ 触发恢复。
  - **RED（变更前，实测形态）**：重开后目录列数**仍是 3**（旧布局）而数据面已是 2 列宽 ⇒ 读 `SELECT id, v FROM t` 按位置取到**错位的值**（旧列序：第 3 个字段读到窄行填充的 NULL / 越界读）⇒ **预期 RED：重开后的读结果与窄化后的预期不一致**（断言落在**可观察的读值**上，不是断言内部状态）⇒ 断言失败 ⇒ **不是**编译失败、**不是**进程 abort。
  - 辅助断言（同一夹具）：重开后 `__tables` 行的列数 == 2（窄化后）、`__columns` 少一行；重开后 `SELECT id, v FROM t` 的每行值 == 窄化前的 `id` 与 `v`（**不**含错位的第三列）。
  - **未损坏对照**：同样的库但**不**执行窄化 ⇒ 重开后行为与既有行为逐点一致（列数 3、读值一致）。
- GREEN condition:
  - 崩溃重开后：目录列数 == 2、列定义少一行、读值 == 窄化后的预期（**不**错位）、库能正常打开（无错误、无 abort）。
  - **未提交**的窄化意图**不被发布**：重开后列数仍为 3、读值与语句执行前一致。
  - **幂等**：正常路径发布一次 + 恢复期再发布一次 ⇒ 目录列数与列定义与只发布一次**相同**（列数 2、列定义 1 行；**不得**出现列行被误删）。
  - M5：在「已提交但未发布」状态下做过 checkpoint 之后崩溃重开 ⇒ 窄化**仍被发布**（列数 == 2、读值不错位）。
  - `cargo test --test ddl_layout_publication_test -- --test-threads=1` 全绿；`cargo test --lib` 与本 Cycle 的定向回归清单全绿；`cargo fmt --check` / `cargo clippy --all-targets` 在**本 Cycle 触碰的文件**上零新增 finding。
- Verification:
  - 行为类（真实入口）：`cargo test --test ddl_layout_publication_test -- --test-threads=1` ⇒ 覆盖「崩溃重开后已提交的窄化被发布」「未提交的窄化不被发布」「发布的幂等性」「检查点不丢失未发布的意图」「未损坏对照」。
  - 单元与库面：`cargo test --lib`。
  - 定向回归（**M19 分批串行纪律**：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2）：
    - 批 1（8 个）：`cargo test --test catalog_bounded_read_test --test ddl_catalog_transactional_test --test ddl_crash_recovery_test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test free_page_lifecycle_test --test legacy_compat_test --test recovery_bounded_read_test -- --test-threads=1`
    - 批 2（8 个）：`cargo test --test recovery_e2e_test --test recovery_test --test release_durability_precondition_test --test wal_auto_checkpoint_test --test wal_integration_test --test wal_record_test --test write_path_slot_bounds_test --test data_page_bounded_read_test -- --test-threads=1`
    - 通过条件：两批全部 0 failed。
  - **本 Cycle 不跑全量回归**（整个 change 的全量预算在 **Iteration 009 / T16**：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、全量 Gate = 全部批次各自 0 failed）。
  - 失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task（不进入下游 task）。
- Stop when: 需要改 `__tables` / `__columns` 的行布局、页格式或文件头（磁盘格式变化）；需要把窄化意图做成非事务作用域的载体；需要把窄化意图写进目录行而不是 WAL；需要改既有 WAL 记录类型 `0x01`–`0x09` 的编码；需要把发布改成非幂等或把「幂等」建立在 `rewrite_column_rows_drop` 本身（它非幂等）；需要让发布发生在重放之后（会使重放按旧布局解码）；需要让发布失败回滚已提交的数据面或把已提交语句改判为失败；需要新增公共 API 字段 / 命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；或需要全量回归才能判定（应回到 Plan 调整验证边界）。

### T7: 提交前失败的 DDL 其目录面变更被补偿（ISS28）

- Requirement/Scenario: DLP「提交前失败的 DDL 目录面变更被补偿」（+ 新增「`ADD COLUMN` 失败后的目录面被回退」Scenario）；`ddl-catalog-transactional`（修订）；`release-durability-precondition`（依赖其「刷盘失败不丢弃脏页写」语义，**不改**该语义）
- Depends on: None（可与 T6 并行；只共享 T8 的令牌化单元的**发布 / 回退所有权**接口面）
- Targets:
  - `src/storage/data/table_manager.rs::alter_add_column`（失败回退）
  - `src/storage/catalog.rs`（新增：回退所需的**非刷盘**列数恢复入口 + 移除新增列行的入口；可复用 `rewrite_column_rows_drop` 的「删除指定序号列行」语义，但**不得**改动它本身）
- Current behavior:
  - `alter_add_column` 是 **catalog-first**（无事务、无 WAL）：`insert_column_row`（追加列行，只改脏页）→ `update_table_column_count_ddl`（**刷盘**）→ 活布局切换。
  - 若 `update_table_column_count_ddl` 的**刷盘失败** ⇒ 语句失败，但缓冲池里 `__tables` 行的 `column_count` **已被上调**（页仍是脏的）⇒ 其后任一次成功刷盘把它落盘 ⇒ **重开库可见该列** ⇒ 失败的 DDL 留下了目录面变更（ISS28 的实测形态）。
  - 窄化 DDL（`DROP COLUMN`）的目录面变更在**提交之后**才发出 ⇒ 提交前失败 / 被放弃**不产生**目录面变更 ⇒ delta 的两个既有 Scenario **已由结构成立**（本 Cycle 须**锁住**该结构性质，不再依赖「恰好如此」）。
- Required behavior:
  1. **失败回退**：DDL 语句在**发出目录面变更之后**失败时，SHALL 把目录内容**恢复为语句执行前的状态**：**列定义**（移除本次新增的列行）与**列数**（恢复为语句前的值）**同时**回退 ⇒ 目录的列定义与列数**都**保持语句执行前的状态。
  2. **回退保持脏、不刷盘**：回退后的内容 SHALL **保持为待刷盘的脏页** ⇒ 其后任一次成功刷盘持久化的是**回退后的内容**（依赖 `release-durability-precondition` 的既有「刷盘失败不丢弃脏页写」语义 ⇒ 该语义**不被修改**）。
  3. **回退不做存储 IO**：回退只改缓冲池页 ⇒ 在存储正在失败时也能完成 ⇒ 补偿本身不得成为新的失败源。
  4. **窄化 DDL 的结构性质（锁定）**：`ALTER TABLE … DROP COLUMN` 在**事务提交之前**失败或被放弃时，目录的列定义与列数 SHALL 保持语句执行前的状态 —— 由「目录发布在提交之后」的结构保证（T6 的 M2），并由测试锁住。
- Preserve:
  - `alter_add_column` 的 **catalog-first 顺序**（列行先、列数后）**零改动**——该顺序是「崩溃不会留下已持久但列行未背书的列数」的既有保证。
  - `update_table_column_count_ddl` 的**刷盘语义**（提交点顺序：除列数页外全部先刷、列数页最后刷）**零改动**。
  - `rewrite_column_rows_drop` 的**既有**语义与调用点**零改动**。
  - `release-durability-precondition` 的「刷盘失败不丢弃脏页写」语义**零改动**。
  - `ddl-rewrite-consistency` 的既有语义**零改动**。
- Forbidden:
  - 把 `alter_add_column` 改成事务化 / WAL 化（那是 D2 未选的方向）。
  - 改 `update_table_column_count_ddl` 的刷盘顺序或让它承担回退语义（回退必须**不刷盘**）。
  - 把回退实现为「再刷一次盘」（会与「保持脏」的要求冲突，且在存储失败时不可行）。
  - 改动 `rewrite_column_rows_drop` 本身的语义（幂等性由调用方保证）。
  - 新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；改 dump / restore 合同面；跑全量回归。
- First test:
  - 位置：`tests/ddl_layout_publication_test.rs`（同一新增测试文件）。
  - **夹具**（真实入口 + 真实写盘失败注入）：
    1. `Database::open(path)` → `CREATE TABLE t (id INT PRIMARY KEY, v TEXT)` → 插入 ≥ 1 行 → 提交。
    2. 让**下一次写盘失败**（对存储层注入一次写失败 ⇒ 真实 I/O 失败，非替身断言）。注入点须是**既有**可用的测试支撑面；若既有面不可用，**Stop when** 返回 Plan（不得新增判定层）。
    3. 执行 `ALTER TABLE t ADD COLUMN m INT` ⇒ **预期 RED（变更前）**：语句**失败**，但目录面已变（`__tables` 行的 `column_count` 已被上调并留在脏页里）⇒ 其后**一次成功刷盘**（关闭或 checkpoint）把它落盘 ⇒ **重开库可见列 `m`** ⇒ 断言「重开后列 `m` 不存在 / 列数 == 语句前的值」**失败** ⇒ RED 成立（**行为失败**，非编译失败）。
    4. GREEN 后：同一夹具 ⇒ 语句失败 ⇒ **其后任一次成功刷盘** ⇒ **重开库**：列 `m` **不存在**、列数 == 语句前的值、`__columns` 无该列行 ⇒ 断言通过。
  - **未损坏对照**：不注入写失败 ⇒ `ALTER TABLE t ADD COLUMN m INT` 成功 ⇒ 重开后列 `m` **存在**且可读写（既有成功路径**零变化**）。
- GREEN condition:
  - 注入写失败的 `ADD COLUMN` 失败后：重开库中列 `m` **不存在**、列数 == 语句执行前的值、列定义无该列行；且**其后任一次成功刷盘**持久化的是回退后的内容（不是「回退只在内存中、刷盘后又出现」）。
  - 未注入写失败的 `ADD COLUMN` 成功路径**逐点不变**。
  - 窄化 DDL 的提交前失败 / 被放弃 ⇒ 重开后列数与列定义 == 语句执行前的值。
  - `cargo test --test ddl_layout_publication_test -- --test-threads=1` 全绿；`cargo test --lib` 与定向回归清单全绿；fmt / clippy 在触碰文件上零新增 finding。
- Verification:
  - 行为类（真实入口）：`cargo test --test ddl_layout_publication_test -- --test-threads=1` ⇒ 覆盖「失败的 `ADD COLUMN` 不留目录面变更」「其后成功刷盘持久化回退后的内容」「窄化 DDL 的提交前失败不留目录面变更」「未损坏的成功 `ADD COLUMN` 零变化」。
  - 单元与库面：`cargo test --lib`。
  - 定向回归：与 T6 **同一份清单与同一分批方式**（批 1 / 批 2 各 8 个目标）⇒ 通过条件 = 两批全部 0 failed。
  - **本 Cycle 不跑全量回归**（预算归 Iteration 009 / T16）。
  - 失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task。
- Stop when: 需要把 `alter_add_column` 改成事务化 / WAL 化；需要改 `update_table_column_count_ddl` 的刷盘顺序；需要用「再刷一次盘」实现回退；需要改动 `rewrite_column_rows_drop` 本身；既有测试支撑面无法注入一次真实写失败（须返回 Plan 决定注入点，**不得**新增判定层 / 替身）；需要新增命令参数 / 配置项 / 缓存 / 闸门机制；或需要全量回归才能判定。

### T8: 同表并发的窄化 DDL 被显式拒绝，在途状态由带所有者令牌的每表状态承载（ISS29）

- Requirement/Scenario: DLP「同表布局 DDL 串行化」（范围裁定为**窄化 DDL**，+ 新增边界 Scenario）；`ddl-catalog-transactional`（修订）
- Depends on: None（但 T6 的发布入口依赖令牌化的清除语义 ⇒ 本 task 的**状态单元改造**须先于 T6 的发布入口落地）
- Targets:
  - `src/storage/data/table_manager.rs::NarrowingState`（`in_flight: AtomicBool` → **所有者令牌**；新增 `try_acquire` / `release` / 所有者门控的 `publish_narrowing` / `clear_narrowing`）
  - `src/storage/data/table_manager.rs::NarrowingGuard`（持有令牌；析构时**只**释放自己的令牌）
  - `src/storage/data/table_manager.rs::alter_drop_column`（**在数据面改写开始之前**取令牌；取不到 ⇒ 显式拒绝）
- Current behavior:
  - 在途状态是**「单表一个布尔量」**（`AtomicBool`）⇒ 两个交互缺陷：
    1. **重叠清除**：同表并发窄化 DDL 中，后完成者 `clear()` 会把仍在窗口内的读拒绝清掉 ⇒ 读者在窗口内不再被拒绝 ⇒ 读到错位的值。
    2. **豁免读按旧 schema 解码**：第二条 DDL 的改写自身读（`alter_drop_column` 读旧行）会把已被第一条 DDL 窄化的行按**旧** schema 解码。
  - 同表的两条窄化 DDL **没有**任何互斥 ⇒ 两条改写争同一条数据链与目录。
- Required behavior:
  1. **所有者令牌**：每表的在途状态由**带所有者令牌的每表状态**承载（**不得**是「单表一个布尔量」）⇒ 令牌取值为每表单调递增计数器（**仅作互斥所有权标记，不承载任何身份语义**：不是 run-id / 指纹 / revision / 时间证明）。
  2. **显式拒绝**：同表已有窄化 DDL 在途时，第二条窄化 DDL SHALL 以**明确的错误**结束，错误 SHALL **点名该表**。
  3. **被拒绝者不启动改写**：被拒绝的语句 SHALL NOT 启动数据面改写 ⇒ 该表的数据面与目录面 SHALL 不受该语句影响。
  4. **只有持有者能清除**：在途状态 SHALL 只由令牌持有者清除 ⇒ 非持有者的路径 SHALL NOT 清除它 ⇒ 读拒绝 SHALL 保持，直到持有者完成发布。
  5. **拒绝面的可判定性**：错误文案点名该表 ⇒ 可观察；**不得**为此新增公共 API 字段 / 判定层。
- Preserve:
  - 读拒绝的**既有**语义与文案形态（`ensure_readable` 点名该表拒绝解码）**零改动**（只改其**清除**的所有权条件，不改拒绝条件本身）。
  - 窄化 DDL 的**既有**成功路径**零变化**（单条 DDL 不受影响）。
  - `alter_add_column` **不纳入**令牌（其同表并发已被 `self.tables.write()` 串行化）⇒ 并发 `ADD COLUMN` 的既有成功行为**零改动**。
  - `ddl-rewrite-consistency` / `mvcc-tombstone-visibility` 的既有语义**零改动**。
- Forbidden:
  - 把「拒绝」改成「排队等待」⇒ 方向裁定是**拒绝**（D4）。
  - 把令牌做成携带身份语义的东西（hash / 指纹 / revision / run-id / 时间戳）。
  - 让**非持有者**也能清除在途状态（那正是 ISS29 的重叠清除缺陷）。
  - 在取得令牌**之前**启动数据面改写。
  - 新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；改 dump / restore 合同面；跑全量回归。
- First test:
  - 位置：`tests/ddl_layout_publication_test.rs`（同一新增测试文件）。
  - **夹具（真实入口）**：
    1. `Database::open(path)` → `CREATE TABLE t (id INT PRIMARY KEY, v TEXT)` + `CREATE TABLE u (id INT PRIMARY KEY, w TEXT)` → 各插入 ≥ 1 行 → 提交。
    2. 用**既有**注入点 `src/executor/ddl_commit_window.rs`（`arm_ddl_commit_window(path)`）把第一条 `ALTER TABLE t DROP COLUMN v` 停在**「提交完成 → 目录应用」窗口内** ⇒ 表 `t` 上有窄化 DDL 在途。
    3. 在第一条在途时执行第二条 `ALTER TABLE t DROP COLUMN w`（表 `t`）⇒ **预期 RED（变更前）**：第二条**不被拒绝**（当前无互斥）⇒ 它启动改写并（可能）成功 ⇒ 断言「第二条以明确错误结束且错误点名该表」**失败** ⇒ RED 成立（**行为失败**）。
    4. **跨表对照**：第一条在途时执行 `ALTER TABLE u DROP COLUMN w`（表 `u`）⇒ **不受影响**（成功）⇒ 断言通过（此条在变更前**已 GREEN**，是对照）。
  - **GREEN 判据**：第二条（表 `t`）以**明确的错误**结束，错误文案**点名该表**；表 `t` 的数据面与目录面**不受**该语句影响（列数与列定义 == 拒绝前的状态）；跨表（表 `u`）的窄化**照常成功**。
- GREEN condition:
  - 同表并发窄化 DDL ⇒ 第二条以明确错误结束（点名该表）+ **未启动改写**（该表目录面与数据面不变）。
  - 跨表并发窄化 DDL ⇒ 不受影响（成功）。
  - 读拒绝在**拒绝路径**后**仍保持**（第一条在途期间读该表仍被拒绝）⇒ 非持有者的清除**未发生**。
  - 单条窄化 DDL 的成功路径**零变化**；`cargo test --test ddl_layout_publication_test -- --test-threads=1` 全绿；`cargo test --lib` 与定向回归清单全绿；fmt / clippy 在触碰文件上零新增 finding。
- Verification:
  - 行为类（真实入口）：`cargo test --test ddl_layout_publication_test -- --test-threads=1` ⇒ 覆盖「同表并发窄化 DDL 被显式拒绝且点名该表」「被拒绝者不启动改写」「跨表并发不受影响」「拒绝路径不清除读拒绝」。
  - 单元与库面：`cargo test --lib`。
  - 定向回归：与 T6 **同一份清单与同一分批方式** ⇒ 通过条件 = 两批全部 0 failed。
  - **本 Cycle 不跑全量回归**（预算归 Iteration 009 / T16）。
  - 失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task。
- Stop when: 需要把「拒绝」改成排队；需要让非持有者也能清除在途状态；需要在取得令牌之前启动改写；需要把令牌做成携带身份语义的东西；需要把 `ADD COLUMN` 纳入令牌（会回退并发 `ADD COLUMN` 的既有成功行为）；需要新增公共 API 字段 / 命令参数 / 配置项 / 缓存 / 闸门机制；或需要全量回归才能判定。

**Invariants**

- **磁盘格式不变**：不新增页、不改页头、不改 `__tables` / `__columns` 的行布局 ⇒ `database-file-format-header` 与 `legacy-format-compat` 的 G1 / G2 / G3 支持范围**逐条保持**。
- **窄化 DDL 的数据面语义不变**：改写的事务性（全有或全无）、行级 DELETE + INSERT + 墓碑 + 索引清理、墓碑可见性判定**逐点一致**。
- **既有恢复语义不变**：恢复的阶段序相对顺序、`will_redo` 门控、`redo_count == 0` 的 clean 打开路径、Iteration 005 交付的有界读与处置分立**零改动**（恢复期发布是**新增的前置阶段**）。
- **既有 WAL 记录类型不变**：`0x01`–`0x09` 的编码与语义**零改动**；新类型**追加**。
- **不引入身份证明机制、判定层、裁决脚本、新命令参数、新配置项、新缓存、新闸门机制、新公共 API 字段**。
- **不触碰** dump / restore 合同面与 `rtsql check` 的诊断分级。
- **失败不静默**：窄化 DDL 的失败（改写失败 / 提交失败 / 发布失败）SHALL 以**明确错误**结束或保持**显式读拒绝**；SHALL NOT 退化为「按旧布局错读」。

**Non-goals**

- T9 / T10（ISS30 / ISS31，写盘失败状态面）→ Iteration 007。
- T11 / T12（ISS33 / ISS32，dump / restore）→ Iteration 008。
- T13–T16（语料对齐 + 收尾全量回归）→ Iteration 009。
- `ALTER TABLE … ADD COLUMN` 的**事务化 / WAL 化**（D2 未选的方向；`schema-persistence` 的该条修订面**仅限**窄化 DDL 的意图记录）。
- `ADD COLUMN` 与窄化 DDL **并发**的竞态（`ADD COLUMN` 读到旧列数而窄化 DDL 正在改写）——见「Risks and Notes」的 Issue 候选，**只报告不登记**。
- `ADD COLUMN` 语句在**列数刷盘期间被放弃**（语句 future 被丢弃）时的目录面回退——见「Risks and Notes」的 Issue 候选，**只报告不登记**。
- 目录行布局的代际升级、`__tables` 的意图列、独立的意图表 / 意图文件。
- 事务作用域之外的窄化意图持久化；WAL 文件格式版本。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A20 | 窄化意图作为**事务作用域的 WAL 记录**与该事务的提交记录**同一次持久化**落地；改写失败 / 语句中止 ⇒ 不产生意图 | DLP「布局窄化的意图与数据面改写原子绑定」+「窄化意图与数据面改写同一次持久化」 | D2 M1 | T6 | `src/wal/record.rs`（新记录类型 `0x0A`）、`src/storage/data/table_manager.rs::alter_drop_column` | `tests/ddl_layout_publication_test.rs::committed_narrowing_is_published_after_crash`（计划名） |
| A21 | 崩溃重开后**已提交**的窄化被发布：目录列数与数据面行宽一致，读**不**按旧列序错位；**未提交**的窄化**不**被发布 | DLP「崩溃重开后已提交的窄化被发布」+「未提交的窄化不被发布」 | D2 M1 + M2 | T6 | `src/wal/recovery.rs::full_recover`（意图收集 + 发布，排在重放之前）、`src/storage/data/table_manager.rs::publish_column_narrowing` | 同上 |
| A22 | 发布是**幂等**的：重复发布得到相同结果（列数与列定义不变、列行不被误删） | DLP「发布的幂等性」 | D2 M2 | T6 | `src/storage/data/table_manager.rs::publish_column_narrowing`（以 `__tables` 行的 `column_count` 判定） | 同上 |
| A23 | 「已提交但未发布」状态下，checkpoint **不使**窄化意图丢失；崩溃重开**仍**发布该窄化 | DLP「未发布的窄化意图不因检查点而丢失」（**本 Cycle 新增**） | D2 M5 | T6 | `src/wal/checkpoint.rs::CheckpointManager::checkpoint`、`src/storage/data/table_manager.rs`（未发布意图登记面） | 同上 |
| A24 | 提交前失败的 DDL：目录的列定义**与**列数都保持语句执行前的状态；回退后的内容**保持为脏页**，其后任一次成功刷盘持久化的是回退后的内容 | DLP「提交前失败的 DDL 目录面变更被补偿」+「失败的 `ADD COLUMN` 目录面被回退」（**本 Cycle 新增**） | D2 M3 | T7 | `src/storage/data/table_manager.rs::alter_add_column`、`src/storage/catalog.rs`（非刷盘的回退入口） | `tests/ddl_layout_publication_test.rs::failed_ddl_catalog_change_is_compensated`（计划名） |
| A25 | 窄化 DDL 在提交前失败 / 被放弃 ⇒ 不留目录面变更（由「发布在提交之后」的结构保证） | DLP「失败的 DDL 语句不留下目录面变更」+「被放弃的 DDL 语句不留下目录面变更」 | D2 M2 + M3 | T7 | `src/executor/alter_table.rs`（DROP COLUMN 臂的提交 / 发布顺序） | 同上 |
| A26 | 同表并发窄化 DDL ⇒ 第二条以**明确错误**结束（错误**点名该表**）且**未启动数据面改写**；跨表并发**不受影响** | DLP「同表布局 DDL 串行化」（范围裁定为窄化 DDL）+「同表并发的窄化 DDL 被显式拒绝」 | D4 | T8 | `src/storage/data/table_manager.rs::NarrowingState`（所有者令牌）、`::alter_drop_column`（改写前取令牌） | `tests/ddl_layout_publication_test.rs::concurrent_same_table_ddl_is_refused`（计划名） |
| A27 | 在途状态**只由令牌持有者清除** ⇒ 拒绝路径**不**丢失读拒绝；读拒绝保持到持有者完成发布 | DLP「在途状态的清除不丢失读拒绝」 | D4 | T8 | `src/storage/data/table_manager.rs::NarrowingState::clear_narrowing`（所有者门控）、`::NarrowingGuard` | 同上 |
| A28 | **磁盘格式不变**；既有 WAL 记录类型、既有恢复阶段序、既有 DDL 数据面语义、既有读拒绝文案**零改动**；未损坏路径行为零变化 | `database-file-format-header` + `legacy-format-compat` G1 / G2 / G3 + `ddl-rewrite-consistency` + `mvcc-tombstone-visibility` + `drop-persistence-boundary` + `release-durability-precondition` | D2「磁盘格式不变」 | T6, T7, T8 | 全部触碰点 | `tests/ddl_layout_publication_test.rs`（未损坏对照）+ `cargo test --lib` + 定向回归清单 |

**Compatibility Cross-Check（兼容面逐条核对：保持 / 修订 / 例外）**

| 既有合同 / 域 | 结论 | 依据 |
|---|---|---|
| `legacy-format-compat`（G1 / G2 / G3 支持范围） | **保持** | G1 / G2 / G3 约束的是**目录行布局代际**（`__tables` / `__columns` 行的字段布局），与 **WAL 记录类型**是独立格式面；本 Cycle 不改目录行布局 ⇒ 不落在 G1 / G2 / G3 的支持范围内。G1 的「旧库带未决写入记录打开后恢复已提交行」：旧 WAL 不含新记录类型 ⇒ 旧库恢复行为逐点不变 ⇒ 保持。 |
| `database-file-format-header` | **保持** | 不新增页、不改页头、不改页格式 ⇒ 文件头与页格式逐字不变。 |
| `schema-persistence` | **修订（最小面）** | 该域的既有条款是「DDL 操作不写 WAL 记录」；本 Cycle 为**窄化 DDL** 新增一种事务作用域的 WAL 记录（窄化意图）。修订只覆盖这一条；`CREATE TABLE` / `DROP TABLE` / `ADD COLUMN` / `CREATE INDEX` 仍不写 WAL 记录 ⇒ 修订面明确且最小。 |
| `ddl-catalog-transactional` | **修订（窄化 DDL 的发布时点）** | 该域原条款「DDL 目录变更随事务提交或中止」对窄化 DDL 不再逐字成立：窄化的**目录发布**改为提交**之后**的幂等步骤（D2 M2）；**数据面**仍随事务提交或中止。`ADD COLUMN` 的既有条款方向不变，其「失败不留变更」由 T7 的**补偿**兑现。 |
| `ddl-rewrite-consistency` | **保持** | 行级改写语义、墓碑可见性、窄行读取语义零改动。 |
| `mvcc-tombstone-visibility` | **保持** | 墓碑可见性判定零改动；本 Cycle 只改目录面的发布时点与恢复期发布。 |
| `drop-persistence-boundary` | **保持** | 本 Cycle 不改释放路径。 |
| `release-durability-precondition` | **保持（被依赖，不被修改）** | T7 的补偿**依赖**「刷盘失败不丢弃脏页写」⇒ 依赖方向是「使用既有语义」。 |
| `ddl-layout-publication`（本 change 的 delta） | **扩展（不新建）** | 本 Cycle 已补 M5 的 Requirement + 2 个 Scenario、T7 的 `ADD COLUMN` 回退 Scenario、T8 的范围与边界 Scenario ⇒ 扩展既有域。 |

**Verification**

- 逐 scenario 的最简直接判定（行为类 scenario 走**真实入口**：真实建库 → 真实写入 → 真实崩溃（丢弃语句 + 不 close）→ 真实重开触发恢复；断言落在**可观察的读值 / 列数 / 退出码**上）：
  1. **A20 / A21 / A22 / A23**：同一夹具（`ddl_commit_window` 停在提交后 / 目录应用前）⇒ 崩溃重开后列数 == 2、列定义少一行、读值 == 窄化后预期（**不**错位）；未提交变体 ⇒ 列数仍 == 3；幂等变体 ⇒ 两次发布后列数与列定义**相同**；检查点变体 ⇒ 检查点 + 崩溃重开后窄化**仍被发布**。
  2. **A24 / A25**：注入一次真实写失败 ⇒ `ADD COLUMN` 失败 ⇒ 其后成功刷盘 ⇒ 重开库列 `m` **不存在**、列数 == 语句前；窄化 DDL 的提交前失败 ⇒ 重开库列数与列定义 == 语句前。
  3. **A26 / A27**：`ddl_commit_window` 让第一条窄化在途 ⇒ 第二条同表窄化以明确错误结束（点名该表）+ 该表目录面不变；跨表窄化成功；拒绝后第一条在途期间读该表**仍被拒绝**。
  4. **A28**：未损坏对照 + `cargo test --lib` + 定向回归 16 个目标 ⇒ 全部 0 failed。
- **不建判定层**：判定只落在 `cargo test` 的断言与退出码；不新增脚本 / 封装命令 / 专用参数；不使用 Hash / 指纹 / revision pin / run-id / manifest / 时间证明。
- **验证边界**：本 Cycle 跑上列定向回归，**不跑全量**（整个 change 的全量预算在 Iteration 009 / T16）。

**Gate 2 Readiness**

- Step 5 逐项复核：TBD / TODO = 无；需求裁剪 = 无（delta spec 的 DLP 域已按 M1 / M2 / M3 / M4 / M5 五面写细；M5 的 Requirement + 2 个 Scenario、T7 的 `ADD COLUMN` 回退 Scenario、T8 的范围与边界 Scenario 已在本 Cycle 补入 ⇒ **Simplified = 0**）；tasks / specs / design 一致 = 一致（T6 / T7 / T8 = D2 的 M1–M5 与 D4，与 DLP delta 的四条 Requirement 逐条对应）；Current-State Evidence 全部来自当前工作区的逐行核对（`src/wal/record.rs` 的类型字节与帧格式、`src/wal/buffer.rs` 的 `append` / `append_commit_and_wait`、`src/transaction/manager.rs` 的 `BeginTxn` / `CommitTxn` 时序、`src/storage/catalog.rs` 的三处写入点与「列数是唯一列数权威」的规则、`src/storage/data/table_manager.rs` 的 `NarrowingState` / `NarrowingGuard` / `alter_drop_column` / `alter_add_column` / `apply_column_drop`、`src/executor/alter_table.rs` 的 DROP COLUMN 臂与提交点、`src/executor/ddl_commit_window.rs` 的注入点、`src/database.rs` 的打开顺序、`src/wal/recovery.rs` 的阶段序与 `extract_pk_key` 对活布局的依赖、`src/wal/checkpoint.rs` 的截断步骤、既有合同的逐条核对）；每个 task 的执行依据完整；调用者、影响范围、测试入口已定位；未修改无关范围（Forbidden 已逐 task 列明）；Iteration Plan 覆盖全部任务且高开销验证预算明确（全量归 Iteration 009 / T16）；**Act 只需读本 Cycle 文件即可建立第一个先行测试并执行**（崩溃窗口的注入点、字节 / 状态观测面、预期 RED 形态、错误文案形态、兼容性核对结论均已写明，无需回读前序 Cycle / 其它规划资料 / 重新调查）；非实质未知项已留在 Risks and Notes；验证直接证明目标行为，行为类 scenario 走真实入口，无身份证明机制与判定层。⇒ **全部 PASS**。
- RTM：DLP「布局窄化的意图与数据面改写原子绑定」+「提交点是事务提交，目录窄化是提交后的幂等发布」+「未发布的窄化意图不因检查点而丢失」⇒ T6 ⇒ Covered；DLP「提交前失败的 DDL 目录面变更被补偿」⇒ T7 ⇒ Covered；DLP「同表布局 DDL 串行化」⇒ T8 ⇒ Covered；既有域的兼容性核对（`schema-persistence` / `ddl-catalog-transactional` 修订面 + 其余 6 个域保持）⇒ T6 / T7 / T8 ⇒ Covered（无 `Missing`、无 `Simplified`）。
- 设计闭合：意图的持久载体（WAL 事务作用域记录）、发布时点（提交后）、发布幂等判据（`__tables` 行的 `column_count`）、恢复期发布的位置（重放之前）、发布失败的语义（不改判失败 + 保留读拒绝）、补偿的内容（列定义 + 列数）与手段（只改脏页、不刷盘）、令牌的所有权与拒绝语义、三个任务的边界与禁止项均已定稿 ⇒ 无 TBD。
- 一致性：本 Cycle 与 change 的 `tasks.md`（T6 / T7 / T8 归属 Iteration 006）、`specs/ddl-layout-publication/spec.md`、`design.md` D2 / D4 一致。
- Persisted Evidence：`none`（见下）。
- 用户批准：本 goal 指令（「继续实施，但是缩小目标，只把当前 ms 做完就好」；原始 goal：「按照流程把最新规划的这一批 ms 做完，对于需要决定方向的地方，按照全局最优解处置，中途发现并登记的 issue 直接在那个 change 分配 iter 进行解决，要求遵守各种规范，尤其是多批次串行测试规范，开始吧」）构成 Gate 1 / Gate 2 的批准授权；**批准不降低判据**，上述各项仍逐条检查并记录。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| DLP「布局窄化的意图与数据面改写原子绑定」（+「窄化意图与数据面改写同一次持久化」「崩溃重开后已提交的窄化被发布」「未提交的窄化不被发布」） | T6 | `tests/ddl_layout_publication_test.rs::committed_narrowing_is_published_after_crash`（计划名） | Covered |
| DLP「提交点是事务提交，目录窄化是提交后的幂等发布」（+「发布的幂等性」） | T6 | 同上（幂等变体） | Covered |
| DLP「未发布的窄化意图不因检查点而丢失」（**本 Cycle 新增**） | T6 | 同上（检查点变体） | Covered |
| DLP「提交前失败的 DDL 目录面变更被补偿」（+「失败的 `ADD COLUMN` 目录面被回退」**本 Cycle 新增**） | T7 | `tests/ddl_layout_publication_test.rs::failed_ddl_catalog_change_is_compensated`（计划名） | Covered |
| DLP「同表布局 DDL 串行化」（范围裁定为窄化 DDL）+「在途状态的清除不丢失读拒绝」 | T8 | `tests/ddl_layout_publication_test.rs::concurrent_same_table_ddl_is_refused`（计划名） | Covered |
| 既有域兼容性（`schema-persistence` / `ddl-catalog-transactional` 修订面；`legacy-format-compat` G1 / G2 / G3、`database-file-format-header`、`ddl-rewrite-consistency`、`mvcc-tombstone-visibility`、`drop-persistence-boundary`、`release-durability-precondition` 保持） | T6, T7, T8 | `tests/ddl_layout_publication_test.rs`（未损坏对照）+ `cargo test --lib` + 定向回归 16 目标 | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 16 个既有回归目标），命令与实际结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录。

**Risks and Notes**

- **既有基线条件（非本 Cycle 引入，不纳入本 change）**：项目无 `rust-toolchain.toml`；HEAD（`e9cb9d3`）上 `cargo fmt --check` 报 **50 处 diff 分布在 9 个既有文件**，`cargo clippy --all-targets -- -D warnings` 报 **17 条 lib 错误**（deny 模式下 lib 即中止）、非 deny 模式 **32 条 warning** ⇒ 基线在 HEAD 即红。本 Cycle **只核对本 Cycle 触碰的文件**零新增 finding，**不重排**无关文件 ⇒ 该排除已记录在 `design.md` D13（ISS36）。
- **写盘失败注入点（条件性风险，非实质未知项）**：T7 需要**一次真实写盘失败**的注入面。既有测试支撑面若不可用，**Stop when** 生效（返回 Plan 决定注入点）⇒ **不得**新增判定层 / 替身 / 专用参数。此项在 Act 的实施顺序中**先于** T7 的实现落地。
- **`rewrite_column_rows_drop` 非幂等（条件性风险）**：该函数第二次执行会删掉已被上移的行 ⇒ 幂等性**必须**由发布入口在改写列行**之前**用 `__tables` 行的 `column_count` 判定 ⇒ **不得**改动该函数本身来「让它幂等」。
- **`ADD COLUMN` 与窄化 DDL 并发的竞态（Issue 候选，只报告不登记）**：`alter_add_column` 不持有 `self.tables.write()` 之外的跨语句互斥 ⇒ 它与在途的窄化 DDL 并发时，可能按**旧**列数追加列行，而窄化 DDL 正在改写同一张表的数据链与目录。D2 未选该形态、ISS29 台账未覆盖 ⇒ **不在本 Cycle 范围**（把 `ADD COLUMN` 纳入令牌会回退「并发 `ADD COLUMN` 成功」的既有行为）⇒ 报告，**不登记**（台账的建立 / 关闭由 Recorder 按用户指令执行）。
- **`ADD COLUMN` 在列数刷盘期间被放弃时的目录面残留（Issue 候选，只报告不登记）**：语句 future 在 `update_table_column_count_ddl` 的刷盘期间被丢弃时，列数可能已上调并落盘 ⇒ 该被放弃的语句留下目录面变更。delta 的「被放弃的 DDL 语句不留下目录面变更」Scenario **限定为窄化 DDL**（其结构已保证）⇒ 该形态**不在本 Cycle 范围** ⇒ 报告，**不登记**。
- **发布失败后的表不可读（已知代价，非缺陷）**：发布失败时保留该表的在途窄化状态 ⇒ 读继续被拒绝（直到下次恢复期发布）⇒ 这是「不改成按旧布局错读」的有意代价（D2 M2）。
- **WAL 新记录类型的向前兼容代价（已知代价，非缺陷）**：新构建写出的 WAL 含新类型 ⇒ **旧构建读不了新 WAL**。该代价**不在** `legacy-format-compat` 的 G1 / G2 / G3 支持范围内（G1 / G2 / G3 约束的是目录行布局代际）⇒ 已在本 Cycle 的兼容性核对中显式记录；WAL 格式无版本字段 ⇒ 不引入版本机制。
- **API 名称的耦合**：新增符号（窄化意图记录变体、`publish_column_narrowing`、令牌化接口）的**名称**由 Act 决定；本 Cycle 只约束**语义**（意图随事务持久化、发布幂等、发布在重放之前、令牌所有权门控）⇒ 名称不同**不是**返工。
- **既有结论的沿用**：Iteration 002 / 003 / 004 / 005 的结论**覆盖范围未变化** ⇒ 直接沿用，不重跑；本 Cycle **不跑全量回归**（change 预算归 Iteration 009 / T16，非跳过而是预算安排）。
- **跳过项**：不做全仓 fmt / clippy 修复（既有基线条件，ISS36）；不改 dump / restore 合同面；不改 `rtsql check` 的诊断分级；不改既有 WAL 记录类型 `0x01`–`0x09`；不改目录行布局与页格式。

**后续任务边界**

- **授权边界**：无（T6 / T7 / T8 在 goal 指令授权范围内）。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 Iteration 007（T9 / T10，写盘失败的状态面）；Act 不得自行创建下一 Cycle / 下一 Iteration。
- **不在本 Cycle、需在后续处理的事项**：T9 / T10（Iteration 007）；T11 / T12（Iteration 008）；T13 / T14 / T15 / T16（Iteration 009）；两个 Issue 候选的登记（由 Recorder 按用户指令执行）。
- **交 Maintainer / Recorder 的事项**：ISS25 / ISS28 / ISS29 台账条目的关闭须等 T6 / T7 / T8 全部完成（由 Recorder 按用户指令回填「处置指针」）；本 Cycle **不登记 Issue、不改台账**、**不改全局状态**（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均不动）、**不合并规格**、**不归档 change**。

## Act Response

**状态**: `reported`
**Cycle**: Iteration 006 / Cycle 000（`initial`）　**Task**: T6 + T7 + T8（DDL 提交点统一模型）
**Act 执行日期**: 2026-10-26

### 接手时的实际状态

- 工作区 HEAD `e9cb9d3`。未提交修改：`.agents/docs/tasks.md`（MS37–MS40 路线批次，既有现场 —— **未回滚、未清理、未提交**）+ Iteration 000–005 的产品代码与测试改动（`src/storage/page_format/slotted_page.rs`、`src/storage/page_format/mod.rs`、`src/storage/data_page.rs`、`src/storage/data/table_manager.rs`、`src/storage/buffer_pool.rs`、`src/storage/catalog.rs`、`src/storage/integrity.rs`、`src/storage/error.rs`、`src/storage/mod.rs`、`src/storage/btree/node.rs`、`src/executor/create_index.rs`、`src/executor/data_scan.rs`、`src/transaction/manager.rs`、`src/pipeline.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`、`tests/data_page_bounded_read_test.rs`、`tests/write_path_slot_bounds_test.rs`、`tests/catalog_bounded_read_test.rs`、`tests/recovery_bounded_read_test.rs`）+ ISS35–ISS38 台账条目 + 本 change 产物目录。
- **本 Cycle 无既有产物**：开工时 `src/wal/record.rs`、`src/wal/recovery.rs` 的 diff **全部**来自 Iteration 005（`src/wal/recovery.rs` 的有界读交付；`src/wal/record.rs` 为零 diff）；`tests/ddl_layout_publication_test.rs` **不存在**；`src/storage/data/table_manager.rs`、`src/storage/catalog.rs` 的既有 diff 来自 Iteration 000–005 ⇒ **不是断点续做，是从零实施**；无重复实现、无他人改动被回退。
- 前置条件核对：`Plan Context: ready`、`Review Result: pending`、无后继 Cycle；Iteration 002–005 均已 `accepted` ⇒ **可执行**。
- **既有结论失效判定**：Iteration 005 的 `cargo test --lib` 与定向回归数字因本 Cycle 改动 `src/wal/recovery.rs` / `src/storage/data/table_manager.rs` / `src/storage/catalog.rs` / `src/wal/record.rs` / `src/executor/alter_table.rs` / `src/database.rs` ⇒ **全部重跑**（见「验证」）。

### 测试顺序事实与 RED / GREEN 证据（调度方追加指令项）

**测试顺序事实**：本 Cycle 的行为变更**先落地、后写测试**（按 Cycle「Implementation Guidance」编号顺序第 5 步）。该编号顺序与同段括注「先行测试在**每个行为变更之前**落地并观察 RED」矛盾；按调度方裁定，**以括注为准** ⇒ 补做**整批真实 RED**：把本 Cycle 的行为变更**逐字回退**到变更前状态 → 运行同一批测试 → 观察真实 RED → **原样恢复** → 观察 GREEN。

| 行为变更 | 测试写入时刻 | RED 观察时刻 | 相对关系 |
|---|---|---|---|
| T8 令牌化在途状态 + 同表并发拒绝（`NarrowingState` / `NarrowingGuard` / `alter_drop_column`） | 变更落地**之后** | 回退观察（**后于**变更落地） | 违反括注 ⇒ 已用回退观察补齐 |
| T6 窄化意图 WAL 记录（`WalRecordType::ColumnNarrowing` / `WalRecord`） | — | — | **纯新增、无既有行为变化**（按 Cycle 原话：写入方使用前不可被产生 ⇒ 不构成 TDD 偏差） |
| T6 写路径写意图 + 幂等发布入口 + 恢复期发布 | 变更落地**之后** | 回退观察 | 同上 ⇒ 补齐 |
| T6 M5（checkpoint 不丢失未发布意图） | 变更落地**之后** | 回退观察 | 同上 ⇒ 补齐 |
| T7 提交前失败的目录面补偿（`alter_add_column` + catalog 非刷盘回退入口） | 变更落地**之后** | 回退观察 | 同上 ⇒ 补齐 |

**回退方式（逐字、可审计）**：以脚本 `/tmp/ms37_iter006/build_pre.py` 把本 Cycle 的 31 处编辑**逐字反向**（`new_text → old_text`），脚本对每处替换**断言恰好命中一次**（31/31 命中）；产物 = `/tmp/ms37_iter006/pre/`（6 个文件）。回退态 `cargo check --all-targets` **编译通过**（前序 Iteration 的改动原样保留）⇒ 回退态是真实的「变更前」状态。

**实测 RED 形态**（`cargo test --test ddl_layout_publication_test -- --test-threads=1`，回退态；完整日志 `/tmp/ms37_iter006/red_output.txt`）：

```
running 9 tests
test abandoned_narrowing_ddl_leaves_no_catalog_change ... ok
test checkpoint_does_not_lose_unpublished_narrowing ... FAILED
test committed_narrowing_is_published_after_crash ... FAILED
test concurrent_same_table_ddl_is_refused ... FAILED
test failed_add_column_is_compensated ... FAILED
test publication_is_idempotent_across_repeated_recoveries ... FAILED
test successful_add_column_still_adds_the_column ... ok
test uncommitted_narrowing_is_not_published ... ok
test undamaged_control_survives_crash_and_reopen ... ok
test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
```

逐用例实测 RED 形态（断言失败，非编译失败、非进程 abort）：

| 用例 | 实测 RED 文本 / 形态 |
|---|---|
| `committed_narrowing_is_published_after_crash` | `assertion left == right failed: after the crash + reopen: the catalog column count must be the narrowed count, got Shape { columns: ["id", "v", "w"], column_count: 3, rows: [[Number(1), String("v1"), Null], …] }` ⇒ `left: 3` / `right: 2`。**与 Cycle 钉死的形态一致**：目录列数仍是 3（旧布局）而数据面已是 2 列宽 ⇒ 第 3 个字段读到窄行填充的 `Null` |
| `checkpoint_does_not_lose_unpublished_narrowing` | 同上形态（`left: 3` / `right: 2`，`columns: ["id","v","w"]`，每行第 3 字段 `Null`）⇒ checkpoint 截断 WAL 后窄化意图丢失 |
| `publication_is_idempotent_across_repeated_recoveries` | 同上形态（第一次重开后 `column_count: 3` / `left: 3` / `right: 2`） |
| `concurrent_same_table_ddl_is_refused` | `panicked at tests/ddl_layout_publication_test.rs:519: the concurrent same-table narrowing DDL must be refused; it returned: AffectedRows(0)` ⇒ 第二条同表窄化 DDL **不被拒绝**（Cycle 钉死形态一致） |
| `failed_add_column_is_compensated` | `assertion left == right failed: the failed ADD COLUMN must not leave a raised column count: Shape { columns: ["id", "v", "w", "m"], column_count: 4, … }` ⇒ `left: 4` / `right: 3` ⇒ 失败语句留下目录面变更（ISS28 实测形态一致：列 `m` 存在 + 列数被抬高） |

**变更前后均为绿（既有行为的回归锁定，非本次行为变更的 RED 载体）**：`abandoned_narrowing_ddl_leaves_no_catalog_change`、`uncommitted_narrowing_is_not_published`、`successful_add_column_still_adds_the_column`、`undamaged_control_survives_crash_and_reopen` —— 变更前即绿 ⇒ 锁定既有行为（ISS21 的「提交前失败 / 被放弃不留目录面变更」在变更后**仍然成立**）。

**恢复的零残留核对**：回退观察后按文件整体还原实现版本，再逐文件 `diff` 比对：

```
identical: src/wal/record.rs
identical: src/storage/data/table_manager.rs
identical: src/wal/recovery.rs
identical: src/storage/catalog.rs
identical: src/executor/alter_table.rs
identical: src/database.rs
```

⇒ **零残留**。其后仅追加了两处**纯格式**改动（`cargo fmt` 对本 Cycle 自己的两个 hunk：`src/wal/record.rs` 的 `from_le_bytes` 排版、`src/storage/data/table_manager.rs` 的 `tables.write()` 链式排版）+ 一处 clippy 排版等价改写（`match` → `if`，语义不变）⇒ 最终代码 = 实现版本；改后重跑全部验证（见「验证」）。

**实测 GREEN 形态**：恢复后同一命令 ⇒ `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`（最终代码再次运行仍为 9 passed / 0 failed）。

### Implemented（实际改动）

**`src/wal/record.rs`** —— 新增 WAL 记录类型 `0x0A`（既有 `0x01`–`0x09` 的编码与语义**零改动**）：

| 符号 | 作用 |
|---|---|
| `WalRecordType::ColumnNarrowing = 0x0A` | 新类型**追加**；`TryFrom<u8>` 增 `0x0A` 分支 |
| `WalRecord::ColumnNarrowing { tx_id, table_name, removed_column_index, new_column_count }` | 窄化意图（事务作用域）；帧体 = `tx_id(8B) + table_name + removed_column_index(4B) + new_column_count(4B)` |
| `WalRecord::record_type` / `tx_id` / `serialize_data` / `deserialize_data` / `Display` | 新变体的四处理由 + 文案 |

**`src/storage/data/table_manager.rs`** —— 在途状态令牌化 + 窄化发布 + T7 补偿：

| 符号 | 作用 |
|---|---|
| `NarrowingState` | `in_flight: AtomicBool` → **`owner: AtomicU64`（所有者令牌）+ `next_token: AtomicU64` + `published: AtomicBool`**；`published` 仍是**读者拒绝条件本身**（拒绝窗口与既有行为逐点一致），令牌只决定「谁有权改变状态」 |
| `NarrowingState::try_acquire` | CAS 把「无持有者」换成自己的令牌 ⇒ 同表第二个窄化 DDL 取不到 ⇒ 被显式拒绝（**拒绝**，不排队） |
| `NarrowingState::publish_narrowing` / `clear_narrowing` / `release` | **所有者门控**：只有令牌持有者能发布 / 清除 / 放弃 ⇒ 非持有者的路径**不会**清掉别人窗口内的读拒绝（ISS29 的「重叠清除」） |
| `NarrowingGuard` | 持有令牌 + 析构处置（`RELEASE` / `KEEP` / `CLEARED`）：放弃路径⇒释放；发布失败⇒**保留**在途状态（读者继续被拒绝） |
| `fn narrowed_meta(old, removed)`（新增，模块内私有） | 从旧 `TableMeta` 推出窄化后布局的**单一**实现（列序 / NOT NULL / DEFAULT / 索引序位 + 链尾 + 共享状态单元）；正常路径与恢复期发布**共用** |
| `TableManager::publish_column_narrowing(table, removed_column_index, new_column_count)`（新增） | **幂等发布入口**：判据 = `__tables` 行的 `column_count`（列数唯一权威）⇒ 已等于 `new_column_count` ⇒ **直接返回成功且不改任何页**（判定**在改写列行之前**，`rewrite_column_rows_drop` 本身**零改动**）；否则「改写列行 → 下调列数 → 切换内存活布局」 |
| `TableManager::catalog_column_count`（新增，私有） | 读目录行广告的列数 |
| `TableManager::register_committed_narrowing` / `forget_committed_narrowing` / `publish_committed_narrowings`（新增） | **M5 登记面**（`committed_narrowings: Mutex<HashMap<String, CommittedNarrowing>>`）：登记「已提交但未发布」的窄化意图；checkpoint 据此先发布再截断 |
| `TableManager::apply_column_drop` | 改为 `publish_column_narrowing` 的**薄封装**：成功⇒令牌持有者清除在途状态 + 撤回登记；**发布失败⇒不回滚已提交数据面、不把已提交语句改判为失败、保留在途窄化状态**（读者继续被拒绝） |
| `TableManager::alter_drop_column` | ① 数据面改写**开始之前**取令牌（取不到 ⇒ 显式拒绝，点名该表）；② 改写**成功之后、事务提交之前**写 `WalRecord::ColumnNarrowing`；③ 返回的 `PendingColumnDrop` 不再携带 `new_meta` |
| `TableManager::alter_add_column` | **提交前失败 ⇒ 补偿**：列行写入失败与列数上调失败两条路径都调 `compensate_column_add` |
| `TableManager::compensate_column_add`（新增，私有） | **先**把广告列数恢复为语句前的值、**再**移除本次新增的列行；回退**只改缓冲池页、不刷盘** ⇒ 存储正在失败时也能完成 ⇒ 补偿本身不成为新的失败源（补偿失败⇒保留语句的原始失败） |
| `PendingColumnDrop` | 新增 `clear_narrowing` / `keep_narrowing`（令牌门控）；移除 `new_meta` 字段（活布局改由发布入口统一推导） |

**`src/wal/recovery.rs`** —— `RecoveryManager::full_recover`：

- Step 1 的事务分类**新增**窄化意图收集（`narrowing_intents`，**不因位点裁剪** —— 意图可能落在 checkpoint 位点之前）。
- **Step 1.5（新增前置阶段）**：对**事务已提交**的意图调用 `publish_column_narrowing`；**未提交**（中止 / 未提交）的意图**不发布**。发布**排在重放之前**（活布局的列定义决定重放与索引重建的解码宽度），发布失败 ⇒ 明确 `WalError::RedoFailed`。
- 既有阶段序（W1 → 重放 → W4 → W3）、`will_redo` 门控、`redo_count == 0` 的 clean 打开路径、Iteration 005 的有界读与处置分立**零改动**（发布是**新增的前置阶段**）。

**`src/storage/catalog.rs`** —— T7 的两个**非刷盘**回退入口（既有方法零改动）：

| 符号 | 作用 |
|---|---|
| `Catalog::restore_table_column_count_no_flush(name, count)`（新增） | 把 `__tables` 行的 `column_count` 恢复为语句前的值；**不刷盘**（与 `update_table_column_count_ddl` 的差别**只有**不刷盘） |
| `Catalog::remove_column_row_at(table, column_index)`（新增） | 移除 `__columns` 链上**指定序号**的列行（`rewrite_column_rows_drop` 的「删除指定序号列行」语义，**不含**序号平移；`rewrite_column_rows_drop` 本身**零改动**） |

**`src/executor/alter_table.rs`** —— DROP COLUMN 臂：事务**提交成功之后**、目录发布之前，**同步**登记「已提交但未发布」的窄化意图（`register_committed_narrowing`）；登记与提交之间无让出点 ⇒ 不出现「已提交但未登记」的窗口。提交 / 发布顺序**零改动**。

**`src/database.rs`** —— `Database::checkpoint()`：在 `CheckpointManager::checkpoint`（其内部截断 WAL）**之前**调 `publish_committed_narrowings()` ⇒ 未发布的窄化意图先落进目录面 ⇒ 截断不再丢失它；发布失败 ⇒ 返回错误 ⇒ **不截断** ⇒ 意图仍在 WAL 中。

**`tests/ddl_layout_publication_test.rs`**（**新建**）—— 9 个用例，真实文件库 + 真实 `Database::open` + 真实文件字节 + **真实进程崩溃**（不 `close` ⇒ 不刷盘 / 不 checkpoint / 不截断 WAL）；两个注入点均为**既有**面（`arm_ddl_commit_window` / `Database::arm_io_failure(IoFailPoint::DataWrite)`）；无替身 / 桩 / 判定层 / 身份证明机制：

| 用例 | 覆盖 |
|---|---|
| `committed_narrowing_is_published_after_crash` | A20 / A21（已提交的窄化在崩溃重开后被发布） |
| `uncommitted_narrowing_is_not_published` | A21（未提交的窄化不被发布）+ A25（提交前被放弃 ⇒ 不留目录面变更） |
| `publication_is_idempotent_across_repeated_recoveries` | A22（两次重开 ⇒ 两次发布 ⇒ 列数与列定义相同，列行不被误删） |
| `checkpoint_does_not_lose_unpublished_narrowing` | A23（M5：checkpoint 后崩溃重开仍发布） |
| `undamaged_control_survives_crash_and_reopen` | A28（未损坏对照） |
| `failed_add_column_is_compensated` | A24（ADD COLUMN 提交前失败 ⇒ 目录面被补偿） |
| `successful_add_column_still_adds_the_column` | A24 对照（补偿不得误伤成功路径） |
| `abandoned_narrowing_ddl_leaves_no_catalog_change` | A25（窄化 DDL 提交前被放弃 ⇒ 不留目录面变更） |
| `concurrent_same_table_ddl_is_refused` | A26 / A27（同表并发拒绝 + 拒绝不留目录面变更 + 读拒绝保持 + 跨表并发不受影响） |

### Acceptance 达成证据（A20–A28 逐条）

| # | 达成证据 |
|---|---|
| **A20** | 意图记录 = `WalRecord::ColumnNarrowing`（`src/wal/record.rs`，类型字节 `0x0A`）；写入点 = `alter_drop_column` 的数据面改写**成功之后**、事务提交**之前**（`src/storage/data/table_manager.rs`，`wal.append(WalRecord::ColumnNarrowing { … })`）⇒ 与该事务的数据面记录、`CommitTxn` 在**同一次持久化**中落地（既有 Group Commit：`append` + `append_commit_and_wait`）。**改写失败 / 语句中止 ⇒ 本行不可达 ⇒ 不产生意图**（由 `uncommitted_narrowing_is_not_published` 与 `abandoned_narrowing_ddl_leaves_no_catalog_change` 覆盖：回退态与实现态均绿 ⇒ 行为成立） |
| **A21** | `src/wal/recovery.rs::full_recover` Step 1.5：收集意图（不因位点裁剪）⇒ 事务已提交 ⇒ 发布（排在重放之前）⇒ 同步切换活布局；未提交 ⇒ 不发布。实测：`committed_narrowing_is_published_after_crash`（回退态 RED `left: 3` / `right: 2`；实现态绿）、`uncommitted_narrowing_is_not_published`（回退态与实现态均绿 ⇒ 未提交意图不被发布） |
| **A22** | `publish_column_narrowing` 的判据 = `__tables` 行的 `column_count`，判定**在改写列行之前**；`rewrite_column_rows_drop` **零改动**（非幂等性由调用方保证）。实测：`publication_is_idempotent_across_repeated_recoveries`（两次重开 ⇒ 两次发布 ⇒ 列数 2、列定义 2 行、列行未被误删） |
| **A23** | 登记面 = `TableManager::committed_narrowings`（`register_committed_narrowing` / `publish_committed_narrowings`）；checkpoint 在截断前发布。实测：`checkpoint_does_not_lose_unpublished_narrowing`（回退态 RED `left: 3` / `right: 2`；实现态绿 ⇒ checkpoint + 崩溃重开后窄化**仍被发布**，列数 == 2、读值不错位） |
| **A24** | `alter_add_column` 的两条失败路径 ⇒ `compensate_column_add`（先降列数、再移除列行；只改缓冲池页、**不刷盘** ⇒ 其后任一次成功刷盘持久化的是回退后的内容）。实测：`failed_add_column_is_compensated`（回退态 RED `columns: ["id","v","w","m"]` / `column_count: 4` / `left: 4` / `right: 3`；实现态绿 ⇒ 列 `m` 不存在、列数 == 语句前）；对照 `successful_add_column_still_adds_the_column` 绿 ⇒ 补偿不误伤成功路径 |
| **A25** | 结构保证：窄化的目录发布在**事务提交之后**（`src/executor/alter_table.rs` 的提交 / 发布顺序**零改动**）；提交前失败 / 被放弃 ⇒ 发布不可达。实测：`abandoned_narrowing_ddl_leaves_no_catalog_change` 绿（提交前放弃 + 崩溃重开 ⇒ 列数与列定义 == 语句前） |
| **A26** | `NarrowingState::try_acquire`（改写**开始之前**取令牌）+ `alter_drop_column` 的拒绝分支（`StorageError::ExecutionError`，**点名该表**：`table 't' already has an in-flight ALTER TABLE DROP COLUMN; concurrent narrowing DDL refused`）⇒ 第二条**未启动数据面改写**。实测：`concurrent_same_table_ddl_is_refused`（回退态 RED `it returned: AffectedRows(0)`；实现态绿）；**跨表并发不受影响**：同用例内 `ALTER TABLE u DROP COLUMN w` 成功（`u` 的令牌单元独立，`ADD COLUMN` 未纳入令牌） |
| **A27** | `clear_narrowing` / `release` **所有者门控**（`NarrowingState`）⇒ 非持有者的路径不改变状态。实测：同用例内「拒绝之后读该表**仍被拒绝**」断言绿（回退态下该读返回错位行 ⇒ RED）⇒ 拒绝路径**未**清掉读拒绝，读拒绝保持到持有者完成发布 |
| **A28** | 磁盘格式不变（不新增页 / 不改页头 / 不改 `__tables` / `__columns` 行布局）；既有 WAL 类型 `0x01`–`0x09` **零改动**（新类型追加）；恢复阶段序**零改动**（发布为新增前置阶段）；DDL 数据面语义（每行 DELETE + 墓碑 + 索引清理 + 重新 INSERT + WAL 记录）与墓碑可见性判定**零改动**；读拒绝文案**零改动**（`ensure_readable` 的原文与触发条件未变）。证据：`undamaged_control_survives_crash_and_reopen` 绿 + `cargo test --lib` 420 passed / 0 failed + 定向回归两批全绿（见下） |

### Deviations from Plan（与计划的偏差及原因）

1. **夹具的列数（Cycle 的 First test 文字内部矛盾）**：Cycle 的 First test 写 `CREATE TABLE t (id INT PRIMARY KEY, v TEXT)`（两列），而同段的辅助断言要求「窄化后列数 == 2」+「`__columns` 少一行」+「`SELECT id, v` 的每行值 == 窄化前的 id 与 v」⇒ 两列删一列得到 1 ⇒ 与该断言矛盾。**按断言取三列表**（`t(id INT PRIMARY KEY, v TEXT, w TEXT)`，删**第三列** `w`）⇒ 窄化后 2 列 2 行定义，`SELECT id, v` 在窄化前后均合法，读值断言自洽。RED 形态与 Cycle 钉死的一致（目录列数仍是 3、数据面 2 列宽、第 3 字段读到 `Null`）。
2. **测试名与 Cycle 的「计划名」不同**：`failed_ddl_catalog_change_is_compensated` → 实际 `failed_add_column_is_compensated`；幂等 / 检查点 / 未提交 / 未损坏对照各有独立用例名（Cycle 的 Risks and Notes 已授权 Act 决定新增符号的名称）。
3. **M5 的挂载点**：`src/database.rs::Database::checkpoint()`（`CheckpointManager::checkpoint` 的**唯一生产调用方**），而非在 `src/wal/checkpoint.rs::CheckpointManager::checkpoint` 内部。理由：① 截断发生在 `CheckpointManager::checkpoint` 内部，而发布必须**先于**截断；② `CheckpointManager` 无表管理器句柄，加一个会把 WAL 层依赖到 `storage::data`（而 `storage::data` 依赖 WAL 层 ⇒ 层反转），或引入新的「闸门机制」（Cycle 的 Forbidden）；③ `Database::checkpoint` 覆盖全部生产路径（`close()` / 显式 checkpoint / 自动 checkpoint）⇒ 可观察要求（A23）逐点满足。
4. **在途状态的清除由令牌持有者执行，而非在 `publish_column_narrowing` 内部**：发布入口不持有令牌 ⇒ 无法做所有者门控的清除（T8 的「只有持有者能清除」同样是冻结方向）⇒ `apply_column_drop`（持有令牌的守卫）在发布成功后清除；恢复期路径无在途状态（全新进程）⇒ 无需清除。
5. **`PendingColumnDrop::new_meta` 移除**：活布局改由发布入口经 `narrowed_meta`（单一实现）从活布局推导 ⇒ 正常路径与恢复期发布**同一条**路径。**已知代价**（Minor，见下）：在「`ADD COLUMN` 与在途窄化 DDL 并发」的窗口内，发布推导出的活布局来自**已被 `ADD COLUMN` 改过**的活布局（该竞态本 Cycle 明确不在范围）。
6. **实施顺序**：T8 的状态单元改造 → T6 的 WAL 记录 → T6 的写路径 / 发布入口 / 恢复期发布 → T7 的补偿（按 Cycle「Implementation Guidance」第 1–4 步与 T6 的 `Depends on: T8`），与 Cycle 建议顺序一致；测试按第 5 步落地（见「测试顺序事实」）。
7. **先行测试的补做方式**：按调度方追加指令，把行为变更**逐字回退**后观察 RED、再原样恢复（脚本 + 逐文件 `diff` 零残留核对）⇒ 满足括注「先行测试在每个行为变更之前落地并观察 RED」的实质要求。

### 自检发现、已修复内容与遗留 Minor 问题

**自检（Gate 3）发现并已修复**：
1. `NarrowingGuard::drop` 用 `match` 判常量 ⇒ clippy `match` 用于相等判定告警 ⇒ 改为 `if`（语义不变）。
2. 两处本 Cycle 新增代码不符合 `cargo fmt` ⇒ 按 rustfmt 结论改排版（只改本 Cycle 自己的 hunk）。
3. 测试初版把「目录面观测」与「数据面读」合成一个 helper ⇒ 在窄化在途窗口内 `SELECT *` 被**明确拒绝** ⇒ helper panic ⇒ 拆为 `catalog_shape`（不读数据面）与 `read_rows`（读数据面）两个入口。
4. M5 用例初版在 checkpoint 后**放行**语句再崩溃 ⇒ 语句的正常路径发布了窄化 ⇒ 该用例在回退态也绿（不具判别力）⇒ 改为 checkpoint 后**直接崩溃**（不放行）⇒ 回退态 RED / 实现态绿。

**遗留 Minor 问题（不构成 Gate 5）**：
- **M1（Minor，范围外）**：在「`ADD COLUMN` 与在途窄化 DDL 并发」的窗口内（窄化改写结束 → 提交 → 发布之间），本实现发布推导出的活布局来自已被 `ADD COLUMN` 改过的活布局 ⇒ 该窗口内的**进程内**读不再自愈（重启后由目录列数收敛）。该竞态是 Cycle **已预报**的 Issue 候选①，**本 Cycle 明确不在范围**（`ADD COLUMN` 不纳入令牌），**未实现**、**未登记**。
- **M2（Minor，范围外）**：Cycle 已预报的 Issue 候选②（`ADD COLUMN` 在**列数刷盘期间被放弃** ⇒ 列数可能已上调并落盘 ⇒ 该被放弃的语句留下目录面变更）**仍然成立**：T7 的补偿只覆盖**错误路径**，**放弃路径**（语句 future 在 `update_table_column_count_ddl` 的刷盘期间被丢弃）不会执行补偿 ⇒ 该形态未被本 Cycle 消除。**只报告，不登记**。
- **M3（Minor，已知代价）**：发布失败后该表保持不可读（保留在途窄化状态）⇒ 直到下次恢复期发布（D2 M2 的有意代价，非缺陷）。

### 验证（命令与实际结果）

| 命令 | 实际结果 |
|---|---|
| `cargo test --test ddl_layout_publication_test -- --test-threads=1`（实现态） | `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test ddl_layout_publication_test -- --test-threads=1`（**回退态** = RED 观察） | `test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out`（逐用例 RED 文本见上） |
| `cargo test --lib` | `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| 定向回归**批 1**：`cargo test --test catalog_bounded_read_test --test ddl_catalog_transactional_test --test ddl_crash_recovery_test --test ddl_layout_publication_test --test ddl_rewrite_consistency_test --test free_page_lifecycle_test --test legacy_compat_test --test recovery_bounded_read_test -- --test-threads=1` | 8 个目标全部 `ok. 0 failed`（5 / 3 / 6 / 9 / 9 / 12 / 16 / 6 passed） |
| 定向回归**批 2**：`cargo test --test recovery_e2e_test --test recovery_test --test release_durability_precondition_test --test wal_auto_checkpoint_test --test wal_integration_test --test wal_record_test --test write_path_slot_bounds_test --test data_page_bounded_read_test -- --test-threads=1` | 8 个目标全部 `ok. 0 failed`（6 / 3 / 2 / 11 / 2 / 14 / 11 / 8 passed） |
| `cargo fmt --check`（**只核对本 Cycle 触碰的文件**） | 全仓 **50 处 diff / 9 个既有文件**（与基线同数）；本 Cycle 触碰的 `src/wal/record.rs`、`src/storage/catalog.rs`、`src/executor/alter_table.rs`、`src/database.rs`、新建 `tests/ddl_layout_publication_test.rs` **零 diff**；`src/storage/data/table_manager.rs` 的 6 处 diff 全部位于 Iteration 000–005 的既有代码（`create_table` 的清理循环、`collect_data_chain_strict` / `collect_overflow_chain_pages_strict` / `collect_tree_pages` 等），**与本 Cycle 的 hunk 无关** ⇒ **零新增** |
| `cargo clippy --all-targets`（非 deny） | 全仓 **32 条 warning**（与基线同数）；本 Cycle 触碰的 6 个源文件 + 新建测试文件 **零 finding** ⇒ **零新增** |

- **全量回归**：本 Cycle **未跑**（整个 change 的全量预算在 Iteration 009 / T16：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、全量 Gate = 全部批次各自 0 failed）⇒ 非跳过，是**预算安排**。
- **既有结论**：Iteration 002–005 的结论（页级槽数判据、恢复面有界读等）覆盖范围未变化 ⇒ 直接沿用；Iteration 005 的 `cargo test --lib` 与定向回归数字因本 Cycle 改动**已失效** ⇒ **已重跑**（见上表）。
- **持久化证据**：本 Cycle 的全部结论可低成本重跑（1 个新增测试目标 + `cargo test --lib` + 16 个既有回归目标），命令与实际结果在本 Response 内承载 ⇒ 不存在「Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ **不创建 `evidence/` 目录**。

### Persisted Evidence

- `None required`（Mode: `none`；未创建 `evidence/` 目录）。

### Experience Candidates

- **None**。（本 Cycle 的两条 Issue 候选是 Cycle **已预报**的、范围外的实质缺陷，按 Cycle 要求**只报告不登记**；登记由 Recorder 按上层指令执行。）

### 未解决问题 / 交办事项

- **未解决（范围外，Cycle 已预报）**：Issue 候选①（`ADD COLUMN` 与在途窄化 DDL 的竞态）——**确认成立**（竞态窗口 = 窄化改写结束 → 提交 → 发布之间；`alter_add_column` 全程持 `self.tables.write()`，而 `alter_drop_column` 在改写期间**不**持该锁 ⇒ 可交错）；本实现未修复，且在该窗口内进程内读不再自愈（见 Minor M1）。**只报告，不登记。**
- **未解决（范围外，Cycle 已预报）**：Issue 候选②（`ADD COLUMN` 在列数刷盘期间被放弃 ⇒ 列数可能已上调并落盘 ⇒ 被放弃的语句留下目录面变更）——**确认仍成立**（T7 的补偿只覆盖错误路径，放弃路径不执行补偿）。**只报告，不登记。**
- **交 Recorder**：ISS25 / ISS28 / ISS29 的台账「处置指针」回填（须等 T6 / T7 / T8 全部完成；本 Cycle **不登记 Issue、不改台账**）。
- **交 Plan**：本 Response 与 Cycle 的 Plan Review（`accepted` / `rework-required` / `replan-required` 判定）。

### 完成声明与边界

- **Gate 3**：T6 / T7 / T8 逐任务通过（Spec compliance + Code quality：计划外修改 = 无；无新增警告；测试不因错误原因通过；无身份证明机制 / 判定层）。
- **Gate 4**：A20–A28 逐条有达成证据；完成声明基于**本 Cycle 当次真实运行**（见「验证」表）。
- **Gate 5**：**未触发**（无阻塞项、无 Blocker Handoff）。
- **跳过项**：不做全仓 fmt / clippy 修复（既有基线条件，ISS36）；不跑全量回归（预算归 Iteration 009 / T16）；不改 dump / restore 合同面；不改 `rtsql check` 的诊断分级；不改既有 WAL 记录类型 `0x01`–`0x09`；不改目录行布局与页格式；**不实现**两条已预报的 Issue 候选。
- **本 Cycle 未做**：**未归档 change**、**未同步全局文档**（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均未改；change 内的 `openspec/changes/2026-10-25-ms37-defect-ledger-closure/tasks.md` 的任务 / Iteration 状态**同步亦未做** —— 该同步属 Plan Review 的写入面，见前序 Iteration 的同位置写法）、**未创建下一 Cycle 或下一 Iteration**、**未登记或修改 Issue 台账**、**未执行 git commit**（工作区保持未提交状态）。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读了 `WalRecord::ColumnNarrowing`、`alter_drop_column`、`publish_column_narrowing`、`apply_column_drop`、`compensate_column_add`、`full_recover` 的 Step 1.5、`Database::checkpoint`、`NarrowingState` 的令牌，以及 `tests/ddl_layout_publication_test.rs` 的 9 个用例。产品文件的修改时间早于本 Cycle 文件，验证覆盖的代码没有在 Act 报告之后再改。Act Response 里的命令结果直接沿用。

A20–A28 与代码一致：

- 意图记录在数据面改写成功之后、`commit` 之前写入。`commit` 的组提交把仍在缓冲里的意图和 `CommitTxn` 一起落地。恢复只发布 `committed_tx_ids` 里的意图，并且排在重放之前。
- `publish_column_narrowing` 在改列行之前用 `__tables` 的 `column_count` 判断。已发布则直接返回。`apply_column_drop` 在发布失败时返回成功，并 `keep_narrowing`。
- `compensate_column_add` 先恢复列数、再删新增列行，两条路径都不刷盘。
- `try_acquire` 在改写开始之前。失败错误点名表名。`clear_narrowing` 只接受持有者令牌。
- 生产 checkpoint（`close`、`maybe_auto_checkpoint`、CLI、truncate）都进 `Database::checkpoint`。它先 `publish_committed_narrowings`，失败则不调用会截断 WAL 的 `CheckpointManager::checkpoint`。
- 页格式、`__tables` / `__columns` 行布局、WAL 类型 `0x01`–`0x09` 没有改。令牌是 `AtomicU64` 计数，不带运行身份。

分批验证符合本 Iteration 已写的规则，也符合 M19 对全量的限制：

- 全量没有跑。预算仍在 Iteration 009 / T16。
- 定向回归是两次独立的 `cargo test`，各 8 个目标，都带 `--test-threads=1`。批 1 为 5/3/6/9/9/12/16/6 passed，批 2 为 6/3/2/11/2/14/11/8 passed，Act 记为全部 0 failed。
- 批 2 的命令把 `data_page_bounded_read_test` 放在最后。Cycle 的 Verification 就是这个顺序。tasks.md 的 Iteration Plan 把它放在批 2 开头。目标集合相同。Act 按 Cycle 执行。
- `cargo test --lib` 是单目标。M19 允许单目标用默认并行度。D10 写的是 `--lib -- --test-threads=1`，本 Cycle 的 Verification 没有要求 `--lib` 加 `--test-threads=1`。Act 按 Cycle 跑了 `cargo test --lib`，420 passed / 0 failed。

非阻塞：

1. 行为变更先落地，再整批回退观察到 5 failed / 4 passed，恢复后 9 passed。回退态的失败文本与 Plan 钉死的形态一致（列数 3 对 2、第二条返回 `AffectedRows(0)`、`ADD COLUMN` 列数 4 对 3）。与 Iteration 005 的测试顺序偏差同类。
2. 夹具是三列表 `t(id, v, w)`。Plan 的建表句子写了两列，同段断言要求窄化后列数为 2。Act 按断言取了三列。
3. M5 挂在 `Database::checkpoint`，不在 `CheckpointManager::checkpoint` 内部。生产路径都经过前者。`CheckpointManager` 的单元测试没有表管理器。
4. 两个 Scenario 没有单独用例。「窄化令牌不约束 ADD COLUMN」：`alter_add_column` 不调用 `try_acquire`。「未提交的窄化不因检查点而被发布」：checkpoint 只发布已登记的已提交意图。`uncommitted_narrowing_is_not_published` 在改写循环里放弃，`ColumnNarrowing` 写在循环之后，所以 WAL 里没有这条未提交意图；它锁住的是提交前放弃不被发布。
5. `schema-persistence` 与 `ddl-catalog-transactional` 的修订写在本 Cycle 的核对表里。本 change 没有这两份主规格的 MODIFIED 增量。新行为在 `ddl-layout-publication`。归档时主规格里的旧句子还会在。这不挡住 A20–A28，留给收尾合并，不返工本 Cycle 的代码。

偏差分类：1 是 Act 的测试顺序，已用真实 RED 补上。2 是 Plan 文字自相矛盾，Act 按可观察断言执行。3 是 Act 偏离挂载点，可观察要求仍满足。4 和 5 是覆盖缺口，不挡住已写明的 Acceptance。

**Evidence**

- 沿用 Act Response「验证」表（2026-10-26）：`ddl_layout_publication_test` 实现态 9 passed / 0 failed；回退态 4 passed / 5 failed；`--lib` 420 passed / 0 failed；定向回归两批各 8 个目标 0 failed。
- 代码核对见 Findings。`tests/ddl_layout_publication_test.rs` 的 `committed_narrowing_is_published_after_crash`、`publication_is_idempotent_across_repeated_recoveries`、`checkpoint_does_not_lose_unpublished_narrowing`、`failed_add_column_is_compensated`、`abandoned_narrowing_ddl_leaves_no_catalog_change`、`concurrent_same_table_ddl_is_refused` 分别落到 A20–A27。A28 由未损坏对照、`--lib` 和两批定向回归支撑。
- Persisted Evidence 为 `none`。没有 evidence 目录，不据此提问题。

**Follow-up Decision**

A20–A28 已满足。上面 5 条都不需要当前 Cycle 修复，也不需要新的执行依据。Iteration Plan 不改。接受本 Iteration，并展开 Iteration 007。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/007-write-failure-state/000-initial.md`

