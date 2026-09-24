# tasks — 任务与里程碑路线

> 最后更新：2026-09-24（milestone-planner 初版后第一批路线规划 MS18-MS22：one-shot 性能收口 / CLI 管理面小收口 / ATTACH 跨库交互 / DDL 演进与二级索引 / 实测驱动性能批——用户批准方向「功能+性能优先，排除微内核重构、加密、构建实测、分发」；improvements 同步：I061/I062/I063/I059/I068/I070 标注已排期 MSxx-Txx、8 项标 MS22 候选、I035/I043/I044 补转 promoted 勘误。前次 2026-09-24：RISC-V musl 交叉构建收尾 + MS17 初版收尾 1101 tests）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

23 个 Milestone：12 completed（MS00-MS02 历史 + MS06/MS07/MS09/MS10/MS11/MS13/MS15/MS16/MS17）+ 6 superseded（MS03/MS04/MS05/MS08/MS12/MS14）+ 5 planned（MS18-MS22 初版后第一批，2026-09-24 规划）。★初版达成★。

规划理念：**先收口正确性 → 建设基础能力 → 实测驱动性能 → 引擎能力收尾 → 应用层可用好用（非交互 CLI / 密钥 / 分析 / 分发）**（2026-09-06 依据 R18 分析扩展应用层轨道）。2026-09-11 按「尽早达成 CLI 数据库初版」重排：分发收口自 MS13 拆出前移（MS14），初版前收口 4 项正确性缺陷（MS15），分析函数/加密/性能后置。2026-09-12 再调整（用户裁定「暂时不分发，先把工作做完再谈分发」）：MS14 后置至执行序末位，新建 MS16 承接 MS15 残差置顶，MS09 扩入 MVCC 域正确性收口（I033/I032）后前移。2026-09-14 再调整（用户裁定「先把整个数据库的初版做出来，优化不是主要工作」）：**MS08 剥离出执行序**——剩余 planned 任务退还 improvements 域留作后续候选（I021/I024/I026/I031/I038 更新 + 新登记 I049/I050）；**MS14 不排入执行序**，分发时点由用户届时裁定（「觉得差不多的时候」）。

执行顺序（2026-09-24 MS17 完成后收口）：**MS15 正确性收口（✅ 2026-09-12）→ MS16 正确性收口第二批（✅ 2026-09-13）→ MS09 引擎能力与 MVCC 收尾（✅ 2026-09-14）→ MS13 分析函数（✅ 2026-09-23）→ MS17 初版分发收口（✅ 2026-09-24，★初版达成★）**。既定执行序已全部完成；性能优化、原 MS08 剩余项与 MS12/MS14 未选取项保留在 improvements 域，等待后续独立规划。旧执行序（2026-09-14、2026-09-12、2026-09-11 及 2026-09-06 序）均已被本序取代。

新执行序（2026-09-24 用户批准方向规划：功能类与性能优化优先，排除微内核重构 I074/I012、加密便利层 I054-I057/I060、构建实测 I073、分发 I051-I053/I058）：**MS18 one-shot 路径性能收口 → MS19 CLI 管理面小收口 → MS20 ATTACH 式跨库交互 → MS21 DDL 演进与二级索引 → MS22 实测驱动性能批（范围量化定稿后转 ready）**。顺序为建议序非硬依赖（各项无环、前置均已满足）；性能批候选项沿用 MS08「先量化再决定」纪律。REPL（I067）、DECIMAL/BLOB（I069）、撕裂树（I031）、代价模型（I016）、流式化（I065）、B+Tree 节点锁（I025）、io_uring（I028）等留 improvements 域未排期。
旧优化项已重新分类：见各 superseded MS 的"原范围"段与 D-candidates 列表。

## 已完成历史

### MS00：核心开发（2026-05-24 归档）

- Status: completed（pre-MS 体系，保留作历史）
- 关键成果：完整 SQL + WAL + Group Commit + 崩溃恢复 + B-Tree Split & Merge + 关联子查询
- 测试基线：464 tests pass（2026-05-24）
- 性能基线：INSERT 332x faster、PK lookup 5.6x faster than SQLite
- 详见 `openspec/changes/archive/` 历史 change 目录

## Milestone Roadmap

### MS01：Phase 1 基础设施 — completed

- **Status**: completed
- **Outcome**: 事务 ID 分配、连接并发限流、网络响应批写三项基础设施级优化完成
- **Stable baseline**: 475 tests pass (2026-06-04)
- **Scope**:

| Task | 优化项 | 预期收益 |
|---|---|---|
| MS01-T01 | 事务 ID AtomicU64 | 分配延迟 100ns→10ns（实测 5.1 ns/op） |
| MS01-T02 | 连接并发 Semaphore | 防连接风暴 |
| MS01-T03 | 网络 BufWriter + TCP_NODELAY | write 调用 -99% |

- **Verification boundary**: 单线程 5.1 ns/op 分配；连接限流 3 压测；网络 N→2 syscalls
- **Diagnostic boundary**: AtomicU64 性能 → benches/tx_id_bench.rs

### MS02：Phase 2 存储引擎核心 — completed

- **Status**: completed
- **Outcome**: 零拷贝读路径 + 数据页直接扫描 + 页面级 MVCC 摘要三项存储引擎优化完成
- **Stable baseline**: 481 tests pass (2026-06-06)
- **Scope**:

| Task | 优化项 | 预期收益 |
|---|---|---|
| MS02-T01 | 零拷贝 SlottedPageRef | 读路径 -2.46%~-8.33% |
| MS02-T02 | DataScan 路径 | 全表扫描 1.81x-2.44x |
| MS02-T03 | 页面级 MVCC | 可见性快速路径 |
| MS02-T04 | 零拷贝 ValueRef | 堆分配 30万→0（目标未直接验证） |

- **Verification boundary**: DataScan 1K/10K 实测；visibility bench 3 场景
- **Diagnostic boundary**: 零拷贝性能 → benches/single, benches/data_scan_bench.rs
- **依赖**: MS02-T01 → MS02-T02/T04（写路径 0 回归），MS02-T02/T03 → 未来 MS08 预取

### MS03：Phase 3 并发控制 — superseded

- **Status**: superseded
- **Superseded by**: MS07（消息传递重构类基础能力）、MS08（行锁与 fsync 等性能优化）
- **替代时点**: 2026-08-25 重规划
- **替代原因**: BufferPool DashMap 优化已落地；剩余项与 MS07 基础能力（消息传递）和 MS08 实测驱动性能更契合
- **原范围**: BufferPool DashMap + miss Sem + per-page loading_locks（done）；行锁 DashMap、WAL fsync 合并、WAL 背压、消息传递重构、pread/pwrite（planned）

### MS04：Phase 4 上层功能 — superseded

- **Status**: superseded
- **Superseded by**: MS07（schema 持久化）、MS09（隔离级别、多 Join、PG 协议、子查询缓存）
- **替代时点**: 2026-08-25 重规划
- **替代原因**: Schema 持久化是 SQL 标准的"能用"前提，应优先；代价模型与 clone 消除移到 D-candidates
- **原范围**: 多隔离级别、多 Join 算法、代价模型 + Join 重排、关联子查询缓存、多层关联子查询、PG Extended Query、clone 消除 Arc/Cow、INSERT 批量执行、表定义持久化

### MS05：Phase 5 高级优化 — superseded

- **Status**: superseded
- **Superseded by**: MS08（实测驱动的性能优化）
- **替代时点**: 2026-08-25 重规划
- **替代原因**: B+Tree 节点级锁、io_uring、瘦内部节点、合并 Tag byte 复杂度高/收益低/风险高，移到 D-candidates
- **原范围**: 预取 Prefetch、Varint Key 编码、B+Tree 节点级锁、脏页 writev、并行扫描、io_uring、瘦内部节点、合并 Tag byte

### MS06：稳定性与正确性收口 — completed

- **Status**: completed（T01-T04 全部完成，2026-08-26）
- **Outcome**: 所有 DML 写入正确的 `create_tx_id`；PlanCache 在 100 并发下不阻塞 runtime；WAL 持续写入不发生文件句柄泄漏；pipeline 执行路径可被独立观测
- **Rationale**: BufferPool 锁优化已完成，但代码层发现 4 类被掩盖的稳定性问题（INSERT `tx_id=0` 注入、PlanCache `std::sync::Mutex` 跨 `.await`、WAL 每写 open/close、pipeline::execute_inner 200+ 行）。这些问题在任何性能/功能扩展前必须先封堵，否则后续 verification 都被噪声污染
- **Dependencies**: None
- **Scope**:

| Task | 状态 | 目标 | 目标文件 | 验收 |
|---|---|---|---|---|
| MS06-T01 | **completed**（2026-08-25） | 修 INSERT/UPDATE/DELETE `tx_id=0` 占位注入 | `src/pipeline.rs:336/350/363`、`src/executor/{insert,update,delete}.rs`、`src/transaction/version_chain.rs` | 写后 `create_tx_id != 0`；MVCC visibility 不再恒成立 ✅ |
| MS06-T02 | **completed**（2026-08-26） | PlanCache 改 DashMap + SQL 规范化 key + 替换 `std::sync::Mutex` | `src/plan_cache.rs`、`src/database.rs:22/64/95`、`src/pipeline.rs:56-65/145/169/206` | 100 并发压测 plan_cache 不阻塞 runtime（实测 0.08s ≪ 5s）；大小写/空白变体 100% hit；504 tests pass ✅ |
| MS06-T03 | **completed**（2026-08-26） | WALWriter 持文件句柄，每条 write 复用 | `src/wal/writer.rs` 全方法 + `tests/wal_handle_test.rs` | 10K tx 压测 fd 净增量 < 10（实测 delta=0/-4 < 10）✅ |
| MS06-T04 | **completed**（2026-08-26） | `pipeline::execute_inner` 拆为 parse/plan/execute 三阶段 + profiling gates | `src/pipeline.rs`（三 pub stage + 编排器）+ `benches/pipeline_stages_bench.rs` + 8 阶段单测 | 三阶段独立 micro-bench（parse 3.25 µs / plan 6.26 µs / execute 796 ns）；单测可分别覆盖 ✅ |

- **Non-goals**: 任何性能优化；新 SQL 方言；新执行器；新隔离级别
- **Workload**: 4 类修复 + 每类加 micro-bench/回归测试 + 重跑 460 tests
- **Stable baseline**: 10K tx 压测无句柄泄漏；100 并发 plan_cache 不阻塞 runtime；DML `create_tx_id != 0`；pipeline 各阶段耗时可独立观测
- **Verification boundary**: 4 项独立测试套件全通过
- **Diagnostic boundary**: 4 个具体代码位置
- **Split signals**: 若 MS06-T02 或 MS06-T04 任一需要 2+ change 完成，拆为两个 MS
- **Related changes**:
  - `2026-08-25-fix-dml-tx-id-injection`（已归档为 `archive/2026-08-25-2026-08-25-fix-dml-tx-id-injection/`，含新增 spec `dml-transaction-lifecycle`）
  - `2026-08-25-ms06-t02-plancache-dashmap`（已归档为 `archive/2026-08-26-2026-08-25-ms06-t02-plancache-dashmap/`，含新增 spec `plancache-key-normalization`；T0 基线 clippy 归零同步并入）
  - `2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages`（已归档为 `archive/2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages/`，含新增 spec `wal-writer-handle-reuse` + `pipeline-stage-decomposition`）

### MS07：基础能力建设 — planned

- **Status**: planned（T01-T06 已完成，2026-09-05；T07 条件性未触发——T05 的 checkpoint 重写截断未暴露 WALBuffer 并发协调需求，是否引入消息传递重构待后续评估）
- **Dependencies**: MS06
- **Outcome**: 表定义持久化到磁盘；restart 后 schema 完整恢复；drop_table 真正释放页；显式事务 API 可用；Checkpoint 真正工作；planner 模块可独立单测；谓词/LIMIT 可下推
- **Rationale**: WAL redo 静默吞错（`src/wal/recovery.rs:148/165/176`）、checkpoint 无效、planner 2266 行单文件、显式事务缺失、谓词无法下推 — 都是 SQL 标准合规的"能用"前提。Schema 持久化是 drop_table、checkpoint、redo verification 的共同前置
- **Scope**:

