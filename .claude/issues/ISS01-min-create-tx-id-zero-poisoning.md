# BufferPool::clear_all_visible 的 or_default() 以 min_create_tx_id=0 毒化 all-invisible 快路径

- Status: open
- Filed: 2026-09-14
- Source: `openspec/changes/2026-09-13-ms09-engine-mvcc-closeout/iterations/000-visibility/002-rework.md`（Act Response Experience Candidates 第 1 条 + Plan Review F1/F4(c)/F5）
- Environment: Linux x86_64（WSL2）、Rust/Cargo；revision HEAD `e51c4a3` + 工作区未提交的 MS16 实施与 MS09 change `2026-09-13-ms09-engine-mvcc-closeout` Iteration 000 三轮 Cycle 改动（002-rework 收尾工作区，2026-09-14 Recorder 只读核对）

## 缺陷描述

对已有代码的指控（预期 / 实际 / 位置）：

- **预期**：`PageVisibilityInfo.min_create_tx_id` 的语义是「页内所有 slot 的最小 create_tx_id」（`src/storage/page_visibility.rs:6-11` 头注释）；页级可见性摘要条目首次建立时，`min_create_tx_id` 应反映真实最小值，或保持「无信息 → 回落逐行检查」的中性语义，不得引入毒化值。
- **实际**：`src/storage/buffer_pool.rs::BufferPool::clear_all_visible`（:390-395）使用 `.entry(page_id).and_modify(...).or_default()`——当该页的 vis 条目不存在时，`or_default()` 建立 `PageVisibilityInfo::default()`，`min_create_tx_id = 0`（`PageVisibilityInfo` derive Default，u64 缺省 0，`src/storage/page_visibility.rs:12-16`）。INSERT 写路径（`src/executor/insert.rs:158-162`）先调 `clear_all_visible(page_id)` 再调 `update_visibility_on_insert(page_id, tx_id)`，而后者在条目已存在时走 `and_modify` 臂取 `min(0, W) = 0`（`src/storage/buffer_pool.rs::BufferPool::update_visibility_on_insert`，:399-410）——该页的 `min_create_tx_id` 被永久钉在 0。
- **后果**：`PageVisibilityInfo::all_invisible_for(t) = min_create_tx_id > t`（`src/storage/page_visibility.rs:21-23`）对毒化页恒为 false，`BufferPool::find_visible_version` 的 all-invisible 整页跳过快路径（`src/storage/buffer_pool.rs:313-315`）对「写路径首次建立条目」的页永久失效，回落逐 slot 可见性判定。唯一能建立 `min_create = W` 真实形态的路径是 DataScan 惰性置位先经 `set_all_visible` 的 `or_insert { min_create: u64::MAX }` 建条目（`src/storage/buffer_pool.rs::BufferPool::set_all_visible`，:378-386），随后写路径 `min(MAX, W) = W`——即只有「先扫描后写入」序列的页保留可用快路径。
- **范围说明**：三处页级快路径消费点曾以读者自身 id（`snapshot.tx_id()`）替代高水位的同域混淆已由 MS09 Iter000 002-rework 修复（消费点现传 `snapshot.high_water()`，`src/storage/buffer_pool.rs:313`、`src/executor/data_scan.rs:413`），不在本 issue 范围；本 issue 仅指控 `or_default()` 首建条目的毒化值。

机制分类：确认（Confirmed）——根因经 002-rework Plan Review F1 独立读码核实，并经 Recorder 2026-09-14 对工作区现状只读复认；非推断。

## 影响

- **当前**：无用户可见错误结果。毒化方向保守——快路径失效仅回落逐行判定，可见性语义正确（`src/storage/page_visibility.rs:25-29` 文档明示 default 是 safe default，fall through to per-row checks）。实际损失是性能优化失效：INSERT 密集且扫描前无 vis 条目的页永久失去整页跳过能力，属 MS08 实测域的全表扫描/点查性能面。
- **潜在**：`min_create_tx_id = 0` 使页级摘要信息失真。若未来引入依赖 `min_create` 正确性的机制（I038 GC 无键链回收域、基于页摘要的统计或回收决策），毒化值会造成误导；届时必须先修复本缺陷。

## 事件记录

None（未爆发——保守方向失效，未引发用户可见故障；无时间线可记）

## 处置

- scheduled → MS08 实测域（`.claude/docs/tasks.md` MS08「先量化再决定」纪律）。依据：002-rework Plan Review Follow-up Decision F4(c) 裁定「本 change 不修，属 MS08 实测域（先量化再优化）」，Act 按裁定保留 Issue 候选；用户 2026-09-14 指令落账。
- 修复方向候选（供届时裁定，未实施）：(a) `clear_all_visible` 的 `or_default()` 改为携带「无信息」哨兵（如 `min_create_tx_id: u64::MAX`，与 `set_all_visible` 的 or_insert 形态一致）；(b) 调整 insert 路径 clear/update 顺序或合并为单次条目更新。均需与 `check_page_all_visible`（`src/storage/buffer_pool.rs:421` 起，条件 2 已于 002-rework 对齐 `create > high_water`）的置位/清旗时序一并推演，并按 MS08 纪律先量化快路径失效的实际代价。
- 关联改进项：无既有 Ixx 编号；与 MS08-T07/T08/T09 同域（实测候选）。

## 证据

- `openspec/changes/2026-09-13-ms09-engine-mvcc-closeout/iterations/000-visibility/002-rework.md`
  - Act Response「Blocker Handoff」机理链 1-4（`or_default` 建条目 → `min(0, W)` 钉零 → `all_invisible_for` 恒 false → 快路径永不触发；「先扫描后写入」是 `min_create = W` 的唯一建立路径）
  - Act Response「Experience Candidates」第 1 条（候选本体）与「Verification Evidence」改形见证行（修复前 `5 passed; 2 failed`、两个 R2-S2 用例 observed `left: []`，改形后 `7 passed; 0 failed`——同时解释扫描 RED 与点查原处方通过两个实测结果）
  - Plan Review F1（三处消费点与机理链独立核实成立）、F4(c)/F5（裁定保留 Issue 候选、本 change 不修，证据齐全）
- 代码核对（Recorder 2026-09-14 只读复认，与 Act Response file:line 一致）：`src/storage/buffer_pool.rs:313-315`（快路径消费 `all_invisible_for(snapshot.high_water())`）、`:390-395`（`clear_all_visible` or_default）、`:399-410`（`update_visibility_on_insert` min 合并）、`:378-386`（`set_all_visible` or_insert u64::MAX）；`src/executor/insert.rs:158-162`（clear 先于 update）；`src/storage/page_visibility.rs:6-11`（min_create 语义文档）、`:21-23`（all_invisible_for 本体）、`:25-29`（default 安全语义）
