# MS25 SQL 读面补全（外连接 / DISTINCT / 集合操作 / CTE / 顺带聚合扩展与 EXPLAIN / ORDER BY 别名排序） — Proposal

## Why

agent 日常 SELECT 代数存在成片能力缺口（R18 主题 2/3 差距 + Explorer 2026-09-24 定位 + I075 promoted 2026-09-26）：

- **外连接**：planner 仅支持 `Inner(On)`（`src/parser/planner/query.rs:319-321`），LEFT/RIGHT/FULL 一律 `UnsupportedJoinType` 点名拒绝——单侧匹配+补 NULL 的基础分析形态不可用。
- **SELECT DISTINCT**：`query.distinct` 在 planner 全无消费——**静默不去重**（重复行照常返回，不报错）。
- **WITH（CTE）**：`query.with` 在 planner 全无消费——**CTE 被静默丢弃**；CTE 名与基表同名时按基表查询（静默错误结果），否则 TableNotFound。
- **集合操作**：UNION/INTERSECT/EXCEPT 计划期点名拒绝（`src/parser/ast.rs` `extract_select_body` → `UnsupportedStatement("SET OPERATION")`）；FROM 派生表同拒绝（`"DERIVED TABLE"`）。
- **聚合扩展**：聚合仅六件（CountStar/Count/Sum/Avg/Min/Max），group_concat 缺席；`COUNT(DISTINCT x)` 的 `distinct` 量化词在聚合路径（`src/parser/planner/aggregate.rs::extract_aggregate_func`）不被读取——**静默按普通 COUNT 计算**。
- **EXPLAIN**：全无匹配——agent 无法观察查询计划形状。
- **ORDER BY 输出别名排序**（I075，MS19 Iteration 002 Review Minor 4 来源）：别名排序键计划期可解析但**静默保持输入序**（语料 `sql-scalar-functions` R5 与 `error-reporting` R7 锁定的既有语义），与 SQLite/PostgreSQL「按输出列排序」不符。

前三处静默面（DISTINCT / WITH / COUNT DISTINCT）属 MS23/MS24 清理过的「静默错误结果」同类——本 change 一并收口，不是纯增量。

## What Changes

语义基准：**SQLite 对齐**（用户裁定 2026-09-26）。

