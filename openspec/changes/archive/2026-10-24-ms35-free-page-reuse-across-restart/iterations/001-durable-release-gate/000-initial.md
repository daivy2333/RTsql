# Iteration 001 / Cycle 000: 释放的持久化判定——未完成释放永不可分配，关闭 MS33 遗留窗口

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4, T5, T6, T7
- Acceptance gaps: None（Iteration 000 的 Acceptance 已满足并被 Review 接受，无待关闭的验收缺口）
- Repair items: None
- Inherited scope:
  - Iteration 000 已验收的成果（工作区现状，可直接复用）：`AsyncStorage::inject_free_pages`（带默认空实现的接缝，**取代**语义）、`FileStorage::inject_free_pages`（装入并替换可分配集合）、`Catalog::collect_chain_pages`、`TableManager::persisted_catalog_rows` + `open_or_init` 的持久目录快照、`TableManager::rebuild_free_page_set` / `collect_live_pages` / `collect_data_chain_strict` / `collect_overflow_chain_pages_strict` / `collect_tree_pages`、`src/storage/free_page_rebuild.rs`（一次性按库路径限定的重建故障注入点）、`integrity::check_data_page` / `check_tree_page`（`pub(crate)` 有界读取器）、`src/database.rs::open_internal` 的重建钩子（读写模式、`attach_index_catalog_contexts()` 之后、出错只告警且打开仍成功）。
  - 有效 requirement：`free-page-lifecycle`（FPL）R1–R4、`drop-table-physical-free`（DTPF）R（MODIFIED）、`drop-persistence-boundary`（DPB）R1（MODIFIED）。
  - 有效约束：M19 分批串行回归纪律；M20 发现方只报告移交；MS35 不改文件格式、不加伴生文件；验证场景只用小表（避开 ISS23）。
- Excluded scope:
  - 释放路径的**有界读取**改造（`drop_table` / `truncate_table` / `DROP INDEX` 扫描损坏页时的 panic 面）——经 Review 裁定为本 change 范围外，按 M20 作为 Issue 候选移交存储域后续 change 承载（理由见本 Cycle 的 `Risks and Notes`）。
  - 尾部截断与整库整理；无键行旧版本 GC（I038）；文件格式与伴生文件变更（MS36）；ISS22 / ISS23 / ISS24 / ISS25 的修复；MS34 三个新增域的语义；`TRUNCATE` 自身的既有崩溃语义。

**Objective**

被释放的页只有在其释放已被一次「在释放完成之后开始且成功完成」的 checkpoint 刷脏页持久化之后，才进入可分配集合；释放动作本身不写零页；刷盘失败时待释放项被丢弃（不提升）。⇒ MS33 R1 末句移交的窗口（「同会话复用释放页后在分配方 catalog 落盘前崩溃」）不再以「旧表数据被清零且不可恢复」的形式出现：崩溃后旧表已提交行仍可查询，新表不存在，完整性无 error 级发现。会话内复用的时机从「释放后立即」改为「释放持久化后」（本 change 唯一的行为回退面，已在 `design.md` D2.6 与 `drop-table-physical-free` 的 MODIFIED 增量中显式声明）。

**Background**

- Iteration 000（`iterations/000-reopen-free-page-reconstruction/000-initial.md`）已 `accepted`：重新打开时按持久目录重建可分配页集合已成立并被 8 个用例钉住。该成果提供**跨重启**维度的空闲页来源，但**会话内**的释放时机仍是 MS33 的「释放后立即可分配」——这正是 MS33 R1 末句移交的窗口。
- 用户指令（goal）：「把已经规划的三个 ms 逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」；补充 2：「实施当中发现的所有 issue 候选，都直接寻找好的承载方式流程进行修复而不是等级」。本 Cycle 是 MS35 的最后一个 Iteration，承载本 change 的收尾全量回归。

**Investigation Facts**

- Starting point:
  - 工作区：HEAD `0a08b14`（ahead of origin 9）；MS34 + issue-fix change + MS35 Iteration 000 的改动**均未提交**（`src/` 下 9 个产品文件、本 Cycle 的 6 个改动文件 + 1 个新模块、`tests/` 下 3 + 1 个测试文件、`.agents/` 与 `openspec/specs/` 下的文档改动）——属既有现场，不要回滚、不要清理、不要提交。
  - 本 Cycle 的起点 = Iteration 000 的最终状态（改动、验证结论、accepted Review 见其 Cycle 文件）。Iteration 000 的验证结论**直接沿用**（来源：Iteration 000 Act Response 的「Gate 3 / Gate 4 结论」节；覆盖范围未失效）：`cargo test --test free_page_lifecycle_test -- --test-threads=1` → 8 passed / 0 failed；`cargo test --test drop_persistence_test --test drop_table_free_test --test overflow_page_test -- --test-threads=1` → 11 + 6 + 27 passed / 0 failed；`cargo test --lib --bins` → 420 passed / 0 failed；定向审计 3 批（63 + 112 + 184）全 0 failed。**全量回归未跑**，预算记在本 Iteration 的 T7。
  - 已知限制：Iteration 000 的 Review 记录了 5 条非阻塞 Minor finding（m1 文档表格笔误、m2 公开面扩大、m3 打开成本 O(活页数)、m4 数据链页未校验 `page_type`、m5 证据行号漂移）。m3 是本 Cycle 的既有性能特征（本 Cycle 不改变打开期重建的复杂度）；m2 的公开测试支撑 API 保留供 T4 的注入用例复用。
- Current-State Evidence:
  - **释放点全集与锚点写**（独立 grep 全部 `free_page(` / `free_overflow_chain(` 调用点所得；`design.md` D2.7 的「六个释放点」是其中**锚点写与释放分离**的那六个，其余四处在本 Cycle 首次被识别出来）：
    | 释放点 | 位置 | 释放的页 | 锚点写（使该页在持久态中不再被引用的写） | 分类 |
    |---|---|---|---|---|
    | DROP TABLE | `src/storage/data/table_manager.rs` 的 `drop_table`（`free_page` 批在 ~1473 行） | 表数据链页、溢出链页、PK / 唯一 / 二级索引树页 | 目录行删除（`__tables` / `__columns`） | (i) 有锚点写 |
    | TRUNCATE | 同文件 `truncate_table`（~1839 / ~1847 / ~1938） | 旧尾页、旧溢出链页、旧索引树页 | 表头页原地重置 + 尾指针归位 + 目录 root 更新 | (i) 有锚点写 |
    | DROP INDEX | `src/executor/drop_index.rs:62` | 该索引的树页 | 目录 secondary 段更新 | (i) 有锚点写 |
    | GC 清理旧版本 | `src/storage/data/table_manager.rs:173` → `src/storage/data_page.rs:249` | 旧版本行引用的溢出链页 | 删除槽位后的数据页写（locator 消失） | (i) 有锚点写（**锚点写在调用方**） |
    | CREATE INDEX 回填失败清理 | `src/executor/create_index.rs:83` | 回填期间分配、尚未登记的索引树页 | 目录行未登记该索引（无锚点写） | (ii) 无锚点写 |
    | 溢出链写入失败清理 | `src/storage/page_format/overflow.rs:140`、`src/storage/data_page.rs:81` | 写入失败时已分配、未落 locator 的溢出链页 | locator 未落（无锚点写） | (ii) 无锚点写 |
    | **B 树内部合并 / 缩根释放** | `src/storage/btree/btree.rs:610`（合并叶子释放右叶子）、`:668`（子合并释放被并入页）、`:774`（缩根释放旧根）、`:868`（内部合并释放右内部页） | 树结构改写后不再被树引用的树页 | 同一写单元内的树页写；`shrink_root` 另有**目录 root 更新**（新根发布） | (i) 有锚点写（`shrink_root` 的锚点写 = 目录 root 更新） |
    | **create_table 失败清理** | `src/storage/data/table_manager.rs:620-627` | 建表失败时已分配的数据页与索引根 | 目录行未登记（无锚点写） | (ii) 无锚点写 |
    | **truncate 树构建失败清理** | `src/storage/data/table_manager.rs:1920` | 新树构建失败时已分配的新树页 | 目录 root 未更新（无锚点写） | (ii) 无锚点写 |
    | **恢复期 redo 失败的溢出链释放** | `src/wal/recovery.rs:214` | redo 失败时已写、locator 未落的溢出链页 | locator 未落（无锚点写） | (ii) 无锚点写 |
    | **恢复期旧索引树释放** | `src/wal/recovery.rs:1377` / `:1386` / `:1396` | 恢复换根后不再被引用的旧树页 | 目录 root 更新（`update_table_root` / `replace_recovery_indexes` 写入内存目录页，尚未落盘） | (i) 有锚点写（**恢复期**） |
  - **持久化边界的位置**：`Database::checkpoint()`（`src/database.rs:860`）→ `reclaim_abandoned_implicit_transactions()` → `CheckpointManager::checkpoint(|| current_tx_id())`（`src/wal/checkpoint.rs:169`）的九步：0 = 零工作早退（WAL 无追加且池无脏页时整体无操作）；1 = 取当前 WAL LSN；1b = 取分配器水位；**2 = `buffer_pool.flush_all()`（刷所有脏页）**；3 = fsync WAL；4–8 = 写位点 / 追加 Checkpoint 记录 / 截断 WAL / 复位位点；9 = 重置写入计数。**释放完成编号必须在第 2 步刷盘之前取样；提升只能发生在第 2 步成功之后；第 2 步失败必须丢弃。** 第 0 步的零工作早退会跳过第 2 步——早退时待释放项不得被永久搁置（见 T5 的可观察要求 4）。
  - **存储层接缝现状**：`AsyncStorage`（`src/storage/async_storage.rs`）当前方法集 = `read_page` / `write_page` / `allocate_page` / `free_page` / `sync`（必实现）+ `page_size` / `page_count` / `inject_free_pages`（**均带默认实现**）。`FileStorage::allocate_page` 的语义：优先弹出可分配集合末项并**写零页**，否则扩展文件并递增 `file_len`；`page_count()` = 已分配页号上界（`file_len`）。
  - **内存库形态**：`Database::open_in_memory()` 的 `checkpoint_manager` 为 `None` ⇒ `checkpoint()` 是无操作（`src/database.rs:861-863`）；`MemoryStorage::free_page` 立即把页写零并移出集合（其页始终「在库里」，无崩溃边界）。内存模式**不进入**重建调用点（`src/database.rs` 的钩子在读写分支内）。
  - **既有断言语义护栏**：`tests/drop_persistence_test.rs` 11 例全部走「释放后崩溃」的既有语义（`drop_table_crash_before_checkpoint_keeps_rows`、`drop_index_crash_before_checkpoint_keeps_index` 等），是本 Cycle 的回归护栏；`tests/drop_table_free_test.rs` 的 `test_cross_restart_after_drop_safe`（重开后重建 + 插入可查）同样不随本 Cycle 改变。
  - **可用注入点**：`Database::arm_io_failure(IoFailPoint::DataWrite)`（`src/database.rs:881`）——下一次数据文件页写入在任何字节落盘前失败并自清除，可用于制造「第 2 步刷盘失败」；`rtsql::storage::free_page_rebuild::{arm_free_page_rebuild_fault, take_free_page_rebuild_fault}`（Iteration 000 引入，一次性、按库路径限定）用于制造重建遍历失败。
  - **崩溃模拟配方**：`.agents/runbooks/in-process-crash-simulation.md`（R49）——`wal_buffer.shutdown()` 后 `drop(db)`，**不调用 `close()`**（`close` 会 checkpoint 并截断 WAL，崩溃语义就没了）。
  - **全量回归配方**：`.agents/runbooks/m19-batched-full-regression.md`（R48）——当前 `tests/*.rs` 共 134 个文件（`ls tests/*.rs | sort`），加 `--lib --bins` 一批 ⇒ 135 个目标；2026-10-24 实例为 133 文件 + lib/bins = 134 目标、13 批、2171 passed / 0 failed / 2 ignored（2 个 ignored 是 `cli_test.rs` 的长期 `#[ignore]` 用例）。
- Code and Critical Path:
  - 关键路径（会话内释放 → 可分配）：`executor/drop_table.rs` / `executor/truncate.rs` / `executor/drop_index.rs` / `executor/create_index.rs` → `TableManager::{drop_table, truncate_table}` → `BufferPool::free_page`（`src/storage/buffer_pool.rs:493`，只清缓存，不写盘）→ `AsyncStorage::free_page` → `FileStorage::free_page`（`src/storage/file_storage.rs`，当前直接 push 进可分配集合）→ 下一次 `allocate_page` 弹出并写零页。**本 Cycle 的改动点就在 `FileStorage::free_page` 之后与 `CheckpointManager::checkpoint` 第 2 步前后**。
  - 持久化边界路径：`Database::checkpoint()` → `CheckpointManager::checkpoint`（九步，第 2 步 = `flush_all()`）→ `BufferPool::flush_all()` → `FileStorage::write_page`。
  - 跨重启路径（Iteration 000 已建立，本 Cycle 复用）：`Database::open_internal` → `TableManager::open_or_init`（持久目录快照）→ WAL 恢复（可能换根并释放旧树页）→ `attach_index_catalog_contexts()` → 重建钩子 `TableManager::rebuild_free_page_set` → `Catalog::collect_chain_pages` / `collect_data_chain_strict` / `collect_overflow_chain_pages_strict` / `collect_tree_pages` → 差集 → `AsyncStorage::inject_free_pages`（取代语义）。

