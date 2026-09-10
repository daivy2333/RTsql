# tasks: MS10-T05 生命周期子命令（new/list/schema/dump/restore/import --csv）

> 状态：计划完成，Gate 2 待用户批准后交 openspec-act。
> 用户决策（2026-09-09，Gate 1 会话）：① import 目标表必须已存在 + 表头按名匹配；② dump 产物 = SQL 文本；③ 仅 new 建父目录；④ 开库子命令对不存在库报错 exit 1（new 唯一创建入口）。
> 默认假设（Gate 1 一并批准）：schema 输出 DDL 文本；list 输出 name+size_bytes；new 已存在报错；restore 非空库拒绝 + `-` stdin + 静默成功；import 逐条 auto-commit + affected_rows 输出；引入 csv crate；裸名冲突子命令优先；退出码归类表（proposal）。
> 审计依据：R20 分析（revision `a5b0a5f`）+ 本会话补充验证（clap 探针实证、Bool 字面量通道 `parser/value.rs:26`、csv crates.io 可达、基线 cli_test 25/0/2 @ `a5b0a5f`，2026-09-09）。

## Iteration Plan

### Iteration 000: 命令面骨架与元数据子命令

- Tasks: T1, T2, T3, T4, T5
- Depends on: None
- Stable baseline: `rtsql` 分发 `new`/`list`/`schema` 三个子命令且行为满足各自 spec 场景；主命令合法输入零回归（cli_test 25 用例零修改全绿）；`resolve_db_path` 外部语义不变
- Verification boundary: `cargo test --all` 全绿（既有零修改 + 新增子命令用例）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/cli/`、`Cargo.toml`（无新依赖）、`tests/cli_test.rs`
- Non-goals: dump/restore/import（Iteration 001）；planner/pipeline/storage 层修改

### Iteration 001: 数据面子命令

- Tasks: T6, T7, T8, T9
- Depends on: Iteration 000（入口分发骨架 + DDL 生成器）
- Stable baseline: `dump`/`restore`/`import --csv` 可用且满足各自 spec 场景；dump-restore 往返等价可验证；全量回归绿
- Verification boundary: `cargo test --all` 全绿；clippy/fmt/openspec validate 全 0/PASS
- Diagnostic boundary: `src/cli/`、`Cargo.toml`（csv 依赖）、`tests/cli_test.rs`
- Non-goals: 流式导出；dump 格式扩展（CSV 等）；planner 修改

**平衡审计**：Iteration 000 = 「多命令入口 + 库生命周期元命令」单一验收域——T1（入口）是全部子命令的公共前置，T2（目录 helper）是 new/list 的公共前置，T3/T4/T5 三个子命令共享信号编排、存在性检查（D3）与渲染复用，故障域连续（CLI 层），验证命令与诊断域完全重叠；拆分会产生"有入口无子命令"或"有子命令无入口"的不可独立验收中间态。Iteration 001 = 「数据进出通道」单一验收域——T6/T7 共享 dump-restore 往返验收与字面量/DDL 生成器，T8 引入唯一新依赖与唯一新解析面（CSV），与 T6/T7 的故障域（SQL 文本流 vs CSV 结构）相邻但同属数据面验收；合并两 Iteration 会使单 Iteration 承载 6 子命令 + 新依赖 + 全部场景（>20 测试），验证面过重。两 Iteration 各约 3-5 个新测试组，工作量适中。

## Tasks

### T1: 入口重构——Option 位置参数 + 子命令分发

- **Requirement/Scenario**: R1 参数化 CLI 入口与主命令 / S 子命令分发与主命令零回归、S 裸名与子命令名冲突
- **Depends on**: None
- **Targets**: `src/cli/mod.rs::CliArgs/run/execute_command`（`:63-112`）
- **当前行为**: `CliArgs { db: String, sql: String, format: Option<FormatArg> }` 扁平结构；无子命令
- **目标行为**: `db: Option<String>` + `sql: Option<String>` + `--format`（`global = true`）+ `#[command(subcommand)] command: Option<Command>`（`New{target}/List/Schema{db}/Dump{db}/Restore{db,file}/Import{db,table,file,--csv}`）；`command` 为 None 时进入主命令臂——`db`/`sql` 任一缺失 → `ExitStatus::Usage`（exit 2）；子命令臂仅分发给后续任务实现（本任务先以 `todo` 之外的方式返回 Usage 或最小占位错误均可，以 T3-T8 契约为准）
- **Required changes**: 结构体重构 + 分发 match + 手动 usage 分支（design D1）
- **Preserve**: 主命令合法输入全链路行为（`execute_command_inner` 两阶段编排、`run_sql`、渲染、退出码）零变化；`--format` 对主命令语义不变
- **Forbidden**: 不改 `execute_command_inner`/`run_sql`/`render.rs`/`resolve_db_path` 签名与行为；不实现子命令业务逻辑（后续任务）；不加退出码枚举
- **Test witness**: RED——`tests/cli_test.rs` 新增 `test_subcommand_dispatch_list_runs`（`rtsql list` exit 0/1/2 之一且不再是"缺 SQL"的旧解析——以 T4 完成后语义为准，本任务先断言 `rtsql`（无参）与 `rtsql db`（缺 sql）exit 2 保持）；`test_missing_sql_arg_exit_2`（`rtsql app` 无 SQL → exit 2）；跑 `cargo test --test cli_test` 观察新增 RED、既有 25 零修改
- **GREEN condition**: 新增用例绿 + 既有 25 用例零修改全绿
- **Verification**: `cargo test --test cli_test`（exit 0）
- **Stop when**: clap 可选位置参数与 global `--format` 组合产生与探针实证相悖的解析行为（→ Blocker Handoff）

