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

### D7（Review 裁定新增，2026-09-06）：WAL 重放位置寻址幂等修复（T0b，Replan Cycle 001）

**根因裁定**（Plan 独立复现 + 代码链调查，2026-09-06）：`recovery.rs::redo_record` 的 Insert/Update 为**追加式逻辑重放**（`row_id: _` 被忽略，经 `data_page.rs::write_tuple_to_data_page` 从内存 tail 顺序追加），注释声称幂等但实际在任何驱逐状态下非幂等。失效链：① 重开时内存 tail 从 catalog 上次刷盘值恢复（`open_or_init`；运行期 tail 经 `tm.write_tuple → update_table_tail` 写 catalog 行，catalog 页驱逐/刷盘滞后于数据页）→ 追加落在 stale tail → 覆盖其 next 指针 → 原链尾部孤儿化；② 已驱逐页上的行被重复追加；③ 重放行全部追加可见 → 计数 = 头部可达遗留 + N。同族缺口（同一 redo 路径）：④ Insert 重放不重建 PK 索引（运行期 `insert.rs:152-155` 维护）→ 索引/数据分叉（与规模无关）；⑤ Delete 重放只清索引，不重放数据页墓碑（运行期 `delete.rs:61-83` 两步）→ 墓碑页丢失时已删行复活；⑥ Update 重放追加新版本且 header 丢 `next_version → old_row_id`（运行期 `update.rs:98`）→ 版本链断裂 + 重复新版本。**Plan 复现**（10k 行、50 行/显式事务、~315 页 > 容量 100、无 kill 干净重开）：count=13190 ≠ 10000，checkpoint 后重开仍 13190（持久化），数据文件 +91 页；与 Act 160k 档 1 列 +3190 同机制同值。

**修复语义（位置寻址重放）**：redo 按记录自带的原始位置写入——
- **Insert**：目标 `row_id`（page P, logical L）。P 未初始化（页类型字节 0）则 `SlottedPage::init(0x03)`；slot L 已存在 → 跳过（幂等判据）；否则 `add_slot`——slot 由 `header.next_logical_id` 单调稠密分配（`slotted_page.rs:219-222`，只追加不回收），落位必为 L，否则显式报错（K05）。
- **Update**：新版本目标写入记录的 `row_id`（**已是新版本位置**，`update.rs:117` `row_id: new_row_id`——零格式变更），header 重建 `VersionHeader::new(tx_id, None).with_next_version(old_row_id)`；`old_row_id` 由 `deserialize_tuple(old_tuple)[pk_index].to_key()` → `index_manager.search(key)` 推导（重放按 LSN 序执行，索引状态 = 运行期该记录时刻状态，与 `update.rs:70` 同源）；索引 `update(key, row_id)` 幂等。推导失败（key 不在索引）显式报错。
- **Delete**：`update_version_header_in_data_page(bp, row_id, mark_deleted)` 墓碑重放（幂等，重标记同态；SlotNotFound 按运行期语义跳过）+ 既有 `find_key_by_row_id` 索引清理保持。
- **索引重建（Insert）**：`deserialize_tuple(tuple_data)[pk_index].to_key()` → `search(key)`：`Some(row_id)` 跳过 / `None` 插入 / `Some(其他)` 显式报错（K05）。
- **页链与 tail 重建**：按表追踪连续重放记录的目标页；页切换 Q→P 时置 next(Q)=P（幂等重写同值）并对 P 未初始化时 init；重放结束内存 tail = 最后目标页。**首条重放记录不置 next**（其前驱边若 < site 已由 checkpoint 全量刷盘持久；若 ≥ site 则由前序转换覆盖——site 语义不变）。重放不调用 `allocate_page`（目标页在原运行中已分配，文件页只增不减）。写入后镜像运行期 M21 可见性维护（`clear_all_visible` / `update_visibility_on_insert`）。
- **不变**：site/位点语义、事务分类、`mark_uncommitted_aborted`、writer/record/buffer/checkpoint、页格式、catalog 结构。

**成立条件**（已验证）：slot 稠密只追加；文件页分配只增不减；checkpoint 刷盘保证 site 前页间边持久；重放按 LSN 序（`read_all_with_lsn` 序）。

