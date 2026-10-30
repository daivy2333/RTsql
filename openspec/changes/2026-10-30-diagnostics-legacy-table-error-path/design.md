## Context

见 `proposal.md` 的 Why。这里只记实现所依赖的当前代码与基线事实。

规划时读到的现状（change 分支 HEAD `6412caf`）：

- `src/profiling.rs` 用 `task_local` 保存 `HashMap<&'static str, Duration>`。`is_profiling_enabled`（`src/profiling.rs:55-57`）只判断 `RTSQL_PROFILING` 是否存在。`record_time`（`src/profiling.rs:21-25`）在旧 scope 未设置时 panic。`print_timings`（`src/profiling.rs:33-52`）向 stderr 打表：表头 `Stage | Time (µs) | % Total`，各行按耗时降序，末行是 `Total`（总耗时取调用方传入的外层耗时，恒为 100.0%）。列宽、`Total` 行算法、表头文本都在本文件里，本变更不改。
- `src/pipeline.rs:1425-1480` 的 `execute_inner`：`profiling = is_profiling_enabled()`；`profiling` 为真时建 `total_start`；调用 `execute_staged`；随后 `if let Some(start) = total_start { print_timings(start.elapsed()); }`（`src/pipeline.rs:1456-1458`）——**无条件**打印，不区分返回路径。
- `src/pipeline.rs:1487-1652` 的 `execute_staged`（MS44 Iteration 001 从 `execute_inner` 主体拆出）：缓存查找 → 命中路径直接 `execute_stage` 后返回（`src/pipeline.rs:1519-1535`）；未命中路径依次经过 parse（`1550`）、多语句拒绝（`1569-1571`）、无语句（`1572-1580`）、占位符编号（`1584-1593`）、参数个数（`1595-1597`）、plan（`1610`）、execute（`1643`）。前六处都是**早退**。
- `record_time` 的调用面：`cache_hit_check`（`1510`）、`parse`（`1554`）、`plan`（`1614`）、`execute`（`1647`），另加 `src/pipeline.rs` 内 `table_metadata_lookup` / `executor_creation` / `executor_execution` 与 `src/executor/index_scan.rs`、`src/executor/index_scan_all.rs` 两处索引子指标。`print_timings` 全仓只有一个调用点：`src/pipeline.rs:1457`。
- 基线 `e69c7e2`（`git show e69c7e2:src/pipeline.rs`，MS44 变更前的 `execute_inner` 单函数，1411-1552 行）：`print_timings` 只有两处——缓存命中路径 `execute_stage` 返回后（1448-1450）与未命中路径 `record_time("execute")` 之后（1548-1550）；两处都**不区分执行成功还是失败**。未命中路径的六个早退（parse 失败 1462-1473、多语句 1479-1480、无语句 1482-1491、占位符 1493-1503、参数个数 1505-1507、plan 失败 1516-1526）全部在任何 `print_timings` 之前返回。基线的 parse 失败分支在 `return` 之前仍 `record_time("parse", ...)`（1465-1467）——**记了时间，只是不打表**。
- `tests/diagnostics_statement_test.rs`：`#![cfg(feature = "cli")]`；已有 `spawn_cli(dir, args, profiling: Option<&str>)`（`325-344`，设置 `RTSQL_HOME`、移除 `RTSQL_KEY` / `RTSQL_BUSY_TIMEOUT`、按需设置或移除 `RTSQL_PROFILING`）与 `wait_cli(child) -> CliOutput { code, stdout, stderr }`（`347-381`）；既有 `legacy_profiling_table_follows_env_var`（`387-459`）已用 `Time (µs)` 区分旧表、已用 `diagnostics ` 前缀区分新诊断输出。该文件按既有约定以 `--test-threads=1` 串行执行（CLI 子进程操纵进程级环境变量）。
- `rtsql import`（`src/cli/lifecycle.rs:681`）是 CLI 侧唯一经 `Database::execute_sql` 进入语句管道的路径：自动建表 DDL（`lifecycle.rs:840`）与逐行 INSERT（`lifecycle.rs:891`）都走 `db.execute_sql` → `pipeline::execute` → `execute_with_params` → `execute_inner`。表名与列名都经 `quote_ident`（`lifecycle.rs:1031-1033`）加双引号，用户输入不产生解析失败面。
- 规划期失败面：`src/parser/planner/ddl_dml.rs:1194-1201` 的「重复列名计划期拒绝」。执行期失败面：键冲突（`Duplicate key`）。

