# Iteration 000 / Cycle 000-initial: import 输入面（stdin / 免建表导入 / `:memory:` 豁免）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 默认假设 1-6、10（stdin `-` 文件位、import 永不创建数据库文件、自动建表仅触发表缺失、推导数据源全量数据行与空字段语义、错误分类 General/Sql 分界、零数据行建表、文档面）；用户裁定 1/2/4（冲突点名拒绝、自动建表无主键、猜测面仅 INT/FLOAT/STRING）；design D1-D5、D9（测试放置）。
- Excluded scope: DEFAULT 声明面（T4-T6，Iteration 001）；import 攒批提交（MS22）；Bool/Date CSV 猜测；JSON 输入格式；`:memory:` 其他生命周期子命令豁免；dump/restore/export 通道变更。

**Objective**

`rtsql import <db> <table> <file|-> --csv` 三形态端到端可用：stdin 与文件位等价导入既有表；目标表不存在时按 CSV 表头自动建表（类型推导矩阵、冲突点名拒绝零副作用、无主键、首列重复值合法、零数据行建全 STRING 表）；`:memory:` 组合可达（`--key` exit 2、`--readonly` 维持全局拒绝）；文件库缺失维持 exit 1 零文件创建；既有表导入语义与错误面逐字节零回归。

**Investigation Facts**

- Current Baseline: 分支 master @ 4e1944f；工作区含 MS28 实施面（change `2026-10-01-ms28-cli-concurrency-checkpoint` 双 Iteration accepted，收尾提交并行进行中——归档与语料合并已在工作区，提交未完成；本 change 代码面与之无交集）与本 change 规划产物（未跟踪）。全量 **1657 tests / 0 failures / 2 ignored**（MS28 Iteration 001 Act Response 分批 M19 + Plan Review accepted 采信，2026-10-01；Act 首跑分批即新鲜基线复认）。MS28 后 `import_csv` 与 `execute_command_inner(_mode)` 均带 `lock_wait: Option<Duration>` 参数（锁冲突经预算 exit 4）——本 change 穿线保持该签名。
- Current-State Evidence（2026-10-01 Plan 读码，行号以当前 revision 为准）：
  - `src/cli/mod.rs:138` `Command::Import { db, table, file, csv }`（`file: String` 文件位、`csv: bool` 显式开关）；`:269-285` 分派臂调 `lifecycle::import_csv(db, table, file, *csv, args.format, args.key.as_deref(), lock_wait)`；`:339-346` `lifecycle_db_target` 把 `Import { db, .. }` 归入 Some 臂 → `:247-253` `:memory:` 守卫（`target == MEMORY_DB` → Usage exit 2）——T3 豁免点；`:257-259` `--readonly` × 子命令全局守卫（memory import 无需新代码）；主命令 `:memory:` × `--key` 拒绝文案先例 `:372-383`。
  - `src/cli/lifecycle.rs:585-729` `import_csv`：`:595` `--csv` 必需（Usage exit 2，先于一切 IO）→ `:600-606` `resolve_db_path` + `!exists` → General「does not exist」（exit 1，保持）→ `:610` `execute_command_inner(&db_path, key, lock_wait, work, sigint, sigterm)` → work 内 `:616-625` `db.get_table(&table)`（缺失 → General「table {table} does not exist in {path}」——T2 自动建表替换点）→ `:626-640` `csv::Reader::from_path(&file)` + `reader.headers()`（T1 stdin 分支点）→ `:644-664` 表头双向匹配（既有表路径保持）→ `:668-674` records 预收集（`row k of n` 定位）→ `:678-692` 逐行 `csv_value(field, col_type)` → `sql_literal` → `:696-716` `INSERT INTO {quote_ident(&table)} VALUES ({values})` 经 `db.execute_sql` auto-commit、fail-fast（`Response::Error` → `ExitStatus::Sql` exit 3）→ `:718-722` render Affected。
  - `:573-580` `read_restore_input`——stdin `-` 先例（`std::io::read_to_string(std::io::stdin())`）；import 为流式 reader 需 `csv::Reader::from_reader(std::io::stdin().lock())` 形态。
  - `:734-762` `csv_value(field, col_type)`——空字段：非 String → NULL、String → 空串；Int i64/Float f64/Bool 大小写不敏感/日期族 String 透传（INSERT 强制解析）；`:449` `sql_literal`；`:698` `quote_ident` 可用（同文件）。
  - `src/pipeline.rs:777-809` `infer_ctas_column_types(columns: &[String], rows: &[Vec<Value>]) -> Result<Vec<StorageColumnType>, PlanError>`——私有 `fn`（T2 扩 `pub(crate)`）；NULL → None 参与任意、同型保留、Int∪Float → Float、冲突 → `PlanError::CtasColumnTypeConflict("{name} ({t1}, {t2})")`、全 NULL/空 → `String(255)`；`:812-822` `ctas_value_type`（Value → 类型）。`StorageColumnType = crate::storage::page_format::ColumnType`（`pipeline.rs:18`）。
  - CTAS「首列隐式主键」是 CTAS 臂显式选择（pipeline.rs `execute_ctas_routed` 内 `node.columns.first()`）；planner `extract_primary_key`（ddl_dml.rs:753）无声明返回 None，普通建表无 PK 合法（无键行语义，MS10-T05）——自动建表 DDL 不写 PRIMARY KEY 即可，零引擎改动。
  - `src/cli/mod.rs:510-573` `execute_command_inner_mode(db_path, key, readonly, in_memory, lock_wait, work, sigint, sigterm)`——memory 臂 `Database::open_in_memory()`（T3 路由目标）；`sigint_future`/`sigterm_future` 与 `open_error_status` 分类复用。
  - 存量测试：`tests/cli_test.rs` import 组 7 用例（`:1582` 基本与乱序、`:1629` 类型与空字段、`:1667` 转换 fail-fast、`:1698` 表头不匹配、`:1747` **`test_import_missing_table_or_db`——表缺失臂待校准**、`:1783` 引号转义、`:2957` 转义表名）；子进程测试经 `run_cli_env`（MS28 校准先例，env 注入 + `env_remove("RTSQL_BUSY_TIMEOUT")`）。
