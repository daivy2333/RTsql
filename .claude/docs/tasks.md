# tasks — 任务与里程碑路线

> 最后更新：2026-09-13（maintainer MS16 收尾：聚合 change 归档 `archive/2026-09-12-ms16-correctness-batch/`——I046 键列类型感知路由 + I047 rekey 索引一致性 + 键列写入类型强制（调查探针新发现并入）+ INSERT 列清单映射（BH-2 并入），2 Iteration 三轮 Plan Review 全部 accepted，892 tests / Plan Review 独立复跑；specs 25→27（新增 key-column-type-conformance / insert-column-list-mapping，修改 planner-key-equality-routing / update-index-maintenance）；I046/I047 转 promoted；**MS16 completed**，执行序下一站 MS09；前次 2026-09-12：milestone-planner 路线重排（分发后置——MS14 移至执行序末位；新建 MS16 置顶；MS09 依赖放松并扩入 I033/I032 前移；MS13 扩入 I035 小项 + I043/I044 随带；MS08 扩入 I031/I038/I021 实测候选；I018/I027 批准归档、I021 归属 MS08）+ MS15-Rest 收尾（I034/I037/I039 三项正确性缺陷清零，**MS15 completed**，867 tests / specs 25，I034/I037/I039 转 promoted + I033 证据强化 + 登记 I047/I048）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

17 个 Milestone：9 completed（MS00-MS02 历史 + MS06/MS07/MS10/MS11/MS15/MS16）+ 3 旧 superseded + 5 planned（MS09、MS13、MS12、MS08 剩余、MS14 分发后置）。

规划理念：**先收口正确性 → 建设基础能力 → 实测驱动性能 → 引擎能力收尾 → 应用层可用好用（非交互 CLI / 密钥 / 分析 / 分发）**（2026-09-06 依据 R18 分析扩展应用层轨道）。2026-09-11 按「尽早达成 CLI 数据库初版」重排：分发收口自 MS13 拆出前移（MS14），初版前收口 4 项正确性缺陷（MS15），分析函数/加密/性能后置。2026-09-12 再调整（用户裁定「暂时不分发，先把工作做完再谈分发」）：MS14 后置至执行序末位，新建 MS16 承接 MS15 残差置顶，MS09 扩入 MVCC 域正确性收口（I033/I032）后前移。

执行顺序（用户批准 2026-09-12 重排，取代 2026-09-11 序）：**MS15 正确性收口（✅ 2026-09-12 完成）→ MS16 正确性收口第二批（✅ 2026-09-13 完成）→ MS09 引擎能力与 MVCC 收尾（下一步）→ MS13 分析函数 → MS12 加密 → MS08 剩余 T03-T06 → MS14 分发收口（后置，时点待工作完成后另行确定）**。★初版达成★ 标记随 MS14 后置移至末位。旧执行序（2026-09-11：MS15 → MS14 → ★初版达成★ → MS13 → MS12 → MS08 剩余 → MS09；及 2026-09-06 序）均已被本序取代。
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

### MS08：性能压测（实测驱动） — planned

- **Status**: planned（T01/T02 已完成，2026-09-05；T03-T06 待后续 change；2026-09-12 重排后执行序位于 MS12 之后，并扩入 I031/I038/I021 三项实测候选——全部先量化再决定）
- **Dependencies**: MS07（MS08-T03 依赖 MS07-T05）
- **Outcome**: 6 类微基准 baseline 落盘；每类优化要么量化改善、要么记录"未达预期"；建立"实施前先 `--save-baseline`"纪律；撕裂树根修/GC 无键链/INSERT 批量三项候选量化后决定实施或记录未达预期
- **Rationale**: 旧路线把性能优化按"假设收益"排列；零拷贝 ValueRef 实施教训（K18）证明未量化目标会重蹈覆辙。本 MS 强调实测驱动
- **Scope**:

| Task | 状态 | 目标 | 关键前置 | 关联 change |
|---|---|---|---|---|
| MS08-T01 | **completed**（2026-09-05） | `pread`/`pwrite` 替代 `seek+read` | 无（最简单 syscall 削减） | `archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/` |
| MS08-T02 | **completed**（2026-09-05） | Prefetch 双缓冲（默认路径实测回退 +17~47%，replan 后默认关闭、`with_prefetch(true)` 显式启用） | MS02-T02 (DataScan)、MS03 (BufferPool 优化) 已 done | 同上 |
| MS08-T03 | planned | 脏页 writev 批量写回 | MS07-T05 | — |
| MS08-T04 | planned | RowLockTable DashMap | **先做 mini-bench 决定是否值得做** | — |
| MS08-T05 | planned | Varint Key 编码 | 无 | — |
| MS08-T06 | planned | WAL fsync 合并 | **做前先验证 fsync 是否真瓶颈** | — |
| MS08-T07 | planned | I031 撕裂树运行期根修（候选）：结构感知刷盘/树快照/no-steal 驱逐，与 T03 同域 | **先量化撕裂增长与恢复代价再决定** | — |
| MS08-T08 | planned | I038 GC 无键链盲区（候选）：数据页链全扫模式或无键链登记结构 | **先量化含无键行表的版本空间增长再决定** | — |
| MS08-T09 | planned | I021 INSERT 批量执行（候选）：bulk_insert/append_batch | **先 bench 逐行路径占比再决定** | — |

- **Non-goals**: 新 SQL 方言；新执行器；多隔离级别；B+Tree 节点级锁；io_uring
- **Workload**: 每优化 1 change（Varint Key 可能 2 个），共约 5-7 change；每 change 前置 baseline
- **Stable baseline**: 性能基线档（`cargo bench --save-baseline before-MS08-T*` 落盘）；6 类微基准数据集；fsync 频率与延迟关联曲线
- **Verification boundary**: 每个 T 必须满足"前置 baseline 留档" AND "实施后某关键指标量化改善 OR 明确记录未达预期"
- **Diagnostic boundary**: 性能问题可定位到 bench 文件 / 数据规模 / commit hash
- **Split signals**: 若 MS08-T01 实施后 syscall 计数无显著变化，说明 syscalls 不是瓶颈，整个 MS 需重新平衡
- **Related changes**:
  - `2026-09-05-ms08-t01-t02-pread-prefetch`（T01/T02 合并 change，已归档为 `archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/`，含新增 spec `storage-io-optimization`，3 Requirement：R1 页 I/O 位置参数化 / R2 零接口零格式变更 / R3 DataScan 预取可选能力默认关闭）

### MS09：引擎能力与 MVCC 收尾 — planned（2026-09-12 重排前移；2026-09-06 重定义，原"SQL 标准与上层能力"）

- **Status**: planned（依赖已满足；2026-09-12 用户批准依赖放松并前移至 MS16 之后——本 MS 扩入 I033/I032 两项 MVCC 域正确性收口，先于新能力执行）
- **Dependencies**: None（原 MS08 为排序性依赖，2026-09-12 放松；T01/T02/T04 技术前置 MS07 已完成）
- **Outcome**: 隔离级别可配（Read Committed）且墓碑抑制语义同步收口（跨进程 update→delete 两步变更不再从扫描面消失）；NLJ/Hash Join 启发式切换；关联子查询结果缓存
- **Rationale**: 原四项中 PG Extended Query 被非交互 CLI 形态决策降级（无消费者，server 保留为库能力，将来 serve 复活时再议）；其余三项同属引擎能力面，与应用层轨道分开验收。2026-09-12 扩入 I033（update→delete 旧版本无快照重现，跨进程实证、原「未来混合负载」预判已现实化）与 I032（mark_tx_aborted 空实现）——两者与 Read Committed 同属事务可见性域且条目自注届时一并处理，故本 MS 前移承接「先把工作做完」
- **Scope**:

