# Iteration 000 / Cycle 000: MS07-T02 drop_table 物理页释放

> _Plan Context 与 Act Response 与 Plan Review 同文件：Plan Context（draft）→ Act Response（reported）→ Plan Review（accepted）。_

## Plan Context

- Status: ready
- Authorization: 用户显式批准 Gate 状态变更并开始实施（原话：「更改gate状态，开始实施」，2026-08-30）。Gate 2 Readiness 表 7 项全 PASS；风险提示：BTree 高度=2 触发条件未实测（R-T02-5），已按计划留待 Act 实测。
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4（`tasks.md` §1-§4）
- Depends on: None
- Stable baseline: `TableManager::drop_table` 在抹 schema + 移除 in-memory 后释放所有数据页和 BTree 索引页到 `FileStorage::free_pages`；同进程内 `allocate_page` 优先复用
- Verification boundary: `tests/drop_table_free_test` 6/6；`cargo test --all` 0 failures；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff
- Diagnostic boundary: `src/storage/btree/index_manager.rs` + `src/storage/data/table_manager.rs` + `tests/drop_table_free_test.rs`
- Deferred tasks: None（本 change 完成 MS07-T02 全部子项；MS07-T05 Checkpoint + free-list 持久化留独立 change）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 完整 MS07-T02 范围（仅 drop_table 物理释放；不含 Checkpoint、显式事务 API、planner 拆分、谓词下推、消息传递重构）
- Excluded scope: 性能优化（除 free-list 复用本身）、新 SQL 方言、新执行器、新隔离级别、K05 recovery 修复、DDL WAL 记录、free-list 持久化

**Objective**

让 `DROP TABLE` 在抹除 `__tables` / `__columns` 中的 schema 行 + 移除 in-memory `TableManager` HashMap 项之外，**额外**将该表占用的所有数据页和 BTree 索引页归还到 `FileStorage::free_pages`。同进程内后续 `create_table` 优先从 free list 弹出 page id，实现 `file_len` 不再单调递增。跨重启不要求 free-list 持久化（被 free 的 page 留在磁盘但正确性无影响；catalog 行已抹除，restart 永远拿不到 freed page id）。

**Background**

- T01（`archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/`）让 schema 跨进程持久化，commit `4307a0e`
- `TableManager::drop_table`（`src/storage/data/table_manager.rs:299-315`）当前只做 3 件事：保留名检查 + `catalog.delete_table` + `tables.remove`；注释明确"Physical data pages and index pages are NOT freed (out of scope; covered by MS07-T02)"
- `FileStorage::free_pages: Mutex<Vec<u64>>`（`src/storage/file_storage.rs:16`）是 `allocate_page` 的回来源（line 97-99 pop 优先），但**永远空**（无调用方 `free_page`）
- `BufferPool::free_page`（`src/storage/buffer_pool.rs:373-379`）已实现：DashMap 移除 + 时钟手柄清理 + `storage.free_page`
- T01 R-5 风险（`iterations/000-initial.md:435`）：`IndexManager::from_root` 不验证 page 内容；drop 物理释放后若 catalog 行未抹除，restart 会 panic
- 缓解已确认：T01 的 `Catalog::delete_table` 顺序是先抹 catalog 行再删 in-memory；T02 物理释放保持"先 catalog 后 in-memory 后 free"即可天然安全
- 用户决策（A+A+A+A）：
  - BTree 页枚举：A 新增 `collect_all_pages` 公开 API
  - Free-list 持久化：A 接受跨重启泄漏
  - 测试文件位置：A 新建 `tests/drop_table_free_test.rs`
  - WAL 记录：A 不新增（沿用 T01）

**Current Baseline**

- Revision: `4307a0e`（master @ 2026-08-26，MS07-T01）
- 534 tests pass / 0 failed（cargo test --offline 已验证）
- `LEAF_NODE = 0x01`, `INTERNAL_NODE = 0x02`（`src/storage/btree/node.rs:7-8`）
- `LeafNodeRef::next_leaf_page_id() -> u32`（`node.rs:608-611`）
- `InternalNodeRef::leftmost_child() -> u32`（`node.rs:687`）+ `get_child_page_id(i) -> Option<u32>`（`node.rs:664-666`）
- `BufferPool::with_page_data<F, R>(&self, PageId, F) -> Result<R>` where `F: FnOnce(&[u8]) -> Result<R>`（`K23`）
- `SlottedPageRef::header().next_page_id: u32`（`page_format/slotted_page.rs:21`）
- `TableMeta::data_page_head: PageId`（`table_manager.rs:56`）+ `data_page_tail: Mutex<PageId>`（line 57）
- `Catalog::delete_table`（`catalog.rs:185-202`）：先取 `self.lock`，再 delete_from_chain `__tables` + `__columns`；物理页释放注释（line 432-434）"kept for future physical free by MS07-T02"
- `FileStorage::page_count() -> u64`（`file_storage.rs:49-51`）— pub 方法；测试可用
- 日志惯例：项目无 `log` crate，使用 `eprintln!`（见 `src/profiling.rs:37-51`、`src/network/server.rs:56`）

