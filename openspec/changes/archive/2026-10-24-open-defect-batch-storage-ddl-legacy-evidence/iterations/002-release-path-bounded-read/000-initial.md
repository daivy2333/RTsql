# Iteration 002 / Cycle 000: 释放路径的有界读

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T8, T9
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 既有域 `free-page-lifecycle`（MS35 已验收：释放完成编号与取样时点语义）、既有域 `drop-persistence-boundary`（`DROP TABLE` 的崩溃边界模型）、`design.md` D6（B-4 的修复形态与边界）、`design.md` D12（收尾全量的预算与纪律）、`specs/release-path-bounded-read/spec.md`（RPBR R1 的 Requirement 与 4 个 Scenario）、ISS23 的规模约束（本 change 内不使用 ≥ 300 行的表规模构造用例）、`design.md` D9（B-3 不承载）。
- Excluded scope: `SlottedPageRef::get_slot_data` 的**签名**改造（波及全部调用点，属范围外候选）；释放路径的**返回语义**变更（既有尽力而为：告警 + 跳过 + 返回）；`check_data_page` / `check_tree_page` 的判据本身（既有 integrity 模块的读法）；完整性扫描的诊断分级（既有域 `data-integrity-verification`）；`FileStorage` 的释放集合语义（`free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` / `pending_frees` / `free_pages`）；磁盘格式；Iteration 001 的 DDL 改写面、Iteration 003 的读路径、Iteration 004 的持久化判定；**Act 实测的 `src/executor/delete.rs` DML 墓碑放弃窗口**（ISS24 的同形面）——它不属于本 Iteration 的根因层（释放路径的**读面**），由 **Iteration 005**（T14–T15）承载，判定理由见 Iteration 001 的 `Plan Review` 的 Issue 候选段与 `tasks.md` 的 Iteration 005 段落。

**Objective**

`DROP TABLE` / `TRUNCATE` / `DROP INDEX` 在**页内容损坏**（槽位偏移或长度越界）时不再 panic、不再导致进程 abort；释放路径的槽位扫描经由**有边界检查**的读取路径（与 `collect_data_chain_strict` 同一来源的 integrity 模块读法），损坏页按释放路径既有的尽力而为语义处理（告警并跳过该页/该链，返回已收集的部分）；未损坏路径的释放行为**零变化**（释放的页集合不遗漏、不重复，既有释放断言保持通过）。

**Background**

B-4 是 MS34 / MS35 / MS36 修复批遗留的开放候选之一（承载于本 change，design D6）。`design.md` D6 已把修复形态与边界定死：释放路径换读法，**不改** `get_slot_data` 的签名，**不**把释放路径改成「遇损坏页整体失败」。本 Cycle 是该决策的首次实施。

**Investigation Facts**

- Starting point: HEAD `0a08b14`（ahead of origin 9）。工作区有**既有未提交改动**（MS34 取消/超时批、MS34 issue-fix 批〔ISS20/ISS21〕、MS35 空闲页批、MS36 兼容批、Iteration 000 与 Iteration 001 的改动及其新增测试文件），属**既有现场**：不得回滚、不得清理、不得提交；本 Cycle 的改动叠加在其上。
  - Iteration 001 的 `Plan Review` Result = **`accepted`**（2026-10-25），其定向验证结论（`ddl_rewrite_consistency_test` 6/0、`cargo test --lib` 420/0）覆盖范围与本 Cycle 不重叠（本 Cycle 不改 `table_manager.rs` 的改写面、`create_index.rs`、`recovery.rs`、`transaction/manager.rs`，也不改 `tests/ddl_rewrite_consistency_test.rs`）⇒ 可沿用。
  - MS35 / MS36 收尾的全量终态 **2192 passed / 0 failed / 2 ignored**（来源 `.agents/runbooks/m19-batched-full-regression.md` R48）覆盖范围已变（Iteration 000 与 001 之后新增了多个测试文件与产品代码改动）⇒ **不得**再作为「当前工作区全量通过」的依据；本 change 的收尾全量在 **Iteration 006**（T21）。
  - 本 Cycle **不跑全量回归**（收尾全量在本 change 内只跑一次，归 Iteration 006）；但**必须**重跑 `cargo test --lib`（本 Cycle 改 `src/storage/data/table_manager.rs` 等 ⇒ Iteration 001 的 420 条 lib 结论对本次改动失效）。
- Current-State Evidence（Plan 已逐行确认，行号为**当前工作区**）:
  - **无边界检查的读取器**：`src/storage/page_format/slotted_page.rs:125-131` `SlottedPageRef::get_slot_data(&self, slot)`：`let start = slot.offset as usize; let end = start + slot.length as usize; &self.data[start..end]`——`offset` / `length` 直接来自页内的槽目录（`slotted_page.rs:96-110`：`slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE`，槽目录 6 字节 = `logical_id(2) | offset(2) | length(2)`，`Slot::SIZE = 6`），**没有任何范围检查** ⇒ 槽偏移或长度越界 ⇒ 越界切片 panic（panic 发生在异步执行路径 ⇒ 进程 abort）。
  - **释放路径上的三处无界读点**：
    1. `src/storage/data/table_manager.rs:1588` `collect_overflow_chain_pages` → `:1600` `slotted.get_slot_data(&slot)`（逐槽取数据以读溢出定位符）。调用者：`drop_table`（`:1529`）、`truncate_table`（`:1908`）。
    2. `src/storage/btree/index_manager.rs:615` `collect_all_pages` → `:645-650` `internal.get_child_page_id(i)` → `src/storage/btree/node.rs:1049-1061`（`:1051` `get_slot_data`）读内部结点的子指针。调用者：`drop_table`（`:1489` PK / `:1500` 唯一 / `:1513` 二级）、`truncate_table`（`:2019`）、`DROP INDEX`（`src/executor/drop_index.rs:60`）。
    3. `src/storage/btree/node.rs:1039-1046` `InternalNodeRef::get_key` → `:1041` `get_slot_data`（同一遍历路径上的第二个无界读点）。
  - **同仓库已有的有界读（同一来源，修复形态的模板）**：`src/storage/integrity.rs:489` `check_data_page(data)` → `DataPageCheck { violations, locators, .. }`（`locators` 即溢出定位符，`integrity.rs:480/514`）；`src/storage/integrity.rs:580` `check_tree_page(data)` → `TreePageCheck { violations, children, .. }`（`children` 即子指针，`integrity.rs:570/599/617`）。两者均**已在生产路径**使用：`table_manager.rs:1778` `collect_data_chain_strict`（数据页）与 `table_manager.rs:1827` `collect_tree_pages`（索引树页）——二者的文档注释明确写着「生产访问器无边界地索引进页、在损坏页上会 panic，而重建每次打开都运行、绝不能因损坏页崩溃」。
  - **释放路径的既有返回语义 = 尽力而为**：`drop_table` 的索引页收集失败走 `eprintln!` 告警并跳过该树（`table_manager.rs:1489-1520`）；`drop_index.rs:60-77` 同形态（`collect_all_pages` 失败 ⇒ 告警 + 放弃该树的物理释放）；`collect_overflow_chain_pages` 的文档注释（`table_manager.rs:1585-1587`）写明与 `collect_data_pages` 同一策略。
  - **打开期对损坏页的容忍**：`src/database.rs:436` 把 `rebuild_free_page_set` 的错误吞掉并告警（`[free-page-rebuild] rebuild abandoned`）⇒ 损坏的数据页**不会**让打开失败 ⇒ 损坏夹具可以「写库 → 关闭 → 改坏页字节 → 重开」后直接执行 DDL。
  - **既有损坏注入先例**：`tests/check_command_test.rs:224-255` 的 `test_check_corrupted_page_exit_1_reports_page_and_semantics` 用 `poke(&path, page_file_offset(head, 0), 0x42)` 按**文件偏移**改写页字节；本 Cycle 的夹具沿用同一手法，但损坏点改为**槽目录项的 offset/length 字段**（页尾 `PAGE_SIZE - (i+1)*6` 起的 6 字节中的第 3–6 字节），以命中 `get_slot_data` 的越界切片。
  - **规格增量的读集澄清**：`specs/release-path-bounded-read/spec.md` 的 Scenario「损坏页上的删索引不 panic」的 GIVEN 写的是「待删二级索引所依赖的**数据页**槽位偏移越界」——但 `DROP INDEX` 的释放路径**只读索引树页与目录**（`drop_index.rs:44-77`），不读数据页 ⇒ 该场景的损坏点必须落在**该语句实际扫描的页**上（`DROP INDEX` ⇒ 索引树页）。此为规格文本与实际读集的措辞偏差，**不改变行为要求**（不 panic），由 **T20**（规格增量与验收边界对齐，Iteration 006）在收敛时澄清。
  - **规模约束**：ISS23 约束本 change 内不使用 ≥ 300 行的表规模构造用例；B-4 的判定不依赖规模（单页 + 数行即可命中越界切片）⇒ 用例表规模 **≪ 300**。
  - **既有测试入口**：`tests/drop_table_free_test.rs`、`tests/drop_persistence_test.rs`、`tests/free_page_lifecycle_test.rs`（释放语义回归）；`tests/check_command_test.rs`（损坏注入手法）。**本 Cycle 新增** `tests/release_path_bounded_read_test.rs`。
