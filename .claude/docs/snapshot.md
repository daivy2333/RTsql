# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: M9 第一阶段完成（DDL + WHERE 表达式求值器）
- **状态**: 正常
- **当前里程碑**: M9 第二阶段准备开始（ORDER BY + LIMIT/OFFSET）

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
│   ├── pipeline.rs         # M7 新增：SQL 执行管道 [M11 扩展：DDL + WHERE]
│   └── storage/
│       ├── mod.rs          # 存储模块导出
│       ├── error.rs        # StorageError (含 SlotNotFound/TableNotFound/DuplicateTable) [M9 扩展：TableAlreadyExists/ConstraintViolation]
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
│   │   ├── mod.rs          # 模块导出 [M9 扩展：predicate/filter/create_table/drop_table]
│   │   ├── value.rs        # Value（Int/String/Null）[M9 扩展：Float/Bool + 比较方法]
│   │   ├── plan.rs         # PhysicalPlan + 5 节点 [M9 扩展：CreateTable/DropTable/Filter + ColumnDef]
│   │   ├── result.rs       # ExecResult（Row/AffectedRows）
│   │   ├── executor_trait.rs
│   │   ├── scan.rs         # ScanExecutor（全表扫描）[M7 重写]
│   │   ├── index_scan.rs   # IndexScanExecutor（读 Tuple）[M7 重写]
│   │   ├── insert.rs       # InsertExecutor（写数据页）[M7 重写]
│   │   ├── update.rs       # UpdateExecutor（版本链）[M7 重写]
│   │   ├── delete.rs       # DeleteExecutor
│   │   ├── predicate.rs    # M9 新增：Predicate trait + Expression trait + ComparisonPredicate/LogicalPredicate
│   │   ├── filter.rs       # M9 新增：FilterExecutor（WHERE 过滤）
│   │   ├── create_table.rs # M9 新增：CreateTableExecutor
│   │   └── drop_table.rs   # M9 新增：DropTableExecutor
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
│   │   ├── error.rs        # PlanError [M9 扩展：EmptyColumnDefinition/MultiplePrimaryKey/ColumnNotFound]
│   │   ├── value.rs        # Value 转换函数 [M9 扩展：Float 解析]
│   │   ├── ast.rs          # AST 辅助函数（parse_sql/extract_select_body/extract_table_name）
│   │   └── planner.rs      # PlanBuilder [M9 扩展：build_create_table/build_drop_table/build_where]
│   └── network/
│       ├── mod.rs
│       ├── error.rs
│       ├── protocol.rs     # Protocol trait + JsonProtocol
│       ├── pg_messages.rs  # M8 新增：PostgreSQL 消息序列化 [M9 扩展：Float8/Bool OID]
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
│   ├── planner_test.rs       (20) [M9 扩展：DDL + WHERE 解析]
│   ├── executor_test.rs      (24) [M7 更新 + M9 新增：FilterExecutor]
│   ├── plan_exec_test.rs     (4)  [M7 更新]
│   ├── table_manager_test.rs (6)  [M7 新增]
│   ├── network_protocol_test.rs (5)
│   ├── network_server_test.rs (4)  [M7 更新]
│   ├── pg_messages_test.rs   (9)  [M8 新增]
│   ├── pg_protocol_test.rs   (9)  [M8 新增]
│   ├── pg_integration_test.rs (1) [M8 新增]
│   ├── predicate_test.rs     (12) [M9 新增]
│   ├── pipeline_test.rs      (9)  [M9 新增]
│   ├── value_test.rs         (19) [M9 新增]
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

**注**: M9 第一阶段完成（DDL + WHERE），232 个测试全部通过。新增 40+ 个测试（predicate:12 + planner:+5 + executor:+3 + pipeline:9 + value:19）。

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
- **最近提交**（M9 第一阶段）:
  - 587a314 feat(pipeline): integrate DDL + WHERE execution
  - 0d3d133 feat(planner): add WHERE expression parsing + Filter plan
  - 3719737 feat(predicate): implement Predicate trait + ComparisonPredicate/LogicalPredicate
  - 9409bfa feat(executor): implement CreateTableExecutor
  - 28e7f1b feat(planner): add CREATE TABLE/DROP TABLE parsing
  - e7d7502 fix(tuple): complete Float/Bool serialization/deserialization
  - ...（M9 共 15+ commits）
