# tasks — 任务与里程碑路线

> 最后更新：2026-10-24（**MS33 完成收尾**：持久化失败与 DROP 边界经单 change `2026-10-23-ms33-drop-persistence-failure` 交付归档——两 Iteration 全 accepted：000 checkpoint 前崩溃〔D1 页清零从释放时移到空闲页弹出时——释放不写盘、分配先写全零页、写零失败页号回退；7 用例覆盖 DROP TABLE/INDEX/VIEW 崩溃保留、同会话再分配、删列、加密建表/删表；改前基线 DROP TABLE/加密删表丢行、DROP INDEX 重开 abort〕/ 001 受控 I/O 失败〔类型化开关 `arm_io_failure`（DataWrite/WalFsync，系统调用前失败一次自清除）+ 三恢复用例——数据文件写失败回 DROP 前状态，WAL fsync 与位点目录失败 DROP 已生效且 check 无 error；Review 期修正 spec 类属：WAL fsync 为九步第 3 步晚于 flush_all、数据文件 sync 无调用点不设开关位、R1 复用窗口界定留 MS35〕；全量终态 2155/0/2；行为规格新增 drop-persistence-boundary 域〔5 Requirement/10 Scenario〕+ wal-recovery-replay-integrity 的 DDL 边界 requirement 修改。引擎正确性批下一项 MS34，MS35/MS36 依赖已满足）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

当前活跃路线有 3 个 planned 里程碑：MS34–MS36（2026-10-23 用户批准，承接 I090、I092、I087；对 MS33 的依赖已满足）。MS22 于 2026-10-23 superseded 退回 improvements；MS30–MS33 为 completed。历史总数曾写作 33（22 completed + 6 superseded + 5 planned，6 个历史段在 carrier `ARC-202609242151`），该算术未按本次状态重算已归档段。初版后统一路线，2026-09-24 规划：MS18-MS27；2026-10-01 用户批准重排，milestone-planner 增补 MS28-MS29 承接 I077/I078/I079 与残留任务；2026-10-19 用户批准底层设计批（依据 improvements 同日优先级排序），milestone-planner 增补 MS30-MS32 承接 I091/I080/I088/I089/I081/I082、执行序前置于 MS22；MS23 于 2026-09-25 完成，MS24 于 2026-09-26 完成〔T04 声明期 DEFAULT 类型校验未实施，2026-10-01 重排并入 MS29-T02〕，MS19 于 2026-09-26 完成，MS18 于 2026-09-26 完成，MS25 于 2026-09-28 完成，MS20 一期于 2026-09-29 完成〔T02 二期跨文件 DML 待后续 change〕，MS21/MS26 于 2026-09-30 完成，MS28 于 2026-10-01 完成，MS29 于 2026-10-01 完成，MS20 于 2026-10-02 完成（T02 二期随 change `2026-10-02-ms20-cross-db-dml` 三 Iteration accepted 归档），MS27 于 2026-10-03 完成（change `2026-10-02-ms27-overflow-integrity` 三 Iteration accepted 归档），MS30 于 2026-10-21 完成（change `2026-10-20-ms30-semantic-binding-differential-net` 四 Iteration accepted 归档——单 change 承载两任务为 2026-10-20 用户指令，T03/T04 为裁定增补段），MS31 于 2026-10-23 完成（change `2026-10-21-ms31-typed-api-param-binding` 三 Iteration accepted 归档——单 change 承载 T01+T02 为 2026-10-21 用户裁定），MS32 于 2026-10-23 完成（change `2026-10-23-ms32-executor-batch-and-query-memory` 四 Iteration accepted 归档——000 生产攒批未过时间门撤回、001 经 replan Cycle；单 change 承载 T01+T02 为计划期裁定）），MS33 于 2026-10-24 完成（change `2026-10-23-ms33-drop-persistence-failure` 两 Iteration accepted 归档——Review 期修正 spec 类属）。★初版达成★。

