# Tasks: MS37 缺陷台账收口（ISS25–ISS34）

**Change**: `2026-10-25-ms37-defect-ledger-closure`
**状态**：`pending` / `in_progress` / `done` / `blocked`。完整状态以 change 产物（Cycle 的 `Act Response`）为准。
**来源**: `.agents/docs/tasks.md` 的 `### MS37` 段（Outcome: 缺陷台账零 `open` 残留；10 条 open 缺陷 ISS25–ISS34 全部处置并关闭）
**裁定**: `design.md` D1–D15（D2 的 DDL 提交点统一模型、D3 的 ISS32 合同化关闭、D4 的每表令牌、D5 的 `UNIQUE` 渲染同型条件、D6 的两条相反处置、D7 的按页型分组、D8 的写盘失败状态模型；D12 的 ISS35 读路径冻结方向、D13 的 ISS36 范围外排除项、D14 的第一次结构修正、**D15 的写路径承载形态与依赖顺序**）

---

## 任务列表

| Task | 标题 | Iteration | 依赖 | 状态 |
|---|---|---|---|---|
| T1 | 索引树叶页条目长度纳入页级判据——先行 RED 用例（读 / 写路径 + 释放路径） | 000 | None | done |
| T2 | `check_tree_page` 叶页分支补条目长度判据 ⇒ 读 / 写路径明确错误、释放路径计数 + 报出 + 跳过 | 000 | T1 | done |
| T3 | 其余页型的有界读：数据页组（数据页 + 缓冲池 + executor / 事务调用点） | 001 | T2 | done（Iteration 001 / Cycle 000，Plan Review 2026-10-25 `accepted`） |
| T4 | 其余页型的有界读：目录页组（`__tables` / `__columns` 链） | 004 | T3, T17, T18, T19 | done（Iteration 004 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T5 | 其余页型的有界读：恢复面组（WAL 重放） | 005 | T4 | done（Iteration 005 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T17 | 页头槽数越界的读路径 abort（ISS35）：槽目录容量界限统一 + 页级判据 + 数据页组应用 | 002 | T2 | done（Iteration 002 / Cycle 000，Plan Review 2026-10-25 `accepted`；编号在 T1–T16 之后追加，避免改动既有编号与 RTM 引用） |
| T18 | 写路径的页头槽数下溢（ISS37）：落位 / 删槽 / 原地改写的页级判据前置 +「页满 / 页损坏」分流 | 003 | T17 | done（Iteration 003 / Cycle 000，Plan Review 2026-10-26 `accepted`；编号在 T17 之后追加，避免改动既有编号与 RTM 引用） |
| T19 | 写路径的槽负载偏移无界写（ISS38）：版本头原地写与目录行原地改写的负载有界写 | 003 | T18 | done（Iteration 003 / Cycle 000，Plan Review 2026-10-26 `accepted`；编号在 T17 之后追加，避免改动既有编号与 RTM 引用） |
| T6 | DDL 提交点统一模型：窄化意图随事务持久化 + 恢复期发布（ISS25） | 006 | T5 | done（Iteration 006 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T7 | DDL 提交点统一模型：提交前失败的目录面补偿（ISS28） | 006 | T6 | done（Iteration 006 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T8 | DDL 提交点统一模型：每表布局 DDL 令牌与同表并发拒绝（ISS29） | 006 | T7 | done（Iteration 006 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T9 | 写盘失败状态：在途写入标记与驱逐不丢弃未落盘内容（ISS30） | 007 | T8 | done（Iteration 007 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T10 | 写盘失败状态：失败标记按页记录与复位，释放提升改按锚点页（ISS31） | 007 | T9 | done（Iteration 007 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T11 | dump 的 `UNIQUE` 渲染按 INTEGER 类型条件（ISS33） | 008 | T10 | done（Iteration 008 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T12 | 多表 dump 部分恢复边界的合同化与真实入口回归锁定（ISS32） | 008 | T11 | done（Iteration 008 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T13 | 语料交接：IRBR 的 Requirement 与 4 个 Scenario 标题措辞对齐（ISS34，交 `openspec-docs-maintainer`） | 009 | T12 | done（Iteration 009 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T14 | 语料对齐：WAL 提交确认与 checkpoint 边界的文档化（I064） | 009 | T13 | done（Iteration 009 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T15 | 交接：14 处主 spec 的 Purpose 补写（I045，交 `openspec-docs-maintainer`） | 009 | T14 | done（Iteration 009 / Cycle 000，Plan Review 2026-10-26 `accepted`） |
| T16 | 收尾全量回归（M19 分批串行纪律，本 change **唯一一次**） | 009 | T15 | done（Iteration 009 / Cycle 000，Plan Review 2026-10-26 `accepted`） |

**任务归属**：每个任务只归属一个 Iteration（T1–T2 → 000；T3 → 001；T17 → 002；**T18–T19 → 003**；T4 → 004；T5 → 005；T6–T8 → 006；T9–T10 → 007；T11–T12 → 008；T13–T16 → 009）。**T17** 是实施中途登记的 ISS35（goal 指令要求「中途发现并登记的 issue 直接在那个 change 分配 iter 进行解决」），编号在 T1–T16 之后**追加**（不改既有编号，避免破坏 RTM 与其它引用），归属 Iteration 002，排在 T4 之前（承载与理由见 Iteration Plan 与 `design.md` D12）。**2026-10-25 结构修正**：原 Iteration 001 同时承载 T3 / T17 / T4 / T5 并用本地 Cycle 序列展开，现按 `design.md` D14 拆为各自承载单一任务的 Iteration 001 / 002 / 003 / 004。**2026-10-25 二次结构修正（D15）**：Iteration 002 的 Act 报告的写路径候选已由 Recorder 登记为 **ISS37 / ISS38** ⇒ 新增 **T18 / T19** 承载写路径的槽界限，归属**独立 Iteration 003**，插在 Iteration 002 与目录页组（原 Iteration 003）之间 ⇒ 原 Iteration 003（目录页组）顺延为 **Iteration 004**，其后 005（恢复面组）/ 006（DDL）/ 007（写盘失败）/ 008（dump-restore）/ 009（语料 + 收尾全量）依次顺延。**依赖理由（硬约束）**：写路径**必须排在 T5（恢复面组）之前**——恢复重放经 `src/wal/recovery.rs:1519` 到达 `src/storage/data_page.rs:272` 的无界写，写路径未收口时 T5 的「恢复面有界」保证是空的；写路径**必须排在 T4（目录页组）之前**——两者同改 `src/storage/catalog.rs:707` / `:924` 的目录写入点，避免两个 Iteration 改同一批写入点。T18 / T19 的编号同样在 T17 之后**追加**（不改既有编号，避免破坏 RTM 与其它引用）。

---

## 任务执行依据要点