- Code and Critical Path:
  - 调用链（释放路径）：`DropTableExecutor` / `TruncateExecutor` / `DropIndexExecutor` → `TableManager::drop_table`（`table_manager.rs:1466`）/ `truncate_table`（`:1894`）/ `IndexManager::collect_all_pages`（`index_manager.rs:615`，`DROP INDEX` 由 `drop_index.rs:60` 调用）→ 无界槽读 → 越界切片 panic。
  - 数据流：页字节 → `SlottedPageRef::get_slot(index)`（有界：槽目录在页内固定位置）→ `get_slot_data(&slot)`（**无界**）→ 槽负载（数据页：元组 + 溢出定位符；树页：分隔键 + 子指针）。
  - 状态变化：`DROP TABLE` / `TRUNCATE` 的物理释放是「收集页集合 → `free_page` → `complete_free_release`」；`DROP INDEX` 是「`take_secondary_index`（目录摘除，锚点写）→ 物理释放旧树页」。
  - 错误路径：`collect_all_pages` 返回 `Err` 时被释放路径降级为告警 + 跳过；本 Cycle 的目标是让**损坏页**走同一条降级路径，而不是新增一种错误形态。
  - 外部影响：当前形态下损坏页上的 `DROP TABLE` / `TRUNCATE` / `DROP INDEX` **进程 abort**（越界切片 panic）；修复后语句按既有尽力而为语义结束。

**Implementation Guidance**

- 建议顺序：先 T8（RED 用例，三条损坏页场景 + 一条未损坏对照），再 T9（换读法），最后跑回归。
- 换读法的两个来源都要用：`check_data_page` 覆盖**数据页**的槽位扫描（`collect_overflow_chain_pages`），`check_tree_page` 覆盖**索引树页**的遍历（`IndexManager::collect_all_pages` 与 `DROP INDEX` 的释放路径）。
- 保持尽力而为语义：损坏页 ⇒ 告警（沿用既有的 `eprintln!` 形态）+ 跳过该页/该链 + 返回已收集的部分；**不**把释放路径改成整体失败，**不**新增错误类型。
- 内部结点页的遍历可直接复用 `check_tree_page` 的 `children`（`integrity.rs:570/599/617`），不另写一份解析逻辑；`IndexManager::collect_all_pages` 的对外签名与返回语义（`Result<Vec<PageId>>`，出错时由调用方降级告警）保持不变。
- 损坏注入沿用 `tests/check_command_test.rs` 的 `poke` 手法（按文件偏移改写页字节），损坏点选在**槽目录项的 offset 字段**（页尾槽目录、6 字节项的第 3–4 字节）⇒ `get_slot_data` 必然越界。
- 夹具的最小形态：建表 + 插入数行 → 关闭 → 改坏数据页（或索引树页）的一个槽偏移 → 重开 → 执行对应 DDL ⇒ 断言「语句返回（成功或既有错误形态）且进程未 abort」。**重开**这一步依赖 `database.rs:436` 的「重建失败只告警」行为。

**Task Definitions**

### T8: 损坏页上的释放路径不 panic

- Requirement/Scenario: `specs/release-path-bounded-read/spec.md` RPBR R1 的三个损坏页 Scenario（删表 / 截断 / 删索引）+「未损坏的释放路径行为不变」Scenario；`design.md` D6。
- Depends on: None
- Targets: `tests/release_path_bounded_read_test.rs`（新增）
- Current behavior: 页内容损坏（槽位偏移或长度越界）时，`DROP TABLE` / `TRUNCATE` / `DROP INDEX` 在释放路径的槽位扫描中越界切片 panic ⇒ 进程 abort。
- Required behavior: 三条语句在损坏页上**不 panic、不 abort**，按释放路径既有的尽力而为语义结束（告警并跳过不可读的部分，返回成功或明确的错误）；未损坏库上的释放行为与修复前一致（释放的页集合不遗漏、不重复）。
- Preserve: 释放路径的返回语义（成功 / 既有错误形态）；`free_page` / `complete_free_release` 的集合语义；未损坏路径的页集合与既有释放断言。
- Forbidden: 不得用「跳过读」之类的替身代替真实损坏（夹具必须真实改坏页字节）；不得改动 `SlottedPageRef::get_slot_data` 的签名；不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、判定层或 timer 阈值行为判据；不得在测试框架外建判定脚本或封装命令。
- First test: 在 `tests/release_path_bounded_read_test.rs` 中新增三条损坏页用例（`DROP TABLE` / `TRUNCATE` / `DROP INDEX` 各一）与一条未损坏对照用例；损坏夹具 = 真实文件库 + 真实 `Database::open` / DDL → 关闭 → 按文件偏移改写数据页（或索引树页）槽目录项的 offset 字段 → 重开 → 执行 DDL。变更前运行 `cargo test --test release_path_bounded_read_test -- --test-threads=1`，预期 **RED**：损坏页用例以 panic 失败（进程 abort 由测试进程捕获为失败），未损坏对照用例通过。
- GREEN condition: 修复后同一条命令 4 个用例全部 passed / 0 failed。
- Verification: `cargo test --test release_path_bounded_read_test -- --test-threads=1` ⇒ 4 passed / 0 failed；失败即表示损坏页路径仍会 panic 或释放语义被改动。
- Stop when: 修复需要改动 `SlottedPageRef::get_slot_data` 的签名或波及全部调用点 ⇒ 回到 Plan；或需要把释放路径改成「遇损坏页整体失败」⇒ 回到 Plan（既有尽力而为语义不可变）；或损坏夹具无法在真实文件库上稳定命中越界切片 ⇒ 回到 Plan（夹具形态是判据的一部分）。

### T9: 释放路径的槽位扫描改用有界读

