# 项目快照

> 最后更新：2026-05-22（M14 Phase 2 T2 完成，性能优化验证）

## 当前状态

- **阶段**: M14 Phase 2 T2 完成
- **状态**: 正常
- **当前里程碑**: 准备进入 M15（聚合函数与 GROUP BY）
- **测试**: 88 lib tests + 74 integration tests（全部通过）
- **代码量**: src 9200 行 + tests 8500 行 + benches 900 行 + profiling 68 行

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置（6 benchmarks）
├── CLAUDE.md               # 文档入口
├── examples/
│   └── bench_minimal.rs    # Profiling benchmark（50 iterations）
├── src/
│   ├── main.rs             # 数据库服务器入口
│   ├── lib.rs              # 库入口
│   ├── database.rs         # Database 协调器 + plan_cache_len()
│   ├── pipeline.rs         # SQL 执行管道（含 profiling timing）
│   ├── profiling.rs        # Task-local profiling（68 行）
│   ├── plan_cache.rs       # LRU plan cache（68 行）
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
│   │   │   ├── slotted_page.rs # SlottedPage + SlottedPageRef（含 slot compacting）
│   │   │   └── tuple.rs    # ColumnType + serialize/deserialize_tuple
│   │   └── btree/
│   │       ├── btree.rs     # BTree 核心（含 search_async + from_root）
│   │       ├── node.rs      # LeafNode + InternalNode + LeafNodeRef
│   │       ├── sync_loader.rs # SyncPageLoader
│   │       ├── async_loader.rs # AsyncPageLoader（直接 async）
│   │       └── index_manager.rs # IndexManager（AtomicPageId + async search）
│   ├── executor/
│   │   ├── mod.rs           # 模块导出
│   │   ├── value.rs         # Value（Int/String/Float/Bool/Null + Eq+Hash）
│   │   ├── plan.rs          # PhysicalPlan（13 种节点）
│   │   ├── result.rs        # ExecResult
│   │   ├── executor_trait.rs # Executor trait
│   │   ├── scan.rs          # ScanExecutor
│   │   ├── index_scan.rs    # IndexScanExecutor（含 profiling timing）
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
│   ├── common/mod.rs        # 共享 helper（setup_db, insert_rows）
│   ├── micro_bench.rs       # 9 种 SQL 操作（50 iterations）
│   ├── concurrent_bench.rs  # 并发压力测试（5 concurrency levels）
│   ├── scale_bench.rs       # 规模扩展测试（1K/10K/100K）
│   ├── sqlite_compare.rs    # SQLite 对比测试
│   ├── rtsql_vs_sqlite_single.rs # 精确单次查询对比
│   └── cache_bench.rs       # Cache 效果测试
├── tests/                   # 集成测试（74 tests）
└── .claude/docs/            # 项目文档
```

## Git 状态

- **当前分支**: master
- **最近提交**（M14 Phase 2 T2）:
  - 10b0bce docs(M14-T2): verify and document 8x speedup credibility
  - aa791db docs(M14-T2): record comprehensive benchmark data
  - ca76e16 perf(M14-T2): adjust benchmark iterations to 50
  - b0d1685 refactor(M14-T2): remove separate comparison benchmark
  - 4aca9eb perf(M14-T2): add RTsql vs SQLite comparison
  - 71689ce fix(M14-T2): implement slot compacting
  - 1959b15 refactor(M14-T2): keep write operations sync
  - 2597835 feat(M14-T2): add BTree::from_root helper
  - 5c3fed9 feat(M14-T2): implement async scan_all
  - 3639e9d feat(M14-T2): implement async search
  - 637ff9e refactor(M14-T2): replace RwLock<BTree> with AtomicPageId

## 关键文件

| 文件 | 作用 | M14 T2 改动 |
|------|------|------------|
| src/storage/btree/index_manager.rs | IndexManager（AtomicPageId + async） | ✅ 重构架构 |
| src/storage/btree/btree.rs | BTree（search_async + from_root） | ✅ 新增方法 |
| src/storage/btree/async_loader.rs | AsyncPageLoader | ✅ 新增文件 |
| src/storage/page_format/slotted_page.rs | SlottedPage（slot compacting） | ✅ 修复 bug |
| benches/rtsql_vs_sqlite_single.rs | 精确对比测试 | ✅ 新增文件 |
| examples/bench_minimal.rs | Profiling benchmark | ✅ 调整 iterations |

## 下一步行动

**优先级**: M15 聚合函数与 GROUP BY

**里程碑路线图**:
1. **M15**: 聚合函数与 GROUP BY（功能完善）
2. **M16**: 子查询支持
3. **M17**: 索引优化（B-Tree split/merge + 非唯一索引）
4. **M18**: WAL 集成 + 写入优化（INSERT 5-10x 提速）

**当前阻塞**: 无

---

## M14 Phase 2 T2 验证结果

**性能优化成功**（17x internal speedup）：
- index_manager_search: 51µs → 2-4µs
- spawn_blocking + SyncPageLoader: 消除
- RwLock<BTree> 锁争用: 消除

**SQLite 对比验证**：
- RTsql PK lookup: ~0.66µs
- SQLite PK lookup: ~5.25µs
- **提速**: 8x faster

**Benchmark 参数**：
- iterations: 50（所有测试）
- concurrency: [1, 4, 8, 16, 32]
- scale: [1K, 10K, 100K]

运行命令：
```bash
cargo bench --bench rtsql_vs_sqlite_single  # 精确对比
cargo bench --bench sqlite_compare           # SQLite 测试
cargo bench --bench micro_bench              # RTsql 微基准
RTSQL_PROFILING=1 cargo run --example bench_minimal  # Profiling
```

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

**优先级**: M14 Phase 2 T2（IndexManager.search 优化）

**里程碑路线图**:
1. **M14 Phase 2 T2**: IndexManager.search 优化（目标：PK 查询 3-5x 提速）
   - 瓶颈已定位：index_manager_search 占 79-81% 总时间
   - 优化方向：BTree 零拷贝 + async search 优化
2. **M15**: 聚合函数与 GROUP BY
3. **M16**: 子查询支持
4. **M17**: 索引优化（B-Tree split/merge + 非唯一索引）
5. **M18**: WAL 集成 + 写入优化（INSERT 5-10x 提速）

**当前阻塞**: 无

---

## M14 Phase 2 T1 验证结果

**Profiling 输出**（RTSQL_PROFILING=1，warm run cache hit）：

```
Stage                    | Time (µs) | % Total
-------------------------|-----------|--------
executor_execution       |      57.0 |   90.5%
index_manager_search     |      51.0 |   81.0%
executor_creation        |       2.0 |    3.2%
cache_hit_check          |       0.0 |    0.0%
parse_and_plan           |       0.0 |    0.0%
-------------------------|-----------|--------
Total                    |      63.0 |  100.0%
```

**瓶颈定位**：
- **IndexManager.search**: 51µs (81%) ← **主要瓶颈**
- Executor execution: 57µs (90.5%, 包含 IndexManager.search)
- Executor creation: 2µs (3.2%)
- Cache hit check + parse/plan: ~0µs（cache hit 场景跳过）

**结论**：
- spawn_blocking + SyncPageLoader 调度开销是主要瓶颈（~81%）
- Plan cache 工作正常（parse/plan = 0µs on cache hit）
- 下一步优化方向：消除 spawn_blocking 或启用 async search 路径

**Git 状态**：
- Merge 成功：19 commits ahead of origin/master
- Binary search bug 修复（主分支原有 bug）
- Tests：88 passed, 0 failed

运行 `cargo run --example bench_minimal`（RTSQL_PROFILING=1）：

### 缓存命中场景（warm up 后）

典型输出：
```
Stage                    | Time (µs) | % Total
-------------------------|-----------|--------
executor_execution      |      57.0 |   90.5%
index_manager_search    |      51.0 |   81.0%
executor_creation       |       2.0 |    3.2%
cache_hit_check         |       0.0 |    0.0%
parse_and_plan          |       0.0 |    0.0%
-------------------------|-----------|--------
Total                   |      63.0 |  100.0%
```

### 性能瓶颈定位

- **IndexManager.search**: ~51µs (81% of total) — **主要瓶颈**
- **Executor execution**: ~57µs (90.5% of total) — 包含 IndexManager.search
- **Executor creation**: ~2-3µs (3.2-4.7%)
- **Cache hit check**: ~0µs (0%)
- **Parse and plan**: ~0µs (0%) — 缓存命中时跳过

**PK lookup 平均耗时**: ~129-138µs（包含缓存检查）

### 关键发现

1. **IndexManager.search 是主要瓶颈**，占总执行时间的 79-81%
2. **Plan cache 有效性已验证**，缓存命中时 parse_and_plan 时间为 0µs
3. **Executor creation overhead 很小**（~2-3µs），不是优化重点
4. **Profiling 框架运行正常**，task-local storage + scope 方案可行

### 下一步优化方向

根据 profiling 结果，M14 Phase 2 T2 应重点优化 **IndexManager.search**：
- BTree 零拷贝（使用 `page_data()` + `SlottedPageRef`）
- Async search 优化（减少 await overhead）
- LeafNode/InternalNode 直接访问优化

### 文件变更

**新增文件**:
- `src/profiling.rs` (68 行) — task-local storage + timing API
- `src/plan_cache.rs` (68 行) — simple plan cache implementation
- `examples/bench_minimal.rs` (34 行) — profiling-enabled benchmark

**修改文件**:
- `src/pipeline.rs` — 添加 profiling timing points + plan cache integration
- `src/executor/index_scan.rs` — 添加 `index_manager_search` timing
- `src/database.rs` — 添加 `plan_cache` field
- `src/lib.rs` — 导出 profiling + plan_cache modules
- `tests/executor_test.rs` — 添加 plan_cache field 到测试 Database 构造