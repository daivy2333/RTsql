# Iteration 000 / Cycle 002: 事务可见性域收口（002-rework：页级快路径高水位修正）

## Plan Context

- Status: ready
- Iteration: 000-visibility
- Cycle: 002-rework
- Cycle Type: rework
- Parent cycle: 001-replan.md

**Iteration Scope**

- Change tasks: T1-T7（tasks.md Iteration 000；T6 按 design D10 修订——本 Cycle 以 repair item 收口其页级快路径遗留面）
- Depends on: None
- Stable baseline: I033 探针序列扫描空集（运行期 + restart 两态）、未提交删除/回滚扫描语义正确、I032 未提交行重启不复活、RC 可配置且全部 6 验收场景通过（扫描与点查两路径）、分配器重启不复用；默认 RR 全量零回归
- Verification boundary: isolation/mvcc 套件全绿 + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: 页级可见性快路径族（`page_visibility.rs`、`data_scan.rs` 快路径实参、`buffer_pool.rs` 两处）+ 父 Cycle 已建立的快照/分配器回归面
- Deferred tasks: T10-T14（Iteration 001 NLJ）、T20-T22（Iteration 002 子查询缓存）

**Cycle Scope**

- Trigger: rework-required（001-replan Plan Review：Act 阻塞交接成立——三处页级快路径以 `Snapshot::tx_id()`（自身 id）作高水位消费，R2-S2 整页误跳；根因为 Plan 消费面清点遗漏，PLAN-OMISSION）
- Acceptance gaps: transaction-isolation-levels R2-S2（语句间提交可见，实测 `[]`；R2-S1 已由父 Cycle 收口转绿）
- Repair items: T6-R2-R1（三处快路径高水位修正 + 点查见证）、T6-R2-R2（GREEN 收尾）
- Inherited scope: 001-replan T6-R1（Snapshot high_water/statement_view 结构分离）、T6-R2（statement_snapshot 切换）、T6-R3（分配器 advance_past 水位推进）全部实施与验证结论（Plan Review 已核对 diff 并复跑：isolation 5/6、lib transaction 47、mvcc 11，采信）；001-replan 的 Invariants、mvcc-tombstone-visibility 全部场景、000-initial T1-T5 绿面
- Excluded scope: 同 001-replan（NLJ、子查询缓存、SQL/CLI 隔离面、写写冲突检测、RR 真快照化、预存索引时序边界、GC 墓碑回收域）

**Objective**

页级可见性快路径的高水位消费源修正后，R2-S2 转绿且 RC 点查路径同型缺口一并闭合（新增点查见证），`isolation_level_test` 6/6；父 Cycle 全部绿面（mvcc 11/11、lib 47、RR 零回归）维持，Iteration 000 Acceptance 完整达成。

**Background**

001-replan 按 D10 完成 Snapshot 自身身份/高水位分离后 R2-S1 转绿，R2-S2 仍失败。Act 按 Gate 6 交接：Plan 的 Investigation Facts（与 design D10 同源）认定「消费面全部经方法、`tx_id` 保留自身身份语义后两处零改动」，遗漏了页级可见性快路径这一第三类消费面——`all_invisible_for` 的两处调用点与 `check_page_all_visible` 条件 2 直接以 `s.tx_id()`（statement_view 下为自身 id）作页级高水位，`min_create_tx_id ∈ (self, high_water]` 的整页被误判 all-invisible 跳过。Plan Review 独立核实三处消费点与 R2-S2 机理链成立（含 `set_all_visible` → insert `update_visibility_on_insert` → commit `clear_all_visible` 后 `min_create=W` 存留、语句 2 `all_invisible_for(R) = W > R` 的短路序列），裁定 PLAN-OMISSION。修复为三处实参一行级修正 + 访问器新增，验收不变、设计处方不变（D10 分离原则的补全），故按 rework 承载，不修改 Iteration Map。

**Investigation Facts**

