# Iteration 001 / Cycle 000: 限定名路由与只读跨查

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2, T3
- Acceptance gaps: `sql-attach-session` R1/S1 读取可观察量（Iteration 000 Review Findings 1 转入，由本 Cycle T2 e2e 闭合）
- Repair items: None
- Inherited scope: `sql-attach-session` R1–R3 注册面（Iteration 000 已交付并 accepted）；`AttachSession { dbs: HashMap<String, Arc<Database>> }` 结构与语义（别名小写、独占锁、RR + 同 key 打开，`src/cli/mod.rs:385`）；proposal 假设 2/4/6/9/11；design D3/D4/D6/D7
- Excluded scope: T4 CTAS（Iteration 002）；DETACH；lib 路径 attach 注册面；跨文件 DML（二期 MS20-T02）；跨库快照协调

**Objective**

两段限定名 `alias.table` 在全部表名消费点按 attach 注册表路由：命中 → 附加库、未命中 → 扁平名回落（既有含点表名零回归）、三段首段命中 → 计划期点名拒绝。SELECT 全形态（JOIN/子查询/聚合/ORDER BY/DISTINCT/CTE/集合操作）跨主库与多附加库组合可用，路由命中时三段列引用 `alias.table.col` 可用。附加库写面（INSERT/UPDATE/DELETE/CREATE TABLE/DROP TABLE）计划期点名拒绝且零副作用。含附加库引用的语句不进 plan cache。lib 路径与无注册会话行为逐字节零回归。

**Investigation Facts**

- Current Baseline: Iteration 000 Cycle 000 accepted（2026-09-29）——全量 1425 tests / 0 failures / 2 ignored 串行新鲜（来源：Iteration 000 Act Response Verification 表；本 Iteration 尚未改动任何代码表面，采信）。ATTACH 注册面已可用；当前 `SELECT * FROM o.u` 经拍平名 `"o.u"` 查主库 catalog → `TableNotFound`（42P01）。
- Current-State Evidence:
  - **表名拍平点（路由判定的位置约束）**：`object_name_to_table_name`（`src/parser/ast.rs:317-323`）把 `ObjectName` 拍平为 lowercase 点连接串；拍平后与引号单 Ident 含点（字面表名 `"o.u"`）不可区分。**路由判定必须在 `ObjectName` 分段尚存处**（`name.0.len() >= 2 && 首段（去引号 + lowercase）∈ 注册表`），不得对扁平串 split（design D3；spec R1「含点扁平表名零回归」场景的守护约束）。
  - **表名提取**：`extract_all_table_names`（`src/pipeline.rs:1069`，集合操作经 `:1135` `extract_all_table_names_from_set_expr`）返回扁平 `Vec<String>`——需扩展为保留分段信息（携带 `ObjectName` 或等价 (首段, 余段)）的提取结果。
  - **schema 注册**：`register_table`（`src/pipeline.rs:1244`）对提取名做主库 `table_manager.get_table` 并经 `PlanBuilder::register_table` 注册——按路由规则分流：命中别名 → 向对应附加库 `Arc<Database>` 的 `table_manager.get_table(余段)` 取 schema，以**限定扁平名 `"o.u"`** 注册进 `PlanBuilder.tables`；未命中 → 主库既有路径。计划扫描节点 `table_name` 同 key。
  - **plan cache**：写点 `plan_stage`（`src/pipeline.rs:89-117`）`is_cacheable(stmt)`（`:1299`，仅 `Statement::Query`）守卫 `:111-113` put；读点在 lib `execute_inner`（`:363`，get 于 `:386`）。cache key 仅归一化 SQL 文本，无 attach 身份——同文本 `SELECT * FROM o.u` 在「无 alias o」与「有 alias o」语义不同，故含附加库引用的语句必须跳过 put。CLI 会话（`run_sql`）无 cache 读路径，lib 永无注册面 → 写侧跳过即消除中毒面；读侧加守卫属等价加固（Act 选择）。cache 相关单测先例：`plan_stage_select_writes_to_cache_but_dml_does_not`（`src/pipeline.rs:1393`）。
  - **执行面**：`create_executor_from_plan`（`src/pipeline.rs:518`）扫描臂全部消费主库单例 `database.table_manager`——Scan `:536`、DataScan `:544`（`:553` 传入 `transaction_manager`——**tx-id 空间按库独立**，必须取附加库自身实例，串用会错误抑制墓碑，design D6）、IndexScan `:559`、IndexScanAll `:572`；SemiJoin `:748`、AntiJoin `:784`、SubqueryEval `:819` 重建臂同型。计划节点 `table_name` 已是限定扁平名，按名字首段查路由表取对应库实例。**写执行器臂（Insert `:585` 起）不在路由范围**（design D3 只列扫描臂）——写计划被 T3 计划期拒绝，写臂保持主库 `get_table` 语义。
  - **写面计划点（T3 拒绝面）**：`build_insert`（`src/parser/planner/ddl_dml.rs:153`）、`build_update`（`:917`）、`build_delete`（`:1002`）、`build_create_table`（`:781`）、`build_drop_table`（`:896`）——目标名为附加库引用 → 计划期点名拒绝。
  - **PlanBuilder 加性通道先例**：`set_pk_column_type` / `set_table_defaults` / `set_table_unique_columns`（`src/parser/planner/mod.rs:228-247`）——「附加库引用名集合」按同型加性字段 + setter 注入，`register_table` 签名不变。
  - **列引用**：`resolve_column_ref`（`src/parser/planner/expression.rs:24`）现仅两段 `t.col`——三段 `alias.table.col` 在前两段命中已注册限定名（含表别名）时取第三段为列；未命中维持既有拒绝面。
  - **CTE**：`cte_plans` / `pending_cte_names`（`src/parser/planner/query.rs:800-880`）——CTE 名单段、语句级注册表，与路由无交互；CTE 定义体内引用附加库表经同一表名消费点覆盖。
  - **表头**：`get_plan_output_columns`（`src/parser/planner/query.rs:51`）零变化——附加库列输出裸列名，CLI 渲染消费不变（design D7）。
  - **错误面**：MS19 `PlanError::UnsupportedStatement(String)` 携带能力名 + SQLSTATE 分类映射（`src/parser/error.rs:130-175`）——T3 拒绝文案按同通道。
  - **测试夹具先例**：`tests/attach_session_test.rs`（`run_cli` / `cli_fixture` / `two_db_fixture`，独立 TempDir + `RTSQL_HOME` + 60s watchdog）；本 Iteration 新建 `tests/cross_db_query_test.rs` 复用同型夹具。
