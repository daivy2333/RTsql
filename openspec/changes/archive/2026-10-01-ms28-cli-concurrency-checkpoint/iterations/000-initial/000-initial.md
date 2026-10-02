# Iteration 000 / Cycle 000-initial: 锁等待策略可配置（busy-timeout 三通道）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3, T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS28 全部任务（T5/T6 属 Iteration 001）；proposal 默认假设 1-9（默认 5000ms、`:memory:` 接受并忽略、毫秒单位、`RTSQL_BUSY_TIMEOUT` 命名、0=仅下限、下限恒存、delete 不等待、迭代序、文档面）；design D1-D8。
- Excluded scope: checkpoint 子命令（T5）、文档同步（T6）、import 攒批/持锁窗口治理（MS22）、FIFO 队列、跨进程多写者。

**Objective**

锁等待预算可配置（flag/env/lib 三通道、默认 5000ms、内部 ~8ms 下限恒存），独占与只读共享锁两侧统一生效；预算内持有者释放后第二方打开成功，预算耗尽维持 `DatabaseLocked`（CLI exit 4）既有错误面；等待可被 SIGINT/SIGTERM 即时中断；`delete` 探测行为不变；存量锁用例按显式短预算机械校准后全量零回归。

**Investigation Facts**

- Current Baseline: 分支 master @ 4e1944f，工作区含 2026-10-01 milestone 重排未提交项（tasks/improvements/SKILL.md，与本 change 代码面无冲突）；全量 1632 tests / 0 failures / 2 ignored（2026-10-01 SNAPSHOT 采信，覆盖面未变化部分直接引用）。锁冲突现状：`WouldBlock` 后 2ms×4 重试（~8ms 窗口）即 `DatabaseLocked`。
- Current-State Evidence（2026-10-01 Plan 读码，行号以当前 revision 为准）：
  - `src/storage/file_storage.rs:67` `try_lock_with_backoff(file, path, lock_fn)`——重试循环本体；调用点 `:107`（独占，`open_with_key` 内）与 `:155`（共享，`open_readonly_with_key` 内）；锁先于头校验与 WAL（`:104-107`）。`WouldBlock`/`Error(e)` 两分支；`DatabaseLocked(path.display())`。
  - `FileStorage` open 家族全同步：`open`→`open_with_key`（`create(true)`）；`open_readonly`→`open_readonly_with_key`（只读句柄、不创建、缺失→`NotADatabase`）；头校验/KDF 同步执行。
  - `src/database.rs:90-157` open 家族全部委托 `open_internal(mode, path, isolation, key)`（`:173`）；步骤 1（`:180-192`）三臂构造 `Arc<dyn AsyncStorage>`——ReadWrite→`FileStorage::open_with_key`、ReadOnly→`open_readonly_with_key`+`assert_no_pending_wal`、Memory→`MemoryStorage::new()`；`Database::checkpoint()`（`:503`）与 `close()`（`:487`）本 change 不触碰。
  - `src/cli/mod.rs:83-101` `CliArgs`（`key` 字段 `:92` 是通道先例：`#[arg(long, global = true, env = "RTSQL_KEY")]`）；`:216-238` `execute_command` 顶部守卫（空 key exit 2 / 水位 env 校验 / `:memory:`×子命令 / `--readonly`×子命令）；`:375` 主命令调 `execute_command_inner_mode`；`:447/462` 两阶段编排（阶段 1 select open vs 信号，`:485-492`；信号分支 `return ExitStatus::Signaled(2/15)`）；`:559-589` `attach_database` 两 open 点（`:582/584`）；`:110`（lifecycle.rs）`delete_db` 自有 `File::open`+`try_lock` 探测（无重试）。
  - 生命周期子命令全部经 `execute_command_inner`（lifecycle.rs `:37/205/249/439/544/847/1068/1136`）——穿线单汇聚点成立。
  - 存量锁测试：`tests/database_file_lock_test.rs` 9 测试（`expect_locked`/`assert_locked` helper；同进程双开、FileStorage 双开、重试耗尽×读写/只读、错误文案、坏文件锁优先、close→重开 300 轮）；结构测试 `phase2_signal_runs_close_and_returns_signaled`（cli/mod.rs `:903`）注入信号源、阶段 1 恒 open 胜出——阶段 1 分支零改动的佐证与回归锚。
