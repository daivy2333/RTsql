# Iteration 001 / Cycle 000-initial: 删除原语精确化与回滚 NULL-rekey 还原

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T6-a…c、T7-a…b（2026-09-29 用户指令扩围，来源 Iter000 Act Response Issue 候选 1/2，design D16/D17）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `update-index-maintenance` ADDED Requirement「回滚与冲突删除路径索引条目一致」（Iter000 已交付，含本 Cycle 新增的 NULL-rekey 场景）；Iter000 D1 的 `read_prev_index_keys` 派生机制与 D2 的在位守卫先例继续有效
- Excluded scope: 二级索引（002）、ALTER（003）、TRUNCATE（004）、VIEW（005）；多语句同事务同行链回滚处理序（pre-existing Minor，Iter000 已记录）；`upsert.rs`/`update.rs` 写侧 rekey 逻辑；`BTree::delete_exact` 重复键原语（002 的 T2-b）

**Objective**

`DELETE` 未命中键零副作用（search 未命中 → AffectedRows(0)、零索引/页/WAL 变动、无 KeyNotFound 误报、命中路径逐字节不变）；SET 唯一列为 NULL 的 UPDATE 回滚后旧唯一键条目还原（占用语义恢复、重开两态一致）；既有矩阵零回归。

**Background**

2026-09-29 Iter000 Plan Review accepted。Act Response 报告两个范围外实质缺陷候选（`IndexManager::delete` 歧义删除原语对缺失键误删「首个 ≥」邻近条目；SET 唯一列值→NULL 回滚后唯一条目缺失），用户指令「把这两个 issue 计划解决」并入本 change 为 T6/T7，按「正确性红线先于新功能」编排纪律插入为 Iteration 001，原 001–004 顺延为 002–005（change tasks.md 与 proposal 已同步）。

**Investigation Facts**

- Current Baseline: revision 0fe6971 + Iter000 未提交实施（工作区 5 文件：`manager.rs`/`upsert.rs`/`explicit_tx_test.rs`/`upsert_test.rs`/`project-model.md`）；全量 1464 passed / 0 failed / 2 ignored（Iter000 Act Response 分批新鲜结论，覆盖面未变化，直接采信）。`explicit_tx_test` 现有 Iter000 新增 5 rekey 用例；`update_index_maintenance_test` 9 用例。
- Current-State Evidence:
  - **T6 缺陷面**：`LeafNode::delete`（`src/storage/btree/node.rs:222-235`）以 `find_key_position(key)` 所得「首个 ≥」插入位直接 `delete_slot`，不验证命中位键相等——对照 `LeafNode::update`（:251-263）有 `existing_key != *key → KeyNotFound` 验证（同文件既有取键方式可复用）。唯一未设防调用点 `DeleteExecutor`（`src/executor/delete.rs`）：`:88 index_manager.search(&self.key)` 未命中（row_id `None`）时 `:135 index_manager.delete(&self.key)` 仍无条件执行，`:169` 恒返回 `AffectedRows(1)`；键大于全部键时 leaf `position >= key_count` → `KeyNotFound` 经 `IndexManager::delete`（`index_manager.rs:272-294`，`await??` 传播）上抛。CLI repro（Iter000 Act Response）：3 行表 id 1/2/3，`DELETE WHERE id = 0` → affected 1、扫描仍在、id=1 点查空集、`INSERT (1,…)` 失去 DuplicateKey；`DELETE WHERE id = 999` → KeyNotFound 误报。
  - **T6 其余调用点全部安全**（精确化后行为不变）：`IndexManager::delete` 的其他调用者删除的键在调用时必存在——`upsert.rs:531/535`（DO UPDATE 旧值键）、`:597`（可读 tuple PK 键）、`:606`（Iter000 守卫臂，`search(key)==Some(rid)` 前置）、`:611`（可读 tuple 唯一键）；`update.rs:307/315`（旧键）、`:343/347`（旧唯一键）；`manager.rs` A 趟各臂（`find_key_by_row_id` 反查键）。唯一列删除面对损坏态从「静默误删」变「显式报错」，属改进。
  - **T6 DELETE 计划面闭合**：`build_delete`（`ddl_dml.rs:1083-1105`）仅支持 PK 等值 WHERE（否则计划期拒绝），`DeleteExecutor` 是 DELETE 语句的全部执行面——修复面完整，无其他删除入口。
  - **T7 缺陷机制**：分桶按 **PK** 反查（`manager.rs:300-316`，`index_manager.find_key_by_row_id`）→ UPDATE 唯一列值→NULL 未动 PK、新版本落 A 趟；A 趟唯一臂以 `uindex.find_key_by_row_id(row_id)` 反查为门（Iter000 修订后 ：377-400）——`update.rs` Step 7 唯一 rekey（:343-347）已删旧条目且 NULL 不入索引 → 反查 None → 整臂跳过 → 旧值占用丢失（同值重插不再 DuplicateKey）。对照：PK→NULL rekey 因 PK 条目同删落 **B** 趟，`restore_tombstone_index_entries`（:401-478）已正确还原 PK 与唯一——缺陷恰为 A 趟唯一列维度的还原臂缺失。PK 不隐含 NOT NULL（`table_manager.rs:260` 仅声明面 not_null），但 PK→NULL 形态已由 B 趟覆盖，无需处理。
  - **T7 复用件**：`read_prev_index_keys`（manager.rs，Iter000 D1 新建）——单次页读 prev tuple + `wal::recovery::extract_index_keys` 派生，SlotNotFound/反序列化失败 → `Ok(None)` 回退、键位 NULL → `Ok(Some((None,…)))`；崩溃恢复两态断言模式先例 `tests/update_index_maintenance_test.rs:437`（`recovery_matches_runtime_after_rekey`：checkpoint 固化 → `wal_buffer.shutdown()` + drop 不 close → 重开）。
  - **恢复语义**：aborted 事务 WAL 记录永不重放（`recovery.rs:474-503` 分类、`:527-533` redo 只重放 committed）；两缺陷均为运行期缺陷，恢复面 rebuild 从数据页收敛天然正确——修复不得触碰恢复路径。
