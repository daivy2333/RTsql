# M10 MVCC 完整性设计文档

> 设计日期：2026-05-21
> 状态：待审查

## 一、概述

### 目标

完善多版本并发控制（MVCC），使嵌入式数据库支持：
1. 完整版本链遍历（follow `next_version` 找第一个可见版本）
2. Commit 时同步标记版本（设置 `commit_tx_id`）
3. Abort 时立即清理未提交版本
4. 可选 GC（用户手动触发或配置）

### 需求决策

| 需求项 | 决策 | 原因 |
|--------|------|------|
| 隔离级别 | 只支持 Repeatable Read | 嵌入式场景常见需求，简化实现 |
| 版本链 GC | 可选功能（手动触发） | 用户可控，嵌入式内存有限 |
| Abort 清理 | 立即清理未提交版本 | 内存不浪费，符合用户预期 |
| Commit 时机 | 同步更新版本链 | 简单直接，无额外结构 |
| 遍历实现 | 封装在 BufferPool 层 | 逻辑复用性好，Executor 简化 |

---

## 二、架构变更

### 现有结构 vs 新增/修改

```
现有结构                      M10 新增/修改
─────────────────────────────────────────────────
TransactionManager           + tx_versions tracking
                             + commit_mark_versions()
                             + abort_cleanup_versions()

BufferPool                   + find_visible_version()
                             + read_version_header()
                             + write_commit_tx_id()

IndexScanExecutor            → 使用 find_visible_version 替代当前逻辑

ScanExecutor                 → 使用 find_visible_version 替代当前逻辑

TableManager                 + gc_table() [可选 GC]
```

### 新增数据结构

```rust
// TransactionManager 新增字段
pub struct TransactionManager {
    tx_id_allocator: TransactionId,
    active_tx_ids: RwLock<HashSet<u64>>,
    // M10 新增：跟踪每个事务的未提交版本
    tx_versions: RwLock<HashMap<u64, HashSet<RowId>>>,
}
```

---

## 三、组件详细设计

### 3.1 TransactionManager 扩展

**新增字段**：
```rust
tx_versions: RwLock<HashMap<u64, HashSet<RowId>>>
```

**新增方法**：
```rust
// 记录事务修改的版本（在 insert/update/delete executor 中调用）
pub async fn record_version(&self, tx_id: u64, row_id: RowId);

// Commit 时：遍历 tx_versions[tx_id]，为每个版本设置 commit_tx_id
pub async fn commit_mark_versions(&self, tx_id: u64, buffer_pool: &BufferPool);

// Abort 时：遍历 tx_versions[tx_id]，删除数据页上的未提交版本
pub async fn abort_cleanup_versions(
    &self, 
    tx_id: u64, 
    buffer_pool: &BufferPool, 
    table_meta: &TableMeta
);
```

**Commit 流程**：
```
1. commit_mark_versions(tx_id) → 遍历版本设置 commit_tx_id
2. active_tx_ids.remove(tx_id)
3. tx_versions.remove(tx_id)
```

**Abort 流程**：
```
1. abort_cleanup_versions(tx_id) → 删除未提交版本
2. active_tx_ids.remove(tx_id)
3. tx_versions.remove(tx_id)
```

### 3.2 BufferPool 版本链遍历

**新增方法**：
```rust
/// 遍历版本链，找到第一个对 snapshot 可见的版本
pub async fn find_visible_version(
    &self,
    row_id: RowId,
    snapshot: &Snapshot,
) -> Result<Option<Vec<u8>>>;

/// 仅读取版本头
pub async fn read_version_header(&self, row_id: RowId) -> Result<VersionHeader>;

/// 写入 commit_tx_id 到版本头
pub async fn write_commit_tx_id(&self, row_id: RowId, commit_tx_id: u64) -> Result<()>;
```

**版本链遍历算法**：
```
current = row_id
Loop:
  read_tuple_from_data_page(current) → (header, tuple)
  if snapshot.is_visible(...): return tuple
  current = header.next_version()
  if current == None: return None
```

### 3.3 Executor 修改

**IndexScanExecutor**：
- 替换 `read_tuple_from_data_page + visibility check`
- 使用 `find_visible_version(row_id, snapshot)`

**ScanExecutor**：
- 同样替换为 `find_visible_version`

**UpdateExecutor**：
- 无需修改（已正确创建版本链）
- 新增：`tx_manager.record_version(tx_id, new_row_id)` 调用

**InsertExecutor**：
- 无需修改（单版本，无版本链）
- 新增：`tx_manager.record_version(tx_id, row_id)` 调用

### 3.4 可选 GC

**TableManager 新增方法**：
```rust
/// 清理已提交的旧版本（仅保留最新可见版本）
/// 用户手动调用，默认不开启
pub async fn gc_table(
    &self, 
    buffer_pool: &BufferPool, 
    tx_manager: &TransactionManager
) -> Result<usize>;
```

**GC 算法**：
```
For each (key, row_id) in index:
  Traverse version chain → collect old versions
  Delete old versions from data page
  Update latest version's next_version = None
Return cleaned_count
```

**注意**：需新增 `delete_tuple_from_data_page` 函数（标记 slot 为空，不物理删除）。

### 3.5 IndexManager 反向查询（Abort 清理必需）

**新增方法**：
```rust
/// 根据 row_id 找到对应的 key（Abort 清理需要恢复索引）
pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>>;
```

**实现思路**：
- 维护反向映射 `row_id → key`（在 `update` 时同步更新）
- 或在 executor 中记录 `key → row_id` 映射，传递给 abort 清理

---

## 四、数据流

