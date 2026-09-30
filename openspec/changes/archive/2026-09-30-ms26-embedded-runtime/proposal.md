# MS26：嵌入式运行形态（只读并发 + WAL 水位自动 checkpoint + :memory:） — Proposal

## Why

- **嵌入式多进程形态缺席**（tasks.md MS26，2026-09-24 初版路线规划）：`open` 一律 `try_lock` 独占（MS10-T02，spec `database-file-lock`）——两个进程连并发只读 SELECT 都互斥，「多读单写」的 SQLite 型嵌入模型不存在；只读分析、多会话观察、备份窗口场景被锁面挡死。
- **长会话 WAL 无界增长**：checkpoint 仅 close()/手动触发（`Database::checkpoint`）；MS18 零工作跳过只解决「无工作」方向，「工作多」的长会话（agent 长任务、常驻进程）WAL 只增不减，恢复重放代价随会话长度线性放大。
- **`:memory:` 缺席**：测试/临时计算形态无承载；one-shot 多语句计算管道（`CREATE;…;INSERT;…;SELECT`，MS10-T04 分片）被迫落盘真实文件并留下清理负担。
- 三面同属「运行形态」主题，聚合为单一阶段成果；roadmap 估 2-3 小 change 可合可分 → 经用户裁定**单 change 三 Iteration 承载**（MS21 先例）。

## What Changes

用户已裁定（2026-09-30，四项决策）：**WAL 未决时只读打开 = 拒绝**；**锁冲突 = 立即拒绝**；**水位 = 16 MiB + env 可覆盖（0 = 禁用）**；**只读 ATTACH = 附加库也只读打开**。

- **T1 只读打开模式**（`readonly-open-mode` 新域 + `database-file-lock` MODIFIED + `error-reporting` MODIFIED）：lib `Database::open_readonly[_with_key]` + CLI 主命令 `--readonly` flag。只读打开取 advisory 共享锁（`try_lock_shared`），默认读写打开维持独占——读读并发、读写互斥、写写互斥，冲突立即 `DatabaseLocked`（CLI exit 4，不加等待语义）。只读打开前提 = WAL 无未决恢复（见假设 2），否则拒绝；只读会话不运行恢复、不产生任何文件写入；DML/DDL 计划期点名拒绝（SQLSTATE 25006）；`ATTACH` 附加库随会话只读打开。
- **T2 WAL 大小水位自动 checkpoint**（`wal-auto-checkpoint` 新域）：字节水位默认 16 MiB，env `RTSQL_WAL_CHECKPOINT_BYTES` 与 lib 打开参数可调，0 = 禁用；提交边界（auto-commit 语句提交后 / 显式 commit 后）检查，显式事务活跃时顺延；触发即执行既有九步 checkpoint（MS18 零工作跳过保持）；失败诚实传播。既有死的记录数阈值接口（`should_checkpoint`/`set_checkpoint_threshold`/`get_checkpoint_threshold`，无消费方）随之删除。
- **T3 `:memory:` 库形态**（`in-memory-database` 新域）：lib `Database::open_in_memory()` + CLI 主命令 `:memory:` 路径特判；新 `MemoryStorage` 实现 `AsyncStorage`（MS10 起即 trait 对象的现成接缝，`src/storage/async_storage.rs`）；内存库不创建/不要求任何文件（`.db`/`.wal`/`.checkpoint`）；checkpoint no-op；全 CRUD/DDL 会话内可用，进程退出即弃；生命周期子命令与 `--key` 对 `:memory:` 点名拒绝。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **Iteration 划分**：000 只读模式（T1）→ 001 `:memory:`（T3；与 T1 同为 open 路径诊断面相邻，模式枚举复用）→ 002 WAL 水位（T2，独立提交路径面）。
2. **只读打开的 WAL 未决谓词**：解析 `.wal` 全部记录，仅允许空文件 / `.wal` 不存在 / 仅含 `Checkpoint` 标记记录（干净关闭后步骤 6/7 的残留形态）；任何 `Begin`/数据/未提交记录 → 拒绝打开（错误点名「存在未决 WAL，需先以读写模式打开完成恢复」）。判定在任何页写入之前完成，只读会话不执行重放。依据：恢复重放经 BufferPool 驱逐刷脏页落盘（`evict_one`），只读会话无法保证不写盘；两个只读者并发恢复 = 共享锁下并发写文件。
3. **只读会话零写入**：主文件以只读句柄打开（不 create）；`.wal` 以只读打开（缺失视为空，不创建）；不运行恢复；`close()`/`checkpoint()` 对只读会话不产生任何文件写入（结构性守卫，不依赖 MS18 零工作跳过的巧合）。加密库只读打开按 `--key` 正常派生解密（只读解密复用既有 `PageCipher`），错误密钥 `InvalidKey` 不变。
4. **锁矩阵**：只读打开 = `try_lock_shared`；读写打开 = `try_lock` 独占（现状逐字节不变）。N 个只读者共存；任一独占者排除一切共享者；冲突立即 `DatabaseLocked`（CLI exit 4）。只读打开对缺失文件报错（不 create）。
5. **DML/DDL 拒绝面**：解析后、任何写入生效前点名拒绝（守卫位置随 Phase 2 调查定稿：pipeline 计划形状守卫或 planner 层）；SELECT / `ATTACH` / `DETACH` / 事务语句不受限。错误携带 SQLSTATE `25006`（read_only_sql_transaction，PG 同类语义）→ `error-reporting` R2 分类表扩一行；CLI 退出码 3（SQL 失败类）。
6. **`:memory:` 语义边界**：主命令专属——`list`/`schema`/`dump`/`restore`/`import`/`new`/`delete` 生命周期子命令对 `:memory:` 点名拒绝（`list` 按目录枚举不受影响但显式拒绝形态统一）；`--key` + `:memory:` 拒绝（无持久化，加密无意义）；ATTACH 在内存会话可用（附加文件库沿既有打开语义，CTAS 物化进内存主库自然成立）；内存库 `close()`/`checkpoint()` no-op；不产生任何伴生文件。崩溃/退出后数据即弃，无恢复语义（诚实边界，写进 spec）。
7. **只读 ATTACH 传递**：`--readonly` 会话内 ATTACH 的附加库一律只读打开（会话级模式传递，不支持逐库混合）；附加库自身 WAL 未决同样拒绝。内存会话内 ATTACH 沿读写打开（现状）。
8. **自动 checkpoint 触发与顺延**：检查点在提交边界（每次提交完成后）；存在其他活跃事务时顺延到后续边界；触发即同步执行九步 checkpoint（与 close 同型路径，含零工作跳过与位点水位闭包）；失败以错误响应报告该提交语句（已提交数据不受影响——WAL 已 fsync），后续边界自然重试。
9. **字节计数实现面**：内存计数器（`write_record`/`write_batch` 累加、checkpoint 步骤 9 处与既有 `reset_write_count` 同位重置），不做每语句 `metadata()` syscall；阈值比较用计数器而非文件长度。
10. **dump/schema 子命令不切共享锁**（现状独占保持）：避免 dump 在 WAL 未决态的行为变化（现为读写打开自动恢复，切只读会拒绝）——收益（写者会话期间备份）留待后续小项。
11. **默认路径零回归**：`Database::open`/`open_with_isolation`/`open_with_key` 语义与签名逐字节不变；`database_file_lock_test.rs` 全矩阵保持。

