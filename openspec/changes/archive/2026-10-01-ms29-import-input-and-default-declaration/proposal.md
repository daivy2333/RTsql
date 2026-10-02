# ms29-import-input-and-default-declaration — Proposal

## Why

- **输入侧只有出口没有入口（I078，2026-10-01 外部评审 #7）**：`import` 是生命周期子命令（`:memory:` 拒绝 exit 2）、目标表必须已存在且表头精确匹配、不收 stdin（`restore` 有 `-` 通道而 import 无）——「echo CSV | rtsql :memory: import -」类 Unix 组合结构性不可能，一次性分析被迫 new + import + delete 三步污染磁盘。输出侧 `--format json/csv/tsv` 管道友好而输入侧结构性残缺。
- **声明/写入分裂残留（MS24-T04，2026-10-01 重排并入 MS29-T02）**：MS24 Iteration 000 Review 发现声明面零校验——Bool 列 `DEFAULT 1` 建表被接受（任意字面量变体持久化），应用默认值的 INSERT 必然被写入类型门点名拒绝。「建表成功但应用默认值必然失败」分裂形态存在。
- **调查实证缺陷（本 change 调查发现，2026-10-01）**：日期族 DEFAULT 的 dump→restore 往返已坏——`CREATE TABLE ev (d DATE DEFAULT '2026-01-02')` 建表成功，dump 渲染 `DEFAULT DATE '2026-01-02'`（`typed_datetime_literal`），restore 解析 TypedString 时 `extract_default_value` 无对应臂报 `Unsupported value type` exit 3（CLI 实证，`sql-write-surface` R1「dump→restore 往返保真 DEFAULT」场景对日期族实际不成立；存量测试仅覆盖 INT/STRING 默认值）。`ALTER TABLE ADD COLUMN` 的 DEFAULT 声明（`ddl_dml.rs` 同一提取面）存在同款分裂。用户裁定两项均并入本 change（2026-10-01）。
- 用户 2026-10-01 批准 MS29 重排新建（输入面与写面声明收口；roadmap 统一执行序 MS28 之后第一项）。

## What Changes

两任务面、两 Iteration 承载：

- **Iteration 000 — T01-T03 import 输入面**（MODIFIED `cli-noninteractive-shell`「生命周期子命令 import --csv」）：
  - **T1 stdin 通道**：`import` 的文件位接受 `-`（读 stdin，对齐 `restore` 的 `read_restore_input` 先例）；RFC4180 解析与逐条 auto-commit 语义不变。
  - **T2 免建表导入**：目标表不存在时按 CSV 表头自动建表——列名与列序取表头；列类型按数据行逐字段猜测（i64→INT、f64→FLOAT、其余一律 STRING 参与推导，用户 2026-10-01 裁定不做 Bool/日期猜测），经 CTAS 推导矩阵合并（同型保留、INT∪FLOAT→FLOAT、NULL 参与任意类型、全 NULL/零数据行列落 STRING，复用 `infer_ctas_column_types` 先例——可见性扩为 `pub(crate)`）；列内类型冲突点名列名与冲突类型拒绝（用户裁定 CTAS 矩阵同款 fail-fast，不做 STRING 宽化）；自动建表**不显式声明主键**（replan 修订：沿用引擎「未声明 PK → 首列隐式主键」语义，首列 INT 重复值 DuplicateKey fail-fast、FLOAT/STRING 首列重复合法，见 Replan 裁定）；推导在收齐数据行后、建表前完成，任何推导失败或建表失败零副作用（目标表不存在于失败结果中）；自动建表经常规 `execute_sql` 建表通道执行（planner 校验/名字归一化/视图重名守卫自然生效）。既有表导入语义（表头匹配、类型转换、fail-fast、输出行数）零变化。
  - **T8 planner 重复列名守卫（replan 增入）**：`build_create_table` 计划期拒绝重复列名（含归一后同名）——根因修复「重复列名表静默建成且不可 dump 往返」既有缺陷，自动建表重复表头场景随之 exit 3。
  - **T3 `:memory:` 豁免面**：`rtsql :memory: import <table> <file|-> --csv` 可达（内存库、进程退出即弃）；`--key` 组合 exit 2 拒绝（内存库永不加密，对齐主命令语义）；`--readonly` 维持既有全局拒绝。文件库目标缺失维持 exit 1 拒绝——import 不创建数据库文件，创建通道收敛 `rtsql new`（既有语义协调边界）。
