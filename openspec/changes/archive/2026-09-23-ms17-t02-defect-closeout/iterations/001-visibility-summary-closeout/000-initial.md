# Iteration 001 / Cycle 000: visibility-summary-closeout 初始执行

## Plan Context

- Status: ready
- Iteration: 001-visibility-summary-closeout
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T5, T6
- Depends on: Iteration 000（surface-defects——Plan Review accepted，2026-09-23；全量门稳定）
- Stable baseline: RC 模式页级摘要无毒化（ISS01 0 毒化与 MAX 毒化双向闭合，快路径恢复可用且未知回落逐行保守正确）；change 收口（全量/clippy/fmt/validate/结构自检）
- Verification boundary: T5 新单测/集成用例全绿 + 既有可见性/隔离套件零修改 + T6 全量验证记录
- Diagnostic boundary: `src/storage/page_visibility.rs`、`src/storage/buffer_pool.rs`、`tests/isolation_level_test.rs`（基建镜像）；T6 为 change 级验证
- Deferred tasks: None

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal T5/T6 承诺 + design D1（Design A 哨兵语义）/D7（验证策略）；Iteration 000 稳定基线（全量 1056 一次绿，采信其 Act Response + accepted Review）
- Excluded scope: 页级快路径性能量化（improvement 域）；MVCC 行级语义变化；ISS 台账落账（Recorder 流程）；IN×JOIN 能力解锁、多列 IN 语义裁决等范围外候选

**Objective**

页级可见性摘要经哨兵语义无毒化——`clear_all_visible` 首建不再注入 0、`set_all_visible` 的 MAX 残留不再与 `all_visible=false` 组合成整页误判；RC 端到端 scan→delete→点查可达；随后 change 全量收口（T6）。

**Background**

MS17-T02 缺陷清账第二棒（proposal Why 节）：ISS01（`or_default()` 0 毒化，台账实证）+ Plan 调查新发现 MAX 毒化（`set_all_visible` 无条目 `or_insert MAX` + 写后 clear 不重置，结构链实证），同函数族同故障域（vis_map 条目生命周期），design D1 裁定哨兵未知语义一并闭合。M21 页级机制仅 RC 快照携带执行激活（design Current-State Evidence 界定），RR 默认路径本就旁路。MS09 Iter000 曾在可见性域三轮拉锯，故独立 Iteration 隔离排障（design D6）。

**Investigation Facts**

- Current Baseline: master `7364bc9` + MS13 实施 + docs sync + Iteration 000 T1-T4 实施（均未提交）；**1056 tests pass / 0 failed / 2 ignored**（Iteration 000 Act 全量 `--no-fail-fast` 一次绿，本 Iteration 未修改其覆盖范围内表面，采信）；clippy/fmt 0、validate PASS（changes 1 + specs 36）。
- Current-State Evidence:
  - **0 毒化链**：`buffer_pool.rs:390-395` `clear_all_visible` 的 `.entry(page_id).and_modify(|i| i.all_visible = false).or_default()`——无条目页首建 `{min=0, all_visible=false}`（`PageVisibilityInfo` derive Default，`page_visibility.rs:12-16`）。INSERT 链 `insert.rs:170-174` 与恢复镜像 `recovery.rs:182-183` 均**先 clear 后 update**；`update_visibility_on_insert`（`buffer_pool.rs:399-410`）`min(0, W)=0` 永久钉零 → `all_invisible_for`（`page_visibility.rs:21-23`，`min > snapshot`）恒 false——「整页不可见」快路径短路永不触发（保守失效：只损性能，语义正确）。
  - **MAX 毒化链**：`buffer_pool.rs:378-386` `set_all_visible` 无条目页 `or_insert {all_visible: true, min_create_tx_id: u64::MAX}`（唯一调用点 `data_scan.rs:522`，页扫毕惰性置位）；该页任一写路径 clear——`delete.rs:77/79`、`update.rs:169/171`、`manager.rs:259`（commit 逐 version 页）、`data_page.rs:144`（standalone write_tuple 路径）——`and_modify` 只置 `all_visible=false`，**min 保持 MAX** → `{MAX, false}` → `find_visible_version`（`buffer_pool.rs:291-316`）`vis_info.all_invisible_for(snapshot.high_water())` = `MAX > hw` 恒真 → `:313-315` `return Ok(None)`；DataScan 快路径同谓词整页跳过——**该页全部行对所有后续 RC 快照不可见（静默漏行），直至该页下一次 INSERT 经 `min(MAX, W)=W` 自愈**。`set_all_visible` 自身 MAX 在 `all_visible=true` 下安全（`:292` all_visible 分支短路先于 `:313` 消费）——危险形态仅「MAX + all_visible=false」组合。
  - **暴露面界定**：`Database::statement_snapshot`（`database.rs:127-139`）RR→`None` / RC→`Some(statement_view)`；`find_visible_version` 三调用方（`scan.rs:65`/`index_scan.rs:82`/`index_scan_all.rs:84`）全部 `if let Some(snapshot)` 门控；DataScan 快路径（`data_scan.rs:404-414`）None 即 false。RR 默认路径 vis_map 条目被写但永不消费——暴露面 = RC lib API（design Current-State Evidence + CLI RR 探针不复现，与界定一致）。
  - **测试基建**：`page_visibility.rs:32-77` 既有 4 单测（真实值语义 `100>50` true / `==100` false / `<150` false、Default 安全、new==default、Clone/Copy）——哨兵守卫不得触碰其断言面。`buffer_pool.rs` 无既有 `#[cfg(test)]` 模块；src 侧构造先例 `data_page.rs:148-160` / `data_scan.rs:555-571`：`tempfile::tempdir()` + `FileStorage::open` + `BufferPool::new(cap, storage)`（vis_map 操作无需 TableManager）。RC 集成镜像：`tests/isolation_level_test.rs` 的 `Database::open_with_isolation(&path, IsolationLevel::ReadCommitted)` + `assert_affected`/`rows`/`create_t` helper（7 既有测试零修改）。
  - **哨兵归一注记**：`set_all_visible` 的 `or_insert` MAX **原样保留**——修复后其值与 `MIN_CREATE_UNKNOWN` 同值、语义归一；`all_visible=true` 下不被 `all_invisible_for` 消费（短路先行），写后 clear 使条目变 `{UNKNOWN, false}` → `all_invisible_for`=false 回落逐行，MAX 毒化闭合，无需改 `set_all_visible`。
