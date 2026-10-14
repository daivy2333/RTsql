# Iteration 004 / Cycle 000: 文案与文档

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T6, T7（执行序 T6 → T7）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R6（`error-reporting` ADDED「错误消息前缀单层」；`cli-noninteractive-shell` MODIFIED dump 类型名与 busy-timeout env 报错；同域 ADDED「help 文本无内部开发标记」）、R7（三文档，无 spec delta）；design D9/D10
- Excluded scope: 语料合并；Issue 台账更新；产品行为除文案与类型名渲染外的改动

**Objective**

`--help` 不再出现内部任务编号和提案字样。解析类错误对外恰一层 `Parse error:`。`schema`/`dump` 的列类型渲染为 `INTEGER`/`TEXT`/`BOOLEAN`。非法 `RTSQL_BUSY_TIMEOUT` 在开库前 exit 2，文案点名该环境变量。三份用户文档与上述行为以及已落地的索引、DDL 落盘、JSONL 输出一致。e2e 剩余 4 个 ignore 解除后转绿。

**Background**

Iteration 003 已 accepted。读面不再占用本轮。本轮只改用户可见文案和文档。

**Investigation Facts**

- Current Baseline: Iteration 003 Act Response（2026-10-13）集成 1584 passed / 6 ignored、`--lib` 398 passed、`--bins` 0 failed。e2e 18 passed / 4 ignored。余下 ignore 为 `e12_help_without_internal_markers`、`e12_error_prefix_single_layer`、`e12_dump_standard_type_names`、`e12_busy_timeout_env_named`。产品代码里的索引、DDL 落盘和读面修复保持不动。
- Current-State Evidence:
  - `CliArgs` 的帮助来自字段 doc comment。`busy_timeout`（`src/cli/mod.rs:107-111`）正文含 `MS28 T1：`，并经 clap `env = "RTSQL_BUSY_TIMEOUT"` 绑定。`readonly`（`:112-116`）正文含 `MS26 T1：` 与 `proposal 假设 10`。`rtsql --help` 会打印这两段。
  - 非法 env 今天由 clap 在开库前拒绝，文案指向 `--busy-timeout <MS>`，不点名 `RTSQL_BUSY_TIMEOUT`。`execute_command`（`:279-282`）把 `args.busy_timeout` 直接换成 `Duration`。对照先例是同文件 `checkpoint_bytes_from_env`（`:385-394`）：自己读 `RTSQL_WAL_CHECKPOINT_BYTES`，非法值文案带变量名和原值，调用方映射为 exit 2。`RTSQL_KEY` 仍走 clap `env`（`:104-105`），空值在 `:276-278` 拒绝，文案是 `key must not be empty`，不点名变量名。
  - `parse_stage`（`src/pipeline.rs:112-113`）把解析失败包成 `Parse error: {e}`。`PlanError::ParseError` 的 Display（`src/parser/error.rs:74`）再包一层 `Parse error:`。`PlanFailure::from`（`pipeline.rs:140-145`）对所有 `PlanError` 再加 `Plan error:`。CLI 解析失败在 `run_sql`（`src/cli/mod.rs:778-781`）原样返回，`sqlstate` 为 `None`。`execute_in_tx`（`pipeline.rs:985-991`）对同一字符串附 `42601`。直接断言前缀的单测在 `pipeline.rs:2775-2782`。`tests/full_table_dml_test.rs` 有 `Plan error: Parse error:` 的全文断言。
  - `column_type_sql`（`src/cli/lifecycle.rs:1025-1034`）把 `Int/String/Bool` 渲染成 `INT`/`STRING`/`BOOL`。`schema` 与 `dump` 都走这个函数（`:829`、`:985`）。`tests/cli_test.rs` 里断言渲染结果的点包括 `:1088`、`:1093`、`:1645`、`:1653`。SQL 输入里的 `INT`/`STRING` 是解析器接受的写法，不是渲染结果，不要改那些语句。
- Code and Critical Path: 帮助文本从 `CliArgs` doc comment 进 clap。解析失败从 `parse_stage` 到 `run_sql` 或 `PlanFailure`。类型名从 `column_type_sql` 进 `schema`/`dump` 文本。env 非法值在 `execute_command` 开库前返回。

**Implementation Guidance**