- Code and Critical Path: 锁参数流＝CLI `CliArgs.busy_timeout`（clap 解析+校验）→ `execute_command(_inner_mode)` / `attach_database` / `lifecycle::checkpoint_db`（001）→ `Database::open_with_lock_wait` → `open_internal` 第 5 参 → `FileStorage::open_with_lock`/`open_readonly_with_lock` → `try_lock_with_backoff(wait)`。`None` 在 FileStorage 锁函数单点解析为 `DEFAULT_LOCK_WAIT`。等待期间 open future 让出（std::thread + oneshot），CLI 阶段 1 select 可达信号分支。

**Implementation Guidance**

实施顺序 T1→T2→T3→T4（自底向上穿线，每步可独立编译验证）。关键点：

- D1：deadline 循环 `let deadline = Instant::now() + wait.max(LOCK_VISIBILITY_FLOOR)`；首试→2ms sleep→重试直至 deadline；`Some(0)` 经 `max` 自然落到 ~8ms 下限＝现状窗口。`LOCK_VISIBILITY_FLOOR`/`DEFAULT_LOCK_WAIT` 为 file_storage 模块常量（pub(crate) 或 pub，随 lib 通道需要）。
- D2：`open_with_lock`/`open_readonly_with_lock` 为新核心（含既有全部头校验/KDF/创建语义），既有 4 fn 一行委托 `None`。
- D5：线程回传用 `tokio::sync::oneshot`；闭包捕获 owned `PathBuf`/`Option<String>`/`lock_wait`；`RecvError` → `StorageError::Internal("storage init failed: ...")`；`Memory` 臂内联不动；ReadOnly 臂的 `assert_no_pending_wal` 仍在 async 侧、构造之后（次序不变）。
- D3：clap 字段 `busy_timeout: Option<u64>`，value_name `"MS"`；穿线时 `Duration::from_millis(v)`。非法值文案由 clap usage 机制给出（无需手写校验函数）。
- D8 校准模式：lib 测试经新参数 `Some(Duration::from_millis(0))`；CLI 子进程测试经 `RTSQL_BUSY_TIMEOUT=0` env 注入（Command::env）。
- 测试放置：新文件 `tests/busy_timeout_test.rs`（锁矩阵 + 通道矩阵 + 信号中断），校准就地修改既有文件。

**Task Contracts**

### T1: FileStorage 锁等待策略化

- Requirement/Scenario: database-file-lock R1 全部场景（单进程独占成功/预算内获得/第二持有者拒绝/零预算/只读共享同款/同进程双开/close→重开/锁调用失败严格报错）
- Depends on: None
- Targets: `src/storage/file_storage.rs::try_lock_with_backoff`、`FileStorage::open_with_key`/`open_readonly_with_key`/`open`/`open_readonly`
- Current behavior: `WouldBlock` 固定 2ms×4 重试（~8ms）后 `DatabaseLocked`；两调用点硬编码；无等待预算概念
- Required behavior: `try_lock_with_backoff` 增 `wait: Duration` 参数改 deadline 循环（2ms 间隔、`wait.max(LOCK_VISIBILITY_FLOOR)` 为窗口）；新增 `FileStorage::open_with_lock(path, key: Option<&str>, lock_wait: Option<Duration>)` 与 `open_readonly_with_lock(path, key, lock_wait)` 核心（构造语义与既有逐字节一致，仅锁窗口参数化）；既有 4 个 open fn 签名与行为零变化、委托传 `None`；`None` → `DEFAULT_LOCK_WAIT`（5000ms）
- Preserve: 锁先于头校验与 WAL；非 `WouldBlock` IO 错误直传不重试；`DatabaseLocked` 文案含路径；只读不创建、缺失 `NotADatabase`；锁生命周期（fd 同生命周期）
- Forbidden: 不改 `MemoryStorage`；不改 `delete_db` 探测（lifecycle.rs）；不改互斥矩阵语义；不引入异步签名；不改锁释放时机
- Test witness: `tests/busy_timeout_test.rs` lib 臂——**RED**：`等待预算内获得锁`（测试进程持独立 flock → 后台线程延时释放 → 第二方 `open_with_lock(_, _, Some(5s))` 成功；现状实现必在 ~8ms 后 `DatabaseLocked`）；GREEN：无争用打开、预算内成功、预算耗尽拒绝（`Some(100ms)` + 持续持有）、`Some(0)` 仅下限、共享锁同款四态
- GREEN condition: 上述矩阵全绿 + `database_file_lock_test` 校准后全绿（T4 完成后）
- Verification: `cargo test --test busy_timeout_test`；`cargo test --test database_file_lock_test`
- Stop when: 需要改锁互斥/生命周期语义本身，或发现 deadline 参数无法在两调用点承载（契约失效返回 Plan）