- Current Baseline: 001-replan T6-R1/R2/R3 改动在工作区（`git status` 38 个已跟踪修改文件，与本 Cycle 前状态一致）；全量 917 passed / 1 failed / 2 ignored（Act 运行，唯一失败即 R2-S2；Plan 复跑决定性套件一致：isolation 5 passed / 1 failed、`cargo test --lib transaction` 47 passed、mvcc 11 passed）；clippy 0 / fmt 0 / validate 28 PASS（Act）。
- Current-State Evidence（Plan 直接读码核实，file:line）：
  - `PageVisibilityInfo { min_create_tx_id, all_visible }`（`page_visibility.rs:10-13`）；`all_invisible_for(t) = min_create_tx_id > t`（:18-20）；头注释（:6-8）明文把参数记作 `snapshot.tx_id()`——文档与实现同源混淆。
  - 消费点 1：`data_scan.rs:410-413`——DataScan 页快路径 `v.all_invisible_for(s.tx_id())`，真值时整页 `JumpToPage/Done` 跳过（:431-438）。R2-S2 直接短路点。
  - 消费点 2：`buffer_pool.rs:313`——`find_visible_version` all-invisible 快路径 `all_invisible_for(snapshot.tx_id()) → return Ok(None)`。RC 点查同型缺口（本次未触发，语义同型）。
  - 消费点 3：`buffer_pool.rs:441`——`check_page_all_visible` 条件 2 `vh.create_tx_id() >= snapshot.tx_id() → false`。statement_view 下仅对 `create ≤ self` 的旧行放行，多数页不置位（保守、损失优化）；语义应与语句视图一致：`create > high_water` 才不可见。该方法同时是 DataScan 惰性置位 `set_all_visible` 的判定器（`data_scan.rs:505-520`）。
  - R2-S2 机理链（复现确认）：语句 1 空页扫描 → 惰性置位 `set_all_visible(P)`（min_create=u64::MAX）→ auto-commit INSERT → `update_visibility_on_insert(P, W)`（min_create=W，all_visible=false）→ commit `clear_all_visible(P)`（min_create 存留）→ 语句 2 statement_view(high_water=W, self=R) → `all_invisible_for(R) = W > R = true` → 整页跳过 → `[]`。
  - 修复健全性：`statement_view` 保证 self ≤ high_water（reader 分配于语句开始前，auto-commit 为 0）；`create > high_water ⇒ is_visible 规则 2 假 ∧ is_visible_self 假`——`all_invisible_for(high_water)` 的整页跳过语义成立且不放过任何可见行。`new` 构造下 high_water == tx_id → 两处快路径与条件 2 行为逐字节不变（RR 面与既有单测/bench 零改动）。RC 高水位随语句单调不减 → all_visible 旗对后续 RC 视图无陈旧风险；RR 生产路径不传快照、快路径休眠。
  - 点查路径快照穿线已在位：`pipeline.rs:483` IndexScan 臂传语句快照，`IndexScan/IndexScanAll` 经 `find_visible_version` 消费——点查见证测试可直接落 `isolation_level_test.rs`。
  - `Snapshot` 现形态（T6-R1 后）：`{ tx_id, high_water, active_tx_ids }`，`statement_view(high_water, self_tx_id, active)` 构造器、`is_visible` 规则 2 用 high_water、`tx_id()` 访问器无（自身 id 无 getter——`superseder_suppresses` 经 `s.tx_id` 字段消费？核实：`data_scan.rs` 抑制臂使用 `s.tx_id()`——为 `Snapshot` 既有 pub 方法 `tx_id()`。`high_water()` 访问器为对称新增）。
- Code and Critical Path: `page_visibility.rs`（注释同步）→ `data_scan.rs:410-413`（实参）→ `buffer_pool.rs:313`（实参）与 `buffer_pool.rs:441`（条件 2）→ `snapshot.rs`（`high_water()` 访问器）；见证 = `tests/isolation_level_test.rs`（既有 R2-S2 扫描用例 + 新增点查用例）。

**Implementation Guidance**

实施顺序：T6-R2-R1（访问器 + 三处修正 + 点查见证，一次 GREEN 观察）→ T6-R2-R2（收尾门）。关键事实：(1) 三处修正均为实参/比较源一行级变更，控制流不动；(2) `new` 构造下 high_water == tx_id，全部既有快路径行为（含单测/bench 的 `new` 形态）逐字节不变——回归风险集中在 RC 新路径，由 isolation 套件覆盖；(3) 点查见证按既有 `rc_statement_sees_commit_between_statements` 夹具形态改用 `execute_in_tx` 的点查语句即可（同一序列的 WHERE id=5 变体），无需新夹具机制；(4) `page_visibility.rs` 头注释随语义更正（`snapshot.tx_id()` → 快照高水位），纯文档面。

**Behavioral Change**

- 当前：statement_view 下三处页级快路径以自身 id 判页级不可见——R2-S2 扫描整页误跳返回空集；RC 点查同型缺口潜伏；`check_page_all_visible` 在 RC 下多数页不置位（损失优化）。
- 目标：三处统一以 `high_water()` 判定——R2-S2 扫描转绿、RC 点查语句间提交可见（新增见证锁定）、RC 下 `check_page_all_visible` 恢复正确置位；`new`（RR）形态行为逐字节不变。
- 接口/错误/状态语义：`Snapshot::high_water()` 访问器新增（additive）；页级快路径判定源变更（行为语义修正，非新接口）；错误面、WAL/页格式、公共 SQL 语义不变。

**Task Contracts**

### T6-R2-R1: 页级快路径高水位修正 + 点查见证

