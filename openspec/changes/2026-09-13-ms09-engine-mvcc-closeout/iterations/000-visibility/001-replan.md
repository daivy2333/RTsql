# Iteration 000 / Cycle 001: 事务可见性域收口（001-replan：RC 快照结构修订 + 分配器水位推进）

## Plan Context

- Status: ready
- Iteration: 000-visibility
- Cycle: 001-replan
- Cycle Type: replan
- Parent cycle: 000-initial.md

**Iteration Scope**

- Change tasks: T1-T7（tasks.md Iteration 000；T6 按 design D10 修订，其余不变）
- Depends on: None
- Stable baseline: I033 探针序列扫描空集（运行期 + restart 两态）、未提交删除/回滚扫描语义正确、I032 未提交行重启不复活、RC 可配置且全部 6 验收场景通过；默认 RR 全量零回归
- Verification boundary: isolation/mvcc 套件全绿 + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: 快照结构族（`snapshot.rs`、`tx_id.rs`、`database.rs::statement_snapshot/open_with_isolation`）+ 000-initial 已建立的可见性族回归面
- Deferred tasks: T10-T14（Iteration 001 NLJ）、T20-T22（Iteration 002 子查询缓存）

**Cycle Scope**

- Trigger: replan-required（000-initial Plan Review：D4 单 id 快照设计与 T6 Preserve「Snapshot 本体不变」在 R2-S1/R2-S2 上结构性冲突，PLAN-INVALID）
- Acceptance gaps: transaction-isolation-levels R2-S1（脏读未排除，实测 `[[7,70]]`）、R2-S2（语句间提交不可见，实测 `[]`）
- Repair items: None（replan 使用修订后的全局 task，不设 repair item）
- Inherited scope: 000-initial T1-T5 全部实施与验证结论（Plan Review 已核对 diff 并复跑套件，采信）；000-initial 的 Task Contract T1-T5、全部 Invariants、mvcc-tombstone-visibility 全部场景
- Excluded scope: NLJ、子查询缓存、SQL/CLI 隔离面、运行中切换、写写冲突检测、RR 真快照化、未提交删除点查/回滚索引时序边界（Issue 候选）、GC 对墓碑链回收（I038 域）

**Objective**

Snapshot 自身身份与可见性高水位分离后，RC 语句视图全部 6 验收场景通过（含 R2-S1 脏读排除、R2-S2 语句间提交可见）；恢复后事务 id 分配器越过已恢复 id（RC 高水位语义健全性前提）；T1-T5 既有绿面与 RR 默认路径零回归维持。

**Background**

000-initial 按 T6 契约完成 RC 接线后，`isolation_level_test` 4/6 通过、R2-S1/R2-S2 实证失败，Act 按 Gate 6 交接阻塞：`Snapshot` 单一 `tx_id` 字段同时承担可见性高水位（`is_visible` 规则 2）与自身身份（`is_visible_self`）两个不相容角色，对 RC 语句视图不存在同时满足二者的单值。Plan Review 独立核实根因成立（代码 + 失败复现），裁定 T6 Preserve「Snapshot/find_visible_version 本体不变」与 D4 快照设计互相矛盾，属 PLAN-INVALID——修复需要修订设计（D10）与新执行契约，故创建本 replan Cycle。修订方案采纳 Act 修复方向 1（结构分离），否决方向 2（另穿 self-id，触碰面更大）与方向 3（缩窄验收语义，验收让步）。

**Investigation Facts**

- Current Baseline: 000-initial Act Response 全部 T1-T6 改动在工作区（diff 与 Changed Files 清单一致，Plan Review 已核对）；全量 907 passed / 2 failed / 2 ignored（Act 收尾运行，材料未变化采信）；`mvcc_tombstone_visibility_test` 10/10、`isolation_level_test` 4/6（Plan Review 本会话复跑确认，失败形态逐字一致）；clippy 0 / fmt 0 / validate 28 PASS（Act）。
- Current-State Evidence（Plan 直接读码核实，file:line）：
  - `Snapshot { tx_id, active_tx_ids }`（`snapshot.rs:10-14`）；`is_visible` 规则 2 `create_tx_id > self.tx_id → false`（`snapshot.rs:41-44`，高水位角色）；`is_visible_self` `create_tx_id == self.tx_id && commit.is_none()`（`snapshot.rs:56-58`，自身身份角色）；`contains_active_tx`（:61-63）。`#[derive(Clone)]`（000-initial 偏差 1）。
  - `statement_snapshot`（`database.rs:119-127`）：RC → `reader = reader_tx_id.unwrap_or(current_tx_id())` + `active_transactions()`。
  - 失败机理（复现确认）：R2-S1——auto-commit reader 取 `current_tx_id()` 与仍活跃写事务 id 相同，未提交行 `create==reader ∧ commit=None` 被 `is_visible_self` 放行；R2-S2——显式事务 reader 固定 `tx.id()`，语句间提交的事务 id > reader id 被规则 2 拒绝。
  - 分配器：`TransactionId::new()` counter 从 0 起、`allocate()` fetch_add +1（`tx_id.rs:8-20`）；`open_with_isolation` 计算 `_max_tx_id` 后丢弃（`database.rs:74-81`）——重启后 id 复用，新事务 abort 的 `mark_aborted`（create_tx 归零）可误伤历史同 id 版本；RC 高水位论证「≤ current 即 committed/aborted/active」要求无复用。
  - 消费面全部经方法：`find_visible_version` 检查 `is_visible ∨ is_visible_self`（`buffer_pool.rs:336-337`）；`superseder_suppresses` 快照臂用 `s.tx_id()`（自身删除）+ `contains_active_tx`（`data_scan.rs`，000-initial T3 重写后形态）——`tx_id` 保留自身身份语义后两处零改动。
  - `Snapshot::new` 生产调用 2 处（`database.rs:126`、`manager.rs:94` begin）+ 单测/bench（snapshot.rs 5、executor_test 4、explicit_tx 2、visibility_bench 3、mvcc_commit_test 1）——`new` 语义保留则全部零改动。
  - `update_version_header_in_data_page` 内部 `clear_all_visible`（`data_page.rs:144`）——T5 mark 步清旗已由既有 helper 覆盖（Act Risks 关切闭环）。
  - 测试夹具面：`isolation_level_test.rs` 6 用例已存在且表达 delta spec 场景（R2-S1 :76-100、R2-S2 :105-140）；重启后 tx id 字面值断言不存在（`dml_tx_id_test` 仅断言 `> 0` 与一致性）。