- Code and Critical Path:
  - 变更面：`src/storage/page_visibility.rs`（新增 `pub const MIN_CREATE_UNKNOWN: u64 = u64::MAX` + `all_invisible_for` 哨兵守卫 + doc 注记）、`src/storage/buffer_pool.rs::clear_all_visible`（`or_default()` → `or_insert(PageVisibilityInfo { min_create_tx_id: MIN_CREATE_UNKNOWN, all_visible: false })`）。
  - 数据流：写路径建条目/翻 flag（6 处 clear / 1 处 set / 2 处 update）→ 读路径（快照携带）`all_visible` 短路 → `all_invisible_for` 整页跳过 → 修复后未知哨兵一律回落逐行检查（保守且正确）。
  - 语义注记：修复后 `min_create_tx_id = 0` 只可能来自真实 tx_id 0（aborted 标记位；MS06-T01 后 DML 不产生）→ `0 > hw` 恒 false，保守无害——`Default` `{0, false}` 语义不需要也不得变（既有单测锁定）。

**Implementation Guidance**

顺序：page_visibility.rs 常量+守卫与哨兵单测（先观察 RED：现状 `u64::MAX` 输入 `all_invisible_for` 为 true）→ buffer_pool.rs `clear_all_visible` 改 or_insert 与毒化链单测（RED：MAX 链残留 `min==MAX`、0 链 `min==0`）→ RC 集成用例（先 RED：点查空集）→ T6 收尾。`clear_all_visible` 修改为单表达式替换；哨兵守卫仅加 `!= MIN_CREATE_UNKNOWN &&` 前置条件。测试落点（buffer_pool 新测试模块、RC 集成新建文件 vs 追加 `isolation_level_test.rs`）为非实质选择，按仓库先例就近决定。

**Behavioral Change**

- 修复前：RC 下两毒化形态——0 毒化使「整页不可见」快路径永不短路（性能保守失效）；MAX 毒化使「扫描→删改」后的页整页误判不可见（静默漏行直至该页下一次 INSERT 自愈）。
- 修复后：未知哨兵（`MIN_CREATE_UNKNOWN`）一律回落逐行检查；INSERT 首建后摘要合并真实 `create_tx_id`（`min(UNKNOWN, W)=W`）；「哨兵残留 + all_visible=false」不再整页误判。RR 默认路径逐字节不变（M21 本就旁路）；行级 MVCC 语义零变化。

**Task Contracts**

### T5: ISS01 + MAX 毒化页级摘要哨兵修复

