# Iteration 002 / Cycle 000: MS07-T06 谓词/LIMIT 下推（扫描层过滤 + 提前封顶）

## Plan Context

- Status: ready
- Iteration: 002-pushdown
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: Iteration 002 的 T1, T2, T3（`tasks.md` §Iteration 002）
- Depends on: None（与 Iteration 000/001 无代码耦合；workspace 现状含未提交的 T04/T05 改动，见 Current Baseline）
- Stable baseline: 非 PK WHERE 下沿到 DataScan 行内过滤（不生成独立 Filter 节点）；无 Sort/Aggregate 介入的 LIMIT 下推进扫描提前封顶；查询结果与改造前完全一致；PK 路径不退化
- Verification boundary: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff；`cargo test --all` 0 failures（≥562）；新增 `tests/pushdown_test.rs` 全绿；既有 predicate/limit/planner/executor 测试全绿
- Diagnostic boundary: `src/parser/planner/query.rs`、`src/executor/{plan.rs,data_scan.rs,filter.rs}`、`src/pipeline.rs`（执行器构造调用点；tasks.md 边界未列 pipeline.rs，为执行器构造签名变化的必然同步面，见 Current Baseline）
- Deferred tasks: None（本 Iteration 完成后本 change 全部交付；T07 与 MS09 另论）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R3 全部场景（S3.1–S3.5）；design.md Iteration 002 决策 1–3
- Excluded scope: T07；隔离级别/快照语义；新 SQL 方言；新执行器类型；代价模型与 Join 重排；`Scan`（BTree 物化扫描）与 `IndexScan`/`IndexScanAll` 的谓词/LIMIT 字段（见 Non-goals 的范围裁定）

**Objective**

把非 PK WHERE 谓词并入 DataScan 执行器行内过滤（planner 对可下推谓词不再生成独立 `Filter` 节点），把无 Sort/Aggregate 介入的 LIMIT（含 OFFSET）下推进 DataScan 提前封顶，顶层 `Limit` 保留为安全封顶；OR 及复杂谓词保留原 `Filter` 节点；全部查询结果与改造前完全一致；PK 等值路径保持 `IndexScan`。

**Background**

- proposal Why-T06：planner 对非 PK WHERE 产生 `Filter(DataScan/Scan)`，LIMIT 是顶层 `Limit` 节点；谓词过滤与截断都在扫描之上执行。MS07-T03 已把 planner 拆为 `query.rs`/`expression.rs`，提供清晰落点。
- design.md Iteration 002 决策 1–3：谓词以可选字段进扫描执行器、LIMIT 仅在无 Sort/OrderBy 时下推并保留顶层 Limit、等价性由既有与新增查询测试证明。
- 本 Cycle 调查对决策 2 的正确性收紧（实质闭合，见 Implementation Guidance）：`Limit` 之下还可能出现 `Aggregate`（`query.rs:394/441`）与 `DerivedScan`/`Sort`——`Limit(Aggregate(scan))` 形态下把 LIMIT 推入扫描会截断聚合输入、产生错误结果。下推资格必须按"Limit 输入链上只有 DataScan（或 Filter(DataScan)）"判定，而非按"无 ORDER BY"判定。

**Current Baseline**

- Revision: `dc662d4`（HEAD）+ Iteration 000/001 未提交工作区（T04 显式事务 + T05 checkpoint；562 tests pass）。本 Iteration 触碰 `src/pipeline.rs` 的 `create_executor_from_plan` DataScan 分支，与前序 Iteration 区域不重叠。
- 测试基线：562 tests pass（2026-09-05 本会话独立复跑确认）。
- 现状：非 PK WHERE → `Filter(FilterNode{input, predicate, table_name})` 包住 `DataScan`（`query.rs:311-327`）；LIMIT → 顶层 `PhysicalPlan::Limit(LimitNode{input, limit, …})`（`query.rs:490-504`）。

**Current-State Evidence**

