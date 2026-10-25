# Iteration 000 / Cycle 000: 重新打开时按持久目录重建空闲页集合

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `free-page-lifecycle` R2（重新打开时按持久目录重建空闲页集合）、R3（重建的失败安全）；`drop-table-physical-free` 的 MODIFIED 增量「跨重启 drop 安全」；MS33（`drop-persistence-boundary`）已验收的 DROP 崩溃语义（R2–R4）；MS34 的 `index-write-side-atomicity` / `ddl-catalog-transactional` / `statement-cancellation-outcome` 语义。
- Excluded scope: 释放的持久化判定与会话内复用时机（Iteration 001）；尾部截断与整库整理；无键行旧版本 GC（I038）；文件格式与伴生文件变更；ISS22 / ISS23 / ISS24 / ISS25 的修复；规格合并与全局状态同步（`openspec-docs-maintainer`）。

**Objective**

本 Cycle 完成后，重新打开一个数据库会依据**持久目录**重建本次打开的可分配页集合：持久目录引用不到的页成为可分配页（释放的空间跨重启可用），而仍被持久目录登记的页**绝不**进入可分配页集合；重建任一环节出错时整次放弃（本次打开无可分配页，既有表照常可读）。可观察结果：跨重启的「建表 → 删表 → 重开 → 建表」不再让库文件单调增长。

**Investigation Facts**

- Starting point:
  - 仓库 `HEAD` = `0a08b14`（ahead of origin 9）。**工作区存在既有未提交改动**（MS34 与 issue-fix change 的产品代码、测试与文档改动，9 个 `src/` 文件 + 3 个 `tests/` 文件 + `.agents/` 与 `openspec/specs/` 文档）：**不得回滚、不得清理、不得提交**。本 change 在此现场之上增量开发。
  - 当前实现：`src/storage/file_storage.rs:50` 的 `free_pages: Mutex<Vec<u64>>` 是**仅内存**空闲列表；`FileStorage::open` 的两条构造路径都把它初始化为空（`:197`、`:253`）⇒ 跨重启的空闲集合必然丢失。
  - 变更前验证（本次会话实际执行，2026-10-24）：`cargo test --test drop_table_free_test --test drop_persistence_test -- --test-threads=1` ⇒ `drop_persistence_test` **11 passed / 0 failed**；`drop_table_free_test` **6 passed / 0 failed**。这两组是本 Cycle 的回归基线（MS33 的崩溃语义基线 + 同会话复用的既有断言基线）。
  - 已知限制：跨重启的空闲集合无任何来源；`drop_table` 的释放只写内存列表（`src/storage/data/table_manager.rs:1389+`）。

- Current-State Evidence（Plan 已核对、与实施直接相关的事实）:
  - **空闲列表与分配路径**：`src/storage/file_storage.rs:435-458` 的 `allocate_page` 先弹 `free_pages`（弹出即写零页），空则扩展文件；`:490-498` 的 `free_page` 把页号直接推进 `free_pages`；`:512-514` 的 `page_count` 返回当前页数。⇒ 空闲页集合**只存在于本次打开**。
  - **存储接缝**：`src/storage/async_storage.rs:5-23` 的 `AsyncStorage` 只有 6 个必需方法 + 2 个带默认实现的方法（`page_size`、`page_count`）⇒ 新增接缝**可以带默认实现**，`MemoryStorage` 与测试替身（`src/storage/page_format/overflow.rs` 的 `FailAfterNAllocs`）无需改动。
  - **打开序列**：`src/database.rs` 的 `open_internal` 顺序为 storage（`:318-337`）→ buffer pool（`:338`）→ `TableManager::open_or_init`（`:342-343`，内部 `src/storage/data/table_manager.rs:269`）→ views（`:350-367`）→ WAL（`:371-386`）→ 恢复（`:393-428`，末步 `attach_index_catalog_contexts()` 在 `:427`）→ plan cache（`:431`）→ checkpoint manager（`:434-445`）。**重建必须挂在 `:427` 之后**（恢复会重建索引树并分配新页）；只读模式不做页分配，不需要重建。
  - **目录与目录行**：`src/storage/catalog.rs:87-111` 的 `CatalogRow` 携带 `data_page_head`、`index_root_page_id`、`unique_roots: Vec<u32>`、`secondary_indexes: Vec<CatalogSecondaryIndex>`（`:58-65`，含 `root: u32`）、`kind`（`:77-83`，`Table` / `View`）。目录链的页遍历模式见 `src/storage/catalog.rs:626-641`（沿 `next_page_id` 走到 0 结束）。目录行的扫描入口 `Catalog::scan_tables`（`:304`）。
  - **页所有者的穷举**（`allocate_page` 的全部调用点，已逐一核对）：`src/storage/catalog.rs:155`/`:162`（目录页 0/1）、`:682`（目录链续页）、`src/storage/data/table_manager.rs:451`（表数据页头）、`src/storage/data_page.rs:56`（数据链续页）、`src/storage/page_format/overflow.rs:155`（溢出链页）、`src/storage/btree/btree.rs:108`/`:214`/`:263`/`:344`（B-Tree 根与分裂页）⇒ **四类所有者**：目录链、表数据链（含溢出链）、三类索引树。视图不拥有页（`open_or_init` 跳过视图行，`src/storage/data/table_manager.rs:281-283`）。
  - **既有遍历原语**（与 `drop_table` 的释放路径同型，可直接复用）：`TableManager::collect_data_pages`（`src/storage/data/table_manager.rs:1472`）、`TableManager::collect_overflow_chain_pages`（`:1505`）、`IndexManager::collect_all_pages`（`src/storage/btree/index_manager.rs:612`）、`IndexManager::from_root`（`:196`）。
  - **恢复期换根**：`src/storage/data/table_manager.rs:305-314` 记录「恢复重建索引树时换根**不**持久化」，`src/database.rs:424-427` 在恢复后调用 `attach_index_catalog_contexts` ⇒ **内存索引树的根可能与持久目录登记的根不同**，两者的页都必须算活页。
  - **完整性检查入口**：`src/storage/integrity.rs:87` 的 `check_integrity(buffer_pool, table_manager)` 返回 `IntegrityReport`（`error_count()`、`info_count()`），只读。
  - **既有测试与观察方式**：`tests/drop_table_free_test.rs:19` 的本地 helper `fn page_count(path: &Path) -> u64`（按文件长度换算页数）是本仓既有的体积观察约定；`tests/drop_persistence_test.rs` 提供崩溃场景的既有写法（进程内丢弃句柄、不调用 `close`，配方见 `.agents/runbooks/in-process-crash-simulation.md` R49）。
  - **失败注入先例**：`src/database.rs:870` 的 `Database::arm_io_failure(IoFailPoint)`（`src/storage/file_storage.rs` 的 `IoFailPoint` / `consume_io_failpoint`）可制造数据文件写失败。

- Code and Critical Path:

```text
Database::open_internal (src/database.rs:318-445)
  └─ FileStorage::open            (src/storage/file_storage.rs:130/...)   ← free_pages 初始化为空（:197/:253）
  └─ BufferPool::new              (src/storage/buffer_pool.rs)
  └─ TableManager::open_or_init   (src/storage/data/table_manager.rs:269)  ← 扫描持久目录
  └─ [读写模式] 恢复 + attach_index_catalog_contexts (src/database.rs:393-427)
        └─ ★ 本 Cycle 的重建钩子：在此之后计算活页集合并注入空闲集合

页释放（Iteration 001 涉及，本 Cycle 只读不改）
  TableManager::drop_table (src/storage/data/table_manager.rs:1389+)
    ├─ catalog.delete_table        ← 锚点写（目录删除）
    └─ storage.free_page(...)      ← src/storage/file_storage.rs:490

页分配
  FileStorage::allocate_page (src/storage/file_storage.rs:435-458)
    └─ free_pages.pop() → 写零页
```

- 动态边与状态变化:
  - 空闲集合是**存储层**状态（`FileStorage`），而「谁拥有哪一页」的知识在**目录层**（`TableManager` / `Catalog`）⇒ 重建逻辑必须放在能同时看到两者的层（`TableManager` 或其同层新模块），注入动作通过存储接缝完成。
  - 重建是**只读**的：读页、遍历、算差集；唯一的写动作是「把算出的空闲页集合交给存储层」，不写任何数据文件字节 ⇒ 不影响只读语义与加密语义。
  - 恢复流程可能在本 Cycle 的钩子之前分配新页（重建索引树）⇒ 空闲集合的上界（页数快照）必须在遍历**之前**取，且必须把内存索引树的页并入活页集合。

