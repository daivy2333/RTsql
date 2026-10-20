# tasks — 任务与里程碑路线

> 最后更新：2026-10-19（**底层设计批路线增补**：milestone-planner 经用户批准新增三个 planned 里程碑——MS30 SQL 语义绑定层一期（T1 I091 差分测试保护网 + T2 I080 JOIN/子查询收口）/ MS31 类型化库结果与参数绑定 API（T1 I088 + T2 I089，依赖 MS30）/ MS32 执行器批量接口与查询资源契约（T1 I081 量化先行 + T2 I082 JOIN 流水/Top-N/内存预算），建议执行序 MS30 → MS31 → MS32 → MS22——性能批殿后不变、候选池不动；六个 I 条目随批转 promoted；依据 `.agents/memory/improvements.md` 2026-10-19 用户裁定优先级排序）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

33 个 Milestone：19 completed + 6 superseded（历史段已归档至 carrier `ARC-202609242151`，本文件仅保留活跃路线）+ 8 planned（初版后统一路线，2026-09-24 规划：MS18-MS27；2026-10-01 用户批准重排，milestone-planner 增补 MS28-MS29 承接 I077/I078/I079 与残留任务；2026-10-19 用户批准底层设计批（依据 improvements 同日优先级排序），milestone-planner 增补 MS30-MS32 承接 I091/I080/I088/I089/I081/I082、执行序前置于 MS22；MS23 于 2026-09-25 完成，MS24 于 2026-09-26 完成〔T04 声明期 DEFAULT 类型校验未实施，2026-10-01 重排并入 MS29-T02〕，MS19 于 2026-09-26 完成，MS18 于 2026-09-26 完成，MS25 于 2026-09-28 完成，MS20 一期于 2026-09-29 完成〔T02 二期跨文件 DML 待后续 change〕，MS21/MS26 于 2026-09-30 完成，MS28 于 2026-10-01 完成，MS29 于 2026-10-01 完成，MS20 于 2026-10-02 完成（T02 二期随 change `2026-10-02-ms20-cross-db-dml` 三 Iteration accepted 归档），MS27 于 2026-10-03 完成（change `2026-10-02-ms27-overflow-integrity` 三 Iteration accepted 归档））。★初版达成★。

统一执行序（2026-10-01 用户裁定重排——功能补齐前置、性能优化殿后独立成批；沿用「批准执行序与编号无关」既定先例，编号只表规划先后，执行以本序为准）：**MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）→ MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）→ MS20-T02 跨文件 DML 二期 → MS27 存储演进批（超页溢出 + 完整性校验）→ MS22 实测驱动性能批（殿后独立，量化定稿后转 ready）**。顺序为建议序非硬依赖（各项无环、前置均已满足）。编排依据（2026-10-01）：外部评审实证的自动化使用摩擦先行收口——并发硬失败 exit 4 → 可配置等待是多 agent 并发使用的语义前提，也是 MS22 持锁窗口治理的前置；输入通道与声明语义同代码面相邻（lifecycle.rs import + ddl_dml.rs 建表/类型面）；存储格式演进体量最大放功能带末尾；性能殿后、先量化再决定（MS08 纪律）。首批排除域维持：微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058。REPL（I067）、DETACH（I076）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）、WAL fsync 组提交（I049——串行单写者模型下无效，MS22 量化时剔除）等留 improvements 域未排期。
旧优化项历史分类（各 superseded MS 原范围段）随 `ARC-202609242151` carrier 留档；未排期候选以 D-candidates 与 improvements 台账为准。
## Milestone Roadmap

### MS22：实测驱动性能优化（第一批） — planned（范围量化定稿后转 ready）

- **Status**: planned（候选范围经 profile/bench 定稿后转 ready——沿用 MS08「先量化再决定」纪律；2026-10-01 重排定位殿后独立成批，功能补齐带完成后启动）
- **Dependencies**: None 硬前置（建议序殿后——MS28 busy-timeout 等待语义先行，持锁窗口治理在其上才有意义；I062 已随 MS18 收口）
- **Outcome**: 经量化证实的性能积压按收益排序清收（第一批）。候选池：I049 WAL fsync 合并（组提交）/ I021 INSERT 多值批量 / I026 脏页 writev 批量写回 / I050 RowLockTable DashMap 化 / I024 Varint Key 变长编码 / I038 GC 无键行盲区 / I020 clone 消除 / I066 分配器评估（jemalloc/mimalloc）/ import 逐行 auto-commit 吞吐（I077-② 持锁窗口治理侧——攒批提交压缩最长持锁尾，与 MS28 busy-timeout 等待语义配套；2026-09-30 实测 ≈4.1 ms/行、2000 行 8.246s、30000 行 ≈123s，逐行提交含 `sync_all`；量化来源 change `2026-09-30-catalog-durability-and-cli-surface` Iteration 000 Act 实测）——每项先 bench/profile，达标立项，不达标退还 improvements 并留量化记录。**避坑注记（2026-10-01 重排）**：I049 组提交在串行单写者模型（one-shot + MS28 busy-timeout）下无效——单写者无并发提交可攒，量化定稿时剔除，除非 I071 serve 扩容路径启动
- **Rationale**: MS08 实测驱动批处理先例的复活（2026-09-14 剥离时各项「先量化再决定」纪律随条目保留）；各项独立故障域但共享同一验收主题「实测证实的性能收益」，每项独立 change 独立验收，单项失败不阻塞其余
- **Scope**: 候选池见 Outcome；定稿范围以量化数据为准（规模：小-中 change × N）
- **Non-goals**: 撕裂树运行期根修（I031 中-大，量化后另行评估）、代价模型（I016）、扫描流式化（I065）、B+Tree 节点级锁（I025）、io_uring（I028）、瘦内部节点（I029）、合并 Tag byte（I030）——全部留 improvements 域
- **Workload**: 视定稿范围 N 个独立小-中 change，每 change 前后 bench
- **Stable baseline**: 定稿项各有 before/after bench 对比数据且全量零回归；未达标项有量化记录可退还
- **Verification boundary**: 每项 bench 对比（`--save-baseline` 纪律，R26/ms08-bench runbook）+ 全量测试零回归
- **Diagnostic boundary**: 各候选项在 improvements 条目自带诊断面
- **Split signals**: 定稿项 ≥4 或出现跨域大项（如 I031 入选）时拆第二批
- **Related changes**: None

### MS30：SQL 语义绑定层一期（差分测试保护网 + JOIN/子查询收口） — planned

- **Status**: planned（2026-10-19 用户批准底层设计批增补；来源 I091/I080——improvements 2026-10-19 优先级排序第 1 位与配套保护位）
- **Dependencies**: None
- **Outcome**: 规划器获得统一的名字/类型绑定阶段（AST → 名字/类型绑定 → 逻辑计划 → 物理计划边界），一期收口 JOIN 与关联子查询面——绑定结果携带关系身份、列身份、类型与可空信息；别名/子查询/投影组合语义的静默错误结果类缺陷经 rusqlite 差分测试网拦截
- **Rationale**: 2026-10-04 至 2026-10-15 多个 change 反复修复别名、子查询、投影组合的同类缺陷（change `2026-10-15-subquery-alias-binding-and-error-prefix` T1/T2 限定符化与关联参数绑定加固、ISS16–ISS18），根因是名字解析依赖字符串限定符、列位置与构建上下文（I080 证据段）；差分网是绑定重构的直接保护网（I091 明示 I080 重构消费其产出），同段建立符合「高风险基础验证可单独承载」先例
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS30-T01 | I091：rusqlite 差分与性质测试网——语义一致 SQL 子集双向对比、别名/派生表/等价谓词变换、B-Tree/版本链/编码往返性质测试；复用既有开发依赖，不建测试框架外判定层 | improvements I091（2026-10-18 用户接受方向；优先级排序配套位） |
| MS30-T02 | I080 一期：绑定阶段基建 + JOIN/关联子查询面收口（不重写全规划器，具体收口面随 Plan 期调查定稿） | improvements I080（2026-10-18 用户接受方向；优先级排序第 1 位） |