- **Iteration 001 — T04-T06 DEFAULT 声明面 + T07 文档**（ADDED `sql-write-surface`「声明期 DEFAULT 类型一致」）：
  - **T04 CREATE TABLE 声明期校验/归一**：规则与写入类型门对齐——NULL 豁免；FLOAT 列整数字面量无损升格归一持久化；日期族列 String 字面量经既有强制解析通道验证（解析失败建表点名拒绝零副作用）；其余跨类型（BOOL 列 DEFAULT 1、INT 列 DEFAULT 'x'、STRING 列 DEFAULT 1、INT 列 DEFAULT 1.5 等）计划期点名拒绝。落点 `build_create_table`（列类型与 DefaultValue 交汇单点）。
  - **T05 TypedString 臂与往返修复**：`extract_default_value` 补日期族 TypedString（`DATE '…'` / `TIMESTAMP '…'`）解析臂——解析验证后落既有 String 存储形态（dump 渲染不变、写入门 coerce 通路不变）；dump 生成的 `DEFAULT DATE/TIMESTAMP '…'` 文本 restore 可重建（实证缺陷闭环）；非日期族 TypedString 维持既有拒绝。
  - **T06 ALTER ADD COLUMN 同款规则**：`build_alter_table` AddColumn 的 DEFAULT 声明适用同款校验/归一（同文件同规则面，用户裁定并入）。
  - **T07 文档同步**：`rtsql-docs/SKILL.md` 命令行功能目录（import stdin/自动建表/`:memory:` 豁免、DEFAULT 声明语义）+ README 双语对应段；`openspec validate`。

## 用户裁定（2026-10-01 集中决策，Gate 1 输入）

1. **自动建表类型冲突 = 点名拒绝**（CTAS 矩阵同款 fail-fast；不宽化 STRING）。
2. ~~**自动建表不声明主键**（首列重复值合法；与 CTAS 首列隐式主键刻意不同）。~~ **2026-10-01 阻塞审计推翻（见下方 Replan 裁定）**：原裁定基于 Plan 错误调查事实——引擎不存在表级无主键概念（`CreateTableExecutor` 对未声明 PK 默认取首列，dump DDL 无法表达无主键），修订为**沿用引擎「首列隐式主键」语义**。
3. **T02 范围 = 两项都并入**（DATE DEFAULT 往返修复 + ALTER ADD COLUMN 同款校验，与 CREATE TABLE 声明期校验同轮交付）。
4. **CSV 逐字段猜测面 = 仅 INT/FLOAT/STRING**（true/false、日期文本均落 STRING 参与推导；不猜 Bool/日期）。

## Replan 裁定（2026-10-01 阻塞审计）

Iteration 000 Act 于 T2 两场景命中 Gate 5 阻塞（Blocker Handoff 在案）；Plan Review 独立核实为 **Plan 错误 ×2**（非 Act 偏离）：①「普通建表无 PK 合法」调查止于 planner 层，漏查 `CreateTableExecutor`（`src/executor/create_table.rs:74-83`）对未声明 PK 默认取首列；②「重复列名守卫自然生效」系把 CTAS 输出列重复检查错误泛化到普通建表通道（planner/TableManager 均无守卫，重复列名表被静默建成且 dump→restore 必触发 `MultiplePrimaryKey`——既有实质缺陷）。用户指令「工作阻塞，进行审计，给出解决」授权处置，集中决策未获即时应答，**按审计推荐执行、可推翻**：

1. **自动建表主键 = 沿用引擎「未声明 PK → 首列隐式主键」语义**（与 `create-table-as-select` 同款；引擎零改动；`rtsql schema` 渲染首列 `PRIMARY KEY`；首列 INT 重复值 DuplicateKey fail-fast——部分导入保留，首列 FLOAT/STRING 重复合法非键行）。绕道建无主键表结构性不可行（dump DDL 无法表达，restore 重默认首列必坏往返）。
2. **重复列名 = planner 建表通道补计划期守卫（新任务 T8，Iteration 000）**——本 change 内根因修复，Act Issue 候选随之闭环（无需落账）；自动建表重复表头经 `execute_sql` 走 exit 3 既有通道。

