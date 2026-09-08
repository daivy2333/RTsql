# Iteration 000 / Cycle 000-initial: 文件 magic/格式版本头落地

## Plan Context

- Status: ready
- Iteration: 000-initial（带头文件格式生效）
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 主库文件自带 64B 自描述头；非 RTsql / 新版 / 未知 flag / 截断文件打开时干净拒绝（exit 1，不 panic）；0 字节新库与伴生文件语义不变；全量回归绿。
- Verification boundary: `cargo test` 全量 0 failed + clippy/fmt/validate 全 0 + `tests/file_header_test.rs` 拒绝矩阵全绿。
- Diagnostic boundary: `src/storage/file_header.rs`、`src/storage/file_storage.rs`、`src/storage/error.rs`、`src/storage/mod.rs`、`src/storage/async_storage.rs`（文档）与对应测试。
- Deferred tasks: None（单 Iteration change）。

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部 What Changes；design D1-D8；用户决策 2026-09-08（exit 1 复用 / 旧文件统一拒绝 / 仅主库加头 / 不加 CRC）。
- Excluded scope: WAL/checkpoint 加头、旧文件嗅探迁移、free-list 持久化、MS12 加密实现、可变页大小、REPL/网络路径、lib API 签名变化。

**Objective**

RTsql 主库文件以 64 字节自描述头开始；`FileStorage::open` 在独占锁之后、任何页解析与 WAL I/O 之前完成头初始化（0 字节新库）或分类校验（非 RTsql / 新版 / 未知 flag / 截断 → additive 错误变体，CLI 退出码 1）；页寻址经 `64 + N*4096` 平移且 `PageId::to_offset` 纯数学语义不变；全量回归零失败。

**Background**

需求来源 MS10-T03（`tasks.md`）："文件 magic/格式版本头（FileStorage open 校验）"，路线图标注"趁零用户落，不兼容即报'文件由新版创建'"。当前 `FileStorage::open` 对内容零校验，实测（2026-09-08）：8192B 垃圾文件 → SlottedPage 无校验切片 panic → `PageGuard::drop` PoisonError unwrap → SIGABRT exit 134；4096B → 误导性 `IO error: failed to fill whole buffer`（catalog 固定触碰页 0/1 越过 EOF）；57B 文本 → 误导性 `Page size mismatch: expected 4096, got 57`。MS12-T01 以本任务为硬前置（加密 flag + Argon2id 盐区 + 页级 transform 不得覆盖头）。

**Current Baseline**

- revision `268fa4f`（master，工作树仅含本 change 产物），636 tests pass / 0 failed / 2 ignored；clippy 0 / fmt 0 / validate 18 PASS。
- `FileStorage::open`（`src/storage/file_storage.rs:20-58`）：create-or-open 读写 → `try_lock` 独占锁（冲突 → `DatabaseLocked`，先于 WAL）→ `file_len % 4096 != 0 → PageSizeMismatch` → `page_count = file_len / 4096`。对内容零校验。
- `PageId::to_offset = id * page_size`（`src/storage/page_id.rs:9-11`），生产调用点仅 file_storage.rs 3 处（读 :69 / 写 :81 / 分配 :109）。
- catalog：`TABLES_PAGE_ID=0`、`COLUMNS_PAGE_ID=1` 固定（`src/storage/catalog.rs:35-37`）；`Catalog::bootstrap` 在空文件按序分配页 0/1 并断言（:98-137）。
- `AsyncStorage::page_count`（`src/storage/async_storage.rs:16-21`）：空文件返回 0，`TableManager::new` 据此 bootstrap/open 分支。
- 伴生文件：`db_path.with_extension("wal")`（`src/wal/writer.rs:27`）、`with_extension("checkpoint")`（`src/wal/checkpoint.rs:52`）——均不加头（用户决策 3）。
- CLI `open_error_status`（`src/cli/mod.rs:143-154`）：仅特判 `DatabaseLocked → exit 4`，其余 → `General` exit 1。

**Current-State Evidence**

