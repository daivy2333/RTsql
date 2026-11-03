# tasks — 任务与里程碑路线

> 最后更新：2026-10-28（**MS41 completed**：change `2026-10-28-ms41-build-and-compat-baseline` 已归档。Linux 本机质量检查、gnu 冒烟、musl 静态二进制冒烟和一次 M19 分批全量已有结果。macOS 未运行。用户原话：「那就不管那个，直接收尾吧，因为我买不起MacBook」。风险：macOS 构建与运行没有实测。ISS36 已关闭。**MS39 completed**：组合索引与范围访问变更 `2026-10-27-ms39-composite-index-and-range-access` 已归档。规格修改 `sql-secondary-indexes` 的三条 requirement，并新增三条。组合非唯一二级索引与范围路由已交付；升级前的单列整数索引范围仍扫描。ISS36 保持 open。**MS38 completed**：索引键编码与字符串二级索引变更 `2026-10-27-ms38-index-key-encoding-and-string-index` 已归档。规格并入新建域 `index-key-encoding-evaluation`，并修改 `sql-secondary-indexes` 的三条 requirement。采用保留 32 字节定长键；字符串单列二级索引已交付。ISS36 保持 open。**MS40 completed**：Join 变更 `2026-10-26-ms40-join-smj-and-build-side` 已归档，规格并入 `join-executor-selection` 与 `legacy-format-compat`；`query-memory-budget` 未合并。ADD COLUMN 变更此前已归档。ISS39、ISS40、ISS41 已关闭（2026-10-27 fixed：ISS39、ISS40 → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`，ISS41 → `openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side`）。**ADD COLUMN 目录面 change 归档**：`2026-10-26-ms40-add-column-catalog` 三轮 accepted 后归档，规格并入 `ddl-catalog-transactional` 与 `ddl-layout-publication`。**MS40 纳入 ISS39、ISS40**：用户指令把这两条新登记的 ADD COLUMN 目录面缺陷纳入现有 MS40。Join 原范围（I015 的 SMJ 余项、I016 的降范围起步）保留，纳入当时状态为 ready。Join 面与这两条缺陷是两个故障域，后续计划分成可独立验证、可独立排障的 change，仍属同一里程碑，不新编号。**查询能力与缺陷收口批规划**：milestone-planner 增补 MS37–MS40——2026-10-25 用户批准批次选目（3 条能力线 + 全部 10 条 open 缺陷纳入本批 + 2 条语料 riders，4 个里程碑上限）：**MS37 缺陷台账收口**〔completed，2026-10-26，change `2026-10-25-ms37-defect-ledger-closure` 已归档；承载 ISS25–ISS34 全量处置——DDL 提交点与并发三条（ISS25/ISS28/ISS29，各含方向性裁定留各自 change 的需求批准关口）/ 存储有界读两条（ISS26 索引叶页条目长度判据、ISS27 其余页型约 17 处无界读点）/ 写盘失败状态两条（ISS30 驱逐交错、ISS31 失败标记复位）/ dump·restore 两条（ISS32 部分恢复含「原子化或维持既有合同后关闭」裁定、ISS33 非整数 UNIQUE 条件渲染）/ 语料收尾（ISS34 规格标题对齐 + I045 十四处主 spec Purpose 补写 + I064 WAL 提交确认与 checkpoint 边界文档化）〕、**MS38 索引键保序编码评估与字符串二级索引**〔planned ← MS37；I024 评估为第一迭代并内建决策门 + I085 前半〕、**MS39 组合索引与范围访问**〔planned ← MS38；I085 后半〕、**MS40 Join 算法补全与基础选择**〔2026-10-25 批准范围：ready，无硬前置；I015 SMJ 余项 + I016 降范围起步，criterion 时间门沿用 MS32 先例。2026-10-26 扩入 ISS39、ISS40，Join 范围与 ready 保持，见本段开头〕；建议执行序 MS37 → MS38 → MS39 → MS40（MS40 可与 MS38/MS39 换位）；MS38 硬依赖 MS37 的理由：编码变更改变索引条目格式，ISS26/ISS27 的判据与有界读面须先在现格式上收口；I045 与 I064 已随该 change 标为 promoted；I024、I085、I015 的 SMJ 余项与 I016 的降范围起步已标为 promoted 并指向 MS38–MS40，工作尚未开始，未标完成。**MS35 / MS36 与缺陷批次收尾**：MS35 空闲页跨重启复用经 change `2026-10-24-ms35-free-page-reuse-across-restart` 交付归档——两 Iteration 全 accepted：000 重新打开时按持久目录重建空闲页集合（T1 先行 RED / T2 存储层接缝与打开钩子 / T3 活页集合重建实现——目录链 + 数据链 + 溢出链 + 三类索引树 + 内存索引树并集 + 页数上界，任一遍历失败即整次放弃）/ 001 释放的持久化判定（T4 先行 RED / T5 待释放集合 + 释放完成编号 + checkpoint 第 2 步钩子 + 六个释放点的完成标记 / T6 既有同会话复用断言的分界更新 / T7 收尾全量 / T8 规格增量场景与验收边界对齐——含一次 replan Cycle，崩溃点改到「DROP TABLE 之后、任何后续 DDL 成功返回之前」）；全量终态 2183/0/2（14 批 / 135 目标）；行为规格新增 `free-page-lifecycle` 域〔4 Requirement/7 Scenario〕，MODIFIED `drop-table-physical-free` 与 `drop-persistence-boundary`；MS36 历史库与 dump 兼容经 change `2026-10-24-ms36-legacy-format-and-dump-compat` 交付归档——两 Iteration 全 accepted：000 兼容样本与刻画断言（固定旧库文件 / 旧库未决 WAL / 旧 dump 文本样本于 `tests/fixtures/legacy/`，新增 `tests/legacy_compat_test.rs`，零产品代码改动）/ 001 支持范围写成为合同与基线（规格增量定稿 + 中英双语 README 对齐 + 收尾全量）；全量终态 2192/0/2（13 批 / 136 目标）；行为规格新增 `legacy-format-compat` 域〔5 Requirement/14 Scenario〕，并在 `database-file-format-header` 增补「格式版本是产品格式契约」一条；缺陷批次经非里程碑 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 交付归档——八 Iteration（000 索引页归属唯一性 / 001 列改写的数据链一致性与放弃可见性 / 002 释放路径的有界读 / 003 改写窗口的读一致性 / 004 释放可分配判定的耐久性前提 / 005 DML 墓碑的放弃窗口收敛 / 006 索引树读路径的有界读与显式拒绝 / 007 旧格式兼容的证据补齐）全 accepted，承载 ISS22 / ISS23 / ISS24 与 ISS25 的并发读变体（其崩溃-重开变体仍开放）及实施中新发现的缺陷候选；收尾全量终态 2231/0/2（14 批 / 143 目标）；行为规格新增 5 域——`index-page-ownership`〔3/6〕、`ddl-rewrite-consistency`〔3/10〕、`release-path-bounded-read`〔1/4〕、`release-durability-precondition`〔2/7〕、`index-read-bounded-read`〔1/5〕；同批应用既有域 `mvcc-tombstone-visibility` 的 AR2 澄清与 `legacy-format-compat` 证据索引更新（E2 / E4 逐字替换，E7 整条替换）；规格库 75 → 82 域；缺陷台账未动——ISS22 / ISS23 / ISS24 可关闭、**ISS25 保持 open**（关闭与登记由 `openspec-experience-recorder` 按用户指令执行））；2026-10-24（**ISS20 / ISS21 修复批收尾**：非里程碑 change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 交付归档——两 Iteration 全 accepted：000 索引写侧原子化〔T1 写侧暂停点夹具 + 先行 RED（修复前 3/3 稳定命中产品断言）；T2 结构变更、根页 id 发布与反向映射更新合并为同一不可分割写单元（同一 blocking 闭包内完成，语句被取消不留半写状态）+ 同一索引的结构写侧经写侧闸门串行化；T3 取消/回滚后被取消语句写入的**全部**键从索引释放（同键可再次插入、不报重复键），索引结构自洽（后续写语句不 abort、完整性检查无 Error 级发现）〕/ 001 DDL 目录事务化〔T4 先行 RED；T5 `ALTER TABLE DROP COLUMN` 的目录窄化延后到语句提交点之后生效——事务体只做守卫检查、布局计算与数据面改写，目录变更以 `PendingColumnDrop` 交回提交点应用（列行先、计数后、再切活布局）；T6 目录面回归对齐（完成路径 + 既有 MS34 回收用例断言补齐）+ 本 change 收尾全量〕；收尾全量终态 2171/0/2（134 个测试目标、13 批 M19 分批串行）；行为规格新增 index-write-side-atomicity 域〔4 Requirement/5 Scenario〕与 ddl-catalog-transactional 域〔3 Requirement/4 Scenario〕，并把 statement-cancellation-outcome R2 的已知边界改写为「键的释放不由本域保证，由 `index-write-side-atomicity` R3/R4 承载」（原描述的缺陷机制已修复，不再陈述为当前行为）；**本 change 不对应任何 MSxx**——按 ISS20 建议与 MS35 相邻、按 ISS21 建议与 MS36 相邻，结果登记在两段的 Related changes；**MS35、MS36 当时保持 ready 不动**）；**MS34 完成收尾**：取消、超时与事务收尾经单 change `2026-10-24-ms34-cancellation-timeout-transaction` 交付归档——两 Iteration 全 accepted：000 被放弃的隐式事务回收〔T01：库属隐式事务登记表 + 三个库自有同步点回收（下一条隐式写语句之前 / checkpoint 之前 / 自动 checkpoint 判定之前），DML、CTAS、ALTER TABLE DROP COLUMN 三处库属隐式事务登记〕/ 001 调用方可见结果与收尾回归〔T01 后半：提交已完成但未送达视为已提交、显式事务继续还是回滚由调用方决定、超时不代替回滚、取消后后台 I/O 按会话级生命周期收尾；Review 期收窄规格口径——取消触发来源只有丢弃与超时到期，客户端连接在结果送达前断开不是取消触发来源（语句照常执行完毕并提交，只表现为结果未送达）；已知边界：取消若落在索引写入中途，被取消语句的行不可见但键仍可能占用主键索引，键的释放不属本域保证面〕；全量终态 2163/0/2；行为规格新增 statement-cancellation-outcome 域〔6 Requirement/10 Scenario〕；**MS34–MS36 转 ready**：依赖 MS33 已满足——MS33 于 2026-10-24 完成归档（两 Iteration 全 accepted、全量终态 2155/0/2），三段 Scope/Non-goals/Split signals 经现场核对仍与仓库一致、拆分信号未触发，2026-10-24 由 milestone-planner 转 ready；**MS33 完成收尾**：持久化失败与 DROP 边界经单 change `2026-10-23-ms33-drop-persistence-failure` 交付归档——两 Iteration 全 accepted：000 checkpoint 前崩溃〔D1 页清零从释放时移到空闲页弹出时——释放不写盘、分配先写全零页、写零失败页号回退；7 用例覆盖 DROP TABLE/INDEX/VIEW 崩溃保留、同会话再分配、删列、加密建表/删表；改前基线 DROP TABLE/加密删表丢行、DROP INDEX 重开 abort〕/ 001 受控 I/O 失败〔类型化开关 `arm_io_failure`（DataWrite/WalFsync，系统调用前失败一次自清除）+ 三恢复用例——数据文件写失败回 DROP 前状态，WAL fsync 与位点目录失败 DROP 已生效且 check 无 error；Review 期修正 spec 类属：WAL fsync 为九步第 3 步晚于 flush_all、数据文件 sync 无调用点不设开关位、R1 复用窗口界定留 MS35〕；全量终态 2155/0/2；行为规格新增 drop-persistence-boundary 域〔5 Requirement/10 Scenario〕+ wal-recovery-replay-integrity 的 DDL 边界 requirement 修改。引擎正确性批下一项 MS34，MS35/MS36 依赖已满足）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

2026-11-03 用户指令「规划这一批你推荐的条目到ms，刚好三个ms就好」，新增 MS45–MS47，依据 [后续工作优先级分析](../analysis/future-work-priority-and-dependencies.md)（R77）在 MS41–MS44 完成后的下一批过滤结果。MS45 主命令 profiling 不再因索引扫描中止（completed，2026-11-03，change `openspec/changes/archive/2026-11-03-ms45-main-command-profiling/`；承接 ISS42）。MS46 无键行版本链进入 GC（ready，承接 I038）。MS47 类型化批量写入入口的去留（ready，承接 I021）。三者无硬前置，建议序 MS45 → MS46 → MS47。macOS 构建与运行不在本批。I077 余项、I083、I065、I069、I084、I016 的完整代价模型、I031 以及已暂缓的交付与密钥便利层留在改进台账。本次只登记路线，不同步改进条目、缺陷状态或 SNAPSHOT。

2026-10-27 用户批准“架构边界与验证基础”批，新增 MS41–MS44，依据 [后续工作优先级分析](../analysis/future-work-priority-and-dependencies.md)（R77）：MS41 构建与兼容验证基线为 completed（macOS 按 2026-10-28 用户决定不再等待）；MS42 可组合模块边界为 completed（2026-10-28 收尾归档，change `openspec/changes/archive/2026-10-28-ms42-module-boundaries/`）；MS43 异步服务状态协作为 completed（2026-10-28 收尾归档，change `openspec/changes/archive/2026-10-28-ms43-service-lifecycle/`）；MS44 统一诊断为 completed（2026-10-30 收尾归档，change `openspec/changes/archive/2026-10-28-ms44-unified-diagnostics/`）。本批覆盖 ISS36、I073、I093、I074、I095、I012、I096；I093 按平台与 feature 组合分别承接，I095 保留按需触发边界，不把未触发定制变成必做工作。

2026-10-25 批次共四个里程碑（2026-10-25 用户批准「查询能力与缺陷收口批」，4 个里程碑上限、全部 10 条 open 缺陷纳入本批；2026-10-26 用户指令把新登记的 ISS39、ISS40 纳入 MS40，不新编号）：**MS37 缺陷台账收口**（completed，2026-10-26，change `2026-10-25-ms37-defect-ledger-closure`）。**MS40 Join 算法补全与 ADD COLUMN 目录面收口**（completed，2026-10-27。ADD COLUMN 目录面 change `2026-10-26-ms40-add-column-catalog` 与 Join 变更 `2026-10-26-ms40-join-smj-and-build-side` 已归档。ISS39、ISS40、ISS41 已关闭（2026-10-27 fixed：ISS39、ISS40 → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`，ISS41 → `openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side`）。无硬前置）；**MS38 索引键保序编码评估与字符串二级索引**（completed，2026-10-27，change `2026-10-27-ms38-index-key-encoding-and-string-index` 已归档。ISS36 保持 open）。**MS39 组合索引与范围访问**（completed，2026-10-27，change `2026-10-27-ms39-composite-index-and-range-access` 已归档。ISS36 保持 open）。前路线 MS35–MS36 于 2026-10-25 实施完成（2026-10-23 用户批准，承接 I092、I087；2026-10-24 依赖 MS33 已满足后转 ready）。MS22 于 2026-10-23 superseded 退回 improvements；MS30–MS36 为 completed（2026-10-25 归档，段原文见文件末墓碑）。历史总数曾写作 33（22 completed + 6 superseded + 5 planned，6 个历史段在 carrier `ARC-202609242151`），该算术未按本次状态重算已归档段。初版后统一路线，2026-09-24 规划：MS18-MS27；2026-10-01 用户批准重排，milestone-planner 增补 MS28-MS29 承接 I077/I078/I079 与残留任务；2026-10-19 用户批准底层设计批（依据 improvements 同日优先级排序），milestone-planner 增补 MS30-MS32 承接 I091/I080/I088/I089/I081/I082、执行序前置于 MS22；MS23 于 2026-09-25 完成，MS24 于 2026-09-26 完成〔T04 声明期 DEFAULT 类型校验未实施，2026-10-01 重排并入 MS29-T02〕，MS19 于 2026-09-26 完成，MS18 于 2026-09-26 完成，MS25 于 2026-09-28 完成，MS20 一期于 2026-09-29 完成〔T02 二期跨文件 DML 待后续 change〕，MS21/MS26 于 2026-09-30 完成，MS28 于 2026-10-01 完成，MS29 于 2026-10-01 完成，MS20 于 2026-10-02 完成（T02 二期随 change `2026-10-02-ms20-cross-db-dml` 三 Iteration accepted 归档），MS27 于 2026-10-03 完成（change `2026-10-02-ms27-overflow-integrity` 三 Iteration accepted 归档），MS30 于 2026-10-21 完成（change `2026-10-20-ms30-semantic-binding-differential-net` 四 Iteration accepted 归档——单 change 承载两任务为 2026-10-20 用户指令，T03/T04 为裁定增补段），MS31 于 2026-10-23 完成（change `2026-10-21-ms31-typed-api-param-binding` 三 Iteration accepted 归档——单 change 承载 T01+T02 为 2026-10-21 用户裁定），MS32 于 2026-10-23 完成（change `2026-10-23-ms32-executor-batch-and-query-memory` 四 Iteration accepted 归档——000 生产攒批未过时间门撤回、001 经 replan Cycle；单 change 承载 T01+T02 为计划期裁定）），MS33 于 2026-10-24 完成（change `2026-10-23-ms33-drop-persistence-failure` 两 Iteration accepted 归档——Review 期修正 spec 类属），MS34 于 2026-10-24 完成（change `2026-10-24-ms34-cancellation-timeout-transaction` 两 Iteration accepted 归档——Review 期收窄取消触发来源口径：断开不是取消触发来源），MS35 于 2026-10-25 完成（change `2026-10-24-ms35-free-page-reuse-across-restart` 两 Iteration accepted 归档——空闲页跨重启复用；含一次 replan Cycle，规格增量场景的崩溃点对齐 DDL 语句边界有序落盘），MS36 于 2026-10-25 完成（change `2026-10-24-ms36-legacy-format-and-dump-compat` 两 Iteration accepted 归档——历史库与 dump 兼容；旧格式支持范围按目录行布局代际定义）。★初版达成★。

