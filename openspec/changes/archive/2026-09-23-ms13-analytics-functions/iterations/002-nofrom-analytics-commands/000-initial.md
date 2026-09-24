# Iteration 002 / Cycle 000: no-FROM 与分析薄命令

## Plan Context

- Status: ready
- Iteration: 002-nofrom-analytics-commands
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T9, T10, T11, T12, T13
- Depends on: Iteration 000（类型底座——stats 日期列 min/max 形态、no-FROM 类型字面量组合消费其 Value 变体）与 Iteration 001（函数族——no-FROM `SELECT now()` 探测场景、date_trunc 分桶与 CLI stats 无直接耦合但全量收口依赖前序全绿）
- Stable baseline: `SELECT 1+1` 无 FROM 可达（单行、拒绝面点名）；`rtsql stats/sample/profile` 三命令四态输出可用；全 change 收口（全量/clippy/fmt/validate + change 结构自检）
- Verification boundary: `tests/no_from_select_test.rs` 全绿 + cli_test 三命令新增组全绿 + T13 全量验证记录
- Diagnostic boundary: `src/parser/planner/query.rs`（no-FROM 分支）、`src/executor/`（SingleRow 节点/执行器 + plan/pipeline 接线）、`src/cli/{mod,lifecycle}.rs`
- Deferred tasks: None（本 Iteration 为 change 收尾）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 决策 4、DA3/DA10/DA11、design D13/D14/D16；Iteration 000/001 全部既有面（Preserve 边界同源——表达式项表头语义、render 四态契约、退出码分类、六子命令分发）
- Excluded scope: strftime/to_date 族（change Out of Scope）；REPL/分发（MS14）/加密（MS12）；窗口函数；`ORDER BY <表达式文本>`

**Objective**

无 FROM 的 `SELECT <表达式项>` 经虚拟单行输入产出恰一行结果（常量/算术/标量函数/CASE/COALESCE/CAST/类型字面量组合），拒绝面显式点名；`rtsql stats/sample/profile <db> <table>` 三命令经既有查询路径拉数后 CLI 侧计算输出，遵循 `--format` 四态与既有退出码分类；全 change 全量收口。

**Background**

tasks.md MS13-T03（I035 no-FROM SELECT，MS10-T04 S5 见证原拟 SQL 曾因不可达修订）+ MS13-T02 CLI 侧（R18 主题 7，决策 4「三命令一次做全」）。I035 于 MS10-T05 登记为 improvements 后由本 change 承接（promoted）。前两 Iteration 已交付类型底座与函数/分桶（Reviews accepted 2026-09-23）。

**Investigation Facts**

- Current Baseline: Iteration 001 最终 Act Response——10 源文件 + 4 测试文件（详单见其 Changed Files），全量 **1027 tests / 0 failed / 2 ignored**（987 基线 + 40 新增；既有校准 2 处见 group-by-expression R2 校准段）、clippy/fmt/validate 全 0；Plan Review accepted（十项偏差全部非阻塞闭环，2026-09-23）。工作区未提交（对照基线 7364bc9 + MS09 收尾 docs 增量）。
- Current-State Evidence（2026-09-23 实读，Iteration 001 后现状）:
  - **no-FROM 入口**：`build_query`（query.rs:448）→ `build_from_clause_with_projection`（:240）在 `from.is_empty()` 时返回 `PlanError::MissingField("FROM clause")`（query.rs:249）——`SELECT 1` 当前报此错（proposal 探针：`Missing required field: FROM clause`）。子查询检测循环（:456-493）在 FROM 构建之前对 `Expr::Subquery` 项递归 `build_query`；`extract_select_body`（:454）为 Query body 提取点，no-FROM 分支置于其后可统一裁决全部 no-FROM 形态（含子查询上下文——`building_subquery` 下同样生效，内层 no-FROM 子查询经既有 SubqueryEval 消费面自然可达）。
  - **SELECT 项编译通路**：`build_expression`（expression.rs:232）已含 TypedString/BinaryOp（Interval 分流先于数值臂）/CASE/COALESCE/CAST/注册名 Function 全部臂（Iteration 000/001 扩展）；Identifier 臂经 `self.tables` / `join_column_layout` 解析——无表注册时 `ColumnNotFound`（spec R2/S3「列不存在类错误」天然成立）；`Expr::Subquery` 无臂 → 既有 `UnsupportedExpression` 兜底（no-FORM 下子查询项经该通道拒绝，非 spec 白名单形态）。
  - **聚合项检测**：`is_aggregate_expr`（query.rs 聚合检测循环 :560-611 前置使用）——no-FROM 分支需自行对聚合项点名拒绝（spec R2/S2 `SELECT COUNT(*)`）。
  - **SingleRow 接线面**：`PhysicalPlan` 枚举（plan.rs，现含 Projection/Aggregate 等 20+ 变体）；`create_executor_from_plan`（pipeline.rs:460）为单一递归函数，`execute_stage`/`execute_stage_in_tx`/`execute_in_tx` 三调用点共享——一个新匹配臂覆盖全部执行路径；`execute_executor`（pipeline.rs:425）的 `Response::QueryResult` 由「无 AffectedRows」兜底产出——SingleRow 自动走查询路径，无需响应分发臂；`get_plan_output_columns`（query.rs :80-121 match）需 SingleRow → `vec![]` 臂；`extract_column_indices`（pipeline.rs:815）需补臂（D13 记 unreachable，补空臂防御）。
  - **ProjectionNode/ProjectionExecutor**（MS11-T01 Iter001）：`ProjectionItem { expr, name }` 逐行求值，表头 = item 名（别名/表达式文本既有语义）；空行 `vec![]` 上 ConstantExpression/FunctionExpression/CASE 等求值正常（不索引行）；ColumnExpression 按索引取行——空行下不可达，因 no-FORM 分支中列引用编译期即 ColumnNotFound（D13 结论，构成安全性质）。
  - **CLI 分发**：`Command` enum（cli/mod.rs:87-124，六生命周期子命令 New/List/Schema/Dump/Restore/Import）+ `execute_command` match（:144-159）；`--format` 为 global arg（:78-79）经 `kind(format)` → `render` 四态（`list` 先例 lifecycle.rs:90-94）；子命令优先于同名裸库（既有分发语义）。主命令臂 `execute_main_command`（:162）不走子命令路径。
  - **lifecycle 模板**：`schema`（lifecycle.rs:102-140）——`resolve_db_path` → exists 检查（缺失 `ExitStatus::General` exit 1，沿用）→ `execute_command_inner` 两阶段信号优雅停机 → work 闭包内执行。`select_all_rows`（:216-260）——`SELECT * FROM {quote_ident(t)}` 经 parse_stage/plan_stage/execute_stage 三 stage 取 `Response::QueryResult { rows }`（`Vec<Vec<serde_json::Value>>`）；**注意其错误面映射为 General（dump 语义）**——stats/sample/profile 的表不存在错误须映射 exit 3（spec R4），应经 `sql_failure_status`（lifecycle 既有 helper，restore/import 调用点 `in_transaction=false` 先例），新函数自带映射、不复用 select_all_rows 的错误臂。
  - **列类型与 schema 序**：`catalog.scan_tables()` / `scan_columns(table)` + `columns.sort_by_key(column_index)`（schema 同源）——列类型供 stats 数值/非数值判别与 profile 类型渲染；`SELECT *` 恒等投影保证行形状 = schema 列序（MS10-T01 R6，select_all_rows 注释记载）。
  - **依赖**：rand 0.8 已在 Cargo.toml:19（reservoir sampling 无新依赖）。
  - **测试入口先例**：datetime_function_test / group_by_expr_test 的 `run_cli` + TempDir `RTSQL_HOME` 隔离夹具（本 change 前两 Iteration 建立）；cli_test 现 65 用例（MS10-T05 生命周期组先例）——三命令用例入 cli_test 与先例一致。
