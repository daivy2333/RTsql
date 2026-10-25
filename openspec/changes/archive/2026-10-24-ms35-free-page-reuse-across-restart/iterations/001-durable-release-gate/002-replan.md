# Iteration 001 / Cycle 002: replan——规格增量场景与验收边界对齐（崩溃点固定在 DDL 边界之前）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T8
- Acceptance gaps: 父 Cycle（`000-initial.md`）的 A3「崩溃前创建的新表不存在」不可与「旧表仍存在」同时成立——`CREATE TABLE` 的语句边界有序落盘会把后续 DDL 的提交点与其它脏页一并落盘，先前的 `DROP TABLE` 目录删除因而随之落盘。该项使父 Cycle 的 A3 未被满足，并使本 change 的两份规格增量（`free-page-lifecycle` R4 场景、`drop-persistence-boundary` R1（MODIFIED）场景）记录了与实现相反的可观察量。本 Cycle 关闭该缺口：把崩溃点固定到「释放之后、任何后续 DDL 之前」，并把 DPB R1（MODIFIED）的前置条件补齐。
- Repair items: None（replan 不使用 repair item）
- Inherited scope:
  - 父 Cycle 已实现并经其 Act Response 验证的释放持久化闸门（工作区现状，本 Cycle 不改）：`FileStorage` 的待释放集合与释放完成编号、`complete_free_release(&[PageId])` / `release_completion_limit()` / `promote_free_releases(limit)` / `discard_free_releases(limit)` 四个带默认实现的 trait 方法、`CheckpointManager::checkpoint` 第 2 步前取样 / 成功提升 / 失败丢弃、第 0 步零工作早退的提升、16 类释放点的完成标记。
  - 父 Cycle 的验证结论**直接沿用**（来源：父 Cycle Act Response 的「Gate 3 / Gate 4 结论」与「Verification Evidence」节；被覆盖的产品代码在本 Cycle 不发生变化）：`cargo test --test free_page_lifecycle_test -- --test-threads=1` → 12 passed / 0 failed；`drop_persistence_test` 11、`drop_table_free_test` 6、`overflow_page_test` 27、lib 单测 2 处全 0 failed；T7 分批全量 14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored。
  - 有效 requirement（**正文不变**）：`free-page-lifecycle`（FPL）R1–R4、`drop-table-physical-free`（DTPF）R（MODIFIED）两条、`drop-persistence-boundary`（DPB）R1（MODIFIED）。
  - 有效约束：M19 分批串行回归纪律；M20 发现方只报告移交；MS35 不改文件格式、不加伴生文件；验证场景只用小表（避开 ISS23）。
- Excluded scope:
  - **产品代码的任何改动**——父 Cycle 的实现与其验证结论保持不变。
  - 未完成释放的提前提升面（DDL 边界刷盘失败后锚点写丢失、后续一次成功刷盘仍提升该释放）：父 Cycle Review 判为非阻塞 Minor finding + Issue 候选，按 M20 移交，不在本 Cycle 修。
  - B 树叶合并 / 子合并路径对同一页的二次释放：既有问题（本次改动未改变其行为），按 M20 作为 Issue 候选移交。
  - 释放路径的有界读取改造（损坏页上的 DROP / TRUNCATE / DROP INDEX panic 面）：父 Cycle 已判为范围外 Issue 候选。
  - 尾部截断与整库整理；无键行旧版本 GC（I038）；文件格式与伴生文件变更（MS36）；ISS22 / ISS23 / ISS24 / ISS25 的修复；`TRUNCATE` 自身的既有崩溃语义；其它 scenario 的文本。
  - 本 change 的收尾（把三份规格增量合并进 `openspec/specs/`、同步全局 tasks / SNAPSHOT / M / R / I、归档 change）——`openspec-docs-maintainer` 的职责，本 Cycle 不做。

**Objective**

本 change 的规格增量只承载已验收行为：两份崩溃场景的崩溃点固定在「`DROP TABLE` 成功返回之后、任何后续 DDL 语句成功返回之前」，其 THEN 逐条对应 `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` 的可观察量；`drop-persistence-boundary` R1（MODIFIED）的 requirement 正文补上「也没有其它 DDL 语句成功返回」的前置条件，使该要求不再覆盖「后续 DDL 的有序落盘已把目录删除写盘」的形态。⇒ 规格库合并后不承载任何与实现相反的断言。

**Background**

- 父 Cycle（`iterations/001-durable-release-gate/000-initial.md`）的实现与验证结论成立：T4–T7 全部 done，T7 分批全量 14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored。
- 父 Cycle 的 Plan Context 的 A3 写入了三项崩溃可观察量：「旧表仍存在且已提交行可查询、**崩溃前创建的新表不存在**、`check_integrity` 的 error 级发现数为 0」。其中第二项与第一项不可同时成立：DDL 的语句边界有序落盘（`src/storage/catalog.rs` 的 `flush_commit_point` → `buffer_pool.flush_all_except`）会写入除提交页外的**全部**脏页，因此 `DROP TABLE` 之后同一会话的 `CREATE TABLE` 会把先前的目录删除一并写盘。父 Cycle 的 Act 发现该冲突后，把用例改以「未完成的释放不是可分配页」承载 R4（本 Cycle 沿用该可观察量）。
- 本 Cycle 的成因（父 Cycle 的 Plan Review 独立核对，结论已写入父 Cycle 的 `Plan Review`）：两份规格增量的场景文本仍写着父 Cycle 之前的旧观察量（「随后同一会话创建新表 `u` 并分配页」+「`u` 不存在（其目录未持久化）」+「重新打开后 `t` 仍存在」）。该文本在本 change 的实现下不可成立，若原样合并进 `openspec/specs/`，规格库会记录实现并不具备的行为（「规格库只保存验收过的行为」）。修正场景文本与 DPB R1 的前置条件属于「验收边界需要改变」，因此走 replan 而非普通返工。
- 用户指令（goal）：「把已经规划的三个 ms 逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」；补充 2：「实施当中发现的所有issue候选，都直接寻找好的承载方式流程进行修复而不是等级」。本 Cycle 是 MS35 收尾前的最后一次 replan。

**Investigation Facts**

- Starting point:
  - 工作区：HEAD `0a08b14`（ahead of origin 9）；MS34 + issue-fix change + MS35 两个 Iteration 的改动**全部未提交**——属既有现场，不要回滚、不要清理、不要提交。
  - 父 Cycle 的实现结论与验证结论沿用（来源：父 Cycle Act Response；被覆盖的产品代码在本 Cycle 不变）：释放持久化闸门成立（16 类释放点的完成标记、第 2 步取样 / 成功提升 / 失败丢弃、第 0 步零工作早退提升）；`free_page_lifecycle_test.rs` 12 例全绿；T7 全量 2183 passed / 0 failed / 2 ignored。
  - 父 Cycle 的 Plan Review 结论：实现与 15 处断言更新均成立，7 条 deviation 逐条裁定通过（详见父 Cycle `Plan Review`）；唯一阻塞项是规格增量场景文本与验收边界（A3）的不一致 ⇒ 本 Cycle 处理。
