# tasks — 任务与里程碑路线

> 最后更新：2026-10-25（**MS35 / MS36 与缺陷批次收尾**：MS35 空闲页跨重启复用经 change `2026-10-24-ms35-free-page-reuse-across-restart` 交付归档——两 Iteration 全 accepted：000 重新打开时按持久目录重建空闲页集合（T1 先行 RED / T2 存储层接缝与打开钩子 / T3 活页集合重建实现——目录链 + 数据链 + 溢出链 + 三类索引树 + 内存索引树并集 + 页数上界，任一遍历失败即整次放弃）/ 001 释放的持久化判定（T4 先行 RED / T5 待释放集合 + 释放完成编号 + checkpoint 第 2 步钩子 + 六个释放点的完成标记 / T6 既有同会话复用断言的分界更新 / T7 收尾全量 / T8 规格增量场景与验收边界对齐——含一次 replan Cycle，崩溃点改到「DROP TABLE 之后、任何后续 DDL 成功返回之前」）；全量终态 2183/0/2（14 批 / 135 目标）；行为规格新增 `free-page-lifecycle` 域〔4 Requirement/7 Scenario〕，MODIFIED `drop-table-physical-free` 与 `drop-persistence-boundary`；MS36 历史库与 dump 兼容经 change `2026-10-24-ms36-legacy-format-and-dump-compat` 交付归档——两 Iteration 全 accepted：000 兼容样本与刻画断言（固定旧库文件 / 旧库未决 WAL / 旧 dump 文本样本于 `tests/fixtures/legacy/`，新增 `tests/legacy_compat_test.rs`，零产品代码改动）/ 001 支持范围写成为合同与基线（规格增量定稿 + 中英双语 README 对齐 + 收尾全量）；全量终态 2192/0/2（13 批 / 136 目标）；行为规格新增 `legacy-format-compat` 域〔5 Requirement/14 Scenario〕，并在 `database-file-format-header` 增补「格式版本是产品格式契约」一条；缺陷批次经非里程碑 change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 交付归档——八 Iteration（000 索引页归属唯一性 / 001 列改写的数据链一致性与放弃可见性 / 002 释放路径的有界读 / 003 改写窗口的读一致性 / 004 释放可分配判定的耐久性前提 / 005 DML 墓碑的放弃窗口收敛 / 006 索引树读路径的有界读与显式拒绝 / 007 旧格式兼容的证据补齐）全 accepted，承载 ISS22 / ISS23 / ISS24 与 ISS25 的并发读变体（其崩溃-重开变体仍开放）及实施中新发现的缺陷候选；收尾全量终态 2231/0/2（14 批 / 143 目标）；行为规格新增 5 域——`index-page-ownership`〔3/6〕、`ddl-rewrite-consistency`〔3/10〕、`release-path-bounded-read`〔1/4〕、`release-durability-precondition`〔2/7〕、`index-read-bounded-read`〔1/5〕；同批应用既有域 `mvcc-tombstone-visibility` 的 AR2 澄清与 `legacy-format-compat` 证据索引更新（E2 / E4 逐字替换，E7 整条替换）；规格库 75 → 82 域；缺陷台账未动——ISS22 / ISS23 / ISS24 可关闭、**ISS25 保持 open**（关闭与登记由 `openspec-experience-recorder` 按用户指令执行））；2026-10-24（**ISS20 / ISS21 修复批收尾**：非里程碑 change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 交付归档——两 Iteration 全 accepted：000 索引写侧原子化〔T1 写侧暂停点夹具 + 先行 RED（修复前 3/3 稳定命中产品断言）；T2 结构变更、根页 id 发布与反向映射更新合并为同一不可分割写单元（同一 blocking 闭包内完成，语句被取消不留半写状态）+ 同一索引的结构写侧经写侧闸门串行化；T3 取消/回滚后被取消语句写入的**全部**键从索引释放（同键可再次插入、不报重复键），索引结构自洽（后续写语句不 abort、完整性检查无 Error 级发现）〕/ 001 DDL 目录事务化〔T4 先行 RED；T5 `ALTER TABLE DROP COLUMN` 的目录窄化延后到语句提交点之后生效——事务体只做守卫检查、布局计算与数据面改写，目录变更以 `PendingColumnDrop` 交回提交点应用（列行先、计数后、再切活布局）；T6 目录面回归对齐（完成路径 + 既有 MS34 回收用例断言补齐）+ 本 change 收尾全量〕；收尾全量终态 2171/0/2（134 个测试目标、13 批 M19 分批串行）；行为规格新增 index-write-side-atomicity 域〔4 Requirement/5 Scenario〕与 ddl-catalog-transactional 域〔3 Requirement/4 Scenario〕，并把 statement-cancellation-outcome R2 的已知边界改写为「键的释放不由本域保证，由 `index-write-side-atomicity` R3/R4 承载」（原描述的缺陷机制已修复，不再陈述为当前行为）；**本 change 不对应任何 MSxx**——按 ISS20 建议与 MS35 相邻、按 ISS21 建议与 MS36 相邻，结果登记在两段的 Related changes；**MS35、MS36 当时保持 ready 不动**）；**MS34 完成收尾**：取消、超时与事务收尾经单 change `2026-10-24-ms34-cancellation-timeout-transaction` 交付归档——两 Iteration 全 accepted：000 被放弃的隐式事务回收〔T01：库属隐式事务登记表 + 三个库自有同步点回收（下一条隐式写语句之前 / checkpoint 之前 / 自动 checkpoint 判定之前），DML、CTAS、ALTER TABLE DROP COLUMN 三处库属隐式事务登记〕/ 001 调用方可见结果与收尾回归〔T01 后半：提交已完成但未送达视为已提交、显式事务继续还是回滚由调用方决定、超时不代替回滚、取消后后台 I/O 按会话级生命周期收尾；Review 期收窄规格口径——取消触发来源只有丢弃与超时到期，客户端连接在结果送达前断开不是取消触发来源（语句照常执行完毕并提交，只表现为结果未送达）；已知边界：取消若落在索引写入中途，被取消语句的行不可见但键仍可能占用主键索引，键的释放不属本域保证面〕；全量终态 2163/0/2；行为规格新增 statement-cancellation-outcome 域〔6 Requirement/10 Scenario〕；**MS34–MS36 转 ready**：依赖 MS33 已满足——MS33 于 2026-10-24 完成归档（两 Iteration 全 accepted、全量终态 2155/0/2），三段 Scope/Non-goals/Split signals 经现场核对仍与仓库一致、拆分信号未触发，2026-10-24 由 milestone-planner 转 ready；**MS33 完成收尾**：持久化失败与 DROP 边界经单 change `2026-10-23-ms33-drop-persistence-failure` 交付归档——两 Iteration 全 accepted：000 checkpoint 前崩溃〔D1 页清零从释放时移到空闲页弹出时——释放不写盘、分配先写全零页、写零失败页号回退；7 用例覆盖 DROP TABLE/INDEX/VIEW 崩溃保留、同会话再分配、删列、加密建表/删表；改前基线 DROP TABLE/加密删表丢行、DROP INDEX 重开 abort〕/ 001 受控 I/O 失败〔类型化开关 `arm_io_failure`（DataWrite/WalFsync，系统调用前失败一次自清除）+ 三恢复用例——数据文件写失败回 DROP 前状态，WAL fsync 与位点目录失败 DROP 已生效且 check 无 error；Review 期修正 spec 类属：WAL fsync 为九步第 3 步晚于 flush_all、数据文件 sync 无调用点不设开关位、R1 复用窗口界定留 MS35〕；全量终态 2155/0/2；行为规格新增 drop-persistence-boundary 域〔5 Requirement/10 Scenario〕+ wal-recovery-replay-integrity 的 DDL 边界 requirement 修改。引擎正确性批下一项 MS34，MS35/MS36 依赖已满足）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

