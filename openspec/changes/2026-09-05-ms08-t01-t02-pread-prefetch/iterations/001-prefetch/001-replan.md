# Iteration 001: T02 DataScan 链预取 / Cycle 001: replan

## Plan Context

- Status: ready
- Iteration: 001-prefetch
- Cycle: 001-replan
- Cycle Type: replan
- Parent cycle: `000-initial.md`

**Iteration Scope**

- Change tasks: T5（预取默认改关 + 默认路径恢复基线）。T1-T4 已由父 Cycle 完成：预取机制、开关、等价测试、bench 证据全部在工作区且验证通过，本 Cycle 不重做。
- Depends on: 父 Cycle 000-initial（预取实现已合入工作区，584 tests 全绿）；Iteration 000（pread/pwrite accepted）
- Stable baseline: `new` 默认不预取；`with_prefetch(true)` 显式启用且开关两态行为等价；默认路径 bench 对 `before-MS08-T02` 基线无可分辨差异（p>0.05）；≥584 tests 全绿
- Verification boundary: 4 项质量命令全绿；`tests/prefetch_test.rs` 全绿（含非空洞的默认关闭断言）；`cargo test --all` 0 failures；默认路径 bench 两档 "No change"（p>0.05，对照组维持）
- Diagnostic boundary: `src/executor/data_scan.rs`（`new` 默认值 + 模块内单测）、`tests/prefetch_test.rs`、bench 判读
- Deferred tasks: 无（本 Cycle accepted 后 Iteration 001 完成，change 全部交付）

**Cycle Scope**

- Trigger: replan-required
- Acceptance gaps: 无既有执行缺口（父 Cycle 形式 Acceptance 全部满足：开关两态等价、在途 ≤1、链尾守卫、对比结论成文）。本 Cycle 执行修订后的计划目标：默认路径不再承担实测回退
- Repair items: None（replan 使用修订后的全局 Task 5，不建 rework repair item）
- Inherited scope: R3 修订版（可选能力，默认关闭）全部场景；R1/R2 不回退（`tests/file_storage_io_test.rs` 4 测试保持 GREEN）
- Excluded scope: 预取机制重设计（spawn 方案保留）；慢存储/受限容量场景的预取收益评估（后续 milestone 事项，本 change 只留开关）；`BufferPool::evict_one` 锁范围改造（见 Current-State Evidence 次要发现）；T03-T06；spec/design/proposal 之外的文档同步（SNAPSHOT/tasks 属 docs-maintainer）

**Objective**

`DataScanExecutor::new` 默认 `prefetch_enabled = false`（一行翻转）；预取经 `with_prefetch(true)` 显式启用、启用路径行为与父 Cycle 完全一致；默认关闭有非空洞性测试断言；默认路径 bench 恢复与 `before-MS08-T02` 基线无可分辨差异。

**Background**

父 Cycle 按原计划（R3 原文"SHALL 对下一页发起预取"、Task Contract"new 默认开启"）交付了预取：实现经 Review 全量 diff 审查正确（不变量逐条成立），等价性测试（开关两态逐行一致）全绿。但 bench 对比显示默认路径显著回退，且经三轮独立验证与根因诊断，结论收敛如下：

**实测结果**：`data_scan/1000` +40~47%、`data_scan/10000` +17~18%（均 p<0.05；Act 采集与 Plan Review 复测两次独立运行一致）。同套件对照组 `scan_via_index` 两档 p=0.74/0.48 无变化——回退真实、可复现、严格局限于本次改动路径。

**根因诊断（Plan Review 应用户要求专项验证，排除实现错误阻塞）**：

