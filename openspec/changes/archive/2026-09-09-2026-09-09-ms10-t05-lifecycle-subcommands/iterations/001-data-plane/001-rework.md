# Iteration 001 / Cycle 001-rework: 无键行落库语义与 R-import S3/S4 收口

## Plan Context

- Status: ready（2026-09-09 用户批准计划，Gate 2 通过；用户指令「更改gate状态，开始实施」交 openspec-act）
- Iteration: 001-data-plane
- Cycle: 001-rework
- Cycle Type: rework
- Parent cycle: [000-initial.md](000-initial.md)（blocked；Review Result: rework-required，Deviation 1 = PLAN-INVALID 非阻塞，阻塞为引擎既有缺陷 NEW-EVIDENCE 性质）

**Iteration Scope**

- Change tasks: T8（经由 repair items，Iteration Map 不变）；T9 随本 Cycle 收口
- Depends on: None（T6/T7 已在父 Cycle 完成且全绿，本 Cycle 不触碰其产出）
- Stable baseline: 与父 Cycle 相同 + R-import S3/S4 转绿（键位不可键控行落库不入索引、空字段语义端到端成立）+ 无键行崩溃恢复语义正确
- Verification boundary: `cargo test --all` 全绿（既有零修改）；clippy/fmt/openspec validate 全 0/PASS
- Diagnostic boundary: `src/executor/insert.rs`、`src/wal/recovery.rs`、`tests/cli_test.rs`、恢复集成测试文件
- Deferred tasks: None（T9 为 change 级终门，随本 Cycle 执行）

**Cycle Scope**

- Trigger: rework-required（父 Cycle Plan Review：阻塞 R-import S3/S4，根因为引擎「键位不可键控行静默丢弃」既有缺陷）
- Acceptance gaps: R-import S3「类型转换与空字段语义」（S4 空字段断言同病）——`test_import_types_and_empty_fields` RED
- Repair items: T8-R1（insert 无键落库）、T8-R2（恢复无键回退）、T8-R3（见证转绿 + 全形状往返/恢复见证 + T9 全量门）
- Inherited scope: 父 Cycle T6/T7 全部产出冻结（dump/restore/字面量纯函数/stdin 夹具）；T8 已完成实现面冻结（csv_value/import_csv/表头匹配/分发接线——仅其 RED 见证随 R1 自然转绿）；用户决策方向 A（2026-09-09）：键位值可键控（Int）行为不变；不可键控（NULL/非 Int）行落库但不入索引
- Excluded scope: `to_key` 键控面扩展（String/Float 键控——B-Tree Key 32 字节定长，独立设计工程，improvement 候选）；DDL 级 PK 类型拒绝；隐式主键机制重设计（improvement 候选）；WAL 磁盘格式/帧格式变更（文件格式版本 bump 会拒绝既有库，已排除）；运行时 NOT NULL 强制；planner/pipeline/CLI 层修改

**Objective**

键位（PK 列，未声明时为第一列）值不可键控的行由「整行静默丢弃」改为「正常落库但不入索引」（可观察行为：行可见、affected 计数如实、dump/restore 往返可用；唯一性检查与索引点查对这类值不适用），并同步补齐恢复路径的 Update 重放无键回退（否则无键行 INSERT+UPDATE+崩溃 → `RedoFailed` → 库不可打开）；R-import S3/S4 按已批准 spec 原文转绿，全量门通过。

**Background**

父 Cycle T8 实现完成时发现：`Value::to_key()` 仅支持 Int（`src/executor/value.rs:82-90`），`InsertExecutor::next` 对键位不可键控的行在 WAL 之前静默丢弃（`src/executor/insert.rs:96-99`）。Review 补充调查确立三个事实：① SQL 路径不存在无主键表——未声明 PK 时第一列被隐式设为主键（`create_table.rs:60-69`）且 schema/dump 渲染 `PRIMARY KEY`（`lifecycle.rs:534`），故 S3 的本质是隐式 PK 列接受 NULL；② Update 重放对 `extract_pk_key(old_tuple)==None` 硬性 `RedoFailed`（`recovery.rs:591-599`），放开无键落库必须同步恢复回退；③ Delete 重放/索引重建/deindexed Insert 重放对无键行已安全（位置寻址 + `if let Some(key)` 守卫）。方向 A 经用户裁定；本 Cycle 同时交付 `wal-recovery-replay-integrity` 的 MODIFIED spec delta（推导措辞扩展至无键行）。

**Current Baseline**

- revision `a5b0a5f` + Iteration 000 两 Cycle + Iteration 001 父 Cycle 工作树实施（未 commit）
- 独立验证（2026-09-09，父 Cycle Review）：`cargo test --all` → lib 与各集成套件全绿，cli_test 46/1/2（唯一失败 = 阻塞见证）；clippy 0 / fmt 干净 / validate PASS
- 无键行现状：INSERT 静默丢弃（无 WAL 残留、affected 少计、exit 0）——父 Cycle Blocker Handoff 三组探针可低成本复现

**Current-State Evidence**