- 打开链（生产唯一调用点）：`Database::open`（`src/database.rs:28-93`）→ `FileStorage::open` → `BufferPool::new(100)` → `TableManager::new`（bootstrap 或 open）→ `open_or_init` → `WalWriter::open` → `RecoveryManager::full_recover` → `CheckpointManager::new`。头校验插入 `FileStorage::open` 内即先于一切页解析与 WAL I/O。
- panic 崩溃链（8192B 垃圾文件实测）：`database.rs:36` → `table_manager.rs:160`（open_or_init）→ `catalog.rs:210`（scan_tables）→ `catalog.rs:392`（scan_chain）→ `buffer_pool.rs:153`（with_page_data）→ `slotted_page.rs:128` panic → `page_frame.rs:101` unwrap PoisonError → 非 unwinding abort。头校验在 `FileStorage::open` 返回前拒绝即可全部规避。
- 偏移收敛：`grep -rn to_offset src/` 仅 page_id.rs 定义 + file_storage.rs 3 调用点；tests 无主库文件裸 I/O（`grep read_exact_at|write_all_at|seek( tests/` 为空）；唯一文件长度断言在 `tests/drop_table_free_test.rs:18-19`（`metadata().len() / PAGE_SIZE`）。
- 契约锚点：`tests/storage_test.rs:10-17`（`to_offset(5)==20480`、`to_offset(0)==0`——D3 下零修改）；`tests/storage_test.rs:110-117`（首分配页 = PageId(0)、page_count 递增）；`tests/file_storage_io_test.rs:65-76`（1 页文件读 `PageId(3)` → `UnexpectedEof`）。
- 错误面：`src/storage/error.rs:7-80` 全部变体；CLI 消费仅 match `DatabaseLocked` + 通配，无穷尽 match 消费方（本会话复核 cli/mod.rs 与 `grep -rn "StorageError::" src/cli/`）。
- sad path 实测矩阵（exit 码，`target/debug/rtsql` @ 268fa4f）：见 Background 三条 + 0 字节文件被当作新库正常 bootstrap（文件 0 → 8192B）。

**Relevant Code**

| 文件 | 职责 |
|---|---|
| `src/storage/file_storage.rs` | 头校验/初始化宿主；open、read/write/allocate 页偏移、锁 |
| `src/storage/page_id.rs` | `to_offset` 纯数学（不改） |
| `src/storage/error.rs` | +3 additive 变体 |
| `src/storage/mod.rs` | 导出 `file_header` |
| `src/storage/async_storage.rs` | `page_count` 文档措辞（"empty file" → "header-only file"） |
| `src/storage/page.rs` | `PAGE_SIZE=4096`（不改） |
| `tests/drop_table_free_test.rs` | `page_count(path)` helper 扣头 |
| `tests/file_header_test.rs` | 新建：roundtrip + 拒绝矩阵 |
| `tests/cli_test.rs` | 增补 e2e |
| `tests/database_file_lock_test.rs` | 增补锁优先场景 |

**Critical Path**

`Database::open(path)` → `FileStorage::open(path)`：锁 → `metadata()` → `len==0 ? encode+write_all_at(0) : read 64B + decode 分类校验` → `(len−64) % 4096` 校验 → `page_count = (len−64)/4096` → 后续 `read_page/write_page/allocate_page` 全部经 `HEADER_SIZE + to_offset` 访问 → `TableManager::new` 以 `page_count()==0` 分支 bootstrap（header-only 文件命中）→ catalog bootstrap 分配页 0/1（文件 8256B）。错误路径：各拒绝分支提前返回，不触碰 WAL/checkpoint。

**Implementation Guidance**

- 建议顺序：T1（模块 + 单测，编译期 RED）→ T2（FileStorage 接线 + 拒绝矩阵 RED→GREEN + 既有回归）→ T3（CLI e2e RED→GREEN + 全量门）。T2 的拒绝矩阵先行 RED 依赖 T1 的 encode 夹具写测试头。
- `file_header.rs` 不依赖 `StorageError`（D2）：`decode` 返回模块私有 `HeaderError`，由 `FileStorage::open` match 后映射 `StorageError` 变体并附路径。
- 头写入：`file.write_all_at(&encode(), 0)?`（open 是同步上下文，`FileExt` 已在 file_storage.rs 引入）；不做 fsync（D7）。
- `NewerFileVersion` 消息需含路径：变体携带 `String`（路径 + found），对齐 `DatabaseLocked(String)` 先例（D5 修正为携带路径的形态，避免 Display 拼接歧义）。
- `KNOWN_FLAGS_MASK = FLAG_ENCRYPTED`；flags/salt/reserved 校验拒绝时消息带字段详情（`IncompatibleHeader(String)`）。
- `allocate_page` 扩容后 `file_len` AtomicU64 语义 = 页数（不含头），`page_count()` 返回值不变；`free_page` 置零路径经 `write_page` 自动平移。
- 既有测试夹具零适配预期：约 30 处空文件 `FileStorage::open` 透明获头；唯 `drop_table_free_test` helper 与 `database_file_lock_test` 增补面。若全量回归出现其他断点，按"文件长度差 64B"排查，不得为过测试改断言语义。

