# Iteration 002 / Cycle 000：缺陷收口（附加短名限定引用 + 恢复重放收敛）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- 所属 Iteration：002-defect-closeout（change tasks 3.1–3.2，T7、T8；2026-10-02 用户指令追加，范围选择「两个都修」）。
- Change tasks：T7、T8。
- Acceptance gaps：None（新范围，非返工）。
- Repair items：None。
- Inherited scope：R1–R8 已交付并 accepted（Iteration 000/001）；本 Iteration 新增 R9/R10（proposal 2026-10-02 追加，design D6/D7）。
- Excluded scope：运行期页分配行为与磁盘格式变化、GC 激活、UPDATE/DELETE 语法扩展、三段以上命名空间、性能优化/bench、归档与全局状态维护。

**Objective**

两项实施期发现的缺陷收口：① 两段附加 FROM 项的隐式短名（`SELECT u.tag FROM t JOIN o.u …`）全子句面计划期可达；② 「语句部分写入回滚 → 重插 → 崩溃」序列的 WAL 恢复不再 RedoFailed 拒绝打开，重放按 WAL logical_id 强制收敛且既有恢复矩阵零回归。完成后按 M19 分批执行收尾轮全量。

**Investigation Facts**

- Current Baseline：2026-10-02，master HEAD `4e1944f`，工作区含 MS28/MS29 与本 change T1–T6 未提交修改（诊断现场）。Iteration 000/001 accepted；全量 1726 passed / 0 failed / 2 ignored（Iteration 001 T6 M19 分批，Act 执行、Review 采信）。
- T7 缺口（Iteration 001 Issue 候选，纯 SELECT 同面）：`ATTACH 'o' AS o; SELECT u.tag FROM t JOIN o.u ON t.id = u.id` 计划期 `Table not found: u`（exit 3）；显式别名（`JOIN o.u x`）与全限定（`o.u.tag`）正常；`tests/insert_select_test.rs` 注释留证、Act 实跑对照。
- T7 机制：FROM 基表臂与 JOIN 右臂仅在**显式**别名时调用 `register_table_alias`（`src/parser/planner/query.rs`，原 :371/:489，实施以符号定位）；`PlanBuilder::resolve_table_qualifier`（`src/parser/planner/mod.rs:292`）单点归一化——别名命中→真实表键、未命中→原样，未命中限定符 `TableNotFound`（42P01）。两段附加 FROM 项的 flat 名（`"o.u"`）由 pipeline `register_table` 注册 schema 并 `mark_attached_table`——PlanBuilder 持有附加标记集合，可作登记条件；字面含点主库表名不带该标记。
- T8 缺口（Iteration 000 Issue 候选，单库零路由复现）：建表 `c(id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE)` + checkpoint → INSERT(1,10,100) → UPSERT … DO UPDATE → rekey UPDATE → INSERT/DELETE → 多行 INSERT 前行已写、后行唯一冲突失败（abort）→ 重插 (2,11,101) → WAL shutdown + drop 不 close → reopen 报 `WalError("WAL redo failed: redo_tuple_at_row_id for row_id RowId { page_id: 2, slot_id: 6 } failed: dense-assignment violation: WAL slot_id=6 but page assigned logical_id=5")`，open 失败；对照组（无 abort / abort 后不重插）恢复干净（Iteration 000 Act Response 探针输出）。
- T8 代码事实（本设计已核实）：`SlottedPage::add_slot`（`src/storage/page_format/slotted_page.rs:202`）按页头 `next_logical_id` 单调分配、无回收路径；`delete_slot_by_logical_id` 仅 catalog/GC 消费（`gc_table` 无运行期调用者）；abort A/B 趟（`src/transaction/manager.rs:282` 起）仅索引回退/还原 + 原位 `mark_aborted` 中性化，不删槽、不回退计数器；WAL 数据记录由执行器执行期写入（`WALBuffer::append` 纯追加，abort 仅追加 `AbortTxn`，无记录剔除）；`redo_tuple_at_row_id`（`src/wal/recovery.rs:134`）幂等命中跳过、未命中 `add_slot` + 稠密落位校验（新分配 id 必须 == WAL slot_id，否则 `RedoFailed`）。
- T8 根因类：稠密校验假设「重放分配序列 == 运行期分配序列」，而重放的幂等跳过/落位与运行期逻辑 id 消耗存在失配形态——失配后后续记录按重放序列分配的 id 与 WAL row_id 错位，硬失败拒绝打开。安全收敛依据：运行期不变量「页内 logical_id 单调分配且不回收 → (page, logical_id) 唯一对应一个版本」+ WAL 记录 CRC 保护。精确失配记录由 Act 以既有 `WalReader` 诊断固定（T8 诊断见证）。

