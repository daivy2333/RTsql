# Iteration 001 / Cycle 000: 绑定作用域表基建与关联子查询分类收口（T02-C1）

## Plan Context

- Status: ready（2026-10-20 Gate 2 PASS，用户批准计划）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T02（C1 面）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-semantic-binding` R1（Scenario「关联子查询参数分类消费绑定结果」全部 + 「JOIN 链绑定身份确定」的作用域登记基建段——JOIN 布局消费统一在 Iteration 002）、R2 全部 2 场景；proposal A5（增量式绑定、不改词表与执行器接口）、A7（零回归硬边界）；design §1.1/§2.2（C1）、D5（PlanBuilder 内加性承载）、D6（名单兜底保留）；Iteration 000 差分网 GREEN 基线（Review `accepted`，2026-10-20）。
- Excluded scope: C2 布局统一（Iteration 002）；执行器 `condition_column_index` 与其余执行期按名点；`ColumnRef` 载荷与 `get_plan_output_columns`（含差分发现 ③ 聚合派生表缺口——处置待用户裁定，默认不在本 Iteration）；视图展开/CTE/attach 段的登记完备化（D6 兜底覆盖，完备性留后续批次）；差分白名单条目增删（缺陷 ①②④ 属执行器层，C1 不触碰其行为）。

**Objective**

`PlanBuilder` 携带绑定作用域表（声明限定符段 + 列清单 + 类型/可空信息，加性字段）；关联子查询参数分类优先经作用域身份查找，`inner_table_names` 字符串名单降级为未登记段兜底；全部既有输入的分类结论与用户可见错误面逐字节一致（既有定向矩阵 + 差分网 + 全量基线三重零回归）。

**Background**

I080 根因（design §1.1）：分类正确性依赖 `extract_subquery_table_names`（静态提取 FROM 声明限定符）与 `build_query`（实际注册表）两个独立提取器对同一 FROM 子句产出一致结论，二者仅由约定耦合、无结构保证。Iteration 000 差分网已 GREEN 纳入全量基线（2045 tests，白名单 6 条目承载已知分歧），自本 Iteration 起承担重构保护职责。

**Investigation Facts**

- Current Baseline: master @ 34a3ae7 + 工作区 `tests/sql_differential_test.rs`、`tests/structural_property_test.rs` 与本 change 目录（Iteration 000 `accepted` 产物，未提交）；全量 2045 passed / 0 failed / 2 ignored（000 Act Response，M19 分批 14 批实测）；差分网定向 10 tests / 性质套件 5 tests 全绿。
- Current-State Evidence（2026-10-20 本会话实读，行号当前有效；Act 无需回读其他资料）:
  - **分类机制**：`qualified_column_expression`（`src/parser/planner/expression.rs:258`）优先级 1（:264-272）——`inner_table_names: Option<Vec<String>>`（`src/parser/planner/mod.rs:186`）Some 时，书写限定符 `eq_ignore_ascii_case` 不命中名单 → 产出 `ParameterExpression("qualifier.column")`；名单由 `extract_subquery_table_names`（`src/parser/planner/subquery.rs:381`，`declared_qualifiers` :404——别名与基表名并列保留）静态提取；三处子查询臂 set/restore 同 builder 实例（`subquery.rs:39-59`、:99-119、:176-195——子查询构建无独立 builder，外层注册表仍驻留 `self.tables`）。
  - **既有通道**：`PlanBuilder`（`mod.rs:146`）`tables: HashMap<String, Vec<String>>`（:148）；`register_table`（:274，键小写）；`register_table_alias`（:289，声明别名→真实键）；`resolve_table_qualifier`（:300，限定名单点归一）；`qualifier_matches_declared`（:321，原文相等或别名归一命中）；`table_column_types: HashMap<String, Vec<ColumnType>>`（:174，`set_table_column_types` :356）——仅真实表经 `register_table_schema`（`src/pipeline.rs:2500`）自 TableMeta 注册逐列类型；**not_null 无 planner 通道**（TableMeta 携带，`register_table_schema` 当前不传递）。
  - **FROM/JOIN 装配点（作用域登记落点）**：`build_from_clause_with_projection`（`src/parser/planner/query.rs:367`）内——FROM 别名 `register_table_alias`（:408）、派生/CTE effective 名 `register_table`（:457、:1278、:1332）、JOIN 右臂别名（:546）；真实表 schema 注册先经 pipeline `register_table`（`pipeline.rs:2668`）完成。
  - **差分网保护面**：Q5（标量子查询相关/非相关）、Q6（IN/EXISTS 内层别名形态）+ 等价变换别名改写直接覆盖分类面；白名单 Q5-05 为执行器求值层缺陷（`predicate.rs` M37），分类本身今日正确——C1 等价重构不改变其行为，条目稳定。
- Code and Critical Path: `src/parser/planner/binding.rs`（新——`ScopeEntry` 与作用域表登记/查询 API）、`mod.rs`（加性字段 + API 宿主）、`expression.rs::qualified_column_expression`（优先级 1 改造）、`subquery.rs`（三臂作用域快照/恢复）、`query.rs`（登记点同步登记）、`pipeline.rs::register_table_schema`（not_null 加性通道，一处）。消费链：FROM/JOIN 注册 → 作用域表 → 分类判定；无执行器/存储/网络面。

**Implementation Guidance**

建议顺序 ①→⑤（①② 纯加性零行为变化，可独立编译验证）：

1. `binding.rs` 定义 `ScopeEntry { qualifier: String, columns: Vec<String>, types: Option<Vec<ColumnType>>, not_null: Option<Vec<bool>> }`（`pub(crate)`，qualifier 键空间 = `join_column_layout` 段键空间的声明限定符）；`mod.rs` 加性字段 `binding_scopes: Vec<ScopeEntry>` 与登记/查询 API。
2. FROM/JOIN 五登记点（query.rs :408/:457/:546/:1278/:1332）在既有注册调用旁同步登记作用域段——真实表段 types 自 `table_column_types` 取、not_null 经 `register_table_schema` 新增加性传递（同 `set_table_column_types` 先例签名形态）；派生/CTE 段 types/not_null = `None`（计划期未知，诚实记录，不猜测）。
3. 子查询三臂（subquery.rs）进入时快照当前作用域表（外层作用域 = 子查询自身 FROM 登记前的累计段），退出恢复。
4. `qualified_column_expression` 优先级 1 改造：内层作用域命中 → 走内层解析（不产 ParameterExpression）；仅外层作用域命中 → `ParameterExpression`；书写限定符未登记于任一作用域（attach 复合键、一期未登记面）→ 回落 `inner_table_names` 名单判定（D6 兜底），结论逐字节不变。
5. 定向套件 → 差分网 → 全量分批 Gate。

判定等价性：已登记段的作用域判定结论应与既有名单判定一致（差分网 + 既有矩阵见证等价重构）；发现不一致组合即测试失败暴露，不得为通过测试校准任何错误文本。

**Task Contracts**

### T02-C1: 绑定作用域表基建与关联参数分类单点化

- Requirement/Scenario: `sql-semantic-binding` R1 Scenario「关联子查询参数分类消费绑定结果」（全部）+ Scenario「JOIN 链绑定身份确定」的登记基建段；R2 全部 2 场景
- Depends on: Iteration 000（差分网 GREEN 保护基线）
- Targets: `src/parser/planner/binding.rs`（新）、`src/parser/planner/mod.rs`、`src/parser/planner/expression.rs::qualified_column_expression`、`src/parser/planner/subquery.rs`、`src/parser/planner/query.rs`、`src/pipeline.rs::register_table_schema`
- Current behavior: 分类 = 字符串名单成员资格（`expression.rs:264-272`），名单静态提取自子查询 FROM 声明限定符（`subquery.rs:381`）；分类正确性依赖提取器与注册表约定一致（I080 根因，无结构保证）
- Required behavior: `PlanBuilder` 携带绑定作用域表（`ScopeEntry`：声明限定符 + 列清单 + Option 类型/可空）；FROM/JOIN 装配点同步登记；关联参数分类优先作用域身份查找（内层命中→内层引用、仅外层命中→`ParameterExpression`），未登记段回落既有名单判定；全部既有输入分类结论与错误面（类型/文本/SQLSTATE）逐字节一致
- Preserve: PhysicalPlan 节点词表与计划形状（`explain_test` 零校准）；执行器接口与 `condition_column_index`；既有 2045 基线零回归；差分网白名单 6 条目与跳过行为零变化
- Forbidden: `join_column_layout` 结构与五消费点改造（C2 面）；`ColumnRef` 载荷、`get_plan_output_columns`；执行器/存储/网络任何文件；错误消息文本任何校准；白名单条目增删；`src/` 之外的任何改动（tests 只允许新增失败暴露时按 A4 报告不改断言）
- First test: 重构类（行为保持型）——先行测试 = 既有定向套件 GREEN 基线观察（000 全量 Gate 已见证，直接采信）+ 差分网相关面 GREEN；重构后保持 GREEN，无 RED 形态
- GREEN condition: 定向套件（`subquery_alias_binding_test` / `subquery_test` / `scalar_subquery_comparison_test` / `table_alias_resolution_test` / `cte_test` / `view_test`）全绿；`cargo test --test sql_differential_test` 全绿；全量 `cargo test`（M19 分批串行一次）零失败零校准
- Verification: ① 六定向套件命令与结果；② `cargo test --test sql_differential_test`（白名单跳过行为与条目零变化）；③ 全量分批一次 rc=0；④ R2 错误面由既有错误断言（planner_test 等）零校准通过见证
- Stop when: 任一既有输入分类结论或错误文本必须变化才能容纳作用域判定（等价性假设失效 → 返回 Plan）；作用域登记必须触碰 PhysicalPlan 词表或执行器接口；差分网出现白名单外新分歧且归因于 C1 改动（先分诊：暴露即分类面隐藏组合，报告不擅自扩面）

**Invariants**

- A7 零回归硬边界：既有 2045 基线零失败，错误面（类型、文本、SQLSTATE）逐字节不变。
- D6 兜底：未登记段（attach 复合键、CTE/派生段一期未登记面）回落名单判定，行为逐字节不变。
- 加性承载（D5）：不引入独立 BindingIR 遍，不改 PlanBuilder 公共 API 语义。
- 固定种子差分网确定性维持；白名单条目集与跳过行为冻结。

**Non-goals**

C2 布局统一（Iteration 002）；③ 聚合派生表缺口修复（`get_plan_output_columns`——处置待用户裁定，默认范围外）；①②④ 执行器层缺陷（Issue 候选处置，见 000 Plan Review）；视图展开/CTE/attach 段登记完备化（D6 兜底覆盖，完备性留后续批次）；执行器 `condition_column_index` 与 `ColumnRef` 通道。

**Acceptance**

- R1/S2（关联子查询参数分类消费绑定结果）：作用域表存在、分类判定经身份查找先行（代码结构 + Review 核对「不依赖字符串限定符与构建上下文约定推断」的消费面切换）；相关/非相关既有矩阵全绿。映射：T02-C1 / Implementation Guidance ④。
- R1/S1（JOIN 链绑定身份确定）基建段：JOIN FROM 臂以声明限定符登记作用域段（含自连接别名两段、同名列清单）；该 Scenario 的布局消费统一在 Iteration 002 完成。映射：T02-C1 / Guidance ②。
- R2/S1-S2（全量零回归 + 错误消息逐字节保持）：全量零失败、错误断言零校准。映射：T02-C1 全部。

**Verification**

| Scenario | 判定 | 通过依据 |
|---|---|---|
| R1/S2 分类消费绑定结果 | 六定向套件 + 差分网 Q5/Q6/等价变换别名面 | 全绿 |
| R1/S1 基建段（JOIN 登记段） | `self_join_test` / `join_output_resolution_test` / `table_alias_resolution_test`（既有 GREEN 保持） | 全绿 |
| R2/S1 全量零回归 | 全量 `cargo test`（M19 分批一次） | rc=0、零校准 |
| R2/S2 错误面逐字节 | 既有错误断言零校准（`explain_test` 计划形状同证 R3 词表边界） | 零校准 |

**Gate 2 Readiness**

检查项全 PASS、无 BLOCKED/WAIVED 项（设计闭合：登记点/分类点/兜底边界/类型与可空通道已定位 file:line；RTM 无 Missing/Simplified）；用户已批准计划（2026-10-20），`ready`。

**Persisted Evidence**

- Mode: none

Act Response 承载定向与全量命令及结果；无不可低成本重跑项。

**Risks and Notes**

- R1（条件性）：已登记段与名单判定不一致的隐藏组合（attach 复合键段、视图展开段）——D6 兜底保证行为不变，不一致面由差分网暴露后按 A4 分诊；非阻塞。
- R2（非实质）：not_null 加性通道只经 `register_table_schema` 传递真实表；派生/CTE 段 `None` 语义为「计划期未知」，不是错误状态。
- R3（预算）：全量分批一次为 Iteration Gate 预算（M19 先例）；C1 无新增测试时间预算压力。

---

## Act Response

- Status: reported（2026-10-20；T02-C1 完成，全量 Gate 通过）
- Executor: openspec-act（本会话）

### 实际改动

六个契约目标文件全部触及，`git diff` 面 +116/−3 行，无契约外改动（git status：tracked 改动仅此五文件，`binding.rs` 新增；工作区其余未跟踪路径为 Iteration 000 既有产物）。

**`src/parser/planner/binding.rs`（新）**

- `ScopeEntry { qualifier, columns, types: Option<Vec<ColumnType>>, not_null: Option<Vec<bool>> }`（`pub(crate)`，qualifier = 声明限定符——有别名即别名，否则 effective 表名；派生/CTE/视图段类型/可空 = `None` 计划期未知）。
- `PlanBuilder` 作用域 API：`register_binding_scope`（段登记）、`table_binding_meta`（真实表类型/可空取值，键小写归一查既有加性通道）、`binding_scope_hit(written, start, end)`（段命中判定）、`restore_binding_scopes(len)`（截断恢复）。

**`src/parser/planner/mod.rs`**

- `mod binding;` 声明；`PlanBuilder` 三个加性字段：`binding_scopes: Vec<ScopeEntry>`、`binding_scope_boundary: usize`、`table_not_null: HashMap<String, Vec<bool>>`（`new()` 同步初始化）；`set_table_not_null` API（同 `set_table_column_types` 先例签名形态）。

**`src/parser/planner/query.rs`（五个登记点 + 两处嵌套臂截断）**

- 基表臂：`base_columns` 取得后按 `qualifier`（别名或表名）登记段，类型/可空自 `table_binding_meta`。
- 派生臂：`register_table(alias_name)` 旁登记段（types/not_null = `None`）。
- JOIN 右臂真实表分支：按 `right_qualifier` 登记段（同基表臂取值）。
- `resolve_cte_reference` / `resolve_view_reference`：`register_table(effective)` 旁登记段（`None`）——两函数为 FROM 基座与 JOIN 右臂共用，一次覆盖两调用面。
- FROM 派生臂与视图展开臂：`build_query` 进入时快照 `binding_scopes.len()` 并置界、成功/失败两路退出时截断恢复（定义内层段不外泄到外层分类面）。
- SELECT 投影位标量子查询臂（:1412 起）：同款快照/置界/两路恢复（见偏差 1）。

**`src/parser/planner/subquery.rs`（三臂）**

- IN / EXISTS / 标量绑定臂：既有 `inner_table_names`/`building_subquery`/`building_from_derived` save/restore 旁同步作用域快照、置界、两路截断恢复。

**`src/parser/planner/expression.rs`（分类判定单点）**

- `qualified_column_expression` 优先级 1：内层段（`binding_scope_boundary..len`）命中 → 沿既有解析通道（不产 `ParameterExpression`）；未命中且（外层段 `[0..boundary)` 命中 或 未登记于名单）→ `ParameterExpression`；段匹配用原文大小写不敏感相等（见偏差 4）。名单兜底分支保留为未登记面的 D6 路径。

**`src/pipeline.rs`**

- `register_table_schema` 增 `builder.set_table_not_null(key, table_meta.not_null.clone())`（主库与附加库路由分支共用点，一处覆盖）。

### 与计划的偏差及原因

1. **第四处子查询臂纳入快照/恢复**：Plan Context Investigation Facts 列三臂（subquery.rs :39-59/:99-119/:176-195），实读发现 SELECT 投影位标量子查询臂（query.rs :1412-1454）为同机制第四处 set/restore 点。契约等价性（分类结论逐字节一致）要求其作用域状态同步管理，属等价局部覆盖，非范围扩展。
2. **FROM 派生臂与视图展开臂加作用域截断**：计划 Guidance ③ 只点名子查询臂。不截断时派生/视图定义的内层 FROM 段滞留内层作用域，其内部限定引用会被误判为内层命中（既有行为为名单判定），破坏字节级等价——截断是等价性要求的契约内保障，非新增设计。
3. **consume_with（CTE 定义构建）不加截断**：CTE 定义仅顶层构建（子查询上下文 WITH 点名拒绝），其段落在任何分类界之前 = 外层作用域；对未登记限定符「外层命中 → ParameterExpression」与「D6 兜底名单未命中 → ParameterExpression」结论相同，等价性保持，取最小改动。
4. **段匹配用原文大小写不敏感相等，不用 `qualifier_matches_declared`**：别名注册表与子查询构建同 builder 驻留，外层别名经 `resolve_table_qualifier` 归一后会误命中内层段（如外层别名归一结果恰等于内层段名）；等价性要求「内层命中集 ⊆ 名单判定放行集」，名单判定为原文相等，故段匹配同型。已在 `binding_scope_hit` 文档段记录理由。
5. **`ScopeEntry` 三字段 dead_code 定点压制**：`columns`/`types`/`not_null` 本 Iteration 只登记不消费（消费方 = Iteration 002 布局段派生与后续批次），R1 要求绑定结果携带全部四元信息；项目已有 `#[allow(dead_code)]` 先例（`src/executor/aggregate.rs:322`）。

