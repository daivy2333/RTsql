# Iteration 000 / Cycle 000: 键位等值对无键行可达——planner 路由修复

## Plan Context

- Status: ready
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: R1 五场景全绿 + R2/R3 锁定绿 + 全量回归零修改（见 change `tasks.md` Iteration 000）
- Verification boundary: 新增测试全绿、`cargo test` 全量只增不减、clippy/fmt 0、`openspec validate` PASS、CLI 探针复核
- Diagnostic boundary: `src/parser/planner/query.rs` SELECT 单表 WHERE 路由段 + `tests/keyless_eq_routing_test.rs`
- Deferred tasks: None

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: spec `planner-key-equality-routing` R1-R3 全部场景；design D1-D5
- Excluded scope: 形态 2（Int 字面量 + Float 键列，待用户裁定）、I037/I034/I039/I038、存储/索引/执行器层、OR→IndexScan 优化

**Objective**

`WHERE <键列> = <不可键控字面量>`（String/Float/Bool/NULL，简单或 AND 组合，隐式或声明非 Int 键列）不再经索引路由漏掉无键行：修复后返回正确行集且 plan 走数据页路径（`DataScan` 下推或 `Filter(DataScan)`）；可键控 Int 字面量的全部既有路由形状（`IndexScan` / `Filter(Scan)`）与结果逐字节不变；全量回归零修改。

**Background**

MS15-T01 / improvements I036：MS10-T05 001-rework 将键位不可键控行改为落库不入索引后，planner 键位等值路由未同步——`has_pk_equality` 结构化判定保留 `Filter(Scan)`（索引遍历），无键行不可达，静默漏行。MS10-T05 与 MS11-T03 双重独立实证；本 change 调查（2026-09-12）新鲜探针复现三形态并新发现形态 2（Int 字面量 + Float 键列，方向 A 不覆盖，待用户裁定）。执行序：MS15 首项（初版前正确性收口）。

**Investigation Facts**

- Current Baseline: 分支 master，最后 commit 179228b；工作区含 MS11-T03 实施未提交改动（SNAPSHOT 同步状态 current）。测试基线 845 passed / 0 failed / 2 ignored（2026-09-11 Plan Review 独立复跑，覆盖范围未变化）。22 capability specs validate PASS。
- Current-State Evidence:
  - 路由链（本会话实读）：`query.rs:501` `extract_pk_from_where`——仅顶层 `pk = Expr::Value` 且 `value_from_sqlparser(..).to_key()==Some`（Int，`src/executor/value.rs:82-90`）返回 `Some(key)`；`query.rs:506` simple → `IndexScan`，`query.rs:517` 非 simple → `Filter(base_plan)`；`query.rs:527` extract None 分支内 `query.rs:536-545` `has_pk_equality`（`query.rs:822-846`，Eq 腿结构判定 + AND 递归、OR 保守 false）为真 → `Filter(Scan)`（**病灶分支**）；`query.rs:546` `contains_or` → `Filter(DataScan)`；`query.rs:565` 谓词下推 `DataScan`（`base_plan` 非 Scan 时保留 Filter 包装）。
  - 病灶机制：不可键控字面量 → extract None → has_pk_eq true → `Filter(Scan)`；`ScanExecutor` 走 `index_manager.scan_all()`，无键行不入索引 → 不可达。
  - 可键控性/字面量事实：`Value::to_key()` 仅 Int → `Some`；`value_from_sqlparser`（`src/parser/value.rs:8-30`）覆盖 Number（Int 优先，溢出回退 Float）/SingleQuotedString/Null/Boolean，其余 `Err(UnsupportedValue)`；负数字面量在 WHERE 中为 `Expr::UnaryOp`（非 Value 腿）。
  - 求值语义事实：`ComparisonPredicate`（`src/executor/predicate.rs:103-134`）NULL 操作数 → `Unknown`；Eq/Ne 经 `Value::equals`（`src/executor/value.rs:118-137`，跨类型 false、Int↔Float 隐式转换）；Gt/Lt/Ge/Le 返回 Result。
  - 隐式键列事实：无声明 PK 时 `CreateTableExecutor` 取首列（`src/executor/create_table.rs:60-67`），catalog 持久化 `pk_column`，SELECT 经 `pipeline.rs:983-1008` `register_table` 注入 planner（仅列名，不传类型——方向 B 扩展点）。
  - 缺陷探针（2026-09-12，`target/debug/rtsql` CLI 直连，全部 exit 0）：三形态 `rows:[]` + 对照路径正常 + 声明 `TEXT PRIMARY KEY` DDL 被接受且同病。详见 proposal Why 表。
  - 既有测试面（无锁定冲突）：`pushdown_test.rs:202/212` PK 形状断言均用可键控 Int 字面量；`expression_e2e_test.rs:521`、`scalar_function_test.rs:239-252` 的字符串等值 WHERE 均在声明 Int PK 表（s 为非键列，走既有下推臂）；`keyless_row_test.rs` WHERE 均为非键列（`v=1`）或可键控 Int（`a=5/7`）；`planner_test.rs:337-341` 对 WHERE plan 形状为宽容匹配（Filter/DataScan/IndexScan 均接受）。
