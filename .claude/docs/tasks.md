# tasks — 任务与里程碑路线

> 最后更新：2026-08-25（MS 路线重规划：从 Phase/Mxx 旧体系升级为 MS/T 新体系）
> 同步状态: current
> 由 openspec-docs-maintainer 维护

## 命名与编号规范

- **MSxx**：Milestone 编号（2 位零填充，递增不重用）
- **MSxx-Txx**：Task 编号（隶属于具体 MS，全局唯一）
- **状态**：`planned` / `ready` / `active` / `blocked` / `completed` / `superseded`

## 路线图结构

10 个 Milestone：3 历史 completed + 3 旧 superseded + 4 新（MS06 ready，MS07-MS09 planned）。

新规划理念：**先收口正确性 → 建设基础能力 → 实测驱动性能 → SQL 标准合规**。
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

- **Status**: planned
- **Dependencies**: MS06
- **Outcome**: 表定义持久化到磁盘；restart 后 schema 完整恢复；drop_table 真正释放页；显式事务 API 可用；Checkpoint 真正工作；planner 模块可独立单测；谓词/LIMIT 可下推
- **Rationale**: WAL redo 静默吞错（`src/wal/recovery.rs:148/165/176`）、checkpoint 无效、planner 2266 行单文件、显式事务缺失、谓词无法下推 — 都是 SQL 标准合规的"能用"前提。Schema 持久化是 drop_table、checkpoint、redo verification 的共同前置
- **Scope**:

| Task | 状态 | 目标 | 关键依赖 | 关联 change |
|---|---|---|---|---|
| MS07-T01 | **completed**（2026-08-26） | 系统表 `__tables` / `__columns` + Schema 页（最大单点） | 无 | `archive/2026-08-26-2026-08-26-ms07-t01-schema-persistence/` |
| MS07-T02 | **completed**（2026-08-30） | drop_table 接 free-list，物理页释放 | MS07-T01 | `archive/2026-08-30-2026-08-26-ms07-t02-drop-table-physical-free/` |
| MS07-T03 | **completed**（2026-08-30） | planner.rs 2266 → 按 build_* 拆分到 4-6 个模块 | 无 | `archive/2026-08-30-2026-08-30-ms07-t03-planner-decomposition/` |
| MS07-T04 | planned | `Database::begin/commit/rollback` 公开 API + 隐式事务向后兼容 | 无 | — |
| MS07-T05 | planned | Checkpoint 真正工作 | MS07-T01 | — |
| MS07-T06 | planned | 谓词/LIMIT 下推到 scan 之上 | 无 | — |
| MS07-T07 | planned | 视 T04/T05 需要决定是否引入消息传递重构 | MS07-T04, MS07-T05 | — |

- **Non-goals**: 性能调优（除 pushdown 收益外）；新 SQL 方言；新执行器；多隔离级别
- **Workload**: 1-2 change（Schema 页）+ 1 change/其他子项，共约 5-6 change
- **Stable baseline**: restart-after-drop-and-reload 完整恢复；DDL/DML 显式事务可单测；checkpoint 触发后 redo 数量下降；planner 任意子模块可独立单测
- **Verification boundary**: 5 项独立测试套件 + restart e2e（redo 不再静默）
- **Diagnostic boundary**: 各子项 1-2 个具体代码位置
- **Split signals**: 若 MS07-T01 因复杂度拆 2 个 change 仍可保留；若 MS07-T03 触发 planner 大规模回归失败，拆为独立 MS
- **Related changes**: None

### MS08：性能压测（实测驱动） — planned

- **Status**: planned
- **Dependencies**: MS07（MS08-T03 依赖 MS07-T05）
- **Outcome**: 6 类微基准 baseline 落盘；每类优化要么量化改善、要么记录"未达预期"；建立"实施前先 `--save-baseline`"纪律
- **Rationale**: 旧路线把性能优化按"假设收益"排列；零拷贝 ValueRef 实施教训（K18）证明未量化目标会重蹈覆辙。本 MS 强调实测驱动
- **Scope**:

| Task | 目标 | 关键前置 |
|---|---|---|
| MS08-T01 | `pread`/`pwrite` 替代 `seek+read` | 无（最简单 syscall 削减） |
| MS08-T02 | Prefetch 双缓冲 | MS02-T02 (DataScan)、MS03 (BufferPool 优化) 已 done |
| MS08-T03 | 脏页 writev 批量写回 | MS07-T05 |
| MS08-T04 | RowLockTable DashMap | **先做 mini-bench 决定是否值得做** |
| MS08-T05 | Varint Key 编码 | 无 |
| MS08-T06 | WAL fsync 合并 | **做前先验证 fsync 是否真瓶颈** |

- **Non-goals**: 新 SQL 方言；新执行器；多隔离级别；B+Tree 节点级锁；io_uring
- **Workload**: 每优化 1 change（Varint Key 可能 2 个），共约 5-7 change；每 change 前置 baseline
- **Stable baseline**: 性能基线档（`cargo bench --save-baseline before-MS08-T*` 落盘）；6 类微基准数据集；fsync 频率与延迟关联曲线
- **Verification boundary**: 每个 T 必须满足"前置 baseline 留档" AND "实施后某关键指标量化改善 OR 明确记录未达预期"
- **Diagnostic boundary**: 性能问题可定位到 bench 文件 / 数据规模 / commit hash
- **Split signals**: 若 MS08-T01 实施后 syscall 计数无显著变化，说明 syscalls 不是瓶颈，整个 MS 需重新平衡
- **Related changes**: None

### MS09：SQL 标准与上层能力 — planned

- **Status**: planned
- **Dependencies**: MS08
- **Outcome**: 支持 Read Committed 隔离；NLJ + Hash Join 可切换；可选的 PG Extended Query 协议；关联子查询结果缓存
- **Rationale**: 旧路线把代价模型、io_uring、B+Tree 节点级锁一起塞入。本 MS 严守"价值 > 复杂度"；D-candidates 移到后续
- **Scope**:

| Task | 目标 | 备注 |
|---|---|---|
| MS09-T01 | Read Committed 隔离 | 不含 Serializable / SSI |
| MS09-T02 | NLJ + 与 Hash Join 启发式切换 | **不含代价模型** |
| MS09-T03 | PG Extended Query 协议子集（Parse/Execute） | Describe/Bind 后续可加 |
| MS09-T04 | 关联子查询结果缓存 | 视 MS09-T02 完成后实际场景 |

- **Non-goals**: Serializable / SSI；代价模型与 Join 重排；io_uring；B+Tree 节点级锁；clone 消除 Arc/Cow（待 MS08 完成后看真实数据再决定）
- **Workload**: 4 change；总工作量适中
- **Stable baseline**: Read Committed 跨并发可验证；NLJ 在小表上优于 Hash；PG 客户端可用 prepared statement；关联子查询 N 行外层不重复执行子查询
- **Verification boundary**: 4 项独立测试套件
- **Diagnostic boundary**: 4 个独立子系统
- **Split signals**: 若 MS09-T01 实施发现 snapshot 与 RR 共享度过低，拆为独立 MS；若 MS09-T03 复杂度超 2 change 拆出
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
                              MS06 (ready)
                                ↓
                              MS07 (planned)
                                ↓
                              MS08 (planned)
                                ↓
                              MS09 (planned)
```

无环；所有依赖指向已存在编号。

## 进行中

- （无）

## 已承诺待办

- （无）

## 阻塞

- （无）

## 最近完成

| 完成日期 | 内容 | commit |
|---|---|---|
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