**拒绝备选**：① 强制 checkpoint 于脏页驱逐时（no-steal）——BufferPool→Checkpoint 层倒置 + 驱逐路径锁内 checkpoint 死锁风险 + 每 ~100 页全量刷盘开销；② 页级 page_lsn（ARIES-lite）——页格式变更（头布局扩展）；③ 只修 stale tail（恢复 tail 精确值）——不解决已驱逐页重复追加，非幂等本质不变；④ Update 格式扩展加 old_row_id 字段——不必要（`row_id` 已是新版本位置，old_row_id 可经索引确定性推导）。

**DDL 持久化模型不变**：CREATE TABLE 无 WAL 记录，小库（无驱逐、无 checkpoint）崩溃重开 table not found——既有引擎持久化模型（witness 夹具以 `checkpoint()` 处理），不并入本修复。

### D8（Review 裁定 2 新增，2026-09-07）：B-Tree 多页规模缺口——既有引擎缺陷进入恢复验收路径

Plan Review 2（Cycle 001-replan 审计）独立探针发现三个**既有** B-Tree 缺口（614 基线无 10k 规模 PK 操作测试，故未暴露；与 T0b 修复正交，但位于其验收路径上）：

- **G1 最小键搜索盲区**：10k 键树 `search` 对最小键未命中（运行树与 redo 重建树一致）→ 运行期重复 INSERT 最小键被接受（唯一性失效）。
- **G2 delete 重平衡 Page-full 泄漏**：批量 DELETE 偶发 `Page full`（运行期与恢复路径同源）；无中位点 checkpoint 的混合 WAL 重开 `delete redo ... Page full` → `Database::open` 整体失败。
- **G3 update 内部节点未实现**（`btree.rs:1041-1047` 显式）：多页树 UPDATE 运行期全败；恢复侧大树重放点 Update 记录 `RedoFailed`。

**裁定**：三缺口均为既有 Acceptance（R1-S2 普遍性、R2-S1 任意形态）的必要条件 → 留在本 Iteration，以 Rework Cycle 002 的 repair item（R-T0b-R2/R3/R4）在 `btree.rs`/`node.rs` 本体修复（恢复层绕行被拒：跳过/容错会造成索引/数据分叉，违反 K05）。**修复面扩大（`src/storage/btree/` + 新 `tests/btree_scale_test.rs`）需用户 Gate 2 批准**。实现要点与 RED 见证见 002-rework Plan Context；G4 口径修正（恢复精确性断言的期望值 = 已提交终态，须计运行期失败语句）随之落入见证 ② 重设计。

### D9（Review 裁定 3 新增，2026-09-07）：catalog root 同步与扫描版本去重——R2-S1 的最后两个既有缺陷

Rework Cycle 002 审计（Plan 独立探针）在 R-T0b-R4 使 UPDATE 真实提交后暴露两个既有引擎缺陷，均直接阻塞 R2-S1（混合负载恢复精确）：

- **G4 stale catalog root**：`BTree::insert` 根分裂返回新根（`btree.rs:205-215`）仅更新 IndexManager 内存 root，catalog 行 `index_root_page_id` 停留 create 时值 → 恢复从 stale root 加载，site 前索引条目不可达（中位点 checkpoint 形态重开失败 `old key not in index`，Plan 独立复现）。**修复**：IndexManager 持 catalog 上下文，任何 root 变化写 `catalog.update_table_root`（镜像 `update_table_tail`）；可恢复性不变量 = 落盘 root ≥ site 时点版本（checkpoint 全量刷盘保证），二次崩溃幂等。
- **G5 DataScan 版本双计**：扫描对被 `next_version` 指向的旧 slot 无替代判定 → 被更新行新旧版本都产出——**运行期即复现**（100 行 + 10 UPDATE → COUNT=110，无恢复介入）→ 恢复后同构（COUNT=10050）。索引点查不受影响（始终指向最新版本）。**修复**：扫描级替代集合（target_rid → 链头映射，按快照可见性条件跳过）；header 全局标记方案被拒（破坏旧快照可见性）。G5 属 executor 扫描语义（新故障域），修复面扩大需用户 Gate 2 批准。