- Code and Critical Path: planner no-FROM 分支（query.rs build_query 前置）→ SingleRow 计划节点（plan.rs）+ 执行器（executor/ 新模块）→ pipeline 接线（create_executor_from_plan / extract_column_indices）→ CLI 三命令（cli/mod.rs 分发 + lifecycle.rs 计算）→ render 四态。引擎执行面零改动（SingleRow 为新叶子节点，纯加性）。

**Implementation Guidance**

- T9 先行（引擎面独立可验）；T10→T11→T12 同构递进（三命令共用「resolve → execute_command_inner → catalog 列类型 → 拉取 → 计算 → render」骨架，T10 建立骨架后 T11/T12 复用）；T13 收尾。
- no-FROM 分支置于 `extract_select_body` 之后、子查询检测之前：先拒绝面检查（通配符/WHERE/GROUP BY/HAVING/ORDER BY/LIMIT/聚合项逐项点名），再逐项 `build_expression` 编译（列引用自然 ColumnNotFound、子查询项自然 UnsupportedExpression 兜底），最后包 `ProjectionNode { input: SingleRow }`。
- SingleRowExecutor 恰产出一行 `vec![]` 后 Done；`PhysicalPlan::SingleRow` 无负载字段（D13）。
- 三命令计算在 work 闭包内完成（json 行 + catalog 列类型）；数值判别以 catalog `ColumnType` 为准（Int/Float 数值族）；Date/Timestamp 的 min/max 直接 json 字符串字典序（DA5 定宽格式字典序 = 时间序）；Bool min/max false<true。
- p50/p90/p99：数值列升序，最近邻秩 `ceil(p/100 × N)`（1-based）；p50 在 N 偶数时取中间双值平均（DA10）；N=0（空表）全 null。
- reservoir sampling：标准算法（前 N 直接收取，其后以 i/N 概率换出）；随机源用 `rand::thread_rng` 即可（spec 要求「行集可不同」，不要求可复现）。
- `--top` 上限语义：K > 20 视为用法错 exit 2（「上限 20」取合法域上界读法，用户审计本 Cycle 时可否决改 clamp）。

**Behavioral Change**

- 当前：无 FROM SELECT 一律 `Missing required field: FROM clause`；无 stats/sample/profile 子命令（clap 未知子命令 exit 2）。
- 目标：no-FROM 表达式查询单行可达 + 拒绝面点名（非 MissingField 文案）；三分析命令可用（输出契约见 Task Contracts）；`SELECT *`/WHERE/聚合等 no-FROM 形态显式拒绝。
- 接口语义：`PhysicalPlan` 加性变体 `SingleRow`（新节点，20+ 变体穷尽匹配编译面同步补臂——预计 get_plan_output_columns / extract_column_indices / create_executor_from_plan / 其他 `PhysicalPlan` 穷尽 match 点）；`Command` enum 加性三变体；无新错误变体（拒绝面走 `PlanError::ParseError` 点名文案，exit 3 经既有 SQL 失败面）。

**Task Contracts**

### T9: no-FROM SELECT 虚拟单行

