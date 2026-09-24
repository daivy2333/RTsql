# Iteration 002 / Cycle 000: 关联子查询结果缓存（T20-T22）

## Plan Context

- Status: ready
- Iteration: 002-subquery-cache
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T20-T22（tasks.md Iteration 002「关联子查询结果缓存」）
- Depends on: Iteration 000（快照穿线面已落地并 accepted——002-rework；执行序在 001 后，001-nlj 已 accepted）
- Stable baseline: 相同关联参数值序列单语句内至多执行一次子查询（代码审查承载）；缓存命中与直执行结果逐字节等价；跨语句无残留；错误语义不变；非关联路径逐字节不变；全量零回归
- Verification boundary: `tests/subquery_test.rs` 新增等价见证用例全绿 + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: 子查询执行器族（`subquery_eval.rs`/`semi_join.rs`/`anti_join.rs` 缓存面）
- Deferred tasks: None（本 Iteration 为 change 最后一个 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: correlated-subquery-cache 全部 Requirement（R1-R5）；design D6 处方（键 = `extract_param_values` 产物、值 = 完整行集、语句生命周期由执行器每语句重建保证、错误不缓存、无计数机制）；Iteration 000 的快照穿线面（`snapshot: Option<Snapshot>` 字段族）；Iteration 001 accepted 的全量绿面 928/0/2 作为回归基线
- Excluded scope: 非关联子查询改造（既有 `cached_result` 保持）；多层关联（I018 已归档）；跨语句/跨连接缓存；LRU 与淘汰策略（D6 修正 DA1——语句界有界，HashMap 足够）；test-only 计数 hooks（身份型证据工程禁令）；子查询 planner 面（本 Iteration 零触及 planner）

**Objective**

三个关联消费面（标量 `SubqueryEvalExecutor`、`SemiJoinExecutorV2`、`AntiJoinExecutor`）获得语句级关联参数缓存——相同参数值序列的重复求值不再重建右计划重复执行；等价见证用例（重复参数值一致性、相异值隔离、NULL 键、缓存≈直执行、错误面、跨语句新鲜度）在实施前后双 GREEN；全量 928 基线零回归。

**Background**

需求来源：tasks.md MS09-T04（I017）+ proposal 决策记录 3（用户指令「把 MS09 规划成一个 change」纳入 T04）+ DA1 默认假设（D6 修正为语句界 HashMap）。现状：关联子查询每外层行 `subquery_plan.clone()` → 注入 → `create_executor_from_plan` 重建执行器 → 全量执行——N 行外层对相同参数值重复执行子查询（I017）。本 Cycle 是 Iteration 002 的首个执行 Cycle，按 design D6 实施。

**Investigation Facts**