- Requirement/Scenario: 同 T8（实现 T8 的 RED）。
- Depends on: T8
- Targets: `src/storage/data/table_manager.rs::collect_overflow_chain_pages`（`:1588`）、`src/storage/btree/index_manager.rs::collect_all_pages`（`:615`）、`src/storage/btree/node.rs`（`InternalNodeRef` 的无界读点 `:1041` / `:1051`，按需要改为经由有界读或改由 `check_tree_page` 提供子指针）
- Current behavior: 释放路径用 `SlottedPageRef::get_slot_data` 无边界地切页内字节；损坏页 ⇒ panic。
- Required behavior: 释放路径的页内容读取经由 **integrity 模块的有界读**（`check_data_page` / `check_tree_page`，与 `collect_data_chain_strict` / `collect_tree_pages` 同一来源）；损坏页按尽力而为语义处理（告警 + 跳过该页/该链 + 返回已收集的部分）；未损坏路径的释放页集合与修复前**逐页相同**。
- Preserve: `IndexManager::collect_all_pages` 的对外签名与 `Result` 语义（调用方的降级告警路径不变）；`drop_table` / `truncate_table` / `DROP INDEX` 的成功 / 失败语义；`free_page` / `complete_free_release` 的集合语义与取样时点。
- Forbidden: 不得改 `SlottedPageRef::get_slot_data` 的签名或把它 `Option` 化（波及全部调用点，属范围外候选）；不得改 integrity 模块的判据本身（`check_data_page` / `check_tree_page` 的 violations 定义）；不得改磁盘格式；不得改 `FileStorage` 的释放集合语义；不得把释放路径改成「遇损坏页整体失败」；不得引入身份证明机制、判定层或 timer 阈值行为判据。
- First test: 沿用 T8 的用例（先 RED 后 GREEN）。若 T8 的损坏夹具在某条语句上未能命中（该语句不扫描被损坏的页），须**更换该用例的损坏点**（使损坏落在该语句实际扫描的页上），而不是放宽断言。
- GREEN condition: `cargo test --test release_path_bounded_read_test -- --test-threads=1` 全部通过，且既有释放相关用例保持通过。
- Verification: `cargo test --test release_path_bounded_read_test -- --test-threads=1`（4/0）；`cargo test --test drop_table_free_test -- --test-threads=1`；`cargo test --test drop_persistence_test -- --test-threads=1`；`cargo test --test free_page_lifecycle_test -- --test-threads=1`；`cargo test --lib -- --test-threads=1`（本 Cycle 改了 `table_manager.rs` 等 ⇒ Iteration 001 的 420 条 lib 结论失效，必须重跑）。任一命令 failed > 0 ⇒ 本 Cycle 未完成。
- Stop when: 修复需要改 `get_slot_data` 的签名、改 integrity 模块的判据、或把释放路径改成整体失败 ⇒ 回到 Plan。

**Invariants**

- `DROP TABLE` / `TRUNCATE` / `DROP INDEX` 在**未损坏**库上的可观察行为**零变化**：释放的页集合、目录变更、返回结果与既有断言一致。
- 释放路径的尽力而为契约不变：读不出 ⇒ 告警 + 跳过 + 返回已收集的部分，**不**新增错误类型、**不**把语句改成整体失败。
- MS35 已验收的 `free-page-lifecycle` 判定时点语义（锚点写发出 ⇒ 释放完成标记）不变；`free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` 的集合语义不变。
- 磁盘格式不变（`VersionHeader` 布局、目录行布局）；MS36 已验收的 G1/G2/G3 兼容面不变。
- 既有域 `drop-persistence-boundary` 的崩溃边界模型不变（`DROP TABLE` 成功返回之后、任何后续 DDL 成功返回之前）。
- 不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、判定层或 timer 阈值行为判据；判定一律在 `cargo test` 内。

**Non-goals**

- `SlottedPageRef::get_slot_data` 的签名改造（波及全部调用点，含热路径与恢复面；属范围外候选）。
- 释放路径的返回语义变更（既有尽力而为）。
- 完整性扫描的诊断分级（既有域 `data-integrity-verification`）。
- B-3（已被已验收规格显式界定为范围外既有行为，见 `design.md` D9）。
- ISS23 的规模面（本 change 内不使用 ≥ 300 行的表规模）。
- DML 墓碑的放弃窗口（Iteration 005 承载）、改写窗口的读一致性（Iteration 003）、释放可分配判定的耐久性前提（Iteration 004）。

**Acceptance**

1. **损坏页上的 `DROP TABLE` 不 panic**：真实文件库 + 真实 `Database::open` + 真实 `DROP TABLE`；损坏点 = 数据页槽目录项的 offset 字段越界；语句 SHALL NOT panic、SHALL NOT 导致进程 abort，且按尽力而为语义结束（告警 + 跳过不可读部分 + 返回成功或既有错误形态）。映射：RPBR R1 Scenario「损坏页上的删表不 panic」、T8、T9。
2. **损坏页上的 `TRUNCATE` 不 panic**：同上形态，损坏点 = 待截断表的数据页槽目录项。映射：RPBR R1 Scenario「损坏页上的截断不 panic」、T8、T9。
3. **损坏页上的 `DROP INDEX` 不 panic**：损坏点 = 该二级索引的**索引树页**槽目录项（该语句实际扫描的页，见 Investigation Facts 的读集澄清）。映射：RPBR R1 Scenario「损坏页上的删索引不 panic」、T8、T9。
4. **未损坏路径行为不变**：内容完好的库执行 `DROP TABLE` / `TRUNCATE` / `DROP INDEX` ⇒ 释放的页集合与既有行为一致（不遗漏、不重复），既有释放断言保持通过。映射：RPBR R1 Scenario「未损坏的释放路径行为不变」、T9 + `tests/drop_table_free_test.rs` / `tests/drop_persistence_test.rs` / `tests/free_page_lifecycle_test.rs`。
5. **判别力**：三条损坏页用例的 RED 必须是**越界切片 panic**（而非任何其它失败形态），且未损坏对照用例在 RED 阶段即通过；损坏夹具必须真实改坏页字节（不得用替身）。映射：T8 的 First test。
6. **既有回归零破坏**：`cargo test --lib` 与三条既有释放用例全部 0 failed；本 Cycle 不跑全量（全量归 Iteration 006）。

**Verification**

- 行为类 scenario（1–4）走**真实入口**：真实文件库 + 真实 `Database::open` + 真实 DDL 语句；损坏通过**按文件偏移改写页字节**注入（真实损坏，不是替身）。
- 判定命令：`cargo test --test release_path_bounded_read_test -- --test-threads=1` ⇒ 4 passed / 0 failed（场景 1–5）。
- 回归命令：`cargo test --test drop_table_free_test -- --test-threads=1`、`cargo test --test drop_persistence_test -- --test-threads=1`、`cargo test --test free_page_lifecycle_test -- --test-threads=1`、`cargo test --lib -- --test-threads=1` ⇒ 各自 0 failed（场景 4、6）。
- 全部命令以 `--test-threads=1` 串行执行；判定只看 cargo 自己的退出码与原生输出（允许用一步通用管道读计数，管道只做计数不做判定）。
- 不跑全量回归（本 change 一次预算，归 Iteration 006 的 T21）。
- 无人工验证项；无身份证明机制；无框架外判定层。

**Gate 2 Readiness**

- 需求与场景：RPBR R1 的四个 Scenario 已逐条映射到 T8/T9 与 Acceptance 1–4 ✓（BLOCKED: 无）。
- 实现调查：三个无界读点、两个有界读来源、释放路径的尽力而为语义、打开期对损坏页的容忍、损坏注入先例、规格增量与实际读集的措辞偏差，均已在 Investigation Facts 中落到**当前工作区的行号** ✓（BLOCKED: 无）。
- 任务分轮：T8（RED）→ T9（GREEN）→ 回归，两轮可独立验证 ✓（BLOCKED: 无）。
- 追踪：tasks.md 的 RTM 行「RPBR R1 释放路径读取页内容时有边界检查 | T8, T9 | `tests/release_path_bounded_read_test.rs::drop_table_on_damaged_page_does_not_panic`」与本 Cycle 一致 ✓（BLOCKED: 无）。
- 当前轮验证：Verification 列出可直接运行的命令与通过条件；无人工项 ✓（BLOCKED: 无）。
- 结论：**Gate 2 通过**——需求、调查、任务分轮、追踪与验证均达执行就绪；Plan Context 状态置 `ready`。

**Persisted Evidence**

- Mode: none

判别读数固化在测试代码中（随用例重跑）；RED / GREEN 的实际输出与命令写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录。

**Risks and Notes**

- **规格文本与实际读集的措辞偏差**：`specs/release-path-bounded-read/spec.md` 的「损坏页上的删索引不 panic」场景的 GIVEN 指向「数据页」，而 `DROP INDEX` 的释放路径只读索引树页 ⇒ 本 Cycle 的夹具按**该语句实际扫描的页**注入损坏（行为要求「不 panic」不变），文本澄清由 T20（Iteration 006）承担。
- **`DROP TABLE` 同时扫描数据页与索引树页**：两条无界读点都在其释放路径上 ⇒ 该语句的损坏夹具只损坏数据页即可命中；若 Act 发现 `drop_table` 在损坏数据页上先于索引树崩溃，则用例仍以「未 panic」为判据 ✓。
- **`collect_all_pages` 的调用面广**：`drop_table` / `truncate_table` / `drop_index` 三处调用 ⇒ 改读法会同时改变三条路径的行为面；回归必须覆盖三条既有释放用例。
- **本 change 内的规模约束**：B-4 不依赖规模，用例表规模保持 ≪ 300 ✓。
- **Act 实测的 DML 同形候选**（`src/executor/delete.rs` 的墓碑放弃窗口）不在本 Cycle 的范围与根因层内 ⇒ 已由 Iteration 005 承载（T14–T15），本 Cycle **不得**顺手修（那是另一个根因层、另一个执行依据）。