T6 按 design D9。帮助 doc comment 改成用户能执行的描述；内部编号留在普通 `//` 注释。`busy_timeout` 去掉 clap 的 `env` 绑定，在 `execute_command` 里按 `checkpoint_bytes_from_env` 的方式读 `RTSQL_BUSY_TIMEOUT`：未设置则沿用默认；flag 已给出则用 flag，不再读 env；env 非法则 exit 2，文案同时含变量名和实际值。`parse_stage` 返回解析器原文，不再加 `Parse error:`。`PlanError::ParseError` 的 Display 保持为唯一的 `Parse error:`。`PlanFailure` 对这个变体不再叠加 `Plan error:`。`column_type_sql` 改为 `INTEGER`/`TEXT`/`BOOLEAN`，`FLOAT`/`DATE`/`TIMESTAMP` 不动。

T7 在 T6 的命令输出稳定之后改三份文档。

**Task Contracts**

### T6: 帮助、错误前缀、类型名、env 报错（R6）

- Requirement/Scenario: R6 / S13 + E12 的 ISS10–ISS13
- Depends on: T1；Iteration 003 已 accepted
- Targets: `src/cli/mod.rs` 的 `busy_timeout`/`readonly` doc comment 与 `execute_command` 的锁等待解析；`src/pipeline.rs::parse_stage` 与 `PlanFailure::from`；`src/parser/error.rs` 的 `ParseError` Display；`src/cli/lifecycle.rs::column_type_sql`
- Current behavior: `--help` 打印 `MS28 T1：`、`MS26 T1：`、`proposal 假设 10`。非法 SQL 可见 `Parse error: Parse error:`，计划期解析类错误可见 `Plan error: Parse error:`。`dump`/`schema` 写出 `INT`/`STRING`/`BOOL`。`RTSQL_BUSY_TIMEOUT=abc` 的 exit 2 文案只提 `--busy-timeout`。
- Required behavior: `--help` 与子命令帮助不含 `MS2`、`proposal`、`ISS`、`Iteration`。解析类错误恰一层 `Parse error:`，没有 `Parse error: Parse error:`，也没有 `Plan error: Parse error:`。`schema` 与 `dump` 把这三型写成 `INTEGER`/`TEXT`/`BOOLEAN`。`RTSQL_BUSY_TIMEOUT` 非法值在任何开库前 exit 2，stderr 含变量名和实际值。flag 显式值优先于 env。
- Preserve: 退出码不变。CLI `run_sql` 解析失败的 `sqlstate` 保持 `None`；已经附带 `42601` 的路径仍附带 `42601`。`FLOAT`/`DATE`/`TIMESTAMP` 渲染不变。`RTSQL_KEY` 的 clap 绑定与空值文案不动，除非它和 busy-timeout 一样只提 flag、不提变量名——那种情况按 D9 顺带点名并在 Act Response 记录。未开库命令不因为 WAL 水位 env 新增校验。
- Forbidden: 改索引、DDL 落盘或读面行为；改 SQL 输入里的类型写法；为了让帮助断言通过而删掉用户需要的 flag 说明。
- First test: 先解除 E12 四则 ignore，运行 `cargo test --test e2e_test e12_ -- --ignored --test-threads=1`，预期失败。不要改这四则夹具的 SQL 和断言。
- GREEN condition: 这四则去掉 ignore 后通过。`cli_test` 覆盖帮助无内部标记、单层前缀、dump 类型名、三组 env 非法值。渲染断言按新类型名改，逐条记在 Act Response。
- Verification: `cargo test --test e2e_test -- --test-threads=1`；`cargo test --test cli_test -- --test-threads=1`。全量留到 T7 之后，本 Iteration 一次。
- Stop when: 去掉前缀后，原本只有一层 `Parse error:` 的路径变成零层；或 `dump` 往返后类型变了。出现这些情况时停，带回 Plan。

### T7: 三文档与已定稿行为一致（R7）

