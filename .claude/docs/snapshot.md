# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: M8 完成（PostgreSQL Simple Query Protocol）
- **状态**: 正常
- **当前里程碑**: M9 准备开始（高级功能与优化）

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置
├── Cargo.lock              # 依赖锁定文件
├── .gitignore              # Git 忽略配置
├── CLAUDE.md               # 文档入口
├── src/
│   ├── main.rs             # 数据库服务器入口
│   ├── lib.rs              # 库入口，导出模块公共接口
│   ├── database.rs         # M7 新增：Database 协调器结构
│   ├── pipeline.rs         # M7 新增：SQL 执行管道
│   └── storage/
│       ├── mod.rs          # 存储模块导出
│       ├── error.rs        # StorageError (含 SlotNotFound/TableNotFound/DuplicateTable)
│       ├── page.rs         # Page 结构（4KB）
│       ├── page_id.rs      # PageId 结构
│       ├── async_storage.rs # AsyncStorage trait
│       ├── file_storage.rs  # FileStorage 实现
│       ├── buffer_pool.rs   # BufferPool（Clock 淘汰）
│       ├── page_frame.rs    # PageFrame + PageGuard
│       ├── data_page.rs    # M7 新增：write/read_tuple_to_data_page
│       ├── data/           # M7 新增：数据存储模块
│       │   ├── mod.rs
│       │   └── table_manager.rs # TableManager + TableMeta
│       ├── page_format/
│       │   ├── mod.rs
│       │   ├── key.rs       # Key（32 bytes）
│       │   ├── row_id.rs    # RowId（page_id + slot_id）
│       │   ├── slotted_page.rs # SlottedPage 通用格式
│       │   └── tuple.rs    # M7 新增：ColumnType + tuple 序列化
│       └── btree/
│           ├── mod.rs
│           ├── node.rs      # LeafNode + InternalNode
│           ├── btree.rs     # BTree（含 scan_all）
│           ├── sync_loader.rs
│           └── index_manager.rs # IndexManager（含 scan_all）
│   ├── executor/
│   │   ├── mod.rs
│   │   ├── value.rs        # Value（Int/String/Null）
│   │   ├── plan.rs         # PhysicalPlan + 5 节点
│   │   ├── result.rs       # ExecResult（Row/AffectedRows）
│   │   ├── executor_trait.rs
│   │   ├── scan.rs         # ScanExecutor（全表扫描）[M7 重写]
│   │   ├── index_scan.rs   # IndexScanExecutor（读 Tuple）[M7 重写]
│   │   ├── insert.rs       # InsertExecutor（写数据页）[M7 重写]
│   │   ├── update.rs       # UpdateExecutor（版本链）[M7 重写]
│   │   └── delete.rs       # DeleteExecutor
│   ├── transaction/
│   │   ├── mod.rs
│   │   ├── tx_id.rs
│   │   ├── error.rs
│   │   ├── snapshot.rs     # Snapshot（MVCC 可见性）
│   │   ├── version_chain.rs # VersionHeader（22B）
│   │   ├── row_lock.rs
│   │   └── manager.rs      # TransactionManager
│   ├── parser/
│   │   ├── mod.rs
│   │   ├── error.rs
│   │   ├── value.rs
│   │   ├── ast.rs
│   │   └── planner.rs      # PlanBuilder
│   └── network/
│       ├── mod.rs
│       ├── error.rs
│       ├── protocol.rs     # Protocol trait + JsonProtocol
│       ├── pg_messages.rs  # M8 新增：PostgreSQL 消息序列化
│       ├── pg_protocol.rs  # M8 新增：PgProtocol 状态机
│       ├── connection.rs   # ConnectionHandler（async handler）
│       ├── handler.rs      # SqlHandler（真实 pipeline）[M7 重写]
│       └── server.rs       # Server（接受 Arc<Database>）[M8 切换 PgProtocol]
├── tests/
│   ├── runtime_test.rs       (3)
│   ├── btree_test.rs         (10)
│   ├── index_manager_test.rs (3)
│   ├── sync_loader_test.rs   (2)
│   ├── concurrent_test.rs    (4)
│   ├── parser_test.rs        (6)
│   ├── planner_test.rs       (8)
│   ├── executor_test.rs      (16) [M7 更新]
│   ├── plan_exec_test.rs     (4)  [M7 更新]
│   ├── table_manager_test.rs (6)  [M7 新增]
│   ├── network_protocol_test.rs (5)
│   ├── network_server_test.rs (4)  [M7 更新]
│   ├── pg_messages_test.rs   (9)  [M8 新增]
│   ├── pg_protocol_test.rs   (9)  [M8 新增]
│   ├── pg_integration_test.rs (1) [M8 新增]
│   └── e2e_test.rs          (7)  [M7 新增]
└── .claude/
    └── docs/
        ├── architecture.md
        ├── learned.md
        ├── optimization.md
        ├── references.md
        ├── rules.md
        ├── snapshot.md
        └── tasks.md
