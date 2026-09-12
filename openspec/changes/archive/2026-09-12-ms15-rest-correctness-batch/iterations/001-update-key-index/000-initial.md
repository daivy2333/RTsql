# Iteration 001 / Cycle 000-initial: UPDATE 键位无键值索引条目清理（I037）

## Plan Context

- Status: ready
- Iteration: 001-update-key-index
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T4, T5, T6
- Depends on: Iteration 000（accepted 2026-09-12，见 `../000-initial/000-initial.md` Plan Review；共享工作区与全量基线，无代码耦合）
- Stable baseline: 键位置 NULL 后旧键 INSERT 成功、旧键点查空集、崩溃恢复两态一致；非键列 SET 与键列原值 SET 行为不变；全量回归零修改
- Verification boundary: T5/T6 全绿 + clippy/fmt 0 + `openspec validate --changes` PASS
- Diagnostic boundary: `src/executor/update.rs`（Step 7）+ `tests/update_index_maintenance_test.rs`
- Deferred tasks: T7-T9（Iteration 002）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change proposal 全部范围约束（含默认假设 2：rekey 可键控形态排除）；design D2 判定条件
- Excluded scope: I039（Iteration 002）、rekey 键列 SET 为另一可键控 Int 形态、WAL/恢复语义修改、插入侧去重逻辑、I046/I036 域

**Objective**

`UPDATE t SET <键列> = NULL`（新值 `to_key()==None`）后：旧键索引条目被删除——旧键 INSERT 不再 DuplicateKey 误拒、旧键点查空集、崩溃恢复后两态一致；非键列 SET 与键列原值 SET 的既有行为逐字节保持；既有套件（`keyless_row_test` 等）零修改通过。

**Background**

tasks MS15-T03 + improvements I037：MS10-T05 001-rework Plan Review Finding 5 代码核实（2026-09-09）。`UpdateExecutor` Step 7 对新值不问可键控性无条件 `index_manager.update(&self.key, new_row_id)`，键位置 NULL 后旧键条目指向键位已无键的新版本——INSERT 旧键被误拒、点查返回无键行、恢复重建后自愈（两态不一致）。恢复侧语义（无键版本不入索引，MS10-T05 keyless 桶 + MS10-T02 R7/R8 重建）已正确，本 Iteration 使运行期对齐。

**Investigation Facts**

- Current Baseline: Iteration 000 最终 Act Response：全量 856 passed / 0 failed / 2 ignored（Plan Review 独立复跑采信，基线 853 + 3）、clippy/fmt 0、validate PASS；Review Result `accepted`（2026-09-12）。工作区含 MS15-T01 与 MS15-Rest Iter000 未提交产物——Act 开始前 `git status`/`git diff` 基线检查；本 Iteration 代码面 `src/executor/update.rs` 自规划调查后未被触碰（git status 复核 2026-09-12，覆盖范围未变化）。
- Current-State Evidence:
  - `UpdateExecutor`（`src/executor/update.rs` 全文 137 行）：字段含 `key: Vec<u8>`（旧键，来自 WHERE `pk = <Int 字面量>` 经 `extract_pk_from_where`）、`column_name: String`（SET 目标列，`assignment.id[0].value.to_lowercase()`，`ddl_dml.rs:383`——已去引号）、`new_value: Value`（`Expr::Value` 或 NULL 标识符，`ddl_dml.rs:386-395`）、`table_meta: Arc<TableMeta>`。
  - Step 1（`:70-73`）`index_manager.search(&self.key)` 定位旧行（不存在 → `KeyNotFound`）；Step 6（`:101-103`）写新版本；Step 7（`:129-133`）无条件 `index_manager.update(&self.key, new_row_id)`——**本 Iteration 唯一修改点**。
  - `TableMeta.pk_column: String`（`src/storage/data/table_manager.rs:53`）——SET 列是否键列的判定依据；`Value::to_key()`（`src/executor/value.rs:82-90`）仅 `Int` → `Some`，String/Null/Float/Bool → `None`（可键控性权威定义，与插入侧 `insert.rs:100`、MS15-T01 同源）。
  - `IndexManager::delete(&self, key: &[u8])`（`src/storage/btree/index_manager.rs:245-266`）：search 移除 `row_to_key` 映射 + `BTree::delete`（spawn_blocking）+ root 回写 + catalog 同步——删除存在的键安全（Step 1 已证存在）。
  - 插入侧误拒机制：`InsertExecutor`（`src/executor/insert.rs:100-110`）键位 `to_key()` Some 时 `search` 命中即 `DuplicateKey`——残留条目是误拒根源。
  - 恢复侧对齐：Update WAL 记录只含 old_tuple/new_tuple（`update.rs:113-122`，索引操作不入 WAL）；重放后索引从最终数据页重建（MS10-T02 R7/R8），键位 NULL 行不入重建索引——运行期 delete 与重建结果由构造对齐。
  - 既有测试入口：`tests/keyless_row_test.rs`（T8-R2 无键行 UPDATE 链 + 崩溃重开，`:140` 起——`UPDATE t SET a = NULL WHERE a = 5` 既有见证，修复后运行期索引状态与该套件恢复面断言一致）；lib 集成测试用 `Database::open`/`execute_sql` 模式。
