# Iteration 000 / Cycle 000: 旧表输出时机收敛与解析阶段断言

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1、T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 权威域 `unified-diagnostics` 的 Requirement「既有计时出口和 stderr 告警保持」由本 change 的 delta 改写；Requirement「成功、失败和取消都结束记录」的「含有解析阶段」子句由本 Cycle 的断言锁定。`RTSQL_PROFILING` 的默认关闭语义、旧表的列与算法、诊断（新摘要）行为、既有 stderr 告警原文、SQL 结果、错误文本、SQLSTATE、取消后的行可见性全部继承为不变量。
- Excluded scope: T3（收尾全量，Iteration 001，只保留在 `tasks.md`）；ISS42（`RTSQL_PROFILING` 对 CLI 主命令无效、scope 外 panic，待用户裁定，不承载）；已归档的 MS44 产物（归档载体不可变）；`openspec/specs/` 权威正文（由本 change 的 delta 承载，收尾时合并）。

**Objective**

旧的三段计时表只在走到执行阶段的返回路径输出，与 MS44 变更前的基点 `e69c7e2` 逐条返回路径对齐：执行阶段之前的失败路径不输出该表，进入执行阶段后（无论成功还是失败）仍输出该表；表头、列、行、`Total` 行算法、计时窗口与 `record_time` 的调用面不变；错误文本与 SQLSTATE 不变；诊断行为不变。同时把权威规格场景「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句用断言锁定。

**Investigation Facts**

- Starting point：
  - change 分支 `changes/2026-10-30-diagnostics-legacy-table-error-path`，基点 master，HEAD `6412caf`；工作区干净。
  - 权威规格 `openspec/specs/unified-diagnostics/spec.md`：9 个 Requirement / 22 个 Scenario。其中 Requirement「既有计时出口和 stderr 告警保持」写有「经语句管道执行的语句在该变量已设置时，每条返回路径都会输出这张表，含解析、规划等失败路径」与场景「失败路径也输出旧的三段计时表」。
  - 该段文字与场景由 MS44 收尾 Review（`openspec/changes/archive/2026-10-28-ms44-unified-diagnostics/iterations/003-final-regression/000-initial.md`）记录为非阻塞 Minor finding：需求 8 原意是「保持」既有计时出口，MS44 只把 delta 文字对齐到实现，没有裁定该行为。Review 的可选处置 (b) 正是本 change 要做的「收敛为错误路径不打印旧表」。
  - 已归档的 MS44 change 全部四个 Iteration 的 Review Result 为 `accepted`；其收尾全量结论（15 批 / 164 个测试二进制 / 2407 passed / 0 failed）可沿用为变更前基线，本 Cycle 不重跑全量。
- Current-State Evidence：
  - **代码事实（当前实现，HEAD `6412caf`）**：
    - `src/pipeline.rs:1425-1480` `execute_inner`：`profiling = is_profiling_enabled()`；`profiling` 为真时创建 `total_start`（在缓存查找之前）；调用 `execute_staged(...)`；随后 `src/pipeline.rs:1456-1458` 无条件执行 `if let Some(start) = total_start { print_timings(start.elapsed()); }`。**该打印不区分返回路径**——这是本 change 要改的点。
    - `src/pipeline.rs:1487-1652` `execute_staged`（MS44 Iteration 001 从 `execute_inner` 主体拆出）：缓存查找 → 命中路径 `execute_stage` 后返回（`1519-1535`）；未命中路径依次经过 parse（`1550`）、多语句拒绝（`1569-1571`）、无语句（`1572-1580`）、占位符编号（`1584-1593`）、参数个数不匹配（`1595-1597`）、plan（`1610`）、execute（`1643`）。前六处都是早退。
    - `record_time` 的调用面：`cache_hit_check`（`1510`）、`parse`（`1554`）、`plan`（`1614`）、`execute`（`1647`），另有 `src/pipeline.rs` 内 `table_metadata_lookup` / `executor_creation` / `executor_execution` 与 `src/executor/index_scan.rs:74`、`src/executor/index_scan_all.rs:90` 两处索引子指标。`print_timings` 全仓只有一个调用点：`src/pipeline.rs:1457`。
    - `src/profiling.rs:33-52` `print_timings`：表头 `Stage | Time (µs) | % Total`，各行按耗时降序，末行 `Total`（取调用方传入的外层耗时，恒 100.0%）。`src/profiling.rs:55-57` `is_profiling_enabled` 只判断 `RTSQL_PROFILING` 是否存在。`src/profiling.rs:21-25` `record_time` 在旧 scope 未设置时 panic（ISS42，不承载）。
  - **基线事实（`git show e69c7e2:src/pipeline.rs`，MS44 变更前的 `execute_inner` 单函数，1411-1552 行）**：`print_timings` 只有两处——缓存命中路径 `execute_stage` 返回后（`1448-1450`）与未命中路径 `record_time("execute")` 之后（`1548-1550`）；两处都不区分执行成功还是失败。未命中路径的六个早退（parse 失败 `1462-1473`、多语句 `1479-1480`、无语句 `1482-1491`、占位符 `1493-1503`、参数个数 `1505-1507`、plan 失败 `1516-1526`）全部在任何 `print_timings` 之前返回。基线的 parse 失败分支在 `return` 之前仍 `record_time("parse", ...)`（`1465-1467`）——记了时间，只是不打表。
  - **运行观察（2026-10-30，Linux x86_64）**：CURRENT = change worktree 的 `target/debug/rtsql`（HEAD `6412caf`）；BASELINE = `/tmp` 下 `e69c7e2` 克隆构建的同名二进制。三种场景逐条实测：

    | 场景 | CURRENT | BASELINE |
    |---|---|---|
    | `rtsql import` 导入重复列名 CSV（表头 `id,id`）→ 自动建表 DDL 规划期失败 | exit 3，stderr 含 1 张旧表（parse / plan / cache_hit_check + `Total`） | exit 3，stderr **无旧表** |
    | 预置同键行后 `rtsql import` 导入单行同键 CSV → INSERT 执行期失败 | exit 3，stderr 恰好 1 张旧表 | exit 3，stderr 恰好 1 张旧表 |
    | `rtsql import` 成功导入 1 行 | exit 0，stderr 2 张旧表 | exit 0，stderr 2 张旧表 |

    三种场景下 CURRENT 与 BASELINE 的错误文本与退出码逐字节一致；差别只在第一行——执行阶段之前的失败路径现在多打了一张表。
  - **失败路径的可观察面**：`rtsql import`（`src/cli/lifecycle.rs:681`）是 CLI 侧唯一经 `Database::execute_sql` 进入语句管道的路径——自动建表 DDL（`lifecycle.rs:840`）与逐行 INSERT（`lifecycle.rs:891`）都走 `db.execute_sql` → `pipeline::execute` → `execute_with_params` → `execute_inner`。规划期失败面是 `src/parser/planner/ddl_dml.rs:1194-1201` 的「重复列名计划期拒绝」；执行期失败面是键冲突（`Duplicate key`）。
  - **Rust 测试框架的捕获限制**（决定判定面的事实）：`eprintln!` 会被 libtest 捕获，且 Rust 1.99 的 `std::thread::add_spawn_hook` 让新线程继承该捕获（`library/test/src/lib.rs` 的 `run_tests_console`），因此在 `cargo test --test <file> -- --test-threads=1` 下库 API 入口的旧表 stderr 在进程内不可读回；只有子进程的 stderr 可读回。
  - **T2 前提**（实测，scratch 程序依赖本 worktree 的 rtsql）：执行 `"SELECT FROM WHERE"` → 错误文本 `Parse error: sql parser error: Expected an expression:, found: EOF`，SQLSTATE `Some("42601")`；打开 stats 采集后摘要 severity 为 Error，阶段表有且仅有 `(Pipeline, "parse")`（count = 1）。权威规格场景「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句在当前实现下**已成立**，只是没有断言锁定。
  - **既有测试与夹具**：`tests/diagnostics_statement_test.rs` 有 `#![cfg(feature = "cli")]`；`spawn_cli(dir, args, profiling: Option<&str>)`（`325-344`，设 `RTSQL_HOME`、移除 `RTSQL_KEY` / `RTSQL_BUSY_TIMEOUT`、按需设置或移除 `RTSQL_PROFILING`）、`wait_cli(child) -> CliOutput { code, stdout, stderr }`（`347-381`）、阶段断言辅助 `stage_stat(summary, module, name)`（`37-43`）。既有 `legacy_profiling_table_follows_env_var`（`387-459`）已用 `Time (µs)` 区分旧表、已用 `diagnostics ` 前缀区分新诊断输出。该文件按既有约定以 `--test-threads=1` 串行执行。
