# Iteration 000 / Cycle 000: CLI 壳全链路

## Plan Context

- Status: ready
- Iteration: 000-cli-shell
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4, T5, T6
- Depends on: None
- Stable baseline: `rtsql <db> <sql>` 端到端稳定（名称解析、四格式渲染、列名表头、退出码分类、close 语义）；新增测试全绿；585 既有测试零修改
- Verification boundary: `cargo test --all` 全绿 + clippy 0 warning + fmt 0 diff + `openspec validate` PASS
- Diagnostic boundary: `src/cli/`、`src/main.rs`、`src/parser/planner/query.rs::get_plan_output_columns`、`src/pipeline.rs::value_to_json` 可见性
- Deferred tasks: None（MS10-T02/T03/T04/T05 是后续 change，不在本 change）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 全部 5 个 Requirement（R1 入口与主命令 / R2 名称解析 / R3 列名表头 / R4 输出格式 / R5 多语句护栏）
- Excluded scope: 文件锁、信号停机、格式头、多语句分片执行、生命周期子命令、REPL、PlanCache 接入、网络 server 路径、锁/密钥的实际产生逻辑

**Objective**

`rtsql <db> <sql> [--format ...]` one-shot 命令全链路可用：参数解析 → 名称解析 → 开库 → 三阶段执行（列名表头 + 多语句拒绝）→ 四格式渲染 → close → 分类退出码。binary 从硬编码 server demo 变为 CLI 工具。

**Background**

MS10 应用层主轨首个 change（roadmap 用户批准 2026-09-06）。R18 分析确认现状是"嵌入式库 + 演示性 PG 服务"，无 CLI。四项执行决策用户同日批准：三阶段组合、JOIN 表头纳入、`$RTSQL_HOME/db/` 裸名解析、JSON columns+rows 形状。

**Current Baseline**

- revision `709c85d`（master；工作树代码与之一致，未提交改动均为文档）。
- 585 tests pass / clippy 0 / fmt 0 / 14 specs validate PASS（2026-09-05）。
- `src/main.rs` 为 27 行硬编码 demo；`src/cli/` 不存在；Cargo.toml 无 clap。

**Current-State Evidence**

（Plan 调查确认，Act 不需要回读 Explorer；来源标注供追溯）

1. **三阶段 API**：`parse_stage(sql) -> Result<Vec<Statement>, String>`（`src/pipeline.rs:42`）、`plan_stage(db, sql, stmt, profiling) -> Result<PhysicalPlan, String>`（`pipeline.rs:56`）、`execute_stage(db, plan, profiling) -> Response`（`pipeline.rs:96`）均为 `pub`。`PhysicalPlan: Clone`（`plan.rs` derive）。`plan_stage` 内部对可缓存语句 `plan_cache.put`（`pipeline.rs:80-82`），CLI 不 get 不影响正确性。
2. **列名提取**：`PlanBuilder::get_plan_output_columns(&self, &PhysicalPlan) -> Vec<String>` 为 `pub(crate)`（`src/parser/planner/query.rs:23`）——CLI 在 crate 内（`pub mod cli`）可直接调 `PlanBuilder::new().get_plan_output_columns(&plan)`。JOIN 三臂返回空 Vec（`query.rs:35-38`）；`JoinConfig/SemiJoinConfig/AntiJoinConfig` 均有 `output_columns: Vec<OutputColumn>`（`src/executor/plan.rs:304/359/375`），`OutputColumn.column` 即列名（`plan.rs:287`）。行组装严格按 `output_columns` 顺序（`src/executor/join.rs:112-122`）。
3. **表头文本语义**：别名优先（`query.rs:409/414`）；无别名聚合 = `result_column_name()`（`COUNT(*)`→`count_star`、`AVG(price)`→`avg_price`，`src/executor/aggregate.rs:48-56`）；普通列小写。
4. **执行结果**：`Response`（`src/network/protocol.rs:34-39`）`QueryResult{rows: Vec<Vec<serde_json::Value>>}` / `AffectedRows(u64)` / `Error{message}` / `Pong`。`value_to_json`（`pipeline.rs:684`，私有）处理 NaN/Inf→Null。
5. **多语句**：`parse_stage` 返回全部语句；`execute_sql`/`execute_in_tx` 内 `statements.first()` 截断（`pipeline.rs:337/237`）——CLI 自行检测 `len() > 1`。
6. **open/close**：`Database::open(&Path) -> Result<Self>`（`src/database.rs:28`）：静默创建（`FileStorage` create(true)）、页对齐校验、WAL redo 失败显式 Err（K05）。`close()` = 全量 checkpoint + WAL 截断（`database.rs:177`）。drop 不 close 重开见空 schema（`database.rs:169-172` 官方注释）。
7. **WAL 路径**：主文件 `with_extension("wal")`（`src/wal/writer.rs:27`）。
8. **运行时**：现 `#[tokio::main]` 多线程；库内 `spawn_blocking` + WALBuffer flush loop + DataScan 预取（默认关）——CLI 沿用同形态即可。
9. **测试入口**：binary 集成测试用 `env!("CARGO_BIN_EXE_rtsql")` + `std::process::Command`（零新 dev-dependency）；TTY 检测 `std::io::IsTerminal`（rustc 1.90 stable）。既有测试不引用 `main.rs`。