**Behavioral Change**

- 当前：打开任意大小合法文件都进入页解析；垃圾文件 panic/abort 或误导性 IO/页大小错误；新库文件从 0 字节起步；无格式概念。
- 目标：非空文件必须携带合法 64B 头才进入页解析；拒绝按原因分类（`NotADatabase` / `NewerFileVersion` / `IncompatibleHeader` / `PageSizeMismatch`）且发生在锁之后、伴生文件与页数据触碰之前；新库文件 open 即 64B；页偏移统一 +64。
- 不变：`Database::open`/`FileStorage::open` 签名、`DatabaseLocked` 语义与优先级、catalog 页 0/1 保留、`page_count` 页数语义、WAL/checkpoint 行为、CLI 退出码表。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/新库带头、v1 重开 | `src/storage/file_header.rs`（新） | 无 | 64B 布局 + encode/decode + 常量 + 单测 |
| T1 | R1 | `src/storage/mod.rs` | 导出 storage 公共项 | 导出 `file_header` |
| T2 | R1/R2/R3 | `src/storage/file_storage.rs::open` | 锁 + 页整除校验 | 头初始化/分类校验插入锁后；页整除改 `(len−64)%4096` |
| T2 | R1 | `file_storage.rs::{read_page_blocking,write_page_blocking,allocate_page}` | `to_offset` 直接作偏移 | 偏移加 `HEADER_SIZE`（3 处） |
| T2 | R2 | `src/storage/error.rs` | StorageError 枚举 | +`NotADatabase(String)`/`NewerFileVersion(String)`/`IncompatibleHeader(String)` |
| T2 | R4 | `src/storage/async_storage.rs` | page_count 文档 | 措辞更新（语义不变） |
| T2 | R4 | `tests/drop_table_free_test.rs:18-19` | `len/PAGE_SIZE` 计页数 | `(len−64)/PAGE_SIZE` |
| T2/T3 | R2/R3/R4 | `tests/file_header_test.rs`（新）、`tests/cli_test.rs`、`tests/database_file_lock_test.rs` | — | 拒绝矩阵 + e2e + 锁优先场景 |

**Task Contracts**

### T1: 文件头模块可独立单测

- Requirement/Scenario: R1（新库带头 / v1 重开 / to_offset 纯数学）
- Depends on: None
- Targets: `src/storage/file_header.rs`（新）、`src/storage/mod.rs` 导出
- Current behavior: 无头模块；格式知识不存在。
- Required behavior: `FileHeader::encode()` 产出 64B：magic `"RTSQLDB\0"`、version u32 LE=1、flags u32 LE、page_size u32 LE、salt/reserved 零；`decode` 对合法字节返回 `FileHeader`，对 magic 错/len<64 由调用方区分、version=0、version>1、未知 flag 位、page_size≠4096、salt/reserved 非零分别给出可分类错误；`HEADER_SIZE=64`、`FORMAT_VERSION=1`、`FLAG_ENCRYPTED`、`KNOWN_FLAGS_MASK` 公开。
- Required changes: 仅新增模块 + mod.rs 导出行；不触碰 FileStorage。
- Preserve: `page_id.rs`、`page.rs`、`error.rs` 零修改。
- Forbidden: 不引入 serde/依赖；不在模块外复制布局知识。
- Test witness: `src/storage/file_header.rs` 内 `#[cfg(test)]`——roundtrip、各非法输入分类断言；TDD：测试随实现同文件先写，编译失败即 RED 起点。
- GREEN condition: `cargo test --lib file_header` 全绿。
- Verification: `cargo test --lib file_header 2>&1 | tail -5`；退出码 0。
- Stop when: decode 需要读文件 I/O（契约失效——本模块只做内存编解码）。

### T2: FileStorage 头校验/初始化与偏移平移