- Current Baseline: HEAD `e51c4a3` + 工作区（Iteration 000 三 Cycle + Iteration 001 两 Cycle 实施改动全部在工作区；代码面自 001-rework 收尾后未变化）。全量 **928 passed / 0 failed / 2 ignored**（001-rework Plan Review 本会话独立复跑，73 个测试二进制 exit 0）；clippy 0 / fmt OK / validate 28 PASS（同会话）。
- Current-State Evidence（Plan 直接读码核实，file:line 为当前工作区现状）：
  - **键类型可直接作 HashMap 键（D6 事实核实成立）**：`Value` derive `Debug, Clone, PartialEq`（`src/executor/value.rs:45`）+ 手工 `impl Eq for Value`（`:61`）与 `impl Hash for Value`（`:65`，discriminant 区分变体 + Float 经 `to_bits`）——`Vec<(String, Value)>` 满足 `Clone + Eq + Hash`；该 Eq/Hash 语义与既有 Semi/Anti Hash Join 的 `Vec<Value>` 键同源（同一 impl 已在生产路径使用）。
  - **SubqueryEvalExecutor**（`src/executor/subquery_eval.rs`）：字段 `correlated_params`/`outer_column_indices`/`database`/`snapshot: Option<Snapshot>`（:32，MS09 Iter000 D4 穿线）/`cached_result: Option<Value>`（:33，非关联臂 :160-164 消费——保持）；关联臂 `next()` :124-157——逐行 `subquery_plan.clone()`（:126）→ `extract_param_values(&row)`（:127；:99-114，返回 `Vec<(String, Value)>`，Clone+Eq+Hash ✓）→ `inject_correlated_values`（:128）→ `create_executor_from_plan`（:129-135，`self.snapshot.clone()` 穿线）→ drain 求标量（:136-157）——`row_count > 1` 即时 `Err(SubqueryReturnsMultipleRow)`（:142-149，早退形态）；`!inner_row.is_empty()` 才取 `row[0]`（:150-152）；0 行 → `Null`（:157）。
  - **SemiJoinExecutorV2 关联臂**（`src/executor/semi_join.rs`）：BuildRight 臂关联时置空 `right_hashmap`/`right_has_rows`（:188-198）；ScanLeft 逐左行重建（:204 区域）——`extract_param_values(&left_row)`（:141-150 同型实现）→ inject → `create_executor_from_plan`（snapshot 穿线）→ drain 逐行 `build_right_key(&row)`：`Some(key)` 入 hashmap、`None`（NULL 键行）不入但 `has_rows` 仍置真（:215-223 区域）→ 探测产出。**`has_rows` 语义含 NULL 键行**——EXISTS 模式 `if has_rows` 依赖之。
  - **AntiJoinExecutor 关联臂**（`src/executor/anti_join.rs`）：同型（:185-218 区域）——NOT EXISTS 模式 `if !has_rows` 产出；重建循环与 Semi 逐字同构。
  - **JoinRelatedConfig**（`src/executor/join_related_config.rs:10-23`）：`right_plan: Option<PhysicalPlan>`（仅关联时 Some）、`database`/`snapshot: Option<Snapshot>`——缓存字段加在执行器本体（非 config）。
  - **构造面（语句生命周期机制核实成立）**：`create_executor_from_plan` Semi 臂（`src/pipeline.rs:685`）/ Anti 臂（:716）/ SubqueryEval 臂（:746）——每语句执行新建执行器；PlanCache 缓存的是 plan 不是执行器，执行器不跨语句复用 → 执行器本地缓存字段天然为语句生命周期（D6「每语句重建构造面保证」成立）。
  - **快照语义支撑**：同一执行器实例内每行复用同一 `self.snapshot.clone()`（三臂同型）→ 语句执行内读取面一致 → 缓存命中无可见性偏差（correlated-subquery-cache R4「语句执行内的数据不变性由快照语义保证」的机制载体）；语句间新执行器新快照 → 跨语句新鲜度天然成立。
  - **测试入口**：`tests/subquery_test.rs` e2e 风格（helpers `open_db`/`exec`/`rows`/`error_msg`/`setup_emp_dept` :8-66）；既有关联用例 `test_correlated_where_in_basic`（:323）/`test_correlated_scalar_subquery`（:339）/`test_correlated_exists`（:358）等 43 个测试函数；TDD 类别：本 Iteration 为**重构类**（等价性 change）——新增用例断言的目标行为在当前无缓存实现上即成立（重复执行本就产出正确结果），测试见证为「实施前 GREEN + 实施后 GREEN」双观察（公共规则 › TDD 重构纪律），无 RED 面（执行次数不作可观察契约，无计数机制）。
- Code and Critical Path: `subquery_eval.rs` 关联臂 + 新缓存字段；`semi_join.rs`/`anti_join.rs` 关联重建臂 + 新缓存字段；错误面（`SubqueryReturnsMultipleRow` 等）与公共接口零变化。数据流：外层行 → `extract_param_values` → 缓存查询 → 命中派生结果 / 未命中 inject+执行+存储（成功后）。

**Implementation Guidance**

实施顺序：T22 前半（等价见证用例先行，在当前无缓存实现上观察 GREEN——重构类纪律）→ T20 → T21 → T22 后半（收尾门）。要点：(1) 缓存字段为执行器私有 `HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`（D6），关联时才使用；(2) `subquery_eval` 值 = 成功行集（0 或 1 行——多行 Err 早退不缓存），命中时标量派生与 miss 路径同语义（`rows.len()==1 && !rows[0].is_empty() → rows[0][0].clone()`，否则 `Null`）；(3) Semi/Anti 值 = **完整右行集（含 `build_right_key=None` 的 NULL 键行）**，命中时从行集重建 `right_hashmap`（仅 `Some(key)` 行）+ `right_has_rows`（= 行集非空）——与 miss 路径的逐行循环同语义，保住 EXISTS/NOT EXISTS 的 `has_rows` 行；(4) 错误路径（`Err` 与多行早退）即时传播、不写入缓存；(5) 缓存查询在 inject/执行之前、存储在成功 drain 之后。

**Behavioral Change**