**Current-State Evidence / Code and Critical Path**

| 文件与符号 | 实际入口、数据与责任 |
|---|---|
| `src/parser/planner/query.rs` FROM 基表臂 / JOIN 右臂 | 基表别名登记点（显式别名 → `register_table_alias(alias, table_name)`）；无别名时 qualifier = flat 名。T7 在两臂「无显式别名 && flat 名带附加标记 && 两段」时以尾段登记隐式关联名 |
| `src/parser/planner/mod.rs:283/292` | `register_table_alias`（HashMap 插入，重复后注册覆盖——显式别名既有语义）/ `resolve_table_qualifier`（单点归一化）。隐式关联名复用同通道，全子句面消费点（JOIN 投影/ON 两臂/WHERE/GROUP BY·聚合参数/ORDER BY）自然生效，零新解析通道 |
| pipeline `register_table` → `mark_attached_table` | 附加判定的唯一权威（ObjectName 分段 + AttachRoute 命中）；PlanBuilder 附加标记集合是 T7 登记条件的数据源（字面含点名不带标记） |
| `src/storage/page_format/slotted_page.rs:202 add_slot` | 单调分配（`next_logical_id += 1`）；`delete_slot_by_logical_id` 存在但仅 GC/catalog 消费。T8 新增 `add_slot_with_logical_id(data, id)`：按给定 id 写 slot + `next_logical_id = max(next, id+1)`，仅恢复面消费 |
| `src/wal/recovery.rs:134 redo_tuple_at_row_id` | redo 落位原语：init 未初始化页 → 幂等命中（`get_slot_by_logical_id`）跳过 → `add_slot` + dense 校验。T8：未命中改走 `add_slot_with_logical_id(…, row_id.slot_id)`，删除 dense 硬失败；「无法落位显式报错」收窄为 `add_slot` 物理失败（页满）与 IO 错误 |
| `src/transaction/manager.rs:282 abort_cleanup_versions` | A 趟索引回退/还原 + 原位中性化；B 趟墓碑还原——T8 修复不触碰本文件 |
| `src/wal/buffer.rs:62 append` | 纯追加（LSN 单调），abort 无记录剔除——诊断时按 LSN 序对照记录 row_id 与重放落位即可固定失配点 |
| 恢复测试面 | `tests/wal_recovery_large_test.rs`、`tests/recovery_test.rs`（如存在）等既有矩阵；`src/wal/recovery.rs` 文件内单测；新 `tests/recovery_replay_convergence_test.rs`（T8）；`tests/cross_db_query_test.rs`（T7 e2e） |

测试入口（RED 基线）：T7——`SELECT u.tag FROM t JOIN o.u ON t.id = u.id` 当前 `Table not found: u`；T8——探针序列当前 open 报 dense-assignment violation。两者均为可低成本复现的既有失败，RED 见证可直接建立。

**Implementation Guidance**

T7、T8 相互独立，可任意顺序（建议 T7 先行——体量小、先行闭环）。T7：在两注册点以「无显式别名 && 附加标记 && 两段」为条件登记尾段隐式关联名；不改 `resolve_table_qualifier`、不改表头/计划形状；planner 单测固定登记条件（附加才登记、显式别名优先、字面含点不登记）。T8：先以新恢复测试建立 RED（探针序列 open 失败），用既有 `WalReader` 固定失配记录（诊断见证写 Act Response）；随后实现 `add_slot_with_logical_id` 并替换 redo 落位臂、删除 dense 硬失败；单测覆盖 gap 落位推进计数器；既有恢复矩阵定向回归。两 task 完成后按 M19 分批执行收尾轮全量。私有 helper 命名、测试内夹具复用形态为非实质选择。