- Code and Critical Path: CLI 参数 `Import{db,table,file,csv}` → `execute_command` 分派（`:memory:` 豁免判定 + `--key` 拒绝新增）→ `import_csv`（memory 路由开关 → `execute_command_inner[_mode]`）→ work：get_table（缺失→自动建表：读 CSV〔文件|stdin〕→ guess → infer → 冲突 General → DDL `execute_sql` → 恒等映射）→ 既有逐行插入循环（零改动）→ render Affected。推导冲突判定在任何写入前（数据预收集后）＝零副作用结构性成立。

**Implementation Guidance**

实施顺序 T1 → T2 → T3（D2 递进：stdin 通道是自动建表 stdin 形态与 `:memory:` 管道组合的前置）。关键点：

- D1：CSV 读取分支 `if file == "-" { csv::Reader::from_reader(std::io::stdin().lock()) } else { csv::Reader::from_path(&file)? }`——两分支错误文案沿用既有 `failed to read {file}` 风格；records 预收集对 stdin 读到 EOF（与 restore 同语义）。
- D2：自动建表路径在 `get_table` Err 臂展开；`csv_field_guess` 为 lifecycle.rs 私有纯函数；推断后 DDL 生成用 `quote_ident`（表名与列名均双引号包裹——planner 对带引号标识符同样 lowercase 归一，ddl_dml.rs:859）；转换用列类型直接取推导结果（Int/Float/String(255)），**不重取 catalog**（规避 `get_table` 大小写归一化二次查询）；插入循环、行定位、错误文案逐字节复用。
- D3：f64/i64 解析与既有 `csv_value` 同口径（`from_str` 语义，不特判 inf/NaN）。
- D5：`lifecycle_db_target` 对 Import 放行的实现形态（match 臂内排除 vs 守卫处特判）非实质；memory 形态下 `resolve_db_path`/存在性校验全部跳过（`:memory:` 不是文件）。
- 测试放置（D9）：新文件 `tests/import_input_surface_test.rs`（子进程 e2e，沿 `run_cli_env` 先例）；cli_test 校准仅 `test_import_missing_table_or_db` 表缺失臂。

**Task Contracts**

### T1: import stdin 通道

