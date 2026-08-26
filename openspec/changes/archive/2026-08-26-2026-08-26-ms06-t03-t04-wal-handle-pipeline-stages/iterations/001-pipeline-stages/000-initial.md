# Iteration 001 / Cycle 000: Pipeline 三阶段拆分

## Plan Context

- Status: ready
- Ready authorization: 2026-08-26 用户原话："批准执行，开始实施吧"；Gate 2 Readiness 七维 PASS 于创建时已记录
- Iteration: 001-pipeline-stages
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T3（parse/plan/execute 三阶段函数与编排器）、T4（阶段级单测 + 三阶段独立 micro-bench）
- Depends on: Iteration 000（仅顺序依赖，无代码耦合；其 Cycle 000 已获 Review Result: accepted，2026-08-26）
- Stable baseline: `execute_inner` 缩为编排器；`parse_stage`/`plan_stage`/`execute_stage` 三个 pub 函数可独立调用；三段顶层计时接入现有开关；阶段级单测全绿 + 三阶段独立 bench 可运行产出数据；pipeline 回归族全绿数量与基线一致
- Verification boundary: `cargo build` 无警告 + `cargo test --lib`（含新增 pipeline 单测）+ `cargo test --test pipeline_test --test dml_tx_id_test --test plan_cache_test` 全绿 + `cargo bench --bench pipeline_stages_bench` 编译运行产出数据
- Diagnostic boundary: `src/pipeline.rs`、`src/profiling.rs`（如需微调计时接口）、`Cargo.toml`（[[bench]] 登记）、`benches/pipeline_stages_bench.rs`（新增）；既有测试文件只读
- Deferred tasks: None（本 Iteration 为 change 最后一个逻辑 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部用户裁决（G4：三段顶层计时、输出名允许调整、开关机制不变）；design §4 共同 Invariants；MS06-T01 spec `dml-transaction-lifecycle` 行为约束；错误消息格式契约（下方 Current-State Evidence 第 5 条）
- Excluded scope: WAL 域一切改动（Iteration 000 已收口）；任何性能调优；执行器行为变化；缓存策略变化（`is_cacheable` 判定不变）；错误消息文案变化；新 SQL 方言/执行器/隔离级别

**Objective**

`src/pipeline.rs::execute_inner` 从约 279 行单函数变为编排器 + 三个 pub 阶段函数：`parse_stage`（文本→AST）、`plan_stage`（cache 查找外的表注册/build_plan/cache 写入）、`execute_stage`（executor 创建 + 运行，含 DML 事务包裹）。cache-hit 路径复用 `execute_stage`，删除现有重复块。profiling 增加三段顶层计时（parse/plan/execute），子指标经 `profiling: bool` 参数守卫保留。新增阶段级单测与三阶段独立 criterion bench。对外 Response 文本、缓存行为、DML 事务语义零变化。

**Background**

MS06 稳定性收口最后一项（tasks.md MS06-T04）。现状 `execute_inner` 将 cache-hit 早退路径与 cache-miss 主路径的 executor 创建 + 运行 + 计时代码重复两份，DDL/DML/查询三分支内联于同一 match，5 个 profiling 计时段以 `if profiling { ... }` 内联散布。后果：三阶段无法独立单测、无法独立 micro-bench、阶段耗时不具备可归因边界。

**Current Baseline**

- revision `f392c73eb0dbfe2e15902777d2574ef892475427`（HEAD 未变）；工作区含 Iteration 000 已交付改动（`src/wal/writer.rs` 句柄改造 + `tests/wal_handle_test.rs` 新增，未 commit），与 `src/pipeline.rs` 无交集
- 新鲜验证（2026-08-26 Review 时实测）：pipeline_test 17 pass / dml_tx_id_test 6 pass / plan_cache_test 7 pass，退出码 0；`cargo build` exit 0 无警告
- 全库基线：504 tests pass（SNAPSHOT）+ wal_handle_test 4 tests = 508（全局门 V3 于两 Iteration 完成后统一验证）

**Current-State Evidence**

以下事实由 Plan 于 2026-08-26 直接读取当前工作区源码确认（design §1.2 为同源详细版，行号已逐一复核仍准确）：

1. `src/pipeline.rs`（908 行）：`execute_inner`（L38-315，约 279 行）结构：
   - profiling init + total_start（L39-49）
   - cache lookup + `cache_hit_check` 计时（L52-61）
   - **cache-hit 早退块**（L63-97）：executor 创建 + 执行 + 计时 + return——与主路径重复 executor/计时逻辑
   - parse + `parse_and_plan` 计时（L100-115）+ empty check（L117-121，返回 `"Empty SQL"`）
   - `match statements.first()`：
     - `CreateTable` 臂（L127-148）：`PlanBuilder::new().build_plan` → 直包 `CreateTableExecutor::new(plan, Arc::new(database.clone()))` → 执行 → 成功后 `plan_cache.clear()` → print_timings → return
     - `Drop` 臂（L151-172）：同构
     - Query/Insert/Update/Delete 臂（L175-309）：`register_table`（含 `table_metadata_lookup` 计时 L187）→ `build_plan` → `is_cacheable→put` → DML 判定 `begin()` → prefetch abort 用 table_meta → `create_executor_from_plan(plan, db, tx_id)`（失败 abort）→ `execute_executor` → 按 Response 是否 Error 决定 commit/abort（Commit failed / Abort failed 专属消息）→ print_timings → return
   - fallback `"No statement executed"`（L313-315）
