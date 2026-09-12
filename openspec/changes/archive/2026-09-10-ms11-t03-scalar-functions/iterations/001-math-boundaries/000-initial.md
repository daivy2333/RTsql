# Iteration 001 / Cycle 000-initial: math 函数与调用面收尾

## Plan Context

- Status: draft
- Iteration: 001-math-boundaries
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T4, T5, T6
- Depends on: Iteration 000（accepted：`000-mechanism-strings/000-initial.md`）
- Stable baseline: MS11-T03 全部 Acceptance 关闭（R1-R6）；math 四函数双侧可用；R4 abs 腿与 R2 表头文本断言补全
- Verification boundary: `scalar_function_test` 全绿（含 R3/R5 组）+ cli_test 表头用例绿 + 全量 ≥827 且 0 failed + clippy/fmt 0 + validate PASS
- Diagnostic boundary: `src/executor/function.rs`（math 段与 REGISTRY）+ `tests/scalar_function_test.rs`、`tests/cli_test.rs`
- Deferred tasks: None

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: spec R3/R5 全部场景 + R4/S1、S3 的 abs 腿 + R2/S1-S2 表头文本断言；design D1-D7；Iteration 000 全部不变量
- Excluded scope: 窗口函数、UDF、日期/时间、聚合扩展、ORDER BY 别名排序能力、预存缺陷 A 修复（improvement 候选）

**Objective**

`abs/round/floor/ceil` 注册并双侧可用（R3 语义 + R4 abs 腿）；R5 六个调用面场景锁定；CLI 表头断言补全（R2/S1-S2 文本面）；全量回归收口。

**Background**

tasks MS11-T03 Iteration 001；Iteration 000 Plan Review accepted（2026-09-11，偏差 5/6 明确本 Iteration 承接 R4 abs 腿与 R2 表头文本断言）。math 函数语义按 spec R3（用户裁定：floor/ceil 返回 Float；abs 同型；round half-away-from-zero + SQLite 边缘）。

**Current Baseline**

- Iteration 000 完成后（工作区未提交，基线 179228b + 3 modified + 2 新增）：827 tests pass / 0 failed / 2 ignored、clippy 0、fmt clean、validate 21 PASS（Plan Review 独立复跑，2026-09-11）。
- `src/executor/function.rs`：`REGISTRY` 含 string 六名；`FunctionExpression` 与 D3/D5 契约落地；`eval_scalar` 对未实现臂 `unreachable!` 兜底。
- planner `Expr::Function` 臂与 `ast.rs` 放行门已按注册表驱动——math 名加入 `REGISTRY` 后自动全链路可用，planner/ast.rs 零再改。
- 表头渲染已实证：函数项按书写形态回放（`upper(s)`）、AS 别名生效（`tr`）。

**Current-State Evidence**

（父 Cycle 未变化材料引用其 Act Response 与 accepted Review；以下为本 Iteration 新涉及表面）

- REGISTRY 扩展点：`src/executor/function.rs:19-26` 追加 `("ABS",1,1)`、`("ROUND",1,2)`、`("FLOOR",1,1)`、`("CEIL",1,1)`；`eval_scalar`（function.rs:169-208）追加四臂，`unreachable!` 兜底保留（防注册/实现漂移）。
- math 语义契约（spec R3 断言值）：`abs(Int)→Int`、`abs(Float)→Float`、其他类型 TypeMismatch；`round(x)`= `f64::round`（half-away-from-zero，`2.5→3.0`、`-2.5→-3.0`）+ `10^digits` 乘除（`3.14159` digits 2 → `3.14`）；`digits<0` 整数位舍入（`123.4,-1 → 120.0`）；digits 为 Float 时向零截断为 Int（用户默认假设 6）；`floor/ceil` 返回 Float（Int 入参转 Float，SQLite `floor(3)=3.0`）。round/floor/ceil 入参严格 Int|Float。
- R4 abs 腿落点：`tests/scalar_function_test.rs` 的 `null_propagation_string_functions` 追加 `abs(NULL) → null` 断言；`nested_coalesce_argument` 追加 `abs(-5) → 5` 断言（`-5` 经 `Expr::UnaryOp Minus` 常量折叠臂，父 Cycle Evidence 已确认）。
- R5 路由矩阵（Plan 本轮查证）：S1 `upper(name)='ABC'` 无 PK 等值无 OR → DataScan 下推；S2 含 OR → `contains_or` 命中（父 Cycle Evidence）→ Filter 保留；S3 聚合混用 → SELECT 路由聚合检测先行（query.rs:343-385）→ `InvalidAggregateArgument`；S4 HAVING 标量名 → `build_having_expression` 未识别 → 既有 `UnsupportedExpression`；S5 ORDER BY 别名 → Sort 排序列未命中 Equal 静默保持输入序（sort.rs:82-95，父 Cycle 探针实证）；S6 `id = abs(5)` → `has_pk_equality` 只认等式两侧 Identifier，左侧 `id` 命中 → true → Filter(Scan) 回退——**该路径对显式 PK 表正确**（本轮探针：`WHERE id = length('ab')` → `[2]` 正确；AND 组合对照 `[5]` 正确）；缺陷 A 为隐式 PK 表特有，不适用本场景表形。
- cli_test 表头断言先例：`tests/cli_test.rs:1828-1842`（COALESCE 表头，二进制夹具 `&["app", "SELECT ..."]` + `parsed["columns"]` JSON 断言）。T5 新用例对齐该模式：`upper(name)` 默认表头 + `AS u` 别名表头。
- T6 命令集：`cargo test`（≥827 / 0 failed）、`cargo clippy --all-targets -- -D warnings`（exit 0）、`cargo fmt --check`（clean）、`openspec validate --specs` + `openspec validate 2026-09-10-ms11-t03-scalar-functions`（PASS）。

