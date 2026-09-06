# tasks — 任务与里程碑路线

> 最后更新：2026-09-06（MS10-T01 完成并增量刷新，commit `03ff1b9`；同日 roadmap 重排：MS09 重定义 + 新增 MS10-MS13 应用层轨道，依据 R18 四轮探索，用户批准）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

14 个 Milestone：5 completed（MS00-MS02 历史 + MS06/MS07）+ 3 旧 superseded + 6 planned（MS08 剩余、MS09 重定义、MS10-MS13 应用层新轨）。

规划理念：**先收口正确性 → 建设基础能力 → 实测驱动性能 → 引擎能力收尾 → 应用层可用好用（非交互 CLI / 密钥 / 分析 / 分发）**（2026-09-06 依据 R18 分析扩展应用层轨道）。

执行顺序（用户批准 2026-09-06）：MS10 优先于 MS08 剩余项（T03-T06 穿插进行）；MS11 与 MS12 顺序可互换（均只依赖 MS10）。
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

- **Status**: planned（T01/T02 已完成，2026-09-05；T03-T06 待后续 change）
- **Dependencies**: MS07（MS08-T03 依赖 MS07-T05）
- **Outcome**: 6 类微基准 baseline 落盘；每类优化要么量化改善、要么记录"未达预期"；建立"实施前先 `--save-baseline`"纪律
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

- **Non-goals**: 新 SQL 方言；新执行器；多隔离级别；B+Tree 节点级锁；io_uring
- **Workload**: 每优化 1 change（Varint Key 可能 2 个），共约 5-7 change；每 change 前置 baseline
- **Stable baseline**: 性能基线档（`cargo bench --save-baseline before-MS08-T*` 落盘）；6 类微基准数据集；fsync 频率与延迟关联曲线
- **Verification boundary**: 每个 T 必须满足"前置 baseline 留档" AND "实施后某关键指标量化改善 OR 明确记录未达预期"
- **Diagnostic boundary**: 性能问题可定位到 bench 文件 / 数据规模 / commit hash
- **Split signals**: 若 MS08-T01 实施后 syscall 计数无显著变化，说明 syscalls 不是瓶颈，整个 MS 需重新平衡
- **Related changes**:
  - `2026-09-05-ms08-t01-t02-pread-prefetch`（T01/T02 合并 change，已归档为 `archive/2026-09-05-2026-09-05-ms08-t01-t02-pread-prefetch/`，含新增 spec `storage-io-optimization`，3 Requirement：R1 页 I/O 位置参数化 / R2 零接口零格式变更 / R3 DataScan 预取可选能力默认关闭）

### MS09：引擎能力收尾 — planned（2026-09-06 重定义，原"SQL 标准与上层能力"）

- **Status**: planned
- **Dependencies**: MS08
- **Outcome**: 隔离级别可配（Read Committed）；NLJ/Hash Join 启发式切换；关联子查询结果缓存
- **Rationale**: 原四项中 PG Extended Query 被非交互 CLI 形态决策降级（无消费者，server 保留为库能力，将来 serve 复活时再议）；其余三项同属引擎能力面，与应用层轨道（MS10-MS13）分开验收
- **Scope**:

| Task | 目标 | 备注 |
|---|---|---|
| MS09-T01 | Read Committed 隔离 | 不含 Serializable / SSI |
| MS09-T02 | NLJ + 与 Hash Join 启发式切换 | **不含代价模型** |
| MS09-T04 | 关联子查询结果缓存 | 视 MS09-T02 完成后实际场景 |

- **Non-goals**: PG Extended Query（降级移出，2026-09-06）；Serializable / SSI；代价模型与 Join 重排；io_uring；B+Tree 节点级锁；clone 消除 Arc/Cow（待 MS08 完成后看真实数据再决定）；多用户权限
- **Workload**: 3 change；总工作量适中
- **Stable baseline**: Read Committed 跨并发可验证；NLJ 在小表上优于 Hash；关联子查询 N 行外层不重复执行子查询
- **Verification boundary**: 3 项独立测试套件
- **Diagnostic boundary**: 3 个独立子系统（事务管理 / Join 执行器 / 子查询执行器）
- **Split signals**: 若 MS09-T01 实施发现 snapshot 与 RR 共享度过低，拆为独立 MS
- **Related changes**: None

### MS10：CLI 非交互命令面 — planned（应用层主轨，2026-09-06 新增）

