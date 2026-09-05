# Iteration 001 / Cycle 000: MS07-T05 Checkpoint（恢复消费位点 + WAL 重写截断 + 静默吞错显式化）

## Plan Context

- Status: ready
- Iteration: 001-checkpoint
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: Iteration 001 的 T1, T2, T3, T4（`tasks.md` §Iteration 001）
- Depends on: None（与 Iteration 000 无代码耦合；workspace 现状含未提交的 T04 改动，见 Current Baseline）
- Stable baseline: 重启后 `redo_count` 随 checkpoint 收敛；WAL 文件有界（checkpoint 后重写截断）；恢复路径静默吞错（K05）显式化为 `Database::open` 可见错误；无 checkpoint 位点时仍从头全量恢复
- Verification boundary: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff；`cargo test --all` 0 failures（≥553）；新增 `tests/checkpoint_redo_reduction_test.rs` 全绿；既有 recovery/wal/drop_table_free/e2e 测试全绿
- Diagnostic boundary: `src/wal/{checkpoint,recovery,reader,writer}.rs`、`src/database.rs`
- Deferred tasks: Iteration 002（T06 下推）；T07 视本 Iteration 暴露的并发协调点另开 change

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R2 全部场景（S2.1–S2.4）；design.md Iteration 001 决策 1–5
- Excluded scope: T06/T07；隔离级别/快照语义；WAL 记录序列化格式变更；WALBuffer/flush actor 改动；写循环阈值自动触发（`should_checkpoint` 接线不做，触发入口为 `Database::checkpoint()` 与 `close()`）；位点文件 16B 格式变更

**Objective**

Checkpoint 真正工作：`full_recover` 消费 checkpoint 位点（有效位点 → 只重放位点之后的记录；无效/缺失 → 安全退化全量重放）；`CheckpointManager::checkpoint()` 在刷脏页后重写截断 WAL 使文件有界；恢复路径的静默吞错（K05 五处）改为显式错误并传播到 `Database::open`；`Database::open` 接线 `CheckpointManager`，`close()` 自动触发 checkpoint。

**Background**

- proposal Why-T05：`CheckpointManager::checkpoint()` 只做"取 LSN → 刷脏页 → 写位点 → 写 Checkpoint 记录 → reset_write_count"，恢复端 `full_recover` 从 WAL 头全量重放、从不调用 `read_checkpoint_site`——位点写而不用，redo 量与 checkpoint 无关，WAL 无界增长。
- K05：recovery 在表缺失/IO 失败时静默跳过（导致数据丢失不可见）。MS07-T01 已交付 schema 持久化，redo 所需表定义已可持久获得，表缺失即真实异常，应显式报错。
- 用户决策（proposal）：T05 恢复消费位点 + 引入 checkpoint 截断 WAL 的决策；T07（消息传递重构）不纳入本 change。

**Current Baseline**

- Revision: `dc662d4`（HEAD）+ Iteration 000 未提交工作区（T04：transaction/database/pipeline/executors/explicit_tx_test；553 tests pass）。本 Iteration 触碰 `src/database.rs`，与 T04 改动同文件但区域不重叠（T04 新增事务 API 方法，T05 改 `open`/`close` 与字段）；逐 Iteration 独立提交的回退方案下，本 Iteration 提交应包含或晚于 T04 提交，由用户在 commit 时编排。
- 测试基线：553 tests pass（2026-09-05 本会话独立复跑确认）。
- 现状：checkpoint 位点写而不用；recovery 全量重放 + 五处静默吞错；`Database::open` 未接线 `CheckpointManager`。

**Current-State Evidence**