- `src/parser/planner/query.rs:281-341` `build_query` WHERE 分派：`try_build_where_subquery`（`subquery.rs:19`，拦截 IN/EXISTS 子查询）→ `extract_pk_from_where` + `is_simple_pk_equality` → 简单 PK 等值 `IndexScan`（:294）；复杂 WHERE 含 PK → `Filter(base_plan=Scan)`（:301-302）；非 PK 且无 PK 等值 → Scan 换成 `DataScan` + `Filter`（:311-327，M19 路由）；有 PK 等值但非简单形态（AND 组合）→ `Filter(base_plan=Scan)` 保持。
- 无 WHERE → 直接 `DataScan`（:334-341）。`DataScanNode` 定义于 `src/executor/plan.rs:74-79`（`table_name` + `columns`，`derive(Clone)`），构造点仅 `query.rs:320/:337` 两处 + pipeline 执行器构造。
- `src/parser/planner/expression.rs:191-235` `build_where`：仅处理 `Identifier`/`CompoundIdentifier`/`Value`/`UnaryOp`/`BinaryOp`/`Nested`（grep 无 Subquery 引用）→ 产出的谓词树全部是行内同步可求值对象（子查询谓词已在 `try_build_where_subquery` 拦截，不进入本分支）。
- `src/executor/predicate.rs:20` `PredicateRef = Arc<dyn Predicate>`；`Predicate::evaluate(&row: &[Value]) -> Result<bool, …>`。`src/executor/filter.rs:22-53` `FilterExecutor::next`：`Ok(true)` → 放行该行；`Ok(false)` → 跳过；`Err(e)` → `StorageError::ExecutionError("Predicate evaluation error: {e}")`。**这是下推必须逐字继承的语义（含错误映射）**。
- `src/executor/data_scan.rs`：`DataScanExecutor` 流式 `next()`（逐 slot、逐页链遍历，`PageAction` 状态机，:110-265），产出行 = 全表 schema 顺序的 `Vec<Value>`（`deserialize_value_refs(tuple_bytes, &schema)`，schema 取自 `table_meta.columns` 全列，:52-56/198-200）→ 谓词列索引与 Filter 现状完全对齐（Filter 收到的也是全列行）。M19/M21 快照与可见性逻辑在 slot 级，谓词过滤插在 `YieldValue` 决策点即可，不触碰可见性。
- `src/executor/scan.rs:9-17` `ScanExecutor` 物化 `results: Vec<Vec<Value>>`（非流式）；其出现形态总在 `Filter` 之下（复杂 PK / has_pk_eq AND 组合，:301-302/:316-318）→ LIMIT 不可穿 Filter 下推（见资格规则），Scan 不需要新字段。
- `src/executor/limit.rs:11-71` `LimitExecutor`：先跳过 `offset` 个 Row，再放行 `limit` 个 Row（非 Row 结果透传不计数）；`limit == 0` 立即返回 None。扫描侧下推等价机制：DataScan 产出满 `offset + limit` 个行后 Done，顶层 Limit 行为不变。
- `src/parser/planner/query.rs:394-504`：`plan_with_aggregate`（有聚合 → `PhysicalPlan::Aggregate`，:441）→ `plan_with_order`（有 ORDER BY → `PhysicalPlan::Sort`，:478）→ `PhysicalPlan::Limit`（:498）。即 `Limit` 输入可为 `Sort`/`Aggregate`/基础计划；FROM 子查询 → `DerivedScan` 为基础计划。
- `IndexScanAll` 无 planner 生成点（`query.rs:34` 仅 output-columns 匹配；grep 全 planner 无构造）；`IndexScan` 仅简单 PK 等值（谓词已消费为 key）→ 二者无现役下推需求。
- 既有测试面：`tests/predicate_test.rs`、`tests/limit_test.rs`、`tests/planner_test.rs`（含 plan 形状断言）、`tests/executor_test.rs`。`plan_stage` 为 pub（MS06-T04），plan 形状可在集成测试中直接断言。

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/parser/planner/query.rs` | `build_query` WHERE/LIMIT 分派 | 谓词与 LIMIT 装载决策（下推资格判定） |
| `src/executor/plan.rs` | `DataScanNode` | 计划节点（新增谓词/封顶字段） |
| `src/executor/data_scan.rs` | `DataScanExecutor::{new, next}` | 行内谓词过滤 + 提前封顶 |
| `src/executor/filter.rs` | `FilterExecutor` | 不可下推谓词的兜底路径（语义基准） |
| `src/pipeline.rs` | `create_executor_from_plan` DataScan 分支 | 把节点新字段传入执行器构造 |

**Critical Path**

```
build_query（非 PK WHERE，无 OR）
  └─► DataScan { table_name, columns, predicate: Some(pred) }   （不再包 Filter）
build_query（WHERE 含 OR / 不可下推）
  └─► Filter { input: DataScan/Scan, predicate }                  （原状保留，S3.5）
Limit 的输入链判定（自 Limit 向下递归）
  ├─ DataScan | Filter(DataScan) → 把 limit+offset 写入 DataScan.scan_cap
  │    （若同链有 Filter 且谓词可下推，谓词一并并入 DataScan，Filter 移除）
  ├─ Sort / Aggregate / DerivedScan / Filter(Scan) / 其他 → 顶层 Limit 原样保留
执行：DataScanExecutor::next
  slot 可见 → 解析 values → predicate.evaluate(&values)
  ├─ Ok(true)  → 已产出行数 < offset+limit ? 返回该行 : Done
  ├─ Ok(false) → 继续下一 slot（不过计数）
  └─ Err(e)    → StorageError::ExecutionError("Predicate evaluation error: {e}")  （= filter.rs）