**Implementation Guidance**

1. **先建立可观察的分界，再改机制**。T4 的四个用例必须先 RED（当前实现是「释放后立即可分配」），RED 的失败原因必须落在**可分配时机**上（不是编译错误、不是 panic）。
2. **机制按 `design.md` D2.3–D2.5**：存储层持有待释放集合与「释放完成编号」；checkpoint 在第 2 步刷盘**之前**取样编号，刷盘**成功**才提升、**失败**必须丢弃；释放完成标记必须在**锚点写已发出之后**才发出（否则会出现「锚点写未落盘但页已被发放」的提前提升）。接口形态要求：`AsyncStorage` 的**必实现方法集合不变**——新增能力必须是带默认实现的 trait 方法（`inject_free_pages` 已是先例），`MemoryStorage` 与测试替身零改动。
3. **释放点按上文表格逐点归类**：(i) 类（有锚点写）必须在锚点写之后发出完成标记；(ii) 类（无锚点写）不需要标记，但必须能证明这些页在重建时仍判为空闲（重建已覆盖：未被任何链或目录行登记 ⇒ 空闲）。`shrink_root` 与恢复期旧树释放是本 Cycle 新识别出的两处高风险点（锚点写分别是目录 root 更新），必须逐点处理并在 Act Response 记录。
4. **内存库形态显式裁定**：内存库没有崩溃边界（页始终在「库」里，`checkpoint()` 是无操作），因此释放持久化闸门**不适用于内存库**——内存库维持「释放后立即可分配」的既有行为。实现上，内存存储的 `free_page` 保持现状（默认实现即无操作语义），文件库存储承担待释放集合与提升/丢弃。`free-page-lifecycle` 增量规格的措辞以「崩溃边界」为限定语，对内存库自然不适用，与 `in-memory-database` 域不冲突；T5 须在 Act Response 记录该裁定，并用一个用例钉住内存库的既有行为不被改变。
5. **不得新增身份证明机制或判定层**：全部判定落在 `cargo test` 的断言内；计数只用一步通用管道；不得引入 timer 阈值、hash/指纹/revision pin/run-id/manifest。

**Task Definitions**

### T4: 先行 RED 用例——释放未持久化前不可分配、持久化后可分配、崩溃后旧表内容完整、刷盘失败不提升

- Requirement/Scenario: FPL R1 场景 1（释放未持久化前页不可分配）、FPL R1 场景 2（释放持久化后可分配）、FPL R4 场景（崩溃后未完成的释放不是可分配页）、DPB R1（MODIFIED）场景（checkpoint 前崩溃保留 DROP 前的已提交状态）；DTPF R（MODIFIED）的同会话复用时机。
- Depends on: None（可与 T5 的实现并行准备，但 RED 必须先于实现成立）
- Targets: `tests/free_page_lifecycle_test.rs`（新增用例）、`.agents/runbooks/in-process-crash-simulation.md`（崩溃模拟配方的引用，不修改）
- Current behavior: 释放后的页立即进入可分配集合（MS33 语义）：同一会话内 DROP TABLE 后新建表即复用被释放的页；`tests/drop_table_free_test.rs` 6 例按此断言。
- Required behavior: 四个用例走真实入口（真实文件库 + 真实 `Database::open` / `close` / `checkpoint`），并分别钉住：
  1. `released_page_not_allocatable_before_release_durable`——DROP TABLE 后**未 checkpoint**：同一会话新建表**不得**占用被释放的页（可观察：新建表的数据页号 ≠ 被释放的页号，且库文件页数上界增长）。
  2. `released_page_allocatable_after_release_durable`——DROP TABLE 后执行一次成功 checkpoint：同一会话新建表**必须**占用被释放的页（可观察：库文件页数上界不增长，且新建表的数据页号 ∈ 被释放的页集合）。
  3. `crash_with_pending_release_keeps_old_table_rows`——按 R49 配方（`wal_buffer.shutdown()` 后 `drop(db)`，**不调用 `close()`**）：DROP TABLE 成功后崩溃 → 重新打开：旧表仍存在且已提交行可查询；崩溃前创建的新表不存在；`check_integrity` 的 error 级发现数为 0。
  4. `flush_failure_does_not_promote_pending_release`——用 `Database::arm_io_failure(IoFailPoint::DataWrite)` 让 checkpoint 的刷盘失败：被释放的页**不得**被提升为可分配（可观察：失败后同一会话新建表不占用被释放的页），且数据库仍可正常使用、`check_integrity` 的 error 级发现数为 0。
- Preserve: 崩溃模拟不得使用真实断电或外部进程 kill（用 R49 的进程内配方）；用例必须走真实入口，不得在框架外另建判定层；表规模保持小表（不触发 ISS23）；`check_integrity` 的 error 计数必须由既有 API 读出，不得新增指标。
- Forbidden: 不得改写既有断言的期望（属 T6）；不得引入判定脚本、封装命令、专用参数、timer 阈值或任何身份证明机制；不得在本 Task 内实现释放持久化判定（属 T5）。
- First test: 变更前（当前工作区）写用例并运行 `cargo test --test free_page_lifecycle_test -- --test-threads=1`，观察 RED：四个新用例中「释放未持久化前不可分配」与「刷盘失败不提升」必须失败，失败原因落在**可分配时机**上（如新建表占用了被释放的页 / 页数上界未增长）；「崩溃后旧表内容完整」在当前实现下**可能平凡通过**（当前实现没有待释放集合，崩溃后旧表行因目录未落盘而仍在），该用例此时作为回归护栏存在，不要求它 RED。
- GREEN condition: 四个用例全部通过，且与 T2/T3 的既有 8 个用例同时 GREEN。
- Verification: `cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ `0 failed`；每个用例的断言直接对应上面 4 条 Required behavior。
- Stop when: 需要改变 requirement 语义、error 语义或用例的可观察量定义时返回 Plan；若发现释放点的锚点写判定与上表冲突且无法在 T5 的执行依据内解决，返回 Plan。

### T5: 释放的持久化判定实现——待释放集合 + 释放完成编号 + checkpoint 第 2 步钩子（成功提升 / 失败丢弃）+ 全部释放点的完成标记

- Requirement/Scenario: FPL R1 场景 1/2、FPL R3（失败安全：不得把活页判成可分配页）、FPL R4、DTPF R（MODIFIED）、DPB R1（MODIFIED）。
- Depends on: T4
- Targets: `src/storage/file_storage.rs`（待释放集合、释放完成编号、存储层入口）、`src/storage/async_storage.rs`（新增**带默认实现**的 trait 能力，必实现方法集合不变）、`src/wal/checkpoint.rs`（第 2 步前取样、成功提升、失败丢弃）、`src/storage/data/table_manager.rs`（`drop_table` / `truncate_table` / 建表与树构建失败清理的释放完成标记）、`src/executor/drop_index.rs`、`src/executor/create_index.rs`、`src/storage/data_page.rs`（GC 路径的释放完成标记）、`src/storage/page_format/overflow.rs`（溢出链写入失败清理）、`src/storage/btree/btree.rs`（B 树内部合并 / 缩根的释放完成标记；`shrink_root` 的标记须在目录 root 更新之后）、`src/wal/recovery.rs`（恢复期旧树释放的释放完成标记）
- Current behavior: `FileStorage::free_page` 直接把页 push 进可分配集合 ⇒ 释放后立即可分配；`CheckpointManager::checkpoint` 的九步中没有释放相关钩子。
- Required behavior（可观察要求，按此判定，不因实现形态不同而改变）:
  1. **释放未持久化前不可分配**：任一释放点释放的页，在其释放在「崩溃边界上成立」之前（即锚点写尚未被一次在释放完成之后开始的 checkpoint 刷出）不得成为可分配页。
  2. **释放持久化后可分配**：上述 checkpoint 成功完成后，该页立即可分配（同一会话内新建对象即取到它）。
  3. **刷盘失败不提升**：第 2 步刷盘失败时，待释放项被丢弃（不提升），且数据库保持一致（不出现活页被发放）。
  4. **不得被永久搁置**：一旦某次释放已持久化，该页必须在**之后第一次成功 checkpoint** 变为可分配；若会话在之前结束，则在**下一次打开的重建**中变为可分配（Iteration 000 的重建已按持久目录判定，未被登记的页即空闲）。checkpoint 第 0 步的零工作早退跳过刷盘时，也必须满足本条的后半句。
  5. **不得提前提升**：锚点写已发出但尚未被刷出的释放，其页在任何时刻都不得可分配（由「释放完成编号在第 2 步之前取样 + 完成标记在锚点写之后发出」共同保证）。
  6. **内存库形态不变**：`:memory:` 存储维持「释放后立即可分配」的既有行为（无崩溃边界、`checkpoint()` 为无操作）；实现不得让内存库进入待释放集合而永不提升。
- Preserve: `AsyncStorage` 的**必实现方法集合不变**（新增能力必须是带默认实现的 trait 方法）；`MemoryStorage` 与既有测试替身零改动；`FileStorage::allocate_page` 的「弹出即写零页」语义不变；`free_page` 的对外语义（页不再被任何链引用）不变；Iteration 000 的重建逻辑与「取代」接缝语义不变（重建仍是跨重启维度的唯一空闲页来源）；文件格式与伴生文件不变（无新增文件、无格式变更）；既有 `drop_persistence_test` 11 例的崩溃语义不变。
- Forbidden: 不得改动既有断言的期望（属 T6）；不得引入持久化空闲页结构（路线 (b) 已被 D1 排除）；不得引入身份证明机制或框架外判定层；不得在本 Task 内修复释放路径的有界读取缺陷（范围外，按 Issue 候选移交）。
- First test: T4 的四个用例已 RED（来源：T4）。实现后同命令 GREEN。
- GREEN condition: T4 四个用例 GREEN，且 T1–T3 的 8 个既有用例仍 GREEN。
- Verification: `cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ `0 failed`；`cargo test --test drop_persistence_test -- --test-threads=1` ⇒ `0 failed`（11 例崩溃语义护栏）；`cargo test --test drop_table_free_test -- --test-threads=1` ⇒ 在 T6 完成前，**本 Task 不要求其通过**（其 5 处断言属 T6 的分界更新范围；T5 完成后它们会因时机变更而失败，这是预期状态，须在 Act Response 中明确记录）。
- Stop when: 需要改变 requirement 语义、公开接口形态或存储层接缝的必实现方法集合时返回 Plan；若「释放完成编号」的取样点无法同时满足要求 2 与要求 5，返回 Plan（不得以放宽其中一条换取通过）。

### T6: 既有同会话复用断言的分界更新（逐处记录旧期望 → 新期望 → 依据哪条 requirement）+ 端到端回归汇总

- Requirement/Scenario: DTPF R（MODIFIED）「可分配时机 SHALL 受释放的持久化判定约束」；FPL R1 场景 1/2；FPL R4。
- Depends on: T5
- Targets: `tests/drop_table_free_test.rs`（5 处断言）、`tests/overflow_page_test.rs`（1 处断言）、`src/storage/page_format/overflow.rs` 的 lib 单测 `free_overflow_chain_returns_pages_for_reuse`（1 处断言）、`tests/free_page_lifecycle_test.rs`（端到端回归汇总：`check_integrity` error 计数与重开可读性断言）
- Current behavior: 上述 7 处断言按「释放后立即可分配」成立（当前全部 GREEN）。
- Required behavior: 逐处把「同会话立即复用」改为「释放持久化之后复用」，**断言语义保留、强度不减**，并按下表逐处记录「旧期望 → 新期望 → 依据哪条 requirement」：

  | # | 位置 | 旧期望 | 新期望 | 依据 |
  |---|---|---|---|---|
  | 1 | `tests/drop_table_free_test.rs` `test_simple_drop_releases_data_and_btree`（`count_after_recreate == count_after_create`，"recreating users must reuse freed pages (free-list) without growing the file"） | 释放后立即复用 ⇒ 文件不增长 | 拆两段：**(a) 未 checkpoint** ⇒ 新建表**不**占用被释放的页（文件页数上界增长，且新建表数据页号 ∉ 被释放页集合）；**(b) checkpoint 之后** ⇒ 新建表占用被释放的页，`count_after_recreate == count_after_create`（与旧期望等价） | DTPF R（MODIFIED）+ FPL R1 场景 1/2 |
  | 2 | `tests/drop_table_free_test.rs` `test_long_data_page_chain_all_released`（`count_after == count_filled`） | 同上 | 同 1 的两段式；checkpoint 后 `count_after == count_filled` | 同上 |
  | 3 | `tests/drop_table_free_test.rs` `test_btree_height_gt_1_all_pages_released`（`count_after == count_filled`） | 同上 | 同 1 的两段式；checkpoint 后 `count_after == count_filled` | 同上 |
  | 4 | `tests/drop_table_free_test.rs` `test_same_process_free_list_reuse`（`count_after <= count_before_drop + 1`） | 释放后立即复用 ⇒ 增长 ≤ 1 页 | **(a) 未 checkpoint** ⇒ 该上限不再成立（页未持久化 ⇒ 不可分配 ⇒ 增长 ≥ 1 页）；**(b) checkpoint 之后** ⇒ 恢复旧上限 `count_after <= count_before_drop + 1` | 同上 |
  | 5 | `tests/drop_table_free_test.rs` `test_concurrent_drop_different_tables`（`count_after_recreate == count_after_drop`） | 同 1 | 同 1 的两段式；checkpoint 后 `count_after_recreate == count_after_drop` | 同上 |
  | 6 | `tests/overflow_page_test.rs` `truncate_releases_overflow_chain_pages_keeps_definition`（断言新建 PK 树根 ∈ 被释放的溢出链页集合，且未取用的链页在磁盘上为零） | TRUNCATE 后立即复用链页 | TRUNCATE 后（未 checkpoint）新树根**不**取自链页；**checkpoint 之后**新建的树**必须**取自该链页集合，且未取用的链页在磁盘上仍为零（该断言保留） | 同上 |
  | 7 | lib 单测 `src/storage/page_format/overflow.rs` `free_overflow_chain_returns_pages_for_reuse`（释放后立即分配取回同一批页） | 释放后立即可分配 | 释放后立即分配**不得**取回这些页；执行**与 checkpoint 钩子同一条**「提升待释放集合」的存储层入口后，分配**必须**取回同一批页（该层无 `Database`，因此持久化边界用存储层的提升入口表达，不引入 `Database::checkpoint()`） | 同上 |

  另需在 `tests/free_page_lifecycle_test.rs` 汇总端到端回归断言：崩溃场景补齐 `check_integrity` 的 error 计数为 0 与重开后旧表行的可读性断言（T4 用例 3 已含，汇总时不得弱化）。
