# Tasks：开放缺陷批（存储/DDL 实质缺陷 + 旧格式兼容证据缺口）

状态：`pending` / `in_progress` / `done` / `blocked`。完整状态以 change 产物（Cycle 的 `Act Response`）为准。

**本 change 不对应任何 MSxx**——它是 MS35 / MS36 / MS34 修复批遗留缺陷与证据缺口的承载批（见 `proposal.md` 的 Why）。

## 任务列表

| ID | 任务 | Iteration | 状态 |
|---|---|---|---|
| T1 | B-2 先行 RED 用例：叶合并后被合并掉的页只进入待释放集合一次、可分配集合中同页号不出现两次 | 000 | done |
| T2 | 消除合并路径的重复释放（`handle_child_merge` 不再重复释放已由 `merge_leaves` 释放的页）——使 T1 GREEN | 000 | done |
| T3 | ISS23 复现与归属判定：大规模删列改写不因索引下降耗尽栈；B-2 修复后仍复现则定位下降环并修复，归属结论写入 Act Response | 000 | done |
| T4 | ISS22 先行 RED 用例：跨页容量边界的删列改写后行数不减、数据页链不分叉 | 001 | done |
| T5 | 改写路径的单一尾游标（DELETE 腿与 INSERT 腿经由同一条自愈并回写目录尾指针的写入口）——使 T4 GREEN | 001 | done |
| T6 | ISS24 先行 RED 用例 + M1/M2 判别：被放弃的删列改写在回收后不隐藏已改写的行；判别缺失行是否落在链上不可达的页 / 是否存在已回滚墓碑链接 | 001 | done |
| T7 | 闭合 M2（以 `next_version` 构造抑制集合的活性规则不承认已回滚指向者）；若判别为 M1 则由 T5 一并关闭——使 T6 GREEN | 001 | done |
| T8 | B-4 先行 RED 用例：损坏页（槽位偏移越界）上的 `DROP TABLE` / `TRUNCATE` / `DROP INDEX` 不 panic、不 abort | 002 | done |
| T9 | 释放路径的槽位扫描改用有界读（与 `collect_data_chain_strict` 同一来源的 integrity 模块读法），损坏页按既有尽力而为语义处理——使 T8 GREEN | 002 | done |
| T10 | ISS25 先行 RED 用例 + 故障注入夹具：在途窄化窗口内的读不发生位置性静默错读（显式拒绝），且未改写的表读路径不受影响 | 003 | done |
| T11 | 在途窄化状态的发布/清除（改写开始发布、提交点清除）+ 读路径的在途检查与显式拒绝——使 T10 GREEN | 003 | done |
| T12 | B-1 先行 RED 用例：写盘失败后该页仍为脏、后续成功刷盘重写它；锚点写未落盘时释放不被提前提升 | 004 | done |
| T13 | 刷盘失败保持脏标记（`flush_all` / `flush_all_except` / `flush_pages`）+ 释放提升以「该释放完成之后未发生过页写盘失败」为前置条件（判据形态见 `design.md` D8）——使 T12 GREEN | 004 | done |
| T14 | DML 墓碑放弃窗口先行 RED 用例：被放弃（丢弃或超时）的 `DELETE` 不让被删行不可见（ISS24 的 DML 同形面，`tests/dml_tombstone_abandon_window_test.rs`） | 005 | done |
| T15 | DML 墓碑写序收敛（先写不带链指针 → 登记 → 补链指针 → 再清理索引条目），使「写出版本槽」与「登记版本」之间的放弃窗口内墓碑不压制前驱版本——使 T14 GREEN | 005 | done |
| T16 | G2 文件级样本产出（仓库之外的隔离克隆 + `cargo build --offline --bin rtsql`）并固定入库；样本来源说明更新（`tests/fixtures/legacy/README.md`） | 007 | done |
| T17 | G2 样本的常态化断言（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复），派生量由断言锁定并与来源说明同步 | 007 | done |
| T18 | 三条常态化用例：恢复出的库可继续写入、旧 G1 可由当前构建 dump、G1 可连续写两次（`tests/legacy_compat_test.rs`） | 007 | done |
| T19 | 迁移路径实测记录的命令列更正为可直接执行的形式（路径形式），并保留「结论本身可复现、失配只在命令写法」的事实 | 007 | done |
| T20 | 规格增量与验收边界对齐 + 收尾交接清单（含 `legacy-format-compat` 证据索引更新交 docs-maintainer 的交接项） | 007 | done |
| T21 | 本 change 收尾全量回归（M19 分批串行） | 007 | done（14 批 / 143 目标 / 2231 passed / 0 failed / 2 ignored） |
| T22 | 索引树读路径的先行 RED 用例：损坏的索引页上普通 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 不 panic、不 abort，且不得静默「查不到该行」（B-4 同根因层的读 / 写热路径面，`tests/index_read_bounded_read_test.rs`） | 006 | done |
| T23 | 索引树访问器族改用有界读 + 损坏索引页的**显式错误**语义（点名损坏的页与索引，SHALL NOT 跳过子树）——使 T22 GREEN | 006 | done |

## 任务执行依据要点

