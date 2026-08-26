# Design: MS07-T01 Schema 持久化（系统表 `__tables` / `__columns`）

## 目标

让 `CREATE TABLE` 与 `DROP TABLE` 的表定义通过系统表 `__tables` / `__columns` 持久化到磁盘；`Database::open` 通过读取 `__tables` 重建 `TableMeta`，使 schema 跨 restart 完整恢复。索引根通过新增 `IndexManager::from_root` 路径绑定到已分配但未记录的 root page。

## 现状（修改前）

### Schema 完全无持久化

```rust
// src/storage/data/table_manager.rs:97-100
pub struct TableManager {
    tables: RwLock<HashMap<String, Arc<TableMeta>>>,
    buffer_pool: Arc<BufferPool>,
}
```

`create_table`（`:117-168`）只 `tables.insert(...)` 写内存；`drop_table`（`:204-218`）只 `tables.remove(...)` 抹内存。注释（`:212-215`）承认物理释放未做。

### IndexManager 同样无持久化

```rust
// src/storage/btree/btree.rs:96-114
pub fn new(loader: Arc<SyncPageLoader>) -> Result<Self> {
    let root_page_id = loader.allocate_page()?;  // 永远分配新 root
    ...
}
```

`IndexManager::new`（`index_manager.rs:25-40`）只走 `BTree::new`，从不调 `BTree::from_root`（`btree.rs:122`）。每个 `create_table` 都分配一个**新的**索引根 page。

### WAL 无 DDL 记录

`WalRecord`（`src/wal/record.rs:80-100`）只有 `BeginTxn/CommitTxn/AbortTxn/Insert/Update/Delete/Commit/Abort/Checkpoint` — **没有 DDL 变体**。recovery 只能从已存在的 TableMeta 读 schema，不能从 WAL 重建。

### recovery 静默吞错

```rust
// src/wal/recovery.rs:146-148, 162-165, 174-177
let table_meta = match table_manager.get_table(table_name).await {
    Ok(m) => m,
    Err(_) => return Ok(()),  // ← 静默吞掉
};
```

三处同款：`Insert` / `Update` / `Delete` 的 redo 路径。

### 测试自爆现状

`tests/recovery_e2e_test.rs:4-9` 注释承认：

```rust
//! Note: TableManager is currently in-memory only — table definitions don't
//! survive restart.
```

`test_data_pages_survive_restart`（`:82-126`）的 workaround：重启后**手动 `CREATE TABLE`**，让 recovery 补数据页；索引数据**永久丢失**。

## 修改方案

### 1. 新增 `src/storage/catalog.rs`

管理 `__tables` 与 `__columns` 两个系统表的 SlottedPage 读写。

**`__tables` SlottedPage 行格式**（payload，version_header 8B 之外）：

| 偏移 | 类型 (Tag) | 字段 | 说明 |
|---|---|---|---|
| 0 | 0x02 String | `table_name` | 表名 |
| var | 0x01 Int | `data_page_head: u32` | 该表数据页链头 |
| var | 0x01 Int | `index_root_page_id: u32` | 该表 BTree 索引根 |
| var | 0x01 Int | `pk_index: u32` | PK 在 columns 数组中的下标 |
| var | 0x02 String | `pk_column` | PK 列名 |
| var | 0x01 Int | `column_count: u32` | 列数 |

**`__columns` SlottedPage 行格式**：

| 偏移 | 类型 (Tag) | 字段 | 说明 |
|---|---|---|---|
| 0 | 0x02 String | `table_name` | 所属表名 |
| var | 0x01 Int | `column_index: u32` | 列下标 |
| var | 0x02 String | `column_name` | 列名 |
| var | 0x01 Int | `column_type: u32` | 编码的 `ColumnType` |
| var | 0x05 Bool | `not_null: bool` | NOT NULL 约束 |
| var | 0x05 Bool | `unique: bool` | UNIQUE 约束 |

**API**：