- **丢弃点**：`InsertExecutor::next`（`src/executor/insert.rs:93-158`）对每行：`pk_value.to_key()` None → `continue`（:96-99，先于 serialize/write_tuple/visibility/WAL/record_version）；Some → `index_manager.search` 重复检查（DuplicateKey，:101-109）→ 写页 → WAL → 版本记录 → `index_manager.insert`（:152-155）→ count += 1。**无键落库 = 跳过 search 与 index.insert 两步，其余逐行路径原样。**
- **键位语义**：`TableMeta.pk_index`/`pk_column`——声明 PK 用声明列；未声明时 executor 取第一列（`create_table.rs:60-69`），经 `create_table_with_constraints` 持久化 `pk_index`/`pk_column`（`table_manager.rs:250-253,304-305`）。planner 点查只对 `primary_keys` 注册表生效且经 `to_key`（`query.rs:745-786`，非 Int 值 → `Ok(None)` → 全扫描兜底，可查到无键行）。
- **恢复路径（`src/wal/recovery.rs`）**：
  - `PkVersionMaps = HashMap<String, HashMap<Vec<u8>, Vec<RowId>>>`（:29，table → PK key → rids）；`build_pk_version_maps`（:191-）预扫描数据页链，`let Some(key) = extract_pk_key(...)` 否则跳过（:228）；
  - deindexed 门控：`will_redo`（任一已提交数据记录 ≥ redo_from）→ `pk_versions: Some`（:425-438）——重放记录必然 deindexed 模式；非 deindexed 的 Insert/Update redo 索引分支对重放不可达；
  - Insert redo deindexed 臂：`if let Some(key)` 追加多映射（:520-530），无键 slot 今日静默不入映射；
  - **Update redo**（:572-677）：`extract_pk_key(old_tuple)` None → `RedoFailed`（:591-599，本 Cycle 缺口）；Some → deindexed 臂经多映射 `derive_old_row_id`（max rid < 记录 row_id + old_tuple 逐字节校验，:600-608）→ 位置写入新版本（`next_version → old_row_id`，:628-642）→ 新版本入映射（`if let Some(new_key)`，:647-655）；
  - Delete redo：位置寻址墓碑 + 非 deindexed 才做 `find_key_by_row_id` 索引清理（:679-731）——无键行安全；
  - 索引重建（redo_count > 0，:736-）：链回溯位置寻址，`if let Some(key)` 守卫下才入 `entries`（:856-863）——无键行安全（不入索引、不触发重复报错）。
- **既有测试兼容性**：`test_pipeline_join_with_null_keys`（`tests/pipeline_test.rs:610-649`）INSERT 响应被弃、JOIN 计数在丢弃/不匹配两语义下同为 1 行；全量套件无其他键位 NULL/非 Int 插入用例（grep 实证）。
- **见证用例**：`tests/cli_test.rs:1512-1545`——表 `t (a INT, b FLOAT, c BOOL, d STRING)`（隐式 PK = a），CSV `,2.5,TRUE,keep` + `7,,false,`，断言两行落库且空字段 → NULL/空串。
- **spec 现状**：`openspec/specs/wal-recovery-replay-integrity/spec.md` Requirement「重放保持 DML 语义」推导措辞钉在「old_row_id 由 old_tuple 提取 PK 经索引推导」——MODIFIED delta 已随本 change 创建（`specs/wal-recovery-replay-integrity/spec.md`）。
- 恢复集成测试夹具先例：`tests/wal_recovery_large_test.rs`（7 测试，drop 不 close → reopen 模式）；recovery 单测在 `src/wal/recovery.rs` `#[cfg(test)]`。

**Relevant Code**

- `src/executor/insert.rs` — T8-R1 宿主（:96-99 丢弃点改为无键落库分支）
- `src/wal/recovery.rs` — T8-R2 宿主（`PkVersionMaps` 结构扩展 + 三处追加点 + Update redo 推导回退）
- `src/executor/value.rs::to_key` — 只读复用，零修改
- `tests/cli_test.rs` — T8-R3 见证转绿 + 全形状往返用例
- `tests/wal_recovery_large_test.rs` / recovery 内联单测 — T8-R2 恢复见证（夹具形态非实质）

**Critical Path**

T8-R1（insert 无键落库）→ T8-R2（恢复回退；其 RED 见证依赖 R1 先落地——R1 后无键行进入 WAL，INSERT+UPDATE+崩溃 reopen 即 `RedoFailed`）→ T8-R3（S3/S4 见证转绿 + 往返/恢复见证）→ T9 全量门。数据流：无键行经正常写页/WAL/版本路径落库；恢复预扫描与重放按 tuple 内容为无键行建候选集；Update 重放按同一 max-rid + 逐字节校验语义派生 old_row_id；重建对无键行不入索引。

**Implementation Guidance**