### T2: lib 打开面通道与可中断 open

- Requirement/Scenario: database-file-lock R1（lib 面）；等待可中断的结构前提（`cli-noninteractive-shell` ADDED 场景「超时错误面不变」的对称面：信号中断）
- Depends on: T1
- Targets: `src/database.rs::open_with_lock_wait`（新）、`open_internal`、既有 open 家族（`open`/`open_with_isolation`/`open_with_key`/`open_readonly`/`open_readonly_with_key`/`open_in_memory`）
- Current behavior: `open_internal(mode, path, isolation, key)` 四参；步骤 1 同步构造 FileStorage（锁等待阻塞 async 任务与 select）
- Required behavior: `open_internal` 增第 5 参 `lock_wait: Option<Duration>`；新 pub 核心 `Database::open_with_lock_wait(path, isolation, key, lock_wait)`，既有家族委托（`open_in_memory` 不消费）；ReadWrite/ReadOnly 臂的 FileStorage 构造移入专用 `std::thread` + `tokio::sync::oneshot` 回传（RecvError → `StorageError::Internal`），async 侧 await——锁等待期间 open future 让出；`Memory` 臂内联不变；`assert_no_pending_wal` 仍在构造后 async 侧
- Preserve: 三模式构造裁剪与次序；既有 open 家族签名与默认行为（默认预算 5s 是本 change 目标行为变化，其余逐字节）；恢复/checkpoint 构造序
- Forbidden: 不改 WAL/恢复/`assert_no_pending_wal` 语义；不改 `close()`/`checkpoint()`；不引入 spawn_blocking+process::exit 方案；不改阶段信号分支（cli/mod.rs）
- Test witness: `tests/busy_timeout_test.rs` lib 臂扩展——**RED**：`open_with_lock_wait` 不存在（编译期见证）+ 同进程双开经 `Some(0)` 立即拒绝（现状同进程双开必等 ~8ms，语义近似；RED 主体为 API 缺失）；GREEN：显式预算双开拒绝、默认预算下持有者延时释放后可达、注入即时信号源经 `execute_command_inner` 等待中返回 `Signaled(2)`（open 在 thread 中等待、select 信号胜出）
- GREEN condition: 上述用例全绿；`phase2_signal_runs_close_and_returns_signaled` 不改动仍绿
- Verification: `cargo test --test busy_timeout_test`；`cargo test --lib`
- Stop when: 线程/oneshot 捕获出现非 `Send` 状态（契约前提失效返回 Plan）

### T3: CLI 通道穿线

- Requirement/Scenario: cli-noninteractive-shell ADDED「锁等待 busy-timeout 通道」全部场景；readonly-open-mode R1（MODIFIED）CLI 面
- Depends on: T2
- Targets: `src/cli/mod.rs::CliArgs`、`execute_command`、`execute_command_inner`/`execute_command_inner_mode`、`attach_database`、主命令/生命周期分派臂
- Current behavior: 无 busy-timeout 通道；编排 fn 固定默认锁行为；ATTACH 同
- Required behavior: `CliArgs.busy_timeout: Option<u64>`（`#[arg(long, value_name = "MS", global = true, env = "RTSQL_BUSY_TIMEOUT")]`——clap 完成 u64 校验与 flag>env 优先，非法值 usage exit 2）；`execute_command_inner(_mode)` 增 `lock_wait: Option<Duration>` 参数并下发三 open 臂（memory 臂不消费）；全部 `execute_command_inner` 调用方（lifecycle 8 处）与主命令、`attach_database` 两点穿线同值；`:memory:` 不加守卫（MemoryStorage 无消费点＝接受并忽略）；`delete_db` 不穿线
- Preserve: 阶段 1/2 信号分支与退出码映射（130/143）；`open_error_status` 分类；`--readonly`×子命令与 `:memory:`×子命令守卫先行序；`RTSQL_WAL_CHECKPOINT_BYTES` 通道行为
- Forbidden: 不改 `delete_db`；不给 `:memory:` 加 busy-timeout 拒绝；不改水位 env 解析先例；不改 ATTACH 错误分类
- Test witness: `tests/busy_timeout_test.rs` CLI 臂——**RED**：`--busy-timeout` flag 不存在（clap usage error）；GREEN：flag 等待后成功（持有者延时释放）、env 生效、flag 优先、`--busy-timeout abc`/`-1`/env 空 exit 2、`:memory:` + env/flag exit 0、超时 exit 4 文案含库名、等待中 SIGINT exit 130（子进程 `kill` + wait）、ATTACH 锁定场景经预算
- GREEN condition: 通道矩阵全绿 + cli_test 存量用例（校准后）全绿
- Verification: `cargo test --test busy_timeout_test`；`cargo test --test cli_test`
- Stop when: clap global+env 组合无法满足「显式 flag 优先」（与 `--key` 先例冲突时返回 Plan）

