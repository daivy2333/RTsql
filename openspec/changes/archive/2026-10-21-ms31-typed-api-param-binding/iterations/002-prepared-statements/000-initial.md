# Iteration 002 / Cycle 000: 可复用预编译语句

## Plan Context

- Status: ready（Gate 2 PASS，2026-10-22 用户批准「批准」）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T9, T10, T11（change `tasks.md` Iteration 002）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal What Changes 2 后半（MS31-T02 预编译面）+ delta spec `sql-parameter-binding` R2 全部四场景 + design D7/D8；Iteration 001 的参数通道（`assign_placeholder_indices` → 个数校验 → plan → `materialize_params` → execute）与 `SqlOutcome`/`SqlError` 形态
- Excluded scope: PG Extended Query 协议（I071 触发锁定维持）、plan cache key 语义重构、prepare 的 attach 路由面特殊化（lib 面与 execute_sql 一致即可）、流式游标/背压（I065）、crate 边界拆分（I074）

**Objective**

prepare 一次、多次带参执行的可复用预编译语句成立：同一语句多次执行结果与每次直接执行一致；schema 变化（DDL 八臂 + CTAS）后透明 re-plan、结果与直接执行一致；显式事务内执行；`&self` 并发安全（每次执行计划私有副本）；版本戳与缓存失效同源单点；无 prepare 语句路径零变化；全量零回归。

**Background**

MS31-T02 后半（I089 promoted）。依赖 Iteration 001 的参数通道（accepted，2026-10-22）。

**Investigation Facts**

- Current Baseline: Iteration 001 accepted 终态（工作区未提交，HEAD `7362b35` 之上叠加）：参数化通道 `Database::execute_with_params`/`execute_in_tx_with_params` 落地，全量 2109/0/2（来源：Iteration 001 最终 Act Response + accepted Review 采信）
- Current-State Evidence（2026-10-22 核对）:
  - `Database` 结构（`src/database.rs:102-121`）：`#[derive(Clone)]`、全部字段为 `Arc`/枚举（buffer_pool/table_manager/transaction_manager/wal_*/plan_cache/checkpoint_manager/views）——`PreparedStatement` 持 `Database` 克隆与现 `Arc<Database>` 消费者同型，close 由调用者编排（design D7 前提成立）
  - 缓存失效站点（生产路径全部清单，grep 核对）：`src/pipeline.rs::execute_stage_routed` 6 处（:371 CreateTable/DropTable、:390 CreateIndex/DropIndex、:401 AlterTable、:412 Truncate、:431 CreateView/DropView、:438 CTAS）+ `execute_stage_in_tx_routed` 1 处（:1169）；`plan_stage_routed` put 门在 :306-312（`is_cacheable && routed.is_empty() && count_placeholders == 0`）；测试模块另有 2 处非生产调用
  - 参数通道复用面：`pipeline::execute_inner(database, sql, params)` 内联于 `execute_with_params`；编号（`assign_placeholder_indices`）/个数校验/`materialize_params`（槽位 + DO UPDATE 改写 + `inject_correlated_values`）均为可独立调用的既有函数；`PhysicalPlan: Clone`
  - `execute_in_tx_with_params`（`src/pipeline.rs:1038`）：显式事务通道，DML 消费调用方 `tx_id`
  - 快照：`execute_stage[_in_tx]` 均接 `with_snapshot` 家族（RR/RC 语句视图）——prepared 执行复用 execute_stage 即自动获得
- Code and Critical Path: T9 `src/database.rs`（字段 + 读取器）+ `src/pipeline.rs`（`bump_schema_version` helper 收敛 7 站点）；T10 `src/database.rs`（`prepare` 家族 + `PreparedStatement` 结构与 execute/execute_in_tx）+ 复用 `pipeline` 参数通道件；T11 `tests/prepared_statement_test.rs` + 全量。数据流：prepare（parse+编号+计数+plan 一次，版本戳快照）→ execute（版本比对 → 命中 `plan.clone()` / 不匹配重跑 parse+plan 更新自持 → `materialize_params` → `execute_stage[_in_tx]`）

**Implementation Guidance**

- 建议顺序 T9 → T10 → T11（版本戳是失效契约的单点基础；T10 在其上建 prepare；T11 矩阵 + 全量）。
- `bump_schema_version(db)` = `plan_cache.clear()` + `schema_version.fetch_add(1)` 同点；替换 7 个生产站点时保持既有调用时序逐字节（成功后与失败后都清的语义——:371 附近注释明示）。
- 版本不匹配 re-plan 须完整重跑 parse+编号+计数+plan（与 `execute_with_params` 同序），不能只重跑 plan（语句文本对应的 AST/编号可能因 schema 变化走不同计划路径）。
- 「计划不经每次执行重建」的复用语义以行为断言（多次执行结果正确）+ 结构断言（prepare/执行后 `plan_cache_len` 不变、非 re-plan 执行不触 plan 函数）承载——不引入计时观测或判定层。

**Task Contracts**

### T9: schema 版本戳单点（可验证结果：版本戳与缓存失效同源）