- Requirement/Scenario: no-from-select R1（单行可达）、R2（拒绝面）、R3（WITH-FORM 零回归与算术解锁回归锚）；设计 D13
- Depends on: None（Iteration 000/001 既有面）
- Targets: `src/executor/plan.rs`（`PhysicalPlan::SingleRow` 变体）、`src/executor/single_row.rs`（新，`SingleRowExecutor`）+ `src/executor/mod.rs` 导出、`src/pipeline.rs`（`create_executor_from_plan` SingleRow 臂 + `extract_column_indices` 补臂）、`src/parser/planner/query.rs`（`build_query` no-FROM 分支 + `get_plan_output_columns` SingleRow 臂）、`tests/no_from_select_test.rs`（新）
- Current behavior: 无 FROM SELECT 一律 `PlanError::MissingField("FROM clause")`（query.rs:249）；`get_plan_output_columns`/`extract_column_indices`/`create_executor_from_plan` 无 SingleRow 形态
- Required behavior: `select.from` 为空时走 no-FROM 分支——(1) 拒绝面先行：投影项含通配符（Wildcard/QualifiedWildcard）→ 点名拒绝；`select.where`/`select.group_by`/`select.having`/`query.order_by`/`query.limit`（含 offset）非空 → 各自点名「not supported without FROM」；投影项含聚合（`is_aggregate_expr`）→ 点名拒绝（spec R2/S2 `SELECT COUNT(*)`）。(2) 可达面：其余投影项逐项经 `build_expression` 编译为顶层 `ProjectionNode`（输入 = SingleRow，恰产出一行空行）；表头 = 别名/表达式文本（既有表达式项语义）；行数恰 1。列引用经既有 `ColumnNotFound`（无需新文案）；子查询项经既有 `UnsupportedExpression` 兜底拒绝。`building_subquery` 上下文同样生效（内层 no-FROM 子查询自然可达，经既有 SubqueryEval 消费）。
- Required changes: 上述五点接线 + 测试——`tests/no_from_select_test.rs` ~8（`SELECT 1+1` 单行 `[[2]]` 表头 `1 + 1`；`SELECT upper('a') AS u` 别名表头；CASE 探针；类型字面量组合 `SELECT DATE '2024-01-01'`；拒绝面：`SELECT *` / `SELECT 1 WHERE 1=0` / `SELECT COUNT(*)` / `SELECT 1 ORDER BY 1` / `SELECT 1 LIMIT 1`；`SELECT nonexistent_col` ColumnNotFound；lib 直连与 CLI 面一致性抽检 1 例）
- Preserve: 含 FROM 全部既有行为零回归（路由/投影/JOIN/子查询/聚合/表达式项）；表达式项表头语义；`building_subquery` 抑制语义对含 FROM 子查询不变；多语句/事务语句分派零变化
- Forbidden: 不做 no-FROM 下通配展开或列引用可达化（ColumnExpression 对空行不可达是安全性质，勿加行数守卫）；不改 `build_from_clause_with_projection` 的 MissingField 文案本身（含 FROM 形态的既有错误面无消费方变化；no-FROM 形态不再到达该点）
- Test witness: no_from_select_test RED——`SELECT 1+1` 当前 `Missing required field: FROM clause` exit 3（断言 `[[2]]` 失败）；拒绝面用例断言点名文案（当前为 MissingField，断言失败）
- GREEN condition: 套件全绿 + 既有全量零修改
- Verification: `cargo test --test no_from_select_test` + lib `execute_sql` 直连一致性抽检，exit 0
- Stop when: no-FROM 形态与 CTE/子查询组合出现 spec 未记载且影响契约语义的解析形态（返回 Plan）

### T10: stats 命令

- Requirement/Scenario: cli-analytics R1（stats 输出契约）、R4（格式四态与错误面）；设计 D14
- Depends on: None（T9 引擎面与本命令无耦合；同 Iteration 骨架先行）
- Targets: `src/cli/mod.rs`（`Command::Stats { db, table }` + `execute_command` 分发臂）、`src/cli/lifecycle.rs`（`stats` 函数）、`tests/cli_test.rs`（stats 组）
- Current behavior: 无 stats 子命令（clap 未知子命令 usage exit 2）；无对应 lifecycle 函数
- Required behavior: `rtsql stats <db> <table>` 输出每列一行 `[column, type, row_count, null_rate, distinct, min, max, p50, p90, p99]`——row_count 列承载 spec R1 总行数（design D14 元组缺项，本契约补列，见 Risks and Notes）；null_rate 百分数 0–100（空表 100）；distinct = 非空值精确计数（HashSet）；min/max 全可比类型（Int/Float 数值序、Bool false<true、String/Date/Timestamp 定宽字典序），全 null 列 min/max null；分位数仅数值列（升序、最近邻秩 ceil(p/100×N) 1-based、p50 偶数双值平均），非数值列该格 null，空表全 null；render 四态（`--format` global）；表不存在 → `sql_failure_status` exit 3（点名差异：不复用 select_all_rows 的 General 映射）；库文件缺失沿用 schema 先例 General exit 1；经 `execute_command_inner` 信号优雅停机
- Required changes: 上述 + cli_test stats 组 ~5（数值列分布含 NULL 的全字段断言、日期列 min/max 日期形态 + 分位 null、空表 0/100/null、表不存在 exit 3、格式四态抽检 json/csv）
- Preserve: 既有六子命令分发与裸名冲突语义；render 既有契约；catalog 只读；引擎零改动
- Forbidden: 不做近似分位数/采样统计；不新增引擎 RANDOM/聚合内建；不动 select_all_rows 本体（dump 依赖其 General 语义）
- Test witness: cli_test stats 组 RED——实施前 `rtsql stats` 为 clap 未知命令 exit 2（断言 exit 0 + 输出失败）
- GREEN condition: stats 组全绿 + 既有 cli_test 零修改
- Verification: `cargo test --test cli_test stats`（或名称过滤），exit 0
- Stop when: catalog 列类型信息不足以支撑数值/非数值判别（返回 Plan）

### T11: sample 命令

- Requirement/Scenario: cli-analytics R2（sample 输出契约）、R4；设计 D14
- Depends on: T10（共用拉取/渲染骨架）
- Targets: `src/cli/mod.rs`（`Command::Sample { db, table, n: Option<usize> }`）、`src/cli/lifecycle.rs`（`sample` 函数 + reservoir sampling）、`tests/cli_test.rs`（sample 组）
- Current behavior: 无 sample 子命令
- Required behavior: `rtsql sample <db> <table> [N]`——N 缺省 10；N=0 → 用法错 exit 2（手动校验，clap 用法文案风格）；非整数 → clap 类型校验 exit 2；reservoir sampling（rand 0.8，thread_rng）；M ≤ N 输出全行；行形状与 `SELECT *` 一致（quote_ident + 恒等投影，列名 = schema 列序）；随机性合法（测试断言行数/列形状/行集 ⊆ 全行集，不断言具体行）；render 四态；表不存在 exit 3（同 T10 映射）
- Required changes: 上述 + cli_test sample 组 ~4（M>N 恰 N 行 + 列形状 + 子集断言、N=0 exit 2、非整数 exit 2、M<N 全行）
- Preserve/Forbidden: 同 T10（引擎零改动；不做种子化可复现采样）
- Test witness: sample 组 RED——实施前 clap 未知命令 exit 2
- GREEN condition: sample 组全绿 + 既有 cli_test 零修改
- Verification: `cargo test --test cli_test sample`，exit 0
- Stop when: rand 0.8 API 与调查记载不符（返回 Plan；低风险）

