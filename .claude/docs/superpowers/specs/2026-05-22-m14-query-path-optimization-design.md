# M14: 查询路径优化设计文档

> 日期：2026-05-22
> 目标：PK 查询 3-5x 提速

## 问题分析

PK 查询比 SQLite 慢 ~10x（~50µs vs ~5.5µs），根因：

1. **parse+plan 占 ~60% 时间**：每次 `execute_sql()` 都完整走 parse→plan→execute
2. **BTree 读操作用 `page()` 克隆 4KB**：未迁移到零拷贝 `page_data()`

## 方案选择

方案 A：BTree 零拷贝 + SQL 文本级 LRU 缓存（已确认）
- 改动适中，收益最大，与已有架构一致
- 需解决 PhysicalPlan Clone

---

## Part 1: BTree 零拷贝迁移

### 现状

`btree.rs` 读操作（search、scan_all）用 `guard.page()` 获取 `&mut Page`，
再通过 `LeafNode::from_page(&mut Page)` 构造。读操作不需要可变性。

### 设计

1. **新增 `LeafNodeRef`**（只读零拷贝，类似 `SlottedPageRef`）
   - 持有 `&[u8]` 切片引用
   - 提供 `get_key()`、`get_row_id()`、`key_count()`、`find_key_position()`
   - 从 `PageDataGuard` 的 `data()` 获取 `&[u8]`

2. **新增 `InternalNodeRef`**（同理）
   - 提供 `get_key()`、`get_child_page_id()`、`find_child_page_id()`

3. **改造读操作**
   - `search()` → `guard.page_data()` + `LeafNodeRef::from_bytes(data)`
   - `scan_all()` → 同上
   - 写操作（insert/delete/update）不变，继续用 `modify_page()`

4. **IndexManager 影响**
   - `search()` 和 `scan_all()` 改用零拷贝路径
   - `insert/delete/update` 不变

---

## Part 2: SQL 文本级 LRU 缓存

### 核心结构

```rust
struct CachedPlan {
    plan: PhysicalPlan,
    table_names: Vec<String>,
}

// Database 中添加:
plan_cache: Arc<Mutex<LruCache<String, CachedPlan>>>
```

### 缓存流程

1. `execute_sql(sql)` → 查缓存
2. 命中 → 跳过 parse + plan + register_table，直接创建 executor
3. 未命中 → 正常流程，完成后写入缓存

### PhysicalPlan Clone

- 13 种节点全部实现 Clone
- 大部分节点已含可 Clone 字段（String、Vec、Value、u32 等）
- `IndexScan.index_btree: Arc<BTree>` — Arc Clone 只增引用计数，无需 BTree impl Clone
- `NestedLoopJoin.left_plan/right_plan: Box<PhysicalPlan>` — 递归 Clone，PhysicalPlan impl Clone 后自动可用
- `Update.index_btree: Arc<BTree>` — 同 IndexScan
- 无需修改任何字段类型

### LRU 配置

- 容量上限 256 条
- 使用 `lru` crate
- 纯 LRU 淘汰，无 TTL

### 缓存失效

- DDL（CREATE TABLE / DROP TABLE）清空相关表的缓存条目
- 简化方案：DDL 操作清空整个缓存（安全优先，DDL 频率极低）

---

## Part 3: 边界情况与安全

| 边界情况 | 处理策略 |
|----------|----------|
| 并发缓存访问 | `Arc<Mutex<LruCache>>` 互斥，简单安全 |
| 缓存容量满 | LRU 自动淘汰最久未用条目 |
| DDL 后缓存失效 | 清空整个缓存（DDL 频率极低，安全优先） |
| PhysicalPlan Clone 边界 | 所有 13 种节点均实现 Clone，无特殊处理 |
| 缓存中 plan 引用已删除的表 | DDL 清缓存兜底，不会出现 |

---

## Part 4: 文件变更范围

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `storage/btree/node.rs` | 新增 | LeafNodeRef + InternalNodeRef |
| `storage/btree/btree.rs` | 修改 | 读操作改用 page_data() + LeafNodeRef |
| `storage/btree/index_manager.rs` | 修改 | search/scan_all 改用零拷贝 |
| `executor/plan.rs` | 修改 | PhysicalPlan 各节点实现 Clone |
| `database.rs` | 修改 | 添加 plan_cache 字段 |
| `pipeline.rs` | 修改 | 缓存查询逻辑 |
| `Cargo.toml` | 修改 | 添加 lru crate |

**不变更**：executor 各实现、WAL、transaction、network（Surgical Changes）

---

## Part 5: 测试策略

- BTree 零拷贝：现有 node.rs tests + 新增 LeafNodeRef 单元测试
- SQL 缓存：新增缓存命中/未命中集成测试 + DDL 清缓存测试
- 性能验证：现有 micro_bench 对比 M13 baseline