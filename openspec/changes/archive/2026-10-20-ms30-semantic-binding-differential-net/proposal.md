# Proposal: MS30 SQL 语义绑定层一期（差分测试保护网 + JOIN/关联子查询收口）

- Date: 2026-10-20
- Status: approved（2026-10-20 用户批准；范围 = 本 proposal What Changes 全项，默认假设 A1-A7 生效）
- 来源：milestone MS30（`.agents/docs/tasks.md`，2026-10-19 用户批准底层设计批增补，执行序前置于 MS22）+ improvements I080/I091（promoted，2026-10-19 用户裁定全候选优先级排序：I080 第 1 位、I091 配套保护位）+ 用户指令「开始规划ms30到一个change里面，要求符合各种规范」（2026-10-20）

## Why

2026-10-04 至 2026-10-15 的多个 change 反复修复别名、子查询、投影组合面的同类「静默错误结果」缺陷，最近一批为 ISS16–ISS18（2026-10-15 实测评测；前 change 归因确认四执行器 `unwrap_or(0)`/`filter_map` 绑参静默回落与 `extract_subquery_table_names` 内层别名限定符丢弃）。根因是名字解析依赖字符串限定符、列位置与构建上下文：`PlanBuilder` 以 `building_subquery`/`building_from_derived` 上下文标志与 `join_column_layout` 位置偏移驱动解析，执行期仍有 `condition_column_index` 类按名定位点（I080 证据，2026-10-18 静态核对）。组合语义靠多处约定一致维持，历史缺陷全部发生在约定不一致的组合上。

既有回归只能锁定已知缺陷：测试按当期已实现形状编写（2026-10-15 覆盖缺口归因结论），缺少主动发现未知组合的手段。`Cargo.toml` 已有 rusqlite 0.31 开发依赖（已核实），但 tests 中未建立差分套件（I091）。差分网与绑定层是先保护网后重构的关系：I091 明示 I080 绑定重构消费差分测试产出；反序令重构无静默偏差拦截。

## What Changes

1. **T01 差分测试网（Iteration 000，先行独立验收）**：`tests/` 内新增 rusqlite 差分套件与结构性质套件——语义一致 SQL 子集双向对比（RTsql vs SQLite）、别名/派生表/等价谓词变换一致性、B-Tree/版本链/编码往返性质测试（固定种子）；结果归一化覆盖排序稳定性、NULL 表示与数值形态；已知分歧白名单逐条带原因分类。复用既有开发依赖，判定逻辑全部位于项目测试框架内，不建框架外判定层。
2. **T02 绑定阶段基建与收口（后续 Iteration，收口面随 Plan 期调查定稿）**：计划构建前建立名字/类型绑定结果（携带关系身份、列身份、类型与可空信息），一期收口 JOIN 组合行布局解析与关联子查询参数分类两个历史缺陷面；不重写全规划器、不改 PhysicalPlan 节点词表；既有解析语义与用户可见错误面逐字节零回归（全量既有基线 + 差分网双保护）。
3. **T03 差分白名单缺陷清偿（Iteration 003，2026-10-20 用户裁定增补）**：Iteration 000 差分网分诊出的四类 suspected 缺陷在本 change 内清偿——① 无主键基表 JOIN/SemiJoin 空集（executor scan 枚举）、② TEXT 相关参数求值 NULL（executor predicate，M37 遗留）、④ NOT IN NULL 外层键不折叠（executor AntiJoin 三值）、③ 聚合派生表外层列引用拒绝（planner `get_plan_output_columns`）；修复后移除对应白名单条目。⑤ 谓词×IN 子查询 AND 组合维持拒绝（Non-goal 不变）。详细范围、delta spec 与验证契约随 Iteration 003 规划定稿（Iteration 002 `accepted` 后，内容可视其结果调整）。

单 change 承载 MS30 两任务（用户指令 2026-10-20，覆盖 tasks.md workload「2-3 个 change」注记）；「差分网先行独立验收」以 Iteration 边界保留——Iteration 000 Review `accepted` 后才展开绑定 Iteration。

## Non-goals

（来自 tasks.md MS30 与 I080/I091 条目）

- 全规划器重写（I080 明示分批）；优化器代价选择（I016）；字符串/组合索引路由（I085）；类型化库结果与参数绑定 API（I088/I089 → MS31）。
- fuzzing 依赖（I091：确需扩大探索时再评估）；rusqlite 版本升级（I093 域；MS30 split signal 点名单独评估）。
- PhysicalPlan 节点词表变更（若 Phase 2 调查发现收口必须动词表 → 按 split signal 拆独立设计 change，本 change replan，不在本 change 内强行承载）。
- 既有拒绝面开放（JOIN 形 IN 子查询、多级相关子查询等维持拒绝）。
- MS31/MS32 范围（类型化结果、批量接口、内存预算）。

## 默认假设（集中决策项——已取默认值闭合，非 TBD；用户审计可调整）

- **A1 单 change 结构**：MS30 两任务一个 change、多 Iteration（000 差分网独立验收 → 绑定 Iterations）；Iteration 边界保留 tasks.md「差分网先行独立验收」意图。
- **A2 差分对比形态**：进程内 lib API 对比（RTsql `Database` vs rusqlite in-memory），非 CLI 子进程——被测对象是 SQL 语义而非 CLI 渲染层（渲染面已有 e2e 套件覆盖）；查询集固定、规模有界，控制全量回归时间增长。
- **A3 子集与白名单策略**：子集只含双方语义一致的语句；已知分歧（严格类型门 vs SQLite type affinity、错误面差异等）逐条文档化白名单，每条带分类（已知语义差异 / 疑似缺陷）与规避方式；白名单外分歧使测试失败，不接受静默跳过。
- **A4 疑似缺陷处置**：差分网或收口调查发现的疑似缺陷——一期收口面内为本 change 修复候选；收口面外按公共规则作为 Issue 候选报告不落账。（2026-10-20 增补：Iteration 000 分诊的 ①②③④ 经用户裁定改为本 change T03/Iteration 003 承载，见 What Changes 3；⑤ 维持拒绝。）
- **A5 绑定层形态**：增量式绑定阶段（计划构建前的绑定结果结构），一期消费面 = JOIN 组合行布局解析 + 关联子查询参数分类 + Phase 2 调查定稿的相邻收口点；不改 PhysicalPlan 词表与执行器接口。
- **A6 性质测试确定性**：固定种子可重现，不做时间敏感断言；全部位于 `cargo test` 可运行集合内。
- **A7 零回归硬边界**：T02 全部 Iteration 以既有 2030 测试基线 + 差分网零回归为验收前提；用户可见错误消息（类型、文本、SQLSTATE）逐字节不变。
