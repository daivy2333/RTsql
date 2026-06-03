## Context

**当前状态**：
- `SlottedPageRef<'a>` (`src/storage/page_format/slotted_page.rs:78`) — 零拷贝只读视图，**已存在**
- `LeafNodeRef<'a>` (`src/storage/btree/node.rs:517`) / `InternalNodeRef<'a>` (`node.rs:964`) — 零拷贝视图，**已存在**并被 B-Tree 内部使用
- `PageDataGuard<'a>` (`src/storage/page_frame.rs:32`) — 持有 `MutexGuard<'a, PageFrame>`，提供 `Deref<Target=[u8]>` 零拷贝访问，**已存在**
- `BufferPool::get_page()` 返回 `PageGuard`（拥有所有权），调用方需调用 `page_data()` 显式获取只读 guard

**瓶颈**：
- `read_tuple_from_data_page` (`src/storage/data_page.rs:65-92`) 在第 89 行执行 `tuple_bytes[VersionHeader::SIZE..].to_vec()`，将页内 slice 拷贝到堆
- `find_visible_version` (`src/storage/buffer_pool.rs:174-203`) 返回 `Option<Vec<u8>>`，版本链遍历时每跳一版都做一次堆分配
- 3 个 Scan 执行器（`ScanExecutor` / `IndexScanExecutor` / `IndexScanAllExecutor`）均通过上述两函数读取数据，扫描 1K 行约 1K 次 Vec 分配

**约束**：
- Rust 借用 + 异步：跨 `.await` 不能持有借用（无 Send），需用 guard scope 模式（参照 `src/storage/btree/btree.rs:164-193` 的 `search_from_page_async`）
- 写路径（`update_version_header_in_data_page` / `delete_tuple_from_data_page`）**必须不动**（Metis 指令）
- 不引入新 trait / 抽象层（Metis 指令）
- 不能引入回归（cargo test 全部通过，其他基准 < 5% 波动）

## Goals / Non-Goals

**Goals**：
- 为数据页 tuple 读取提供零拷贝 API（`&[u8]` 借用 + guard 持有生命周期）
- 消除 `read_tuple_from_data_page` / `find_visible_version` 的 `Vec<u8>` 堆分配
- 3 个 Scan 执行器全部切换到零拷贝路径
- 验收：Full Scan 1K rows ≥ 15% 提速，写路径 0 回归

**Non-Goals**：
- 不动 `SlottedPageRef` / `LeafNodeRef` / `InternalNodeRef` 已有 API（B-Tree 已在用）
- 不动写路径任何代码
- 不实现 M19 (DataScan) / M36 (零拷贝 ValueRef) — 本次只解决"页 → tuple bytes"环节
- 不引入 `bytes::Bytes` / `Arc<[u8]>` 等新共享所有权原语

## Decisions（闭包方案 — 2026-06-03 修订）

> **修订原因**：原决策 1-2（返回 `PageDataGuard<'_>` / tuple `(PageGuard, PageDataGuard<'_>)`）在 safe Rust 中不可行（E0505 / self-referential struct hang），详见 `learned/spec.md` L022。现全面改为闭包 API。

### 决策 1: 新增 `BufferPool::with_page_data()` 闭包 API

**选择**：在 `BufferPool` 上新增闭包方法，替代原 `get_page_ref`：

```rust
/// 零拷贝读取页数据。闭包内 &[u8] 有效，闭包结束后释放锁。
///
/// SAFETY: 闭包内不可 .await（MutexGuard 不可跨 await 持有）。
///         闭包内不可递归调用 BufferPool 方法（死锁）。
pub async fn with_page_data<F, R>(&self, page_id: PageId, f: F) -> Result<R>
where
    F: FnOnce(&[u8]) -> Result<R>,
{
    let guard = self.get_page(page_id).await?;
    let data_guard = guard.page_data();
    f(&data_guard)
}
```