顺序 R1 → R2 → R3。R1：`to_key()==None` 分支保留 `continue` 语义删除——改为跳过「重复检查 + index.insert」两步（`index_manager.search` 与末尾 `index_manager.insert`），行写入主体（serialize → write_tuple/write_tuple_to_data_page → visibility → WAL append → record_version → count += 1）逐行不动；不新增错误变体。R2：`PkVersionMaps` 建议重构为 struct `{ keyed: HashMap<String, HashMap<Vec<u8>, Vec<RowId>>>, keyless: HashMap<String, HashMap<Vec<u8>, Vec<RowId>>> }`（keyless 桶 key = tuple 原始字节；struct 化或并行 map 为非实质），三处追加点——预扫描 :228（None → keyless 桶）、Insert redo :522 else、Update redo :647 else（新版本 tuple 也可能无键）；Update redo :591 None → 不再 RedoFailed → 从 keyless 桶按 old_tuple 字节取 candidates → 复用 `derive_old_row_id`（同一 max rid < record.row_id + 逐字节校验）；非 deindexed 分支保持现状（对重放不可达）。R3 见证：`test_import_types_and_empty_fields` 断言零修改转绿；新增全形状往返（含 NULL 键位行 + String 首列表 dump → new → restore → SELECT 比对）与无键行 INSERT+UPDATE+崩溃恢复 Database 级测试（drop 不 close → reopen → 更新值可见、行数精确、无 RedoFailed、可键控行不受影响）。lib 单测：可键控行路径行为保持守卫（DuplicateKey 仍拒绝）+ 无键行 affected 计数如实。

**Behavioral Change**

| 场景 | 当前 | 目标 |
|---|---|---|
| 键位值 NULL（隐式或声明 PK 列）INSERT | 整行静默丢弃，affected 0，exit 0 | 落库可见，affected 1；不入索引（无唯一性检查，NULL≠NULL） |
| 键位值 String/Float/Bool（如 String 首列隐式 PK 表）INSERT | 整表所有行静默丢弃 | 落库可见；不入索引（唯一性不强制——文档化限制） |
| 键位值 Int | 入索引 + 唯一性检查 | 逐字节不变 |
| 无键行 INSERT + UPDATE + 崩溃 reopen | 不可达（行不存在） | 恢复成功，更新可见，无 RedoFailed |
| dump → new → restore（含无键行形状） | restore 后无键行丢失 | 往返等价 |
| 点查 `WHERE 键位 = 值` | 可键控走索引；非 Int 值全扫描 | 不变（无键行经全扫描可达） |
| 既有全部测试 | — | 零修改全绿 |

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T8-R1 | R-import/S3、S4（上游数据面） | `src/executor/insert.rs::next`（:96-99） | to_key None → continue | 跳过索引两步，行写入主体原样 |
| T8-R2 | wal-recovery-replay-integrity/无键行场景（新增） | `src/wal/recovery.rs`（PkVersionMaps/build_pk_version_maps/Insert redo/Update redo） | 无键 slot 不入映射；Update 重放 RedoFailed | keyless 桶三处追加 + 推导回退 |
| T8-R3 | R-import/S3、S4 + R-dump-restore/S1 全形状 | `tests/cli_test.rs`、恢复集成测试 | 见证 RED | 转绿 + 新增往返/恢复用例 |
| T9 | change 级验证边界 | 全仓只读 | 基线 46/1/2（cli_test） | 全量门 + clippy/fmt/validate |

**Task Contracts**

### T8-R1: insert 无键落库（executor）

- Requirement/Scenario: R-import / S3、S4（上游数据面）
- Depends on: None
- Targets: `src/executor/insert.rs::next`
- Current behavior: `pk_value.to_key()` None → `continue`（整行丢弃，不写页、不入 WAL、不计 affected、不报错）
- Required behavior: None → 跳过重复检查（`index_manager.search`）与索引插入（`index_manager.insert`），行写入主体（serialize → write_tuple → visibility → WAL append → record_version → count += 1）原样执行；可键控行路径逐字节不变（search → DuplicateKey → 写页 → WAL → 版本 → index.insert 顺序不变）
- Required changes: :96-99 match 臂重构 + 末尾 index.insert 条件化
- Preserve: 可键控行全部行为；`to_key` 本体；WAL 记录形状；无新错误变体
- Forbidden: 不改 `to_key`；不做 DDL 级 PK 类型拒绝；不引入运行时 NOT NULL 强制；不改 WAL 格式
- Test witness: 变更前 RED——lib/集成级：String 首列隐式 PK 表 INSERT 2 行 → SELECT 计 2 行（今日 0 行）；NULL 键位行 INSERT → affected 1 + SELECT 可见；行为保持守卫：可键控重复插入仍 `DuplicateKey`
- GREEN condition: 新增见证绿 + `cargo test --lib` 与 `cargo test --test cli_test` 既有用例零回归（见证用例 `test_import_types_and_empty_fields` 随本 repair 自然转绿或留 T8-R3 确认）
- Verification: `cargo test --lib && cargo test --test cli_test`（exit 0）
- Stop when: 无键落库破坏 MVCC 可见性/扫描去重（R6）/事务回滚语义（实质冲突 → Blocker Handoff）

### T8-R2: 恢复无键回退（wal）

