# 主命令在 profiling 开关下不再中止，并输出旧三段计时表

来源：里程碑 MS45（`.agents/docs/tasks.md`，Status ready），承接 ISS42（`.agents/issues/ISS42-cli-main-command-profiling-scope-absent.md`，Status open）。关联约束：M19、M22、M23。R81 指向该缺陷台账。

Gate 1 为 PASS。批准来自本次 goal 的需求批准。本文件不更新里程碑状态，不改 SNAPSHOT，不改缺陷台账，不改 `openspec/specs/` 里的权威正文。

## Why

设置 `RTSQL_PROFILING` 后，主命令 `rtsql <db> "<sql>"` 自己做 parse、plan、execute，不经过会建立 profiling scope 的语句管道。计划走到索引扫描时，执行器按该变量调用 `record_time`，task-local 未设置，进程中止。没有索引扫描的语句则正常结束，但 stderr 没有旧的三段计时表。同一变量下，import 已经会输出这张表。

MS45 把「主命令是否输出这张表」留到需求批准时裁定。本次批准的裁定是：输出，并且与 import 已有的表、以及 `pipeline-stage-decomposition` 里「三段顶层计时观测」对齐。

## What Changes

- `RTSQL_PROFILING` 已设置时，主命令不得再以进程中止结束。含索引扫描的语句把查询结果交回调用方，或者返回该语句本来就会返回的 SQL 错误。
- 同一条件下，进入执行阶段的语句向 stderr 输出旧的 parse / plan / execute 三段计时表。一次调用里有多条这样的语句时，每条各一张表。
- 未设置该变量时，主命令保持现在的结果、错误文本、SQLSTATE、退出码，并且不输出这张表。
- import 路径已有的表、统一诊断门面的观察输出保持。预编译语句入口不在本里程碑的验收里，所选做法不改变它。

无 **BREAKING**。未设置该变量时的 SQL 结果、错误文本和 SQLSTATE 不变。不新增磁盘格式，不新增 CLI 参数。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `pipeline-stage-decomposition`：Requirement「三段顶层计时观测」补上主命令在该变量已设置时的进程结局和旧表输出。域路径 `openspec/specs/pipeline-stage-decomposition/`。该 Requirement 原有场景「profiling 输出三段耗时」保留。`unified-diagnostics` 不改。

## Impact

- 代码：主命令执行分段 `src/cli/mod.rs` 的 `run_sql`。在变量已设置时，为进入执行阶段的语句建立已有的 profiling scope，记下 parse / plan / execute，并调用已有的 `print_timings`。不新增直接写 stderr 的出口（M22：沿用 `src/profiling.rs` 里已有的那张表）。
- 不改：`src/profiling.rs` 的判定、表算法，以及 scope 外 `record_time` 的现有行为；`src/pipeline.rs` 的打印点；`src/database.rs` 的预编译语句入口；import；统一诊断的采集和渲染；未设置该变量时的 SQL 结果、错误文本和 SQLSTATE。
- 测试：新增主命令 CLI 子进程测试。import 上已有的旧表测试保持通过。收尾全量按 M19 分批串行，整个变更只跑一次，放在最后一个 Iteration。
- 关联：MS45；ISS42；M19；M22；M23。

## 需求

下列 1–5 是本次 goal 的需求批准，按原裁定写入。

1. `RTSQL_PROFILING` 已设置时，主命令不得因进程中止结束。含索引扫描的语句交回查询结果，或交回既有 SQL 错误。
2. 同一条件下，主命令输出旧的 parse / plan / execute 三段计时表，与 import 路径已有的表同一可观察输出，并与 `openspec/specs/pipeline-stage-decomposition` 的「三段顶层计时观测」对齐。
3. 未设置该变量时，主命令行为保持，包括不输出该表。
4. import 路径已有的三段表、统一诊断门面的观察语义保持。不改写统一诊断的输出口径。
5. 预编译语句路径不在 MS45 验收内。本次选择的做法只包住主命令，不改变该路径的可观察行为，因此不为它单列任务，也不把它写成新的产品要求。

裁定 2 在本变更里的落地（与 import 当前输出对齐，不另定一套打表规则）：

- 表就是 `print_timings` 打到 stderr 的那一张：表头含 `Time (µs)`，行里有 parse、plan、execute，末行是 `Total`。子指标行仍由已经存在的 `record_time` 写入。
- 只有进入执行阶段的语句才输出。解析失败、规划失败，以及 BEGIN、COMMIT、ROLLBACK、ATTACH 这些不进入执行阶段的语句，不输出。进入执行阶段之后，成功和 SQL 错误都输出。
- 多条语句各一张表。后一张表不带上只有前一条语句才会记下的子指标。

## 不做

- 不把主命令改道到 `Database::execute_sql` / `execute_with_params`。那条路径拒绝多语句，并且会走统一诊断的摘要渲染。
- 不让 `record_time` 在没有 scope 时变成空操作，也不为预编译语句建立 scope。
- 不改 import 的打表时机，不改 `unified-diagnostics` 的 Requirement。
- 不改未设置该变量时的 SQL 结果、错误文本、SQLSTATE 和退出码。变量已设置时，这些字段同样保持，stderr 只多出这张表，进程不再中止。
- 不新增取消 API、profiling 公共 API、CLI 参数或 SQL 语法。
- 不顺带做批量写入或 GC。
- 不新增操作标识、运行标识、内容哈希或其他身份证明机制，不新增判定层、专用 CLI 或审计器。
- 不修改 `openspec/specs/` 的权威正文。规格增量只放在本变更的 delta。
- 不预建「修复尚未发现的缺陷」的 Iteration。