## Act Response

- Status: reported

### 任务状态

| Task | 状态 | 交付 |
|---|---|---|
| T8 损坏页上的释放路径不 panic | completed | `tests/release_path_bounded_read_test.rs`（新增）：3 条损坏页用例 + 1 条未损坏对照用例 + 3 条真实 CLI 入口用例（共 7 个） |
| T9 释放路径的槽位扫描改用有界读 | completed | `src/storage/data/table_manager.rs::collect_overflow_chain_pages`、`src/storage/btree/index_manager.rs::collect_all_pages` / `collect_all_pages_tolerant`、`src/storage/integrity.rs::violation_summary`（新增） |

### 实际改动（文件与符号）

1. **`src/storage/integrity.rs`** — 新增 `pub(crate) fn violation_summary(violations: &[String]) -> String`：损坏页告警的**计数**措辞（`1 violation` / `N violations: <明细>`）。`check_data_page` / `check_tree_page` 的判据本身（violations 定义、locators/children 的取法）**零改动**——只新增一个消息格式化助手。
2. **`src/storage/data/table_manager.rs::collect_overflow_chain_pages`**（`:1588`）— 槽位扫描由 `SlottedPageRef::get_slot_data`（无边界切片）改为 integrity 模块的 `check_data_page`：
   - 健康页语义逐项等价（旧：`get_slot_data` → `decode_locator(slot_data[VersionHeader::SIZE..])`；新：`check_data_page` 对同一段字节 `decode_locator(data[start+VersionHeader::SIZE..end])`，locator 顺序相同、`first == 0` 同样跳过）；
   - 页有 violations ⇒ **计数并报出**（`[overflow-release] data page N is damaged: 1 violation: …`）+ 跳过该页的 locator ⇒ 该页引用的溢出链**不被释放**；已收集的部分照常返回。
3. **`src/storage/btree/index_manager.rs::collect_all_pages`**（`:615`）— 子指针遍历由 `InternalNodeRef::get_child_page_id`（无边界切片）改为 `check_tree_page` 的 `children`：
   - `InvalidPageType` 的 `Err` 形态与对外签名（`Result<Vec<PageId>>`）不变 ⇒ 调用方的降级告警路径不变；
   - 页有 violations ⇒ **计数并报出**（`[index-release] index tree page N is damaged: 1 violation: …`）+ 跳过该页的子树；已收集的页照常返回。
4. **`src/storage/btree/index_manager.rs::collect_all_pages_tolerant`**（`:679`）— 同一根因层的第二个释放路径（`truncate_table` 失败路径与恢复路径的旧树释放）里的无界读点一并改为 `check_tree_page`；洞页（invalid type）与损坏页（violations）各自的既有降级语义不变（跳过子树 + 告警）。
5. **`tests/release_path_bounded_read_test.rs`**（新增）— 夹具与断言见下。

### 先行测试：RED 证据（实际运行）

损坏夹具（真实文件库 + 真实 `Database::open` / DDL → 关闭 → 按文件偏移把页尾槽目录项第 0 项的 offset 字段改写为 `0xFFFF` → 重开 → 执行 DDL）：

```
$ cargo test --test release_path_bounded_read_test drop_table_on_damaged_data_page_does_not_panic
running 1 test
[free-page-rebuild] rebuild abandoned: internal error: data page 2 of the persisted directory is damaged: slot 0 data range [65535, 65566) overlaps the page header or the slot area
thread 'drop_table_on_damaged_data_page_does_not_panic' panicked at src/storage/page_format/slotted_page.rs:128:19:
range start index 65535 out of range for slice of length 4096
```

```
$ cargo test --test release_path_bounded_read_test truncate_on_damaged_data_page_does_not_panic
thread 'truncate_on_damaged_data_page_does_not_panic' panicked at src/storage/page_format/slotted_page.rs:128:19:
range start index 65535 out of range for slice of length 4096

$ cargo test --test release_path_bounded_read_test drop_index_on_damaged_index_page_does_not_panic
thread 'drop_index_on_damaged_index_page_does_not_panic' panicked at src/storage/page_format/slotted_page.rs:128:19:
range start index 65535 out of range for slice of length 4096
```

- 实际 panic 点：`src/storage/page_format/slotted_page.rs:128`（`SlottedPageRef::get_slot_data` 的 `&self.data[start..end]`）⇒ **越界切片 panic**，非其它失败形态（Acceptance 5）。
- 进程级后果：panic 发生在异步执行路径 ⇒ 测试进程 `SIGABRT`（`thread caused non-unwinding panic. aborting.`）。
- 真实 CLI 入口（子进程）同样 abort：`rtsql <db> "DROP TABLE t"` / `"TRUNCATE TABLE t"` / `"DROP INDEX idx_v"` 的退出码为 `-1`（被信号杀死），stderr 为同一条 panic。
- 未损坏对照用例在 RED 阶段即通过：`test undamaged_release_paths_free_the_expected_pages ... ok`（`test result: ok. 1 passed; 0 failed`）。

### GREEN 证据（实际运行，最终工作区）

```
$ cargo test --test release_path_bounded_read_test -- --test-threads=1
running 7 tests
test drop_index_cli_reports_the_damaged_page ... ok
test drop_index_on_damaged_index_page_does_not_panic ... ok
test drop_table_cli_reports_the_damaged_page ... ok
test drop_table_on_damaged_data_page_does_not_panic ... ok
test truncate_cli_reports_the_damaged_page ... ok
test truncate_on_damaged_data_page_does_not_panic ... ok
test undamaged_release_paths_free_the_expected_pages ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.69s
```

### 三条释放路径的损坏页行为（用例与判据）

| 路径 | 损坏点（该语句实际扫描的页） | 用例 | 判据 |
|---|---|---|---|
| `DROP TABLE` | 数据页槽目录项 offset 越界 | `drop_table_on_damaged_data_page_does_not_panic` | 语句返回成功（`Response` 非 `Error`）；**不 panic / 不 abort**；释放页集合 == {数据页头, PK 树根}——损坏页的溢出链**未被释放**（若损坏被「忽略」而照常处理，链页会出现在释放集合里，用例即失败） |
| `TRUNCATE TABLE` | 数据页槽目录项 offset 越界 | `truncate_on_damaged_data_page_does_not_panic` | 语句返回成功；**不 panic / 不 abort**；释放页集合 == {旧 PK 树根}——损坏页的溢出链未被释放（头页原地重置，不进释放集） |
| `DROP INDEX` | 该二级索引的**索引树页**（根为内部结点）槽目录项 offset 越界 | `drop_index_on_damaged_index_page_does_not_panic` | 语句返回成功；**不 panic / 不 abort**；释放页集合 == {被损坏的根页}——损坏内部结点的子树被跳过 |
| 三条路径的「损坏被计数并报出」 | 同上 | `drop_table_cli_reports_the_damaged_page` / `truncate_cli_reports_the_damaged_page` / `drop_index_cli_reports_the_damaged_page`（真实 CLI 入口，退出码 + stderr） | 退出码 0（不 abort）；stderr 含 `data page N is damaged: 1 violation` / `index tree page N is damaged: 1 violation` ⇒ 损坏被**计数**（违规条数进消息）并**报出**（页号 + 明细），不是被静默吞掉 |
| 未损坏对照 | — | `undamaged_release_paths_free_the_expected_pages` | `DROP TABLE` 释放 == {数据页头, PK 树根, 溢出链页}；`TRUNCATE` 释放 == {旧 PK 树根, 溢出链页}；`DROP INDEX` 释放包含二级索引根及其整棵树 ⇒ 页集合不遗漏、不重复 |

