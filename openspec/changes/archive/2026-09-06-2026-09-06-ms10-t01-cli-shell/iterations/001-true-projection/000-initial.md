# Iteration 001 / Cycle 000: 扫描执行器真投影

## Plan Context

- Status: ready（2026-09-06 用户显式授权"更改gate状态，开始实施"；Gate 2 Readiness 七维全 PASS + 方向 B 扩展已获批准，授权记录留此）
- Iteration: 001-true-projection
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T7, T8, T9, T10
- Depends on: Iteration 000（completed，Review accepted 2026-09-06；CLI 使投影语义可观察）
- Stable baseline: 子集投影在全部扫描路径返回投影列的表头与行；PK 点查聚合返回正确值；投影外排序键正确排序；既有测试校准后 `cargo test --all` 全绿
- Verification boundary: `cargo test --all` 全绿（含校准后既有断言）+ `tests/projection_test.rs` 三症状定向回归 + clippy/fmt/validate 干净
- Diagnostic boundary: `src/executor/{scan,data_scan,index_scan,index_scan_all}.rs`、`src/parser/planner/query.rs`（节点构造与列映射）、`src/pipeline.rs`（executor 传参）、`tests/`（校准面）
- Deferred tasks: None（JOIN 投影、派生表重投影、`SELECT *` 语义变化均不在本 change——前两者天然不需要，后者行为保持）

**Cycle Scope**

- Trigger: initial（Iteration 000 Review 的 NEW-EVIDENCE findings 1/2，用户 2026-09-06 探索会话决策方向 B + 单 Iteration 全做）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 全部既有 Requirement 不回退（R1-R5 行为保持，含 Iteration 000 的 12 个 cli_test 用例语义）；新增 IR1 扫描执行器真投影
- Excluded scope: JOIN/派生表/子查询重投影、`SELECT *` 语义、投影的性能论证、planner 代价模型、MS10-T02/T03/T04/T05

**Objective**

`SELECT <子集投影>` 在全部扫描路径（Scan/DataScan/IndexScan/IndexScanAll）返回投影列的表头与行；三症状（表头错位、PK 点查聚合静默 Null、投影外 ORDER BY 静默失效）全部消除且有定向回归锁定。

**Background**

Iteration 000 Review 发现 IndexScan 表头与行错位（finding 1，实机复现）；用户要求的后续探索（2026-09-06 本会话）揭示同一根因的两个正确性 bug——聚合静默 Null（`SELECT SUM(price) ... WHERE id=2` → `[[null]]`）与 ORDER BY 静默失效——并经 CLI binary 实测复现。用户决策：方向 B（执行器真投影，非元数据对齐），单 Iteration 全做。

**Current Baseline**

- 工作树含 Iteration 000 未提交实现（608 tests pass，Review accepted）；代码基线 = Iteration 000 完成态。
- 三症状实测复现记录（本会话，`target/debug/rtsql`）：
  - `SELECT name FROM s WHERE id = 1` → `{"columns":["name"],"rows":[[1,"Alice"]]}`（错位）
  - `SELECT SUM(price) FROM s WHERE id = 2` → `[[null]]`（正确 20）；对照无 WHERE `[[30]]`、非 PK WHERE `[[50]]` 均正确
  - `ORDER BY name`（name 不在投影）→ 静默原序

**Current-State Evidence**

（Plan 调查确认，2026-09-06 本会话实读代码 + binary 实测）

1. **根因链**：四个扫描执行器不做投影恒返回全 schema 行——`IndexScanExecutor::next` 反序列化全 schema（`src/executor/index_scan.rs:66-90`，schema 来自 `table_meta.columns`）、`IndexScanAllExecutor` 同构、`ScanExecutor`/`DataScanExecutor` 同构。planner 唯一在 PK 等值点查时给 `IndexScanNode.columns` 塞投影子集（`src/parser/planner/query.rs:308` `extract_columns(&select.projection)`）；`ScanNode`/`DataScanNode.columns` 携带全 schema（`query.rs:80-83` `base_columns`）。
2. **聚合 Null 机制**：`input_schema` 从输入 plan 提取（`query.rs:458-471`）——Scan/DataScan/Filter(Scan|DataScan) 臂取 `node.columns`，IndexScan 输入落入 `_ => vec![]` 兜底；`column_indices` 为空 → `AggregateExecutor::extract_value` 对缺失列静默 `Value::Null`（`src/executor/aggregate.rs:271-274`）。`column_indices` 同时用于 group key 提取（`aggregate.rs:282-285`）——GROUP BY 同样受害。
3. **ORDER BY 失效机制**：`SortNode.columns` 构造用 `projection_columns`（`query.rs:526`）；`SortExecutor::compare_rows` 按该列集 `position` 查找排序键（`src/executor/sort.rs:59-63`），找不到即 `Ordering::Equal`（`sort.rs:73`）——静默不排序。注意投影含排序键时**当前行为已正确**（实测 `ORDER BY id DESC` 正确）。
4. **谓词全 schema 语义（关键约束）**：WHERE/下推谓词的 `column_index` 由 planner 按 `self.tables`（register_table 注册全 schema，`planner/mod.rs:50-54`）解析（`expression.rs:114-118/147-155`）；`ComparisonPredicate` 按行 index 取值（`predicate.rs:164`）。⇒ **投影必须发生在谓词求值之后**。DataScan 的两个行产出点（`data_scan.rs:336/356`）都先 `filter_row`（下推谓词）再产出——裁剪插在这两个产出点之后。
5. **消费面全集**（`node.columns` 的全部读者）：
   - `get_plan_output_columns`（`query.rs:23-45`）：CLI 表头 + 派生表列注册（`query.rs:95`）。
   - `extract_column_indices`（`pipeline.rs:705+`）：join 条件/子查询相关列 index 映射——仅用于 Join/SemiJoin/AntiJoin/SubqueryEval/Filter/Aggregate 上下文，IndexScan 不出现在这些输入位（JOIN 的 WHERE 被 planner 拒绝，`query.rs:288-290`）。
   - 聚合 `input_schema`（`query.rs:458-471`，见 2）。
   - `get_subquery_first_column`（`subquery.rs:383-401`）：`SELECT col FROM ...` 子查询的列引用——投影后语义自洽（首投影列）。
   - pipeline executor 构造（`pipeline.rs:435-473`）：当前不传 columns 给执行器。