**Relevant Code**

- `src/main.rs`（重写）、`src/lib.rs`（+`pub mod cli`）、`src/cli/{mod,resolve,render}.rs`（新建）、`src/parser/planner/query.rs:35-38`（三臂）、`src/pipeline.rs:684`（可见性）、`Cargo.toml`（clap 4）、`tests/cli_test.rs`（新建）。

**Critical Path**

`main()` → `cli::run()` → clap 解析（错→退出 2）→ `resolve_db_path`（HOME 缺失→退出 1）→ `Database::open`（失败→退出 1）→ `parse_stage`（len>1→close+退出 3；parse 错→close+退出 3）→ `plan_stage`（错→close+退出 3）→ `get_plan_output_columns` → `execute_stage`（`Response::Error`→close+退出 3）→ 渲染到 stdout（IO 错→退出 1）→ `close()`（失败→退出 1）→ 退出 0。

**Implementation Guidance**

- TDD 顺序 T1→T2→T3→T4→T5（T4 消费前三者；T5 是 T4 的 open 错误分支收口）；T6 收尾回归。
- clap 用 derive 宏（`Parser`）；`--format` 自定义校验失败即退出 2（clap `value_enum`）。
- 渲染函数签名建议：`fn render(kind: OutputKind, columns: &[String], payload: &QueryPayload) -> String`，`QueryPayload` 覆盖 rows/affected 两种。
- 环境变量单测注意 `cargo test` 默认并行：涉及 env 的用例集中在一个 `#[test]` 串行函数内或用锁。
- 集成测试断言 close 语义：执行 INSERT 后进程退出，重新以 CLI 查询该行可见且 `.wal` 文件为空内容/最小长度（checkpoint 截断的等价可观察断言）。

**Behavioral Change**

- binary 行为：server demo → one-shot CLI（破坏性替换，用户已批准）。
- 查询输出首次带真列名（含 JOIN）。
- 多语句从静默截断变为显式拒绝（CLI 路径；`execute_sql` 的库内行为不变）。
- `get_plan_output_columns` JOIN 臂返回值变化（唯一消费方派生表列注册不受实际影响，回归守卫）。

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R3 | `src/parser/planner/query.rs::get_plan_output_columns` | JOIN 三臂返回空 Vec | 返回 `output_columns` 列名 |
| T2 | R2 | `src/cli/resolve.rs`（新） | 不存在 | 裸名/路径 → PathBuf |
| T3 | R4/R3 | `src/cli/render.rs`（新） | 不存在 | 四格式纯函数渲染 |
| T4 | R1/R5 | `src/cli/mod.rs`（新）+ `src/main.rs` + `src/lib.rs` + `src/pipeline.rs::value_to_json` + `Cargo.toml` | main 为 server demo | CLI 编排 + 退出码 + 多语句护栏 |
| T5 | R2 | `src/cli/mod.rs`（open 分支）+ `tests/cli_test.rs` | demo 吞错 | open 失败分类退出 1；不存在路径建库 |
| T6 | 全部 | 验证命令 | — | 全量回归门 |

**Task Contracts**

### T1: JOIN 列名表头臂

- Requirement/Scenario: R3 / "JOIN 查询表头"、"JOIN 臂补齐不影响既有派生表路径"
- Depends on: None
- Targets: `src/parser/planner/query.rs::PlanBuilder::get_plan_output_columns`
- Current behavior: `PhysicalPlan::Join(_)|SemiJoin(_)|AntiJoin(_)` 臂返回 `Vec::new()`（`query.rs:35-38`）
- Required behavior: 返回 `node.output_columns.iter().map(|c| c.column.clone()).collect()`
- Required changes: 合并臂拆开或保持合并（三个节点类型不同，需分别匹配后映射同一逻辑）；`OutputColumn` 已含 `column: String`
- Preserve: 其余全部臂行为不变；`pub(crate)` 可见性不变；派生表列注册（`query.rs:81`）语义不变
- Forbidden: 不改 `OutputColumn`/`JoinConfig` 结构；不动执行器；不改其他 planner 模块
- Test witness: RED——新单测构造含 JOIN 的 SELECT（经 `PlanBuilder` 完整 plan），断言 `get_plan_output_columns` 返回投影列名（现在返回空 → RED）；命令 `cargo test --lib get_plan_output_columns`（或所在模块单测名）
- GREEN condition: 该单测通过且 `cargo test --all` 全绿（585 零修改）
- Verification: `cargo test --all`；失败意味着 JOIN 臂映射错误或破坏派生表路径
- Stop when: 发现 JOIN plan 节点实际无 `output_columns` 字段（与调查矛盾），或派生表路径确有 JOIN 输入场景（影响面扩大）