### T2: resolve 目录 helper

- **Requirement/Scenario**: R-list（基目录推导）、R-new（集中区目录）；R2 名称解析（不变量保持）
- **Depends on**: None
- **Targets**: `src/cli/resolve.rs`
- **当前行为**: `RTSQL_HOME`/`HOME` 基目录推导内联在 `resolve_db_path`（`:16-25`），无独立访问器
- **目标行为**: 提取 `pub(crate) fn rtsql_home() -> Result<PathBuf, String>`（base 推导）与 `pub(crate) fn db_dir() -> Result<PathBuf, String>`（= base joined `db`）；`resolve_db_path` 改为消费 `rtsql_home()`，对外行为逐字不变
- **Required changes**: 提取两个 helper + `resolve_db_path` 内联段替换
- **Preserve**: `resolve_db_path` 三个既有场景（路径直用 / 裸名默认 / 双 env 缺失 Err）语义与错误文案零变化
- **Forbidden**: 不建目录；不改解析规则（含 `/` 直用语义）
- **Test witness**: 变更前 GREEN——resolve.rs 既有 2 单测先行确认全绿；新增 `db_dir` 单测（RTSQL_HOME 指定 / HOME 默认 / 双缺失 Err，复用 EnvGuard 顺序模式）
- **GREEN condition**: 新增单测绿 + 既有单测零修改绿
- **Verification**: `cargo test --lib`（resolve 模块段）
- **Stop when**: 提取导致 `resolve_db_path` 行为可观察变化（→ 返回 Plan）

### T3: new 子命令

- **Requirement/Scenario**: R-new 全部 4 场景
- **Depends on**: T1（分发臂）、T2（db_dir）
- **Targets**: `src/cli/`（`Command::New` 臂实现）
- **当前行为**: `rtsql new foo` 被主命令解析为"库名 new 缺 SQL" → exit 2
- **目标行为**: 存在性检查（已存在含 0 字节 → `General("... already exists")` exit 1，文件不动）→ `create_dir_all` 父目录（含裸名集中区 `db/`）→ 复用 `execute_command_inner`（work = 空闭包 Success）建库 + close checkpoint → 静默 exit 0（design D4）
- **Required changes**: `Command::New` 分支 + 存在性/建目录前置
- **Preserve**: 主命令静默建库契约不受影响；信号编排语义（Signaled 128+n）；不新增退出码
- **Forbidden**: 不做事务性文件创建；不 fsync 头（既有 D7 语义）；不修改既有建库路径
- **Test witness**: RED——`tests/cli_test.rs` 新增 `test_new_creates_db_and_dirs`（RTSQL_HOME 无 db/ 子目录 → `rtsql new app` exit 0 + 文件存在 + 主命令立即可用）、`test_new_path_creates_parents`（深层路径 exit 0 + 存在）、`test_new_existing_file_rejected`（先建文件 → exit 1 + stderr 含 `already exists` + 内容不变；含 0 字节文件变体）
- **GREEN condition**: 3 用例绿；既有用例零回归
- **Verification**: `cargo test --test cli_test`（exit 0）
- **Stop when**: open 建库链对新文件产生计划外副作用（→ Blocker Handoff）

### T4: list 子命令