**Task Contracts**

### T7：两段附加 FROM 项的隐式短名限定引用可达

- Requirement/Scenario：R9 / S16（proposal）；delta spec `sql-table-alias-resolution` MODIFIED「FROM 表别名注册」新增两场景。
- Depends on：None。
- Targets：`src/parser/planner/query.rs` FROM 基表臂与 JOIN 右臂别名登记点；`tests/cross_db_query_test.rs`（e2e）；planner 单测（mod.rs 或 query.rs 文件内测试模块）。
- Current behavior：两段附加 FROM 项无显式别名时不登记任何关联名；短名限定列 `u.tag` 归一化回落扁平名 `u` → `TableNotFound`。
- Required behavior：登记条件「无显式别名 && flat 名在附加标记集合 && 名分两段」满足时，以尾段调用既有 `register_table_alias(尾段, flat名)`——短名限定列经 `resolve_table_qualifier` 归一化为 `o.u` 后沿既有通道解析，SELECT/ON/WHERE/GROUP BY/聚合参数/ORDER BY 全子句面可达；显式别名时行为逐字节不变（显式登记优先，隐式不登记）；主库表与字面含点表名（无附加标记）不登记；表头、`OutputColumn` 真实表名、计划形状不变量沿用 R1。
- Preserve：`resolve_table_qualifier` 单点归一化语义零变化；三段限定仅 attach 复合键可达不变；未知限定符 `TableNotFound` 42P01 面不变；显式别名/全限定/纯主库/字面含点全部既有用例零回归。
- Forbidden：新增第二解析序或回退尝试；改写 `resolve_table_qualifier`；为歧义关联名新增拒绝面（重复关联名沿既有别名覆盖语义，与显式别名一致）；扩大命名空间（三段、schema 层级）。
- Test witness：先建 RED——`cross_db_query_test` 新用例当前 `Table not found: u` 失败：JOIN 短名（SELECT/ON/WHERE 组合）、单表短名（`SELECT u.tag FROM o.u`）、显式别名优先（`JOIN o.u x` 后 `x.tag` 可达且 `u.tag` 仍按既有面）、字面含点不误登记（`FROM "o.u"` 的短名形态仍点名）。planner 单测固定登记条件三形态。
- GREEN condition：上述用例全绿；`cargo test --test cross_db_query_test --test table_alias_resolution_test -- --test-threads=1` 0 failed（后者既有 16 例零回归）。
- Verification：`cargo test --test cross_db_query_test --test table_alias_resolution_test --test planner_test -- --test-threads=1`；`cargo clippy --lib --bin rtsql --tests -- -D warnings` 无新告警。
- Stop when：登记条件需扩大到非附加两段名、或短名解析需要改别名遮蔽/表头语义——返回 Plan。

### T8：崩溃恢复 redo 按 WAL logical_id 强制收敛

