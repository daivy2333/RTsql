## Purpose

索引项目依赖的内部产物和外部资料。条目使用 `Rxx` 编号，类型包括 dependency/external-doc/schema/runbook/analysis。

## Requirements

### Requirement: 参考可定位

参考 SHALL 记录类型、路径或 URL、版本或日期、用途和状态。

#### Scenario: 登记持久化产物

- **WHEN** 新分析、Runbook 或 Incident 需要跨会话复用
- **THEN** 使用递增 R 编号登记检索元数据

---

## 依赖文档

## R01: Cargo 运行时依赖

- **类型**: dependency
- **路径**: `Cargo.toml`
- **用途**: RTsql 运行时所需的 Rust crate 依赖
- **内容**:

  | 依赖 | 版本 | 链接 | 用途 |
  |---|---|---|---|
  | tokio | 1.x | https://docs.rs/tokio | async 运行时（rt-multi-thread, macros, sync, time, net, fs, io-util） |
  | sqlparser-rs | 0.44 | https://docs.rs/sqlparser | SQL 解析 |
  | async-trait | 0.1 | https://docs.rs/async-trait | async trait 支持 |
  | thiserror | 1.0 | https://docs.rs/thiserror | 错误类型派生 |
  | anyhow | 1.0 | https://docs.rs/anyhow | 错误处理 |
  | futures | 0.3 | https://docs.rs/futures | 异步原语 |
  | tokio-util | 0.7 | https://docs.rs/tokio-util | Tokio 工具（rt） |
  | serde | 1.0 | https://docs.rs/serde | 序列化框架 |
  | serde_json | 1.0 | https://docs.rs/serde_json | JSON 输出 |
  | rand | 0.8 | https://docs.rs/rand | 随机数 |
  | lru | 0.12 | https://docs.rs/lru | LRU 缓存（PlanCache） |
  | crc32fast | 1.4 | https://docs.rs/crc32fast | WAL CRC32 校验 |
  | dashmap | 6 | https://docs.rs/dashmap | 并发 HashMap（BufferPool vis_map） |

- **状态**: active
- **Legacy**: R001

## R02: Cargo 开发依赖

- **类型**: dependency
- **路径**: `Cargo.toml` [dev-dependencies]
- **用途**: RTsql 测试与基准所需依赖
- **内容**:

  | 依赖 | 版本 | 链接 | 用途 |
  |---|---|---|---|
  | criterion | 0.5 | https://bheisler.github.io/criterion.rs | 基准测试 |
  | rusqlite | 0.31 | https://docs.rs/rusqlite | SQLite 对比测试 |
  | tempfile | 3.x | https://docs.rs/tempfile | 测试临时目录 |
  | which | 6.0 | https://docs.rs/which | 查找可执行文件 |

- **状态**: active
- **Legacy**: R001

## R03: sqlparser-rs 0.44 关键 AST

- **类型**: external-doc
- **来源**: https://docs.rs/sqlparser/0.44
- **用途**: SQL 解析库的关键 AST 节点参考
- **内容**:

  | 类型 | 说明 |
  |---|---|
  | `GroupByExpr::All` | GROUP BY ALL |
  | `GroupByExpr::Expressions(Vec<Expr>)` | 显式分组列 |
  | `Expr::Function(Function)` | 函数调用（含聚合） |
  | `FunctionArg::Unnamed(FunctionArgExpr::Wildcard)` | COUNT(*) 的 * |
  | `FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))` | COUNT(col) 的 col |

- **状态**: active
- **Legacy**: R002

## 数据库设计参考

## R04: 数据库设计参考来源

- **类型**: external-doc
- **用途**: RTsql 架构设计的理论依据
- **内容**:

  | 主题 | 来源 |
  |---|---|
  | Volcano 迭代器模型 | Goetz Graefe "Volcano—An Extensible and Parallel Query Evaluation System" |
  | Hash Aggregation | 《数据库系统概论》聚合查询章节 |
  | MVCC | PostgreSQL MVCC 设计文档 |
  | B-Tree 页格式 | SQLite B-Tree 页格式文档 |
  | WAL | SQLite WAL 模式文档 |