- Requirement/Scenario: sql-parameter-binding R2「schema 变化后失效契约」（基础机制）
- Depends on: None
- Targets: `src/database.rs`（`schema_version: Arc<AtomicU64>` 字段〔open 家族全部构造点初始化为 0〕+ `pub fn schema_version(&self) -> u64` 读取器）、`src/pipeline.rs`（`bump_schema_version(database: &Database)` helper——`clear` + `fetch_add` 同点；替换 7 个生产 `plan_cache.clear()` 站点：`execute_stage_routed` ×6〔:371/:390/:401/:412/:431/:438〕+ `execute_stage_in_tx_routed` ×1〔:1169〕）
- Current behavior: 各 DDL/CTAS 执行臂裸调 `database.plan_cache.clear()`（7 站点）；无版本概念
- Required behavior: 全部生产站点经单点 helper 收敛——版本只增不减；DDL 执行（真实入口 `execute_sql("CREATE TABLE …")` 等）后 `schema_version()` 递增且 `plan_cache_len` 归零；无 DDL 语句执行版本零变化；同一 Database 克隆间版本共享（Arc 语义——prepare 持有的克隆能观测 bump）
- Preserve: 缓存失效时序逐字节（各臂成功/失败后清的既有次序）；`plan_cache` 容量/key/put-get 语义；测试模块 2 处 clear 调用形态不强制迁移（非生产路径）
- Forbidden: prepare/PreparedStatement（T10）；plan cache 数据结构变更；版本持久化（内存语义，open 重置为 0——prepared 不跨 open 存活）
- First test: `tests/prepared_statement_test.rs`（新文件）首批——DDL 执行后版本递增 + 缓存归零；SELECT/INSERT 执行版本不变。RED：`schema_version` 读取器不存在（编译失败）
- GREEN condition: 上述测试过；`cargo test --lib` 零回归
- Verification: `cargo test --test prepared_statement_test`（定向）+ `cargo test --lib`
- Stop when: 发现 clear 站点在 helper 无法同点覆盖的路径（如 drop_table 内部深处再清）——记录后回 Plan 定站点面

### T10: prepare/PreparedStatement（可验证结果：预编译复用与失效契约成立）

- Requirement/Scenario: sql-parameter-binding R2「一次 prepare 多次执行」「schema 变化后失效契约」「显式事务内执行」「并发共享安全」
- Depends on: T9
- Targets: `src/database.rs`（`pub async fn prepare(&self, sql: &str) -> Result<PreparedStatement, SqlError>` + `PreparedStatement { db: Database, sql: String, plan: PhysicalPlan, param_count: usize, schema_version: u64 }` + `pub async fn execute(&self, params: &[Value]) -> Result<SqlOutcome, SqlError>` + `pub async fn execute_in_tx(&self, params: &[Value], tx: &Transaction)`）；复用 `pipeline` 既有件（`assign_placeholder_indices`/`count` 校验面/`materialize_params`/`execute_stage[_in_tx]`）
- Current behavior: 无 prepare 面；每次 `execute_with_params` 全程 parse+plan
- Required behavior: prepare 一次（parse → 单语句检查 → 编号 → 计数 → plan，版本戳快照）；`execute(&self, params)`——版本匹配 → `plan.clone()` → `materialize_params`（私有副本）→ `execute_stage`；版本不匹配 → 透明 re-plan（完整重跑 parse+编号+计数+plan，更新自持计划与版本戳，结果与直接执行一致）；`execute_in_tx` 同款走 `execute_stage_in_tx`（事务语义与既有 `execute_in_tx_with_params` 一致）；每次执行操作计划私有副本（`&self` 并发安全，无共享可变状态——`Mutex`/内部可变仅限 re-plan 自持更新且不得与其余并发执行交叠错误计划）；prepared 不读写 `plan_cache`；prepare 语句面与 `execute_sql` 一致（单语句、多语句拒绝、attach 路由上下文不特殊化）；DDL 可 prepare、执行触发 bump → 后续执行透明 re-plan（幂等）；参数个数错误在 execute 期点名（与 Iteration 001 同文案同 SQLSTATE）
- Preserve: `execute_with_params`/`execute_in_tx_with_params` 语义零变化（通道复用或同型实现）；错误面同源（文案/SQLSTATE/触发点）；快照语义（prepared 执行复用 execute_stage 即得）
- Forbidden: plan cache key 语义重构；PG Extended Query 协议；字符串替换式绑定；执行器改动
- First test: `tests/prepared_statement_test.rs` 先行——①一次 prepare 两次不同参数执行（INSERT 两条 + SELECT 读回各自正确，与直接执行结果一致）；②schema 变化后（ALTER ADD COLUMN/DROP TABLE/建删索引各至少一形态）透明 re-plan 且结果与直接执行一致；③显式事务内参数化 DML 提交/回滚语义正确；④同一 prepared 经 `tokio::join` 多任务并发不同参数执行结果正确；⑤prepare 后 `plan_cache_len` 不变（结构断言）。RED：`prepare` 不存在（编译失败）
- GREEN condition: 上述测试过；`cargo test --lib` + `--test prepared_statement_test` 零回归
- Verification: 同上定向命令
- Stop when: re-plan 需要路由上下文（attach 语句 prepare 面与 execute_sql 出现语义分叉）或计划私有副本对某节点形态不可 Clone——回 Plan

