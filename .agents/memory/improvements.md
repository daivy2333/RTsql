## Purpose

记录用户提出或接受、尚未承诺实施的改进方向，以及已转入路线条目的来源。按主题索引检索，正文按原编号递增保留。当前项目描述见 [SNAPSHOT](../docs/SNAPSHOT.md)。

## Requirements

### Requirement: 改进项可评估

改进项 SHALL 包含分类、问题、证据、影响、建议和状态。

#### Scenario: 发现未排期问题

- **WHEN** 已有证据表明存在改进机会但尚未批准实施
- **THEN** 使用递增 I 编号记录

#### Scenario: 批准实施

- **WHEN** 用户批准实施改进项
- **THEN** 创建 OpenSpec change 并把原条目标记 promoted

---

## 阅读与状态说明

- 本次补充来源：2026-10-18 用户接受数据库局限与技术选型讨论，并要求「记录到I文档里面，然后重新整理一下I文档里面的内容」。本次只登记方向，不批准实施、不新增路线或任务。
- `open` 表示已登记、待调查；`planned` 和 `long-term` 保留既有候选含义，不代表执行就绪；`promoted` 的承诺范围与状态以任务台账、对应变更为准。旧阶段名和旧任务编号只保留来源含义。
- 新条目区分静态读码事实、已有运行记录与未验证判断。历史性能数字只代表采集时的环境，不能作为当前收益承诺；本次未运行新测试或压测。
- 同一方向优先补充原条目，各条目的关联说明界定范围，不重复安排已转入路线的工作。既有暂缓决定继续有效。

## 主题索引

| 主题 | 条目 |
|---|---|
| SQL 绑定、优化器与索引能力 | I015（NLJ 已实施，SMJ 未排期）、I016、I069、I080、I085、I089 |
| 批量执行、物化与导入 | I020、I021、I065、I081、I082、I083 |
| 页空间、索引格式与 I/O | I024、I025、I026、I028、I029、I030、I031、I038、I087 |
| 持久化、安全与取消 | I049、I054、I055、I056、I057、I060、I064、I086、I090 |
| 库 API、并发与模块边界 | I012、I050、I066、I074、I084、I088、I093、I094 |
| CLI 与长会话 | I067、I071、I076 |
| 测试、兼容性与交付 | I045、I051、I052、I053、I058、I073、I091、I092 |
| 已转入路线的来源 | I077（锁等待已交付，持锁窗口治理仍为性能候选） |

## 优先级排序（2026-10-19 用户裁定）

来源：2026-10-19 用户指令——全部候选统一排序（含未解锁项），「可能或者明确设计底层的改动优化、功能补充提升到前面」。本排序是候选工作的优先级视图：不改变任何条目的状态与承诺，不修改 tasks 路线（统一执行序中 MS22 性能批仍为下一步）；是否按本排序重排路线，待用户另行裁定。梯队内按依赖与立项把握排序，未解锁项标注解锁条件。覆盖 50 项工作：48 条非 promoted 条目 + I015 的 SMJ 余项 + I077 的 ② 余项（I078/I079 已交付不计）。

2026-10-19 后续裁定：第一批纳入路线——MS30（T1 I091/T2 I080）、MS31（T1 I088/T2 I089）、MS32（T1 I081/T2 I082）经用户批准创建，建议执行序前置于 MS22；上述 6 条已标 promoted，其余条目维持本表状态与解锁条件，后续批次另议。

### 梯队一 A：语义、优化器与 API 底层

| 序 | 条目 | 内容 | 状态 |
|---|---|---|---|
| 1 | I080 | SQL 名字与类型统一绑定阶段（AST→绑定→逻辑→物理） | promoted——MS30-T02 |
| 2 | I088 | 类型化库结果，核心/协议解耦 | promoted——MS31-T01 |
| 3 | I089 | 库参数绑定与预编译语句（消费 I080/I088） | promoted——MS31-T02 |
| 4 | I016 | 代价模型 + Join 重排（降范围起步：选较小 Hash 构建侧） | planned（P4），可进行 |
| 5 | I012 | 消息传递重构（Arc<Mutex> → mpsc/oneshot/Notify/watch） | planned（P3），可进行 |
| 6 | I074 | crate 化路线 B——首步现 crate 内 feature 边界评估，拆分以「出现清晰接缝」为准 | 方向已批未排期（long-term） |
| 7 | I081 | 执行器批量异步接口（先测量装箱开销） | promoted——MS32-T01 |
| 8 | I082 | JOIN 流水处理、Top-N 与查询内存预算 | promoted——MS32-T02 |
| 9 | I090 | 查询取消、超时与事务状态契约 | open，未承诺 |
| 10 | I065 | 扫描结果流式化（Response 物化架构重构） | long-term，未排期 |

### 梯队一 B：存储、并发与表示底层

| 序 | 条目 | 内容 | 状态 |
|---|---|---|---|
| 11 | I024 | Key 保序变长编码评估——I025/I029 前置、I085 字符串索引地基 | MS22 候选，可进行（量化先行） |
| 12 | I031 | 撕裂树运行期根修（BufferPool 驱逐策略重设计） | 锁定：撕裂/恢复代价量化后另评 |
| 13 | I029 | 瘦内部节点（依赖 I024） | long-term，未解锁 |
| 14 | I030 | 合并 Tag byte（序列化格式变更） | long-term，未解锁 |
| 15 | I025 | B+Tree 节点级锁（latch coupling，依赖 I024） | planned（P5），可进行 |
| 16 | I028 | io_uring 后端（在 I026 之后评估） | planned（P5），可进行 |
| 17 | I087 | 空闲页跨重启复用（持久空闲页管理或打开时安全重建） | open，未承诺 |
| 18 | I038 | GC 无键行版本链盲区（数据页链全扫模式） | MS22 候选，可进行 |
| 19 | I020 | Value 表示/clone 消除（profile 先行，与 I082/I066 分层归因） | MS22 候选，可进行 |
| 20 | I026 | 脏页 writev 批量写回（对照 M18 恢复不变量） | MS22 候选，可进行 |
| 21 | I084 | 跨进程一写多读能力评估 | open，未承诺 |
| 22 | I086 | DROP 与持久化失败边界验证加固（盘点先行，可能引出底层加固） | open，未承诺 |

配套：I091（SQL 差分与性质测试）不是底层改动本身，但 I080 的绑定重构直接消费它（rusqlite 开发依赖已就位）——已随 MS30-T01 纳入路线，先于 I080 实施。

### 梯队二：功能补充

| 序 | 条目 | 内容 | 状态 |
|---|---|---|---|
| 23 | I085 | 字符串与组合索引、范围访问（与 I024 保序编码协同） | open，未承诺 |
| 24 | I069 | DECIMAL/BLOB 列类型（格式深水区，参照 MS13 先例） | long-term，未排期 |
| 25 | I060 | WAL 与 checkpoint 伴生文件加密（安全功能 + 载荷格式设计） | 锁定：初版非目标 |
| 26 | I015 余项 | SMJ 多 Join 算法补全 | promoted 余项，无里程碑归属 |
| 27 | I067 | REPL 交互模式与元命令 | 锁定：用户裁定后续可选 |
| 28 | I071 | PG Extended Query + serve 复活 | 锁定：串行模型达天花板后触发 |
| 29 | I076 | DETACH 注销附加库 | 锁定：REPL/长会话落地才重评 |
| 30 | I054 | TTL 密钥缓存 | 锁定：裁剪退还 |
| 31 | I055 | key set/remove/status 子命令 | 锁定：裁剪退还 |
| 32 | I056 | --password-file 密钥通道 | 锁定：裁剪退还 |
| 33 | I052 | GitHub Releases 预编译矩阵 | 锁定：暂缓保留 |
| 34 | I053 | crates.io 发布 | 锁定：暂缓保留 |
| 35 | I051 | CI workflow（重评从定向检查起步） | 锁定：暂缓保留 |
| 36 | I058 | man 页生成 | 锁定：暂缓保留 |