- Requirement/Scenario: wal-recovery-replay-integrity / 新增场景「无键行 Update 崩溃恢复语义正确」（MODIFIED delta 已随 change 创建）
- Depends on: T8-R1（无键行进入 WAL 后其 RED 才可观测）
- Targets: `src/wal/recovery.rs`（`PkVersionMaps`、`build_pk_version_maps`、Insert redo deindexed 臂、Update redo）
- Current behavior: 无键 slot 不入映射（预扫描 :228 / Insert redo :522 / Update redo :647 三处 `if let Some(key)` 跳过）；Update 重放 `extract_pk_key(old_tuple)` None → `RedoFailed`（:591-599）
- Required behavior: 无键行按 tuple 原始字节入 keyless 桶（三处追加点全接）；Update redo None → 从 keyless 桶按 old_tuple 字节取 candidates → 复用 `derive_old_row_id`（max rid < 记录 row_id + 逐字节校验）→ 位置写入新版本；可键控行恢复路径逐字节不变；Delete redo/索引重建/非 deindexed 分支不动
- Required changes: `PkVersionMaps` 结构扩展（keyed + keyless 双桶）+ 三处追加点 + Update redo 推导回退
- Preserve: `RedoFailed` 其余全部触点与 K05 显式报错语义；可键控行多映射推导逐字节不变；WAL 磁盘格式/帧格式/文件格式版本零变化
- Forbidden: 不改 WAL 编码；不动索引重建段；不改非 deindexed 分支；不引入全页扫描式推导
- Test witness: 变更前 RED（依赖 R1）——Database 级：建表（含 NULL 键位行与可键控行）→ INSERT 无键行 → UPDATE 之 → drop 不 close（崩溃模拟）→ reopen：今日 `RedoFailed` → `Database::open` Err；GREEN：恢复成功、更新值可见、行数精确、可键控行恢复不受影响
- GREEN condition: 恢复见证绿 + `tests/wal_recovery_large_test.rs` 等既有恢复套件零回归
- Verification: `cargo test --all`（exit 0）
- Stop when: 无键回退与 R6 扫描去重/R8 重建谓词/墓碑语义冲突（实质 → Blocker Handoff）

### T8-R3: S3/S4 见证转绿 + 全形状见证 + T9 全量门

- Requirement/Scenario: R-import/S3、S4 + R-dump-restore/S1 全形状 + change 级验证边界
- Depends on: T8-R1, T8-R2
- Targets: `tests/cli_test.rs`（既有 RED 见证 + 新增用例）、全仓只读验证
- Current behavior: `test_import_types_and_empty_fields` RED（断言零修改）；往返用例仅 Int-PK 形状
- Required behavior: 见证用例断言零修改转绿；新增全形状往返用例（含 NULL 键位行与 String 首列表：dump → new → restore → SELECT 比对等价）；T9 全量门通过
- Required changes: 新增集成用例（约 2 个）；无代码改动（T9）
- Preserve: 既有断言零修改；dump/restore/import 实现面（父 Cycle 产出）零修改
- Forbidden: 不放宽任何既有断言；不给 CLI 层加无键行特判
- Test witness: R1 后 `cargo test --test cli_test test_import_types_and_empty_fields` 转绿；新增往返用例 RED→GREEN 随 R1/R2 落地；T9 全量输出
- GREEN condition: `cargo test --all` 全绿（既有零修改）；`cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` 全 0/PASS
- Verification: `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`
- Stop when: 既有测试出现计划外破坏（→ Blocker Handoff，不得静默改断言）

**Invariants**

- 可键控（Int 键位值）行全路径行为零变化：insert（重复检查/索引）、点查、恢复推导、重建、GC。
- 既有全部测试零修改（唯一预期状态变化 = `test_import_types_and_empty_fields` RED → GREEN 与新增用例；`test_pipeline_join_with_null_keys` 等已预检兼容）。
- WAL 磁盘格式/帧格式/文件格式版本零变化；`to_key` 零修改；无新 StorageError/WalError 变体。
- CLI 层（Iteration 000/001 已实现面：dump/restore/import/new/list/schema/主命令）零修改。
- 无键行唯一性不强制与不入索引为**文档化语义**（SQLite NULL-PK 先例），不是缺陷：其在 `sql_literal`/DDL 渲染侧无特判。

**Non-goals**

String/Float/Bool 键控扩展；DDL 级 PK 类型拒绝；隐式主键机制重设计；WAL 格式变更；运行时 NOT NULL 强制；GC 对无键行版本链的覆盖（`gc_table` 经 `index_manager.scan_all` 不可达无键链——既有可选维护路径，improvement 候选）；planner/pipeline/CLI 层修改。

**Acceptance**

- R-import S3/S4：`test_import_types_and_empty_fields` 断言零修改 GREEN。
- wal-recovery-replay-integrity 新场景：无键行 INSERT+UPDATE+崩溃恢复 Database 级见证 GREEN（无 RedoFailed、终态精确）。
- R-dump-restore S1 全形状：含 NULL 键位行/String 首列表的 dump → new → restore 往返等价用例 GREEN。
- 行为保持守卫：可键控重复插入仍 `DuplicateKey`；既有恢复套件（`tests/wal_recovery_large_test.rs` 等）零回归。
- T9 全量门：`cargo test --all` 全绿 + clippy/fmt/validate 全 0/PASS。
- 映射：S3/S4 → T8-R1+T8-R3；恢复场景 → T8-R2；全形状往返 → T8-R3；insert.rs/recovery.rs/cli_test.rs/恢复测试 → 上述命令。

**Verification**