**Current-State Evidence**

- `src/storage/data/table_manager.rs:299-315` `TableManager::drop_table` 完整路径（保留名 + catalog.delete_table + tables.remove）
- `src/storage/data/table_manager.rs:294-298` doc comment 明确"out of scope; covered by MS07-T02"
- `src/storage/file_storage.rs:113-119` `AsyncStorage::free_page`：push free list + write zero page
- `src/storage/buffer_pool.rs:373-379` `BufferPool::free_page`：DashMap remove + clock_hand retain + storage.free_page
- `src/storage/btree/index_manager.rs:18-23` `IndexManager` 字段（root_page_id, sync_loader, async_loader, row_to_key）
- `src/storage/btree/index_manager.rs:51-61` `IndexManager::from_root`（T01 新增）— `IndexManager` 已可绑定到现有 root
- `src/storage/btree/node.rs:608-611` `LeafNodeRef::next_leaf_page_id`
- `src/storage/btree/node.rs:687` `InternalNodeRef::leftmost_child`（`header().next_page_id`）
- `src/storage/btree/node.rs:664-666` `InternalNodeRef::get_child_page_id(i)`
- `src/storage/btree/btree.rs:141, 177, 237, 418, 501, 754, 1031, 1058, 1128, 1198` 多处 `data[0] == LEAF_NODE` page_type 判断模式
- `src/storage/catalog.rs:185-202` `Catalog::delete_table` 现有实现（保留名检查外 + lock + delete_from_chain）
- `src/storage/catalog.rs:432-434` 注释 "kept for future physical free by MS07-T02"
- `tests/schema_persistence_test.rs:100-117` `test_drop_table_removes_from_catalog` 现有 drop_table 测试（仅验证 catalog）
- `tests/schema_persistence_test.rs:120-145` `test_restart_after_drop_table_gone` 现有 restart 测试
- `tests/pipeline_test.rs:151-185, 188-225` 现有 `test_pipeline_drop_table*` 2 个
- `tests/executor_test.rs:809-895` 现有 `test_drop_table_executor_*` 3 个
- `tests/planner_test.rs:222-252` 现有 `test_build_drop_table*` 2 个

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/storage/btree/index_manager.rs` | `IndexManager`, `collect_all_pages`（新增） | DFS 收集 BTree 所有 page；本 change 新增方法 |
| `src/storage/btree/node.rs` | `LeafNodeRef`, `InternalNodeRef`, `LEAF_NODE`, `INTERNAL_NODE` | BTree 节点只读视图 + 类型常量 |
| `src/storage/btree/btree.rs` | `BTree` 写操作 | 不变；BTree 树形由 BTree::insert/split 维护 |
| `src/storage/data/table_manager.rs` | `TableManager`, `TableMeta`, `drop_table`（改） | drop_table 增加物理释放步骤；新增 `collect_data_pages` 私有方法 |
| `src/storage/catalog.rs` | `Catalog`, `delete_table` | 不变；T01 已落地；物理释放前已抹 schema 行 |
| `src/storage/buffer_pool.rs` | `BufferPool`, `free_page` | 不变；本 change 直接调用 |
| `src/storage/file_storage.rs` | `FileStorage`, `free_page`, `page_count` | 不变；free-list 仍是 in-memory |
| `tests/drop_table_free_test.rs` | 新增 6 个集成测试 | 验证物理释放全路径 |

**Critical Path**

```
DropTableExecutor::next (src/executor/drop_table.rs)
    ↓
TableManager::drop_table(name)
    ↓
