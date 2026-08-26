# Iteration 000 / Cycle 000: WAL 持久句柄复用

## Plan Context

- Status: ready
- Ready authorization: 2026-08-26 用户批准（原话："批准，更新gate状态，然后开始实施吧"）；Gate 2 Readiness 七维 PASS 于创建时已记录
- Iteration: 000-wal-handle
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1（WalWriter 单句柄改造）、T2（fd 上界与行为见证测试）
- Depends on: None
- Stable baseline: WalWriter 全部 5 个 IO 方法经单一句柄操作；10K tx 压测 fd 净增量 < 10 断言进 cargo test；错误/LSN 对外语义零变化；WAL 族回归全绿
- Verification boundary: `cargo test --test wal_handle_test` 全绿 + WAL 回归族全绿 + `cargo build` 无新警告
- Diagnostic boundary: `src/wal/writer.rs`、`tests/wal_handle_test.rs`（新增）；调用方文件只读
- Deferred tasks: T3, T4（Iteration 001-pipeline-stages）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部用户裁决（G1 全方法复用 / G2 /proc/self/fd 断言 / G3 错误不重试 / G5 LSN 文件位置语义）
- Excluded scope: pipeline.rs 一切改动（Iteration 001）；错误重试策略；LSN 内存计数器方案；writer task 化重构

**Objective**

`src/wal/writer.rs::WalWriter` 从"每方法逐次 open/close"改为持有单一个 `Arc<std::sync::Mutex<File>>` 持久句柄，5 个 IO 方法全部经该句柄完成操作；新增集成测试以 `/proc/self/fd` 断言证明 10K tx 压测下 fd 净增量 < 10，并固化 LSN 偏移语义、truncate 后追加语义与并发写一致性。对外方法签名、错误类型、磁盘格式零变化。

**Background**

MS06 稳定性收口剩余两项之一（tasks.md MS06-T03）。现状每条 WAL 写入都经历 open→write→close，fd churn 随写入量线性发生且无上界保证。用户 2026-08-26 Gate 1 批准范围扩展至全部 5 方法（超出任务书原列 3 方法行号）。

**Current Baseline**

- revision `f392c73eb0dbfe2e15902777d2574ef892475427`（2026-08-26，干净工作区）
- 新鲜验证：wal_writer_test 5 pass / wal_buffer_test 4 pass / checkpoint_test 3 pass / recovery_e2e_test 6 pass / dml_tx_id_test 6 pass / pipeline_test 17 pass，退出码 0
- 全库基线：504 tests pass（SNAPSHOT 记录）

**Current-State Evidence**

以下事实由 Plan 于 2026-08-26 直接读取源码确认（design.md §1.1 为同源详细版）：

1. `WalWriter`（src/wal/writer.rs L13-17）字段仅 `wal_path: PathBuf` + `write_count: AtomicU64` + `checkpoint_threshold: u64`，不持句柄
2. `open()`（L21-37）打开 create+append+read 后丢弃句柄
3. 五方法现状流程：
   - `write_record`（L40-68）：spawn_blocking 内 open(append) → seek(End(0)) → stream_position() 得 lsn → write_all(buf) → 返回 lsn
   - `fsync`（L71-86）：open(write) → sync_all
   - `truncate_to`（L89-105）：open(write) → set_len(lsn)
   - `get_current_lsn`（L129-147）：open(read) → metadata().len()
   - `write_batch`（L153-179）：open(append) → 逐条 serialize_with_lsn+write_all → sync_all
