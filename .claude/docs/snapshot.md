# 项目快照

> 最后更新：2026-05-21

## 当前状态

- **阶段**: M10 完成（MVCC 完整性）
- **状态**: 正常
- **当前里程碑**: M11 准备开始（WAL 持久化）
- **测试**: 279 passed

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
│   ├── pipeline.rs         # M7 新增：SQL 执行管道 [M9 扩展：DDL + WHERE + ORDER BY + LIMIT]
│   └── storage/
│       ├── mod.rs          # 存储模块导出
│       ├── error.rs        # StorageError [M9 扩展：TableAlreadyExists/ConstraintViolation]
│       ├── page.rs         # Page 结构（4KB）
│       ├── page_id.rs      # PageId 结构
│       ├── async_storage.rs # AsyncStorage trait
│       ├── file_storage.rs  # FileStorage 实现
│       ├── buffer_pool.rs   # BufferPool（Clock 淘汰）
│       ├── page_frame.rs    # PageFrame + PageGuard
│       ├── data_page.rs    # M7 新增：write/read_tuple_to_data_page
│       ├── data/           # M7 新增：数据存储模块
│       │   ├── mod.rs
│       │   └── table_manager.rs # TableManager + TableMeta [M9 扩展：drop_table + ColumnSchema]
│       ├── page_format/
│       │   ├── mod.rs
│       │   ├── key.rs       # Key（32 bytes）
│       │   ├── row_id.rs    # RowId（page_id + slot_id）
│       │   ├── slotted_page.rs # SlottedPage 通用格式
│       │   └── tuple.rs    # M7 新增：ColumnType + tuple 序列化 [M9 扩展：Float/Bool]
│       └── btree/
│           ├── mod.rs
│           ├── node.rs      # LeafNode + InternalNode
│           ├── btree.rs     # BTree（含 scan_all）
│           ├── sync_loader.rs
│           └── index_manager.rs # IndexManager（含 scan_all）
│   ├── executor/
│   │   ├── mod.rs          # 模块导出 [M9 扩展：predicate/filter/create_table/drop_table/sort/limit]
│   │   ├── value.rs        # Value [M9 扩展：Float/Bool + 比较方法]
│   │   ├── plan.rs         # PhysicalPlan [M9 扩展：CreateTable/DropTable/Filter/Sort/Limit + ColumnDef/OrderByColumn]
│   │   ├── result.rs       # ExecResult（Row/AffectedRows）
│   │   ├── executor_trait.rs
│   │   ├── scan.rs         # ScanExecutor（全表扫描）[M7 重写]
│   │   ├── index_scan.rs   # IndexScanExecutor（读 Tuple）[M7 重写]
│   │   ├── insert.rs       # InsertExecutor（写数据页）[M7 重写]
│   │   ├── update.rs       # UpdateExecutor（版本链）[M7 重写]
│   │   ├── delete.rs       # DeleteExecutor
│   │   ├── predicate.rs    # M9 Phase 1 新增：Predicate trait + Expression trait
│   │   ├── filter.rs       # M9 Phase 1 新增：FilterExecutor（WHERE 过滤）
│   │   ├── sort.rs         # M9 Phase 2 新增：SortExecutor（ORDER BY 排序）
│   │   ├── limit.rs        # M9 Phase 2 新增：LimitExecutor（LIMIT/OFFSET 分页）
│   │   ├── create_table.rs # M9 Phase 1 新增：CreateTableExecutor
│   │   └── drop_table.rs   # M9 Phase 1 新增：DropTableExecutor
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
│   │   ├── error.rs        # PlanError [M9 扩展]
│   │   ├── value.rs        # Value 转换函数 [M9 扩展：Float 解析]
│   │   ├── ast.rs          # AST 辅助函数
│   │   └── planner.rs      # PlanBuilder [M9 扩展：DDL/WHERE/ORDER BY/LIMIT 解析]
│   └── network/
│       ├── mod.rs
│       ├── error.rs
│       ├── protocol.rs     # Protocol trait + JsonProtocol
│       ├── pg_messages.rs  # M8 新增：PostgreSQL 消息序列化
│       ├── pg_protocol.rs  # M8 新增：PgProtocol 状态机
│       ├── connection.rs   # ConnectionHandler（async handler）
│       ├── handler.rs      # SqlHandler（真实 pipeline）[M7 重写]
│       └── server.rs       # Server [M8 切换 PgProtocol]
├── tests/
│   ├── runtime_test.rs       (3)
│   ├── btree_test.rs         (10)
│   ├── index_manager_test.rs (3)
│   ├── sync_loader_test.rs   (2)
│   ├── concurrent_test.rs    (4)
│   ├── parser_test.rs        (6)
│   ├── planner_test.rs       (25) [M9 扩展：DDL + WHERE + ORDER BY + LIMIT 解析]
│   ├── executor_test.rs      (24) [M7 更新 + M9 扩展：FilterExecutor]
│   ├── plan_exec_test.rs     (4)  [M7 更新]
│   ├── table_manager_test.rs (6)  [M7 新增]
│   ├── network_protocol_test.rs (5)
│   ├── network_server_test.rs (4)  [M7 更新]
│   ├── pg_messages_test.rs   (9)  [M8 新增]
│   ├── pg_protocol_test.rs   (9)  [M8 新增]
│   ├── pg_integration_test.rs (1) [M8 新增]
│   ├── predicate_test.rs     (12) [M9 Phase 1 新增]
│   ├── sort_test.rs          (6)  [M9 Phase 2 新增]
│   ├── limit_test.rs         (5)  [M9 Phase 2 新增]
│   ├── pipeline_test.rs      (12) [M9 新增]
│   ├── value_test.rs         (19) [M9 Phase 1 新增]
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