- **T01 OUTER JOIN 三态**（`sql-outer-join`，新）：LEFT [OUTER] JOIN 交付（匹配行组合 + 未匹配左行 NULL 右补；ON 谓词三值 NULL 语义＝不匹配；非等值 ON 经既有 NLJ 通道）；RIGHT/FULL 入范围，实现面随 Phase 2 NLJ 改造成本调查裁定（超预期按 split signal 提请拆出，LEFT 先行收口）。既有 INNER 面零回归。
- **T02 SELECT DISTINCT**（`select-distinct`，新）：输出行去重——单列/多列元组、NULL 相等折叠、投影表达式按求值后值去重；与 ORDER BY（输出项）、聚合输出组合；DISTINCT ON 等不支持形态点名拒绝。修复静默不去重缺陷。
- **T03 集合操作**（`sql-set-operations`，新）：UNION [ALL] / INTERSECT / EXCEPT 双分支与链式复合；去重语义 NULL 相等；列数不匹配点名拒绝；INTERSECT 优先级高于 UNION/EXCEPT（左结合同级）；ORDER BY/LIMIT 作用于复合结果；INTERSECT ALL/EXCEPT ALL 与分支括号化点名拒绝。
- **T04 非递归 CTE**（`sql-common-table-expressions`，新）：顶层 WITH 可达——单/多定义、前向引用（后定义引用先定义）、列别名列表、CTE 名遮蔽基表、跨引用行集一致（同一语句快照；当前实现按引用内联求值=每引用一次派生表物化，共享物化为后续优化）；RECURSIVE 关键字（含非递归内容）、自引用成环、子查询内 WITH 点名拒绝。修复静默丢弃缺陷。
- **T05 顺带聚合扩展 + EXPLAIN**（`sql-aggregate-extensions` / `explain-plan-output`，新）：`group_concat(X[, sep])`（默认逗号、字面量分隔符、NULL 跳过、全 NULL 组 → NULL）；`COUNT(DISTINCT X)` 真实去重计数（修复静默忽略）+ `SUM/AVG(DISTINCT X)`（MIN/MAX 接受 DISTINCT 等价普通形态，SQLite 对齐）；`EXPLAIN <SELECT>` 最小计划树文本（节点名 + 关键参数，不执行语句；DML/DDL 点名拒绝）。
- **T06 ORDER BY 输出别名排序**（`order-by-alias-sorting`，新 + 两处语料修订）：排序键解析升级为「输出别名（输出名优先）→ 输入列」语义序，别名键按输出列真实排序；同步修订语料 `error-reporting` R7 与 `sql-scalar-functions` R5 的「静默保持输入序」语义；未知键 `ColumnNotFound` 拒绝与既有可达形态零回归。
- **T08 JOIN 输出形状与名字解析收口**（`join-output-shape-resolution`，新；2026-09-27 replan 增补）：消除聚合×JOIN 与 JOIN 投影解析的三面静默缺陷（2026-09-27 真实二进制实证，Iteration 000 Act Issue 候选经用户裁定并入）——①聚合参数不在 SELECT 清单时静默 NULL；②裸名同列 GROUP BY/聚合参数经 `column_indices` HashMap 末位胜出静默错误分组；③JOIN 基座未知列/别名列零宽行。目标语义：聚合×JOIN 在完整组合形状上解析（限定名精确、参数 plan 期解析为索引）；同名裸名歧义 `AmbiguousColumn` 点名拒绝（SQLite 对齐）；未知列 `ColumnNotFound` 点名拒绝；纯列引用别名列项可达；既有无歧义面零回归。
- **T09 聚合输出形状一致性收口**（`aggregate-output-shape-conformance`，新；2026-09-27 用户裁定并入 Iteration 004）：消除聚合直出路径的行/表头错配（Iteration 001 Act Issue 候选经用户裁定不单独落账、并入本 change 收口）——`SELECT count(*) FROM t GROUP BY g` 现状表头 1 列（`count_star`）而行 2 值（分组键 ++ 聚合值，query.rs 直出路径 `node_output_columns = agg_output_columns` 不含未入选分组键，执行器分组行恒为 `group_key ++ aggregates`）；同类第二错配面：SELECT 键序 ≠ GROUP BY 序时行按 GROUP BY 序、表头按 SELECT 序置换错位。目标语义（SQLite/PostgreSQL 对齐）：聚合查询输出行与表头恒为 SELECT 清单形状——未入选的分组键不进输出（仅供 HAVING 绑定）；形状不安全的查询改走既有包装路径（wrap_projection 重排、头 = SELECT 名）；既有真直出形态逐字节不变。

## 范围与决策记录（用户批准 2026-09-26）