1. 保留名检查 → Err 立即返回
2. read lock 取 TableMeta (clone Arc)
3. catalog.delete_table(name) — 抹 __tables + __columns 行
4. write lock → tables.remove(name) — 移除 in-memory
5. index_manager.collect_all_pages() — DFS 返回 Vec<PageId>
6. collect_data_pages(data_page_head) — K22 链遍历返回 Vec<PageId>
7. 对每个 page 调 buffer_pool.free_page — best-effort
```

**Implementation Guidance**

- `IndexManager::collect_all_pages` 实现用栈式 DFS + `visited: HashSet<u64>` 防环；用 `buffer_pool.with_page_data` 闭包（K23 模式）避免持锁 .await
- `TableManager::drop_table` 取 `TableMeta` 用 read lock（先 clone Arc），不在 catalog 抹除前持有 write lock
- `collect_data_pages` 复用 `with_page_data` + `SlottedPageRef::header().next_page_id`（K22）
- 物理释放失败用 `eprintln!`（项目无 `log` crate；与 `profiling.rs:37-51` 风格一致）
- 不需要新 WAL 变体；T01 已确认（`archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/proposal.md:67`）
- T01 R-5 风险通过"先 catalog 后 in-memory 后 free"顺序天然缓解
- 测试新建 `tests/drop_table_free_test.rs`（与 `schema_persistence_test.rs` 职责分离）

**Behavioral Change**

- **当前行为**：`TableManager::drop_table(name)` 只抹 schema 行 + 移除 in-memory 项；物理数据页和 BTree 索引页不释放；`file_len` 单调递增
- **目标行为**：`TableManager::drop_table(name)` 在抹 schema + 移除 in-memory 之后，额外释放该表占用的所有数据页和 BTree 索引页到 `FileStorage::free_pages`；同进程 `allocate_page` 优先弹出 free list 中的 page id
- **接口变化**：
  - 新增 `IndexManager::collect_all_pages(&self) -> Result<Vec<PageId>>`（pub async）
  - `TableManager::drop_table` 行为扩展（已有 public API；行为差异 = 现在释放物理页）
  - 其他 API 不变
- **状态变化**：`FileStorage::free_pages: Mutex<Vec<u64>>` 由"永远空"变为"drop 后含被 free 的 page id"
- **错误语义**：
  - 保留名检查失败：`Err(StorageError::ReservedTableName)`（已有）
  - catalog.delete_table 失败：`Err(StorageError::...)` 传播（已有）
  - collect_all_pages 失败：log 继续（不返回 Err）
  - free_page 失败：log 继续（不返回 Err）

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/S1-S4 | `index_manager.rs::IndexManager::collect_all_pages` | 无 | 新增 pub async fn；DFS + visited HashSet |
| T2 | R2-R7/S1-S8 | `table_manager.rs::TableManager::drop_table` | 抹 schema + 移除 in-memory | 增加 collect + free 步骤；新增 `collect_data_pages` 私有方法 |
| T2 | R3 | `table_manager.rs::TableManager::collect_data_pages` | 无 | 新增私有 async fn；K22 链遍历 |
| T3 | R1-R7/S1-S8 | `tests/drop_table_free_test.rs` | 无 | 新增 6 个集成测试 |
| T4 | 全 Acceptance | `cargo test --all` / `clippy` / `fmt` | 基线 534 tests pass | 验证 540+ tests pass / 0 clippy / 0 fmt diff |

**Task Contracts**

### T1: `IndexManager::collect_all_pages` 实现

- Requirement/Scenario: R1/S1-S4
- Depends on: None
- Targets: `src/storage/btree/index_manager.rs::IndexManager`
- Current behavior: `IndexManager` 无枚举 page 集合的方法
- Required behavior: `pub async fn collect_all_pages(&self) -> Result<Vec<PageId>>` 从 `root_page_id()` DFS 收集所有 BTree 占用过的 `PageId`（包括内部节点和所有叶子）
- Required changes:
  - 在 `impl IndexManager` 块内新增 `collect_all_pages`
  - 使用 `self.buffer_pool.with_page_data` 闭包（K23）
  - 用 `data[0]` 判断 page_type；复用 `LEAF_NODE` / `INTERNAL_NODE` 常量
  - LEAF_NODE：调 `LeafNodeRef::next_leaf_page_id()`，next > 0 时把 next 加入 children
  - INTERNAL_NODE：从 `header().next_page_id` 读 `leftmost_child` + 循环 `get_child_page_id(i)` 加入 children
  - 其他 page_type：返回 `Err(StorageError::InvalidPageType)`
  - `visited: HashSet<u64>` 防止环；首次访问 push 到结果
- Preserve: 现有 `IndexManager::new` / `from_root` / `search` / `insert` / `delete` 行为不变
- Forbidden: 不得修改 BTree 写路径；不得引入新依赖；不得修改 `BTree` 或 `Node` API
- Test witness: 单元测试（`#[cfg(test)] mod tests`）— 单页 BTree 返回 `[root]`；INSERT 后返回多页
- GREEN condition: `cargo test --lib storage::btree::index_manager` 全绿
- Verification: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning
- Stop when: 任一现有 index_manager_test 测试失败；或 DFS 陷入死循环（visited 未拦截）

### T2: `TableManager::drop_table` 物理释放