当前路线无 ready 里程碑：MS35–MS36 于 2026-10-25 实施完成（2026-10-23 用户批准，承接 I092、I087；2026-10-24 依赖 MS33 已满足后转 ready）。MS22 于 2026-10-23 superseded 退回 improvements；MS30–MS36 为 completed（2026-10-25 归档，段原文见文件末墓碑）。历史总数曾写作 33（22 completed + 6 superseded + 5 planned，6 个历史段在 carrier `ARC-202609242151`），该算术未按本次状态重算已归档段。初版后统一路线，2026-09-24 规划：MS18-MS27；2026-10-01 用户批准重排，milestone-planner 增补 MS28-MS29 承接 I077/I078/I079 与残留任务；2026-10-19 用户批准底层设计批（依据 improvements 同日优先级排序），milestone-planner 增补 MS30-MS32 承接 I091/I080/I088/I089/I081/I082、执行序前置于 MS22；MS23 于 2026-09-25 完成，MS24 于 2026-09-26 完成〔T04 声明期 DEFAULT 类型校验未实施，2026-10-01 重排并入 MS29-T02〕，MS19 于 2026-09-26 完成，MS18 于 2026-09-26 完成，MS25 于 2026-09-28 完成，MS20 一期于 2026-09-29 完成〔T02 二期跨文件 DML 待后续 change〕，MS21/MS26 于 2026-09-30 完成，MS28 于 2026-10-01 完成，MS29 于 2026-10-01 完成，MS20 于 2026-10-02 完成（T02 二期随 change `2026-10-02-ms20-cross-db-dml` 三 Iteration accepted 归档），MS27 于 2026-10-03 完成（change `2026-10-02-ms27-overflow-integrity` 三 Iteration accepted 归档），MS30 于 2026-10-21 完成（change `2026-10-20-ms30-semantic-binding-differential-net` 四 Iteration accepted 归档——单 change 承载两任务为 2026-10-20 用户指令，T03/T04 为裁定增补段），MS31 于 2026-10-23 完成（change `2026-10-21-ms31-typed-api-param-binding` 三 Iteration accepted 归档——单 change 承载 T01+T02 为 2026-10-21 用户裁定），MS32 于 2026-10-23 完成（change `2026-10-23-ms32-executor-batch-and-query-memory` 四 Iteration accepted 归档——000 生产攒批未过时间门撤回、001 经 replan Cycle；单 change 承载 T01+T02 为计划期裁定）），MS33 于 2026-10-24 完成（change `2026-10-23-ms33-drop-persistence-failure` 两 Iteration accepted 归档——Review 期修正 spec 类属），MS34 于 2026-10-24 完成（change `2026-10-24-ms34-cancellation-timeout-transaction` 两 Iteration accepted 归档——Review 期收窄取消触发来源口径：断开不是取消触发来源），MS35 于 2026-10-25 完成（change `2026-10-24-ms35-free-page-reuse-across-restart` 两 Iteration accepted 归档——空闲页跨重启复用；含一次 replan Cycle，规格增量场景的崩溃点对齐 DDL 语句边界有序落盘），MS36 于 2026-10-25 完成（change `2026-10-24-ms36-legacy-format-and-dump-compat` 两 Iteration accepted 归档——历史库与 dump 兼容；旧格式支持范围按目录行布局代际定义）。★初版达成★。