### T2: 名称解析

- Requirement/Scenario: R2 / 4 个 Scenario
- Depends on: None
- Targets: 新建 `src/cli/resolve.rs`
- Current behavior: 不存在
- Required behavior: `resolve_db_path(arg: &str) -> Result<PathBuf, String>`：含 `/` → `PathBuf::from(arg)`；否则 `RTSQL_HOME`（默认 `$HOME/.rtsql`）+ `db` + `<arg>.db`；`RTSQL_HOME` 与 `HOME` 均未设置 → Err
- Required changes: 新模块 + `mod.rs` 挂 `pub mod resolve`（或 mod 树内挂载）
- Preserve: 不创建目录（父目录缺失时由 open 报 IO 错）；含 `/` 判定用 `arg.contains('/')`（`Path::new(arg).components()` 含 `ParentDir`/`RootDir` 等价实现可接受）
- Forbidden: 不做 `~` 展开；不解析 `file://` 等 scheme；不静默 mkdir
- Test witness: RED——新单测 4 例（裸名默认拼接 / `RTSQL_HOME` 覆盖 / `./x.db` 与 `/abs/x.db` 原样 / 双 env 缺失 Err）；env 用例置于单 `#[test]` 内串行（避免并行污染）
- GREEN condition: 4 例通过
- Verification: `cargo test --lib resolve`；失败即解析规则错
- Stop when: 需要引入目录创建语义（超出"打开即用"现状，返回 Plan 决策）

### T3: 渲染四态

- Requirement/Scenario: R4 / 5 个 Scenario + R3 表头文本
- Depends on: None
- Targets: 新建 `src/cli/render.rs`
- Current behavior: 不存在
- Required behavior: 纯函数渲染——table（表头+分隔线+行，列宽=最大字节宽度，NULL 空串，Bool true/false）；json（`{"columns":[..],"rows":[[..]]}`，DML `{"affected_rows":N}`）；csv（RFC 4180：`,`/`"`/`\n` 触发引号包裹、引号翻倍，NULL 空字段）；tsv（`\t` 分隔，字段内 `\t`→`\\t` `\n`→`\\n` `\r`→`\\r` `\\`→`\\\\`，NULL 空字段）
- Required changes: `OutputKind` enum + render 函数族；输入用 `serde_json::Value` 行（与 Response 一致）
- Preserve: 渲染函数无 IO、无 env 读取、无 clock；NaN/Inf 已在上游转 Null（`value_to_json`），渲染层遇非有限数字按 Null 处理防御
- Forbidden: 不改 `Response` 类型；不引入新渲染依赖（手写 CSV/TSV）
- Test witness: RED——逐格式单测：table 对齐含表头；json 形状与 DML 变体；csv 边界值 `a"b,c`→`"a""b,c"`；tsv 内嵌 `\t`/`\n` 还原；NULL/Bool 全格式
- GREEN condition: 单测全过
- Verification: `cargo test --lib render`；失败即格式语义错
- Stop when: 需要支持流式渲染（大结果集内存策略）——超出本契约，返回 Plan

### T4: CLI 编排与退出码

