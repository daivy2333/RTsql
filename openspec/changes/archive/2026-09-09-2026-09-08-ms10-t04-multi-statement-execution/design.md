# design: MS10-T04 多语句执行修复（`;` 分片逐条执行）

> 采集 revision：`5c42ec8`（master，工作树干净）；基线验证 2026-09-08：
> `cargo test --lib` → 187 passed / 0 failed（exit 0）；`cargo test --test cli_test` → 21 passed / 0 failed / 2 ignored（exit 0）。
> 调查输入：本会话 Explorer 即时结论（同 revision，逐条复核有效）+ 上述基线运行。

## 当前行为 vs 目标行为

| 维度 | 当前 | 目标 |
|---|---|---|
| CLI `rtsql db "stmt1; stmt2"` | exit 3 拒绝（护栏，`src/cli/mod.rs:203-208`） | 逐条执行、顺序渲染、exit 0 |
| CLI 中间语句失败 | —（拒绝，零执行） | 前 k-1 条已生效；exit 3，错误含序号（第 k/共 n 条）+ 失败语句文本 + 已生效注明 |
| CLI 语法错误 | 整体拒绝（parse_stage 全串解析） | 不变（零执行），错误保留 sqlparser 行/列文本 |
| 网络 `execute_sql` 多语句 | 静默只执行第一条（`pipeline.rs:341`） | `Response::Error`（显式拒绝），零执行 |
| `execute_in_tx` 多语句 | 静默只执行第一条（`pipeline.rs:237`） | `Response::Error`（显式拒绝），零执行 |
| plan_cache（多语句串作键） | 网络路径把完整串作键 put 首条 plan（键污染） | 显式拒绝在 put 之前发生，污染消失；CLI 逐条用每条语句自身 canonical 文本作键 |
| 单语句（含尾分号） | 正常 | 零变化 |

## 关键决策

### D1 分片执行位于 CLI 层（循环复用既有 pub pipeline stages）
`run_sql` 对 `parse_stage` 产出的 `Vec<Statement>` 逐条调用 `plan_stage` + `execute_stage`（`src/pipeline.rs:56/96`，均 pub）。理由：`Response` 单结果契约与协议层零改动；网络/事务路径以显式拒绝收口；改动面最小。替代方案否决：pipeline 层循环需要扩 `Response` 形状或协议多结果消息（超出 T04 诊断边界）；pipeline 层整体隐式事务被用户决策 4 否决。

### D2 顺序渲染每条结果（复用 `render()`，`render()` 本身零修改）
每条语句执行后立即渲染并写 stdout（流式，不累积）：`Response::QueryResult` → `QueryPayload::Rows`；`AffectedRows` → `QueryPayload::Affected`；渲染文本经既有 `emit_stdout`（追加 `\n`）逐段写出。`json` 格式下自然形成「每条语句一个独立 JSON 文档、每文档一行」（行集 `{"columns":[...],"rows":[...]}`、DML `{"affected_rows":N}`，`src/cli/render.rs:24-37`）——JSONL 风格逐行可解析。`table/csv/tsv` 为多段拼接（限制在 proposal Impact 与本文档注明；机器消费以 json 为准）。`Response::Pong` 不可达（仅网络 `Request::Ping` 产生，`src/network/handler.rs:19`），分支保持防御性现状（Success，无输出）。`columns` 提取保持现状做法：对每条计划调用 `get_plan_output_columns`，按 Response 分支使用。

### D3 fail-fast 错误定位：语句序号 + 失败语句文本
错误模板：`statement {k} of {n} failed: {error}; statement: {stmt_text}`；`stmt_text` 为失败语句 canonical 文本（`Statement::to_string()`），超过 200 字符截断加 `...`。已生效注明追加 `; previous statement(s) were committed`（k > 1 时）。错误文案的具体措辞为非实质项，序号、语句文本、已生效注明三项内容为契约（delta spec S3）。语法错误路径不套模板（parse_stage 整串解析、零执行，错误文本已含 `at Line: X, Column Y`，`pipeline.rs:43` + sqlparser `parser_err!` 拼接）。

### D4 事务边界：逐条 auto-commit（用户决策 4）
循环内每条语句经 `execute_stage` 独立 begin/commit/abort（MS06-T01 语义，`pipeline.rs:115-192`），不加整体包裹。失败前语句已提交是可观察事实，由 D3 错误模板注明。DDL 非事务性因此自然正确。

### D5 plan_cache 键：逐条 canonical 文本
`run_sql` 对**每条**语句（含单语句）以 `stmt.to_string()` 作为 `plan_stage` 的 `sql` 实参。理由：完整多语句串作键会让每条 SELECT 互相覆盖同一键，且后续相同多语句串 cache-hit 直接执行被缓存的最后一条 plan（静默错结果）；单语句换键只影响 one-shot 进程内命中身份（plan_cache 每库每进程新建，CLI one-shot 进程内几乎无复用价值），无跨进程可观察影响。`plan_stage` 签名与 normalize 逻辑零修改。网络路径修复后不再产生完整串键 put（D6）。

