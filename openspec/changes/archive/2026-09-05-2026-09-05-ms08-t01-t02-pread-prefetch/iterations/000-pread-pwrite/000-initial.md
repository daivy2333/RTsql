# Iteration 000: T01 页 I/O 位置参数化 / Cycle 000: initial

## Plan Context

- Status: ready
- Iteration: 000-pread-pwrite
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1（baseline 采集）、T2（TDD 改造）、T3（回归 + after 证据）
- Depends on: None（基线 HEAD = `4d410ac`，工作树干净，577 tests 全绿）
- Stable baseline: 页读写经 pread64/pwrite64 完成（strace 页路径 lseek = 0）；并发冷读正确性有永久回归守卫；577 既有测试零修改全绿；`before-MS08-T01` criterion 基线落盘
- Verification boundary: 4 项质量命令全绿 + 新增 `tests/file_storage_io_test.rs` 全绿 + strace before/after 对比成立 + bench 对比结论（改善或明确未达预期）写入 Act Response
- Diagnostic boundary: `src/storage/file_storage.rs`、`tests/file_storage_io_test.rs`
- Deferred tasks: Iteration 001（T02 DataScan 预取）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1（页 I/O 位置参数化）+ R2（零接口零格式变更）
- Excluded scope: T02 预取（Iteration 001）；T03 writev；WalWriter；write_page clone 消除；任何 BufferPool 改动

**Objective**

`FileStorage` 页读写使用 `FileExt::read_exact_at`/`write_all_at`（pread64/pwrite64），syscall 序列由"每页 lseek+read/write"变为"每页 1 次位置参数调用"，共享偏移竞态被结构性消除，并有测试守卫与 strace/bench 量化证据。

**Background**

MS08 第一个任务（无前置依赖）。Explorer 2026-09-05 调查发现：每页 2 syscall 冗余 + `Arc<File>` 无互斥下并发 miss 读的 seek 竞态窗口（未实测复现，但结构成立）。用户 Gate 1 决策（2026-09-05）：接受 S4 竞态测试概率性 RED；对比基线用 micro/data_scan/buffer_pool 三套 bench + strace syscall 计数；T02 prefetch 并入本 change（本 Iteration 不做）。

**Current Baseline**

- revision `4d410ac`（master，工作树干净）
- `FileStorage::read_page_blocking`（`src/storage/file_storage.rs:53-64`）：`seek(SeekFrom::Start(offset))` + `read_exact(&mut buf)`
- `FileStorage::write_page_blocking`（`file_storage.rs:66-77`）：`seek` + `write_all`
- `FileStorage` 是唯一生产 `AsyncStorage` 实现；`file: Arc<std::fs::File>` 无互斥
- 577 tests pass（2026-09-05，MS07-T06 提交后）
- strace 5.16 可用（`/usr/bin/strace`）；测试环境 WSL2（bench 数据需标注）

**Current-State Evidence**

- 入口与调用链：`BufferPool::get_page` miss → `storage.read_page`（`src/storage/buffer_pool.rs:116`）→ `spawn_blocking(read_page_blocking)`；驱逐/flush → `storage.write_page`（`buffer_pool.rs:196/228`）→ `spawn_blocking(write_page_blocking)`。`write_page` 调用前 `page.data.clone()` 4KB（`file_storage.rs:91`，本次不动）。
- 并发边界：miss 信号量 16 permits（`buffer_pool.rs:16`）；`loading_locks` 只串行化同页加载（`buffer_pool.rs:97-102`）；不同页并发加载时 `read_page_blocking` 内的 seek 交错 → 错读窗口。
- 错误路径：`read_exact` 短读 → `UnexpectedEof` → `StorageError::IoError`（经 `?`）。`read_exact_at` 短读同语义。
- 测试入口：`cargo test --all`；`tests/storage_test.rs` 等既有套件为回归面。`Page.data` 公开（`src/storage/page.rs:7-10`），测试可直接构造模式页。
- WAL 对照：`WalWriter` seek 在 `Arc<Mutex<File>>` 内（`src/wal/writer.rs:49-66`），无竞态，不在本 Iteration。

**Relevant Code**

- `src/storage/file_storage.rs` — `FileStorage`（唯一改动文件）：`read_page_blocking`/`write_page_blocking` 两个私有函数与 `use std::io::{Seek, SeekFrom}` 导入
- `tests/file_storage_io_test.rs` — 新增测试（往返/越界/并发）
- `src/storage/page.rs` — `Page::PAGE_SIZE=4096`、`from_bytes`（页构造，只读参考）

