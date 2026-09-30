# MS26 嵌入式运行形态 — Tasks

## 任务清单

| Task | 目标 | Iteration | Requirement 映射 |
|---|---|---|---|
| T1 | lib 只读打开面：`FileStorage::open_readonly`（只读句柄 + `try_lock_shared` + 头校验复用 + 写路径守卫）、WAL 未决谓词（`WalPendingRecovery`）、`Database::open_readonly[_with_key]` 入口、`OpenMode` + 字段 Option 化（pipeline 5 处 + alter_table + CLI shutdown 等价改写）、`close`/`checkpoint` 非 ReadWrite 结构性 no-op | 000 | readonly-open-mode R1/R2/R3、database-file-lock R1（MODIFIED）、readonly-open-mode R6 |
| T2 | 只读拒绝面与 CLI：pipeline `readonly_rejection` 守卫（13 写变体，25006）、CLI `--readonly` flag 与错误映射（WAL 未决 exit 1）、ATTACH 会话模式传递（附加库只读打开）、测试 `tests/readonly_session_test.rs`（锁矩阵/未决拒绝/零写入 checksum/拒绝矩阵/ATTACH/加密只读） | 000 | readonly-open-mode R1/R2/R4/R5、error-reporting R2（MODIFIED 25006 场景） |
| T3 | `MemoryStorage`（`src/storage/memory_storage.rs`，AsyncStorage 全实现）+ `Database::open_in_memory()`（构造裁剪：无 WAL/恢复/checkpoint，事务机制经 Option 通道自然降级） | 001 | in-memory-database R1/R2 |
| T4 | CLI `:memory:` 特判（主命令多语句管道、`--key` 组合拒绝 exit 2、生命周期子命令点名拒绝）+ 测试 `tests/memory_db_test.rs`（全 CRUD/DDL、多语句、重启即弃、零文件、事务语义、ATTACH/CTAS 组合、拒绝面） | 001 | in-memory-database R1/R3/R4 |
| T5 | WAL 字节水位：`WalWriter` `bytes_appended` 计数器 + `checkpoint_bytes_threshold`（默认 16 MiB，0 禁用）+ `set_checkpoint_bytes_threshold`、pipeline 两处提交直呼收敛为 `database.commit()`、`Database::commit` 尾部水位检查（活跃事务顺延、失败传播）+ 测试 `tests/wal_auto_checkpoint_test.rs`（触发回落/顺延/禁用/失败传播/恢复代价有界） | 002 | wal-auto-checkpoint R1/R3/R4 |
| T6 | env 配置 `RTSQL_WAL_CHECKPOINT_BYTES`（CLI 层读取 → setter，非法值打开期点名拒绝）+ 死接口删除（`should_checkpoint`/`get/set_checkpoint_threshold`/阈值字段）+ `RTSQL_WAL_CHECKPOINT_BYTES` 文档面同步（README/SKILL，2026-09-30 用户指令文档债随本 change 收口）+ 全量回归 | 002 | wal-auto-checkpoint R2/R5 |
| T7 | 文档同步：`--readonly` flag 与 `:memory:` 形态进入 `README.md` / `README.zh-CN.md` / `rtsql-docs/SKILL.md` 命令面（共享锁多读单写、25006 拒绝、WAL 未决 exit 1、锁冲突 exit 4、零文件即弃、生命周期与 `--key` 拒绝 exit 2），描述与实现一致 | 003 | Iteration 000 Plan Review Minor 2 + Act 001 Remaining Issues；2026-09-30 用户指令 |
| T8 | 派生表/CTE/视图基座外层投影收口：`base_schema` 提取增 DerivedScan 臂 + `DerivedScanNode.projection` 载体（执行器/`get_plan_output_columns`/构造点接线）——纯列清单按外层清单裁剪行与表头、未知外层列 `ColumnNotFound` 点名、WHERE/ORDER BY/CTAS 组合同型、恒等面（`*`/聚合/别名项）不回归 + delta spec `join-output-shape-resolution` ADDED R5 + 测试扩展 | 003 | join-output-shape-resolution R5（ADDED）；2026-09-30 用户指令「Act 001 Issue 候选本 change 解决」 |
| T9 | `--readonly`×生命周期子命令组合拒绝自动化用例（`tests/readonly_session_test.rs` +1 例，锁定 Iteration 000 偏差 6 行为；readonly-open-mode delta spec R1 增补场景） | 003 | readonly-open-mode R1（场景增补）；Iteration 000 Plan Review Minor 1 |
| T10 | 测试告警与工作区孤儿清理：5 条 `cargo clippy --tests` 告警（readonly_session_test.rs:84/211/291、executor_test.rs:838/863 未用绑定）+ 仓库根 0 字节 `:memory:.wal`（2026-09-05 旧夹具副作用产物，git 已跟踪，随收尾批次提交删除） | 003 | Act 001 Remaining Issues；Surgical Changes 孤儿清理 |

