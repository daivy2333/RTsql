# Iteration 000 / Cycle 000-initial: WAL 恢复逐帧无歧义解析

## Plan Context

- Status: draft
- Iteration: 000-wal-recovery-fix（WAL 恢复逐帧无歧义）
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

> **Replan 注记**：本 Cycle 是 Gate 2 前审计（2026-09-06）的产物。原计划（锁+停机单 Iteration）审计发现两个阻塞项——①引擎 P0：WalReader 格式嗅探使 ≥19 条记录的 WAL 恢复必失败（本 Iteration 修复，是原计划 kill-recovery e2e 验收的硬前提）；②执行阶段确定性 e2e 不可构造（重构为 Iteration 001 的库级结构测试方案）。原 Iteration 000-lock-shutdown 的 draft Cycle 未交接即被本 replan 取代，其范围移入 Iteration 001（见 change tasks.md Map）。

**Iteration Scope**

- Change tasks: T0, T1
- Depends on: None
- Stable baseline: 含 ≥19 条记录（>1KB）的 WAL 崩溃恢复成功、数据完整；混合格式流兼容；损坏帧仍显式报错（K05）；614 基线零回归
- Verification boundary: `cargo test --all` 除 T4-RED 白名单（T1 枚举的 3 个 Iteration-001 信号用例）外全绿（新增 `tests/wal_recovery_large_test.rs`；既有测试断言语义零修改，4 文件夹具锁适配见 design D6）+ clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/wal/reader.rs`、`tests/wal_recovery_large_test.rs`
- Deferred tasks: T2, T3, T4, T5（Iteration 001-lock-shutdown，见 change tasks.md）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部 What Changes 项（含 Replan 增补的 reader 修复）；用户决策 1（信号退出码 130/143）与决策 2（flock 严格失败）属 Iteration 001；工作区存量并入决策（2026-09-06 Plan 修订：T2/T3 存量收编、4 文件夹具标定、T1 白名单——见 design「工作区存量」节，实现面在 Iteration 001）
- Excluded scope: `WALBuffer::do_flush` 并发互斥（improvement 候选）；Checkpoint 记录改新格式；magic 文件头（T03）；锁/信号实现（Iteration 001）

**Objective**

任何大小完好（帧完整、CRC 有效）的 WAL——无论新旧格式混排——`Database::open` 恢复成功且数据完整；真实损坏仍显式报错。以大 WAL 恢复测试锁定（当前 RED：恢复必失败）。

**Background**

审计实验（2026-09-06，revision `1a9c91f`）发现：2000 行库（90 行/显式事务批量插入）`wal_buffer.shutdown()` + drop 后重开，`Database::open` 必报 `WalError("Incomplete WAL record")`。严格帧遍历证明 WAL 文件本身完好——2046 条记录内嵌 lsn==实际偏移、CRC 全部有效；而 `WalReader` 的格式嗅探对其中 79 条误判。根因：新格式帧 `[lsn:8][type:1][len:4][body][crc:4]` 的首字节是内嵌 LSN 的 LSB，`reader.rs:63-65` 以「byte[8] 是合法 type 且 byte[0] 不是」判别格式，当文件偏移低字节 ∈ {0x01..0x09}（合法 WalRecordType 值）时误判为旧格式，从 LSN 高位字节读出垃圾长度，游标 derail 直到 EOF 报 IncompleteRecord。首个碰撞点 ≈ 第 19 条记录（offset 1033）。既有测试全部活在 <1KB WAL 盲区，故 614 基线全绿。生产 WAL 是混合格式流：`do_flush → write_batch`（新格式）+ `checkpoint.rs:118 → write_record`（旧格式 Checkpoint 记录）——逐帧歧义是真实负载。本修复是 Iteration 001「kill 后 WAL 恢复 e2e」与大 WAL fixture 的硬前提。

**工作区并入（2026-09-06 Plan 修订）**：replan 前被中断的 Act 尝试（原 Iteration 000-lock-shutdown draft Cycle，未交接即被取代）遗留大量未提交改动：T2/T3 生产代码与测试（完成态）、T4 全部 e2e 脚手架与标定常量（生产接线缺位）、Cargo signal/libc 接线、4 个既有测试文件的独占锁夹具适配。本轮 Plan 盘点实测后并入计划：存量实现由 Iteration 001 契约收编（RED 以「临时摘除 hunk」复现）；既有测试标定列为 T2 必要后果（断言语义零修改）；**本 Iteration 范围不受影响**（`src/wal/` 零改动、`tests/wal_recovery_large_test.rs` 不存在，T0 仍为全新 RED→GREEN）。测试注释中的旧任务编号（"T02"/"T3"）与失效的「见 Act Response」引用来自被取代的旧 Cycle，属非实质遗留。

**Current Baseline**

- revision `1a9c91f`（master HEAD）；`cargo test --all`：614 passed / 0 failed（2026-09-06 Plan 实测）
- 工作区（未提交）实测（2026-09-06 Plan，受影响 6 目标）：`database_file_lock_test` 4/4、`btree_test` 10/10、`storage_test` 21/21、`schema_persistence_test` 8/8、`drop_table_free_test` 6/6 全绿；`cli_test` 13 绿 / 4 RED / 2 ignored——RED 为 3 个 T4 信号用例 + `test_sigkill_leaves_recoverable_db`，失败模式均为 T0 后果（子进程在信号落地前因 `Incomplete WAL record` 以 exit 1 死亡；锁锚点用例因恢复过短 10s 轮询超时）。T0 落地后 sigkill 用例应转绿，3 个信号用例转为真正的 T4 RED（`code==None`）
- 审计实验数据：2000 行批量插入 14k rows/s（90 行/事务）；auto-commit 单行 742 rows/s；close() checkpoint 正常（WAL 截断至 21B，重开 228µs）

**Current-State Evidence**

- 帧格式（`src/wal/record.rs:173-188`）：`serialize_with_lsn(lsn)` = `[lsn:8 LE][type:1][len:4 LE][body][crc:4 LE]`，CRC 覆盖 `[lsn+type+len+body]`。`WalRecordType` 合法值 0x01-0x09（`record.rs:20-30`）。
- 旧格式（`record.rs:123-139` `serialize()`）：`[type:1][len:4 LE][body]`，无 LSN/CRC。生产调用者：`checkpoint.rs:118`（Checkpoint 记录）经 `writer.rs:45 write_record`。
- Writer（`src/wal/writer.rs:195-218` `write_batch`）：`Arc<Mutex<File>>` 串行 + O_APPEND 追加，帧级原子；`get_current_lsn()` = 文件长度（`:174-189`）。
- Reader 缺陷点（`src/wal/reader.rs:39-105`）：`read_next_with_lsn` 先 peek 13B（`:46-50`）；判别 `is_new_format = bytes_read >= 9 && type_ok(peek[8]) && type_err(peek[0])`（`:63-65`）——**缺陷**；新格式分支按 `peek[9..13]` 的 len 读全帧后 `deserialize_with_lsn`（验 type+CRC，`:84`）；旧格式分支按 `peek[1..5]` 的 len（`:88-89`）。`IncompleteRecord` 错误产生点：`:57`（bytes_read<5）、`:70`（新格式分支 bytes_read<13）。
- 消费链：`RecoveryManager::full_recover`（`src/wal/recovery.rs:27/65/74/237/251`）经 `WalReader` 全量/位点过滤重放；`Database::open` 失败即库不可开。
- `WALBuffer::shutdown()`（`src/wal/buffer.rs:196-215`）：置标志 → do_flush → await handle；测试夹具用它确保全量落 WAL（先例 `checkpoint_redo_reduction_test.rs:291`）。
- 实验证据（/tmp/rtsql-plan-exp，release 构建）：① 2000 行库 WAL=113334B，严格遍历 2046 帧全对，嗅探误判 79 帧（首例 1033，lsb=9）；② `Database::open` 三次实验（99/90 行批、auto-commit 20k）均 `Incomplete WAL record`；③ checkpointed 库 reopen 228µs / scan 2000 行 986µs / sort 565µs；④ 多行 INSERT 1000 行 55ms、10k 行 `Abort failed: Page full`；⑤ 标量子查询 `Unsupported expression type`。
- 工作区存量盘点（git numstat，未提交）：`src/storage/file_storage.rs` +11（try_lock 分支）、`src/storage/error.rs` +3（`DatabaseLocked`）、`src/cli/mod.rs` +5/-1（Locked 映射；**无信号接线**）、`Cargo.toml` +2/-1（tokio `signal` feature + dev-dep `libc`）、`tests/cli_test.rs` +264/-3（既有 12 用例零修改；新增 lock 用例 + 信号段 4 用例 + 2 个 `#[ignore]` 诊断）、`tests/database_file_lock_test.rs` 新增 4 用例、4 个既有测试夹具标定（`btree` +7 / `storage` +3 / `schema_persistence` +10/-7 / `drop_table_free` +6/-5）
- 夹具标定明细（断言语义零修改）：`btree_test` 重开前 drop 持有者；`storage_test` durability 用例先 drop；`schema_persistence_test` 页数探针作用域化；`drop_table_free_test::page_count` 第二句柄探针改 `std::fs::metadata` 长度高水位（断言调用点零修改）