### T12: profile 命令

- Requirement/Scenario: cli-analytics R3（profile 输出契约）、R4；设计 D14
- Depends on: T10（共用骨架）
- Targets: `src/cli/mod.rs`（`Command::Profile { db, table, top: Option<usize> }`）、`src/cli/lifecycle.rs`（`profile` 函数）、`tests/cli_test.rs`（profile 组）
- Current behavior: 无 profile 子命令
- Required behavior: `rtsql profile <db> <table> [--top K]`——每列一行 `[column, type, min, max, top_k]`；min/max 语义同 T10；top_k 仅 String 列（k 默认 5；K > 20 → 用法错 exit 2），非空值频次降序、并列字典序升序、格式 `val(cnt), val(cnt), ...`（多次执行输出确定——计数后排序，非插入序）；NULL 不参与 top-k；非 String 列 top_k 格 null；render 四态；表不存在 exit 3（同 T10）
- Required changes: 上述 + cli_test profile 组 ~4（混合列画像、String 并列稳定 + 两次执行一致、数值列无 top-k、K 上限 exit 2）
- Preserve/Forbidden: 同 T10；不做数值列 top-k（DA10 裁定无分析价值）
- Test witness: profile 组 RED——实施前 clap 未知命令 exit 2
- GREEN condition: profile 组全绿 + 既有 cli_test 零修改
- Verification: `cargo test --test cli_test profile`，exit 0
- Stop when: 并列排序语义出现与 spec R3「并列取字典序最前者稳定」冲突的实现约束（返回 Plan）

### T13: 收尾全量验证

- Requirement/Scenario: datetime-type-system R7 / datetime-functions R4 / group-by-expression R2 / no-from-select R3 / cli-analytics R4 的零回归面聚合 + change 级收口；设计 D16
- Depends on: T9, T10, T11, T12
- Targets: 无产品代码（验证与自检任务）
- Current behavior: 全量 1027 基线（Iteration 001 收口值）
- Required behavior: 全量 `cargo test --no-fail-fast` 1027 既有 + 本 Iteration 新增全绿、零修改；`cargo clippy --all-targets -- -D warnings` 0；`cargo fmt --check` 0；`openspec validate` PASS；change 结构自检——tasks 状态与实际完成一致、specs/design 与已实现行为一致、Iteration 与 Cycle 文件齐全、Review Result 与流程状态一致
- Required changes: 无（验证记录写入 Act Response）
- Preserve: 既有全部测试零修改（含前两 Iteration 校准段既定 2 处）
- Forbidden: 不为收口新增排除路径或白名单
- Test witness: 本任务即见证（命令输出记入 Act Response）
- GREEN condition: 全部命令 exit 0 + 自检无差异
- Verification: 上述四命令 + 结构自检清单
- Stop when: 全量出现与本 Iteration 变更因果的既有测试失败且三次修复无效（Gate 6）

**Invariants**

- Iteration 000/001 全部 Preserve/Forbidden 边界延续（类型底座、函数族、分桶行为零回归；谓词日期守卫、写入 coerce、WAL/恢复零改动）。
- 引擎执行面零改动原则：stats/sample/profile 纯 CLI 拉取计算；SingleRow 为唯一引擎面新增（纯加性叶子节点）。
- 既有退出码分类不变（0/1/2/3/4/5）；render 四态契约不变。
- Evidence 预算与身份型证据禁令。

**Non-goals**

strftime/date_format/to_date；时区；窗口函数；REPL；分发（MS14）；加密（MS12）；数值列 top-k；近似分位数；种子化可复现采样；no-FROM 下通配展开或列引用可达化。

**Acceptance**

no-from-select R1–R3 全场景经 `tests/no_from_select_test.rs`（~8 用例）覆盖；cli-analytics R1–R4 全场景经 cli_test stats/sample/profile 新增组（~13 用例）覆盖；全 change 零回归面经 T13 全量收口（映射见 change tasks.md RTM 对应行：no-from-select 3 行 → T9/T13，cli-analytics 4 行 → T10–T12，各域 R7/R4 零回归行 → T13）。

**Verification**