### 梯队三：性能执行与测量

| 序 | 条目 | 内容 | 状态 |
|---|---|---|---|
| 37 | I021 | INSERT 批量执行（类型化批量写入口，I083/I077-② 前置） | MS22 候选，可进行 |
| 38 | I077 余项② | import 攒批提交/持锁窗口治理（已有实测 ≈4.1ms/行） | MS22 池，可进行 |
| 39 | I066 | 分配器评估（profile 同时给 I020 定序） | MS22 候选，可进行 |
| 40 | I083 | CSV 导入内存与逐行 SQL 往返（等 I021 写入口） | open，未承诺 |
| 41 | I050 | RowLockTable DashMap 化（mini-bench 前置） | MS22 候选，可进行 |
| 42 | I049 | 现有 WAL 组提交收益评估 | MS22 池名义项，避坑注记已注定剔除 |
| 43 | I057 | 加密性能正式 bench 基线 | 锁定：裁剪退还 |
| 44 | I094 | CLI runtime 复评 | open，休眠（目标变化才重启） |

### 梯队四：文档、兼容与交付支撑

| 序 | 条目 | 内容 | 状态 |
|---|---|---|---|
| 45 | I045 | 14 个主 spec Purpose 占位补写 | planned，可进行 |
| 46 | I064 | WAL 提交确认与 checkpoint 边界文档化 | planned，可进行 |
| 47 | I092 | 历史库与 dump 升级兼容性验证 | open，未承诺 |
| 48 | I093 | 依赖兼容升级与 feature 组合验证 | open，未承诺 |
| 49 | I073 | x86_64-musl 静态与 macOS 构建验证 | planned，未排期 |

## 改进条目（按编号）

## I012: M42 消息传递重构

- **分类**: 架构 / 可维护性
- **问题**: 多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**: WAL→mpsc，提交→oneshot，Checkpoint→Notify，BufferPool→watch
- **依赖**: M32（I011）
- **状态**: planned（P3）
- **Legacy**: O012

## I015: M25 多 Join 算法

- **分类**: 功能 / 查询优化
- **问题**: 原问题为只有 Hash Join；现已有 NLJ 与结构性启发式选择，尚未排期的是 SMJ，不能按「缺少 NLJ」再次立项。
- **方案**: NLJ + SMJ + 启发式选择
- **依赖**: 无
- **状态**: promoted（NLJ 部分已于 MS09-T02 实施——`NestedLoopJoin` 执行器 + 计划期启发式（纯等值 Hash 保持 / 非等值·混合·字面量腿经 NLJ，用户裁定一并解锁），spec `join-executor-selection`，change 归档 `openspec/changes/archive/2026-09-13-ms09-engine-mvcc-closeout/`；SMJ 维持未排期）
- **Legacy**: O015

## I016: M26 代价模型 + Join 重排

- **分类**: 功能 / 优化器
- **问题**: 固定 join 顺序，无 cardinality/selectivity
- **方案**: `TableStatistics` + `CostEstimator` + DP/贪心重排
- **依赖**: M25（I015）
- **状态**: planned（P4）
- **Legacy**: O016
- **补充建议（2026-10-18）**: 先评估行数、不同值数量与选择率统计能否支持小范围收益，例如选择较小的 Hash JOIN 构建侧，再决定是否引入完整代价模型和 JOIN 重排；不以更换 SQL 解析器替代规划器优化。

## I020: M37 clone 消除 Arc/Cow

- **分类**: 性能 / 分配
- **问题**: `Value::clone()` 在聚合/排序/JOIN 中反复调用
- **方案**: `Value::Text` 内部 `Arc<str>` + `Cow<'_, str>` 延迟分配
- **依赖**: M20（已完成）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 P4）
- **Legacy**: O020
- **补充依据与边界（2026-10-18）**: `src/executor/join.rs` 的匹配集合当前通过 `.cloned()` 获取；先测量并减少可避免的整组复制，再评估 Arc/Cow 等表示变化。JOIN 流水处理归 I082，分配器选择归 I066，避免同时改变多层表示后无法归因。

## I021: M39 INSERT 批量执行

- **分类**: 性能 / 写入
- **问题**: 多值 INSERT 逐行执行
- **方案**: `bulk_insert(keys)` + `append_batch(records)`
- **依赖**: M20（已完成）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；先 bench 逐行路径占比再决定；原 MS08-T09 排期撤销，2026-09-14 MS08 剥离退还）
- **Legacy**: O021
- **补充建议（2026-10-18）**: 复用既有类型、约束、索引和回滚路径构建类型化批量写入口；import 的批次事务与持锁窗口候选沿用 I077，立项前明确逐条提交、整批回滚及部分成功的选择。I083 只负责 CSV 输入与 SQL 文本往返成本，不重复登记批次事务。

## I024: M23 Key 保序变长编码评估（原 Varint 方向）

- **分类**: 性能 / 存储
- **问题**: `src/storage/page_format/key.rs` 固定保存、序列化并比较 32 字节 Key；短键有空间开销，具体占比需按实际编码和页布局测量。
- **方案**: 比较保序变长编码、前缀压缩与现有布局；编码必须保持逻辑比较、字节比较、相等性和恢复语义一致，不能直接假定普通 Varint 保序，也不预定每键改成堆分配 Vec。
- **预期**: 历史「索引空间约缩减 70%」仅为待验证估算；以索引页数、缓存命中和读写吞吐实测决定，不能作为已证明收益。
- **依赖**: 无
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 MS08-T05 排期撤销，2026-09-14 MS08 剥离退还）
- **Legacy**: O024, D02 successor

## I025: M33 B+Tree 节点级锁

- **分类**: 并发 / 锁
- **问题**: 每次操作 lock 整棵树
- **方案**: Semaphore 限流 + latch coupling（crabbing protocol）
- **依赖**: M23（I024）
- **状态**: planned（P5）
- **Legacy**: O025

## I026: M35 脏页 writev 批量写回

- **分类**: 性能 / Checkpoint
- **问题**: 当前页写入经 `FileStorage::write_page` 单页 `spawn_blocking` + `write_all_at`，`BufferPool::flush_all` 仍逐页调用；同步由独立 `sync()` 执行，原「每页单独 fsync」描述不作为当前事实。
- **方案**: 连续页合并写 + `writev()` 向量化
- **预期**: 历史「Checkpoint 5–10x」为未验证估算；需分别测量任务提交、写系统调用与同步耗时。批量刷盘设计须对照项目模型 M18 的恢复不变量。
- **依赖**: M31（已完成）+ M48（I013）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 MS08-T03 排期撤销，2026-09-14 MS08 剥离退还）
- **Legacy**: O026, D12 下游