**Relevant Code**

| 文件/符号 | 职责与本 Iteration 关系 |
|---|---|
| `src/wal/reader.rs::read_next_with_lsn` | 逐帧解析入口；**唯一修改点**（判别 + 回退） |
| `src/wal/record.rs::deserialize_with_lsn / deserialize` | 两种格式的结构+CRC 验证；语义不动 |
| `src/wal/recovery.rs::full_recover` | 消费者；不动（自动受益） |
| `src/wal/{writer,checkpoint,buffer}.rs` | 写侧；**禁止修改** |
| `tests/wal_recovery_large_test.rs`（新） | RED→GREEN 主场景 |
| 既有 `wal_record_test` / `wal_handle_test` / `checkpoint_redo_reduction_test` / `recovery_e2e_test` | 旧格式兼容、损坏帧显式报错、位点语义的回归守护（零修改） |
| `tests/database_file_lock_test.rs`（工作区已就位，4 用例实测绿） | T2 见证；Iteration 001 范围，本 Iteration 仅作回归 |
| `tests/cli_test.rs` 新增段（lock + 信号 + 诊断） | T3/T4 见证；当前 4 RED 均为 T0 后果；Iteration 001 范围，本 Iteration 仅作回归（T1 白名单口径） |

**Critical Path**

`Database::open` → `RecoveryManager::full_recover` → `WalReader::read_all_with_lsn` → `read_next_with_lsn`（逐帧：peek 13B → 判别格式 → 读全帧 → deserialize 验证）→ redo 重放。修复点仅在判别与回退；位点过滤（`≥ site lsn`）与重放语义不动。