### T4: 存量锁用例机械校准

- Requirement/Scenario: database-file-lock R1 与 readonly-open-mode R1 的零回归场景（close→重开可达、错误面不变）
- Depends on: T1-T3（默认值生效后校准才有意义）
- Targets: `tests/database_file_lock_test.rs`、`tests/readonly_session_test.rs`、`tests/attach_session_test.rs`、`tests/cli_test.rs`（锁冲突用例组）
- Current behavior: 用例依赖默认行为＝~8ms 即拒绝；默认变 5s 后持锁跨越窗口的断言用例每个 +5s
- Required behavior: D8 白名单原则——凡断言 `DatabaseLocked`/exit 4 且持锁跨越旧窗口的用例显式短预算：lib 用例经 `open_with_lock`/`open_with_lock_wait(…, Some(0))`；CLI 子进程用例经 `RTSQL_BUSY_TIMEOUT=0` env；断言本身（错误变体/文案/退出码）零变化
- Preserve: `test_close_reopen_loop_no_false_locked`、坏文件锁优先、错误文案用例等不依赖等待时长者**零校准**；测试语义（断言对象）一律不变
- Forbidden: 不删除或弱化任何锁断言；不为校准引入新判定工具（只改参数/env 注入）
- Test witness: 校准前后同名测试断言不变（diff 仅参数/env 行）；全量套件无 5s 级时延回归
- GREEN condition: 受影响测试文件全绿且套件总时延无显著增长（无单个用例 >1s 级新增等待）
- Verification: `cargo test --test database_file_lock_test --test readonly_session_test --test attach_session_test --test cli_test`
- Stop when: 发现某用例语义上必须依赖「默认即立即拒绝」（与目标行为冲突，返回 Plan 裁定）

**Invariants**

- 锁互斥矩阵（读读并发/读写互斥/写写互斥）与锁生命周期（fd 同生命周期、drop/进程退出释放）不变。
- `DatabaseLocked` 错误文案与 CLI exit 4 映射不变；`StorageError` 其余变体传播路径不变。
- 打开序（锁 → 头校验 → WAL/恢复）不变；被拒第二打开者不触碰伴生文件。
- `delete` 子命令探测语义（立即、无重试）不变；`:memory:`/lib `Database::open` 建库语义不变。
- 既有 open 家族公开签名零变化；`--key`/水位 env 通道行为不变。
- 阶段 1/2 信号分支代码与退出码映射零改动；`phase2_signal_runs_close_and_returns_signaled` 不改动保持绿。

**Non-goals**

- checkpoint 子命令与文档（Iteration 001）；import 攒批/持锁窗口治理（MS22）；FIFO 公平队列；`rtsql serve`（I071）；跨进程多写者；`delete` 等待语义；锁公平性。

**Acceptance**