- Requirement/Scenario: R1 全部、R2 全部、R3 锁优先、R4 零回归
- Depends on: T1
- Targets: `src/storage/file_storage.rs::{open,read_page_blocking,write_page_blocking,allocate_page}`、`src/storage/error.rs`、`src/storage/async_storage.rs`（文档）、`tests/drop_table_free_test.rs:18-19`、`tests/file_header_test.rs`（新）、`tests/database_file_lock_test.rs`（增补）
- Current behavior: open 零内容校验（`file_storage.rs:20-58`）；偏移 = `to_offset`；垃圾文件 panic/abort。
- Required behavior: 按 design D4 分支（len==0 写头；<64/magic/version=0 → `NotADatabase(path)`；version>1 → `NewerFileVersion`；未知 flags/page_size/salt 非零 → `IncompatibleHeader`；`(len−64)%4096≠0` → `PageSizeMismatch`）；页 I/O 偏移 +64；新变体 Display 含路径与字段详情；`page_count` 语义不变。
- Required changes: D3/D4/D5/D6 全部落点；错误变体 additive。
- Preserve: 锁在头校验之前（file_storage.rs:31-37 不动）；`to_offset` 纯数学；`DatabaseLocked` 语义；`PageId(0)` 首分配；`UnexpectedEof` 越界语义；约 30 处空文件测试零适配通过。
- Forbidden: 不改 WAL/checkpoint/recovery；不改 catalog 常量；不加 CRC；不迁移旧文件；不改 `PageId::to_offset`。
- Test witness: `tests/file_header_test.rs` 拒绝矩阵（含 `garbage_8k_file_opens_with_clean_error`——当前实现 panic → RED；先落测试观察 RED 再接线）；`database_file_lock_test` 增补"坏文件 + 持锁 → DatabaseLocked"；`storage_test`/`file_storage_io_test` 既有断言零修改保持绿。
- GREEN condition: 拒绝矩阵全绿 + 全量 `cargo test` 0 failed（除 cli_test 新用例外）。
- Verification: `cargo test --test file_header_test --test storage_test --test file_storage_io_test --test drop_table_free_test --test database_file_lock_test 2>&1 | tail -8`；退出码 0。
- Stop when: 偏移平移导致既有测试出现非 64B 差异的失败（实质基线问题 → Blocker Handoff）；或 `TableManager` bootstrap 分支在 header-only 文件上不触发（契约失效）。

### T3: CLI e2e 与全量验证门

- Requirement/Scenario: R2（垃圾/新版场景）、R3（锁优先、伴生文件）、R4（伴生文件不变）
- Depends on: T2
- Targets: `tests/cli_test.rs`（增补约 4 用例）
- Current behavior: CLI 打开垃圾文件 → 进程 panic/abort（exit 134）或误导错误；无格式错误分类。
- Required behavior: `rtsql <garbage> "SELECT 1"` → exit 1 + stderr "not an RTsql database: <path>"；构造 `NewerFileVersion` 文件 → exit 1 + "newer version" 文案；拒绝路径不创建 `.wal`/`.checkpoint`；坏文件被另一持有者锁定 → exit 4。既有 cli_test 17 用例零修改通过。
- Required changes: 仅测试文件。
- Preserve: 既有用例断言与退出码语义。
- Forbidden: 不改 `src/cli/` 生产代码（Display 途经即满足）；不加退出码分类。
- Test witness: 新用例在 T2 完成前 RED（panic 而非断言失败，注意测试进程隔离）；T2 后 GREEN。
- GREEN condition: cli_test 全量（17 + 4）绿。
- Verification: `cargo test --test cli_test 2>&1 | tail -5` → 全量 `cargo test 2>&1 | tail -4`、`cargo clippy -- -D warnings 2>&1 | tail -3`、`cargo fmt --check`、`openspec validate`；全部退出码 0。
- Stop when: e2e 需要修改生产 CLI 代码才能表达断言（说明错误 Display 面不足 → Blocker Handoff）。

**Invariants**

- 锁（`DatabaseLocked`，exit 4）先于头校验；锁语义与 MS10-T02 一致。
- `PageId::to_offset` 纯数学；catalog `TABLES_PAGE_ID=0`/`COLUMNS_PAGE_ID=1` 不变。
- `page_count()` 只统计页数；`page_count()==0 → catalog bootstrap` 契约保持。
- 头位于页空间之外；MS12 页级 transform 未来不覆盖头（本 change 只保证布局）。
- WAL / `.checkpoint` 创建、消费、截断行为零变化。
- 退出码表 0/1/2/3/4/5 + 128+signum 不扩。

