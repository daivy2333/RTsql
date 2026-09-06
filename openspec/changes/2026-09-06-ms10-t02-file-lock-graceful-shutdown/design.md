# design: MS10-T02 跨进程文件锁 + 优雅停机

## Current Behavior（基线 `1a9c91f`，2026-09-06 实测 614 tests pass / 0 failed）

- `FileStorage::open`（`src/storage/file_storage.rs:20-47`）：`OpenOptions read+write+create+truncate(false)` 打开主文件，4KB 对齐校验，**无任何跨进程锁**（仅 free_pages 进程内 `Mutex`，`:93/:110`）。持有 `file: Arc<std::fs::File>` 唯一主文件 fd。
- 生产调用链唯一：`cli::execute_command`（`src/cli/mod.rs:89-116`）→ `Database::open`（`src/database.rs:28-88`）→ 第一步 `FileStorage::open`（`:30`，先于 `WalWriter::open` 与 `RecoveryManager::full_recover`）。
- open 错误全部 → `ExitStatus::General`（exit 1，`src/cli/mod.rs:95-104`）；`ExitStatus::Locked` 枚举留位（`:18-52`）无产生路径。
- 全仓无信号处理代码（grep 零命中）；tokio features 无 `"signal"`（Cargo.toml）。SIGINT/SIGTERM 走默认终止，`close()`（= checkpoint，`src/database.rs:177-189`）不触发。
- 文件形态：`<db>`（4KB 裸页流）+ `<db>.wal`（`writer.rs:27`）+ `<db>.checkpoint`（16B 位点，`checkpoint.rs:52`）。

### 工作区存量（未提交，2026-09-06 Plan 盘点 + 实测）

replan 前被中断的 Act 尝试（原 Iteration 000-lock-shutdown draft Cycle）遗留未提交改动，本轮并入计划。测试注释引用的「Act Response」已随旧 Cycle 失效，标定数据以本节与 D5 为准：

- **T2 存量（完成态，实测绿）**：`file_storage.rs` open 内 try_lock 分支（+11 行，与 D1 一致）、`error.rs` `DatabaseLocked`（+3 行，与 D2 一致）、`tests/database_file_lock_test.rs` 4 用例（同进程双开拒绝 / 释放重开 / FileStorage 直开双拒 / 错误消息含路径）。
- **T3 存量（完成态，实测绿）**：`cli/mod.rs` `DatabaseLocked → Locked` 映射（+5/-1，与 D3 一致）、`cli_test.rs::test_lock_conflict_exit_4`。
- **T4 存量（测试就位、生产缺位）**：Cargo.toml tokio `signal` feature + dev-dep `libc`；`cli_test.rs` 信号段——`build_big_wal`（50 行/事务分块显式事务，缓冲记录数恒低于 WALBuffer capacity=100，规避 do_flush 并发窗口；默认 `WAL_ROWS=20_000`）、`SIGNAL_DELAY_MS=200`、`assert_row_count_intact`、3 个信号 e2e + `test_sigkill_leaves_recoverable_db` + 2 个 `#[ignore]` 诊断（`calibration_recovery_time` 恢复耗时标定、`diagnostic_wal_parse` WAL 逐帧解析）。`cli/mod.rs` 无任何信号接线（D4 生产代码未动，信号用例天然 RED）。
- **既有测试标定（T2 必要后果，断言语义零修改，实测绿）**：`btree_test` 重开前 drop 持有者；`storage_test` durability 用例先 drop；`schema_persistence_test` 页数探针作用域化；`drop_table_free_test::page_count` 第二句柄探针改 `std::fs::metadata` 长度高水位（断言调用点零修改）。
- **实测基线（2026-09-06 本机，受影响 6 目标）**：`database_file_lock_test` 4/4、`btree_test` 10/10、`storage_test` 21/21、`schema_persistence_test` 8/8、`drop_table_free_test` 6/6 全绿；`cli_test` 13 绿 / 4 RED / 2 ignored——RED 为 3 个信号用例 + `test_sigkill_leaves_recoverable_db`，失败模式均为 T0 后果（子进程在信号落地前因 `Incomplete WAL record` 以 exit 1 死亡；锁锚点用例因恢复过短 10s 轮询超时）。T0 落地后 sigkill 用例应转绿，3 个信号用例转为真正的 T4 RED（`code==None`）。

## Target Behavior

打开被占用库 → `DatabaseLocked` → CLI exit 4；SIGINT/SIGTERM → 中止当前阶段 →（已打开则）`close()` checkpoint → exit 130/143；正常路径与 614 基线完全一致。