- Current-State Evidence:
  - **DDL 语句边界有序落盘**：`src/storage/catalog.rs:226` `insert_table` → `:259` `flush_commit_point(&commit_pages)` → `:271` `buffer_pool.flush_all_except(&exclude)`（Phase 1 收集并清脏标记、Phase 2 逐页写盘）→ 除提交页外的**全部**脏页被写盘；随后 `:272` `flush_pages(commit_pages)` 写提交页。因此 `DROP TABLE` 之后同一会话的任一 `CREATE TABLE` 会把先前的目录删除一并写盘。
  - **`DROP TABLE` 的目录删除只在缓冲池**：`src/storage/catalog.rs:787` `delete_from_chain` 经 `bp.get_page()` + `modify_page()` 删除目录行（页变脏），`delete_table` **不**调用 `flush_commit_point` ⇒ `DROP TABLE` 语句返回时目录删除尚未落盘。
  - **既有规格依据**：`openspec/specs/wal-recovery-replay-integrity/spec.md` 的「DDL 语句边界 catalog 有序落盘」要求——`CREATE TABLE` 等 DDL 的 catalog 效果在语句成功返回前有序落盘（提交页最后落盘）；`DROP TABLE` / `DROP INDEX` / `DROP VIEW` / `TRUNCATE` 保持 checkpoint 边界，不在语句成功返回前把 catalog 删除当成 commit point。
  - **独立核对结论**（父 Cycle Plan Review 的一次性探针，探针文件已删除、工作区已复原到 Review 进入时的状态）：`DROP t → CREATE u → 崩溃` ⇒ 重开后 **`t` 不存在、`u` 存在**；`DROP t → 崩溃`（其后无 DDL）⇒ 重开后 **`t` 存在**。前者否定了两份增量场景的旧文本，后者证实了修正后的崩溃点形态。
  - **用例现状**：`tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows`（会话一建 `t` 并写入已提交行后 `close`；会话二 `DROP TABLE t` 后按 R49 崩溃；重开后断言 `t` 的行可查询、释放的页不被新表占用、`check_integrity` error 计数 0）——**没有 `other` 表**，修正后的 FPL R4 场景要求 `other` 的行可查询。
- Code and Critical Path:
  - 本 Cycle 的改动面只有两处：`tests/free_page_lifecycle_test.rs` 的一个用例（新增 `other` 表覆盖 + 注释对齐）与两份规格增量文本的核对。规格增量的文本修正已由 Plan 在创建本 replan Cycle 时完成（属 replan 的计划更新，见 `Iteration Plan Update`），Act 只需核对并把用例与修正后的场景逐条对齐。
  - 不涉及 `src/` 的任何符号；不涉及 checkpoint / 待释放集合 / 16 类释放点标记。

**Implementation Guidance**

1. **只加不减**：给崩溃用例补 `other` 表（会话一建表并写入已提交行），崩溃后重开断言 `other` 的行可查询；`t` 的行可查询、释放页不被占用、`check_integrity` error 计数 0 三条既有断言原样保留。
2. **注释对齐**：用例的文档注释改为引用修正后的场景文本，并写明崩溃点为何取在「释放之后、任何后续 DDL 之前」（DDL 语句边界有序落盘会把后续 DDL 的提交点与其它脏页一并落盘）。
3. **规格增量只核对不重写**：两份增量文本已由 Plan 改写（`free-page-lifecycle` R4 场景与 `drop-persistence-boundary` R1（MODIFIED）的正文与场景）。Act 逐条核对「场景 THEN ↔ 用例断言」，若发现不一致，返回 Plan，不得自行改写 requirement 正文。
4. **不重跑全量**：产品代码未变 ⇒ 父 Cycle 的 T7 结论沿用（依据：公共规则 › 验证「被覆盖的代码在结论产生后发生变化」不成立——本 Cycle 不改 `src/`）；本 Cycle 唯一变更的测试文件由本 Cycle 的定向命令覆盖。

**Task Definitions**

### T8: 崩溃用例与修正后的场景对齐（`other` 表覆盖 + 场景引用）

- Requirement/Scenario: FPL R4 场景（修正后「释放未持久时崩溃后旧表内容完整」）；DPB R1（MODIFIED）场景（修正后「释放未持久时的分配不摧毁旧表内容」）与 R1（MODIFIED）的 requirement 正文（新增前置条件「也没有其它 DDL 语句成功返回」）
- Depends on: None
- Targets: `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows`；`openspec/changes/2026-10-24-ms35-free-page-reuse-across-restart/specs/free-page-lifecycle/spec.md`（R4 场景，核对）；`openspec/changes/2026-10-24-ms35-free-page-reuse-across-restart/specs/drop-persistence-boundary/spec.md`（R1 MODIFIED 正文与场景，核对）
- Current behavior: 该用例覆盖「`t` 的已提交行在崩溃后仍可查询」「未完成的释放所涉及的页不是可分配页（重开后的新表不占用 `t` 的数据页与索引树页）」「`check_integrity` 的 error 级发现数为 0」；**不覆盖**修正后场景要求的 `other` 的行可查询；用例注释仍引用父 Cycle 之前的观察量组合。
- Required behavior: 会话一在 `t` 之外再建 `other` 并写入已提交行；崩溃后重开断言 `other` 的行可查询（既有断言只增不减）；用例注释引用修正后的两个场景文本与崩溃点的理由。
- Preserve: `t` 的行可查询断言、释放页不被占用的两条断言、`check_integrity` error 计数 0 断言；崩溃配方（R49：`wal_buffer.shutdown()` 后 `drop(db)`，**不调用 `close()`**）；小表规模（避开 ISS23）；两份增量文件的 requirement 正文与 R1–R4 / DTPF 的全部 requirement 正文；父 Cycle 的产品代码。
- Forbidden: 不得修改 `src/` 下任何文件；不得弱化、删除或跳过既有断言；不得改写任何 requirement 正文；不得引入判定脚本、封装命令、专用参数、timer 阈值或任何身份证明机制；不得在本 Cycle 内重跑全量回归。
- First test: 变更前 `cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ GREEN（当前 12 例全绿）。本任务是**覆盖补强 + 文本对齐**，断言的是既有行为而非新行为，故属 TDD 的「变更前 GREEN」分支；不得为此制造 RED。
- GREEN condition: 12 例仍全绿（用例数不变、断言数不减）；两份增量的场景 THEN 与用例断言逐条对应。
- Verification: `cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 退出码 0 且 `test result:` 行 failed 计数 0、passed 计数 12；外加人工核对（最短步骤）：读两份增量文件的两个场景与 DPB R1（MODIFIED）正文，再读该用例的断言，逐条比对可观察量，所见即判定。
- Stop when: 需要改写 requirement 正文、需要改动产品代码，或两份增量的场景与用例断言出现无法用文本对齐解决的冲突时，返回 Plan。

**Invariants**

- 产品代码零改动：父 Cycle 的实现与其全部验证结论不变（`src/` 变更集与父 Cycle 结束时一致）。
- 父 Cycle 的 T7 全量结论沿用：14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored（来源：父 Cycle Act Response；被覆盖的产品代码未变化；本 Cycle 唯一变更的测试文件由本 Cycle 的定向命令覆盖）。
- 规格库只承载已验收行为：两份增量场景的每条 THEN 都必须能对应到用例中的一条断言；不得出现「未被用例验收」或「与实现相反」的场景文本。
- 需求正文（requirement 正文与 Purpose）不变：本次修正只涉及场景实例的可观察量表述与 DPB R1（MODIFIED）前置条件的一个限定词。
- `AsyncStorage` 的必实现方法集合不变；`MemoryStorage` 与测试替身零改动；文件格式与伴生文件不变。

**Non-goals**