- **Status**: planned（T01 已完成，2026-09-06，含 Iteration 001 真投影扩展——用户批准方向 B；T02-T05 待后续 change）
- **Dependencies**: MS08
- **Outcome**: `rtsql <db> <sql>` 主命令全链路可用——裸名集中存储（`$RTSQL_HOME`，默认 `~/.rtsql/db/`）+ 含 `/` 路径直开；`new/list/schema/dump/restore/import` 生命周期子命令；TTY 默认表格、非 TTY 默认 JSON、`--format table|json|csv|tsv`；退出码分类（0 成功/2 用法/3 SQL 错/4 锁冲突/5 密钥）；多语句 `;` 分片逐条执行+报错行号；跨进程文件锁（advisory 独占，占用报 `database is locked`）；优雅停机（信号→`close()` checkpoint）；文件 magic/格式版本头（趁零用户落，不兼容即报"文件由新版创建"）
- **Rationale**: 应用层一切能力的载体（R18 主题 7）；文件锁/优雅停机/格式头/多语句修复是正确性前置而非增强，与 CLI 壳同一验收域（"CLI 全链路可用"），不拆
- **Scope**:

| Task | 状态 | 目标 | 关键前置 | 关联 change |
|---|---|---|---|---|
| MS10-T01 | **completed**（2026-09-06，含 Iteration 001 真投影） | CLI 壳：参数化入口 + 名称解析（裸名/路径）+ 主命令 + 输出格式 + 退出码 | 无（main.rs 重写为参数化入口） | `archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/` |
| MS10-T02 | planned | 跨进程文件锁 + 优雅停机（信号接线 `close()`） | MS10-T01 | — |
| MS10-T03 | planned | 文件 magic/格式版本头（FileStorage open 校验） | MS10-T01 | — |
| MS10-T04 | planned | 多语句执行修复（`;` 分片逐条执行 + 明确报错，替换 `pipeline.rs` first() 截断） | MS10-T01 | — |
| MS10-T05 | planned | 生命周期子命令：`new/list/schema/dump/restore/import --csv` | MS10-T01（schema 为 agent 发现刚需） | — |

- **Non-goals**: REPL（后续可选）；密钥/加密（MS12）；分析函数（MS11/MS13）；安装分发（MS13）；鉴权/多用户
- **Workload**: CLI 模块新建（clap 依赖 + `src/cli/`）+ file_storage 锁/格式头 + pipeline 多语句 + 5 子命令
- **Stable baseline**: 脚本 `rtsql db "SELECT ..."` 端到端稳定；并发打开同一文件得到明确错误（退出码 4）；kill 后 WAL 恢复 e2e
- **Verification boundary**: CLI 集成测试（参数/名称解析/格式三态/退出码分类/锁冲突/多语句含报错行号/信号停机 e2e）
- **Diagnostic boundary**: `src/cli/`（新模块）+ `src/main.rs` + `src/storage/file_storage.rs`（锁/格式头）+ `src/pipeline.rs`（多语句）
- **Split signals**: 多语句分片需动 pipeline 事务语义时拆出独立 change 级任务；加密讨论提前成熟时 MS12 并行
- **Related changes**:
  - `2026-09-06-ms10-t01-cli-shell`（T01 + Iteration 001 真投影扩展（用户批准方向 B，超出 T01 原始范围的引擎级修复），已归档为 `archive/2026-09-06-2026-09-06-ms10-t01-cli-shell/`，含新增 spec `cli-noninteractive-shell`，6 Requirement：R1 入口与主命令 / R2 名称解析 / R3 列名表头 / R4 输出格式 / R5 多语句护栏 / R6 扫描执行器真投影。规划依据：R18 `usability-gap-cli-form.md`）

### MS11：SQL 表达式与函数层 — planned（分析能力主体，2026-09-06 新增）

- **Status**: planned
- **Dependencies**: MS10（CLI 可发现 schema，agent 可验证函数行为）
- **Outcome**: WHERE/SELECT 支持 `IN / LIKE / BETWEEN / IS NULL / CASE / COALESCE / CAST`；标量函数库第一批（string: upper/lower/length/substr/replace/trim；math: abs/round/floor/ceil）；SQL 级 `BEGIN/COMMIT/ROLLBACK` 语句（复用 MS07-T04 显式事务 API 接线 planner 语句臂）
- **Rationale**: 分析能力的主体在 SQL 层而非 CLI 命令（R18 主题 7 结论——agent 是写 SQL 的）；事务语句与表达式四件套同为"agent 写 SQL 的日常件"，同域验收；复用已有事务 API，实现面小
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS11-T01 | 表达式四件套 + CASE/COALESCE/CAST（`parser/planner/expression.rs` 扩展） | 无 |
| MS11-T02 | SQL 事务语句 `BEGIN/COMMIT/ROLLBACK`（planner 语句臂 + pipeline 事务态接线） | 无（API 已有，MS07-T04） |
| MS11-T03 | 标量函数库第一批（函数注册机制 + string/math 各 5-7 个） | MS11-T01（表达式层就绪） |

