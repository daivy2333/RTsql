## Why

`MS07-T01`（commit `4307a0e`）让表定义通过系统表 `__tables` / `__columns` 持久化，restart 后 schema 完整恢复。但 `TableManager::drop_table`（`src/storage/data/table_manager.rs:299-315`）只做 3 件事：

1. 保留名检查
2. 调 `Catalog::delete_table` 抹除 `__tables` / `__columns` 中行
3. 从 in-memory `tables: HashMap` 移除

**物理数据页和索引页完全未释放**（注释明确："out of scope; covered by MS07-T02"）。后果：

- 反复 `CREATE → INSERT → DROP → CREATE` 后 `file_len` 单调递增，磁盘永不回收
- `FileStorage::free_pages: Mutex<Vec<u64>>`（`src/storage/file_storage.rs:16`）虽然是 `allocate_page` 的回来源（line 97-99 pop 优先），但**永远空**，因为没有调用方 `free_page`
- T01 plan R-5（`iterations/000-initial.md:435`）已记录此风险：`IndexManager::from_root` 不验证 page 内容；如果 `__tables` 中记录的 `index_root_page_id` 是被 free 的 page（drop_table 物理释放后会），重启后会 panic
- 缓解已确认：T01 的 `Catalog::delete_table` 顺序是先抹 catalog 行再删 in-memory，T02 物理释放只需保持"先 catalog 后 in-memory 后 free"即可天然安全（restart 永远拿不到 free 的 page id）

`MS07-T02`（`tasks.md:127`）将本任务定位为"基础能力建设 / drop_table 物理页释放"。

## What Changes

按用户决策（**A+A+A+A**），`drop_table` 物理释放仅做最小必要修复，**不引入**新 schema 维度：

- **改 `src/storage/btree/index_manager.rs::IndexManager`**：
  - 新增 `pub async fn collect_all_pages(&self) -> Result<Vec<PageId>>`：从 `root_page_id` 出发，DFS 内部节点（`INTERNAL_NODE=0x02`）+ 沿 `next_leaf_page_id` 链走所有叶子节点（`LEAF_NODE=0x01`），返回完整 PageId 列表
  - 复用现有 `LEAF_NODE` / `INTERNAL_NODE` 常量（`src/storage/btree/node.rs:7-8`）和 `with_page_data` 闭包式 API（K09/K23）
- **改 `src/storage/data/table_manager.rs::TableManager::drop_table`**：
  - 顺序：reserved name 检查 → `catalog.delete_table`（已有）→ `tables.remove`（已有）→ 收集 `index_pages` + 沿 `data_page_head` 链收集 `data_pages`（`K22` 模式）→ 对每个 page 调 `buffer_pool.free_page`（最佳努力）
  - `free_page` 失败（IO 错误）→ `log::warn!` 记录并继续（schema 已抹除，restart 看不到这些 page；泄漏可接受）
  - 顺序安全：`catalog.delete_table` 先于 `tables.remove` 先于 `free_pages`（即使 free 部分失败，restart 拿不到 freed page id）
- **新增 `tests/drop_table_free_test.rs`**：覆盖以下场景
  - 简单 drop：1 张表 + 1 个 data page + 1 个 BTree root，drop 后所有 page 进 free list
  - 长数据页链：INSERT 触发 5+ 页数据链，drop 后所有 page 进 free list
  - BTree 高度 > 1：INSERT 100+ 行触发 BTree 内部节点产生，drop 后内部节点和叶子都进 free list
  - 同进程复用：drop → 立即 create 同名表 → INSERT → `file_len` 不超过 `drop 前 + 1 页`（即 free list 复用）
  - 跨重启：drop → 关闭 db → 重启 → 重新 create 同名表正常（不 panic，不读到 stale 索引根）
  - 并发 drop：N 个并发 drop 不同表，catalog 写锁序列化，全部成功

## Capabilities

### New Capabilities

- `drop-table-physical-free`：`drop_table` 在抹除 catalog 行 + 移除 in-memory 项后，将该表的数据页和 BTree 索引页全部归还到 `FileStorage` free list；同进程内后续 `allocate_page` 优先复用；跨重启不要求 free-list 持久化（接受磁盘泄漏，正确性无影响）
  - 改前：`drop_table` 只抹 schema；`file_len` 永不缩；`free_pages` 永远空
  - 改后：`drop_table` 释放数据页和 BTree 页；同进程 free-list 立即复用；新能力 `IndexManager::collect_all_pages` 可被未来 GC/迁移复用
  - 关联 M/K：`M01`（执行管道）、`M02`（两层分离索引）、`M04`（SlottedPage）、`K22`（数据页链表遍历模式）、T01 R-5 风险缓解

