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

<!-- arc: a61891f --> 11 段已归档 (2026-10-20)：MS18-MS21、MS23-MS29 已完成里程碑段（用户指令「做完了的不管时间长短也都进行归档」；MS22/MS30-MS32 planned 保留；各段 Status/Outcome/Scope 原文经 git show a61891f:.agents/docs/tasks.md 取回）

<!-- arc: 4190580 --> 「最近完成」段已归档 (2026-10-20)：2026-09-25～2026-10-15 十九批完成记录（用户指令「最近完成也可以清理，没必要保留」；各批原文经 git show 4190580:.agents/docs/tasks.md 取回，权威记录在各归档 change carrier；完成状态摘要见「路线图结构」段与「建议执行序」✓ 标记）