- **T1**：用例必须走真实入口（真实文件库 + 真实 `Database::open` / DML / `ALTER TABLE DROP COLUMN`），不得在框架外另建判定层。可分配页集合用既有可观察量断言（如 `storage.page_count()` / 完整性扫描），**不新增指标**。
- **T2**：修复面限于「同一页号在一次删除操作中被推进待释放集合两次」；**不改** `free_page` / `complete_free_release` 的集合语义（MS35 已验收的 `free-page-lifecycle` R1 的判定时点不变）。
- **T3**：判定协议见 `design.md` D3。**ISS23 的规模约束**：本 change 内不使用 ≥ 300 行的表规模构造用例；若必须用更大规模，先解决 ISS23 本身。归属结论（B-2 的下游后果 / 独立成因）与其定位证据必须写入 Act Response；若定位需要超出当前 Task Definition 的新执行依据，按 Review 分类退回 Plan（`replan-required`），**不得**在 Iteration 000 内扩大范围。
- **T4 / T5**：改写路径必须收敛到**单一尾游标**；DELETE 腿不得继续直接调用底层写入口（该入口不做陈旧尾自愈、不回写目录尾指针）。目录**列**的窄化时机不变（`ddl-catalog-transactional` R1/R2 不变）。
- **T6 / T7**：判别步骤是 RED 用例的一部分，不是可选项（见 `design.md` D5）。M2 的修复面限于「以 `next_version` 链接构造抑制集合的活性规则」：`table_manager.rs` 的 `alter_drop_column` / `table_has_live_rows`、`executor/create_index.rs` 的回填、`wal/recovery.rs` 的 `rebuild_pk_indexes`——判据与 `DataScanExecutor::superseder_suppresses` 的既有判据同源（`create_tx_id == 0` 的 aborted 标记不抑制）。**不改** `VersionHeader::mark_aborted` 的「保留链指针」语义。
- **T8 / T9**：损坏夹具必须**真实损坏**页内容（越界的槽位偏移/长度），不得用「跳过读」之类的替身；释放路径的返回语义保持既有的尽力而为（告警 + 跳过 + 返回），**不改** `drop_table` / `truncate_table` 的成功/失败语义。**不改** `SlottedPageRef::get_slot_data` 的签名（波及全部调用点，属范围外候选）。
- **T10 / T11**：故障注入夹具按既有形态设计——路径限定、一次消费、默认不生效（先例：`src/storage/free_page_rebuild.rs` 的 `arm_free_page_rebuild_fault`）。**不得**引入身份证明机制（run-id / 指纹 / revision pin）。在途状态**只在内存**（不改磁盘格式，见 `design.md` D7）；改写窗口的锚点用**可观察状态 + `yield_now` 轮询**定位，**timer 只作夹具失控兜底**。
- **T12 / T13**：修复面限于 (a) 写盘失败时恢复该页的脏标记；(b) 释放提升时要求「该释放完成之后未发生过页写盘失败」（原表述「锚点写所在页**已不脏**」经 Iteration 004 的 Plan Review 2026-10-25 实测排除——取样点取法与 MS35 已验收的提升互斥、刷盘后取法判据落空，见 `design.md` D8）。**不改** checkpoint 九步的顺序、**不改** MS35 已验收的释放完成编号与取样时点语义。
- **T14 / T15**：DML 墓碑与 DDL 改写的墓碑**同形**——写序必须是「先写（不带链指针）→ 登记 → 补链指针 → 再清理索引条目」，使「写出版本槽」与「登记版本」之间的放弃窗口内墓碑**不带链指针**（不压制任何前驱版本）。修复面限于 `src/executor/delete.rs`（及同形的 `upsert.rs` 墓碑路径）的**写序**；**不改** `VersionHeader::mark_aborted` 的链指针语义、**不改**既有域 `mvcc-tombstone-visibility` 的判据（`superseder_suppresses`）。用例走真实入口（真实 `DELETE` 语句 + 锚点式放弃 + 下一条写语句触发回收），断言锚定真实可观察行为（可见 id 集合 / 点查命中数）；**不得**引入身份证明机制、判定层或 timer 阈值判据（timer 只作夹具失控兜底）。
- **T16**：旧构建在**仓库之外**的隔离目录产出（`git clone --no-hardlinks --shared` + `git checkout <G2 提交>` + `cargo build --offline --bin rtsql`）；样本**固定入库**为固定输入。**不得**运行时伪造版本或行布局；**不得**用 hash/revision 比对证明样本身份。G2 样本行数必须 **≪ 300**（避开 ISS23 触发面）。
- **T17**：派生量（如恢复行数）由**断言**锁定；样本重新生成时**必须同步**更新 `tests/fixtures/legacy/README.md` 的来源说明与用例中的参考值常量。
- **T18**：三条用例走**真实入口**（CLI 或库 API 的真实调用），断言强度不得低于原一次性实测记录。
- **T19**：只改记录文本，不改任何已记录的**结论**（退出码与结果本身可复现）。
- **T20**：只对齐本 change 的规格增量文本与验收边界，**不改** `openspec/specs/`；`legacy-format-compat` 的证据索引更新作为**交接项**列出。
- **T21**：执行依据见下方「收尾验证」，纪律逐条写死在 `iterations/007-legacy-compat-evidence/000-initial.md` 的 T21 段。
- **T22 / T23**：损坏夹具沿用 Iteration 002 的形态（真实文件库 + 真实 `Database::open` → 按文件偏移把**索引树页**的槽目录项 offset 字段改写为越界值 → 重开 → 执行真实 `SELECT` / `INSERT` / `DELETE` / `UPDATE`）。修复面限于 `src/storage/btree/node.rs` 的**索引树访问器族**（8 处 `get_slot_data`）及其调用点的错误语义；损坏的索引页 SHALL 给出**显式错误**（点名损坏页与索引），SHALL NOT 跳过子树（跳过 ⇒「查不到该行」⇒ 静默错读），SHALL NOT abort。**不改** `SlottedPageRef::get_slot_data` 的签名、**不改**磁盘格式、**不改**未损坏路径的读 / 写行为。判定锚定真实可观察行为（语句返回明确错误 / 正常结果，进程不 abort）；**不得**引入身份证明机制、判定层或 timer 阈值行为判据。

## Requirements Traceability Matrix

`index-page-ownership`（IPO）、`ddl-rewrite-consistency`（DRC）、`release-path-bounded-read`（RPBR）、`release-durability-precondition`（RDP）、`index-read-bounded-read`（IRBR）五个**本 change 新增**规格域；另引用既有域 `mvcc-tombstone-visibility`（MTV）与 `ddl-catalog-transactional`（DCT）承载 ISS24 的目标行为。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| IPO R1 被合并掉的树页只释放一次 | T1, T2 | `tests/index_page_ownership_test.rs::merged_index_page_enters_the_allocatable_set_once`、`::sibling_merge_releases_the_merged_page_once`、`::internal_merge_releases_the_merged_page_once` | Done |
| IPO R2 被释放的页不同时属于两个对象 | T1, T2 | 同上（三个用例的可分配集合断言）+ `tests/index_page_ownership_test.rs::alter_table_drop_column_completes_without_exhausting_the_stack`（索引树子指针遍历无重复引用） | Done |
| IPO R3 索引插入路径的下降必然终止 | T3 | `tests/index_page_ownership_stack_overflow_test.rs::released_index_page_never_breaks_the_insert_descent`（ISS23 复现）、`tests/index_page_ownership_test.rs::insert_into_a_cyclic_structure_returns_an_error`（成环时返回错误）、`::alter_table_drop_column_completes_without_exhausting_the_stack` | Done |
| DRC R1 列改写不丢失已提交行 | T4, T5 | `tests/ddl_rewrite_consistency_test.rs::drop_column_across_page_boundary_keeps_every_row_on_one_chain`（含链可达性断言；Act 记录改名，原计划名 `…_keeps_all_rows`） | Covered |
| DRC R2 被放弃的列改写不隐藏已改写的行 | T6, T7 | `tests/ddl_rewrite_consistency_test.rs::abandoned_drop_column_keeps_rewritten_row_visible` | Covered |
| DRC R3 改写在途期间读路径不静默错读 | T10, T11 | `tests/ddl_rewrite_consistency_test.rs::read_during_narrowing_in_flight_is_refused`（Act 记录的实际用例名；同文件另两条对照用例 `::read_after_narrowing_completes_returns_narrowed_rows`、`::read_of_other_table_during_narrowing_is_unaffected`） | Done |
| RPBR R1 释放路径读取页内容时有边界检查 | T8, T9 | `tests/release_path_bounded_read_test.rs::drop_table_on_damaged_data_page_does_not_panic`、`::truncate_on_damaged_data_page_does_not_panic`、`::drop_index_on_damaged_index_page_does_not_panic`、`::undamaged_release_paths_free_the_expected_pages`（+ 三条真实 CLI 入口用例 `::drop_table_cli_reports_the_damaged_page`、`::truncate_cli_reports_the_damaged_page`、`::drop_index_cli_reports_the_damaged_page`；Act 记录的实际用例名） | Done |
| RDP R1 刷盘失败不丢弃脏页写 | T12, T13 | `tests/release_durability_precondition_test.rs::failed_write_keeps_page_dirty` | Done |
| RDP R2 释放的可分配判定以锚点写已持久为准 | T12, T13 | `tests/release_durability_precondition_test.rs::release_not_promoted_when_anchor_write_not_durable` | Done（提升判定已实现并验证；场景 1 原文的「重开后该页不被判为可分配页」半句**不可满足**，已由 Iteration 004 的 Plan Review 2026-10-25 就地改写为可达且已验收的观察——见 `specs/release-durability-precondition/spec.md`） |
| IRBR R1 损坏的索引页在读 / 写路径上给出明确的错误（本 change **新增**域） | T22, T23 | `tests/index_read_bounded_read_test.rs::damaged_index_page_makes_select_fail_with_a_named_error`（计划名；含 `INSERT` / `DELETE` / `UPDATE` 三条同形用例与一条未损坏对照用例） | Done（Iteration 006 的 Plan Review 2026-10-25：Acceptance 1–6 全部满足） |
| MTV R1 墓碑自描述且不影响前驱版本（既有域，ISS24 的目标行为） | T6, T7 | `tests/ddl_rewrite_consistency_test.rs::abandoned_drop_column_keeps_rewritten_row_visible`（M2 判别断言） | Covered |
| MTV「未提交与已回滚墓碑不抑制、评估回溯前驱」（既有域，ISS24 的 **DML 同形面**） | T14, T15 | `tests/dml_tombstone_abandon_window_test.rs::abandoned_delete_keeps_the_deleted_row_visible` | Done |
| DCT R3 放弃 DDL 之后重开库目录与数据面一致（既有域，ISS24 的目标行为） | T6, T7 | `tests/ddl_rewrite_consistency_test.rs::abandoned_drop_column_reopen_keeps_all_rows` | Covered |