- Requirement/Scenario: transaction-isolation-levels R2-S2（阻塞场景本体）+ R2 语义的点查路径一致性（mvcc-tombstone-visibility R2「扫描路径与既有索引路径 SHALL 语义一致」同型原则）
- Depends on: None（父 Cycle T6-R1/R2/R3 已完成）
- Targets: `src/transaction/snapshot.rs`（`high_water()` 访问器新增）；`src/executor/data_scan.rs:410-413`（`all_invisible_for` 实参）；`src/storage/buffer_pool.rs:313`（`find_visible_version` 快路径实参）；`src/storage/buffer_pool.rs:441`（`check_page_all_visible` 条件 2）；`src/storage/page_visibility.rs:6-8`（头注释语义更正）；`tests/isolation_level_test.rs`（点查见证用例）
- Current behavior: 三处以 `s.tx_id()`（自身 id）作页级高水位；R2-S2 扫描 `[]`；点查路径无 RC 语句间提交见证
- Required behavior: 三处统一改传/比较 `snapshot.high_water()`；新增 `rc_point_lookup_sees_commit_between_statements`（显式事务内语句 1 点查空 → auto-commit INSERT 提交 → 语句 2 点查得行）；`page_visibility.rs` 头注释与实现对齐
- Required changes: 访问器 + 三处实参/比较源 + 注释 + 1 测试用例
- Preserve: `find_visible_version` 控制流与其余判定（all_visible 快路径、逐 slot 链走）不动；`superseder_suppresses` 与 DataScan 抑制判定不动；`all_invisible_for` 本体语义（`min_create > t`）不动；既有 `new` 形态快路径行为逐字节不变；`check_page_all_visible` 条件 1/3 不动
- Forbidden: 修改 `PageVisibilityInfo` 结构或 `all_invisible_for` 签名；为快路径新增身份型字段；触碰 `is_visible`/`is_visible_self`/`statement_view` 语义（父 Cycle 已收口）；改变 RR 生产路径（快照恒 None）
- Test witness: 既有 `rc_statement_sees_commit_between_statements` 由 FAIL 转 PASS（RED 已在位，observed `[]`）；新增点查用例先于实现建立 RED（预期 observed `[]`/None——与扫描同机理）→ 修正后 GREEN
- GREEN condition: `cargo test --test isolation_level_test` 6 passed / 0 failed（6 既有 + 1 新增 = 7 用例全绿，套件计数 7）
- Verification: `cargo test --test isolation_level_test` + `cargo test --lib transaction`（47 保持）
- Stop when: 修正后仍有意料外场景失败且形态指向快路径之外的语义层——返回 Plan

### T6-R2-R2: Iteration 收尾

- Requirement/Scenario: transaction-isolation-levels R3、mvcc-tombstone-visibility R5（零回归类）
- Depends on: T6-R2-R1
- Targets: 全局
- Current behavior: 全量 917/1/2（1 failed 即 R2-S2）
- Required behavior: isolation 7/7 + mvcc 11/11 + lib transaction 47 + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Required changes: 如全量暴露依赖旧行为的既有测试，按 D7 在 delta spec 记录校准后实施（调查预判为零）；否则仅验证
- Preserve: 校准不放宽断言语义
- Forbidden: 静默放宽既有断言
- Test witness: 全量输出决定性片段
- GREEN condition: 全量 0 failed / 2 ignored（≥918 passed：917 基线 + 点查见证 1）
- Verification: `cargo test` 全量 + clippy/fmt/validate
- Stop when: 全量出现非校准可解失败——返回 Plan

**Invariants**

- 001-replan Invariants 全部继承，唯一下列Enumerated relaxation：`find_visible_version` 与 DataScan 的「本体不动」约束在本 Cycle 限定为——控制流、`superseder_suppresses`、`all_invisible_for` 本体语义、all_visible 快路径判定不动；仅 repair item 点名的三处高水位实参/比较源（`data_scan.rs:410-413`、`buffer_pool.rs:313`、`buffer_pool.rs:441` 条件 2）按契约修正。
- 页格式 22B VersionHeader、WAL 记录格式、文件格式版本不变。
- 默认 RR 路径行为逐字节不变；事务 id 单调不复用；公共 API 仅 additive（`high_water()`）。
- 身份型证据工程禁令。

**Non-goals**

- NLJ（Iteration 001）、子查询缓存（Iteration 002）、SQL/CLI 隔离面、运行中切换、Serializable/SSI、写写冲突检测、RR 真快照化。
- 测试构造 `new` 小 id 快照的快路径陈旧旗预存形状（生产不可达，本修正不扩大、不修）。
- 未提交删除期间点查不可达 / DELETE 回滚后 PK 点查不可达（预存边界，Issue 候选维持）；GC 对墓碑链回收（I038 域）；分配器状态持久化。

**Acceptance**

1. `tests/isolation_level_test.rs` 7/7：R2-S1 脏读排除、R2-S2 语句间提交可见（扫描）、R2-S2 点查见证、R2-S3 语句间删除消失、R2-S4 自身写可见、R2-S5 auto-commit 等价、R1-S2 RC 打开可用——映射 transaction-isolation-levels R1/R2。
2. `tests/mvcc_tombstone_visibility_test.rs` 11/11 保持——映射 mvcc-tombstone-visibility R1-R4 + D10 分配器推进。
3. `cargo test --lib transaction` 47 保持（父 Cycle 单测绿面继承）。
4. 全量回归零修改通过（≥918 passed / 0 failed / 2 ignored）——映射 R3/R5 零回归类。
5. clippy/fmt/validate 全 0/PASS。
Acceptance 与 requirement/scenario/design/task/代码/测试映射见 tasks.md RTM（全部 Covered；repair item 映射 T6-R2/D10）。