## 关键设计决策

### D0（Replan 新增）：WalReader 逐帧无歧义解析

**根因**（实验实证，2026-09-06）：`reader.rs:63-65` 以「byte[8] 是合法 type 且 byte[0] 不是」判别新/旧格式。新格式帧首字节 = 内嵌 LSN 的 LSB；文件偏移低字节 ∈ {0x01..0x09}（合法 type 值）时误判旧格式 → 从 LSN 高位读垃圾长度 → derail → `Incomplete WAL record`。首个碰撞 ~offset 1033（≈第 19 条记录）。严格帧遍历实证 WAL 完好（2046 帧 lsn==偏移、CRC 全对），误判 79 帧。生产流为**混合格式**：`do_flush → write_batch` 写新格式，`checkpoint.rs:118 → write_record` 写旧格式 Checkpoint 记录——逐帧歧义是真实负载，不能简单"只认新格式"。

**修复语义**：歧义偏移（byte[0] 为合法 type 值）上先按新格式解析并以 **CRC 验证**为接受判据；失败（CRC/结构/EOF）则 seek 回帧首按旧格式解析；两路皆败 → 显式报错（K05 不变）。非歧义偏移维持现有分支。CRC 误通过概率 2^-32 可忽略。

**拒绝备选**：只认新格式（破坏 Checkpoint 旧格式记录）；Checkpoint 记录改新格式（格式变更，属 T03）；文件头 magic（T03）。

### D1：锁实现与落点——`FileStorage::open` 内 `std::fs::File::try_lock`（用户批准范围 2026-09-06）

```rust
let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
match file.try_lock() {
    Ok(()) => {}
    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
        return Err(StorageError::DatabaseLocked(path.display().to_string()));
    }
    Err(e) => return Err(StorageError::Io(e)),  // 严格失败（用户决策 2），不降级无锁
}
```

- **API 选型**：std 原生 `File::try_lock`（stable 1.89，本机 rustc 1.90 实证：同 fd 复锁幂等 `Ok(())`、跨 fd 冲突 `Err(WouldBlock)`、Linux 底层 advisory flock）。零新依赖，符合"原生能力优先"。
- **落点理由**：fd 所在地；生产唯一调用点先于 WAL 打开与恢复（第二持有者在触碰 `.wal`/`.checkpoint` 前即被拒）；13 处测试直调 `FileStorage::open` 均为独立文件不受影响。
- **生命周期**：锁随 fd——`Database`/`FileStorage` drop、进程退出、kill 均自动释放；`Database: Clone` 共享同一 `Arc<FileStorage>` 单 fd，clone 复锁幂等不自冲突。
- **拒绝备选**：fcntl 锁（POSIX 语义复杂、需 libc/nix 依赖）；Database::open 层加锁（需额外 LockGuard 结构保活 fd，无收益）。

### D2：错误变体（additive）

`StorageError::DatabaseLocked(String)`（路径），`#[error("database is locked: {0}")]`。已核查：无任何穷尽 `match StorageError` 消费方（测试侧仅 `matches!` 单变体断言），open 失败测试为字符串断言（`checkpoint_redo_reduction_test.rs:199-205`），additive 变体零影响。

### D3：CLI 锁冲突映射

`execute_command` 的 `Database::open` Err 分支扩展：

```rust
Err(e) => return match e {
    StorageError::DatabaseLocked(_) =>
        ExitStatus::Locked(format!("database is locked: {}", db_path.display())),
    _ => ExitStatus::General(format!("failed to open database {}: {}", db_path.display(), e)),
},
```

`Locked` 经既有 `emit_stderr` 输出、映射 exit 4（`src/cli/mod.rs:40-51` 现成）。其余 open 错误（页对齐、权限、redo 失败）维持 General(1)——既有 `test_corrupt_file_open_fails_exit_1` 守护。

### D4：优雅停机——两阶段 select（用户决策 1：POSIX 130/143）

`Cargo.toml` tokio features +`"signal"`（现有依赖的 feature，非新依赖）。`ExitStatus` 新增 `Signaled(i32)`（信号编号），`ExitCode = 128 + signum`：SIGINT(2)→130、SIGTERM(15)→143。信号退出**无 stderr 消息**（POSIX 惯例，130/143 自解释）。

