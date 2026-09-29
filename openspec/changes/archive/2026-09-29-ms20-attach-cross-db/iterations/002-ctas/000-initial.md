# Iteration 002 / Cycle 000: CTAS 主库物化

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: ATTACH 注册面（Iteration 000）与限定名路由/只读边界（Iteration 001）——跨库 SELECT 作为物化源可用；`plan_stage_routed`/`execute_stage_routed`/`execute_stage_in_tx_routed` 穿线通道；proposal 假设 7/10（用户裁定 2026-09-29：首列隐式主键 + 失败清理、形状/类型边界 design 定稿）；design D5
- Excluded scope: CTAS 携带约束声明；显式事务内 CTAS（点名拒绝）；CTAS 目标写附加库（点名拒绝，T3 拒绝面保持）；DETACH；跨文件 DML

**Objective**

`CREATE TABLE t AS SELECT ...` 在主库端到端物化：列名取 SELECT 输出表头、类型自输出形状推导、目标已存在/失败零副作用、物化结果跨重启可读；SELECT 源可含附加库引用（一次性物化）；显式事务内 CTAS 与混合形态/LIKE/CLONE 计划期点名拒绝（消除 `query` 字段静默丢弃）。

**Investigation Facts**

- Current Baseline: Iteration 001 Cycle 000 accepted（2026-09-29）——全量串行 1441 passed + 2 ignored（= 1425 历史总数口径 + 净增 18，对账见 Iteration 001 Plan Review Findings 1）；ATTACH 注册 + 限定名路由 + 附加库只读边界可用；`AttachRoute`/`RoutedTables` 穿线通道现成。
- Current-State Evidence:
  - **现状（诚实化收口目标）**：`build_plan` 的 `Statement::CreateTable { name, columns, constraints, .. }` 臂（`src/parser/planner/mod.rs:311-316`）**静默丢弃 `query` 字段**——`CREATE TABLE t (a INT) AS SELECT ...` 建出空表不报错；LIKE/CLONE 成分同域（sqlparser 0.44 `CreateTable` AST 携带 `query`/`like`/`clone` 字段，拒绝臂对照实际 ast 定稿）。CTAS 的 SELECT 部分不经 `register_table`（`plan_stage` DDL 裸臂 `src/pipeline.rs:182-190` 只 `build_plan`），源表存在性现状不查。
  - **建表通道**：`TableManager::create_table_with_constraints(name, Vec<(String, ColumnType, bool, bool, Option<Value>)>, pk)`（`src/storage/data/table_manager.rs:315`）——CTAS 用 flags=false / DEFAULT=None / pk=首列名；**重复检查在函数顶部**（`DuplicateTable`，先于任何页分配）→「目标已存在零副作用」由既有检查天然满足。首列隐式主键 fallback 先例 `src/executor/create_table.rs:74-88`（`primary_key: None` → `columns.first()`）。
  - **崩溃一致性硬约束（design D5-1）**：catalog 无 WAL（DDL 不落 WAL，schema 仅靠脏页 flush 持久化），WAL redo 按表名查 catalog（`src/wal/recovery.rs`），缺表 → `RedoFailed` → 库变砖——**必须先 `BufferPool::flush_all()`（`src/storage/buffer_pool.rs:216`）落 catalog 页，再写任何数据 WAL 记录**；此后崩溃残留「schema 完整的空表」，redo 不变砖。
  - **类型推导层约束（design D5-2）**：`Response` 只有 JSON 行，类型推导须在 `ExecResult::Row(Vec<Value>)` 层完成（`Value` 变体携带类型；DerivedScan 物化先例 `src/pipeline.rs:1063-1082`）。
  - **插入通道**：`InsertExecutor::with_table_manager`（`src/pipeline.rs:585` 起 DML 臂同型；`src/executor/insert.rs`）——隐式事务 begin → 逐行 insert → commit/abort，全部既有校验门自然生效（`DuplicateKey`/MS24 类型门/MS23 NOT NULL）。
  - **执行编排**：`execute_stage_routed` 的 DML 臂（`src/pipeline.rs:262` 起）是「begin → abort-meta 预取 → create_executor(tx_id) → execute → commit/abort」的既有编排；CTAS 混合臂的插入段按同型（design D5 步骤 6-7：失败 abort + best-effort `drop_table` 清理）。
  - **输出表头**：`get_plan_output_columns`（`src/parser/planner/query.rs:51`）提供 SELECT 输出列名（表达式项 = 别名或 Display 列名，MS25 装配）——CTAS 列名的直接来源。
  - **显式事务判定**：CLI 会话 `session.tx_id()`（`src/cli/mod.rs:587`）——活跃事务内 CTAS 在此点名拒绝（会话态不变，拒绝路径同既有 `sql_failure_status` 收口）。
  - **附加库目标拒绝（已由 T3 交付）**：`mark_attached_ddl_targets`（`src/pipeline.rs:1536-1548`）对 `CREATE TABLE o.m ...` 不查 catalog 即标记 → `reject_attached_write` 计划期拒绝——**CTAS 改造必须保持该行为**（测试 `attached_database_ddl_rejected_at_plan_time` 已锁定 `CREATE TABLE o.newt`；T4 补 CTAS 形态见证）。
  - **plan cache**：DDL 执行后 `plan_cache.clear()`（`src/pipeline.rs:143` DDL 臂先例）；CTAS 同型（design D5 步骤 8）。