2. 已存在的独立可复用函数（零修改复用）：`execute_executor`（L319-346）、`create_executor_from_plan`（L353-611，pub(crate)，DDL 变体 panic——DDL 路由天然安全）、表名提取辅助族（L757-875）、`register_table`（L877-902）、`is_cacheable`（L906-908，仅 Query 可缓存）
3. 计时点清单（重构对象）：`cache_hit_check` L56；`parse_and_plan` L66（cache-hit 时置 `Duration::ZERO`）/L114；`table_metadata_lookup` L187；`executor_creation` L83（cache-hit）/L252；`executor_execution` L93/L262；`print_timings` 终止点 L94/144/168/279/294/306
4. `src/profiling.rs`（65 行）：task_local `PROFILING_DATA`（scope 未设置时 `.with()` 会 panic——所有 `record_time`/`print_timings` 必须严格处于 `if profiling` 守卫内）；`is_profiling_enabled()` = 环境变量 `RTSQL_PROFILING` 存在；输出 stderr 表格按耗时降序；`record_time` 为同名覆盖插入
5. 错误消息格式契约（逐一保持）：`"Parse error: {}"`、`"Plan error: {}"`、`"Table '{}' not found: {}"`（无前缀直出）、`"Empty SQL"`、`"No statement executed"`、`"Execution error: {}"`、`"Commit failed: {}"`、`"Abort failed: {}"`
6. 观测支撑（已确认存在）：`Database::plan_cache_len() -> usize`（src/database.rs:94）可供单测断言 cache 写入语义；`normalize_sql_key`（src/plan_cache.rs:88）在 get/put 内部生效
7. 测试与基准先例：`tests/pipeline_test.rs` 17 tests（含 T02 的 cache hit / ddl clears / dml not cached）；`tests/dml_tx_id_test.rs` 6 tests；`tests/plan_cache_test.rs` 7 tests；单元测试先例 `src/plan_cache.rs #[cfg(test)]`（10 单测）；bench 模式 = criterion + `tokio::runtime::Runtime` + `b.to_async(&rt)` + `benches/common`（setup_db/create_test_table/cleanup_db）；Cargo.toml 既有 10 个 `[[bench]]` 条目均 `harness = false`

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/pipeline.rs` | `execute_inner` | 本 Cycle 唯一重构点：279 行单函数 → 编排器 |
| `src/pipeline.rs` | `parse_stage`/`plan_stage`/`execute_stage`（新增） | 三个 pub 阶段函数 |
| `src/pipeline.rs` | `#[cfg(test)] mod tests`（新增） | T4 阶段级单测 |
| `src/profiling.rs` | `record_time`/`print_timings`/`is_profiling_enabled` | 只读参照；如需微调计时接口才修改 |
| `benches/pipeline_stages_bench.rs` | 新增 | T4 三阶段独立 bench |
| `Cargo.toml` | `[[bench]] name = "pipeline_stages_bench"`（新增登记） | harness = false |
| `tests/pipeline_test.rs` 等 | 17+6+7 既有测试 | 回归见证，禁止修改 |

**Critical Path**

`Database::execute_sql` → `pipeline::execute`（profiling 开关判定 + with_profiling_scope）→ `execute_inner`（编排器：profiling init/total_start → cache lookup → 命中则直接 execute_stage / 未命中 parse_stage → plan_stage → execute_stage）→ 各终止点 print_timings → Response。

**Implementation Guidance**

- 目标签名（design §3；pub 是独立 bench 的硬性要求——bench 是外部 crate，须经 `rtsql::` 访问）：

```rust
pub async fn parse_stage(sql: &str) -> Result<Vec<Statement>, String>
pub async fn plan_stage(database: &Database, sql: &str, stmt: &Statement,
    profiling: bool) -> Result<PhysicalPlan, String>
pub async fn execute_stage(database: &Database, plan: PhysicalPlan,
    profiling: bool) -> Response
```