- Preserve: 每处断言的**断言语义与强度**（不得把「等于」改成「小于等于」来让增长通过，不得删除断言，不得跳过用例，不得改小被观察的对象范围）；`test_cross_restart_after_drop_safe` 与 `tests/drop_persistence_test.rs` 的 11 例**不改变**（跨重启路径与崩溃语义不在本次分界内）；既有 helper（`page_count` 等）不新增指标。
- Forbidden: 不得让 Act 自行放宽断言或自行决定新旧期望（必须按上表逐处落实，并把「旧期望 → 新期望 → 依据」写入 Act Response）；不得改动产品代码来迁就断言；不得修改 `tests/` 之外的既有测试文件。
- First test: 变更前为 GREEN（当前实现下 7 处全部成立）。逐处修改后，**先观察 RED**（在 T5 已实现的前提下，改后的两段式断言中「未 checkpoint ⇒ 不占用被释放的页」一段应在 T5 完成后 GREEN，而「checkpoint 后 ⇒ 占用」一段也应 GREEN）——若出现 RED，说明 T5 的实现与本表的期望不一致，须返回 T5 或 Plan，不得放宽断言。
- GREEN condition: 7 处逐处落实且 `cargo test --test drop_table_free_test -- --test-threads=1`、`cargo test --test overflow_page_test -- --test-threads=1`、`cargo test --lib free_overflow_chain_returns_pages_for_reuse -- --test-threads=1` 全部 `0 failed`。
- Verification: 上述三条命令 ⇒ 各自 `0 failed`；Act Response 中逐处给出「旧期望 → 新期望 → 依据哪条 requirement」的对照表。
- Stop when: 某处断言在 T5 完成后仍无法按上表落实（说明 T5 的语义与设计不符）时返回 Plan，不得放宽断言。

### T7: 本 change 的收尾全量回归（M19 分批串行，逐条写死纪律）

- Requirement/Scenario: 本 change 的收尾验证（`tasks.md`「收尾验证：M19 分批全量回归」）；不直接对应单条 requirement，为全 change 的行为回退面（唯一的行为变更面：会话内复用时机）提供回归证据。
- Depends on: T4, T5, T6
- Targets: 全量测试目标（`tests/*.rs` 全部文件 + `--lib --bins`）
- Current behavior: 本 change 尚未跑过全量；最近一次全量基线为 2026-10-24 MS34 收尾实例（134 个目标 / 13 批 / 2171 passed / 0 failed / 2 ignored，2 个 ignored 为 `cli_test.rs` 的长期 `#[ignore]` 用例）。
- Required behavior（纪律逐条写死，SHALL）:
  1. 目标清单 = `ls tests/*.rs | sort`（**文件名字典序**，当前 134 个文件，含本 change 新增的 `tests/free_page_lifecycle_test.rs`）+ `--lib --bins` **单独成一批** ⇒ 135 个目标。
  2. 切分：`tests/*.rs` 按字典序每批 8–12 个目标（确定性算法：剩余 ≤ 12 则取剩余，否则取 11）。
  3. 每批一次**独立**调用：`cargo test --test <f1> --test <f2> ... --test <fk> -- --test-threads=1`；`--lib --bins` 批为 `cargo test --lib --bins -- --test-threads=1`。
  4. 批间**严格串行**，同一时刻只有一批在跑。
  5. **SHALL NOT** 用一次无目标过滤的全量调用跑完（无论串行还是默认并行度）。
  6. **SHALL NOT** 用默认并行度跑任何全量批次（每批必须带 `--test-threads=1`）。
  7. 每批判定 = cargo 自己的**退出码为 0** 且该批 `test result:` 行的 **failed 计数为 0**；计数可用一步通用管道读出（`… 2>&1 | grep -E "^test result:"`），管道**只计数、不判定**。
  8. **不得**新增判定脚本、封装命令、专用参数或判定层；**不得**引入身份证明机制（hash/指纹/revision pin/run-id/manifest/时间顺序证明）与 timer 阈值。
  9. 逐批记录 passed / failed / ignored 计数，并做**覆盖对账**：目标清单不重不漏（分批数 × 每批目标数 = 135）。
  10. 全量 Gate 等价于「**全部批次各自 0 failed**」；缺一批不得声明全量通过。
  11. 单次目标验证（`cargo test --test <name>`）不受本纪律约束，可用默认并行度。
  12. 配方参照 `.agents/runbooks/m19-batched-full-regression.md`（R48）。
- Preserve: 计数与判定只用 cargo 的原生输出；不改测试代码来让全量通过；不提交、不清理工作区的既有现场。
- Forbidden: 不得用一次无过滤的全量调用代替分批；不得并行跑多批；不得在缺批的情况下声明全量通过；不得把 T7 的结果用于替代 T4–T6 的定向验证。
- First test: 不适用（非测试驱动的任务；前置条件是 T4–T6 的定向验证已全部 GREEN）。
- GREEN condition: 全部批次退出码 0 且各自 failed 计数为 0；覆盖对账无遗漏无重复。
- Verification: 逐批记录（批号、目标清单、退出码、passed / failed / ignored）；对照目标清单做覆盖对账；最终结论写成「N 批 / M 目标 / X passed / 0 failed / Y ignored」。
- Stop when: 任一批 failed 计数非 0 时，按 `tasks.md` 的 Diagnostic boundary 定位（待释放集合入队 → 释放完成编号推进 → checkpoint 第 2 步钩子取样 → 刷盘成功提升 / 失败丢弃 → 释放点的完成标记），修复后**只重跑受影响批次**，并在记录中说明；连续同批 3 次失败按公共规则 › 三次失败 停止并返回 Plan。

**Cycle RTM**

| Requirement / Scenario | Task | First Test | Status |
|---|---|---|---|
| FPL R1 场景 1（释放未持久化前页不可分配） | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::released_page_not_allocatable_before_release_durable` | Planned |
| FPL R1 场景 2（释放持久化后可分配） | T4, T5 | `tests/free_page_lifecycle_test.rs::released_page_allocatable_after_release_durable` | Planned |
| FPL R3（重建的失败安全：不得把活页判成可分配页） | T5（保持 Iteration 000 的成果不变） | `tests/free_page_lifecycle_test.rs::rebuild_failure_keeps_live_pages_intact` | 既有（Iteration 000 已验收） |
| FPL R4（崩溃后未完成的释放不是可分配页） | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` | Planned |
| DTPF R（MODIFIED：可分配时机受释放的持久化判定约束） | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::released_page_not_allocatable_before_release_durable` + `tests/drop_table_free_test.rs`（7 处更新后的断言） | Planned |
| DPB R1（MODIFIED：窗口归属移交 FPL 域） | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` + 既有 `tests/drop_persistence_test.rs`（11 例） | Planned |
| 收尾全量回归 | T7 | `cargo test --lib --bins -- --test-threads=1` + 分批 `tests/*.rs` | Planned |

命名说明：矩阵中的测试名是**计划中的用例名**；Act 可在不改变断言语义的前提下调整命名，并在 Act Response 记录实际名称。

**Invariants**

- 跨重启维度：可分配页集合的唯一来源仍是**持久目录**（Iteration 000 的重建），本 Cycle 不引入第二份真相、不持久化空闲页结构。
- 活页绝不被发放：任何机制调整都不得把仍被目录或链登记的页变成可分配页（FPL R3）。
- 释放动作本身不写零页（零页写入仍只发生在分配时）。
- 会话内复用的时机是本 change 唯一的行为回退面；其余既有语义（崩溃后旧表可见、同会话读不到已释放页的旧行、跨重启重建与查询可读）保持不变。
- 文件格式与伴生文件不变；加密库的页读写语义不变（重建与提升都走既有 `write_page` 通道）。
- `AsyncStorage` 的必实现方法集合不变；`MemoryStorage` 与既有测试替身零改动。

**Non-goals**

- 释放路径的有界读取改造（损坏页上的 DROP/TRUNCATE/DROP INDEX panic 面）——范围外，按 M20 移交。
- 尾部截断与整库整理；空闲页的持久化结构（路线 (b)）；无键行旧版本 GC（I038）；MS36 的历史库兼容面；ISS22–ISS25 的修复；MS34 三域语义；`TRUNCATE` 自身的既有崩溃语义（`truncate-table` 域记录的「部分清空」形态）。

**Acceptance**

- **A1（释放未持久化前不可分配）**：DROP TABLE（以及 TRUNCATE / DROP INDEX / GC 溢出释放）之后、释放尚未被一次在释放完成之后开始且成功完成的 checkpoint 持久化之前，被释放的页不可被同一会话的新对象占用（新建对象取到的是新页；库文件页数上界增长）。⇒ FPL R1 场景 1；DTPF R（MODIFIED）。
- **A2（释放持久化后可分配）**：上述 checkpoint 成功完成后，被释放的页立即可被同一会话的新对象占用（新建对象取到被释放的页；库文件页数上界不增长）。⇒ FPL R1 场景 2；DTPF R（MODIFIED）。
- **A3（崩溃语义）**：DROP TABLE 成功后（未 checkpoint）按 R49 配方崩溃 → 重新打开：旧表仍存在且已提交行可查询、崩溃前创建的新表不存在、`check_integrity` 的 error 级发现数为 0。⇒ FPL R4；DPB R1（MODIFIED）。
- **A4（刷盘失败不提升）**：checkpoint 刷盘失败时待释放项被丢弃（不提升），数据库保持一致且 `check_integrity` 的 error 级发现数为 0。⇒ FPL R1 的失败安全侧；T5 要求 3。
- **A5（既有断言分界更新）**：T6 表中的 7 处逐处落实「旧期望 → 新期望 → 依据哪条 requirement」，断言语义与强度不减；既有 `tests/drop_persistence_test.rs` 11 例与 `test_cross_restart_after_drop_safe` 不改变。⇒ DTPF R（MODIFIED）。
- **A6（收尾全量回归）**：T7 的分批全量回归全部批次退出码 0 且 failed 计数为 0，覆盖对账无遗漏无重复。⇒ 本 change 的收尾验证（`tasks.md`「收尾验证」）。
- **A7（内存库形态）**：`:memory:` 存储的释放后立即可分配行为不变（一个用例钉住）。⇒ `in-memory-database` 域不冲突。

**Verification**

| Scenario | 判定方式 | 通过依据 |
|---|---|---|
| A1 | `cargo test --test free_page_lifecycle_test -- --test-threads=1`（用例 `released_page_not_allocatable_before_release_durable`） | 该用例 `0 failed`；断言直接观察「新建对象未占用被释放页 + 文件页数上界增长」 |
| A2 | 同上命令（用例 `released_page_allocatable_after_release_durable`） | `0 failed`；断言直接观察「新建对象占用被释放页 + 文件页数上界不增长」 |
| A3 | 同上命令（用例 `crash_with_pending_release_keeps_old_table_rows`） | `0 failed`；断言直接观察「旧表行可查 + 新表不存在 + `check_integrity` error 计数为 0」 |
| A4 | 同上命令（用例 `flush_failure_does_not_promote_pending_release`） | `0 failed`；断言直接观察「释放页未被提升 + error 计数为 0」 |
| A5 | `cargo test --test drop_table_free_test -- --test-threads=1`、`cargo test --test overflow_page_test -- --test-threads=1`、`cargo test --lib free_overflow_chain_returns_pages_for_reuse -- --test-threads=1` | 三条命令各自 `0 failed`；Act Response 给出 7 处逐处对照表 |
| A6 | T7 的分批命令（见 T7 要求 3） | 每批退出码 0 且 `test result:` 行 failed 计数 0；覆盖对账 135 个目标不重不漏 |
| A7 | `cargo test --test free_page_lifecycle_test -- --test-threads=1`（内存库用例） | `0 failed`；断言直接观察内存库的释放页立即可分配行为未变 |