| Task | 状态 | 目标 | 关键依赖 | 关联 change |
|---|---|---|---|---|
| MS07-T01 | **completed**（2026-08-26） | 系统表 `__tables` / `__columns` + Schema 页（最大单点） | 无 | `archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/` |
| MS07-T02 | **completed**（2026-08-30） | drop_table 接 free-list，物理页释放 | MS07-T01 | `archive/2026-08-30-2026-08-26-ms07-t02-drop-table-physical-free/` |
| MS07-T03 | **completed**（2026-08-30） | planner.rs 2266 → 按 build_* 拆分到 4-6 个模块 | 无 | `archive/2026-08-30-2026-08-30-ms07-t03-planner-decomposition/` |
| MS07-T04 | **completed**（2026-09-05） | `Database::begin/commit/rollback/execute_in_tx` 公开 API + 事务内执行路径 + 版本按表聚合多表回滚 | 无 | `archive/2026-09-05-2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown/` |
| MS07-T05 | **completed**（2026-09-05） | Checkpoint 真正工作：恢复消费位点 + WAL 重写截断（有界）+ 恢复静默吞错显式化（K05） | MS07-T01 | 同上 |
| MS07-T06 | **completed**（2026-09-05） | 谓词/LIMIT 下推到 DataScan 行内过滤与提前封顶（OR/Sort/Aggregate 保留原路径） | 无 | 同上 |
| MS07-T07 | planned | 视 T04/T05 需要决定是否引入消息传递重构 | MS07-T04, MS07-T05 | — |

- **Non-goals**: 性能调优（除 pushdown 收益外）；新 SQL 方言；新执行器；多隔离级别
- **Workload**: 1-2 change（Schema 页）+ 1 change/其他子项，共约 5-6 change
- **Stable baseline**: restart-after-drop-and-reload 完整恢复；DDL/DML 显式事务可单测；checkpoint 触发后 redo 数量下降；planner 任意子模块可独立单测
- **Verification boundary**: 5 项独立测试套件 + restart e2e（redo 不再静默）
- **Diagnostic boundary**: 各子项 1-2 个具体代码位置
- **Split signals**: 若 MS07-T01 因复杂度拆 2 个 change 仍可保留；若 MS07-T03 触发 planner 大规模回归失败，拆为独立 MS
- **Related changes**:
  - `2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown`（T04/T05/T06 合并 change，已归档为 `archive/2026-09-05-2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown/`，含新增 spec `ms07-rest-tx-checkpoint-pushdown`，3 Requirement：R1 显式事务 / R2 Checkpoint / R3 谓词-LIMIT 下推）

### MS08：性能压测（实测驱动） — superseded（2026-09-14 用户裁定剥离）

- **Status**: superseded（2026-09-14 用户裁定「先把整个数据库的初版做出来，优化不是主要工作」——剩余 planned 任务剥离出执行序，退还 improvements 域留作后续候选；T01/T02 已于 2026-09-05 完成交付，本 MS 不再承载剩余工作）
- **Superseded by**: 无 MS 承接——退还 improvements 域：I026（T03 脏页 writev）/ **新 I050**（T04 RowLockTable DashMap）/ I024（T05 Varint Key）/ **新 I049**（T06 WAL fsync 合并）/ I031（T07 撕裂树运行期根修）/ I038（T08 GC 无键链）/ I021（T09 INSERT 批量）——全部未排期候选，「先量化再决定」纪律随各条目保留
- **替代时点**: 2026-09-14
- **替代原因**: 性能优化为增强性质，初版达成（MS13/MS12 功能面 + MS14 分发）不需要；「实施前先 `--save-baseline`」「先量化再决定」纪律随 I 条目保留，用户提议时按独立 change 规划
- **原范围（已完成交付）**: T01 `pread`/`pwrite` 页 I/O 位置参数化（每页 1 syscall）+ T02 Prefetch 双缓冲（默认路径实测回退，replan 后默认关闭、`with_prefetch(true)` 显式启用）——归档 change `archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/`，含新增 spec `storage-io-optimization`（3 Requirement：R1 页 I/O 位置参数化 / R2 零接口零格式变更 / R3 DataScan 预取可选能力默认关闭）
- **Related changes**: 同上（T03-T09 剥离时均无 change）

### MS09：引擎能力与 MVCC 收尾 — completed（2026-09-14）

- **Status**: completed（聚合 change `2026-09-13-ms09-engine-mvcc-closeout` 单 change 收口 T01/T02/T04，2026-09-14；用户指令「把 MS09 规划成一个 change」聚合，Gate 1 批准 2026-09-13；3 Iteration 7 Cycle 全部 Plan Review accepted——000 经 D10 快照结构修订、001 经用例 8 见证改形裁定，Cycle 序列见归档 carrier）
- **Dependencies**: None（原 MS08 为排序性依赖，2026-09-12 放松；T01/T02/T04 技术前置 MS07 已完成）
- **Outcome**: 隔离级别经 lib API 可配（`Database::open_with_isolation`，默认 RR 逐字节等价）且 RC 语句级已提交视图可验证（他事务未提交写不可见——脏读排除、语句间提交可见·删除消失、自身未提交写可见）；I033 墓碑抑制语义收口（墓碑独立版本 slot 自描述删除者：已提交墓碑抑制整条链、未提交/已回滚墓碑不抑制回溯前驱，运行期与崩溃恢复两态一致——跨进程 update→delete 两步变更不再从扫描面消失）；I032 实施（恢复期未提交行显式 `mark_uncommitted_aborted` 中性化，不复活；`mark_tx_aborted` no-op 移除）；NLJ 执行器 + 计划期启发式切换（非等值/混合/字面量腿 ON 经 NLJ 可达，等值 Hash 逐字节保持，不含代价模型）；关联子查询语句级缓存（三执行器关联臂，相同参数值至多执行一次、命中与直执行逐字节等价）
- **Rationale**: 原四项中 PG Extended Query 被非交互 CLI 形态决策降级（无消费者，server 保留为库能力，将来 serve 复活时再议）；其余三项同属引擎能力面，与应用层轨道分开验收。2026-09-12 扩入 I033（update→delete 旧版本无快照重现，跨进程实证、原「未来混合负载」预判已现实化）与 I032（mark_tx_aborted 空实现）——两者与 Read Committed 同属事务可见性域且条目自注届时一并处理，故本 MS 前移承接「先把工作做完」
- **Scope**:

| Task | 状态 | 目标 | 关联 change |
|---|---|---|---|
| MS09-T01 | **completed**（2026-09-14） | Read Committed 隔离 + I033 墓碑抑制语义（已提交墓碑抑制整条链 / 未提交墓碑不抑制、回溯前驱）+ I032 实施或文档化（裁定：实施） | `archive/2026-09-13-ms09-engine-mvcc-closeout/`（Iter 000，3 Cycle） |
| MS09-T02 | **completed**（2026-09-14） | NLJ + 与 Hash Join 启发式切换（非等值 JOIN 一并解锁——用户裁定 2026-09-13） | 同上（Iter 001，2 Cycle） |
| MS09-T04 | **completed**（2026-09-14） | 关联子查询结果缓存（语句级，原「视 MS09-T02 完成后实际场景」由用户聚合裁定取代） | 同上（Iter 002，1 Cycle） |

- **Non-goals**: PG Extended Query（降级移出，2026-09-06）；Serializable / SSI；代价模型与 Join 重排；io_uring；B+Tree 节点级锁；clone 消除 Arc/Cow（待 MS08 完成后看真实数据再决定）；多用户权限
- **Workload**: 3-4 change（T01 含 I033/I032，视调查结果可独立成 change）；总工作量适中
- **Stable baseline**: Read Committed 跨并发可验证；跨进程 update→delete 序列扫描面不再重现已删行旧版本；NLJ 在小表上优于 Hash；关联子查询 N 行外层不重复执行子查询
- **Verification boundary**: 3 项独立测试套件 + I033 跨进程探针场景
- **Diagnostic boundary**: 3 个独立子系统（事务管理 / Join 执行器 / 子查询执行器）
- **Split signals**: 若 MS09-T01 实施发现 snapshot 与 RR 共享度过低，或 I033 方案牵动 WAL/header 格式演进过大，各自拆为独立 MS
- **Related changes**: `2026-09-13-ms09-engine-mvcc-closeout`（T01/T02/T04 聚合 change，已归档为 `archive/2026-09-13-ms09-engine-mvcc-closeout/`，含新增 specs `transaction-isolation-levels`（3 Requirement）、`mvcc-tombstone-visibility`（5 Requirement + 已知边界段）、`join-executor-selection`（5 Requirement）、`correlated-subquery-cache`（5 Requirement），明细见各 spec；**936 tests pass / 0 failed / 2 ignored**（Plan Review 独立复跑）；clippy/fmt/validate 全 0/PASS；见证书证改形两处经用户裁定；Issue 落账 ISS01（min_create_tx_id=0 毒化 → MS08 域）/ ISS02（IN×JOIN 误报，未排期）/ ISS03（标量子查询表头形状，未排期）；用户 7364bc9 统一入库 MS16 收尾 + MS09 规划与实施）

### MS10：CLI 非交互命令面 — completed（2026-09-09）

- **Status**: completed（T01-T05 全部完成，2026-09-06 至 2026-09-09；T05 含 Iteration 001 001-rework 无键行落库语义——引擎键位不可键控行由静默丢弃改为落库不入索引 + 恢复无键回退，用户裁定方向 A）
- **Dependencies**: MS08
- **Outcome**: `rtsql <db> <sql>` 主命令全链路可用——裸名集中存储（`$RTSQL_HOME`，默认 `~/.rtsql/db/`）+ 含 `/` 路径直开；`new/list/schema/dump/restore/import` 生命周期子命令；TTY 默认表格、非 TTY 默认 JSON、`--format table|json|csv|tsv`；退出码分类（0 成功/2 用法/3 SQL 错/4 锁冲突/5 密钥）；多语句 `;` 分片逐条执行+报错行号；跨进程文件锁（advisory 独占，占用报 `database is locked`）；优雅停机（信号→`close()` checkpoint）；文件 magic/格式版本头（趁零用户落，不兼容即报"文件由新版创建"）
- **Rationale**: 应用层一切能力的载体（R18 主题 7）；文件锁/优雅停机/格式头/多语句修复是正确性前置而非增强，与 CLI 壳同一验收域（"CLI 全链路可用"），不拆
- **Scope**:

| Task | 状态 | 目标 | 关键前置 | 关联 change |
|---|---|---|---|---|
| MS10-T01 | **completed**（2026-09-06，含 Iteration 001 真投影） | CLI 壳：参数化入口 + 名称解析（裸名/路径）+ 主命令 + 输出格式 + 退出码 | 无（main.rs 重写为参数化入口） | `archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/` |
| MS10-T02 | **completed**（2026-09-08，含 Iteration 000 WAL 恢复引擎正确性收口——T0 reader 帧解析 / T0b 位置寻址重放 / B-Tree 规模缺口 G1-G3 / catalog root 同步 R5 / 扫描去重 R6 / 恢复期索引去信任+重放后重建 R7/R8，design D0+D7-D10） | 跨进程文件锁 + 优雅停机（信号接线 `close()`） | MS10-T01 | `archive/2026-09-08-2026-09-06-ms10-t02-file-lock-graceful-shutdown/` |
| MS10-T03 | **completed**（2026-09-08） | 文件 magic/格式版本头（FileStorage open 校验） | MS10-T01 | `archive/2026-09-08-2026-09-08-ms10-t03-file-format-header/` |
| MS10-T04 | **completed**（2026-09-09） | 多语句执行修复（`;` 分片逐条执行 + fail-fast 序号定位 + lib 两路径显式拒绝，替换 `pipeline.rs` first() 截断；护栏退役） | MS10-T01 | `archive/2026-09-09-2026-09-08-ms10-t04-multi-statement-execution/` |
| MS10-T05 | **completed**（2026-09-09，含 Iteration 000 001-rework 建库约束持久化通道 + Iteration 001 001-rework 无键行落库语义与恢复回退） | 生命周期子命令：`new/list/schema/dump/restore/import --csv` | MS10-T01（schema 为 agent 发现刚需） | `2026-09-09-ms10-t05-lifecycle-subcommands` |