- **状态**: active
- **Legacy**: R003

## 项目测试统计

## R05: 项目测试统计（2026-06-04）

- **类型**: schema
- **用途**: 当前测试覆盖与基准测试清单
- **内容**:
  - 总测试数: 475 tests pass, 0 failures（2026-06-04 统计；M31 完成后 481 tests pass）
  - Executor 测试: executor_test.rs（29 tests，含 M19 DataScan 8 tests）
  - 聚合测试: aggregate_test.rs（19 tests）
  - B-Tree 测试: btree_test.rs + btree_split_test.rs + btree_merge_test.rs（22 tests）
  - Visibility 测试: visibility_test.rs（5 tests，含 M21 页面级 MVCC）
  - 基准测试: 8 套（micro/concurrent/scale/sqlite_compare/single/precise_compare/data_scan/visibility）
- **状态**: active（数值会随实施更新）
- **Legacy**: R004

## 已迁移的旧 analysis 文档（指针）

## R06: M19 DataScan 路径分析（已实施）

- **类型**: analysis（已实施迁移）
- **状态**: completed
- **原因**: DataScan 已实施并归档到 M19 change；分析内容已沉淀到 K19 (实测性能) + M22 (数据页链表)
- **Legacy**: R007

## R07: M21 页面级 MVCC 遗留项分析（已解决）

- **类型**: analysis（已实施迁移）
- **状态**: completed
- **原因**: M21 遗留项 (DELETE mark_deleted + 惰性 set_all_visible + benchmark) 全部完成（commit `78a3b01`）；内容已沉淀到 K12 (mark_deleted) + K13 (惰性 set_all_visible)
- **Legacy**: R008

## 已归档 Change 索引

## R08: 2026-06-03-consolidate-m41-tx-id-atomic

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-consolidate-m41-tx-id-atomic/`
- **状态**: archived
- **内容**: M41 事务 ID AtomicU64 实施（commit `634764d` + `ee9ceee`）
- **关联决策**: D09
- **关联知识**: K16

## R09: 2026-06-03-consolidate-rules-into-claude-md

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/`
- **状态**: archived
- **内容**: 废弃 `openspec/specs/rules/`，规则合并到 CLAUDE.md
- **legacy carrier**: `openspec/changes/archive/2026-06-03-consolidate-rules-into-claude-md/archive/spec.md`（旧 rules.md 内容）+ `archive/CLAUDE.md.before`（旧 CLAUDE.md 内容）

## R10: 2026-06-03-m20-zero-copy-slotted-page-ref

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-m20-zero-copy-slotted-page-ref/`
- **状态**: archived
- **内容**: M20 零拷贝 SlottedPageRef 实施
- **关联决策**: D12 的 predecessor
- **关联知识**: K09 (闭包设计), K17 (性能实测)

## R11: 2026-06-03-m36-zero-copy-value-ref

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-03-m36-zero-copy-value-ref/`
- **状态**: archived
- **内容**: M36 零拷贝 ValueRef 实施
- **关联知识**: K18 (性能与局限)

## R12: 2026-06-04-m19-datascan-path

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-04-m19-datascan-path/`
- **状态**: archived
- **内容**: M19 DataScan 路径实施
- **关联知识**: K19 (1.81x-2.44x 提速), K22 (数据页链表)

## R13: 2026-06-04-m21-page-visibility-map

- **类型**: change-archive
- **路径**: `openspec/changes/archive/2026-06-04-m21-page-visibility-map/`
- **状态**: archived
- **内容**: M21 页面级 MVCC 实施
- **关联决策**: D11
- **关联知识**: K08, K09, K10, K12, K13