1. **零可阻塞对象配置下回退最大**：strace 显示 1000 行档测量期 `pread64` 仅 554 次且全部来自 setup 建表（数据 17 页 < 池容量 100，纯暖缓存、稳态零 miss）。测量期每个预取任务的全部工作是 task 分配 → 一次 DashMap 命中读 → 销毁，碰不到 IO、miss 信号量、loading lock——不存在可阻塞的对象，而该档回退反而最大（+47%）。折算 ≈3.9µs/页 × 17 页 ≈ +67µs，与实测 164→232µs 吻合。成本只能是 task 生命周期本身（用户态分配、无锁调度队列操作、DashMap 分片缓存行弹跳）。
2. **syscall 层无差异**：对照实验（同进程、同池、同运行时，对照组不经 DataScanExecutor 无 spawn）：futex 88,627 vs 87,957、write 11,226 vs 11,100、pwrite64 372 vs 372——无可分辨差异。排除"每页 spawn 触发跨线程 eventfd 唤醒/系统调用阻塞"类机制。
3. **加性成本模型同洽两档**：1000 档 9.7µs/页基数 + 3.9µs/页 spawn 开销 → +40~47%；10000 档 164 页 > 容量 100，46µs/页基数（miss + 淘汰写回）+ ~8.3µs/页（spawn 4µs + miss 路径协调开销 − 隐藏的 ~2-3µs 页缓存读）→ +17~18%。若是锁护航类 bug，签名应为方差爆炸、挂起或超线性增长；实测为稳定线性、584 测试零挂起、OFF 路径回基线速度。
4. **环境边界**：WSL2 下文件驻留 OS 页缓存，4KB pread64 ≈2-3µs 内存拷贝——预取设计要隐藏的"磁盘延迟"在本环境不存在（真冷读 HDD 为 ms 级，8µs spawn 换 ms 级隐藏即净赚）。负结果是本环境的准确实测，不构成对设计在慢存储场景的证伪。

**用户决策（2026-09-05，四选一）**：默认改关。依据是 MS08"实测驱动"纪律的自然结论——测量结果决定默认值；预取能力完整保留为显式开关，供慢存储/冷读场景后续评估。此决策改变 R3 requirement 语义与父 Cycle Task Contract"new 默认开启"，故走 replan（非 rework）。

**Current Baseline**

- 工作树：Iteration 000 + 001 T1-T4 已实现（staged：`M src/storage/file_storage.rs`、`M src/executor/data_scan.rs`、`A tests/file_storage_io_test.rs`、`A tests/prefetch_test.rs`、change 产物）；HEAD = `4d410ac`，未 commit
- 584 tests pass / 0 failed；clippy/fmt 0
- 预取默认当前为**开启**（`new` 中 `prefetch_enabled: true`）——本 Cycle 的翻转对象
- `before-MS08-T02` 基线 4 条目已落盘（**prefetch 实现前**采集，代表无预取默认路径性能——翻转后可直接对比，不得覆盖重采）

**Current-State Evidence**

- `DataScanExecutor::new`（`src/executor/data_scan.rs` 构造器）：`prefetch_enabled: true` 初始化——翻转目标行
- `with_prefetch(bool)`（同文件，pub builder）：已存在，`true` 路径无需改动；`trigger_prefetch` 的 `!self.prefetch_enabled` 早退使关闭态即真实无预取路径（父 Cycle 已验证）
- `tests/prefetch_test.rs`（3 测试）：全部 ON 路径经 `DataScanExecutor::new(...)` 默认构造——**翻转后若不显式 `.with_prefetch(true)`，等价断言退化为 "off vs off" 恒真（空转）**，这是本 Cycle 测试改动的核心点
- 项目内单测先例：`src/storage/catalog.rs` 10 个 `#[cfg(test)]` 单测、planner 模块单测——`data_scan.rs` 增加同形态模块内单测符合项目惯例（私有字段模块内可见，可直接断言默认值）
- bench：`data_scan_bench` 经 `new` 构造（度量默认路径）——翻转后与 `before-MS08-T02`（prefetch 实现前采集）同口径可比
- 次要发现（既有热点，非本次引入，本 Cycle 明确不做）：`BufferPool::evict_one`（`src/storage/buffer_pool.rs:156`）在淘汰循环全程持有 `clock_hand` tokio 写锁并跨越脏页写回 `.await`（`buffer_pool.rs:196`），而 `get_page` miss 路径也取同一写锁（`buffer_pool.rs:121`）——预取的第二股 miss 流与真实读在该锁互等，解释 10000 档每页成本 4→8µs 的次要部分。单流 miss 时无竞争（既有行为），属 BufferPool 改动，超出本 change 责任边界；I 候选登记待用户授权 docs-maintainer

**Relevant Code**