- `src/wal/checkpoint.rs:83-110` `checkpoint()`：`get_current_lsn` → `buffer_pool.flush_all` → `write_checkpoint_site(lsn, ts)` → 写 `WalRecord::Checkpoint { lsn, timestamp }` → `reset_write_count`，返回捕获的 `lsn`。不截断 WAL。
- `src/wal/checkpoint.rs:37-58` `read_checkpoint_site`：文件缺失 → `Ok(None)`；读取 < 16B → `Ok(None)`；否则解析 `[lsn: u64 LE][timestamp: u64 LE]`。`write_checkpoint_site`（:61-80）truncate 写 16B + `sync_all`。
- `src/wal/recovery.rs:60-131` `full_recover(db_path, buffer_pool, table_manager)`：`WalReader::read_all` 全量 → 分类（`BeginTxn` → all；`Commit`/`CommitTxn` → committed；`Abort`/`AbortTxn` → aborted；Insert/Update/Delete → data_records）→ uncommitted = all − committed − aborted → 仅对 committed 的 data_records `redo_record`（`.is_ok()` 才计 redo_count）→ `mark_uncommitted_aborted`。
- 静默吞错五处（K05）：`recovery.rs:114-119`（redo 失败被 `.is_ok()` 吞掉、不计不报）、`:146-149` 与 `:162-165`（`get_table` Err → `return Ok(())`）、`:178-180`（Delete 的 `find_key`/`index_manager.delete` `let _ =`）、`:188-194`（`mark_tx_aborted` `let _ =`）。
- `src/wal/reader.rs:31-101` `read_next`：13B peek 自检旧/新格式；新格式 `[lsn:8][type:1][len:4][body:len][crc:4]`，`deserialize_with_lsn` 返回 `(lsn, record, consumed)`，lsn 即该记录起始**文件字节偏移**（当前被丢弃）。`read_all`（:95-101）从当前位置读到 EOF；`seek_to(lsn)` = 字节偏移 seek（:104-109）。
- `src/wal/writer.rs:16-43` `WalWriter`：`file: Arc<Mutex<File>>`（`create+append+read` 打开，O_APPEND）；`write_record`（:46-71）seek End 取偏移为 LSN；`truncate_to`（:89-102）= `set_len(lsn)`，只能截**尾部**，不能裁头部；`get_current_lsn`（:126-141）= 文件 `metadata().len()`。
- `src/wal/buffer.rs:147-182` `do_flush`：`get_current_lsn()` 取 base → 逐条 `serialize_with_lsn(offset)` → `write_batch` + fsync。**磁盘 LSN 一律是写入时的文件字节偏移**（内存逻辑 LSN 在 flush 时被丢弃）→ WAL 文件重写截断后，后续写入自动按新偏移编 LSN，无需改 WALBuffer（T07 不触发）。
- `src/database.rs:27-79` `open`：每次打开**无条件** `full_recover`（:52-55），错误 map 为 `StorageError::WalError`；`close()`（:163-166）仅 `flush_all`；`CheckpointManager` 未接线。
- redo 不幂等于已落盘页：`write_tuple_to_data_page`（`src/storage/data_page.rs:11-35`）盲 `add_slot`，无存在性检查 → clean close（页已刷）后重开会把 committed Insert 再追加一遍。这是既有边界（现网测试未做精确行数断言：`recovery_e2e_test.rs:119-126`、`schema_persistence_test.rs:90-98`）。checkpoint 截断后 clean-reopen 的重放集≈空，该路径自然消除；测试构造"崩溃"必须用 drop-without-close（页未刷 → 全量重放重建，无重复）。
- 既有测试兼容面：`tests/recovery_test.rs` 全部走 `recover()`（仅分类、无 redo），不受 redo 显式化影响；`tests/checkpoint_test.rs` 直接构造 `CheckpointManager`（tempdir fixture 可复用），`test_checkpoint_flow` 断言 `checkpoint_lsn > 0`；`tests/drop_table_free_test.rs:177-185` restart-after-drop **先 `close()` 再重开** → 接线后 close() 触发 checkpoint 截断 pre-drop stale 记录，显式化不破坏该流程。
- `src/executor/drop_table.rs` 不写任何 WAL 记录（grep 无 wal 引用）→ WAL 中不存在 drop 标记，恢复端无法从 WAL 判断表已被 drop（见 Risks）。

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/wal/checkpoint.rs` | `CheckpointManager::{checkpoint, read_checkpoint_site, write_checkpoint_site}` | 位点读写 + 刷脏页 + 本 Cycle 增加重写截断 |
| `src/wal/recovery.rs` | `RecoveryManager::{full_recover, redo_record, mark_uncommitted_aborted}` | 恢复分类 + redo + 本 Cycle 消费位点与显式化 |
| `src/wal/reader.rs` | `WalReader::{read_next, read_all, seek_to}` | 记录读取（含 embedded LSN） |
| `src/wal/writer.rs` | `WalWriter`（`file: Arc<Mutex<File>>`, `get_current_lsn`, `truncate_to`） | WAL 追加写 + 本 Cycle 重写截断的落点 |
| `src/database.rs` | `Database::{open, close}` | 接线 `CheckpointManager` + close 触发 |

**Critical Path**

```
Database::checkpoint() / close()
  └─► CheckpointManager::checkpoint()
        ├─ L = wal_writer.get_current_lsn()          （字节偏移，返回值语义保持）
        ├─ buffer_pool.flush_all()                    （位点前缀的页效果全部落盘）
        ├─ wal_writer.fsync()
        ├─ write_checkpoint_site(L, ts)               （先写有效位点：语义="重放 ≥ L"）
        ├─ 写 WalRecord::Checkpoint（追加）
        └─ 重写截断（持 writer 文件互斥，单次临界区）：
              读 [L..end) 字节 → 从 0 覆写 → set_len(|suffix|) → sync_all
              → write_checkpoint_site(0, ts2)         （截断后位点失效为"重放全部"）
崩溃重启：Database::open ──► full_recover
        ├─ site = read_checkpoint_site()              （None → 全量）
        ├─ records = read_all_with_lsn()；分类仍用全部记录（uncommitted 标记不裁剪）
        ├─ redo 过滤：site=(L,_) 且 L ≤ file_len → 仅 embedded_lsn ≥ L；
        │             site 缺失/损坏/L > file_len → 全量
        └─ redo/清理任何失败 → Err 传播 → Database::open 失败可见