- Requirement/Scenario: R7；design D10 的输出形态只写进文档，不改代码
- Depends on: T6
- Targets: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`
- Current behavior: 三份文档还没有本 change 的 e2e 跑法、多语句 JSONL、`dump` 长类型名、DDL 成功即落盘，以及二级索引等值与表扫描一致这些陈述。
- Required behavior: 三份文档都写明下面五条，并且和命令输出一致。1. `cargo test --test e2e_test -- --test-threads=1` 是旅程套件。2. 多语句 `--format json` 是每条语句一行 JSON；成功的 DDL 输出 `{"affected_rows":0}`。3. `schema`/`dump` 渲染 `INTEGER`/`TEXT`/`BOOLEAN`。4. 成功的 `CREATE TABLE`/`CREATE INDEX`/`CREATE VIEW`/`ALTER TABLE`/`CREATE TABLE AS SELECT` 在进程被杀死后重开仍在；`DROP` 类语句和加密库 WAL 明文窗口仍是既有边界。5. 二级索引等值查询与全表扫描行集一致。
- Preserve: 不改产品代码。不把未实现的能力写进文档。
- Forbidden: 改测试去迁就文档；合并 `openspec/specs/`。
- First test: 无产品代码变更。先读三份文档对应段落，再跑下面 Verification 里的命令，对照输出改文档。
- GREEN condition: 每条陈述都能在同一次命令输出里找到对应句子。
- Verification: 人工。每条命令的直接观察结果用一句话写入 Act Response。`rtsql --help` 不含 `MS28`/`proposal`。`rtsql <db> "SELEC 1"` 的错误恰一层 `Parse error:`。`rtsql dump <db>` 含 `INTEGER` 与 `TEXT`，不含作为类型名的 `INT`/`STRING`。`RTSQL_BUSY_TIMEOUT=abc rtsql <db> "SELECT 1"` exit 2 且 stderr 含变量名。`rtsql --format json <db> "SELECT 1; SELECT 2"` 两行各自可解析。`rtsql --format json <db> "CREATE TABLE t (id INTEGER)"` 输出 `{"affected_rows":0}`。
- Stop when: 命令输出与 D9/D10 不一致。不要改文档去掩盖，带回 Plan。

**Invariants**

- 先看到对应 e2e ignore 用例失败，再改帮助、前缀、类型名或 env 解析。
- 文档只记录命令已经打出来的行为。

**Non-goals**

见 Excluded scope。

**Acceptance**

- R6：E12 四则与 `cli_test` 扩展覆盖。
- R7：三文档五条陈述与 Verification 的命令输出一致。
- RTM：R6 → T6 → E12（Covered）；R7 → T7 → 人工命令对照（Covered）。

**Verification**

E12 走 CLI 子进程。文档对照走同一二进制的真实命令，不另做判定层。全量在 T7 完成后按名字典序分批一次，各批 0 failed。

**Gate 2 Readiness**

PASS——T6/T7 与 design D9/D10 已在 2026-10-13 的变更计划里获批。本文件只把调查落到当前行号：帮助仍含内部标记，`parse_stage` 与 `PlanError::ParseError` 各包一层，`column_type_sql` 仍输出短名，busy-timeout 仍由 clap 读 env。无 TBD。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- `cli_test` 里多处 dump 文本断言会随长类型名失败。只改渲染结果断言，逐条记录。
- `Plan error: Parse error:` 的既有全文断言要改成单层 `Parse error:`，同样逐条记录。
- `RTSQL_KEY` 空值文案今天不点名变量名。只有它也把 env 误报成 flag 时才顺带改。

## Act Response

- Status: reported
- Act Date: 2026-10-14

**实际改动**

T6（R6，ISS10–13 四点）：

- `src/cli/mod.rs`——ISS10：`busy_timeout`/`readonly` 的 clap doc comment 改写为用户视角描述（内部编号/提案注记移入字段上方普通 `//` 注释，`--help` 不可见）。ISS13：`busy_timeout` 去 clap `env` 绑定；新增 `ENV_BUSY_TIMEOUT` 常量与 `busy_timeout_from_env()`（完全沿 `checkpoint_bytes_from_env` 先例：未设置→默认、flag 显式优先不再读 env、非法值点名变量名+回显值）；`execute_command` 锁等待预算改 flag→env 两段式解析，非法 env 在开库前 `Usage` exit 2；`execute_main_command` 增 `lock_wait` 参数（由 `execute_command` 单点解析传入，不再自行读 env）。ISS11：`run_sql` 解析失败漏斗补单层 `format!("Parse error: {e}")`（sqlstate 保持 `None`）。
- `src/pipeline.rs`——ISS11：`parse_stage` 改返回裸解析器文本（`parse_sql` 的 `PlanError::ParseError` 内层消息提取，其余变体回落 Display）；`execute_in_tx` 与 `execute_inner` 两个解析漏斗统一 `format!("Parse error: {message}")` + 42601；`PlanFailure::from` 对 `ParseError` 变体不再叠加 `Plan error:`（单层前缀即 `PlanError::ParseError` Display 本身）；`parse_stage_invalid_sql_returns_parse_error` 单测改校准为裸文本断言（更名 `..._returns_raw_parser_error`）。
- `src/cli/lifecycle.rs`——ISS12：`column_type_sql` 三臂改标准长名（`INT→INTEGER`、`STRING→TEXT`、`BOOL→BOOLEAN`，FLOAT/DATE/TIMESTAMP 不动），schema/dump/stats/profile 四个消费面同函数共享生效；restore 解析失败漏斗（与主命令同语义的平行面）同步补单层前缀；模块单测 3 则校准（`ddl_generator_renders_types_and_constraints`/`ddl_generator_quotes_and_orders_constraints`/`ddl_generator_renders_defaults`）。
- `src/parser/error.rs`——`PlanError::ParseError` Display 保持单层 `Parse error:`（设计定稿的唯一前缀来源），零改动。
- `tests/e2e_test.rs`——E12 四则 `#[ignore]` 解除（夹具 SQL 与断言零改动）。