顶层 LimitExecutor 行为不变（安全封顶 + 兼容未下推形状）
```

**Implementation Guidance**

- 顺序：T1（谓词下推）→ T2（LIMIT 下推）→ T3（回归）。T1 先行，因为 T2 的资格判定要在"谓词已并入 DataScan"的新形状上识别 `Filter(DataScan)` → `DataScan` 的变化。
- 谓词下推资格（T1，实质规则）：进入非 PK 分支的谓词全部来自 `build_where`（行内同步可求值）；但 **OR 必须保留 Filter**（design 决策 1 明示的退化示例，也是 S3.5 的见证路径）——planner 在装载前递归检查谓词来源 AST（或对生成结构做等价判定）：任意深度出现 `OR` → `Filter(DataScan)` 原状；否则 `DataScan{predicate: Some}`。AND 链、比较、IS NULL、IN 字面量、BETWEEN 等全部下推。
- LIMIT 下推资格（T2，实质规则）：自 `Limit` 节点向下，输入链必须恰为 `DataScan` 或 `Filter(DataScan)`（Filter 谓词可下推时先按 T1 并入）；链上出现 `Sort`/`Aggregate`/`DerivedScan`/`Filter(Scan)`/任何其他节点 → 不下推。**`Limit(Aggregate(scan))` 下推会截断聚合输入**——这是 design 决策 2"仅排除 Sort/OrderBy"的缺口，按本规则闭合。下推值 = `offset + limit`（扫描侧产出行封顶）；顶层 `Limit` 节点保留（安全封顶 + 未下推形状的唯一截断点）。
- 执行器语义（T1/T2 共同）：行内过滤的求值对象与 Filter 完全相同（全列 `Vec<Value>`，同列索引）；`Ok(false)` 不消耗封顶计数；错误映射逐字复制 `filter.rs:39-43`。封顶计数只统计已产出行（可见 + 谓词通过），达到 `offset + limit` 即 `Done`；`limit == 0` 时 DataScan 直接 Done（与 `LimitExecutor::next` 首分支一致）。
- `DataScanNode` 新字段的 Clone/Debug 派生保持；`query.rs` 两处构造点与 `pipeline.rs` DataScan 执行器构造点全量同步；`index_scan*.rs`/`scan.rs`/`limit.rs`/`filter.rs` 本体不改。
- 形状见证用 `plan_stage` 直接断言（pub API，pipeline 单测同款构造方式）；结果等价用 SQL 端到端断言。

**Behavioral Change**

- 当前行为：非 PK WHERE → `Filter(DataScan)`（逐行物化后上层过滤）；LIMIT → 仅顶层截断；OR/复杂谓词与简单谓词走同一 Filter 形态。
- 目标行为：无 OR 的非 PK WHERE → `DataScan` 行内过滤（计划中不再出现 Filter）；资格满足时 DataScan 在产满 `offset+limit` 行后提前结束；计划形状变化但查询结果逐行一致；`Filter`/`Limit` 执行器本体行为不变（兜底与封顶角色）。
- 接口变化：`DataScanNode` 新增字段（`predicate: Option<PredicateRef>`、扫描封顶字段——具体形态由 Act 定，如 `scan_cap: Option<usize>`）；`DataScanExecutor::new` 签名相应扩展；`PhysicalPlan`/`create_executor_from_plan` 其余分支不变。
- 错误语义：谓词求值错误在扫描内以与 Filter 相同的 `ExecutionError` 文本传播（顶层可见行为不变）。

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R3/S3.1, S3.4, S3.5 | `planner/query.rs` 非 PK 分支；`executor/plan.rs::DataScanNode`；`executor/data_scan.rs`；`pipeline.rs` DataScan 构造 | `Filter(DataScan)` 上层过滤 | 谓词装入 DataScan 行内过滤；OR 保留 Filter；PK/IndexScan 不动 |
| T2 | R3/S3.2, S3.3 | `planner/query.rs` Limit 分派；`executor/plan.rs::DataScanNode`；`executor/data_scan.rs` | 顶层 Limit 截断 | 资格判定 + 扫描侧 `offset+limit` 提前封顶；顶层 Limit 保留 |
| T3 | R3（全场景）, R5 | 全工作区 | — | 等价性回归 + 全量验证 |

**Task Contracts**

### T1: 谓词下推到 DataScan 行内过滤

- Requirement/Scenario: R3/S3.1（结果不变）、S3.4（PK 不退化）、S3.5（复杂谓词退化）
- Depends on: None
- Targets: `src/parser/planner/query.rs`（非 PK WHERE 分支）；`src/executor/plan.rs::DataScanNode`；`src/executor/data_scan.rs::DataScanExecutor`；`src/pipeline.rs`（DataScan 执行器构造点）
- Current behavior: 非 PK WHERE 生成 `Filter(FilterNode{input: DataScan, predicate, table_name})`；谓词在扫描之上逐行求值
- Required behavior: 无 OR 的谓词由 planner 装入 `DataScan`（新增字段），DataScan 在产出该行前行内求值：`Ok(true)` 放行、`Ok(false)` 跳过且不物化给上层、`Err` 以 `ExecutionError("Predicate evaluation error: …")` 返回（与 filter.rs 逐字一致）；计划中不再出现该 Filter 节点；含 OR（任意深度）的谓词保留 `Filter(DataScan)` 原状；简单 PK 等值仍 `IndexScan`、复杂 PK 仍 `Filter(Scan)`（零变化）
- Required changes: `DataScanNode` 谓词字段 + 两处 planner 构造点 + executor 构造与 `next()` 求值点；下推资格判定（OR 检查）
- Preserve: `FilterExecutor` 本体与 filter.rs 语义（作为不可下推路径与语义基准）；`IndexScan`/`Scan`/`IndexScanAll` 零改动；M19 路由与 M21 可见性逻辑；DataScan 产出的行集与行序与改造前完全一致（含 NULL/类型边界）
- Forbidden: 改 `Predicate::evaluate` 语义或谓词构建（`expression.rs` 零改动）；把 OR 谓词推入扫描；改可见性/快照逻辑；改 `Filter`/`Limit` 执行器本体
- Test witness: 新增 `tests/pushdown_test.rs`：（a）非 PK WHERE（数值/字符串/NULL 边界）结果与预期行集逐行一致；（b）`plan_stage` 断言：非 PK WHERE 计划为 `DataScan`（无 Filter 包裹）且谓词字段为 Some；OR 查询计划仍含 `Filter` 节点；（c）`WHERE pk = v` 计划仍为 `IndexScan`（RED→GREEN）
- GREEN condition: 上述测试全绿；既有 `tests/predicate_test.rs`、`tests/planner_test.rs`、`tests/executor_test.rs` 全绿（断言零逻辑修改）
- Verification: `cargo test --test pushdown_test --test predicate_test --test planner_test --test executor_test`
- Stop when: 需要改谓词构建、Filter 语义或可见性逻辑才能通过；或发现谓词列索引与全列行不对齐（返回 Plan）

### T2: LIMIT 下推与扫描侧提前封顶

- Requirement/Scenario: R3/S3.2（提前封顶 + 行数一致）、S3.3（Sort+Limit 不提前终止）
- Depends on: T1
- Targets: `src/parser/planner/query.rs`（Limit 分派）；`src/executor/plan.rs::DataScanNode`；`src/executor/data_scan.rs::DataScanExecutor`
- Current behavior: LIMIT/OFFSET 仅由顶层 `LimitExecutor` 处理；DataScan 产出全部行由上层截断
- Required behavior: `Limit` 输入链恰为 `DataScan` 或 `Filter(DataScan)`（谓词已按 T1 并入时为纯 `DataScan`）时，`offset + limit` 写入 DataScan 封顶字段，扫描产满即 `Done`；链上出现 `Sort`/`Aggregate`/`DerivedScan`/`Filter(Scan)`/其他节点则不下推；顶层 `Limit` 节点在所有形状下保留（安全封顶）；`limit == 0` 扫描立即 Done
- Required changes: `DataScanNode` 封顶字段 + planner 资格判定与装载 + executor 计数封顶
- Preserve: `LimitExecutor` 本体行为（含 offset 跳过与非 Row 透传）；`Sort`/`Aggregate` 计划路径零变化；行集与行序不变
- Forbidden: 穿 `Aggregate`/`Sort`/`Filter(Scan)` 下推；移除顶层 `Limit` 节点；改 `limit.rs`
- Test witness: `tests/pushdown_test.rs`：（a）`LIMIT n` / `LIMIT n OFFSET m`（含 n 超过行数、offset 超过行数边界）行数与内容与预期一致；（b）`plan_stage` 断言：LIMIT 查询计划中 DataScan 带封顶字段（Some(offset+limit)）且顶层 Limit 保留；（c）`ORDER BY c LIMIT n` 计划中 Sort 保留、DataScan 无封顶字段、结果为排序后前 n 行；（d）聚合 + LIMIT（如 `SELECT COUNT(*) FROM t LIMIT 1`）结果与全量聚合一致（证明不穿 Aggregate 下推）（RED→GREEN）
- GREEN condition: 上述测试全绿；既有 `tests/limit_test.rs` 全绿（断言零逻辑修改）
- Verification: `cargo test --test pushdown_test --test limit_test`
- Stop when: 资格判定需要改 `Sort`/`Aggregate`/`DerivedScan` 计划结构；或发现封顶语义与 `LimitExecutor` 不等价

### T3: 等价性回归与全量验证

- Requirement/Scenario: R3（全场景）, R5
- Depends on: T2
- Targets: 全工作区
- Current behavior: 无（T2 已完成）
- Required behavior: 4 项质量命令全绿；下推后全量测试与改造前行为等价
- Required changes: 验证（无代码改动）
- Preserve: 公共 SQL/网络接口；既有测试断言
- Forbidden: 为过检查引入 `#[allow]`；改既有测试逻辑
- Test witness: `cargo test --all`
- GREEN condition: `cargo test --all` 0 failures（≥562）；`cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` 全 0；`openspec validate --all` 通过
- Verification: 四项命令 + `openspec validate --all`
- Stop when: 任何 check 失败需返工；或公共行为变化