- **T1 / T2（ISS26）**：落点是 `src/storage/integrity.rs` 的 `check_tree_page` **叶页分支**——它只做槽边界校验，而**内部页分支**另有条目长度校验（`MAX_KEY_LEN + 4`）；叶页条目格式是 `MAX_KEY_LEN + RowId::SIZE`（= 38）⇒ 短叶条目落在页内时 `violations` 为空 ⇒ `node.rs` 的 `get_key` / `get_row_id` 返回 `None` ⇒ 该条目被当作「没有这个条目」静默跳过。**两条相反处置**（读 / 写路径 = 明确错误；释放路径 = 计数 + 报出 + 跳过）由**同一处判据扩展**同时达成。
- **T3–T5（ISS27）**：约 21 处无界槽读点，按页型分三组（数据页 + 缓冲池 + executor / 事务调用点 / 目录页 / 恢复面），各组**分别判定**损坏页的正确行为（见 `design.md` D7 的分组表）。
- **T17（ISS35，实施中途登记并在本 change 内承载）**：页头声称的槽数超出槽目录容量 ⇒ 槽目录定位整数下溢 ⇒ 读路径 abort。**与 ISS27 同域不同机制**（ISS27 = 槽**负载**越界；ISS35 = 槽**目录**定位下溢），不合并。承载形态与三个待判问题的裁定见 `design.md` D12；任务归属 Iteration 002（与 T3 同域、同验证面），排在 T4 之前。
- **T18 / T19（ISS37 / ISS38，实施中途登记并在本 change 内承载）**：写路径的**同形**缺保护 —— ISS37 = 页头槽数被无保护地参与减法（`slotted_page.rs` 的 `add_slot_inner` / `free_space` / `delete_slot`）⇒ 写入时 panic / abort，且既有「`Err(_)` ⇒ 换新页继续」的分支会把损坏页**跳过**而继续落盘；ISS38 = 槽负载写入无边界检查（`data_page.rs` 的版本头原地写、`update_slot_in_place`）⇒ 越界写 / 静默改写相邻字节。**与 T17 同域（同一页格式层、同一判据族）不同面（写 vs 读）**，不合并；承载形态、错误形态与写路径失败语义见 `design.md` D15 与 Iteration 003 的 Plan Context。**同族待判点纳入同一 Iteration 的理由**：`delete_slot` 的同形减法与 ISS37 **同形、同文件、同原语族且有生产触点**（`data_page.rs:327`、`catalog.rs:824` / `:987`），排除它会留下一个同规则的已知 abort 原语；`update_slot_in_place` 与 ISS38 **同族**且是目录行**唯一**的生产原地改写路径（`catalog.rs:924`），排除它会使「写路径的槽界限」不完整。
- **T6–T8（ISS25 / ISS28 / ISS29）**：见 `design.md` D2 的 M1–M5。**磁盘格式不变** ⇒ MS36 的 G1 / G2 / G3 兼容面不受影响。
- **T9 / T10（ISS30 / ISS31）**：见 `design.md` D8。失败状态是**内存态**，不落盘。
- **T11 / T12（ISS33 / ISS32）**：见 `design.md` D5 / D3。T11 改变 dump 的**产物内容**（属功能语义变更，须独立验收）；T12 **不改行为**，只把既有边界写成规范条款并以真实入口回归锁定。
- **T13（ISS34）**：标题措辞与正文及已实现行为一致。**排在存储 Iteration 之后**——ISS26 会扩展该域的判据覆盖面与 Scenario 集合，先扩内容再一次性对齐措辞，避免二次返工。
  **承载形态 = 交接项**（不是增量）：OpenSpec 的 `MODIFIED` 块在归档时被校验「当前规格中的每个 Scenario 标题必须逐字出现在该块内」，否则**归档拒绝该增量** ⇒ 既有 Scenario 的标题改名**无法**由增量表达；Requirement 标题虽可用 `RENAMED` 表达，但改名后仍与既有 Scenario 标题并存 ⇒ 本 change 的 `index-read-bounded-read` 增量**只承载行为变化**（ISS26）并逐字保留既有 Requirement 与 Scenario 标题；**ISS34 的标题对齐以文档交接项交 Maintainer 在合并时同批应用**（与 T15 同一交接批次）。
- **T14（I064）**：文档面（README 的性能对比段落 + 承载 WAL 提交语义的既有域的边界条款），**不扩大**既有承诺。
- **T15（I045）**：14 处主 spec 的 Purpose 属**文档面**（不含行为要求）⇒ 按 AGENTS.md 的「轻量工作」规则不建 change；本 change 以**交接项**形式交 `openspec-docs-maintainer`，在收尾合并时随增量一并落位。
- **T16（全量回归）**：见下方「收尾验证」。

---

## Requirements Traceability Matrix

| Requirement / Scenario（增量规格） | Task | First Test（计划名） | Status |
|---|---|---|---|
| IRBR「未通过页级判据的索引树页在读 / 写路径上给出明确的错误」+「叶页条目长度过短的索引树页给出明确的错误」 | T1, T2 | `tests/index_read_bounded_read_test.rs::short_leaf_entry_makes_select_fail_with_a_named_error`（计划名） | Covered |
| RPBR「释放路径读取页内容时有边界检查」+「叶页条目长度过短的索引树页被计数、报出并跳过」 | T1, T2 | `tests/release_path_bounded_read_test.rs::drop_index_cli_reports_the_short_leaf_entry`（计划名） | Covered |
| PRBR「其余页型的页内容读取经由有边界检查的路径」（四个页型 Scenario；恢复面 Scenario 已按恢复类别写细） | T3, T4, T5 | T3：`tests/data_page_bounded_read_test.rs::…`（计划名随 Cycle 000 定稿）；T4：`tests/catalog_bounded_read_test.rs::…`（已定稿，5 个用例）；T5：`tests/recovery_bounded_read_test.rs::…`（已定稿，6 个用例：`scanning_class_skips_the_damaged_slot_and_the_replay_continues` / `scanning_class_reports_the_skipped_slot_on_the_read_write_cli_stderr` / `replay_class_fails_the_open_with_a_named_error_on_a_damaged_page_header` / `replay_class_fails_the_open_with_the_existing_redo_error_on_a_damaged_old_row` / `scanning_class_reports_the_skipped_slot_while_marking_uncommitted_rows` / `undamaged_recovery_is_unchanged`） | Covered |
| PRBR「页头声称的槽数超出槽目录容量时读路径不 abort」（ISS35 的行为条款；页头槽数越界 = 槽**目录**定位下溢，与上面的槽**负载**越界并列） | T17, T4, T5 | T17：`tests/data_page_bounded_read_test.rs::…`（页头槽数越界用例；计划名随 Iteration 002 / Cycle 000 定稿）；T4：`tests/catalog_bounded_read_test.rs::…`（页级形态用例，已定稿）；T5：`tests/recovery_bounded_read_test.rs::…`（页级形态用例：页头 `slot_count` 越界 ⇒ 恢复面不 abort） | Covered |
| PWBW「页头声称的槽数超出槽目录容量时写路径不 abort 且不换页继续」（ISS37 的行为条款；写侧与读侧**同名同形**的错误，失败语义不同：写入失败 ⇒ 语句失败 / 回滚） | T18 | `tests/write_path_slot_bounds_test.rs::…`（页头槽数越界的落位 / 删槽用例；计划名随 Iteration 003 / Cycle 000 定稿） | Covered |
| PWBW「槽负载越界或过短时写路径不越界写且不静默改写相邻字节」（ISS38 的行为条款） | T19 | `tests/write_path_slot_bounds_test.rs::…`（槽 offset 越界 / 负载不足用例；计划名随 Iteration 003 / Cycle 000 定稿） | Covered |
| PWBW「写入失败使语句失败并回滚事务」+「未损坏路径的行为零变化」 | T18, T19 | 同上 + 会话事务面用例 + 未损坏对照 + 定向回归 20 目标 | Covered |
| DLP「布局窄化的意图与数据面改写原子绑定」+「提交点是事务提交，目录窄化是提交后的幂等发布」 | T6 | `tests/ddl_layout_publication_test.rs::committed_narrowing_is_published_after_crash`（计划名） | Covered |
| DLP「未发布的窄化意图不因检查点而丢失」（**2026-10-26 Plan 补入**） | T6 | 同上（检查点变体） | Covered |
| DLP「提交前失败的 DDL 目录面变更被补偿」（+「失败的 `ADD COLUMN` 目录面被回退」，**2026-10-26 Plan 补入**） | T7 | `tests/ddl_layout_publication_test.rs::failed_ddl_catalog_change_is_compensated`（计划名） | Covered |
| DLP「同表布局窄化 DDL 串行化」（范围裁定为窄化 DDL，**2026-10-26 Plan 收窄措辞并补边界 Scenario**） | T8 | `tests/ddl_layout_publication_test.rs::concurrent_same_table_ddl_is_refused`（计划名） | Covered |
| 既有域兼容性：`schema-persistence` / `ddl-catalog-transactional` 的**最小面修订**；`legacy-format-compat` G1/G2/G3、`database-file-format-header`、`ddl-rewrite-consistency`、`mvcc-tombstone-visibility`、`drop-persistence-boundary`、`release-durability-precondition` **保持** | T6, T7, T8 | `tests/ddl_layout_publication_test.rs`（未损坏对照）+ `cargo test --lib` + 定向回归 16 目标 | Covered |
| WFS「驱逐不得丢弃未落盘的内容」+「写盘失败状态按页记录并可复位」+「释放的可分配判定按锚点页的失败状态表达」 | T9, T10 | T9：`failed_flush_then_evict_keeps_the_new_bytes`；T10：`anchor_is_promoted_after_its_page_is_rewritten` / `anchor_stays_unpromoted_while_its_page_is_failed`（Iteration 007 / Cycle 000 定稿） | Covered |
| CLI「生命周期子命令 dump 与 restore」（`UNIQUE` 渲染 + 多表部分恢复） | T11, T12 | T11：`tests/cli_test.rs::non_integer_unique_stock_dump_roundtrips_without_the_constraint`；T12：`tests/cli_test.rs::multi_table_restore_reports_the_partial_apply`（Iteration 008 / Cycle 000 定稿） | Covered |
| SCE「UNIQUE 形态与类型面诚实化」的「存量非 INT UNIQUE 库的边界已知」 | T11 | `non_integer_unique_stock_dump_roundtrips_without_the_constraint` + `legacy_dump_with_non_integer_unique_is_rejected` | Covered |
| LFC「范围外的显式拒绝与迁移路径」的「非整数唯一列的旧 dump 明确拒绝」 | T11 | `legacy_dump_with_non_integer_unique_is_rejected` | Covered |
| IRBR 的 Requirement 与 4 个 Scenario 标题措辞（ISS34） | T13 | 交接项核对（交接措辞逐字可应用；非代码） | Covered |

