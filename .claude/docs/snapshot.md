# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: M1 完成（文件/缓存层已实现）
- **状态**: 正常
- **当前里程碑**: M2 准备开始

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
│       ├── buffer_pool.rs   # BufferPool（Clock 淘汰）
│       └── page_frame.rs    # PageFrame + PageGuard
│   ├── executor/mod.rs      # 执行引擎模块（占位符）
│   ├── transaction/mod.rs   # 事务管理模块（占位符）
│   ├── parser/mod.rs        # SQL 解析模块（占位符）
│   └── network/mod.rs       # 网络层模块（占位符）
├── tests/
│   ├── runtime_test.rs      # 运行时功能验证测试（3 个测试）
│   └── storage_test.rs      # 存储层测试（17 个测试）
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

**注**: M1 文件/缓存层已完成，包含 AsyncStorage trait、FileStorage、BufferPool（Clock 淘汰）和 PageGuard。

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
  - ac40676 feat(storage): complete M1 implementation with BufferPool and Clock eviction
  - 5121ebf feat(storage): implement PageFrame and PageGuard with ref counting
  - 09b71b6 feat(storage): implement FileStorage with async read/write/allocate/sync
  - f065b44 feat(storage): define AsyncStorage trait with async methods
  - bd717bc feat(storage): implement Page struct with 4KB fixed size
- **未提交更改**: 无（working tree clean）

**注**: M1 代码已全部提交，17 个测试通过，clippy 仅有 1 个可接受警告（await_holding_lock）。

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
| tests/storage_test.rs | 存储层测试 | ✅ 17 测试通过 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | src/storage/*, tests/storage_test.rs | M1 完整实现 |
| 2026-05-20 | Cargo.toml | 添加 async-trait/thiserror/anyhow/tempfile |
| 2026-05-20 | .claude/docs/superpowers/* | M1 设计规范和实现计划 |
| 2026-05-20 | src/*, tests/* | M0 骨架实现 |

## 下一步行动

1. 开始 M2 里程碑：B-Tree 索引与存储引擎
2. 实现同步 B-Tree 索引内核
3. 通过 `spawn_blocking` 暴露为 async API
4. 实现 Slotted Page 行存储格式