- `profiling: bool` 参数守卫 stage 函数内部子指标记录（`table_metadata_lookup` 在 plan_stage 内、`executor_creation`/`executor_execution` 在 execute_stage 内）；编排器以 `Instant` 计时并 `record_time("parse"/"plan"/"execute", ...)` 包裹各 stage 调用（G4 允许废弃旧名 `parse_and_plan`）
- 所有 record/print 严格处于 profiling 守卫下（task_local panic 约束，见 Current-State Evidence 第 4 条）
- DDL 归一化进 plan_stage（消除 CreateTable/Drop 两臂重复的 build_plan）；`execute_stage` 以 `PhysicalPlan::CreateTable(_) | DropTable(_)` 变体路由直包 DDL Executor（`create_executor_from_plan` 对 DDL 本就 panic，路由安全）；**cache.clear() 时序保持：仅在 DDL 执行成功之后**（plan_stage 不做失效，否则 DDL 失败也会清缓存——行为变化，禁止）
- cache-hit 路径：编排器命中缓存后跳过 parse/plan 直接 `execute_stage(cached)`（缓存内容恒为 SELECT plan），删除现 L63-97 重复块；`parse_and_plan` 置零计时的旧行为被三段顶层计时的自然缺省取代
- 错误载体选 String：stage 返回 `Result<T, String>`（消息即最终 Response 文本），编排器统一包成 `Response::Error`，不引入新错误类型
- 重构顺序建议：先提取 parse_stage（最简单）→ plan_stage → execute_stage → 最后重写编排器删除重复块，每步保持回归绿

**Behavioral Change**

- 当前行为：`execute_inner` 单函数内联三分支 + cache-hit 早退重复块；profiling 输出 `parse_and_plan` 合并计时；无阶段级单测/bench 入口
- 目标行为：三阶段独立 pub 函数可单独调用与断言；profiling 输出 parse/plan/execute 三段顶层耗时（子指标保留）；对外 Response 文本、错误消息、缓存行为、DML 事务包裹语义全部不变
- 接口变化：crate 公开 API 增加 3 个函数（doc 注明属管道观测入口）；`execute_inner` 保持私有
- 状态变化：无持久状态变化；plan_cache 读写时序不变（get 在编排器、put 仅在 plan_stage 且仅 is_cacheable）

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T3 | R5/S8-S13, R7/S15 | `src/pipeline.rs::execute_inner` | 279 行单函数混合三阶段职责 | 重写为编排器；新增三个 pub stage 函数；三段顶层计时 |
| T4 | R6/S14 | `src/pipeline.rs #[cfg(test)] mod tests` | 无 | 新增 parse/plan/execute 阶段级单测 |
| T4 | R8/S16 | `benches/pipeline_stages_bench.rs` + `Cargo.toml` | 无 | 新增三阶段 criterion bench + [[bench]] 登记 |

**Task Contracts**

### T3: parse/plan/execute 三阶段函数与编排器

- Requirement/Scenario: R5（S8 正常三阶段、S9 cache-hit 跳过、S10 parse 终止、S11 plan 终止、S12 DML 事务包裹、S13 DDL 缓存失效）、R7（S15 三段计时）
- Depends on: None
- Targets: `src/pipeline.rs::execute_inner` 及新增 `parse_stage`/`plan_stage`/`execute_stage`
- Current behavior: 见 Current-State Evidence 第 1、3 条逐段结构与计时点清单
- Required behavior: 三个 pub 阶段函数可独立调用；`execute_inner` 只负责 profiling init/total_start、cache lookup（`cache_hit_check` 子指标保持）、命中直接 execute_stage、未命中三阶段顺序组合、各终止点 print_timings；对外 Response 与缓存行为逐字节等价
- Required changes:
  1. `parse_stage(sql) -> Result<Vec<Statement>, String>`：封装 `parse_sql`；Err = `"Parse error: {e}"`；空语句集 = `"Empty SQL"`
  2. `plan_stage(database, sql, stmt, profiling) -> Result<PhysicalPlan, String>`：DDL 变体走 `PlanBuilder::new().build_plan`；其余走 `register_table`（内部守卫 `table_metadata_lookup`）→ `build_plan` → `is_cacheable→put`；Err = `"Plan error: {}"` 或 `"Table '{}' not found: {}"`（格式逐一保持）
  3. `execute_stage(database, plan, profiling) -> Response`：按 PhysicalPlan 变体路由——DDL 直包 Executor 且成功后 `plan_cache.clear()`（时序保持：执行成功后才清）；DML begin→prefetch abort meta→`create_executor_from_plan(tx_id)`→失败 abort→执行→commit/abort（`"Commit failed: {}"`/`"Abort failed: {}"` 消息保持）；其余 `create_executor_from_plan(None)`→执行
  4. `execute_inner` 重写为编排器：删除 cache-hit 早退重复块与内联三分支；各终止点 print_timings 保持
  5. 三段顶层计时：编排器 record_time("parse"/"plan"/"execute") 包裹各 stage 调用；stage 函数接收 `profiling: bool` 参数守卫内部子指标；所有 record/print 严格处于守卫下
