# Iteration 000 / Cycle 000-initial: 命令面骨架与元数据子命令

## Plan Context

- Status: ready（2026-09-09 用户批准计划，Gate 2 通过）
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4, T5
- Depends on: None
- Stable baseline: `rtsql` 分发 `new`/`list`/`schema` 且满足各自 spec 场景；主命令合法输入零回归（cli_test 25 用例零修改全绿）；`resolve_db_path` 外部语义不变
- Verification boundary: `cargo test --all` 全绿（既有零修改 + 新增用例）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/cli/`、`tests/cli_test.rs`
- Deferred tasks: T6, T7, T8, T9（Iteration 001 数据面子命令）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部 What Changes；用户决策 ①import 表必须已存在+表头匹配 ②dump=SQL 文本 ③仅 new 建父目录 ④开库子命令对不存在库报错（2026-09-09）；8 项默认假设经 Gate 1 批准
- Excluded scope: dump/restore/import（Iteration 001）；planner/pipeline/storage 层修改；`render.rs` 修改；新依赖（csv 属 Iteration 001）；退出码枚举扩展；REPL/serve/key

**Objective**

`rtsql` 从单一 one-shot 命令扩展为多命令入口：`new`（显式建库+父目录）、`list`（集中区枚举）、`schema`（DDL 发现）可用且满足 spec 场景；主命令 `rtsql <db> "<sql>"` 合法输入行为零变化。

**Background**

MS10 主轨 T01-T04 后 one-shot 主命令全链路可用，但生命周期面空白：建库只能静默（无显式入口）、agent 无 schema 发现步骤、集中区不可枚举（R18 主题 7；tasks.md MS10-T05）。本 Iteration 交付命令面骨架（T1 入口重构是全部子命令的公共前置）与三个元数据子命令；数据面（dump/restore/import）留给 Iteration 001。

**Current Baseline**

- revision `a5b0a5f`（master，工作树干净）
- 基线（2026-09-09 实测）：`cargo test --test cli_test` → 25 passed / 0 failed / 2 ignored（exit 0）；全量 671/0/2（SNAPSHOT，commit `8827700` 后）
- CLI 现状：单一 one-shot 命令；`rtsql list` 等被解析为"库名 list + 缺 SQL 参数" → exit 2

**Current-State Evidence**

- 入口链：`main.rs`（6 行转发）→ `cli::run`（`cli/mod.rs:88-95`）→ `CliArgs::parse()`（clap derive）→ `execute_command(&args)`（`:97-112`）→ `resolve::resolve_db_path` → `execute_command_inner(db_path, work_factory, sigint, sigterm)`（`:162-196`）
- `execute_command_inner` 契约：阶段 1 `Database::open` 与信号竞争；阶段 2 `work(&db)` 与信号竞争；所有路径收口 `db.close()`（checkpoint + WAL 截断）；`work` 为 HRTB 工厂 `impl for<'a> FnOnce(&'a Database) -> WorkFuture<'a> + Send`；信号 future 工厂 `impl Fn() -> SignalFuture + Send`（`:114-141` 已有 `sigint_future`/`sigterm_future`）；结构测试注入见 mod.rs:287-371
- `open_error_status`（`:143-154`）：`StorageError::DatabaseLocked` → `Locked("database is locked: <path>")` exit 4；其他 → `General("failed to open database <path>: ...")` exit 1
- `ExitStatus`（`:24-60`）：Success 0 / General 1 / Usage 2 / Sql 3 / Locked 4 / InvalidKey 5 / Signaled(128+n)；`run` 对 `message()` 非 None 者 emit_stderr
- `CliArgs` 现状（`:63-77`）：`db: String` + `sql: String` + `#[arg(short, long, value_enum)] format: Option<FormatArg>`；`FormatArg`（`:79-85`）Table/Json/Csv/Tsv → `kind()`（`:256-271`，TTY table / 非 TTY json 默认）
- **clap 实证（`/tmp/clap-probe` 探针，clap 4 derive，2026-09-09）**：`db: Option<String>` + `sql: Option<String>` + `Option<Subcommand>` 组合下——`list` → SUBCOMMAND List；`new foo` → SUBCOMMAND New{foo}；`schema mydb` → SUBCOMMAND Schema；`mydb "SELECT 1"` → POSITIONAL 填充；无参数 → 双 None（需手动 usage）；`list "SELECT 1"` → 子命令臂报 unexpected argument exit 2（裸名冲突 = 子命令优先）。**必需** `db: String` 变体下子命令完全不可达（先报缺 `<DB>`）——入口必须用 Option
- 名称解析（`cli/resolve.rs:12-26`）：含 `/` 路径直用；裸名 → `base/db/<name>.db`，base = `$RTSQL_HOME`（未设 → `$HOME/.rtsql`）；双缺失 → Err；不建目录；基目录推导内联（T2 提取点）；既有 2 单测（`:61-93`，EnvGuard 顺序模式）
- 建库链（`new` 依赖）：`FileStorage::open`（`storage/file_storage.rs:41-`）`OpenOptions::create(true).truncate(false)` → try_lock → 0 字节写 64B 头（无 fsync）→ `TableManager::new` 按 `page_count()==0` → `Catalog::bootstrap`（内含 `flush_all`，catalog.rs:119）；`Database::open`（`database.rs:28-93`）全链；`close()` = checkpoint（`database.rs:182-194`）；静默建库契约测试 `tests/cli_test.rs:506`
- schema 数据源（`schema` 依赖）：系统表不在 `TableManager.tables` map（SQL 不可查，`ReservedTableName` 守卫）；`db.table_manager.catalog()`（`data/table_manager.rs:136`）→ `scan_tables() -> Vec<CatalogRow>`（`storage/catalog.rs:205-211`）/ `scan_columns(table_name) -> Vec<CatalogColumnRow>`（`:214-220`）；`CatalogRow { table_name, data_page_head, index_root_page_id, pk_index, pk_column, column_count, data_page_tail }`、`CatalogColumnRow { table_name, column_index, column_name, column_type(存储面), not_null, unique }`（catalog.rs:49-69）
- DDL 往返依据（已核实）：DDL 词法面 `convert_data_type`（`parser/planner/ddl_dml.rs:133-169`）INT 族/STRING 族（Varchar/Char/Text/Clob 等）/FLOAT 族/BOOL 族四族归一；`STRING` 关键字有既有先例（`tests/cli_test.rs` `CREATE TABLE users (id INT PRIMARY KEY, name STRING)`）；约束提取 `extract_column_constraints`（`:170-198`）接受 NotNull/Unique（PK 单独处理）；String 长度：SQL 面 `executor::ColumnType::String` 无长度 → 存储 `String(255)` 固定（`executor/plan.rs:195-201`），CLI 建库恒 255；DEFAULT 不在 catalog 序列化面（`catalog.rs:652-672` 仅 not_null/unique）
- 渲染复用面：`render(kind, columns: &[String], payload: &QueryPayload)`（`cli/render.rs:24-37`）纯函数；`QueryPayload::Rows(Vec<Vec<serde_json::Value>>)` / `Affected(u64)`；json 行集 `{"columns":[...],"rows":[...]}`；`emit_stdout`（mod.rs:273-280）写文本+`\n`+flush
- 测试夹具：`tests/cli_test.rs` `run_cli(dir, args)`（`:26-41`，真二进制 `env!("CARGO_BIN_EXE_rtsql")`、`RTSQL_HOME` 指向 TempDir、stdin null、管道非 TTY → 默认 json）、`fixture()`（`:81-86`，TempDir + 预建 `db/`——新目录场景需自行建无 `db/` 的 TempDir）、60s 超时 kill
- lib 单测落点：`src/cli/mod.rs` tests 模块（`:287-371`）/ 各新子模块 `#[cfg(test)]`；纯函数（DDL 生成）必须可直接单测（D10）