- **Non-goals**: REPL（后续可选）；密钥/加密（MS12）；分析函数（MS11/MS13）；安装分发（MS13）；鉴权/多用户
- **Workload**: CLI 模块新建（clap 依赖 + `src/cli/`）+ file_storage 锁/格式头 + pipeline 多语句 + 5 子命令
- **Stable baseline**: 脚本 `rtsql db "SELECT ..."` 端到端稳定；并发打开同一文件得到明确错误（退出码 4）；kill 后 WAL 恢复 e2e
- **Verification boundary**: CLI 集成测试（参数/名称解析/格式三态/退出码分类/锁冲突/多语句含报错行号/信号停机 e2e）
- **Diagnostic boundary**: `src/cli/`（新模块）+ `src/main.rs` + `src/storage/file_storage.rs`（锁/格式头）+ `src/pipeline.rs`（多语句）
- **Split signals**: 多语句分片需动 pipeline 事务语义时拆出独立 change 级任务；加密讨论提前成熟时 MS12 并行
- **Related changes**:
  - `2026-09-09-ms10-t05-lifecycle-subcommands`（T05 生命周期子命令 + 两轮引擎收口——000 001-rework 建库约束持久化通道（NOT NULL/UNIQUE 经 `create_table_with_constraints` 写入 catalog）、001 001-rework 无键行落库语义（键位不可键控行落库不入索引 + 恢复 keyless 桶回退，用户裁定方向 A），已归档为 `archive/2026-09-09-2026-09-09-ms10-t05-lifecycle-subcommands/`，修改 spec `cli-noninteractive-shell`（R1 子命令分发扩展 + 新增 Requirement 5 个）、`wal-recovery-replay-integrity`（+「无键行 Update 崩溃恢复语义正确」场景）。规划依据 R20 + R18 主题 7/5；4 Iteration，两轮 blocked 均为引擎既有缺陷经 CLI 数据面暴露，Plan Review 记录 PLAN-OMISSION 与 PLAN-INVALID 各一并关闭；704 tests pass / 0 failed / 2 ignored；登记 I036-I040）
  - `2026-09-08-ms10-t04-multi-statement-execution`（T04 多语句执行修复，已归档为 `archive/2026-09-09-2026-09-08-ms10-t04-multi-statement-execution/`，修改 spec `cli-noninteractive-shell`（R1 修正 + R5 替换为「多语句分片逐条执行」6 场景）。规划依据 R18 主题 2/3 + spec R5 原文承诺；单 Iteration 000-initial accepted（6 非阻塞 finding），含 S5 见证 SQL 经用户批准修订为含 FROM 等价（no-FROM SELECT 不可达登记 I035；裸 DataScan 表头缺口登记 I034））
  - `2026-09-08-ms10-t03-file-format-header`（T03 文件 magic/格式版本头，已归档为 `archive/2026-09-08-2026-09-08-ms10-t03-file-format-header/`，含新增 spec `database-file-format-header`（4 Requirement）。规划依据 R19 + 用户决策 2026-09-08（exit 1 复用/旧无头文件统一拒绝/仅主库加头/不加 CRC）；单 Iteration 000-initial accepted，含 Guidance 掩码行勘误裁定——`KNOWN_FLAGS_MASK=0`（加密位拒绝至 MS12））
  - `2026-09-06-ms10-t02-file-lock-graceful-shutdown`（T02 + Iteration 000 WAL 恢复引擎正确性收口（4 Iteration 6 Cycle），已归档为 `archive/2026-09-08-2026-09-06-ms10-t02-file-lock-graceful-shutdown/`，含新增 spec `database-file-lock`/`wal-recovery-frame-parsing`/`wal-recovery-replay-integrity`（各 2 Requirement）+ 修改 `cli-noninteractive-shell`（锁冲突 exit 4 + 优雅停机 4 场景）。规划依据 MS10 稳定基线「kill 后 WAL 恢复 e2e」；design D0/D7-D10）
  - `2026-09-06-ms10-t01-cli-shell`（T01 + Iteration 001 真投影扩展（用户批准方向 B，超出 T01 原始范围的引擎级修复），已归档为 `archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/`，含新增 spec `cli-noninteractive-shell`（6 Requirement）。规划依据 R18 `usability-gap-cli-form.md`）

### MS11：SQL 表达式与函数层 — completed（2026-09-11）

- **Status**: completed（T01/T02 2026-09-10；T03 2026-09-11；三任务均单 change 收口，无 Split 触发）
- **Dependencies**: MS10（CLI 可发现 schema，agent 可验证函数行为）
- **Outcome**: WHERE/SELECT 支持 `IN / LIKE / BETWEEN / IS NULL / CASE / COALESCE / CAST`；标量函数库第一批（string: upper/lower/length/substr/replace/trim；math: abs/round/floor/ceil）；SQL 级 `BEGIN/COMMIT/ROLLBACK` 语句（复用 MS07-T04 显式事务 API 接线 planner 语句臂）
- **Rationale**: 分析能力的主体在 SQL 层而非 CLI 命令（R18 主题 7 结论——agent 是写 SQL 的）；事务语句与表达式四件套同为"agent 写 SQL 的日常件"，同域验收；复用已有事务 API，实现面小
- **Scope**:

| Task | 状态 | 目标 | 关键前置 | 关联 change |
|---|---|---|---|---|
| MS11-T01 | **completed**（2026-09-10） | 表达式四件套 + CASE/COALESCE/CAST（`parser/planner/expression.rs` 扩展）+ SELECT 派生列 | 无 | `archive/2026-09-10-ms11-t01-sql-expressions/` |
| MS11-T02 | **completed**（2026-09-10） | SQL 事务语句 `BEGIN/COMMIT/ROLLBACK`（planner 语句臂 + pipeline 事务态接线） | 无（API 已有，MS07-T04） | `archive/2026-09-10-ms11-t02-sql-transaction-statements/` |
| MS11-T03 | **completed**（2026-09-11） | 标量函数库第一批（函数注册机制 + string 6 + math 4） | MS11-T01（表达式层就绪） | `archive/2026-09-10-ms11-t03-scalar-functions/` |

- **Non-goals**: 日期/时间类型与函数（MS13 深水区）；窗口函数（OVER/PARTITION BY）；自定义函数（UDF）；聚合扩展
- **Workload**: expression builder 扩展 + 函数注册新模块 + planner 事务臂 + 每函数/表达式测试
- **Stable baseline**: 日常过滤/派生列 SQL 全绿；SQL 事务语句往返（BEGIN→DML→COMMIT/ROLLBACK 语义与 API 等价）
- **Verification boundary**: 每函数/表达式独立测试 + parser/planner 回归（既有 585+ 零修改）
- **Diagnostic boundary**: `src/parser/planner/expression.rs` + 新函数注册模块 + `src/parser/planner/mod.rs` 事务臂
- **Split signals**: 单批函数超 1 change 时按 string/math 分两批；CAST 触发类型系统深层改动时拆出
- **Related changes**:
  - `2026-09-10-ms11-t03-scalar-functions`（T03 标量函数库第一批 string 6 + math 4，已归档为 `archive/2026-09-10-ms11-t03-scalar-functions/`，含新增 spec `sql-scalar-functions`（6 Requirement：R1 注册与分派机制 / R2 string 六件 / R3 math 四件 / R4 NULL 语义与嵌套 / R5 调用面与边界 / R6 零回归）。规划依据 tasks MS11-T03 + R18 主题 7（用户 2026-09-10 裁定 4 项语义 + 2 项默认假设）；2 Iteration 各 1 Cycle accepted；新增 `src/executor/function.rs` REGISTRY 单点注册表 + planner `Expr::Function` 臂 + Trim/Ceil/Floor 独立变体接线 + ast.rs 两放行门；scalar_function_test 28 + function.rs 18 单测 + cli_test 增至 56；845 tests pass / 0 failed / 2 ignored；clippy/fmt/validate 全 0/PASS；两轮 Review 偏差与 findings 见归档；登记 I043/I044/I045 + I036 佐证）
  - `2026-09-10-ms11-t02-sql-transaction-statements`（T02 SQL 事务语句 BEGIN/COMMIT/ROLLBACK，已归档为 `archive/2026-09-10-ms11-t02-sql-transaction-statements/`，含新增 spec `sql-transaction-statements`（5 Requirement）+ 修改 spec `cli-noninteractive-shell`「多语句分片逐条执行」（事务上下文语义，6→8 场景）。规划依据 tasks MS11-T02 + R18 主题 7（用户 2026-09-10 裁定 5 项决策：仅 CLI 适用面 / AffectedRows(0) / 边界子句全拒绝 / 边界语义默认包 / NO CHAIN 按裸语句同义）；2 Iteration 各 1 Cycle accepted；新增 `TxStatementKind`/`TransactionSession` + CLI 会话分派；tx_statement_test 19 测试；797 tests pass；登记 I041/I042）
  - `2026-09-10-ms11-t01-sql-expressions`（T01 SQL 表达式四件套与值表达式（WHERE/SELECT），已归档为 `archive/2026-09-10-ms11-t01-sql-expressions/`，含新增 spec `sql-expression-evaluation`（6 Requirement：R1 谓词四件套 / R2 三值语义 / R3 CASE-COALESCE-CAST / R4 SELECT 派生列 / R5 INSERT 负数字面量 / R6 零回归）；I040 并入并标记 promoted。规划依据 R18 主题 7 + tasks MS11-T01（用户 2026-09-10 批准计划）；2 Iteration 各 1 Cycle accepted；两轮 Review 各修正 1 项 Plan 侧问题（见归档）；769 tests pass / 0 failed / 2 ignored）

### MS12：整库加密与 sudo 式密钥 — superseded（2026-09-23 用户裁定裁剪合并入 MS17）

- **Status**: superseded（2026-09-23 用户裁定初版分发形式收窄——T01 核心最小加密（格式头 flag + Argon2id KDF + 页级 AES-256-GCM + 错误密钥显式拒绝 + 明文库零回归）与 T02 收窄（密钥通道只保留 `--key <arg>` + `RTSQL_KEY` 两条）及 T03 行为验收面（错误密码拒绝/损坏密文检测/明密互斥/零回归 + 打开延迟顺带实测记录）并入 **MS17-T01**；T02 剩余（`--password-file` 通道 / TTL 密钥缓存 / `rtsql key set/remove/status` 子命令）与 T03 正式性能 bench 基线退 improvements 域（I054/I055/I056/I057 待登记）；密钥轮换维持 Non-goal 不入候选）
- **Superseded by**: MS17
- **替代时点**: 2026-09-23
- **替代原因**: 用户裁定 v0.1 只需要「最基本的加密功能」与「可编译可安装」——sudo 式密钥管理层（缓存/子命令/密码文件）是便利层非最小集；加密的分阶段验收随 MS17 保留（split signal 原样携带：加密拆出独立 MS 时其余块独立成立）
- **Dependencies**: MS10（已满足；依赖关系由 MS17 继承）
- **原范围**（保留作历史）:

| Task | 目标 | 处置 |
|---|---|---|
| MS12-T01 | 文件头加密 flag + Argon2id KDF + 页级 AES-GCM transform（FileStorage 读写路径） | → **MS17-T01**（核心收入） |
| MS12-T02 | 密钥来源三通道 + TTL 缓存 + `key` 子命令 | → MS17-T01 收窄为两通道；TTL 缓存 / `key` 子命令 / `--password-file` 退 I054/I055/I056 |
| MS12-T03 | BDD：错误密码拒绝/损坏密文检测/明密互斥/缓存过期/性能基线（MS08 纪律） | → MS17-T01 行为验收收入（缓存过期随 T02 退；正式 bench 基线退 I057，Act 顺带实测保留） |

