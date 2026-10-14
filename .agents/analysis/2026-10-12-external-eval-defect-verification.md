# 外部评测缺陷核实（2026-10-12 批）

> Snapshot: [SNAPSHOT](../docs/SNAPSHOT.md)
> Captured revision: 44d04d9（工作区干净）
> Captured at: 2026-10-12
> 复现现场：`/tmp/rt-verify/`（本次新建）与 `/tmp/rtsql-eval/db/`（评测方原样现场，含砖化 perfdb）

## 目标与范围

核实外部评测报告的 13 项问题（P0×2 / P1×3 / P2×7 / 待核对×1）在当前 HEAD 是否存在、机制与代码落点。验证方式：`./target/debug/rtsql`（HEAD 构建）行为复现 + 源码读证。全部 13 项**确认存在**（其中 2 项为「确认的行为现状、缺陷定性属设计取舍」，见各条）。

## 已确认事实

### P0-1 索引等值查询错误结果 — 确认，且比报告严重（重复发射 + 条目缺失两机制叠加）

复现（`/tmp/rt-verify/t1.db`，30000 行 n∈1..100，`CREATE INDEX i1 ON t(n)` 后）：

- 基线（DataScan）：`WHERE n=50` → COUNT(*)=281，COUNT(DISTINCT id)=281 ✓
- 建索引后：COUNT(*)=493，COUNT(DISTINCT id)=270 —— 既重复（+212 次发射）又漏（11 行从未发射）；DROP INDEX 恢复 281；重建索引同样错（确定性）。
- 重复放大率随同值键规模增长：8000 行、85% n=2 的表（t2）→ 6805 真值发射 17783 次（2.6×），DISTINCT 恰好完整。
- 唯一键索引不受影响（t1 上 `CREATE INDEX iu ON t(id)`：点查=1、范围 2000 行均正确）。
- 复合谓词（`n=50 AND id<=…`）与范围谓词不路由 IndexScanAll，结果正确——与评测观察一致。

**机制 A（重复发射，已钉死代码行）**：`src/storage/btree/index_manager.rs:214-237` `search_all_from_page_async`——对**每一个**等于查找键的分隔键都收集其左右两个孩子且**无 break**；相邻两个等值分隔键共享的中间孩子被收集两次，递归逐层相乘。查询路径唯一消费面：`src/executor/index_scan_all.rs:84`（`IndexManager::search_all` 异步版）。

**机制 B（条目缺失，现象确证、产生行未钉死）**：漏发的 11 行（t1 中 id=193/207/417/…/1434，全部为最早插入批）的索引条目**不在树里**——证据：`UPDATE t SET n=51 WHERE id=193` 报 `Execution error: Key not found`（delete_exact 找不到旧键条目）。候选来源：`src/executor/create_index.rs:105-183` 回填链尾扫描漏行，或 `src/storage/btree/btree.rs:262-295` 叶分裂/再插入路径丢条目；本调查未用插桩区分（未确认项）。

**顺带发现（超出报告范围）**：上述失败 UPDATE 执行后索引状态发生了变化（COUNT 493→495、DISTINCT 270→271——回滚路径的 abort 重插把缺失条目"修复"了）——失败语句零副作用语义在索引面被违反。

**潜伏变体**：同步版 `src/storage/btree/btree.rs:1070-1131` `search_all_from_page` 带 `break`（只搜首个等值分隔键两侧）→ 漏行变体；当前无生产调用者（仅 `IndexManager::search_all` 异步版被查询消费）。

**既有测试为何未覆盖**：`index_manager.rs` 单测（:625-644）只构造 2-3 条目小树，等值分隔键连串需多叶规模才出现。

### P0-2 崩溃砖化窗口 — 确认（全新因果复现 + 评测现场 perfdb 双重验证）

全新复现（`/tmp/rt-verify/brick3/`）：`rtsql new` → `import --csv`（自动建表 `big`，逐行 auto-commit）→ 5 秒后 SIGKILL（WAL 279KB、未达 16MiB 自动水位、无 checkpoint）→ 之后：

