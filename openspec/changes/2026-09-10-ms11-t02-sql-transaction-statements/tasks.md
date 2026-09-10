# tasks — MS11-T02 SQL 事务语句 BEGIN/COMMIT/ROLLBACK

> 规划：openspec-plan 2026-09-10；Gate 1 已批准（需求 + 场景 + 范围，用户裁定 4 项决策）。
> 实施：Iteration 000（T1-T3）已完成（2026-09-10，Act Response 见 `iterations/000-tx-statement-layer/000-initial.md`；781 tests pass / 0 failed / 2 ignored）。
> 实施：Iteration 001（T4-T6）已完成（2026-09-10，Act Response 见 `iterations/001-cli-session/000-initial.md`；797 tests pass / 0 failed / 2 ignored）。

## Task List

| Task | 状态 | 目标 | 关键产出 | Iteration |
|---|---|---|---|---|
| T1 | **completed**（2026-09-10） | 事务语句分类器 + planner 臂 + `PlanError::TransactionStatement` | `classify_transaction_statement`（`src/parser/planner/mod.rs`）+ 单测；`build_plan` 前置分类（R2 lib 侧文案、R4 session-only 拒绝） | 000 |
| T2 | **completed**（2026-09-10） | `TransactionSession` 会话基元 | `src/transaction/session.rs`（begin/commit/rollback/tx_id/is_active/tx，错误文案按 design D3）+ 单测 | 000 |
| T3 | **completed**（2026-09-10） | lib 非会话路径拒绝集成测试 | `tests/tx_statement_test.rs` lib 段：`execute_sql("BEGIN")` / `execute_in_tx("COMMIT")` / `SqlHandler` JSON 同源拒绝 + plan cache 长度不变（R4 S1-S3） | 000 |
| T4 | **completed**（2026-09-10） | CLI `run_sql` 会话接线 | 分类分派（BEGIN/COMMIT/ROLLBACK → session；边界子句 → Sql 错误；其余 → 事务内 `execute_stage_in_tx` / 既有 `execute_stage`）+ `sql_failure_status` 上下文后缀 + 两条退出路径收尾回滚（R1/R3 行为面） | 001 |
| T5 | **completed**（2026-09-10） | CLI e2e 测试 | `tests/tx_statement_test.rs` CLI 段 16 测试：R1 S1-S5、R2 S1-S5（含 S5 `AND NO CHAIN` AST 等价锁定）、R3 S1-S4、多语句修订 S7-S8（真二进制夹具复用 cli_test 模式；pre-T4 RED 12 / post-T4 全绿） | 001 |
| T6 | **completed**（2026-09-10） | 全量回归清扫 | `cargo test` 797 passed / 0 failed / 2 ignored（781+16 只增不减）、clippy/fmt 0、`openspec validate` PASS（R5） | 001 |

## Iteration Plan

### Iteration 000: lib 事务语句层（分类器、会话基元、非会话路径拒绝）

- Tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 事务语句在两条 lib 路径获得精确拒绝文案（R2/R4 关闭）；`TransactionSession` 可用但未被 CLI 接线；CLI 对事务语句的报错文案由 `Plan error: Unsupported statement type` 变为精确文案，其余 CLI 行为零变化
- Verification boundary: 分类器/session 单测 + R4 集成测试绿；全量回归 0 failed；clippy/fmt 0、validate PASS
- Diagnostic boundary: `src/parser/planner/mod.rs`、`src/parser/error.rs`、`src/transaction/session.rs`
- Non-goals: CLI 会话行为（T4-T6）、引擎事务语义、渲染层

### Iteration 001: CLI 会话接线与端到端验收

- Tasks: T4, T5, T6
- Depends on: Iteration 000（分类器与会话基元）
- Stable baseline: MS11-T02 全部 Acceptance 关闭（R1-R5 + 多语句修订场景）
- Verification boundary: T5 e2e 全绿 + T6 全量基线（测试总数只增不减）
- Diagnostic boundary: `src/cli/mod.rs::run_sql`/`sql_failure_status` + `tests/tx_statement_test.rs`
- Non-goals: 网络路径会话支持、PG 事务状态字节、SAVEPOINT 族、隔离级别语义

### 平衡审计

- Iteration 000 为 lib 层内聚结果（parser 判定 + transaction 基元 + 拒绝面），可独立验证与排障；不含 CLI 接线，故障域（解析/事务）与 Iteration 001（CLI 编排）分离。
- Iteration 001 收敛于端到端可观察验收，不混入 lib 层新增。
- 单 Iteration 承载全部任务会混合 parser/transaction 与 CLI 两个故障域且验证面过宽——按两轮拆分；不按行数/task 数切分。