- Code and Critical Path:
  - 修改点唯一：`src/parser/planner/query.rs` `build_select`（或等价单表 SELECT 规划函数）WHERE 路由段的 `has_pk_eq` 分支（`query.rs:536-545`）+ `has_pk_equality`（`query.rs:822-846`）分类升级。
  - 不触碰：`extract_pk_from_where`（`query.rs:851-892`，形态 2 保留）、`is_simple_pk_equality`（`query.rs:781`，仅 extract Some 路径可达）、执行器层、`contains_or`（MS11-T01 七变体）、pipeline/CLI。
  - 测试入口：新增 `tests/keyless_eq_routing_test.rs`（lib 集成测试，`Database::open` + `execute_sql`，参照 `tests/keyless_row_test.rs` 夹具先例：DDL 后 `flush_all` 落盘）；既有 `tests/pushdown_test.rs` 两用例零修改复跑。

**Implementation Guidance**

建议顺序：T1 先写目标断言观察 RED（行为断言 + plan 形状断言）→ T2 改路由 → T1 GREEN → T3 补锁定 → T4 收尾。判定实现建议（非实质细节，Act 可在契约内调整）：将 `has_pk_equality` 的遍历升级为分类（如返回枚举 `Keyable` / `NonKeyableLiteral`，或平行 helper），分支条件由 `if has_pk_eq` 收窄为「has_pk_eq 且无不可键控字面量腿」；收窄后自然落入既有 OR/下推臂，两臂代码逐字节复用、不新增第三条 plan 路径。字面量腿判定 = `Eq` 一侧键列 `Identifier` + 另一侧 `Expr::Value`，`value_from_sqlparser` 失败按既有 `extract_pk_from_where` 方式传播 `PlanError`（与顶层形态现行错误时序一致）。restart 场景参照 `keyless_row_test.rs` 先例（shutdown + reopen）。

**Behavioral Change**

- 当前：键位等值腿含不可键控字面量的 WHERE → `Filter(Scan)` 索引遍历 → 无键行不可达，静默空集 exit 0。
- 目标：同形态 → 含 OR 时 `Filter(DataScan)`、否则谓词下推 `DataScan` → 数据页行内三值求值 → 正确行集；`NULL` 字面量腿求值 Unknown → 空集（与修复前可观察一致）。
- 接口/错误语义：无新错误、无错误文案变化；plan 形状对目标形态变化（可观察面为 spec R1 锁定的行集 + plan 断言），其余形态逐字节不变。

**Task Contracts**

### T1: R1 行为与 plan 形状的 RED 测试见证

