# practical relational core 缺口批：建表页初始化、聚合嵌套、UPDATE 表达式、非 INT PK 诚实化

## Why

外部评测（2026-10-03，探针实测全部复现，探针输出见会话记录）确认的四项定位内缺口——其中 ① 是真 bug，②③ 是「practical relational core」自宣称能力面内的实测缺失，④ 是约束诚实化哲学的一致性缺口。四项文档均未声明（SKILL.md 已于本轮直接补写 ②③ 的现状边界，①④ 随本 change 修复）：

1. **空表 check 误报（bug）**：`CREATE TABLE` 在 `src/storage/data/table_manager.rs:424` 经 `allocate_page()` 分配首数据页后从不写它（直接去建索引、写 catalog），页保持零字节（page type 0x00）。裸建表后 `rtsql check` 立即报两行 error（`free space offset 0 lies outside the data area` + `data page of table 'e' has page type 0x00, expected 0x03`）并 exit 1；首次 INSERT 或 TRUNCATE 后转绿。新库一建就「红」直接削弱 check 卖点自身的可信度；且 `data-integrity-verification` 的干净库测试全部带数据，未覆盖零行表。
2. **聚合只能作顶层 select 项**：`SELECT MAX(id)+1`、`COALESCE(MAX(id),0)` 报 `Unsupported expression type: MAX`；`HAVING MAX(id)+1 > 2` 报 `Unsupported expression type: EXPRESSION`。聚合无法出现在任何值表达式内部。连带后果：SQL 层没有 next-id 方案（只能应用层两步走）。纯聚合 `INSERT INTO t SELECT MAX(id) FROM t` 单列目标可行，但聚合与常量项混排（`SELECT MAX(id), 1`）被「Non-aggregated column '1' must appear in GROUP BY」点名拒绝——字面量被误分类为非聚合列。
3. **UPDATE SET 只收字面量**：`SET v = v + 1`、常量表达式 `SET v = 1 + 2`、`SET v = CAST('42' AS INTEGER)` 全部报 `Unsupported value type`。无法表达「按旧值更新」。
4. **非整数 PRIMARY KEY 静默不强制**：`CREATE TABLE s(name TEXT PRIMARY KEY, ...)` 建表成功，重复插入 `('a',1)`、`('a',2)` 全部成功零警告（keyless row 文档化语义，TEXT 值无键控）。这与项目「约束要么写路径强制要么计划期点名拒绝」的诚实化哲学不一致：声明被接受但永不强制（SQLite 中 TEXT PRIMARY KEY 是查重的，「SQLite NULL-PK 先例」只辩护 NULL 不辩护非空 TEXT）。
5. **恢复错误面误导（遗留 Issue 候选，2026-10-03 用户指令并入）**：lib `Database` 句柄 drop 而不 `close()` 时，未 checkpoint 的 catalog 脏页丢失；重开恢复对「WAL 记录引用 catalog 中不存在的表」以 `WAL redo failed: table 't' lookup failed during redo: table not found: t` 报错——把「DDL 从未持久化（无 checkpoint 关闭）」诊断为「redo 目标表缺失」，排障指向性弱（来源：本 change Iteration 000 Act Response Issue 候选 + Plan Review Finding 4；用户裁定并入本 change 解决）。

## What Changes

2026-10-03 用户批准集中决策（见「已批准的集中决策」）。本文件为需求基线，实现调查完成前 design/tasks/Gate 2 均为草稿。