- 产品代码的任何改动（释放闸门机制、零工作早退提升、16 类释放点的完成标记）。
- 未完成释放的提前提升面与 B 树二次释放——按 M20 作为 Issue 候选移交，不在本 Cycle 修。
- 释放路径的有界读取改造；ISS22–ISS25 的修复；MS36；`TRUNCATE` 自身的既有崩溃语义。
- 本 change 的收尾动作（合并规格增量进 `openspec/specs/`、同步全局 tasks / SNAPSHOT / M / R / I、归档 change）——由 `openspec-docs-maintainer` 执行。

**Acceptance**

- **A8（场景文本与已验收行为一致）**：两份增量文件的崩溃场景——`free-page-lifecycle` R4 场景与 `drop-persistence-boundary` R1（MODIFIED）场景「释放未持久时的分配不摧毁旧表内容」——的 GIVEN / WHEN / THEN 与 `crash_with_pending_release_keeps_old_table_rows` 的可观察量逐条对应；DPB R1（MODIFIED）的 requirement 正文含「也没有其它 DDL 语句成功返回」的前置条件。⇒ 关闭父 Cycle 的 A3 缺口。
- **A9（用例覆盖补强）**：该用例覆盖 `other`（已提交行在崩溃后重开可查询）；`t` 的行可查询、释放页不被占用、`check_integrity` error 计数 0 三条既有断言仍在。
- **A10（定向验证）**：`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 12 passed / 0 failed。
- **A11（产品代码零改动）**：`git diff --stat -- src/` 与父 Cycle 结束时的 `src/` 变更集一致（本 Cycle 不新增 `src/` 变更）。

**Verification**

| Scenario | 判定方式 | 通过依据 |
|---|---|---|
| A8 | 人工核对（最短步骤）：读 `specs/free-page-lifecycle/spec.md` 的 R4 场景、`specs/drop-persistence-boundary/spec.md` 的 R1（MODIFIED）正文与「释放未持久时的分配不摧毁旧表内容」场景，再读 `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` 的断言，逐条比对 | 场景每条 THEN 有对应断言；DPB R1（MODIFIED）正文含「也没有其它 DDL 语句成功返回」；场景 WHEN 的崩溃点在「`DROP TABLE` 之后、任何后续 DDL 之前」 |
| A9 | 同上命令的输出（用例名 + 断言） | 用例含 `other` 的行可查询断言；三条既有断言仍在 |
| A10 | `cargo test --test free_page_lifecycle_test -- --test-threads=1` | 退出码 0；`test result:` 行 failed 计数 0、passed 计数 12 |
| A11 | `git diff --stat -- src/` | 与父 Cycle 结束时的 `src/` 变更集一致（无新增条目） |

全部判定落在 `cargo test` 的断言内或对文本的直接核对；不引入判定层、timer 阈值或身份证明机制。

**Gate 2 Readiness**

PASS —— Step 5 计划审查：PASS（目标、范围、依赖、requirement、验证方式与验收边界的**唯一**变化是崩溃场景的可观察量表述与 DPB R1（MODIFIED）的一个前置条件限定词；其余维度沿用父 Cycle 且材料未变，引用父 Cycle Gate 2 的 PASS 结论，只补 Acceptance gap 涉及的维度）；RTM：无 `Missing`、无 `Simplified`（requirement → task → 测试的映射不变，修正的是场景实例文本，矩阵已在 `tasks.md` 的「场景文本修正说明」中同步）；设计闭合：PASS（不改 `design.md`——D2.3–D2.7 的机制与本次修正无交互；D2.6 的「唯一行为回退面」声明仍成立）；与 change 产物一致：PASS（`tasks.md` 的 T8 与 Iteration 001 Plan、两份增量文本已同步）；Persisted Evidence 模式：`none`（判定由 `cargo test` 与文本直接核对承载，可低成本重跑）；用户批准：PASS（change 计划已在 Gate 1 获批；本 Cycle 的推进授权来自 goal 指令对阶段推进的显式授权）。无 `BLOCKED`、无 `SKIPPED`。

**Persisted Evidence**

- Mode: none

理由：验收结论由 `cargo test` 的真实运行与对规格文本的直接核对承载，可低成本重跑；无「结果无法低成本复现 / 一次性环境即将消失 / 需保留现场 / 摘要会丢失决定性结构」情形。不创建 `evidence/001-durable-release-gate/002-replan/` 目录。

**Risks and Notes**

- **父 Cycle 移交、本 Cycle 不修的项**：(1) 未完成释放在「DDL 边界刷盘失败 → 锚点写丢失 → 后续一次成功刷盘仍提升」路径上的提前提升面（父 Cycle Review 的 Minor finding + Issue 候选）；(2) B 树叶合并 / 子合并路径对同一页的二次释放（既有缺陷，本次改动未改变其行为）；(3) 释放路径的有界读取改造。三项按 M20 移交，由上游按用户补充 2 选择承载 change。
- **既有现场**：工作区含 MS34、issue-fix change 与 MS35 两个 Iteration 的未提交改动，属既有现场，不得回滚、清理或提交。
- **非实质未知项**：无（文本与用例的对应关系已在本 Cycle 的 Current-State Evidence 中逐条给出，Act 无需再作设计判断）。
- **收尾交接**：本 Cycle 获得 `accepted` 后，本 change 的最后一个 Iteration 即完成；合并三份规格增量进 `openspec/specs/`、同步全局 tasks / SNAPSHOT / M / R / I、归档 change 由 `openspec-docs-maintainer` 执行（不在本 Cycle 内）。

## Act Response

- Status: reported

### Implemented

#### T8 —— 崩溃用例与修正后的场景对齐（`other` 表覆盖 + 场景引用）

本 Cycle 的改动面 = **1 个测试文件**（Task Definition 的 Targets 明列）+ change 的 `tasks.md` 任务状态；**`src/` 零改动**（产品代码零改动不变式）。两份规格增量按 Plan Context 的实现指导 3 **只核对不重写**（Plan 已在创建本 Cycle 时完成场景文本修正），本次未改动其任何字符。

| 位置 | 改动 |
|---|---|
| `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows`（文档注释，`:433`–`:444`） | 改为引用修正后的两个场景文本（`free-page-lifecycle` R4 场景「释放未持久时崩溃后旧表内容完整」与 `drop-persistence-boundary` R1（MODIFIED）场景「释放未持久时的分配不摧毁旧表内容」），并写明崩溃点为何取在「`DROP TABLE` 成功返回之后、任何后续 DDL 语句成功返回之前」：DDL 语句边界的有序刷盘（`src/storage/catalog.rs` 的 `flush_commit_point` → `flush_all_except`）会把该语句的提交点与其它脏页一并落盘，先前的 `DROP TABLE` 目录删除因而随之落盘——该形态下重开后旧表不再存在，不属于本场景的崩溃点 |
| 同上（会话一，`:456`–`:458`） | 在 `t` 之外再建活表 `other` 并写入已提交行（`INSERT INTO other VALUES (1, 100)`），随会话一的 `close()` 一并落盘（WAL 已截断） |
| 同上（重开后，`:482`–`:487`） | **新增**断言：`other` 的已提交行可查询（`SELECT id, n FROM other` ⇒ `[[1, 100]]`）；既有的 `t` 行可查询、释放页不被占用（数据页 + 索引树页两条）、`check_integrity` error 计数 0 三条断言**原样保留**、未弱化、未删除 |
| `openspec/changes/2026-10-24-ms35-free-page-reuse-across-restart/tasks.md` | T8 任务状态 `pending` → `done`（change 结构自检：tasks 状态与实际完成一致）。**RTM 表格未改动**（requirement → task → test 映射不变） |

用例数不变（12 例）、断言数只增不减（+1 条 `other` 行可查询断言），崩溃配方不变（R49：`wal_buffer.shutdown()` 后 `drop(db)`，**不调用 `close()`**），表规模仍为小表（避开 ISS23）。

#### A8 核对：两份增量场景 ↔ 用例断言（逐条）

| 场景（增量文件） | GIVEN / WHEN / THEN | 用例中的对应断言（`tests/free_page_lifecycle_test.rs`） |
|---|---|---|
| `specs/free-page-lifecycle/spec.md` R4 场景「释放未持久时崩溃后旧表内容完整」（`:74`–`:80`） | GIVEN `t` 有已提交行（已落盘且 WAL 已截断），`other` 有已提交行 | 会话一建 `t` + `other` 并写入已提交行后 `close()`（落盘 + 截断 WAL）`:452`–`:464` |
| 同上 | WHEN `DROP TABLE t` 成功返回（未 checkpoint），此后没有其它 DDL 语句成功返回，再丢弃句柄崩溃（不调用 `close`） | 会话二仅执行 `DROP TABLE t` 后 `crash(db)`（不 `close()`），此后无任何 DDL `:466`–`:471` |
| 同上 | THEN 重新打开后 `t` 仍存在且原行可查询 | `t` 的行断言（`[[1,10],[2,20]]`）`:474`–`:481` |
| 同上 | AND `other` 的行可查询 | **`other` 的行断言（`[[1,100]]`）`:482`–`:487`（本次新增）** |
| 同上 | AND 未完成持久化的释放所涉及的页不是可分配页（新表不占用 `t` 的数据页与索引树页） | 新表 `v` 的数据页 ≠ `t` 数据页、索引树根 ≠ `t` 索引树根 `:488`–`:498` |
| 同上 | AND `check_integrity` 的 error 级发现数为 0 | `report.error_count() == 0` `:499`–`:511` |
| `specs/drop-persistence-boundary/spec.md` R1（MODIFIED）场景「释放未持久时的分配不摧毁旧表内容」（`:25`–`:31`） | GIVEN `t` 有已提交行（已落盘且 WAL 已截断） | 同上会话一 `:452`–`:464` |
| 同上 | WHEN 同 FPL R4 场景（崩溃点在 `DROP TABLE` 之后、任何后续 DDL 之前） | 同上会话二 `:466`–`:471` |
| 同上 | THEN 重新打开后 `t` 仍存在且原行可查询 | `t` 的行断言 `:474`–`:481` |
| 同上 | AND 未完成持久化的释放所涉及的页不是可分配页（新表不占用 `t` 的数据页与索引树页，**`t` 的磁盘页内容保持原样**） | 两条页号断言 `:488`–`:498`；「磁盘页内容保持原样」由 `t` 的原行在重开后仍逐行可读承载（页未被清零）`:474`–`:481` |
| 同上 | AND `check_integrity` 的 error 级发现数为 0 | `report.error_count() == 0` `:499`–`:511` |
| `specs/drop-persistence-boundary/spec.md` R1（MODIFIED）requirement 正文（`:5`） | 前置条件含「**也没有其它 DDL 语句成功返回**」 | 文本核对：正文含「…也没有页分配复用其释放的页、也没有其它 DDL 语句成功返回时…」 |

⇒ 两条增量场景的每条 GIVEN / WHEN / THEN 均能对应到用例中的一条断言；DPB R1（MODIFIED）requirement 正文含新增前置条件 ⇒ **A8 成立，父 Cycle 的 A3 缺口（崩溃可观察组合）关闭**。

### Deviations from Plan

None —— T8 按 `002-replan.md` 的 Task Definition 执行：改动限于其 Targets 明列的测试文件与 change 的 `tasks.md` 状态；两份增量只核对不重写；`src/` 零改动；未重跑全量。

### Blocker Handoff

None required.

### Blocker Resolution

None（未发生阻塞）。

### Self-Review

**Gate 3（逐任务即时 Review）**

| 任务 | 结论 | 依据 |
|---|---|---|
| T8 | PASS | Spec compliance：两条增量场景的 GIVEN/WHEN/THEN 与用例断言逐条对应（见上表），`other` 覆盖已补齐且三条既有断言原样保留、只增不减；崩溃点、崩溃配方、小表规模、`src/` 零改动、不引入判定脚本/专用参数/timer 阈值/身份证明机制——均符合 Task Definition 的 Preserve 与 Forbidden。Code quality：diff 仅 1 个测试文件的注释与 2 处语句增量，无计划外修改、无新增警告；`other` 断言不因错误原因通过（表缺失时 `rows()` 直接 panic，内容不符时等值断言失败） |

**自检发现与处理**

- 发现 1（已处理）：新增 `other` 会给库增加一张活表 ⇒ 复核其对既有「释放页不被占用」断言的影响——`other` 的页是活页（不在可分配集合内），`t` 的被释放页判定不受影响；实测 12 例全绿，断言语义不变。
- 发现 2（已处理）：`other` 的行必须来自磁盘而非 WAL 重放 ⇒ 沿用会话一 `close()`（checkpoint + 截断 WAL）的既有形态，断言的判别力落在「崩溃后未被触及的活表内容仍完整」。
- 遗留 Minor（不阻塞，**范围外未改**）：`design.md` D4 的第二条从句「崩溃前分配方语句的目录未落盘 ⇒ 分配方对象在重开后不存在」所描述的窗口在 MS35 闸门下已不可达（`CREATE TABLE` 的目录在语句边界必落盘，旧表因此不再存在）。T8 的 Targets 不含 `design.md`，且 Plan 的设计闭合结论为「不改 `design.md`」⇒ 本 Cycle 不改，作为 Minor 记录移交（承载方式：由 Plan 或 change 收尾时决定是否收敛该句措辞）。
- 遗留 Minor（不阻塞，**按规则不改**）：父 Cycle 的 `000-initial.md`（其 Plan Context 的 A3「崩溃前创建的新表不存在」等历史措辞）保持原样——「Plan Context 始终不可改写」且已交接的 Cycle 文件不得回改；该缺口的裁定与取代由父 Cycle 的 `Plan Review`（`replan-required`）与本 Cycle 的 Plan Context 承载。

**Gate 4（Task Definition 的验证逐项）**

| 验证 | 结论 |
|---|---|
| A8 场景文本与已验收行为一致（人工核对：读两份增量的 R4 场景、DPB R1（MODIFIED）正文与场景，再读用例断言，逐条比对） | ✅ 见上表；DPB R1（MODIFIED）正文含「也没有其它 DDL 语句成功返回」；两条场景 WHEN 的崩溃点均在「`DROP TABLE` 之后、任何后续 DDL 之前」 |
| A9 用例覆盖补强（`other` 的行在重开后可查询 + 三条既有断言仍在） | ✅ `other` 断言已加（`:482`–`:487`）；`t` 行可查询、释放页不被占用（两条）、`check_integrity` error 计数 0 三条原样保留 |
| A10 `cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 12 passed / 0 failed | ✅ 退出码 0；`test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| A11 产品代码零改动（`git diff --stat -- src/` 与父 Cycle 结束时一致） | ✅ 改动前后同一结果：`20 files changed, 1258 insertions(+), 164 deletions(-)`，无新增条目；`git status --porcelain -- src/` 亦无新增文件 |

### Verification Evidence

- **定向验证**（本次实际运行）：`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ 退出码 **0**；输出 `running 12 tests` / `test crash_with_pending_release_keeps_old_table_rows ... ok` / `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s`。
- **A11 核对**：`git diff --stat -- src/` 在本次改动前后均为 `20 files changed, 1258 insertions(+), 164 deletions(-)`（`src/` 20 个已改文件 + 未跟踪的 `src/storage/free_page_rebuild.rs`，与父 Cycle 结束时一致）。
- **数量核对（requirement / scenario 未变）**：`free-page-lifecycle` 4 requirement / 7 scenario（R1:2、R2:3、R3:1、R4:1）；`drop-persistence-boundary` 1 requirement / 4 scenario；`drop-table-physical-free` 2 requirement / 5 scenario —— 与父 Cycle `Plan Review` 记录的数量清单（R1（场景 1/2）、R2（3 场景）、R3（1 场景）、R4（1 场景）；DPB 的三个既有场景 + 一个修正场景；DTPF 2 条 requirement / 5 个场景）逐项一致。本 Cycle 未增删任何 requirement 或 scenario。
- **RTM 映射核对**：`tasks.md` 的 Requirements Traceability Matrix 表格**未改动**——FPL R1→T4,T5、R2→T1,T2,T3、R3→T1,T3、R4→T4,T5,T6、DTPF 两条、DPB R1→T4,T5,T6 及其 First Test 指向的用例名全部保持原样（矩阵后的「场景文本修正说明」段落亦未改动）；本次只改 T8 任务行末的状态单元格 `pending`→`done`。
- **全文措辞扫描**：在 change 目录内检索旧观察量措辞（`目录未持久化` / `同一会话创建新表 \`u\`` / `随后同一会话` / `\`u\` 不存在` / `新表不存在` / `崩溃前创建的新表`）——命中项**全部**位于父 Cycle 的 `000-initial.md`（其 Plan Context 与 `Plan Review` 的历史叙述，按规则不改）与本 Cycle 的 `Plan Context`（不可改写）；两份增量、`tasks.md`、`design.md`、`proposal.md` 中**无残留**。
- **未重跑全量的依据**：本 change 的收尾全量已由父 Cycle 的 T7 完成（14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored，覆盖对账 missing 0 / extra 0 / duplicate 0，基线 2171 → 差额 +12 = 本 change 新增用例数），来源为父 Cycle `Act Response`；本 Cycle **不改 `src/`**（被覆盖的产品代码自结论产生后未变化），且本 Cycle 唯一变更的测试文件由上述定向命令覆盖 ⇒ 沿用父 Cycle 结论（SKIPPED: 高开销验证已按 change 预算执行完毕，重复运行不构成新证据）。
- **未触及**：`openspec/specs/`（规格合并是 `openspec-docs-maintainer` 的收尾职责；MS35 的三个域仍未合并，其中 `free-page-lifecycle` 尚未建立）、全局 `tasks.md` / `SNAPSHOT` / M / R / I、Issue 台账、父 Cycle 文件、Plan Review。

