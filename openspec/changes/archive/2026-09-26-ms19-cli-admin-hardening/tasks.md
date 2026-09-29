# MS19 CLI 管理面小收口 — Tasks

> 来源：MS19（`.agents/docs/tasks.md`）T01/T02/T03 全部并入本 change（用户裁定 2026-09-26）；Iteration 002（T4/T5/T6）为 2026-09-26 用户批准扩围——Iteration 001 Act 的 Issue 候选 ①② 与 Plan 探针发现的同族静默面（列解析显式化），来源与范围见 proposal 扩围段、design D6-D9。
> 设计依据：`design.md` D1–D8（D9 为不实施记录）。

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T1 | `rtsql delete <db>` 删库子命令（D1：锁探测同窗口删除、伴生清理、释放报告、`--dry-run`、缺失/占用拒绝）+ I072 文件对备份边界实测与 README 双语文档化（D5） | R1（`cli-noninteractive-shell` ADDED） | 000 |
| T2 | PlanError 携带能力名（D2）+ PG 协议错误码分类（D3：`sqlstate()` 映射 + `Response::Error` 可选载荷 + `PlanFailure` 通道） | R2、R3、R4（`error-reporting` ADDED） | 001 |
| T3 | 会话边界组合场景 e2e（D4：`tx_statement_test.rs` 两用例锁定活跃事务 × 边界子句语义） | R5（`sql-transaction-statements` MODIFIED R2） | 001 |
| T4 | 纯列路径投影严格解析（未知裸列/限定列/混合 → `ColumnNotFound` 拒绝）+ 纯非聚合查询 `col AS alias` 别名表头生效（D6） | R6、R7（`error-reporting` ADDED） | 002 |
| T5 | 值表达式/WHERE 列缺失重分类 `ColumnNotFound`（D7：expression.rs 两处 ParseError 站点，文案 `Column not found: {col}`、PG 42703） | R8（`error-reporting` ADDED） | 002 |
| T6 | ORDER BY 排序键计划期校验（D8：按 Sort 节点真实输入列集校验，未知键 `ColumnNotFound` 拒绝） | R9（`error-reporting` ADDED） | 002 |

## Iteration Plan

### Iteration 000: 删库子命令与文件对备份边界

- Tasks: T1
- Depends on: None
- Stable baseline: `rtsql delete` 全场景可用（裸名/路径、dry-run、缺失/占用拒绝、伴生清理、逐文件报告）；README 双语含备份边界段且声明与实测矩阵一致；既有生命周期子命令与主命令零回归
- Verification boundary: `tests/cli_test.rs` delete 矩阵 + 全量测试零回归 + I072 实测矩阵执行并记录
- Diagnostic boundary: `src/cli/mod.rs`（Command 枚举与分派）、`src/cli/lifecycle.rs`（delete_db）、`tests/cli_test.rs`、README
- Non-goals: 错误面改动（T2）、tx e2e（T3）、`install.sh` 语义变更

### Iteration 001: 错误可操作化与会话边界锁定

- Tasks: T2, T3
- Depends on: Iteration 000（仅顺序约定，无共享代码面）
- Stable baseline: `UnsupportedStatement`/`UnsupportedExpression` 报错点名能力；PG 错误响应按类别携带 SQLSTATE（0A000/42601/42P01/42703/42702，其余 58000）；JSON 协议输出字节不变；会话边界组合场景有 e2e 锁定；全量零回归
- Verification boundary: planner/cli 错误面测试 + `PlanError::sqlstate` 映射单测 + `pg_integration_test` 分类断言 + JSON 序列化不变单测 + `tx_statement` 组合用例 + 全量零回归
- Diagnostic boundary: `src/parser/error.rs`、`src/parser/planner/`（31 构造点）、`src/pipeline.rs`（plan_stage/PlanFailure）、`src/network/protocol.rs`、`src/network/pg_protocol.rs`、`tests/{planner,cli,pg_integration,tx_statement}_test.rs`
- Non-goals: StorageError/NetworkError 分类扩展（维持 58000 兜底）；新错误类别文案重写；REPL

### Iteration 002: 列解析静默面收口（投影严格解析 / 列缺失重分类 / ORDER BY 键校验）

- Tasks: T4, T5, T6
- Depends on: Iteration 001（顺序约定；T4 复用其 `ColumnNotFound`→42703 分类通道，无代码耦合面）
- Stable baseline: 未知投影列（裸名/限定名/混合）与未知排序键计划期 `ColumnNotFound` 点名拒绝（不再静默 identity/静默跳过）；纯非聚合查询 `col AS alias` 表头 = 别名、行 = 投影列；WHERE/值表达式列缺失与多表路径同模（`Column not found: {col}`，PG 42703）；既有可解析投影形态、聚合别名、既有可达排序形态、JOIN 列解析零回归；全量零回归（除 spec 白名单内的两处文案变化）
- Verification boundary: `tests/planner_test.rs` 拒绝/零回归矩阵 + `tests/projection_test.rs`/`tests/projection_expression_test.rs` 零校准 + `tests/cli_test.rs` e2e 拒绝与别名表头 + `tests/pg_integration_test.rs` 42703 分类扩展 + 全量串行零回归
- Diagnostic boundary: `src/parser/planner/query.rs`（`resolve_projection_indices`、投影路由 :747-758、角色分类 :570-635、ORDER BY 构造 :1126-1162）、`src/parser/planner/expression.rs`（:283/:340）、`tests/{planner,projection,projection_expression,cli,pg_integration}_test.rs`
- Non-goals: 无聚合 GROUP BY 整体被忽略（D9——语义缺口，Issue 候选不实施）；JOIN 投影解析面改动（既有 JOIN 列解析路径零回归即可）；表达式查询「按非输出基础列排序」的能力扩展（D8 只做校验，不重构 Sort/Projection 链）；聚合路径别名行为改动