- Requirement/Scenario：R10 / S17（proposal）；delta spec `wal-recovery-replay-integrity` MODIFIED「重放位置寻址且幂等」新增「中止后重插的崩溃恢复收敛」场景。
- Depends on：None。
- Targets：`src/storage/page_format/slotted_page.rs`（新 `add_slot_with_logical_id`）；`src/wal/recovery.rs::redo_tuple_at_row_id`（落位臂替换 + dense 硬失败删除）；新 `tests/recovery_replay_convergence_test.rs`；slotted_page 文件内单测。
- Current behavior：redo 未命中时 `add_slot` 自然分配 + 稠密校验（新分配 id 必须 == WAL slot_id）；探针序列（abort 消耗逻辑 id 后重插 + 崩溃）重放失配 → `RedoFailed` → open 失败。
- Required behavior：redo 落位以 WAL 记录的 logical_id 为准——幂等命中跳过；未命中按 `row_id.slot_id` 强制落位（`next_logical_id = max(next, id+1)`）；dense-assignment 硬失败删除；「显式报错」收窄为物理落位失败（页满 `No enough space in page`、IO 错误），K05 语义保持。运行期 `add_slot` 及全部执行器/事务路径零变化；新原语仅恢复面消费。诊断见证先行：以既有 `WalReader` 固定探针 WAL 的记录序列与失配点（写 Act Response）。
- Preserve：重放幂等重跑一致；恢复后行/PK/唯一/二级索引/版本链与运行期提交终态一致；既有恢复矩阵（驱逐规模、索引重建、无键行回退、墓碑重放、mark_uncommitted_aborted）零回归；磁盘格式与运行期行为零变化。
- Forbidden：改运行期页分配/abort/GC 行为；为诊断新增产品开关、日志协议或判定层；放宽 CRC 或幂等语义；触碰 checkpoint/位点逻辑。
- Test witness：先建 RED——`tests/recovery_replay_convergence_test.rs::aborted_insert_reinsert_crash_recovery_succeeds`（探针序列：表含 NOT NULL/UNIQUE + 二级索引，checkpoint 后提交含 UPSERT/rekey 的写入，多行语句前行已写后行唯一冲突回滚，重插同键提交，WAL shutdown + drop 不 close，open 当前 dense violation 失败）；两对照用例（无 abort / abort 后不重插，当前 GREEN，修复后保持）；重跑幂等用例。slotted_page 单测：gap 落位（跳号 id）推进计数器、后续自然分配不冲突。
- GREEN condition：探针序列 open 成功且行集/PK 点查/唯一占用/二级索引查询与提交终态一致；对照与幂等保持；`cargo test --test wal_recovery_large_test --test recovery_test --test checkpoint_redo_reduction_test -- --test-threads=1`（既有目标按仓库实际名）0 failed。
- Verification：`cargo test --test recovery_replay_convergence_test -- --test-threads=1` + 既有恢复套件定向；`cargo fmt --check`；收尾轮 M19 全量分批（本 Iteration 为 change 最后实施轮，T8 完成后执行）。
- Stop when：诊断发现违反「页内 logical_id 唯一且不回收」不变量的运行期路径（即强制收敛可能掩盖真实不一致的形态）、或落位需要页格式变化——返回 Plan。

**Invariants**

- T7 仅在既有别名机制上登记，不新增解析通道；显式别名优先；表头/计划形状不变量沿用 `sql-table-alias-resolution` R1。
- T8 运行期行为与磁盘格式零变化；恢复面收敛以「(page, logical_id) 唯一对应一个版本 + WAL CRC」为安全性依据。
- 复用现有实现优先；不加依赖、不重构公共 API、不清理用户无关修改。
- 每项任务先建立 RED 见证再修改；完成后先 spec review 再 code review，Gate 3 结果写 Act Response。

**Non-goals**

GC 激活或页回收策略变更、磁盘格式、UPDATE/DELETE 语法扩展、三段以上命名空间、同语句关联名歧义拒绝、性能优化/bench、REPL/serve/DETACH、跨文件原子性、持久化 Evidence 与工程经验产物、归档或提交。

**Acceptance**

| 条件 | 任务/测试与可观察量 |
|---|---|
| C1：短名限定引用全子句面可达 | T7 cross_db_query_test 用例（JOIN/单表/显式别名优先/字面含点不登记）+ planner 单测 GREEN，既有别名面零回归 |
| C2：中止后重插崩溃恢复收敛 | T8 探针序列 RED→GREEN + 两对照 + 重跑幂等 + 既有恢复套件定向 0 failed |
| C3：收尾轮全量 | M19 分批（`--lib`/`--bins` + 108 集成目标字典序、每批 `--test-threads=1`）全部 0 failed；fmt/clippy clean |

Iteration 002 RTM：R9→T7、R10→T8（change tasks.md RTM 新增两行）。全部 Covered（计划映射完整；见证待本轮实施）。

**Verification**