### 自检发现（Gate 3）

- Spec compliance：契约 Targets 六文件逐一面吻合、无遗漏无越界（git diff 核对）；分类判定经作用域身份查找先行、名单降级兜底（代码结构可见，待 Review 独立核对）；不变量逐条落实——A7 零回归（全量 2045/0/2 见证）、D6 兜底（未登记面回落名单分支保留）、加性承载（无独立 BindingIR 遍、PlanBuilder 公共 API 仅加法）、白名单冻结（差分网 10 tests 全绿含白名单结构自检）；Preserve 面——PhysicalPlan 词表零变化（`explain_test` 全量内零校准）、执行器/存储/网络零改动（diff 面证实）、错误文本零校准（`planner_test` 等错误断言全量内零失败）。
- Code quality：零新增编译警告（唯一 warning 为环境级 `~/.cargo/config` 弃用提示）；六处快照/置界/恢复严格配对（成功与错误两路均有，逐处核对）；无身份型证据机制、无框架外判定层。
- Gate 后改动：fmt 检查修正 query.rs 一处换行（纯空白重排，rustfmt 语义保持性质 + `cargo check` 通过见证，不影响 Gate 结论）；`tests/sql_differential_test.rs` 存在既有 fmt 漂移（Iteration 000 产物，本契约 Preserve 禁改 tests，报告不修）——遗留 Minor。
- 遗留 Minor：上述测试文件 fmt 漂移一项（归 Iteration 000 产物，建议 maintainer 收尾批或后续批次顺手处理）。