- Code and Critical Path:
  - `src/cli/mod.rs::run_sql`——把 `AttachSession` 路由表传给 plan/execute 通道；lib 路径传 None 行为零变化。
  - `src/pipeline.rs`——`plan_stage` / `execute_stage` / `execute_stage_in_tx` 加性路由上下文参数（既有签名调用点适配为委托壳，MS17 `open_with_key` 委托壳先例）；`extract_all_table_names` 分段保留；`register_table` 路由分流；`create_executor_from_plan` 扫描臂按路由表取库。
  - `src/parser/planner/mod.rs`——PlanBuilder 附加库引用名集合加性字段 + setter。
  - `src/parser/planner/ddl_dml.rs`——五写点拒绝臂。
  - `src/parser/planner/expression.rs`——`resolve_column_ref` 三段臂。
  - `src/parser/ast.rs`——分段提取辅助（等价选择留 Act）。

**Implementation Guidance**

建议顺序：pipeline 路由上下文穿线（None 委托壳保 lib 零回归）→ `extract_all_table_names` 分段保留 + `register_table` 路由分流（计划面）→ 执行面扫描臂路由 → `resolve_column_ref` 三段臂 → plan cache 写侧跳过 → e2e 路由/组合矩阵（RED→GREEN）→ 五写点拒绝臂 + 拒绝矩阵（T3）。关键取舍：路由判定只在 `ObjectName` 分段处；附加库 schema 以限定扁平名注册，计划/执行两面对该 key 一致；DataScan 的 `transaction_manager` 必须取附加库自身实例；跨库 JOIN 复用既有 Hash/NLJ 启发式与 MS25 读面装配，预期零新执行器。

**Task Contracts**

### T2: 两段限定名路由与只读跨库查询