- **未提交更改**: 无

**M9 第一阶段总结**: DDL + WHERE 表达式求值器完成（232 tests），解决用户无法通过 SQL 创建表的阻塞，实现完整 WHERE 条件过滤。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| src/database.rs | Database 协调器（BufferPool+TableManager+TxManager） | ✅ M7 |
| src/pipeline.rs | SQL→parse→plan→execute→Response 管道 | ✅ M11 扩展 DDL + WHERE |
| src/storage/data/table_manager.rs | TableManager 表元数据注册 + drop_table | ✅ M9 扩展 |
| src/storage/page_format/tuple.rs | ColumnType + serialize/deserialize_tuple | ✅ M9 扩展 Float/Bool |
| src/executor/value.rs | Value（Int/String/Float/Bool/Null）+ 比较方法 | ✅ M9 扩展 |
| src/executor/plan.rs | PhysicalPlan（CreateTable/DropTable/Filter）+ ColumnDef | ✅ M9 扩展 |
| src/executor/predicate.rs | Predicate trait + Expression trait + ComparisonPredicate/LogicalPredicate | ✅ M9 新增 |
| src/executor/filter.rs | FilterExecutor（WHERE 过滤） | ✅ M9 新增 |
| src/executor/create_table.rs | CreateTableExecutor | ✅ M9 新增 |
| src/executor/drop_table.rs | DropTableExecutor | ✅ M9 新增 |
| src/parser/planner.rs | PlanBuilder（build_create_table/build_drop_table/build_where） | ✅ M9 扩展 |
| tests/predicate_test.rs | Predicate 单元测试（12 tests） | ✅ M9 新增 |
| tests/pipeline_test.rs | DDL + WHERE 集成测试（9 tests） | ✅ M9 新增 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | src/executor/predicate.rs | M9 新增 Predicate trait + Expression trait |
| 2026-05-20 | src/executor/filter.rs | M9 新增 FilterExecutor（WHERE 过滤） |
| 2026-05-20 | src/executor/create_table.rs | M9 新增 CreateTableExecutor |
| 2026-05-20 | src/executor/drop_table.rs | M9 新增 DropTableExecutor |
| 2026-05-20 | src/parser/planner.rs | M9 新增 DDL + WHERE 解析方法 |
| 2026-05-20 | src/executor/value.rs | M9 新增 Float/Bool + 比较方法 |
| 2026-05-20 | src/storage/page_format/tuple.rs | M9 新增 Float/Bool 序列化 |
| 2026-05-20 | src/pipeline.rs | M11 扩展 DDL + WHERE 流程集成 |
| 2026-05-20 | tests/predicate_test.rs | M9 新增 12 个测试 |
| 2026-05-20 | tests/pipeline_test.rs | M9 新增 9 个集成测试 |
| 2026-05-20 | tests/value_test.rs | M9 新增 19 个 Value 测试 |

## 下一步行动

**优先级**: M9 第二阶段（ORDER BY + LIMIT/OFFSET）- SQL 基础能力继续完善

**里程碑路线图**:
1. **M9 Phase 2**: ORDER BY 排序 + LIMIT/OFFSET 分页
2. **M10** (🟡 中优先级): 完整版本链遍历 + 版本链 GC
3. **M11** (🔴 高优先级): WAL 持久化 + 崩溃恢复
4. **M12** (🟢 低优先级): JOIN 多表支持
5. **M13**: 性能优化与完善

**当前阻塞**: 无（M9 Phase 1 解决了 DDL阻塞）

**注意事项**:
- M9 Phase 1 实现了完整的 DDL + WHERE，用户现在可以通过 SQL 创建表和执行复杂查询
- PostgreSQL 协议层（M8）可能分离或删除，嵌入式数据库不需要外部连接
- 重点完善嵌入式数据库核心功能（SQL + MVCC + WAL）