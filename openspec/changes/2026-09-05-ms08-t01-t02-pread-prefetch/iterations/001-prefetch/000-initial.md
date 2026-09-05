# Iteration 001: T02 DataScan 链预取 / Cycle 000: initial

## Plan Context

- Status: ready（用户 2026-09-05 授权原话："更改gate状态，开始实施吧"；Gate 2 表格已自评 PASS，用户显式批准执行授权）
- Iteration: 001-prefetch
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1（TDD 预取行为测试）、T2（DataScanExecutor 预取实现）、T3（全量回归 + after 证据）、T4（质量门 + validate + 交付）
- Depends on: Iteration 000（pread/pwrite 已 accepted；页 I/O 底层稳定；criterion 基线体系可复用；`before-MS08-T01` 已落盘 18 条目）
- Stable baseline: 全表扫描行序/结果与无预取逐行一致（含谓词 + LIMIT 组合）；链尾无无效预取；预取在途 ≤1；`data_scan_bench` 前后对比结论（改善或明确未达预期）成文
- Verification boundary: 4 项质量命令全绿；`tests/prefetch_test.rs` 全绿；`tests/pushdown_test.rs` 15 测试零修改全绿（等价守卫）；`cargo test --all` 0 failures（≥581 + 新增）；`before-MS08-T02` 基线落盘 + bench 对比结论写入 Act Response
- Diagnostic boundary: `src/executor/data_scan.rs`（`next()` 换页路径 + 预取 helper + 构造器新参数）、`tests/prefetch_test.rs`
- Deferred tasks: 无（本 Iteration 完成后 change 全部交付；MS08-T03 writev 另开 change，依赖本 change 的 pwrite 底层）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R3（DataScan 链预取）全部 5 场景；R1/R2 由 Iteration 000 已满足（本 Iteration 不得回退：`tests/file_storage_io_test.rs` 4 测试必须保持 GREEN）
- Excluded scope: T03 writev、T04/T05/T06、IndexScan/IndexScanAll/Scan 执行器、BufferPool 任何改动、淘汰策略、预取深度 >1

**Objective**

`DataScanExecutor` 沿 `next_page_id` 链推进时对下一页发起预取（装入 BufferPool 缓存），使下一页 miss 加载与当前页行处理重叠；行序、查询结果、可见性、错误语义零变化，并有测试等价守卫与 bench 量化结论。

**Background**

MS08-T02（tasks.md：Prefetch 双缓冲，前置 MS02-T02 DataScan + MS03 BufferPool 优化已 done）。Explorer 2026-09-05 调查：扫描是串行"用完再取"——页耗尽才经 `JumpToPage` 发起下一页 `with_page_data` miss 加载。设计选择（change design.md）：预取目标是 BufferPool 缓存本身（`get_page` 幂等 + loading_locks 保证并发正确），用 `tokio::spawn` 丢弃结果实现"当前页处理 ∥ 下一页装载"的重叠，不建独立用户态缓冲；预取深度 1（单游标扫描，深度 1 已覆盖当前页处理窗口）。

**Current Baseline**

- 工作树：Iteration 000 完成后未 commit（`M src/storage/file_storage.rs`、`?? tests/file_storage_io_test.rs`、`?? openspec/changes/...`）；HEAD = `4d410ac`
- 581 tests pass（577 既有 + 4 新增 file_storage_io_test）；clippy/fmt 0
- `src/executor/data_scan.rs` 325 行，无任何预取逻辑；`next()`（`data_scan.rs:162-324`）主循环结构见 Current-State Evidence
- `data_scan_bench` 存在且用 `DataScanExecutor::new` 直连扫描（`benches/data_scan_bench.rs:70-74`），预取路径将被 bench 自然覆盖

**Current-State Evidence**

- **扫描主循环**（`src/executor/data_scan.rs:162-324`）：每轮 `with_page_data(page_id, closure)`；closure 内解析 SlottedPage，页耗尽（`slot_index >= slot_count`，`data_scan.rs:205-212`）或 all-invisible 快跳（`data_scan.rs:196-203`）时读 `SlottedPageHeader.next_page_id` 返回 `PageAction::JumpToPage(next)`；外层 match（`data_scan.rs:313-317`）更新 `current_page_id = Some(PageId(next_id))`、`current_slot_index = 0` 后 continue——下一轮 `with_page_data` 才触发 miss。链尾 `next_page_id == 0` → `PageAction::Done`（`data_scan.rs:198/208`）。
- **结构体字段**（`data_scan.rs:36-54`）：`buffer_pool: Arc<BufferPool>`、`schema`、`snapshot: Option<Snapshot>`、`predicate: Option<PredicateRef>`（MS07-T06 行内谓词）、`scan_cap: Option<usize>` + `produced`（MS07-T06 提前封顶）、`current_page_id: Option<PageId>`、`current_slot_index: usize`。
- **构造器与调用点**：`DataScanExecutor::new(table_meta, buffer_pool, snapshot, predicate, scan_cap)`（`data_scan.rs:57-83`，从 `table_meta.data_page_head` 起扫）。生产构造点唯一：`src/pipeline.rs:446`（`create_executor_from_plan` 的 `PhysicalPlan::DataScan` 臂，传 `node.predicate, node.scan_cap`）。bench 构造点：`benches/data_scan_bench.rs:70-74`（`DataScanExecutor::new(tm, bp, None, None, None)`）。`src/executor/correlated.rs:36` 消费 `PhysicalPlan::DataScan` 节点但按 plan 注入参数，不经构造器（不受构造器签名变化影响，但若其内部也构造 DataScan 执行器需同步——Act 须以编译器为准核对全部构造点，见 Task Contract T2 Preserve）。
- **BufferPool 预取依赖的行为**：`get_page` 幂等（缓存命中直接返回 `PageGuard`，`src/storage/buffer_pool.rs:74-126`）；同页并发加载被 per-page `loading_locks` 串行化（`buffer_pool.rs:97-108`，双检模式）——预取与真实读取并发安全，真实读取若赶上预取进行中只是等待同一次加载。miss 信号量 16 permits 全 miss 路径共享（`buffer_pool.rs:16`）；预取占用 ≤1/16。淘汰：clock 算法 `evict_one`（`buffer_pool.rs:155-206`），ref_count > 0 的页不被淘汰——预取装入后无人持有时可被正常淘汰，不破坏正确性。
- **tokio 上下文**：`#[tokio::test]` 默认 current_thread flavor 的测试里 `tokio::spawn` 只入队不执行，任务会在 runtime drop 时被取消——**预取不能依赖后台任务被调度**，真实读取路径必须完全不依赖预取完成（本设计天然满足：预取只是缓存预热）。306 个测试用默认 flavor（9 个 multi_thread）；`#[tokio::test]` 宏属性在 current_thread 下 spawn 不 panic（只入队），语义安全。
- **错误路径**：预取任务 `let _ = get_page(...).await;` 丢弃错误；真实读取遇同页 IO 错误时在 `with_page_data` 显式报告——与无预取一致。
- **测试入口**：`cargo test --test pushdown_test`（15 测试，MS07-T06 等价守卫）；`cargo test --test executor_test`（39 测试）；`cargo test --all`；`cargo bench --bench data_scan_bench`。