- **Non-goals**（沿用）: 多用户/角色权限（OS 文件权限即边界，R18 主题 6 结论）；列级加密；密钥轮换；key agent 常驻进程
- **Related changes**: None

### MS13：分析函数 — completed（2026-09-23）

- **Status**: completed（聚合 change `2026-09-23-ms13-analytics-functions` 单 change 收口 T01/T02/T03，2026-09-23；3 Iteration 各 1 Cycle 全部 Plan Review accepted——000 与 001 首轮 accepted，002 经当前 Cycle 修复（change tasks 状态行同步）复审 accepted）
- **Dependencies**: MS10 + MS11（均已满足；不依赖 MS12——加密与分析正交；不依赖 MS14——分发与分析正交）
- **Outcome**: DATE/TIMESTAMP 真类型全链贯通（值变体 + tuple TAG 0x06/0x07 + catalog COL_TAG + DDL 显式映射不再回退 String + 类型字面量/裸字符串写入强制解析 + 时间序比较排序 + CAST 矩阵 + DA5 渲染 + dump/restore/CSV 往返 + 恢复两态一致）；日期函数族 10 项（now/date/year~second/date_trunc/datediff）+ INTERVAL 表达式算术（同日锚定截月末 + sqlparser 吞比较解缠绕）+ `GROUP BY date_trunc(...)`/别名/位置分桶与混合投影解锁；no-FROM SELECT 虚拟单行（I035）+ `stats/sample/profile` 三分析薄命令；I043（abs 溢出显式错误/round 极端 digits 饱和）与 I044（大小写 SQL 层见证）随带收口
- **Rationale**: 分析深水区（日期类型 = 磁盘格式变更，依赖 MS11 函数层）；分发收口拆出后本 MS 只含分析能力单一成果，与加密/分发互不阻塞，可独立验收
- **Scope**:

| Task | 状态 | 目标 | 关键前置 | 关联 change |
|---|---|---|---|---|
| MS13-T01 | **completed**（2026-09-23，Iter 000） | 日期/时间类型（Value/tuple/catalog/DDL/字面量/写入强制/比较/CAST/渲染/导入导出全链） | MS11（函数层） | `archive/2026-09-23-ms13-analytics-functions/`（Iter 000） |
| MS13-T02 | **completed**（2026-09-23，Iter 001 引擎侧 + Iter 002 CLI 侧） | 日期函数族 + INTERVAL 算术 + GROUP BY 表达式分桶（引擎）；`stats/sample/profile` 薄命令（CLI）；顺带 I043/I044 | MS13-T01 | 同上（Iter 001/002） |
| MS13-T03 | **completed**（2026-09-23，Iter 002） | I035：no-FROM SELECT——SingleRow 虚拟单行输入 + 拒绝面点名 | 无 | 同上（Iter 002） |

- **Non-goals**: 窗口函数（远期）；安装分发（MS14）；密钥/加密（MS12）；strftime/to_date 族；时区/TIMESTAMPTZ/TIME；INTERVAL 作存储列类型；日期键控（非 Int 键列走 MS16 路由回退）
- **Workload**: 单聚合 change 3 Iteration（类型底座 / 函数与分桶 / no-FROM 与 CLI 命令 + 收尾）
- **Stable baseline**: 日期列可存可查可过滤可恢复；`GROUP BY date_trunc(...)` 三引用形态分桶等价；`SELECT 1+1` 单行可达；stats 输出行数/null 率/distinct/min/max/分位数
- **Verification boundary**: `tests/datetime_type_test.rs`（16）+ `tests/datetime_function_test.rs`（19）+ `tests/group_by_expr_test.rs`（12）+ `tests/no_from_select_test.rs`（9）+ cli_test 三命令组（13）+ 全量零修改（T13 收口）
- **Diagnostic boundary**: `src/executor/{datetime,function,predicate,aggregate,single_row}.rs` + `src/parser/planner/{query,expression,ddl_dml}.rs` + `src/storage/{page_format/tuple,catalog,error}.rs` + `src/cli/{mod,lifecycle}.rs`
- **Split signals**: 无（单 change 收口，未触发拆分）
- **Related changes**: `2026-09-23-ms13-analytics-functions`（T01/T02/T03 聚合 change，已归档为 `archive/2026-09-23-ms13-analytics-functions/`，含新增 specs `datetime-type-system`（7 Requirement + R7 校准段）、`datetime-functions`（4）、`group-by-expression`（2 + R2 校准段）、`no-from-select`（3）、`cli-analytics-commands`（4）与修改 spec `sql-scalar-functions`（R1/R3），明细见各 spec；**1050 tests pass / 0 failed / 2 ignored**；clippy/fmt/validate 全 0/PASS；归档期注记（WHERE 算术腿见证、doc 勘误、RTM 注记）见归档 carrier）

### MS14：分发收口（初版可获取） — superseded（2026-09-23 用户裁定裁剪合并入 MS17）

- **Status**: superseded（2026-09-23 用户裁定分发形式收窄——原 T01 不按原形实施：CI workflow / GitHub Releases 预编译矩阵 / crates.io 元数据（`cargo install`）/ man 页生成退 improvements 域（I051/I052/I053/I058 待登记，用户裁定「CI 没必要、Release 也没必要、暂不发布」）；completions 以收窄形态（`rtsql completions <shell>` 隐藏子命令 + 一键安装脚本安装，man 不做）并入 **MS17-T03**；★初版达成★ 标记由 MS17 承接；随带 I048 并入 MS17-T02）
- **Superseded by**: MS17
- **替代时点**: 2026-09-23
- **替代原因**: v0.1 可获取性改由「本机一键编译安装脚本」承载（原 Non-goals 中「install 脚本按需求再议」转正为 MS17-T03）；预编译矩阵与 crates.io 为发布规模化项，待真实需求出现时按 I 候选重启（用户裁定「暂时不发布」）
- **Dependencies**: MS10（已满足；依赖关系由 MS17 继承）
- **原范围**（保留作历史）:

| Task | 目标 | 处置 |
|---|---|---|
| MS14-T01 | CI workflow + Releases 预编译矩阵（cargo-zigbuild 或 runner 原生）+ completions/man + crates.io 元数据（`cargo install` 可用）（原 MS13-T03 全部内容） | CI / Releases / crates.io / man → I051/I052/I053/I058；completions 收窄 → **MS17-T03**；I048 → MS17-T02 |

- **Non-goals**: Homebrew tap；deb/AUR；Windows（FileExt 限 Unix 需重写页 I/O 层，MS17 沿用 out of scope）；密钥/加密（原 MS12 → MS17）；分析函数（MS13 已完成）
- **Related changes**: None

### MS15：初版前正确性收口 — completed（2026-09-12）

- **Status**: completed（T01 独立 change + T02-T04 聚合 change `2026-09-12-ms15-rest-correctness-batch`，2026-09-12 全部完成；用户 2026-09-11 批准为初版前必做）
- **Dependencies**: MS10（缺陷全部在 MS10-T04/T05 与 MS11-T03 验收面发现）
- **Outcome**: 4 项用户可见正确性缺陷清零——键位等值不再静默漏行、CLI 表头与行形状一致、UPDATE 键位置 NULL 不再唯一性误拒、多代 dump/restore 表名保真
- **Rationale**: 首次对外发布不带已知静默错误结果。4 项均为小改动但分属 planner 路由 / CLI 渲染 / update 执行器索引维护 / dump 命名面，聚合为同一成果「初版前正确性积压清零」；每项独立 change 独立验证，单项失败不阻塞其余
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS15-T01 | **completed**（2026-09-12）I036：键位等值过滤对无键行可达（不可键控字面量禁用索引路由，消除静默漏行） | MS10-T05 001-rework + MS11-T03 双重实证（improvements I036，已 promoted）；change `archive/2026-09-12-ms15-t01-keyless-eq-routing/` |
| MS15-T02 | **completed**（2026-09-12，聚合 change Iter 000）I034：裸 DataScan 子集投影 CLI 表头与行形状一致（`get_plan_output_columns` scan 臂应用 projection 裁剪表头） | MS10-T04 Plan Review finding（improvements I034，已 promoted） |
| MS15-T03 | **completed**（2026-09-12，聚合 change Iter 001，经 001-replan 校准）I037：UPDATE 键位置 NULL/非 Int 后旧键索引条目清理（Step 7 分支化，消除唯一性误拒与两态不一致） | MS10-T05 001-rework Plan Review（improvements I037，已 promoted） |
| MS15-T04 | **completed**（2026-09-12，聚合 change Iter 002，经 001-rework 收口转义名 dump）I039：dump/restore 表名保真（解析侧归一化，11 处消费点去引号 + `select_all_rows` 经 `quote_ident`） | MS10-T05 Iter001 Act（improvements I039，已 promoted） |

- **Non-goals**: I035 no-FROM SELECT（能力扩展，初版后）；I032/I038（无当前正确性影响）；I041/I042/I044 测试加固（随相应面顺带）；I043 标量函数极端输入（需用户裁定语义方向）；性能优化
- **Workload**: 4 个独立小 change（planner 路由 / CLI 渲染 / update 执行器 / lifecycle DDL 生成）
- **Stable baseline**: 4 项各有 RED→GREEN 测试见证；既有 845 全量零回归；`tests/keyless_row_test.rs` 与 dump/restore 往返在新语义下全绿
- **Verification boundary**: 每项独立测试 + 全量回归零修改通过
- **Diagnostic boundary**: planner query 路由（`src/parser/planner/query.rs`）/ CLI 渲染（`src/cli/render.rs` + `get_plan_output_columns`）/ `src/executor/update.rs` / `src/cli/lifecycle.rs` DDL 生成
- **Split signals**: 单项实施触发磁盘格式变更或跨子系统重构时，拆出独立 MS
- **Related changes**: `2026-09-12-ms15-rest-correctness-batch`（T02/T03/T04 聚合 change：3 Iteration 各 1-2 Cycle 全部 accepted，2026-09-12 归档，spec `cli-noninteractive-shell`「扫描执行器真投影」修改 + 新增 `update-index-maintenance` / `table-name-resolution`；I034/I037/I039 转 promoted，范围外登记 I047/I048 + I033 证据强化）；T01 关联 change 见上表 `archive/2026-09-12-ms15-t01-keyless-eq-routing/`

### MS16：正确性收口第二批 — completed（2026-09-13）

- **Status**: completed（聚合 change `2026-09-12-ms16-correctness-batch` 单 change 收口，2026-09-13；T01/T02 对应 I046/I047；范围按用户裁定扩入键列写入类型强制（调查探针新发现，不登记直接修复，proposal 决策记录 1）与 INSERT 列清单映射（Plan Review BH-2 裁定并入，裁定记录 6）；2 Iteration 三轮 Review 两 replan 后全部 accepted，BH 校准明细见归档 carrier）
- **Dependencies**: None（I046/I047 均为 MS15 已实施面上的已实证残差，路由判定点与索引数据面已在 d8a244f 落地）
- **Outcome**: 键位等值过滤对全类型键列形态可达（Float 键列 + Int 字面量不再经 IndexScan 静默漏行）；Int 键列越界写入显式拒绝（KeyTypeMismatch，零副作用）；INSERT 列清单恰为表列排列时值正确落位、非法清单计划期拒绝（panic 消除）；rekey 后索引条目与数据页两态一致（新键点查可达、旧键点查空集、旧键 INSERT 不再误拒、碰撞写入前拒绝、恢复两态一致）
- **Rationale**: MS15 收尾实证的最后一组用户可见「静默错误结果」类缺陷（两项均有探针/代码级证据）；同属「索引可达性/一致性」故障域，聚合为一个阶段成果，先行于一切新能力——分发虽后置，正确性纪律不变
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS16-T01 | **completed**（2026-09-13）I046：键列类型感知路由（方向 B）——planner `register_table` 加性传递列类型，键列非 Int 时键位等值形态统一回退 DataScan（MS15-T01 判定点即落点，扩展不冲突） | MS15-T01 调查新发现（improvements I046，promoted） |
| MS16-T02 | **completed**（2026-09-13）I047：rekey 判定（新旧键均可键控且不等）改为删旧键条目 + 插新键条目；新键撞已有行 DuplicateKey 拒绝（与恢复侧重建重复 PK 显式报错语义一致） | MS15-Rest 调查新发现（improvements I047，promoted） |