- `cargo test --lib`（R1 行为守卫单测）
- `cargo test --test cli_test`（S3/S4 转绿 + 全形状往返；既有 46 零修改）
- `cargo test --all`（含恢复套件零回归）
- `cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 丢弃链/恢复依赖链/重建守卫/deindexed 门控/既有测试兼容性全部独立核实（父 Cycle Review Evidence 节 + 本文件 Current-State Evidence）；Act 候选②与 WAL 格式路线经证据排除 |
| Design | PASS | 方向 A 经用户裁定（2026-09-09 AskUserQuestion）；无键行语义（落库不入索引 + keyless 桶回退 + max-rid/字节校验沿用）闭合；内容等同孪生行链分叉的内容等价论证见 Risks |
| Iteration Plan | PASS | Map 不变；repair items 映射 T8，同 Iteration 目录 |
| Cycle Scope | PASS | gap = R-import S3/S4 单项；inherited（T6/T7 + T8 实现面）冻结、excluded 明确 |
| Task Contracts | PASS | T8-R1/R2/R3 自包含（目标符号、行为、见证、停止条件）；T9 验证契约 |
| Traceability | PASS | gap → repair → file → test 链闭合（Acceptance 节）；spec delta 已随 change 创建 |
| Verification | PASS | RED→GREEN 见证 + 零回归门 + T9 全量门；无身份型证据 |
| 用户批准计划 | PASS | 2026-09-09 用户指令「更改gate状态，开始实施」（Gate 2 批准）；Status 已置 `ready` 并交 openspec-act |

**Persisted Evidence**

- Mode: none

`none`：全部验证命令低成本可重跑，Act Response 的命令 + ≤20 行决定性输出 + 退出码足以承载 Gate 5 与 Review；无一次性环境、无 Incident 现场。Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- **内容等同孪生无键行**：无键行无唯一性检查，同内容多副本时 keyless 桶同 key 多 rid；`derive_old_row_id` 的 max-rid 规则可能链到内容等同的孪生版本——链拓扑分叉但字节内容等同，读路径可观察行为不变；Delete 墓碑按 rid 位置寻址不受链分叉影响。以 T8-R2 恢复见证锁定实际行为；若测试暴露可观察偏差 → Blocker Handoff。
- **GC 盲区**：无键行版本链经 `index_manager.scan_all` 不可达，`gc_table` 不覆盖（既有可选维护路径，improvement 候选，不阻塞）。
- 既有测试若存在未预检的丢弃行为依赖 → T8-R3 全量门暴露 → Blocker Handoff（不得静默改断言）；已预检 `test_pipeline_join_with_null_keys` 兼容。
- 非 deindexed Update/Insert redo 分支保持现状：对重放记录不可达（`will_redo` ⇔ deindexed，recovery.rs:425-438 已核实）；若 Act 发现可达路径 → Blocker Handoff。
- 非实质留 Act：`PkVersionMaps` struct 化 vs 并行 map、恢复见证夹具形态（`tests/wal_recovery_large_test.rs` 先例 vs recovery 内联单测）、R1 分支实现形态。

## Act Response

- Status: reported（2026-09-09，T8-R1 + T8-R2 + T8-R3 + T9 全部完成且全量门通过）

**Implemented**

- **T8-R1（完成）**：`InsertExecutor::next` 键位 `pk_value.to_key()` None 分支由整行 `continue` 改为「跳过重复检查（`index_manager.search` → DuplicateKey）与索引插入（`index_manager.insert`）两步」，行写入主体（serialize → write_tuple/table_manager.write_tuple → visibility → WAL append → record_version → count += 1）逐行原样；可键控行路径逐字节不变（search → DuplicateKey → 写页 → WAL → 版本 → index.insert 顺序不变）；无新错误变体。无键行落库不入索引以代码注释锚定为文档化语义（SQLite NULL-PK 先例）。
- **T8-R2（完成）**：`PkVersionMaps` 由单 map 类型别名重构为 `#[derive(Default)]` struct 双桶——`keyed`（PK 键字节）/ `keyless`（tuple 原始字节，桶键即 tuple 字节）；`build_pk_version_maps` 预扫描无键 slot 经新私有 `PrescanSlotKey::Keyless` 入 keyless 桶（双桶候选集尾排序经 `.chain()` 保持）；Insert redo deindexed 臂与 Update redo 新版本追加点按有无键分桶；Update redo `extract_pk_key(old_tuple)` None 不再 `RedoFailed`——从 keyless 桶按 old_tuple 字节取候选集，复用 `derive_old_row_id`（max rid < record.row_id + 逐字节校验）；**非 deindexed Update 分支保持现状**（None → RedoFailed 语义与文案逐字保留）；Delete redo / `rebuild_pk_indexes` / 非 deindexed Insert 臂零触碰；WAL 磁盘/帧格式零变化。
- **T8-R3（完成）**：`test_import_types_and_empty_fields` 断言零修改转绿（R1 落地后 cli_test 47/0 实测）；新增 `test_dump_restore_roundtrip_full_shape`（NULL 键位行 + String 首列隐式 PK 表的 dump → new → restore → SELECT 比对等价，含 dump 侧无键行 INSERT 文本断言与两表行集等价断言）。
- **T9（完成）**：全量门 `cargo test --all`（704 passed / 0 failed / 2 ignored，62 测试二进制全 ok）+ clippy 0 code warning + fmt clean + `openspec validate` PASS。

**Changed Files and Symbols**