**Verification**

- `cargo test --test isolation_level_test`（目标套件，R2-S2 + 点查见证 RED→GREEN）
- `cargo test --lib transaction`（父 Cycle 单测面保持）
- `cargo test --test mvcc_tombstone_visibility_test`（11 保持）
- `cargo test`（全量，零回归门）
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- 决定性输出各取 ≤20 行写入 Act Response。

**Gate 2 Readiness**

- 无 Missing requirement（RTM 全 Covered）：PASS（repair item 映射 T6-R2/D10，见 tasks.md RTM）
- 无 Simplified 未批准：PASS
- 调查完整（三处消费点、机理链、健全性论证、点查穿线均有 file:line 证据；父 Cycle 已核实维度直接引用）：PASS（Investigation Facts + 001-replan Plan Review 复跑）
- 设计闭合（处方补全、健全性论证、预存形状边界已记录）：PASS（design.md D10 修正段）
- 任务可执行（T6-R2-R1/R2 有位置/行为变化/测试见证/停止条件）：PASS（Task Contracts）
- 分轮合理（单 rework Cycle 收口单一 Acceptance gap）：PASS
- 追踪完整：PASS
- 验证充分（R2-S2 双路径 RED→GREEN + 全量零回归 + 工具链门）：PASS（Verification）
- 无身份型证据工程：PASS
- 无实质未知项留 Act：PASS（非实质项见 Risks）
- tasks/specs/design/Iteration/Cycle 一致：PASS（design D10 修正段、本 Cycle、父 Cycle Review 同步）
- Persisted Evidence 模式明确：PASS（none，见下）
- 用户批准计划：PASS（2026-09-13 用户批准「批准，交接 Act」）

**Persisted Evidence**

- Mode: none

理由：全部验收由可重复运行的测试与工具链命令承载；阻塞证据（R2-S2 失败用例）本身可低成本复现，命令、决定性输出、退出码写入 Act Response 即满足公共规则 › 验证。

- Budget: 不适用（none）

**Risks and Notes**

- `check_page_all_visible` 条件 2 修正后 RC 下恢复置位：all_visible 旗对后续 RC 视图的健全性由高水位单调不减保证（Investigation Facts 论证）；若全量暴露与惰性置位/清旗时序相关的既有测试失败，属非实质局部修正（Act Response 记录），不改变契约语义。
- 点查见证依赖 `update_visibility_on_insert`/`clear_all_visible` 后 `min_create` 存留的机理（R2-S2 扫描失败已实证该形态）；若点查路径因索引缓冲等差异未复现 RED，按实际形态记录并回到 Plan（不虚构 RED）。
- 非实质留白（Act 自行决定）：`high_water()` 访问器的文档措辞；点查见证的断言形态（行集/affected）；`page_visibility.rs` 注释的措辞幅度。

## Act Response

- Status: reported

**Implemented**

（原阻塞态已按 Plan Review `Follow-up Decision` 收口；本 Response 为覆盖后的完整当前快照，阻塞期观察保留于 Verification Evidence 与 Blocker Handoff。）

T6-R2-R1（见证改形 + 三处高水位修正）：

1. 见证改形（F4(a) 裁定）：`rc_point_lookup_sees_commit_between_statements` 语句 1 由点查改为全扫描 `SELECT * FROM t`（RC 显式事务内，DataScan 惰性置位建立 vis 条目 `min_create=MAX`），语句 2 保持点查 `SELECT * FROM t WHERE id = 5`；用例文档注释记载见证机理（扫描建立页摘要、索引路径消费——mvcc-tombstone-visibility R2「扫描路径与既有索引路径 SHALL 语义一致」的直接见证）。改形先于实现建立 RED（观察 `5 passed; 2 failed`，两个 R2-S2 用例均 observed `left: []`，与裁定机理 `all_invisible_for(R) = W > R → Ok(None)` 一致）。
2. `Snapshot::high_water()` 访问器新增（additive，文档记载页级快路径应消费可见性高水位而非读者自身 id）。
3. 三处消费点统一改传/比较 `high_water()`：`data_scan.rs:413`（DataScan 页快路径 `all_invisible_for` 实参）、`buffer_pool.rs:313`（`find_visible_version` all-invisible 快路径实参）、`buffer_pool.rs:441`（`check_page_all_visible` 条件 2：`create >= tx_id` → `create > high_water`，与 `is_visible` 规则 2 对齐；`new` 形态下自建行先被条件 1 拒绝，RR 行为不变）。
4. `page_visibility.rs` 头注释更正（比较源 `snapshot.tx_id()` → 快照高水位）。

T6-R2-R2（收尾）：isolation 7/7 + mvcc 11/11 + lib transaction 47 + 全量回归零修改通过（919/0/2 ≥ 918）；clippy/fmt/validate 全 0/PASS。既有测试零校准（契约调查预判「全量暴露依赖旧行为的既有测试为零」成立）。`min_create=0` 毒化按 F4(c) 维持 Issue 候选，未修。

**Changed Files and Symbols**