**Relevant Code**

- `src/executor/data_scan.rs` — `DataScanExecutor`（结构体 + 构造器 + `next()` + `filter_row`/`yield_capped`/`find_visible_in_chain`）
- `src/pipeline.rs:446` — 生产构造点（`create_executor_from_plan` DataScan 臂）
- `benches/data_scan_bench.rs:70-74` — bench 构造点
- `src/storage/buffer_pool.rs` — `get_page`/`loading_locks`/`evict_one`（只读依赖，零改动）
- `tests/prefetch_test.rs` — 新增测试

**Critical Path**

`next()` → `with_page_data(current)`（closure 返回 `JumpToPage(next)`）→ 外层更新游标 → **[新增] 触发对 next 的下一页预取** → continue → `with_page_data(next)`（命中预取装好的缓存或等待在途加载）。数据流：预取只写 BufferPool 缓存，不触碰行处理路径。状态变化：`DataScanExecutor` 新增预取控制字段（在途跟踪）。

**Implementation Guidance**

- 预取触发点：外层 `JumpToPage` 分支（`data_scan.rs:313-317`）之后——此时新当前页 id 已知但页数据未读；下一页 id 须等新当前页的 header 读到后才知道，所以实际可预取的是"新当前页"本身：进入新页前对该页发起预取，与"当前页剩余行处理"重叠的窗口是零（游标已切换）。**两个可行口径**：(a) `JumpToPage` 时对新当前页 spawn 预取——重叠窗口来自下一轮 `with_page_data` 之前无关（真实读取与预取同页，loading_locks 去重，收益为零但无害）；(b) closure 返回 `JumpToPage(next)` 的同时返回 `next` 的 next（两级 lookahead 需读 next 页 header——不可行，next 页未加载）。**正确口径是 (a) 的变体**：在进入新页的 `with_page_data` 之前 spawn 对该页的预取没有意义；真正有收益的是——当 `with_page_data(current)` closure 已拿到 current 页 header 的 `next_page_id` 时，**立即在 await 返回后对 next 页 spawn 预取**，让 next 页加载与"current 页剩余行处理 + 外层循环开销"重叠。Act 按"closure 已知 next，spawn 预取 next"实现：`PageAction::JumpToPage(next)` 分支中 `next != 0` 时 spawn `get_page(PageId(next))`。这是唯一既可行又有重叠窗口的触发点。
- 在途控制：`Option<tokio::task::JoinHandle<()>>` 字段；发起前 `let _ = handle.take()` 丢弃旧 handle（JoinHandle drop 不 abort 任务，旧预取自然完成装入缓存，无害）；扫描结束时 Drop 亦可（不 abort，任务自然结束）。"在途 ≤1"按"未 take 的 handle ≤1"计数。
- 构造器扩展：`new` 增加参数会使既有调用点（pipeline.rs:446、benches、以及编译器揭示的任何其他点）需要同步。为最小化调用面扰动，**推荐**：保持 `new` 签名不变，预取默认启用由内部状态承载，等价对照通过**测试钩子**实现——`#[cfg(test)]` 或 `pub` 的 `prefetch_enabled: bool` 字段 + `with_prefetch(bool)` builder 方法（`new` 默认 `true`，测试传 `false` 得无预取对照）。生产路径零变化，对照路径一行构造。非实质选择（字段可见性/方法名）留给 Act。
- 等价对照测试设计：同一数据集，`DataScanExecutor::new(...)`（预取开）与 `with_prefetch(false)`（预逐渐关）各跑全表扫描，逐行断言相等；SQL 级等价（`execute_sql("SELECT * ...")` 前后行为）由 pushdown_test 15 测试 + 既有 581 全量守卫。
- bench：Iteration 001 Act 开始时先 `cargo bench --bench data_scan_bench -- --save-baseline before-MS08-T02`（注：Iteration 000 的改动已包含在工作区，本基线是"prefetch 前"状态），实施后 `-- --baseline before-MS08-T02` 对比。bench 直连构造器，预取自然生效。
- WSL2 噪声带：Iteration 000 已证明 ms 级条目噪声 ±5-18%；data_scan 条目（1K/10K 行）量级更大，若改善不可辨则以 strace 结构证据（可用 `-e trace=pread64` 计数验证预取确实提前了读）+ 等价性收尾，结论如实记录。