- Code and Critical Path: T6 变更面 `src/storage/btree/node.rs::LeafNode::delete` + `src/executor/delete.rs::DeleteExecutor::next`；T7 变更面 `src/transaction/manager.rs::abort_cleanup_versions` A 趟唯一索引臂。见证面 `tests/update_index_maintenance_test.rs`（删除路径索引维护域一致）与 `tests/explicit_tx_test.rs`（回滚矩阵域一致，helper `open_db`/`expect_ok`/`expect_affected`/`expect_rows` :14-42 既有）。

**Implementation Guidance**

T6 先 leaf 层后执行器层：① `LeafNode::delete` 在 `delete_slot(position)` 前验证命中位键与目标键相等（取键与 `update` :258-262 同型），不等或越界 → `KeyNotFound`；② `DeleteExecutor::next` 在 `:88 search` 后对 `row_id == None` 直接 `Ok(Some(AffectedRows(0)))` 早退（先于 unique_keys 提取/墓碑/PK 删除/WAL）。T7 在 A 趟唯一臂 `if let Some(key) = uindex.find_key_by_row_id(row_id).await` 的 None 侧增加对称分支：`prev_row_id` 存在时经 `read_prev_index_keys` 派生，对应唯一键 `Some` → `uindex.insert(派生键, prev)`（纯插入，无 delete-先于-insert 次序约束），`None` → 跳过。两任务无代码耦合可并行实施；测试先 RED。

**Task Contracts**

### T6: DELETE 未命中键零副作用