6. **写路径不受影响**：Update/Delete 不经 IndexScan plan（`ddl_dml.rs`/`update.rs`/`delete.rs` 无 IndexScan 引用）；Insert 产 AffectedRows。
7. **IndexScanAll 是死计划路径**：planner 从不构造 `IndexScanAllNode`（全仓唯一构造是 enum 定义与 correlated 的 match 透传）；只有 `executor_test.rs` 单测直接构造 executor。T7 需同步改它（统一语义）但无生产查询路径。
8. **JOIN 天然投影**：`JoinExecutor::build_output_row` 按 `output_columns` 逐项提取（`join.rs:112-122`）——JOIN 结果已是投影语义，不在本 Iteration 范围。
9. **既有测试约束**：`planner_test::test_select_by_pk` 断言 `node.columns == ["id","name"]`（`tests/planner_test.rs:14-27`，投影恰好=全 schema 所以不变）；`executor_test` 的 IndexScan/IndexScanAll 单测用单列表（`executor_test.rs:81`）断言全 schema 行=投影行，不变；受影响的是 pipeline/plan_exec 等集成测试中"子集投影返回全 schema 行"的断言（T9 逐个核对）。
10. **MVCC 快照路径**：IndexScan 的 `find_visible_version` 闭包按全 schema 反序列化（`index_scan.rs:65-78`）——投影插在可见性判定之后的行产出处，MVCC 语义不动。

**Relevant Code**

- `src/parser/planner/query.rs`（IndexScanNode 构造 `:308`；`input_schema` `:458-471`；SortNode `:522-527`；DataScan/Scan 节点 columns 语义）
- `src/executor/{scan,data_scan,index_scan,index_scan_all}.rs`（行产出点投影）
- `src/executor/sort.rs`（排序列查找）、`src/executor/aggregate.rs`（extract_value/group key，行为随列映射对齐自动修复）
- `src/pipeline.rs:435-473`（executor 构造传参）
- `tests/projection_test.rs`（新）+ `tests/` 校准面

**Critical Path**

planner：投影列表 → 四个 scan 节点 `columns`（统一为输出投影；`SELECT *` = 全 schema）。执行器：全 schema 行（谓词/可见性求值）→ 按投影 index 裁剪 → `ExecResult::Row`。聚合/排序：输入行形状=投影后，`input_schema`/SortNode.columns 与之一致。CLI/PG 协议消费投影后行——表头与行天然一致。

**Implementation Guidance**

- 投影表示：planner 侧解析投影列 → 按全 schema 求 index（`table_meta` 列序），节点携带 `projection_indices: Vec<usize>` 或列名+执行器自解析（Act 在契约内选；倾向 index，避免执行器再查 schema）。
- 裁剪时机：全 schema 行在手、谓词与 MVCC 已判定 → `values` 裁剪后返回。DataScan 的两处产出点（`data_scan.rs:336/356`）、IndexScan 的两处（snapshot/no-snapshot）、Scan/IndexScanAll 各一处。
- T8 排序键在投影外：排序键的全 schema index 可在 SortExecutor 比较时从**裁剪前行**取（执行器排序缓冲持有裁剪前行、产出时裁剪）或 Sort 节点携带全 schema index——前者改动更局部；Act 自选。
- TDD：T7 先写 `tests/projection_test.rs` 子集投影 RED（两条路径），T8 加聚合/排序 RED，GREEN 后进 T9 校准。

**Behavioral Change**

- 子集投影查询：全 schema 行 → 投影行（四路径统一）。这是本 Iteration 的目标语义。
- PK 点查聚合：`[[null]]` → 正确值。投影外 ORDER BY：静默原序 → 正确排序。
- `SELECT *`/全列投影/JOIN/子查询/DML/网络协议路径：行为不变。
- Invariant 边界说明：Iteration 000 的"585 既有测试零修改"不变量按 Iteration 边界解释——本 Iteration 允许校准既有断言（行为变化即目标），校准纪律见 T9。

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T7 | IR1/S1,S2,S4 | `query.rs`（节点构造）+ 四执行器 + `pipeline.rs:435-473` | 执行器全 schema 行；IndexScan 元数据谎言 | 节点携带投影；执行器产出前裁剪 |
| T8 | IR1/S3,S6（聚合/排序） | `query.rs:458-471` + `sort.rs:59-73` + SortNode 构造 | 聚合列映射空兜底 Null；Sort 按投影 position 查找 | 列映射与投影后行形状一致；投影外排序键正确求值 |
| T9 | IR1/S5（校准） | `tests/`（含 `cli_test.rs` ④⑥） | 全 schema 断言 | 按投影语义校准 + 定向回归入 `projection_test.rs` |
| T10 | 全部 | 验证命令 | — | 全量回归门 |