### Persisted Evidence

None required —— 验收结论由 `cargo test` 的真实运行与对规格文本的直接核对承载，可低成本重跑；无「结果无法低成本复现 / 一次性环境即将消失 / 需保留现场 / 摘要会丢失决定性结构」情形。不创建 `evidence/001-durable-release-gate/002-replan/` 目录。

### Experience Candidates

None（本 Cycle 未新增候选）：父 Cycle 的 `Act Response` 已记录 Issue 候选（B 树叶合并路径的重复释放）与 Runbook 候选（M19 批计划 135 目标 / 14 批 / 2183 passed / 0 failed / 2 ignored），父 Cycle 的 `Plan Review` 另记录 3 项候选（DDL 边界刷盘失败路径的提前提升面、DDL 边界有序落盘把先前目录删除一并写盘、释放路径的有界读取缺陷）。本 Cycle 只补测试覆盖与 change 文档，无新的运行证据，故不重复登记。

### Remaining Issues

- 本 Cycle 未产生 Blocker；无 `SKIPPED` 项（未重跑全量的理由见 Verification Evidence，按 change 预算沿用父 Cycle 结论）。
- 遗留 Minor（不阻塞）：`design.md` D4 第二条从句的措辞（见 Self-Review）——范围外，移交 Plan 或 change 收尾时决定。
- 父 Cycle 移交的三项候选（提前提升面、B 树重复释放、有界读取改造）与本轮 Review 新增的候选仍待上游按用户补充 2 选择承载 change（Act 只报告不登记）。