4. 调用方（grep 全仓确认）：`database.rs:34-37`（Arc 存入 Database.wal_writer）、`wal/buffer.rs`（get_current_lsn L163 + write_batch L175，do_flush 内）、`wal/checkpoint.rs`（get_current_lsn L85 + write_record L104）。**`truncate_to` 生产调用方为零**（仅 tests/wal_writer_test.rs:139）
5. WALBuffer 的 do_flush 在写批前用 get_current_lsn() 取 base_offset 再逐条累加序列化长度计算偏移 LSN（buffer.rs L162-171）；持久层权威 LSN = 文件字节偏移
6. 并发面：CheckpointManager.write_record 与 WALBuffer.write_batch 可并发执行（各自开 fd），存在 Checkpoint 记录插入他人已算 base_offset 之后的交错窗口——本 Cycle 单锁串行化自然消除该窗口（记录为正面效应，非独立验收项）
7. 测试直接构造点（不得破坏）：tests/executor_test.rs 5× `WalWriter::open(":memory:")`；checkpoint_test/recovery_test/wal_buffer_test 各自 open；benches/wal_group_commit_bench.rs
8. 现有行为见证：tests/wal_writer_test.rs 5 测试含 test_truncate_wal、test_fsync_after_write——本 Cycle 完成后必须零修改通过

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/wal/writer.rs` | `WalWriter` 及其全部方法 | 本 Cycle 唯一产品代码修改点 |
| `src/wal/buffer.rs` | `WALBuffer::do_flush` | 只读：调用方语义参照 |
| `src/wal/checkpoint.rs` | `CheckpointManager::checkpoint` | 只读：调用方语义参照 |
| `tests/wal_writer_test.rs` | 5 个既有测试 | 回归见证，禁止修改 |
| `tests/wal_handle_test.rs` | 新增 | T2 交付物 |

**Critical Path**

Database::open → WalWriter::open（持句柄起点）→ DML 执行路径 TransactionManager commit → WALBuffer.append/append_commit_and_wait → flush_loop 或容量触发 do_flush → write_batch（锁内批量写+fsync）→ Checkpoint 路径 write_record/get_current_lsn 同锁串行。

**Implementation Guidance**

- 目标结构（design §2）：

```rust
pub struct WalWriter {
    file: Arc<std::sync::Mutex<std::fs::File>>, // create+append+read 打开一次
    wal_path: PathBuf,                          // 保留诊断用途
    write_count: AtomicU64,
    checkpoint_threshold: u64,
}
```

- 每方法模式：clone `self.file.clone()` → `spawn_blocking(move || { let mut f = lock; ... })`；锁跨度 = 单次 IO 操作
- `write_record` 锁内顺序保持 seek(End(0)) → stream_position() → write_all：返回值 = 写前末尾偏移（G5 语义）
- `truncate_to` 后无需 seek：O_APPEND 保证下次 append 写在新末尾
- `get_current_lsn` 持锁读 metadata()：避免读到 truncate 中间态
- std::sync::Mutex 选择理由：spawn_blocking 内纯同步短临界区；tokio Mutex 无增益（design §2 已论证）
- Mutex 中毒处理 `.lock().unwrap()`：与库内既有 unwrap 风格一致；中毒仅在持锁 panic 时发生

**Behavioral Change**

- 当前行为：每次 IO 操作 open 新 fd 用后即弃；fd 数随并发调用瞬时波动；不同调用方可真实并发写 fd（存在交错窗口）
- 目标行为：进程生命周期内 WAL 写路径恒定 1 个持久 fd（+测试期临时资源）；全部操作经单锁串行；可观察的对外契约（返回值、错误类型、磁盘字节）不变
- 接口变化：无公开签名变化。内部字段增加
- 状态变化：WalWriter 从无状态 IO 门面变为持资源对象——`drop(WalWriter)` 时句柄随之关闭（与现状"最后一次 open 的句柄在方法返回时关闭"等价收口）

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/S1-S5, R3/S3, R4/S7 | `src/wal/writer.rs::WalWriter{open,write_record,fsync,truncate_to,get_current_lsn,write_batch}` | 逐次 open/close 的 IO 门面 | 持 `Arc<Mutex<File>>` 单句柄；5 方法改锁内操作 |
| T2 | R2/S6(fd断言), R1 固化 | `tests/wal_handle_test.rs`（新增） | 无 | 4 个集成测试：fd 上界 / LSN 偏移 / truncate 追加 / 并发一致 |

**Task Contracts**

### T1: WalWriter 单句柄改造

- Requirement/Scenario: R1（持久句柄复用 S1-S5）、R3（错误语义 S3）、R4（LSN 文件位置 S7）
- Depends on: None
- Targets: `src/wal/writer.rs` 全部 impl 块
- Current behavior: 见 Current-State Evidence 第 3 条逐方法流程
- Required behavior: 五方法经共享单句柄完成同等操作；对外返回值与错误逐一等价
- Required changes:
  1. 结构体加 `file: Arc<std::sync::Mutex<std::fs::File>>`；open() 保留句柄
  2. 五方法删除 OpenOptions 逐次 open，改为 clone Arc + spawn_blocking + lock 内操作
  3. write_record 保持 seek(End)+stream_position 取 LSN；write_batch 保持逐条 serialize_with_lsn+write_all+sync_all；get_current_lsn 持锁读 metadata().len()
- Preserve:
  - 公开方法签名（名称/参数/&self async 形态）不变
  - 错误映射：所有底层 io::Error → `WalError::IoError(e.to_string())`
  - `write_count`/`checkpoint_threshold` 计数逻辑不变
  - spawn_blocking 包裹（不得把阻塞 IO 直接放进 async fn）
- Forbidden:
  - 修改 src/wal/buffer.rs、src/wal/checkpoint.rs、src/database.rs、src/wal/reader.rs、src/wal/recovery.rs
  - 引入重试/重开逻辑
  - 改动 record 序列化或任何 on-disk 格式
  - 新增依赖
- Test witness: RED 阶段 = T2.5 在未改造代码上运行 wal_handle_test 观察并记录现状 fd 行为对照；GREEN 阶段 = 改造后全部通过
- GREEN condition: `cargo test --test wal_handle_test --test wal_writer_test --test wal_buffer_test --test checkpoint_test --test recovery_e2e_test` 全绿
- Verification: `cargo build` 无警告；上述测试命令退出码 0
- Stop when: 发现某调用方依赖"每方法独立 fd"语义（如需并行 fsync 与 append）；或锁内操作需要 await；或现有回归测试出现与本改动无关的既有失败

### T2: fd 上界与行为见证测试

- Requirement/Scenario: R2（S6 fd 上界断言）、R1 场景固化（S1 顺序、S2 并发、S4 truncate 追加、S5 截断后 LSN）
- Depends on: T1（GREEN 验证依赖；RED 对照可在 T1 前运行）
- Targets: `tests/wal_handle_test.rs`（新建）
- Current behavior: 无此测试文件；fd 行为无自动断言
- Required behavior: 四个测试按下列契约通过
- Required changes:
  1. `test_fd_bound_under_10k_tx`：tempdir → `Database::open` → CREATE TABLE → 循环 10_000 次 `execute_sql("INSERT ...")`（值各不相同避免冲突）→ 期间与结束时 `std::fs::read_dir("/proc/self/fd")?.count()` 相对压测前采样净增量 `< 10` 断言。压测前采样点在 Database::open 之后（排除打开瞬间的固有 fd）
  2. `test_write_record_lsn_equals_file_offset`：直接构造 WalWriter::open(temp wal path)，顺序 write_record ≥3 条；每条返回 lsn == 该次写入前文件 metadata().len()；lsn 严格递增；首条 == 0
  3. `test_truncate_then_append_same_handle`:写 ≥3 条 → truncate_to(第 2 条边界) → get_current_lsn() == 截断长度 → 再 write_record → 新 lsn == 截断长度 且读取验证记录完整落在截断点之后
  4. `test_concurrent_writers_recovery_consistent`：≥4 tokio 任务并发共享 Arc<WalWriter> 各 write_record 多条 → 全部 join → drop writer → RecoveryManager（或 WalReader 逐条解析）读回总数 == 写入总数且无解析错误
- Preserve: 既有测试文件零修改；测试用 tempfile crate（dev-dependencies 已有）
- Forbidden: 断言数值阈值变更（< 10 为验收口径）；为凑速度缩减 10K 规模；引入 lsof 外部命令依赖
- Test witness: 本任务交付物即测试本身；RED 对照见 T1 契约
- GREEN condition: `cargo test --test wal_handle_test` 4 passed 0 failed
- Verification: 测试输出计数证据写入 Act Response（fd 净增量的实际数字）
- Stop when: 10K 压测单测耗时 >60s（报告实际值并等待 Plan 决定，不得擅自降额）；或发现 /proc/self/fd 在目标环境不可读

**Invariants**

- WAL on-disk 格式与 RecoveryManager 读路径零接触
- `Database::open`/`execute_sql` 公开 API 不变
- Arc<WalWriter> 共享形态与 Send+Sync 性质保持
- 现有测试文件零修改通过（wal_writer/wal_buffer/checkpoint/recovery/recovery_e2e/executor 六族）

**Non-goals**

- Iteration 001 的 pipeline 工作（T3/T4）
- 错误重试、LSN 语义改造、writer task 化、性能优化（MS06 non-goal）

**Acceptance**

| # | 可观察条件 | 映射 |
|---|---|---|
| A1 | 5 方法不再逐次 open；单句柄贯穿（代码审查 + grep 无残留 OpenOptions 于方法体内） | R1/T1/design§2 |
| A2 | `test_fd_bound_under_10k_tx` 通过且 Response 记录实际净增量数字 | R2/S6/T2 |
| A3 | LSN 偏移语义断言通过（首条==0、逐条递增、等于写前长度） | R4/S7/T2.2 |
| A4 | truncate 后同句柄追加位置与 get_current_lsn 正确 | R1/S4,S5/T2.3 |
| A5 | 并发写后恢复解析完整无错 | R1/S2/T2.4 |
| A6 | WAL 回归六族全绿数量与基线一致 | design§2 测试见证 |

**Verification**

```bash
# V-000-1 构建无警告
cargo build 2>&1 | tail -3          # 期望无 warning；退出码 0
# V-000-2 新增测试
cargo test --test wal_handle_test   # 期望 4 passed; 0 failed
# V-000-3 回归族
cargo test --test wal_writer_test --test wal_buffer_test --test checkpoint_test \
  --test recovery_test --test recovery_e2e_test   # 期望 5+4+3+N+N passed; 0 failed