- Requirement/Scenario: R1-S1/S2/S3/S4（行为）+ 修复形态 plan 形状
- Depends on: None
- Targets: `tests/keyless_eq_routing_test.rs`（新建）
- Current behavior: 文件不存在；对应行为为缺陷空集
- Required behavior: 四场景目标行为断言（String 隐式键列简单等值 / AND 组合 / Float 键列 Float 字面量 / 声明 TEXT PRIMARY KEY）+ 非键控腿 plan 形状断言（简单非键控 → `DataScan` 含谓词；非键控 + OR → `Filter(DataScan)`）
- Required changes: 仅新增测试文件；夹具参照 `keyless_row_test.rs`（DDL 后 `flush_all`）
- Preserve: 不修改任何既有测试与源码
- Forbidden: 不在本任务实现修复
- Test witness: `cargo test --test keyless_eq_routing_test` → RED（行为断言失败：实际空集；plan 断言失败：实际 `Filter(Scan)`）；记录失败清单
- GREEN condition: 无（T2 转绿）
- Verification: 命令 + 失败输出 + 退出码写入 Act Response
- Stop when: 目标断言无法在现有测试 API 下表达（如 plan 匹配辅助不可达）→ Blocker Handoff

### T2: 路由修复——不可键控字面量腿禁用索引路由

- Requirement/Scenario: R1 全部场景；R3-S1（路径变化、结果不变）
- Depends on: T1（RED 已观察）
- Targets: `src/parser/planner/query.rs` `has_pk_equality`（或其分类替身）+ `build_select` WHERE 路由 `has_pk_eq` 分支条件
- Current behavior: 见 Investigation Facts 病灶机制
- Required behavior: design D2 路由表「修复后」列逐行成立；`extract Some` 两路径、OR 臂、下推臂、非 Eq 路径逐字节不变
- Required changes: 键位等值腿不可键控字面量判定（String/Float/Bool/NULL 字面量 → `to_key()==None`）+ 分支条件收窄落入既有臂；`value_from_sqlparser` 腿级失败按既有方式传播
- Preserve: `extract_pk_from_where` / `is_simple_pk_equality` 语义零修改；执行器层零修改；`contains_or` 既有语义；plan cache 键机制
- Forbidden: 不改存储/索引/执行器；不实现形态 2 修复（`WHERE f = 5` 于 Float 键列仍走既有 extract→IndexScan 路径）；不新增第三条 plan 路径
- Test witness: T1 全部转 GREEN（`cargo test --test keyless_eq_routing_test` 全绿）
- GREEN condition: T1 断言全绿 + `pushdown_test`/`planner_test`/`expression_e2e_test` 零修改通过
- Verification: 上述命令 + 退出码
- Stop when: 收窄后既有测试出现非预期失败且原因指向分类判定误伤（实质语义问题）→ Blocker Handoff

### T3: R2/R3 回归锁定

- Requirement/Scenario: R2-S1/S2（既有见证复核）+ R3-S1（空结果不变）+ R1-S5（restart）
- Depends on: T2
- Targets: `tests/keyless_eq_routing_test.rs`（追加用例）
- Current behavior: R3-S1 形态（声明 Int PK + `WHERE id='abc'`）空集；无 restart 场景用例
- Required behavior: R3-S1 用例断言空集（变更前后 GREEN，锁结果不锁路径）；restart 用例断言 reopen 后 `WHERE s='x'` 返回行；`pushdown_test` 两 PK 形状用例零修改通过（复核命令 + 输出）
- Required changes: 仅追加测试用例
- Preserve: 既有测试零修改
- Forbidden: 不为锁定面新增源码改动
- Test witness: 追加用例 GREEN；`cargo test --test pushdown_test` 全绿
- GREEN condition: 同左
- Verification: 命令 + 退出码
- Stop when: restart 场景暴露恢复面缺陷（非本 change 范围）→ Blocker Handoff

### T4: 全量收尾与探针复核

- Requirement/Scenario: R3-S2（全量零回归）+ proposal 三探针形态复核
- Depends on: T3
- Targets: 无源码目标（验证任务）
- Current behavior: 基线 845 passed / 0 failed / 2 ignored
- Required behavior: `cargo test` 全量只增不减、0 failed；`cargo clippy -- -D warnings` 0、`cargo fmt --check` 0 diff、`openspec validate` PASS；CLI 探针（`target/debug/rtsql`，临时库）：String 隐式键列 `WHERE s='x'`、Float 键列 `WHERE f=5.0`、`TEXT PRIMARY KEY` `WHERE s='x'` 均输出正确行集 exit 0
- Required changes: None
- Preserve: 验证直接观察行为输出，不引入身份型证据构造
- Forbidden: 不为验证新增工具/脚本入库
- Test witness: 各命令决定性输出（每项 ≤20 行）
- GREEN condition: 同 Required behavior
- Verification: 同左
- Stop when: 全量出现与本 change 相关失败 → 回 T2 契约修复；无关失败 → 记录并按既有结论判定