- Code and Critical Path: `snapshot.rs`（结构 + 双构造器 + 方法语义）→ `database.rs::statement_snapshot`（语句视图构造）与 `open_with_isolation`（水位推进）；`tx_id.rs`（advance 原语）经 `TransactionManager` 透传；全部分析/点查消费面方法级隔离，无需触碰。

**Implementation Guidance**

实施顺序：T6-R1（Snapshot 结构与单测）→ T6-R2（statement_snapshot 切换语句视图）→ T6-R3（分配器水位推进）→ T6-R4（GREEN 收尾，即原 T7）。关键事实：(1) `new` 保留原语义（self == high_water == tx_id），RR begin 快照与既有单测/bench 零改动；(2) `statement_view` 的 self=0 安全性：id 0 为 `mark_aborted` 的 aborted 标记值，真实版本 `create_tx_id` 恒 > 0，`is_visible_self(0)` 永假；(3) 高水位取 `current_tx_id()`（语句开始时点）而非 begin 时点——语句间提交的事务 id ≤ 该值且不在 active 集，R2-S2 由此转绿；(4) `advance_past` 用 CAS 循环保证单调（并发 begin 下不回退）；(5) `mark_uncommitted_aborted` 在 uncommitted 空时于 catalog scan 前早退（`recovery.rs:1004-1006`），干净打开零新增 I/O。

**Behavioral Change**

- 当前：RC auto-commit 扫描产出他事务未提交行（R2-S1 脏读）；显式事务内语句间提交不可见（R2-S2）；重启后事务 id 复用（分配器从 0 重来）。
- 目标：RC 语句视图 = 语句开始时点已提交集 + 自身未提交写；R2-S1/S2 转绿；重启后分配器越过已恢复 max id，RC 重启后已提交行对新鲜语句视图可见。
- 接口/错误/状态语义：`Snapshot` 新增 `high_water` 字段与 `statement_view` 构造器（additive）；`is_visible` 规则 2 高水位源由 `tx_id` 改为 `high_water`（`new` 构造下两值相同 → RR 面行为不变）；`TransactionId::advance_past` / `TransactionManager` 透传（additive）；错误面、WAL/页格式、公共 SQL 语义不变。

**Task Contracts**

### T6-R1: Snapshot 自身身份/高水位分离

- Requirement/Scenario: transaction-isolation-levels R2（S1-S5）结构前提；mvcc-tombstone-visibility R3-S2（自身删除判定语义保持）
- Depends on: None（T1-T5 已完成）
- Targets: `src/transaction/snapshot.rs`（结构 + `statement_view` 构造器 + `is_visible` 规则 2）
- Current behavior: 单 `tx_id` 双角色；`is_visible` 规则 2 用 `self.tx_id`
- Required behavior: 新增 `high_water` 字段；`is_visible` 规则 2 改用 `self.high_water`；`is_visible_self`/`contains_active_tx`/`tx_id()` 语义不变；`new(tx_id, active)` 保留原构造（high_water = tx_id）；新增 `statement_view(high_water, self_tx_id, active)`
- Required changes: 结构字段 + 构造器 + 规则 2 一处比较源变更
- Preserve: `new` 的现有调用点（manager.rs begin、全部单测/bench）零改动且行为逐字节不变；`Clone` derive 保持；方法签名不破坏消费面
- Forbidden: 修改 `find_visible_version` 本体；修改 DataScan 抑制判定；改变 `new` 语义；引入运行时切换或写写冲突检测
- Test witness: `snapshot.rs` 单测新增——`statement_view` 三态（R2-S1 形态：create==high_water 活跃未提交 → 不可见；R2-S2 形态：create > self ∧ create ≤ high_water ∧ committed → 可见；self 写可见：create==self ∧ commit=None → `is_visible_self` 真）+ `new` 既有 5 单测零修改通过
- GREEN condition: 新单测通过且既有 5 单测不改一字通过
- Verification: `cargo test --lib transaction`
- Stop when: 规则 2 源变更需要触碰 `new` 语义或消费面本体——返回 Plan