```

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | design.md §1.1 全文级源码事实 + 调用方 grep 清单 + 新鲜基线 6 族测试绿（2026-08-26 @ f392c73） |
| Design | PASS | design §2 结构/选型/替代方案/兼容性闭合；无 TBD |
| Iteration Plan | PASS | tasks.md Iteration Plan 两 Iteration + 平衡审计四条结论 |
| Cycle Scope | PASS | initial 范围 = T1+T2；Excluded scope 明确 |
| Task Contracts | PASS | T1/T2 含 targets/current/required/preserve/forbidden/witness/green/stop 全字段；Act 只读本 Cycle 即可开工 |
| Traceability | PASS | tasks.md RTM 8 行全覆盖，0 Missing |
| Verification | PASS | V-000-1/2/3 命令与通过条件明确 |

**Persisted Evidence**

- Mode: none

（Act Response 记录命令、≤20 行决定性输出、退出码即可。fd 净增量实际数字属 Act Response 内容，不要求单独 Evidence 文件。）

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个；单个文本 ≤500 行 ≤256 KiB

**Risks and Notes**

- fd 上界测试时长未知（预估秒级~十秒级）：>60s 时按 T2 stop 条件上报，不得擅自降额
- 单锁串行化对吞吐的影响不在本轮度量范围（性能优化是 MS06 non-goal）；如 bench 出现显著回退，Act Response 记录数据交 Plan 决定
- print_timings/stderr 类问题与本 Cycle 无关

## Act Response

- Status: reported

**Implemented**

- T1：`WalWriter` 改造为持有 `Arc<std::sync::Mutex<std::fs::File>>` 单一持久句柄。`open()` 以 create+append+read 打开一次并保留句柄；`write_record`/`fsync`/`truncate_to`/`get_current_lsn`/`write_batch` 五方法全部删除逐次 `OpenOptions::open`，改为 clone Arc → spawn_blocking → lock 内完成同等操作。锁跨度为单次 IO 操作（write_batch 为整批+fsync，与 design §2 Critical Path 一致）。
- T2：新增 `tests/wal_handle_test.rs` 四测试——10K tx fd 上界断言（/proc/self/fd 净增量 <10）、LSN==写前文件偏移（首条 0、严格递增）、truncate 后同句柄追加位置与 get_current_lsn 正确、4 任务并发共享 Arc\<WalWriter\> 写入后恢复解析完整。
- RED 对照（T2.5）：改造前 @ f392c73 运行 wal_handle_test，fd 测试偶然通过（before=8 after=8 delta=0；顺序负载下逐次 open/close 瞬时采样不可见 churn），按契约记录对照数字不阻塞；LSN/truncate/concurrent 三测试现状即绿（语义本就保持）。改造后全绿。

**Changed Files and Symbols**

- `src/wal/writer.rs` — `WalWriter` 结构体（新增 `file: Arc<Mutex<File>>` 字段，`wal_path` 保留并加 `#[allow(dead_code)]`）；`open`、`write_record`、`fsync`、`truncate_to`、`get_current_lsn`、`write_batch` 方法体
- `tests/wal_handle_test.rs` — 新增文件（4 集成测试 + mk_insert/assert_ok/count_fds 辅助）
- `iterations/000-wal-handle/000-initial.md` — Plan Context draft→ready（用户批准）+ 本 Act Response
- change `tasks.md` — T1/T2 共 16 个复选框置完成

