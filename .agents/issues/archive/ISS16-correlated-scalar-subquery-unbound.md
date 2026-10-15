# 相关标量子查询外层参数未绑定：WHERE 形态静默空集，select list 形态塌缩或运行期报错

- Status: closed（2026-10-15 fixed → change `2026-10-15-subquery-alias-binding-and-error-prefix`：T1 声明限定符化消除幻影相关参数源 + T2 三执行器绑参未命中点名报错加固 + T4 e2e 见证）
- Filed: 2026-10-15
- Source: 用户实测评测（2026-10-15 上手实操评测）
- Environment: Linux x86_64、rtsql 0.2.0（`~/.local/bin/rtsql` 2026-10-15 安装二进制，源自源码 HEAD `23495df`）、rustc 1.95.0-nightly (859951e3c)、隔离 RTSQL_HOME 临时目录

## 缺陷描述

相关（correlated）标量子查询的外层行参数绑定失效，子查询按无相关参数求值。文档明确宣称 "scalar and IN subqueries, correlated included — scalar subqueries serve in WHERE comparisons and the select list"，两种形态实测均不成立：

- WHERE 比较形态：外层每行的子查询本应经参数收窄到单行，实际未注入参数、对全表求值。子查询多行时按标量语义应报 `Subquery returns multiple rows`，实测最小形状下**静默返回空集**（exit 0）。
- select list 形态：缺陷随计划形态表现两种表象——在评测主库表现为**输出形状塌缩为全部基列且子查询列恒空**（静默）；在最小库表现为运行期 `Subquery returns multiple rows`（exit 3）。同一绑定失效，两种用户可见形态。

位置（Inferred）：标量子查询过滤/投影路径（`ScalarSubqueryFilterNode` 及投影位相关代码，2026-10-04 change `external-review-gaps` Iteration 001 T4 引入相关执行器）；登记时未独立追读确认。

根因：Unconfirmed。

## 影响

当前：任何使用相关标量子查询的查询静默产出错误结果（空集/塌缩形状），属「静默错误结果」正确性最高危同类；数据无损但查询结论不可信。潜在：文档宣称支持相关标量，用户会自然使用该形态做关联统计、存在性检查；错误无任何提示，极难自查。上游 `scalar_subquery_comparison_test`（14 例）与 `subquery_test`（43 例）全绿，相关形状测试覆盖缺口确定存在。触发条件与表数据量、列数无关（1–2 列最小库即复现），通用性强。

## 事件记录

None（2026-10-15 用户实操评测中以 WHERE 形态发现，select list 形态随后复现；未爆发为数据损坏或交付事件）。

## 处置

无（Status: open，待用户决定指派 change / MSxx）。

## 证据

2026-10-15 评测实测，全新库逐字复现（`RTSQL_HOME` 隔离临时目录）：

```bash
rtsql new iss16
rtsql iss16 "CREATE TABLE a (id INTEGER PRIMARY KEY); CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)"
rtsql iss16 "INSERT INTO a VALUES (1),(2); INSERT INTO t VALUES (1,'Lin'),(2,'Zed')"

# WHERE 形态：期望 {1,2}，实际空集，exit 0（静默）
rtsql --format table iss16 "SELECT a.id FROM a WHERE (SELECT COUNT(*) FROM t x WHERE x.id = a.id) = 1"

# select list 形态：期望 2 行（id + who），实际 exit 3 报错且错误前缀双层化（另见 ISS19）
rtsql --format table iss16 "SELECT a.id, (SELECT x.name FROM t x WHERE x.id = a.id) AS who FROM a"
# statement 1 of 1 failed: Execution error: execution error: Subquery returns multiple rows
#   (scalar subquery requires single row)
```

旁证（select list 塌缩形态，评测主库 demo，10 行 people / 3 行 orders）：`SELECT o.item, (SELECT name FROM people p WHERE p.id = o.person_id) AS who FROM orders o` 输出全部 5 个基列（oid/who/person_id/item/qty/price）且 who 列恒空，exit 0。