**Implementation Guidance**

- 建议顺序：T1（先行 RED 用例 + 观察能力）→ T2（接缝与打开钩子，RED 仍红）→ T3（重建实现，GREEN）。T2 单独不会让用例转绿，这是刻意的分层：T2 提供可验证的接缝（能注入空闲集合、能被调用），T3 提供正确的集合内容。
- 关键取舍：
  1. **接缝用带默认实现的 trait 方法**，不改 `MemoryStorage` 与测试替身的必实现集合（`src/storage/async_storage.rs` 的既有先例：`page_size` / `page_count` 就是默认实现）。
  2. **失败安全优先于复用收益**：任一遍历出错 ⇒ 整次重建放弃（不注入任何页），并在可观察位置留下告警（沿用仓内既有的告警/日志风格）。绝不允许「部分注入」。
  3. **页数上界快照在遍历之前取**，否则会把本会话新分配的页判成空闲。
  4. **活页集合取并集**：持久目录登记的树页 ∪ 内存索引管理器的树页（恢复期换根）。`IndexManager::collect_all_pages_tolerant`（`src/storage/btree/index_manager.rs:676`）在读失败时返回部分结果——**本 Cycle 的重建不得使用 tolerant 变体**（部分结果会被误判成空闲），必须用 `collect_all_pages`（`:612`）并在出错时整次放弃。
  5. **不新增任何持久结构**：不改文件头、不加伴生文件、不改格式版本（否则牵动 MS36）。
- 与既有约定的复用：体积观察沿用 `tests/drop_table_free_test.rs:19` 的本地 helper 写法；完整性断言沿用 `check_integrity` 的 `error_count()`；崩溃场景沿用 R49 的进程内丢弃句柄写法。

**Task Definitions**

### T1: 先行 RED 用例——跨重启复用与「重建不把活页判成空闲页」

- Requirement/Scenario: `free-page-lifecycle` R2（场景：重开后释放的页可再次分配；场景：重建不把活页判成空闲页）、R3（场景：遍历失败时打开仍成功且不误判）；`drop-table-physical-free` MODIFIED「跨重启 drop 安全」。
- Depends on: None
- Targets: `tests/free_page_lifecycle_test.rs`（新增）
- Current behavior: 跨重启后释放的页不可复用——重开后新建表会让库文件体积继续增长（空闲列表在 `FileStorage::open` 时为空，`src/storage/file_storage.rs:197`/`:253`）。
- Required behavior: 用例覆盖三条可观察结果：(a) 会话一建表 `t`、写入、删除并完成一次 checkpoint 后关闭，会话二重开并建表 `u` ⇒ `u` 占用 `t` 释放的页号，**库文件体积不因 `u` 而增长**；(b) 重开后新建表**不得**使任何活表的已提交行丢失或不可读，且 `check_integrity(...).error_count() == 0`；(c) 重建遍历出错时打开仍成功、活表仍可读、且本次打开不产生任何可分配页（不误判活页）。
- Preserve: 既有用例 `tests/drop_table_free_test.rs` 的 6 个用例与 `tests/drop_persistence_test.rs` 的 11 个用例在 T1 完成后**仍然全绿**（T1 只新增用例，不改既有行为）。
- Forbidden: 不改产品代码（本任务只写用例）；不改 `openspec/specs/`；不引入判定脚本、封装命令或专用参数；不引入身份证明机制；不建新持久文件；不用 timer 阈值作行为判据（需要等待时用可观察状态 + `yield_now` 轮询，timer 只作夹具失控兜底）；不使用会触发 ISS23 的表规模（行数 ≪ 300）。
- First test: 位置 `tests/free_page_lifecycle_test.rs`（新增，RED）。命令 `cargo test --test free_page_lifecycle_test -- --test-threads=1`。预期 RED：跨重启复用用例失败（重开后新建表使库文件体积增长），其余两条用例按预期通过或以明确断言失败区分「重建尚未实现」与「实现错误」。
- GREEN condition: 本任务完成时用例**已写好且为 RED**（失败原因指向「跨重启不复用」），且既有 17 个用例仍全绿。T1 的 GREEN 是「测试存在并如实失败」。
- Verification: `cargo test --test free_page_lifecycle_test -- --test-threads=1`（观察 RED 的失败信息指向体积增长断言）；`cargo test --test drop_table_free_test --test drop_persistence_test -- --test-threads=1`（观察 11 + 6 全绿，与变更前基线一致）。
- Stop when: 用例无法在不触发 ISS23（表规模）的条件下表达跨重启复用，或需要改动产品代码才能观察体积 → 返回 Plan。

### T2: 存储层接缝与打开钩子——可注入的空闲集合 + 恢复之后的重建调用点

- Requirement/Scenario: `free-page-lifecycle` R2（机制前提）、R3（失败安全的语义承载）。
- Depends on: T1
- Targets: `src/storage/async_storage.rs`（`AsyncStorage` 的带默认实现的接缝方法）、`src/storage/file_storage.rs`（`FileStorage` 的空闲集合注入实现）、`src/database.rs`（`open_internal` 的读写模式恢复之后，`:427` 附近的重建调用点）、`src/storage/data/table_manager.rs`（重建入口的承载位置）
- Current behavior: 存储层没有「外部注入空闲页集合」的入口；`open_internal` 在恢复之后没有任何空闲集合重建动作（空闲集合恒为空）。
- Required behavior: (a) `AsyncStorage` 新增**带默认实现**的空闲集合注入方法，使 `MemoryStorage` 与测试替身不需要改动；(b) `FileStorage` 接受注入并把注入的页纳入既有可分配集合（与 `allocate_page` 的弹出路径一致）；(c) `open_internal` 在**读写模式**下、恢复与 `attach_index_catalog_contexts` 之后调用重建入口；只读模式与内存模式不调用；(d) 重建入口在「遍历尚未实现」时**不注入任何页**（安全默认值＝本次打开无可分配页），不产生错误、不影响打开成功。
- Preserve: `AsyncStorage` 的必实现方法集合不变（`MemoryStorage`、`src/storage/page_format/overflow.rs` 的 `FailAfterNAllocs` 等测试替身零改动）；只读打开路径仍然零写入；内存模式行为不变；`allocate_page` 的「弹出即写零页」语义不变。
- Forbidden: 不改 `free_page` 的语义（释放的持久化判定属 Iteration 001）；不改文件头与格式版本；不加伴生文件；不改 `check_integrity` 的语义；不引入身份证明机制；不新增判定层。
- First test: 沿用 T1 的用例（RED 保持），并补一条接缝级断言：注入 N 个页后 `allocate_page` 连续弹出的是这 N 个页（该断言在 T3 之前即成立，可先写成 RED 的一部分）。
- GREEN condition: 接缝与调用点就位；`cargo test --test free_page_lifecycle_test -- --test-threads=1` 仍为 RED（重建内容尚未实现）**且失败原因不变**（仍是「体积增长」而不是编译错误或 panic）；既有 17 个用例仍全绿。
- Verification: `cargo test --test free_page_lifecycle_test -- --test-threads=1`（观察失败原因未变）；`cargo test --test drop_table_free_test --test drop_persistence_test -- --test-threads=1`（全绿）；`cargo build --lib --bins`（无编译错误）。
- Stop when: 接缝必须改动 `MemoryStorage` 或测试替身的必实现集合，或只能在 `open_internal` 之外的位置注入（会破坏只读/内存语义）→ 返回 Plan。

### T3: 活页集合重建实现——按持久目录算出可分配页集合