统一执行序（2026-10-01 用户裁定重排——功能补齐前置、性能优化殿后独立成批；沿用「批准执行序与编号无关」既定先例，编号只表规划先后，执行以本序为准）：**MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）→ MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）→ MS20-T02 跨文件 DML 二期 → MS27 存储演进批（超页溢出 + 完整性校验）**。MS22 已于 2026-10-23 退出本序并退回 improvements。同日批准的后续执行序：**MS33 持久化失败与 DROP 边界 → MS34 取消、超时与事务收尾**；**MS33 → MS35 空闲页跨重启复用**；**MS33 → MS36 历史库与 dump 兼容**。MS35 与 MS36 互不依赖。此前各项顺序为建议序非硬依赖（无环、前置均已满足）。编排依据（2026-10-01）：外部评审实证的自动化使用摩擦先行收口——并发硬失败 exit 4 → 可配置等待是多 agent 并发使用的语义前提，也是当时持锁窗口治理的前置；输入通道与声明语义同代码面相邻（lifecycle.rs import + ddl_dml.rs 建表/类型面）；存储格式演进体量最大放功能带末尾；性能殿后、先量化再决定（MS08 纪律）。首批排除域维持：微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058。REPL（I067）、DETACH（I076）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）、WAL fsync 组提交（I049——串行单写者模型下无效）等留 improvements 域未排期。MS22 候选池于 2026-10-23 整体退回该域。
旧优化项历史分类（各 superseded MS 原范围段）随 `ARC-202609242151` carrier 留档；未排期候选以 D-candidates 与 improvements 台账为准。
## Milestone Roadmap

### MS22：实测驱动性能优化（第一批） — superseded（2026-10-23 退回 improvements）

- **Status**: superseded（2026-10-23 用户指令：本里程碑退出路线，退回 `.agents/memory/improvements.md`。不由其他里程碑替代。原候选不再是已承诺工作。）
- **Dependencies**: None
- **Outcome**: 未形成阶段成果。原候选恢复为未承诺改进方向：I020、I021、I024、I026、I038、I049、I050、I066，以及 I077 的持锁窗口余项。I031 仍留在 improvements，不挂本里程碑。
- **Rationale**: 性能批在范围量化定稿前保持 planned。用户于 2026-10-23 要求将其退回改进台账；「先量化再决定」留在对应条目，不再占用路线。
- **Related changes**: None

### MS30：SQL 语义绑定层一期（差分测试保护网 + JOIN/子查询收口） — completed

- **Status**: completed（2026-10-21；change `2026-10-20-ms30-semantic-binding-differential-net` 四 Iteration 全部 accepted 归档——Iteration 000 差分与性质测试网〔T01〕、001 绑定作用域表基建与关联子查询分类收口〔T02-C1，含 replan Cycle 碰撞形状 SQLite 语义〕、002 JOIN 布局段源自作用域表统一〔T02-C2〕、003 差分白名单缺陷清偿与登记收口〔用户裁定增补的 T03 四缺陷 + T04〕；全量终态 2057/0/2，差分白名单 6 → 1〔余 ORD-NULL-ORDERING 已知语义差异〕；单 change 承载两任务为 2026-10-20 用户指令）
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
- **Related changes**: `openspec/changes/archive/2026-10-20-ms30-semantic-binding-differential-net/`（2026-10-21 归档）

### MS31：类型化库结果与参数绑定 API — completed