- 当前：关联子查询每外层行重建右计划并完整执行（相同参数值重复执行，I017）；非关联臂已有缓存。
- 目标：语句级缓存——相同参数值序列单语句内至多执行一次；命中结果与直执行逐字节等价（行集、行序、列形状）；错误按既有语义传播且不被缓存；NULL 参数值为键合法成分（Null==Null 命中，与任何非 Null 值区分）；非关联臂与非关联 Semi/Anti 快路径逐字节不变。
- 接口/错误/状态语义：无公共接口变化；`SubqueryReturnsMultipleRow` 等错误类型与文案逐字节保持；执行次数不作为可观察契约（不建立计数机制——D6 纪律）。

**Task Contracts**

### T20: SubqueryEvalExecutor 关联臂语句级缓存

- Requirement/Scenario: correlated-subquery-cache R1（相同参数值复用）/ R2（不同值独立、NULL 键）/ R3（等价 + 错误不缓存）/ R4（语句生命周期）
- Depends on: T22 前半（等价见证用例在位且观察 GREEN）
- Targets: `src/executor/subquery_eval.rs::SubqueryEvalExecutor`（新字段 + `next()` 关联臂 :124-157）
- Current behavior: 每外层行 clone→inject→重建执行器→执行（无缓存）
- Required behavior: 关联臂先查 `correlated_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`——命中从缓存行集派生标量（不重建执行器）；未命中按现路径执行，成功 drain 后存储行集；`Err` 与多行早退即时传播且不缓存
- Required changes: 新私有字段 + `new()` 初始化 + 关联臂查/存逻辑
- Preserve: 非关联臂 `cached_result` 路径（:158-165）逐字节不变；`extract_param_values`（:99-114）不变；标量派生语义（空行跳过 / 0 行 Null / 多行 Err 文案）不变；`snapshot` 穿线不变；`result_column_index` 插入逻辑不变
- Forbidden: 计数 hooks 或任何执行次数可观测机制；跨语句存续（static/global/连接级结构）；缓存错误对象；LRU/淘汰策略；改动 plan cache 或执行器复用方式；触碰 planner
- Test witness: T22 等价见证用例实施前 GREEN（当前行为）→ 实施后保持 GREEN
- GREEN condition: `cargo test --test subquery_test` 全绿 + 全量零回归
- Verification: `cargo test --test subquery_test` + `cargo test --no-fail-fast`
- Stop when: 缓存命中语义需要语句内可见性失效协议（返回 Plan——当前快照语义已保证语句内读取面一致，不应发生）

### T21: Semi/Anti 关联臂同型缓存

- Requirement/Scenario: correlated-subquery-cache R5（Semi/Anti 同型 + 既有零回归）/ R1-R4（同 T20 语义）
- Depends on: T22 前半（见证在位）；与 T20 无代码耦合（不同执行器）
- Targets: `src/executor/semi_join.rs::SemiJoinExecutorV2`、`src/executor/anti_join.rs::AntiJoinExecutor`（各新字段 + 关联重建臂）
- Current behavior: 每左行重建右计划并逐行构建 hashmap（`build_right_key` Some 入表 / has_rows 对任何 Row 置真）
- Required behavior: 重建前查缓存——命中从缓存行集重建 `right_hashmap`（仅 Some(key) 行）+ `right_has_rows`（行集非空，含 NULL 键行语义）；未命中按现路径执行，成功后存储完整行集（含 NULL 键行）；错误即时传播不缓存
- Required changes: 两执行器各新私有字段 + 构造初始化 + 关联重建臂查/存逻辑
- Preserve: 非关联快路径（BuildRight 一次性物化）逐字节不变；`build_right_key`/探测/`build_output_row` 不变；EXISTS `if has_rows` / NOT EXISTS `if !has_rows` 语义不变（NULL 键行计入 has_rows）；`JoinRelatedConfig` 结构零改动；snapshot 穿线不变
- Forbidden: 同 T20 Forbidden；不得只缓存 hashmap 而丢失 NULL 键行的 has_rows 语义
- Test witness: T22 等价见证用例（EXISTS/NOT EXISTS 重复参数值场景）实施前 GREEN → 实施后保持 GREEN
- GREEN condition: `cargo test --test subquery_test` 全绿 + 全量零回归
- Verification: `cargo test --test subquery_test` + `cargo test --no-fail-fast`
- Stop when: 命中重建无法与 miss 构建语义等价（返回 Plan）

### T22: 等价见证用例 + GREEN 收尾