### T11: 失效/并发/事务测试矩阵 + 全量分批回归（M19）预算点（纯验证任务）

- Requirement/Scenario: R2 四场景矩阵收口 + 全量零回归
- Depends on: T9, T10
- Targets: 无代码变更（矩阵断言按需补入 `tests/prepared_statement_test.rs`）
- Current behavior: N/A
- Required behavior: M19 分批全量回归（字典序分组、每批 ≤10-12 目标、批内 `--test-threads=1`、批间串行、限并发编译 `-j 2`、内存守卫——Iteration 000/001 实测定型纪律）全部 0 failed；验证记录注明分批方式与各批计数
- Preserve: N/A
- Forbidden: 为通过回归修改既有断言（发现失败按 T9/T10 契约修复后重跑受影响批）
- First test: 不适用（纯验证任务；SKIPPED: 验证任务无先行测试）
- GREEN condition: 全部批次各自 0 failed（缺一批不得声明全量通过）
- Verification: 分批 `cargo test --test <name> -- --test-threads=1` 序列 + `cargo test --lib`
- Stop when: 任一批 failed——契约内定位修复重跑该批，契约外回 Plan

**Invariants**

- 无 prepare 语句路径全链行为逐字节不变（含 `execute_with_params` 家族）。
- 缓存失效时序逐字节（7 站点 helper 化不改调用次序）；plan cache key/容量/put-get 语义零变化。
- `render.rs`、`pg_protocol.rs`、JSON 序列化零改动；执行器零改动（本 Iteration 目标面为 database/pipeline 装配层）。
- 错误文案与 SQLSTATE 分类同源（新面复用既有错误形态，不造第二套文案）。
- 版本戳为内存语义（open 重置），不持久化、不进 WAL/catalog。

**Non-goals**

- PG Extended Query 协议（I071）；prepare 的 attach 路由面特殊化；流式游标/背压（I065）；crate 边界拆分（I074）。
- 「计划不经每次执行重建」的计时级证明（以行为+结构断言承载）。

**Acceptance**

| Requirement | Scenario | 验证载体 |
|---|---|---|
| sql-parameter-binding R2 | 一次 prepare 多次执行 | T10 先行测试①（真实入口 `Database::prepare`） |
| 同上 | schema 变化后失效契约 | T9 版本戳单点测试 + T10 先行测试②（透明 re-plan） |
| 同上 | 显式事务内执行 | T10 先行测试③ |
| 同上 | 并发共享安全 | T10 先行测试④ |
| 全量零回归 | — | T11 M19 分批 |

**Verification**

- 行为类 scenario 全部走真实入口（`Database::prepare` + `PreparedStatement::execute[_in_tx]`），先行测试即集成测试。
- 全量分批回归（M19）在 T11 预算一次；批内失败按契约修复后重跑受影响批。

**Gate 2 Readiness**

PASS（RTM 无 Missing/Simplified；D7/D8 设计闭合；`Database` Clone/全 Arc 前提、7 个 clear 站点、参数通道复用面均经 2026-10-22 实际代码核对；Persisted Evidence = none 满足白名单）。Status 维持 draft，等待用户批准后转 ready。

**Persisted Evidence**

- Mode: none

全部验证为 `cargo test` 集合内行为断言，可低成本重跑；无一次性环境与现场保留需求；Act Response 承载命令与结果足够。

**Risks and Notes**

- re-plan 的完整性（须重跑 parse 而非仅 plan）是失效契约正确性前提（Implementation Guidance 已记）；Act 若发现可仅重跑 plan 的形态，按实际面记录后回 Plan。
- prepared 与 `plan_cache` 的互不读写以结构断言锁定（`plan_cache_len` 不变），不做侵入式观测。
- 并发安全测试用 `tokio::join` 定向并发形态（单测内可控），不引入压力测试框架。

## Act Response

- Status: reported
- Cycle: Iteration 002 / Cycle `000-initial`（initial）
- Tasks: T9, T10, T11 —— 全部完成（Gate 3 逐任务 Review 通过、Gate 4 验证满足）
- Date: 2026-10-23

### 实际改动

**T9 — schema 版本戳单点**

| 文件 | 改动 |
|---|---|
| `src/database.rs` | `Database.schema_version: Arc<AtomicU64>` 字段（open 家族唯一构造点 `open_internal` 初始化为 0——内存语义，不持久化、不进 WAL/catalog）+ `pub fn schema_version(&self) -> u64` 读取器 |
| `src/pipeline.rs` | `bump_schema_version(database)` helper（`plan_cache.clear()` + `fetch_add(1)` 同点）+ 7 处生产 `plan_cache.clear()` 收敛到该 helper（`execute_stage_routed` ×6：CreateTable/DropTable、CreateIndex/DropIndex、AlterTable、Truncate、CreateView/DropView、CTAS；`execute_stage_in_tx_routed` ×1）。各臂「成功后与失败后都清」的既有时序逐字节保持（bump 在 executor 返回之后，与原 clear 同一位置）。测试模块 2 处 clear 调用形态未迁移（非生产路径，按契约） |

**T10 — prepare / PreparedStatement**