**注**: M9 Phase 2 完成（ORDER BY + LIMIT/OFFSET），256 个测试全部通过。新增 24 个测试（sort:6 + limit:5 + planner:+5 + pipeline:+3 + sort_unit:5）。

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
- **最近提交**（M9 Phase 2）:
  - 45b67d2 feat(M9): complete ORDER BY + LIMIT/OFFSET implementation
  - 52cde7f fix(sort): correct column index mapping in SortExecutor
  - 5a87380 test(pipeline): add ORDER BY + LIMIT end-to-end tests
  - dac8e4d feat(pipeline): integrate SortExecutor + LimitExecutor
  - a7b0b6e test(parser): add ORDER BY + LIMIT/OFFSET parsing tests
  - 01301d0 feat(parser): add ORDER BY + LIMIT/OFFSET parsing
  - ...（M9 Phase 2 共 15 commits）
- **未提交更改**: 无

**M10 总结**: MVCC完整性完成（279 tests），实现完整版本链遍历、commit标记、abort清理、可选GC。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| src/transaction/manager.rs | TransactionManager（tx_versions跟踪 + commit标记 + abort清理） | ✅ M10 新增 |
| src/storage/buffer_pool.rs | BufferPool（find_visible_version版本链遍历） | ✅ M10 新增 |
| src/storage/btree/index_manager.rs | IndexManager（find_key_by_row_id反向映射） | ✅ M10 新增 |
| src/executor/index_scan.rs | IndexScanExecutor（版本链遍历集成） | ✅ M10 修改 |
| src/executor/scan.rs | ScanExecutor（版本链遍历集成） | ✅ M10 修改 |
| src/storage/data_page.rs | 数据页操作（update_version_header + delete_tuple） | ✅ M10 新增 |
| src/storage/data/table_manager.rs | TableManager（gc_table可选GC） | ✅ M10 新增 |
| tests/mvcc_record_test.rs | tx_versions记录测试（5 tests） | ✅ M10 新增 |
| tests/mvcc_commit_test.rs | commit标记测试（4 tests） | ✅ M10 新增 |
| tests/mvcc_abort_test.rs | abort清理测试（3 tests） | ✅ M10 新增 |
| tests/version_chain_test.rs | 版本链遍历测试（3 tests） | ✅ M10 新增 |
| tests/gc_test.rs | GC测试（3 tests） | ✅ M10 新增 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-21 | src/transaction/manager.rs | M10 新增 tx_versions, record_version, commit_mark_versions, abort_cleanup_versions |
| 2026-05-21 | src/storage/buffer_pool.rs | M10 新增 find_visible_version, read_version_header, write_commit_tx_id |
| 2026-05-21 | src/storage/btree/index_manager.rs | M10 新增 find_key_by_row_id, row_to_key反向映射 |
| 2026-05-21 | src/executor/index_scan.rs | M10 修改为使用 find_visible_version |
| 2026-05-21 | src/executor/scan.rs | M10 修改为使用 find_visible_version |
| 2026-05-21 | src/executor/insert.rs, update.rs | M10 新增 record_version 调用 |
| 2026-05-21 | src/storage/data_page.rs | M10 新增 update_version_header_in_data_page, delete_tuple_from_data_page |
| 2026-05-21 | src/storage/data/table_manager.rs | M10 新增 gc_table |
| 2026-05-21 | tests/mvcc_*.rs | M10 新增 15 个 MVCC 测试 |
| 2026-05-21 | tests/version_chain_test.rs | M10 新增 3 个版本链测试 |
| 2026-05-21 | tests/gc_test.rs | M10 新增 3 个 GC 测试 |

## 下一步行动

**优先级**: M11（WAL 持久化）- 崩溃恢复能力

**里程碑路线图**:
1. **M11** (🔴 高优先级): WAL 持久化 + 崩溃恢复 + Checkpoint
2. **M12** (🟢 低优先级): JOIN 多表支持
3. **M13**: 性能优化与完善

**当前阻塞**: 无

**注意事项**:
- M10 完整实现了 MVCC 版本链遍历、commit标记、abort清理
- 可选 GC 已实现（gc_table），用户可手动触发清理旧版本
- 下一步重点：WAL 持久化（嵌入式数据库崩溃恢复必需）