**Task Contracts**

### T7: plan 节点携带投影 + 执行器按投影裁剪行

- Requirement/Scenario: IR1 / S1（全路径投影行）、S2（表头行一致）、S4（`SELECT *` 不变）
- Depends on: None
- Targets: `src/parser/planner/query.rs`（IndexScanNode `:308`、ScanNode/DataScanNode 构造与 columns 语义）、`src/executor/scan.rs`、`src/executor/data_scan.rs`（产出点 `:336/356`）、`src/executor/index_scan.rs`（产出点 `:65-90`）、`src/executor/index_scan_all.rs`、`src/pipeline.rs:435-473`
- Current behavior: 四执行器恒返回全 schema 行；`IndexScanNode.columns` = 投影子集（元数据与行不一致）
- Required behavior: 四个 scan 节点的 `columns`/投影元数据统一表示**输出投影**（`SELECT *`/全列 = 全 schema）；执行器在全 schema 行上完成谓词与 MVCC 判定后按投影裁剪产出；表头（`get_plan_output_columns`）与行形状在所有路径一致
- Required changes: planner 解析投影并传到节点（含 IndexScan 构造点修正为真实投影）；执行器产出点裁剪；pipeline 构造 executor 时传递投影
- Preserve: 谓词 `column_index` 的全 schema 解析语义（投影在谓词后）；MVCC 可见性判定语义；`SELECT *` 与全列投影行为逐字节不变；JOIN/DML/DDL 路径零变化
- Forbidden: 不改 JOIN `build_output_row`；不做派生表重投影；不做性能优化论证；不引入新 plan 节点类型
- Test witness: RED——`tests/projection_test.rs`（新文件）：(a) `SELECT name FROM s` 断言行 `[["Alice"]]`（现在双字段 → RED）；(b) `SELECT name FROM s WHERE id = 1` 断言单列行（现在双字段 → RED）；(c) `SELECT * FROM s` 断言全 schema 行不变（现在 GREEN，防回归）；命令 `cargo test --test projection_test`
- GREEN condition: (a)(b)(c) 全过
- Verification: `cargo test --test projection_test` + `cargo test --all`（此时允许存在待校准的既有失败，T9 收口；但 projection_test 自身必须绿）
- Stop when: 发现第五个行产出点或谓词依赖投影后形状的场景（契约缺口，返回 Plan）

### T8: 聚合 input_schema 与 Sort 列映射对齐

- Requirement/Scenario: IR1 / S3（PK 点查聚合）、S6（投影外排序）
- Depends on: T7（输入行形状=投影后）
- Targets: `src/parser/planner/query.rs:458-471`（`input_schema` 提取）、`src/executor/sort.rs:59-73`（排序键查找）、`query.rs:522-527`（SortNode 构造）
- Current behavior: 聚合对 IndexScan 输入列映射空 → 静默 Null；Sort 按投影列集 position 查找，投影外排序键静默不排序
- Required behavior: 聚合 `column_indices` 与输入 plan 真实输出列（= 投影后）一致，PK 点查聚合返回正确值；排序键不在投影内时仍按全 schema 语义正确排序（排序缓冲持有裁剪前行或节点携带全 schema index，Act 自选实现）
- Required changes: `input_schema` 提取覆盖全部输入 plan 形状（或经 `get_plan_output_columns` 统一）；Sort 键解析与行形状解耦（投影外键可达）
- Preserve: 聚合函数语义（COUNT/SUM/AVG/MIN/MAX）；GROUP BY/HAVING 行为（随列映射对齐自然修复）；聚合投影含聚合列时现有正确行为
- Forbidden: 不以显式报错替代投影外排序（SQL 语义裁剪）；不动 `AggregateFunc`/聚合执行器算法本体
- Test witness: RED——`projection_test.rs` 增：(d) `SELECT SUM(price) FROM s WHERE id = 2` 断言 `[[20]]`（现在 `[[null]]` → RED）；(e) `SELECT id FROM s WHERE price > 15 ORDER BY name DESC` 断言按 name 降序（现在原序 → RED）
- GREEN condition: (d)(e) 过 + 既有聚合/排序测试（校准后）全绿
- Verification: `cargo test --test projection_test` + `cargo test --all`
- Stop when: 排序键投影外语义需要改 SortNode 结构导致 plan 序列化/缓存兼容问题（plan_cache 缓存克隆的 PhysicalPlan，节点字段新增是 Clone 安全的，预期不触发）

### T9: 既有测试校准 + 三症状定向回归