1. 默认（无任何配置）：第二方在持有者释放前打开会等待，5s 预算内释放则成功、否则 `DatabaseLocked`——独占与只读共享两侧一致（R1 场景 1/2/5；T1/T2/T3）。
2. `--busy-timeout <MS>` 与 `RTSQL_BUSY_TIMEOUT` 可配置预算，flag 优先；非法值任意命令前 exit 2（ADDED 通道场景 1/2/3；T3）。
3. `Some(0)`/`RTSQL_BUSY_TIMEOUT=0` ＝ 仅 ~8ms 下限窗口即拒绝（现状语义显式表达）（R1 场景 4；T1/T2）。
4. `:memory:` 组合 flag/env 接受并忽略 exit 0（ADDED 通道场景 5；T3）。
5. 等待期间 SIGINT 即时退出 130、SIGTERM 143（无 close，与既有打开阶段信号语义一致）（T2/T3）。
6. close→立即重开 300 轮无假 `DatabaseLocked`（既有用例零校准保持）；锁系统调用非 `WouldBlock` 失败严格报错（R1 场景 7/8；T1/T4）。
7. 存量锁用例校准后全绿、断言语义不变；全量分批零回归（T4）。

**Verification**

- 逐 scenario 最简直接判定：`cargo test --test busy_timeout_test`（矩阵 1-5）、`cargo test --test database_file_lock_test`（6）、受影响四文件（7）、全量分批 M19（`cargo test -- --test-threads=N` 分批，SNAPSHOT 纪律）。
- RED 见证：T1/T2/T3 各自 Test witness 注明的修复前失败形态（等待成功用例超时失败 / API 缺失 / flag 缺失），记录于 Act Response。
- 人工步骤：无（全部自动可判定）。

**Gate 2 Readiness**

其余检查项 PASS（无 Missing requirement、无未批准裁剪、调查/设计/任务/追踪/验证闭合、无实质未知项、无身份型证据工程、Evidence 模式 none）。**2026-10-01 用户批准计划与 proposal 默认假设 1/2（默认 5000ms、`:memory:` 接受并忽略），原话：批准——Gate 2 全部 PASS。**

**Persisted Evidence**

- Mode: none

（Act Response 记录命令、决定性输出与退出码即足；无不可低成本重跑的决定。）

**Risks and Notes**

- 校准遗漏用例会产生 5s 级时延：T4 是兜底轮；全量分批时按 D8 就地校准并在 Response 记录（非实质）。
- 线程 panic 形态的错误面变化（Internal 化）：无测试依赖，验收不涉及。
- `RTSQL_BUSY_TIMEOUT` 全局校验波及 `list/new/delete/completions`：与 `RTSQL_KEY` 先例一致，目标语义。
- WSL2 flock 释放窗口：下限窗口承接，既有 300 轮循环用例是回归锚。
- 非实质留 Act：测试 helper 命名、clap 非法值文案细节、确认性行文案（001）。

## Act Response

- Status: reported

**Implemented**