- **Status**: completed（2026-10-23；change `2026-10-21-ms31-typed-api-param-binding` 三 Iteration 全部 accepted 归档——Iteration 000 类型化结果基建与调用面迁移〔T01：公共结果类型与边界转换、入口签名替换与全调用面迁移、列类型装配〕、001 占位符与类型化参数执行〔T02 前半：AST 占位符计数与拒绝面、计划面占位符编译、参数执行通道〕、002 可复用预编译语句〔T02 后半：schema 版本戳单点、prepare/PreparedStatement、失效/并发/事务矩阵〕；全量终态 2123/0/2；单 change 承载两任务为 2026-10-21 用户裁定）
- **Dependencies**: MS30（I089 消费 I080 绑定结果；I088 的列元数据与绑定信息衔接）
- **Outcome**: 嵌入式库 API 面定型——公共结果携带原生 Value 与列名/类型元数据（CLI/JSON/PG 在边界转换），类型化参数 execute/query 接口与可复用预编译语句；调用者不再自行渲染值、可直接消费内部类型
- **Rationale**: 核心执行结果当前依赖 `network::protocol::Response`（JSON 值数组、无完整列元数据），核心↔协议耦合（I088 证据：`src/database.rs::execute_sql`、`src/pipeline.rs::execute_executor` 与 `value_to_json`）；程序调用需自行渲染 SQL 文本、重复 SQL 的计划生命周期缺公共契约（I089）。两任务严格串行、同 API 故障域，聚合一段
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS31-T01 | I088：类型化公共结果 + 核心/协议解耦（既有 execute_sql 调用者兼容迁移；CLI/JSON/PG 边界转换，输出行为面零变化） | improvements I088（2026-10-18 用户接受方向；优先级排序第 2 位） |
| MS31-T02 | I089：类型化参数 execute/query + 可复用预编译语句（定型、NULL、参数个数/类型错误、schema 变化后计划失效契约；不以字符串替换实现） | improvements I089（2026-10-18 用户接受方向；优先级排序第 3 位） |

- **Non-goals**: PG Extended Query 协议（I071 触发锁定维持）；流式游标/背压（I065 long-term）；crate 边界拆分（I074 长期方向，不以本段为前置）
- **Workload**: 中，约 2 个 change（实际：单 change 承载 T01+T02，2026-10-21 用户裁定）
- **Stable baseline**: lib 公共契约为类型化结果 + 参数绑定；`network::protocol::Response` 退化为边界转换词表——CLI/Server 消费者与嵌入式使用者各取所需；参数以原生 `Value` 绑定（位置式 `?`，不用字符串替换），预编译语句自持计划、schema 变化后透明重计划
- **Verification boundary**: 类型化结果与既有 JSON 输出逐字节等价（边界转换处）+ 参数绑定错误矩阵 + CLI/PG 既有面零回归 + 全量分批回归（M19）
- **Diagnostic boundary**: `src/database.rs::execute_sql`/`execute_in_tx` + `src/network/protocol.rs::Response` + `src/pipeline.rs` 转换点与 plan cache 失效面
- **Split signals**: 兼容迁移若迫使破坏性 API 变更（调用者面无法平滑）时停下裁定迁移策略；预编译语句若牵动 plan cache key 语义重构时拆独立设计
- **Related changes**: `openspec/changes/archive/2026-10-21-ms31-typed-api-param-binding/`（2026-10-23 归档）

### MS32：执行器批量接口与查询资源契约 — completed