**Implementation Guidance**

建议实现：保留 peek-13B 结构；判别逻辑改为——(a) byte[0] 非合法 type（0x01-0x09）→ 必为新格式，走现有新分支；(b) byte[0] 为合法 type → **歧义**：先按新格式读全帧（len 取 peek[9..13]）尝试 `deserialize_with_lsn`（type+CRC 验证），成功即接受；失败（含 read_exact EOF 与 CRC 不符）**seek 回帧首**，按旧格式（len 取 peek[1..5]）读帧尝试 `deserialize`；两路皆败 → 显式 `WalError`。注意：新格式尝试中的 EOF/CRC 失败必须转化为「回退信号」而非直接向调用者传播错误。场景 (a) 的现有行为不变（绝大多数帧）。

关键取舍（已定）：CRC 为新格式接受判据（最强验证器）；旧格式无 CRC，以结构合法性（type+len 可读且 deserialize 成功）为判据；不引入逐帧以外的格式状态机。非实质留给 Act：回退时 seek 的具体实现（stream_position 记录帧首）、错误映射细节。

**Behavioral Change**

| 输入 | 现行为 | 目标行为 |
|---|---|---|
| WAL ≥19 条完好记录后重开 | `Incomplete WAL record`，库打不开 | 恢复成功，数据完整 |
| 歧义偏移上的新格式帧 | 误判旧格式 → derail | CRC 验证接受 |
| 歧义偏移上的旧格式帧（Checkpoint 记录恰在歧义偏移） | 碰巧正确（旧分支） | 先试新格式失败 → 旧格式接受（行为不变，路径更明确） |
| 真实损坏帧 | 显式报错 | 不变（K05） |
| 小 WAL（<19 条） | 成功 | 不变 |

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T0 | wal-recovery-frame-parsing S1 大 WAL | `src/wal/reader.rs::read_next_with_lsn` | 嗅探判别 → 单分支解析 | 歧义偏移双格式尝试：新格式 CRC 优先，旧格式回退 |
| T0 | S2 混合流 / S3 损坏帧 | 同上 | 同上 | 回退语义保持两格式均可达；K05 不变 |
| T1 | 回归门 | 无新代码 | — | 四命令验证 |