- Requirement/Scenario: `free-page-lifecycle` R2（全部三个场景）、R3（场景：遍历失败时打开仍成功且不误判）。
- Depends on: T2
- Targets: `src/storage/data/table_manager.rs`（活页集合计算与注入）、`src/storage/catalog.rs`（目录链页遍历所需的链信息）、`src/storage/btree/index_manager.rs`（索引树页遍历的既有原语的使用）
- Current behavior: 打开后没有任何空闲页集合重建；跨重启的释放空间永久丢失。
- Required behavior: 在 T2 的调用点上完成重建并满足：
  1. **页数上界**：先取 `storage.page_count()` 快照作为空闲集合的上界（`[0, snapshot)`），再遍历；
  2. **活页集合**覆盖：目录链（`__tables` 页 0 与 `__columns` 页 1，沿 `next_page_id` 走到 0）、每个 `kind == Table` 的目录行所指的表数据链（从 `data_page_head`）、这些数据页槽内 locator 指向的溢出链页、该表的 PK 树（`index_root_page_id`）、唯一索引树（`unique_roots`）、二级索引树（`secondary_indexes[].root`），**并**该表在内存中的 `IndexManager::collect_all_pages` 结果（恢复期换根 ⇒ 两组页都算活页）；
  3. **空闲集合** = 上界减去活页集合；视图行（`kind == View`）不拥有页；
  4. **失败安全**：任一次遍历返回错误 ⇒ 整次重建放弃（不注入任何页），打开仍成功；
  5. 重建是只读的：不写任何数据文件字节。
- Preserve: `drop_table` / `truncate_table` / `DROP INDEX` 的既有释放路径语义不变（本 Cycle 不改释放时机）；`check_integrity` 语义不变；只读打开零写入；目录行的序列化格式不变（不新增目录字段）。
- Forbidden: 不用「扫描页内容猜测归属」代替可达性遍历；不使用 `collect_all_pages_tolerant` 的部分结果做活页判定；不改文件格式；不加持久结构；不改 MS33 / MS34 的语义；不把活页判成空闲页（哪怕只在一部分页上）。
- First test: 沿用 T1 的用例（`reopened_database_reuses_released_pages`、`rebuild_failure_keeps_live_pages_intact` 等），本任务使其转 GREEN。
- GREEN condition: `cargo test --test free_page_lifecycle_test -- --test-threads=1` 全绿；既有 17 个用例全绿；活页误判为零——由「重建出的空闲集合恰好等于被释放的页集合」这一断言见证（重建后新建表不得使任何活表行丢失、`check_integrity` 的 `error_count() == 0`）。
- Verification: `cargo test --test free_page_lifecycle_test -- --test-threads=1`（全绿）；`cargo test --test drop_table_free_test --test drop_persistence_test -- --test-threads=1`（11 + 6 全绿）；`cargo test --test overflow_page_test -- --test-threads=1`（TRUNCATE 路径未受影响）。
- Stop when: 活页集合的枚举无法在不新增持久结构、不改目录格式的前提下覆盖某类页所有者，或出现「无法判定某页是否活页」的页类 → 返回 Plan（不得以猜测兜底）。

**Invariants**

- **N1（本 change 的核心）**：一个页成为可分配页，当且仅当它的释放在崩溃边界上已经成立（由 Iteration 001 完整实现；本 Cycle 只实现「持久空闲页」这一来源）。
- **N2**：可分配页集合中的页，持久目录一定不引用它 ⇒ 交予新对象时不会摧毁任何持久对象的页内容。
- 释放不写零页；清零只在分配时发生（沿用 MS33 的既有约定）。
- MS33 已验收语义不变：checkpoint 前崩溃保留 DROP 前的已提交状态；同会话再分配不读到已释放页的旧行；`check_integrity` 的 error 级发现数为 0。
- MS34 的 `index-write-side-atomicity` / `ddl-catalog-transactional` / `statement-cancellation-outcome` 语义不变。
- 只读打开零写入；内存模式（`:memory:`）行为不变；加密库的读写语义不变（重建只读既有页）。
- 不引入身份证明机制（Hash/指纹、revision pin、run-id、artifact manifest、日志 Hash 链、时间顺序证明等）与其采集/审计工具；判定逻辑全部落在 `cargo test` 集合内；timer 只作夹具失控兜底。
- 不改 `openspec/specs/`（规格合并由 `openspec-docs-maintainer` 在收尾执行）；不同步全局 tasks / SNAPSHOT / M/R/I；不登记或修改 Issue。

**Non-goals**

- 释放的持久化判定（待释放集合、释放完成编号、checkpoint 钩子）与会话内复用时机的改变——Iteration 001。
- 尾部截断、整库整理、VACUUM 式空间归并（MS35 拆分信号 / Non-goal）。
- 无键行旧版本 GC（I038，MS35 Non-goal）。
- 文件格式版本、伴生文件、加密语义、CLI 文件族（`src/cli/lifecycle.rs` 的删除与列举）——MS36 的兼容面。
- ISS22 / ISS23 / ISS24 / ISS25 的修复（相邻 open Issue，只作约束）。
- 性能优化（打开时的遍历成本本 Cycle 只保证正确，不做缓存或增量优化）。

**Acceptance**

| ID | 可观察验收条件 | 映射 |
|---|---|---|
| A1 | 会话一「建表 `t` → 写入 → 删表 → checkpoint → 关闭」，会话二重开并建表 `u`：`u` 占用 `t` 释放的页号，库文件体积不因 `u` 增长 | FPL R2 场景 1；DTPF「跨重启 drop 安全」MODIFIED |
| A2 | 重开后新建表不影响任何活表：活表已提交行仍可查询，`check_integrity(...).error_count() == 0` | FPL R2 场景 2 |
| A3 | 恢复期重建过索引树的库，重开后内存索引树页与持久目录登记的树页都不进入可分配页集合 | FPL R2 场景 3 |
| A4 | 重建遍历出错时：打开成功、活表可读、错误计数为 0、本次打开无可分配页（不误判活页、不崩溃） | FPL R3 场景 1 |
| A5 | 既有 17 个基线用例（`drop_persistence_test` 11 + `drop_table_free_test` 6）仍全绿；`overflow_page_test` 全绿 | Invariants；MS33 语义保持 |
| A6 | 变更后行为与规格增量一致：跨重启复用成立；本 Cycle 不改变会话内释放时机（`drop_table_free_test` 的同会话复用断言原样通过） | FPL R2；DTPF MODIFIED 增量 |

**Verification**

| Scenario / Acceptance | 判定方式 | 命令 | 通过依据 | 失败含义 |
|---|---|---|---|---|
| A1（FPL R2 场景 1） | 真实入口端到端（真实文件库 + 真实 open/close） | `cargo test --test free_page_lifecycle_test -- --test-threads=1` | 用例内断言：重开后新建表后 `page_count`（按 `tests/drop_table_free_test.rs:19` 的本地 helper 读文件长度换算）不增长，且新表页号 ∈ 被释放页集合 | 重建未生效或把活页判成空闲 |
| A2（FPL R2 场景 2） | 真实入口端到端 | 同上 | 活表行可查询 + `check_integrity(...).error_count() == 0` | 活页误判或遍历不完整 |
| A3（FPL R2 场景 3） | 真实入口端到端 | 同上 | 内存索引树页与持久目录树页都不在可分配集合（新表不占用它们，且两棵树仍可查） | 未取并集 |
| A4（FPL R3 场景 1） | 真实入口端到端（故障注入或损坏夹具） | 同上 | 打开成功 + 活表可读 + `error_count() == 0` + 本次无可分配页 | 未失败安全（可能误判活页） |
| A5 | 定向回归 | `cargo test --test drop_persistence_test --test drop_table_free_test --test overflow_page_test -- --test-threads=1` | 全部 0 failed | 破坏了 MS33 语义或释放路径 |
| A6 | 定向回归 | `cargo test --test drop_table_free_test -- --test-threads=1` | 6 个用例 0 failed（会话内释放时机本 Cycle 未变） | 越界改了释放语义 |

全量回归**不在本 Cycle**（收尾验证记在 Iteration 001，按 M19 分批串行执行）。

**Gate 2 Readiness**

Gate 2 通过：需求与 BDD 已成型（`proposal.md` 的 What Changes + `specs/free-page-lifecycle/spec.md` 的四组 requirement/scenario + 两处 MODIFIED 增量）；实现调查已完成（页所有者穷举、打开序列、既有遍历原语、失败注入与崩溃配方、测试观察约定均已核对并落到 Task Definitions）；任务已分轮（Iteration 000 = T1–T3，Iteration 001 = T4–T7）；RTM 已建立（`tasks.md`）；当前轮验证就绪（每个 Task Definition 带 First test 与 GREEN condition）。**无 BLOCKED 项**；**无 SKIPPED 项**。用户批准依据：goal 指令原文（记于 `proposal.md` 的「授权与批准依据」），Gate 1 与 Gate 2 的「用户批准」检查点视为已批准，检查标准未降低。