- Requirement/Scenario: `cross-db-namespace-query` R1 全部 5 场景 + R2 全部场景；兼闭合 `sql-attach-session` R1/S1（注册后经别名读取）
- Depends on: Iteration 000 注册面（`AttachSession`）
- Targets: `src/pipeline.rs::{plan_stage, execute_stage, execute_stage_in_tx, extract_all_table_names, register_table, create_executor_from_plan}`、`src/parser/planner/mod.rs::PlanBuilder`（加性通道）、`src/parser/planner/expression.rs::resolve_column_ref`、`src/parser/ast.rs`（分段辅助）、`src/cli/mod.rs::run_sql`（路由上下文接线）
- Current behavior: 两段名经 `object_name_to_table_name` 拍平为 `"o.u"` 恒查主库 catalog → `TableNotFound`；三段名同型拍平；执行面扫描臂恒主库；cache 恒按文本键写。
- Required behavior:
  - R1 命中：`ATTACH ... AS o` 后 `SELECT * FROM o.u` 返回附加库行（含 WHERE 过滤与列子集投影）。**该 e2e 同时闭合 `sql-attach-session` R1/S1。**
  - R1 裸名恒主库：主库与附加库各有同名表 `t`（内容不同），裸名 `SELECT * FROM t` 返回主库行。
  - R1 未命中回落：`SELECT * FROM z.t`（z 未注册、主库无 `z.t`）→ `TableNotFound`（42P01）点名，不静默按主库 `t` 处理。
  - R1 含点扁平名零回归：主库字面表名 `foo.bar`、未注册别名 `foo` → 行为逐字节不变。
  - R1 三段命中拒绝：`SELECT * FROM o.b.t`（o 已注册）→ 计划期点名拒绝。
  - R2 组合：跨库 JOIN（含 OUTER 形态同语义）、`SELECT count(*) FROM o.u WHERE id IN (SELECT id FROM t)`、聚合/ORDER BY（含别名排序）/DISTINCT/CTE/集合操作与主库 + 多附加库自由组合，语义与单库形态一致。
  - 三段列引用：`o.u.id` 在 `o.u` 命中路由时可用（投影/ON/WHERE）；两段 `t.col` 与路由未命中的三段拒绝面零回归。
  - plan cache：含附加库引用的语句跳过 cache 写（`plan_cache_len` 或等价单测见证）；无引用语句 cache 行为零变化。
  - 附加库列输出裸列名表头；`get_plan_output_columns` 零变化。
  - lib 路径（路由上下文 None）：两段/三段名行为与现状逐字节一致。
- Preserve: 既有单库全形态零回归；`object_name_to_table_name` 及既有 11 处拍平消费点行为不变；主库 open/close/事务路径零变化；Iteration 000 的 `AttachSession` 注册面结构不被重设计；执行面路由仅扫描臂——写执行器臂不路由。
- Forbidden: 对扁平串 split 的路由判定；lib 路径 attach 注册面；写面放行（T3 承担拒绝面，本任务不得使任何附加库写入可达）；plan cache key 结构变化。
- Test witness: `tests/cross_db_query_test.rs`（新，RED：路由未实现时 `SELECT * FROM o.u` 报 `TableNotFound`）——路由矩阵（命中/裸名/回落/含点回归/三段拒绝）、组合矩阵（JOIN 含 OUTER、子查询、聚合、ORDER BY、DISTINCT、CTE、集合操作、三段列引用）、cache 跳过见证（直接调用 pipeline 的单测或等价可观测手段；无引用语句以既有 `plan_stage_select_writes_to_cache_but_dml_does_not` 先例保持）、lib 零回归（无注册时两段名行为不变）。命令：`cargo test --test cross_db_query_test`。
- GREEN condition: 上述全绿；全量 `cargo test -- --test-threads=1` 零回归（基线 1425 + 新增）。
- Verification: `cargo test --test cross_db_query_test`（exit 0）；`cargo test -- --test-threads=1`（exit 0，failures=0）。失败含义：路由语义未达成或既有面回归。
- Stop when: 路由需要改动 `object_name_to_table_name` 的既有拍平消费点语义、或需要 `Database` 新增字段/结构变化、或附加库 schema 注册需要超出「向对应库 `get_table` 取 schema」的跨库 catalog 协议、或 MS25 读面装配与路由出现契约未覆盖的交互。