**Critical Path**

读：`get_page` → `read_page` → `spawn_blocking` → `read_page_blocking`（seek+read → 改 read_exact_at）。
写：驱逐/`flush_all` → `write_page` → `spawn_blocking` → `write_page_blocking`（seek+write → 改 write_all_at）。
状态变化：无（文件偏移不再被页路径触碰；`file_len`/`free_pages` 逻辑不变）。

**Implementation Guidance**

- 导入改为 `use std::os::unix::fs::FileExt;`（Linux 唯一支持平台）。
- S4 并发测试设计要点：直接持有 `Arc<FileStorage>`（绕过 BufferPool），16 任务各分配专属页并写入任务专属模式（如 `page_id` 相关填充），每任务循环多轮 read+校验（高频交错放大窗口），校验失败即断言错误。测试须独享 tempdir。
- strace 口径：`strace -f -e trace=lseek,pread64,pread,pwrite64,pwrite,read,write -c <test binary>`，对比 before/after 的 lseek 计数差值与 pread64 出现。选一个页读写密集的测试二进制（如 `tests/storage_test.rs` 对应二进制），before/after 用同一二进制同口径。
- bench baseline 名固定 `before-MS08-T01`（tasks.md 已引用），criterion 落盘 `target/criterion`，Act Response 只记摘要。

**Behavioral Change**

- 当前：每页读 = lseek+read（2 syscall）；每页写 = lseek+write；并发冷读不同页有错读窗口。
- 目标：每页读 = pread64；每页写 = pwrite64；位置参数不触碰共享偏移，并发读结构性正确。
- 接口/错误/磁盘格式：零变化（`StorageError::IoError` 包裹 io::Error；短读语义一致）。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | MS08 纪律（tasks.md 验证边界） | 命令行（bench/strace/test） | 无基线留档 | before-MS08-T01 基线落盘 + strace before 计数 |
| T2 | R1/S1/S3/S4 | `src/storage/file_storage.rs::{read_page_blocking, write_page_blocking}` | seek+read_exact / seek+write_all | read_exact_at / write_all_at，删 seek 导入 |
| T2 | R1/S1/S3/S4 | `tests/file_storage_io_test.rs`（新增） | 不存在 | 往返等价 + 越界报错 + 16 任务并发冷读校验 |
| T3 | R1/S4 + R2/S1 | 命令行验证 | — | strace after、`--baseline before-MS08-T01` 对比、全量回归、结论入 Act Response |

**Task Contracts**

### T1: baseline 采集（MS08 前置纪律）

- Requirement/Scenario: MS08 验证边界（tasks.md：前置 baseline 留档）；支撑 R1 syscall 场景对比
- Depends on: None
- Targets: 命令行（无代码改动）
- Current behavior: 无留档基线
- Required behavior: `cargo test --all` 确认全绿；`cargo bench --bench micro_bench --bench data_scan_bench --bench buffer_pool_concurrency_bench --save-baseline before-MS08-T01` 完成落盘；strace before 计数完成
- Required changes: 无代码改动，只采集与记录
- Preserve: 工作树在 T1 结束时仍干净（bench/strace 不产生仓库内文件）
- Forbidden: 任何源码修改；跳过 baseline 直接进入 T2
- Test witness: `cargo test --all` 输出（0 failures）作为变更前 GREEN 基线
- GREEN condition: 三条命令全部成功执行且结果记录在案（Act Response 摘要 ≤20 行/项）
- Verification: bench 命令退出码 0；strace 计数表可读
- Stop when: bench 因环境原因无法运行（如磁盘空间）→ 记录 Blocker，不得跳过留档纪律

### T2: TDD — FileStorage 位置参数化改造