- Requirement/Scenario: `update-index-maintenance` ADDED「DELETE 未命中键零副作用」3 场景
- Depends on: None
- Targets: `src/storage/btree/node.rs::LeafNode::delete`；`src/executor/delete.rs::DeleteExecutor::next`
- Current behavior: search 未命中仍删键（「首个 ≥」邻近条目被误删：点查漏行、同值 INSERT 失去 DuplicateKey）且恒报 affected 1；键大于全部键报 `KeyNotFound`
- Required behavior: 未命中 → `AffectedRows(0)`、零索引/数据页/墓碑/WAL 变动、无报错（两形态：中间缺失键、大于全部键）；命中路径（含 SlotNotFound 容忍臂）逐字节不变；`IndexManager::delete` 对缺失键变为显式 `KeyNotFound`（修复后其余调用者不可达，损坏态从静默误删变显式报错）
- Preserve: `LeafNode::update`、`find_key_position` 零触碰；delete.rs 命中路径（唯一键提取/墓碑/PK 删除/record_version/WAL）逐字节不变；恢复路径零触碰
- Forbidden: 修改 `update.rs`/`upsert.rs`/`manager.rs`；修改 `BTree::delete_exact`（002 引入）；为删除原语新增第二套删除机制或调用方白名单
- Test witness: `tests/update_index_maintenance_test.rs` 新增 ≥3 用例——① 3 行表 DELETE 未命中中间键（id=0）：affected 0 + 三行点查均可达 + `INSERT (1,…)` 仍 DuplicateKey（RED：affected 1、id=1 点查空集、INSERT 成功）；② DELETE 大于全部键（id=999）：affected 0 不报错（RED：`KeyNotFound` 错误）；③ 命中路径与 SlotNotFound 容忍由既有用例零修改覆盖
- GREEN condition: 新用例绿 + `update_index_maintenance`/`upsert`/`mvcc_tombstone_visibility`/`keyless_row` 既有用例零修改通过
- Verification: `cargo test --test update_index_maintenance_test --test upsert_test --test mvcc_tombstone_visibility_test --test keyless_row_test -- --test-threads=1`（退出码 0、0 failed）+ 全量分批零回归
- Stop when: leaf 命中位取键不可达（结构性障碍）或精确化在全量中暴露 B-Tree 内部结构对「非精确删除」的隐式依赖——返回 Plan

### T7: SET 唯一列为 NULL 的回滚还原旧值占用

- Requirement/Scenario: `update-index-maintenance` ADDED「回滚与冲突删除路径索引条目一致」场景「SET 唯一列为 NULL 的回滚还原旧值占用」
- Depends on: None（与 T6 无代码耦合；依赖 Iter000 交付的 `read_prev_index_keys` 既有形态）
- Targets: `src/transaction/manager.rs::abort_cleanup_versions` A 趟唯一索引臂（反查 None 侧）
- Current behavior: 值→NULL 回滚后旧唯一键条目缺失，同值 INSERT 不再被 UNIQUE 拒绝（UNIQUE 静默失效）
- Required behavior: 回滚后旧唯一键条目还原指向 prev 版本——同值 INSERT 被 DuplicateKey 拒绝、`WHERE <唯一列> = <旧值>` 扫描可达 prev 行；干净重开与崩溃恢复两态一致；既有四分支（rekey 值→值、NULL→值、非 rekey 快路径、无前驱删除）行为逐字节不变
- Preserve: 分桶趟（PK 反查）判定不变；B 趟不变；PK 臂不变；`mark_aborted` 中性化与 AbortTxn WAL 时序不变；`update.rs` 写侧零触碰
- Forbidden: 修改 B 趟/恢复路径；扩展到多语句同行链（pre-existing Minor）；为还原新增第二派生源
- Test witness: `tests/explicit_tx_test.rs` 新增 ≥2 用例——① 值→NULL 回滚：`INSERT (2, 5)` 被 DuplicateKey 拒绝 + `WHERE code = 5` 扫描可达 prev 行（RED：INSERT 成功）；② 崩溃重开两态一致（`:437` 模式）；另 Iter000 `rollback_of_unique_set_from_null_releases_new_value`（NULL→值）零修改通过作反义对照
- GREEN condition: 新用例绿 + `explicit_tx`/`update_index_maintenance`/`mvcc_abort` 既有用例零修改通过
- Verification: `cargo test --test explicit_tx_test --test update_index_maintenance_test --test mvcc_abort_test -- --test-threads=1`（0 failed）+ 全量分批零回归
- Stop when: A 趟上下文无法安全读 prev tuple（结构性障碍）或还原分支与分桶语义冲突——返回 Plan

**Invariants**

- aborted 事务 WAL 记录不被重放、恢复终态=运行期落盘状态；修复不得引入恢复路径改动。
- DELETE 命中路径与 upsert/update/manager 既有索引删除调用点行为逐字节不变。
- 非目标形态（NULL→值、值→值 rekey、无前驱、B 趟各形态）行为与 Iter000 后现状逐字节一致。
- 不新增身份型证据工程、判定层或第二套删除/派生机制。