| 文件 | 改动 |
|---|---|
| `src/database.rs` | `Database::prepare(&self, sql) -> Result<PreparedStatement, SqlError>`（parse → 单语句 → 编号 → 计数 → plan → 版本戳快照）；`PreparedStatement`（`db`/`sql`/`state`/`exec_gate`）+ `PreparedState`（`plan`/`param_count`/`schema_version`）+ `Debug` 实现；`execute(&self, params)` / `execute_in_tx(&self, params, tx)`；`resolve_plan`（版本比对 → 复用自持计划 or 透明 re-plan）；`check_param_count`（个数校验复用参数化通道文案与 SQLSTATE） |
| `src/pipeline.rs` | `plan_stage_impl`（`cache_put` 开关，既有路径恒 `true`）+ `plan_stage_uncached`（预编译面：不写 plan cache）；`plan_prepared`（parse → 单语句 → 编号 → 计数 → plan，返回 `(计划, 占位符个数)`）；`materialize_params` / `parameter_count_mismatch` 可见性放宽至 `pub(crate)`；`multi_statement_rejected` 返回类型由 `Result<SqlOutcome, SqlError>` 改为 `SqlError`（文案逐字节不变，2 处调用点同步为 `Err(...)`） |
| `tests/prepared_statement_test.rs`（新，14 条） | T9 版本戳 3 条 + T10 复用/失效/事务/并发/缓存/错误面 11 条（真实入口 `Database::prepare` + `PreparedStatement::execute[_in_tx]`） |

**T11 — 矩阵与全量**：无产品代码变更；矩阵断言落在 `tests/prepared_statement_test.rs`（见上），全量分批回归见「验证」段。

### Deviations from Plan（偏差与原因）

1. **并发机制由「每次执行操作计划私有副本」改为「每次执行持有计划副本 + 同一条预编译语句的并发执行经闸门串行化」**（触及 design D7 与 Iteration 002 Stable baseline 的机制描述；见「需 Plan Review 裁定的设计回折」）。原因：契约前提「`plan.clone()` 得私有副本」不成立——`ExpressionRef = Arc<dyn Expression>`（`src/executor/predicate.rs:76`）、`PredicateRef = Arc<dyn Predicate>`（`:47`），`PhysicalPlan` 的 `Clone`（`src/executor/plan.rs:22-23`）对表达式/谓词子树是**浅克隆**；`ParameterExpression { param_name: String, value: Mutex<Value> }`（`src/executor/predicate.rs:427-430`）的参数值是共享可变状态。两条备选均被契约自身排除：①每次执行重新 plan → 违反 spec 场景 THEN「计划不经每次执行重建」；②执行器侧深拷贝能力（`Expression`/`Predicate` trait 增克隆重建方法）→ T10 Forbidden 明列「执行器改动」，且本 Iteration 目标面为 database/pipeline 装配层。故取同语句串行化：闸门覆盖「参数注入 + 执行」全段，注入与执行不交错，参数值不串扰。
   - **既有同型先例**：执行期相关子查询注入走 `plan.clone()` + 就地注入（`src/executor/semi_join.rs:243`、`src/executor/subquery_eval.rs:189`、`src/executor/scalar_subquery_filter.rs:170`）——项目既有模型即「计划树参数节点为就地改写的暂态状态」，本偏差与该模型一致，非新引入的并发模型。
   - **可观察行为不变**：并发执行结果正确（T10 先行测试④ 见证，含写面值位与读面谓词位两种形态）；不同预编译语句各有闸门、互不阻塞。
   - **已考虑并否决的备选**：仅对含表达式位参数的语句加闸门（DML 值位的 `plan.clone()` 本已深拷贝 `InsertNode.values`）——需在 parser 面新增「占位符位置分类」接口，误分类即静默串扰，收益不抵风险。
2. **`PreparedStatement` 字段形态**：契约与 D7 列举 `{ db, sql, plan, param_count, schema_version }`；实现为 `db`/`sql` + `state: Mutex<PreparedState { plan, param_count, schema_version }>` + `exec_gate`。原因：`&self` 下 re-plan 须更新自持计划，字段须置于锁内（契约允许「`Mutex`/内部可变仅限 re-plan 自持更新」）；`exec_gate` 为偏差 1 的并发闸门。
3. **版本戳快照取「构建计划之前」的读数**（保守）：读数早于计划所见的 schema 时最多多一次 re-plan；读数晚于计划所见的 schema 会把「按旧 schema 建的计划」标成新版本 → 陈旧计划被复用。prepare 与 re-plan 两处同此口径。
4. **`multi_statement_rejected` 返回类型收敛**：预编译计划面需复用同一错误构造，避免第二套文案（契约「不造第二套文案」）；错误文案与 SQLSTATE 逐字节不变。
5. **`plan_stage` 拆为 `plan_stage_impl` + `plan_stage_uncached`**：`plan_stage` / `plan_stage_routed` 签名不变（既有调用面零改动），内部加 `cache_put` 开关，既有路径恒 `true` → plan cache 的 put 门、key、容量、get 语义逐字节不变。

### 自检发现与修复（Gate 3）