## Iteration Plan

### Iteration 000: 只读打开模式（多读单写 + 零写入会话）

- Tasks: T1, T2
- 交付（2026-09-30 Act Response `reported` + Plan Review `accepted`）：T1 + T2 均已实施并经全量分批回归（1554 passed / 0 failed / 2 ignored）；Plan Review 独立 diff 审读核对一致，两条非阻塞 Minor finding 记录于 Plan Review（偏差 6 组合拒绝缺自动化用例、`--readonly` 文档面收尾同步）。逐项结论见 `iterations/000-readonly-open-mode/000-initial.md`。Iteration 001 已展开（`iterations/001-memory-db/000-initial.md`）。
- Depends on: None
- Stable baseline: `Database::open_readonly[_with_key]` 与 CLI `--readonly` 可用——两连接并发只读同库、锁矩阵（读读并行/读写互斥双向）、WAL 未决拒绝、只读会话零写入（checksum 见证）、DML/DDL 计划期 25006 拒绝、ATTACH 只读传递、加密库只读；默认读写路径逐字节回归（`database_file_lock_test` 全矩阵通过）；字段 Option 化后全量 GREEN。
- Verification boundary: `tests/readonly_session_test.rs` 全绿 + `database_file_lock_test.rs` 零回归 + 受影响边界（attach_session/cross_db/tx_statement）+ 全量分批（M19）零回归。
- Diagnostic boundary: `src/storage/file_storage.rs`、`src/database.rs`、`src/pipeline.rs` 入口守卫、`src/cli/mod.rs`（主命令 + AttachSession）、`src/wal/reader.rs`（谓词消费）。
- Non-goals: `:memory:`（Iteration 001）；WAL 水位（002）；dump/schema 共享锁化（proposal 假设 10）。

### Iteration 001: `:memory:` 库形态

- Tasks: T3, T4
- Depends on: Iteration 000（OpenMode 与字段 Option 化形态复用）
- 交付（2026-09-30 Act Response `reported`，待 Plan Review）：T3 + T4 均已实施并经全量分批回归（1569 passed / 0 failed / 2 ignored）——T3 落定 `MemoryStorage`（页拷贝 / free list 优先 / 回收写零 / `sync` no-op / `page_count` 单调分配计数）+ `OpenMode::Memory` + `Database::open_in_memory()`（零 WAL/恢复/checkpoint 通道，`close`/`checkpoint` 结构性 no-op）；T4 落定 CLI `:memory:` 特判（`resolve_db_path` 之前）、`--key`/`--readonly`/六生命周期子命令拒绝面（exit 2，守卫早于建父目录）、`execute_command_inner_mode` 内存打开臂；新增 `tests/memory_db_test.rs` 15 例。逐项结论见 `iterations/001-memory-db/000-initial.md`。
- Stable baseline: `open_in_memory()` 与 CLI `:memory:` 全形态可用——CRUD/DDL/事务/CTAS/ATTACH 组合、多语句计算管道、零文件（无 `.db`/`.wal`/`.checkpoint`）、close/checkpoint no-op、生命周期与 `--key` 拒绝面、重启即弃。
- Verification boundary: `tests/memory_db_test.rs` 全绿 + attach/ctas 组合 + 全量分批零回归。
- Diagnostic boundary: `src/storage/memory_storage.rs`（新）、`src/database.rs` 构造序、`src/cli/mod.rs` 主命令臂。
- Non-goals: 内存库持久化/加密；水位（002）。

### Iteration 002: WAL 大小水位自动 checkpoint