### 验证命令与实际结果

| 命令 | 实际结果 |
|---|---|
| `cargo test --test subquery_alias_binding_test` | 22 passed / 0 failed |
| `cargo test --test subquery_test` | 43 passed / 0 failed |
| `cargo test --test scalar_subquery_comparison_test` | 14 passed / 0 failed |
| `cargo test --test table_alias_resolution_test` | 19 passed / 0 failed |
| `cargo test --test cte_test` | 14 passed / 0 failed |
| `cargo test --test view_test` | 19 passed / 0 failed |
| `cargo test --test self_join_test`（R1/S1 基建段） | 6 passed / 0 failed |
| `cargo test --test join_output_resolution_test`（R1/S1 基建段） | 19 passed / 0 failed |
| `cargo test --test sql_differential_test`（保护网，白名单跳过行为与条目零变化） | 10 passed / 0 failed，1.16s |
| 全量（M19 分批串行）：`cargo test --lib`、`--bins`、125 个集成目标按字典序 11/批 × 12 批，批内 `-- --test-threads=1`、批间串行 | 14 批全部 rc=0；合计 **2045 passed / 0 failed / 2 ignored**（与 000 基线 2045/0/2 逐项一致——零回归零校准）；总耗时 600s（各批 0s-166s，含本次改动重编译） |