- `src/executor/insert.rs`：`InsertExecutor::next`（key 提取 Option 化 + 两处索引操作 `if let Some` 条件化；写入主体未动）。
- `src/wal/recovery.rs`：`PkVersionMaps`（类型别名 → struct 双桶）、`PrescanSlotKey`（新私有 enum）、`PreScanPage`（条目类型随之）、`build_pk_version_maps`（双桶收集 + 双桶尾排序）、`redo_record` Insert 臂 deindexed 分支（分桶追加）、Update 臂（old_row_id 派生桶选择 + 新版本分桶追加；非 deindexed 分支 None → RedoFailed 保留）、`extract_pk_key` doc comment 更新（RedoFailed 语义范围注记）。
- `tests/keyless_row_test.rs`（新文件，4 测试）：R1 三用例（String 首列隐式 PK 表两行落库可见、NULL 键位行 affected 1 + SELECT 可见、可键控重复 DuplicateKey 守卫）+ R2 崩溃恢复见证（无键行 INSERT + SET-NULL 链 UPDATE + drop 不 close 重开）。
- `tests/cli_test.rs`：`test_dump_restore_roundtrip_full_shape`（新增 1 用例）。
- `iterations/001-data-plane/001-rework.md`：Plan Context Status `draft` → `ready`（Gate 2 用户批准留痕）+ Gate 2 Readiness 批准行 + 本 Act Response。

**Deviations from Plan**

1. **R1 见证断言调整（计划证据修正，非契约裁剪）**：Plan 证据/行为变化表断言「点查 `WHERE 键位 = 值` 非 Int 值全扫描兜底，无键行经全扫描可达」——实测 M19 路由对「简单 PK 等值 + 非 Int 字面量」（`WHERE s = 'x'`）经 `has_pk_equality` 分支生成 Filter(**Scan**)（`ScanExecutor` 走 `index_manager.scan_all()` 索引遍历），无键行经该 WHERE 形态不可达（T8-R1 暴露的既有 planner 行为——此前此类表恒为空、行为不可观察）。见证改按契约原文形态（SELECT COUNT(*) + 非 PK 谓词走 DataScan 下推可达无键行）；planner 层修改属本 cycle Excluded scope，不处理，登记 Remaining Issues #1。
2. **R2 见证构造形态**：契约草图为「INSERT 无键行 → UPDATE 之」——`build_update` 要求 WHERE 为 `pk = 可键控值` 且经索引定位，INSERT 的无键行无法被 UPDATE 命中（SQL 面不可达）；改用可达构造：键行 (5,0) → `UPDATE SET a = NULL WHERE a = 5`（v2 无键，索引 key5 → v2）→ 再次 `UPDATE SET v = 42 WHERE a = 5`（old_tuple 无键 = RedoFailed 形态）。见证判定性质与契约一致（RED：`Database::open` Err RedoFailed；GREEN：恢复成功、行数精确、更新值可见、可键控行守卫保持）。
3. **测试文件组织**：R1/R2 见证落新文件 `tests/keyless_row_test.rs`（契约允许 lib/集成级、未点名文件；本仓库 per-task 测试文件先例），cli_test.rs 仅新增 T8-R3 契约内用例。夹具采用 `wal_recovery_large_test.rs` 先例（flush_all + shutdown + drop 不 close）。
4. `cargo fmt` 对新测试文件一处 `assert_eq!` 换行做机械格式化（非实质）。

**Blocker Handoff**

None（本 Cycle 无阻塞）

**Blocker Resolution**

None（未经历阻塞）

**Self-Review**

- Plan compliance: T8-R1/R2/R3 按契约全绿；Invariants 逐项核实——可键控行全路径零变化（insert 键控路径、DuplicateKey 守卫、恢复/B-Tree 套件 8 套 55 测试零回归）；既有测试零修改（唯一预期状态变化 = S3/S4 见证转绿 + 新增用例）；WAL 磁盘/帧格式/文件格式版本零变化；`to_key` 零修改；无新 StorageError/WalError 变体；CLI 层零修改；无键行不入索引为文档化语义（注释锚定，`sql_literal`/DDL 渲染无特判）。
- Full diff reviewed: yes——`insert.rs`/`recovery.rs` 全量 diff 逐 hunk 过目（非 deindexed 分支、Delete redo、`rebuild_pk_indexes`、`redo_tuple_at_row_id` 零触碰核实）；`keyless_row_test.rs` 全文与 `cli_test.rs` 增量过目；跨任务交互（R1 无键行落库 ↔ R2 恢复追踪 ↔ R3 往返）无计划外修改。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1（planner M19 简单 PK 等值 + 非 Int 值路由 Filter(Scan) 对无键行不可达——Deviation 1，既有行为新暴露，Excluded scope 不处理，见 Remaining Issues）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T8-R1 见证 RED | `cargo test --test keyless_row_test`（实施前） | `3 failed; 1 passed`（三无键用例 panic `AffectedRows { count: 0 }`；守卫用例绿） | RED 确认 |
| T8-R1 GREEN | `cargo test --test keyless_row_test && cargo test --test cli_test` | keyless `3 passed`（R2 见证除外）；cli_test `47 passed; 0 failed; 2 ignored`（含 S3/S4 转绿） | PASS |
| T8-R2 见证 RED | `cargo test --test keyless_row_test keyless_row_update_recovery_after_crash`（R2 前） | `Database::open` Err：`WalError("WAL redo failed: update redo: table 't' old PK extraction failed")` | RED 确认（契约预测形态） |
| T8-R2 GREEN | `cargo test --test keyless_row_test` | `4 passed; 0 failed` | PASS |
| 恢复边界回归 | `cargo test --test wal_recovery_large_test --test recovery_test --test recovery_e2e_test --test btree_scale_test --test checkpoint_redo_reduction_test --test checkpoint_test --test btree_merge_test --test btree_split_test` | 8 套全 ok（10+5+12+9+3+6+3+7 = 55 passed / 0 failed） | PASS |
| T8-R3 目标测试 | `cargo test --test cli_test test_dump_restore_roundtrip_full_shape` | `1 passed; 0 failed` | PASS |
| T9 全量门 | `cargo test --all` | `passed=704 failed=0 ignored=2`（62 测试二进制全 ok） | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | 0 code warning（仅 cargo config 弃用提示，非代码告警） | PASS |
| 格式 | `cargo fmt --check` | 无 diff | PASS |
| OpenSpec | `openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` | `Change ... is valid` | PASS |