- Requirement/Scenario: cli-noninteractive-shell import MODIFIED「stdin 导入既有表」
- Depends on: None
- Targets: `src/cli/lifecycle.rs::import_csv`（CSV reader 构造段 :626）
- Current behavior: `file` 仅接受路径；`-` 被 `csv::Reader::from_path` 当路径打开失败 → General「failed to read -: …」exit 1
- Required behavior: `file == "-"` 时经 `csv::Reader::from_reader(std::io::stdin().lock())` 读 stdin；其余路径形态逐字节不变；RFC4180、records 预收集、行定位、逐条 auto-commit、输出渲染零变化
- Preserve: `--csv` 必需检查先行序；`resolve_db_path`/存在性校验先行序；既有错误文案与退出码；`lock_wait` 穿线
- Forbidden: 不新增 flag；不改 restore 的 stdin 语义；不改表头匹配与转换逻辑
- Test witness: `tests/import_input_surface_test.rs`——**RED**：`printf 'a,b\n1,x\n' | rtsql import app t - --csv` 修复前失败（exit 1 路径打开失败）；GREEN：stdin 导入既有表 exit 0 + `{"affected_rows":1}` + SELECT 比对
- GREEN condition: stdin 用例全绿 + 既有 import 矩阵零回归
- Verification: `cargo test --test import_input_surface_test`；`cargo test --test cli_test import`
- Stop when: stdin 流式 reader 与 records 预收集组合出现平台性挂起（如管道未关闭不返回）——返回 Plan 裁定

### T2: 免建表导入（自动建表）

- Requirement/Scenario: cli-noninteractive-shell import MODIFIED「目标表不存在自动建表（类型推导）」「同型列推导与全 NULL 列」「零数据行（仅表头）自动建表」「类型冲突点名拒绝零副作用」「自动建表无主键（首列重复值合法）」「自动建表经保留名/重复列名守卫拒绝」
- Depends on: T1（stdin 形态复用同一读取分支）
- Targets: `src/cli/lifecycle.rs::import_csv`（`get_table` Err 臂 + 新 `csv_field_guess` + DDL 生成）、`src/pipeline.rs::infer_ctas_column_types`（可见性 `pub(crate)`，逻辑零改动）
- Current behavior: 表缺失 → General「table {table} does not exist in {path}」exit 1；无任何建表能力
- Required behavior: `get_table` 失败时自动建表——读全量 CSV（headers + records）→ 逐字段 `csv_field_guess`（空→Null、i64→Int、f64→Float、其余 String）→ `infer_ctas_column_types(&headers, &rows)` → `Err(CtasColumnTypeConflict(msg))` → `ExitStatus::General`（exit 1，文案含列名与冲突类型，零副作用）；`Ok(schema)` → DDL `CREATE TABLE {quote_ident(table)} ({quote_ident(col)} INT|FLOAT|STRING, ...)`（无主键）经 `db.execute_sql` → 失败 `ExitStatus::Sql`（exit 3，既有 SQL 失败语义）→ 插入转换列类型直接取推导结果（恒等列映射，不重取 catalog）→ 复用既有逐行插入循环零改动（空字段：String 列空串、Int/Float 列 NULL）
- Preserve: 既有表路径（`get_table` Ok 臂）逐字节不变——表头双向匹配、`csv_value` 转换、fail-fast、输出；冲突/建表失败零副作用（表不存在于失败结果）；`row k of n` 定位语义
- Forbidden: 不改 `infer_ctas_column_types` 矩阵逻辑；不引入 Bool/Date 猜测；不给自动建表声明主键或约束；不把推导冲突改道 SQL 失败通道（保持 General exit 1）；不改 `sql_literal`/`csv_value`
- Test witness: `tests/import_input_surface_test.rs`——**RED**：表缺失自动建表用例修复前失败（exit 1「does not exist」）；GREEN 矩阵：类型推导（INT/FLOAT/STRING + Int∪Float 提升 + 全 NULL 列 STRING + 零数据行全 STRING）、冲突点名列名零副作用（`rtsql schema` 零输出）、无主键首列重复值全导入、重复列名经建表守卫 exit 3 零副作用；`test_import_missing_table_or_db` 表缺失臂按新语义校准（库缺失臂不变）
- GREEN condition: 自动建表矩阵全绿 + 既有 import 组零回归
- Verification: `cargo test --test import_input_surface_test`；`cargo test --test cli_test`
- Stop when: 推导结果与 INSERT 写入门出现类型分歧（如 Float 列整数字段被写入门拒绝）——返回 Plan

### T3: `:memory:` import 豁免面