### T6-R2: statement_snapshot 切换语句视图

- Requirement/Scenario: transaction-isolation-levels R2-S1、R2-S2（阻塞场景本体）；R2-S3/S4/S5 保持
- Depends on: T6-R1
- Targets: `src/database.rs::statement_snapshot`
- Current behavior: RC → `Snapshot::new(reader, active)`（单 id 双角色，两场景失败）
- Required behavior: RC → auto-commit `Snapshot::statement_view(self.transaction_manager.current_tx_id(), 0, active)`；显式事务 `Snapshot::statement_view(self.transaction_manager.current_tx_id(), reader_tx_id, active)`
- Required changes: 构造调用切换（一处）
- Preserve: RR 分支返回 None；`execute_stage`/`execute_stage_in_tx` 调用形态不变（reader 参数语义不变）；快照不进 plan cache
- Forbidden: 改变 DML 臂 tx_id 语义；在快照中引入语句时间戳/时钟；改动调用方
- Test witness: `cargo test --test isolation_level_test` —— R2-S1、R2-S2 由 RED 转 GREEN；R2-S3/S4/S5 保持绿（high_water 与 active 集语义覆盖已提交删除消失、自身写可见、auto-commit 等价）
- GREEN condition: isolation_level_test 6/6（T6-R3 未完成时允许 restart 相关项除外——本套件无 restart 用例，实际即 6/6）
- Verification: `cargo test --test isolation_level_test`
- Stop when: 切换后仍有意料外场景失败且形态指向 `Snapshot` 方法语义之外——返回 Plan

### T6-R3: 恢复后分配器水位推进

- Requirement/Scenario: transaction-isolation-levels R2 语义健全性前提（重启后 RC 视图高水位覆盖已恢复 id）；D10
- Depends on: T6-R1（共享 high_water 论证）
- Targets: `src/transaction/tx_id.rs`（`advance_past` 新增）；`src/transaction/manager.rs`（透传方法）；`src/database.rs::open_with_isolation`（`_max_tx_id` 消费）
- Current behavior: `_max_tx_id` 计算后丢弃（`database.rs:74-81`）；重启后 `allocate()` 从 1 重来
- Required behavior: `TransactionId::advance_past(max_used)` CAS 保证 counter ≥ max_used；`TransactionManager` 透传；`open_with_isolation` 以 committed ∪ aborted ∪ uncommitted 的 max 调用；`current()` 返回推进后值
- Required changes: 一个原子方法 + 一个透传 + 一处调用
- Preserve: `allocate()`/`current()` 既有语义与单测；恢复流程顺序（recovery → 推进，位置即现 `_max_tx_id` 计算处）；WAL/恢复格式不变
- Forbidden: 持久化分配器状态（文件格式变更）；改变恢复重放逻辑；为推进机制新增身份型字段或指纹
- Test witness: `tx_id.rs`/`manager.rs` 单测（advance_past 单调性 + 并发不回退 + current 反映推进）；`tests/mvcc_tombstone_visibility_test.rs` restart 用例保持绿；全量门暴露任何依赖复用 id 的既有测试（调查预判不存在——重启后 tx id 字面值断言缺位）
- GREEN condition: 推进后重启库再 begin/写入，WAL 新记录 id > 已恢复 max（`dml_tx_id_test` 形态断言可扩展于 mvcc 套件或单测承载）
- Verification: `cargo test --lib transaction` + `cargo test --test mvcc_tombstone_visibility_test`
- Stop when: 推进需要触碰恢复流程或文件格式——返回 Plan

### T6-R4: Iteration 收尾（原 T7 继任）

- Requirement/Scenario: transaction-isolation-levels R3、mvcc-tombstone-visibility R5（零回归类）
- Depends on: T6-R1/R2/R3
- Targets: 全局
- Current behavior: 全量 907/2/2（2 failed 即 R2-S1/S2）
- Required behavior: isolation 6/6 + mvcc 10/10 + 全量回归零修改 + `cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` 0、`openspec validate --specs --changes` PASS
- Required changes: 如全量暴露依赖旧缺陷/复用 id 行为的既有测试，按 D7 在 delta spec 记录校准后实施（调查预判为零）；否则仅验证
- Preserve: 校准不放宽断言语义
- Forbidden: 静默放宽既有断言
- Test witness: 全量输出决定性片段
- GREEN condition: 全量 0 failed / 2 ignored（≥912 passed：907 基线 + 新单测）
- Verification: `cargo test` 全量 + clippy/fmt/validate
- Stop when: 全量出现非校准可解失败——返回 Plan

**Invariants**

- 页格式 22B VersionHeader、WAL 记录格式、文件格式版本不变（000-initial Invariants 全部继承）。
- 默认 RR 路径行为逐字节不变（`new` 语义保留 + RR 恒 None）；892 基线 + mvcc 10 全量零修改。
- 公共 API 仅 additive（`statement_view`、`advance_past`、透传方法）；`Database::open`/`execute_in_tx`/`begin/commit/rollback` 签名不变。
- `find_visible_version` 与 DataScan 抑制判定本体不动（方法消费面）。
- 事务 id 全生命周期单调不复用（运行期与重启后一致）。
- 身份型证据工程禁令。