**C 组（C-1 / C-2 / C-3）不产生规格增量**：三条条款的行为已写在 MS36 的**待合并**增量 `legacy-format-compat` 中（本 change 不能 MODIFY 一个尚未进入 `openspec/specs/` 的域，否则增量无法应用）。C 组的验收由 **T16**（G2 文件级样本）、**T17**（G2 样本的常态化断言）、**T18**（三条一次性条款常态化）、**T19**（迁移路径记录的命令列更正）的任务验收条目承载；`legacy-format-compat` 的证据索引（E2 / E4 / E7 与 G2 行的能力格）按实际用例名更新，属 `openspec-docs-maintainer` 的合并职责——该交接项由 **T20**（规格增量与验收边界对齐 + 收尾交接清单）成文。**T20** 另承载三处**措辞**对齐（既有域 `mvcc-tombstone-visibility` 的 AR2 澄清、既有域 `release-path-bounded-read` 的「尽力而为」AND 子句、本 change 增量域 `index-read-bounded-read` 的 Requirement 措辞收敛——后者由 Iteration 006 的 Plan Review F6 追加登记），**只改措辞、不改行为要求**。

**T20 的三处措辞对齐落点（Iteration 007 / Cycle 000 实施结果）**：

| 对齐点 | 落点 | 形态 |
|---|---|---|
| RPBR「尽力而为」AND 子句（`DROP INDEX` 场景缺该子句） | 本 change 增量 `specs/release-path-bounded-read/spec.md` | **就地补子句**（与同域 `DROP TABLE` 场景同形） |
| IRBR Requirement 措辞收敛（Iteration 006 的 Plan Review F6） | 本 change 增量 `specs/index-read-bounded-read/spec.md` | **就地改措辞**：「读取到损坏的索引页时」→「读取到**未通过页级判据**的索引树页时」（Purpose 同词一并对齐） |
| MTV 的 AR2 字面读法澄清（Iteration 005 的 Plan Review F3） | **交接项**（见下「收尾交接清单」**第 5 条**）——`openspec/specs/` 按本 change 的执行依据**不修改**，措辞以「改动前 → 改动后」原文交 `openspec-docs-maintainer` 应用 | **不成文入库**，措辞随交接清单成文 |

C 组任务状态同步（Iteration 007 / Cycle 000 收尾）：T16–T21 全部 **done**；`legacy_compat_test` 由 9 条增至 **16** 条（新增 G2 四条 + 常态化三条）；收尾全量 **2231 passed / 0 failed / 2 ignored**（14 批 / 143 目标）。

**B-3 不在本矩阵内**：已被 MS35/MS36 已验收规格显式界定为范围外既有行为，不承载（见 `design.md` D9）。

命名说明：矩阵中的测试名是**计划中的用例名**；Act 可在不改变断言语义的前提下调整命名，并在 Act Response 记录实际名称。

## Iteration Plan

### Iteration 000: 索引页归属唯一性——合并页只释放一次，下降必然终止

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: 索引树删除路径上的叶合并对被合并掉的右叶页**只释放一次**；被释放的页**不会被交给两个对象**（可分配集合中同一页号不出现两次）；索引插入路径的下降**必然终止**——`ALTER TABLE DROP COLUMN` 在索引树多层规模下不再因下降成环而栈溢出。⇒ ISS23 的归属结论确定（`design.md` D3 的两条路径之一）。
- Verification boundary: 独立完成判据为 T1–T3 的定向用例通过（`cargo test --test index_page_ownership_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**：全量是本 change 的收尾验证，预算记在 Iteration 007（一次）。
- Diagnostic boundary: 失败时按「叶合并的释放点 → `MergeInfo.freed_page_id` 的传递 → `handle_child_merge` 的重复释放 → 待释放集合的入队 → 提升后的可分配集合 → 插入路径的下降页号序列」六段定位；ISS23 若在 T2 之后仍复现，判读依据是「gdb 栈上的同 `page_id` 帧序列」与「该页号在待释放集合中的出现次数」。
- Non-goals: DDL 改写面的数据链与可见性（Iteration 001）；释放路径的有界读（Iteration 002）；改写窗口的读一致性（Iteration 003）；释放可分配判定的耐久性前提（Iteration 004）；DML 墓碑的放弃窗口（Iteration 005）；索引树读路径的有界读（Iteration 006）；旧格式兼容证据（Iteration 007）；MS35 已验收的 `free-page-lifecycle` 判定时点语义。

### Iteration 001: 列改写的数据链一致性与放弃可见性

- Tasks: T4, T5, T6, T7
- Depends on: Iteration 000
- Stable result: `ALTER TABLE DROP COLUMN` 的改写使用**单一尾游标**、数据页链**不分叉**、已提交行**不丢失**（跨页容量边界与重开库两个维度均可观察）；被放弃的改写在回收后**不隐藏**它已开始改写的行，且**已回滚的改写墓碑不抑制其前驱版本**（既有域 `mvcc-tombstone-visibility` R1 的实现面闭合）。
- Verification boundary: 独立完成判据为 T4–T7 的定向用例通过（`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**。
- Diagnostic boundary: 失败时按「改写两腿的尾游标 → 页写满时的链 `next` 改写 → 链可达性（是否存在不可达页）→ 墓碑的 aborted 标记 → 抑制集合的构造」五段定位；缺失行的判读依据是「缺失行是否落在链上不可达的页」与「其版本链上是否存在 `create_tx_id == 0` 的指向者」。
- Non-goals: 改写窗口的读路径可见性（Iteration 003）；目录列定义的提交点时机（既有域 `ddl-catalog-transactional` 不变）；`ADD COLUMN` 路径的宽度感知解码语义；`VersionHeader::mark_aborted` 的链指针语义。

### Iteration 002: 释放路径的有界读