**Missing**: 无。所有增量规格的 Requirement / Scenario 均已映射到任务；所有任务的 Requirement 均在增量规格中。

---

## Iteration Plan

### Iteration 000: 索引树叶条目长度纳入页级判据（ISS26）

- **阶段结果**：叶页条目长度进入页级判据 ⇒ 短叶条目在读 / 写路径给出明确错误、在释放路径计数 + 报出 + 跳过。
- **稳定结果**：`index-read-bounded-read` 与 `release-path-bounded-read` 的页级判据覆盖槽边界与条目长度两类内容；MS38 的「页级判据先行」前置达成。
- **验证边界**：定向回归（`index_read_bounded_read_test` / `release_path_bounded_read_test` / `index_manager_test` / `secondary_index_test` / `index_page_ownership_test` / `index_page_ownership_stack_overflow_test` / `cargo test --lib`）。**不跑全量**（本 change 的全量预算在 Iteration 009 / T16）。
- **诊断边界**：判据扩展只落在一处（`check_tree_page` 的叶页分支）；若读 / 写路径未给出明确错误 ⇒ 闸门覆盖缺口（回 Plan）；若健康页被误判 ⇒ 判据过宽（回 Plan）。
- **Non-goals**：其余页型的无界读点（Iteration 001 / 004 / 005）；磁盘格式；`free_page` 集合语义；索引名注入 `IndexManager`。
- **Tasks**: T1, T2。

### Iteration 001: 数据页组的有界读（ISS27 · T3）

- **阶段结果**：数据页组（数据页 + 缓冲池 + executor / 事务调用点）的无界槽读点全部有界化：越界槽在读 / 写路径给出点名该页与该槽的明确错误，缓冲池的布尔型页辅助降级为安全值。
- **稳定结果**：新增域 `page-read-bounded-read` 的数据页部分成立；T3 交付的 `SlottedPageRef::get_slot_data_checked` 与 `StorageError::CorruptedDataPage` 被后续 Iteration 复用（冻结原语）。
- **验证边界**：定向回归（20 目标，M19 分批串行）+ `cargo test --lib`。**不跑全量**（整个 change 的收尾全量归 Iteration 009 / T16，M19 分批串行纪律）。
- **诊断边界**：11 个生产读点 + 1 个调用方 + 1 个测试辅助，同一组夹具与同一条判据形态；越界槽未点名页与槽 ⇒ 判据覆盖缺口（回 Plan）；健康页被误判 ⇒ 判据过宽（回 Plan）。
- **Non-goals**：目录页组（Iteration 004）；恢复面组（Iteration 005）；页头槽数越界形态（Iteration 002 / T17）；索引树叶条目长度（Iteration 000）；索引树访问器（已有界）；数据页的**页级**判据扩展；完整性扫描的诊断分级；损坏页的修复；可写变体的负载偏移无界写（Issue 候选 → 已登记为 ISS37 / ISS38，由 Iteration 003 承载）。
- **Tasks**: T3。**依赖**: 000。
- **Cycle**: `iterations/001-data-page-bounded-read/000-initial.md`（唯一 Cycle；Plan Context `ready`；**Plan Review 2026-10-25 = `accepted`**）。
- **按页型分组的拆分信号裁决（2026-10-25，Plan，依 goal 授权按全局最优解裁定并记录理由）**：三组**均不触发**拆分信号 ⇒ 三组的**方向**不需要新的产品决策；每组各自成为一个 Iteration（承载面独立），见 Iteration 001 / 004 / 005。
  - **T3 数据页组**：D7 的方向（「读路径明确错误，点名页与槽」；缓冲池布尔辅助降级为安全值）与已被验收的 `index-read-bounded-read` 同型，只是换到数据页；数据页的释放 / 重建 / `rtsql check` 面早已由既有 `check_data_page` 页级判据有界 ⇒ 无需新方向。**承载面 = Iteration 001**（`iterations/001-data-page-bounded-read/000-initial.md`；**Plan Review 2026-10-25 = `accepted`**）。
  - **T4 目录页组**：**裁定 = 目录页损坏 ⇒ 该库 SHALL NOT 打开**，打开 / 查询路径以明确错误结束（点名该页与该槽；表名仅在负载尚可读出时一并点名，不臆造）。理由：目录是「哪些页仍然存活」的根依据，跳过该目录行会让该表静默消失（其页成为孤儿、后续释放可能回收活页）⇒ 比打不开坏得多；打不开同时严格优于现状（今天是 panic / abort）。该裁定与 D7 同向 ⇒ **不是新方向**，不拆分。**可见后果已写入 delta spec**：目录损坏时该库的整个使用面（打开 / 完整性扫描 / 导出 / 模式查看）报出打开错误，而不是给出基于残缺目录的结论。**承载面 = Iteration 004**（见该 Iteration 的计划；原为 Iteration 003，因写路径组插入而顺延，见 `design.md` D15）。
  - **T5 恢复面组**：D7 的单一方向按恢复类别写细（**不是改方向**）：① **已提交记录重放类**（更新重放所需的旧行推导）⇒ 越界槽 SHALL **NOT** 跳过该记录（跳过会让不可忠实重放的已提交记录被静默丢弃，与既有 `wal-recovery-replay-integrity` K05 直接冲突）⇒ SHALL 以明确错误结束恢复与打开；② **扫描类**恢复读取（重放前候选扫描 / 重放后索引重建 / 未提交事务标记）⇒ 跳过该槽、计入恢复告警并报出、其余记录继续重放（与这三处既有的「过短即跳过」容错同向，只是扩到「越界」并要求报出）。⇒ 不拆分，**承载面 = Iteration 005**（见该 Iteration 的计划；原为 Iteration 004，因写路径组插入而顺延，见 `design.md` D15）。
- **ISS36（`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 两个门在 HEAD 即不可满足）**：**不在本 change 实施**（范围外排除项）——修复形态是仓库级的格式重排（跨 12 个文件、其中数个正是本 change 修改的文件）或新增工具链固定文件并改变整个仓库的构建工具链（后者会使在途各 Cycle 已取得的编译与验证基线全部失效，须在独立 change 中连同全量回归一并处理）⇒ 违反本 change 的 Surgical Changes 边界。完整记录（问题、根因基线事实、理由、台账路径、对收尾措辞的影响）见 `design.md` D13。
- **读点计数（Plan 逐点核对，以本计数为准；适用于 ISS27 域的 Iteration 001 / 004 / 005）**：`src/` 内 `SlottedPageRef::get_slot_data` 调用点共 **21** 处 = 生产 **20**（数据页组 11 + 目录页组 5 + 恢复面组 4）+ `#[cfg(test)]` 辅助 1（`src/storage/data/table_manager.rs:2762`）；另有 2 处**已在界内**、不属 ISS27（`src/storage/btree/node.rs:39` / `:51`，先判界后取数）。ISS27 台账条目写的「约 17 处」是低估值；`design.md` D7 的「约 21 处」与本清单一致。
- **结构修正记录（2026-10-25，Plan）**：本 Iteration 原同时承载 T3 / T17 / T4 / T5 并用同目录的本地 Cycle 序列展开，已按 `design.md` D14 拆为各自承载单一任务的 Iteration 001 / 002 / 003 / 004；**各任务的 Task Definition / Acceptance / 验证内容一律未变**。

### Iteration 002: 页头槽数越界的读路径 abort（ISS35 · T17）