### D6 lib 显式拒绝（消灭静默截断）
`execute_inner`（`pipeline.rs:341` 处）：`parse_stage` 成功后 `statements.len() > 1` → 返回 `Response::Error`，文案说明该路径仅支持单语句、多语句请用 CLI 分片（具体措辞非实质，显式 Error + 零执行为契约）。发生在 `plan_stage` 的 cache put 之前 → 键污染消失。`execute_in_tx`（`pipeline.rs:237` 处）同样处理。单语句路径零变化（cache get 在 parse 前的既有顺序保持）。

### D7 测试见证策略（RED 起步）
| 见证 | 位置 | RED 形态 |
|---|---|---|
| 多条 DML 逐条生效 + 两段输出 | `tests/cli_test.rs` 重写 `test_multi_statement_rejected` → `test_multi_statement_executes` | 现状 exit 3 拒绝 → RED |
| json 顺序两文档 | `tests/cli_test.rs` 新增 `test_multi_statement_sequential_render` | 现状 exit 3 → RED |
| fail-fast 部分生效 + 序号 | `tests/cli_test.rs` 新增 `test_multi_statement_fail_fast` | 现状 exit 3 且零执行 → RED |
| 语法错误零执行（含 `Line:` 断言） | `tests/cli_test.rs` 新增 `test_multi_statement_parse_error_zero_exec` | 部分满足（现状也零执行但文案不同）→ 断言级 RED |
| 分号边界（连续/字符串内/尾随） | `tests/cli_test.rs` 新增 `test_multi_statement_semicolon_boundaries` | 现状 exit 3 → RED |
| lib 显式拒绝 ×2（execute_sql / execute_in_tx） | `src/pipeline.rs` tests 模块（复用 `open_db_with_table` fixture） | 现状返回首条结果 / 截断 → RED |

既有 `cli_test` 其余 20 用例 + 全量 665 基线零修改回归。单语句含尾分号回归由 `test_multi_statement_semicolon_boundaries` 与既有用例共同覆盖。

### D8 sqlparser 事实（已核实，Act 直接依赖）
- `Parser::parse_sql` 全串解析、按 `SemiColon` token 分片：连续分号跳过空语句、尾随分号合法、字符串字面量内 `;` 不分片（sqlparser-0.44.0 `src/parser/mod.rs:401-424`）。
- 解析器错误文本自带 ` at Line: X, Column Y`（`parser_err!` 拼接 + `Location` Display；TokenizerError 同）。
- `Statement: Display` 提供 canonical 文本（缓存键与错误模板用）。
- 分片条数与 `parse_stage` 返回的 `Vec<Statement>.len()` 一致，无需 token 预扫描对齐（用户决策 3 采用序号口径，预扫描不做）。

## Change Surface

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R-分片/S1,S2,S5 | `src/cli/mod.rs::run_sql` | 护栏拒绝多语句；单语句执行+单段渲染 | 移除护栏；逐条循环（plan/execute/渲染/写 stdout）；键换 `stmt.to_string()` |
| T2 | R-分片/S3,S4 | `src/cli/mod.rs::run_sql` | 错误直接透传（单语句语义） | 失败语句序号/文本/已生效注明模板；parse 错误路径保持 |
| T3 | R-分片/S6 | `src/pipeline.rs::execute_inner`、`execute_in_tx` | `statements.first()` 静默截断 | len>1 → `Response::Error` 显式拒绝（put 之前） |
| T4 | R1 全部场景（回归保持） | 全仓 | 既有 665 基线 | `cargo test --all` 零回归 + clippy/fmt/validate |

## Invariants

- `parse_stage` / `plan_stage` / `execute_stage` / `Response` / `ExitStatus` / `render()` 签名与语义不变。
- 单语句执行（含尾分号）行为零变化；网络与显式事务路径的单语句行为零变化。
- 信号两阶段停机机制（`execute_command_inner`）不变；循环取消时已提交语句保留、close() checkpoint 照常。
- `plan_cache::normalize_sql_key` 与容量/淘汰逻辑不变；网络路径单语句缓存键（完整串=单语句文本）不变。
- 不新增依赖、不新增模块。

## Non-goals

SQL 事务语句（MS11-T02）；整脚本隐式事务；网络多结果协议；`execute_in_tx` 多语句支持；真·行号预扫描；REPL/T05；Windows。

## Risks and Notes

- `Statement::to_string()` 的 Display 保真度未逐语法验证（非实质：键仅用于进程内缓存身份，同 AST 必产同 plan；不同语法写法归并为同键属期望行为）。
- table/csv/tsv 多段拼接的整段 stdout 不再是单一文档（机器消费以 json/JSONL 为准；proposal Impact 已注明）。
- sqlparser 错误行号文本对个别 parser 错误路径可能缺失（非 `expected` 系错误）——测试只断言 `Line:` 在典型语法错误中出现，不断言所有错误类。