**Invariants**

- 可键控 Int 字面量的既有路由与结果逐字节不变（R2）。
- 无键行存储语义（落库不入索引）不变；本 change 只动 planner 路由。
- 既有错误文案零变化；无新错误变体。
- `tests/` 既有文件零修改（含 MS11-T03 注记用例）。

**Non-goals**

形态 2 修复；列-列/参数腿路由变更；子查询上下文的专门验证；存储/执行器/索引层；OR→IndexScan 优化；多列 PK；I037/I034/I039/I038。

**Acceptance**

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 | S1 简单等值 | D1/D2 | T1,T2 | 000 | `query.rs` has_pk_eq 分支 | `keyless_eq_routing_test.rs` | None | Covered |
| R1 | S2 AND 组合 | D2 | T1,T2 | 000 | 同上 | 同上 | None | Covered |
| R1 | S3 Float 键列 | D1/D2 | T1,T2 | 000 | 同上 | 同上 | None | Covered |
| R1 | S4 TEXT PRIMARY KEY | D1/D2 | T1,T2 | 000 | 同上 | 同上 | None | Covered |
| R1 | S5 restart | D5 | T3 | 000 | 同上 | 同上（restart 用例） | None | Covered |
| R2 | S1 IndexScan 保持 | D2/D5 | T3 | 000 | `query.rs`（不触碰面） | `pushdown_test.rs::simple_pk_equality_still_index_scan` | None | Covered |
| R2 | S2 Filter(Scan) 保持 | D2/D5 | T3 | 000 | 同上 | `pushdown_test.rs::complex_pk_equality_still_filter_over_scan` | None | Covered |
| R3 | S1 空结果不变 | D3 | T3 | 000 | 同上 | `keyless_eq_routing_test.rs`（R3-S1 用例） | None | Covered |
| R3 | S2 全量零回归 | D5 | T4 | 000 | 全仓 | `cargo test` 全量 + clippy/fmt/validate | None | Covered |

无 Simplified 项；无 Missing 项。

**Verification**

按 task contract 逐项：目标测试 RED→GREEN（T1/T2）、锁定 GREEN（T3）、全量 + 静态检查 + CLI 探针（T4）。全部直接观察行集、plan 形状、退出码；不使用身份型证据工程。

**Gate 2 Readiness**

- 无 Missing requirement: **PASS**（RTM 全 Covered）
- 无未批准 Simplified: **PASS**（无 Simplified 项）
- 调查完整: **PASS**（路由链/字面量转换/求值语义/隐式键列/测试面全部实读 + 缺陷探针新鲜复现；证据见 Investigation Facts）
- 设计闭合: **PASS**（D1-D5：方向、逐形态路由表、边界语义、残差清单、测试策略；无契约级 TBD）
- 任务可执行: **PASS**（T1-T4 契约含位置/行为/见证/停止条件）
- 分轮合理: **PASS**（单 Iteration 平衡审计通过，change tasks.md）
- 追踪完整: **PASS**（RTM 9 场景全链路）
- 验证充分: **PASS**（行为 + plan 形状 + 全量回归三层，直接观察目标行为）
- 无身份型证据工程: **PASS**（无新增工具/协议/指纹）
- 无实质未知项: **PASS**（形态 2 可达性已探针实证，属范围决策非未知项；残差 D4 已定性）
- OpenSpec 产物一致: **PASS**（proposal/specs/design/tasks/cycle 互查一致）
- Persisted Evidence 模式明确: **PASS**（none，见下）
- 用户批准计划: **PASS**（用户 2026-09-12 批复"批准"：需求与范围按 I036 已记录方案（方向 A）生效；形态 2 处置采纳建议 (a)——独立后续 change、方向 B 为候选，I 项登记由 docs-maintainer 执行）

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证输出；无不可低成本重跑的决定性证据需求。Budget: 不适用。

