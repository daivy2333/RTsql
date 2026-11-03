# CLI 主命令路径未建立 profiling scope：`RTSQL_PROFILING` 无三段计时表，索引扫描查询 panic

- Status: closed
- Filed: 2026-10-30
- Source: Act Response 的 Issue 候选 1 与「与计划的偏差（Deviations from Plan）」第 1 条，`openspec/changes/2026-10-28-ms44-unified-diagnostics/iterations/001-statement-phases/000-initial.md`（Act Response Status `reported`；该 change 尚未归档，位于 worktree `/home/daivy/projects/RTsql-changes/2026-10-28-ms44-unified-diagnostics`，主仓 `openspec/changes/` 下无此目录）；Recorder 资格核验时在同一入口补证 panic 表现
- Environment: WSL2（Linux 6.18.40.1-microsoft-standard-WSL2 x64）；被测二进制为 change worktree `/home/daivy/projects/RTsql-changes/2026-10-28-ms44-unified-diagnostics` 的 `target/debug/rtsql`（工作树 HEAD `5d0d212`，构建产物时间 2026-10-30 18:43）；主仓 master `3686a05` 无构建产物，master 同构性由代码比对确认，未在 master 实跑

## 缺陷描述

- 预期（规格）：`openspec/specs/pipeline-stage-decomposition/spec.md` 的 Requirement「三段顶层计时观测」写明「profiling 启用时 SHALL 输出 parse / plan / execute 三段顶层耗时；现有 profiling 开关机制（默认关闭）不变」，其 Scenario 为「开启 profiling 后执行一条 SQL → 输出包含 parse、plan、execute 三段各自的耗时」。开关是环境变量 `RTSQL_PROFILING`（`src/profiling.rs::is_profiling_enabled` 只判断该变量是否存在），对用户是进程级、入口无关的开关。
- 实际一（运行观察，2026-10-30）：主命令形态 `rtsql <db> "<sql>"` 在 `RTSQL_PROFILING=1` 下 stderr 为空，不含三段计时表。同一环境下 `rtsql import <db> t <csv> --csv` 正常输出三段表（parse / plan / execute 三行 + Total 行）。同一环境变量在两个 CLI 入口上行为不同。
- 实际二（运行观察，2026-10-30）：主命令形态在计划含索引扫描时直接崩溃。`RTSQL_PROFILING=1 rtsql <db> "SELECT * FROM u WHERE a = 1"`（`u` 有 `PRIMARY KEY` 列 `a`）退出码 101，stderr 为 `thread 'main' panicked at src/profiling.rs:22:20: cannot access a task-local storage value without setting it first`，stdout 为空——没有 SQL 结果，也没有业务错误文案。
- 位置：
  - `src/cli/mod.rs::run_sql`（846 行起，由主命令分发 `src/cli/mod.rs:545` 调用）自行分段执行：`parse_stage(sql)` → `plan_stage_routed(db, sql, stmt, false, ...)`（964 行，profiling 参数字面量 `false`）→ `execute_stage_routed(db, plan, false, &routed)`（991 行，同为 `false`）。该路径不经 `pipeline::execute_with_params`。
  - `src/pipeline.rs::execute_with_params`（116 行）是 `src/` 内唯一建立 profiling scope 的位置（127-133 行 `with_profiling_scope`）；`with_profiling_scope` 无其他调用点。
  - `src/profiling.rs::record_time`（21-25 行）在 22 行无 scope 保护地调用 `PROFILING_DATA.with(...)`；task-local 未设置时该调用 panic。
  - `src/executor/index_scan.rs::next`（69 行取 `is_profiling_enabled()`，74 行 `record_time("index_manager_search", ...)`）与 `src/executor/index_scan_all.rs`（86 行取开关，90 行 `record_time("index_manager_search_all", ...)`）的守卫只看环境变量，与「当前任务是否已建立 scope」无关。
- 根因（Confirmed）：profiling scope 的所有权只在 `pipeline::execute_with_params`；CLI 主命令自建的执行分段不经过该入口，因此既得不到旧表输出，也在执行器侧 `record_time` 调用时因 scope 缺失而 panic。执行器侧守卫只看环境变量，是 panic 的直接条件；scope 缺失是根因。
- 边界：
  - 未设 `RTSQL_PROFILING` 时主命令行为正常（开关为 false → 不调用 `record_time`，也不打表）。本缺陷只在设置该变量时显现。
  - 崩溃只发生在计划含索引扫描的语句（`IndexScanExecutor` / `IndexScanAllExecutor`）；无索引扫描的语句只是缺少旧表输出，不崩溃。
  - 主命令形态「是否应当输出旧的三段计时表」属产品决策（Act Response 已明确指出）。规格文本未把入口写死，本条只登记事实与影响，不替产品决定该表该不该接主命令；panic 表现不依赖该决策。