- **Non-goals**: 全规划器重写（I080 明示分批）；优化器代价选择（I016 未纳入本批）；字符串/组合索引路由（I085 后续批次）；类型化 API 面（→MS31）
- **Workload**: 中-大，2-3 个 change（差分网先行独立验收，绑定基建与收口面按 Plan 拆分）
- **Stable baseline**: 绑定结果（关系身份/列身份/类型/可空）成为计划期可依赖输入；差分测试网进入回归基线——后续规划器改动（I016 代价选择、I085 索引路由等）有静默偏差拦截
- **Verification boundary**: 差分套件对语义一致子集的通过与已知差异白名单 + 绑定收口面既有子查询/别名矩阵零回归 + 全量分批回归（M19）
- **Diagnostic boundary**: `src/parser/planner/`（构建上下文与 query/subquery 消费面）+ 按名定位的执行期残留点（如 `src/executor/join.rs::condition_column_index`）+ 新增差分测试模块
- **Split signals**: 绑定基建若需改变 PhysicalPlan 词表结构（触及 I074 第三刀单点）时拆出独立设计 change；差分网若牵动 rusqlite 版本升级（I093 域）时单独评估
- **Related changes**: None

### MS31：类型化库结果与参数绑定 API — planned

- **Status**: planned（2026-10-19 用户批准底层设计批增补；来源 I088/I089——优先级排序第 2–3 位）
- **Dependencies**: MS30（I089 消费 I080 绑定结果；I088 的列元数据与绑定信息衔接）
- **Outcome**: 嵌入式库 API 面定型——公共结果携带原生 Value 与列名/类型元数据（CLI/JSON/PG 在边界转换），类型化参数 execute/query 接口与可复用预编译语句；调用者不再自行渲染值、可直接消费内部类型
- **Rationale**: 核心执行结果当前依赖 `network::protocol::Response`（JSON 值数组、无完整列元数据），核心↔协议耦合（I088 证据：`src/database.rs::execute_sql`、`src/pipeline.rs::execute_executor` 与 `value_to_json`）；程序调用需自行渲染 SQL 文本、重复 SQL 的计划生命周期缺公共契约（I089）。两任务严格串行、同 API 故障域，聚合一段
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS31-T01 | I088：类型化公共结果 + 核心/协议解耦（既有 execute_sql 调用者兼容迁移；CLI/JSON/PG 边界转换，输出行为面零变化） | improvements I088（2026-10-18 用户接受方向；优先级排序第 2 位） |
| MS31-T02 | I089：类型化参数 execute/query + 可复用预编译语句（定型、NULL、参数个数/类型错误、schema 变化后计划失效契约；不以字符串替换实现） | improvements I089（2026-10-18 用户接受方向；优先级排序第 3 位） |

- **Non-goals**: PG Extended Query 协议（I071 触发锁定维持）；流式游标/背压（I065 long-term）；crate 边界拆分（I074 长期方向，不以本段为前置）
- **Workload**: 中，约 2 个 change（T01 一 change；T02 视 Plan 调查可 1-2）
- **Stable baseline**: lib 公共契约为类型化结果 + 参数绑定；`network::protocol::Response` 退化为边界转换词表——CLI/Server 消费者与嵌入式使用者各取所需
- **Verification boundary**: 类型化结果与既有 JSON 输出逐字节等价（边界转换处）+ 参数绑定错误矩阵 + CLI/PG 既有面零回归 + 全量分批回归（M19）
- **Diagnostic boundary**: `src/database.rs::execute_sql`/`execute_in_tx` + `src/network/protocol.rs::Response` + `src/pipeline.rs` 转换点与 plan cache 失效面
- **Split signals**: 兼容迁移若迫使破坏性 API 变更（调用者面无法平滑）时停下裁定迁移策略；预编译语句若牵动 plan cache key 语义重构时拆独立设计
- **Related changes**: None

### MS32：执行器批量接口与查询资源契约 — planned

- **Status**: planned（2026-10-19 用户批准底层设计批增补；来源 I081/I082——优先级排序第 7–8 位；两项均量化先行，不达标部分沿 MS22 纪律退还 I 台账留量化记录）
- **Dependencies**: None 硬前置（建议序在 MS30 后——I082 触碰 JOIN 执行面，避免与绑定收口同面并发改动）
- **Outcome**: 查询执行资源模型成立——`next_batch` 批量接口与查询内存预算/超限错误成为执行器公共契约；Hash JOIN 构建侧缓存 + 左侧全量物化 + 匹配集合克隆的内存放大收敛（左侧逐行探测），ORDER BY + LIMIT 走 Top-N
- **Rationale**: `Executor::next()` 逐行 + async-trait 装箱的分配/调度耗时占比未测量（I081 先测量再评估，不预设向量化框架）；Sort 全量收集、Hash JOIN 双侧全量缓存与匹配集合 `.cloned()`（I082 证据：`src/executor/sort.rs::initialize`、`src/executor/join.rs::next` 三阶段）；两项共同定义「执行与资源」一个阶段成果，并与 I020（表示层）/I066（分配器）分层归因、不同时改多层
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS32-T01 | I081：执行器批量异步接口（量化装箱/调度开销 → `next_batch`，热点算子逐步转换、复用现有 Value 与行容器，保留 `dyn Executor` 装配） | improvements I081（2026-10-18 用户接受方向，先测量） |
| MS32-T02 | I082：JOIN 流水处理（左侧逐行探测/减少集合复制）+ Top-N + 查询内存预算与超限错误（外部排序/溢写确有需求才另行设计；INSERT SELECT 同表源物化快照语义保持） | improvements I082（2026-10-18 用户接受方向） |

- **Non-goals**: Arrow/完整向量化框架（I081 明示不预设）；外部排序与分区 JOIN 溢写（I082 确有需求才设计）；Value 表示层改动（I020 维持 MS22 候选池）；分配器更换（I066 维持 MS22 候选池）
- **Workload**: 中，约 2 个 change（每任务各一；量化结果允许裁剪范围）
- **Stable baseline**: 执行器公共契约含批量接口与内存上限——大表/大值/多匹配键组合的首行等待与内存峰值有明确预算边界
- **Verification boundary**: 量化基线（装箱开销/克隆规模 before/after bench，`--save-baseline` 纪律）+ 内存预算超限拒绝矩阵 + 既有 JOIN/Sort/大值用例零回归 + 全量分批回归（M19）
- **Diagnostic boundary**: `src/executor/executor_trait.rs` + `src/executor/sort.rs::initialize` + `src/executor/join.rs::next` 三阶段 + 热点算子转换清单
- **Split signals**: `next_batch` 若需改变 `dyn Executor` 装配协议（牵动全执行器）时拆独立设计 change；内存预算若牵动跨算子资源管理器时单独收口
- **Related changes**: None

## D-candidates（不归入当前 MS，待后续决定）