统一执行序（2026-10-01 用户裁定重排——功能补齐前置、性能优化殿后独立成批；沿用「批准执行序与编号无关」既定先例，编号只表规划先后，执行以本序为准）：**MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）→ MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）→ MS20-T02 跨文件 DML 二期 → MS27 存储演进批（超页溢出 + 完整性校验）**。MS22 已于 2026-10-23 退出本序并退回 improvements。同日批准的后续执行序：**MS33 持久化失败与 DROP 边界 → MS34 取消、超时与事务收尾**；**MS33 → MS35 空闲页跨重启复用**；**MS33 → MS36 历史库与 dump 兼容**。MS35 与 MS36 互不依赖。此前各项顺序为建议序非硬依赖（无环、前置均已满足）。编排依据（2026-10-01）：外部评审实证的自动化使用摩擦先行收口——并发硬失败 exit 4 → 可配置等待是多 agent 并发使用的语义前提，也是当时持锁窗口治理的前置；输入通道与声明语义同代码面相邻（lifecycle.rs import + ddl_dml.rs 建表/类型面）；存储格式演进体量最大放功能带末尾；性能殿后、先量化再决定（MS08 纪律）。首批排除域维持：微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058。REPL（I067）、DETACH（I076）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）、WAL fsync 组提交（I049——串行单写者模型下无效）等留 improvements 域未排期。MS22 候选池于 2026-10-23 整体退回该域。2026-10-25 新批执行序：**MS37 缺陷收口 → MS38 索引编码与字符串索引 → MS39 组合索引与范围访问**（索引能力主线，缺陷面先收口）+ **MS40 Join 补全与 ADD COLUMN 目录面两条缺陷**（无硬前置；2026-10-26 起含 ISS39、ISS40；Join 与这两条缺陷在里程碑内分 change；可与 MS38/MS39 换位）。
旧优化项历史分类（各 superseded MS 原范围段）随 `ARC-202609242151` carrier 留档；未排期候选以 D-candidates 与 improvements 台账为准。
## Milestone Roadmap

