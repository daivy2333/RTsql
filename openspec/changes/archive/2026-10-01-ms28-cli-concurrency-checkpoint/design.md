# ms28-cli-concurrency-checkpoint — Design

## 现状行为（Current-State Evidence，2026-10-01 Plan 读码复核）

- **锁获取单点**：`src/storage/file_storage.rs:67` `try_lock_with_backoff(file, path, lock_fn)`——`try_lock`/`try_lock_shared` 失败（`WouldBlock`）后同 fd `std::thread::sleep(2ms)` × 4 次重试（连同首次 5 次尝试，总窗口 ~8ms），耗尽 `StorageError::DatabaseLocked(path)`；`Error(e)` 非 `WouldBlock` 直传不重试。两个调用点：读写独占 `open_with_key`:107、只读共享 `open_readonly_with_key`:155。锁先于 WAL 打开与恢复（`:104-107` 注释 + 代码序）。
- **FileStorage open 家族**（全同步 fn）：`open`→`open_with_key`（`create(true)`）；`open_readonly`→`open_readonly_with_key`（只读句柄、不创建、缺失 `NotADatabase`）。头校验/KDF（Argon2id，同步阻塞数十至数百 ms）均在 async 上下文内直接执行（既有事实）。
- **Database open 家族**：`open`→`open_with_isolation`→`open_with_key`→`open_internal(mode, path, isolation, key)`（`src/database.rs:90-192`）；`open_readonly*` 亦汇入 `open_internal`；`open_in_memory` 走 `MemoryStorage`（无锁）。`open_internal` 步骤 1 直接同步调用 `FileStorage::open_*`。
- **CLI 信号编排**：`src/cli/mod.rs:462` `execute_command_inner_mode`——阶段 1 `tokio::select!`（open future vs 信号 future）：信号处理器在 future 构造期同步安装（MS25 T11，早于 open 首次轮询），但 select 与 open 同任务，open 的同步前缀（文件锁 + KDF）阻塞期间信号分支不可达；信号分支 `return ExitStatus::Signaled(2/15)`。阶段 2 work select + `close()`。
- **CLI 开库汇聚点**：主命令与全部开库生命周期子命令（new/schema/dump/restore/import/stats/sample/profile）均经 `execute_command_inner`→`execute_command_inner_mode` 的三个 open 臂（`:476-484`，memory 臂 `open_in_memory`）；ATTACH 独立两点 `attach_database`:581-585；`delete_db`:110 自有 `File::open`+`try_lock` 探测（无重试，`WouldBlock`→exit 4）。
- **CLI 通道先例**：`--key`＝clap `#[arg(long, global = true, env = "RTSQL_KEY")]`（flag 优先由 clap 保证；空值在 `execute_command` 顶部对全命令 exit 2）；`RTSQL_WAL_CHECKPOINT_BYTES`＝手动解析（仅开库路径校验 exit 2，open 成功后 `set_checkpoint_bytes_threshold` 单点应用）。
- **checkpoint 面**：`Database::checkpoint()`（`src/database.rs:503`，pub，`Result<()>`）委托 `checkpoint_manager`（只读/内存为 None→`Ok(())`）；MS18 零工作守卫谓词 `get_write_count() == 0 && !has_dirty_pages()`（`WalWriter::get_write_count` writer.rs:167 pub、`BufferPool::has_dirty_pages` buffer_pool.rs:243 pub；`db.wal_writer`/`db.buffer_pool` 为 pub 字段）——恢复重放的 redo 页置脏，故「WAL 有未决」时谓词正确为 false。零工作早退 `Ok`，不触 WAL/位点/脏页。
- **既有拒绝面**：`--readonly` × 任意子命令 → 全局守卫 exit 2（`:242`）；六 lifecycle db 目标 × `:memory:` → `lifecycle_db_target` 守卫 exit 2（`:232`）；缺库 → `resolve_existing_db`（lifecycle）exit 1 / 主命令存在性校验（`:363`）exit 1。
- **存量锁测试**：`tests/database_file_lock_test.rs` 9 测试（同进程双开、FileStorage 双开、重试耗尽×读写/只读、错误文案、坏文件锁优先、close→重开循环 300 轮）；另有 readonly_session/attach_session/cli_test 的锁冲突用例——默认预算变 5s 后凡跨越旧 ~8ms 窗口的断言用例需机械校准。