| Task | 目标 | 备注 |
|---|---|---|
| MS09-T01 | Read Committed 隔离 + I033 墓碑抑制语义（已提交墓碑抑制整条链 / 未提交墓碑不抑制、回溯前驱）+ I032 mark_tx_aborted 评估后实施或文档化 | 不含 Serializable / SSI；I033 方案（对照 WAL committed 集合 vs header 编码扩展）留实现调查裁定 |
| MS09-T02 | NLJ + 与 Hash Join 启发式切换 | **不含代价模型** |
| MS09-T04 | 关联子查询结果缓存 | 视 MS09-T02 完成后实际场景 |

- **Non-goals**: PG Extended Query（降级移出，2026-09-06）；Serializable / SSI；代价模型与 Join 重排；io_uring；B+Tree 节点级锁；clone 消除 Arc/Cow（待 MS08 完成后看真实数据再决定）；多用户权限
- **Workload**: 3-4 change（T01 含 I033/I032，视调查结果可独立成 change）；总工作量适中
- **Stable baseline**: Read Committed 跨并发可验证；跨进程 update→delete 序列扫描面不再重现已删行旧版本；NLJ 在小表上优于 Hash；关联子查询 N 行外层不重复执行子查询
- **Verification boundary**: 3 项独立测试套件 + I033 跨进程探针场景
- **Diagnostic boundary**: 3 个独立子系统（事务管理 / Join 执行器 / 子查询执行器）
- **Split signals**: 若 MS09-T01 实施发现 snapshot 与 RR 共享度过低，或 I033 方案牵动 WAL/header 格式演进过大，各自拆为独立 MS
- **Related changes**: None

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
  - `2026-09-09-ms10-t05-lifecycle-subcommands`（T05 生命周期子命令 + 两轮引擎收口（Iteration 000 001-rework 建库约束持久化通道——NOT NULL/UNIQUE 经 `create_table_with_constraints` 写入 catalog；Iteration 001 001-rework 无键行落库语义——键位不可键控行落库不入索引 + 恢复 keyless 桶回退，用户裁定方向 A），已归档为 `archive/2026-09-09-2026-09-09-ms10-t05-lifecycle-subcommands/`，修改 spec `cli-noninteractive-shell`（R1 子命令分发扩展 + 新增 Requirement：new/list/schema/dump-restore/import 共 5 个）、`wal-recovery-replay-integrity`（「重放保持 DML 语义」修改 + 新增场景「无键行 Update 崩溃恢复语义正确」）。规划依据：R20 分析 + R18 主题 7/主题 5；4 Iteration（000 000-initial→001-rework、001 000-initial→001-rework），两轮 blocked 均为引擎既有缺陷经 CLI 数据面暴露（约束丢弃链 / 键位静默丢弃），Plan Review 记录 PLAN-OMISSION 与 PLAN-INVALID 各一并关闭；704 tests pass / 0 failed / 2 ignored；登记 I036-I040）
  - `2026-09-08-ms10-t04-multi-statement-execution`（T04 多语句执行修复，已归档为 `archive/2026-09-09-2026-09-08-ms10-t04-multi-statement-execution/`，修改 spec `cli-noninteractive-shell`（R1 多语句语义修正 + R5 临时护栏退役、替换为 Requirement「多语句分片逐条执行」6 场景）。规划依据：R18 主题 2/3 + spec R5 原文承诺；单 Iteration 000-initial，Plan Review accepted（6 非阻塞 finding），含 S5 见证 SQL 经用户批准修订为含 FROM 等价（no-FROM SELECT 不受支持登记 I035；裸 DataScan 表头缺口登记 I034））
  - `2026-09-08-ms10-t03-file-format-header`（T03 文件 magic/格式版本头，已归档为 `archive/2026-09-08-2026-09-08-ms10-t03-file-format-header/`，含新增 spec `database-file-format-header`，4 Requirement：R1 头布局与生命周期 / R2 格式错误显式拒绝 / R3 打开顺序守卫 / R4 既有语义零回归。规划依据：R19 分析 + 用户决策 2026-09-08（exit 1 复用 / 旧无头文件统一拒绝 / 仅主库加头 / 不加 CRC）；单 Iteration 000-initial，Plan Review accepted，含 Guidance 掩码行勘误裁定——`KNOWN_FLAGS_MASK=0`（加密位拒绝至 MS12））
  - `2026-09-06-ms10-t02-file-lock-graceful-shutdown`（T02 + Iteration 000 WAL 恢复引擎正确性收口（4 个 Iteration、6 个 Cycle：000-initial → 001-replan → 002-rework → 003-rework → 004-rework → 001-lock-shutdown/000-initial），已归档为 `archive/2026-09-08-2026-09-06-ms10-t02-file-lock-graceful-shutdown/`，含新增 spec `database-file-lock`（R1 锁语义 / R2 生命周期）、`wal-recovery-frame-parsing`、`wal-recovery-replay-integrity`（各 2 Requirement）与修改 spec `cli-noninteractive-shell`（R1 锁冲突退出码 4 + 新增 Requirement 优雅停机 4 场景）。规划依据：MS10 稳定基线「kill 后 WAL 恢复 e2e」；design D0/D7-D10）
  - `2026-09-06-ms10-t01-cli-shell`（T01 + Iteration 001 真投影扩展（用户批准方向 B，超出 T01 原始范围的引擎级修复），已归档为 `archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/`，含新增 spec `cli-noninteractive-shell`，6 Requirement：R1 入口与主命令 / R2 名称解析 / R3 列名表头 / R4 输出格式 / R5 多语句护栏 / R6 扫描执行器真投影。规划依据：R18 `usability-gap-cli-form.md`）

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
  - `2026-09-10-ms11-t03-scalar-functions`（T03 标量函数库第一批 string 6 + math 4，已归档为 `archive/2026-09-10-ms11-t03-scalar-functions/`，含新增 spec `sql-scalar-functions` 6 Requirement：R1 注册与分派机制（大小写不敏感/arity/OVER-DISTINCT-FILTER-命名-通配符-零参点名拒绝/未知名文案不变/聚合互斥）/ R2 string 六件 / R3 math 四件 / R4 NULL 语义与嵌套 / R5 调用面与边界 / R6 零回归。规划依据：tasks MS11-T03 + R18 主题 7（用户 2026-09-10 裁定 4 项语义 + 2 项默认假设）；2 Iteration（000 注册机制与 string 函数 T1-T3 → 001 math 与调用面收尾 T4-T6）各 1 Cycle accepted；新增 `src/executor/function.rs` 单点注册表（REGISTRY 元数据驱动 planner 校验入口 `is_scalar_function`/`check_scalar_function` + `FunctionExpression` 分派求值，D3 求值顺序/NULL 短路/严格类型/D5 evaluate_ref 三先例模式）+ planner `Expr::Function` 臂扩展 + `Expr::Trim`/`Expr::Ceil`/`Expr::Floor` 独立 sqlparser 变体接线（`build_ceil_floor` 共享 helper，TO 形态点名拒绝）+ `ast.rs` 两放行门注册名/独立变体放行；新增 `tests/scalar_function_test.rs` 28 测试 + function.rs 18 单测 + cli_test 2 表头用例（增至 56）；845 tests pass / 0 failed / 2 ignored；clippy/fmt/validate 全 0/PASS；两 Iteration Plan Review 均 accepted（000 轮 7 偏差：R1/S1 spec 文案勘误 SELECT 位置实为 `Unsupported statement type`、ast.rs 放行门 PLAN-OMISSION 补齐、TRIM 独立变体接线、R4 abs 腿/R2 表头断言归属 001、R2/S5 表形规避预存缺陷（I036 同根佐证）；001 轮 5 偏差：CEIL/FLOOR 独立变体接线 PLAN-OMISSION 按 TRIM 先例补救、TO 拒绝锁、approx_constant scoped 豁免）；登记 I043/I044/I045 + I036 佐证）
  - `2026-09-10-ms11-t02-sql-transaction-statements`（T02 SQL 事务语句 BEGIN/COMMIT/ROLLBACK，已归档为 `archive/2026-09-10-ms11-t02-sql-transaction-statements/`，含新增 spec `sql-transaction-statements` 5 Requirement：R1 CLI 会话往返 / R2 边界子句显式拒绝（含 R2/S5 `AND NO CHAIN` AST 等价）/ R3 会话状态边界 / R4 非会话路径显式拒绝 / R5 零回归，与修改 spec `cli-noninteractive-shell`「多语句分片逐条执行」（事务上下文语义 + 未提交注明 + 收尾回滚，6→8 场景）。规划依据：tasks MS11-T02 + R18 主题 7（用户 2026-09-10 批准计划，裁定 5 项决策——仅 CLI 适用面 / AffectedRows(0) / 边界子句全拒绝 / 边界语义默认包 / NO CHAIN 按裸语句同义）；2 Iteration（000 lib 事务语句层 T1-T3 → 001 CLI 会话接线与 e2e T4-T6）各 1 Cycle accepted；新增 `TxStatementKind`/`classify_transaction_statement`（planner mod.rs，`build_plan` 前置分类）+ `PlanError::TransactionStatement` + `TransactionSession`（src/transaction/session.rs）+ CLI `run_sql` 会话分派（`execute_stage_in_tx` 接线 + `rollback_session` 全错误路径收尾 + `sql_failure_status` 事务上下文后缀）；新增 `tests/tx_statement_test.rs` 19 测试（3 lib + 16 CLI e2e）；797 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑 + 组合路径探针）；clippy/fmt/validate 全 0/PASS；登记 I041（resolve env 测试竞态）/ I042（边界子句 × 活跃会话组合路径覆盖））
  - `2026-09-10-ms11-t01-sql-expressions`（T01 SQL 表达式四件套与值表达式（WHERE/SELECT），已归档为 `archive/2026-09-10-ms11-t01-sql-expressions/`，含新增 spec `sql-expression-evaluation` 6 Requirement：R1 谓词四件套 / R2 三值语义 / R3 CASE-COALESCE-CAST / R4 SELECT 派生列 / R5 INSERT 负数字面量 / R6 零回归；I040 并入并标记 promoted。规划依据：R18 主题 7 + tasks MS11-T01（用户 2026-09-10 批准计划）；2 Iteration（000 WHERE 侧表达式 → 001 SELECT 派生列）各 1 Cycle accepted；两轮 Review 各修正 1 项 Plan 侧问题（Iter 000：R3 SELECT 断言归属 Iter 001 + spec R1/S3 笔误；Iter 001：`building_subquery` 子查询抑制为 T8 Preserve 必要面 + spec R6「19 种」与 design D4 矛盾勘误）；769 tests pass / 0 failed / 2 ignored）