**Task Contracts**

### T0: WAL 恢复逐帧无歧义解析

- Requirement/Scenario: `wal-recovery-frame-parsing` 全部 3 场景
- Depends on: None
- Targets: `src/wal/reader.rs::WalReader::read_next_with_lsn`
- Current behavior: 嗅探判别使偏移低字节 ∈ {0x01..0x09} 的新格式帧被误判为旧格式，恢复 derail → `Incomplete WAL record` → `Database::open` 失败
- Required behavior: design D0/D（本 Cycle Implementation Guidance）——歧义偏移先新格式（CRC 判据）后旧格式（结构判据）回退；两路皆败显式报错；非歧义偏移行为不变
- Required changes: `read_next_with_lsn` 判别与回退重排（约 +30 行）；不改 record/writer/checkpoint/buffer/recovery
- Preserve: 新格式 CRC 验证；旧格式流兼容；损坏帧显式报错（K05）；`seek_to(lsn)` 位点语义；`read_next`/`read_all` 公共签名
- Forbidden: writer/checkpoint/BufferPool/recovery 修改；格式版本头；do_flush 并发修复
- Test witness（RED 先行）: 新建 `tests/wal_recovery_large_test.rs`（lib 集成，tokio::test）——用例 ①：TempDir 建库 → `CREATE TABLE t (id INT PRIMARY KEY, v INT)` → 单条多行 `INSERT INTO t VALUES (0),(1),...,(499)`（实验实证 1000 行 55ms 可行，500 行足够 >19 条 WAL 记录；若单语句记录数不足 19 则改用 90 行/事务批量至 WAL >2KB）→ `db.wal_buffer.shutdown().await` → drop 不 close → `Database::open` 重开 → 断言 Ok 且 `SELECT COUNT(*)` == 500。RED：当前 `unwrap()` panic（Incomplete WAL record）。用例 ②：5 行小 WAL 重开成功（守护既有路径，改造前后均绿）
- GREEN condition: ①② 绿 + `cargo test --all` 614 基线零回归
- Verification: `cargo test --test wal_recovery_large_test` 输出+退出码记 Act Response
- Stop when: 双格式尝试出现无法以 CRC/结构消解的双匹配；或大 WAL 恢复暴露位点语义错误（recovery.rs 层）→ 返回 Plan

### T1: Iteration 000 验证门