- **R1 建表首数据页初始化**（域 `data-integrity-verification` ADDED）：全部建表通道（CREATE TABLE / CTAS / import 自动建表 / restore / ATTACH 目标库）SHALL 初始化首数据页为合法 data page（page type 0x03 合法页头），新建即含零行表的库 `rtsql check` SHALL exit 0 单行干净；`check` 对 0x00 页的 error 分级 SHALL 保持（损坏检测不放松）；存量空表按最小修复语义（首次写入/TRUNCATE 自然转绿，open 不自愈）。
- **R2 聚合表达式嵌入**（域 `sql-aggregate-extensions` ADDED）：SELECT 清单值表达式、HAVING 条件表达式、聚合查询 ORDER BY 键 SHALL 可嵌套聚合调用（`SELECT MAX(id)+1`、`COALESCE(MAX(id),0)+1`、`HAVING MAX(id)+1 > 2`、GROUP BY 查询 `ORDER BY MAX(v)`）；不含聚合也不含非聚合列引用的常量项 SHALL 与聚合混排合法（`SELECT MAX(id), 1`，INSERT SELECT 源同语义）；WHERE / JOIN ON 内聚合 SHALL 维持计划期拒绝（聚合在行过滤前无定义，标准 SQL 语义）；聚合嵌套聚合（`MAX(MIN(x))`）SHALL 维持拒绝；非聚合列引用无 GROUP BY 的既有拒绝语义 SHALL 保持。聚合输出引用解析 SHALL 按注册输出正确定位（Iteration 001 Review 补强，S24）：聚合项显示名重名（重复别名或别名撞 `result_column_name`）时，条件包装定位、嵌入表达式引用与 HAVING 引用 SHALL 解析到各自正确的注册聚合输出；同一调用多次出现仍共享一个输出列。
- **R3 UPDATE SET 值表达式**（域 `sql-write-surface` ADDED）：`UPDATE ... SET col = <值表达式>` SHALL 接受完整值表达式右值——列引用（`v = v + 1`，逐行对同一旧行快照求值）、算术、CAST、标量函数、CASE/COALESCE；写入类型一致门（含 FLOAT 列 Int 升格、日期族 coerce）、NOT NULL 强制、键列类型门与 rekey/碰撞预检路径 SHALL 照常生效；SET 右侧聚合 SHALL 拒绝；子查询 SHALL 维持既有拒绝边界；`ON CONFLICT DO UPDATE SET` 三形态与 `REPLACE INTO` 既有语义 SHALL 零变化。
- **R4 非 INTEGER PRIMARY KEY 计划期拒绝**（域 `sql-constraint-enforcement` ADDED）：`CREATE TABLE` 显式声明 PRIMARY KEY 的列 SHALL 声明 INTEGER 类型；其余类型（FLOAT/TEXT/BOOLEAN/DATE/TIMESTAMP）SHALL 计划期点名拒绝、零副作用（对齐 CHECK/FK 诚实化拒绝面）；未显式声明主键的「首列隐式主键」既有引擎语义 SHALL 零变化（INTEGER 首列强制重复、非 INTEGER 首列非键行——文档化行为保持）；存量库含非 INTEGER PK 的表 SHALL 继续可用（打开/读写/删除正常）。
- **R5 dump DDL 渲染适配**（域 `cli-noninteractive-shell` MODIFIED「生命周期子命令 dump 与 restore」）：dump 的 CREATE TABLE DDL 渲染 SHALL 仅在主键列声明类型为 INTEGER 时输出 `PRIMARY KEY`——存量非 INTEGER PK 表（声明从未被强制，输出与语义一致）与隐式 PK 非 INTEGER 首列表的 dump 产物在新拒绝面下 SHALL 可 restore 往返；INTEGER PK 表 dump 输出 SHALL 逐字节不变。
- **R6 恢复重放目标表缺失的错误面诚实**（域 `wal-recovery-replay-integrity` ADDED，2026-10-03 用户指令并入）：恢复重放遇「WAL 已提交记录引用的表在 catalog 中不存在」时 SHALL 点名表名、陈述 catalog 缺失事实与最可能原因（上次关闭未经 checkpoint，如 lib 句柄未 `close()` 即 drop、未持久化 DDL 随脏页丢失），并保持打开失败（K05 显式报错语义不变——SHALL NOT 跳过或静默丢弃不可重放记录）；其他 redo 失败类别（页/索引操作失败）的既有错误文案 SHALL 零变化。