- Requirement/Scenario: R1/页内容读写往返等价、R1/越界读取报错语义不变、R1/并发冷读不同页无错读、R1/并发读写混合不串页
- Depends on: T1
- Targets: `src/storage/file_storage.rs::{read_page_blocking, write_page_blocking}`；`tests/file_storage_io_test.rs`（新增）
- Current behavior: seek+read_exact / seek+write_all；无并发正确性测试
- Required behavior: read_exact_at / write_all_at；新测试四场景存在且改造后全 GREEN
- Required changes: 测试先行（T2.1-2.2）观察初始状态（往返/越界可能直接 GREEN——既有行为已满足，记录之；两个并发场景预期 RED，用户已接受概率性）；然后改造两函数与导入；新测试全 GREEN
- Preserve: `AsyncStorage` trait 签名；`FileStorage` 公开方法；错误类型（`StorageError::IoError`）；`allocate_page`/`free_page`/`sync` 不动
- Forbidden: 修改 `write_page` 的 clone 路径；动 BufferPool/WAL/执行器；引入依赖
- Test witness: `cargo test --test file_storage_io_test`（改造前运行记录初始状态，改造后全 GREEN）
- GREEN condition: 新测试 3+ 场景全过；`cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff
- Verification: `cargo test --test file_storage_io_test` 退出码 0
- Stop when: S4 改造后仍失败（说明改动未消除竞态——真 bug）；或发现 `read_exact_at` 语义与预期不符（如越界行为不同）→ 返回 Plan

### T3: 全量回归 + after 证据 + 对比结论

- Requirement/Scenario: R1/S4（syscall 场景）、R2/S1（零修改回归）
- Depends on: T2
- Targets: 命令行验证（无代码改动，除非回归失败需修复）
- Current behavior: 改造已完成，未验证全局影响
- Required behavior: `cargo test --all` 0 failures（≥577+新增，零修改）；strace after：页读路径 lseek=0、pread64>0；`cargo bench ... --baseline before-MS08-T01` 对比结论（改善或明确未达预期，WSL2 环境标注）
- Required changes: 无新改动；回归失败时在 T2 契约范围内修复
- Preserve: 既有测试文件零修改（发现测试因合理原因需要改 = 实质偏差 → Stop）
- Forbidden: 为通过回归而修改既有测试或弱化新测试
- Test witness: 全量测试 + strace + bench 输出
- GREEN condition: 全部命令退出码 0，结论成文
- Verification: Act Response 记录命令、≤20 行决定性输出、退出码、结论
- Stop when: 既有测试失败且原因不在本改动面（基线变化）→ 记录并返回 Plan

**Invariants**

- `AsyncStorage` trait 与 `FileStorage` 公开签名不变
- 磁盘页布局不变（无迁移）
- WAL 子系统零改动
- 既有测试零修改通过
- 错误类型/语义（短读 UnexpectedEof）不变

**Non-goals**

- T02 预取（Iteration 001）、T03 writev、write_page clone 消除、WalWriter seek 路径、BufferPool 任何改动

**Acceptance**

| 条件 | 映射 |
|---|---|
| 页读写位置参数化（代码事实） | R1 ← T2 |
| 往返/越界/并发冷读/读写混合测试全 GREEN | R1 四场景 ← T2 |
| strace 页路径 lseek=0、pread64/pwrite64 出现 | R1/S4 ← T3 |
| 577+ 零修改全绿 | R2/S1 ← T3 |
| before/after bench + syscall 对比结论成文 | MS08 纪律 ← T1/T3 |

**Verification**

`cargo build`（0 warning）；`cargo clippy -D warnings`（0 warning）；`cargo fmt --check`（0 diff）；`cargo test --all`（0 failures）；`cargo test --test file_storage_io_test`（全 GREEN）；strace before/after 计数；`cargo bench --bench micro_bench --bench data_scan_bench --bench buffer_pool_concurrency_bench --baseline before-MS08-T01`（对比结论）；`openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch`。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | file_storage.rs/buffer_pool.rs/data_scan.rs 逐行读取（本会话）；调用链、并发边界、错误路径、测试入口已记录；唯一实现/唯一替身确认 |
| Design | PASS | design.md：行为差异、导入选择（FileExt）、错误语义（UnexpectedEof 一致）、责任边界、迭代划分理由 |
| Iteration Plan | PASS | tasks.md 两 Iteration；T01 与 T02 不同验证域（底层正确性 vs 行为等价）分开验收；单 Iteration 工作量适中 |
| Cycle Scope | PASS | initial，范围 = R1+R2，排除项明确 |
| Task Contracts | PASS | T1/T2/T3 有目标符号、行为差异、测试见证、GREEN、停止条件；Act 无需回读任何外部文档 |
| Traceability | PASS | RTM 见下 |
| Verification | PASS | 命令与通过条件全部明确；直接观察目标行为（测试内容校验、syscall 计数、零修改回归） |

**RTM**

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 页 I/O 位置参数化 | 页内容读写往返等价 | design §目标行为 T01 | T2 | 000 | `file_storage.rs::read_page_blocking/write_page_blocking` | `file_storage_io_test` 往返用例 | None | Covered |
| R1 | 越界读取报错语义不变 | 同上 | T2 | 000 | 同上 | `file_storage_io_test` 越界用例 | None | Covered |
| R1 | 并发冷读不同页无错读 | design §当前行为（竞态） | T2/T3 | 000 | 同上 | `file_storage_io_test` 16 任务并发用例 + strace | None | Covered |
| R1 | 并发读写混合不串页 | 同上（写路径 seek 同样扰动偏移） | T2 | 000 | 同上 | `file_storage_io_test` 读写混合用例 | None | Covered |
| R1 | syscall 序列位置参数化 | design §关键技术选择 1 | T1/T3 | 000 | 同上 | strace before/after 计数 | None | Covered |
| R2 零接口零格式变更 | 既有行为全量回归 | design §责任边界 | T3 | 000 | 全部改动面 | `cargo test --all` 零修改全绿 | None | Covered |
| R3 DataScan 链预取 | R3 全部 5 场景 | design §目标行为 T02 | Iteration 001 T1-T3 | 001（未展开） | `data_scan.rs::next` | `prefetch_test` + `pushdown_test` 回归 | None | Covered（Iteration 001 展开） |

**Persisted Evidence**

- Mode: none

Act Response 承载：baseline 数值摘要、strace 计数表、bench 对比结论（均 ≤20 行/项）。criterion 基线由 criterion 自身落盘于 `target/criterion`（构建产物区，非仓库），Act Response 引用基线名即可。无需持久化证据文件：所有验证可低成本重跑（命令固定），摘要不丢失决定性结构。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- S4 概率性 RED（用户已接受，2026-09-05 Gate 1 决策）：若改造前未观察到失败，以 strace 结构证据收尾，S4 作为改造后永久守卫。
- bench 改善可能不显著：syscall 计数是主要量化证据；bench 结论允许"未达预期"（MS08 纪律原文）。
- WSL2 环境标注：所有 bench 数值结论需带环境说明。
- 非实质选择留给 Act：测试内部组织、模式内容生成方式、strace 二进制选择（页读写密集即可）。

## Act Response

- Status: reported

**Implemented**

用户 2026-09-05 授权（"更改gate状态，开始实施"）将 Plan Context 置为 `ready` 后，完成 Iteration 000 全部任务。

- **T1 baseline 采集**：`cargo test --all` 两轮全绿（577 passed / 0 failed，变更前 GREEN 基线）；criterion 基线 `before-MS08-T01` 落盘（18 条目：micro 11 + data_scan 4 + buffer_pool_concurrency 3）；strace before（对 `storage_test` 测试二进制，`-f -c -e trace=lseek,pread64,pwrite64,read,write`）：lseek=33、read=38、write=135、pread64=4（动态链接器本底）、pwrite64=0。
- **T2 TDD**：新建 `tests/file_storage_io_test.rs` 四场景。改造前初始状态：往返/越界 GREEN（既有行为已满足，重构见证）；并发冷读与读写混合 **RED 且串页损坏实测复现**（`cross-page read detected: page 8 offset 0 corrupted: got 54, expected 248`）——比 Plan 预期的"概率性 RED"更强。改造：`read_page_blocking` → `FileExt::read_exact_at`、`write_page_blocking` → `write_all_at`，导入 `std::io::{Read, Seek, SeekFrom, Write}` 替换为 `std::os::unix::fs::FileExt`。改造后四场景全 GREEN（双轮运行）。
- **T3 回归 + after 证据**：`cargo test --all` 581 passed / 0 failed（577 既有零修改 + 新增 4）；strace after 同口径同二进制：lseek 33→3（页路径贡献归零，余 3 为加载器/环境）、pread64 4→26（+22 页读迁移）、pwrite64 0→8（页写迁移）、read 38→15；bench after 对比与结论见 Verification Evidence。

**Changed Files and Symbols**

- `src/storage/file_storage.rs`（3 insertions / 7 deletions）— `read_page_blocking`（seek+read_exact → `read_exact_at`）、`write_page_blocking`（seek+write_all → `write_all_at`）、模块导入（io traits → `std::os::unix::fs::FileExt`）。
- `tests/file_storage_io_test.rs`（新增，4 测试）— `pattern_byte` / `fill_page` / `verify_page` + 往返 / 越界 / 16 任务并发冷读 / 8 页读写混合。
- change 内产物：`iterations/000-pread-pwrite/000-initial.md`（Status 由用户授权置为 ready；本 Act Response）、change `tasks.md`（Iteration 000 复选框）。

**Deviations from Plan**

全部为非实质等价调整，未改变行为面与 Acceptance：

1. bench 语法：`--save-baseline` / `--baseline` 是 criterion 参数，须经 `--` 透传（`cargo bench ... -- --save-baseline before-MS08-T01`）；tasks.md 原写法被 cargo 拒绝（`unexpected argument '--save-baseline'`）。
2. strace 5.16（x86_64）不识别 `pread` / `pwrite` syscall 名，trace 集调整为 `lseek,pread64,pwrite64,read,write`（Plan Context 口径含 pread/pwrite）。
3. `cargo clippy -D warnings` → `cargo clippy -- -D warnings`（`-D` 须传给 clippy-driver）。
4. Plan Context 写错误变体 `StorageError::IoError`，实际变体为 `StorageError::Io`（`#[from] std::io::Error`）；测试按实际变体断言 `Io(UnexpectedEof)`，错误语义与计划一致。
5. T2.1 测试文件创建与 T1.2 bench 后台运行时间重叠：测试文件不参与 `cargo bench` 编译与测量，基线有效性不受影响。
6. T2.2 首次运行编译失败（E0382：测试内同名遮蔽绑定被 move，测试自身编写缺陷），修正为具名克隆后观察初始状态；RED 结论以修正后运行为准。
7. 竞态结论升级：Plan 预期 S4"概率性 RED（未实测复现）"，实测直接复现串页损坏——强于计划假设的证据，无需以 strace 结构证据兜底。