**Persisted Evidence**

None required（`none` 模式：全部验证命令低成本可重跑，上表决定性输出足以承载 Gate 5 与 Review；无一次性环境、无 Incident 现场）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None（常规 TDD 缺口修复 + 恢复语义补齐，无已验证可重复操作路径或已发生故障） |

**Remaining Issues**

1. planner M19「简单 PK 等值 + 非 Int 字面量」路由 Filter(Scan)（`ScanExecutor` 索引遍历），无键行经该 WHERE 形态不可达（Deviation 1）——T8-R1 暴露的既有行为；非 PK 谓词与无 WHERE 的 DataScan 路径可达无键行，本 change 全部 spec 场景不依赖该形态。属 planner 路由面（本 cycle Excluded scope），建议 Plan Review 评估登记 improvement。
2. 无键行 GC 盲区（`gc_table` 经 `index_manager.scan_all` 不可达无键链）——Plan Context Risks 既登 improvement 候选，本 Cycle 未触碰。
3. 多代表名引号膨胀与负数字面量 INSERT 限制——父 Cycle（001-data-plane/000-initial）Remaining Issues #2/#3 既有记录，本 Cycle 无新增。

**Commit or Diff Reference**

未 commit（工作树现场：Iteration 000 两 Cycle + 001 父 Cycle 产出 + 本 rework——`src/executor/insert.rs`、`src/wal/recovery.rs` 修改，`tests/keyless_row_test.rs` 新增，`tests/cli_test.rs` 增量 + change 目录与 R20 登记等规划产物）

## Plan Review

- Review Result: accepted（2026-09-09）

**Findings**

1. **T8-R1/R2/R3 + T9 独立验证 PASS**：Plan 独立重跑 `cargo test --all`（两次）→ `passed=704 failed=0 ignored=2`、`FAILED` 计数 0、编译错误 0（与 Act 声明一致）；`cargo clippy --all-targets -- -D warnings` exit 0（仅 cargo config 弃用提示，非代码告警）；`cargo fmt --check` 干净；`openspec validate` PASS。
2. **Diff 逐项比对契约吻合**：
   - T8-R1（`src/executor/insert.rs`）：仅两处索引操作条件化（重复检查 `search` → DuplicateKey 与末尾 `index_manager.insert`），行写入主体（serialize → write_tuple → visibility → WAL → record_version → count）逐行未动；可键控行路径顺序不变；无新错误变体；`to_key` 零修改。
   - T8-R2（`src/wal/recovery.rs`）：`PkVersionMaps` 单别名 → `#[derive(Default)]` struct 双桶（keyed = PK 键字节 / keyless = tuple 原始字节）+ `PrescanSlotKey` enum；三处追加点全部接通——预扫描无键 slot 入 keyless 桶（:246-260）、Insert redo deindexed 臂分桶（:544-570）、Update redo 新版本分桶（:692-706）；Update 推导回退：`old_key` Option 化，None → keyless 桶按 old_tuple 字节取候选集 → 复用 `derive_old_row_id`（同一 max rid < record.row_id + 逐字节校验）；**非 deindexed 分支 `RedoFailed` 语义与文案逐字保留**；Delete redo / `rebuild_pk_indexes` / 非 deindexed Insert 臂 / `record.rs`（WAL 格式）零触碰（均不在 diff 中）。
   - T8-R3：`test_import_types_and_empty_fields` 当前正文与本 Review 前置审计（父 Cycle Review 时留存）逐字一致——断言零修改转绿；`test_dump_restore_roundtrip_full_shape` 双表全形状（NULL 键位行 + 键位 Int 行含 NULL 非键字段/空串 + String 首列隐式 PK 表全无键行）dump→new→restore 断言闭合，含 dump 侧无键 INSERT 文本断言与 restore 静默断言。