| 标题 | 不建议做的原因 | 何时重评 |
|---|---|---|
| 代价模型 + Join 重排 | 价值/复杂度不匹配 | 视后续 join 性能实测（MS09-T02 已完成） |
| B+Tree 节点级锁 | ~500 行业务代码；460 tests 未证明有争用 | 视后续性能实测（MS08 已剥离至 improvements 域） |
| clone 消除 Arc/Cow | 零拷贝 ValueRef 教训（K18）：先量化再优化 | 视后续 clone 频率 profiling |
| io_uring | 高风险低收益 | 视后续整体性能实测 |
| 瘦内部节点 | 依赖 Varint Key | I024 实施后单独评估 |
| 合并 Tag byte | 改动序列化格式 | 暂搁 |

## 长期方向（未规划具体里程碑）

- **io_uring 集成**（改进项 I028）：Linux 5.1+ tokio-uring 批量提交
- **jemalloc/mimalloc 优化 (K37)**：已转入 improvements 台账 **I066**（2026-09-24 用户指令，权威位置移至 `.agents/memory/improvements.md`）

## 依赖关系图

初版后统一路线（2026-09-24 规划、2026-10-01 用户批准重排：功能补齐前置、性能殿后独立，均无硬前置，编号与执行序无关；2026-10-19 用户批准底层设计批 MS30–MS32 前置、性能批殿后不变）：

```
MS23 约束面 ──(唯一索引/约束语义)──→ MS24 写面
MS23 ──(索引基建)──→ MS21 DDL/二级索引
MS18 one-shot 性能 ──(checkpoint 策略面)──→ MS26 运行形态 ──(存储后端)──→ MS27 存储演进
MS25 SQL 读面 · MS20 ATTACH 跨库（独立，序自由）
MS28 并发等待语义 ──(持锁窗口治理前提)──→ MS22 import 攒批（性能批候选）
MS30 语义绑定层 ──(绑定结果/参数定型输入)──→ MS31 类型化 API
MS32 执行资源契约（无硬前置，建议序 MS30 后避开 JOIN 同面并发改动）
全部功能带（MS28/MS29/MS20-T02/MS27）──→ 底层设计批（MS30–MS32，2026-10-19 批准）──→ MS22 实测驱动性能批（殿后独立，量化定稿后转 ready）
```

建议执行序：**MS23 ✓ → MS24 ✓ → MS19 ✓ → MS18 ✓ → MS25 ✓ → MS20 一期 ✓ → MS21 ✓ → MS26 ✓ → MS28 ✓ → MS29 ✓ → MS20-T02 ✓ → MS27 ✓ → MS30 → MS31 → MS32 → MS22**（截至 2026-10-03 前十二项已完成；2026-10-19 批准底层设计批前置，下一项 MS30 SQL 语义绑定层一期，MS22 性能批仍殿后——候选范围量化定稿后转 ready）

无环；所有依赖均已满足。初版后工作（MS18-MS32）按上方统一执行序推进，编号只表规划先后；已完成历史全部随 `ARC-202609242151` 归档。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

- **AntiJoin 关联参数绑定期加固**（2026-10-15）：change `2026-10-15-antijoin-correlated-param-hardening` 单 Iteration 单任务全 accepted 收尾归档。来源：change `2026-10-15-subquery-alias-binding-and-error-prefix` Plan Review Finding 1（T2 三执行器加固漏掉的同构点）+ 用户指令「我们继续解决这个」（不落账独立 Issue，沿 2026-10-05 先例）。T1 `AntiJoinExecutor::extract_param_values`（`src/executor/anti_join.rs:151-175`）`filter_map` 静默丢弃改 `Result`：`map` + `.copied().ok_or_else(...)` 未命中外层列报错 `correlated parameter column not found in outer row: {列名}`（`StorageError::ExecutionError`，58000 兜底），唯一调用点 `?` 上浆；命中臂 `left_row[idx].clone()` 直接索引逐字节保持（A3 不统一三处 `.get(idx)` 回退差异）；先行单测 `missing_outer_column_reports_error_naming_column`（`right_plan`/`database` 给齐防落空——`:206` 守卫否则跳过被测路径；NOT EXISTS 形态聚焦绑定期）RED `80 passed; 1 failed`（`Ok(None)` 静默空集与预测逐字一致）→ GREEN 81；`extract_param_values` 全库四处全部 `Result` 化收口。零回归：矩阵 22 + subquery_test 43 定向全绿；Iteration Gate `2030 tests / 0 failed / 2 ignored`（2029 基线 + 本单测 1，对账吻合）。Plan Review accepted（独立 diff 阅读 + 本会话重跑复核 lib 81/0 与两套件全绿 + fmt 干净；信息项一条：实施由并行执行体完成、本会话清理其重叠写入产生的重复测试块后审计）。1 域 delta 合并（`correlated-subquery-cache` MODIFIED「关联参数外层绑定失败显式报错」——枚举扩至四含 AntiJoin、修复前形态补 `filter_map` 丢弃与缓存键短项化后果），域数 64 不变、validate --specs 64/64。ISS04 维持活动目录不动。

- **子查询别名限定符绑定修复（ISS16–ISS18）与执行器错误前缀单层化（ISS19）**（2026-10-15）：change `2026-10-15-subquery-alias-binding-and-error-prefix` 单 Iteration 四任务全 accepted 收尾归档。来源：2026-10-15 用户实测评测四缺陷（ISS16 相关标量外参未绑定、ISS17 EXISTS 内层别名四静默形态、ISS18 IN×别名×索引恒空集、ISS19 执行器双层前缀）+ 用户指令（「可以，交由你来跑plan」、覆盖缺口归因并入、Gate 5 豁免「按你的建议进行改动」、「开始审计」、「进行收尾吧提交同步」）。T1 `extract_subquery_table_names` 声明限定符化（新私有辅助 `declared_qualifiers`：别名优先 + 基表名并列保留〔Gate 5 豁免方案①——别名内层保持「别名+基名限定」宽松语义，A1 由证伪转锚点 `a1_base_name_qualifier_inside_aliased_inner`〕、JOIN 臂同规则、Derived 取别名）——`qualified_column_expression`/`extract_correlated_params` 五消费方自动一致修复，零调用点改动；先行矩阵 `tests/subquery_alias_binding_test.rs` 22 用例（算子 6 面 × 内层形态 3 面 × 谓词列位置 × 相关性 3 面 × IN 索引面 + EXPLAIN `correlated_params` 虚高回落双向断言 + JOIN 形 IN 拒绝面 ×2〔含「别名不得绕开拒绝面」〕+ A1 锚点 + 无别名零回归锚点 ×3），RED 10 passed/12 failed → 豁免后 22/22。T2 三执行器 `extract_param_values`（SemiJoin/ScalarSubqueryFilter/SubqueryEval）`unwrap_or(0)` 静默回落改 `Result` 点名报错 `correlated parameter column not found in outer row: {列名}`（58000 兜底不新增分类），命中路径与缓存键语义逐字节不变；lib 单测 ×3 经 `next()` 触发（新 `#[cfg(test)] StubRowExecutor`）。T3 `StorageError::ExecutionError` thiserror 前缀去除、单层 `Execution error: ` 由 pipeline 漏斗唯一赋予（ISS11 同手法）；`tests/subquery_test.rs:632` 锚点校准（RED 先行）+ `grep "execution error: execution error"` 零命中。T4 `tests/e2e_test.rs` +5 旅程（E14：ISS16 WHERE/投影位相关标量、ISS17 EXISTS 四形态 + 审计新增 NOT EXISTS 别名与 JOIN 双臂别名、ISS18 IN×别名×索引、ISS19 单层前缀端到端，各带无别名对照）27/27。同批覆盖缺口归因（既有 57 子查询用例内层零别名、AntiJoin 零覆盖、`:632` 锁定 bug 为缺口主因）与同类面审计四新形状（NOT EXISTS/NOT IN 别名全放行、EXISTS JOIN 双臂恒空集、Derived 别名截断）随 T1 矩阵与 T4 落地。全量 2029 tests / 0 failures / 2 ignored（1999 基线 + 矩阵 22 + lib 3 + e2e 5，M19 分批串行一次，对账吻合）；fmt / clippy 零新增。Plan Review accepted（独立 diff 阅读 + 消费方矩阵核查；非阻塞 Issue 候选一项：AntiJoin `extract_param_values` `filter_map` 静默丢弃未加固——T1 后经 SQL 不可触达、R2 显式只约束三执行器，用户未裁定落账，留档 Cycle Plan Review Findings 1）。3 域 delta 合并——`sql-table-alias-resolution` +1 ADDED（现 7 Requirement）/`correlated-subquery-cache` +1 ADDED（现 6）/`error-reporting` MODIFIED「错误消息前缀单层」追加执行器通道（现 8），域数 64 不变、validate --specs 64/64。同批台账闭环：ISS16–ISS19 建账即闭（issues/archive/、R44–R47 状态 closed 含各自修复摘要）。**下一里程碑 MS22 实测驱动性能批**（候选范围量化定稿后转 ready）。