- Requirement/Scenario: cli-noninteractive-shell import MODIFIED「`:memory:` import 免建表可达」「`:memory:` import `--key` 拒绝」「`:memory:` 其他生命周期子命令维持拒绝」
- Depends on: T2（`:memory:` 组合的完整形态是 stdin + 自动建表）
- Targets: `src/cli/mod.rs::lifecycle_db_target`（Import 放行）、`src/cli/mod.rs` Import 分派臂（memory 判定 + `--key` 拒绝）、`src/cli/lifecycle.rs::import_csv`（memory 路由）
- Current behavior: `:memory:` × import → `lifecycle_db_target` 命中守卫 → Usage「:memory: is only supported by the main command」exit 2
- Required behavior: `rtsql :memory: import <table> <file|-> --csv` 可达——memory 形态跳过 `resolve_db_path`/存在性校验，经 `execute_command_inner_mode(..., in_memory=true, ...)` 开库（进程退出即弃）；`--key` 组合 → Usage exit 2（文案对齐主命令 `:memory:` × `--key`）；其余生命周期子命令的 `:memory:` 守卫零变化
- Preserve: `--readonly` 全局守卫先行序与行为；文件库路径（resolve/存在性/锁）零变化；`in-memory-database` R3 既有拒绝面（除 import 豁免外）
- Forbidden: 不给 stats/sample/profile 等其他子命令开豁免；不新增 `:memory:`×`--busy-timeout` 拒绝（MS28 已裁定接受并忽略）；不改 memory open 语义
- Test witness: `tests/import_input_surface_test.rs`——**RED**：`:memory:` import 用例修复前 exit 2 守卫拒绝；GREEN：stdin + 自动建表 + `:memory:` 组合 exit 0、`--key` exit 2、`:memory:` schema 维持 exit 2
- GREEN condition: memory 矩阵全绿 + 既有 `:memory:` 拒绝面用例（`in-memory-database` 域测试）零回归
- Verification: `cargo test --test import_input_surface_test`；`cargo test --test memory_db_test`
- Stop when: `execute_command_inner_mode` 的 work 闭包签名无法承载 memory 分支差异（契约前提失效）——返回 Plan

**Invariants**

- import 逐条 auto-commit 与 fail-fast 语义不变；失败前已提交行保留；成功输出渲染语义不变。
- 既有表导入路径（表头双向匹配、`csv_value` 转换、错误文案、退出码）逐字节零回归。
- import 永不创建数据库文件（文件库缺失 exit 1 零文件创建；`:memory:` 无文件）。
- `infer_ctas_column_types` 矩阵逻辑与其既有消费点（CTAS 臂）零变化；CTAS 语义（含首列隐式主键）不受影响。
- `csv_value`/`sql_literal`/`quote_ident`/`render`/`emit_stdout` 零改动。
- `--readonly` 全局守卫、`:memory:` 其余子命令守卫、`--key` 既有通道（文件库加密导入）、`RTSQL_BUSY_TIMEOUT` 通道、水位 env 校验全部零变化。
- 错误分类：推导冲突 General exit 1（写入前判定）；建表 DDL 失败 Sql exit 3；库缺失 General exit 1；锁冲突 exit 4。

**Non-goals**

- DEFAULT 声明面（Iteration 001）；import 攒批提交/持锁窗口治理（MS22，I077-②）；Bool/Date CSV 猜测；JSON/其他输入格式；自动建表约束声明（NOT NULL/UNIQUE/DEFAULT）；`:memory:` 其他子命令豁免；export 通道变更；列改名/改类型 schema 演进。

**Acceptance**

1. stdin 通道：`printf '…' | rtsql import <db> <table> - --csv` 导入既有表成功（exit 0 + affected 渲染 + SELECT 比对），与文件位等价（import MODIFIED「stdin 导入既有表」；T1）。
2. 自动建表：表缺失时按表头建表并导入——类型推导矩阵（同型/Int∪Float 提升/全 NULL→STRING/零数据行全 STRING）、无主键（首列重复值合法）、推导冲突点名列名与类型拒绝零副作用、重复列名等经建表守卫 exit 3 零副作用（import MODIFIED 五场景；T2）。
3. `:memory:` 豁免：`rtsql :memory: import <table> <file|-> --csv` 端到端可达（stdin + 自动建表组合 exit 0）、`--key` exit 2、其余子命令维持 exit 2（import MODIFIED 三场景；T3）。
4. 既有面零回归：既有表导入全矩阵、库缺失 exit 1 零文件创建、锁冲突 exit 4、`--readonly` 全局拒绝（import MODIFIED 既有场景；T1-T3）。
5. 全量分批零回归（基线 1657 不降）（T1-T3）。

**Verification**

- 逐 scenario 最简直接判定：`cargo test --test import_input_surface_test`（矩阵 1-3）、`cargo test --test cli_test`（4：import 组零回归 + 校准组）、`cargo test --test memory_db_test`（3：既有 `:memory:` 拒绝面）、全量分批 M19（5，MS28 收尾提交完成则以提交后基线复认）。
- RED 见证：T1/T2/T3 各自 Test witness 注明的修复前失败形态，记录于 Act Response。
- 人工步骤：无（全部自动可判定）。