**Relevant Code**

- `src/cli/mod.rs` — 入口重构宿主（T1）+ `Command` 枚举分发；`ExitStatus`/`emit_stdout`/`kind` 复用
- `src/cli/resolve.rs` — helper 提取（T2）
- `src/cli/` 新子模块（组织非实质，D10）— new/list/schema 实现（T3/T4/T5）+ DDL 生成纯函数（T5）
- `src/storage/catalog.rs`、`src/database.rs`、`src/storage/file_storage.rs` — 只读复用，零修改
- `tests/cli_test.rs` — 新增用例（本 Iteration 约 10 个）

**Critical Path**

`run` → clap 解析 → `command` 分支：None → 主命令臂（db/sql 校验 → `execute_command`）；`Some(cmd)` → 各子命令臂。开库子命令（new/schema）：存在性/目录前置（文件系统操作，open 之前）→ `execute_command_inner`（open → work → close）。list：纯文件系统，不经编排。数据流：new 产出空库文件对（.db + .wal 截断）；list 产出行集 → render → stdout；schema 产出 DDL 行序列 → stdout。错误路径：前置检查失败不开库（无锁副作用）；open 链错误经 `open_error_status` 归类。

**Implementation Guidance**

顺序：T1（入口重构先行——分发臂先以 Usage 占位，后续任务逐个填充实现）→ T2（helper，独立可验证）→ T3 → T4 → T5。子命令实现建议一个新子模块（如 `src/cli/lifecycle.rs`）+ DDL 纯函数（可放 `src/cli/ddl.rs` 或子模块内），文件组织非实质。主命令缺参文案建议含用法提示（如 `usage: rtsql <db> <sql>` 或子命令列表）；spec 只锁定 exit 2 + stderr 输出。new 的存在性检查用 `std::path::Path::exists`；目录创建 `std::fs::create_dir_all`（失败 → General）。list 排序用文件名字典序；`size_bytes` 用 `metadata().len()`（u64 → `Value::from`）。schema 的 DDL 行逐表 `emit_stdout`（每行一个 `\n` 结尾输出）。DDL 生成器签名建议 `(row: &CatalogRow, columns: &[CatalogColumnRow]) -> String`（调用方先按 `column_index` 排序）。

**Behavioral Change**