- `cargo test --test no_from_select_test`：全绿。
- `cargo test --test cli_test`：新增组全绿 + 既有 65 零修改。
- `cargo test --no-fail-fast`：1027 既有 + 新增全绿、零修改。
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`：0。
- `openspec validate 2026-09-23-ms13-analytics-functions`：PASS。
- 探针抽检：`rtsql db "SELECT 1+1"`（单行 `[[2]]`）+ `rtsql stats <db> <table>` json 形态（Act Response 记录输出）。

**Gate 2 Readiness**

| 检查项 | 状态 | 证据 |
|---|---|---|
| 无 Missing requirement | PASS | change tasks.md RTM 对应行（no-from-select R1–R3 / cli-analytics R1–R4）→ T9–T12 全映射，T13 收口零回归行 |
| 无未批准 Simplified | PASS | 无 Simplification（决策 4 与 DA3/DA10/DA11 经 Gate 1 批准；`--top>20 → exit 2` 为本 Cycle 契约裁定，用户审计可否决） |
| 调查完整 | PASS | Investigation Facts：Iteration 001 accepted 现状（1027/0/2 + 静态 0）+ no-FROM 入口（query.rs:249/:448/:454）、接线五点（plan/pipeline/get_plan_output_columns/extract_column_indices/execute_executor 兜底）、CLI 分发（cli/mod.rs:87-159）、lifecycle 模板（schema/select_all_rows/sql_failure_status 差异点）、rand 依赖（Cargo.toml:19）行号级证据 |
| 设计闭合 | PASS | D13/D14/D16 无契约级 TBD；design D14 stats 元组缺 row_count 一处已由本 Cycle 契约补列闭合（见 Risks and Notes，非 Act 决定项） |
| 任务可执行 | PASS | T9–T13 契约各含 Targets/Current/Required/Preserve/Forbidden/witness/GREEN/Verification/Stop |
| 分轮合理 | PASS | change 级 3 Iteration 审计结论维持；002 为「便利面」单一成果 + 收尾任务 |
| 追踪完整 | PASS | RTM 链路齐（no-from-select/cli-analytics 域行） |
| 验证充分 | PASS | 每任务 RED→GREEN + Iteration 收口全量零修改（T13）+ 探针 |
| 无身份型证据工程 | PASS | Persisted Evidence none；目标行为观察 |
| 无需 Act 决定的实质未知项 | PASS | `--top` 上限语义、row_count 补列、distinct 非空计数、Bool 序、p50 偶数平均均已在本 Cycle 契约/Guidance 定值；子查询/CTE 组合形态有 Stop-when 定义 |
| 产物一致 | PASS | Cycle 契约与 design/tasks/specs 互检（本文件创建时）；`openspec validate` 于 Iteration 001 Review 后维持 PASS 状态，T9 实施后随 Gate 复验 |
| Persisted Evidence 契约 | PASS | Mode none（白名单四问全否） |
| 用户批准计划 | PASS | 用户批准「批准」原话记录于会话（2026-09-23，Iteration 001 Review 交付后）；stats row_count 补列与 `--top>20 → exit 2` 两项契约定值随计划一并获批 |

**Persisted Evidence**

- Mode: none
- 全部验证经 Act Response 承载（命令/决定性输出/退出码）；无白名单情形（可低成本重跑、无一次性环境、无 Issue 现场）。
- Budget: 不适用（none）。

**Risks and Notes**

- **design D14 stats 元组缺 row_count（PLAN-OMISSION，本 Review 前置记录）**：spec cli-analytics R1 要求输出「总行数」，design D14 的 9 元组 `[column, type, null_rate, distinct, min, max, p50, p90, p99]` 未承载——本 Cycle 契约以 `row_count` 列补列（每列一行重复承载，保持 render 单行集契约）。spec 未锁定行形状，无需 spec 修订；归档合并时按实现行为核对 spec 措辞。
- `--top > 20 → exit 2` 为契约定值（合法域上界读法）；用户如倾向 clamp 到 20，属一行语义调整，审计时否决即可（非 TBD——已定值，Act 不决定）。
- stats 全量拉取的大表性能为已裁定边界（proposal 决策 4、design R3 文档化），本 Iteration 不做上限。
- I041（resolve env 测试竞态）为既有登记项，全量运行偶发假失败沿用 Iteration 000 Review F3 用户裁定（复跑即绿，不新登记）。
- no-FROM 与 CTE 组合形态 spec 未覆盖——T9 契约 Stop-when 已定义（返 Plan），非 Act 决定项。

## Act Response

- Status: reported

**Implemented**

T9–T12 全部完成（RED→GREEN 逐任务见证）；T13 全量收口与 change 结构自检闭环：

- **T9 no-FROM SELECT 虚拟单行**：`PhysicalPlan::SingleRow`（unit 变体，无负载字段 design D13）+ 新 `src/executor/single_row.rs::SingleRowExecutor`（恰产出一行空行）；`src/parser/planner/query.rs` 新 `build_no_from_select` 方法置于 `extract_select_body` 之后、子查询检测之前——拒绝面点名（通配符 / WHERE / GROUP BY / HAVING / ORDER BY / LIMIT / OFFSET / 聚合项）+ 逐项 `build_expression` 编译并包 `Projection(SingleRow)`；`get_plan_output_columns` 加 `SingleRow → Vec::new()` 臂；`pipeline::create_executor_from_plan` 加 `SingleRow → SingleRowExecutor::new()` 臂；`pipeline::extract_column_indices` 加 `SingleRow → (empty, "")` 防御空臂；`executor::correlated::inject_correlated_values` 加 `SingleRow => {}` 穷尽匹配臂。
- **T10 stats 命令**：`Command::Stats { db, table }` + `lifecycle::stats`（共享 `resolve_existing_db` + `fetch_table_rows` + `render_rows` 骨架）；`compute_stats_rows` 纯函数——null_rate 二位小数四舍五入（空表 100，整数零除保护）、distinct 精确计数（HashSet<Value>）、min/max 按列类型可比分发（数值/字典序/Bool false<true/Date-Timestamp DA5 字典序=时间序）、p50 偶数双值平均 + p90/p99 最近邻秩；missing table 经 `sql_failure_status` exit 3（不复用 `select_all_rows` General 映射）、missing db 沿用 schema 先例 General exit 1。
- **T11 sample 命令**：`Command::Sample { db, table, n: Option<usize> }` + `lifecycle::sample`；`reservoir_sample` 标准算法（前 N 直接收取，其后 `j = rng.gen_range(0..=n+i)`，`j < n` 换入，rand 0.8 `thread_rng`），`M ≤ N` 短路直返全行；N=0 手动校验 exit 2、非整数由 clap 解析 exit 2。
- **T12 profile 命令**：`Command::Profile { db, table, top: Option<usize> }` + `lifecycle::profile`；`compute_profile_rows` 仅 String 列产出 top_k，NULL 不参与，频次降序 + 并列字典序升序（计数后排序而非插入序），格式 `v(c), v(c)`；`--top` 仅接受 1..=20，越界 exit 2。
- **T13 收尾全量验证与结构自检**：全量/clippy/fmt/validate 命令面见证见 Verification Evidence 表；change 结构自检——tasks T1–T13 状态与实际完成一致（T9–T13 状态行同步 `done`，见 Self-Review 修复记录）、specs/design 与已实现行为一致、Iteration 与 Cycle 文件齐全、Review Result 与流程状态一致。

**Changed Files and Symbols**

- 新增：`src/executor/single_row.rs`（`SingleRowExecutor` + 1 单测）、`tests/no_from_select_test.rs`（9 e2e + 1 lib 一致性）。
- 修改：
  - `src/executor/plan.rs`：`PhysicalPlan` 枚举加 `SingleRow`（unit 变体）。
  - `src/executor/mod.rs`：`mod single_row` + `pub use single_row::SingleRowExecutor`。
  - `src/executor/correlated.rs`：`inject_correlated_values` 加 `PhysicalPlan::SingleRow => {}` 穷尽匹配臂。
  - `src/parser/planner/query.rs`：`build_query` 入口 no-FROM 分流（`if select.from.is_empty() { return self.build_no_from_select(select, query) }`）；新方法 `build_no_from_select`（拒绝面逐项点名 + 空布局覆盖列引用编译 + 包 `Projection(SingleRow)`）；`get_plan_output_columns` 加 `SingleRow → Vec::new()` 臂。
  - `src/pipeline.rs`：import 加 `SingleRowExecutor`；`create_executor_from_plan` 加 `SingleRow` 臂；`extract_column_indices` 加防御空臂。
  - `src/cli/mod.rs`：`Command` 枚举加 `Stats`/`Sample`/`Profile` 三变体；`execute_command` 分发三臂。
  - `src/cli/lifecycle.rs`：新加 `resolve_existing_db` / `fetch_table_rows` / `select_all_sql` / `render_rows` / `stats` / `compute_stats_rows` / `json_number` / `round2` / `column_min_max` / `json_lt` / `json_gt` / `column_percentiles` / `sample` / `reservoir_sample` / `profile` / `compute_profile_rows` / `top_k_string` 共 ~400 行。
  - `tests/cli_test.rs`：末尾追加 stats（5 用例）+ sample（4 用例）+ profile（4 用例）三组，共 13 e2e。
- change 产物（当前 Cycle 修复，无产品代码与测试改动）：`openspec/changes/2026-09-23-ms13-analytics-functions/tasks.md`（T9–T13 状态行 `pending` → `done`）+ 本 Act Response 覆盖为含修复的完整当前快照。

**Deviations from Plan**

1. **no-FORM 列引用编译：空布局覆盖（Plan-Investigation-Misread）** — Plan Investigation Facts 记载「Identifier 臂经 `self.tables` / `join_column_layout` 解析——无表注册时 `ColumnNotFound`（spec R2/S3「列不存在类错误」天然成立）」，但实测 build_expression 的 Identifier 臂对未注册表名报 `PlanError::ParseError("Table '' not found")`，与契约点名的既有 `ColumnNotFound` 不一致。为达契约错误面零新文案要求，`build_no_from_select` 复用既有 MS09-T02 NLJ layout 机制（`self.join_column_layout = Some(Vec::new())` save/restore 配对与 NLJ 先例同型），空布局下 Identifier 臂自然 `ColumnNotFound`、CompoundIdentifier 臂自然 `TableNotFound`——无新错误变体、无新文案。修复后契约点名错误面（"Column not found"）经 e2e 验证。
2. **top_k 输出策略（Plan 边界补全）** — Plan 与 spec 未锁定 top_k 输出中频次的可见性（如多行并列频次相同的展示策略）；契约定值「频次降序 + 并列字典序升序」覆盖，标准 `sort_by(|a,b| bc.cmp(ac).then_with(|| a.cmp(b)))` 实现。`compute_profile_rows` 与 `top_k_string` 验证用 e2e 锁死确定性（twotie 表两次执行字节一致）。

**Blocker Handoff**

None required。

**Blocker Resolution**

（未发生阻塞。）

**Self-Review**

- Plan compliance: T9–T12 契约逐项覆盖（Targets/Preserve/Forbidden 无违反）；CLAP 入参与 stats/sample/profile 命名约定匹配既有子命令；多语句分片 (`;`) 对分析命令形态不变（分析命令为单 SQL 不受影响）；`building_subquery` 抑制语义对含 FROM 子查询不变（相邻 subquery_test 全绿）。
- Full diff reviewed: 本 Cycle 改动集中于 `src/cli/lifecycle.rs`（+475）、`src/parser/planner/query.rs`（+~120）、`src/pipeline.rs`（+2 arms + import）、`src/executor/{plan,mod,correlated}.rs`（+~20）；未触 engine catalog / buffer pool / WAL / MVCC / 索引 / 写路径。RESERVED_NAMES、ColumnType 枚举变体（Iteration 000 已扩展）、MS16 键位预检顺序、谓词日期守卫均不参与本次改动。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: (a) `compute_stats_rows` 的 Date/Timestamp min/max 按 DA5 字符串字典序——若格式假设失效（迄今 Iter 000-001 校准固定），min/max 仍反映字典序而非显式时间序（DA5 字典序 = 时间序已是设计结论，沿用）；(b) `column_percentiles` 将 Int 值经 f64 计算，i64 极大值（>2^53）有精度边界——分析命令全表拉取仅用于统计非生产 OLAP，符合改进项 I038 同类边界定位。
- 当前 Cycle 修复记录（Plan Review Follow-up Decision 步骤 1/2）：步骤 1——change `tasks.md` T9–T13 状态行 `pending` → `done`（仅状态行，无新测试；命令面验证结论已由 Review 采信）；步骤 2——本 Response 覆盖为含原实施与修复的完整当前快照，Status 恢复 `reported`；顺手更正 Review F1 计数表述——`no_from_select_test` 实为 8 CLI e2e + 1 lib = 9 个测试（原分解 4+4+1+1=10 把列引用同时计入拒绝面与独立项），更正为「单行可达 4 / 拒绝面 4（列引用计入其中）/ lib 一致性 1」。T13 GREEN 条件「自检无差异」就此闭环，Review Gap 1 关闭，待 Plan 复审终态。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T9 no-FROM e2e | `cargo test --test no_from_select_test` | `9 passed; 0 failed`（8 CLI e2e + 1 lib 一致性） | 单行可达 4 / 拒绝面 4（列引用计入其中）/ lib 一致性 1 | PASS |
| T10–T12 cli_test | `cargo test --test cli_test -- test_stats test_sample test_profile` | `13 passed; 0 failed` | stats 5 / sample 4 / profile 4 | PASS |
| 相邻回归 | `cargo test --test expression_e2e_test --test projection_expression_test --test planner_test --test pushdown_test --test scalar_function_test --test subquery_test --test datetime_type_test --test datetime_function_test --test group_by_expr_test --test projection_test` | 全部 0 failed | 表达式/投影/计划/下推/函数/子查询/日期/分桶/CLI | PASS |
| 全量 | `cargo test --no-fail-fast` | `passed=1050 failed=0`（基线 1027 + 本 Cycle 23 新增） | 全部 75+ bins | PASS |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` | 0 warnings（exit 0） | 全 workspace | PASS |
| 格式 | `cargo fmt --check` | 0 diff（exit 0） | 全 workspace | PASS |
| OpenSpec | `openspec validate 2026-09-23-ms13-analytics-functions` | `Change ... is valid` | change 结构 | PASS |
| 结构自检（当前 Cycle 修复） | `openspec validate 2026-09-23-ms13-analytics-functions` | `Change 2026-09-23-ms13-analytics-functions is valid` | tasks 状态行同步后的 change 结构（Follow-up Decision 步骤 1 复验） | PASS |
| 探针 | `rtsql probe "SELECT 1+1"` / `rtsql stats app m --format csv` | `{"columns":["1 + 1"],"rows":[[2]]}` / `column,type,...` 头形状 | CLI no-FORM 与 stats 输出 | PASS |

