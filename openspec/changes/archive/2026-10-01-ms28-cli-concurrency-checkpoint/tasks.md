# ms28-cli-concurrency-checkpoint — Tasks

## 任务清单

| Task | 目标 | Iteration | Requirement 映射 |
|---|---|---|---|
| T1 | FileStorage 锁等待策略化：`try_lock_with_backoff` 改 deadline 循环（`wait: Duration` 参数、2ms 间隔、`LOCK_VISIBILITY_FLOOR` ≈8ms 下限恒存、`DEFAULT_LOCK_WAIT` 5000ms、`None`→默认）；`FileStorage::open_with_lock(path, key, lock_wait: Option<Duration>)` / `open_readonly_with_lock(path, key, lock_wait)` 新核心，既有 4 个 open fn 签名零变化委托 `None` + lib 单测（无争用打开、预算内等待成功、预算耗尽拒绝、`Some(0)` 仅下限窗口、非 `WouldBlock` 直传） | 000 | database-file-lock R1（MODIFIED） |
| T2 | lib 打开面通道：`Database::open_with_lock_wait(path, isolation, key, lock_wait: Option<Duration>)` 新核心 + `open_internal` 第 5 参 + 既有 open 家族委托壳零变化；FileStorage 构造段移入专用 `std::thread` + `tokio::sync::oneshot` 回传（RecvError→`StorageError::Internal`；Memory 臂内联不变）——open future 在锁等待期间让出、信号分支可达、detached 线程随进程退出终止（design D5）+ 单测（同进程双开经预算拒绝、显式 0 立即拒绝、默认预算下持有者释放后可达、注入即时信号返回 `Signaled`） | 000 | database-file-lock R1（lib 面） |
| T3 | CLI 通道：`CliArgs.busy_timeout: Option<u64>`（`--busy-timeout <MS>` global + env `RTSQL_BUSY_TIMEOUT`，clap 校验非法值 exit 2、flag 优先）；`execute_command_inner`/`execute_command_inner_mode` 加 `lock_wait` 参数穿线主命令与全部开库子命令（memory 臂不消费）；`attach_database` 两 open 点穿线；信号分支零改动（阶段 1 让出由 T2 承载）+ cli/subprocess 测试（flag 等待后成功、env 生效、flag 优先、非法值 exit 2、`:memory:` 组合接受忽略、超时 exit 4 错误面、等待中 SIGINT 立即退 130） | 000 | cli-noninteractive-shell「锁等待 busy-timeout 通道」（ADDED）、database-file-lock R1（CLI 面）、readonly-open-mode R1（MODIFIED，共享锁等待） |
| T4 | 存量锁用例机械校准（D8 白名单原则）：断言 `DatabaseLocked`/exit 4 且持锁跨越旧 ~8ms 窗口的用例显式短预算——`database_file_lock_test`（同进程双开/FileStorage 双开/重试耗尽×2/错误文案持锁组）、`readonly_session_test` 冲突组、`attach_session_test` 锁定附加、`cli_test` 锁冲突子进程（env `RTSQL_BUSY_TIMEOUT=0`）；close→重开循环/坏文件锁优先等不依赖等待时长的用例零校准；断言语义零变化 | 000 | database-file-lock R1 场景零回归、readonly-open-mode R1 场景零回归 |
| T5 | checkpoint 子命令：`Command::Checkpoint { db }` + `lifecycle_db_target` 增臂（`:memory:` exit 2）+ `lifecycle::checkpoint_db`（`resolve_existing_db` 缺失 exit 1 → `execute_command_inner` rw 开库 → 零工作谓词前置取样 → `Database::checkpoint()` → stdout 一行确认含零工作明示；失败 General `checkpoint failed: ...`）+ 测试矩阵（正常落盘 WAL 截断可见、零工作明示、缺库 exit 1、锁冲突经预算 exit 4、`--readonly`/`:memory:` exit 2、`--key` 成功/错钥 exit 5） | 001 | cli-noninteractive-shell「生命周期子命令 checkpoint」（ADDED） |
| T6 | 文档同步：`rtsql-docs/SKILL.md` 命令行功能目录增 `--busy-timeout`/`RTSQL_BUSY_TIMEOUT` 通道说明与 `rtsql checkpoint` 子命令条目、锁等待默认值与并发使用语义；README 双语对应段核验一致；`openspec validate` | 001 | 文档任务（无 spec 映射，验收 = 文档与实现逐条一致） |

