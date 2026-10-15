# EXISTS 内层表带别名时静默错误结果：非相关形态空集/按位截断，相关形态全放行或全拒

- Status: closed（2026-10-15 fixed → change `2026-10-15-subquery-alias-binding-and-error-prefix`：T1 声明限定符化消除幻影相关参数源 + T2 三执行器绑参未命中点名报错加固 + T4 e2e 见证；同批审计四形状一并修复）
- Filed: 2026-10-15
- Source: 用户实测评测（2026-10-15 上手实操评测）
- Environment: Linux x86_64、rtsql 0.2.0（`~/.local/bin/rtsql` 2026-10-15 安装二进制，源自源码 HEAD `23495df`）、rustc 1.95.0-nightly (859951e3c)、隔离 RTSQL_HOME 临时目录

## 缺陷描述

`WHERE EXISTS (SELECT 1 FROM <inner> <alias> WHERE ...)` 在内层表带别名时产生多种静默错误结果（全部 exit 0）；内层表不带别名的同形状查询全部正确。实测四种错误形态：

1. 非相关 + 谓词在内层非首列：恒为空集（子查询判定被整体丢弃）。
2. 非相关 + 谓词在内层首列：输出按「内层存活行数」截断外层行（1 个存活行只产出外层第 1 行）， EXISTS 被计划成位置配对而非存在性判定。
3. 相关 + 外层经非 PK 列关联（外层 5 列）：谓词不生效，全部外层行放行（期望只放行有匹配的行）。
4. 相关同形状在评测主库（orders↔people）表现为形态 1 的恒空集——同一绑定失效随数据/计划形态在「全放行」与「全拒」之间切换。

附带观察：外层别名与内层表名同名时（`FROM a6 t ... FROM t x`），内层 `t` 静默遮蔽外层别名 `t` 且相关引用未报未知限定符错误，退化为恒空集——限定符解析在别名遮蔽下无保护。

位置（Inferred）：SemiJoin/AntiJoin 执行与内层别名列解析路径（2026-10-04 change `external-review-gaps` T11 及 2026-10-13 change `e2e-verification-and-defect-closure` Iteration 001 均触碰该区）；登记时未独立追读确认。

根因：Unconfirmed。

## 影响

当前：EXISTS 是半连接语义的主力入口，带别名又是文档宣称的常规写法（「FROM table aliases across all clauses」）；四种形态全部静默错结果。真实查询（如「存在未完成的订单吗」）会得出恒否或恒是的结论。上游相关测试（subquery/semijoin 面）全绿，EXISTS×别名×列位置组合形状覆盖缺口确定存在。形态 3/4 表明错误结果随数据形态切换，同一条 SQL 在库 A 全放行、在库 B 全拒，排障成本极高。

## 事件记录

None（2026-10-15 用户实操评测中发现；未爆发为数据损坏或交付事件）。

## 处置

无（Status: open，待用户决定指派 change / MSxx）。建议与 ISS16、ISS18 同批调查——三个缺陷同属子查询别名/参数绑定面，可能共享根因。

## 证据

2026-10-15 评测实测，全新库逐字复现（`RTSQL_HOME` 隔离临时目录）：

```bash
rtsql new iss17
rtsql iss17 "CREATE TABLE a (id INTEGER PRIMARY KEY); CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT); CREATE TABLE a6 (id INTEGER PRIMARY KEY, ref INTEGER, c1 TEXT, c2 TEXT, c3 TEXT)"
rtsql iss17 "INSERT INTO a VALUES (1),(2); INSERT INTO t VALUES (1,'Lin'),(2,'Zed'); INSERT INTO a6 VALUES (1,1,'a','b','c'),(2,2,'d','e','f'),(9,9,'g','h','i')"

# 形态1 非相关+谓词非首列：期望 {1,2}，实际空集
rtsql --format table iss17 "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.name = 'Lin')"
# 形态2 非相关+谓词首列：期望 {1,2}，实际 {1}
rtsql --format table iss17 "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = 1)"
# 对照 内层无别名：期望 {1,2}，实际 {1,2}（正确）
rtsql --format table iss17 "SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t WHERE t.name = 'Lin')"
# 形态3 相关+经外层非PK列：期望 {1,2}（a6 第 3 行 ref=9 无匹配），实际 {1,2,9} 全放行
rtsql --format table iss17 "SELECT o.id FROM a6 o WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = o.ref)"
```

形态 4 旁证（评测主库 demo，orders 3 行、people/ppl3 10 行、EXPLAIN 两库计划形状一致均为 `SemiJoin correlated_params=2`）：

```sql
SELECT o.oid FROM orders o WHERE EXISTS (SELECT 1 FROM ppl3 x WHERE x.id = o.person_id)
-- 期望 {10,11,12}，实际空集；同语义非相关+无别名形状在 demo 上正确
```