**Persisted Evidence**

None required（Mode none：全部验证可低成本重跑，无一次性环境与 Issue 现场）。

**Experience Candidates**

None。

**Remaining Issues**

None（I041 resolve env 测试竞态为已登记既有项，本次全量运行未复现假失败）。

**Commit or Diff Reference**

未提交（待用户触发）；对照基线 `7364bc9` + MS09 收尾 docs sync 增量（非本 Cycle 改动），本 Cycle 工作区增量为：

- 新增：`src/executor/single_row.rs`、`tests/no_from_select_test.rs`
- 修改：`src/{executor/plan.rs, executor/mod.rs, executor/correlated.rs, parser/planner/query.rs, pipeline.rs, cli/mod.rs, cli/lifecycle.rs}`、`tests/cli_test.rs`
- 当前 Cycle 修复：change `tasks.md` T9–T13 状态行 + 本 Act Response 快照覆盖（见 Self-Review 修复记录）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查覆盖本 Cycle 全部新增/修改面的当前实现（工作区即 Iteration 002 结果态）：`src/executor/single_row.rs` 全读、`src/executor/plan.rs` SingleRow 变体、`src/parser/planner/query.rs` no-FROM 分流（:467-469，置于 extract_select_body 之后、子查询检测之前，与契约一致）与 `build_no_from_select`（:1231-1333）及 `get_plan_output_columns` 臂（:118）、`src/pipeline.rs` 两臂（:786-789 / :955）、`src/executor/correlated.rs` 臂（:87）、`src/cli/mod.rs` 三变体与分发（:125-149 / :183-189）、`src/cli/lifecycle.rs` 新增函数（:609-1045 全读）、`tests/no_from_select_test.rs` 全读、`tests/cli_test.rs` stats/sample/profile 三组与 seed 夹具。核心核对结论：