- **阶段结果**：数据页组在页头声称的槽数超出槽目录容量时不再因槽目录定位下溢而 abort；读 / 写路径在**取槽之前**识别该页头损坏，返回点名该页与页头声称槽数的明确错误。
- **稳定结果**：槽目录容量界限在全项目**只有一处定义**（页格式层）；页级判据的形状被后继 Iteration 复用（冻结原语）。
- **验证边界**：定向回归（20 目标，M19 分批串行）+ `cargo test --lib`。**不跑全量**（收尾全量归 Iteration 009 / T16，M19 分批串行纪律）。
- **诊断边界**：12 个取槽点 + 可写变体的负载偏移相邻面；页级判据缺失 ⇒ 形态覆盖缺口（回 Plan）；索引树侧的取值或判据变化 ⇒ 常量迁移偏离（回 Plan）。
- **Non-goals**：目录页组（Iteration 004）；恢复面组（Iteration 005）；可写变体的负载偏移无界写（Issue 候选 → 已登记为 ISS37 / ISS38，由 Iteration 003 承载）；全量回归。
- **Tasks**: T17。**依赖**: 001。
- **Cycle**: `iterations/002-slot-count-underflow/000-initial.md`（唯一 Cycle；Plan Context `ready`；Act Response `reported`；**Plan Review 2026-10-25 判定 `accepted`**）。
- **T17 / ISS35 的承载裁定（2026-10-25，Plan，依 goal 指令「中途发现并登记的 issue 直接在那个 change 分配 iter 进行解决」）**：
  - **承载形态 = 独立 Iteration 002**（`iterations/002-slot-count-underflow/000-initial.md`，Cycle Type = `initial`），**排在 T4（Iteration 004）之前**。**理由（执行正确性）**：T4 / T5 的读点同样经由 `get_slot` 取得 `&Slot`；若 ISS35 的修复改变 `get_slot` 的形状（返回 `Option` / `Result` 或新增界限），T4 / T5 必须写在最终形状上，否则返工 ⇒ 把 ISS35 排在**紧邻 T4 之前**的独立 Iteration（002），既保证执行顺序，又不与其它页型组共用一个 Iteration。**（原承载形态为「Iteration 001 内的本地后继 Cycle，Cycle Type = `replan`」；2026-10-25 结构修正见 `design.md` D14；2026-10-25 二次修正见 `design.md` D15 —— 目录页组顺延为 Iteration 004。）**
  - **ISS35 台账的三个待判问题**（修复位置 / 页型行为 / 可写变体是否纳入）与**不实施**的部分见 `design.md` D12。
  - **Plan Review 结论（2026-10-25）**：**`accepted`**（A6 / A7 / A8 全部满足，独立代码核对支撑；Act 报告的 Deviation 第 1 项为「现状变化 / Plan 预期偏差」——T3 的逐槽判据已使真实 SQL 入口在变更前不 abort，冻结方向未被改动；详见该 Cycle 文件的 `Plan Review` 段）。

### Iteration 003: 写路径的槽界限（ISS37 + ISS38 · T18 / T19）

- **阶段结果**：写路径（落位 / 删槽 / 槽负载原地改写 / 行版本头原地改写）在页头槽数越界与槽负载越界 / 载荷不足时**不 panic / 不 abort**、不下溢、不越界写；以**点名该页**（负载形态时同时点名该槽与区间）的明确错误结束。
- **稳定结果**：新增域 `page-write-bounded-write` 成立（写侧与读侧**同名同形**的错误、**同一**判据）；写入失败的语义成文：**写入失败 ⇒ 语句失败 / 事务回滚**，页损坏**不得**换页继续、**不得**折叠成页满 / 行不存在、**不得**静默跳过；与 `write-failure-state` 域（ISS30 / ISS31）的边界成文（页损坏 = 写入前的确定性永久条件；刷盘失败 = 刷盘面的可重试暂态；两者不共用状态、不互相遮蔽）。
- **验证边界**：定向回归（20 目标，M19 两批各 10、批内 `--test-threads=1`、批间串行）+ `cargo test --lib` + 新增测试目标 `tests/write_path_slot_bounds_test.rs`。**不跑全量**（收尾全量归 Iteration 009 / T16）。
- **诊断边界**：页级下溢（落位 / 删槽）与负载无界写（版本头 / 目录行）两个面各自独立验证；「页满 / 页损坏」的分流若无法在既有调用点保持既有「换页继续」行为 ⇒ 回 Plan；写失败类型若无法让 `src/storage/btree/**` 与 `src/wal/**` 零 diff ⇒ 回 Plan（不得就地扩大那两处的改动面）。
- **Non-goals**：读路径（已交付并接受）；目录页组的**读**点（Iteration 004）；恢复面组的读点与恢复类别语义（Iteration 005）；索引树侧的取值与判据（既有闸门已保护）；写盘失败状态面（Iteration 007）；全量回归。
- **Tasks**: T18, T19。**依赖**: 002。
- **Cycle**: `iterations/003-write-path-slot-bounds/000-initial.md`（唯一 Cycle；Cycle Type = `initial`；Plan Context `ready`；Act Response `reported`；**Plan Review 2026-10-26 判定 `accepted`**）。
- **承载裁定（2026-10-25，Plan，依 goal 指令「中途发现并登记的 issue 直接在那个 change 分配 iter 进行解决」）**：ISS37 / ISS38 由 Recorder 建档（**处置指针字段归 Recorder，本 change 不改台账**；承载分配记录在本文件与 `design.md` D15，由上层转交 Recorder 回填）⇒ 承载形态 = **独立 Iteration 003**（T18 = ISS37 的页级下溢；T19 = ISS38 的负载无界写），**插在 Iteration 002 与目录页组之间**。
  - **依赖顺序（硬约束）**：**排在 T5（恢复面组）之前** —— 恢复重放经 `src/wal/recovery.rs:1519` 到达 `src/storage/data_page.rs:272` 的无界写；写路径未收口时 T5 的「恢复面有界」保证是空的。**排在 T4（目录页组）之前** —— 写路径触及 `src/storage/catalog.rs:707` / `:924`，与 T4 同文件 ⇒ 避免两个 Iteration 改同一批目录写入点。
  - **纳入同族点**：ISS37 的 `delete_slot` 同形减法（`:349` / `:350` / `:357`）与 ISS38 的 `update_slot_in_place` 同族兄弟点（`:397-399`）**纳入同一 Iteration** —— 前者与 ISS37 同形、同文件、同原语族且有生产触点（`data_page.rs:327`、`catalog.rs:824` / `:987`），排除它会留下一个同规则的已知 abort 原语；后者是目录行唯一的生产原地改写路径（`catalog.rs:924`），排除它会使「写路径的槽界限」不完整。
  - **形态一致性（与 D12 的读路径冻结方向一致）**：写侧**复用** Iteration 002 冻结的页级判据（唯一实现、阈值 `slot_count > MAX_PAGE_SLOTS`）与 T3 冻结的槽负载判界（**上提为读侧 / 写侧共用的唯一实现**），并复用 `StorageError::CorruptedDataPage { page_id, detail }` 这一**同名同形**的错误；**不另造阈值、不另造错误形态**。写路径与读路径的**失败语义不同**（写入失败 ⇒ 语句失败 / 回滚；查询失败 ⇒ 错误返回），该差异已写入 delta spec 的「写入失败使语句失败并回滚事务」Scenario。

### Iteration 004: 目录页组的有界读（ISS27 · T4）

- **阶段结果**：目录页（`__tables` / `__columns` 链）的无界槽读点全部有界化：越界槽在打开 / 读取路径上给出点名该页与该槽的明确错误。
- **稳定结果**：新增域 `page-read-bounded-read` 的目录页部分成立；目录损坏的使用者面后果成文（打开 / 完整性扫描 / 导出 / 模式查看报出打开错误，而不是给出基于残缺目录的结论）。
- **验证边界**：定向回归（18 目标）+ `cargo test --lib`。**不跑全量**（收尾全量归 Iteration 009 / T16）。
- **诊断边界**：5 处读点 + 5 条传播链；目录损坏 ⇒ 该库打不开 ⇒ DDL 写路径的可达性边界（回 Plan）；错误文案点名槽序号与区间（回 Plan）。
- **Non-goals**：恢复面组（Iteration 005）；页级槽数判据的**实现**（Iteration 002）；写路径的槽界限（Iteration 003）；数据页组（Iteration 001）；目录行「过短负载」的既有语义。
- **Tasks**: T4。**依赖**: 001, 002, 003。
- **Cycle**: `iterations/004-catalog-bounded-read/000-initial.md`（唯一 Cycle；Plan Context `ready`；**目录原 `iterations/003-catalog-bounded-read/` 已按 `design.md` D15 重命名，内容除「所属 Iteration 标识」与「路径 / 顺序引用」外逐字保留**；**2026-10-26 追加标注**：来自 Iteration 003 / Cycle 000 的 Plan Review F9 —— 目录**删除**路径的两处错误丢弃 / 折叠（`catalog.rs:831` / `:1005-1014`）按可达性判定归入本 Iteration；若本 Cycle 的读点未先行拦截，则该标注转为 T4 的一个 work item；冻结的 Plan Context 正文未改写）。
- **Plan Review（2026-10-26，Plan）**：**`accepted`** —— Acceptance A9 / A10 / A11 全部满足；5 处目录读点全部在取槽之前消费 Iteration 002 冻结的**唯一**页级判据（`MAX_PAGE_SLOTS` 全项目唯一定义）、槽负载全部经 `get_slot_data_checked`；新增 `StorageError::CorruptedCatalogPage` 为**纯声明性新增**（此前无任何路径能产生它）；打开路径以该错误结束 ⇒ 该库 SHALL NOT 打开，`check` / `dump` / `.schema` 报出打开错误而非基于残缺目录的结论；3 项 Deviation 全部为非实质偏差（Deviation ① 裁定为**非实质的执行顺序偏差**、非 TDD 铁律的实质违规，理由见 Cycle 文件的 `Plan Review`）；**追加标注（F9）关闭**：两处标注位置 + Act 额外发现的第三处（`catalog.rs:1764`）在两种损坏形态下**均不可达**（逐条可达性论证见 Cycle 文件的 F7）⇒ 按公共规则 › Scope Control 不实施、不新立 Iteration、不登记 Issue；遗留 1 条非阻塞 Minor finding（重复 logical_id 形态当前不可达，非实质缺陷）。
- **方向裁定（2026-10-25，Plan）**：**裁定 = 目录页损坏 ⇒ 该库 SHALL NOT 打开**，打开 / 查询路径以明确错误结束（点名该页与该槽；表名仅在负载尚可读出时一并点名，不臆造）。理由：目录是「哪些页仍然存活」的根依据，跳过该目录行会让该表静默消失（其页成为孤儿、后续释放可能回收活页）⇒ 比打不开坏得多；打不开同时严格优于现状（今天是 panic / abort）。该裁定与 D7 同向 ⇒ **不是新方向**。**可见后果已写入 delta spec**：目录损坏时该库的整个使用面（打开 / 完整性扫描 / 导出 / 模式查看）报出打开错误，而不是给出基于残缺目录的结论。
- **执行顺序依赖**：本 Iteration 必须在 Iteration 002 **与** Iteration 003 被接受后执行 —— 页级槽数判据由 Iteration 002 交付并冻结；`src/storage/catalog.rs` 的**写入点**（`:707` 目录页链追加、`:924` 目录行原地改写）由 Iteration 003 交付并冻结（避免两个 Iteration 改同一批目录写入点）。