**Relevant Code**

- `src/executor/function.rs` — REGISTRY、`eval_scalar` math 臂、单测追加。
- `tests/scalar_function_test.rs` — R3 组新增 + R4 腿补全 + R5 组新增。
- `tests/cli_test.rs` — 表头用例追加。
- `src/executor/sort.rs`、`src/parser/planner/query.rs` — R5 场景的既有路由（零修改，验证面）。

**Critical Path**

math 注册 → planner 臂查表命中（零代码改动）→ `FunctionExpression` 构造 → `eval_scalar` 分派 → `Value`。R5 场景沿既有路由矩阵（见 Current-State Evidence）。错误路径同父 Cycle（plan 期 ParseError/UnsupportedExpression；执行期 TypeMismatch 经执行器包装）。

**Implementation Guidance**

- round 实现：`d = digits 向零截断 as i64`；`factor = 10f64.powi(d as i32)`；`(x * factor).round() / factor`——`f64::round` 即 half-away-from-zero，无需自写方向逻辑。digits 为负同式（`10^-1 = 0.1`，`123.4*0.1=12.34 → round 12 → /0.1 = 120.0`，浮点表示误差在 spec 锁定值上不出现）。
- floor/ceil：入参 Int/Float 统一 `as f64` → `f64::floor()/ceil()` → `Value::Float`。
- abs：match Int→`Value::Int(n.abs())`、Float→`Value::Float(f.abs())`。
- R3 e2e 表形：spec GIVEN `t(i INT, f FLOAT)` 与 `t(f FLOAT)`——均无声明 PK，首列 Int 的 `to_key()` 有效，SELECT 无 WHERE 不触发路由问题；不要给 R3 测试加 WHERE 字符串等值。
- R5 e2e 表形：S1/S2 用 spec GIVEN `t(id INT PRIMARY KEY, name VARCHAR)`；S5 需要稳定输入序——DataScan 下推路径按页序产出，两行 INSERT 顺序即产出序（父 Cycle 探针已实证 `B`、`A` 顺序成立）。
- 单测：math 四函数的语义矩阵（abs 同型/类型拒绝、round 方向+digits 正负+Float 截断、floor/ceil Float 形态与 Int 入参）放 function.rs `#[cfg(test)]`。

**Behavioral Change**

- 当前：`abs/round/floor/ceil` 未注册 → `UnsupportedExpression`（SELECT 位置 `UnsupportedStatement`）。
- 目标：四函数双侧可用（spec R3 语义）；R4 两场景 abs 腿补全；R5 六场景行为锁定（其中 S3/S4/S5 为既有行为回归锁）；CLI 表头文本断言落地。
- 接口：REGISTRY +4 名、eval_scalar +4 臂；无新类型、无 planner/ast.rs/pipeline/cli 渲染改动。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T4 | R3/S1-S3、R4/S1、S3(abs 腿) | `src/executor/function.rs`（REGISTRY/eval_scalar/单测）、`tests/scalar_function_test.rs`（R3 组 + R4 腿） | math 未注册 | 注册四名 + 四臂实现 + 单测 + e2e |
| T5 | R5/S1-S6、R2/S1-S2(表头) | `tests/scalar_function_test.rs`（R5 组）、`tests/cli_test.rs`（表头用例） | 既有路由（零修改验证面） | 六场景 e2e + cli 表头断言 |
| T6 | R6/S1-S2 | 全仓库 | — | 全量回归 + lint + validate |

**Task Contracts**

### T4: math 四函数与 R4 腿补全