**Non-goals**

- 二级索引（002）与 `delete_exact` 重复键原语（002 T2-b）；多语句同行链处理序；DELETE 非 PK 等值形态（计划期拒绝现状）；Float PK 等无键行的点删语义（keyless 现状：search 永不命中 → 修复后 affected 0，与既有文档化语义一致）。

**Acceptance**

1. 「DELETE 未命中键零副作用」3 场景 + NULL-rekey 场景全部有可观察见证（RED→GREEN）。
2. RTM：

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| update-index-maintenance › DELETE 未命中键零副作用（3 场景） | T6 | update_index_maintenance_test 新增矩阵 | Covered |
| update-index-maintenance › 回滚与冲突删除路径索引条目一致（NULL-rekey 场景） | T7 | explicit_tx_test 新增 2 用例 | Covered |
| 既有矩阵零回归 | T6/T7 | 既有套件零修改 | Covered |

**Verification**

- T6 矩阵：`cargo test --test update_index_maintenance_test --test upsert_test --test mvcc_tombstone_visibility_test --test keyless_row_test -- --test-threads=1` → 0 failed。
- T7 矩阵：`cargo test --test explicit_tx_test --test update_index_maintenance_test --test mvcc_abort_test -- --test-threads=1` → 0 failed。
- 全量：按 M19 分批形态（确定性分组、批内 `--test-threads=1`、批间顺序）全部批次 0 failed（基线 1464 + 本轮新增）。
- 静态门：`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-29-ms21-ddl-evolution` 全部通过。

**Gate 2 Readiness**

Gate 2 全部 PASS（2026-09-29 用户批准扩围计划——「批准」；无 BLOCKED、无 WAIVED 项）。调查（两缺陷根因/修复面/见证可达性均有 file:line 代码证据）、设计（D16/D17 闭合无 TBD）、任务契约完备、验证最简直接（原生 cargo test 退出码判定）、Persisted Evidence 无必要、迭代平衡（单一正确性主题、独立验证与诊断边界）各项通过。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- `LeafNode::delete` 精确化后，直构夹具若存在「对缺失键 delete」的隐式依赖将在全量暴露（`mvcc_abort_test` 等以 `insert` 为主，预期无此形态）——如出现属夹具与产品语义对齐问题，Act 在 Response 记录偏差，不静默放宽断言。
- T7 的 prev tuple 读为异步页读（A 趟既有同型，无新增并发面）；新分支仅在「PK 反查 Some ∧ 唯一反查 None」可达，与既有四分支互斥，不影响快路径。
- keyless 行（Float PK/NULL PK）点删在修复后统一 affected 0——与「键位等值路由对无键行」既有语义一致，keyless_row 既有用例为零回归见证。
- 非实质选择留给 Act：用例命名与断言组织、leaf 取键的局部实现方式、T6 早退的具体位置形态。

## Act Response

- Status: reported
- Date: 2026-09-29

### 实际改动

**T6**（两处，契约两 Targets）：

- `src/storage/btree/node.rs::LeafNode::delete`：`delete_slot(position)` 前新增命中位键相等验证（`get_key(position)` 与目标键比较，与 `LeafNode::update` 既有验证同型）；不等或越界 → `StorageError::KeyNotFound`。歧义「插入位删除」原语对所有调用者消除——`IndexManager::delete` 对缺失键从静默误删「首个 ≥」邻近条目变为显式 `KeyNotFound`。
- `src/executor/delete.rs::DeleteExecutor::next`：`search` 之后、唯一键提取/墓碑写入/PK 条目删除/WAL 之前新增 `row_id.is_none()` 早退 `Ok(Some(ExecResult::AffectedRows(0)))`。命中路径（含 SlotNotFound 容忍臂、record_version、WAL Delete）逐字节不变。

**T7**（`src/transaction/manager.rs::abort_cleanup_versions` A 趟唯一索引臂）：`find_key_by_row_id` 反查 None 侧新增对称还原分支——`prev_row_id` 存在且 `derived_key` 为 `Some(Some(dk))` 时 `uindex.insert(dk, prev)` 纯插入还原旧唯一值占用（无 delete-先于-insert 次序约束）；键位 NULL（`Some(None)`）或派生不可得（`None`）跳过，无前驱跳过。既有四分支（Some 侧 match）逐字节不变；分桶趟、B 趟、PK 臂、`mark_aborted` 中性化、AbortTxn WAL 时序零改动；`read_prev_index_keys` 复用未改（单一派生源）。