### Iteration 005: 恢复面组的有界读（ISS27 · T5）

- **阶段结果**：WAL 重放面的无界槽读点全部有界化，损坏槽按恢复类别分别处置（已提交记录重放类 ⇒ 明确错误；扫描类 ⇒ 跳过 + 计入告警并报出）。
- **稳定结果**：新增域 `page-read-bounded-read` 的恢复面部分成立；恢复面的两类处置语义成文。
- **验证边界**：定向回归（20 目标，2 批各 10）+ `cargo test --lib`。**不跑全量**（收尾全量归 Iteration 009 / T16）。
- **诊断边界**：已提交记录重放类与扫描类分别验证（2 个面 + 2 种损坏形态）；恢复告警的报出面与既有容错形态不一致 ⇒ 判定缺口（回 Plan）。
- **Non-goals**：目录页组（Iteration 004）；页级槽数判据的**实现**（Iteration 002）；写路径的槽界限（Iteration 003，`src/wal/recovery.rs:1519` 的无界写已由其收口）；数据页组（Iteration 001）；磁盘格式；dump / restore 合同面。
- **Tasks**: T5。**依赖**: 004。
- **Cycle**: `iterations/005-recovery-bounded-read/000-initial.md`（唯一 Cycle；Plan Context `ready`；**2026-10-26 由 Plan 展开** —— 在 Iteration 004 被接受后按正常流程创建，非预创建）。承载 **5 处**恢复面槽读取点：`build_pk_version_maps`（`:374`，扫描类）、`derive_old_row_id`（`:459` 取槽 + `:462` 负载，已提交记录重放类）、`rebuild_pk_indexes`（`:1177`，扫描类）、`mark_uncommitted_aborted`（`:1490`，扫描类）、`redo_tuple_at_row_id` 的幂等探测（`:166`，已提交记录重放类）。
- **新识别并纳入本 Iteration 的点（2026-10-26，Plan）**：`redo_tuple_at_row_id` 的幂等探测（`src/wal/recovery.rs:166` 的 `get_slot_by_logical_id`）**无页级判据前置** ⇒ 页头槽数越界时其内部 `get_slot(i)` 定位下溢 ⇒ abort；该形态（ISS35）在 `src/wal/**` **未被前序 Iteration 覆盖**（Iteration 002 的 12 个取槽点全在数据页组）⇒ 由 delta spec 的页级形态 Scenario（WHEN 明确含「按逻辑 id 查找某个槽」）纳入本 Iteration ⇒ **不构成范围扩张**。
- **delta spec 措辞补全（2026-10-26，Plan）**：`specs/page-read-bounded-read/spec.md` 的 Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」原写「恢复面 ⇒ 按恢复类别分别处置」，而其被引用的恢复面 Scenario 只覆盖**槽负载越界**形态 ⇒ 已在该 Scenario 内补全**页级形态**的逐面处置与粒度（扫描类 ⇒ 跳过该**页** + 计入告警并报出 + 继续重放；已提交记录重放类 ⇒ 明确的错误结束恢复与打开）。补全只收紧措辞、不新增方向 ⇒ 与本 change 的方向裁定同向。
- **方向裁定（2026-10-25，Plan）**：D7 的单一方向按恢复类别写细（**不是改方向**）：① **已提交记录重放类**（更新重放所需的旧行推导）⇒ 越界槽 SHALL **NOT** 跳过该记录（跳过会让不可忠实重放的已提交记录被静默丢弃，与既有 `wal-recovery-replay-integrity` K05 直接冲突）⇒ SHALL 以明确错误结束恢复与打开；② **扫描类**恢复读取（重放前候选扫描 / 重放后索引重建 / 未提交事务标记）⇒ 跳过该槽、计入恢复告警并报出、其余记录继续重放（与这三处既有的「过短即跳过」容错同向，只是扩到「越界」并要求报出）。

### Iteration 006: DDL 提交点统一模型（ISS25 + ISS28 + ISS29）

- **阶段结果**：布局窄化的发布点唯一且可由已提交的持久事实判定；提交前失败被补偿；同表布局窄化 DDL 串行化。
- **稳定结果**：新增域 `ddl-layout-publication` 成立（**扩展**既有 delta 域，不新建）；**磁盘格式不变** ⇒ `legacy-format-compat` 的 G1 / G2 / G3 支持范围与 `database-file-format-header` **保持**；`ddl-rewrite-consistency`、`mvcc-tombstone-visibility`、`drop-persistence-boundary`、`release-durability-precondition` **保持**；`schema-persistence`（DDL 不写 WAL 记录）与 `ddl-catalog-transactional`（目录变更随事务提交或中止）**最小面修订**（详见 Cycle 文件的「Compatibility Cross-Check」表）。
- **验证边界**：定向回归（**16 个目标，M19 两批各 8**：批 1 = `catalog_bounded_read_test` / `ddl_catalog_transactional_test` / `ddl_crash_recovery_test` / `ddl_layout_publication_test` / `ddl_rewrite_consistency_test` / `free_page_lifecycle_test` / `legacy_compat_test` / `recovery_bounded_read_test`；批 2 = `data_page_bounded_read_test` / `recovery_e2e_test` / `recovery_test` / `release_durability_precondition_test` / `wal_auto_checkpoint_test` / `wal_integration_test` / `wal_record_test` / `write_path_slot_bounds_test`；批内 `--test-threads=1`、批间串行）+ `cargo test --lib` + 新增测试目标 `tests/ddl_layout_publication_test.rs`。**不跑全量**（整个 change 的全量预算在 **Iteration 009 / T16**：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、全量 Gate = 全部批次各自 0 failed）。
- **诊断边界**：**M1**（意图随事务持久化）/ **M2**（提交点即事务提交 + 幂等发布）/ **M3**（提交前失败的补偿）/ **M4**（每表令牌 + 同表并发拒绝）/ **M5**（未发布意图不因检查点而丢失）五个面**各自独立验证**；M1 若需要改磁盘格式 ⇒ 回 Plan（并按 `design.md` D2 的要求补写兼容面与 `database-file-format-header` 的增量）；补偿若必须靠「再刷一次盘」实现 ⇒ 回 Plan（与「保持脏页」的要求冲突，且在存储失败时不可行）。
- **Non-goals**：原子 restore；跨语句事务；磁盘格式变更；改写窗口的读一致性（既有域已覆盖）；`ALTER TABLE … ADD COLUMN` 的事务化 / WAL 化；`ADD COLUMN` 与窄化 DDL 并发的竞态、`ADD COLUMN` 在列数刷盘期间被放弃时的目录面残留（均作为 **Issue 候选只报告不登记**，见 Cycle 文件的「Risks and Notes」）。
- **Tasks**: T6, T7, T8。**依赖**: 005。
- **Cycle**: `iterations/006-ddl-commit-point-unified-model/000-initial.md`（唯一 Cycle；Cycle Type = `initial`；Plan Context `ready`，Gate 2 全部 PASS；**Plan Review 2026-10-26：`accepted`**。2026-10-26 由 Plan 展开，在 Iteration 005 被接受后按正常流程创建，非预创建）。
- **方向裁定（`design.md` D2 + D4，已冻结，本 Iteration 未另选方向）**：窄化意图随事务持久化 + 恢复期发布 + 提交点即事务提交 + 提交前失败的目录面补偿 + 每表布局 DDL 令牌（拒绝并发，而非排队）。**本 Iteration 的范围裁定（Plan，依 goal 授权按全局最优解处置并记录理由）**：① **T8 的令牌只覆盖窄化 DDL**（`ALTER TABLE … DROP COLUMN`）—— `ADD COLUMN` 不纳入：它全程持有 `self.tables.write()` ⇒ 同表的两条 `ADD COLUMN` 已被串行化，纳入令牌会把今天**能成功**的并发 `ADD COLUMN` 改成被拒绝 ⇒ 属行为回退；delta spec 的 Requirement 标题与措辞已相应收窄为「同表布局**窄化** DDL 串行化」并补边界 Scenario。② **T7 的补偿覆盖 `ADD COLUMN` 的失败路径**（ISS28 的实测形态）；窄化 DDL 的提交前失败 / 被放弃**不留目录面变更**由「目录发布在提交之后」的结构保证（D2 M2）⇒ delta spec 的两个既有 Scenario 由此成立并被测试锁住。③ **M5 纳入 T6**（D2 明列的模型面，且是 M1 得以成立的必要条件：WAL 是窄化意图的唯一持久载体 ⇒ 检查点不得使未发布的意图丢失）。
- **兼容性核对的结论（Plan 逐条读码核对，2026-10-26）**：`legacy-format-compat` 的 G1 / G2 / G3 约束的是**目录行布局代际**，与 **WAL 记录类型**是两个独立的格式面 ⇒ 新增一种 WAL 记录类型**不落在** G1 / G2 / G3 的支持范围内 ⇒ **保持**；G1 的恢复要求（旧库带未决写入记录打开后恢复已提交行）**保持**（旧 WAL 不含新记录类型）。`schema-persistence` 与 `ddl-catalog-transactional` 的修订面**最小且明确**（见 Cycle 文件的兼容性核对表）。**已知代价（非缺陷）**：新构建写出的 WAL 含新记录类型 ⇒ 旧构建读不了新 WAL；该代价**不在** G1 / G2 / G3 的支持范围内，且 WAL 格式**不引入版本机制**。