## Iteration Plan

### Iteration 000: 锁等待策略可配置（busy-timeout 三通道）

- Tasks: T1, T2, T3, T4
- Depends on: None
- 交付（2026-10-01 Act Response `reported` + Plan Review `accepted`）：T1 `try_lock_with_backoff` deadline 循环（`LOCK_VISIBILITY_FLOOR=8ms` 下限恒存 + `DEFAULT_LOCK_WAIT=5000ms`，2ms 间隔）+ `FileStorage::open_with_lock`/`open_readonly_with_lock` 新核心（既有 4 fn 委托壳零变化）；T2 `Database::open_with_lock_wait` + `open_internal` 第 5 参 + `construct_file_storage`（专用 std::thread + oneshot，锁等待期间 open future 让出、信号分支零改动即时中断）+ Plan 遗漏补口 `open_readonly_with_lock_wait`（Deviation 1，只读臂预算可达）；T3 `CliArgs.busy_timeout`（global+env，clap 校验 exit 2、flag 优先）单点解析穿线 8 生命周期子命令 + 主命令 + ATTACH 两点（memory 臂不消费、`delete_db` 不穿线）；T4 四测试文件 D8 校准（`ZERO_WAIT`/`RTSQL_BUSY_TIMEOUT=0`，断言零变化；close→重开 300 轮等零校准保持）。新增 `tests/busy_timeout_test.rs` 18 例 + cli/mod.rs 单元 1（阶段 1 信号中断 `Signaled(2)`）；全量 1651 / 0 failed / 2 ignored（分批 M19）；`database_file_lock_test` 0.24s 证明校准生效。5 项 Deviation 与 3 项 Minor（SIGTERM 无直接见证、lock_wait 双点解析、WaitTimeout 夹具 helper）均非阻塞，见 Cycle Plan Review。逐项见 `iterations/000-initial/000-initial.md`。
- Stable baseline: 锁等待预算可配置且默认 5000ms——预算内持有者释放后第二方（独占与只读共享两侧）打开成功；预算耗尽维持 `DatabaseLocked`（CLI exit 4）既有错误面；`Some(0)`/`RTSQL_BUSY_TIMEOUT=0` 回到仅 ~8ms 下限的现状语义；`--busy-timeout`/env 通道全域生效（含 ATTACH、生命周期子命令）、非法值 exit 2、`:memory:` 组合接受忽略；等待期间 SIGINT/SIGTERM 即时退出（130/143）；delete 探测行为不变；存量锁用例校准后全量零回归。
- Verification boundary: `tests/busy_timeout_test.rs`（新，lib+CLI 锁矩阵与通道矩阵）+ `tests/database_file_lock_test.rs` 校准后全绿 + readonly_session/attach_session/cli_test 受影响组 + 注入信号源的阶段 1 退出用例 + 全量分批（M19 纪律）。
- Diagnostic boundary: `src/storage/file_storage.rs`（锁原语与 open 家族）、`src/database.rs`（open_internal 与 open 家族）、`src/cli/mod.rs`（CliArgs/execute_command_inner_mode/attach_database/信号分支）。
- Non-goals: checkpoint 子命令（001）；import 攒批/持锁窗口治理（MS22）；FIFO 公平队列；跨进程多写者；`delete` 等待语义。

### Iteration 001: 显式 checkpoint 子命令与文档收尾