- Code and Critical Path：
  - `src/pipeline.rs::execute_inner`（`1425`）——旧表打印点的当前所在（无条件）；本 change 的唯一产品改动落点。
  - `src/pipeline.rs::execute_staged`（`1487`）——语句管道的分段实现；六个执行阶段之前的早退点与两个应保留打印的返回路径都在这里。
  - `src/pipeline.rs::execute_with_params`（`116`）——`with_profiling_scope` 的唯一建立点（`127-133`）；`RTSQL_PROFILING` 未设置时该 scope 不建立，`record_time` 不被调用。本 change 不动它。
  - `src/profiling.rs::print_timings`（`33`）——旧表的渲染；**零改动**。
  - `src/cli/lifecycle.rs::import_csv`（`681`）——失败路径的真实 CLI 观察入口（`840` DDL、`891` INSERT）。
  - `tests/diagnostics_statement_test.rs`——T1 与 T2 的断言落点。

**Implementation Guidance**

- 顺序：先在 `tests/diagnostics_statement_test.rs` 写 T1 的两个用例（先跑出 RED），再改 `src/pipeline.rs` 的旧表打印点，再跑 GREEN；然后做 T2 的断言补强，最后跑该文件的完整 GREEN。
- 打印点的目标形态：把旧表的打印从「`execute_staged` 返回后无条件打印」改回「走到执行阶段的返回路径才打印」。语义上对应基点的两处：缓存命中路径在 `execute_stage` 返回后；未命中路径在进入执行阶段之后。执行阶段之前的六个早退（parse 失败、多语句拒绝、无语句、占位符编号失败、参数个数不匹配、plan 失败）不打印。
- `total_start` 的计时窗口起点保持在缓存查找之前（与基点一致）：基点的 `total_start` 在 `execute_inner` 开头、缓存查找之前创建。
- `record_time` 的调用点与传入的 `profiling` 布尔参数不动。基点在 parse 失败分支里仍 `record_time("parse", ...)`，当前实现把同一次测量在分支之前记入，两者等价；记录与打印是两件事，本 change 只改打印。
- 断言面用真实 CLI 子进程：库 API 入口的 stderr 在 libtest 下不可读回（见 Current-State Evidence），子进程的 stderr 是产品真实输出面。判定用表头特征串 `Time (µs)`（既有 `legacy_profiling_table_follows_env_var` 已用同一判据）。
- 不引入任何新的环境变量操作、fd 重定向、判定层、专用参数、脚本或审计器。

**Task Definitions**

### T1: 旧三段计时表只在走到执行阶段的返回路径输出

- Requirement/Scenario: `unified-diagnostics` 的 Requirement「既有计时出口和 stderr 告警保持」；场景「执行阶段之前的失败路径不输出旧的三段计时表」（新增）、场景「失败路径也输出旧的三段计时表」（改写为执行阶段被触达的失败路径）、场景「旧开关仍输出三段耗时」（保持）。
- Depends on: None
- Targets:
  - `src/pipeline.rs::execute_inner`（`1425`）——旧表打印点的当前所在。
  - `src/pipeline.rs::execute_staged`（`1487`）——六个执行阶段之前的早退点与两个应保留打印的返回路径。
  - `tests/diagnostics_statement_test.rs`——新增两个用例。
- Current behavior: `RTSQL_PROFILING` 已设置时，`execute_inner` 在 `execute_staged` 返回后无条件调用 `print_timings`（`src/pipeline.rs:1456-1458`）。因此解析失败、规划失败、多语句拒绝、占位符编号失败、参数个数不匹配、无语句这些执行阶段之前的失败路径，也会打出一张只含已完成阶段的旧表。
- Required behavior:
  1. 执行阶段之前的失败路径（解析、规划、多语句、占位符、参数个数、无语句）不输出旧的三段计时表。
  2. 走到执行阶段的返回路径输出该表：计划缓存命中后直接执行、以及解析与规划都成功后进入执行，两者都输出。
  3. 进入执行阶段后无论成功还是失败，都输出该表。
  4. 表头、列、行的内容与排序、`Total` 行算法、`total_start` 的计时窗口起点、`record_time` 的调用面不变；`RTSQL_PROFILING` 未设置时不输出该表。
  5. 语句返回的错误文本与 SQLSTATE 不变；诊断摘要、取消语义、取消后的行可见性不变。
