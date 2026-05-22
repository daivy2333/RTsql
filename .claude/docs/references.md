# 外部参考资料

> 最后更新：2026-05-22

## 核心依赖文档

| 库 | 版本 | 用途 | 文档 |
|----|------|------|------|
| Tokio | 1.x | async 运行时 | https://docs.rs/tokio |
| sqlparser-rs | 0.44 | SQL 解析 | https://docs.rs/sqlparser |
| serde_json | 1.x | JSON 输出 | https://docs.rs/serde_json |
| criterion | 0.5 | 基准测试 | https://bheisler.github.io/criterion.rs |
| rusqlite | 0.31 | SQLite 对比测试 | https://docs.rs/rusqlite |
| tempfile | 3.x | 测试临时目录 | https://docs.rs/tempfile |

## sqlparser-rs 0.44 关键 AST

| 类型 | 说明 |
|------|------|
| `GroupByExpr::All` | GROUP BY ALL |
| `GroupByExpr::Expressions(Vec<Expr>)` | 显式分组列 |
| `Expr::Function(Function)` | 函数调用（含聚合） |
| `FunctionArg::Unnamed(FunctionArgExpr::Wildcard)` | COUNT(*) 的 * |
| `FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))` | COUNT(col) 的 col |

## 数据库设计参考

| 主题 | 来源 |
|------|------|
| Volcano 迭代器模型 | Goetz Graefe "Volcano—An Extensible and Parallel Query Evaluation System" |
| Hash Aggregation | 《数据库系统概论》聚合查询章节 |
| MVCC | PostgreSQL MVCC 设计文档 |
| B-Tree 页格式 | SQLite B-Tree 页格式文档 |
| WAL | SQLite WAL 模式文档 |

## 项目测试统计

- 总测试数: 149
- M15 聚合测试: 19（aggregate_test.rs）
- 基准测试: 6 套（micro/concurrent/scale/sqlite_compare/single/precise_compare）