| 场景 | 当前 | 目标 |
|---|---|---|
| `rtsql list` | exit 2（缺 SQL 解析） | 枚举集中区 `*.db` 行集，exit 0 |
| `rtsql new <name\|path>` | exit 2 | 建空库 + 父目录，静默 exit 0；已存在 → exit 1 |
| `rtsql schema <db>` | exit 2 | DDL 文本输出 exit 0；空库无输出；库不存在 exit 1 |
| `rtsql <db> "<sql>"` 合法输入 | 正常 | 零变化 |
| `rtsql` / `rtsql <db>`（缺参） | clap 自动 exit 2 | 手动 usage exit 2（文案可不同，退出码与 stderr 语义同） |
| 裸名 = 子命令名 | exit 2（缺 SQL） | 分发子命令（此类库需路径形式打开） |

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/S 分发、冲突 | `src/cli/mod.rs::CliArgs/run/execute_command` | 扁平位置参数 one-shot | Option 参数 + `Command` 枚举 + 手动 usage + 分发骨架 |
| T2 | R-list/R-new 前置、R2 不变量 | `src/cli/resolve.rs` | 内联基目录推导 | 提取 `rtsql_home()`/`db_dir()`，`resolve_db_path` 消费之 |
| T3 | R-new/S1-S4 | `src/cli/`（Command::New） | 无 | 存在性检查 + mkdir + 编排复用建库 |
| T4 | R-list/S1-S3 | `src/cli/`（Command::List） | 无 | db_dir 枚举 + render 行集 |
| T5 | R-schema/S1-S4 | `src/cli/`（DDL 纯函数 + Command::Schema） | 无 | DDL 生成器 + catalog 扫描输出 |

**Task Contracts**

### T1: 入口重构——Option 位置参数 + 子命令分发

- Requirement/Scenario: R1 / S 子命令分发与主命令零回归、S 裸名与子命令名冲突
- Depends on: None
- Targets: `src/cli/mod.rs::CliArgs/run/execute_command`
- Current behavior: 扁平 `db: String` + `sql: String`；无子命令
- Required behavior: `db: Option<String>` + `sql: Option<String>` + `--format`（`global = true`）+ `command: Option<Command>`（`New{target}/List/Schema{db}/Dump{db}/Restore{db,file}/Import{db,table,file,csv:bool}`）；`command: None` 时主命令臂——db/sql 任一缺失 → `ExitStatus::Usage`；`Some(cmd)` 分发（本任务允许臂内先返回 `Usage` 占位，由 T3-T5 契约填充真实实现）
- Required changes: 结构体重构 + 分发 match + 手动 usage 分支（design D1）
- Preserve: 主命令合法输入全链路（`execute_command` → `execute_command_inner` → `run_sql`）零修改；`--format` 对主命令语义不变；`ExitStatus` 枚举不动
- Forbidden: 不改 `execute_command_inner`/`run_sql`/`resolve_db_path`；不实现子命令业务逻辑；不加退出码
- Test witness: RED——`tests/cli_test.rs` 新增 `test_missing_sql_arg_exit_2`（`rtsql app` → exit 2）；`test_subcommand_dispatch_list_runs`（`rtsql list` 不再走主命令缺参路径——本任务先断言其退出码 ≠ 主命令"缺 SQL"语义下的旧形态，T4 完成后自然转绿为 exit 0；若本任务期占位实现为 exit 2 则本用例断言放宽为「stderr 非空且 exit 2」，T4 契约负责收紧）。跑 `cargo test --test cli_test` 观察：新增用例按上述定义、既有 25 零修改
- GREEN condition: 新增用例绿 + 既有 25 用例零修改全绿
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: Option 位置参数 + global `--format` 组合产生与探针实证相悖的解析行为（→ Blocker Handoff）

### T2: resolve 目录 helper

- Requirement/Scenario: R-list（基目录）、R-new（集中区目录）；R2 不变量
- Depends on: None
- Targets: `src/cli/resolve.rs`
- Current behavior: 基目录推导内联于 `resolve_db_path`（`:16-25`）
- Required behavior: 提取 `rtsql_home() -> Result<PathBuf, String>` 与 `db_dir() -> Result<PathBuf, String>`（= `rtsql_home()?.join("db")`）；`resolve_db_path` 改为消费 `rtsql_home()`，对外行为逐字不变
- Required changes: 两 helper + `resolve_db_path` 内联段替换
- Preserve: 既有三场景（路径直用 / 裸名默认 / 双 env 缺失 Err）语义与文案零变化；不建目录
- Forbidden: 不改 `resolve_db_path` 签名；不建目录
- Test witness: 变更前 GREEN——先跑 resolve 既有 2 单测确认全绿；新增 `db_dir` 单测（RTSQL_HOME 指定 / HOME 默认 / 双缺失 Err，EnvGuard 顺序模式）
- GREEN condition: 新增单测绿 + 既有零修改绿
- Verification: `cargo test --lib`（exit 0）
- Stop when: 提取导致 `resolve_db_path` 可观察行为变化（→ Blocker Handoff）

### T3: new 子命令