- Preserve:
  - `src/profiling.rs` 零改动：`print_timings` 的表头、列、`Total` 行算法，`is_profiling_enabled` 的判定，`record_time` 在旧 scope 外仍 panic 的既有行为。
  - `record_time` 的调用点与 `plan_stage` / `execute_stage` 的 `profiling` 布尔参数传递不变。
  - `[recovery]` 等既有 stderr 告警原文不变。
  - SQL 结果、SQLSTATE、取消后的行可见性不变；诊断（新摘要）的采集与输出不变。
  - 既有 `tests/diagnostics_statement_test.rs` 用例的行为不变；既有 `tests/recovery_bounded_read_test.rs` 的 `[recovery]` 断言不变。
- Forbidden:
  - 改旧表的列、行、`Total` 算法或计时窗口。
  - 把「所有错误路径都不输出」一起改掉（执行阶段的失败路径仍输出该表）。
  - 改诊断（新摘要）的任何行为。
  - 让 `record_time` 具备 scope 安全性，或让 CLI 主命令接入旧表（ISS42 不在承载面）。
  - 新增操作标识字段、运行标识、内容哈希或其他身份证明机制；新增判定层、专用 CLI、脚本、专用参数或审计器。
  - 修改 `openspec/specs/` 权威正文（本 change 的 delta 已就位，不在 Act 改写）。
  - 跑全量（全量在 T3）。
- First test:
  - 位置：`tests/diagnostics_statement_test.rs`。
  - 用例一 `legacy_table_absent_before_execute_stage`（RED）：
    1. `spawn_cli(dir, &["new", <db>], None)` → 退出码 0（建库）。
    2. **正例**（先跑，证明本次 stderr 观测有效）：写 CSV `id\n1\n`，`spawn_cli(dir, &["import", <db>, "t_ok", <csv>, "--csv"], Some("1"))` → 退出码 0，且 stderr 含 `Time (µs)`。
    3. **反例**（区分点）：写 CSV `id,id\n1,2\n`，`spawn_cli(dir, &["import", <db>, "t_bad", <csv>, "--csv"], Some("1"))` → 退出码 3，stderr 含 `duplicate column name`，且 stderr **不含** `Time (µs)`。
    4. 复用文件里既有的 `spawn_cli` / `wait_cli` 辅助，不新增进程管理代码。
  - 用例二 `legacy_table_present_on_execute_stage_failure`（守另一半，改前 GREEN）：
    1. `spawn_cli(dir, &["new", <db>], None)` → 退出码 0。
    2. `spawn_cli(dir, &[<db>, "CREATE TABLE t (id INT PRIMARY KEY)"], None)` → 退出码 0（CLI 主命令不经语句管道，不产生旧表）。
    3. `spawn_cli(dir, &[<db>, "INSERT INTO t VALUES (1)"], None)` → 退出码 0（预置同键行）。
    4. 写 CSV `id\n1\n`，`spawn_cli(dir, &["import", <db>, "t", <csv>, "--csv"], Some("1"))` → 退出码 3，stderr 含 `Duplicate key`，且 stderr 含 `Time (µs)`（这次导入只执行一条语句，所以这张表只能是失败的那条 INSERT 打出的）。
  - RED 判定：命令 `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`，用例一失败且失败点是第 3 步「stderr 不含 `Time (µs)`」——当前实现下执行阶段之前的失败路径仍打出这张表。用例二此时应已通过（当前实现与基线在该路径上逐字节一致）。