- T9：拒绝面九项逐项点名（Wildcard/QualifiedWildcard/聚合项/WHERE/GROUP BY/HAVING/ORDER BY/LIMIT/OFFSET），文案均含 "not supported without FROM" 子串，e2e `expect_rejected` 同时断言 exit 3、点名子串存在、既有 MissingField 文案不存在——拒绝面不落入兜底得到负向锁定。空布局覆盖的 save/restore 配对在编译错误路径亦成立（闭包同步执行后无条件恢复）；`build_expression` Identifier 臂在 layout 未设时报 `Table '' not found`（expression.rs:276-278）、空布局 0 命中报 `ColumnNotFound`（expression.rs:271）——Dev 1 声明经源码核实成立（见偏差分类）。表头别名 `alias.value` 原始大小写与既有表达式项路径（query.rs:616）同语义。
- T10：stats 表头十列含 `row_count`（lifecycle.rs:701-715，契约补列）；空表 null_rate=100、min/max/分位数 null 零除保护（测试逐字段断言）；分位数仅 Int/Float 列、最近邻秩 `ceil(p×N/100)` 1-based、p50 偶数双值平均（测试 seed [10,20,30,NULL,50] 数学核验一致）；表缺失经 `sql_failure_status` exit 3、库文件缺失 General exit 1、`--format` csv/json 抽检断言齐全；三命令均经 `execute_command_inner` 接线信号优雅停机。
- T11：N=0 手动校验 exit 2 先于 db resolve；reservoir sampling 概率 n/(n+i+1) 正确（`gen_range(0..=(n+i))`，标准算法）；M≤N 短路直返全行；随机性断言合法（行数/列形状/子集，不断言具体行）。
- T12：`--top` 仅接受 1..=20、越界 exit 2；top_k 计数后排序（频次降序 + 并列字典序升序）确定性成立（两次执行 byte-identical 断言 + 精确串断言 `apple(2), zebra(2), mango(1)`）；NULL 不参与；非 String 列 top_k 格 null。
- Preserve/Forbidden：`select_all_rows` 本体与 dump General 错误语义未动（新 `fetch_table_rows`/`select_all_sql` 分立，:216-257 与 Iteration 001 前状态一致）；`build_from_clause_with_projection` 的 MissingField 文案未动（query.rs:252）；`expr_to_column_name` 等既有路由面零触碰；引擎执行面唯一新增为 SingleRow 叶子节点（纯加性，与不变量一致）。

非阻塞 finding：

- **F1（Minor，Act Response 文档精度）**：Changed Files 记 no_from_select_test「9 e2e + 1 lib 一致性」，实际为 8 CLI e2e + 1 lib = 9 个测试；Verification Evidence 分解「单行可达 4 / 拒绝面 4 / 列引用 1 / lib 一致性 1」合计 10 与「9 passed」不一致（列引用被同时计入拒绝面）。测试函数与 spec 场景映射逐条核对无缺口——纯记录精度问题，Act 已在覆盖 Response 时更正（8 CLI e2e + 1 lib = 9，列引用计入拒绝面），本复审核实闭环。
- **F2（Minor，RTM 映射归属）**：no-from-select R3「WITH-FORM 算术解锁」场景（R3/S1）的实际见证在 `tests/datetime_type_test.rs:364-381`（Iteration 000 已 accepted 套件，change 内覆盖成立），RTM 该行 Test Witness 记「no_from_select_test + 全量」——归属记录不精确，覆盖无缺口；随归档期语料库事项处理。
- **F3（Minor，沿用 Act Self-Review 披露）**：stats 的 Date/Timestamp min/max 依赖 DA5 定宽格式字典序=时间序；数值列 i64>2^53 值经 f64 比较存在精度边界——分析薄命令已裁定边界（决策 4；改进项 I038 同类定位），非阻塞，不要求修复。
- **F4（观察，非本 Iteration Acceptance 面）**：no-FROM 与子查询/CTE 组合（如 `WHERE id IN (SELECT 1)`）无 spec 场景亦无测试；契约 Stop-when 未触发、Acceptance 不要求，留作未来行为域扩展注记，不构成返工理由。