- Requirement/Scenario: R-new / S1 裸名+目录、S2 路径父目录、S3 已存在拒绝、S4 立即可用
- Depends on: T1, T2
- Targets: `src/cli/`（`Command::New` 臂）
- Current behavior: `rtsql new foo` 走主命令缺参 → exit 2
- Required behavior: resolve target（裸名/路径）→ `Path::exists` → 已存在（含 0 字节）→ `General("<path> already exists")` exit 1 文件不动；否则 `create_dir_all` 父目录 → `execute_command_inner(path, 空work, sigint, sigterm)` 建库 + close → 静默 `Success`（design D4）
- Required changes: `Command::New` 分支 + 前置检查 + 目录创建
- Preserve: 主命令静默建库契约不动；信号编排语义；open 链零修改
- Forbidden: 不做事务性文件创建；不 fsync；不改建库路径
- Test witness: RED——`tests/cli_test.rs` 新增 `test_new_creates_db_and_dirs`（TempDir 不预建 db/：`rtsql new app` exit 0 无 stdout + `<tmp>/db/app.db` 存在 + 紧接 `rtsql app "CREATE TABLE t (id INT)"` exit 0）、`test_new_path_creates_parents`（`<tmp>/x/y/data.db` 深路径 exit 0 + 存在）、`test_new_existing_file_rejected`（预写文件含 0 字节变体 → exit 1 + stderr 含 `already exists` + 内容不变）
- GREEN condition: 3 用例绿；既有零回归
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: open 链对新文件产生计划外副作用（→ Blocker Handoff）

### T4: list 子命令

- Requirement/Scenario: R-list / S1 枚举、S2 空行集、S3 双 env 缺失
- Depends on: T1, T2
- Targets: `src/cli/`（`Command::List` 臂）
- Current behavior: `rtsql list` 走主命令缺参 → exit 2
- Required behavior: `db_dir()` → `read_dir`（目录不存在 → 空行集）→ 过滤扩展名 `.db` 的常规文件 → `(name, size_bytes)` 名称排序 → `render(kind(format), &["name","size_bytes"], &QueryPayload::Rows(rows))` → `emit_stdout`；双 env 缺失 → `General` exit 1；不开库（design D5）
- Required changes: `Command::List` 分支
- Preserve: `render()`/`kind()` 语义（`--format` 与 TTY/非 TTY 默认适用）；不触碰锁/格式头
- Forbidden: 不开库校验内容；不经编排（无 DB 操作）
- Test witness: RED——`tests/cli_test.rs` 新增 `test_list_enumerates_db_files`（fixture 内建 a.db/b.db 两文件（任意字节内容）+ notes.txt → 默认 json 断言 `{"columns":["name","size_bytes"],"rows":[["a.db",100],["b.db",200]]}`，notes.txt 不出现）、`test_list_empty_or_missing_dir`（空 db/ 或删除 db/ → rows 空数组 exit 0）；T1 的 `test_subcommand_dispatch_list_runs` 收紧为 exit 0 断言
- GREEN condition: 2 新增 + 1 收紧用例绿；既有零回归
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: 渲染行集形状与 `render()` 契约冲突（→ Blocker Handoff）

### T5: DDL 生成器 + schema 子命令

- Requirement/Scenario: R-schema / S1 DDL 输出、S2 空库、S3 不存在、S4 锁冲突
- Depends on: T1
- Targets: `src/cli/`（DDL 生成纯函数 + `Command::Schema` 臂）
- Current behavior: 无 schema 命令；catalog 读 API 无 CLI 消费者
- Required behavior: DDL 纯函数（签名建议 `(row: &CatalogRow, columns: &[CatalogColumnRow]) -> String`：类型 Int→INT/Float→FLOAT/Bool→BOOL/`String(_)`→STRING；标识符恒双引号内部 `"` 加倍；约束序 PRIMARY KEY→NOT NULL→UNIQUE；调用方按 `column_index` 升序传入）+ schema 流程（resolve → `Path::exists` 检查 → 缺失 `General("<path> does not exist")` exit 1 → `execute_command_inner` → work：`scan_tables()` 空 → 无输出 `Success`；否则逐表 `scan_columns` 排序 → DDL 行 `emit_stdout`）（design D6/D3）
- Required changes: DDL 纯函数 + `Command::Schema` 分支
- Preserve: 系统表不可 SQL 查询现状（走内部 catalog API）；锁冲突经 `open_error_status` → exit 4；DEFAULT 不输出
- Forbidden: 不修改 catalog；不做非持久化约束推断；不扩 SQL 面
- Test witness: RED——lib 单测（新子模块 `#[cfg(test)]`）`ddl_generator_renders_types_and_constraints`（手工构造 CatalogRow + 2 CatalogColumnRow：Int PK + String NOT NULL → 断言含 `CREATE TABLE "users"`、`"id" INT PRIMARY KEY`、`"name" STRING NOT NULL`、不含 255）；集成 `test_schema_outputs_ddl`（主命令建表含 NOT NULL → `rtsql schema app` exit 0 + stdout 含表名/列名/约束词）、`test_schema_missing_db_errors`（exit 1 + stderr 含 `does not exist` + 无文件创建）、`test_schema_empty_db_no_output`（new 后直接 schema → exit 0 + stdout 空）
- GREEN condition: 1 lib 单测 + 3 集成用例绿；既有零回归
- Verification: `cargo test --lib && cargo test --test cli_test`（exit 0）
- Stop when: catalog 读 API 无法满足列序/约束还原（→ Blocker Handoff）