| # | 级别 | 发现 | 处置 |
|---|---|---|---|
| 1 | Important | 契约前提「`plan.clone()` 得计划私有副本」不成立——表达式/谓词子树浅克隆共享，参数节点值是共享可变状态 → 并发执行同一预编译语句会参数串扰 | 已按契约约束修复（偏差 1）：同语句并发执行经闸门串行化（注入与执行不交错）；`t10_prepared_concurrent_execution_with_different_params` 见证（写面值位 + 读面谓词位并发） |
| 2 | Important | 自持计划的版本戳若在 re-plan **之后**读取，可能把「按旧 schema 建的计划」标成新版本 → 陈旧计划被当作有效复用 | 已修复：版本戳取 re-plan 之前的读数（保守口径，多一次 re-plan 而非漏判失效） |
| 3 | Minor | 先行测试首版对 INSERT 执行断言 `Rows`（实为 `AffectedRows`）——测试断言错误，非产品缺陷 | 已修复：改为断言受影响行数 + 读回逐条一致 |
| 4 | Minor | 先行测试 `tokio::join!` 中临时数组借用（E0716 编译失败） | 已修复：参数切片绑定到局部变量 |
| 5 | Minor | `PreparedStatement` 无 `Debug`（`unwrap_err()` 要求 `T: Debug`，E0277） | 已修复：加 `Debug` 实现（只输出语句文本，不加锁） |
| 6 | Minor | `cargo fmt --check` 三个文件非零 diff | 已修复：`cargo fmt`（仅本 Cycle 三个文件） |
| — | 遗留 Minor | 无。 | — |

计划范围外修改：无（`git diff --stat -- src/` 的改动文件全部属本 change 既有面；Iteration 002 新增改动仅 `src/database.rs`、`src/pipeline.rs`、`tests/prepared_statement_test.rs`）。

### 验证（全部真实运行）

- **T9/T10 定向**：`cargo test --test prepared_statement_test -- --test-threads=1` → **14 passed / 0 failed**（版本戳递增与缓存归零、非 DDL 版本不变、克隆间版本共享、一次 prepare 多次执行、DDL 后透明 re-plan（ALTER / DROP / 建删索引四形态）、显式事务提交与回滚、并发带参执行、prepare 不触缓存、个数错误文案与 SQLSTATE 同源、多语句与解析错误同面、DDL 可 prepare）。
- **库单测**：`cargo test --lib -- --test-threads=1` → **414 passed / 0 failed / 0 ignored**。
- **参数化通道面**：`cargo test --test param_binding_test -- --test-threads=1` → **28 passed / 0 failed**。
- **类型化结果面**：`cargo test --test typed_api_test -- --test-threads=1` → **17 passed / 0 failed**。
- **构建 / 静态面**：`cargo build --lib --tests --bins -j 2` → 零错误零警告；`cargo check --benches --examples -j 2` → 零错误零警告；`cargo fmt --check` → 全仓零 diff。
- **全量（M19 分批，串行）**：前置 `cargo build --lib --tests --bins -j 2`（限并发编译）→ **13 批 / 130 目标**（`--lib` + `--bins` + 128 个集成测试文件按**字典序**分组，每批 ≤10 目标），批内 `cargo test --test <name> -- --test-threads=1`、**批间串行**、每批前可用内存守卫（本轮实测 3995–5015 MiB，未触发）。分批计数：

| 批 | 目标数 | 范围（字典序） | passed/failed/ignored |
|---|---|---|---|
| 1 | 10 | `--lib` … `btree_split_test` | 534/0/0 |
| 2 | 10 | `btree_test` … `checkpoint_test` | 76/0/0 |
| 3 | 10 | `cli_test` … `datetime_function_test` | 259/0/2 |
| 4 | 10 | `datetime_type_test` … `executor_test` | 152/0/0 |
| 5 | 10 | `explain_test` … `index_manager_test` | 114/0/0 |
| 6 | 10 | `insert_column_list_test` … `memory_db_test` | 99/0/0 |
| 7 | 10 | `mvcc_abort_test` … `outer_join_test` | 69/0/0 |
| 8 | 10 | `overflow_page_test` … `planner_test` | 222/0/0 |
| 9 | 10 | `predicate_test` … `recovery_replay_convergence_test` | 105/0/0 |
| 10 | 10 | `recovery_tail_persistence_test` … `select_distinct_test` | 113/0/0 |
| 11 | 10 | `self_join_test` … `sync_loader_test` | 148/0/0 |
| 12 | 10 | `table_alias_resolution_test` … `version_chain_test` | 149/0/0 |
| 13 | 10 | `view_test` … `write_type_conformance_test` | 83/0/0 |

- **覆盖核对**：ran 130 = want 130（集成目标不重不漏）；全量合计 **2123 passed / 0 failed / 2 ignored**（基线 Iteration 001 终态 2109/0/2 + 本轮新增 `prepared_statement_test` 14 条 = 2123/0/2，无回归；ignored 2 与基线一致，位于 `cli_test`）。
- **一次未捕获的库单测偶发失败（如实记录）**：T10 批次内的一次 `cargo test --lib` 首跑为 **413 passed / 1 failed**，该次输出经 `tail -3` 截断，**失败用例名未捕获**。此后针对该面的复核：完整 `cargo test --lib` 连续 **14 次全绿**（含上述定向复核 3 次 + M19 批内 1 次 + 事后审计 10 次）；嫌疑用例 `transaction::row_lock::tests::test_row_lock_different_rows` 的上界计时断言（`elapsed < 20ms`，`src/transaction/row_lock.rs:129`）在 CPU 负载下可失效——该用例定向复跑 20 次全绿、8 进程满载下全量复跑 9 次全绿，均未复现。本 Cycle 改动面（plan cache 失效点、plan_stage 拆分、多语句错误构造、新增预编译类型）不含计时或调度敏感代码。**结论：未复现、用例身份未确定**，作为遗留观察记入「未解决问题」，不作为完成声明的依据之一（完成声明基于上表 13 批全绿）。