```

**Implementation Guidance**

- 顺序：T1（位点消费）→ T2（显式化）→ T3（重写截断 + 接线）→ T4（回归）。T1/T2 只动 recovery 侧、可用手写 WAL/位点文件测试；T3 是生产侧，依赖 T1 的消费语义先就位。
- T1：`read_all` 现返回 `Vec<WalRecord>` 且丢弃 LSN——新增 `read_all_with_lsn() -> Vec<(u64, WalRecord)>`（或等价）供 `full_recover` 过滤，保持 `read_all` 兼容（现有调用方不改）。位点读取复用 `CheckpointManager::read_checkpoint_site` 的 16B 语义：`full_recover` 手头只有 `db_path`，可将位点读取提为按路径的自由函数或在 recovery 内按 `db_path.with_extension("checkpoint")` 读——形态由 Act 定，语义必须是：缺失/<16B → None；否则 `(lsn, ts)`。
- T2：五处吞错逐一显式化；`redo_record` 的表缺失错误信息须含表名（S2.3 见证断言用）。`mark_uncommitted_aborted` 失败也须传播（可将 `mark_tx_aborted` 的结果收集后统一 `?`）。
- T3：重写截断必须在 writer 的 `Arc<Mutex<File>>` 单次临界区内完成"读后缀 → 覆写 → set_len → sync"，防止与 `do_flush`/`write_record` 的并发追加交错（O_APPEND 下覆写后 set_len 之前的中间态只可能多出旧尾部字节，解析遇垃圾 → T2 的显式错误兜底）。**禁止 temp 文件 + rename**：rename 后 writer 持有的 FD 仍指向旧 inode，后续 WAL 写全部丢失。`Database::open` 增加公开字段 `checkpoint_manager: Arc<CheckpointManager>`（`Database` 已是 `Clone` + 全 Arc 字段）；`close()` 改为先 `checkpoint()`（其内部已 flush_all，close 的"显式落盘"效果保持）。T3 的 S2.1 无重见证依赖"崩溃 = drop-without-close"：pre-checkpoint 行来自已刷页、post-checkpoint 行来自后缀重放，精确行数断言成立。
- 位点过滤与重写的配合已按崩溃窗口审计：位点先于截断写入（截断前的崩溃用 `≥ L` 过滤、无丢无重），截断后位点写 0（后续恢复全量重放已缩短的文件）。残余窗口见 Risks。

**Behavioral Change**

- 当前行为：`full_recover` 全量重放 + 静默吞错；checkpoint 不截断 WAL、位点不被消费；`Database::open` 不接触 CheckpointManager；`close()` 只刷页。
- 目标行为：有效位点使 redo 只重放位点之后的记录；checkpoint 后 WAL 文件物理缩短（有界）；恢复失败显式报错并使 `Database::open` 失败；`close()` 自动触发 checkpoint；`Database::checkpoint()` 公开可调。
- 接口变化：`Database` 新增公开字段 `checkpoint_manager` 与公开方法 `checkpoint()`；`full_recover` 错误语义从"部分静默成功"变为"失败即 Err"（`RecoveryResult` 字段与成功路径形状不变）；`WalReader` 新增带 LSN 的读取方法（`read_all` 签名不变）。
- 错误语义：恢复期表缺失/IO 失败/记录损坏 → `Err`（含表名或记录上下文）→ `Database::open` 返回 `Err`，由调用方决定中止。

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R2/S2.1, S2.2, S2.4 | `wal/recovery.rs::full_recover`；`wal/reader.rs`（带 LSN 读取） | 全量分类 + 全量 redo | 消费位点：有效位点过滤 redo，分类保持全量；退化路径保持 |
| T2 | R2/S2.3 | `wal/recovery.rs::{redo_record, mark_uncommitted_aborted, full_recover}` | 五处静默吞错 | 显式 Err 传播到 `full_recover` 返回值 |
| T3 | R2/S2.1 | `wal/checkpoint.rs::checkpoint`；`wal/writer.rs`（互斥截断辅助）；`database.rs::{open, close}` | 位点写而不用、不截断、未接线 | 重写截断 + `open` 接线 + `close()` 触发 + `checkpoint()` wrapper |
| T4 | R2/R5 | 全工作区 | — | 全量回归 |

**Task Contracts**

### T1: full_recover 消费 checkpoint 位点

- Requirement/Scenario: R2/S2.1（位点消费面）、S2.2、S2.4
- Depends on: None
- Targets: `src/wal/recovery.rs::full_recover`；`src/wal/reader.rs`（新增带 LSN 读取）
- Current behavior: `full_recover` 对全部 data_records 无差别 redo；`read_all` 丢弃记录起始偏移；位点文件从不被读取
- Required behavior: 位点缺失/损坏（<16B）→ 全量 redo（S2.2/S2.4，行为与现状一致）；位点 `(L, _)` 有效且 `L ≤ WAL 文件长度` → 分类仍覆盖全部记录（uncommitted 标记依赖完整 `BeginTxn` 集合），redo 仅对 embedded LSN ≥ L 的 data_records；`L > 文件长度`（代际失效）→ 全量 redo。`redo_count` 只统计实际执行 redo 的记录数
- Required changes: recovery 读位点（16B 语义与 `read_checkpoint_site` 一致）；redo 循环按位点过滤；`WalReader` 提供记录起始偏移（`read_all_with_lsn` 或等价；`read_all` 保持兼容）
- Preserve: `recover()`（basic）、`read_wal()`、`needs_recovery()` 签名与语义；`RecoveryResult` 字段；分类逻辑；WAL 记录格式
- Forbidden: 裁剪分类范围（只读位点之后记录会导致 uncommitted 漏标）；改位点文件 16B 格式；改 `recover()` 语义
- Test witness: 新增 `tests/checkpoint_redo_reduction_test.rs`：手写 WAL（复用 `recovery_test.rs` 的 `WalRecord::serialize_with_lsn` fixture 风格）+ 手写 16B 位点文件 → `full_recover` 的 `redo_count` 只含位点之后记录；位点缺失/截断为 15B/`L > file_len` → 全量（RED→GREEN）
- GREEN condition: 上述位点场景测试全绿；既有 `tests/recovery_test.rs`、`tests/recovery_e2e_test.rs` 全绿
- Verification: `cargo test --test checkpoint_redo_reduction_test --test recovery_test --test recovery_e2e_test`
- Stop when: 过滤语义需改 WAL 记录格式、位点格式，或需要代际标记文件

### T2: 恢复静默吞错显式化（K05）

- Requirement/Scenario: R2/S2.3
- Depends on: T1（同文件顺序实施）
- Targets: `src/wal/recovery.rs::full_recover`、`redo_record`、`mark_uncommitted_aborted`
- Current behavior: 表缺失 `return Ok(())`（:148/:164）；Delete 索引清理 `let _ =`（:179）；`mark_tx_aborted` `let _ =`（:193）；redo 失败被 `.is_ok()` 吞掉（:114-119）
- Required behavior: redo 期间 `get_table` 失败 → `Err`（消息含表名）；redo 的页写入/索引操作失败 → `Err` 传播；uncommitted 标记失败 → `Err`；`full_recover` 对任何失败返回 `Err`，`Database::open` 将其map 为打开失败。已提交记录重复重放不因"版本已存在"报错（现 redo 盲追加本就不检查，天然满足，保持）
- Required changes: 五处吞错点改为 `?` / 收集后传播；错误上下文含表名或 tx_id
- Preserve: 成功路径的 redo 结果与 `redo_count` 语义（成功才计数）；正常恢复的输出不变
- Forbidden: 保留任何"失败即跳过"的恢复路径；把表缺失降级为警告
- Test witness: S2.3 —— 手写含 `Insert{table:"ghost"}` + `CommitTxn` 的 WAL（catalog 无 ghost 表）→ `full_recover` 返回 `Err` 且消息含 `"ghost"`；同文件 `Database::open` 返回 `Err`（RED→GREEN）
- GREEN condition: S2.3 测试绿；既有 wal/recovery 测试全绿（若既有测试依赖静默跳过，允许机械同步调用点/fixture，断言逻辑不得修改）
- Verification: `cargo test --test checkpoint_redo_reduction_test`；`cargo test --all` 中 wal/recovery 相关套件
- Stop when: 显式化导致既有合法流程（如 restart-after-drop）无法在保持其 Acceptance 的前提下通过（此时回 Plan 评估语义）

### T3: checkpoint() WAL 重写截断 + Database 接线

- Requirement/Scenario: R2/S2.1（截断 + redo 收敛）
- Depends on: T1, T2
- Targets: `src/wal/checkpoint.rs::checkpoint`；`src/wal/writer.rs`（受文件互斥保护的重写截断辅助）；`src/database.rs::{open, close}` + 新增 `checkpoint()`
- Current behavior: checkpoint 六步、不截断；`Database` 无 CheckpointManager；`close()` 仅 `flush_all`
- Required behavior: `checkpoint()` 按 Critical Path 次序：捕获 L → flush_all → wal fsync → `write_checkpoint_site(L, ts)` → 写 Checkpoint 记录 → 持 writer 文件互斥单次临界区内"读 `[L..end)` → 从 0 覆写 → `set_len(|suffix|)` → `sync_all`" → `write_checkpoint_site(0, ts2)`；返回值保持"本次捕获的 LSN"。截断后 WAL 文件长度 ≈ 后缀 + Checkpoint 记录（有界）。`Database::open` 构造并持有 `checkpoint_manager: Arc<CheckpointManager>`（公开字段）；新增 `Database::checkpoint()` 公开 wrapper；`close()` 先触发 `checkpoint()` 再保持现有落盘效果
- Required changes: 上述行为；`WalWriter` 如需暴露"互斥内读后缀+覆写"的辅助方法（形态由 Act 定，须单临界区）
- Preserve: WAL 记录序列化格式；`write/read_checkpoint_site` 16B 格式；`WALBuffer` 及 flush loop 零改动；既有 `tests/checkpoint_test.rs` 断言（`checkpoint_lsn > 0` 等）全绿；`close()` 的"显式落盘"效果
- Forbidden: temp 文件 + rename（writer FD 指向旧 inode，后续写丢失）；改 WALBuffer/flush actor（T07 红线）；`should_checkpoint` 写循环接线
- Test witness: S2.1 —— （a）insert N 事务 → `db.checkpoint()` → WAL 文件长度显著小于 checkpoint 前；（b）再 insert M 事务 → drop-without-close（崩溃模拟）→ `Database::open` → `SELECT` 行数精确等于 N+M 对应行（无丢无重：pre-checkpoint 行来自已刷页、post-checkpoint 行来自后缀重放）；（c）drop-without-close 后直接 `full_recover`：`redo_count` 显著小于 checkpoint 前全部 data 记录数（RED→GREEN）；`close()` 触发：open→insert→close → WAL 长度有界
- GREEN condition: 上述测试全绿；`cargo test --test checkpoint_test --test checkpoint_redo_reduction_test` 全绿
- Verification: `cargo test --test checkpoint_redo_reduction_test --test checkpoint_test --test drop_table_free_test`
- Stop when: 实现被迫改 WALBuffer、记录格式或位点格式；或发现必须 actor 化才能保证截断原子性（T07 触发点——记录于 Response，返回 Plan）

### T4: 全量回归与验证

- Requirement/Scenario: R2（全场景）, R5
- Depends on: T3
- Targets: 全工作区
- Current behavior: 无（T3 已完成）
- Required behavior: 4 项质量命令全绿；新增与既有测试全绿
- Required changes: 验证（无代码改动）
- Preserve: 隐式路径、公共 SQL/网络接口、既有测试断言
- Forbidden: 为过检查引入 `#[allow]`；改既有测试逻辑
- Test witness: `cargo test --all`
- GREEN condition: `cargo test --all` 0 failures（≥553）；`cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` 全 0；`openspec validate --all` 通过
- Verification: 四项命令 + `openspec validate --all`
- Stop when: 任何 check 失败需返工；或公共行为变化