**Invariants**

- 主命令 `rtsql <db> "<sql>"` 合法输入行为零变化：渲染、退出码、静默建库、close checkpoint、信号语义（cli_test 既有 25 用例零修改为硬约束）。
- `resolve_db_path` 对外语义与文案零变化；`render.rs` 零修改；`ExitStatus` 枚举与映射不动；pipeline/planner/storage 层零修改。
- 开库子命令的锁冲突（exit 4）与格式拒绝（exit 1）语义由 `FileStorage::open` 既有链自然继承，不新建错误路径。
- 裸名解析规则（含 `/` 直用）不变；子命令的 db 参数一律经 `resolve_db_path`。

**Non-goals**

dump/restore/import（Iteration 001）；csv 依赖（Iteration 001）；planner/pipeline/storage 修改；`render.rs` 修改；退出码扩展；DEFAULT/非 255 String 长度保真；REPL/serve/key；`--format` 之外的子命令选项（除 import `--csv` 属 Iteration 001）。

**Acceptance**

- R1 分发场景：`list/new/schema` 三命令分发可达、主命令零回归（T1 用例 + 既有 25 零修改）。
- R-new 全 4 场景（T3 3 用例 + lib 建库链既有测试）。
- R-list 全 3 场景（T4 2 用例 + T2 helper 单测）。
- R-schema 全 4 场景（T5 1 lib + 3 集成；锁冲突由既有 `test_lock_conflict_exit_4` 机制 + `open_error_status` 复用承载）。
- 全量：`cargo test --all` 全绿、clippy/fmt/validate 全 0/PASS（Iteration 000 内先跑一次全量门，T9 复跑为 change 级终门）。

**Verification**

- `cargo test --test cli_test`（新增约 8 用例 + 既有 25 零修改）
- `cargo test --lib`（DDL 生成 + resolve helper 新单测）
- `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`
- 通过条件：全绿全 0；失败含义：对应 task 契约的 GREEN condition 未满足，不得标记完成

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | R20 分析（R20 登记）+ clap 探针实证 + Bool/DEFAULT/plan_stage/csv 可达核实 + 基线 cli_test 25/0/2 实测（2026-09-09） |
| Design | PASS | design.md D1-D10（入口结构实证、退出码归类表、DDL 映射往返依据、new/list schema 流程） |
| Iteration Plan | PASS | tasks.md Iteration Plan + 平衡审计（000 骨架/001 数据面，依赖有序，验证/诊断边界明确） |
| Cycle Scope | PASS | 本文件 Cycle Scope：T1-T5，excluded scope 明确 |
| Task Contracts | PASS | T1-T5 契约自包含（目标符号、行为、测试见证、停止条件；Act 只读本文件 + change 内文档即可实施） |
| Traceability | PASS | tasks.md RTM 全 Covered，无 Simplified/Missing |
| Verification | PASS | 验证节命令与通过条件直接证明 Acceptance；无身份型证据 |

**Persisted Evidence**

- Mode: none

`none`：验证命令均低成本可重跑，Act Response 的命令 + ≤20 行决定性输出 + 退出码足以承载 Gate 5 与 Review；无一次性环境、无 Incident 现场、无摘要丢失结构的风险。Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- clap `global = true` 的 `--format`：接受位置拓宽（`rtsql --format json list` 合法），行为只增不减；探针已验证 Option 位置参数与子命令共存，但 global flag 组合未单独探针——非实质（行为只增不减且主命令场景由既有测试锁定），Act 遇实证相悖按 T1 Stop when 处理。
- `new` 信号中断（open 后、close 前）可留半初始化文件：重试 `new` 得 already-exists，手工删除即可；不引入事务性创建（proposal Impact 注记）。
- 手动 usage 文案与 clap 自动文案不同（spec 只锁 exit 2 + stderr）。
- dump/restore/import 的实施事实在 Iteration 001 展开时由 Plan 按 Map 补充为后继 Cycle 的 Plan Context；本 Cycle 文件不承载。
- 非实质留 Act：子命令文件组织、new/list/schema 的 stderr 文案细节（保留 spec 锁定关键词 `already exists`/`does not exist`/`database is locked`）、list 过滤实现细节（extension 判定方式）。

## Act Response

- Status: blocked（2026-09-09，T5 R-schema S1 被 NOT NULL 持久化缺口阻塞，见 Blocker Handoff；T1-T4 完成且全绿）

**Implemented**