- Requirement/Scenario: IR1 / S5（校准与回归）
- Depends on: T7、T8
- Targets: `tests/`（executor_test / pipeline_test / plan_exec_test / aggregate 相关等全 schema 断言）、`tests/cli_test.rs`（④ 多语句"零执行"断言、⑥ CSV e2e——改回子集投影锁定新语义）
- Current behavior: 大量断言假设子集投影返回全 schema 行
- Required behavior: 断言按投影语义校准（只改行形状/列数期望，不改测试意图）；校准清单（文件→用例→变更）记入 Act Response；三症状定向回归留在 `projection_test.rs`
- Required changes: 逐文件核对 `cargo test --all` 失败项 + 主动排查未失败但断言全 schema 形状的用例（如 `SELECT name` 断言双字段）
- Preserve: 测试意图不变；`cli_test` ① 的表头断言语义（列名存在）
- Forbidden: 不删测试；不弱化断言语义凑通过；不为校准而改产品代码（产品行为以 T7/T8 契约为准）
- Test witness: 校准过程本身（清单 + 前后对比）
- GREEN condition: `cargo test --all` 全绿
- Verification: `cargo test --all`
- Stop when: 校准暴露的行为差异超出投影语义（如聚合函数值变化）——那是新缺陷，返回 Plan

### T10: 全量回归与 Iteration 验证门

- Requirement/Scenario: 全部 Requirement（R1-R5 + IR1 回归门）
- Depends on: T7-T9
- Targets: 无代码变更
- Required behavior: `cargo test --all` 全绿、`cargo clippy -D warnings` 0、`cargo fmt --check` 0、`openspec validate 2026-09-06-ms10-t01-cli-shell` PASS
- Test witness: 命令输出（每项 ≤20 行决定性片段）+ 退出码记入 Act Response
- GREEN condition: 四命令全过
- Verification: 同上
- Stop when: 回归失败定位到本 Iteration 未触及模块（基线变化，返回 Plan）

**Invariants**

1. 谓词（WHERE/下推/JOIN 条件）与 MVCC 可见性判定在全 schema 行上求值——投影只发生在行产出最后一步。
2. `SELECT *` 与全列投影的行为逐字节不变（含 585 基线中覆盖这些形状的既有断言零修改）。
3. JOIN/DML/DDL/子查询/网络协议路径行为零变化。
4. 三症状消除以 `tests/projection_test.rs` 定向回归锁定，不再依赖 CLI 手工验证。
5. 退出码体系与 CLI 编排（Iteration 000 成果）零变化。
6. 既有测试校准只改行形状期望，不改测试意图；校准清单必须完整记录。

**Non-goals**

见 Excluded scope；另：不做投影裁剪的性能 bench（正确性 Iteration）、不改 `SELECT *` 展开逻辑、不处理 `SELECT` 表达式层新语法。

**Acceptance**

| # | 条件 | 映射 |
|---|---|---|
| B1 | 子集投影在 Scan/DataScan/IndexScan 路径返回投影行，表头与行一致 | IR1/S1,S2 + T7(a)(b) |
| B2 | `SELECT *`/全列投影行为不变 | IR1/S4 + T7(c) |
| B3 | PK 点查聚合返回正确值（SUM 示例 `[[20]]`） | IR1/S3 + T8(d) |
| B4 | 投影外排序键正确排序 | IR1/S6 + T8(e) |
| B5 | 既有断言按投影语义校准且全绿，清单完整 | IR1/S5 + T9 |
| B6 | R1-R5（Iteration 000 成果）零回退：cli_test 12 用例（校准后）全绿 | 继承 + T9/T10 |
| B7 | clippy/fmt/validate 干净 | 全部 + T10 |

**Verification**

`cargo test --test projection_test`、`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t01-cli-shell`。输出记入 Act Response；不需要持久化 Evidence。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 1-10（本会话实读代码 + binary 三症状实测复现；消费面/写路径/死路径/MVCC 边界全覆盖） |
| Design | PASS | 谓词前投影后顺序、节点元数据语义统一、排序键解耦三决策闭合；方向 B 用户批准 2026-09-06 |
| Iteration Plan | PASS | tasks.md Iteration 001 + 平衡审计（三症状一根因，拆分产生部分投影中间态） |
| Cycle Scope | PASS | initial，IR1 六场景全覆盖，排除项明确 |
| Task Contracts | PASS | T7-T10 四契约，每项含 Target/见证/GREEN/停止条件 |
| Traceability | PASS | RTM 见下 |
| Verification | PASS | B1-B7 映射到具体测试与断言 |

**Persisted Evidence**

- Mode: none

三症状复现命令（binary 一行式）低成本可重跑；全部验证命令可重跑；Act Response 承载足够支撑 Review。

- Budget: 本 Cycle 最多 5 个文件（含 README）；change 合计 ≤20；单文件 ≤500 行且 ≤256 KiB。

**Risks and Notes**

- 既有测试校准面规模未知（预计几十处，集中在 pipeline/plan_exec/aggregate 测试）——T9 逐文件核对，失败项 + 主动排查双路径控制遗漏。
- `plan_cache` 缓存含投影字段的节点：SELECT plan 缓存跨进程不持久（内存 cache），同进程内新旧 plan 不混存；节点字段新增对 Clone/Debug derive 安全。
- 排序键投影外的实现取向（裁剪前行入排序缓冲 vs 节点携带全 schema index）为非实质选择，Act 定；两者行为等价。
- 非实质选择留给 Act：投影的节点字段表示（index vs 列名）、`tests/projection_test.rs` 内部组织。
- WSL2 环境噪声对纯逻辑测试无影响。