## I028: M45 io_uring 批量提交

- **分类**: 性能 / I/O
- **问题**: 当前 `src/storage/file_storage.rs` 使用位置读写并通过 `spawn_blocking` 执行页 I/O；Rust 异步接口不意味着文件系统调用由完成式 I/O 后端执行。
- **方案**: 先评估 I026 批量读写、预取和请求粒度，再比较 Linux io_uring 后端；具体库、内核要求与缓冲区所有权在立项时核对，不预先锁定历史 `tokio-uring` 方案。
- **预期**: 历史「I/O 延迟降低 30–50%」为未验证估算；以实际负载决定，不替换现有跨平台后端作为默认前提。
- **依赖**: Linux 与选定后端所需能力；原 Linux 5.1+ 仅为历史方向，具体版本下限待核对。
- **状态**: planned（P5）
- **Legacy**: O028, K36

## I029: M46 瘦内部节点

- **分类**: 性能 / 存储
- **问题**: B+Tree 内部节点只存 separator keys，不存完整 Key
- **难度**: 高，需重构 B+Tree 分裂/合并逻辑
- **依赖**: M23（I024）
- **状态**: long-term
- **Legacy**: O029

## I030: M47 合并 Tag byte

- **分类**: 性能 / 序列化
- **问题**: Slot Tag byte 合并进 VersionHeader，省 1 byte/slot
- **难度**: 低，但影响序列化格式兼容性
- **状态**: long-term
- **Legacy**: O030

## I031: B-Tree 撕裂树运行期根修（结构感知刷盘/驱逐改造）

- **分类**: 正确性 / 存储引擎
- **问题**: 页驱逐按 LRU 而非树拓扑刷盘——checkpoint 后的运行期修改使磁盘 B-Tree 含洞（父页指向未刷盘子页）与孤儿页（裸读实测 `scan_all` 184/10000 + `InvalidPageType` 洞页）。恢复侧已由 `redo_count > 0` 时索引去信任 + 重放后重建消解（MS10-T02 R7/R8），但运行期检查点间的磁盘树仍处于撕裂状态；MS12 整库加密若引入页级 transform 将放大对磁盘树一致性的依赖
- **候选方案**: 结构感知刷盘（子树后序）/ checkpoint 树快照 / no-steal 驱逐——均为 BufferPool 驱逐策略重设计，MS10-T02 design D10 拒绝并入
- **量化支撑**: D10 恢复重建代价实测 40k 行→8.86s、160k 行→40.7s（100 页池随机访存主导；撕裂树运行期根修可同时压缩该恢复代价）
- **状态**: planned（未排期——MS22 Non-goal，撕裂增长与恢复代价量化后另行评估；原 MS08-T07 排期撤销，2026-09-14 MS08 剥离退还）

## I038: GC 对无键行版本链不可达（gc_table scan_all 盲区）

- **分类**: 资源 / GC 覆盖面
- **问题**: `TableMeta::gc_table` 经 `index_manager.scan_all()` 枚举版本链，键位不可键控的行（MS10-T05 001-rework 起落库不入索引）不在索引中——其旧版本链永不被 GC 清理
- **证据**: MS10-T05 Iteration 001 001-rework Plan Context Risks 预判 + 实施后语义成立（2026-09-09，归档 change 同上）；`gc_table` 为可选维护路径（M10）
- **影响**: 含无键行的表长期频繁 UPDATE 场景下旧版本空间不回收；无正确性影响
- **方案**: GC 增加数据页链全扫模式（不经索引）或无键链登记结构；需评估成本后独立 change
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 MS08-T08 排期撤销，2026-09-14 MS08 剥离退还）

## I045: 主 specs Purpose 占位与 TBD/TODO 残留（validate --specs 持续 WARNING）

- **分类**: 文档质量 / OpenSpec 语料库
- **问题**: 多个主 spec 的 `## Purpose` 仍为 `openspec archive` 自动写入的占位句或含 TBD/TODO 标记——`openspec validate --all --strict` 持续 WARNING（passed/failed 不受影响）。2026-09-29 MS25 收尾实测现存 **14 个**：cli-noninteractive-shell、database-file-format-header、database-file-lock、dml-transaction-lifecycle、drop-table-physical-free、ms07-rest-tx-checkpoint-pushdown、pipeline-stage-decomposition、plancache-key-normalization、planner-module-decomposition、schema-persistence、storage-io-optimization、wal-recovery-frame-parsing、wal-recovery-replay-integrity、wal-writer-handle-reuse。**新 spec 不再新增占位**：MS25 归档生成的 9 个新域（sql-outer-join、select-distinct、sql-set-operations、sql-common-table-expressions、explain-plan-output、sql-aggregate-extensions、order-by-alias-sorting、join-output-shape-resolution、aggregate-output-shape-conformance）已在归档当轮补写真实 Purpose，全量 validate 由 26 passed/23 failed 回到 35 passed/14 failed
- **证据**: MS11-T03 Iteration 001 Act Remaining Issue #3 + Plan Review 复跑 validate 输出（2026-09-11，归档 change 同上）；MS25 change `2026-09-26-ms25-sql-read-surface` 收尾归档当轮 `openspec validate --all --strict` 逐域复验（2026-09-29）
- **影响**: 语料库能力入口可读性下降；validate 输出噪声持续
- **方案**: 逐 spec 补写真实 Purpose（一句能力定位 + 来源 change 引用，格式对齐 `no-from-select`/`sql-scalar-functions` 先例）；纯文档工作，可一次性小 change 或随下一次 docs 收尾顺带——归档当轮补写可避免新增
- **状态**: planned

## I049: 现有 WAL 组提交收益评估

- **分类**: 性能 / WAL
- **问题**: 历史条目按「缺少组提交」登记，但当前已有 `WALBuffer::append_commit_and_wait`、后台 `do_flush` 与 `WalWriter::write_batch`；`TransactionManager::commit` 已等待持久化通知，不能重复建设。
- **证据**: 2026-10-18 静态核对 `src/wal/buffer.rs`、`src/wal/writer.rs`、`src/transaction/manager.rs`；本次未测量多事务合并率。
- **方案**: 只在实际存在同一数据库实例内并发提交、且 fsync 是瓶颈时评估现有合并率、等待与吞吐；串行 one-shot 场景先考虑 I021/I077 的批次事务。
- **前置**: 保留先量化再决定的纪律；与任务台账中串行单写者模型下排除组提交优化的说明一致。
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；2026-09-14 MS08 剥离退还登记）

## I050: RowLockTable DashMap 化

- **分类**: 性能 / 并发
- **问题**: 行锁表为非并发友好结构（原 MS03 原范围项，曾排期 MS08-T04 未实施）
- **方案**: RowLockTable 迁移 DashMap
- **前置**: 先做 mini-bench 决定是否值得做（原 MS08 实测纪律保留）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；2026-09-14 MS08 剥离退还登记）

## I051: CI workflow（构建/测试自动化流水线）

- **分类**: 分发 / CI
- **问题**: 无 CI workflow——构建、测试、静态检查靠本地手工执行（原 MS14-T01 组成项，2026-09-23 用户裁定「CI 没必要」裁剪）
- **方案**: GitHub Actions workflow（build + test + clippy/fmt）；按需求重启时规划
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选，用户裁定暂不做）
- **补充建议（2026-10-18）**: 用户接受本轮建议登记；重评时可从定向检查与项目模型 M19 规定的串行分批回归开始，不以一次性全量并行执行为前提。本次没有撤销此前暂缓决定或启动 CI。