- Requirement/Scenario: mvcc-tombstone-visibility（delta）新增 Requirement「页级可见性摘要无毒化（哨兵语义）」S1（INSERT 首建真实 min）/ S2（写后清除不整页误判）/ S3（RC 端到端可达）/ S4（既有语义零回归）
- Depends on: None（Iteration 000 全量门为 Iteration 级依赖，非本任务依赖）
- Targets: `src/storage/page_visibility.rs`（常量 + `all_invisible_for`）、`src/storage/buffer_pool.rs::clear_all_visible`
- Current behavior: 见 Investigation Facts 两毒化链——`clear_all_visible` 首建 `{0, false}`（`buffer_pool.rs:390-395`）；`all_invisible_for` 对 `u64::MAX` 输入返回 true（`page_visibility.rs:21-23`）；MAX 链残留条目使 `find_visible_version` `:313-315` 整页 `Ok(None)`
- Required behavior: `all_invisible_for` 对 `MIN_CREATE_UNKNOWN` 返回 false（回落逐行）；`clear_all_visible` 首建 `{MIN_CREATE_UNKNOWN, false}`；INSERT 序后 `min_create_tx_id == W`；RC 端到端 scan→delete→点查与全扫可达
- Required changes: page_visibility.rs 新增 `pub const MIN_CREATE_UNKNOWN: u64 = u64::MAX;`（与 `PageVisibilityInfo` 同址）；`all_invisible_for` 改为 `self.min_create_tx_id != MIN_CREATE_UNKNOWN && self.min_create_tx_id > snapshot_tx_id`；doc 注释更新（哨兵语义 + MS17-T02 收口注记）；`clear_all_visible` `or_default()` → `or_insert(PageVisibilityInfo { min_create_tx_id: MIN_CREATE_UNKNOWN, all_visible: false })`
- Preserve: `Default`/`new` 语义 `{0, false}` 不变（既有 4 单测零修改锁定）；`set_all_visible`/`update_visibility_on_insert`/`check_page_all_visible` 本体零改动（`set_all_visible` 的 `or_insert` MAX 原样保留，语义经哨兵归一）；全部调用方与调用序零改动、不重排（`insert.rs:172-174` / `delete.rs:77,79` / `update.rs:169,171` / `manager.rs:259` / `recovery.rs:182-183` / `data_page.rs:144`）；`find_visible_version` 消费结构（`:291-316` all_visible 短路先于 all_invisible_for）零改动；行级 MVCC 可见性语义（`is_visible`/`is_visible_self`/墓碑抑制）零触碰
- Forbidden: 修改任何执行器/恢复/事务代码；重排 insert.rs clear/update 调用序（design 拒绝的替代方案 c）；新增 CLI 隔离级别旗标（DA5：RC 端到端用 lib API）；性能 bench 设施；将 `Default` 改为 UNKNOWN
- Test witness（RED 先行）:
  - page_visibility.rs 单测新增：UNKNOWN 哨兵 `all_invisible_for(any_hw)` == false（现状 `u64::MAX` 输入 → true，RED）；真实值语义保持断言（与既有 `test_all_invisible_when_min_gt_snapshot` 同形）
  - buffer_pool.rs 新测试模块：`set_all_visible(p)`（无条目）→ `clear_all_visible(p)` → `get_visibility(p)` 的 `all_invisible_for(0)` == false（现状 true，RED）；`clear_all_visible(p)` → `update_visibility_on_insert(p, W)` → `min_create_tx_id == W`（现状 0，RED）
  - RC 集成用例（镜像 `isolation_level_test.rs` 基建）：三行已提交表 → 全表扫描（触发惰性置位）→ DELETE 一行 → 未删行键位点查可达 + 全扫返回两行（现状点查空集 → RED）
- GREEN condition: 新单测/集成用例全绿 + `tests/mvcc_tombstone_visibility_test.rs` / `tests/isolation_level_test.rs` / `tests/gc_test.rs` / `tests/version_chain_test.rs` 零修改全绿
- Verification: `cargo test --lib page_visibility`、`cargo test --lib buffer_pool`、`cargo test --test isolation_level_test --test mvcc_tombstone_visibility_test`（决定性输出 ≤10 行/项，退出码 0；RED 记录写入 Act Response）
- Stop when: 哨兵守卫导致既有可见性断言失败且无法归因为本缺陷校准；或 RC 集成 RED 无法构造（毒化结构链断裂——返回 Plan，不得放宽断言或绕过惰性置位造假见证）

### T6: 收尾全量验证与 change 结构自检

- Requirement/Scenario: 全部域零回归（RTM「全部域零回归」行，design D7）
- Depends on: T5
- Targets: change 级验证，无产品代码变更面
- Current behavior: Iteration 000 末全量 1056 一次绿（采信）；clippy/fmt 0、validate PASS
- Required behavior: T5 合入后全量零回归 + 四项静态/结构门通过 + change 结构自检
- Required changes: 无代码变更；执行全量 `cargo test --no-fail-fast`（基线 1056 + T5 新增，零回归）、`cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` 0、`openspec validate` PASS；结构自检——change tasks 状态与实际完成一致、specs/design 与已实现行为一致、Iteration 与 Cycle 文件齐全、`Review Result` 与流程状态一致
- Preserve: 既有测试套件零修改（T5 新增用例除外）
- Forbidden: 为判定验证结果新增脚本/封装/判定器；重跑已通过验证增强信心
- Test witness: T6 各命令决定性输出与退出码记入 Act Response（全量输出 ≤20 行）
- GREEN condition: 全量 0 failed、clippy/fmt 0、validate PASS、自检各项一致
- Verification: 上述四命令 + 结构自检对照（逐项在 Act Response 列结论）
- Stop when: 全量出现非预期失败（先对照 Iteration 000 Act Response Changed Files 定位归属，再判回归或基线漂移；无法归属时返回 Plan）

**Invariants**

- 行级 MVCC 可见性语义（is_visible / superseder_suppresses / 墓碑抑制 / RC 语句视图）零触碰——本 Iteration 只修摘要信息质量。
- RR 默认路径（无快照）行为逐字节不变。
- vis_map 为进程内存态：无磁盘格式变化、无恢复语义变化（recovery.rs 调用序镜像保持）。
- 既有公共 API、退出码分类、plan cache 不变。
- 既有测试套件零修改通过（新增用例除外；无校准项——调查确认无既有断言锁定被改行为）。

**Non-goals**