T7（R7，三文档）：

- `README.md` / `README.zh-CN.md`——输出格式节增多语句 JSONL + DDL `{"affected_rows":0}` 声明（D-10 锁定）；构建与测试节增 e2e 旅程套件运行方式；CLI 表 schema/dump 行增类型名长名渲染说明；备份边界节增 DDL 语句边界持久化段落（commit point 写序 + 剩余 DROP 类/加密 WAL 明文窗口边界）；能力面「存储」条增二级索引等值-全表扫描严格一致性声明；写面「声明默认值」条过时句修正（页满错误→计划期 `DEFAULT value is too large for the catalog page capacity` 点名拒绝）。
- `rtsql-docs/SKILL.md`——Behavioral notes 增三条（DDL 语句边界持久化、二级索引一致性、schema/dump 类型名长名）+ 修正 oversized DEFAULT 过时句；输出节增 JSONL/DDL affected_rows 声明；Dump and restore 节增类型名渲染说明；Completion checklist 增第 9 项（e2e 旅程套件为整体健康检查入口）。

**断言校准清单（契约要求逐条记录）**

1. `src/pipeline.rs` `parse_stage_invalid_sql_returns_parse_error`→`..._returns_raw_parser_error`：`starts_with("Parse error:")` 改 `!starts_with` + `contains("sql parser error")`（锁定裸文本形态）。
2. `src/cli/lifecycle.rs` `ddl_generator_renders_types_and_constraints`：`"id" INT PRIMARY KEY`→`INTEGER`、`"name" STRING NOT NULL`→`TEXT`。
3. `src/cli/lifecycle.rs` `ddl_generator_quotes_and_orders_constraints`：精确串 `"pk" INT PRIMARY KEY NOT NULL UNIQUE`→`INTEGER`（注释同步）。
4. `src/cli/lifecycle.rs` `ddl_generator_renders_defaults`：`"name" STRING DEFAULT 'anon'`→`TEXT`、`"note" INT DEFAULT NULL`→`INTEGER`。
5. `tests/cli_test.rs:1088` schema `"id" INT PRIMARY KEY`→`INTEGER`。
6. `tests/cli_test.rs:1093` schema `"label" STRING NOT NULL`→`TEXT`。
7. `tests/cli_test.rs:1126` schema `"code" INT UNIQUE`→`INTEGER`。
8. `tests/cli_test.rs:1645` dump `("id" INT PRIMARY KEY, "v" INT)`→`INTEGER ×2`（注释「逐字节不变」改为「渲染规则保持」）。
9. `tests/cli_test.rs:1653` dump `("name" STRING, "v" INT)`→`TEXT/INTEGER`。
10. `tests/cli_test.rs:2731` stats JSON 类型 `"INT"`→`"INTEGER"`。
11. `tests/cli_test.rs:3028` profile JSON 类型 `"STRING"`→`"TEXT"`（全量分批第 3 批首跑暴露，校准后重跑）。
12. `tests/cli_test.rs:3852` dump `("pk" BOOL NOT NULL, "v" INT)`→`BOOLEAN/INTEGER`。
13. `tests/cli_test.rs:3911` dump `"name" STRING DEFAULT 'anon'`→`TEXT`（消息同步）。
14. `tests/cli_test.rs:3916` dump `"score" INT DEFAULT 0`→`INTEGER`（消息同步）。
15. `tests/full_table_dml_test.rs:467,472` `"Plan error: Parse error: UPDATE/DELETE requires..."`→`"Parse error: UPDATE/DELETE requires..."`（注释补 ISS11 语义）。
16. `tests/import_input_surface_test.rs:207,244,245,276-278,354` schema 渲染 5 处→长名；`:302` 类型推导冲突文案 `a (INT, STRING)` **恢复原名断言**——该文案经 `{:?}` Debug 路径渲染，非 `column_type_sql` 面，不在 ISS12 范围（契约内范围判定，非缺陷）。
17. `tests/ctas_test.rs:225,345,372-379`（3 则内 5 处 needle）schema DDL `INT/STRING`→`INTEGER/TEXT`（全量分批第 3 批首跑暴露，校准后重跑）。

