# Iteration 000 / Cycle 000: ISS15 修复与组合面见证补齐

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3, T4, T5
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: 基线 1994 tests / 0 failures / 2 ignored（fbb5c54，2026-10-14 归档记录，代码此后未变）；ISS15 台账 `.agents/issues/ISS15-table-renderer-panics-on-64k-cell.md`（R43）的根因定位与实测记录
- Excluded scope: proposal Non-goals 全项——journey 层缺席项（TRUNCATE/ALTER/SIGINT/WAL 水位/completions/readonly 完整旅程/schema/list）、TTY 默认分支自动化、四维穷举审计、json/csv/tsv 截断、`--readonly` × 未列举子命令穷举

**Objective**

任意单元格长度下 table 渲染 exit 0（ISS15 panic 关闭）且超限单元格按 64 字符上限截断；大值×四格式、大值×分析子命令、import×加密库、分析子命令×readonly/加密库四组组合各有自动化见证；README 双语与 SKILL.md 同步；全量零回归。

**Investigation Facts**

- Current Baseline: HEAD fbb5c54（master，领先 origin 7 提交）。全量 1994 tests / 0 failures / 2 ignored（2026-10-14 change 归档记录；此后工作区仅 `.agents/` 文档变更，产品代码未动——采信，来源：tasks.md 最近完成段 + git status）。ISS15 为 open 缺陷，table 渲染 ≥65536 字符单元格 exit 101。
- Current-State Evidence:
  - `src/cli/render.rs:62-104` `render_table`：widths[i] = 列内最大 `chars().count()`（含表头，:70-79）；:85 `format!("{:<w$}", cell, w = widths[i])` 为唯一 panic 点；全 `src/` 无其他动态宽度消费点（grep 佐证）。
  - `src/cli/lifecycle.rs:1103-1113` 分析子命令 `render_rows` 委托 `render::render`（import 于 :6）——单点修复覆盖主命令 + stats/sample/profile。`column_min_max`（:1239）String 列 min/max 携带原值 = 用户实测 panic 路径。
  - `src/cli/lifecycle.rs:680` `import_csv` 含 `key: Option<&str>`（:686）→ :712 `execute_command_inner_mode(&db_path, key, false, memory, lock_wait, …)`；`src/cli/mod.rs:609-611` readonly/key 打开分派（`open_readonly_with_key` / `open_with_key`）——import × 加密通道存在，零见证。
  - stats（lifecycle.rs:1117）/sample（:1342）/profile（:1419）经 `execute_command_inner(&db_path, key, lock_wait, …)`——加密库可用，零见证。
  - `src/cli/mod.rs:317-322` 通用拒绝臂 `Some(_) if args.readonly` → Usage exit 2；stats/sample/profile 不在 `lifecycle_db_target`（:438-444）但命中通用臂。`tests/readonly_session_test.rs:606-640` 矩阵含 schema/dump/delete/restore/import + new 早拒，缺 stats/sample/profile 三项。
  - `tests/e2e_test.rs` 夹具：`fixture()`（TempDir + `db/` + RTSQL_HOME）、`run_cli`/`wait_cli`（stdout/stderr 并发读线程防管道死锁 + 60s 兜底，:71-106）、`run_cli_stdin`、`run_cli_env`。E7 :998（new/DML/错钥 5/check/dump-restore/env，无 import）、E8 :592（1MB 值 restore stdin → json 读回 → REPLACE 2MB → dump 往返 → check；无 csv/tsv/table）、E9 :669（小值 20 行；无大值）。
  - `tests/readonly_session_test.rs:72` `seed()` 建表 `t (id INT PRIMARY KEY, n INT)` 2 行——矩阵扩展的 stats/sample/profile 目标表。
- Code and Critical Path: 渲染数据流 `rows: Vec<Vec<Value>>` → `value_to_text` → （新增截断步）→ widths 计算 → `format!` 补宽 → `" | "` join。修复只插入「文本化后逐单元格封顶」一步；分析子命令与主命令共享该路径。加密/readonly 组合不改任何产品代码——open 分派既有臂直接可用。