- 页级快路径性能量化（improvement 域，I057 同域）；bench 设施。
- MVCC 行为语义扩展；`Default` 语义变更；`set_all_visible` 签名或算法变更。
- ISS 台账落账（Recorder 按用户指令流程）；范围外 improvement 候选落账。

**Acceptance**

- T5：delta spec「页级可见性摘要无毒化（哨兵语义）」S1-S4 全部有测试见证——S1 单测 `min==W`、S2 单测 `all_invisible_for==false`、S3 RC 集成用例、S4 既有套件零修改全绿（requirement→scenario→design D1→T5→`page_visibility.rs`/`buffer_pool.rs`→新单测+集成用例链路）。
- T6：全量 `--no-fail-fast` 零回归（基线 1056 + 新增）+ clippy/fmt 0 + validate PASS + change 结构自检通过（design D7→T6→命令输出链路）。

**Verification**

- T5：`cargo test --lib page_visibility` → 新哨兵用例 + 既有 4 用例全绿，exit 0；`cargo test --lib buffer_pool` → 新毒化链用例全绿，exit 0；`cargo test --test isolation_level_test --test mvcc_tombstone_visibility_test` → 零修改全绿 + RC 集成新用例全绿，exit 0。RED 记录（哨兵现状 true / 毒化链残留值 / 点查空集）写入 Act Response。
- T6：`cargo test --no-fail-fast`（决定性输出 ≤20 行）；`cargo clippy --all-targets -- -D warnings` → 0 warning；`cargo fmt --check` → 0 diff；`openspec validate` → PASS。每项判定以被测命令退出码与原生输出为准，无判定层。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 7 行全部 Covered（PASS；`tasks.md` RTM——本 Iteration 对应 mvcc 新 Requirement 行与「全部域零回归」行）。
- 无 Simplified requirement（PASS；proposal Out of Scope 均非本 change 范围）。
- 调查完整：两毒化链五点读码互证（design Current-State Evidence 行号在本 Cycle 全部复核命中）；消费面与暴露面界定齐备；测试基建先例定位（PASS；Investigation Facts）。
- 设计闭合：D1 哨兵语义 + 拒绝替代方案（b/c）+ 四形态闭合推演（INSERT/写后清除/孤立 clear/恢复镜像）（PASS；design D1）。
- 任务可执行：T5/T6 契约有代码位置、行为变化、RED 见证与停止条件（PASS；Task Contracts）。
- 分轮合理：Iteration Plan 两轮（000 accepted）；001「高危域修复 + change 级验证闭环」内聚，平衡审计已记录（PASS；tasks.md Iteration Plan + design D6）。
- 追踪完整：requirement→scenario→design→task→代码→测试链路齐备（PASS；RTM）。
- 验证充分：S1-S4 逐场景最简直接判定（单测值断言 + RC e2e + 全量零回归），无身份型证据工程/判定层（PASS；D7）。
- 无需 Act 决定的实质未知项：RC e2e RED 可构造性有结构链推演与 design 探针支撑，构造失败走 T5 Stop when 返回 Plan——非实质未知项已入 Risks（PASS）。
- OpenSpec tasks/specs/design/当前 Iteration/Cycle 一致（PASS；本文件与 tasks.md/design 同源撰写）。
- Persisted Evidence 明确：两 Iteration 均 `none`（PASS；白名单核对——各验证命令输出 ≤20 行可入 Act Response，无不可低成本重跑项）。
- 计划批准：change 规划（含 Iteration Map 与 D1 设计方向）经用户 2026-09-23 批准（tasks.md MS17 条目 + proposal Gate 1 记录）；本 Cycle 为该 Map 的既定展开。

**Persisted Evidence**

- Mode: none

（Act Response 承载全部验证输出；无不可重跑项，无 Evidence 预算消耗。）

- Budget: 按公共规则 Evidence 预算执行（0 文件）。

**Risks and Notes**

- RC e2e RED 的前置是首扫 `check_page_all_visible` 通过（全部行已提交且 create_tx ≤ hw）——夹具须先提交全部 INSERT 再扫描（单连接顺序 auto-commit 即可满足）；若时序导致惰性置位未发生，按 T5 Stop when 返回 Plan。
- `min_create_tx_id = 0` 修复后仍可能来自真实 tx 0（aborted 标记位）——`0 > hw` 恒 false，保守无害（design D1 语义注记）；不得「顺手」改 `Default`（Preserve 锁定）。
- 全量基线 1056 为 Iteration 000 采信结论；T6 计数不符时先对照其 Act Response Changed Files 复核归属，再判回归或基线漂移。
- MAX 毒化的 `min(MAX, W)` 自愈窗口随本修复闭合；修复前 RC 用户如遇「重启→扫描→删改→点查空」即本缺陷现象（README 已知限制不涉及——RC 为 lib API 特性）。

## Act Response

- Status: reported

**Implemented**

T5、T6 全部按 Task Contract 完成，RED→GREEN 见证齐全：