1. **单 change 全量六任务**（选项「单 change 全量」）：四个 Iteration——000 OUTER JOIN / 001 DISTINCT+集合操作+顺带聚合 / 002 CTE+EXPLAIN / 003 ORDER BY 别名排序；每 Iteration 独立验收。
2. **OUTER JOIN 三态入范围 + 调查裁定**：LEFT 确定；RIGHT/FULL 由 Phase 2 NLJ 改造成本调查定实现面，成本超预期时按 split signal 提请拆出并回报，LEFT 先行收口。
3. **语义基准 SQLite 对齐**：DISTINCT/集合操作 NULL 相等；INTERSECT ALL/EXCEPT ALL 点名拒绝；CTE 多定义+前向引用+名字遮蔽基表；group_concat SQLite 语义；ORDER BY 输出名优先；递归 CTE/窗口函数/LATERAL 点名拒绝。
4. **EXPLAIN 最小计划树**：仅 SELECT 族、不执行语句、节点名+关键参数文本；DML/DDL 点名拒绝。
5. **D2 与既有见证冲突的 replan 裁定**（Iteration 000 Act 阻塞 → Plan Review，2026-09-27 用户裁定方案 (a)）：维持 WHERE×JOIN 统一解禁（含 INNER）；两处既有见证按前提退役校准（`nested_loop_join_test.rs::where_over_non_equi_join_stays_rejected` 改写为可用性见证；`subquery_test.rs::test_in_subquery_where_join_keeps_existing_rejection` 期望文案校准为 `InSubqueryJoinUnsupported`——拒绝保留、文案与无 WHERE 形态统一）；新增 `in-subquery-join-rejection` MODIFIED delta（WHERE+JOIN 场景文案随 D2 更新）；校准白名单记录于 `sql-outer-join` R4。
6. **T08 并入（2026-09-27 用户裁定）**：Iteration 000 Act 遗留 Issue 候选（聚合×JOIN 同名列末位胜出错误分组）经 Plan 存疑核查实证成立且实际缺陷面更宽（三面：聚合参数静默 NULL / 裸名同列末位胜出 / JOIN 基座零宽行——均为 MS25 前既有静默面，D2 仅扩大可达面），用户裁定不单独落账 Issue、以 **T08 + Iteration 004** 并入本 change 收口；因果表述由「D2 引入」修正为「MS25 前既有、D2 扩大可达」。
7. **T09 并入（2026-09-27 用户裁定）**：Iteration 001 Act 报告的 Issue 候选（聚合直出路径行/表头错配——`SELECT count(*) FROM t GROUP BY g` 表头 1 列而行 2 值，机制经 Plan Review 独立核实：query.rs 直出路径 `node_output_columns = agg_output_columns` vs 执行器分组行 `group_key ++ aggregates`）经用户裁定**不单独落账 Issue**，以 **T09 并入已规划的 Iteration 004**（与 T08 同为聚合输出装配/形状解析面，共享 `query.rs` 聚合分支 + `executor/aggregate.rs` + `plan.rs::AggregateNode` 诊断边界；Iteration 000 Minor 1 并入 T08 同先例）。目标语义 SQLite/PostgreSQL 对齐：输出行与表头恒为 SELECT 清单形状。

## 默认假设（Gate 1 审阅项；均为 SQLite 对齐推论，可逐项推翻）

- **DISTINCT × ORDER BY**：DISTINCT 查询的 ORDER BY 项 SHALL 出现在输出列集（输出名或表达式文本），非输出项点名拒绝（SQLite DISTINCT 约束对齐）。
- **集合操作类型面**：分支间列类型混合允许（行按 Value 流动）；去重/匹配用既有 Value 等值语义 + NULL 相等；分支列数不匹配点名拒绝。
- **CTE 范围**：仅顶层语句 WITH；子查询内 WITH 点名拒绝（后续需要时另行扩展）；CTE 未定义引用按既有 TableNotFound；已定义未引用合法。
- **group_concat 分隔符**：仅字面量；非字面量表达式分隔符点名拒绝。
- **ORDER BY 位置序号与表达式/限定名键**（`ORDER BY 1`、`ORDER BY t.col`、`ORDER BY a*10`）：不在范围——现状已诚实拒绝（`ParseError`「ORDER BY only supports column names」，`query.rs::extract_column_name` 仅接受裸标识符），保持不变。
- **INNER JOIN USING**：现状为约束静默丢弃（`query.rs:320` `Inner(_) => None`）——本 change 不触碰（范围控制），仅外连接形态对 USING 点名拒绝。

## Capabilities

### Added Capabilities

- `sql-outer-join`（T01）
- `select-distinct`（T02）
- `sql-set-operations`（T03）
- `sql-common-table-expressions`（T04）
- `sql-aggregate-extensions`（T05 聚合扩展）
- `explain-plan-output`（T05 EXPLAIN）
- `order-by-alias-sorting`（T06）
- `join-output-shape-resolution`（T08，2026-09-27 replan 增补）
- `aggregate-output-shape-conformance`（T09，2026-09-27 用户裁定并入）