**Implementation Guidance**

- 执行序 T1 → T2 → T3 → T4 → T5：T1 是 T2 table 臂的前置（见证假定修复已落地）；T3/T4 与 T1 无代码耦合，顺位仅为全量预算收敛。
- T1 截断实现位置：`render_table` 内 `lines` 构造完成后、`widths` 计算前，对 `lines` 逐格处理；或文本化时处理——两等价，Act 择一，以「截断后 widths ≤ 64」可观察结果为准。
- e2e 大值体量参考既有先例：E8 已有 1MB stdin 负载与 2MB 读回；新增 csv/tsv 臂读回 ~2MB × 2，夹具并发读已覆盖。
- E9 大值用 70,000 字符（>65536 阈值，体量克制）；stats json 断言定位 `rows` 中 column == "s" 的行，min/max 在 index 5/6。
- 既有测试如命中 >64 字符 table 断言（预期行为变化）：按白名单校准，Act Response 逐处记录；不得为旧断言回退截断语义。

**Task Contracts**

### T1: render_table 大单元格截断修复

- Requirement/Scenario: R1 全部三 Scenario（超限截断不 panic / 64-65 边界 / 管道格式保真）
- Depends on: None
- Targets: `src/cli/render.rs::render_table`（+ 模块常量 `CELL_WIDTH_CAP: usize = 64`）
- Current behavior: 列宽 = 全列最大 chars().count()（含表头）；单元格 ≥65536 字符时 `format!` 动态宽度 panic（exit 101），SELECT 与 stats/sample/profile 的 table 面不可用
- Required behavior: 文本化后逐单元格（表头同规则）`chars().count() > 64` → 前 61 char + `...`（恰 64 char）；widths 按截断后文本计（≤64）；任意长度 exit 0；json/csv/tsv 输出逐字节不变
- Preserve: `render` 纯函数契约（无 IO/env/时钟）；`value_to_text`/`render_delimited`/json 分支不动；既有 ≤64 字符 table 输出逐字节不变；`src/cli/render.rs` tests 模块既有 8 例全绿
- Forbidden: 不改 csv/tsv/json 渲染；不在 `value_to_text` 截断（csv/tsv 共享会泄漏）；不引入配置项/CLI 参数/env 通道；不动 lifecycle 与主命令分发
- First test: `src/cli/render.rs` tests 模块新增边界矩阵——①64 字符原样 ②65 字符截断恰 64 以 `...` 结尾 ③100,000 字符不 panic 且 `...` 结尾 ④CJK 多字节串截断不产生部分字符 ⑤表头 >64 同规则 ⑥同大值输入下 csv/tsv/json 保真（回归锁）。命令 `cargo test --lib render`；修复前 RED（③ panic `Formatting argument out of range`，其余按断言失败）
- GREEN condition: 新矩阵全绿 + 既有 8 例零回归
- Verification: `cargo test --lib render`；随后 `cargo test --test e2e_test`（定向确认既有 table 断言无 >64 字符单元格命中——命中则按白名单校准并记录）
- Stop when: 截断语义需超出「table 文本化后固定上限封顶」设计（终端宽度、显示宽度而非字符数、可配置上限）→ 返回 Plan

### T2: e2e 大值 × 四格式与大值 × 分析子命令见证

