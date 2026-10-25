# Iteration 001 / Cycle 001: 规格收窄与边界回归（replan）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: 2.6
- Acceptance gaps: None（2.1–2.5 的 Acceptance 全部满足；本 Cycle 不返工它们）
- Repair items: None
- Inherited scope: Iteration 000 的稳定结果（被放弃的隐式语句不提交、隐式事务在下一条写语句之前回收、未提交数据不随 checkpoint 落盘）与 Iteration 001 Cycle 000 的四个新用例（2.1–2.4）；`tests/statement_cancellation_test.rs` 现有的 8 个用例与其夹具（`setup` / `setup_with_isolation` / `anchor_tx` / `abandon_statement` / `await_anchor_tx` / `expect_affected` / `expect_rows` / `expect_affected_in_tx` / `expect_rows_in_tx`）；2.5 的 M19 分批串行全量结论 **2163 / 0 / 2**（13 批，逐批退出码 0、failed 计数 0，覆盖对账 missing 0 / extra 0 / duplicate 0）；父 Cycle `000-initial.md` 的 Act Response 与其 Plan Review 结论；change `tasks.md` 的 Iteration Map（Iteration 000 已 `accepted`）。
- Excluded scope: **索引写侧原子性与键释放的修复**（Issue 候选 E1、E2，落在 `src/storage/btree/` 与索引写入路径，不在本 change 修）；被放弃的 ALTER TABLE DROP COLUMN 的数据面形态（Issue 候选 C1）；协作式取消上下文与 deadline 公共 API（I096）；WAL 格式与 checkpoint 九步；CLI 信号停机行为；`openspec/specs/` 的合并（docs-maintainer 的收尾职责）；MS35 / MS36。

**Objective**

`statement-cancellation-outcome` 的规格只描述本 change 已验收的行为：取消后**行**不可见、隐式事务被回收（活跃集清空）、未提交数据不随 checkpoint 落盘；「键的索引条目释放」不再作无条件断言，改为已知边界（由存储域的索引写侧契约承载）。同时把收窄后场景 1 保留的「被放弃语句写入的行不可见」在**同一会话**形态上落成仓库里的直接断言，并按「可承载」格式报告 E1 / E2 / C1 三个 Issue 候选。

**Background**

父 Cycle `000-initial.md`（Act Response `reported`）实施完成 2.1–2.5，其 Plan Review 判 **`replan-required`**，理由（完整判定见父 Cycle 的 Plan Review）：

- 2.1–2.5 的 Acceptance 逐条满足，偏差（2.1 无 RED、RC 夹具、换目标表、偏移键、缩窗口）经裁定均不削弱断言。
- 但规格 delta 无条件断言「索引条目被还原」与「同一个键可以再次插入成功」。Review 独立复现：在显式事务内取消一条多行 `INSERT … SELECT` 后回滚，被取消语句中**索引写入被中断的那一行**的键仍占用主键索引，再插同键报 `DuplicateKey`——该保证只对被取消语句的**首个键**成立。规格即将合并进 `openspec/specs/`，按原样合并会把未验收的行为写进权威规格库。
- 修复点落在存储/索引层（`IndexManager::insert` 的两段写与 `abort_cleanup_versions` 的分桶），超出本 change 的 Impact（`src/database.rs` / `src/pipeline.rs` / `src/executor/alter_table.rs` / `src/transaction/manager.rs`）与 design D1–D6；按用户本轮约定「实施中发现的所有 issue 候选都直接寻找好的承载方式流程进行修复」，缺陷由 Issue 承载、由存储域的后续 change 修复，本 change 不吸收。

按 `references/iteration-planning.md` 的 Replan Cycle 规则：已更新 change 的 `tasks.md`、`specs/statement-cancellation-outcome/spec.md`、`design.md` 与未完成的 Iteration 001 Plan（收窄 + 边界注记 + 新增任务 2.6），本 Cycle 以修订后的 Acceptance 为边界重新执行。已交接的 Cycle 文件不变。

**Investigation Facts**

- Starting point: revision `0a08b14`（MS33 收尾入库）+ 工作区既有改动（`src/database.rs`、`src/pipeline.rs`、`src/executor/alter_table.rs`、`src/transaction/manager.rs`、`tests/statement_cancellation_test.rs`；`.agents/docs/tasks.md` 的未提交修改属全局文档，不属本 Cycle）。**沿用未失效的既有结论**：2.5 的 M19 分批全量 2163 / 0 / 2（来源：父 Cycle Act Response 的分批明细表）；该结论产生后被覆盖的代码未发生变化（`git status --short` 与父 Cycle 实施完成时一致，`tests/statement_cancellation_test.rs` 仅在本 Cycle 被追加断言），因此其余 12 批的结论直接沿用，**本 Cycle 不重跑全量**。
- Current-State Evidence:
  - **规格 delta 已由本 replan 收窄**（`openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/specs/statement-cancellation-outcome/spec.md`），共五处改动：
    1. requirement「提交前被取消的语句不提交」的枚举去掉「索引条目被还原」——保留「未提交版本被墓碑化、事务移出活跃事务集合、回收时机、页锁随 future 释放」。
    2. 场景「隐式语句在执行中被放弃后不提交」的 THEN 去掉「同一个键可以再次插入成功（索引条目已还原）」——保留「被放弃语句写入的行不可见；下一条写语句开始之前该语句的隐式事务已回滚，`TransactionManager::active_transactions()` 为空」。
    3. 场景「被放弃语句的未提交数据不随 checkpoint 落盘」的 THEN 去掉「同一个键可以再次插入成功」——保留「被放弃语句写入的行不存在」。
    4. requirement「显式事务继续还是回滚由调用方决定」的正文把「该事务内全部修改……被撤销」改为「该事务内的**行修改**……被撤销」，并指向本 requirement 的已知边界。
    5. 新增**已知边界**注记（放在「提交前被取消的语句不提交」的三个场景之后）：取消若落在索引写入中途，被取消语句的**行**不可见（本 requirement 的验收面），但它的**键**仍可能占用主键索引；键的释放不属本域的保证面，由存储域的索引写侧契约承载，作为 Issue 候选另行处置。
  - **收窄后每条规格断言的已有证据**（Act 无需重跑既有验证，只需在 Act Response 给出映射表）：
    | 规格断言 | 证据 |
    |---|---|
    | 取消触发集合 = 语句 future 被放弃（丢弃 / 超时同一类事件） | 用例 1.1（丢弃）、2.3（超时） |
    | 被放弃语句写入的行不可见 | 用例 1.3、2.2、2.4（重开后 / 显式事务回滚后形态）+ **同一会话形态目前无直接断言**（见下） |
    | 下一条写语句之前隐式事务已回滚、活跃集为空 | 用例 1.1 |
    | 未提交数据不随 checkpoint 落盘 | 用例 1.3、2.4 |
    | 提交已完成但结果未送达则已提交 | 用例 2.1 |
    | 显式事务仍活跃可用 / 回滚撤销 / 提交保留 | 用例 2.2（三个场景） |
    | 超时不代替回滚 | 用例 2.3 |
    | 取消之后会话与后台 I/O 按会话级生命周期收尾 | 用例 2.4 |
  - **唯一缺直接断言的条款**：「被放弃语句写入的行不可见」的**同一会话**形态（场景 1 的 WHEN 是「随后调用方在同一个 `Database` 句柄上执行下一条写语句」）。仓库里现有断言都在重开后的形态（1.3 / 2.4）。本 Review 的独立探针（临时文件，已删除）观测到该形态当前为真：回收完成后同一会话 `SELECT id FROM t2` 只读到后续写入的那一行（RR 与 RC 各一次，行数均为 1）→ 可直接落成断言。
  - **2.4 的现状与断言位置**：`tests/statement_cancellation_test.rs:746-799` —— `setup(&dir)` → `abandon_statement(&db, ABANDONED_SQL)` → 同会话写入 `t2` 的 9001 / 9002 与 `t3` 的 9003 → `close()` → 重开 → 断言（重开后 `t2` 两行、`t3` 一行、被放弃语句的行不存在、`check_integrity` 无 Error）。**同一会话**形态的断言位置：在 `expect_affected(&db, "INSERT INTO t3 VALUES (9003)").await;` 之后、`db.close().await.unwrap();` 之前。
  - **决定断言写法的事实**：`ABANDONED_SQL = "INSERT INTO t2 SELECT id FROM t"`，`t` 预置 `id` 1..2000 → 被放弃语句写入 `t2` 的键是 1..2000；2.4 的后续写入用 9001 / 9002 / 9003（在 1..2000 之外，避开 Issue 候选 E2 的残留键）。因此同一会话读 `SELECT id FROM t2` 的结果应恰为 9001、9002 两行，不含 1..2000 的任何键。
- Code and Critical Path: `tests/statement_cancellation_test.rs`（`expect_rows(&db, sql)` 返回 `Vec<Vec<serde_json::Value>>`；`expect_affected`；`setup(&dir)` 预置 `t` 2000 行与 `t2`）；`Database::execute_sql`（`src/database.rs`）→ `pipeline::execute_sql` → 回收挂载点（DML 臂 `src/pipeline.rs:560` 附近）在下一条隐式写语句开始之前回收被放弃的隐式事务 → `TransactionManager::abort` → `abort_by_id` → `abort_cleanup_versions`（`src/transaction/manager.rs:321`）——**行**的不可见由墓碑化保证，**键**的释放按反向映射分桶（E2 的根因）。规格与设计文档：`openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/specs/statement-cancellation-outcome/spec.md`、`.../tasks.md`、`.../design.md`。

**Implementation Guidance**

- 本 Cycle **不改产品代码**，只做「规格准确性收尾 + 一条断言 + Issue 候选报告」。规格 delta、`tasks.md`、`design.md` 的收窄已由本 replan 写入，Act 只需复核一致性并在 Act Response 给出「规格断言 → 用例」映射表。
- 断言写法：复用 `expect_rows(&db, "SELECT id FROM t2")`；断言读到的行**不含**被放弃语句写入的键（例如：行数为 2 且每行的 `id` 都大于 2000）。判定必须落在库入口返回的行集上，不得用内部状态、哈希、指纹或时间顺序。
- 该断言属**表征分支**：当前行为已成立（Review 的独立探针已观测），因此先观察 GREEN 并记录基线，**不得为制造 RED 改产品代码**。
- 用例数必须保持 **8**：新增用例会改变 2.5 的全量总数对账（2163 → 2164），代价大于收益；加断言不改变计数。
- 已知边界注记只描述「行不可见 / 键可能仍占用」，**不得**断言键一定可复用，也**不得**断言键一定被占用（那会把缺陷写成契约）；修复方案不写进规格。