- **ISS15 table 渲染截断与组合面 e2e 补齐**（2026-10-15）：change `2026-10-14-iss15-table-truncation-and-combination-gaps` 单 Iteration 五任务全 accepted 收尾归档。T1 `render_table` 列宽封顶 + 省略号截断（`CELL_WIDTH_CAP=64`、表头同规则、chars 边界安全、widths 按截断后文本计——≥65536 单元格 std::fmt 动态宽度 panic〔ISS15，exit 101〕单点关闭；主命令与 stats/sample/profile 经 `render_rows` 委托同面覆盖；json/csv/tsv 保真逐字节不变；lib 单测边界矩阵先行 RED〔4 failed 1 passed，panic 于 render.rs:85 与契约预测一致〕→ GREEN 17 passed）+ T2 e2e 大值×四格式（E8 扩展：csv/tsv 读回 2,000,000 字符全值保真、table exit 0 恰 3 行且数据行 64 字符 `...` 结尾）与大值×分析子命令（E9 扩展：独立 big_stats 表 70,000 字符值，stats/sample/profile × table exit 0、stats × json min/max 全值——用户实测 panic 路径）+ T3 import×加密库与分析子命令×加密库（E7 扩展：对钥 import exit 0 且 COUNT 可查、错钥 exit 5、`--key pw stats enc s --format json` exit 0——key 穿线通道首次见证）+ T4 `--readonly`×stats/sample/profile 拒绝矩阵 +3（readonly_session_test 既有循环断言自动覆盖，exit 2 点名 flag 归属）+ T5 README 双语与 SKILL.md table 截断说明（64/61/`...` 三处一致）；1 域 delta 合并（`cli-noninteractive-shell`「输出格式四态」MODIFIED +「大值行集输出组合（锁定）」「import 与分析子命令非默认运行形态（锁定）」2 ADDED），域数 64 不变、validate --specs 64/64；新增 lib 单测 5 例，e2e/readonly 均为既有函数内扩展零删除行；全量 1999 tests / 0 failures / 2 ignored（1994 基线净增 5；Iteration Gate M19 分批串行一次，T5 文档-only 未重跑）；ISS15 关闭归档（R43 → issues/archive/）；journey 层缺席项（TRUNCATE/ALTER/SIGINT/WAL 水位/completions/readonly 完整旅程/schema/list）维持分层现状不提升，TTY 默认格式分支自动化结构性不可达（用户接受）；来源：2026-10-14 用户实测评测〔ISS15 panic + 组合空格扫描确认四组零见证组合〕+ 用户裁定「issue 和这些 e2e 的修复一起规划到一个 change」。

- **e2e 旅程验证与缺陷清偿批（e2e 验证面 + ISS05–ISS14 全量收口）**（2026-10-14）：change `2026-10-13-e2e-verification-and-defect-closure` 六 Iteration 全 accepted 收尾归档。Iteration 000 e2e 旅程套件（T1：`tests/e2e_test.rs` 22 测试函数〔12 passed + 10 `#[ignore]` 承载 ISS 标记〕，`--ignored` 10/10 pre-fix RED 见证；夹具三处自检修复〔管道并发读防死锁、两处假阳性断言〕；E1–E13 覆盖全生命周期/导入/崩溃恢复/索引/并发锁/跨库/加密/大值/分析子命令/输出面/派生视图/文案/错误面）。Iteration 001 索引与树结构正确性（T2：ISS05 机制调查修正——回填无辜、真凶为等值下探遇等值分隔键跳过边界子树〔`search_all_from_page_async` 闭区间下探 + `delete_exact` 同集合下探〕；同面新缺陷 B1 `shift_slots_right` 叶链指针清零、B2 `scan_all` 内部页当叶读 + page-0 终止符〔多层树全表 DML 静默失真〕顺带修复；`insert_separator` 分隔键落位审计修正；30000 行多叶等值连串双向严格相等、失败语句索引零副作用）。Iteration 002 DDL 持久化与原子性（T3：ISS06 自动建表未 checkpoint 崩溃砖化窗口关闭——catalog 写序重排〔列行先、表行最后 = commit point〕+ `flush_commit_point` 有序冲刷接线 + 失败清理零残留；ISS08 建表失败零残留可重试〔`open_or_init` 按 column_count 裁剪孤儿列行〕；ISS14 DEFAULT 超限 `catalog_column_row_fits` 计划期点名预检；DROP 类维持 checkpoint-based 边界）。Iteration 003 planner 读面（T4：ISS07 派生表/视图内层别名与表达式列外层可见——`building_from_derived` 放行扩至 DISTINCT 门 + 五分类门 + `alias_reroutable` + 视图展开置位，IN/EXISTS/标量抑制逐字节保持；T5：ISS09 SemiJoin/AntiJoin 基座全宽承载——`output_columns` 基表全列 + 节点 `projection` 下游裁剪 + `sort_projection` 两消费面对齐，`ORDER BY <非输出列>` 可用）。Iteration 004 文案批与文档（T6：ISS10 help 用户视角改写/ISS11 错误前缀单层〔parse_stage 去前缀 + 漏斗点单层赋予〕/ISS12 `column_type_sql` 标准长名/ISS13 `RTSQL_BUSY_TIMEOUT` env 非法值点名 + 断言校准矩阵〔cli_test +3、九处渲染断言、pipeline/lifecycle 单测、full_table_dml 双层前缀、ctas/import_input 等逐条记录〕；T7 README 双语五条 + 六组命令人工实测）。Iteration 005 技能文档与 IN 首列（T8：`get_subquery_first_column` 增 `IndexScan`/`IndexScanAll` 两臂——Iteration 003 Issue 候选按用户 D-4 裁定并入修复，`projected_columns` 提 `pub(crate)` 两消费方规则同源；T9 SKILL.md 五条落点 + 原 192 行页满句改计划期点名）。新增 `tests/e2e_test.rs` 22 例 + `tests/secondary_index_equality_test.rs` + `tests/ddl_crash_recovery_test.rs` 6 + `tests/default_precheck_test.rs` 5 + `tests/subquery_test.rs` +8（r8 矩阵 5 + r5 扩展 3）等；全量 1994 tests / 0 failures / 2 ignored（1929 基线净增 65）；8 域 delta 合并（`sql-secondary-indexes`/`sql-views`/`cli-noninteractive-shell` 各 MODIFIED ×2、`join-output-shape-resolution` ADDED ×2、`error-reporting`/`sql-write-surface`/`wal-recovery-replay-integrity`/`in-subquery-join-rejection` 各 ADDED ×1——共 7 ADDED + 3 MODIFIED），域数 64 不变；每 Iteration 一次全量预算（M19 分批）逐轮执行；来源：2026-10-13 用户双指令 + 2026-10-12 外部评测 13 项反馈（R32 分析，10 项落账 ISS05–ISS14，ISS05 机制段以本 change 调查为准）