**Non-goals**

见 proposal Out of Scope（WAL/checkpoint 加头、旧文件迁移、CRC、free-list 持久化、MS12 加密、可变页大小、Windows、REPL/网络、API 签名）。

**Acceptance**

1. 新库（0 字节或不存在）打开后文件以 64B 头开始，重开校验通过，SQL 行为与基线一致（R1/S1-S2）。
2. 8192B 垃圾文件经 CLI 打开：exit 1 + 明确 stderr，进程不 panic（R2/S3；RED 基线 exit 134）。
3. version>1 / 加密 flag / page_size≠4096 / 截断 / <64B / 旧无头文件分别得到 `NewerFileVersion` / `IncompatibleHeader` / `IncompatibleHeader` / `PageSizeMismatch` / `NotADatabase` / `NotADatabase`，CLI 均 exit 1（R2/S4-S8）。
4. 坏文件被持锁时第二打开者得 `DatabaseLocked` exit 4（R3/S9）；头拒绝路径不创建/修改伴生文件（R3/S10）。
5. 0 字节 = 新库契约保持：`storage_test.rs` 既有断言（含 to_offset 两个单测）零修改通过（R4/S11）。
6. 全量 `cargo test`（636 基线 + 新增）0 failed；clippy/fmt/validate 全 0（R4）。

**Verification**

- `cargo test`（全量）→ 0 failed；预期通过数 = 636 − 2 ignored + 新增（file_header_test ~10 + cli_test 4 + lock_test 1）。
- `cargo test --lib file_header`、`cargo test --test file_header_test --test cli_test`（分项）。
- `cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate`。
- 手工 e2e（可选佐证）：`target/debug/rtsql ./garbage.db "SELECT 1"; echo $?` → 1。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | R19 分析 + 本 Cycle Current-State Evidence（打开链、panic 链、偏移收敛 grep、错误面复核、sad path 实测矩阵） |
| Design | PASS | design D1-D8 闭合（布局/模块/偏移/顺序/变体/契约/写入/测试策略）；无契约语义 TBD |
| Iteration Plan | PASS | tasks.md Iteration Plan（单 Iteration，T1→T2→T3 依赖有序）+ 平衡审计 |
| Cycle Scope | PASS | initial，Acceptance gaps None，Excluded scope 与 proposal Out of Scope 一致 |
| Task Contracts | PASS | 3 个契约含代码位置、行为变化、RED/GREEN、验证与停止条件；Act 只读本 Cycle 可直接建立测试见证 |
| Traceability | PASS | tasks.md RTM：R1-R4 × 场景 × D1-D8 × T1-T3 × 代码面 × 测试见证，无 Missing |
| Verification | PASS | Verification 节命令与通过条件直接证明 Acceptance 1-6；无身份型证据 |

**Persisted Evidence**

- Mode: none

所有验证（全量测试、clippy、fmt、validate、e2e 退出码）均可低成本重跑，Act Response 记录命令、≤20 行决定性输出与退出码即可；无一次性环境、无 Incident 现场、无不可复现结果。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- 头写入无 fsync：崩溃残留半头 → 下次 open 按 `NotADatabase` 拒绝；新库尚无数据，重建零损失（design D7 已论证）。
- `NewerFileVersion` 变体形态：design D5 原写 `NewerFileVersion(u32)`，Implementation Guidance 修正为携带路径的 `String` 形态（对齐 `DatabaseLocked` 先例、避免 Display 歧义）——以 Task Contract 为准，属非实质形态选择。
- 全量回归若出现 64B 差异之外的断点，视为实质基线问题（可能存在未发现的文件长度假设），按契约 Stop 条件返回 Plan，不得改断言硬过。
- MS12 启用盐时需放宽 salt 全 0 校验——已记录于 design D4，属未来 change 面。

## Act Response

- Status: reported

**Implemented**

T1/T2/T3 全部完成，Acceptance 1–6 关闭：