**Task Definitions**

### 2.6: 规格收窄与边界回归

- Requirement/Scenario: 「提交前被取消的语句不提交」/「隐式语句在执行中被放弃后不提交」；「显式事务继续还是回滚由调用方决定」（键的释放移出本域保证面）；change 任务 1.3 与 Iteration 000 Stable result 的边界修正
- Depends on: 2.5
- Targets: `openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/specs/statement-cancellation-outcome/spec.md`（已由 Plan 收窄，Act 复核）、`openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/tasks.md`（2.6、RTM 注记、Iteration 000 / 001 的边界注记，已由 Plan 更新，Act 复核）、`openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/design.md`（D2 的索引条目还原边界注记、验证段的「同键再插入」限定，已由 Plan 更新，Act 复核）、`tests/statement_cancellation_test.rs`（`session_keeps_working_after_abandoned_statement`）
- Current behavior: 规格 delta 已被收窄到已验收边界并加注已知边界；「被放弃语句写入的行不可见」在**同一会话**形态上当前为真（回收完成后同一会话 `SELECT id FROM t2` 只读到后续写入的那一行），但仓库里没有直接断言该形态——现有断言都在重开后的形态。
- Required behavior: (a) 复核三处文档（spec delta / `tasks.md` / `design.md`）措辞一致，且不含任何未验收的无条件断言（「索引条目被还原」「同一个键可以再次插入成功」在取消场景下不再出现）；(b) 在 2.4 用例上补一条**同一会话**形态的断言（回收完成后同一会话读不到被放弃语句写入的行）；(c) 在 Act Response 给出「收窄后每条规格断言 → 用例」的映射表；(d) 按「一句话结论 / 根因位置 / 可复现路径或已观察证据 / 影响面与触发条件 / 与本 change 范围的关系 / 建议修复方向 / 承载方式」报告 E1、E2、C1 三个 Issue 候选（只报告，不登记）。
- Preserve: 2.1–2.5 的验收结论；2.5 的 M19 全量结论 2163 / 0 / 2（用例数不变时该结论仍成立）；产品代码零改动；`tests/statement_cancellation_test.rs` 的用例数仍为 8；已交接的 Cycle 文件不变；`openspec/specs/` 不在本 Cycle 的写入面。
- Forbidden: 改任何产品代码（`src/`）；新增测试用例（用例数必须保持 8）；在规格中断言键一定可复用或断言键一定被占用；修改已交接的 Cycle 文件；重跑全量回归（只重跑受影响目标）；为让断言通过而改夹具的隔离级别或键的取值范围。
- First test: 在 `tests/statement_cancellation_test.rs` 的 `session_keeps_working_after_abandoned_statement` 内，于 `expect_affected(&db, "INSERT INTO t3 VALUES (9003)").await;` 之后、`db.close().await.unwrap();` 之前，加入同一会话的读与断言（读 `SELECT id FROM t2`，断言其不含被放弃语句写入的键 1..2000）。命令：`cargo test --test statement_cancellation_test session_keeps_working_after_abandoned_statement -- --test-threads=1`。**本任务是表征用例**（行为当前已成立），不要求修复前 RED；须在 Act Response 记录首次运行即通过的基线与理由，不得为制造 RED 改产品代码。
- GREEN condition: 上列命令退出码 0；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（八个用例全部通过、failed 计数 0，用例数仍为 8）；batch 10 单独重跑退出码 0 且 failed 计数 0（passed 仍为 168）。
- Verification: (1) `cargo test --test statement_cancellation_test session_keeps_working_after_abandoned_statement -- --test-threads=1`；(2) `cargo test --test statement_cancellation_test -- --test-threads=1`；(3) batch 10 的一次独立调用：`cargo test --test select_distinct_test --test self_join_test --test set_operations_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test --test subset_insert_test -- --test-threads=1`（批内 `--test-threads=1`）。失败含义：任一命令的 failed 计数非 0 → 该目标必须单独定位，不得用其他批次的通过掩盖；断言失败若显示同一会话仍读到被放弃语句的行 → 说明收窄后的条款也不成立，按 Stop when 返回 Plan。
- Stop when: 同一会话读到的行集包含被放弃语句写入的键（收窄后的条款也不成立）→ 返回 Plan；或让断言通过需要改产品代码 → 返回 Plan；或复核发现三处文档仍含未验收的无条件断言 → 返回 Plan。

**Invariants**

- 产品代码零改动；`tests/statement_cancellation_test.rs` 的用例数保持 8。
- 2.5 的 M19 分批全量结论 2163 / 0 / 2 继续成立（受影响的一批重跑，其余 12 批结论沿用）。
- 规格库只保存已验收行为：取消域的规格不断言索引写侧的键释放能力，也不断言其反面。
- 缺陷（E1 / E2 / C1）不留在规格里，也不留在本 change 里——由 Issue 承载，由存储域的后续 change 修复。
- 不引入身份证明机制（哈希、指纹、run-id、manifest、revision pin、时间顺序证明），不在项目测试框架之外建判定层。

**Non-goals**

- E1（取消 + 回滚并发下的 B 树结构损坏 → 栈溢出 → 进程 abort）与 E2（取消落在索引写入中途 → 该键仍占用主键索引）的**修复**——落在 `src/storage/btree/` 与索引写入路径，不在本 change 修。
- C1（被放弃的 ALTER TABLE DROP COLUMN 之后「目录已窄化 + 数据面混合宽度」）的修复。
- 新增测试用例、改夹具的隔离级别、改键的取值范围。
- 重跑全量回归；修改 `openspec/specs/`；同步全局 tasks / SNAPSHOT / M / R / I（docs-maintainer 的收尾职责）。

**Acceptance**

- 2.6：`specs/statement-cancellation-outcome/spec.md`、`tasks.md`、`design.md` 三处的措辞一致，取消场景下不再出现「索引条目被还原」「同一个键可以再次插入成功」的无条件断言，已知边界注记只描述「行不可见 / 键可能仍占用」；`tests/statement_cancellation_test.rs` 的 2.4 用例新增同一会话形态断言并通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（八个用例全部通过、failed 计数 0）；batch 10 单独重跑退出码 0 且 failed 计数 0；收窄后每条规格断言都能映射到一个用例；E1 / E2 / C1 三个 Issue 候选按可承载格式在 Act Response 报告。

**Verification**

- 全部从库入口观察（`Database::execute_sql` 的返回行集、`TransactionManager::active_transactions()`、重开后的读、`check_integrity`）；不使用哈希、指纹、run-id、manifest、时间顺序等身份证明机制，不在本项目测试框架之外建判定层。
- 自动判定：`cargo test --test statement_cancellation_test -- --test-threads=1`（八个用例、failed 0）与 batch 10 的单次独立调用（failed 0、passed 168）。
- 人工判定（自动命令无法判定）：「收窄后的规格断言 → 用例」映射表的逐条对照，由 Plan 在 Review 中复核。
- **本 Cycle 不跑全量回归**——2.5 的全量结论覆盖范围未变（只改了测试文件与文档），本 Cycle 只重跑受影响的一批。

**Gate 2 Readiness**

PASS。范围（只做规格收窄 + 一条断言 + Issue 候选报告，不改产品代码）、requirement 映射（RTM 不变，验收面按 2.6 收窄并注记）、任务（2.6 单一任务，Targets / Current & Required behavior / Preserve / Forbidden / First test / GREEN condition / Verification / Stop when 齐备）、验收边界（逐条可映射到用例）、验证方式（定向命令 + 受影响批次）均已确定；Persisted Evidence 模式为 `none`（全部验证可低成本重跑，无一次性现场，无摘要会丢失决定性结构的长输出）。用户批准依据：2026-10-24 用户原话「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」与补充「因为这个项目有点大，三个ms也是串行实施就好」「对于实施当中发现的所有issue候选，都直接寻找好的承载方式流程进行修复而不是等级」——goal 指令构成目标范围内阶段推进的显式授权；规格收窄是「保持规格准确 + 把缺陷交给承载流程」的直接实现，不扩大 change 范围（替代路径「把索引写侧原子性纳入 MS34」超出 MS34 已批准的范围与 Impact，故不在本次授权内）。无 SKIPPED、无 BLOCKED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- **收窄是本 Cycle 的实质交付**：规格 delta 即将由 docs-maintainer 合并进 `openspec/specs/`。若收窄后的条款仍与实现不符，错误会直接进入权威规格库——因此 Act 的复核（(a) 项）是本 Cycle 的关键动作，不得省略。
- **E2 的修复不在本 Cycle**：收窄后的已知边界只说明「键的释放不属本域保证面」，不构成对缺陷的认可或掩盖。E1 / E2 必须按父 Cycle Plan Review 的 Issue 候选格式登记（Recorder 按用户指令执行），并落到存储域的后续里程碑。
- **同一会话断言的取值范围**：断言用「不含被放弃语句写入的键 1..2000」而不是「行数恰好为 2」，避免把 2.4 后续写入的行数也写进断言（后者会与「后续写入的行完整可读」的既有断言耦合）。
- **不重跑全量的理由**：2.5 的 13 批结论产生后，被覆盖的产品代码未变（只有测试文件与文档被本 replan 改动）；受影响的只有 batch 10，重跑该批即可保持全量结论成立。
- 若同一会话断言不成立（回收后同一会话仍读到被放弃语句的行），说明「行不可见」在同会话形态上也不成立——此时不得改产品代码迁就，应返回 Plan 重新界定规格。

## Act Response

- Status: reported

> 本节已被**整体覆盖**为当前 Cycle 的最新完整状态：包含原实施（change 任务 2.6：规格收窄 + 同一会话断言 + Issue 候选报告）、repair item `2.6-R1`（取消触发来源的枚举修正）与 repair item `2.6-R2`（测试文件用例注释的残留断开表述修正）。不保留逐轮历史。

### 实际改动

产品代码零改动（`src/` 未被本 Cycle 修改）。本轮 repair 只改了**测试文件内的一处文档注释**与 change 的 `tasks.md` 状态记录；测试的**行为**（断言、夹具、用例数）零改动。