## 目标行为

见 delta specs（`specs/database-file-lock`、`specs/cli-noninteractive-shell`、`specs/readonly-open-mode`）：锁等待预算可配置（默认 5000ms、内部 ~8ms 下限恒存）、`--busy-timeout`/`RTSQL_BUSY_TIMEOUT`/lib 三通道、超时维持 `DatabaseLocked` exit 4、`rtsql checkpoint <db>` 显式落盘子命令（零工作明示）、等待可被 SIGINT/SIGTERM 中断。

## 设计决策

- **D1 等待预算模型**：`try_lock_with_backoff` 改 deadline 循环——首试→`WouldBlock`→2ms 间隔重试，`Instant::now() >= deadline` 即 `DatabaseLocked`。`deadline = now + max(预算, LOCK_VISIBILITY_FLOOR)`；`LOCK_VISIBILITY_FLOOR = 8ms`（既有可见性窗口吸收，正确性机制恒存，非用户策略）；`DEFAULT_LOCK_WAIT = 5000ms`；`None` → 默认（应用单点在 FileStorage 锁函数内）。2ms 固定间隔：5s 预算 ≈ 2500 次 flock 系统调用，可忽略；预算 ≤ 8ms 时行为与现状同窗。`Some(0)` ＝「不额外等待」，仅下限生效（即现状语义的显式表达）。
- **D2 lib 形状（加性，委托壳先例 MS17）**：`FileStorage::open_with_lock(path, key, lock_wait: Option<Duration>)` 与 `open_readonly_with_lock(path, key, lock_wait)` 为新核心；既有 `open/open_with_key/open_readonly/open_readonly_with_key` 签名零变化、委托传 `None`。`Database::open_with_lock_wait(path, isolation, key, lock_wait: Option<Duration>)` 为新核心，`open_internal` 加第 5 参，既有 `open/open_with_isolation/open_with_key/open_readonly/open_readonly_with_key/open_in_memory` 委托。**默认值本身是行为变化**（既有调用方锁冲突等待从 ~8ms 变为 5s 预算）——这是 MS28 Outcome 的目标行为，不是兼容性事故；存量测试校准白名单见 T4。
- **D3 CLI 通道（对齐 `--key` 先例，偏离 `RTSQL_WAL_CHECKPOINT_BYTES` 手动解析先例）**：`CliArgs` 增 `busy_timeout: Option<u64>`，`#[arg(long, value_name = "MS", global = true, env = "RTSQL_BUSY_TIMEOUT")]`。clap value_parser（u64）使非法 flag/env（非数字/负数/空）在任何命令前 exit 2（usage 错误），flag 显式提供时 env 不消费（clap 语义即「显式优先」）。与水位通道「仅开库路径校验」的偏差是刻意的：clap env 解析天然全局，且 `RTSQL_KEY` 空值先例已是全命令校验——全局校验语义更简单可预期。`:memory:` 不加守卫：锁参数对 `MemoryStorage` 天然无消费点，接受并忽略（避免全局 env 误伤内存库使用，proposal 假设 2）。
- **D4 穿线路径（单汇聚点）**：`execute_command_inner`/`execute_command_inner_mode` 加 `lock_wait: Option<Duration>` 参数，覆盖主命令三臂（memory 臂不消费）+ 全部开库生命周期子命令；新增 checkpoint 子命令同样经此通道。`attach_database` 两 open 点由 `run_sql` 会话链路穿线同值。`delete_db` **不穿线**（自有探测路径，破坏性操作保持立即探测立即拒绝，行为不变）。
- **D5 等待可中断（信号语义保持，零信号分支改动）**：`open_internal` 步骤 1 的 FileStorage 构造（ReadWrite/ReadOnly 两臂）改在**专用 `std::thread`** 中执行，经 `tokio::sync::oneshot` 回传结果，async 侧 await——open future 在锁等待期间真实让出，阶段 1 `tokio::select!` 的信号分支可达，SIGINT/SIGTERM 即时 `return ExitStatus::Signaled(n)`（分支代码零修改）。立即退出的机理：detached `std::thread` 不被 tokio runtime drop 等待，`#[tokio::main]` 的 main 返回即进程退出、线程随进程终止，fd/锁由 OS 回收——与「打开阶段信号无 close 立即退」（≡ kill -9，恢复收敛）既有取消安全论证一致，且无需 `process::exit`。附带收益：Argon2id KDF 移出 reactor。`Memory` 臂无锁无阻塞，保持内联。错误映射：oneshot `RecvError`（线程 panic）→ `StorageError::Internal`——仅 panic 形态的观察面变化，正常错误路径逐字节不变。备选否决：FileStorage open 异步化（公共签名 churn 大）；`spawn_blocking` + 阶段 1 信号分支 `process::exit`（runtime drop 等待 blocking task 导致仅 return 不够，而 process::exit 使注入信号源的在进程结构测试不可用——本方案两者兼得）；不处理（并发特性带 5s 不可中断等待，UX 不可接受）。
- **D6 checkpoint 子命令**：`Command::Checkpoint { db }` → `lifecycle::checkpoint_db(db, key, lock_wait)`：`resolve_existing_db`（缺失 exit 1）→ `execute_command_inner` 两阶段编排（rw 独占开库；锁冲突经预算后 `open_error_status` exit 4）→ work 闭包内**先取零工作谓词**（`db.wal_writer.get_write_count() == 0 && !db.buffer_pool.has_dirty_pages()`；单会话持独占锁期间无并发写者，取样稳定；恢复重放置脏使「WAL 有未决」正确判 false）→ `db.checkpoint().await`。输出 stdout 一行：零工作 `checkpoint complete (no pending work; wal untouched)`，正常 `checkpoint complete (wal truncated; checkpoint site advanced)`；失败 `ExitStatus::General("checkpoint failed: ...")`（exit 1，词根对齐 MS26 自动 checkpoint 失败点名）。`:memory:` 拒绝：`lifecycle_db_target` 增 Checkpoint 臂（守卫 exit 2）；`--readonly` 组合由既有全局子命令守卫自动 exit 2；加密库经既有全局 `--key`，错钥 `InvalidKey` → exit 5。只读/内存会话 `checkpoint()` 的 `Ok(())` 空转臂不可达（组合已在前置守卫拦截），保持原样不删。
- **D7 readonly-open-mode R1 校准**：其「冲突 SHALL NOT 等待或重试」措辞与共享锁等待预算冲突，MODIFIED 为「按 `database-file-lock` R1 等待预算执行」；「写者持独占锁时只读者被拒」场景语义保持（预算耗尽后拒绝）。
- **D8 校准白名单原则**：凡断言 `DatabaseLocked`/exit 4 且持锁跨越旧 ~8ms 窗口的用例，改为显式短预算（lib 经新参数 `Some(Duration::from_millis(0))`；CLI 子进程经 `RTSQL_BUSY_TIMEOUT=0` env），断言语义零变化；close→重开循环、错误文案、锁优先于坏文件报错等不依赖等待时长的用例零校准。

## 风险与非实质未知项

- **套件时延**：校准遗漏的锁冲突用例每个 +5s。T4 白名单是兜底校准轮；Act 全量跑批时若发现超时用例按 D8 原则就地校准（非实质，Response 记录）。
- **spawn_blocking panic 面**：open 路径 panic 由 JoinError 转 `Internal` 错误返回（原为进程 panic）。无测试依赖该形态。
- **WSL2 flock 释放窗口**：由 `LOCK_VISIBILITY_FLOOR` 承接，`test_close_reopen_loop_no_false_locked`（300 轮）是既有回归见证，零校准。
- **env 全局校验**波及 `list/new/delete/completions`：与 `RTSQL_KEY` 先例一致，属目标语义。
- 非实质（留 Act）：确认行文案微调、测试 helper 命名、clap 非法值文案细节；均不影响契约语义。