原 T2 阻塞两场景在修订语义下由 `001-replan.md` 校准收口；T1/T3 完成面与验证结论采信继承。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **stdin 语法 = 文件位 `-`**：`rtsql import <db> <table> - --csv`（对齐 restore 实参位）；不新增 flag。
2. **import 永不创建数据库文件**：文件库缺失维持既有 `does not exist` exit 1（`resolve_db_path` 后存在性校验不变）；`:memory:` 是唯一免建库形态（无文件）。
3. **自动建表仅触发表不存在时**：既有表导入路径（含 `--key` 加密库、锁冲突 exit 4、`--format` 四态输出）零变化。
4. **推导数据源 = 全部数据行**：空字段按 NULL 参与推导（不制造类型冲突）；与既有插入语义的空字段差异（推断时 NULL、String 列插入时空串）随实现文档化——推断后列类型为 STRING 的列，空字段仍按既有语义插入空串。
5. **推导冲突/建表失败的错误分类**：推导冲突在 CLI 侧（收齐数据后、DDL 执行前）判定 → 既有转换失败同类的 General exit 1，文案点名列名与冲突类型对；自动建表 DDL 经 `execute_sql` 失败（保留名/重名视图/重复列名等）→ 既有 SQL 失败通道 exit 3；两者均零副作用。
6. **零数据行（仅表头）CSV**：自动建表成功（全列落 STRING），导入 0 行 exit 0；空输入（无表头）维持既有解析错误拒绝。
7. **声明期归一形态**：FLOAT 列 Int 默认值归一为 Float 持久化（dump 渲染跟随）；日期族默认值维持 String 目录形态（解析验证，不经类型变换）——catalog 读回、写入门 coerce、dump 渲染三通路均零改动。
8. **声明期拒绝错误面**：计划期错误（新 `PlanError` 变体或既有变体扩文案，Phase 2 定稿），点名列名/期望类型/实际值类型；SQLSTATE 归类随 `error-reporting` 既有分类。
9. **迭代序 = 000 → 001**：T04-T06 与 import 面无代码耦合（不同故障域）；T07 文档消费全 change 语义，随 001 收尾。
10. **文档面**：SKILL.md 当前为 MS28 收尾后已提交状态；本 change 在其上继续编辑。

## 范围外（Non-goals）

- import 攒批提交 / 持锁窗口治理（→ MS22 候选池，I077-②）。
- CSV 之外的输入格式（JSON 等）；export 通道变更；列改名/改类型等 schema 演进扩展（MS29 Non-goals 既定）。
- 自动建表的约束声明（NOT NULL/UNIQUE/DEFAULT——CSV 无约束信息；需要约束走既有建表路径）。
- Bool/Date 的 CSV 类型猜测（用户裁定排除）。
- `:memory:` 其他生命周期子命令豁免（仅 import；stats/sample/profile 沿既有 `resolve_existing_db` 拒绝面）。
- 写入类型门（`sql-write-surface` R2）运行期语义变更——本 change 只加声明期校验，门本身不动。
- dump 渲染格式变更（`typed_datetime_literal` 形态保持）。

## Impact

- **受影响代码面**（Phase 2 复核）：`src/cli/lifecycle.rs`（`import_csv` stdin 读取 + 自动建表推导与 DDL 生成 + `:memory:` 路由）、`src/cli/mod.rs`（`lifecycle_db_target` import 豁免 + dispatch 臂 + `:memory:`×`--key` 守卫）、`src/pipeline.rs`（`infer_ctas_column_types` 可见性 `pub(crate)`）、`src/parser/planner/ddl_dml.rs`（`extract_default_value` TypedString 臂 + `build_create_table` 声明门 + `build_alter_table` AddColumn 门 + 可能的 `PlanError` 变体）、`rtsql-docs/SKILL.md`、`README.md`/`README.zh-CN.md`。
- **受影响 specs**：MODIFIED `cli-noninteractive-shell`（「生命周期子命令 import --csv」扩 stdin/自动建表/`:memory:` 豁免，原「目标表或库不存在拒绝」场景按新语义校准）；ADDED `sql-write-surface`（声明期 DEFAULT 类型一致——含 TypedString 往返与 ALTER 面；**+ CREATE TABLE 重复列名计划期拒绝**〔replan 增入〕）。语料域数不变（61）。
- **测试**：新增 import stdin/自动建表/`:memory:` 矩阵 + 声明期 DEFAULT 拒绝/归一矩阵 + 日期族 DEFAULT 往返修复见证（修复前 RED 已实证在案）；存量 `test_import_missing_table_or_db` 表缺失臂按自动建表语义校准（断言意图由新用例承接）；全量零回归（change 创建时基线 1657 tests / 0 failures / 2 ignored——MS28 收尾后全量，2026-10-01 Iteration 001 Act Response 分批 M19）。

## 验证边界

- **T01-T03**：import 矩阵 e2e——stdin 管道导入既有表与自动建表、类型推导矩阵（INT/FLOAT/STRING、Int∪Float 提升、全 NULL 列、零数据行）、冲突点名拒绝零副作用、自动建表无主键（首列重复值）、`:memory:` 组合可达与 `--key` exit 2、文件库缺失 exit 1 零文件创建、既有表导入全矩阵零回归。修复前以「stdin 不支持 / 自动建表缺失 / `:memory:` 拒绝」用例见证 RED。
- **T04-T06**：声明期矩阵（跨类型拒绝逐形态、FLOAT 升格归一、日期族解析通过/失败、NULL 豁免、TypedString 声明、ALTER ADD COLUMN 面）+ 日期族 DEFAULT dump→restore 往返修复（修复前 RED = 本 proposal 实证序列）+ 既有 DEFAULT 场景零回归 + 全量零回归。
- **T07**：SKILL.md/README 双语与实现逐条一致（人工核对 + `rtsql --help`/`rtsql import --help` 抽样自动执行）。