### Iteration 007: 写盘失败的状态面（ISS30 + ISS31）

- **阶段结果**：被驱逐页的内容不再丢失；写盘失败状态按页记录并可复位。
- **稳定结果**：新增域 `write-failure-state` 成立；`release-durability-precondition` 的 R1 保持、R2 的提升判据改为按锚点页口径。
- **验证边界**：一批 4 个目标，名字典序，批内 `--test-threads=1`：`drop_persistence_boundary_test` / `free_page_lifecycle_test` / `release_durability_precondition_test` / `write_failure_state_test`。然后单独 `cargo test --lib -- --test-threads=1`。两步串行。**不跑全量**（收尾全量仍在 Iteration 009 / T16）。
- **诊断边界**：驱逐交错（ISS30）与失败标记复位（ISS31）两个面独立验证。ISS30 的交错用容量 1 的缓冲池单测闸门存储稳定命中（`arm_io_failure` 打不中「清脏之后、写盘返回之前」）。ISS31 的锚点页由调用点传入，替换 `last_failed_write`。
- **Non-goals**：释放集合语义的时点（`id < limit`、刷盘失败 discard 保持）；磁盘格式；持久化的失败状态；全量回归。
- **Tasks**: T9, T10。**依赖**: 006。
- **Cycle**: `iterations/007-write-failure-state/000-initial.md`（唯一 Cycle；Cycle Type = `initial`；Plan Context `ready`，Gate 2 全部 PASS；**Plan Review 2026-10-26：`accepted`**。2026-10-26 由 Plan 展开，在 Iteration 006 被接受后按正常流程创建）。R2 的「失败之后本会话永不提升」由本 change 的 `specs/release-durability-precondition/spec.md` MODIFIED 改为按锚点页。定向回归实际目标名是 `drop_persistence_test`（计划文本里的 `drop_persistence_boundary_test` 不是 cargo 目标名）；一批 4 个目标 + `cargo test --lib`，均 `--test-threads=1`，两步串行，全量未跑。

### Iteration 008: dump / restore 的合同面（ISS32 + ISS33）

- **阶段结果**：`dump` 的 `UNIQUE` 渲染按 INTEGER 类型条件；多表部分恢复的边界成为规范条款并被回归锁定。
- **稳定结果**：`cli-noninteractive-shell` 的 dump / restore 条款修订；`sql-constraint-enforcement` 的「存量非 INT UNIQUE 库的边界已知」Scenario 修订；`legacy-format-compat` 的迁移路径说明补写。
- **验证边界**：定向回归一批 3 个目标，名字典序，批内 `--test-threads=1`：`cli_test` / `legacy_compat_test` / `planner_test`。然后单独 `cargo test --lib -- --test-threads=1`。两步串行。**不跑全量**（收尾全量仍在 Iteration 009 / T16）。
- **诊断边界**：T11 改变产物内容（独立验收）；T12 只加条款与回归（不改行为）。
- **Non-goals**：原子 restore；全量预检；非整数列 `UNIQUE` 的计划期支持。
- **Tasks**: T11, T12。**依赖**: 007。
- **Cycle**: `iterations/008-dump-restore-contract/000-initial.md`（唯一 Cycle；Cycle Type = `initial`；Plan Context `ready`，Gate 2 全部 PASS；**Plan Review 2026-10-26：`accepted`**。2026-10-26 由 Plan 展开，在 Iteration 007 被接受后按正常流程创建）。T11 只给 `create_table_sql` 的 `UNIQUE` 加上与主键同型的整数类型条件，并在两份 README 的存量兼容段各补半句。T12 不改 `restore`，用真实 CLI 锁住多表部分恢复。定向回归一批 3 个目标，名字典序，批内 `--test-threads=1`，然后单独 `cargo test --lib -- --test-threads=1`。全量不在本 Cycle。

### Iteration 009: 语料收口与收尾全量回归（ISS34 + I045 + I064 + T16）

- **阶段结果**：IRBR 标题措辞与正文一致；WAL 提交确认与 checkpoint 边界的文档化落地；14 处主 spec 的 Purpose 交 Maintainer 落位；本 change 的**唯一一次**全量回归完成。
- **稳定结果**：MS37 的 10 条缺陷全部处置并关闭（零 `open` 残留）；全量回归结论可交付 Maintainer。
- **验证边界**：**本 change 的全量回归在此执行一次**（M19 分批串行，见下方「收尾验证」）。
- **诊断边界**：语料面零产品代码改动；全量批次失败时定位到具体批次与目标，不重跑全量。
- **Non-goals**：新增缺陷的修复（登记为 Issue 候选交 Recorder）；Issue 台账的登记与关闭（由 Recorder 按用户指令执行）。
- **Tasks**: T13, T14, T15, T16。**依赖**: 008。
- **Cycle**: `iterations/009-corpus-and-full-regression/000-initial.md`（唯一 Cycle；Cycle Type = `initial`；Plan Context `ready`，Gate 2 全部 PASS；**2026-10-26 由 Plan 展开** —— 在 Iteration 008 被接受后按正常流程创建）。T13 / T15 只写 `corpus-handoff.md`，不改主规格库。T14 改两份 README 的性能对比段，并新增 `specs/wal-auto-checkpoint` 的边界 Scenario。T16 是本 change 唯一一次全量：目标名字典序，等分后每批 11 或 12，`--lib --bins` 单独一批，批内 `--test-threads=1`，批间串行，不得一次无过滤全量，不得默认并行度。**Plan Review 2026-10-26：`accepted`**。实际 `N = 148`，前 8 批各 11、后 5 批各 12，`--lib --bins` 为第 00 批。14 批退出码都是 0，每条汇总的 failed 都是 0；覆盖核对缺、多、重复都是 0。不展开后继 Iteration。

---

## 收尾验证：M19 分批全量回归（T16，记在最后一个 Iteration 009）

- **预算**：整个 change 的全量回归**只跑一次**，在本 Iteration（009）执行。
- **分批方式**：测试目标按**目标名字典序**分组，每组 **8–12 个目标**，每批一次独立的 `cargo test --test <目标> … -- --test-threads=1`；批间**顺序执行**，同一时刻只有一批在跑。
- **SHALL NOT** 用一次无目标过滤的全量调用跑完；**SHALL NOT** 用默认并行度跑任何全量批次。
- **Gate 等价条件**：全部批次各自 **0 failed**；缺一批不得声明全量通过。
- **前次终态不可作依据**：2231 passed / 0 failed / 2 ignored（143 目标 / 14 批）在本次改动前**已失效**。
- **交付**：批次清单与每批结果写入 Act Response；不创建 `evidence/` 目录（Persisted Evidence = `none`）。