- `src/executor/data_scan.rs` — `DataScanExecutor::new`（默认值翻转 + `#[cfg(test)]` 默认关闭单测）
- `tests/prefetch_test.rs` — ON 路径显式化
- `openspec/changes/2026-09-05-ms08-t01-t02-pread-prefetch/proposal.md` — What Changes 段补默认关闭说明（与修订后 R3 一致）

**Critical Path**

`new` 默认值翻转 → `trigger_prefetch` 早退（关闭态零 spawn）→ `next()` 行为与父 Cycle OFF 路径完全一致 → bench 默认路径回到基线。无新数据流、无新状态。

**Implementation Guidance**

- 顺序（TDD）：先改测试——prefetch_test 的 ON 路径显式 `.with_prefetch(true)` + 新增默认关闭断言（模块内单测断言 `new(...).prefetch_enabled == false` 与 `new(...).with_prefetch(true).prefetch_enabled == true`，最简非空洞手段；形态非实质）→ 运行确认默认关闭断言 RED（当前默认 true）→ 翻转 `new` 默认值 → 全 GREEN。
- bench 判读沿父 Cycle 方法：data_scan 两档需 "No change"（p>0.05），对照组 scan_via_index 维持无变化；若默认路径仍 p<0.05 显著变化，说明翻转未生效或存在未知机制，停止并按 Deviation 处理。
- `captured_next` 捕获在关闭态仍执行（每 closure 一次内存读，开销可忽略）——不为微优化触碰行处理路径（Forbidden）。

**Behavioral Change**

- 当前（父 Cycle 后）：`new` 默认开启预取（默认路径实测回退 17-47%）。
- 目标：`new` 默认关闭（默认路径零预取开销，bench 回基线）；`with_prefetch(true)` 显式启用（行为与父 Cycle ON 路径一致）。
- 接口：`new`/`with_prefetch` 签名不变；启用路径行为不变；磁盘格式/BufferPool/WAL 零变化。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T5.1 | R3/默认构造不发起预取 | `src/executor/data_scan.rs::DataScanExecutor::new` | `prefetch_enabled: true` | `prefetch_enabled: false` |
| T5.2 | R3/默认构造不发起预取 + 等价场景维持 | `tests/prefetch_test.rs`、`src/executor/data_scan.rs` `#[cfg(test)]` | ON 路径经默认构造；无默认关闭断言 | ON 路径显式 `.with_prefetch(true)`；新增默认关闭单测 |
| T5.3 | 验证边界 | 命令行 | — | 全量回归 + 质量门 |
| T5.4 | R3/默认构造不发起预取（bench 无变化子句） | 命令行 | — | 默认路径 bench 两档 "No change"（p>0.05） |
| T5.5 | 文档一致性 | `proposal.md` What Changes 段 | 描述默认开启 | 补默认关闭说明 |

**Task Contracts**

### T5: 预取默认改关 + 默认路径恢复基线

- Requirement/Scenario: R3/默认构造不发起预取（含"默认路径与无预取基线无可分辨差异"子句）；R3 等价/谓词+LIMIT/链尾场景维持（ON 路径显式化后继续由同测试守卫）
- Depends on: None（父 Cycle T1-T4 产物已在工作区）
- Targets: `src/executor/data_scan.rs::DataScanExecutor::new`（默认值）、`src/executor/data_scan.rs` `#[cfg(test)]`（新增单测）、`tests/prefetch_test.rs`（ON 路径显式化）、`openspec/changes/2026-09-05-ms08-t01-t02-pread-prefetch/proposal.md`（What Changes 补一句）
- Current behavior: `new` 默认 `prefetch_enabled: true`；prefetch_test ON 路径依赖默认值；无默认关闭断言
- Required behavior: `new` 默认 `false`；`with_prefetch(true)` 路径行为不变；prefetch_test 全部 ON 用例显式 `.with_prefetch(true)` 且全 GREEN；存在能区分"默认关闭"与"显式开启"的非空洞性断言；proposal What Changes 段与默认关闭一致
- Required changes: 先改测试（ON 显式化 + 默认关闭断言）观察 RED，再翻转默认值，后全 GREEN（见 Implementation Guidance 顺序）
- Preserve: `with_prefetch` 签名与语义；`trigger_prefetch` 逻辑；行处理路径；父 Cycle 全部测试语义不变；`tests/file_storage_io_test.rs` GREEN；既有 581 测试零修改
- Forbidden: 重设计预取机制；改 `trigger_prefetch`/closure 捕获逻辑（默认值外的一切行处理路径改动）；删除或弱化父 Cycle 测试；慢存储场景评估；BufferPool/evict_one 改动
- Test witness: 翻转前运行新增默认关闭断言 → 预期 RED（当前默认 true）；翻转后 `cargo test --test prefetch_test` + 模块内单测全 GREEN
- GREEN condition: `cargo test --all` 0 failures（≥584，含新单测）；clippy/fmt/validate 全 0；`cargo bench --bench data_scan_bench -- --baseline before-MS08-T02` 中 data_scan 两档均 "No change"（p>0.05）
- Verification: Act Response 记录命令、≤20 行决定性输出、退出码、bench 判读（含对照组）
- Stop when: 默认路径 bench 仍显著变化（p<0.05）→ 翻转未生效或有未知机制，返回 Plan；或等价测试在 ON 显式化后失败（开关路径被翻转破坏）→ 返回 Plan