**Non-goals**

- NLJ（Iteration 001）、子查询缓存（Iteration 002）。
- SQL/CLI 隔离面、运行中切换、Serializable/SSI、写写冲突检测、RR 真快照化。
- 未提交删除期间点查不可达 / DELETE 回滚后 PK 点查不可达（预存边界，Issue 候选维持）。
- GC 对墓碑链的回收策略（I038 域）；分配器状态的持久化。

**Acceptance**

1. `tests/isolation_level_test.rs` 6/6：R2-S1 脏读排除、R2-S2 语句间提交可见、R2-S3 语句间删除消失、R2-S4 自身写可见、R2-S5 auto-commit 等价、R1-S2 RC 打开可用——映射 transaction-isolation-levels R1/R2。
2. `tests/mvcc_tombstone_visibility_test.rs` 10/10 保持（T1-T5 绿面继承）——映射 mvcc-tombstone-visibility R1-R4。
3. `snapshot.rs`/`tx_id.rs`/`manager.rs` 新单测通过且既有单测零修改——映射 D10 结构与推进语义。
4. 全量回归零修改通过（≥912 passed / 0 failed / 2 ignored）——映射 R3/R5 零回归类。
5. clippy/fmt/validate 全 0/PASS。
Acceptance 与 requirement/scenario/design/task/代码/测试映射见 tasks.md RTM（全部 Covered）。

**Verification**

- `cargo test --test isolation_level_test --test mvcc_tombstone_visibility_test`（目标套件，RED→GREEN）
- `cargo test --lib transaction`（快照/分配器单测）
- `cargo test`（全量，零回归门）
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- 决定性输出各取 ≤20 行写入 Act Response。

**Gate 2 Readiness**

- 无 Missing requirement（RTM 全 Covered）：PASS（tasks.md RTM，T6/R2 行已同步 D10）
- 无 Simplified 未批准：PASS
- 调查完整（阻塞根因、分配器、消费面、调用点均有 file:line 证据；父 Cycle 已核实维度直接引用）：PASS（Investigation Facts + 000-initial Plan Review）
- 设计闭合（修订方案、语义论证、替代案否决已记录）：PASS（design.md D10）
- 任务可执行（T6-R1/R2/R3/R4 各有位置/行为变化/测试见证/停止条件）：PASS（Task Contracts）
- 分轮合理（Iteration 000 范围不变，单 Cycle 收口修订面）：PASS
- 追踪完整（RTM 无 Missing）：PASS
- 验证充分（目标套件 RED→GREEN + 全量零回归 + 工具链门）：PASS（Verification）
- 无身份型证据工程：PASS（行为面测试）
- 无实质未知项留 Act：PASS（非实质项见 Risks）
- tasks/specs/design/Iteration/Cycle 一致：PASS（design D10、tasks T6 行与 RTM、本 Cycle 同步更新）
- Persisted Evidence 模式明确：PASS（none，见下）
- 用户批准计划：PASS（2026-09-13 用户批准「批准，交接 Act」）

**Persisted Evidence**

- Mode: none

理由：全部验收由可重复运行的测试与工具链命令承载，命令、决定性输出、退出码写入 Act Response 即满足公共规则 › 验证；阻塞证据（两条失败用例）本身可低成本复现。

- Budget: 不适用（none）

**Risks and Notes**

- 既有测试若隐含依赖重启后 tx id 复用（字面小值断言）：调查预判不存在（重启场景无 tx_id 字面值断言）；全量门暴露时按 D7 校准，属契约内非实质项。
- `all_visible` 快路径：`update_version_header_in_data_page` 已内部 `clear_all_visible`（`data_page.rs:144`），T5 mark 步清旗闭环；RC 快照首次大规模流经 `find_visible_version` 后若发现既有 `check_page_all_visible` 语义冲突，属非实质局部修正（Act Response 记录），不改变契约语义（继承父 Cycle Risks 裁定）。
- `advance_past` 与并发 `begin` 的竞争：CAS 循环保证单调，恢复先于任何用户 begin（open 内顺序），实际无竞争窗口；单测仍覆盖并发形态。
- RC 快照读 `active_transactions()` 与语句执行间的并发窗口：沿用父 Cycle 裁定（语句开始时点视图，同级语义）。
- 非实质留白（Act 自行决定）：`statement_view` 参数序与命名；透传方法命名；新单测的组织位置。

## Act Response

- Status: blocked

**Implemented**

T6-R1 与 T6-R3 完整完成并 GREEN；T6-R2 按契约实施完毕、R2-S1 转绿，R2-S2 仍失败（Gate 6 阻塞，见 Blocker Handoff——根因已定位为 Plan 消费面清点遗漏的页级快路径角色混用，修复触碰 Invariants 保护面）。T6-R4 的全量与工具链门已运行（917 passed / 1 failed / 2 ignored，唯一失败即阻塞项；RR 路径与全部既有套件零回归），但 Iteration 收尾被 T6-R2 验收缺口阻塞，不声明完成。

