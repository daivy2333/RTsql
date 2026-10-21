# Iteration 002 / Cycle 000: JOIN 组合行布局与作用域表统一（T02-C2）

## Plan Context

- Status: ready（2026-10-20 Gate 2 自评 PASS，用户批准计划）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T02（C2 面）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-semantic-binding` R1 Scenario「JOIN 链绑定身份确定」布局消费段（Iteration 001 已交付登记基建——五登记点 / 六处快照恢复 / `table_not_null` 通道 / `is_outer_reference` 判定单源，replan Cycle Review `accepted` 2026-10-20）；R3（架构边界：词表 / 执行器 / EXPLAIN 形状不变）全部场景；R2 零回归不变量维持；proposal A5（增量式绑定）、A7（零回归硬边界）；design D5（PlanBuilder 内加性承载）；差分网 GREEN 保护基线（10/0，白名单 6 条目冻结）
- Excluded scope: 执行器接口与 `condition_column_index`（执行期按名定位留后续批次）；Hash JOIN 条件键 `ColumnRef` 通道；剩余按名解析点收口（I080 后续批次）；plan cache 语义；视图展开 / CTE / attach 段登记完备化（D6 兜底维持）；投影臂无别名派生外层段登记不完备（C1 痕迹，书写限定符不可达——见 001-replan Plan Review 遗留移交）；差分白名单条目增删与 ①-⑤ 缺陷处置

**Objective**

`join_column_layout` 的段键与侧别判定自绑定作用域表窗口派生（段键空间单源：声明限定符 + 声明序只存在作用域表一处）；`current_qualifiers` 平行记账收敛移除；全部既有输入的计划形状（EXPLAIN 输出）与行为、错误面逐字节不变（等价重构，重构 TDD 形态：先观察 GREEN 保持）。

**Investigation Facts**

- Current Baseline: 工作区 = Iteration 001 replan `accepted` 现场（未提交：planner 五文件 + `binding.rs` + `pipeline.rs` + `tests/subquery_alias_binding_test.rs` + 两差分/性质测试文件 + 本 change 目录）；全量 2046 passed / 0 failed / 2 ignored（replan Act，M19 分批 14 批实测）；差分网 10/0、白名单 6 条目冻结；`cargo fmt -- --check` 全净。
- Current-State Evidence（2026-10-20 本会话实读，行号当前有效；Act 无需回读其他资料）:
  - **布局字段与消费**：`join_column_layout: Option<Vec<(String, Vec<String>)>>`（`mod.rs:202`）——与 `binding_scopes` 平行的段结构。两消费点在 `expression.rs`：限定名臂 :294-311（段键匹配经 `qualifier_matches_declared`——原文相等或别名归一命中，含 attach 隐式短名 `u` ↔ `o.u`；未命中段 `TableNotFound`、列未命中 `ColumnNotFound`）、裸名臂 :350-370（全布局搜索，多段命中 `AmbiguousColumn`）。
  - **五个布局窗口（query.rs 置位 → 编译 → 清除，清除含错误路）**：① NLJ ON 臂 :815-853——段清单自 `current_qualifiers`/`current_tables` zip + 右臂构造，RIGHT 交换变体把右臂段排前（:815-824）；② `join_layout` 构造 :1601-1603——`join_combined_columns(&base_plan)`（基座计划实际输出列）+ `combined_qualifiers` 经 `join_row_layout`（:3475），:1905 置位（Projection 纯列项编译）/:1934 清除；③ WHERE×JOIN :2077-2080——同款 `join_row_layout`；④ ORDER BY 别名键表达式 :2892/:2911——复用 ② 的 `join_layout`；⑤ 无 FROM Projection :3129-3154——空布局 `Some(Vec::new())` save/restore（既有 `ColumnNotFound`/`TableNotFound` 错误面载体）。
  - **侧别判定**：`extract_join_conditions`（`ddl_dml.rs:181-254`；Hash 路由调用点 `query.rs:786-793`，参数 `current_qualifiers`/`right_qualifier` 自 query.rs 穿线）；`expr_join_side`（`ddl_dml.rs:263-284`）——限定腿按 `qualifier_matches_declared` 匹配左链声明限定符（any → 左输入）/ 右臂（→ 右输入）/ 均未命中 `TableNotFound`；裸名腿 `None` 交调用方按书写位置定侧；三段 / 表达式腿在 Hash 接受面外（`is_pure_equi_join_on` `ddl_dml.rs:120-134`）。
  - **`current_qualifiers` 全消费点**（grep 证实仅五处）：:504 初始化、:645 `all_qualifiers` 逐列限定符清单构造（段键按各臂列数展开）、:789 `extract_join_conditions` 参数、:819-822/:826-829 NLJ 布局构造、:900 右臂 push。`current_tables` 另有非布局用途（:613 起输出列构造、:844 `build_where` 基表参数）——**不属键记账，保留**。
  - **收窄过滤事实（关键设计边界）**：末步非 `carry_all_columns` 时 `output_columns`/`output_qualifiers`/`output_sides` 按 SELECT 清单 kept 过滤（:731-776 段）——窗口 ②③④ 的布局反映基座计划**实际输出列**（可能为逐列子集），非全臂段。窗口 ①（NLJ ON）与侧别判定用**全臂列**、声明序——恰等于作用域表窗口段。
  - **作用域表登记序 = 声明序**：`build_from_clause_with_projection`（`query.rs:367` 起）内基表臂 :417-427、JOIN 右臂真实表 :577-585、派生臂 :480、CTE `resolve_cte_reference` :1312、视图 `resolve_view_reference` :1375——逐臂登记且**先于同步 ON 分支**（:611/:786 前）；键空间同为声明限定符（design D4 与 C1 同键空间）。ON 分支时点的作用域表尾窗 `[entry..]`（entry = FROM 构造入口长度）= 左链各臂段 + 本步右臂段，序与内容同 `current_qualifiers`+右臂。
  - **matcher 差异（设计约束）**：布局/侧别消费用 `qualifier_matches_declared`（别名归一命中，attach 隐式短名面）；C1 分类用 `binding_scope_hit`（原文大小写不敏感相等——父 Cycle 偏差 4 论证别名归一会误命中内层段）。段键空间统一以**来源统一**为界，matcher 语义各自冻结。
  - **投影臂临时段与窗口边界**：投影位循环先于 FROM 构建，其外层段临时登记（空列清单）在臂退出即截断；子查询内层 FROM 构建时作用域表含临时段，但临时段位于该构造入口**之前**——窗口取 `[构造入口长度..]` 天然排除（`binding_scope_boundary` 同款纪律先例）。
- Code and Critical Path: `src/parser/planner/query.rs`（五窗口 + `all_qualifiers` 派生 + `current_qualifiers` 收敛）、`src/parser/planner/ddl_dml.rs`（`extract_join_conditions`/`expr_join_side` 判定源切换）、`src/parser/planner/binding.rs`（窗口快照/派生 helper 如需，加性）、`src/parser/planner/mod.rs`（字段与 API 宿主）、`src/parser/planner/expression.rs`（消费点——matcher 不动，仅段来源变化不可见）。消费链：FROM/JOIN 装配登记作用域段 → 布局窗口自作用域表派生段键 → `expression.rs` 消费（matcher 不变）→ 计划形状不变。

**Implementation Guidance**

建议顺序 ①→⑤（① 纯观察，②③ 为等价重构主体）：

1. **窗口快照纪律**：`build_from_clause_with_projection` 入口取 `layout_scope_entry = self.binding_scopes.len()`（局部变量——同步构造、无跨 await）；各窗口在**置位点一次性派生并物化**段清单（非编译期活查询），与现行 `join_column_layout` 置位快照语义一致；子查询臂构建期间的 scope restore 不影响已物化副本。
2. **窗口 ① 与侧别判定切换**：NLJ ON 布局段 = `binding_scopes[entry..]`（限定符 + 列清单直取；RIGHT 交换变体在派生后重排，同现行 :815-824）；`extract_join_conditions`/`expr_join_side` 改查窗口（左 = 窗口除末段各键，右 = 末段键），匹配仍经 `qualifier_matches_declared`；匹配语义、裸名定侧、`TableNotFound` 面零变化。
3. **`all_qualifiers` 派生与 `current_qualifiers` 移除**：:645 逐列限定符清单改自 `binding_scopes[entry..]` 段键按段列数展开（左链段 + 本步右臂段，序不变）；kept 过滤（:731-776）在派生清单上照旧进行；全部消费点切换后删除 `current_qualifiers`（:504/:900 记账与 :789/:819-830 构造面）。
4. **窗口 ②③④ 列源保持**：布局 = 基座计划实际输出列（`join_combined_columns` + `join_row_layout` 照旧）——统一的是段键/顺序的来源（经 ③ 派生的 `combined_qualifiers`），**不得**以作用域表全列段替换收窄后布局（列内容是「实际输出行形状」，与「声明段全列」是两个事实）。
5. **窗口 ⑤ 维持**：空窗口派生等价于 `Some(Vec::new())`，错误面零变化。
6. 等价重构验证：定向 JOIN 套件 → 差分网 → `explain_test` 零校准 → 全量分批一次。

**Task Contracts**

### T02-C2: JOIN 布局段源自作用域表统一与侧别判定切换

- Requirement/Scenario: `sql-semantic-binding` R1 Scenario「JOIN 链绑定身份确定」布局消费段（全部）；R3 Scenario「物理计划词表不变」（全部）；R2 零回归不变量维持
- Depends on: Iteration 001（作用域表基建 + 判定单源，`accepted`）
- Targets: `src/parser/planner/query.rs`（五布局窗口 / `all_qualifiers` 派生 / `current_qualifiers` 移除）、`src/parser/planner/ddl_dml.rs::extract_join_conditions/expr_join_side`、`src/parser/planner/binding.rs`（窗口派生 helper，加性如需）、`src/parser/planner/mod.rs`（记账字段收敛）
- Current behavior: 布局段与侧别判定自 `current_qualifiers`/`right_qualifier`/`combined_qualifiers` 平行记账构造——与作用域表同键空间、同登记序，但双源无结构保证（I080 根因同族形态：正确性依赖装配点两处记账一致）
- Required behavior: 段键与声明序的唯一事实源 = 绑定作用域表窗口（`binding_scopes[构造入口..]`）；NLJ ON 布局段与侧别判定直查窗口；逐列限定符清单（`all_qualifiers` 谱系）自窗口段键派生；`current_qualifiers` 移除；全部既有输入 EXPLAIN 输出、行为与错误面逐字节不变
- Preserve: PhysicalPlan 节点词表与计划形状（`explain_test` 零校准）；`expression.rs` 两消费点 matcher 语义（`qualifier_matches_declared`——attach 隐式短名 / 别名归一面）；执行器 / 存储 / 网络零改动；既有 2046 基线零回归；差分网白名单 6 条目与跳过行为冻结；C1 交付面原样（`is_outer_reference` 单源、五登记点、六处快照恢复、投影臂临时登记）；`binding_scope_hit` matcher 冻结（分类语义不变）
- Forbidden: 执行器文件；`condition_column_index` 与 `ColumnRef` 通道；错误消息文本任何校准；白名单条目增删；任何输入的语义演进（含以作用域表全列段替换收窄布局）；`current_tables` 移除（非键记账，有输出列构造与 `build_where` 基表用途）
- First test: 重构类（行为保持型）——先行测试 = 既有定向 JOIN/别名/CTE/视图/attach 套件 GREEN 基线观察（replan Act 全量 Gate 已见证 2046/0/2，直接采信）+ 差分网 JOIN/自连接/链式面 GREEN；重构后保持 GREEN，无 RED 形态
- GREEN condition: 定向套件（`join_output_resolution_test` / `self_join_test` / `chain_join_test` / `nested_loop_join_test` / `outer_join_test` / `subquery_alias_binding_test` JOIN 形态 / `table_alias_resolution_test` / `cte_test` / `view_test` / `cross_db_query_test` / `attach_session_test`）全绿；`explain_test` 零校准；`cargo test --test sql_differential_test` 全绿（白名单 6 条目不变）；全量 `cargo test`（M19 分批一次）零失败零校准
- Verification: ① 定向套件命令与结果；② `cargo test --test explain_test`（R3 计划形状）；③ 差分网（白名单冻结）；④ 全量分批一次 rc=0
- Stop when: 任一输入计划形状或行为必须变化才能容纳窗口派生（等价性假设失效 → 返回 Plan）；派生必须触碰执行器接口、`expression.rs` matcher 或 `binding_scope_hit`；作用域登记序与逐列限定符序出现不一致组合（套件/差分暴露 → 报告不擅自扩面）；`current_qualifiers` 移除迫使非布局消费点（输出列构造 / `build_where`）改语义

**Invariants**

- A7 零回归硬边界：既有 2046 基线零失败，错误面（类型、文本、SQLSTATE）逐字节不变，EXPLAIN 输出零校准。
- 段键空间统一以「来源统一」为界：作用域表为段键与声明序唯一事实源；matcher 语义按消费点各自冻结（`qualifier_matches_declared` vs `binding_scope_hit`）。
- 窗口 ②③④ 布局列源 = 基座计划实际输出列（收窄后子集也是事实本身），不与全臂段混同。
- 加性承载（D5）：PlanBuilder 公共 API 仅加法；窗口派生 helper 为 `pub(crate)` 内部演进。
- 固定种子差分网确定性维持；白名单条目集与跳过行为冻结。

**Non-goals**

执行器接口与 `condition_column_index`；Hash JOIN 条件键 `ColumnRef` 通道；剩余按名解析点收口（I080 后续批次）；plan cache 语义；视图 / CTE / attach 段登记完备化（D6 兜底维持）；投影臂无别名派生外层段登记不完备（C1 痕迹——书写限定符不可达，001-replan Review 遗留移交，纯记录）；差分白名单 ①-⑤ 缺陷处置（待用户裁定）。

**Acceptance**

- R1/S1 布局消费段（JOIN 链绑定身份确定）：布局段与侧别判定消费绑定作用域表窗口（代码结构 + Review 核对「段键与声明序单源」）；JOIN / 自连接 / 链式 / 别名 / CTE / 视图 / attach 隐式短名矩阵全绿。映射：T02-C2 / Guidance ②③。
- R3/S1（物理计划词表不变）：`explain_test` 零校准 + 执行器 / 词表 diff 零改动。映射：全量 Gate + Review diff 核对。
- R2 维持（全量零回归 + 错误面逐字节）：全量零失败、错误断言零校准。映射：全量 Gate。

**Verification**

| Scenario | 判定 | 通过依据 |
|---|---|---|
| R1/S1 布局消费（窗口派生） | 定向 JOIN/别名/CTE/视图/attach 套件 + 差分网 JOIN/自连接/链式面 | 全绿 |
| R1/S1 侧别判定（窗口查询） | `join_output_resolution_test` / `self_join_test` / `chain_join_test`（两腿异侧、同侧拒绝、裸名定侧） | 全绿 |
| R3 计划形状 | `explain_test` + EXPLAIN 输出零校准 | 零校准 |
| R2 全量零回归 | 全量 `cargo test`（M19 分批一次） | rc=0、零校准 |
| 单源结构（`current_qualifiers` 收敛） | 代码结构（Review 核对） | 记账移除、窗口派生 |

**Gate 2 Readiness**

自评 PASS：RTM 无 Missing/Simplified（R1/R3 既映射 T02，本 Cycle 为其 C2 消费面）；设计闭合（五窗口 / 侧别判定源 / matcher 冻结边界 / RIGHT 交换变体 / 收窄列源边界已定位 file:line）；契约含完整执行边界与停止条件；Persisted Evidence `none`。用户已批准计划（2026-10-20），`ready`。

**Persisted Evidence**

- Mode: none

Act Response 承载定向与全量命令及结果；无不可低成本重跑项。

**Risks and Notes**

- R1（条件性）：无别名 CTE/视图右臂的 `right_qualifier`（书写名大小写）与作用域段键（effective 小写）的大小写差异——各 matcher 均大小写不敏感，等价性由定向套件零回归见证；若暴露不一致组合按 Stop when 报告。
- R2（非实质）：每窗口一个 usize 入口快照 + 置位点一次性物化——planner 规模无感知。
- R3（预算）：全量分批一次为 Iteration Gate 预算（M19 先例）。
- R4（设计边界提醒）：窗口 ②③④ 布局列源是实际输出列（收窄子集），Guidance ④ 已锁定不得以全臂段替换——Act 若发现两者在某个未过滤形态外仍不一致，返回 Plan。

**Act 实施后的风险消解**

- R1 消解（事实核对，非推断）：`extract_join_table_name` 经 `object_name_parts` 全段小写，CTE / 视图引用的 effective 键与其无别名书写名逐字节相同（`resolve_cte_reference` / `resolve_view_reference` 的 `normalized = name.to_lowercase()`，入参即已小写）；有别名时两侧同为别名小写。作用域段键与既有声明限定符在全部可达形态下逐字节一致——大小写差异面不存在。
- R4 消解：窗口 ②③④ 的列源未被替换——段键改自窗口，列清单仍取自基座计划实际输出列（`join_combined_columns` + `join_row_layout` 照旧）。

---

## Act Response

- Status: reported

### 实际改动

段键与声明序的唯一事实源改自绑定作用域表窗口；`current_qualifiers` 平行记账整体移除。

- `src/parser/planner/binding.rs`：新增 `PlanBuilder::binding_scope_window(entry) -> &[ScopeEntry]`（作用域窗口切片：自入口起的段，声明序；入口前的段——外层作用域段、CTE 定义内部残留段、投影臂临时登记段——不在窗口内）与自由函数 `scope_window_layout(window) -> Vec<(String, Vec<String>)>`（窗口段物化为组合行布局同形的 `(限定符, 列清单)` 列表）。`ScopeEntry::columns` 的 `dead_code` 定点压制移除（Iteration 002 起被消费）；`types`/`not_null` 仍只登记不消费，压制保留。
- `src/parser/planner/query.rs`：`build_from_clause_with_projection` 入口取 `layout_scope_entry = self.binding_scopes.len()`；基座臂元组 `(plan, base_table, base_qualifier)` 收为 `(plan, base_table)`（声明限定符不再随元组外传）；JOIN 右臂元组 `(right_table, right_columns, right_plan)` 收为 `(right_table, right_plan)`（段列清单不再自 `self.tables` 平行取列）；`current_qualifiers` 整体移除（初始化、每步 push 两处记账）；逐列限定符清单 `all_qualifiers` 改自窗口派生（左链各臂段按段列数展开 + 末段本步右臂段按段列数展开）；NLJ ON 布局段改自窗口派生（非 RIGHT = 左链段 + 右臂段；RIGHT = 右臂段 + 左链段，派生后重排）；侧别判定调用点改传窗口。
- `src/parser/planner/ddl_dml.rs`：`extract_join_conditions` 入参 `left_qualifiers: &[String]` + `right_qualifier: &str` 收为 `scope_window: &[(String, Vec<String>)]`，函数内自窗口派生左链段键与右臂段键（末段 = 右臂，其余 = 左链，声明序）；`expr_join_side` 签名与匹配语义不变（仍经 `qualifier_matches_declared`：原文相等或别名归一命中；左链优先次序不变）。
- 执行器 / 存储 / 网络零改动；`PhysicalPlan` 词表零改动；`expression.rs` 两消费点 matcher 与 `binding_scope_hit` 零改动；`current_tables` 按契约保留（非键记账，仍服务输出列构造与 `build_where` 基表用途）；差分白名单 6 条目零增删；错误消息文本零校准。

### Deviations from Plan

1. **窗口拆分点位置（Plan 未指定）**：Plan Context 只写「改查窗口（左 = 窗口除末段各键，右 = 末段键）」，未指定拆分发生点。Act 在 `extract_join_conditions` 内按叶条件拆分（AND 递归每层一次），未在调用点预拆为两个入参——理由：拆分约定（末段 = 右臂）单点维护，避免两处各自实现同一约定。拆分结果与匹配次序（左链先判、末段后判）与既有语义逐字一致。
2. **`cargo fmt` 校准**：本 Act 新增代码有一处 rustfmt 偏差（单行超宽），已 `cargo fmt` 收敛（全仓 `cargo fmt --check` 零 diff；格式化后重跑定向套件全绿，行为零差异）。

### Gate 3 自检

- **Spec compliance**：R1/S1 五个窗口逐一核对——① NLJ ON 布局段自窗口派生（RIGHT 变体派生后重排，同既有语义）；②③④ 段键经逐列限定符清单自窗口派生，列源仍为基座计划实际输出列（收窄子集，未以全臂段替换——R4 边界守住）；⑤ 无 FROM 路径维持空布局。侧别判定改查窗口，匹配语义与「左链优先」次序保留。R3/S1：`explain_test` 10/0，计划形状零校准，执行器 / 词表 diff 零改动。R2 错误面：全量 2046/0/2，错误断言零校准。
- **Code quality**：改动面 = `binding.rs` / `query.rs` / `ddl_dml.rs` 三个文件，执行器 / 存储 / 网络零改动，无计划外修改；`cargo build --lib` 无新增警告，`cargo fmt --check` 全仓零 diff；无身份型证据机制或判定层新增。窗口拆分的 `expect` 仅落在结构不可达路径（基座臂与本步右臂各登记一段，窗口至少两段），与既有 `expect("validated table must exist in metadata")` 同型。
- **单源结构核对**：`current_qualifiers` 已从代码中消失（仅存于「平行记账移除」的说明注释）；`join_column_layout` 全部写入点的段键溯源到作用域表窗口——① NLJ ON 直取窗口段；②③④ 经 `join_row_layout` + `combined_qualifiers` 间接溯源；⑤ 空布局（无 FROM，无窗口）。布局 / 侧别面的「自 `self.tables` 按表键取列」平行路径已移除。
- 已修复：Deviation 2（rustfmt 校准）。
- 遗留 Minor：无。
- Experience Candidates：None。

### 验证（全部真实运行）

- **先行测试（重构类，先观察 GREEN）**：改前 `cargo test --test <16 个定向目标> -- --test-threads=1` → 16 目标全绿、223 passed / 0 failed；改后同一命令 → 同一计数全绿（重构保持 GREEN，无 RED 形态）。
- **GREEN 定向（改后，逐目标计数）**：`attach_session_test` 11 / `chain_join_test` 9 / `cross_db_query_test` 21 / `cte_test` 14 / `derived_scan_alias_test` 23 / `explain_test` 10 / `join_output_resolution_test` 19 / `join_test` 7 / `nested_loop_join_test` 9 / `order_by_alias_sort_test` 7 / `outer_join_test` 16 / `self_join_test` 6 / `sql_differential_test` 10 / `subquery_alias_binding_test` 23 / `table_alias_resolution_test` 19 / `view_test` 19——全部 0 failed。
- **差分网**：`sql_differential_test` 10/0，白名单 6 条目与跳过行为冻结不变、无新分歧。
- **EXPLAIN 计划形状**：`explain_test` 10/0，零校准。
- **全量（M19 分批一次，最终代码）**：14 批 = `--lib` + 125 个集成目标按字典序每批 ≤10 目标，每批独立 `cargo test … -- --test-threads=1`、批间顺序执行、无无过滤全量调用、无默认并行；全部批次 rc=0，**合计 2046 passed / 0 failed / 2 ignored**（与 Iteration 001 replan 基线 2046/0/2 逐批计数一致）。目标覆盖经 ran-vs-want 清单 diff 核对：ran 125 = want 125、唯一目标 125——不重不漏。分批计数：

  | 批 | 目标（组） | passed/failed |
  |---|---|---|
  | 1 | lib | 407/0 |
  | 2 | aggregate_extension_test…busy_timeout_test（10） | 148/0 |
  | 3 | cache_perf_test…concurrent_test（10） | 182/0 |
  | 4 | connection_limit_test…ddl_crash_recovery_test（10） | 147/0 |
  | 5 | default_declaration_test…explicit_tx_test（10） | 158/0 |
  | 6 | expression_e2e_test…insert_select_test（10） | 100/0 |
  | 7 | isolation_level_test…mvcc_commit_test（10） | 88/0 |
  | 8 | mvcc_record_test…parser_test（10） | 95/0 |
  | 9 | pg_integration_test…projection_expression_test（10） | 203/0 |
  | 10 | projection_test…runtime_test（10） | 64/0 |
  | 11 | scalar_function_test…sql_differential_test（10） | 139/0 |
  | 12 | storage_test…tx_statement_test（10） | 153/0 |
  | 13 | update_expression_test…wal_handle_test（10） | 122/0 |
  | 14 | wal_integration_test…write_type_conformance_test（5） | 40/0 |

- **构建 / 格式化**：`cargo build --lib` 无警告；`cargo fmt --check` 全仓零 diff。

### Persisted Evidence

None required（计划 Mode: none；全部验证可低成本重跑，无不可重跑项——全量分批可由同一命令序列复现）。

### Experience Candidates

None（本 Cycle 无范围外的实质缺陷发现；白名单 5 条 suspected 缺陷的处置仍待 Iteration 003 规划，非本 Cycle 产出）。

### 未解决问题

无。

---

## Plan Review

- Review Result: accepted
- Reviewer: openspec-plan（2026-10-21 本会话）

**Findings**

1. **[核定——通过] 五窗口与侧别判定窗口派生（R1/S1 布局消费段）**：`layout_scope_entry` 于 FROM 构造入口取得（`query.rs:384-387`）；窗口在每步 JOIN 迭代于 ON 编译前一次性物化（`query.rs:651` `scope_window_layout(self.binding_scope_window(...))`——Guidance ① 窗口快照纪律满足，ON 编译期子查询臂的 scope 截断不影响已物化副本）；窗口 ① NLJ ON：非 RIGHT = 左链段 + 右臂段、RIGHT 派生后重排（`query.rs:817-826`，同既有 :815-824 语义）；窗口 ②③④ 段键经 `all_qualifiers`（`query.rs:652-664`，自窗口段按段列数展开）→ kept 过滤 → `combined_qualifiers`（`query.rs:886` 原样保留）间接溯源，列源仍为基座计划实际输出列（R4 边界守住——`join_combined_columns`/`join_row_layout` 不在 diff）；窗口 ⑤ 无 FROM 空布局不在 diff（维持）。侧别判定：`extract_join_conditions` 收窗口、`split_last` 拆末段为右臂（`ddl_dml.rs:217-227`），`expr_join_side` 匹配仍经 `qualifier_matches_declared` 且左链优先次序保留（`ddl_dml.rs:280/:284`，函数体不在 diff）。
2. **[核定——通过] 单源结构（Acceptance「current_qualifiers 收敛」项）**：`grep -rn current_qualifiers src/ tests/` 仅 `query.rs:648` 注释一处（「平行记账移除」说明）；初始化与 push 记账、`extract_join_conditions` 的 `left_qualifiers`/`right_qualifier` 入参、NLJ 布局的 `table_columns` 闭包（自 `self.tables` 按表键取列的平行路径）全部移除；基座臂/右臂元组收窄（`(plan, base_table)` / `(right_table, right_plan)`）与 Act 声明一致。
3. **[核定——通过；偏差分类 Act 局部选择] Deviation 1（窗口拆分点在 `extract_join_conditions` 内）**：拆分约定（末段 = 右臂）单点维护，`split_last` 的 `expect` 落在结构不可达路径（基座臂与本步右臂各登记一段，窗口 ≥2 段；投影臂临时段位于构造入口之前不在窗口内——窗口边界设计消解了 001-replan 遗留移交的「空列清单段 × 布局派生」风险点）；拆分结果与「左链先判、末段后判」次序与既有语义逐字一致。
4. **[核定——通过] Preserve/Forbidden 面**：执行器/存储/网络零改动（git status：tracked 改动 = planner 五文件 + `pipeline.rs` + `tests/subquery_alias_binding_test.rs`，其中 `expression.rs`（分类块 :261-282）/`mod.rs`（+23 全为 001 作用域基建）/`subquery.rs`/`pipeline.rs`（not_null 通道 +3）均为 001 replan 已审范围，002 实际改动面 = `binding.rs`/`query.rs`/`ddl_dml.rs` 与 Act 声明一致）；`expr_join_side` 与 `binding_scope_hit` matcher 零改动；`current_tables` 按契约保留；`ScopeEntry.columns` dead_code 压制移除、`types`/`not_null` 压制保留（`binding.rs:25-28`，消费状态与 Act 声明一致）；白名单 6 条目（1 known + 5 defect）逐条核对未变。
5. **[核定——通过；Deviation 2] rustfmt 收敛**：格式化后定向重跑全绿，行为零差异。
6. **[Minor——新证据，非阻塞] 段宽等价边界记录**：`all_qualifiers`/NLJ 布局的段列宽自「live `self.tables` 查找」改为「登记时作用域快照」。两源在全部既有输入逐字节一致（每臂登记列即取自 `self.tables` 同一值）；理论分歧仅存在于「内层子查询/CTE 以不同列宽重注册外层 FROM 同名键」的病理形状——该形状下旧 live 查找会与计划节点实际列宽错位（旧缺陷形态），新快照与计划节点一致（registration 与 Scan/DerivedScan 节点构造同源同刻）。全量 2046 + 差分网 + explain 零校准见证无既有输入命中；无 Acceptance 缺口，纯记录。

**Acceptance Gaps**

None——三项 Acceptance 全部满足：R1/S1 布局消费段（Finding 1 代码核对 + 定向 JOIN/别名/CTE/视图/attach 套件 223/0 采信 Act）；R3 计划形状零校准（`explain_test` 10/0 + 词表/执行器 diff 零改动独立见证）；R2 全量零回归 + 错误面（全量 2046/0/2 采信 Act M19 分批实测——Act 报告后覆盖表面未变化，本 Review 全程只读 git 操作核对工作区与 Act 现场一致）。

**Evidence**

- 完整 diff 独立阅读：`binding.rs`（新文件全文）、`query.rs`（176 行 diff 全读）、`ddl_dml.rs`（40 行 diff 全读）、`mod.rs`/`expression.rs`/`pipeline.rs` diff（确认 001 范围）。
- 结构核对命令：`grep -rn current_qualifiers src/ tests/`（仅注释）、`grep -n "binding_scope_window\|scope_window_layout"`（helper 落地与消费点）、白名单条目清单实读（`sql_differential_test.rs:167-205`，恰 6 条）。
- 采信 Act Response 验证结论（未失效）：16 定向目标 223/0、差分网 10/0、`explain_test` 10/0 零校准、全量分批 14 批 2046/0/2、`cargo fmt --check` 零 diff。

**Follow-up Decision**

接受：Acceptance 全部满足、无阻塞 finding；两条 Deviations 均为契约内非实质局部选择；Minor 一项（Finding 6）为记录性。遗留处置（2026-10-21 用户指令「把这些问题和iter3本来要做的规划在一起解决掉」）：001-replan 遗留移交的「投影臂无别名派生外层段登记不完备」并入 Iteration 003（T04）；差分白名单 5 条 suspected 缺陷按既定裁定由 003 T03 承载；「视图/CTE/attach 段登记完备化」经本次 Review 期调查定论——五个 FROM 面臂（基表含 attach 复合扁平名 `query.rs:413-437`、JOIN 右臂、派生、CTE、视图）均已登记，唯一 code-witnessed 缺口即投影臂 Derived 空名（T04 承载），attach 隐式短名命中面维持 D6 兜底且无缺陷证据，不扩面。

**Iteration Plan Update**

None（Iteration 002 按 Map 完成；003 范围扩充属下一 Iteration 展开时的 Map 更新，见 tasks.md）

**Next Cycle**

None

**Next Iteration**

`iterations/003-defect-clearance-and-scope-completion/000-initial.md`（T03 差分白名单缺陷清偿 ①②③④ + T04 投影臂无别名派生段登记收口；Plan Context `draft`——待用户批准后转 `ready`）
