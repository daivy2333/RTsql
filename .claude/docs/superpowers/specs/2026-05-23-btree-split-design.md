# M17-Phase2 B-Tree Split 机制设计

> 日期：2026-05-23
> 状态：Approved

## 概述

实现 B-Tree 的 split 机制，使 insert 操作在叶节点满时能正确分裂并向上传播，支持树的动态增长。

## 设计决策

| 决策 | 选择 | 原因 |
|------|------|------|
| 分裂策略 | 中间分裂 | 实现简单，非唯一索引友好 |
| 非唯一 key 跨页 | 允许 | search_all 已支持跨页查找，无需额外处理 |
| 实现方式 | 递归 insert + split 回传 | 经典算法，与现有代码无缝衔接 |
| root_page_id 更新 | BTree::insert 返回 Option<PageId> | 调用方（IndexManager）负责更新 AtomicU64 |

## T6: LeafNode Split 逻辑

### 方法签名

```rust
impl LeafNode {
    pub fn split(&mut self, new_page: &mut Page) -> Result<SplitResult, StorageError>
}
```

### 流程

1. 读取当前所有 entries（key + row_id 对），按 key 排序
2. 计算中间点 `mid = entries.len() / 2`
3. 清空原页，重新插入前半部分 entries（0..mid）
4. 初始化新页为 LeafNode，插入后半部分 entries（mid..end）
5. middle_key = 后半部分第一个 entry 的 key
6. 维护 leaf 链表：
   - 新页 next = 原页旧 next
   - 原页 next = 新页 page_id
7. 返回 `SplitResult { middle_key, new_page_id }`

### 关键细节

- 非唯一 key 允许跨越分裂，不需要特殊处理
- 复用现有"重建页"模式（shift_slots_right 已有先例）
- LeafNode 的 node_type 标记字节在 `from_page_init` 时写入

## T7: 递归 Insert + Split 传播

### 核心变更

`BTree::insert_into_page` 返回 `Result<Option<SplitResult>>`

### 递归逻辑

```
insert_into_page(page_id, key, row_id) -> Result<Option<SplitResult>>:
  load page, check node type

  if leaf:
    try leaf.insert(key, row_id)
    if PageFull:
      allocate new page
      leaf.split(new_page) -> Some(split_result)
    else:
      None

  if internal:
    child_id = internal.find_child(key)
    split = insert_into_page(child_id, key, row_id)?  // 递归
    if split is Some:
      try internal.insert_separator(split.middle_key, split.new_page_id)
      if PageFull:
        allocate new page
        internal.split(new_page) -> Some(split_result)
      else:
        None
    else:
      None
```

### InternalNode Split 逻辑

```rust
impl InternalNode {
    pub fn split(&mut self, new_page: &mut Page) -> Result<SplitResult, StorageError>
}
```

1. 读取所有 separators（key + right_child 对），按 key 排序
2. `mid = separators.len() / 2`
3. middle_key = separators[mid].key（上推，不保留在任一子节点）
4. 原页保留：leftmost_child + separators[0..mid]
5. 新页：separators[mid].right_child 作为新页的 leftmost_child + separators[mid+1..end] 的 key+right_child
6. 返回 `SplitResult { middle_key, new_page_id }`

### 关键细节

- InternalNode split 时 middle key 上推（不保留在子节点），这是 B-Tree 与 B+Tree 的关键区别
- 新页的 leftmost_child = 原页 separators[mid+1] 的 child（如果存在），否则需要特殊处理
- `insert_into_page` 需要 mutable 访问页数据，使用 `modify_page` API

## T8: 根分裂处理

### 逻辑

```
BTree::insert(key, row_id) -> Result<Option<PageId>>:
  split = insert_into_page(root_page_id, key, row_id)?
  if split is Some:
    // 根分裂：创建新根
    new_root_page = allocate_page()
    初始化 new_root_page 为 InternalNode:
      leftmost_child = 原 root_page_id
      insert_separator(split.middle_key, split.new_page_id)
    self.root_page_id = new_root_page_id
    return Some(new_root_page_id)  // 通知调用方更新
  else:
    return None
```

### root_page_id 更新机制

- `BTree::insert` 返回 `Option<PageId>`：Some 表示新根 page_id，None 表示无根分裂
- `IndexManager` 调用 `BTree::insert` 后检查返回值，更新 `AtomicU64 root_page_id`
- 这保持了 BTree 与 IndexManager 的解耦

## T9: 测试套件

### 场景覆盖

| 场景 | 类型 | 描述 | 测试方法 |
|------|------|------|----------|
| S1: 空树首次分裂 | Happy | 插入直到叶节点满，触发第一次 split | 插入 >97 entries，验证树结构 |
| S2: 连续分裂 | Edge | 大量插入触发多层分裂 | 插入 300+ entries，验证 3 层 B-Tree |
| S3: 根分裂 | Edge | 根节点满时触发 split | S2 自然覆盖 |
| S4: 非唯一 key 分裂 | Edge | 相同 key 的 entries 跨越两个叶页 | 插入大量相同 key |
| S5: InternalNode 分裂 | Edge | 内节点也满时触发分裂 | S2 自然覆盖 |
| S6: Leaf 链表维护 | Happy | split 后 scan_all 仍能遍历所有 entries | 分裂后 scan_all 验证 |
| S7: 分裂后搜索 | Happy | split 后 search 能正确路由到新页 | 分裂后 search 验证 |
| S8: 分裂后删除 | Sad | split 后 delete_by_key/delete_exact 仍正确 | 分裂后删除验证 |
| S9: 单 entry 分裂 | Edge | 叶节点只有 2 个 entries 时分裂 | 构造最小分裂场景 |

### 测试策略

- 使用 `tempfile` 创建临时数据库
- 辅助函数：`count_leaf_pages()`、`get_tree_height()`、`collect_all_entries()`
- 每个场景独立测试函数

## 影响范围

| 文件 | 变更类型 | 描述 |
|------|----------|------|
| src/storage/btree/node.rs | 修改 | 添加 LeafNode::split、InternalNode::split |
| src/storage/btree/btree.rs | 修改 | 重写 insert 逻辑为递归 + split 回传 |
| src/storage/btree/index_manager.rs | 修改 | 处理 BTree::insert 返回的新 root_page_id |
| tests/btree_split_test.rs | 修改 | 扩展测试套件覆盖所有场景 |

## 不在范围内

- 页释放 API（属于 M17.5 Merge 机制）
- InternalNode 的 delete/merge 路径
- 并发 insert 的锁优化