**Invariants**

- 开关两态行为与父 Cycle 完全一致（ON = 父 Cycle 默认路径；OFF = 父 Cycle `with_prefetch(false)` 路径）
- `new`/`with_prefetch` 签名不变；行处理路径零改动；BufferPool/WAL/file_storage 零触碰
- 既有 581 测试零修改；`file_storage_io_test` 4 测试 GREEN（R1/R2 不回退）
- `before-MS08-T02` 基线不被覆盖重采（对比有效性依赖它）

**Non-goals**

- 预取机制重设计；慢存储/受限容量场景收益评估（后续 milestone）；`evict_one` 锁范围改造（I 候选另议）；T03-T06；spec/design/proposal 之外的项目文档同步

**Acceptance**

| 条件 | 映射 |
|---|---|
| `new` 默认不预取（非空洞断言 GREEN） | R3/默认构造不发起预取 ← T5.1/T5.2 |
| ON 路径显式化后父 Cycle 等价/谓词+LIMIT/链尾测试全 GREEN | R3 等价场景维持 ← T5.2 |
| `cargo test --all` 0 failures（≥584）+ 质量门全 0 | 验证边界 ← T5.3 |
| 默认路径 bench 两档 "No change"（p>0.05，对照组维持） | R3 bench 无变化子句 ← T5.4 |
| proposal 与默认关闭一致 | 文档一致性 ← T5.5 |

**Verification**

`cargo build`（0 warning）；`cargo clippy --all-targets -- -D warnings`（0 warning）；`cargo fmt --check`（0 diff）；`cargo test --test prefetch_test`（全绿）；`cargo test --all`（0 failures，≥584 含新单测）；`cargo bench --bench data_scan_bench -- --baseline before-MS08-T02`（data_scan 两档 p>0.05 "No change"，对照组维持）；`openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch`。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 父 Cycle diff 全文已审（Review）；`new`/`with_prefetch`/`trigger_prefetch` 当前实现已读；prefetch_test ON 路径构造方式已核（3 处 `new` 构造）；回退根因已诊断（Background 四点：零阻塞对象配置 + syscall 对照 + 成本模型 + 环境边界）；bench 基线可比性已核（`before-MS08-T02` = prefetch 实现前采集） |
| Design | PASS | R3 修订版（spec 已更新）+ design.md 实测修订记录；翻转语义、测试非空洞要求、bench 判读标准闭合；`evict_one` 次要发现已划出范围 |
| Iteration Plan | PASS | tasks.md 已修订（Task 5 + Stable baseline/Verification boundary 更新）；单任务 replan，工作量适中 |
| Cycle Scope | PASS | replan；范围 = Task 5；继承 R1/R2/R3 不回退约束；排除项明确（含 evict_one） |
| Task Contracts | PASS | T5 单契约含目标符号、行为差异、TDD 顺序（先断言 RED 后翻转）、非空洞断言要求、Preserve/Forbidden、停止条件 |
| Traceability | PASS | RTM 见下 |
| Verification | PASS | 命令与通过条件明确（bench p 值判读标准写死）；直接观察目标行为（默认关闭断言、等价测试、bench 无变化） |