- `tests/isolation_level_test.rs`：`rc_point_lookup_sees_commit_between_statements` 见证改形（语句 1 点查→全扫描，文档注释记载机理）+ rustfmt 重排（阻塞态引入的非规范换行，纯形式面）。
- `src/transaction/snapshot.rs`：新增 `Snapshot::high_water()`（`tx_id()` 对称访问器，additive）。
- `src/executor/data_scan.rs`：页快路径 `all_invisible_for` 实参 `s.tx_id()` → `s.high_water()`（1 行）。
- `src/storage/buffer_pool.rs`：`find_visible_version` 快路径实参 `snapshot.tx_id()` → `snapshot.high_water()`（1 行）；`check_page_all_visible` 条件 2 `>= snapshot.tx_id()` → `> snapshot.high_water()` + 方法 doc 条目 2 与行内注释同步。
- `src/storage/page_visibility.rs`：`min_create_tx_id` 头注释比较源更正（纯文档面）。
- 父 Cycle（001-replan T6-R1/R2/R3）改动原样保留，本 Cycle 未回滚、未触碰。

**Deviations from Plan**

None——见证改形与实施面均按 Plan Review `Follow-up Decision` 执行（改形本身是 Plan 裁定，非 Act 偏差）。非实质留白按 Plan Risks 授权 Act 自行决定：`high_water()` 文档措辞、`check_page_all_visible` 注释幅度、见证用例断言形态（行集断言，与既有用例一致）。

**Blocker Handoff**（已解决——恢复记录与结果见 Blocker Resolution，实施与验证见上方快照）

- 发现位置：T6-R2-R1 / Test witness 步 / Gate 3（同触 Gate 6「实际代码与契约存在实质冲突」——契约要求新增点查用例先于实现建立 RED，实测修复前即通过）。
- Plan 预期：点查见证依赖「`update_visibility_on_insert`/`clear_all_visible` 后 `min_create` 存留」机理（Investigation Facts：条目在 insert 时不存在 → `or_insert {min_create: W}`），语句 2 `all_invisible_for(R) = W > R = true` → `buffer_pool.rs:313` 短路 `Ok(None)` → observed `[]`/None。
- 实际机理（file:line，已同时解释扫描 RED 与点查通过两个实测结果）：
  1. insert 路径对 vis map 的首次触碰是 `insert.rs:160` `clear_all_visible(page_id)`，其 `or_default()`（`buffer_pool.rs:390-395`）在条目不存在时插入 `{min_create_tx_id: 0, all_visible: false}`；
  2. 随后 `insert.rs:161-162` `update_visibility_on_insert(page_id, W)` 走 `and_modify`：`min_create = min(0, W) = 0`（`buffer_pool.rs:399-410`）；
  3. `min_create = 0` 使 `all_invisible_for`（`page_visibility.rs:18-20`）对任意快照恒 false → `buffer_pool.rs:313` 快路径在该形状下永不触发 → 逐行判定可见 → 用例通过。
  4. 对照：扫描 RED 用例的机理链成立仅因语句 1 扫描先经惰性置位 `set_all_visible`（`data_scan.rs:511-525` → `buffer_pool.rs:378-386` `or_insert {min_create: u64::MAX}`）建立条目，insert 的 `min(MAX, W) = W` 才成立——即「先扫描后写入」是 `min_create = W` 形态的唯一建立路径（`data_page.rs:144` 的 `clear_all_visible` 在 `delete_tuple_from_data_page` 内，与本序列无关）。
- 影响：
  1. 契约处方的点查见证（纯点查序列）**不能证明** `buffer_pool.rs:313` 修复（修复前后均通过），按契约实施将产生无效见证（Gate 4「测试不会因错误原因通过」）；
  2. `:313` 缺口本身真实存在但仅在「语句 1 扫描（建立 `{min_create: W}` 条目）→ 语句 2 点查」序列可达——改用该形状属于测试策略变更，超出契约处方（「同一序列的 WHERE id=5 变体，无需新夹具机制」）；
  3. `min_create = 0` 毒化同时意味：任何首次 vis 条目由写路径建立的页，all-invisible 快路径永久失效（保守安全方向、损失优化），并与 `check_page_all_visible` 条件 2 修正的语义交互需 Plan 一并裁定。
- 任务状态：T6-R2-R1 未开始实施（0/2 repair items；既有扫描 RED 观察与代码复核为已完成准备）；T6-R2-R2 未开始。无部分完成的实施面。
- 工作区：本 Cycle 仅新增上列 1 个测试用例；产品代码零改动。父 Cycle 改动（001-replan T6-R1/R2/R3，38 个已跟踪修改文件）原样保留，未回滚。
- 已通过 Gate：Gate 3 阻塞于首个 repair item 的测试见证步；Gate 4/5 未进入。
- Evidence：None required（偏差机理已全文记录于本 Handoff，两个实测结果各一条命令可复现，见 Verification Evidence）。
- 恢复条件：Plan 裁定（任一）——(a) 点查见证改形为「语句 1 扫描 → INSERT 提交 → 语句 2 点查」并确认三处修正契约不变；(b) 维持原形状并调整 witness/验收结构；(c) 将 min_create=0 毒化纳入设计处方（先行修复或登记 Issue）。用户可解决阻塞或调用 `openspec-plan`；恢复后 Act 自本 Handoff 恢复点继续，重新建立受影响见证。