## Plan Review

- Review Result: accepted
- Next Iteration: None
- Cycle 状态：冻结（本 Review 给出最终结论并完成收尾交接）

**Findings**

本节按「独立核对结论 → T8 改动核对 → A8–A11 逐项核对 → GAP-1/GAP-2 闭合核对 → 规格增量最终可合并性 → T7 结论沿用核对 → 身份机制与判定层 → 两项裁定 → Acceptance Gaps → 收敛判断 → Issue/Runbook 候选 → 收尾交接」组织。依据：独立阅读当前 Cycle 的 Plan Context 与 Act Response、change 的 `proposal.md` / `design.md` / `tasks.md` / `README.md` 与三份规格增量、`tests/free_page_lifecycle_test.rs` 的该用例及相关产品代码；Act 的 Self-Review 只作输入，不作依据。

*A. 独立核对结论（实现与现场）*

1. **崩溃可观察组合的成因与修正方向成立**：`src/storage/catalog.rs:226` `insert_table` → `:259` `flush_commit_point`（`:269` 定义）→ `:271` `buffer_pool.flush_all_except(&exclude)` 写入除提交页外的**全部**脏页、`:272` 再写提交页；而 `DROP TABLE` 的目录删除走 `delete_table`（`:279`）→ `delete_from_chain`（`:787`）经 `modify_page`（`:821`）只把目录页改脏，`delete_table` 自身**不**调用 `flush_commit_point`（该调用点只有 `:259` / `:442` / `:615`，均不在 `delete_table` 路径上）⇒ 「`DROP TABLE` 之后同一会话任一 `CREATE TABLE` 会把先前的目录删除一并写盘」成立 ⇒ 「旧表仍存在 ∧ 新表不存在」不可满足。修正后的崩溃点（`DROP TABLE` 成功返回之后、**任何后续 DDL 语句成功返回之前**）**可由产品真实产生**，且是本 change 唯一被验收的崩溃形态。
2. **`src/` 变更集未变**：`git diff --stat -- src/` = `20 files changed, 1258 insertions(+), 164 deletions(-)`，`git status --porcelain -- src/` 的唯一未跟踪路径是 `src/storage/free_page_rebuild.rs`（Iteration 000 引入，非本次）⇒ A11 成立，且父 Cycle 的实现与其验证结论的**被覆盖代码**未变。
3. **T8 的唯一代码改动面与 Act 声明一致**：按 mtime 复核，`src/` 下最新改动为 `src/storage/file_storage.rs`（19:56:50），`tests/` 下其余文件最新为 `tests/overflow_page_test.rs`（20:21:41），而 `tests/free_page_lifecycle_test.rs` 为 20:43:39 ⇒ T8 只改了该一个测试文件（该文件为未跟踪新文件，故不出现在 `git diff` 中，与 `git status` 的 `??` 一致），change 产物侧只改了 `tasks.md`（20:44:21）与本 Cycle 文件。

*B. T8 改动核对（独立阅读当前文件内容，逐处与 Act 声明比对）*