**Gate 2 Readiness**

其余检查项 PASS（无 Missing requirement、无未批准裁剪、调查/设计/任务/追踪/验证闭合、无实质未知项、无身份型证据工程、Evidence 模式 none）。**2026-10-01 用户批准计划（Gate 1 需求与范围 + 四项场景裁定 + Gate 2 执行就绪一并批准，原话：「批准，交 openspec-act」）——Gate 2 全部 PASS。**

**Persisted Evidence**

- Mode: none

（Act Response 记录命令、决定性输出与退出码即足；全部验证可低成本重跑。）

**Risks and Notes**

- stdin records 预收集在管道未关闭时阻塞至 EOF——与 restore stdin 先例同语义，符合 Unix 管道直觉；测试用 `printf … |`（自然关闭）形态。
- `:memory:` import 的数据进程退出即弃——验收只断言命令自身行为（exit 0 + affected 输出），不跨进程断言持久化（`in-memory-database` 域既有语义）。
- f64 猜测接受 `inf`/`NaN` 文本——与既有 Float 转换同口径，非本 change 语义（不特判）。
- CSV 表头含引号包裹名/特殊字符：`quote_ident` 双引号包裹 + planner lowercase 归一；表头大小写与归一后列名的差异对自动建表路径不可见（恒等映射）——既有表路径的大小写敏感匹配行为不变（潜在既有边界，不在本 change 扩围）。
- 校准遗漏风险：`test_import_missing_table_or_db` 表缺失臂必须校准否则必失败——T2 兜底；全量分批时按 D9 就地处理并在 Response 记录（非实质）。
- 非实质留 Act：`csv_field_guess` 命名与放置、`lifecycle_db_target` Import 放行的实现形态、memory 路由的参数形态（flag vs 分派臂分支）。

## Act Response

- Status: blocked

**Implemented**

T1/T3 完整实现并 GREEN；T2 六场景实现并 GREEN，两场景阻塞于 Gate 5（见 Blocker Handoff）。

- `src/cli/lifecycle.rs`：`import_csv` 重构——新增 `memory: bool` 参数，memory 分支跳过 `resolve_db_path`/存在性校验并经 `execute_command_inner_mode(in_memory=true)` 开库（T3）；reader 双分支 `file == "-"` → `csv::Reader::from_reader(Box::new(std::io::stdin()))`，文件分支经 `from_path` 后 `into_inner()` 重包为 `Box<dyn Read + Send>` 统一两臂类型（错误文案 `failed to read {file}` 与 from_path 语义保持，T1）；`get_table` Err 改 `.ok()` 进入自动建表臂——records 预收集 → `csv_field_guess` 逐字段猜测 → `infer_ctas_column_types` 推导 → 冲突 General exit 1（`cannot infer column types from CSV: {col} ({t1}, {t2})`，写入前判定零副作用）→ `CREATE TABLE {quote_ident(table)} (…)` DDL 经 `execute_sql`（失败 Sql exit 3 `failed to auto-create table {t}: …`）→ 恒等列映射（schema = 表头 + 推导类型，不重取 catalog）→ 复用既有插入循环零改动（T2）；新私有纯函数 `csv_field_guess`（空→Null、i64→Int、f64→Float、其余 String）；孤儿清理：`db_path_in_work` 副本随表缺失错误文案移除；imports 增 `Value`/`PlanError`/`infer_ctas_column_types`；doc 更新（stdin/自动建表/`:memory:`）。
- `src/cli/mod.rs`：`lifecycle_db_target` 移除 `Command::Import` 臂（`:memory:` 守卫豁免 import）+ doc 更新；Import 分派臂增 memory 判定与 `--key` × `:memory:` Usage exit 2（文案对齐主命令）+ `memory` 实参穿线；`Command::Import` 帮助文案更新（`file` 接受 `-`、目标表可自动创建）；`:memory:` 守卫注释同步豁免注记。
- `src/pipeline.rs`：`infer_ctas_column_types` `fn` → `pub(crate) fn`（逻辑零改动，doc 注记）。
- `tests/import_input_surface_test.rs`（新，12 用例）：T1 stdin 2 + T2 自动建表 8 + T3 memory 2；夹具沿 `run_cli`/`run_cli_stdin` 先例（RTSQL_HOME 注入 + RTSQL_KEY/RTSQL_BUSY_TIMEOUT 隔离）。
- `tests/cli_test.rs`：`test_import_missing_table_or_db` 表缺失臂校准为自动建表语义（exit 0 + affected 1；库缺失臂不变）。