**RTM**

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R3（修订：可选能力默认关闭） | 默认构造不发起预取 | design §目标行为 T02 修订段 | T5 | 001 | `data_scan.rs::new` | 模块内默认值单测 + bench p>0.05 | None | Covered |
| R3 | 预取下全表扫描行序与结果等价（维持） | design §目标行为 T02 | T5（ON 显式化） | 001 | `data_scan.rs::with_prefetch` | `prefetch_test` 等价用例（ON 显式） | None | Covered |
| R3 | 预取不破坏谓词下推与 LIMIT 语义（维持） | 同上 | T5 | 001 | 同上 | `prefetch_test` 谓词+LIMIT 用例 + `pushdown_test` 15 零修改 | None | Covered |
| R3 | 链尾页不发起无效预取（维持） | 同上 | T5 | 001 | 同上 | `prefetch_test` 链尾用例 | None | Covered |
| R3 | 预取与并发 miss 共存 / 错误丢弃（维持） | design §关键技术选择 3 | 父 Cycle 已交付，本 Cycle 不回退 | 001 | `trigger_prefetch` | `cargo test --all` 并发面 | None | Covered |

**Persisted Evidence**

- Mode: none

Act Response 承载：翻转前后测试状态（RED→GREEN）、全量回归、bench 判读（p 值 + 对照组）。全部可低成本重跑。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件（change 已用 0）；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- bench "No change" 判读受 WSL2 噪声影响（低）：父 Cycle 对照组经验显示环境稳定（p>0.05 一致）；若遇边缘情况（如 p=0.04），结合幅度与方向判断——幅度 <2% 且无方向一致性可判读为噪声并如实记录。
- `captured_next` 在关闭态仍执行（可忽略开销）：不为微优化触碰行处理路径（Forbidden）。
- 非实质选择留给 Act：单测具体形态（模块内字段断言 vs 其他可观察手段）、proposal 补句措辞。

## Act Response

- Status: reported

**Implemented**

按 Task Contract T5 以 TDD 顺序完成预取默认改关：

1. T5.2 先改测试：`tests/prefetch_test.rs` 全部 4 处 ON 路径构造显式追加 `.with_prefetch(true)`（multipage / predicate+LIMIT 循环 / 链尾空表 / 链尾单页；OFF 路径 `.with_prefetch(false)` 保持不变），模块头注释补默认关闭说明；`src/executor/data_scan.rs` 新增 `#[cfg(test)] mod tests` 单测 `new_defaults_prefetch_off`，断言 `new(...).prefetch_enabled == false` 且 `new(...).with_prefetch(true).prefetch_enabled == true`（直接断言私有字段值，非空洞性区分默认与显式开启；空表栈复用 tempfile + FileStorage + BufferPool + TableManager 项目惯例）。
2. 观察 RED：翻转前运行单测，`panicked: new() must default to prefetch disabled`（当前默认 true，预期失败成立）。
3. T5.1 翻转 `DataScanExecutor::new` 构造器 `prefetch_enabled: true → false`（一行），同步修正字段与 `with_prefetch` 的 doc comment（原"default on"表述已过期）。`trigger_prefetch`/closure/行处理路径零逻辑改动。
4. T5.5 `proposal.md` What Changes 段新增"预取默认关闭（replan 2026-09-05，用户决策）"要点（实测回退数据 + 默认值依据 + 能力保留说明）。

**Changed Files and Symbols**

- `src/executor/data_scan.rs` — `DataScanExecutor` 字段 doc（prefetch_enabled）、`DataScanExecutor::new`（默认值翻转）、`with_prefetch` doc、新增 `#[cfg(test)] mod tests::new_defaults_prefetch_off`
- `tests/prefetch_test.rs` — 4 处 ON 构造显式 `.with_prefetch(true)`、模块头注释
- `openspec/changes/.../proposal.md` — What Changes 段补默认关闭说明
- `openspec/changes/.../tasks.md` — Task 5 各项勾选（5.4 附判读注记）
- `openspec/changes/.../iterations/001-prefetch/001-replan.md` — Plan Context draft→ready（用户批准）；本 Act Response