按 Task Contract 直接目标 → 受影响边界 → fmt/clippy → M19 收尾轮全量顺序；判定仅依据 Rust 测试框架原生输出与退出码；无附加判定层。新增用例独立 TempDir/`tempdir()`；不重复运行已通过且覆盖未变的验证。诊断见证（T8 失配记录固定）以 Act Response 记录，不建 Evidence 目录。

**Gate 2 Readiness**

PASS（2026-10-02 用户批准本 Iteration 计划，原话「批准」）：技术调查（T7 注册点/标记条件、T8 落位原语/abort/WAL 追加路径）、设计（D6/D7 含安全收敛依据）、任务契约、RTM 与验证契约已闭合；非实质未知项 None（T8 精确失配记录为 Act 诊断见证，非阻塞未知——修复方向对整个失配类收敛）；无 Missing/Simplified。Gate 2 技术面经 Plan 独立审计复核（T7 两登记点仅显式别名 + 附加标记两调用点；T8 dense 校验/单调分配/abort 原位中性化/槽位删除消费者穷尽——`delete_tuple_from_data_page` 仅 `gc_table` 消费且无运行期调用者，安全收敛前提成立）；BLOCKED/WAIVED：None。

**Persisted Evidence**

- Mode: none。
- 命令、每项 ≤20 行决定性输出、退出码、修改符号与 Gate 结论写 Act Response；探针与测试均可低成本重跑。

**Risks and Notes**

- T8 诊断若揭示「页内 id 唯一且不回收」不变量被运行期某路径违反，强制收敛不安全——按 Stop when 返回 Plan（当前代码调查未发现此类路径：abort 原位中性化、GC 无运行期调用者、add_slot 单调）。
- T7 同语句重复隐式关联名沿既有别名覆盖语义（与显式别名一致）；标准 SQL 的歧义拒绝不在本期（Non-goals）。
- 回退二进制后 R10 场景回到既有 RedoFailed 形态、R9 场景回到 `TableNotFound`（proposal Impact 已记录）。
- 收尾轮 M19 全量在 T7/T8 均完成后执行一次；两 task 之间不重复全量。
- 本轮结束停在 Act Response，待 Plan 独立完整 diff Review；不创建后继 Cycle/Iteration、不维护全局状态、不归档。

## Act Response

- Status: reported

### 实际改动

**T7（R9）——两段附加 FROM 项的隐式短名限定引用可达**

- `src/parser/planner/mod.rs`：`PlanBuilder` 新增两方法——`register_implicit_attached_short_name(flat_name)`（登记条件：flat 名 ∈ `attached_tables` ∧ 恰分两段 → `register_table_alias(尾段, flat名)`；字面含点主库表名不带附加标记不登记）与 `qualifier_matches_declared(written, declared)`（原文相等 ∨ 书写限定符经 `resolve_table_qualifier` 归一化命中声明限定符）。`resolve_table_qualifier` 本体零变化。
- `src/parser/planner/query.rs`：FROM 基表臂（无显式别名分支）与 JOIN 右臂（`match table_factor_alias` 的 `None` 臂）调用隐式登记；三处既有匹配点接归一化——JOIN 输出过滤的严格校验 hits 计数与选择谓词（原 `eq_ignore_ascii_case` 直比）、`resolve_join_projection_indices`（自由函数加 `builder: &PlanBuilder` 首参，两处 match 消费 helper）、`resolve_layout_index`（同款加 `builder` 首参，`alias_matches` 消费 helper；4 个调用点均在 `&self` 方法内传 `self`）。
- `src/parser/planner/expression.rs`：NLJ 组合行布局臂的书写限定符匹配（原 `table == table_ref`）接 `qualifier_matches_declared`。
- 测试：`tests/cross_db_query_test.rs` +4 例（`attached_join_short_name_reaches_all_clauses`〔SELECT/ON/WHERE/ORDER BY + GROUP BY/COUNT(*) 两语句〕、`attached_single_table_short_name_qualifier`、`attached_short_name_explicit_alias_priority`〔显式别名可达 + 短名仍 `TableNotFound: u` 守护〕、`attached_short_name_literal_dotted_not_registered`）；`src/parser/planner/mod.rs` 测试模块 +4 单测（隐式登记、附加标记必要、显式别名并存不覆盖、匹配 helper 两分支）。