- T6-R1：`Snapshot` 新增 `high_water` 字段与 `statement_view(high_water, self_tx_id, active)` 构造器；`is_visible` 规则 2 比较源由 `self.tx_id` 改为 `self.high_water`；`new(tx_id, active)` 保留原语义（high_water = tx_id，既有调用点 manager.rs begin、全部单测/bench 零改动）；`is_visible_self`/`contains_active_tx`/`tx_id()` 语义不变；`Clone` derive 保持。新增 3 单测（R2-S1 形态：create==high_water 活跃未提交不可见且 `is_visible_self` 不误放；R2-S2 形态：create > self ∧ create ≤ high_water ∧ committed 可见；self 写可见 + 他人未提交不可见）。RED（E0599 ×3）→ GREEN：`cargo test --lib transaction` 42 passed / 0 failed，既有 5 snapshot 单测零修改通过。
- T6-R2：`Database::statement_snapshot` RC 臂切换为 `Snapshot::statement_view(self.transaction_manager.current_tx_id(), reader_tx_id.unwrap_or(0), active)`（auto-commit self=0——id 0 为 aborted 标记，真实版本 create 恒 > 0，`is_visible_self` 不误放；显式事务 self=reader_tx_id）。结果：`rc_dirty_read_excluded`（R2-S1）转绿，4/6 → 5/6；`rc_statement_sees_commit_between_statements`（R2-S2）仍 observed `[]`（结构性根因见 Blocker Handoff）。
- T6-R3：`TransactionId::advance_past(max_used)` CAS 循环保证 counter ≥ max_used（并发下单调、低水位 no-op）；`TransactionManager::advance_past` 透传；`Database::open_with_isolation` 以 committed ∪ aborted ∪ uncommitted 的 max 调用（原 `_max_tx_id` 丢弃点改为消费点，恢复流程顺序不变）。见证：tx_id.rs 4 单测（水位推进、低水位 no-op、与 allocate 并发不回退 ×2 形态）+ manager.rs 1 单测（current 反映推进 + 下一次 begin id=43 > 42）+ `tests/mvcc_tombstone_visibility_test.rs` 新增 `tx_id_allocator_advances_past_recovered_ids_after_restart`（重启后 begin id 必须大于已恢复 max——RED 实测 "got 1 with max recovered 2" 即复用缺陷，GREEN 后 3 > 2）。RED → GREEN：`cargo test --lib transaction` 47 passed / mvcc 套件 11 passed。

**Changed Files and Symbols**

本 Cycle 改动（叠加于 MS16 + 000-initial T1-T6 工作区之上）：

产品代码：
- `src/transaction/snapshot.rs` — `Snapshot`（+`high_water` 字段、`statement_view` 构造器、`is_visible` 规则 2 比较源、文档注释同步）
- `src/database.rs` — `Database::statement_snapshot`（RC 臂构造切换）、`Database::open_with_isolation`（`max_tx_id` 消费 + `transaction_manager.advance_past`）
- `src/transaction/tx_id.rs` — `TransactionId::advance_past`（新增）
- `src/transaction/manager.rs` — `TransactionManager::advance_past`（新增透传）

测试：
- `src/transaction/snapshot.rs` — 3 新单测（statement_view 三态）
- `src/transaction/tx_id.rs` — 4 新单测（advance_past 单调/no-op/并发 ×2）
- `src/transaction/manager.rs` — 1 新单测（advance_past 透传 + 下一次 begin 越过水位）
- `tests/mvcc_tombstone_visibility_test.rs` — 1 新用例（重启后分配器越过已恢复 max，D10）

**Deviations from Plan**

1. T6-R1 的 struct/构造器/`is_visible` 文档注释随结构变更同步改写（契约 Required changes 字面为「结构字段 + 构造器 + 规则 2 一处比较源变更」）：文档所述「snapshot ID」在分离后不再准确，属必要文档面同步，语义零变化。非实质。
2. T6-R3 见证粒度：契约列「advance_past 单调性 + 并发不回退 + current 反映推进」三类断言，实施拆为 tx_id.rs 4 单测（并发形态按「低水位竞争」与「高水位竞争」分立）+ manager.rs 1 单测 + mvcc 重启用例（GREEN condition 明示可由单测或 mvcc 套件承载）。非实质。
3. `cargo fmt` 对 tx_id.rs 新测试段做机械重排（同 000-initial 偏差 10 先例）。
其余实施与 Task Contract 逐项一致；无 Preserve/Forbidden 项被触碰。

**Blocker Handoff**