| 文件 | 符号 / 位置 | 性质 |
|---|---|---|
| `tests/statement_cancellation_test.rs` | `timeout_in_explicit_tx_is_not_a_rollback` 的文档注释（原 `:579-580`，现 `:579-582`） | **repair item `2.6-R2`**：「丢弃、超时与断开在库入口是同一类事件」→「丢弃与超时在库入口是同一类事件；连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果」 |
| 同上 | `session_keeps_working_after_abandoned_statement`（用例体内 `expect_affected(&db, "INSERT INTO t3 VALUES (9003)")` 之后、`db.close()` 之前） | 原实施 2.6 (b)：新增**同一会话**形态断言（读 `SELECT id FROM t2`，断言读到的行集不含被放弃语句写入的键 1..2000；另加一条「读到的行集非空」的前置断言，防空洞通过） |
| 同上 | 同用例的文档注释 | 原实施 2.6：补记同一会话断言的形态与范围（纯注释） |
| `openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/specs/statement-cancellation-outcome/spec.md` | Purpose（`:5`）、R1 正文（`:11`）、场景标题（`:13`）与 WHEN（`:15`） | **repair item `2.6-R1`**：取消触发来源的枚举去掉「连接在结果送达前断开」；R1 内补入「断开不是取消触发来源」的准确陈述 |
| `.../design.md` | D1（`:21`）、时序图 Note（`:56`） | 同口径修正 |
| `.../proposal.md` | `:11`、`:41` | 同口径对齐（纯措辞，无行为内容） |
| `.../README.md` | `:3` | 同口径对齐（纯措辞，无行为内容） |
| `.../tasks.md` | 任务 1.2 下的新注记 | **边界对齐**（措辞对齐，不改语义，见 Deviations 1） |
| 同上 | 任务 2.6 下的 repair item 实施注记（`2.6-R1`、本轮 `2.6-R2`） | **任务状态维护**（不改完成状态 / RTM 映射，见 Deviations 4、5） |
| 同上 | Iteration 001 的「状态」行 | 记录 replan Cycle `001-replan.md` 与两个 repair item 的实施状态（Act Response `reported`、待 Plan 重新 Review） |

未触碰：`src/`（产品代码）、权威规格库 `openspec/specs/`、`.agents/docs/tasks.md`（全局文档的既有未提交修改）、父 Cycle `iterations/001-caller-visible-outcomes-and-final-regression/000-initial.md` 与 Iteration 000 的两个 Cycle 文件（已交接，按禁止项不回改）。

### repair item `2.6-R2` 的实施：测试文件用例注释的残留断开表述

**问题（Plan Review 的 F3 / GAP-2 判定）**：`tests/statement_cancellation_test.rs` 的用例 `timeout_in_explicit_tx_is_not_a_rollback` 的文档注释仍把「连接断开」列为与丢弃/超时**同类的取消事件**，与实现相反，并与已修正的规格 delta（`specs/statement-cancellation-outcome/spec.md:11` 的 R1 正文、`:13` 的场景标题「丢弃与超时是同一类事件」）及同一 delta 的 R3 冲突。

**修复前（`:579-580`，逐字）**：

> 「超时是语句 future 被放弃的另一种来源（丢弃、超时与断开在库入口是同一类事件）。显式事务路径不因语句结束而收尾事务——超时到期后事务仍然活跃、行仍在事务内，只有调用方显式回滚才撤销它。」

**修复后（`:579-581`，逐字）**：

> 「超时是语句 future 被放弃的另一种来源（丢弃与超时在库入口是同一类事件；连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果）。显式事务路径不因语句结束而收尾事务——超时到期后事务仍然活跃、行仍在事务内，只有调用方显式回滚才撤销它。」

- 采用 Plan Review 在 repair item `2.6-R2` 中**规定的替换文本**（「丢弃与超时在库入口是同一类事件；连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果」），与规格场景标题「丢弃与超时是同一类事件」同形。
- 注释里其余句子（「显式事务路径不因语句结束而收尾事务——超时到期后事务仍然活跃、行仍在事务内，只有调用方显式回滚才撤销它。」）**逐字保留**，只因换行位置调整而分行（段内 3 行 → 4 行；行宽 74 / 76 / 74 / 46 列，均在该文件 doc 注释的既有宽度内，最大 79）。
- **仅注释**：断言（`assert!` / `assert_eq!` 共 39 处）、夹具（隔离级别、键取值范围、`ABANDONED_SQL`、`SOURCE_ROWS`、`ANCHOR_TIMEOUT`、`TIMEOUT_CANCEL_WINDOW`）、用例数（仍为 8）、产品代码（`src/`）均未改。

**实现事实复核**（Plan Review F2 的读码结论，本轮复读确认，用于决定写进注释的准确措辞）：

- `src/network/connection.rs:18-35`：每连接处理器是顺序循环——`parse_request`（`:23`）→ `self.handler.execute(req).await`（`:27`，语句执行到结束）→ `write_response`（`:30`，只有到这里才可能因客户端断开而 I/O 失败）。**语句执行期间没有任何观察 socket 存活状态的点。**
- `src/network/server.rs:39-66`：accept 循环里的 `tokio::select!` 只用于「接收新连接 vs 停机」；已建立的连接任务在 `handle` 返回前不会被取消。
- `src/network/pg_protocol.rs:216-229`：Ready 态只处理 `Q`（Query）与 `X`（Terminate），**没有** CancelRequest 的处理路径。
- `src/parser/planner/mod.rs:437-441`：库入口拒绝 `BEGIN` / `COMMIT` / `ROLLBACK` → 网络路径上的每条语句都是自动提交语句，故「语句执行完毕并提交」在断开门径上**没有反例**。
- 结论：客户端断开时语句 future 仍被驱动到执行完毕并提交，断开只表现为**结果未送达**（落在 R3 一侧）——注释中的准确陈述与实现一致。

### 全仓扫描复核结果（repair item `2.6-R2` 的验收判据）

本次实跑检索（`grep -rn`，全仓，排除 `target/`）：

| 检索面 | 命中 | 每一处现在说了什么 | 是否残留「断开 = 取消」 |
|---|---|---|---|
| `tests/`（`断开` / `断连` / `disconnect`） | **1** 处：`tests/statement_cancellation_test.rs:580` | 「连接断开**不是**取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果」 | **否**——该处即准确陈述 |
| `tests/`（`同一类事件` / `同一件事` / `同一个事件`） | **1** 处：`tests/statement_cancellation_test.rs:579` | 「丢弃与超时在库入口是同一类事件」 | **否**——断开已不在枚举内 |
| `src/`（`同一类事件` / `同一件事` / `同一个事件`） | **0** | — | 否 |
| `src/`（`断开` / `断连` / `disconnect`） | **1** 处：`src/network/connection.rs:33` | 英文源码注释「keeping the connection semaphore permit held until I/O error disconnects.」——描述连接任务自身的 permit 释放，与取消触发来源无关 | 否 |
| `openspec/specs/`（72 个域，`断开` / `断连` / `disconnect`） | **0** | — | 否（合并 delta 不会与既有权威域冲突） |
| change 目录（`断开`） | `specs/…/spec.md:11`、`design.md:21`、`proposal.md:11`、`tasks.md:29`/`:30`/`:62`、Iteration 000 的两个 Cycle 文件、Iteration 001 的 `000-initial.md` 与当前 Cycle 文件 | 规格 / design / proposal 的三处**准确陈述**（断开不是取消触发来源）；`tasks.md` 的**实施记录与 Review 的事实性描述**；Cycle 文件的**历史现场记录**（按禁止项不回改） | 否——没有任何一处把断开列为取消触发来源 |

- **结论**：repair item `2.6-R2` 的实质判据（「本 change 的全部产物不再残留『断开 = 取消』的表述」）**满足**。唯一残留的「断开」字面命中是本轮写入的**准确陈述**，以及 Plan Review 允许保留的历史记录与实施记录。
- **字面判据的偏差（须报告）**：Plan Review 的验证步骤写「在 `tests/statement_cancellation_test.rs` 全文检索『断开』→ 应命中 **0** 处」，但同一 Review 规定的替换文本（「连接断开不是取消触发来源——……」）本身含「断开」一词，该字面判据与 Review 自己的替换文本互斥、无法同时满足。本次按**权威执行依据**（Review 规定的替换文本 + Review 的实质判据）执行，实际命中 **1** 处且该处即准确陈述。详见 Deviations 6。

### requirement / scenario 数量核对与用例数核对

| 项 | 实测 | 结论 |
|---|---|---|
| `specs/statement-cancellation-outcome/spec.md` 的 Requirement 数 | `grep -c '^### Requirement:'` = **6** | 与修复前一致，未增删 |
| 同上，Scenario 数 | `grep -c '^#### Scenario:'` = **10** | 与修复前一致，未增删 |
| `tests/statement_cancellation_test.rs` 的用例数 | `#[tokio::test]` / `#[test]` = **8** | 与修复前一致，未增删 |
| 断言数 | `assert!` / `assert_eq!` 共 **39** 处 | 未改（本轮只改注释） |

- R1 的 requirement 标题「取消触发集合是语句 future 被放弃」未改；R2–R6 正文与「已知边界」注记未改；`tasks.md` 的 RTM 映射未改。

### 三处文档一致性复核（2.6 (a)）

逐处结论——三处措辞一致，取消场景下已无「索引条目被还原」「同一个键可以再次插入成功」的**无条件**断言；已知边界注记只描述「行不可见 / 键可能仍占用」。

**1. 规格 delta `specs/statement-cancellation-outcome/spec.md`**

- requirement「提交前被取消的语句不提交」正文：枚举为「未提交版本被墓碑化、事务移出活跃事务集合、回收时机、页锁随 future 释放」——**已无「索引条目被还原」**。
- 场景「隐式语句在执行中被放弃后不提交」THEN：「被放弃语句写入的行不可见；下一条写语句开始之前该语句的隐式事务已回滚，`TransactionManager::active_transactions()` 为空」——**已无「同一个键可以再次插入成功」**。
- 场景「被放弃语句的未提交数据不随 checkpoint 落盘」THEN：「被放弃语句写入的行不存在」——**已无同键断言**。
- 场景「取消不留下悬空事务」：只涉自动 checkpoint 的顺延判定，无索引断言。
- **已知边界注记**：只写「被取消语句的**行**不可见（本 requirement 的验收面），但它的**键**仍可能占用主键索引（再插同键被拒）。键的释放不属本域的保证面，由存储域的索引写侧契约承载，作为 Issue 候选另行处置」——用的是「**可能**占用」，既不断言键一定可复用，也不断言键一定被占用（后者会把缺陷写成契约）。
- requirement「显式事务继续还是回滚由调用方决定」：正文已是「该事务内的**行修改**……被撤销」，并显式声明「键的索引条目释放不属本域的保证面（见已知边界）」——与已知边界注记自洽。

**2. `design.md`**