**Invariants**

- 查询结果（行集、行序、NULL/类型边界、错误文本）与改造前完全一致。
- `Predicate::evaluate` 语义、`build_where`/`expression.rs`、`FilterExecutor`/`LimitExecutor` 本体零改动。
- 谓词求值对象为全表 schema 顺序的全列 `Vec<Value>`，与 Filter 现状同索引。
- 简单 PK 等值 → `IndexScan`、复杂 PK → `Filter(Scan)` 路径零变化（S3.4）。
- 顶层 `Limit` 节点在任何形状下保留。
- M19 DataScan 路由与 M21 页面可见性逻辑零改动。
- 扫描 `snapshot: None` 语义不变（无隔离级别引入）。

**Non-goals**

- T07 消息传递重构；代价模型与 Join 重排。
- `Scan`（BTree 物化扫描）的谓词/LIMIT 字段：其现役形态总在 `Filter` 之下，按资格规则不可下推；为其加字段属无产出现众的死代码。
- `IndexScan`/`IndexScanAll` 的谓词字段：前者仅简单 PK 等值（谓词已消费为 key），后者无 planner 生成点（design 决策 1 的罗列按现役面落地，R3 场景全部覆盖，见 Risks）。
- 子查询谓词下推（`try_build_where_subquery` 拦截路径不动）。
- PlanCache 键与缓存语义变化（计划形状变化对缓存透明——键为规范化 SQL 文本）。

**Acceptance**

| Acceptance | 验证 |
|---|---|
| R3/S3.1 谓词下推结果不变 | T1(a)：非 PK WHERE 各边界行集逐行一致；既有 predicate 测试全绿 |
| R3/S3.2 LIMIT 下推提前封顶 | T2(a)(b)：行数内容一致 + DataScan 带封顶字段且顶层 Limit 保留 |
| R3/S3.3 Sort+Limit 不提前终止 | T2(c)：Sort 保留、扫描无封顶字段、结果为排序前 n 行 |
| R3/S3.4 PK 等值仍 IndexScan | T1(c)：计划形状断言 IndexScan 不回归 |
| R3/S3.5 复杂谓词退化 | T1(b)：OR 查询保留 Filter 节点、结果不变 |
| R5 质量门 | T3：4 项命令 + `openspec validate --all` |

**Verification**

