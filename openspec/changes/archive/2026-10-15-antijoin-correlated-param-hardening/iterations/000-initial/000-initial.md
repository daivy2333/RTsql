# Iteration 000 / Cycle 000: AntiJoin 关联参数绑定期加固

## Plan Context

- Status: ready（2026-10-15 用户指令「开始实施」——Gate 2 逐项 PASS 已记录于下，计划获批转 ready）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1（correlated-subquery-cache MODIFIED「关联参数外层绑定失败显式报错」枚举扩至四执行器）；前 change `2026-10-15-subquery-alias-binding-and-error-prefix` T2 三执行器同模先例（错误类型/文案/上浆手法）；proposal 默认假设 A1–A3
- Excluded scope: proposal Non-goals 全项（`inject_correlated_values`/表达式 `inject_parameters` 未注入求值 NULL 面、计划器/管道、缓存键结构、新 SQLSTATE 分类、文档变更）

**Objective**

`AntiJoinExecutor::extract_param_values` 未命中外层列由静默丢弃改点名报错（`correlated parameter column not found in outer row: {列名}`，`ExecutionError`），先行单测 RED→GREEN；既有 NOT EXISTS/NOT IN 套件（含前 change 22 用例矩阵的 AntiJoin 面）零回归；全量 `cargo test` 零回归。

**Background**

前 change Plan Review Finding 1（2026-10-15 归档）：T2 加固三执行器时漏掉 AntiJoin 同构点（`filter_map` 静默丢弃，比 `unwrap_or(0)` 更隐蔽——参数未注入求值 NULL + 缓存键短项化）。用户指令「我们继续解决这个」批准修复并沿用 2026-10-05 先例不落账独立 Issue。

**Investigation Facts**

- Current Baseline: 源码 HEAD `05a8d79`（2026-10-15 子查询别名批收尾回填提交），rtsql 0.2.0。产品代码自前 change Act Gate 后零变更（`05a8d79` 仅文档），全量 2029 tests / 0 failures / 2 ignored 结论采信（前 change Iteration 000 Act 验证记录，M19 分批串行 + 对账 1999+22+3+5=2029 吻合）。
- Current-State Evidence（代码级 Confirmed，本会话独立重读 `anti_join.rs` 全文）：
  1. `src/executor/anti_join.rs:151-161` `extract_param_values`——`filter_map` 丢弃未命中参数，返回 `Vec<(String, Value)>`；唯一调用点 `:209`（ScanLeft 相关分支内、`:206` `if let (Some(right_plan), Some(database))` 块中——`right_plan`/`database` 为 None 时整块跳过不经过绑定期，测试构造必须给齐）。
  2. 调用点上下文：`:210-266` 缓存查/重建右计划——`param_values` 为缓存键（`:211` get / `:263` insert）并经 `inject_correlated_values(&plan, &param_values)`（`:230-232`）注入；缺项键的缓存误共享与参数未注入（NULL 求值）为静默丢弃的双重后果。
  3. 错误面先例（前 change T2，逐字采信）：`StorageError::ExecutionError(format!("correlated parameter column not found in outer row: {}", cp.outer_column))`；Display 无前缀（前 change T3 去 thiserror 前缀、pipeline 漏斗唯一赋予单层 `Execution error: `）；58000 兜底。
  4. 命中路径差异事实：AntiJoin 命中臂为 `left_row[idx].clone()` 直接索引（三处为 `row.get(idx).cloned().unwrap_or(Value::Null)`）——假设 A3 保持原样。
  5. 测试基建先例：`src/executor/mod.rs` 既有 `#[cfg(test)] StubRowExecutor`（前 change T2 新增，按行序列产出）；SemiJoin/ScalarSubqueryFilter/SubqueryEval 三处同构内联单测（各文件 `mod tests`）可直接比照；`JoinRelatedConfig` 字段集见 `semi_join.rs` T2 测试构造（left/right/conditions/projection/correlated_params/left_column_indices/right_column_indices/right_plan/database/snapshot/routed_tables）。
  6. 同类面审计：全库 `extract_param_values` 共四处（grep 确认），本任务后全部 `Result` 化收口；`inject_correlated_values`/`inject_parameters`/`set_parameter_value` 未注入求值 NULL 面为更深同类点（表达式层签名连锁），前 change T2 已审定为非目标。