**Deviations from Plan**

- **T5.4 bench 判读偏离 GREEN 条款字面（环境侧偏差，非代码问题，提请 Plan Review 裁定）**：契约要求 data_scan 两档均 "No change"（p>0.05）。实测：`data_scan/1000` 两轮均 No change（p=0.66 / 0.61）✅；`data_scan/10000` 两轮均 p<0.05 但方向为**改善**（-3.81% / -3.38%）。判读为会话环境漂移而非本 diff 效应，依据三点：① 机制不可能——OFF 路径每页工作量 ≥ 基线代码（多 captured_next 捕获 + 早退检查），本 diff 不可能造成真实加速，且被守卫的回退签名（+17~18%）已消失；② 对照组同幅漂移——`scan_via_index/1000` 两轮均 p<0.05（-1.57% / -1.97%）、`scan_via_index/10000` 首轮 +4.45%（p=0.00）次轮 +1.36%（p=0.26）不复现，对照组不经过 DataScanExecutor，其漂移直接证明环境相对基线采集时存在数个百分点漂移；③ 方向有利——绝对值 7.31/7.34ms 低于基线约 7.6ms，回退（父 Cycle 实测 ~8.9ms） definitively 消除。`before-MS08-T02` 基线未被覆盖（仅 `--baseline` 对比，未 `--save-baseline`）。
- 无其他偏差。单测形态、proposal 措辞为契约留给 Act 的非实质选择。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS
- Full diff reviewed: PASS（完整 diff 逐行审查：仅计划内文件与符号；行处理路径零逻辑改动；`with_prefetch` 签名与语义不变；prefetch_test 等价断言体未动，仅构造显式化）
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1