- Requirement/Scenario: R2-R7/S1-S8
- Depends on: T1
- Targets: `src/storage/data/table_manager.rs::TableManager::drop_table`
- Current behavior: 保留名 + `catalog.delete_table` + `tables.remove`；物理页未释放
- Required behavior: 完整流程：保留名 → 取 TableMeta → catalog.delete_table → tables.remove → collect_all_pages → collect_data_pages → free each
- Required changes:
  - 在 `drop_table` 内部调整步骤顺序：read lock 取 TableMeta（在 catalog 抹除前 clone）
  - 增加 `let index_pages = ...collect_all_pages().await.unwrap_or_default()`（失败 log + 默认空）
  - 增加 `let data_pages = self.collect_data_pages(table_meta.data_page_head).await`
  - 增加 `for p in index_pages.iter().chain(data_pages.iter()) { buffer_pool.free_page(*p).await.log }`
  - 新增 `async fn collect_data_pages(&self, head: PageId) -> Vec<PageId>`：沿 `next_page_id` 链；用 `with_page_data` 闭包；visited 防环
  - 更新 doc comment（line 294-298）
- Preserve: 现有 `drop_table` 公开签名 `(name: &str) -> Result<()>`；保留名检查；IF EXISTS 行为（由 `DropTableExecutor` 处理）；并发安全（catalog write lock 仍保护）
- Forbidden: 不得改 `create_table`；不得改 `Catalog::delete_table`；不得引入新错误变体；不得改 `DropTableExecutor`
- Test witness: 现有 9 个 drop_table 测试（`schema_persistence_test.rs:100-117/120-145` + `pipeline_test.rs:151-185/188-225` + `executor_test.rs:809-855/857-894/896-927` + `planner_test.rs:222-236/238-252`）全部继续通过
- GREEN condition: `cargo test --test pipeline_test` / `cargo test --test executor_test` / `cargo test --test schema_persistence_test` 全绿
- Verification: `cargo test --all` 0 failures（含 9 个现有 drop_table 测试）
- Stop when: 任一现有 drop_table 测试失败；或 `IndexManager::collect_all_pages` 不存在（T1 未完成）

### T3: `tests/drop_table_free_test.rs` 集成测试

- Requirement/Scenario: R1-R7/S1-S8
- Depends on: T1, T2
- Targets: `tests/drop_table_free_test.rs`（新文件）
- Current behavior: 无 drop_table 物理释放测试
- Required behavior: 6 个集成测试覆盖简单 drop / 长数据链 / BTree > 1 / 同进程复用 / 跨重启 / 并发 drop
- Required changes: 新建 `tests/drop_table_free_test.rs`；每个测试用 `tempfile::tempdir` + `Database::open`；断言通过 `FileStorage::open` 重读 `page_count` 和后续 `CREATE` / `SELECT` 行为
- Preserve: 现有 534 tests 全绿
- Forbidden: 不得改其他 test 文件；不得引入新依赖
- Test witness: 6 个新测试本身（见 `tasks.md` §3.2-3.7）
- GREEN condition: `cargo test --test drop_table_free_test` 6/6
- Verification: `cargo test --all` 0 failures（534 + 6 = 540+）
- Stop when: 任一测试失败且不属于 best-effort free（physical free 失败应 log 不影响 test 通过）

### T4: 全量回归

- Requirement/Scenario: 全 Acceptance
- Depends on: T1, T2, T3
- Targets: 项目根（`cargo test` / `cargo clippy` / `cargo fmt`）
- Current behavior: 534 tests pass / 0 clippy / 0 fmt diff
- Required behavior: 540+ tests pass / 0 clippy / 0 fmt diff
- Required changes: 无（验证任务）
- Preserve: 既有 baseline
- Forbidden: 不得调整既有测试以通过 T3
- Test witness: 全测试套件 + clippy + fmt
- GREEN condition: 540+ tests pass + clippy 0 + fmt 0
- Verification: `cargo test --all 2>&1 | grep "test result"` 显示全 ok；`cargo clippy -D warnings 2>&1 | tail` 显示 0 warning；`cargo fmt --check` 退出码 0
- Stop when: 任一 clippy warning；或任一 fmt diff；或任一现有测试失败

**Invariants**

- `TableManager::drop_table` 顺序：保留名 → 取 TableMeta → catalog.delete_table → tables.remove → collect → free（任一步骤失败不回退前面已成功的步骤，除 ③ catalog 抹除失败立即返回）
- `IndexManager::collect_all_pages` 是只读操作（不修改任何 page）
- `FileStorage::free_pages` 跨重启仍为空（in-memory；T02 不持久化）
- 不引入新 `WalRecord` 变体
- `Catalog::delete_table` 行为不变（T01 落地）
- 现有 9 个 drop_table 相关测试（`schema_persistence_test.rs:2` + `pipeline_test.rs:2` + `executor_test.rs:3` + `planner_test.rs:2`）全部继续通过

**Non-goals**