- Requirement/Scenario: R1 / 4 个 Scenario + R5 多语句拒绝
- Depends on: T1、T2、T3
- Targets: 新建 `src/cli/mod.rs`；重写 `src/main.rs`；`src/lib.rs` +`pub mod cli`；`src/pipeline.rs::value_to_json` → `pub(crate)`；`Cargo.toml` +`clap = "4"`（derive feature）
- Current behavior: `main.rs` 打开 `rtsql.db` 建 test 表起 server（`:9876`）
- Required behavior: 按 Critical Path 编排：clap（`rtsql <db> <sql> [--format table|json|csv|tsv]`，无参数/非法值退出 2）→ resolve → open（失败退出 1）→ parse（len>1 → close+退出 3"每次一条语句"文案；parse 错退出 3）→ plan（错退出 3）→ `get_plan_output_columns` → execute（Error 退出 3）→ 渲染（`IsTerminal` 定默认；stdout IO 错退出 1）→ close（失败退出 1）→ 0
- Required changes: `ExitStatus` enum 含 Success/General/Usage/Sql/Locked/InvalidKey（后两者留位无产生路径）+ ExitCode 映射；错误信息进 stderr；结果进 stdout
- Preserve: `execute_sql`/`execute`/`execute_in_tx` 库内行为零变化；网络模块零变化；`plan_stage` 传 `profiling: false`
- Forbidden: 不实现文件锁/密钥逻辑；不接信号处理；不查 PlanCache；不实现多语句分片；不改 pipeline 编排器本体
- Test witness: RED——`tests/cli_test.rs`（`env!("CARGO_BIN_EXE_rtsql")`）：①建表+插行+SELECT 退出 0 且 stdout 含列名表头与数据；②无参数退出 2；③`SELEC typo`/不存在的表退出 3；④双 INSERT 分号语句退出 3 且行数 0（再查证零执行）；⑤管道（`Stdio::piped`）下默认输出合法 JSON（serde_json 解析断言 columns/rows）；⑥`--format csv` 边界值转义；⑦INSERT 输出 affected_rows 退出 0；⑧退出后再查数据可见（close 落盘）
- GREEN condition: 集成测试全过；`cargo test --all` 全绿（585 零修改）
- Verification: `cargo test --test cli_test` + `cargo test --all` + `cargo clippy -D warnings` + `cargo fmt --check`
- Stop when: 三阶段组合暴露契约缺口（如 `plan_stage` 需要的 sql key 语义与 CLI 传参冲突）、或 585 回归出现非预期失败

### T5: 打开不存在路径与 open 失败路径

- Requirement/Scenario: R2 / "打开不存在路径创建空库"
- Depends on: T4
- Targets: `src/cli/mod.rs` open 错误分支 + `tests/cli_test.rs` 补用例
- Current behavior: T4 后 open 失败已归退出 1（分支存在）；未验证不存在路径成功路径
- Required behavior: 不存在路径静默建库 + CREATE TABLE 成功退出 0；页不对齐文件（预写垃圾字节）→ stderr 错误退出 1
- Required changes: 仅测试（编排分支已由 T4 建立；若 T4 未覆盖页对齐错误分支则补齐 open Err → 1 映射）
- Preserve: 不加"已创建新库"提示（用户默认假设，记录于 proposal 风险区）
- Forbidden: 不加交互确认；不自动 mkdir -p
- Test witness: GREEN 基础上补两例：新裸名库 CREATE TABLE→SELECT 往返退出 0；`tempfile` 写 100 字节垃圾文件后 `rtsql <path>` 退出 1
- GREEN condition: 两例通过
- Verification: `cargo test --test cli_test`
- Stop when: 垃圾文件触发的不是 StorageError 而是进程 panic（需修 panic→错误码映射，属 T4 范围回补）

### T6: 全量回归与收尾验证

- Requirement/Scenario: 全部 Requirement 的回归门
- Depends on: T1-T5
- Targets: 无代码变更
- Current behavior: —
- Required behavior: `cargo test --all` 全绿（585+新增）、`cargo clippy -D warnings` 0、`cargo fmt --check` 0、`openspec validate 2026-09-06-ms10-t01-cli-shell` PASS
- Required changes: 无（发现回归按各自契约返修）
- Preserve: 无
- Forbidden: 不为通过回归修改既有测试
- Test witness: 命令输出（每项 ≤20 行决定性片段）+ 退出码记入 Act Response
- GREEN condition: 四命令全过
- Verification: 同上
- Stop when: 回归失败定位到本 change 未触及模块（基线变化，返回 Plan）

**Invariants**

1. 585 既有测试零修改全绿。
2. `execute_sql`/`execute`/`execute_in_tx` 及网络路径行为零变化。
3. lib 公开 API 只增不改（`pub mod cli`；`value_to_json` 是 crate 内可见性调整，非公开 API）。
4. CLI 进程任何退出路径（0/1/2/3）在库已打开时都执行 `close()`（clap 用法错误发生在 open 前，无库可关）。
5. 渲染纯函数无 IO/env。
6. 退出码 4/5 仅枚举留位，无产生路径。

**Non-goals**

见 Cycle Scope Excluded scope；另：不加"新库已创建"提示、不做 `~` 展开、不流式渲染、不接 PlanCache。

**Acceptance**