- **Status**: completed（2026-10-23；change `2026-10-23-ms32-executor-batch-and-query-memory` 四 Iteration 全部 accepted 归档——Iteration 000 批量接口〔T01：`next_batch` 默认方法与逐行同序、`max=0` 不消费；DataScan 攒批未过 10% 时间门，生产接线撤回、逐行路径保留〕、001 哈希 JOIN 左表逐行〔经 replan Cycle；两阶段状态机、左表不整表驻留、匹配桶不整桶克隆〕、002 有界排序〔`SortNode.bounded` 仅两 Limit 包装点按 `Limit(Sort)`/`Limit(Projection(Sort))` 形状赋值；EXPLAIN 文本不变〕、003 查询内存上限〔语句级预算、双通道配置、D5 驻留点计费、三入口 53200 错误面、计数不跨语句〕；全量终态 2144/0/2）
- **Dependencies**: None 硬前置（建议序在 MS30 后——I082 触碰 JOIN 执行面，避免与绑定收口同面并发改动）
- **Outcome**: 查询执行资源模型成立——`next_batch` 批量接口与查询内存预算/超限错误成为执行器公共契约；Hash JOIN 构建侧缓存 + 左侧全量物化 + 匹配集合克隆的内存放大收敛（左侧逐行探测），ORDER BY + LIMIT 走 Top-N
- **Rationale**: `Executor::next()` 逐行 + async-trait 装箱的分配/调度耗时占比未测量（I081 先测量再评估，不预设向量化框架）；Sort 全量收集、Hash JOIN 双侧全量缓存与匹配集合 `.cloned()`（I082 证据：`src/executor/sort.rs::initialize`、`src/executor/join.rs::next` 三阶段）；两项共同定义「执行与资源」一个阶段成果，并与 I020（表示层）/I066（分配器）分层归因、不同时改多层
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS32-T01 | I081：执行器批量异步接口（量化装箱/调度开销 → `next_batch`，热点算子逐步转换、复用现有 Value 与行容器，保留 `dyn Executor` 装配） | improvements I081（2026-10-18 用户接受方向，先测量） |
| MS32-T02 | I082：JOIN 流水处理（左侧逐行探测/减少集合复制）+ Top-N + 查询内存预算与超限错误（外部排序/溢写确有需求才另行设计；INSERT SELECT 同表源物化快照语义保持） | improvements I082（2026-10-18 用户接受方向） |

- **Non-goals**: Arrow/完整向量化框架（I081 明示不预设）；外部排序与分区 JOIN 溢写（I082 确有需求才设计）；Value 表示层改动（I020 留在 improvements）；分配器更换（I066 留在 improvements）
- **Workload**: 中，约 2 个 change（每任务各一；量化结果允许裁剪范围）
- **Stable baseline**: 执行器公共契约含批量接口与内存上限——大表/大值/多匹配键组合的首行等待与内存峰值有明确预算边界
- **Verification boundary**: 量化基线（装箱开销/克隆规模 before/after bench，`--save-baseline` 纪律）+ 内存预算超限拒绝矩阵 + 既有 JOIN/Sort/大值用例零回归 + 全量分批回归（M19）
- **Diagnostic boundary**: `src/executor/executor_trait.rs` + `src/executor/sort.rs::initialize` + `src/executor/join.rs::next` 三阶段 + 热点算子转换清单
- **Split signals**: `next_batch` 若需改变 `dyn Executor` 装配协议（牵动全执行器）时拆独立设计 change；内存预算若牵动跨算子资源管理器时单独收口
- **Related changes**: `openspec/changes/archive/2026-10-23-ms32-executor-batch-and-query-memory/`（2026-10-23 归档）

### MS33：持久化失败与 DROP 边界 — completed

- **Status**: completed（2026-10-24；change `2026-10-23-ms33-drop-persistence-failure` 两 Iteration 全部 accepted 归档——Iteration 000 checkpoint 前崩溃〔T01：D1 把页清零从 `free_page` 移到 `allocate_page` 弹出空闲页时——释放不写盘、分配先写全零页、写零失败页号回退错误返回；`tests/drop_persistence_test.rs` 7 用例：DROP TABLE/INDEX/VIEW 崩溃保留对象与已提交内容且 `check_integrity` error 为 0、同会话再分配只见新行、删列崩溃保留窄表、加密库建表/删表同类；改前基线见证 DROP TABLE 与加密删表丢行、DROP INDEX 重开 abort——清零树页遍历不收敛〕、Iteration 001 受控 I/O 失败〔T01 后半：类型化失败开关 `Database::arm_io_failure`〔`IoFailPoint::{DataWrite, WalFsync}`，下一次同型调用在系统调用前失败一次并自清除；未设置时字节行为不变；主库 FileStorage/WalWriter 接线、`:memory:`/ATTACH/readonly 不波及、无 CLI flag 不入用户文档〕+ 三恢复用例——数据文件写失败〔九步第 2 步之前〕checkpoint 返回错误且重开回 DROP 前状态，WAL fsync 与位点目录失败〔脏页已写完〕DROP 已生效且 check error 为 0；Plan Review 期修正 spec 类属——WAL fsync 为九步第 3 步晚于 flush_all 归「DROP 已生效」类、数据文件 sync 无生产调用点不设开关位、R1 复用窗口界定留 MS35〕；全量终态 2155/0/2；崩溃模型为进程内丢弃句柄，真实断电结论未宣称）
- **Dependencies**: None
- **Outcome**: 已盘点的持久化失败类各有可观察的恢复结果。现有测试已经覆盖的记入范围；确认有缺口的路径才加固
- **Rationale**: I086 仍是未验证的引擎正确性边界。DROP、断电、磁盘满、短写和同步失败不能从进程崩溃记录直接推定
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS33-T01 | 盘点 DROP 与持久化失败的现有覆盖，给每类失败写下可恢复结果；只加固盘点确认的缺口 | improvements I086 |