唯一 Minor：T5.4 bench 判读的环境侧偏差（见 Deviations），已按契约风险条款"结合幅度与方向判断并如实记录"处理；不涉及代码正确性，待 Plan Review 裁定是否接受判读。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| RED 见证（翻转前） | `cargo test --lib executor::data_scan::tests::new_defaults_prefetch_off` | `panicked at src/executor/data_scan.rs:406: new() must default to prefetch disabled` | RED 如期 |
| 单测 GREEN（翻转后） | 同上 | `test executor::data_scan::tests::new_defaults_prefetch_off ... ok`（1 passed; 0 failed） | GREEN |
| 预取等价测试 | `cargo test --test prefetch_test` | `3 passed; 0 failed`（equivalence / predicate+LIMIT / chain_tail） | GREEN |
| 全量回归 | `cargo test --all` | 各套件全 `ok`，合计 **585 passed; 0 failed**（含 pushdown 15、file_storage_io 4、prefetch 3、新单测 1），退出码 0 | PASS |
| 构建/静态 | `cargo build`；`cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | build 仅 `~/.cargo/config` 弃用提示（用户配置，非代码）；clippy `Finished` 退出码 0；fmt `FMT_CLEAN` | PASS |
| OpenSpec | `openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch` | `Change ... is valid` | PASS |
| bench 对比（第 1 轮） | `cargo bench --bench data_scan_bench -- --baseline before-MS08-T02` | `data_scan/1000 change [-1.38%, -0.27%, +0.94%] (p=0.66) No change`；`data_scan/10000 change [-5.36%, -3.81%, -2.28%] (p=0.00) improved`；对照组 `scan_via_index/1000 (p=0.01)`、`/10000 +4.45% (p=0.00)` | 1000 档达标；10000 档+对照组见 Deviations |
| bench 对比（第 2 轮复测） | 同上 | `data_scan/1000 (p=0.61) No change`；`data_scan/10000 [-5.01%, -3.38%, -1.82%] (p=0.00) improved`；`scan_via_index/10000 (p=0.26) No change` | 偏差可复现且方向一致，判读见 Deviations |

**Persisted Evidence**

None required（Plan 设 none；全部验证可低成本重跑）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | — | 无故障、无可复用高风险操作路径；bench 判读方法已沉淀于本 Cycle Plan Context 与 Deviations |

**Remaining Issues**

1. T5.4 bench 判读偏差提请 Plan Review 裁定（唯一 Acceptance 字面缺口，环境侧、有利方向、机制排除，详见 Deviations）。
2. `BufferPool::evict_one` 锁范围跨越脏页写回的既有热点（本 change 划出范围）——I 候选登记仍待用户授权 docs-maintainer。

**Commit or Diff Reference**

未提交（沿用户惯例由用户 commit）。工作区 = 父 Cycle staged 产物 + 本 Cycle 未暂存增量（5 文件，+67/−15）；HEAD = `4d410ac`。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（不依赖 Act Self-Review）：

1. **代码事实核对 PASS**（`src/executor/data_scan.rs` 全文 421 行 + `tests/prefetch_test.rs` 全文 211 行逐行审查）：`new` 默认 `prefetch_enabled: false`（`data_scan.rs:95`，一行翻转）；字段与 `with_prefetch` doc comment 同步修订为 default-off 表述；`trigger_prefetch`（`!prefetch_enabled` 早退 + 去重 + take handle + spawn）与 closure 捕获逻辑零改动；`filter_row`/`yield_capped`/`find_visible_in_chain`/MVCC 判定/JumpToPage/Done 臂全部未触碰——符合 Forbidden 边界"默认值外的一切行处理路径改动"。BufferPool/WAL/file_storage 本 Cycle 零触碰。
2. **非空洞断言与等价守卫 PASS**：单测 `new_defaults_prefetch_off` 同断言默认 `false` 与显式 `true` 两态（直接断言私有字段，可区分默认与显式开启；tempdir + FileStorage + BufferPool + TableManager 符合项目单测惯例）。`prefetch_test.rs` 4 处 ON 构造（multipage / predicate+LIMIT 循环 / 链尾空表 / 链尾单页）全部显式 `.with_prefetch(true)`，OFF 路径 `.with_prefetch(false)` 不变，等价断言体零改动——翻转后 ON 语义经显式 opt-in 保持，无 "off vs off" 空转退化。
3. **新鲜复测全部一致**（Review 独立重跑）：`cargo test --all` TOTAL 585 passed / 0 failed（581 既有零修改 + 新单测 1；含 prefetch 3/3、pushdown 15/15 零修改、file_storage_io 4/4）；`cargo test --lib executor::data_scan::tests` GREEN；`cargo clippy --all-targets -- -D warnings` exit 0（仅 `~/.cargo/config` 环境既有弃用提示）；`cargo fmt --check` 0 diff；`openspec validate` valid。
4. **T5.4 判读偏差独立裁定（决定性验证，判读维持）**：Review 第三轮独立复跑 `cargo bench --bench data_scan_bench -- --baseline before-MS08-T02`：`data_scan/1000` change [-1.76%, -0.68%, +0.42%]（p=0.24）No change；`data_scan/10000` change [-1.41%, +0.28%, +1.90%]（p=0.73）No change；对照组 `scan_via_index/1000` +1.17%（p=0.06）、`/10000` +1.78%（p=0.13）。**Act 两轮的 10000 档 -3.4~-3.8%（p=0.00）改善在本轮未复现**，两档同时满足契约字面 "No change (p>0.05)"；绝对值 7.62ms 回到基线 ~7.6ms（父 Cycle ON 路径两轮实测 ~8.9ms 回退签名消失）。Act 的"会话环境漂移"判读由未复现直接证实。
5. **机制不可能论证复核成立**：OFF 路径每次 closure 入口相对基线代码严格增加工作（`captured_next` 初始化 + 每次 entry 一次 header 读——基线仅 all-invisible/exhausted 分支各读一次，行产出 entry 原为零——post-await 一次比较 + `trigger_prefetch` 早退调用），不存在任何 entry 类型做更少工作；严格加性代码不可能产生真实因果加速。对照组（`ScanExecutor::new`，`benches/data_scan_bench.rs:60`，本 change 全程未触碰该路径）跨轮 ±1~5% 漂移且出现 p<0.05，独立证明环境漂移带量级与 10000 档观测相符。
6. **基线完整性 PASS**：`target/criterion/**/before-MS08-T02` 4 条目实存，`sample.json` mtime 全部为 2026-09-05 19:25-19:26 原始采集时间——对比运行未覆盖重采，对比有效性成立。
7. **文档一致性 PASS**：spec R3（可选能力默认关闭 + "默认构造不发起预取"场景含 p>0.05 子句）、design.md §目标行为 T02 默认关闭修订段（NEW-EVIDENCE）、tasks.md Task 5 勾选与 5.4 判读注记、proposal.md What Changes 默认关闭要点——四产物口径一致。
8. **Acceptance 五项逐条满足**：默认关闭非空洞断言 ✓；ON 显式化后父 Cycle 等价/谓词+LIMIT/链尾测试全 GREEN ✓；全量回归 0 failures + 质量门全 0 ✓；默认路径 bench 两档 No change（Review 轮 p=0.24/0.73，对照组维持）✓；proposal 一致 ✓。

Minor findings（不阻塞，记录不返工）：

- M1/M2 沿父 Cycle 记录：全页不可见快跳预取零重叠窗口（loading_locks 去重无害）；`scan_cap` 提前封顶一次性无效预取（≤1 页、可淘汰）——深度 1 设计固有，不返工。
- 本 Review 无新 Minor finding。

**Deviation Classification**

BASELINE-CHANGED（环境侧，非阻塞）：T5.4 唯一偏差的根因是固定基线采集会话与对比会话之间的环境漂移（对照组未改动路径同幅漂移直接证明），基线数据本身未被覆盖（sample.json mtime 证实）。Act 判读（机制排除 + 对照组佐证 + 方向有利）正确，并由 Review 第三轮未复现最终证实；契约字面 GREEN 条件在 Review 独立运行中满足。非 ACT-DEVIATION（Act 执行了契约要求的全部动作），非 PLAN-INVALID（判读框架已由 Plan Context Risks 预置"结合幅度与方向判断"）。

**Acceptance Gaps**

None。五项 Acceptance 逐条核对满足（见 Findings 8）；T5.4 的字面缺口经 Review 独立复跑闭合（两档 p=0.24/0.73 No change）。

**Convergence**

N/A（本 Cycle 首次 Review；无既有 gap）

**Evidence**

- `src/executor/data_scan.rs`（421 行）与 `tests/prefetch_test.rs`（211 行）全文审查；`benches/data_scan_bench.rs` 对照组构造点核实（`ScanExecutor::new`，line 60）
- Review 复测：`cargo test --all`（TOTAL 585 passed / 0 failed，exit 0）；`cargo test --lib executor::data_scan::tests`（1 passed）；`cargo test --test prefetch_test`（3 passed）；`--test pushdown_test`（15 passed，零修改）；`--test file_storage_io_test`（4 passed）；`cargo clippy --all-targets -- -D warnings`（exit 0）；`cargo fmt --check`（exit 0）；`openspec validate 2026-09-05-ms08-t01-t02-pread-prefetch`（valid）
- Review 复测 bench（第三轮）：data_scan/1000 p=0.24 No change、data_scan/10000 p=0.73 No change、scan_via_index 对照组 p=0.06/0.13 No change——Act 10000 档改善未复现，环境漂移判读证实
- `target/criterion/**/before-MS08-T02/sample.json` mtime（19:25-19:26，未被覆盖）
- spec/design/tasks/proposal 默认关闭口径一致性核对

**Follow-up Decision**

Acceptance 全部满足，无阻塞 finding，T5.4 唯一提请裁定项由 Review 独立复跑闭合。Iteration 001 完成；Iteration Plan 无剩余 Iteration，本 change 全部交付。遗留用户决策项（非本 change Acceptance）：① 工作树全部改动待用户 commit；② 两条 Experience Candidates（MS08 基线采集方法 Runbook、bench 回退因果判读方法 Runbook）是否授权 Recorder；③ `BufferPool::evict_one` 锁范围跨脏页写回的 I 候选是否授权 docs-maintainer 登记。

**Iteration Plan Update**

None（Iteration Map 不变：000 accepted + 001 accepted，无剩余 Iteration）

**Next Cycle**

None

**Next Iteration**

None（Iteration 001 随本 Review accepted 完成；change 全部交付，无剩余 Iteration）