- **外部评测反馈批（计划形状/恢复收敛/JOIN 装配/子查询面/全表写/SQLSTATE/文档）**（2026-10-06）：change `2026-10-04-external-review-gaps` 四 Iteration 全 accepted 收尾归档。Iteration 000 计划形状与恢复正确性批（T1 IndexScanAll 列元数据契约统一——`columns` 恒 base_schema、输出形状由 projection 唯一决定，二级索引等值非首列投影 panic 与静默错位双缺陷关闭，EXPLAIN/聚合/CTAS/INSERT SELECT 消费面对齐；T2 恢复会话 WAL 收敛——`RecoveryResult.wal_had_tx_records` → 零工作守卫第三条件，未提交 SIGKILL 后纯读会话 close/checkpoint 收敛、`check`/`--readonly` 永久 exit 1 楔死解除、marker-only 形态收敛、MS18 零工作跳过保持；T3 JOIN 基座 ORDER BY 限定键；T9 自连接侧身份正确性——交换判定按声明限定符、`JoinSide`/`output_sides` 显式侧标志归属、双腿同侧 `InvalidQuery` 点名）〔含 000-initial blocked → 001-replan：T9 经用户裁定并入〕。Iteration 001 子查询面与 JOIN 装配收口批（T4 WHERE 标量子查询比较 + 投影位形状——`ScalarSubqueryFilterNode/Executor` 逐行参数注入 + 语句级缓存、多行拒绝/零行 NULL 三值、JOIN 基座维持拒绝、投影恒等门摘除消除全表列泄漏；T5 FROM 派生表内集合操作——`building_from_derived` 独立标志、IN/EXISTS 内维持拒绝；T11 多表链式 JOIN 装配正确性——非末步链前缀全组合承载、链前缀列绝对偏移重定基、条件键（真实表,列）限定解析，计划失败/panic/静默串列三形态全关闭；T10 JOIN 基座输出列序 SELECT 化——kept 三清单按 SELECT 序重建、WHERE/ORDER BY 裁剪路径随之）〔001-subquery/000-initial 原始计划未执行即被 001-replan 取代——用户 2026-10-05 裁定 T10/T11 并入〕。Iteration 002 写面与 CLI 可观测批（T6 无 WHERE 全表 UPDATE/DELETE——第一重门移除、执行器 `scan_all` 全表臂复用单行逻辑、逐行碰撞隐式事务回滚、附加库路由继承、TRUNCATE 对照零变化；T7 CLI 错误输出携带 SQLSTATE——文本行尾 ` [SQLSTATE xxxx]` + JSON `sqlstate` 字段缺省省略，退出码与文案零变化、PG 面零变化）。Iteration 003 文档同步批（T8 README 双语 + SKILL.md 按 D8 七项清单逐条对齐：JOIN 基座限定排序、标量子查询边界、全表写声明、SQLSTATE 输出面、FROM 集合操作、纯读恢复收敛话术；Act/Review 双语逐句位核对 + 人工实测六组命令）。新增 `tests/recovery_wal_convergence_test.rs` + `tests/scalar_subquery_comparison_test.rs` 14 + `tests/chain_join_test.rs` 9 + `tests/self_join_test.rs` + `tests/full_table_dml_test.rs` 10 + table_alias/set_operations/join_output_resolution/secondary_index/subquery/cli/readonly_session 等扩展；全量 1929 tests / 0 failures / 2 ignored（1882 基线 + Iter001 +35 + Iter002 +12）；10 域 delta 合并（`cli-noninteractive-shell` +1 / `cross-db-dml` ~1 / `join-output-shape-resolution` +2 / `ms07-rest-tx-checkpoint-pushdown` ~1 / `sql-expression-evaluation` +2 / `sql-secondary-indexes` +1 / `sql-set-operations` +1 / `sql-table-alias-resolution` +1~1 / `sql-write-surface` +1 / `wal-recovery-replay-integrity` +1——共 10 ADDED + 3 MODIFIED），域数 64 不变；来源：2026-10-04 外部评测 8 条反馈〔7 成立 1 不成立；8a/8b 随 T4/T6 语义修复消解、8c 当前版本不复现无工作项〕+ Iteration 000 Review 两范围外 Issue 候选（T10/T11，用户裁定并入不落账）。

- **practical relational core 缺口批（建表页初始化 + 聚合嵌入 + UPDATE 表达式 + 非 INT PK 诚实化 + 恢复错误面）**（2026-10-04）：change `2026-10-03-practical-core-gaps` 三 Iteration 全 accepted——Iteration 000 正确性与诚实化小批：T1 建表首数据页初始化（`create_table_with_constraints` 在 `allocate_page()` 后经 `SlottedPage::init` 初始化 0x03 页头，全建表通道单点覆盖；裸建表即含零行表的库 `rtsql check` exit 0 单行干净；`check` 对 0x00 页 error 分级保持；存量 0x00 head 最小修复语义——open 不自愈、首次写入/TRUNCATE 转绿）；T2 非 INTEGER 显式 PRIMARY KEY 计划期拒绝（`UnsupportedConstraint("NON-INTEGER PRIMARY KEY")` 五类型 × 列级/表级矩阵；隐式首列主键与 import 通道零波及；存量非 INT PK 表可用）；T3 dump DDL 渲染适配（`create_table_sql` 仅 INTEGER PK 输出 `PRIMARY KEY`，存量表 dump→restore 往返可达）。Iteration 001 聚合表达式嵌入：T4 提取-注册基建 + SELECT 项嵌入与常量混排（`contains_aggregate_expr` + 条件 Projection 包装通道）；T5 HAVING 完整表达式编译；T6 ORDER BY 聚合表达式键；T7 拒绝面收口（WHERE/ON 聚合、嵌套聚合、NonAggregated 固化）。Iteration 002 写面能力 + 遗留并入：T8 UPDATE 计划面多列化（`UpdateAssignment`/`UpdateValue` 节点演进、右值五臂路由、子查询 `UPDATE SUBQUERY` 点名）；T9 执行器多列化（同旧行快照求值、per-assignment 门序、预检前置、Step 7 按 `pk_assigned` 三分支）；T10 SKILL.md/README 双语同步；T11 恢复重放目标表缺失定向错误面（`redo_table_lookup_error` 变体判别——`TableNotFound` 四要素文案〔表名/catalog 缺失/未 checkpoint 关闭原因/打开拒绝〕，其余归并文案逐字节不变，K05 打开拒绝不变）；T12 聚合显示名重名去歧义（`compile_aggregate_row_expr` 按规范名解析 + `register_aggregate_calls` 去重键改规范名 + wrap 臂按注册序号定位，S24 四形态修复）。新增 `tests/update_expression_test.rs` 14 + `tests/recovery_error_surface_test.rs` 3 + aggregate_extension +5 + planner +7 + Iter000/001 测试扩展；全量 1862 tests / 0 failures / 2 ignored（1802 基线净增 60）；增量规格 5 域各 +1 Requirement（`data-integrity-verification` 现 3 / `wal-recovery-replay-integrity` 现 6 / `sql-aggregate-extensions` 现 5 / `sql-constraint-enforcement` 现 7 / `sql-write-surface` 现 9）+ `cli-noninteractive-shell` dump/restore MODIFIED，域数 64 不变；来源：2026-10-03 外部评测探针实测四缺口（2026-10-03 用户四项集中决策批准）+ Iter000 遗留 Issue 候选并入（用户指令）。