**Deviations from Plan**

1. fd 测试追加 drop 后采样观察（Database 无公开 close()，README 所述 close API 不存在）：drop 前后 fd 9→8，验证句柄随对象释放。仅追加观察输出，不改契约断言口径（<10 断言、采样点在 open 之后均按契约执行）。
2. 见证阶段发现并修正测试自身缺陷一处：首轮 LSN 测试的"首条 == 0"断言误用循环结束后的 prev_lsn（持末条 LSN 89）。属新写测试代码缺陷，非产品缺陷；修正后逐条偏移断言全通过。

**Blocker Handoff**

None

**Self-Review**

- Plan compliance: T1.1-T1.9、T2.1-T2.7 全部完成；Forbidden 清单零触犯（buffer/checkpoint/database/reader/recovery 未修改、无重试逻辑、on-disk 格式零接触、无新增依赖、公开签名不变）
- Full diff reviewed: 是 —— writer.rs 全量 diff 逐块核对（imports/struct/open/五方法），新测试文件逐行核对；无计划外修改
- Critical findings unresolved: 无
- Important findings unresolved: 无
- Minor findings unresolved: ① `wal_path` 字段以 `#[allow(dead_code)]` 保留（计划明示保留用于诊断，附注释说明，非遗留缺陷）；② 范围外观察：README 的 `db.close()` 示例与实际 API 不符（文档漂移，不在本 Cycle 范围）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| 构建 | `cargo build` | exit 0；产品代码 0 warning（仅环境级 cargo config 弃用提示） | PASS |
| 新增测试 | `cargo test --test wal_handle_test` | `test result: ok. 4 passed; 0 failed`（14.50s） | PASS |
| fd 实测（A2 数字） | 同上 `--nocapture test_fd_bound_under_10k_tx` | `fd bound: before=9 after=9 delta=0`；drop 后 `9→8` | PASS |
| WAL 回归族 | `cargo test --test wal_writer_test --test wal_buffer_test --test checkpoint_test --test recovery_test --test recovery_e2e_test --test executor_test` | 5+4+3+3+6+39 = **60 passed; 0 failed**（数量与基线一致） | PASS |
| A1 残留检查 | `grep -n OpenOptions src/wal/writer.rs` | 仅 L6 import 与 L30 open() 内，方法体零残留 | PASS |
| OpenSpec | `openspec validate 2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages` | `Change ... is valid` | PASS |