```rust
async fn execute_command(args: &CliArgs) -> ExitStatus {
    let db_path = resolve_db_path(&args.db)?;          // General，现状不变
    let db = tokio::select! {                          // 阶段 1：open
        r = Database::open(&db_path) => match r {
            Ok(db) => db,
            Err(e) => return open_error_status(&db_path, e),   // D3
        },
        _ = sigint()  => return ExitStatus::Signaled(2),   // 无 close：安全论证见下
        _ = sigterm() => return ExitStatus::Signaled(15),
    };
    let status = tokio::select! {                      // 阶段 2：执行
        r = run_sql(&db, &args.sql, args.format) => r,
        _ = sigint()  => ExitStatus::Signaled(2),
        _ = sigterm() => ExitStatus::Signaled(15),
    };
    match db.close().await { /* 现有语义扩展一条： */ }
    // close Err 时：Success → General（现状）；Signaled → **保持 Signaled**，
//   close 错误 emit_stderr 提示（信号语义主导，数据由 WAL 兜底）；other → other（现状）
}
```

- 信号源：`tokio::signal::ctrl_c()`（SIGINT）+ `tokio::signal::unix::signal(SignalKind::terminate())`（SIGTERM）。每次 select 新建信号 future 或 pin 共享 stream——**非实质选择，留给 Act**。
- **打开阶段取消安全性**（无 close 立即退出的论证）：① WAL 只被 checkpoint/close 截断，recovery 从不截断——中断恢复后下次 open 从位点完整重放收敛（与 kill -9 mid-recovery 等价，既有 crash e2e 覆盖该语义）；② `spawn_blocking` 页写不因 future drop 取消，但内容是 redo 重放，未完成部分下次恢复重做；③ flock 随 fd drop/进程退出释放。
- **执行阶段取消**：SELECT 无 WAL 写；DML 被取消 = 无 commit record 的未提交事务，恢复期按 uncommitted 清理（既有语义）。
- **close 期间信号**：不新增 select（close 为快路径）；tokio 信号驱动注册为进程级持久，二次 Ctrl-C 不强杀——已知行为，kill -9 兜底（proposal 风险已记录）。
- **安装窗口**：信号 future 在 `execute_command` 入口创建；进程启动至安装之间的 SIGINT 按默认终止——所有 CLI 的固有竞态，不处理。

### D5：信号停机验证构造（Replan 修订版——含阶段判别观测物与竞态防御）

**实验结论**（2026-09-06 实测）：执行阶段无法用真实工作负载构造确定性秒级窗口——`SELECT COUNT` 扫描 2000 行 ≈ 1ms（凑 500ms 需 ~100 万行 ≈ 70s 构建）；标量子查询不支持（`Unsupported expression type`）；多行 VALUES INSERT 1000 行 ≈ 55ms、10k 行触发 `Page full`。**因此 phase-2（执行+close）的信号见证采用库级编排结构测试，phase-1（open）与 kill e2e 用真实二进制**：

**工作区并入版（2026-09-06，与已就位测试对齐）**：

1. **e2e（真二进制）——执行阶段 SIGINT/SIGTERM**：夹具 = lib 侧 `build_big_wal`（50 行/事务分块显式事务——缓冲记录数恒低于 WALBuffer capacity=100，规避 do_flush 并发窗口；默认 `WAL_ROWS=20_000`）→ drop 不 close（无 checkpoint）→ spawn rtsql → `sleep SIGNAL_DELAY_MS=200ms` → `libc::kill(SIGINT/SIGTERM)`。断言：exit 130/143；重开数据完整（`assert_row_count_intact`）。标定：`#[ignore]` 用例 `calibration_recovery_time`（`RTSQL_CALIBRATION_ROWS` 可调）实测恢复耗时 T，要求 T ≥ 500ms 且 200ms ∈ [T/4, T/2]，不满足时倍增 N 重标定（Act 在 Act Response 记录 T）。
2. **e2e（真二进制，确定性锚点）——打开阶段 SIGINT**：同 1 夹具 → spawn → 轮询「子进程已持有主文件锁」（对主文件 `try_lock()==WouldBlock`，上限 10s）→ 立即 SIGINT → 断言 exit 130。锚点保证信号落在 open（恢复）阶段；T0 修复后恢复耗时数百 ms 级，窗口稳定。**观测物：WAL 长度仍大（>2KB，close 未执行的直接证据——spec「无 close」的观测物；Act 在既有用例补 1 行断言）**。重开数据完整性由用例 1 与 4 覆盖，本用例不重复。
3. **e2e（真二进制）——kill -9 恢复**：同 1 夹具 → SIGKILL → 断言 `code==None`（signal death）→ 重开 exit 0 + 数据完整（锁无残留 + 恢复成功；依赖 T0，T0 后本用例转绿）。
4. **锁冲突 e2e**：测试进程 `std::fs::File::try_lock` 持有 → spawn → exit 4（确定性，无时序）——工作区已就位且实测绿（`test_lock_conflict_exit_4`）。
5. **库级结构测试（确定性，待建）——执行+close 阶段信号**：`execute_command` 的编排重构为信号 future 可注入（`#[cfg(test)]` 可见，零公共 API 变化）；测试注入 pending 慢工作负载 + 确定信号 future → select 走信号臂 → 断言 `close()` 已执行（WAL 截断 <1KB）+ 返回 `Signaled(2)`。