- **命中 Gate**：T6-R2 GREEN condition / Gate 5；T6-R2 Stop condition「切换后仍有意料外场景失败且形态指向 `Snapshot` 方法语义之外——返回 Plan」；Gate 6「Task Contract 无法覆盖达到既有 Acceptance 所需的工作」+「实际代码与契约存在实质冲突」。
- **task/step**：T6-R2「statement_snapshot 切换语句视图」；验收场景 transaction-isolation-levels R2-S2（`rc_statement_sees_commit_between_statements`）。
- **Plan 预期**：按 T6-R1/R2 契约构造 statement_view（`is_visible` 规则 2 用 high_water、`is_visible_self` 用自身 id）后 `isolation_level_test` 6/6；001-replan Investigation Facts 认定「消费面全部经方法……`tx_id` 保留自身身份语义后两处零改动」（消费点仅 `find_visible_version` 与 `superseder_suppresses`）。
- **实际情况**：R2-S1 转绿；R2-S2 observed `[]`。根因为 **Plan 消费面清点遗漏的页级可见性快路径**——三处把 `Snapshot::tx_id()`（statement_view 下 = 自身 id R，auto-commit 为 0）当作可见性高水位消费，`min_create_tx_id ∈ (self, high_water]` 的整页被误判 all-invisible 跳过：
  1. `src/executor/data_scan.rs:413` — DataScan 页快路径 `v.all_invisible_for(s.tx_id())`（本次 R2-S2 失败的直接短路点）；
  2. `src/storage/buffer_pool.rs:313` — `find_visible_version` all-invisible 快路径 `all_invisible_for(snapshot.tx_id())`（RC 点查同型缺口，本次未触发）；
  3. `src/storage/buffer_pool.rs:441` — `check_page_all_visible` 条件 2 `vh.create_tx_id() >= snapshot.tx_id()`（同型角色混用；statement_view 下保守安全——永不置位 all_visible 仅损失优化，但同一混淆应一并修）。

  R2-S2 机理链：语句 1 空页扫描 → `check_page_all_visible` 真空真 → `set_all_visible(P)`（min_create=u64::MAX）→ auto-commit INSERT → `update_visibility_on_insert(P, W)`（min_create=W，all_visible=false）→ commit `clear_all_visible(P)` → 语句 2（statement_view(high_water=W, self=R, active=[R])）→ `all_invisible_for(R)` = W > R = true → 整页跳过 → `[]`。
- **根因定位对 000-initial Blocker Handoff 的修正**：000-initial 将 R2-S2 归因于 `is_visible` 规则 2（create > reader 被拒）——语义层根因成立且已由 T6-R1 修复；本次定位补全：页级快路径在逐 slot 规则判定之前短路，000-initial 的 R2-S2 失败同样由该路径直接产生（旧形态 `all_invisible_for(R)` 同真）。规则 2 修复必要但不充分。
- **影响**：T6-R2 验收未达成（5/6）；T6-R4 收尾门（isolation 6/6）不满足，Iteration 不声明完成。T6-R1/T6-R3 完整收口；RR 默认路径与全部既有套件零回归（全量 917 passed / 1 failed / 2 ignored，唯一失败即 R2-S2）。
- **修复方向供 Plan 裁定**（均超出当前契约授权，未实施）：
  1. **Snapshot 增加 `high_water()` 访问器 + 三处消费点改传高水位**（推荐评估）：`new` 构造下 high_water == tx_id → RR 面与既有单测/bench 逐字节不变；statement_view 下 all-invisible 快路径不再误跳——该路径对 RC 语句视图本无合法触发面（语句开始时已存在行的 create ≤ high_water 恒成立），回落逐 slot 判定即正确语义；`check_page_all_visible` 条件 2 改高水位后 RC 下恢复 all-visible 置位优化。注意 `find_visible_version` 本体（buffer_pool.rs:313）受 001-replan Invariants「`find_visible_version` 与 DataScan 抑制判定本体不动」明文保护，需 Plan 显式修订该 Invariant。
  2. 比较逻辑内聚进 Snapshot（如 `page_all_invisible(min_create)` 方法）——消费面调用同样需要改动，触碰面与方向 1 相同。
  3. 缩窄 R2-S2 验收语义——属 spec 裁定让步，不建议（同 000-initial 方向 3 先例）。
- **任务状态**：T6-R1 completed；T6-R2 implemented-per-contract / acceptance unmet（5/6，R2-S2 FAIL）；T6-R3 completed；T6-R4 not completed（全量与工具链门已运行并记录，收尾声明被阻塞）。
- **工作区状态**：MS16 + 000-initial T1-T6 改动之上叠加本 Cycle 改动（文件清单见 Changed Files）；无 plan 外修改；clippy 0 / fmt 0 / openspec validate 28 PASS。
- **已通过 Gate**：Gate 3（T6-R1/T6-R3 新 API 编译 RED + R2-S2 运行时 RED 见证；T6-R2 切换前 4/2 失败态新鲜复跑）；Gate 4（逐任务 spec/code review + Response 前完整 diff review，结论见 Self-Review）。
- **恢复条件**：用户批准修复方向并调用 `openspec-plan`。Plan Context 不可改写，预计以 001-replan 的后继 Cycle（002-rework 或 002-replan）承载三处快路径修复并修订对应 Invariant；本 Cycle 的 T6-R1/T6-R3 实施与验证结论由后继 Cycle Inherited scope 采信继承。

**Blocker Resolution**

None（尚未恢复）

**Self-Review**