- D2 的「登记表的语义」仍写「回收走既有中止路径：…还原索引条目…」，但**紧接其后**的「**索引条目还原的边界（2026-10-24 Plan Review 修正）**」已明确该句「描述的是中止路径的既有语义，不是在取消场景下的无条件保证」，并写明缺口只对被取消语句的首个键之外的键成立、键的释放不属本 change 的验收面。
- 验证段：「同键再插入（**限被放弃语句的首个键**，见 D2 的边界注记）」——已限定。
- 背景段的「同一个键再次插入会拿到 `DuplicateKey`」描述的是**改动前的问题态**，不是目标行为的断言——保留原文正确，不构成残留断言。

**3. `tasks.md`**

- 任务 1.3 保留原文「同一个键可以再次插入成功」，其下**已带**「2026-10-24 Review 边界修正」注记，把已验收范围限定为被放弃语句的首个键，并指向 Issue 候选 E2——是有界表述，不是无条件断言。
- Iteration 000 的 Stable result 保留原文「索引条目已还原、同键可再插」，其下**已带**同形的边界修正注记——同上。
- RTM 注记、任务 2.6 正文、Iteration 001 的状态 / Stable result / Verification boundary / Non-goals 均已写明「键的索引条目释放被标注为存储域边界，不属本 change 的验收面」。
- 任务 1.2（`:12`）的「回收走既有中止路径（`AbortTxn`、版本墓碑化、**索引还原**、移出活跃集、清版本条目）」已带一条 dated 边界对齐注记（不改 1.2 的原文与完成状态），与 `design.md` D2 和规格 delta 的已知边界三处对齐。

**复核结论**：三处在取消场景下都不再含未验收的无条件断言；已知边界注记只描述「行不可见 / 键可能仍占用」；未发现需要回报 Plan 的实质冲突。

### 2.4 新增的同一会话断言（原实施 2.6 (b)）

- 位置：`tests/statement_cancellation_test.rs`（`expect_affected(&db, "INSERT INTO t3 VALUES (9003)").await;` 之后、`db.close().await.unwrap();` 之前），按 Task Definition 的 First test 指定位置落成。
- 判定依据：库入口返回的行集（`expect_rows(&db, "SELECT id FROM t2")`）——**行不可见**是断言对象；键是否释放**不是**断言对象（既不断言键一定可复用，也不断言键一定被占用）。
- 取值范围：断言「读到的每行 id > 2000」（即不含被放弃语句写入的键 1..2000），**不用**「行数恰好为 2」（避免与「后续写入的行完整可读」的既有断言耦合）。
- 非空洞保证：额外断言读到的行集非空——否则「不含被放弃键」在空行集上会空洞通过。
- 未用内部状态、哈希、指纹、run-id、manifest 或时间顺序；判定全部落在 `cargo test` 内的 `assert!`；无框架外判定层；`SOURCE_ROWS` 是夹具常量而非判据，`ANCHOR_TIMEOUT` 仍是夹具失控兜底。
- **基线记录（表征用例分支）**：该断言属「行为当前已成立」的表征用例——**首次行为运行即 GREEN**（验证表第 1 行），按 AGENTS.md 的表征分支处理，**未改任何产品代码去制造 RED**。父 Cycle 的 Plan Review 已用独立探针观测到该形态为真（RR 与 RC 各一次）。

### 验证命令与实际结果