**Deviations from Plan**

- 测试命令形态：delta spec 场景写 `rtsql :memory: import t - --csv`，该语序在 clap 语法下不可达（`:memory:` 被主命令 db 位置消费、`import` 成子命令后 `t`/`-` 落到 DB/TABLE，`<FILE>` 缺失 exit 2）；`:memory:` 是 import 子命令的库实参（对齐既有拒绝面 `rtsql schema :memory:` 先例），测试按 `rtsql import :memory: t - --csv` 实证。实现面 targeting（`lifecycle_db_target`/分派臂/`import_csv` 路由）与契约一致，无偏差。spec 文本语序为笔误，待 Plan 校准（Minor）。
- D1 stdin 分支实现形态：`csv::Reader::from_reader(std::io::stdin().lock())` 在 if/else 表达式位置存在临时借用问题，改用 `Box::new(std::io::stdin())`（`Stdin` 为 owned handle）——语义等价，非实质。
- `db_path_in_work`：表缺失错误文案移除后成为孤儿副本，随改清理（Plan Context 未点名，按 Surgical Changes 孤儿清理）。

**Blocker Handoff**

- 发现位置：T2 GREEN 验证步——Test witness「自动建表无主键（首列重复值合法）」与「自动建表经保留名/重复列名守卫拒绝」两用例失败；命中 Gate 5「实际代码与契约存在实质冲突」。
- Plan 预期 vs 实际：
  1. Plan 调查事实：「planner `extract_primary_key` 无声明返回 None，普通建表无 PK 合法（无键行语义，MS10-T05）——自动建表 DDL 不写 PRIMARY KEY 即可，零引擎改动」。实际：`CreateTableExecutor::execute`（`src/executor/create_table.rs:74-81`）对 `primary_key: None` **默认取第一列作主键**（代码注释明写「如果没有指定主键，使用第一列作为主键」）；本引擎不存在表级无主键概念——「无键行」是行级语义（PK 值为 NULL 的行不入索引），不是无主键表。直证（脱离 import 路径）：`CREATE TABLE pkcheck (a INT, b STRING)` 成功后 `rtsql schema` 渲染 `CREATE TABLE "pkcheck" ("a" INT PRIMARY KEY, "b" STRING);`；重复首列值第二次 INSERT → `Execution error: Duplicate key`（exit 3，即失败用例所见）。
  2. Plan 调查事实：「保留名/重名视图/重复列名等常规建表守卫自然生效」。实际：保留名守卫存在（TableManager，exit 3，用例已绿）；**重复列名守卫不存在**——planner 与 TableManager 均接受 `CREATE TABLE dup ("a" INT, "a" INT)`（rc=0，schema 渲染 `CREATE TABLE "dup" ("a" INT PRIMARY KEY, "a" INT PRIMARY KEY);`）。
- 影响：批准的 Acceptance #2 中两场景（用户裁定 2「自动建表无主键」+ spec scenario「首列重复值合法」「重复列名守卫拒绝 exit 3」）无法在当前执行契约内达成——契约 Forbidden「不给自动建表声明主键或约束」与 Invariant「零引擎改动」封死契约内修复；改引擎默认行为属既有公开语义变更（存量无声明 PK 表 schema 均渲染首列 PRIMARY KEY），属新设计/范围问题，按 Gate 5 返回 Plan。
- 任务状态：T1 完成（GREEN）；T3 完成（GREEN）；T2 部分完成（六场景 GREEN：类型推导/INT∪FLOAT 提升、全 NULL 列、零数据行、冲突点名零副作用、保留名拒绝、库缺失 stdin 形态；两场景 BLOCKED）；`tests/cli_test.rs` 校准已写入且 import 组 7/7 GREEN；Gate 4 全量分批未执行。
- 修改文件与工作区：`src/cli/lifecycle.rs`、`src/cli/mod.rs`、`src/pipeline.rs`、`tests/import_input_surface_test.rs`（新）、`tests/cli_test.rs`；工作区另含 MS28 既有未提交项（本 change 之前已存在，未触碰）。已通过的 Gate：RED 见证、T1/T3 全场景与 T2 六场景的 Gate 3 局部验证。
- 恢复条件（Plan 裁定其一，均需用户批准）：
  (a) 引擎扩围——支持真无主键建表或为自动建表提供无 PK 通道（改 `CreateTableExecutor` 默认语义，破坏性）；
  (b) 修订 scenario——自动建表沿用「未声明 PK → 首列隐式主键」既有引擎语义，首列重复值不再要求合法（用户裁定 2 重新裁定，spec delta 同步修订）；
  (c) 重复列名场景——planner 建表通道补重复列名计划期守卫（扩围），或修订 scenario 改由 CLI 侧推导前点列拒绝（General exit 1，与推导冲突同通道）。