- **Non-goals**: I033/I032（→MS09 事务可见性域）；I038/I031/I021（→MS08 实测域）；I035（能力扩展→MS13 小项）；I042/I044/I048 随带加固项；性能优化
- **Workload**: 2 个独立小 change（planner 路由 / update 执行器索引维护）；另建议 I041（resolve env 测试竞态，全量约 1/6 假失败源）同期以独立小 change 顺带消除（测试基建，不入本 MS 验收面）——实际实施为单聚合 change（用户裁定探针新发现与审计裁定项并入直接修复）
- **Stable baseline**: 两项各有 RED→GREEN 测试见证；既有 867 全量零回归；键位等值过滤全形态行集正确、索引条目运行期/恢复两态一致
- **Verification boundary**: 每项独立测试 + 全量回归零修改通过
- **Diagnostic boundary**: planner query 路由（`src/parser/planner/query.rs` + `register_table` 类型传递面）/ `src/executor/update.rs`
- **Split signals**: 单项实施触发磁盘格式变更或跨子系统重构时，拆出独立 MS
- **Related changes**: `2026-09-12-ms16-correctness-batch`（T01/T02 + 键列写入类型强制 + INSERT 列清单映射聚合 change，已归档为 `archive/2026-09-12-ms16-correctness-batch/`，修改 spec `planner-key-equality-routing`（R2 收窄至 Int 键列 + 新增「键列类型感知路由」）、`update-index-maintenance`（新增「键位 rekey 后索引条目一致」，含 BH-3 校准段）+ 新增 `key-column-type-conformance`（1 Requirement 7 场景，含 BH-1 校准段）与 `insert-column-list-mapping`（1 Requirement 6 场景），明细见各 spec；I046/I047 转 promoted；三轮 Plan Review（两 replan-required 后 accepted）；**892 tests pass / 0 failed / 2 ignored**（Plan Review 独立复跑）；规划依据 tasks MS16-T01/T02 + proposal 探针实证 + BH-2/BH-3 审计裁定）

### MS17：初版分发收口（最小加密 + 可安装面 + 预存缺陷清账） — completed（2026-09-24，★初版达成★）

- **Status**: completed（T01/T02/T03/T04 全部完成，2026-09-23～2026-09-24；★初版达成★。两个聚合 change 均完成归档：T02 缺陷清账 + T01/T03/T04 最小加密/安装面/双语文档）
- **Dependencies**: MS10（已完成——CLI 是密钥入口载体与安装面载体；MS12 排序理由「tuple 格式扩展定稿后再上加密」经 MS13 完成已满足）
- **Outcome**: 数据库文件可选整库加密可用（Argon2id KDF + 页级 AES-256-GCM，SQLCipher 模型——格式头 flag 区分明/密库、明文库行为零回归、错误密钥显式拒绝；密钥通道 `--key` 与 `RTSQL_KEY` 两条）；ISS01/ISS02/ISS03 + I041 + I048 五项预存缺陷清账；`rtsql completions <bash|zsh|fish>` 隐藏子命令 + 一键编译安装脚本（build --release / strip / 安装 / completions 安装 / `--no-completions` 开关 / PATH 提示 / `--uninstall` 卸载两模式〔只清理程序 / 带数据清理须显式 flag〕）；双语 README（中英）；agent 用 SKILL.md 说明书（面向 AI agent 的安装部署/使用管理/卸载操作手册，2026-09-23 用户指令增补）
- **Rationale**: 用户裁定 v0.1 交付形式 = 「本机一条命令可编译可安装 + 最基本的加密 + 有一份能读的文档」；三项独立故障域（加密格式 / 缺陷清账 / 安装面文档）合并依据为同一验收主题「初版可交付状态」（MS15/MS16 批处理先例）；缺陷清账与安装脚本同属分发前质量面，加密是其中唯一重活
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS17-T01 | **completed**（2026-09-24，change Iter000）最小加密落地：64B 头激活加密位/32B 盐/12B KDF 参数，Argon2id（19456/2/1）+ 4096B 页 AES-256-GCM 4124B 记录（12B nonce + tag，AAD=page_id），`--key`/`RTSQL_KEY` 全开库命令面，错误密钥/明密互斥/无钥/损坏页 exit 5；明文路径零回归；spec `database-encryption` | `2026-09-23-ms17-initial-release` |
| MS17-T02 | **completed**（2026-09-23，聚合 change 含用户裁定追加的 RC 重启水位 Iteration 002）ISS01（min_create_tx_id=0 毒化 + MAX 毒化，`MIN_CREATE_UNKNOWN` 哨兵语义双向闭合）+ ISS02（IN×JOIN 计划期诚实拒绝，`InSubqueryJoinUnsupported`）+ ISS03（SubqueryEval 表头按 `result_column_index` 插列）+ I041（resolve env 测试合并单测试，结构性消除竞态）+ I048（import 表名 `quote_ident` 包裹）；范围扩展两处经用户裁定：MAX 毒化并入（proposal Gate 1）、RC 重启可见性回归追加 Iter 002（proposal 用户决策 5） | 均为已实证小项；ISS 正文自带修复方向候选，change 调查时定稿 |
| MS17-T03 | **completed**（2026-09-24，change Iter001）安装面落地：隐藏 `rtsql completions <bash\|zsh\|fish>`；`install.sh` 支持 build/strip/prefix/当前 shell 补全/`--no-completions`/PATH 提示与 `--uninstall [--purge-data]` 两模式；spec `install-script` | `2026-09-23-ms17-initial-release` |
| MS17-T04 | **completed**（2026-09-24，change Iter002）英文 `README.md`、中文 `README.zh-CN.md` 与英文 agent 手册 `rtsql-docs/SKILL.md` 落地；安装/卸载、CLI、SQL/引擎能力、加密、限制与互链均经实际命令核对 | `2026-09-23-ms17-initial-release` |

- **Non-goals**: CI workflow（I051）；GitHub Releases 预编译矩阵（I052）；crates.io 发布（I053，用户裁定暂不发布）；TTL 密钥缓存（I054）；`rtsql key` 子命令（I055）；`--password-file` 通道（I056）；正式加密性能 bench 基线（I057）；man 页（I058）；Homebrew/deb/AUR；Windows（页 I/O 层限 Unix）；密钥轮换；多用户权限；REPL；性能优化域（I021/I024/I026/I031/I038/I049/I050 不变）
- **Workload**: 实际 2 个聚合 change——`2026-09-23-ms17-t02-defect-closeout`（T02）与 `2026-09-23-ms17-initial-release`（T01/T03/T04，3 Iteration）
- **Stable baseline**: 加密库与明文库行为等价（既有全量零修改；错误密钥打开显式失败 exit 5）；ISS 五项各有 RED→GREEN 见证；`install.sh` 在 Linux/macOS 本机一条命令完成安装且 `rtsql --version` 与基础 CRUD 可用；卸载两模式行为正确（只清理程序后 `rtsql` 不可用而数据目录保留；带数据清理仅经显式 flag 触发并正确清除 `RTSQL_HOME` 目录）；README 与 agent SKILL.md 说明书均与实现一致
- **Verification boundary**: 加密 BDD 场景（错误密码拒绝/损坏密文检测/明密互斥/零回归）+ 五缺陷独立测试 + completions 子命令测试 + 脚本冒烟（本机安装 + CRUD + 补全装载 + 卸载两模式各一轮：程序卸载后数据仍在、带数据清理后目录消失）+ 全量 `--no-fail-fast` 零回归（I041 清账后假失败源消除，全量门稳定）
- **Diagnostic boundary**: `src/storage/file_storage.rs`（加解密层）+ 文件头 + `src/cli/{mod,lifecycle}.rs`（completions 子命令）+ `install.sh`（新）+ 各缺陷对应诊断面（ISS 正文各带位置级指控）
- **Split signals**: 加密实施牵动 tuple 格式或范围超预期时，拆出独立 MS（脚本/README/缺陷清账独立成立）；ISS02 两缺口裁定引发子查询域大改时单独成 change，不阻塞其余
- **Related changes**: `2026-09-23-ms17-t02-defect-closeout`（T02 聚合 change，已归档为 `archive/2026-09-23-ms17-t02-defect-closeout/`，含新增 spec `in-subquery-join-rejection`（2 Requirement：JOIN 形态诚实拒绝 / 既有 IN 零回归）与修改 specs `cli-noninteractive-shell`（+「标量子查询输出列的表头形状」，13 Requirement）、`table-name-resolution`（+「import 表名实参转义可达」，4 Requirement）、`mvcc-tombstone-visibility`（+「页级可见性摘要无毒化（哨兵语义）」，6 Requirement）、`transaction-isolation-levels`（+「RC 重启后可见性高水位健全（checkpoint 水位持久化）」，4 Requirement）；**1065 tests pass / 0 failed / 2 ignored**；3 Iteration 各 1 Cycle 全部 Plan Review accepted；checkpoint 位点 16B→24B 携带 tx watermark（design D8 健全性推演）；clippy/fmt/validate 全 0/PASS）
- `2026-09-23-ms17-initial-release`（T01/T03/T04 聚合 change，已归档为 `archive/2026-09-23-ms17-initial-release/`：3 Iteration 各 1 Cycle 全部 accepted——000 最小加密（Argon2id + 页级 AES-256-GCM + `--key`/`RTSQL_KEY` + exit 5）、001 completions/install.sh、002 双语 README + agent SKILL；新增 specs `database-encryption` / `install-script`，修改 `cli-noninteractive-shell` / `database-file-format-header`；**1101 tests pass / 0 failed / 2 ignored**；clippy/fmt/validate 全 0/PASS；WAL/checkpoint 加密转 I060）
- **消耗 ISS**: ISS01、ISS02、ISS03（T02 已实施修复——ISS01 含 MAX 毒化一并闭合 + RC 重启水位回归；台账状态翻账〔修复完成〕由 Recorder 按用户指令落账）

### MS18：one-shot 路径性能收口 — planned

- **Status**: planned
- **Dependencies**: None
- **Outcome**: 零写入会话 `close()` 不再全价执行 checkpoint（I062——WAL 未决记录为零/低于阈值时跳过重写，「close 即落盘」语义不变）；CLI runtime 规格经 RSS/时延/吞吐三组实测定形（I063——必要时切 `current_thread`）；one-shot 固定开销向 SQLite 量级收敛（实测基线：`SELECT 1` 10.8ms vs SQLite 1.15ms、RSS 16.7MiB vs 4.1MiB）
- **Rationale**: agent/脚本高频 one-shot 是本数据库的主消费形态，固定开销是当前最实测可得的最大体验差距（README 对比板块 + R26 runbook）；两项同属 open/close 生命周期资源开销域，共享时延/RSS/吞吐验证边界，聚合成单一阶段成果「one-shot 固定开销收敛」
- **Scope**:

| Task | 目标 | 缺陷依据 |
|---|---|---|
| MS18-T01 | I062：checkpoint 前检查 WAL 未决记录，为零/低于阈值时跳过重写直接返回（`WalWriter` 已有文件长度查询） | improvements I062（2026-09-24 实测登记） |
| MS18-T02 | I063：CLI runtime flavor 评估（multi_thread 默认 vs current_thread），以三组实测数据决定是否切换 | improvements I063（2026-09-24 实测登记，先量化再决定） |