- Code and Critical Path: planner 相关分类（前 change T1 已修）→ `AntiJoinNode.correlated_params` → `AntiJoinExecutor::next()` BuildRight（空相关参数哨兵态）→ ScanLeft → `extract_param_values`（本任务变更点）→ 缓存键 + `inject_correlated_values` → 右计划重建。既有覆盖：`tests/subquery_alias_binding_test.rs` 22 用例（NOT EXISTS/NOT IN 别名矩阵面）+ `tests/subquery_test.rs` + `tests/e2e_test.rs` E14。

**Implementation Guidance**

顺序：先行单测（RED）→ 加固（GREEN）。单测置于 `anti_join.rs` 文件尾 `#[cfg(test)] mod tests`（文件当前无测试模块，新设；先例同构于 `semi_join.rs` T2 测试）。构造：`JoinRelatedConfig { left: StubRowExecutor::new(vec![vec![Value::Int(7), Value::Int(9)]]), right: StubRowExecutor::new(Vec::new()), conditions: Vec::new()（NOT EXISTS 形态）, projection: Vec::new(), correlated_params: vec![CorrelatedParam::new("o".to_string(), "missing_col".to_string(), "o.missing_col".to_string())], left_column_indices: HashMap::from([("id", 0usize), ("ref", 1usize)]), right_column_indices: HashMap::new(), right_plan: Some(PhysicalPlan::SingleRow), database: Some(Arc::new(Database::open_in_memory().await.unwrap())), snapshot: None, routed_tables: Arc::new(crate::pipeline::RoutedTables::new()) }`；断言 `exec.next().await` 为 `Err` 且 `to_string()` 含 `missing_col`、`expect_err("绑参未命中应报错而非静默丢弃该参数")`。加固本体：`filter_map`→`map` + `ok_or_else`（同款消息），签名 `-> Result<Vec<(String, Value)>>`，`:209` 调用点加 `?`；命中臂表达式逐字节不动。A1/A2/A3 默认值已闭合，无需 Act 决策。

**Task Contracts**

### T1: AntiJoin 绑定期加固 + 先行单测

- Requirement/Scenario: R1（delta MODIFIED）；scenario「外层缺列时点名报错」+「正常相关查询绑定期命中」（既有套件承载）
- Depends on: None
- Targets: `src/executor/anti_join.rs::AntiJoinExecutor::extract_param_values`（含唯一调用点 `:209` 上浆）
- Current behavior: `outer_column` 未命中 `left_column_indices` 时 `filter_map` 静默丢弃该参数——`param_values` 缺项注入右计划（未注入参数求值 NULL）且 `correlated_rows_cache` 键短项化（不同外层行可误共享缓存项）；当前代码下先行测试 RED（静默空集/行产出，无报错）
- Required behavior: 未命中返回 `StorageError::ExecutionError("correlated parameter column not found in outer row: {列名}")`，经 `next()` 传播，无数据行产出；命中路径 `(param_name, left_row[idx].clone())` 逐项与顺序逐字节不变；空 `correlated_params` 路径（BuildRight/独立子查询分支）不受影响
- Preserve: 命中路径行为与缓存键结构（完整 param 集合为键）；NOT EXISTS / NOT IN 全部既有语义（独立与相关）；`JoinRelatedConfig` 构造面与 `AntiJoinExecutor::new` 签名；前 change T2 三执行器错误文案与类型
- Forbidden: 不改 `inject_correlated_values` / 表达式层 `inject_parameters` / `set_parameter_value`；不改 pipeline / 计划器；不改缓存键结构；不新增 SQLSTATE 分类；不统一四执行器命中路径索引语义（A3）
- First test: `anti_join.rs` 文件尾新增 `#[cfg(test)] mod tests`（构造见 Implementation Guidance）；当前代码下 `cargo test --lib executor` 该用例 RED（`next()` 返回 `Ok(None)`/行而非 `Err`）
- GREEN condition: 该单测绿 + `cargo test --lib` 全绿 + `cargo test --test subquery_alias_binding_test --test subquery_test` 零回归
- Verification: `cargo test --lib executor` 定向（单测 + 既有 lib）；`cargo test --test subquery_alias_binding_test --test subquery_test` 零回归；Iteration Gate 全量一次
- Stop when: 命中路径出现任何行为变化（返回 Plan）；既有 NOT EXISTS/NOT IN 用例失败（说明变更面越界，返回 Plan）