全部判定落在 `cargo test` 的断言内；计数只用一步通用管道；无身份证明机制、无框架外判定层、无 timer 阈值。

**Gate 2 Readiness**

PASS —— Step 5 计划审查：PASS（Iteration 001 的目标、范围、依赖、requirement、验证方式与验收边界沿用 `tasks.md` 已获批的 Iteration Plan，本次展开未作实质变更，判定不构成 `replan-required`）；RTM：无 `Missing`、无 `Simplified`；设计闭合：PASS（释放时机语义、失败语义、持久化边界、存储层接缝形态由 `design.md` D2.3–D2.7 确定，本 Plan Context 补齐释放点全集与锚点写规则，不改设计）；与 change 产物一致：PASS（与 `tasks.md` T4–T7、`design.md`、三份规格增量一致）；Persisted Evidence 模式：`none`（判定全部由 `cargo test` 承载，可低成本重跑，无「一次性环境即将消失 / 需保留现场 / 摘要丢失决定性结构」情形）；用户批准：PASS（change 计划已在 Gate 1 获批；本 Iteration 的展开授权来自 goal 指令「把已经规划的三个 ms 逐步完成」对阶段推进的显式授权，且补充 2 明确要求实施中发现的 issue 候选走承载流程修复）。无 `BLOCKED`、无 `SKIPPED`。

**Persisted Evidence**

- Mode: none

理由：全部验收结论由 `cargo test` 的真实运行承载（每个用例的断言直接对应 A1–A7），可低成本重跑；无「结果无法低成本复现 / 一次性环境即将消失 / 需保留现场 / 摘要会丢失决定性结构」情形。不创建 `evidence/<iteration>/<cycle>/` 目录。

**Risks and Notes**

- **范围外的实质缺陷（Issue 候选，不在本 change 修复）**：释放路径（`drop_table` / `truncate_table` / `DROP INDEX`）的 best-effort 扫描使用无检查的 `SlottedPageRef::get_slot_data`（`src/storage/page_format/slotted_page.rs:128`），在损坏页上会越界切片 panic（进程 abort）。裁定为**本 change 范围外**：按 M20「发现方只报告与移交，不在当前 change 内扩范围修复；落在存储/索引层的由存储域后续 change 承载」，且本 change 的 `proposal.md` 影响面未包含释放路径的读健壮性，Scope Control 下不属完成本 Iteration 结果的必要条件。承载建议：由 `openspec-experience-recorder` 登记为 Issue，再由存储域后续 change 承载（与 MS35 的文件面相邻，建议排在 MS35 之后，避免与 T5 的释放点改动并发冲突）。
- **非实质未知项（不阻塞，交由 Act 在实现中确定）**：待释放集合与释放完成编号的具体存放形态、存储层新增 trait 方法的命名与拆分方式、释放完成标记在各释放点的具体落点——均属等价实现选择，不改变可观察语义；Act 可自行决定并在 Act Response 记录。
- **需要在实现中显式裁定的两点（非实质，但不得留给 Act 猜）**：(1) `:memory:` 存储的适用性——已在本 Plan Context 裁定为「闸门不适用于内存库，维持既有行为」，T5 须按此实现并用一个用例钉住；(2) B 树内部释放与恢复期旧树释放的锚点写归属——已在本 Plan Context 的 Current-State Evidence 表格中给出，T5 须逐点落实并在 Act Response 记录。
- **性能**：本 Cycle 不改变打开期重建的复杂度（O(活页数)，Iteration 000 Review 的 m3）；待释放集合与空闲集合的内存占用随会话释放量线性增长，属设计 D2.3 的既定形态。
- **既有现场**：工作区含 MS34 与 issue-fix change 的未提交改动，属既有现场，不得回滚、清理或提交。

## Act Response

- Status: reported

### Implemented

#### T4 —— 先行 RED 用例（`tests/free_page_lifecycle_test.rs`，新增 4 例）

| 用例 | 钉住的要求 | 可观察量 |
|---|---|---|
| `released_page_not_allocatable_before_release_durable` | FPL R1 场景 1 | `DROP TABLE` 后**未 checkpoint**：新建表的数据页号 ≠ 被释放页号，且库文件页数上界增长 |
| `released_page_allocatable_after_release_durable` | FPL R1 场景 2 | 一次**成功 checkpoint** 后：新建表的数据页号 ∈ 被释放页集合，且库文件页数上界不增长 |
| `crash_with_pending_release_keeps_old_table_rows` | FPL R4 + DPB R1（MODIFIED） | 崩溃后旧表仍存在且已提交行可查询；未完成的释放**不是**可分配页（重开后的新表不占用旧表的页）；`check_integrity` 的 error 级发现数为 0 |
| `flush_failure_does_not_promote_pending_release` | FPL R1 场景 3（失败安全） | `arm_io_failure(IoFailPoint::DataWrite)` 使 checkpoint 刷盘失败：失败后同一会话新建表不占用被释放的页；库仍可写可查；`check_integrity` 的 error 级发现数为 0 |

四个用例全部走真实入口（真实文件库 + `Database::open` / `checkpoint` / `close`），崩溃用例按 R49 配方（`wal_buffer.shutdown()` 后 `drop(db)`，不调用 `close()`）；表规模为小表（不触发 ISS23）。