- Persisted Evidence：None required（两偏差均可低成本复现：`cargo test --test import_input_surface_test` 两失败用例 + Handoff 所列直证命令）。

**Blocker Resolution**

（未恢复）

**Self-Review**

- Gate 3 spec compliance（T1/T3 全场景、T2 六场景）：通过——scenario 逐条对齐，错误文案与退出码按契约，Invariants 未破坏（既有表路径逐字节保持——cli_test import 组 7/7 见证），Forbidden 未触碰（未新增 flag、未动 restore stdin 语义、未给自动建表声明约束）。
- Gate 3 code quality：diff 限于计划面；无计划外修改；命名与注释沿项目 MS 编号惯例；`csv_field_guess` 与 `csv_value` 同口径（D3）。
- 遗留 Minor：delta spec `:memory:` import 语序笔误（见 Deviations）；重复列名接受为范围外既有缺陷（见 Experience Candidates）。
- 未解决的 Critical/Important：无——两 Acceptance gap 已按 Gate 5 阻塞交接，不伪装解决。

**Verification Evidence**

- RED（实施前）：`cargo test --test import_input_surface_test` → **1 passed; 11 failed**（exit 1）——stdin 臂 `failed to read -: No such file or directory (os error 2)`（与 T1 契约预测一致）、自动建表臂 `table t does not exist in …` exit 1（与 T2 预测一致）、`:memory:` 守卫 exit 2（与 T3 预测一致）。
- GREEN（T1/T3 全部 + T2 六场景）：`cargo test --test import_input_surface_test` → **10 passed; 2 failed**（exit 1；仅余两 Blocker 场景）。
- 既有面：`cargo test --test cli_test import` → **7 passed; 0 failed**（exit 0）——import 组零回归 + 校准臂通过。
- 直证（引擎既有行为，脱离 import 路径）：`CREATE TABLE pkcheck (a INT, b STRING)` rc=0 后 `schema` 渲染 `"a" INT PRIMARY KEY`；`CREATE TABLE dup ("a" INT, "a" INT)` rc=0 且 schema 渲染双 PRIMARY KEY。
- 未执行：`memory_db_test`、全量分批——Gate 4 完整验证阻塞于 T2 两场景。

**Persisted Evidence**

None required（Mode: none 维持；全部验证可低成本重跑，Act Response 记录已足）。

**Experience Candidates**

- Issue 候选（当前 change 范围外、有证据的既有实质缺陷）：planner `build_create_table` 接受重复列名（`CREATE TABLE t ("a" INT, "a" INT)` rc=0），TableManager 亦不拒；`create_table_sql` 按列名匹配 PK 渲染成 `"a" INT PRIMARY KEY, "a" INT PRIMARY KEY`，该表 dump→restore 将触发 `MultiplePrimaryKey` 计划期错误——重复列名表被静默建成且不可 dump 往返。证据：本轮 probe2 直证 + `auto_create_duplicate_column_names_rejected` 用例输出（见 Verification Evidence）。建议 Recorder 立账；本 change 不落账。

**Remaining Issues**

- T2 两场景 Acceptance gap（见 Blocker Handoff）——Plan 裁定恢复路线后继续当前 Cycle。
- delta spec `:memory:` import 语序笔误——Plan 校准 spec 文本（`rtsql :memory: import …` → `rtsql import :memory: …`）。

## Plan Review

- Review Result: replan-required

**Findings**

独立核实（读码 + 直证复现，2026-10-01）：