**Blocker Handoff**

None

**Blocker Resolution**

None（未发生阻塞）

**Self-Review**

- Plan compliance: PASS
- Full diff reviewed: PASS
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 0

全量 diff（`git diff` + 新增文件）逐行审查：生产改动面仅两个函数体 + 导入替换；`write_page` 的 `page.data.clone()`、`allocate_page`/`free_page`/`sync`/`page_count`、`AsyncStorage` trait 签名、BufferPool/WAL 全部未触碰；文件共享偏移在页路径不再被触碰（`set_len`/`sync_all`/`metadata` 均不依赖偏移）。测试不为错误原因通过：两个并发测试改造前因串页损坏 RED、改造后因位置参数结构性消除竞态而 GREEN；往返/越界为语义保持见证（改造前后 GREEN）。Deviations 1-6 为命令/文字等价调整与测试编写过程，非代码问题；无遗留 Minor。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| 基线健康 T1.1 | `cargo test --all` | `TOTAL passed=577 failed=0`（两轮） | PASS（exit 0） |
| bench 基线 T1.2 | `cargo bench --bench micro_bench --bench data_scan_bench --bench buffer_pool_concurrency_bench -- --save-baseline before-MS08-T01` | `target/criterion` 下 18 个 `before-MS08-T01` 基线目录 | PASS（exit 0） |
| strace before T1.3 | `strace -f -c -e trace=lseek,pread64,pwrite64,read,write <storage_test 二进制>` | `lseek 33 / read 38 / write 135 / pread64 4 / pwrite64 0` | PASS（exit 0） |
| 新测试初始状态 T2.2 | `cargo test --test file_storage_io_test`（改造前） | `2 passed; 2 failed`；panic：`cross-page read detected: page 8 offset 0 corrupted: got 54, expected 248` | PASS（预期 RED 实测复现） |
| 新测试改造后 T2.4 | 同上（双轮） | `4 passed; 0 failed` ×2（0.14-0.15s） | PASS（exit 0） |
| build T2.4 | `cargo build` | `Finished dev profile`（0 代码警告；唯一 warning 为 `~/.cargo/config` 弃用提示，环境既有非本次引入） | PASS |
| clippy T2.5 | `cargo clippy -- -D warnings` | `Finished dev profile in 2.40s` | PASS（exit 0，0 warning） |
| fmt T2.5 | `cargo fmt --check` | 0 diff | PASS（exit 0） |
| 全量回归 T3.1 | `cargo test --all` | `TOTAL passed=581 failed=0` | PASS（exit 0；577 既有零修改 + 4 新增） |
| strace after T3.2 | 同 before 口径（改造后重建二进制） | `lseek 3 / read 15 / write 127 / pread64 26 / pwrite64 8` | PASS（lseek 33→3 页路径归零；pread64 +22、pwrite64 0→8 为页 I/O 迁移；read/write 差值与迁移量吻合） |
| bench 对比 T3.3 | `cargo bench（三套） -- --baseline before-MS08-T01` | 见下方结论 | PASS（exit 0） |
| OpenSpec T3.4 | `openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch` | `Change ... is valid` | PASS（exit 0） |