| 位置 | 独立读到的内容 | 核对结论 |
|---|---|---|
| `tests/free_page_lifecycle_test.rs:433–:444`（文档注释） | 引用修正后的两个场景（FPL R4「释放未持久时崩溃后旧表内容完整」、DPB R1（MODIFIED）「释放未持久时的分配不摧毁旧表内容」），并写明崩溃点取在 DDL 边界之前的理由（`flush_commit_point` → `flush_all_except` 把提交点与其它脏页一并落盘） | 与 Act 声明一致 ✓ |
| `:452–:464`（会话一） | `create_small(t)` + `INSERT INTO t` + **新增** `create_small(other)`（`:457`）+ `INSERT INTO other VALUES (1, 100)`（`:458`）+ `close()`（`:461`，checkpoint + WAL 截断） | 活表 `other` 的已提交行确由磁盘承载（非 WAL 重放）✓ |
| `:466–:471`（会话二） | 仅 `DROP TABLE t` 后 `crash(db)`（`wal_buffer.shutdown()` + `drop(db)`，**不 `close()`**），其后无任何 DDL | 崩溃点形态与修正后的场景一致 ✓ |
| `:474–:481` / `:488–:498` / `:499–:511` | `t` 的行断言、两条页号断言（`v` 的数据页 ≠ `t` 数据页；`v` 的索引树根 ≠ `t` 索引树根）、`check_integrity` error 计数 0 | 三条既有断言**逐字保留**，未弱化/删除/跳过 ✓ |
| `:482–:487` | **新增**断言 `rows(&db, "SELECT id, n FROM other") == [[1, 100]]` | 与 Act 声明一致 ✓ |
| `change/tasks.md` | 仅 T8 行末状态单元格 `pending`→`done`；RTM 表与「场景文本修正说明」未动 | ✓ |

用例数核对：文件内 `#[tokio::test]` 计数 = **12**（与 T7 时的 12 一致，未增删用例）；断言数 +1 ✓。

*C. A8–A11 逐项核对*

| Acceptance | 独立核对结论 |
|---|---|
| A8（两份增量场景 ↔ 已验收行为逐条对应） | FPL R4 场景的 GIVEN/WHEN/THEN 与 DPB R1（MODIFIED）场景的 GIVEN/WHEN/THEN，均与上表 `:452–:511` 的断言一一对应；DPB R1（MODIFIED）正文含新增前置条件「也没有其它 DDL 语句成功返回」（`specs/drop-persistence-boundary/spec.md:5`）✓ |
| A9（`other` 覆盖补强 + 三条既有断言仍在） | `:482–:487` 新增断言在位；`:474–:481` / `:488–:498` / `:499–:511` 三条原样保留 ✓ |
| A10（定向验证 12 passed / 0 failed） | 沿用 Act 的真实运行（`cargo test --test free_page_lifecycle_test -- --test-threads=1` ⇒ exit 0，`12 passed; 0 failed; 0 ignored`）：该结论覆盖的代码（`src/` 未变 + 该测试文件）自运行后**未再变化**，覆盖范围未失效 ⇒ 沿用并注明来源；本 Review **不重复运行**（公共规则 › 验证：禁止为增强信心重复已通过的验证） |
| A11（产品代码零改动） | 独立复核 `git diff --stat -- src/` = `20 files changed, 1258 insertions(+), 164 deletions(-)`，`git status --porcelain -- src/` 无新增路径 ✓ |

*D. GAP-1/GAP-2 闭合核对*

1. **GAP-1（父 Cycle 的 A3 崩溃可观察组合）已关闭**：修正后的崩溃点不再要求「崩溃前创建的新表不存在」这一不可满足项；本 Cycle 的用例以「未完成持久化的释放所涉及的页不是可分配页」承载 FPL R4 的 requirement 正文三项（旧主人的已提交行可查 / 未触及的活表 `other` 的行可查 / 释放页不被取用 / 完整性 0 error）。
2. **GAP-2（两份增量记录了实现不具备的可观察量）已关闭**：`specs/free-page-lifecycle/spec.md` 的 R4 场景（`:74–:80`）与 `specs/drop-persistence-boundary/spec.md` 的 R1（MODIFIED）场景（`:25–:31`）均把崩溃点固定在「释放之后、任何后续 DDL 之前」，且每条 THEN 都对应一条断言；DPB R1（MODIFIED）正文补齐了前置条件。
3. **旧措辞残留独立复核**：在 change 目录内检索 `目录未持久化` / `同一会话创建新表` / `新表不存在` / `崩溃前创建的新表` / `分配方对象在重开后不存在`，命中项全部位于父 Cycle `000-initial.md`（冻结的历史叙述，Plan Context 不可改写）、本 Cycle 的 Plan Context（不可改写）与 `design.md` 的 D4 第二分句——后者为 Act 报告的遗留 Minor，**本次 Review 已直接更正**（见裁定二）；两份增量、`tasks.md`、`proposal.md` 中无残留 ✓。
4. **数量核对**：FPL 4 requirement / 7 scenario（R1:2、R2:3、R3:1、R4:1）、DPB 1 / 4、DTPF 2 / 5 —— 与父 Review 的清单逐项吻合，本 Cycle 未增删任何 requirement 或 scenario ✓。

*E. 规格增量最终可合并性（即将合并进 `openspec/specs/`）*

逐条 requirement / scenario ↔ 已验收用例（本 change 三份增量自身的每条陈述**均有**验收用例，未把未验收行为写成已验收）：

| 增量 | requirement / scenario | 验收用例 |
|---|---|---|
| FPL | R1 场景 1 / 场景 2 | `released_page_not_allocatable_before_release_durable` / `released_page_allocatable_after_release_durable` |
| FPL | R2 场景 1 / 2 / 3 | `reopened_database_reuses_released_pages` / `rebuild_never_marks_live_pages_free` / `recovery_rebuilt_index_pages_are_not_reallocated` |
| FPL | R3 场景 1 | `rebuild_failure_keeps_live_pages_intact` |
| FPL | R4 场景 1 | `crash_with_pending_release_keeps_old_table_rows` |
| DPB | R1（MODIFIED）场景 1 / 2 / 3 | 既有 `tests/drop_persistence_test.rs`（11 例） |
| DPB | R1（MODIFIED）场景 4 | `crash_with_pending_release_keeps_old_table_rows` |
| DTPF | 2 条 requirement / 5 场景 | `tests/drop_table_free_test.rs`（更新后的分界断言）+ `tests/free_page_lifecycle_test.rs`（父 Review 已逐条核对） |

与既有 75 域的冲突核对（独立阅读）：`ddl-catalog-transactional`、`index-write-side-atomicity`、`statement-cancellation-outcome` 三域与 FPL/DPB 无交集断言；`readonly-open-mode`（只读零写入 / WAL 未决拒绝）、`in-memory-database`（内存库无持久语义）、`truncate-table`、`overflow-page-storage`（溢出页无孤儿页，不规定复用时机）、`catalog-update-persistence`、`wal-recovery-replay-integrity`（DDL 语句边界有序落盘）均不与 FPL/DPB 的陈述冲突；`drop-persistence-boundary` 基线的 R2（释放后立即分配得到空页）、R3（删列崩溃）、R4（加密库）、R5（受控 I/O 失败的两种结果）在合并后仍与 DPB R1（MODIFIED）自洽（R5 的「写入在改文件之前失败」形态下待释放项被丢弃 ⇒ 旧表仍可见；「同步失败 / 位点失败」形态下第 2 步刷盘已成功 ⇒ 目录删除已落盘、`t` 不存在，两形态均与 MS35 的闸门一致）⇒ **两份增量合并后不产生与实现相反的断言**。

