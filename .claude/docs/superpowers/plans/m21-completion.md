# M21 Completion Plan — DELETE 修复 + 惰性 set_all_visible + Benchmark

> 2026-06-04 | Gate 1 approved

## 任务清单

### T1: DELETE mark_deleted（正确性修复）

**目标**：DELETE 后 DataScan 不返回已删除行

**改动**：
1. `src/transaction/version_chain.rs` — 新增 `mark_deleted()` 方法
   ```rust
   pub fn mark_deleted(mut self) -> Self {
       self.commit_tx_id = UNSET_TX_ID;  // sentinel: deleted
       self
   }
   pub fn is_deleted(&self) -> bool {
       self.commit_tx_id == UNSET_TX_ID && self.create_tx_id != 0
   }
   ```
   注意：`UNSET_TX_ID` 已用于表示"未提交"。DELETE 后 `commit_tx_id` 仍为 UNSET，但 `create_tx_id` 不变。
   需要区分"未提交"和"已删除"。方案：用 `commit_tx_id = u64::MAX - 1` 作为删除标记。

2. `src/executor/delete.rs` — 在删索引前标记 version header
   ```rust
   // 标记 version header 为已删除
   let vh = self.buffer_pool.read_version_header(rid).await?;
   let deleted_vh = vh.mark_deleted();
   update_version_header_in_data_page(&self.buffer_pool, rid, deleted_vh, &[]).await?;
   // M21: 清除 visibility 标记
   self.buffer_pool.clear_all_visible(PageId(rid.page_id as u64));
   // 再删索引
   self.index_manager.delete(&self.key).await?;
   ```

3. `src/executor/data_scan.rs` — `is_visible` 检查排除已删除行
   - `check_page_all_visible` 中增加 `!vh.is_deleted()` 检查
   - DataScan 闭包中增加 `vh.is_deleted()` 跳过

**验证**：
- `test_visibility_delete_clears_all_visible` 期望 4 行（非 5）
- `test_visibility_full_scan_after_delete` 新增测试

**依赖**：无

---

### T2: check_page_all_visible（惰性设置）

**目标**：扫描路径发现页全已提交时惰性设置 `all_visible`

**改动**：
1. `src/storage/buffer_pool.rs` — 新增 `check_page_all_visible` 方法
   ```rust
   pub async fn check_page_all_visible(
       &self,
       page_id: PageId,
       snapshot: &Snapshot,
   ) -> bool {
       self.with_page_data(page_id, |data| {
           let slotted = SlottedPageRef::new(data);
           for i in 0..slotted.slot_count() {
               let slot = match slotted.get_slot(i) {
                   Some(s) => s,
                   None => continue,
               };
               let slot_data = slotted.get_slot_data(&slot);
               if slot_data.len() < VersionHeader::SIZE {
                   return false;
               }
               let vh = match VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]) {
                   Some(v) => v,
                   None => return false,
               };
               // 三个条件：
               // 1. 已提交（非 None 且非删除标记）
               // 2. create_tx_id < snapshot.tx_id
               // 3. create_tx_id NOT IN active_tx_ids
               match vh.commit_tx_id() {
                   None | Some(u64::MAX) => return false,
                   _ => {}
               }
               if vh.create_tx_id() >= snapshot.tx_id() {
                   return false;
               }
               if snapshot.contains_active_tx(vh.create_tx_id()) {
                   return false;
               }
           }
           true
       }).await.unwrap_or(false)
   }
   ```

2. `src/transaction/snapshot.rs` — 新增 `contains_active_tx` 方法
   ```rust
   pub fn contains_active_tx(&self, tx_id: u64) -> bool {
       self.active_tx_ids.contains(&tx_id)
   }
   ```

3. `src/executor/data_scan.rs` — 页面扫描结束后惰性设置
   ```rust
   // 在 page exhausted 时检查
   if !page_all_visible && slot_index >= slot_count {
       if let Some(snapshot) = self.snapshot.as_ref() {
           if self.buffer_pool.check_page_all_visible(page_id, snapshot).await {
               self.buffer_pool.set_all_visible(page_id);
           }
       }
   }
   ```

**验证**：
- 已有 `test_visibility_all_visible_page_skips_per_row_checks` 通过
- benchmark 对比 all_visible 快速路径

**依赖**：T1（DELETE mark_deleted 影响 check 逻辑）

---

### T3: visibility benchmark

**目标**：量化 all_visible 快速路径收益

**改动**：
1. `benches/visibility_bench.rs` — 新建 benchmark
   ```rust
   // 场景 1: 全已提交表 × 10K 行 → all_visible 快速路径
   // 场景 2: 全未提交表 × 10K 行 → all_invisible 快速路径
   // 场景 3: 混合表 × 10K 行 → 无快速路径 baseline
   ```

2. `Cargo.toml` — 添加 bench 入口

**验证**：`cargo bench --bench visibility_bench` 运行成功

**依赖**：T2

---

### T4: 测试修复 + 全量回归

**目标**：修复已有测试，确保全量通过

**改动**：
1. `tests/visibility_test.rs` — 修复 DELETE 测试期望（5 → 4）
2. 新增测试：DELETE 后 DataScan 不返回已删除行
3. 新增测试：all_visible 惰性设置后快速路径生效

**验证**：
- `cargo test --lib --tests` 0 failures
- `cargo clippy` 0 new warnings
- `cargo fmt` 0 diff

**依赖**：T1, T2

---

## 执行顺序

```
T1 (DELETE mark_deleted)
  ↓
T2 (check_page_all_visible)
  ↓
T3 (benchmark) ← 可与 T4 并行
T4 (测试修复)
```

## 风险评估

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| mark_deleted 与 UNSET_TX_ID 冲突 | 中 | 高 | 用专用 sentinel 值 |
| check_page_all_visible 性能开销 | 低 | 中 | 仅在页面扫描结束时调用 |
| 并发 INSERT 破坏 all_visible | 低 | 中 | MVCC 三条件检查 |