实际 CLI stderr（`--nocapture` 观察，非判定层）：

```
[free-page-rebuild] rebuild abandoned: internal error: data page 2 of the persisted directory is damaged: slot 0 data range [65535, 65566) overlaps the page header or the slot area
[overflow-release] data page 2 is damaged: 1 violation: slot 0 data range [65535, 65566) overlaps the page header or the slot area
```

```
[free-page-rebuild] rebuild abandoned: internal error: index tree page 9 is damaged: slot 0 data range [65535, 65571) overlaps the page header or the slot area
[index-release] index tree page 9 is damaged: 1 violation: slot 0 data range [65535, 65571) overlaps the page header or the slot area
```

### 验证命令与实际结果（Plan Context 的 Verification 全部执行）

| 命令 | 结果 |
|---|---|
| `cargo test --test release_path_bounded_read_test -- --test-threads=1` | `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test drop_table_free_test -- --test-threads=1` | `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test drop_persistence_test -- --test-threads=1` | `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test free_page_lifecycle_test -- --test-threads=1` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --lib -- --test-threads=1` | `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` ⇒ **Iteration 001 的 420 条 lib 基线仍成立**（本 Cycle 改了 `table_manager.rs` / `index_manager.rs` / `integrity.rs`，该基线按 Plan 已失效并重跑） |
| `cargo test --test index_page_ownership_test -- --test-threads=1` | `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` | `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |

- 本 Cycle **未跑全量回归**（change 预算内只跑一次，归 Iteration 006 / T21）；MS35 / MS36 的全量终态 2192/0/2 对当前工作区已失效，不作为结论。
- 所有命令以 `--test-threads=1` 串行执行；判定只看 cargo 自身的退出码与 `test result:` 行。
- 用例表规模：1 行 / 150 行（≪ 300，ISS23 约束 ✓）。

### Gate 3（Task Review）逐项结论

- **Spec compliance**：RPBR R1 的 4 个 Scenario 全部映射并实现——损坏页上的删表 / 截断 / 删索引不 panic（不 abort）+ 尽力而为语义（告警 + 跳过 + 返回成功）；未损坏路径页集合不变（既有三条释放用例 0 failed）。Requirement「释放路径读取页内容时有边界检查」：数据页走 `check_data_page`、索引树页走 `check_tree_page`（与 `collect_data_chain_strict` / `collect_tree_pages` 同一来源）✓。
- **禁止修改项**：`SlottedPageRef::get_slot_data` 的签名零改动 ✓；integrity 模块的判据（violations 定义）零改动 ✓；磁盘格式零改动 ✓；`free_page` / `complete_free_release` / `promote_free_releases` 的集合语义零改动 ✓；释放路径未被改成「遇损坏页整体失败」（语句仍返回成功）✓；未新增错误类型（复用既有 `StorageError::InvalidPageType` + `eprintln!` 告警形态）✓；无身份证明机制、无判定层、无 timer 阈值判据 ✓。
- **不变量**：未损坏库上「释放的页集合、目录变更、返回结果与既有断言一致」✓；尽力而为契约不变 ✓；MS35 的释放完成标记时点不变 ✓；MS36 的 G1/G2/G3 兼容面不变 ✓。
- **Code quality**：diff 无计划外修改（仅 3 个产品文件 + 1 个新测试文件）；无新增编译警告（`cargo build --lib` 无 warning）；测试不会因错误原因通过（损坏夹具真实改坏页字节，RED 为真越界 panic）；错误路径完整（页级 violations / invalid type / 读取失败三条降级路径各自保留）。

### Gate 4（Verified Completion）逐项结论

- Task Definition 的 Verification 全部满足且 0 failed ✓（上表：Plan 的 5 条 Verification 命令 + 3 条被覆盖代码变化的复核命令）。
- 完成声明基于当次真实运行 ✓（RED 与 GREEN 的实际输出见上）。
- 沿用的既有结论的复核：Iteration 000/001 的用例（`index_page_ownership_test` 5 passed、`index_page_ownership_stack_overflow_test` 1 passed、`ddl_rewrite_consistency_test` 6 passed）覆盖 `index_manager.rs` / `table_manager.rs`——本 Cycle 改了这两个文件 ⇒ 按「被覆盖的代码在结论产生后发生变化」定向复核并重跑（上表 3 行），原结论**仍成立**。
- 无人工验证项；无 `required` Evidence。

### 自检发现与处置

- **发现 1（已修复，Important）**：`collect_overflow_chain_pages` 起初按「有界读」换法时，若只用 `check_data_page` 的 locators 而**不**处理 violations，损坏页会被静默当作「没有溢出链」——即「忽略损坏」。已按「计数 + 报出 + 跳过该页」实现，并由 CLI 用例的 `… is damaged: 1 violation` 断言钉住。
- **发现 2（已修复，Important）**：`collect_all_pages_tolerant`（truncate 失败路径 / 恢复路径的旧树释放）与 `collect_all_pages` 同源同根因，仍走 `InternalNodeRef::get_child_page_id` 的无界切片 → 一并改为 `check_tree_page`（洞页与损坏页各自的既有降级语义不变）。
- **发现 3（已修复，Minor）**：`collect_overflow_chain_pages` 的文档注释仍写着「header-only slot reads」（与实际读法不符）→ 随换读法更正。
- **遗留 Minor 1**：`src/storage/btree/node.rs` 的四组结点访问器仍用无界切片读槽负载——`LeafNode`（`:86` / `:98`）、`LeafNodeRef`（`:555` / `:565`）、`InternalNode`（`:673` / `:685`）、`InternalNodeRef`（`:1041` / `:1051`，Plan 点名的两处）：在**读/写热路径**（`find_child_page_id`（`:1064`）/ `find_child_page_id_binary`（`:1097`）/ `get_all_separators`（`:1004`）与 BTree 下降）上，损坏的索引页仍可能越界切片 panic。该面属**读路径**（本 Cycle 的 Excluded scope 明列「Iteration 003 的读路径」），本 Cycle 不改；已作为 Issue 候选记录（见下）。
- **遗留 Minor 2**：索引树页的**遍历顺序**变化——`check_tree_page` 的 `children` 对内部结点给出「槽子指针… + 最左子指针」，与修复前的「最左子指针 + 槽子指针」相反 ⇒ 多页索引树的**释放页序列**顺序变化（最左子树先被遍历）。**释放页集合完全相同**，既有断言全部保持通过（Plan Context 的 Invariants 以「释放的页集合」为准，Implementation Guidance 明确要求「直接复用 `check_tree_page` 的 `children`」）。记录为非阻塞 Minor。

### Deviations from Plan

1. **用例数多于 Plan 的 4 条**（7 条）：Plan 的 4 条 = 3 条损坏页 + 1 条未损坏对照；本次额外增加 3 条**真实 CLI 入口**用例。理由：「损坏页被计数并报出」是本 Cycle 的行为要求之一，而进程内的 `eprintln!` 在 libtest 下被捕获、无法在进程内断言 ⇒ 用真实入口（子进程退出码 + stderr）观察「计数 + 报出」，并顺带满足「场景走真实入口」的验证纪律。Plan 的 4 条用例全部保留且判据不变。
2. **`collect_all_pages_tolerant` 一并换读法**（Plan 的 T9 Targets 只点名 `collect_all_pages`）：同文件、同根因层、同为释放路径（truncate 失败路径 / 恢复路径的旧树释放）；不换则该路径仍会因损坏页 abort。属执行依据范围内的有限扩展，已在自检发现 2 记录。
3. **新增 `integrity::violation_summary`**：Plan 的 Forbidden 只禁「判据本身」的改动；本助手只格式化告警文案（计数措辞），不参与任何判定。
4. **收尾全量的 Iteration 指针**：冻结的 Plan Context 未更新（change 的 Iteration Map 已调整为 000→006），收尾全量归 **Iteration 006 / T21**；本 Cycle 按 M19 只跑定向验证，未跑全量。

### Persisted Evidence