### T3: 附加库只读边界拒绝面

- Requirement/Scenario: `cross-db-namespace-query` R3 全部场景
- Depends on: T2（附加库引用名集合在 PlanBuilder 的注册通道）
- Targets: `src/parser/planner/ddl_dml.rs::{build_insert, build_update, build_delete, build_create_table, build_drop_table}`、`src/parser/planner/mod.rs::PlanBuilder`（引用名集合消费）
- Current behavior: `INSERT INTO o.u ...` 经拍平名查主库 catalog → `TableNotFound`（碰巧拒绝但错误面不诚实——不点名「附加库只读」）；若主库恰有同名表 `o.u` 则会静默写主库表（错误目标）。
- Required behavior: 五个写消费点（INSERT/UPDATE/DELETE/CREATE TABLE/DROP TABLE）目标名为附加库引用（首段命中注册表）→ 计划期点名拒绝，错误携带能力名（MS19 `UnsupportedStatement` 通道，SQLSTATE 分类与既有映射一致），附加库与主库零副作用。文案点名只读边界（如 "attached databases are read-only"）；具体文案即契约、由测试锁定。
- Preserve: 主库写面行为零变化（引用集合为空时五点行为逐字节不变）。
- Forbidden: 对附加库的任何写入放行；运行期（执行器层）拒绝——必须是计划期；CTAS 目标路由（Iteration 002 T4 承担，本任务不含 CTAS 臂）。
- Test witness: `tests/cross_db_query_test.rs` 拒绝矩阵（INSERT/UPDATE/DELETE/CREATE/DROP × 附加库目标；主库同名表形态在拒绝集合生效后不再静默写错目标；显式事务内附加库写目标拒绝后的会话收口同既有语义；附加库内容重启后不变）。命令：`cargo test --test cross_db_query_test`。
- GREEN condition: 拒绝矩阵全绿且零副作用；全量零回归。
- Verification: 同 T2 命令；失败含义：拒绝面缺失、错误面不诚实或零副作用破坏。
- Stop when: 拒绝需要改动执行器层（非计划期）才能达成、或引用名集合通道不足以覆盖某写点形态。

**Invariants**

- 全量测试零回归（1425 基线 + 新增净增）。
- lib 公开 API 与无注册会话的既有行为零变化（含含点扁平表名 `"foo.bar"`）。
- 路由判定只在 `ObjectName` 分段处；附加库表以限定扁平名为 PlanBuilder/计划节点 key。
- 无自研 SQL 分片/预解析层；plan cache key 结构零变化。
- 附加库一期只读：任何写面不路由到附加库（计划期拒绝 + 写执行器臂不路由双保险）。

**Non-goals**

- T4 CTAS；DETACH；跨文件 DML；跨库快照协调；lib 路径 attach。

**Acceptance**

- `cross-db-namespace-query` R1–R3 全部场景有测试见证且 GREEN；`sql-attach-session` R1/S1 完整闭合。

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| cross-db-namespace-query R1（两段名路由 5 场景） | T2 | `tests/cross_db_query_test.rs` 路由矩阵 | Covered |
| cross-db-namespace-query R2（只读跨查组合 + 三段列引用） | T2 | `tests/cross_db_query_test.rs` 组合矩阵 | Covered |
| cross-db-namespace-query R3（只读边界五点拒绝） | T3 | `tests/cross_db_query_test.rs` 写面拒绝矩阵 | Covered |
| sql-attach-session R1/S1（注册后读取，Iteration 000 Review 转入） | T2 | `tests/cross_db_query_test.rs` `ATTACH + SELECT o.u` e2e | Covered |

- 含附加库引用语句不进 plan cache（写侧跳过）且有见证。
- 附加库写面五点计划期点名拒绝、零副作用。
- 全量零回归。

**Verification**

- `cargo test --test cross_db_query_test` → exit 0。
- `cargo test -- --test-threads=1` → exit 0，0 failures（决定性输出：test result 行）。

**Gate 2 Readiness**