- Tasks: T8, T9
- Depends on: Iteration 001
- Stable result: `DROP TABLE` / `TRUNCATE` / `DROP INDEX` 在**页内容损坏**时不再 panic、不再导致进程 abort；释放路径的槽位扫描经由有边界检查的读取路径，损坏页按既有尽力而为语义处理；未损坏路径的释放行为**零变化**。
- Verification boundary: 独立完成判据为 T8–T9 的定向用例通过（`cargo test --test release_path_bounded_read_test -- --test-threads=1` + 既有释放相关用例 `cargo test --test drop_table_free_test -- --test-threads=1`、`cargo test --test drop_persistence_test -- --test-threads=1`、`cargo test --test free_page_lifecycle_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**。
- Diagnostic boundary: 失败时按「损坏夹具的形态（槽位偏移/长度越界）→ 释放路径的扫描点 → 有界读的判据 → 尽力而为的告警与跳过」四段定位。
- Non-goals: `SlottedPageRef::get_slot_data` 的签名改造（波及全部调用点，属范围外候选）；完整性扫描的诊断分级（既有域 `data-integrity-verification` 不变）；释放路径的返回语义变更。

### Iteration 003: 改写窗口的读一致性

- Tasks: T10, T11
- Depends on: Iteration 001
- Stable result: `ALTER TABLE DROP COLUMN` 的**在途窄化状态**对读路径可见：处于该状态的表被读时给出**明确拒绝**（点名该表正在被修改），SHALL NOT 返回按位置错位的值；改写完成后读路径恢复正常；未改写的表不受影响；`ADD COLUMN` 的窄行解码语义不变。
- Verification boundary: 独立完成判据为 T10–T11 的定向用例通过（`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**。
- Diagnostic boundary: 失败时按「在途状态的发布点 → 提交点的清除 → 读路径的检查点 → 拒绝错误的形态 → 夹具的窗口定位」五段定位；窗口定位用**可观察状态 + `yield_now` 轮询**，timer 只作夹具失控兜底。
- Non-goals: 磁盘格式（不改 `VersionHeader` / 目录行布局——会破坏 MS36 已验收的 G1/G2/G3 兼容面）；读路径等待 DDL 完成的阻塞语义（会触及执行器生命周期与 `statement-cancellation-outcome` 域）；崩溃后在途窗口的持久化（**当前无人承担**：在途状态是纯内存、崩溃即消失 ⇒ 重开后目录仍广告旧列数而数据面已是窄化的已提交行 ⇒ 读仍按位置错位；关闭它需持久标记或移动提交点，分别与 MS36 兼容面、ISS21 / D5 冲突 ⇒ 见 `design.md` D7 的就地更正与 Iteration 003 的 Plan Review 2026-10-25 登记的 Issue 候选，ISS25 保持 `open`）；索引树访问器的无界读（Iteration 006 承载）。

### Iteration 004: 释放可分配判定的耐久性前提

- Tasks: T12, T13
- Depends on: Iteration 002
- Stable result: 写盘失败**不丢弃**脏页写（失败页保持脏，后续成功刷盘重写它）；释放被提升为可分配时其**锚点写已持久**，未持久化的释放**不被提前提升**（判据：该释放完成之后未发生过页写盘失败——形态见 `design.md` D8）；提升后的页在重开库时与持久目录一致。
- Verification boundary: 独立完成判据为 T12–T13 的定向用例通过（`cargo test --test release_durability_precondition_test -- --test-threads=1` + 既有 `cargo test --test free_page_lifecycle_test -- --test-threads=1`、`cargo test --test drop_persistence_test -- --test-threads=1`、`cargo test --test checkpoint_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**。
- Diagnostic boundary: 失败时按「刷盘的两阶段（清脏标记 → 写盘）→ 失败页的脏标记 → 锚点写所在页 → checkpoint 第 2 步的取样与提升 → 提升后的可分配集合 → 重开库重建」六段定位；「提升过早」与「永远不提升」是两个相反的失效方向，分别用「重开库后旧内容缺失」与「库文件持续增长」判读。
- Non-goals: checkpoint 九步的顺序与 MS35 已验收的释放完成编号取样时点；`DROP TABLE` 的崩溃边界模型（既有域 `drop-persistence-boundary`）；DDL 语句边界落盘的顺序语义（B-3，见 `design.md` D9）；真实断电模型（沿用 R49 的进程内丢弃句柄形态）。

### Iteration 005: DML 墓碑的放弃窗口收敛（ISS24 的同形面）

- Tasks: T14, T15
- Depends on: Iteration 001
- Stable result: 被放弃（丢弃或超时）的 `DELETE` 在回收后**不让被删行不可见**——DML 墓碑的写序与 DDL 改写路径**同形**（先写不带链指针 → 登记 → 补链指针 → 再清理索引条目），既有域 `mvcc-tombstone-visibility` 的「未提交与已回滚墓碑不抑制、评估回溯前驱」在 DML 路径上闭合；DDL 改写路径的既有成果（Iteration 001）不受影响。
- Verification boundary: 独立完成判据为 T14–T15 的定向用例通过（`cargo test --test dml_tombstone_abandon_window_test -- --test-threads=1` + 既有 `cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1`、`cargo test --test mvcc_abort_test -- --test-threads=1`、`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**。
- Diagnostic boundary: 失败时按「墓碑的写序 → 登记点 → 链指针的补写点 → 索引条目的清理点 → 回收时的中性化」五段定位；夹具用**可观察状态 + `yield_now` 轮询**锚定放弃点，timer 只作夹具失控兜底。
- Non-goals: DDL 改写路径（Iteration 001 已闭合）；`VersionHeader::mark_aborted` 的链指针语义；既有 MTV 判据（`superseder_suppresses`）；崩溃后未提交墓碑的可见性（登记为 Issue 候选，由 Recorder 承载）。

### Iteration 006: 索引树读路径的有界读与显式拒绝

- Tasks: T22, T23
- Depends on: Iteration 002
- Stable result: 损坏的索引树页在**读 / 写热路径**（`SELECT` / `INSERT` / `DELETE` / `UPDATE` 的索引下降与条目提取）上**不再越界切片 panic、不再导致进程 abort**；损坏的索引页给出**明确的错误**（点名损坏的页与索引），SHALL NOT 被跳过成「查不到该行」（那会是另一种静默错读）；未损坏路径的读 / 写行为**零变化**。⇒ 承载面：新增规格增量域 `index-read-bounded-read`（IRBR，见 `specs/index-read-bounded-read/spec.md`）。
- Verification boundary: 独立完成判据为 T22–T23 的定向用例通过（`cargo test --test index_read_bounded_read_test -- --test-threads=1` + 既有 `cargo test --test index_page_ownership_test -- --test-threads=1`、`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1`）。**本 Iteration 不跑全量回归**。
- Diagnostic boundary: 失败时按「损坏夹具的形态 → 索引树的下降路径 → 访问器的有界读 → 损坏页的错误语义 → 调用点的错误传播」五段定位；「panic」与「静默查不到」是两个相反的失效方向（前者 = 未做边界检查，后者 = 把损坏页当成空子树），分别用「进程 abort」与「点查未命中而全表扫描命中」判读。
- Non-goals: 存储层其余无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处——各自的「损坏页正确行为」需单独判定，另立 change 承载）；`SlottedPageRef::get_slot_data` 的签名改造；磁盘格式；释放路径的有界读（Iteration 002 已闭合）；改写窗口的读一致性（Iteration 003）。

### Iteration 007: 旧格式兼容的证据补齐

- Tasks: T16, T17, T18, T19, T20, T21
- Depends on: Iteration 003, Iteration 004, Iteration 005, Iteration 006
- Stable result: G2 代际有**真实来自旧构建**的**文件级样本**（固定入库为固定输入）与其常态化断言（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复，派生量由断言锁定并与来源说明同步）；MS36 的三条一次性兼容条款转为**常态化用例**（恢复出的库可继续写入 / 旧 G1 可由当前构建 dump / G1 可连续写两次）；迁移路径记录的命令列**可直接执行**；本 change 的规格增量与验收边界对齐，收尾交接清单就绪。
- Verification boundary: 独立完成判据为 T16–T20 的定向用例通过（`cargo test --test legacy_compat_test -- --test-threads=1`）。
  **本 Iteration 承担本 change 的收尾验证：M19 分批全量回归（见下）**——按 M19 与 `.agents/runbooks/m19-batched-full-regression.md`（R48）执行一次，覆盖目标清单 = **执行时** `tests/` 全部测试文件 + `--lib --bins`。该收尾全量的执行依据是 **T21**（纪律逐条写死在 `iterations/007-legacy-compat-evidence/000-initial.md` 的 T21「M19 分批串行全量回归纪律」第 1–12 条）。