**理由**：
- 闭包是 Rust 异步 + 锁借用的标准范式（guard scope 由闭包自然控制）
- `FnOnce(&[u8]) -> Result<R>` 支持闭包内错误传播（`SlotNotFound` 等）
- 与项目现有 `PageGuard::modify_page` 闭包模式一致
- 不需要 `Box::pin` / `AsyncFnOnce` — 闭包内是同步操作（`SlottedPageRef` 解析、`deserialize_tuple`）

**备选方案**：
- (A) 返回 `PageDataGuard<'_>` — safe Rust 不可行（E0505），unsafe hang
- (B) 返回 `(PageGuard, PageDataGuard<'_>)` tuple — 编译不过（E0505）
- (C) `PageGuard::Clone` + tuple — 需补 Clone 实现，语义不清晰
- ✅ (D) 闭包 API — 最小改动，编译安全，与项目范式一致

### 决策 2: `read_tuple_from_data_page` 改为闭包形式

**选择**：BREAKING 签名变更：

```rust
/// 零拷贝读取 tuple。闭包接收 (VersionHeader, &[u8])，
/// 其中 &[u8] 是 tuple payload（不含 VersionHeader）。
/// 闭包结束后释放页锁。
pub async fn read_tuple_from_data_page<F, R>(
    buffer_pool: &BufferPool,
    row_id: RowId,
    f: F,
) -> Result<R>
where
    F: FnOnce(VersionHeader, &[u8]) -> Result<R>,
{
    let page_id = PageId(row_id.page_id as u64);
    buffer_pool.with_page_data(page_id, |data| -> Result<R> {
        let slotted = SlottedPageRef::new(data);
        let (slot, _) = slotted
            .get_slot_by_logical_id(row_id.slot_id)
            .ok_or(StorageError::SlotNotFound(row_id))?;
        let slot_data = slotted.get_slot_data(&slot);
        let version_header = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
            .ok_or_else(|| StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData, "malformed version header",
            )))?;
        let tuple_bytes = &slot_data[VersionHeader::SIZE..];
        f(version_header, tuple_bytes)
    }).await?
}
```

**调用方迁移示例**：

```rust
// 之前
let (_, tuple_bytes) = read_tuple_from_data_page(&pool, row_id).await?;
let values = deserialize_tuple(&tuple_bytes, &schema)?;

// 之后（零拷贝，无 Vec 分配）
let values = read_tuple_from_data_page(
    &pool, row_id,
    |_vh, bytes| deserialize_tuple(bytes, &schema),
).await?;

// 之后（需要 Vec 的写路径，闭包内 to_vec）
let (vh, tuple_bytes) = read_tuple_from_data_page(
    &pool, row_id,
    |vh, bytes| Ok((vh, bytes.to_vec())),
).await?;
```

**备选方案**：
- (A) 保留 `Vec<u8>` + 新增 zero-copy 版本 — 双 API 技术债
- ✅ (B) BREAKING 改唯一签名 — 编译期强制迁移，无遗漏

### 决策 3: `find_visible_version` 改为闭包形式

**选择**：BREAKING 签名变更，闭包内零拷贝消费可见版本：

```rust
/// 遍历版本链找可见版本。闭包接收可见 tuple 的 &[u8]，返回 Result<R>
/// （让 deserialize_tuple 等错误自动传播）。
/// 可见版本在页锁内零拷贝传递给闭包。
/// 不可见版本只读 VersionHeader（8B），不拷贝 tuple payload。
pub async fn find_visible_version<F, R>(
    &self,
    row_id: RowId,
    snapshot: &Snapshot,
    f: F,
) -> Result<Option<R>>
where
    F: FnOnce(&[u8]) -> Result<R>,
```

**核心设计**：引入 `VisibilityResult<R>` 辅助枚举：

```rust
enum VisibilityResult<R> {
    Visible(R, Option<RowId>),   // (闭包结果, next_version)
    NotVisible(Option<RowId>),   // next_version 供继续遍历
    NotFound,                    // slot 不存在
}
```