- Requirement/Scenario: R3/S1（abs 同型）、S2（round 方向与 digits）、S3（floor/ceil Float）；R4/S1（abs(NULL) 腿）、S3（abs(-5) 腿）
- Depends on: None（Iteration 000 机制已就绪）
- Targets: `src/executor/function.rs::REGISTRY/eval_scalar`、`tests/scalar_function_test.rs`
- Current behavior: math 名未注册（SELECT 位置 `Unsupported statement type`、谓词位置 `Unsupported expression type`）；R4 两测试无 abs 腿
- Required behavior: spec R3 全部断言值成立（`abs(-5)=5` Int、`abs(-5.5)=5.5`、`round(3.7)=4.0`、`round(2.5)=3.0`、`round(-2.5)=-3.0`、`round(3.14159,2)=3.14`、`round(123.4,-1)=120.0`、`floor(3.7)=3.0`、`ceil(3.2)=4.0`、`floor(-3.7)=-4.0`、`ceil(-3.7)=-3.0`）；非 Int|Float 入参 TypeMismatch；R4 两腿输出 NULL/5
- Required changes: REGISTRY +4 名；eval_scalar +4 臂；function.rs math 单测矩阵；e2e R3 组 3 测试 + R4 两测试补腿
- Preserve: D3 求值顺序与 NULL 短路（math 不做特殊 NULL 处理）；string 六函数行为与测试零变化；REGISTRY 顺序无关性
- Forbidden: 修改 planner/ast.rs/pipeline/cli/projection.rs/aggregate.rs；round 引入自写方向算法（用 `f64::round`）
- Test witness: function.rs 单测先行 RED（math 未注册 → registry_membership 断言失败）；e2e R3 RED（`Unsupported statement type`）；补腿 RED（abs 未注册）
- GREEN condition: 单测 + R3 组 + R4 两腿全绿
- Verification: `cargo test --lib`、`cargo test --test scalar_function_test`
- Stop when: round 浮点表示误差使 spec 锁定值不可精确判定（如出现需返回 Plan 调整断言值）

### T5: R5 调用面 e2e 与 CLI 表头

- Requirement/Scenario: R5/S1（下推）、S2（OR）、S3（聚合混用）、S4（HAVING）、S5（ORDER BY 别名静默）、S6（PK+函数）；R2/S1-S2（表头文本，偏差 6 承接）
- Depends on: T4（S6 需 abs 注册；其余场景 string 函数已足）
- Targets: `tests/scalar_function_test.rs`（R5 组 6 测试）、`tests/cli_test.rs`（表头用例 1-2 个）
- Current behavior: R5 各场景行为存在但无函数形态锁定；cli_test 无函数表头用例（COALESCE 先例 line 1828）
- Required behavior: S1 仅命中行返回；S2 两行返回；S3/S4 报错（关键词断言）；S5 exit 0、输入序输出、不报错；S6 返回正确行（Filter(Scan) 路径，探针已证）；cli 表头 `upper(name)` 与 `u`
- Required changes: 6 个 e2e 测试 + 1-2 个 cli_test 用例（对齐 line 1828 夹具模式）
- Preserve: 既有路由与 sort.rs/query.rs 零修改；S5 断言"静默保持输入序"而非"按别名排序"；既有 cli_test 零修改
- Forbidden: 为 S5 实现 ORDER BY 别名排序（Out of Scope）；修改 R2/S5 的表形（缺陷 A 规避注记保持）
- Test witness: S3/S4/S5 为既有行为锁（实施前后均绿）；S1/S2/S6 与 cli 表头在 T4 后转绿
- GREEN condition: R5 组全绿 + cli 用例绿
- Verification: `cargo test --test scalar_function_test`、`cargo test --test cli_test`
- Stop when: S5 输入序在两行场景不稳定（DataScan 页序假设失效）——返回 Plan 改用多行确定性形态

### T6: 全量回归清扫

- Requirement/Scenario: R6/S1（全量基线）、S2（既有测试零修改）
- Depends on: T4, T5
- Targets: 全仓库验证命令
- Current behavior: 827 tests / 0 failed / 2 ignored（Iteration 000 后基线）
- Required behavior: 测试总数只增不减（≥827）、0 failed；clippy exit 0；fmt clean；`openspec validate --specs` 21 PASS + change validate PASS
- Required changes: 无代码改动（纯验证任务）
- Preserve: 既有测试文件零修改（R6/S2）
- Forbidden: 为凑数添加无断言测试
- Test witness: 全量命令输出
- GREEN condition: 四项命令全过
- Verification: `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate --specs && openspec validate 2026-09-10-ms11-t03-scalar-functions`
- Stop when: 全量出现非本 change 引入的失败（BASELINE-CHANGED → Blocker Handoff）

**Invariants**