- 读写打开：exit 1，`WAL redo failed: table 'big' referenced by committed WAL records is missing from the catalog; … cannot be opened and no records were skipped`
- `check`：exit 1 `database has pending WAL, open read-write once to recover` ——**提示的恢复路径本身不可达**（读写打开正是失败的那个操作），死锁闭环。
- `schema`：同 exit 1。

评测现场 `/tmp/rtsql-eval/db/perfdb`（WAL 85KB、无 .checkpoint）在副本上复现同一行为。

**机制**：DDL 不写 WAL（MS18 设计事实——"DDL 不写 WAL 故脏页条件必需"）；catalog 页仅在 checkpoint/close/缓冲池驱逐时落盘；恢复重放对 catalog 缺表硬失败不跳过（K05；`redo_table_lookup_error` 四要素文案是 practical-core-gaps T11 引入的诚实化，把该形态从含混报错变成定向文案，但可达性结局未变）。

**窗口边界（对照实验）**：
- 表已存在的库同型崩溃恢复正常（200000 行 import 中途 SIGKILL → 恢复打开、count=6669，仅 stderr 一行 `[recovery] old index page … skipping subtree` 容忍性告警）。
- 大数据量触发缓冲池驱逐、catalog 脏页被自然刷盘 → 不砖化（同上对照）。
- 小数据量、catalog 页留在池内、WAL 已有已提交行、未 checkpoint → 砖化。`RTSQL_WAL_CHECKPOINT_BYTES` 调小可缩小窗口（评测方结论一致）。

### P1-3 派生表列别名/计算列外层不可见 — 确认，且范围比报告大（视图同样中招）

复现（`/tmp/rt-verify/p1/d.db`，people 3 行）：

| 形态 | 结果 |
|---|---|
| `SELECT age FROM (SELECT age FROM people) t` | ✓ 正常 |
| `SELECT v FROM (SELECT age AS v FROM people) t` | ✗ 42703 `Column not found: v` |
| `SELECT t.tier FROM (SELECT CASE … END AS tier FROM people) t` | ✗ 58000 `Invalid aggregate argument: Expected column name` |
| `SELECT tier FROM (…同上…)` | ✗ 同上 |
| 内层两条查询独立执行 | ✓ 均正常（引擎支持表达式项+别名） |
| `WITH x AS (SELECT age AS v FROM people) SELECT v FROM x` | ✓ **CTE 通道正常** |
| `CREATE VIEW vw AS SELECT age AS v …; SELECT v FROM vw` | ✗ **42703 —— 视图通道同样中招** |

**机制**：FROM 派生臂（`src/parser/planner/query.rs:409-447`）置 `building_subquery=true`；该上下文对内层 SELECT 的表达式项/别名顶层 Projection 路由的抑制（MS11-T01 引入，为 IN/EXISTS 标量子查询设计）一并生效——`change 2026-10-04-external-review-gaps` T5 的 `building_from_derived` 只放宽了集合操作守卫，未解除该抑制。58000 错误来自 `expr_to_column_name` 对非列表达式抛 `InvalidAggregateArgument("Expected column name")`（`src/parser/planner/expression.rs:875`，`ast.rs:392` 同型）——纯 SELECT 查询收到聚合错误文案。视图展开走同一 DerivedScan/building_subquery 通道，与语料「CTE 通道展开与内联派生表逐字节一致」声明矛盾（语料-代码冲突记入未确认项供修复 change 处理）。

### P1-4 CREATE TABLE 失败残留半表 — 确认

复现（`/tmp/rt-verify/p4/e.db`）：`CREATE TABLE t (id INTEGER, pad TEXT DEFAULT '<5000字符>')` → `Execution error: Page full`（rc 3）。失败后：

- `schema` 输出 `CREATE TABLE "t" ("id" INT PRIMARY KEY);` ——半表已持久化（首列行写入成功、第二列行页满失败）。
- `INSERT INTO t (id) VALUES (1)` 成功（affected_rows:1）——半表可写。
- `CREATE TABLE t (id INTEGER)` → `table already exists`（rc 3）——无法重建。

DDL 无原子性：CreateTableExecutor 逐条写 catalog 表行/列行，中途失败不回滚已写行。与文档「拒绝时零副作用」语义冲突。

### P1-5 IN 子查询 × ORDER BY 非输出列 — 确认

