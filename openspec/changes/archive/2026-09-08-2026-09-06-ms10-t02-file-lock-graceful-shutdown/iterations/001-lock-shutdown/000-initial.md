# Iteration 001 / Cycle 000-initial: 并发互斥与可中断的 CLI

## Plan Context

- Status: ready
- Iteration: 001-lock-shutdown（并发互斥与可中断的 CLI）
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None（本 Iteration 首个 Cycle）

> **展开注记**：Iteration 000 于 Cycle 004-rework Review `accepted` 后完成（撕裂树修复落地，`mixed_dml_recovery_semantics` 转绿，白名单口径全绿）。本 Cycle 按 Map 展开既有 Iteration 001——**范围与契约零变化**，Gate 2 依据 = change 级计划批准（2026-09-06，含工作区并入修订，见 proposal「Plan 并入」节）+ 当前基线复核（下方 Current Baseline）。T2/T3/T4/T5 执行契约以 change `tasks.md` 对应 task 为权威依据（含 Targets/行为/Preserve/Forbidden/见证/Verification/Stop 完整字段），本文件不复制契约正文，只载明 Cycle 级基线、存量收编要求与当前状态证据。

**Cycle Scope**

- Tasks: T2（打开即独占锁）、T3（CLI 锁冲突退出码 4）、T4（优雅停机 130/143）、T5（全量回归与 Iteration 001 验证门）
- Depends on: Iteration 000（已完成：kill-recovery e2e 与大 WAL 恢复正确）
- Inherited scope: Iteration 000 全部成果（reader 修复、位置寻址重放、B-Tree 修复、R5/R6/R7/R8、`btree_scale_test` 5 + `wal_recovery_large_test` 7 见证）保持零回退
- Excluded scope: T03 格式头、T04 多语句分片、T05 生命周期子命令、锁等待、Windows/非 Linux、server 路径、REPL、lib API 签名

**Objective**

打开被占用库 → `DatabaseLocked` / CLI exit 4；SIGINT/SIGTERM 优雅停机（已打开则 `close()` checkpoint → exit 130/143）；同进程双开被拒；正常路径与 614 基线行为一致；白名单 3 个信号 RED 见证随 T4 转绿，全量门收口。

**Current Baseline**（Plan 独立复核，2026-09-08，Cycle 004 Review 时点）

- `cargo test --all --no-fail-fast`：cli_test **14 passed / 3 failed / 2 ignored**——3 失败恰为 T4-RED 白名单（`test_sigint_during_open_130` / `test_sigint_during_run_graceful_130` / `test_sigterm_during_run_143`，`code==None` 形态 = 生产信号接线缺位的真 RED）；`test_sigkill_leaves_recoverable_db` 绿；其余全部套件绿。
- `cargo test --test database_file_lock_test` 4 用例与 `tests/cli_test.rs::test_lock_conflict_exit_4` 绿（T2/T3 存量实现实测态，待 Act 按契约复核收编）。
- clippy 0 / fmt 0 / validate PASS。
- 工作区：`src/storage/file_storage.rs` / `src/storage/error.rs` / `src/cli/mod.rs` / `Cargo.toml` / `tests/database_file_lock_test.rs` / `tests/cli_test.rs` 自 `590fdc6` 后**零改动**——design「工作区存量」节描述与当前代码一致。

**Current-State Evidence**

- **T2 存量（完成态）**：`file_storage.rs::open` try_lock 分支（`WouldBlock` → `StorageError::DatabaseLocked(path)`，其他 IO 错误原样传播——design D1 逐字实现）；`error.rs::DatabaseLocked`（D2）。**Act 义务**：临时摘除 try_lock hunk 复现 ①③④ RED（② 天然绿）→ 恢复 → GREEN，输出记 Act Response（tasks.md T2 契约「RED 复现」节）。
- **T3 存量（完成态）**：`cli/mod.rs` open Err 分支 `DatabaseLocked → ExitStatus::Locked`（D3）。**Act 义务**：临时摘除 Locked 分支（5 行 hunk）复现 RED → 恢复 → GREEN（tasks.md T3 契约）。
- **T4 缺位（生产代码）**：`cli/mod.rs::execute_command` 无任何信号接线（D4 两阶段 select 未实现）；`ExitStatus::Signaled` 变体缺位；`Cargo.toml` tokio `signal` feature + dev-dep `libc` 已就位；3 个信号 e2e + kill e2e 脚手架已就位且呈预期 RED。**Act 义务**：D4 两阶段 select 重构 + `Signaled(i32)` 枚举与映射 + D5 之 ⑤ 库级结构测试（信号 future 注入点，**待建**）+ D5 之 ② 打开阶段用例增补 1 行 WAL 长度断言（>2KB）+ 标定（`calibration_recovery_time` 实测 T ≥ 500ms 且 200ms ∈ [T/4, T/2]，不足倍增 `RTSQL_CALIBRATION_ROWS`）。
- **D5/D6 测试布局**：既有 12 cli 用例零修改守护；夹具 `build_big_wal`（50 行/事务分块显式事务，规避 do_flush 并发窗口）/ `SIGNAL_DELAY_MS=200` / `run_cli` fixture 直接复用；phase-2 结构测试落 `src/cli/mod.rs` `#[cfg(test)]`。