### Persisted Evidence

None required —— 全部验证为 `cargo test` 集合内的行为断言（可低成本重跑），无一次性环境、无需保留现场；命令与逐批计数已在本 Response 承载。

### Experience Candidates

1. **Runbook 候选——M19 分批回归执行器（单目标失败不中止同批后续目标）**：本轮 13 批 130 目标实跑稳定通过；执行器对每个目标独立记录 passed/failed，单目标失败只标记不终止同批后续目标（Iteration 001 轮次因 cargo 中止导致 4 目标被中止需补跑——本轮形态免去该补跑步骤）。纪律要点：前置 `cargo build --lib --tests --bins -j 2` 限并发编译、批内 `--test-threads=1`、批间串行、每批前可用内存守卫、逐目标计数落盘、结束汇总 ran-vs-want 覆盖核对。证据：本 Response「验证」段分批计数表与覆盖核对行。

### 需 Plan Review 裁定的设计回折

偏差 1 触及 design D7 与 Iteration 002 Stable baseline 的机制描述——D7 原句为「**每次执行操作计划私有副本**——`&self` 并发安全（A7），无共享可变状态」，实现改为「每次执行持有计划副本；同一条预编译语句的并发执行经闸门串行化（计划树参数节点为共享可变状态）」。Act 只记录偏差与理由，不改写 Plan 产物。Plan Review 需裁定：①D7 与 Stable baseline 的并发机制表述是否回折为闸门串行化；②是否接受该回折（可观察 Acceptance 四场景全部成立、并发正确性由先行测试④见证）；③若不接受，替代路径为执行器侧深拷贝能力（`Expression`/`Predicate` trait 增克隆重建方法）——超出本 Iteration 的 database/pipeline 目标面与 T10 Forbidden 范围，须另立设计。

### 未解决问题

- **一次未捕获的库单测偶发失败**：用例身份未确定、未复现（详见「验证」段末条）。本 Cycle 不据其做任何完成声明；建议后续在全量回归中保留逐目标日志以便定位。
- **既有观察（change 范围外，未复现，仅代码面证据）**：plan cache 存的是 `plan.clone()`（浅克隆），执行期相关子查询注入对缓存计划的表达式节点就地改写——同一 SQL 文本的并发执行存在串扰面。该行为属既有设计（计划树参数节点为暂态状态），本 Cycle 未改动该路径、未复现并发串扰，故不作为缺陷立案；若后续需要「同一缓存语句并发执行」的正确性保证，须独立评估。
- **DML WHERE 定位键位参数化仍不可达**（design R4，需执行期键求值重构）：Iteration 001 既有边界，本 Cycle 未触及。

## Plan Review

- Review Result: accepted（2026-10-23 复审——T9-T11 独立核对通过；D7 并发机制设计回折裁定接受并回写 design.md 与 Iteration Plan；全量终态 2123/0/2 对账采信）

**Findings（复审）**