| # | 命令 / 人工步骤 | 实际结果 | 退出码 |
|---|---|---|---|
| 1 | `cargo fmt --check` | 无 diff（仅 cargo 的 `~/.cargo/config` 弃用告警，与本仓无关） | 0 |
| 2 | `cargo test --test statement_cancellation_test -- --test-threads=1`（**本轮 repair 后实跑**） | `running 8 tests` → 8 个用例逐个 `ok` → `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 40.57s` | 0 |
| 3 | 人工逐处对照：全仓检索「断开 / 断连 / disconnect」与「同一类事件 / 同一件事 / 同一个事件」 | 逐处命中与结论见「全仓扫描复核结果」表；无任何一处把断开列为取消触发来源 | —（人工判定） |
| 4 | 人工核对：requirement / scenario / 用例数 / 断言数 | Requirement = 6、Scenario = 10、用例数 = 8、断言数 = 39，均与修复前一致 | —（人工判定） |
| 5 | `cargo test --test statement_cancellation_test session_keeps_working_after_abandoned_statement -- --test-threads=1`（原实施 2.6 首版） | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 5.63s`（**首次行为运行即 GREEN**，表征型基线） | 0 |
| 6 | `cargo test --test statement_cancellation_test -- --test-threads=1`（原实施 2.6 末版） | `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 42.82s` | 0 |
| 7 | batch 10 单次独立调用（命令见下） | 11 个目标逐个 `test result: ok`，**合计 168 passed / 0 failed / 0 ignored** | 0 |
| 8 | `cargo clippy --test statement_cancellation_test --tests` | 退出码 0；来自 `tests/statement_cancellation_test.rs` 的告警 **0** 条 | 0 |

**batch 10 的目标清单与逐目标计数**（M19 纪律：一次独立调用、批内 `--test-threads=1`、不与其它批并行）：

命令：`cargo test --test select_distinct_test --test self_join_test --test set_operations_test --test sort_test --test sql_differential_test --test statement_cancellation_test --test storage_test --test structural_property_test --test subquery_alias_binding_test --test subquery_test --test subset_insert_test -- --test-threads=1`

| 目标 | passed | failed | ignored |
|---|---|---|---|
| select_distinct_test | 12 | 0 | 0 |
| self_join_test | 6 | 0 | 0 |
| set_operations_test | 21 | 0 | 0 |
| sort_test | 8 | 0 | 0 |
| sql_differential_test | 10 | 0 | 0 |
| statement_cancellation_test | 8 | 0 | 0 |
| storage_test | 21 | 0 | 0 |
| structural_property_test | 5 | 0 | 0 |
| subquery_alias_binding_test | 25 | 0 | 0 |
| subquery_test | 46 | 0 | 0 |
| subset_insert_test | 6 | 0 | 0 |
| **合计** | **168** | **0** | **0** |

**未重跑全量的依据**（AGENTS.md › 验证）：

- 高开销全量回归按 change 预算：**整个 change 一次**，已由任务 2.5 完成（13 批全 0 failed，合计 **2163 passed / 0 failed / 2 ignored**，逐批退出码 0，覆盖对账 missing 0 / extra 0 / duplicate 0）。
- 2.5 的结论产生后，**被覆盖的产品代码未发生变化**：本 Cycle 与本轮 repair 均未改 `src/`（`git status --short` 的 `src/` 四项改动与父 Cycle 实施完成时一致）；本轮只改了一处文档注释与 change 目录内的文档。
- 因此其余 12 批的结论**直接沿用**（来源：父 Cycle `000-initial.md` 的分批明细表）；受影响的唯一目标是 `statement_cancellation_test`，已按 repair item `2.6-R2` 的验证要求单独重跑（验证表第 2 行，8 passed / 0 failed，退出码 0）。
- 不属于 AGENTS.md 列出的重跑情形（结论与代码不矛盾、被覆盖代码未变化、无失败、非同一问题第三次失败、非 rework / replan 受影响范围）。

### 收窄后「规格断言 → 用例」映射

| # | 收窄后的规格断言 | 规格位置 | 对应用例 |
|---|---|---|---|
| A1 | 丢弃 / 超时在库入口是同一类取消事件，结果不因触发来源不同而不同 | R1 + 场景「丢弃与超时是同一类事件」 | 1.1 `abandoned_implicit_statement_is_not_committed`（丢弃）、2.3 `timeout_in_explicit_tx_is_not_a_rollback`（超时） |
| A2 | 库入口不新增取消或 deadline 公共 API；未被取消的语句的行为不变 | R1 正文 | 结构性不变量（`src/` 无新增公共 API）；并发面由 `concurrent_statement_does_not_abort_other_statement_tx` 证明（另一条语句的回收不中止仍在执行的活事务） |
| A3 | 库不提交被取消的语句 | R2 | 1.1、1.3 `abandoned_statement_does_not_survive_reopen` |
| A4 | 中止隐式事务：未提交版本被墓碑化 → **行不可见** | R2 + 场景 1 | 1.3、2.4（重开后形态）+ 2.4 的**同一会话新增断言** |
| A5 | 事务移出活跃事务集合（`active_transactions()` 为空） | R2 + 场景 1 | 1.1 |
| A6 | 回收在下一条隐式写语句开始之前完成 | R2 + 场景 1 | 1.1（同句柄下一条写语句执行完后活跃集为空） |
| A7 | 回收在下一次 checkpoint 之前完成 | R2 + 场景 2 | 1.3（`close()` → 重开后行不存在） |
| A8 | 被取消语句持有的页锁随 future 释放 | R2 正文 | 2.4（同一会话后续对**同一张表** `t2` 的写入成功 → 页锁未残留） |
| A9 | 取消不留下悬空事务（自动 checkpoint 的顺延判定不被被放弃的事务顺延） | R2 + 场景 3 | 1.1（活跃集为空 → 顺延条件对已回收的事务不成立） |
| A10 | 提交已完成但结果未送达 → 已提交：同一会话读得到、`close` 后重开仍在 | R3 + 场景 | 2.1 `committed_statement_survives_undelivered_result` |
| A11 | 显式事务内的语句被放弃 → 库不提交也不回滚，事务保持活跃且可用 | R4 + 场景 1 | 2.2 `abandoned_statement_in_explicit_tx_keeps_tx_open`、2.3 |
| A12 | 调用方回滚撤销该事务内的**行修改** | R4 + 场景 2 | 2.2 |
| A13 | 调用方提交保留该事务内的行修改并持久化 | R4 + 场景 3 | 2.2、2.3 |
| A14 | 超时是对语句的放弃，不是对事务的回滚 | R5 + 场景 | 2.3 |
| A15 | 取消之后会话级对象（WAL writer / WAL buffer / 缓冲池）不被拆除；后续写入正常提交、重开后数据完整 | R6 + 场景 | 2.4 `session_keeps_working_after_abandoned_statement` |
| A16 | **已知边界**：取消落在索引写入中途 → 行不可见（验收面），键**可能**仍占用主键索引（不属本域保证面） | R2 已知边界注记 | **不作断言**（显式声明不属验收面）；缺陷本体由 Issue 候选 E2 承载 |

- **结论**：收窄后每条**行为性**规格断言（A1–A15）都映射到至少一个用例；A16 是显式声明的边界，不是待验收的行为断言。
- **条款级覆盖边界（如实列出，供 Plan Review 判定）**：
  1. **断开不是取消触发来源**——客户端连接在结果送达前断开时，库入口不放弃正在执行的语句，语句照常执行完毕并提交，调用方得到的只是未送达的结果 → 该结果由 **R3 覆盖，已由 2.1 验收**（提交已完成但未送达 → 已提交）。本 change 不改网络协议面（Non-goal：只作触发来源核对，不改协议面），故断开路径本身无独立用例；R1 的该句是对 R3 的指路，不是新增的独立保证。本轮 repair 把这条准确陈述同步到了测试文件的用例注释。
  2. 「语句级只读后台任务（预取）随语句结束而自然终止」——预取只读、不写盘（design D5），其终止无可观察的数据面后果，由 2.4 的「会话继续可用」间接覆盖，无独立用例。
- 两条都**不是**收窄引入的新断言：收窄前的规格 delta 即如此，父 Cycle 的 Plan Review 已判定「收窄后唯一缺直接断言的条款」是「行不可见」的**同一会话**形态——该缺口已由 2.4 的新增断言闭合。

### Deviations from Plan

1. **`tasks.md` 任务 1.2 补了一条边界对齐注记**（措辞对齐，不改变已收窄的 requirement 语义）。复核项 (a) 发现：`design.md` D2 对「回收走既有中止路径：…还原索引条目…」这句有明确的边界注记，而 `tasks.md` 任务 1.2 的同一句**没有**——三处措辞不齐，且该句位于取消场景的描述里。按 Task Definition 允许的「不改变语义的措辞对齐」补齐（新增 dated 注记，1.2 的原文与完成状态不变）。分类：**Act 补齐 Plan 遗漏的措辞对齐**（不改语义、不改状态）。
2. **2.4 的断言额外加了一条「读到的行集非空」的前置断言**。Plan 只要求「不含被放弃语句写入的键 1..2000」；空行集会让该断言空洞通过（Gate 3 的「测试不会因错误原因通过」），故加前置断言。断言对象仍是**行不可见**，不涉及键是否释放，也未把「行数恰好为 2」写进断言（保持 Plan 的取值范围要求）。
3. **2.4 用例的文档注释补记了同一会话断言的形态与范围**（纯注释，无行为影响）。
4. **repair item `2.6-R1` 的变更范围清单未含 change 的 `tasks.md`**。按「按 change 的 `tasks.md` 维护任务状态 / change 结构自检：tasks 状态与实际完成一致」在 `tasks.md` 补了两处状态记录：任务 2.6 下追加 repair item 的实施注记、Iteration 001 的「状态」行记录 `2.6-R1` 已实施。**未改任何任务的完成状态、未改 RTM 映射**（repair item 的禁止项）。分类：**Act 的任务状态维护**，不改变行为语义与验收边界。
5. **repair item `2.6-R2` 的变更范围清单同样只列了 `tests/statement_cancellation_test.rs:579-580`，未含 `tasks.md`**。本轮按同一理由补记：任务 2.6 下追加 `2.6-R2` 的实施注记、Iteration 001 的「状态」行记录 `2.6-R2` 已实施。**未改任何任务的完成状态、未改 RTM 映射**（repair item 的禁止项）。分类：**Act 的任务状态维护**，与 Deviations 4 同形。
6. **Plan Review 的「检索『断开』应命中 0 处」字面判据未按字面满足**。该判据与 Review 自己规定的替换文本（「连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果」）互斥——替换文本含「断开」一词，两者无法同时成立。本次按**权威执行依据**（Review 规定的替换文本 + Review 的实质判据「不再残留『断开 = 取消』的表述」）执行：实际命中 **1** 处，且该处**正是**写入的准确陈述。分类：**Plan 验证步骤的字面判据与自身的替换文本冲突**，Act 取替换文本并如实记录实测命中数；不构成行为语义偏差，也不掩盖任何残留。

无产品代码偏差；未改夹具的隔离级别、键取值范围与用例数（仍为 8）；未改断言；未增删 requirement 或 scenario（仍为 6 / 10）；未重跑全量；未创建下一 Cycle / 下一 Iteration；未填写 Plan Review；未改 `openspec/specs/`；未动全局 `tasks.md` / SNAPSHOT / 项目记忆。

### 自检发现、已修复内容与遗留 Minor

| # | 发现 | 处理 |
|---|---|---|
| 1 | 三处文档复核发现 `tasks.md` 任务 1.2 的「索引还原」缺 `design.md` D2 同形的边界注记——措辞不齐 | 已补 dated 边界对齐注记（不改语义、不改状态）→ 见 Deviations 1 |
| 2 | 「不含被放弃键」的断言在空行集上会空洞通过 | 已加「行集非空」前置断言 → 见 Deviations 2 |
| 3 | 2.4 的用例文档注释未反映新增的同一会话断言 | 已补记（纯注释） |
| 4 | 规格 delta 把「网络连接在结果送达前断开」列为与丢弃/超时同类的取消触发来源，与实现相反并与 R3 冲突（Plan Review F1 / GAP-1） | 已按 repair item `2.6-R1` 修正 Purpose / R1 正文与场景、design D1 与时序图 Note、proposal、README，并补入准确陈述 |
| 5 | **本轮**：测试文件 `timeout_in_explicit_tx_is_not_a_rollback` 的用例注释仍把「连接断开」列为与丢弃/超时同类的取消事件（Plan Review F3 / GAP-2） | 已按 repair item `2.6-R2` 修正为「丢弃与超时在库入口是同一类事件；连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果」（仅注释） |
| 6 | **本轮**：Plan Review 的「检索『断开』应命中 0 处」字面判据与 Review 自己规定的替换文本互斥 | 按替换文本执行并如实记录实测命中（1 处，即准确陈述）→ 见 Deviations 6 |
| 7 | **本轮**：全仓扫描发现 `src/network/connection.rs:33` 的英文注释含 `disconnects` | 已核对：描述的是连接任务自身的 semaphore permit 在 I/O 错误时释放，与「取消触发来源」无关，不是残留断言；**不改**（不在本 Cycle 的变更面内，且改它对读者理解无影响） |

- **遗留 Critical/Important：无。** 产品代码零改动；本轮修复只改一行文档注释；无断言改动；用例数 8、requirement / scenario 6 / 10 未变；无身份证明机制、无框架外判定层、无 timer 阈值作行为判据。
- **遗留 Minor：无（本轮新增）。** `cargo fmt --check` 与 clippy 的结论沿用（本轮未改 Rust 代码路径，覆盖范围未变化）；文档改动无命名/结构问题。
- **沿用的既有 Minor（不属本 Cycle 修复范围，记录备查）**：Plan Review 的 F7 记两条 Issue 候选承载格式的小改进（C1 已由该 Review 补齐最短可观察复现步骤；E1 的信号名在父 Cycle 两处记载不一致，统一采用父 Cycle Plan Review 的 `SIGABRT`）；F8 记「由存储域的索引写侧契约承载」是前瞻引用（该契约随修复它的后续 change 建立）。两条均不阻塞，由后续承载处理。

### Gate 结论

- **Gate 3（任务级 Review）— PASS**
  - Spec compliance：repair item `2.6-R2` 的变更位置精确落地（`tests/statement_cancellation_test.rs` 的用例注释，`:579-580` → `:579-582`）；注释中的「丢弃与超时是同一类事件」与规格场景标题（`spec.md:13`）同形，「连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果」与 R1 正文（`spec.md:11`）同口径；断言 / 夹具 / 用例数（8）/ requirement 与 scenario 数量（6 / 10）均未改；产品代码零改动；已交接的 Cycle 文件未改；权威规格库 `openspec/specs/` 未动。
  - Code quality：diff 无计划外修改（注释外无任何改动）；未引入「断开 → 已提交」以外的新语义；无身份证明机制、无判定层、无 timer 阈值作行为判据；行宽与该文件既有 doc 注释一致（≤ 79 列）。
- **Gate 4（Verified Completion）— PASS**：repair item `2.6-R2` 的自动判定全部满足——`cargo fmt --check` 退出码 0；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0，`test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 40.57s`，用例数仍为 8；两项人工判定（全仓逐处检索核对、6 / 10 / 8 / 39 计数核对）均已实际执行并记录结果。完成声明基于本次真实运行与本次实跑的检索计数；未重跑全量的依据已写明（2.5 的 2163 / 0 / 2 覆盖范围未变，受影响的唯一目标已单独重跑）。
- **Gate 5：未命中。** 修复不需要新的执行依据、未改动任何产品代码或测试策略、未发现需要回报 Plan 的实质冲突。

### Persisted Evidence

- **None required**：全部验证均为可低成本重跑的定向命令（单文件目标 / fmt / clippy）与可复现的人工检索判定，逐条命令与实际计数已完整记录在本 Response；无一次性环境即将消失的现场，无摘要会丢失决定性结构的长输出，未创建 `evidence/` 目录。

### Experience Candidates

> 格式按用户本轮约定（「不要只做等级分类，必须给出可承载的完整信息」）。三条均为**本 change 范围外**的实质缺陷，只报告、不登记、不修复——登记由 Recorder 按用户指令执行，修复由后续 change 承担。**承载来源**：本 Cycle 的 Plan Review「Issue 候选」段（与 Act Response 的 E1 / E2 / C1 段内容一致）；**本轮 repair 未重新复现这三条**（本轮只改测试文件的一行注释，不改产品代码，三条也不在本 change 的验收面内）。

**E1（进程级 abort；必须登记并修复）**

- **一句话结论**：在显式事务里取消一条正在执行的多行 `INSERT … SELECT`，随后对该事务调用回滚，会让 B 树结构损坏——之后的一次索引插入沿「内部节点的子指针 = 0」无限下降，栈溢出使整个进程 abort。
- **根因位置**：`src/storage/btree/btree.rs:327`（`BTree::insert_into_page` 的内部节点下降递归）、`src/storage/btree/btree.rs:243`（叶子/内部判定 `data_guard[0] == LEAF_NODE`）；损坏入口 `src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert`：B 树条目在 `spawn_blocking` 内写入——tokio 的 blocking 任务不可取消，反向映射 `row_to_key` 在其后的 await 之后才写）与 `src/storage/btree/index_manager.rs:261-282`（`IndexManager::delete`：回滚路径删条目时会合并/重分布叶子并可能换根）。两条写路径之间没有写者串行化，根页 id 的更新与树内容变更也不在同一临界区。
- **可复现路径 / 已观察证据**：父 Cycle 的 `cargo test --test statement_cancellation_test timeout_in_explicit_tx_is_not_a_rollback`（超时窗口 100ms 时连续两次复现）。gdb 现场：`thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT；栈上是 339+ 层 `BTree::insert_into_page`，帧内 `child_page_id_u32 = 0`（内部节点的子指针为 0 → 反复下降到第 0 页）；崩溃那次插入的键为被取消语句写入的键 111，`BTree::root_page_id = 75` 而该行的 `RowId.page_id` 也是 75（根页 id 指向了一个不是 B 树节点的页）。父 Cycle 的 Plan Review 独立读码确认机制成立，6 次复现尝试未命中 → **竞态依赖**。
- **影响面与触发条件**：触发条件 =「一条会写多行的 INSERT…SELECT 在索引写入中被取消」+「该事务随后被回滚（回滚要删这些行的索引条目）」。用户可见后果是**整个进程崩溃**（不止当前语句失败），且崩溃点与取消无关、出现在之后的任意索引写入上——同一句柄上任何后续写语句都可能踩到。取消窗口越大越容易触发（100ms 窗口取消前约 523 行 → 稳定复现；20ms 窗口约 100 行 → 本机 8 次未复现）。隐式事务的回收路径（同样调 `abort_by_id`）可能同形命中。
- **与本 change 范围的关系（为什么超出）**：修复点落在存储/索引层的**写者串行化**与取消时的原子性，不在「取消/超时/事务收尾」的调用方语义面；超出本 change 的 Impact（`src/database.rs` / `src/pipeline.rs` / `src/executor/alter_table.rs` / `src/transaction/manager.rs`）与 design D1–D6；两个 Iteration 的 Plan 都明确不改索引语义，Task Definition 2.2 的 Stop when 也规定「需要改 `Database::rollback` / `abort` 的既有语义 → 返回 Plan」。本 change 以「规格收窄 + 已知边界注记」结清，不吸收修复。
- **建议修复方向**：① 按索引串行化结构变更（插入/删除/合并/换根），并让「B 树条目 + 反向映射 + 根页 id」对其他写者原子可见；或 ② 让取消路径不留下半写状态（把两次写放进同一不可中断的临界区，根更新同临界区）；③ 回归用例：取消一条多行 INSERT…SELECT → 回滚 → 同句柄上的后续写语句不得崩溃，且 `check_integrity` 无 Error 级发现。
- **建议的承载方式**：登记为 Issue（`.agents/issues/`），修复放在**存储/索引写侧**的后续里程碑——建议新开一段「索引写侧原子性与取消后一致性」，与 MS35「空闲页跨重启复用」相邻（E1 现场涉及根页 id 指向被复用的数据页，两段在 `src/storage/` 的改动面可能重叠，须由 milestone-planner 统一排期）；**不在 MS34 内修**。