- **Requirement/Scenario**: R-list 全部 3 场景
- **Depends on**: T1、T2
- **Targets**: `src/cli/`（`Command::List` 臂实现）
- **当前行为**: `rtsql list` 被解析为"库名 list 缺 SQL" → exit 2
- **目标行为**: `db_dir()` → `read_dir`（目录不存在 → 空行集）→ 过滤扩展名 `.db` 的常规文件 → `(name, size_bytes)` 按名称排序 → `render(kind(format), ["name","size_bytes"], Rows)` → stdout；不开库；`RTSQL_HOME`/`HOME` 双缺失 → `General` exit 1（design D5）
- **Required changes**: `Command::List` 分支
- **Preserve**: `render()` 与 TTY/非 TTY 默认格式语义（复用 `kind()`）；不触碰锁/格式头
- **Forbidden**: 不开库校验内容；不引入新的输出通道（走 `emit_stdout`）
- **Test witness**: RED——`tests/cli_test.rs` 新增 `test_list_enumerates_db_files`（a.db/b.db + notes.txt → json `{"columns":["name","size_bytes"],"rows":[["a.db",N],["b.db",M]]}`，notes.txt 不出现）、`test_list_empty_or_missing_dir`（空行集 exit 0）
- **GREEN condition**: 2 用例绿；既有用例零回归
- **Verification**: `cargo test --test cli_test`（exit 0）
- **Stop when**: 渲染契约与 `render()` 行集形状冲突（→ Blocker Handoff）

### T5: DDL 生成器 + schema 子命令

- **Requirement/Scenario**: R-schema 全部 4 场景
- **Depends on**: T1
- **Targets**: `src/cli/`（DDL 生成纯函数 + `Command::Schema` 臂）
- **当前行为**: 无 schema 命令；catalog 读 API（`catalog().scan_tables()/scan_columns()`，catalog.rs:205/214）已存在但无 CLI 消费者
- **目标行为**: DDL 纯函数（design D6：`INT/FLOAT/BOOL/STRING` 映射、恒双引号标识符、PK/NOT NULL/UNIQUE 约束序、按 column_index 升序）+ schema 流程（存在性检查 D3 → `execute_command_inner` → work：scan_tables 为空 → 无输出 Success；否则逐表 scan_columns 排序 + DDL 行 `emit_stdout`）→ 静默 exit 0
- **Required changes**: DDL 生成纯函数 + `Command::Schema` 分支
- **Preserve**: 系统表不可 SQL 查询的现状（走内部 catalog API，不扩 SQL 面）；锁冲突经 `open_error_status` → exit 4
- **Forbidden**: 不修改 catalog 序列化；不输出 DEFAULT；不做非持久化约束推断
- **Test witness**: RED——lib 单测 `ddl_generator_renders_types_and_constraints`（CatalogRow/CatalogColumnRow 构造 → 断言 DDL 文本含 `CREATE TABLE "users"`、`"id" INT PRIMARY KEY`、`"name" STRING NOT NULL`、String 长度信息不出现）；集成 `test_schema_outputs_ddl`（建表含 NOT NULL → `rtsql schema app` exit 0 + stdout 含表名与列）、`test_schema_missing_db_errors`（exit 1 + `does not exist`）、`test_schema_empty_db_no_output`（空库 exit 0 无输出）
- **GREEN condition**: 1 lib 单测 + 3 集成用例绿；既有用例零回归
- **Verification**: `cargo test --lib && cargo test --test cli_test`（exit 0）
- **Stop when**: catalog 读 API 无法满足列序/约束还原（→ Blocker Handoff）

### T6: dump 子命令

- **Requirement/Scenario**: R-dump-restore / S dump-restore 往返等价（dump 半）、S dump 空库无输出、S 库不存在、S 锁冲突
- **Depends on**: T5（DDL 生成器）
- **Targets**: `src/cli/`（SQL 字面量纯函数 + `Command::Dump` 臂）
- **当前行为**: 无 dump 命令
- **目标行为**: 存在性检查（D3）→ `execute_command_inner` → work：scan_tables 为空 → 无输出 Success；否则逐表（D6 DDL 行 + `SELECT * FROM "t"` 经 parse/plan/execute 三 stage 取 `QueryResult.rows` → 每行 `INSERT INTO "t" VALUES (...)`，字面量纯函数 design D7：Number→十进制、Bool→TRUE/FALSE、String→单引号加倍、Null→NULL）→ 全部经 `emit_stdout` 写出 → exit 0
- **Required changes**: 字面量转义纯函数 + `Command::Dump` 分支
- **Preserve**: 不修改 pipeline stage 与 `Response`；catalog 表序即输出序
- **Forbidden**: 不做流式/分块导出优化；不输出 COPY 等非 SQL 面语句（仅 6 语句面内的 CREATE/INSERT）
- **Test witness**: RED——lib 单测 `sql_literal_escaping`（单引号加倍 / TRUE/FALSE / NULL / 整数浮点）；集成 `test_dump_restore_roundtrip`（建表+多类型数据 → dump 重定向文件 → `rtsql new b` → `rtsql restore b file` → SELECT 比对，本任务验证 dump 侧产物含 DDL+INSERT；restore 执行依赖 T7，本用例与 T7 合并交付——先以断言 stdout 内容形态 witnessing）、`test_dump_empty_db_no_output`
- **GREEN condition**: lib 单测 + dump 侧用例绿；`test_dump_restore_roundtrip` 在 T7 完成后转绿（两任务共享用例）
- **Verification**: `cargo test --lib && cargo test --test cli_test`（exit 0）
- **Stop when**: `SELECT *` 行形状与 schema 列序不一致（实质冲突 → Blocker Handoff）