- Plan compliance: T6-R1/T6-R3 逐项符合 Task Contract；T6-R2 按契约实施、验收缺口为 Plan 消费面清点遗漏（非 Act 偏离）
- Full diff reviewed: yes（Phase 4 完整 diff 审查：不变量——页格式 22B/WAL 记录格式/文件格式版本未动；公共 API 仅 additive（`statement_view`/`advance_past`/透传）；`new` 语义保留；DataScan 抑制判定与 `find_visible_version` 本体未动；plan cache 键语义未动；RR 恒 None；`Database::open`/`execute_in_tx`/begin/commit/rollback 签名不变；事务 id 单调不复用；无身份型证据机制；无 plan 外修改；测试不因错误原因通过——1 个失败为真实验收缺口而非测试缺陷）
- Critical findings unresolved: 0（R2-S2 缺口属 Plan 层契约与 Invariant 冲突，已按 Gate 6 交接，非 Act 范围内可修复项）
- Important findings unresolved: 0
- Minor findings unresolved: 2
  - M1：`check_page_all_visible` 条件 2（buffer_pool.rs:441）在 statement_view 下保守安全（永不置位 all_visible，仅损失优化），但与阻塞项属同一 `tx_id()`/高水位角色混用，应随阻塞修复一并处理（已列入修复方向 1 范围）。
  - M2：并发单测的 `current()` 断言采用不变量（≥ 下界）而非精确值——CAS 竞争下精确终值不确定，不变量断言即该形态的正确测试意图；记录供 Review 追认。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T6-R1 RED | `cargo test --lib transaction`（新增 statement_view 三态单测后、实现前） | `E0599: no function or associated item named statement_view` ×3 | statement_view API 缺失 | 编译 RED PASS |
| T6-R1 GREEN | `cargo test --lib transaction`（实现后） | `42 passed; 0 failed`（新增 3 + 既有 5 snapshot 单测零修改） | Snapshot 结构/双构造器/规则 2 | GREEN PASS |
| T6-R2 RED（基线） | `cargo test --test isolation_level_test`（切换前） | `4 passed; 2 failed`；R2-S1 observed `[[7,70]]`、R2-S2 observed `[]` | 阻塞场景新鲜复现 | 与 Blocker Handoff/父 Cycle 形态逐字一致 PASS |
| T6-R2 切换后 | `cargo test --test isolation_level_test` | `5 passed; 1 failed`；R2-S1 转绿，R2-S2 observed `[]` | transaction-isolation-levels R1/R2 | R2-S2 FAIL（阻塞项） |
| T6-R3 RED | `cargo test --lib transaction` + `cargo test --test mvcc_tombstone_visibility_test tx_id_allocator` | `E0599: no method named advance_past` ×6；`got 1 with max recovered 2`（复用缺陷复现） | advance_past 缺失 + 重启 id 复用 | RED PASS |
| T6-R3 GREEN | `cargo test --lib transaction` + `cargo test --test mvcc_tombstone_visibility_test` | `47 passed; 0 failed`；`11 passed; 0 failed`（重启后 begin id 3 > max 2） | 分配器推进/透传/open 接线 + 重启面 | GREEN PASS |
| 全量回归 | `cargo test --no-fail-fast` | `TOTAL passed: 917 failed: 1 ignored: 2`；唯一失败 `rc_statement_sees_commit_between_statements` | 892 基线 + 000-initial 新增 + 本 Cycle 新增 9 | RR/既有面零回归 PASS；缺口即阻塞项 |
| clippy | `cargo clippy --all-targets -- -D warnings` | 0 error | 全目标静态分析 | PASS |
| fmt | `cargo fmt --check`（应用格式化后复验） | 0 diff | 全仓格式 | PASS |
| validate | `openspec validate --specs --changes` | `28 passed, 0 failed`（WARNING 为 wal-writer-handle-reuse 既有占位提示，非失败） | change/specs 结构 | PASS |

**Persisted Evidence**

None required（Persisted Evidence 模式 none；全部验收由可重复命令承载，输出已摘录于上；阻塞证据即 `rc_statement_sees_commit_between_statements` 失败用例本体，`cargo test --test isolation_level_test` 可随时复现）

**Experience Candidates**

None（000-initial 已登记的「无 close/checkpoint 崩溃态模拟」Runbook 候选被 T6-R3 重启用例直接复用，无新增候选）

**Remaining Issues**

- 阻塞项本体：页级可见性快路径 `tx_id()`/高水位角色混用（三处，见 Blocker Handoff——属当前 change 范围内，非范围外 Issue 候选，不落账）。
- 预存边界维持（Plan Context 已列，Act 未触碰、Issue 候选不落账）：未提交删除期间与 DELETE 回滚后的 PK 点查不可达。
- 000-initial Act Response「新增观察」（open 后分配器未越过已恢复 id）已由 T6-R3 修复收口，从遗留清单划除。

**Commit or Diff Reference**

未 commit（待用户触发；对照基线 d8a244f + 工作区既有 MS16 + 000-initial 未提交改动）。本 Cycle 改动可按 `git status` 中 src/transaction/snapshot.rs、src/database.rs、src/transaction/tx_id.rs（本 Cycle 新触碰）、src/transaction/manager.rs + tests/mvcc_tombstone_visibility_test.rs 的对应 hunks 识别。

## Plan Review

- Review Result: rework-required

**Findings**