**Cycle 级不变量**

- `Database::open` 成功路径与签名、`AsyncStorage` trait、页读写/分配/释放语义零变化；锁先于 `WalWriter::open` 与恢复（`database.rs:30` 落点不变）。
- 页对齐/权限/redo 失败仍 exit 1（`test_corrupt_file_open_fails_exit_1` 守护）；`run_sql`/渲染/resolve 不感知信号；无信号路径行为零变化。
- WAL/恢复/索引代码零修改（Iteration 000 冻结面）；不引入产品新依赖；既有测试断言语义零修改（D6 所列 4 文件夹具锁适配为既定例外）。
- Iteration 000 全部见证保持绿。

**Acceptance**（= tasks.md Iteration 001 Stable baseline）

1. 打开被占用库 → `DatabaseLocked` / CLI exit 4 + stderr `database is locked: <path>`——`database_file_lock_test` 4 用例 + `test_lock_conflict_exit_4`。
2. SIGINT/SIGTERM 优雅停机：执行阶段信号 → `close()` checkpoint → 130/143；打开阶段信号 → 无 close（WAL 仍大）→ 130/143——D5 ①②③ 全绿。
3. kill -9 恢复守护——`test_sigkill_leaves_recoverable_db` 保持绿。
4. 全量门（T5）：`cargo test --all` **无白名单**全绿（白名单 3 用例随 T4 转绿，0 failed / 0 unexpected）+ clippy 0 / fmt 0 / validate PASS。

**Verification**

- `cargo test --test database_file_lock_test`、`cargo test --test cli_test`、`cargo test --all`（终态无白名单口径）
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- 标定数据（WAL 行数 N、恢复耗时 T、信号延迟 D）记 Act Response
- 输出（每项 ≤20 行）+ 退出码记 Act Response

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | design「工作区存量」节 + 本 Cycle Current Baseline 独立复核（存量实现/缺位面/测试布局全部代码级在案） |
| Design | PASS | D1（锁实现/落点/生命周期/备选拒绝）、D3（退出码映射）、D4（两阶段 select + 取消安全论证）、D5（见证构造与标定）、D6（测试布局）全部闭合，无 TBD |
| Task Contracts | PASS | tasks.md T2/T3/T4/T5 契约完整（依赖/目标/行为/Preserve/Forbidden/RED-GREEN/Verification/Stop） |
| Iteration Plan | PASS | Map 不变；平衡审计在案（tasks.md Iteration 001 节） |
| Scope | PASS | change 级批准（2026-09-06，含工作区并入）；此后零范围变化 |
| Verification | PASS | 见证直接证明目标行为；白名单转绿路径明确；Persisted Evidence = none |

**Persisted Evidence**

- Mode: none

**Risks and Notes**

1. 信号 e2e 时序余量依赖恢复耗时标定（D5）：Act 以本机实测 T 调整行数；T 无法稳定 ≥500ms 时按 T4 契约 Stop-when 返回 Plan。
2. T2 的 RED 复现需临时摘除生产 hunk——复现后必须恢复并复跑 GREEN，hunk 状态在 Act Response 以 diff 摘录自证。
3. `close()` 期间二次 Ctrl-C 不强杀（tokio 信号驱动进程级持久）——已知行为（proposal 风险在案），kill -9 兜底。
4. Iteration 000 的撕裂树修复使 `mixed_dml` 形态夹具（若 T4 标定复用大 WAL 负载）恢复耗时稳定——标定以实测为准。

## Act Response

- Status: reported（2026-09-08，Cycle 000-initial）

### 实际改动

**T2 打开即独占锁（存量收编，生产代码零修改）**

- `src/storage/file_storage.rs::FileStorage::open` try_lock 分支（存量，与 D1 一致）复核收编。RED 复现：临时摘除 try_lock hunk（11 行）→ `database_file_lock_test` `1 passed; 3 failed`（①③④ 以 `expected StorageError::DatabaseLocked, got Ok` 失败，② 天然绿，如约）→ 恢复 hunk → `4 passed; 0 failed`（exit 0）；恢复后与 HEAD 零 diff。