- T1 新模块 `src/storage/file_header.rs`：D1 的 64B 布局、`FileHeader { version, flags, page_size }` + `current()/encode()/decode()`、常量 `HEADER_SIZE=64`/`FORMAT_VERSION=1`/`FLAG_ENCRYPTED=1`/`KNOWN_FLAGS_MASK=0`、私有 `HeaderError` 六分类（BadMagic/ZeroVersion/NewerVersion/UnknownFlags/PageMismatch/ReservedNonZero）；模块内 10 单测 RED→GREEN；`mod.rs` 增 `mod file_header;` 并导出 5 公共项（decode/HeaderError 保持模块内）。
- T2 `FileStorage::open` 按 D4 接线：try_lock 块原样 → 0 字节 `write_all_at` 写头（无 fsync，page_count=0）→ `<64B`/magic 不符/version=0 → `NotADatabase(path)` → version>1 → `NewerFileVersion("file version {v}: {path}")` → 未知 flags（含加密位）/page_size≠4096/保留区非 0 → `IncompatibleHeader(detail + path)` → `(len−64)` 非页整除 → 既有 `PageSizeMismatch`；`read_page_blocking`/`write_page_blocking`/`allocate_page` 3 处偏移加 `HEADER_SIZE`（`PageId::to_offset` 零修改）；`error.rs` 3 个 additive 变体（D5 文案）；`async_storage.rs` page_count 文档措辞（empty → header-only）；`drop_table_free_test.rs` helper 扣 64B 头。
- T3 `tests/cli_test.rs` 增补 4 e2e：垃圾 8k → exit 1 + `not an RTsql database` + 路径 + 文件内容未被修改；version=2 → exit 1 + `newer version`；头拒绝零伴生文件（无 `.wal`/`.checkpoint`）；持锁垃圾文件 → exit 4。生产 CLI 零改动（Display 途经 `open_error_status` General 分支）。

**Changed Files and Symbols**

| 文件 | 变化 |
|---|---|
| `src/storage/file_header.rs` | 新建：布局常量、FileHeader、encode/decode、HeaderError、10 单测 |
| `src/storage/mod.rs` | +`mod file_header;` +`pub use file_header::{FileHeader, FLAG_ENCRYPTED, FORMAT_VERSION, HEADER_SIZE, KNOWN_FLAGS_MASK}` |
| `src/storage/error.rs` | +`NotADatabase(String)`/`NewerFileVersion(String)`/`IncompatibleHeader(String)` |
| `src/storage/file_storage.rs` | `header_rejection` 映射函数；`open` 头初始化/分类校验；`read_page_blocking`/`write_page_blocking`/`allocate_page` 偏移 +64 |
| `src/storage/async_storage.rs` | `page_count` 文档措辞（语义不变） |
| `tests/file_header_test.rs` | 新建：14 用例（头生命周期 2 + 拒绝矩阵 10 + 锁优先守卫 1 + Database 级垃圾 8k） |
| `tests/database_file_lock_test.rs` | +`test_locked_bad_file_reports_lock_not_format` |
| `tests/cli_test.rs` | +4 用例（T03 节） |
| `tests/drop_table_free_test.rs` | `page_count` helper 扣头 |

**Deviations from Plan**

1. **`KNOWN_FLAGS_MASK = 0`（非 Implementation Guidance 第 80 行的 `= FLAG_ENCRYPTED`）**：spec「未知特性 flag 拒绝（加密库前向防护）」场景、Cycle 自身 Acceptance #3、design D4「含加密位——不把密文页当明文解析」三处行为权威一致要求当前构建拒绝加密位；若掩码含 FLAG_ENCRYPTED，真实 MS12 密文库将被当明文解析，恰为该场景要阻止的事故。Guidance 行与三处权威矛盾，按 spec 执行并在此记录——Plan Review 请裁定 Guidance 文案勘误。
2. **RED 观察形态**：拒绝矩阵测试引用新 `StorageError` 变体，对基线代码无法编译（E0599 ×12）——该编译失败本身即「当前实现无此错误分类」的 RED 见证；运行时 RED 经不依赖新变体的层观察：CLI e2e 3 用例 RED（垃圾 → `slotted_page.rs:128` panic → `page_frame.rs:101` PoisonError → abort，`code=None`；version=2 → 误导性 `Page size mismatch: expected 4096, got 64`；伴生场景 → exit 3 且 `.wal` 被创建）+ 2 个 GREEN 守卫（截断 variant、锁优先）。D8 预期的 Database 级垃圾 panic 链在 CLI 子进程失败输出中逐帧复现（与 Plan Current-State Evidence 记录的崩溃链一致）。
3. **clippy 等价改写**：`data_len % page_size as u64 != 0` → `!data_len.is_multiple_of(page_size as u64)`（`-D clippy::manual_is_multiple_of`，等价改写非行为变化）。
4. **Plan Context 状态翻转（draft → ready）由 Act 执行**：用户指令「更改gate状态，开始实施」（2026-09-08）的直接执行；Gate 2 表七维已全 PASS。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS
- Full diff reviewed: PASS
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1