- **WAL 恢复重放 tail 收敛修复（恢复收尾写回 + 写路径自愈）**（2026-10-03）：change `2026-10-03-recovery-tail-persistence` 单 Iteration 双 Cycle 全 accepted——父 Cycle `000-initial` Act 阶段 T1 契约核读触发 Stop-when 阻塞返 Plan（Plan「三臂全经 `redo_tuple_at_row_id`」断言失实：Delete 墓碑臂直调 `write_tuple_to_data_page` 绕行，原 `table_last_page` 写回集合对该类表欠覆盖且计划内 INSERT 驱动测试不可见），Plan Review `rework-required`；rework Cycle `001-rework` 修复：T1-R1 写回集合改为 catalog 表清单 + 内存/持久 tail 差异比较（结构性覆盖任意重放臂，失败 `RedoFailed` 点名表、`redo_count == 0` 零变化）+ T1-R2 DELETE-only 重放见证（28B 墓碑 footprint 几何 + 夹具自检防失准，对旧机制必失败的直接 gap 见证）；T2 写路径 `heal_stale_tail` 自愈（内存 tail 页 `next ≠ 0` 沿链至 terminal、内存/持久双写回、健康路径单次探测零行走查、恢复面共享原语隔离保持）；T3 `check_command_test` 恢复后形态断言白名单校准（info 行 → 干净单行 exit 0）。新增 `tests/recovery_tail_persistence_test.rs` 4 用例 + `stale_tail_heals_on_write` 单元测试；全量 1802 tests / 0 failures / 2 ignored（1797 基线 + 净增 5）；增量规格 `wal-recovery-replay-integrity` +3 Requirement 现 5（域数 64 不变）；来源：MS27 Plan Review 002 确认的 Issue 候选（用户裁定立 change 解决，集中决策 A 双层修复）。

- **MS27 存储演进批（超页溢出页链 + rtsql check 完整性校验）**（2026-10-03）：change `2026-10-02-ms27-overflow-integrity` 三 Iteration 全 accepted——Iteration 000 溢出表示与存储原语：T1 长字符串 tag 0x08（serialize/deserialize 三入口 + compute_tuple_size + 65535/65536 切换点）；T2 `overflow.rs` 溢出页格式（page_type 0x04）与链原语（locator 编解码/chunk 分解/链装配读/收集释放）；T3 写路径 >4074B 溢出分支（先链后 locator、失败清理零泄漏）；T4 读路径两相装配（`read_tuple_from_data_page` Overflow 装配 + DataScan `YieldOverflow` + `find_visible_version` 出锁装配、内联零拷贝保持）。Iteration 001 生命周期、恢复与全通道：T5 恢复重放链页重分配 + locator 末步落位幂等 + `build_pk_version_maps`/`rebuild_pk_indexes`/`derive_old_row_id` 四消费点两相装配（CREATE INDEX 回填遗漏同轮修复）；T6 GC/drop/TRUNCATE 溢出链释放（delete 单点 + `collect_overflow_chain_pages` 接线）；T7 全通道与形态 e2e + ~1MB 基线（矩阵/tag 切换点/CTAS/跨库/加密/`:memory:`/只读/dump-restore/import）；T8 README 双语 + SKILL.md 长值边界（含 DEFAULT 字面量 ~4KB catalog 页内容量约束）；T12 catalog DEFAULT 段长形式（`[0x08][u32 len]`，>65535B；2026-10-02 用户指令 replan 并入、design D11 表示一致性硬化）。Iteration 002 rtsql check：T9 lib `integrity.rs` 只读结构扫描（页 census/catalog/数据链 walk/bounds-checked slot/溢出链/三类索引树、损坏零 panic、自实现 walk 替代 tolerant 收集器）；T10 CLI `check` 子命令（readonly 编排、error/info 分级——恢复后 tail 良性领先 info 不改退出码、退出码 0/1/4/5/2 矩阵、主库文件全字节不变）；T11 README 双语命令表 + SKILL.md Lifecycle/Behavioral notes。新增 `tests/overflow_page_test.rs` 27 + `tests/check_command_test.rs` 8 + integrity 单测 6；全量 1797 tests / 0 failures / 2 ignored；增量规格 +2 域（`overflow-page-storage` / `data-integrity-verification`），语料 62 → 64 域；Plan Review 002 三项 Minor finding 记录不返工；范围外 Issue 候选一项（WAL 恢复重放不持久化 data_page_tail）待用户裁定。

- **MS20 二期跨文件 DML（附加库写入 + INSERT SELECT + 缺陷收口）**（2026-10-02）：change `2026-10-02-ms20-cross-db-dml` 三 Iteration 全 accepted——Iteration 000 附加库普通写入与生命周期：T1 目标事务/metadata/WAL/缓冲池/回滚路由（abort 表键用 metadata 本地名，四写执行器经既有路由取目标库资源）；T2 全部退出路径统一关闭附加库（AttachSession 所有权移 `execute_command_inner_mode` 编排器、select 外持有；关闭失败原执行成功时 exit 1 点名、已有错误/信号保留退出码）；T3 显式事务 0A000（源求值与 begin 前点名、主库目标+附加源不误拒）、只读 25006 优先、DDL/CTAS 附加目标拒绝保持、目标库恢复两态一致。Iteration 001 查询结果写入与文档：T4 `PhysicalPlan::InsertSelect` 节点 + `build_insert` 可变借用房 `build_query` 源编译、源表注册进 `extract_all_table_names`、目标列映射/全宽 DEFAULT/NULL 填充、新节点不进 plan cache；T5 源完整物化原生 Value 行（不经 JSON/SQL 往返）→ 单隐式事务写入、失败回滚本语句全部行与索引、ON CONFLICT/REPLACE 复用、主库显式事务复用事务、空源 affected_rows=0；T6 README 双语 + rtsql-docs 使用面。Iteration 002 缺陷收口：T7 两段附加 FROM 项隐式短名关联名（尾段经既有 `register_table_alias` 登记、`qualifier_matches_declared` 全子句面匹配接线、显式别名优先、字面含点不误登记）；T8 恢复重放按 WAL logical_id 强制落位（`SlottedPage::add_slot_with_logical_id` 恢复面专用、dense-assignment 硬失败删除、K05 收窄为页满/IO、运行期零变化）；新增 `tests/cross_db_dml_test.rs` 17 + `tests/insert_select_test.rs` 10 + `tests/recovery_replay_convergence_test.rs` 3 + cross_db_query_test +4 与 planner/slotted_page 单测；全量 1739 tests / 0 failures / 2 ignored；增量规格 +1 域 `cross-db-dml` + 5 处修改（`cross-db-namespace-query` 附加库只读边界收窄、`readonly-open-mode` R5、`sql-write-surface` +INSERT SELECT、`sql-table-alias-resolution` R1、`wal-recovery-replay-integrity` R1），语料 61 → 62 域。

