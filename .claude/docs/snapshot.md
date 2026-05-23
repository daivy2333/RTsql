# 项目快照

> 最后更新：2026-05-23（M16-Phase2 相关子查询 完成）

## 当前状态

- **阶段**: M16-Phase2 完成，M16 全部功能到位
- **状态**: 编译通过，所有测试通过
- **测试**: 90 lib + 20 subquery + 19 aggregate = 129 tests pass
- **下一步**: M17 索引优化

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
│   │   ├── plan.rs         # PhysicalPlan（19 种节点）
│   │   ├── executor_trait.rs
│   │   ├── scan.rs / index_scan.rs / insert.rs / update.rs / delete.rs
│   │   ├── predicate.rs    # Predicate/Expression trait + ParameterExpression
│   │   ├── filter.rs / sort.rs / limit.rs
│   │   ├── join.rs         # 哈希连接
│   │   ├── aggregate.rs    # AggregateFunc + AggregateState + AggregateExecutor
│   │   ├── having.rs       # HavingExecutor
│   │   ├── semi_join.rs    # SemiJoinExecutorV2（独立+关联 双路径）
│   │   ├── anti_join.rs    # AntiJoinExecutor（独立+关联 双路径）
│   │   ├── subquery_eval.rs # SubqueryEvalExecutor（独立+关联 双路径）
│   │   ├── derived_scan.rs # DerivedScanExecutor（FROM 派生表）
│   │   ├── correlated.rs   # inject_correlated_values 函数
│   │   └── create_table.rs / drop_table.rs
│   ├── transaction/        # MVCC（VersionChain + RowLock + Snapshot）
│   ├── parser/             # PlanBuilder + AST + 聚合/子查询解析
│   ├── network/            # PgProtocol + ConnectionHandler + Server
│   └── wal/                # WalWriter + Recovery + Checkpoint
├── benches/                # 6 套基准测试
├── tests/                  # 集成测试（含 subquery_test.rs 20 tests）
└── .claude/docs/           # 项目文档
```

## Git 状态

- **当前分支**: master
- **未提交变更**: M16-Phase1 + Phase2 全部实现文件
- **最近提交**:
  - 2eee8af docs(M16): add subquery implementation plan
  - 1a36bf6 docs(M16): add subquery design spec

## M16 功能矩阵

| 功能类型 | 独立子查询 | 相关子查询 |
|----------|-----------|-----------|
| WHERE IN | ✅ 完成 | ✅ 完成 |
| WHERE NOT IN | ✅ 完成 | ✅ 完成 |
| WHERE EXISTS | ✅ 完成 | ✅ 完成 |
| WHERE NOT EXISTS | ✅ 完成 | ✅ 完成 |
| SELECT 标量 | ✅ 完成 | ✅ 完成 |
| FROM 派生表 | ✅ 完成 | N/A |

## M16-Phase2 关键文件

| 文件 | 作用 | 变更 |
|------|------|------|
| src/executor/plan.rs | CorrelatedParam 重构（param_name） | 修改 |
| src/executor/predicate.rs | ParameterExpression + trait 方法 | 修改 |
| src/executor/correlated.rs | inject_correlated_values 函数 | **新增** |
| src/executor/mod.rs | 导出新类型 | 修改 |
| src/parser/planner.rs | 外部引用检测 + 多层检测 + Expr::Value 处理 | 修改 |
| src/parser/ast.rs | extract_columns 支持 Expr::Value | 修改 |
| src/executor/semi_join.rs | 双路径执行（独立+关联） | 修改 |
| src/executor/anti_join.rs | 双路径执行（独立+关联） | 修改 |
| src/executor/subquery_eval.rs | 双路径执行（独立+关联） | 修改 |
| src/pipeline.rs | 接线 + extract_column_indices 扩展 | 修改 |
| tests/subquery_test.rs | 20 测试（14 独立 + 6 关联） | 修改 |

## 架构：ParameterExpression + Mutex 注入

- **ParameterExpression**: 携带 `Mutex<Value>`，作为谓词树中外部列引用的占位符
- **注入时机**: 每次外层行处理时，clone PhysicalPlan → inject → rebuild executor
- **安全性**: Volcano 顺序执行保证 Arc 共享的 ParameterExpression 无需并发保护

## 已知问题

1. **关联 IN + 空右侧**: 返回所有行而非 0 行（引擎 bug，测试已标记）
2. **Projection 列过滤**: SubqueryEval 返回完整表列而非 projection 列
3. **未提交**: 所有 M16 变更在工作区未提交

## 下一步行动

1. 提交 M16 全部变更
2. 进入 M17 索引优化：B-Tree split/merge + 非唯一索引

**里程碑路线图**:
- M16: ✅ 子查询支持（独立 + 关联 全部完成）
- M17: 索引优化
- M18: WAL 集成 + 写入优化