### Modified Capabilities

- `error-reporting`：R7「ORDER BY 排序键计划期校验」——输出别名键从「静默保持输入序」升级为「按输出列真实排序」（T06）。
- `sql-scalar-functions`：R5「调用面与边界」——ORDER BY 表达式别名语义同步升级（T06）。
- `in-subquery-join-rejection`：R1「IN 子查询 JOIN 形态诚实拒绝」——子查询内 `WHERE + JOIN` 的拒绝文案随 D2 统一为 `InSubqueryJoinUnsupported`（拒绝语义不变；replan 裁定 2026-09-27 方案 (a)）（T01）。

## Impact

- **代码**（Phase 2 调查后精确定位）：
  - `src/parser/planner/query.rs` — JOIN 臂扩展（LeftOuter/RightOuter/FullOuter）、`query.distinct`/`query.with` 消费、排序键解析序升级
  - `src/parser/planner/mod.rs` / `src/parser/ast.rs` — 顶层 Query 结构消费（WITH/集合操作）与 AST 放行清单扩展
  - `src/parser/planner/aggregate.rs` — DISTINCT 量化词消费 + group_concat 注册
  - `src/executor/` — 外连接扩展（`join.rs` Hash 臂 / `nested_loop_join.rs` NLJ 臂）、新去重执行器（DISTINCT 与集合操作共用）、CTE 物化节点、EXPLAIN 输出面
  - `src/executor/plan.rs`（PhysicalPlan 新变体）+ `src/pipeline.rs`（构造臂接线）
- **测试**：每能力独立测试文件（outer_join / select_distinct / set_operations / cte / aggregate_extension / explain / order_by_alias）+ planner_test 计划形状与拒绝矩阵 + cli_test 错误面 e2e；全量串行零回归（M19）；静默面修复各含 RED→GREEN 见证（现状静默错误用例先行）
- **已知边界**：plan cache key 为 SQL 文本——WITH/集合操作语句文本自带完整结构，key 面无新变化（Phase 2 复核）

## 场景草图

### T01 外连接

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 左表 3 行、右表 2 行（1 行左无匹配） | `SELECT ... FROM l LEFT JOIN r ON l.id = r.id` | 3 行：匹配 2 行组合 + 1 行左值右 NULL | ON 为 NULL 时按三值语义不匹配→右 NULL |
| 同上 | `RIGHT JOIN` | 3 行：匹配 2 行 + 1 行右值左 NULL | 调查裁定后交付 |
| 同上 | `FULL JOIN` | 4 行：匹配 2 + 左补 1 + 右补 1 | 调查裁定后交付 |
| 外连接结果集 | 外接 WHERE 过滤 / ORDER BY / LIMIT | 过滤与排序作用于补 NULL 后行流 | WHERE 引右列=值把未匹配行滤掉（标准行为） |
| 任意 | NATURAL JOIN / CROSS APPLY 等 | 计划期点名拒绝（现状保持） | — |

### T02 DISTINCT

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 表含重复值 | `SELECT DISTINCT col FROM t` | 去重后行集 | 现状 RED：静默不去重 |
| 多列元组重复 | `SELECT DISTINCT a, b FROM t` | 按元组去重 | NULL 与 NULL 视为相等折叠为一行 |
| 表达式投影 | `SELECT DISTINCT upper(name) FROM t` | 按求值后值去重 | — |
| DISTINCT 结果 | `ORDER BY <输出项>` | 排序生效 | ORDER BY 非输出项点名拒绝 |
| 任意 | `SELECT DISTINCT ON(col) ...` | 点名拒绝 | — |