**Risks and Notes**

- 工作区含 MS11-T03 未提交改动：Act 开始前建议用户先 commit（避免验证基线混叠）；若直接实施，T4 全量基线以实施时点实测为准。
- 形态 2 在本 change 后依旧存在（探针实锤的静默漏行）：User 决策前不实施修复；如用户裁定扩入本 change，将触发 replan（更新范围与 T 契约）。
- 非实质选择留给 Act：分类判定的函数形态（枚举返回 vs 平行 helper）、断言组织方式、夹具细节。

## Act Response

- Status: reported

**Implemented**

T1-T4 全部完成。T1 新建 `tests/keyless_eq_routing_test.rs`（R1 四行为场景 + 简单非键控 plan 形状 + OR 形态守卫，修复前观察 RED：5 failed / 1 passed）。T2 按设计 D2 建议的「平行 helper」形态实施路由修复：新增 `has_non_keyable_pk_literal_leg`（镜像 `has_pk_equality` 遍历：Eq 腿键列 Identifier + 另一侧 `Expr::Value` → `value_from_sqlparser(...).to_key().is_none()`；AND 递归、OR 保守 false；转换失败按 `extract_pk_from_where` 同方式 `?` 传播），`has_pk_equality` 分支条件收窄为 `has_pk_eq && !has_non_keyable_pk_literal_leg(..)?`，非键控腿自然落入既有 OR 臂 / 谓词下推臂，未新增第三条 plan 路径。T3 追加 R3-S1 空结果不变用例与 R1-S5 restart 用例（8/8 GREEN），R2 两锁定用例零修改复核通过。T4 全量 + 静态检查 + validate + CLI 三探针全部通过。

**Changed Files and Symbols**

- `src/parser/planner/query.rs`（+61/-1）：`build_select` WHERE 路由段 has_pk_eq 分支条件收窄（约 :537-547，含 MS15-T01 约束注释）；新增私有方法 `PlanBuilder::has_non_keyable_pk_literal_leg`（约 :855-905，`has_pk_equality` 之后）。未触碰：`extract_pk_from_where`、`is_simple_pk_equality`、`has_pk_equality` 本体、`contains_or`、执行器/存储/索引层、plan cache。
- `tests/keyless_eq_routing_test.rs`（新建，8 用例）：`string_implicit_key_simple_equality_reaches_keyless_rows`、`string_implicit_key_and_combined_equality_reaches_keyless_rows`、`float_key_float_literal_equality_reaches_keyless_rows`、`text_primary_key_equality_reaches_keyless_rows`、`simple_non_keyable_equality_plan_is_data_scan_with_predicate`、`non_keyable_equality_with_or_keeps_filter_over_data_scan`（T1）；`int_key_non_int_literal_still_empty_result`（R3-S1）、`keyless_equality_reachability_survives_restart`（R1-S5，T3）。
- `openspec/changes/.../tasks.md`：T1-T4 状态 planned → done，头部状态行同步（Phase 4 步骤 9 的 Iteration 状态维护）。

**Deviations from Plan**

1. 基线现场与 Investigation Facts 记载不同（非实质）：Plan 时工作区含 MS11-T03 未提交改动（基线 179228b）；Act 开始时用户已将其提交为 0708427 并完成文档同步（HEAD=f9e1e1f），工作区干净。845 测试基线已含 MS11-T03，与 Plan Context 一致；Plan Risks 预留的「用户先 commit」分支即实际路径。
2. T1 的 OR 形态 plan 断言修复前即 GREEN（非 RED）：`has_pk_equality` 对 OR 保守 false，`WHERE s = 'x' OR n = 2` 修复前已走 `contains_or` → `Filter(DataScan)`。契约 RED 预测（"plan 断言失败：实际 Filter(Scan)"）仅适用于简单非键控形态；OR 断言转为 T2 不得干扰 OR 臂的守卫见证。RED 清单如实记录：5 failed（4 行为断言实际空集 + 简单 plan 断言实际 `Filter(Scan)`）、1 passed（OR 守卫）。
3. 实现形态采用设计 D2 两个建议方案中的「平行 helper」而非枚举升级：`has_pk_equality` 本体零改动使 R2 保留路径判定按构造逐字节不变，且其唯一调用点即本分支（grep 实证），无连带影响。属 Plan 明示留给 Act 的非实质选择。
4. clippy 按 `cargo clippy --all-targets -- -D warnings` 运行（契约写 `cargo clippy -- -D warnings`）：范围只增不减（含 tests），结论更强。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

