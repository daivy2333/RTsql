# 项目快照

> 最后更新：2026-05-22（M15 聚合函数与 GROUP BY 完成）

## 当前状态

- **阶段**: M15 完成
- **状态**: 正常
- **当前里程碑**: 准备进入 M16（子查询支持）
- **测试**: 149 tests（全部通过）

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置（6 benchmarks）
├── CLAUDE.md               # 文档入口
├── examples/bench_minimal.rs
├── src/
│   ├── main.rs             # 数据库服务器入口
│   ├── lib.rs              # 库入口
│   ├── database.rs         # Database 协调器 + plan_cache
│   ├── pipeline.rs         # SQL 执行管道（含 profiling）
│   ├── profiling.rs        # Task-local profiling
│   ├── plan_cache.rs       # LRU plan cache
│   ├── storage/
│   │   ├── buffer_pool.rs  # Clock 淘汰 + 两阶段锁
│   │   ├── page_frame.rs   # PageGuard + PageDataGuard（零拷贝）
│   │   ├── data_page.rs    # 零拷贝页读取
│   │   ├── data/           # TableManager + TableMeta + ColumnSchema
│   │   ├── page_format/    # SlottedPage + SlottedPageRef（含 compacting）
│   │   └── btree/          # BTree + AsyncPageLoader + AtomicPageId
│   ├── executor/
│   │   ├── value.rs        # Value + arithmetic（add/lt_agg/div）
│   │   ├── plan.rs         # PhysicalPlan（15 种节点）
│   │   ├── executor_trait.rs
│   │   ├── scan.rs / index_scan.rs / insert.rs / update.rs / delete.rs
│   │   ├── predicate.rs / filter.rs / sort.rs / limit.rs
│   │   ├── join.rs         # 哈希连接
│   │   ├── aggregate.rs    # AggregateFunc + AggregateState + AggregateExecutor
│   │   ├── having.rs       # HavingExecutor
│   │   └── create_table.rs / drop_table.rs
│   ├── transaction/        # MVCC（VersionChain + RowLock + Snapshot）
│   ├── parser/             # PlanBuilder + AST + 聚合解析
│   ├── network/            # PgProtocol + ConnectionHandler + Server
│   └── wal/                # WalWriter + Recovery + Checkpoint
├── benches/                # 6 套基准测试
├── tests/                  # 集成测试（含 aggregate_test.rs 19 tests）
└── .claude/docs/           # 项目文档
```

## Git 状态

- **当前分支**: master
- **最近提交**（M15）:
  - c9ad00e feat(M15): integrate Aggregate/Having into pipeline
  - 33cf82d feat(M15): aggregate parsing in PlanBuilder
  - 5baf4ae feat(M15): HavingExecutor
  - 27d93d5 feat(M15): AggregateExecutor implementation
  - 5b115f6 feat(M15): AggregateFunc + AggregateState types

## 关键文件（M15 新增/修改）

| 文件 | 作用 | M15 改动 |
|------|------|----------|
| src/executor/aggregate.rs | 聚合函数 + 累积器 + 执行器 | ✅ 新增 |
| src/executor/having.rs | HAVING 过滤执行器 | ✅ 新增 |
| src/executor/plan.rs | PhysicalPlan + AggregateNode + HavingNode | ✅ 新增 2 节点 |
| src/executor/value.rs | Value::add/lt_agg/div | ✅ 新增算术方法 |
| src/parser/planner.rs | 聚合解析 + HAVING 构建 | ✅ 重构 |
| src/parser/ast.rs | Expr::Function 处理 | ✅ 修改 |
| src/parser/error.rs | 聚合错误类型 | ✅ 新增 4 变体 |
| src/pipeline.rs | Aggregate/Having executor 创建 | ✅ 修改 |

## 下一步行动

**优先级**: M16 子查询支持

**里程碑路线图**:
1. **M16**: 子查询支持
2. **M17**: 索引优化（B-Tree split/merge + 非唯一索引）
3. **M18**: WAL 集成 + 写入优化（INSERT 5-10x 提速）

**当前阻塞**: 无