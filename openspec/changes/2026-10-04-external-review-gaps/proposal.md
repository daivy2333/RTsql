# 外部评测反馈批：索引投影契约、恢复收敛、JOIN 排序、标量子查询、无 WHERE 写面、SQLSTATE 与文档一致性

## Why

2026-10-04 外部评测者提交 8 条反馈（隔离 RTSQL_HOME 实测）。本会话于 HEAD 9538647（release 二进制比全部源码新）逐条复测：**7 条成立、1 条细节不成立**，另发现 2 个反馈未点名的相邻缺陷。全部在当前代码成立（除注明外）：

1. **【崩溃，正确性红线】二级索引等值 + 非首列投影 panic（exit 101）**：`SELECT name FROM q6 WHERE salary = 90`（salary 上有二级索引）在 `src/parser/planner/query.rs:47`（执行路径 CLI 表头）与 `src/executor/plan_format.rs:348`（EXPLAIN）双点 panic，与反馈逐字一致。**相邻发现（更严重）**：投影列号恰在界内时（`SELECT name, id WHERE salary=90`）不崩但**静默返回表头与行错位的结果**（表头 `["id","name"]`、行 `[name值, id值]`）——静默错误结果面。根因（源码闭合）：`IndexScanAllNode.columns` 构造时（query.rs:1836-1840）被赋成已裁剪的投影列名，而 `node.projection` 存全表列号，消费方（query.rs:95、plan_format.rs:348）按「基表全列 × 投影号」契约再裁剪即越界/错位；PK 路由（IndexScan）因消费方持「已收窄」契约不受影响。引入提交 2f83dd9f（MS21）复制 PK 臂装配时违约于 d8a244f1（MS15 I034）确立的消费契约。
2. **【工具失效】崩溃恢复后 check 永久误报 pending WAL**：事务中途 SIGKILL → 纯 SELECT 的读写打开触发恢复（数据正确：已提交完好、未提交丢弃）→ close 走 MS18 零工作跳过（该形态恢复重放零页写入零 WAL 追加，`redo_count=0`）→ WAL 文件原样留存 → `check`/`--readonly` 永久 exit 1，`checkpoint` 报 "no pending work; wal untouched"，死循环。实测逃生通道：只有该会话发生**真实写入**才收敛。文档承诺（README.md:180、SKILL.md:156/441「open it read-write once to recover, then check」）对纯读恢复会话失实。反馈中「报错路径拼成 demo.wal 而真实侧车是 demo.db.wal」一项在当前版本**不成立**（现版侧车命名统一 `<stem>.wal`，报错路径与实际一致）——无工作项，随本条记录。
3. **【文档不符】JOIN 后 ORDER BY 限定列失败**：无 WHERE 无聚合的 JOIN 基座上 `ORDER BY e.id`/`e.salary`/`emp.id` 全部 `Column not found` exit 3（带 WHERE 的 JOIN 正常、单表正常）。`sql-table-alias-resolution` spec「限定名排序与既有语义零回归」（spec.md:102）的 JOIN 场景本就要求该行为——代码违反既有 spec。
4. **【文档不符】标量子查询做比较不支持**：`WHERE salary = (SELECT MAX(salary) FROM emp)`（含 `>`、PK 列）报 `Unsupported expression type: EXPRESSION` exit 3（`expression.rs:688` 兜底臂）。文档承诺「scalar and IN subqueries, correlated included」（SKILL.md:249、README.md:253）。**相邻发现**：标量子查询在 SELECT 投影位可用但输出形状异常（`SELECT name, (SELECT MAX(salary) FROM q6) FROM q6` 返回全表列 + `__subquery` 插 index 1，请求的投影被忽略——`query.rs:1685-1686` 把含子查询的投影恒等化，SubqueryEval 只插列从不裁剪）。
5. **【文档误导】UPDATE/DELETE 强制要求 WHERE**：两者均被拒（`Missing required field: WHERE clause`）。README.md:282（双语）与 SKILL.md:251 的 `SET v = v + 1`、`SET a = a + 1, b = a` 示例均无 WHERE，照抄必错；全部文档无任何「需要 WHERE」声明。**实现事实（调查修正）**：这不是拆门问题——`UpdateNode/DeleteNode.key` 是必填单键、执行器为单点操作，全表形态需要新执行形态（`IndexManager::scan_all` 原语现成，逐行 MVCC/门序/索引维护逻辑可复用）。
6. **【能力缺口】集合操作不能进子查询**：`FROM (… UNION …)` 被拒（planner query.rs:828-832 点名拒绝）。全链（SetOperation 计划/执行器/DerivedScan/列描述/参数注入）对输入形状通用，FROM 派生表内放行只差一道门。
7. **【输出缺口】SQLSTATE 不出现在 CLI 输出**：只读会话拒绝写等错误只有文字描述，文本与 JSON 均无 SQLSTATE；`Response::Error` 已携带可选 `sqlstate` 载荷（MS19，PG 协议消费），CLI 渲染未输出。README.md:192（双语）、SKILL.md:141/437 承诺 `25006`/`0A000`。
8. **【消息质量】**：a) `Unsupported expression type: EXPRESSION`——随反馈 4 的实现自然消失（该形态被支持）；b) `Missing required field: WHERE clause for UPDATE`——随反馈 5 的语义决策消失或改写；c) WAL 路径拼接——当前版本不成立（见 2）。**本条无独立工作项**。

