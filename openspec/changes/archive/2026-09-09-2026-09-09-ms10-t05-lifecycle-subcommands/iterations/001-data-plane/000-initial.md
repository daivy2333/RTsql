# Iteration 001 / Cycle 000-initial: 数据面子命令（dump/restore/import --csv）

## Plan Context

- Status: ready（2026-09-09 用户批准计划，Gate 2 通过；用户指令「更改gate状态，开始实施」交 openspec-act）
- Iteration: 001-data-plane
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None（新 Iteration 首个 Cycle；前一 Iteration 见 `../000-initial/`，其 Review Result: accepted）

**Iteration Scope**

- Change tasks: T6, T7, T8, T9
- Depends on: Iteration 000（已 accepted：入口分发骨架、resolve helper、DDL 生成器、new/list/schema 可用、约束持久化通道）
- Stable baseline: `dump`/`restore`/`import --csv` 可用且满足各自 spec 场景；dump-restore 往返等价可验证；全量回归绿（688 预期基线 + 新增用例）
- Verification boundary: `cargo test --all` 全绿（既有零修改）；clippy/fmt/openspec validate 全 0/PASS
- Diagnostic boundary: `src/cli/lifecycle.rs`、`src/cli/mod.rs`（分发接线）、`Cargo.toml`（csv）、`tests/cli_test.rs`
- Deferred tasks: None（change 最后一个 Iteration；T9 为 change 级终门）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部 What Changes 数据面部分；用户决策 ①import 表必须已存在+表头匹配 ②dump=SQL 文本（2026-09-09）；DDL 生成器 `create_table_sql`（含结尾分号）与 `quote_ident`/`column_type_sql` 复用；约束持久化通道（restore 经 SQL 建表自然继承）
- Excluded scope: planner/pipeline/storage 层修改（pipeline stage 函数只复用不修改）；dump CSV/其他格式；流式导出；planner INSERT 列清单；运行时约束强制；REPL/serve/key

**Objective**

`rtsql dump <db>`（SQL 文本导出）、`rtsql restore <db> <file|->`（空库恢复）、`rtsql import <db> <table> <file> --csv`（CSV 导入）可用且满足 spec 场景；dump→new→restore 往返等价；全量门（T9）通过，change 达到可收尾状态。

**Background**

Iteration 000 交付命令面骨架与元数据子命令（含约束持久化修复）；本 Iteration 补齐数据进出通道——逻辑导出（规避 .db/.wal 文件对配对问题，R18 主题 5）、空库恢复与 CSV 导入（R18 主题 7 设计空间；tasks.md MS10-T05）。spec 场景：R-dump-restore 7 个、R-import 8 个。

**Current Baseline**

- revision `a5b0a5f` + Iteration 000 工作树实施（未 commit；含 `src/cli/{mod,resolve,lifecycle}.rs`、`src/executor/create_table.rs`、`src/storage/data/table_manager.rs`、`tests/cli_test.rs`）
- 独立验证（2026-09-09，001-rework Review）：`cargo test --all` 零失败（lib 193/0；cli_test 36/0/2）；clippy exit 0 / fmt 干净 / validate PASS
- dump/restore/import 现为分发占位：`subcommand_placeholder`（`src/cli/mod.rs:154`）返回 `Usage("rtsql <name>: not implemented yet")` exit 2

**Current-State Evidence**

- **分发接线现状**：`execute_command`（`src/cli/mod.rs:142`）已含 `Some(Command::Dump { .. })` 等变量，但统一落 `Some(command) => subcommand_placeholder(command)`（`:148`）——T6-T8 各自把对应臂改为 `lifecycle::dump/restore/import` 调用（`Command` 变量字段名已定义：`Dump{db}`、`Restore{db,file}`、`Import{db,table,file,csv:bool}`，mod.rs:71-116 区段）
- **既有可复用符号（全部已验证）**：
  - 存在性前置模式（schema 先例，`lifecycle.rs:99-106`）：resolve → `!db_path.exists()` → `General("<path> does not exist")` exit 1，先于 `execute_command_inner`
  - 编排复用模式（`lifecycle.rs:107-136`）：`super::execute_command_inner(&db_path, move |db| Box::pin(async move { ... }), sigint_future, sigterm_future)`；所有路径收口 close()
  - DDL 生成器：`create_table_sql(table: &CatalogRow, columns: &[CatalogColumnRow]) -> String`（`lifecycle.rs:145`，pub(crate)，含结尾分号；调用方按 `column_index` 升序传列）；`quote_ident`（`:173`，私有——dump 复用时调整可见性为非实质）
  - 主命令执行循环（`run_sql`，`src/cli/mod.rs:277-320`）：`parse_stage(sql)` → 逐条 `stmt.to_string()` 作键 `plan_stage(db, &statement_text, stmt, false)` → `get_plan_output_columns` → `execute_stage(db, plan, false)` → Response 分支渲染；`sql_failure_status(k, n, error, statement_text)`（`:323-334`，≤200 字符截断 + k>1 追加已生效注明）
  - pipeline stage 签名：`parse_stage(sql: &str) -> Result<Vec<Statement>, String>`（`pipeline.rs:42`）、`plan_stage(database, sql, stmt, profiling: bool) -> Result<PhysicalPlan, String>`（`:56`，第 4 参 = profiling 非 tx）、`execute_stage(database, plan, profiling: bool) -> Response`（`:96`）
  - `Response` 分支（run_sql 消费形态）：`QueryResult { rows: Vec<Vec<serde_json::Value>> }` / `AffectedRows { count }` / `Error { message }` / `Pong`（Pong 在 pipeline 不可达，run_sql 有臂）
  - 渲染：`render(kind(format), columns, &QueryPayload::Rows/Affected)`（`render.rs:24-37`）+ `emit_stdout`（mod.rs:352）+ `kind`（`:336`，TTY table / 非 TTY json）
  - catalog 读：`db.table_manager.catalog()` → `scan_tables()` / `scan_columns(table)`（排序按 `column_index`）；表清单即 catalog 序
  - 表元数据（import 用）：`db.get_table(name) -> Result<Arc<TableMeta>>`（`database.rs:104`）；`TableMeta.columns: Vec<(String, ColumnType)>` 为 schema 列序（table_manager.rs `schema_cols` 构造，Iteration 000 rework 已核实）