规划时实测的运行事实（Linux x86_64；CURRENT = change worktree 的 `target/debug/rtsql`，HEAD `6412caf`；BASELINE = `/tmp` 下 `e69c7e2` 的克隆构建）：

| 场景 | CURRENT | BASELINE |
|---|---|---|
| `rtsql import` 导入重复列名 CSV（表头 `id,id`）→ 自动建表 DDL 规划期失败 | exit 3，stderr 含 1 张旧表（parse / plan / cache_hit_check + `Total`） | exit 3，stderr 无旧表 |
| 预置同键行后 `rtsql import` 导入单行同键 CSV → INSERT 执行期失败 | exit 3，stderr 恰好 1 张旧表 | exit 3，stderr 恰好 1 张旧表 |
| `rtsql import` 成功导入 1 行 | exit 0，stderr 2 张旧表 | exit 0，stderr 2 张旧表 |

三种场景下 CURRENT 与 BASELINE 的错误文本与退出码逐字节一致。差别只在第一行：执行阶段之前的失败路径现在多打了一张表。

## Goals / Non-Goals

**Goals:**

- 旧三段计时表的输出时机与 `e69c7e2` 逐条返回路径对齐。
- 成功路径、缓存命中路径、执行阶段失败路径的旧表输出保持不变。
- 权威规格与实现一致，不再有「失败路径也输出旧表」的规格文字。
- 补齐「解析失败摘要含解析阶段」的断言缺口。

**Non-Goals:**

- 见 `proposal.md` 的「不做」。这里不重复 ISS42 的 `record_time` scope 安全性，不改旧表的列与算法，不改诊断（新摘要）行为。

## Decisions

### 1. 打印点回到「走到执行阶段的返回路径」

旧表的打印点从 `execute_inner` 的无条件调用改回基线的两处位置语义：缓存命中路径在 `execute_stage` 返回后、未命中路径在进入执行阶段之后。执行阶段之前的六个早退（parse 失败、多语句拒绝、无语句、占位符编号失败、参数个数不匹配、plan 失败）不打印。

`total_start` 的计时窗口起点保持在缓存查找之前，与基点一致（基点的 `total_start` 就在 `execute_inner` 开头、缓存查找之前）。窗口内的内容（各行阶段耗时、`Total` 的外层耗时）随打印点一起回到基点语义。

`record_time` 的调用面不变：基点在 parse 失败分支里仍然 `record_time("parse", ...)`，当前实现把同一次测量在分支之前记入，两者等价。`record_time` 在旧 scope 外仍 panic 的既有行为不变（ISS42 不在本 change 承载）。

备选：给 `execute_staged` 加一个「是否打印旧表」的返回标志，由 `execute_inner` 统一打印。不采用。执行阶段失败时该标志要随错误一起返回，错误路径要多带一个载荷，反而把「打印」这件事和错误结果耦合；且打印点分散在两处时基点语义更难逐条核对。备选：改 `print_timings` 的写入方式（例如改用 `std::io::stderr()`）以便在进程内断言。不采用。那是为了测试可观察性改产品写法，旧表的输出字节与算法必须逐字保持。备选：把「所有错误路径都不输出」一起改掉。不采用。用户指令明确要求精确对齐基点条件，执行阶段的失败路径在基点是打印的。

### 2. 失败路径的判定走真实 CLI 入口

旧表打到 stderr。Rust 测试框架会捕获 `eprintln!`（含被 spawn 的线程，Rust 1.99 的 `std::thread::add_spawn_hook` 让新线程继承捕获），因此在 `cargo test --test <file> -- --test-threads=1` 下，库 API 入口的旧表 stderr 在进程内不可读回。改用真实 CLI 子进程观察：子进程的 stderr 是产品真实输出面，由 `Command` 管道收进 `CliOutput.stderr`。

可经 CLI 到达的语句管道失败面：

- 执行阶段之前：`rtsql import` 的自动建表 DDL 在规划期被拒（重复列名）。
- 执行阶段：预置同键行后逐行 INSERT 在执行期被拒（Duplicate key）。
- 成功：正常导入。

解析失败面在 CLI 侧不可达（`rtsql import` 生成的 SQL 文本都经 `quote_ident`，不产生解析失败）。因此解析失败与其他执行阶段之前的早退由「打印点逐条对齐基点」保证，规格用一条覆盖「执行阶段之前」的规则表述，不为覆盖一个不可观察面新增失败注入点。