- Code and Critical Path:
  - `src/parser/planner/mod.rs::build_plan`——`CreateTable` 臂解构扩展（`query` 消费）；CTAS 计划形态（新 `PhysicalPlan::CreateTableAsSelect` 节点或等价携带：目标名 + SELECT 计划 + 输出列名）。
  - `src/pipeline.rs`——`plan_stage_routed` DDL 臂为 CTAS 的 SELECT 部分开 `register_table` 注册通道（routed 表照常判定）；`execute_stage_routed` 新 CTAS 混合臂（物化 → 类型推导 → 建表 → flush_all → 插入事务 → 失败清理 → cache clear）。
  - `src/parser/planner/ddl_dml.rs`——混合形态/LIKE/CLONE 计划期点名拒绝（诚实化）。
  - `src/cli/mod.rs::run_sql`——普通臂路由 CTAS 计划至混合臂；活跃事务内点名拒绝。
  - `src/storage/data/table_manager.rs` / `src/executor/create_table.rs` / `src/executor/insert.rs`——经既有通道消费，预期零签名变化。

**Implementation Guidance**

建议顺序：诚实化拒绝（query 消费 + LIKE/CLONE，先行 RED）→ CTAS 计划形态与 plan_stage 注册通道 → execute_stage 混合臂（类型推导矩阵 → 建表 → flush → 插入）→ 失败清理 → 显式事务拒绝 → e2e 矩阵。关键序：**建表 → flush_all → 再插入**（catalog 先于数据 WAL，design D5 硬约束）；类型推导在全物化行上逐列扫描（全 NULL → String(255)、Int∪Float → Float、全同型 → 该型、其他混合/列名重复 → 建表前点名拒绝零副作用）；列名取 `get_plan_output_columns`。跨库源经 Iteration 001 路由通道自然可达（SELECT 计划含附加库扫描节点，执行时按 `routed` 取库）。

**Task Contracts**

### T4: CREATE TABLE AS SELECT 主库物化