- **Non-goals**: 空闲页跨重启复用（MS35）、历史库兼容（MS36）、伴生文件加密（I060）、撕裂树运行期根修（I031）
- **Workload**: 中。盘点与加固可在同一阶段完成；若加固要求改变刷盘策略或 WAL 格式，停止并另立里程碑
- **Stable baseline**: 后续取消语义和兼容样本可以依赖这里写明的提交与恢复结果
- **Verification boundary**: 在现有测试中直接观察打开、查询和已提交状态。进程终止、受控 I/O 失败和真实断电的结论分开记录。全量回归沿用 M19
- **Diagnostic boundary**: DROP、checkpoint、WAL 与存储写入失败路径
- **Split signals**: 盘点要求改变刷盘策略或 WAL 格式时拆出，不把 I031 纳入本阶段
- **Related changes**: `openspec/changes/archive/2026-10-23-ms33-drop-persistence-failure/`（2026-10-24 归档）

### MS34：取消、超时与事务收尾 — planned

- **Status**: planned（2026-10-23 用户批准。依赖 MS33 的提交边界，MS33 完成前不转 ready）
- **Dependencies**: MS33
- **Outcome**: 提交前中断、提交已完成但调用方尚未收到、显式事务继续还是回滚，这三种结果有确定约定。超时不代替回滚
- **Rationale**: I090 与 MS33 都触及提交是否完成，但故障域不同：这里是取消和调用方生命周期，不是磁盘失败
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS34-T01 | 约定库入口在丢弃、超时和断开后的事务、锁和句柄结果，并核对执行、事务与后台 I/O 的收尾 | improvements I090 |

- **Non-goals**: 完整分级诊断系统（I096）；不重复已有的 CLI 信号停机行为
- **Workload**: 中
- **Stable baseline**: 调用方可以依据约定判断取消后语句是否提交、锁是否释放、句柄能否继续使用
- **Verification boundary**: 三种结果都能从库入口观察到。全量回归沿用 M19
- **Diagnostic boundary**: 执行入口、事务会话、后台 I/O 的取消与收尾
- **Split signals**: 若最小约定必须变成完整诊断系统，停止并留在 I096
- **Related changes**: None

### MS35：空闲页跨重启复用 — planned

- **Status**: planned（2026-10-23 用户批准。依赖 MS33，避免与 DROP 失败路径同时改空闲页；MS33 完成前不转 ready）
- **Dependencies**: MS33
- **Outcome**: 已释放的页在重新打开后仍可安全复用，并覆盖再次分配与崩溃恢复
- **Rationale**: I087 是存储资源行为：空闲页列表目前只在当次打开有效。它和 MS33 的失败语义不是同一层，但两边都会碰到页释放
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS35-T01 | 使释放页在重新打开后可复用，覆盖重分配和崩溃恢复。不把内存空闲列表原样写成持久格式作为唯一方案 | improvements I087 |