详细的已完成里程碑段已归档，见文件末墓碑。各里程碑的完成摘要见「路线图结构」段与「依赖关系图」的 ✓ 标记；MS22 已 superseded 退回 improvements，其候选见「D-candidates」段与改进台账。当前路线上尚未执行的里程碑是 MS46、MS47。

### MS45：主命令 profiling 不再因索引扫描中止

- **Status**: completed（2026-11-03。change `openspec/changes/archive/2026-11-03-ms45-main-command-profiling/` 已归档。Iteration 000 与 001 的 Review Result 均为 accepted。设置 `RTSQL_PROFILING` 时，主命令的索引扫描返回查询结果或既有 SQL 错误，进入执行阶段的语句向 stderr 输出旧三段计时表；未设置该变量时主命令不输出该表。收尾全量 15 批，每批第一次退出码 0、failed 为 0）
- **Outcome**: 设置 `RTSQL_PROFILING` 时，主命令执行含索引扫描的语句不再以进程中止结束；调用方得到查询结果或既有 SQL 错误。主命令是否输出旧的三段计时表，在本里程碑需求批准时裁定，裁定后的行为成为稳定结果。未设置该变量时，主命令行为保持。
- **Rationale**: 这是当前唯一开放缺陷。故障域是旧 profiling 开关在主命令上的作用域，与无键行回收、批量写入无关。并进后两项会让进程中止和资源合同或性能决策共用一个排查范围。
- **Dependencies**: None
- **Scope**: 消除该开关下主命令索引扫描路径的进程中止。需求批准时裁定主命令是否输出旧三段计时表，并按裁定交付。未设置该变量的语句、import 路径上已有的三段表、统一诊断门面的观察语义保持。
- **Non-goals**: 替换或扩展 MS44 的统一诊断；新增取消或 profiling 公共 API；改变未设置该变量时的 SQL 结果、错误文本和 SQLSTATE；顺带做批量写入或 GC。
- **Stable result**: 该开关下主命令不再因索引扫描中止。三段表是否出现在主命令上有已批准的行为，后续测量可以依赖这一行为。
- **Related changes**: `openspec/changes/archive/2026-11-03-ms45-main-command-profiling/`
- **Consumes**: ISS42
- **Split signals**: 若三段表的裁定演变成改写统一诊断的输出口径，退回范围审议，不夹进本里程碑。

