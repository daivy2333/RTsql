# gc_test SlottedPage SlotID 失效 Bug 修复设计

> 日期：2026-05-24
> 状态：Approved
> 范围：修复 gc_test 3 个 panic，引入逻辑 Row ID 解耦 slot_index 与 row_id

---

## 1. 问题分析

### 1.1 根因

`RowId` 的 `slot_id` 字段直接等于 `SlottedPage` 的物理 slot index。当 GC 执行 `delete_slot` + compact 后，slot 数组重新编号，但版本链（`VersionHeader.next_version`）和索引中的 `row_id` 仍持有旧 slot_id → `read_tuple_from_data_page` 通过旧 slot_id 查找 → 找到错误/空 slot → slice 越界 panic。

### 1.2 影响范围

- `tests/gc_test.rs`：3 个测试 panic
- 任何涉及 `delete_slot` + compact 的场景都有风险
- 阻塞 M18 Phase3 所有后续任务（T4-T8）

### 1.3 修复方向

引入**逻辑 Row ID**：`RowId.slot_id` 从物理 slot_index 变为 logical_id。`SlottedPage` 内部维护 `logical_id → slot_index` 映射，compact 时只更新映射，`logical_id` 永不改变。

---

## 2. 设计方案：SlottedPage 内部映射表

### 2.1 核心变更

**Slot 条目扩展**：从 4B 扩展为 6B，新增 `logical_id` 字段。

```
旧格式: [offset: u16][length: u16]             = 4 bytes
新格式: [logical_id: u16][offset: u16][length: u16] = 6 bytes
```

**Header 变更**：利用现有 5 字节 padding 中的 2 字节存储 `next_logical_id`。

```rust
pub struct SlottedPageHeader {
    pub page_type: u8,            // 不变
    pub slot_count: u16,          // 不变
    pub free_space_offset: u16,   // 不变
    pub next_page_id: u32,        // 不变
    pub next_logical_id: u16,     // 新增：下一个可分配的 logical_id
    _padding: [u8; 3],            // 缩减：5 → 3
}
```

### 2.2 RowId 语义变更

`RowId` 结构不变（`(page_id: u32, slot_id: u16)`），但 `slot_id` 字段语义从物理 slot_index 变为 logical_id。二进制格式不变，向后兼容。

### 2.3 SlottedPage API 变更

```rust
impl SlottedPage {
    /// 通过 logical_id 查找 slot，返回 (Slot, slot_index)
    pub fn get_slot_by_logical_id(&self, logical_id: u16) -> Option<(Slot, usize)>;

    /// 添加新 slot，返回 (logical_id, slot_index)
    /// logical_id = next_logical_id++
    pub fn add_slot(&mut self, data: &[u8]) -> Result<(u16, usize), String>;

    /// 通过 logical_id 删除 slot
    /// 内部执行 compact，更新 logical_id → slot_index 映射
    pub fn delete_slot_by_logical_id(&mut self, logical_id: u16) -> Result<(), String>;

    /// 保留旧 API（通过 slot_index 访问），供内部使用
    pub fn get_slot(&self, index: usize) -> Option<Slot>;
}
```

**SlottedPageRef 同步变更**：新增 `get_slot_by_logical_id` 方法。

### 2.4 Compact 逻辑

`delete_slot_by_logical_id` 流程：
1. 遍历 slot 数组，找到 `logical_id` 匹配的 slot_index
2. 从 slot 数组移除该条目（后移前填）
3. `slot_count -= 1`
4. **所有 slot 的 `logical_id` 不变**，只是物理位置（slot_index）可能变化
5. `next_logical_id` 不变（不重用已删除的 logical_id）

### 2.5 DataPage API 变更

```rust
// write_tuple_to_data_page: 返回 RowId{page_id, slot_id: logical_id}
//   内部: add_slot 返回 (logical_id, _slot_index)，用 logical_id 构造 RowId

// read_tuple_from_data_page: 改用 get_slot_by_logical_id(row_id.slot_id)
//   替代原来的 get_slot(row_id.slot_id as usize)

// update_version_header_in_data_page: 改用 get_slot_by_logical_id

// delete_tuple_from_data_page: 改用 delete_slot_by_logical_id
```

### 2.6 B-Tree 层变更

B-Tree 和 Data Page 共享同一个 `SlottedPage` + `Slot` 结构。Slot 格式从 4B → 6B 后：

- `Slot::SIZE` 常量从 4 变为 6，所有引用该常量的空间计算自动适配
- B-Tree 的 `free_space`、`split` 阈值、`add_slot` 等自动适配新大小
- B-Tree 的 `delete_slot` 使用物理 slot_index（通过 `position` 参数），不受 logical_id 影响
- B-Tree slot 中存储的 RowId 是数据内容（key 的 value），不是 slot index
- **结论**：B-Tree 只需适配 `Slot::SIZE` 变化，不需要 logical_id 映射

---

## 3. 影响评估

### 3.1 空间开销

- 每个 Slot 从 4B → 6B（+50% slot 开销）
- 假设 4096B 页，100B/tuple，约 36 个 tuple：
  - 旧：36 × 4B = 144B slot 开销
  - 新：36 × 6B = 216B slot 开销
  - 增加约 72B（1.8% 页空间）
- **影响可忽略**

### 3.2 性能影响

- `get_slot_by_logical_id` 需要遍历 slot 数组查找（O(n)，n = slot_count）
- 典型 slot_count < 50，线性查找 < 50 次比较，**可忽略**
- 如果未来需要，可加 slot_count 较大时的二分查找优化

### 3.3 改动范围

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `slotted_page.rs` | 重构 | Slot 6B + logical_id API + Header next_logical_id |
| `data_page.rs` | 修改 | read/write/update/delete 改用 logical_id |
| `table_manager.rs` | 无变更 | gc_table 逻辑不变（delete_tuple_from_data_page 内部适配） |
| `gc_test.rs` | 无变更 | 测试逻辑不变，修复后应全部通过 |
| `btree/node.rs` | 自动适配 | Slot::SIZE 变 6B，空间计算自动适配，无需 logical_id |

---

## 4. 测试策略

### 4.1 单元测试

| 测试 | 覆盖 |
|------|------|
| `test_slotted_page_logical_id_increment` | add_slot 返回递增 logical_id |
| `test_slotted_page_delete_preserves_logical_id` | 删除后剩余 slot 的 logical_id 不变 |
| `test_slotted_page_get_by_logical_id` | 通过 logical_id 正确读取数据 |
| `test_slotted_page_delete_by_logical_id` | 通过 logical_id 正确删除 |
| `test_slotted_page_compact_after_delete` | 删除后 compact 映射正确 |

### 4.2 集成测试

| 测试 | 覆盖 |
|------|------|
| gc_test 3 个测试 | 全量通过（修复验证） |
| `cargo test` 全量 | 无回归 |

---

## 5. 不做的事

- **不重用 logical_id**：删除后的 logical_id 永远不重用，避免 ABA 问题
- **不加全局 Row ID Registry**：过度设计，当前 bug 修复不需要
- **不改 RowId 二进制格式**：`slot_id` 字段语义变更但格式不变
- **不做 B-Tree 的 logical_id**：B-Tree 的 slot 用物理 index，与 RowId 解耦，不需要映射