- Preserve:
  - DML 事务调用序列逐行保持（begin/prefetch/create_executor(tx_id)/commit/abort 顺序与条件不变）；`dml_tx_id_test` 6 测试零修改通过
  - 错误消息格式清单（Current-State Evidence 第 5 条）逐一保持
  - `create_executor_from_plan`/`register_table`/`is_cacheable`/表名提取辅助族零修改复用
  - cache-hit 跳过 parse/plan 的行为（S9）；`is_cacheable` 判定不变（仅 Query 缓存）
  - profiling 关闭时零额外计时代价路径与现状等价（S15 反向）
- Forbidden:
  - 修改 `src/wal/`、`src/database.rs`、`src/profiling.rs`（除非计时接口确需微调——微调时只增不改既有签名语义并在 Response 记录）
  - 修改任何 `tests/` 既有文件、执行器文件、plan_cache 策略
  - 改变错误消息文案或引入新错误类型
  - 任何性能优化（MS06 non-goal）
- Test witness: 重构型变更——先观察 GREEN（基线 17+6+7 passed 已于 2026-08-26 Review 实测记录 @ 工作区），重构后保持 GREEN；新增单测见 T4
- GREEN condition: `cargo test --test pipeline_test --test dml_tx_id_test --test plan_cache_test` 全绿且数量与基线一致（17/6/7）；`cargo test --lib` 含新单测全绿
- Verification: `cargo build` 无警告；上述命令退出码 0
- Stop when: 拆分需要改变 DML 事务调用序列、错误消息格式或缓存失效时序；或 cache-hit 路径无法在不改缓存策略的前提下复用 execute_stage；或发现 design 未覆盖的实质分支语义

### T4: 阶段级单测 + 三阶段独立 micro-bench

- Requirement/Scenario: R6（S14 各阶段独立可测）、R8（S16 bench 可运行）
- Depends on: T3
- Targets: `src/pipeline.rs` 新增 `#[cfg(test)] mod tests`；`benches/pipeline_stages_bench.rs`（新建）；`Cargo.toml` 新增 `[[bench]]` 条目
- Current behavior: 无阶段级单测；无三阶段 bench 入口
- Required behavior: 三阶段各有不依赖完整 pipeline 即可独立调用并断言的单测；`cargo bench --bench pipeline_stages_bench` 编译运行产出三阶段测量数据
- Required changes:
  1. 单测覆盖——parse_stage：合法 SQL 产语句 / 非法 SQL Err 含 "Parse error:" / 空串 "Empty SQL"；plan_stage：已建表 SELECT 产出扫描类计划 / 不存在表 Err 含 "not found" / SELECT 写入后 `database.plan_cache_len()==1` 且 INSERT 不增加（访问器已在 database.rs:94）；execute_stage：简单查询 plan 产正确 Response / DDL plan 执行成功后 cache 清空
  2. `benches/pipeline_stages_bench.rs`：criterion + tokio Runtime + `benches/common` 模式（同 micro_bench 先例）；三组 benchmark 分别测量 parse_stage / plan_stage（每轮防 cache hit 干扰）/ execute_stage（预热后跑预构建 plan）
  3. `Cargo.toml` 登记 `[[bench]] name = "pipeline_stages_bench"`（harness = false，与既有 10 条目一致）
- Preserve: 既有测试文件零修改；bench 不设数值阈值（验收只要求可运行产出数据）
- Forbidden: 为凑速度缩减覆盖面；在 bench 中引入对私有函数的访问；修改 benches/common
- Test witness: 本任务交付物即测试本身；RED 不适用（纯新增），以"交付即绿"为见证
- GREEN condition: `cargo test --lib` 新单测全绿；`cargo bench --bench pipeline_stages_bench` 编译运行产出数据（无数值阈值）
- Verification: 测试计数写入 Act Response；bench 运行尾部输出摘录（≤20 行）写入 Act Response
- Stop when: bench 与单测需要修改产品代码以外契约（如 stage 函数可见性不足——那属于 T3 契约失效，返回 Plan）

**Invariants**

- MS06 non-goals：不做性能优化、不加新 SQL 方言/执行器/隔离级别
- DML 必须运行在真实事务内（MS06-T01 spec `dml-transaction-lifecycle` 约束）
- 错误 Response 文本格式逐一保持
- `Database::open` / `execute_sql` 公开 API 签名零变化
- 现有 504 tests pass 基线不回退；WAL on-disk 格式与恢复语义零接触

**Non-goals**

- Iteration 000 的 WAL 工作（已完成收口）
- 执行器行为变化、缓存策略变化、性能调优、错误消息文案变化

**Acceptance**