---

## 平衡审计

| Iteration | 任务数 | 独立可验证结果 | 诊断边界 | 结论 |
|---|---|---|---|---|
| 000 | 2 | 页级判据覆盖条目长度 | 单处判据 + 单组夹具 | 通过 |
| 001 | 1 | 数据页组有界化 | 11 个生产读点 + 1 个调用方 + 1 个测试辅助，同一组夹具与同一条判据形态 | 通过 |
| 002 | 1 | 页头槽数越界关闭 + 槽目录容量界限唯一 | 12 个取槽点 + 页级判据形状冻结（排在写路径组与 T4 之前） | 通过 |
| 003 | 2 | 写路径的槽界限关闭（页级下溢 + 负载无界写） | 页级下溢（落位 / 删槽）与负载无界写（版本头 / 目录行）两个面；「页满 / 页损坏」分流（排在 T4 与 T5 之前） | 通过 |
| 004 | 1 | 目录页组有界化 + 目录损坏 ⇒ 该库 SHALL NOT 打开 | 5 处读点 + 5 条传播链 | 通过 |
| 005 | 1 | 恢复面组有界化 | 5 处读点（4 处槽负载 + 1 处槽目录幂等探测）× 已提交记录重放类 / 扫描类两个面 × 槽负载越界 / 页头槽数越界两种形态 | 通过 |
| 006 | 3 | DDL 提交点统一模型 | M1 / M2 / M3 / M4 / M5 五个面（意图随事务持久化 / 提交点即事务提交与幂等发布 / 提交前失败的补偿 / 每表令牌与同表并发拒绝 / 未发布意图不因检查点而丢失） | 通过 |
| 007 | 2 | 写盘失败状态 | 驱逐交错 / 失败标记两个面 | 通过 |
| 008 | 2 | dump / restore 合同面 | 产物变更 / 条款与回归两个面 | 通过 |
| 009 | 4 | 语料收口 + 全量回归 | 语料面零代码改动；全量按批次 | 通过 |

- **首个 Iteration 未承载整个 change**：000 只承载 ISS26（2 个任务），其余 **9** 个 Iteration 各自独立。
- **每个 Iteration 形成稳定结果**：000 = 判据覆盖；001 = 数据页组有界；002 = 页头槽数越界关闭 + 容量界限唯一；**003 = 写路径的槽界限关闭（页级下溢 + 负载无界写）+ 写入失败的语义成文**；004 = 目录页组有界；005 = 恢复面组有界；006 = 发布点唯一且可由已提交的持久事实判定（意图随事务持久化 + 恢复期发布 + 提交前失败被补偿 + 同表窄化被拒绝）；007 = 内容不丢失 + 失败可复位；008 = 合同面收口；009 = 语料 + 全量。
- **合并检查**：000 与 001 未合并（根因位置与承载面不同——页级判据覆盖面 vs. 无界读点分布面，且 001 的各页型需各自判定）；001 / 002 / **003** / 004 / 005 各自承载一个页型组或一个独立缺陷，未合并（各组的判据形状、夹具与验证面互不相同，且 ISS35 必须先于写路径组与 T4 冻结 `get_slot` 的最终形状；写路径组必须先于 T4 与 T5 收口，否则 T4 的目录写入点与 T5 的恢复面有界保证都建立在未收口的写路径上）；**003 内的 T18 / T19 未拆为两个 Iteration**（同域同层：同一批页格式层写入原语、同一套夹具、同一条「判据唯一 + 错误同名同形」的形态；T19 依赖 T18 落定的共享判界原语，拆开会重复引入判据）；006 内的三个任务未拆为三个 Iteration（共享同一模型与同一验证面，拆开会重复引入 WAL / 目录面语义）；007 内两任务未拆（同域同验证面）；008 内两任务未拆（同一子命令族）。
- **拆分信号**：若 001 / 004 / 005 的某一页型组需要**新的产品决策**（如目录页损坏是否允许打开库）⇒ 该组回 Plan 请求裁定；若 003 的写路径需要**新的产品决策**（如页损坏是否允许换页继续、写入失败是否回滚）⇒ 该组回 Plan 请求裁定（当前已裁定，见 `design.md` D15）；若 006 的 M1 被判定必须改磁盘格式 ⇒ 按 `design.md` D2 补兼容面与 `database-file-format-header` 增量，并评估是否拆出独立 change。
- **结构修正（2026-10-25，Plan）**：原 Iteration 001 同时承载 T3 / T17 / T4 / T5 并用同目录的本地 Cycle 序列展开，偏离公共规则「当前 Iteration 只有在 Review Result 为 `accepted` 后才能完成并展开 Map 中的下一 Iteration」与「Rework / Replan Cycle 使用本地编号」的形态（详见 `design.md` D14）⇒ 拆为各自承载单一任务的 Iteration 001 / 002 / 003 / 004；**各任务的 Task Definition / Acceptance / 验证内容未变**。
- **二次结构修正（2026-10-25，Plan，`design.md` D15）**：Iteration 002 的 Act 报告的写路径候选已由 Recorder 登记为 ISS37 / ISS38 ⇒ 新增**独立 Iteration 003** 承载写路径的槽界限（T18 / T19），插在 Iteration 002 与目录页组之间 ⇒ 原 Iteration 003 及之后依次顺延（003→004 … 008→009）；目录页组的 Cycle 目录随之重命名为 `iterations/004-catalog-bounded-read/`（内容除「所属 Iteration 标识」与「路径 / 顺序引用」外逐字保留）。

---

## Iteration Map