**Behavioral Change**

- 当前：页链逐页串行访问，页耗尽后同步加载下一页。
- 目标：页耗尽跳转时对下一页发起后台预取（spawn，结果/错误丢弃），下一页的真实读取命中缓存或在途加载；行序/结果/可见性/错误语义零变化。
- 接口：`DataScanExecutor::new` 签名不变（推荐方案）；新增 `with_prefetch(bool)`（或等价 builder/字段，非实质）。`PhysicalPlan::DataScan`/pipeline/SQL 层零变化。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R3/预取下全表扫描行序与结果等价、R3/链尾页不发起无效预取 | `tests/prefetch_test.rs`（新增） | 不存在 | 等价对照（开/关预取逐行相等）+ 谓词+LIMIT 组合 + 链尾/空表 |
| T2 | R3 全部场景 | `src/executor/data_scan.rs::{DataScanExecutor, next, with_prefetch}` | 串行逐页访问 | `JumpToPage` 分支 spawn 预取 next 页（next≠0），在途 ≤1，结果/错误丢弃 |
| T2 | 构造点同步 | `src/pipeline.rs:446`、`benches/data_scan_bench.rs:70-74`（+编译器揭示的其他点） | 传 5 参 | 签名不变则零改动；若 Act 选改签名则同步全部构造点 |
| T3 | R3/预取与并发 miss 共存、MS08 纪律 | 命令行验证 | 无 after 证据 | `before-MS08-T02` 基线 + 全量回归 + bench 对比 + strace 预取生效证据 |
| T4 | 质量门 | 命令行 | — | clippy/fmt/validate + Act Response 收尾 |

**Task Contracts**

### T1: TDD — 预取行为测试

- Requirement/Scenario: R3/预取下全表扫描行序与结果等价、R3/预取不破坏谓词下推与 LIMIT 语义、R3/链尾页不发起无效预取
- Depends on: None（可与 T3 的 before 基线采集并行）
- Targets: `tests/prefetch_test.rs`（新增）
- Current behavior: 无预取测试
- Required behavior: 测试文件存在且覆盖：多页数据集（≥3 数据页，如 2000 行）上开启预取与关闭预取的全表扫描逐行相等；带 WHERE 谓词与 LIMIT（含 limit=0）的扫描在预取开启时结果与关闭时一致；空表/单页表扫描正常完成（链尾路径）
- Required changes: 只新建测试，不改生产代码；对照机制经 `with_prefetch(false)`（若 T2 尚未提供该 API，测试先按目标 API 编写——RED 为编译失败也算有效 RED，或在契约内先用最小桩；推荐先写测试再实现 API，见 TDD 铁律"测试定义期望"）
- Preserve: 不修改既有测试文件
- Forbidden: 为使测试通过而改变等价语义（对照必须是真实无预取路径，不是 mock）
- Test witness: `cargo test --test prefetch_test` — 实施前运行记录初始状态（预取未实现时：对照测试预期 GREEN——两侧都是无预取路径；这是回归守卫定位，记录之）
- GREEN condition: T2 完成后 `cargo test --test prefetch_test` 全绿
- Verification: 命令退出码 0
- Stop when: 等价语义无法用公开 API 表达（需要破坏性接口变化）→ 返回 Plan

### T2: DataScanExecutor 预取实现

- Requirement/Scenario: R3 全部 5 场景（等价/谓词+LIMIT/链尾/并发共存/错误丢弃）
- Depends on: T1（测试已定义期望）
- Targets: `src/executor/data_scan.rs`（`DataScanExecutor` 结构体 + `next()` 的 `JumpToPage` 分支 + `with_prefetch` 或等价开关）
- Current behavior: `JumpToPage` 分支只更新游标（`data_scan.rs:313-317`），下一页在下一轮 `with_page_data` 同步加载
- Required behavior: `JumpToPage(next)` 且 `next != 0` 时，spawn 一个丢弃结果的 `buffer_pool.get_page(PageId(next))` 任务；在途任务 ≤1（take 旧 handle 再发新）；预取开关存在且 `new` 默认开启、`with_prefetch(false)` 关闭；链尾（next==0）与 `Done` 不预取
- Required changes: 新增预取控制字段与 spawn 逻辑；构造点保持兼容（`new` 签名不变则 pipeline/bench 零改动，Act 须以编译器输出核对全部构造点，包括 `correlated.rs` 若其直接构造）
- Preserve: 行处理路径（closure 内逻辑）零改动；`filter_row`/`yield_capped`/`find_visible_in_chain`/MVCC 快照判定零改动；`BufferPool` 零改动；`PhysicalPlan::DataScan`/planner 零改动；`tests/file_storage_io_test.rs` 保持 GREEN
- Forbidden: 预取深度 >1；改 BufferPool/淘汰策略；把预取结果直接返回给扫描（绕过 `with_page_data`）；IndexScan 系列执行器
- Test witness: `cargo test --test prefetch_test`（T1 测试转 GREEN）；`cargo test --test pushdown_test`（15 测试零修改全绿）
- GREEN condition: prefetch_test + pushdown_test + `cargo test --all` 全绿；`cargo build` 0 warning
- Verification: `cargo test --all` 0 failures（≥581 + 新增）
- Stop when: 预取与 loading_locks 产生死锁（get_page 在 spawn 任务里 await loading_lock，真实读取也 await 同锁——若发生说明并发假设错误）→ 立即停止并记录 Blocker；或等价测试失败且原因在可见性/行序语义 → 返回 Plan