- Tasks: T5, T6
- Depends on: Iteration 000（`Database::commit` 单点形态与字段 Option 化基线）
- 交付（2026-09-30 Act Response `reported`，待 Plan Review）：T5 落定 `WalWriter` 字节计数器（`bytes_appended` 追加累加 / checkpoint 步骤 9b 归零）+ 16 MiB 阈值（`0` 禁用）+ `Database::commit` 尾部水位检查（`maybe_auto_checkpoint` 四项合取判定，失败点名 `checkpoint failed`）+ pipeline 两站点收敛（CTAS 拆两段，checkpoint 失败不触发 `drop_table`）；T6 落定 `RTSQL_WAL_CHECKPOINT_BYTES` env 通道（打开前非法值点名拒绝 exit 2 / `execute_command_inner_mode` 单点应用）+ 死记录数阈值接口整体删除 + README 双语与 rtsql-docs SKILL 文档行。新增 `tests/wal_auto_checkpoint_test.rs` 11 例；全量 M19 分批 **1590 tests / 0 failed**（1250 集成 10 批 + 340 lib）。逐项结论、偏差与待裁定 Minor 见 `iterations/002-wal-auto-checkpoint/000-initial.md`。
- Stable baseline: 长会话 WAL 有界——提交边界累计追加字节达水位即执行既有九步 checkpoint（`.wal` 物理回落）、活跃事务顺延、`0` 禁用、env 与 lib 双通道配置、非法值打开期拒绝且零副作用、失败点名传播且已提交数据不丢（含 CTAS 表存活）；手动 / close / MS18 零工作 / 位点水位 / 恢复重放语义零回归。
- Verification boundary: `tests/wal_auto_checkpoint_test.rs` 全绿 + `checkpoint_redo_reduction_test.rs` / `checkpoint_skip_test.rs` 零回归 + 受影响边界（explicit_tx / recovery_e2e / wal_recovery_large / ctas / cte / checkpoint / wal_writer）零回归 + 全量分批（M19）。
- Diagnostic boundary: `src/wal/writer.rs`（计数器与阈值）、`src/wal/checkpoint.rs`（归零位）、`src/database.rs::commit`/`maybe_auto_checkpoint`、`src/pipeline.rs` 两站点、`src/cli/mod.rs`（env 读取校验与应用）。
- Non-goals: 时间水位/后台 checkpoint；dump/schema 共享锁化；WALBuffer 组提交参数调整；`commit_by_id` 接入水位。

### Iteration 003: 派生基座投影收口 + 变更收尾批

