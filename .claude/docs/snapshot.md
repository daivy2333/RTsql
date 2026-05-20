# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: M6 完成（网络层已实现）
- **状态**: 正常
- **当前里程碑**: M7 准备开始

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置，含 Tokio/async-trait/thiserror/anyhow/sqlparser 依赖
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
│   ├── executor/           # M4-M5: 执行引擎模块
│   │   ├── mod.rs          # 模块导出
│   │   ├── value.rs        # Value enum（Int/String/Null + to_key()）
│   │   ├── plan.rs         # PhysicalPlan + 5 节点结构
│   │   ├── result.rs       # ExecResult（RowId/AffectedRows/NotImplemented）[M5]
│   │   ├── executor_trait.rs # Executor trait（async next）[M5]
│   │   ├── scan.rs         # ScanExecutor [M5]
│   │   ├── index_scan.rs   # IndexScanExecutor [M5]
│   │   ├── insert.rs       # InsertExecutor [M5]
│   │   ├── update.rs       # UpdateExecutor [M5]
│   │   └── delete.rs       # DeleteExecutor [M5]
│   ├── transaction/        # M3 新增：事务管理模块
│   │   ├── mod.rs          # 模块导出
│   │   ├── tx_id.rs        # TransactionId（AtomicU64）
│   │   ├── error.rs        # TransactionError
│   │   ├── snapshot.rs     # Snapshot（可见性判断）
│   │   ├── version_chain.rs # VersionHeader（版本链）
│   │   ├── row_lock.rs     # RowLockTable（行级锁）
│   │   └── manager.rs      # TransactionManager
│   ├── parser/             # M4 新增：SQL 解析模块
│   │   ├── mod.rs          # 模块导出
│   │   ├── error.rs        # PlanError（7 种错误类型）
│   │   ├── value.rs        # Value 转换函数
│   │   ├── ast.rs          # AST 辅助函数
│   │   └── planner.rs      # PlanBuilder（AST → PhysicalPlan）
│   └── network/            # M6 新增：网络层模块
│       ├── mod.rs          # 模块导出
│       ├── error.rs        # NetworkError
│       ├── protocol.rs     # Protocol trait + JsonProtocol + Request/Response
│       ├── connection.rs   # ConnectionHandler
│       ├── handler.rs      # SqlHandler（mock executor）
│       └── server.rs       # Server + TcpListener + shutdown
├── tests/
│   ├── runtime_test.rs     # 运行时功能验证测试（3 个测试）
│   ├── btree_test.rs       # M2 新增：BTree 核心测试（10 个测试）
│   ├── index_manager_test.rs # M2 新增：IndexManager 异步测试（3 个测试）
│   ├── sync_loader_test.rs # M2 新增：SyncPageLoader 测试（2 个测试）
│   ├── concurrent_test.rs  # M3 新增：并发事务测试（4 个测试）
│   ├── parser_test.rs      # M4 新增：SQL 解析测试（6 个测试）
│   ├── planner_test.rs     # M4 新增：计划构建测试（8 个测试）
│   ├── executor_test.rs    # M5 新增：Executor 单元测试（7 个测试）
│   ├── plan_exec_test.rs   # M5 新增：集成测试（4 个测试）
│   ├── network_protocol_test.rs # M6 新增：JSON 协议测试（5 个测试）
│   └── network_server_test.rs   # M6 新增：Server 集成测试（4 个测试）
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

**注**: M6 网络层已完成，包含 Protocol trait + JsonProtocol + Server + ConnectionHandler + SqlHandler，124 个测试全部通过。

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 最新稳定版（建议 1.75+） |
| 构建工具 | Cargo | Rust 内置 |
| 异步运行时 | Tokio | 1.x（多线程 scheduler + io-util） |
| SQL 解析 | sqlparser-rs | 0.44 ✅ 已集成 |
| 协议层 | serde + serde_json | 1.0 ✅ 已集成 |
| Shutdown | tokio-util (CancellationToken) | 0.7 ✅ 已集成 |
| 测试框架 | tempfile + 内置测试 | 3.x |
| 代码格式化 | rustfmt | Rust 内置 |
| Lint | clippy | Rust 内置 |