### MS12：整库加密与 sudo 式密钥 — planned（安全域，2026-09-06 新增）

- **Status**: planned（依赖已满足；2026-09-12 重排后执行序位于 MS13 之后、MS08 之前——格式扩展定稿后再上加密包裹，避免加密库随 tuple 格式变动重测）
- **Dependencies**: MS10（CLI 是密钥入口载体；锁与密钥是不同错误类、不同退出码）
- **Outcome**: Argon2id KDF（密码+文件头随机盐）+ 页级 AES-256-GCM（SQLCipher 模型，加密可选——格式头 flag 区分明/密库，明文库保留）；密钥来源 `--key <arg>` / `--password-file <path>` / `RTSQL_KEY` 环境变量（终端 export 一次后续继承，sudo 式语义主路径）+ TTL 密钥缓存（派生密钥非明文，`~/.rtsql/keys/` 0600，过期重问）；`rtsql key set/remove/status` 子命令；`invalid key` 退出码 5
- **Rationale**: 密码必须落在整库加密上才有意义（明文文件下"打开验证"只是礼貌性门锁）；安全域独立成段——新依赖（argon2/aes-gcm）+ 密码学安全路径 + 磁盘格式变更，与 CLI 壳/函数层互不依赖，独立验收
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS12-T01 | 文件头加密 flag + Argon2id KDF + 页级 AES-GCM transform（FileStorage 读写路径） | MS10-T03（格式头） |
| MS12-T02 | 密钥来源三通道 + TTL 缓存 + `key` 子命令 | MS12-T01 |
| MS12-T03 | BDD：错误密码拒绝/损坏密文检测/明密互斥/缓存过期/性能基线（MS08 纪律） | MS12-T01/T02 |

- **Non-goals**: 多用户/角色权限（OS 文件权限即边界，R18 主题 6 结论）；列级加密；密钥轮换；key agent 常驻进程
- **Workload**: 加密依赖引入 + FileStorage 加解密层 + 密钥管理 CLI + 密码学面 BDD + bench 基线
- **Stable baseline**: 加密库与明文库行为等价（既有全量测试零修改）；打开延迟增量实测留档（Argon2 参数 = 打开延迟↔暴力破解权衡，专门决策）
- **Verification boundary**: BDD 场景全绿 + 全量回归 + bench 对比结论（改善或明确未达预期）
- **Diagnostic boundary**: `src/storage/file_storage.rs` 加解密层 + 密钥管理 CLI + 文件头
- **Split signals**: 页加解密开销 >10% 时评估硬件加速或格式层优化；Argon2 参数争议大时单独出决策项
- **Related changes**: None

### MS13：分析函数 — planned（2026-09-11 重定义：分发收口拆出为 MS14；2026-09-06 新增，原"分析函数与安装分发"）

- **Status**: planned（依赖已满足；2026-09-12 重排后执行序位于 MS09 之后、MS12 之前；2026-09-11 分发收口拆出为 MS14）
- **Dependencies**: MS10 + MS11（不依赖 MS12——加密与分析正交；不依赖 MS14——分发与分析正交）
- **Outcome**: 日期/时间类型与函数（类型系统扩展 + date_trunc/interval 等分析最高频维度）；`stats/sample/profile` 薄命令（底层聚合 SQL：行数/空值率/distinct/min/max/分位数）；no-FROM SELECT 常量查询可达（I035）
- **Rationale**: 分析深水区（日期类型 = 磁盘格式变更，依赖 MS11 函数层）；分发收口拆出后本 MS 只含分析能力单一成果，与加密/分发互不阻塞，可独立验收。2026-09-12 扩入 I035（SQL 便利小项，可裁）——T02 触碰 `function.rs` 注册表面时顺带 I043（abs 溢出/round 极端 digits 语义方向届时裁定）/I044 测试加固
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS13-T01 | 日期/时间类型（tuple.rs 格式扩展 + 序列化/解析） | MS11（函数层） |
| MS13-T02 | 日期函数（date_trunc/interval/now 等）+ `stats/sample/profile` 薄命令 | MS13-T01；触碰 `function.rs` 时顺带 I043/I044 |
| MS13-T03 | I035：no-FROM SELECT——planner 单行虚拟输入臂（常量表达式查询、无表探测可达） | 无（小项，独立可验） |