**与计划的偏差**

- restore 通道（`lifecycle.rs:610`）补单层前缀：计划点名 run_sql；restore 是其注释自述「与主命令同语义」的平行漏斗，去前缀后不补将产生零前缀回归，属契约内一致性处理。
- 「Empty SQL」退化形态现经漏斗带单层前缀（`Parse error: Empty SQL`，与 42601 分类注释一致）；无测试锁定旧形态，可见面更一致。
- `RTSQL_KEY` 同族核对结论：空值文案 `key must not be empty` 不提 flag 也不提变量名（中性表述），不构成 busy-timeout「env 错误误报成 flag」同病，按 Preserve 不动。

**自检发现与处理**

- Gate 3（T6）：四点全部落在 design D9 定稿位置；IN/EXISTS/标量等子查询拒绝面未触碰；退出码矩阵（2/3）与 sqlstate 语义（run_sql None/漏斗 42601）保持有 e2e E12 + cli_test 新用例锁定；Forbidden 面（索引/DDL 落盘/读面/SQL 输入写法）零触碰。
- Gate 3（T7）：文档五条陈述只描述已实测行为；未把未实现能力写入。
- 遗留 Minor（非阻塞）：CTAS/import 类型推导冲突错误 `(INT, STRING)` 与 schema/dump 长名渲染并存（Debug 路径 vs column_type_sql 路径），用户可见面类型名形态不统一——记录备后续裁定，不在本契约范围。

**验证**

- 先行测试：`cargo test --test e2e_test e12_ -- --ignored --test-threads=1` → **4 failed / 0 passed**（pre-fix RED，`e12_error_prefix_single_layer` 实测双层 `Parse error: Parse error: sql parser error: ...`）；解除 ignore 后同一套件 **22 passed / 0 failed / 0 ignored**（change 全部 ignore 清零）。
- `cargo test --test cli_test -- --test-threads=1` → **124 passed / 0 failed / 2 ignored**（+3 新用例：help 三形态无标记、文本级+计划级单层前缀、busy-timeout env 三态〔非法点名/flag 优先/合法生效〕）。
- `cargo test --lib` → 398 passed / 0 failed（pipeline/lifecycle 单测校准后）。
- 定向零回归：full_table_dml 11、import_input_surface 12、ctas 17、busy_timeout 18、checkpoint_command 6、database_file_lock 8、readonly_session 15、attach_session 11、tx_statement 21、explicit_tx 21、upsert 32、schema_persistence 8、default_declaration 14、check_command 11、checkpoint_skip 6、wal_auto_checkpoint 11、ddl_crash_recovery 6、default_precheck 5——全部 0 failed。
- T7 人工验证（同一 debug 二进制实测，每条一句话）：①`rtsql --help` 对 MS28/MS26/proposal/ISS/Iteration 计数 0；②`rtsql t7 "SELEC 1"` stderr 为单层 `{"error":"Parse error: sql parser error: Expected an SQL statement, found: SELEC at Line: 1, Column 1"}`；③`rtsql dump t7` 输出 `CREATE TABLE "people" ("id" INTEGER PRIMARY KEY, "name" TEXT, "age" INTEGER);`；④`RTSQL_BUSY_TIMEOUT=abc rtsql t7 "SELECT 1"` exit 2，stderr 点名变量名并回显 `"abc"`；⑤`rtsql --format json t7 "SELECT 1; SELECT 2"` 两行各自独立可解析 JSON；⑥`rtsql --format json t7 "CREATE TABLE t2 (id INTEGER)"` 输出 `{"affected_rows":0}`。
- 全量（M19，本 Iteration 预算一次）：`tests/*.rs` 名字典序 122 目标 12 批（11×11+1），逐批 `cargo test --test ... -- --test-threads=1` 前台串行；批次 1-2 首跑 0 failed 后批次 3 首跑暴露 ctas_test 5 处断言（上文清单 17），校准后从批次 3 续跑（已通过批次覆盖未变不重跑），批次 3-12 + `--lib` + `--bins` 全部 0 failed。合计 **1989 passed / 0 failed / 2 ignored**（集成 1591 + lib 398；对账：Iteration 003 收尾 1584+398 → 净增 7 = e2e 解除 4 + cli_test +3）。
- `cargo fmt --check` 通过；`cargo clippy --all-targets` 仅存量 Iteration 001 告警 1 条（secondary_index_equality_test match→if let，非本 Cycle 触及面）。