`cargo check --all-targets` 零 src 警告；`cargo fmt -- --check` 六个 src 契约文件零 diff（tests 文件漂移见自检段）。

### Persisted Evidence

- None required（Mode: none 满足——验证结果由本 Response 承载，全部命令可低成本重跑，无不可复现现场）

### Experience Candidates

- Issue 候选：None（差分网白名单条目与跳过行为零变化——本 Iteration 未暴露新的范围外缺陷证据；视图/派生定义内嵌于相关子查询时内部限定引用的既有行为在重构前后由全量零回归见证一致，不构成新候选）。
- Runbook 候选：None（M19 分批执行沿用既有约束，无新可复用操作路径）。

### 未解决问题

- Iteration 000 遗留的范围外缺陷处置（①无主键基表 JOIN 空集 / ②TEXT 相关参数 NULL / ④NOT IN NULL 键 / ③聚合派生表列引用拒绝 / ⑤谓词×IN 组合拒绝）仍待用户裁定，本 Iteration 不改变其证据与建议（见 000 Plan Review Follow-up Decision）。
- Iteration 002（T02-C2 JOIN 布局统一）待本 Iteration Plan Review `accepted` 后展开。

---

## Plan Review

- Review Result: replan-required（2026-10-20 用户裁定：碰撞形状语义对齐 SQLite——回归修复与语义演进合并入 replan Cycle，见 Follow-up Decision 与 Next Cycle）
- Reviewer: openspec-plan（本会话）