```rust
pub struct Catalog {
    buffer_pool: Arc<BufferPool>,
    storage: Arc<dyn AsyncStorage>,
    // __tables 当前 root page id（单页 or 链头）
    tables_root: Mutex<PageId>,
    // __columns 当前 root page id
    columns_root: Mutex<PageId>,
}

impl Catalog {
    pub async fn bootstrap(buffer_pool, storage) -> Result<Arc<Self>>;
    pub async fn insert_table(&self, meta: &TableMeta, columns: &[(String, ColumnType)]) -> Result<()>;
    pub async fn delete_table(&self, name: &str) -> Result<()>;
    pub async fn scan_tables(&self) -> Result<Vec<CatalogRow>>;
    pub async fn scan_columns(&self, table_name: &str) -> Result<Vec<CatalogColumnRow>>;
}
```

**bootstrap 流程**（新文件）：

1. `storage.allocate_page()` → `PageId(0)`（FileStorage 对新文件自然返回 0）
2. 在 page 0 写入空 `__tables` SlottedPage（page_type=0x03，slot_count=0，next_logical_id=0）
3. `storage.allocate_page()` → `PageId(1)`
4. 在 page 1 写入空 `__columns` SlottedPage
5. 返回 Catalog

**recover 流程**（已存在文件）：

1. 读 page 0 作为 `__tables` root
2. 读 page 1 作为 `__columns` root
3. 不需要扫描（recover 在 TableManager.open_or_init 中按需调 `scan_tables`）

**写入策略**：

- 单 page 不够时（行数 > 单页容量），`add_slot` 失败 → 分配新 page → 在原 page `data[5..9]` 写 next page 指针（沿用现有 `data_page.rs:46` 模式）→ 新 page `SlottedPage::init` → 在新 page 重试 `add_slot`
- `tables_root` / `columns_root` Mutex 维护当前链尾 page id（与 `data_page_tail` 同模式，参见 `data_page.rs:20`）

**MVCC 处理**：

- `__tables` / `__columns` 不走 MVCC；用固定 `version_header { create_tx_id: 0, commit_tx_id: 1, next_version: None, deleted: false }` 让所有 reader 视为"已提交"
- 单写者（`create_table` / `drop_table` 路径已被 `TableManager` write lock 序列化）

### 2. 改 `src/storage/data/table_manager.rs::TableManager`

**签名变更**：

```rust
// 改前
pub fn new(buffer_pool: Arc<BufferPool>) -> Self

// 改后
pub async fn new(buffer_pool: Arc<BufferPool>, storage: Arc<dyn AsyncStorage>) -> Result<Arc<Self>>
```

**新方法 `open_or_init`**：

```rust
pub async fn open_or_init(self: &Arc<Self>) -> Result<()> {
    let tables = self.catalog.scan_tables().await?;
    let mut guard = self.tables.write().await;
    for row in tables {
        let columns = self.catalog.scan_columns(&row.table_name).await?;
        let table_meta = build_table_meta(&row, &columns, self.buffer_pool.clone()).await?;
        guard.insert(row.table_name.clone(), table_meta);
    }
    Ok(())
}
```

**`create_table` 末尾追加**：

```rust
// 现有：tables.insert(name.to_string(), table_meta);
self.catalog.insert_table(&table_meta, &columns).await?;
```

**`drop_table` 替换**：

```rust
// 改前
let mut tables = self.tables.write().await;
tables.remove(name).ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
// TODO: 未来应删除数据页 / index / free pages

// 改后
let mut tables = self.tables.write().await;
tables.remove(name).ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
self.catalog.delete_table(name).await?;
// 物理页释放留给 MS07-T02
```

**保留名检查**：

```rust
const RESERVED_TABLE_NAMES: &[&str] = &["__tables", "__columns"];

if RESERVED_TABLE_NAMES.contains(&name) {
    return Err(StorageError::ReservedTableName(name.to_string()));
}
```

在 `create_table` 入口加；`drop_table` 不限制（允许 drop 系统表？不，drop 系统表会破坏 bootstrap；所以也限制）

### 3. 改 `src/storage/btree/index_manager.rs::IndexManager`

**新增 `from_root`**：

```rust
impl IndexManager {
    pub fn from_root(buffer_pool: Arc<BufferPool>, root_page_id: PageId) -> Result<Self> {
        // 不分配新 root；绑定到指定 page
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());
        Ok(Self {
            root_page_id: AtomicU64::new(root_page_id.0),
            sync_loader,
            async_loader,
            row_to_key: RwLock::new(HashMap::new()),
        })
    }
}
```

