# tasks — 任务与里程碑路线

> 状态由 `openspec-docs-maintainer` 按用户指令同步；路线由 `openspec-milestone-planner` 规划与调整，milestone 与 change 数量不绑定。
> 进行中、待办与阻塞由各 `MSxx` 的 Status 与活跃 change 表达，完成叙事只写在对应 `MSxx` 的 Status 一处。

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## Milestone Roadmap

详细的已完成里程碑段已归档，见文件末墓碑。MS22 已 superseded 退回 improvements，候选见 `.agents/memory/improvements.md`。当前路线上的里程碑均已完成。

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

- **Status**: completed（2026-11-10。change `openspec/changes/archive/2026-11-10-ms46-keyless-row-gc/` 已归档。Iteration 000 的 Review Result 为 accepted。`gc_table` 回收无键链上已提交且不是链头的旧版本，链头与未提交版本仍可读；带键行的既有 GC 合同不变。收尾全量按 Iteration Plan 跳过）
- **Outcome**: 键位不可入索引的行，其旧版本链能被 GC 回收。可见性判断和释放规则与已有 GC 相同。带键行的既有 GC 合同不变。
- **Rationale**: 这条链今天走不到 GC，长更新下旧版本空间不回收。补的是覆盖面。速度、锁和分配器不在同一验收里。
- **Dependencies**: None
- **Scope**: 让 GC 到达无键行的版本链，并按既有可见性判断与释放规则回收。仍可见的版本不回收。
- **Non-goals**: 提高 GC 速度；改变 MVCC 可见性；改索引格式、空闲页生命周期或刷盘策略；节点级锁、分配器、clone 消除。
- **Stable result**: 无键行的旧版本不再因为不在索引里而永远留在表上。后续空间和性能测量可以假定这条链会被 GC 看见。
- **Related changes**: `openspec/changes/archive/2026-11-10-ms46-keyless-row-gc/`
- **Consumes**: I038
- **Split signals**: 若回收必须改可见性语义或空闲页合同，退回范围审议。

### MS47：类型化批量写入入口的去留

- **Status**: completed（2026-11-10。change `openspec/changes/archive/2026-11-10-ms47-typed-batch-write/` 已归档。Iteration 000 的 Review Result 为 accepted。`total_ns = 332234286`，`execute_exclusive_ns = 9804119`，`9804119 * 10 < 332234286`，不新增入口。Iteration 001 不展开。收尾全量按 Iteration Plan 跳过）
- **Outcome**: 当前引擎上有逐行插入成本的测量。测量达到本里程碑需求批准时写下的采纳条件，则存在一条类型化批量写入入口，类型检查、约束、索引维护和失败回滚与逐行写入一致。未达到采纳条件，则不新增入口。
- **Rationale**: 导入攒批和 CSV 去掉逐行 SQL 文本都要复用同一条写入口。先把入口的去留定下来，避免两条后续改进各做一套写路径。2026-10-23 退回的性能批不整包恢复。
- **Dependencies**: None。建议序在 MS45、MS46 之后。测量使用已交付的统一诊断和现有基准，不把 MS45 写成硬前置。
- **Scope**: 测量逐行插入在端到端写入中的占比。需求批准时写下采纳条件。达到则交付可复用的类型化批量写入入口；未达到则记录不建入口。
- **Non-goals**: import 攒批提交和持锁窗口（I077 余项）；CSV 有界读取和去掉逐行 SQL 文本（I083）；分配器、clone 消除、节点锁、io_uring、组提交；完整代价模型与 JOIN 重排。
- **Stable result**: 批量写入入口要么存在，且是后续导入批次的唯一复用点；要么明确不存在。入口不存在时，I077 余项和 I083 保持未排期，不得另建写路径。
- **Related changes**: `openspec/changes/archive/2026-11-10-ms47-typed-batch-write/`
- **Consumes**: I021
- **Split signals**: 入口的失败语义若要改成部分成功，先回需求裁定。import 与 CSV 适配即使入口落地，也不并入本里程碑。

### MS48：模块边界与会话通道收口