- Iteration 000 全部不变量延续（COALESCE 臂、聚合路径、plan cache 键、渲染层、pipeline/CLI）。
- 既有 827 测试零修改通过；测试总数只增不减。
- planner/ast.rs 在本 Iteration 零修改（math 注册表驱动）。

**Non-goals**

- 预存缺陷 A 修复（improvement 候选，用户决定）；窗口函数、UDF、日期/时间；ORDER BY 别名排序能力。

**Acceptance**

- R3/S1-S3、R5/S1-S6 e2e 全绿；R4/S1、S3 abs 腿断言绿；R2/S1-S2 表头文本断言绿（cli_test）。
- 全量 `cargo test` ≥827 且 0 failed；clippy 0；fmt clean；validate PASS。
- RTM（R3/R5 行收口 + R4/R2 剩余断言腿）：

| R | Scenario | Design | Task | Iter | Code Surface | Test Witness | Status |
|---|---|---|---|---|---|---|---|
| R3 | S1-S3 | D3/D4 | T4 | 001 | `function.rs` math 段 | `scalar_function_test` R3 组 + math 单测 | Covered |
| R4 | S1-S3(abs 腿) | D3/D5 | T4 | 001 | `function.rs` math 段 | R4 两测试补腿 | Covered |
| R5 | S1-S6 | D7 | T5 | 001 | 既有路由（零修改验证面） | `scalar_function_test` R5 组 | Covered |
| R2 | S1-S2(表头) | D6 | T5 | 001 | CLI 渲染（零修改验证面） | `cli_test` 表头用例 | Covered |
| R6 | S1-S2 | — | T6 | 001 | 全仓库 | 全量命令 | Covered |

**Verification**

- 直接观察：e2e 断言行集/列名/错误关键词/退出码；单测断言函数级输入输出。
- 命令：`cargo test`（≥827 / 0 failed）、`cargo clippy --all-targets -- -D warnings`（exit 0）、`cargo fmt --check`（clean）、`openspec validate --specs && openspec validate 2026-09-10-ms11-t03-scalar-functions`（PASS）。
- 无身份型证据工程；Persisted Evidence 见下。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 父 Cycle Act Response + accepted Review（材料未变化）；本 Cycle 补查：`has_pk_equality` 只认 Identifier 侧 + Filter(Scan) 显式 PK 正确探针、REGISTRY/eval_scalar 扩展点、cli_test 夹具先例 |
| Design | PASS | design.md D1-D7 覆盖 math 语义与 R5 路由；spec R3 断言值明确 |
| Iteration Plan | PASS | tasks.md Iteration 001 + 平衡审计（父 Cycle 已批，Map 未变） |
| Cycle Scope | PASS | Cycle Scope 节（initial；R4 腿/R2 表头承接自偏差 5/6） |
| Task Contracts | PASS | T4-T6 契约（断言值逐一列出、路由矩阵、停止条件） |
| Traceability | PASS | RTM 收口行全 Covered |
| Verification | PASS | Verification 节（目标行为验证，无身份型工程） |

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证；无不可低成本复现产物。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- round 负 digits 依赖 `10f64.powi(-1)=0.1` 的浮点表示——spec 锁定值（120.0）在 double 精度下无误差；若 Act 实测出现边界值偏差，按 T4 Stop when 返回 Plan。
- S5 的两行输入序依赖 DataScan 页序产出（父 Cycle 探针实证）；若不稳定按 T5 Stop when 处理。
- 非实质选择留给 Act：math 单测组织、e2e 测试命名、cli 用例数量（1-2 个）。

## Act Response

- Status: reported

**Implemented**