### Out of Scope（本 change 不做）

- **Free-list 持久化**：`FileStorage::free_pages` 保持 in-memory（`file_storage.rs:16`）；跨重启被 free 的页不进 free list 但正确性无影响（catalog 行已删，重启后看不到）
- **MS07-T05 Checkpoint 真工作**：独立 change
- **MS07-T04 显式事务 API** `Database::begin/commit/rollback`：独立 change
- **MS07-T03 planner 拆分**：独立 change
- **MS07-T06 谓词下推**：独立 change
- **MS07-T07 消息传递重构**：独立 change
- **DDL WAL 记录**：沿用 T01 决策（`proposal.md:67`），不增 `WalRecord::DropTable` 变体；崩溃期间 drop 行为：catalog 行已抹除 → restart 看不到该表 → 已被 free 的 page 不会被重启后任何路径访问
- **GC 集成**（`TableMeta::gc_table` 已有，`table_manager.rs:67-96`）：T02 只 free 整页，不调 GC；GC 收集旧版本留在后续 change
- **性能优化**（除 free list 复用本身外）

## Impact

- **影响模块**：
  - `src/storage/btree/index_manager.rs`（新增 `collect_all_pages` 方法）
  - `src/storage/data/table_manager.rs`（`drop_table` 增加物理释放步骤）
  - `tests/drop_table_free_test.rs`（新增）
- **影响接口**：
  - 新增 `IndexManager::collect_all_pages(&self) -> Result<Vec<PageId>>`（pub async）
  - `TableManager::drop_table` 行为扩展（已有 public API；行为差异 = 现在释放物理页）
  - 其他 API 不变
- **影响行为**：
  - **行为差异**：`drop_table` 释放数据页和 BTree 页到 `FileStorage::free_pages`
  - **行为差异**：同进程 `create_table` 后续 `allocate_page` 优先复用 free list 中的 page id
  - **行为差异**：`drop_table` 后 `file_len` 不再增长（同进程内）
  - **行为不变**：`Catalog::delete_table` 抹 schema 行；in-memory `tables.remove`；`if_exists` 行为；`ReservedTableName` 检查；并发安全（catalog write lock 仍保护）；WAL 不写新变体
- **兼容性**：
  - 现有 `rtsql.db`（35KB）向后兼容 — drop_table 行为扩展，旧文件 restart 不变
  - 现有 6 个 `tests/table_manager_test.rs` 测试 API 不变（`drop_table` 仍是 `async` + `&str` + `Result`）
  - 现有 `tests/schema_persistence_test.rs` 2 个 drop_table 测试（`test_drop_table_removes_from_catalog` / `test_restart_after_drop_table_gone`）继续通过（drop 行为扩展是 superset）
  - 现有 `tests/executor_test.rs` 3 个 drop_table executor 测试（`test_drop_table_executor_*`）继续通过
  - 现有 `tests/pipeline_test.rs` 2 个 drop_table pipeline 测试继续通过
- **风险**：
  - **中**：`free_page` 失败（IO 错误）只 log，不返回 error；如果失败率高，被 free 的 page 留在磁盘但不会被重启后任何路径访问（因为 catalog 行已删）。缓解：log 错误便于诊断；后续 T05 Checkpoint 可加更严格的回收
  - **低**：`collect_all_pages` 在 BTree 高度 = 1（单页）时直接返回 `[root]`；高度 > 1 时需递归内部节点；可能误入"已经 free 的 page"（被另一个 drop 释放又被新 create 复用）→ 缓解：catalog write lock 序列化 drop，drop 期间其他 drop 阻塞；同进程不会同时有 2 个 drop
  - **低**：`BufferPool::free_page` 会从 `pages` DashMap 移除（`buffer_pool.rs:373-379`）；如果 free 失败但已从 cache 移除 → 内存态正确（page 不再被引用），磁盘态有泄漏（page 数据残留在文件）→ 同前条缓解
- **回退方案**：`git revert` 本 change 即可；T01 抹 catalog 逻辑保留，物理释放回退到"只抹 schema"

## 关联

- 关联里程碑：**MS07-T02**（基础能力建设 / `drop_table` 接 free-list）
- 关联 M/K：`M01`（执行管道）、`M02`（两层分离索引）、`M04`（SlottedPage）、`K22`（数据页链表遍历）
- 关联已落地 change：`archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence`（T01 抹 catalog + from_root 路径 + R-5 风险记录）
- 后续依赖 change（不在本 change 范围）：
  - MS07-T05：Checkpoint 真工作（持久化 free-list 可作为 T05 的一部分）
  - MS07-T04：显式事务 API
  - K05 修复：recovery 静默吞错（独立 change）