- Requirement/Scenario: `create-table-as-select` R1 全部 9 场景
- Depends on: Iteration 001 路由（跨库 SELECT 源）
- Targets: `src/parser/planner/mod.rs::build_plan`（CreateTable 臂 query 消费）、`src/parser/planner/ddl_dml.rs`（混合/LIKE/CLONE 拒绝）、`src/pipeline.rs::{plan_stage_routed, execute_stage_routed}`（CTAS 注册通道 + 混合臂 + 类型推导）、`src/cli/mod.rs::run_sql`（事务内拒绝 + 混合臂接线）
- Current behavior: `query` 字段被静默丢弃——`CREATE TABLE t (a INT) AS SELECT ...` 建空表不报错、`CREATE TABLE t AS SELECT ...` 报 `EmptyColumnDefinition`、LIKE/CLONE 成分被忽略；无任何物化能力。
- Required behavior:
  - 主库物化：`CREATE TABLE m AS SELECT a, b FROM t WHERE a > 10` → 主库出现 `m` 含过滤行；重开库后 `SELECT * FROM m` 一致。
  - 跨库源：`ATTACH ... AS o` 后 `CREATE TABLE m AS SELECT * FROM o.u` → 主库 `m` 物化 `o.u` 全部行（一次性，后续附加库变化不影响 `m`）。
  - 目标已存在：点名拒绝（`DuplicateTable` 同型），既有表零变化。
  - 附加库目标：`CREATE TABLE o.m AS SELECT ...` 计划期点名拒绝（T3 拒绝面保持）。
  - 表达式与别名：`SELECT a + 1 AS inc` → 列名 `inc`、值为求值结果。
  - 类型推导矩阵：全 NULL 列 → `String(255)`；Int∪Float 混合 → `Float`；全同型 → 该型（含 Date/Timestamp）；不可行混合 → 建表前点名拒绝零副作用；输出列名重复（`SELECT a, a`）→ 建表前点名拒绝。
  - 隐式主键冲突：首列 Int 重复值 → `DuplicateKey` 点名失败；失败后 `m` 不存在（abort + best-effort drop，同名可重试）。
  - 源失败：`SELECT * FROM nope` → 点名失败且主库无 `m`。
  - 显式事务内：`BEGIN; CREATE TABLE m AS SELECT ...` → 点名拒绝，会话事务状态不变。
  - 混合/LIKE/CLONE：列清单与 query 并存 / `LIKE other` / `CLONE other` → 计划期点名拒绝（SHALL NOT 静默忽略成分）。
  - CTAS 执行后 plan cache 全清（DDL 同型）。
- Preserve: 既有 CREATE TABLE（无 query/like/clone）路径零变化；T3 附加库 DDL 拒绝行为与既有测试保持；既有 DML/DDL 执行编排零变化；`create_table_with_constraints` 签名不变。
- Forbidden: CTAS 携带约束声明（PK/UNIQUE/NOT NULL/DEFAULT 不带入）；事务内 CTAS 放行；CTAS 目标写附加库；catalog/WAL 格式变更；新增建表旁路（必须走 `create_table_with_constraints`）。
- Test witness: `tests/ctas_test.rs`（新，RED：CTAS 形态当前报 `EmptyColumnDefinition` 或静默建空表）——物化形状与重启读回、跨库源、目标已存在、附加库目标、类型推导矩阵、列名重复、隐式主键冲突零副作用（失败后同名重试成功）、源失败零副作用、事务内拒绝（事务仍可用）、混合/LIKE/CLONE 拒绝矩阵。命令：`cargo test --test ctas_test`；全量 `cargo test -- --test-threads=1`。
- GREEN condition: 上述全绿；全量零回归（基线 1441 passed + 后续净增，引用输出行原样）。
- Verification: `cargo test --test ctas_test`（exit 0）；`cargo test -- --test-threads=1`（exit 0，0 failures）。失败含义：物化语义、崩溃一致性序或零副作用承诺未达成。
- Stop when: 类型推导需要 `ExecResult::Row` 层之外的信息、或崩溃一致性需要新 WAL/catalog 机制（超出「先 flush 后插入」序）、或 sqlparser CTAS 形态解析超出已查证范围、或失败清理需要超出「abort + drop_table」的新回滚协议。

**Invariants**

- 全量测试零回归（基线 1441 passed / 2 ignored + 本轮净增；记录引用输出行原样）。
- 崩溃一致性序：catalog flush 先于任何数据 WAL 记录（design D5 硬约束，不可倒置）。
- lib 公开 API 零变化；无注册会话与 lib 路径行为零变化（CTAS 混合臂仅在 CLI 会话经路由上下文可达——lib `execute_stage` 委托壳行为零变化）。
- 失败零副作用：任何失败点之后目标表不存在于库中。
- 附加库一期只读：CTAS 只写主库。

**Non-goals**

- CTAS 携带约束；事务内 CTAS；CTAS 目标写附加库；`CREATE VIEW`；增量物化/活视图；DETACH；跨文件 DML。

**Acceptance**

- `create-table-as-select` R1 全部 9 场景有测试见证且 GREEN。

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| create-table-as-select R1（物化/跨库/拒绝/零副作用矩阵） | T4 | `tests/ctas_test.rs` 9 场景矩阵 + 重启读回 | Covered |