**Findings**

1. **[阻塞——Plan 遗漏；违反 R2/A7「逐字节不变」] 碰撞形状分类/收集两侧脱节 → 未绑定参数静默 NULL → 静默错误结果回归。**
   - 形状：外层未别名声明表 N + 子查询内层以**别名**声明同表 N（FROM/JOIN 臂、真实表/CTE/视图引用均可触发）+ 子查询内按**基表名 N** 限定引用。IN/EXISTS/标量绑定与投影位标量四子查询面同机制。
   - 实测（本 Review 临时探针 `tests/zz_review_probe.rs`，`git stash push -- src/` 新旧对照，跑完即删；users 三行 id 1..3）：
   
   | 查询 | 旧代码 | 新代码 | SQLite |
   |---|---|---|---|
   | `SELECT id FROM users WHERE EXISTS (SELECT 1 FROM users u WHERE u.v = users.v) ORDER BY id` | `{1,2,3}` | **`{}`（回归）** | `{1,2,3}` |
   | 同形 `u.id = users.id + 1` | `{}` | `{}` | `{1,2}` |
   
   - 机制：`qualified_column_expression` 新条件（`expression.rs:272-276`）在该形状「内层段未命中（段只登记别名 `u`）+ 外层段命中（外层 `users`）+ 名单命中（A1 基表名并列保留）」→ 产出 `ParameterExpression("users.v")`；而收集侧 `collect_outer_column_refs`（`subquery.rs:461`，纯名单判定、本 Iteration 未改）因 `users ∈ inner_table_names` **不收录** → 执行期参数无注入值 → 静默求值 NULL → 谓词 Unknown → 行被过滤。空集无任何错误输出；全量 2045 与差分网 Q5/Q6 均无「外层同名 + 内层别名 + 基名引用」组合，Act 三重验证通道未覆盖，故未暴露。
   - 偏差分类：Plan 遗漏——Guidance ④ 的作用域优先语义与「全部既有输入分类结论逐字节一致」在碰撞形状上不相容（计划未识别）；计划亦未规定收集侧与分类点的判定一致性。Act 按 Guidance ④ 忠实实现。
   - 控制组（同探针）：非碰撞宽松形状（内层 `FROM orders o` + 书写 `orders.m` + 外层无 orders）新旧一致空集——D6 兜底按计划工作；无别名派生显式 `derived.k` 病理形状两侧同款错误 `Execution error: correlated parameter column not found in outer row: k`（分类侧差异不可观察，非回归）。