**RTM**

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| IR1 真投影 | S1/S2/S4 | Critical Path | T7 | 001 | 四执行器 + query.rs 节点构造 + pipeline 传参 | projection_test (a)(b)(c) | None | Covered |
| IR1 真投影 | S3 | Current-State Evidence 2 | T8 | 001 | query.rs:458-471 + aggregate.rs 列映射 | projection_test (d) | None | Covered |
| IR1 真投影 | S6 | Current-State Evidence 3 | T8 | 001 | sort.rs:59-73 + SortNode 构造 | projection_test (e) | None | Covered |
| IR1 真投影 | S5 | T9 契约 | T9 | 001 | tests/ 校准面 | 校准清单 + cargo test --all | None | Covered |
| R1-R5（继承） | 全部 | 不变量 5 | T9/T10 | 001 | 零回退 | cli_test 12 用例（校准后） | None | Covered |

## Act Response

- Status: reported

**Implemented**

T7-T10 全部完成（Cycle 000-initial；Plan Context 由 `draft` 置 `ready`——用户 2026-09-06 显式授权"更改gate状态，开始实施"，Gate 2 Readiness 七维 PASS + 方向 B 已批准，授权记录见 Plan Context Status 行）。

- **T7（plan 节点携带投影 + 执行器按投影裁剪行）**：`ScanNode`/`DataScanNode`/`IndexScanNode`/`IndexScanAllNode`/`FilterNode` 新增 `projection: Vec<usize>`（空 = 恒等，产出全 schema 行）；五个执行器（scan/data_scan/index_scan/index_scan_all/filter）新增 `with_projection` builder（与 MS08-T02 `with_prefetch` 同风格），在**谓词求值与 MVCC 判定之后**的行产出点裁剪（scan 2 处 push、data_scan 2 处 `yield_capped` 前、index_scan 2 处 return、index_scan_all 2 处 return、filter 谓词通过后）；`pipeline.rs` 6 个构造点接线。planner 侧：新增 `resolve_projection_indices`（SELECT 列表 → 全 schema index，按投影序；wildcard/别名/表达式/聚合/含标量子查询时返回 None = 恒等回退）；聚合检测循环上移至 WHERE 处理前（`has_aggregates` 门控投影，机械移动无行为变化）；IndexScan 构造点 `columns` 修正为真实输出形状（投影时 = 投影列名，恒等时 = 全 schema），替换原 `extract_columns(projection)` 元数据谎言；`get_plan_output_columns` Filter 臂按投影裁剪描述输出形状；`extract_columns` 的 IndexScan 语义谎言消除。
- **T8（聚合 input_schema 与 Sort 列映射对齐）**：聚合 `input_schema` 由手写 match（IndexScan/IndexScanAll 落入空兜底 → 静默 Null）统一为 `get_plan_output_columns(&plan_with_where)`——覆盖全部输入 plan 形状且与行形状一致；`SortNode` 新增 `projection`，`columns` 语义改为"排序键查找所用输入行形状"（非聚合时 = `get_plan_output_columns(input)`，聚合时保持 projection_columns 与既有行为逐字节一致）；`SortExecutor` 持有全形状行比较排序键、物化时按投影裁剪（design D10 取向一"排序缓冲持有裁剪前行、产出时裁剪"）；投影外排序键正确排序（不报错）。放置策略：投影由**最外层安全节点**拥有——有 ORDER BY 时 Sort 拥有（下方链路恒等全 schema 行，`is_base_scan_chain` 门控保证索引有效），无 ORDER BY 时 scan/Filter 拥有；聚合路径全程恒等（聚合输出形状由 AggregateExecutor 自有）。
- **T9（既有测试校准 + 三症状定向回归）**：校准面实际为空——`cargo test --all` 在 T7/T8 落地后零既有断言失败，证明不存在"子集投影 + 全 schema 行断言"的既有用例（Plan 预计几十处，实测 0，见 Deviation 7）。实际校准/排查动作：① `tests/executor_test.rs::test_planner_no_where_routes_to_data_scan` 解构模式补 `projection` 绑定（编译必需）并新增断言 `projection == [0,1]`（锁定显式全列投影的索引语义）；② cli_test ④⑥ 按契约改回子集投影（`SELECT name FROM users`）锁定新语义——⑥ 同时命中 Iteration 000 Review finding 1 的 IndexScan 表头错位场景，现断言正确行为；③ `tests/pushdown_test.rs` 1 处过时注释修正（其用例 `SELECT *` 恒等投影断言不变）。三症状定向回归锁定于 `tests/projection_test.rs`（6 测试：(a) DataScan 投影 (b) IndexScan 投影 (c) SELECT * 不变守卫 (f) Filter 形状投影 (d) PK 点查聚合 (e) 投影外 ORDER BY）。
- **T10（全量回归门）**：四命令全过（见 Verification Evidence）。

**Changed Files and Symbols**

