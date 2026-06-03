## Why

`SlottedPage` 与 `LeafNode`/`InternalNode` 的零拷贝只读视图类型 (`SlottedPageRef<'a>` / `LeafNodeRef<'a>` / `InternalNodeRef<'a>`) **已在 M13/M14 阶段实现并被 B-Tree 内部使用**，但执行器 Scan 路径读取数据页 tuple 时仍通过 `read_tuple_from_data_page` 返回 `Vec<u8>` 拷贝、`find_visible_version` 遍历版本链同样返回 `Vec<u8>`，每次读出都要做 4KB 页内 slice 的堆分配。Phase 1 基础设施（M41/M30/M38）已全部完成，进入 Phase 2 存储引擎核心优化，M20 是 M19 (DataScan) / M36 (零拷贝 ValueRef) 的前置依赖，必须先打通"页 → 元组"的零拷贝链路。

## What Changes

- **BufferPool 新增 `get_page_ref(page_id)` API**：返回 `PageDataGuard<'_>`（零拷贝只读），调用方拿到 `&[u8]` 后可构造 `SlottedPageRef<'_>` 直接读取 slot 数据，生命周期由 guard 持有
- **重写 `read_tuple_from_data_page`**：返回 `(VersionHeader, &[u8])` 而非 `(VersionHeader, Vec<u8>)`，消除 `.to_vec()` 分配
- **重写 `find_visible_version`**：改为遍历闭包形式，调用方在闭包作用域内持有 page guard 引用，避免 Vec 分配
- **执行器 Scan 路径适配**：`ScanExecutor` / `IndexScanExecutor` / `IndexScanAllExecutor` 全部切换到零拷贝 API，row 数据通过 `&[u8]` 引用传递，跨 await 时显式 drop 前一页 guard
- **新增零拷贝基准对比**：在 `benches/` 套件中对比改前改后 Full Scan 1K rows 性能，验收门槛 ≥ 15% 提速，回归 < 5%

**不做什么**：
- 不动写路径 (`write_tuple_to_data_page` / `update_version_header_in_data_page` / `delete_tuple_from_data_page`) — Metis 明确禁止，避免引入锁顺序/可见性风险
- 不新增 trait 或抽象层 — 现有 `PageDataGuard` + `SlottedPageRef` 组合已足够
- 不改动 `SlottedPageRef` / `LeafNodeRef` / `InternalNodeRef` 的现有 API（已存在且 B-Tree 内部在用）

## Capabilities

### New Capabilities
- `zero-copy-page-access`: 数据页 tuple 读取的零拷贝 API（`BufferPool::get_page_ref` + `read_tuple_from_data_page` 重构 + 闭包式版本链遍历）

### Modified Capabilities
（无现有 spec 涵盖此能力，全部为新增）

## Impact

**影响代码**（按行数估算）：
- `src/storage/buffer_pool.rs`: +30 行（`get_page_ref` 新方法）、`find_visible_version` 重构 -10/+25 行
- `src/storage/data_page.rs`: `read_tuple_from_data_page` 重构 -5/+15 行
- `src/executor/scan.rs`: 适配零拷贝 API -20/+30 行
- `src/executor/index_scan.rs`: 适配 -20/+30 行
- `src/executor/index_scan_all.rs`: 适配 -20/+30 行
- `benches/single_bench.rs`: 新增 baseline 对比逻辑 +20 行

**影响测试**（必须保持全绿）：
- `tests/storage_test.rs` — 写路径测试
- `tests/executor_test.rs` — Scan 执行器测试
- `tests/mvcc_commit_test.rs` — MVCC 提交测试
- `tests/version_chain_test.rs` — 版本链测试

**性能影响**：
- 预期 Full Scan 1K rows ≥ 15% 提速（消除 4KB × N 的 Vec 分配与拷贝）
- 无回归（其他基准 < 5% 波动）
- 内存分配器压力降低（高频 Scan 场景下尤为明显）

**风险**：
- Rust lifetime 约束 + async 跨 await — 参照 `src/storage/btree/btree.rs:164-193` 的 guard scope 模式
- 版本链遍历时多页 guard 顺序 drop — 防止锁堆积
- API 是 **BREAKING** 变化：所有调用 `read_tuple_from_data_page` / `find_visible_version` 的代码需同步改造

**回滚方案**：
- 全部变更在单一 git commit 内
- 通过 `git revert <commit>` 即可回滚到 M20 前状态
- 涉及 3 个执行器，但接口边界清晰，影响范围可控

**相关 ADR**：
- 引用已有 `architecture/spec.md` 中关于零拷贝页访问的 ADR 设计决策
- 本变更不产生新 ADR，但作为"零拷贝读路径"原则的具体实施
