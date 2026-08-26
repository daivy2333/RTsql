## Why

RTsql 的 `TableManager::tables` 是纯内存 `RwLock<HashMap<String, Arc<TableMeta>>>`（`src/storage/data/table_manager.rs:98`），每次 `Database::open` 都 `TableManager::new(buffer_pool.clone())`（`src/database.rs:30`）— HashMap 全新。因此 `CREATE TABLE` 的结果**完全不持久化**，重启后所有表定义丢失，剩下裸数据页和孤儿索引根。

这个缺陷还引发 K05（recovery 静默吞错）：

```rust
// src/wal/recovery.rs:146-148（Insert/Update/Delete 三处同款）
let table_meta = match table_manager.get_table(table_name).await {
    Ok(m) => m,
    Err(_) => return Ok(()),  // ← 静默吞掉：表不在内存就跳过整条 redo
};
```

后果：WAL 中有 `Insert (id=1, name='alice')`，但重启后表不在 → `get_table` 失败 → 整条 redo 默默被丢弃。`tests/recovery_e2e_test.rs:4-9` 自爆承认现状：

```rust
//! Note: TableManager is currently in-memory only — table definitions don't
//! survive restart. These tests validate:
//! ...
//! 3. When tables are recreated after restart, data pages are accessible
```

测试 `test_data_pages_survive_restart`（`:82-126`）的 workaround 是重启后**手动 `CREATE TABLE`**，再让 recovery 补数据；但索引根节点是新的，**索引数据永久丢失**。

`IndexManager` 同样没有持久化路径：`IndexManager::new`（`src/storage/btree/index_manager.rs:25-40`）**总是**调 `BTree::new` 拿新 root（`btree.rs:100` `let root_page_id = loader.allocate_page()?;`）。`BTree::from_root`（`btree.rs:122`）已存在但无调用方。

WAL 也没有 DDL 记录：`WalRecord` 枚举（`src/wal/record.rs:80-100`）只有 `BeginTxn/CommitTxn/AbortTxn/Insert/Update/Delete/Commit/Abort/Checkpoint`，**没有 `CreateTable`/`DropTable`/`AlterTable`**。

`MS07-T01`（`tasks.md:99-127`）将本任务定位为"MS07 最大单点"：schema 持久化是 T02（drop_table 物理释放）、T05（Checkpoint 真工作）、K05 修复的共同前置。

## What Changes

按用户决策（**B 方案**），通过**系统表 `__tables` / `__columns`** 持久化 schema，page 0 预留给 `__tables` 起始页：

- **改 `src/storage/file_storage.rs::FileStorage::open`**：在 open 路径上把"新文件 page 0 已分配"作为约定；首次 `allocate_page` 自然得到 page 0
- **新增 `src/storage/catalog.rs`**：管理 `__tables` 与 `__columns` 两个系统表的 SlottedPage 读写；提供 `bootstrap` / `recover` / `insert_table` / `delete_table` / `scan_all` / `scan_columns` API
- **改 `src/storage/data/table_manager.rs::TableManager`**：
  - `new` 接受 `Arc<dyn AsyncStorage>`，调 `Catalog::bootstrap` 初始化 `__tables`/`__columns`（新文件）或 `Catalog::recover` 重建 TableMeta（已存在文件）
  - `create_table` 完成后调 `Catalog::insert_table` 写 schema
  - `drop_table` 调 `Catalog::delete_table` 抹除 schema（标记删除；物理页释放留给 MS07-T02）
- **改 `src/storage/btree/index_manager.rs::IndexManager`**：新增 `from_root(buffer_pool, root_page_id)` 路径供 `Catalog::recover` 重建已分配但未记录的索引根
- **改 `src/database.rs::Database::open`**：在 `TableManager::new` 之后调 `table_manager.open_or_init()` 完成 bootstrap/recover
- **新增 `tests/schema_persistence_test.rs`**：覆盖以下场景
  - create_table 后 `__tables` SlottedPage 实际写入 page 0+
  - restart 后 `TableManager::get_table(name)` 命中（不返回 `TableNotFound`）
  - restart 后 DML（INSERT/SELECT/DELETE）正常工作
  - restart 后 IndexManager 的 root_page_id 与重启前一致
  - drop_table 抹除 `__tables` 和 `__columns` 行
  - restart 后已 drop 的表不再出现在 `get_table`
  - `__tables` 和 `__columns` 自身是 SQL 保留名（`CREATE TABLE __tables` 报错）
  - 现有 6 个 `tests/table_manager_test.rs` 测试不破坏（API 兼容）

## Capabilities

### New Capabilities