- Code and Critical Path: `build_update`（`ddl_dml.rs:359-407`，键/列/新值来源）→ `UpdateExecutor::next`（update.rs，Step 1-7 顺序：search → 读旧元组 → 改列 → 序列化 → 新版本头 → 写页 → visibility 清除 → WAL → record_version → 索引维护）→ 失败路径经 DML auto-commit 包裹回滚（MS06-T01）。

**Implementation Guidance**

实现顺序：T4 先写 RED（五用例：INSERT 误拒 / 点查残留 / 恢复两态 / 非键列保持 / 原值保持——前二与恢复两态 RED，后二基线 GREEN），确认 RED 形态后 T5 改 Step 7，T6 回归收尾。T5 形态建议（非实质细节可就地调整）：

```rust
// Step 7:
if self.column_name == self.table_meta.pk_column && self.new_value.to_key().is_none() {
    self.table_meta.index_manager.delete(&self.key).await?;
} else {
    self.table_meta.index_manager.update(&self.key, new_row_id).await?;
}
```

注意 `to_key()` 返回 `Option<Key>`（Key 为 32 字节定长），不引入新转换源；判定只用既有字段，无 schema/格式变更。

**Behavioral Change**

- 当前：`UPDATE SET <键列> = NULL WHERE <键列> = 5` 成功后——`INSERT (5,...)` 被 `DuplicateKey` 误拒；`SELECT * FROM t WHERE id = 5` 经残留索引条目返回 `(NULL, 100)`；崩溃重开后自愈（条目消失）。
- 目标：同 UPDATE 后——INSERT 成功；点查空集；崩溃重开后行为一致（两态一致）。
- 接口/状态：`UpdateExecutor` Step 7 分支化，无公共 API、plan 形状、WAL 记录、错误文案变化；索引内容变化即缺陷修复本体。
- 错误语义：无新增错误路径；delete 失败经既有传播与回滚路径（Step 6 版本写入被 auto-commit 包裹回滚，无半态）。

**Task Contracts**

### T4: RED 测试见证——R2 缺陷形态与保持面锚点