- **Status**: completed（2026-11-10。change `openspec/changes/archive/2026-11-10-ms48-module-boundary-and-session-channels/` 已归档。Iteration 000 与 001 的 Review Result 均为 accepted。WAL 记录和待提交事务号经无界通道传递；容量、关闭、停止后等待、checkpoint 与 close 的现有结果保持；包仍然只有一个。收尾全量 15 批，每批退出码 0、failed 为 0）
- **Outcome**: 已裁定的模块边界要么形成可独立构建的组合，要么明确记录没有独立构建或裁剪收益因而不拆。会话里尚未替换的生产者与消费者队列改为已有的异步传递。WAL、提交和检查点的职责归属不变。
- **Rationale**: I074 和 I012 是 MS42、MS43 留下的同一条结构线。拆包和通道若分开立项，会在同一批调用边上各改一次。
- **Dependencies**: None
- **Scope**: I074 的 crate 拆分只在有独立构建或替换收益时做。I012 只替换分析点名的剩余消息传递，不统一换掉全部锁。
- **Non-goals**: 没有具体依赖对象时的依赖升级（I093）和按需定制（I095）；诊断上报；SQL 语义与性能改动。
- **Stable result**: 后续工作可以按这次定下的构建单元和会话传递方式继续。
- **Related changes**: `openspec/changes/archive/2026-11-10-ms48-module-boundary-and-session-channels/`
- **Consumes**: I074 的拆分余项、I012 的通道余项
- **Split signals**: 拆 crate 与通道替换若不能用同一次构建和会话测试判断，分成两个 change，不新编号。若拆分必须改公开 SQL 行为，退回范围审议。

### MS49：诊断上报与比较指标

- **Status**: completed（2026-11-14。change `openspec/changes/archive/2026-11-14-ms49-diagnostics-reporting/` 已归档。Iteration 000、001、002 的 Review Result 均为 accepted。五个旧子指标进入当前语句阶段树；比较指标从保留摘要读出首行等待、吞吐、P95，进程内存峰值在 Linux 上读 `ru_maxrss`。诊断仍只观察。收尾全量 15 批，每批退出码 0、failed 为 0）
- **Outcome**: 各模块向已交付的诊断门面上报自身事件与资源统计。比较指标覆盖首行等待、吞吐、较慢请求耗时和内存峰值。诊断仍只观察，不改变 SQL 结果、错误和 SQLSTATE。
- **Rationale**: 这是分析里和模块化衔接的诊断收口。门面已经存在，缺的是上报和可比口径。它和拆 crate 不是同一个故障域。
- **Dependencies**: None。建议序在 MS48 之后。不把拆 crate 写成硬前置。
- **Scope**: I096 尚未交付的两部分。复用现有门面和已有 profiling，不另建计时系统。
- **Non-goals**: 替换 MS44 的门面；把诊断输出当作正确性判据；WAL 组提交或加密。
- **Stable result**: 后续性能测量可以引用这组比较指标和模块上报。
- **Related changes**: `openspec/changes/archive/2026-11-14-ms49-diagnostics-reporting/`
- **Consumes**: I096 的余项
- **Split signals**: 若比较指标必须改 SQL 输出或错误文本，退回范围审议。

### MS50：结果消费与值类型

- **Status**: completed（2026-11-14。结果消费 change `openspec/changes/archive/2026-11-14-ms50-result-consumption/` 已归档，交付 I065 的最小结果接口。类型 change `openspec/changes/archive/2026-11-14-ms50-decimal-and-blob/` 已归档，交付 I069。Iteration 000 的返工 Review Result 为 accepted。Iteration 001 的补齐 Review Result 为 accepted。只跑过一次分批全量。B08 第一次和一次整批复核失败，原因是中止夹具首字节 0x0A 与新的 BLOB 标签相撞。夹具改为 0x0C 后，那条测试通过。B08 里当时没跑到的 10 个目标后来逐个通过）
- **Outcome**: 大结果可以增量消费，并有背压边界。DECIMAL 与 BLOB 可以声明、写入、比较和读回。旧库打开有明确结果。
- **Rationale**: 两项都先定结果和值的接口，避免后面的性能工作反复改这两处。它们不是彼此的实现步骤。
- **Dependencies**: None
- **Scope**: I065 的最小结果接口，以及 I069 的两种类型。两种类型可以分 change。流式只做最小接口。
- **Non-goals**: 代价模型与 JOIN 重排；导入攒批；为流式改诊断口径。
- **Stable result**: 结果如何被消费，以及值类型集合，有后续性能工作可以依赖的边界。
- **Related changes**: `openspec/changes/archive/2026-11-14-ms50-result-consumption/`；`openspec/changes/archive/2026-11-14-ms50-decimal-and-blob/`
- **Consumes**: I065 的最小结果接口已由结果消费变更交付；I069 已由类型变更交付
- **Split signals**: 流式接口与新类型若不能用同一组查询验收，分成两个 change，不新编号。新类型若要改页格式代际，先写明兼容，不夹进流式 change。