**Blocker Resolution**

- 恢复依据：本 Cycle Plan Review `Follow-up Decision`（裁定 F1-F4，见证改形 F4(a) 采纳）+ 用户指令「继续实施」（openspec-act 恢复阻塞）。恢复前核实：无后继 Cycle、Review Result 仍为 `pending`、工作区与阻塞交接记载一致。
- 解决办法：T6-R2-R1 见证改形——`rc_point_lookup_sees_commit_between_statements` 语句 1 由点查改为全扫描 `SELECT * FROM t`（RC 显式事务内，DataScan 惰性置位建立 vis 条目 `min_create=MAX`），语句 2 保持点查；用例注释记载见证机理。实施面（`high_water()` 访问器 + 三处消费点 + `page_visibility.rs` 注释）按原契约不变。`min_create=0` 毒化维持 Issue 候选，本 change 不修（F4(c)）。
- 风险：改形见证若仍不复现 RED，按 Plan Risks 预案记录并返回 Plan，不开始第四次同类盲试（Gate 6）。
- 恢复点：T6-R2-R1 测试见证步（改形 → 观察 RED → 实施三处修正 → GREEN），随后 T6-R2-R2 收尾门。
- 所需验证：isolation 7/7 + `cargo test --lib transaction` 47 + mvcc 11/11 + 全量 ≥918/0/2 + clippy/fmt/validate 全 0/PASS。
- 既有见证处置：父 Cycle（001-replan）绿面（mvcc 11、lib 47）覆盖的材料自结论产生后未变化，继续采信并在 Verification Evidence 注明来源；本 Cycle 将修改 `isolation_level_test.rs` 与三处消费点，受影响验证按新见证重跑。
- 执行结果：已按恢复点完成——改形见证 RED 在位、三处修正实施、isolation 7/7 转绿、收尾门全过；Response 已覆盖为完整快照并置 `reported`。

**Self-Review**

- Plan compliance: T6-R2-R1/T6-R2-R2 按 `Follow-up Decision` 与原 Task Contract 完成——改形见证先于实现建立 RED（Gate 3 重建立，未虚构 RED）；三处消费点 + 访问器 + 注释与契约 Targets 逐项对应；Preserve（`find_visible_version` 控制流、`superseder_suppresses`、`all_invisible_for` 本体、`check_page_all_visible` 条件 1/3、既有 `new` 形态快路径行为）与 Forbidden（`PageVisibilityInfo` 结构/`all_invisible_for` 签名/身份型字段/`is_visible` 族语义/RR 生产路径）全部未触。
- Full diff reviewed: 是——本 Cycle 增量 diff = 见证改形（含 fmt 重排）+ 4 个源文件 5 处修改（见 Changed Files and Symbols）；`cargo fmt --check` 修复前全仓唯一 diff 点即见证文件，fmt 未扩散到父 Cycle 文件；无计划外修改。调用点完整性核查：`all_invisible_for` 全仓恰两处生产消费点（`data_scan.rs:413`、`buffer_pool.rs:313`，均已改传 `high_water()`），`check_page_all_visible` 全仓恰一调用者（`data_scan.rs:519`），与 Plan Investigation Facts 一致，无第四消费点。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 0（`min_create = 0` 毒化为 F4(c) 裁定保留的 Issue 候选，非本 Cycle 未解决项，见 Experience Candidates）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 既有扫描 RED 观察（阻塞期基线） | `cargo test --test isolation_level_test` | `5 passed; 1 failed`（`rc_statement_sees_commit_between_statements` failed: left: [], right: [[Number(5), Number(50)]]） | RC 语句间提交可见（扫描路径，R2-S2） | 与 Plan Context 基线一致，RED 在位 |
| 点查用例原处方观察（阻塞期） | 同上（新增用例后） | `running 7 tests … 6 passed; 1 failed`（失败仍仅扫描 R2-S2；`rc_point_lookup_sees_commit_between_statements ... ok`） | RC 点查路径 R2-S2 语义 | **预期 RED 未复现**（修复前即通过）→ 触发阻塞（后经 Plan Review F4(a) 裁定改形） |
| 改形见证 RED（修复前） | `cargo test --test isolation_level_test` | `5 passed; 2 failed`（两个 R2-S2 用例 failed: left: [], right: [[Number(5), Number(50)]]） | R2-S2 扫描 + 点查双路径 | RED 在位，改形见证有效（Gate 3 重建立） |
| T6-R2-R1 GREEN | 同上（三处修正后） | `test result: ok. 7 passed; 0 failed` | transaction-isolation-levels R1/R2 全场景 + mvcc-tombstone-visibility R2 语义一致 | PASS |
| 父 Cycle 单测面 | `cargo test --lib transaction` | `47 passed; 0 failed` | 事务族单测（snapshot/version_chain/manager/session） | PASS（绿面保持） |
| mvcc 套件 | `cargo test --test mvcc_tombstone_visibility_test` | `11 passed; 0 failed` | mvcc-tombstone-visibility R1-R4 + D10 分配器推进 | PASS（绿面保持） |
| 全量回归 | `cargo test`（套件行聚合） | `passed=919 failed=0 ignored=2`（fmt 前后各一次全量，两次一致） | 全局零回归门（≥918 / 0 / 2） | PASS（期间 1 例 lib 套件一次性假失败致 cargo 中止于 230/1/0，同材料重跑即绿——按公共规则 › 验证「已知 flaky 重跑不采信」，记为 I041 佐证观测，非回归） |
| clippy | `cargo clippy --all-targets -- -D warnings` | 0 warning（Finished dev profile；仅环境 `~/.cargo/config` 弃用提示） | 静态分析 | PASS |
| fmt | `cargo fmt --check` | clean（先修复见证文件 1 处 diff） | 格式 | PASS |
| validate | `openspec validate --specs --changes` | `Totals: 28 passed, 0 failed (28 items)` | change/规格结构 | PASS |