**修复复审（当前 Cycle 修复，Follow-up Decision 步骤 3）**

Gap 1 独立核验（本 Review 只读检查）：change `tasks.md` 13 个任务状态行全部 `done`（:7-:92 grep 实证）；Act Response Status `reported` 且含修复记录快照（F1 计数表述已同步更正——8 CLI e2e + 1 lib = 9，列引用计入拒绝面）；修复为纯 change 产物编辑（tasks.md 状态行 ×5 + Response 快照定点整合），无产品代码与测试改动，工作区其余部分与原审查基线一致。`openspec validate` 于修复后运行 exit 0（本会话新鲜，其后覆盖范围未变化，采信）。无新发现。

**Deviation Classification**

- Dev 1（no-FORM 列引用编译：空布局覆盖）→ **PLAN-INVALID**：Investigation Facts 记载「无表注册时 `ColumnNotFound` 天然成立」与实际行为不符——`build_expression` Identifier 臂在 layout 未设时对未注册表名报 `Table '' not found`（expression.rs:276-278，本次 Review 源码实核）。Act 实测发现后以既有 MS09-T02 NLJ layout 机制的空表形态（`join_column_layout = Some(Vec::new())`，save/restore 与 NLJ 配对同型）达成契约点名的 ColumnNotFound/TableNotFound 错误面——零新错误变体、零新文案、复用既有机制，为最小合规补救，正确。
- Dev 2（top_k 输出策略）→ **非偏差**：T12 契约 Required behavior（「频次降序、并列字典序升序、格式 val(cnt)...、多次执行输出确定——计数后排序」）与 spec R3 场景（并列按字典序、多次执行一致）已锁定该语义；Act 实现与契约一致，其描述属冗余复述而非计划外决定。
- design D14 stats 元组缺 row_count（Plan Context Risks and Notes 前置自记）→ **PLAN-OMISSION**：spec R1「SHALL 输出：总行数 + 每列…」未锁定行形状，Cycle 契约以 `row_count` 列补列闭合；实现表头含该列（lifecycle.rs:701-715 实核）。归档合并时按实现行为核对 spec 措辞（Risks and Notes 已记载，闭环）。

**Acceptance Gaps**

None。（原 Gap 1——T13「change 结构自检——tasks 状态与实际完成一致」未满足，T9–T13 状态行 `pending` 与实现不一致——经当前 Cycle 修复关闭，见修复复审。）其余 Acceptance 保持满足：no-from-select R1–R3 场景见证齐（R3/S1 见 F2 归属注记）；cli-analytics R1–R4 场景见证齐（13 用例）；T13 命令面收口（全量 1050 / clippy / fmt / validate）经采信成立；Iteration 002 零既有测试校准（Preserve 遵守）。

**Convergence**

Gap 1 与上一版 Review 比较：closed（状态行同步即为 reduced → closed，与上版复审指引预判一致）；无其他 gap，无 unchanged/expanded 项。

**Evidence**

- 代码独立审查：上述逐文件读取与行号级核对——Dev 1 两分支错误面（expression.rs:271 vs :276-278）、空布局 save/restore 错误路径配对、stats 表头/分位数/空表保护、sample 概率与边界、profile 排序确定性与 --top 校验、三命令错误面映射（exit 3/1/2）、Preserve/Forbidden 面（select_all_rows、MissingField 文案、表达式项表头语义）均源码级确认；测试 seed 数据与断言数学核验（score [10,20,30,NULL,50] → p50=25/p90=50；d 列字典序 min/max；tie-break apple<zebra）。
- 采信（覆盖范围未失效——只读基线检查：HEAD 仍 `7364bc9` 无新 commit，工作区 57 项与 Act Response「Commit or Diff Reference」一致，本 Review 未修改任何产品代码）：Act Response Verification Evidence 表——`no_from_select_test` 9 passed / cli_test stats+sample+profile 13 passed / 全量 `cargo test --no-fail-fast` passed=1050 failed=0 / `cargo clippy --all-targets -- -D warnings` 0 / `cargo fmt --check` 0 / `openspec validate` PASS / 两 CLI 探针（`SELECT 1+1` 单行 [[2]]、stats csv 表头形状）。
- spec 交叉核对：no-from-select 与 cli-analytics-commands 全部场景 → 测试函数映射逐条核对（结果见 Findings 与 Acceptance Gaps）。
- 修复复审采信与核验：见「修复复审」段——`openspec validate` exit 0（修复后新鲜运行）+ 状态行 grep 实证 + 修复面纯产物编辑确认；原审查全部代码级证据与命令面采信结论（全量 1050 / clippy / fmt / 探针）自记录以来覆盖范围未变化（本修复未触及），保持有效。

**Follow-up Decision**

接受本 Cycle：Gap 1 经当前 Cycle 修复关闭（Act 步骤 1/2 完成并经本复审步骤 3 核验），无新发现、无阻塞 finding——**Iteration 002 完成**（change 最后一个 Iteration）。F1 已顺手闭环；F2/F3 为归档期语料库注记；F4 留作未来行为域扩展注记——均不构成返工。

**Iteration Plan Update**

None（Iteration Map 不变；change 级 3 Iteration 全部完成）。

**Next Cycle**

None（无 rework/replan Cycle；Gap 1 经当前 Cycle 修复闭环）。

**Next Iteration**

None（change 无剩余 Iteration，3 个 Iteration 全部 accepted；可由用户调用 `openspec-docs-maintainer` 执行 change 收尾）。