Spec review（逐任务 + 全量 diff 复核）：R1-S1..S5 行为与 plan 形状全实现并有 RED→GREEN 见证；R2-S1/S2 由既有用例零修改通过锁定；R3-S1/S2 见证齐备。D2 路由表「修复后」列逐行成立：非键控简单/AND → 下推 `DataScan`、含 OR → `Filter(DataScan)`（T1 断言）；Int 字面量简单/AND → `IndexScan`/`Filter(Scan)` 不变（pushdown 锁定用例经收窄分支回归验证，证明可键控腿判定不误伤）；负数字面量/列-列腿非 `Expr::Value` → 分类不触发 → 既有路由保持（D3，代码路径未动）。Preserve 面零修改（diff 实证：仅分支条件 + helper + 注释）；Forbidden 项全部遵守（形态 2 未实现——`f = 5` Int 字面量仍走 extract Some → IndexScan）。

Code quality review：diff 无计划外修改；错误/边界路径齐备（无 PK 表 → false、非 Eq/OR → false、非字面量腿 → false、转换失败 `?` 传播且实践中先被 `build_where` 同源转换拦截，错误语义零变化）；无新增警告（clippy 0）、无死代码、无重复实现（helper 与 `has_pk_equality` 共享遍历形状但回答不同问题，属设计明示方案）；测试按结构断言 plan 形状、按行集断言行为，无身份型证据机制；命名与局部结构随项目惯例。

已修复发现：无（逐任务与全量 Review 均未发现 Critical/Important 问题）。遗留 Minor 问题：见 Remaining Issues 第 2 条（不阻塞）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 RED | `cargo test --test keyless_eq_routing_test` | `test result: FAILED. 1 passed; 5 failed`；4 行为断言 `left: []`；plan 断言 `实际 Filter(FilterNode { input: Scan(..) })` | 病灶分支可观察行为 | RED 符合预期 |
| T2 GREEN | 同上（修复后） | `test result: ok. 6 passed; 0 failed` | R1-S1..S4 行集 + 修复形态 plan 形状 + OR 守卫 | PASS（exit 0） |
| T2 保留边界 | `cargo test --test pushdown_test --test planner_test --test expression_e2e_test` | `16 passed` / `36 passed` / `24 passed`，均 `0 failed` | IndexScan/Filter(Scan) 形状、planner 全面、表达式链路 | PASS（exit 0） |
| T3 锁定 | `cargo test --test keyless_eq_routing_test` | `test result: ok. 8 passed; 0 failed` | +R3-S1 空集不变 + restart 可达 | PASS（exit 0） |
| T3 R2 复核 | `cargo test --test pushdown_test -- simple_pk_equality_still_index_scan complex_pk_equality_still_filter_over_scan` | `test result: ok. 2 passed; 0 failed` | R2-S1/S2 既有见证零修改通过 | PASS（exit 0） |
| T4 静态 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | 均 0 警告 / 0 diff | 全仓 lint/format | PASS（exit 0） |
| T4 全量 | `cargo test` | `TOTAL passed=853 failed=0 ignored=2`（845 基线只增不减，+8 为本 change） | 全仓回归 | PASS（exit 0，完整单次运行） |
| T4 validate | `openspec validate --changes` | `✓ change/2026-09-12-ms15-t01-keyless-eq-routing`；`Totals: 1 passed, 0 failed` | change 产物结构 | PASS |
| T4 CLI 探针 | `target/debug/rtsql <tmp>/pN.db "<DDL/INSERT/SELECT>"` ×3 | `{"rows":[["x",1]]}`（String 隐式键列）；`{"rows":[[5.0,1]]}`（Float 键列）；`{"rows":[["x",1]]}`（TEXT PRIMARY KEY） | 三缺陷形态端到端行集 | PASS（均 exit 0） |

