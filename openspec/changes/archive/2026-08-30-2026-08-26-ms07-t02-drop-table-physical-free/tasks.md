# 任务清单：ms07-t02-drop-table-physical-free

> 关联里程碑：**MS07-T02**（基础能力建设 / `drop_table` 接 free-list，物理页释放）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 spec：`specs/drop-table-physical-free/spec.md`
> 关联 Iteration：仅一个 Iteration 000（含 4 个 task）

## Iteration Plan

### Iteration 000: drop_table 物理页释放

- Tasks: T1, T2, T3, T4
- Depends on: None（T01 已合并 commit `4307a0e`，本 change 是 T01 后续）
- Stable baseline: `TableManager::drop_table` 在抹 schema + 移除 in-memory 后释放所有数据页和 BTree 索引页到 `FileStorage::free_pages`；同进程内 `allocate_page` 优先复用
- Verification boundary: `tests/drop_table_free_test` 6/6；`cargo test --all` 0 failures；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff
- Diagnostic boundary: `src/storage/btree/index_manager.rs` + `src/storage/data/table_manager.rs` + `tests/drop_table_free_test.rs`
- Deferred tasks: None（本 change 完成 MS07-T02 全部子项；MS07-T05 Checkpoint + free-list 持久化留独立 change）

## Task 1: 新增 `IndexManager::collect_all_pages` 公开 API

- [x] 1.1 在 `src/storage/btree/index_manager.rs::IndexManager` 新增 `pub async fn collect_all_pages(&self) -> Result<Vec<PageId>>`
- [x] 1.2 实现 DFS：栈式遍历，从 `self.root_page_id()` 出发
- [x] 1.3 对每个 page 调 `self.buffer_pool.with_page_data(page_id, |data| -> Result<(Vec<PageId>, bool)>)` 读取 `data[0]` page_type
- [x] 1.4 LEAF_NODE 分支：调 `LeafNodeRef::next_leaf_page_id()`，next > 0 时把 next 加入 children
- [x] 1.5 INTERNAL_NODE 分支：从 `header().next_page_id` 读 `leftmost_child`，再循环 `get_child_page_id(i)` 加入 children
- [x] 1.6 其他 page_type：返回 `Err(StorageError::InvalidPageType { expected: LEAF_NODE, actual: page_type })`
- [x] 1.7 `visited: HashSet<u64>` 防止环；首次访问 push 到结果 `Vec<PageId>`；`children` 全部 push 到 stack
- [x] 1.8 复用既有 imports：`LeafNodeRef`, `InternalNodeRef`, `LEAF_NODE`（node.rs:7-8）；`use std::collections::HashSet`
- [x] 1.9 单元测试：单页 BTree 返回 `[root]`；多页 BTree（需先 INSERT 触发）返回所有 internal + leaves
  - 位置：`#[cfg(test)] mod tests` 内（index_manager.rs 末尾或新增 mod）
  - 临时 BTree 创建：`BTree::new(sync_loader)` → `index_manager.insert(...)` × N → `collect_all_pages()`
  - 验证：返回 `Vec<PageId>` 长度 = scan_all 推导出的总页数（间接）

## Task 2: 改 `TableManager::drop_table` 物理释放

- [x] 2.1 在 `src/storage/data/table_manager.rs` 修改 `pub async fn drop_table(&self, name: &str) -> Result<()>` 完整流程
- [x] 2.2 顺序：① 保留名检查（已有）→ ② 取 `TableMeta`（read lock，clone Arc）→ ③ `catalog.delete_table`（已有）→ ④ `tables.write().remove`（已有，但顺序调整）→ ⑤ `index_manager.collect_all_pages().await`（最佳努力，失败 eprintln! + 空 Vec）→ ⑥ 沿 `data_page_head` 链收集（新增 helper `collect_data_pages`）→ ⑦ 对每个 page 调 `buffer_pool.free_page`（最佳努力，失败 eprintln! + 继续）
- [x] 2.3 新增私有方法 `async fn collect_data_pages(&self, head: PageId) -> Vec<PageId>`：沿 `next_page_id` 链（K22 模式）；用 `with_page_data` 闭包；visited 防止环；任一 read 错误 log + break
- [x] 2.4 错误传播策略：
  - ① 保留名检查：Err(ReservedTableName) 立即返回
  - ② 取 TableMeta：None → Err(TableNotFound)
  - ③ catalog.delete_table：失败返回 Err（已有）
  - ④ tables.write().remove：已有；失败不会发生
  - ⑤ ⑥ ⑦ 物理释放：失败 eprintln! + 继续（不返回 Err）