- Requirement/Scenario: 全部（回归门）
- Depends on: T0
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- Current behavior: 614 pass / clippy 0 / fmt 0 / validate PASS（`1a9c91f` 实测）；工作区实测（2026-09-06 Plan）：受影响 6 目标中仅 `cli_test` 4 RED（3 信号 + sigkill，均为 T0 后果），其余全绿
- Required behavior: **T4-RED 白名单**（`test_sigint_during_run_graceful_130`、`test_sigint_during_open_130`、`test_sigterm_during_run_143`——Iteration 001 T4 的 RED 见证，无 T4 生产代码必失败）之外全部通过，0 unexpected failed；`test_sigkill_leaves_recoverable_db` 在 T0 后**必须转绿**（其失败归因 T0，不属白名单）；测试总数 = 614 + 工作区已就位新增 + wal_recovery_large_test 用例数
- Required changes: 无
- Preserve: 既有测试断言语义零修改（4 文件夹具锁适配为既定例外，design D6）
- Forbidden: 为通过而弱化断言；扩大白名单
- Test witness: 各命令决定性输出（≤20 行）与退出码
- GREEN condition: 四项达标（`cargo test --all` 按白名单口径 + clippy/fmt/validate 全 0/PASS）
- Verification: 输出记 Act Response
- Stop when: 白名单之外的回归失败且无法归因于 T0 → BASELINE-CHANGED 返回 Plan

**Invariants**

- `WalRecord` 序列化格式、writer/checkpoint/buffer/recovery 语义零变化
- 既有测试（614）断言语义零修改全绿；4 文件夹具锁适配（design D6 列表）为 T2 既定必要后果，本 Iteration 一并回归守护
- 损坏帧显式报错语义（K05）不变
- 不引入哈希/校验和/内容指纹新增；不新建 Evidence 占位目录

**Non-goals**

`do_flush` 并发互斥、Checkpoint 记录改格式、magic 头（T03）、锁/信号（Iteration 001）、多行 INSERT Page full（观察项）。

**Acceptance**

1. `wal-recovery-frame-parsing` S1：大 WAL（≥19 条记录）恢复成功、数据完整——`wal_recovery_large_test` ①（RED→GREEN）。
2. S2 混合流兼容：既有 checkpoint/recovery 套件零修改全绿（Checkpoint 记录旧格式帧继续被解析）。
3. S3 损坏帧显式报错：K05 语义保持（既有测试守护）。
4. 回归门：`cargo test --all` 除 T4-RED 白名单（3 个信号用例）外全绿、clippy 0、fmt 0、openspec validate PASS——T1。

**Verification**

- `cargo test --test wal_recovery_large_test`（新，先 RED 后 GREEN）
- `cargo test --all`（614+新增，0 unexpected failed——白名单见 T1）
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- 全部输出（每项 ≤20 行决定性片段）+ 退出码记 Act Response

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 根因定位到 `reader.rs:63-65` 嗅探逻辑；帧格式/合法值/消费链/写侧格式全部来自实际代码；三组本机实验（帧遍历/重开失败/吞吐标定）提供决定性证据；工作区存量盘点（git numstat + 6 测试目标实测，2026-09-06）确认本 Iteration 范围（`src/wal/`）未被存量触及 |
| Design | PASS | D0 修复语义闭合（CRC 判据 + 回退 + K05 保持）；混合流约束（checkpoint.rs:118 旧格式 Checkpoint 记录）已纳入；工作区并入修订（D5 并入版 / D6 / 责任边界 4 文件标定例外）无 TBD |
| Iteration Plan | PASS | 单 Iteration 两任务（T0 修复 + T1 门）；平衡审计：独立引擎正确性成果 + 001 硬前提，对照 MS07-T03 先例不过碎；工作区并入不改变 Map 与任务归属 |
| Cycle Scope | PASS | initial；gap None；Excluded 明确（do_flush 并发、T03、001 范围含存量收编） |
| Task Contracts | PASS | T0/T1 含 Targets/行为/Preserve/Forbidden/RED-GREEN/Verification/Stop；只读本 Cycle 可执行；T1 白名单口径与白名单外失败归因规则明确 |
| Traceability | PASS | RTM：R(S1)→T0→reader.rs→wal_recovery_large_test①；S2/S3→T0+既有套件；回归门→T1。无 Missing |
| Verification | PASS | 验证直接证明目标行为（大 WAL 恢复 + 数据完整 + 回归按白名单口径零意外失败）；Persisted Evidence = none |

