# tasks: MS10-T02 跨进程文件锁 + 优雅停机

> 状态：Replan（2026-09-06，Gate 2 前审计：WalReader 嗅探 P0 修复纳入范围，Iteration 拆分）已完成，Gate 2 待用户批准后交 openspec-act。
> 工作区并入（2026-09-06，Plan 修订）：replan 前被中断的未提交实现已盘点实测并并入计划——T2/T3 存量实现（RED 以「临时摘除 hunk」复现）、T4 测试脚手架（生产接线缺位）、4 个既有测试夹具锁适配（T2 必要后果，断言语义零修改）、T1 门对 3 个 Iteration-001 信号 RED 见证设白名单。详见 design「工作区存量」节 / D5 / D6。
> 审计依据：本会话实验（严格帧遍历 2046 帧完好/嗅探误判 79 帧；执行阶段秒级语句不可构造；多行 INSERT Page full 观察）+ 本轮受影响 6 测试目标实测（2026-09-06）。

## Iteration Plan

### Iteration 000: WAL 恢复逐帧无歧义（引擎正确性前提） — planned

- Tasks: T0, T1
- Depends on: None
- Stable baseline: 含 ≥19 条记录（>1KB）的 WAL 崩溃恢复成功、数据完整；混合格式流（新格式 + 旧格式 Checkpoint 记录）兼容；损坏帧仍显式报错（K05）；614 基线零回归
- Verification boundary: `cargo test --all` 除 T4-RED 白名单（T1 枚举的 3 个 Iteration-001 信号用例）外全绿（新增 `tests/wal_recovery_large_test.rs`；既有测试断言语义零修改，4 文件夹具锁适配见 design D6）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/wal/reader.rs`、`tests/wal_recovery_large_test.rs`
- Non-goals: `WALBuffer::do_flush` 并发互斥（improvement 候选）；Checkpoint 记录改新格式（T03 范畴）；magic 文件头（T03）

**平衡审计**：T0 是 001 的硬前提（kill-recovery e2e 与大 WAL fixture 依赖 ≥19 条记录的恢复能力），完成即形成独立可验收的引擎正确性成果（任意大小完好 WAL 恢复正确），故障域（`src/wal/`）与 001（`src/cli/`、`src/storage/`）不同。单独成 Iteration，不过碎（对照 MS07-T03 先例）。

### Iteration 001: 并发互斥与可中断的 CLI — planned

- Tasks: T2, T3, T4, T5
- Depends on: Iteration 000（kill-recovery e2e 与大 WAL 打开阶段 fixture 依赖大 WAL 恢复正确；基线 614 tests）
- Working-tree note（2026-09-06 Plan 修订）: T2/T3 生产代码与测试、T4 e2e 脚手架已存在于工作区（未提交、未验证，见 design「工作区存量」节）；Act 按契约复核收编（含 RED 复现），不重写
- Stable baseline: 打开被占用库 → `DatabaseLocked` / CLI exit 4；SIGINT/SIGTERM 优雅停机（close checkpoint → 130/143）；同进程双开被拒；正常路径零回归
- Verification boundary: `cargo test --all` 全绿（含 Iteration 000 白名单中的 3 个信号用例转绿；`tests/database_file_lock_test.rs` 已就位 + `tests/cli_test.rs` 增补；既有测试断言语义零修改，4 文件夹具锁适配见 design D6）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/storage/{file_storage,error}.rs`、`src/cli/mod.rs`、`Cargo.toml`、`tests/{database_file_lock_test,cli_test}.rs`
- Non-goals: T03 格式头、T04 多语句分片、T05 生命周期子命令、锁等待、Windows/非 Linux、server 路径、REPL、lib API 签名

**平衡审计**：锁（引擎层）、退出码映射（CLI）、信号停机（CLI）、回归门同属"进程生命周期安全"单一验收域；故障域重叠（文件 fd / 信号 / 进程退出码），锁与信号共享同一集成夹具与 open 路径。拆分产生不可独立验收中间态。不拆分。

## Tasks

### T0: WAL 恢复逐帧无歧义解析（Replan 新增）