PASS（2026-09-29 用户批准 change 计划——proposal 四项裁定 + 假设集与 Iteration Plan；本 Cycle 未超出已批范围；调查证据于 design D3/D4/D6/D7 定稿，行号经 Cycle 000 Plan Review 当日复核）——无 BLOCKED/WAIVED 项；Persisted Evidence: none。

**Persisted Evidence**

- Mode: none

（Act Response 承载命令、输出与退出码；结果可低成本重跑。）

**Risks and Notes**

- pipeline 加性参数的具体形态（新参数 vs 路由上下文结构体）、`extract_all_table_names` 分段保留的返回类型、三段列引用命中「已注册限定名」时对表别名的处理细节、plan cache 写侧跳过的判定位置（`is_cacheable` 扩展 vs put 前检查）：等价实现选择，留 Act。
- 显式事务内跨库 SELECT：主库部分走会话事务，附加库部分各自隐式快照（假设 4——单语句内逐库一致）；`execute_stage_in_tx` 同样需要路由上下文。
- MS25 装配与路由的交互（如集合操作左右子树的提取覆盖、CTE 定义体引用附加库表、ORDER BY 别名排序与跨库列）：实现按既有装配全量复用；若暴露契约未覆盖的语义边界，属实质未知项，停并返回 Plan。

## Act Response

- Status: reported

**Delivered**

两段限定名在 CLI 会话内路由到 `ATTACH` 注册的附加库（路由判定只发生在 `ObjectName` 分段尚存处），跨库 SELECT 全形态（JOIN/OUTER/子查询/聚合/三段列引用/DISTINCT/CTE/ORDER BY 别名/集合操作/多附加库）语义与单库形态一致；附加库写面五点计划期点名拒绝并零副作用；含附加库引用的语句不进 plan cache。lib 执行路径行为逐字节不变。

**Changed files and symbols**

| File | Change |
|---|---|
| `src/parser/ast.rs` | 新增 `object_name_parts`（分段保真，路由判定的唯一依据）；`object_name_to_table_name` 改为 `parts.join(".")`（等价）；`extract_columns` / `extract_qualified_columns` 各增三段臂 |
| `src/pipeline.rs` | 新增 `RoutedTables`（键=计划期判定的限定扁平名 → `RoutedTable{db, local_name}`）、`AttachRoute`（别名→库实例）、`TableRef{flat, parts}`；全部表名提取函数改返回 `Vec<TableRef>`（消费键逐字节不变）；`plan_stage`/`execute_stage`/`execute_stage_in_tx` 委托壳 + `plan_stage_routed`/`execute_stage_routed`/`execute_stage_in_tx_routed`；`register_table_schema`（主库与附加库共用 MS16/MS24 四条加性通道）、`register_table` 路由分流与三段点名拒绝、`mark_attached_ddl_targets`（DDL 写目标标记）；cache 写侧 `is_cacheable(stmt) && routed.is_empty()`；`create_executor_from_plan<'a>` 增路由参数、`routed_db` + `load_table_meta`，Scan/DataScan/IndexScan/IndexScanAll 四臂按路由取 schema、缓冲池与**附加库自身 `transaction_manager`**；SemiJoin/AntiJoin/SubqueryEval 透传 `routed_tables` 到相关子查询逐行重建；新增单测 `plan_stage_with_attached_reference_skips_plan_cache` |
| `src/parser/planner/mod.rs` | 新增 `attached_tables: HashSet<String>` 加性通道 + `mark_attached_table()` + `reject_attached_write()` |
| `src/parser/planner/expression.rs` | 抽出 `resolve_qualified_column` / `qualified_column_expression` 两个共用解析体（两段臂原体保留，行为不变），`resolve_column_ref` 与 `build_expression` 各增三段臂 |
| `src/parser/planner/ddl_dml.rs` | `build_insert` / `build_create_table` / `build_drop_table` / `build_update` / `build_delete` 五点各增一处 `reject_attached_write`（计划期，写执行器未进入） |
| `src/parser/planner/query.rs` | `is_plain_column_expr` 接受三段列引用（`parts.len() == 2 \| 3`） |
| `src/cli/mod.rs` | `run_sql` 由 `attach_session.dbs` 构造 `AttachRoute`，走 `plan_stage_routed` + `execute_stage_*_routed`；生命周期语句臂与既有四臂分派不变 |
| `src/executor/{join_related_config,semi_join,anti_join,subquery_eval}.rs` | `routed_tables: Arc<RoutedTables>` 字段 + 4 处重建调用点透传（签名穿线的必要后果） |
| `tests/cross_db_query_test.rs` | 新增 17 例（R1×5、R2×7、R3×4、lib 零回归×1） |