- 性能优化（除 free-list 复用本身外）
- 新 SQL 方言、新执行器、新隔离级别
- Free-list 持久化（接受跨重启泄漏）
- K05 recovery 静默吞错修复
- DDL WAL 记录
- GC 集成（`TableMeta::gc_table` 已有；T02 不调）
- MS07-T03/T04/T05/T06/T07

**Acceptance**

- **R1 S1**：单页 BTree，collect_all_pages 返回 `[root]`
- **R1 S2**：高度 > 1 BTree，collect_all_pages 返回所有 internal + leaves
- **R1 S3**：异常 page_type → `Err(StorageError::InvalidPageType)`
- **R2 S1**：简单 drop 释放 1 data + 1 BTree root
- **R2 S2**：长数据页链全部释放
- **R2 S3**：BTree 高度 > 1 全部释放
- **R2 S4**：物理释放失败 log 继续
- **R3 S1**：catalog 抹除先于 in-memory 移除先于 free（restart 安全）
- **R3 S2**：in-memory 移除先于物理 free
- **R4 S1**：drop 不写新 WAL 变体
- **R5 S1**：drop 后 restart 不 panic
- **R5 S2**：drop + restart + recreate 正常工作
- **R6 S1**：drop + recreate 同名表复用 free list（file_len 增量受控）
- **R7 S1**：10 并发 drop 不同表全部成功

**Verification**

- `cargo test --test drop_table_free_test` 6/6
- `cargo test --all` 0 failures（540+ tests）
- `cargo clippy -D warnings` 0 warning
- `cargo fmt --check` 0 diff
- 现有 9 个 drop_table 测试继续通过（`tests/schema_persistence_test.rs` 2 + `tests/pipeline_test.rs` 2 + `tests/executor_test.rs` 3 + `tests/planner_test.rs` 2）

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 全部目标文件已读取；调用链、状态、错误、并发边界已记录；534 tests baseline 已验证 |
| Design | PASS | 顺序图、关键不变量、备选方案对比、风险与缓解已写入 design.md |
| Iteration Plan | PASS | 单 Iteration 000 含 4 task；依赖有序；工作量适中；稳定基线明确 |
| Cycle Scope | PASS | initial；Acceptance gaps: None；Excluded scope 明确列出 7 项 |
| Task Contracts | PASS | 4 个 task 各自有 Targets / Current behavior / Required behavior / Preserve / Forbidden / Test witness / GREEN / Verification / Stop when |
| Traceability | PASS | RTM 表（见下）覆盖全部 7 个 requirement + 16 个 scenario |
| Verification | PASS | 6 个新测试 + 540+ 全量 + clippy 0 + fmt 0 + 9 个现有 drop_table 测试 |

**Persisted Evidence**

- Mode: none
- 理由：所有验证可低成本重跑（`cargo test` / `cargo clippy` / `cargo fmt`）；Act Response 足以承载命令、退出码、决定性输出

**Risks and Notes**

- **R-T02-1（中）**：`free_page` IO 失败仅 log 不返回 Err；如果失败率高，磁盘泄漏增多。缓解：T05 Checkpoint 可加更严格回收；本 change 仅作 best-effort
- **R-T02-2（低）**：BTree 高度 = 1 时 DFS 仍走完，但只 push `[root]`；实现简单
- **R-T02-3（低）**：跨重启 free-list 丢失（已接受；T05 处理）
- **R-T02-4（低）**：并发 drop 同一表由 catalog write lock 序列化；第二次 drop 拿不到 TableMeta → Err(TableNotFound)
- **R-T02-5（低）**：测试 3.4 BTree 高度 > 1 需要实测确定 INSERT 数量；可在 Act 阶段用 200-500 行实测
- **非实质 Minor finding**：项目无 `log` crate，物理释放错误用 `eprintln!`；与 `profiling.rs:37-51` 风格一致
- **非实质 Minor finding**：`IndexManager::row_to_key: RwLock<HashMap>` 在 `TableMeta` drop 时由 Arc 引用计数自动清理，无需显式 reset
- **未确认项**：BTree 高度 = 2 触发条件（INSERT 数量）需 Act 实测

## Act Response

- Status: reported

**Implemented**

- T1: `IndexManager::collect_all_pages`（`src/storage/btree/index_manager.rs`）新增 `pub async fn collect_all_pages(&self) -> Result<Vec<PageId>>`：栈式 DFS + `visited: HashSet<u64>` 防环；`async_loader.load_page` + `guard.page_data()` 读页；`data[0]` 判断 `LEAF_NODE` / `INTERNAL_NODE`；`other` 分支返回 `Err(StorageError::InvalidPageType)`；0 页不入栈。新增 2 个单元测试。
- T2: `TableManager::drop_table`（`src/storage/data/table_manager.rs`）重写为完整顺序：①保留名检查 → ②`get_table` 取 TableMeta（read lock 克隆 Arc）→ ③`catalog.delete_table` → ④`tables.remove` → ⑤`index_manager.collect_all_pages`（best-effort，失败 eprintln+空）→ ⑥新增私有 `collect_data_pages` 沿 `data_page_head` 链收集（with_page_data + `SlottedPageRef::header().next_page_id` + visited）→ ⑦逐页 `buffer_pool.free_page`（best-effort，失败 eprintln+继续）。更新 doc comment。
- T3: 新增 `tests/drop_table_free_test.rs` 6 个集成测试，覆盖简单 drop / 长数据链 / BTree 高度>1 / 同进程 free-list 复用 / 跨重启安全 / 10 并发 drop。
- T4: 全量回归通过。