- 崩溃一致性序（先 flush 后插入）在实现中成立且被混合臂结构保证。
- 全量零回归。

**Verification**

- `cargo test --test ctas_test` → exit 0。
- `cargo test -- --test-threads=1` → exit 0，0 failures（决定性输出：test result 行原样）。

**Gate 2 Readiness**

PASS（2026-09-29 用户批准 change 计划——proposal 假设 7/10 用户裁定与 Iteration Plan；本 Cycle 未超出已批范围；CTAS 执行序与硬约束于 design D5 定稿，代码表面行号经 Iteration 001 Plan Review 当日复核）——无 BLOCKED/WAIVED 项；Persisted Evidence: none。

**Persisted Evidence**

- Mode: none

（Act Response 承载命令、输出与退出码；全部结论可低成本重跑。）

**Risks and Notes**

- CTAS 计划形态（新 `PhysicalPlan` 变体 vs 复用 CreateTable 节点加性字段）、混合臂在 `execute_stage_routed` 的具体编排位置、类型推导的具体数据结构：等价实现选择，留 Act。
- `SELECT a + 1`（无别名）的列名取 Display 文本（MS25 既有表头语义）；`SELECT *` 展开经既有投影装配。
- 大结果集物化内存量级与 Response 物化同阶（design D5），不做流式化（I065 域）。
- `DROP TABLE` 清理失败（如页释放错误）时的收口：best-effort 语义（design D5 步骤 7），原始错误优先传播；若暴露清理失败导致残留表的可达形态，属实质未知项，停并返回 Plan。

## Act Response

- Status: reported
- Cycle: `iterations/002-ctas/000-initial.md`（initial，T4）
- Review Result: pending（由 Plan Review 填写）