| # | 条件 | 映射 |
|---|---|---|
| A1 | `rtsql <db> <sql>` 成功执行并输出带列名结果，退出 0，close 后 WAL 截断 | R1/S1 + T4⑧ |
| A2 | 用法错误退出 2 且不开库 | R1/S2 + T4② |
| A3 | SQL 错误（parse/plan/execute）退出 3 | R1/S3 + T4③ |
| A4 | 退出码枚举含 4/5 留位 | R1/S4 + T4 |
| A5 | 裸名/RTSQL_HOME/路径/不存在路径四类解析正确 | R2 + T2/T5 |
| A6 | 普通与 JOIN 查询表头为真实列名（别名/聚合语义正确） | R3 + T1/T3/T4① |
| A7 | 四格式渲染语义正确（转义/NULL/Bool/DML） | R4 + T3/T4⑤⑥⑦ |
| A8 | 多语句显式拒绝退出 3 且零执行 | R5 + T4④ |
| A9 | 585 既有测试零修改全绿 + clippy/fmt/validate 干净 | 全部 + T6 |

**Verification**

`cargo test --all`、`cargo test --test cli_test`、`cargo test --lib`（cli 单测）、`cargo clippy -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t01-cli-shell`。输出记入 Act Response；不需要持久化 Evidence。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 1-9（本会话 Explorer + Plan 复核，代码 `709c85d` 实读） |
| Design | PASS | design.md D1-D7；四项用户决策 2026-09-06 |
| Iteration Plan | PASS | tasks.md 单 Iteration + 平衡审计（单一可验收结果） |
| Cycle Scope | PASS | initial，5 Requirement 全覆盖，排除项明确 |
| Task Contracts | PASS | T1-T6 六契约，每项含 Target/见证/GREEN/停止条件 |
| Traceability | PASS | RTM 见下 |
| Verification | PASS | A1-A9 映射到具体测试命令与断言 |

**Persisted Evidence**

- Mode: none

全部验证命令低成本可重跑；Act Response 承载命令+决定性输出+退出码足够支撑 Review。

- Budget: 本 Cycle 最多 5 个文件（含 README）；change 合计 ≤20；单文件 ≤500 行且 ≤256 KiB。

**Risks and Notes**

- 打开不存在路径静默建库不提示（用户默认假设；proposal 风险区已记）。
- open 失败退出码 1（tasks.md 退出码表未覆盖此类；proposal 已记，docs 收尾对齐）。
- WSL2 下集成测试进程 spawn 开销可接受（单测进程 ~585 基线已含大量 `#[tokio::test]`）。
- 非实质选择留给 Act：`src/cli/` 内文件拆分粒度、clap derive 细节、table 渲染分隔样式（` | ` 与 `+---+` 均可，测试断言列名与数据存在而非装饰符）。
- T4 测试⑧"close 后 WAL 截断"断言采用等价可观察形式（重开查询可见 + WAL 文件最小化），避免依赖内部实现细节。

**RTM**

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 入口与主命令 | S1-S4 | D1/D3/D4/D7 | T4 | 000 | `src/cli/mod.rs::run` | cli_test ①②③⑧ | None | Covered |
| R2 名称解析 | S1-S4 | D6 | T2/T5 | 000 | `src/cli/resolve.rs` | resolve 单测 4 例 + cli_test 新库/垃圾文件 | None | Covered |
| R3 列名表头 | S1-S4 | D2 | T1 | 000 | `query.rs::get_plan_output_columns` | T1 单测 + cli_test ①（含 JOIN 用例） | None | Covered |
| R4 输出格式 | S1-S5 | D5 | T3/T4 | 000 | `src/cli/render.rs` | render 单测 + cli_test ⑤⑥⑦ | None | Covered |
| R5 多语句护栏 | S1 | D7 | T4 | 000 | `src/cli/mod.rs`（parse 后判定） | cli_test ④ | None | Covered |

（R3 的 cli_test ① 需含一个 JOIN 查询表头用例——列入 T4 测试① 断言范围。）

## Act Response

- Status: reported

**Implemented**

T1-T6 全部完成（Cycle 000-initial，Plan Context ready，2026-09-06 用户批准后执行）。