- T5（ISS01 + MAX 毒化）：`src/storage/page_visibility.rs` 新增 `pub const MIN_CREATE_UNKNOWN: u64 = u64::MAX`（含哨兵语义 + MS17-T02 收口注记 doc，置于 struct 之前保证 doc 归属正确）；`all_invisible_for` 改为 `self.min_create_tx_id != MIN_CREATE_UNKNOWN && self.min_create_tx_id > snapshot_tx_id`（与 design D1 逐字一致）；struct 字段 doc 补哨兵语义注记。`src/storage/buffer_pool.rs::clear_all_visible` 的 `or_default()` 改 `or_insert(PageVisibilityInfo { min_create_tx_id: MIN_CREATE_UNKNOWN, all_visible: false })`（doc 注明首建哨兵与 min() 合并依据）；`src/storage/mod.rs` 补 `pub use page_visibility::MIN_CREATE_UNKNOWN`（导入路径必要后果）。`set_all_visible`/`update_visibility_on_insert`/`check_page_all_visible`/`find_visible_version` 本体与全部调用方零改动。
- T5 测试见证（RED 先行）：`page_visibility.rs` 追加 2 单测（UNKNOWN 哨兵回落 + 真实值语义保持）；`buffer_pool.rs` 新增 `#[cfg(test)] mod tests`（基建镜像 `data_page.rs` 先例：tempdir + FileStorage + BufferPool，vis_map 操作无需 TableManager）2 单测（MAX 链 set→clear 残留 + 0 链首建钉零，含 S1 THEN t<W/t≥W 断言）；`tests/isolation_level_test.rs` 追加 1 RC 端到端用例（restart→scratch 水位抬升→scan 置位→delete→点查与全扫可达，既有 7 用例零修改）。
- T6：全量验证与 change 结构自检（见 Verification Evidence；tasks 状态行已同步、delta spec S1-S4 与 design D1 逐点核对一致、Iteration/Cycle 文件齐全、Review Result 与流程状态一致）。

**Changed Files and Symbols**

- `src/storage/page_visibility.rs` — `MIN_CREATE_UNKNOWN` 常量（新）、`PageVisibilityInfo` doc（扩）、`all_invisible_for`（哨兵守卫 + doc）、tests 模块 +2（`test_all_invisible_unknown_sentinel_falls_through`、`test_real_values_unchanged_by_sentinel_guard`）；既有 4 单测零修改。
- `src/storage/buffer_pool.rs` — import 补 `MIN_CREATE_UNKNOWN`；`clear_all_visible`（`or_default()` → `or_insert` + doc）；新 `#[cfg(test)] mod tests`（`setup` + `cleared_page_after_set_all_visible_never_reports_all_invisible` + `insert_after_first_build_records_real_min_create`）。
- `src/storage/mod.rs` — +1 行 `pub use page_visibility::MIN_CREATE_UNKNOWN`。
- `tests/isolation_level_test.rs` — +1 用例 `rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart`（含 doc 注记夹具机理与预存缺陷指引）；既有 7 用例零修改。

**Deviations from Plan**

1. **RC 集成夹具校准（scratch 表水位抬升 + 显式作用域 close）**：契约夹具简图（三行已提交表 → 全表扫描 → DELETE → 点查/全扫）依赖两个现状事实，本会话探针实证后校准——(a) Rust 变量遮蔽不提前 drop：第一会话不显式作用域 + `close()` 则 advisory 文件锁不释放，reopen 报 `DatabaseLocked`（首跑 RED 即此形态，属夹具缺陷非毒化见证）；(b) 更实质：close() 的 checkpoint 截断 WAL → reopen `redo_count == 0` → `advance_past` 未被告知 → 分配器归零 → RC 语句 `high_water == 0`，修复前**契约前置条件本身不可达**（重启后三行已提交行对 RC 不可见，首版夹具 RED 于前置断言 0 行）。且 MAX 毒化的 `set_all_visible` `or_insert` 置位要求 vis_map 无条目（进程内任一写路径先到即建条目）→ restart 是毒化链可达的必要条件，不可用进程内序列替代。校准：reopen 后先对 **scratch 表**执行 6 INSERT + 6 DELETE 抬升分配器水位越过重启前 tx id（scratch 写仅建 scratch 页条目，t 表页面保持无条目，扫描经 `or_insert` 置位 `{MAX, true}`）。**断言语义未放宽**：正式 RED 见证为毒化签名本身（点查 `[]` vs `[[2, 20]]`），GREEN 断言与契约一致。发现 (b) 属 T5 范围外预存缺陷，见 Experience Candidates 1。
2. **fmt 首跑修复（记录）**：`cargo fmt --check` 首跑 `buffer_pool.rs` import 块与 `isolation_level_test.rs` 换行（本 Cycle 新增代码）→ `cargo fmt` 后归零；工具结果确认仅触及本 Cycle 两文件，无计划外修改。clippy 首跑即 0。
3. **GREEN 后局部补强（记录）**：按 delta spec S1 THEN 补 `all_invisible_for(6)` / `!all_invisible_for(7)` 两断言（契约 Test witness 仅要求 `min == W`，补强使单测对齐场景全文）；修正 `buffer_pool.rs` doc intra-doc 链接笔误（`PageVisibilityInfo::MIN_CREATE_UNKNOWN` → `MIN_CREATE_UNKNOWN`，避免 `cargo doc` 告警）。均为测试/文档局部，修后重跑受影响套件与全量（修改后必需重跑，非重复增强）。