### MS51：查询代价与 JOIN 重排

- **Status**: completed（2026-11-17。change `openspec/changes/archive/2026-11-17-ms51-query-cost-and-join-reorder/` 已归档。Iteration 000、001、002 的 Review Result 均为 accepted。会话内 `ANALYZE` 保存基表行数和列不同值；合格查询按估计更换左深叶序，结果多重集合与书写顺序一致，估算不足时保持书写叶序。收尾全量 16 批，每批退出码 0，合计 2504 passed、0 failed、2 ignored）
- **Outcome**: 有可用的统计与估算。JOIN 重排要么按估算生效且结果与重排前一致，要么记录估算不足以支持重排并保持现有选择。
- **Rationale**: 索引访问路径已经交付。完整代价模型与已做完的构建侧选择不是同一项。
- **Dependencies**: None
- **Scope**: I016 的余项。不重开已收尾的构建侧评估。
- **Non-goals**: 节点锁、I/O 后端、新类型、流式结果。
- **Stable result**: 后续查询改动可以知道 JOIN 顺序是估算决定的，还是仍由现有规则决定。
- **Related changes**: `openspec/changes/archive/2026-11-17-ms51-query-cost-and-join-reorder/`
- **Consumes**: I016 的完整代价模型余项已由该变更交付
- **Split signals**: 若重排会改变结果语义而不只是计划选择，退回范围审议。

### MS52：撕裂树与跨进程访问的去留

- **Status**: completed（2026-11-17。change `openspec/changes/archive/2026-11-17-ms52-torn-tree-and-cross-process/` 已归档。Iteration 000 与 001 的 Review Result 均为 accepted。不改刷盘和驱逐，不改「有重放就重建索引」。跨进程一写多读不成立，保持独占锁和共享锁。收尾全量 16 批，每批退出码 0，failed 为 0。产品代码相对 `0019bf1` 没有差异）
- **Outcome**: 撕裂树的恢复代价有测量，并有刷盘、驱逐或维持现状的结论。跨进程一写多读有场景是否成立的结论。成立则快照、缓存失效和检查点协议被写明；不成立则保持现有文件锁。
- **Rationale**: 两项都是先测量再决定是否改持久化或共享合同。真正的大改不放在结论之前。
- **Dependencies**: None
- **Scope**: I031、I084 的评估和去留。结论为维持现状则该项结束。结论要求改刷盘、驱逐或文件锁时，改动仍属本里程碑，并限在该结论点名的合同内。
- **Non-goals**: 脏页批量写的实现、io_uring、索引页格式、节点锁。
- **Stable result**: 后续页 I/O 可以依赖这里的刷盘与共享结论，不必再把撕裂树或跨进程访问当作未决前提。
- **Related changes**: `openspec/changes/archive/2026-11-17-ms52-torn-tree-and-cross-process/`
- **Consumes**: I031、I084 已由该变更消耗。结论是维持刷盘、驱逐、恢复门控和文件锁
- **Split signals**: 若两项都要从评估进入实现，分成两个 change，不新编号。若改动超出结论点名的合同，退回范围审议。

### MS53：复制、分配与行锁