- **Non-goals**: 引擎级分配器（I066→MS22 候选）；Server 面 runtime；扫描流式化（I065）；WAL fsync 合并（I049）与 writev 批量写回（I026）→MS22；加密性能 bench（I057）
- **Workload**: 1-2 个小 change（close 路径一项 + runtime 评估一项，可合可分）
- **Stable baseline**: `rtsql <db> "SELECT 1"` 时延与 RSS 较基线显著下降且行为不变；写入会话 close 仍完整 checkpoint；全量测试零修改通过
- **Verification boundary**: R26 runbook 时延/RSS 复测对比 + 写入会话落盘语义回归 + 全量零回归
- **Diagnostic boundary**: `src/database.rs` close/checkpoint 接线 + `src/main.rs` runtime 构造
- **Split signals**: current_thread 切换牵动引擎内部 `spawn_blocking`（WAL/页 I/O）深改时拆出独立 MS
- **Related changes**: None

### MS19：CLI 管理面小收口（删库子命令 + 错误可操作化） — planned

- **Status**: planned
- **Dependencies**: None
- **Outcome**: `rtsql delete <db>` 删库子命令闭环（I061——复用 `resolve_existing_db`，删除主文件与 `.wal`/`.checkpoint` 伴生文件并报告释放结果，打开中经 advisory 锁显式拒绝）；`PlanError` 携带特性名与错误分类（I070——`UnsupportedStatement`/`UnsupportedExpression` 点名不支持的能力，协议错误码分类面评估）
- **Rationale**: 两项均为 CLI 应用层小项（各 1 个小 change），同属「日常管理与排错体验」主题——生命周期命令闭环 + 错误信息可操作；MS15/MS17 批处理先例，合并工作量适中且各项独立验收、单项失败不阻塞其余
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS19-T01 | I061：`rtsql delete <db>` 子命令（命令命名、dry-run/确认交互、路径形态边界随 change 调查定稿） | improvements I061（2026-09-24 用户方向） |
| MS19-T02 | I070：PlanError 特性名携带 + 错误分类（评估小改动面） | improvements I070（R18 主题 3） |

- **Non-goals**: REPL（I067）；备份配对验证文档（I072——可与 T01 同面顺带，届时裁定）；`install.sh --purge-data` 语义变更
- **Workload**: 2 个独立小 change
- **Stable baseline**: 生命周期子命令含 delete 闭环（`list` 可枚举、`delete` 可回收）；不支持语句/表达式报错直接可读特性名
- **Verification boundary**: delete 子命令独立测试（伴生文件清理/锁占用拒绝/路径形态/释放报告）+ 错误面快照测试 + 全量零回归
- **Diagnostic boundary**: `src/cli/lifecycle.rs`（+`resolve.rs`）与 `src/parser/planner` 错误构造面
- **Split signals**: 错误分类牵动 PG 协议层大改时拆出；delete 交互设计膨胀时裁剪为最小语义
- **Related changes**: None

### MS20：ATTACH 式跨库交互 — planned

- **Status**: planned
- **Dependencies**: None（建议序在 MS18 后——one-shot 收敛对多库 CLI 场景有益，非硬前置）
- **Outcome**: 多 `.db` 文件经 `ATTACH` 关联检索与物化可用（I059，用户点名方向）——一期：attach 注册表 + 「别名.表」命名空间解析（I039 落地的表名消费点扩展）+ 只读跨查 + `CREATE TABLE AS SELECT` 物化（`new`+attach+CTAS 组合覆盖「数据库视作表」全场景）；二期：跨文件 DML（每文件独立提交，非原子 v1 语义文档化）
- **Rationale**: 用户方向登记（2026-09-24）：两个及以上 db 文件关联检索、跨库增删查改、跨库视图物化；嵌入式正统设计（SQLite ATTACH 同型，PostgreSQL 单连接锁死单库为反例）；一期完成即有独立项目价值（跨库查询/抽取/合并/分析），二期依赖一期 attach 基础设施——同一阶段成果「跨文件关联检索与物化」分两期推进
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS20-T01 | 一期：attach 注册面 + 命名空间解析 + 只读跨查 + CTAS | improvements I059（2026-09-24 用户方向） |
| MS20-T02 | 二期：跨文件 DML（独立提交语义 + 边界文档化） | 同上 |

- **Non-goals**: 跨 attach 原子提交（master journal 级机制）；分布式/网络多库；每文件独立 RC 快照之外的隔离语义；crate 化「多文件会话」抽象（I074 排除域，若 I074 将来启动则顺势承接）
- **Workload**: 2-3 change（一期 attach+跨查+CTAS；二期 DML；视调查可合并）
- **Stable baseline**: 一期后跨库只读查询与 CTAS 物化 e2e 可用且单库行为零回归；二期后跨文件增删改可用且非原子边界明确；交叉 attach 死锁结构性不可能（try_lock 即 `DatabaseLocked` exit 4）
- **Verification boundary**: attach/命名空间/跨查/CTAS/跨文件 DML 独立测试 + 文件锁冲突场景 + 全量零回归
- **Diagnostic boundary**: `src/database.rs` attach 注册面 + planner 表名解析（`object_name_to_table_name` 消费点扩展）+ pipeline 执行器路由
- **Split signals**: 二期跨文件 DML 语义或锁交互超预期时拆出独立 MS，一期独立收口
- **Related changes**: None

### MS21：DDL 演进——ALTER TABLE 与二级索引 — planned

- **Status**: planned
- **Dependencies**: None（catalog 持久化通道由 MS07-T01 满足）
- **Outcome**: `ALTER TABLE ADD/DROP COLUMN` 与 `CREATE INDEX`/`DROP INDEX` 可用（I068）——列结构演进不再依赖 dump→改 DDL→restore 重建，非 PK 查询列可建二级索引；`CREATE VIEW`/`TRUNCATE`/`EXPLAIN` 届时一并裁定是否并入范围
- **Rationale**: SQL DDL 面最大功能缺口（R18 主题 2/3）；触及 catalog/序列化/重建路径的独立故障域，工作量中-大需独立阶段，与功能小项和性能批互不阻塞
- **Scope**:

| Task | 目标 | 依据 |
|---|---|---|
| MS21-T01 | CREATE/DROP INDEX 二级索引（catalog 登记 + 执行器 + 查询路由可达 + 恢复重建） | improvements I068（R18 主题 3） |
| MS21-T02 | ALTER TABLE ADD/DROP COLUMN（catalog/序列化/数据面演进） | 同上 |

- **Non-goals**: DECIMAL/BLOB 类型（I069 深水区另议）；代价模型与 Join 重排（I016）；在线 schema 变更的并发语义精细化
- **Workload**: 2-3 change（INDEX 与 ALTER 各自独立验收）
- **Stable baseline**: 非 PK 列可建索引且查询计划可达、drop/restart 后索引一致；加列/删列后数据与 schema 持久化往返一致
- **Verification boundary**: ALTER/INDEX 独立测试（含崩溃恢复两态一致）+ 全量零回归
- **Diagnostic boundary**: `src/storage/catalog.rs` + `src/parser/planner/ddl_dml.rs` + `src/storage/btree/` 索引管理与执行器
- **Split signals**: DROP COLUMN 触发全行重写格式变更过大时先收 ADD COLUMN + INDEX，DROP 另行评估
- **Related changes**: None

### MS22：实测驱动性能优化（第一批） — planned（范围量化定稿后转 ready）

- **Status**: planned（候选范围经 profile/bench 定稿后转 ready——沿用 MS08「先量化再决定」纪律）
- **Dependencies**: None 硬前置（建议序在 MS18 后——I062 先收 close/checkpoint 路径，避免与 I026/I049 同面冲突）
- **Outcome**: 经量化证实的性能积压按收益排序清收（第一批）。候选池：I049 WAL fsync 合并（组提交）/ I021 INSERT 多值批量 / I026 脏页 writev 批量写回 / I050 RowLockTable DashMap 化 / I024 Varint Key 变长编码 / I038 GC 无键行盲区 / I020 clone 消除 / I066 分配器评估（jemalloc/mimalloc）——每项先 bench/profile，达标立项，不达标退还 improvements 并留量化记录
- **Rationale**: MS08 实测驱动批处理先例的复活（2026-09-14 剥离时各项「先量化再决定」纪律随条目保留）；各项独立故障域但共享同一验收主题「实测证实的性能收益」，每项独立 change 独立验收，单项失败不阻塞其余
- **Scope**: 候选池见 Outcome；定稿范围以量化数据为准（规模：小-中 change × N）
- **Non-goals**: 撕裂树运行期根修（I031 中-大，量化后另行评估）、代价模型（I016）、扫描流式化（I065）、B+Tree 节点级锁（I025）、io_uring（I028）、瘦内部节点（I029）、合并 Tag byte（I030）——全部留 improvements 域
- **Workload**: 视定稿范围 N 个独立小-中 change，每 change 前后 bench
- **Stable baseline**: 定稿项各有 before/after bench 对比数据且全量零回归；未达标项有量化记录可退还
- **Verification boundary**: 每项 bench 对比（`--save-baseline` 纪律，R26/ms08-bench runbook）+ 全量测试零回归
- **Diagnostic boundary**: 各候选项在 improvements 条目自带诊断面
- **Split signals**: 定稿项 ≥4 或出现跨域大项（如 I031 入选）时拆第二批
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
- **jemalloc/mimalloc 优化 (K37)**：已转入 improvements 台账 **I066**（2026-09-24 用户指令，权威位置移至 `openspec/specs/improvements/spec.md`）

## 依赖关系图

```
MS00 → MS01 → MS02
                  ├→ MS03 [superseded → MS07+MS08]
                  ├→ MS04 [superseded → MS07+MS09]
                  └→ MS05 [superseded → MS08]
                                ↓
                              MS06 (completed)
                                ↓
                              MS07 (completed)
                 ┌──────────────┼──────────────────┐
                 ↓              ↓                  ↓
        MS09 (completed 2026-09-14，依赖放松)   MS10 CLI 非交互命令面 (completed)
        I033/I032 技术前置 MS07 已完成        ↓
              ┌──────────────┬────────────┼──────────────┐
              ↓              ↓            ↓              ↓
         MS12 加密      MS15 正确性收口  MS14 分发收口   MS11 表达式函数
  [superseded→MS17]     (completed)  [superseded→MS17]  (completed)
                                                             ↓
                                                     MS13 分析函数
                                              (completed 2026-09-23)
                                                             ↓
                                                   MS17 初版分发收口
                                      (completed 2026-09-24，★初版达成★——
                                       最小加密 + 安装面 + 文档 + 缺陷清账)

独立：MS16 正确性收口第二批 (completed 2026-09-13，无前置——I046/I047 为 MS15 已实施面残差)

初版后第一批（2026-09-24 规划，均无硬前置，建议序非依赖）：

```
MS18 one-shot 路径性能收口 (planned) ──┐
MS19 CLI 管理面小收口 (planned)         │
MS20 ATTACH 式跨库交互 (planned) ───────┼→ MS22 实测驱动性能批 (planned，
MS21 DDL 演进与二级索引 (planned) ──────┘   范围量化定稿后转 ready；建议序殿后，
                                            避免与 I062 同面冲突)
```
```

无环；所有依赖均已满足。MS17 于 2026-09-24 完成，MS12/MS14 由其承接的最小加密与本机安装面已交付。批准执行序与编号无关：**MS16（✅）→ MS09（✅）→ MS13（✅）→ MS17（✅，★初版达成★）**；既定执行序已全部完成，后续方向保留在 improvements 域。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