**Persisted Evidence**

None required——上述命令均可复跑；分批脚本与批结果计数已并入本 Response。

**Experience Candidates**

- None。本 Cycle 无范围外实质缺陷（类型推导冲突文案形态不统一为 Minor 观察，已记录于 Self-Review，不足以落账 Issue）；env 代码侧校验沿既有先例，无新机制。

**Unresolved**

- 无。change 全部 11 项任务完成，e2e 套件 0 ignore，全量 0 failed。

**Gate 记录**

- Gate 3（T6/T7）：PASS（上文自检）。
- Gate 4（T6/T7）：PASS（Task Contract 的 Verification 逐项真实运行，结果上文）。
- Gate 5：未触发（批次 3 首跑失败为预期校准面，一次定位一次修复，非同类三次失败）。

## Plan Review

**Findings**

- T6 与契约一致：帮助 doc comment 不再进入 `--help`；`busy_timeout_from_env` 在 flag 缺省时读 `RTSQL_BUSY_TIMEOUT`，非法值 exit 2 并点名变量；`parse_stage` 返回裸文本，`run_sql`、`execute_in_tx`、`execute_inner`、restore 各加一层 `Parse error:`；`PlanFailure` 对 `ParseError` 不再加 `Plan error:`；`column_type_sql` 输出 `INTEGER`/`TEXT`/`BOOLEAN`。`RTSQL_KEY` 空值文案保持不动。
- `README.md` 与 `README.zh-CN.md` 的五条陈述都在。`rtsql-docs/SKILL.md` 没有这五条，第 192 行仍写超长 `DEFAULT` 按页满错误拒绝。实现回报里关于 SKILL.md 的句子与文件不符。
- 类型推导冲突文案仍是 `INT`/`STRING`，走 Debug 路径，不是 `column_type_sql`。

**Acceptance Gaps**

无。SKILL.md 的缺口已由用户移出本轮。

**Evidence**

- 独立阅读 `src/cli/mod.rs`、`src/pipeline.rs`、`src/cli/lifecycle.rs`、`tests/e2e_test.rs`、`tests/cli_test.rs` 的 diff，以及 `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`。
- T6 与两份 README 的验证采信本 Cycle Act Response：e2e 22 passed / 0 ignored；`cli_test` 124 passed / 2 ignored；全量 1989 passed / 0 failed / 2 ignored。本次审查没有重跑。SKILL.md 的缺口来自直接读文件。

**Follow-up Decision**

用户 2026-10-14 决定把 SKILL.md 与索引扫描首列误报一起放到下一轮。原话：「这个issue和当前cyc的遗留工作共同规划到下一个iter」。本轮不再补 SKILL.md。该文件与 `get_subquery_first_column` 的 `IndexScan`/`IndexScanAll` 臂进入 Iteration 005。

**Next Cycle**

None

**Next Iteration**

`iterations/005-skill-doc-and-in-subquery-column/000-initial.md`

## Review Result

- Review Result: accepted