- `cargo build`（0 rustc warning）
- `cargo clippy --all-targets -- -D warnings`（0 warning）
- `cargo fmt --check`（0 diff）
- `cargo test --all`（≥562 tests，0 failures）
- `cargo test --test pushdown_test`（新增）
- `openspec validate --all`

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 逐条核实：WHERE 分派/LIMIT 生成/`DataScanNode` 两构造点/`build_where` 变体面（无子查询）/`PredicateRef` 同步求值/DataScan 全列行与流式结构/`ScanExecutor` 物化/`LimitExecutor` 语义/`IndexScanAll` 无生成点 |
| Design | PASS | 下推资格两规则闭合（OR 保留 Filter = design 决策 1；Limit 输入链白名单闭合 design 决策 2 的 Aggregate 缺口）；行内求值语义与错误映射逐字继承 filter.rs；顶层 Limit 保留 |
| Iteration Plan | PASS | Iteration 002 单一职责 T1-T3，依赖有序；稳定基线/验证/诊断边界明确（tasks.md §Iteration 002 + pipeline.rs 同步面补充） |
| Cycle Scope | PASS | initial；T1/T2 覆盖 R3 全部 scenario（S3.1-S3.5），T3 质量门 |
| Task Contracts | PASS | 每 Task 有 Targets/Current/Required/Preserve/Forbidden/Test witness/GREEN/Verification/Stop |
| Traceability | PASS | tasks.md RTM R3 → Iter 002 → `tests/pushdown_test.rs`（新增）→ query.rs/plan.rs/data_scan.rs/pipeline.rs |
| Verification | PASS | 4 项质量命令 + 新增测试通过条件明确；形状与结果两类见证均可执行断言 |

**Persisted Evidence**

- Mode: none

`none` —— 全部验证（四项质量命令、新增/既有测试）可低成本本地重跑，决定性输出写入 Act Response 即可；计划形状与结果等价由测试断言与 diff 审查确认，无需持久化。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超 256 KiB。本 Cycle 无 `required`，不创建 Evidence 目录。

**Risks and Notes**

- **中**：谓词列索引对齐——DataScan 产出全列行、Filter 现状收到的也是全列行（已核实一致）；若 Act 发现某路径存在投影前置（列索引错位），立即停止并返回 Plan（T1 Stop 条款）。
- **中**：`Limit(Aggregate)` 误下推会产生错误聚合结果——资格规则已把 Aggregate/Sort/DerivedScan/Filter(Scan) 排除；T2(d) 用聚合查询见证。
- **低**：design 决策 1 罗列的 `IndexScan`/`IndexScanAll` 谓词字段按现役面收缩到 `DataScan`——R3 五个场景全部不依赖二者（S3.4 仅要求 PK 路径不回归）；收缩记录于 Non-goals，不构成需求裁剪。
- **低**：PlanCache 以规范化 SQL 文本为键，计划形状变化不影响命中；同键下新旧行为经等价性测试背书。
- **非实质（留 Act）**：`DataScanNode` 新字段的具体形态（`Option<PredicateRef>` + `Option<usize>` 或等价）；资格判定的实现位置（build_query 内联函数或独立 helper）；错误文本的拼装方式（须与 filter.rs 一致）。

## Act Response

- Status: reported

**Implemented**

Iteration 002 全部 3 个 Task 完成（2026-09-05，工作区未提交，待用户触发 commit；与 T04/T05 未提交变更同工作区，提交顺序由用户编排）：

- **T1（谓词下推）**：`DataScanNode` 新增 `predicate: Option<PredicateRef>` 字段；planner 非 PK WHERE 分派重构为三路——`has_pk_eq` → `Filter(Scan)` 原状；`contains_or(where_expr)`（任意深度 OR，含 UnaryOp/Nested 递归）→ `Filter(DataScan{predicate: None})` 原状；否则谓词移入 `DataScan{predicate: Some}`、不再生成 Filter 节点（DerivedScan 等其他源保留 Filter 包裹）。`DataScanExecutor::next` 在两个行产出点（页内 Yield 与版本链可见版本）经 `filter_row` 行内求值：`Ok(true)` 放行、`Ok(false)` 跳过且不物化、`Err` 以与 `filter.rs` 逐字一致的 `"Predicate evaluation error: {e}"` 传播；求值发生在可见性判定之后、全列行上（与 Filter 收到的行完全同源）。`contains_or` 为 query.rs 自由函数，递归面与 `build_where` 可构建面精确对齐。
- **T2（LIMIT 下推）**：`DataScanNode` 新增 `scan_cap: Option<usize>`；`build_query` LIMIT 分派在构造顶层 `Limit`（任何形状下保留）前，仅当输入链恰为 `DataScan` 时写入 `scan_cap = Some(offset.saturating_add(limit))`（`limit == 0` → `Some(0)`，扫描立即 Done，与 `LimitExecutor` 首分支等价）；`Sort`/`Aggregate`/`DerivedScan`/`Filter`/其他链不下推。`DataScanExecutor::yield_capped` 按通过可见性 + 谓词后的产出行计数，达到 cap 即终止扫描（`current_page_id = None`），与 LimitExecutor 的拉取次数（offset+limit）精确等价。
- **T3（回归）**：全量验证 577 passed / 0 failed（基线 562 + 本 Iteration 新增 15）；build/clippy/fmt/`openspec validate --all` 全 0（见 Verification Evidence）。

**Changed Files and Symbols**