- **T1**（`src/storage/file_storage.rs`）：`try_lock_with_backoff` 改 deadline 循环（新增 `wait: Duration` 参、`Instant::now() >= now + wait.max(LOCK_VISIBILITY_FLOOR)` 判定、2ms 间隔不变）；模块常量 `LOCK_VISIBILITY_FLOOR = 8ms`、`DEFAULT_LOCK_WAIT = 5000ms`（均私有）。新核心 `FileStorage::open_with_lock(path, key, lock_wait)` / `open_readonly_with_lock(path, key, lock_wait)`——`None` 在各自函数单点解析为 `DEFAULT_LOCK_WAIT`；既有 4 个 open fn 签名零变化、一行委托传 `None`（默认预算即 MS28 目标行为变化，design D2）。锁先于头校验/WAL、非 `WouldBlock` 直传、`DatabaseLocked(path)` 文案、只读不创建缺失 `NotADatabase` 全部保持。
- **T2**（`src/database.rs`）：`open_internal` 增第 5 参 `lock_wait: Option<Duration>`；新 pub 核心 `Database::open_with_lock_wait(path, isolation, key, lock_wait)`；既有 open 家族（`open`/`open_with_isolation`/`open_with_key`/`open_readonly`/`open_readonly_with_key`/`open_in_memory`）委托传 `None`、签名零变化。新增私有 `construct_file_storage`：ReadWrite/ReadOnly 臂的 FileStorage 构造移入专用 `std::thread` + `tokio::sync::oneshot` 回传（`RecvError` → `StorageError::Internal("storage init failed: …")`），open future 在锁等待期间真实让出；`Memory` 臂内联不变；`assert_no_pending_wal` 仍在构造后 async 侧（次序不变）。
- **T3**（`src/cli/mod.rs`、`src/cli/lifecycle.rs`、`src/database.rs`）：`CliArgs.busy_timeout: Option<u64>`（`#[arg(long, value_name = "MS", global = true, env = "RTSQL_BUSY_TIMEOUT")]`——clap 完成 u64 校验与 flag>env 优先，非法值/负数/空 env 任意命令前 exit 2）。`execute_command` 单点解析 `lock_wait = args.busy_timeout.map(Duration::from_millis)` 并穿线 8 个生命周期子命令 + 主命令 + ATTACH 两点（`attach_database` 增参，经 `run_sql` 透传）。`execute_command_inner`/`execute_command_inner_mode` 增 `lock_wait` 参下发三 open 臂（memory 臂不消费）；只读臂经新增 `Database::open_readonly_with_lock_wait`（加性入口，见 Deviations 1）。`:memory:` 无守卫（接受并忽略）；`delete_db` 探测不穿线（行为不变）。
- **T4**（4 测试文件，D8 白名单）：`database_file_lock_test.rs` 6 用例经 `Some(0)`（新 `ZERO_WAIT` 常量）/`open_with_lock`/`open_readonly_with_lock` 校准；`readonly_session_test.rs` 2 lib 用例经 `open_readonly_with_lock_wait`/`open_with_lock_wait` `ZERO_WAIT` 校准 + 2 CLI 锁冲突块经 `run_cli_env(..., RTSQL_BUSY_TIMEOUT=0)` 校准；`attach_session_test.rs` 2 CLI 用例经 `run_cli_env` 校准；`cli_test.rs` 2 用例经既有 `run_cli_env` 校准。断言对象/文案/退出码零变化；`test_close_reopen_loop_no_false_locked`、`test_reopen_after_drop_succeeds`、delete 锁探测用例等不依赖等待时长者零校准。两个测试文件各增 env-capable `run_cli_env` helper（原 `run_cli` 委托）。
- **新测试**：`tests/busy_timeout_test.rs`（18 例：T1 FileStorage 臂 8 + T2 lib 臂 2 + T3 CLI 臂 8——锁矩阵、通道矩阵 flag/env/优先级/非法值、`:memory:` 忽略、超时 exit 4 文案、SIGINT 130、ATTACH 预算）；`src/cli/mod.rs` 单元测试 `phase1_signal_during_lock_wait_returns_signaled`（注入即时信号源）。

**Deviations from Plan**

1. **补加 `Database::open_readonly_with_lock_wait`（加性新入口）**：T3 契约要求 `lock_wait`「下发三 open 臂」，但 T2 只建立了 RW 通道 `open_with_lock_wait`，只读臂无预算可达路径。补加只读变体（既有 `open_readonly_with_key` 改为委托它传 `None`，签名零变化）是契约「三臂穿线 + readonly-open-mode R1 MODIFIED 共享锁同款等待」的必要完成，属 design D2「新增入口或加性参数，MS17 委托壳先例」范围内。
2. **T2 信号中断见证的测试位置**：契约示例写在 `busy_timeout_test.rs`，但 `execute_command_inner` 为模块私有、集成测试无法注入信号源，故该见证落在 `src/cli/mod.rs` 单元测试（既有 `phase2_signal_runs_close_and_returns_signaled` 同模式）；矩阵文件仍承载 lib+CLI 全矩阵。测试放置偏差，非实质。
3. **`execute_command_inner_mode` 8 参触发 clippy `too_many_arguments`**：按项目既有先例（`subquery_eval.rs`/`update.rs`/`upsert.rs`）加 `#[allow(clippy::too_many_arguments)]` + 理由注释，未做参数 bundling（编排面参数均为调用方直传独立通道，bundling 只引入间接层）。
4. **T4 校准伴随修正（测试文件内）**：`readonly_session_test.rs` 存在段中途 `use std::time::{Duration, Instant}` 与文件顶部新导入重复——顶部保留 `Duration`（`ZERO_WAIT` 所需），段内改为 `use std::time::Instant;`（消除 E0252，语义零变化）。
5. **测试夹具两处修正（非产品代码）**：(a) `busy_timeout_test.rs::hold_exclusive` 增 2ms×50 重试——并行负载下原始 `try_lock` 探针命中平台 flock 释放可见性窗口假 `WouldBlock`（产品 `LOCK_VISIBILITY_FLOOR` 吸收的同类平台行为），helper 只建立夹具、不构成断言弱化；(b) attach 用例补 `att_db.close().await`——lib 契约「drop 不 close + DDL 无 WAL」（MS18 design D7 持久化模型，`phase2` 测试注释同），夹具原写法在重开时 WAL redo 报 table not found，修正后断言语义不变。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Gate 3 逐任务 diff 审读完成：契约内文件/符号/调用点全处理；Forbidden 项（不改 MemoryStorage/delete 探测/信号分支/水位通道/锁互斥语义）核实未触碰；无身份型证据机制；无新增警告（clippy --all-targets 0 warning）。
- 实施中发现并已修复：clippy `needless_question_mark`（`construct_file_storage` 尾部改 `match rx.await`）；clippy `too_many_arguments`（见 Deviations 3）；`unused_mut` 1 处；E0252 重复导入（见 Deviations 4）；`cargo fmt` 一次整形（lifecycle.rs 两个签名换行）。
- 遗留 Minor：无。