- **T1（完成）**：`CliArgs` 扁平位置参数 → `db: Option<String>` + `sql: Option<String>` + `--format`（`global = true`）+ `command: Option<Command>`；`Command` 六变量全定义（New/List/Schema/Dump/Restore/Import{--csv}）；`execute_command` 分发 + 手动 usage 臂（缺参 exit 2）；主命令全链路（`execute_main_command` → `execute_command_inner` → `run_sql`）逐字保留；dump/restore/import 占位分发（Usage exit 2，Iteration 001 填充）。RED：`test_subcommand_dispatch_list_runs` 对旧 clap 缺参路径失败（stderr 含 "required arguments were not provided"）→ 实现后 GREEN。
- **T2（完成）**：`rtsql_home()`/`db_dir()` 提取自 `resolve_db_path` 内联段，错误文案逐字不变；既有 2 单测先行 GREEN 确认后新增 `test_db_dir_env_cases`（编译 RED → 实现 → GREEN）。
- **T3（完成）**：`lifecycle::new_db`——已存在（含 0 字节）拒绝 exit 1 + `already exists` → `create_dir_all` 父目录 → `execute_command_inner` 空 work 建库 + close checkpoint → 静默 exit 0；3 集成用例 RED（对 T1 占位 exit 2）→ GREEN；既有静默建库契约 `test_new_database_created_silently` 零修改保持。
- **T4（完成）**：`lifecycle::list`——`db_dir()` 枚举 `.db` 常规文件（目录缺失 → 空行集），名称排序，`render` 行集（name/size_bytes）输出；2 用例 RED → GREEN；T1 的 dispatch 用例按契约收紧为 exit 0 + 行集断言。
- **T5（部分完成，阻塞）**：DDL 生成器 `create_table_sql` 纯函数（类型映射/恒双引号/约束序 PK→NOT NULL→UNIQUE/不含存储长度）+ 2 lib 单测 GREEN；`lifecycle::schema`（存在性检查 → catalog scan_tables/scan_columns 按 column_index 排序 → 逐表 DDL 行 emit_stdout）；3 集成用例中 S3 不存在（exit 1 + `does not exist`）与 S2 空库无输出 GREEN；**S1「表结构 DDL 输出」RED 不转绿**——NOT NULL 在建库链未持久化（见 Blocker Handoff），`test_schema_outputs_ddl` 保持 RED 作为缺口见证（未改断言）。

**Changed Files and Symbols**

- `src/cli/mod.rs`：`CliArgs`（Option 化 + subcommand）、`Command`（新枚举，clap Subcommand derive）、`execute_command`（分发重构）、`execute_main_command`（新，主命令臂）、`subcommand_placeholder`（新，dump/restore/import 占位）；`run`/`execute_command_inner`/`run_sql`/`ExitStatus` 零修改。
- `src/cli/resolve.rs`：`rtsql_home()`/`db_dir()`（新 pub(crate) helper）、`resolve_db_path`（内联段替换为消费 helper，对外行为不变）。
- `src/cli/lifecycle.rs`（新文件）：`new_db`/`list`/`schema`/`create_table_sql`/`quote_ident`/`column_type_sql` + `#[cfg(test)]` 2 单测。
- `tests/cli_test.rs`：新增 10 集成用例（T1×2、T3×3、T4×2、T5×3）+ `test_subcommand_dispatch_list_runs` 收紧；既有用例零修改。

**Deviations from Plan**

1. T1 契约将 `test_missing_sql_arg_exit_2` 列于 RED 组；实际该用例变更前即 GREEN（旧 clap 自动缺参路径同为 exit 2）——它是行为保持守卫而非 RED 见证；真 RED 见证为 `test_subcommand_dispatch_list_runs`（对旧 clap 文案的否定断言）。用例按契约放宽形式落地（exit 2 + stderr 非空 + 非 clap 缺参文案），T4 后收紧。
2. `schema()` 不接收 `--format` 参数：DDL 文本行非行集/受影响行数，`render`/`kind` 不适用（契约目标行为亦未列 format；`--format` 对 schema 为无害可忽略项）。
3. 分发占位臂随任务逐个替换（T1 全占位 → T3/T4/T5 接线），占位匹配用 `unreachable!` 收窄已实现臂——同一「先以占位错误分发到位」契约的中间形态选择，非实质。

**Blocker Handoff**

- **发现位置**：T5 GREEN 验证（Gate 5）；task T5；requirement R-schema scenario S1（表结构 DDL 输出）。
- **Plan 预期**：design D6 / Plan Context 断言 catalog 持久化面含 not_null/unique（catalog 序列化格式确实含，catalog.rs:651-671），schema 经 `scan_columns` 即可还原约束输出。
- **实际（证据链）**：NOT NULL/UNIQUE 在 SQL 建库链于执行器→存储边界被丢弃——
  1. planner 侧约束解析正常：`ColumnDef::to_schema_column()` 将 NotNull/Unique 装入 `ColumnSchema`（`src/executor/plan.rs:205-224`）；
  2. `CreateTableExecutor` 将 `ColumnSchema` 经 `to_tuple()` 压成 `(name, ColumnType)`，约束丢弃（`src/executor/create_table.rs:46-54`）;
  3. `TableManager::create_table` 签名只收 `Vec<(String, ColumnType)>`，无约束通道（`src/storage/data/table_manager.rs:209-214`）；
  4. catalog 列行写入硬编码 `not_null: false, unique: false`（`src/storage/data/table_manager.rs:282-293`）。
  运行证据：`cargo test --test cli_test test_schema_outputs_ddl` → 输出 `CREATE TABLE "items" ("id" INT PRIMARY KEY, "label" STRING);`（NOT NULL 丢失），断言 `"label" STRING NOT NULL` 失败。