**E2（键仍被占用；必须登记并修复）**

- **一句话结论**：显式事务内一条多行 `INSERT … SELECT` 被取消后调用方回滚，被取消语句写入的键（**首个键之外**）仍留在主键索引的正向条目里，同一个键再插入报 `DuplicateKey`；行本身不可见。
- **根因位置**：`src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert` 的两段写：正向条目在 `spawn_blocking` 内完成，反向映射 `row_to_key.insert` 在其后的 await 之后才写——取消落在两者之间时，正向条目 key→row_id 已在、反向映射 row_id→key 缺失）与 `src/transaction/manager.rs:344-355`（`abort_cleanup_versions` 的 A/B 分桶只查反向映射 `find_key_by_row_id`；反向映射缺失的行被误判为「墓碑」而跳过，其正向索引条目不被移除）。
- **可复现路径 / 已观察证据**：父 Cycle 2.2 用例首版（同表同键重复「放弃 → 提交」）实测 RED：`the statement finished before the abandon anchor was reached: Err(SqlError { message: "Execution error: Duplicate key" })`。诊断现场（回滚前）：活跃事务的版本集合 = `{RowId{page 4, slot 0}, RowId{page 4, slot 1}, RowId{page 4, slot 2}}`；`find_key_by_row_id(slot 0) = Some(key 1)`、`find_key_by_row_id(slot 1) = None`、`find_key_by_row_id(slot 2) = Some(key 9999)`；索引里 `search(key 1) = Some(slot 0)`、`search(key 2) = Some(slot 1)`。回滚后：`search(key 1) = None`（已释放 ✓）、`search(key 2) = Some(slot 1)`（**残留 ✗**）。父 Cycle 的 Plan Review 独立复现：显式路径与隐式路径、RR 与 RC 各一组——键 2 报 `Duplicate key`，键 1 与 3..10 成功；被取消行的 `find_key_by_row_id` 为 `None`。
- **影响面与触发条件**：触发条件 =「`INSERT` 被取消在索引写入中途」+「调用方回滚 / 回收」。用户可见后果：被取消语句占用的键无法复用（`DuplicateKey`），且索引条目指向已中止的行。隐式回收路径同样受影响——Iteration 000 的用例只复用了**首个**键，所以没有暴露这一点。
- **与本 change 范围的关系（为什么超出）**：修复点在**索引写入的原子性**与**中止路径的行分类**，不在调用方语义面；同时是 Iteration 000 稳定结果「索引条目已还原、同键可再插」在首个键之外不成立的证据，故父 Cycle 的 Plan Review 判 `replan-required` 收窄规格，而不是在本 change 修实现。收窄后的规格把「键的释放」移出本域保证面（已知边界），缺陷本体不在本 change 结清。
- **建议修复方向**：① 让 `IndexManager::insert` 的正向条目与反向映射成为不可分割的一步（同一临界区，取消不留下半写状态）；② 让 `abort_cleanup_versions` 的分桶不只依赖反向映射（例如同时按正向条目反查该 row_id，或从元组派生 PK 键反查条目），使「有正向条目但反向映射缺失」的行仍走 A 趟（删除条目）；③ 回归用例：取消多行 INSERT → 回滚 → **全部**被取消行的键都能再次插入。① 与 E1 的修复面重叠，应与 E1 合并到同一后续 change。
- **建议的承载方式**：与 E1 合并为同一 Issue 主题（「索引写侧原子性」），由 Recorder 登记，修复落在上述存储/索引侧的后续里程碑；本 change 以「规格收窄 + 已知边界注记」结清，键释放的行为规格随该后续 change 进入 `openspec/specs/`。

**C1（数据面形态不一致；范围外实质问题）**

- **一句话结论**：被放弃的 ALTER TABLE DROP COLUMN 之后，表处于「目录已窄化 + 数据面仍为混合宽度」的形态——回收只回滚数据面，目录侧的窄化不在事务内。
- **根因位置**：`src/storage/data/table_manager.rs:793-802`（`alter_drop_column` 先改目录并 `flush_all`，窄化不在事务内；事务只覆盖随后的逐行 DELETE + INSERT 数据面改写）。
- **可复现路径 / 已观察证据**：Iteration 000 与其 rework Cycle 的 Plan Review 均报告（第三处库属隐式事务站点只补了回收契约的回归用例 `abandoned_alter_table_drop_column_is_reclaimed`，**不断言数据面形态**）；本 change 的回收只覆盖数据面回滚，目录侧不在回收范围内。**最短可观察步骤**（Plan Review 补齐）：在库入口发起一条 `ALTER TABLE t DROP COLUMN v` 并在其执行期放弃该语句的 future → 触发下一条写语句的回收 → `close()` 后重开 → `SELECT * FROM t` 的目录列数（已窄化）与数据面行宽（仍按旧宽度存放）不一致。本 Cycle 未重新复现。
- **影响面与触发条件**：触发条件 =「ALTER TABLE DROP COLUMN 的语句 future 被放弃」；后果是目录与数据面的列宽不一致，后续查询按窄化后的目录解释旧数据。
- **与本 change 范围的关系（为什么超出）**：DDL **目录变更的事务化**不在「取消/超时/事务收尾」的调用方语义面；Iteration 000 / 001 的 Non-goals 已明确列为「change 范围外的实质问题，只报告不登记」。
- **建议修复方向**：把 DDL 的目录变更纳入事务/回滚边界——要么窄化与数据面改写在同一可回滚单元内，要么窄化延后到语句确定完成之后。
- **建议的承载方式**：登记为 Issue，修复放在 **DDL/目录事务化**的后续里程碑（与 MS36「历史库与 dump 兼容」相邻——两者都触及目录变更与存储格式的边界），由 milestone-planner 排期；**不在 MS34 内修**。

### 未解决问题与停止条件

- **E1 / E2 / C1 未修复、未登记**（范围外，见上）。本 Cycle 只报告不登记不修复——登记由 Recorder 按用户指令执行，修复由后续 change 承担。
- **父 Cycle 的规避仍然只是规避**：2.3 的 20ms 窗口与 2.4 的 9001+ 偏移键回避了 E1 / E2 的暴露面，没有修复它们；E1 的触发条件在更大的窗口下依然成立，E2 的残留条目依然存在。本 Cycle 与本轮 repair 均未新增规避（2.4 的键与隔离级别未改）。
- **repair item `2.6-R2` 已完成**：GAP-2 涉及的那一处注释已修正为与实现和规格 delta 一致的准确陈述；全仓复核无「断开 = 取消」的残留表述；requirement / scenario 数量仍为 6 / 10；用例数仍为 8；无待办任务；未命中 Gate 5。**停止条件**：写入本 Act Response（状态 `reported`）后终止——不填写 Plan Review（`Review Result` 仍为 `pending`）、不创建下一 Cycle 或下一 Iteration、不归档 change、不同步全局 tasks / SNAPSHOT / M / R / I、不调用 Maintainer / Recorder。

## Plan Review

- Review Result: accepted

> 本节是**第三次 Review（最终）**：复审 repair item `2.6-R2` 的实施结果，并给出本 change 的最终结论。按「最新完整状态」覆盖上一版 Review（第二次）的正文；上一版的关键判定与 repair item 的定义在下文按结论复述（`Act Response` 中对「F3 / GAP-2」「repair item `2.6-R2`」「Plan Review 的 F1」的指代仍可在本节解析）。`Plan Context` 与 `Act Response` 未被本节改动。

**Findings**

**F1（已闭合；非阻塞）—— GAP-2 的修复逐字落地：测试文件与实现、规格 delta 同口径。**