**Invariants**

- WAL 记录序列化格式不变（重写复用原字节；embedded LSN 保持写入时偏移语义）。
- 位点文件 16B `[lsn LE][timestamp LE]` 格式不变。
- `WALBuffer`、flush loop、Group Commit 语义零改动（T07 红线）。
- `recover()`（basic 分类）、`read_wal()`、`needs_recovery()` 对外语义不变。
- 无位点时 `full_recover` 行为与现状一致（全量重放）。
- 分类始终覆盖全部记录；uncommitted 标记不因位点裁剪而漏标。
- 恢复失败必须显式报错；不存在新的静默跳过路径。
- `TransactionManager` 是事务生命周期 WAL 记录的唯一来源（M10/MS06-T01 契约），checkpoint 只追加 `WalRecord::Checkpoint`。

**Non-goals**

- T06 谓词/LIMIT 下推（Iteration 002）；T07 消息传递重构。
- `should_checkpoint` 写循环阈值自动触发。
- redo 的逐记录 applied 标记 / clean-close 后重复回放的通用修复（截断使 clean-reopen 重放集≈空，见 Risks；通用修复属后续 change）。
- 事务 ID 分配器跨重启回跳（`database.rs:57-64` `_max_tx_id` 计算后丢弃）——既有问题，不在本 Iteration。
- 位点文件多代/校验和扩展。

**Acceptance**

| Acceptance | 验证 |
|---|---|
| R2/S2.1 redo 收敛 + 无丢无重 | T3(b)(c)：崩溃重开行数精确；`redo_count` 显著小于 checkpoint 前记录数；T1 位点过滤见证 |
| R2/S2.2 无 checkpoint 全量恢复 | T1：位点缺失 → 全量 redo，行为与现状一致（既有 recovery 测试全绿背书） |
| R2/S2.3 损坏/表缺失显式报错 | T2：ghost 表 WAL → `full_recover` Err（含表名）→ `Database::open` Err |
| R2/S2.4 位点损坏安全退化 | T1：位点 <16B / `L > file_len` → 全量重放不 panic |
| R5 质量门 | T4：4 项命令 + `openspec validate --all` |