- **SQL 字面量往返依据（已核实）**：`INSERT INTO "t" VALUES (...)` 全列形式（planner `extract_insert_values` 只收 `SetExpr::Values`，`ddl_dml.rs:94-128`）；`TRUE/FALSE` → `Value::Bool`（`src/parser/value.rs:26` `SqlValue::Boolean`）；数字字面量 `Number` 先 i64 后 f64（value.rs:11-20）；字符串单引号（`SingleQuotedString`）；`NULL` 字面量；标识符双引号（`quote_ident` 转义 `"`→`""`）
- **JSON 值形状（dump 字面量生成输入）**：`value_to_json`（`pipeline.rs:711-726`）——Int→Number(i64)、Float→Number(f64)（**非有限值已转 Null**，且 SQL 文本无法表达 NaN，CLI 可达数据无此形态）、String→String、Bool→Bool、Null→Null；字面量函数按 `Number::as_i64()` 优先、`as_f64()` 回退处理
- **Float 精度**：f64 Display（serde_json Number to_string）→ SQL Number f64 parse，Rust 保证最短往返表示
- **空库判定（restore 前置）**：work 内 `db.table_manager.catalog().scan_tables()` 非空 → `General` 拒绝（目标非空库）；存在性检查仍在编排之前（schema 先例）
- **csv 依赖**：Cargo.toml 当前无 csv（grep 0 命中）；crates.io 可达已验证（2026-09-09 Plan 会话 `cargo add csv` 成功拉取 csv + ryu/serde 等依赖）；`csv = "1"` 提供 `Reader::from_path`/`Reader::from_reader`、`headers()`、`records() -> StringRecord`（RFC4180 引号/转义/跨行内建）
- **测试夹具**：`run_cli(dir, args)`（60s 超时）/`fixture()`（TempDir + 预建 `db/`）/`seed_users(dir)`（建 users 表 + 1 行）；管道非 TTY 默认 json；`test_dump_restore_roundtrip` 等多步用例直接串联多次 `run_cli`
- **ExitStatus 语义**（不变）：Usage 2（import 缺 `--csv`；其余参数错由 clap）/ General 1（库不存在、表不存在、CSV 结构与转换错误、IO）/ Sql 3（restore/import SQL 执行失败）/ Locked 4 / Signaled 128+n

**Relevant Code**

- `src/cli/lifecycle.rs` — dump/restore/import 实现宿主（新增 `dump`/`restore`/`import_csv` 函数 + 字面量/转换纯函数 + lib 单测）
- `src/cli/mod.rs` — 仅分发臂接线（`:148-152` 占位替换为对应 lifecycle 调用）；`run_sql`/`sql_failure_status`/`emit_stdout`/`kind` 复用
- `Cargo.toml` — 新增 `csv = "1"`（dependencies）
- `tests/cli_test.rs` — 新增约 9 个集成用例（T6×2、T7×3、T8×5 中 1 个与 T6 共享）

**Critical Path**

- dump：存在性检查 → open（编排）→ work：`scan_tables` 空 → 无输出 Success；否则逐表（`scan_columns` 排序 → `create_table_sql` 行 → `parse_stage("SELECT * FROM <quote_ident>")` 单语句 → `plan_stage(db, stmt_text, stmt, false)` → `execute_stage` → `QueryResult.rows` → 逐行 `INSERT INTO "t" VALUES (lit, ...);`）→ 逐段 `emit_stdout` → close
- restore：存在性检查 → open → work：`scan_tables()` 非空 → `General` 拒绝；读文件（`-` → stdin `read_to_string`）→ `parse_stage` 全串 → 逐条 `plan_stage(db, stmt.to_string(), stmt, false)` + `execute_stage`（**不渲染**）→ 失败复用 `sql_failure_status` → close（DDL 持久化靠 close，编排保证）
- import：`--csv` 缺失 → `Usage`（先于一切 IO）→ 存在性检查 → open → work：`get_table(table)` Err → `General` 表不存在 → csv Reader（`Reader::from_path`）→ `headers()` 按名匹配（表全部列必须在表头、表头不得含表外列，违规 `General` 含列名）→ 每数据行按 header→schema index 重排 → 逐字段转换（纯函数）→ `INSERT INTO "t" VALUES (...)` → `db.execute_sql` 逐条 auto-commit → SQL 失败 `Sql`（`import row {k} of {n} failed: {error}`）→ 成功 `emit_stdout(render(kind(format), &[], &Affected(total)))` → close
- 状态与数据流：dump 只读（SELECT）；restore/import 写入走既有 DML auto-commit 路径；约束经 SQL 建表在 restore 中自然持久化（Iteration 000 rework 通道）

**Implementation Guidance**