上一轮外部评测（2026-10-03）的四缺口已随 `2026-10-03-practical-core-gaps` 修复归档；本轮 8 条为新增一批，来源同为外部实测；第 10/11 条为 Iteration 000 Review 报告的范围外 Issue 候选（2026-10-05 用户裁定并入）。

9. **【正确性红线，replan 并入】自连接等值 JOIN 行配对与投影归属双重缺陷**（Iteration 000 Act 阻塞期间发现，Plan Review 2026-10-04 独立核实，用户裁定并入）：自连接 Hash 等值 JOIN 的 ON 条件交换判定以归一真实表名比较（`resolve_qualified_column` 把书写限定符归一为真实键 → `extract_join_conditions` 的 `left_ref.table == right_table` 对两腿恒真）——左腿先写形态被恒误判交换，行配对错误；Hash 与 NLJ 两执行器的输出行归属按 `col.table_alias == left_table_name`（真实表名）判定，自连接两侧同名 → 全部列恒读左侧行，`e1.x`/`e2.x` 投影静默读错表。证据：改动前 release 二进制复现（正向 `ON e1.id = e2.manager_id` → `[["r1","r1"],["r2","r2"]]`，期望 `[["m1","r1"],["m2","r2"]]`；NLJ 路径 `LEFT JOIN` 同形态归属同错、配对正确）。既有语料唯一自连接用例 `ON a.id = b.id`（两侧行值恒同）恰好掩盖两缺陷。此即 Iteration 000 T3-S11 Blocker 的根因，修复后 S11 解除阻塞转验收。

10. **【能力缺口，2026-10-05 replan 并入】JOIN 基座输出列序恒为表序**：纯列清单 SELECT 的列序重排在 Join/NLJ 基座不生效（Scan 基座按 SELECT 序），表头与行自洽无错位但与标准 SELECT 序语义不符。两条路径同源：① 裸纯列清单的 `kept` 分支按表序过滤 `all_columns`；② WHERE/ORDER BY 裁剪路径 `resolve_join_projection_indices` 按 combined 表序收集命中索引。证据：Iteration 000 Cycle 001 Act 探针实测（S33 校准记录，Plan Review accepted 在案），T9 前后不变。
11. **【正确性红线，2026-10-05 replan 并入】三表链式 JOIN 全形态已坏**：单 FROM ≥3 表因子——hash+hash 链与 hash+NLJ 混合链计划期 `Column not found` 失败（装配循环每步把「当前迭代组合形状」当全作用域做存在性校验/裁剪，第三表列在第一步即点名）；仅 SELECT 前两表形态 `join.rs` panic exit 101（中间步 kept 裁剪使左输入行缺 ON 条件列，执行器键查表 `.expect` 崩溃）；且链上同名列的条件键按裸列名查表存在静默串列面。证据：replan 调查以改动前 release 二进制实测三形态（HEAD 9538647 与工作区同；T9 Preserve 边界未触及），Plan Review 001 Issue 候选在案。

## What Changes

工作面划分（用户要求明确区分）：