**另发现（既有域残留，非本 change 增量引入 ⇒ 非阻塞 Minor finding + Issue 候选）**：既有域 `openspec/specs/drop-table-physical-free/spec.md` 中**未被本 change MODIFIED** 的 requirement「TableManager::drop_table 物理释放」，其场景 1–3 断言「`drop_table` 后 `FileStorage::free_pages` SHALL 至少包含被释放的页」「后续 `allocate_page` SHALL 优先返回这些 freed page id」——该断言在 MS35 的闸门下已不成立（`src/storage/file_storage.rs:504–:516` 的 `free_page` 只把页推入 `pending_frees`，`:462` 的 `allocate_page` 只从 `free_pages` 弹出），且与已验收用例 `tests/drop_table_free_test.rs::test_simple_drop_releases_data_and_btree`（(a) 段断言「未 checkpoint 前新建表不得占用被释放页」）**直接相反**。同一域另有两处更早的同类残留（MS33 的 checkpoint 边界语义所致）：requirement「不引入新 WAL 记录」的 `kill -9` 场景（断言 `__tables` 中 `users` 行已不存在）与 requirement「drop_table 操作顺序」的「catalog 抹除先于 in-memory 移除」场景（断言崩溃后 `__tables` 不含 `users` 行）。**裁定：不阻塞本 change**，理由：① 本 change 的三份增量自身的每条陈述均准确且已被用例验收，合并时**新增**的文本不含任何与实现相反的断言（与父 Review 的 GAP-2 判据一致——GAP-2 阻塞的是「增量自身写入了相反的断言」，本次不存在）；② 该残留位于既有域中**未被本 change 触碰**的 requirement，其与实现的偏差是「该域基线文本与实现的整体对齐」问题，且其中两处属 MS33 已验收语义（`proposal.md` 的范围外条款「不改 MS33 已验收的 DROP 崩溃语义」），在 MS35 内只补其中一处无法使该域自洽；③ 承载路径已确定（见 Issue 候选 1）。⇒ 以候选形式移交，由承载 change 一次性把该域基线文本与实现对齐。

*F. T7 全量结论沿用核对（批 6 定位与是否需重跑）*

- **定位**：`tests/free_page_lifecycle_test.rs` 落在 T7 分批计划的**第 6 批**（`file_storage_io / free_page_lifecycle / full_table_dml / gc / group_by_expr / import_input_surface / index_manager / index_write_atomicity / insert_column_list / insert_select / isolation_level`，11 个目标，T7 结论 `exit 0 / passed 88 / failed 0 / ignored 0`）。
- **是否需要重跑该批**：**不需要**。理由：① 被覆盖的**产品代码**自 T7 结论产生后未变化（`src/` 变更集与 T7 时一致，`git diff --stat -- src/` 无新增条目）；② 该批内**唯一**在 T7 之后发生变化的目标（`free_page_lifecycle`）已由本 Cycle 的定向命令以**同参数**（`--test-threads=1`）重跑并通过（12 passed / 0 failed / 0 ignored，用例数与 T7 时一致，本 Cycle 只增断言不增用例）；③ 该批其余 10 个目标是各自独立的测试二进制，其输入（`src/` 与各自测试文件）均未变化，`cargo test` 的 `--test` 目标之间无交叉耦合 ⇒ 其结果不可能因本 Cycle 的改动而变化。⇒ AGENTS.md › 验证列出的重跑情形（被覆盖代码在结论产生后发生变化）在本批不成立（全批范围），该批结论沿用。
- **若后续需要该批的完整复核**（M19 纪律，只重跑该批、不重跑全量）：目标清单 = 上列 11 个目标，执行方式 = 一次 `cargo test --test file_storage_io --test free_page_lifecycle --test full_table_dml --test gc --test group_by_expr --test import_input_surface --test index_manager --test index_write_atomicity --test insert_column_list --test insert_select --test isolation_level -- --test-threads=1`，判据 = 退出码 0 且各 `test result:` 行 failed 计数 0；与其它批**串行**，不得并行、不得用无目标过滤的全量调用、不得用默认并行度。本 Review 不执行此重跑（结论已足）。

*G. 身份证明机制与判定层核对*

`git diff -- src/` 与 `git diff -- tests/` 中无 hash / 指纹 / revision pin / run-id / manifest / checksum / 时间顺序证明；`src/storage/free_page_rebuild.rs`（42 行，未跟踪）只提供**一次性**的页读失败注入（按库路径限定，用于制造 FPL R3 的遍历失败），无身份字段、无判定逻辑；新增的 4 个 `AsyncStorage` 方法均带默认实现（`MemoryStorage` 与测试替身零改动）；全部判定落在 `cargo test` 的断言内，无框架外判定层、无 timer 阈值作行为判据 ⇒ **无身份证明机制、无框架外判定层**。

*H. 两项裁定*

**裁定一：「Task Definition 优先于派生指令」——Act 的取舍正确，予以确认。**
事实：`002-replan.md` 的 T8 **Targets 明列** `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows`；**Forbidden** 只禁 `src/` 下的改动、弱化/删除/跳过既有断言、改写 requirement 正文、引入判定脚本/封装命令/专用参数/timer 阈值/身份证明机制、在本 Cycle 内重跑全量——**未禁改测试文件**；A9 要求该用例覆盖 `other`，不变量要求「两份增量场景的每条 THEN 都必须能对应到用例中的一条断言」，二者合起来**只能**通过修改该测试文件实现。派发指令（上游转述的「禁止改产品代码**或测试文件**」「只改 change 内的规格增量与 tasks.md 文本」）与已批准的 Task Definition 冲突，且与 A9/不变量**互斥**。
裁定依据：Task Definition 是 Act 的**任务级执行依据**（AGENTS.md：Act 直接按 ready 的 Plan Context 建立先行测试并实施；Act 可处理非实质局部差异并在 Response 记录；「不探索清楚不实现 / 不计划清楚不实现」），派生指令不得收窄已批准的执行依据；冲突时以 Task Definition 为准，并记录冲突后执行。Act 的实际改动（`src/` 零改动 + 只改 Targets 明列的测试文件 + `tasks.md` 状态 + 两份增量只核对不重写）与 Task Definition 完全一致 ⇒ **取舍正确，确认通过**。
附带记录（非阻塞）：Act Response 的 `Deviations from Plan: None` 与 Self-Review 未记录「派发指令与 Task Definition 冲突」这一事实。按「Act Response 记录自检发现」的规则，本项应补记；因结论与执行依据一致、不影响任何判定，判为**记录完整性 Minor finding**（见下），不阻塞。

**裁定二：`design.md` D4 的遗留 Minor——非阻塞，且本次 Review 已直接更正。**
事实核对：D4 第二分句原写「崩溃前分配方语句的目录未落盘 ⇒ 分配方对象在重开后不存在」。该窗口在 MS35 闸门下**不可达**（释放在崩溃边界上成立之前，被释放的页不会被任何新对象取用 ⇒ 不存在「分配方对象」）；且作为一般规则它也不准确（DDL 如 `CREATE TABLE` 在语句边界即把自己的目录写盘，非 DDL 的分配根本不涉及目录落盘状态）。判定：**非阻塞的文档准确性问题**（`design.md` 不进入 `openspec/specs/`，不影响任何 requirement、场景或用例断言），但它是 change 自身的产物，且 Review 有权写 change 产物 ⇒ **本次 Review 直接更正**该分句（只改该分句，不动 D4 的其余内容）：改写为「MS33 描述的窗口在本闸门下**不可达**……因而不存在『分配方对象在重开后消失』的可观察形态；旧对象在重开后是否仍可见，取决于崩溃点是否落在任何后续 DDL 语句成功返回之前……该崩溃模型的归属见 `drop-persistence-boundary` 域」，与修正后的 FPL R4 / DPB R1（MODIFIED）文本一致。