**Persisted Evidence**

- Mode: none

理由：本 Cycle 的验收结论完全由 `cargo test` 的真实运行结果承载（每个用例的断言直接对应 A1–A6，且可在低成本下重跑）；Act Response 足以记录命令、实际结果与结论；无需保存完整日志目录、源码副本或测试套件输出，也不需要任何采集/审计工具（若为 Evidence 新增机制将违反禁止身份证明机制的约束）。实现过程中若出现「一次性环境即将消失」或「Issue/Blocker 需要保留现场」的情形，由 Act 按公共规则单独创建 `evidence/<iteration>/<cycle>/`。

**Risks and Notes**

- **风险 R1（最高）：活页枚举不完整会把活页判成空闲页（静默损坏）。** 对策已内置：所有者集合由 `allocate_page` 的全部调用点穷举确定；遍历原语与 `drop_table` 的释放路径同型（被释放的页正是重建不判空闲的页）；失败安全（出错即整次放弃）；验收用例断言「重建出的空闲集合恰好等于被释放的页集合」且活表零丢失。
- **风险 R2：恢复期换根导致内存树与持久目录树不同。** 对策：活页集合取并集（A3 用例钉住）。
- **风险 R3：页数上界取晚会把本会话新分配的页判成空闲。** 对策：先取快照再遍历（T3 的 Required behavior 1）。
- **风险 R4：ISS23（BTree 插入无限递归 → 栈溢出，open）。** 本 change 的验证场景必须用小表（行数 ≪ 300）；需要多页时用行宽/页容量控制而非行数。**不得**在本 change 内修它（由后续承载 change 处理）。
- **风险 R5：相邻 open Issue（ISS22 数据页写满后丢行、ISS24 被放弃的 DDL 隐藏改写行、ISS25 提交点窗口位置性错读）与本 change 的存储层相邻。** 只作约束记录，不在本 change 内修改，不作为本 Cycle 的 Acceptance。
- **非实质未知项（留给 Act 的实现选择，不阻塞）**：活页集合计算的具体承载位置（新模块 vs `TableManager` 方法）、目录链页遍历是复用 `Catalog` 内部链信息还是由 `TableManager` 自行沿 `next_page_id` 遍历、告警的输出形式。
- **Issue 候选**：本 Cycle 调查未发现 change 范围外的实质缺陷（页所有者穷举与既有遍历原语均已核对）。若 Act 在实施中发现候选，按公共规则只在 Act Response 报告，不登记 Issue。
- **本 Cycle 不跑全量回归**：全量是收尾验证，预算记在 Iteration 001（全 change 一次）。

## Act Response

- Status: reported
- Cycle: `iterations/000-reopen-free-page-reconstruction/000-initial.md`（initial）
- Iteration: 000 `000-reopen-free-page-reconstruction`
- Tasks 完成: T1、T2、T3（全部 `done`；change `tasks.md` 已同步）

### 实际改动（文件与符号）

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/storage/async_storage.rs` | `AsyncStorage::inject_free_pages(&self, Vec<u64>)` | 新增**带默认实现**的接缝方法（默认空操作 ⇒ `MemoryStorage` 与测试替身零改动，必实现方法集合不变）。语义：注入集合**取代**本存储当前的可分配集合（重建语义，见 Deviations D1） |
| `src/storage/file_storage.rs` | `FileStorage::inject_free_pages` | 实现接缝：清空并装入重建出的空闲集合 |
| `src/storage/catalog.rs` | `Catalog::collect_chain_pages(root) -> Result<Vec<PageId>>` | 目录链页遍历（沿 `next_page_id` 走到 0），读失败即报错（不做部分结果） |
| `src/storage/data/table_manager.rs` | `TableManager::persisted_catalog_rows`（新字段）+ `open_or_init` | 捕获**打开时**（恢复之前）的持久目录行快照 |
| `src/storage/data/table_manager.rs` | `TableManager::rebuild_free_page_set(db_path) -> Result<()>` | 重建入口：先取页数上界快照 → 算活页集合 → 差集得空闲集合 → 经接缝注入 |
| `src/storage/data/table_manager.rs` | `collect_live_pages` / `collect_data_chain_strict` / `collect_overflow_chain_pages_strict` / `collect_tree_pages` | 活页集合遍历（目录链 + 数据链 + 溢出链 + 目录登记的三类索引树 + 内存索引树并集），严格模式：任一出错即整次放弃 |
| `src/storage/free_page_rebuild.rs`（新增） | `arm_free_page_rebuild_fault(path)` / `take_free_page_rebuild_fault(path)` | R3 的受控页读失败注入点（一次性、按库路径限定 ⇒ 并行测试互不干扰） |
| `src/storage/integrity.rs` | `check_data_page` / `check_tree_page` 及其结果结构 | 可见性改为 `pub(crate)`（**仅可见性**，`check_integrity` 语义不变）：重建复用其有界读取器，避免损坏页触发 panic |
| `src/storage/mod.rs` / `src/storage/data/mod.rs` | 模块声明与再导出 | 新模块挂载与导出 |
| `src/database.rs` | `open_internal`（读写模式，`attach_index_catalog_contexts()` 之后） | 重建调用点：出错只告警（`[free-page-rebuild] rebuild abandoned: …`），打开仍成功；只读模式与内存模式不调用 |
| `tests/free_page_lifecycle_test.rs`（新增） | 8 个用例 | A1–A4 + 接缝断言 + 既有语义保持（只读零写入、内存库、同会话释放时机） |

### 先行测试的 RED / GREEN 证据

**T1（先行 RED，用例先于实现）**

命令：`cargo test --test free_page_lifecycle_test -- --test-threads=1`
结果：`test result: FAILED. 4 passed; 2 failed; 0 ignored` —— 两个失败点都指向「重建尚未实现」：

- `reopened_database_reuses_released_pages`：
  `assertion 'left == right' failed: the reopened database must allocate 'u' from the released pages instead of growing the file / left: 6 / right: 4`
  ⇒ 失败原因落在**体积增长断言**上：跨重启后新建表让库文件从 4 页长到 6 页（释放空间未复用）。
- `recovery_rebuilt_index_pages_are_not_reallocated`：
  `assertion 'left != right' failed: the new table must not take the persisted tree root page / left: 3 / right: 3`
  ⇒ 恢复换根后，**持久目录登记的旧树页**被交给了新表（活页误判为可分配）。
- 同批次的 A2（活表不被判空闲）与「只读零写入 / 内存库 / 同会话释放」用例在 RED 阶段即通过（当时无重建，断言平凡成立）——它们在实现后成为回归护栏。

**A2 的用例强化与判别力检查**：初版 A2 的库里没有已释放的表 ⇒ 可分配集合为空 ⇒「活页被排除」平凡成立。强化后 A2 先建一张随后删除的表 `gone`（其页在重开后构成可分配集合），再建带溢出链的活表 `live` ⇒ 若活页被误判为空闲，新表就会占用活表的数据页 / 索引树 / 溢出链页。判别力用一次变异检查确认（临时不把数据链页计入活页集合，随后还原）：

命令：`cargo test --test free_page_lifecycle_test rebuild_never_marks_live_pages_free -- --test-threads=1`
结果：`test result: FAILED. 0 passed; 1 failed` —— `assertion 'left != right' failed: the new table must not take the live table's data page / left: 2 / right: 2`；还原后同命令 GREEN。

**T2（接缝 + 调用点就位，RED 保持）**：接缝级断言 `injected_free_pages_become_the_allocatable_set` 随接缝一同加入（该用例在接缝存在前无法编译）；A1 的失败原因不变（仍是体积增长，不是编译错误或 panic）。

**T3（重建实现 → GREEN）**

命令：`cargo test --test free_page_lifecycle_test -- --test-threads=1`
结果：`test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`（8 个用例：A1、A2、A3、A4、接缝断言、同会话释放、只读零写入、内存库）。

**A4（失败安全）的 RED 说明**：该用例在受控注入点就位后加入，其 RED 通过**临时关闭**重建对注入点的消费取得（`if false && take_free_page_rebuild_fault(…)`，随即还原）：

命令：`cargo test --test free_page_lifecycle_test rebuild_failure_keeps_live_pages_intact -- --test-threads=1`
结果：`test result: FAILED. 0 passed; 1 failed` —— `panicked at tests/free_page_lifecycle_test.rs:308:5: an abandoned rebuild must not reuse any released space`（重建未放弃 → 释放空间被复用）。恢复消费点后同命令 GREEN（`ok. 8 passed`）。