**测试见证**（先 RED 后 GREEN，4 新用例）：

- `tests/update_index_maintenance_test.rs` +2：`delete_missing_middle_key_leaves_no_side_effects`（3 行表 DELETE id=0：affected 0 + 三行点查可达 + 同值 INSERT DuplicateKey + COUNT 3）、`delete_key_beyond_all_keys_reports_zero_without_error`（DELETE id=999：affected 0 不报错 + 既有行不扰动）。
- `tests/explicit_tx_test.rs` +2：`rollback_of_unique_set_to_null_restores_old_value_occupation`（值→NULL 回滚：同值 INSERT DuplicateKey + 原行经唯一列扫描可达）、`rollback_of_unique_set_to_null_consistent_across_crash_reopen`（`:437` 模式崩溃重开两态同型断言）。
- 契约 ③「命中路径与 SlotNotFound 容忍由既有用例零修改覆盖」按契约原文由既有矩阵承担（本文件 9 既有用例 + upsert_test 夹具用例），未新增第三测试函数。

### 与计划的偏差

None——三处实施均按 Implementation Guidance 原文落位（leaf 层先验证后删、执行器层 search 后早退、A 趟唯一臂 None 侧 `if let Some(Some(dk))` 纯插入）；测试用例形态与契约 witness 一一对应。fmt 对新增测试文件有一处排版归一（`cargo fmt` 全局跑批），无语义变化。

### 自检发现与遗留

- Gate 3 自检（Spec compliance + Code quality）：两任务 requirement/scenario 全覆盖、Preserve/Forbidden 零触碰（`update.rs`/`upsert.rs` 写侧、`delete.rs` 命中路径、恢复路径、B 趟、分桶趟均未改动）；无新增警告、死代码、判定层或第二套删除/派生机制；`delete.rs` 既有 `unique_keys` match 的 `None => Vec::new()` 臂在早退后不可达但按「命中路径逐字节不变」保留原形。
- 既有调用面核验：`IndexManager::delete` 全部调用点（delete.rs:140 唯一键派生自 tuple、upsert/update 各点键在调用时在位、manager.rs A 趟反查键、recovery.rs:817 有 `find_key_by_row_id` 前置守卫）删除的键在调用时必存在，精确化后行为不变；测试直构调用点（`index_manager_test`/`btree_merge_test`）均删已存在键。
- 遗留 Minor：无新增。多语句同行链回滚处理序（pre-existing，Iter000 已记录）维持 Non-goal；二级索引重复键 `delete_exact` 原语归 002（T2-b）。

### 验证

- RED：4 新用例全部在缺陷断言失败——`delete_missing_middle_key_leaves_no_side_effects`：`DELETE 未命中键必须 affected 0，实际 AffectedRows { count: 1 }`；`delete_key_beyond_all_keys_reports_zero_without_error`：`实际 Error { message: "Execution error: Key not found" }`；T7 两用例：`旧唯一值 5 必须仍被占用，got: AffectedRows { count: 1 }`（UNIQUE 静默失效实证）。
- GREEN（契约矩阵，fmt 后复跑）：`cargo test --test explicit_tx_test --test update_index_maintenance_test --test upsert_test --test mvcc_tombstone_visibility_test --test mvcc_abort_test --test keyless_row_test -- --test-threads=1` → 82 passed / 0 failed（21/11/32/11/3/4，实施后首跑与 fmt 后复跑同结果；既有用例零修改通过）。
- 全量（M19 分批形态：字典序分组、批内 `--test-threads=1`、批间顺序，批 0 lib+bins + 93 集成目标 × 10 批）：批 0 **327**/0、批 1 **98**/0、批 2 **210**/0/2i、批 3 **131**/0、批 4 **97**/0、批 5 **68**/0、批 6 **88**/0、批 7 **170**/0、批 8 **139**/0、批 9 **115**/0、批 10 **25**/0——**合计 1468 passed / 0 failed / 2 ignored**（= Iter000 基线 1464 + 本轮新增 4；全部批次 0 failed）。
- 静态门：`cargo clippy --all-targets -- -D warnings` 0 errors；`cargo fmt --check` 通过（首跑对 update_index_maintenance_test 一处排版差异，`cargo fmt` 修正后复检通过并复跑契约矩阵）；`openspec validate 2026-09-29-ms21-ddl-evolution` → valid。