### 4.1 Commit 数据流

```
Executor (Update)
  → write_tuple_to_data_page(new_row_id)
  → tx_manager.record_version(tx_id, new_row_id)

TransactionManager.commit(tx)
  → commit_mark_versions(tx_id)
    → For each row_id in tx_versions[tx_id]:
      → read_version_header(row_id)
      → write_commit_tx_id(row_id, tx_id)
  → active_tx_ids.remove(tx_id)
  → tx_versions.remove(tx_id)
```

### 4.2 Abort 数据流

```
TransactionManager.abort(tx)
  → abort_cleanup_versions(tx_id)
    → For each row_id in tx_versions[tx_id]:
      → read_version_header(row_id)
      → Check if latest version in index:
        → If yes: restore index to next_version
        → Delete data page entry
      → If old version: directly delete
  → active_tx_ids.remove(tx_id)
  → tx_versions.remove(tx_id)
```

### 4.3 版本链遍历数据流

```
Executor (IndexScan/Scan)
  → find_visible_version(row_id, snapshot)
    → current = row_id
    → Loop:
      → read_tuple_from_data_page(current)
      → if visible: return tuple
      → current = next_version
      → if None: return None
```

---

## 五、错误处理

### 5.1 Commit 错误

| 场景 | 处理 |
|------|------|
| 版本头读取失败 | 返回 `StorageError::Io`，事务失败 |
| 写入 commit_tx_id 失败 | 返回 `StorageError::Io`，事务保留活跃 |

### 5.2 Abort 错误

| 场景 | 处理 |
|------|------|
| 版本不存在 | 忽略（可能已被 GC） |
| 索引恢复失败 | 记录警告，继续清理 |
| 数据页删除失败 | 返回 `StorageError::Io` |

### 5.3 版本链遍历错误

| 场景 | 处理 |
|------|------|
| 版本链断裂 | 返回 `StorageError::VersionChainBroken` |
| 所有版本不可见 | 返回 `Ok(None)`（非错误） |

---

## 六、测试策略

### 6.1 新增测试文件

| 文件 | 内容 |
|------|------|
| `tests/version_chain_test.rs` | 版本链遍历（4-6 tests） |
| `tests/mvcc_commit_test.rs` | commit 标记（3-5 tests） |
| `tests/mvcc_abort_test.rs` | abort 清理（4-6 tests） |
| `tests/gc_test.rs` | 可选 GC（2-3 tests，可选） |

### 6.2 测试场景

**Commit 标记**：
- Tx1 更新行，commit 后版本链可见性正确
- Tx1 更新多次，commit 后所有版本都被标记

**版本链遍历**：
- 最新版本不可见 → 遍历找到旧版本
- 所有版本不可见 → 返回 None
- 自创建版本（未提交）可见

**Abort 清理**：
- Tx1 更新行，abort 后索引恢复指向旧版本
- Tx1 插入新行，abort 后索引删除条目
- Tx1 更新多次，abort 后清理所有版本

**并发测试**：
- Tx1 未提交，Tx2 看到旧版本
- Tx1 commit 后，Tx2（Repeatable Read）仍看到旧版本
- Tx3（新事务）看到 Tx1 的新版本

---

## 七、实现顺序

### Phase 1：基础结构
1. TransactionManager 新增 `tx_versions` 字段和 `record_version` 方法
2. BufferPool 新增版本链遍历方法（`find_visible_version`, `read_version_header`, `write_commit_tx_id`）
3. IndexManager 新增反向查询 `find_key_by_row_id`（Abort 清理必需）

### Phase 2：Executor 集成 record_version
1. UpdateExecutor 调用 `tx_manager.record_version(tx_id, new_row_id)`
2. InsertExecutor 调用 `tx_manager.record_version(tx_id, row_id)`
3. 测试：tx_versions 正确记录版本

### Phase 3：Commit 标记
1. 实现 `commit_mark_versions`
2. 修改 `commit` 流程
3. 测试：commit 后版本可见性

### Phase 4：版本链遍历集成
1. IndexScanExecutor 使用 `find_visible_version`
2. ScanExecutor 使用 `find_visible_version`
3. 测试：版本链遍历正确（最新版本不可见时找到旧版本）

### Phase 5：Abort 清理
1. 实现 `abort_cleanup_versions`（使用 `find_key_by_row_id` 恢复索引）
2. 修改 `abort` 流程
3. 测试：abort 清理未提交版本、索引恢复指向旧版本

### Phase 6：可选 GC（低优先级）
1. 实现 `delete_tuple_from_data_page`（标记 slot 为空）
2. 实现 `gc_table` 方法
3. 测试：GC 清理旧版本

---

## 八、风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 版本链遍历性能 | 每次读可能多次 IO | 后续可考虑缓存热点版本链 |
| Abort 清理索引恢复 | 索引指向恢复可能失败 | 记录日志，允许手动恢复 |
| tx_versions 内存增长 | 长事务可能记录大量版本 | GC 可清理已提交版本 |
| 并发冲突 | 多事务同时修改同一行 | RowLockTable 已实现 |

---

## 九、后续优化方向（M13）

1. 版本链遍历缓存（热点数据）
2. 异步 commit 标记（减少 commit 延迟）
3. 自动 GC 策略（定期触发）
4. WAL 集成（M11）

---

## 十、总结

M10 通过最小改动完善 MVCC：
- 新增 `tx_versions` 跟踪未提交版本
- BufferPool 封装版本链遍历
- Commit 同步标记，Abort 立即清理
- 可选 GC 用户手动触发

改动范围可控，测试增量添加，符合嵌入式数据库轻量原则。