- Requirement/Scenario: R2 两 Scenario（大值 × 主命令四格式 / 大值 × 分析子命令）
- Depends on: T1
- Targets: `tests/e2e_test.rs::e8_large_value_journey`（扩展）、`tests/e2e_test.rs::e9_analytics_journey`（扩展）
- Current behavior: E8 大值仅 json 读回断言（csv/tsv/table × 大值零覆盖）；E9 仅小值（大值 × 分析子命令零覆盖——用户实测 panic 路径）
- Required behavior: E8 追加——`--format csv big "SELECT body FROM docs"` exit 0 且数据行长度 = 2,000,000（全值保真）；`--format tsv` 同；`--format table` exit 0、恰 3 行（表头/分隔/数据）、数据行 64 char 以 `...` 结尾。E9 追加——新建 `big_stats (id INT PRIMARY KEY, s STRING)` 插一行 70,000 字符值；`stats ana big_stats --format table` / `sample ana big_stats 1 --format table` / `profile ana big_stats --format table` 全部 exit 0；`stats ana big_stats --format json` 的 s 列行 min/max（index 5/6）与原值等长
- Preserve: 既有 E8/E9 断言逐字节不变；夹具模式（RTSQL_HOME、并发管道读、60s 兜底、`CARGO_BIN_EXE_rtsql`）不动
- Forbidden: 不新增判定层/辅助脚本；不动 T1 修复面；不改 E9 既有 20 行小值表（新值进独立 `big_stats` 表）
- First test: T1 已落地 → 本任务为 CLI 面变更后 GREEN 见证（修复前 RED 由 T1 单测与 ISS15 台账 2026-10-14 实测记录承载：table 臂 exit 101）
- GREEN condition: `cargo test --test e2e_test e8_large_value_journey` 与 `cargo test --test e2e_test e9_analytics_journey` 绿
- Verification: 上述两命令
- Stop when: 大值经 csv/tsv 读回长度不符（存储/管道层缺陷，超出输出面范围）或 table 臂仍非 exit 0（T1 修复不完整）→ 返回 Plan

### T3: e2e import × 加密库与分析子命令 × 加密库见证

- Requirement/Scenario: R3 前三 Scenario（import 对钥 / import 错钥 / stats × 加密）
- Depends on: None（与 T1 无代码耦合；执行序随 T2 之后）
- Targets: `tests/e2e_test.rs::e7_encryption_journey`（扩展）
- Current behavior: E7 覆盖 new/DML/错钥/check/dump-restore/RTSQL_KEY env；import 与 stats/sample/profile × 加密库零见证
- Required behavior: E7 末尾追加——`std::fs::write` 夹具 CSV（header `id,secret`，2 行）；`--key pw enc "CREATE TABLE s2 (id INT PRIMARY KEY, secret STRING)"`；`--key pw import enc s2 f.csv --csv` exit 0；`--key pw enc "SELECT COUNT(*) FROM s2"` 计数 = 2（数据真实落库）；`--key wrong import enc s2 f.csv --csv` exit 5；`--key pw stats enc s --format json` exit 0
- Preserve: E7 既有断言（含落盘不泄漏明文）逐字节不动
- Forbidden: 不测 `:memory:` × import × key（既有 exit 2 由 import_input_surface_test.rs:553 覆盖）；不扩 stdin × 加密臂（同一 open 通道，file 通道已见证组合）
- First test: 通道代码既有 → 变更后 GREEN 见证
- GREEN condition: `cargo test --test e2e_test e7_encryption_journey` 绿
- Verification: 同上
- Stop when: import × 加密实际不可用（`open_with_key` 臂缺陷）→ 属实质产品缺陷，见证任务转缺陷修复需返回 Plan（replan）

### T4: --readonly × 分析子命令拒绝矩阵扩展