判定用表头特征串 `Time (µs)`（既有 `legacy_profiling_table_follows_env_var` 已用同一判据区分旧表与新诊断输出）。不引入行标识、运行标识、内容哈希或其他身份机制，也不新增脚本、专用参数或审计器。

备选：用 `dup2` 把 fd 2 重定向到临时文件后在进程内读回。不采用。`eprintln!` 仍被测试框架捕获，文件里只会是空内容，断言会变成恒真。备选：在测试进程里改 `RTSQL_PROFILING` 环境变量。不采用。进程级环境变量与并发测试的交互没有可控边界，且同样受捕获限制。备选：给库加一个测试专用的 stderr 导出口。不采用。那是判定层。

### 3. 规格 delta 保留既有场景标题，改写其适用范围

`openspec validate --strict` 要求 MODIFIED 块保留当前规格里该 Requirement 的全部场景标题：删掉任何一个会判 ERROR，并在 `openspec archive` 时阻断归档（除非显式 `--no-validate --yes`）。本仓库当前的基线是 `openspec validate --all --strict` 90 passed / 0 failed（规划前实测；唯一的 failed 就是本 change 的场景标题缺失）。

因此场景「失败路径也输出旧的三段计时表」保留标题，把 GIVEN / WHEN 收窄到「已触达执行阶段的失败路径」；再新增互补的场景「执行阶段之前的失败路径不输出旧的三段计时表」。两个场景合起来覆盖失败路径的两半，Requirement 正文把规则写死（执行阶段被触达才输出，执行阶段之前不输出）。用户对场景的授权是「删除或改写」，改写在授权内。

备选：删除该场景并接受 `openspec validate` 失败、归档时用 `--no-validate --yes`。不采用。那会让仓库的 validate 基线出现唯一的 ERROR，并把归档的默认门禁降级；改写标题保留、正文收窄既满足授权也满足集成约束。备选：改 Requirement 的名字后整体 REMOVED + ADDED。不采用。为一次行为修正更换 Requirement 标题会打断与已归档 MS44 的引用关系。

### 4. 断言「含有解析阶段」是锁定既有行为

权威规格场景「解析失败留下失败摘要且错误不变」写明「摘要状态为失败，且含有解析阶段」，但 `tests/diagnostics_statement_test.rs` 的 `parse_error_keeps_error_text_and_summary_fails` 只断言 severity、错误文本与 SQLSTATE。实测该子句在当前实现下已成立（解析失败摘要有且仅有 `parse` 阶段），因此这是断言补强，不是产品行为变更：补断言前后都是 GREEN。

补强与旧表收敛互不干扰：断言消费的是进程内摘要（`db.diagnostic_summaries()`），旧表收敛改的是 stderr 的打印点。

### 5. 验证预算

定向测试：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`（该文件按既有约定串行执行，因为它的用例经 CLI 子进程操纵进程级 `RTSQL_PROFILING`）。

全量：整个 change 只跑一次，放在最后一个 Iteration，按 M19 分批串行，算法写在 `tasks.md` 的 T3。禁止一次无过滤的 `cargo test`，禁止全量批次使用默认并行度。

## Risks / Trade-offs

- [错误路径上的 stderr 变少，可能被误读为「诊断没工作」] → 诊断（新摘要）的行为完全不变，失败摘要照常产生；规格把两者的输出面分开写。
- [执行阶段失败与执行阶段之前失败的判定容易混] → 两种失败各有独立场景与独立的 CLI 复现形状（规划期重复列名 / 执行期键冲突），实现侧以基点的两个打印点为准。
- [「对齐基点」被读成「所有错误都不输出」] → 场景「执行阶段失败仍输出旧的三段计时表」守住另一半。
- [全库 clippy 曾有独立基线问题] → 本 change 的通过条件不扩大成仓库级 clippy 清零。

## Migration Plan

无磁盘迁移。无格式迁移。无新的磁盘字段。默认配置下不产生新 stderr。回滚是放弃 change 分支，见 `proposal.md`。

实施顺序：先改旧表的打印点并让 RED 转 GREEN，再补「含有解析阶段」的断言，最后一次分批全量。前一步未通过时不开始下一步。

## Open Questions

无。旧表的输出条件、判定面与断言缺口已在本文件写死。若实施发现某个返回路径无法在不改变错误文本与 SQLSTATE 的前提下对齐基点，停止并回到计划，不另选打印条件。