*Acceptance Gaps*

- **None**。本 Cycle 的 A8–A11 全部满足；父 Cycle 的 GAP-1 / GAP-2 已由本 Cycle 关闭；Iteration 000（`accepted`）与 Iteration 001 的其余 Acceptance（A1–A7）经父 Review 与本次 Review 核对均已满足。本 change 的三份规格增量合并后**不新增**任何与实现相反的断言。

*收敛判断*

- Iteration 001 = 本 change 的**最后一个** Iteration（`tasks.md` 的 Iteration Map：000 重建 → 001 释放持久化判定与闸门），其 Stable result「被释放的页只有在释放在崩溃边界上成立之后才进入可分配集合 ⇒ MS33 遗留窗口不再以『旧表数据被清零且不可恢复』的形式出现」已由 T4–T6 的定向用例 + T7 的分批全量（14 批 / 135 目标 / 2183 passed / 0 failed / 2 ignored）+ 本 Cycle 的 T8 覆盖并验收。
- **Next Iteration: None**（Iteration Map 无后继 Iteration；不新建 Cycle）。
- 本 change 进入收尾：合并三份规格增量、同步全局状态、归档 change 由 `openspec-docs-maintainer` 执行（见下方交接说明）。

*Issue 候选与 Runbook 候选（只报告不登记，按 M20；承载方式按用户补充 2 给出）*

1. **Issue 候选（新增，规格文本准确性）——既有域 `drop-table-physical-free` 的基线 requirement 与实现不一致**：`TableManager::drop_table 物理释放` 的场景 1–3（`free_pages` 立即含被释放页 / 下一次 `allocate_page` 优先返回它们）已被 MS35 的闸门推翻，且与已验收用例 `tests/drop_table_free_test.rs::test_simple_drop_releases_data_and_btree` 的 (a) 段直接相反；同域另有两处 MS33 遗留残留（「不引入新 WAL 记录」的 `kill -9` 场景、「drop_table 操作顺序」的「catalog 抹除已发生」场景）。**承载方式**：随后的「承载候选」change 一次性把该域基线文本与实现对齐——以 MODIFIED 增补「TableManager::drop_table 物理释放」等 requirement（正文改「归还到待释放集合 / 释放在崩溃边界上成立前不得被分配」，场景改「进入待释放集合」「持久化并提升之后才可分配」），并同时收敛两处 MS33 遗留残留；**不需要**产品代码改动。
2. **Issue 候选（沿用父 Review 的 3 项 + 父 Cycle Act 的 1 项，不重复登记）**：① 未完成释放在「DDL 边界刷盘失败 → 锚点写丢失 → 后续一次成功刷盘仍提升」路径上的提前提升面；② B 树叶合并/子合并路径对同一页的二次释放；③ DDL 语句边界有序落盘把先前未落盘的目录删除一并写盘（与 `wal-recovery-replay-integrity` 的「DROP … 保持 checkpoint 边界」之间的张力）；④ 释放路径的有界读取改造（损坏页上的 DROP / TRUNCATE / DROP INDEX panic 面）。
3. **Issue 候选（新增，规格文本精度，低）**：`free-page-lifecycle` R2 的正文「重新打开数据库时 SHALL 依据持久目录重建本次打开的可分配页集合」未写明**只读打开不重建**这一既有裁定（`design.md` D3.5 与 `tasks.md` T2 执行依据均已记录该例外，实现侧由 `readonly_open_does_not_rebuild_free_pages` 钉住）。合并后不与 `readonly-open-mode` 冲突（只读会话不分配页，重建无写入），属措辞精度问题 ⇒ 可与候选 1 在同一承载 change 内顺手补「读写打开时」。
4. **Runbook 候选（沿用父 Cycle Act 的记录）**：R48（`.agents/runbooks/m19-batched-full-regression.md`）补本次实例——135 目标 / 14 批（12×11 + 2 + `--lib --bins`）/ 2183 passed / 0 failed / 2 ignored；`.agents/runbooks/in-process-crash-simulation.md`（R49）可补「崩溃点须取在任何后续 DDL 成功返回之前」这一形态约束。

*非阻塞 Minor findings（记录，不在本 change 内修复）*

1. Act Response 的 `Deviations from Plan: None` 与 Self-Review 未记录「派发指令与 Task Definition 冲突」这一事实（裁定一）——记录完整性问题，执行结果不受影响。
2. `tests/free_page_lifecycle_test.rs` 的**文件级**文档注释仍只写「覆盖面：A1, A2, A3」与「R2/R3」，未反映本文件现已覆盖 R1/R4；用例级注释已是准确的 ⇒ 注释精度问题。
3. `change/README.md` 的「计划状态」段仍写 Iteration 001「待 Plan Review」——该文件的状态同步属 `openspec-docs-maintainer` 的「同步指定 change 结果」职责，本次不改（见交接说明）。

*收尾交接说明（交由 `openspec-docs-maintainer` 执行；本次 Review **不**执行）*

1. **合并三份规格增量进 `openspec/specs/`**：
   - 新建 `openspec/specs/free-page-lifecycle/spec.md`：4 requirement / 7 scenario（R1:2、R2:3、R3:1、R4:1），来源 `openspec/changes/2026-10-24-ms35-free-page-reuse-across-restart/specs/free-page-lifecycle/spec.md`。
   - 合并 `openspec/specs/drop-persistence-boundary/spec.md` 的 MODIFIED R1（1 requirement / 4 scenario），保留该域其余 requirement（R2–R5）不变。
   - 合并 `openspec/specs/drop-table-physical-free/spec.md` 的 MODIFIED 两条 requirement（「同进程 free-list 复用」2 场景、「跨重启 drop 安全」3 场景），保留该域其余 requirement 不变；**并按 Issue 候选 1 的处置计划，把该域基线 requirement 的文本对齐纳入随后的承载 change**（本次合并不夹带该域基线修正）。
2. **同步全局状态**：`.agents/docs/tasks.md` 的 MS35 状态与里程碑结果、`.agents/docs/SNAPSHOT.md` 的当前项目描述、`.agents/memory/project-model.md`（如有新增约束）、`.agents/memory/references.md`（如登记 R）、`.agents/memory/improvements.md`（I087 的落地状态）。
3. **归档 change**：按项目惯例把 `openspec/changes/2026-10-24-ms35-free-page-reuse-across-restart/` 移入归档，并把 `change/README.md` 的状态段更新为最终结论（accepted，两 Iteration）。
4. **Issue / Runbook 登记**：本次 Review 报告的 Issue 候选 1 与 3、以及父 Cycle 已移交的 4 项候选，由 `openspec-experience-recorder` 按用户补充 2 建立/更新（承载 change 尚未创建，故先登记候选，不在本次 Review 内登记）。
5. **工作区现状提示**：HEAD `0a08b14`（ahead of origin 9），MS34 + issue-fix change + MS35 的改动**全部未提交**（`src/` 20 个已改文件 + `src/storage/free_page_rebuild.rs`，`tests/` 5 个已改文件 + 4 个新增测试文件），属既有现场——收尾归档**不**包含代码提交，提交与否由用户决定。