- **Non-goals**: 窗口函数（远期）；安装分发（拆出至 MS14，2026-09-11）；密钥/加密（MS12）
- **Workload**: 类型系统格式扩展 + 日期函数 + 3 薄命令 + no-FROM SELECT 小项
- **Stable baseline**: `stats` 命令输出分位数；日期列可存可查可过滤
- **Verification boundary**: 日期类型/函数独立测试 + stats 命令输出验证
- **Diagnostic boundary**: 类型系统层（`src/storage/page_format/tuple.rs`）+ stats 命令层
- **Split signals**: 日期类型实现超 2 change 时拆出独立 MS
- **Related changes**: None

### MS14：分发收口（初版可获取） — ready（2026-09-11 自 MS13 拆出，原 T03）

- **Status**: ready（依赖 MS10 已满足；用户 2026-09-11 批准为初版达成里程碑；**2026-09-12 用户裁定后置——执行序移至末位，先完成引擎/功能工作，分发时点待工作完成后另行确定**）
- **Dependencies**: MS10（CLI 壳稳定）
- **Outcome**: 初版 v0.1 可获取——GitHub Releases 预编译矩阵（x86_64-linux-gnu / x86_64-linux-musl 静态 / aarch64-linux / macOS aarch64+x86_64）+ `cargo install` crates.io 元数据 + clap 生成 completions/man + CI workflow
- **Rationale**: 初版的价值在"可下载可运行"，纯工程面不触引擎。自 MS13 拆出（2026-09-11 拆分审计：分发与分析是两个可分别验收的成果，中途稳定检查点 = 可安装的 v0.1，任一失败另一部分独立成立）
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS14-T01 | CI workflow + Releases 预编译矩阵（cargo-zigbuild 或 runner 原生）+ completions/man + crates.io 元数据（`cargo install` 可用）（原 MS13-T03 全部内容） | MS10 |

- **Non-goals**: Homebrew tap（Releases 稳定后再议）；deb/AUR/install 脚本（按需求再议）；Windows（FileExt 限 Unix，需重写页 I/O 层，明确 out of scope）；密钥/加密（MS12）；分析函数（MS13）
- **Workload**: CI workflow 文件 + 打包矩阵 + completions/man 生成 + crates.io 发布元数据；随带 I048（import 历史带引号表名边界——quote_ident 包裹或文档化，可裁）
- **Stable baseline**: 全平台二进制可下载可运行；`cargo install` 可用；completions/man 随发布产物提供
- **Verification boundary**: CI 矩阵各平台 smoke 测试（`rtsql --version` + 基础 CRUD）全绿
- **Diagnostic boundary**: CI workflow 文件 + 打包配置（cargo-zigbuild 参数 / runner 选择）
- **Split signals**: macOS 矩阵需专门修复时拆出独立 MS；crates.io 发布受阻时 CI/Releases 先行收口
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

- **Status**: completed（聚合 change `2026-09-12-ms16-correctness-batch` 单 change 收口，2026-09-13；T01/T02 对应 I046/I047；范围按用户裁定扩入键列写入类型强制（调查探针新发现，不登记直接修复，proposal 决策记录 1）与 INSERT 列清单映射（Plan Review BH-2 裁定并入，裁定记录 6）；2 Iteration——000（路由类型门 + 写入类型强制 + 列清单映射，经 001-replan 校准 BH-1/并入 BH-2）与 001（rekey 索引一致性，经 000-initial BH-3 阻塞 → 001-replan 校准 6 个依赖缺陷行为的既有直连执行器测试）——全部 Plan Review accepted）
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
- **Related changes**: `2026-09-12-ms16-correctness-batch`（T01/T02 + 键列写入类型强制 + INSERT 列清单映射聚合 change，已归档为 `archive/2026-09-12-ms16-correctness-batch/`，修改 spec `planner-key-equality-routing`（「可键控字面量路由保持」收窄至 Int 键列 + 新增「键列类型感知路由」）、`update-index-maintenance`（新增「键位 rekey 后索引条目一致」，含 BH-3 校准段）+ 新增 `key-column-type-conformance`（1 Requirement「Int 键列写入类型强制」7 场景，含 BH-1 校准段）与 `insert-column-list-mapping`（1 Requirement 6 场景）；I046/I047 转 promoted；2 Iteration 各 1-2 Cycle，三轮 Plan Review（000-initial replan-required、001-initial replan-required、两轮 001-replan accepted）；**892 tests pass / 0 failed / 2 ignored**（Plan Review 独立复跑）；规划依据：tasks MS16-T01/T02 + proposal 探针实证 + BH-2/BH-3 审计裁定）

## D-candidates（不归入当前 MS，待后续决定）

| 标题 | 不建议做的原因 | 何时重评 |
|---|---|---|
| 代价模型 + Join 重排 | 价值/复杂度不匹配 | 视 MS09-T02 完成后 join 性能数据 |
| B+Tree 节点级锁 | ~500 行业务代码；460 tests 未证明有争用 | 视 MS08 完成后索引争用数据 |
| clone 消除 Arc/Cow | 零拷贝 ValueRef 教训（K18）：先量化再优化 | 视 MS08 完成后 clone 频率 profiling |
| io_uring | 高风险低收益 | 视 MS09 完成后整体性能 |
| 瘦内部节点 | 依赖 Varint Key | MS08-T05 完成后单独评估 |
| 合并 Tag byte | 改动序列化格式 | 暂搁 |

## 长期方向（未规划具体里程碑）

- **io_uring 集成 (K36)**：Linux 5.1+ tokio-uring 批量提交
- **jemalloc/mimalloc 优化 (K37)**：减少 String/Vec 分配开销

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
        MS09 (planned，依赖放松)   MS10 CLI 非交互命令面 (completed)
        I033/I032 技术前置 MS07 已完成        ↓
              ┌──────────────┬────────────┼──────────────┐
              ↓              ↓            ↓              ↓
         MS12 加密      MS15 正确性收口  MS14 分发收口   MS11 表达式函数
         (planned)      (completed)   (ready，后置)    (completed)
                                                             ↓
                                                     MS13 分析函数
                                                     (planned)