- **代码面（T1–T7、T9–T11）**：R1–R7、R9–R11 的行为修复，全部有 spec delta。
- **文档面（T8）**：README.md / README.zh-CN.md / rtsql-docs/SKILL.md 与 R1–R7 定稿行为同步；无独立 spec delta（文档声明随代码修复成真或按决策收窄）。
- **无工作项**：反馈 8c（WAL 路径拼接，当前版本不复现）。

需求基线：

- **R1 二级索引等值扫描列元数据契约**（域 `sql-secondary-indexes` ADDED）：IndexScanAll 节点 `columns` SHALL 恒为基表全列名（输出形状由 `projection` 基表列号决定）；任意投影组合（单列/乱序多列/后位列）SHALL 产出对齐的表头与行，不再 panic 不再错位；EXPLAIN 同契约；以该扫描为输入的聚合/派生/CTAS/INSERT SELECT 列解析 SHALL 正确；PK 路由（IndexScan 已收窄契约）与范围谓词回落 SHALL 零变化。
- **R2 恢复会话 WAL 收敛**（域 `wal-recovery-replay-integrity` ADDED + `ms07-rest-tx-checkpoint-pushdown` MODIFIED「Checkpoint 零工作跳过」）：打开时 WAL 曾含事务记录（任一非 Checkpoint 记录）的读写会话，其 close()/checkpoint() SHALL 执行完整 checkpoint 截断 WAL（该标志视同未决工作）；此后 `check`/`--readonly` SHALL 正常打开——「open read-write once to recover」对纯读会话成立；`checkpoint` 命令在该状态 SHALL 报告实际执行；「WAL 从无事务记录」形态的零工作跳过 SHALL 保持（MS18 语义）；redo>0 收敛路径（checkpoint_skip_test S5 等）零变化。
- **R3 JOIN 基座 ORDER BY 限定键**（域 `sql-table-alias-resolution` MODIFIED「限定名排序与既有语义零回归」）：无 WHERE 无聚合 JOIN 基座上 ORDER BY 两段限定键 SHALL 按组合行布局解析并真实排序；自连接同名列 SHALL 按限定符精确解析（`resolve_layout_index` 布局索引，消除裸名 first-hit 局限）；单表/带 WHERE JOIN/聚合/DISTINCT/集合操作尾 ORDER BY 既有语义 SHALL 零变化。
- **R4 WHERE 标量子查询比较与投影位形状**（域 `sql-expression-evaluation` ADDED ×2）：谓词比较腿（`= != < > <= >=`，任一侧）中的标量子查询 SHALL 支持——非相关（每语句一次）与相关（逐行参数注入 + 语句级缓存，复用 SubqueryEval/Correlated 机制）；空结果 → NULL → 三值语义（比较 Unknown → 行不匹配）；多行 SHALL 点名拒绝；含标量子查询的谓词 SHALL NOT 下推进扫描节点；JOIN 基座 WHERE 内标量子查询 SHALL 维持既有拒绝（镜像 InSubquery/Exists）。SELECT 投影位标量子查询输出形状 SHALL 与请求一致（投影裁剪生效、标量按 SELECT 位插入、未请求列不泄漏）。伴随：`Unsupported expression type: EXPRESSION` 在该面消失（反馈 8a 闭环）。
- **R5 FROM 派生表内集合操作**（域 `sql-set-operations` ADDED）：`FROM (SELECT … UNION [ALL]/INTERSECT/EXCEPT SELECT …)` SHALL 可作派生表源（列名取左臂，外层投影/WHERE/聚合/ORDER BY 照常）；IN/EXISTS/标量子查询内的集合操作 SHALL 维持既有点名拒绝（范围收口为评测点名形态）。
- **R6 无 WHERE 全表 UPDATE/DELETE**（域 `sql-write-surface` ADDED + `cross-db-dml` MODIFIED「UPDATE DELETE 保持已有语法边界」）：UPDATE/DELETE 省略 WHERE SHALL 作用于全部行——逐行版本写/墓碑、per 行类型门（FLOAT 升格）/NOT NULL/键列类型门、唯一与二级索引维护、碰撞预检照常；语句失败（含逐行碰撞预检命中）SHALL 隐式事务回滚全部已写行零残留；`affected_rows` SHALL 为实际行数；显式事务内 SHALL 可用可回滚（区别于 TRUNCATE 的 checkpoint-first 物理释放）；附加库目标经既有路由同语义（每文件独立提交）；TRUNCATE 与带 WHERE 形态（含子查询拒绝面）SHALL 零变化。
- **R7 CLI 错误输出携带 SQLSTATE**（域 `cli-noninteractive-shell` ADDED）：CLI 错误输出文本 SHALL 在 `Response::Error.sqlstate` 存在时附加 `[SQLSTATE xxxx]`；`--format json` SHALL 输出 `sqlstate` 字段（None 时省略）；错误文案与退出码 SHALL 零变化；PG 协议面零变化。
- **R8 文档一致性**（文档面 T8，无 spec delta）：README 双语与 SKILL.md 随 R1–R7 定稿行为同步——ORDER BY 限定列声明成真；标量子查询按 R4 范围声明；UPDATE 示例与无 WHERE 语义成真并显式声明；SQLSTATE 输出面说明；FROM 集合操作能力行；恢复收敛话术核对（R2 后承诺成立，可选补纯读会话说明）。
- **R9 自连接侧身份正确性**（域 `sql-table-alias-resolution` MODIFIED，replan 2026-10-04 并入）：自连接等值 JOIN 的 ON 条件侧归属 SHALL 按书写限定符与声明限定符（别名原文/归一命中/隐式短名）匹配判定，交换只发生在实际反序书写形态；双腿同侧 SHALL 计划期点名拒绝零副作用；裸名腿维持按位置解析既有语义；未知限定符维持 `TableNotFound`。Hash 与 NLJ 两执行器的输出行（含外连接补齐行）SHALL 按计划期显式侧标志归属，自连接 `e1.x`/`e2.x` 投影各读对应侧行；非自连接全部形态（正向/反向/裸名/别名/附加库/内/外连接）SHALL 零回归。伴随：T3-S11（自连接 ORDER BY 限定列）解除 `#[ignore]` 转验收。
- **R10 JOIN 基座输出列序与请求一致**（域 `join-output-shape-resolution` ADDED，replan 2026-10-05 并入）：纯列清单（裸路径与 WHERE/ORDER BY 裁剪路径）的 JOIN 基座输出列序 SHALL 按 SELECT 清单序（对齐 Scan 基座既有语义）；重复项 SHALL 重复产出；`SELECT *` 与 carry-all 全组合承载形态（聚合/派生/CTAS 输入、Sort/Filter 承载裁剪）SHALL 既有语义零变化；表达式项 Projection 路径既有 SELECT 序 SHALL 零回归；表头与行 SHALL 同源自洽。
- **R11 多表链式 JOIN 装配正确性**（域 `join-output-shape-resolution` ADDED，replan 2026-10-05 并入）：单 FROM ≥3 表因子的左深链 SHALL 可达——hash+hash 纯等值链与 hash+NLJ 混合链端到端产出正确配对行；每步等值 ON 条件键 SHALL 按（真实表, 列）限定解析，链上同名列 SHALL 不串列；中间节点 SHALL 承载链前缀全组合列且输出列索引为左输入行绝对偏移；仅 SELECT 部分表 SHALL 正常裁剪零 panic；两表单步全形态（含测试直构 `JoinConfig` 裸名键）SHALL 逐字节零回归；错误面（真未知列）SHALL 不变。