- Requirement/Scenario: correlated-subquery-cache R1-S1/R2 全/R3 全/R4-S1/R5-S1
- Depends on: None（前半先行）；后半依赖 T20/T21
- Targets: `tests/subquery_test.rs` 扩展（复用既有 helpers）
- Current behavior: 新用例缺失
- Required behavior: 新增用例（e2e，经既有 helper）：(1) `重复参数值结果一致`——外层多行关联参数值相同，各行标量/EXISTS 结果一致且与单行直执行一致（R1-S1）；(2) `相异参数值不串结果`——参数 1/2 各得各自结果（R2-S1 等价锚点）；(3) `NULL 参数值键语义一致`——参数为 NULL 的行结果一致（R2 NULL 成分）；(4) `缓存与直执行逐字节一致`——含重复值的查询结果集与直执行参照一致（R3-S1）；(5) `子查询错误不被缓存改变`——关联标量子查询对某参数值多行 → `SubqueryReturnsMultipleRow` 错误文案不变（R3-S2）；(6) `跨语句不残留`——语句 1 查询 → 语句 2 修改数据 → 语句 3 同查询按新数据求值（R4-S1）；(7) Semi/Anti 关联面——EXISTS/NOT EXISTS 重复参数值与相异值行集正确（T21 面）
- Required changes: 仅新增用例与必要 helper 扩展；GREEN 收尾（T20/T21 落地后全量 + 工具链门）
- Preserve: 既有 43 用例零修改；断言语义不放宽
- Forbidden: 计数 hooks；执行次数断言；为错误场景放宽/收窄既有错误面断言
- Test witness: 前半——新用例在 T20/T21 实施前运行观察 **GREEN**（重构类双 GREEN 之第一观察，输出录入 Act Response）；后半——实施后同套件保持 GREEN
- GREEN condition: `cargo test --test subquery_test` 全绿（既有 + 新增）+ 全量 ≥928+新增 / 0 failed / 2 ignored + clippy/fmt/validate 全 0/PASS
- Verification: `cargo test --test subquery_test` / `cargo test --no-fail-fast` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- Stop when: 新用例在当前实现上即失败（说明等价性前提断裂，返回 Plan）

**Invariants**

- 非关联子查询面（`cached_result`、Semi/Anti 非关联快路径）逐字节不变。
- 错误面逐字节不变（类型与文案）；错误与多行早退永不入缓存。
- 快照/隔离语义零触及（Iteration 000 面消费不修改）；页格式/WAL 零触及。
- 执行器每语句重建 → 缓存天然语句生命周期；禁止任何跨语句存续结构。
- 无身份型证据工程；无计数 hooks；`Value` 的 Eq/Hash impl（value.rs:61/:65）零改动。
- 全量基线（928/0/2）零回归。

**Non-goals**

- 非关联子查询与 `DerivedScan` 物化路径；多层关联（I018 归档）；跨语句/事务/连接缓存与失效协议；LRU 淘汰（lru 依赖维持未用）；子查询 planner/注入机制（`inject_correlated_values` 零触及）；性能优化（MS08 域）；I041 测试竞态（独立小 change）。

**Acceptance**

1. 等价见证用例集（`tests/subquery_test.rs` 新增 ≥7 用例）实施前后双 GREEN——映射 R1-S1/R2 全/R3 全/R4-S1。
2. Semi/Anti 关联面缓存等价（用例 7 + 既有 IN/EXISTS 用例零回归）——映射 R5。
3. 全量回归零修改通过（0 failed / 2 ignored，passed ≥ 928 + 新增数）——映射 R5-S1。
4. clippy/fmt/validate 全 0/PASS。
5. 「相同参数值至多执行一次」由代码审查承载（Self-Review 逐臂核对：查缓存在 inject/执行前、存缓存仅在成功 drain 后）——映射 R1（D6 纪律，无计数机制）。
Acceptance 与 requirement/scenario/design/task/代码/测试映射见 tasks.md RTM（correlated-subquery-cache 5 Requirement 全 Covered；design 依据 D6）。

**Verification**

- `cargo test --test subquery_test`（既有 43 + 新增，前/后双观察）
- `cargo test --no-fail-fast`（全量，零回归门）
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- 决定性输出各取 ≤20 行写入 Act Response。

**Gate 2 Readiness**