**T8（R10）——崩溃恢复 redo 按 WAL logical_id 强制收敛**

- `src/storage/page_format/slotted_page.rs`：`add_slot` 主体重构为私有 `add_slot_inner(data, forced: Option<u16>)`（自然分配臂逐字节等价），新增 `add_slot_with_logical_id(data, logical_id)` 恢复面专用强制落位原语（`next_logical_id = max(next, id+1)`，`saturating_add` 防溢出）；运行期 `add_slot` 及全部执行器/事务路径零变化。
- `src/wal/recovery.rs`：`redo_tuple_at_row_id` 落位臂替换——未命中时 `add_slot_with_logical_id(&slot_data, row_id.slot_id)`，**dense-assignment 硬失败删除**；显式报错收窄为物理落位失败（页满 `No enough space in page` → 既有 `RedoFailed` 包装、`get_page` IO 错误不变），K05 语义保持；幂等命中臂（2b）不变；函数 doc 步骤 4 同步改写。
- 测试：新 `tests/recovery_replay_convergence_test.rs` 3 例（`aborted_insert_reinsert_crash_recovery_succeeds`〔探针序列 + 恢复终态断言：COUNT/PK 点查×4/唯一占用×2/冲突零副作用/二级索引 `idx_c_n` 点查 + 重跑幂等〕、`committed_writes_without_abort_recover_unchanged`、`abort_without_reinsert_recovers_cleanly`〔含中止行不复活断言〕）；`slotted_page.rs` 文件内 +2 单测（跳号落位推进计数器 + 后续自然分配不冲突、低位补位 max 语义不回退）。

### 诊断见证（T8，Plan Context 指定）

一次性诊断用例经既有 `WalReader::read_all_with_lsn` 转储探针 WAL（22 条记录，用后已删，无产品代码）：LSN 0 CHECKPOINT → tx1 INSERT slot 0 → tx2 UPDATE slot 1（UPSERT DO UPDATE）→ tx3 UPDATE slot 2（rekey）→ tx4 INSERT slot 3 → tx5 DELETE slot 3（墓碑经 `write_tuple_to_data_page` 自然追加，无 dense 面）→ **tx6 INSERT slot 5 → ABORT_TXN tx=6**（LSN 692/757）→ tx7 INSERT slot 6（LSN 807）。失配点固定：redo 只重放已提交事务（`recovery.rs` Step 2 门 `committed_tx_ids.contains(&tx_id)`），tx6 的 slot 5 记录不落位而其运行期 id 已消耗，tx7 记录携带 slot 6 时重放自然分配仅到 4 → 分配 5 → dense violation——与 Plan Context 根因类判定一致，强制收敛对该失配类整体安全（(page, logical_id) 唯一对应一个版本 + 单调不回收）。对照组两面无后续记录引用被跳过 id，故恢复干净。

### 与计划的偏差

1. **T7 接线面超出 Targets 清单（非实质）**：Task Contract Targets 只列两登记点；「SELECT/ON/WHERE/GROUP BY/聚合参数/ORDER BY 全子句面可达」的 Required behavior 要求 JOIN 输出过滤、`resolve_join_projection_indices`、NLJ 布局臂与 `resolve_layout_index` 四处既有匹配点一并消费别名归一（此前按书写限定符原文直比声明限定符）。机制恰为计划指定的既有通道（`resolve_table_qualifier` 归一化），无新解析序、无回退尝试；短名与全限定产生逐字节相同的 `ColumnRef`/输出列命中。
2. **T7 守护用例断言文案校准**：`TableNotFound` 的实际错误面为 `Plan error: Table not found: u`（exit 3），非 `'u' not found`——首版 needle 修正后 RED 面干净（恰 2 缺口用例失败）。
3. **T8 探针值按 Plan Context 记载序列具化**（UPSERT `SET n = 12`、rekey `id 1→5`、多行 `(2,11,101),(3,12,100)`）——RED 错误签名与 Iteration 000 Act Response 记载逐字一致（`RowId { page_id: 2, slot_id: 6 } … WAL slot_id=6 but page assigned logical_id=5`）。
4. `cargo fmt` 归一 + `clippy --fix` 对新测试文件一处 `bool_comparison` 自动修正（`!matches!(…) == b` → `matches!(…) != b`）。