- 文件唯一性（本次实跑）：`find . -name 'statement_cancellation_test.rs' -not -path './target/*'` → **只有一份**，仓库根 `tests/statement_cancellation_test.rs`（824 行）。**change 目录内不存在第二份副本**——上一版 Review 与上游交接指令里「可能还有一份」的前提不成立；Act 的路径更正属实，本 Review 独立复核通过。
- 改后文本（本次实读，`tests/statement_cancellation_test.rs:579-582`）：
  > 「超时是语句 future 被放弃的另一种来源（丢弃与超时在库入口是同一类事件；连接断开不是取消触发来源——语句照常执行完毕并提交，调用方得到的只是未送达的结果）。显式事务路径不因语句结束而收尾事务——超时到期后事务仍然活跃、行仍在事务内，只有调用方显式回滚才撤销它。」
- 与上一版 Review **规定的替换文本逐字一致**；与 `spec.md:11`（R1 正文）、`spec.md:13`（场景标题「丢弃与超时是同一类事件」）同口径；断开的准确陈述与实现一致（F3 的读码结论）。
- 改动性质（本次实读全文核对）：**仅注释**。断言、夹具（隔离级别、键取值范围、`ABANDONED_SQL`、`SOURCE_ROWS`、两个定时量）、用例数（8）、断言数（39）均未变；产品代码零改动（`git diff --stat` 仍为 4 个 `src/` 文件 +278 / -20）。
- 计数独立核对（本次实跑）：`grep -c '^### Requirement:'` = **6**、`grep -c '^#### Scenario:'` = **10**、`#[tokio::test]` = **8**、`assert!` / `assert_eq!` 行 = **39**——与 Act 报告一致。
- 全仓扫描（本次实跑，独立于 Act）：「断开 / 断连 / disconnect」在 `tests/` 命中 **1** 处（`:580`，即写入的准确陈述本身）；`src/` 命中 **1** 处（`src/network/connection.rs:33`——`disconnects` 描述连接任务自身的 semaphore permit 在 I/O 错误时释放，与取消触发来源无关，**Act 判定不改成立**）；`openspec/specs/` 全库 **0** 命中；change 目录内其余命中为准确陈述、已交接 Cycle 文件的历史记录（不可回改）与实施记录（事实性描述）。**没有任何一处再把断开列为取消触发来源。**

**F2（非阻塞）—— deviation 6 的裁定：Act 的取舍正确；本 Review 不再要求「0 命中」。**

- 上一版 Review 的验证条目同时要求「检索测试文件中的『断开』→ **0 命中**」并规定了**必然含「断开」一词**的替换文本——两条判据互斥，无法同时成立。
- Act 的处置：执行**规定的替换文本**（执行依据的权威版本），满足**实质判据**（无残留「断开 = 取消」的表述），实测命中 1 处且该处即准确陈述；未为凑 0 命中而扭曲措辞，并如实记录实测命中数。
- **裁定：正确。** 执行依据优先于与之冲突的派生计数判据；「完成声明基于真实运行」不允许为满足自相矛盾的判据而改写事实。本 Review 的判据（取代上一版）：**不得有任何一处把断开列为取消触发来源**；命中数本身不是判据。

**F3（非阻塞）—— 规格 delta 的最终可合并性：6 条 Requirement / 10 条 Scenario 的每一条行为性断言都已由本 change 的用例验收。**

- **R1**「取消触发集合是语句 future 被放弃」/ 场景「丢弃与超时是同一类事件」→ 1.1（丢弃）、2.3（超时）；「不新增公共 API」→ 新增方法全为 `pub(crate)` / 私有（`implicit_tx_guard`、`reclaim_abandoned_implicit_transactions`、`reclaim_implicit_tx`）+ `TransactionManager` 的 `abort_by_id` / `discard_tx_version_tables`（该类型变更前已有 `abort` / `commit_by_id` / `abort_cleanup_versions`，属内部机械面）；「不改变未被取消的语句的行为」→ 并发用例 `concurrent_statement_does_not_abort_other_statement_tx`。
- **R2**「提交前被取消的语句不提交」/ 场景 1 → 1.1（活跃集为空、回收早于下一条写语句）+ 2.4 的同一会话断言（`:776-786`：读到的行全部 `id > SOURCE_ROWS`）；场景 2 → 1.3 + 2.4（重开后被放弃语句的行不存在）；场景 3 → 1.1（活跃集为空、无悬空事务）；「回收早于下一次 checkpoint」→ 1.3；「页锁随 future 释放」→ 2.4（回收后同一会话对同表 `t2` 的后续写入成功）。
- **R3** → 2.1（判别位 `commit_returned` 确保放弃点落在提交完成之后；同会话读得到、重开仍在、活跃集为空、后续写成功）。
- **R4** / 三个场景 → 2.2 的 (a)(b)(c)（事务仍活跃可用且库不代替收尾 / 回滚后行不存在且事务结束 / 提交后持久且重开仍在）。
- **R5** → 2.3 的 (a)–(e)（超时放弃、事务仍活跃、行仍在事务内、回滚才撤销、提交则持久）。
- **R6** → 2.4（后续写入正常提交、重开后数据完整、`check_integrity` 0 error）；「语句级只读后台任务自然终止」→ 读码 `src/executor/data_scan.rs:177-187`（一次只读页读、结果丢弃、不写盘）。
- **已知边界注记（`spec.md:40`）准确**（本次逐句核对 + 读码）：「可能只落下 B 树条目而未落下反向映射」↔ `src/storage/btree/index_manager.rs:239-257`（正向条目在 `:244-248` 的 `spawn_blocking` 内写入，反向映射在 `:256` 的 await 之后才写）；「中止路径无法据此识别并移除该条目」↔ `src/transaction/manager.rs:344-355`（A/B 分桶只查 `find_key_by_row_id`，缺反向映射者被误判为墓碑 → 正向条目不被移除）；「行不可见（本 requirement 的验收面）」↔ 1.3 / 2.4 的断言；「键仍可能占用主键索引（再插同键被拒）」↔ E2 的实测证据（键 2 报 `DuplicateKey`，键 1 与 3..10 成功）。注记用「**可能**」，既不断言键一定可复用，也不断言键一定被占用 → **没有把未验收行为写成已验收** ✓。「由存储域的索引写侧契约承载」是前瞻引用（该契约尚未建立），沿用上一版 F8 的非阻塞 Minor 记录。
- **没有因收窄丢失已实现且已验收的行为**：收窄移出的只是「键的索引条目释放」——它只对被取消语句的**首个键**成立，不是可验收的域保证；行不可见、隐式事务回收、未提交数据不随 checkpoint 落盘三条全部保留且有直接断言；键释放的行为规格按 E2 的承载方式随存储域的后续 change 进入 `openspec/specs/`。
- **断开门径的独立验证**（本次实读）：`src/network/connection.rs:21-34` 的每连接处理器是顺序循环（`parse_request` → `execute(req).await` → `write_response`），语句执行期间没有观察 socket 存活的点；`src/parser/planner/mod.rs:437-441` 对 `BEGIN` / `COMMIT` / `ROLLBACK` 一律点名拒绝 → 网络路径的每条语句都是自动提交语句，故断开门径下「语句执行完毕并提交」成立，落在 R3 一侧。
- **结论：delta 可合并**（6 / 10，无残留无条件断言）。

**F4（非阻塞）—— 2163 / 0 / 2 的沿用成立；本轮未重跑全量。**

- 独立加总（本次实算，批次表来源：父 Cycle `000-initial.md:225-238`）：149+210+163+169+103+75+143+222+107+168+151+83 = **1743**；+ `--lib --bins` 批 **420** = **2163**；ignored **2**（批次 02）；failed 全 0；与基线 2155 + 本 change 新增 8 = 2163 吻合 ✓。
- 覆盖范围（本次实查 mtime）：产品代码 `src/database.rs` / `src/pipeline.rs` / `src/executor/alter_table.rs` / `src/transaction/manager.rs` = **12:02–12:26**，自 2.5 结论产生后未变；此后唯一的代码改动是 `tests/statement_cancellation_test.rs`（批次 10 的一个目标），已由 Act 在 R2 之后单独重跑（`168 passed / 0 failed`，与批次 10 的原计数逐目标一致：12+6+21+8+10+8+21+5+25+46+6 = 168）；change 目录内只有文档 → **其余 12 批的结论未失效**。
- 无 AGENTS.md「验证」章节列出的重跑情形命中（结论与代码不矛盾、被覆盖代码未变化、无失败、非同一问题第三次失败、非 rework / replan 受影响范围）→ **本轮未重跑任何测试命令**。

**F5（非阻塞）—— 无身份证明机制、无框架外判定层。**

- 全文件无 Hash / 指纹 / run-id / manifest / revision pin / peer-host pin / 时间顺序证明；判定全部在 `cargo test` 的 `assert!` / `assert_eq!` 内；观察点只有库入口返回的行集、`active_transactions()`、重开后的读与 `check_integrity`。
- 两个定时量都不是行为判据：`ANCHOR_TIMEOUT`（10s）只作夹具失控兜底（触发即 panic = 判失败）；`TIMEOUT_CANCEL_WINDOW`（20ms）只作**取消触发源**。
- 2.4 的同一会话断言**非空洞**（本次独立读码）：`setup()` 用 `RepeatableRead`（`tests/statement_cancellation_test.rs:55`），而 `src/database.rs:477-489` 的 `statement_snapshot` 在 RR 下返回 `None` → 扫描路径不做可见性过滤；若回收没有墓碑化被放弃语句的版本，同一会话读就会看到键 1..2000，断言即失败；「行集非空」前置断言再堵住空集真空通过。

**F6（非阻塞）—— Issue 候选 E1 / E2 / C1 的「可承载」格式复核通过；三条均只报告不登记。**

- 七要素齐备（一句话结论 / 根因位置（文件 + 符号 + 行）/ 可复现路径或已观察证据 / 影响面与触发条件 / 与本 change 范围的关系 / 建议修复方向 / 建议承载方式）→ 可直接交 Recorder。正文以当前 Cycle 的 `Act Response`「Experience Candidates」段为准（不在本节复制）。
- 根因位置本次独立读码核对（与报告一致）：
  - **E2**：`src/storage/btree/index_manager.rs:239-257`（正向条目 `spawn_blocking` 写入不可取消，反向映射 `:256` 在 await 之后才写）+ `src/transaction/manager.rs:344-355`（A/B 分桶只查反向映射）✓。
  - **E1**：`src/storage/btree/btree.rs:327`（内部节点下降递归 `insert_into_page`，其 `child_page_id_u32` 来自 `:319-323`，与 gdb 现场的 `child_page_id_u32 = 0` 对应）、`:243`（`load_page`；叶子/内部判定式实际在 `:248` 的 `data_guard[0] == LEAF_NODE`——**行号引用精度 Minor，不影响可承载性**）+ `src/storage/btree/index_manager.rs:261-282`（`delete` 的合并 / 重分布 / 换根）✓。
  - **C1**：`src/storage/data/table_manager.rs:792-802`（目录改写 + `update_table_column_count` + `flush_all` 先于任何数据 WAL 记录 → 窄化不在事务内）✓。