- **影响**：R-schema S1「stdout 输出含……全部列及约束的 DDL 文本」不可满足；UNIQUE 同病。DDL 生成器本身正确（lib 单测以手工构造行证明渲染无缺陷）；PK 不受影响（独立持久化于 `pk_column`）。
- **修复所需超出当前契约**：需修改 `CreateTableExecutor`、`TableManager::create_table` 签名、TableMeta/`open_or_init` 重载路径，并波及 lib `Database::create_table` API 与多个既有测试——与 cycle Invariants「planner/pipeline/storage 层零修改」、proposal Out of Scope「storage 层任何修改 / lib API 签名变化」、T5 Forbidden「不修改 catalog」直接冲突。且 NOT NULL 当前全引擎无运行时语义（解析后即弃）：持久化后是否在 INSERT/恢复路径执行、restore 对 NOT NULL DDL 的语义，均为新设计决策，必须返回 Plan。
- **进度现场**：T1-T4 完成且验证绿；T5 的 DDL 生成器 + schema 骨架已实现（S2/S3 绿）；未开始 Iteration 001（T6-T8）与 T9 终门。`openspec validate` PASS。
- **工作区状态**：修改 `src/cli/mod.rs`、`src/cli/resolve.rs`、`tests/cli_test.rs`；新增 `src/cli/lifecycle.rs`；未 commit。
- **恢复条件**：Plan 就 NOT NULL/UNIQUE 持久化做出设计决策（建议方向：将 `CreateTableNode.columns` 的约束经执行器传入 `TableManager::create_table` 并写入 catalog 现有字段；需同步定义 INSERT 期是否强制、恢复重建路径是否回读），产出修订契约（replan Cycle 或修订本 Cycle 契约）后 Act 恢复。

**Blocker Resolution**

None（未恢复）

**Self-Review**

- Plan compliance: T1-T4 按契约全绿；T5 部分合规（S1 被契约外缺口阻塞）
- Full diff reviewed: yes——`git diff`（mod.rs/resolve.rs/cli_test.rs）+ lifecycle.rs 全文过目；跨任务交互检查（占位臂与实现臂共存、既有 25 用例零修改全绿、`--format` global 化对主命令无影响）无计划外修改
- Critical findings unresolved: 0（阻塞为计划外设计缺口，非实现缺陷）
- Important findings unresolved: 0
- Minor findings unresolved: 0（1 项已修复：测试内冗余 `mut` 重赋值）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 目标测试 | `cargo test --test cli_test` | `test result: ok. 27 passed; 0 failed; 2 ignored` | PASS（25 既有零修改 + 2 新增） |
| T2 目标测试 | `cargo test --lib cli::resolve` | `test result: ok. 3 passed; 0 failed` | PASS（既有 2 零修改 + db_dir 1） |
| T3 目标测试 | `cargo test --test cli_test` | `test result: ok. 30 passed; 0 failed; 2 ignored` | PASS |
| T4 目标测试 | `cargo test --test cli_test` | `test result: ok. 32 passed; 0 failed; 2 ignored` | PASS |
| T5 lib 单测 | `cargo test --lib cli::lifecycle` | `test result: ok. 2 passed; 0 failed` | PASS |
| T5 集成 | `cargo test --test cli_test` | `test result: FAILED. 34 passed; 1 failed; 2 ignored`（唯一失败 `test_schema_outputs_ddl`，即阻塞项） | BLOCKED（S1）/ S2、S3 PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | `Finished \`dev\` profile`，0 warning | PASS |
| 格式 | `cargo fmt --check` | 无 diff 输出 | PASS |
| OpenSpec | `openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` | `Change ... is valid` | PASS |

**Persisted Evidence**

None required（阻塞缺口可低成本复现：`cargo test --test cli_test test_schema_outputs_ddl`，失败输出 ≤20 行已录入 Blocker Handoff；无一次性环境、无 Incident 现场）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None（计划/设计缺口发现，非已验证可重复操作或故障） |

**Remaining Issues**

1. `test_schema_outputs_ddl` 保持 RED——R-schema S1 缺口见证，等待 Plan 设计决策（见 Blocker Handoff）。
2. dump/restore/import 为 Usage 占位（exit 2），属 Iteration 001 契约范围。
3. 裸名冲突场景（`$RTSQL_HOME/db/list.db` 存在时 `rtsql list "SELECT 1"` 不开库 + 路径形式可打开）由 RTM 指定的 dispatch 用例承载分发优先断言；如 Plan 认为需独立见证用例可在 Review 提出。

**Commit or Diff Reference**

未 commit（工作树现场：`src/cli/mod.rs`、`src/cli/resolve.rs`、`tests/cli_test.rs` 修改，`src/cli/lifecycle.rs` 新增；另含本 change 目录与 R20 登记等规划产物）

## Plan Review

- Review Result: rework-required（2026-09-09）

**Findings**