- `None required`（Plan 的 Persisted Evidence = `none`）：判别读数固化在测试代码中（随用例重跑）；RED / GREEN 的实际输出与命令已写入本 Response，无需另存 `evidence/` 目录。

### Experience Candidates

1. **Issue 候选（范围外实质缺陷）**：`src/storage/btree/node.rs` 的结点访问器族（`LeafNode` `:86`/`:98`、`LeafNodeRef` `:555`/`:565`、`InternalNode` `:673`/`:685`、`InternalNodeRef` `:1041`/`:1051`）在**读/写热路径**上仍无边界检查——损坏的索引页在普通 `SELECT` / `INSERT` 路径上仍可能越界切片 panic（与 B-4 同根因层，但不在释放路径）。证据：本 Cycle 的 RED 输出给出同一无界读点在释放路径上的 panic 形态（`slotted_page.rs:128`，`range start index 65535 out of range for slice of length 4096`），而 `index_manager.rs:299-346` / `:532-540` 的读路径仍走 `InternalNodeRef` / `LeafNodeRef`。承载建议：本 change 的 **Iteration 003（读路径）**，或另开一个 change 的读面修复；本 Cycle 不实施（Excluded scope 明列读路径）。
2. **Runbook 候选**：无（本次流程为一次性定向验证，无「已端到端验证成功且可重复或风险较高」的可复用操作路径需要沉淀；损坏夹具手法已固化在 `tests/release_path_bounded_read_test.rs` 的代码里）。

### 未解决问题与停止条件

- 未解决的 Critical / Important 问题：**无**。
- 未解决项：遗留 Minor 1（读/写热路径的 `InternalNodeRef` 无界读点，范围外，已记 Issue 候选）；遗留 Minor 2（索引树释放页的遍历顺序变化，集合不变，非阻塞）。
- 停止条件未触发：修复未改动 `SlottedPageRef::get_slot_data` 的签名、未改 integrity 模块的判据、未把释放路径改成整体失败 ⇒ T9 的 Stop when 均未命中。
- 本 Cycle 的 task（T8、T9）全部完成且 Gate 3 / Gate 4 通过 ⇒ 等待 Plan Review（`Review Result` 仍为 `pending`，由 Plan 填写）。


## Plan Review

- Review Result: accepted

**Findings**

1. **健康路径的语义等价——独立复核成立**（偏差分类：无偏离）。

   - `collect_overflow_chain_pages`：修复前 `get_slot_data(&slot)` → `decode_locator(slot_data[VersionHeader::SIZE..])`；修复后 integrity 的 `check_data_page` 对**同一段字节** `decode_locator(data[start + VersionHeader::SIZE..end])`。逐项比对：① 槽集合同源同序（同一 `get_slot(i)` 序列）；② 字节区间完全相同；③ `len == VersionHeader::SIZE` 的边界——旧路径给 `decode_locator` 一个空切片 ⇒ 返回 `None`，新路径直接跳过 ⇒ 同一结果；④ `first == 0` 的跳过规则不变；⑤ locator 顺序不变。⇒ **合法页上等价**。
   - `collect_all_pages`（索引树）：修复前的子指针集合 = `leftmost_child()` + 每个槽的 `get_child_page_id(i)`（要求 `data.len() >= MAX_KEY_LEN + 4`，否则返回 `None` 被跳过）；修复后 = `check_tree_page` 的 `children` = 每个**通过槽边界校验**且 `end - start >= MAX_KEY_LEN + 4` 的槽的子指针 + `header.next_page_id`（> 0 时）。两个判据的**阈值相同**、**字段相同**（`header.next_page_id` 即 `leftmost_child()` 与 `next_leaf_page_id()` 读的同一字段）。⇒ **合法页上子指针集合完全相同**。
   - `validate_slots` 在合法页上**无误判**（这是「未损坏路径行为不变」的前提）：合法页满足 `free_space_offset ∈ [16, slot_area_start]`（`free_space_offset` 是行数据区的末端，槽目录在其后），且槽的 `start/end` 落在数据区内 ⇒ 不产生 violation ⇒ 合法页不会被误判为损坏页。
   - 叶子页：旧路径只读 `header.next_page_id`（`LeafNodeRef::next_leaf_page_id()` = `slotted.header().next_page_id`），新路径读同一字段 ⇒ 相同。

2. **`violation_summary` 只做格式化，不参与判定**（偏差分类：无偏离）。

   - `git diff HEAD -- src/storage/integrity.rs` 的全部改动 = 新增 `pub(crate) fn violation_summary` + `check_data_page` / `check_tree_page` / `DataPageCheck` / `TreePageCheck` 的可见性放宽（MS35 已做的 `pub(crate)`，注释已标注来源）。violations 的定义、locators / children 的取法**逐字未改**。
   - 调用点仅 3 处（`src/storage/data/table_manager.rs:1614`、`src/storage/btree/index_manager.rs:653`、`:728`），全部位于 `eprintln!` 内 ⇒ 不参与任何判定。

3. **「损坏被计数并报出，不被静默吞掉」——独立复核成立**（偏差分类：无偏离）。

   - 进程内：三条损坏页用例断言释放集合**不含**被跳过的链页 / 子树 ⇒ 若实现「忽略损坏照常处理」，链页会出现在释放集合里 ⇒ 用例失败 ⇒ 判别力强。
   - 真实 CLI：退出码 0 + stderr 含 `data page N is damaged: 1 violation: …` / `index tree page N is damaged: 1 violation: …` ⇒ 「计数」（违规条数进消息）与「报出」（页号 + 明细）两个要求都被**用户可见输出**钉住，而非内部状态。
   - Act 自检发现 1 已闭合「只取 locators 而不处理 violations ⇒ 损坏被静默当作没有溢出链」这一风险。

4. **损坏页跳过导致的空间泄漏——独立判定：可接受，且是唯一安全选项**（偏差分类：新证据）。

   - 事实：被跳过的**溢出链**与**子树**页既不释放、也不进可分配集合 ⇒ **永久泄漏**（空间不回收、页号不再复用）；损坏页**自身**仍在释放集合内（页号在计算 children / locators 之前入集）。
   - 判定：这是**不 abort 的代价**，且延续既有契约——`collect_all_pages_tolerant` 的既有文档注释即写明损坏结点的子树「leaked by design, warned」；释放路径的既有语义是尽力而为（`design.md` D6）。替代方案只有两种：① 照旧 abort（旧行为，等于什么都没修）；② 猜测哪些页属于被跳过的链 / 子树 ⇒ 会释放属于其它对象的页 ⇒ 静默损坏。⇒ **保守跳过是唯一安全选项**。
   - 与「是否引入新泄漏」的区别：修复前损坏页 ⇒ 进程 abort（整个会话丢失）；修复后 ⇒ 会话存活、空间泄漏。⇒ 相对旧行为是**严格改善**，不是新引入的缺陷。
   - 一处需要记录的**窄化**（非阻塞 Minor）：旧 `collect_all_pages` 的叶子分支只读 `header.next_page_id`、**不读槽** ⇒ 一个**叶子页**损坏时旧代码仍会沿 `next_leaf_page_id` 继续遍历并释放后续叶子；新代码把该叶子判为损坏页 ⇒ **不再沿链继续** ⇒ 后续叶子页改为泄漏。⇒ 损坏页上的释放集合比修复前**更小**（保守方向，不影响「未损坏路径行为不变」这一 Acceptance）。