2. **[Minor] Act Response 计数误差**：「`+116/−3` 行」——实际 numstat 合计 **+115/−3**（12+23+52+25+3）。叙述误差，代码面无歧义，本 Review 计数为准。
3. **[Minor] design §2.2 C1 文本自相矛盾**：「外层作用域表中命中限定符 → 内层引用」与 Plan Guidance ④「仅外层作用域命中 → `ParameterExpression`」方向相反；代码实现循 Guidance ④。设计文本笔误，随本 Review 记录更正，不阻塞。
4. **[核定——通过] 结构逐项独立核对**：五个登记点齐备，`resolve_cte_reference`/`resolve_view_reference` 共用函数覆盖 JOIN 右臂 CTE/视图分支（`query.rs:551`/`:558` 调用即注册点）；六处快照/置界/恢复严格配对（query.rs FROM 派生 / 视图展开 / 投影位标量 + subquery.rs IN / EXISTS / 标量；视图臂恢复先于 `?` 传播，两路覆盖）；`set_table_not_null` 与既有 `set_*` 同键空间（`pipeline.rs:2544`）；新 API 消费面 grep 确认仅 `expression.rs` + `binding.rs`；Preserve 面核实——执行器/存储/网络零改动、PhysicalPlan 词表零变化。
5. **[Issue 候选——已由用户裁定消化] 碰撞形状的既有语义与 SQLite 标准语义相悖**：标准语义中别名遮蔽基表名，`users.id` 应解析到外层（`+1` 变体 SQLite `{1,2}` vs 旧代码 `{}`）——与 Iteration 000 发现 ①②④ 同类的潜在语义缺陷（本次为分类面）。**2026-10-20 用户裁定**：碰撞形状对齐 SQLite 语义（外层相关参数），随 replan Cycle 与本 Finding 1 回归修复合并实施——不作为独立 Issue 落账。