**Persisted Evidence**

None required——全部验收由可重复运行的测试与工具链命令承载（Plan Context mode: none）；改形见证的 RED/GREEN 各一条命令可复现，决定性输出已入上表，不满足 Evidence 白名单必要性。

**Experience Candidates**

1 个 Issue 候选（F4(c) 裁定保留，只报告不落账，待用户指令触发 Recorder）：`BufferPool::clear_all_visible` 的 `or_default()`（`buffer_pool.rs:390-395`）在条目不存在时以 `min_create_tx_id = 0` 建条目，insert 路径（`insert.rs:160-162`）先 clear 后 update 的顺序使 `min_create` 被 `min(0, W)` 永久钉在 0——该页 all-invisible 快路径（`page_visibility.rs:18-20` / `buffer_pool.rs:313`）对「写路径首建条目」的页永久失效（保守方向，损失优化）。证据：机理链（Plan Review F1 独立核实）+ 本 Response Verification Evidence 改形见证 RED/GREEN 两行。

另：全量回归期间 lib 套件 1 例一次性假失败（重跑即绿，见 Verification Evidence 全量行）——与已登记 I041（resolve env 测试竞态，约 1/6 假失败源）形态一致，作为 I041 的补充观测记录，不构成新候选。

**Remaining Issues**

1. `min_create = 0` 毒化（Experience Candidates 第 1 条，F4(c) 裁定本 change 不修，属 MS08 实测域）。
2. 既有预存边界不变（Plan Context Non-goals 已列）：未提交删除期间点查不可达 / DELETE 回滚后 PK 点查不可达 / GC 墓碑回收域。
3. lib 套件偶发假失败（I041 已登记，本会话新增 1 例佐证观测）。

**Commit or Diff Reference**

未提交（工作区待用户统一触发）。本 Cycle 完整增量 = 见证改形 + fmt 重排（`tests/isolation_level_test.rs`）+ `Snapshot::high_water()` + 三处消费点修正与注释（`data_scan.rs`/`buffer_pool.rs`/`page_visibility.rs`）；对照基线为父 Cycle 001-replan 后工作区状态（38 个已跟踪修改文件 + 历史未跟踪新测试文件，本 Cycle 未触碰）。

## Plan Review

- Review Result: accepted

**Findings**