- **F1（阻塞，裁定依据）**：Act 的阻塞交接成立。三处页级快路径消费点经独立读码核实：`data_scan.rs:410-413`（DataScan 页快路径 `all_invisible_for(s.tx_id())`，R2-S2 直接短路点）、`buffer_pool.rs:313`（`find_visible_version` all-invisible 快路径，RC 点查同型缺口）、`buffer_pool.rs:441`（`check_page_all_visible` 条件 2，statement_view 下保守仅损失优化）。`page_visibility.rs:6-8` 头注释与实现同源混淆。R2-S2 机理链（`set_all_visible` → insert `update_visibility_on_insert` → commit `clear_all_visible` 后 `min_create=W` 存留 → 语句 2 `all_invisible_for(R) = W > R` 整页跳过）逐环节核实成立。
- **F2（PLAN-OMISSION，Plan 责任）**：001-replan Investigation Facts「消费面全部经方法……`tx_id` 保留自身身份语义后两处零改动」为错误论断（design D10 同源）——页级快路径作为第三类消费面被清点遗漏；Act 按契约实施无偏离，R2-S1 转绿即证语句视图语义本身正确。阻断性质：修复需触碰 001-replan Invariants 明文保护的消费面（`find_visible_version` 本体内一行实参）与 DataScan 快路径实参，超出父 Cycle 契约授权。
- **F3（非阻塞，T6-R1/T6-R3 追认）**：diff 核对与 Act 一致——`Snapshot` 结构分离 + `statement_view` + 3 单测、`advance_past` CAS + 透传 + open 接线 + 重启见证用例；复跑 `cargo test --lib transaction` 47 passed、mvcc 11 passed（含新重启用例）。000-initial 遗留观察（`_max_tx_id` 丢弃）已由 T6-R3 收口划除，确认。偏差 1-3（文档同步、见证粒度拆分、fmt 机械重排）均非实质，追认；M2（并发断言用不变量）为该形态的正确测试意图，追认。M1 的「永不置位」表述略欠精确（对 `create ≤ self` 旧行的页面仍可置位，语义对 RC 后续视图仍健全），随 F1 的统一修正一并消除，无需独立动作。
- **F4（非阻塞）**：修复方向 1（`high_water()` 访问器 + 三处消费点改传高水位）经健全性推演采纳——`create > high_water ⇒ is_visible 规则 2 假 ∧ is_visible_self 假`（self ≤ high_water 由 statement_view 构造保证），整页跳过语义成立；`new` 构造下 high_water == tx_id → RR 面与既有单测/bench 逐字节不变；RC 高水位单调不减 → all_visible 旗无陈旧风险。方向 2（方法内聚）触碰面相同、无额外收益；方向 3（缩窄验收）沿用 000-initial 先例否决。

**Deviation Classification**

- 阻塞本体：**PLAN-OMISSION**（Plan 消费面清点遗漏页级快路径第三类消费点，导致 Task Contract 与 Invariants 无法覆盖达到既有 Acceptance 所需的修复面）
- Act 偏差 1、2、3：非实质（文档面同步、见证粒度、fmt 机械重排），追认
- 无 ACT-DEVIATION、PLAN-INVALID、BASELINE-CHANGED、NEW-EVIDENCE

**Acceptance Gaps**

- transaction-isolation-levels R2-S2（语句间提交立即可见）：FAIL——实测 `[]` ≠ `[[5,50]]`（Plan 本会话复现，isolation 5 passed / 1 failed）；根因已收窄至三处页级快路径高水位实参
- R2-S1 已由本 Cycle 收口转绿（gap 收窄）；T6-R4 收尾门（isolation 全绿）未满足

**Convergence**

reduced——父 Cycle（000-initial）阻塞时 gap 为 R2-S1 + R2-S2 两条且根因在语句视图语义层；本 Cycle 收口 R2-S1，R2-S2 根因收窄至页级快路径三处实参（一行级修复面），无扩大。

**Evidence**

- 复现：`cargo test --test isolation_level_test` → `5 passed; 1 failed`（R2-S2 observed `[]`）、`cargo test --lib transaction` → 47 passed、`cargo test --test mvcc_tombstone_visibility_test` → 11 passed（本会话，退出码 1/0/0）
- 代码核实：`page_visibility.rs:6-20`、`data_scan.rs:410-438`（快路径计算 + 整页跳过）、`buffer_pool.rs:313`、`buffer_pool.rs:441`、`snapshot.rs` diff（high_water/statement_view/规则 2）、`tx_id.rs` diff（advance_past CAS）、`database.rs:86`（advance_past 接线）与 `:127-137`（statement_snapshot 切换）
- 基线：`git status` 38 个已跟踪修改文件与本 Cycle Changed Files 一致；Act 全量 917/1/2 按公共规则 › 验证采信（材料未变化 + 决定性套件独立复跑）

**Follow-up Decision**

创建 rework Cycle（非当前 Cycle 修复、非 replan）：修复 = `Snapshot::high_water()` 访问器 + 三处快路径实参/比较源一行级修正 + 点查见证，验收与设计处方不变（D10 分离原则的补全），Iteration Map 不变——但需触碰 001-replan Invariants 明文保护的消费面并新增 repair item 契约，属「Plan 遗漏需要新 repair item、Task Contract 与 Gate 2」的 rework 形态（iteration-planning › Review 分类第 2 行）。已创建 `iterations/000-visibility/002-rework.md`（repair items T6-R2-R1/T6-R2-R2，Plan Context 置 draft 待 Gate 2 用户批准）；design.md D10 增实施修正段（消费面事实更正 + 处方补全 + 健全性论证）。本 Cycle 的 T6-R1/R2/R3 实施与验证结论由 002-rework Inherited scope 采信继承，不重做。

**Iteration Plan Update**

<None>

**Next Cycle**

`iterations/000-visibility/002-rework.md`

**Next Iteration**

<None>