- [x] 2.5 更新 doc comment（line 294-298）：从"Physical data pages and index pages are NOT freed"改为"data/index pages freed to FileStorage::free_pages; cross-restart leak accepted"
- [x] 2.6 `cargo build` 0 warning

## Task 3: 新增 `tests/drop_table_free_test.rs`

- [x] 3.1 文件头 `//! MS07-T02 drop_table 物理页释放集成测试` + imports（Database, FileStorage, ColumnType, tempdir, Arc）
- [x] 3.2 测试 `test_simple_drop_releases_data_and_btree`：
  - CREATE `users` 无 INSERT
  - `db.table_manager.drop_table("users")`
  - 重开 FileStorage → `storage.page_count()` 不变
  - 验证 `FileStorage` 的 `free_pages` 包含 data page + BTree root（通过间接方式：后续 `create_table` 应该复用这些 page id）
  - 最简验证：`CREATE TABLE users (...)` 后 `FileStorage::open` + 重新读 `__tables` + `Catalog::scan_tables` 找到新 `users` 行
- [x] 3.3 测试 `test_long_data_page_chain_all_released`：
  - CREATE `users` → `INSERT 0..999`（1000 行）触发 5+ 页数据链
  - DROP `users`
  - 通过 `db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")` 验证新表能创建（无错误）
  - 通过 `FileStorage::open` + `storage.page_count()` 验证 `file_len` 接近 `drop 前 + 1`
- [x] 3.4 测试 `test_btree_height_gt_1_all_pages_released`：
  - CREATE `users` → `INSERT 0..199` 触发 BTree 高度 = 2（需要实测确认；可调整 INSERT 数量）
  - DROP `users`
  - 验证后续 `CREATE` 不 panic；`file_len` 接近 `drop 前 + 1`
- [x] 3.5 测试 `test_same_process_free_list_reuse`：
  - CREATE `users` → INSERT 一些行 → DROP → `CREATE TABLE users` → INSERT
  - 验证两次操作的 `file_len` 增量受控（不超过 DROP 前的 + 1 页）
  - 可选：通过 `db.execute_sql("SELECT * FROM users")` 验证第二次 create 的 users 表能用
- [x] 3.6 测试 `test_cross_restart_after_drop_safe`：
  - CREATE `users` → INSERT → DROP
  - `db.close()` → drop db
  - `Database::open(path)` → `get_table("users")` → `Err(TableNotFound)`（不 panic）
  - 重新 `CREATE TABLE users` + INSERT 验证可用
- [x] 3.7 测试 `test_concurrent_drop_different_tables`：
  - CREATE `t0..t9` 10 张表
  - `tokio::spawn` 10 个并发 `drop_table(t0..t9)` → `await` 全部
  - 全部成功；`get_table(tK)` 全部 `TableNotFound`
  - 验证 `FileStorage::free_pages` 无 page id 重复（可通过 `CREATE` 后续表复用情况间接验证）

## Task 4: 全量回归

- [x] 4.1 `cargo test --all` 0 failures（基线 534 + T3 新增 6 = 540+）
- [x] 4.2 `cargo clippy -D warnings` 0 warning
- [x] 4.3 `cargo fmt --check` 0 diff（必要时 `cargo fmt` 自动修复）
- [x] 4.4 验证现有 drop_table 测试仍通过：
  - `tests/schema_persistence_test.rs::test_drop_table_removes_from_catalog`
  - `tests/schema_persistence_test.rs::test_restart_after_drop_table_gone`
  - `tests/pipeline_test.rs::test_pipeline_drop_table*` (2 个)
  - `tests/executor_test.rs::test_drop_table_executor_*` (3 个)
  - `tests/planner_test.rs::test_build_drop_table*` (2 个)

## 验收

| Acceptance | 验证 |
|---|---|
| R1 IndexManager::collect_all_pages API | T1 单元测试通过；T3.4 BTree 高度 > 1 端到端验证 |
| R2 TableManager::drop_table 物理释放 | T3.2 简单 drop；T3.3 长数据链；T3.4 BTree > 1；T3.5 free-list 复用 |
| R3 操作顺序 | T3.6 跨重启不 panic（catalog 抹除先于 restart） |
| R4 不引入新 WAL | `tests/wal_*` 不新增；`grep "DropTable" src/wal/record.rs` 无结果 |
| R5 跨重启 drop 安全 | T3.6 |
| R6 同进程 free-list 复用 | T3.5 |
| R7 并发 drop 序列化 | T3.7 |