| 文件 | 变更 |
|---|---|
| `src/executor/plan.rs` | `DataScanNode` 新增 `predicate`/`scan_cap` 字段 |
| `src/parser/planner/query.rs` | 非 PK WHERE 三路分派（含 `contains_or` 自由函数）；无 WHERE 分支字面量补字段；LIMIT 分派 cap 装载；内部单测 `test_unsupported_where` 断言同步 |
| `src/executor/data_scan.rs` | `DataScanExecutor` 新增 `predicate`/`scan_cap`/`produced` 字段与 `new` 5 参签名；新增 `filter_row`/`yield_capped`；`next()` 两个行产出点接入 |
| `src/pipeline.rs` | DataScan 构造点传递 `node.predicate`/`node.scan_cap` |
| `src/executor/correlated.rs` | `inject_correlated_values` 新增 DataScan 臂（见 Deviations 2） |
| `tests/pushdown_test.rs`（新增） | 15 集成测试覆盖 S3.1–S3.5（行为等价 + 计划形状 + 资格规则） |
| `tests/planner_test.rs` | 5 处非 PK WHERE 形状断言 Filter → DataScan（OR 用例保持 Filter）；`test_build_where_comparison_operators` 增加 DataScan 合法臂 |
| `tests/executor_test.rs` | 6 处 `DataScanExecutor::new` 签名同步；`DataScanNode` 解构补字段（强化断言无 WHERE/LIMIT 时两字段为 None）；M19 路由测试 `test_planner_non_pk_where_routes_to_filter_data_scan` 断言同步 |
| `benches/{data_scan,visibility}_bench.rs` | 5 处 `DataScanExecutor::new` 签名同步 |

**Deviations from Plan**

1. **机械同步面超出 Plan 变更面**（`PLAN-OMISSION`，与 Iter 000 delete.rs 同类）：Plan 只列 `pipeline.rs` 为执行器构造同步面；实际 `DataScanExecutor::new` 签名扩展波及 `tests/executor_test.rs` 6 处、`benches/` 5 处，`DataScanNode` 新字段波及 query.rs 内部单测、`tests/planner_test.rs` 5 处形状断言、`tests/executor_test.rs` 解构与 M19 路由测试。全部为签名/形状机械同步，断言语义未弱化（executor_test 解构处反而新增两字段为 None 的强化断言）。旧形状断言（Filter for non-PK WHERE）与目标行为直接冲突，同步为 DataScan 断言——与 Iter 001 Deviation 2 同性质（Preserve 旧形状与 Required Behavior 不可兼得），以契约 Test witness (b) 的目标形状为准。
2. **`correlated.rs` 注入臂（`PLAN-OMISSION`，机械后果，本 Cycle 唯一 Critical 发现，已在范围内修复）**：T1 全量回归首次运行时 5 个相关子查询测试失败（`subquery_test` IN/NOT IN/EXISTS/NOT EXISTS/NULL outer，0 行 ≠ 5 行）。根因：内层相关子查询 WHERE `dept.id = emp.dept` 右侧为 CompoundIdentifier，`has_pk_equality`（仅匹配 `Expr::Identifier`）返回 false → 内层计划走非 PK 分支 → 谓词（含 `ParameterExpression`）被推入 `DataScan`；而 `inject_correlated_values` 只向 Filter/Having 节点注入相关参数、把 DataScan 视为无谓词叶子 → 参数保持 Null → SemiJoin 恒无匹配。修复为 DataScan 臂注入 `node.predicate`（唯一正确实现，grep 全量核对无其他谓词遍历点）；修复后 `subquery_test` 20/20 转绿。Plan Current-State Evidence 未枚举该 walker（其记录的 DataScan 构造点/遍历面清单不含 correlated.rs），属计划调查遗漏而非设计缺陷；T1 Stop 条款（谓词构建/Filter 语义/可见性/列对齐）均未触发。
3. **T2 资格面裁定：链上残留 `Filter(DataScan)` 不下推 cap**（解释性偏差，非阻塞）：契约 Critical Path 字面列出「DataScan | Filter(DataScan) → 写入 scan_cap」。T1 合并后，残留的 `Filter(DataScan)` 只携带不可下推的 OR 谓词——Filter 非行透明，cap 穿透其下推会截断 Filter 输入、改变行数（违反 Invariant「查询结果与改造前完全一致」）。故资格面实现为「恰为纯 `DataScan`」，与契约括注「Filter 谓词可下推时先按 T1 并入（届时链上已无 Filter）」自洽；`or_filter_limit_no_cap_under_filter` 测试见证。
4. **新测试文件书写修正**（测试侧，非实现回归）：初版行断言按「单列投影」书写，实测单表扫描路径本就返回全 schema 行（执行器层无投影，改动前后一致，既有测试无此路径的行宽断言），修正为 `SELECT *` 全列行断言；OR 用例期望行集算术错误修正；Sort 用例因既有「`SELECT *` + ORDER BY 不排序」边界改用显式投影（见 Remaining Issues 1）。

**Blocker Handoff**

None（正常完成）

**Blocker Resolution**

None（未恢复）

**Self-Review**