## I052: GitHub Releases 预编译矩阵

- **分类**: 分发 / 发布
- **问题**: 无预编译产物分发——x86_64-linux-gnu / musl 静态 / aarch64 / macOS 矩阵（原 MS14-T01 组成项，2026-09-23 用户裁定「Release 也没必要」裁剪）
- **方案**: Releases + cargo-zigbuild 或 runner 原生矩阵 + 各平台 smoke（`rtsql --version` + 基础 CRUD）；macOS 矩阵受阻可独立收口（原 MS14 split signal 随条目保留）
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选，用户裁定暂不做）
- **补充边界（2026-10-18）**: 本轮重新提出交付能力改进，仍先确认真实分发对象和平台需求；不自动启用预编译矩阵，原暂缓决定保留。兼容性验证另见 I092。

## I053: crates.io 发布（cargo install 可用）

- **分类**: 分发 / 发布
- **问题**: 未发布 crates.io——`cargo install rtsql` 不可用（原 MS14-T01 组成项，2026-09-23 用户裁定「暂时不发布」）
- **方案**: crates.io 元数据 + 发布流程；发布不可逆性与名额届时确认
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选，用户裁定暂不发布）
- **补充边界（2026-10-18）**: 本轮只登记发布能力作为后续方向，不授权发布或占用包名；原暂不发布决定保留。

## I054: TTL 密钥缓存（sudo 式密钥管理层）

- **分类**: 安全便利层 / 密钥管理
- **问题**: 派生密钥每次打开需重输（原 MS12-T02 组成项：TTL 缓存 `~/.rtsql/keys/` 0600 + 过期重问，2026-09-23 用户裁定初版最小加密不含便利层）
- **方案**: 派生密钥（非明文）落盘 TTL 缓存 + 过期重问；依赖 MS17-T01 最小加密先行
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选，初版最小集外）

## I055: `rtsql key set/remove/status` 密钥管理子命令

- **分类**: 安全便利层 / 密钥管理 CLI
- **问题**: 无密钥管理子命令（原 MS12-T02 组成项，2026-09-23 裁剪）
- **方案**: key 三子命令（set/remove/status）；依赖 MS17-T01 与 I054 的密钥面定型
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选）

## I056: `--password-file <path>` 密钥通道

- **分类**: 安全便利层 / 密钥通道
- **问题**: 初版仅 `--key` 与 `RTSQL_KEY` 两通道（原 MS12-T02 三通道之一被裁，2026-09-23）；脚本场景可暂以 `RTSQL_KEY` 环境变量兜底
- **方案**: 补 `--password-file <path>` 通道；依赖 MS17-T01
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选）

## I057: 加密性能正式 bench 基线

- **分类**: 性能 / 加密（MS08「先量化再决定」纪律随条目保留）
- **问题**: 加密后打开延迟与吞吐影响仅 Act Response 顺带实测记录（MS17-T01），无正式 bench 基线设施（原 MS12-T03 组成项，2026-09-23 裁剪）
- **方案**: 加密开关两态对比 bench（open 延迟 / CRUD 吞吐，`--save-baseline` 纪律）；依赖 MS17-T01 落地后实施
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选，先量化再决定）

## I058: man 页生成（clap_mangen）

- **分类**: 分发 / 文档
- **问题**: 无 man 页（原 MS14-T01 组成项，2026-09-23 裁剪——v0.1 文档职责由双语 README 承担）
- **方案**: clap_mangen 从 Command 定义构建期生成 man 页；与 I052 发布形态一并规划为宜
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选）

## I060: WAL 与 checkpoint 伴生文件加密

- **分类**: 安全 / 存储
- **问题**: MS17-T01 只加密主数据库文件；`.wal` 仍含可重放记录，`.checkpoint` 仍公开位点与事务水位。加密数据库运行期间，伴生文件可能泄露表/行内容或恢复元数据
- **方案**: 独立 change 评估 WAL 帧与 checkpoint 载荷的加密/认证格式，明确密钥派生、nonce/tag、格式协商、旧明文库兼容、错误密钥拒绝与恢复失败边界；先确认威胁模型和兼容要求，再决定是否实施
- **状态**: planned（2026-09-24 MS17 初版收尾登记；用户裁定为初版非目标，未排期）
- **补充建议（2026-10-18）**: 优先完善现有 AES-GCM/Argon2id 的覆盖范围，无依据时不更换算法；区分 WAL 行内容机密性与 checkpoint 恢复元数据保护。密钥轮换可单独评估，与 I054 的落盘 TTL 缓存不是同一需求；本次不批准密钥缓存或轮换实现。

## I064: WAL 提交确认与 checkpoint 边界文档化

- **分类**: 文档 / 持久化语义
- **问题**: WAL 写入批次追加后执行 `sync_all`，事务提交等待持久化通知；多个并发提交可共享批次，不能把「一条语句一次 fsync」作为所有运行形态的固定计数。历史 INSERT 约 52x 对比采用双方默认日志模式，缺少充分的持久化配置语境。
- **证据**: 代码面 `src/wal/writer.rs:72-79/188-211`；2026-09-24 对比测量（README 对比板块）
- **影响**: 无行为影响；文档缺失使性能声明缺乏耐久性语境、易受质疑
- **方案**: 说明追加、提交确认与 checkpoint 各自承诺，性能对比区分逐条/批量提交、日志模式和同步配置；仅有 `sync_all` 调用或进程 SIGKILL 测试时，不宣称已验证完整断电安全。故障注入与 DROP 边界验证归 I086。
- **状态**: planned（2026-09-24 登记随带）

## I065: 扫描结果流式化/分页（Response 物化架构，远期）

- **分类**: 性能 / 执行器与响应架构（long-term）
- **问题**: 全表扫描 1k 行 297µs vs SQLite 98µs（剔除 SQLite 侧 prepare 不对称后真实差距更大）——火山执行器逐行 MVCC 判定后将结果整体物化 `Vec<Vec<Value>>` 进 Response，大结果集的内存与时延随行数线性放大
- **证据**: 2026-09-24 SQLite 对比测量（README 对比板块、R26 runbook）；Response 物化架构见 SNAPSHOT pipeline 描述
- **影响**: 大表导出/分析吞吐受限；当前嵌入式单机与 agent 分析负载规模下不构成实际瓶颈
- **方案**: 远期评估流式响应（分页/chunk 协议）——牵动 `network::protocol::Response`、CLI 渲染与 plan cache 交互面，需独立设计与 change；先量化真实负载中全扫占比再决定
- **状态**: planned（long-term，2026-09-24 登记未排期）
- **补充依据与关联（2026-10-18）**: `src/pipeline.rs::execute_executor` 当前收集完整 JSON 行集；I088 负责原生类型结果与列元数据，本条负责增量消费及背压，I082 负责算子内部物化。三者不能互相替代；历史扫描差距尚未证明由物化单独造成。

## I066: 分配器评估与切换（jemalloc/mimalloc）