- Requirement/Scenario: R2（delta spec `specs/update-index-maintenance/spec.md`）S「键位置 NULL 后旧键 INSERT 不再误拒」、S「键位置 NULL 后旧键点查返回空集」、S「键位置 NULL 后崩溃恢复两态一致」、S「非键列更新不影响索引条目」、R3 S「键列原值更新条目保持」
- Depends on: None
- Targets: 新增 `tests/update_index_maintenance_test.rs`（`Database::open`/`execute_sql` + tempfile 模式，参照 `keyless_row_test.rs`）
- Current behavior: UPDATE SET id = NULL 后 INSERT (5,…) Err DuplicateKey；点查 `WHERE id = 5` 返回 `(NULL, 100)`；崩溃重开后 INSERT 成功/点查空（两态不一致）；非键列 SET 与原值 SET 行为正确
- Required behavior: 五用例断言目标行为；修复前 INSERT 误拒与点查残留 RED、恢复两态 RED（运行期行为与恢复面不一致）、保持面两用例 GREEN
- Required changes: 仅新增测试文件；不修改产品代码
- Preserve: `keyless_row_test.rs` 既有用例零修改
- Forbidden: 修改 `src/`；修改既有测试
- Test witness: `cargo test --test update_index_maintenance_test`，RED 形态与契约预期逐字对应
- GREEN condition: T5 后全部转 GREEN
- Verification: `cargo test --test update_index_maintenance_test`，退出码 0
- Stop when: RED 形态与契约预期不符（如 INSERT 未被误拒或点查未返回残留行——基线与 I037 记录矛盾，返回 Plan）

### T5: R2 实现——Step 7 条件删除（design D2）

- Requirement/Scenario: R2 全部场景
- Depends on: T4
- Targets: `src/executor/update.rs::UpdateExecutor::next` Step 7（`:129-133`）
- Current behavior: 无条件 `index_manager.update(&self.key, new_row_id)`
- Required behavior: `column_name == table_meta.pk_column && new_value.to_key().is_none()` → `index_manager.delete(&self.key)`；否则既有 `update` 保持
- Required changes: 仅 Step 7 分支化；无其他行改动
- Preserve: Step 1-6 全部语义（search/版本链/WAL/visibility/record_version）；非键列 SET 与键列可键控 SET（含原值与 rekey 形态）行为不变；`KeyNotFound`、多列 SET 拒绝、非 PK WHERE 拒绝等错误语义不变
- Forbidden: 修改 WAL 记录、恢复重放、插入侧去重、`IndexManager` API、`extract_pk_from_where`
- Test witness: T4 五用例转 GREEN；`tests/keyless_row_test.rs` 4 用例零修改通过
- GREEN condition: `cargo test --test update_index_maintenance_test --test keyless_row_test` 全绿
- Verification: 同上
- Stop when: 判定条件需引入新元数据（pk_column 不可用）或 delete 语义与恢复重建出现新的不一致（实质，返回 Plan）

### T6: R2 回归收尾