| 完成日期 | 内容 | commit |
|---|---|---|
| 2026-09-24 | RISC-V 64 musl 交叉构建产物（change `2026-09-24-riscv64-musl-build-artifacts`，单 Iteration 000 单 Cycle，Plan Review accepted；MS17 后续分发扩展）：新增根脚本 `build-riscv64-musl.sh`（固定 `riscv64gc-unknown-linux-musl`、一次收集式 preflight、子进程级 linker 注入 + `+crt-static` 静态 ELF、`cargo pkgid` 版本派生、staging→版本化 tar.gz+SHA256SUMS 安全替换 `dist/` 子目录、成功摘要声明 target execution not verified；脚本内 checksum 自引用校验按用户审计指令移除）+ `.gitignore` `/dist/` + 双语 README 交叉构建章节。宿主静态门全过；cross-build 实跑成功两次；RISC-V 目标机运行 SKIPPED（用户 waiver：无硬件且契约禁止）；Plan Review accepted（2 项非阻塞 Minor：成功后窄窗 backup 目录残留、spec R2 工具清单未列 bash 超集）；specs 39→40（新增 `riscv64-musl-build` 4 Requirement）。范围外共居工作流随统一提交入库：install.sh bashrc PATH 写入/卸载移除、docs/SKILL.md→rtsql-docs/SKILL.md 迁移、SNAPSHOT/tasks 路径勘误及两 README 对应 hunk | 本轮统一提交（提交基线 145bba4）；change 归档 `archive/2026-09-24-riscv64-musl-build-artifacts/` |
| 2026-09-24 | MS17 初版分发收口（T01/T03/T04 聚合 change `2026-09-23-ms17-initial-release`，3 Iteration 各 1 Cycle 全部 accepted）：最小整库加密（Argon2id + 4124B AES-256-GCM 页记录 + `--key`/`RTSQL_KEY` + exit 5）、completions + `install.sh`、英文/中文 README + `rtsql-docs/SKILL.md`；specs 37→39；**1101 tests pass / 0 failed / 2 ignored**；clippy/fmt/validate 全 0/PASS；MS17 completed，★初版达成★；WAL/checkpoint 加密登记 I060 | 本轮统一提交（提交基线 7364bc9）；change 归档 `archive/2026-09-23-ms17-initial-release/` |
| 2026-09-23 | MS17-T02 缺陷清账（聚合 change `2026-09-23-ms17-t02-defect-closeout`，3 Iteration 各 1 Cycle 全部 accepted）：000 surface-defects——I041 env 测试合并单测试（消除全量假失败源）+ ISS02 `PlanError::InSubqueryJoinUnsupported` 计划期点名拒绝（JOIN 不误报多列）+ ISS03 SubqueryEval 表头按 `result_column_index` 插列 + I048 import 表名 `quote_ident` 包裹；001 visibility-summary-closeout——`MIN_CREATE_UNKNOWN` 哨兵（ISS01 0/MAX 毒化双向闭合）；002 restart-watermark（用户裁定追加）——checkpoint 位点 16B→24B 携带 tx watermark（兼容读三分支 + `advance_past(max(WAL 观测, 位点水位))`，干净 close 重开 RC 立即见全部已提交行）+ T5 夹具 workaround 移除；specs 36→37 + 4 域修改；**1065 tests pass / 0 failed / 2 ignored**（表面批 6/哨兵 5/水位 4）；clippy/fmt/validate 全 0/PASS；Experience Candidates（多列 IN 首列静默语义、IN×JOIN 能力解锁）已报告待 Recorder/improvement 流程 | 未 commit（待用户触发；对照基线 7364bc9）；change 归档 `archive/2026-09-23-ms17-t02-defect-closeout/`；docs sync 本次写入 |
| 2026-09-23 | MS13 分析函数（聚合 change `2026-09-23-ms13-analytics-functions`，T01/T02/T03，3 Iteration 各 1 Cycle 全部 accepted）：000 datetime 类型底座（Value/ValueRef Date/Timestamp 全套 + tuple TAG/catalog COL_TAG + DDL 显式映射 + TypedString plan 期解析 + 写入强制解析 `InvalidDateTime` + CAST 矩阵 + compare_values 两臂）；001 函数与分桶（REGISTRY +10 + eval_scalar 臂 + I043 abs/round 边界 + I044 大小写见证 + INTERVAL 算术同日锚定 + GROUP BY 四级解析序与混合投影解锁）；002 no-FROM 与 CLI（`SingleRowExecutor` + 九项拒绝面 + `stats/sample/profile` 三命令）；specs 31→36 + sql-scalar-functions 修改 R1/R3；归档期注记（WHERE 算术腿见证、doc 勘误、RTM 注记、D14 row_count 核对）见归档 carrier；**1050 tests pass / 0 failed / 2 ignored**；clippy/fmt/validate 全 0/PASS | 未 commit（待用户触发；对照基线 7364bc9）；change 归档 `archive/2026-09-23-ms13-analytics-functions/`；docs sync 本次写入 |
| 2026-09-14 | MS09 引擎能力与 MVCC 收尾（聚合 change `2026-09-13-ms09-engine-mvcc-closeout`，T01/T02/T04，3 Iteration 7 Cycle 全部 accepted）：000——I033 墓碑 slot 化（`superseder_suppresses` 按删除者提交状态）+ I032（恢复 `mark_uncommitted_aborted`，no-op 移除）+ RC 隔离（经 001-replan D10 快照结构与 `advance_past`、002-rework 页级快路径修正）；001——NLJ 执行器 + `is_pure_equi_join_on` 启发式分流（经 001-rework 用例 8 见证改形裁定）；002——三执行器关联子查询语句级缓存 + 等价见证 8 用例；specs 27→31；**936 tests pass / 0 failed / 2 ignored**；clippy/fmt/validate 全 0/PASS；ISS01/ISS02 落账（ISS03 经 Iter002 Review 登记） | 7364bc9（用户统一入库 MS16 收尾 + MS09 规划与实施）；docs sync 同批，待用户 commit |
| 2026-09-13 | MS16 正确性收口第二批（聚合 change `2026-09-12-ms16-correctness-batch`，I046/I047 + 键列写入类型强制 + INSERT 列清单映射；2 Iteration，三轮 Review 两 replan 后 accepted）：000——路由类型门（`primary_key_types`/`set_pk_column_type` + `pk_type_known_non_int` 两判定门，非 Int 键列键位等值回退 DataScan）+ 写入类型强制（`KeyTypeMismatch` 零副作用前置校验）+ 列清单映射（`build_insert` 校验 + `map_insert_values` 重排）；001——rekey 前置碰撞预检 + Step 7 三分支（BH-1/BH-2/BH-3 校准见归档）；修改 specs `planner-key-equality-routing`/`update-index-maintenance` + 新增 `key-column-type-conformance`/`insert-column-list-mapping`；**892 tests pass / 0 failed / 2 ignored**（Plan Review 独立复跑）；clippy/fmt/validate 全 0/PASS；I046/I047 转 promoted | 未 commit（待用户触发；对照基线 d8a244f）；change 归档 `archive/2026-09-12-ms16-correctness-batch/`；docs sync 本次写入 |
| 2026-09-12 | MS15-Rest 初版前正确性收口第二批（T02/T03/T04 聚合 change，I034/I037/I039，3 Iteration 各经 1 轮 replan/rework 收口）：I034 `get_plan_output_columns` projected_columns（CLI 表头与行形状一致）；I037 UpdateExecutor Step 7 分支化（新值不可键控删旧键条目，两态一致）；I039 表名解析归一化 `object_name_to_table_name` 统一 11 处消费点 + dump 经 `quote_ident` 包裹（转义名多代往返恒等，经 001-rework 收口）；**867 tests pass / 0 failed / 2 ignored**（Plan Review 两轮独立复跑）；clippy/fmt/validate 全 0/PASS；I034/I037/I039 转 promoted + I033 证据强化（同根去重）+ 登记 I047/I048 | d8a244f（T01 + T02/T03/T04 实施 + 归档 + docs sync 单提交，对照基线 f9e1e1f） |
| 2026-09-12 | MS15-T01 I036 键位等值路由修复（单 Iteration 000）：`has_non_keyable_pk_literal_leg` 分类 helper + `has_pk_eq` 分支条件收窄，非键控字面量腿（String/Float/Bool/NULL）落入既有 OR/谓词下推臂，未新增第三条 plan 路径；形态 2（Int 字面量 + Float 键列）登记 I046 按批准范围保留；新增 `tests/keyless_eq_routing_test.rs` 8 测试（RED 5 failed/1 passed → GREEN 8 passed）；853 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑全量 + clippy/fmt/validate 全 0/PASS + CLI 三探针逐字节一致）；Plan Review accepted（4 偏差全非阻塞）；I036 转 promoted + 登记 I046；spec `planner-key-equality-routing` 合并 | d8a244f（与 MS15-Rest 实施 + docs sync 单提交入库） |
| 2026-09-11 | MS11-T03 标量函数库第一批（string 6 + math 4，2 Iteration 各 1 Cycle accepted）：`src/executor/function.rs` 单点注册表（`REGISTRY` 元数据 + planner 校验入口 `is_scalar_function`/`check_scalar_function` + `FunctionExpression`，D3 求值序/NULL 短路/严格类型）+ planner `Expr::Function` 臂 + Trim/Ceil/Floor 独立变体接线（`build_ceil_floor` 共享 helper，TO 形态点名拒绝）+ ast.rs 两放行门 + string 六函数（substr SQLite 边缘/trim 仅 U+0020）+ math 四函数（round 半数远离零/负 digits 整数位）；新增 `tests/scalar_function_test.rs` 28 + function.rs 18 单测 + cli_test 增至 56；845 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑）；两轮 Review accepted（偏差与 findings 见归档）；登记 I043/I044/I045 + I036 佐证 | 实施未提交（待用户触发；对照基线 179228b）；docs sync 同工作区待一并 commit |
| 2026-09-10 | MS11-T02 SQL 事务语句 BEGIN/COMMIT/ROLLBACK（2 Iteration 各 1 Cycle accepted）：000 lib 层——`TxStatementKind`/`classify_transaction_statement`（`build_plan` 前置分类：边界子句点名文案直达、干净事务语句 session-only 拒绝）+ `PlanError::TransactionStatement` + `TransactionSession`（session.rs，5 单测）+ lib 段 3 测试；001 CLI 层——`run_sql` 分类三臂分派（事务语句 `Affected(0)` 直渲染）+ `rollback_session` 覆盖全部错误路径 + 收尾回滚 stderr 提示 + `sql_failure_status` 事务上下文后缀；`tests/tx_statement_test.rs` 19 测试（3 lib + 16 CLI e2e）；797 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑 + 组合路径探针）；登记 I041/I042 | 179228b（实施，含 change 目录）；docs sync 本提交（归档 + spec 合并 + I041/I042 登记） |
| 2026-09-10 | MS11-T01 SQL 表达式四件套与值表达式（WHERE/SELECT，2 Iteration 各 1 Cycle accepted）：000——三值求值内核 `Ternary`/`evaluate_ternary`（短路保持式，既有行为逐字节等价）+ Like/IsNull/Not 谓词 + planner 七个 WHERE 臂（IN→OR 链/BETWEEN→AND/LIKE/IS NULL/NOT 脱糖；ESCAPE/TRY_CAST 拒绝）+ CASE/COALESCE/CAST + I040 负数字面量折叠；001——`ProjectionNode`/`ProjectionExecutor` 派生列 + SELECT 表达式项路由（AS 别名/Display 列名/四拒绝面）+ `building_subquery` 子查询抑制（R6 回归修复）；新增 expression_e2e 24 + projection_expression 16 + 追加 24（cli_test 增至 54）；769 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑 + 17 项二进制探针）；clippy/fmt/validate 全 0/PASS | 4813374（实施，含 change 目录）；docs sync 046a76c（归档 + spec 合并 + I040 promoted）；c468055（SNAPSHOT hash 记录勘误） |
| 2026-09-09 | MS10-T05 生命周期子命令（4 Iteration 双轮收口）：000 入口重构 Option 位置参数 + 六子命令分发 + `rtsql_home()/db_dir()` helper + new/list/schema（`create_table_sql` DDL 生成器）；000 001-rework 建库约束持久化（`create_table_with_constraints`，NOT NULL/UNIQUE 写入 catalog）；001 dump/restore/import（`sql_literal`/`csv_value` + 空库前置 + `-` stdin + fail-fast + 表头双向匹配）；001 001-rework 无键行落库语义（用户裁定方向 A——键位不可键控行落库不入索引 + 恢复 keyless 桶回退 `PkVersionMaps` 双桶）；新增 `tests/keyless_row_test.rs` 4 + 全形状往返；704 tests pass / 0 failed / 2 ignored（独立复跑 2 次）；clippy/fmt/validate 全 0/PASS；两轮 Plan Review accepted | b51985f（实施，含 change 目录）；docs sync 046a76c（归档 + spec 合并 + I036-I040/R20 登记） |
| 2026-09-09 | MS10-T04 多语句执行修复：`run_sql` 护栏移除 → `;` 分片逐条循环（canonical 文本缓存键 + 逐条 auto-commit + 顺序渲染）+ `sql_failure_status` 定位模板（k/n + 已提交提示，parse 错误透传）；`pipeline.rs` `execute_inner`/`execute_in_tx` 对 len>1 显式拒绝（替换静默 first() 截断）；cli_test 护栏用例重写 + 4 新增；671 tests pass / 0 failed / 2 ignored；clippy/fmt/validate 全 0/PASS；Plan Review accepted（6 非阻塞 finding，含裸 DataScan 表头 NEW-EVIDENCE→I034） | 8827700（实施，含 change 目录）；docs sync 归档 + 修改 spec `cli-noninteractive-shell`（R1 修正 + R5 替换为「多语句分片逐条执行」）；登记 I034/I035 |
| 2026-09-08 | MS10-T03 文件 magic/格式版本头：新模块 `src/storage/file_header.rs`（64B 布局 encode/decode 纯函数 + HeaderError 六分类；KNOWN_FLAGS_MASK=0——加密位拒绝至 MS12）+ `FileStorage::open` 接线（0 字节写头/分类校验先于页解析与 WAL 触碰）+ 3 处页偏移 +HEADER_SIZE + error.rs 三新变体（CLI exit 1，锁冲突 exit 4 优先）；新增 `tests/file_header_test.rs` 14（RED 基线 SIGABRT 134 消除）+ database_file_lock_test 1 + cli_test 4；665 tests pass / 0 failed / 2 ignored；clippy/fmt/validate 全 0；Plan Review accepted（4 非阻塞 finding，含 Guidance 掩码勘误） | 2eda010；change 归档至 `archive/2026-09-08-2026-09-08-ms10-t03-file-format-header/`；新增 spec `database-file-format-header`（4 Requirement） |
| 2026-09-08 | MS10-T02 跨进程文件锁 + 优雅停机 + Iteration 000 WAL 恢复引擎正确性（design D0/D7-D10）：try_lock 独占锁（`DatabaseLocked` 先于 WAL 打开，CLI exit 4）+ 两阶段 select 优雅停机（open/执行与信号竞争，信号臂 `close()` checkpoint，exit 130/143）；引擎收口——T0 reader 逐帧无歧义解析（修嗅探 derail 79/2046 帧）、T0b 位置寻址重放（修 10k 重开 13190≠10000）、G1-G3 B-Tree 规模缺口、R5 catalog root 同步、R6 DataScan 替代集合去重（运行期/恢复同源）、R7/R8 恢复期索引去信任 + 重放后重建（撕裂树根因：checkpoint 后页驱逐按 LRU 而非树拓扑，磁盘树含洞实测 184/10000）；6 Cycle 6 轮 Plan Review（2 次 rework 扩面经用户 Gate 2 批准）；636 tests pass / 0 failed（白名单清零） | 5855245；change 归档至 `archive/2026-09-08-2026-09-06-ms10-t02-file-lock-graceful-shutdown/`；新增 spec `database-file-lock`/`wal-recovery-frame-parsing`/`wal-recovery-replay-integrity`，修改 spec `cli-noninteractive-shell`（锁冲突 + 优雅停机）；登记 I031-I033 + K38 |
| 2026-09-06 | MS10-T01 CLI 壳 + 扫描执行器真投影（2 Iteration）：000 `src/cli/{mod,resolve,render}.rs` 新建 + main.rs 重写 one-shot 入口（裸名/路径解析、四格式、退出码 0/1/2/3 + 4/5 留位、多语句护栏、`close()` checkpoint、JOIN 三臂真表头）；001 真投影（6 plan 节点 + 6 执行器 `with_projection` 谓词/MVCC 判定后裁剪，修 IndexScan 表头错位/聚合静默 Null/排序失效，`tests/projection_test.rs` 6 测试锁定）；614 tests pass，clippy/fmt/validate 全 0；两轮 Plan Review accepted | 03ff1b9；change 归档至 `archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/`；新增 spec `cli-noninteractive-shell`（6 Requirement，R6=真投影） |
| 2026-09-05 | MS08-T01+T02 页 I/O 位置参数化 + 扫描预取：T01 `read_exact_at`/`write_all_at` 每页 1 syscall（strace 页路径 lseek 33→3；并发冷读串页损坏 RED→GREEN，4 测试）；T02 DataScan 后继页预取（`with_prefetch` 开关）——默认路径实测回退 +40~47% → replan 默认关、显式启用（3 测试；Review 第三轮 bench 两档 No change 回基线）；585 tests pass；clippy/fmt/validate 全 0；Plan Review accepted（T5.4 判读裁定环境侧 BASELINE-CHANGED 非阻塞） | dac6783；change 归档至 `archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/`；新增 spec `storage-io-optimization`（3 Requirement） |
| 2026-09-05 | MS07-T06 谓词/LIMIT 下推：`DataScanNode` + `predicate`/`scan_cap`；非 PK WHERE 无 OR 时谓词装入 DataScan（OR 保留 Filter），Limit 输入链恰为纯 DataScan 时 `offset+limit` 封顶（顶层 Limit 任何形状保留）；执行器两个行产出点接 `filter_row`/`yield_capped`（语义逐字对齐 filter.rs）+ `correlated.rs` 注入臂；`tests/pushdown_test.rs` 15 测试（577 tests pass；clippy/fmt/validate 全 0；Plan Review accepted） | 5d652a2；change 归档至 `archive/2026-09-05-2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown/` |
| 2026-09-05 | MS07-T05 Checkpoint 真正工作：`full_recover` 消费 16B 位点（无效安全退化全量）；K05 六处静默吞错显式化（`WalError::RedoFailed`，open 失败可见）；`rewrite_truncate` 单临界区原地截断（禁止 temp+rename）+ 九步 checkpoint 流程 + `Database` 接线/公开 `checkpoint()`/`close()` 自动触发；`tests/checkpoint_redo_reduction_test.rs` 9 测试（Plan Review accepted） | 0df2b93（与 T04 同提交） |
| 2026-09-05 | MS07-T04 显式事务：`Database::{begin,commit,rollback,execute_in_tx}` 公开 API；`tx_versions` 按表聚合 + 多表回滚（含墓碑，修回滚幽灵行）；`execute_in_tx`/`execute_stage_in_tx` 用户事务路径（隐式路径零变化）；`tests/explicit_tx_test.rs` 8 测试（Plan Review accepted） | 0df2b93 |
| 2026-08-30 | MS07-T03 planner 模块化拆分：`planner.rs`（2266 行）→ `src/parser/planner/` 6 模块（mod/query/expression/aggregate/subquery/ddl_dml）；`PlanBuilder` 三字段 pub(crate)；12 单测随函数迁移；API/re-export/SQL 语义零变化（planner_test 29 + executor_test 39 零修改全绿）；542 tests pass；clippy 0/fmt 0/validate 12 PASS；Plan Review accepted | 49a85ef；change 归档至 `archive/2026-08-30-2026-08-30-ms07-t03-planner-decomposition/`；新增 spec `planner-module-decomposition`（5 Requirement） |
| 2026-08-30 | MS07-T02 drop_table 物理页释放：`IndexManager::collect_all_pages`（栈式 DFS + visited 防环，pub async）；`drop_table` 重写（保留名→meta→catalog.delete→BTree/数据页收集→free-list）+ 私有 `collect_data_pages`（K22 链遍历）；`tests/drop_table_free_test.rs` 6 测试；542 tests pass；clippy 0/fmt 0/validate 11 PASS；Plan Review accepted | bd038da；change 归档至 `archive/2026-08-26-2026-08-26-ms07-t02-drop-table-physical-free/`；新增 spec `drop-table-physical-free`（7 Requirement） |
| 2026-08-26 | MS07-T01 系统表 `__tables`/`__columns` + Schema 页：新模块 `src/storage/catalog.rs`（~908 行/7 方法/10 单测，链式 SlottedPage + 页 0/1 保留 + 保留名检查）+ `IndexManager::from_root` + `TableManager::new` async/`open_or_init`/跨页 tail 同步 + `Database::close()` + InsertExecutor `Option<Arc<TableManager>>` + `AsyncStorage::page_count` + `StorageError::ReservedTableName`；`tests/schema_persistence_test.rs` 8 测试 + 14 个测试文件批量签名适配；534 tests pass；Plan Review accepted（11 偏差 0 阻塞；遗留 Minor 见 R16） | 4307a0e；change 归档至 `archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/`（R16 登记）；新增 spec `schema-persistence`（7 Requirement） |
| 2026-08-26 | MS06-T03 + T04：T03 WalWriter 持 `Arc<Mutex<File>>` 单一持久句柄（5 IO 方法去逐次 open；`tests/wal_handle_test.rs` 4 测试：fd 上界/LSN 偏移/truncate 追加/并发一致）；T04 `execute_inner` 279 行 → 编排器 + parse/plan/execute 三 pub stage + 8 阶段单测 + 三阶段 bench（516 tests pass；build/clippy 0 warning） | 未 commit（待用户触发）；change 归档至 `archive/2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages/`；新增 spec `wal-writer-handle-reuse` + `pipeline-stage-decomposition` |
| 2026-08-26 | MS06-T02 PlanCache DashMap + SQL 规范化：`HashMap + &mut self` → DashMap + `&self`；`normalize_sql_key`（ASCII 折叠 + 空白折叠 + trim + 单引号 toggle）；plan_cache 去 `Arc<Mutex<>>` 5 调用点；7 集成 + 10 单测；T0 clippy 归零 + 36 处表外 mechanical 修复（504 tests pass） | 未 commit（待用户触发）；change 归档至 `archive/2026-08-26-2026-08-25-ms06-t02-plancache-dashmap/`；新增 spec `plancache-key-normalization` |
| 2026-08-25 | 修复 DML `tx_id=0` 占位注入：pipeline 事务包裹 + Insert/Update/Delete WAL 唯一来源 + VersionHeader::commit 墓碑守卫 + 6 个新测试（487 tests pass） | 未 commit（待用户触发）；change 归档至 `archive/2026-08-25-2026-08-25-fix-dml-tx-id-injection/` |
| 2026-06-06 | BufferPool DashMap + miss Sem + per-page loading_locks + concurrent tests + bench | f64c874, b55a9a1, 5fc5494, fcaeb7c, faa87a4, ad90379 |
| 2026-06-04 | 页面级 MVCC DELETE mark_deleted + 惰性 set_all_visible + visibility benchmark | 78a3b01 |
| 2026-06-04 | DataScan 数据页直接遍历 + Planner 路由 + criterion bench | 6f1d00f, b9b9a08, 602f8fe |
| 2026-06-03 | 零拷贝 ValueRef 闭包 API + 集成测试 | 73076ac, 95bb3f9, b75d307, bf4cbc1 |
| 2026-06-03 | 零拷贝 SlottedPageRef 闭包方案（多次失败后最终设计） | （多个） |
| 2026-06-03 | 事务 ID AtomicU64 实施 | 634764d, ee9ceee |
| 2026-06-03 | 网络 BufWriter + TCP_NODELAY | （多个） |
| 2026-06-03 | 连接并发 Semaphore | （多个） |

## 与 OpenSpec Changes 同步

- 每个 MSxx 内的 MSxx-Txx 实施时通过 `openspec/changes/<date>-<t-tag>/` 创建 change
- 完成的 change 通过 `openspec archive` 归档
- 归档的 change carrier 保持不可变
- 新发现的问题写 `openspec/specs/improvements/spec.md` (Ixx) — **注意**：Ixx 编号待重新审视，旧 Ixx 多数已重新归位到 MSxx-Txx
- 完整迁移的旧版 entry 记录在 `.claude/legacy/2026-08-25-openspec-init-migration/COVERAGE.md`