- GREEN condition: `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 退出码 0、failed 为 0；用例一与用例二都在其中。用例二的「stderr 含 `Time (µs)`」必须在 GREEN 后仍然成立（防止过度收敛）。
- Verification: `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`。该文件经 CLI 子进程操纵进程级 `RTSQL_PROFILING`，批内单线程。命令退出码 0 且 failed 为 0 才算通过；用例一失败说明执行阶段之前的失败路径仍在打旧表；用例二失败说明把打印点收得过窄（执行阶段的失败路径不再输出）。
- Stop when: 为了让执行阶段之前的失败路径不打旧表，必须改变错误文本、SQLSTATE、诊断摘要内容、取消后的行可见性，或必须改动 `src/profiling.rs` 的表渲染与 `record_time` 的调用面才能完成时，停止并回到计划。

### T2: 锁定权威规格场景「含有解析阶段」子句

- Requirement/Scenario: `unified-diagnostics` 的 Requirement「成功、失败和取消都结束记录」；场景「解析失败留下失败摘要且错误不变」中的「含有解析阶段」子句。
- Depends on: T1（同一测试文件；T1 的 GREEN 后再做本任务，避免在同一文件上交叉改动）
- Targets:
  - `tests/diagnostics_statement_test.rs::parse_error_keeps_error_text_and_summary_fails`（`183-215`）——补断言。
- Current behavior: 该用例断言摘要 severity 为 Error、错误文本逐字节相同、SQLSTATE 相同；规格写明的「含有解析阶段」没有断言锁定。
- Required behavior: 解析失败语句的摘要里存在 `parse` 阶段（`Module::Pipeline`）。用文件里既有的 `stage_stat(summary, module, name)` 辅助断言该阶段存在即可——该辅助在阶段缺失时 panic，断言强度与文件既有风格一致。
- Preserve: 产品代码零改动；该用例已有的 severity、错误文本、SQLSTATE 断言原样保留；权威规格正文不改（该子句已经写明，只是没有断言锁定）。
- Forbidden: 改产品代码让断言成立；改该用例已有的错误文本、SQLSTATE、severity 断言；改权威规格正文；把断言写成对阶段耗时数值的相等比较（只断言阶段存在与阶段名）。
- First test: 本任务没有 RED——断言在改前就成立，属 characterization（实测：解析失败摘要有且仅有 `(Pipeline, "parse")`，count = 1）。变更前 GREEN：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 中 `parse_error_keeps_error_text_and_summary_fails` 通过。
- GREEN condition: 补断言后同一命令仍退出码 0、failed 为 0，且该用例仍通过。
- Verification: `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`，退出码 0 且 failed 为 0。
- Stop when: 补上的断言在改前不成立，或与 T1 的改动冲突（同一文件上的改动互相影响、或断言要求产品行为变化）时，停止并回到计划，不自行改规格正文。

**Invariants**

- `RTSQL_PROFILING` 未设置时，管道不调用 `record_time` / `print_timings`，stderr 不含旧表。
- 旧表本身不变：表头 `Stage | Time (µs) | % Total`、各行按耗时降序、末行 `Total` 取外层耗时且恒 100.0%。
- `record_time` 在旧 scope 之外仍 panic（ISS42 未获裁定，本 change 不触碰）。
- 默认诊断配置下不写出新的诊断 stderr；既有用户可见 stderr 告警（含 `[recovery]`）原文不变。
- 诊断（新摘要）行为不变：失败语句仍产生失败摘要，取消仍记为取消，摘要字段集合不变。
- SQL 结果、错误文本、SQLSTATE、取消后的行可见性不变。
- 本 change 不引入操作标识字段、运行标识、内容哈希或其他身份证明机制；不新增判定层、专用 CLI、专用参数、脚本或审计器。

**Non-goals**

- 不让旧表在所有错误路径都不输出（执行阶段的失败路径仍输出该表）。
- 不改旧表的列、行、`Total` 算法、计时窗口或 `record_time` 的调用面。
- 不改诊断（新摘要）的任何行为，不改默认配置下的 stderr，不改既有告警原文。
- 不修复 ISS42：不让 `record_time` 具备 scope 安全性，不让 CLI 主命令接入旧表。
- 不修改已归档的 MS44 产物，不修改 `openspec/specs/` 权威正文。
- 不跑全量（全量在 T3 / Iteration 001）。
- 不为覆盖不可观察的解析失败面新增失败注入点、专用 CLI 或判定层。

**Acceptance**

1. **旧表只在走到执行阶段的返回路径输出**（对应场景「执行阶段之前的失败路径不输出旧的三段计时表」）：`RTSQL_PROFILING` 已设置、经语句管道执行一条在执行阶段之前失败的语句时，stderr 不含旧的三段计时表。代码位置：`src/pipeline.rs::execute_inner` / `execute_staged` 的打印点。测试：`tests/diagnostics_statement_test.rs::legacy_table_absent_before_execute_stage`。
2. **进入执行阶段后失败仍输出该表**（对应场景「失败路径也输出旧的三段计时表」）：同条件下，一条已触达执行阶段后失败的语句，stderr 仍含旧表。代码位置同上。测试：`tests/diagnostics_statement_test.rs::legacy_table_present_on_execute_stage_failure`。
3. **成功路径与开关语义不变**（对应场景「旧开关仍输出三段耗时」）：`RTSQL_PROFILING` 已设置时成功语句的 stderr 仍含 parse、plan、execute 与 `Total` 行；未设置时不含该表。测试：`tests/diagnostics_statement_test.rs` 的 `legacy_profiling_table_follows_env_var`（既有）与 `legacy_table_absent_before_execute_stage` 的正例步骤。
4. **错误文本与 SQLSTATE 不变**：收敛前后，失败导入的退出码与错误文案逐字节一致（实测 CURRENT 与 BASELINE 一致）；库级入口的 `SqlError` 构造路径零改动，既有 `parse_error_keeps_error_text_and_summary_fails` 继续锁定「错误文本与 SQLSTATE 与诊断关闭时相同」。代码位置：`src/pipeline.rs` 的 `SqlError` 构造处（本 change 不动）。
5. **诊断行为不变**：失败语句仍产生失败摘要，摘要含 `parse` 阶段。测试：`tests/diagnostics_statement_test.rs::parse_error_keeps_error_text_and_summary_fails`（补断言后）。
6. **既有告警原文不变**：`[recovery]` 等既有 stderr 告警仍在产生它的失败条件下出现（`tests/recovery_bounded_read_test.rs` 的既有断言，本 change 不改该文件；在 T3 的分批全量中一并复核）。
7. **规格同步**：本 change 的 delta（`specs/unified-diagnostics/spec.md`）把 Requirement「既有计时出口和 stderr 告警保持」的输出时机写实，并改写、新增对应场景；`openspec validate --strict` 通过。

**Verification**

| Scenario | 判定方式 | 通过依据 |
|---|---|---|
| 执行阶段之前的失败路径不输出旧表 | `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`（用例一） | 用例通过；用例子进程的真实 stderr 不含 `Time (µs)`，退出码 3，错误文案含 `duplicate column name` |
| 进入执行阶段后失败仍输出旧表 | 同一命令（用例二） | 用例通过；子进程 stderr 含 `Time (µs)` 与 `Duplicate key`，退出码 3 |
| 成功路径与开关语义不变 | 同一命令（既有 `legacy_profiling_table_follows_env_var` + 用例一的正例步骤） | 成功导入 stderr 含 `Time (µs)`；未设置变量时不含 |
| 错误文本与 SQLSTATE 不变 | 同一命令；库级 SQLSTATE 由既有 `parse_error_keeps_error_text_and_summary_fails` 锁定 | 失败导入退出码 3 且文案含 `duplicate column name`；库级错误文本与 SQLSTATE 断言通过 |
| 诊断行为不变 /「含有解析阶段」 | 同一命令（`parse_error_keeps_error_text_and_summary_fails`） | 摘要 severity 为 Error 且存在 `(Pipeline, "parse")` 阶段 |
| 既有告警原文不变 | T3 分批全量中 `tests/recovery_bounded_read_test.rs` 所在批次 | 该批次退出码 0、failed 为 0 |
| 规格同步 | `openspec validate 2026-10-30-diagnostics-legacy-table-error-path --strict` | 输出 `Change '...' is valid` |

判定面说明：旧表写到 stderr，Rust 测试框架捕获 `eprintln!`（含被 spawn 的线程），因此旧表的判定必须经子进程的真实 stderr——这是真实入口、真实用户所见，不需要额外的判定层。

**Gate 2 Readiness**

PASS。Plan 审查 11 项逐条通过：规格 delta 与权威正文一致且 `openspec validate --strict` 通过；设计闭合（打印点条件、计时窗口、记录与打印的分工、判定面、备选方案已写死）；当前 Cycle 与 change 的 `tasks.md`、`specs/`、`design.md` 一致；RTM 无 Missing、无 Simplified；Persisted Evidence 为 `none`；验证直接观察目标行为（子进程 stderr），无身份证明机制与判定层；无 TBD/TODO；无未批准的需求裁剪；未修改无关范围（产品改动只落在 `src/pipeline.rs` 的打印点）；Act 不需回读本 change 之外的规划资料即可建立先行测试。无 BLOCKED 项，无 SKIPPED 项。

**Persisted Evidence**

- Mode: none

命令、实际结果、修改文件与符号写入 Act Response 即可。失败批只整批复核一次的复核记录也写在 Act Response。

**Risks and Notes**

- 打印点收得过窄（把执行阶段的失败路径也一并抑制）会让用例二失败；用例二与用例一在同一个文件里，一起跑，互相约束。
- 失败路径的库级 stderr 在本仓库的测试框架内不可观察——这是 libtest 捕获 `eprintln!` 的既有事实，不是本 change 引入的限制。因此解析失败面不由用例直接覆盖，而由「打印点与基点逐条对齐」保证（基点 `e69c7e2` 的 parse 失败分支在任何 `print_timings` 之前返回）。这一点是已知的覆盖缺口，不是被忽略的缺口。
- 权威规格的场景「失败路径也输出旧的三段计时表」按 OpenSpec 集成约束保留标题、把适用范围收窄到执行阶段被触达的失败路径（`openspec validate --strict` 要求 MODIFIED 块保留已有场景标题，删除会判 ERROR 并阻断归档）。两个场景合起来覆盖失败路径的两半。
- 基点 `e69c7e2` 是 master 上 MS44 变更的 merge-base（`git merge-base --is-ancestor e69c7e2 master` 成立；从 MS44 的产品提交 `1081a5f` 起到规划时的 master HEAD `15de1c0`，`git diff e69c7e2 -- src/pipeline.rs` 的改动只涉及 `execute_with_params`、`execute_inner` 与新增的 `execute_staged`）。
- ISS42 保持 open，不在本 Cycle 承载；本 Cycle 不触碰 `src/profiling.rs`，也不让 CLI 主命令接入旧表。
- 对照实测用的基线二进制来自 `/tmp` 下 `e69c7e2` 的临时克隆（仓库之外），测量完成后已删除；复现步骤见上面 Current-State Evidence 的「运行观察」一节。
- 非实质未知项：旧表各行的具体耗时数值随机器与负载变化，不作为判定条件；判定只认表头特征串与行的存在性。

## Act Response

- Status: reported

### 实际改动

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/pipeline.rs` | `execute_inner`（1425） | 删除「`execute_staged` 返回后无条件 `print_timings`」；`total_start` 的创建点不动，改为随 `execute_staged` 调用传入 |
| `src/pipeline.rs` | `execute_staged`（1490） | 新增参数 `total_start: Option<Instant>`；新增两处旧表打印——缓存命中路径在 `execute_stage` 返回后（1538-1541）、未命中路径在 execute 阶段之后（1659-1663）。执行阶段之前的六个早退不经过这两处 |
| `tests/diagnostics_statement_test.rs` | `legacy_table_absent_before_execute_stage`（新增） | CLI 子进程：执行阶段之前失败（自动建表 DDL 规划期拒绝——重复列名）→ stderr 不含 `Time (µs)`；同用例先做一次成功导入证明 stderr 观测面有效 |
| `tests/diagnostics_statement_test.rs` | `legacy_table_present_on_execute_stage_failure`（新增） | CLI 子进程：执行阶段内失败（预置同键行后导入触发 `Duplicate key`）→ stderr 仍含 `Time (µs)`，守「执行阶段失败仍输出」这一半 |
| `tests/diagnostics_statement_test.rs` | `parse_error_keeps_error_text_and_summary_fails`（断言在 215-217） | 补 `stage_stat(&summaries[0], Module::Pipeline, "parse")` 断言，锁定「含有解析阶段」子句 |