**Verification**

- `cargo build`（0 rustc warning）
- `cargo clippy --all-targets -- -D warnings`（0 warning）
- `cargo fmt --check`（0 diff）
- `cargo test --all`（≥553 tests，0 failures）
- `cargo test --test checkpoint_redo_reduction_test`（新增）
- `openspec validate --all`

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 逐条核实于当前工作区（checkpoint/recovery/reader/writer/buffer/database + 4 个测试文件交互面）；磁盘 LSN=文件偏移、redo 盲追加、DropTable 无 WAL 记录、restart-after-drop 先 close 等关键事实已确认 |
| Design | PASS | 位点消费/显式化/重写截断/接线四决策闭合；崩溃窗口次序（位点先于截断、截断后位点置 0）与禁止 temp+rename 的理由已写入契约 |
| Iteration Plan | PASS | Iteration 001 单一职责 T1-T4，依赖有序；稳定基线/验证/诊断边界明确（tasks.md §Iteration 001） |
| Cycle Scope | PASS | initial；T1-T3 覆盖 R2 全部 scenario（S2.1-S2.4），T4 质量门 |
| Task Contracts | PASS | 每 Task 有 Targets/Current/Required/Preserve/Forbidden/Test witness/GREEN/Verification/Stop |
| Traceability | PASS | tasks.md RTM R2 → Iter 001 → `tests/checkpoint_redo_reduction_test.rs`（新增）→ 上述代码面 |
| Verification | PASS | 4 项质量命令 + 新增测试通过条件明确；关键行为（redo 收敛、无丢无重、显式报错、退化）均有可执行断言 |

**Persisted Evidence**

- Mode: none

`none` —— 全部验证（四项质量命令、新增/既有测试）可低成本本地重跑，决定性输出（退出码、warning/failure 计数、redo_count 数值）写入 Act Response 即可；崩溃窗口的正确性由测试断言与 diff 审查确认，无需持久化。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超 256 KiB。本 Cycle 无 `required`，不创建 Evidence 目录。

**Risks and Notes**

- **中（设计固有权衡，非阻塞）**：重写截断的崩溃窗口——正在覆写/`set_len` 时崩溃可能留下部分后缀或旧尾部字节；恢复端表现为解析错误 → 显式 open 失败（T2 兜底，不静默丢）。位点先于截断写入保证截断前崩溃无丢无重；"文件自上次 checkpoint 起倍增 + 崩溃于截断后、位点置 0 前"的极端窗口存在丢弃后追加记录的理论路径。Act 在 Response 记录实际实现的窗口边界。
- **中**：显式化后，"drop 已持久化到 catalog + pre-drop Insert 记录仍在 WAL + 崩溃未 checkpoint"窗口下 `Database::open` 显式失败（设计决策 3 接受的语义）。clean 路径由 close() 触发 checkpoint 截断 stale 记录保障（`drop_table_free_test.rs:177-185` 先 close 再重开，T3 后继续通过）。
- **低**：无 checkpoint 的崩溃恢复中，若部分页已被 BufferPool 驱逐刷盘，全量重放会重复追加 committed 行（redo 盲追加的既有边界，现网测试不做精确行数断言）；checkpoint 截断使 clean-reopen 重放集≈空，自然消除主路径。通用修复（applied 标记）列为 Non-goal。
- **低**：`database.rs:57-64` 恢复后事务 ID 分配器不回跳（`_max_tx_id` 丢弃）——既有问题，本 Iteration 不动。
- **非实质（留 Act）**：位点读取的实现形态（自由函数/构造 manager）；带 LSN 读取的方法名与形状；重写截断辅助方法放 `writer.rs` 还是 `checkpoint.rs`；错误信息具体措辞（须含表名/tx_id 上下文）。

## Act Response

- Status: reported

**Implemented**

Iteration 001 全部 4 个 Task 完成（2026-09-05，工作区未提交，待用户触发 commit；与 T04 未提交变更同工作区，提交顺序由用户编排）：

- **T1（位点消费）**：`full_recover` 读取位点文件（16B 语义经 `checkpoint::read_site_file` 自由函数与 `CheckpointManager` 共享单一实现），有效位点 `(L, _)` 且 `L ≤ WAL 文件长度` 时 redo 仅对记录偏移 `≥ L` 的 committed 数据记录；缺失/`<16B`/`L > file_len` → `redo_from = 0` 全量重放。分类始终覆盖全部记录（uncommitted 标记不裁剪）。`WalReader` 新增 `read_next_with_lsn`/`read_all_with_lsn`（新格式返回内嵌 LSN＝写入时文件偏移；旧格式退化为读取时偏移），`read_all`/`read_next` 改为委托、签名与语义不变。`redo_count` 只统计实际执行 redo 的记录数。
- **T2（K05 显式化）**：六处静默吞错全部显式化——redo 失败 `.is_ok()` 计数（recovery.rs Step 2）、Insert/Update/Delete 的 `get_table` 失败 `return Ok(())`、Delete 索引清理 `let _ =`、`mark_tx_aborted` `let _ =`。`redo_record` 返回 `Result<(), WalError>`，`mark_uncommitted_aborted` 同步返回 Result 并在 `full_recover` 以 `?` 传播；新增 `WalError::RedoFailed(String)` 变体，错误上下文含表名/tx_id/row_id。`Database::open` 经既有映射以 `StorageError::WalError` 打开失败。
- **T3（重写截断 + 接线）**：`WalWriter::rewrite_truncate(lsn)` 在 writer 文件互斥**单次临界区**内完成"读 `[lsn..end)` 后缀 → 非 append 句柄从 0 覆写 → `set_len(|suffix|)` → `sync_all`"（持有句柄 O_APPEND 的 write 恒落文件尾，覆写头部必须独立句柄；禁止 temp+rename，注释写明理由）。`CheckpointManager::checkpoint()` 按 Critical Path 次序：捕获 L → `flush_all` → `wal fsync` → `write_checkpoint_site(L)` → 追加 Checkpoint 记录 → `rewrite_truncate(L)` → `write_checkpoint_site(0)` → `reset_write_count`，返回捕获的 L。`Database` 新增公开字段 `checkpoint_manager: Arc<CheckpointManager>` 并在 `open` 接线；新增公开 `Database::checkpoint()`；`close()` 改为触发 checkpoint（内部 `flush_all` 保持"显式落盘"效果）。
- **T4（全量回归）**：562 passed / 0 failed（基线 553 + 本 Iteration 新增 9）；build/clippy/fmt/`openspec validate --all` 全 0（见 Verification Evidence）。