### D6：测试布局（工作区并入版）

- 新 `tests/wal_recovery_large_test.rs`（lib 层，T0，**待建**）：≥19 条记录的大 WAL 恢复成功（RED→GREEN 主场景）+ 小 WAL（5 行）守护用例。
- `tests/database_file_lock_test.rs`（**工作区已就位，实测 4/4 绿**）：同进程双开拒绝、释放重开、`FileStorage` 直开双拒、错误消息含路径。
- `tests/cli_test.rs`（既有 12 用例零修改，实测绿；**新增段已就位**）：`test_lock_conflict_exit_4`（绿）；信号段 4 用例（当前 RED：T0 后果；T0+T4 后转绿）——执行阶段 SIGINT 130 + 重开完整 / 打开阶段 SIGINT 130（锁锚点）+ WAL 仍大 / SIGTERM 143 / SIGKILL → code None + 重开完整；2 个 `#[ignore]` 诊断（恢复耗时标定、WAL 逐帧解析）；phase-2 信号结构测试在 `src/cli/mod.rs` `#[cfg(test)]`（**待建**）。
- **既有测试标定（T2 必要后果，断言语义零修改，实测绿）**：`tests/btree_test.rs`（重开前 drop 持有者）、`tests/storage_test.rs`（durability 用例先 drop）、`tests/schema_persistence_test.rs`（页数探针作用域化）、`tests/drop_table_free_test.rs`（`page_count` 探针由第二句柄改 `std::fs::metadata` 长度高水位；断言调用点零修改）。**除此 4 文件外既有测试零修改。**
- `run_cli`/`spawn_cli`/`wait_cli` fixture（60s 超时守护、TempDir、RTSQL_HOME）直接复用。
- **信号发送**：dev-dependencies +`libc = "0.2"`（工作区已接入），测试进程 `libc::kill(child.id() as i32, SIGINT/SIGTERM/SIGKILL)`——`std` 无发信号能力（`Child::kill()` 仅 SIGKILL），libc 已由 tokio 传递引入（lockfile 零新包），产品依赖零变化。

## 责任边界

- **修改**：`src/wal/reader.rs`（逐帧无歧义解析，D0）、`src/storage/file_storage.rs::open`（try_lock）、`src/storage/error.rs`（+变体）、`src/cli/mod.rs::execute_command`（D3/D4）+ `ExitStatus`（+Signaled）、`Cargo.toml`（+signal feature）、既有测试 4 文件夹具锁适配（D6 列表）。
- **保持**：`Database::open` 成功路径与签名、`AsyncStorage` trait、WAL/恢复/checkpoint 全部语义（除 D0 解析修复外）、`run_sql`/渲染/resolve、退出码 0/1/2/3/5 映射、`close()` 正常路径语义、混合格式流兼容。
- **禁止修改**：network server 路径、pipeline/executor、页格式、`WALBuffer::do_flush` 并发语义（improvement 候选，另行处理）、既有测试**断言语义**（D6 所列 4 文件的夹具锁适配为唯一既定例外）。

## 行为变化汇总

| 输入 | 现行为 | 目标行为 |
|---|---|---|
| WAL ≥19 条记录后重开恢复 | `Incomplete WAL record`，库打不开 | 恢复成功，数据完整（D0） |
| 打开被占用库 | 静默双写（损坏风险） | `DatabaseLocked` → stderr `database is locked: <path>` → exit 4 |
| 同进程双开同库 | 静默双实例（损坏风险） | 第二次 `Database::open` → `DatabaseLocked` |
| flock 不支持的 FS | （不存在锁） | open 失败 exit 1（严格，不降级） |
| 执行中 SIGINT/SIGTERM | 默认终止，无 checkpoint | close() checkpoint → exit 130/143 |
| 打开中 SIGINT/SIGTERM | 默认终止 | 立即 exit 130/143（无 close，恢复可重放） |
| 无信号全部路径 | — | 零变化（614 基线守护） |
| 既有测试探针在第一持有者存活期间二次打开 | 正常通过 | 夹具按独占锁适配（4 文件，断言语义不变，D6） |