#### T5 —— 释放的持久化判定实现

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/storage/async_storage.rs` | `trait AsyncStorage` | 新增 **4 个带默认实现**的 trait 方法：`release_completion_limit()`、`complete_free_release(&[PageId])`、`promote_free_releases(limit)`、`discard_free_releases(limit)`。**必实现方法集合不变**（`read_page` / `write_page` / `allocate_page` / `free_page` / `sync` / `page_count`）⇒ `MemoryStorage` 与既有测试替身**零改动** |
| `src/storage/file_storage.rs` | `FileStorage`（字段 `pending_frees` / `release_id`）、`free_page`、4 个新 trait 方法 | `free_page` 改为把页记入**待释放集合**（不再直接进可分配集合，且仍不写零页）；`complete_free_release` 只完成**本次释放的页**（未完成的释放任何时刻都不可提升）；`promote_free_releases` / `discard_free_releases` 按 checkpoint 取样的编号提升或丢弃 |
| `src/storage/buffer_pool.rs` | `BufferPool::complete_free_release` | 接缝：把释放完成标记转给存储层 |
| `src/storage/btree/sync_loader.rs` | `SyncPageLoader::complete_free_release` | 同步侧接缝（B 树在 `spawn_blocking` 内运行） |
| `src/wal/checkpoint.rs` | `CheckpointManager::checkpoint` | 第 2 步刷盘**之前**取样释放完成编号；刷盘**成功**⇒提升；刷盘**失败**⇒丢弃并原样返回错误；第 0 步零工作早退也提升（无脏页 ⇒ 已发出的锚点写都已落盘） |
| `src/storage/data/table_manager.rs` | `drop_table` / `truncate_table`（步骤 1 与步骤 4 + 树构建失败清理）/ `create_table` 失败清理 | 4 处释放完成标记 |
| `src/executor/drop_index.rs` | `DropIndexExecutor::execute` | 1 处释放完成标记（锚点写 = `take_secondary_index` 的目录摘除） |
| `src/executor/create_index.rs` | `CreateIndexExecutor::execute` | 1 处释放完成标记（回填失败清理，无锚点写） |
| `src/storage/page_format/overflow.rs` | `free_overflow_chain` / `write_overflow_chain` 失败清理 | 各 1 处释放完成标记（`free_overflow_chain` 一处覆盖 GC 路径、`data_page.rs:81`、`recovery.rs:214` 三个调用方） |
| `src/storage/btree/btree.rs` | `merge_leaves` / `handle_child_merge` / `merge_internal_nodes` | 3 处释放完成标记（锚点写 = 父节点分隔键移除，同一写单元内） |
| `src/storage/btree/index_manager.rs` | `IndexManager::delete` | 1 处释放完成标记（**缩根**：锚点写 = 目录 root 更新之后） |
| `src/wal/recovery.rs` | 恢复期旧索引树释放 | 1 处释放完成标记（锚点写 = 步骤 4 的目录 root 更新） |

**释放点覆盖对照**（Plan Context「释放点全集」逐类 → 完成标记落点；`complete_free_release` 的调用点即覆盖点）：

| 释放点 | 释放位置 | 锚点写 | 完成标记落点 |
|---|---|---|---|
| DROP TABLE | `table_manager.rs:1479` | 目录删除（`:1414`，**先于**释放） | `table_manager.rs:1487` |
| TRUNCATE 步骤 1（溢出链页 + 尾数据页） | `table_manager.rs:1851` / `:1860` | 表头原地重置 + 目录尾指针更新（**晚于**释放） | `table_manager.rs:1879`（步骤 2 之后） |
| TRUNCATE 步骤 4（旧索引树） | `table_manager.rs:1964` | 步骤 3 的目录 root 更新 | `table_manager.rs:1980` |
| TRUNCATE 树构建失败清理 | `table_manager.rs:1940` | 无（未登记） | `table_manager.rs:1947` |
| 建表失败清理 | `table_manager.rs:626` | 无（未登记） | `table_manager.rs:632` |
| DROP INDEX | `drop_index.rs:63` | 目录摘除（**先于**释放） | `drop_index.rs:81` |
| CREATE INDEX 回填失败清理 | `create_index.rs:84` | 无（未登记） | `create_index.rs:90` |
| GC 清理旧版本（含溢出链） | `data_page.rs:249` → `overflow.rs:263` | 旧版本 slot 移除的数据页写（**先于**释放） | `overflow.rs:268` |
| 溢出链写入失败清理（`data_page.rs:81`） | `overflow.rs:263` | 无 | `overflow.rs:268` |
| 溢出链写入失败清理（`overflow.rs:142`） | `overflow.rs:142` | 无 | `overflow.rs:147` |
| 恢复期 redo 失败溢出释放 | `recovery.rs:214` → `overflow.rs:263` | 无 | `overflow.rs:268` |
| 恢复期旧树释放 | `recovery.rs:1378/1389/1401` | 步骤 4 的目录 root 更新 | `recovery.rs:1413` |
| B 树叶合并 | `btree.rs:610` | 父节点分隔键移除（`:612-618`） | `btree.rs:622` |
| B 树子合并处理 | `btree.rs:672` | 父节点分隔键移除（`:658-666`） | `btree.rs:675` |
| B 树内节点合并 | `btree.rs:877` | 父节点分隔键移除（`:870-876`） | `btree.rs:889` |
| B 树缩根 | `btree.rs:781` | 目录 root 更新（`IndexManager::delete`） | `index_manager.rs:461`（目录更新之后） |

⇒ Plan Context 的释放点全集 **16 类全部被完成标记覆盖**（Plan 的「六个释放点」+ Plan Context 追加的 B 树 4 处、建表/树构建失败清理 2 处、恢复期 2 处、TRUNCATE 步骤 4 1 处）。

#### T6 —— 既有断言的期望更新（15 处逐处对照）

| # | 位置 | 旧期望 | 新期望 | 依据 |
|---|---|---|---|---|
| 1 | `tests/drop_table_free_test.rs::test_simple_drop_releases_data_and_btree` | 释放后立即复用 ⇒ `count_after_recreate == count_after_create`（"recreating users must reuse freed pages (free-list) without growing the file"） | 拆两段：**(a) 未 checkpoint** ⇒ 新建表不占用被释放的页（页数上界增长、新表页号 ≥ 释放前的高水位）；**(b) checkpoint 之后** ⇒ 新建表占用被释放的页（页数上界不增长、新表数据页号 ∈ 被释放页集合） | DTPF R（MODIFIED）+ FPL R1 场景 1/2 |
| 2 | `tests/drop_table_free_test.rs::test_long_data_page_chain_all_released` | 同上（`count_after == count_filled`） | 同 1 的两段式；checkpoint 后页数不增长且新表页号 < 释放前高水位（即取自被释放的页） | 同上 |
| 3 | `tests/drop_table_free_test.rs::test_btree_height_gt_1_all_pages_released` | 同上（`count_after == count_filled`） | 同 1 的两段式；checkpoint 后页数不增长且新表页号 < 释放前高水位 | 同上 |
| 4 | `tests/drop_table_free_test.rs::test_same_process_free_list_reuse` | 释放后立即复用 ⇒ 增长 ≤ 1 页（`count_after <= count_before_drop + 1`） | **(a) 未 checkpoint** ⇒ 旧上限不再成立（增长 > 1 页）且新表页号 ≥ 释放前高水位；**(b) checkpoint 之后** ⇒ 旧上限恢复（增长 ≤ 1 页，基线重锚到 (b) 段重建之前） | 同上 |
| 5 | `tests/drop_table_free_test.rs::test_concurrent_drop_different_tables` | 同 1（`count_after_recreate == count_after_drop`） | 同 1 的两段式；checkpoint 后 10 张新表页数不增长且页号 < 释放前高水位 | 同上 |
| 6 | `tests/overflow_page_test.rs::truncate_releases_overflow_chain_pages_keeps_definition` | TRUNCATE 后新树根取自被释放的链页 | **(a) 未提升**：重建的 PK 根**不**取自链页；链页在磁盘上仍为零（该断言保留）；**(b) 提升之后**：被释放的链页可分配（分配取回该链页集合） | 同上 |
| 7 | `src/storage/page_format/overflow.rs` lib 单测 `free_overflow_chain_returns_pages_for_reuse` | 释放后立即分配取回同一批页 | 释放后立即分配**不得**取回这些页；执行**与 checkpoint 钩子同一条**存储层提升入口后，分配**必须**取回同一批页 | 同上 |
| 8 | `tests/free_page_lifecycle_test.rs::same_session_release_still_reuses_pages`（改名为 `same_session_release_reuses_pages_after_release_durable`） | 同会话释放后立即复用（MS33 语义「本 Cycle 不改释放时机」） | 未 checkpoint ⇒ 不复用（页数增长、新表页号 ≥ 释放前高水位）；checkpoint 之后 ⇒ 复用（页数不增长、新表页号 ∈ 被释放页集合） | 同上（Iteration 000 的用例钉的正是本次被修改的语义） |
| 9 | `tests/overflow_page_test.rs::gc_releases_oversized_version_chain` | GC 后分配即取回被释放的链页 | (a) 未提升 ⇒ 分配到的是 fresh 页（非链页）；(b) 提升之后 ⇒ 分配取回同一批链页 | 同上 |
| 10 | `tests/overflow_page_test.rs::drop_table_releases_overflow_chain_pages` | drop 后分配即取回「数据页 + 索引页 + 链页」 | (a) 未提升 ⇒ 分配到的是 fresh 页；(b) 提升之后 ⇒ 分配取回同一批页 | 同上 |
| 11 | `tests/btree_merge_test.rs::test_free_page_reuse` | 释放的页立即被复用（"Freed page must be reused"） | (a) 未提升 ⇒ 释放的页不被复用；(b) 完成 + 提升之后 ⇒ 被复用 | 同上 |
| 12 | `tests/storage_test.rs::test_free_page_reuse` | 释放的页立即被复用且文件页数不变（`page_count() == 3`） | (a) 未提升 ⇒ 释放的页不被复用；(b) 完成 + 提升之后 ⇒ 被复用，且页数在复用前后不变 | 同上 |
| 13 | `tests/storage_test.rs::test_free_page_zeroed` | 释放的页立即被复用且被清零 | (a) 未提升 ⇒ 释放的页不被复用；(b) 完成 + 提升之后 ⇒ 被复用且**仍**被清零（弹出即写零页的语义保留） | 同上 |
| 14 | `tests/constraint_enforcement_test.rs::drop_table_with_unique_index_releases_pages_for_reuse` | drop 后同进程新建表可写且页数不增长（`pages_after <= pages_before`） | (a) 未 checkpoint ⇒ 页数上界增长；(b) checkpoint 之后 ⇒ 页数不增长（基线重锚到 (b) 段新建表之前） | 同上 |
| 15 | `src/storage/page_format/overflow.rs` lib 单测 `write_failure_frees_partially_allocated_chain` | 写入失败清理掉的页立即被复用（"cleanup must have freed the partially allocated chain page"） | (a) 未提升 ⇒ 清理掉的页不被复用；(b) 完成 + 提升之后 ⇒ 被复用（测试替身 `FailAfterNAllocs` **零改动**，生命周期落在其内层 `FileStorage` 上） | 同上 |

每处均为「拆两段 + 追加页号断言」的加强形式：断言数量不减、强度不减（「等于」未被改成「小于等于」，无删除、无跳过）；#9/#10/#11/#12/#13/#15 的持久化边界用**存储层的提升入口**表达（该层无 `Database`），与 checkpoint 钩子调用同一条入口。

#### T7 —— 收尾全量回归（M19 分批串行）

批次计划（确定性算法：剩余 ≤ 12 取剩余，否则取 11；`--lib --bins` 单独成批）：`tests/*.rs` 文件名字典序 134 个文件 ⇒ 12 批 × 11 + 1 批 × 2；加 `--lib --bins` 一批 ⇒ **14 批 / 135 个目标**。

| 批 | 目标 | 退出码 | passed | failed | ignored |
|---|---|---|---|---|---|
| 1 | `--lib --bins` | 0 | 420 | 0 | 0 |
| 2 | aggregate_extension / aggregate_output_shape / aggregate / alter_table / attach_session / btree_merge / btree_scale / btree_split / btree / busy_timeout / cache_perf | 0 | 149 | 0 | 0 |
| 3 | catalog_durability / chain_join / check_command / checkpoint_command / checkpoint_redo_reduction / checkpoint_skip / checkpoint / cli / concurrent / connection_limit / constraint_enforcement | 0 | 210 | 0 | 2 |
| 4 | cross_db_dml / cross_db_query / ctas / cte / database_file_lock / datetime_function / datetime_type / ddl_catalog_transactional / ddl_crash_recovery / default_declaration / default_precheck | 0 | 140 | 0 | 0 |
| 5 | derived_scan_alias / dml_tx_id / drop_persistence / drop_table_free / e2e / encryption / executor / explain / explicit_tx / expression_e2e / file_header | 0 | 191 | 0 | 0 |
| 6 | file_storage_io / free_page_lifecycle / full_table_dml / gc / group_by_expr / import_input_surface / index_manager / index_write_atomicity / insert_column_list / insert_select / isolation_level | 0 | 88 | 0 | 0 |
| 7 | join_output_resolution / join / key_type_conformance / keyless_eq_routing / keyless_row / limit / memory_db / mvcc_abort / mvcc_commit / mvcc_record / mvcc_tombstone_visibility | 0 | 97 | 0 | 0 |
| 8 | nested_loop_join / network_protocol / network_server / no_from_select / order_by_alias_sort / outer_join / overflow_page / param_binding / parser / pg_integration / pg_messages | 0 | 122 | 0 | 0 |
| 9 | pg_protocol / pipeline / plan_cache / plan_exec / planner / predicate / prefetch / prepared_statement / projection_expression / projection / pushdown | 0 | 224 | 0 | 0 |
| 10 | query_memory / readonly_session / recovery_e2e / recovery_error_surface / recovery_replay_convergence / recovery_tail_persistence / recovery / recovery_wal_convergence / runtime / scalar_function / scalar_subquery_comparison | 0 | 102 | 0 | 0 |
| 11 | schema_persistence / secondary_index_equality / secondary_index / select_distinct / self_join / set_operations / sort / sql_differential / statement_cancellation / storage / structural_property | 0 | 129 | 0 | 0 |
| 12 | subquery_alias_binding / subquery / subset_insert / sync_loader / table_alias_resolution / table_manager / truncate / tx_statement / typed_api / update_expression / update_index_maintenance | 0 | 174 | 0 | 0 |
| 13 | upsert / value / version_chain / view / visibility / wal_auto_checkpoint / wal_buffer / wal_handle / wal_integration / wal_record / wal_recovery_large | 0 | 120 | 0 | 0 |
| 14 | wal_writer / write_type_conformance | 0 | 17 | 0 | 0 |
| — | **合计** | 全部 0 | **2183** | **0** | **2** |

- 纪律核对：每批一次独立调用且带 `--test-threads=1` ✓；批间严格串行（同一时刻只有一批在跑）✓；未使用任何无目标过滤的全量调用 ✓；未使用默认并行度 ✓；判定只看 cargo 退出码与 `test result:` 行（计数用一步管道 `awk` 读出，管道只计数不判定）✓；无判定脚本 / 封装命令 / 专用参数 / 框架外判定层 / timer 阈值 / 身份证明机制 ✓。
- 覆盖对账：目标清单 135（134 文件 + `--lib --bins`）⇒ **missing 0 / extra 0 / duplicate 0**（134 个文件各出现恰好一次）。
- 总数对账：基线 **2171 / 0 / 2** → 本次 **2183 / 0 / 2**，差额 **+12** = Iteration 000 新增 `tests/free_page_lifecycle_test.rs` 的 8 例 + T4 新增 4 例（改名与断言更新不改变用例数）。
- 2 个 ignored 仍是 `cli_test.rs` 的长期 `#[ignore]` 用例（`calibration_recovery_time`、`diagnostic_wal_parse`），与本 change 无关。

### Deviations from Plan

1. **释放完成标记的粒度由「全局计数推进」改为「按本次释放的页」**（`complete_free_release(&[PageId])`，design D2.3 的字面形态为无参 `complete_free_release()`）。理由：Plan Context 的 T5 要求 1 与 5 要求「释放在崩溃边界上成立之前不得可分配」是**无条件**的；全局计数模型在两个释放操作交错时会提前提升未完成的释放（另一操作的完成推进计数即使其条目可被样本覆盖），而本项目**没有语句级 DDL 串行化**（`src/pipeline.rs` 无语句闸门）⇒ 全局模型只能依赖「释放操作不交错」这一未写下的假设。按页完成后，未完成的条目不带完成编号，任何样本都不会提升它。D2.3 的其余形态（待释放集合 + 单调完成编号 + checkpoint 第 2 步取样 / 成功提升 / 失败丢弃）保持不变。
2. **checkpoint 第 0 步的零工作早退也做提升**（design D2.3 把钩子只挂在第 2 步）。理由：零工作早退的前提正是「池无脏页」⇒ 已发出的锚点写都已落盘 ⇒ 该批释放在崩溃边界上成立；若不提升，它们要等到下一次全量 checkpoint 才可分配，而释放早已持久化（违反 T5 要求 4 的前半句），且「DROP → checkpoint → 新建表」这一常见路径会多涨库文件（MS33 行为的回退）。T5 要求 4 的后半句（零工作早退时页不得被永久搁置）随之满足。
3. **T6 的更新处由计划表的 7 处增至 15 处**：#8–#15 是全量回归实测暴露的**同一语义变化**（断言「释放后立即可分配」）的其余落点——计划表（源自 `design.md` D2.6，早于本 Cycle 的实现细节）未列出它们；这些改动**没有超出**该语义变化（每一处都是同一「两段式 + 存储层提升入口」的形式），其中 #8 是 Iteration 000 自己新增的用例（它钉的正是本次被修改的语义）。
4. **T4 崩溃用例的「崩溃前创建的新表不存在」一项不可与「旧表仍存在」同时成立**：`CREATE TABLE` 的语句边界有序刷盘（`src/storage/catalog.rs` 的 `flush_commit_point`）会把 `CREATE` 的提交点**与其它脏页**落盘——先建新表则新表必然存在，后建新表则先前的 `DROP TABLE` 目录删除被一并刷盘（旧表不再存在）。故该用例改以 FPL R4 的直接可观察量承载：「重开后的新表不占用旧表的页」（未完成的释放不是可分配页）+ 旧表已提交行可查询 + `check_integrity` error 计数 0。
5. **T6 两段式的页数基线重锚**：计划表把 (b) 段的比较对象写成释放前的计数（如 `count_after_recreate == count_after_create`）。但计划表同时要求 (a) 段「文件页数上界增长」，两者在同一会话内不可兼得（文件长度不回落）。故 (b) 段的基线取「(b) 段新建表之前」的计数，并**追加**页号断言（新建表页号 < 释放前高水位 ⇒ 取自被释放的页）——属性（复用不增长）与强度均不减。
6. **TDD 顺序偏离（过程）**：实现先于用例落笔。RED 证据以「临时还原变更前的释放路径」（`FileStorage::free_page` 直接 push 进可分配集合 = MS33 行为）复现，失败点落在**可分配时机**断言上；随后恢复实现观察 GREEN。
7. **完成标记的落点**由 Act 自行决定（Plan Context 明示为非实质未知项），实际落点见上表（16 类释放点全覆盖）。

### Blocker Handoff

None required.

### Blocker Resolution

None（未发生阻塞）。

### Self-Review

**Gate 3（逐任务即时 Review）**

| 任务 | 结论 | 依据 |
|---|---|---|
| T4 | PASS | 4 个新用例的断言逐条对应 T5 的 4 条可观察要求；RED 落在可分配时机断言上；与 T1–T3 的 8 个既有用例同时 GREEN |
| T5 | PASS | 6 条可观察要求逐条核对（见下）；16 类释放点的完成标记全覆盖；`AsyncStorage` 必实现方法集合不变；`MemoryStorage` 与既有测试替身零改动；`allocate_page` 的「弹出即写零页」不变；`free_page` 的对外语义（页不再被任何链引用）不变；Iteration 000 的重建与「取代」接缝不变；文件格式与伴生文件不变；`drop_persistence_test` 11 例未改 |
| T6 | PASS | 15 处逐处落实，断言未放宽；既有 helper 未新增指标；`test_cross_restart_after_drop_safe` 与 `drop_persistence_test` 11 例未改 |
| T7 | PASS | 14 批全部退出码 0 且 failed 计数 0；覆盖对账 missing 0 / extra 0 / duplicate 0；总数与基线差额 +12 逐项归因 |

T5 的 6 条要求逐条结论：要求 1（未持久化前不可分配）由 `released_page_not_allocatable_before_release_durable` 钉住；要求 2（持久化后可分配）由 `released_page_allocatable_after_release_durable` 钉住；要求 3（刷盘失败不提升）由 `flush_failure_does_not_promote_pending_release` 钉住（失败后新建表不占用被释放页 + 库仍可用 + error 计数 0）；要求 4（不得被永久搁置）由「零工作早退也提升」+「会话结束由重开重建回收」共同满足；要求 5（不得提前提升）由「按页完成 + 第 2 步前取样 + 完成标记在锚点写之后」保证，并由 `crash_with_pending_release_keeps_old_table_rows` 作为回归护栏；要求 6（内存库形态不变）由 `MemoryStorage` 零改动 + 新增 trait 方法全部带默认实现保证，并由既有 `in_memory_database_is_unaffected` 用例钉住。

**自检发现与处理**

- 发现 1（已修复）：T6 的 (b) 段 checkpoint 在纯 DDL 会话里命中第 0 步零工作早退（DDL 的有序刷盘已把脏页落盘）⇒ 释放未被提升 ⇒ (b) 段的文件不增长断言失败（`drop_table_free_test` 4 例 + `constraint_enforcement_test` 1 例首次运行失败）。处理：按 T5 要求 4 的前半句给零工作早退加提升（见 Deviations 2），修复后全部 GREEN。
- 发现 2（已修复）：`drop_table_free_test` 的 (b) 段页号断言最初按「{数据页头, 索引树根}」做集合成员判断，但空闲表是**后进先出**弹出（先弹出链尾页）⇒ 成员断言不成立。处理：改为「新建表页号 < 释放前页数高水位」（= 该页必然取自被释放的页，因为空闲表此刻只含被释放的页）+ 文件不增长断言，等价且更稳。
- 发现 3（已修复）：T4 崩溃用例最初写成「DROP → 新建表 → 崩溃」，结果重开时旧表不存在——DDL 的语句边界有序刷盘把 `DROP` 的目录删除一并落盘。处理：改为两会话形态（会话一先让已提交行落盘并截断 WAL，会话二 DROP 后崩溃），与 `drop_persistence_test` 的既有形态一致；并把不可兼得的那一项换成 R4 的直接可观察量（见 Deviations 4）。
- 遗留 Minor（不阻塞）：`tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` 的注释记录了「崩溃前创建的新表不存在」在 DDL 有序刷盘下不可兼得的事实，供后续读者判读；无其它遗留 Minor。

**Gate 4（Task Definition 的验证逐项）**

| 验证（Task Definition） | 结果 |
|---|---|
| T4/T5：`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 0 failed | ✅ 12 passed / 0 failed / 0 ignored（8 既有 + 4 新增） |
| T5：`cargo test --test drop_persistence_test -- --test-threads=1` ⇒ 0 failed（11 例崩溃语义护栏） | ✅ 11 passed / 0 failed / 0 ignored |
| T6：`cargo test --test drop_table_free_test -- --test-threads=1` ⇒ 0 failed | ✅ 6 passed / 0 failed / 0 ignored |
| T6：`cargo test --test overflow_page_test -- --test-threads=1` ⇒ 0 failed | ✅ 27 passed / 0 failed / 0 ignored |
| T6：`cargo test --lib free_overflow_chain_returns_pages_for_reuse -- --test-threads=1` ⇒ 0 failed | ✅ 1 passed / 0 failed（419 filtered out） |
| T7：全部批次退出码 0 且各自 failed 计数 0 | ✅ 14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored |

**RED → GREEN 证据（T4/T5）**

- RED（变更前行为 = MS33 的「释放后立即可分配」，以临时还原 `FileStorage::free_page` 的直接 push 复现）：
  `cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ `test result: FAILED. 9 passed; 3 failed`（复现时的用例数为 12，中间调整过崩溃用例形态）。
  - `released_page_not_allocatable_before_release_durable` FAILED @ `tests/free_page_lifecycle_test.rs:379`：`assertion left != right failed: the new table must not take the released data page 2 / left: 2 / right: 2` ⇒ 失败落在**可分配时机**断言上 ✓
  - `flush_failure_does_not_promote_pending_release` FAILED @ `:530`：同一形态的可分配时机断言 ✓
  - `crash_with_pending_release_keeps_old_table_rows` 在该状态下**平凡通过**（崩溃后旧表行因目录未落盘而仍在）⇒ 作为回归护栏 ✓（与 Plan Context 的预判一致）
- GREEN（实现后）：`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ `test result: ok. 12 passed; 0 failed; 0 ignored` ✓

### Verification Evidence

见上「Gate 4」与「T7」两表的命令与实际计数；全部结论来自当次 `cargo test` 的真实运行输出（无框架外判定层、无身份证明机制、无 timer 阈值）。本轮未运行无目标过滤的全量调用。

### Persisted Evidence

None required —— 全部验收结论由 `cargo test` 的真实运行承载，可低成本重跑；无「结果无法低成本复现 / 一次性环境即将消失 / 需保留现场 / 摘要会丢失决定性结构」情形（与 Plan Context 的 `Mode: none` 一致）。未创建 `evidence/001-durable-release-gate/000-initial/` 目录。

### Experience Candidates

1. **Issue 候选 —— B 树叶合并路径的重复释放**（当前 change 范围外的实质缺陷）：`src/storage/btree/btree.rs:610` 的 `merge_leaves` 释放右叶页并把它作为 `MergeInfo.freed_page_id` 返回，随后 `handle_child_merge`（`:672`）**再次**释放同一页 ⇒ 同一页号被两次推入空闲表 ⇒ `FileStorage::allocate_page`（`src/storage/file_storage.rs:448`，无去重的 `Vec::pop`）可能把同一页交给两个对象。证据：代码路径自 `merge_leaves` 的 `free_page(right_id)` 到 `handle_child_merge` 的 `free_page(merge_info.freed_page_id)`（`MergeInfo.freed_page_id == right_id`）；本 change 的待释放集合同样保留两条记录（提升后仍会两次进入可分配集合），即**行为与变更前一致**（变更前空闲表亦被推入两次）。承载建议：由 `openspec-experience-recorder` 登记为 Issue，由存储/索引域的后续 change 修复（去重或消除重复释放），**不在本 change 内扩范围修复**。
2. **Runbook 候选 —— M19 分批全量回归的批计划更新**：本实例为 134 个 `tests/*.rs` + `--lib --bins` = **135 个目标 / 14 批**（12 批 × 11 + 1 批 × 2 + lib/bins 批），合计 **2183 passed / 0 failed / 2 ignored**；可追加到 `.agents/runbooks/m19-batched-full-regression.md`（R48）作为 2026-10-24 MS35 实例的批计划与对账记录（由 `openspec-experience-recorder` 执行）。
3. （沿用 Plan Context 已记录的候选，未新增证据：释放路径 best-effort 扫描使用无检查的 `SlottedPageRef::get_slot_data` 的有界读取缺陷——按 M20 移交存储域后续 change。）

### Remaining Issues

- 上述 Issue 候选 1（B 树重复释放）为**当前 change 范围外**的既有缺陷，未在本 change 内修复；它不违反本 change 的任何 requirement（释放时机语义与重复释放计数是两个正交问题），但影响空闲表的完整性，移交承载流程处理。
- 本 Cycle 未产生 Blocker；无 `SKIPPED` 项。

## Plan Review

- Review Result: replan-required

**Findings**

本节按「独立核对结论 → 7 条 deviation 的裁定 → 15 处断言更新的裁定 → 规格增量的核对 → 身份机制与判定层 → T7 记录核对」组织。所有结论基于独立阅读实际代码与 diff、逐处核对 Act Response 的声明；Act 的 Self-Review 只作输入，不作依据。

*A. 独立核对结论（实现层）*

1. **闸门机制与 Plan Context 一致**：`src/wal/checkpoint.rs` 的第 2 步在 `flush_all()` **之前**取样 `release_completion_limit()`，成功 → `promote_free_releases(limit)`，失败 → `discard_free_releases(limit)` 并返回原始错误；第 0 步零工作早退也提升。`src/storage/file_storage.rs` 的 `free_page` 只把 `(page, None)` 入待释放集合（不写零页），`complete_free_release(&[PageId])` 按**本次释放的页**标记完成编号，`promote_free_releases` 只提升「完成编号 < 样本」且已标记的条目，未完成条目（`None`）任何时刻都不提升 ⇒ 与 T5 的要求 1/2/5 一致。
2. **接口形态合规**：`src/storage/async_storage.rs` 新增 4 个方法（`release_completion_limit` / `complete_free_release` / `promote_free_releases` / `discard_free_releases`）**全部带默认实现**，必实现方法集合不变；`MemoryStorage` 与既有测试替身零改动（`git status` 中无相应文件变更）⇒ T5 的 Preserve 成立；内存库维持「释放后立即可分配」（默认实现即空语义），A7 由 `in_memory_database_is_unaffected` 钉住。
3. **16 类释放点的完成标记全覆盖**：独立 grep `complete_free_release` 的产品代码调用点 = `table_manager.rs:632/1487/1879/1947/1980`、`drop_index.rs:81`、`create_index.rs:90`、`overflow.rs:147/268`（覆盖 GC 路径 `data_page.rs:249`、溢出链写入失败 `data_page.rs:81`、恢复期 redo 失败 `recovery.rs:214`）、`recovery.rs:1413`、`btree.rs:622/675/889`、`index_manager.rs:461` ⇒ 与 Act 的 16 行覆盖表逐行对应。**两处锚点写顺序已独立核对**：`index_manager.rs:461` 的标记在 `sync_root_to_catalog(new_root_id)`（目录 root 更新）**之后**；`recovery.rs:1413` 的标记在步骤 4 的目录 root 更新**之后**；`table_manager.rs:1879` 的标记在表头原地重置 + 目录尾指针更新**之后**；`table_manager.rs:1980` 的标记在步骤 3 的目录 root 更新**之后** ⇒ 「完成标记在锚点写已发出之后」成立（T5 的实现指导 2）。
4. **崩溃用例的判别力**：`crash_with_pending_release_keeps_old_table_rows` 断言三条——`t` 的已提交行在重开后逐行可查、释放的页（数据页 + 索引树根）不被重开后的新表占用、`check_integrity` error 计数 0。三条均直接对应 FPL R4 的 requirement 正文（未完成持久化的释放所涉及的页不是可分配页 / 持久目录仍登记的对象其已提交内容可查 / error 计数 0）。关于「提升被破坏时能否失败」的核对结论：该用例在**同会话**的提升时机上判别力有限（跨崩溃后待释放集合已随进程消失，其判定依据是持久目录的活页枚举，属 Iteration 000 的重建语义）；提升时机的判别力由 `released_page_not_allocatable_before_release_durable`、`flush_failure_does_not_promote_pending_release`、`same_session_release_reuses_pages_after_release_durable` 三例承担 ⇒ R4 的 requirement 正文与 R1 的场景 1/2 各自有判别力，无覆盖缺口。

*B. 7 条 deviation 的裁定*

| # | Act 的偏差 | 裁定 | 理由（独立核对） |
|---|---|---|---|
| ① | 释放完成标记由「全局计数推进」改为「按本次释放的页」 | **接受（Plan/设计错误的纠正，需求语义不变）** | design D2.3 的字面模型把「释放编号」记在 `free_page` 时（`pending_frees` 元素 = (当前释放编号, 页号)），两个释放操作交错时无法区分条目归属：操作 X 的条目与其后完成的操作 Y 共享同一编号，Y 的完成会推进计数，使 X（锚点写尚未发出）的条目被样本覆盖 ⇒ 提前提升。该推理**成立**，且本项目确有并发释放：`tests/drop_table_free_test.rs::test_concurrent_drop_different_tables` 并发 drop 10 张表。Act 的按页集实现消除了该窗口：未完成的条目不带完成编号，任何样本都不提升它；完成编号由 `fetch_add` 原子分配，取样与完成在并发下仍然自洽（样本之后的完成拿到 ≥ 样本的编号）。FPL R1 的 requirement 正文只约束「释放在崩溃边界上成立之后才可分配」，未规定计数粒度 ⇒ **不改变已批准 requirement 的语义**，属设计缺陷在 Cycle 内的等价纠正。 |
| ② | 第 0 步零工作早退也做提升（design D2.3 只把钩子挂在第 2 步） | **接受（字面偏离 FPL R1 的定义，实质成立）** | 推理链成立：零工作早退的前提是「池无脏页」，而锚点写是对池内页的改写（`catalog.rs:787` `delete_from_chain` 经 `get_page` + `modify_page` 使页变脏），因此「无脏页」蕴含「已发出的锚点写已被某次成功刷盘写盘」——使其落盘的可以是 DDL 语句边界有序落盘（`catalog.rs:271` `flush_all_except` 写入除提交页外的全部脏页）。故早退时的提升在**正常路径**上正确，且避免了「DROP → checkpoint → 新建表」多涨库文件。字面偏离：FPL R1 把「成立」定义为「已被一次在释放完成之后开始且成功完成的 **checkpoint** 刷脏页写入数据文件」，早退的 checkpoint 本身没有刷盘。**边界情形**：若一次失败的刷盘（`buffer_pool.rs` 的 `flush_all` 先清脏标记再写，失败页不再重试）丢掉了锚点写，该页在池中呈「干净」，随后的成功刷盘不会重写它，而提升仍会发生 ⇒ 存在一条窄的提前提升路径（见下方 Issue 候选 2）。该路径需要 I/O 失败恰好落在 DDL 边界刷盘上，且不改变本 Cycle 的任何验收结论 ⇒ 判为**非阻塞 Minor finding + Issue 候选**。 |
| ③ | T6 的更新处由计划的 7 处增至 15 处 | **接受** | 逐处核对（见 C）：#1–#7 与计划表一一对应，#8–#15 是同一语义变化（断言「释放后立即可分配」）在其余落点的回归发现，全部落在「会话内复用时机」这一唯一行为变更面内，无一处超出该语义。 |
| ④ | 崩溃用例的「崩溃前创建的新表不存在」不可与「旧表仍存在」同时成立 | **事实成立，但处置不完整 ⇒ 阻塞项** | 事实核对成立：`CREATE TABLE` 的语句边界有序落盘（`catalog.rs:226` `insert_table` → `:259` `flush_commit_point` → `:271` `flush_all_except`）会写入除提交页外的全部脏页，先前的 `DROP TABLE` 目录删除因而随之落盘；`delete_table` 自身**不**做语句边界落盘（`catalog.rs:787` 只把页改脏）。独立探针（Review 期一次性，已删除、工作区已复原）实测：`DROP t → CREATE u → 崩溃` ⇒ 重开后 `t` 不存在、`u` 存在；`DROP t → 崩溃` ⇒ 重开后 `t` 存在。Act 用「未完成的释放不是可分配页」承载 R4 requirement 正文的方向正确（requirement 正文三项均被用例断言），**但两份规格增量的场景文本仍写着旧观察量**（FPL R4 场景的「随后同一会话创建新表 `u` 并分配页」+「`u` 不存在（其目录未持久化）」；DPB R1（MODIFIED）场景的同一 WHEN +「重新打开后 `t` 仍存在」）⇒ 原样合并会把实现不具备的行为写进规格库。 |
| ⑤ | T6 两段式的页数基线重锚 | **接受** | 计划表自身要求 (a) 段「文件页数上界增长」与 (b) 段「页数回到释放前的计数」在同一会话内不可兼得（文件长度不回落）；Act 取「(b) 段新建表之前」的计数并**追加**页号断言（新建表页号 < 释放前高水位 ⇒ 取自被释放的页），属性（复用不增长）不减、强度增加。 |
| ⑥ | TDD 顺序偏离：实现先落笔，RED 由「临时还原变更前的释放路径」复现 | **不构成阻塞项；但过程偏离成立，须记录并纠正** | 证据链核对：RED 复现时 `9 passed; 3 failed`，失败点落在**可分配时机**断言上（`released_page_not_allocatable_before_release_durable` @ `:379` 与 `flush_failure_does_not_promote_pending_release` @ `:530`，均为 `left: 2 / right: 2` 的「新建表占用了被释放的页」），不是编译错误、不是 panic；崩溃用例平凡通过与 Plan Context 的预判一致。**判别力真实**：断言值取自 Plan Context 在实现之前写死的 Required behavior（用例名 + 观察量），不是从实现反推；「临时还原变更前路径」是一种变异式取证，能证明断言对被改动的语义敏感。**过程偏离成立**：AGENTS.md 执行约束 7「没有先行测试不变更」被违反（用例未先于实现落笔）。**裁定**：不阻塞——完成声明的证据链（可观察断言在变更前失败、变更后通过）成立且可复现；偏离按 M20 记录并要求**后续 change 恢复「先写用例」的顺序**（本 Cycle 的 T8 属覆盖补强，不适用）。 |
| ⑦ | 完成标记的落点由 Act 决定 | **接受** | Plan Context 明示为非实质未知项；独立核对逐点确认标记都在锚点写之后（见 A.3）。 |

*C. 15 处断言更新的逐处裁定（关键项）*

逐处核对「旧期望 → 新期望 → 依据」，并**排查是否有更新超出「会话内复用改为 checkpoint 后可分配」这一语义变化，或掩盖了别的问题**：

| # | 位置 | 裁定 | 依据与强度核对 |
|---|---|---|---|
| 1 | `drop_table_free_test.rs::test_simple_drop_releases_data_and_btree` | 合法 | (a) 段断言新表不占用被释放的页 + 页数上界增长；(b) 段断言页数不增长 **且** 新表数据页/索引根 ∈ 被释放页集合 ⇒ 旧属性（「重建复用释放页且文件不增长」）不减反增（加了页号断言）。旧基线 `count_after_create` 换为 (b) 段前的计数属必要重锚（文件长度不回落），非放宽。 |
| 2 | `::test_long_data_page_chain_all_released` | 合法 | 同 1 的两段式；旧 `count_after == count_filled` → (b) 段基线 + `t_meta.data_page_head.0 < high_water` ⇒ 直接证明取自被释放的页。 |
| 3 | `::test_btree_height_gt_1_all_released` | 合法 | 同上。 |
| 4 | `::test_same_process_free_list_reuse` | 合法 | (a) 段断言旧上限（增长 ≤ 1 页）**不再**成立且新表不占用被释放的页；(b) 段恢复旧上限（基线重锚到 (b) 段重建之前）⇒ 旧属性保留、语义边界清晰。 |
| 5 | `::test_concurrent_drop_different_tables` | 合法 | (a) 段 10 张新表页号 ≥ 高水位 + 文件增长；(b) 段 10 张新表页号 < 高水位 + 页数不增长（10 表需 20 页 ⇒ 等价于旧断言的「空闲表至少含 20 个不同页」强度）。 |
| 6 | `overflow_page_test.rs::truncate_releases_overflow_chain_pages_keeps_definition` | 合法（**加强**） | 旧期望「新 PK 根 ∈ 被释放链页」反转为「∉」（未持久化不得取用）；旧断言里对 `new_pk_root` 的 `continue` 跳过被**删除** ⇒ 现在要求全部链页在磁盘上为零（truncate 既有清零落盘语义）⇒ 断言数不减、强度增加。 |
| 7 | `overflow.rs` lib 单测 `free_overflow_chain_returns_pages_for_reuse` | 合法 | 「立即分配不得取回」+「提升后必须取回同一批」⇒ 旧期望（立即取回同一批）在持久化边界后仍被断言。 |
| 8 | `free_page_lifecycle_test.rs::same_session_release_still_reuses_pages` → 改名 `same_session_release_reuses_pages_after_release_durable` | 合法 | 改名与期望同步；旧期望钉的正是本次被修改的语义（Plan Context 已列为 T6 目标）⇒ 属同一语义变化。 |
| 9 | `overflow_page_test.rs::gc_releases_oversized_version_chain` | 合法 | (a) 段 fresh 分配不含链页 + (b) 段分配取回同一批链页。 |
| 10 | `overflow_page_test.rs::drop_table_releases_overflow_chain_pages` | 合法 | (a) 段 fresh 不含被释放集合 + (b) 段「分配取回的集合 == 被释放集合」（旧断言保留）。 |
| 11 | `btree_merge_test.rs::test_free_page_reuse` | 合法 | 旧断言「Freed page must be reused」改为「持久化前不得复用」+「完成并提升后必须复用」⇒ 旧期望在边界后仍被断言。 |
| 12 | `storage_test.rs::test_free_page_reuse` | 合法 | `page_count() == 3` 重锚为「复用前后页数不变」（旧绝对值在 (a) 段多分配一页后不再成立）；「reused == PageId(1)」保留。 |
| 13 | `storage_test.rs::test_free_page_zeroed` | 合法 | 「立即复用且被清零」→ (a) 段不复用 + (b) 段复用**且仍被清零**（弹出即写零页语义保留）。 |
| 14 | `constraint_enforcement_test.rs::drop_table_with_unique_index_releases_pages_for_reuse` | 合法 | (a) 段页数上界增长 + (b) 段旧属性 `pages_after <= pages_before`（基线重锚到 (b) 段）保留。 |
| 15 | `overflow.rs` lib 单测 `write_failure_frees_partially_allocated_chain` | 合法 | (a) 段清理掉的页不复用 + (b) 段复用；测试替身 `FailAfterNAllocs` 零改动（生命周期落在内层 `FileStorage`）⇒ 与「必实现方法集合不变」一致。 |

⇒ **15 处全部落在「会话内复用时机」这一唯一行为变更面内**，无一处放宽断言、无删除、无跳过；#6、#1–#5、#9–#15 的页号断言与两段式结构使强度不减反增；基线重锚（#1–#5、#12、#14）均由「文件长度不回落 / 持久化边界前多分配一页」这一物理事实强制，且每处都补了可判定的页号断言以替代被重锚的绝对值。**未发现任何一处掩盖了别的问题**。

*D. 规格增量核对（即将合并进 `openspec/specs/`）—— 阻塞项*

- `drop-table-physical-free`（MODIFIED，2 条 requirement / 5 个场景）：逐条与已验收用例对应（`drop+recreate 同名表复用 free list` ↔ `drop_table_free_test` (b) 段；`释放未持久化前不复用` ↔ (a) 段；`restart after drop 不 panic` ↔ 既有 `test_cross_restart_after_drop_safe`（未改）；`restart after drop+recreate 正常工作` ↔ 同用例；`restart after drop 复用释放的页` ↔ Iteration 000 的 `reopened_database_reuses_released_pages`）⇒ **全部已验收，无未验收行为被写成已验收**，与既有 75 域无冲突。
- `free-page-lifecycle`（新增域，R1–R4）：R1（场景 1/2）、R2（3 场景）、R3（1 场景）、R4（1 场景）均有对应用例；**R4 场景的文本与实现相反**（见 B.④ 与下方 Acceptance Gaps）⇒ 阻塞。
- `drop-persistence-boundary`（MODIFIED，R1）：三个既有场景（DROP TABLE / DROP INDEX / DROP VIEW 后崩溃）由既有 `tests/drop_persistence_test.rs` 11 例覆盖 ✓；**新增/修改的场景「释放未持久时的分配不摧毁旧表内容」与 R1 的 requirement 前置条件在 MS35 的闸门下出现缺口**（见下）⇒ 阻塞。

*缺口的技术内容*：DPB R1（MODIFIED）的 requirement 前置条件是「此后没有成功完成 checkpoint、也没有页分配复用其释放的页」。MS33 语义下「DROP 后同会话建新表」会复用被释放的页 ⇒ 该形态被前置条件排除，requirement 的结论（旧对象仍可见）从不与之冲突；MS35 的闸门使新表**不再**复用被释放的页 ⇒ 前置条件成立，而该形态下旧对象**并不**可见（其目录删除已被后续 DDL 的有序落盘写盘）⇒ requirement 正文与场景在 MS35 之后不再自洽。故修正不是「放宽断言」，而是把 requirement 的崩溃点与场景的可观察量对齐到实现已经具备的形态。

*E. 身份证明机制与判定层核对*

`src/` 与 `tests/` 的改动中未出现 hash / 指纹 / revision pin / run-id / session id / manifest / 时间顺序证明；`src/storage/free_page_rebuild.rs` 的一次性故障注入点按库路径限定且只用于制造遍历失败（Iteration 000 引入，非本次）；全部判定落在 `cargo test` 的断言内；T7 的计数只用一步通用管道读出且管道只计数不判定；无 timer 阈值作行为判据（`arm_io_failure` 是写入前的一次性失败注入，属目标行为的故障注入面，不是判据）⇒ **无身份证明机制、无框架外判定层**。

*F. T7 全量回归记录核对（只做覆盖性与结构核对，未重跑）*

- 批计划：`ls tests/*.rs | sort` 当前 **134** 个文件；按「剩余 ≤ 12 取剩余，否则取 11」切分为 12 批 × 11 + 1 批 × 2，加 `--lib --bins` 一批 ⇒ **14 批 / 135 目标** ✓。独立复算：把 `ls tests/*.rs | sort` 的实际输出按 11 个一组重排，与 Act 记录的批次逐一对应（批 2 = aggregate_extension…cache_perf、批 3 = catalog_durability…constraint_enforcement、…、批 14 = wal_writer + write_type_conformance）⇒ **批次划分与文件名字典序一致，每批 8–12 个目标（实际 11 或 2）** ✓。
- 覆盖对账：134 个文件在批次表中各出现**恰好一次**（missing 0 / extra 0 / duplicate 0）✓；无目标过滤的全量调用；每批一次独立调用且带 `--test-threads=1`；批间严格串行 ✓（Act 记录与批次表结构自洽）。
- 计数自洽：各批 passed 相加 = 420+149+210+140+191+88+97+122+224+102+129+174+120+17 = **2183** ✓ 与合计行一致；failed 全 0；ignored 2（批 3 的 `cli_test.rs`）✓。
- 差额归因：基线 2171 / 0 / 2（2026-10-24 MS34 收尾实例）→ 本次 2183 / 0 / 2，差额 **+12** = 本 change 新增的 `tests/free_page_lifecycle_test.rs` 的 12 例（Iteration 000 的 8 例 + T4 的 4 例）✓ 自洽；既有文件的改动只改断言、未增删用例 ✓（与 C 的逐处核对一致）。
- **不重跑的理由**：被覆盖的产品代码自结论产生后未变化（工作区 `src/` 变更集与 Act 报告一致，无提交、无回滚），不构成 AGENTS.md › 验证列出的重跑情形；且本 Review 的结论不依赖全量重跑。

*G. 非阻塞 Minor finding 与 Issue 候选（只报告不登记，按 M20）*

1. **未完成释放在「DDL 边界刷盘失败」路径上的提前提升面**（偏差 ② 的边界情形）：`buffer_pool.rs` 的 `flush_all` 先清脏标记再写盘，失败页的内容不会重试；若该次失败发生在 DDL 语句边界的有序落盘上，先前 `DROP TABLE` 的锚点写（目录删除）可能丢失且该页在池中呈「干净」，随后任一次成功刷盘仍会按「释放完成编号 < 样本」提升该释放 ⇒ 持久目录仍登记旧主人而其页已可分配（窄窗口，需 I/O 失败恰好落在该处）。**不阻塞**本 Cycle 的验收（正常路径与已覆盖的 checkpoint 刷盘失败路径均正确），作为 Issue 候选移交。承载建议：存储域后续 change（需决定「锚点写丢失」如何传播到待释放集合，例如让存储层感知非 checkpoint 刷盘的失败）。
2. **B 树叶合并路径的重复释放**（Act 已报告，本次独立核对确认）：`btree.rs:610` `merge_leaves` 释放右叶页并作为 `MergeInfo.freed_page_id` 返回，`handle_child_merge`（`:672`）再次释放同一页 ⇒ 同一页号两次进入待释放集合、两次进入可分配集合，`allocate_page` 无去重 ⇒ 可能把同一页交给两个对象。**既有问题、本次改动未改变其行为**（变更前空闲表亦被推入两次）⇒ 不构成本 change 的 Acceptance Gap；作为 Issue 候选移交（承载建议：索引/存储域后续 change 去重或消除重复释放）。
3. **DDL 语句边界有序落盘会把先前未落盘的目录删除一并写盘**（本 Review 新发现的既有行为张力）：`catalog.rs:271` `flush_all_except` 写入除提交页外的全部脏页，因此 `DROP TABLE` 的 checkpoint 边界在后续任一 DDL 成功返回后不再成立；既有域 `wal-recovery-replay-integrity` 的「DROP … SHALL 保持 checkpoint 边界」在字面上仍成立（`DROP` 自身不把删除当 commit point），但其效果已被后续 DDL 越过。**既有问题、非本 change 引入**，作为 Issue 候选移交（承载建议：存储域后续 change 决定「DROP 的删除是否应随后续 DDL 落盘」或把既有域的表述与实现对齐）。
4. **释放路径 best-effort 扫描的有界读取缺陷**（沿用父 Cycle 的记录，未新增证据）。
5. **Runbook 候选**：R48（`.agents/runbooks/m19-batched-full-regression.md`）应补本次实例：135 目标 / 14 批（12×11 + 2 + lib/bins）/ 2183 passed / 0 failed / 2 ignored。

**Acceptance Gaps**

- **GAP-1（阻塞，A3 的第二项）**：父 Cycle Plan Context 的 A3 写明「崩溃前创建的新表不存在」必须成立。独立探针实测：`DROP t → CREATE u → 崩溃` 后重开 **`t` 不存在、`u` 存在**（DDL 语句边界有序落盘把先前的目录删除一并写盘）。该观察量与 A3 的第一项「旧表仍存在」**不可同时成立**，因此 A3 的第二项在当前实现下不可满足；Act 用 FPL R4 requirement 正文的可观察量（「未完成的释放不是可分配页」）替代，该替代**满足 requirement 正文**，但 A3 的字面验收边界未满足 ⇒ 属「验收边界需要改变」，不能留在当前 Cycle 用返工掩盖。
- **GAP-2（阻塞，规格增量）**：`free-page-lifecycle` R4 场景与 `drop-persistence-boundary` R1（MODIFIED）场景（「释放未持久时的分配不摧毁旧表内容」）的 WHEN/THEN 记录了实现不具备的行为（原样合并会向规格库写入与实现相反的断言）；DPB R1（MODIFIED）的 requirement 前置条件在 MS35 的闸门下不再排除「后续 DDL 成功返回」的形态，而该形态下旧对象不可见 ⇒ requirement 正文与实现之间出现缺口。
- GAP-1 与 GAP-2 同源（DDL 语句边界有序落盘与 MS35 闸门共同造成），由**一个** replan Cycle 关闭。
- 其余 Acceptance（A1、A2、A4–A7）经独立核对**已满足**：A1/A2 由 `released_page_not_allocatable_before_release_durable` / `released_page_allocatable_after_release_durable` 钉住；A4 由 `flush_failure_does_not_promote_pending_release` 钉住；A5 由 C 的 15 处逐处核对满足（强度不减）；A6 由 F 的批次结构与覆盖对账满足；A7 由 `in_memory_database_is_unaffected` 钉住且 `MemoryStorage` 零改动 ✓。

**Evidence**

- 代码与 diff（本 Review 独立阅读）：`src/storage/async_storage.rs`（4 个默认实现方法，必实现集合不变）、`src/storage/file_storage.rs`（`pending_frees` / `release_id` / `free_page` 入队 / `complete_free_release` 按页标记 / `promote_free_releases` / `discard_free_releases`）、`src/wal/checkpoint.rs`（第 2 步前取样、成功提升、失败丢弃、第 0 步早退提升）、`src/storage/buffer_pool.rs`（`flush_all` 先清脏标记再写；`flush_all_except`；`has_dirty_pages` 读路径）、`src/storage/data/table_manager.rs`（5 处完成标记与锚点写顺序）、`src/storage/btree/{btree,index_manager}.rs`、`src/storage/page_format/overflow.rs`、`src/wal/recovery.rs`、`src/executor/{drop_index,create_index}.rs`、`src/storage/btree/sync_loader.rs`、`src/storage/catalog.rs`（`insert_table` / `flush_commit_point` / `delete_from_chain`）、全部改动的测试文件、`tasks.md` / `design.md` / `proposal.md` / 三份增量。
- 一次性探针（Review 期临时文件 `tests/zz_review_probe_tmp.rs`，**已删除**，工作区已复原到 Review 进入时的状态；`git status --porcelain -- tests/` 与进入时一致）：`cargo test --test zz_review_probe_tmp -- --test-threads=1 --nocapture` ⇒ `2 passed; 0 failed`，输出 `PROBE after-crash: t_exists=false u_exists=true`（DROP → CREATE u → 崩溃）与 `PROBE drop-only-crash: t_exists=true`（DROP → 崩溃）。
- 沿用（来源：父 Cycle Act Response「Gate 3 / Gate 4 结论」与「Verification Evidence」；被覆盖的代码未变化）：`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 12 passed / 0 failed；`drop_persistence_test` 11、`drop_table_free_test` 6、`overflow_page_test` 27、lib 单测 2 处全 0 failed；RED ⇒ `9 passed; 3 failed`（失败落在可分配时机断言）；T7 全量 14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored。
- 覆盖性复算（本 Review）：`ls tests/*.rs | wc -l` = 134；`ls tests/*.rs | sort` 按 11 个一组重排与 Act 的批次表逐一对应；各批 passed 相加 = 2183。

**Follow-up Decision**

**不能 `accepted`**：GAP-1 使父 Cycle 的 A3 未被满足（其第二项不可满足），GAP-2 使两份即将合并的规格增量记录了与实现相反的行为。若在此状态下 `accepted` 并进入收尾，`openspec/specs/` 会承载「崩溃后新表不存在 / 旧表仍存在」这一实现不具备的组合，违反「规格库只保存验收过的行为」。

**不能 `rework-required`**：返工针对的是实现，而实现本身满足 FPL R1–R4 / DTPF / DPB 的 requirement 正文（15 处断言更新与 16 类释放点标记经独立核对成立）；缺口在**验收边界与规格文本**——A3 的第二项与两份场景的 WHEN/THEN 需要改写，而「验收边界需要改变」按 `references/iteration-planning.md` 的判定表属 `replan-required`（「目标、范围、依赖、requirement、设计、验证方式或验收边界需要改变」）。普通返工不得掩盖范围或验证方式的变化。

**判定 `replan-required`**，并按该引用的 replan 路径执行：① 按需更新 change 的 `specs`（两份增量的场景文本 + DPB R1（MODIFIED）的前置条件，已在本 Review 中完成改写，改写内容与依据见 `Iteration Plan Update`）与未完成的 Iteration Plan（`tasks.md`：新增 T8、Iteration 001 的 Tasks 与 Stable result 同步、RTM 后的「场景文本修正说明」）；② 在同一 Iteration 目录创建后继 replan Cycle（`002-replan.md`），其 Plan Context 自包含（Act 无需回读父 Cycle），承载 T8：崩溃用例补 `other` 表覆盖并与修正后的场景逐条对齐 + 定向验证 + 产品代码零改动核对；③ 当前 Cycle 的 Plan Context 不改写，父 Cycle 的实现与验证结论保持有效并被 replan Cycle 沿用。

**Iteration Plan Update**

| 变更点 | 变更前 | 变更后 | 依据 |
|---|---|---|---|
| FPL R4 场景的 WHEN / THEN | 「随后同一会话创建新表 `u` 并分配页，再丢弃句柄崩溃」+「重新打开后 `t` 仍存在且原行可查询」+「`u` 不存在（其目录未持久化）」 | 「此后没有其它 DDL 语句成功返回，再丢弃句柄崩溃（不调用 `close`）」+「重新打开后 `t` 仍存在且原行可查询，`other` 的行可查询」+「未完成持久化的释放所涉及的页不是可分配页（重开后的新表不占用 `t` 的数据页与索引树页）」+「`check_integrity` 的 error 级发现数为 0」；requirement 正文补一句说明崩溃点为何取在 DDL 边界之前 | 独立探针实测（`t_exists=false u_exists=true` / `t_exists=true`）+ `catalog.rs:259/271` 的有序落盘路径 + 父 Cycle 用例的既有断言 |
| DPB R1（MODIFIED）requirement 正文的前置条件 | 「此后没有成功完成 checkpoint、也没有页分配复用其释放的页时」 | 增补「、也没有其它 DDL 语句成功返回」，并补一句说明 DDL 边界有序落盘会把先前目录删除随之写盘 | MS35 闸门使「页分配复用」不再发生 ⇒ 原前置条件不再排除该形态；`catalog.rs:271` |
| DPB R1（MODIFIED）场景「释放未持久时的分配不摧毁旧表内容」 | 「同一会话创建新表并分配页，再丢弃句柄崩溃」+「重新打开后 `t` 仍存在且原行可查询」 | 与 FPL R4 场景同形的崩溃点与可观察量（含「释放所涉及的页不是可分配页」一条） | 同上 |
| change 的 `tasks.md` | T1–T7（Iteration 001：T4–T7，全部 done）；Iteration 001 的 Stable result 写「崩溃后旧表已提交行仍可查询，**新表不存在**，完整性无 error 级发现」 | 新增 **T8**（规格增量场景与验收边界对齐）；Iteration 001 的 Tasks = T4, T5, T6, T7, T8；Stable result 的崩溃可观察量改为「旧表已提交行仍可查询、未完成的释放所涉及的页不是可分配页、完整性无 error 级发现」并写明崩溃点的限定 | 本 Review 的 GAP-1 / GAP-2 |
| 目标 / 范围 / 依赖 / requirement 正文 / 设计 / 验证方式 | — | **均不变** | 本次修正只涉及场景实例的可观察量表述与一个前置条件限定词；`design.md` D2.3–D2.7 不受影响；验证方式仍是 `cargo test` 内的真实入口断言 |

**Next Cycle**

`iterations/001-durable-release-gate/002-replan.md`（同一 Iteration 目录内的 replan Cycle；承载 T8）。

**Next Iteration**

None —— 本 Iteration（001）在该 replan Cycle 获得 `accepted` 前保持未完成；其后本 change 无剩余 Iteration，收尾（合并三份规格增量进 `openspec/specs/`、同步全局 tasks / SNAPSHOT / M / R / I、归档 change）由 `openspec-docs-maintainer` 执行。