**裁定**：两者均为 R2-S1 必要条件（缺一不可）→ 留在 Iteration 000 以 Rework Cycle 003 完成（R-T0b-R5/R6 + R-T0b-R1 精确化 + R-Gate）。Map 不变。设计细节与 RED 见证见 003-rework Plan Context。

### D10（Review 裁定 4 新增，2026-09-08）：恢复期索引去信任 + 重放后重建——撕裂树的修复方向

**根因终审**（Plan Review 3 独立复现，2026-09-08；临时探针已删除，recipe 见 003 Act Response）：R5 落地后 catalog root 已正确同步（裸读实测 `catalog_root=155` = LIVE root），但裸读磁盘树 `scan_all` 仅 184/10000 条可达、`collect_all_pages` 命中 `InvalidPageType expected 0x1, got 0x0` 洞页——**撕裂在树页本身**：中位点 checkpoint 后的运行期修改经页驱逐按 LRU 而非树拓扑刷盘，磁盘树同时含指向未刷盘子页的父页（洞）与已刷盘但不可达的孤儿页。恢复重放对磁盘树的任何消费（update redo 的 `old key` 搜索、insert redo 判重、delete redo 反查）都不健全——本轮探针中 update redo 先失败（`old key not in index`），insert redo 的判重搜索与插入同样发生在撕裂基座上。**结论：任何 catalog root 策略（R5 及其变体）都无法闭合 G4；修复必须使恢复路径在 `redo_count > 0` 时完全不消费磁盘索引树。**

**方案裁定（方向 a 修正版：a′）**：

- **触发条件**：`RecoveryResult.redo_count > 0`（有已提交数据记录被重放）。`redo_count == 0`（checkpoint-clean 关闭后打开）→ 磁盘树可信（checkpoint 全量刷盘保证一致），全部现有路径零变化。不洁关闭必有 site 后记录 → 必触发重建；clean 关闭必不触发——判定完备。
- **redo 去索引化（R-T0b-R7）**：redo 三臂不再读写 B-Tree。Update 的 `old_row_id` 派生改从**磁盘版本多映射**：恢复开始时扫描各表数据页构建 `HashMap<pk_bytes, Vec<RowId>>`（rid 升序；同键版本链 rid 序 == LSN 序——同行并发写者被行锁串行化、提交序即执行序），派生规则 = `max{rid ∈ 链 : rid < record.row_id}`（重放中 Insert/Update 向映射追加 rid，保持后续派生正确；Delete 不移除——墓碑版本 rid 参与过滤无害），并对派生槽显式校验 tuple == old_tuple（不符 → `RedoFailed`，K05）。Insert 判重/索引插入、Delete 索引清理整体移除——幂等性由位置寻址 slot 判定承担，重复 PK 损坏检测职责移至重建判重。`from_root` 实例在 redo 期零页访问（惰性构造），重建后旧实例直接丢弃。
- **重放后重建（R-T0b-R8）**：redo + mark_uncommitted_aborted 完成后，每表新建 `IndexManager::new`（spawn_blocking，先例 `create_table`）→ 扫描最终数据页链 → 按 PK 分组、自链尾沿 new→old 链回溯取第一个「已提交 ∧ 非墓碑」版本（已提交判定对照 header 编码与 WAL committed 集合，Act 实现时核实 `mark_deleted`/`commit_tx_id` 编码）→ `insert(key, rid)`；**重复 key → 显式 `Err`（K05，承接原 redo 判重的损坏检测）**→ 新 IndexManager 换入 TableMeta → `catalog.update_table_root(name, new_root)` 直写（redo 期无 catalog context，无递归；`attach_index_catalog_contexts` 照旧在 open 后段附加到换入实例）→ 旧树页释放：**洞容忍 DFS 收集**（父指针可枚举洞 id）+ `buffer_pool.free_page`（先例 `drop_table`）；其他收集错误 warn + 放弃释放（泄漏记录，先例 `drop_table:353-364`）。
- **幂等性**：二次恢复 redo_count 仍 > 0 → 再次重建（重建树自身的部分刷盘被无条件弃用，003 偏差 2 的递归入环类风险被结构性消除）；数据页位置寻址幂等（T0b 已证）。
- **拒绝备选**：(b) B-Tree 页洞容忍（search 遇洞跳过）——静默丢条目，与 K05 显式失败哲学冲突，且孤儿页条目同样丢失，拒绝；(c) 结构感知刷盘 / checkpoint 树快照 / no-steal 驱逐——BufferPool 驱逐策略重设计，超出既有 Acceptance 必要面，列为 improvement 候选（撕裂树的运行期根修），不并入本 change；redo 期消费磁盘树 + 事后修补——任何部分消费都在撕裂基座上进行，拒绝。
- **代价**：恢复期一次性 O(N) 页扫描 + O(N·logN) 索引构建 + O(版本数) 内存映射（10k 行级可忽略）；仅不洁打开承担，clean 打开零开销。