**Invariants**

- 全量既有测试零修改通过；NOT EXISTS / NOT IN 相关与独立形态语义逐字节不变；错误类型与消息同模前 change T2；退出码矩阵不变。

**Non-goals**

proposal Non-goals 全项；本 Cycle 不关闭任何 issue（无独立 Issue）；不动文档。

**Acceptance**

- R1：AntiJoin 纳入「关联参数外层绑定失败显式报错」执行器枚举（delta MODIFIED 合并后语料与实现一致）；先行单测 RED→GREEN（错误点名未命中外层列、无数据行）；命中路径零变化由 `subquery_alias_binding_test`（22 用例含 NOT EXISTS/NOT IN 别名矩阵）与 `subquery_test` 零回归承载。
- RTM：

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 绑定期未命中点名报错（枚举扩至 AntiJoin） | T1 | `anti_join.rs` 内联 `missing_outer_column_reports_error_naming_column`（RED 先行） | Covered |
| R1 命中路径等价锚 | T1 | 既有 `tests/subquery_alias_binding_test.rs` 22 用例零回归 | Covered |

- 全量 `cargo test`（2029 基线 + 1）零失败为 Iteration Gate。

**Verification**

逐 scenario 直接判定：单测与集成命令真实运行；无跨接口新 scenario（R1 为防御路径直构判定，既有 NOT EXISTS/NOT IN SQL 面已由前 change 矩阵覆盖且本变更不触及命中路径）；全量一次为 Iteration Gate 时机，禁止任务内重复跑全量。

**Gate 2 Readiness**

PASS（逐项：BDD 缺口扫描完成——Happy〔相关 NOT EXISTS/NOT IN 执行，既有套件〕/Sad〔外层缺列点名报错，先行单测〕/Edge〔空 correlated_params 不走变更点、多参数首个未命中即报错——map+? 短路语义，A1 同模〕/组合〔缓存键短项化消除——错误先于缓存交互〕/兼容〔错误面收窄、退出码与 SQLSTATE 不变〕；默认假设 A1–A3 闭合；场景草图 = delta spec 两 Scenario；调查来自本会话对 `anti_join.rs` 全文的独立重读与 grep 同类面审计，基线采信未失效结论；Task Contract 含变更面/先行测试/GREEN/验证/停止条件；调用者与测试入口已定位〔唯一调用点 :209、内联测试先例三处〕；单 Iteration 单任务通过平衡审计；Persisted Evidence = none——单条定向命令可低成本重跑；RTM 无 Missing/Simplified）。

**Persisted Evidence**

- Mode: none

全部验收经 `cargo test` 定向命令与全量一次可低成本重跑；Act Response 足以承载验证结论；无一次性环境或不可复现现场。

**Risks and Notes**