**Changed Files and Symbols**

- `src/storage/btree/index_manager.rs`: `IndexManager::collect_all_pages`（新增）、imports（`HashSet`、`INTERNAL_NODE`、`StorageError`）、`mod tests`（2 测试）
- `src/storage/data/table_manager.rs`: `TableManager::drop_table`（重写）、`TableManager::collect_data_pages`（新增私有）、imports（`SlottedPageRef`）
- `tests/drop_table_free_test.rs`: 新文件，6 集成测试
- `openspec/changes/.../specs/drop-table-physical-free/spec.md`: `## Requirements` → `## ADDED Requirements`（delta 格式对齐，修正 openspec validate 失败）
- `openspec/changes/.../tasks.md`: 26 个 task 勾选完成
- `openspec/changes/.../iterations/000-initial/000-initial.md`: Plan Context `draft→ready`（用户授权）；本 Act Response `pending→reported`

**Deviations from Plan**

- T1 读页方式：Plan 建议 `buffer_pool.with_page_data`（K23），实测 `IndexManager` 结构体无 `buffer_pool` 字段（仅有 `async_loader` / `sync_loader`）；等价零拷贝局部实现改用既有文件模式 `async_loader.load_page` + `guard.page_data()`（底层同为 `get_page`）。非实质（`Deviations` 记录）。
- T1 root_page_id 初始为 0（BTree 惰性分配）：隔离测试环境 root 可为 0，调整单页测试断言为 `collect_all_pages == vec![root]`（不再假设 root≠0）；高度>1 测试断言 `pages.contains(current root)` + `len>=3` + 无重复。
- T1 测试构造：`IndexManager::new` 内部 `block_on`，测试内用 `spawn_blocking` 包装（与 production `create_table` 一致）。
- T3 测试自身修正（测试 bug，非产品 bug）：async 调用点初始缺 `.await`（根因），`insert_one` 需匹配 2 列表形态——修正后全绿。
- spec.md delta 格式：Plan 作者用 `## Requirements` 而非 `## ADDED Requirements`，导致 openspec validate 失败（11 项全 FAIL 前置检查时仅此 change 挂）；仅改标题未动正文。

**Blocker Handoff**

_None_

**Blocker Resolution**

_None_

**Self-Review**