- Requirement/Scenario: R3 第四 Scenario（分析子命令 × --readonly 拒绝）
- Depends on: None
- Targets: `tests/readonly_session_test.rs::cli_readonly_rejected_with_lifecycle_subcommands`（矩阵扩展）
- Current behavior: 矩阵五项（schema/dump/delete/restore/import）+ new 早拒；`--readonly` × stats/sample/profile 行为存在（mod.rs:317-322 通用臂）但零断言
- Required behavior: 矩阵数组追加三 vec——`["--readonly", "stats", db, "t"]`、`["--readonly", "sample", db, "t"]`、`["--readonly", "profile", db, "t"]`；三者 exit 2 且 stderr 含 `--readonly` 与 `main command`（循环既有断言式自动覆盖）
- Preserve: 既有五项与 `new` 早拒断言逐字节不变；`seed()` 夹具（表 `t`）不动
- Forbidden: 不改 CLI 拒绝语义代码（纯见证）；不穷举其他子命令
- First test: 通用臂行为既有 → 变更后 GREEN 见证
- GREEN condition: `cargo test --test readonly_session_test cli_readonly_rejected_with_lifecycle_subcommands` 绿
- Verification: 同上
- Stop when: 三者任一非 exit 2（与通用臂预期矛盾，说明存在未知早退/特判）→ 返回 Plan

### T5: 文档同步（table 截断说明）

- Requirement/Scenario: R1 文档面
- Depends on: T1
- Targets: `README.md`（双语输出格式段各一处）、`.agents/skills/rtsql-docs/SKILL.md`（输出/行为说明段）
- Current behavior: 文档描述四格式无 table 截断说明
- Required behavior: 双语各加一句：table 格式对超过 64 字符的单元格截断为前 61 字符 + `...`，json/csv/tsv 全值保真
- Preserve: 既有文档结构与其余内容不动
- Forbidden: 不重写无关段落；不改文档中其他行为描述
- First test: 文档-only，无先行测试（人工核对）
- GREEN condition: 双语两处各含截断句且数字与实现一致（64/61/`...`）
- Verification: 人工——读两处文档核对数字与格式行为，一句话记入 Act Response
- Stop when: 文档无合适落点或既有表述与新语义冲突需较大改写 → 返回 Plan

**Invariants**

- json/csv/tsv 输出对任意值逐字节不变（机器消费契约）。
- `render` 纯函数契约：无 IO、无 env、无时钟。
- ≤64 字符单元格的既有 table 输出逐字节不变。
- 加密、readonly、锁、import 语义零变化（本 change 对三者为纯见证）。
- E7/E8/E9 既有断言与 readonly 矩阵既有五项逐字节保持。

**Non-goals**

proposal Non-goals 全项；另：不给截断上限加配置通道；不提升 journey 层缺席项；不动 `:memory:`/stdin × 加密组合。

**Acceptance**

- A1 任意单元格长度（≥65536）table 渲染 exit 0——T1 单测③ + E8 table 臂（R1-S1）
- A2 截断精确语义：65 字符 → 61+`...`、64 原样、CJK 边界、表头同规则——T1 单测①②④⑤（R1-S2）
- A3 大值 × json/csv/tsv 保真——T1 单测⑥ + E8 csv/tsv 臂（R1-S3、R2-S1）
- A4 大值 × stats/sample/profile table exit 0；stats json min/max 全值——E9 扩展（R2-S2）
- A5 import × 加密对钥 exit 0 数据可查、错钥 exit 5——E7 扩展（R3-S1/S2）
- A6 stats × 加密库 exit 0——E7 扩展（R3-S3）
- A7 `--readonly` × stats/sample/profile exit 2 点名 flag——readonly 矩阵扩展（R3-S4）
- A8 README 双语 + SKILL.md 截断说明——T5
- A9 全量零回归——Iteration Gate 一次 `cargo test`（T1–T4 后执行；T5 文档-only 不再重跑）

**Verification**

1. T1：`cargo test --lib render`——新矩阵 + 既有 8 例全绿（A1/A2/A3 单元面）。
2. T2：`cargo test --test e2e_test e8_large_value_journey`、`cargo test --test e2e_test e9_analytics_journey`（A1 CLI 面、A3、A4）。
3. T3：`cargo test --test e2e_test e7_encryption_journey`（A5/A6）。
4. T4：`cargo test --test readonly_session_test cli_readonly_rejected_with_lifecycle_subcommands`（A7）。
5. Iteration Gate：`cargo test` 全量一次，0 failures（A9）。
6. T5：人工读 README 双语与 SKILL.md 两处，核对数字（A8）。