**T3 CLI 锁冲突退出码 4（存量收编，生产代码零修改）**

- `src/cli/mod.rs` `DatabaseLocked → ExitStatus::Locked` 映射（存量，D3）复核收编。RED 复现：临时摘除 Locked 分支（5 行）→ `test_lock_conflict_exit_4` 失败（`left: Some(1), right: Some(4)`，stderr 为 General 文案，如约）→ 恢复 → GREEN（exit 0）。

**T4 优雅停机（本 Cycle 新实现）**

- `src/cli/mod.rs`：
  - `ExitStatus` +`Signaled(i32)`；`message()` 返回 `None`（130/143 自解释）；`ExitCode` 映射 `128 + signum`。
  - `execute_command` 重构为两阶段编排 `execute_command_inner(db_path, work, signal_int, signal_term)`：阶段 1 `Database::open` 与信号竞争（信号臂无 close 立即 `Signaled`）；阶段 2 `run_sql` 与信号竞争（信号臂 `Signaled` 后仍执行 `db.close()`）；close Err 语义扩一条——`Signaled` 保持信号退出码 + stderr 提示 close 失败，`Success`/其他维持现状（D3/D4 逐字）。
  - 信号源工厂 `sigint_future`（`ctrl_c`）/ `sigterm_future`（`SignalKind::terminate`）；安装失败/流关闭保持 pending 不伪造信号；每阶段经工厂生成新 future（design 明示留给 Act 的非实质选择）。
  - `open_error_status` helper 承载 D3 映射（DatabaseLocked → Locked(4)，其余 General(1)）。
  - D5-⑤ 库级结构测试 `phase2_signal_runs_close_and_returns_signaled`（`#[cfg(test)]`，`src/cli/mod.rs`）：Notify 握手（工作负载 future 在阶段 2 内放行许可）使信号确定落在执行阶段；断言 close() 已执行（WAL 截断 <1KB）+ 返回 `Signaled(2)`。
- `tests/cli_test.rs`：D5-② 打开阶段用例增补 WAL 长度断言（>2048 字节，无 close 的直接观测物）；标定注释重写 + `WAL_ROWS` 160_000 → 40_000（偏差 1）。

### 与计划的偏差

1. **标定落点偏移（D5 窗口不可稳健命中）**：D10 恢复重建落地后恢复耗时量级改变——本机实测 40k→8.86s、160k→40.7s（驱逐规模下重放后重建 B-Tree 在 100 页池上的随机访存主导；小库无驱逐档为毫秒级，[T/4, T/2] 窗口恰在耗时悬崖内、不可稳健命中）。`WAL_ROWS` 取 40k（T=8.86s，T ≥ 500ms 以 17 倍余量满足），D=200ms 低于 T/4 → e2e ①③ 的信号确定落在打开阶段；两条用例注释均明示容忍该落点（两条路径退出码与数据完整性一致），执行+close 阶段见证由 D5-⑤ 结构测试确定性承担。未选 160k 档：套件耗时 ~10 分钟且落点结论相同。
2. **结构测试前置补充 schema checkpoint**：首版夹具（create_table + 100 INSERT + drop 不 close）在 `execute_command_inner` 重开时恢复失败（`table 't' lookup failed during redo`）——DDL 无 WAL 记录的既有持久化模型（design D7 已载明 witness 夹具以 checkpoint 处理）。修正为 create_table → checkpoint（落盘 schema）→ 300 INSERT（WAL 重新增长 >1KB）→ `wal_buffer.shutdown()`（停后台 flush loop，恢复期无同进程并发写者）→ drop 不 close。夹具形态修正，契约断言（close 已执行 + Signaled(2) + WAL<1KB）不变。
3. **`cli_test.rs` 工作区存量面**：diff 相对 HEAD 含收编的非语义改动（import 排序、`matches!`/`is_multiple_of` 形式化）——design「工作区存量」节在案，随收编保留。
4. **结构测试过程发现（已修复）**：WAL 路径误推 `sig.db.wal`（实际 `with_extension("wal")` → `sig.wal`）；`args.format` 闭包捕获在 HRTB 下借用 `args` 致生命周期错误（改为按值拷出 `sql`/`format`）。

### Self-Review（Gate 4）

**Spec compliance**（对照 `database-file-lock` R1/R2 与 `cli-noninteractive-shell` R1 修改 + 优雅停机新增）：