### T03 集合操作

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 两表部分重叠 | `SELECT a FROM t1 UNION SELECT a FROM t2` | 并集去重（NULL 相等） | 现状：UnsupportedStatement 点名拒绝 |
| 同上 | `UNION ALL` | 保留重复与序 | — |
| 同上 | `INTERSECT` / `EXCEPT` | 交集 / 差集（去重） | NULL 相等参与匹配 |
| 列数不同分支 | `SELECT a,b ... UNION SELECT a ...` | 点名拒绝 | — |
| 三分支链 | `a UNION b EXCEPT c` | 左结合同级、INTERSECT 先结合 | INTERSECT ALL/EXCEPT ALL 拒绝 |
| 复合结果 | 复合语句尾 ORDER BY/LIMIT | 作用于复合输出 | — |

### T04 CTE

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 任意库 | `WITH c AS (SELECT ...) SELECT * FROM c` | CTE 行集可达 | 现状 RED：WITH 静默丢弃 |
| 同名基表存在 | CTE 名与基表同名 | CTE 遮蔽基表 | 现状静默按基表查询 |
| CTE 被引用两次 | `... FROM c a JOIN c b ...` | 各引用行集一致（内联求值，同一快照） | — |
| 多定义 | `WITH a AS (...), b AS (SELECT ... FROM a) ...` | 前向引用合法 | 反向引用（前定义引后定义）拒绝 |
| 任意 | `WITH RECURSIVE c AS (含自引用) ...` | 点名拒绝 | 非递归内容带 RECURSIVE 关键字同拒绝 |

### T05 聚合扩展与 EXPLAIN

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 表含重复 | `SELECT count(DISTINCT col) FROM t` | 去重非 NULL 计数 | 现状 RED：静默按 COUNT(col) |
| 分组数据 | `group_concat(name)` / `group_concat(name, '\|')` | 逗号连接 / 自定义分隔符 | NULL 跳过；全 NULL 组 → NULL；非字面量分隔符拒绝 |
| 任意 | `EXPLAIN SELECT ...` | 计划树文本、语句不执行 | EXPLAIN INSERT/DDL 点名拒绝；缺表按计划期错误 |

### T06 ORDER BY 别名排序

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 表 name='b','a' | `SELECT upper(name) AS u FROM t ORDER BY u` | 按输出列排序（A、B） | 现状 RED：静默输入序 |
| 别名与源列同名 | `SELECT name AS x, id FROM t ORDER BY x` | 别名（输出列）优先 | SQLite 输出名优先对齐 |
| 既有形态 | 基础列排序 / 聚合投影列排序 / 未知键 | 零回归 / 零回归 / ColumnNotFound | — |

### T08 JOIN 输出形状与名字解析收口（2026-09-27 增补）

种子：`p(g PK, x)` 含 (1,100),(2,100)；`q(h PK, x)` 含 (5,200),(6,300)；非等值 `ON p.g < q.h`（2026-09-27 真实二进制实证）。

| 前置 | 动作 | 结果 | 失败/边界 |
|---|---|---|---|
| 同上 | `SELECT p.g, sum(p.x) … GROUP BY p.g`（参数不在 SELECT 清单） | 每组 200（按 p.x 求和） | 现状 RED：静默 NULL |
| 同上 | `SELECT x, count(*) … GROUP BY x`（裸名同列） | `AmbiguousColumn` 点名拒绝 | 现状 RED：末位胜出按 q.x 错误分组 |
| 同上 | `sum(x)`（裸名歧义参数） | `AmbiguousColumn` 点名拒绝 | 现状静默取末位 |
| 同上 | `SELECT nosuch FROM p JOIN q …` | `ColumnNotFound` 点名拒绝 | 现状 RED：零宽行 |
| 同上 | `SELECT p.g AS pg FROM p JOIN q …`（纯列引用别名） | 可达，表头 = pg | 现状 RED：零宽行 |
| 同上 | 任意表达式项 × JOIN / 无歧义既有形态 | 维持拒绝 / 逐字节不变 | — |