**Acceptance Gaps**

- R2（既有解析语义与错误面逐字节保持）：碰撞形状违反——失败输入、机制与新旧对照见 Finding 1。
- R1/S2（分类消费绑定结果）：身份查找先行机制存在，但「分类结论与名单判定等价」在碰撞形状破裂（分类与收集两侧脱节）。
- 其余无缺口：R1/S1 基建段登记、全量 2045/0/2 零失败、错误断言零校准、差分网白名单冻结——定向套件与全量结论采信（Act 验证后覆盖表面未变化，只读 git 核对一致）。

**Evidence**

- 完整 diff 独立阅读：五 tracked 文件 +115/−3 与 `binding.rs`（76 行）全文；Act Response 各声明（登记点、六处快照配对、dead_code 定点压制、偏差 1-5）逐项与代码比对一致（除 Finding 2 计数误差）。
- 探针实测新旧对照 + SQLite 参考：`cargo test --test zz_review_probe -- --nocapture` 于当前工作区与 `git stash push -- src/` 后各跑一次；输出摘录于 Finding 1；探针文件已删，SQL 与期望值记录于 Follow-up Decision 供 Act 复原为回归测试。
- 只读 git 检查：`git stash pop` 后工作区与 Act 报告现场一致（5 modified + binding.rs/两测试文件/change 目录 untracked），Act 验证结论产生后覆盖表面未变化。
- 采信 Act Response 验证结论（未失效）：六定向套件 + `self_join_test`/`join_output_resolution_test` + 差分网 10/0 + 全量 M19 分批 2045/0/2、600s——均针对非碰撞表面，与本 Finding 未覆盖形状不冲突。