| Iteration | 标题 | Tasks | 依赖 | 状态 |
|---|---|---|---|---|
| 000 | 索引树叶条目长度纳入页级判据 | T1, T2 | None | accepted（Plan Review 2026-10-25：Acceptance 1–5 全部满足；实现与 Plan Context 一致，delta spec 逐字对应；2 条非阻塞 Minor finding 记录不改） |
| 001 | 数据页组的有界读（ISS27 · T3） | T3 | 000 | accepted（Plan Review 2026-10-25：Acceptance A1–A5 全部满足；11 个生产读点全部改走有边界检查的访问器，判据与索引树侧同形；2 条非阻塞 Minor finding 记录不改） |
| 002 | 页头槽数越界的读路径 abort（ISS35 · T17） | T17 | 001 | accepted（唯一 Cycle `iterations/002-slot-count-underflow/000-initial.md`；**Plan Review 2026-10-25：Acceptance A6–A8 全部满足**；12 个取槽点全部在取槽之前消费页级判据；`MAX_PAGE_SLOTS` 全项目唯一定义、索引树侧零 diff；Deviation 第 1 项为「现状变化 / Plan 预期偏差」，冻结方向未改；1 条非阻塞 Minor finding 记录不改） |
| 003 | 写路径的槽界限（ISS37 + ISS38 · T18 / T19） | T18, T19 | 002 | accepted（唯一 Cycle `iterations/003-write-path-slot-bounds/000-initial.md`；**Plan Review 2026-10-26：Acceptance A12–A15 全部满足**；六个写侧入口全部在任何算术 / 字节写之前消费页级判据，槽级判界由读侧上提为读写共用；写侧错误统一经单一映射点落为 `StorageError::CorruptedDataPage`（未新增错误变体）、与读侧同名同形；`MAX_PAGE_SLOTS` 全项目唯一定义、索引树侧与 `src/wal/**` 零 diff；Deviation 第 1 项为「现状变化 / Plan 预期偏差」（读侧先拦截使夹具 A' / B / B2 在 SQL 入口变更前已 GREEN，钉死的 RED 改由写原语直接入口复现），冻结方向未改；2 条非阻塞 Minor finding：目录删除路径的错误丢弃 / 折叠 → 归入 Iteration 004（可达性判定，见其 Cycle 文件的追加标注）；delta spec「区间」措辞已在本 change 内修正为析取形式） |
| 004 | 目录页组的有界读（ISS27 · T4） | T4 | 001, 002, 003 | accepted（唯一 Cycle `iterations/004-catalog-bounded-read/000-initial.md`；**Plan Review 2026-10-26：Acceptance A9–A11 全部满足**；5 处目录读点全部在取槽之前消费 Iteration 002 冻结的唯一页级判据、槽负载全部经 `get_slot_data_checked`；新增 `StorageError::CorruptedCatalogPage` 为纯声明性新增；打开以该错误结束 ⇒ 该库 SHALL NOT 打开，`check` / `dump` / `.schema` 报出打开错误；3 项 Deviation 均为非实质偏差（Deviation ① 裁定为非实质的执行顺序偏差，非 TDD 铁律的实质违规）；含来自 Iteration 003 Plan Review 的追加标注（F9）—— 经**逐条可达性论证**关闭（两处标注位置 + Act 额外发现的第三处 `catalog.rs:1764` 在两种损坏形态下均不可达）⇒ 删除路径未改动，不新立 Iteration、不登记 Issue） |
| 005 | 恢复面组的有界读（ISS27 · T5） | T5 | 004 | accepted（唯一 Cycle `iterations/005-recovery-bounded-read/000-initial.md`；**Plan Review 2026-10-26：Acceptance A16–A19 全部满足**；5 处恢复面读点全部在取槽之前消费 Iteration 002 冻结的唯一页级判据、槽负载全部经 `get_slot_data_checked`、`src/wal/recovery.rs` 内 `get_slot_data` 零调用点；扫描类（W1 / W3 / W4）跳过并**报出**（局部计数 + stderr，点名该页与该槽序号），重放类（W2 / W5）以 `CorruptedDataPage` 明确错误结束恢复与打开（未新增错误变体 / 命令参数 / 配置项 / 缓存 / 闸门机制）；8 项 Deviation 均为非实质偏差：**Deviation ① 裁定为 Plan 调查事实的偏差**（Cycle 的「W2 越界槽分支不可达」判定被 `redo_record` Insert 臂对同一份候选映射的再次写入推翻 ⇒ 该分支**实际可达**且由用例 C 直接覆盖 ⇒ 以**追加标注**更正失效指示，冻结的 Plan Context 正文未改写）；**Deviation ②**（新增用例 D 承载 W4）裁定为**非阻塞的测试顺序偏差** —— Cycle 层面满足「没有先行测试不变更」（用例 A / B / C 变更前即真实 RED），W4 单点层面的测试写在实现之后（以回退 W4 取得真实 RED），该事实已在 Cycle 的 `Plan Review` F6 补记；F7–F10 为形态 / 表述层面（报出多一行汇总、Target 行号按符号重定位、页级 RED panic 文本与预测不同、夹具用 `flush_all` / CLI 用读写模式 `checkpoint`）；1 条非阻塞 Minor finding（W2 页级分支在当前恢复流下不可达，已按 Requirement 实现）；**无 Issue 候选**） |
| 006 | DDL 提交点统一模型 | T6, T7, T8 | 005 | accepted（唯一 Cycle `iterations/006-ddl-commit-point-unified-model/000-initial.md`；**Plan Review 2026-10-26：Acceptance A20–A28 全部满足**。窄化意图是事务作用域 WAL 记录 `0x0A`，恢复在重放之前发布已提交意图；发布以目录列数为幂等判据；checkpoint 经 `Database::checkpoint` 先发布再截断。`ADD COLUMN` 失败补偿只改脏页、不刷盘。同表第二条窄化 DDL 在改写前取不到令牌即拒绝并点名该表。定向回归两批各 8 个目标、批内 `--test-threads=1`、批间串行，两批 0 failed；`cargo test --lib` 420 passed / 0 failed。全量未跑（预算在 009 / T16）。非阻塞：测试先于回退观察 RED；夹具用三列表；两个边界 Scenario 无独立用例。Issue 候选两条，只报告不登记） |
| 007 | 写盘失败的状态面 | T9, T10 | 006 | accepted（唯一 Cycle `iterations/007-write-failure-state/000-initial.md`；**Plan Review 2026-10-26：Acceptance A29–A33 全部满足**。在途写入不按干净页移出池，写盘失败后新字节仍在池内且错误上抛。失败集合按页，锚点仍失败时不提升，成功重写后提升。`id < limit`、刷盘失败 discard、R1 保持。定向回归一批 4 个目标、批内 `--test-threads=1`，然后单独 `cargo test --lib -- --test-threads=1`：11 / 12 / 2 / 3 passed，`--lib` 421 passed / 0 failed。计划文本中的目标名 `drop_persistence_boundary_test` 不是 cargo 目标，实际跑的是 `drop_persistence_test`。全量未跑（预算在 009 / T16）。非阻塞：用例名仍描述修订前的 R2；「写成功且等待期间又被改脏」已实现，验收表没有单独一行。无 Issue 候选） |
| 008 | dump / restore 的合同面 | T11, T12 | 007 | accepted（唯一 Cycle `iterations/008-dump-restore-contract/000-initial.md`；**Plan Review 2026-10-26：Acceptance A40–A44 全部满足**。`UNIQUE` 仅在 `ColumnType::Int` 时输出。存量 `TEXT` + `unique` 的 dump / schema 不含 `UNIQUE`，restore 退出码 0，重复值仍在，新库再插入成功。整数 `UNIQUE` 仍渲染并拒绝重复值。旧 dump 与计划期拒绝未改。多表部分恢复退出码 3，点名语句 3 / 4，此前语句已提交，`kept` 的行还在，`later` 不存在；`restore` 与 `sql_failure_message` 无 diff。定向回归一批 3 个目标，名字典序，批内 `--test-threads=1`：`cli_test` 126 passed / 2 ignored / 0 failed，`legacy_compat_test` 16 passed / 0 failed，`planner_test` 107 passed / 0 failed；然后单独 `cargo test --lib -- --test-threads=1`，421 passed / 0 failed。全量未跑（预算在 009 / T16）。无 Issue 候选） |
| 009 | 语料收口与收尾全量回归 | T13, T14, T15, T16 | 008 | accepted（唯一 Cycle `iterations/009-corpus-and-full-regression/000-initial.md`；**Plan Review 2026-10-26：Acceptance A45–A48 全部满足**。交接文件有五对标题，增量仍用旧标题，主规格库无 diff。两份 README 插入规定段落，表格数字未改；`wal-auto-checkpoint` 增量逐字保留原 Requirement，只追加边界 Scenario。13 条 Purpose 替换各带占位句中的 change id，`wal-recovery-replay-integrity` 标明不替换。全量 `N = 148`，前 8 批各 11、后 5 批各 12，`--lib --bins` 为第 00 批，批内 `--test-threads=1`，批间串行。14 批退出码都是 0，150 条汇总的 failed 都是 0；覆盖核对缺、多、重复都是 0。没有无过滤全量，没有默认并行度。无 Issue 候选。009 是 Map 的最后一项，不展开后继 Iteration） |

**Next Iteration**: None。009 是 Map 的最后一项，已于 2026-10-26 被 Plan Review 判定 `accepted`。不展开后继 Iteration。此前唯一 Cycle 为 `iterations/009-corpus-and-full-regression/000-initial.md`。Iteration 008 已于 2026-10-26 被 Plan Review 判定 `accepted`。**（2026-10-25 结构修正见 `design.md` D14：原 Iteration 001 承载 T3 / T17 / T4 / T5，现拆为各自承载单一任务的 Iteration 001 / 002 / 003 / 004；各任务的 Task Definition / Acceptance / 验证内容未变。2026-10-25 二次修正见 `design.md` D15：写路径组插入为 Iteration 003，其后依次顺延。2026-10-26：Iteration 003 被接受，Next Iteration 推进到 004；Iteration 003 Plan Review 的 Minor finding（目录删除路径的错误丢弃 / 折叠）以**追加标注**记入 `iterations/004-catalog-bounded-read/000-initial.md`，不新立 Iteration、不顺延 005–009。2026-10-26：Iteration 004 被接受（追加标注 F9 经逐条可达性论证**关闭**，含 Act 额外发现的第三处 `catalog.rs:1764`），Next Iteration 推进到 005 并由 Plan 展开；下游 006–009 **顺延不变**。2026-10-26：Iteration 005 被接受（Deviation ① 裁定为 Plan 调查事实的偏差并以**追加标注**更正失效指示；Deviation ② 裁定为非阻塞的测试顺序偏差并补记测试顺序事实），Next Iteration 推进到 006 并由 Plan 展开；下游 007–009 **顺延不变**。2026-10-26：Iteration 006 被接受（A20–A28 满足；定向回归两批各 8、批内 `--test-threads=1`；全量未跑，预算仍在 009 / T16），Next Iteration 推进到 007 并由 Plan 展开；下游 008–009 **顺延不变**。2026-10-26：Iteration 007 被接受（A29–A33 满足；定向回归一批 4 个目标、批内 `--test-threads=1`，然后单独 `cargo test --lib`；全量未跑，预算仍在 009 / T16），Next Iteration 推进到 008 并由 Plan 展开；下游 009 **顺延不变**。2026-10-26：Iteration 008 被接受（A40–A44 满足；定向回归一批 3 个目标、批内 `--test-threads=1`，然后单独 `cargo test --lib -- --test-threads=1`；全量未跑，预算在 009 / T16），Next Iteration 推进到 009 并由 Plan 展开。2026-10-26：Iteration 009 被接受（A45–A48 满足；全量 14 批、批内 `--test-threads=1`、批间串行，每批 failed 为 0）。Next Iteration 改为 None，不展开后继 Iteration。）**