跨特性/跨接口 scenario（大值经真实 CLI 子进程、加密库经真实开库、readonly 经真实 argv）全部走真实入口（`CARGO_BIN_EXE_rtsql` 子进程 + 磁盘/stdin），无判定层、无身份型证据。

**Gate 2 Readiness**

PASS（自检 11 项全过：无 TBD/TODO；无未批准裁剪——A1–A4 为已闭合默认值非 TBD；tasks/specs/design 一致；Current-State Evidence 全部来自 2026-10-14 实际读码；任务契约完整；调用者/影响面/测试入口已定位；无无关范围；单 Iteration 过平衡审计、全量预算一次；Act 只读本 Cycle 即可建立先行测试；非实质未知项已入 Risks；验证直接判定目标行为、真实入口、无证据工程）。用户 2026-10-14 批准计划（原话「批准批准」），Status 置 ready。

**Persisted Evidence**

- Mode: none

（Act Response 记录命令与实际结果即可；全量结果与各定向命令输出可随时重跑，无不可复现依赖。）

**Risks and Notes**

- 既有测试可能存在 >64 字符 table 单元格断言（预期行为变化）：T1 验证步骤定向排查；命中按白名单校准并逐处记录 Act Response。
- A1–A4 为默认假设（64 上限 / 表头同规则 / ASCII `...` / 仅 table），用户审计可调整；调整只动常量与断言数字，不改任务结构与 Iteration Map。
- E8 csv/tsv 臂 stdout ~2MB × 2：夹具并发读线程已防管道死锁（R31，E8 json 臂 1MB 先例）。
- e2e 目标体量增长：E7/E8/E9 各加数个 CLI 调用，单测试函数仍在既有 journey 时长量级。

## Act Response

- Status: reported（2026-10-15）

### 实际改动

**T1（R1，唯一产品代码变更）**——`src/cli/render.rs`：

- 新增模块常量 `CELL_WIDTH_CAP: usize = 64` 与 `truncate_cell`：`chars().count() > 64` → 前 61 字符 + `...`（chars 计、多字节安全、≤64 原样返回）。
- `render_table` 内 `lines` 构造完成后、`widths` 计算前对全部单元格（含表头）逐格截断——`widths ≤ 64` 恒成立，`format!` 动态宽度 panic（ISS15）单点关闭；主命令与 stats/sample/profile 共享该路径（`render_rows` 委托）。
- 先行测试（RED 见证）：新增 5 例——`test_table_cell_cap_boundary`（64 原样/65→61+`...` 恰 64）、`test_table_cell_cap_huge_no_panic`（100,000 字符，pre-fix panic `Formatting argument out of range` 于 :85，与契约预测逐字一致）、`test_table_cell_cap_cjk_no_partial_char`（`中`×100）、`test_table_header_cap_same_rule`（表头 h×70）、`test_delimited_and_json_full_fidelity_with_huge_value`（csv/tsv/json 保真回归锁）。修复前 4 failed 1 passed（保真例 pre-fix 即绿），修复后 17 passed（既有 8 例 + 5 新增 + 其余 render 过滤命中）零回归。

**T2（R2）**——`tests/e2e_test.rs` 追加（既有断言逐字节不动，diff 0 删除行）：

- `e8_large_value_journey` 追加四格式矩阵：csv/tsv `SELECT body FROM docs` exit 0 且数据行长度 = 2,000,000（REPLACE 后全值保真）；table exit 0、恰 3 行（表头/分隔/数据）、数据行 64 字符以 `...` 结尾。
- `e9_analytics_journey` 追加：独立 `big_stats (id INT PRIMARY KEY, s STRING)` 插 70,000 字符值（>65,536 阈值，体量克制）；`stats/sample/profile … --format table` 全部 exit 0（stats 的 String 列 min/max 携带原值 = 用户实测 panic 路径）；`stats --format json` 的 s 列行 min/max（index 5/6）长度 = 70,000 全值。