- Diagnostic boundary: 失败时按「旧构建产出 → 样本入库 → 样本形态断言（目录行代际）→ 派生量一致性 → 常态化用例的入口与断言」五段定位；派生量不一致时**先核对样本字节是否被重新生成**，不得就地改断言常量迁就样本。
- Non-goals: G1 / G3 的样本扩充与代际定义（MS36 已验收）；旧 dump 文本的支持范围（MS36 已验收）；加密旧库的专项断言（MS36 已记录为不覆盖）；MS36 规格增量本身的合并（属 docs-maintainer）。

## 收尾验证：M19 分批全量回归（记在最后一个 Iteration）

**时机**：本 change 的最后一个 Iteration（007）收尾时执行一次（全 change 一次预算）。中途如需追加全量，须在该 Iteration Plan 记录理由。

**执行纪律（SHALL）**：

1. 测试目标按**确定性顺序**分组（`--lib --bins` 一批；`tests/*.rs` 按文件名字典序），每组一次独立的 `cargo test --test <f1> ... --test <fN> -- --test-threads=1` 调用。
2. 批间**顺序执行**，同一时刻只有一批在跑。
3. **SHALL NOT** 用一次无目标过滤的全量调用跑完（无论串行还是默认并行度）。
4. **SHALL NOT** 用默认并行度跑任何全量批次（每批必须带 `--test-threads=1`）。
5. 批大小 **8–12 个目标**（按机器能力调整）。
6. 全量 Gate 等价于「**全部批次各自 0 failed**」；缺一批不得声明全量通过。
7. 结论必须记录**每批的 passed/failed 计数**，并做覆盖对账：目标清单 = 变更前 `tests/` 全部测试文件 + `--lib --bins`（分批不重不漏）。
8. 判定只看 cargo 自己的退出码与原生输出；计数可用一步通用管道读出（`… 2>&1 | grep -E "^test result:"`），管道只做计数、不做判定；**不得**新增判定脚本、封装命令或判定层。
9. 单次目标验证（`cargo test --test <name>`）不受本纪律约束，可用默认并行度。

**覆盖对账基准**：收尾时按实际文件列表重算 = 变更前 `tests/*.rs` 全部文件 + `--lib --bins`，并把本 change 新增的测试文件（`tests/index_page_ownership_test.rs`、`tests/ddl_rewrite_consistency_test.rs`、`tests/release_path_bounded_read_test.rs`、`tests/release_durability_precondition_test.rs`、`tests/dml_tombstone_abandon_window_test.rs`）与既有 `tests/legacy_compat_test.rs` 一并纳入。

**基线（2026-10-24 本 Plan 实测）**：`tests/*.rs` **135** 个文件 + `--lib --bins` = **136** 个目标 / **13** 批；最近一次全量终态 **2192 passed / 0 failed / 2 ignored**。

**基线对当前工作区已失效**（Plan Review 2026-10-25 / Iteration 007 登记）：该基线测于本 change 之前，此后有**七个 Iteration**的改动（本 change 新增 6 个测试文件 + 既有测试文件的改动）⇒ 执行时的目标清单按**实际文件列表**重算（Plan 时点为 `tests/*.rs` **142** 个 + `--lib --bins` = **143** 个目标），**期望终态**不再由基线数字反推；与基线的差额（目标数变化 + 新增用例数）**必须由实际结果说明**，且全量通过判据**只**是「**全部批次各自 0 failed**」（见 `iterations/007-legacy-compat-evidence/000-initial.md` 的 T21 第 7 / 10 / 11 / 12 条）。

## 收尾交接清单（T20，Iteration 007 / Cycle 000 成文）

本 change 的**增量规格合并**属 `openspec-docs-maintainer`（见上文「收尾验证」与 maintainer 的合并职责）；本清单是交 Maintainer 的**逐条**输入，不代做合并。

### 1. 各增量域的最终状态（5 个，全部 `## ADDED Requirements`）

| 增量域 | Requirement | Scenario | 最终状态 |
|---|---|---|---|
| `index-page-ownership`（IPO） | 3 | 6 | 定稿（Iteration 000） |
| `ddl-rewrite-consistency`（DRC） | 3 | 10 | 定稿（Iteration 001 / 003） |
| `release-path-bounded-read`（RPBR） | 1 | 4 | 定稿（Iteration 002 + 本 Iteration 的「尽力而为」AND 子句补写，Scenario 数不变） |
| `release-durability-precondition`（RDP） | 2 | 7 | 定稿（Iteration 004，含 Plan Review 就地改写的场景 1 半句） |
| `index-read-bounded-read`（IRBR） | 1 | 5 | 定稿（Iteration 006 + 本 Iteration 的 Requirement 措辞收敛，Scenario 数不变） |

本 change **不新增**规格域、不 MODIFY 已合并的既有域（C 组的理由见 RTM 段）。

### 2. 交 Maintainer：`legacy-format-compat` 证据索引更新（MS36 的待合并增量）

MS36 的增量 `legacy-format-compat` 仍**待合并**；其**证据索引**里三处指向「本 change 的真实 CLI 实测记录」的一次性观察，现已由**常态化用例**承载，合并时应按下列**实际用例名**替换：

| 证据项 | 现文（一次性实测记录） | 更正为（常态化用例，`tests/legacy_compat_test.rs`） |
|---|---|---|
| **E2（G1 写入）**末句 | 「**写入后仍能再次写入**由本 change 的真实 CLI 实测记录（同一 G1 旧库上连续两次写入，退出码均为 0，写入后查询可见）」 | 「**写入后仍能再次写入**由 `tests/legacy_compat_test.rs::legacy_g1_db_accepts_two_consecutive_writes` 覆盖（同一 G1 旧库上连续两次写入，退出码均为 0，写入后查询可见：users 5 行、audit 2 行）」 |
| **E4（G1 dump）**末句 | 「**当前构建导出 G1 旧库**由本 change 的真实 CLI 实测记录（退出码 0、导出按当前格式渲染长类型名）」 | 「**当前构建导出 G1 旧库**由 `tests/legacy_compat_test.rs::current_build_dumps_legacy_g1_db` 覆盖（退出码 0、stderr 空、导出 3 表 + 全部行；导出文本可恢复到新库且行集等价）」 |
| **E7（G2 无专属断言）** | 「**没有**按 G2 形态构造的用例，仓库内也没有 G2 布局的文件级样本」 | 「G2 形态已有**文件级样本**与断言：样本 `tests/fixtures/legacy/legacy_g2.db`（由 revision `6e8dcf5` 的旧构建在仓库之外产出，来源与再生步骤见 `tests/fixtures/legacy/README.md`）；用例 `tests/legacy_compat_test.rs::legacy_g2_catalog_rows_are_in_g2_generation`（目录行形态：含唯一根段、不含二级索引段与表类型字节）、`::legacy_g2_db_opens_queries_and_writes`（打开 / 查询 / 写入 / 完整性 + 被触及表的目录行升级）、`::legacy_g2_db_open_rebuild_keeps_live_table_intact`（打开时重建不放弃）、`::legacy_g2_dump_restores_into_current_db`（dump / 恢复往返等价）」 |

**E7 的替换方式（Iteration 007 的 Plan Review 2026-10-25 补注）**：上表 **E2 / E4 的「现文」与 MS36 增量 `specs/legacy-format-compat/spec.md` 的对应文本**逐字相等 ⇒ 可按「现文 → 更正为」直接 find-and-replace；**E7 的「现文」是节选**（省略了「（唯一根段存在、二级索引段与表类型字节缺失）」与后半句「G2 与 G1 的唯一差别…」）⇒ **不能**按字符串替换，须**整条替换**该证据索引里的 E7 条目（以「- **E7（G2 无专属断言）**：」开头的整行）。