实现要点：
- `f` 用 `Option<F>` 包装，`take()` 确保只消费一次
- 每次迭代：`with_page_data` 闭包内解析 `SlottedPageRef` + 判断可见性
- 可见：在同一闭包内调用 `f(tuple_bytes)` 零拷贝消费，返回 `VisibilityResult::Visible`
- 不可见：只返回 `VisibilityResult::NotVisible(next_version)`，不拷贝 tuple payload
- 闭包结束后页锁释放，下一迭代获取新一页锁

**性能收益**：
- 不可见版本：省掉整个 tuple 的 `.to_vec()`（只读 8B VersionHeader）
- 可见版本：零拷贝 `&[u8]` 直接传给 `deserialize_tuple`

**备选方案**：
- (A) 返回 `Option<(&[u8], PageDataGuard<'_>)>` — 借用逃逸
- (B) 拆成 `find_visible_version_header` + `get_tuple_bytes` — 两次页访问
- ✅ (C) 闭包形式 — 借用 scope 自然控制，零拷贝消费可见版本

### 决策 4: 删除 `get_page_ref`，不引入 `Box::pin` 模式

**选择**：删除当前编译不过的 `get_page_ref`，所有零拷贝访问统一走 `with_page_data` 闭包。

**理由**：
- `get_page_ref` 返回 `(PageGuard, PageDataGuard<'_>)` 编译不过（E0505）
- 闭包方案完全替代其功能，不需要 `Box::pin` + guard scope 模式
- 闭包内是同步操作，不需要 `Box::pin` 钉堆

### 决策 5: 写路径适配策略

**选择**：写路径（`write_commit_tx_id` / `UpdateExecutor`）在闭包内 `.to_vec()` 获取 `Vec<u8>`。

```rust
// write_commit_tx_id
let (version_header, tuple_bytes) = read_tuple_from_data_page(
    self, row_id, |vh, bytes| Ok((vh, bytes.to_vec()))
).await?;

// UpdateExecutor
let (_version_header, old_tuple_bytes) = read_tuple_from_data_page(
    &self.buffer_pool, old_row_id, |vh, bytes| Ok((vh, bytes.to_vec()))
).await?;
```

**理由**：
- 写路径需要 `Vec<u8>` 所有权（WAL 记录、版本链更新）
- `.to_vec()` 在闭包内执行，等价于原实现
- 不动 `update_version_header_in_data_page` / `delete_tuple_from_data_page` 本体

### 决策 6: Scan 执行器改造

**选择**：3 个 Scan 执行器统一改为闭包调用，消除 `Vec<u8>` 中间分配。

| 执行器 | 当前 | 改造后 |
|--------|------|--------|
| `ScanExecutor` (有 snapshot) | `find_visible_version` → `Vec<u8>` → `deserialize_tuple` | `find_visible_version(row_id, snapshot, \|bytes\| deserialize_tuple(bytes, &schema))` |
| `ScanExecutor` (无 snapshot) | `read_tuple_from_data_page` → `Vec<u8>` → `deserialize_tuple` | `read_tuple_from_data_page(&pool, row_id, \|_vh, bytes\| deserialize_tuple(bytes, &schema))` |
| `IndexScanExecutor` | 同上 | 同上 |
| `IndexScanAllExecutor` | 同上 | 同上 |

### 决策 7: `read_version_header` / `write_commit_tx_id` 适配