**Persisted Evidence**

- Mode: none

Act Response（命令、输出、退出码）足以承载全部 Acceptance；实验可低成本重跑。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

1. 歧义偏移上新格式尝试的 read_exact EOF 必须转化为回退信号而非传播——语义已写入契约，实现细节（错误分类匹配）留给 Act。
2. CRC 双匹配概率 2^-32，理论存在不处理。
3. `do_flush` 三入口无互斥（并发时内嵌 LSN 可能重复/错位，影响 checkpoint 截断语义）——本会话实验未触发帧损坏（writer mutex + O_APPEND 保帧完整），登记为 improvement 候选，不在本 change 修复。Iteration 001 的夹具（工作区存量 `build_big_wal`）用 50 行/事务分块（缓冲记录数恒低于 WALBuffer capacity=100 阈值，不触发 appender-threshold 并发 flush 窗口，loop 为唯一写者）规避。
4. 多行 INSERT 万行级 `Page full`（含 abort 失败）——观察项，improvement 候选，不阻塞本 Iteration（500 行远低于阈值）。
5. 单条多行 INSERT 产生的 WAL 记录数未精确清点（实验确认 ≥19 的充分性由"1000 行 55ms 成功 + WAL>2KB"间接保证）；Act 在 RED 阶段以"WAL 文件 >2KB 或重开失败"为 RED 判据，不依赖精确记录数。
6. 工作区存量（T2/T3/T4-脚手架）未经验证、无 Act Response 记录——Iteration 001 Act 必须按契约复核（含 RED 复现）后收编，不得默认正确；复核发现与契约 Preserve/Forbidden 冲突且无法局部消解时按各自 Stop when 返回 Plan。本 Iteration 的 T1 门以实测口径（白名单）守护存量不引入意外回归。
7. `test_sigint_during_open_130` 的锁锚点依赖「恢复耗时显著大于轮询间隔」——T0 修复后恢复为数百 ms 级，窗口稳定；若 T0 落地后该锚点仍超时，恢复耗时标定（D5）需重开，返回 Plan。
8. `database-file-lock` R1-S4（锁系统调用非 WouldBlock 失败严格报错）无法在本机构造（本地 FS 均支持 flock）——见证为 diff 审查（`Err(e) => Err(StorageError::Io(e))` 直通映射），Iteration 001 Act 在 Act Response 记录审查结论，不建专门测试；此为已接受的见证边界，非 Simplified。

## Act Response

- Status: pending

**Implemented**

（Act 填写）

**Changed Files and Symbols**

（Act 填写）

**Deviations from Plan**

（Act 填写；没有则 None）

**Blocker Handoff**

（正常完成写 None）

**Blocker Resolution**

（未恢复时写 None）

**Self-Review**

- Plan compliance: 
- Full diff reviewed: 
- Critical findings unresolved: 
- Important findings unresolved: 
- Minor findings unresolved: 

（Act 填写）

**Verification Evidence**

（Act 填写：命令、≤20 行决定性输出、退出码、支持的 Acceptance）

**Persisted Evidence**

（`None required`）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|

（没有候选时写 None）

**Remaining Issues**

（Act 填写或 None）

**Commit or Diff Reference**

（可选）

## Plan Review

- Review Result: pending

**Findings**

（Plan 填写）

**Deviation Classification**

（Plan 填写）

**Acceptance Gaps**

（Plan 填写）

**Convergence**

N/A（首次 Review）

**Evidence**

（Plan 填写）

**Follow-up Decision**

（Plan 填写）

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None