顺序：T6（dump + 字面量纯函数）→ T7（restore，与 T6 共享往返用例）→ T8（csv 依赖 + import）→ T9（全量门）。字面量纯函数建议 `sql_literal(v: &serde_json::Value) -> String`：`Number` → `as_i64` 优先十进制、否则 `as_f64` Display；`Bool(b)` → `TRUE/FALSE`；`String(s)` → 单引号加倍；`Null` → `NULL`。CSV 转换纯函数建议 `csv_value(field: &str, col_type: &ColumnType) -> Result<Value, String>`：空字段 → Null（非 String）/ `String("")`；Int `i64::from_str`；Float `f64::from_str`；Bool `true/false` 大小写不敏感；String 原样；错误信息含列名与原值。表头匹配失败文案含缺失列名/未知列名。restore 的 stdin 读取（`-`）用 `std::io::read_to_string(std::io::stdin())`（启动阶段阻塞读可接受；异步化非实质）。import 行号定位：数据行 1-based（不含表头），失败文案 `row {k}: column "{c}": {原因}`。dump 逐表逐段 emit_stdout 与整缓冲等价（stdout 内容一致，非实质）。

**Behavioral Change**

| 场景 | 当前 | 目标 |
|---|---|---|
| `rtsql dump <db>` | Usage 占位 exit 2 | SQL 文本（DDL+INSERT 流）到 stdout，exit 0；空库无输出；库不存在 exit 1 |
| `rtsql restore <db> <file\|->` | Usage 占位 exit 2 | 空库逐条执行静默 exit 0；非空库/库不存在 exit 1；SQL 失败 exit 3 序号定位；`-` 读 stdin |
| `rtsql import <db> <table> <file> --csv` | Usage 占位 exit 2 | 表头匹配 CSV 导入，affected_rows 输出 exit 0；结构/转换错 exit 1；SQL 失败 exit 3；表/库不存在 exit 1 |
| 既有命令 | — | 零变化 |

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T6 | R-dump-restore / S1(dump半)、S2、S6、S7 | `src/cli/lifecycle.rs`（dump + 字面量纯函数）、`mod.rs:148` | Dump 臂占位 | dump 流 + `sql_literal` 纯函数 + lib 单测 |
| T7 | R-dump-restore / S1(restore半)、S3、S4、S5、S6、S7 | `src/cli/lifecycle.rs`（restore）、`mod.rs:148` | Restore 臂占位 | restore 流（空库检查 + 静默循环 + stdin） |
| T8 | R-import / S1-S8 | `Cargo.toml`、`src/cli/lifecycle.rs`（import + 转换纯函数）、`mod.rs:148` | Import 臂占位；无 csv 依赖 | csv 依赖 + import 流 + `csv_value` 纯函数 + lib 单测 |
| T9 | R1 回归 + change 级验证边界 | 全仓只读 | 基线 688/0/2 | 全量门 + clippy/fmt/validate |

**Task Contracts**

### T6: dump 子命令（SQL 文本导出）

- Requirement/Scenario: R-dump-restore / S1 往返（dump 半）、S2 空库、S6 库不存在、S7 锁冲突
- Depends on: None（Iteration 000 产出为既成基线）
- Targets: `src/cli/lifecycle.rs`（新增 `dump` + `sql_literal` 纯函数）、`src/cli/mod.rs:148`（Dump 臂接线）
- Current behavior: Dump 臂占位 `Usage` exit 2
- Required behavior: 存在性检查（`General("<path> does not exist")`）→ 编排 open → work：`scan_tables` 空 → 无输出 `Success`；否则逐表（DDL 行 + 全行 INSERT 流）经 `emit_stdout` 写出 → `Success`（design D7）；锁冲突经 `open_error_status` → exit 4
- Required changes: `dump` 函数 + `sql_literal` 纯函数（Number i64 优先/f64 回退、Bool TRUE/FALSE、String 单引号加倍、Null NULL）+ 分发接线
- Preserve: `create_table_sql`/`quote_ident`/pipeline stage/`render` 零修改；catalog 表序即输出序；不输出 6 语句面之外的 SQL
- Forbidden: 不做流式/分块；不动 planner/pipeline；不给 `--format` 接入 dump（DDL 文本非行集，父 Cycle deviation 2 同口径）
- Test witness: RED——lib 单测 `sql_literal_escaping`（单引号加倍/TRUE/FALSE/NULL/整数/浮点）；集成 `test_dump_empty_db_no_output`（RED：现占位 exit 2）；`test_dump_restore_roundtrip` 与 T7 共享（本任务先以 dump 产物断言 witnessing：输出含 `CREATE TABLE` 与 `INSERT INTO`）
- GREEN condition: lib 单测 + dump 侧断言绿；往返用例在 T7 后整体转绿
- Verification: `cargo test --lib && cargo test --test cli_test`（exit 0）
- Stop when: `SELECT *` 行形状与 schema 列序不一致（→ Blocker Handoff）

### T7: restore 子命令（空库恢复）

- Requirement/Scenario: R-dump-restore / S1 往返（restore 半）、S3 stdin、S4 非空库、S5 fail-fast、S6 库不存在、S7 锁冲突
- Depends on: T6（共享往返用例）
- Targets: `src/cli/lifecycle.rs`（新增 `restore`）、`src/cli/mod.rs:148`（Restore 臂接线）
- Current behavior: Restore 臂占位 `Usage` exit 2
- Required behavior: 存在性检查 → 编排 open → work：`scan_tables()` 非空 → `General`（目标非空库，exit 1）；读文件（`-` → stdin）→ `parse_stage` 全串 → 逐条 `plan_stage(db, stmt.to_string(), stmt, false)` + `execute_stage`（不渲染）→ 失败复用 `sql_failure_status`（exit 3，序号 + 前序已生效）→ 成功静默 `Success`（design D8）
- Required changes: `restore` 函数 + 静默逐条循环 + 分发接线
- Preserve: `run_sql`（主命令渲染路径）零修改；逐条 auto-commit；close 由编排保证（DDL 持久化）
- Forbidden: 不做整脚本事务；不渲染语句结果；不动 pipeline
- Test witness: RED——`test_restore_rejects_nonempty_target`（含表库 → exit 1）、`test_restore_fail_fast`（第二条重复主键 → exit 3 + stderr 含序号 + 首条已生效重开可见）、`test_restore_stdin_pipe`（`dump a | restore b -` 需先 `rtsql new b`；等价 SELECT 断言）；`test_dump_restore_roundtrip` 转绿
- GREEN condition: 3 新增 + 1 共享用例绿；既有零回归
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: 静默循环与 `sql_failure_status` 模板语义冲突（→ Blocker Handoff）