**Verification Evidence**

- T1 RED：`cargo test --test busy_timeout_test` → E0599 ×8（`open_with_lock`/`open_readonly_with_lock` 缺失），exit 101。T1 GREEN：同命令 → **8 passed / 0 failed**，exit 0；`cargo test --test database_file_lock_test`（校准前跑通性确认）→ **8 passed / 0 failed**（5s 预算等待符合预期，5.01s），exit 0。
- T2 RED：同文件扩展后 → E0599 ×2（`open_with_lock_wait` 缺失），exit 101。T2 GREEN：`cargo test --test busy_timeout_test` → **10 passed / 0 failed**，exit 0；`cargo test --lib` → **354 passed / 0 failed**（含新 `phase1_signal_during_lock_wait_returns_signaled` 与未改动 `phase2_signal_runs_close_and_returns_signaled`），exit 0。
- T3 RED：CLI 臂 6 用例失败于「unexpected argument '--busy-timeout'」（exit 2 ≠ 期望 0/4），exit 101。T3 GREEN：`cargo test --test busy_timeout_test` → **18 passed / 0 failed**（0.56s），exit 0。
- T4 GREEN：`cargo test --test database_file_lock_test --test readonly_session_test --test attach_session_test --test cli_test` → **8 + 14 + 11 + 117 passed / 0 failed / 2 ignored**（0.24s + 1.19s + 3.14s + 26.61s），exit 0——database_file_lock 0.24s 证明校准生效（校准前 5.01s）。
- 静态检查：`cargo fmt --check` → clean，exit 0；`cargo clippy --all-targets` → 0 warning/error。
- 全量分批（M19：103 集成目标 × `--test-threads=1`，四批顺序 + lib/bins）：

  ```
  批1  passed=389 failed=0 ignored=2     批3  passed=348 failed=0 ignored=0
  批2  passed=273 failed=0 ignored=0     批4  passed=287 failed=0 ignored=0
  lib+bins passed=354 failed=0
  合计 1651 passed / 0 failed / 2 ignored
  ```

  计数核对：前 change 权威 1632 + 本 Cycle 净增 19（busy_timeout_test 18 + cli/mod.rs 单元 1；lib 353→354、集成 1279→1297 账实相符）= 1651，缺零批。
- `openspec validate 2026-10-01-ms28-cli-concurrency-checkpoint --strict` → `Change '2026-10-01-ms28-cli-concurrency-checkpoint' is valid`，exit 0。

**Persisted Evidence**

- None required（Plan 指定 mode = none；全部决定性输出 ≤ 20 行且已在本 Response 摘录，均可低成本复现）。

**Experience Candidates**

- Runbook 候选：None（无新的可重复高风险操作路径）。
- Issue 候选：None（夹具 flock 释放窗口与 lib「drop 不 close」语义均为既有已记录行为的测试写法事项，非产品缺陷；本 change 范围外未发现实质缺陷）。

**Remaining Issues**

- None。

## Plan Review

- Review Result: accepted

**Findings**

独立审读完整 diff（4 产品文件 + 5 测试文件 + 新测试文件）并逐项核对 Act 声明，结论：