- 风险：无实质未知项。非实质：AntiJoin 命中臂直接索引与三处 `.get(idx)` 回退的差异按 A3 保持（行短于索引表时行为不同属历史现状，本 change 不统一）。
- 测试构造注意（防测试落空）：`right_plan`/`database` 必须给 `Some`，否则 `:206` 守卫跳过相关分支、不经过被测路径。

## Act Response

- Status: reported

### 已实施

| Task | 改动 | 文件 / 符号 |
|---|---|---|
| T1 | `AntiJoinExecutor::extract_param_values` 由 `filter_map` 静默丢弃改为 `Result`：`map` + `.copied().ok_or_else(...)`，未命中外层列报错 `correlated parameter column not found in outer row: {列名}`（`crate::storage::StorageError::ExecutionError`），签名改 `-> Result<Vec<(String, Value)>>` | `src/executor/anti_join.rs:151-175` |
| T1 | 唯一调用点 `ScanLeft` 相关分支内 `let param_values = self.extract_param_values(&left_row)?;` 上浆（`:222`） | `src/executor/anti_join.rs:222` |
| T1 | 先行单测 `missing_outer_column_reports_error_naming_column`：`#[cfg(test)] mod tests` 新设于文件尾，直构 `AntiJoinExecutor`（`JoinRelatedConfig` + 既有 `StubRowExecutor`），`right_plan: Some(PhysicalPlan::SingleRow)` + `database: Some(内存库)`，`conditions` 取空（NOT EXISTS 形态），断言 `next()` 返回 `Err` 且消息点名 `missing_col` | `src/executor/anti_join.rs:333-386` |

命中路径 `Ok((cp.param_name.clone(), left_row[idx].clone()))` 逐字节保持（含直接索引语义，A3 未统一）；缓存键结构、退出码、SQLSTATE（58000 兜底）零变化。Forbidden 项（`inject_correlated_values` / 表达式层 `inject_parameters` / `set_parameter_value` / pipeline / 计划器 / 缓存键 / 文档）零触碰。

TDD 顺序与 RED 见证：`cargo test --lib executor` 在产品代码未改时 `80 passed; 1 failed`——panic 信息 `绑参未命中应报错而非静默丢弃该参数: None`，即 `next()` 返回 `Ok(None)`（静默空集路径，与 Plan 预测逐字一致）；加固后同命令 `81 passed; 0 failed`。

### Deviations from Plan

- 加固体形态：Implementation Guidance 给的是 `.map(|&idx| …).ok_or_else(…)` 闭包内联式，实现改用前 change T2 `semi_join.rs` 的 `let idx = …copied().ok_or_else(…)?; Ok((…))` 同款写法——A1 要求与前三执行器逐字一致，取先例原文更稳，语义等价。
- 先行测试构造与 Plan 给定一致（`right_plan`/`database` 均 `Some`），按 Risks and Notes 的防落空提示执行。

无实质偏差。

### 自检发现

- **测试不会因错误原因通过**：`right_plan`/`database` 若给 `None`，相关分支整体跳过、`right_has_rows` 保持 `false`，NOT EXISTS 形态会产出左行 `Ok(Some(Row))`；RED 实际观察到的是 `None`（静默空集），反证测试确实进入了被测分支。
- `touch src/executor/anti_join.rs && cargo build --lib` 强制重编译：本次改动零编译警告（输出中仅有环境侧 `~/.cargo/config` 弃用提示，与本改动无关）。
- 遗留 Minor：无。未解决 Critical / Important：无。

### 验证记录