**Blocker Handoff**

None required

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: 变更面与契约 Targets 精确一致（`page_visibility.rs` 常量+守卫+doc、`buffer_pool.rs::clear_all_visible`）+ `mod.rs` 导出（必要后果）。Preserve 逐项核对：`Default`/`new` 语义 `{0, false}` 未动（既有 4 单测零修改通过）；`set_all_visible`（`or_insert` MAX 原样，语义经哨兵归一）/`update_visibility_on_insert`/`check_page_all_visible`/`find_visible_version` 消费结构（all_visible 短路先于 `all_invisible_for`）本体零改动；六个调用方（`insert.rs` / `delete.rs`×2 / `update.rs` / `manager.rs` / `recovery.rs` / `data_page.rs`）零触碰、调用序未重排；行级 MVCC（`is_visible`/`is_visible_self`/`superseder_suppresses`/RC 语句视图）零触碰。Forbidden 面（执行器/恢复/事务代码、CLI 旗标、bench、Default 改 UNKNOWN）未进入 diff。
- Full diff reviewed: 是。本 Cycle diff 限 4 文件（上列，逐行核对）；工作区其余改动均为 Plan Context 声明基线（MS13 实施 + 收尾 docs sync + Iteration 000 T1-T4，未提交）。跨任务交互：T5/T6 无代码交互；与 Iteration 000 变更面（resolve/error/subquery/query/lifecycle + 对应测试）零重叠。无计划外修改。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: 无（S1 补强与 doc 链接已在 Cycle 内处理，Deviations 3）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T5 RED（哨兵） | `cargo test --lib page_visibility`（实施前） | `1 failed`（`test_all_invisible_unknown_sentinel_falls_through`：UNKNOWN 输入返回 true）；`5 passed` | 哨兵误判现状形态 + 既有 4 单测不受测试代码影响 | PASS（预期 RED） |
| T5 RED（毒化链） | `cargo test --lib buffer_pool`（实施前） | `2 failed; 0 passed`（`left: 0, right: 7`——min 钉零现状） | MAX 链残留 true / 0 链钉零现状 | PASS（预期 RED） |
| T5 RED（RC 端到端） | `cargo test --test isolation_level_test rc_scan_then_delete`（实施前） | 首跑 `DatabaseLocked`（夹具缺陷，Deviations 1a）；次跑前置断言 `left: 0, right: 3`（发现预存缺陷，Deviations 1b）；校准后正式 RED：点查 `left: [], right: [[Number(2), Number(20)]]` | 毒化签名本身（非夹具伪失败） | PASS（预期 RED） |
| T5 GREEN | `cargo test --lib page_visibility` / `--lib buffer_pool` | `6 passed; 0 failed` / `2 passed; 0 failed`，exit 0 | 哨兵守卫 + or_insert 首建 + 既有 4 单测零修改 | PASS |
| T5 GREEN（端到端） | `cargo test --test isolation_level_test` | `8 passed; 0 failed`（7 既有零修改 + 1 新增），exit 0 | RC 端到端可达 + 隔离面零回归 | PASS |
| T5 边界 | `cargo test --test mvcc_tombstone_visibility_test` / `--test gc_test` / `--test version_chain_test` | `11 passed` / `3 passed` / `3 passed`，全部 `0 failed` | 既有可见性/恢复两态/回滚面零修改 | PASS |
| T6 全量 | `cargo test --no-fail-fast`（最终代码态） | **1061 passed; 0 failed; 2 ignored**（= 基线 1056 − 0 + 新增 5），exit 0 | 全仓测试面 | PASS |
| T6 clippy | `cargo clippy --all-targets -- -D warnings` | `Finished`，0 代码告警（仅环境级 `~/.cargo/config` 弃用提示），exit 0 | 全目标静态分析 | PASS |
| T6 fmt | `cargo fmt --check`（首跑 2 处新增代码 diff → `cargo fmt` → 归零） | clean，exit 0 | 全仓格式 | PASS |
| T6 OpenSpec | `openspec validate --changes` / `--specs` | `1 passed, 0 failed` / `36 passed, 0 failed`，exit 0（changes 侧有归档期 Purpose 占位 WARNING——Iteration 000 起既有 change 级提示，非验证失败，归档时由 Maintainer 处理） | change 结构 + specs 语料库 | PASS |
| 结构自检 | 对照检查 | tasks T1-T6 状态与实际一致；delta spec S1-S4 与 design D1 逐点核对与实现一致；2 Iteration × 000-initial 文件齐全；Review Result 000 accepted / 001 pending（本 Response） | change 结构 | PASS |

**Persisted Evidence**