### 自检发现与修复

- GREEN 期间发现 T8 恢复终态断言的行渲染比较写法缺陷（`{r:?}` 产出 `Number(..)` 形态导致假失败）——恢复本身已收敛，改为 `serde_json::json!` 值直比后全绿；属测试代码问题，非产品行为。
- 遗留 Minor 问题：None。

### 验证（判定依据 Rust 测试框架原生输出与退出码）

| 验证 | 命令（均 `--test-threads=1`） | 决定性输出 | 退出码 |
|---|---|---|---|
| T7 RED | `cargo test --test cross_db_query_test` | `2 failed; 19 passed`（恰两缺口用例，`Table not found: u`） | 101 |
| T7 GREEN | `cargo test --test cross_db_query_test` | `ok. 21 passed; 0 failed` | 0 |
| T7 单测/回归 | planner 单测过滤 + `--test table_alias_resolution_test --test planner_test` | `10 passed` / `16 passed` / `96 passed`，全 0 failed | 0 |
| T8 RED | `cargo test --test recovery_replay_convergence_test` | 探针 `unwrap` on `Err: WalError("WAL redo failed: … dense-assignment violation: WAL slot_id=6 but page assigned logical_id=5")`；两对照 `ok` | 101 |
| T8 GREEN | 同上 | `ok. 3 passed; 0 failed` | 0 |
| T8 原语单测 | `cargo test --lib storage::page_format::slotted_page` | `15 passed; 0 failed`（含 2 新） | 0 |
| 既有恢复套件 | `--test wal_recovery_large_test --test recovery_test --test checkpoint_redo_reduction_test --test recovery_e2e_test` | `7 / 3 / 9 / 6 passed`，全 0 failed | 0 |
| fmt / clippy | `cargo fmt --check`；`cargo clippy --lib --bin rtsql --tests -- -D warnings` | `FMT CLEAN`；无输出（clippy 通过） | 0 / 0 |
| M19 收尾轮全量 | 109 集成目标按字典序 11 批 + `--lib` + `--bins`，每批独立串行 | 每批 `0 failed`；合计 **1739 passed / 0 failed / 2 ignored**（`cli_test` 117+2i；`--lib` 370；`--bins` 0 例），较 Iteration 001 基线 1726 净增 13（T7 e2e 4 + planner 单测 4 + 恢复 3 + slotted_page 2） | 0（每批） |

### Persisted Evidence

None required（Mode: none——探针与测试均可低成本重跑，本 Response 载明决定性输出；未创建 `evidence/` 目录）。

### Experience Candidates

None（T7/T8 即本 change 实施期 Issue 候选的收口，随 change 闭环不落账；无 change 范围外新缺陷发现；M19 分批配方已有 tasks.md 既有记载，非新操作路径）。

### 未解决问题

None。

## Plan Review

- Review Result: accepted

**Findings**

独立审读范围：T7/T8 全部变更面 diff——`src/parser/planner/{mod,query,expression}.rs`、`src/storage/page_format/slotted_page.rs`、`src/wal/recovery.rs`、`tests/cross_db_query_test.rs` 增量、`tests/recovery_replay_convergence_test.rs` 新文件——与 Act Response 逐项声明核对。结论：Act 声明与代码一致，Acceptance C1/C2/C3 全部满足。