- **Status**: completed（2026-11-19。三份归档见 Related changes。嵌套循环组合行不再克隆，SQL 结果保持；分配器测量后保留系统默认分配器；行锁测量后保留现有 RowLockTable）
- **Outcome**: 复制、分配和行锁争用有测量。只有被测量证明值得改的那一处进入实现，且 SQL 结果不变。其余明确保留原实现。
- **Rationale**: 三者都在执行路径的内存和争用上。先减少不必要复制，再看分配器，便于把收益归因到一处。
- **Dependencies**: None。建议序在 MS49 之后，以便使用比较指标。已有诊断门面足够测量，新指标不是硬前置。
- **Scope**: I020、I066、I050。行锁先测争用，再决定是否换成并发结构。
- **Non-goals**: 节点锁、页 I/O、io_uring、导入攒批。
- **Stable result**: 这三处要么已改且行为不变，要么明确保留原实现。后续 I/O 优化不把未证实的复制热点算进自己的收益。
- **Related changes**: `openspec/changes/archive/2026-11-19-ms53-avoidable-copy/`；`openspec/changes/archive/2026-11-19-ms53-system-allocator/`；`openspec/changes/archive/2026-11-19-ms53-row-lock/`
- **Consumes**: I020、I066、I050
- **Split signals**: 三处不能用同一次测量判断时，分成多个 change，不新编号。任一改动若改变可见性或锁等待的对外错误，退回范围审议。

### MS54：脏页批量写与 I/O 后端

- **Status**: completed（2026-11-24。change `openspec/changes/archive/2026-11-24-ms54-dirty-page-batch-io/` 已归档。Iteration 000、001、002 与 003 的 Review Result 均为 accepted。连续 64 个脏页的五次测量后裁定值得交付，`flush_all` 与 `flush_all_except` 把连续页号合成一次 `pwritev`，失败重试、释放提升和恢复门控保持。io_uring 五次比较后裁定值得采用，增加默认关闭的 `io-uring` feature，默认 features 仍是 `cli`、`network`、`crypto`。收尾全量在 Iteration 003，默认 features：15 个文件批和库测试批退出码都是 0，failed 都是 0）
- **Outcome**: 连续脏页写入有测量。批量写值得做则交付，并保留失败重试、释放耐久性和恢复约定。其后才比较 io_uring。不值得或平台不适合则保留现有后端。
- **Rationale**: I028 需要 I026 先给出批量 I/O 基线。两者都在页写入路径上。撕裂树的去留会决定刷盘和驱逐，所以排在 MS52 之后。
- **Dependencies**: MS52
- **Scope**: I026、I028。先完成批量写的去留，再比较后端。
- **Non-goals**: 索引页格式、节点锁、分配器、组提交评估。
- **Stable result**: 页写入要么走已交付的批量路径，要么仍是逐页写。I/O 后端有采用或保留现状的结论。
- **Related changes**: `openspec/changes/archive/2026-11-24-ms54-dirty-page-batch-io/`
- **Consumes**: I026、I028
- **Split signals**: io_uring 若要替换默认跨平台后端，退回范围审议。批量写若要改恢复不变量，退回范围审议。

### MS55：索引页空间与锁