- **分类**: 性能 / 分配器
- **问题**: 当前使用系统默认分配器（glibc malloc，SNAPSHOT 技术栈无自定义分配器依赖）——执行器热路径大量 String/Vec 小分配（扫描物化 `Vec<Vec<Value>>`、JOIN/聚合 clone、sqlparser/plan 构造）的吞吐与 RSS 受分配器行为影响；原 tasks 长期方向 K37，2026-09-24 用户指令转入台账
- **证据**: tasks 长期方向 K37（原有记录）；2026-09-24 SQLite 对比测量（RSS 16.7MiB、扫描吞吐差距——分配器因素为推断，未单独 profile）
- **影响**: 分配密集路径的吞吐与常驻内存；无正确性影响
- **方案**: 先 profile 分配热点（MS08「先量化再决定」纪律），再评估 jemalloc/mimalloc 以 feature-gated 依赖引入（crate 化后可作为微内核拓扑的可选件 feature，呼应 R25）；与 I020（clone 消除）、I065（流式化减少物化）互补——先减分配次数还是先换分配器，以 profile 数据定序
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项）

## I067: REPL 交互模式与元命令

- **分类**: 功能 / CLI 交互形态
- **问题**: 无 REPL 的交互式读取循环（readline/多行输入/历史）与元命令（`.tables`/`.schema`/`.mode`/`.import`/`.dump`/`.quit` 等）；import/restore 已有非交互 stdin 通道，不能再按「无 stdin 读取」描述。生命周期子命令已覆盖 list/schema/dump 等多数场景，交互式探索仍缺位。
- **证据**: R18 主题 3 差距清单两行（REPL「中：新 cli/ 模块；执行后端复用 lib（不经网络）」、元命令「小-中：REPL 层实现；.dump 需语句反序列化」）+ 主题 7 用户决策记录（2026-09-05：非交互优先，REPL 不是核心可作后续可选）
- **影响**: 交互式使用/教学场景不可用；agent/脚本场景不受影响（非交互面已完整）
- **方案**: 新 `src/cli/` REPL 模块（readline/多行/历史），执行后端复用 lib；元命令逐个独立验收（`.open`/`.databases` 见 R18 主题 6 设计空间表）
- **状态**: planned（用户裁定 2026-09-05 降为后续可选，未排期）

## I069: DECIMAL/BLOB 列类型

- **分类**: 功能 / 类型系统
- **问题**: 列类型 6 类（Int/String/Float/Bool + MS13 的 Date/Timestamp），无 DECIMAL/BLOB——精确小数以 Float 近似、二进制数据无承载类型
- **证据**: R18 主题 3 差距清单「DATE/TIMESTAMP/DECIMAL/BLOB | 大：改序列化格式，深水区应靠后」；前两类已由 MS13-T01 落地，DECIMAL/BLOB 未做
- **影响**: 金额类精确计算与二进制存储场景不可用；常规分析负载不受影响
- **方案**: tuple TAG/catalog COL_TAG 扩展的格式变更深水区，参照 MS13 日期类型先例独立规划
- **状态**: planned（long-term，未排期）

## I071: PG Extended Query 与 serve 子命令复活

- **分类**: 功能 / 网络协议
- **问题**: PG 层仅 Simple Protocol，无 Extended Query（prepared statement）；`psql` 及依赖协议扩展的生态工具不可用；server 保留为库能力，无 `rtsql serve` 入口
- **证据**: R18 主题 3 差距清单「PG Extended Query | 对内置 CLI 形态优先级下降（REPL 不需要）；对 psql 生态工具仍有价值」+ 主题 4 规划输入 3（2026-09-05 用户形态决策：server 代码停止投入，将来可选 `rtsql serve` 复活；原 MS09-T03 移除）
- **影响**: 仅影响 psql 生态消费者；非交互 CLI 形态与嵌入式库用法不受影响
- **方案**: 届时随 `rtsql serve` 子命令复活一并评估；协议扩展对 server 库能力独立成立。2026-10-01 定位注记（随 I077 登记）：serve 复活定位为 CLI 串行化并发模型（I077）达吞吐天花板后的扩容路径（FIFO 公平 / psql 生态 / 会话连续性需求触发），与串行模型非对立方案
- **状态**: planned（用户裁定降级未排期，2026-09-05/06）

## I073: 主流平台构建验证（x86_64-linux-musl 静态与 macOS）

- **分类**: 分发 / 平台验证
- **问题**: RISC-V 64 musl 静态交叉构建已验证（`build-riscv64-musl.sh`，change `2026-09-24-riscv64-musl-build-artifacts`），但 x86_64-linux-musl 全静态与 macOS（`FileExt` 理论可用）均未实测，macOS 下测试套件未运行过；安装脚本宣称目标 Linux/macOS
- **证据**: R18 主题 8 事实（musl 全静态可行——「本机未装 musl target，未实测」）+ 未确认项 8（musl 全静态与 macOS 构建未实测）
- **影响**: macOS 支持无验证记录；musl 静态是容器/scratch 部署前提
- **方案**: 装对应 rustup target 实测 cargo build + 测试套件 + CLI 冒烟，结果回写 README/分发文档；与 I052（预编译矩阵）相关但独立——本条是平台可编译性验证，I052 是发布设施
- **状态**: planned（未排期）

## I074: workspace crate 化路线 B（微内核数据库形态）

- **分类**: 架构 / 模块化（long-term 方向）
- **问题**: 单 crate 的核心、协议和依赖边界仍耦合；当前 Cargo.toml 未区分可选依赖，`Database` 仍返回 `network::protocol::Response`。两条依赖环、11 模块和 99 处 `pub(crate)` 是 R25 在 2026-09-24 的历史采集结果，不能当作本轮新计数；单 crate 本身并不阻止 feature 化。
- **证据**: R25 分析（2026-09-24 读码：F1 依赖图谱 / F2 依赖环 / F3 词表熔接点 Value/PhysicalPlan≈20 变体/ColumnType / F4 现成接缝 AsyncStorage/REGISTRY / F6 Response 错层 / F7 加密 feature-ready / F8 无 CI）+ 四刀迁移顺序草图（词表下沉→存储域→语言域→组装层+可选件）；2026-09-24 用户裁定初版后启动路线 B
- **影响**: 纯结构重构，无行为收益；断环可能重塑执行器装配协议，返工成本以「周」计（R25 边界段）
- **方案**: 按 R25 四刀顺序，每刀独立验收全量 GREEN→GREEN、磁盘格式/行为零变化硬锚；启动前置条件包含 I051（CI）落地或等价的本地 feature 组合门纪律（F8：组合无人跑则缺陷无人发现）；`PhysicalPlan` 词表化 vs trait 化为第三刀单点架构决策需专项设计；matklad 告诫 crate 墙有真实成本——每刀以「边界值得墙」准入，两刀后停留也是合法终态；与 I059（多文件会话抽象）、I066（可选件 feature 化）顺风
- **状态**: planned（2026-09-24 用户裁定初版后启动，未排期）
- **补充建议（2026-10-18）**: 可先在现有 crate 中评估 net/crypto/CLI 的可选依赖和 feature 边界，只有出现清晰公共接缝、独立构建或依赖治理收益时再拆 crate；原四刀为长期候选设计，不作为完成小范围改进的强制前置。I088 负责类型化公共结果，I093 负责依赖兼容升级。

## I076: DETACH 注销附加库语句