### T3: 全量回归 + after 证据

- Requirement/Scenario: R3/预取与并发 miss 共存（全量并发测试面）+ MS08 纪律（对比结论）
- Depends on: T2
- Targets: 命令行验证（无代码改动，除非回归失败需在契约内修复）
- Current behavior: 预取已实现，未验证全局影响与量化收益
- Required behavior: `cargo test --all` 0 failures（≥581+新增，既有零修改）；`before-MS08-T02` 基线已在本 Iteration 开始时落盘；`cargo bench --bench data_scan_bench -- --baseline before-MS08-T02` 对比结论（改善或明确未达预期，WSL2 标注）；可选 strace 佐证预取提前读（如 pread64 计数结构对比）
- Required changes: 无新改动
- Preserve: 既有测试零修改
- Forbidden: 为通过回归修改既有测试或弱化新测试
- Test witness: 全量测试 + bench 输出
- GREEN condition: 全部命令退出码 0，结论成文入 Act Response
- Verification: Act Response 记录命令、≤20 行决定性输出、退出码、结论
- Stop when: 并发测试（如 concurrent_test）出现非确定性失败且定位到预取 → 记录并返回 Plan

### T4: 质量门 + validate + 交付

- Requirement/Scenario: change 级验证边界（tasks.md Iteration 001 Verification boundary）
- Depends on: T3
- Targets: 命令行验证
- Current behavior: —
- Required behavior: `cargo clippy --all-targets -- -D warnings` 0 warning；`cargo fmt --check` 0 diff；`openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch` PASS；tasks.md Iteration 001 复选框勾选；Act Response 完整（含 Experience Candidates 评估）
- Required changes: 无代码改动
- Preserve: —
- Forbidden: 跳过质量门
- Test witness: 三命令输出
- GREEN condition: 全部退出码 0
- Verification: Act Response 记录
- Stop when: 质量门失败且修复超出本 Iteration 范围 → 返回 Plan

**Invariants**

- 行序、查询结果、MVCC 可见性判定、错误语义与无预取完全一致
- `DataScanExecutor::new` 生产调用路径（pipeline）行为与签名兼容（推荐签名不变）
- `BufferPool`、`AsyncStorage`、磁盘格式零改动
- Iteration 000 成果不回退（file_storage_io_test 4 测试 GREEN；pread/pwrite 路径不变）
- 预取在途 ≤1；链尾 `PageId(0)` 永不被预取
- 既有测试零修改通过

**Non-goals**

- T03 writev、T04 RowLock、T05 Varint、T06 fsync；IndexScan/IndexScanAll/ScanExecutor 预取；预取深度 >1；BufferPool/淘汰策略改动；prefetch 配置化（公开配置项）

**Acceptance**

| 条件 | 映射 |
|---|---|
| `JumpToPage` 分支 spawn 预取 next≠0 页（代码事实） | R3 ← T2 |
| 开/关预取全表扫描逐行等价（含谓词+LIMIT、limit=0） | R3/等价 + 谓词+LIMIT ← T1/T2 |
| 链尾/空表不预取 PageId(0)，扫描正常结束 | R3/链尾 ← T1/T2 |
| 预取在途 ≤1，spawn 任务错误丢弃不 panic | R3/共存 + 错误丢弃 ← T2 |
| pushdown_test 15 + 既有 581 零修改全绿 | R3 等价守卫 + R1/R2 不回退 ← T2/T3 |
| before-MS08-T02 基线落盘 + 对比结论成文 | MS08 纪律 ← T3 |
| clippy/fmt/validate 全 0 | 验证边界 ← T4 |

**Verification**

`cargo build`（0 warning）；`cargo clippy --all-targets -- -D warnings`（0 warning）；`cargo fmt --check`（0 diff）；`cargo test --test prefetch_test`（全绿）；`cargo test --test pushdown_test`（15 全绿）；`cargo test --all`（0 failures，≥581+新增）；`cargo bench --bench data_scan_bench -- --save-baseline before-MS08-T02`（T3 开始前）+ `-- --baseline before-MS08-T02`（对比）；`openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch`。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | data_scan.rs 全文（325 行）、构造点（pipeline.rs:446 + bench + correlated.rs 消费方式）、BufferPool get_page/loading_locks/evict 逐行、tokio flavor 分布（306 默认 current_thread）、测试入口全部读取；预取触发点唯一可行口径已推导（closure 已知 next 时 spawn） |
| Design | PASS | design.md §目标行为 T02 + 本 Context Implementation Guidance（触发点、在途控制、开关注入、bench 口径、噪声带对策）；错误/等价/并发语义闭合 |
| Iteration Plan | PASS | tasks.md Iteration 001 四任务依赖有序（T1 期望定义 → T2 实现 → T3 证据 → T4 质量门）；与 000 不同验证域（行为等价+重叠收益），平衡审计通过 |
| Cycle Scope | PASS | initial；R3 五场景 + 000 不回退约束；排除项明确 |
| Task Contracts | PASS | 四份契约含目标符号、行为差异、Preserve/Forbidden、测试见证、GREEN、停止条件（含死锁停止条件）；Act 无需回读外部文档 |
| Traceability | PASS | RTM 见下 |
| Verification | PASS | 命令与通过条件明确；直接观察目标行为（逐行等价、零修改回归、bench/strace）；无身份型证据工程 |

