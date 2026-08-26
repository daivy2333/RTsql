# 任务清单：ms07-t01-schema-persistence

> 关联里程碑：**MS07-T01**（基础能力建设 / 系统表 `__tables` / `__columns` + Schema 页；最大单点）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 Iteration：仅一个 Iteration 000（含 7 个 task）
> 关联 spec：`specs/schema-persistence/spec.md`

## 1. 新增 `src/storage/catalog.rs`（系统表管理核心）

- [ ] 1.1 定义 `CatalogRow` 结构（`table_name: String, data_page_head: u32, index_root_page_id: u32, pk_index: u32, pk_column: String, column_count: u32, data_page_tail: u32`）
- [ ] 1.2 定义 `CatalogColumnRow` 结构（`table_name: String, column_index: u32, column_name: String, column_type: u32, not_null: bool, unique: bool`）
- [ ] 1.3 实现 `Catalog` 结构（持有 `buffer_pool: Arc<BufferPool>, storage: Arc<dyn AsyncStorage>, tables_root: Mutex<PageId>, columns_root: Mutex<PageId>`）
- [ ] 1.4 实现 `Catalog::bootstrap(buffer_pool, storage) -> Result<Arc<Self>>`：allocate page 0 + page 1，分别 init 为空 `__tables` / `__columns` SlottedPage（page_type=0x03）
- [ ] 1.5 实现 `Catalog::open(buffer_pool, storage) -> Result<Arc<Self>>`：从 page 0 + page 1 读取已有 root，不修改
- [ ] 1.6 实现 `Catalog::insert_table(meta, columns)`：在 `__tables` 与 `__columns` 各追加一行
- [ ] 1.7 实现 `Catalog::delete_table(name)`：在 `__tables` / `__columns` 中按 `table_name` 物理删除 slot（compact 即可）
- [ ] 1.8 实现 `Catalog::scan_tables()`：顺序扫描 `__tables` 链所有页，返回 `Vec<CatalogRow>`
- [ ] 1.9 实现 `Catalog::scan_columns(table_name)`：顺序扫描 `__columns` 链所有页，返回该表所有 `CatalogColumnRow`
- [ ] 1.10 实现 `Catalog::update_table_tail(name, new_tail)`：`__tables` 对应行更新 `data_page_tail`
- [ ] 1.11 SlottedPage append 满页处理：分配新 page → 在原 page `data[5..9]` 写 next page 指针（沿用 `data_page.rs:46` 模式）→ 更新 `tables_root` 或 `columns_root` Mutex
- [ ] 1.12 单元测试：bootstrap → insert_table × N → scan_tables 顺序一致；delete_table 后 scan 不到

## 2. 改 `src/storage/btree/index_manager.rs::IndexManager`

- [ ] 2.1 新增 `pub fn from_root(buffer_pool: Arc<BufferPool>, root_page_id: PageId) -> Result<Self>`：不调 `BTree::new`，直接绑定到指定 `root_page_id`
- [ ] 2.2 验证：`from_root(buffer_pool, PageId(0))`（非空叶子）能正常 `search` / `insert` / `delete`
- [ ] 2.3 保留 `IndexManager::new(buffer_pool)`（首次创建用）

## 3. 改 `src/storage/data/table_manager.rs::TableManager`

- [ ] 3.1 改 `TableManager::new(buffer_pool)` → `pub async fn new(buffer_pool: Arc<BufferPool>, storage: Arc<dyn AsyncStorage>) -> Result<Arc<Self>>`
- [ ] 3.2 新增字段 `catalog: Arc<Catalog>`
- [ ] 3.3 新增 `pub async fn open_or_init(&self) -> Result<()>`：调 `Catalog::scan_tables` + `scan_columns` 重建所有 `TableMeta` 并 insert 到 `self.tables`
- [ ] 3.4 辅助 `async fn build_table_meta(row, columns, buffer_pool) -> Result<Arc<TableMeta>>`：用 `IndexManager::from_root` 构造 index_manager；`data_page_tail: Mutex::new(PageId(row.data_page_tail as u64))`
- [ ] 3.5 `create_table` 末尾追加 `self.catalog.insert_table(&table_meta, &columns).await?`（在 `tables.insert` 之后）
- [ ] 3.6 `create_table` 入口加保留名检查：`["__tables", "__columns"]` 报 `StorageError::ReservedTableName`
- [ ] 3.7 `drop_table` 调 `self.catalog.delete_table(name).await?`（在 `tables.remove` 之后）
- [ ] 3.8 `drop_table` 也加保留名检查（不允许 drop 系统表）

## 4. 改 `src/storage/data_page.rs::write_tuple_to_data_page`