## 责任边界（D7-D10 修订后）

- **修改**：`src/wal/reader.rs`（逐帧无歧义解析，D0，已完成）、`src/wal/recovery.rs`（位置寻址重放 D7/T0b + redo 去索引化与重放后重建编排 D10/R-T0b-R7/R8）、`src/storage/btree/index_manager.rs`（root 同步 R5 已落地 + 洞容忍页收集 D10）、`src/storage/data/table_manager.rs`（R5 attach 已落地 + D10 重建换入 API）、`src/storage/catalog.rs`（`update_table_root` R5 已落地）、`src/executor/data_scan.rs`（扫描版本去重 D9/R6 已落地）、`src/storage/file_storage.rs::open`（try_lock）、`src/storage/error.rs`（+变体）、`src/cli/mod.rs::execute_command`（D3/D4）+ `ExitStatus`（+Signaled）、`Cargo.toml`（+signal feature）、既有测试 4 文件夹具锁适配（D6 列表）+ `tests/{btree_scale,wal_recovery_large}_test.rs`（R5/R6/R7/R8 见证）。
- **保持**：`Database::open` 成功路径与签名、`AsyncStorage` trait、WAL 记录格式与 writer/checkpoint/buffer 全部语义、页格式、site/位点与事务分类语义、`run_sql`/渲染/resolve、退出码 0/1/2/3/5 映射、`close()` 正常路径语义、混合格式流兼容、DDL 无 WAL 记录的既有持久化模型、BufferPool 驱逐策略与 LRU 行为（D10 容忍撕裂，不改变驱逐）、`redo_count == 0` 打开路径（零变化）。
- **禁止修改**：network server 路径、pipeline 及 D9 R6（data_scan）以外的 executor、页格式、`WALBuffer::do_flush` 并发语义（improvement 候选，另行处理）、既有测试**断言语义**（D6 所列 4 文件的夹具锁适配为唯一既定例外）。

## 行为变化汇总

| 输入 | 现行为 | 目标行为 |
|---|---|---|
| WAL ≥19 条记录后重开恢复 | `Incomplete WAL record`，库打不开 | 恢复成功，数据完整（D0，已完成） |
| 驱逐规模（>100 页）+ 未 checkpoint WAL 崩溃重开 | 行数虚增、重复行、原链尾部丢失，损坏经 checkpoint 持久化 | 恢复精确（行数 = 已提交数、索引一致、语义正确，D7） |
| 恢复重跑（crash-during-recovery） | 叠加重复 | 收敛到同一状态（D7 幂等） |
| 打开被占用库 | 静默双写（损坏风险） | `DatabaseLocked` → stderr `database is locked: <path>` → exit 4 |
| 同进程双开同库 | 静默双实例（损坏风险） | 第二次 `Database::open` → `DatabaseLocked` |
| flock 不支持的 FS | （不存在锁） | open 失败 exit 1（严格，不降级） |
| 执行中 SIGINT/SIGTERM | 默认终止，无 checkpoint | close() checkpoint → exit 130/143 |
| 打开中 SIGINT/SIGTERM | 默认终止 | 立即 exit 130/143（无 close，恢复可重放） |
| 无信号全部路径 | — | 零变化（614 基线守护） |
| 既有测试探针在第一持有者存活期间二次打开 | 正常通过 | 夹具按独占锁适配（4 文件，断言语义不变，D6） |