**RTM**

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R3 DataScan 链预取 | 预取下全表扫描行序与结果等价 | design §目标行为 T02 | T1/T2 | 001 | `data_scan.rs::next` JumpToPage 分支 | `prefetch_test` 开/关对照逐行相等 | None | Covered |
| R3 | 预取不破坏谓词下推与 LIMIT 语义 | design §目标行为 T02 | T1/T2 | 001 | 同上（行处理路径不动） | `prefetch_test` 谓词+LIMIT 用例 + `pushdown_test` 15 零修改 | None | Covered |
| R3 | 链尾页不发起无效预取 | design §目标行为 T02 | T1/T2 | 001 | JumpToPage next==0 分支 | `prefetch_test` 空表/单页用例 | None | Covered |
| R3 | 预取与并发 miss 共存不饥饿 | design §关键技术选择 3 | T2/T3 | 001 | spawn 预取 + miss_sem 共享 | `cargo test --all` 并发面 + 在途 ≤1 代码事实 | None | Covered |
| R3 | 预取任务错误被丢弃且不影响正确性 | design §目标行为 T02 | T2 | 001 | spawn 任务 `let _ =` | `prefetch_test` + 既有错误路径测试零修改 | None | Covered |
| R3 | MS08 纪律（baseline + 结论） | tasks.md 验证边界 | T3 | 001 | 命令行 | `before-MS08-T02` 基线 + bench 对比入 Act Response | None | Covered |

**Persisted Evidence**

- Mode: none

Act Response 承载：等价测试结果、全量回归、bench 对比结论（≤20 行/项）。criterion 基线由 criterion 落盘 `target/criterion`。全部验证可低成本重跑，摘要不丢失决定性结构。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件（change 已用 0）；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- bench 改善可能不可辨（中）：数据页在 bench 的 setup 阶段已大量进缓存，miss 次数有限——重叠收益依赖真实 miss；对策：结论如实记录"未达预期"（MS08 纪律允许），或以 strace/容量受限场景佐证；不为此扩大范围。
- current_thread 测试里 spawn 任务不被调度（低风险，已设计规避）：预取不承担正确性职责，真实读取路径独立于预取完成。
- `correlated.rs` 若直接构造 DataScanExecutor（未逐行确认）：编译器会在 T2 暴露，Act 按编译错误同步该点即可（契约已授权）。
- 非实质选择留给 Act：开关的形态（builder/字段/`#[cfg(test)]`）、字段与方法命名、handle 管理细节。
- 死锁停止条件（T2 contract）：spawn 任务 await loading_lock 与真实读取 await 同锁理论上无死锁（loading_lock 是短暂持有的 per-page Mutex，非跨 await 长持），但若实测出现挂起立即停止。

## Act Response

- Status: reported

**Implemented**

用户 2026-09-05 授权（"更改gate状态，开始实施吧"）将 Plan Context 置为 `ready` 后，完成 Iteration 001 全部任务。

- **T1 TDD 预取行为测试**：新建 `tests/prefetch_test.rs`（3 测试 + 4 helper）。多页等价（2000 行、BufferPool 容量 8 制造 miss/驱逐压力、链页数 ≥3 前提守卫、开/关逐行相等 + 全量内容守卫）；谓词+LIMIT 组合（复用项目既有 `ComparisonPredicate`/`ColumnExpression`/`ConstantExpression` 构造 `id > threshold`，5 组用例含 limit=0、谓词全过滤、谓词+cap 组合，逐组开/关相等 + 期望行数核对）；链尾路径（空表扫描正常结束、单页表页数守卫 + 行序等价）。对照路径按 Plan 推荐方案实现为 `with_prefetch(false)`（真实无预取路径，非 mock）。初始状态观察：**编译 RED**（`E0599: no method named with_prefetch`）——契约允许"RED 为编译失败也算有效 RED"。
- **T3 前置基线**（实现前落盘）：`cargo bench --bench data_scan_bench -- --save-baseline before-MS08-T02` exit 0，4 条目（data_scan/1000 ≈ 164.30 µs、data_scan/10000 ≈ 7.60 ms、scan_via_index/1000 ≈ 255.10 µs、scan_via_index/10000 ≈ 16.77 ms）。
- **T2 预取实现**（`src/executor/data_scan.rs`）：结构体新增 `prefetch_enabled: bool` / `prefetch_handle: Option<tokio::task::JoinHandle<()>>` / `prefetched_page: Option<PageId>` 三字段；`new` 默认开启、`with_prefetch(bool)` builder 覆盖（`new` 签名不变，pipeline/benches/executor_test 全部构造点零改动）；私有 `trigger_prefetch(next)`：去重（`prefetched_page == Some(next)` 跳过）→ take 旧 handle（不 abort，任务自然完成装入缓存）→ `tokio::spawn` 执行 `let _ = buffer_pool.get_page(next).await`（结果/错误丢弃）。触发点：页数据 closure 顶部捕获 `header().next_page_id` 到 `captured_next`（原两个分支的局部 header 读提升为共享一次，等价重构），`.await` 返回并提交 slot_index 后 `captured_next != 0` 时触发。链尾 `next == 0` 与 `Done` 永不触发；每页至多一次 spawn，未 take 的在途 handle ≤1。
- **T3 回归 + after 证据**：`cargo test --all` 584 passed / 0 failed（581 既有零修改 + 3 新增）；pushdown_test 15/15、executor_test 39/39 零修改全绿；bench 对比出现**显著回退**（结论与机制见 Verification Evidence / Remaining Issues）；按 Plan Risks 对策如实记录，未扩大范围。
- **T4 质量门**：clippy `--all-targets -D warnings` 0 warning；fmt 首查 `tests/prefetch_test.rs` 有 diff，`cargo fmt` 修复后 0 diff（测试复验仍 3/3 GREEN）；`openspec validate` PASS。