- T1：`get_plan_output_columns` 拆出 Join/SemiJoin/AntiJoin 三臂，返回 `output_columns` 列名（or-pattern 绑定类型不一致，按契约"合并臂拆开"方案）。RED（返回空 Vec）→ GREEN。
- T2：新建 `src/cli/resolve.rs::resolve_db_path`：含 `/` 原样；裸名 → `RTSQL_HOME`（默认 `$HOME/.rtsql`）+ `db/` + `<name>.db`；双 env 缺失 Err。4 场景单测（env 用例集中于单 `#[test]` + EnvGuard 防 panic 泄漏）。
- T3：新建 `src/cli/render.rs`：`OutputKind` + `QueryPayload::{Rows,Affected}` + 纯函数 `render`。table 列宽对齐 / json `{"columns":..,"rows":..}` 与 `{"affected_rows":N}` / csv RFC4180 / tsv 反斜杠系转义；NULL 空串、Bool true/false。8 单测逐格式断言。
- T4：新建 `src/cli/mod.rs`（clap derive 入口 + `ExitStatus` 六态枚举含 Locked/InvalidKey 留位 + 编排）；重写 `src/main.rs`（27 行 server demo → `cli::run()` 委托）；`lib.rs` +`pub mod cli`；`Cargo.toml` +clap 4 derive。Critical Path 全实现（open 失败→1；parse/plan/execute 错→3；多语句护栏→3 且零执行；stdout IO 错→1；close 失败→1）。`tests/cli_test.rs` 12 用例真二进制验证（60s 防挂起守卫；每测试独立 TempDir + `RTSQL_HOME` 注入）。
- T5：`tests/cli_test.rs` 补 2 用例——不存在裸名库静默建库 + 建表往返（退出 0，文件落盘）；100 字节垃圾文件 open 失败 → stderr 报错退出 1（编排分支 T4 已建立，无需产品代码改动）。
- T6：全量回归四命令全过（见 Verification Evidence）。

**Changed Files and Symbols**

| 文件 | 变更 |
|---|---|
| `src/parser/planner/query.rs` | `get_plan_output_columns` Join/SemiJoin/AntiJoin 三臂返回真列名；+1 单测 `test_get_plan_output_columns_join` |
| `src/cli/resolve.rs`（新） | `resolve_db_path` + 2 单测（4 场景） |
| `src/cli/render.rs`（新） | `OutputKind`/`QueryPayload`/`render`/`render_rows`/`render_table`/`render_delimited`/`csv_escape`/`tsv_escape`/`value_to_text` + 8 单测 |
| `src/cli/mod.rs`（新） | `ExitStatus`（含 `From<&ExitStatus> for ExitCode`）、`CliArgs`/`FormatArg`（clap derive）、`run`/`execute_command`/`run_sql`/`emit`/`kind`/`emit_stdout`/`emit_stderr` |
| `src/main.rs` | 整体重写：`#[tokio::main] async fn main() -> ExitCode { cli::run().await }` |
| `src/lib.rs` | +`pub mod cli` |
| `src/pipeline.rs` | `value_to_json` 私有 → `pub(crate)`（仅可见性，无行为变化） |
| `Cargo.toml` / `Cargo.lock` | +`clap = { version = "4", features = ["derive"] }` |
| `tests/cli_test.rs`（新） | 12 集成测试（真二进制） |

**Deviations from Plan**

1. **T4 测试按引擎实际行形状校准**：Plan 的 T4④ 测试 witness 未约定列投影行为；实现中发现引擎扫描路径（DataScan/IndexScan/IndexScanAll 执行器）恒返回全 schema 行、不做投影列裁剪（`src/executor/data_scan.rs:77-81`、`src/executor/index_scan.rs:27-31,71-87`，执行器不接收投影参数）。测试改用全 schema 投影查询（`SELECT id, name ...`），④ 的"零执行"与 ⑥ 的"CSV 转义 e2e"见证意图完全保留。由此暴露的 IndexScan 投影子集表头错位记入 Remaining Issues（详见该节，超出本 change 范围，不擅自扩面）。
2. **`value_to_json` 无 CLI 调用点**：契约预期"CLI JSON 渲染复用"；实际 `Response::QueryResult.rows` 已是 `serde_json::Value`（`pipeline.rs:394` 在管线内转换完毕），渲染层直接消费。可见性变更按契约落实（`pub(crate)`），语义零变化。
3. **DML 在 table/csv/tsv 的形状**：D5 仅规定 JSON 的 `{"affected_rows":N}`；table/csv/tsv 实现为单列 `affected_rows` 表头 + 数值行（与行集输出同构，自描述）。契约 T3"drops affected_rows 逐格式断言"的授权范围内选择。
4. **close 失败与既有错误并存的优先级**：D7 未覆盖"SQL 错 + close 失败"组合；实现为 close 失败仅在原状态 Success 时覆盖为 General(1)，否则保留原始错误状态（更相关的错误优先，数据已由 WAL 兜底）。
5. **集成测试 env 隔离**：`run_cli` 向子进程注入 `RTSQL_HOME=<TempDir>` 并预建 `db/` 子目录（产品代码按契约不建目录）；同时覆盖 R2 的 RTSQL_HOME 覆盖场景。
6. T2/T3 的 RED 以 `todo!()` stub 观察（编译型语言新函数的 RED 机制），行为接线为 GREEN 步骤。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: pass（T1-T6 契约逐项达成；Invariants 1-6 全部成立——608 既有+新增测试全绿且既有测试零修改；库内 `execute_sql`/`execute`/`execute_in_tx` 与网络路径零变化；lib 公开 API 仅增 `pub mod cli`；任何已开库退出路径执行 `close()`；渲染纯函数无 IO/env；退出码 4/5 无产生路径）
- Full diff reviewed: pass（`git diff` 修改文件 5 个 + 新增 5 个，与 Change Surface 完全一致，无计划外修改；跨任务交互检查——T1 表头臂被 T4 编排消费、T2 解析被 T4 消费、T3 渲染被 T4 两种 Response 分发消费、T5 复用 T4 open 分支，均经 cli_test 端到端覆盖）
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1（`resolve_db_path` 对"已设置但为空串"的 `RTSQL_HOME` 视为空基目录，产生相对路径 `db/<name>.db`；未在契约场景内，记录不改）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| 全量测试 | `cargo test --all` | `TOTAL passed: 608 failed: 0`（基线 585 + lib 新增 11 + cli_test 12，既有测试零修改） | PASS |
| cli 集成 | `cargo test --test cli_test` | `test result: ok. 12 passed; 0 failed` | PASS |
| lib 单测（T1-T3） | `cargo test --lib`（按名过滤） | T1 `1 passed`；T2 `2 passed`；T3 `8 passed`（各自先 RED 后 GREEN） | PASS |
| Lint | `cargo clippy --all-targets -- -D warnings` | `Finished` 0 warning，退出码 0 | PASS |
| 格式 | `cargo fmt --check` | 无 diff，退出码 0 | PASS |
| OpenSpec | `openspec validate 2026-09-06-ms10-t01-cli-shell` | `Change ... is valid`，退出码 0 | PASS |

