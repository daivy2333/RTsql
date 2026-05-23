# IndexScanAllExecutor 设计文档

> 创建日期：2026-05-23
> 里程碑：M18 Phase2 Executor层非唯一索引测试覆盖

## 1. 需求概述

**目标**：在 Executor 层支持非唯一索引查询，返回所有匹配键的行。

**当前状态**：
- B-Tree 层已实现 `search_all(key) -> Vec<RowId>`（M17-Phase1）
- IndexManager 缺少 `search_all` 方法
- Executor 只有 `IndexScanExecutor`（单键查询）
- 测试缺少非唯一索引 executor 测试

**任务分解**（完整实现 4 个任务）：
- T1: 新增 IndexManager::search_all 方法
- T2: 新增 IndexScanAllExecutor 结构体 + execute
- T3: executor_test.rs 新增非唯一索引测试
- T4: SQL层集成（Planner + Pipeline）

---

## 2. 核心设计决策

### 2.1 返回行为（已确认）

**决策**：返回多个 ExecResult::Row（逐行返回）

**原因**：
- ✅ 符合现有 Executor 模式（ScanExecutor 也是逐行返回）
- ✅ Pipeline 层无需改动
- ✅ 与其他 Executor 一致，易于集成

**替代方案**（已排除）：
- ExecResult::Rows(Vec<Vec<Value>)：破坏接口一致性
- ExecResult::RowIds(Vec<RowId>)：增加上层复杂度

---

### 2.2 MVCC 版本链处理（已确认）

**决策**：复用 IndexScanExecutor 的 MVCC 逻辑

**实现**：
- 对每个 row_id 调用 `buffer_pool.find_visible_version(row_id, snapshot)`
- 只返回 snapshot 可见的版本
- 某个 row_id 所有版本不可见时，跳过该 row_id，继续下一个

**原因**：
- ✅ 与现有 IndexScanExecutor 一致
- ✅ 复用成熟的 MVCC 逻辑
- ✅ 保证事务隔离语义

---

### 2.3 IndexManager::search_all 实现（自动决策）

**新增方法**：
```rust
pub async fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>> {
    let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
    let key_obj = Key::new(key);

    self.search_all_from_page_async(root_page_id, &key_obj).await
}
```

**设计要点**：
- 复用 AtomicPageId 无锁读取模式（与 search 方法一致）
- 实现 async 版本，调用 BTree::search_all 逻辑
- 返回 Vec<RowId>（而非 Option<RowId>）

---

### 2.4 IndexScanAllExecutor 结构（自动决策）

**结构体**：
```rust
pub struct IndexScanAllExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    key: Vec<u8>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    row_ids: Vec<RowId>,      // 查询结果
    current_idx: usize,        // 迭代索引
    initialized: bool,         // 是否已执行 search_all
}
```

**关键特性**：
- 惰性初始化：避免不必要的 search_all 调用
- MVCC 可见性：跳过不可见版本，继续下一个 row_id
- 逐行返回：符合 Executor 接口约定

---

### 2.5 SQL层集成（自动决策）

**Planner 零件**：
- 新增 `PhysicalPlan::IndexScanAll { table, key, snapshot }`
- Planner 根据 IndexManager.is_unique 判断使用哪种 Executor

**Pipeline 零件**：
- 新增 `PhysicalPlan::IndexScanAll` 分支
- 创建 `IndexScanAllExecutor` 实例

**SQL 解析**：
- 不需要改动 SQL parser（非唯一索引是索引属性，SQL 不变）

---

### 2.6 测试覆盖（自动决策）

**executor_test.rs 新增测试**：
- `test_index_scan_all_executor_basic`：插入 3 行相同键，验证返回 3 行
- `test_index_scan_all_executor_mvcc`：多事务可见性验证
- `test_index_scan_all_executor_empty`：查询不存在的键
- `test_index_scan_all_executor_single`：查询唯一键（回退场景）

---

## 3. 实现细节

### 3.1 IndexManager::search_all_from_page_async

**实现逻辑**（复用 BTree::search_all 的逻辑，但用 async loader）：

```rust
fn search_all_from_page_async<'a>(
    &'a self,
    page_id: PageId,
    key: &'a Key,
) -> Pin<Box<dyn Future<Output = Result<Vec<RowId>> + Send + 'a>> {
    Box::pin(async move {
        let child_page_ids = {
            let guard = self.async_loader.load_page(page_id).await?;
            let data_guard = guard.page_data();

            if data_guard[0] == LEAF_NODE {
                // Leaf: 直接收集所有匹配的 row_id
                let leaf = LeafNodeRef::new(&data_guard);
                let matches = leaf.find_all_matches(key);
                let mut row_ids = Vec::new();
                for idx in matches {
                    if let Some(rid) = leaf.get_row_id(idx) {
                        row_ids.push(rid);
                    }
                }
                return Ok(row_ids);
            } else {
                // Internal: 复制 BTree::search_all 的逻辑
                // 检查 separator 是否匹配 key，需要搜索左右子树
                let internal = InternalNodeRef::new(&data_guard);
                let count = internal.key_count();

                let mut child_page_ids = Vec::new();
                for i in 0..count {
                    if let Some(sep_key) = internal.get_key(i) {
                        if sep_key == *key {
                            // 匹配 separator：需要搜索左右子树
                            let left_child = if i == 0 {
                                internal.leftmost_child()
                            } else {
                                internal.get_child_page_id(i - 1)
                            };
                            let right_child = internal.get_child_page_id(i);
                            child_page_ids.push(PageId(left_child as u64));
                            child_page_ids.push(PageId(right_child as u64));
                        }
                    }
                }

                // 如果没有匹配 separator，按正常路由继续
                if child_page_ids.is_empty() {
                    let child_page_id = internal.find_child_page_id_binary(key);
                    child_page_ids.push(PageId(child_page_id as u64));
                }

                child_page_ids
            }
        }; // guard and data_guard dropped

        // 递归搜索所有子树
        let mut results = Vec::new();
        for child_page_id in child_page_ids {
            let child_results = self.search_all_from_page_async(child_page_id, key).await?;
            results.extend(child_results);
        }
        Ok(results)
    })
}
```