- Tasks: T8, T9, T10, T7（T8 为 Act 001 报告的 Issue 候选修复；T9/T10/T7 为两轮 Iteration 的 Minor/Remaining 收尾批，与 T8 无耦合、同批收口——MS25 Iter004 收口批先例）
- Depends on: Iteration 001（`get_plan_output_columns` 消费面与测试基线）
- **执行序**：本 Iteration **先于 Iteration 002 执行**——T8 是读面「静默错误结果」收口（正确性红线优先纪律；编号与执行序无关，MS21-T03 阶段内优先执行先例）。2026-09-30 用户指令：Act 001 报告的 Issue 候选在本 change 内解决，不落账 Recorder。
- 交付（2026-09-30 Act Response `reported`，待 Plan Review）：T8 派生基座外层投影收口（`DerivedScanNode.projection` 载体 + 解析/装配/表头三处根因 + Sort 承载门 + 执行器经既有 `apply_projection` 裁剪 + 三构造点恒等默认；R5 五场景 11 例见证）；T9 `--readonly`×子命令组合拒绝 exit 2 自动化用例（另见证 `new` 守卫早于父目录创建）；T10 clippy 5 条告警清零 + 孤儿 `:memory:.wal` `git rm`；T7 双语 README 与 rtsql-docs 命令面小节。全量 1581 tests / 0 failed（99 集成目标 M19 分批 1241 + lib 340），`openspec validate` 通过。逐项结论与遗留 Minor 见 `iterations/003-derived-projection-closeout/000-initial.md`。
- Stable baseline: 三通道派生基座（FROM 子查询/CTE/视图）外层投影行与表头裁剪一致（含 WHERE/ORDER BY/CTAS 组合、未知外层列 42703 点名、`*`/聚合/别名项恒等面不回归）；`--readonly`×子命令组合拒绝有自动化见证；`cargo clippy --tests` 0 warning；孤儿 `:memory:.wal` 移除；README 双语与 rtsql-docs 命令面含 `--readonly`/`:memory:` 且与实现一致。
- Verification boundary: 修复定向测试（derived_scan_alias/cte/view/subquery/join_output_resolution）+ 全量分批（M19）+ 文档内容人工核对 + clippy。
- Diagnostic boundary: `src/parser/planner/query.rs`（投影解析与装配）、`src/executor/{plan,derived_scan}.rs`、`src/pipeline.rs` 构造臂、`tests/readonly_session_test.rs`、三处文档命令面小节。
- Non-goals: WAL 水位（002，其后执行）；JOIN 组合布局解析扩展；视图/CTE 语义其他面；stats/sample/profile 的 `:memory:` 点名拒绝。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| readonly-open-mode R1 入口与共享锁 | T1, T2 | `tests/readonly_session_test.rs`（并发只读/锁矩阵/缺失文件/加密只读）+ 组合拒绝用例（T9） | Covered |
| readonly-open-mode R2 WAL 未决拒绝 | T1, T2 | `tests/readonly_session_test.rs`（未决拒绝/恢复后可用） | Covered |
| readonly-open-mode R3 只读会话零写入 | T1 | `tests/readonly_session_test.rs`（checksum 见证/checkpoint no-op） | Covered |
| readonly-open-mode R4 只读拒绝写操作 | T2 | `tests/readonly_session_test.rs`（DML/DDL 拒绝矩阵 + 只读语句不受限） | Covered |
| readonly-open-mode R5 ATTACH 只读 | T2 | `tests/readonly_session_test.rs`（附加库只读/锁冲突/未决） | Covered |
| readonly-open-mode R6 既有语义零回归 | T1, T2 | `tests/database_file_lock_test.rs` + 全量 | Covered |
| database-file-lock R1（MODIFIED 按模式收窄） | T1, T2 | `tests/database_file_lock_test.rs` 全矩阵 + 新锁矩阵 | Covered |
| error-reporting R2（MODIFIED 25006） | T2 | `tests/readonly_session_test.rs`（25006 断言） | Covered |
| join-output-shape-resolution R5 派生基座外层投影（ADDED，本 change） | T8 | `tests/derived_scan_alias_test.rs` 扩展（三通道裁剪/WHERE/ORDER BY/CTAS/42703/恒等面） | Covered |
| in-memory-database R1 打开与全形态 | T3, T4 | `tests/memory_db_test.rs`（CRUD/DDL/多语句/即弃） | Covered |
| in-memory-database R2 checkpoint/WAL 无操作 | T3 | `tests/memory_db_test.rs`（零文件副作用/事务语义） | Covered |
| in-memory-database R3 边界拒绝面 | T4 | `tests/memory_db_test.rs`（生命周期/`--key` 拒绝） | Covered |
| in-memory-database R4 ATTACH 零特殊化 | T4 | `tests/memory_db_test.rs`（ATTACH/CTAS 组合）+ 既有三测试文件 | Covered |
| wal-auto-checkpoint R1 水位触发 | T5 | `tests/wal_auto_checkpoint_test.rs`（触发回落/零工作保持） | Covered |
| wal-auto-checkpoint R2 配置 | T6 | `tests/wal_auto_checkpoint_test.rs`（env 覆盖/0 禁用/非法拒绝） | Covered |
| wal-auto-checkpoint R3 事务顺延 | T5 | `tests/wal_auto_checkpoint_test.rs`（顺延场景） | Covered |
| wal-auto-checkpoint R4 失败传播 | T5 | `tests/wal_auto_checkpoint_test.rs`（夹具失败注入） | Covered |
| wal-auto-checkpoint R5 零回归与死接口删除 | T6 | `checkpoint_*_test.rs` + 全量 + 编译期删除见证 | Covered |

平衡审计：000/001/002/003 各形成独立可验证阶段成果（锁形态 / 存储后端 / checkpoint 触发面 / 派生投影收口+收尾批），诊断边界互不重叠，依赖单向（000 → 001、000 → 002、001 → 003）；**执行序 000 → 001 → 003 → 002**（T8 静默错误结果收口优先于水位特性，正确性红线纪律）；无过碎或过重（最大 003 为 4 任务——1 中等修复 + 3 轻量收尾，MS25 Iter004 收口批先例；2026-09-30 用户指令扩入）。