```

**注**: M8 PostgreSQL Simple Query Protocol 完成，159 个测试全部通过。新增 19 个测试（pg_messages:9 + pg_protocol:9 + pg_integration:1）。

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 1.75+ |
| 构建工具 | Cargo | 内置 |
| 异步运行时 | Tokio | 1.x（multi-thread + io-util） |
| SQL 解析 | sqlparser-rs | 0.44 |
| 协议层 | serde + serde_json | 1.0 |
| Shutdown | tokio-util (CancellationToken) | 0.7 |
| 测试框架 | tempfile + 内置 | 3.x |
| 格式化 | rustfmt | 内置 |
| Lint | clippy | 内置 |

## Git 状态

- **当前分支**: master
- **最近提交**（M8 + 优化计划）:
  - a3f81b6 docs: 嵌入式异步高性能优化计划（基于最佳实践）
  - e900678 docs: 重新规划里程碑（M9-M13）
  - 7f63e53 docs: mark M8 PostgreSQL protocol complete, update snapshot
  - 2a25b7f test(pg_integration): add startup connection test
  - 1e4e3fc feat(server): switch to PgProtocol (PostgreSQL protocol)
  - ...（M8 共 15 commits：PgProtocol + pg_messages + 测试）
- **未提交更改**: tests/pg_integration_test.rs（SQL 执行测试，DDL 不支持）

**M8 总结**: PostgreSQL Simple Query Protocol 完成（159 tests），发现 Critical Issues（异步页缓存 + 零拷贝）

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| src/database.rs | Database 协调器（BufferPool+TableManager+TxManager） | ✅ M7 |
| src/pipeline.rs | SQL→parse→plan→execute→Response 管道 | ✅ M7 |
| src/storage/data/table_manager.rs | TableManager 表元数据注册 | ✅ M7 |
| src/storage/data_page.rs | write_tuple_to_data_page / read_tuple_from_data_page | ✅ M7 |
| src/storage/page_format/tuple.rs | ColumnType + serialize/deserialize_tuple | ✅ M7 |
| src/executor/insert.rs | InsertExecutor（真实数据页写入） | ✅ M7 重写 |
| src/executor/index_scan.rs | IndexScanExecutor（读 Tuple + MVCC 可见性） | ✅ M7 重写 |
| src/executor/scan.rs | ScanExecutor（全表 BTree 扫描） | ✅ M7 重写 |
| src/executor/update.rs | UpdateExecutor（版本链创建） | ✅ M7 重写 |
| src/executor/delete.rs | DeleteExecutor（含 tx_id） | ✅ M7 更新 |
| src/network/handler.rs | SqlHandler（真实 pipeline，async） | ✅ M7 重写 |
| src/network/server.rs | Server（接受 Arc<Database>） | ✅ M8 更新 PgProtocol |
| src/network/pg_messages.rs | PostgreSQL 消息序列化（Startup/Query/Error） | ✅ M8 新增 |
| src/network/pg_protocol.rs | PgProtocol 状态机（Startup/Ready/Query） | ✅ M8 新增 |
| src/storage/btree/btree.rs | BTree（新增 scan_all） | ✅ M7 更新 |
| tests/e2e_test.rs | 7 个端到端 TCP 测试 | ✅ M7 新增 |
| tests/pg_messages_test.rs | 9 个 PostgreSQL 消息序列化测试 | ✅ M8 新增 |
| tests/pg_protocol_test.rs | 9 个 PostgreSQL 协议状态机测试 | ✅ M8 新增 |
| tests/pg_integration_test.rs | 1 个 PostgreSQL 集成测试 | ✅ M8 新增 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | .claude/docs/optimization.md | 重新规划优化（Critical Issues + 嵌入式最佳实践） |
| 2026-05-20 | .claude/docs/tasks.md | 重新规划 M9-M13（DDL/WHERE/MVCC/WAL 优先级） |
| 2026-05-20 | .claude/docs/architecture.md | 开发路线图重新规划（优先级调整） |
| 2026-05-20 | .claude/docs/snapshot.md | Git 状态更新（M8 完成 + Critical Issues） |
| 2026-05-20 | src/network/pg_messages.rs | M8 PostgreSQL 消息序列化 |
| 2026-05-20 | src/network/pg_protocol.rs | M8 PgProtocol 状态机 |
| 2026-05-20 | src/network/server.rs | M8 切换到 PgProtocol |
| 2026-05-20 | tests/pg_messages_test.rs | M8 消息序列化测试（9 tests） |
| 2026-05-20 | tests/pg_protocol_test.rs | M8 协议状态机测试（9 tests） |
| 2026-05-20 | tests/pg_integration_test.rs | M8 集成测试（1 test） |
| 2026-05-20 | src/executor/* | M7 5 Executor 重写（MVCC + 真实存储） |
| 2026-05-20 | src/network/handler.rs, server.rs | M7 SqlHandler 真实 pipeline |
| 2026-05-20 | src/storage/btree/* | M7 BTree::scan_all + IndexManager::scan_all |
| 2026-05-20 | tests/e2e_test.rs | M7 端到端 TCP 测试（7 tests） |
| 2026-05-20 | tests/executor_test.rs | M7 更新所有测试（16 tests） |
| 2026-05-20 | tests/table_manager_test.rs | M7 TableManager 测试（6 tests） |

## 下一步行动

**优先级**: SQL 基础能力完善（M9）- 让用户能通过 SQL 正常使用数据库

**里程碑路线图**:
1. **M9** (🔴 高优先级): DDL + WHERE 表达式 + ORDER BY/LIMIT
2. **M10** (🟡 中优先级): 完整版本链遍历 + 版本链 GC
3. **M11** (🔴 高优先级): WAL 持久化 + 崩溃恢复
4. **M12** (🟢 低优先级): JOIN 多表支持
5. **M13**: 性能优化与完善

**当前阻塞**: 用户无法通过 SQL 创建表（必须用 TableManager API）

**注意事项**:
- PostgreSQL 协议层（M8）可能分离或删除，嵌入式数据库不需要外部连接
- 重点完善嵌入式数据库核心功能（SQL + MVCC + WAL）