- **分类**: SQL 会话面 / 能力扩展
- **问题**: MS20 一期（change `2026-09-29-ms20-attach-cross-db`）提供 `ATTACH '<path>' AS <alias>` 会话内注册附加库，但无对应 DETACH 注销能力——会话内无法中途释放附加库文件锁或以同别名换挂其他库
- **证据**: 2026-09-29 MS20 一期 Plan 期用户裁定豁免（change proposal 假设 5，含原话）——sqlparser 0.44 无 DETACH 关键字/AST 变体且 `Statement` 枚举不可扩展，实现须引入文本级预解析（破坏「无自研分片器」不变量，`src/cli/mod.rs::run_sql` 现依赖 `parse_stage` 整体解析）或语义挪用 `DEALLOCATE`/`DISCARD`（违背错误面诚实纪律）；one-shot 形态下注册面随进程退出释放，DETACH 无真实消费场景
- **影响**: one-shot 形态无实质缺失；长会话形态落地后用户挂错库无法卸下、附加库锁被长会话持续持有
- **方案**: 重评触发条件为 REPL（I067）或 serve/PG 长会话形态（I071）落地——届时 sqlparser 版本与自有元命令通道（REPL 元命令形态可能优先于 SQL 语句）可能已改变实现结论；触发前不实施
- **状态**: open（2026-09-29 登记，MS20 一期豁免转登记）

## I077: CLI 串行化并发模型（busy-timeout 锁等待 + 持锁窗口治理）

- **分类**: 架构 / 并发与进程形态（嵌入式 one-shot 形态的并发策略）
- **问题**: 登记时并发写同一库立即 `DatabaseLocked` exit 4；该缺口已由锁等待预算解决。剩余候选为长操作持锁窗口，历史 import 样本逐条 auto-commit 约 4.1ms/行、30000 行约 123s 全程持锁，不能当作当前新测结果。
- **证据**: 2026-10-01 用户方向（原话要点：「把一个命令行视为一个原子化的……同一时间多个并发的操作命令行，全都接受但是实际上一个一个不相互干扰地串行实施，就像操作系统底层调度 task 一样……达到假并发，实则高效异步原子化输入……要求数据库足够高效，或者实现一个合理的调度机制」）；设计判断随登记采信——advisory 文件锁本身即跨进程串行化原语（调度器已存在），同步阻塞 CLI 下「队列调度」与「等锁再执行」对调用者不可区分，无需常驻进程；MS18 one-shot 固定开销 ~6.5ms（串行吞吐 ~150 ops/s 量级天花板）；import 量化来源 change `2026-09-30-catalog-durability-and-cli-surface` Iteration 000 Act 实测
- **影响**: 多 agent 并发写场景从硬失败变为全部接受、逐个原子完成（每次调用全程持独占锁，重叠结构性不可能）；语义对齐 SQLite `busy_timeout` 先例；跨进程多写者决策可大幅推迟
- **方案**: 三件套——① 锁等待策略可配置：`--busy-timeout` / env / lib 三通道（沿用 `RTSQL_KEY`/`RTSQL_WAL_CHECKPOINT_BYTES` 通道先例，复用 `try_lock_with_backoff` 改策略，默认保守值，超时后维持 `DatabaseLocked` exit 4 既有错误面；读共享锁与写独占锁两侧统一生效）；② 长操作持锁窗口治理：import 攒批提交（fsync 次数按批数量级下降）+ I021 批量 INSERT，压缩最长持锁尾；③ I071 定位注记已随本条更新——`rtsql serve` 为串行模型达吞吐天花板后的扩容路径，非对立方案
- **边界**: 原子单元 = 单次 CLI 调用，跨调用语句序列不构成事务（复合原子性由单次调用内多语句/显式事务承担，MS11-T02 已支持）；flock 非 FIFO，不自造票据队列（公平性诉求留给 I071 扩容路径）；I049 组提交在本模型下无效（串行单写者无并发提交可攒）——MS22 范围裁定的避坑项
- **状态**: promoted（2026-10-01 用户批准重排纳入路线——① busy-timeout → MS28-T01、② import 攒批/持锁窗口治理 → MS22 候选池、③ I071 定位注记已随本条完成；`.agents/docs/tasks.md`）
- **语义澄清（2026-10-18）**: 上述「单次调用原子」只表示文件锁覆盖的互斥执行窗口，不表示调用内多条 auto-commit 语句全有或全无；事务原子性仍由显式事务及各语句契约决定。批次事务候选沿 I021 保留，I083 不重复承诺；跨进程读写并行另见 I084。

## I080: SQL 名字与类型的统一绑定阶段

- **分类**: 正确性 / 规划器与语义模型
- **问题**: 名字解析依赖字符串限定符、列位置和构建上下文，部分执行路径仍按列名定位；近期别名、子查询和投影修复表明组合语义值得统一。
- **证据**: 2026-10-18 静态读取 `src/parser/planner/mod.rs` 的 `building_subquery` / `building_from_derived` / `join_column_layout` 与 `src/executor/join.rs::condition_column_index`；已发生的修复来源见任务台账 2026-10-04、2026-10-15 记录。
- **影响**: 增加 JOIN、关联子查询及派生查询组合的维护成本；不是尚未复现的新缺陷声明。
- **建议**: 逐步建立 AST → 名字/类型绑定 → 逻辑计划 → 物理计划的边界，绑定结果携带关系身份、列身份、类型与必要的可空信息；先收口 JOIN 和子查询，不一次重写全部规划器。
- **关联**: I016 负责代价选择，I089 消费参数绑定结果，I093 负责解析依赖升级；更换解析器不能代替绑定层。
- **状态**: promoted（2026-10-19 用户批准底层设计批纳入路线——MS30-T02；`.agents/docs/tasks.md`）

## I081: 执行器批量异步接口