### 重建逻辑的覆盖面证据

页所有者（`allocate_page` 的全部调用点，Plan 已穷举）→ 活页集合的覆盖：

| 页所有者 | `allocate_page` 调用点 | 重建中的覆盖路径 |
|---|---|---|
| 目录页 0 / 1 | `catalog.rs:155` / `:162` | `Catalog::collect_chain_pages(PageId(TABLES_PAGE_ID))` / `PageId(COLUMNS_PAGE_ID)` ✓ |
| 目录链续页 | `catalog.rs:682` | 同上（沿 `next_page_id` 走到 0；链只增不减，当前链覆盖持久链）✓ |
| 表数据页头 | `table_manager.rs:451` | `collect_data_chain_strict(row.data_page_head)` ✓ |
| 数据链续页 | `data_page.rs:56` | 同上 ✓ |
| 溢出链页 | `overflow.rs:155` | 数据页槽 locator → `collect_overflow_chain_pages_strict` → `overflow::collect_overflow_pages` ✓ |
| B 树根 / 分裂页（PK / 唯一 / 二级） | `btree.rs:108` / `:214` / `:263` / `:344` | `collect_tree_pages`：目录登记根（`index_root_page_id` / `unique_roots` / `secondary_indexes[].root`）∪ 内存索引管理器根 ✓ |

释放点 → 「持久目录不再登记 ⇒ 判空闲」：

- `DROP TABLE`（`table_manager.rs:1389+`）：目录行删除 ⇒ 页不再被引用 ⇒ 跨重启后可分配（A1 用例见证）。
- `TRUNCATE`（`:1590+`）：表头重置 + 尾指针/根更新 ⇒ 旧尾页与旧树不再被引用。
- `DROP INDEX`（`executor/drop_index.rs:62-63`）：目录 secondary 段更新 ⇒ 旧树不再被引用。
- GC 清理旧版本（`table_manager.rs:170` → `data_page.rs:249`）：释放的是溢出链页（槽删除后 locator 消失）⇒ 不再被引用。
- `CREATE INDEX` 回填失败清理（`executor/create_index.rs:83`）与溢出链写入失败清理（`page_format/overflow.rs:140`、`data_page.rs:81`）：分配的页未落 locator ⇒ 不被任何链引用。
- **恢复期旧索引树释放**（`wal/recovery.rs` 第 5 步）：持久目录仍登记旧根 ⇒ 重建判为**活页**，不进入可分配集合（A3 用例钉住）。

被证明不在范围内的：视图行（`kind == View`，`open_or_init` 跳过，不拥有页 ✓）；尾部截断 / 整库整理（Non-goal）；无键行旧版本 GC（I038）；文件格式与伴生文件（MS36）；ISS22–ISS25。

### 「checkpoint 后才可分配」这条不变量的落实与验证

N1 的完整形态（释放完成 → 在释放完成之后开始且成功完成的 checkpoint → 才可分配）属 Iteration 001（T4/T5）。本 Cycle 落实该不变量的**跨重启来源**：可分配集合只由**持久目录**推导——释放在崩溃边界上成立的判据即「持久目录不再登记该页」：

- 释放已被一次成功完成的 checkpoint 持久化 ⇒ 持久目录不再登记 ⇒ 该页成为可分配页（A1：`reopened_database_reuses_released_pages` 见证体积不增长且 `u` 占用被释放的页号）。
- 释放未被持久化（checkpoint 前崩溃）⇒ 恢复回滚该释放，持久目录仍登记 ⇒ 该页**不**进入可分配集合（`tests/drop_persistence_test.rs` 11 例全绿，其中 `drop_table_crash_before_checkpoint_keeps_rows` 见证旧表行完整、`check_integrity` error 数为 0）。
- 重建遇错 ⇒ 整次放弃 ⇒ 本次打开无可分配页（R3，A4 用例见证）。

会话内释放时机本 Cycle **不改**（同会话立即复用仍是既有语义，`drop_table_free_test` 6 例原样通过）——该分界属 Iteration 001（T5/T6）。

### 既有断言的期望更新（逐处）

本 Cycle **未改动任何既有断言**：`tests/drop_table_free_test.rs` 的 5 处、`tests/overflow_page_test.rs` 的 1 处、lib 单测 `free_overflow_chain_returns_pages_for_reuse` 的期望全部原样成立。依据：本 Cycle 不改变会话内释放时机（Plan 的 A6 与 `tasks.md` T6 明确把该分界更新划给 Iteration 001；T6 状态仍为 `pending`）。证据：

- `cargo test --test drop_table_free_test -- --test-threads=1` → `6 passed; 0 failed`
- `cargo test --test overflow_page_test -- --test-threads=1` → `27 passed; 0 failed`
- `cargo test --lib free_overflow_chain_returns_pages_for_reuse -- --test-threads=1` → `1 passed; 0 failed`
- `cargo test --lib --bins` → `420 passed; 0 failed`

（若在后续 Cycle 实施 T6，届时逐处记录「旧期望 → 新期望 → 依据哪条 requirement」。）

### Gate 3 / Gate 4 结论

**Gate 3（Task Review）— 通过**，逐 task：

- T1：spec compliance——用例覆盖 R2 场景 1/2/3 与 R3 场景 1，全部走真实入口（真实文件库 + 真实 `Database::open`/`close`），体积断言沿用既有 `page_count` helper；无判定脚本、无专用参数、无身份证明机制、无 timer 阈值；表规模为小表（未触发 ISS23）。
- T2：`AsyncStorage` 必实现方法集合未变（`MemoryStorage`、`FailAfterNAllocs` 零改动 ✓）；只读打开零写入 ✓（用例断言文件长度不变）；内存模式不调用重建 ✓；`allocate_page` 的「弹出即写零页」语义未变 ✓；`free_page` 语义未改 ✓。
- T3：活页集合覆盖 Plan 穷举的全部页所有者（见上表）；页数上界在遍历**之前**取 ✓；失败安全（任一出错 ⇒ 整次放弃、不注入任何页、打开仍成功）✓；重建只读既有页、不写任何数据文件字节 ✓；未用 tolerant 变体做活页判定 ✓；目录行序列化格式未改 ✓。
- Code quality：diff 无计划外修改（仅本 change 的 6 个产品文件 + 1 个新模块 + 1 个新测试文件；其余工作区改动为既有现场，未触碰）；无新增编译警告；错误/边界/资源生命周期正确（损坏页 ⇒ 遍历错误 ⇒ 放弃，不 panic）；测试不会因错误原因通过（RED 证据已记录）。

**Gate 4（Verified Completion）— 通过**，Task Definition 的验证全部满足：

| 验证 | 命令 | 实际结果 |
|---|---|---|
| A1–A4 定向 | `cargo test --test free_page_lifecycle_test -- --test-threads=1` | `8 passed; 0 failed` |
| A5 基线 | `cargo test --test drop_persistence_test --test drop_table_free_test --test overflow_page_test -- --test-threads=1` | `11 + 6 + 27 passed; 0 failed` |
| A6 会话内释放时机 | `cargo test --test drop_table_free_test -- --test-threads=1` | `6 passed; 0 failed` |
| 构建 | `cargo build --lib --bins` | 成功，无警告（仅 `.cargo/config` 弃用提示，既有） |
| 单元测试 | `cargo test --lib --bins` | `420 passed; 0 failed` |

**受影响范围的定向审计**（本 Cycle 不跑全量；全量是本 change 收尾验证，记在 Iteration 001 的 T7）——按 M19 分批纪律（每批一次独立调用、批间顺序执行、`--test-threads=1`、记各批 passed/failed）执行 3 批：