- Requirement/Scenario: `wal-recovery-frame-parsing` 全部 3 场景
- Depends on: None
- Targets: `src/wal/reader.rs::WalReader::read_next_with_lsn`
- 当前行为: 格式嗅探（`reader.rs:63-65`「byte[8] 合法且 byte[0] 非法 → 新格式」）在文件偏移低字节 ∈ {0x01..0x09} 时把新格式帧误判为旧格式，恢复 derail → `Incomplete WAL record` → `Database::open` 失败（实验实证：2046 帧完好 WAL 被误判 79 帧，首例 offset 1033）
- 目标行为: 按 design D0——歧义偏移（byte[0] 为合法 type 值 0x01-0x09）先按新格式解析并以 CRC 验证为接受判据；失败（CRC/结构/EOF）seek 回帧首按旧格式解析；两路皆败显式报错。非歧义偏移维持现有分支。混合流（新格式 + 旧格式 Checkpoint 记录）兼容
- Required changes: `read_next_with_lsn` 判别与回退逻辑重排（约 +30 行）；`deserialize_with_lsn`/`deserialize` 语义不动
- Preserve: 新格式帧 CRC 验证语义；旧格式流兼容（既有 `wal_record_test`/`wal_handle_test` 守护）；损坏帧显式报错（K05）；`seek_to(lsn)`/位点语义不变
- Forbidden: 不改 writer/checkpoint/BufferPool；不引入格式版本头；不做 do_flush 并发修复
- Test witness（RED 先行）: 新建 `tests/wal_recovery_large_test.rs`——lib 侧建库 + 单条多行 `INSERT INTO t VALUES (0),(1),...,(499)`（约 500 条 WAL 记录，>1KB）→ `wal_buffer.shutdown().await` + drop 不 close → `Database::open` 重开。RED：当前返回 `Err(WalError ...)`（Incomplete WAL record）；GREEN：恢复成功 + `SELECT COUNT(*)` == 500。补第二条用例：小 WAL（<19 条，如 5 行）重开恒成功（防修复破坏既有路径）
- GREEN condition: 两用例绿 + `cargo test --all` 零回归（既有 checkpoint/recovery 套件覆盖混合流与损坏帧语义）
- Verification: `cargo test --test wal_recovery_large_test` 输出+退出码记 Act Response
- Stop when: 修复后发现旧格式回退路径与新格式 CRC 判定存在无法消解的双匹配（语义需重开）；或大 WAL 恢复暴露 reader 之外的位点语义错误

### T1: Iteration 000 验证门

- Requirement/Scenario: 全部（回归门）
- Depends on: T0
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- 当前行为: 基线 614 pass / clippy 0 / fmt 0 / validate PASS（`1a9c91f` 实测）；工作区实测（2026-09-06 Plan）：受影响 6 目标中仅 `cli_test` 4 RED（3 信号 + sigkill，均为 T0 后果），其余全绿
- 目标行为: **T4-RED 白名单**（`test_sigint_during_run_graceful_130`、`test_sigint_during_open_130`、`test_sigterm_during_run_143`——Iteration 001 T4 的 RED 见证，无 T4 生产代码必失败）之外全部通过，0 unexpected failed；`test_sigkill_leaves_recoverable_db` 在 T0 后**必须转绿**（其失败归因 T0，不属白名单）；测试总数 = 614 + 工作区已就位新增 + wal_recovery_large_test 用例数
- Test witness: 各命令决定性输出（≤20 行）与退出码记 Act Response
- GREEN condition: 四项达标（`cargo test --all` 按白名单口径 + clippy/fmt/validate 全 0/PASS）
- Stop when: 白名单之外的回归失败且无法归因于 T0 → BASELINE-CHANGED 返回 Plan

### T2: 打开即独占锁（引擎层）