## 场景草图

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| S1 新建空库 check 干净 | `rtsql new` 后裸 `CREATE TABLE`（零行） | `rtsql check` | exit 0，恰一行 `check ok: ...`；文件全字节不变。失败边界（修复前）：两行 error、exit 1 |
| S2 建表通道全覆盖 | CTAS 空源建表 / restore 空表 dump / ATTACH 目标库建表 | 对应 check | 全部通道建出的零行表 check 干净（同一初始化原语） |
| S3 持久两态 | 建表后正常 close | 重开 → check | 仍干净（页初始化持久，恢复重放不破坏） |
| S4 损坏检测不放松 | 注入 0x00 页的损坏库（既有夹具） | check | error 分级与 exit 1 保持（既有 check_command_test 损坏注入用例零回归） |
| S5 存量空表最小修复语义 | lib 直建的 0x00 head 存量夹具表 | 首次 INSERT / TRUNCATE → check | 写入或 TRUNCATE 后转绿（写路径既有初始化行为）；open 零自愈写入 |
| S6 SELECT 嵌套聚合 | 表 t(id,v) 三行 | `SELECT MAX(id)+1 FROM t`；`SELECT COALESCE(MAX(id),0)+1 FROM t` | 单行结果 max+1 / coalesce+1；表头 = 别名优先或表达式 Display。失败边界（修复前）：Unsupported expression type: MAX |
| S7 HAVING 嵌套 | 同上 | `SELECT v, MAX(id) FROM t GROUP BY v HAVING MAX(id)+1 > 2` | 过滤正确，仅保留组内 max+1>2 的组 |
| S8 ORDER BY 聚合表达式 | 同上 | `SELECT v, MAX(id) FROM t GROUP BY v ORDER BY MAX(id)+1`（或 ORDER BY MAX(v) 形态） | 按聚合表达式值排序生效 |
| S9 常量混排合法 | 同上 | `SELECT MAX(id), 1 FROM t`；`INSERT INTO t2 SELECT MAX(id), 1 FROM t` | 常量项合法返回/写入；INSERT SELECT 两列目标可达。失败边界（修复前）：Non-aggregated column '1' 点名 |
| S10 拒绝面保持 | 同上 | WHERE 内聚合 / JOIN ON 内聚合 / `MAX(MIN(x))` / 无 GROUP BY 的非聚合列引用 | 四形态全部计划期点名拒绝（语义与文案点名列） |
| S11 DISTINCT 聚合组合 | 同上 | `SELECT SUM(DISTINCT v)+1 FROM t` | DISTINCT 去重语义在表达式内保持 |
| S12 UPDATE 列引用表达式 | t(id,v) v=10 | `UPDATE t SET v = v + 1 WHERE id = 1` | v=11；仅命中行变更。失败边界（修复前）：Unsupported value type |
| S13 常量/函数/CAST 右值 | 同上 | `SET v = 1+2` / `SET name = upper('ada')` / `SET v = CAST('42' AS INTEGER)` | 全部生效（逐行常量一致） |
| S14 多列同旧行快照 | 同上 | `UPDATE t SET a = a + 1, b = a WHERE ...`（多列混合） | 全部右值基于同一旧行求值（SQL 标准语义），左至右不影响 |
| S15 类型门与约束照常 | FLOAT 列 / NOT NULL 列 / 日期列 | 表达式结果跨类型 / NULL 违反 NOT NULL / Date 列字符串 coerce | 类型门拒绝零副作用；NOT NULL 零副作用拒绝；FLOAT 升格、日期 coerce 生效 |
| S16 键列表达式 rekey | id INTEGER PRIMARY KEY | `UPDATE t SET id = id + 100 WHERE id = 1` | rekey 语义（删旧键插新键）、碰撞预检零副作用、非 Int 结果 KeyTypeMismatch |
| S17 UPDATE 拒绝面 | 同上 | `SET v = MAX(v)` / `SET v = (SELECT ...)`；DO UPDATE SET / REPLACE 既有形态 | 前两者计划期拒绝；后两者语义零回归 |
| S18 非 INT PK 建表拒绝 | 空库 | `CREATE TABLE s(name TEXT PRIMARY KEY, ...)`（FLOAT/BOOLEAN/DATE/TIMESTAMP 同型） | 计划期点名拒绝 exit 3 零副作用；INTEGER PK 全形态（含 INT 别名拼写随调查定）保持 |
| S19 存量表可用 | lib 夹具存量 TEXT PK 表（含行） | 打开 / 读写 / dump | 全部正常；行集不变 |
| S20 dump 渲染适配 | 存量 TEXT PK 表 / 隐式 PK 非 INT 首列表 / INTEGER PK 表 | dump → new → restore 往返 | 前两者 DDL 不含 PRIMARY KEY、往返一致；后者逐字节不变、往返一致 |
| S21 import 通道零变化 | CSV 自动建表（INT 首列 / FLOAT·TEXT 首列） | import + schema + 重复首列 | INT 首列重复 DuplicateKey fail-fast、FLOAT/TEXT 首列重复合法——与今日逐字节一致（不显式声明 PK 的既有语义不被拒绝面波及） |
| S22 恢复错误面指因 | lib 打开 → CREATE TABLE → INSERT（提交）→ 句柄 **drop 不 `close()`**（catalog 脏页丢失、WAL 留有已提交记录） | 重新 open | open 失败，错误点名表名 + 陈述 catalog 缺失与「上次关闭未经 checkpoint」可能原因（失败边界——修复前：`WAL redo failed: table 't' lookup failed during redo: table not found: t`，把 DDL 未持久化误诊为表缺失） |
| S23 正常关闭对照 | 同 S22 序列但走 `close()` | 重新 open + 查询 | open 成功、行可见（证明定向错误面只覆盖无 checkpoint 关闭形态，不误伤正常恢复） |
| S24 聚合显示名重名解析正确 | t(g,a,b) 多组数据 | ① `SELECT MAX(a) AS m, g, MAX(b) AS m FROM t GROUP BY g`（重复别名）；② `SELECT MAX(a) AS max_b, g, MAX(b) FROM t GROUP BY g`（别名撞 result_column_name）；③ 同②形态 `HAVING MAX(b) > 150`；④ 同②形态嵌入 `MAX(b)+1` | 四形态各自解析到正确的注册聚合输出（失败边界——修复前：②③④与①均解析到首个同名注册项，静默取错值/错过滤；探针实测见 Iteration 001 Plan Review）；同一调用多次出现仍共享一个输出列 |