1. **Plan 错误 ×2（阻塞根因，均独立核实成立）**：
   - 调查事实「普通建表无 PK 合法、零引擎改动」**错误**——`src/executor/create_table.rs:74-83` 对 `primary_key: None` 默认取第一列作主键（注释明写「如果没有指定主键，使用第一列作为主键」）。Plan 独立复现：`CREATE TABLE pkcheck (a INT, b STRING)` rc=0 后 `rtsql schema` 渲染 `"a" INT PRIMARY KEY`。根因：Plan 调查止于 planner 层（`extract_primary_key` 返回 None）与 `create_table_with_constraints` 签名，未追踪执行器默认臂；「无键行」是行级语义（PK 值为 NULL 的行不入索引）而非表级无主键；且 dump DDL 无法表达无主键——绕道建出不可 restore，结构性死路。
   - 调查事实「保留名/重名视图/重复列名等常规建表守卫自然生效」**部分错误**——保留名/重名视图守卫存在（Act 用例已绿），**重复列名守卫不存在**：Plan 独立复现 `CREATE TABLE dup ("a" INT, "a" INT)` rc=0、schema 渲染双 `PRIMARY KEY`（该形态 dump→restore 必触发 `MultiplePrimaryKey` 计划期错误——既有实质缺陷，Act 已登记 Issue 候选）。根因：Plan 把 CTAS 的输出列重复检查（`build_create_table_as_select` 内）错误泛化到普通建表通道。
2. **Act 偏离 3 项均非实质（核实接受）**：stdin 臂 `Box::new(std::io::stdin())`（if/else 表达式位置临时借用问题的等价解，语义与 `Stdin` owned handle 一致）；`db_path_in_work` 孤儿副本随表缺失错误文案移除（Surgical Changes 孤儿清理义务）；delta spec `:memory:` import 语序笔误（`rtsql :memory: import …` 在 clap 语法下不可达，正确形态 `rtsql import :memory: <table> <file|-> --csv`）——spec 文本由 Plan 在 replan 中校准。
3. **Minor（记录不返工）**：文件分支 `from_path` 后 `into_inner()` 重包统一两臂类型——实现细节合理；`:memory:` 分派臂 `--key` 拒绝文案对齐主命令——符合契约；`auto_create_duplicate_column_names_rejected` 等 2 失败用例保留为 replan 校准对象（不删除不弱化）。

**Acceptance Gaps**

- T2 两场景未满足：「自动建表无主键（首列重复值合法）」（要求引擎不具备的表级语义——Plan 错误 1）与「自动建表经保留名/重复列名守卫拒绝」（依赖不存在的 planner 守卫——Plan 错误 2）。
- 其余 Acceptance 全部 GREEN 在案（Act Response Verification Evidence：RED 1/11 → GREEN 10/12；`cli_test import` 7/7 含校准臂；T1/T3 全场景；既有表路径逐字节零回归）。

**Evidence**

- 独立读码：`src/executor/create_table.rs:74-83`（PK 默认臂）；`build_create_table`（ddl_dml.rs:799-953——Plan 首轮读码已在案且无重复列名检查，当时未识别为缺口）。
- 直证复现（本 Review 独立执行，非采信 Act）：两 CLI probe rc=0 + schema 渲染（Findings 1 所引）。
- Act Response 验证结论采信（覆盖未失效：工作区自结论后仅 replan 文档变化）：`import_input_surface_test` 10 passed / 2 failed、`cli_test import` 7 passed / 0 failed。

**Follow-up Decision**

两 Acceptance gap 的修复需要修订 requirement 语义（自动建表主键）与新增 planner 守卫（重复列名）——超出当前 Cycle 契约（Invariant「零引擎改动」、T2 Forbidden「不给自动建表声明主键或约束」），按 iteration-planning 分类为 **replan-required**。用户指令「工作阻塞，进行审计，给出解决」授权审计与解决；集中决策两问未获即时应答，**按审计推荐执行并标记可推翻**（原话引用见 001-replan Gate 2 Readiness）：主键语义 = 沿用引擎「未声明 PK → 首列隐式主键」（与 `create-table-as-select` 同款、引擎零改动、失败面收窄至「首列推导 INT 且重复值」——Float/String 首列重复合法）；重复列名 = planner 建表通道补计划期守卫（本 change 内根因修复，Act Issue 候选随之闭环，Issue 无需落账）。修订产物：proposal/design/tasks/spec delta 同步更新；后继 Cycle `001-replan.md` 承载修订后的 T2 契约与新增 T8。

**Iteration Plan Update**

Map 结构不变：000 = T1, T2, T3 + **T8（replan 增入——planner 重复列名守卫，T2 Acceptance 的依赖前置）**；001 = T4-T7 不变。T1/T3 已完成且验证结论未失效，replan Cycle 直接采信继承；T2 剩余工作 = 修订语义下的两场景校准。详见 tasks.md 与 001-replan.md。

**Next Cycle**

`openspec/changes/2026-10-01-ms29-import-input-and-default-declaration/iterations/000-initial/001-replan.md`（Cycle Type: replan）

**Next Iteration**

None（Iteration 000 未完成）