**能力矩阵的 G2 行**（「逐代能力矩阵」）随之更新：G2 行的 7 列由「行级既有覆盖（E6）/ 无 G2 专属断言（E7）」改为**文件级样本（E7）**（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复七列均已由上列用例覆盖）。

另：R4 S1「恢复出的库使用当前格式、可继续写入」条款的证据，由 `tests/legacy_compat_test.rs::restored_legacy_db_accepts_further_writes`（恢复出的库可继续写入：users 4 行、audit 计数 2）承载；「可再次导出且等价」仍由 `::legacy_dump_restores_into_current_db` 承载。

### 3. 本 change 的 T21 全量结论（交 Maintainer 记录到收尾）

M19 分批串行全量回归：**14 批**（`--lib --bins` 单独 1 批 + `tests/*.rs` 按文件名字典序 13 批，每批 8–12 个目标），目标清单 = 执行时 `ls tests/*.rs | sort` 的 **142** 个文件 + `--lib --bins` = **143** 个目标（对应 144 个测试二进制 / 144 条 `test result:` 行）。终态 **2231 passed / 0 failed / 2 ignored**（2 个 ignored 为 `cli_test.rs` 的长期 `#[ignore]` 用例）。覆盖对账：missing 0 / extra 0 / duplicate 0。

### 4. Iteration 006 登记的两条 Issue 候选（IC-1 / IC-2）的移交

| 候选 | 内容 | 承载建议（Iteration 006 的 Act Response 已写明） | 本 change 的处置 |
|---|---|---|---|
| **IC-1** | 叶页槽负载短于条目格式（38 字节）时不被页级判据判为损坏 ⇒ 该条目被静默跳过 | 另立 change 承载「叶页条目长度的页级判据」，按**读 / 写路径 = 明确错误**、**释放路径 = 计数 + 报出 + 跳过**两条相反处置方向分别设计 | 本 change **不承载**（T23 Forbidden + Iteration 006 的 Non-goals）；本 Iteration 的 IRBR 措辞收敛已把该行为**排除在**已验收主张之外 |
| **IC-2** | 存储层其余页型的无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处） | 按**页型**分组另立 change，每组各自判定「损坏页在该页型的正确行为」；夹具沿用 `slot_offset_field_offset` / `page_file_offset` / `corrupt_slot_offset` | 本 change **不承载**（Iteration 006 的 Non-goals + 验证预算） |

两条候选的**建立 / 关闭 / 重开**由 Recorder 按用户指令执行；本 change 只报告、不登记。

### 5. 交 Maintainer：既有域 `mvcc-tombstone-visibility` 的 AR2 字面读法澄清（措辞原文）

Iteration 005 的 Plan Review F3 裁定：AR2 以「条目数恰好 1 / 真正提交删除后同键可再插入 / 再插入后点查命中 1」三条为准 ⇒ 需在既有域的措辞上澄清，使 AR1（放弃窗口后该行仍可见）与 AR2 不再字面矛盾。按本 change 的执行依据**不修改 `openspec/specs/`**，故以「改动前 → 改动后」原文交接：

- **位置**：`openspec/specs/mvcc-tombstone-visibility/spec.md` 的 Requirement「未提交与已回滚墓碑不抑制、评估回溯前驱」正文。
- **改动前**：「删除者事务尚未提交（读取时仍在活跃集合，或 RC 视图下语句开始时仍活跃）时，该墓碑 SHALL NOT 抑制版本链，评估回溯产出删除前对该读者可见的最新版本。墓碑事务 ROLLBACK 后，该墓碑 SHALL 中性化（不再作为已提交删除抑制链），行为与该删除未发生一致（扫描路径）。」
- **改动后**：「删除者事务尚未提交（读取时仍在活跃集合，或 RC 视图下语句开始时仍活跃）时，该墓碑 SHALL NOT 抑制版本链，评估回溯产出删除前对该读者可见的最新版本。**该行仍可见 ⇒ 其键仍被占用（对该键的插入按既有唯一性语义被拒绝——该拒绝是键被占用的正常结果，不是残留条目造成的假阳性；此处「无残留条目」指该键的索引条目数为 1，即墓碑不为同一键留下第二条条目，而不是指该键可被再次插入）；只有该删除真正被提交之后，同键的插入才可再次成功。** 墓碑事务 ROLLBACK 后，该墓碑 SHALL 中性化（不再作为已提交删除抑制链），行为与该删除未发生一致（扫描路径）。」
- **形态**：只加**解释性括注**，既有 SHALL 逐条不变（不新增、不放宽任何行为要求）；与同域「回滚后墓碑行的索引条目还原」Requirement（回滚后同键插入以 `DuplicateKey` 拒绝）不冲突。

## 平衡审计

| Iteration | 聚合/拆分判定 | 结论 |
|---|---|---|
| 000 | 内聚结果 = 「索引页的归属唯一性与下降终止性」；同在索引/空闲页生命周期层（B-2 与 ISS23 共享「页号归属」这一根因链，且 ISS23 的首要候选通路就是 B-2 的下游后果）、同一定位边界（释放点与下降页号序列）、同一验证入口（新增 `tests/index_page_ownership_test.rs`）。T1/T2/T3 分属「建立先行测试 → 实施修复 → 复现与归属判定」，三者单独不构成稳定结果，合并为一个 Iteration 成立。 | 不拆分 |
| 001 | 内聚结果 = 「列改写的数据链一致性与放弃可见性」；落在 DDL 改写面 + 版本链活性层，与 Iteration 000 的索引层是不同故障域、不同定位边界（尾游标与链可达性 vs 释放点与下降序列）、不同验证入口（`tests/ddl_rewrite_consistency_test.rs`）。ISS22 与 ISS24 的候选机制 M1 指向**同一条**改写链分叉，必须在同一批次内判别与闭合。 | 不拆分 |
| 002 | 内聚结果 = 「释放路径的有界读」；单机制、单文件族（`slotted_page.rs` 读取面 + 释放路径扫描点）、独立可验收，与 Iteration 001 的改写面和 Iteration 004 的持久化判定是不同故障域。 | 不拆分 |
| 003 | 内聚结果 = 「改写窗口的读一致性」；修复面在**读路径**与在途状态的发布面，与 Iteration 001 的**写路径**（尾游标 / 链 / 墓碑）是不同故障域、不同定位边界（读路径检查点 vs 写路径链可达性）；且需要独立的故障注入夹具。依赖 Iteration 001（改写行为稳定后才好构造窗口夹具）。 | 不拆分 |
| 004 | 内聚结果 = 「释放可分配判定的耐久性前提」；落在刷盘/checkpoint 持久化层，与 Iteration 002 的读面、Iteration 003 的读一致性是不同故障域。 | 不拆分 |
| 005 | 内聚结果 = 「DML 墓碑的放弃窗口收敛（ISS24 的同形面）」；落在与 Iteration 001 **同一条根因链**（墓碑写序 + 放弃窗口）上的**另一条语句族**（DML 删除），修复面在 `src/executor/delete.rs` / `upsert.rs` 的写序，与 Iteration 002 的释放路径读面、Iteration 003 的读路径、Iteration 004 的持久化判定是不同故障域；既有域 `mvcc-tombstone-visibility` 的「未提交与已回滚墓碑不抑制」在 DML 路径上尚未闭合。 | 不拆分 |
| 006 | 内聚结果 = 「索引树读路径的有界读与显式拒绝」；落在**索引树访问器族**（`node.rs` 的 8 处 `get_slot_data`）与其调用点的错误语义面，与 Iteration 002 的释放路径读面、Iteration 003 的改写窗口读一致性是不同故障域、不同夹具形态（字节级页损坏 vs DDL 窗口注入）、不同验证入口；且**「正确行为」语义不同**——释放路径可以「告警 + 跳过」（物理释放尽力而为），读路径不可以（跳过子树 ⇒「查不到该行」⇒ 静默错读）⇒ 必须是显式错误。由 Iteration 002 的 Plan Review 2026-10-25 追加登记（Act 实测的同根因层读 / 写热路径面）。 | 不拆分 |
| 007 | 内聚结果 = 「旧格式兼容的证据补齐」；零产品代码改动，交付固定样本 + 常态化用例 + 记录更正 + 交接清单，与前七个 Iteration 的产品代码面完全解耦。 | 不拆分 |
| change 整体 | 八个 Iteration 覆盖同一批候选的不同根因层，共享**同一次**收尾全量回归预算与**同一份**交付边界（修缺陷 + 补证据 + 把已验收行为写入规格增量）；拆成多个 change 会把 M19 分批全量回归的执行次数按组数翻倍，而 C 组（零产品代码）单独吃一次全量预算收益极低，且 C 组的样本与派生量说明与断言耦合不能拆。用户指令「所有 issue 候选都直接寻找好的承载方式流程进行修复而不是等级」指向**一次缺陷闭环交付**。 | 一个 change、八个 Iteration |