## 场景草图

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| S1 二级索引后位单列投影 | q6(id PK, name, dept_id, salary) + idx(salary)，2 行 | `SELECT name FROM q6 WHERE salary = 90` | 返回 `[["Ada"]]`、表头 `["name"]`（失败边界——修复前：panic exit 101 `query.rs:47`） |
| S2 乱序多列投影对齐 | 同上 | `SELECT name, id FROM q6 WHERE salary = 90` | 表头 `["name","id"]`、行 `["Ada",1]` 对齐（失败边界——修复前：静默错位） |
| S3 EXPLAIN 不崩 | 同上 | `EXPLAIN SELECT name FROM q6 WHERE salary = 90` | 计划树文本含 IndexScanAll 与正确列形状（修复前：panic `plan_format.rs:348`） |
| S4 PK 路由与范围谓词零回归 | 同上 | `SELECT name WHERE id=1`；范围谓词查询 | 行为零变化（既有契约不破坏） |
| S5 二级索引扫描上的聚合 | 同上 | `SELECT COUNT(*), MAX(id) FROM q6 WHERE salary = 90`；GROUP BY 组合 | 聚合与分组键经基表列号正确映射（修复前：input schema 误映射风险面） |
| S6 未提交事务崩溃后纯读收敛 | 15000 已提交行 + 显式事务未提交写入中途 SIGKILL | 纯 SELECT 读写打开 → close → `rtsql check` | 已提交数据完好；check exit 0 单行干净（修复前：永久 exit 1 pending WAL） |
| S7 checkpoint 命令报告实际执行 | 同 S6 恢复后（未再写入） | `rtsql checkpoint demo` | `wal truncated; checkpoint site advanced`（修复前：`no pending work; wal untouched`） |
| S8 零工作跳过保持 | 干净库（WAL 从无事务记录） | 纯读会话 close / 显式 checkpoint | 位点与 WAL 字节不变（MS18 S1/S6 语义保持） |
| S9 marker-only WAL 收敛 | 空事务 BEGIN;COMMIT 后崩溃形态 | 纯读读写打开 → check | 收敛 exit 0（修复前：同样楔死） |
| S10 JOIN ORDER BY 限定键 | emp e JOIN dept d（无 WHERE） | `SELECT e.name … ORDER BY e.id` / `e.salary` / `emp.id` | 按键真实排序（修复前：Column not found exit 3） |
| S11 自连接同名列精确解析 | emp e1 JOIN emp e2 ON … | `ORDER BY e1.salary` / `e2.salary` | 各按限定符对应列排序（顺带修复裸名 first-hit） |
| S12 排序既有面零回归 | 单表/带 WHERE JOIN/聚合/DISTINCT/集合尾 | 各既有 ORDER BY 形态 | 全部零变化 |
| S13 非相关标量比较 | emp 表 | `WHERE salary = (SELECT MAX(salary) FROM emp)`；`>` 变体 | 返回最高薪行（修复前：Unsupported expression type: EXPRESSION） |
| S14 相关标量比较 | emp/dept | `WHERE salary = (SELECT MAX(salary) FROM emp e2 WHERE e2.dept_id = e.dept_id)` | 逐行按部门最高薪匹配 |
| S15 空子查询三值语义 | 空结果子查询 | `WHERE v = (SELECT v FROM t WHERE 1=0)` | 零行（NULL 比较为 Unknown） |
| S16 多行标量点名拒绝 | 多行子查询 | `WHERE v = (SELECT v FROM t)` | `SubqueryReturnsMultipleRow` 既有错误面 |
| S17 拒绝面保持 | JOIN 基座 | `… JOIN … WHERE e.salary = (SELECT …)` | 维持点名拒绝（镜像 WHERE CLAUSE ON JOIN）；谓词不下推有测试见证 |
| S18 投影位形状 | q6 表 | `SELECT name, (SELECT MAX(salary) FROM q6) FROM q6` | 表头 `["name","__subquery"]`（或别名），未请求列不泄漏（修复前：全表列泄漏 + 位次错乱；subquery_test 参考形状校准） |
| S19 FROM 派生表集合操作 | dept/emp | `SELECT COUNT(*) FROM (SELECT id FROM dept UNION SELECT dept_id FROM emp)` | 可执行（修复前：点名拒绝）；列名取左臂 |
| S20 派生表组合 | 同上 | 外层 WHERE/投影/ORDER BY 叠加 | 照常生效 |
| S21 IN 内集合操作维持拒绝 | 同上 | `WHERE id IN (SELECT a FROM t UNION SELECT b FROM t)` | 点名拒绝（范围收口） |
| S22 全表 DELETE | t 有 N 行 | `DELETE FROM t` | affected_rows=N，逐行墓碑；重开一致；dump 无行 |
| S23 全表 UPDATE | 同上 | `UPDATE t SET v = v + 1` | 全部行 v+1；per 行类型门/NOT NULL/键列门生效 |
| S24 逐行碰撞回滚 | id PK 表全体 rekey 形态（如 `SET id = id + 100` 且中途碰撞） | 无 WHERE UPDATE | 语句失败、全部已写行回滚零残留（隐式事务语义）；显式事务内可手动 ROLLBACK |
| S25 附加库目标 | ATTACH 后 | 附加表无 WHERE UPDATE/DELETE | 经路由同语义、每文件独立提交（cross-db-dml 修订） |
| S26 TRUNCATE 与带 WHERE 零变化 | 同库 | `TRUNCATE TABLE t`；带 WHERE/带子查询（拒绝）形态 | 语义零变化 |
| S27 SQLSTATE 文本 | 只读会话 | `--readonly` 下 INSERT | 错误行含 `[SQLSTATE 25006]`，文案与 exit 3 不变 |
| S28 SQLSTATE JSON | 同上 | `--format json` | JSON 含 `"sqlstate":"25006"`；无 sqlstate 的错误省略字段 |
| S29 PG 面零变化 | serve/PG | 既有 pg_integration 断言 | 零回归 |
| S30 文档一致性 | — | 逐条对照实测 | R8 清单全部与行为一致 |
| S31 Hash 自连接正向配对 | emp2(id,name,manager_id)：`(1,'m1',NULL),(2,'m2',NULL),(10,'r1',1),(11,'r2',2)` | `SELECT e1.name, e2.name FROM emp2 e1 JOIN emp2 e2 ON e1.id = e2.manager_id` | `[["m1","r1"],["m2","r2"]]`（失败边界——修复前 `[["r1","r1"],["r2","r2"]]`） |
| S32 反序书写交换按限定符 | 同上 | 同查询 `ON e2.manager_id = e1.id` | 同 S31 结果（失败边界——修复前 `[["m1","m1"],["m2","m2"]]`） |
| S33 自连接投影归属 | 同上 | 两列同名任意投影组合 | 各列读其限定符对应侧行（修复前恒读左侧） |
| S34 自连接外连接（NLJ） | 同上 | `LEFT JOIN` 同 ON；FULL/非等值形态同归属语义 | `[["m1","r1"],["m2","r2"],["r1",NULL],["r2",NULL]]`（修复前全部 `[x,x]`） |
| S35 双腿同侧点名拒绝 | 同上 | `ON e1.id = e1.manager_id` | 计划期点名拒绝零副作用（修复前静默误判执行） |
| S36 非自连接零回归 | 独立表矩阵 | 正向/反向/裸名/别名/附加库/内/外连接 | 行为逐字节零变化 |
| S37 T3-S11 解除阻塞 | 同 S31 | 移除 `self_join_same_name_columns_resolve_by_qualifier` 的 `#[ignore]` | GREEN（`ORDER BY e1.salary`/`e2.salary` 精确解析） |
| S38 JOIN 基座 SELECT 序（裸路径） | emp3 e JOIN dept d | `SELECT d.dname, e.ename FROM …` | 表头与行均 `[dname, ename]` 序（失败边界——修复前表序 `[ename, dname]`） |
| S39 WHERE 路径 SELECT 序 | 同上 + WHERE | `SELECT d.dname, e.ename … WHERE e.id = 1` | `[dname, ename]` 序（失败边界——修复前 Filter 裁剪按表序） |
| S40 ORDER BY 路径 SELECT 序 | 同上 | `SELECT d.dname, e.ename … ORDER BY e.id` | Sort 裁剪序 = SELECT 序 |
| S41 重复项与全组合形态零回归 | 同上 | `SELECT e.ename, e.ename`；`SELECT *`；聚合/派生/CTAS 消费 JOIN 输出 | 重复项产出两列；`*` 表序不变；消费面表头随输出序自洽（CTAS 列名取输出表头与 Scan 基座一致） |
| S42 hash+hash 三表链 | emp/dept/sal 三表（sal.dept_id 外键） | 三表 SELECT 等值链 | 端到端可达且每步配对正确（失败边界——修复前 `Column not found` 计划失败） |
| S43 hash+NLJ 混合链 | 同上，第三步非等值 ON | 三表 SELECT | 可达且谓词作用全组合行（失败边界——修复前计划失败） |
| S44 仅前两表 SELECT | 同上 | `SELECT e.ename, d.dname FROM 三表链` | 正常输出前两表列（失败边界——修复前 panic exit 101） |
| S45 链上同名列不串列 | t1(id)/t2(id)/t3 链 | ON 条件跨链同名列（如 `t2.id = t3.x`） | 键按（真实表, 列）解析、配对正确（失败边界——修复前裸名查表静默串列/缺列 panic） |
| S46 两表单步零回归 | 既有矩阵 | 两表全形态（含测试直构 `JoinConfig` 裸名键、EXPLAIN、附加库） | 逐字节零变化 |