**Changed Files and Symbols**

- `src/executor/data_scan.rs`（+56/-3）— `DataScanExecutor`（3 个私有预取字段）、`DataScanExecutor::new`（字段初始化）、`DataScanExecutor::with_prefetch`（新增 pub builder）、`DataScanExecutor::trigger_prefetch`（新增私有方法）、`Executor::next`（closure 顶部 successor 捕获 + 两分支 header 读提升 + post-await 触发）。
- `tests/prefetch_test.rs`（新增，3 测试）— `setup_table` / `count_chain_pages` / `scan_ids` / `id_above` helper + 多页等价 / 谓词+LIMIT 组合 / 链尾路径。
- change 内产物：`iterations/001-prefetch/000-initial.md`（Plan Context ready + 本 Act Response）、change `tasks.md`（Iteration 001 复选框）。

**Deviations from Plan**

全部为非实质等价调整，未改变行为面与 Acceptance：

1. **触发点口径**：契约 Required behavior 字面为"`JumpToPage(next)` 且 `next != 0` 时 spawn"；实际实现为 post-await 去重触发点（closure 捕获 successor，await 返回后 spawn）。依据是 Plan Context Implementation Guidance"正确口径"原句（"当 closure 已拿到 next_page_id 时，立即在 await 返回后对 next 页 spawn 预取，让 next 页加载与 current 页剩余行处理重叠"——契约字面口径的自身分析指出进入新页前 spawn 同页预取"收益为零"）与 tasks.md T2.1（"读取**新当前页**的 next_page_id（需在页数据 closure 内捕获），对下一页 spawn"）。JumpToPage 分支同样被覆盖：跳转目标在前一页首槽处理时已被预取，跳转时去重为 no-op——重叠窗口严格大于字面口径。契约全部不变量（next≠0、结果/错误丢弃、在途 ≤1、链尾不预取、开关）逐条满足。
2. **header 读提升**：closure 内两处分支局部 `slotted.header().next_page_id` 提升为 closure 顶部一次读取并复用（等价表达式重构，为捕获所需）。
3. **T1.3 初始状态与预期不同**：契约预期"等价测试初始全 GREEN（回归守卫定位）"，实测为编译 RED（`with_prefetch` 不存在）——契约明示"RED 为编译失败也算有效 RED"，按实际状态记录。
4. **bench 结论方向为回退**（Plan Risks 预判"改善可能不可辨"，实测更差为显著回退）：按 Plan 对策原文"结论如实记录'未达预期'……不为此扩大范围"处理；keep/gate/revert 决策上交 Plan Review（见 Remaining Issues）。

**Blocker Handoff**

None

**Blocker Resolution**

None（未发生阻塞）

**Self-Review**

- Plan compliance: PASS
- Full diff reviewed: PASS
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 2（M1/M2，见下，均不阻塞）

全量 diff（`git diff src/executor/data_scan.rs` + 新增 `tests/prefetch_test.rs` 全文）逐行审查：生产改动面仅 data_scan.rs 三字段 + 构造器 + 两个新方法 + closure 捕获/触发；行处理路径（closure 槽位逻辑、`filter_row`/`yield_capped`/`find_visible_in_chain`、MVCC 判定）零改动；BufferPool/WAL/pipeline/planner 零触碰；全部既有构造点（pipeline.rs:446、benches ×5、executor_test ×6）零改动（`new` 签名不变，编译器验证）。测试不为错误原因通过：OFF 路径是 `trigger_prefetch` 早退的真实无预取路径；RED（编译失败）真实；等价断言逐行逐序。Minor findings：

- M1：全页不可见快跳路径上，预取目标即跳转目标，重叠窗口为零（loading_locks 去重使其实际无害）——深度 1 设计的固有边界，记录不返工。
- M2：`scan_cap` 提前封顶（如 limit=1）时，当前页首槽发起的下一页预取成为一次性无效缓存填充（≤1 页、可被正常淘汰、无正确性影响）——记录不返工。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 RED 见证 | `cargo test --test prefetch_test`（实现前） | `error[E0599]: no method named 'with_prefetch' found`（3 处） | PASS（预期 RED 实测复现） |
| T3 前置基线 | `cargo bench --bench data_scan_bench -- --save-baseline before-MS08-T02` | 4 条目落盘（data_scan/1000 164.30 µs；data_scan/10000 7.60 ms） | PASS（exit 0） |
| T2 GREEN | `cargo test --test prefetch_test` | `3 passed; 0 failed`（0.26-0.27s） | PASS（exit 0） |
| 等价守卫（零修改） | `cargo test --test pushdown_test` / `--test executor_test` | `15 passed` / `39 passed` | PASS（exit 0） |
| build | `cargo build` | `Finished dev profile`（0 代码警告；`~/.cargo/config` 弃用提示为环境既有） | PASS |
| 全量回归 T3.1 | `cargo test --all` | `TOTAL passed=584 failed=0`（581 既有零修改 + 3 新增） | PASS（exit 0） |
| bench 对比 T3.2/T3.3 | `cargo bench --bench data_scan_bench -- --baseline before-MS08-T02` | data_scan/1000 **+40.4%**（p<0.05）；data_scan/10000 **+17.9%**（p<0.05）；scan_via_index/1000 -0.72%（p=0.25）；scan_via_index/10000 +0.41%（p=0.69） | PASS（exit 0；结论=DataScan 路径显著回退，详见下） |
| clippy T4 | `cargo clippy --all-targets -- -D warnings` | `Finished`（0 warning） | PASS（exit 0） |
| fmt T4 | `cargo fmt --check` | 首查 `tests/prefetch_test.rs` 有 diff → `cargo fmt` → 0 diff（测试复验 3/3 GREEN） | PASS（exit 0） |
| OpenSpec T3.3/T4 | `openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch` | `Change ... is valid` | PASS（exit 0） |