## 范围外（Non-goals）

- 多进程并发写（保持单写者）；读会话升级写（需重开）；跨进程 MVCC 快照语义精细化。
- 锁等待/超时语义（busy_timeout 型）——用户裁定立即拒绝。
- 只读会话的内存恢复（WAL 未决即拒绝，决策记录于假设 2）。
- checkpoint 的时间水位、后台任务化、增量/分页 checkpoint（仅大小水位 + 提交边界同步触发）。
- Server 面多租户与 `rtsql serve`（I071）；REPL（I067）。
- dump/schema/restore 等生命周期子命令的共享锁化（假设 10）。
- `:memory:` 的持久化、加密或跨会话共享形态。

## Impact

- **受影响代码面**（Phase 2 调查复核）：`src/storage/file_storage.rs`（共享锁打开面 + 只读句柄守卫）、`src/database.rs`（`open_readonly[_with_key]`/`open_in_memory` 入口、模式传递、close/checkpoint 守卫）、`src/storage/async_storage.rs`（`MemoryStorage` 新实现）、`src/wal/writer.rs`（字节计数器 + 只读 WAL 打开形态 + 死阈值接口删除）、`src/wal/checkpoint.rs`（内存/只读 no-op 守卫或调用方守卫）、`src/cli/mod.rs` + `src/cli/resolve.rs`（`--readonly` flag、`:memory:` 特判、错误面 exit 码）、pipeline/planner（只读 DML/DDL 拒绝守卫位置随调查定稿）、`src/network/protocol.rs`（25006 分类映射，如经 error-reporting 通道）。
- **受影响 specs**：新增 `readonly-open-mode`、`wal-auto-checkpoint`、`in-memory-database` 三域；MODIFIED `database-file-lock`（R1 按模式收窄）+ `error-reporting`（R2 增 25006 分类）。
- **测试**：新增 3 个测试文件（只读锁矩阵与拒绝面 / 水位触发与配置 / 内存库 e2e）；全量零回归（change 创建时基线 1541 tests / 0 failures / 2 ignored，M19 分批纪律）。

## 验证边界

- **T1**：双连接并发只读 e2e + 锁矩阵（读读并行 / 读写互斥双向 / 写写互斥）+ WAL 未决拒绝 + 只读会话零写入（前后文件 checksum 一致）+ DML/DDL 拒绝矩阵（含 SQLSTATE/exit 码）+ ATTACH 只读传递 + 加密库只读 + 默认路径零回归。
- **T2**：水位触发 e2e（长会话 WAL 有界：触发后 `.wal` 长度回落）+ env/lib 配置矩阵 + 0 禁用 + 显式事务顺延 + 失败传播 + MS18 零工作跳过保持 + 全量零回归。
- **T3**：内存库全 CRUD/DDL e2e + 多语句计算管道 + 重启即弃 + 无伴生文件 + 生命周期/`--key` 拒绝 + ATTACH/CTAS 组合 + 全量零回归。