- 与本次 change 的关系（change 范围外）：`git diff master -- src/profiling.rs src/executor/ src/cli/` 为空（master `3686a05` 与 change 分支 `5d0d212` 之间无差异）；`git diff master -- src/pipeline.rs` 的改动限于 `execute_with_params`、`execute_inner` 与新增的 `execute_staged`，未触及 `plan_stage_routed` / `execute_stage_routed`。本缺陷是既有缺陷，不是本次 change 引入的回归。

## 影响

- 用户可见（已发生，2026-10-30 实测）：主命令形态开启 `RTSQL_PROFILING` 后无任何三段计时表输出——为性能排查打开该变量的用户在最常用入口得不到输出，而规格把该开关写成入口无关的进程级开关。
- 用户可见（已发生，2026-10-30 实测）：同一开关下主命令执行索引扫描查询直接 panic 退出（退出码 101），既无结果也无错误文案。崩溃发生在执行器内部，不是用户输入非法所致；用该变量做性能排查的会话被崩溃打断。
- 未受影响（已实测）：`rtsql import` 路径（`src/cli/lifecycle.rs` 经 `db.execute_sql` → `pipeline::execute_with_params`）输出正常三段表；库 API `Database::execute_sql` / `execute_with_params` 同样不受影响。
- 潜在（Inferred，代码路径确定但未实跑）：库 API `PreparedStatement::execute`（`src/database.rs:1071-1078`）把 `is_profiling_enabled()` 直接传给 `pipeline::execute_stage`，同样不建立 scope；该路径上的 `record_time`（`src/pipeline.rs:639/649/711/721/950/985`，守卫是传入的 `profiling` 参数）与索引扫描执行器一样会在设置该变量时 panic。CLI 不使用预编译语句入口，本条未由 CLI 实跑证实。
- 影响面限于设置了 `RTSQL_PROFILING` 的会话；默认配置（不设该变量）下主命令行为不变。

## 事件记录

2026-10-30，Recorder 资格核验，真实 CLI，Linux x86_64，被测二进制为 change worktree `5d0d212` 的 `target/debug/rtsql`：

1. 读 `src/cli/mod.rs::run_sql`、`src/pipeline.rs`、`src/profiling.rs`，确认 scope 只由 `execute_with_params` 建立，CLI 主命令自建分段不经该入口。
2. `RTSQL_PROFILING=1 rtsql <db> "CREATE TABLE t(a INT, b INT)"` → 退出码 0，stderr 为空，无 `Time (µs)` 表头，stdout `{"affected_rows":0}`。
3. `RTSQL_PROFILING=1 rtsql <db> "SELECT * FROM u WHERE a = 1"`（`u(a INT PRIMARY KEY, b INT)`，已插入 2 行）→ 退出码 101，stderr 为 `panicked at src/profiling.rs:22:20: cannot access a task-local storage value without setting it first`，stdout 为空。
4. `RTSQL_PROFILING=1 rtsql import <db> t <csv> --csv` → 退出码 0，stderr 含三张计时表，每张含 parse / plan / execute 行与 Total 行。

- 触发条件：`RTSQL_PROFILING` 存在 + 主命令形态 + 计划含索引扫描。
- 根因：Confirmed（代码路径与运行观察一致；panic 位置由 stderr 指名 `src/profiling.rs:22`）。
- 检测：退出码 101 与 stderr panic 文案；业务错误文案不存在，无法按 SQL 错误语义理解。
- 恢复：进程已退出。本崩溃发生在只读查询的执行器内部，未见数据面影响；崩溃后数据库文件完整性未检查，此处不作声明。

## 处置

- 未排期。本条不在 `2026-10-28-ms44-unified-diagnostics` change 的范围内。
- 两部分处置不同：
  - panic 部分（`src/profiling.rs::record_time` 的 scope 安全性，或 CLI 主命令路径补 scope）属必须修复的缺陷，无需产品决策。
  - 三段表部分（主命令形态是否输出旧表）属产品决策：`openspec/specs/pipeline-stage-decomposition/spec.md` 的「三段顶层计时观测」未把入口写死，需产品侧决定主命令是否接入旧表；若决定接入，可与 MS44 的统一诊断门面一并处理（该 change 的 `proposal.md` 需求 8 只要求「`RTSQL_PROFILING` 旧表保持」，已在 import 入口验证）。