**bench 对比结论（T3.3，WSL2 同机采集，criterion 自带 p 值与 before-MS08-T02 基线）**

- **结论：DataScan 路径显著回退，明确未达预期。** data_scan/1000：164.30 µs → 231.74 µs（+40.4%，p<0.05）；data_scan/10000：7.60 ms → 8.96 ms（+17.9%，p<0.05）。
- **对照组证明非噪声**：同 bench 同环境下未改动路径 scan_via_index 两条（不经 DataScanExecutor）变化 -0.72% / +0.41%，均 p>0.05——环境稳定，回归严格局限于本次改动的代码路径。
- **机制归因**：`Database` 默认池容量 100 页（`database.rs:31`）。1000 行集 ≈17 页 → 完全暖缓存、稳态零 miss，预取只剩每页一次 `tokio::spawn` + 跨线程 wake/poll 开销（实测折算 ≈4-8 µs/页）而无可隐藏的加载延迟（WSL2 OS 页缓存下 4KB pread64 ≈2-6 µs）→ 纯开销；10000 行集 ≈164 页 > 容量 → 部分冷路径，重叠收益抵消部分 spawn 开销后仍净回退（回退幅度差 40% vs 18% 与该归因自洽）。预取未造成额外 I/O（loading_locks 去重，同一页只加载一次）。
- **strace 未执行的理由**：预取不改变 pread64 计数（只改变时序），计数对比无法支持或反驳本回归；机制是调度开销而非 I/O 结构变化。契约标注 strace 为"可选"，criterion 双条目 p<0.05 + 同 bench 对照组即为决定性量化证据。

**Persisted Evidence**

None required（Plan Context Mode: none；全部验证命令可低成本重跑，Act Response 摘要未丢失决定性结构；未创建 `evidence/` 目录）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook | MS08 对比判读方法：bench 回退的因果判定流程（同套件未改动路径作对照组验证环境稳定 → criterion p 值判定非噪声 → 机制归因需对齐容量/数据规模与成本模型：跨线程 spawn/wake 开销 vs 页缓存读取延迟；strace 计数对"时序型"变化无判别力） | 本 Act Response Verification Evidence 表 bench 对比行 + bench 对比结论段 | 已端到端完成一次真实判定；MS08-T03（writev）及后续各 T 对比时将面临同类判读；含"对照组判定"与"strace 局限"两个非显然结论 |

**Remaining Issues**

无阻塞。核心遗留为**方向性决策，交 Plan Review**：

1. **预取回退的处置**：data_scan/1000 +40.4%、data_scan/10000 +17.9%（均 p<0.05，对照组不变）。机制为 spawn/wake 调度开销 > 本环境（WSL2 + OS 页缓存）可隐藏的加载延迟；BufferPool 无"页已缓存"公开探测 API 且本契约 Forbidden 改 BufferPool，契约内无缓解杠杆。Plan Review 可选：(a) 接受回退、按 MS08 纪律记录"未达预期"并收尾（本 Act Response 已满足全部 GREEN 条件）；(b) 将默认值改为关闭（违反现契约"new 默认开启"，需 replan）；(c) 放弃 spawn 方案改用其他重叠机制（新设计，replan）；(d) 在慢存储或受限容量场景补充评估后再定（本环境冷读也快，改善空间可能天然有限）。
2. M1/M2 两个 Minor finding（全页不可见快跳零重叠窗口、scan_cap 提前封顶的一次性无效预取）不阻塞，深度 1 设计固有，是否处理由 Plan Review 判断。

**Commit or Diff Reference**

未 commit（待用户触发）；工作树在 Iteration 000 基础上新增：`M src/executor/data_scan.rs`、`?? tests/prefetch_test.rs`、change 产物更新（tasks.md 复选框 + 本 Cycle 文件）。

## Plan Review

- Review Result: replan-required

**Findings**

独立检查（不依赖 Act Self-Review）：