- 批 A（打开/恢复/DDL/完整性面，12 个目标）：`ddl_crash_recovery_test` 6、`ddl_catalog_transactional_test` 3、`recovery_test` 3、`recovery_e2e_test` 6、`recovery_replay_convergence_test` 3、`recovery_tail_persistence_test` 4、`check_command_test` 11、`schema_persistence_test` 8、`catalog_durability_test` 4、`truncate_test` 7、`gc_test` 3、`index_write_atomicity_test` 5 ⇒ 共 63 passed / 0 failed。
- 批 B（存储/加密/锁/WAL 面，12 个目标）：`checkpoint_test` 2、`database_file_lock_test` 8、`encryption_test` 7、`file_header_test` 14、`file_storage_io_test` 4、`index_manager_test` 3、`recovery_error_surface_test` 3、`recovery_wal_convergence_test` 5、`secondary_index_test` 27、`storage_test` 21、`update_index_maintenance_test` 11、`wal_recovery_large_test` 7 ⇒ 共 112 passed / 0 failed。
- 批 C（DDL/索引/DML 面，12 个目标）：`alter_table_test` 16、`btree_split_test` 12、`btree_test` 10、`ctas_test` 17、`e2e_test` 27、`executor_test` 39、`full_table_dml_test` 11、`import_input_surface_test` 12、`keyless_eq_routing_test` 14、`keyless_row_test` 4、`secondary_index_equality_test` 3、`view_test` 19 ⇒ 共 184 passed / 0 failed。

覆盖对账：定向批次合计 `52 + 63 + 112 + 184 = 411` 个集成用例 + `420` 个 lib/bins 用例，全部 0 failed；尚未覆盖的测试文件属 Iteration 001 的 T7 全量批次（预算未提前动用）。

### 自检发现、已修复内容与遗留 Minor 问题

**已修复（Critical/Important，均在本 Cycle 范围内）**

1. **损坏页导致打开 panic（Important，已修复）**：首版实现用生产页访问器（`SlottedPageRef::get_slot_data` 的无检查切片 + `IndexManager::collect_all_pages`）扫描页镜像，损坏页会**越界切片 panic**；实测既有 lib 用例 `storage::integrity::tests::slot_area_corruption_detected` 触发 `panicked at src/storage/page_format/slotted_page.rs:128` 并 `thread caused non-unwinding panic. aborting.`（SIGABRT，进程 abort）。修复：重建改用 `integrity::check_data_page` / `check_tree_page`（有界读取器）——损坏页变成遍历错误 ⇒ 整次放弃（R3）⇒ 打开仍成功。修复后 `cargo test --lib --bins` 回到 `420 passed; 0 failed`。
2. **恢复换根后持久目录登记的旧树页被交给新表（Important，已修复）**：T1 的 A3 RED 证据（`left: 3, right: 3`）显示该页在重建后仍可分配；修复路径为「接缝语义取重建（取代）语义」+「活页集合并入打开时的持久目录快照」，A3 转 GREEN，且 `check_integrity` 保持 0 error。

**遗留 Minor（记录，不在本 Cycle 扩展）**

1. 打开成本：每次读写打开都会遍历所有权图（O(活页数)）并物化空闲集合（O(空闲页数) 内存）。Plan 已把性能优化列为 Non-goal。
2. 放弃重建时以 `eprintln!` 告警（沿用仓内既有告警风格，无日志框架可用）。
3. 接缝的「取代」语义意味着打开期被释放但仍被持久目录登记的页（如恢复换根释放的旧树页）本次打开不可复用——这是安全性取舍（持久目录仍登记 ⇒ 不得复用），空间在下次 checkpoint + 重启后回收。

### Deviations from Plan

- **D1（接缝语义）**：Plan 的 T2(b) 写「把注入的页纳入**既有**可分配集合」，实现取**重建（取代）**语义（注入集合成为本次打开的可分配集合）。原因：R2 的原文是「**重建**本次打开的可分配页集合」；A3 要求「持久目录登记的树页不进入可分配集合」，而恢复换根释放的旧树页此时仍在存储层空闲列表里（`wal/recovery.rs` 第 5 步释放），「叠加」语义无法满足 A3（RED 证据：新表拿到了持久目录登记的旧树根页）。取代语义同时关闭了「同会话把旧树页交给新表」的既有隐患。
- **D2（索引树遍历原语）**：Plan 的 T3 建议复用 `IndexManager::collect_all_pages`；实现改用 `integrity::check_tree_page` 的有界读取器自建同型可达性遍历。原因：`collect_all_pages` 对损坏页会 panic（见自检发现 1 的 SIGABRT 证据），而重建在每次打开时运行，绝不能让打开崩溃。连带结果：`src/storage/btree/index_manager.rs` 本次未改动（Plan 的 T3 Targets 提及该文件）。
- **D3（活页集合的目录行来源）**：活页集合并入**打开时的持久目录行快照**（`open_or_init` 在恢复前捕获）与**恢复后的当前目录行**两者的引用页。原因：恢复的 `update_table_root` 会把新根写进内存目录行，只读当前行无法区分新旧两棵树（A3 的前提）。
- **D4（新增测试支撑 API）**：新增 `rtsql::storage::free_page_rebuild::arm_free_page_rebuild_fault(path)`（一次性、按库路径限定）与 `integrity::check_data_page` / `check_tree_page` 的 `pub(crate)` 可见性。原因：R3 的场景要求「页读失败 + `check_integrity` error 数为 0」，损坏夹具无法同时满足（损坏页会被 `check_integrity` 记为 error 级发现）⇒ 必须用一次性受控注入。注入点默认不武装，未武装时行为与该模块不存在等价。
- **D5（A4 的 RED 取证方式）**：A4 用例在受控注入点就位后加入，其 RED 由「临时关闭注入消费点」取得（命令与输出见上），而非先写用例后实现。恢复消费点后 GREEN。

### Persisted Evidence

`None required` —— 验收结论全部由 `cargo test` 的真实运行承载（每个用例的断言直接对应 A1–A6，可在低成本下重跑）；无「一次性环境即将消失 / 需保留现场 / 摘要丢失决定性结构」情形，未创建 `evidence/<iteration>/<cycle>/` 目录，也未新增任何采集、审计或身份证明机制。

### Experience Candidates

1. **Issue 候选（当前 change 范围外、有证据的实质缺陷）——释放路径对损坏页会 panic**
   - 结论：`TableManager::drop_table` / `truncate_table` / `DROP INDEX` 的 best-effort 扫描（`collect_data_pages`、`collect_overflow_chain_pages`、`IndexManager::collect_all_pages`）使用无检查的 `SlottedPageRef::get_slot_data`（`&self.data[start..end]`），对损坏的数据页或内部树页会越界切片 panic，导致 DROP/TRUNCATE 进程崩溃而不是「best-effort 释放」。
   - 根因位置：`src/storage/page_format/slotted_page.rs:128`（`get_slot_data` 无边界检查）；调用点：`src/storage/data/table_manager.rs` 的 `collect_data_pages` / `collect_overflow_chain_pages`、`src/storage/btree/index_manager.rs` 的 `collect_all_pages`。
   - 复现路径 / 证据：本 Cycle 的重建曾走同一访问器路径，被既有 lib 用例 `storage::integrity::tests::slot_area_corruption_detected`（把数据页 `slot_count` 改成 `0xFFFF` 后重开）打出 `panicked at src/storage/page_format/slotted_page.rs:128` + `thread caused non-unwinding panic. aborting.`；释放路径使用的是同一无检查切片与同一 `get_slot` 读取。重建侧已改用 `integrity::check_data_page` / `check_tree_page`（有界）修复。
   - 影响面与触发条件：库文件存在损坏页（位翻转 / 写入撕裂 / 外部改写）且随后执行 DROP TABLE / TRUNCATE / DROP INDEX ⇒ 进程 abort（DML 路径之外的崩溃面）。
   - 与本 change 范围的关系：释放路径属 Iteration 001（T5 的六个释放点），本 Cycle 只读不改。
   - 修复方向：把释放路径的扫描改为有界读取（复用 `integrity::check_data_page` / `check_tree_page`，或给 `SlottedPageRef` 增加返回 `Option` 的有界访问器并让调用方按「best-effort + 告警」处理）。
   - 承载落点建议：MS35 Iteration 001（T5 触及六个释放点）内的同批修复，或独立承载 change（存储层有界读取统一化）。
2. **Runbook 候选（可重复、风险较高的操作路径）——「重建失败安全」的受控验证配方**：对「重建遍历出错」这一场景，损坏夹具不适用（损坏页会被 `check_integrity` 记为 error 级发现，与该场景的「error 数为 0」冲突），已用一次性、按库路径限定的注入点（`arm_free_page_rebuild_fault` → 打开 → 观察「打开成功 + 活表可读 + 本次打开无可分配页 + 注入被消费后下一次打开恢复复用」）端到端验证通过（`tests/free_page_lifecycle_test.rs::rebuild_failure_keeps_live_pages_intact`，8/8 全绿）。该配方可复用为「存储层遍历失败」类场景的验证手段。