## Git 状态

- **当前分支**: master
- **最近提交**:
  - 8ac8913 style(m6): apply cargo fmt formatting
  - dbc245c test(m6): add Server integration tests for query/insert/ping flows
  - 46a10f9 feat(m6): implement Server with TcpListener and graceful shutdown
  - c6ac485 feat(m6): implement ConnectionHandler for per-connection coroutine
  - b48ab0a feat(m6): implement SqlHandler with mock executor
  - 70748af test(m6): add JsonProtocol serialization/deserialization tests
- **未提交更改**: 无（working tree clean）

**注**: M6 代码已全部提交，124 个测试通过，clippy 有 minor warnings（无 Critical）。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| Cargo.toml | Rust 项目配置 | ✅ 完成 |
| src/lib.rs | 库入口，模块导出 | ✅ M4 更新 |
| src/storage/mod.rs | 存储模块导出 | ✅ 完成 |
| src/storage/page_format/key.rs | Key 结构（32 bytes） | ✅ M2 完成 |
| src/storage/btree/btree.rs | BTree 核心逻辑 | ✅ M2 完成 |
| src/storage/btree/index_manager.rs | IndexManager API | ✅ M2 完成 |
| src/executor/mod.rs | 执行模块导出 | ✅ M4 完成 |
| src/executor/value.rs | Value enum | ✅ M4 完成 |
| src/executor/plan.rs | PhysicalPlan + 5 nodes | ✅ M4 完成 |
| src/parser/mod.rs | 解析模块导出 | ✅ M4 完成 |
| src/parser/error.rs | PlanError | ✅ M4 完成 |
| src/parser/ast.rs | AST 辅助函数 | ✅ M4 完成 |
| src/parser/planner.rs | PlanBuilder | ✅ M4 完成 |
| src/transaction/manager.rs | TransactionManager | ✅ M3 完成 |
| src/network/mod.rs | 网络模块导出 | ✅ M6 完成 |
| src/network/protocol.rs | Protocol trait + JsonProtocol | ✅ M6 完成 |
| src/network/server.rs | Server + TcpListener | ✅ M6 完成 |
| src/network/connection.rs | ConnectionHandler | ✅ M6 完成 |
| src/network/handler.rs | SqlHandler (mock) | ✅ M6 完成 |
| tests/network_protocol_test.rs | JSON 协议测试 | ✅ M6 完成（5 测试）|
| tests/network_server_test.rs | Server 集成测试 | ✅ M6 完成（4 测试）|

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | src/network/*, tests/network_* | M6 Protocol trait/JsonProtocol/Server/ConnectionHandler/SqlHandler 实现 |
| 2026-05-20 | Cargo.toml | 添加 tokio-util/io-util/serde/serde_json 依赖 |
| 2026-05-20 | .claude/docs/superpowers/* | M6 设计规范和实现计划 |
| 2026-05-20 | src/executor/*, tests/executor_test.rs, tests/plan_exec_test.rs | M5 ExecResult/Executor trait/5 Executors 实现 |
| 2026-05-20 | src/parser/*, tests/parser_test.rs | M4 PlanError/Value/AST/PlanBuilder 实现 |
| 2026-05-20 | src/executor/*, tests/planner_test.rs | M4 PhysicalPlan + 5 nodes 实现 |
| 2026-05-20 | .claude/docs/superpowers/* | M4-M5 设计规范和实现计划 |
| 2026-05-20 | src/transaction/*, tests/concurrent_test.rs | M3 TransactionManager + MVCC 实现 |
| 2026-05-20 | src/storage/btree/*, tests/btree_* | M2 B-Tree 索引与存储引擎 |
| 2026-05-20 | src/storage/page_format/* | M2 Key/RowId/SlottedPage 实现 |

## 下一步行动

1. 开始 M6 里程碑：全流程集成 + 网络层
2. 实现数据存储层（TableManager、Row 数据存储）
3. 实现 TCP 服务器（tokio::net::TcpListener）
4. 整合事务到执行引擎
5. 端到端测试