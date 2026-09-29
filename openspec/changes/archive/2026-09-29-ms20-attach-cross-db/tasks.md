# Tasks — MS20 一期 ATTACH 式跨库交互

## Tasks

- [x] T1: ATTACH 会话语句与注册面（`sql-attach-session` R1–R3）——分类器 + CLI 四臂分派 + AttachSession 生命周期 + 拒绝矩阵 + lib session-only 拒绝
- [x] T2: 两段限定名路由与只读跨库查询（`cross-db-namespace-query` R1–R2）——分段判定路由 + PlanBuilder 附加库 schema 注册 + 执行面按库路由 + plan cache 绕过 + 三段列引用
- [x] T3: 附加库只读边界拒绝面（`cross-db-namespace-query` R3）——五个写消费点计划期点名拒绝
- [x] T4: CREATE TABLE AS SELECT 主库物化（`create-table-as-select` R1）——CTAS 混合臂 + 类型推导 + catalog 先落盘 + 失败零副作用清理 + 混合形态/LIKE/CLONE 诚实化拒绝（同臂顺带）

## Iteration Plan

### Iteration 000: ATTACH 注册面与拒绝矩阵

- Tasks: T1
- Depends on: None
- Stable baseline: `ATTACH '<path>' AS <alias>` 在 CLI 会话注册附加库（独占锁、密钥通道复用、存在性前置检查）；拒绝矩阵全绿（重复别名/路径缺失/锁冲突含自附加/lib session-only）；后续 Iteration 可依赖注册面取附加库实例
- Verification boundary: `tests/attach_session_test.rs` 注册/拒绝矩阵 e2e（含 exit 3/4/5 映射、多语句同调用可见、进程退出锁释放）+ planner 单元测试（分类器/build_plan 拒绝）+ 全量零回归
- Diagnostic boundary: `src/parser/planner/mod.rs`（classify + build_plan 前置）、`src/cli/mod.rs`（run_sql 四臂/epilogue）、`src/cli/resolve.rs`
- Non-goals: 限定名路由、跨库查询、CTAS、DETACH（已豁免）

### Iteration 001: 限定名路由与只读跨查

- Tasks: T2, T3
- Depends on: Iteration 000（注册面）
- Stable baseline: SELECT 全形态跨库组合可用；两段名路由（命中别名→附加库、未命中→扁平名回落零回归）；三段列引用在路由命中时可用；附加库写面全点名拒绝；含附加库引用语句不进 plan cache
- Verification boundary: `tests/cross_db_query_test.rs`（路由矩阵：命中/回落/含点扁平名回归/三段拒绝；JOIN/子查询/聚合/CTE/集合操作跨库组合；写面拒绝矩阵；plan cache 不污染见证）+ 全量零回归
- Diagnostic boundary: `src/pipeline.rs`（extract/register_table/create_executor_from_plan/cache 绕过）、`src/parser/planner/{query,expression,ddl_dml}.rs`、`src/parser/ast.rs`
- Non-goals: CTAS（Iteration 002）；lib 路径 attach（保持零变化）

### Iteration 002: CTAS 主库物化

- Tasks: T4
- Depends on: Iteration 001（跨库 SELECT 作为物化源）
- Stable baseline: `CREATE TABLE t AS SELECT ...`（源可含附加库引用）端到端物化、跨重启可读；类型推导矩阵（全 NULL→String(255)/Int∪Float→Float/冲突点名拒绝）；隐式主键 DuplicateKey 与目标已存在失败零副作用；显式事务内 CTAS 点名拒绝；catalog 先落盘保崩溃不变砖
- Verification boundary: `tests/ctas_test.rs`（物化形状/类型推导矩阵/失败零副作用矩阵/重启读回/源含附加库/事务内拒绝）+ 全量零回归
- Diagnostic boundary: `src/pipeline.rs`（CTAS 混合臂）、`src/parser/planner/ddl_dml.rs`（CTAS plan 臂）、`src/storage/data/table_manager.rs`、`src/executor/insert.rs`
- Non-goals: CTAS 携带约束；显式事务内 CTAS（点名拒绝）；CTAS 目标写附加库（点名拒绝）

## 平衡审计

- Iteration 000 小（单语句 + 注册面）但形成独立可验收基线（注册/拒绝矩阵），且是 001/002 的硬前置；合并进 001 会使路由改造与注册面故障域混杂。
- Iteration 001 为最大 Iteration（读面路由 + 写面拒绝），两者共享同一批表名消费点与 PlanBuilder 通道，拆分会造成同面两次穿越。
- Iteration 002 独立故障域（DDL+DML 混合臂、类型推导、崩溃一致性序），依赖 001 的跨库 SELECT 能力，不并入。