产品改动只落在旧表的打印点：`src/profiling.rs` 零改动（表渲染与 `record_time` 的 scope 外 panic 行为未触碰）；`record_time` 的调用点与 `profiling` 布尔参数传递不变；`SqlError` 构造路径不变（错误文本与 SQLSTATE 逐字不动）；诊断（新摘要）的采集、渲染与 stderr 时序不变。

### 与计划的偏差（Deviations from Plan）

1. **`total_start` 的传递方式（实施细节，Plan 未指定）**：Plan 只写「把旧表的打印改回到执行阶段的返回路径」，未定打印点如何拿到外层计时起点。实现选择把 `execute_inner` 里既有的 `total_start` 作为参数传给 `execute_staged`（创建点不动 ⇒ 计时窗口起点仍在缓存查找之前，与基点一致），而没有在 `execute_staged` 内重建起点，也没有引入「是否打印」标志（design D1 已否决后者）。可观察行为与基点一致。
2. **delta 规格未改写**：Plan 已把 delta 写进 change 产物（Task Definition 的 Forbidden 项写明「delta 已就位，不在 Act 改写」）。Act 逐条核对 delta 与实现一致（输出时机段落、两个失败场景的适用范围），确认无需改动，未改写；`openspec validate --strict` 通过。
3. **手工核对场景 C 的预置行**：手工核对时目标表已含 id=1 行，预置 INSERT 因主键冲突退出 3（主命令不经语句管道，本就不产生旧表）。判定不受影响——随后的导入仍触发执行期 `Duplicate key` 并打出恰好 1 张表。

### 自检发现与处理

| 项 | 发现 | 处理 |
|---|---|---|
| 格式 | `cargo fmt --check` 标出我新增测试中的一处数组字面量排版 | 已按 rustfmt 修正；本次触碰的两个文件现在 rustfmt 干净 |
| 既有基线偏差 | `cargo fmt --check` 仍标出 `tests/diagnostics_facade_test.rs:216` 与 `tests/session_lifecycle_test.rs:15`（HEAD 即存在，本次未触碰） | 未改（Surgical Changes；与本 change 无关） |
| lint | `cargo clippy --offline --lib --test diagnostics_statement_test` 只有 1 条 warning（`src/diagnostics.rs:396` redundant closure，既有） | 本次触碰的文件 0 条 warning；既有 lint 未改 |
| 行为核对 | 三个场景的真实 CLI 运行结果与 Plan 记录的基点 `e69c7e2` 实测表逐行一致 | 无问题 |

无 Critical / Important 问题，无遗留 Minor 问题；未引入操作标识字段、运行标识、内容哈希或任何判定层、专用 CLI、脚本、审计器。