- R1 四场景：独占打开成功（636 全量隐证）/ 第二持有者被拒（FileStorage 直开 + e2e exit 4）/ 同进程双开被拒 / 锁调用失败严格报错（`TryLockError::Error → Io` 路径在案）。
- R2 两场景：正常关闭可重开 + SIGKILL 强杀无死锁（e2e ④，重开成功 + 恢复执行）。
- cli R1 修改：锁冲突 exit 4 + stderr `database is locked` 前缀 + stdout 空 + 释放后可执行（`test_lock_conflict_exit_4`）。
- 优雅停机四场景：执行阶段（D5-⑤ close 已执行 + e2e ① 退出码/数据完整）/ 打开阶段（锁锚点确定性 + WAL>2KB 无 close）/ SIGTERM 143 / 无信号回归（既有 12 cli 用例零修改绿）。
- Preserve 逐项核对：`Database::open` 签名与成功路径、`AsyncStorage` trait、页语义、`run_sql`/渲染/resolve、退出码 0/1/2/3/5、close 正常路径语义、WAL/恢复代码——零修改。Forbidden 全部满足：无 SIGHUP、close 无 select、130/143 以外无信号退出码、产品依赖零变化（`Cargo.toml`/`file_storage.rs`/`error.rs`/`database_file_lock_test.rs` 相对 HEAD 零 diff）。

**Code quality**：

- 全量 diff 复审（`git diff src/cli/mod.rs tests/cli_test.rs`）：无计划外修改；信号 future 注入形状（FnOnce work + Fn 信号工厂 + HRTB）为 design 留给 Act 的非实质选择；错误/边界覆盖（信号源安装失败 pending、close Err 三分支、取消安全论证随 doc 注释在案）；无死代码/重复；测试不因错误原因通过（结构测试以 WAL 字节数观测 close，e2e 以退出码 + WAL 长度观测）。
- 已修复发现：结构测试夹具恢复失败（偏差 2）、WAL 路径误推、`args.format` 生命周期、fmt 缩进 1 处（`cargo fmt`）。
- 遗留 Minor：①ENOLCK 场景无自动化见证（本地无法构造不支持 flock 的 FS；代码路径在案，契约见证清单未要求）；②`execute_command` 对 `args.sql` 一次 String clone（一次性 CLI，可忽略）；③D10 恢复代价在 160k 档实测 40.7s（Iteration 000 既定 improvement 候选「撕裂树运行期根修/驱逐改造」的量化支撑数据，非本 Cycle 变更面）。

### 验证（Gate 5）

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T2 RED | （摘除 hunk）`cargo test --test database_file_lock_test` | `FAILED. 1 passed; 3 failed`（①③④ `expected DatabaseLocked, got Ok`） | RED 如约 |
| T2 GREEN | `cargo test --test database_file_lock_test` | `ok. 4 passed; 0 failed`，exit 0 | PASS |
| T3 RED | （摘除分支）`cargo test --test cli_test test_lock_conflict_exit_4` | `left: Some(1), right: Some(4)`，exit 101 | RED 如约 |
| T3 GREEN | 同上（恢复后） | `ok. 1 passed`，exit 0 | PASS |
| T4 结构测试 | `cargo test --lib cli::tests::phase2` | `ok. 1 passed`（Signaled(2) + WAL<1KB） | PASS |
| D5 标定 | `RTSQL_CALIBRATION_ROWS=40000|160000 cargo test --test cli_test calibration_recovery_time -- --ignored --nocapture` | `recovery T: 40k→8.86s、160k→40.7s` | T≥500ms 达标（偏差 1） |
| 全量门 | `cargo test --all` | `TOTAL passed=636 failed=0 ignored=2`，exit 0（fmt 修正后终态复跑） | PASS |
| 白名单转绿 | `cargo test --test cli_test` | `ok. 17 passed; 0 failed; 2 ignored`，exit 0 | PASS |
| 静态门 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | 双 exit 0（fmt 先修 1 处缩进） | PASS |
| OpenSpec | `openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown` | `is valid`，exit 0 | PASS |

### Persisted Evidence

None required（Plan Context Mode: none；全部验证可低成本复跑，无一次性现场）。

### Experience Candidates

None。（D10 恢复耗时标定数据——40k→8.86s / 160k→40.7s——已作为 improvement 候选「撕裂树运行期根修/驱逐改造」的量化支撑随 change 收尾处理，不构成 Runbook/Incident。）

### 未解决问题

- improvement 候选三件（tasks.md 裁定 5 在案）：撕裂树运行期根修/驱逐改造、`mark_tx_aborted` 空转补全、update→delete 旧值重现——随 change 收尾处理。
- 信号安装窗口（进程启动至信号 future 首次 poll 之间按默认终止）——design D4 既定不处理。