- 证据强度按原样保留：E1 为竞态依赖（6 次未命中）、E2 有父 Cycle 的实测 RED 与 Plan Review 的独立复现、C1 本 change 未重新复现（最短可观察步骤已由上一版 Review 补齐）。**只报告、不登记、不在本 change 修复。**

**F7（非阻塞 Minor，不返工）**

1. **键复用的无条件表述残留在两处非权威产物**：`proposal.md:12`（「……索引条目被还原；……（同一键可以再次插入）」）、`:29` / `:30`（场景草图 1 / 2 的「同一个键可以再次插入成功（索引条目已还原）」）；`tests/statement_cancellation_test.rs:199`、`:212-214`、`:241`（Iteration 000 的 1.1 / 1.3 用例注释）。这些表述在**被取消语句的首个键之外**不成立（E2）；但它们各自对应的断言（键 1 再插成功）在该用例里**为真**，不改变任何行为。判定为非阻塞的理由：① 2.6 的 Acceptance 只点名 `spec.md` / `tasks.md` / `design.md` 三处，这两处不在验收面；② 进入权威规格库的是 delta，delta 已收窄且带已知边界注记，无残留无条件断言；③ 缺陷的完整事实由 E2 承载（含「首个键之外不成立」的明确表述），change 的其余近期维护产物（`tasks.md` 任务 1.2 / 1.3 的 dated 注记、`design.md` D2 的边界注记、Iteration 000 的 Stable result 注记、2.2 / 2.3 / 2.4 用例注释）已采用收窄口径。处置：记录并交给收尾环节（归档前可选对齐），**不在本 change 返工**。
2. **Act Response 的行号引用不一致**：`:157` 写「修复后（`:579-581`，逐字）」，而 `:134` / `:331` 写「`:579-580` → `:579-582`」；实为 `:579-582`（4 行）。逐字文本本身正确，仅引用精度问题。
3. **上一版 Review 的文件行数引用**：上一版记「全文 823 行」，实测现为 824 行；测试文件未跟踪、无法用 git 复原 R2 之前的版本，但断言数（39）、用例数（8）、夹具与 R2 规定的替换文本均逐项核对一致，且 R2 只改注释 → **不影响 GAP-2 的闭合判定**。
4. **`README.md` 的状态行过时**：`:5` 仍写「Iteration 000……Cycle 000 `000-initial.md`，Plan Context `ready`，待 Act」，`:6` 仍写「Iteration 001……只在本 Iteration 的 Iteration 被接受后展开」——与实际（Iteration 000 已 `accepted` 且有两个 Cycle；Iteration 001 本轮 `accepted`）不符。归档前应更新。
5. **全局 `.agents/docs/tasks.md:122` 的 MS34-T01 行**仍把「断开」列为触发源（「约定库入口在丢弃、超时和断开后的事务、锁和句柄结果」）——全局文档属 docs-maintainer 的同步范围，本 Review 不改。

**Acceptance Gaps**

无。GAP-1 与 GAP-2 均已闭合：

- **GAP-1（关闭）**：规格 delta 的 R1 / Purpose / 场景与同源措辞（`design.md` D1、时序图 Note、`proposal.md`、`README.md`）不再把「断开」列为取消触发来源；补入的陈述与实现一致（F3 的读码）；requirement / scenario 仍为 6 / 10。分类：Plan 错误 + Act 偏离 + 新证据 → 已由 repair item `2.6-R1` 闭合。
- **GAP-2（关闭）**：`tests/statement_cancellation_test.rs` 的用例注释已改为与实现和规格 delta 一致的准确陈述；全仓无「断开 = 取消」的残留表述（F1）。分类：Plan 遗漏 + Act 偏离 → 已由 repair item `2.6-R2` 闭合。

**Evidence**

- **本轮独立阅读**：当前 Cycle 的 `Plan Context` + 最新 `Act Response` + 上一版 `Plan Review`；父 Cycle `000-initial.md`（Act Response 与 Plan Review，含 2.5 的分批明细表）；Iteration 000 的两个 Cycle（历史结论引用）；change 的 `proposal.md` / `design.md` / `README.md` / `tasks.md` / `specs/statement-cancellation-outcome/spec.md`（全文）；`tests/statement_cancellation_test.rs`（全文 824 行）；`git diff src/`（4 个文件 +278 / -20 全文读过）。
- **本轮实跑（检索与计数，非沿用）**：`find` 的文件唯一性；`grep -c` 的 Requirement 6 / Scenario 10 / 用例 8 / 断言 39；「断开 / 断连 / disconnect」与「同一类事件」的逐处检索（`tests/`、`src/`、`openspec/specs/`、change 目录、全局文档）；批次加总 1743 + 420 = 2163；关键文件 mtime（产品代码 12:02–12:26、测试文件 14:14、change 文档 14:00–14:16）；`git status --short` 与 `git diff --stat`。
- **本轮实读的实现事实（用于判定 delta 的陈述与 Issue 候选的根因）**：`src/network/connection.rs:18-35`、`src/parser/planner/mod.rs:437-441`、`src/executor/data_scan.rs:177-187`、`src/database.rs:477-489`、`src/storage/btree/index_manager.rs:239-282`、`src/storage/btree/btree.rs:238-249` 与 `:318-327`、`src/transaction/manager.rs:335-365`、`src/storage/data/table_manager.rs:788-807`。
- **沿用未失效的结论（来源：当前 Cycle 的 Act Response）**：整文件 `cargo test --test statement_cancellation_test -- --test-threads=1` → 退出码 0、**8 passed / 0 failed**（40.57s）；batch 10 的单次独立调用 → 退出码 0、**168 passed / 0 failed**；`cargo fmt --check` → 退出码 0。
- **沿用未失效的结论（来源：父 Cycle 的 Act Response）**：2.5 的 M19 分批串行全量 **2163 passed / 0 failed / 2 ignored**（13 批，逐批退出码 0，覆盖对账 missing 0 / extra 0 / duplicate 0）。
- **本轮未重跑任何测试命令**（理由见 F4）；**未创建临时探针或临时文件**；进入 Review 与退出 Review 的工作区状态一致（本次只写 change 目录内的文档）。
- Persisted Evidence 模式为 `none`，本 Cycle 未创建 `evidence/` 目录——按 Review 规则不因此提出问题。

**Follow-up Decision**

`accepted`（非 `rework-required`，非 `replan-required`，非当前 Cycle 修复）。

- **GAP-1 / GAP-2 均已闭合**，闭合方式经本轮独立复核（读码 + 逐字比对 + 计数 + 全仓检索），不是采信 Act 自述。
- **不走 `rework-required`**：既有 Acceptance（2.6 的 (a)(b)(c)(d) 与 2.1–2.5）全部满足；剩余发现（F7）全是不阻塞 Acceptance 的 Minor finding——按 `references/iteration-planning.md`「只有不阻塞 Acceptance 的 Minor finding → `accepted`」，不为表面问题创建返工 Cycle。
- **不走 `replan-required`**：目标、范围、依赖、requirement 的行为语义、验收边界与验证方式均未改变。
- **不留在当前 Cycle 修复**：当前 Cycle 无待修项。
- **不以 Act 自评代替独立检查**：本轮独立读了完整 diff、测试文件全文、delta 全文与相关实现代码，独立核对了 Issue 候选的根因位置与行号，独立复算了 2163 / 0 / 2，独立裁定了 deviation 6。

**Iteration Plan：保持不变**

任务集合（1.1–1.3、2.1–2.6）、依赖、Stable result、Verification boundary、Diagnostic boundary、Non-goals 均不调整；Iteration 001 仍是最后一个 Iteration。

**Next Cycle**

None——当前 Cycle 获 `accepted` 后不再创建后继 Cycle；`Plan Context` 保持不可改写，本 Cycle 冻结。

**Next Iteration**

None——Iteration 001 是本 change 的最后一个 Iteration。change 的 **Iteration Map 终态**：Iteration 000 `accepted`（Cycle 000 `rework-required` → Cycle 001 `accepted`）+ Iteration 001 `accepted`（Cycle 000 `replan-required` → Cycle 001 `accepted`）。

**收尾交接（供上游派发 openspec-docs-maintainer；本 Review 不执行）**

本 Review 判 `accepted`，本 change **可以收尾**。以下均属 docs-maintainer 的收尾职责，不在 Plan / Act 的写入面：

1. **合并规格 delta**：把 `openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/specs/statement-cancellation-outcome/spec.md`（6 Requirement / 10 Scenario，含 R2 的已知边界注记）合并进权威规格库 `openspec/specs/`（新域 `statement-cancellation-outcome`；语料库 72 → 73 域）。合并前确认与既有 72 域无冲突——本次已核 `openspec/specs/` 全库无「断开 / 断连 / disconnect」命中。
2. **同步全局状态**：`.agents/docs/tasks.md`（MS34 → completed；MS35 / MS36 仍为 planned；**并把 `:122` 的 MS34-T01 行「丢弃、超时和断开」按收窄口径改为「丢弃、超时」**，注明断开不是取消触发来源）、`.agents/docs/SNAPSHOT.md`、`project-model.md` 的 M、`references.md` 的 R、`improvements.md` 的 I（I090 已 promoted）。
3. **登记 Issue 候选**（由 openspec-experience-recorder 按用户指令执行，`.agents/issues/`）：E1 + E2 合并为「索引写侧原子性与取消后一致性」主题（修复落在存储/索引侧的后续里程碑，建议与 MS35「空闲页跨重启复用」相邻排期——两段在 `src/storage/` 的改动面可能重叠，须由 milestone-planner 统一排期）；C1 承载于「DDL 目录事务化」主题（建议与 MS36 相邻）。**不在 MS34 内修。**
4. **归档本 change**（`openspec archive`）。归档前可选对齐两处非权威产物的措辞：F7-1（`proposal.md:12/:29/:30` 与 `tests/statement_cancellation_test.rs:199/:212-214/:241` 的键复用表述）、F7-4（`README.md:5-6` 的状态行）。
5. **本 Review 明确未做**：未修改 `openspec/specs/`、未归档 change、未同步全局 tasks / SNAPSHOT / M / R / I、未改产品代码与测试文件、未回滚既有改动、未提交、未登记 Issue、未创建后继 Cycle / 后继 Iteration。