- **F1（实施面核实）**：四源文件五处修改全部按契约 Targets 逐项对应——`snapshot.rs` 新增 `high_water()` 访问器、`data_scan.rs:413`（页快路径实参）、`buffer_pool.rs:313`（`find_visible_version` 快路径实参）、`buffer_pool.rs:441`（`check_page_all_visible` 条件 2 `>=` → `>` 高水位，与 `is_visible` 规则 2 对齐；行内注释与 doc 条目同步更新）、`page_visibility.rs` 头注释更正。Preserve 全部未触：`find_visible_version` 控制流、`superseder_suppresses`、`all_invisible_for` 本体语义（`min_create > t`）与签名、`check_page_all_visible` 条件 1/3 保留；`new` 形态下 high_water == tx_id → RR 面与既有单测/bench 逐字节不变。Forbidden 项（`PageVisibilityInfo` 结构、身份型字段、`is_visible` 族语义、RR 生产路径）零触碰。
- **F2（调用点完整性核查）**：全仓 `all_invisible_for` 生产消费恰 2 处（`data_scan.rs:413`、`buffer_pool.rs:313`），均改传 `high_water()`，无第四消费点；`page_visibility.rs:42-44,52` 三处为本体单测（验证 `min_create > t` 的语义），Forbidden 明文保护，保留正确。全仓 `check_page_all_visible` 恰 1 调用者（`data_scan.rs:519`），与 Plan Investigation Facts 一致；惰性置位经高水位后行为合法（RC 高水位单调不减 → all_visible 旗对后续 RC 视图无陈旧风险；RR 恒 None → 路径休眠，零变化）。Plan 在 F1/find_visible_version 的「第三方消费点遗漏」风险已闭合。
- **F3（见证改形有效）**：`rc_point_lookup_sees_commit_between_statements`（`tests/isolation_level_test.rs:154-186`）按 Follow-up Decision F4(a) 改形——语句 1 全扫描 `SELECT * FROM t`（DataScan 惰性置位建立 `{min_create: u64::MAX}` 条目），语句 2 点查 `WHERE id = 5`（`find_visible_version` 消费）。RED 形态：`all_invisible_for(R) = W > R → Ok(None)` observed `[]`；GREEN 后逐行判定可见。改形见证恰为 delta spec R2-S2 场景原文（语句 1 本就是 SELECT），且是 mvcc-tombstone-visibility R2「扫描路径与既有索引路径 SHALL 语义一致」的直接见证——见证力强于原处方。Act 报告改形 RED：`5 passed; 2 failed`（两个 R2-S2 同步转 RED），与机械机理推演一致，未虚构 RED。
- **F4（验证采信）**：本会话独立复跑：isolation `7 passed; 0 failed`、lib transaction `47 passed; 0 failed`、mvcc `11 passed; 0 failed`、clippy 0 / fmt 0 / validate 28 passed/0 failed。Act 全量 919/0/2 结论按公共规则 › 验证采信（材料未变化 + 决定性套件独立复跑 + fmt 仅触及见证文件 1 处，未扩散父 Cycle 文件）。lib 套件 1 例偶发假失败按 Act 标注属 I041 既有项观测，不构成本 change 回归。
- **F5（残留观察非阻塞）**：`min_create=0` 毒化（`or_default` 在写路径首建条目时把 min_create 永久钉 0，导致该页 all-invisible 快路径保守失效）按 Follow-up Decision F4(c) 维持 Issue 候选、不在本 change 修——属 MS08 实测域（先量化再优化），由用户指令触发 Recorder 落账。Act 的 Experience Candidates 草稿完整，证据齐全。

**Deviation Classification**

- 阻塞本体（已收口）：见证处方建立机制假设错误（PLAN-OMISSION，父 Cycle Plan 责任）——通过 Plan Review Follow-up Decision 见证改形 F4(a) 收口，本 Cycle Act 按裁定执行无偏离。
- Act 偏差：None（实施面与契约 Targets 逐项对应、Preserve/Forbidden 全部维持、偏差章节留空）。
- 无 ACT-DEVIATION、PLAN-INVALID、BASELINE-CHANGED、NEW-EVIDENCE。

**Acceptance Gaps**

<None>

**Convergence**

N/A（gap 收口——父 Cycle gap R2-S2 经本 Cycle 改形见证 + 三处修正全部转绿，Isolation R1/R2 + mvcc-tombstone-visibility R1-R4 验收面闭合；Iteration 000 的 stable baseline「RC 可配置且全部 6 验收场景通过」连同 mvcc 11、分配器重启不复用、RR 零回归全部达成）

**Evidence**

- 代码核实：`snapshot.rs::high_water()`、`data_scan.rs:413`、`buffer_pool.rs:313`、`buffer_pool.rs:441`（条件 2 `>` 高水位，与 `is_visible` 规则 2 对齐）、`page_visibility.rs:6-14` 注释更正；调用点完整性见 F2 核查。
- 见证核实：`tests/isolation_level_test.rs:154-186` 语句 1 全扫描建立页摘要、语句 2 点查消费（mvcc-tombstone-visibility R2 直接见证）。
- 复跑：本会话 `cargo test --test isolation_level_test` → 7 passed/0 failed、`cargo test --lib transaction` → 47 passed/0 failed、`cargo test --test mvcc_tombstone_visibility_test` → 11 passed/0 failed、`cargo clippy --all-targets -- -D warnings` → 0 warning、`cargo fmt --check` → 0 diff、`openspec validate --specs --changes` → 28 passed/0 failed（退出码 0）。
- 基线：`git status` 39 个已跟踪修改文件（vs 本 Cycle 前 38 个，+1 为见证改形与四源修改叠加）；父 Cycle 改动原样保留，未回滚。
- Act 验证采信：全量 919/0/2（材料未变化 + 决定性套件独立复跑）。

**Follow-up Decision**

接受。Iteration 000 的 stable baseline 完整达成：I033 探针序列扫描空集（运行期 + restart 两态）、未提交删除/回滚扫描语义正确、I032 未提交行重启不复活、RC 可配置且全部 6 验收场景（扫描与点查两路径）通过；默认 RR 全量零回归（isolation 7/7 + mvcc 11/11 + lib 47 + 全量 919/0/2）。Iteration 000 可宣告完成，下一步展开 Iteration 001（NLJ，T10-T14）。

`min_create=0` 毒化问题非本 change 范围，Act 的 Experience Candidates 草稿（issue candidate 1）由用户指令触发 `openspec-experience-recorder` 决定落账与否；lib 套件偶发假失败作为 I041 补充观测记录，无需新动作。

**Iteration Plan Update**

<None>

**Next Cycle**

<None>

**Next Iteration**

`iterations/001-nlj/000-initial.md`（按 tasks.md Iteration 001 的 T10-T14 展开，由下次 `openspec-plan` 创建）