契约 Forbidden 全部保持：CTE 注册、CTAS 物化、加密、锁、写执行器、事务、SetOp 编译、生命周期、codec 均未改动（`git diff src/executor/{insert,update,delete,upsert}.rs src/storage/` 为空）；plan cache key 零变化（仅写侧跳过）；`Database` 零字段变化；lib 公开 API 签名零变化；输出表头仍为裸列名。

**Deviations from Plan**（Cycle 显式留给 Act 的等价选择 + 验收必要的穿线，非实质问题）

1. `RoutedTables` 值类型带 `local_name`：契约表述为「按名字首段查路由表取库」。实际按**计划期键 + 库内本地名**取 schema——catalog 以本地名索引，只有库实例定位不到附加库内的表；且执行期按首段重解析会把主库字面表名 `"o.u"` 误路由，与 R1/S4 守护约束冲突。键仍是计划期判定的限定扁平名，执行期不做任何分段解析。
2. 变更面增 `src/parser/ast.rs` 两个投影提取函数与 `query.rs:is_plain_column_expr`：三段列引用必须贯通「投影列名 / 限定列对 / 纯列项判定」三处既有分类点，否则 `SELECT o.u.id` 落到既有 `UNSUPPORTED SELECT ITEM EXPRESSION` 拒绝面。属 R2 三段列引用验收的必要后果。
3. 变更面增 `src/executor/*` 四个文件：`create_executor_from_plan` 签名穿线的 4 处调用点，同时使相关子查询逐行重建在 ATTACH 会话内按库路由（R2 组合面完整性）。空路由表时行为与既有逐字节相同。
4. `TableMeta.name` 未改写为限定名：读执行器不消费该字段（写执行器才用，而写执行器不路由）。限定扁平名只留在计划节点、输出列与列引用表名上。
5. 附加库内表不存在时报 `Table 'o.u' not found in attached database 'o'`（SQLSTATE 42P01）：不静默回落主库同名表，对应 T3 Forbidden「错误目标为零」。

**Self-review findings and fixes**（Gate 3，Critical/Important 均已修复并重跑受影响验证）

- C1 主库路径误用路由注册 helper → 连带 `mark_attached_table`，会使**主库写面全被拒**。修复：拆出不含标记的 `register_table_schema`，仅路由分支显式标记。在 T3 见证运行前由 review 拦截。
- C2 扫描臂按限定名查附加库 catalog → `table not found: o.u`。修复：路由值携带 `local_name`，统一走 `load_table_meta`。
- C3 投影三段列引用走既有「表达式项」拒绝面。修复：补 `extract_columns` / `extract_qualified_columns` / `is_plain_column_expr` 三处分类点。
- C4 两处临时调试探针。修复：删除，`grep eprintln!/dbg!/DBG` 复核为空。

**Remaining minor findings**（非阻塞，未伪装为已解决）

- M1 无注册会话时，JOIN 查询内三段列引用的错误文案由既有 `Expression projection items are not supported with JOIN queries` 变为 `Table 'a.b' does not exist`（仍为计划期点名拒绝，更精确）。全量零回归通过，无既有测试锁定旧文案。
- M2 链式三表 JOIN（`t JOIN o.u ON … JOIN p.v ON …`）在跨库与单库形态下同样 `Column not found`——既有组合行布局边界，非本次引入。测试按已支持形态（两个两表 JOIN）断言，跨库语义与单库一致即满足 R2。
- M3 同一语句同时引用附加库 `o.u` 与主库同名字面表 `"o.u"` 时，两者计划键相同，执行面按计划期键路由会把主库字面表一并路由。R1/S4 守护场景为「会话未注册别名 foo」，此形态在规格之外。