**Persisted Evidence**

None required（Plan 模式 none；全部验证输出摘录于上表，可低成本重跑）

**Experience Candidates**

None

**Remaining Issues**

1. 既有 flaky 测试（先于本 change，与本次改动无关）：`src/cli/resolve.rs::tests::test_bare_name_env_cases` 与相邻 env 用例（:104 起）并发改写进程级 `HOME`/`RTSQL_HOME`，cargo test 并行下互相污染——实测 lib 套件约 5 次中 2 次失败（断言期望 `/home/testuser/...` 实得 `/tmp/rtshome/...`），模块注释声称「env 用例集中单个 #[test]」的缓解与实际存在两个 env 用例相矛盾。本 Cycle 诊断边界不含 `src/cli/`，未修复；全量套件完整单次运行已通过，按 T4 契约「无关失败 → 记录并按既有结论判定」。建议后续登记 improvement（测试隔离：env 用例串行化或改依赖注入）。
2. Minor：R1 requirement SHALL 的 Bool/NULL 字面量形态无独立场景——实现按 `to_key()` 类型泛化覆盖（与 String/Float 同一代码路径），且二者无可观察行集差异（跨类型 equals 恒 false / NULL 求值 Unknown），RTM 九场景亦未列；如需 plan 形状级见证可后补两条廉价断言。
3. 形态 2（Int 字面量 + Float 键列）按批准范围未修，静默漏行依旧存在（探针实锤）；待用户裁定的独立后续 change。

**Commit or Diff Reference**

未提交（用户未指示 commit）；工作区 diff = `src/parser/planner/query.rs` +61/-1、`tests/keyless_eq_routing_test.rs` 新增、change 目录（tasks.md 状态 + 本 Response）。基线 HEAD = f9e1e1f。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（代码 + diff + 测试文件全读，非 Self-Review 复述）：T2 修改面与 Act Response 记载一致——仅 `build_select` WHERE 路由段 `has_pk_eq` 分支条件收窄（`query.rs:542`，含约束注释）+ 新增私有 `has_non_keyable_pk_literal_leg`（`query.rs:864-906`）；`has_pk_equality` 本体、`extract_pk_from_where`、`is_simple_pk_equality`、`contains_or`、执行器/存储层、plan cache 全部零改动（diff 实证）。helper 遍历与 `has_pk_equality` 逐臂镜像（Eq 腿键列判定 + AND 递归 + OR 保守 false），`value_from_sqlparser` 腿级失败 `?` 传播与 `extract_pk_from_where` 同式；收窄后落入既有 OR 臂 / 下推臂，未新增第三条 plan 路径。列-列腿 / 负数字面量腿（非 `Expr::Value`）分类不触发 → `Filter(Scan)` 保持，与 design D2/D3 边界表逐行一致。测试文件 8 用例与 Response 清单一致，夹具遵循 `keyless_row_test.rs` 先例（flush_all / shutdown + reopen）。