## 场景草图

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Happy | 变量已设置；整数主键表有一行 | 主命令执行该主键的简单等值 `SELECT` | 退出码 0，stdout 有该行；stderr 有一张含 parse、plan、execute、`Total` 的表，并有 `index_manager_search`；无 `diagnostics ` 前缀 | 进程中止，或没有这张表，或没走到索引点查 |
| S2 | Happy | 变量已设置；非主键列上有二级索引 | 主命令执行该列的等值 `SELECT` | 退出码 0，stdout 有该行；表含三段与 `index_manager_search_all` | 进程中止，或走成全表扫描却被当成通过 |
| S3 | Happy | 变量已设置；表有数据 | 主命令执行一条进入执行阶段、计划不含索引点查的语句 | 退出码 0，结果仍在；stderr 仍有三段表 | 只有索引扫描才打表 |
| S4 | 兼容 | 变量未设置；同 S1 的表和语句 | 主命令执行主键等值 `SELECT` | 退出码 0，stdout 有该行；stderr 没有 `Time (µs)` | 未设置变量也打表，或结果变了 |
| S5 | Sad | 变量已设置 | 主命令执行无法解析的文本 | 退出码 3，stderr 是解析错误且无 SQLSTATE；没有这张表 | 解析失败也打表，或补上 SQLSTATE |
| S6 | Sad | 变量已设置 | 主命令查询不存在的表 | 退出码 3，stderr 含 `Table not found` 与 SQLSTATE `42P01`；没有这张表 | 规划失败也打表，或错误文本变了 |
| S7 | Sad | 变量已设置；主键行已存在 | 主命令再次插入同一主键 | 退出码 3，stderr 含 `Execution error: Duplicate key` 且错误对象无 sqlstate；同时有这张表 | 执行期错误被吃掉，或不再打表 |
| S8 | 组合 | 变量已设置；同 S1 的表 | 主命令一次执行 `BEGIN`、主键等值 `SELECT`、`COMMIT` | 退出码 0，stdout 有该行；stderr 恰好一张表，含三段 | 事务内索引扫描仍中止，或 BEGIN/COMMIT 各打一张表 |
| S9 | 组合 | 变量已设置；同 S1 的表 | 主命令先主键等值 `SELECT`，再 `SELECT COUNT(*)` | 退出码 0；两张表都有三段；后一张没有 `index_manager_search` | 两句共用一张表，或子指标串到后一句 |
| S10 | 兼容 | 变量已设置 | `rtsql import` 成功导入 | stderr 仍含 parse、plan、execute；统一诊断默认配置下不出现新的诊断输出口径 | 为了主命令改动 import 或诊断摘要 |

## 回滚

改动在变更分支的 `src/cli/mod.rs` 与新增的 CLI 测试，加上本变更的规格 delta。回滚是还原或放弃该分支。主分支在合并前不被改写。无格式迁移，无新的磁盘字段。放弃后，设置该变量的主命令回到现在的行为：索引扫描会中止进程，其他语句不打这张表。

## 默认假设

下列假设只闭合场景草图里裁定没有逐字写下的输出细节。它们不扩大 goal：不改预编译语句，不改 import，不改统一诊断输出口径，不改未设置变量时的结果。

1. 观察走真实主命令子进程。管道 stdout 下用 `--format json`，避免依赖是否为终端。stderr 上的表用表头 `Time (µs)` 识别，与 `tests/diagnostics_statement_test.rs` 已有判据相同。依据：主命令的退出码、stdout 和 stderr 是调用方实际看到的；库内 `eprintln!` 会被测试框架捕获。
2. 建表、插入、建索引这些准备步骤不设置该变量。被观察的那次主命令才设置或显式去掉变量，避免准备语句的表混进 stderr。依据：要断言的是被测语句自己的 stderr。
3. 整段 SQL 仍只调用一次现有的 `parse_stage`。每张表的 parse 行记这一次解析的耗时，不按语句重新解析。`Total` 的窗口是这次解析的耗时，加上该语句从 plan 开始到 execute 返回的墙钟。百分比算法仍用现有的 `print_timings`。依据：主命令今天就是整段解析一次；重解析会改动错误定位和计划缓存写入。
4. 显式事务里的语句仍调用现有的 `execute_stage_in_tx_routed`，不给它增加 profiling 参数。表的三段由主命令包在这次调用外面记下。索引子指标仍由执行器在已经建立的 scope 里写入。依据：该函数没有 profiling 参数；库预编译语句的事务内执行也调用它，加参数并让库调用方打开，会改变验收外的路径。
5. 重复主键插入代表「进入执行阶段后的既有 SQL 错误」。它的文案是 `Execution error: Duplicate key`，无 SQLSTATE。索引点查在查找成功之后才 `record_time`；查找本身返回错误时，今天就不会因为缺 scope 而中止。S1 与 S2 覆盖会中止的那两条成功路径。依据：`src/executor/index_scan.rs` 与 `src/executor/index_scan_all.rs` 的调用顺序。
6. 预编译语句保持现状，不写入本变更的规格增量。依据：裁定 5，以及所选做法不碰 `PreparedStatement`。

## 后续 change

不创建。主命令的进程结局、旧表和规格 delta 都在本变更内。预编译语句仍不在本里程碑里。

## 实施期缺陷

实施中新发现的实质缺陷，按 M23 在本变更上追加 Iteration 就地承载。超出本 goal 的实质缺陷只作为候选报告，不在本变更里扩范围，也不在本阶段登记。本计划不预建那一轮。

## 未自行批准的缺口

无。打表时机按裁定 2 与 import 的现有规则对齐。预编译语句、统一诊断输出口径、未设置变量时的结果，都落在「不做」里。