- 无 Missing requirement（RTM 全 Covered，映射不变）：PASS（tasks.md RTM correlated-subquery-cache 5 Requirement 全 Covered）
- 无 Simplified 未批准：PASS（无需求裁剪；DA1 的 LRU→语句界 HashMap 为 D6 已记录的设计修正，随 change 既有批准面）
- 调查完整（三执行器关联臂、键类型 Eq/Hash、构造面语句生命周期、快照语义支撑、测试入口均有 file:line 证据；基线 928/0/2 本会话独立复跑）：PASS（Investigation Facts）
- 设计闭合（缓存结构、命中派生语义、错误路径、NULL 键语义、生命周期机制均已定稿，无 TBD）：PASS（design D6 + Implementation Guidance）
- 任务可执行（T20/T21/T22 各有 Targets/行为变化/测试见证/停止条件）：PASS
- 分轮合理（单 Iteration 单故障域子查询执行器族，T20-T22 一 Cycle 承载）：PASS（tasks.md 平衡审计原文）
- 追踪完整（Acceptance 映射 RTM 不变）：PASS
- 验证充分（等价见证双 GREEN + 全量零回归 + 工具链门）：PASS
- 无身份型证据工程：PASS（执行次数由代码审查承载，Forbidden 计数机制——D6 纪律）
- 无实质未知项留 Act：PASS（非实质留白见 Implementation Guidance 末句与 Risks）
- tasks/specs/design/Iteration/Cycle 一致：PASS（spec/design 零变更；tasks.md Map 不变）
- Persisted Evidence 模式明确：PASS（none，见下）
- 用户批准计划：PASS（2026-09-14 用户批准「批准」——批准内容含本计划要点：三执行器关联臂语句级缓存、错误不缓存、非关联路径逐字节不变、重构类双 GREEN 见证、全量 ≥928+新增 零回归）

**Persisted Evidence**

- Mode: none

理由：全部验收由可重复运行的测试与工具链命令承载；无不可低成本复现的现场。

- Budget: 不适用（none）

**Risks and Notes**

- `Value` Eq/Hash 的 NaN/±0.0 边缘：与既有 Semi/Anti Hash Join 键语义同源（同一 impl 生产在用）；缓存最坏退化为 miss（重复执行，结果仍正确），不产生错误共享——非实质。
- 全量已知 I041 resolve env 竞态假失败（约 1/6）——若遇单点失败按已知 flaky 重跑处置（001-rework 先例），决定性聚合取干净运行。
- 非实质留白（Act 自行决定）：缓存字段命名、命中重建的 helper 组织、缓存行集存储与派生的具体写法、用例内表/数据具体值组合。

## Act Response

- Status: reported

**Implemented**

- T22 前半（等价见证先行）：`tests/subquery_test.rs` 新增 8 个见证用例（≥7 达标）+ `name_value_map` helper，在当前无缓存实现上观察第一 GREEN（28 passed / 0 failed，含既有 20）。用例映射：R1-S1 重复参数值一致 / R2-S1 相异值隔离 / R2 NULL 参数键身份 / R3-S1 重复执行逐字节一致 + 直执行参照 / R3-S2 错误文案逐字节 / R4-S1 跨语句新鲜度 / T21 Semi 面（EXISTS+IN 重复参数行集）/ T21 Anti 面（NOT EXISTS+NOT IN 行集）。
- T20：`SubqueryEvalExecutor` 新增私有字段 `correlated_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`（`new()` 初始化空）；关联臂重写为：`extract_param_values` → 缓存查询（命中从缓存行集派生标量：`rows.first()` 非空取 `rows[0][0]`，否则 `Null`——与 miss 派生逐语义等价）→ 未命中按现路径 clone/inject/重建执行器/drain（保留 row_count>1 早退 Err 于原位置）→ **成功 drain 后**存储行集（0 或 1 行；错误与多行早退永不入缓存）。
- T21：`SemiJoinExecutorV2`/`AntiJoinExecutor` 各新增私有字段 `correlated_rows_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`（`new()` 初始化）；关联重建臂改为：查缓存——命中从缓存完整行集重建 `right_hashmap`（仅 `build_right_key()==Some` 行，行序保持）+ `right_has_rows = !rows.is_empty()`（**NULL 键行计入 has_rows**，EXISTS/NOT EXISTS 语义保真）；未命中按现路径执行并收集完整行集（含 NULL 键行），成功 drain 后存储；错误经既有 `?` 传播不入缓存。
- 语句生命周期机制：执行器每语句重建（pipeline 构造面零改动），缓存字段为执行器本地状态，天然语句界；无 static/global/连接级结构、无计数 hooks、无 LRU。

**Changed Files and Symbols**