### T7: restore 子命令

- **Requirement/Scenario**: R-dump-restore / S 往返等价（restore 半）、S stdin 管道、S 非空库拒绝、S fail-fast、S 库不存在、S 锁冲突
- **Depends on**: T1、T6（共享往返用例）
- **Targets**: `src/cli/`（`Command::Restore` 臂）
- **当前行为**: 无 restore 命令
- **目标行为**: 存在性检查（D3）→ `execute_command_inner` open → work 内 `catalog().scan_tables()` 非空 → `General` 拒绝（目标非空库）→ 读文件（`-` → stdin `read_to_string`）→ `parse_stage` 全串 → 逐条 `plan_stage(db, stmt.to_string(), stmt, false)` + `execute_stage`（不渲染）→ 失败复用 `sql_failure_status`（exit 3，序号 + 前序已生效）→ 成功静默 exit 0（design D8）
- **Required changes**: `Command::Restore` 分支 + 静默逐条执行循环
- **Preserve**: `run_sql`（主命令渲染路径）不动——restore 使用独立静默循环；逐条 auto-commit 语义；close checkpoint 由编排保证
- **Forbidden**: 不做整脚本事务；不渲染每条语句结果；不修改 pipeline
- **Test witness**: RED——`tests/cli_test.rs` 新增 `test_restore_rejects_nonempty_target`（含表库 → exit 1）、`test_restore_fail_fast`（第二条重复主键 → exit 3 + stderr 含序号 + 首条已生效）、`test_restore_stdin_pipe`（dump | restore - → 等价）；`test_dump_restore_roundtrip`（T6 创建）转绿
- **GREEN condition**: 3 新增 + 1 共享用例绿；既有零回归
- **Verification**: `cargo test --test cli_test`（exit 0）
- **Stop when**: 静默循环与 fail-fast 定位模板冲突（→ Blocker Handoff）

### T8: import --csv 子命令

- **Requirement/Scenario**: R-import 全部 8 场景
- **Depends on**: T1
- **Targets**: `Cargo.toml`（+`csv = "1"`）、`src/cli/`（类型转换纯函数 + `Command::Import` 臂）
- **当前行为**: 无 import 命令；Cargo.toml 无 csv 依赖
- **目标行为**: 未提供 `--csv` → `Usage` exit 2；存在性检查（D3）→ `execute_command_inner` open → work：`db.get_table(table)`（Err → `General` 表不存在）→ csv Reader 解析（首行表头）：表全部列必须在表头、表头不得含表外列（违规 → General 含列名）→ 每数据行按 header→schema 重排 → 类型转换纯函数（design D9：Int i64 / Float f64 / Bool true|false 大小写不敏感 / String 原样；空字段 → Null 或 String("")；失败 → General 含行号列名原值）→ `INSERT INTO "t" VALUES (...)` 全列 schema 序 → `db.execute_sql` 逐条 → SQL 失败 `Sql`（`import row {k} of {n} failed`）→ 成功 `emit_stdout(render(kind, [], Affected(total)))` exit 0
- **Required changes**: csv 依赖 + 转换纯函数 + `Command::Import` 分支
- **Preserve**: 零 planner 修改（CLI 侧重排值序）；逐条 auto-commit；`--format` 对 affected 输出语义沿用
- **Forbidden**: 不做自动建表；不做流式批事务；不扩 INSERT 语法面
- **Test witness**: RED——lib 单测 `csv_value_conversion`（Int/Float/Bool 大小写/空字段 Null 与空串/非法值错误）；集成 `test_import_basic_and_header_order`（含 affected_rows 输出）、`test_import_conversion_fail_fast`（行号定位 + 前序行已生效）、`test_import_header_mismatch_rejected`、`test_import_missing_table_or_db`、`test_import_quoted_fields`（RFC4180 逗号/引号/跨行）
- **GREEN condition**: 1 lib 单测 + 5 集成用例绿；既有零回归
- **Verification**: `cargo test --lib && cargo test --test cli_test`（exit 0）
- **Stop when**: csv crate 拉取不可达（网络）或解析行为与 RFC4180 场景冲突（→ Blocker Handoff）