| 文件 | 变更 |
|---|---|
| `src/executor/plan.rs` | ScanNode/DataScanNode/IndexScanNode/IndexScanAllNode/FilterNode/SortNode +`projection: Vec<usize>`（文档注明空 = 恒等语义） |
| `src/executor/mod.rs` | +`pub(crate) fn apply_projection(&[usize], Vec<Value>) -> Vec<Value>` |
| `src/executor/scan.rs` | +`projection` 字段 +`with_projection`；2 个行 push 点裁剪 |
| `src/executor/data_scan.rs` | +字段 +`with_projection`；2 个 `yield_capped` 产出点裁剪（`filter_row` 之后） |
| `src/executor/index_scan.rs` | +字段 +`with_projection`；snapshot/no-snapshot 2 个产出点裁剪 |
| `src/executor/index_scan_all.rs` | +字段 +`with_projection`；2 个产出点裁剪 |
| `src/executor/filter.rs` | +字段 +`with_projection`；谓词通过后裁剪 |
| `src/executor/sort.rs` | +字段 +`with_projection`；排序后物化时裁剪（比较仍用输入形状） |
| `src/parser/planner/query.rs` | 聚合检测循环上移；+`resolve_projection_indices`/`is_base_scan_chain`；投影解析与门控块；IndexScan 构造修正；4 处 FilterNode/3 处 DataScanNode/2 处 ScanNode 构造接线；input_schema 统一；SortNode 构造重写；`get_plan_output_columns` Filter/Sort 臂 |
| `src/pipeline.rs` | 6 个 executor 构造点 +`.with_projection(node.projection)` |
| `src/executor/correlated.rs` | 测试 fixture：FilterNode/ScanNode 字面量 +`projection: Vec::new()`（机械） |
| `src/plan_cache.rs` | 测试 `dummy_plan` 字面量补字段 + 注释更新（机械） |
| `tests/executor_test.rs` | 解构模式补绑定 + `projection == [0,1]` 断言 |
| `tests/pushdown_test.rs` | 1 处过时注释修正（断言零变化） |
| `tests/cli_test.rs` | ④⑥ 查询改回子集投影 `SELECT name FROM users`，锁定真投影语义 |
| `tests/projection_test.rs`（新） | 6 集成测试（三症状定向回归 + 守卫） |

**Deviations from Plan**

1. **FilterNode/FilterExecutor 携带投影（契约目标文件未列 filter.rs）**：T7 目标清单只含四 scan 执行器，但 `Filter(DataScan)` 形状（OR 谓词 / AND-PK 组合）中谓词由包装 Filter 求值——若 scan 投影则谓词破坏，若不投影则该路径表头与行不一致，违反 T7 Required behavior"表头与行形状在所有路径一致"与 Iteration Objective"全部扫描路径"。处置：D8 原则（投影在谓词之后）应用于 wrapper 谓词，Filter 在谓词通过后裁剪；机械形式与 T8 契约内 sort.rs 完全相同。plan.rs 的 FilterNode 字段扩展与"节点携带投影"契约同构。判定为非实质局部差异（达成既定 Acceptance 所需的最小机制延伸），不构成 Blocker。
2. **投影放置策略系统化**：契约给出 Sort 键解耦的两个实现取向（D10）并授权 Act 择一；实现取"最外层安全节点拥有裁剪"的一致规则（无 Sort → scan/Filter；有 Sort → Sort，下方恒等；聚合/标量子查询/不可解析投影 → 恒等回退），比"仅 Sort 特判"更统一，且恒等回退保证通配符/别名/表达式/子查询形状零回归。
3. **executor 传递投影用 builder `with_projection` 而非改构造函数签名**：契约要求"pipeline 构造 executor 时传递投影"；builder 形式使 ~40 处既有测试/bench 直接构造调用点零修改（join_test/executor_test/plan_exec_test/sort_test/prefetch_test/benches），与 `DataScanExecutor::with_prefetch` 项目先例一致。
4. **聚合检测循环上移至 WHERE 处理之前**：`has_aggregates` 门控投影解析所需；循环体逐字移动，无行为变化。
5. **Scan/IndexScanAll 实际各有 2 个行产出点**（snapshot/no-snapshot 分支）：契约概称"各一处"，实现全部覆盖；无第五类未知产出点（Stop 条件未触发）。
6. **IndexScanNode.columns 语义细化**：投影时 = 投影列名（`projection_columns`，与 `projection` 索引一一对应），恒等时 = 全 schema——契约"节点元数据统一为输出投影"的落实；`planner_test::test_select_by_pk`（全列投影断言 `["id","name"]`）零修改通过。
7. **既有测试校准面为 0（Plan 预计几十处）**：全量零失败证明既有断言无"子集投影 + 全 schema 行"组合；校准清单为空集 + 上述 ①②③ 三处（一处编译性绑定、两处契约点名的语义锁定）。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: pass（T7-T10 契约逐项达成；Invariants 1-6 逐项核对：① 裁剪点全部位于谓词求值与 MVCC 判定之后（data_scan `filter_row` 后 / filter `evaluate` 后 / index_scan 可见性后），谓词 `column_index` 全 schema 语义不动；② `SELECT *` 与全列投影恒等投影，(c) 守卫 + 614 全绿（既有断言除 1 处编译性绑定外零修改）；③ JOIN（`build_output_row` 未动）/DML/DDL/子查询（SemiJoin/AntiJoin `build_output_row` 天然投影未动；标量子查询路径恒等回退）/网络协议（未触碰）可观察行为零变化；④ 三症状由 projection_test (a)(b)(f)/(d)/(e) 锁定；⑤ `src/cli/` 零触碰、退出码体系零变化；⑥ 校准清单完整记录于 Implemented/Deviations）
- Full diff reviewed: pass（本 Cycle 16 文件逐一对照 Change Surface 与 Deviations，无计划外修改；跨任务交互检查——T7 节点字段被 T8 Sort 消费、T7 恒等回退保证 T8 聚合路径输入形状、T9 校准依赖 T7/T8 语义、`with_projection` 链与 Iteration 000 的 pipeline 改动正交；Iteration 000 既有工作树改动不在本 Cycle diff 范围且已由其 Review accepted 覆盖）
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 4（见 Remaining Issues）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T7 RED | `cargo test --test projection_test`（实现前） | `FAILED. 1 passed; 3 failed`（(a)(b)(f) 全 schema 行 vs 投影行；(c) 守卫 GREEN） | RED 符合预期 |
| T7 GREEN | `cargo test --all --no-fail-fast` | `passed: 612, failed: 0`（608 基线 + 4 新测试） | PASS |
| T8 RED | `cargo test --test projection_test`（T8 实现前） | `FAILED. 4 passed; 2 failed`（(d) `[[null]]`、(e) 未排序全 schema 行） | RED 符合预期 |
| T8 GREEN | `cargo test --test projection_test` | `test result: ok. 6 passed; 0 failed` | PASS |
| 全量测试 | `cargo test --all --no-fail-fast`（fmt 后连续 4 轮） | `final: passed: 614, failed: 0`（608 基线 + projection 6 + 校准面 0） | PASS |
| Lint | `cargo clippy --all-targets -- -D warnings` | `Finished` 0 warning，退出码 0 | PASS |
| 格式 | `cargo fmt --check` | 无 diff，退出码 0 | PASS |
| OpenSpec | `openspec validate 2026-09-06-ms10-t01-cli-shell` | `Change ... is valid`，退出码 0 | PASS |