- `src/executor/subquery_eval.rs` — `SubqueryEvalExecutor`：新字段 `correlated_cache` + `new()` 初始化 + `next()` 关联臂查/存逻辑 + 模块与结构体 doc 更新；`extract_param_values`/`eval_subquery`（非关联臂）/`cached_result` 路径零改动。
- `src/executor/semi_join.rs` — `SemiJoinExecutorV2`：新字段 `correlated_rows_cache` + `new()` 初始化 + `ScanLeft` 关联重建臂命中/未命中分支；非关联快路径/`build_right_key`/`build_left_key`/`build_output_row`/探测逻辑零改动；`JoinRelatedConfig` 零改动。
- `src/executor/anti_join.rs` — `AntiJoinExecutor`：同 Semi 同型改动。
- `tests/subquery_test.rs` — 仅追加：8 用例 + `name_value_map` helper；既有 20 用例零修改。

**Deviations from Plan**

1. （非实质，测试形式）见证用例的标量值断言按**预存直执行形状**校准：标量子查询 select-list 路径（关联与非关联同形）的实际输出为「全外层行 + 标量插入 select-list 位置（index 1）」，表头为基表全列（4 列头对 5 值行）——Plan 未定死列位置；`name_value_map` 按 name=idx 2 / 标量=idx 1 断言，即直执行参照本身。该预存形状属 I034 同族（Subquery 臂未在 I034 扫描臂修复范围），见 Experience Candidates。
2. （非实质，文案校准）R3-S2 错误文案实际为 lib Response 层双重包装：`Execution error: execution error: Subquery returns multiple rows (scalar subquery requires single row)`——见证按实际串逐字节断言（比 Plan 预期的单层包装更严格，非放宽）。
3. （非实质，工具链）`cargo fmt` 对 3 个改动文件重排；复跑全量确认结论新鲜。

**Blocker Handoff**

None

**Blocker Resolution**

None（本 Cycle 未发生阻塞）

**Self-Review**

- Spec review（逐任务）：T20/T21 查缓存在 inject/执行前、存缓存在成功 drain 后（Acceptance 5 代码审查锚点逐臂核对：subquery_eval 关联臂 insert 位于 drain 循环后；Semi/Anti insert 位于 while 循环后）；R1 相同参数值复用 / R2 相异值独立 + NULL 键身份（`Value` Eq/Hash impl 零改动，Null==Null 命中、与非 Null 区分）/ R3 命中等价 + 错误不缓存 / R4 语句生命周期（构造面机制）/ R5 Semi/Anti 同型 + 既有零回归——全部覆盖。Preserve 清单逐项核对：非关联 `cached_result` 路径、`extract_param_values`×3、标量派生语义（0 行 Null / 多行同位 Err / 首列取值）、`result_column_index` 插入、快照穿线、`build_right_key`/探测/输出行、EXISTS `if has_rows`/NOT EXISTS `if !has_rows`、`JoinRelatedConfig`——零触碰。Forbidden 清单：无计数 hooks、无跨语句存续、无错误缓存、无 LRU、planner/plan cache 零触及。
- Code review（完整 diff）：无计划外修改（fmt 重排除外，见 Deviation 3）；命中重建与 miss 构建逐语义等价（hashmap 仅 Some(key) 行、行序保持、has_rows=行集非空——NULL 键行语义与 miss 逐字节一致）；错误路径（执行器 `?`、多行早退）均先于缓存写入传播；无新增警告（clippy -D warnings 0）、无死代码、无身份型证据机制；测试以可观察行集/错误文案断言，不因错误原因通过。
- 已修复发现：初版见证 6 用例按「两列理想形状」（name=idx 0）断言失败（`row[0]` 为 Int 的 unwrap panic）——CLI/库探针复核确认预存形状后按直执行参照校准 helper 与错误文案（Deviation 1/2），未改引擎代码；校准后第一 GREEN 达成。
- 遗留 Minor：无。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 等价见证（第一 GREEN，无缓存实现） | `cargo test --test subquery_test` | `test result: ok. 28 passed; 0 failed` | T22 前半 8 用例 + 既有 20 用例（T20/T21 实施前） | PASS |
| 等价见证（T20 后） | `cargo test --test subquery_test` | `test result: ok. 28 passed; 0 failed` | subquery_eval 缓存面 | PASS |
| 等价见证（T21 后，最终态） | `cargo test --test subquery_test` | `test result: ok. 28 passed; 0 failed` | subquery_eval + semi_join + anti_join 缓存面 | PASS |
| 全量回归（最终态） | `cargo test --no-fail-fast` | 73 binaries all `test result: ok`；合计 **936 passed / 0 failed / 2 ignored**（基线 928 + 新增 8；2 ignored 为 cli_test 既有信号标定项） | 全仓零回归门 | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | `Finished` 退出码 0 | 全 targets | PASS |
| 格式 | `cargo fmt --check` | 无 diff 退出码 0 | 全仓 | PASS |
| OpenSpec 结构 | `openspec validate --specs --changes` | `Totals: 28 passed, 0 failed (28 items)` 退出码 0 | specs + changes（wal-writer-handle-reuse Purpose 占位 WARNING 为预存项，非本次引入） | PASS |