- T4 math 四函数：`REGISTRY` 追加 `("ABS",1,1)`、`("ROUND",1,2)`、`("FLOOR",1,1)`、`("CEIL",1,1)`（registry 驱动，planner arity/OVER/DISTINCT 等拒绝面自动生效，planner 函数臂与 ast.rs 注册名放行门零再改）；`eval_scalar` 追加四臂——ABS 同型返回（Int→Int/Float→Float，其他类型 `ValueError::TypeMismatch`）、ROUND `f64::round`（half-away-from-zero）+ `10^digits` 乘除（digits Int 或 Float，Float 按向零截断 `as i64`；负 digits 整数位舍入）、FLOOR/CEIL 入参 Int/Float 统一 `as f64` 后 `f64::floor()/ceil()` 返回 `Value::Float`；新增 `float_arg` helper（Int|Float 严格校验，无字符串解析）。D3 求值顺序与 NULL 短路沿用 `eval_owned` 既有层，math 无特殊 NULL 处理。CEIL/FLOOR 因独立 sqlparser 变体需接线（见 Deviations 2）：`ast.rs` 两放行门追加 `Expr::Ceil/Expr::Floor`，planner 新增 `Expr::Ceil/Expr::Floor` 两臂 + `build_ceil_floor` 共享 helper（仅放行 `fn(x)` 纯形态，`TO DateTimeField` 形态 plan 期点名拒绝）。
- T5 R5 调用面 e2e + CLI 表头：`tests/scalar_function_test.rs` 追加 R5 组 6 测试（S1 DataScan 下推、S2 OR Filter 路径、S3 聚合混用既有拒绝 `Invalid aggregate argument`、S4 HAVING 标量名既有拒绝 `Unsupported expression type`——`build_having_expression` 只认聚合五名，注册标量名不改变该路径（aggregate.rs:88 本轮核实）、S5 ORDER BY 别名静默保持输入序既有语义锁、S6 `WHERE id = abs(5)` Filter(Scan) 普通路径正确）；`tests/cli_test.rs` 追加 2 表头用例（`SELECT id, upper(name)` 默认表头 `upper(name)` + 行值 `ALICE`；`SELECT upper(name) AS u` 别名表头 `u`），对齐 COALESCE 表头先例夹具模式（cli_test.rs:1828）。
- T6 全量回归清扫：`cargo test` 845 passed / 0 failed / 2 ignored（Iteration 000 后基线 827 + 本 Iteration 新增 18：math 单测 6 + e2e R3 组 3 + TO 拒绝锁 1 + R5 组 6 + cli 表头 2；既有测试零修改，总数只增不减）；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` clean；`openspec validate --specs` 21 passed / 0 failed + `openspec validate 2026-09-10-ms11-t03-scalar-functions` valid。

**Changed Files and Symbols**

- `src/executor/function.rs`：`REGISTRY` +4 名；`eval_scalar` ABS/ROUND/FLOOR/CEIL 四臂；新 `float_arg`；模块文档注释更新（十函数全注册）；单测 +6（`math_arity_checks`、`abs_keeps_input_type`、`round_half_away_from_zero_with_digits`（含 `#[allow(clippy::approx_constant)]`）、`round_accepts_int_forms`、`floor_ceil_return_float`、`math_null_argument_yields_null`）+ `registry_membership` 扩至十名。
- `src/parser/planner/expression.rs`：新 `PlanBuilder::build_ceil_floor` helper + `Expr::Ceil`/`Expr::Floor` 两臂（偏差 2）。
- `src/parser/ast.rs`：`extract_columns`/`extract_qualified_columns` 两放行门 or-list 追加 `Expr::Ceil { .. }`/`Expr::Floor { .. }`（偏差 2）。
- `tests/scalar_function_test.rs`：R3 组 3 测试 + `ceil_floor_to_datetime_field_rejected` 拒绝锁 + R4 两测试补 abs 腿 + R5 组 6 测试 + 文件头见证范围注记更新。
- `tests/cli_test.rs`：+2 表头用例（`test_scalar_function_display_header`、`test_scalar_function_alias_header`），既有用例零修改。
- 禁止面核对（本 Cycle）：`src/pipeline.rs`、`src/cli/`（产品代码）、`src/executor/projection.rs`、`src/executor/aggregate.rs`、`src/parser/planner/query.rs`、`src/executor/sort.rs` 零修改（`git diff --stat` 仅 4 modified 文件 + 2 untracked，其中本 Cycle 触及 function.rs/expression.rs/ast.rs/scalar_function_test.rs/cli_test.rs）；无哈希/校验和。

**Deviations from Plan**

1. **执行授权记录**：Act 启动时本 Cycle Plan Context 状态字段为 `draft`（Gate 2 Readiness 表全 PASS）。用户于 2026-09-11 明确指令「更改gate，开始实施」，按 CLAUDE.md 用户豁免规则以该原话作为计划获批记录（与 Iteration 000 偏差 1 同例）；Act 未改写 Plan Context。
2. **CEIL/FLOOR 独立 sqlparser 变体接线（计划不变量违反）**：Plan 不变量「planner/ast.rs 在本 Iteration 零修改（math 注册表驱动）」对 abs/round 成立，对 ceil/floor 不成立——sqlparser 0.44 将 `CEIL(x)`/`FLOOR(x)` 解析为专用变体 `Expr::Ceil { expr, field }`/`Expr::Floor { expr, field }`（非 `Expr::Function`，`field` 为 `DateTimeField`），不接线则 SELECT 位置被 ast.rs 放行门拒绝（RED 实测 `Plan error: Unsupported statement type`）、谓词位置落 build_expression catch-all，R3/S3 无法达成。按本 change Iteration 000 已接受的 Trim 先例（父 Cycle 偏差 4）补齐：ast.rs 两门放行 + planner 两臂 + TO 形态点名拒绝。Plan 遗漏分类交 Plan Review。
3. **R4/S3 abs 腿断言为两行**：spec AND 子句「`SELECT abs(-5) FROM t` 输出 `5`」在 GIVEN 两行表上逐行产出 `[[5],[5]]`（常量函数项逐行求值）；初版断言单行 RED 后修正，值语义与 spec 一致。
4. **`#[allow(clippy::approx_constant)]` 3 处**：spec 锁定值 `3.14`/`3.14159` 触发 clippy approx_constant（近似 PI），按仓库 scoped-allow 惯例（page_visibility.rs 先例）在测试函数上豁免并注明理由（锁定值非 PI）。
5. **Ceil/Floor TO 形态拒绝锁 e2e 追加**：偏差 2 引入的新拒绝面（`TO DateTimeField`）以 `ceil_floor_to_datetime_field_rejected` 锁定，镜像父 Cycle `trim_specification_rejected` 先例；T4 契约未列，属新拒绝面的行为见证面。