None required（Plan Context Mode: none；全部验证命令可低成本重跑，Act Response 承载决定性输出）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Issue（范围外实质缺陷） | RC 重启后可见性回归：close()（checkpoint 截断 WAL）→ reopen → `redo_count == 0` → `advance_past` 未被告知 → 分配器归零 → RC `statement_view` `high_water == 0` → 重启前已提交行（create_tx > 0）对 RC 不可见，直至新事务分配 id 渐进越过旧水位才「复现」。RR 无快照路径不受影响。 | 本 Cycle 夹具探针：close+reopen RC 后 `SELECT` 返回 0 行（`tests/isolation_level_test.rs` 前置断言失败现场，Deviations 1b）；机理链 `database.rs:122-129`（`current_tx_id()` 作 high_water）+ `tx_id.rs::advance_past` 仅由恢复重放驱动 + checkpoint 截断语义（`checkpoint_redo_reduction_test` S2.1 对照：RR reopen 有行） | T5 Preserve/Forbidden 禁止进入事务/恢复代码，未修复；修复方向（如恢复后从数据页派生水位或 checkpoint 持久化水位）需 Plan 调查。建议 Recorder 落账 |
| Runbook（可重复测试基建） | 进程内 reopen 配方：(a) 第一会话须显式作用域 + 块尾 `close()`——变量遮蔽不提前 drop，advisory 文件锁在原绑定存活期间不释放（`DatabaseLocked`）；(b) RC 场景 reopen 后须先经 DML（他表）抬升分配器水位越过重启前 id，已提交行才对 RC 可见（上项缺陷修复前的 workaround） | 本 Cycle Deviations 1 + `tests/isolation_level_test.rs` 追加用例夹具 | MS17-T01 加密测试（with/without key reopen）将复用该路径；两坑均为本会话实证（DatabaseLocked 首跑 + 前置 0 行） |
| 台账注记（非新候选） | Iteration 000 Experience Candidates 第 3 行（ISS01 台账补记 MAX 毒化方向与修复结果）的修复面已由本 Cycle 落地 | 本 Response Implemented T5 | ISS01 台账补记仍待 Recorder 按用户指令执行 |

**Remaining Issues**

- 无阻塞遗留。范围外发现（RC 重启可见性回归）已作为 Issue 候选报告，交 Recorder，不落账、不影响本 change Acceptance。

**Commit or Diff Reference**

未创建 commit（工作区含基线 MS13 实施与 docs sync 待用户统一提交；本 Cycle 改动以本 Response Changed Files 清单为准）

## Plan Review

- Review Result: accepted

**Findings**

- **F1（非阻塞，Deviation 1 分类）**：契约夹具简图（三行已提交 → 扫描 → DELETE → 点查/全扫）经 Act 探针证实**按原样不可达**——(a) Rust 变量遮蔽不提前 drop，advisory 锁不释放（夹具笔误级）；(b) close() checkpoint 截断 → reopen `redo_count == 0` → `advance_past` 未被告知 → RC `high_water == 0`，前置断言（重启后三行可见）本身失败于预存缺陷。Act 校准（显式作用域 + close；scratch 表 DML 抬水位）**未放宽断言语义**：正式 RED 见证为毒化签名本身（点查 `[]` vs `[[2, 20]]`），GREEN 断言与契约 Required behavior 一致（本 Review 读码核对 `tests/isolation_level_test.rs` 新用例：precondition 3 行 / DELETE affected 1 / 点查 `row(2,20)` / 全扫 2 行）。(a) 归 `PLAN-INVALID`（Plan 侧夹具简图缺陷）；(b) 归 `NEW-EVIDENCE`（范围外新缺陷发现，见 F2），夹具校准在 Risks 预授权与 Stop-when 边界内完成——合规。
- **F2（非阻塞，NEW-EVIDENCE——Issue 候选机理独立确认）**：Experience Candidate 1（RC 重启后可见性回归）经本 Review 独立读码证实机理链成立——`database.rs:79-86` `advance_past(max_tx_id)` 仅由恢复观测 id 驱动（committed/aborted/uncommitted 三集合并集取 max，checkpoint 截断后三集皆空 → max=0）；`statement_snapshot` RC 臂（`database.rs:127-139`）以 `current_tx_id()` 为 high_water；`database.rs:77-81` 注释自证该前提即 RC 高水位健全性基础（"every id ≤ the allocator's current value is committed, aborted, or active"）——checkpoint 截断使其失效。预存缺陷、与 T5 哨兵修复正交（T5 Preserve/Forbidden 禁止进入事务/恢复域）、不阻塞本 Iteration Acceptance（e2e workaround 合法：被测断言为毒化签名而非水位）。正确报告为 Issue 候选，落账走 Recorder，本 Review 不落账。
- **F3（非阻塞，Deviation 2）**：fmt 首跑 2 处（本 Cycle 新增代码 import 块/换行）→ `cargo fmt` 归零——新增代码首跑修复，非已通过验证的重试，Gate 5 无违例（与 Iteration 000 F4 同性质）。
- **F4（非阻塞，Deviation 3）**：GREEN 后按 delta spec S1 THEN 补 `all_invisible_for(6)` / `!all_invisible_for(7)` 两断言 + doc intra-doc 链接修正——测试/文档局部补强使单测对齐场景全文，修改后重跑受影响套件属「修改后必需重跑」而非重复增强，合规；断言与 spec S1 THEN 逐字一致（本 Review 读码核对）。
- **F5（Minor，记录，不处理）**：delta spec S2 GIVEN「惰性置位（条目首建携带 UNKNOWN 哨兵）」——代码 `set_all_visible` 的 `or_insert` 仍写字面 `u64::MAX`（design D1 明确「原样保留，语义归一」），数值上 `MIN_CREATE_UNKNOWN == u64::MAX`，spec 陈述数值成立、与 design 无矛盾。无需动作。
- **F6（记录，无需动作）**：e2e 夹具的 scratch 水位抬升是 F2 缺陷修复前的 workaround；该缺陷未来修复后本段冗余但无害（水位抬升仍合法）。未来 Issue 修复 change 可顺带简化夹具，本 Cycle 不动。
- **F7（记录，无需动作）**：`openspec validate` changes 侧归档期 Purpose 占位 WARNING 为 change 级既有提示（Iteration 000 起已记录），归档时由 Maintainer 处理，非本 Cycle 验证失败。
- **独立代码检查**：四文件 diff 逐行核对与 Task Contract/Act Response 声明精确一致——`page_visibility.rs`：常量 + doc（置于 struct 前，doc 归属正确）、守卫 `!= MIN_CREATE_UNKNOWN && >`（与 design D1 逐字一致）、+2 单测、既有 4 单测零修改；`buffer_pool.rs`：仅 import + `clear_all_visible` `or_insert` + doc + 新测试模块（setup + 2 用例），`set_all_visible`/`update_visibility_on_insert`/`check_page_all_visible`/`find_visible_version` 均不在 diff（零改动属实）；`storage/mod.rs`：+1 行 pub use；`isolation_level_test.rs`：+1 e2e 用例（doc 注记夹具机理与预存缺陷指引），既有 7 用例零修改。Preserve 面（六调用方文件）diff 中零可见性相关改动（`insert.rs`/`update.rs` 工作区改动均为 MS13 日期域）。RED 记录与修复前语义逻辑一致（`MAX > hw` 恒 true / `or_default` → 0 / `min(0,W)=0`）。Forbidden 面未进入变更面。