**首个 Iteration 承载范围检查**：Iteration 000 只承载「索引页归属唯一性 + ISS23 归属判定」这一层成果，不因编号为 `000` 而承载整个 change（DDL 改写面在 001、释放读面在 002、改写窗口读面在 003、持久化判定在 004、DML 同形面在 005、索引树读路径在 006、兼容证据与收尾全量在 007）✓。

**行为回退面声明**：本 change 的行为变更面共 5 处，均为**收紧**或**显式化**，无放宽：

1. 改写在途的表被读时**显式拒绝**（此前返回按位置错位的值）——由静默错读改为明确错误（`design.md` D7）。
2. 释放的可分配判定**追加**「锚点写所在页已不脏」的前置条件（此前仅要求「锚点写已发出」）——收紧，不改变 MS35 已验收的判定时点（`design.md` D8）。
3. 释放路径在损坏页上**告警并跳过**而非 panic——保持既有尽力而为语义，仅消除进程 abort（`design.md` D6）。
4. 改写的 DELETE 腿**回写目录尾指针**（此前不写）——数据链既有目录字段的回写时机变化，不触及目录**列**定义面，`ddl-catalog-transactional` 不变（`design.md` D4）。
5. 损坏的索引树页在读 / 写路径上给出**明确的错误**（此前越界切片 panic ⇒ 进程 abort）——由进程崩溃改为明确错误；**不**降级为「跳过该子树」（那会变成静默「查不到该行」）（`design.md` D13）。

## Iteration Map

| Iteration | 标题 | Tasks | 依赖 | 状态 |
|---|---|---|---|---|
| 000 | 索引页归属唯一性——合并页只释放一次，下降必然终止 | T1, T2, T3 | None | accepted（Plan Review：Acceptance 1–5 全部满足；ISS23 归属结论 = B-2 的下游后果） |
| 001 | 列改写的数据链一致性与放弃可见性 | T4, T5, T6, T7 | 000 | accepted（Plan Review 2026-10-25：Acceptance 1–6 全部满足；ISS24 的缺失行钉在第 4 层（墓碑中和）+ 第 2 层（主键条目被删未还原），M2 为独立缺陷已修） |
| 002 | 释放路径的有界读 | T8, T9 | 001 | accepted（Plan Review 2026-10-25：Acceptance 1–6 全部满足；损坏被**计数并报出**、未被静默吞掉；未损坏路径的释放集合零变化） |
| 003 | 改写窗口的读一致性 | T10, T11 | 001 | accepted（Plan Review 2026-10-25：Acceptance 1–6 全部满足；9 处检查覆盖 13 个用户可见解码点、无漏检，改写自身读的豁免严格限于改写路径；拒绝的发布 / 清除 / 放弃 / panic 四条路径齐全且不过宽；T10 的 RED 经独立复核坐实为**真实错位值**。**ISS25 只闭合「并发读」变体，「崩溃-重开」变体仍开放** ⇒ ISS25 保持 `open`） |
| 004 | 释放可分配判定的耐久性前提 | T12, T13 | 002 | accepted（Plan Review 2026-10-25：Acceptance 1–4 全部满足；脏标记在三个刷盘入口均归还、提升闸门落在既有 `promote_free_releases` 内且无新 trait 方法 / 无持久状态 / 无磁盘格式改动；Deviation (a) 的取样点冲突经独立复核坐实并以「失败历史」为判据，(b) 的不可满足半句已就地改写为可达且已验收的观察——见 `specs/release-durability-precondition/spec.md`） |
| 005 | DML 墓碑的放弃窗口收敛（ISS24 的同形面） | T14, T15 | 001 | accepted（Plan Review 2026-10-25：Acceptance 1–6 全部满足；写序与 DDL 侧逐段同形、墓碑槽终态逐字节等价、REPLACE 同形闭合；**「未注册窗口未被夹具直接观测」经独立读码裁定为非阻塞 Minor**——该窗口内的墓碑不带链指针 ⇒ 不进入抑制集合 ⇒ 结构上无害，防护不依赖「该窗口不可放弃」这一不成立的论证） |
| 006 | 索引树读路径的有界读与显式拒绝 | T22, T23 | 002 | accepted（Plan Review 2026-10-25：Acceptance 1–6 全部满足；独立枚举确认索引树页访问点与条目提取点**无遗漏**（页级校验 22 处 = `btree.rs` 19 + `index_manager.rs` 3，条目提取面 8 个访问器全部有界读）；**「读路径绝不跳过」不变量逐分支成立**（22 处闸门均以 `?` 传播，`sibling_ids` 的 2 个调用点均传播 `Err`，唯一 `continue` 只容忍 `KeyNotFound`）；**健康页不误报**经独立推导坐实（`add_slot_inner` ⇒ `fso <= slot_area_start` 恒成立，槽回收后仍成立，半满叶页与满叶页均不触发守卫）；RED 形态确为 `slotted_page.rs:128` 的越界切片 panic；Act 追加的 7 处闸门裁定为**计划遗漏的补齐**而非超范围改动；登记 2 条范围外 Issue 候选（叶页条目长度判据缺口 / 其余页型无界读点）） |
| 007 | 旧格式兼容的证据补齐 | T16, T17, T18, T19, T20, T21 | 003, 004, 005, 006 | accepted（Plan Review 2026-10-25：Acceptance 1–6 全部满足；零产品代码改动经 `src/` mtime 独立核验；G2 判别为**结构化锁定**（目录行形态）且全链路**无 hash / revision / 版本比对**；C-2 三条常态化用例断言强度不低于原一次性记录（判别力经「故意改错期望」验证）；C-3 更正**彻底**且**已记录的退出码与结果逐条未动**；T20 ① 的落点裁定为**交接项**（依据：change `tasks.md` 的 T20 执行要点 + Plan Context 的 Preserve / Forbidden / Excluded scope 三处一致 + AGENTS.md 的文档地图；Plan Context 的 Targets 措辞与其自身冲突，归类为 **Plan 措辞误标**，非 Act 偏离），交接措辞经**逐字节核对可直接应用**；额外 2 条 G2 用例裁定为 **Acceptance 2 的覆盖补齐**（dump / 恢复在计划名集合中本无专属断言）；**T21 的 M19 分批串行 12 条纪律逐条成立**（14 批 / 143 目标 / 每批 10–11 个目标 / 全局字典序 / 覆盖对账 missing 0 / extra 0 / duplicate 0 / 全批 0 failed / **2231 passed / 0 failed / 2 ignored**，差额 **+39 = 32 + 7** 经独立复算吻合）；9 份规格增量均可合并且彼此与既有 75 域不冲突；登记 4 条 Issue 候选（IC-1 / IC-2 沿用 Iteration 006，IC-3 非缺陷，IC-4 为 MS35 的归档格式障碍）） |