### 未解决问题与停止条件

- 未解决问题：自检发现的两个 Important 问题均已在本 Cycle 修复；遗留 Minor 3 条（打开成本、告警形式、取代语义的取舍）已记录，不影响 A1–A6。
- 停止条件（后续 Iteration 的边界，本 Cycle 不越界）：
  - Iteration 001（T4–T7）承担：释放的持久化判定与**会话内复用时机的改变**（⇒ `drop_table_free_test` 5 处 / `overflow_page_test` 1 处 / lib 单测 `free_overflow_chain_returns_pages_for_reuse` 的分界更新，T6）与本 change 的 M19 分批收尾全量回归（T7）。
  - 存储层有界读取统一化（释放路径 panic 面）建议由 Iteration 001 或独立承载 change 处理（见 Experience Candidates 1）。
  - 本 Cycle 未做：全局 tasks / SNAPSHOT / 项目记忆 / `openspec/specs/` 的任何写入；未归档 change；未创建下一 Cycle 或下一 Iteration；未填写 `Plan Review`；未登记或修改任何 Issue。

## Plan Review

- Review Result: accepted

### 检查范围与方式

独立阅读了本 Cycle 的全部产品改动与测试（`src/storage/async_storage.rs`、`src/storage/file_storage.rs`、`src/storage/catalog.rs`、`src/storage/data/table_manager.rs`、`src/storage/free_page_rebuild.rs`、`src/storage/integrity.rs`、`src/database.rs`、`src/storage/mod.rs`、`tests/free_page_lifecycle_test.rs` 全文）与 change 的 `tasks.md` / `design.md` / `proposal.md` / 三份规格增量；用 `git status` / `git diff` 把本 Cycle 的改动与工作区既有现场（MS34 与 issue-fix change 的未提交改动）分开。**未采信 Act Self-Review 代替独立检查**；Act 已报告且覆盖范围未失效的验证结论直接沿用（来源：本文 Act Response 的「Gate 3 / Gate 4 结论」节），本次未重跑全量、未重跑已通过的定向验证（公共规则 › 验证：禁止以增强信心为由重复运行已通过的验证；Plan Review 的独立检查是「独立阅读代码与 diff、核对 Act 声明与代码一致」）。

### 独立核对结论（正面确认，非 finding）

1. **覆盖面独立复算**：独立枚举生产代码的全部 `allocate_page()` 调用点——`catalog.rs:155` / `:162`（目录页 0/1）、`catalog.rs:715`（目录链续页）、`table_manager.rs:465`（表数据页头）、`data_page.rs:56`（数据链续页）、`overflow.rs:155`（溢出链页）、`btree.rs:108` / `:214` / `:263` / `:345`（PK / 唯一 / 二级三类索引树的根与分裂页）——逐一映射到四类页属主，重建实现全部覆盖（目录链 → `Catalog::collect_chain_pages`；表数据链 → `collect_data_chain_strict`；溢出链 → 数据页槽 locator → `collect_overflow_chain_pages_strict`；索引树 → 目录登记根 ∪ 内存索引管理器根 → `collect_tree_pages`）。**未发现遗漏的页属主**。
2. **释放路径独立复算**：独立 grep 全部 `free_page(` / `free_overflow_chain(` 调用点，得到 Act 报告的六类释放点之外还有四处（B 树内部合并/缩根释放 `btree.rs:610/668/774/868`、`create_table` 失败清理 `table_manager.rs:620-627`、`truncate` 树构建失败清理 `table_manager.rs:1920`、恢复期旧树释放 `wal/recovery.rs:1377/1386/1396`）。这四处释放的页同样**不被任何持久目录行登记**（树结构变更后即无引用；失败清理的页从未被登记；恢复期旧树页仍被登记故判为活页），因此**重建对它们的判定是正确的**，不构成本 Cycle 的缺口；但它们**不落在 `design.md` D2.7 的「六个释放点」清单内**，而 D2.7 的清单是 Iteration 001（T5）释放完成标记的接线依据——已作为 Investigation Fact 写入 Iteration 001 的 Plan Context（见该文件「释放点全集与锚点写」节）。
3. **「持久目录 = 恢复前快照」的假设成立且无更早/更可靠时点**：`TableManager::open_or_init`（`src/database.rs:343`）捕获 `persisted_catalog_rows`，其时点早于 WAL 恢复（`src/database.rs:394`）与重建钩子（`src/database.rs:436`）。目录行的 root 只在恢复期被 `update_table_root` / `replace_recovery_indexes` 改写（内存目录页变脏、尚未落盘），因此「恢复前的持久目录行」是唯一能同时区分新旧两棵树的时点。Act 的假设成立。
4. **D1（取代语义）与 D3（并集）不漏不误**：并集（持久快照 ∪ 当前行）覆盖恢复换根前后的两棵树；取代语义保证「恢复期释放但仍被持久目录登记的旧树页」不进入可分配集合——这正是 A3 的保护点。两者方向均为保守（宁可漏回收，不可误发活页）。
5. **A2 变异判别力独立确认**：Act 记录的变异输出 `the new table must not take the live table's data page / left: 2 / right: 2` 与代码推演一致——A2 中 `gone` 先建先删，其页在同会话被 `live` 复用，故 `live` 的数据页 = 2；变异（不把数据链页计入活页）后 2 进入可分配集合，且空闲集合按页号升序装配、按最大页号优先发放（`FileStorage::allocate_page` 弹出空闲集合末项），故新表首先拿到 2 ⇒ 仅该断言失败。变异确实只影响对应断言的判别力。
6. **A4 变异判别力独立确认**：临时禁用注入消费点后重建不放弃 ⇒ 释放空间被复用 ⇒ `an abandoned rebuild must not reuse any released space` 失败。与实现一致（`take_free_page_rebuild_fault` 未被消费时重建照常成功）。
7. **损坏页未被弱化为「静默忽略」**：修复后重建在损坏页上返回错误 → 整次放弃 → 打开仍成功；既有 lib 用例 `storage::integrity::tests::slot_area_corruption_detected` 的断言走的是 `check_integrity` 独立检查路径（`integrity.rs` 本次**仅可见性变更**，检测语义未动），损坏仍被计为 error 级发现并被断言命中。R3 的「error 数为 0」前提是「错误被计数」而非「错误被吞掉」——本用例恰好证明两者可分离：重建的失败安全不吞掉 `check_integrity` 的计数。
8. **「本 Cycle 未更新任何既有断言」独立确认**：`git status` 显示 `tests/drop_table_free_test.rs`、`tests/overflow_page_test.rs`、`src/storage/page_format/overflow.rs` 均**不在**本次工作区改动清单内 ⇒ Act 的该声明属实，T6 的 7 处断言确未被提前改动。
9. **接缝缝隙边界**：`inject_free_pages` 是 `AsyncStorage` 的**带默认空实现的公开 trait 方法**（`src/storage/async_storage.rs:39`）⇒ `MemoryStorage` 与测试替身零改动、必实现方法集合不变（T2 约束满足）；内存模式根本不进入重建调用点（`src/database.rs` 的钩子在读写分支内，只读与内存模式均不调用）⇒ 两种存储不会在同一条 requirement 上产生不一致断言。默认空实现本身不构成行为分叉。
10. **无身份证明机制与判定层**：新代码与新测试中未出现 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明，未出现 timer 阈值；全部判定在 `cargo test` 的断言内。
11. **ATTACH 附加库无缺口**：附加库经 `Database::open_with_lock_wait`（`src/cli/mod.rs:810`）打开 → 走同一 `open_internal` → 重建钩子同样生效。

### Findings

**阻塞 Acceptance 的实质问题：无。**

以下均为**非阻塞 Minor finding**（记录，不在本 Cycle 扩展）：