**Persisted Evidence**

None required（Plan Context mode: none——全部验收由可重复运行的测试与工具链命令承载，无不可低成本复现现场）。

**Experience Candidates**

- Issue 候选（当前 change 范围外，只报告不落账）：**标量子查询 select-list 输出表头与行形状不一致**——`SELECT col, (subquery) AS alias FROM t` 的表头为基表全列（N 列），行为基表全列 + 标量插入（N+1 值），CLI JSON 与库 Response 均如此；关联与非关联同形（`{"columns":["id","name","dept","salary"],"rows":[[1,"East","Alice",10,50000],...]}`，`SELECT emp.name` 单列则正常裁剪）。属 I034「CLI 表头与行形状一致」同族缺口——I034 修复仅覆盖 DataScan/Scan/IndexScan 臂，`get_plan_output_columns` 的 SubqueryEval 臂未计入插入列。证据：本 Response Verification 段探针记载 + `test_correlated_scalar_subquery` 等既有用例按此形状断言通过。处置建议：独立小 change（planner `get_plan_output_columns` SubqueryEval 臂补插入列名），由 Recorder/用户裁定登记。
- Runbook 候选：None（本 Cycle 无高风险可复用操作路径；形状探针为普通一次性命令）。

**Remaining Issues**

- Experience Candidates 中的 Issue 候选（范围外预存缺陷，本 Cycle 以直执行参照校准见证，未改变其存在）。
- I041（resolve env 测试竞态）本次全量未出现假失败；维持独立小 change 处置。
- 无其他。

**Commit or Diff Reference**

未提交（工作区待用户统一触发；对照面 = 父 Iteration 001-rework 收尾工作区状态）。本 Cycle 完整增量 = `src/executor/{subquery_eval,semi_join,anti_join}.rs` 缓存面 + `tests/subquery_test.rs` 追加（8 用例 + helper）；`cargo fmt` 同步重排上述文件。

## Plan Review

- Review Result: accepted

**Findings**

- **F1（三执行器缓存面逐臂核实，与契约逐点吻合）**：(1) `subquery_eval.rs`——缓存查询在 clone/inject/重建执行器之前（:134-135）；命中派生标量与 miss 路径逐语义等价（`rows.first()` 非空取 `[0]`、空行或 0 行 → `Null`，:136-139）；多行 `row_count > 1` 早退 `Err(SubqueryReturnsMultipleRow)` 保持在原位置、先于缓存写入（:157-164 vs :175）——错误与多行永不入缓存；`collected` 于成功 drain 后存储（:173-175）；非关联臂 `cached_result` 路径逐字节原样（:178-185）。(2) `semi_join.rs`/`anti_join.rs` 同型——命中从缓存完整行集重建 `right_hashmap`（仅 `build_right_key()==Some` 行、行序保持）+ `right_has_rows = !rows.is_empty()`（NULL 键行计入，EXISTS/NOT EXISTS 语义保真）；miss 收集全部行（含 NULL 键行）且存储在 drain 循环后；错误经既有 `?` 在存储前传播；非关联快路径/`build_right_key`/探测/输出行零触碰。Acceptance 5「相同参数值至多执行一次」代码审查锚点独立复核成立（查在执行前、存在成功后）。
- **F2（Act 偏差 3 项核实为真实且非实质）**：Deviation 1（见证断言按直执行参照形状校准）——形状预存性核实：`get_plan_output_columns` SubqueryEval 臂（`query.rs:104`）返回 `get_plan_output_columns(&node.input)`，不含执行器插入的标量列（表头 N 列对 N+1 值行）；`query.rs:476` 既有注释即记载「标量子查询项会追加一列（SubqueryEval 移位输出形状）」；本 Cycle 未触碰 query.rs。契约未定死列位置，以直执行参照为断言锚点正是等价见证的正确形态。Deviation 2（错误文案双重包装逐字节断言 `Execution error: execution error: Subquery returns multiple rows (scalar subquery requires single row)`）——错误构造代码（`StorageError::ExecutionError(PlanError::…)`）与 Response 包装层本 Cycle 零改动，实际串即既有面；逐字节断言比 Plan 预期更严格，非放宽。Deviation 3（fmt 重排触及文件）——机械伴随面，`cargo fmt --check` 0 diff 复核。
- **F3（验证独立复现，与 Act 决定性聚合一致）**：本会话复跑 `cargo test --test subquery_test` **28 passed / 0 failed**（20 既有零修改 + 8 新增）；全量 `cargo test --no-fail-fast` 首轮出现单点失败（935/1/2），连续两轮复跑干净，第三轮聚合 **936 passed / 0 failed / 2 ignored**（73 bins）——与 Act 决定性运行逐数一致；单点失败复跑即消失且失败项未在干净运行复现，符合已知 I041 resolve env 竞态假失败形态（Act Risks 预授权处置 + 001-rework 先例），按公共规则 › 验证（已知 flaky 重跑）不采信失败运行。`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` 0 diff；`openspec validate --specs --changes` **28 passed / 0 failed**（1 条 wal-writer-handle-reuse Purpose 占位 WARNING 为预存项）。
- **F4（范围外实质发现——Issue 候选核实成立，非阻塞）**：Act 报告的「标量子查询 select-list 输出表头与行形状不一致」（`SELECT col, (subquery) AS alias FROM t` 表头为基表 N 列、行为 N+1 值）独立核实为真且预存——根因 `query.rs:104` SubqueryEval 臂未计入插入列，属 I034（已 promoted，修复面仅 DataScan/Scan/IndexScan 臂）同族缺口；关联与非关联同形，既有用例按此形状断言通过。本 Cycle 以直执行参照校准见证未改变也未掩盖该形状；Acceptance（等价性 + 零回归）不受影响。属 change 范围外实质缺陷，按规则只报告不落账，处置留 Recorder/用户（独立小 change：SubqueryEval 臂补插入列名）。
- **F5（Forbidden/Invariant 全过）**：无计数 hooks、无 static/global 跨语句结构、无 LRU/淘汰、无错误缓存、planner/plan cache/`inject_correlated_values`/`Value` Eq-Hash impl/快照语义/页格式/WAL 零触及（改动面 = 3 执行器 + 测试文件，与 Act Changed Files 清单精确一致；diff 含 Iteration 000 快照字段属既有工作区叠加，非本 Cycle 引入）。