- **MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）**（2026-10-01）：change `2026-10-01-ms29-import-input-and-default-declaration` 两 Iteration 全 accepted——Iteration 000 import 输入面：T1 stdin `-` 通道（对齐 restore 先例、records 预收集/逐条 auto-commit 零变化）；T2 目标表缺失按 CSV 表头自动建表（`csv_field_guess` 猜测 → `infer_ctas_column_types` pub(crate) 复用推导、冲突 General exit 1 点名零副作用、常规建表通道守卫自然生效、首列隐式主键语义〔INT 首列重复 DuplicateKey fail-fast、FLOAT/STRING 重复合法〕——首 Cycle T2 两场景 Gate 5 阻塞经 replan 修订〔Plan 错误 ×2：引擎无表级无主键、重复列名守卫不存在〕）；T3 `:memory:` import 豁免（`--key` exit 2、其余子命令维持拒绝）；T8 planner 重复列名计划期守卫（replan 增入，既有「静默建成且不可 dump 往返」缺陷闭环）；Iteration 001：T4 `validate_default_value` 单点建表校验/归一（NULL 豁免/FLOAT 升格/日期族解析验证/异型 `DefaultTypeMismatch` 点名）+ T5 TypedString 臂（日期族 DEFAULT dump→restore 往返修复）+ T6 ALTER ADD COLUMN 同款 + T7 SKILL.md/README 双语同步；新增 `tests/import_input_surface_test.rs` 12 例 + `tests/default_declaration_test.rs` 14 例 + planner_test +6；全量 1689 tests / 0 failures / 2 ignored；增量规格合并 2 域（`cli-noninteractive-shell` import MODIFIED、`sql-write-surface` +2 Requirement 现 7），域数 61 不变。

- **MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）**（2026-10-01）：change `2026-10-01-ms28-cli-concurrency-checkpoint` 两 Iteration 全 accepted——T1-T4 锁等待预算策略化（storage `open_with_lock`/`open_readonly_with_lock` deadline 循环 + lib `open_with_lock_wait` 专用线程让出、信号即时中断 + CLI `--busy-timeout`/env 三通道穿线全部开库命令与 ATTACH、默认 5000ms/显式 0 仅下限、存量锁用例 D8 校准断言零变化）；T5 `rtsql checkpoint <db>` 显式落盘（零工作明示两形态、缺库 exit 1、锁冲突经预算 exit 4、`--readonly`/`:memory:` exit 2、加密对钥 exit 0/错钥 exit 5）；T6 SKILL.md/README 双语命令面同步（含只读句 DETACH 不实声明移除）；新增 `tests/busy_timeout_test.rs` 18 例 + `tests/checkpoint_command_test.rs` 6 例；全量 1657 tests / 0 failures / 2 ignored；增量规格合并 3 域修改（`cli-noninteractive-shell` +2 Requirement、`database-file-lock` R1、`readonly-open-mode` R1 + Purpose 同步），语料域数 61 不变。

- **MS26 嵌入式运行形态（只读并发 + `:memory:` + WAL 水位 + 派生投影收口）**（2026-09-30）：change `2026-09-30-ms26-embedded-runtime` 四 Iteration 全 accepted——T1/T2 只读打开模式（共享锁多读单写 + WAL 未决拒绝 + 零写入会话 + 13 写变体 25006 + CLI `--readonly` + ATTACH 只读传递）；T3/T4 `:memory:` 库形态（`MemoryStorage` + `open_in_memory()` + CLI 特判与拒绝面、零文件退出即弃）；T8 派生基座外层投影收口（FROM 子查询/CTE/视图三通道裁剪行+表头、未知列 42703，2026-09-30 用户指令 Issue 候选本 change 解决）+ T9/T10/T7 收尾批（组合拒绝用例 / clippy 清零 / 孤儿移除 / 文档三处）；T5/T6 WAL 字节水位（提交边界 `maybe_auto_checkpoint` 单点、16 MiB 默认 + env/lib 双通道、0 禁用、顺延、失败传播且 CTAS 表存活、死接口删除）；增量规格 +3 域 + 3 处修改，语料 56 → 59 域；全量 1590 tests / 0 failures。

- **MS21 DDL 演进与二级索引（CREATE/DROP INDEX + ALTER TABLE + TRUNCATE + VIEW + 索引一致性收口）**（2026-09-30）：change `2026-09-29-ms21-ddl-evolution` 六 Iteration 全 accepted——T1 索引条目维护一致性收口（rekey UPDATE 回滚按 rekey 语义回退、SET 唯一列为 NULL 的旧键还原、UPSERT `SlotNotFound` 容忍臂与 DELETE 对齐）；T2 二级索引端到端（catalog secondary 尾段 + `CatalogRootSlot::Secondary` + `IndexManager::delete_exact` + 建索引回填 + 写路径六处挂钩 + 恢复重建扩展 + 非 PK 等值 `IndexScanAll` 路由 + DROP INDEX 页释放回落）；T3 ALTER TABLE ADD/DROP COLUMN（`deserialize_tuple_width` 宽度感知 + DEFAULT 覆盖层 + 尾部加列 + 内隐式事务逐行重写删列 + 索引引用/最后一列守卫）；用户扩围 T4 TRUNCATE TABLE（checkpoint-first 序 + 数据页释放 + 三类索引空重建 + 无墓碑崩溃不复活 + 事务内拒绝 + 空表幂等）、T5 CREATE/DROP VIEW（catalog kind=View 行持久 + 视图注册表 + 创建期验证封死循环 + CTE 通道展开与内联逐字节一致 + 显式列清单 + DML 只读拒绝 + 非基表 DDL 目标拒绝 + plan cache 全清 + 附加库引用创建期拒绝）；二次扩围 T6 删除原语精确化（`LeafNode::delete` 精确键验证，DELETE 未命中零副作用不误删邻近条目/不误报 KeyNotFound）、T7 回滚 NULL-rekey 还原补口（SET 唯一列为 NULL 回滚后旧值占用恢复）；增量规格 +4 域（`sql-secondary-indexes` / `alter-table-column-evolution` / `truncate-table` / `sql-views`）+ `update-index-maintenance` 扩展至 5 Requirement，语料 52 → 56 域；全量 1541 tests / 0 failures / 2 ignored（净增 4 测试文件 47 例 + catalog lib 单测）。