**bench 对比结论（T3.3，WSL2 同会话采集，18 条目全覆盖）**

- 量化改善 6 项（miss / 大扫描密集路径，与 syscall 减半机制一致）：cache_miss_16_tasks -6.4%、miss_backpressure_200_tasks -7.6%、data_scan/10000 -6.3%、scan_via_index/10000 -5.7%、insert/single_row/0 -5.7%、join/inner_join -3.0%。
- 无显著变化 7 项：data_scan/1000、scan_via_index/1000、select/pk_lookup、delete/by_pk、scan/full_table、filter、sort。
- 回退 4 项，判定为 WSL2 噪声带（非因果）：cache_hit +4.5%（纯缓存命中，不经磁盘，无因果通道）、insert/single_row/50 +5.5%、insert/single_row/99 +17.8%、update/single_column +8.4%——同型操作 insert/single_row/0 为 -5.7%，同操作三档 ±18% 散布远超单次 syscall 收益量级。
- 总结论：**部分量化改善 + 部分未达预期（噪声带内）**；决定性量化证据为 strace syscall 结构变化（页读写 2 syscall → 1）。满足 MS08 纪律"量化改善 OR 明确记录未达预期"。

**Persisted Evidence**

None required（Plan Context Mode: none；全部验证命令可低成本重跑，Act Response 摘要未丢失决定性结构；未创建 `evidence/` 目录）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook | MS08 基线采集方法：criterion 命名基线须经 `--` 透传（`cargo bench ... -- --save-baseline <名>`，cargo 拒绝裸 `--save-baseline`）+ `strace -f -c -e trace=lseek,pread64,pwrite64,read,write <测试二进制>`（x86_64 strace 5.16 无 pread/pwrite 名）+ before/after 同一二进制同口径对比、pipefail 捕获退出码 | 本 Act Response Verification Evidence 表（T1.2/T1.3/T3.2/T3.3） | 已端到端验证成功；MS08 后续各 T 需重复执行；含两个非显然命令陷阱 |