- Requirement/Scenario: `database-file-lock` R1 全部 4 场景 + R2 全部 2 场景
- Depends on: Iteration 000
- Targets: `src/storage/file_storage.rs::FileStorage::open`、`src/storage/error.rs::StorageError`（+`DatabaseLocked(String)`）
- 当前行为: open 无跨进程锁；双持有者静默双写
- 目标行为: open 打开 fd 后立即 `try_lock()`（design D1）：`WouldBlock` → `Err(StorageError::DatabaseLocked(path.display().to_string()))`；其他 IO 错误 → `Err(StorageError::Io(e))` 原样传播；成功 → 锁随 fd 生命周期保持
- Required changes: open 内锁分支（约 8-12 行）+ error.rs additive 变体（`#[error("database is locked: {0}")]`）
- Preserve: `Database::open` 签名与成功路径零变化；`AsyncStorage` trait、页读写/分配/释放语义零变化；锁在 `WalWriter::open` 与 `RecoveryManager::full_recover` 之前完成（`database.rs:30` 落点保证）
- Forbidden: 锁等待/重试/busy timeout；`.wal`/`.checkpoint` 独立锁；network 路径；产品新依赖
- Test witness（**工作区已就位，4 用例实测绿**；RED 复现见下）: `tests/database_file_lock_test.rs`——① 同进程两次 `Database::open(同一 path)`：第一次 ok，第二次 `matches!(err, StorageError::DatabaseLocked(_))`；② drop 第一次实例后重开 ok；③ 两次 `FileStorage::open(同一 path)`：第二次 `DatabaseLocked`；④ 错误消息含路径。**RED 复现（存量实现下）**：临时摘除 `file_storage.rs` 的 try_lock 分支（11 行 hunk）→ ①③④ 失败（第二次 open 成功）、② 天然绿 → 恢复 hunk → GREEN；复现输出与退出码记 Act Response
- GREEN condition: 四用例绿 + `cargo test --all` 零回归
- Verification: `cargo test --test database_file_lock_test` 记 Act Response
- Stop when: `try_lock` 暴露非 `WouldBlock` 的冲突类错误语义（D1 错误分类需重开）；或存量实现与契约 Preserve/Forbidden 冲突且无法以局部调整消解

### T3: CLI 锁冲突退出码 4

- Requirement/Scenario: `cli-noninteractive-shell` 修改 R1 的场景"锁冲突退出码 4"
- Depends on: T2
- Targets: `src/cli/mod.rs::execute_command`（`Database::open` Err 分支 `:95-104`）
- 当前行为: （基线）所有 open 错误 → `ExitStatus::General` → exit 1；工作区存量已完成 `DatabaseLocked → Locked` 映射（与 D3 一致，`test_lock_conflict_exit_4` 实测绿）
- 目标行为: `StorageError::DatabaseLocked(_)` → `ExitStatus::Locked(format!("database is locked: {}", db_path.display()))` → exit 4 + stderr（经既有 `emit_stderr`/映射表，`src/cli/mod.rs:40-52` 现成）；其余 open 错误维持 General(1)（design D3）
- Required changes: open Err 分支 match 扩展；无其他
- Preserve: 页对齐/权限/redo 失败仍 exit 1（既有 `test_corrupt_file_open_fails_exit_1` 守护）；`resolve_db_path` 不动
- Forbidden: Locked 以外退出码映射；锁重试；文案偏离 `database is locked` 前缀
- Test witness（**工作区已就位，实测绿**）: `tests/cli_test.rs::test_lock_conflict_exit_4`——测试进程 `std::fs::File::open(目标 db)` + `try_lock()` 持有 → `run_cli` spawn → `code==Some(4)`、`stderr` 以 `database is locked` 开头、`stdout.is_empty()`；drop 持有者 → 再 spawn → `code==Some(0)`。**RED 复现（存量实现下）**：临时摘除 `cli/mod.rs` 的 Locked 分支（5 行 hunk）→ exit 1 且文案不同 → 恢复 hunk → GREEN；复现输出记 Act Response
- GREEN condition: 新用例绿 + 既有 12 用例零修改绿
- Verification: `cargo test --test cli_test` 记 Act Response
- Stop when: Locked 分支无法与既有错误路径隔离（结构冲突）

### T4: 优雅停机（信号 → close → 130/143）