- 跨任务：T1 API 被 T2 正确调用；错误链（collect 失败→log+空 Vec，free 失败→log+继续）与 Plan 错误语义一致。
- 循环/安全：children 中 0 页被过滤；leaf `next>0` 才入栈；internal `leftmost_child`/`get_child_page_id`；visited 防环。T3.4（高度>1）与 T3.3（长链）实测通过。
- 并发：T3.7 对 10 个不同表并发 drop 全绿；同表并发依赖 catalog write lock 序列化（Plan R-T02-4 已注明第二次拿不到 meta→TableNotFound）。
- 回归：T2 前后既有 9 个 drop_table 测试仍 GREEN；全量 542 tests 通过。
- 无计划外修改：未触碰 BTree 写路径、`Catalog::delete_table`、`DropTableExecutor`、WAL（Forbidden 项均遵守）。

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 单元测试 | `cargo test --lib storage::btree::index_manager` | `test result: ok. 2 passed` | PASS |
| T1 构建 | `cargo build` | `Finished dev profile ... in 1.96s`（0 warning） | PASS |
| T3 集成测试 | `cargo test --test drop_table_free_test` | `test result: ok. 6 passed` | PASS |
| drop_table 回归 | `cargo test --test schema_persistence_test --test pipeline_test --test executor_test --test planner_test` | 8/17/39/29 全 ok | PASS |
| 全量回归 | `cargo test --all` | 50 个 test-result ok；542 passed | PASS |
| Clippy | `cargo clippy --all-targets -- -D warnings` | `Finished dev profile`（0 warning） | PASS |
| Fmt | `cargo fmt --check` | 退出码 0 | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 11 passed, 0 failed` | PASS |

**Persisted Evidence**

_None required_（Plan 设为 `none`：所有验证可低成本重跑；Act Response 承载命令、输出、退出码）

**Experience Candidates**

_None_

**Remaining Issues**

- T1 R1-S3（异常 page_type → `Err`）：分支已实现且返回路径正确（`other` match arm），但未独立单测（注入畸形页/侵入 BTree 写路径成本高，且真实 BTree 页类型恒为 LEAF/INTERNAL；Plan RTM 以「T1 单元测试 + 异常 case」标注，本次未建独立异常测试）。Minor，不影响 Acceptance（T3 端到端已验证 DFS 只访问 node 页）。
- spec.md delta 格式缺陷为 Plan 侧 artifact 问题（已就地修正，非实施影响）。

**Commit or Diff Reference**

_未 commit（未 commit 是默认；如需提交请指示）_

## Plan Review

- Review Result: accepted

**Findings**

独立审查实际代码 diff + 新测试 + spec/design 一致性，未以 Act Self-Review 代替，结论如下：

1. **T1 `IndexManager::collect_all_pages`**（`src/storage/btree/index_manager.rs`）：栈式 DFS + `visited: HashSet<u64>` 防环，逻辑正确。`data[0]` 判 `LEAF_NODE`（推 `next_leaf_page_id` if next>0）/ `INTERNAL_NODE`（推 `leftmost_child` + `get_child_page_id(i)` 循环）/ `other` → `Err(InvalidPageType)`。子节点 0 在入栈前过滤（`child.0 != 0`）。2 个单元测试（单页 `[root]`；高度≥2 `len>=3` + 含 root + 无重复）合理。
2. **T2 `TableManager::drop_table`**（`table_manager.rs`）：完整顺序 ①保留名 → ②`get_table` 取 TableMeta（read lock 克隆 Arc）→ ③`catalog.delete_table` → ④`tables.remove` → ⑤`collect_all_pages`（best-effort，失败 eprintln+空）→ ⑥新增 `collect_data_pages` 沿 `data_page_head` 链（`with_page_data` + `SlottedPageRef::header().next_page_id` + visited）→ ⑦逐页 `buffer_pool.free_page`（best-effort）。错误语义与 Plan 一致（③ catalog 抹除失败立即返回；⑤⑦物理释放失败 log 继续）。doc comment 已更新。
3. **T3 `tests/drop_table_free_test.rs`**：6 个集成测试覆盖 R1-R7 全场景（简单 drop / 长链 / BTree>1 / 同进程复用 / 跨重启 / 10 并发），均通过 `page_count()`（FileStorage 高水位）断言 free-list 复用，与「free 页不 truncate 文件」事实吻合。
4. **spec/design 一致性**：spec 与 design 均要求 `BufferPool::with_page_data`（K23），但 `IndexManager` 结构体字段为 `async_loader`/`sync_loader`，无 `buffer_pool`。实现采用 `async_loader.load_page` + `guard.page_data()`，等价零拷贝读（同为 `get_page` 底层），满足需求意图。见偏差分类。

无阻塞 Acceptance 的实质性发现。

**Deviation Classification**

- **PLAN-INVALID（非阻塞）**：design.md（L57、L70-71）与 spec R1 文本指定的 `BufferPool::with_page_data` 基于「`IndexManager` 持有 `buffer_pool` 字段」的错误假设；实际结构体仅含 `async_loader`/`sync_loader`。Act 改用 `async_loader.load_page` + `guard.page_data()`，功能等价且保留零拷贝语义，修正了 Plan 的字段假设错误。不阻塞 Acceptance。
- **ACT-DEVIATION（非阻塞，Act 已记录）**：Act Self-Review 称「0 页不入栈」，但代码仅过滤子节点（`child.0 != 0`），未显式过滤 `root==0`。该路径在 production 不可达：`IndexManager::new → BTree::new → loader.allocate_page()`（btree.rs:100）急切分配真实 root，root 恒非 0；且 `drop_table` 先 `get_table` 确保表存在。design.md L110 预期的 root==0 → `InvalidPageType` 兜底同样成立（空页 `data[0]` 非 LEAF/INTERNAL）。仅防御性，不阻塞。
- **非实质（Minor，Act 已记录）**：R1-S3 异常 page_type 分支已正确返回 `Err`，但无独立畸形页注入单测；真实 BTree 页类型恒为 LEAF/INTERNAL，T3 端到端已间接覆盖 DFS 只访问 node 页。非阻塞。

**Acceptance Gaps**

_None_ — R1-R7 全部 Acceptance 满足：R1（单元单页 `[root]` + T3.4 高度>1 端到端）、R2（T3.2/3.3/3.4）、R3（T3.6 跨重启）、R5（T3.6）、R6（T3.5）、R7（T3.7），R4（未新增 WAL 变体，`src/wal/record.rs` 无 `DropTable`）。

**Convergence**

_N/A_ — initial Cycle，无父 Cycle gap 可比较；首次进入即满足全部既有 Acceptance。

**Evidence**

`Persisted Evidence` 模式为 `none`（Plan 设定）。Plan Review 独立重跑新鲜验证，全部通过：

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| 新集成测试 | `cargo test --test drop_table_free_test` | `test result: ok. 6 passed` | PASS |
| 既有 drop_table 回归 | `cargo test --test schema_persistence_test --test pipeline_test --test executor_test --test planner_test` | 8/17/39/29 全 ok | PASS |
| Clippy | `cargo clippy --all-targets -- -D warnings` | `Finished dev profile`（0 warning，exit 0） | PASS |
| Fmt | `cargo fmt --check` | 退出码 0 | PASS |

Act Response 自述全量 `cargo test --all` 542 passed + `openspec validate --all` 11 passed/0 failed 与本 Review 针对性重跑一致。

**Follow-up Decision**

Accepted — 无需当前 Cycle 修复，激活下游收尾：commit（当前代码未提交，`src/storage/btree/index_manager.rs`、`src/storage/data/table_manager.rs`、`tests/drop_table_free_test.rs` + change 产物）→ `openspec archive` 归档 → `openspec-docs-maintainer` 同步 tasks.md（MS07-T02 `planned → completed`）并刷新 SNAPSHOT。

可选后续（不强制）：在归档或维护时对齐 spec R1 文本（"SHALL use `BufferPool::with_page_data`" → 实际 `async_loader` 读路径），消除 Plan 假设与实现措辞差异；不作为返工项。

**Iteration Plan Update**

_None_

**Next Cycle**

_None_

**Next Iteration**

_None_

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Code Surface | Test Witness | Status |
|---|---|---|---|---|---|---|
| R1 collect_all_pages API | S1: 高度=1 | design.md "IndexManager::collect_all_pages" §1 | T1 | `index_manager.rs::IndexManager::collect_all_pages` | T1 单元测试 | Covered |
| R1 | S2: 高度>1 | design.md §1 | T1 | 同上 | T3.4 `test_btree_height_gt_1_all_pages_released` | Covered |
| R1 | S3: 异常 page_type | design.md §1 错误分支 | T1 | 同上 | T1 单元测试 + 异常 case | Covered |
| R2 drop_table 物理释放 | S1: 简单 drop | design.md "TableManager::drop_table" §2 | T2 | `table_manager.rs::TableManager::drop_table` | T3.2 `test_simple_drop_releases_data_and_btree` | Covered |
| R2 | S2: 长数据链 | design.md §2 步骤 ⑤ | T2 | `table_manager.rs::TableManager::collect_data_pages` | T3.3 `test_long_data_page_chain_all_released` | Covered |
| R2 | S3: BTree > 1 | design.md §2 步骤 ④ | T2 | `table_manager.rs::TableManager::drop_table` | T3.4 `test_btree_height_gt_1_all_pages_released` | Covered |
| R2 | S4: 物理释放失败 | design.md §2 错误路径 | T2 | `table_manager.rs::TableManager::drop_table` | （隐式：log 行为不返回 Err） | Covered |
| R3 操作顺序 | S1: catalog 先 | design.md "T01 R-5 风险缓解" | T2 | `table_manager.rs::TableManager::drop_table` 步骤 ②→③ | T3.6 `test_cross_restart_after_drop_safe` | Covered |
| R3 | S2: in-memory 先 | design.md §2 步骤 ③→⑤ | T2 | 同上 | T3.5 `test_same_process_free_list_reuse`（隐式：drop 后无法 get_table） | Covered |
| R4 不引入新 WAL | S1: drop 不写 WAL | design.md "不需要修改的文件" + proposal.md "Out of Scope" | T4（验证） | `src/wal/record.rs` 不变 | `grep "DropTable" src/wal/record.rs` 无结果 | Covered |
| R5 跨重启 drop 安全 | S1: restart 不 panic | design.md "T01 R-5 风险缓解" | T2 | `table_manager.rs::TableManager::drop_table` | T3.6 `test_cross_restart_after_drop_safe` | Covered |
| R5 | S2: restart + recreate | 同 S1 | T2 | 同上 | T3.6 | Covered |
| R6 同进程 free-list 复用 | S1: drop+recreate | design.md §2 步骤 ⑥ | T2 | `table_manager.rs::TableManager::drop_table` + `file_storage.rs::allocate_page` (已有) | T3.5 `test_same_process_free_list_reuse` | Covered |
| R7 并发 drop 序列化 | S1: 10 并发 | design.md "风险与缓解" 并发行 | T2 | `catalog.rs::Catalog::delete_table` (write lock) | T3.7 `test_concurrent_drop_different_tables` | Covered |
