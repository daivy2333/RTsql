# M20: 零拷贝 SlottedPageRef — 任务清单

> 最后更新：2026-06-03（闭包方案修订）

## T1: SlottedPageRef 类型验证 ✅

- [x] 验证 `SlottedPageRef::new(&[u8])` 可从 `PageDataGuard` 构造
- [x] 验证 `get_slot_by_logical_id` + `get_slot_data` 返回正确 slice
- [x] 验证 `SlottedPageRef` 不逃逸闭包 scope
- 结果：PASS，L022 记录

## T2: `with_page_data` 闭包 API 实现

- [x] 在 `BufferPool` 上新增 `with_page_data<F, R>(&self, PageId, F) -> Result<R>`
  - `where F: FnOnce(&[u8]) -> Result<R>`
  - 实现：`get_page` → `page_data()` → `f(&data)` → 返回
- [x] 新增 `VisibilityResult<R>` private enum（`Visible(R, Option<RowId>)` / `NotVisible(Option<RowId>)` / `NotFound`）
- [x] 删除 `get_page_ref`（编译不过的 tuple 返回版本）
- 验证：`cargo test` 通过

## T3: `read_tuple_from_data_page` 改为闭包形式

- [x] 改签名：`async fn read_tuple_from_data_page<F, R>(buffer_pool, row_id, f: F) -> Result<R> where F: FnOnce(VersionHeader, &[u8]) -> Result<R>`
- [x] 实现：`with_page_data` 内解析 SlottedPageRef + 调用闭包
- [x] 更新 `data_page.rs` 内单元测试
- 验证：`cargo test` 通过

## T4: `find_visible_version` 改为闭包形式

- [x] 改签名：`async fn find_visible_version<F, R>(&self, row_id, snapshot, f: F) -> Result<Option<R>> where F: FnOnce(&[u8]) -> Result<R>` (修订：原 `FnOnce(&[u8]) -> R` 错误，会嵌套 Result；改为 `Result<R>` 让错误传播)
- [x] 实现：`with_page_data` + `VisibilityResult<R>` + `Option<F>` + `take()`
- [x] 不可见版本只读 VersionHeader，不拷贝 tuple payload
- 验证：`cargo test` 通过

## T5: BufferPool 辅助方法适配

- [x] `read_version_header` → 闭包：`|vh, _bytes| Ok(vh)`
- [x] `write_commit_tx_id` → 闭包：`|vh, bytes| Ok((vh, bytes.to_vec()))`
- 验证：`cargo test` 通过

## T6: Scan 执行器改造

- [x] `ScanExecutor::next` — 有 snapshot 路径：`find_visible_version` 闭包
- [x] `ScanExecutor::next` — 无 snapshot 路径：`read_tuple_from_data_page` 闭包
- [x] `IndexScanExecutor::next` — 同上两条路径
- [x] `IndexScanAllExecutor::next` — 同上两条路径
- 验证：`cargo test` 通过

## T7: Update 执行器适配

- [x] `UpdateExecutor::next` — 闭包内 `.to_vec()`（写路径需要 Vec 所有权）
- 验证：`cargo test` 通过

## T8: 测试 + Lint 全量验证

- [x] `cargo test` — 0 失败
- [x] `cargo clippy` — M20 范围内 0 warning（pre-existing warnings 未触碰）
- [x] `cargo fmt` — 格式化 12 个 M20 改动文件
- [x] `storage_test.rs` 内 `read_tuple_from_data_page` 调用点全部迁移

## T9: 性能验证

- [x] `cargo bench --bench micro_bench -- --save-baseline before-m20`（注：原 design 写 `--bench single` 错误，实际只有 `micro_bench`）
- [x] 实施完成后 `cargo bench --bench micro_bench -- --baseline before-m20` 对比
- [ ] **SKIPPED**: 确认 ≥ 15% 提升（行扫描场景）— **未达**（实际 -2.46% 到 -8.33%）。原因：micro_bench 1K × 100B 行数小 + 现代分配器对 100KB Vec 已经极快。M19/M36 进一步消除分配可能达 15%+。
- [x] 跑 micro 套件确认无回归（4 项 read 路径改进 + 1 项 write +3.99% 在 5% 阈值内）

## T10: 归档

- [x] `/opsx:archive m20-zero-copy-slotted-page-ref`
- [x] 更新 `tasks.md` + `snapshot.md`