复现矩阵（people/dept）：裸 `SELECT name FROM people ORDER BY id` ✓；`+ WHERE id IN (SELECT did FROM dept)` → ✗ 42703 `Column not found: id`；`ORDER BY name`（输出列）✓；去掉 ORDER BY ✓；`WHERE id IN (1,3)`（字面量）+ ORDER BY id ✓。

**机制**：SemiJoin 节点的 `output_columns` 直接按 SELECT 投影列装配（`src/parser/planner/subquery.rs:63-64` `build_output_columns_for_table(table_name, projection_columns)`），不像其他基座「全宽行 + Sort/Projection 负责裁剪」（design D10 不变量）；排序键计划期校验（`src/parser/planner/query.rs:2703-2730`）以 `get_plan_output_columns(SemiJoin)` = 投影列集为可解析集 → `id` 拒绝。字面量 IN 走谓词下推路径、行保持全宽故不受影响。

### P2 批（全部确认）

| # | 声明 | 核实 | 落点 |
|---|---|---|---|
| P2-6 | help 泄漏内部任务标记 | `--busy-timeout` help 含「MS28 T1：」、`--readonly` help 含「MS26 T1：」「proposal 假设 10」，均用户可见（clap doc comment） | `src/cli/mod.rs:107,112-114` |
| P2-7 | 错误前缀嵌套重复 | `Parse error: Parse error: …` 实测；双层包装：pipeline 包一次 + PlanError Display 再包一次；「Plan error: Parse error:」为嵌套形态 | `src/pipeline.rs:113` + `src/parser/error.rs:74` |
| P2-8 | DDL 渲染非标准类型名 | schema 与 dump 均输出 `INT`/`STRING`/`BOOL`（声明 INTEGER/TEXT）；往返仍可达（解析器接受短名），属保真度问题 | `src/cli/lifecycle.rs:1025-1034` `column_type_sql` |
| P2-9 | BUSY_TIMEOUT 非法值只提 flag | `RTSQL_BUSY_TIMEOUT=abc` → `invalid value 'abc' for '--busy-timeout <MS>'`，未提 env 名 | clap env 校验路径 |
| P2-10 | 多语句 JSON 非单一文档 | 两条语句输出两个独立 JSON 对象（JSONL）。确认为现状行为；定性属设计取舍（自动化须按 JSONL 解析） | CLI 多语句顺序渲染设计 |
| P2-11 | DDL 输出 affected_rows: 0 | 实测 CREATE TABLE/INDEX 均输出 `{"affected_rows":0}`。确认为现状行为；定性属设计取舍 | CLI 渲染通道 |
| P2-12 | 超大 DEFAULT 报 Page full | 实测报 `Page full` 不指名默认值过大（文档已承认该措辞）；与 P1-4 叠加成残留半表 | catalog 列行序列化页满路径 |

### 待核对项：加密库 .wal/.checkpoint 明文 — 证实为真

加密库（`new --key` 创建）import 中途 SIGKILL 后：主文件 0 处明文标记、`.wal` 内 **2081 处**明文行数据标记。与语料 `database-encryption` 声明一致（WAL/checkpoint 保持明文为文档化设计），本次取得脏窗口实证样本：崩溃窗口期行数据以明文滞留磁盘。

## 未确认项

1. **P0-1 机制 B 的产生行**：回填漏行 vs 插入/分裂丢条目，未插桩区分——Plan 阶段需定向调查；修复验收必须同时覆盖「结果集与 DataScan 严格相等」（重复与缺失两个方向）。
2. **P1-3 语料-代码冲突的处置**：`sql-views` R3「CTE 通道展开与内联派生表逐字节一致」与实测（CTE ✓ / 内联派生 ✗ / 视图 ✗）矛盾——修复 change 需同步修订或以实现收敛。

## 边界与失败路径（要点）

