# Design: MS07-T02 drop_table 物理释放

## 概述

T01 让 schema 跨进程持久化。T02 在 `TableManager::drop_table` 中加入"物理页释放"步骤，使同进程 `CREATE → DROP → CREATE` 不再让 `file_len` 单调递增。free-list 跨重启丢失（in-memory），但正确性无影响（catalog 行已抹除，restart 永远拿不到 freed page id）。

## 序列图

### Happy Path（drop 释放数据页 + BTree 页）

```
用户 → DropTableExecutor::next
    ↓
TableManager::drop_table("users")
    │
    ├─ ① 保留名检查（"__tables" / "__columns" → Err）
    │
    ├─ ② catalog.delete_table("users")
    │     └─ Catalog 写锁（self.lock）
    │     └─ delete_from_chain(__tables) → 抹 "users" 行
    │     └─ delete_from_chain(__columns) → 抹该表所有列行
    │
    ├─ ③ tables.write().remove("users")
    │     └─ 内存 HashMap 移除（保留名检查之外的常规路径）
    │
    ├─ ④ index_manager.collect_all_pages()
    │     └─ 根节点 page_type 判断
    │     │   ├─ LEAF_NODE → 沿 next_leaf_page_id 链走
    │     │   └─ INTERNAL_NODE → leftmost_child + get_child_page_id(i) → 递归/DFS
    │     └─ visited HashSet 防止环
    │
    ├─ ⑤ collect_data_pages(data_page_head)
    │     └─ 沿 next_page_id 链走（K22 模式）
    │
    └─ ⑥ 对每个 page 调 buffer_pool.free_page(page_id)
          └─ free_pages 失败 → eprintln! 警告 → 继续（best-effort）
```

### 错误路径（free_page IO 失败）

```
buffer_pool.free_page(p) → Err(StorageError::Io(_))
    ↓
eprintln!("drop_table: failed to free page {}: {}", p, e)
    ↓
继续处理下一个 page（schema 已抹除；restart 看不到这些 page）
    ↓
最终 Ok(()) 返回给调用方
```

## 关键实现细节

### IndexManager::collect_all_pages

```rust
/// 收集 BTree 全部占用过的 PageId（内部节点 + 所有叶子）。
/// DFS 使用 buffer_pool.with_page_data 闭包（K23 模式）。
pub async fn collect_all_pages(&self) -> Result<Vec<PageId>> {
    let mut pages = Vec::new();
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut stack: Vec<PageId> = vec![self.root_page_id()];

    while let Some(page_id) = stack.pop() {
        if !visited.insert(page_id.0) {
            continue;
        }

        // 用闭包读取 page_type 和 children
        let (children, is_internal): (Vec<PageId>, bool) = self
            .buffer_pool
            .with_page_data(page_id, |data| -> Result<(Vec<PageId>, bool)> {
                let page_type = data[0];
                if page_type == LEAF_NODE {
                    let leaf = LeafNodeRef::new(data);
                    let next = leaf.next_leaf_page_id();
                    let children = if next == 0 {
                        Vec::new()
                    } else {
                        vec![PageId(next as u64)]
                    };
                    Ok((children, false))
                } else if page_type == INTERNAL_NODE {
                    let iref = InternalNodeRef::new(data);
                    let mut children = vec![PageId(iref.leftmost_child() as u64)];
                    for i in 0..iref.key_count() {
                        if let Some(c) = iref.get_child_page_id(i) {
                            children.push(PageId(c as u64));
                        }
                    }
                    Ok((children, true))
                } else {
                    Err(StorageError::InvalidPageType {
                        expected: LEAF_NODE,
                        actual: page_type,
                    })
                }
            })
            .await?;

        pages.push(page_id);
        if is_internal || !children.is_empty() {
            stack.extend(children);
        }
    }
    Ok(pages)
}
```

**关键不变量**：
- `root_page_id() == PageId(0)` 表示未初始化的 IndexManager；此时 `with_page_data(0, ...)` 读到 `data[0]` = 0（空页），返回 `InvalidPageType` 错误。**缓解**：调用方 (`drop_table`) 在调 `collect_all_pages` 前应已确保表存在（已通过 `tables.remove` 验证）
- `visited` 防止环（理论上 BTree 不会有环，但 defensive）
- DFS 而非 BFS：实现简单，stack 大小 = 树高

### TableManager::drop_table 完整流程

```rust
pub async fn drop_table(&self, name: &str) -> Result<()> {
    // ① 保留名检查
    if name == TABLES_SYSTEM_NAME || name == COLUMNS_SYSTEM_NAME {
        return Err(StorageError::ReservedTableName(name.to_string()));
    }

    // 先取出 TableMeta 拿到 index_manager 和 data_page_head（在 catalog 抹除前）
    // 注意：必须用 read lock 取，避免和 create_table 的 write lock 互锁
    let table_meta = {
        let tables = self.tables.read().await;
        tables.get(name).cloned()
    };
    let table_meta = match table_meta {
        Some(m) => m,
        None => return Err(StorageError::TableNotFound(name.to_string())),
    };

    // ② catalog.delete_table（已有；先抹除 schema 行）
    self.catalog.delete_table(name).await?;

    // ③ tables.write().remove（已有；移除 in-memory）
    {
        let mut tables = self.tables.write().await;
        tables.remove(name);
    }

    // ④ 收集 index pages
    let index_pages = match table_meta.index_manager.collect_all_pages().await {
        Ok(pages) => pages,
        Err(e) => {
            eprintln!("drop_table: failed to collect index pages for {}: {}", name, e);
            Vec::new()
        }
    };

    // ⑤ 收集 data pages（沿 next_page_id 链）
    let data_pages = self.collect_data_pages(table_meta.data_page_head).await;

    // ⑥ 物理释放（best-effort）
    for page_id in index_pages.iter().chain(data_pages.iter()) {
        if let Err(e) = self.buffer_pool.free_page(*page_id).await {
            eprintln!("drop_table: failed to free page {}: {}", page_id.0, e);
            // 不返回错误；schema 已抹除，正确性无影响
        }
    }

    Ok(())
}

/// 沿 next_page_id 链收集数据页（K22 模式）
async fn collect_data_pages(&self, head: PageId) -> Vec<PageId> {
    let mut pages = Vec::new();
    let mut current = Some(head);
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
    while let Some(page_id) = current {
        if !visited.insert(page_id.0) {
            break;
        }
        let next = match self
            .buffer_pool
            .with_page_data(page_id, |data| -> Result<u32> {
                let slotted = SlottedPageRef::new(data);
                Ok(slotted.header().next_page_id)
            })
            .await
        {
            Ok(n) => n,
            Err(e) => {
                eprintln!("drop_table: failed to read next_page_id for {}: {}", page_id.0, e);
                break;
            }
        };
        pages.push(page_id);
        current = if next == 0 { None } else { Some(PageId(next as u64)) };
    }
    pages
}
```