- F1（Minor，PLAN-OMISSION，非阻塞）：delta spec R1 requirement 正文「不可键控字面量 SHALL 涵盖 String、Float、Bool 与 NULL 字面量」中 Bool/NULL 两形态无场景级见证（RTM 九场景亦未列）。实现按 `to_key().is_none()` 类型泛化覆盖，Bool/NULL 与 String/Float 走同一代码路径（代码实证：helper 对任意 `Expr::Value` 单点转换，无类型分支），且二者无可观察行集差异（Bool 跨类型 equals 恒 false；NULL 求值 Unknown）——Act Remaining Issues 2 已如实披露。不构成 Acceptance gap（RTM 九场景全 Covered），不要求当前 Cycle 修复；如需 plan 形状级见证可由用户决定后补两条廉价断言。
- F2（信息项，非本 change 范围）：`src/cli/resolve.rs` env 用例并发污染（Act Remaining Issues 1，先于本 change 的既有 flaky，诊断边界外）。建议收尾时由 docs-maintainer 登记 improvement（测试隔离：env 用例串行化或依赖注入），是否登记由用户决定。
- F3（信息项）：形态 2（Int 字面量 + Float 键列静默漏行）按批准范围保留（`f = 5` 仍走 extract Some → IndexScan，代码路径未动，Forbidden 遵守）；用户 2026-09-12 已裁定独立后续 change（方向 B 为候选），I 项登记由 docs-maintainer 收尾时执行。

**Deviation Classification**

1. BASELINE-CHANGED（非阻塞）：Act 开始时 MS11-T03 已提交（0708427 + docs sync f9e1e1f），与 Investigation Facts 记载的工作区状态不同；Plan Risks 预留的「用户先 commit」分支即实际路径，845 基线含义不变。非实质。
2. PLAN-INVALID（非阻塞）：T1 Test witness 对 OR 形态 plan 断言的 RED 预测错误——Investigation Facts 自身已记载 `has_pk_equality` OR 保守 false，修复前 `WHERE s = 'x' OR n = 2` 即走 `contains_or` → `Filter(DataScan)`，预测与自家调查事实矛盾。Act 如实记录（RED 清单 5 failed / 1 passed）并将断言转为 T2 守卫见证，Acceptance 不受影响。
3. 无偏差（Plan 明示留给 Act 的非实质选择）：实现形态取「平行 helper」而非枚举升级，Plan Implementation Guidance 明示两案均可；`has_pk_equality` 本体零改动使 R2 保留路径按构造不变。
4. ACT-DEVIATION（非阻塞，范围增强）：clippy 按 `--all-targets` 运行（契约写 `-- -D warnings`），覆盖只增不减。

**Acceptance Gaps**

None。RTM 九场景全链路（requirement/scenario/design/task/code surface/test witness）逐项成立：R1-S1..S4 行集 + 简单/OR plan 形状、R1-S5 restart、R2-S1/S2 既有 pushdown 用例零修改通过、R3-S1 空结果不变、R3-S2 全量零回归。D2 路由表「修复后」列逐行与代码一致；Forbidden 项（形态 2 / 存储执行器层 / 第三条 plan 路径）全部遵守。

**Convergence**

N/A（首次 Review，无前序 gap；一次通过）

**Evidence**

Review 独立复跑（2026-09-12，基线检查先行：`git status`/`git diff --stat` 与 Act Response「Commit or Diff Reference」一致，HEAD f9e1e1f，覆盖范围未变化，Act 结论按公共规则 › 验证 采信并以下列新鲜复跑加强）：

| 验证项 | 命令 | 结果 | 结论 |
|---|---|---|---|
| 目标套件 | `cargo test --test keyless_eq_routing_test` | `8 passed; 0 failed`，exit 0 | R1-S1..S5 + plan 形状 + R3-S1 复现 GREEN |
| 全量回归 | `cargo test` | `passed=853 failed=0 ignored=2`（845 基线 + 8） | R3-S2 成立，exit 0 |
| R2 锁定 | 全量含 `pushdown_test` | 0 failed（全量汇总） | R2-S1/S2 零修改通过 |
| 静态 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | 均 0，exit 0 | PASS |
| validate | `openspec validate --changes` | `Totals: 1 passed, 0 failed` | PASS |
| CLI 探针 | `target/debug/rtsql` 三缺陷形态 | `[["x",1]]` / `[[5.0,1]]` / `[["x",1]]`，均 exit 0 | 与 Act Response 记载逐字节一致 |

Persisted Evidence 模式 `none`，按规则不因 Evidence 目录缺失提出问题。

**Follow-up Decision**

None（无需当前 Cycle 修复；无 rework/replan）

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（单 Iteration change，无剩余 Iteration；change 实施侧完成，收尾归档由 docs-maintainer 按用户指令执行）