### MS46：无键行版本链进入 GC

- **Status**: ready
- **Outcome**: 键位不可入索引的行，其旧版本链能被 GC 回收。可见性判断和释放规则与已有 GC 相同。带键行的既有 GC 合同不变。
- **Rationale**: 这条链今天走不到 GC，长更新下旧版本空间不回收。补的是覆盖面。速度、锁和分配器不在同一验收里。
- **Dependencies**: None
- **Scope**: 让 GC 到达无键行的版本链，并按既有可见性判断与释放规则回收。仍可见的版本不回收。
- **Non-goals**: 提高 GC 速度；改变 MVCC 可见性；改索引格式、空闲页生命周期或刷盘策略；节点级锁、分配器、clone 消除。
- **Stable result**: 无键行的旧版本不再因为不在索引里而永远留在表上。后续空间和性能测量可以假定这条链会被 GC 看见。
- **Related changes**: None
- **Consumes**: I038
- **Split signals**: 若回收必须改可见性语义或空闲页合同，退回范围审议。

### MS47：类型化批量写入入口的去留

- **Status**: ready
- **Outcome**: 当前引擎上有逐行插入成本的测量。测量达到本里程碑需求批准时写下的采纳条件，则存在一条类型化批量写入入口，类型检查、约束、索引维护和失败回滚与逐行写入一致。未达到采纳条件，则不新增入口。
- **Rationale**: 导入攒批和 CSV 去掉逐行 SQL 文本都要复用同一条写入口。先把入口的去留定下来，避免两条后续改进各做一套写路径。2026-10-23 退回的性能批不整包恢复。
- **Dependencies**: None。建议序在 MS45、MS46 之后。测量使用已交付的统一诊断和现有基准，不把 MS45 写成硬前置。
- **Scope**: 测量逐行插入在端到端写入中的占比。需求批准时写下采纳条件。达到则交付可复用的类型化批量写入入口；未达到则记录不建入口。
- **Non-goals**: import 攒批提交和持锁窗口（I077 余项）；CSV 有界读取和去掉逐行 SQL 文本（I083）；分配器、clone 消除、节点锁、io_uring、组提交；完整代价模型与 JOIN 重排。
- **Stable result**: 批量写入入口要么存在，且是后续导入批次的唯一复用点；要么明确不存在。入口不存在时，I077 余项和 I083 保持未排期，不得另建写路径。
- **Related changes**: None
- **Consumes**: I021
- **Split signals**: 入口的失败语义若要改成部分成功，先回需求裁定。import 与 CSV 适配即使入口落地，也不并入本里程碑。

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
MS33 持久化失败与 DROP 边界（无硬前置）──→ MS34 取消与事务收尾 ✓
MS33 ──→ MS35 空闲页跨重启复用
MS33 ──→ MS36 历史库与 dump 兼容
MS37 缺陷台账收口（无前置）──→ MS38 索引键编码与字符串索引 ──→ MS39 组合索引与范围访问
MS40 Join 算法补全与 ADD COLUMN 目录面两条缺陷（无硬前置；Join 与 ISS39、ISS40 分 change；建议序 MS37 后）
MS41 构建与兼容验证基线 ✓（macOS 未运行，用户决定不再等待）
MS42 可组合模块边界 ✓ ──→ MS43 异步服务状态协作 ✓ ──→ MS44 统一分级诊断 ✓
MS45 主命令 profiling 不再因索引扫描中止 ✓（无前置）
MS46 无键行版本链进入 GC（无前置）
MS47 类型化批量写入入口的去留（无前置；建议序 MS45、MS46 之后）
```

建议执行序：**MS23 ✓ → MS24 ✓ → MS19 ✓ → MS18 ✓ → MS25 ✓ → MS20 一期 ✓ → MS21 ✓ → MS26 ✓ → MS28 ✓ → MS29 ✓ → MS20-T02 ✓ → MS27 ✓ → MS30 ✓ → MS31 ✓ → MS32 ✓ → MS33 ✓ → MS34 ✓**，同时 **MS33 ✓ → MS35 ✓**、**MS33 ✓ → MS36 ✓**（MS35 与 MS36 互不依赖）。截至 2026-10-26，MS37 及之前的执行序各项已完成。MS22 已 superseded，退回 improvements。MS35、MS36 于 2026-10-25 完成（2026-10-24 依赖 MS33 已满足后转 ready，2026-10-25 实施完成并归档）。MS37 于 2026-10-26 完成（change `2026-10-25-ms37-defect-ledger-closure` 归档）。2026-10-25 新批建议执行序：**MS37 ✓ → MS38 ✓ → MS39 ✓ → MS40 ✓**（MS40 无硬前置，可与 MS38/MS39 换位；2026-10-26 起 MS40 同时承载 ISS39、ISS40，与 Join 分 change。ADD COLUMN 与 Join 两条 change 均已归档，MS40 状态为 completed。ISS39、ISS40、ISS41 已关闭（2026-10-27 fixed：ISS39、ISS40 → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`，ISS41 → `openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side`）。MS38 硬依赖 MS37——编码变更改变索引条目格式，ISS26/ISS27 的页级判据与有界读面须先在现格式上收口。MS38 于 2026-10-27 完成，change `2026-10-27-ms38-index-key-encoding-and-string-index` 已归档。MS39 于 2026-10-27 完成，change `2026-10-27-ms39-composite-index-and-range-access` 已归档。2026-10-28 change `2026-10-28-ms41-build-and-compat-baseline` 已归档：Linux 本机质量检查、gnu 冒烟、musl 静态二进制冒烟和一次 M19 分批全量已有结果。macOS 未运行。同日用户原话：「那就不管那个，直接收尾吧，因为我买不起MacBook」，MS41 记为 completed。风险：macOS 没有实测。ISS36 已关闭）。