**Persisted Evidence**

None required（Plan Mode: none；全部验证命令低成本可重跑，本 Response 承载命令+决定性输出+退出码）

**Experience Candidates**

None

**Remaining Issues**

1. **[Minor，非回归] 别名/表达式投影回退恒等**：`SELECT name AS n`（别名）、`SELECT price*2`（表达式）因名字不可解析为 base 列而恒等回退（返回全 schema 行 + 全 schema 表头，与 change 前逐字节一致）。索引型投影（`Vec<usize>`）无法携带输出名映射；真别名投影需 (index, output_name) 对，属后续增强，不在本 Iteration 契约内。
2. **[Minor，非回归] 聚合 over SemiJoin/AntiJoin 且聚合参数列不在 select list** 的极端角落仍返回 Null（change 前同样 Null——统一后 column_indices 来自 select-list 过滤的 output_columns，参数列缺失仍不可达）。Plan 未列此形状，行为无回退。
3. **[Minor，环境] 瞬态单测试失败一次**：fmt 修复后首轮全量出现 1 个失败（未捕获名称），随后连续 4 轮全量 614/0 未复现；与 WSL2 并行负载噪声特征一致（Iteration 000 Risks 已注），无产品代码关联证据。
4. **[Minor] SortNode.columns 在聚合路径保持 `projection_columns`**（与 `get_plan_output_columns(Aggregate)` 内容等价但未统一）——保持聚合+排序行为逐字节不变的保守选择。

**Commit or Diff Reference**

- 工作树未提交（用户触发 commit；与仓库惯例一致）
- 基线：Iteration 000 完成态（608 tests，工作树未提交）；本 Cycle 变更面 16 文件（15 修改 + 1 新增 `tests/projection_test.rs`），另 Plan Context Status 行与 tasks.md T10 Status 为流程记录

## Plan Review

**Findings**

1. **[Minor，非阻塞] Invariant 3 对网络路径的表述不精确（Plan 与 Act Self-Review 同源）**：网络协议代码确实零触碰（join/dml/ddl/子查询/网络模块 diff 为空）；但引擎级真投影使子集投影的行形状经网络路径被动变化——PG 协议列名按 `rows[0].len()` 派生（`pg_protocol.rs:254-255`），表头 `col0..colN` 与行保持自洽。Plan Invariant 3 的原意是"协议代码无需改动"（design 影响边界未列网络面，Behavioral Change 明示"四路径统一"为引擎级目标语义），Act 实现与该意图一致；仅 Self-Review"网络协议可观察行为零变化"措辞过强。记录不改。
2. **[Minor，PLAN-OMISSION，已覆盖] 行产出点计数偏差**：Plan 概称 Scan/IndexScanAll"各一处"，实际各有 2 处（snapshot / no-snapshot 分支，diff 逐点核对）。Act 全部覆盖，无第五类产出点（T7 Stop 条件未触发），无 Acceptance gap。
3. **[Minor，NEW-EVIDENCE，已验证] 校准面为空**：Plan 预计"几十处"既有断言需校准，实测 T7/T8 落地后 `cargo test --all` 零既有失败——不存在"子集投影 + 全 schema 行"组合的既有用例。T9 契约纪律（失败项核对 + 主动排查）已履行；校准清单（空集 + ①②③）经 diff 独立核对完整：executor_test.rs 补 `projection` 绑定 + `[0,1]` 全列索引断言；pushdown_test.rs 仅注释修正（断言零变化）；cli_test ④⑥ 改回子集投影——⑥ 正是 Iteration 000 finding 1 的 IndexScan 表头错位场景，现断言 `name` 单列正确形状。
4. **[Minor，既有限制，非回归] Act Remaining Issues 1-4 维持不改**：别名/表达式投影恒等回退（索引型投影无法携带输出名映射，change 前行为逐字节保留）；聚合 over SemiJoin/AntiJoin 极端角落仍 Null（change 前同值）；一次瞬态测试失败未复现（Act 连续 4 轮 + 本 Review 独立重跑 2 轮全绿）；SortNode.columns 聚合路径保守保留。均不阻塞，不在本 Cycle 处理。

**Deviation Classification**