- P0-1 触发条件：非唯一二级索引 + 等值谓词路由（IndexScanAll）+ 重复键值形成多叶连串；树高 ≥2 即可重复（t2），规模越大放大率越高。范围/复合谓词、唯一索引不受影响。
- P0-2 触发条件：DDL 后 catalog 页仅脏在池内 + WAL 已有该表已提交行 + 未 checkpoint/close/驱逐刷盘即崩溃。恢复出口不存在（打开拒绝且 check 指引不可达）。
- P1-3 触发条件：FROM 派生表/视图 + 内层别名项或表达式项被外层引用；裸列名不受影响。
- P1-5 触发条件：IN/EXISTS 子查询（SemiJoin/AntiJoin 路径）+ ORDER BY 引用非输出列。
- 失败 UPDATE 的索引面副作用（P0-1 顺带发现）：缺失条目存在时 `UPDATE … SET n=常量 WHERE id=…` 报 Key not found 且 abort 重插改变索引内容。

## 测试、验证入口与影响面

- 验证二进制：`./target/debug/rtsql` @ 44d04d9（2026-10-12 构建）；评测方现场 `/tmp/rtsql-eval` 由已安装 0.2.0 产生，行为与 HEAD 构建一致（perfdb 副本双验证）。
- 关键复现命令（节选，全部在 /tmp/rt-verify 可重放）：
  - P0-1：批量 INSERT 30000 行 → `CREATE INDEX i1 ON t(n)` → `SELECT COUNT(*) / COUNT(DISTINCT id) FROM t WHERE n=50`；缺失探针 `UPDATE t SET n=51 WHERE id=<缺失id>`。
  - P0-2：`rtsql new` → `import --csv <db> big small.csv`（10000 行）→ 5s 后 `kill -9` → 任意开库命令。
  - P1-3：见上表七形态。
  - P1-4：`CREATE TABLE t (id INTEGER, pad TEXT DEFAULT '<5000×x>')` → `schema` → `INSERT` → 重建。
- 既有测试缺口：secondary_index_test 无多叶等值连串规模用例；view_test/derived_scan_alias_test 无「外层引用内层别名/表达式项」用例；崩溃恢复测试无「建表后未 checkpoint」窗口用例；cli help 文本无快照测试。
- 影响面：`src/storage/btree/{index_manager,btree}.rs`（P0-1）、WAL/恢复策略与 catalog 落盘时序（P0-2，涉及设计决策）、`src/parser/planner/query.rs` 派生臂与 `building_subquery` 抑制面（P1-3）、CreateTableExecutor/catalog 写序（P1-4）、`src/parser/planner/subquery.rs` SemiJoin 形状（P1-5）、`src/cli/mod.rs` 文案（P2-6/9）、`src/pipeline.rs`+`src/parser/error.rs`（P2-7）、`src/cli/lifecycle.rs`（P2-8）。

## 关键文件

- `src/storage/btree/index_manager.rs:152-257`（异步 search_all，重复发射）
- `src/storage/btree/btree.rs:1070-1131`（同步 search_all，漏行变体，无生产调用者）
- `src/executor/index_scan_all.rs`（唯一查询消费面）
- `src/executor/create_index.rs:105-183`（回填，机制 B 候选之一）
- `src/wal/recovery.rs`（redo_table_lookup_error / 砖化出口）
- `src/parser/planner/query.rs:409-447`（FROM 派生臂 building_subquery 抑制）
- `src/parser/planner/expression.rs:868-880`（expr_to_column_name / 58000 文案）
- `src/parser/planner/subquery.rs:40-138`（SemiJoin/AntiJoin output_columns 装配）
- `src/parser/planner/query.rs:2703-2760`（排序键计划期校验）
- `src/cli/mod.rs:107-114`（help 内部标记）
- `src/pipeline.rs:110-113` + `src/parser/error.rs:74`（前缀双层）
- `src/cli/lifecycle.rs:1025-1034`（类型名渲染）

## Issue 候选（仅列出，未登记；登记归 experience-recorder 按用户指令）

P0-1（重复发射 + 条目缺失，正确性红线）、P0-2（崩溃砖化窗口，可用性红线）、P1-3（派生表/视图别名与计算列外层不可见 + 语料矛盾）、P1-4（DDL 失败残留半表）、P1-5（SemiJoin 排序键形状）、P2-6/7/8/9（文案与错误面小批）、P2-12（DEFAULT 页满文案）；P2-10/11 为行为定性项，是否立项由用户裁定。