**Blocker Handoff**

- None required（未发生 Gate 6 阻塞）。

**Blocker Resolution**

- None（未阻塞）。

**Self-Review**

- Plan compliance: PASS（除上述 5 项已记录偏差；偏差 2 为达成 R3/S3 的必要接线，按 change 内既有先例处理；RTM R3/R4/R5/R2 表头/R6 行的代码面与测试见证齐备）。
- Full diff reviewed: PASS（本 Cycle 增量逐 hunk 审查：REGISTRY 四名与 eval_scalar 四臂对应无漂移、`f64::round` 无自写方向算法、D3/NULL 层未动、string 六函数语义与测试零变化、`build_having_expression`/`sort.rs`/`query.rs`/渲染/pipeline/plan cache 键零触碰、ceil/floor 三位置（SELECT/WHERE/HAVING catch-all）行为一致、fmt 后最终状态复核）。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings resolved: ① e2e `nested_coalesce_argument` abs 腿初版单行断言错误（偏差 3）已修；② clippy approx_constant 3 处（偏差 4）与 doc 头注 `doc_lazy_continuation`（文件头一行以 `+` 起始被识别为列表）已修；③ fmt 3 处 diff 已应用。
- Minor findings open: 无（abs `i64::MIN` 与 round 极端 digits 边界记入 Remaining Issues，均不在 spec 锁定面内）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| RED 见证（单测） | `cargo test --lib executor::function`（T4 实施前） | `FAILED. 12 passed; 6 failed`（`registry_membership` 断言失败 + math 语义测试命中 drift-guard `unreachable!`） | math 未注册基线 | RED 已见 |
| RED 见证（e2e） | `cargo test --test scalar_function_test`（T4 实施前） | `FAILED. 16 passed; 5 failed`（R3/S1-S3 + R4 两腿全部 `Plan error: Unsupported statement type`） | math SELECT 位置既有拒绝 | RED 已见 |
| 单元测试 | `cargo test --lib executor::function` | `test result: ok. 18 passed; 0 failed` | 十函数语义/arity/NULL 层/D5 递归 | PASS |
| e2e（本 change） | `cargo test --test scalar_function_test` | `test result: ok. 28 passed; 0 failed` | R1-R5 全部场景 + 拒绝锁 + 回归锁 | PASS |
| CLI 集成 | `cargo test --test cli_test` | `test result: ok. 56 passed; 0 failed; 2 ignored` | 表头文本断言 + 既有 CLI 面 | PASS |
| 全量回归 | `cargo test` | `TOTAL passed=845 failed=0 ignored=2` | 全仓库（基线 827 + 新增 18，只增不减，既有测试零修改） | PASS |
| Lint | `cargo clippy --all-targets -- -D warnings` | exit 0 | 全仓库 | PASS |
| 格式 | `cargo fmt --check` | clean | 全仓库 | PASS |
| OpenSpec | `openspec validate --specs` + `openspec validate 2026-09-10-ms11-t03-scalar-functions` | `Totals: 21 passed, 0 failed (21 items)`；`Change ... is valid` | 全部 capability specs + 本 change delta | PASS |

**Persisted Evidence**

- None required（Plan 设定 `Mode: none`；全部验证可低成本复现，命令与决定性输出已录入本 Response）。

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| — | None | — | 无满足 Runbook/Incident 门槛的可复用操作路径或故障现场 |

**Remaining Issues**