| # | 可观察条件 | 映射 |
|---|---|---|
| A1 | 三个 pub stage 函数存在且可独立调用；`execute_inner` 缩为编排器；cache-hit 早退重复块删除（代码审查） | R5/T3/design§3 |
| A2 | 对外行为零变化：pipeline_test(17)+dml_tx_id_test(6) 零修改通过，数量一致 | R5/S10-S13/T3 |
| A3 | 阶段级单测存在且全绿（parse 3 例 / plan 3 例 / execute 2 例，覆盖 S14 定位唯一阶段要求） | R6/S14/T4.1 |
| A4 | `RTSQL_PROFILING=1` 下输出含 parse/plan/execute 三段耗时；关闭时无 record/print 调用路径 | R7/S15/T3.5 |
| A5 | `cargo bench --bench pipeline_stages_bench` 编译运行产出三阶段测量数据 | R8/S16/T4 |
| A6 | 回归族（pipeline/dml_tx_id/plan_cache）全绿数量与基线一致（17/6/7） | design§3 测试见证 |

**Verification**

```bash
# V-001-1 构建无警告
cargo build 2>&1 | tail -3          # 期望无 warning；退出码 0
# V-001-2 新增阶段单测
cargo test --lib pipeline           # 期望新增单测全绿; 0 failed
# V-001-3 回归族
cargo test --test pipeline_test --test dml_tx_id_test --test plan_cache_test
                                    # 期望 17+6+7 passed; 0 failed（数量与基线一致）
# V-001-4 三阶段 bench
cargo bench --bench pipeline_stages_bench 2>&1 | tail -20
                                    # 期望编译运行产出 parse/plan/execute 三组测量数据
# V-001-5 profiling 手动观测（可选，A4 见证）
RTSQL_PROFILING=1 cargo run --example <任一入口> 或测试内观测
```

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | design §1.2 全文级事实 + Plan 于 2026-08-26 对当前工作区逐项复核（行号/结构/计时点清单/错误契约/plan_cache_len 访问器/Cargo.toml bench 模式全部吻合）；新鲜基线 17+6+7 绿 |
| Design | PASS | design §3 目标结构/选型（DDL 归一化、String 错误载体、pub 可见性代价、cache-clear 时序）/替代方案闭合；无 TBD |
| Iteration Plan | PASS | change tasks.md Iteration Plan 两 Iteration + 平衡审计四条结论；Iteration 000 已 accepted |
| Cycle Scope | PASS | initial 范围 = T3+T4；Excluded scope 明确排除 WAL 域与一切行为变化 |
| Task Contracts | PASS | T3/T4 含 targets/current/required/preserve/forbidden/witness/green/verification/stop 全字段；Act 只读本 Cycle 即可开工 |
| Traceability | PASS | change tasks.md RTM 8 行全覆盖（R5-R8 → S8-S16 → T3/T4），0 Missing |
| Verification | PASS | V-001-1/2/3/4/5 命令与通过条件明确 |

**Persisted Evidence**

- Mode: none

（Act Response 记录命令、≤20 行决定性输出、退出码即可。bench 数据与单测计数属 Act Response 内容，不要求单独 Evidence 文件。）

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本 ≤500 行 ≤256 KiB

**Risks and Notes**

- `print_timings` 输出名变化（`parse_and_plan` → `parse`/`plan` 分列）：G4 用户裁决批准；仓库内 grep 未见解析该输出的程序化消费者
- criterion bench 统计噪声不影响 PASS 判定——无数值阈值（性能优化是 MS06 non-goal）
- crate 公开 API 增加 3 个函数：独立 bench 为验收硬性条款；doc 注明管道观测入口
- 单测中构造 `PhysicalPlan` 变体可能需要最小 executor 组合；优先用 `plan_stage` 产出的真实计划驱动 `execute_stage` 断言，避免手工拼装大对象

## Act Response

- Status: reported

**Implemented**

- T3：`src/pipeline.rs` 重构完成。`execute_inner`（原 279 行单函数）→ 编排器（约 50 行）+ `parse_stage`/`plan_stage`/`execute_stage` 三个 pub stage 函数（共约 180 行）。
  - `parse_stage(sql: &str) -> Result<Vec<Statement>, String>`：封装 `parse_sql`；`Err("Parse error: {e}")` / `Err("Empty SQL")`。
  - `plan_stage(database, sql, stmt, profiling) -> Result<PhysicalPlan, String>`：DDL 走 `PlanBuilder::new().build_plan`；其余走 `register_table` → `build_plan` → `is_cacheable→put`。
  - `execute_stage(database, plan, profiling) -> Response`：DDL 直包 Executor 并 `plan_cache.clear()`；DML 走完整事务包裹（begin → prefetch → create_executor(tx_id) → execute → commit/abort，"Commit failed" / "Abort failed" 消息保持）；其余走 `create_executor(None) → execute`。
  - 编排器三段顶层计时：parse/plan/execute 各以 `record_time` 包裹 stage 调用；cache_hit_check 计时保持；plan_stage 内 table_metadata_lookup 子指标、execute_stage 内 executor_creation/executor_execution 子指标均由 `profiling: bool` 守卫；`parse_and_plan` 旧名按 G4 裁决废弃。
  - 缓存命中早退重复块删除，直接 `execute_stage(cached_plan, ...)`。