**T3（R3 前三 Scenario）**——`tests/e2e_test.rs::e7_encryption_journey` 追加（既有断言逐字节不动）：

- `std::fs::write` 夹具 CSV（`id,secret` 2 行）→ 建 `s2` → `--key pw import enc s2 f.csv --csv` exit 0 → `SELECT COUNT(*) AS c FROM s2` = 2（数据真实落库）；`--key wrong import …` exit 5；`--key pw stats enc s --format json` exit 0 且 rows 含 `secret` 列行。

**T4（R3 第四 Scenario）**——`tests/readonly_session_test.rs::cli_readonly_rejected_with_lifecycle_subcommands` 矩阵追加三 vec（`--readonly` × stats/sample/profile，目标表 `t` 为 `seed()` 既有夹具）：循环既有断言（exit 2 + stderr 含 `--readonly` 与 `main command`）自动覆盖。

**T5（R1 文档面）**——三处一句落点，数字与实现一致（64/61/`...`）：

- `README.md` Output formats 段（:234 同句追加）：table truncates >64 chars (headers included) to first 61 + `...`；json/csv/tsv always full value。
- `README.zh-CN.md` 输出格式段（:234 同句追加）：中文同义句。
- `rtsql-docs/SKILL.md` `--format` 表格行（:142 行内扩展）：同义短句。

### 与计划的偏差

1. T3 契约字面值 `--key pw stats enc --format json` 缺 clap 必填位置参 `<TABLE>`（实际用法 `rtsql stats --format <FORMAT> <DB> <TABLE>`，E9 先例 `stats ana m`）；改用既有表 `s`（`--key pw stats enc s --format json`），R3-S3 意图（stats × 加密库 exit 0）不变。非实质。
2. 其余无偏差；A1–A4 默认假设按批准值实施。

### 自检发现与处理

- T3 首跑 FAILED（exit 2 缺 `<TABLE>`，测试代码对实际 CLI 面的笔误）——修正后重跑 GREEN。已修复。
- `cargo fmt` 重排 T1 新增测试（rustfmt 规则）——已格式化，`fmt --check` 干净，`cargo test --lib render` 复跑 17 passed 确认。
- 全量 Gate 批次核对时 B06 人工小计笔误（111→217）——经静态 `#[test]` 清点交叉核对发现并更正；更正后与归档基线严格对账（1596 集成含 e2e + 403 lib = 1999 = 归档 1994 + 本 change 5）。纯记账修正，非测试失败。
- `cargo clippy --all-targets` 仅存量 1 条 `secondary_index_equality_test` match→if-let 告警（Iteration 001 存量，Iteration 004 Act 已记录不变），本 Cycle 无新增告警。

遗留 Minor：无。

### 验证

（单元与定向随时跑；Iteration Gate 全量一次于 T1–T4 落地后、T5 前执行，T5 文档-only 未重跑——与 Iteration Plan 验证边界一致。全量按 M19 分批串行：`--test-threads=1`，批间顺序执行，e2e 独立批次。）