**`build_table_meta` 辅助**（在 `table_manager.rs` 中）：

```rust
async fn build_table_meta(
    row: &CatalogRow,
    columns: &[(String, ColumnType)],
    buffer_pool: Arc<BufferPool>,
) -> Result<Arc<TableMeta>> {
    let index_manager = Arc::new(
        IndexManager::from_root(buffer_pool.clone(), PageId(row.index_root_page_id as u64))?
    );
    // 验证 root page 存在（轻量：只 check page 在 FileStorage 范围内）
    Ok(Arc::new(TableMeta {
        name: row.table_name.clone(),
        columns: columns.to_vec(),
        pk_column: row.pk_column.clone(),
        pk_index: row.pk_index as usize,
        index_manager,
        data_page_head: PageId(row.data_page_head as u64),
        data_page_tail: Mutex::new(PageId(row.data_page_head as u64)),  // 恢复时 head == tail；后续 DML 会自动追加
    }))
}
```

**`data_page_tail` 恢复**：从 `__tables` 读 `data_page_head` 即可；写入路径会在 `data_page.rs:56` 自动更新 `tail`。但若表已有数据页（restart 场景），`tail` 应该是链的最后一页 — 这需要扫描数据页链或额外存一个 `data_page_tail` 字段。**简化方案**：本 change 不存 `data_page_tail`；`CatalogRow` 只有 `data_page_head`。新数据页追加时，`data_page.rs` 现有的 `page.data[5..9]` next page 链接会确保正确性，但 TableMeta.tail 初始化为 head 后可能错过已有的后续页。

**修正方案**（更稳）：在 `CatalogRow` 加 `data_page_tail: u32` 字段；`create_table` 写时两者都填 head；后续 DML 追加新页时通过 `Catalog::update_table_tail` 写 `__tables` 中对应行的 tail。但这会让每次 INSERT 都要更新 `__tables`，开销大。

**最终决定**（降低范围）：本 change 在 `data_page_tail` 字段上接受一个限制：restart 后**已存在的表**只读（不写入新数据页）；新表可正常追加。Act 实现时把 `tail` 初始化为 `head`，并加注释说明"restart 后的表不支持 INSERT；如需 INSERT 请 drop + create"。**回归场景**：测试只验证 SELECT（restart 后能查） + 验证对**新建**表 INSERT 仍工作。

**等等，这破坏 MS07 目标**。让我重新考虑。

**重新决定**：`CatalogRow` 加 `data_page_tail: u32` 字段；`Catalog::update_table_tail` 在 `data_page.rs:56` 现有 `*table_meta.data_page_tail.lock() = new_page_id;` 之后被调用。性能影响：每次 INSERT 跨页 1 次额外 `__tables` 写（同样 SlottedPage update + sync）。这是可接受的代价。

### 4. 改 `src/storage/file_storage.rs::FileStorage::open`

仅文档与约定变更，代码层面**不需要**改：

- 新文件（file_len=0）时，第一次 `allocate_page()` 自然返回 `PageId(0)`
- 已存在文件（file_len>0）时，page 0 + 1 必须被识别为 `__tables` / `__columns` 根
- 在 `FileStorage::open` 注释中明确："page 0 = `__tables` root; page 1 = `__columns` root; reserved by Catalog bootstrap"

### 5. 改 `src/database.rs::Database::open`

```rust
// 改前
let table_manager = Arc::new(TableManager::new(buffer_pool.clone()));

// 改后
let storage_for_tm = storage.clone();
let table_manager = TableManager::new(buffer_pool.clone(), storage.clone()).await?;
table_manager.open_or_init().await?;
```

### 6. 改 `src/executor/create_table.rs` / `drop_table.rs`

不需改。这两个 executor 走 `TableManager::create_table` / `drop_table`；TableManager 内部会调 `Catalog::*` 完成持久化。

### 7. 新增 `tests/schema_persistence_test.rs`