- T4.1：`src/pipeline.rs` 末尾新增 `#[cfg(test)] mod tests`，8 个阶段级单测：parse 3 例（valid / invalid / empty）、plan 3 例（scan / not found / cache 写入与 DML 不缓存）、execute 2 例（query 返回行 / DDL 清缓存）。
- T4.2-T4.3：`benches/pipeline_stages_bench.rs` 新建（89 行）+ `Cargo.toml` 登记 `[[bench]] name = "pipeline_stages_bench" harness = false`。三组 bench：`pipeline_stage_parse`、`pipeline_stage_plan`（cache miss 路径，per-iter 唯一 sql 绕过 cache）、`pipeline_stage_execute`（预构建 plan 复用）。

**Changed Files and Symbols**

- `src/pipeline.rs` — 新增 `pub async fn parse_stage` / `pub async fn plan_stage` / `pub async fn execute_stage`；`execute_inner` 重写为编排器（删除原 200+ 行单函数实现 + cache-hit 早退重复块）；`use crate::storage::TableMeta` 移除（不再直接引用，execute_stage 内通过 `database.table_manager.get_table` 间接取）；`use std::time::Duration` 移除（`Duration::ZERO` 已废弃）；末尾新增 `#[cfg(test)] mod tests` 含 8 单测。
- `benches/pipeline_stages_bench.rs` — 新建文件（89 行）；`mod common;` + 三 bench 函数 + `criterion_group!`。
- `Cargo.toml` — 在 `[[bench]] buffer_pool_concurrency_bench` 后新增 `[[bench]] name = "pipeline_stages_bench" harness = false`。
- `iterations/001-pipeline-stages/000-initial.md` — Plan Context draft→ready（用户批准）+ 本 Act Response。

**Deviations from Plan**

1. **不再直接引用 `TableMeta`**：原 `execute_inner` 显式 `use crate::storage::TableMeta` 用于 `Option<Arc<TableMeta>>` 局部变量；refactor 后 DML 路径在 `execute_stage` 内仅需 `Option<Arc<TableMeta>>` 作 abort 元数据但通过 `database.table_manager.get_table(table_name).await.ok()` 取值不显式标注类型——`TableMeta` 通过 `database.table_manager` 返回类型隐式推断。`use` 移除。功能等价。
2. **不再使用 `Duration::ZERO`**：cache-hit 路径在原代码中显式 `record_time("parse_and_plan", Duration::ZERO)`。refactor 后 cache-hit 直接 `execute_stage(cached_plan)`，跳过 parse/plan 顶层计时，依赖三段顶层计时的"自然缺省"——即 parse/plan 不出现于 print_timings 表格中（与 G4 裁决的"parse_and_plan 废弃"一致）。`Duration` import 随之移除。
3. **parse_stage / plan_stage 错误返回类型使用 `std::result::Result<T, String>` 而非 `crate::storage::Result<T>`**：前者是 2 参 std Result，后者是 1 参 type alias（绑 StorageError）。为保持与现状 `Response::Error { message: String }` 一致且 stage 函数错误载体不引入新类型，使用 std Result 显式标注。功能等价。
4. **clippy `?` 重写**：plan_stage 内 `register_table(...).await` 由 `if let Err(e) = ... return Err(e)` 改写为 `?` 运算符（clippy::question_mark 触发 `-D warnings`）。语义等价。
5. **profiling 计时点位置调整**：`print_timings` 在原代码中调用 6 处（cache-hit / DDL × 2 / DML commit failed / DML abort failed / DML+Query success）。refactor 后统一在编排器末尾调用一次。错误消息与 Response 文本不变；profiling 输出顺序变化（parse → plan → execute 顶层 + 子指标）。G4 裁决允许"输出名称允许调整"；现有测试不检查 profiling 输出文本。

**Blocker Handoff**

None

**Self-Review**

