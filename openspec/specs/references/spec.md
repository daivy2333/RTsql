# References — 外部参考与依赖

> 版本：v1.1 | 最后更新：2026-06-04（R004 测试统计更新 + R008 状态更新）
> 由 openspec-init 从 `.claude/docs/references.md` 迁移。
> 条目格式: <!-- R{编号} --> 标记开头，支持 grep 精确定位。

---

## Purpose

记录 RTsql 项目的所有外部依赖、技术参考文档和领域知识，便于查阅依赖版本、API 文档和设计参考来源。

---

## Requirements

### Requirement: 依赖版本可查

所有项目依赖 SHALL 记录版本号和文档链接。

#### Scenario: 查阅依赖 API
- **WHEN** 需要了解某个依赖库的 API 用法
- **THEN** 在依赖文档表中查找对应库的版本和链接

#### Scenario: 升级依赖
- **WHEN** 计划升级某个依赖版本
- **THEN** 更新依赖文档表中的版本号

### Requirement: 技术参考可溯

数据库设计参考 SHALL 记录来源论文或文档。

#### Scenario: 理解设计决策背景
- **WHEN** 需要理解某个架构设计的理论依据
- **THEN** 在数据库设计参考表中查找对应主题的来源

---

## 依赖文档

<!-- R001 --> | 依赖 | 版本 | 链接 | 用途 |
|------|------|------|------|
| Tokio | 1.x | https://docs.rs/tokio | async 运行时（rt-multi-thread, macros, sync, time, net, fs, io-util） |
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
| criterion | 0.5 | https://bheisler.github.io/criterion.rs | 基准测试 |
| rusqlite | 0.31 | https://docs.rs/rusqlite | SQLite 对比测试 |
| tempfile | 3.x | https://docs.rs/tempfile | 测试临时目录 |
| which | 6.0 | https://docs.rs/which | 查找可执行文件 |

---

## sqlparser-rs 0.44 关键 AST

<!-- R002 -->
| 类型 | 说明 |
|------|------|
| `GroupByExpr::All` | GROUP BY ALL |
| `GroupByExpr::Expressions(Vec<Expr>)` | 显式分组列 |
| `Expr::Function(Function)` | 函数调用（含聚合） |
| `FunctionArg::Unnamed(FunctionArgExpr::Wildcard)` | COUNT(*) 的 * |
| `FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))` | COUNT(col) 的 col |

---

## 数据库设计参考

<!-- R003 -->
| 主题 | 来源 |
|------|------|
| Volcano 迭代器模型 | Goetz Graefe "Volcano—An Extensible and Parallel Query Evaluation System" |
| Hash Aggregation | 《数据库系统概论》聚合查询章节 |
| MVCC | PostgreSQL MVCC 设计文档 |
| B-Tree 页格式 | SQLite B-Tree 页格式文档 |
| WAL | SQLite WAL 模式文档 |

---

## 项目测试统计

<!-- R004 -->
- 总测试数: 475 tests pass, 0 failures（2026-06-04 统计）
- Executor 测试: executor_test.rs（29 tests，含 M19 DataScan 8 tests）
- 聚合测试: aggregate_test.rs（19 tests）
- B-Tree 测试: btree_test.rs + btree_split_test.rs + btree_merge_test.rs（22 tests）
- Visibility 测试: visibility_test.rs（5 tests，含 M21 页面级 MVCC）
- 基准测试: 8 套（micro/concurrent/scale/sqlite_compare/single/precise_compare/data_scan/visibility）

---

## 领域知识笔记

<!-- R005 -->
> 由 openspec-explorer 写入，由 openspec-assistant 日常维护。
> 添加时格式: <!-- R{编号} --> 笔记内容

---

## 项目分析文档

<!-- R006 -->
> 由 openspec-explorer 写入，由 openspec-assistant 日常维护，由 openspec-archivist 周期清理。
> 添加时格式: <!-- R{编号} --> | 主题 | 路径 | 内容概要 |

<!-- R007 --> | M19 DataScan 路径 | .claude/analysis/m19-datascan-path.md | 数据页链表遍历优化方案，跳过索引层，~2x 全表扫描提速 |
<!-- R008 --> | M21 页面级 MVCC 遗留项分析（✅ 已解决） | .claude/analysis/m21-page-visibility-incomplete.md | DELETE mark_deleted + 惰性 set_all_visible + benchmark，全部完成 |