---

### 3.2 IndexScanAllExecutor::next

**核心逻辑**：

```rust
impl Executor for IndexScanAllExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // 惰性初始化：首次调用时执行 search_all
        if !self.initialized {
            self.row_ids = self.table_meta.index_manager
                .search_all(&self.key).await?;
            self.initialized = true;
        }

        // 逐个返回可见版本
        while self.current_idx < self.row_ids.len() {
            let row_id = self.row_ids[self.current_idx];
            self.current_idx += 1;

            // MVCC 可见性检查（复用 IndexScanExecutor 逻辑）
            if let Some(ref snapshot) = self.snapshot {
                let tuple_bytes = self.buffer_pool
                    .find_visible_version(row_id, snapshot).await?;

                if let Some(data) = tuple_bytes {
                    let values = deserialize_tuple(&data, &self.schema)?;
                    return Ok(Some(ExecResult::Row(values)));
                }
                // 不可见：跳过，继续下一个 row_id
            } else {
                // 无 snapshot：直接读取最新版本
                let (_, tuple_bytes) = read_tuple_from_data_page(
                    &self.buffer_pool, row_id).await?;
                let values = deserialize_tuple(&tuple_bytes, &self.schema)?;
                return Ok(Some(ExecResult::Row(values)));
            }
        }

        Ok(None) // 所有 row_id 处理完毕
    }
}
```

---

## 4. 数据流示例

**SQL 查询**：
```sql
SELECT * FROM users WHERE name = 'Alice'
```

**执行流程**：
```
SQL: SELECT * FROM users WHERE name = 'Alice'
↓
Parser: SELECT * FROM users WHERE name = 'Alice'
↓
Planner:
  - 检查 users.name 是否有索引
  - 检查 IndexManager.is_unique
  - is_unique=false → PhysicalPlan::IndexScanAll
↓
Pipeline: 创建 IndexScanAllExecutor
↓
Executor.next():
  1. search_all('Alice') → [RowId(1), RowId(2), RowId(3)]
  2. find_visible_version(RowId(1)) → Row(['Alice', 25])
  3. find_visible_version(RowId(2)) → Row(['Alice', 30])
  4. find_visible_version(RowId(3)) → Row(['Alice', 35])
  5. 返回 None（迭代完成）
```

---

## 5. 关键约束

### 5.1 Surgical Changes 原则

**修改范围**：
- `src/storage/btree/index_manager.rs`：新增 search_all 方法
- `src/executor/index_scan_all.rs`：新文件
- `src/executor/mod.rs`：导出新 executor
- `tests/executor_test.rs`：新增测试
- `src/planner/mod.rs`：新增 PhysicalPlan::IndexScanAll
- `src/pipeline.rs`：新增 IndexScanAllExecutor 创建逻辑

**不改动**：
- 现有 IndexScanExecutor
- SQL parser
- BTree 层实现（search_all 已存在）

---

### 5.2 TDD Iron Law

**测试驱动顺序**：
1. 先写 executor_test.rs（定义期望行为）
2. 实现 IndexManager::search_all
3. 实现 IndexScanAllExecutor
4. 运行测试验证功能

**验证要点**：
- search_all 返回多行
- MVCC 可见性正确
- 边界场景覆盖

---

### 5.3 Requirements Integrity

**不裁剪需求**：
- ✅ 完整实现 4 个任务
- ✅ 不简化 MVCC 处理
- ✅ 不跳过 SQL层集成

---

## 6. 实现顺序

**依赖关系**：
```
T1: IndexManager::search_all 方法
  ↓ 依赖 BTree::search_all（已存在）

T2: IndexScanAllExecutor 结构体 + execute
  ↓ 依赖 T1

T3: executor_test.rs 新增测试
  ↓ 依赖 T1/T2

T4: SQL层集成（Planner + Pipeline）
  ↓ 依赖 T1/T2/T3
```

**预估工期**：1 天（与 tasks.md 一致）

---

## 7. 后续优化

**暂不考虑**（遵循 YAGNI）：
- 批量查询版本链（性能优化）
- 并行处理多个 row_id（并发优化）
- 缓存 search_all 结果（缓存优化）

**后续里程碑**（M19+）：
- 索引统计信息（cardinality 估算）
- 查询优化器自动选择 IndexScan vs IndexScanAll