1. **abs 极值边界（不在 spec 锁定面）**：`abs(-9223372036854775808)`（i64::MIN）按计划契约实现为 `i64::abs()`，debug 构建下溢出 panic（release 回绕）。spec R3 仅锁定 `abs(-5)`/`abs(-5.5)` 与严格类型；如需产品化防御（如显式溢出错误）另立任务，交 Plan Review 裁定。
2. **round 极端 digits 边界（不在 spec 锁定面）**：按计划契约公式 `10f64.powi(digits as i32)`，|digits|>308 时 factor 溢出为 inf/0，结果为 inf/NaN（SQLite `round(1,1000)=1.0`）。spec 仅锁定 ±1 与 2 位场景。
3. **`openspec validate --specs` 既有 WARNING**：`overview: Purpose section is still a placeholder`——主 specs 存在归档时留下的 Purpose 占位（多个既有 spec 含 TBD/TODO 标记），本 change 未触碰任何主 spec（worktree `openspec/specs/` 零修改），21 passed / 0 failed 不受影响；属 docs-maintainer 收尾面。
4. **预存缺陷 A**（父 Cycle 定性，未修复、与函数无关）：无声明 PK 表首列字符串 Eq 静默空结果——improvement 候选，维持父 Cycle Review 建议（登记由用户决定）。
5. **M36 既有限制**：`ParameterExpression::evaluate_ref` 对 String 参数值返回 Null（predicate.rs:426 `M37 TODO`）——本 Cycle 未触碰。

**Commit or Diff Reference**

- 未提交（工作区：4 modified + 2 untracked 新文件 + change 目录；commit 由用户触发）。对照基线 `179228b`（master，Iteration 000 与本 Cycle 均在其上工作区累积）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查（revision 基线 179228b + 工作区 4 modified + 2 新增；非采信 Act Self-Review）：