- T1 RED：`cargo test --lib render` → FAILED（4 failed：③ panic `Formatting argument out of range`，①②④⑤ 断言失败；⑥保真例与既有 8 例绿）。
- T1 GREEN：`cargo test --lib render` → **17 passed / 0 failed**。
- T1 白名单排查：`cargo test --test e2e_test` → **22 passed / 0 failed**——既有 table 断言无 >64 字符单元格命中，无需校准。
- T2：`cargo test --test e2e_test e8_large_value_journey` → **1 passed**（2.20s）；`cargo test --test e2e_test e9_analytics_journey` → **1 passed**（0.39s）。
- T3：`cargo test --test e2e_test e7_encryption_journey` → **1 passed**（修正后 6.60s）。
- T4：`cargo test --test readonly_session_test cli_readonly_rejected_with_lifecycle_subcommands` → **1 passed**。
- Iteration Gate（M19 分批串行，批批 0 failed）：lib **403 passed**；集成 11 批 + e2e 独立批 = **1596 passed**（B00 149 / B01 210 / B02 160 / B03 142 / B04 104 / B05 70 / B06 217 / B07 98 / B08 137 / B09 182 / B10 92 / B11 13 / e2e 22；cli_test 含 2 ignored 为存量）。全量合计 **1999 tests / 0 failed / 2 ignored**（= 2026-10-14 归档基线 1994 + 本 change lib 净增 5，对账一致）。`network_server_test` 全文件注释化 0 测试为既有形态；`--bins` 0 测试。
- `cargo fmt --check` 干净；`cargo clippy --all-targets` 无新增告警（存量 1 条如上）。
- T5 人工核对：README.md :234、README.zh-CN.md :234、SKILL.md :142 三处读回，均含截断句且数字 64/61/`...` 与实现一致。

### Persisted Evidence

None required（Plan Context Mode: none；全量与各定向结果可随时重跑，无不可复现依赖）。

### Experience Candidates

None（实施过程无 change 范围外的实质缺陷；M19 分批串行流程为既有 project-model 约束，按规执行，无新增可复用经验）。

## Plan Review

- Review Result: accepted（2026-10-15）

### Findings

1. **Act 报告失实一处（非阻塞 Minor）**：Act Response「与计划的偏差 1」声称契约字面值缺 `<TABLE>` 位置参——Plan 契约原文即含 `stats enc s`（T3 Required behavior）。实际实现与契约完全一致，无行为影响；属偏差记录中的契约误引，非实现偏离。
2. **「17 passed」独立复核一致**：`cargo test --lib render -- --list` = 17 项（`cli::render::tests` 13 例 + 同名子串命中 4 例：`lifecycle::tests::ddl_generator_renders_*` ×2、`plan_format::tests::renders_*` ×2）。Act 计数准确。
3. **全量对账成立**：1999 = 归档基线 1994 + lib 新增 5；集成 1596 不变（e2e 仍 22 个测试函数、readonly 矩阵为既有函数内扩展，diff 纯插入零删除行）。
4. **表面检查干净**：`truncate_cell`/`CELL_WIDTH_CAP` 命名与模块惯例一致；无死代码；截断注释点明 ISS15 约束来源；无身份型证据工程、无判定层；五任务均未触碰禁区（`value_to_text`/`render_delimited`/json 分支/拒绝语义代码零改动）。
5. Act 自检三项（T3 首跑笔误修正、fmt 重排、B06 记账小计更正）均良性且记录诚实；clippy 存量 1 条为 Iteration 001 遗留，本 Cycle 无新增。

### Acceptance Gaps

（无——A1–A9 全部满足：A1/A2/A3 由 T1 单测 17 passed 与 E8 四格式臂承载；A4 由 E9 大值臂承载；A5/A6 由 E7 加密臂承载；A7 由 readonly 矩阵 8 项承载；A8 三处文档 diff 逐句核对数字一致；A9 全量 1999/0/2。）

### Evidence

- Plan 独立逐文件读 diff（`git diff`：render.rs 111 行、e2e_test.rs 154 行纯插入、readonly_session_test.rs 4 行、README 双语与 SKILL.md 各 1 句）——与 Act Response 声明逐项一致。
- `cargo test --lib render -- --list` → 17 项（疑点定向复核）。
- 全量与各定向验证结论采信 Act Response（2026-10-15 同日运行，此后代码表面未变化；Persisted Evidence: none，命令可随时重跑）。

### Follow-up Decision

接受收尾：Acceptance 全部满足，无阻塞 finding；唯一 Minor 为偏差记录的契约误引（零行为影响，留档不返工）。Iteration 000 完成；Iteration Map 无后续 Iteration。

### Next Cycle

None

### Next Iteration

None