Acceptance 对照：A1 ✅（grep 证据）；A2 ✅（delta=0 <10，实际数字已录）；A3 ✅（首条 0/递增/等于写前长度）；A4 ✅（truncate 后 LSN 与追加位置正确）；A5 ✅（4×25 并发写读回 100 条无解析错误）；A6 ✅（六族回归全绿数量一致）。

**Persisted Evidence**

None required（Mode: none；fd 净增数字与命令输出已按计划记入本 Response）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| （无） | None | — | 本次为常规行为保持型重构，无可重复高风险操作路径或故障现场 |

**Remaining Issues**

- 无 Critical/Important 遗留。
- 范围外备忘：README 快速开始示例引用不存在的 `Database::close()`（文档漂移，建议后续 docs 收口时处理）。

**Commit or Diff Reference**

未 commit（待用户触发）。diff 基准：工作区 vs `f392c73eb0dbfe2e15902777d2574ef892475427`，涉及 `src/wal/writer.rs` + `tests/wal_handle_test.rs`。

## Plan Review

- Review Result: accepted

**Findings**

独立审计（2026-08-26，Review 直接读取代码/diff 并新鲜重跑验证，不以 Act Self-Review 代替）：

- A1 ✅ 单句柄贯穿：`grep -n OpenOptions src/wal/writer.rs` 仅 L6 import 与 L30 open()，方法体零残留；结构体持 `file: Arc<Mutex<File>>`（L20），`wal_path` 以 `#[allow(dead_code)]` 保留（计划 T1.8 明示，非遗留缺陷）；五方法全部 clone Arc → spawn_blocking → lock 内完成同等操作，无重试/重开逻辑；`write_record` 保持 seek(End(0)) → stream_position → write_all（G5 语义）；`get_current_lsn` 持锁读 metadata().len()
- A2 ✅ fd 上界：新鲜运行 `cargo test --test wal_handle_test -- --nocapture` = **4 passed / 0 failed**（15.86s）；实测 `before=13 after=9 delta=-4 < 10`（与 Act 记录的 delta=0 不同属环境采样波动，负值源于运行期间临时资源释放，断言口径满足）；drop 后 9→8 验证句柄随对象释放
- A3-A5 ✅ 行为见证：LSN==写前文件偏移、首条 0、严格递增；truncate_to(lsns[2]) 后 get_current_lsn 正确且追加落点==截断点、读回恰 3 条完整记录；4 任务×25 条并发共享 Arc\<WalWriter\> 写入后读回 100 条无解析错误——逐条对照 Task Contract 契约通过
- A6 ✅ 回归六族：新鲜运行 5+4+3+3+6+39 = **60 passed / 0 failed**，数量与基线一致；executor_test 5 处 `WalWriter::open(":memory:")` setup 零修改通过
- Forbidden 零触犯：`git diff --name-only f392c73 -- src/wal/buffer.rs src/wal/checkpoint.rs src/database.rs src/wal/reader.rs src/wal/recovery.rs` 输出为空；公开方法签名不变；record 序列化/on-disk 格式零接触；无新增依赖
- `openspec validate` 新鲜运行 PASS；`cargo build` exit 0 无产品代码 warning
- Act Self-Review 与独立检查结论一致，无 Self-Review 未覆盖的新发现