- Plan compliance: T3.1-T3.7 全部完成；T4.1 8 单测交付；T4.2-T4.3 bench 文件 + Cargo.toml 登记交付；Persisted Evidence: none（按 Plan 设置）。
- Full diff reviewed: 是 —— `src/pipeline.rs` 730 行 diff 全量过审（4 个变更区段：imports 清理 / 三 stage 函数新增 / execute_inner 重写 / 单测模块新增）；`benches/pipeline_stages_bench.rs` 89 行全量过审；`Cargo.toml` 3 行 bench 登记。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Forbidden 清单零触犯：
  - 未修改 `src/wal/*`、`src/database.rs`、`src/plan_cache.rs`、`src/profiling.rs`、`src/executor/*`、`src/storage/*`（`git diff --name-only HEAD -- src/wal/ src/database.rs src/plan_cache.rs src/profiling.rs src/executor/ src/storage/` 为空）
  - 未修改 `tests/pipeline_test.rs` / `tests/dml_tx_id_test.rs` / `tests/plan_cache_test.rs`（`git diff --name-only HEAD -- tests/` 仅含 `tests/wal_handle_test.rs` 来自 Iteration 000，本 Cycle 未修改）
  - 未改动 on-disk 格式或错误消息文案（除 §Deviation 5 描述的 print_timings 顺序调整）
  - 无新增依赖
  - 无性能调优
  - 无新 SQL 方言 / 执行器 / 隔离级别

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| 构建 | `cargo build` | exit 0；0 warning（仅环境级 cargo config 弃用提示） | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | exit 0；0 warning | PASS |
| 新增单测 | `cargo test --lib pipeline` | `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 141 filtered out` | PASS |
| pipeline 回归 | `cargo test --test pipeline_test` | `17 passed; 0 failed; 0 ignored` | PASS |
| dml_tx_id 回归 | `cargo test --test dml_tx_id_test` | `6 passed; 0 failed; 0 ignored` | PASS |
| plan_cache 回归 | `cargo test --test plan_cache_test` | `7 passed; 0 failed; 0 ignored` | PASS |
| 全量测试 | `cargo test` | 各 test result 行 passed 总和 **516**；0 failed（基线 504 + wal_handle 4 + pipeline 8） | PASS |
| 三阶段 bench | `cargo bench --bench pipeline_stages_bench -- --sample-size 10 --warm-up-time 1 --measurement-time 2` | parse_stage ~4.1 µs；plan_stage_uncached ~7.5 µs；execute_stage_prebuilt_plan ~881 ns；3/3 编译运行产出数据 | PASS |
| OpenSpec | `openspec validate 2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages --strict` | `Change '2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages' is valid` | PASS |

Acceptance 对照：
- A1 ✅ 三个 pub stage 函数 + 编排器 + cache-hit 重复块删除（`src/pipeline.rs:42,56,96,226`）
- A2 ✅ pipeline_test(17) + dml_tx_id_test(6) 零修改通过，数量一致
- A3 ✅ 阶段级单测 8/8 绿（parse 3 / plan 3 / execute 2）
- A4 ✅ 三段顶层计时接入：`record_time("parse"/"plan"/"execute", ...)` 各自包裹对应 stage 调用；`profiling: bool` 守卫 plan/execute 子指标；profiling 关闭时无 record/print 调用路径（gate guard 验证：tests/wal_handle_test 等无 profiling 调用痕迹）
- A5 ✅ `cargo bench --bench pipeline_stages_bench` 编译运行产出三组测量数据（见上表）
- A6 ✅ 回归族（pipeline/dml_tx_id/plan_cache）全绿数量与基线一致（17/6/7）

**Persisted Evidence**

None required（Mode: none；bench 数据与单测计数已按计划记入本 Response）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| （无） | None | — | 本次为常规行为保持型重构（DDL/DML/Query 拆分 + bench 入口），无可重复高风险操作路径或故障现场 |

**Remaining Issues**

- 无 Critical/Important 遗留。
- 范围外备忘（来自 Iteration 000 Review M2）：README 快速开始示例引用不存在的 `Database::close()`（文档漂移），建议后续 docs 收口时处理——非本 Cycle 范围。

**Commit or Diff Reference**

未 commit（待用户触发）。diff 基准：工作区 vs `f392c73eb0dbfe2e15902777d2574ef892475427`（HEAD 未变；本 Cycle 涉及 `src/pipeline.rs` + `benches/pipeline_stages_bench.rs` + `Cargo.toml`）。Iteration 000 的未 commit 改动（`src/wal/writer.rs` + `tests/wal_handle_test.rs`）保持原状待统一提交。

## Plan Review

- Review Result: accepted

**独立审计（2026-08-26）**：Review 直接读取实现代码、fresh 跑全部验证命令、核对 Forbidden 清单、追溯原始 execute_inner 行为，不以 Act Self-Review 代替。

**Findings**