- [ ] 4.1 跨页分配新 page 后（line 56 `*table_meta.data_page_tail.lock() = new_page_id;`）追加：`let table_name = &table_meta.name; let _ = table_meta.index_manager.something().await;` — 实际是：`database` 不可见。**改成通过 buffer_pool 反查 table_meta 不可行；改用方案 B：在 `data_page.rs` 暴露新函数 `write_tuple_to_data_page_and_update_tail(bp, table_meta, ..., catalog: &Catalog)`，由 `InsertExecutor` 在写完后调**
- [ ] 4.2 在 `InsertExecutor`（`src/executor/insert.rs`）写完 row 后调 `catalog.update_table_tail(&table_name, new_tail)`（如果有新 page）

## 5. 改 `src/database.rs::Database::open`

- [ ] 5.1 `TableManager::new` 改为 async 调用（`database.rs:30`）
- [ ] 5.2 紧接其后 `table_manager.open_or_init().await?;`
- [ ] 5.3 `cargo build` 通过

## 6. 改 `tests/table_manager_test.rs`（API 适配）

- [ ] 6.1 `setup()` 函数（line 6-12）改造：返回 `(Arc<TableManager>, Arc<BufferPool>, TempDir)`；通过 `TableManager::new(bp.clone(), storage).await.unwrap()` 构造
- [ ] 6.2 6 个测试函数（`create_and_get_table`, `duplicate_table_error`, `table_not_found`, `create_table_allocates_data_page`, `pk_column_validation`, `table_exists_check`）全部加 `.await` + 适配新签名
- [ ] 6.3 `cargo test --test table_manager_test` 全绿

## 7. 新增 `tests/schema_persistence_test.rs`

- [ ] 7.1 测试 `test_create_table_writes_to___tables_page0`：创建 db → `CREATE TABLE users (id INT PRIMARY KEY, name TEXT)` → 读 `FileStorage::page_count` 验证 ≥ 2（page 0 = `__tables` + page 1 = `__columns` + 至少 1 个 user data page）→ 通过 `Catalog::scan_tables()` 验证 `users` 行存在
- [ ] 7.2 测试 `test_restart_recovers_table`：创建 db → CREATE → DROP database → 重新 `Database::open` 同 path → `get_table("users").is_ok()`
- [ ] 7.3 测试 `test_restart_dml_works`：创建 db → CREATE + INSERT (1, 'alice') + (2, 'bob') → restart → `SELECT * FROM users` 拿到 2 行
- [ ] 7.4 测试 `test_drop_table_removes_from_catalog`：创建 db → CREATE + DROP → `Catalog::scan_tables()` 返回空
- [ ] 7.5 测试 `test_restart_after_drop_table_gone`：CREATE → INSERT → DROP → restart → `SELECT * FROM users` 报 `TableNotFound`
- [ ] 7.6 测试 `test_index_root_persists_across_restart`：CREATE + INSERT (1, 'a') + restart → 验证重启前后的 `index_manager.root_page_id()` 相等；restart 后 `SELECT WHERE id = 1` 命中
- [ ] 7.7 测试 `test___tables_is_reserved`：`CREATE TABLE __tables` 报 `StorageError::ReservedTableName`
- [ ] 7.8 测试 `test_data_page_tail_persists`：CREATE + INSERT 1000 行触发跨页 → restart → 验证 `data_page_tail` 与原 tail 一致
- [ ] 7.9 `cargo test --test schema_persistence_test` 全绿

## 8. 全量回归

- [ ] 8.1 `cargo fmt --all` 通过
- [ ] 8.2 `cargo clippy --all-targets -- -D warnings` 通过
- [ ] 8.3 `cargo test --lib` 全绿（基线 116 单元测试 + 现有事务/storage/btree 相关测试）
- [ ] 8.4 `cargo test --tests` 全绿（基线 ~400 集成测试 + 新增 8 个 = ~408；`table_manager_test` 6 个 + `schema_persistence_test` 8 个）
- [ ] 8.5 重点回归 `tests/e2e_test.rs` / `pipeline_test.rs`（走 execute_sql 路径）
- [ ] 8.6 重点回归 `tests/mvcc_*_test.rs`（MVCC 不受 catalog 影响）
- [ ] 8.7 重点回归 `tests/wal_*_test.rs`（WAL 格式兼容）
- [ ] 8.8 重点回归 `tests/btree_*_test.rs`（`IndexManager::from_root` 不破坏 BTree 自身）

## 验收标准

| 标准 | 命令 | 预期 |
|------|------|------|
| 8 个新 schema persistence 测试 | `cargo test --test schema_persistence_test` | 8/8 pass |
| 6 个原 table manager 测试（API 适配后） | `cargo test --test table_manager_test` | 6/6 pass |
| 全量回归 | `cargo test --all` | 0 failures |
| 公共规则 | `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` | exit 0 |
| 行为契约 | restart 后能 `SELECT` 已存在的表 | e2e 验证 |
| 行为契约 | drop 后 restart 表不再存在 | test 7.5 验证 |
| 行为契约 | `__tables` / `__columns` 不可被 `CREATE TABLE` | test 7.7 验证 |
| 行为契约 | 索引根跨 restart 稳定 | test 7.6 验证 |
| 行为契约 | `data_page_tail` 跨 restart 稳定 | test 7.8 验证 |