逐任务 Gate 4（spec compliance → code quality）+ Response 前完整 diff Review 结论：diff 面与 Change Surface 逐项一致，无计划外修改；锁块/`to_offset`/catalog/WAL 未触碰；`header_rejection` 对 `HeaderError` 穷尽匹配；无死代码（FLAG_ENCRYPTED 供测试与 MS12 API 面）、无新增警告。已修复发现：`allocate_page` 测试缺 `AsyncStorage` trait 导入（编译错误，即时修复）；`open` 中未使用的 `header` 绑定（警告，改为不绑定）。遗留 Minor：`PageSizeMismatch.actual` 分支中 `data_len % page_size as u64` 计算两次（检查与取值各一次）——可读性等价，不值得引入绑定。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 RED | `cargo test --lib file_header`（todo!() 存根） | `test result: FAILED. 0 passed; 10 failed` | RED ✓ |
| T1 GREEN | `cargo test --lib file_header` | `test result: ok. 10 passed; 0 failed` | PASS |
| T2/T3 RED | `cargo test --test cli_test -- test_garbage_file_clean_rejection …`（基线） | `test result: FAILED. 1 passed; 3 failed`（垃圾 abort code=None；`Page size mismatch: expected 4096, got 64`；exit 3 + wal 创建） | RED ✓ |
| 拒绝矩阵 | `cargo test --test file_header_test …` | `test result: ok. 14 passed; 0 failed`（含 `garbage_8k_file_opens_with_clean_error`） | PASS |
| 既有回归 | `cargo test --test storage_test --test file_storage_io_test --test drop_table_free_test --test database_file_lock_test` | `21 passed / 4 passed / 6 passed / 5 passed`，全部 `0 failed`（storage/io 断言零修改） | PASS |
| CLI 全量 | `cargo test --test cli_test` | `test result: ok. 21 passed; 0 failed; 2 ignored`（17 既有零修改 + 4 新增） | PASS |
| 全量 Gate | `cargo test` | 61 个测试二进制全部 `test result: ok`，`FINAL_EXIT=0`；2 ignored（cli 标定设计项） | PASS |
| Lint | `cargo clippy -- -D warnings` | `Finished \`dev\` profile … in 2.66s`，exit 0 | PASS |
| Format | `cargo fmt --check` | exit 0 | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 19 passed, 0 failed (19 items)`（18 spec + 本 change） | PASS |

**Persisted Evidence**

None required（Plan Context Mode: none；全部验证可低成本重跑，上表即决定性输出）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| None | | | 无满足 Runbook/Incident 门槛的候选（TDD 流程常规；垃圾文件 panic 已由本 change 修复并记录于 proposal/design，非独立故障事件） |

**Remaining Issues**

1. 首次全量 `cargo test` 中 cli_test 出现 1 次失败（当时未捕获用例名）；随后独立重跑 ×2 与全量重跑 ×1 均 21 passed / 0 failed，未复现。结合该套件 4 个信号用例的计时敏感性（并行负载下 200ms 信号窗口与 10s 锁轮询窗口）判断为偶发，与头机制无关联路径（头仅在 open 时多一次 64B 写）。建议后续 Cycle Review 时关注是否再现；再现 3 次则按 Gate 6 立案。

**Commit or Diff Reference**

未提交工作树（用户未触发 commit）；本 Cycle 产物：`iterations/000-initial/000-initial.md` + 上表 9 个代码/测试文件。

## Plan Review

- Review Result: accepted

**Findings**

非阻塞发现 4 项（0 阻塞）：

1. **Guidance 掩码行勘误（PLAN-INVALID，已裁定）**：Implementation Guidance「`KNOWN_FLAGS_MASK = FLAG_ENCRYPTED`」行与三处行为权威矛盾——spec「未知特性 flag 拒绝（加密库前向防护）」、design D4「含加密位——不把密文页当明文解析」、Cycle Acceptance #3。Act 按 spec 取 `KNOWN_FLAGS_MASK = 0` 是**正确执行**（若按 Guidance，MS12 密文库将被当明文库解析，恰为该场景要阻止的事故）。裁定：该 Guidance 行为勘误，以 spec/design/Acceptance 为准；Plan Context 不可改写，勘误以本条为准。MS12-T01 启用加密时由彼时 change 扩展掩码并放宽 salt 全 0 校验（design D4 已预留）。
2. **RED 见证机制（PLAN-OMISSION，minor）**：D8 预期在 file_header_test 内观察运行时 panic RED；实际引用新 `StorageError` 变体的测试对基线无法编译（E0599 ×12），RED 以「编译失败（= 当前实现无此分类）+ CLI e2e 运行时 RED（垃圾 abort code=None、version=2 误导页大小错、伴生文件被创建）」组合见证，panic 链与 Current-State Evidence 记录逐帧一致。接受该替代形态——RED 存在性与方向均已见证。
3. **clippy 等价改写（非实质）**：`data_len % page_size != 0` → `!data_len.is_multiple_of(...)`（`-D clippy::manual_is_multiple_of`），行为等价。
4. **cli_test 一次性偶发失败（观察项）**：Act 首次全量出现 1 次 cli_test 失败（用例名未捕获），此后 Act ×2 + Plan Review ×2 全量重跑均干净（21 passed，含 4 信号用例）。与头机制无因果路径（open 仅多一次 64B 写）。继续观察：再现 3 次按 Gate 6 立案。

独立检查结论：9 个代码/测试文件的 diff 与 Change Surface 逐项一致，无计划外修改；锁块、`to_offset` 纯数学、catalog 常量、WAL/checkpoint 路径零触碰；`header_rejection` 对 `HeaderError` 穷尽匹配；新测试 14 + lock 1 + cli 4 用例与 spec 场景一一对应。

**Deviation Classification**

PLAN-INVALID（Guidance 掩码行，勘误已裁定）+ PLAN-OMISSION（RED 见证机制）——均非阻塞 Minor。ACT-DEVIATION：None（Act 对矛盾 Guidance 的偏离实为对权威 spec 的正确遵循）；BASELINE-CHANGED：None；NEW-EVIDENCE：None。

**Acceptance Gaps**

None。Acceptance 1-6 由 Plan 独立复验：

1. 新库头布局 hexdump 逐字节匹配 D1（`RTSQLDB\0` / version=1 LE / flags=0 / page_size=4096 LE / 盐区零），页 0 首字节 `0x03`（catalog page_type）位于偏移 0x40——页空间恰从 64B 起。
2. 8192B 垃圾文件 e2e：`failed to open database ./garbage2.db: not an RTsql database: ./garbage2.db`，exit 1，无 panic（RED 基线 SIGABRT 134 已消除）。
3-4. 拒绝矩阵 14 用例 + 锁优先守卫 + CLI 4 用例全绿（Act 运行 + Plan 全量复跑确认）。
5. `storage_test` 既有断言（含 to_offset 两个单测）零修改通过。
6. 全量 `cargo test`：61 个测试二进制全部 `test result: ok`、0 FAILED；`cargo clippy -- -D warnings` exit 0；`cargo fmt --check` OK；`openspec validate --all` → `Totals: 19 passed, 0 failed (19 items)`。

**Convergence**

N/A（首次 Review）

**Evidence**

- Plan 独立重跑（2026-09-08，工作树未提交状态）：`cargo test` 61 ok / 0 failed（两轮，cli_test 偶发项未再现）；`cargo clippy -- -D warnings` exit 0；`cargo fmt --check` OK；`openspec validate --all` 19 passed / 0 failed。
- 磁盘布局实证：`xxd -l 64 audit.db`（新库创建后）+ `xxd -s 64 -l 8`（页 0 page_type 0x03）。
- 拒绝 e2e：8192B 随机文件 → exit 1 + NotADatabase 消息（Plan 在 /tmp 独立构造，与 Act 测试互证）。
- diff 审查：`git diff` 9 个代码/测试文件全文 + `file_header.rs`（217 行）/ `file_header_test.rs`（274 行）通读。

**Follow-up Decision**

Acceptance 1-6 全部满足、无阻塞项，Iteration 000 完成——**accepted**。Finding 1 的勘误裁定以本 Review 为准（掩码 = 0 正确）；Finding 4 列观察项不构成返工。change 收尾（tasks/SNAPSHOT 同步 + 归档）待用户调用 `openspec-docs-maintainer`；commit 由用户触发。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None