- **T9（版本戳单点）核对通过**：`Database.schema_version: Arc<AtomicU64>`（`src/database.rs:118`）构造点唯一——open 家族单点 `open_internal` 的唯一 `Ok(Self { … })`（`:385`）初始化为 0，内存语义成立；读取器 `schema_version()`（`:618-620`）SeqCst。`bump_schema_version`（`src/pipeline.rs:368-373`）clear + fetch_add 同点、先清后增——「缓存已清空」蕴含「版本已递增」，预编译语句不会持比缓存更旧的失效判据。7 个生产站点全部收敛：`execute_stage_routed` ×6（`:427` CreateTable/DropTable、`:446` CreateIndex/DropIndex、`:457` AlterTable、`:468` Truncate、`:487` CreateView/DropView、`:494` CTAS）+ `execute_stage_in_tx_routed` ×1（`:1226`）；逐臂核对 bump 均在 `execute_executor`/`execute_ctas_routed` 返回之后、response 返回之前——与原 `plan_cache.clear()` 同一位置，「成功后与失败后都清」的时序逐字节保持。测试模块 2 处（`:3630`/`:3773`）按契约未迁移（非生产路径）。
- **T10（prepare/PreparedStatement）核对通过**：`prepare` 序 = 版本戳先读 → `plan_prepared`（parse → 单语句 → 编号 → 计数 → `plan_stage_uncached`）→ 快照入 state（`database.rs:591-606`）。`execute`/`execute_in_tx` 同款五步（gate → resolve_plan → check_param_count → materialize → execute_stage[_in_tx]，`:734-761`）——闸门覆盖「计划解析 + 注入 + 执行」全段，注入与执行不交错。`resolve_plan`（`:766-783`）：版本匹配 → `state.plan.clone()`；不匹配 → **完整重跑** parse+编号+计数+plan（非仅 plan，Guidance 满足——AST 可能因 schema 变化走不同计划路径），re-plan 后 `state.plan = plan.clone()` 归还执行者原件——自持副本的 owned 容器（`InsertNode.values` 等）跨执行保持占位 Null 形态。`state: std::sync::Mutex` 两段临界区均不持锁跨 await（先解锁再 `plan_prepared`，写回为短临界区）——无死锁面。参数值无跨执行残留可读：语句 param_count 固定、每次成功执行注入全部参数后才执行、个数错误在注入前返回、无参语句无参数节点。
- **Deviation 1 前提独立验证成立**：`PhysicalPlan` 为 `#[derive(Clone)]`（`src/executor/plan.rs:21-22`）——对 `ExpressionRef = Arc<dyn Expression>`（`predicate.rs:76`）/`PredicateRef = Arc<dyn Predicate>`（`:47`）子树浅克隆；`ParameterExpression { param_name, value: Mutex<Value> }`（`predicate.rs:427-430`）参数值为计划树共享可变状态。既有同型先例核实：`semi_join.rs:243`/`subquery_eval.rs:189`/`scalar_subquery_filter.rs:170` 均为 `plan.clone()` + `inject_correlated_values` 就地注入——项目既有模型即「计划树参数节点为暂态共享状态」，闸门串行化与该模型一致、非新引入的并发模型。两条契约内出路均被排除属实：每执行重 plan 违反 spec THEN「计划不经每次执行重建」；执行器深拷贝在 T10 Forbidden。被否决备选（仅表达式位参数语句加闸门）的否决理由成立——parser 面位置分类接口误分类即静默串扰。spec R2 只约束可观察安全（「各次执行结果正确，不因并发交错产生错误结果」），不指定机制；D7 原「无共享可变状态」为不可达的机制表述。**裁定：接受回折**（沿 Iteration 001 D5/D6 Review 回折先例——可观察契约不变、设计机制段由 Review 回写）。
- **Deviations 2-5 裁定**：②字段形态 `state: Mutex<PreparedState>` + `exec_gate`——接受（`&self` re-plan 更新自持计划的必然形态，契约明示「内部可变仅限 re-plan 自持更新」）；③版本戳取计划前读数——接受，方向论证独立验证成立（读数偏晚 = 按旧 schema 建的计划标新版本 = 陈旧计划被复用，不可接受向；偏早仅多一次 re-plan，保守向）；④`multi_statement_rejected` 返回类型收敛——接受（文案与 SQLSTATE 逐字节不变经 diff 核对，2 处调用点同步 `Err(...)`，预编译面复用同一错误构造、不造第二套文案）；⑤`plan_stage_impl` 拆分——接受（`plan_stage`/`plan_stage_routed` 签名不变、既有路径恒 `cache_put=true`，开关仅前置到 put 门条件式，key/容量/get 语义零变化）。
- **不变量核对通过**：plan cache 三面——get 面（prepared 不经 `execute_inner`，结构性不读缓存）、put 面（`plan_stage_uncached` 恒 false，既有门 `:351` 四条件不动）、失效面（7 站点 helper 化、时序逐字节）。无 prepare 路径零变化：`execute_with_params`/`execute_in_tx_with_params` 为 Iteration 001 既有面、002 未触碰；本 Cycle 对既有函数的改动仅 bump 站点替换与 `multi_statement_rejected` 类型收敛。错误面同源：`check_param_count` → `pipeline::parameter_count_mismatch` 同一构造点；`plan_prepared` 的 parse/plan 错误形态与参数化通道同式（`Parse error: {msg}` + 42601 / `PlanFailure` 全文）。执行器/parser 零改动：grep 核对两目录无 002 符号。版本戳内存语义：无持久化触点。工作区全量 diff hunk 逐一归属（database.rs 7 hunk、pipeline.rs 全部 hunk）——均属 001 已验收面或本 Cycle 声明面，无范围外改动。
- **测试矩阵核对通过**：`tests/prepared_statement_test.rs` 14 条 = 13 条常规属性 + 1 条 `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]`（grep 字面计数 13 + 1 核对）。R2 四场景逐一映射：①一次 prepare 多次执行——`t10_prepare_once_execute_twice_with_different_params`（写面 2+1 行与直接执行逐条一致）+ `t10_prepared_select_binds_each_execution_params`（读面逐次参数生效、无跨执行残留）；②失效契约——t9 三条（bump+缓存归零 / 非 DDL 不变 / 克隆间共享）+ `t10_schema_change_triggers_transparent_replan`（ALTER 后列数 2→3——直接见证 re-plan 真实发生而非旧计划复用）+ `t10_dropped_table_yields_same_error_as_direct_execution` + `t10_index_change_triggers_transparent_replan`（建/删索引两形态）+ `t10_ddl_can_be_prepared_and_executed`（bump + 幂等）；③显式事务——提交/回滚双路径；④并发共享安全——`tokio::join!` 写面值位 + 读面谓词位（后者为浅克隆共享节点的真实串扰面）。结构断言（`plan_cache_len` 不变×2）与错误面同源断言（个数/多语句/解析/重复建表四组与直接执行逐字比对）齐备。
- **验证采信与对账**：全量算术逐项吻合——13 批 passed 合计 2123 = 基线 2109（Iteration 001 accepted 终态）+ 新增 14；130 目标 = `--lib` + `--bins` + 128 集成文件（文件系统实测 128 = MS30 终态 126 + param_binding + prepared_statement）；ignored 2 落于批 3 `cli_test` 范围，与基线一致；M19 纪律（限并发编译 `-j 2`、批内 `--test-threads=1`、批间串行、批前内存守卫实测 3995-5015 MiB）执行合规。定向 14/0、lib 414/0、param_binding 28/0、typed_api 17/0、build/check/fmt 零告警零 diff 采信（Act Response 记录，覆盖面未失效——其后无代码变更）。
- **一次未捕获偶发失败的处置核对**：413/1 首跑（用例名经 `tail -3` 截断丢失）→ 完整 lib 复核连续 14 次全绿 + 嫌疑用例定向 20 次 + 8 进程满载 9 次均未复现；本 Cycle 改动面（plan cache 失效点、plan_stage 拆分、多语句错误构造、新增预编译类型）经本 Review 独立核对不含计时或调度敏感代码——处置符合验证规则（失败后复核按通过处理、注明首跑失败），完成声明基于 13 批全绿表而非偶发记录，如实分离。用例身份未定 → 无法按实名落账 Issue；「全量回归保留逐目标日志」建议合理，随 Experience 候选由用户裁定。