- Plan compliance: PASS——T1/T2 契约逐项核对（Targets/Current/Required/Preserve/Forbidden/Test witness/GREEN/Stop）；Invariants 全保持（查询结果行集/行序/NULL/错误文本不变——行为测试在实现前后均绿；`Predicate::evaluate`/`expression.rs`/`FilterExecutor`/`LimitExecutor` 本体零改动；谓词求值对象为全列行同索引；简单 PK → IndexScan、复杂 PK → Filter(Scan) 零变化；顶层 Limit 任何形状保留；M19 路由与 M21 可见性零改动；扫描 `snapshot: None` 不变）；Non-goals 全未触碰（Scan/IndexScan/IndexScanAll 无新字段、无子查询谓词下推、PlanCache 语义不变）。
- Full diff reviewed: PASS——完整 diff 逐文件复核（4 源文件 + correlated.rs + 2 既有测试 + 2 bench + 新增测试文件）；与 T04/T05 未提交变更的 `pipeline.rs` 同文件重叠已区分（本 Cycle 仅 DataScan 构造点 +2 行）。
- Critical findings unresolved: None（Deviations 2 的相关参数注入缺失为全量回归捕获，已修复并复验）。
- Important findings unresolved: None
- Minor findings unresolved: None

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 见证 RED | `cargo test --test pushdown_test`（实现前） | `error[E0609]: no field 'predicate'/'scan_cap' on type 'DataScanNode'`（×3），编译失败 | 预期 RED ✅ |
| T1 GREEN | `cargo test --test pushdown_test --test planner_test --test executor_test --test predicate_test` | `8 passed` + `29 passed` + `39 passed` + `12 passed`; 0 failed | PASS |
| T2 行为基线（实现前 GREEN） | `cargo test --test pushdown_test` | 14 passed；仅 `limit_plan_pushes_cap_into_datascand` FAILED（cap 形状） | 预期 RED ✅ |
| T2 GREEN | `cargo test --test pushdown_test --test limit_test` | `15 passed` + `5 passed`; 0 failed | PASS |
| Critical 修复边界 | `cargo test --test subquery_test`（correlated.rs 修复前 → 后） | 修复前 `5 failed`（correlated 0 行）；修复后 `20 passed; 0 failed` | PASS |
| 全量测试（终跑） | `cargo test --all` | `passed=577 failed=0`，exit 0（基线 562 + 15 新增） | PASS |
| 格式 | `cargo fmt --check` | clean，exit 0 | PASS |
| 构建 | `cargo build` | 0 条 rustc warning（仅 `~/.cargo/config` 弃用提示），exit 0 | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | 0 finding（修复 pushdown_test 未用 import 后），exit 0 | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 13 passed, 0 failed (13 items)` | PASS |

**Persisted Evidence**

`None required`（mode: none）——全部验证可低成本本地重跑，决定性输出（退出码、通过/失败计数、RED 症状）已录入上表；计划形状与行为等价由测试断言与 diff 审查确认。

**Experience Candidates**

None

**Remaining Issues**

1. **既有「`SELECT *` + ORDER BY 不排序」边界（观察，非本 change 引入）**：`SortExecutor::compare_rows` 以投影列名（`SortNode.columns`）解析排序列索引；`SELECT *` 时投影列无具体列名可解析 → 排序静默退化为输入序（`sort.rs:46-56` 回落 `Ordering::Equal`）。与本 diff 无代码交集（构造性论证：Sort/ORDER BY 路径未触碰，DataScan 在 predicate/cap 为 None 时行输出逐字节不变；计划形状断言通过、行集与改前一致）。显式投影（`SELECT id, a, b ... ORDER BY id`）排序正常（pushdown_test sort 用例见证）。建议作为 Improvement 候选交用户决策。
2. **既有存储层竞态 / `do_flush` base 竞态 / 语句级原子性与 DDL 非事务性 / 重写截断极端窗口**（Iteration 000/001 Review 遗留，本 Cycle 未触碰相关代码，维持待用户决策）。
3. **下推谓词与快照可见性的求值次序**：谓词在可见性判定之后求值（不可见行不做谓词求值）。当前生产路径 `snapshot: None`，无行为差异；未来引入隔离级别（MS09）时谓词错误可见性语义需重审（记录，不在本 change 范围）。

**Commit or Diff Reference**

未提交（待用户触发 commit，沿用项目「未 commit（待用户触发）」惯例；与 Iteration 000/001 的 T04/T05 变更同工作区，提交编排由用户决定）。本 Cycle 变更面：4 个源文件 + `correlated.rs`（机械臂）+ `pipeline.rs` 增量 2 行 + 新增 1 测试文件 + 2 既有测试文件 + 2 bench 文件。

## Plan Review

- Review Result: accepted
- Review Date: 2026-09-05（Plan 独立审查；非 Act Self-Review 复述）

**审查方法**：逐文件核对未暂存 diff（`git diff src/executor/plan.rs src/parser/planner/query.rs src/executor/data_scan.rs src/executor/correlated.rs src/pipeline.rs tests/ benches/`）对照 T1–T3 Task Contract；交叉核对契约外引用面（filter.rs 错误文本、expression.rs `build_where` 变体面、predicate.rs `inject_parameters` 传播链、plan_cache.rs get/put 克隆语义、其余 `PhysicalPlan::DataScan` 匹配点、`DataScanNode` 全部构造点）；独立复跑定向测试、全量测试与四项质量命令。

**代码契约核对（逐项 PASS）**

- T1 谓词下推：`query.rs` 非 PK WHERE 三路分派与契约一致——`has_pk_eq` → `Filter(Scan)` 原状；`contains_or`（任意深度 OR，递归面为 `build_where` 可构建面的超集，超集方向安全：不可构建形态在分派前已被 `build_where` 拒绝）→ `Filter(DataScan{predicate: None})`；否则 `DataScan{predicate: Some}` 且不生成 Filter。非 Scan 源（DerivedScan 等）保留 Filter 包裹。`filter_row` 对两个行产出点（页内 YieldValue 与版本链可见版本）求值，错误文本与 `filter.rs:39-43` 逐字一致（`"Predicate evaluation error: {}"`，独立比对确认）；求值发生在可见性判定之后、全列行上。
- T2 LIMIT 下推：cap 仅装载于纯 `DataScan` 输入链；`limit == 0` → `Some(0)`、溢出用 `saturating_add`；顶层 `Limit` 节点无条件保留。`yield_capped` 只统计通过可见性 + 谓词的产出行，达 cap 置 `current_page_id = None`（循环头立即返回 `Ok(None)`，与 `LimitExecutor` 拉取语义等价）。
- Deviation 2 修复（correlated.rs 注入臂）：`PhysicalPlan::DataScan` 分支对 `node.predicate` 调 `inject_parameters`；`LogicalPredicate`/`ComparisonPredicate` 递归传播已核实（predicate.rs:99/146-148）；全库 grep 确认谓词遍历注入点仅此 walker 一处。`subquery_test` 20/20 复跑通过。
- 移动安全：`create_executor_from_plan` 按值收 plan，但 PlanCache `get` 返回 clone（plan_cache.rs:33-36）、pipeline 调用点以 `plan.clone()` 传入（pipeline.rs:101/266）——move `node.predicate` 不污染缓存副本。
- 边界外匹配点无遗漏：`subquery.rs:392`、`pipeline.rs:716/1049` 均为列布局提取，与谓词/cap 无关；`DataScanNode` 构造点全集为 query.rs 3 处（全部补字段）+ executor_test.rs:1677 解构。
- Invariants 全保持：expression.rs / filter.rs / limit.rs 零改动（diff --stat 证实）；M19 路由与 M21 可见性逻辑未触碰（diff 仅包裹两个行产出点，懒 set_all_visible 触发条件不受影响——谓词过滤 continue 与 Filter 包裹时同页行为等价）；`snapshot: None` 不变。

**测试见证核对（PASS）**：`tests/pushdown_test.rs` 15 用例覆盖契约全部见证点（T1 a/b/c、T2 a/b/c/d、Deviation 3 的 `or_filter_limit_no_cap_under_filter`、cap 只计通过行）。既有测试同步无断言弱化：executor_test 确解构处新增两字段为 None 的强化断言；planner_test 5 处形状断言与目标行为一致且既有 `test_build_where_logical_or`（断言 Filter）未触碰、复跑通过；bench 5 处为纯签名同步。

**偏差分类**

| 偏差 | 分类 | 裁定 |
|---|---|---|
| 1 机械同步面超出 Plan 变更面（tests/benches） | PLAN-OMISSION | 非阻塞——签名/形状机械同步，断言语义未弱化（executor_test 反而强化） |
| 2 correlated.rs 注入臂 | PLAN-OMISSION（计划调查未枚举该 walker） | 非阻塞——全量回归捕获、唯一正确修复点、修复后复验；T1 Stop 条款未触发 |
| 3 cap 资格收紧为「恰为纯 DataScan」 | 解释性偏差（ACT-DEVIATION，契约括注自洽） | 非阻塞——Filter 非行透明，穿 Filter 下推会截断输入改变行数，违反 Invariant；有测试见证 |
| 4 新测试文件书写修正 | 测试侧修正 | 非阻塞 |

**独立验证复跑（2026-09-05，全部与 Act Response 一致）**

| 命令 | 结果 | 退出码 |
|---|---|---|
| `cargo test --test pushdown_test --test predicate_test --test planner_test --test executor_test --test limit_test --test subquery_test` | 15+12+29+39+5+20 全部 0 failed | 0 |
| `cargo test --all` | **577 passed, 0 failed**（基线 562 + 新增 15） | 0 |
| `cargo build` | 0 rustc warning（仅环境级 `~/.cargo/config` 弃用提示，逐行核实） | 0 |
| `cargo clippy --all-targets -- -D warnings` | 0 finding | 0 |
| `cargo fmt --check` | clean | 0 |
| `openspec validate --all` | 13 passed, 0 failed | 0 |

**Acceptance 核对**

| Acceptance | 裁定 | 证据 |
|---|---|---|
| R3/S3.1 谓词下推结果不变 | PASS | pushdown_test T1(a) 4 用例（含 NULL/AND 边界）+ predicate_test 12/12 |
| R3/S3.2 LIMIT 提前封顶 | PASS | T2(a)(b)（含 OFFSET 越界/LIMIT 0/cap 计通过行） |
| R3/S3.3 Sort+Limit 不提前终止 | PASS | T2(c) Sort 保留 + 无 cap + 排序结果 |
| R3/S3.4 PK 路径不退化 | PASS | T1(c) IndexScan + Filter(Scan) 形状断言 |
| R3/S3.5 复杂谓词退化 | PASS | T1(b) OR 保留 Filter + 结果等价 + 既有 logical_or 测试 |
| R5 质量门 | PASS | 四项命令独立复跑全 0 + validate 13/13 |

**Acceptance Gaps**: None。**收敛状态**: N/A（无 gap）。

**Findings（Minor，不阻塞，记录备查）**

1. 既有「`SELECT *` + ORDER BY 不排序」边界（Act Remaining Issues 1）：审查确认与本 diff 无代码交集（Sort 路径未触碰）；属 Improvement 候选，交用户决策，不属本 Cycle 修复。
2. 谓词在可见性判定之后求值（Act Remaining Issues 3）：当前 `snapshot: None` 无行为差异；MS09 引入隔离级别时需重审。已记录，无需当前动作。
3. Act Deviation 3 的资格面收紧属正确解释而非契约违反——契约 Critical Path 字面与括注存在轻微张力，实际实现取括注语义且方向正确（保守不截断），记录为 Plan 表述瑕疵，不影响本 Review 结论。

**Iteration Plan Update**: None。**Next Cycle**: None。**Next Iteration**: None（Iteration 002 为本 change 最后一个 Iteration；change 全部 3 个 Iteration 的 Review Result 均为 accepted）。

**Follow-up Decision**: None（无当前 Cycle 修复项）。

**结论**：T1/T2/T3 全部满足契约与 Invariants，R3/R5 全场景有测试见证，独立复跑全部验证与 Act Response 一致，无阻塞 finding。Iteration 002 完成；本 change 实现侧交付完毕，未提交工作区（T04/T05/T06）由用户编排 commit 后，可交 `openspec-docs-maintainer` 收尾。
