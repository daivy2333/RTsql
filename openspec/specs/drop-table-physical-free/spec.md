# drop-table-physical-free Specification

## Purpose
`TableManager::drop_table` 在抹除目录行并移出内存表项之后，把该表的数据页和 BTree 页归还到 `FileStorage::free_pages`。来源：`2026-08-26-ms07-t02-drop-table-physical-free`
## Requirements
### Requirement: IndexManager::collect_all_pages 公开 API

`IndexManager` SHALL 提供 `pub async fn collect_all_pages(&self) -> Result<Vec<PageId>>`，从 `root_page_id` 出发，返回该 BTree 占用过的所有 `PageId`（包括内部节点和所有叶子节点）。实现 SHALL 使用 `BufferPool::with_page_data` 闭包式 API（`K23`）和已存在的 `LEAF_NODE` / `INTERNAL_NODE` 常量（`src/storage/btree/node.rs:7-8`）做页类型判断，**不引入新依赖**。

#### Scenario: 高度 = 1（单页 BTree）返回单元素

- **GIVEN** 一个表只有 1 个 BTree root（`root_page_id = PageId(N)`）且该页是 `LEAF_NODE`
- **WHEN** 调用 `index_manager.collect_all_pages().await`
- **THEN** 返回 `vec![PageId(N)]`（长度 = 1）

#### Scenario: 高度 > 1 返回所有内部 + 叶子页

- **GIVEN** 一个表有 100+ 行（触发 BTree 内部节点产生），`root_page_id = PageId(N)` 是 `INTERNAL_NODE`
- **WHEN** 调用 `index_manager.collect_all_pages().await`
- **THEN** 返回的 `Vec<PageId>` SHALL 包含 `PageId(N)`（根）以及所有内部节点的 PageId 和所有叶子节点的 PageId
- **AND** 集合大小 SHALL 等于该 BTree 的总页数（可通过 `TableMeta::gc_table` 调用 `index_manager.scan_all()` 后间接对比）

#### Scenario: 页类型异常返回错误

- **GIVEN** 某 `PageId` 的 `data[0]` 既不是 `LEAF_NODE` 也不是 `INTERNAL_NODE`（例如被误复用的 system page）
- **WHEN** `collect_all_pages` 遇到该页
- **THEN** SHALL 返回 `Err(StorageError::InvalidPageType { ... })`
- **AND** 不 panic

### Requirement: TableManager::drop_table 物理释放

`TableManager::drop_table(name)` SHALL 在抹除 `__tables` / `__columns` 行 + 移除 in-memory HashMap 项之后，**额外**将该表占用的所有数据页和 BTree 页归还到 `FileStorage::free_pages`。具体顺序：①保留名检查 → ②`catalog.delete_table`（已有）→ ③`tables.remove`（已有）→ ④收集索引页（`IndexManager::collect_all_pages`）→ ⑤沿 `data_page_head` 链收集数据页（`K22`）→ ⑥对每个收集的 page 调 `buffer_pool.free_page`。

#### Scenario: 简单 drop 释放 1 个 data page + 1 个 BTree root

- **GIVEN** 一个新建表 `users`（`data_page_head = PageId(2)`, `index_root_page_id = PageId(3)`，无 INSERT）
- **WHEN** `drop_table("users")`
- **THEN** `FileStorage::free_pages` SHALL 至少包含 `PageId(2)` 和 `PageId(3)`
- **AND** 后续 `allocate_page` SHALL 优先返回这些 freed page id

#### Scenario: 长数据页链全部释放

- **GIVEN** 一个表 `users` 通过 `INSERT 1000` 触发 5+ 页数据链（`data_page_head = PageId(2)`, `data_page_tail = PageId(6)`）
- **WHEN** `drop_table("users")`
- **THEN** `FileStorage::free_pages` SHALL 包含 `PageId(2), PageId(3), PageId(4), PageId(5), PageId(6)`（顺序不要求）
- **AND** 释放后 `file_len` 不变（page 仍在文件但进了 free list）

#### Scenario: BTree 高度 > 1 全部释放

- **GIVEN** 一个表 `users` 通过 `INSERT 200` 触发 BTree 高度 = 2（`root_page_id = PageId(5)` 是 INTERNAL_NODE + 2 个 LEAF_NODE）
- **WHEN** `drop_table("users")`
- **THEN** `FileStorage::free_pages` SHALL 包含 `PageId(5)`（root）和 2 个叶子 page

#### Scenario: 物理释放失败不阻塞 schema 抹除

- **GIVEN** `buffer_pool.free_page(page_id)` 因 IO 错误返回 `Err`
- **WHEN** `drop_table` 调用 `free_page` 遇到错误
- **THEN** SHALL `eprintln!("drop_table: failed to free page {}: {}", page_id, e)` 记录并继续 drop（schema 已抹除，正确性无影响）
- **AND** 不向调用方返回错误

### Requirement: drop_table 操作顺序

`TableManager::drop_table` SHALL 严格按以下顺序执行：①保留名检查 → ②`catalog.delete_table(name)` → ③`tables.write().remove(name)` → ④收集 index pages → ⑤收集 data pages → ⑥对每个 page 调 `buffer_pool.free_page`。**T01 R-5 风险**（`IndexManager::from_root` 不验证 page 内容）通过此顺序天然缓解：catalog 行先抹除 → restart 拿不到 freed page id。

#### Scenario: catalog 抹除先于 in-memory 移除