**Remaining Issues**

无阻塞。供 Plan Review 参考：insert/update 类 ms 级 bench 条目在 WSL2 的噪声带约 ±5-18%，后续 MS08 各 T 对比此类条目时建议以 syscall 结构证据为主、bench 为辅。

**Commit or Diff Reference**

未 commit（待用户触发）；工作树：`M src/storage/file_storage.rs`、`?? tests/file_storage_io_test.rs`、`?? openspec/changes/2026-09-05-ms08-t01-t02-pread-prefetch/`

## Plan Review

- Review Result: accepted

**Findings**

独立检查（不依赖 Act Self-Review）：

1. **生产 diff**（`git diff src/storage/file_storage.rs`，3+/7-）：恰为 Task Contract 指定的两函数体 + 导入替换，无越界改动。`read_exact_at`/`write_all_at` 语义核对无误：短读报 `UnexpectedEof`（S3 断言 `StorageError::Io(UnexpectedEof)` 锁定）；位置参数不触碰共享偏移（S4/S5 结构性修复）。`write_page` clone 路径、`allocate_page`/`free_page`/`sync`、`AsyncStorage` trait、BufferPool/WAL 零触碰——符合 Forbidden 边界。
2. **测试质量**（`tests/file_storage_io_test.rs`，154 行 4 测试）：`pattern_byte` 以 31 与 256 互质保证跨页模式必然区分，串页即暴露；并发冷读 16 任务 × 250 轮、读写混合 8 页 × 写者+读者 × 150 轮，交错密度设计合理；读写混合场景"写者内容恒等于模式"的设计使校验失败只可能来自串页，无假阴性通道。测试为错误原因通过（vacuous pass）的风险排除。
3. **RED 见证真实性**：Act 记录改造前 `2 passed; 2 failed`，panic 消息含具体串页证据（`page 8 offset 0 corrupted: got 54, expected 248`）——数字与模式函数可交叉验证（`8*31+0=248`，`54` 为他页字节）。这比 Plan 预期的"概率性 RED"强，竞态从"结构推断"升级为"实测复现"。
4. **新鲜复测**（Review 时独立重跑）：`cargo test --test file_storage_io_test` 4 passed；`cargo test --all` TOTAL 581 passed / 0 failed（577 既有零修改 + 4 新增）；`cargo clippy --all-targets -- -D warnings` 0 warning；`cargo fmt --check` 0 diff；strace 同口径 `storage_test` 二进制：lseek=3（加载器本底）、pread64=26、pwrite64=8——与 Act Response after 计数完全一致，R1 syscall 场景（页路径 lseek=0、pread64/pwrite64>0）成立。
5. **criterion 基线**：`target/criterion` 下 18 个 `before-MS08-T01` 目录实存（micro 11 + data_scan 4 + buffer_pool 3），与 Act 声明一致，Iteration 001 可复用。
6. **MS08 纪律符合性**：baseline 先行（T1 在改造前落盘）+ 量化结论成文（6 项改善 / 7 项无显著变化 / 4 项判定噪声带）。"回退 4 项判定为噪声"的论证成立：`insert/single_row` 同操作三档 -5.7%/+5.5%/+17.8% 散布，且 `cache_hit +4.5%` 无磁盘因果通道。总结论"部分量化改善 + 部分未达预期 + syscall 结构变化为决定性证据"满足验证边界。
7. **Deviation 1-7 评估**：全部为命令语法（cargo `--` 透传、strace syscall 名、clippy `--`）、文档笔误（`IoError`→`Io`）、过程性说明（bench 并行、测试初版编译缺陷自修正）与证据升级（竞态实测复现），无行为面偏离。Deviation 4 属 Plan 笔误（BASELINE 无关），Act 按实际变体断言且语义与计划一致。