5. **7 个用例的判别力——逐条核对**（偏差分类：无偏离）。

   - RED 的形态：三条损坏页用例的失败点是 `src/storage/page_format/slotted_page.rs:128:19` 的 `range start index 65535 out of range for slice of length 4096` ⇒ **真越界切片**（夹具把槽目录项的 offset 字段改写为 `0xFFFF` ⇒ `start = 65535`）。**独立佐证**（非重跑）：HEAD 版的 `collect_overflow_chain_pages` / `collect_all_pages` 确实经 `get_slot_data` 取槽数据，而 `get_slot_data`（`slotted_page.rs:125-129`）就是 `&self.data[start..end]`、**无边界检查** ⇒ 旧代码在该夹具下必然 panic ⇒ RED 不是自造的失败形态。
   - 进程级后果：panic 发生在异步执行路径 ⇒ 非 unwind 展开 ⇒ 进程 abort ⇒ 测试二进制整体失败（不是单条用例的普通失败）。
   - 真实 CLI 入口：`Command::new(env!("CARGO_BIN_EXE_rtsql"))` ⇒ 真实二进制；判定只看**退出码**与 **stderr 文本** ⇒ 锚定用户实际所见。
   - 未损坏对照：断言完好库上 `DROP TABLE` / `TRUNCATE` / `DROP INDEX` 的**精确**释放集合（含溢出链页、含整棵二级索引树）⇒ 防空洞通过。

6. **身份证明机制与判定层——独立核对：不存在**（偏差分类：无偏离）。

   - 本次改动面无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明；夹具里的页类型断言是**夹具前置条件**（读文件字节确认被改的是索引树页），不是身份证明。
   - 判定全部落在 `cargo test` 内（`tests/release_path_bounded_read_test.rs`）；无框架外脚本 / 封装命令 / 专用参数；无 timer 阈值行为判据（CLI 子进程无超时判据）。

7. **Act 报告的遗留 Minor ① 实际范围大于报告**（偏差分类：新证据）。

   - Act 点名的 8 处**全部确认**：`src/storage/btree/node.rs` 的 `LeafNode` `:86`/`:98`、`LeafNodeRef` `:555`/`:565`、`InternalNode` `:673`/`:685`、`InternalNodeRef` `:1041`/`:1051` 均走 `get_slot_data`。
   - 可达性**独立确认**：`src/storage/btree/index_manager.rs:299-346`（`search_from_page_async` → `LeafNodeRef::find_key_position_binary` / `InternalNodeRef::find_child_page_id_binary` → `get_key` → `get_slot_data`）与 `:532-540`（`search_all_from_page_async` → `leaf.find_all_matches` / `leaf.get_row_id`）是普通 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 的索引路径 ⇒ 损坏的索引页在这些路径上仍会越界切片 panic。⇒ Issue 候选成立。
   - **范围更正**：同一无界读形态在生产代码中共约 **25 处**——`src/wal/recovery.rs` ×4、`src/executor/data_scan.rs` ×2、`src/executor/create_index.rs`、`src/transaction/manager.rs`、`src/storage/data/table_manager.rs` ×3、`src/storage/data_page.rs` ×2、`src/storage/btree/node.rs` ×8、`src/storage/buffer_pool.rs` ×3、`src/storage/catalog.rs` ×6。⇒ `node.rs` 只是其中一族，承载决策必须按**子系统与「正确行为」语义**切分，不能按文件切分（见 Follow-up Decision）。

8. **规格增量 `release-path-bounded-read/spec.md` 的一处措辞缺陷——已在 Plan 层更正**（偏差分类：Plan 遗漏）。

   - 原文「损坏页上的删索引不 panic」的 GIVEN 写的是「其某个待删二级索引所依赖的**数据页**槽位偏移越界」。独立复核：`DROP INDEX` 的执行器（`src/executor/drop_index.rs:44-77`）只读该索引**自己的树页**（`old_tree.collect_all_pages()`），**从不读该表的数据页** ⇒ 按字面实现的夹具会去损坏一个该语句根本不扫描的页 ⇒ 该 Scenario 会**空洞通过**。Plan Context 的 Investigation Facts 已登记该偏差并交给 T20 收敛。
   - 实际实现是对的：Cycle 的 Acceptance 3 与夹具都损坏**索引树页**（Act 实测 stderr 明确是 `index tree page 9 is damaged`）。
   - ⇒ 本次 Review 在 Plan 层**直接更正**了该 delta 的 GIVEN 页种措辞（`specs/release-path-bounded-read/spec.md`），使其与已验收行为一致；**Requirement 语义不变**（仍是「不 panic」+ 尽力而为）。T20 保留其余对齐项（本 Scenario 仍缺一条与另两个 Scenario 平行的「按尽力而为语义结束」AND 子句，属 T20 的对齐范围）。

9. **Act 的 Deviations 复核**（偏差分类：Plan 遗漏，Act 在既有授权内补齐 ⇒ 不需返工）。

   - Deviation 1（7 条用例 > Plan 的 4 条）：新增的 3 条是真实 CLI 入口用例；Plan 的 Acceptance 5 要求「损坏被计数并报出」，而进程内的 `eprintln!` 在 libtest 下被捕获、**无法在进程内断言** ⇒ 真实入口是**必要**的补充，不是范围蔓延；Plan 的 4 条用例全部保留且判据不变。成立。
   - Deviation 2（`collect_all_pages_tolerant` 一并换读法）：同文件、同根因层、同为释放路径（truncate 失败路径 / 恢复路径的旧树释放）；不换则该路径仍会 abort ⇒ 属 T9「释放路径的槽位扫描改用有界读」的执行依据内。成立。
   - Deviation 3（新增 `violation_summary`）：只格式化，不参与判定（见 Finding 2）。成立。
   - Deviation 4（收尾全量指针记为 Iteration 006 / T21）：与 `tasks.md` 的 Iteration Map 一致；本轮因承载落点新增而顺延为 **Iteration 007 / T21**（见 Iteration Plan Update）。成立。

**Acceptance Gaps**

- **无阻塞项**：Cycle 的 Acceptance 六项逐项满足——① 损坏页上的 `DROP TABLE` 不 panic（`drop_table_on_damaged_data_page_does_not_panic`）；② 损坏页上的 `TRUNCATE` 不 panic（`truncate_on_damaged_data_page_does_not_panic`）；③ 损坏页上的 `DROP INDEX` 不 panic（`drop_index_on_damaged_index_page_does_not_panic`）；④ 未损坏路径行为不变（`undamaged_release_paths_free_the_expected_pages` + 三条既有释放用例 0 failed）；⑤ 判别力（RED 为真越界切片 panic，未损坏对照在 RED 阶段即通过）；⑥ 既有回归零破坏（`cargo test --lib` 420/0）。
- **非阻塞 1**（承载落点变更）：Act 的 Issue 候选（`node.rs` 无界读点）经独立复核**范围更大**（同形态约 25 处）且**与 Iteration 003 的故障域不同** ⇒ 不并入 Iteration 003；改由**本 change 的新增 Iteration 006** 承载（见 Follow-up Decision 与 Iteration Plan Update）。
- **非阻塞 2**（规格增量措辞）：RPBR delta 的「删索引」Scenario 的 GIVEN 页种写错 ⇒ 已在 Plan 层更正（Finding 8）。
- **非阻塞 3**（损坏页上的释放集合窄化）：叶子页损坏时不再沿兄弟链继续 ⇒ 后续叶子页泄漏（Finding 4）⇒ 保守方向，不影响任何 Acceptance。

**Evidence**

- 本次 Review **独立阅读**的代码与产物：`src/storage/integrity.rs`（全文 + `git diff HEAD`）、`src/storage/data/table_manager.rs`（`collect_overflow_chain_pages` / `collect_data_pages` / `alter_drop_column` / `apply_column_drop`）、`src/storage/btree/index_manager.rs`（`collect_all_pages` / `collect_all_pages_tolerant` / 读路径 `:299-346`、`:532-540`）、`src/storage/btree/node.rs`（四组访问器 + `get_key` / `get_child_page_id` / `find_child_page_id_binary`）、`src/storage/page_format/slotted_page.rs`（`get_slot` / `get_slot_data`）、`src/executor/drop_index.rs`（全文）、`tests/release_path_bounded_read_test.rs`（全文）；change 的 `tasks.md` / `design.md` / `proposal.md` / 五份规格增量；`.agents/issues/ISS25-*`。
- 与 HEAD 的语义比对：`git show HEAD:src/storage/btree/index_manager.rs` 与 `git show HEAD:src/storage/data/table_manager.rs` 的原始 `collect_all_pages` / `collect_all_pages_tolerant` / `collect_overflow_chain_pages` 逐行比对。**说明**：HEAD `0a08b14` 是 MS33 基线，MS34/MS35/MS36 与 Iteration 000/001 的改动**全部未提交** ⇒ 比对按**函数体**而非按提交边界进行。
- RED 的独立佐证（非重跑）：HEAD 版释放路径经 `get_slot_data`（`slotted_page.rs:125-129`，`&self.data[start..end]` 无边界检查）；夹具把槽目录项 offset 改写为 `0xFFFF` ⇒ `start = 65535` ⇒ 该切片必 panic ⇒ 与 Act 记录的 RED 输出（`range start index 65535 out of range for slice of length 4096`）一致。
- **沿用** Act 的验证结论（覆盖范围未变化，按公共规则不重复运行）：`release_path_bounded_read_test` 7/0、`drop_table_free_test` 6/0、`drop_persistence_test` 11/0、`free_page_lifecycle_test` 12/0、`cargo test --lib` 420/0、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`ddl_rewrite_consistency_test` 6/0。
- 覆盖范围未变化的只读核对：三个产品文件与新测试文件的最后修改时间（13:01–13:06）**早于**本 Cycle 的 Act Response 落盘时间（13:09），且本批无其它子代理写入工作区 ⇒ 沿用结论成立。
- 未做：全量回归（收尾全量归本 change 的最后一个 Iteration，一次预算）。