**Deviation Classification**

- ACT-DEVIATION: None（3 项偏差均契约内非实质——测试形式 / 文案校准从严 / fmt 机械重排）。无 PLAN-OMISSION、PLAN-INVALID、BASELINE-CHANGED、NEW-EVIDENCE。

**Acceptance Gaps**

- None——5 项 Acceptance 全部满足：等价见证 8 用例（≥7）实施前后双 GREEN（Act 表第 1/3 行 + 本会话复跑 28/28）；Semi/Anti 关联面覆盖（用例 7/8 + 既有 IN/EXISTS 全绿）；全量 936/0/2（= 928 基线 + 8 新增，零修改）；clippy/fmt/validate 全 0/PASS；「至多执行一次」代码审查独立复核成立。

**Convergence**

- N/A（initial Cycle 首次 Review，无 gap 历史可比；父 Iteration 001-rework accepted 面未变化）

**Evidence**

- 代码核实：`subquery_eval.rs:130-185`（关联臂缓存面 + 非关联臂原样）、`semi_join.rs`/`anti_join.rs` 关联重建臂 diff（命中重建 + miss 收集/存储位置）、`query.rs:104`（SubqueryEval 表头臂——Issue 候选根因）、`query.rs:476`（预存形状注释）、`tests/subquery_test.rs` 8 新用例 + `name_value_map` helper（错误用例逐字节断言、跨语句新鲜度断言均经本会话读码核对）。
- 本会话复跑：subquery_test 28/28；全量 936/0/2（第 3 轮，第 1 轮单点失败按已知 I041 flaky 处置）；clippy 0、fmt OK、validate 28 passed/0 failed。
- 采信：Act Verification Evidence 全表（实施前第一 GREEN 28/28、T20/T21 后 28/28——材料未变化，本 Review 读码与复跑核对一致）。

**Follow-up Decision**

None——Acceptance 完整达成、无 Minor finding 需当前 Cycle 修复。Issue 候选（标量子查询 select-list 表头/行形状，I034 同族）为范围外预存缺陷，只报告不落账，处置留 Recorder/用户。**Iteration 002（T20-T22）完成，本 change 三个 Iteration 全部 accepted——change 实施面收口，待用户触发 openspec-docs-maintainer 收尾。**

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Iteration 002 为 change 最后一个 Iteration；三个 Iteration 全部 accepted）