```rust
// read_version_header — 只需 VersionHeader，零拷贝
pub async fn read_version_header(&self, row_id: RowId) -> Result<VersionHeader> {
    read_tuple_from_data_page(self, row_id, |vh, _bytes| Ok(vh)).await
}

// write_commit_tx_id — 需要 Vec<u8>，闭包内 to_vec()
pub async fn write_commit_tx_id(&self, row_id: RowId, commit_tx_id: u64) -> Result<()> {
    let (version_header, tuple_bytes) = read_tuple_from_data_page(
        self, row_id, |vh, bytes| Ok((vh, bytes.to_vec()))
    ).await?;
    let new_header = version_header.commit(commit_tx_id);
    crate::storage::update_version_header_in_data_page(self, row_id, new_header, &tuple_bytes).await?;
    Ok(())
}
```

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| **闭包内递归调用 BufferPool** → 死锁 | `with_page_data` 文档注释明确禁止；编译期无法强制，靠 API 文档 + review |
| **闭包内 .await** → MutexGuard 跨 await | clippy `await_holding_lock` 检查；闭包签名是 `FnOnce`（非 async），编译期强制 |
| **版本链遍历锁堆积** → 连续持有 page guard 触发死锁 | 每次迭代闭包结束后页锁自动释放；不可见版本只读 8B VersionHeader |
| **API BREAKING 漏改** → 3 个执行器 + 测试 + write_commit_tx_id | 编译期强制 — 旧 `Vec<u8>` 签名完全删除，调用方必须同步改 |
| **写路径误改** → 开发者顺手优化写路径函数 | 写路径 commit 中禁止出现写路径函数引用的改动 |
| **基准噪声掩盖回归** → criterion 波动大 | `--save-baseline before-m20` + `--baseline before-m20`，阈值看中位数差异 |
| **性能提升不达 15%** → 分配器已优 | 改测小行数 (< 100B) 场景；行越大 Vec 分配占比越高 |
| **`find_visible_version` 闭包 `Option<F>` + `take()`** → 风格争议 | 比自引用 struct / unsafe 更安全；比两次页访问更快 |

## Migration Plan

**步骤**：
1. 新增 `with_page_data` + `VisibilityResult<R>` 枚举（buffer_pool.rs）
2. 删除 `get_page_ref`（buffer_pool.rs）
3. 改 `read_tuple_from_data_page` 为闭包形式（data_page.rs）
4. 改 `find_visible_version` 为闭包形式（buffer_pool.rs）
5. 改 `read_version_header` / `write_commit_tx_id` 适配闭包（buffer_pool.rs）
6. 改 `ScanExecutor::next`（scan.rs）— 两条路径（有/无 snapshot）
7. 改 `IndexScanExecutor::next`（index_scan.rs）— 两条路径
8. 改 `IndexScanAllExecutor::next`（index_scan_all.rs）— 两条路径
9. 改 `UpdateExecutor::next`（update.rs）— 闭包内 `.to_vec()`
10. 改 `data_page.rs` 内单元测试 → 闭包调用
11. 改 `storage_test.rs` 内测试 → 闭包调用
12. 跑全量 `cargo test` 验证 0 失败
13. 跑 `cargo clippy` 验证 0 warnings
14. 跑 `cargo bench --bench single -- --save-baseline before-m20` 留底
15. 提交 git commit（feat 风格）
16. 跑 `cargo bench --bench single -- --baseline before-m20` 对比，确认 ≥ 15% 提升
17. 跑 micro 套件确认无回归
18. 归档 OpenSpec 变更

**回滚**：
- 单 commit revert 即可
- 不涉及数据迁移 / schema 变更 / 外部 API 破坏
- 风险窗口：commit 到下一次 production 部署前

## Open Questions

- **Q1**: `find_visible_version` 闭包是否需要 `AsyncFnOnce`？ — 当前所有调用方都是同步（`deserialize_tuple` 是同步函数），用 `FnOnce` 即可。M36 (零拷贝 ValueRef) / M29 (PG Extended Query) 可能需要 async，届时再升级
- **Q2**: 是否需要 `with_page_data_many(page_ids: &[PageId])` 批量 API？ — 不在 M20 范围，M43 (并行扫描) 时再考虑
- **Q3**: `VisibilityResult<R>` 是否应定义为独立类型？ — 当前只在 `find_visible_version` 内部使用，定义为 `buffer_pool.rs` 内 private enum 即可
- **Q4**: 闭包内 `f` 用 `Option<F>` + `take()` 是否有更优雅的方式？ — Rust 不支持 `FnOnce` 多次检查是否已调用，`Option<F>` 是标准模式