### 平衡审计

- Iteration 000 单任务形成完整可验收结果（管理命令闭环 + 数据安全文档），非过碎；验证边界与诊断边界自洽（纯 CLI 文件操作域）。
- Iteration 001 两任务同属错误报告主题（错误面文案 + 错误码分类 + 错误路径回归锁定），共享 planner 错误构造与错误面测试面，聚合不过重（T3 为纯测试）。拆分为两个 Iteration 会在同一批文件上制造两轮机械改动与两轮全量回归，无独立价值。
- Iteration 002 三任务同属「计划期列解析显式化」单一故障域（`resolve_projection_indices` 的 identity 回落桶、`build_expression` 单表误建模、SortExecutor 键静默跳过为同一「静默错误结果」主题的三个成员），共享 planner 错误面测试与 PG 42703 分类见证；T4 内含别名连带（探针证实不可拆——严格解析单独落地会把 `col AS alias` 从静默退化误变为错误拒绝）。拆分会制造三轮同文件往返与三轮全量回归，无独立价值；聚合后工作量适中（每任务 ≤30 行产品代码 + 矩阵测试）。
- 三 Iteration 故障域不同（文件管理 / 错误报告面 / 列解析静默面），拆分成立；依赖边无环。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 delete 子命令（8 场景） | T1 | `tests/cli_test.rs`：delete_happy_bare_name_reports_freed / delete_dry_run_keeps_files / delete_locked_rejects_without_deletion / delete_missing_db_errors / delete_path_form_with_companions / delete_corrupt_file_deletable / delete_absent_companions_skipped + 既有生命周期用例零回归 | Covered |
| R2 能力名携带（3 场景） | T2 | `tests/planner_test.rs` 或 `tests/cli_test.rs`：unsupported_statement_names_capability（MERGE 形态）/ unsupported_expression_names_capability / explicit_rejection_messages_unchanged（CHECK/FK、IN 子查询 JOIN、会话外事务语句文案逐字节） | Covered |
| R3 PG 错误码分类（4 场景） | T2 | `src/parser/error.rs` 单测：planerror_sqlstate_mapping（全变体表）；`tests/pg_integration_test.rs`：pg_capability_error_0a000 / pg_parse_error_42601 / pg_name_resolution_errors（42P01/42703）；serde 单测：response_error_json_omits_none_sqlstate | Covered |
| R4 既有错误语义零回归（2 场景） | T2 | 全量测试（退出码矩阵既有用例集零修改通过）+ R2 的 explicit_rejection_messages_unchanged；`contains` 断言 7 处不修改仍通过 | Covered |
| R5 会话边界组合（+2 场景） | T3 | `tests/tx_statement_test.rs`：active_session_savepoint_rejected_and_rolled_back / active_session_set_transaction_rejected_and_rolled_back | Covered |
| R6 投影列解析显式化（4 场景） | T4 | `tests/planner_test.rs`：unknown_projection_column_rejected（裸名）/ mixed_known_unknown_rejected / unknown_qualified_column_rejected / resolvable_projection_shapes_unchanged（含 `*`/子集/重排/重复/限定有效列）；`tests/cli_test.rs`：test_unknown_projection_column_cli（exit 3）；`tests/pg_integration_test.rs`：pg_unknown_column_42703（裸名形态） | Covered |
| R7 纯查询列别名表头（3 场景） | T4 | `tests/planner_test.rs` 或 `tests/cli_test.rs`：alias_header_and_projected_rows（`name AS n` → `["n"]`）/ alias_unknown_column_rejected；`tests/projection_expression_test.rs`（或 aggregate 面既有用例）：聚合别名零回归（`name AS n, COUNT(*) GROUP BY name` → `["n","count_star"]` 零校准） | Covered |
| R8 列缺失分类一致（3 场景） | T5 | `tests/planner_test.rs`：where_column_missing_column_not_found（文案 `Column not found: nosuch`）/ expression_arg_column_missing（abs 形态）/ join_layout_zero_regression（既有 JOIN ON 用例零校准）；`tests/pg_integration_test.rs`：pg_where_column_missing_42703（SHALL NOT 42601）；既有 `pg_name_resolution_errors`（JOIN ON 42703）零修改 | Covered |
| R9 ORDER BY 键校验（3 场景；2026-09-26 Review 修订——输出别名键维持既有静默语义，不纳入拒绝面） | T6 | `tests/planner_test.rs`：unknown_order_by_key_rejected（单键 + 混合键）/ reachable_order_by_shapes_unchanged（基础列/表达式查询基础列/聚合投影名三形态）；`tests/cli_test.rs`：test_unknown_order_by_key_cli（exit 3）；输出别名静默语义：`tests/scalar_function_test.rs` order_by_expression_alias_keeps_input_order（语料 `sql-scalar-functions` R5 e2e，零修改恢复 GREEN） | Covered |
| R4 既有错误语义零回归（白名单修订） | T2（已交付）+ T5 | R4 白名单扩为「两变体能力名 + 列缺失重分类文案」：既有 contains 断言零修改（T2 已见证）+ T5 的 JOIN ON 用例零校准 + 全量零回归（除白名单） | Covered |

Scenario→Task→Test 链路完整，无 Simplified、无 Missing。