### Persisted Evidence

- None required（Plan 设 none；全部验证可由上述命令低成本重放）。

### Experience Candidates

- Runbook 候选：None（M19 分批全量已是 project-model 现行约束，非新操作路径）。
- Issue 候选：None（本轮修复的两个缺陷即 Iter000 扩围 T6/T7 来源，按 tasks.md 同步注记不再重复落账；实施中未发现新的范围外实质缺陷）。

### 未解决问题

- None（多语句同行链处理序维持 pre-existing Minor 记录，归后续裁定）。

## Plan Review

- Review Result: accepted
- Date: 2026-09-29

**Findings**

非阻塞 Minor findings（2）：

1. `delete.rs` 既有 `unique_keys` match 的 `None => Vec::new()` 臂在早退后不可达——Act 按「命中路径逐字节不变」保留原形，属最小 diff 纪律的正确取舍，不要求清理。
2. 契约 T6 witness ③ 未新增第三测试函数，按契约原文由既有矩阵（update_index_maintenance 9 既有 + upsert_test 夹具 + keyless_row）承担——与契约「由既有用例零修改覆盖」原文一致，非偏差。

**Acceptance Gaps**

None——「DELETE 未命中键零副作用」3 场景 + NULL-rekey 场景全部有 RED→GREEN 可观察见证，RTM Covered 链路与代码一致。

**Evidence**

- 独立 diff 审读（Plan Review 双检查之二）：三处产品改动与契约逐项一致——`node.rs::LeafNode::delete` 在 `delete_slot` 前以 `get_key(position)` 验证命中位键相等（与 `update` :258-262 同型，不等/越界 → KeyNotFound）；`delete.rs` 在 `search` 后 `row_id.is_none()` 早退 `AffectedRows(0)`，位于唯一键提取/墓碑/PK 删除/WAL 之前；`manager.rs` A 趟唯一臂 `else if let Some(prev)` + `if let Some(Some(dk)) = derived_key` 纯插入还原，与既有 Some 侧四分支互斥，`None`（派生不可得）与 `Some(None)`（prev 键位 NULL）均跳过。分桶趟、B 趟、PK 臂、`mark_aborted`、AbortTxn WAL 时序、`update.rs`/`upsert.rs` 写侧零改动（git diff 全量核对，本轮净增面 = 3 产品文件 + 2 测试文件 + Act Response）。
- `IndexManager::delete` 调用面复核：recovery 的 Delete redo 臂（`recovery.rs:817` 附近）有 `find_key_by_row_id` 前置守卫——正常重放路径键在树中，精确化后行为不变；损坏态从静默误删变显式 `RedoFailed`，属改进。Act 的调用面枚举与代码核对一致。
- 测试见证：4 新用例断言与 6 场景逐一对应（含 COUNT 3 与崩溃重开两态同型断言）；RED 记录的三个缺陷形态（affected 1 / KeyNotFound 误报 / UNIQUE 静默失效）与本 Review Iter000 阶段确认的根因一致。
- 验证采信：GREEN 82/0（21/11/32/11/3/4，用例计数与各文件基线吻合）、全量分批 1468/0/2i（= 基线 1464 + 4；批 3 +2、批 9 +2 与新增用例归属一致）、clippy/fmt/validate 均为本会话 Act 新鲜运行，其后工作区未变化（只读 git 检查），按覆盖未变化采信（双检查之一由 Act GREEN 承担）。
- 身份型证据检查：无新增身份型证据工程或判定层。

**Follow-up Decision**

接受：既有 Acceptance 全部满足，实施与 Implementation Guidance 原文逐点一致，无偏差、无范围外新发现。按 2026-09-29 批准的扩围 Map 展开下一 Iteration 002（CREATE/DROP INDEX 二级索引端到端，T2-a…g）。

**Next Cycle**

None

**Next Iteration**

`iterations/002-secondary-indexes/000-initial.md`（Status: draft——Gate 2 待用户批准后转 ready 交 Act）