| # | finding | 类别 | 偏差原因 |
|---|---|---|---|
| m1 | Act Response 的改动文件表把 `src/storage/data/mod.rs` 列为已改动；`git status` 显示本次只改了 `src/storage/mod.rs`，`src/storage/data/mod.rs` 未出现在工作区改动中 | 文档级偏差 | Act 偏离（表格笔误） |
| m2 | 公开面扩大：`AsyncStorage::inject_free_pages`（公开 trait 方法，带默认实现）、`pub mod free_page_rebuild` + `arm_free_page_rebuild_fault` / `take_free_page_rebuild_fault`（测试支撑，因集成测试需公开可达而暴露）、`Catalog::collect_chain_pages`、`TableManager::rebuild_free_page_set`。必实现方法集合未变、外部实现者不受影响；测试支撑 API 长期留在公开面上，建议后续收敛到 `test_support` 子模块或 feature 门控 | API 面 | Plan 遗漏（D4 的副产物） |
| m3 | 每次读写打开都遍历全部活页并物化空闲集合（O(活页数) 时间、O(空闲页数) 内存）。`design.md` D1.2 已把性能列为 Non-goal 并接受该取舍 | 性能风险（已声明接受） | 现状变化（D1 路线选择的固有成本） |
| m4 | 重建遍历数据页时不校验 `page_type`（`integrity::check_data_page` 返回页类型但重建未判）。页类型异常时只会让遍历覆盖更多页，方向保守，不会误判活页为空闲 | 覆盖精度 | Act 偏离（非实质） |
| m5 | A4 的 RED 记录行号 `tests/free_page_lifecycle_test.rs:308` 与最终文件不一致（最终断言在 ~315 行），失败消息文本与最终断言完全一致；取证后用例被追加导致行号漂移 | 证据卫生 | Act 偏离（非实质） |

### 路线选择裁定（D1）

**裁定：路线 (a)「打开时按持久目录安全重建 + 释放持久化闸门」成立，排除 (b)「持久空闲页管理」正确。**

- 排除 (b) 的理由成立：(b) 会引入**第二份持久真相**。空闲集只有在「释放先于发放且两者都已持久化」的窄窗口内才与目录一致；崩溃点落在「空闲集已持久化、catalog 的删除尚未持久化」之间时，陈旧空闲集会认领 catalog 仍登记的活页 ⇒ 后续发放把活页交给新对象 ⇒ **静默损坏**（活页被清零/改写，目录与页内容不一致，且无自检信号）。(a) 每次打开都从唯一持久真相（持久目录）推导可分配集合，崩溃点落在任何位置都不会产生第二份真相，也不需要「空闲集与目录的原子提交」。
- MS35-T01 的约束（「不把内存空闲列表原样写成持久格式作为唯一方案」）被满足：实现**没有**持久化任何空闲列表，`FileStorage` 的空闲集合始终是内存态，重开时重建。
- 残余成本（已记录为 m3 与设计 D1.2 的取舍）：重建为 O(活页数)；跨重启复用的空间在「释放被持久化之后的下一次打开」才回到可分配集合。

### Deviations 裁定

| Deviation | 裁定 | 理由 |
|---|---|---|
| D1（接缝取「取代」语义而非 Plan 的「叠加」） | **成立**（Plan 遗漏） | R2 原文是「**重建**本次打开的可分配页集合」；A3 要求持久目录登记的树页不进入可分配集合，而恢复换根释放的旧树页此刻仍在存储层空闲列表里 ⇒ 「叠加」语义在物理上无法满足 A3。取代语义是满足 R2 + A3 的唯一相容语义，且与重建的来源（持久目录）自洽 |
| D2（树遍历改用 `integrity::check_tree_page` 而非 `IndexManager::collect_all_pages`） | **成立**（Plan 错误） | `collect_all_pages` 对损坏页会 panic，而重建在**每次打开**运行，打开不可崩溃。已独立核对该有界读取器与生产遍历的可达性等价：内部节点子指针取槽内 `MAX_KEY_LEN` 偏移处的 4 字节（与 `InternalNodeRef::get_child_page_id` 同）、叶子后继取页头 `next_page_id`（与 `LeafNodeRef::next_leaf_page_id` 同）、内部节点左孩子取页头 `next_page_id`（与 `leftmost_child` 同）⇒ 覆盖面未被削减 |
| D3（活页集合 = 持久目录快照 ∪ 当前目录行） | **成立**（现状变化） | 恢复期 `update_table_root` 改写内存目录行，只读当前行无法区分新旧两棵树；并集方向保守 |
| D4（新增测试支撑注入 API + `pub(crate)` 有界读取器） | **成立**（Plan 遗漏） | R3 要求「页读失败」且「`check_integrity` error 数为 0」，损坏夹具与该前提互斥（损坏页必被 `check_integrity` 记为 error 级发现）⇒ 必须用一次性受控注入。注入点默认不武装（未武装时行为与该模块不存在等价）、按库路径限定（并行测试互不干扰）；`pub(crate)` 读取器未对外暴露（`integrity` 模块虽 `pub`，条目为 crate 级） |
| D5（A4 的 RED 由临时禁用注入消费点取得） | **成立**（取证方式差异） | 不改变断言语义；消费点已恢复，同命令 GREEN |

### Acceptance Gaps

无。Iteration 000 的既有 Acceptance（FPL R2 重建、FPL R3 失败安全、DTPF「跨重启 drop 安全」MODIFIED）已由 8 个用例的 GREEN 运行与本次独立代码核对共同证明；无未满足项。

### Evidence

- 代码：`src/storage/async_storage.rs:24-39`（接缝与取代语义）、`src/storage/file_storage.rs`（`inject_free_pages` 装入并替换空闲集合；`allocate_page` 弹出即写零页 ⇒ 复用页必被清零）、`src/storage/catalog.rs`（`collect_chain_pages` 沿 `next_page_id` 走到 0，环检测，读失败即报错）、`src/storage/data/table_manager.rs:284`（持久目录快照）、`:1599-1790`（`rebuild_free_page_set` / `collect_live_pages` / `collect_data_chain_strict` / `collect_overflow_chain_pages_strict` / `collect_tree_pages`）、`src/storage/free_page_rebuild.rs`（一次性按路径限定的注入点）、`src/storage/integrity.rs`（仅可见性变更）、`src/database.rs:343/394/436`（快照 → 恢复 → 重建钩子的时序；钩子出错只告警、打开仍成功）。
- 沿用的验证结论（来源：Act Response「Gate 3 / Gate 4 结论」，覆盖范围未失效）：`cargo test --test free_page_lifecycle_test -- --test-threads=1` → 8 passed / 0 failed；基线 `drop_persistence_test` 11 + `drop_table_free_test` 6 + `overflow_page_test` 27 全 0 failed；`cargo test --lib --bins` → 420 passed / 0 failed；定向审计 3 批（63 + 112 + 184）全 0 failed。**全量回归未跑**，按 `tasks.md` 的收尾验证预算记在 Iteration 001 的 T7。
- 工作区：`git status` 显示本 Cycle 的产品改动 = 6 个改动文件 + 1 个新模块 + 1 个新测试文件；`tests/drop_table_free_test.rs`、`tests/overflow_page_test.rs`、`src/storage/page_format/overflow.rs` 未被改动（⇒ T6 的 7 处断言未被提前改动）。

### 收敛判断

Iteration 000 的 Acceptance 已满足，无阻塞项，无当前 Cycle 修复需求；`Review Result` 取 `accepted`，Iteration 000 完成。

### Iteration Plan Update

无。`tasks.md` 的 Iteration 000（Stable result / Verification boundary / Diagnostic boundary / Non-goals）与平衡审计保持不变；Iteration 001 的 Plan 亦不变（T4–T7 的任务划分、验证边界与收尾全量预算均沿用 `tasks.md` 已获批的版本）。

### Follow-up Decision

**接受 Iteration 000**（`accepted`）：不创建 rework / replan Cycle，当前 Cycle 无需修复。按 `tasks.md` 的 Iteration Map 展开下一个逻辑 Iteration —— `iterations/001-durable-release-gate/000-initial.md`（Plan Context 状态 `ready`）。

Act 报告的 Issue 候选（释放路径 `drop_table` / `truncate_table` / `DROP INDEX` 使用无检查的 `SlottedPageRef::get_slot_data`，在损坏页上 panic）经裁定为**本 change 范围外**，按 M20「发现方只报告与移交，不在当前 change 内扩范围修复；落在存储/索引层的由存储域后续 change 承载」移交，不写入 Iteration 001 的 Task Definition；理由与承载建议见下方「报告的 Issue 候选」。

### Next Cycle

None

### Next Iteration

`openspec/changes/2026-10-24-ms35-free-page-reuse-across-restart/iterations/001-durable-release-gate/000-initial.md`