无环。MS33–MS41 已完成。MS41 的 macOS 分支没有运行，按 2026-10-28 用户决定不再等待并记为 completed。MS35、MS36 对 MS33 的依赖已满足并已实施完成。MS39 对 MS38 的前置已满足并已实施完成，MS40 无前置。ISS39、ISS40 写入 MS40 的 Consumes，不新增指向 MS38 或 MS39 的依赖，也不给 MS40 增加前置。无环。已完成历史段随 `ARC-202609242151` 归档。

本批建议执行关系（2026-10-27 批准）：MS41 优先取得可通过的本机质量检查基线，目标平台验证可独立穿插；MS42 → MS43 → MS44 为结构与诊断主线。MS42 实施前须确认本机质量检查基线，但不等待 MS41 全部平台完成；未运行的平台分支不能计入 MS41 完成。现有 MS38、MS39 的范围、依赖及状态保持；结构改造建议在索引线形成稳定结果后启动，以减少格式与架构同时变化的返工，此为建议序，不新增硬依赖。每个里程碑可由一个或多个独立计划承载，不预定数量。本次只登记路线，不同步改进、缺陷台账或已有变更状态。

2026-11-03 批：MS45 为 completed（2026-11-03，change `openspec/changes/archive/2026-11-03-ms45-main-command-profiling/`），MS46、MS47 仍为 ready，依赖为 None，无环。建议执行序 MS45 → MS46 → MS47。MS45 先做，是为了旧 profiling 开关在主命令上不再中止进程；MS47 的测量不依赖这条修复也能用统一诊断和现有基准进行。MS46 与另外两项没有代码合同上的先后。每个里程碑可由一个或多个变更承载，不预定数量。I 条目与 ISS42 的状态不在本次改写。

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
<!-- arc: 0a08b14 --> 7 段已归档 (2026-10-25)：MS30 / MS31 / MS32 / MS33 / MS34 / MS35 / MS36 已完成里程碑段（用户指令「把已经做完的ms和issue，I条目等等都归档了吧，直接做就行」；MS22 superseded 保留；MS30–MS33 的完成态原文经 git show 0a08b14:.agents/docs/tasks.md 取回；MS34–MS36 的完成态原文自 0a08b14 之后进入工作区，随本次归档所在提交以 diff 形式可取回——git show <该提交> -- .agents/docs/tasks.md；完成摘要仍见「路线图结构」段与「建议执行序」✓ 标记）

<!-- arc: 08e474c --> 9 段已归档 (2026-10-30)：MS22（superseded，退回 improvements）/ MS37 / MS38 / MS39 / MS40 / MS41 / MS42 / MS43 / MS44（MS37–MS44 于 2026-10-26～2026-10-30 全部 completed，对应 change 已归档；用户指令「把已经做完的、排期的、关闭的都清理归档一下」；段原文经 git show 08e474c:.agents/docs/tasks.md 取回；完成摘要仍见「路线图结构」段与「依赖关系图」的 ✓ 标记）