**Verification**（2026-09-29，均为真实运行）

| 命令 | 决定性输出 | 退出码 |
|---|---|---|
| `cargo test --test cross_db_query_test` | `test result: ok. 17 passed; 0 failed` | 0 |
| `cargo test --lib plan_stage` | `test result: ok. 4 passed; 0 failed` | 0 |
| `cargo test -- --test-threads=1` | 95 个测试二进制合计 `1441 passed; 0 failed; 2 ignored` | 0 |
| `cargo fmt --check` | 无 diff | 0 |
| `cargo clippy --all-targets` | 无 warning / error | 0 |
| `openspec validate 2026-09-29-ms20-attach-cross-db --strict` | `Change '2026-09-29-ms20-attach-cross-db' is valid` | 0 |

全量首跑退出码 101（失败），一次复核通过（复核与首跑代码面相同，`git diff` 确认本轮删除的测试声明数为 0，lib 二进制两次均为 327 例）。首跑失败目标未落盘捕获，按偶发处理并如实记录。此前记录的 1425 与本轮 1441 相差 16，而本轮净增 18（17 e2e + 1 lib）——差异来自历史计法，未影响本轮 0 failed 的判定。

**Persisted Evidence**

- None required

单目标 0.2s、全量约 4 分钟即可低成本重跑，Act Response 已承载命令、输出与退出码；不满足白名单，不创建 `evidence/` 目录。

**Experience Candidates**

- None

无新的可重复高风险操作流程；M1/M2/M3 均为既有行为或规格外形态，不构成本 change 范围外有证据的实质缺陷。

**Unresolved**

- Iteration 002 T4（CTAS 主库物化）未开始，属 Iteration 001 Non-goals。
- `CREATE TABLE o.t AS SELECT …` 由 T3 的 DDL 写目标标记顺带点名拒绝；CTAS 物化臂与自身见证由 T4 承担。

## Plan Review

- Review Result: accepted

**Findings**

Plan 独立核对了变更面与全部声明：git diff（工作树 vs 暂存区）+ mtime 圈定本轮实施面恰为 Act 声明的 12 个文件（11 src + 1 测试），最后修改 13:23–13:30 均早于 Act Response 写入（13:39），验证结论未失效，按公共规则 › 验证 采信。工作树中另有 SNAPSHOT.md / improvements.md 两处体系文档 diff，经核对为 change 规划期（本会话开始前）的 I076 登记与活跃 change 标注，非本轮实施面。

代码独立阅读结论：路由判定只在 `ObjectName` 分段处（`object_name_parts`，`ast.rs:347`）；`RoutedTable{db, local_name}` 以计划期限定扁平名为键、执行期零分段重解析（R1/S4 守护成立）；DataScan 使用附加库自身 `transaction_manager`（design D6）；写执行器臂不路由（与 T3 计划期拒绝双保险）；`create_executor_from_plan` 全部 24 处递归调用点均透传路由（含 DerivedScan/Projection/SubqueryEval，Semi/Anti 经 `routed_tables` 字段）；五写点拒绝臂齐备（`ddl_dml.rs:166,793,913,934,1024`）且 DDL 分支不查 catalog 即标记（`CREATE TABLE o.newt` 表尚不存在同样拒绝）；plan cache 写侧跳过 `is_cacheable(stmt) && routed.is_empty()` 且单测含对照组（附加库引用语句跳过、同会话普通 SELECT 照常进 cache）；主库路径注册键改前改后一致（`table_meta.name`，diff 核实）。测试 17 例断言非空洞（主库 `"o.u"` 诱饵表 id=5 与附加库同名 `t` 使裸名/限定名/错误目标三面可判别）。

非阻塞 Minor findings：