| 验证 | 命令 | 结果 |
|---|---|---|
| T1 RED | `cargo test --lib executor` | `80 passed; 1 failed`（`expect_err` panic 于 `anti_join.rs:366`：`…: None`） |
| T1 GREEN | 同上 | `81 passed; 0 failed` |
| lib 全量（定向目标） | `cargo test --lib` | `407 passed; 0 failed` |
| 零回归（契约指定套件） | `cargo test --test subquery_alias_binding_test --test subquery_test` | `22 passed` / `43 passed`，均 0 failed |
| Iteration Gate 全量（M19 分批串行：lib 1 批 + 123 集成目标 13 批，字典序、10 目标/批、每批 `--test-threads=1`、批间顺序执行同一时刻只跑一批） | 逐批 `cargo test --lib -- --test-threads=1` / `cargo test --test ×10 -- --test-threads=1` | 各批均 0 failed；lib 407 + 集成 1623 = **2030 tests / 0 failed / 2 ignored**（2029 基线 +1，净增即本 Cycle 新增单测，对账吻合） |

### Persisted Evidence

- None required（Plan 定 Mode `none`，Act 复核维持：全部验收经定向命令与分批全量可低成本重跑，无一次性环境或不可复现现场）。

### Experience Candidates

None——本次无可复用的构建/运行流程，无当前 change 范围外且有证据的实质缺陷（表达式层「未注入参数求值 NULL」面为前 change 已审计在案的设计内非目标，非本次新发现）。

### 未解决问题

None。

## Plan Review

- Review Result: accepted

**Findings**

1. **[信息，已处理]** 本会话存在并行写入重叠：用户「进行审计」指令下达时，另一执行体已按用户「开始实施」指令完成 T1 全部实施与验证（本 Act Response 即其产物）；本会话重建先行测试时产生了一个重复的 `mod tests` 块（编译冲突），Review 中发现并已删除（`anti_join.rs` 回到 386 行、单一测试模块、fmt 干净）。除该重叠外，本会话对产品代码零改动。
2. **[Minor｜Act 偏离，已记录]** 加固体写法取 `semi_join.rs` T2 的 `let idx = …copied().ok_or_else(…)?; Ok((…))` 形态，而非 Implementation Guidance 的闭包内联式——语义等价、与 A1「与前三执行器逐字一致」更贴合，Act 已在 Deviations 记录。不阻塞。

无实质 finding。无 Acceptance Gap。

**Evidence**

- 独立 diff 阅读（`git diff src/executor/anti_join.rs`）：变更面与 Task Contract 逐字一致——`extract_param_values` `filter_map`→`map` + `ok_or_else`（同款消息 `correlated parameter column not found in outer row: {列名}`，`StorageError::ExecutionError`），唯一调用点 `?` 上浆，命中臂 `left_row[idx].clone()` 直接索引逐字节保持（A3）；新增 `#[cfg(test)] mod tests` 一个（防落空注记齐备：`right_plan`/`database` 必须 `Some`）。
- 采信 Act 验证记录（Review 现场清理后工作区与 Act Gate 运行态逐字节一致）：T1 RED `80 passed; 1 failed`（`Ok(None)` 静默空集，与 Plan 预测逐字一致）→ GREEN `81 passed`；lib 全量 `407 passed`；Iteration Gate `2030 tests / 0 failed / 2 ignored`（2029 基线 + 本单测 1，对账吻合）。
- 本会话重跑复核：`cargo test --lib executor` 81/0（清理后）；`cargo test --test subquery_alias_binding_test --test subquery_test` 22/43 全绿；`cargo fmt --check` 干净。
- 不变量核查：空 `correlated_params` 路径不经变更点（BuildRight 分支独立子查询形态不受影响）；缓存键结构未动（`:224` get / `:276` insert 同键类型）；Forbidden 项零触碰（diff 仅 anti_join.rs，git status 确认无其他产品文件改动）。

**Follow-up Decision**

R1 满足：AntiJoin 绑定期未命中点名报错（先行单测 RED→GREEN + 错误点名 `missing_col` + 无数据行）；命中路径零变化由 22 用例矩阵与 `subquery_test` 43 用例零回归承载；四执行器 `extract_param_values` 全库收口。两个 finding 均不阻塞。无 Acceptance Gap，无需 rework / replan。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

None（单 Iteration change，Iteration 000 完成）。