## 集中决策（2026-10-04 用户批准，原话「批准」）

2026-10-04 用户指令「开始计划工作，我们在一个 change 里面解决掉，分清 SKILL.md 调整与代码优化」。计划期四项语义决策按环境规则以推荐项为默认假设提交，用户于 2026-10-04 审计批准（原话「批准」），四项默认决策全部确认生效：

1. **无 WHERE UPDATE/DELETE = 支持全表形态**（R6）：标准 SQL 语义，文档示例照抄即对，反馈 8b 消息问题消失。备选（否决后启用）：维持拒绝 + 改文案 + 文档声明 WHERE 要求（T6 缩为小任务，cross-db-dml 修订取消）。
2. **SQLSTATE = CLI 输出携带**（R7）：文本后缀 + JSON 字段，承诺变真。备选（否决后启用）：文档收窄到 serve/PG（T7 取消，T8 改收窄声明）。
3. **标量子查询 = 非相关 + 相关都做**（R4）：文档「correlated included」变真。备选（否决后启用）：仅非相关 + 文档收窄（T4 缩小为构造期物化注值单路线）。
4. **子查询内集合操作 = 仅 FROM 派生表**（R5）：评测点名形态，范围收口。备选（否决后启用）：连 IN/EXISTS/标量内一起（`get_subquery_first_column` 补 SetOperation 臂，中成本）。