### T8: import --csv 子命令

- Requirement/Scenario: R-import / S1-S8 全部
- Depends on: T1 产出（分发骨架，已就绪）
- Targets: `Cargo.toml`（+`csv = "1"`）、`src/cli/lifecycle.rs`（新增 `import_csv` + `csv_value` 纯函数）、`src/cli/mod.rs:148`（Import 臂接线）
- Current behavior: Import 臂占位 `Usage` exit 2；Cargo.toml 无 csv
- Required behavior: `csv: false` → `Usage`（当前唯一格式必须显式）→ 存在性检查 → 编排 open → work：`get_table(table)` Err → `General` 表不存在 → 表头匹配（表全部列必须在表头、表头不得含表外列，违规 `General` 含列名）→ 行重排 → `csv_value` 转换（空→Null/空串；Int i64/Float f64/Bool true|false 大小写不敏感/String 原样；失败 `General` 含数据行号与列名与原值）→ `INSERT INTO "t" VALUES (...)` 全列 schema 序 → `db.execute_sql` 逐条 → SQL 失败 `Sql`（`import row {k} of {n} failed: {error}`）→ 成功 `emit_stdout(render(kind(format), &[], &Affected(total)))`（design D9）
- Required changes: csv 依赖 + `import_csv` 函数 + `csv_value` 纯函数 + 分发接线
- Preserve: 零 planner 修改（CLI 侧重排值序）；逐条 auto-commit；`--format` 对 affected 输出语义沿用
- Forbidden: 不做自动建表；不做批事务；不扩 INSERT 语法面；csv 拉取失败不得手写解析替代（→ Blocker Handoff）
- Test witness: RED——lib 单测 `csv_value_conversion`（类型/空字段/大小写 Bool/非法值错误信息）；集成 `test_import_basic_and_header_order`（乱序表头 + `{"affected_rows":2}`）、`test_import_conversion_fail_fast`（行号定位 + 前序行已生效）、`test_import_header_mismatch_rejected`、`test_import_missing_table_or_db`、`test_import_quoted_fields`（RFC4180 逗号/引号/跨行）
- GREEN condition: 1 lib 单测 + 5 集成用例绿；既有零回归
- Verification: `cargo test --lib && cargo test --test cli_test`（exit 0）
- Stop when: csv 拉取不可达（网络）或 RFC4180 场景与 crate 行为冲突（→ Blocker Handoff）

### T9: 回归门与全量验证（change 级终门）

- Requirement/Scenario: R1（全部既有场景回归保持）+ change 级验证边界
- Depends on: T6, T7, T8
- Targets: 全仓（只读验证）
- Current behavior: 基线 lib 193/0、cli_test 36/0/2（2026-09-09 rework Review 实测）
- 目标行为: `cargo test --all` 全绿（既有测试零修改）；clippy/fmt/openspec validate 全 0/PASS
- Required changes: 无代码改动（验证任务）
- Preserve: 既有测试断言语义零修改
- Forbidden: 不以放宽断言换取通过
- Test witness: 基线已留档（本文件 Current Baseline）；全量命令输出
- GREEN condition: 全量门通过
- Verification: `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`
- Stop when: 既有测试出现计划外破坏（→ Blocker Handoff，不得静默改断言）

**Invariants**

- 主命令与 Iteration 000 三个子命令（new/list/schema）行为零变化；既有全部测试零修改（唯一新增为 T6-T8 用例）。
- `render.rs`/`ExitStatus`/`resolve_db_path`/pipeline stage/`run_sql`/DDL 生成器零修改；planner/pipeline/storage 层零修改。
- dump/restore 均经编排收口 close（checkpoint）；restore/import 写入走既有 auto-commit 路径，无新事务语义。
- 运行时约束强制不引入（Iteration 000 设计决策 1 延续）；restore 对含约束 DDL 只持久化元数据。

**Non-goals**

dump CSV/其他格式；流式导出；planner INSERT 列清单；自动建表；批事务；REPL/serve/key；退出码枚举扩展；DEFAULT 持久化。

**Acceptance**

- R-dump-restore 全 7 场景：S1（`test_dump_restore_roundtrip`）、S2（`test_dump_empty_db_no_output`）、S3（`test_restore_stdin_pipe`）、S4（`test_restore_rejects_nonempty_target`）、S5（`test_restore_fail_fast`）、S6（dump/restore 不存在——T6/T7 用例或共享断言）、S7 锁冲突（`open_error_status` 复用，RTM 既定不逐场景建测）。
- R-import 全 8 场景：S1/S2（`test_import_basic_and_header_order`）、S3（`csv_value_conversion` + 转换用例）、S4（同用例空字段断言）、S5（`test_import_conversion_fail_fast`）、S6（`test_import_header_mismatch_rejected`）、S7（`test_import_missing_table_or_db`）、S8（`test_import_quoted_fields`）、S9 锁冲突（同上复用承载）。
- T9 全量门：`cargo test --all` 全绿 + clippy/fmt/validate 全 0/PASS。