- **MS20 一期 ATTACH 式跨库交互（ATTACH 注册 + 跨库只读查询 + CTAS 主库物化）**（2026-09-29）：change `2026-09-29-ms20-attach-cross-db` 三 Iteration 全 accepted——T1 ATTACH 会话注册面（`resolve_db_path` 语义复用、独占锁 `try_lock` 冲突 exit 4、自附加 flock 同 inode 拒绝、重复别名/路径缺失零副作用拒绝、lib session-only 点名拒绝、DETACH 豁免登记 I076）；T2 两段限定名路由（`alias.table` 全表名消费点先查注册表、未命中扁平名回落零回归、三段列引用路由命中可用、`AttachRoute`/`RoutedTables` 计划期判定执行期取库、附加库缺表 42P01 点名、含附加库引用语句不进 plan cache）；T3 附加库只读边界（五写消费点计划期点名拒绝、DDL 不查 catalog 即标记）；T4 CTAS 主库物化（列名取 SELECT 输出表头、类型推导矩阵——全 NULL→String(255) 含零行退化/Int∪Float→Float/同型保留/其余混合 `CtasColumnTypeConflict` 建表前点名拒绝、输出列名重复计划期拒绝、目标已存在守卫前置物化、建表→`flush_all()`→隐式事务插入崩溃一致性序、失败 abort+尽力 drop 零副作用同名可重试、隐式主键 DuplicateKey、`CREATE TABLE o.m AS SELECT` 计划期拒绝、LIKE/CLONE/列清单+query/表约束+query 诚实化拒绝、显式事务内 execute 期点名拒绝 0A000、执行后 plan cache 全清）；Review 补跑 Act 未覆盖的全量串行 Gate（1458 passed / 0 failed / 2 ignored）；增量规格 +3 域合并（`sql-attach-session`/`cross-db-namespace-query`/`create-table-as-select`，含事务内收口语义与零行源退化两处校准），语料 49 → 52 域。
- **MS25 SQL 读面补全（外连接/DISTINCT/集合操作/CTE/ORDER BY 别名排序）**（2026-09-28）：change `2026-09-26-ms25-sql-read-surface` 五 Iteration 全 accepted——T1–T7（OUTER JOIN 三态经 NLJ 统一承载 LEFT/FULL + RIGHT 计划期左右交换、WHERE×JOIN 解禁含 INNER、SELECT DISTINCT 三分支链位、UNION[ALL]/INTERSECT/EXCEPT 左结合展开与非递归 CTE、COUNT/SUM/AVG DISTINCT 真实去重与 group_concat、EXPLAIN 计划树文本不执行内层、ORDER BY 输出别名真实排序含 decorate-sort-undecorate）与 T8–T13（JOIN 基座聚合参数/分组键完整形状解析 + 同名裸名歧义 `AmbiguousColumn` 点名 + 未知列 `ColumnNotFound`、聚合直出行/表头形状一致性收口、DerivedScan 基座普通列别名应用、信号处理器注册先于 open 锁获取、聚合参数两基座计划期解析——未知列与别名引用由静默 0/NULL 转 `ColumnNotFound` 点名、`SELECT t.*` 拒绝文案点名 QUALIFIED WILDCARD）；收口三处静默错误结果/静默不出错缺陷（DISTINCT 静默不去重、CTE 静默丢弃、COUNT DISTINCT 静默忽略）；增量规格合并 +32 added / 5 modified / 1 removed，语料 40 → 49 域；全量 1408 tests / 0 failures / 2 ignored。
- **MS18 one-shot 路径性能收口**（2026-09-26）：change `2026-09-26-ms18-oneshot-fixed-cost` 双 Iteration 全 accepted——checkpoint 零工作跳过（`CheckpointManager::checkpoint` 顶部守卫：`write_count == 0 && !BufferPool::has_dirty_pages()` → 整体无操作早退 `Ok(0)`，九步全量路径与「close 即落盘」逐字节不变；谓词经 Plan 调查修正为双条件——checkpoint 后 WAL 恒含标记记录致文件长度探针永假、DDL 不写 WAL 致单条件丢表，安全论证按未决工作来源穷举；新增 `tests/checkpoint_skip_test.rs` 6 场景 + 2 单测 RED→GREEN 见证，one-shot 时延 12.35→5.85ms 背靠背）+ runtime flavor 实测定形（两 flavor × 三组实测 R26 口径 ×3 中位，批准判据合取 RSS 臂 +2.1% FAIL → 保守分支保持 multi_thread；I063 RSS 假设证伪与 current_thread 时延 -68% 留档 design D5）+ README 双语对比板块刷新（One-shot 10.8→6.5 ms、Peak RSS 16.7→16.4 MiB、注记与 prose 归因修正）；增量规格合并 `openspec/specs/ms07-rest-tx-checkpoint-pushdown/`（+1 Requirement「Checkpoint 零工作跳过」，现 4 个）；全量 1285 tests / 0 failures / 2 ignored。
- **MS19 CLI 管理面小收口**（2026-09-26）：change `2026-09-26-ms19-cli-admin-hardening` 三 Iteration 全 accepted——`rtsql delete <db>` 删库子命令（锁探测同窗口删除、伴生清理、逐文件报告、`--dry-run`、缺失/占用拒绝、加密库免密钥、corrupt 可删）+ I072 文件对备份边界 7 形态实测与 README 双语声明（沉淀 Runbook R30）；`PlanError` 携带能力名 + PG 协议错误码按类分类（0A000/42601/42P01/42703/42702，其余 58000 兜底，JSON 字节不变）+ 会话事务 × 边界子句组合 e2e 锁定；列解析静默面收口（未知投影列/未知排序键计划期 `ColumnNotFound` 点名拒绝、纯查询 `col AS alias` 别名表头生效、WHERE/值表达式列缺失重分类 42703；T6 谓词经 Plan Review 裁定收窄为「输入列集 ∪ 顶层投影输出别名」以保持语料 R5 既有静默语义）；全量 1277 tests / 0 failures / 2 ignored。
- **MS24 SQL 写面补全（子集 INSERT 与 UPSERT）**（2026-09-26）：change `2026-09-25-ms24-write-surface-completion` 双 Iteration（000 写入类型门与子集 INSERT/DEFAULT / 001 UPSERT 与 REPLACE INTO，其 replan Cycle 承接 PK 预检补齐与回滚后墓碑行索引条目还原）全 accepted——非键列写入值类型门端到端生效（ISS04 收口，dump/restore/import 零误报）、子集列清单 INSERT 与 DEFAULT 持久化与应用闭环（跨重启与 dump→restore 保真）、`ON CONFLICT DO NOTHING/DO UPDATE`（字面量 + `excluded.col` + 旧行裸列三形态赋值）与 `REPLACE INTO` 端到端可用（冲突仲裁确定、碰撞预检零副作用、恢复两态一致）、删除者事务回滚后 PK 与唯一索引条目按存活版本还原；全量 1239 tests / 0 failures / 2 ignored。残留 MS24-T04（声明期 DEFAULT 类型校验）未实施。
- **MS23 数据完整性约束执行面**（2026-09-25）：change `2026-09-24-ms23-constraint-enforcement` 双 Iteration（000 约束诚实化与 NOT NULL 强制 / 001 UNIQUE 强制端到端）全 accepted——NOT NULL 写入零副作用强制、CHECK/FK/方言项与非 INT/组合 UNIQUE 计划期点名拒绝、INT 列 UNIQUE 经专属唯一索引端到端强制（INSERT/UPDATE/DELETE/回滚 + 干净重开与崩溃恢复两态一致）+ F1 修复轮（唯一列非 Int 值 `KeyTypeMismatch` 守卫）；全量 1152 tests / 0 failures / 2 ignored；同轮登记 ISS04（非键列写入类型校验缺失，R29）。

历史完成记录（2026-05-24～2026-09-24，36 行）已全部归档至 carrier
`openspec/changes/archive/2026-09-24-ARC-202609242151/`（`archive/tasks-completed-roadmap.md`）；
当前项目状态见 SNAPSHOT「仓库现场」与各归档 change carrier。
## 与 OpenSpec Changes 同步

- 每个 MSxx 内的 MSxx-Txx 实施时通过 `openspec/changes/<date>-<t-tag>/` 创建 change
- 完成的 change 通过 `openspec archive` 归档
- 归档的 change carrier 保持不可变
- 新发现的问题写 `.agents/memory/improvements.md` (Ixx) — **注意**：Ixx 编号待重新审视，旧 Ixx 多数已重新归位到 MSxx-Txx
- 完整迁移的旧版 entry 记录在 `.agents/legacy/2026-08-25-openspec-init-migration/COVERAGE.md`

<!-- arc: ARC-202609242151 --> 6 块已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609242151/proposal.md

<!-- arc: a61891f --> 11 段已归档 (2026-10-20)：MS18-MS21、MS23-MS29 已完成里程碑段（用户指令「做完了的不管时间长短也都进行归档」；MS22/MS30-MS32 planned 保留；各段 Status/Outcome/Scope 原文经 git show a61891f:.agents/docs/tasks.md 取回，完成摘要仍见「最近完成」段）