**Follow-up Decision**

2026-10-20 用户裁定（原话：「我们直接给出replan cyc进行修复实施的同时解决碰撞形状的旧语义本身与 SQLite 标准相悖这个issue」）：

1. **语义裁定**：碰撞形状对齐 SQLite 标准——别名遮蔽基表名，子查询内按基表名限定引用解析为**外层相关参数**并正确注入求值。Finding 5 的 Issue 候选由此消化（不另落账）。无外层同名的基名引用（A1 宽松形状，2026-10-15 豁免）维持既有内层解析——裁定范围为碰撞形状，非全遮蔽。
2. **处置路径**：replan-required——语义演进超出本 Cycle「逐字节不变」执行契约（R2/不变量 A7 与 tests 冻结均需修订），不留在当前 Cycle 修复；按 replan 流程修订 delta spec（R1 增碰撞场景、R2 增例外）、design §2.2 C1 与 Iteration 001 计划块后创建后继 Cycle 重新通过 Gate 2 并实施。
3. 本 Review 初稿的「恢复旧语义 + 当前 Cycle 修复」方案被本裁定取代；分类/收集判定基准统一的修复目标并入 replan Cycle 设计（单源谓词 + 作用域快照，避免 `&self` 签名连锁）。

**Iteration Plan Update**

`replan-required`（2026-10-20 用户裁定）——Iteration 001 验收边界修订：

- `sql-semantic-binding` R1 增 Scenario「别名遮蔽基表名的外层同名解析（碰撞形状）」：碰撞形状解析为外层相关参数并注入求值（对齐 SQLite）。
- R2「逐字节保持」增例外：碰撞形状改为外层相关语义（该形状无既有测试覆盖，改动以新增差分与回归测试锁定）；全量既有基线零回归维持。
- 验证契约：新增碰撞形状差分查询（Q5 ×2 / Q6 ×1）与回归测试（`subquery_alias_binding_test` 新增函数，含 A1 宽松控制组）；差分网白名单 6 条目维持冻结。
- Iteration Map 不变（000/001/002 三段，T02-C1 语义边界随本修订）。

**Next Cycle**

`iterations/001-binding-scope/001-replan.md`（T02-C1 修订契约：分类/收集判定单源化 + 碰撞形状 SQLite 语义；Plan Context `ready`——Gate 2 PASS，用户指令「直接给出replan cyc进行修复实施」授权实施）。

**Next Iteration**

None（Iteration 002 待 001 replan Cycle `accepted` 后展开）。