## 已批准的集中决策

2026-10-03 用户四项集中决策（均采纳推荐项）：

1. **非 INTEGER PK 处置 = 计划期拒绝 + 通道兼容**：`CREATE TABLE` 点名拒绝非 INTEGER 显式 PK；import 自动建表不显式声明 PK 的既有语义零改动（调查确认其本就不经显式声明路径）；dump 渲染仅对 INTEGER PK 输出 `PRIMARY KEY`（存量非 INT PK 声明从未被强制，输出与语义一致）。备选否决：仅文档（声明被静默无视的不一致保留）、真实强制唯一（打破 keyless row 设计，改动面大）。
2. **存量空表 = 最小修复**：只改 CREATE TABLE 路径初始化 head 页；存量 0x00 head 表在首次写入/TRUNCATE 时自然转绿（写路径既有行为），open 不自愈。备选否决：打开时扫描自愈（新增打开期写路径，干净打开成本回归）。
3. **聚合嵌套 = 标准 SQL 面**：SELECT 清单值表达式、HAVING 条件、聚合查询 ORDER BY 解锁；WHERE/JOIN ON 维持拒绝（聚合在行过滤前无定义）；嵌套聚合维持拒绝。备选否决：仅 SELECT 清单（语义不完整）。
4. **UPDATE SET = 值表达式全表面**：列引用、算术、CAST、标量函数、CASE/COALESCE；聚合与子查询拒绝（与现有 UPDATE 无子查询边界一致）；类型门/NOT NULL/键列门照常。备选否决：仅常量表达式（无法表达按旧值更新，实用价值有限）。
5. **（2026-10-03 用户指令追加）下一轮 Iteration 展开 + 遗留 Issue 并入**：原话「给出下一轮iter并把遗留issue记入一起解决」——授权（a）完成 Iteration 001 Review 后展开 Iteration 002；（b）将 Iteration 000 Review 遗留的 Issue 候选（drop 不 close 的恢复错误面误导）并入本 change 范围作为 R6/T11 一并解决。伴随 Review 发现的聚合重名解析回归（Iteration 001 变更面自身引入，非范围外缺陷）按同一指令以 T12 并入 Iteration 002 修复，映射 R2（S24）。

## Non-goals

- 非整数键的真实唯一性强制（keyless row 设计保持；TEXT PK 表的既有语义是「无强制」，不是「加强制」）
- 聚合进入 WHERE/JOIN ON、窗口函数、递归 CTE、AUTOINCREMENT/序列
- UPDATE 子查询能力（既有边界保持）
- `check` 对 0x00 页 error 分级的调整或 open 期自愈（损坏检测不放松、最小修复已批准）
- 恢复重放对不可重放记录的跳过、静默丢弃或自动清理（K05 显式报错语义保持；R6 只改错误面指因，不改恢复行为与打开结果；本场景下的库修复/补救指引不属本 change）
- `rtsql schema` 渲染语义变更（隐式 PK 渲染保持）
- R3/R4 之外的写面扩展（DEFAULT 关键字进入 UPDATE SET 等）

## 工作状态与边界

- Gate 1：PASS（2026-10-03 用户批准需求与范围，四项集中决策亲选；同日用户指令追加第 5 项集中决策——展开 Iteration 002 并将遗留 Issue 候选并入为 R6/T11，原话见「已批准的集中决策」）。
- design.md / tasks.md / iterations /：调查完成前均为草稿，不标记 ready。
- 本 change 不更新 SNAPSHOT/tasks（Maintainer 收尾职责）。