**Follow-up Decision**

- **当前 Cycle 无需修复**：无阻塞 Acceptance 缺口 ⇒ **不**创建 rework / replan Cycle；`Plan Context` 不可改写，保持原样；Act Response 保持原样。
- **后续承载**：Act 的 Issue 候选（`node.rs` 无界读点）**不并入 Iteration 003**，改由**新增的 Iteration 006「索引树读路径的有界读与显式拒绝」**（T22–T23）承载。判定理由（与 Iteration 003 的故障域切分）：① **「正确行为」语义不同**——释放路径可以「告警 + 跳过」（物理释放尽力而为，`design.md` D6），读路径**不可以**：跳过子树 ⇒ 「查不到该行」⇒ **静默错读**，因此读路径必须是**显式错误**；② **夹具形态不同**——字节级页损坏（Iteration 002 同形态）vs. Iteration 003 的 DDL 窗口故障注入；③ **验证入口不同**；④ 工作量不同（8 个访问器族 + 其调用点的错误语义传播 + 写路径变体）⇒ 并入会使 Iteration 003 过载并模糊诊断边界。
- **剩余同形态读点**（`wal/recovery.rs` / `data_scan.rs` / `create_index.rs` / `transaction/manager.rs` / `table_manager.rs` / `data_page.rs` / `buffer_pool.rs` / `catalog.rs` 共约 17 处）属**其它子系统**（数据页 / 目录页 / 恢复面 / 缓冲池），各自的「损坏页正确行为」需单独判定（目录页是否拒绝打开？数据页是否跳过该行？恢复面如何处置？）⇒ **不并入本 change 的任一 Iteration**，登记为 Issue 候选交 Recorder 承载（见下）。
- **收尾全量指针**：原 Iteration 006（旧格式兼容证据补齐）顺延为 **Iteration 007**；收尾全量（T21）随之落在 Iteration 007。

**Issue Candidates**（只报告不登记；登记由 Recorder 执行）

1. **可承载**：`src/storage/btree/node.rs` 的索引树访问器族在读 / 写热路径上仍无边界检查（损坏的索引页 ⇒ 普通 `SELECT` / `INSERT` 路径越界切片 panic）。
   - **结论**：已实测的实质缺陷（进程 abort 级）。**归属本 change 的新增 Iteration 006**（T22–T23），已写入 `tasks.md` 的任务表、执行依据要点、Iteration Plan 与 Iteration Map；**Issue 的登记由 Recorder 执行**（Plan 只报告不登记）。
   - **根因位置**：`src/storage/btree/node.rs` 的 8 处 `get_slot_data`（`LeafNode` `:86`/`:98`、`LeafNodeRef` `:555`/`:565`、`InternalNode` `:673`/`:685`、`InternalNodeRef` `:1041`/`:1051`）。
   - **复现路径与实测证据**：本 Cycle 的 RED 输出给出同一无界读点在释放路径上的 panic 形态（`slotted_page.rs:128`，`range start index 65535 out of range for slice of length 4096`）；读路径仍走这些访问器（`index_manager.rs:299-346`、`:532-540`）⇒ 同一夹具（按文件偏移改写槽目录项 offset 为 `0xFFFF`）落在索引树页上即可复现。
   - **修复方向**：损坏的索引页在读 / 写路径上给出**显式错误**（点名损坏的页与索引），SHALL NOT 跳过子树（会变成静默「查不到」）SHALL NOT abort；未损坏路径的读 / 写行为零变化；不改磁盘格式。
   - **承载落点**：`tasks.md` 的 **T22（RED 用例）+ T23（修复）**，Iteration **006**（依赖 Iteration 002，排在收尾全量之前）；验证入口 `tests/index_read_bounded_read_test.rs`。
2. **可承载（另立 change）**：存储层其余无界读点（约 17 处：`wal/recovery.rs` ×4、`executor/data_scan.rs` ×2、`executor/create_index.rs`、`transaction/manager.rs`、`storage/data/table_manager.rs` ×3、`storage/data_page.rs` ×2、`storage/buffer_pool.rs` ×3、`storage/catalog.rs` ×6）。
   - **结论**：同一机制形态（`SlottedPageRef::get_slot_data` 无边界切片）在**其它子系统**上的分布。
   - **不并入本 change 的理由**：各子系统的「损坏页正确行为」不同（目录页：是否拒绝打开？数据页：跳过该行还是整体失败？恢复面：如何处置该页？），需要各自的调查与设计决定；且并入会使本 change 的收尾全量预算与缺陷批的边界失控。**不是按等级搁置，而是按子系统边界切分。**
   - **承载落点**：登记为 Issue 候选，由 Recorder 承载（另立一次「存储层有界读」专项 change）。
3. **无其它 Issue 候选**：除上述两条，本次 Review 未发现新的实质缺陷面。

**Iteration Plan Update**

- Iteration 002 的计划**不变**：Acceptance 六项全部满足；4 条 Deviations 均在既有授权内（见 Finding 9）；稳定结果与诊断边界按 `tasks.md` 的 Iteration 002 段落保留。
- **change 层面的计划调整**（因 Issue 候选的承载落点与收尾全量指针）：
  1. `tasks.md` 新增 **T22–T23** 与 **Iteration 006「索引树读路径的有界读与显式拒绝」**（承载 `node.rs` 的索引树访问器族；依赖 Iteration 002）；原 Iteration 006（旧格式兼容证据补齐）顺延为 **007** 并继续承担收尾全量。**T16–T21 的任务编号不变**（任务 ID 是稳定标识；不重编既有 ID，避免使已冻结 Cycle 的前向引用与 RTM 失效——与 Iteration 001 登记 Iteration 005 时的做法不同，此处是明示的取舍）。
  2. `design.md` 的 D1（Iteration 数 7 → 8 与根因层清单）、D12（收尾全量指针 006 → 007）同步更新；新增 **D13**（索引树读路径的损坏页语义：显式错误而非跳过）。
  3. 副作用（明示）：已冻结的 Iteration 000 / 001 / 002 的 `Plan Context` 与 Act Response 中「收尾全量在 Iteration 006」的指针随之**过时**（Plan Context 不可改写 ⇒ 未修改；Act Response 属 Act 的区域 ⇒ 未修改）；以 `tasks.md` 的 Iteration Map 为准。
  4. `specs/release-path-bounded-read/spec.md` 的「删索引」Scenario 的 GIVEN 页种更正（Finding 8）。

**Next Cycle**

None

**Next Iteration**

- Iteration 003 已展开：`iterations/003-rewrite-window-read-consistency/000-initial.md`（Cycle 000，`Plan Context` 状态 `ready`，承载 T10–T11：ISS25 的在途窄化窗口读一致性）。
- 下一个待展开 Iteration 为 **004**（释放可分配判定的耐久性前提）。**Iteration 006**（索引树读路径的有界读与显式拒绝）由本次 Plan Review 登记承载。
