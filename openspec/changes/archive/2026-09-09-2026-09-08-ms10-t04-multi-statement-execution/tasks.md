# tasks: MS10-T04 多语句执行修复（`;` 分片逐条执行）

> 状态：计划完成，Gate 2 待用户批准后交 openspec-act。
> 用户决策（2026-09-08，Gate 1 会话）：① CLI 逐条执行 + lib 路径显式拒绝；② 顺序渲染每条结果（json 每条独立文档）；③ 错误定位 = 语句序号 + 失败语句文本（「报错行号」口径简化已批准，parse 错误保留行列文本）；④ 逐条 auto-commit。
> 审计依据：本会话 Explorer 调查（revision `5c42ec8`）+ 基线运行（lib 187/0，cli_test 21/0/2，2026-09-08）。

## Iteration Plan

### Iteration 000: 多语句分片逐条执行与静默截断收口

- Tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: `rtsql db "stmt1; stmt2; ..."` 逐条执行、顺序渲染、exit 0；fail-fast 部分生效语义 + 序号错误定位；网络路径与 `execute_in_tx` 多语句显式拒绝；单语句行为零变化（665 基线零回归）
- Verification boundary: `cargo test --all` 全绿（`test_multi_statement_rejected` 重写为分片执行语义，其余既有测试零修改）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/cli/mod.rs`、`src/pipeline.rs`、`tests/cli_test.rs`
- Non-goals: SQL 事务语句（MS11-T02）、整脚本隐式事务、网络多结果协议、`execute_in_tx` 多语句支持、真行号预扫描、REPL/T05

**平衡审计**：T1（分片循环+渲染）、T2（fail-fast 定位）、T3（lib 显式拒绝）、T4（回归门）同属「多语句脚本执行可达且无静默截断」单一验收域；改动面 2 个源文件 + 1 个测试文件，故障域连续（CLI 循环 / 错误路径 / lib 拒绝），验证命令与诊断域完全重叠（cli_test + pipeline 单测 + 全量门）。拆分会产生不可独立验收的中间态（有分片无错误语义 / 有 CLI 无 lib 收口）。合并为单 Iteration，不过重（约百行级源改动 + 6 个新测试 + 1 个重写）。

## Tasks

### T1: CLI 多语句分片逐条执行与顺序渲染

- **Requirement/Scenario**: R-多语句分片逐条执行 / S1 多条 DML 逐条生效、S2 顺序渲染混合语句结果、S5 分号边界语义
- **Depends on**: None
- **Targets**: `src/cli/mod.rs::run_sql`（护栏块 `:203-208` 与单语句执行体 `:210-221`）
- **当前行为**: `statements.len() > 1` → exit 3 拒绝（文案 `one statement at a time`）；单语句 `plan_stage(db, sql, &statements[0], false)`（完整串作缓存键）+ `execute_stage` + 单段渲染
- **目标行为**: 移除护栏；对每条语句 `i`（1-based 总数 n）：以 `stmt.to_string()` 作键调 `plan_stage` → `get_plan_output_columns` → `execute_stage` → 按 Response 渲染并经 `emit_stdout` 逐段写出（QueryResult → Rows / AffectedRows → Affected / Pong → 无输出继续）；全部成功 → `ExitStatus::Success`。分片条数与边界（连续分号、字符串内分号、尾随分号）由 parse_stage/sqlparser 语义自然承载
- **Required changes**: `run_sql` 重构为循环；缓存键换逐条 canonical 文本（design D5）
- **Preserve**: `render()`、`emit_stdout`、`kind()`、信号两阶段机制、单语句可观察行为（输出内容与退出码）零变化
- **Forbidden**: 不改 `render.rs`、`pipeline.rs` 的 stage 函数签名、`plan_cache` 逻辑；不做整体事务包裹
- **Test witness**: RED——重写 `tests/cli_test.rs::test_multi_statement_rejected` → `test_multi_statement_executes`（双 INSERT exit 0 + 两段输出 + 重开两行）；新增 `test_multi_statement_sequential_render`（json 两独立文档）、`test_multi_statement_semicolon_boundaries`（`SELECT 1;; SELECT 'a;b';` 两语句 exit 0）；跑 `cargo test --test cli_test` 观察 RED
- **GREEN condition**: 上述 3 用例绿；单语句既有用例零修改绿
- **Verification**: `cargo test --test cli_test`（exit 0）
- **Stop when**: 循环语义与 `Response` 单结果契约冲突，或 `stmt.to_string()` 作键引发缓存行为异常（实质冲突 → Blocker Handoff）

### T2: fail-fast 错误定位与部分生效语义

- **Requirement/Scenario**: R-多语句分片逐条执行 / S3 中间语句失败 fail-fast、S4 语法错误整体拒绝
- **Depends on**: T1（同一循环体；实现顺序上先有循环）
- **Targets**: `src/cli/mod.rs::run_sql`（错误分支）
- **当前行为**: 语句错误直接透传消息（`Response::Error { message }` → `ExitStatus::Sql(message)`），无序号无定位
- **目标行为**: 执行/计划错误 → `ExitStatus::Sql`，文案 = `statement {k} of {n} failed: {error}; statement: {stmt_text}`（stmt_text ≤200 字符截断）+ k>1 时追加 `; previous statement(s) were committed`（design D3）；语法错误保持 parse_stage 全串拒绝（零执行，文本含 `at Line: X, Column Y`）
- **Required changes**: 循环错误分支组装定位模板
- **Preserve**: 退出码 3 语义；parse 错误现有文案形态（`Parse error: ...` 前缀 + 行列文本）；零执行语义
- **Forbidden**: 不做 token 预扫描/行号计算；不改 parse_stage
- **Test witness**: RED——新增 `test_multi_statement_fail_fast`（INSERT 成功 + INSERT missing_table + 第三条 → exit 3、stderr 含 `statement 2 of 3`、重开仅第 1 条生效）；新增 `test_multi_statement_parse_error_zero_exec`（`INSERT ...; SELEC typo` → exit 3、stderr 含 `Line:`、表仍空）；跑 `cargo test --test cli_test` 观察 RED
- **GREEN condition**: 2 用例绿；T1 用例不回退
- **Verification**: `cargo test --test cli_test`（exit 0）
- **Stop when**: 错误定位与 spec S3 内容契约冲突

### T3: lib 单结果路径显式拒绝多语句

- **Requirement/Scenario**: R-多语句分片逐条执行 / S6 lib 单结果路径显式拒绝
- **Depends on**: None（与 T1/T2 并行无共享代码面）
- **Targets**: `src/pipeline.rs::execute_inner`（`:341` first() 处）、`src/pipeline.rs::execute_in_tx`（`:237` first() 处）
- **当前行为**: `statements.first()` 静默截断（第二条起丢弃）；网络路径还把完整串作键 put 首条 plan（键污染）
- **目标行为**: `parse_stage` 成功后 `len > 1` → 返回 `Response::Error`（文案说明该路径仅支持单语句、多语句请用 CLI 分片），零执行；拒绝发生在 plan_stage/cache put 之前（键污染消失）。单语句路径（含 cache get 在 parse 前的顺序）零变化
- **Required changes**: 两处 first() 消费前加显式拒绝分支
- **Preserve**: 既有 8 个 pipeline stage 单测零修改；单语句网络/事务行为零变化；`Response` 枚举不变
- **Forbidden**: 不扩 `Response` 形状；不做网络多结果；不改 `execute_in_tx` 单语句语义
- **Test witness**: RED——`src/pipeline.rs` tests 模块新增 `execute_sql_multi_statement_returns_error`（`db.execute_sql("SELECT 1; SELECT 2")` → `Response::Error`）与 `execute_in_tx_multi_statement_returns_error`（多语句 → `Response::Error` 且事务仍可用：后续单语句 `execute_in_tx` 正常执行）；跑 `cargo test --lib` 观察 RED
- **GREEN condition**: 2 单测绿；既有 lib 187 基线零回归
- **Verification**: `cargo test --lib`（exit 0）
- **Stop when**: 显式拒绝与既有调用方语义产生实质冲突

### T4: 回归门与全量验证

- **Requirement/Scenario**: R1 参数化 CLI 入口与主命令（全部既有场景回归保持）+ change 级验证边界
- **Depends on**: T1, T2, T3
- **Targets**: 全仓（只读验证）
- **当前行为**: 基线 lib 187/0、cli_test 21/0/2（2026-09-08 实测）
- **目标行为**: `cargo test --all` 全绿（`test_multi_statement_rejected` 已重写，其余既有测试零修改）；clippy/fmt/openspec validate 全 0/PASS
- **Required changes**: 无代码改动（验证任务）
- **Preserve**: 既有测试断言语义零修改（唯一例外：`test_multi_statement_rejected` 重写属 T1）
- **Forbidden**: 不以放宽断言换取通过
- **Test witness**: 变更前基线已留档（本文件头部审计依据）；全量命令输出
- **GREEN condition**: 全量门通过
- **Verification**: `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-08-ms10-t04-multi-statement-execution`
- **Stop when**: 既有测试出现计划外破坏（→ Blocker Handoff，不得静默改断言）

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 参数化入口（文字修正） | S1-S5 既有场景 | Invariants | T4 | 000 | `src/cli/mod.rs` | 既有 cli_test 20 用例零修改 | None | Covered |
| R-多语句分片逐条执行 | S1 逐条生效 | D1/D4/D5/D7 | T1 | 000 | `cli/mod.rs::run_sql` | `test_multi_statement_executes` | None | Covered |
| R-多语句分片逐条执行 | S2 顺序渲染 | D2/D7 | T1 | 000 | `cli/mod.rs::run_sql` + 复用 `render.rs` | `test_multi_statement_sequential_render` | None | Covered |
| R-多语句分片逐条执行 | S3 fail-fast | D3/D4/D7 | T2 | 000 | `cli/mod.rs::run_sql` | `test_multi_statement_fail_fast` | None | Covered |
| R-多语句分片逐条执行 | S4 语法错误零执行 | D3/D8 | T2 | 000 | `cli/mod.rs::run_sql`（parse 先行） | `test_multi_statement_parse_error_zero_exec` | None | Covered |
| R-多语句分片逐条执行 | S5 分号边界 | D8 | T1 | 000 | `cli/mod.rs::run_sql`（sqlparser 语义） | `test_multi_statement_semicolon_boundaries` | None | Covered |
| R-多语句分片逐条执行 | S6 lib 显式拒绝 | D6/D7 | T3 | 000 | `pipeline.rs::execute_inner`/`execute_in_tx` | pipeline 单测 ×2 | None | Covered |

简化登记：tasks.md MS10 outcome「报错行号」→ 执行期错误用语句序号 + 语句文本（parse 错误保留行列文本）——用户决策 3 已批准（2026-09-08），非未批准裁剪。
