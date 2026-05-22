# 项目快照

> 最后更新：2026-05-22 (M13 完成)

## 当前状态

- **阶段**: M13 完成（性能基准测试与关键优化）
- **状态**: 正常
- **当前里程碑**: M14 准备开始（聚合函数与 GROUP BY）
- **测试**: 83 lib tests + 74 integration tests（全部通过）
- **代码量**: src 9190 行 + tests 8463 行 + benches 750 行

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置（criterion + rusqlite dev-deps）
├── CLAUDE.md               # 文档入口
├── src/
│   ├── main.rs             # 数据库服务器入口
│   ├── lib.rs              # 库入口
│   ├── database.rs         # Database 协调器（BufferPool+TableManager+TxManager+WalWriter）
│   ├── pipeline.rs         # SQL 执行管道（parse→plan→execute→Response）
│   ├── storage/
│   │   ├── mod.rs          # 存储模块导出（含 PageDataGuard, SlottedPageRef）
│   │   ├── error.rs        # StorageError
│   │   ├── page.rs         # Page（4KB Box<[u8]>）
│   │   ├── page_id.rs      # PageId(u64)
│   │   ├── page_frame.rs   # PageFrame + PageGuard + PageDataGuard（零拷贝）
│   │   ├── async_storage.rs # AsyncStorage trait
│   │   ├── file_storage.rs  # FileStorage 实现
│   │   ├── buffer_pool.rs   # BufferPool（Clock 淘汰 + 两阶段锁）
│   │   ├── data_page.rs    # write/read/update/delete tuple（零拷贝读取）
│   │   ├── data/           # TableManager + TableMeta + ColumnSchema
│   │   ├── page_format/
│   │   │   ├── key.rs       # Key（32 bytes）
│   │   │   ├── row_id.rs    # RowId（page_id + slot_id）
│   │   │   ├── slotted_page.rs # SlottedPage + SlottedPageRef（只读零拷贝）
│   │   │   └── tuple.rs    # ColumnType + serialize/deserialize_tuple
│   │   └── btree/
│   │       ├── btree.rs     # BTree 核心
│   │       ├── node.rs      # LeafNode + InternalNode
│   │       ├── sync_loader.rs # SyncPageLoader
│   │       └── index_manager.rs # IndexManager（含 scan_all）
│   ├── executor/
│   │   ├── mod.rs           # 模块导出
│   │   ├── value.rs         # Value（Int/String/Float/Bool/Null + Eq+Hash）
│   │   ├── plan.rs          # PhysicalPlan（13 种节点）
│   │   ├── result.rs        # ExecResult
│   │   ├── executor_trait.rs # Executor trait
│   │   ├── scan.rs          # ScanExecutor
│   │   ├── index_scan.rs    # IndexScanExecutor
│   │   ├── insert.rs        # InsertExecutor
│   │   ├── update.rs        # UpdateExecutor
│   │   ├── delete.rs        # DeleteExecutor
│   │   ├── predicate.rs     # Predicate + Expression trait
│   │   ├── filter.rs        # FilterExecutor（WHERE）
│   │   ├── sort.rs          # SortExecutor（ORDER BY）
│   │   ├── limit.rs         # LimitExecutor（LIMIT/OFFSET）
│   │   ├── join.rs          # JoinExecutor（INNER JOIN 哈希连接）
│   │   ├── create_table.rs  # CreateTableExecutor
│   │   └── drop_table.rs    # DropTableExecutor
│   ├── transaction/
│   │   ├── tx_id.rs         # AtomicU64 事务 ID
│   │   ├── error.rs         # TransactionError
│   │   ├── snapshot.rs      # Snapshot（Repeatable Read 可见性）
│   │   ├── version_chain.rs # VersionHeader（22B）
│   │   ├── row_lock.rs      # RowLockTable（异步行锁）
│   │   └── manager.rs       # TransactionManager
│   ├── parser/
│   │   ├── error.rs         # PlanError（含 JOIN 错误类型）
│   │   ├── value.rs         # Value 转换
│   │   ├── ast.rs           # AST 辅助（含 extract_join_table_name）
│   │   └── planner.rs       # PlanBuilder（含 build_from_clause/resolve_column_ref）
│   ├── network/
│   │   ├── protocol.rs      # Protocol trait + JsonProtocol
│   │   ├── pg_messages.rs   # PostgreSQL 消息序列化
│   │   ├── pg_protocol.rs   # PgProtocol 状态机
│   │   ├── connection.rs    # ConnectionHandler
│   │   ├── handler.rs       # SqlHandler（真实 pipeline）
│   │   └── server.rs        # Server + Graceful shutdown
│   └── wal/
│       ├── record.rs        # WalRecord enum + serialize/deserialize
│       ├── writer.rs        # WalWriter（async write + fsync + truncate）
│       ├── reader.rs        # WalReader（read_next + seek_to）
│       ├── checkpoint.rs    # CheckpointManager（checkpoint flow + 位点）
│       └── recovery.rs      # RecoveryManager（recover + needs_recovery）
├── benches/
│   ├── common/mod.rs        # 共享 helper（setup_db, insert_rows, create_join_tables）
│   ├── micro_bench.rs       # 9 种 SQL 操作微基准
│   ├── concurrent_bench.rs  # 并发压力测试（read/write/mixed/conflict）
│   ├── scale_bench.rs       # 规模扩展测试（1K/10K/100K）
│   └── sqlite_compare.rs    # SQLite 对比测试
├── tests/                   # 集成测试（74 tests）
└── .claude/docs/            # 项目文档
```

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 1.75+ |
| 异步运行时 | Tokio | 1.x（multi-thread） |
| SQL 解析 | sqlparser-rs | 0.44 |
| 序列化 | serde + serde_json | 1.0 |
| Shutdown | tokio-util (CancellationToken) | 0.7 |
| 基准测试 | criterion.rs | 0.5（html_reports + async_tokio） |
| SQLite 对比 | rusqlite | 0.31 |
| 临时文件 | tempfile | 3.x |

## Git 状态

- **当前分支**: master
- **最近提交**（M13 性能优化）:
  - b7b6865 docs(M13): update project docs
  - dd777fb perf(M13): PageGuard zero-copy + BufferPool two-phase lock
  - 0d4d0a7 feat(M13): implement all benchmark suites
  - 2b68835 feat(M13): add benchmark common helper module
  - ac32658 chore(M13): add criterion + rusqlite dev-dependencies

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| src/storage/page_frame.rs | PageGuard + PageDataGuard（零拷贝） | ✅ M13 优化 |
| src/storage/page_format/slotted_page.rs | SlottedPage + SlottedPageRef | ✅ M13 新增 SlottedPageRef |
| src/storage/buffer_pool.rs | BufferPool（两阶段锁） | ✅ M13 优化 |
| src/storage/data_page.rs | read_tuple_from_data_page（零拷贝） | ✅ M13 优化 |
| benches/micro_bench.rs | 9 种 SQL 微基准 | ✅ M13 新增 |
| benches/concurrent_bench.rs | 并发压力测试 | ✅ M13 新增 |
| benches/scale_bench.rs | 规模扩展测试 | ✅ M13 新增 |
| benches/sqlite_compare.rs | SQLite 对比测试 | ✅ M13 新增 |

## 下一步行动

**优先级**: M14（聚合函数与 GROUP BY）

**里程碑路线图**:
1. **M14**: 聚合函数（COUNT/SUM/AVG/MIN/MAX）+ GROUP BY
2. **M15**: 子查询支持
3. **M16**: 索引优化（B-Tree split/merge）

**当前阻塞**: 无