**Next Iteration**: Iteration 005 已 **accepted**（Plan Review 2026-10-25，`iterations/005-dml-tombstone-abandon-window/000-initial.md`；承载 T14–T15：DML 墓碑的放弃窗口收敛——写序与 DDL 改写路径逐段同形「不带链指针写 → 登记 → 补链指针 → 再清理索引条目」，墓碑槽终态与修复前逐字节等价，REPLACE 冲突行删除段同形闭合；「未注册窗口未被夹具直接观测」经独立读码裁定为**非阻塞 Minor**：该窗口内的墓碑不带链指针 ⇒ 不进入以 `next_version` 构造的抑制集合 ⇒ 结构上无害，防护**不依赖**「该窗口不可放弃」这一不成立的论证）。Iteration 006 已 **accepted**（Plan Review 2026-10-25，`iterations/006-index-read-bounded-read/000-initial.md`；承载 T22–T23：索引树读路径的有界读与显式拒绝；新增规格增量域 `index-read-bounded-read`；Acceptance 1–6 全部满足；独立复核确认索引树页访问点与条目提取点**无遗漏**（页级校验 22 处 = `btree.rs` 19 + `index_manager.rs` 3；条目提取面 8 个访问器全部有界读）、**「读路径绝不跳过」不变量逐分支成立**、**健康页不误报**经独立推导坐实；Act 追加的 7 处闸门裁定为**计划遗漏的补齐**而非超范围改动；登记 2 条范围外 Issue 候选）。Iteration 007 已 **accepted**（Plan Review 2026-10-25，`iterations/007-legacy-compat-evidence/000-initial.md`；承载 T16–T21：旧格式兼容的证据补齐（C-1 G2 文件级样本与断言 / C-2 三条一次性条款常态化 / C-3 迁移路径记录更正 / T20 的三条措辞对齐 + 交接清单）+ **本 change 唯一一次收尾全量回归 T21**，M19 分批串行 12 条纪律逐条写死在该 Cycle 的 T21；Acceptance 1–6 全部满足；零产品代码改动；G2 判别为结构化锁定且无身份机制；T20 ① 的落点裁定为**交接项**（Plan Context 的 Targets 与其 Preserve / Forbidden / Excluded scope 冲突，归类为 Plan 措辞误标）；额外 2 条 G2 用例裁定为覆盖补齐；T21 的 14 批 / 143 目标 / 2231 passed / 0 failed / 2 ignored 与覆盖对账经独立复算吻合）。**本 change 的八个 Iteration（000–007）全部 `accepted` ⇒ `Next Iteration: None`。****Iteration 005（DML 墓碑的放弃窗口收敛）由 Iteration 001 的 Plan Review 2026-10-25 登记承载**——Act 实测的 `src/executor/delete.rs` 同形放弃窗口（被放弃的 `DELETE` 使其行不可见）不在 Iteration 001/002 的范围内（Iteration 001 的 Included scope 是 DDL 改写路径；Iteration 002 是释放路径的读面），故新开一个 Iteration 承载它，并置于收尾全量之前。**Iteration 006（索引树读路径的有界读与显式拒绝）由 Iteration 002 的 Plan Review 2026-10-25 登记承载**——Act 实测的 `src/storage/btree/node.rs` 同根因层无界读点（损坏的索引页在普通 `SELECT` / `INSERT` 路径上仍越界切片 panic）不在 Iteration 002 的范围内（Iteration 002 是**释放路径**的读面），且其「正确行为」语义与释放路径不同（读路径不得跳过子树 ⇒ 静默「查不到」），故另开一个 Iteration 承载它，同样置于收尾全量之前；原 Iteration 006（旧格式兼容证据补齐）顺延为 **007** 并继续承担收尾全量。**Iteration 003 的 Plan Review 2026-10-25 另行登记了一条范围外 Issue 候选**：「提交完成 → 目录应用」窗口内的崩溃-重开 / 提交后放弃仍按位置错位（关闭它需持久标记 ⇒ 改磁盘格式并破坏 MS36 的 G1/G2/G3 兼容面，或移动提交点 ⇒ 与 ISS21 / D5 冲突）⇒ **ISS25 保持 `open`**，不随 Iteration 003 关闭。**Iteration 004 的 Plan Review 2026-10-25 登记了一条范围外 Issue 候选**：「失败的 DDL 语句其目录面变更在后续刷盘落地」（修复后可达已实测坐实，与既有域 `ddl-catalog-transactional` 的「失败的 DDL 语句不留目录面变更」存在字面张力）⇒ 由 Recorder 择承载。

## 最终状态（Iteration 007 / Cycle 000 的 Plan Review 2026-10-25 写入）

- **Review Result**：`accepted`（Iteration 007 / Cycle 000）；八个 Iteration（000–007）**全部 accepted** ⇒ **`Next Iteration: None`**。
- **任务**：T1–T23 **全部 done**（T21 = 14 批 / 143 目标 / **2231 passed / 0 failed / 2 ignored**）。
- **产品代码**：本 change 的行为变更面共 5 处（见上文「行为回退面声明」，均为**收紧**或**显式化**）；Iteration 007 **零产品代码改动**。
- **规格增量**：5 个新增域，全部 `## ADDED Requirements`，全部**待 Maintainer 合并**（`openspec/specs/` 现为 75 域 ⇒ 合并后 80 域）：`index-page-ownership`（3/6）、`ddl-rewrite-consistency`（3/10）、`release-path-bounded-read`（1/4）、`release-durability-precondition`（2/7）、`index-read-bounded-read`（1/5）。
- **本 Review 未执行、留给 Maintainer / Recorder 的收尾事项**（逐条见 `iterations/007-legacy-compat-evidence/000-initial.md` 的「收尾交接说明」）：
  1. 合并本 change 的 5 份增量进 `openspec/specs/`，**同批应用**「收尾交接清单」第 5 条的 MTV AR2 澄清（既有域 `mvcc-tombstone-visibility`）。
  2. 合并 **MS36** 的 2 份增量，**同批应用**「收尾交接清单」第 2 条的 `legacy-format-compat` 证据索引更新（E2 / E4 逐字替换；E7 **整条替换**）。
  3. 合并 **MS35** 的 3 份增量——**前置**：先把 `specs/free-page-lifecycle/spec.md` 的段头改为 `## ADDED Requirements`（该域在 `openspec/specs/` 下不存在 ⇒ 语义为新增）；否则 `openspec validate --changes` 失败、`openspec archive` 无法合并。
  4. 同步全局状态：`.agents/docs/SNAPSHOT.md`、`.agents/docs/tasks.md`、M / R / I 登记。
  5. 归档三个 change（本 change / MS35 / MS36，均已 `accepted`）。
  6. Issue 台账（`.agents/issues/`，只读核对；登记 / 关闭由 Recorder 按用户指令执行）：**ISS22 / ISS23 / ISS24 可关闭**（本 change 已修并验收）；**ISS25 保持 `open`**（Iteration 003 只闭合「并发读」变体，「崩溃-重开」变体仍开放）。