```rust
// 覆盖 8 个场景
async fn test_create_table_persists_to___tables();  // H1 步骤 1
async fn test_restart_recovers_tables();            // H1 步骤 2
async fn test_restart_dml_works();                  // H2
async fn test_drop_table_removes_from_catalog();    // H3
async fn test_restart_after_drop_table_gone();      // H4
async fn test_index_root_persists_across_restart(); // H1 索引部分
async fn test___tables_is_reserved();               // E2
async fn test_existing_table_manager_tests_still_pass();  // C2
```

每个测试用 `tempfile::TempDir` 起独立 db；用 `Database::open` 跑流程。

## 关键技术选择

### 为什么选 B 方案（系统表）

- 与现有 data page 模式（`data_page.rs`）一致；dogfood 自身存储格式
- 不引入新的 page 类型；BufferPool 路径不变
- 写并发粒度与用户表相同（单 RwLock 在 TableManager；page 0 写也是这个锁）
- 与"无 DDL WAL"决策兼容：catalog 页是 source of truth，restart 直接读

### 为什么 page 0 预留

- `FileStorage::open` 对新文件 size=0；首次 `allocate_page()` 自然得到 page 0
- 约定 page 0 = `__tables` 起始页；page 1 = `__columns` 起始页
- 不需要新加 "page 0 标记" 字段；靠 bootstrap 顺序保证
- 风险：现有代码若直接 allocate page 0 不识别会污染 schema → 缓解：所有 page 分配经 `TableManager`；`TableManager` 内部加 RESERVED 检查

### 为什么不在本 change 写 DDL WAL

- 用户决策：scope 仅 T01 持久化；DDL WAL 留给未来
- 当前 `__tables` 页是 source of truth；restart 直接读；不依赖 WAL 重放 schema
- 风险：checkpoint truncate WAL 之后若没 fsync `__tables` 页，restart 可能拿到 stale schema — 缓解：本 change 不动 checkpoint 路径；T05 真工作之后再考虑 DDL WAL

## 不变量

- `__tables` / `__columns` 自身不能被 `CREATE TABLE` 或 `DROP TABLE` 当作表名操作
- `Database::open` 完成时 `TableManager::tables` 与磁盘上 `__tables` 一致
- restart 后 `get_table(name)` 的 `Arc<TableMeta>` 与原内存版本字段一致（`name`、`columns`、`pk_column`、`pk_index`、`data_page_head`、`index_manager.root_page_id`）
- `data_page_tail` 跨 restart 保持最新值（通过 `Catalog::update_table_tail`）
- 系统表读写不写 WAL（与"无 DDL WAL"决策一致）
- 现有 7 个执行器对 `CreateTable` / `DropTable` 节点的引用路径不变
- 现有 6 个 `tests/table_manager_test.rs` 测试：API 兼容（`TableManager::new` 加 `storage` 参数 + 改 async）

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 系统表并发读写与现有 HashMap 锁冲突 | Catalog 操作纳入 TableManager write lock 路径；先写 catalog 再写 HashMap；或反过来（先 HashMap 再 catalog，rollback HashMap on error） |
| `data_page_tail` 跨 restart 同步 | `Catalog::update_table_tail` 在 DML 跨页时被调；性能影响可接受 |
| `__tables` / `__columns` 是 SQL 保留名 | `TableManager` 入口检查；测试覆盖 |
| page 0 预留是约定 | 文档化；`FileStorage::open` 注释；code review 关注 |
| `IndexManager::from_root` 验证 root page 有效 | `build_table_meta` 中检查 `page_id < page_count`（FileStorage.file_len） |
| 现有测试 `tests/table_manager_test.rs` 6 个全因 API 变更失败 | 加 `storage` 参数；不改断言 |
| 现有 `rtsql.db` 文件不向后兼容 | pre-release 阶段可接受；README 注明 |
| 首次 bootstrap 失败时 file 已扩展到 2 pages | 不需要恢复；失败时整个 `Database::open` 失败；用户重试即可（no partial state） |

## 验收标准

- 8 个 `tests/schema_persistence_test.rs` 测试全绿
- 6 个 `tests/table_manager_test.rs` 测试全绿（API 适配后）
- `cargo test --all` 0 failures
- `cargo fmt --all -- --check` exit 0
- `cargo clippy --all-targets -- -D warnings` exit 0
- `cargo build` 0 warnings
- 手动验证：创建 db → 写表 → 退出 → 重启 → 仍能 `SELECT`