- 2026-10-30 登记。Status 为 open。任务列表尚未指向本条。
- 2026-10-30 当前状态：**待用户裁定，未在任何变更内承载**。该 Cycle 的 Plan Review（Iteration 001，Review Result `accepted`）已把本候选独立复核为变更范围外的既有行为，判「不在本 change 承载」——修复需要新增「CLI 主命令接入旧计时表」的产品语义，超出 MS44 已批准范围，属产品决策。另有一处约束冲突需先裁定：MS44 的 Preserve 项写明「`src/profiling.rs` 零改动；`record_time` 仍只在旧 scope 内调用（scope 外 panic 条件未变）」（同 Review 的 Preserve 核对第 3 条），因此让 `record_time` 具备 scope 安全性、或让 CLI 主命令补 scope，都会改动本 change 已批准的 Preserve 语义；是否解除该 Preserve 由用户裁定后再决定承载方式（另起 change 或纳入后续里程碑）。本条保持 open，不在本流程内修复、也不在本流程内关闭。
- 2026-11-03 关闭。Status 为 closed。处置指针：fixed → openspec/changes/archive/2026-11-03-ms45-main-command-profiling/。`RTSQL_PROFILING` 已设置时，主命令不中止；进入执行阶段的语句输出旧三段表；未设置时不输出该表。预编译语句不在该次验收内。`run_sql` 在该变量已设置时，对该语句的 plan 与 execute 调用 `with_profiling_scope`，并在 execute 返回后输出旧表；plan 返回错误时不输出该表。未设置时不调用 `with_profiling_scope`。

## 证据

- `openspec/changes/2026-10-28-ms44-unified-diagnostics/iterations/001-statement-phases/000-initial.md`（worktree `/home/daivy/projects/RTsql-changes/2026-10-28-ms44-unified-diagnostics`，HEAD `5d0d212`）：Act Response 的「Experience Candidates」Issue 候选 1 与「与计划的偏差」第 1 条（Act Response Status `reported`）；同文件记录 Plan 的 Current-State Evidence 曾推断主命令形态产生三段表，与实际代码矛盾。
- `openspec/specs/pipeline-stage-decomposition/spec.md`（工作树 `5d0d212`）：Requirement「三段顶层计时观测」及其 Scenario。
- 代码事实（工作树 `5d0d212`）：`src/cli/mod.rs:545`（主命令分发）、`src/cli/mod.rs::run_sql`（846 起）、`src/cli/mod.rs:964`（`plan_stage_routed(..., false, ...)`）、`src/cli/mod.rs:991`（`execute_stage_routed(..., false, ...)`）、`src/pipeline.rs::execute_with_params`（116 起）与 `src/pipeline.rs:127-133`（唯一 `with_profiling_scope`）、`src/profiling.rs:21-25`（`record_time`）、`src/profiling.rs:56`（`is_profiling_enabled`）、`src/executor/index_scan.rs:69-79`、`src/executor/index_scan_all.rs:86-94`、`src/database.rs:1071-1078`（`PreparedStatement::execute`）。
- 运行观察（2026-10-30，Linux x86_64，change worktree `5d0d212` 的 `target/debug/rtsql`）：见「事件记录」四条命令与实际输出。
- 变更范围比对：`git diff master -- src/profiling.rs src/executor/ src/cli/` 输出为空；`git diff master -- src/pipeline.rs` 的改动限于 `execute_with_params` / `execute_inner` / 新增 `execute_staged`。master 为 `3686a05`，change 分支为 `5d0d212`。
- 未知项：主命令形态「应当」输出旧表这一点，规格文本与产品意图均未写明；崩溃后数据库文件完整性未检查；`PreparedStatement::execute` 路径的 panic 未实跑。
- 2026-11-03 关闭所依据的位置：`openspec/changes/archive/2026-11-03-ms45-main-command-profiling/iterations/000-main-command-profiling/000-initial.md`（Act Response Status `reported`；Plan Review Result `accepted`）；`openspec/changes/archive/2026-11-03-ms45-main-command-profiling/iterations/001-full-suite/000-initial.md`（Act Response Status `reported`；15 批第一次都是退出码 0、failed 0；Plan Review Result `accepted`）；`openspec/specs/pipeline-stage-decomposition/spec.md` 的 Requirement「三段顶层计时观测」。