- **分类**: 性能 / 执行接口
- **问题**: `Executor` 以 `async next()` 逐行返回，组合树使用动态执行器；`async-trait` 展开为装箱 Future，实际分配及调度耗时占比尚未测量。
- **证据**: `src/executor/executor_trait.rs`、`src/executor/sort.rs`；[async-trait 官方展开说明](https://docs.rs/async-trait/latest/async_trait/#explanation)。
- **影响**: 高频扫描和多层算子可能承受重复调用及装箱开销；目前没有收益测量。
- **建议**: 先测量，再评估 `next_batch()`，优先复用现有 Value 与行容器，逐步转换热点算子；不预设引入 Arrow 或完整向量化框架，也不直接删除支撑 `dyn Executor` 的 async-trait。
- **关联**: I082 管理算子物化与内存，I065 管理响应流式消费，I066 评估剩余分配器成本。
- **状态**: promoted（2026-10-19 用户批准底层设计批纳入路线——MS32-T01，量化先行纪律随任务保留；`.agents/docs/tasks.md`）

## I082: JOIN 流水处理、Top-N 与查询内存预算

- **分类**: 资源 / 执行器与查询规模
- **问题**: 排序先收集全部输入；Hash JOIN 构建右侧哈希表后继续缓存全部左侧行，并克隆右侧匹配集合。本轮所读路径未见对应的算子内存预算或临时文件溢写。
- **证据**: `src/executor/sort.rs::initialize`、`src/executor/join.rs::next` 的 BuildRight / ScanLeft / Output 三阶段，2026-10-18 静态核对。
- **影响**: 大表、大值和多匹配键组合会增加内存占用及首行等待；未运行新的规模测试。
- **建议**: 先评估左侧逐行探测及减少集合复制，再评估 ORDER BY + LIMIT 的 Top-N；明确查询内存上限与超限错误，确有需求时才设计外部排序或分区 JOIN 溢写。
- **关联**: I020 负责复制表示，I016 负责较小构建侧选择，I065 负责算子之外的结果流；INSERT SELECT 同表源等依赖物化快照的行为须单独保留语义。
- **状态**: promoted（2026-10-19 用户批准底层设计批纳入路线——MS32-T02；`.agents/docs/tasks.md`）

## I083: CSV 导入输入内存与逐行 SQL 往返

- **分类**: 性能 / CLI 输入与写入适配
- **问题**: import 先收集 CSV 记录，逐行解析为值，再渲染 SQL 字面量、拼装 INSERT 并调用 `execute_sql()`；重复经过解析与规划。
- **证据**: `src/cli/lifecycle.rs::import_csv` 的 records 收集、`sql_literal` 与逐行执行循环，2026-10-18 静态核对。
- **影响**: 导入内存和 CPU 成本随输入增长；历史逐行提交耗时见 I077，不能把耗时全部归因于解析。
- **建议**: 复用 I021 的类型化写入口减少文本往返；评估有界读取批次，并明确自动建表类型推导需要全输入时的策略及失败副作用，不能直接截取少量样本改变现有推导结果。
- **关联**: I021/I077 已承接批量提交与持锁窗口候选，本条不重复安排；I078 的 stdin/自动建表已交付。
- **状态**: open（2026-10-18 用户接受方向，未承诺实施）

## I084: 跨进程一写多读的能力评估

- **分类**: 并发 / 文件锁与快照
- **问题**: 读写打开持有 fd 生命周期内的独占文件锁，读写进程互斥；锁等待预算改善接受方式，未提供同时读写。实例内 MVCC 不自动等价于跨进程快照。
- **证据**: `src/storage/file_storage.rs::open_with_lock` / `open_readonly_with_lock`；README 只读打开的共享/独占锁边界。
- **影响**: 一个长读写会话可阻塞其他 CLI 读者；是否值得增加复杂度取决于实际并发负载。
- **建议**: 先测量等待与持锁时间，再评估读快照、跨进程可见性协调、缓存失效和 checkpoint 安全边界；不能只放宽 flock。比较保留串行 CLI 与 I071 长会话路径的成本。
- **关联**: I077 的已交付等待语义继续有效；本条不预设多写者或跨文件原子事务。
- **状态**: open（2026-10-18 用户接受方向，待场景确认）

## I085: 字符串与组合索引、范围访问能力

- **分类**: 功能 / 索引与查询路由
- **问题**: CREATE INDEX 目前限单列 INT；显式主键限 INTEGER，UNIQUE 限单列 INT，字符串键和组合键的通用索引面受限。
- **证据**: `src/parser/planner/ddl_dml.rs::build_create_index` 的 INT/单列守卫、建表约束守卫；README 约束说明。
- **影响**: 常见字符串标识和多列过滤难以通过当前二级索引能力表达；不是对所有范围扫描路径均缺失的断言。
- **建议**: 按真实查询选定字符串或组合索引的最小范围，评估保序编码、NULL/类型比较、范围路由、唯一性、写入维护和恢复；不一次开放所有索引语法。
- **关联**: I024 是物理 Key 编码与空间评估，本条是用户可见能力；I069 的 DECIMAL/BLOB 另行保留。
- **状态**: open（2026-10-18 用户接受方向，未承诺实施）

## I086: DROP 与持久化失败边界的验证和加固

- **分类**: 可靠性 / DDL、WAL 与错误恢复
- **问题**: 文档明确 DROP 类页释放以 checkpoint 为边界；已读的进程崩溃记录不能独立证明断电、磁盘满、短写和同步失败等边界已覆盖。
- **证据**: `README.zh-CN.md` 备份与 schema 持久化段、`src/storage/data/table_manager.rs::drop_table`；未复现新损坏，不将疑点登记为已确认缺陷。
- **影响**: 重要数据使用前，需要明确每类失败的可恢复结果与已提交状态。
- **建议**: 先盘点现有测试，再在项目测试框架中定向补充存储写入/同步失败、checkpoint 中断及 DROP 恢复场景；依据实际失败决定加固范围。进程 kill、受控 I/O 故障与真实断电结论分开。
- **关联**: I031 的树刷盘策略、I060 的伴生文件加密、I064 的语义说明各保留独立范围；禁止另建证据身份或外部判定协议。
- **状态**: open（2026-10-18 用户接受方向，待测试缺口调查）

## I087: 空闲页跨重启复用与文件整理

- **分类**: 资源 / 页分配与磁盘空间
- **问题**: FileStorage 空闲页列表仅保存在内存，打开时为空；DROP 释放页可在同次打开复用，重启后列表不恢复。
- **证据**: `src/storage/file_storage.rs` 的 `free_pages` 初始化与 allocate/free 路径；`src/storage/data/table_manager.rs::drop_table` 注释明确说明列表不持久化，2026-10-18 静态核对。
- **影响**: 反复 one-shot 建删表可能使已释放页失去跨会话复用机会；文件增长规模本次未测量。
- **建议**: 先评估可靠的持久空闲页管理或打开时安全重建，覆盖释放/重分配/崩溃恢复；尾部截断或 VACUUM 式整理作为后续独立选择，不只序列化内存列表。
- **关联**: I038 处理无键行旧版本 GC，I086 处理 DROP 失败边界；三者不是同一回收层次。
- **状态**: open（2026-10-18 用户接受方向，未承诺实施）

## I088: 类型化库结果与核心/协议解耦

- **分类**: API / 嵌入式使用与模块边界
- **问题**: `Database::execute_sql` 返回 `network::protocol::Response`，QueryResult 是 JSON 值数组且无完整列元数据，库使用者不能直接消费内部 Value 类型。
- **证据**: `src/database.rs::execute_sql`、`src/network/protocol.rs::Response`、`src/pipeline.rs::execute_executor` 与 `value_to_json`。
- **影响**: 核心依赖网络结果词表；类型和 schema 信息需由调用者另行处理，核心执行到 JSON 的转换不能按需选择。
- **建议**: 评估原生 Value、列名/类型等元数据组成的公共结果，CLI/JSON/PG 在边界转换；明确现有 execute_sql 调用者的兼容与迁移，不预设更换序列化库。
- **关联**: I065 负责流式游标/背压，I074 负责可选 feature 与 crate 边界；本条不以整体拆 crate 为前置。
- **状态**: promoted（2026-10-19 用户批准底层设计批纳入路线——MS31-T01；`.agents/docs/tasks.md`）

## I089: 库参数绑定与可复用预编译语句

- **分类**: API / SQL 参数与计划复用
- **问题**: 当前常用库入口接收 SQL 字符串；程序调用需要自行渲染值，重复 SQL 的参数类型与计划生命周期缺少统一公共契约。
- **证据**: `src/database.rs::execute_sql` / `execute_in_tx` 的字符串签名，以及 I083 的逐行 SQL 构造消费场景；本次未逐一验证全部占位符语法。
- **影响**: 嵌入式使用的转义、类型保真和重复执行成本增加。
- **建议**: 先评估接受类型化参数的 execute/query 接口，再评估 prepare；明确定型、NULL、参数个数/类型错误和 schema 变化后的计划失效，不能用字符串替换实现绑定。
- **关联**: I080 提供语义绑定，I088 提供类型化结果；I071 是 PG Extended Query 协议，库参数绑定不依赖 serve 复活。
- **状态**: promoted（2026-10-19 用户批准底层设计批纳入路线——MS31-T02；`.agents/docs/tasks.md`）

## I090: 查询取消、超时与事务状态契约

- **分类**: 可靠性 / 异步 API 与资源生命周期
- **问题**: CLI 已有 SIGINT/SIGTERM 编排，但库 Future 被丢弃、查询超时和网络断开后的事务结果不能仅由 CLI 关闭行为推定；本轮所读 Executor 接口未携带统一查询取消上下文。
- **证据**: `src/cli/mod.rs` 信号 select/close 路径、`src/executor/executor_trait.rs`；[Tokio timeout](https://docs.rs/tokio/latest/tokio/time/fn.timeout.html)、[spawn_blocking 的取消限制](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)。本次未运行取消安全验证，不声明已发现事务损坏。
- **影响**: 调用者可能无法判断取消后语句是否提交、锁是否释放及句柄能否继续使用。
- **建议**: 明确提交前中断、提交已完成但未返回、显式事务继续/回滚等结果；核对执行器、事务和后台 I/O 的生命周期，再决定协作取消与 deadline 的最小接口。超时不是自动回滚的替代实现。
- **关联**: I082 管理内存预算，本条管理取消与事务收尾；既有 CLI 信号测试可复用，不重复建设。
- **状态**: open（2026-10-18 用户接受方向，待语义调查）

## I091: SQL 差分、等价变换与结构性质测试

- **分类**: 测试 / 静默错误结果与未知组合
- **问题**: 已有回归能锁定已知缺陷，但近期别名、子查询、索引投影等修复表明仍需主动发现未知组合；目前本轮检索未见 tests 中使用 rusqlite 建立差分套件。
- **证据**: `Cargo.toml` 已有 rusqlite 开发依赖；任务台账 2026-10-04 至 2026-10-15 记录；[数据库逻辑错误测试研究](https://arxiv.org/abs/2001.04174)。本次不声称仓库所有测试均缺乏性质验证。
- **影响**: 只围绕单一预期答案编写回归，可能遗漏等价 SQL 或多特性组合产生的静默偏差。
- **建议**: 在现有 Rust 测试框架内复用 rusqlite，仅比较双方语义一致的 SQL 子集；增加别名/派生表/等价谓词变换，以及 B-Tree、版本链和编码往返的性质测试。规范化排序、NULL 与数值差异，缩减失败输入；确需扩大探索时再评估 fuzzing 依赖。
- **关联**: I080 的绑定重构可消费这些测试；不在测试框架外建立判定脚本、身份协议或专用审计工具，不重复已有先行测试。
- **状态**: promoted（2026-10-19 用户批准底层设计批纳入路线——MS30-T01；`.agents/docs/tasks.md`）

## I092: 历史数据库与 dump 的升级兼容性验证

- **分类**: 兼容性 / 格式演进与版本交付
- **问题**: 项目已有文件头版本守卫和局部向后兼容分支，但文档同时保留旧约束声明 dump 在新版本被拒的边界；升级对已有数据的影响需要统一可测试的契约。
- **证据**: `src/storage/file_header.rs`、README 约束存量兼容段；本次未运行跨版本测试，不断言现有局部兼容用例缺失。
- **影响**: 旧库能打开、旧 dump 能恢复与新功能可写入是不同能力，版本升级可能需要用户迁移。
- **建议**: 先定义支持的旧格式/旧 dump 范围，再在现有测试框架维护少量必要兼容样本，直接断言打开、查询、写入和恢复结果；不兼容输入给出明确拒绝或迁移路径。版本是产品格式契约，不添加运行身份匹配机制。
- **关联**: I051/I052/I053 分别管理 CI、预编译与发布，本条不启用这些此前暂缓的设施；I024/I060 等格式变更需考虑本条的兼容要求。
- **状态**: open（2026-10-18 用户接受方向，未承诺实施）

## I093: 依赖兼容升级与可选功能组合验证

- **分类**: 维护性 / 依赖与构建配置
- **问题**: Cargo.toml 中解析、运行时、网络和加密依赖共同组成当前构建；升级依赖与调整 feature 都可能影响 AST、错误面及已有调用者，不能把换版本当成语义修复。
- **证据**: `Cargo.toml` 的 sqlparser 0.44、Tokio features 与无 optional 声明的依赖；本轮没有确认特定依赖漏洞或需要立即升级的版本。
- **影响**: 长期维护与可选构建组合可能受版本变化影响，收益和兼容成本需要分开评估。
- **建议**: 按真实兼容、安全或维护需求逐项核对官方版本与变更，再用受影响测试验证；保留 Rust/Tokio 基础选型。原生 async trait 的可用性与 `dyn Executor` 兼容单独判断，不能仅按编译器支持就移除 async-trait。
- **关联**: I074 负责 feature 边界，I081 负责执行接口，I080 负责绑定语义；不为追新同时升级全部依赖。
- **状态**: open（2026-10-18 用户接受方向，待具体升级理由）

## I094: CLI 运行时规格的目标权衡复评

- **分类**: 性能 / one-shot 启动与资源
- **问题**: CLI 当前使用多线程 Tokio；历史 runtime flavor 评估因 RSS 判据未通过而保留现状，不能把 current_thread 视为尚未试过或已证明全面更优。
- **证据**: `src/main.rs` 的 `#[tokio::main]`；任务台账 one-shot 阶段记录和 `openspec/changes/archive/2026-09-26-ms18-oneshot-fixed-cost/design.md`，均是历史测量，本次未重跑。
- **影响**: 启动时延、常驻内存与长任务吞吐可能需要不同取舍。
- **建议**: 只有实际负载或用户优先目标变化时再评估；先明确时延/RSS/吞吐的接受条件，复用历史对比方法，保留库与网络运行形态的独立要求。本次登记不推翻已验收的多线程选择。
- **关联**: I066 是分配器评估，I028 是文件 I/O 后端；即使 current_thread，spawn_blocking 仍可创建阻塞线程，不能承诺整体单线程。
- **状态**: open（2026-10-18 用户接受复评方向，未承诺实施）

<!-- arc: ARC-202609092322 --> 7 条已归档 (2026-09-09) → openspec/changes/archive/2026-09-09-ARC-202609092322/proposal.md
<!-- arc: ARC-202609241843a --> 2 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609241843a/proposal.md

<!-- arc: ARC-202609242151 --> 24 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609242151/proposal.md

<!-- arc: 4e1944f --> 9 条已归档 (2026-10-01)：I001-I003（Phase 1 已完成段）/ I004-I008（Phase 2 已完成段）/ I075（promoted → MS25-T06 已实施，spec `order-by-alias-sorting`）

<!-- arc: a61891f --> 2 条已归档 (2026-10-20)：I078（promoted → MS29-T01 已交付，change `2026-10-01-ms29-import-input-and-default-declaration` 归档）/ I079（promoted → MS28-T02 已交付，change `2026-10-01-ms28-cli-concurrency-checkpoint` 归档）；原文 `git show a61891f:.agents/memory/improvements.md`