**关键不变量**：
- 顺序：① → ② → ③ → ④ → ⑤ → ⑥；任一步骤失败不回退前面已成功的步骤（除 ② 失败时直接返回）
- ② 失败 = catalog 写盘失败 → 整 drop 失败；in-memory 和物理页都不变
- ③ 失败 = in-memory 锁不可用（不会发生，tokio RwLock）
- ④ ⑤ 失败 → `eprintln!` 继续
- ⑥ 失败 → `eprintln!` 继续（这是 best-effort 的核心）

### T01 R-5 风险缓解

T01 R-5（`iterations/000-initial.md:435`）记录：
> `IndexManager::from_root` 不验证 page 内容；如果 `__tables` 中记录的 `index_root_page_id` 是被 free 的 page（drop_table 物理释放后会），重启后会 panic。

**T02 顺序天然缓解**：
- ② `catalog.delete_table` 先抹 `__tables` 中 `users` 行的 `index_root_page_id` 引用
- restart 时 `Catalog::recover` / `open_or_init` 读 `__tables` 已没有 `users` 行 → `IndexManager::from_root` 不会被调用
- 即使 `free_page` 失败，restart 仍不会 panic（catalog 行已抹除）

## 备选方案对比

| 方案 | 描述 | 优点 | 缺点 | 选择 |
|---|---|---|---|---|
| **A（采用）** | 新增 `IndexManager::collect_all_pages()` public API | 可复用（未来 GC/迁移）；职责清晰；可单测 | 多一个 pub API | ✅ |
| B | 在 `TableManager::drop_table` 内联 DFS | 少一个 pub API | 逻辑散；难以单测；DFS 逻辑不能复用 | ❌ |
| C | 只走 `next_leaf_page_id` 不递归内部节点 | 实现简单 | 漏掉内部节点 → 物理页泄漏 | ❌ |

| 方案（free-list 跨重启） | 描述 | 优点 | 缺点 | 选择 |
|---|---|---|---|---|
| **A（采用）** | 接受跨重启泄漏 | T02 范围最小；正确性无影响 | 跨重启 `file_len` 不缩 | ✅ |
| B | 持久化 free-list 到 header page | 跨重启也能复用 | 范围扩大；需 schema 改动（page 0/1 分配） | ❌ |
| C | 留给 MS07-T05 Checkpoint 一并处理 | 职责聚合 | T02 完成后才有"完整"文件管理 | ❌（用户决策） |

## 关键依赖关系

```
T01 (schema-persistence, 已合并 commit 4307a0e)
    ↓ 提供
TableManager.drop_table 现有逻辑（保留名 + catalog delete + in-memory remove）
    ↓ 本 change
+ ④ IndexManager::collect_all_pages
+ ⑤ collect_data_pages (K22 模式)
+ ⑥ buffer_pool.free_page (file_storage.rs 已有)
```

## 不需要修改的文件

- `src/storage/file_storage.rs`：`free_page` 已存在
- `src/storage/buffer_pool.rs`：`free_page` 已存在
- `src/storage/btree/btree.rs`：BTree 实现不变；新 API 在 IndexManager 层
- `src/storage/catalog.rs`：`delete_table` 已存在；不需要改
- `src/executor/drop_table.rs`：执行器逻辑不变；通过 `TableManager::drop_table` 间接生效
- `src/wal/*`：不引入新 WAL 变体

## 风险与缓解

| 风险 | 严重度 | 缓解 |
|---|---|---|
| `free_page` 失败（IO 错误）但 schema 已抹除 | 中 | `eprintln!` 记录；不返回错误；orphan page 在磁盘但 restart 看不到 |
| `collect_all_pages` 在 BTree 高度 = 1 时返回 `[root]` | 低 | 设计如此；测试覆盖 |
| 跨重启 free-list 丢失 | 低 | 决策已接受；T05 Checkpoint 可加持久化 |
| 内部节点 free 顺序错误导致 BTree 不一致 | 低 | 全部 free（无顺序依赖）；不再访问 BTree |
| DFS 环（理论上 BTree 无环） | 低 | `visited: HashSet` defensive 防御 |
| 并发 drop 同一表 | 中 | catalog write lock 序列化；in-memory remove 后 `get_table` 失败，第二次 drop 立即返回 TableNotFound |
| 并发 drop 不同表 | 低 | catalog write lock 序列化；各自的 page 集合无交集 |