Minor findings（不阻塞，记录不返工）：

- M1：Act Response Verification Evidence 表中 strace before 的 pread64=4 归因"动态链接器本底"——Review 复测 after lseek=3 与 before 差值吻合，归因合理但 before 的 4 次 pread64 未逐一归属（非阻塞，计数差值法已自洽）。
- M2：bench 回退项的噪声带判定依据是同操作多档散布对比，未做重复采样——WSL2 环境下可接受（Act 已在 Remaining Issues 建议后续以 syscall 证据为主），不阻塞本轮 Acceptance。

**Deviation Classification**

None（7 项 Deviation 均为非实质等价调整，不构成 PLAN-OMISSION/PLAN-INVALID/ACT-DEVIATION/BASELINE-CHANGED/NEW-EVIDENCE）

**Acceptance Gaps**

None。Acceptance 五项逐条核对：页读写位置参数化（diff 事实）✓；四场景测试全 GREEN（复测）✓；strace 页路径 lseek=0、pread64/pwrite64>0（复测一致）✓；581 零修改全绿（复测）✓；bench+syscall 对比结论成文（Act Response）✓。

**Convergence**

N/A（首次 Review；无既有 gap）

**Evidence**

- `git diff src/storage/file_storage.rs`（3+/7-，两函数+导入）
- `tests/file_storage_io_test.rs` 全文审查（154 行）
- Review 复测命令与输出：`cargo test --test file_storage_io_test`（4 passed，exit 0）；`cargo test --all`（TOTAL 581 passed，无 failed 行）；`cargo clippy --all-targets -- -D warnings`（exit 0）；`cargo fmt --check`（exit 0）；`strace -f -c -e trace=lseek,pread64,pwrite64,read,write <storage_test>`（lseek=3/pread64=26/pwrite64=8，与 Act after 一致）
- `target/criterion/**/before-MS08-T01`（18 目录实存）
- Act Response Verification Evidence 表 + bench 对比结论段

**Follow-up Decision**

Acceptance 全部满足，无阻塞 finding，无需当前 Cycle 修复。两个 Minor finding（M1 归因粒度、M2 噪声带未重复采样）不阻塞且修复成本大于价值，仅记录。Iteration 000 接受；Experience Candidate（MS08 基线采集 Runbook：`--` 透传 + strace syscall 名 + 同口径对比）证据充分，留给用户决定是否授权 Recorder。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-prefetch/000-initial.md`（Iteration 001: T02 DataScan 链预取，待用户指令后由 Plan 展开）
