# proposal: MS10-T02 跨进程文件锁 + 优雅停机

> **Replan（2026-09-06，Gate 2 前审计）**：计划审计 + 本机实验发现两个阻塞项，经实证后修订范围：(1) **引擎 P0——`WalReader` 格式嗅探缺陷**：WAL 含 ≥~19 条记录（约 2KB）后重开恢复必失败（`Incomplete WAL record`，库无法打开；严格帧遍历证明 WAL 本身完好，嗅探误判 79/2046 帧）；本 change 的「kill 后 WAL 恢复 e2e」验收以其修复为前提，纳入 T0。(2) **T3 执行阶段 e2e 不可构造**：扫描/排序/多行 INSERT 均为毫秒级（无确定性秒级语句），见证重构为「open 阶段真实二进制 e2e + 库级编排结构测试」组合。Iteration 拆为 000-wal-recovery-fix → 001-lock-shutdown。

> **Plan 并入（2026-09-06，Gate 2 前，本轮）**：工作区存在 replan 前被中断的未提交实现（原 Iteration 000-lock-shutdown draft Cycle 遗留，无 Act Response 记录）：T2/T3 生产代码与测试、T4 全部 e2e 脚手架与标定常量（生产接线缺位）、Cargo signal/libc 接线、4 个既有测试文件的独占锁夹具适配。本轮 Plan 盘点实测后并入：① 既有测试标定列为 T2 必要后果（断言语义零修改，design D6）；② T2/T3 的 RED 见证改为「临时摘除对应 hunk 复现」；③ T4 测试先行的天然 RED（当前 cli_test 4 个失败均实测为 T0 后果）；④ Iteration 000 验证门对 3 个 Iteration-001 信号 RED 见证设白名单（tasks T1）。存量实现视为未验证，由 Iteration 001 Act 按契约复核后收编。

## Why

MS10-T01 落地 one-shot CLI 后，并发与生命周期两个正确性缺口成为"CLI 全链路可用"的短路点：

1. **无跨进程互斥**：两个进程（或同进程两实例）同时打开同一 db 文件 = 两个独立 BufferPool 写同一组页，结构上必然损坏（R18 主题 5 结构推断）。`FileStorage` 仅持进程内 `Mutex`（free_pages），无任何 flock/fcntl。
2. **无优雅停机**：全仓无信号处理代码；Ctrl-C 直接杀进程，`Database::close()`（checkpoint + WAL 截断）不触发——数据安全由 WAL redo 兜底，但已提交数据滞留 WAL，重开 redo 变多，且用户无法区分"被打断"与"正常完成"。
3. **（审计新增）WAL 恢复在真实数据量下必失败**：`WalReader::read_next_with_lsn` 以「byte[8] 是合法 type 且 byte[0] 不是」嗅探新旧格式；新格式帧首字节是内嵌 LSN 的 LSB，当记录的文件偏移低字节 ∈ 0x01-0x09（合法 type 值，约 3.5% 记录）时误判为旧格式，从 LSN 高位字节读出垃圾长度，解析 derail 直至 `Incomplete WAL record`——`Database::open` 失败。首个碰撞点在 ~第 19 条记录（offset 1033）；既有测试全部活在 <1KB WAL 的盲区。实验实证：2000 行库的 WAL 严格帧遍历 2046 条完好（lsn==偏移、CRC 全对），同一文件被嗅探误判 79 帧。任何真实使用（≥20 条未 checkpoint 记录后重开）都触发；本 change 的 kill/信号恢复 e2e 验收以其修复为前提。

tasks.md 将锁与优雅停机定位为正确性前置而非增强（MS10-T02）。T01 的退出码枚举已为 4（锁冲突）留位无产生路径。

**用户决策（2026-09-06，本会话）**：

1. 信号退出码：**POSIX 惯例 130/143**（SIGINT→130、SIGTERM→143），脚本可区分"被中断"与"真错误"。
2. flock 不被文件系统支持（WouldBlock 以外错误）：**严格失败**（exit 1），不降级无锁运行——宁可拒开也不冒双写损坏风险。

## What Changes