（无 commit——工作区含 Iteration 000 + 001 全部未提交改动，commit 由用户触发。）

## Plan Review

- Review Result: accepted

（Plan 独立审查，2026-09-08。方法：T4 生产代码与测试 diff 逐段审读 + delta spec 场景覆盖核对 + 独立复跑全部验证。Act Self-Review 仅作输入。）

### 独立验证结果（全部与 Act Response 一致）

| 验证项 | Plan 独立手段 | 结果 |
|---|---|---|
| 全量门 | 复跑 `cargo test --all --no-fail-fast` 聚合 | **TOTAL passed=636 failed=0 ignored=2**——白名单三用例全部转绿，零失败 ✓ |
| cli 明细 | 复跑 `cargo test --test cli_test` | 17 passed / 0 failed / 2 ignored（`test_lock_conflict_exit_4` 与 4 信号用例在通过之列）✓ |
| D5-⑤ 结构测试 | 复跑 `cargo test --lib cli::tests::phase2` | 1 passed（Signaled(2) + WAL<1KB = close 已执行）✓ |
| 静态门 | 复跑 clippy/fmt/validate | clippy exit 0（仅 `.cargo/config` 环境弃用提示，非代码 lint）/ fmt 0 diff / valid ✓ |
| Preserve 面零 diff | `git diff --stat HEAD` 核对 | 本轮生产改动仅 `src/cli/mod.rs`；`Cargo.toml`/`file_storage.rs`/`error.rs`/`database_file_lock_test.rs` 相对 `590fdc6` 零 diff ✓ |
| 偏差 1 落点与依据 | diff + 注释核对 | `WAL_ROWS=40_000` + 重标定注释在案（40k→8.86s / 160k→40.7s，D5 自身「倍增 N」升级路径已穷尽）；打开阶段用例含 WAL>2KB 断言 ✓ |

### Findings 与偏差分类

1. **[ACT-DEVIATION，根因 BASELINE-CHANGED，非阻塞] 偏差 1（标定落点偏移）**：D10 恢复重建（Iteration 000 accepted 成果）改变恢复耗时量级，D5 的 `200ms ∈ [T/4, T/2]` 窗口落在耗时悬崖内不可稳健命中；Act 按契约升级路径尝试 160k 档后如实记录并改取 40k。spec SHALL 为阶段无关（已打开→close→130/143；未打开→立即退出），每个 THEN 子句均有确定性见证：close 执行由结构测试承担、真二进制退出码/不挂起/数据完整由 e2e ①③ 承担、打开阶段无 close 由锁锚点 + WAL>2KB 断言承担、SIGTERM 143 与无信号回归各自独立——**覆盖无缺口**。
2. **[ACT-DEVIATION，非阻塞] 偏差 2（结构测试夹具 schema checkpoint）**：首版夹具命中的正是 D7 载明的 DDL-无-WAL 模型（witness 夹具以 checkpoint 处理），修正方向与设计一致，契约断言不变。
3. **[MINOR] 偏差 3（cli_test 形式化收编）**：非语义改动（import 排序、`matches!`/`is_multiple_of`），工作区存量在案，随收编保留。
4. **[MINOR] Act 遗留 ①②③**：ENOLCK 场景无自动化见证（本地不可构造，代码路径在案）——接受；`args.sql` 一次 clone——一次性 CLI 可忽略；D10 恢复耗时量化数据（40k→8.86s / 160k→40.7s）作为 improvement 候选「撕裂树运行期根修/驱逐改造」的支撑数据随收尾登记。
5. 结构测试 Notify 握手时序经独立推演确认确定性（阶段 1 signal future 被 drop 后 permit 存入 `Notify`，阶段 2 必然立即收到——无竞态窗口）。

### Acceptance 判定

T2/T3（存量收编 + RED 复现在案）、T4（生产接线 + 结构测试 + e2e 转绿）、T5（全量门无白名单 636/0/2 + clippy/fmt/validate）全部满足 → **Iteration 001 完成**。

### 后继产物

- **Next Cycle**: None
- **Next Iteration**: **None**——change 两个 Iteration（000/001）全部 accepted，**change 实施面完成**
- 收尾待用户指令：commit 触发、`openspec-docs-maintainer` 同步全局状态（SNAPSHOT / roadmap MS10-T02 → completed / 归档 change）与 improvement 登记（撕裂树运行期根修+量化数据、`mark_tx_aborted` 空转、update→delete 旧值重现）、`openspec-experience-recorder` 落撕裂树 Incident（003 Act Response 具备完整诊断链）