- Requirement/Scenario: R3 既有语义零回归全场景
- Depends on: T5
- Targets: 全量验证命令（无代码修改）
- Current behavior: —
- Required behavior: 全量 856+新增 全绿；clippy/fmt 0；validate PASS
- Required changes: 无代码修改
- Preserve: —
- Forbidden: 为凑绿修改既有测试
- Test witness: `cargo test`（全量）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate --changes`
- GREEN condition: 全部退出码 0；基线 856 只增不减
- Verification: 同上
- Stop when: 全量出现无法归因于本变更面的失败（返回 Plan；已知 I041 偶发 env 竞态除外——重跑即绿按 I041 记录处置并注明）

**Invariants**

- 可键控性权威定义单一来源 `Value::to_key()`；索引操作不入 WAL、恢复重建语义不变（MS10-T02 R7/R8、MS10-T05 keyless 桶）；MVCC 版本链与事务包裹语义不变；无键行存储语义不变（落库不入索引）。

**Non-goals**

- rekey 键列 SET 为另一可键控 Int（默认假设 2，独立 I 项候选）；I039；`SET 键列 = 'x'` 类型校验时序（非实质未知项，实测行为记入 Act Response——若 serialize 先拒绝则 UPDATE 失败无索引面变化，若接受则 delete 与恢复重建一致）；GC/I038；性能。

**Acceptance**

R2 delta spec 场景全部满足且既有套件零回归。RTM（Iteration 001 范围）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 | INSERT 误拒消除 | D2 | T4/T5 | 001 | `update.rs` Step 7 | `update_index_maintenance_test` 新增 | None | Covered |
| R1 | 旧键点查空集 | D2 | T4/T5 | 001 | 同上 | 同上 | None | Covered |
| R1 | 崩溃恢复两态一致 | D2 | T4/T5 | 001 | 同上 + 既有恢复路径（不改） | 同上（drop 不 close 重开模式） | None | Covered |
| R1 | 非键列更新条目保持 | D2 | T4/T5 | 001 | 同上（else 臂） | 同上 | None | Covered |
| R2 | 键列原值更新条目保持 | D2 | T4/T5 | 001 | 同上（else 臂） | 同上 | None | Covered |
| R2 | 既有 UPDATE 零回归 | D2 | T5/T6 | 001 | Step 1-6（不改） | `keyless_row_test` 4 用例 + 全量 | None | Covered |

**Verification**

- `cargo test`（全量，基线 856 只增不减）、`cargo clippy --all-targets -- -D warnings`（0）、`cargo fmt --check`（0）、`openspec validate --changes`（PASS）。
- 缺陷形态 lib 探针（决定性输出记入 Act Response，≤20 行）：UPDATE SET id = NULL 后 `INSERT` 返回 `AffectedRows(1)`（修复前 DuplicateKey）、`SELECT * FROM t WHERE id = 5` 空集（修复前返回无键行）。
- Persisted Evidence 为 none：验证命令与决定性输出写入 Act Response 即可。

**Gate 2 Readiness**

- 无 Missing requirement：PASS（RTM 全 Covered，delta spec 6 场景均有 task/代码/测试映射）
- Simplified requirement 已批准：PASS（无 Simplified 项）
- 调查完整：PASS（update.rs 全文 137 行行级实证 + pk_column/to_key/delete/恢复对齐链 2026-09-12 行级证据；`src/executor/update.rs` 覆盖范围自调查后未变化——git status 复核）
- 设计闭合：PASS（判定条件、恢复一致性论证、失败路径分析见 design D2 与本 Context）
- 任务可执行：PASS（T4-T6 均有位置/行为/见证/停止条件）
- 分轮合理：PASS（平衡审计见 tasks.md；单域单文件面 + 单测试文件）
- 追踪完整：PASS（RTM 链路闭合）
- 验证充分：PASS（RED→GREEN + keyless_row 零修改 + 全量收尾）
- 无身份型证据工程：PASS（验证直接观察行集、错误变体与测试退出码）
- 无实质未知项/TBD：PASS（serialize 类型校验时序为非实质未知项，已入 Non-goals/Risks 并约定 Act 记录实测）
- OpenSpec 产物一致：PASS（proposal/design/tasks/delta spec/cycle 交叉一致）
- Persisted Evidence 模式明确：PASS（none，无 required 项）
- 用户批准计划：PASS（2026-09-12 用户"批准"覆盖本 change Iteration Map 与 T4-T6 范围（Gate 1/2 change 级）；本 Cycle 契约为该范围内实现细节的展开，无范围/验收变化——先例：MS10-T02 四 Iteration 均按 change 级批准推进，仅 rework 扩面另行取得 Gate 2 批准）

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无满足 Evidence 白名单的不可重跑项。Budget: 不适用。

**Risks and Notes**

- `SET 键列 = 'x'`（String 入 Int 键列）的 serialize 类型校验时序未验证——delete 条件覆盖该形态（`to_key()==None`），两种时序下索引状态均与恢复重建一致（无键行不入索引），非阻塞；Act 实测并在 Response 记录。
- 全量已知偶发：I041（resolve env 竞态，约 1/6 全量、重跑即绿）——失败时先重跑一次再判定，勿误报回归。
- 工作区未提交产物累积（MS15-T01 + Iter000 + 本 Iteration）——建议用户在本 Iteration 收尾后一并 commit（历次先例每 change 一 commit，本 change 三 Iteration 共享工作区，最终一次 commit 即可）。

## Act Response

- Status: blocked

**Implemented**

- T4（RED 见证，completed）：新增 `tests/update_index_maintenance_test.rs` 五用例——R1-S1 旧键 INSERT 误拒消除、R1-S2 旧键点查空集、R1-S3 崩溃恢复两态一致（含崩溃前运行期点查作为两态比较面）、R1-S4 非键列 SET 条目保持、R2-S1 键列原值 SET 条目保持，全部与 delta spec 场景 THEN 逐字对应。实测 RED（实现前）：`3 failed; 2 passed`——INSERT 误拒 `Execution error: Duplicate key`、点查返回残留行（rows.len()=1 ≠ 0）、两态运行期面（rows.len()=1 ≠ 0）——与 T4 契约预期形态逐字一致；保持面两用例基线 GREEN。实现前 `keyless_row_test` 基线独立观察 `4 passed`。
- T5（design D2，实施完成）：`UpdateExecutor::next` Step 7 分支化——`self.column_name == self.table_meta.pk_column && self.new_value.to_key().is_none()` → `index_manager.delete(&self.key)`；否则既有 `index_manager.update(&self.key, new_row_id)` 保持。新套件转 GREEN（`5 passed; 0 failed`）。
- T5 契约要求的 Risks 实测（`SET 键列 = 'x'` serialize 时序）：`UPDATE t SET id = 'x' WHERE id = 5` 返回 affected 1——serialize_tuple 不拒绝 String 入 Int 列，delete 分支执行；其后 `WHERE id = 5` 点查空集、`SELECT COUNT(*)` = 1（行落库不入索引）。索引状态与恢复重建一致（Risks 预判的"接受则一致"分支），非阻塞未知项消除。

**Changed Files and Symbols**

- `src/executor/update.rs`：`UpdateExecutor::next` Step 7（原 `:129-133` 无条件 `update`）改为条件 delete/update 分支 + 4 行注释；Step 1-6 与文件其余部分零修改。
- `tests/update_index_maintenance_test.rs`：新增测试文件（5 用例 + 模块级文档）。

**Deviations from Plan**

1. 非实质：新测试文件初版经 `cargo fmt` 一次格式校准（行宽），断言与契约面零变化；按最终状态复跑目标套件，结论保持。

**Blocker Handoff**

- 命中位置：T5 GREEN condition / T6 全量回归（Gate 5 保存面）；Gate 6 条目「实际代码与契约存在实质冲突」。
- Plan 预期 vs 实际：Iteration 001 Investigation Facts 断言「修复后运行期索引状态与该套件恢复面断言一致」，T5 契约要求 `tests/keyless_row_test.rs` 4 用例零修改通过。实际：`keyless_row_update_recovery_after_crash` 在 `tests/keyless_row_test.rs:171` 失败——`UPDATE t SET v = 42 WHERE a = 5` 返回 `Execution error: Key not found`。
- 机制（行级）：T8-R2 流程 = `UPDATE t SET a = NULL WHERE a = 5`（修复后按 D2 删除键 5 条目）→ `UPDATE t SET v = 42 WHERE a = 5`（`build_update` 从 WHERE 字面量提取 key=5，Step 1 `index_manager.search(5)` → None → `KeyNotFound`）。第二次 UPDATE 依赖的恰是 I037 缺陷的残留条目可达性（T8-R2 注释自证「索引 key5 → v2」）；失败点在 Step 1 search，任何 Step 7 形态都无法回避——R1 验收（S1/S2/S3 的 delete 语义）与 T5 保存面（T8-R2 零修改通过）互斥。
- 冲突层级：delta spec `specs/update-index-maintenance/spec.md` R2 场景「既有 UPDATE 行为零回归」原文要求「全部通过且除本 change 新增见证外零修改」——与同 spec R1 的 delete 语义矛盾。Act 无权修改既有测试与 delta spec，返回 Plan。
- 实证半径（`cargo test --no-fail-fast` 全量）：`860 passed / 1 failed / 2 ignored`——唯一失败 = T8-R2；其余全部 target（含 keyless_row_test 其余 3 用例、MS15-T01 套件、全部既有套件）通过。
- 结构性影响（Plan 决策所需）：修复后无键行不入索引、`build_update` 要求 WHERE PK 等值且仅 Int 字面量可键控（MS15-T01）→ 运行期 SQL 无法再产生 old_tuple 无键的 Update WAL 记录；T8-R2 是该形态的唯一运行期生产者，其恢复面断言（v=42 可见、COUNT=3）随之级联失效。`wal-recovery-replay-integrity` spec 场景「无键行 Update 崩溃恢复语义正确」（MS10-T05 登记，不在本 change delta 面内）失去运行期见证路径；重放代码路径本身未动（keyless 桶仍在，旧 WAL 文件兼容），语义未变。
- 任务状态：T4 completed（RED→GREEN）；T5 实施完成、新套件 GREEN，GREEN condition 被 T8-R2 阻塞；T6 未开始（全量回归按构造必含 T8-R2 失败）。
- 工作区状态：上述两个文件改动保留（Gate 6 不回滚既有工作）；clippy/fmt/validate 干净（见 Verification Evidence）。
- 恢复条件（供 Plan 与用户裁量）：a) 修订 T8-R2 流程/断言适配修复后语义（`KeyNotFound` 是键位无键行对旧键等值 WHERE 的正确结果），并重述 keyless old_tuple Update 恢复覆盖的见证方式（合成 WAL 或接受该形态 SQL 不可达并收窄覆盖）——涉及 delta spec R2 零修改措辞与 MS10-T05 既有 spec 场景，需 Plan 与用户批准；b) 或重新设计 I037 修复形态以维持残留可达性（与 R1 验收矛盾，需推翻 D2）。二者均超出当前 Cycle 执行契约。

**Blocker Resolution**

None

**Self-Review**

Spec compliance review（逐任务）：

- T4 契约全项满足：五用例与 R1-S1..S4 / R2-S1 场景一一映射；仅新增测试文件、未修改产品代码与既有测试；RED 形态与契约预期逐字对应（INSERT 误拒 / 点查残留 / 两态运行期面 RED，保持面两侧 GREEN）。
- T5 实现与 D2 判定条件逐字同构（仅用既有字段 `column_name`/`pk_column`/`new_value`，无新元数据）；Step 1-6 零触碰；Preserve 面（非键列 SET、键列可键控 SET、`KeyNotFound` 等错误语义）经新用例 S4/S1 保持锚点与全量 860 passed 佐证；禁止项（WAL 记录、恢复重放、插入侧去重、`IndexManager` API、`extract_pk_from_where`）零触碰。
- GREEN condition 无法满足 → 按 Gate 6 阻塞交接，不以修改既有测试等方式伪装解决。

Code quality review（完整 diff）：

- diff 无计划外修改：`git status` 新增面 = 本 change 两文件（测试文件 + update.rs Step 7）；工作区其余为基线既有未提交产物（MS15-T01 + Iteration 000），本 Cycle 未触碰。
- 判定条件复用可键控性权威定义 `Value::to_key()` 与 `TableMeta.pk_column`，无重复实现、无死代码、无新增警告（clippy `-D warnings` exit 0）。
- 测试以正确原因通过：RED 观察到确切缺陷形态后转 GREEN；保持面锚点在变更前后双侧 GREEN；无身份型证据机制。
- 发现问题：无 Critical / 无 Important 属 Act 可修复面——唯一 Important 即 Blocker 本体（契约冲突，返回 Plan）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T4 RED 见证 | `cargo test --test update_index_maintenance_test`（实现前） | `2 passed; 3 failed`——`Duplicate key` 误拒 / 点查残留 1 行 / 运行期两态面 1 行 | I037 两缺陷形态 + 两态不一致 | PASS（预期 RED） |
| 保持面基线 | `cargo test --test keyless_row_test`（实现前） | `4 passed; 0 failed` | keyless_row 套件变更前 GREEN | PASS |
| T5 新套件 GREEN | `cargo test --test update_index_maintenance_test` | `5 passed; 0 failed` | R1/R2 全场景 | PASS |
| 保存面 | `cargo test --test keyless_row_test` | `3 passed; 1 failed`——T8-R2 `Key not found`（`keyless_row_test.rs:171`） | T8-R2 | FAIL（Blocker 本体） |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | exit 0 | 全目标 | PASS |
| 格式 | `cargo fmt --check` | 0 diff | 全仓库 | PASS |
| OpenSpec | `openspec validate --changes` | `1 passed, 0 failed` | change 产物 | PASS |
| 全量半径 | `cargo test --no-fail-fast` | `TOTAL passed: 860 failed: 1 ignored: 2`；唯一失败 target = keyless_row_test（T8-R2） | 全仓库 | FAIL（仅 T8-R2） |
| Risks 探针 | `cargo build` + `rtsql` CLI：建表/INSERT (5,100)/`UPDATE t SET id = 'x' WHERE id = 5`/点查/COUNT | affected 1 → 点查 `rows:[]` → COUNT 1 | serialize 时序 + delete 分支一致性 | PASS |

**Persisted Evidence**

None required（Blocker 偏差可低成本复现：目标套件两条 cargo test 命令即可重现，不满足 Evidence 白名单）

**Experience Candidates**

None

**Remaining Issues**

1. Blocker 本体（见 Blocker Handoff）：R1 delete 语义与 T8-R2 零修改保存面互斥，及 keyless old_tuple Update WAL 运行期生产路径消失后的恢复覆盖见证问题——返回 Plan 与用户裁量。
2. Minor：T8-R2 注释「索引 key5 → v2」描述的残留条目状态即 I037 缺陷形态本身——若 Plan 选择修订该测试，注释需一并按新语义改写。

**Commit or Diff Reference**

未提交（工作区）。本 Cycle diff = `src/executor/update.rs`（Step 7 分支化）+ `tests/update_index_maintenance_test.rs`（新增）；工作区同时含基线既有未提交产物（MS15-T01 实施 + docs sync + Iteration 000 实施），本 Cycle 未触碰。

## Plan Review

- Review Result: replan-required

**Findings**

基于实际代码、diff 与独立复现的检查（Act Self-Review 未代替独立检查）：

- **F1（PLAN-OMISSION，阻塞本 Cycle Acceptance，根因）**：Plan 调查遗漏——Investigation Facts 断言「修复后运行期索引状态与该套件恢复面断言一致」、T5 契约要求「keyless_row_test 4 用例零修改」，未追查 T8-R2（`keyless_row_update_recovery_after_crash`，`tests/keyless_row_test.rs:127-215`）的链路前提：其第二次 UPDATE（`:169` `SET v = 42 WHERE a = 5`）依赖的恰是 I037 缺陷的残留索引条目（注释 `:163`「索引 key5 → v2」自证）——修复删除该条目后 Step 1 `search` → `KeyNotFound`，失败点先于 Step 7，**R1 delete 语义与 T8-R2 零修改保存面互斥**（Act 机制分析经 Plan 独立复现确认：`cargo test --test keyless_row_test` → 3 passed / 1 failed，失败名一致）。连带：Iteration 验收边界「全量回归零修改」与 delta spec R2「既有测试套件 SHALL 零修改通过」按原文均不可满足——验收边界需要改变。
- **F2（非阻塞，已消除）**：Risks 未知项实测收口——`UPDATE SET id = 'x'`（String 入 Int 键列）serialize_tuple 不拒绝（tag 序列化，预存引擎特性），delete 分支执行且索引状态与恢复重建一致（Act 探针：affected 1 → 点查空 → COUNT 1）。
- **F3（非阻塞，结构性影响定性）**：修复后运行期 SQL 无法再产生 old_tuple 无键的 Update WAL 记录——T8-R2 是该形态唯一运行期生产者，主 spec `wal-recovery-replay-integrity`「无键行 Update 崩溃恢复语义正确」的运行期见证路径收窄（重放代码未动、语义未变，keyless 桶转为 legacy WAL 兼容面）；keyless NEW_tuple 重放路径仍被 T4 恢复用例与校准后 T8-R2 见证。处置：design D2 已补结构性后果注；合成 WAL 见证基建排除出本 change，作 I 项候选归 docs-maintainer 收尾登记。
- **F4（非阻塞）**：T8-R2 文档注释（`:127-138`、`:163`）描述残留条目状态即缺陷本体——校准时一并按新语义改写（Act Remaining #2 并入校准契约）。
- **实现核对（与 Blocker 无关的部分全部有效）**：update.rs Step 7 diff 与 D2 判定条件逐字同构（条件 + delete/update 分支 + 注释，Step 1-6 零触碰）；新测试文件 5 用例与 R1 场景一一映射、RED 形态与契约逐字对应（INSERT DuplicateKey 误拒 / 点查残留 / 两态面 RED，保持面 GREEN）；clippy/fmt/validate 干净（Plan 独立复跑确认）。
- **Act 过程评价**：Gate 6 阻塞处置正确——未以修改既有测试等方式伪装 GREEN，Blocker Handoff 机制分析行级完整、恢复条件清晰，工作区状态保留合规。

**Deviation Classification**

- PLAN-OMISSION ×1（F1 根因：T8-R2 前提依赖未追查，阻塞）
- ACT-DEVIATION ×0 实质（Deviation 1 fmt 格式校准非实质）
- PLAN-INVALID ×0、BASELINE-CHANGED ×0、NEW-EVIDENCE ×0

**Acceptance Gaps**

R2 S「既有 UPDATE 行为零回归」未满足：keyless_row_test 1/4 失败（T8-R2）、全量 860 passed / 1 failed / 2 ignored（Plan 独立 `--no-fail-fast` 复跑与 Act 证据一致）。Gap 根因是 Plan 契约冲突而非实现缺陷——实现本身满足 R1（新套件 5/5 GREEN）。

**Convergence**

N/A（首次 Review）

**Evidence**

- 独立复现：`cargo test --test keyless_row_test` → `test result: FAILED. 3 passed; 1 failed`（`keyless_row_update_recovery_after_crash`）；`cargo test --no-fail-fast` 全量 → `total passed: 860 failed: 1 ignored: 2`（唯一 FAILED = T8-R2）。
- 独立代码检查：`git diff src/executor/update.rs`（Step 7 分支与 D2 同构）；`tests/update_index_maintenance_test.rs`（5 用例 + 模块文档）；`tests/keyless_row_test.rs:127-215`（T8-R2 链路前提）。
- 采信 Act 未失效结论：新套件 5/5 GREEN、Risks 探针（serialize 时序）、clippy/fmt/validate 干净——覆盖范围未变化。
- 方向 b（维持残留可达性）裁定拒绝：与用户已批准的 R1 直接矛盾。

**Follow-up Decision**

既有 Acceptance 未满足且修复需改变验收边界（「零修改」→「零修改 + T8-R2 语义校准」）与 spec R2 措辞——超出当前 Cycle 执行契约，构成 **replan-required**。已执行 replan 流程：(1) change 文档修订完成——delta spec R1 增场景「键位无键行对键位等值 UPDATE 不可达」、R2 增校准条款、tasks T6 修订、Iteration 001 Stable baseline 措辞修订、design D2 增结构性后果注；(2) 后继 Cycle 已创建：`iterations/001-update-key-index/001-replan.md`（Plan Context `draft`，T6 校准 + 收尾契约自包含）。恢复条件 a 采纳（校准 T8-R2 适配 R1 语义）、b 拒绝。**待用户批准**：校准涉及修改既有测试（MS10-T05 产物）——批准后 replan Cycle 转 `ready` 交接 Act；F3 的 I 项登记与 F4 并入校准，归 Act Response / docs-maintainer 收尾面。

**Iteration Plan Update**

Iteration Map 结构不变（001 内 replan，002 留 Map）；Iteration 001 Stable baseline 与验证边界措辞已修订（「全量回归零修改」→「全量回归通过（T8-R2 按 R1 语义校准除外）」），tasks T6 同步——见 tasks.md 与 001-replan.md。

**Next Cycle**

`iterations/001-update-key-index/001-replan.md`（Cycle Type: replan；Plan Context `draft`，待用户批准后 `ready`）

**Next Iteration**

None（Iteration 001 于 replan Cycle accepted 前保持未完成；Iteration 002 展开以 accepted 为前提）