**Verification**

- `cargo test --lib`（字面量/转换纯函数新单测）
- `cargo test --test cli_test`（约 9 新用例 + 既有 36 零修改）
- `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 分发接线现状（mod.rs:148/154）+ 可复用符号锚点（编排/DDL/stage/渲染/catalog/get_table）+ SQL 字面量往返依据（value.rs:26 等）+ JSON 值形状（value_to_json）+ csv 可达验证——全部为 Iteration 000 期间独立核实过的事实 |
| Design | PASS | design.md D7/D8/D9（dump 流/restore 流/import 流）+ 本 Cycle Critical Path 落位 |
| Iteration Plan | PASS | tasks.md Map 不变（T6-T9 本 Iteration）；平衡审计已在 tasks.md 记录 |
| Cycle Scope | PASS | initial 范围 = T6-T9；excluded 明确 |
| Task Contracts | PASS | T6-T8 契约自包含（父 Cycle 已验证的复用符号 + 本文件证据节，Act 只读本文件可实施）；T9 验证契约 |
| Traceability | PASS | tasks.md RTM R-dump-restore/R-import 行 + 本文件 Acceptance 映射闭合 |
| Verification | PASS | RED→GREEN 见证 + 零回归门；无身份型证据 |

**Persisted Evidence**

- Mode: none

`none`：全部验证命令低成本可重跑，Act Response 承载 Gate 5 与 Review；无一次性环境、无 Incident 现场。Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- csv 依赖需网络拉取（本地缓存无；crates.io 可达已验证 2026-09-09）——Act 时网络不可用 → Blocker Handoff。
- restore/import 大文件全文读入内存（proposal Impact 注记，嵌入式单机场景可接受）。
- dump 的 `SELECT * FROM "<t>"` 依赖 SELECT * 恒等投影（MS10-T01 R6 语义，`tests/projection_test.rs` 锁定）；表名经 `quote_ident` 转义后 parse 往返已由 DDL 往返测试覆盖同机制。
- import 的 affected 输出走 `render(kind(format), ...)`——非 TTY 默认 json `{"affected_rows":N}`，spec S1 锁定该形状。
- 非实质留 Act：dump 分段 vs 整缓冲写出、stdin 读取方式、错误文案微调（保留 spec 锁定关键词 `does not exist`/`database is locked`/affected 形状/序号模板）、新函数命名与可见性（`quote_ident` 提升可见性为非实质）。

## Act Response

- Status: blocked（2026-09-09，T8 R-import S3/S4 被引擎「键位不可键控行静默丢弃」缺口阻塞，见 Blocker Handoff；T6/T7 完成且全绿，T8 实现完成且其余场景全绿）

**Implemented**

- **T6（完成）**：`dump` 子命令——`sql_literal` 纯函数（Number i64 优先/f64 回退、Bool TRUE/FALSE、String 单引号加倍、Null NULL；引擎值四形状全覆盖，非 JSON 形状防御性归 NULL）+ `select_all_rows`（`SELECT *` 经 pipeline 三 stage 取行集）+ `dump` 流（存在性检查 → 编排 open → work：scan_tables 空 → 无输出；逐表 DDL 行 + 全行 `INSERT INTO … VALUES (…);` 经 emit_stdout）→ close；Dump 臂接线。RED（lib E0425 + 集成占位 exit 2）→ GREEN。
- **T7（完成）**：`restore` 子命令——存在性检查 → 编排 open → work：`scan_tables()` 非空 → `General` 拒绝（先空库检查后读文件，D8 顺序注记）→ 读文件（`-` → stdin `read_to_string`）→ `parse_stage` 全串（parse 错误 `Sql`，与主命令同语义）→ 逐条 `plan_stage`+`execute_stage` 静默循环 → 失败复用 `sql_failure_status`（exit 3 序号 + 前序已生效）→ 成功静默 `Success`；Restore 臂接线。3 集成用例 RED（占位 exit 2）→ GREEN；往返用例整体转绿。
- **T8（实现完成，S3/S4 场景阻塞）**：`Cargo.toml` +`csv = "1"`（1.4.0 拉取成功，网络可达）；`csv_value` 纯函数（Int i64 / Float f64 / Bool 大小写不敏感 / String 原样；空字段 → Null（非 String）/ 空串（String）；错误含原值）+ `import_csv`（缺 `--csv` → `Usage` 先于一切 IO → 存在性检查 → get_table 表校验 → 表头双向匹配（缺表列/含表外列 `General` 含列名）→ 行重排 → 转换 → `INSERT` 全列 schema 序 → `db.execute_sql` 逐条 auto-commit → SQL 失败 `Sql`（`import row {k} of {n} failed`）→ 成功 `render Affected`）；Import 臂接线、`subcommand_placeholder` 退役（T1 占位孤儿清理）。lib `csv_value_conversion` GREEN；6 集成用例中 5 GREEN，唯一失败 `test_import_types_and_empty_fields` 即 S3/S4 阻塞见证（RED 保持，断言未放宽）。

**Changed Files and Symbols**

- `src/cli/lifecycle.rs`：新增 `dump`/`select_all_rows`/`sql_literal`/`restore`/`read_restore_input`/`import_csv`/`csv_value`；tests 新增 `sql_literal_escaping`/`csv_value_conversion`；模块 doc 更新。
- `src/cli/mod.rs`：`execute_command` 六臂分发（Dump/Restore/Import 接线）；`subcommand_placeholder` 删除。
- `Cargo.toml`：`csv = "1"`。
- `tests/cli_test.rs`：新增 `spawn_cli_stdin`/`run_cli_stdin` 夹具（stdin 管道，写后关闭写端供 EOF）；新增集成用例 10 个（T6×2、T7×3、T8×6 中 roundtrip 为 T6/T7 共享递进交付）；`use std::io::{Read, Write}`。

**Deviations from Plan**

1. dump 内部 SELECT 表名用 catalog 原名而非契约 Critical Path 的 quote_ident——真二进制探针实证引擎以 ObjectName 的 Display 形式为表名（`pipeline.rs:950/963/859`、`query.rs:101`、`ddl_dml.rs:287` 均 to_string 查表/建表），`SELECT * FROM "items"` 查不到裸名建的 `items` 表（dump 主场景必失败）；原名写入 SQL 经 sqlparser Display 往返后与目录名恒等，对裸名/带引号两种建表来源都正确。INSERT 输出仍用 quote_ident，与冻结的 DDL 生成器（quote_ident）同文件自洽（探针 3 证实带引号 CREATE+INSERT 匹配）。Plan Risk 注记「表名经 quote_ident 转义后 parse 往返已由 DDL 往返测试覆盖同机制」被证伪（当时并无 DDL 执行往返测试）。
2. 往返用例种子避开负数字面量——planner `extract_insert_values` 只接受 `Expr::Value`/裸 NULL，负数被 sqlparser 解析为 UnaryOp → `UnsupportedValue`（`ddl_dml.rs:119`，既有限制）；`sql_literal` 对负数的渲染由 lib 单测锁定。
3. 契约外补充 1 个集成用例 `test_import_types_and_empty_fields`：契约 5 用例未端到端覆盖 S3/S4 的空字段语义（a 空→NULL / d 空→空串），而 RTM Acceptance 要求 S3/S4 见证——该用例即为阻塞缺口见证。
4. restore/import 闭包内错误文案使用 `db_path` 副本（`&db_path` 借用与 move 闭包冲突的机械解法，非实质）。

**Blocker Handoff**

- **发现位置**：T8 GREEN 验证（Gate 5）；task T8；requirement R-import scenario S3「类型转换与空字段语义」（S4 空字段断言同病）。
- **Plan 预期**：design D9 / Plan Context 断言「空字段 → Null（非 String 列）」经 INSERT 落库、SELECT 可比对该行。
- **实际（证据链）**：引擎对「键位（PK/默认首列）值为不可键控类型」的行**整行静默丢弃**——
  1. `Value::to_key()` 仅支持 Int：String/Null/Float/Bool 一律 `None`（`src/executor/value.rs:82-90`，doc 明示「仅 Int 类型支持」）；
  2. `InsertExecutor::next` 对 PK 值 `to_key()==None` 的行 `continue`——不写页、不计入 affected_rows、不报错（`src/executor/insert.rs:94-99`）；
  3. 无 PK 表的键位落在首列（真二进制探针实证：`t(a INT,b FLOAT,c BOOL,d STRING)` 无 PK 时首列 NULL 行丢弃、次行正常）。
  运行证据（探针均可低成本复现）：
  - `INSERT INTO t2 VALUES (NULL,2,3)`（无 PK，首列 NULL）→ `{"affected_rows":0}`，COUNT=0；
  - `INSERT INTO t5(a STRING,b STRING) VALUES ('x','y')`（**无 NULL 对照组**）→ 同样 `affected_rows:0`——非 INT 键位表整表不可经 SQL 插入；
  - `INSERT INTO t6(id INT PRIMARY KEY, note STRING) VALUES (9,NULL)` → affected 1、`[9,null]` 可见——非键位 NULL 正常。
- **影响**：
  1. R-import S3 按 spec 表形状（无 PK 表首列空字段 → NULL）不可满足——该行恰落键位被静默丢弃；`test_import_types_and_empty_fields` 保持 RED 作缺口见证。
  2. import/restore 对 NULL 键位行与非 INT 键位表存在**静默数据丢失**（exit 0 且 affected_rows 少计），与本 change 数据面保真目标相悖；S5 类 fail-fast 只覆盖显式错误，不覆盖此静默面。
  3. dump 读路径不受影响；restore 经同一 INSERT 通道继承（2）的丢失面。
- **修复所需超出当前契约**：需修改 `src/executor/insert.rs`（静默 continue → 显式拒绝或无索引插入）与/或 `executor/value.rs::to_key`（扩展 String/Float 键控、NULL 主键语义），并连带决策无 PK 表键位默认、点查/唯一性/恢复重建（MS10-T02 R7/R8）语义——属 executor/storage 层，与本 cycle Invariants「planner/pipeline/storage 层零修改」（Iteration 000 rework 收窄面仅限建库约束通道）直接冲突；「NULL 主键响亮拒绝 vs 无索引插入」「String/Float 主键是否键控」均为有语义后果的设计决策，必须返回 Plan。
- **进度现场**：T6、T7 完成且全绿；T8 实现完成（lib 195/0；cli_test 46 passed / 1 failed（唯一失败=阻塞见证）/ 2 ignored）；T9 全量门未跑（阻塞门约束，仅留 lib+cli_test+clippy+fmt 快照）。已通过 Gate：T6/T7 全部（Gate 3/4/5）；T8 Gate 3/4 通过、Gate 5 对 S3/S4 BLOCKED。工作树未 commit。
- **恢复条件**：Plan 就键位不可键控行的引擎语义做出设计决策（方向候选：① `insert.rs` 对 `to_key()==None` 显式报错——最小修复，import/restore 转响亮失败但 S3 仍不可满足，需同步修订 spec；② 扩展 `to_key` 键控面 + NULL 主键语义——S3 可满足，波及索引/点查/恢复面；③ 修订 spec R-import S3 表形状为含 INT PK 表——范围决策），产出 rework/replan 契约后 Act 恢复。

**Blocker Resolution**

None（未恢复）

**Self-Review**

- Plan compliance: T6/T7 按契约全绿；T8 按契约实现，S3/S4 被契约外引擎缺口阻塞（Blocker Handoff）
- Full diff reviewed: yes——阻塞前对 T6/T7/T8 完整 diff 逐项过目（分发臂与占位孤儿清理、错误路径、逐条 auto-commit、dump 文件 DDL/INSERT 引号自洽、restore 静默循环与 `sql_failure_status` 复用、import 表头双向匹配与计数语义）；跨任务交互无计划外修改
- Critical findings unresolved: 1（键位不可键控行静默丢弃 → Blocker Handoff）
- Important findings unresolved: 0
- Minor findings unresolved: 0

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T6 lib 单测 | `cargo test --lib cli::lifecycle` | `4 passed; 0 failed`（sql_literal_escaping + csv_value_conversion） | PASS |
| 集成主体 | `cargo test --test cli_test` | `test result: FAILED. 46 passed; 1 failed; 2 ignored`（唯一失败 `test_import_types_and_empty_fields` = 阻塞见证；T6×2、T7×3+往返、T8×5 全绿） | T6/T7 PASS；T8 S3/S4 BLOCKED |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | 0 code warning（仅 cargo config 弃用提示） | PASS |
| 格式 | `cargo fmt --check` | fmt-clean | PASS |
| 阻塞复现 | 真二进制探针（Blocker Handoff 三组） | 键位 NULL/非 INT 键位行 `affected_rows:0` 静默丢弃 | BLOCKED |

**Persisted Evidence**

None required（`none` 模式：探针与测试均可低成本复现，无一次性环境、无 Incident 现场）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None（发现的引擎缺陷属 Plan 设计决策事项，非已验证可重复操作路径或已发生故障） |

**Remaining Issues**

1. R-import S3/S4 阻塞（见 Blocker Handoff）——含 import/restore 对键位不可键控行的静默丢失面。
2. 引擎「ObjectName Display 形式即表名」约定（Deviation 1）：dump 的带引号 DDL 在 restore 重建后表名含引号字符（`items` → `"items"`），对该库再 dump 出现引号膨胀（`"""items"""`）——一代往返数据等价（测试锁定），多代往返表名不保真；schema 命令对带引号建表同病（Iteration 000 已有行为）。超出本 change 场景，建议 Plan 评估是否登记。
3. 负数字面量 INSERT 不可达（UnaryOp → `UnsupportedValue`，Deviation 2）：import/restore 对含负数的 CSV/dump 文本将 exit 3 响亮失败（非静默）；既有限制，建议随阻塞一并评估登记。
4. dump 对 lib 直建的非 Display 安全表名（如含空格）SELECT 解析失败——CLI 不可达（CLI 建表必经 SQL），边界记录。

**Commit or Diff Reference**

未 commit（工作树现场：Iteration 000 两 Cycle 产出 + 本 Cycle T6-T8 产出——`src/cli/{mod,resolve,lifecycle}.rs`、`src/executor/create_table.rs`、`src/storage/data/table_manager.rs`、`Cargo.toml`、`tests/cli_test.rs` + change 目录 + R20 登记）

## Plan Review

- Review Result: rework-required（2026-09-09）

**Findings**

1. **T6/T7 独立验证 PASS**：Plan 独立重跑 `cargo test --all` → lib 与各集成套件全绿，cli_test `46 passed; 1 failed; 2 ignored`（唯一失败 `test_import_types_and_empty_fields`，即阻塞见证自身；失败输出实证首行 `[null,2.5,true,"keep"]` 整行丢失、次行 `[7,null,false,""]` 在）；`cargo clippy --all-targets -- -D warnings` 0 warning；`cargo fmt --check` 干净；`openspec validate` PASS。diff 逐项比对契约吻合：dump 流（存在性检查 → 编排 → scan_tables 空 → 无输出；DDL 行 + `sql_literal` INSERT 流）、`sql_literal` 四形状、restore 静默逐条循环（先空库检查后读文件）与 `sql_failure_status` 复用、stdin 夹具、import 表头双向匹配与 `csv_value` 转换、`subcommand_placeholder` 退役；`run_sql`/`render.rs`/pipeline stage 零修改核实。
2. **Blocker 核实成立（阻塞）**：独立复核证据链全部属实——`Value::to_key()` 仅 Int（`src/executor/value.rs:82-90`）；`InsertExecutor::next` 对 `to_key()==None` 在 tuple 序列化/WAL/版本记录之前 `continue`（`src/executor/insert.rs:96-99`，静默丢弃无恢复残留）；全量重跑失败输出与 Act 探针一致。
3. **Review 补充调查（决定修复面的新证据）**：
   - **SQL 路径不存在无主键表**：`CreateTableExecutor` 对未声明 PK 取第一列为主键（`src/executor/create_table.rs:60-69`），`pk_column` 持久化于 catalog 且 `create_table_sql` 渲染 `PRIMARY KEY`（`src/cli/lifecycle.rs:534`）——S3 的本质是「隐式 PK 列（首列 a INT）接受 NULL」，t5 探针的本质是「String 隐式主键表整表不可插」。
   - **恢复路径硬依赖键控**：Update 重放对 `extract_pk_key(old_tuple)==None` 硬性 `RedoFailed`（`src/wal/recovery.rs:591-599`）——放开无键落库若不同步恢复回退，「无键行 INSERT + UPDATE + 崩溃」将使 `Database::open` 永久失败，比静默丢弃更严重。Delete 重放（位置寻址 + `find_key_by_row_id` Option）、索引重建（链回溯位置寻址 + `if let Some(key)` 跳过无键 slot，recovery.rs:856）、deindexed Insert 重放（`if let Some(key)`，:522）对无键行均已安全；非 deindexed 的 Update/Insert redo 分支对重放记录不可达（`will_redo` ⇔ `pk_versions` Some，:425-438）。
   - **WAL 格式变更路线排除**：Update 记录携带 `old_row_id` 需改记录编码 → 文件格式版本 bump → 既有库全数被 `IncompatibleHeader` 拒绝，不可接受。
   - **既有测试无静默丢弃钉死**：`test_pipeline_join_with_null_keys`（pipeline_test.rs:610-649）INSERT 响应被弃、JOIN 计数在「行被丢弃」与「NULL 不匹配」两语义下同为 1 行——零修改约束与修复兼容。
   - **Act 候选②被证据排除**：B-Tree `Key` 32 字节定长（MS10-T02 G1 尾零比较修复即其代价），String 键控为独立设计工程；且 S3 所需 NULL 在任何键控方案下均不可为键——无键回退不可避免。
4. **修复方向经用户裁定（2026-09-09，本会话 AskUserQuestion）**：**方向 A——无键值落库不入索引 + 恢复无键回退**（S3 按已批准原文满足；NULL-in-PK 有 SQLite 先例；dump→restore 往返对全表形状可用；String/Bool/Float 首列隐式主键表由「整表静默不可插」变为可用）。
5. **Minor（非阻塞）**：`lifecycle.rs` dump 的 `SELECT` 用 catalog 原名（Deviation 1）——多代表名引号膨胀与含空格表名边界留 improvement 候选；import/restore 大文件全文读入（proposal Impact 既注记）。

**Deviation Classification**

- **Deviation 1（dump 内部 SELECT 用 catalog 原名而非 quote_ident）**：PLAN-INVALID（非阻塞）——Plan Risk 注记「quote_ident 转义 parse 往返已被 DDL 往返测试覆盖」被证伪（当时并无该测试）；Act 真二进制探针实证引擎以 ObjectName Display 为表名并选择正确形态，INSERT 输出仍用 quote_ident 与冻结 DDL 生成器自洽。
- **Deviation 2（往返用例回避负数字面量）**：NEW-EVIDENCE（非阻塞）——引擎既有限制（`ddl_dml.rs:119` UnaryOp → `UnsupportedValue`），非本 change 引入；`sql_literal` 对负数的渲染由 lib 单测锁定。
- **Deviation 3（契约外补充 `test_import_types_and_empty_fields`）**：ACT-DEVIATION（非阻塞）——RTM Acceptance 要求 S3/S4 见证而 T8 契约 5 用例未端到端覆盖空字段语义；补充用例即缺口见证，符合验收意图，未放宽任何断言。
- **Deviation 4（`db_path` 闭包副本）**：非实质（借用与 move 闭包冲突的机械解法）。

**Acceptance Gaps**

- **R-import S3「类型转换与空字段语义」**：`test_import_types_and_empty_fields` RED——键位 NULL 行被静默丢弃（S4 空字段断言同病）。R-import 其余场景（S1/S2/S5/S6/S7/S8）GREEN；S9 锁冲突由 `open_error_status` 复用承载（RTM 既定）。
- **R-dump-restore 全部场景 GREEN**（往返/空库/stdin/非空拒绝/fail-fast/不存在/锁冲突复用）。
- **T9 change 级全量门未跑**——Act 受阻塞门约束仅留 lib+cli_test+clippy+fmt 快照；待 gap 关闭后随 T9 执行。

**Convergence**

N/A（initial Cycle 首次 Review；gap 由引擎既有缺陷经本 Iteration 数据面首次暴露引出，无父 gap 可比较）

**Evidence**

- `cargo test --all` → lib 全绿；cli_test `46 passed; 1 failed; 2 ignored`（唯一失败 = 阻塞见证）；`cargo clippy --all-targets -- -D warnings` → 0 code warning；`cargo fmt --check` → 干净；`openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` → valid
- 代码核实：`src/executor/value.rs:82-90`、`src/executor/insert.rs:96-99`、`src/executor/create_table.rs:60-69`、`src/cli/lifecycle.rs:534`（create_table_sql PK 渲染）、`src/wal/recovery.rs:591-599`（Update 重放 RedoFailed）/`:522`（Insert 重放 if-let）/`:425-438`（deindexed 门控）/`:856`（重建无键跳过）/`record.rs:62-68`（Update 携带 row_id）；`tests/pipeline_test.rs:610-649`（NULL JOIN 用例两语义兼容）；`tests/cli_test.rs:1512-1545`（见证用例表形状与断言）

**Follow-up Decision**

创建 **001-rework**（同 Iteration 目录）：修复需要新执行契约——触碰面从 CLI 层扩展至 `src/executor/insert.rs`（无键落库）与 `src/wal/recovery.rs`（Update 重放无键回退），且恢复推导语义涉及 `wal-recovery-replay-integrity` spec（MODIFIED delta 随本 change 交付），超出本 Cycle 契约与 Invariants。设计决策已由用户裁定为方向 A（2026-09-09）。repair items：T8-R1（insert 无键落库）、T8-R2（恢复无键回退）、T8-R3（S3/S4 见证转绿 + 全形状往返与崩溃恢复见证）。requirement/验收边界不变，Iteration Map 不变，不构成 replan。

**Iteration Plan Update**

None（Iteration Map 不变）

**Next Cycle**

`001-rework.md`（同 Iteration 目录，repair items：T8-R1、T8-R2、T8-R3）

**Next Iteration**

None（Iteration 001 未完成；001-rework accepted 后 T9 收口）