- 代码审阅：`function.rs` math 段逐项核对——REGISTRY 四名（`ROUND(1,2)` 其余 `(1,1)`）与 spec arity 契约一致；`eval_scalar` 四臂语义与 spec R3 断言值逐条对应（abs 同型 `Int→Int/Float→Float`、round 用 `f64::round` 半数远离零 + `10^digits` 乘除、digits Float 向零截断 `as i64`、floor/ceil 统一 `as f64` 后返回 `Value::Float`）；`float_arg` 严格 Int|Float 无字符串解析；math 无特殊 NULL 处理（D3 层 `eval_owned` 共享，错误先传播、NULL 短路跳过类型校验——计划契约保持）。`build_ceil_floor` 仅放行 `NoDateTime` 纯形态，TO 形态点名拒绝且文案含函数名；args 经 `build_expression` 递归（嵌套可用）。
- planner/ast 接线核对：`ast.rs` 两放行门 or-list 追加 `Expr::Trim/Ceil/Floor`；函数名门两处均先 `to_uppercase()` 再查 `is_scalar_function`（spec R1 大小写不敏感在实现层成立，见证缺口见 F1）。未注册名两门维持 `_ => Err(UnsupportedStatement)` 既有拒绝。
- 测试核对：e2e 28 测试与 spec 场景一一对应（R1×4 + 3 文本拒绝锁 + 2 回归锁、R2×6 + TRIM 规格化拒绝锁、R3×3 + TO 拒绝锁、R4×3 含 abs 腿、R5×6）；R2/S4 五个 substr 断言、R3/S2 五个 round 断言与 spec 逐字一致；cli_test +26 行仅 2 个新表头用例（既有用例零修改，对齐 COALESCE 先例夹具）。
- 禁止面：`git diff --stat` 仅 4 modified + 2 untracked；`src/pipeline.rs`、`src/cli/`（产品代码）、`projection.rs`、`aggregate.rs`、`query.rs`、`sort.rs` 零触碰；无哈希/校验和。既有 expression/predicate/planner 测试文件零修改（R6/S2 成立）。
- R5/S4 路径复核：`build_having_expression`（aggregate.rs:26-125）`Expr::Function` 臂仅认聚合五名、标量名落 `_ => UnsupportedExpression`——Act「注册标量名不改变该路径」的核实结论成立，且该文件零修改。
- 独立复跑验证：`cargo test` → passed=845 failed=0 ignored=2（基线 827 + 18，只增不减）；`cargo test --test scalar_function_test` → 28 passed；`cargo test --lib executor::function` → 18 passed；`cargo test --test cli_test` → 56 passed / 2 ignored；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` clean；`openspec validate --specs` 21 passed / 0 failed（Purpose 占位 WARNING 为既有遗留，见 Remaining Issues 3）+ `openspec validate 2026-09-10-ms11-t03-scalar-functions` valid。与 Act Response 全部一致。

- F1（非阻塞）：spec R1 要求文本「函数名匹配 SHALL 大小写不敏感」无 SQL 层测试见证——e2e 全部使用小写形态，tests/ 全目录无 `UPPER(`/`ABS(` 等大写变体调用（grep 实证）。实现层已核实成立（两处门均规范化大写），但该 SHALL 子句在 Gate 1 锁定的 23 场景中无对应场景、两轮 RTM 均按场景映射未覆盖。属 spec 文本与场景覆盖缺口（Iteration 000 R1 场景已 accepted，非本 Iteration Acceptance 缺口）。交 docs-maintainer 收尾时由用户决定：向 delta spec 补一个大写变体场景，或登记 improvement。
- F2（非阻塞，正向核实）：偏差 2 的技术前提独立成立——sqlparser 0.44 将 `ceil(x)`/`floor(x)` 解析为 `Expr::Ceil/Floor { expr, field }` 专用变体（接线前 RED 实测 `Unsupported statement type` 与 ast.rs 门逻辑吻合），Act 的补救是达成 R3/S3 的必要面且最小。
- F3（非阻塞）：预存缺陷 A、abs `i64::MIN`、round 极端 digits、主 spec Purpose 占位 WARNING、M36 限制均正确停留在 Remaining Issues，未越界修复；improvement 登记候选留 docs-maintainer 收尾。

**Deviation Classification**

- 偏差 1（draft 状态下用户「更改gate，开始实施」指令）：用户豁免记录（原话保留于 Act Response），与 Iteration 000 偏差 1 同例，不计入偏差分类。
- 偏差 2（CEIL/FLOOR 独立变体接线，违反计划不变量「planner/ast.rs 本 Iteration 零修改」）：**PLAN-OMISSION**——Plan 的 Current-State Evidence 断言「math 名加入 REGISTRY 后自动全链路可用」时漏查 sqlparser 0.44 对 ceil/floor 的专用变体解析，不变量对这两个名字事实错误；Act 按本 change 已接受的 TRIM 先例（父 Cycle 偏差 4）最小补救（两门放行 + 两臂 + 共享 helper + TO 点名拒绝），处理正确，非阻塞。
- 偏差 3（R4/S3 abs 腿两行断言）：**PLAN-OMISSION**（Minor）——spec AND 子句在两行 GIVEN 上常量函数项逐行产出，Act 初版单行断言 RED 后修正，值语义与 spec 一致。
- 偏差 4（`#[allow(clippy::approx_constant)]` 3 处 scoped 豁免）：非实质，仓库 scoped-allow 惯例，理由注明（锁定值非 PI）。
- 偏差 5（TO 形态拒绝锁 e2e 追加）：**NEW-EVIDENCE**——偏差 2 新引入拒绝面的行为见证面，镜像父 Cycle `trim_specification_rejected` 先例，合理。

**Acceptance Gaps**

- 无：R3/S1-S3、R5/S1-S6 e2e 全绿；R4/S1、S3 abs 腿断言绿；R2/S1-S2 表头文本断言绿（cli_test）；T6 全量基线达成（845 ≥ 827 且 0 failed、既有测试零修改、总数只增不减、clippy/fmt/validate 全过）。RTM 收口行（R3/R4/R5/R2 表头/R6）全部满足。

**Convergence**

N/A（本 Cycle 首次 Review，无前次 Acceptance gap 可比较）。

**Evidence**

- 复跑：`cargo test` → passed=845 failed=0 ignored=2；scalar_function_test 28 passed；lib executor::function 18 passed；cli_test 56 passed / 2 ignored；clippy exit=0；fmt clean；validate 21 PASS + change valid（2026-09-11 独立复跑）。
- 代码：`src/executor/function.rs`（REGISTRY/eval_scalar math 四臂/float_arg/18 单测）、`src/parser/planner/expression.rs`（build_ceil_floor + Ceil/Floor 臂 diff）、`src/parser/ast.rs`（两放行门 diff）、`tests/scalar_function_test.rs`（28 测试）、`tests/cli_test.rs`（+2 用例 diff）。
- 路径复核：aggregate.rs `build_having_expression` 只读核实（R5/S4）；`git diff --stat` 禁止面核对；tests/ 大写变体 grep（F1）。

**Follow-up Decision**

接受（accepted）：全部 finding 非阻塞（F1 为 spec 覆盖缺口记录，不要求当前 Cycle 修复）；Act 的 5 项偏差处理均正确且在授权范围内；Iteration 001 Acceptance 达成，change 两 Iteration 全部完成。无当前 Cycle 修复项。docs-maintainer 收尾待决事项：F1 大小写见证（补场景或登记 improvement，用户决定）、预存缺陷 A 与 abs/round 极端边界登记 improvement、主 spec Purpose 占位清扫；commit 由用户触发。

**Iteration Plan Update**

None（Iteration Map 不变；两 Iteration 均完成）。

**Next Cycle**

None。

**Next Iteration**

None——change Map 无剩余 Iteration，实施完成；待用户审计后由 `openspec-docs-maintainer` 收尾（状态同步、improvement 登记、delta spec 合并、归档）。