**Acceptance Gaps**

None——R2 四场景全部有真实入口见证（`Database::prepare` + `PreparedStatement::execute[_in_tx]`）；Invariants（plan cache 三面、无 prepare 路径零变化、执行器/parser 零改动、错误面同源、版本戳内存语义、render/pg_protocol/序列化零改动）逐项满足；全量零回归（2123 = 2109 + 14 对账吻合）。

**Evidence**

- 独立读全部 002 面代码与 diff：`src/database.rs`（字段/构造点/prepare 家族/PreparedStatement 全段）、`src/pipeline.rs`（plan_stage 拆分、put 门、bump helper 与 7 站点上下文、plan_prepared、materialize_params、multi_statement_rejected）、`tests/prepared_statement_test.rs` 全文（449 行）。
- 关键前提独立验证：`PhysicalPlan` derive(Clone) 与 Arc 子树浅克隆、`ParameterExpression` Mutex 共享、三处执行器先例。
- 工作区 hunk 全量归属核对（database.rs 7 hunk、pipeline.rs 24 hunk）——无本 Cycle 范围外改动；executor/parser 目录 grep 无 002 符号。
- 测试清点：属性计数 13+1=14 与声明一致；四场景→具体测试函数逐一映射。
- 采信 Act Response 验证（覆盖面未失效）：定向四组、构建/静态面、M19 13 批计数表与覆盖核对。

**Issue 候选（报告不落账，处置权在 Recorder/用户）**

1. 计时断言族偶发不达的第二例：本 Cycle 嫌疑 `transaction::row_lock::tests::test_row_lock_different_rows`（`elapsed < 20ms` 上界，`src/transaction/row_lock.rs:129`），用例身份未捕获确定；与 Iteration 001 已报告的 e2e 锁等待计时断言（已具名）同类——机器负载敏感的既有测试稳定性问题，非产品缺陷。两例是否合并立案由 Recorder 裁定。
2. plan cache 浅克隆 + 执行期相关子查询就地注入的既有并发串扰面（Act「未解决问题」第二条）：经本 Review 独立核对确认为既有设计模型（暂态参数节点 + clone 注入先例三处），非本 Cycle 引入、未复现——维持 Act 的「报告不立案」处置；若后续出现「同一缓存语句并发执行」的使用场景须独立评估。

**Follow-up Decision**

全部契约点满足、五项偏差均有据且经独立验证、D7 并发机制回折裁定**接受**——Review Result 置 **accepted**，Iteration 002 完成。设计回折按裁定回写：design.md D7（并发机制段替换 + 字段形态 + 版本戳保守口径 + re-plan 完整性）、change tasks.md（Iteration 002 Status → completed、Stable baseline 并发表述同步、头注收口）。MF-A（见下）记录不返工。

**MF-A（非阻塞 Minor，错误路径效率）**：`execute` 中 `check_param_count` 位于 `resolve_plan` 之后——版本不匹配且参数个数错误的执行先付一次 re-plan 再报个数错误（参数化通道为个数校验在 plan 前）。可观察行为一致（同一错误构造、planning 无副作用），仅错误路径多做工作；后续触达时可将 state 内 param_count 前置校验。不返工。

**Iteration Plan Update**

design.md D7 回折（见 Follow-up Decision）；tasks.md Iteration 002 Stable baseline 并发机制表述同步。Iteration Map 本身无变化（三 Iteration 边界与任务归属不动）。

**Next Cycle**

None

**Next Iteration**

None——change 三个 Iteration 全部 accepted（T1-T11 全数完成）；收尾（两域 delta specs 合并、tasks/SNAPSHOT 同步、归档）由用户指令 `openspec-docs-maintainer` 执行。