- `schema-persistence`：表定义通过系统表 `__tables` / `__columns` 持久化，restart 后 schema 完整恢复
  - 改前：表定义在内存 HashMap，restart 即空
  - 改后：`create_table` 写 `__tables`+`__columns` SlottedPage；`Database::open` 通过 `Catalog::recover` 重建 `TableMeta`；索引通过 `IndexManager::from_root` 绑定已分配 root
  - 关联 M/K：`M01`（执行管道）、`M04`（SlottedPage）、`K05`（recovery 静默吞错的根本修复前置，本 change 完成后下一 change 修 K05）

### Out of Scope（本 change 不做）

- **MS07-T02 drop_table 物理页释放**：本 change 仅抹除 schema 行；数据页 / 索引页 / WAL 残留由独立 change 处理
- **MS07-T05 Checkpoint 真工作**：checkpoint 位点读取 + WAL truncate 逻辑独立 change
- **K05 recovery 静默吞错修复**：本 change 完成后 `get_table` 不再因 restart 失败，但 `recovery.rs:146-148` 的"找不到表就静默 return Ok(())"仍存在；修复需在 T01 完成后单独 change
- **DDL WAL 记录**：`WalRecord` 枚举不增 `CreateTable`/`DropTable` 变体；若未来需要 checkpoint 后 schema 变更可重放再补
- **MS07-T04 显式事务 API**
- **MS07-T03 planner 拆分**
- **MS07-T06 谓词下推**
- **MS07-T07 消息传递重构**
- **性能优化**（除必要的 page 0 init 外）

## Impact

- **影响模块**：
  - 新增 `src/storage/catalog.rs`（核心系统表管理）
  - `src/storage/data/table_manager.rs`（TableManager API 扩展 + 内部状态持久化）
  - `src/storage/btree/index_manager.rs`（新增 `from_root` 路径）
  - `src/storage/file_storage.rs`（open 路径对 page 0 的约定）
  - `src/database.rs::Database::open`（增加 `open_or_init` 步骤）
  - `src/executor/create_table.rs` / `src/executor/drop_table.rs`（API 不变；走 TableManager 即可生效）
  - `tests/table_manager_test.rs`（6 个测试可能因 TableManager::new 签名变化需要小幅调整）
  - `tests/schema_persistence_test.rs`（新增）
- **影响接口**：
  - `TableManager::new(buffer_pool)` → `TableManager::new(buffer_pool, storage)`（新增 storage 参数用于 bootstrap；pub 改动但项目 pre-release 阶段可接受）
  - 新增 `TableManager::open_or_init()` 公开方法
  - `IndexManager::from_root(buffer_pool, root_page_id)` 新增 pub 静态方法
  - 其他 `TableManager` / `Database` 公开 API 不变
- **影响行为**：
  - **行为差异**：`Database::open` 对已存在文件会读 `__tables` 并重建 TableMeta；新文件会 bootstrap `__tables` 与 `__columns`
  - **行为差异**：`create_table` 之后表定义持久化到 page 0+ 的 SlottedPage
  - **行为差异**：`drop_table` 抹除 `__tables` / `__columns` 中的对应行（物理页未释放）
  - **行为差异**：`__tables` / `__columns` 自身不可作为表名被 `CREATE TABLE` 使用
  - **行为不变**：单进程内 DML/SELECT 路径、PlanCache、WAL 事务边界、MVCC 可见性、并发安全（仍受 RwLock 保护）
- **兼容性**：
  - 现有 `rtsql.db`（35KB）、`:memory:.wal`（0B）文件**不向后兼容** — 项目 pre-release 阶段可接受（仓库现场就这状态）
  - 现有 6 个 `tests/table_manager_test.rs` 通过 `TableManager::new` 直构造，签名变更后需加 `storage` 参数
  - 现有 7 个执行器对 `CreateTable`/`DropTable` 节点的引用通过 `TableManager` API 间接；不需改
  - WAL 文件格式不变（不引入 DDL WAL 记录）
  - `Database::open` 启动后多一步 `open_or_init`；其余流程不变
- **风险**：
  - **高**：表定义是数据库核心；任何回归都会让 7 个执行器全部失败
  - **中**：系统表读写的并发安全（write lock 与现有 HashMap 锁的协调）
  - **中**：page 0 预留是约定性约束；后续代码若直接 `allocate_page` 拿 0 而不识别系统表，会污染 schema
  - **低**：`__tables` / `__columns` 命名与 SQL parser 的交互（标准 SQL 不允许 `__` 开头的 identifier；待 Act 验证）
- **回退方案**：`git revert` 本 change 即可；`TableManager` 旧实现已存在

## 关联

- 关联里程碑：**MS07-T01**（基础能力建设 / 系统表 + Schema 页）
- 后续依赖 change（不在本 change 范围）：
  - MS07-T02：`drop_table` 物理页释放（依赖本 change 的 `Catalog::delete_table`）
  - MS07-T05：Checkpoint 真工作（依赖本 change 的 `Database::open` 稳定恢复）
  - K05 修复：recovery 静默吞错（依赖本 change 让 `get_table` 在 restart 后命中）
