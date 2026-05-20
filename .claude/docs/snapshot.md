# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: M2 完成（B-Tree 索引与存储引擎已实现）
- **状态**: 正常
- **当前里程碑**: M3 准备开始

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置，含 Tokio/async-trait/thiserror/anyhow/tempfile 依赖
├── Cargo.lock              # 依赖锁定文件
├── .gitignore              # Git 忽略配置
├── CLAUDE.md               # 文档入口
├── src/
│   ├── main.rs             # 数据库服务器入口（#[tokio::main]）
│   ├── lib.rs              # 库入口，导出模块公共接口
│   └── storage/
│       ├── mod.rs          # 存储模块导出
│       ├── error.rs        # StorageError 错误类型
│       ├── page_id.rs      # PageId 结构
│       ├── page.rs         # Page 结构（4KB 固定大小）
│       ├── async_storage.rs # AsyncStorage trait
│       ├── file_storage.rs  # FileStorage 实现（spawn_blocking I/O）
│       ├── buffer_pool.rs   # BufferPool（Clock 淘汰）+ storage() 方法
│       ├── page_frame.rs    # PageFrame + PageGuard + modify_page()
│       ├── page_format/     # M2 新增：页格式模块
│       │   ├── mod.rs       # 模块导出
│       │   ├── key.rs       # Key 结构（固定 32 bytes）
│       │   ├── row_id.rs    # RowId 结构（page_id + slot_id）
│       │   └── slotted_page.rs # SlottedPage 通用格式
│       └── btree/           # M2 新增：B-Tree 索引模块
│           ├── mod.rs       # 模块导出
│           ├── node.rs      # LeafNode + InternalNode 结构
│           ├── btree.rs     # BTree 核心逻辑
│           ├── sync_loader.rs # SyncPageLoader（block_on 包装）
│           └── index_manager.rs # IndexManager 异步 API
│   ├── executor/mod.rs      # 执行引擎模块（占位符）
│   ├── transaction/mod.rs   # 事务管理模块（占位符）
│   ├── parser/mod.rs        # SQL 解析模块（占位符）
│   └── network/mod.rs       # 网络层模块（占位符）
├── tests/
│   ├── runtime_test.rs      # 运行时功能验证测试（3 个测试）
│   ├── btree_test.rs        # M2 新增：BTree 核心测试（10 个测试）
│   ├── index_manager_test.rs # M2 新增：IndexManager 异步测试（3 个测试）
│   └── sync_loader_test.rs  # M2 新增：SyncPageLoader 测试（2 个测试）
└── .claude/
    └── docs/
        ├── architecture.md    - 架构决策记录
        ├── learned.md         - 学习记忆
        ├── optimization.md    - 优化方向与技术债务
        ├── references.md      - 外部参考资料
        ├── rules.md           - 编码规范与行为约束
        ├── snapshot.md        - 项目状态快照
        ├── tasks.md           - 任务清单
        └── superpowers/
            ├── specs/         - 设计规范
            └─ plans/          - 实现计划
```

**注**: M2 B-Tree 索引与存储引擎已完成，包含 Key、RowId、SlottedPage、LeafNode、InternalNode、BTree、SyncPageLoader、IndexManager，53 个测试全部通过。

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 最新稳定版（建议 1.75+） |
| 构建工具 | Cargo | Rust 内置 |
| 异步运行时 | Tokio | 1.x（多线程 scheduler） |
| SQL 解析 | sqlparser-rs | 待集成 |
| 测试框架 | tempfile + 内置测试 | 3.x |
| 代码格式化 | rustfmt | Rust 内置 |
| Lint | clippy | Rust 内置 |

## Git 状态

- **当前分支**: master
- **最近提交**:
  - 30c2f1d feat(storage): complete M2 implementation with B-Tree index and storage engine
  - 5beb916 feat(m2-btree): implement IndexManager async API
  - f4117f8 feat(m2-btree): implement BTree core logic with proper page write-back
  - 68946e5 test(m2-btree): add failing tests for BTree core logic
  - 2e6afc8 feat(m2-btree): implement SyncPageLoader with async wrapper
  - 5b7d532 feat(btree): implement LeafNode and InternalNode structures
  - c939ff0 feat(page_format): implement SlottedPage with slot array and row data layout
  - 3b7bf47 feat(page_format): implement RowId structure (page_id + slot_id)
  - 76469a9 feat(page_format): implement Key structure with fixed 32 bytes length
- **未提交更改**: 无（working tree clean）

**注**: M2 代码已全部提交，53 个测试通过，clippy 有 11 个可接受警告。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| Cargo.toml | Rust 项目配置 | ✅ 完成 |
| src/storage/mod.rs | 存储模块导出 | ✅ 完成 |
| src/storage/error.rs | StorageError 类型 | ✅ 完成 |
| src/storage/page_id.rs | PageId 结构 | ✅ 完成 |
| src/storage/page.rs | Page 结构（4KB） | ✅ 完成 |
| src/storage/async_storage.rs | AsyncStorage trait | ✅ 完成 |
| src/storage/file_storage.rs | FileStorage 实现 | ✅ 完成 |
| src/storage/buffer_pool.rs | BufferPool + Clock 淘汰 | ✅ 完成 |
| src/storage/page_frame.rs | PageFrame + PageGuard | ✅ 完成 |
| src/storage/page_format/mod.rs | 页格式模块导出 | ✅ M2 完成 |
| src/storage/page_format/key.rs | Key 结构（32 bytes） | ✅ M2 完成 |
| src/storage/page_format/row_id.rs | RowId 结构 | ✅ M2 完成 |
| src/storage/page_format/slotted_page.rs | SlottedPage 格式 | ✅ M2 完成 |
| src/storage/btree/mod.rs | B-Tree 模块导出 | ✅ M2 完成 |
| src/storage/btree/node.rs | LeafNode + InternalNode | ✅ M2 完成 |
| src/storage/btree/btree.rs | BTree 核心逻辑 | ✅ M2 完成 |
| src/storage/btree/sync_loader.rs | SyncPageLoader | ✅ M2 完成 |
| src/storage/btree/index_manager.rs | IndexManager API | ✅ M2 完成 |
| tests/btree_test.rs | BTree 测试 | ✅ M2 完成（10 测试）|
| tests/index_manager_test.rs | IndexManager 测试 | ✅ M2 完成（3 测试）|
| tests/sync_loader_test.rs | SyncPageLoader 测试 | ✅ M2 完成（2 测试）|

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | src/storage/page_format/*, tests/page_format_test | M2 Key/RowId/SlottedPage 实现 |
| 2026-05-20 | src/storage/btree/*, tests/btree_* | M2 B-Tree 索引与存储引擎 |
| 2026-05-20 | .claude/docs/superpowers/* | M2 设计规范和实现计划 |
| 2026-05-20 | src/storage/*, tests/storage_test.rs | M1 文件/缓存层完整实现 |

## 下一步行动

1. 开始 M3 里程碑：事务与 MVCC
2. 实现全局事务 ID 分配（`AtomicU64`）
3. 实现 MVCC 快照读（无锁）
4. 实现异步读写锁（`tokio::sync::RwLock`）