1. **T1-T4 独立验证 PASS（非阻塞）**：Plan 独立重跑 `cargo test --all` → lib 192/0，各集成套件全绿，唯一失败 `test_schema_outputs_ddl`（即阻塞项自身，34 passed / 1 failed / 2 ignored）；`cargo clippy --all-targets -- -D warnings` 0 warning；`cargo fmt --check` 干净；`openspec validate` PASS。diff 逐项比对契约：`CliArgs` Option 化 + `Command` 六变量 + global `--format` + 手动 usage 臂符合 T1；`execute_command_inner`/`run_sql`/`render.rs`/`ExitStatus` 零修改核实；`rtsql_home`/`db_dir` 提取与错误文案逐字保留符合 T2；`lifecycle.rs` 的 new/list 实现与 D4/D5 一致（空 work 复用编排、不开库枚举、目录缺失空行集）；10 新测试与契约一一对应，既有 25 用例零修改（git diff 核实）。
2. **T5 阻塞核实成立（阻塞）**：Plan 独立复核 NOT NULL/UNIQUE 丢弃链——`CreateTableExecutor` 经 `to_schema_column().to_tuple()` 压平丢约束（`src/executor/create_table.rs:46-54`）；`TableManager::create_table` 签名无约束通道（`table_manager.rs:209-214`）；catalog 列行硬编码 `not_null: false, unique: false`（`:282-293`）。Act 的证据链、失败输出与修复影响判断全部属实。
3. **Act 偏差 1-3 均非实质，接受**：① `test_missing_sql_arg_exit_2` 变更前即 GREEN（行为保持守卫而非 RED 见证）——记录诚实，用例形式符合契约放宽定义；② schema 不接 `--format`——DDL 文本非行集，契约目标行为未含 format，接受（`--format` 对 schema 为可忽略项）；③ 占位臂 `unreachable!` 收窄——同一契约的中间形态选择。
4. **Remaining Issues #3（裸名冲突独立见证）**：接受现状不加用例——分发优先断言由 `test_subcommand_dispatch_list_runs` 承载（RTM 指定），路径形式打开由既有路径参数测试覆盖（`test_path_arg_used_as_is` 等），无 Acceptance 缺口。
5. **Minor**：`lifecycle.rs` `list` 对 `metadata()` 失败取 size 0 的降级行为（非契约细节，可观察行为合理）；无其他发现。

**Deviation Classification**

PLAN-OMISSION——Plan 在 R20-F9/design D6 断言「约束数据已就绪」时只核实了 catalog 结构字段存在（真）与序列化格式含 not_null/unique（真），未核实 SQL 建库链的写入路径是否填充该字段（假：执行器→存储边界丢弃）。阻塞由该遗漏导致，非 Act 偏离；Act 的 Blocker Handoff 证据链完整且影响判断准确。

**Acceptance Gaps**

- **R-schema S1「表结构 DDL 输出」**：`test_schema_outputs_ddl` RED——schema 输出缺 NOT NULL（UNIQUE 同病）；根因在上游持久化缺口，DDL 生成器本身正确（2 lib 单测以手工构造行证明）。S2/S3 GREEN；S4 锁冲突由 `open_error_status` 复用路径承载（RTM 既定）。其余 Iteration Acceptance（R1 分发/R-new/R-list）已满足。

**Convergence**

N/A（首次 Review，initial Cycle 无父 gap 比较）

**Evidence**

- `cargo test --all` → lib `192 passed; 0 failed`；cli_test `FAILED. 34 passed; 1 failed; 2 ignored`（唯一失败 `test_schema_outputs_ddl`）
- `cargo clippy --all-targets -- -D warnings` → 0 warning；`cargo fmt --check` → 干净；`openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` → valid
- 代码核实：`src/executor/create_table.rs:46-54`、`src/storage/data/table_manager.rs:209-214/282-293`、`src/executor/plan.rs:160-230`（ColumnDef.constraints → ColumnSchema 字段完整）、`src/parser/value.rs:26`；调用方普查：`Database::create_table` 测试/bench 调用 76 处（签名必须保持），`TableManager::create_table` 直调 5 处（database.rs:101、executor/create_table.rs:71、transaction/manager.rs:335/530、data_page.rs:161；另 data_scan.rs:527 为 Database 路径）

**Follow-up Decision**

创建 **001-rework**（同 Iteration 目录）：修复需要新执行契约——涉及 storage 建库链（`TableManager` 新约束通道 + catalog 写入填充）与 `CreateTableExecutor` 约束透传，与本 Cycle Invariants「storage 层零修改」、proposal Out of Scope、T5 Forbidden 直接冲突，不构成当前 Cycle 有限修复。两项契约语义决策由 Plan 按 Scope Control 裁定并写入 rework 契约：① **运行时语义 = 仅元数据持久化**（不引入 INSERT/恢复路径强制——当前引擎约束解析后即弃、无运行时语义，spec R-schema 只要求输出真实持久化约束；强制执行属新能力，不在本 change 需求内）；② **API 兼容 = `Database::create_table` 与 `TableManager::create_table` 签名零变化**（前者内部映射 false/false 与今日行为逐字一致，76 处测试调用零波及；新增 `create_table_with_constraints` 供 SQL 路径使用）。修复完成既有 Acceptance（R-schema S1），requirement/验收边界不变，不构成 replan。

**Iteration Plan Update**

None（Iteration Map 不变：000 范围/依赖/验收不变，001 待 000 accepted 后展开）

**Next Cycle**

`001-rework.md`（同 Iteration 目录，repair items：T5-R1 约束持久化通道、T5-R2 R-schema S1 见证转绿）

**Next Iteration**

None（Iteration 000 未完成；accepted 后展开 `../001-data-plane/`）