**Deviation Classification**

- Deviation 1（RC 集成夹具校准）：(a) `PLAN-INVALID`（夹具简图笔误）、(b) `NEW-EVIDENCE`（预存缺陷发现）——均非阻塞；断言语义未放宽，校准在 Risks 预授权与 Stop-when 边界内。
- Deviation 2（fmt 首跑）：非偏差（F3）。
- Deviation 3（S1 断言补强 + doc 链接）：非偏差，测试/文档局部完善（F4）。

**Acceptance Gaps**

None —— T5：S1 ↔ `insert_after_first_build_records_real_min_create`（`min==7` + `all_invisible_for(6)` true / `(7)` false，与 S1 THEN 逐字一致）、S2 ↔ `cleared_page_after_set_all_visible_never_reports_all_invisible`、S3 ↔ `rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart`、S4 ↔ 既有 4 单测零修改 + 全量零回归，requirement→scenario→test 链路闭合；T6：全量 1061 / clippy 0 / fmt 0 / validate PASS + 结构自检（采信，见 Evidence）。

**Convergence**

N/A（首次 Review，无父 Cycle gap 可比较）

**Evidence**

- 独立读码：`src/storage/page_visibility.rs`（常量/守卫/doc/2 单测全文）、`src/storage/buffer_pool.rs:388-405`（`clear_all_visible` or_insert + doc）、`src/storage/mod.rs:38`（pub use）、`tests/isolation_level_test.rs:294-392`（新 e2e 全文，含 doc 注记）；Preserve 面六文件 diff 逐个 grep 可见性符号（零命中）；`src/database.rs:60-139`（恢复 advance_past 接线 + statement_snapshot，F2 机理确认）；`src/transaction/tx_id.rs`（advance_past 语义）；delta spec `mvcc-tombstone-visibility` S1-S4 与 design D1 逐点比对。
- 验证采信（公共规则 › 验证：Act 结论产生后工作区无任何写入——本 Review 会话只读，覆盖面文件与 Act Response Changed Files 记录一致；计数自洽 1056（Iter 000 全量）+ 5（page_visibility 2 + buffer_pool 2 + isolation_level 1）= 1061，isolation_level 7+1=8）：Act Response Verification Evidence 全表 PASS 采信——全量 `--no-fail-fast` 1061 passed / 0 failed / 2 ignored、clippy 0、fmt 0、validate changes 1 + specs 36 passed。
- Persisted Evidence Mode `none`：无 Evidence 目录要求，目录不存在不构成 finding。

**Follow-up Decision**

接受（accepted）。实现满足本 Iteration 全部既有 Acceptance（T5 S1-S4 逐场景见证 + T6 全量收口）；夹具校准属非阻塞偏差且断言语义未放宽；两项 Experience Candidates（RC 重启可见性回归 Issue 候选、reopen 两坑 Runbook 候选）与 ISS01 台账补记交 Recorder 按用户指令落账，不阻塞本 Iteration。无需当前 Cycle 修复。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Iteration Map 仅 000/001，T1-T6 全部完成；本 Iteration accepted 后 change 全部 Iteration 完成，可进入收尾流程——Maintainer 归档与 specs 合并、Recorder 落账，均待用户点名）