**Persisted Evidence**

None required（Plan Mode: none；全部验证命令低成本可重跑，本 Response 承载命令+决定性输出+退出码）

**Experience Candidates**

None

**Remaining Issues**

1. **IndexScan/IndexScanAll 投影子集表头与全 schema 行错位（引擎既有不一致，非本 change 引入）**：`IndexScanNode.columns` 存投影列（`query.rs:297` `extract_columns(&select.projection)`），而 `IndexScanExecutor`/`IndexScanAllExecutor` 恒反序列化全 schema 行（执行器不接收投影参数，`src/executor/index_scan.rs`）；CLI 真表头使该错位对 `SELECT name FROM t WHERE id = <pk>` 类查询可见（T4 期间实测复现：`columns:["name"]` vs 2 字段行）。Spec R3 的场景全部为全 schema 投影，A1-A9 不受影响；DataScan 路径自洽（columns=全 schema）。方向候选：planner 侧让 IndexScan/IndexScanAll 节点 columns 携带全表 schema（元数据层，执行器不消费，`planner_test::test_select_by_pk` 仅断言全 schema 形态）或 CLI 侧对齐。交 Plan Review 分类（新证据 / Plan 调查遗漏），如需纳入范围由 Plan 创建后继 Cycle。

**Commit or Diff Reference**

- 工作树未提交（用户触发 commit；与仓库惯例一致）
- 基线 revision：`709c85d`；变更面：修改 5 文件（+43/-28 行）+ 新增 `src/cli/{mod,resolve,render}.rs`、`tests/cli_test.rs`

## Plan Review

- Review Result: accepted

**Findings**

1. **[NEW-EVIDENCE，非阻塞]** IndexScan/IndexScanAll 投影子集表头与全 schema 行错位——引擎既有不一致，本 change 的真表头使其可见。Plan Review 独立实机复现（2026-09-06，`target/debug/rtsql`，RTSQL_HOME 指向临时目录）：`SELECT name FROM users WHERE id = 1` 输出 `{"columns":["name"],"rows":[[1,"Alice"]]}`（表头 1 列、行 2 字段）；对照无 WHERE 的 DataScan 路径 `SELECT name FROM users` 输出 `{"columns":["id","name"],"rows":[[1,"Alice"]]}`（自洽——引擎所有扫描执行器均不做投影、恒返回全 schema 行，`IndexScanNode.columns` 是唯一携带投影子集的元数据，`query.rs:297`）。根因在 planner/执行器层，非本 change 引入。**不能在当前 Cycle 修复**：执行器/投影变更违反 Invariant 2（库内行为零变化），CLI 侧表头对齐（按行宽补 `col<N>` 或改用全 schema）是未授权的新设计决策。处置：记录 finding，交后续 change/I 候选（方向二选一：planner 侧让 IndexScan 节点元数据携带全 schema，或引擎侧实现真投影——后者是正确终态但触及执行器契约）。注意该错位命中最高频查询形态（PK 点查 + 子集投影），agent/CLI UX 影响实际，建议 MS10 内优先安排后续 change。
2. **[Minor，同族]** 引擎整体无投影：`SELECT name` 返回 id+name 两列（既有行为，DataScan 表头如实反映）。与 finding 1 同属"投影缺失"家族，留待同一后续决策。
3. **[Minor]** `resolve_db_path` 对空串 `RTSQL_HOME`（已设置但为空）视为空基目录产生相对路径（Act 已记录；契约场景外，不改）。
4. **[Minor]** `value_to_json` 提为 `pub(crate)` 后无 CLI 调用点（`Response::QueryResult.rows` 在管线内已转换完毕）——变更惰性但无害，与 Plan 预期偏差已被 Act 如实记录（Deviation 2）。