1. **Deviation 1（补加 `Database::open_readonly_with_lock_wait`）＝ Plan 遗漏类，非阻塞**：T2 契约只建立了 RW 通道，而 T3 要求「三臂穿线」——只读臂在原契约下无可达预算路径。Act 补加的加性入口（既有 `open_readonly_with_key` 改委托它传 `None`，签名零变化，经 diff 核实）正是 readonly-open-mode R1 MODIFIED「共享锁同款等待」的必要完成，在 design D2「新增入口或加性参数」授权范围内。T2/T3 契约边界未被扩大。
2. **Deviations 2-5 ＝ Act 偏离，均非实质**（逐一 diff 核实）：信号中断见证落位 `src/cli/mod.rs` 单元测试（`execute_command_inner` 模块私有，集成测试无法注入信号源——合理且与既有 `phase2` 结构测试同模式）；`#[allow(clippy::too_many_arguments)]` 有先例与理由注释；E0252 段内导入修正语义零变化；`hold_exclusive` 夹具重试（2ms×50）是锁获取 helper 非断言弱化、attach 夹具显式 `close()` 符合 lib「drop 不 close + DDL 无 WAL」既有契约。
3. **Minor（不阻塞，记录不返工）**：(a) SIGTERM 143 无直接测试见证——SIGINT 130 已双层见证（子进程 + 单元），且阶段 1/2 信号分支经 diff 核实零改动、机制同构，SIGTERM 语义由不变量覆盖；(b) `execute_main_command` 与 `execute_command` 各自解析一次 `lock_wait`（重复无害）；(c) `WaitTimeout` 轮询 helper 为夹具层等待工具（60s 挂起判定），非产品行为判定层。
4. **Forbidden 与不变量核实**：`delete_db` 探测、水位 env 解析、`open_error_status`、`:memory:` 守卫面、信号分支、锁互斥矩阵、打开序（锁→头校验→WAL）、既有 open 家族签名——diff 内均未触碰。新测试无身份型证据机制、无断言弱化。

**Acceptance Gaps**

None——Acceptance 1-7 全部有测试见证（T1 8 例 / T2 2 例 + 单元 1 / T3 8 例 / T4 校准 12 例 + 零校准保持）。

**Evidence**

- 独立 diff 审读：`src/storage/file_storage.rs`（deadline 循环、`LOCK_VISIBILITY_FLOOR=8ms`/`DEFAULT_LOCK_WAIT=5000ms`、双核心、委托壳、打开序保持）；`src/database.rs`（`construct_file_storage` thread+oneshot、RecvError→Internal、Memory 臂内联、`assert_no_pending_wal` 次序、既有家族委托）；`src/cli/mod.rs`+`src/cli/lifecycle.rs`（clap attr 与 `--key` 先例同构、8 子命令+主命令+ATTACH 穿线、memory 臂不消费、`delete_db` 未动）；4 测试文件校准 diff（断言零变化、`test_close_reopen_loop_no_false_locked`/`test_reopen_after_drop_succeeds`/delete 探测用例零校准）；`tests/busy_timeout_test.rs` 18 例矩阵。
- 验证采信（用户 2026-10-01 裁定「测试可信」）：Act Response Verification Evidence——全量分批 **1651 passed / 0 failed / 2 ignored**（1632 + 净增 19，账实相符）；`database_file_lock_test` 0.24s（校准生效，校准前 5.01s）；`cargo fmt --check` clean、clippy 0 warning；`openspec validate --strict` valid。RED→GREEN 三阶段见证在案（E0599 ×8 / E0599 ×2 / CLI flag 缺失）。

**Follow-up Decision**

全部 Acceptance 满足，findings 均为非阻塞 Minor 或契约范围内的必要完成——`accepted`。Iteration 000 完成；按既有 Map 展开下一 Iteration（无 replan/rework 需要；tasks.md Iteration 000 段随本 Review 补记交付）。

**Iteration Plan Update**

None（Map 不变）。

**Next Cycle**

None（Iteration 000 无后继 Cycle）。

**Next Iteration**

`openspec/changes/2026-10-01-ms28-cli-concurrency-checkpoint/iterations/001-checkpoint-command/000-initial.md`（已展开，Status: ready）