Minor findings（均不阻塞 Acceptance）：

- M1：fd 测试追加 drop 后采样观察输出（见 Deviation Classification #1）
- M2：README 快速开始示例引用不存在的 `Database::close()`（文档漂移）——范围外备忘，与本 Iteration Acceptance 无关，留后续 docs 收口处理

**Deviation Classification**

| # | 偏差 | 分类 | 实质性判定 | 处理 |
|---|---|---|---|---|
| 1 | fd 测试在契约断言之外追加 drop 后采样观察（println 输出 before_drop/after_drop） | ACT-DEVIATION | 非实质——仅追加观察输出；契约断言口径（delta < 10）与采样点（open 之后）均按原契约执行；已在 Act Response 如实记录 | 记录并接受 |
| 2 | 见证阶段发现并修正自写测试缺陷一处（首轮 LSN 测试"首条 == 0"断言误用循环结束后的 prev_lsn） | ACT-DEVIATION | 非实质——属新写交付物自身的作者缺陷，GREEN 交付版正确且经独立重跑验证；非产品代码缺陷 | 记录并接受 |

无 PLAN-OMISSION / PLAN-INVALID / BASELINE-CHANGED / NEW-EVIDENCE。

**Acceptance Gaps**

None —— A1-A6 全部满足且有新鲜证据。

**Convergence**

N/A

**Evidence**

Review 新鲜验证记录（2026-08-26 @ 工作区，diff 基准 f392c73eb0dbfe2e15902777d2574ef892475427）：

| 验证项 | 命令 | 决定性输出 | 结论 |
|---|---|---|---|
| 构建 | `cargo build` | exit 0；grep warning/error 仅环境级 cargo config 弃用提示 | PASS |
| 新增测试 | `cargo test --test wal_handle_test -- --nocapture` | `4 passed; 0 failed`；`fd bound: before=13 after=9 delta=-4`；drop 后 `9→8` | PASS |
| 回归六族 | `cargo test --test wal_writer_test --test wal_buffer_test --test checkpoint_test --test recovery_test --test recovery_e2e_test --test executor_test` | 3+39+6+3+4+5 = **60 passed; 0 failed**（各二进制计数与基线一致） | PASS |
| A1 残留 | `grep -n OpenOptions src/wal/writer.rs` | 仅 L6/L30 两处 | PASS |
| Forbidden | `git diff --name-only f392c73 -- <五个调用方文件>` | 空（0 行） | PASS |
| OpenSpec | `openspec validate 2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages` | Change ... is valid | PASS |

**Follow-up Decision**

按 iteration-planning.md 判定问题："该工作是否是达到当前 Iteration 原有 Acceptance 的必要条件？"——两项偏差与 M1/M2 答案均为否，不构成返工理由。Acceptance 已全部满足且无阻塞项 → `accepted`。M2（README close() 漂移）按职责归属 docs 维护路径处理，不得以返工名义扩大本 Cycle。

**Iteration Plan Update**

None —— Map 不变，T3/T4 归属 Iteration 001 维持原计划。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages/iterations/001-pipeline-stages/000-initial.md`（已于本次 Review 创建展开；Plan Context Status: draft，Gate 2 七维评估 PASS，待用户批准计划后转 ready 交 openspec-act 执行）