- **GIVEN** 表 `users` 已持久化（`__tables` 含 `users` 行）
- **WHEN** `drop_table("users")` 执行到一半（任何步骤）
- **AND** 进程崩溃后 restart
- **THEN** `__tables` SHALL 不含 `users` 行（catalog 抹除已发生）
- **AND** restart 后 `get_table("users")` SHALL 返回 `Err(TableNotFound)`，不会触发 `from_root` 拿 stale page id

#### Scenario: in-memory 移除先于物理 free

- **GIVEN** `drop_table` 执行中
- **WHEN** 完成 `tables.remove(name)` 但还没调 `free_page`
- **THEN** `TableManager::get_table(name)` SHALL 返回 `Err(TableNotFound)`（in-memory 已移除）
- **AND** 后续 `collect_all_pages` / `free_page` 错误不会回退 in-memory 状态

### Requirement: 不引入新 WAL 记录

`TableManager::drop_table` SHALL NOT 写新 `WalRecord` 变体（沿用 T01 决策）。`Catalog::delete_table` 已抹除 `__tables` / `__columns` 行，崩溃后 redo 不会重放 drop（也无副作用）。

#### Scenario: drop 不写 WAL

- **GIVEN** WAL 已开启
- **WHEN** 执行 `DROP TABLE users` 后立即 `kill -9` 进程
- **THEN** 重启后 WAL SHALL NOT 包含 `DropTable` 相关记录类型
- **AND** `__tables` / `__columns` 中 `users` 行 SHALL 不存在（catalog 抹除在 WAL 之前持久化）

### Requirement: 跨重启 drop 安全

`drop_table` 后进程重启 SHALL NOT panic 且 SHALL NOT 读 stale BTree 根。释放的 page id 在重启后 SHALL 仍不被任何持久对象引用（catalog 行已删），因此可以安全地被重新分配：重启时 SHALL 依据持久目录重建空闲页集合，把持久目录引用不到的页纳入本次打开的可分配页集合（详见 `free-page-lifecycle` 域）。重建 SHALL NOT 把持久目录仍登记的页纳入可分配页集合。

#### Scenario: restart after drop 不 panic

- **GIVEN** 1 个表 `users` 已被 drop（catalog 行已抹）
- **WHEN** 进程重启 + `Database::open(path)`
- **THEN** SHALL NOT panic
- **AND** `get_table("users")` SHALL 返回 `Err(TableNotFound)`

#### Scenario: restart after drop+recreate 正常工作

- **GIVEN** 1 个表 `users` 已被 drop
- **WHEN** 进程重启 + `Database::open(path)` + `CREATE TABLE users` + `INSERT` + `SELECT`
- **THEN** 全部操作 SHALL 成功
- **AND** `users` 的 BTree SHALL 从新分配的 page 开始，无 stale root

#### Scenario: restart after drop 复用释放的页

- **GIVEN** 1 个表 `users` 已被 drop，且该 drop 已完成一次 checkpoint（目录删除已持久化）
- **WHEN** 进程重启 + `Database::open(path)` + `CREATE TABLE users`
- **THEN** 新表占用的是 `users` 释放的页号，库文件体积不因新建表而增长
- **AND** 库中其他活表的页不进入可分配页集合

### Requirement: 同进程 free-list 复用

被 drop 释放的 page id SHALL 进入 `FileStorage` 的空闲集合；同进程内后续 `allocate_page` SHALL 优先从空闲集合弹出，实现 file_len 单调不增。可分配时机 SHALL 受释放的持久化判定约束：被释放的页只有在其释放已被一次成功完成的 checkpoint 持久化之后才可被分配（详见 `free-page-lifecycle` 域）。释放动作本身 SHALL NOT 写零页；清零发生在该页被弹出分配时。

#### Scenario: drop+recreate 同名表复用 free list

- **GIVEN** 1 个表 `users` 通过 `INSERT 200` 占用 2 个 data page + 1 个 BTree root + 1 个 BTree leaf
- **WHEN** `drop_table("users")` 后完成一次 checkpoint（释放已持久化），再 `CREATE TABLE users (id INT PRIMARY KEY)`
- **THEN** 新表的 `data_page_head` SHALL 等于 `users` drop 时释放的某个 page id（free list 复用）
- **AND** `file_len` SHALL 不超过 `drop 前 + 1`（catalog pages 0/1 + 新表的 1 个 data page；BTree page 也复用 free list）

#### Scenario: 释放未持久化前不复用

- **GIVEN** 1 个表 `users` 已被 `drop_table` 释放，其目录删除尚未被 checkpoint 刷盘
- **WHEN** 同一会话 `CREATE TABLE users (id INT PRIMARY KEY)`
- **THEN** 新表不得占用 `users` 释放的页（该页尚未成为可分配页）
- **AND** `users` 的磁盘页内容保持原样（未被清零）

### Requirement: 并发 drop 序列化

N 个并发 `drop_table` 不同表 SHALL 由 `Catalog::write_lock` 序列化，全部成功且无 page 双重 free。

#### Scenario: 10 并发 drop 不同表

- **GIVEN** 10 个表 `t0..t9` 全部存在
- **WHEN** 10 个 `tokio::spawn` 并发执行 `drop_table(t0..t9)`
- **AND** `await` 全部完成
- **THEN** 10 个 drop SHALL 全部成功
- **AND** `FileStorage::free_pages` SHALL 不含任何 page id 重复
- **AND** `TableManager::get_table(tK)` for any K SHALL 返回 `Err(TableNotFound)`