**Changed Files and Symbols**

| 文件 | 变更 |
|---|---|
| `src/wal/reader.rs` | 新增 `read_next_with_lsn`/`read_all_with_lsn`；`read_next`/`read_all` 改为委托（签名不变） |
| `src/wal/checkpoint.rs` | 位点读取提为 `pub(crate) fn read_site_file`（`read_checkpoint_site` 委托）；`checkpoint()` 重写为 9 步截断流程 |
| `src/wal/recovery.rs` | `full_recover` 消费位点 + redo 过滤；`redo_record`/`mark_uncommitted_aborted` 显式错误化 |
| `src/wal/record.rs` | 新增 `WalError::RedoFailed(String)` + Display 分支 |
| `src/wal/writer.rs` | 新增 `rewrite_truncate`；`wal_path` 字段注释更新并移除失效的 `#[allow(dead_code)]`；import `Read` |
| `src/database.rs` | 新增字段 `checkpoint_manager` + `open` 接线 + 公开 `checkpoint()`；`close()` → `checkpoint()` |
| `tests/checkpoint_redo_reduction_test.rs`（新增） | 9 集成测试：S2.1(a)(b)(c)/S2.2/S2.3/S2.4 + close 触发 |
| `tests/checkpoint_test.rs` | 2 处既有断言按截断后位点语义同步（见 Deviations 2） |
| `tests/recovery_e2e_test.rs` | `test_data_pages_survive_restart` fixture 增一行 `flush_all()`（见 Deviations 3），断言零修改 |
| `tests/executor_test.rs` | 5 处 `Database` 字面量构造机械补 `checkpoint_manager` 字段（fixture 级，断言零修改） |

**Deviations from Plan**

1. **`rewrite_truncate` 采用独立非 append 句柄覆写头部**（`ACT` 契约内形态选择）：Plan Critical Path 写"从 0 覆写"，但 writer 持有句柄以 O_APPEND 打开——`seek(0)+write` 恒写文件尾，物理上无法覆写头部。实现为：互斥内经持有句柄读后缀，再以 `OpenOptions::write(true)`（无 append）临时句柄覆写 + `set_len` + `sync_all`，全程持锁单临界区（`write_record`/`write_batch`/`get_current_lsn` 同锁串行）。temp+rename 红线保持。
2. **`tests/checkpoint_test.rs` 2 处既有断言同步（`checkpoint_lsn > 0` → `assert_eq!(checkpoint_lsn, 0)`）**：`PLAN-ERROR`——Preserve 条款要求该断言全绿，与 Critical Path 的"截断后 `write_checkpoint_site(0, ts2)`"直接矛盾（位点置 0 是安全必需：截断后新写入从文件头重新编 LSN，若保留旧位点 L，文件重新增长超过 L 时过滤器会错误排除新记录）。按规范性 Critical Path 实施；同步时增强见证（新增 `captured_lsn > 0` 断言保留"返回捕获 LSN"的检查）。请 Plan Review 定分类。
3. **`tests/recovery_e2e_test.rs::test_data_pages_survive_restart` fixture 同步**：该测试不 `close()`，catalog 从未落盘；K05 显式化后"WAL Insert 引用 catalog 缺失的表"按设计语义（决策 3）使 open 显式失败。按 T2 契约"允许机械同步 fixture、断言逻辑不得修改"，在崩溃模拟前补 `db.buffer_pool.flush_all()` 使表定义持久（保持"崩溃不 close"特性）。同因同果：`drop_table_free_test::test_cross_restart_after_drop_safe` 在 T2 完成后、T3 完成前出现**预期暂态失败**（Plan Risks 已预见），T3 接线 close→checkpoint 截断 stale 记录后转绿，最终全绿。
4. **`tests/executor_test.rs` 5 处机械同步**：该文件以字面量构造 `Database`（Plan 变更面未列举测试侧构造点），为公共字段新增的必然编译后果，机械补字段。
5. **位点读取形态**：契约提供"自由函数或 recovery 内读"两选项，Act 选 `pub(crate) fn read_site_file` 于 checkpoint.rs——16B 语义单一实现，recovery 按 `db_path.with_extension("checkpoint")` 调用（与 `CheckpointManager::new` 派生规则一致）。
6. **`Database::checkpoint()` 返回 `Result<()>`**（丢弃捕获 LSN；`CheckpointManager::checkpoint` 仍返回 `Result<u64>`）：契约仅要求"公开 wrapper"，形态由 Act 定。
7. **`redo_record` 签名 `storage::Result<()>` → `Result<(), WalError>`**：契约要求表缺失/IO 失败显式 `Err` 且上下文含表名；`WalError::RedoFailed` 为新增变体（WAL 记录格式/位点格式未动，Forbidden 项全部保持）。

**Blocker Handoff**

None（正常完成）

**Blocker Resolution**

None（未恢复）

**Self-Review**

