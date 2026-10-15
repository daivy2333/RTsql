# IN 子查询内层带别名且谓词由二级索引服务时静默恒空集

- Status: closed（2026-10-15 fixed → change `2026-10-15-subquery-alias-binding-and-error-prefix`：T1 声明限定符化后别名谓词不再走幻影相关路径 + T4 e2e 见证）
- Filed: 2026-10-15
- Source: 用户实测评测（2026-10-15 上手实操评测）
- Environment: Linux x86_64、rtsql 0.2.0（`~/.local/bin/rtsql` 2026-10-15 安装二进制，源自源码 HEAD `23495df`）、rustc 1.95.0-nightly (859951e3c)、隔离 RTSQL_HOME 临时目录

## 缺陷描述

`WHERE <expr> IN (SELECT <col> FROM <inner> <alias> WHERE <predicate>)` 在内层表带别名、且谓词由二级索引服务时恒为空集（exit 0，静默）；内层不带别名的完全同形查询结果正确。别名 + 无索引谓词、无别名 + 索引谓词的组合均正确，触发条件为「别名 ∧ 索引服务谓词」的合取。

文档宣称 "scalar and IN subqueries, correlated included" 与 "An equality predicate on the indexed column is served automatically"，两特性交叉处结果错误。IN 子查询计划取首列的投影裁剪（2026-10-13 change `e2e-verification-and-defect-closure` Iteration 005 T8，`get_subquery_first_column` 经 IndexScan/IndexScanAll 两臂取 `projected_columns` 首项）与别名列解析的交互高度可疑。

位置（Inferred）：IN 子查询 IndexScan 计划构造的列投影/谓词下推路径；登记时未独立追读确认。

根因：Unconfirmed。

## 影响

当前：带索引的表上的 IN 反连接（最常见的「在集合中」过滤形态）静默漏行，属「静默错误结果」正确性最高危同类。错误结果恒为空集，用户极易把「无匹配」当成真实业务结论。上游相关测试全绿，`IN × 别名 × 索引谓词` 组合形状覆盖缺口确定存在。

## 事件记录

None（2026-10-15 用户实操评测中发现；未爆发为数据损坏或交付事件）。

## 处置

无（Status: open，待用户决定指派 change / MSxx）。建议与 ISS16、ISS17 同批调查——三个缺陷同属子查询别名/参数绑定面，可能共享根因。

## 证据

2026-10-15 评测实测，全新库逐字复现（`RTSQL_HOME` 隔离临时目录）：

```bash
rtsql new iss18
rtsql iss18 "CREATE TABLE a (id INTEGER PRIMARY KEY); CREATE TABLE c (id INTEGER PRIMARY KEY, age INTEGER)"
rtsql iss18 "INSERT INTO a VALUES (1),(2); INSERT INTO c VALUES (1,30),(2,41)"
rtsql iss18 "CREATE INDEX idx_c_age ON c (age)"

# 期望 {2}，实际空集（静默，exit 0）：
rtsql --format table iss18 "SELECT id FROM a WHERE a.id IN (SELECT x.id FROM c x WHERE x.age = 41)"
# 对照 内层无别名同形状：期望 {2}，实际 {2}（正确）：
rtsql --format table iss18 "SELECT id FROM a WHERE a.id IN (SELECT id FROM c WHERE c.age = 41)"
```

判别性事实：别名 + 谓词走索引 → 空集（本缺陷）；无别名 + 谓词走索引 → 正确；别名 + 无索引谓词（`SELECT x.aid FROM b x WHERE x.aid = 1`，评测 repro 库实测）→ 正确。