3. **Deviation 1 技术声明独立核实成立**：`query.rs:430-475` 非 PK WHERE 分支中 `has_pk_equality`（结构化检查，不问可键控性）为真 → `Filter(base_plan=Scan)`，`ScanExecutor` 走索引遍历 → 无键行经「简单 PK 等值 + 非 Int 字面量」WHERE 形态不可达；Plan 行为变化表「点查非 Int 值全扫描可达」对该形态错误（PLAN-INVALID，非阻塞）——Act 以契约原文形态（`COUNT(*)` / 非 PK 谓词 DataScan 下推）修正见证，planner 面按 Excluded scope 正确不处理。
4. **Deviation 2 构造核实成立**：`update.rs:70` `search(&self.key)` 证实 `build_update` 要求 WHERE `pk = 可键控值` 经索引定位，INSERT 产生的无键行 SQL 面不可 UPDATE；「键行 → `SET a = NULL` → 再 UPDATE」链产生同判定形态（old_tuple 无键）的 Update 记录，见证判定性质（RED：open Err `old PK extraction failed`，与 Plan 预测形态一致；GREEN：恢复成功/行数精确/更新可见/可键控守卫保持）与契约 Test witness 一致。
5. **Review 新观察（Minor，非阻塞，既有行为）**：键行经 UPDATE 变为无键值后，运行期索引旧键条目被无条件 update 指向无键新版本（`update.rs:130-133` 以旧 key 调 `index_manager.update`）——运行期唯一性检查按旧键误拒（无行实际持有该键值），点查可达性语义含混；崩溃恢复后重建自然清除（无键版本不入重建索引，R2 见证「恢复后 (7) 拒绝 / (8) 成功」已锁定重建面正确）。该形态在修复前会使 Update 重放 `RedoFailed`（库不可打开）——本 rework 使其恢复面由「库打不开」变为「正确恢复」，净改善；运行期条目语义属既有 update 执行器面（本 cycle 零触碰），登记 improvement 候选。
6. **测试组织（Deviation 3）符合契约留白**（夹具形态非实质 + 本仓 per-task 测试文件先例，`wal_recovery_large_test.rs` flush_all 先例用于 catalog 持久化——DDL 无 WAL 记录的处理正确）；Deviation 4 机械格式化非实质。

**Deviation Classification**

- **Deviation 1**：PLAN-INVALID（非阻塞）——Plan 行为变化表对「简单 PK 等值 + 非 Int 字面量」形态的路由断言错误；Act 实测证伪、见证按契约原文形态落地、Excluded scope 的 planner 面正确不处理并登记 Remaining Issues #1。
- **Deviation 2**：ACT-DEVIATION（非阻塞）——R2 见证构造形态适配 SQL 可达性约束，判定性质与契约一致。
- **Deviation 3/4**：非实质（契约留白内的测试文件组织 / 机械格式化）。

**Acceptance Gaps**

None——R-import S3/S4 关闭（见证断言零修改 GREEN）；无键行崩溃恢复场景 GREEN（RED→GREEN 见证齐全）；R-dump-restore S1 全形状 GREEN；可键控行为保持守卫 GREEN（DuplicateKey 拒绝、恢复后点查/判重/新键插入）；T9 全量门 704/0/2 + clippy/fmt/validate 全 0/PASS。

**Convergence**

reduced——父 Cycle 唯一 gap（R-import S3/S4）完全关闭，无剩余、无扩大；本 Cycle 无新增 gap。

**Evidence**

- `cargo test --all`（Plan 独立复跑 2 次）→ `passed=704 failed=0 ignored=2`、`FAILED` 0、`^error` 0；`cargo clippy --all-targets -- -D warnings` → exit 0；`cargo fmt --check` → 干净；`openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` → valid
- 代码核实：`git diff src/executor/insert.rs`（两处条件化）；`git diff src/wal/recovery.rs`（双桶/三追加点/推导回退/非 deindexed 保留/未触碰段不在 diff）；`tests/keyless_row_test.rs` 4 测试全文；`tests/cli_test.rs` 见证正文逐字比对 + 全形状往返用例；`src/parser/planner/query.rs:430-475`（has_pk_equality 路由）；`src/executor/update.rs:70/130-133`（索引定位与旧键无条件 update）

**Follow-up Decision**

接受：Acceptance 全部满足且无阻塞项。**Iteration 001 完成；本 Iteration 为 change 最后一个 Iteration（T9 change 级终门已随本 Cycle 通过）→ Next Iteration: None，change 达到可收尾状态**（最终 Review Result: accepted，可交 `openspec-docs-maintainer` 正常收尾：docs sync、spec 应用含 `cli-noninteractive-shell` delta 与 `wal-recovery-replay-integrity` MODIFIED delta、change 归档）。

Improvement 候选（收尾时随登记，均非阻塞）：① planner「简单 PK 等值 + 非 Int 字面量」路由 Filter(Scan) 对无键行不可达（Deviation 1 / Remaining Issues #1）；② UPDATE 键位为无键值后运行期旧键条目指向无键版本（本 Review Finding 5）；③ GC 无键链盲区（Plan Context Risks 既登）；④ 多代表名引号膨胀与负数字面量 INSERT 限制（父 Cycle Remaining Issues #2/#3）。

**Iteration Plan Update**

None（Map 不变）

**Next Cycle**

None（Iteration 001 完成于本 Cycle accepted）

**Next Iteration**

None（无剩余 Iteration；change 可收尾）