- Requirement/Scenario: `cli-noninteractive-shell` 新增"优雅停机（信号接线 close）"全部 4 场景
- Depends on: T2（打开阶段取消依赖锁随 fd 释放；夹具共用）＋ Iteration 000（大 WAL 恢复正确）
- Targets: `src/cli/mod.rs::execute_command`（两阶段 select 重构 + 信号 future 注入点）、`ExitStatus`（+`Signaled(i32)`）、`Cargo.toml`（tokio features +`"signal"`）
- 当前行为: 无信号处理（**存量核对：`cli/mod.rs` 无任何信号接线，D4 生产代码缺位**）；SIGINT/SIGTERM 默认终止（子进程 `status.code()==None`），close 不触发；Cargo signal feature 与 cli_test 信号段已就位（实测 4 RED，当前失败模式为 T0 后果：子进程因 `Incomplete WAL record` 提前退出 1）
- 目标行为: design D4——`execute_command` 拆两阶段：阶段 1（open）select 信号臂 → `Signaled` 无 close 立即返回；阶段 2（run_sql）select 信号臂 → `db.close()` → `Signaled`；close Err 与 Signaled 并存 → 保持 Signaled + stderr 提示 close 失败（其余现状）。`Signaled(signum)` → exit `128+signum`（SIGINT=2→130、SIGTERM=15→143），`message()` 返回 None。tokio features +`"signal"`（工作区已接入）
- Required changes: execute_command 结构重构（db 提升为阶段间值；信号 future 注入点供 `#[cfg(test)]` 使用）、枚举 +1 变体 + 映射（Cargo.toml 已就位）
- Preserve: `run_sql`/渲染/resolve 不感知信号；无信号路径逐字节一致（既有 12 用例 + 全量守护）；close 正常路径语义（Success→General 映射）零变化；信号安装前窗口按默认终止（固有竞态，不处理）；close 期间信号不新增 select
- Forbidden: SIGHUP/其他信号；close 加 select/超时；130/143 以外信号退出码；WAL/恢复代码（D0 已在 000 完成）
- Test witness（e2e 4 用例**已就位、天然 RED 已实测**；结构测试与生产接线待建；按 design D5 并入版）:
  - ① 库级结构测试（`src/cli/mod.rs` `#[cfg(test)]`，确定性，**待建**）：信号 future 注入 + pending 慢工作负载 → phase-2 select 走信号臂 → 断言 `close()` 已执行（**WAL 截断 <1KB**——阶段判别观测物）+ 返回 `Signaled(2)`。RED：改造前无该编排结构（编译失败即为 RED）
  - ② e2e 打开阶段 SIGINT（**已就位** `test_sigint_during_open_130`，确定性锁锚点：轮询子进程持有主文件锁 ≤10s → 立即 `libc::kill(SIGINT)`）：断言 `code==Some(130)`；**Act 增补 1 行断言：WAL 长度仍大（>2KB，spec「无 close」的直接观测物）**；重开数据完整性由 ④ 覆盖，本用例不重复。标定：`#[ignore]` `calibration_recovery_time`（`RTSQL_CALIBRATION_ROWS` 可调）实测恢复耗时 T，要求 T≥500ms 且 200ms ∈ [T/4, T/2]，不足则倍增 N（T 记 Act Response）。RED：T0 落地后自然 RED（`status.code()==None`）
  - ③ e2e SIGTERM（**已就位** `test_sigterm_during_run_143`，夹具 `build_big_wal`：50 行/事务分块显式事务 × `WAL_ROWS=20_000`，drop 不 close；`SIGNAL_DELAY_MS=200`）→ `code==Some(143)`
  - ④ e2e kill -9（**已就位** `test_sigkill_leaves_recoverable_db`；守护性，T0 后、T4 前即应转绿）：SIGKILL → `code==None` → 重开 spawn → `code==Some(0)` + 数据完整（锁无残留 + 恢复成功；Iteration 000 已保证大 WAL 恢复正确）
  - ⑤ 无信号回归：既有 12 cli 用例零修改守护（已实测绿）
- GREEN condition: ①②③④ 绿 + 全量零回归（含 Iteration 000 白名单 3 用例转绿）
- Verification: `cargo test --test cli_test` + 标定数据（WAL 行数 N、恢复耗时 T、D）记 Act Response
- Stop when: 本机恢复耗时无法稳定 T≥500ms（时序构造需重设计——实质问题返回 Plan）；或 select 重构破坏既有 close 错误语义；或存量 e2e 脚手架与 D5 契约冲突无法以局部调整消解

**依赖注记**：信号发送用 dev-dependency `libc = "0.2"` + `libc::kill(pid, sig)`——libc 已由 tokio 传递引入（lockfile 零新包），产品 `[dependencies]` 零变化。

### T5: 全量回归与 Iteration 001 验证门

- Requirement/Scenario: 全部（回归门）
- Depends on: T2, T3, T4
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- 当前行为: Iteration 000 完成后——除 T4-RED 白名单（3 个信号用例）外全绿
- 目标行为: 全部通过（含白名单 3 用例随 T4 转绿），测试总数 = 614 + 新增总数，0 failed
- Test witness: 各命令决定性输出（≤20 行）与退出码记 Act Response
- GREEN condition: 四项全绿
- Stop when: 回归失败且无法归因于 T2-T4 → BASELINE-CHANGED 返回 Plan