1. **实现正确性 PASS**：全量 staged diff（`src/executor/data_scan.rs` +56/-3、`tests/prefetch_test.rs` 205 行）逐行审查。三字段 + `with_prefetch` builder + `trigger_prefetch`（去重 → take 旧 handle → spawn 丢弃结果）；契约不变量逐条成立：`captured_next != 0` 守卫使 `PageId(0)` 永不被预取；未 take 的在途 handle ≤1；行处理路径（closure 槽位逻辑、`filter_row`/`yield_capped`/`find_visible_in_chain`、MVCC 判定）零改动；`new` 签名不变，全部构造点（pipeline.rs:446、benches ×5、executor_test ×6）零改动。链推进语义核对：每页恰一次 spawn（`prefetched_page` 去重，页 id 单调递增无误去重）。
2. **Deviation 1（触发点口径）评估为合理偏差**：契约字面（"JumpToPage 分支 spawn"）与 Implementation Guidance"正确口径"（closure 已知 successor、await 返回后 spawn，重叠窗口 = 当前页剩余行处理）不一致是 Plan 自身的措辞缺陷；实现忠于 Guidance 且重叠窗口严格优于字面口径（进页首槽即预取后继，覆盖后续全部行处理时间）。全不变量满足。非 ACT-DEVIATION。
3. **测试质量 PASS**：OFF 路径是 `trigger_prefetch` 早退的真实无预取路径（非 mock）；容量 8 制造 miss/驱逐压力；`count_chain_pages` 前提守卫防止数据集退化为单页；5 组谓词+LIMIT 用例含 limit=0、全过滤边界；内容守卫（排序后全集相等）防两侧同错。
4. **新鲜复测全部一致**：prefetch_test 3/3、pushdown_test 15/15（零修改）、file_storage_io_test 4/4（Iteration 000 不回退）、`cargo test --all` TOTAL 584/0、clippy 0、fmt 0、`before-MS08-T02` 基线 4 目录实存、池容量 100（`database.rs:31`）核实无误。
5. **性能回退独立复现（决定性发现）**：Review 重新执行 `cargo bench --bench data_scan_bench -- --baseline before-MS08-T02`：`data_scan/1000` **+47.3%**（p=0.00）、`data_scan/10000` **+17.1%**（p=0.00），对照组 `scan_via_index/1000` -0.22%（p=0.74）、`/10000` +1.03%（p=0.48）无变化——与 Act 报告（+40.4%/+17.9%）量级一致。回退真实、可复现、严格局限于改动路径。机制归因核实成立：1000 行集 ≈17 页 < 容量 100 全暖缓存（零 miss 可隐藏 → spawn 纯开销）；10000 行集 ≈164 页 > 容量（部分重叠收益）——回退幅度差与容量边界自洽。
6. **形式 Acceptance 已满足**：等价性（开关两态逐行一致）✓、在途 ≤1 ✓、"对比结论成文" ✓（MS08 纪律明文允许"明确记录未达预期"）。Act 未发生契约违反。
7. **但默认路径回退构成必须处置的产品问题**：R3 原措辞"SHALL 对下一页发起预取"+ 契约"new 默认开启"意味着收尾即把实测 17-47% 回退固化为默认行为，与本 MS"实测驱动性能"目的冲突。Act 已按流程上交方向决策。
8. **用户决策（2026-09-05）**：四个选项（默认改关 / 接受现状 / 全部回退 / 先收尾后评估）中选定**默认改关**——保留预取能力与开关（供慢存储/冷读场景后续评估），`new` 默认 `false`，默认路径 bench 恢复基线。此决策改变 R3 requirement 语义与 Task Contract T2"new 默认开启"，属 replan 范畴。
9. Minor findings（不阻塞，记录）：M1 全页不可见快跳预取零重叠窗口（loading_locks 去重使其无害）；M2 `scan_cap` 提前封顶时的一次性无效预取（≤1 页、可淘汰、无正确性影响）；M3 strace 跳过理由成立（预取改时序不改 IO 计数，无判别力）。

**Deviation Classification**

NEW-EVIDENCE（bench 实测默认路径回退 +47%/+17%，p<0.05，对照组不变——设计假设"预取净收益"在本环境被测量证伪；处置由用户决策驱动 replan）

**Acceptance Gaps**

None（本 Cycle 形式 Acceptance 全部满足：开关两态等价、在途 ≤1、链尾守卫、对比结论成文。回退处置不是本 Cycle 的执行缺口，而是后续 replan 的新验收目标）

**Convergence**

N/A（首次 Review；无既有 gap）

**Evidence**

- staged diff：`src/executor/data_scan.rs`（+56/-3）全文审查；`tests/prefetch_test.rs`（205 行）全文审查
- Review 复测：`cargo test --test prefetch_test`（3 passed）；`--test pushdown_test`（15 passed）；`--test file_storage_io_test`（4 passed）；`cargo test --all`（TOTAL 584/0）；`cargo clippy --all-targets -- -D warnings`（0）；`cargo fmt --check`（0）
- Review 复测 bench：data_scan/1000 +47.3%（p=0.00）、data_scan/10000 +17.1%（p=0.00）、scan_via_index 对照组 p=0.74/0.48 无变化
- `target/criterion/**/before-MS08-T02`（4 目录实存）；`src/database.rs:31`（池容量 100）
- Act Response Verification Evidence 表 + bench 对比结论段 + Remaining Issues（方向决策上交记录）

**Follow-up Decision**

实现正确、验证可复现，但 NEW-EVIDENCE（实测回退）+ 用户决策（默认改关）改变 R3 requirement 语义与执行契约 → replan-required。已按流程完成：spec R3 修订为"可选能力，默认关闭"（新增默认关闭场景）；design.md 补实测修订记录；tasks.md Iteration 001 增补 Task 5（默认翻转 + 默认关闭断言 + 默认路径回基线验证）并更新验收映射；创建同 Iteration replan Cycle。M1/M2/M3 记录不返工。

**Iteration Plan Update**

Iteration 001 修订（2026-09-05 replan）：Tasks 由 T1-T4 扩为 T1-T5；Stable baseline 增补"预取默认关闭、with_prefetch(true) 显式启用、默认路径 bench 恢复无变化（p>0.05）"；Verification boundary 增补默认关闭断言与默认路径 bench 验证；R3 验收映射增补 T5。Iteration 000 与既有 T1-T4 产物不动。

**Next Cycle**

`iterations/001-prefetch/001-replan.md`（replan：预取默认改关 + 默认路径恢复基线）

**Next Iteration**

None（Iteration 001 replan Cycle accepted 后本 change 全部交付）