- **Non-goals**: 日期/时间类型与函数（MS13 深水区）；窗口函数（OVER/PARTITION BY）；自定义函数（UDF）；聚合扩展
- **Workload**: expression builder 扩展 + 函数注册新模块 + planner 事务臂 + 每函数/表达式测试
- **Stable baseline**: 日常过滤/派生列 SQL 全绿；SQL 事务语句往返（BEGIN→DML→COMMIT/ROLLBACK 语义与 API 等价）
- **Verification boundary**: 每函数/表达式独立测试 + parser/planner 回归（既有 585+ 零修改）
- **Diagnostic boundary**: `src/parser/planner/expression.rs` + 新函数注册模块 + `src/parser/planner/mod.rs` 事务臂
- **Split signals**: 单批函数超 1 change 时按 string/math 分两批；CAST 触发类型系统深层改动时拆出
- **Related changes**: None

### MS12：整库加密与 sudo 式密钥 — planned（安全域，2026-09-06 新增）

- **Status**: planned
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

### MS13：分析函数与安装分发 — planned（终段收口，2026-09-06 新增）

- **Status**: planned
- **Dependencies**: MS10 + MS11（不依赖 MS12——加密与分析正交）
- **Outcome**: 日期/时间类型与函数（类型系统扩展 + date_trunc/interval 等分析最高频维度）；`stats/sample/profile` 薄命令（底层聚合 SQL：行数/空值率/distinct/min/max/分位数）；安装分发：GitHub Releases 预编译矩阵（x86_64-linux-gnu/musl 静态/aarch64-linux/macOS aarch64+x86_64）+ `cargo install` crates.io 元数据 + clap 生成 completions/man + CI workflow
- **Rationale**: 分析深水区（日期类型 = 磁盘格式变更，依赖 MS11 函数层）与分发（所有能力就绪才有可分发产物）都依赖前置完成且彼此正交，按"最后集中收口"合并为终段；CI 是分发的前置而非独立价值
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS13-T01 | 日期/时间类型（tuple.rs 格式扩展 + 序列化/解析） | MS11（函数层） |
| MS13-T02 | 日期函数（date_trunc/interval/now 等）+ `stats/sample/profile` 薄命令 | MS13-T01 |
| MS13-T03 | CI workflow + Releases 预编译矩阵 + completions/man + `cargo install` | MS10（CLI 壳稳定） |

- **Non-goals**: 窗口函数（远期）；Homebrew tap（Releases 稳定后再议）；deb/AUR/install 脚本（按需求再议）；Windows（FileExt 限 Unix，需重写页 I/O 层，明确 out of scope）
- **Workload**: 类型系统格式扩展 + 日期函数 + 3 薄命令 + CI/打包（cargo-zigbuild 或 runner 原生）
- **Stable baseline**: 全平台二进制可下载可运行；`stats` 命令输出分位数；日期列可存可查可过滤
- **Verification boundary**: 各平台 smoke 测试（CI 矩阵跑 `rtsql --version` + 基础 CRUD）+ 日期类型/函数独立测试
- **Diagnostic boundary**: 类型系统层（`src/storage/page_format/tuple.rs`）+ CI workflow 文件 + stats 命令层
- **Split signals**: 日期类型实现超 2 change 时拆出独立 MS；分发矩阵若 macOS 需专门修复拆出；Windows 支持呼声强烈时单独立项
- **Related changes**: None

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
                                ↓
                              MS08 (planned；T03-T06 穿插)
                 ┌──────────────┼──────────────────┐
                 ↓              ↓                  ↓
        MS09 引擎能力收尾   MS10 CLI 非交互命令面（应用层主轨，优先执行）
                            ├─────────┤
                            ↓         ↓
                     MS11 表达式与   MS12 整库加密与密钥
                     函数层（分析）   （安全域；与 MS11 可互换）
                            ↓
                     MS13 分析函数与安装分发（终段收口）
```

无环；所有依赖指向已存在编号；MS11 与 MS12 均只依赖 MS10、互不依赖；MS13 依赖 MS10+MS11 而非 MS12（加密与分析正交）。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

| 完成日期 | 内容 | commit |
|---|---|---|
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
