# 项目快照

> 最后更新：2026-05-22

## 当前状态

- **阶段**: M12 完成（INNER JOIN 多表查询）
- **状态**: 正常
- **当前里程碑**: M13 准备开始（性能优化与完善）
- **测试**: 319 passed（全部测试）

## 项目结构

新增 JOIN 模块（M12）：

```
src/executor/
├── join.rs            # JoinExecutor（哈希连接实现）
└── value.rs           # Value Eq + Hash trait（HashMap 键）

新增 JOIN 解析（M12）：
src/parser/
├── planner.rs         # build_from_clause + resolve_column_ref + extract_join_conditions
└── ast.rs             # extract_join_table_name 辅助函数

新增 JOIN 测试（M12）：
tests/
├── join_test.rs       # JoinExecutor 单元测试（7 tests）
└── pipeline_test.rs   # JOIN 集成测试（+2 tests）
```

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
│   │   ├── mod.rs          # 模块导出 [M9 扩展：predicate/filter/create_table/drop_table/sort/limit + M12: join]
│   │   ├── value.rs        # Value [M9 扩展：Float/Bool + 比较方法 + M12: Eq+Hash]
│   │   ├── plan.rs         # PhysicalPlan [M9 扩展：CreateTable/DropTable/Filter/Sort/Limit + M12: Join/JoinCondition/ColumnRef/OutputColumn]
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
│   │   ├── join.rs         # M12 新增：JoinExecutor（INNER JOIN 哈希连接）
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
│   │   ├── error.rs        # PlanError [M9 扩展 + M12: AmbiguousColumn/TableNotFound/MissingOnClause/UnsupportedJoinType]
│   │   ├── value.rs        # Value 转换函数 [M9 扩展：Float 解析]
│   │   ├── ast.rs          # AST 辅助函数 [M12 扩展：extract_join_table_name]
│   │   └── planner.rs      # PlanBuilder [M9 扩展：DDL/WHERE/ORDER BY/LIMIT 解析 + M12: build_from_clause/resolve_column_ref]
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
│   ├── planner_test.rs       (29) [M9 扩展：DDL + WHERE + ORDER BY + LIMIT + M12: JOIN 解析]
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
│   ├── pipeline_test.rs      (14) [M9 新增 + M12: JOIN 集成测试]
│   ├── value_test.rs         (19) [M9 Phase 1 新增]
│   ├── join_test.rs          (7)  [M12 新增：JoinExecutor 单元测试]
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

**注**: M12 完成（INNER JOIN 哈希连接），319 个测试全部通过。新增 9 个测试（join:7 + pipeline:+2）。

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
- **最近提交**（M12 INNER JOIN）:
  - 2242955 style: apply rustfmt to various files
  - 1a5fa20 test(M12): add JOIN pipeline integration tests
  - 7a8786a feat(M12): integrate JoinExecutor into pipeline
  - 097d0e8 test(M12): add JoinExecutor unit tests
  - d89c3dc fix(M12): handle NULL in join keys per SQL semantics
- **未提交更改**: 无

**M12 总结**: INNER JOIN 哈希连接实现完成（JoinNode + JoinExecutor + ON 子句解析 + NULL 处理 + 多表链式 JOIN），9 相关测试通过。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| src/executor/join.rs | JoinExecutor 哈希连接实现（NULL 处理） | ✅ M12 新增 |
| src/executor/plan.rs | JoinNode + JoinCondition + ColumnRef + OutputColumn | ✅ M12 扩展 |
| src/executor/value.rs | Value Eq + Hash trait（HashMap 键支持） | ✅ M12 扩展 |
| src/parser/planner.rs | build_from_clause + resolve_column_ref + extract_join_conditions | ✅ M12 扩展 |
| src/parser/error.rs | AmbiguousColumn/TableNotFound/MissingOnClause/UnsupportedJoinType | ✅ M12 扩展 |
| src/parser/ast.rs | extract_join_table_name 辅助函数 | ✅ M12 扩展 |
| src/pipeline.rs | JoinExecutor 创建 + extract_column_indices | ✅ M12 扩展 |
| tests/join_test.rs | JoinExecutor 单元测试（7 tests） | ✅ M12 新增 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-22 | src/executor/join.rs | M12 新增 JoinExecutor（哈希连接 + NULL 处理） |
| 2026-05-22 | src/executor/plan.rs | M12 新增 JoinNode/JoinCondition/ColumnRef/OutputColumn |
| 2026-05-22 | src/executor/value.rs | M12 添加 Value Eq + Hash trait |
| 2026-05-22 | src/parser/planner.rs | M12 新增 build_from_clause + ON 条件解析 |
| 2026-05-22 | src/parser/error.rs | M12 新增 4 个 JOIN 相关错误类型 |
| 2026-05-22 | src/pipeline.rs | M12 JoinExecutor 创建逻辑 |
| 2026-05-22 | tests/join_test.rs | M12 新增 7 个 JoinExecutor 测试 |

## 下一步行动

**优先级**: M13（性能优化与完善）

**里程碑路线图**:
1. **M13**: 性能优化与完善

**当前阻塞**: 无

**注意事项**:
- M12 INNER JOIN 基础实现完成（仅 INNER JOIN，ON 子句 + AND 组合，哈希连接）
- LEFT/RIGHT/FULL OUTER JOIN 推迟
- NULL 处理符合 SQL 语义（NULL != NULL）
- 多表链式 JOIN 支持（Join(Join(A,B),C)）