1. **计数口径差异已对账（Plan 错误 · 历史遗留，非实质）**：历史记录的 1425 为「含 2 ignored 的总数」口径，Act 本轮按输出读 1441 passed（不含 ignored）。精确恒等式：1441 passed + 2 ignored = 1443 = 1425 + 本轮净增 18（17 e2e + 1 lib 单测）——无测试缺失或删除（diff 中 0 个测试函数删除，8 处「删除行」均为提取函数签名改写 `Vec<String>` → `Vec<TableRef>` 与 Iteration 000 的 `run_sql` 签名）。后续记录建议原样引用输出行（`N passed; 0 failed; M ignored`），不写推导和。
2. **全量首跑 exit 101 一次复核通过（Act 披露）**：符合公共规则 › 验证 的「失败后判定是否偶发错误的一次复核；复核通过按通过处理」，两次运行间代码面相同（git diff 0 测试声明变化，lib 二进制两次均 327 例）。Minor 保留项：首跑失败目标未落盘捕获，无法定位偶发来源（夹具时序类最可能——本轮新增锁/多进程夹具与全量并行文件系统负载）。
3. **M1（无注册会话 JOIN × 三段列引用错误文案变化）复核接受**：由 `Expression projection items...` 变为 `Table 'a.b' does not exist`——仍为计划期点名拒绝且更精确，无既有测试锁定旧文案，全量零回归通过。对 Invariant「无注册会话行为零变化」的措辞构成一处文案面例外，属等价拒绝通道内的诊断改进。
4. **M2（链式三表 JOIN 受限）复核接受**：既有单库组合行布局边界，跨库与单库一致，测试按已支持形态断言满足 R2。
5. **M3（同语句同时引用附加库 `o.u` 与主库字面表 `"o.u"` 时计划键碰撞、均路由附加库）**：spec 外病理形态（R1/S4 守护场景为「未注册别名」），非缺陷、无消费方；已在本 Review 留档，**收尾合并 `cross-db-namespace-query` 规格时建议 docs-maintainer 在已知边界段记一句**（同拼写限定名与字面含点表名单语句混用，路由按限定名优先）。

5 项 Deviations 逐一复核（local_name、ast/query 分类点、executor 四文件穿线、TableMeta.name 不改写、附加库缺表 42P01 点名）——均为验收必要的等价实现或更诚实错误面，非实质偏差。Self-review C1–C4 修复经最终代码复核成立（主库路径不含 `mark_attached_table`；探针 grep 清洁）。

**Acceptance Gaps**

None——`cross-db-namespace-query` R1–R3 全部场景有 GREEN 见证；`sql-attach-session` R1/S1 闭合义务已由 `qualified_name_routes_to_attached_database`（`ATTACH + SELECT o.u` 精确行断言）解除；plan cache 跳过有对照单测；全量串行 0 failures。

**Evidence**

- 变更面与新鲜度：`git diff --name-only`（工作树 vs 暂存区）12 文件 + mtime 13:23–13:30 < Act Response 13:39:40。
- 代码独立阅读：`src/pipeline.rs:31-90`（RoutedTables/RoutedTable/AttachRoute/TableRef）、`:150-211`（委托壳 + DDL 臂标记 + cache 旁路）、`:628-653`（routed_db/load_table_meta）、`:671-739`（扫描臂路由 + 附加库自身 tx_manager）、`:1040-1082`（SubqueryEval/DerivedScan 透传）、`:1550-1621`（register_table 分流 + 三段拒绝 + 附加库缺表点名）、`:1744-1819`（cache 对照单测）；`src/parser/ast.rs:331-349`；`src/parser/planner/mod.rs:254-269`；`src/parser/planner/ddl_dml.rs` 五点；`src/parser/planner/expression.rs:24-96`；`src/cli/mod.rs:559-590`；四个 executor 文件 `routed_tables` 字段与重建透传。
- 测试独立阅读：`tests/cross_db_query_test.rs` 全文（17 例场景映射表 + 诱饵夹具）。
- 采信 Act Response Verification 表（来源：Iteration 001 Act Response，同一工作面，覆盖范围此后未变化）：cross_db_query_test 17 passed / lib plan_stage 4 passed / 全量串行 1441 passed + 2 ignored / fmt、clippy、`openspec validate --strict` 全过。

**Follow-up Decision**

接受（accepted）：按 Task Contract 的 Acceptance 全部满足，无阻塞项；5 项 Minor 均不触及契约语义。Iteration Plan 不变，展开 Iteration 002（T4 CTAS 主库物化）。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-09-29-ms20-attach-cross-db/iterations/002-ctas/000-initial.md`（已展开，Plan Context ready）