- Deviation 1（FilterNode/FilterExecutor 携带投影）= PLAN-OMISSION × 非实质：T7 目标清单未列 filter.rs，但 OR/AND-PK 形状（`Filter(DataScan)`）的谓词由包装 Filter 求值——若 scan 投影则谓词破坏、若不投影则该路径表头与行不一致；扩展是达成 T7 Required behavior"所有路径表头与行一致"的最小机制延伸，与 D8"投影在谓词后"同构（filter.rs 裁剪点在 `predicate.evaluate` 之后，已核对）。
- Deviation 2（最外层安全节点拥有裁剪 + 恒等回退）= ACT-DEVIATION，落在 D10 授权的实现取向空间内；`is_base_scan_chain` 门控（Having/JOIN 形状防御性回退恒等）+ 聚合/标量子查询/通配符/别名恒等回退保证既有形状零回归。已机制级核对：`SELECT *` 经 `extract_columns` 产出 `"*"` 字面量 → `resolve_projection_indices` 返回 None → 恒等（`src/parser/ast.rs:97`）。
- Deviation 3（builder `with_projection` 传递投影）= ACT-DEVIATION，满足契约"pipeline 构造 executor 时传递投影"；~40 处既有直接构造调用点零修改，与 MS08-T02 `with_prefetch` 项目先例一致。
- Deviation 4（聚合检测循环上移至 WHERE 前）= ACT-DEVIATION，diff 核对循环体逐字移动，`has_aggregates` 门控投影解析所需，无行为变化。
- Deviation 5（产出点 2+2 而非 1+1）= PLAN-OMISSION，非实质，全部覆盖（见 Findings 2）。
- Deviation 6（IndexScanNode.columns = 真实输出形状）= 契约内（D9 的落实），`planner_test::test_select_by_pk`（全列投影）零修改通过。
- Deviation 7（校准面 0 而非"几十处"）= NEW-EVIDENCE，非实质（见 Findings 3）。

**Acceptance Gaps**

None。B1-B7 逐项映射到新鲜验证（见 Evidence）：B1 projection_test (a)(b)(f) + `get_plan_output_columns` Filter/Sort 裁剪臂；B2 (c) 守卫 + `"*"`→None 机制级恒等；B3 (d) `[[20]]`；B4 (e) 投影外排序键正确排序；B5 校准清单完整记录；B6 cli_test 12 用例全绿（Iteration 000 成果零回退，`src/cli/` 零触碰）；B7 四命令干净。

**Convergence**

N/A（本 Cycle 首次 Review；无上一版 Acceptance Gaps 可比较）

**Evidence**

| 检查项 | 结果 |
|---|---|
| 变更面一致性 | PASS——`git diff --stat` 20 文件 = Iteration 000 已接受面 + 本 Iteration 16 文件声明面逐项对应，无计划外文件（.claude/docs/tasks.md、references/spec.md 为 change 前文档改动） |
| Invariant 1（谓词后裁剪） | PASS——逐 diff 核对五执行器全部裁剪点：data_scan 两处 `filter_row` 后 / filter `evaluate` 后 / index_scan 与 index_scan_all 可见性判定后 / sort 比较用输入形状、物化时裁剪；下推 `scan_cap` 计数发生在投影后的产出行上（LIMIT 语义正确） |
| Invariant 2（`SELECT *` 不变） | PASS——机制级恒等（`"*"` → None）+ projection_test (c) 守卫 + executor_test `[0,1]` 全列投影索引断言 |
| Invariant 3（JOIN/DML/DDL/子查询/网络零触碰） | PASS——上述模块 diff 为空；标量子查询经 `subquery_evals` 门控恒等回退；网络路径被动行形状变化见 Findings 1（按目标语义） |
| Invariant 4/5/6 | PASS——projection_test 6 测试锁定三症状（(a)(b)(f)/(d)/(e)）；`src/cli/` 零触碰、退出码体系零变化；校准清单完整（见 Findings 3） |
| planner 门控逻辑 | PASS——`has_aggregates`/`subquery_evals`→恒等；`sort_due`→Sort 持有裁剪且 `is_base_scan_chain` 门控；OR 路径内层 DataScan 显式恒等避免双重裁剪；qualified 列 `s.name` 经 `extract_columns` 小写化可正常进投影；重复列 `SELECT id, id`→`[0,0]` 合法 |
| 全量测试（新鲜重跑 ×2） | PASS——`cargo test --all --no-fail-fast` 57 套件 `TOTAL passed: 614, failed: 0`，退出码 0（两轮独立） |
| Lint / 格式 / OpenSpec（新鲜重跑） | PASS——`cargo clippy --all-targets -- -D warnings` 0 warning（退出码 0）；`cargo fmt --check` 0 diff（退出码 0）；`openspec validate 2026-09-06-ms10-t01-cli-shell` valid |

**Follow-up Decision**

None——B1-B7 全部满足，无阻塞 finding，无当前 Cycle 修复项；Findings 1-4 均为非阻塞记录（Findings 2/3 属 Plan 侧偏差，已由 Act 覆盖且不构成 gap）。

**Iteration Plan Update**

None（Iteration Map 不变：000 completed、001 随本 Review accepted 完成）

**Next Cycle**

None

**Next Iteration**

None（change 无剩余 Iteration；可交 `openspec-docs-maintainer` 按正常流程收尾归档，commit 由用户触发）

- Review Result: accepted（2026-09-06，独立实机验证后最后改写）