### Gate 记录

- Gate 3（T1，GREEN 后）：requirement 与两个场景完整实现；计划列出的文件与符号全部处理；`src/profiling.rs` 与 `record_time` 调用面未破坏；先行测试符合任务类型（用例一 RED 证明目标行为、用例二防过度收敛）。通过。
- Gate 3（T2，GREEN 后）：只加断言、零产品改动；既有 severity / 错误文本 / SQLSTATE 断言原样保留；断言强度与文件既有风格一致（`stage_stat` 缺失即 panic）。通过。
- Gate 4：定向验证退出码 0、failed 为 0；完成声明基于当次真实运行。通过。

### 验证命令与实际结果

1. **RED（T1 先行测试）**：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`
   → `test result: FAILED. 8 passed; 1 failed`；失败用例 `legacy_table_absent_before_execute_stage`，失败点是第 3 步「stderr 不含 `Time (µs)`」——当次 stderr 实际含一张表（parse / plan / cache_hit_check + `Total`）。同批 `legacy_table_present_on_execute_stage_failure` 通过（改前 GREEN，守另一半）。
2. **GREEN（T1 实现后）**：同一命令 → `test result: ok. 9 passed; 0 failed`。
3. **GREEN（T2 断言后 + rustfmt 修正后复跑）**：同一命令 → `test result: ok. 9 passed; 0 failed`（含既有 `legacy_profiling_table_follows_env_var`、`parse_error_keeps_error_text_and_summary_fails`）。
4. **真实 CLI 运行核对**（被测二进制为 change worktree 的 `target/debug/rtsql`，HEAD `f0e36a5` + 本 Cycle 改动；`RTSQL_PROFILING=1`）：

   | 场景 | 观测 | 基点 `e69c7e2`（Plan 记录） |
   |---|---|---|
   | 成功导入 1 行 | exit 0，stderr 2 张旧表 | exit 0，2 张旧表 |
   | 自动建表 DDL 规划期失败（重复列名） | exit 3，stderr 0 张旧表，错误文本含 `duplicate column name` | exit 3，无旧表 |
   | 逐行 INSERT 执行期失败（同键） | exit 3，stderr 恰好 1 张旧表，错误文本含 `Duplicate key` | exit 3，恰好 1 张旧表 |

5. **规格**：`openspec validate 2026-10-30-diagnostics-legacy-table-error-path --strict` → `Change '2026-10-30-diagnostics-legacy-table-error-path' is valid`，退出码 0；`openspec validate --all --strict` → `Totals: 91 passed, 0 failed`。
6. **全量**：本轮不跑（全量在 T3 / Iteration 001，M19 分批预算）。

### Persisted Evidence

None required（Plan 设定的模式为 `none`；命令、实际结果与修改清单已在本 Response 记录）。

### Experience Candidates

None。本轮未发现 change 范围外的实质缺陷：ISS42（`RTSQL_PROFILING` 对 CLI 主命令无效、scope 外 panic）按用户指令不承载、保持 open，本 change 未触碰 `src/profiling.rs`，也未让 CLI 主命令接入旧表；未产生新的可复用 Runbook 候选。

### 未解决问题与需要用户决定的事项

- **ISS42 保持 open**：按 goal 指令「除了 iss42，其他的都顺便做掉吧」，本 change 不承载其修复，也未改变其判定前提。
- **全量回归待下一轮**：T3（Iteration 001）尚未执行，按 M19 分批串行跑一次；本 Cycle 不宣称全量通过。
- **已知覆盖缺口（Plan 已记录，非本轮新增）**：解析失败等执行阶段之前的早退在库入口的 stderr 于本仓库测试框架内不可观察（libtest 捕获 `eprintln!`），因此该面由「打印点逐条对齐基点」保证，不由测试直接覆盖。
- **无其它需要用户决定的事项**：本 Cycle 未扩大已批准范围，未修改已批准 Acceptance，未新增产品语义。

## Plan Review

- Review Result: accepted

**Findings**

本 Review 独立阅读了完整 diff（merge-base `6412caf` → HEAD `9401b2d`）、实际代码与测试，逐条核对 Act Response 声明与代码的一致性、Acceptance 的满足情况与 Preserve / Forbidden 的成立情况。Act 已报告且覆盖范围未失效的验证结论直接沿用（来源见 Evidence），未重复运行任何已通过的定向验证，本次未跑全量。

1. **T1 的打印点与基点 `e69c7e2` 逐条返回路径等价（偏差归类：无——Plan 判断正确、Act 实施正确）**

   核对依据：`git show e69c7e2:src/pipeline.rs` 的基点 `execute_inner`（1411-1552 行，`print_timings` 仅 1449、1549 两处）与 HEAD `9401b2d` 的 `execute_inner`（1425-1477）/ `execute_staged`（1490-1665，`print_timings` 仅 1540、1662 两处；全仓 `grep -rn print_timings` 无第三处调用）。

   | 返回路径 | 基点 `e69c7e2` | HEAD `9401b2d` | 等价 |
   |---|---|---|---|
   | `RTSQL_PROFILING` 未设置 | `total_start = None`，两处打印点均不触发 | 同（`execute_inner:1441-1445` 建点，两处 `if let Some(start) = total_start` 均不成立） | 是 |
   | 计时窗口起点 | `execute_inner` 开头、缓存查找之前（1423-1427） | 同（1441-1445，仍在 `execute_staged` 调用之前；缓存查找在 `execute_staged:1506-1510`） | 是 |
   | 缓存命中 → 执行（成功或失败） | `execute_stage` 返回后 `print_timings`（1448-1450）；命中路径不记 `execute` 行 | `execute_staged:1534-1542`：`execute_stage` 返回 → `close_stage` → `print_timings` → return；仍不 `record_time("execute")` | 是（表内容与行集合不变） |
   | 缓存未命中 → 解析失败 | `record_time("parse")` 后 `return Err`，不打印（1462-1473） | `record_time("parse")` 在分支前记一次（1559-1565），`return Err` 于 1569，不打印 | 是（记录次数与时点等价） |
   | → 多语句拒绝 | `return Err`，不打印（1479-1480） | 同（1577-1579） | 是 |
   | → 无语句可执行 | `return Err`，不打印（1482-1491） | 同（1580-1588） | 是 |
   | → 占位符编号失败 | `return Err`，不打印（1493-1503） | 同（1592-1601） | 是 |
   | → 参数个数不匹配 | `return Err`，不打印（1505-1507） | 同（1603-1605） | 是 |
   | → 规划失败 | `record_time("plan")` 后 `return Err`，不打印（1516-1526） | `record_time("plan")` 在分支前记一次（1619-1625），`return Err` 于 1629，不打印 | 是 |
   | → 进入执行阶段（成功或失败） | `record_time("execute")` 后 `print_timings`，再返回结果（1544-1552） | `record_time("execute")` + `close_stage` 后 `print_timings`（1652-1664），返回结果 | 是（执行失败同样打印） |

   结论：两版的打印点数量与位置语义一致（基点 2 处 = 当前 2 处），六个执行阶段之前的早退在两版都不经过打印点。**「精确恢复基点 `e69c7e2` 的旧表输出条件」成立**，且没有落入「所有错误路径都不输出」的过度收敛——执行阶段内的失败路径仍打印。

   三点补充核对：
   - 旧表内容不受诊断阶段影响：`close_stage` / `record_stage` 是诊断侧 API，无绑定时 `try_with` 为 no-op（`src/diagnostics.rs:733-741`），不写 `PROFILING_DATA`；旧表 map 只由 `record_time` 写（`src/profiling.rs:21-25`）。因此缓存命中路径新增的 `close_stage("execute", ...)` 不会给旧表新增 `execute` 行，与基点一致。
   - 旧表与诊断摘要的相对顺序不变：变更前（merge-base `6412caf`）是「先打印旧表 → 后渲染诊断摘要」（`execute_inner:1456-1458` → `1461-1474`）；当前实现把打印点移入 `execute_staged`，仍在摘要渲染之前，顺序不变。
   - `print_timings` 仍在 `PROFILING_DATA` 作用域内调用（`execute_with_params` 的 `with_profiling_scope`，`src/pipeline.rs:129-141`），ISS42 的 scope 外 panic 前提未被触碰。

2. **T2 的断言与权威规格一致（偏差归类：无）**

   `tests/diagnostics_statement_test.rs:215-217` 在既有的 severity、错误文本逐字节、SQLSTATE 断言之后追加 `stage_stat(&summaries[0], Module::Pipeline, "parse")`；`stage_stat` 在阶段缺失时 panic（该文件 37-43 行），断言强度与文件既有风格一致。对应权威规格 `openspec/specs/unified-diagnostics/spec.md:133-138` 场景「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句。产品代码零改动（diff 只触及 `src/pipeline.rs` 与 `tests/diagnostics_statement_test.rs`）。

3. **delta 规格与实现逐条对应（偏差归类：无）**

   - Requirement 正文：「只在执行阶段被触达的返回路径输出这张表」+ 六类执行阶段之前的失败路径「SHALL NOT 输出」→ 实现侧这六条路径（`src/pipeline.rs:1569/1578/1583/1596/1604/1629`）都没有打印点，规则逐条成立；「执行阶段被触达指两种情形：计划缓存命中后直接执行，以及解析与规划都成功后进入执行」→ 实现侧正是 `1534-1542`（命中）与 `1651-1663`（未命中进入执行）两条，无第三条路径。
   - 场景「失败路径也输出旧的三段计时表」（改写后收窄到已触达执行阶段的失败路径）→ `1659-1663` 在 execute 阶段之后打印，execute 失败也打印；测试 `legacy_table_present_on_execute_stage_failure` 覆盖。
   - 场景「执行阶段之前的失败路径不输出旧的三段计时表」（新增）→ 六个早退无打印点；测试 `legacy_table_absent_before_execute_stage` 覆盖其中的规划期失败实例。
   - 场景「旧开关仍输出三段耗时」与「恢复告警原文仍在」→ 前者由未改动的 `legacy_profiling_table_follows_env_var` 覆盖；后者涉及的 `[recovery]` 告警代码本轮未触及（Acceptance 6，按计划推迟到 T3 复核）。
   - OpenSpec 集成约束（MODIFIED 块保留既有场景标题）：delta 保留「旧开关仍输出三段耗时」「恢复告警原文仍在」「失败路径也输出旧的三段计时表」三个既有标题，并新增互补的一个；Requirement 标题「既有计时出口和 stderr 告警保持」未改。`openspec validate --strict` 通过（沿用 Act 结论；delta 文件自 `f0e36a5` 起未被改动）。

4. **Act Response 声明与代码一致（偏差归类：无）**

   Act Response「实际改动」表的 5 行与 diff 逐条相符：`execute_inner` 删除无条件打印并把 `total_start` 传入 `execute_staged`（1453）、`execute_staged` 新增参数与两处打印（1538-1541、1659-1663）、两个新用例（`legacy_table_absent_before_execute_stage` 473 行、`legacy_table_present_on_execute_stage_failure` 549 行）、补断言（217 行）。Act Response 的三条偏差属实：偏差 2「delta 未改写」经 `git diff f0e36a5..9401b2d` 证实（该区间只含 Cycle 文件与 `tasks.md`）。

5. **Preserve 与 Forbidden 逐条成立（偏差归类：无）**

   `src/profiling.rs` 零改动（diff 未触及该文件：表头、列、`Total` 行算法、`is_profiling_enabled` 判定、`record_time` 的 scope 外 panic 行为均未变）；`record_time` 的四个调用点未变（`cache_hit_check` 1514、`parse` 1562、`plan` 1622、`execute` 1655），`profiling` 布尔参数的传递未变；`SqlError` 构造路径零改动（错误文本与 SQLSTATE 逐字不动）；诊断（新摘要）的采集、渲染与时序未变；`[recovery]` 等既有告警原文未触及；未引入操作标识字段、运行标识、内容哈希或其他身份证明机制；未新增判定层、专用 CLI、专用参数、脚本或审计器；未修改 `openspec/specs/` 权威正文；本轮未跑全量。

6. **Minor findings（均不阻塞，不要求返工）**

   - Minor 1（实施细节，Act 偏差 1）：打印点的实现机制与 Plan Context 的字面表述不同——把既有的 `total_start` 作为参数传入 `execute_staged`，而不是 Plan 在 design D1 已否决的「是否打印」标志。可观察行为与基点等价，不是缺陷。
   - Minor 2（覆盖缺口，Plan 已记录）：六个执行阶段之前的早退中，只有规划期失败被测试直接覆盖；解析失败、多语句、占位符、参数个数、无语句靠「这些路径上不存在打印点」的结构性等价保证。CLI 侧到不了解析失败面，libtest 捕获 `eprintln!` 使库入口 stderr 不可读回。当前实现让六条路径结构完全一致，该缺口已被构造消除，不是被忽略的缺口。
   - Minor 3（判定面粒度）：delta 的两个失败场景都含「AND 返回的错误文本与 SQLSTATE 不变」；CLI 层只能观察退出码与错误文案，SQLSTATE 由未改动的 `SqlError` 构造路径与库级断言 `parse_error_keeps_error_text_and_summary_fails` 保证。
   - Minor 4（既有噪声，不在本 change 判定面）：`tests/diagnostics_facade_test.rs:216` 与 `tests/session_lifecycle_test.rs:15` 的既有 rustfmt 漂移（Act 实测报告；两处都不在本 change 的 diff 内，本轮未触碰）。

**Acceptance Gaps**

无阻塞缺口。逐条对照 Cycle 的 7 条 Acceptance：

| # | Acceptance | 结论 | 依据 |
|---|---|---|---|
| 1 | 执行阶段之前的失败路径不输出旧表 | 满足 | 实现：六个早退无打印点（`src/pipeline.rs:1569/1578/1583/1596/1604/1629`）；测试 `legacy_table_absent_before_execute_stage` |
| 2 | 进入执行阶段后失败仍输出该表 | 满足 | 实现：`1659-1663` 在 execute 阶段之后打印；测试 `legacy_table_present_on_execute_stage_failure` |
| 3 | 成功路径与开关语义不变 | 满足 | 实现：`1659-1663` 覆盖成功路径；既有 `legacy_profiling_table_follows_env_var` 未改且随批通过；用例一的正例步骤覆盖「设置变量即出表」 |
| 4 | 错误文本与 SQLSTATE 不变 | 满足 | diff 未触及任何 `SqlError` 构造；用例断言退出码 3 与 `duplicate column name` / `Duplicate key` 文案；库级 `parse_error_keeps_error_text_and_summary_fails` 锁定错误文本与 SQLSTATE |
| 5 | 诊断行为不变 / 含解析阶段 | 满足 | T2 断言（`tests/diagnostics_statement_test.rs:217`）；`src/diagnostics.rs` 零改动 |
| 6 | 既有告警原文不变 | 计划内推迟到 T3 | `[recovery]` 告警代码未触及；由 `tests/recovery_bounded_read_test.rs` 所在批次在全量中复核（Iteration 001） |
| 7 | 规格同步 | 满足 | delta 的 MODIFIED 块保留三个既有场景标题并新增一个；`openspec validate --strict` 通过（沿用 Act 结论） |

Acceptance 6 是 Plan Context 明确推迟到 T3 的项（Verification 表写明「T3 分批全量中 `tests/recovery_bounded_read_test.rs` 所在批次」），不构成本轮缺口；其余 6 条本轮全部满足。

**Evidence**

- Diff 范围：change 分支 merge-base `6412caf` → HEAD `9401b2d`，8 个文件：change 产物 6 个（`.openspec.yaml` / `design.md` / `proposal.md` / `specs/unified-diagnostics/spec.md` / `tasks.md` / 本 Cycle 文件），产品与测试 2 个（`src/pipeline.rs` +17/-4、`tests/diagnostics_statement_test.rs` +154/-0）。
- 基点核对：`git show e69c7e2:src/pipeline.rs`——基点 `execute_inner` 1411-1552；`print_timings` 两处（1449、1549）；六个早退（1462、1479、1482、1493、1505、1516）。
- 当前实现：`src/pipeline.rs::execute_inner`（1425-1477）、`execute_staged`（1490-1665）；全仓 `print_timings` 调用点仅 `src/pipeline.rs:1540` 与 `1662`（`grep -rn`）。
- 测试断言落点：`tests/diagnostics_statement_test.rs`——`legacy_table_absent_before_execute_stage`（473）、`legacy_table_present_on_execute_stage_failure`（549）、`stage_stat(&summaries[0], Module::Pipeline, "parse")`（217）；`spawn_cli` / `wait_cli` / `stage_stat` 复用文件内既有实现，未新增进程管理代码。
- 沿用的验证结论（来源：本 Cycle 的 Act Response「验证命令与实际结果」；被覆盖代码自 `9401b2d` 起未变化，无公共规则 › 验证 列出的重跑触发条件）：RED `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` → 8 passed / 1 failed（失败点为「stderr 不含 `Time (µs)`」，当次 stderr 实含一张表）；GREEN（T1 后）同一命令 → 9 passed / 0 failed；GREEN（T2 断言后）同一命令 → 9 passed / 0 failed；真实 CLI 三场景核对——成功导入 exit 0 / 2 张表、规划期失败 exit 3 / 0 张表且文案含 `duplicate column name`、执行期失败 exit 3 / 恰好 1 张表且文案含 `Duplicate key`，与基点 `e69c7e2` 的 Plan 记录逐行一致；`openspec validate 2026-10-30-diagnostics-legacy-table-error-path --strict` → `Change '...' is valid`；`openspec validate --all --strict` → 91 passed / 0 failed。
- 本 Review 实际执行的命令：`git diff`（两处区间）、`git show e69c7e2:src/pipeline.rs`、`git diff --numstat`、`git status`、`git log`、`grep -rn`（只读检索）。未跑全量，未重复运行任何已通过的定向验证。

**Follow-up Decision**

接受。逐条返回路径与基点 `e69c7e2` 的等价性成立（对照表见 Findings 1），delta 与实现逐条对应，Acceptance 除计划内推迟到 T3 的第 6 条外全部满足，Preserve / Forbidden 无违反，无阻塞 finding。四条 Minor 均不阻塞，按公共规则 › Iteration 与 Cycle 线程 不为其创建修复、不要求当前 Cycle 返工。当前 Cycle 无需修复，不创建 rework / replan Cycle；Iteration 000 完成，展开 Map 中的下一 Iteration。

**Iteration Plan Update**

None。目标、范围、依赖、验证方式与验收边界均未改变：`tasks.md` 的 Iteration Plan 保持 000 = 旧表输出时机收敛（T1、T2）、001 = 收尾全量（T3）。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-10-30-diagnostics-legacy-table-error-path/iterations/001-final-regression/000-initial.md`（Iteration 001：收尾全量，任务 T3）。