**Deviation Classification**

- Deviation 1（T4 测试按引擎实际行形状校准）= NEW-EVIDENCE（引擎无投影是计划时未确认的事实；测试见证意图零损失，④ 零执行与 ⑥ CSV e2e 完整保留）+ 本 review finding 1 的来源。
- Deviation 2（value_to_json 无调用点）= ACT-DEVIATION（非实质，惰性变更保留）。
- Deviation 3（DML table/csv/tsv 单列形状）= 契约授权内的非实质选择（D5 已定 csv/tsv 同值单字段；table 同构合理）。
- Deviation 4（close 失败与既有错误的优先级）= PLAN-OMISSION（D7 未覆盖组合场景）× 非实质（Act 选择更相关错误优先，数据由 WAL 兜底，语义合理且已记录）。
- Deviation 5/6（测试 env 隔离、todo!() RED 机制）= 非实质实现选择。

**Acceptance Gaps**

None。A1-A9 全部验证（见 Evidence）。

**Convergence**

N/A（首次 Review；无 Acceptance gap 可比较）

**Evidence**

| 检查项 | 结果 |
|---|---|
| 代码与 Change Surface 一致 | PASS——`git diff` 修改 7 文件（Cargo.toml/lock、lib.rs、main.rs、query.rs、pipeline.rs）+ 新增 `src/cli/{mod,resolve,render}.rs`、`tests/cli_test.rs`，与 Plan Change Surface 逐项对应，无计划外修改（`.claude/docs/tasks.md`、`references/spec.md` 为 change 前已存在的文档改动，非本 Act 产物） |
| 既有测试零修改 | PASS——`git diff HEAD -- tests/` 为空；608 = 585 基线 + lib 新增 11（T1 1 + T2 2 + T3 8）+ cli_test 12 |
| 全量测试（新鲜重跑） | PASS——`cargo test --all` 汇总 608 passed / 0 failed，退出码 0 |
| clippy / fmt / validate（新鲜重跑） | PASS——`cargo clippy --all-targets -- -D warnings` 0 warning；`cargo fmt --check` 0 diff；`openspec validate 2026-09-06-ms10-t01-cli-shell` valid |
| A1/A6 实机 | PASS——表头/数据/退出 0 实测；JOIN 表头 `["id","total"]` 实测正确；别名聚合 `["cnt","avg_id"]`（cli_test 断言） |
| A5 实机 | PASS——裸名经 RTSQL_HOME 解析、新库静默创建、垃圾文件退出 1（cli_test 12 用例全绿） |
| A8 多语句 | PASS——cli_test④ 退出 3 + 再查仅剩种子行（零执行） |
| Invariants 1-6 | PASS——逐项核对代码：ExitStatus 六态含 4/5 留位无产生路径（mod.rs:18-52）；任何已开库路径 close()（mod.rs:106-115）；渲染纯函数无 IO/env（render.rs）；库内路径零变化（diff 仅可见性） |
| finding 1 复现 | 见 Findings 1（PK 点查 `columns:["name"]` vs 行 `[1,"Alice"]`；DataScan 路径自洽） |

**Follow-up Decision**

A1-A9 全部满足且无阻塞 finding——accept。finding 1 无法在当前执行契约内修复（触及 Invariant 2 或需新设计决策），且属引擎既有缺陷而非本 change 引入，不构成返工理由；按规则记录 finding 交后续。**后续建议**（需用户决定，本 Review 不自动登记）：finding 1+2 作为 I 候选登记 `openspec/specs/improvements/spec.md`，或在 MS10 轨道内排一个独立小 change（候选名：index-scan-header-consistency；两种方向见 Findings 1）。Minor findings 3/4 记录不改。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（单 Iteration change；Iteration 000 完成，change 可按正常流程收尾归档）