独立：MS16 正确性收口第二批 (completed 2026-09-13，无前置——I046/I047 为 MS15 已实施面残差)
```

无环；所有依赖指向已存在编号。MS12/MS14 只依赖 MS10（已满足）；MS13 依赖 MS10+MS11（已满足）；MS09 依赖放松为 None（2026-09-12 用户批准——原 MS08 为排序性依赖，T01/T02/T04 技术前置 MS07 已完成）；MS16 无前置依赖。批准执行序与编号无关：**MS16 → MS09 → MS13 → MS12 → MS08 剩余 → MS14（后置）**（2026-09-12 用户批准，取代 2026-09-11 序）。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

| 完成日期 | 内容 | commit |
|---|---|---|
| 2026-09-13 | MS16 正确性收口第二批（聚合 change，I046/I047 + 键列写入类型强制 + INSERT 列清单映射）：2 Iteration——000（T1/T2 路由类型门：`primary_key_types`/`set_pk_column_type` 加性传递 + `pk_type_known_non_int` 两判定门，Float/String/Bool 键列键位等值统一回退 DataScan；T3/T4 写入类型强制：`StorageError::KeyTypeMismatch` + InsertExecutor/UpdateExecutor 前置校验，Int 键列只接受 Int/NULL；000-initial BH-1（既有测试依赖 Int 收负 Float 形态）触发 replan → 001-replan：T7 列清单映射（`build_insert` plan 期校验 + `map_insert_values` 重排，消除错位/panic/未知列三形态）+ T8 校准 + 收尾，888 tests）；001（T5 RED 见证 4 用例 + T6 rekey 实现：前置块碰撞预检（新键命中即 DuplicateKey 零副作用）+ Step 7 三分支（NULL 删 / 同键 update / rekey 先删后插）；000-initial BH-3（6 个 M10 直连执行器测试依赖旧键残留缺陷行为）触发 replan → 001-replan：T9 校准 gc_test ×3 / version_chain_test ×2 / plan_exec_test ×1 按行当前键寻址 + 收尾）；三轮 Plan Review（两个 replan-required + 两轮 accepted）；**892 tests pass / 0 failed / 2 ignored**（Plan Review 独立复跑）；clippy/fmt/validate 全 0/PASS（specs 27）；I046/I047 转 promoted | 未 commit（待用户触发；对照基线 d8a244f）；change 归档 `archive/2026-09-12-ms16-correctness-batch/`；docs sync 本次写入（specs 合并 27 + I 登记 + tasks/SNAPSHOT 同步） |
| 2026-09-12 | MS15-Rest 初版前正确性收口第二批（T02/T03/T04 聚合 change，I034/I037/I039）：3 Iteration——000（I034）：`get_plan_output_columns` 新增 `projected_columns`，DataScan 臂必需应用 projection（Scan/IndexScanAll 恒等加固、IndexScan 保持构造期收窄），CLI 表头与行形状一致（json `columns`/`rows` 字段数一致），cli_test +3；001（I037）：`UpdateExecutor` Step 7 分支化（新值 `to_key()==None` → `index_manager.delete(&self.key)`），旧键 INSERT 误拒消除、点查空集、运行期/恢复两态一致，新增 `tests/update_index_maintenance_test.rs` 5 测试 + `keyless_row_test` T8-R2 校准（000-initial Gate 6 阻塞 → 001-replan 用户批准）；002（I039）：`ast.rs` 新增 `object_name_to_table_name`（`Ident.value` + lowercase + `.` 连接）统一 11 处表名消费点，带引号与裸名拼写全语句等价 + dump→restore→dump 恒等 + schema 同名重建，cli_test +5；转义名 dump 探针（R1-S3）Gate 6 阻塞（`select_all_rows` 裸插值 × T8 契约 Forbidden 冲突）→ Plan Review rework-required → 001-rework T9-R1：`select_all_rows` 经既有 `quote_ident` 包裹 + 注释按归一化语义改写 + 转义名 dump 往返测试（cli_test +1 至 65）；**867 tests pass / 0 failed / 2 ignored**（Plan Review 两轮独立复跑：blocked 审计 866/0/2、收口 867/0/2）；clippy/fmt/validate 全 0/PASS（specs 25）；I034/I037/I039 转 promoted + I033 证据强化（跨进程同键 UPDATE→DELETE 丢失实证，同根去重）+ 登记 I047（rekey 旧键残留）/I048（import 实参边界） | d8a244f（T01 + T02/T03/T04 实施 + 归档 + docs sync 单提交，对照基线 f9e1e1f） |
| 2026-09-12 | MS15-T01 I036 键位等值路由修复：单 Iteration 000（T1-T4）——`src/parser/planner/query.rs` 新增 `has_non_keyable_pk_literal_leg` 分类 helper（镜像 `has_pk_equality` 遍历：Eq 腿键列 Identifier + 另一侧 `Expr::Value` → `value_from_sqlparser(...).to_key().is_none()`；AND 递归、OR 保守 false、转换失败 `?` 传播）+ `build_select` WHERE 路由 `has_pk_eq` 分支条件收窄为 `has_pk_eq && !has_non_keyable_pk_literal_leg(..)?`，非键控腿（String/Float/Bool/NULL 字面量）落入既有 OR 臂（`Filter(DataScan)`）/ 谓词下推臂（`DataScan`），未新增第三条 plan 路径；`extract_pk_from_where`/`is_simple_pk_equality`/`has_pk_equality` 本体/执行器/存储层零改动，形态 2（Int 字面量 + Float 键列，登记 I046）按批准范围保留；新增 `tests/keyless_eq_routing_test.rs` 8 测试（R1 四行为场景 + 简单非键控/OR plan 形状 + R3-S1 空集不变 + R1-S5 restart；RED 5 failed/1 passed → GREEN 8 passed）；853 tests pass / 0 failed / 2 ignored（基线 845 只增不减，Plan Review 独立复跑全量 + clippy --all-targets/fmt/validate 全 0/PASS + CLI 三探针行集逐字节一致）；Plan Review accepted（4 偏差全非阻塞：BASELINE-CHANGED 基线前移、PLAN-INVALID OR 形态 RED 预测与自家调查事实矛盾、平行 helper 属 Plan 留白、clippy 范围增强；3 findings：F1 Bool/NULL 无场景级见证 Minor、F2 同 I041 既有项、F3 形态 2 残差）；I036 转 promoted + 登记 I046；spec `planner-key-equality-routing` 合并（3 Requirement 9 场景） | d8a244f（与 MS15-Rest 实施 + docs sync 单提交入库；docs sync 曾先行写入工作区） |
| 2026-09-11 | MS11-T03 标量函数库第一批（string 6 + math 4）：Iteration 000（T1-T3）——`src/executor/function.rs` 新模块单点注册表（`REGISTRY` 元数据 + planner 校验入口 `is_scalar_function`/`check_scalar_function` + `FunctionExpression` 统一节点，D3 求值顺序：全部参数求值→错误先传播→任一 NULL→NULL 跳过类型校验；`evaluate_ref` 三先例模式 String 结果报错、`set_parameter_value` 递归）+ planner `Expr::Function` 臂扩展（COALESCE 逐字节保持 → OVER/DISTINCT/FILTER/NULL treatment/ORDER BY/命名/通配符点名拒绝 → arity 校验 → 构造；未注册名维持既有文案）+ `Expr::Trim` 独立变体臂（仅纯 `trim(s)`，规格化形态点名拒绝）+ `ast.rs` 两放行门（注册名/`Expr::Trim` 放行，未知名维持既有拒绝）+ string 六函数语义（upper/lower ASCII、length Unicode 字符计数、substr SQLite 边缘五断言、replace 空 from 原样、trim 仅 U+0020）+ 严格类型；Iteration 001（T4-T6）——REGISTRY +4（ABS/ROUND/FLOOR/CEIL）+ eval_scalar 四臂（abs 同型、round `f64::round` 半数远离零 + `10^digits` 乘除（digits Int/Float 向零截断、负 digits 整数位）、floor/ceil 返回 Float）+ `float_arg` 严格 Int|Float + `Expr::Ceil/Floor` 独立变体接线（`build_ceil_floor` 共享 helper，TO DateTimeField 点名拒绝——PLAN-OMISSION 按 TRIM 先例补救）+ R5 六调用面场景（下推/OR/聚合混用/HAVING/ORDER BY 别名静默锁/PK+函数）+ cli_test 2 表头用例；新增 `tests/scalar_function_test.rs` 28 测试 + function.rs 18 单测 + cli_test 增至 56；845 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑全量 + 三目标套件精确数 + clippy/fmt/validate 全 0/PASS）；两 Iteration Plan Review 均 accepted（000 轮 7 偏差含 spec R1/S1 文案勘误——SELECT 位置实为 `Unsupported statement type`、ast.rs 放行门 PLAN-OMISSION；001 轮 5 偏差含 CEIL/FLOOR 接线）；登记 I043/I044/I045 + I036 佐证（预存缺陷 A 同根去重） | 实施未提交（待用户触发；对照基线 179228b）；docs sync 本次写入（归档 + spec 合并 + I 登记 + tasks/SNAPSHOT 同步），同工作区待用户一并 commit |
| 2026-09-10 | MS11-T02 SQL 事务语句 BEGIN/COMMIT/ROLLBACK：Iteration 000（T1-T3 lib 事务语句层）——`TxStatementKind` + `classify_transaction_statement`（`src/parser/planner/mod.rs`，`build_plan` 函数首行前置分类：边界子句 D3 点名文案直达、干净事务语句 session-only 拒绝、其余逐字节不变）+ `PlanError::TransactionStatement`（消息即全文）+ `TransactionSession`（`src/transaction/session.rs` 新模块，begin/commit/rollback/is_active/tx_id/tx，错误文案即契约，5 单测）+ `tests/tx_statement_test.rs` lib 段 3 测试（execute_sql/execute_in_tx/SqlHandler 三路径拒绝 + plan cache 不变 + 拒绝不终结事务）；Iteration 001（T4-T6 CLI 会话接线与 e2e）——`run_sql` 分类三臂分派（事务语句驱动会话 + `Affected(0)` 直渲染不经 executor；`Ok(None)` 会话活跃走 `execute_stage_in_tx`、空闲走既有 `execute_stage`）+ `rollback_session` helper 覆盖全部错误返回路径 + 收尾回滚 + stderr `uncommitted transaction was rolled back at exit`（exit 0）+ `sql_failure_status` 增 `in_transaction` 上下文后缀（lifecycle.rs 2 调用点补 `false`）；CLI e2e 16 测试（R1 提交可见/回滚无残留/事务内 SELECT 三文档化场景 + R2 五拒绝含 `AND NO CHAIN` 等价 + R3 边界四场景 + S7/S8）；797 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑全量 + clippy/fmt/validate 全 0/PASS + 组合路径探针）；两 Iteration Plan Review 均 accepted（000 轮修正 spec R2 NO CHAIN 不可实现 + R3/S4 病句；001 轮零返工，F1-F5 全非实质） | 179228b（实施，含 change 目录）；docs sync 本提交（归档 + spec 合并 + I041/I042 登记） |
| 2026-09-10 | MS11-T01 SQL 表达式四件套与值表达式（WHERE/SELECT）：Iteration 000（T1-T6）三值求值内核 `Ternary`/`evaluate_ternary`（Comparison/Logical 三值重写、`evaluate()`=fold、短路保持式——既有行为逐字节等价）+ `LikePredicate`/`IsNullPredicate`/`NotPredicate` + planner 七个 WHERE 臂（IN→OR 链 / BETWEEN→AND / LIKE / IS NULL / NOT 脱糖；ESCAPE/TRY_CAST/未知 DataType 计划期拒绝）+ `contains_or` 七变体扩展 + `CaseExpression`/`CoalesceExpression`/`CastExpression`（CAST 严格四族 + 转换矩阵）+ I040 负数字面量折叠；Iteration 001（T7-T9）`ProjectionNode`（PhysicalPlan 第 20 变体）+ `ProjectionExecutor`（owned `evaluate` 逐项求值）+ SELECT 表达式项路由（AS 别名 / Display 列名 / 四拒绝面 / 聚合报错保持 / `SELECT 42` 单列怪癖修正）+ `building_subquery` 子查询上下文抑制（R6 回归修复）；新增 `tests/expression_e2e_test.rs` 24 + `tests/projection_expression_test.rs` 16 + predicate/planner/pushdown/cli 追加 24（cli_test 增至 54）；769 tests pass / 0 failed / 2 ignored（Plan Review 独立复跑 + 17 项二进制探针）；clippy/fmt/validate 全 0/PASS；两 Iteration Plan Review 均 accepted | 4813374（实施，含 change 目录）；docs sync 046a76c（归档 + spec 合并 + I040 promoted）；c468055（SNAPSHOT hash 记录勘误） |
| 2026-09-09 | MS10-T05 生命周期子命令：4 Iteration 双轮收口（000：T1 入口重构 Option 位置参数 + `Command` 六子命令分发 + 手动 usage、T2 resolve 目录 helper `rtsql_home()/db_dir()`、T3 new（存在性拒绝 + create_dir_all + 编排复用建库）、T4 list（db_dir 枚举 .db 行集 render 输出）、T5 DDL 生成器 `create_table_sql` + schema（catalog scan → 逐表 DDL 行）；000 001-rework：建库约束持久化通道——`TableManager::create_table_with_constraints`（旧签名委托壳）+ `CreateTableExecutor` 约束透传，NOT NULL/UNIQUE 真实写入 catalog，S1 转绿；001：T6 dump（`sql_literal` 纯函数 + DDL 行 + 全行 INSERT 流）、T7 restore（空库前置 + 静默逐条循环 + `-` stdin + fail-fast 复用 `sql_failure_status`）、T8 import --csv（csv 1.4 + 表头双向匹配 + `csv_value` 类型转换 + 逐条 auto-commit + affected 输出）；001 001-rework：无键行落库语义（用户裁定方向 A）——`InsertExecutor` 键位不可键控行（NULL/非 Int）由静默丢弃改为落库不入索引 + 恢复 Update 重放 keyless 桶回退（`PkVersionMaps` keyed/keyless 双桶，非 deindexed 分支保留 RedoFailed），S3/S4 转绿；新增 `tests/keyless_row_test.rs` 4 + `test_dump_restore_roundtrip_full_shape` 全形状往返；704 tests pass / 0 failed / 2 ignored（独立复跑 2 次）；clippy/fmt/validate 全 0/PASS；两轮 Plan Review accepted（Deviation 1 PLAN-INVALID×2 非阻塞：dump SELECT 原名 + M19 路由断言修正） | b51985f（实施，含 change 目录）；docs sync 046a76c（归档 + spec 合并 + I036-I040/R20 登记） |
| 2026-09-09 | MS10-T04 多语句执行修复：`src/cli/mod.rs::run_sql` T01 护栏移除 → 分片逐条循环（每条独立 `plan_stage` 缓存键 = `stmt.to_string()` canonical 文本 D5 → `get_plan_output_columns` → `execute_stage` 逐条 auto-commit D4 → `render`+`emit_stdout` 顺序写出 D2）；`sql_failure_status` 定位模板 `statement {k} of {n} failed: {error}; statement: {stmt_text}`（200 字符截断）+ k>1 追加 `; previous statement(s) were committed`（D3），parse 错误保持透传（零执行+行列文本）；`src/pipeline.rs` `execute_inner`/`execute_in_tx` 对 `len>1` 在 plan_stage/cache put 前返回 `Response::Error`（D6，共享 `multi_statement_rejected` helper）——静默 first() 截断收口；cli_test 护栏用例重写为 `test_multi_statement_executes` + 新增 sequential_render / semicolon_boundaries / fail_fast / parse_error_zero_exec（S5 见证经用户批准修订为含 FROM 等价 SQL）；file_header_test 2 处 `repeat_n` clippy 债务清偿；671 tests pass / 0 failed / 2 ignored；clippy/fmt/validate 全 0/PASS；Plan Review accepted（6 非阻塞 finding：S5 修订×2、数值勘误、裸 DataScan 表头 NEW-EVIDENCE→I034、kind() 重算不处理） | 8827700（实施，含 change 目录）；docs sync 归档至 `openspec/changes/archive/2026-09-09-2026-09-08-ms10-t04-multi-statement-execution/`；修改 spec `cli-noninteractive-shell`（R1 修正 + R5 替换为「多语句分片逐条执行」6 场景）；登记 I034/I035 |
| 2026-09-08 | MS10-T03 文件 magic/格式版本头：`src/storage/file_header.rs` 新模块（64B 布局 D1：magic `RTSQLDB\0` + version u32 LE=1 + flags u32 LE + page_size u32 LE=4096 + 32B 盐预留 + 12B 保留；encode/decode 纯函数 + 私有 HeaderError 六分类；KNOWN_FLAGS_MASK=0——加密位拒绝至 MS12）；`FileStorage::open` 按 D4 接线（锁原样 → 0 字节写头无 fsync → 分类校验先于页解析/WAL 触碰）；read/write/allocate 3 处页偏移 +HEADER_SIZE（`to_offset` 纯数学不变）；error.rs +NotADatabase/NewerFileVersion/IncompatibleHeader（additive，CLI General 分支 exit 1，锁冲突仍 exit 4 优先）；新增 `tests/file_header_test.rs` 14（头生命周期 + 拒绝矩阵 + Database 级垃圾 8k 干净拒绝——RED 基线 SIGABRT 134 消除 + 锁优先守卫）+ database_file_lock_test 1 + cli_test 4（垃圾 exit 1 内容未改 / newer version 文案 / 零伴生 / 锁优先 exit 4）；665 tests pass / 0 failed / 2 ignored；clippy/fmt/validate 全 0；Plan Review accepted（4 非阻塞 finding，含 Guidance 掩码勘误） | 2eda010；change 归档至 `openspec/changes/archive/2026-09-08-2026-09-08-ms10-t03-file-format-header/`；新增 spec `database-file-format-header`（4 Requirement） |
| 2026-09-08 | MS10-T02 跨进程文件锁 + 优雅停机 + Iteration 000 WAL 恢复引擎正确性（design D0/D7-D10）：T2 `FileStorage::open` try_lock 独占锁（`StorageError::DatabaseLocked`，先于 WAL 打开与恢复）+ T3 CLI 锁冲突 exit 4（存量收编，hunk 摘除 RED 复现）；T4 两阶段 select 优雅停机（`execute_command_inner`：open/执行各与信号竞争，信号臂 → `close()` checkpoint → `Signaled(signum)` exit 130/143；打开阶段无 close）+ D5-⑤ 库级结构测试（Notify 握手使信号确定落在执行阶段：WAL<1KB 证 close 已执行）+ 打开阶段 WAL>2KB 断言 + 重标定（D10 后 40k→8.86s、160k→40.7s，`WAL_ROWS=40_000`）；Iteration 000 引擎正确性：T0 reader 逐帧无歧义（歧义偏移先新格式 CRC 验证、失败回退旧格式，修复嗅探 derail 79/2046 帧）、T0b 位置寻址重放（redo 按记录 `row_id` 写入：slot 已存在跳过/稠密落位校验/未初始化页 init + Update 版本链重建 + Delete 墓碑，修复 10k 重开 13190≠10000）、G1-G3 B-Tree 规模缺口（`Key::deserialize` 32 字节定长比较修最小键盲区 / delete 重平衡 Page-full / 内部节点 update）、R5 catalog `index_root_page_id` 根变更同步、R6 DataScan 替代集合去重（产出 ⟺ 可见 ∧ 无已提交非墓碑替代者——运行期与恢复同源修复 110→100）、R7/R8 恢复期索引去信任 + 重放后重建（撕裂树修复：中位点 checkpoint 后页驱逐按 LRU 而非树拓扑刷盘致磁盘树含洞/孤儿（裸读实测 184/10000 可达 + 洞页）→ `redo_count > 0` 时恢复零消费磁盘索引树，Update `old_row_id` 由磁盘版本多映射 max-rid 派生 + old_tuple 校验，重放后从最终数据页重建 PK 索引（链尾回溯 + 重复 PK 显式报错保 K05）+ `replace_index_manager` 换入 + catalog root 写回 + 洞容忍释放旧树；`redo_count == 0` 路径零变化）；6 个 Cycle（000-initial → 001-replan → 002-rework → 003-rework → 004-rework；001-lock-shutdown/000-initial），6 轮 Plan Review（2 次 rework 扩面经用户 Gate 2 批准）；636 tests pass / 0 failed（白名单清零）/ clippy 0 / fmt 0 / validate 18 PASS | 5855245；change 归档至 `openspec/changes/archive/2026-09-08-2026-09-06-ms10-t02-file-lock-graceful-shutdown/`；新增 spec `database-file-lock`（R1 独占锁语义 / R2 生命周期与释放）、`wal-recovery-frame-parsing`（2 Requirement）、`wal-recovery-replay-integrity`（2 Requirement），修改 spec `cli-noninteractive-shell`（R1 锁冲突 exit 4 场景 + 新增 Requirement 优雅停机 4 场景）；登记 I031-I033 + K38 |
| 2026-09-06 | MS10-T01 CLI 壳 + 扫描执行器真投影：Iteration 000（`src/cli/{mod,resolve,render}.rs` 新建 + `main.rs` 重写 one-shot 入口：clap 参数、裸名→`$RTSQL_HOME/db/<name>.db`（默认 `~/.rtsql/`）/含 `/` 直开、table/json/csv/tsv 四格式（TTY 表格 / 非 TTY JSON 默认）、退出码 0/1/2/3 + 4/5 枚举留位、多语句显式拒绝护栏（文案指向 T04）、`close()` checkpoint 截断 WAL、`get_plan_output_columns` 补 JOIN 三臂真表头；608 tests 既有零修改）＋ Iteration 001（真投影：6 plan 节点携带 `projection: Vec<usize>`、6 执行器（4 scan + Filter + Sort）`with_projection` 在谓词求值与 MVCC 判定后裁剪、聚合 `input_schema` 统一经 `get_plan_output_columns`（修复 PK 点查聚合静默 Null 与 GROUP BY 映射）、投影外 ORDER BY 正确排序（Sort 比较用输入形状、物化时裁剪）；IndexScan 表头错位/聚合静默 Null/排序失效三症状由 `tests/projection_test.rs` 6 测试锁定；既有测试校准面实测 0；614 tests pass，clippy/fmt/validate 全 0；两轮 Plan Review accepted） | 03ff1b9；change 归档至 `openspec/changes/archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/`；新增 spec `cli-noninteractive-shell`（6 Requirement，R6=真投影） |
| 2026-09-05 | MS08-T01+T02 页 I/O 位置参数化 + 扫描预取：T01 `FileStorage::read_page_blocking`/`write_page_blocking` 改 `FileExt::read_exact_at`/`write_all_at`（每页 2 syscall→1；strace 页路径 lseek 33→3、pread64 4→26、pwrite64 0→8；并发冷读串页损坏实测复现 RED→修复 GREEN，`tests/file_storage_io_test.rs` 4 测试）；T02 `DataScanExecutor` 后继页预取（closure 捕获 successor + spawn 丢弃结果 + 页 id 去重 + 在途 ≤1，`with_prefetch` 开关），默认路径实测回退 +40~47%/+17~18%（p<0.05，对照组不变）→ replan 默认改关、显式启用（`tests/prefetch_test.rs` 3 测试 + 默认关闭单测；Review 第三轮 bench 两档 No change p=0.24/0.73 回基线）；585 tests pass；clippy/fmt/validate 全 0；Plan Review accepted（T5.4 判读偏差裁定为环境侧 BASELINE-CHANGED 非阻塞） | dac6783；change 归档至 `openspec/changes/archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/`；新增 spec `storage-io-optimization`（3 Requirement） |
| 2026-09-05 | MS07-T06 谓词/LIMIT 下推：`DataScanNode` 新增 `predicate`/`scan_cap`；planner 非 PK WHERE 无 OR 时谓词装入 DataScan（不再生成 Filter），OR 保留 Filter(DataScan)；Limit 输入链恰为纯 DataScan 时写入 `offset+limit` 封顶（limit=0 → Some(0) 立即 Done），顶层 Limit 任何形状保留；DataScanExecutor 两个行产出点接入 `filter_row`/`yield_capped`（语义逐字对齐 filter.rs）；`correlated.rs` 补 DataScan 相关参数注入臂（Plan 遗漏面）；新增 `tests/pushdown_test.rs` 15 测试（577 tests pass；clippy/fmt/validate 全 0；Plan Review accepted） | 5d652a2；change 归档至 `openspec/changes/archive/2026-09-05-2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown/` |
| 2026-09-05 | MS07-T05 Checkpoint 真正工作：`full_recover` 消费 16B 位点（有效位点只重放 `≥ L`，缺失/损坏/代际失效安全退化全量；分类不裁剪）；K05 六处静默吞错显式化（`WalError::RedoFailed` 含表名/tx_id/row_id，`Database::open` 失败可见）；`WalWriter::rewrite_truncate` 单临界区原地截断（禁止 temp+rename）；`CheckpointManager::checkpoint()` 九步流程；`Database` 接线 `checkpoint_manager` + 公开 `checkpoint()` + `close()` 自动触发；新增 `tests/checkpoint_redo_reduction_test.rs` 9 测试（Plan Review accepted） | 0df2b93（与 T04 同提交） |
| 2026-09-05 | MS07-T04 显式事务：`Database::{begin,commit,rollback,execute_in_tx}` 公开 API；`tx_versions` 按表聚合 + `abort_cleanup_versions` 多表回滚（含墓碑 `mark_deleted`，修复 snapshot 无关扫描的回滚幽灵行）；`pipeline::execute_in_tx/execute_stage_in_tx` 用户事务路径（DML 消费 tx_id、无隐式包裹、隐式路径零变化）；新增 `tests/explicit_tx_test.rs` 8 测试（Plan Review accepted） | 0df2b93 |
| 2026-08-30 | MS07-T03 planner 模块化拆分：`src/parser/planner.rs`（2266 行）按职责拆为 `src/parser/planner/` 目录 6 模块（`mod.rs` + `query`/`expression`/`aggregate`/`subquery`/`ddl_dml`）；`PlanBuilder` 三字段 `pub(crate)`；12 单测随函数迁移（mod 3 / query 5 / ddl_dml 4）；公共 API / re-export / SQL 语义零变化（`tests/planner_test.rs` 29 + `executor_test.rs` 39 零修改全绿；542 tests pass；clippy 0 warning；fmt 0 diff；openspec validate 12 passed；Plan Review accepted） | 49a85ef；change 归档至 `openspec/changes/archive/2026-08-30-2026-08-30-ms07-t03-planner-decomposition/`；新增 spec `planner-module-decomposition`（5 Requirement） |
| 2026-08-30 | MS07-T02 drop_table 物理页释放：新增 `src/storage/btree/index_manager.rs::IndexManager::collect_all_pages`（栈式 DFS + visited 防环，pub async）；`TableManager::drop_table` 重写为「保留名→取 meta→catalog.delete→tables.remove→collect BTree→collect data→free」+ 新增私有 `collect_data_pages`（K22 链遍历）；`tests/drop_table_free_test.rs` 6 集成测试（542 tests pass；clippy 0 warning；fmt 0 diff；openspec validate 11 passed；Plan Review accepted） | bd038da；change 归档至 `openspec/changes/archive/2026-08-30-2026-08-26-ms07-t02-drop-table-physical-free/`；新增 spec `drop-table-physical-free`（7 Requirement） |
| 2026-08-26 | MS07-T01 系统表 `__tables` / `__columns` + Schema 页：新增 `src/storage/catalog.rs`（~908 行 / 7 方法 + 10 单元测试）；`IndexManager::from_root(buffer_pool, root_page_id)` 路径（不调 `BTree::new`）；`TableManager::new(buffer_pool, storage) -> Result<Arc<Self>>` async + `open_or_init` 重建 + 保留名检查（`ReservedTableName`） + 跨页 `data_page_tail` 同步；`Database::open` 接 `open_or_init` + 新增 `close()` 显式 flush；`InsertExecutor` `Option<Arc<TableManager>>` + `with_table_manager`；`AsyncStorage::page_count` trait 方法；`StorageError::ReservedTableName` 变体；`ColumnType` 加 `Eq`；`tests/schema_persistence_test.rs` 8 集成测试；14 个其他 test 文件批量改签名（534 tests pass；clippy 0 warning；fmt 0 diff） | 4307a0e；change 归档至 `openspec/changes/archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/`；Plan Review `accepted`（R16 登记）；新增 spec `schema-persistence`（7 Requirement） |
| 2026-08-26 | MS06-T03 + MS06-T04 一并完成：T03 `WalWriter` 持 `Arc<Mutex<File>>` 单一持久句柄，5 个 IO 方法删除逐次 open；`tests/wal_handle_test.rs` 4 测试（fd 上界 / LSN 偏移 / truncate 追加 / 并发一致）。T04 `pipeline::execute_inner` 279 行单函数 → 编排器 + `parse_stage`/`plan_stage`/`execute_stage` 三个 pub 函数 + `#[cfg(test)] mod tests` 8 单测 + `benches/pipeline_stages_bench.rs` 三阶段 bench（516 tests pass；`cargo build` 0 warning；`cargo clippy -D warnings` 0 warning） | 未 commit（待用户触发）；change 归档至 `openspec/changes/archive/2026-08-26-2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages/`；新增 spec `wal-writer-handle-reuse` + `pipeline-stage-decomposition` |
| 2026-08-26 | MS06-T02 PlanCache DashMap + SQL 规范化：`HashMap + &mut self` → `DashMap + &self`；新增 `normalize_sql_key`（ASCII 折叠 + 空白折叠 + trim + 单引号 toggle）；`Database.plan_cache: Arc<Mutex<PlanCache>>` → `Arc<PlanCache>`；pipeline 5 处调用点去锁；`tests/plan_cache_test.rs` 7 集成测试 + 10 单测；T0 基线 clippy 归零 + 36 处表外 mechanical 修复（504 tests pass） | 未 commit（待用户触发）；change 归档至 `openspec/changes/archive/2026-08-26-2026-08-25-ms06-t02-plancache-dashmap/`；新增 spec `plancache-key-normalization` |
| 2026-08-25 | 修复 DML `tx_id=0` 占位注入：pipeline 事务包裹 + Insert/Update/Delete WAL 唯一来源 + VersionHeader::commit 墓碑守卫 + 6 个新测试（487 tests pass） | 未 commit（待用户触发）；change 归档至 `openspec/changes/archive/2026-08-25-2026-08-25-fix-dml-tx-id-injection/` |
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