### T9: 回归门与全量验证

- **Requirement/Scenario**: R1（全部既有场景回归保持）+ change 级验证边界
- **Depends on**: T3, T4, T5, T6, T7, T8
- **Targets**: 全仓（只读验证）
- **当前行为**: 基线 cli_test 25/0/2（2026-09-09 @ `a5b0a5f` 实测）；SNAPSHOT 全量 671/0/2
- **目标行为**: `cargo test --all` 全绿（既有测试零修改）；clippy/fmt/openspec validate 全 0/PASS
- **Required changes**: 无代码改动（验证任务）
- **Preserve**: 既有测试断言语义零修改
- **Forbidden**: 不以放宽断言换取通过
- **Test witness**: 基线已留档（本文件头部审计依据）；全量命令输出
- **GREEN condition**: 全量门通过
- **Verification**: `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`
- **Stop when**: 既有测试出现计划外破坏（→ Blocker Handoff，不得静默改断言）

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 入口与主命令（分发扩展） | S 子命令分发/主命令零回归 | D1/D2 | T1 | 000 | `cli/mod.rs::CliArgs/run` | `test_subcommand_dispatch_list_runs` + 既有 25 零修改 | None | Covered |
| R1 入口与主命令（分发扩展） | S 裸名与子命令名冲突 | D1 | T1 | 000 | `cli/mod.rs::CliArgs` | `test_subcommand_dispatch_list_runs`（分发优先断言） | None | Covered |
| R2 名称解析 | 既有场景（不变量） | D5 | T2 | 000 | `cli/resolve.rs` | 既有 2 单测零修改 | None | Covered |
| R-new | S1 裸名新建+目录 / S2 路径父目录 / S3 已存在拒绝 / S4 立即可用 | D3/D4 | T3 | 000 | `cli/`（Command::New） | `test_new_creates_db_and_dirs` 等 3 | None | Covered |
| R-list | S1 枚举 / S2 空行集 / S3 双 env 缺失 | D2/D5 | T2, T4 | 000 | `cli/resolve.rs::db_dir` + `Command::List` | `test_list_enumerates_db_files` 等 2 | None | Covered |
| R-schema | S1 DDL 输出 / S2 空库 / S3 不存在 / S4 锁冲突 | D3/D6 | T5 | 000 | DDL 纯函数 + `Command::Schema` | lib `ddl_generator_*` + `test_schema_*` 3 | None | Covered |
| R-dump-restore | S1 往返等价 | D6/D7/D8 | T6, T7 | 001 | 字面量纯函数 + Dump/Restore 臂 | `test_dump_restore_roundtrip`（共享） | None | Covered |
| R-dump-restore | S2 空库无输出 | D7 | T6 | 001 | `Command::Dump` | `test_dump_empty_db_no_output` | None | Covered |
| R-dump-restore | S3 stdin 管道 | D8 | T7 | 001 | `Command::Restore` | `test_restore_stdin_pipe` | None | Covered |
| R-dump-restore | S4 非空库拒绝 / S5 fail-fast / S6 库不存在 / S7 锁冲突 | D3/D8 | T7 | 001 | `Command::Restore` | `test_restore_rejects_nonempty_target` 等 | None | Covered |
| R-import | S1 基本导入 / S2 乱序表头 | D9 | T8 | 001 | 转换纯函数 + `Command::Import` | `test_import_basic_and_header_order` | None | Covered |
| R-import | S3 类型转换与空字段 / S5 转换失败 / S6 表头不匹配 / S7 表库不存在 / S8 引号转义 / S9 锁冲突 | D9/D2/D3 | T8 | 001 | 同上 | `csv_value_conversion` + `test_import_*` 4 | None | Covered |

简化登记：无。8 项默认假设经 Gate 1 用户批准（2026-09-09），非未批准裁剪；spec 场景覆盖的 35 个场景中，锁冲突类由 `open_error_status` 复用路径承载（不逐场景新建测试——既有 `test_lock_conflict_exit_4` 锁定该机制，子命令共享同一入口）。