- **（Replan 新增）WAL 恢复逐帧无歧义解析（改 `src/wal/reader.rs`）**：消除格式嗅探歧义——歧义偏移（byte[0] 为合法 type 值）上先按新格式解析并以 CRC 验证为准，CRC/结构失败再按旧格式解析（含回退时 seek 回帧首与 EOF 读失败的回退语义）；两路皆败才显式报错（K05 语义不变）。混合格式流（新格式记录 + 旧格式 Checkpoint 记录，`checkpoint.rs:118` 经 `write_record` 写旧格式）兼容保持。
- **打开即独占锁（改 `src/storage/file_storage.rs`）**：`FileStorage::open` 对主文件 fd 以 `std::fs::File::try_lock`（stable 1.89，本机 rustc 1.90 实证）获取 advisory 独占锁，非阻塞；先于 WAL 打开与崩溃恢复。冲突 → 新 `StorageError::DatabaseLocked`；锁调用其他 IO 错误 → 普通失败（严格，用户决策 2）。零新依赖（std 原生），flock 随 fd/进程退出自动释放（kill -9 无死锁残留）。
- **错误类型（改 `src/storage/error.rs`）**：新增 `StorageError::DatabaseLocked(String)`（携带路径），additive 变体。
- **CLI 锁冲突接线（改 `src/cli/mod.rs`）**：`Database::open` 错误 match `DatabaseLocked` → `ExitStatus::Locked("database is locked: <path>")` → 退出码 4（留位落地）；其余 open 错误维持 General(1)。
- **优雅停机（改 `src/cli/mod.rs` + `Cargo.toml`）**：启用 tokio `signal` feature（现有依赖的 feature，非新依赖）；`execute_command` 两阶段（open / 执行+close）`tokio::select!` 信号臂：信号到达 → 中止当前阶段 → 已打开则 `close()` → `ExitStatus::Signaled(signum)` → 退出码 128+signum（130/143）。打开阶段被信号中止无需 close（WAL 未截断，恢复可完整重放；flock 随退出释放）。
- **退出码枚举**：`ExitStatus` 新增 `Signaled(i32)` 变体（信号中断非错误类，不占用 0-5 分类位）。

## Out of Scope（本 change 不做）

- 文件 magic/格式版本头（T03）、多语句 `;` 分片（T04）、生命周期子命令（T05）。
- 锁等待 / busy timeout（tasks.md 契约为占用立即报错，非阻塞等待）。
- WAL / `.checkpoint` 伴生文件的独立锁——主文件锁在 `FileStorage::open` 先于一切伴生文件 I/O 获取，第二进程被拒时不会触碰它们。
- Windows 及非 Linux 平台适配（`FileExt` 限 Unix，MS13 明确 non-goal）。
- network server 路径改动——server 经 `Database::open` 自动获得锁保护，代码零改动。
- REPL、密钥（MS12）、lib API 签名变化。
- **（Replan 明确排除）`WALBuffer::do_flush` 三入口无互斥**（并发时内嵌 LSN 可能重复/错位，影响 checkpoint 截断语义；帧完整性由 writer `Arc<Mutex<File>>` + O_APPEND 保证，本会话实验未触发帧损坏）——独立 improvement 候选，不并入本 change。多行 INSERT 万行级 `Page full`（含 abort 路径失败）同列为观察项。

## Impact

- **修改**：`src/wal/reader.rs`（逐帧无歧义解析，约 +30 行）、`src/storage/file_storage.rs`（open 内 try_lock，约 +15 行）、`src/storage/error.rs`（+1 变体）、`src/cli/mod.rs`（锁错误映射 + 两阶段 select 信号接线）、`Cargo.toml`（tokio features +`"signal"`）、既有测试 4 文件夹具锁适配（`tests/{btree,storage,schema_persistence,drop_table_free}_test.rs`，断言语义零修改，见 design D6）。
- **新增**：WAL 恢复回归测试（大 WAL 恢复，≥19 条记录，待建）、lib 锁测试（`tests/database_file_lock_test.rs`，工作区已就位）、CLI 集成测试增补（`tests/cli_test.rs`：锁冲突 exit 4、信号停机 130/143、kill -9 恢复 e2e——锁与信号用例工作区已就位；`src/cli/mod.rs` phase-2 信号结构测试待建）。
- **测试依赖**：dev-dependencies +`libc = "0.2"`（测试进程向子进程发 SIGINT/SIGTERM/SIGKILL 需要 `kill(2)`；libc 已由 tokio 传递引入，lockfile 零新包，产品 `[dependencies]` 零变化）。
- **行为变化**：① 打开已被占用的库 → 报错退出 4（此前：静默双写损坏风险）；② 同进程双开同一文件 → `DatabaseLocked`（新增保护，此前静默双实例）；③ SIGINT/SIGTERM → 优雅停机 130/143（此前：默认终止，无 checkpoint）；④ **（修复）含 ≥19 条记录的 WAL 恢复成功**（此前：`Database::open` 失败）；⑤ 既有测试断言语义与正常路径行为零变化（4 个既有测试文件的夹具按独占锁适配、断言不变，见 design D6）。
- **兼容性**：`Database::open` 签名不变；新错误变体为 additive（已核查无穷尽 match `StorageError` 的消费方）；`Database::open`/`FileStorage::open` 调用点（生产 1 处 + 测试 13 处）：生产唯一调用点为顺序单实例模式不受影响；测试 13 处中 4 处存在「第一持有者存活期间二次打开」的探针模式，已按独占锁适配夹具（断言语义不变，design D6），其余不受影响——以全量回归守门；旧格式 WAL（Checkpoint 记录流）兼容由既有 checkpoint 回归测试守护。
- **风险**：
  - 信号停机 e2e 依赖"恢复耗时 > 信号延迟"的时序余量——用大 WAL 标定（Act 以本机实测恢复耗时为准调整行数），余量不足时测试波动（design D5 给标定方法）。
  - 信号处理安装前的 SIGINT（进程启动瞬间）仍按默认终止——与所有 CLI 相同的固有窗口，不处理。
  - close() 期间二次 Ctrl-C 不再强杀（tokio signal 驱动注册后进程级持久）——kill -9 兜底，记录为已知行为。