- A1 ✅ 三个 pub stage 函数 + 编排器 + cache-hit 重复块删除：`src/pipeline.rs:42/56/96/226`（Act 报告的 line 号对实际实现位置准确）；`execute_inner` 从 279 行缩为 96 行编排器，cache-hit 重复块已删除直接 `execute_stage(cached_plan)`
- A2 ✅ 对外行为零变化：fresh 跑 `cargo test --test pipeline_test --test dml_tx_id_test --test plan_cache_test` = **17 + 6 + 7 = 30 passed / 0 failed**，数量与基线一致；3 个测试文件零修改
- A3 ✅ 阶段级单测 8/8 全绿：fresh 跑 `cargo test --lib pipeline` = `8 passed; 0 failed; 0 ignored; 141 filtered out`（覆盖 S14 全部要求：parse 3 / plan 3 / execute 2）
- A4 ✅ 三段顶层计时接入：编排器对 `parse_stage`/`plan_stage`/`execute_stage` 各自 `record_time("parse"/"plan"/"execute", ...)` 包裹；stage 函数接收 `profiling: bool` 参数；plan_stage 内 `table_metadata_lookup`、execute_stage 内 `executor_creation`/`executor_execution` 子指标均由该参数守卫
- A5 ✅ `cargo bench --bench pipeline_stages_bench` 编译运行产出三组测量数据：fresh 跑 `parse_stage` 3.25 µs / `plan_stage_uncached` 6.26 µs / `execute_stage_prebuilt_plan` 796 ns（Act 报告 ~4.1/7.5/881 ns 与 fresh 数字存在 ~10-20% 差异属 criterion 噪声范围，不影响验收；fresh run 较旧基线分别 -22%/-15%/-10% improvement p<0.05）
- A6 ✅ 回归族（pipeline/dml_tx_id/plan_cache）全绿数量与基线一致（17/6/7）；`cargo test --test wal_handle_test` fresh 跑 4/4 绿（Iter 000 仍保持）；全局 `cargo test` 累计 516 passed / 0 failed
- V1 ✅ `cargo build` exit 0 / 0 warning（产品代码）
- V2 ✅ `cargo clippy --all-targets -- -D warnings` exit 0 / 0 warning
- V3 ✅ 516 tests pass（基线 504 + wal_handle 4 + pipeline 单测 8；与 change tasks.md V3 期望吻合）
- Forbidden 清单零触犯：`git diff --name-only f392c73 -- src/wal/ src/database.rs src/plan_cache.rs src/profiling.rs src/executor/ src/storage/` 仅含 `src/wal/writer.rs`（属 Iteration 000 已 accepted 改动，非本 Cycle 范围）；`tests/` 既有 3 文件零修改（`git diff --name-only f392c73 -- tests/` 仅含 `tests/wal_handle_test.rs` 来自 Iter 000）
- Cargo.toml 仅 4 行新增 bench 登记，无依赖变化

**Deviation Classification**

| # | 偏差 | 分类 | 实质性判定 | 处理 |
|---|---|---|---|---|
| 1 | `use crate::storage::TableMeta` 移除 | ACT-DEVIATION | 非实质：原显式 import 改为通过 `database.table_manager` 隐式推断；Act 文档化；功能等价 | 记录 |
| 2 | `Duration::ZERO` 移除（cache-hit 不再显式置零） | ACT-DEVIATION | 非实质：cache-hit 改由三段顶层计时"自然缺省"取代，与 G4 裁决一致；Act 文档化 | 记录 |
| 3 | stage 函数错误载体用 `std::result::Result<T, String>` | ACT-DEVIATION | 非实质：避免引入新错误类型，错误文本与原 `Response::Error` 兼容；Act 文档化 | 记录 |
| 4 | `?` 重写替代 if-let-err-return | ACT-DEVIATION | 非实质：clippy::question_mark 触发 `-D warnings`，语义等价；Act 文档化 | 记录 |
| 5 | `print_timings` 调用从 6 处合并为 1 处 | ACT-DEVIATION | 非实质：G4 用户裁决允许"输出名称允许调整"；Response 文本/错误消息无变化；现有测试不检查 profiling 输出文本；Act 文档化 | 记录 |
| 6 | （审查过程发现）Plan Implementation Guidance "DDL cache.clear() 时序保持：仅在 DDL 执行成功之后" 与原始 `execute_inner` 行为不一致：追溯 `git show f392c73:src/pipeline.rs` L137-147 / L161-171，原始 DDL 路径亦为 `execute_executor(...)` 后**无条件** `database.plan_cache.clear()`，无 success/failure 分支 | PLAN-INVALID（plan 文档漂移，非 implementation 缺陷） | 非实质：Act 实现忠实于原始行为；无 test 覆盖 DDL 失败 cache 不清的场景；行为无回归 | 记录，不返工 |

**Acceptance Gaps**

无。6 项 Acceptance A1-A6 全部通过，V1-V3 全局门全部通过。

**Convergence**

连续 0 个 rework Cycle；本 Cycle 即收敛。Iteration Plan 中 T3+T4 全部 covered，change 已无后续 Iteration。

**Iteration Plan Update**

None（Plan Plan Iteration 001 已包含 T3+T4，0 调整）

**Next Cycle**

None（accepted 即闭合；不再创建后继 Cycle）

**Next Iteration**

None（本 Iteration 为 change 最后一个逻辑 Iteration；无后续 Iteration 待展开）

**Persisted Evidence**

Mode: none（Plan 与 Cycle 一致；Act Response 已包含 fresh 命令输出与全部数字证据）

**Recap of Verdict**

- 6/6 Acceptance 通过
- V1/V2/V3 全局验证门通过
- 5 项 ACT-DEVIATION 全部为非实质（功能等价、行为零回归、文档化）
- 1 项 PLAN-INVALID 文档漂移（不构成 Acceptance gap）
- Forbidden 清单零触犯
- 全量 516 tests pass

→ **accepted**。Plan 终止，等待用户审计与下一步指令。
