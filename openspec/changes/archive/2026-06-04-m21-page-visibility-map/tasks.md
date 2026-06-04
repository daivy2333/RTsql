## 1. PageVisibilityInfo 数据结构 + BufferPool 集成 (T1) ✅

- [x] 1.1 新增 `PageVisibilityInfo` 结构体（`src/storage/page_visibility.rs`）
- [x] 1.2 `BufferPool` 新增 `vis_map: DashMap<PageId, PageVisibilityInfo>` + 4 公开方法
- [x] 1.3 `src/storage/mod.rs` 导出新模块 + 4 单元测试通过
- **验收**: ✅ `cargo test page_visibility` 4/4 passed

## 2. 扫描路径快速路径集成 (T2) ✅

- [x] 2.1 `BufferPool::find_visible_version` 页面级快速路径：`all_visible` + `all_invisible_for` 检查
- [x] 2.2 `DataScanExecutor::next()` 页面级快速路径：闭包外查询 visibility_map，闭包内 skip/skip-page
- [x] 2.3 惰性设置 `all_visible`：延后实现（Plan Agent 建议：避免竞态条件，先保证正确性再优化）
- **验收**: ✅ 129 lib + 全量集成测试通过 (0 failures)

## 3. 写路径可见性摘要更新 (T3) ✅

- [x] 3.1 INSERT 路径：`InsertExecutor` 调用 `clear_all_visible` + `update_visibility_on_insert`
- [x] 3.2 DELETE 路径：`delete_tuple_from_data_page` 调用 `clear_all_visible`
- [x] 3.3 UPDATE 路径：`UpdateExecutor` 清新旧两页的 `all_visible`
- [x] 3.4 COMMIT 路径：`commit_mark_versions` 循环内调用 `clear_all_visible`（Plan Agent 发现的缺口）
- **验收**: ✅ `cargo test --tests` 全部通过

## 4. 可见性检查基准测试 (T4) ⏸️ 延后

- [ ] 4.1 新增 `benches/visibility_bench.rs`
- [ ] 4.2 `Cargo.toml` 注册新 bench 入口
- [ ] 4.3 criterion 基准测试 + 数据对比
- **延后原因**：`set_all_visible` 暂无调用者（惰性设置延后），基准测试需等可见性摘要实际产生效果后再添加