- Tasks: T5, T6
- Depends on: Iteration 000（锁冲突场景消费等待预算与 CLI 通道；文档消费全 change 语义）
- 交付（2026-10-01 Act Response `reported` + Plan Review `accepted`，Next Iteration: None——change 具备收尾条件）：T5 `Command::Checkpoint { db }` + `lifecycle_db_target` 增臂（`:memory:` 守卫自动生效）+ `lifecycle::checkpoint_db`（`resolve_existing_db` → `execute_command_inner` rw 独占开库 → 零工作谓词 checkpoint() 前取样 → 确认行两形态按 design D6 定稿文案 → 失败 `General("checkpoint failed: ...")`）；T6 SKILL.md（`--busy-timeout` flag 行 / `RTSQL_BUSY_TIMEOUT` env 行 / `checkpoint` 子命令行 / 锁等待语义两处 / WAL 段显式命令入口）+ README 双语（命令表 / 只读锁冲突等待语义 / 退出码 4 行 / 自动 checkpoint 段显式入口 + I079 备份同步点 / lib API 清单两新入口）逐条一致，`rtsql --help` 与 `rtsql checkpoint --help` 抽样核验。新增 `tests/checkpoint_command_test.rs` 6 例（修复前 RED ＝ 主命令回退「database 'checkpoint' does not exist」exit 1，非预判的 unknown subcommand）；全量 1657 / 0 failed / 2 ignored（基线 1651 + 6，分批 M19）；`openspec validate --changes --strict` 通过；clippy --tests 与 rustfmt 净。逐项见 `iterations/001-checkpoint-command/000-initial.md`。
- Stable baseline: `rtsql checkpoint <db>` 端到端可用——已提交数据落稳（WAL 截断/位点推进）且外部备份可见；零工作明示 exit 0；缺库 exit 1、锁冲突经预算 exit 4、`--readonly`/`:memory:` exit 2、加密 `--key` 成功/错钥 exit 5；close/水位自动 checkpoint 语义零变化；SKILL.md 与 README 双语逐条一致。
- Verification boundary: `tests/checkpoint_command_test.rs`（新）矩阵全绿 + cli_test 回归 + SKILL.md 命令人工核对与抽样自动执行 + 全量分批（M19）+ `openspec validate`。
- Diagnostic boundary: `src/cli/mod.rs`（Command 枚举/lifecycle_db_target/分派）、`src/cli/lifecycle.rs`（checkpoint_db）、`rtsql-docs/SKILL.md`、`README.md`/`README.zh-CN.md`。
- Non-goals: `--format` 四态支持；checkpoint 进度/统计输出；MS27-T02 完整性校验；只读/内存会话 checkpoint 语义变更。

## 平衡审计

- 000（锁策略四任务）：单一故障域（advisory 锁获取策略），T1→T2→T3 为同一参数自底向上穿线（storage→lib→CLI），拆开任一段都产生「通道存在但不可达」的半成品面；T4 是默认值行为变化的必要伴随校准。共享验证边界（锁矩阵 + 通道矩阵）与诊断面（file_storage/database/cli-mod）——不拆。
- 001（子命令 + 文档）：T5 独立可验收子命令，T6 消费全 change 语义属收尾批（MS26/前 change 收尾批先例）；与 000 验证面不同（子命令 e2e vs 锁矩阵）、无反向依赖——不合，独立成轮。
- 两 Iteration 工作量适中（000 中、001 小），各自形成稳定可验证阶段成果——通过审计。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| database-file-lock R1（MODIFIED，等待预算可配置） | T1, T2 | `busy_timeout_test`（预算内成功/耗尽拒绝/`Some(0)` 下限/共享锁同款）+ `database_file_lock_test` 校准组 | Covered |
| database-file-lock R1 场景（close→重开、错误面不变） | T1, T4 | `test_close_reopen_loop_no_false_locked` 零校准保持 + 重试耗尽拒绝用例 | Covered |
| cli-noninteractive-shell ADDED（busy-timeout 通道） | T3 | cli/subprocess 通道矩阵（flag/env/优先级/非法值 exit 2/`:memory:` 忽略/超时 exit 4） | Covered |
| readonly-open-mode R1（MODIFIED，共享锁预算等待） | T3, T4 | `busy_timeout_test` 只读共享锁等待用例 + `readonly_session_test` 校准组 | Covered |
| cli-noninteractive-shell ADDED（生命周期子命令 checkpoint） | T5 | `checkpoint_command_test` 矩阵（落盘/零工作/缺库/锁冲突/组合拒绝/加密） | Covered |
| 文档一致性（T6，无 spec 映射） | T6 | SKILL.md/README 逐条人工核对 + 抽样自动执行 | Covered |