1. **偏差 1 分类——Plan 遗漏（非实质，不阻塞）**：Task Contract Targets 只列两登记点，而 Required behavior「全子句面可达」客观要求 JOIN 输出过滤两处、`resolve_join_projection_indices`（两臂）、`resolve_layout_index`（`builder` 首参 + 4 调用点）与 `expression.rs` NLJ 布局臂一并消费限定符匹配——否则书写短名在 JOIN 严格校验路径仍被 `eq_ignore_ascii_case` 直比拒绝。接线全部经计划指定的既有通道（`qualifier_matches_declared` → `resolve_table_qualifier` 归一化），无新解析序、无回退尝试；属 Plan 侧 Targets 清单不完整，不构成 Act 偏离。
2. **Minor finding 1（非阻塞，记录不返工）——显式别名 × 裸名同表混用的角落语义移位**：`qualifier_matches_declared` 第二分支使**已注册别名**的书写限定符可二次命中「声明键 = 该别名真实表名」的另一 FROM 臂（`resolve_table_qualifier` 对未注册限定符返回 None，纯裸名/真实表名不受影响）。形态 `FROM people t JOIN people …`（两臂含同名列）下 `t.col` 由既有「唯一命中解析到 t 臂」变为双命中 `AmbiguousColumn`。方向保守（报错而非静默错数据）；该形态无规格场景、无既有测试见证，既有别名面（table_alias_resolution 16 例 + 全量 1739）零回归，契约 Preserve 枚举四形态（显式别名/全限定/纯主库/字面含点）均零变化。留作后续 change 可选收口，不影响本期 Acceptance。
3. **Minor finding 2（非阻塞，记录不返工）——测试注释场景编号不同轴**：`recovery_replay_convergence_test.rs` 注释「R10/S1」与 `cross_db_query_test.rs` T7 注释「R9/S1–S4」为测试文件局部编号，与 proposal 场景编号（S16/S17）不同轴，阅读时以注释正文为准。纯注释问题。
4. 偏差 2–4（`TableNotFound` 文案 needle 校准、T8 探针值按 Plan Context 具化、fmt/clippy 对新测试一处自动修正）核实为非实质，与 Act 声明一致。
5. **契约边界核查（全部通过）**：`resolve_table_qualifier`/`register_table_alias` 本体零改动（diff 中相关字符串均为新增注释/测试引用）；`add_slot` 自然分配臂逐字节等价（`forced=None` 路径，全部运行期调用者零变化）；`add_slot_with_logical_id` 全仓唯一消费点 `recovery.rs:176`（恢复面专用）；dense 硬失败无残留；恢复幂等命中臂（2b）先于强制落位，页内 `(page, logical_id)` 唯一性保持；运行期页分配/abort/GC/磁盘格式零触碰；无身份型证据工程、无判定层、无产品开关。T7 守护用例（显式别名优先 + 字面含点不误登记）与 T8 双对照用例均与契约 Test witness 一致。
6. 无 change 范围外实质缺陷发现（Issue 候选：None）。

**Evidence**

- 代码核对：`git diff` 逐文件独立审读（Findings 第 1、5 条）；T7 4 e2e + 4 单测、T8 3 e2e + 2 单测与 Act 清单一一对应。
- 验证采信：Act Response 验证表全部结论（T7 RED `2 failed` → GREEN `21 passed`；T8 RED dense violation → GREEN `3 passed`；既有恢复套件 `7/3/9/6 passed`；fmt/clippy clean；M19 收尾轮 11 批 + `--lib` + `--bins` 合计 **1739 passed / 0 failed / 2 ignored**）——只读核查确认无 src/tests 文件 mtime 晚于本 Cycle 文件（验证结论产生后覆盖范围未变化），按公共规则 › 验证 直接采信，不重跑；Plan 独立检查为完整 diff 审读与声明核对，不以 Act Self-Review 代替。
- Persisted Evidence：Mode none 符合白名单，未创建目录不构成问题。

**Follow-up Decision**

Acceptance C1（短名全子句面可达 + 既有别名面零回归）、C2（探针 RED→GREEN + 双对照 + 重跑幂等 + 既有恢复套件零回归）、C3（M19 收尾轮全量每批 0 failed + fmt/clippy clean）全部满足；两项 Minor finding 均非既有 Acceptance 的必要条件，不需要当前 Cycle 修复，不构成 rework/replan。故接受本 Iteration；Iteration Map 无后继 Iteration，change 实施面完成，待用户指令收尾（增量规格 6 域合并与 change 归档由 openspec-docs-maintainer 承担）。

**Next Cycle**

None

**Next Iteration**

None