**replan 决策（2026-10-04）**：Iteration 000 Act 因自连接双重缺陷（范围外，反馈未点名、评测未触发）阻塞 T3-S11；Plan Review 独立核实（代码路径 + 改动前二进制行为复现）后按 Blocker Handoff 给出三选一。用户裁定（原话「同意，给出replan，把isuee也解决」）：方案 (a)——replan 并入本 change 修复（R9/T9），Issue 随 T9 交付解决，不另立 change、不落账独立 Issue；T3 的 S11 用例解除 `#[ignore]` 后并入 T9 验收。代价用户已知悉：本 change 体量增加一项中任务，Iteration 000 扩围为 T1/T3/T2/T9。

**replan 决策（2026-10-05）**：Iteration 001 首 Cycle 展开后、Act 未执行前，用户指令（原话「重新给出00cyc，把issue也计划解决进去」）——把 Iteration 000 Review 报告的两项范围外 Issue 候选并入本 change 解决：JOIN 基座 SELECT 序表序（R10/T10）、三表链式 JOIN 全形态已坏（R11/T11），不落账独立 Issue。Iteration 001 扩围为 T4/T5/T11/T10，以 replan Cycle `iterations/001-subquery/001-replan.md` 执行（原 `000-initial.md` 未执行、零工作区变化，冻结保留）。代价用户已知悉：Iteration 001 体量增加两项中-大任务，主题从「子查询面收口」扩为「子查询面与 JOIN 装配收口」。用户于 2026-10-05 审计批准该 replan 计划（原话「批准」）。

## 验收边界

- 每个需求有直接目标测试（新增）+ 既有套件定向回归 + 全量 `cargo test` 零回归。基线演进：1862（change 起点，HEAD 9538647）→ 1875 passed / 0 failed / 3 ignored（Iteration 000 部分交付后工作区现状，S11 `#[ignore]` 在案）；T9 落地后 S11 转 GREEN、ignore 回落 2 → **1882 passed / 0 failed / 2 ignored（Iteration 000 完成态，Cycle 001 Act Response + Plan Review 定向复跑采信，2026-10-05 replan 起点）**；Iteration 001 各任务落地后全量零回归、新增净增量以 Act Response 为准。
- R8 以人工逐条核对（对照实测命令输出）验收，不建判定工具。
- Persisted Evidence：`none`（全部可由测试命令 + Act Response 承载）。