- Plan compliance: PASS——T1-T4 契约逐项核对（Targets/Current/Required/Preserve/Forbidden/Test witness/GREEN/Stop）；Invariants 全保持（WAL 记录序列化格式、位点 16B 格式、WALBuffer/flush loop/Group Commit 零改动、`recover()`/`read_wal()`/`needs_recovery()` 语义、无位点全量重放、分类不裁剪、无新增静默路径、TransactionManager 唯一生命周期 WAL 源 + checkpoint 仅追加 Checkpoint 记录）；Non-goals 全未触碰（无 should_checkpoint 接线、无 applied 标记、无 tx-id 分配器改动、无位点代际扩展）。
- Full diff reviewed: PASS——`git diff` 逐文件复核全部本 Cycle 变更（wal 5 文件 + database.rs + 4 测试文件）；与 T04 未提交变更的 `database.rs` 同文件重叠区域已区分（T04 区域：事务 API 四方法，本 Cycle 区域：import/字段/open 接线/checkpoint/close，互不覆盖）。
- Critical findings unresolved: None
- Important findings unresolved: None
- Minor findings unresolved: 1 项，见 Remaining Issues 3（既有 do_flush 并发 base 竞态观察，非本 change 引入）。

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 见证 RED | `cargo test --test checkpoint_redo_reduction_test`（实现前） | `valid_site_limits_redo_to_records_after_site ... FAILED`：`left: 5, right: 2`（位点被忽略全量重放），exit 101 | 预期 RED ✅ |
| T1 GREEN | 同上（实现后） | `test result: ok. 4 passed; 0 failed`，exit 0 | PASS |
| T1 受影响边界 | `cargo test --test checkpoint_redo_reduction_test --test recovery_test --test recovery_e2e_test` | 4+3+6 passed; 0 failed，exit 0 | PASS |
| T2 见证 RED | `cargo test --test checkpoint_redo_reduction_test missing_table`（实现前） | `unwrap_err()` on `Ok(RecoveryResult { redo_count: 1, .. })`——ghost 重放被静默吞掉且计入 redo_count，exit 101 | 预期 RED ✅ |
| T2 GREEN | `cargo test --test checkpoint_redo_reduction_test --test recovery_test --test recovery_e2e_test` | 5+3+6 passed; 0 failed，exit 0 | PASS |
| T3 见证 RED | `cargo test --test checkpoint_redo_reduction_test`（实现前） | `error[E0599]: no method named 'checkpoint' found for struct 'Database'`（×3），exit 101 | 预期 RED ✅ |
| T3 GREEN（受影响边界） | `cargo test --test checkpoint_redo_reduction_test --test checkpoint_test --test drop_table_free_test --test recovery_test --test recovery_e2e_test` | 9+3+6+3+6 passed; 0 failed（含暂态转绿的 `test_cross_restart_after_drop_safe`），exit 0 | PASS |
| 全量测试 | `cargo test --all`（fmt 后终跑） | `total passed: 562 failed: 0`（基线 553 + 新增 9），exit 0 | PASS |
| 构建 | `cargo build` | 0 条 rustc warning（仅 `~/.cargo/config` 弃用提示），exit 0 | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | 0 finding，exit 0 | PASS |
| 格式 | `cargo fmt --check`（`cargo fmt` 后） | clean，exit 0 | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 13 passed, 0 failed (13 items)` | PASS |

**Persisted Evidence**

`None required`（mode: none）——全部验证可低成本本地重跑，决定性输出（退出码、通过/失败计数、RED 症状）已录入上表；截断/重放行为由测试断言与 diff 审查确认。

**Experience Candidates**

None

**Remaining Issues**

1. **既有存储层竞态**（Iteration 000 Review 遗留，本 Cycle 未触碰相关代码，维持待用户决策）：`FileStorage::allocate_page` 与并发 `get_page` 的文件扩展竞态（`Io(UnexpectedEof)`）。
2. **语句级原子性与 DDL 非事务性**（Iteration 000 记录的规格外边界，本 Cycle 不变）。
3. **既有 `do_flush` 并发 base 竞态（观察，非本 change 引入）**：`WALBuffer::do_flush` 的 `get_current_lsn()`（取 base）与 `write_batch`（持锁写入）非原子——并发触发（capacity 内联 flush 与 flush_loop 定时/notify flush）下两个 flusher 可先后取得同一 base，向相同偏移交错写入。属 WALBuffer 红线范围（T07），本 Cycle 未触碰；10K tx 压测（MS06-T03/T04）未暴露。建议作为 Improvement 候选交用户决策。
4. **重写截断的极端崩溃窗口**（Plan Risks 已记录，实测窗口边界）：位点置 0 前崩溃（窗口 = 一次 16B 位点文件写入的时长）且截断后追加记录使文件重新超过 L 时，`site(L)` 过滤可能排除低 embedded-LSN 的新记录。实现将位点置 0 紧跟截断，窗口已压至最小；超出 Plan 已接受范围，未做额外防护。

**Commit or Diff Reference**

未提交（待用户触发 commit，沿用项目「未 commit（待用户触发）」惯例；与 Iteration 000 的 T04 变更同工作区，`src/database.rs` 两区域不重叠，提交编排由用户决定）。本 Cycle 变更面：5 个 wal 源文件 + `database.rs`（增量）+ 新增 1 测试文件 + 3 个既有测试文件机械同步。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（非 Act Self-Review 代替）结果：R2 全部场景有测试见证，全部验证门独立复跑通过，无阻塞 finding。

- 代码独立核对：`full_recover` 位点消费（`read_site_file` 16B 语义单一实现 + `lsn ≤ wal_len` 校验 + 分类不裁剪 + `≥ redo_from` 过滤）；K05 五处吞错全部显式化（`RedoFailed` 上下文含表名/tx_id/row_id）；`checkpoint()` 九步次序与 Critical Path 逐项吻合（含新增的 wal fsync 步）；`rewrite_truncate` 单临界区、无 temp+rename；`Database::open` 接线 + `close()` → `checkpoint()`（flush_all 效果保留）。
- Minor 1（记录，不阻塞）：`tests/recovery_e2e_test.rs::test_data_pages_survive_restart` 补 `flush_all()` 后，该测试场景变为"页已落盘 + 无位点全量重放"——committed 行会被盲追加 redo 重复（Plan Risks 已记录的既有 redo 非幂等边界）；测试弱断言（`!rows.is_empty()`）容忍之，测试意图（重启后数据可访问）保持成立。该重复回放边界维持 Non-goal（applied 标记属后续 change）。
- Minor 2（记录，不阻塞）：`rewrite_truncate` 在持有 writer 互斥期间打开第二个写句柄——单进程内与持锁语义一致，跨进程并发写同一 WAL 文件本就在支持范围之外。
- Act Remaining Issues 4 项核实并维持：存储层竞态（Iter 000 遗留，待用户决策）；语句级原子性/DDL 边界（不变）；**`do_flush` 并发 base 竞态观察**（见 Deviation Classification 后说明）；重写截断极端窗口（Plan Risks 已接受范围）。
- 关于 `do_flush` base 竞态的实质评估：T05 的 redo 过滤依赖"embedded LSN = 记录起始偏移"，该竞态可破坏此不变量；但位点在稳态下恒为 0（全量重放），过滤仅在与 `site(L>0)` 共存的崩溃窗口（一次 16B 位点写入时长）内消费 embedded LSN，实际暴露面极窄；竞态本体是 WALBuffer 既有行为（本 Cycle 红线禁改）。判定为非阻塞 Minor，建议立 Ixx 交用户决策。

**Deviation Classification**

Act 记录的 7 项偏差逐一独立核对 diff 后分类：

1. `rewrite_truncate` 独立非 append 句柄覆写头部——`ACT-DEVIATION`（契约内形态选择）。Plan Critical Path 的"从 0 覆写"未指明句柄形态，而持有句柄为 O_APPEND、物理无法覆写头部；Act 的实现保持全部契约属性（writer 互斥单临界区、无 temp+rename、原地截断、embedded LSN 字节不变）。非实质，不阻塞。
2. `tests/checkpoint_test.rs` 2 处断言 `checkpoint_lsn > 0` → `assert_eq!(checkpoint_lsn, 0)`——**`PLAN-INVALID`**：Plan 的 Preserve 条款（"该断言全绿"）与 Plan 自己的 Required Behavior/Critical Path（"截断后位点置 0"）直接矛盾，属计划缺陷。Act 按规范性 Critical Path 实施、以 `captured_lsn > 0` 新增"返回捕获 LSN"契约的见证（强于原断言）、同步处注明理由——是唯一自洽解。核查属实，不阻塞。
3. `tests/recovery_e2e_test.rs` fixture 增 `flush_all()`——`ACT-DEVIATION`（T2 契约明确允许"机械同步 fixture，断言逻辑不得修改"）。根因是设计决策 3 的显式化语义使"catalog 未落盘 + WAL 引用该表"从静默跳过变为显式失败（Plan Risks 已预见）；`drop_table_free_test::test_cross_restart_after_drop_safe` 的暂态失败→T3 接线后转绿，与 Plan Risks 预测一致。
4. `tests/executor_test.rs` 5 处构造补字段——`PLAN-OMISSION`（Plan 未列举测试侧 `Database` 字面量构造点），公共字段新增的必然编译后果，机械同步，断言零修改。
5. 位点读取提为 `pub(crate) read_site_file`——契约提供的选项之一，非偏差。
6. `Database::checkpoint()` 返回 `Result<()>`——契约"公开 wrapper"的形态自由，非偏差。
7. `redo_record` → `Result<(), WalError>` + 新增 `WalError::RedoFailed(String)`——T2 契约的必要实现；错误变体不触 WAL 记录格式/位点格式（Forbidden 全保持）。

全部不阻塞。

**Acceptance Gaps**

None。逐项核验（全部独立复跑）：S2.1（`valid_site_limits_redo_to_records_after_site`：redo_count==2 且分类不裁剪 ≥5 committed；`crash_after_checkpoint_replays_suffix_exactly`：8 行精确 = 无丢无重；`redo_count_converges_after_checkpoint`：redo_count==3；`checkpoint_truncates_wal_file`：WAL 物理缩短至 <128B）；S2.2（`missing_site_falls_back_to_full_redo`：redo_count==5）；S2.3（`missing_table_during_redo_fails_explicitly`：`full_recover` Err 含 "ghost" + `Database::open` Err 含 "ghost"）；S2.4（15B 截断位点 / LSN 超文件长 → 全量重放 redo_count==5）；R5 质量门独立复跑全绿。

**Convergence**

N/A（首次 Review，无上一版 gap 可比较）

**Evidence**

独立复跑（2026-09-05，本工作区）：

- `cargo build` → 0 rustc warning（仅 `~/.cargo/config` 弃用提示），exit 0
- `cargo clippy --all-targets -- -D warnings` → 0 finding，exit 0
- `cargo fmt --check` → clean，exit 0
- `cargo test --all` → `passed=562 failed=0`，exit 0（基线 553 + 新增 9）
- 目标套件：`checkpoint_redo_reduction_test` 9/9、`checkpoint_test` 3/3、`recovery_test` 3/3、`recovery_e2e_test` 6/6、`drop_table_free_test` 6/6，全部 0 failed
- `openspec validate --all` → `Totals: 13 passed, 0 failed (13 items)`

代码独立核对：`git diff` 逐文件复核 `reader.rs`（`read_next_with_lsn` 起始偏移捕获 + 新旧格式分支）、`record.rs`（`RedoFailed`）、`checkpoint.rs`（九步 + `read_site_file`）、`recovery.rs`（位点消费/过滤/显式化）、`writer.rs`（`rewrite_truncate`）、`database.rs`（接线，与 T04 区域不重叠）、4 个测试文件同步。Persisted Evidence mode `none`，无 Evidence 目录要求。

**Follow-up Decision**

接受（`accepted`）：全部 R2 Acceptance 有测试见证且独立验证通过；7 项偏差中 1 项为 `PLAN-INVALID`（Plan Preserve 条款自相矛盾，Act 的消解正确且增强见证）、其余为契约内形态选择或机械同步，均不阻塞。2 项 Minor finding 记录不返工。后续动作按职责分离：commit 由用户触发；`do_flush` base 竞态与存储层竞态是否立 Ixx、SNAPSHOT 刷新由用户调用 `openspec-docs-maintainer` 决定；本 Review 不修改产品代码与全局状态。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/002-pushdown/000-initial.md`（T06 谓词/LIMIT 下推，Plan Context 已展开，Gate 2 全 PASS，Status: ready；含对 design 决策 2 的 Aggregate 缺口闭合与 design 决策 1 执行器罗列按现役面收缩的记录）