- **Status**: completed（2026-11-24。两份归档见 Related changes。索引页可移除字节为 0，内部节点槽保持 36 字节分隔条目，数据槽保持 22 字节版本头紧贴 tuple。写争用没有独立于现有 write_gate 的加锁门槛。每个 IndexManager 仍是一把 `Arc<Mutex<()>>`。不加节点锁）
- **Outcome**: 索引页的空间和锁粒度有测量。瘦内部节点或 Tag 合并只有在空间收益盖过格式兼容成本时才改格式。节点锁只有在写争用被测量证实后才加，并保留写单元原子性、根发布、取消和回滚。未过门槛的保持原格式和现有锁。
- **Rationale**: 三者都作用在已经稳定的定长键布局上。格式和锁要分开验收，但不值得各占一个里程碑。
- **Dependencies**: None。建议在 MS52 之后，避免格式与驱逐策略同时变。不硬依赖 MS54。
- **Scope**: I025、I029、I030。不沿用变长键假设。
- **Non-goals**: 脏页批量写、io_uring、代价模型。
- **Stable result**: 索引页格式和锁粒度有可依赖的结论。
- **Related changes**: `openspec/changes/archive/2026-11-24-ms55-index-page-space/`；`openspec/changes/archive/2026-11-24-ms55-index-node-lock/`
- **Consumes**: I025、I029、I030
- **Split signals**: 格式变更与节点锁若不能用同一次恢复和并发测试判断，分成两个 change，不新编号。格式变更若使旧库打不开且没有已写明的兼容路径，退回范围审议。

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
MS46 无键行版本链进入 GC ✓（无前置）
MS47 类型化批量写入入口的去留 ✓（无前置；建议序 MS45、MS46 之后）
MS48 模块边界与会话通道收口 ✓（无前置）
MS49 诊断上报与比较指标 ✓（无前置；建议序 MS48 后）
MS50 结果消费与值类型 ✓（无前置）
MS51 查询代价与 JOIN 重排 ✓（无前置）
MS52 撕裂树与跨进程访问的去留 ✓（无前置）
MS53 复制、分配与行锁 ✓（无前置；建议序 MS49 后）
MS52 ──→ MS54 脏页批量写与 I/O 后端 ✓
MS55 索引页空间与锁 ✓（无前置；建议序 MS52 后）
```

## 与 OpenSpec Changes 同步

- 每个 MSxx 内的 MSxx-Txx 实施时通过 `openspec/changes/<date>-<t-tag>/` 创建 change
- 完成的 change 通过 `openspec archive` 归档
- 归档的 change carrier 保持不可变
- 新发现的问题写 `.agents/memory/improvements.md` (Ixx) — **注意**：Ixx 编号待重新审视，旧 Ixx 多数已重新归位到 MSxx-Txx
- 完整迁移的旧版 entry 记录在 `.agents/legacy/2026-08-25-openspec-init-migration/COVERAGE.md`

历史完成记录（2026-05-24～2026-09-24，36 行）已全部归档至 carrier
`openspec/changes/archive/2026-09-24-ARC-202609242151/`（`archive/tasks-completed-roadmap.md`）；
当前项目状态见 SNAPSHOT「仓库现场」与各归档 change carrier。

<!-- arc: ARC-202609242151 --> 6 块已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609242151/proposal.md

<!-- arc: a61891f --> 11 段已归档 (2026-10-20)：MS18-MS21、MS23-MS29 已完成里程碑段（用户指令「做完了的不管时间长短也都进行归档」；MS22/MS30-MS32 planned 保留；各段 Status/Outcome/Scope 原文经 git show a61891f:.agents/docs/tasks.md 取回）

<!-- arc: 4190580 --> 「最近完成」段已归档 (2026-10-20)：2026-09-25～2026-10-15 十九批完成记录（用户指令「最近完成也可以清理，没必要保留」；各批原文经 git show 4190580:.agents/docs/tasks.md 取回，权威记录在各归档 change carrier；完成状态摘要见各 `MSxx` 的 Status 与「依赖关系图」的 ✓ 标记）
<!-- arc: 0a08b14 --> 7 段已归档 (2026-10-25)：MS30 / MS31 / MS32 / MS33 / MS34 / MS35 / MS36 已完成里程碑段（用户指令「把已经做完的ms和issue，I条目等等都归档了吧，直接做就行」；MS22 superseded 保留；MS30–MS33 的完成态原文经 git show 0a08b14:.agents/docs/tasks.md 取回；MS34–MS36 的完成态原文自 0a08b14 之后进入工作区，随本次归档所在提交以 diff 形式可取回——git show <该提交> -- .agents/docs/tasks.md；完成摘要仍见各 `MSxx` 的 Status 与「依赖关系图」的 ✓ 标记）

<!-- arc: 08e474c --> 9 段已归档 (2026-10-30)：MS22（superseded，退回 improvements）/ MS37 / MS38 / MS39 / MS40 / MS41 / MS42 / MS43 / MS44（MS37–MS44 于 2026-10-26～2026-10-30 全部 completed，对应 change 已归档；用户指令「把已经做完的、排期的、关闭的都清理归档一下」；段原文经 git show 08e474c:.agents/docs/tasks.md 取回；完成摘要仍见各 `MSxx` 的 Status 与「依赖关系图」的 ✓ 标记）

<!-- arc: 6ad789a --> 5 段已归档（2026-11-18，按新规范移除文件级状态段与路线叙述）：顶部「最后更新」摘要、「路线图结构」段、「D-candidates」段、「长期方向」段、依赖关系图后的执行序叙述段，以及「进行中」「已承诺待办」「阻塞」三段，全文见 `git show 6ad789a:.agents/docs/tasks.md`。完成状态以各 `MSxx` 的 Status 与归档 change carrier 为准；未承诺方向以 `.agents/memory/improvements.md` 为权威。