- **Non-goals**: 尾部截断和整库整理；无键行旧版本 GC（I038）
- **Workload**: 中
- **Stable baseline**: 反复打开并删除表之后，已释放空间仍可被后续分配使用，崩溃后不把未完成释放当成可分配页
- **Verification boundary**: 释放、重新打开、再分配和崩溃恢复的结果可直接观察。全量回归沿用 M19
- **Diagnostic boundary**: 页分配、释放和重新打开
- **Split signals**: 尾部截断或整库整理单独评估，不并入本阶段
- **Related changes**: None

### MS36：历史库与 dump 兼容 — planned

- **Status**: planned（2026-10-23 用户批准。依赖 MS33，使兼容样本覆盖当时的恢复行为；MS33 完成前不转 ready）
- **Dependencies**: MS33
- **Outcome**: 写明支持的旧库格式和旧 dump 范围。范围内可以打开、查询、写入和恢复；范围外给出明确拒绝或迁移路径
- **Rationale**: I092 与 MS33 的故障域不同。兼容样本放在恢复边界确定之后，避免样本还在跟随未完成的恢复改动
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS36-T01 | 定义旧格式与旧 dump 的支持范围，并用少量样本断言打开、查询、写入和恢复 | improvements I092 |

- **Non-goals**: CI、预编译发布和 crates.io 发布（I051–I053）；键编码格式变更（I024）；伴生文件加密（I060）
- **Workload**: 小到中
- **Stable baseline**: 后续格式变更可以对照这份支持范围判断是否破坏已有数据
- **Verification boundary**: 样本直接断言打开、查询、写入和恢复结果。版本是产品格式契约。全量回归沿用 M19
- **Diagnostic boundary**: 文件头版本判断与 dump 恢复
- **Split signals**: 若要启用已暂缓的 CI 或发布设施，停止，不在本阶段打开
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

初版后统一路线（2026-09-24 规划、2026-10-01 用户批准重排：功能补齐前置、性能优化殿后独立，均无硬前置，编号与执行序无关；2026-10-19 用户批准底层设计批 MS30–MS32 前置。2026-10-23 性能批 MS22 退出本图，退回 improvements）：

```
MS23 约束面 ──(唯一索引/约束语义)──→ MS24 写面
MS23 ──(索引基建)──→ MS21 DDL/二级索引
MS18 one-shot 性能 ──(checkpoint 策略面)──→ MS26 运行形态 ──(存储后端)──→ MS27 存储演进
MS25 SQL 读面 · MS20 ATTACH 跨库（独立，序自由）
MS30 语义绑定层 ──(绑定结果/参数定型输入)──→ MS31 类型化 API
MS32 执行资源契约（无硬前置，建议序 MS30 后避开 JOIN 同面并发改动）
全部功能带（MS28/MS29/MS20-T02/MS27）──→ 底层设计批（MS30–MS32，2026-10-19 批准）
MS33 持久化失败与 DROP 边界（无硬前置）──→ MS34 取消与事务收尾
MS33 ──→ MS35 空闲页跨重启复用
MS33 ──→ MS36 历史库与 dump 兼容
```

建议执行序：**MS23 ✓ → MS24 ✓ → MS19 ✓ → MS18 ✓ → MS25 ✓ → MS20 一期 ✓ → MS21 ✓ → MS26 ✓ → MS28 ✓ → MS29 ✓ → MS20-T02 ✓ → MS27 ✓ → MS30 ✓ → MS31 ✓ → MS32 ✓ → MS33 ✓ → MS34**，同时 **MS33 ✓ → MS35**、**MS33 ✓ → MS36**（MS35 与 MS36 互不依赖）。截至 2026-10-24，MS33 及之前的执行序各项已完成。MS22 已 superseded，退回 improvements。MS34–MS36 为 planned 里程碑，对 MS33 的依赖已满足。

无环。MS33 已完成；MS34、MS35、MS36 对 MS33 的依赖已满足。已完成历史段随 `ARC-202609242151` 归档。

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