统一执行序（2026-10-01 用户裁定重排——功能补齐前置、性能优化殿后独立成批；沿用「批准执行序与编号无关」既定先例，编号只表规划先后，执行以本序为准）：**MS28 CLI 并发与生命周期小收口（busy-timeout 串行化 + 显式 checkpoint）→ MS29 输入面与写面声明收口（stdin/免建表导入 + DEFAULT 声明期校验）→ MS20-T02 跨文件 DML 二期 → MS27 存储演进批（超页溢出 + 完整性校验）**。MS22 已于 2026-10-23 退出本序并退回 improvements。同日批准的后续执行序：**MS33 持久化失败与 DROP 边界 → MS34 取消、超时与事务收尾**；**MS33 → MS35 空闲页跨重启复用**；**MS33 → MS36 历史库与 dump 兼容**。MS35 与 MS36 互不依赖。此前各项顺序为建议序非硬依赖（无环、前置均已满足）。编排依据（2026-10-01）：外部评审实证的自动化使用摩擦先行收口——并发硬失败 exit 4 → 可配置等待是多 agent 并发使用的语义前提，也是当时持锁窗口治理的前置；输入通道与声明语义同代码面相邻（lifecycle.rs import + ddl_dml.rs 建表/类型面）；存储格式演进体量最大放功能带末尾；性能殿后、先量化再决定（MS08 纪律）。首批排除域维持：微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058。REPL（I067）、DETACH（I076）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）、WAL fsync 组提交（I049——串行单写者模型下无效）等留 improvements 域未排期。MS22 候选池于 2026-10-23 整体退回该域。
旧优化项历史分类（各 superseded MS 原范围段）随 `ARC-202609242151` carrier 留档；未排期候选以 D-candidates 与 improvements 台账为准。
## Milestone Roadmap

### MS22：实测驱动性能优化（第一批） — superseded（2026-10-23 退回 improvements）

- **Status**: superseded（2026-10-23 用户指令：本里程碑退出路线，退回 `.agents/memory/improvements.md`。不由其他里程碑替代。原候选不再是已承诺工作。）
- **Dependencies**: None
- **Outcome**: 未形成阶段成果。原候选恢复为未承诺改进方向：I020、I021、I024、I026、I038、I049、I050、I066，以及 I077 的持锁窗口余项。I031 仍留在 improvements，不挂本里程碑。
- **Rationale**: 性能批在范围量化定稿前保持 planned。用户于 2026-10-23 要求将其退回改进台账；「先量化再决定」留在对应条目，不再占用路线。
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
MS33 持久化失败与 DROP 边界（无硬前置）──→ MS34 取消与事务收尾 ✓
MS33 ──→ MS35 空闲页跨重启复用
MS33 ──→ MS36 历史库与 dump 兼容
```

建议执行序：**MS23 ✓ → MS24 ✓ → MS19 ✓ → MS18 ✓ → MS25 ✓ → MS20 一期 ✓ → MS21 ✓ → MS26 ✓ → MS28 ✓ → MS29 ✓ → MS20-T02 ✓ → MS27 ✓ → MS30 ✓ → MS31 ✓ → MS32 ✓ → MS33 ✓ → MS34 ✓**，同时 **MS33 ✓ → MS35 ✓**、**MS33 ✓ → MS36 ✓**（MS35 与 MS36 互不依赖）。截至 2026-10-25，MS36 及之前的执行序各项已完成。MS22 已 superseded，退回 improvements。MS35、MS36 于 2026-10-25 完成（2026-10-24 依赖 MS33 已满足后转 ready，2026-10-25 实施完成并归档）。

无环。MS33–MS36 已完成；MS35、MS36 对 MS33 的依赖已满足并已实施完成。已完成历史段随 `ARC-202609242151` 归档。

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
