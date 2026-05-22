# 项目快照

> 最后更新：2026-05-22（M14 部分完成 + 性能瓶颈重新定位）

## 当前状态

- **阶段**: M14 部分完成（零拷贝+缓存已实现，性能目标未达成，需继续优化）
- **状态**: 正常
- **当前里程碑**: M14 继续（精确性能测试 + 消除 spawn_blocking 调度瓶颈）
- **测试**: 全量通过（lib + integration + 新增缓存测试）
- **代码量**: src ~9500 行 + tests ~8700 行 + benches ~1000 行

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置（criterion + rusqlite + lru dev-deps）
├── CLAUDE.md               # 文档入口
├── src/
│   ├── main.rs             # 数据库服务器入口
│   ├── lib.rs              # 库入口
│   ├── database.rs         # Database 协调器（BufferPool+TableManager+TxManager+WalWriter+plan_cache）
│   ├── pipeline.rs         # SQL 执行管道（parse→plan→execute + plan cache）
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
│   │       ├── btree.rs     # BTree 核心（读路径零拷贝）
│   │       ├── node.rs      # LeafNode + LeafNodeRef + InternalNode + InternalNodeRef
│   │       ├── sync_loader.rs # SyncPageLoader
│   │       └── index_manager.rs # IndexManager（含 scan_all）
│   ├── executor/
│   │   ├── mod.rs           # 模块导出
│   │   ├── value.rs         # Value（Int/String/Float/Bool/Null + Eq+Hash）
│   │   ├── plan.rs          # PhysicalPlan（13 种节点，derive Clone）
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
│   ├── concurrent_bench.rs  # 并发压力测试
│   ├── scale_bench.rs       # 规模扩展测试
│   ├── sqlite_compare.rs    # SQLite 对比测试
│   └── cache_bench.rs       # 缓存命中/未命中对比基准
├── tests/                   # 集成测试 + cache_perf_test
└── .claude/docs/            # 项目文档
```

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 1.75+ |
| 异步运行时 | Tokio | 1.x（multi-thread） |
| SQL 解析 | sqlparser-rs | 0.44 |
| Plan 缓存 | lru | 0.12 |
| 序列化 | serde + serde_json | 1.0 |
| Shutdown | tokio-util (CancellationToken) | 0.7 |
| 基准测试 | criterion.rs | 0.5（html_reports + async_tokio） |
| SQLite 对比 | rusqlite | 0.31 |
| 临时文件 | tempfile | 3.x |

## Git 状态

- **当前分支**: master
- **最近提交**（M14）:
  - 9358c0c docs(M14): update project docs with performance analysis findings
  - 4a97330 feat(M14): integrate plan cache into Pipeline execute_sql
  - d0d3312 feat(M14): add plan_cache LRU field to Database
  - e403369 perf(M14): migrate BTree read path to zero-copy
  - 458981e feat(M14): add LeafNodeRef and InternalNodeRef
  - f663389 chore(M14): add lru crate dependency

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| src/database.rs | Database + plan_cache | ✅ M14 新增 |
| src/pipeline.rs | Pipeline + cache hit/miss | ✅ M14 新增 |
| src/storage/btree/node.rs | LeafNodeRef + InternalNodeRef | ✅ M14 新增 |
| src/storage/btree/btree.rs | BTree 零拷贝读路径 | ✅ M14 修改 |
| src/executor/plan.rs | PhysicalPlan Clone | ✅ M14 验证 |

## 下一步行动

**优先级**: M14 继续 — 精确性能测试 + 消除 spawn_blocking 瓶颈

**性能瓶颈定位**：
1. spawn_blocking + SyncPageLoader::block_on 调度链（~25µs）
2. Mutex<BTree> 全局锁（~5µs）
3. parse+plan 已不是瓶颈（仅 ~5µs）

**M14 剩余任务**：
1. 精确性能参数测试（分阶段计时，量化各环节开销）
2. 消除 spawn_blocking 调度瓶颈（async BTree 或专用线程池）