### 实际改动

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/executor/plan.rs` | `PhysicalPlan::CreateTableAsSelect` / `CreateTableAsSelectNode` | 新计划形态（`table_name` + `select: Box<PhysicalPlan>` + `columns`） |
| `src/executor/mod.rs` | `pub use plan::{…}` | 节点再导出 |
| `src/parser/planner/ddl_dml.rs` | `build_create_table` | 签名 `&self` → `&mut self`，新增 `query` / `like` / `clone` 三个入参；`LIKE` / `CLONE` / 「列清单或表约束 + query」混合形态计划期点名拒绝；`query` 分支委派新私有方法 |
| `src/parser/planner/ddl_dml.rs` | `build_create_table_as_select`（新） | `build_query` 构造源计划 → `get_plan_output_columns` 取输出表头 → 空表头与输出列名重复拒绝 → 产出 CTAS 节点 |
| `src/parser/planner/mod.rs` | `build_plan` 的 `CreateTable` 臂 | 透传 `query` / `like` / `clone`（原 `..` 静默丢弃） |
| `src/parser/planner/query.rs` | `get_plan_output_columns` | CTAS 臂返回节点 `columns` |
| `src/parser/error.rs` | `PlanError::CtasColumnTypeConflict` | 新变体（`CTAS column type conflict: <col> (<A>, <B>)`）+ SQLSTATE 归类 58000 + 分类表测试行 |
| `src/pipeline.rs` | `extract_all_table_names` | `CreateTable { query: Some(q) }` 臂 → 提取源 SELECT 表名（CTE / JOIN / 子查询由既有 `extract_all_query_table_names` 承担） |
| `src/pipeline.rs` | `plan_stage_routed` | 新 CTAS 臂：写目标走 `mark_attached_ddl_targets`（附加库目标计划期拒绝），物化源走 `register_table`（跨库路由）；不进 plan cache（非 `Statement::Query`） |
| `src/pipeline.rs` | `execute_stage_routed` | 新 CTAS 混合臂 → `execute_ctas_routed`；其后 `plan_cache.clear()`（与 DDL 臂同款） |
| `src/pipeline.rs` | `execute_ctas_routed`（新） | 目标已存在守卫 → 物化 → 类型推导 → `create_table_with_constraints` → `buffer_pool.flush_all()` → 隐式事务 + `InsertExecutor::with_table_manager` → 失败 abort + 尽力 `drop_table` → `AffectedRows{count}` |
| `src/pipeline.rs` | `infer_ctas_column_types` / `ctas_value_type` / `ctas_type_name`（新） | 类型推导：全 NULL → `String(255)`；同型保留；`Int∪Float` → `Float`；其余混合 `CtasColumnTypeConflict`；行宽缺槽按 NULL 参与 |
| `src/pipeline.rs` | `execute_stage_in_tx_routed` | 新 CTAS 臂：显式事务内点名拒绝（`0A000`） |
| `src/pipeline.rs` | `create_executor_from_plan` | CTAS 臂（与既有 DDL 臂同型 panic 提示——CTAS 由混合臂消费，无单 executor） |
| `src/executor/correlated.rs` | `inject_correlated_values` | CTAS 臂递归源计划（无谓词/表达式自有面） |
| `src/executor/plan_format.rs` | `render_node` / `describe` | CTAS 行 + 源计划下探（穷尽性；见遗留 Minor 2） |
| `tests/ctas_test.rs`（新，17 例） | — | 9 场景矩阵 + 类型推导矩阵 + 输出列名重复 + 空源边界 + lib 路径扁平名零回归 |

崩溃一致性序由混合臂结构保证：`create_table_with_constraints` 成功 → `flush_all()` → 才进隐式事务落行（WAL 记录在 `InsertExecutor` 内），`flush_all` 失败时 abort + `drop_table` 收口。

### Deviations from Plan

1. **事务内拒绝的落点**：Plan Target 列 `src/cli/mod.rs::run_sql`（活跃事务内点名拒绝、会话态不变、同既有 `sql_failure_status` 收口）。实现落在 `execute_stage_in_tx_routed` 的 CTAS 臂，理由：① 该臂按构造即「会话事务活跃」，CLI 与 lib 两条在途路径共用同一文案；② `Response::Error` 经 `run_sql` 既有 `Response::Error` 臂收口（`sql_failure_status` + `rollback_session`），可观察行为与 Plan 要求逐项一致（exit 3、同 rollback 收口、拒绝点在 execute 之前）；③ lib `execute_in_tx` 路径无需在 `run_sql` 之外再补一处。差异为等价实现选择，非实质偏差。
2. **输出列名重复拒绝提前到计划期**：Plan 要求「建表前点名拒绝零副作用」。实现放在 `build_create_table_as_select`（计划期），零副作用更强、且早于物化。错误面点名不变。
3. **目标已存在守卫前置到物化之前**：Plan/design 序列在建表处拒绝；实现提前到函数入口。错误面同型（`StorageError::DuplicateTable` → `duplicate table: <name>`），既有表零变化，且不为注定失败的语句物化整个源结果集。
4. **新增 `PlanError::CtasColumnTypeConflict` 变体**：Plan 只钉错误文案（`CTAS column type conflict`），未钉变体归属；复用 `UnsupportedStatement` 会输出「Unsupported statement type: CTAS COLUMN TYPE CONFLICT」，语义错位，故新增变体并同步 SQLSTATE 分类表与其穷尽性测试。

### 自检发现与已修复内容

- Gate 3 逐文件核对 diff：确认既有非 CTAS `CREATE TABLE` 路径逐字节不变（`LIKE` / `CLONE` / `query` 三个守卫均以 `None` 通过），`mark_attached_ddl_targets` 拒绝面保持，`create_table_with_constraints` 无旁路（CTAS 目标已存在守卫复用同一错误面，不另建表路径），plan cache 对 CTAS 不写。
- 测试侧修正 4 处断言（非实现缺陷）：`m` 需先由普通 `CREATE TABLE` 播种才构成「目标已存在」；`schema` DDL 中首列带 `PRIMARY KEY` 修饰；`m_mix` 的 UNION 分支列名（`t` 无 `v` 列）；`WITH COLUMN DEFINITIONS` 文案大小写。
- clippy 报 `tests/ctas_test.rs` 未用 `tempdir` 导入 → 已删除，clippy 复跑干净。

### 遗留 Minor

1. **空结果集源的类型退化**：零行源无可观测值，全部输出列落入「全 NULL → `String(255)`」规则（零行是全 NULL 的退化形态），建表与空表形态正常。规格未单列零行场景；已用 `ctas_empty_source_creates_empty_string_typed_table` 把该行为固化为显式见证而非偶然形态。若后续要求零行沿用源列静态类型，属行为面变更。
2. **`EXPLAIN CREATE TABLE … AS`**：仍被既有「EXPLAIN of non-SELECT statements」计划期拒绝（零变化），`plan_format` 的 CTAS 两臂当前仅为穷尽性完整。若将来放开 EXPLAIN 覆盖 DDL，这两臂直接生效。

### 验证

| 命令 | 结果 | 退出码 |
|---|---|---|
| `cargo test --test ctas_test -- --test-threads=1` | `17 passed; 0 failed`（0.29s） | 0 |
| `cargo test --test planner_test -- --test-threads=1` | `90 passed; 0 failed` | 0 |
| `cargo test --test pipeline_test -- --test-threads=1` | `17 passed; 0 failed` | 0 |
| `cargo test --test cross_db_query_test -- --test-threads=1` | `17 passed; 0 failed` | 0 |
| `cargo test --test attach_session_test -- --test-threads=1` | `11 passed; 0 failed` | 0 |
| `cargo test --test cli_test -- --test-threads=1` | `111 passed; 0 failed; 2 ignored` | 0 |
| `cargo test --lib -- --test-threads=1` | `327 passed; 0 failed` | 0 |
| `cargo test --test parser_test -- --test-threads=1` | `6 passed; 0 failed` | 0 |
| `cargo test --test explain_test -- --test-threads=1` | `10 passed; 0 failed` | 0 |
| `cargo test --test plan_cache_test -- --test-threads=1` | `7 passed; 0 failed` | 0 |
| `cargo test --test tx_statement_test -- --test-threads=1` | `21 passed; 0 failed` | 0 |
| `cargo test --test explicit_tx_test -- --test-threads=1` | `14 passed; 0 failed` | 0 |
| `cargo fmt --check` | 无差异 | 0 |
| `cargo clippy --all-targets` | 无 error / warning | 0 |
| `openspec validate 2026-09-29-ms20-attach-cross-db --strict` | `Change '2026-09-29-ms20-attach-cross-db' is valid` | 0 |

TDD 见证：`tests/ctas_test.rs` 建立后先观察 RED（`15 failed; 1 passed`，决定性输出 `Plan error: Empty column definition in CREATE TABLE`），实施后转 GREEN（17 passed）。

### 未覆盖的验证

- **全量串行 Gate 未在本 Cycle 建立**：`cargo test -- --test-threads=1` 整轮未运行（约 85 个测试目标，本次按用户约束分 12 批串行执行覆盖上表 11 个目标 + `--lib`）。剩余约 70 个集成测试目标（`storage_test` / `wal_*` / `btree_*` / `concurrency` 系列等）未运行——它们不触达本 Cycle 变更面（`CreateTableAsSelect` 计划面、CTAS 混合臂、`PlanError` 新变体、`build_create_table` 签名），但本 Cycle 的全量零回归结论**未经真实运行**。收尾前需在可承受负载下补跑，或由 Plan Review 显式记录该豁免。

### Persisted Evidence

- None required（Plan Mode: none）。全部结论由上表命令低成本重跑可复现（目标批 ≤ 2.4s；`cli_test` 100s）。

### Experience Candidates

- None。本 Cycle 未产生当前 change 范围外的实质缺陷；既有 Runbook（in-process reopen testing、test bench diagnosis、file pair backup boundary）不因本次 CTAS 改动需要更新。

## Plan Review

- Review Result: accepted

**Findings**

Plan 独立核对了变更面与全部声明：对 Act 所列 11 个 src 文件 + 1 个测试文件逐一阅读当前代码（工作树含 MS18/MS19/MS25 未提交批次，git diff 无法单独圈定本轮；以 `grep -rn "CreateTableAsSelect|CtasColumnTypeConflict|ctas"` 全源码触点清单与 Act 改动表精确对应替代——8 文件触点无一多余），并独立核对 Act 未覆盖的全量串行 Gate（见 Evidence）。

代码独立阅读结论：混合臂执行序与 design D5 逐条成立——目标已存在守卫（`pipeline.rs:436`，物化前）→ 物化（语句级快照 + routed 取库）→ 类型推导（`infer_ctas_column_types:610`，建表前冲突零副作用）→ `create_table_with_constraints`（首列隐式 PK）→ `flush_all()`（`:531`，先于任何数据 WAL）→ 隐式事务 `InsertExecutor::with_table_manager` → 失败 abort（带 `abort_tables` 走 MS23/MS24 回滚还原通道）+ 尽力 `drop_table`、提交失败同收口；planner 侧诚实化拒绝臂齐备（`ddl_dml.rs:805/810/818/823`：LIKE/CLONE/列清单+query/表约束+query，均先经 `reject_attached_write`）；`plan_stage_routed` CTAS 臂（`pipeline.rs:186-201`）写目标 `mark_attached_ddl_targets` + 源经 `register_table`（`extract_all_table_names:1627` 消费 `extract_all_query_table_names`），不进 plan cache，执行后 `plan_cache.clear()` 与 DDL 同款；`execute_stage_in_tx_routed:746` CTAS 臂拒绝（`0A000`、先于 execute、CLI 与 lib 共用）；新变体 `CtasColumnTypeConflict`（`error.rs:23`）+ 58000 分类表行；`create_executor_from_plan:1096` CTAS 臂 panic 提示同 DDL 型；`correlated.rs:87` / `plan_format.rs:43,166` 穷尽臂与声明一致；`create_table_with_constraints` 签名未变（`table_manager.rs:315`）。非 CTAS 路径守卫穿透（`query`/`like`/`clone` 全 `None` 直通原流程）。测试 17 例映射 R1 全部 9 场景 + 类型推导矩阵 + 空源边界 + lib 扁平名，断言非空洞（重启读回经独立进程、隐式 PK 冲突与源失败均含同名重试见证、跨库一次性快照、附加库目标拒绝后附加库无残留）。

**全量零回归不变量已闭合**：Act 申报「全量串行 Gate 未运行」（约 70 个集成目标未触），属 Act 偏离——验证覆盖不足但已诚实披露并给出理由；由本 Review 补跑闭合（非 Acceptance gap，见 Evidence：1458 passed / 0 failed / 2 ignored，与对账预期 1441 + 17 精确一致）。

4 项 Deviations 逐一复核，均认可为等价或更强实现，非实质偏差：

1. 事务内拒绝落点 `execute_stage_in_tx_routed`——CLI/lib 共用同一拒绝面与文案，拒绝点先于 execute，零副作用，exit 3 同 Plan 目标；行为可观察面一致。
2. 输出列名重复拒绝提前到计划期（`build_create_table_as_select:954`）——零副作用更强、早于物化，错误面点名不变。
3. 目标已存在守卫前置物化——`DuplicateTable` 同型错误面、既有表零变化、免跑注定失败的物化。
4. 新增 `PlanError::CtasColumnTypeConflict`——复用 `UnsupportedStatement` 会产出语义错位文案，新变体 + SQLSTATE 归类同步，正确。

非阻塞 Minor findings：

1. **delta spec 场景「会话事务状态不变」措辞与可观测行为（Plan 措辞 + Act 描述不精确，行为正确）**：事务内 CTAS 拒绝经 `run_sql` 既有错误路径收口——`rollback_session` + exit 3（`cli/mod.rs:629-636`）。fail-fast 纪律下「事务保持活跃」不可达：任何语句错误（含边界子句拒绝，`cli/mod.rs:547-553`）同走收口回滚，既有语料 `sql-transaction-statements` R2 同型（「拒绝 + 收尾回滚 + 重开无残留」）。测试见证为「点名拒绝 + 表不存在 + 库仍可用」（`ctas_inside_explicit_transaction_rejected`），与可实现语义一致。Act Deviation 1 自述「与 Plan 要求逐项一致」对「会话事务状态不变」一项表述不精确——实为「与既有会话收口语义一致」。**交 docs-maintainer：合并 `create-table-as-select` 规格时把该场景 THEN 校准为「点名拒绝；拒绝零副作用，会话按既有错误收口回滚（`sql-transaction-statements` R2 同型），库状态无残留」**（BH 校准段先例）。附带备注：拒绝在 execute 期而非 plan 期，plan 错误（如源表缺失）会先于「事务内不支持」文案出现——同 exit 3、零副作用，无实质影响。
2. **Plan Context 不变量括注预测失准（Plan 遗漏，非实质）**：「CTAS 混合臂仅在 CLI 会话经路由上下文可达」与实现不符——lib `execute`（空路由表委托壳）同样可达混合臂，扁平名 CTAS 经 lib 可物化（`lib_path_ctas_two_part_name_keeps_flat_semantics` 见证），lib `execute_in_tx` 同样拒绝事务内 CTAS。行为为 spec 兼容超集（spec 未限定 CLI-only），lib 委托壳对既有计划零变化成立。Plan Context 冻结，留档本 Review；**交 docs-maintainer：合并规格时无需特别处理（spec 本就未限定入口），仅知悉**。
3. **零行源类型退化为显式见证（Act 遗留 Minor 1 复核接受）**：零行源全列落「全 NULL → String(255)」，`ctas_empty_source_creates_empty_string_typed_table` 已固化为显式见证；spec 推导规则授权 design 定稿，无 gap。**交 docs-maintainer：合并规格时在推导规则处补一句「零行源视同全 NULL 退化形态」**。

**Acceptance Gaps**

None——`create-table-as-select` R1 全部 9 场景有 GREEN 见证；崩溃一致性序（建表 → flush_all → 插入）由混合臂结构保证并经代码阅读确认；全量零回归以 Review 补跑新鲜运行闭合（1458 passed / 0 failed / 2 ignored）。

**Evidence**

- 代码独立阅读：`src/pipeline.rs:247-277`（execute_stage_routed CTAS 臂 + cache clear）、`:427-603`（execute_ctas_routed 全体：守卫/物化/推导/建表/flush/事务/收口）、`:610-667`（类型推导三函数）、`:186-201`（plan_stage_routed CTAS 臂）、`:746-755`（in-tx 拒绝臂）、`:1094-1097`（create_executor panic 臂）、`:1616-1627`（extract CTAS 源表）、`:1834`（mark_attached_ddl_targets）；`src/parser/planner/ddl_dml.rs:788-828,938-966`；`src/parser/planner/mod.rs:311-326`；`src/parser/planner/query.rs:137`；`src/parser/error.rs:23,84-86,188`；`src/executor/plan.rs:46,338-345`；`src/executor/correlated.rs:87-91`；`src/executor/plan_format.rs:43,166-170`；`src/storage/data/table_manager.rs:315-320`（签名不变）；`src/cli/mod.rs:547-553,588,629-636`（会话收口）。
- 测试独立阅读：`tests/ctas_test.rs` 全文（17 例 → 9 场景映射 + 类型矩阵 + 重试见证 + lib 扁平名）。
- **Review 补跑全量串行 Gate（Act 未覆盖项）**：`cargo test -- --test-threads=1` → 96 个测试目标，`1458 passed; 0 failed; 2 ignored`，exit 0（含 `ctas_test` 在内全目标；对账 1441 + 净增 17 = 1458 精确一致）。首跑因 Review 自身管道截断输出无效，重跑完整捕获——两次运行间代码零变化。
- 采信 Act Response Verification 表（来源：Iteration 002 Act Response，同一工作面，其后代码零变化）：ctas_test 17 passed / planner_test 90 / pipeline_test 17 / cross_db_query_test 17 / attach_session_test 11 / cli_test 111 / --lib 327 / fmt、clippy、`openspec validate --strict` 全 exit 0；TDD RED 见证（15 failed → GREEN 17 passed）按 GREEN 真实运行采信。

**Follow-up Decision**

接受（accepted）：Task Contract Acceptance 全部满足，3 项 Minor 均不触及契约语义（2 项为合并期规格措辞校准、1 项为显式见证的边界行为留档）。无当前 Cycle 修复项，无 Issue 候选（Act Experience Candidates None 复核成立）。Iteration Plan 不变；T4 为 change 最后任务，无剩余 Iteration。

**Next Cycle**

None

**Next Iteration**

None（change 三 Iteration 全部 accepted，待 docs-maintainer 收尾：合并 3 份增量规格〔含上述校准点〕、归档、刷新 SNAPSHOT/tasks）
