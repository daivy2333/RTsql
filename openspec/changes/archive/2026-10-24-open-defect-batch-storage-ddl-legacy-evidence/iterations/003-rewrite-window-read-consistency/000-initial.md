# Iteration 003 / Cycle 000: 改写窗口的读一致性（ISS25）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T10, T11
- Acceptance gaps: None
- Repair items: None
- Inherited scope: DRC R1 / R2（Iteration 001 已 `accepted`：单一尾游标、数据链不分叉、墓碑写序、M2 活性判据）；DRC R3（本 Iteration 承载）；`design.md` D4（单一尾游标）、D5（先判别再修 + 墓碑写序）、D7（在途窄化状态的内存发布 / 清除 + 读路径显式拒绝）、D12（收尾全量预算）；既有域 `ddl-catalog-transactional` R1 / R2（目录**列**定义的提交点时机不变）；既有域 `mvcc-tombstone-visibility`（墓碑可见性判据不变）；ISS23 的规模约束（本 change 内不使用 ≥ 300 行的表规模构造用例）。
- Excluded scope: 磁盘格式（`VersionHeader` / 目录行布局 —— 会破坏 MS36 已验收的 G1/G2/G3 兼容面）；`deserialize_tuple_width` / `deserialize_row` 的**解码契约**本身（不以其位置性解码作为拒绝手段）；`ADD COLUMN` 路径的宽度感知解码语义；读路径**等待 DDL 完成**的阻塞语义（触及执行器生命周期与 `statement-cancellation-outcome` 域）；崩溃窗口（「提交完成 → 目录应用」之间崩溃后重开）的**持久化修复**（见 Risks）；释放路径的有界读（Iteration 002）；索引树访问器的无界读（新增 Iteration 006 承载）；`free_page` / `complete_free_release` 的集合语义；完整性扫描的诊断分级。

**Objective**

`ALTER TABLE DROP COLUMN` 的**在途窄化状态**对读路径可见：处于该状态的表被读时给出**明确拒绝**（点名该表正在被修改），SHALL NOT 返回按位置错位的值；改写完成后读路径恢复正常；未改写的表不受影响；`ADD COLUMN` 的窄行解码语义不变。⇒ 把 ISS25 从「读码结论」升级为「已复现 → 已修复」。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `0a08b14`（ahead of origin 9）。MS34 + issue-fix + MS35 + MS36 + 本 change 的 Iteration 000 / 001 / 002 的改动**全部未提交**（既有现场：不得回滚、不得清理、不得提交）；本 Cycle 的改动叠加在其上。
  - Iteration 000 / 001 / 002 的 Plan Review Result 均为 **`accepted`**（Iteration 002 于 2026-10-25 由本次 Plan Review 判定）。
  - **ISS25（`.agents/issues/ISS25-ddl-commit-point-positional-misread.md`，Status `open`）目前只是读码结论**：根因确认，**可达性未确认**（登记原文：「本条未构造故障注入夹具、未实跑复现，不得按『已复现』引用」）⇒ 本 Cycle 的第一个动作就是用夹具把它**复现**出来（T10）。
  - 可沿用的既有验证结论（覆盖范围未变化）：`cargo test --lib` **420 passed / 0 failed**（Iteration 002 收尾时）；`tests/ddl_rewrite_consistency_test.rs` **6 passed / 0 failed**（Iteration 001 建立）。本 Cycle 改 `table_manager.rs` / `alter_table.rs` 与读路径 ⇒ 这两条 lib / DRC 结论对本次改动**失效**，须重跑。
  - 既有全量终态 2192 / 0 / 2 对当前工作区已失效，不作为依据；本 change 的收尾全量在**最后一个 Iteration**（收尾验证一次预算）。
- Current-State Evidence（Plan 已逐行确认，行号为**当前工作区**）:
  - **窗口的确切位置（提交点顺序）** —— `src/executor/alter_table.rs:100-126`（`AlterTableOp::DropColumn` 分支）：
    ```
    let mut guard = self.database.implicit_tx_guard(tx_id);                        // :100
    let result = ...table_manager.alter_drop_column(...).await;                    // :101-111 改写（窄化行已写出，尚未提交）
    match result {
      Ok(pending) => {
        self.database.transaction_manager.commit(tx, &self.database.buffer_pool)     // :114-117 ← 窄化行在此变为已提交 / 可见
                                .await?;
        guard.disarm();                                                             // :119
        self.database.table_manager.apply_column_drop(&pending).await?;             // :122-125 ← 目录窄化与活布局切换在此生效
    ```
    ⇒ **「提交完成 → 目录应用」之间**存在一个真实窗口：数据面已是 **2 宽的已提交行**，目录与活布局仍是 **3 列** ⇒ 读路径按位置取值 ⇒ 静默错读。这与 ISS25 登记的触发条件（「提交完成 → 目录应用」的窗口内并发读）一致。
  - **根因（位置性解码）** —— `src/storage/page_format/tuple.rs:189-222`（`deserialize_tuple_width` 的文档与实现，函数体自 `:204` 起）：按**位置**逐列取值；行短于 schema 时**尾部补 `Value::Null`** 并返回实际解码列数；**不校验列序、不校验列数**。2 宽元组在 3 列目录下读作 `[id, w, NULL]`（`v` 列读到 `w` 的值、`w` 列读 NULL）且**无声**。
  - **全部读路径都走它**：`src/executor/data_scan.rs:519`、`:566`；`src/executor/scan.rs:119`、`:134`；`src/executor/index_scan.rs:85`、`:100`；`src/executor/index_scan_all.rs:105`、`:122`；`src/executor/create_index.rs:184`；`src/executor/delete.rs:82`；`src/executor/update.rs:142`；`src/executor/upsert.rs:350`、`:616`；`src/storage/data/table_manager.rs:928`（**改写自身**对旧行的内部读）。
  - **窄行的合法形态（不得误伤）** —— `ALTER TABLE ADD COLUMN` 之后遗留的窄行同样表现为「行短于 schema」，既有解码是「补 NULL + 叠加声明 DEFAULT」（`src/storage/page_format/tuple.rs:352` 的 `deserialize_row`）。⇒ **仅凭宽度无法区分**「合法窄行（ADD COLUMN）」与「缺陷窄行（DROP COLUMN 在途 / 提交点窗口）」⇒ 读侧**不能**用宽度检查代替在途状态（这是 `design.md` D7 选择内存状态而非读侧列序校验的前提）。
  - **改写循环内各阶段的可见性** —— `src/storage/data/table_manager.rs:924-1050`：每行先走 DELETE 腿（墓碑**不带链指针**写入 → `record_version` 登记 → 补链指针 → 索引清理 → WAL Delete），再走 INSERT 腿（窄化行经 `write_tuple(&old, &vh, &buf)` 写入，**未提交**）。⇒ 改写**进行中**时窄化行尚未提交；**提交之后、目录应用之前**窄化行已提交且可见 ⇒ 错读窗口成立。
  - **活布局的来源与读者的快照** —— `src/storage/data/table_manager.rs:277-310`（`open_or_init`：重开库时列定义取自目录列行并按 `column_count` 截断）；`TableManager::get_table`（`:641`）返回活布局的 `Arc<TableMeta>`；`src/executor/data_scan.rs:116-123` 的 `new` 从该 `Arc` 中**拷出** schema ⇒ 读者持有的是**计划期快照** ⇒ 在途状态必须能被该快照观察到。
  - **状态载体的既有约束** —— `TableMeta`（`src/storage/data/table_manager.rs:91-120`）的构造点共 8 处：`:382`、`:544`、`:698`、`:854`（`alter_drop_column` 的窄化副本 `new_meta`）、`:1235`、`:1286`、`:1349`、`:1408`。
  - **既有可借鉴的注入点形态** —— `src/storage/free_page_rebuild.rs:22-42`（路径限定 + 一次消费 + 默认不生效的 `arm_free_page_rebuild_fault` / `take_free_page_rebuild_fault`）；`src/storage/file_storage.rs:85-107`（`IoFailPoint` / `IoFailPointSlot` / `consume_io_failpoint` 的「按型、一次消费」形态）；`src/database.rs:881`（`arm_io_failure`）；`src/database.rs:143-150` 的**语句作用域守卫**先例（登记项的唯一来源是守卫的 `Drop`，正常提交 / 中止成功后显式解除）。
  - **既有测试入口与规模约束** —— `tests/ddl_rewrite_consistency_test.rs`（现有 6 条用例，Iteration 001 建立）；`tests/ddl_catalog_transactional_test.rs`（目录面回归）；`tests/ddl_crash_recovery_test.rs`（崩溃恢复面回归）；ISS23 约束：本 change 内**不使用 ≥ 300 行**的表规模。
- Code and Critical Path:
  - **写入面（在途状态的发布 / 清除）**：`src/storage/data/table_manager.rs::alter_drop_column`（`:735`，改写开始处发布）→ `::apply_column_drop`（`:1101`，目录列行改写 + 列数更新 + 活布局切换完成后清除）。**异常 / 放弃路径**（`alter_drop_column` 返回 `Err`、语句 future 被放弃）必须同样清除 ⇒ 该表不得因此永久不可读 ⇒ 形态取「语句作用域守卫」（先例 `src/database.rs:143-150`）或等价的「离开作用域即清除」机制。
  - **读取面（在途检查 + 显式拒绝）**：解码元组的全部用户可见读路径（Current-State Evidence 第 3 条列出的调用点）。**豁免**：`src/storage/data/table_manager.rs:928` 的改写自身读 —— 它读的是尚未窄化的旧行并用**旧 schema** 解码，本就正确，若一并拒绝将使改写无法进行。
  - **状态载体（必须让「持有旧活布局的读者」也能看到）**：在途状态必须与**表**绑定，并随活布局一起被读者观察到 ⇒ 放在 `TableMeta` 携带的**每表共享单元**里（新旧两个 `TableMeta` 指向**同一**单元：`new_meta` 必须复用旧表的单元，否则窗口内拒绝对计划期快照失效）；**不**放在「目录的当前值」里（目录的当前值在窗口内尚未窄化，读者从目录取值会读到错误的信息）。
  - **读者侧检查点**：读者在**解码之前**检查该状态 ⇒ 命中即返回**明确的错误**（点名该表正在被修改），SHALL NOT 返回任何按位置取值的结果，SHALL NOT 表现为「查询成功但列值错位」，也不得静默返回空结果。
  - **外部影响**：当前形态下，窗口内的并发读**无声返回错位值**（用户无从察觉）；修复后同一读返回明确错误。

**Implementation Guidance**

- 建议顺序：先 T10（夹具 + RED，观察真实错读），再 T11（发布 / 清除 + 读路径拒绝 + 两条对照用例），最后跑回归。
- **夹具形态**：路径限定 + 一次消费 + 默认不生效的注入点，把 `ALTER TABLE … DROP COLUMN` **停在「提交完成 → 目录应用」之间**（`src/executor/alter_table.rs:114-125` 之间），并暴露一个**可观察的锚点状态**供测试用 `yield_now` 轮询定位（timer 只作夹具失控兜底）。先例：`arm_free_page_rebuild_fault`（`src/storage/free_page_rebuild.rs`）。注入点必须默认不生效，且**路径限定**（避免并行测试二进制互抢）。
- **先看到错读，再谈修复**：T10 的 RED 必须是**真实的错位值**（`v` 列显示 `w` 的值、`w` 列显示 NULL），而不是「读到旧宽行」「读到空结果」或「读到明确错误」。若窗口内读到的不是错位值 ⇒ **不得**放宽断言：把实际读数写进 Act Response 并按 T10 的 Stop when 退回 Plan（ISS25 的可达性与错读形态本身是待验证项）。
- **修复形态**：在改写开始处发布在途状态、在目录窄化生效后清除；异常 / 放弃路径由守卫兜底清除（否则该表永久不可读，比原缺陷更严重）。读路径在解码前检查并给出明确拒绝。
- **关键取舍**：**不得**改磁盘格式 ⇒ 不能给元组加列数标记，也不能给目录行加「在途窄化」字段；**不得**用「行宽 ≠ schema 宽度」作为拒绝条件（会误伤 `ADD COLUMN` 的合法窄行，违反 DRC R3 的「窄行与宽行的解码语义不变」）。
- **拒绝的粒度**：按**表**（错误信息点名该表），不是按库 / 全局。
- **读路径的覆盖面**：所有会解码该表元组的用户可见读路径都要检查（含索引扫描与 DML 的旧行读取），但**改写自身的内部读**豁免。

**Task Definitions**

### T10: ISS25 的先行 RED 用例 + 故障注入夹具（复现在途窄化窗口内的位置性错读）

- Requirement/Scenario: DRC R3（`specs/ddl-rewrite-consistency/spec.md`）Scenario「在途窄化的表被读时显式拒绝」；`design.md` D7；ISS25 的登记现场（可达性待验证）。
- Depends on: None
- Targets: `tests/ddl_rewrite_consistency_test.rs`（新增用例）；测试支撑的注入点（路径限定 + 一次消费 + 默认不生效）。
- Current behavior: 已知（**读码结论，未实跑**）：`ALTER TABLE t DROP COLUMN` 的「提交完成 → 目录应用」窗口内，`SELECT` 该表按 3 列目录解码 2 宽的已提交行 ⇒ 返回 `[id, w, NULL]`（`v` 列读到 `w` 的值、`w` 列读 NULL）且**无错误**。
- Required behavior: 用例必须**真实复现**该错读：真实文件库 + 真实 `Database::open` + 真实 `ALTER TABLE … DROP COLUMN` + 真实 `SELECT`；断言当前实现读到的是**错位值**（不是错误、不是空结果），从而把 ISS25 从「读码结论」升级为「已复现」。
- Preserve: `ADD COLUMN` 路径的窄行解码语义（补 NULL + 声明 DEFAULT 叠加）；既有 6 条 `ddl_rewrite_consistency_test` 用例的行为；磁盘格式。
- Forbidden: 不得用「跳过读」之类的替身代替真实窗口；不得改磁盘格式；不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、框架外判定脚本 / 封装命令 / 专用参数或 timer 阈值行为判据（timer 只作夹具失控兜底）；不得使用 ≥ 300 行的表规模（ISS23 约束）。
- First test: 在 `tests/ddl_rewrite_consistency_test.rs` 中新增用例 `read_during_narrowing_in_flight_is_refused`（用例名可微调，断言语义不变）：建三列表（`id INT PRIMARY KEY, w INT, v STRING`）并写入若干行（≪ 300 行）⇒ 布防夹具 ⇒ 在独立任务中执行真实 `ALTER TABLE t DROP COLUMN v` ⇒ 锚点到「提交完成 → 目录应用」窗口 ⇒ 在窗口内执行真实 `SELECT * FROM t` ⇒ 断言「应当是明确拒绝」而当前实现返回**错位值** ⇒ RED。变更前运行 `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`，预期 **RED**（新用例失败且失败信息显示错位值），既有 6 条用例仍通过。
- GREEN condition: 本 Task 产出**真实错读的 RED 证据**后进入 T11；本 Task **不改产品代码**（除夹具所需的测试支撑注入点外，注入点默认不生效）。
- Verification: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` ⇒ 新用例 **RED**（失败形态 = 读到错位值），既有 6 条 **0 failed**。
- Stop when: 夹具无法稳定停在「提交完成 → 目录应用」窗口 ⇒ 退回 Plan 调整夹具形态；或窗口内读到的**不是**错位值（读到旧宽行 / 空结果 / 明确错误）⇒ **不得**放宽断言：把实际读数写进 Act Response 并退回 Plan（ISS25 的可达性与错读形态是待验证的实质未知项）；或复现需要改磁盘格式 ⇒ 退回 Plan。

### T11: 在途窄化状态的发布 / 清除 + 读路径的在途检查与显式拒绝

- Requirement/Scenario: DRC R3 的四个 Scenario（在途窄化的表被读时显式拒绝 / 改写完成后读路径恢复正常 / 未改写的表读路径不受影响 / 窄行与宽行的解码语义不变）；`design.md` D7。
- Depends on: T10
- Targets: `src/storage/data/table_manager.rs`（`alter_drop_column` / `apply_column_drop` / `TableMeta` 及其 8 个构造点）；`src/executor/alter_table.rs`（提交点两侧的发布 / 清除时机与异常 / 放弃路径的兜底清除）；解码元组的用户可见读路径（`src/executor/data_scan.rs`、`scan.rs`、`index_scan.rs`、`index_scan_all.rs`、`delete.rs`、`update.rs`、`upsert.rs`、`create_index.rs`）。
- Current behavior: 窗口内读路径按位置解码 2 宽行 ⇒ 静默错读（T10 的 RED 证据）。
- Required behavior: ① 改写开始时**发布**该表的在途窄化状态；② 目录窄化生效后**清除**该状态；③ 异常 / 放弃路径也**清除**（该表不得因此永久不可读）；④ 处于该状态的表被**任何**用户可见读路径读取时给出**明确的拒绝**（错误信息点名该表正在被修改），SHALL NOT 返回任何按位置取值的结果，SHALL NOT 表现为「查询成功但列值错位」，也不得静默返回空结果；⑤ 改写完成后读路径恢复正常；⑥ 未改写的表读路径不受影响。
- Preserve: `ADD COLUMN` 的窄行解码语义；`deserialize_tuple_width` / `deserialize_row` 的既有解码契约（**不**以位置性解码本身作为拒绝手段）；磁盘格式（`VersionHeader` / 目录行布局）；目录**列**定义的提交点时机（既有域 `ddl-catalog-transactional` R1 / R2 不变）；「被放弃 / 失败的 DDL 不留目录面变更」（ISS21 / D5 的既有保证）；Iteration 001 的单一尾游标与墓碑写序；改写自身对旧行的内部读（豁免检查）；`free_page` / `complete_free_release` 的集合语义。
- Forbidden: 不得改磁盘格式（会破坏 MS36 已验收的 G1/G2/G3 兼容面）；不得用「行宽 ≠ schema 宽度」作为拒绝条件（会误伤 `ADD COLUMN` 的合法窄行）；不得引入读路径等待 DDL 完成的阻塞语义（触及 `statement-cancellation-outcome` 域）；不得引入身份证明机制、框架外判定层或 timer 阈值行为判据；不得改 `mvcc-tombstone-visibility` 的既有判据。
- First test: 沿用 T10 的用例（先 RED 后 GREEN）；并补两条对照用例 —— 「改写完成后读路径恢复正常」与「未改写的表读路径不受影响」——与 T10 同文件、同夹具形态。
- GREEN condition: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` ⇒ 全部通过（T10 的用例由 RED 转 GREEN；既有 6 条保持通过；两条对照用例通过）。
- Verification: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`（0 failed）；`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`（目录面零回归）；`cargo test --test ddl_crash_recovery_test -- --test-threads=1`（崩溃恢复面零回归）；`cargo test --lib -- --test-threads=1`（lib 基线 420 条）。任一命令 failed > 0 ⇒ 本 Cycle 未完成。
- Stop when: 拒绝需要改磁盘格式或改 `deserialize_tuple_width` 的解码契约 ⇒ 回到 Plan；或在途状态无法在异常 / 放弃路径上可靠清除（该表会永久不可读）⇒ 回到 Plan；或 T10 的 RED 证据与 ISS25 的登记现场不一致 ⇒ 回到 Plan。

**Invariants**

- 磁盘格式不变（`VersionHeader` 布局、目录行布局）；MS36 已验收的 G1/G2/G3 兼容面不变。
- `ADD COLUMN` 的窄行解码语义不变（补 NULL + 声明 DEFAULT 叠加）。
- 目录**列**定义的提交点时机不变（既有域 `ddl-catalog-transactional` R1 / R2）；改写被放弃 / 失败时**不留目录面变更**。
- Iteration 001 的成果不变：单一尾游标、数据链不分叉、墓碑写序与 M2 活性判据。
- 未改写的表的读路径**零变化**；改写完成后读路径恢复正常。
- 不引入身份证明机制、框架外判定层或 timer 阈值行为判据；判定一律在 `cargo test` 内。

**Non-goals**

- 磁盘格式与目录行布局的任何改动。
- 崩溃窗口（「提交完成 → 目录应用」之间崩溃后重开仍读到窄行）的**持久化修复**（见 Risks）。
- 索引树访问器的无界读（新增 Iteration 006 承载）。
- 读路径等待 DDL 完成的阻塞语义；`ADD COLUMN` 路径的宽度感知解码行为；`free_page` 集合语义；完整性扫描的诊断分级。

**Acceptance**

1. **在途窄化的表被读时显式拒绝**：真实文件库 + 真实 `ALTER TABLE t DROP COLUMN`（三列表）+ 真实 `SELECT`；窗口内读取 SHALL 给出**明确的拒绝**（点名该表正在被修改），SHALL NOT 返回按位置取值的结果，SHALL NOT 表现为「查询成功但列值错位」。映射：DRC R3 Scenario「在途窄化的表被读时显式拒绝」、T10、T11。
2. **改写完成后读路径恢复正常**：同一库在改写完成后 `SELECT` ⇒ 正常返回窄化后的行，SHALL NOT 再给出在途拒绝。映射：DRC R3 Scenario「改写完成后读路径恢复正常」、T11。
3. **未改写的表读路径不受影响**：另一张未改写的表在同一窗口内被读 ⇒ 正常返回其行，SHALL NOT 给出在途拒绝。映射：DRC R3 Scenario「未改写的表读路径不受影响」、T11。
4. **窄行与宽行的解码语义不变**：`ADD COLUMN` 之后遗留的窄行仍按「补 NULL + 声明 DEFAULT 叠加」解码。映射：DRC R3 Scenario「窄行与宽行的解码语义不变」、T11 + 既有 `tests/ddl_rewrite_consistency_test.rs`。
5. **判别力**：T10 的 RED 必须是**真实的错位值**（`v` 列读到 `w` 的值、`w` 列读 NULL）而非其它失败形态；夹具必须真实停在「提交完成 → 目录应用」窗口；未改写的表与改写完成后的读**不**被误拒（防空洞通过）。映射：T10 的 First test。
6. **既有回归零破坏**：`cargo test --lib` 与既有 DDL / 目录 / 崩溃恢复用例全部 0 failed；本 Cycle 不跑全量（全量归本 change 的最后一个 Iteration）。映射：T11 的 Verification。

**Verification**

- 行为类 scenario（1–4）走**真实入口**：真实文件库 + 真实 `Database::open` + 真实 `ALTER TABLE` / `SELECT`；窗口由**路径限定的注入点**制造（真实暂停，不是替身）。
- 判定命令：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` ⇒ 0 failed（scenario 1–5）。
- 回归命令：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`、`cargo test --test ddl_crash_recovery_test -- --test-threads=1`、`cargo test --lib -- --test-threads=1` ⇒ 各自 0 failed（scenario 4、6）。
- 全部命令以 `--test-threads=1` 串行执行；判定只看 cargo 自己的退出码与原生输出（允许用一步通用管道读计数，管道只做计数不做判定）。
- 不跑全量回归（本 change 一次预算，归最后一个 Iteration 的收尾全量）。
- 无人工验证项；无身份证明机制；无框架外判定层。

**Gate 2 Readiness**

- 需求与场景：DRC R3 的四个 Scenario 已逐条映射到 T10 / T11 与 Acceptance 1–4 ✓（BLOCKED: 无）。
- 实现调查：窗口的确切位置（`src/executor/alter_table.rs:114-125`）、根因位置（`src/storage/page_format/tuple.rs:189-222`）、全部读路径调用点、窄行合法形态与「宽度不可区分」的前提、改写循环的可见性阶段、活布局与读者快照、状态载体约束、注入点与守卫先例 ⇒ 均已落到**当前工作区的行号** ✓（BLOCKED: 无）。
- 任务分轮：T10（夹具 + RED）→ T11（发布 / 清除 + 拒绝 + 两条对照用例）两轮可独立验证 ✓（BLOCKED: 无）。
- 追踪：`tasks.md` 的 RTM 行「DRC R3 改写在途期间读路径不静默错读 | T10, T11 | `tests/ddl_rewrite_consistency_test.rs::read_during_narrowing_in_flight_is_refused`」与本 Cycle 一致 ✓（BLOCKED: 无）。
- 当前轮验证：Verification 列出可直接运行的命令与通过条件；无人工项 ✓（BLOCKED: 无）。
- 结论：**Gate 2 通过**——需求、调查、任务分轮、追踪与验证均达执行就绪；Plan Context 状态置 `ready`。

**Persisted Evidence**

- Mode: none

判别读数固化在测试代码中（随用例重跑）；RED / GREEN 的实际输出与命令写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录。

**Risks and Notes**

- **ISS25 的可达性是待验证项**（登记原文：「未构造故障注入夹具、未实跑复现」）：T10 就是它的验证动作。若窗口不可复现（读到的不是错位值），按 T10 的 Stop when 退回 Plan，**不得**改断言迁就。
- **崩溃窗口不在本 Cycle 的修复面**：「提交完成 → 目录应用」之间**崩溃**后重开，在途状态（纯内存）已消失 ⇒ 目录仍是 3 列而数据面是 2 宽的已提交行 ⇒ 重开后的读**仍会错读**。该形态需要持久化标记或改变提交点顺序（后者与 ISS21 / D5 的「被放弃 / 失败的 DDL 不留目录面变更」冲突）⇒ 属新的设计决定，**不**在本 Cycle 的范围内。Act 必须在 Act Response 记录实际观察（是否可复现、重开后的读数），由 Plan 在收尾时作为 Issue 候选承载。
- **计划期快照的读者**：读者持有的是计划期的 `Arc<TableMeta>`（旧 schema）⇒ 在途状态必须随该快照可见（共享单元），否则窗口内的拒绝不会生效。
- **既有并发面（非本 Cycle 引入）**：改写完成（目录已窄化）后，仍在执行的旧计划读者按旧 schema 解码新数据 —— 属既有的计划失效面（`Database` 的 `schema_version` 机制负责计划缓存侧），不由本 Cycle 引入，也不由本 Cycle 修复。
- 规模：用例表规模 ≪ 300 行（ISS23 约束）。


## Act Response

- Status: reported

### 任务状态

| Task | 状态 | 交付 |
|---|---|---|
| T10 ISS25 先行 RED 用例 + 故障注入夹具 | completed | `tests/ddl_rewrite_consistency_test.rs`（新增 `ROWS_T10`、`park_at_commit_window` 夹具 + `read_during_narrowing_in_flight_is_refused`）；`src/executor/ddl_commit_window.rs`（新增：路径限定 + 一次消费 + 默认不生效的受控暂停点）；`src/executor/mod.rs`（`pub mod ddl_commit_window;`）；`src/database.rs`（`Database::path` + `path: PathBuf`） |
| T11 在途窄化状态的发布 / 清除 + 读路径的显式拒绝 | completed | `src/storage/data/table_manager.rs`（`NarrowingState` / `NarrowingGuard` / `TableMeta.narrowing` / 8 个构造点 / `alter_drop_column` 发布 / `apply_column_drop` 清除）；`src/storage/data/mod.rs`、`src/storage/mod.rs`（导出）；8 个读路径执行器（`data_scan` / `scan` / `index_scan` / `index_scan_all` / `create_index` / `delete` / `update` / `upsert`）；`tests/ddl_rewrite_consistency_test.rs`（两条对照用例） |
| T12 Plan Context 的 Verification（收尾验证） | completed | 4 条定向命令全部 0 failed（见下）；**未跑全量**（收尾全量归 Iteration 007 / T21） |

> 编号说明：派单写的「T10–T12」中，change `tasks.md` 的 **T10 / T11** 属 Iteration 003，**T12 / T13** 属 Iteration 004（释放可分配判定的耐久性前提，另一故障域）；派单的「T12」= Plan Context 的 **Verification** 段。本 Cycle 只做 T10 + T11 + Verification，**未触碰 Iteration 004 的 T12 / T13**。

### 前次在途工作的独立核对结论（逐点）

| 核对点 | 结论 | 依据 |
|---|---|---|
| 暂停点的**语义位置**（必须在「提交完成、目录窄化之前」） | **一致**，无需修正 | `src/executor/alter_table.rs`：`alter_drop_column`（`:104`）→ `commit(tx, …)`（`:114-117`）→ `guard.disarm()`（`:119`）→ `pause_at_ddl_commit_window(self.database.path())`（`:125-128`）→ `apply_column_drop(&pending)`（`:131-134`）。夹具恰好落在 Plan Context 指定的窗口内 |
| **不布防时的零行为差异** | **一致** | `pause_at_ddl_commit_window`：`ARMED` 为 `None` 时 `as_mut()` 得 `None` ⇒ 立即返回；未布防时唯一额外动作是一次 `Mutex` 取锁（无 I/O、无状态变更、无返回值语义） |
| **路径限定的正确性** | **一致** | `Database::path()`（`src/database.rs:309`）返回 `open_internal` 里 `path.to_path_buf()` 的副本；测试布防路径与 `Database::open` 入参同源（`db_path(&dir)`，绝对路径）；内存会话的占位 `":memory:"` 不被任何构造步骤消费（`OpenMode::Memory` 裁剪 WAL / 恢复 / checkpoint 三段）⇒ 内存会话不会误命中，也不会被真实库路径命中 |
| **一次消费语义** | **一致** | `ARMED.lock().unwrap().as_mut()` 后 `armed.remove(path)` ⇒ 取走即从表中移除 ⇒ 一次消费；未布防分支不消耗任何东西 |
| **是否引入身份机制** | **一致（无）** | 无 hash / 指纹 / revision pin / run-id / session-execution ID / artifact manifest / 日志 Hash 链 / 时间顺序证明。路径只作**作用域限定**（与 `src/storage/free_page_rebuild.rs::arm_free_page_rebuild_fault` 同形），不是身份证明；`CommitWindowGate` 只含一个 `AtomicBool` 锚点 + 一个 `Notify` 放行信号 |
| 编译错误（borrow 层） | **已修** | 见下 |

**编译错误的修法**：`park_at_commit_window` 的形参 `statement: &mut Pin<Box<F>>` 是**不可变绑定** ⇒ `&mut statement` 报 `E0596: cannot borrow statement as mutable`。修法：形参改为 `mut statement: &mut Pin<Box<F>>` ⇒ `&mut statement` 的类型是 `&mut &mut Pin<Box<F>>`，而 `Pin<Box<F>>` 同时满足 `Future`（`Pin<P>: Future where P: DerefMut, P::Target: Future`）与 `Unpin`（`Box<F>` 恒 `Unpin`）⇒ `&mut &mut Pin<Box<F>>` 可被 `tokio::select!` 轮询。**备选 `statement.as_mut()` 不可行**：它给出 `Pin<&mut F>`，要求 `F: Unpin`（Plan 的泛型未加该约束）。

**另有一处与派单描述的差异（已核对，非缺陷）**：派单说在途工作只追加了 `ROWS_T10` 与 `park_at_commit_window` 夹具；实际工作区里**还包含 T11 的两条对照用例**（`read_after_narrowing_completes_returns_narrowed_rows`、`read_of_other_table_during_narrowing_is_unaffected`）。已逐条核对断言语义（与 Plan Context 的 T11 First test 一致），**未重做**，只重跑验证。

### T10 的先行测试 RED 证据（实际运行，真实错读）

```
$ cargo test --test ddl_rewrite_consistency_test -- --test-threads=1
running 9 tests
...
test read_during_narrowing_in_flight_is_refused ... FAILED
test read_after_narrowing_completes_returns_narrowed_rows ... ok
test read_of_other_table_during_narrowing_is_unaffected ... ok
...

---- read_during_narrowing_in_flight_is_refused stdout ----
thread 'read_during_narrowing_in_flight_is_refused' panicked at tests/ddl_rewrite_consistency_test.rs:841:46:
reading a table whose DROP COLUMN rewrite is in flight must be refused; the read returned misaligned rows instead: [[Int(1), Int(100), Null], [Int(2), Int(200), Null], [Int(3), Int(300), Null], [Int(4), Int(400), Null], [Int(5), Int(500), Null]]

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.53s
```

**错读形态的逐项核对**（表 `t (id INT PRIMARY KEY, v INT, w INT)`，第 `id` 行的值为 `(id, id*10, id*100)`；DROP 掉 `v` 后窄化行为 `(id, id*100)`）：

| 行 | 期望（`v`, `w`） | 窗口内实读 | 判读 |
|---|---|---|---|
| 1 | (10, 100) | `[Int(1), Int(100), Null]` | **`v` 列显示 `w` 的值（100 ≠ 10）** ✓ |
| 2 | (20, 200) | `[Int(2), Int(200), Null]` | 同上 ✓ |
| 3 | (30, 300) | `[Int(3), Int(300), Null]` | 同上 ✓ |
| 4 | (40, 400) | `[Int(4), Int(400), Null]` | 同上 ✓ |
| 5 | (50, 500) | `[Int(5), Int(500), Null]` | 同上 ✓ |

⇒ **`v` 列读到 `w` 的值、`w` 列读 NULL** —— 正是 ISS25 登记的错位形态；**不是**「读到旧宽行」、**不是**「读到空结果」、**不是**「读到明确错误」。⇒ **T10 的硬门通过，ISS25 的可达性坐实**（登记原文「未构造故障注入夹具、未实跑复现」已被实跑取代）。既有 6 条用例 + 两条对照用例在 RED 阶段全部通过（8 passed）⇒ 夹具未破坏既有行为。**未放宽断言**。

### T11 的修复形态与「显式拒绝」的用户实际所见

**修复形态**（四段）：

1. **状态载体（表级共享单元）** —— `src/storage/data/table_manager.rs`
   - `NarrowingState`（`:103`）：`table: String` + `in_flight: AtomicBool`；`ensure_readable()`（`:137`）在窗口内给出点名该表的拒绝。纯内存、不持久化、不进 WAL / 目录 ⇒ 磁盘格式不变。
   - `TableMeta.narrowing: Arc<NarrowingState>`（`:210`）：**同一张表的任何活布局版本共享同一单元**——8 个构造点中，6 处「由 `old` 重建活布局」的点用 `old.narrowing.clone()`（`alter_add_column`、`new_meta`、恢复期 PK 换索引、恢复期索引换树、注册二级索引、注销二级索引），2 处「新建表」的点（`open_or_init` 的目录恢复、`create_table`）用全新单元。⇒ 计划期快照（旧 `TableMeta`）与目录窄化后的新活布局**看到的是同一个单元** ⇒ 拒绝对计划期快照生效。
2. **发布（改写开始处）** —— `alter_drop_column`：`old.narrowing.publish()`（`:982`），位置在全部守卫检查通过之后、数据面改写循环之前。
3. **清除（目录窄化生效后）** —— `apply_column_drop`：`pending.new_meta.narrowing.clear()`（`:1232`），位置在「目录列行改写 + 列数更新 + 活布局切换」**之后**。
4. **异常 / 放弃路径兜底** —— `NarrowingGuard`（`:156`，`Drop` impl `:166`）随 `PendingColumnDrop` 的私有字段 `_narrowing_guard`（`:236`）活到语句结束；`Drop` 时清除 ⇒ **该表不会因失败的 / 被放弃的 DDL 而永久不可读**（Plan Context 要求 ③）。守卫在发布点内部创建 ⇒ 放弃路径不依赖执行器取到正确的单元。

**读路径的覆盖面**（9 处 `ensure_readable()` 覆盖 13 个解码点，全部在**解码之前**）：

| 执行器 | 检查位置 | 覆盖的解码点 |
|---|---|---|
| `src/executor/data_scan.rs:380` | `DataScanExecutor::next()` 入口（新增 `narrowing: Arc<NarrowingState>` 字段） | `:529`、`:576` |
| `src/executor/scan.rs:107` | `ScanExecutor::next()` 入口（`executed` 置位之前） | `:124`、`:139` |
| `src/executor/index_scan.rs:62` | `IndexScanExecutor::next()` 入口 | `:90`、`:105` |
| `src/executor/index_scan_all.rs:79` | `IndexScanAllExecutor::next()` 入口 | `:110`、`:127` |
| `src/executor/create_index.rs:187` | 索引回填的解码点前 | `:189` |
| `src/executor/delete.rs:86` | 唯一 / 二级键提取的解码点前 | `:87` |
| `src/executor/update.rs:140` | 旧行快照的解码点前 | `:147` |
| `src/executor/upsert.rs:353` / `:625` | DO UPDATE 旧行读取 / REPLACE 冲突行读取的解码点前 | `:355`、`:626` |
| `src/storage/data/table_manager.rs:1041` | — | **豁免**（改写自身读旧行、用旧 schema 解码，本就正确；一并拒绝会使改写无法进行） |

**「显式拒绝」的用户实际所见**（真实运行捕获，`SqlError::message` 全文）：

```
Execution error: table 't' is being modified by an in-flight ALTER TABLE DROP COLUMN; read refused
```

- `sqlstate` = `None`（`StorageError::ExecutionError` 无分类，与既有执行面错误同族）。
- `Execution error: ` 前缀由 pipeline 漏斗唯一赋予（`src/pipeline.rs:1702`），与 ISS19 的单层前缀纪律一致。
- 拒绝**点名该表**（`'t'`）并说明「正在被修改」⇒ 不是「查不到该行」、不是空结果、不是任何按位置取值的结果。

### 两条对照用例的作用与判别力

| 用例 | 作用（对应 Acceptance） | 判别力（若实现有偏则该用例失败） |
|---|---|---|
| `read_after_narrowing_completes_returns_narrowed_rows` | 改写完成后读路径**恢复正常**（Acceptance 2） | 若 `apply_column_drop` 的清除漏掉、或守卫的 `Drop` 不清除 ⇒ 该读会被拒绝（而非返回窄化后的行）⇒ 失败。断言：5 行、每行**恰好 2 列**、`(id, id*100)` 的**精确集合**比对 ⇒ 目录未窄化（3 列）也会失败 |
| `read_of_other_table_during_narrowing_is_unaffected` | **未改写的表读路径不受影响**（Acceptance 3）——拒绝面按**表**而非按库 / 全局 | 若在途状态做成库级 / 全局（或按 `Database` 而非按表）⇒ 同一窗口内读 `t2` 也会被拒绝 ⇒ 失败。断言：`t2` 正常返回 `[[Int(1), Int(11)]]` |

两条在 RED 阶段**即已通过**（8 passed）⇒ 它们不是「随实现一起变绿」的空洞断言，而是防误伤 / 防空洞通过的反向闸门。Acceptance 5 的另一半（T10 的 RED 必须是真实错位值）由上文 RED 输出直接证明。

### 崩溃-重开变体（Plan Context Risks 的实际观察）

按 Risks 的要求做了**两个一次性观察探针**（临时测试文件，跑完即删，**不留**在测试面；它们只产出观察，不参与任何验收判定）。

**探针 1 —— 崩溃-重开**（进程内崩溃模型：丢弃语句 future + 丢弃库句柄**不 close** ⇒ 脏页不落盘，重开走 WAL 恢复；先例 R49）：

```
重开后目录仍广告 3 列：PROBE_REOPEN_COLUMNS: [ColumnMeta { name: "id", … }, ColumnMeta { name: "v", … }, ColumnMeta { name: "w", … }]
重开后的读：      PROBE_REOPEN_ROWS: [[Int(1), Int(100), Null], [Int(2), Int(200), Null], [Int(3), Int(300), Null], [Int(4), Int(400), Null], [Int(5), Int(500), Null]]
```

⇒ **可复现，且重开后仍然错读**（`v` 读到 `w` 的值、`w` 读 NULL）⇒ **该变体不在 Iteration 003 的闭合范围内**：在途状态纯内存、崩溃即消失。闭合它需要**持久标记**或**移动提交点**（后者与 ISS21 / D5「被放弃 / 失败的 DDL 不留目录面变更」冲突）⇒ 真实的设计决定。**未自行扩大范围**。

**探针 2 —— 提交后放弃**（在窗口放弃语句、进程不重启）：

```
同会话再读：PROBE_ABANDON_ROWS: [[Int(1), Int(100), Null], [Int(2), Int(200), Null], [Int(3), Int(300), Null]]
```

⇒ 与崩溃变体**同族**（目录 3 列 + 2 宽已提交行）。守卫按 Plan Context 要求 ③「异常 / 放弃路径也清除」⇒ 该变体在清除后**不被拒绝**。**这与本 Cycle 之前的行为一致**（修复前同样错读）⇒ **不是本 Cycle 引入的回归**；但它证明「内存状态」只能覆盖**活着的窗口**，覆盖不了提交点之后的持久化不一致。

### 验证（实际命令与结果，最终工作区）

按 Plan Context 的 Verification，**全部 `--test-threads=1`、串行、逐条执行**；均为**定向**单目标调用，**未跑全量回归**（收尾全量按 change 预算归 **Iteration 007 / T21**）：

```
$ cargo test --test ddl_rewrite_consistency_test -- --test-threads=1
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.59s

$ cargo test --test ddl_catalog_transactional_test -- --test-threads=1
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.29s

$ cargo test --test ddl_crash_recovery_test -- --test-threads=1
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.68s

$ cargo test --lib -- --test-threads=1
test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 22.37s
```

- `cargo test --lib` 的 **420 基线仍成立**。本 Cycle 改到了 `table_manager.rs` / `alter_table.rs` / `database.rs` ⇒ Iteration 002 收尾时的 420 结论对本次改动**已失效** ⇒ 按 Plan Context 的 Verification **重跑**（不是沿用）。
- MS35 / MS36 的全量终态 2192/0/2 对当前工作区仍**失效**（预算归 Iteration 007），本 Cycle 未引用、未重跑。
- `cargo build --tests`：**0 warning**（含无新增 dead_code / unused 警告）。
- M19 多批次串行纪律：本 Cycle **不需要**全量批次 ⇒ 未使用批协议；上述 4 条均为单目标定向调用，批间顺序执行、同一时刻只有一条在跑、**无**一次无目标过滤的全量调用。

**既有结论的处置**：`tests/ddl_rewrite_consistency_test.rs` 因追加了夹具与用例，原「6 passed」结论**失效** ⇒ 已重跑为 9 passed；`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`release_path_bounded_read_test` 7/0 三条与本次改动面无交集，**未重跑**（覆盖范围未变化）。

### Gate 3（任务级 Review）

- **T10 —— PASS**
  - Spec compliance：真实文件库 + 真实 `Database::open` + 真实 `ALTER TABLE … DROP COLUMN` + 真实 `SELECT`；夹具形态 = 路径限定 + 一次消费 + 默认不生效；RED 形态 = **真实错位值**；既有 6 条用例行为不变；磁盘格式未改；表规模 5 ≪ 300（ISS23）。
  - Code quality：注入点默认不生效（未布防时零行为差异）；无新增警告；无身份机制 / 判定层 / timer 阈值判据（`ANCHOR_TIMEOUT` 只作夹具失控兜底，触发即 panic = 失败）；无计划外修改。
- **T11 —— PASS**
  - Spec compliance：①改写开始发布 ②目录窄化生效后清除 ③异常 / 放弃路径由守卫清除 ④任何用户可见读路径在解码前给出点名该表的明确拒绝（不返回按位置取值的结果、不表现为「查询成功但列值错位」、不静默返回空结果）⑤改写完成后恢复正常 ⑥未改写的表不受影响；13 个解码点全覆盖（改写自身读豁免）。
  - Preserve 逐条核对：`ADD COLUMN` 的窄行解码语义**未动**（`deserialize_row` 未改；`alter_add_column` 只共享状态单元，不改解码）；`deserialize_tuple_width` / `deserialize_row` 的解码契约**未改**；磁盘格式（`VersionHeader` / 目录行布局）**未改**；目录**列**定义的提交点时机**未改**（`apply_column_drop` 仍在提交点之后）；「被放弃 / 失败的 DDL 不留目录面变更」**未改**；Iteration 001 的单一尾游标 / 墓碑写序 / M2 活性判据**未改**；`free_page` / `complete_free_release` 的集合语义**未改**。
  - Code quality：状态所有权清晰（表级共享单元，与活布局同寿命）；资源生命周期正确（守卫 `Drop` 不会留下永久不可读的表）；无新增警告；无身份机制 / 判定层；错误路径给出明确文案而非降级为空结果。
- **T12（Verification）—— PASS**：4 条命令 0 failed（见上）。

### Gate 4（Verified Completion）

**PASS** —— Task Definition 的 Verification 全部满足，完成声明基于本次实际运行（RED 1 次 + GREEN 4 条命令 + 2 个观察探针），无沿用失效结论。

### 自检发现、已修复内容与遗留 Minor

- **发现 1（已修复，Important）**：`NarrowingGuard` 首次实现**遗漏 `Drop` impl** ⇒ 编译器报 `field state is never read` ⇒ 守卫实际**不清除**状态 ⇒ 异常 / 放弃路径会让该表**永久不可读**（比 ISS25 原缺陷更严重）。已补 `Drop` impl（`self.state.clear()`），重新编译 0 warning。
- **发现 2（已修复，Minor）**：`NarrowingState::is_in_flight` 起初为 `pub` 但仅 `ensure_readable` 使用 ⇒ 收窄为私有，去掉未使用的公共面。
- **发现 3（已修复，Minor）**：`TableManager::narrowing_state()` 访问器在实现过程中被证明不需要（守卫改到发布点内部创建）⇒ 删除，避免孤儿 API。
- **发现 4（已修复，Minor）**：`read_during_narrowing_in_flight_is_refused` 原先对 `Ok(SqlOutcome::Rows)` 分支用 `panic!` 展示错位行 —— 该形态**正确**（它是 RED 证据的载体），但 RED 阶段的失败信息必须包含真实读数 ⇒ 已确认 panic 文案含完整行数组（见 RED 输出）。未放宽断言。
- **遗留 Minor 1**：读路径的检查位于各执行器的 `next()` 入口 ⇒ `ScanExecutor` / `DataScanExecutor` 的**逐行** `next()` 调用每次多一次原子读（`SeqCst`）。对读热路径的量级影响可忽略（一次 relaxed 级原子读），但不是零成本；如后续需要可用 `Relaxed` + 单次取样优化。
- **遗留 Minor 2**：`DataScanExecutor` 现在多持有一个 `Arc<NarrowingState>`（每个扫描执行器多一次 clone / drop）；`TableMeta` 多一个 `Arc` 字段（每表 +8 字节 + 一次引用计数）。非阻塞。
- **遗留 Minor 3**：拒绝文案用 `table '{name}' is being modified by …`（单引号包裹表名），与既有错误文案的 `table not found: {name}` / `column '{column}' expects …` 两种风格都不完全一致（前者无引号、后者用引号）。非阻塞。

### Deviations from Plan

1. **前次在途工作的 borrow 层偏差已按 Plan Context 修正**：`park_at_commit_window` 形参改为 `mut statement: &mut Pin<Box<F>>`（原为不可变绑定 ⇒ `&mut statement` 报 E0596）。属等价修法，不改断言语义、不改夹具语义。
2. **守卫的载体形态**：Plan Context 把发布点定在 `alter_drop_column`、清除点定在 `apply_column_drop`，并要求「异常 / 放弃路径由语句作用域守卫兜底」。实现把守卫放进 `PendingColumnDrop` 的**私有字段** `_narrowing_guard`（而非在执行器侧另建守卫对象）⇒ 守卫与发布点同处一个作用域，异常 / 放弃路径的覆盖**不依赖执行器取到正确的单元**。Plan 的语义要求逐条满足，形态属等价实现。
3. **读路径检查的落点**：Plan 列出 13 个解码点；实现按「每个执行器入口一次检查」收拢为 9 处 `ensure_readable()`（覆盖同执行器内的 1–2 个解码点）⇒ 检查在解码之前，语义等价；改写自身读豁免。
4. **收尾全量的 Iteration 指针**：冻结的 Plan Context 里的指针已过时（change 的 Iteration Map 已调整为 000→007）⇒ 按当前实际的 **Iteration 007 / T21** 记录（与 Iteration 002 的 Plan Review 一致）。
5. **前次在途工作已含 T11 的两条对照用例**（派单只提到夹具）⇒ 未重做，只核对断言语义并重跑。
6. **两个一次性观察探针**（崩溃-重开 / 提交后放弃）是临时文件、跑完即删 ⇒ 不留在测试面。理由：Plan Context 的 Risks 要求 Act 记录**实际观察**，而该变体明确在 Non-goals 内 ⇒ 用一次性观察取证，不新增永久测试面。

### Persisted Evidence

`None required` —— Plan 的 `Persisted Evidence` = `none`；判别读数固化在测试代码中（随用例重跑）；RED / GREEN 的实际输出、命令与两个探针的观察输出已写入本 Response。

### change 状态同步

`tasks.md` 的 **T10 / T11 → `done`**；RTM 的 **DRC R3 行 → `Done`**（First Test 列补记实际用例名 + 两条对照用例名）；Iteration Map 的 **003 行 → 「Act 已完成（Review Result 待 Plan 填写）」**。**未触碰**全局 `tasks.md` / `SNAPSHOT.md` / 项目记忆 / `openspec/specs/`，未归档 change，未创建下一 Cycle 或下一 Iteration，未填写 `Plan Review`，未登记或修改任何 Issue。

### Experience Candidates

1. **Issue 候选（范围外实质缺陷）——「提交完成 → 目录应用」的持久化不一致**：该点**崩溃后重开**（探针 1）或该点**之后语句被放弃**（探针 2）后，目录仍是 3 列而数据面是 2 宽的已提交行 ⇒ 读**仍然错位**。证据：本 Response 的两个探针输出（`PROBE_REOPEN_COLUMNS` / `PROBE_REOPEN_ROWS` / `PROBE_ABANDON_ROWS`）。与修复前行为一致（本 Cycle 未引入，也未覆盖）⇒ 闭合需要**持久标记**或**移动提交点**（后者与 ISS21 / D5 冲突）⇒ 真实设计决定。承载建议：Plan 在收尾时新开 Iteration（或另开 change）承载；**Issue 的登记由 Recorder 执行**（Act 只报告）。
2. **既有 Issue 的现场更新建议（非新候选）**：`.agents/issues/ISS25-ddl-commit-point-positional-misread.md`（Status `open`）的登记原文「未构造故障注入夹具、未实跑复现」已过时 ⇒ 本 Cycle 已把可达性坐实并修复**活窗口**面（崩溃 / 放弃变体仍未闭合，见候选 1）。建议用户指示 Recorder 更新该 Issue 的现场描述。**本 Cycle 未登记 / 未修改任何 Issue**。
3. **Runbook 候选**：无（本次流程为一次性定向验证与一次性观察，无「已端到端验证成功且可重复或风险较高」的可复用操作路径需要沉淀；夹具手法已固化在 `tests/ddl_rewrite_consistency_test.rs` 的代码里）。

### 未解决问题与停止条件

- 未解决的 **Critical / Important** 问题：**无**。
- 未解决项：**崩溃 / 提交后放弃窗口的持久化不一致**（范围外实质缺陷，已作 Issue 候选 1 上报）；遗留 Minor 1 / 2 / 3（非阻塞，已记录）。
- 停止条件**均未触发**：T10 —— 夹具**稳定**停在「提交完成 → 目录应用」窗口（锚点命中 100%）；窗口内读到的是**错位值**（非旧宽行 / 空结果 / 明确错误）；未改磁盘格式。T11 —— 未改 `deserialize_tuple_width` 的解码契约；未用「行宽 ≠ schema 宽度」作拒绝条件；在途状态在异常 / 放弃路径上**可靠清除**（既有 5 条放弃 / 回滚用例 + 守卫的 `Drop` 共同钉住）。
- 本 Cycle 的 T10 / T11 全部完成，Gate 3 / Gate 4 通过 ⇒ 等待 Plan Review（`Review Result` 仍为 `pending`，由 Plan 填写）。

## Plan Review

- Review Result: accepted

**Findings**

按公共规则「实施反馈 Review」分类：先判是否阻塞本 Iteration 的既有 Acceptance，再判偏差原因（Plan 遗漏 / Plan 错误 / Act 偏离 / 现状变化 / 新证据）。下列核对均为**独立读码 + 独立定向复核**，不以 Act 自检代替。

1. **13 个用户可见解码点的覆盖 —— 完整，无漏检（非阻塞；Act 声明成立）**。独立枚举 `src/` 全部 `deserialize_row(` 调用点，逐点核对「检查是否在同一条调用路径上、严格早于解码、且判定对象是同一张表」：
   - `data_scan.rs:529`、`:576` ← 检查 `data_scan.rs:380`（`next()` 入口，`DataScanExecutor::new` 由 `table_meta.narrowing.clone()` 取单元）；`executor_trait.rs:21` 的 `next_batch` 默认实现逐行调 `next()` ⇒ 无旁路。
   - `scan.rs:124`、`:139` ← 检查 `scan.rs:107`（`next()` 入口，`executed` 置位之前）；`scan.rs` 的 `collect_keyless_rows()` 走 `DataScanExecutor`，自带检查。
   - `index_scan.rs:90`、`:105` ← 检查 `index_scan.rs:62`；`index_scan_all.rs:110`、`:127` ← 检查 `index_scan_all.rs:79`（均在 `next()` 入口、任何解码之前）。
   - `create_index.rs:189` ← 检查 `create_index.rs:187`（回填循环内、解码前）；该文件是唯一解码点，不存在「未走到检查就解码」的分支（被跳过的槽位不解码）。
   - `delete.rs:87` ← 检查 `delete.rs:86`；`update.rs:147` ← 检查 `update.rs:140`；`upsert.rs:355` ← 检查 `upsert.rs:353`；`upsert.rs:626` ← 检查 `upsert.rs:625`（四处均在解码前；`upsert.rs` / `delete.rs` / `update.rs` 各只有这一个解码点）。
   - 合计 **9 处 `ensure_readable()` 覆盖 13 个用户可见解码点**，全部严格早于解码。
   - 其余 `deserialize_row` 调用点不在用户可见读路径上：`src/storage/data/table_manager.rs:1041`（**改写自身的内部读**，豁免，见下）、`src/wal/recovery.rs:265`/`:298`（恢复期重建，进程启动面，在途状态本就不跨崩溃存活）、`src/transaction/manager.rs:730`（回滚清理）、`src/storage/page_format/tuple.rs`（单元测试）。⇒ **无「静默错读」缺口**。
2. **改写自身读的豁免严格限于改写路径（非阻塞；Act 声明成立）**。豁免的实现在 `src/storage/data/table_manager.rs:1041`——它读的是**尚未窄化的旧行**并用 `old_schema`（`alter_drop_column` 的 `old_schema`，3 列）解码，本就正确；该读**不在任何用户可见读路径上**（不在 `src/executor/` 的任一执行器内）。⇒ 豁免没有让任何用户可见解码点失去检查。
3. **拒绝语义：四条路径的清除点齐全、拒绝面不过宽（非阻塞；Act 声明成立）**。独立核对 `NarrowingGuard`（`table_manager.rs:156`/`:166`）的析构清除与四个出口：
   - **成功**：`apply_column_drop`（`:1232`）在「目录列行改写 + 列数更新 + 活布局切换」**之后**清除 ⇒ 正常 `DROP COLUMN` 之后读表成功（由 `read_after_narrowing_completes_returns_narrowed_rows` 与 `ddl_catalog_transactional_test` 3/0、`ddl_crash_recovery_test` 6/0、`cargo test --lib` 420/0 钉住）。
   - **失败（改写本身返回 Err）**：发布点 `:982` 与守卫创建点 `:985` 相邻（其间无可失败操作），守卫是 `alter_drop_column` 的局部变量 ⇒ 返回即析构 ⇒ 清除。
   - **失败（提交失败 / `apply_column_drop` 失败 / 语句被放弃 / panic）**：`pending`（携带 `_narrowing_guard`）随执行器 future 析构 ⇒ 守卫析构 ⇒ 清除。⇒ 该表**不会因失败的 DDL 而永久不可读**（Act 自检发现 1 修的正是这个洞）。
   - **不过宽**：状态单元按**表**创建（`open_or_init` 的每目录行一处 `NarrowingState::new(table_name)`（`:492`）、`create_table` 一处（`:656`）），6 处「由 `old` 重建活布局」的点用 `old.narrowing.clone()` ⇒ 跨表不共享 ⇒ 由 `read_of_other_table_during_narrowing_is_unaffected` 反向钉住。
4. **状态载体的并发正确性（非阻塞；Act 声明成立）**。`TableManager.tables: RwLock<HashMap<String, Arc<TableMeta>>>`（`:285`）⇒ 活布局以 `Arc<TableMeta>` 共享；`narrowing` 是 `TableMeta` 的 `Arc<NarrowingState>` 字段，同表新旧布局指向**同一**单元（`new_meta.narrowing = old.narrowing.clone()`，`:968`）⇒ 计划期快照（旧 `TableMeta`）与目录窄化后的新活布局看到同一状态 ⇒ 窗口内的拒绝对计划期快照生效。判定对象 = 该表自己的单元 ⇒ 一个表的窄化状态**不会**误伤另一个表。
5. **T10 的 RED 形态 —— 独立复核确认是「真实错读」（非阻塞；Act 声明成立）**。Act 的 RED 输出与当前测试代码**逐字吻合**（panic 站点 `tests/ddl_rewrite_consistency_test.rs:841`、文案 `…the read returned misaligned rows instead: {rows:?}`、行值 `[Int(1), Int(100), Null]…`）；断言未被放宽（`Ok(SqlOutcome::Rows { rows, .. })` 分支仍是 `panic!`，只接受 `Err`）。为独立确认「失败原因落在列值错位、而非旧宽行 / 空结果 / 报错」，我**另跑了一个一次性探针**（跑后已删除，工作区已核对与进入 Review 时逐字一致）：在真实窗口内取活布局（目录仍广告 `["id","v","w"]` 三列）与已提交行字节（18 字节 ⇒ 两宽），按**三列 schema** 解码得 `[Int(1), Int(100), Null]`，按两列 schema 解码得 `[Int(1), Int(100)]` ⇒ 与 Act 的 RED 读数逐行一致 ⇒ 错位形态坐实，ISS25 的可达性由「读码结论」升级为「已实跑复现」。
6. **规格增量 DRC R3 的准确性 —— 未把「崩溃后仍错读」写成已验收（非阻塞）**。`specs/ddl-rewrite-consistency/spec.md` 的 R3「改写在途期间读路径不静默错读」的正文与四个 Scenario 全部限定在**进程内的在途窗口**（GIVEN 改写已写出窄化行而目录尚未窄化），**没有任何一条声称崩溃 / 重开后的行为已修复** ⇒ 规格陈述与已实现行为一致，不构成准确性问题。四个 Scenario 的验收面：Scenario 1 ← `read_during_narrowing_in_flight_is_refused`；Scenario 2 ← `read_after_narrowing_completes_returns_narrowed_rows`；Scenario 3 ← `read_of_other_table_during_narrowing_is_unaffected`；Scenario 4（「窄行与宽行的解码语义不变」）← `src/storage/page_format/tuple.rs` **未被本次改动触碰**（不在 `git status` 内），其既有单元测试 `width_aware_narrow_row_pads_null_tail`（`:571`）与 `overlay_defaults_fills_missing_tail`（`:645`）随 `cargo test --lib` 420/0 通过，另有 `tests/alter_table_test.rs` 的 ADD COLUMN 端到端断言（既有行读 NULL / 声明 DEFAULT，含重开后）。**Plan Context 把 Scenario 4 映射到「既有 `tests/ddl_rewrite_consistency_test.rs`」不准确**（该文件的 6 条用例全是 DROP COLUMN 面）⇒ 记为非阻塞 Minor finding（映射指针不精确，实际覆盖在 `--lib` 与 `tests/alter_table_test.rs`）。
7. **范围外实质缺陷 —— 提交点窗口的持久化不一致（新证据；Act 已上报，本 Review 独立确认）**。Act 的两个一次性探针的结论成立，且可由代码独立推出：在途状态是**纯内存**（`NarrowingState` 不进 WAL / 目录），`apply_column_drop`（`:1215`）才把活布局切到 `new_meta` 并窄化目录 ⇒ 若在「提交完成 → 目录应用」之间**崩溃**（活布局切换前）或该点之后**语句被放弃**（`pending` 析构 ⇒ 守卫清除 ⇒ 拒绝消失），重开后的活布局仍取自目录列行（旧列数）而数据面已是两宽的已提交行 ⇒ 读仍按位置错位。⇒ **ISS25 只闭合了两个触发变体中的一个**（见「Follow-up Decision」的 ISS25 处置）。本形态**不是本 Cycle 引入的**（修复前同样错读），且闭合它需要**持久标记**（改磁盘格式 ⇒ 破坏 MS36 已验收的 G1/G2/G3 兼容面）或**移动提交点**（与 ISS21 / D5「被放弃 / 失败的 DDL 不留目录面变更」冲突）⇒ 属新的设计决定，不在本 change 范围 ⇒ **作为 Issue 候选上报，只报告不登记**。
8. **design.md D7 的理由句与实测证据矛盾（Plan 错误 / 新证据）⇒ 已就地更正**。D7 的「改写窗口的边界」段称崩溃后的窗口「由既有崩溃模型承担 … 其一致性由 ISS22/ISS24 的修复（链一致 + 不隐藏行）承担」——该句被 finding 7 的证据证否：ISS22 / ISS24 的修复承担的是链一致与行可见性，**不承担**「目录列数与数据面行宽一致」在提交点窗口内的保证。**决策**（在途状态只在内存、不引入新的持久化状态、不扩大崩溃模型）不变，**仅更正承担者的事实陈述**（先例：MS34 批次 Plan Review 对 design D5 的同类就地更正）。
9. **tasks.md Iteration 003 的 Non-goals 理由句同上（Plan 错误 / 新证据）⇒ 已就地更正**。原文「崩溃后在途窗口的持久化（由既有崩溃模型与 Iteration 001 的成果承担）」与 finding 7 / 8 同源；**范围决定**（列为 Non-goal）不变，仅更正理由并指向本 Review 的 Issue 候选。
10. **非阻塞 Minor finding（记录，不返工）**：
    - (a) `ScanExecutor` / `DataScanExecutor` 的逐行 `next()` 每次多一次 `SeqCst` 原子读；`DataScanExecutor` 多持一个 `Arc`；`TableMeta` 多一个 `Arc` 字段——量级影响可忽略。
    - (b) 拒绝文案 `table 't' is being modified by …` 的引号风格与既有两类错误文案不完全一致。
    - (c) **并发同表 DDL 的交互**（本 Cycle 未引入，也未修复）：在途状态是**单表一个布尔量**，同表上并发的第二个 DDL 若先完成并清除状态，会让第一个仍在窗口内的读失去拒绝；同时第二个 DDL 的**改写自身读**（豁免路径）会把已被第一个 DDL 窄化的行按旧 schema 解码。同表并发 DDL 在本改动之前即不安全（两条改写争同一条数据链与目录），本 Cycle 不扩大该面；该交互需要「同表 DDL 串行化」这一新设计决定 ⇒ 记录为非阻塞 finding + 范围外 Issue 候选，不在本 Iteration 处理。
    - (d) `plan_at_commit_window` 的 `mut statement: &mut Pin<Box<F>>` 修法（borrow 层）等价实现——不阻塞。

**Acceptance Gaps**

无。Acceptance 1–6 逐条核对：1（窗口内明确拒绝，点名该表；不返回按位置取值的结果、不表现为「查询成功但列值错位」）✓ 由 `read_during_narrowing_in_flight_is_refused` + 我独立复核的窗口内探针（活布局三列 / 行两宽 / 三列解码得 `[Int(1), Int(100), Null]`）钉住；2（改写完成后恢复正常，2 列且值精确匹配）✓ `read_after_narrowing_completes_returns_narrowed_rows`；3（未改写的表不受影响）✓ `read_of_other_table_during_narrowing_is_unaffected`；4（窄行解码语义不变）✓ `tuple.rs` 未触碰 + `--lib` 两条既有单元测试 + `tests/alter_table_test.rs`；5（判别力：RED 是真实错位值；夹具真实停在窗口；两条对照用例在 RED 阶段即通过 ⇒ 反向闸门）✓；6（既有回归零破坏）✓（定向 3 条 + `--lib` 420/0，`cargo build --tests` 0 警告；全量按预算归 Iteration 007 / T21，未跑，符合 Plan Context）。

**Evidence**

- **本 Review 的独立定向复核**（Plan 补跑 Act 未覆盖的检查；覆盖范围未变化，Act 其余结论直接沿用）：
  - `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` ⇒ `test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`（本次实际运行）。
  - 一次性探针（`tests/zz_tmp_review_probe_iss25.rs`，**跑后已删除**）：窗口内 `PROBE_LIVE_COLUMNS: ["id","v","w"]`、`PROBE_TUPLE_LEN: 18`、`PROBE_DECODED_AS_3COL: [Int(1), Int(100), Null]`、`PROBE_DECODED_AS_2COL: [Int(1), Int(100)]`、`PROBE_READ_OUTCOME: ERR Execution error: table 't' is being modified by an in-flight ALTER TABLE DROP COLUMN; read refused`。探针只产出观察、不参与任何验收判定；结束后 `git status --porcelain` 与进入 Review 时的快照**逐行相同**（工作区已恢复）。
- **Act 报告的验证结论（沿用，来源：Act Response「验证」段，覆盖范围未变化）**：`ddl_rewrite_consistency_test` 9/0、`ddl_catalog_transactional_test` 3/0、`ddl_crash_recovery_test` 6/0、`cargo test --lib` 420/0、`cargo build --tests` 0 警告；全量未跑（预算归 Iteration 007 / T21）。
- **代码位点（本 Review 独立读码）**：`src/storage/data/table_manager.rs:91-173`（`NarrowingState` / `NarrowingGuard` + `Drop`）、`:204-210`（`TableMeta.narrowing`）、`:233-236`（`PendingColumnDrop._narrowing_guard`）、`:492`/`:656`/`:810`/`:968`/`:1374`/`:1421`/`:1485`/`:1546`（8 个构造点）、`:982`（发布）/`:985`（守卫创建）/`:1197`（守卫随 pending 移交）/`:1232`（清除）；`src/storage/data/table_manager.rs:285`（`tables: RwLock<HashMap<String, Arc<TableMeta>>>`）、`:1041`（改写自身读，豁免）；`src/executor/{data_scan:380, scan:107, index_scan:62, index_scan_all:79, create_index:187, delete:86, update:140, upsert:353, upsert:625}`（9 处检查）；`src/executor/alter_table.rs:113-135`（提交 → 窗口 → 目录应用）；`src/executor/ddl_commit_window.rs`（路径限定 + 一次消费 + 默认不生效）；`src/database.rs:309`（`Database::path()`）。
- **身份证明机制与判定层核对**：夹具只含 `AtomicBool` 锚点 + `Notify` 放行信号 + 按路径索引的静态表 ⇒ 无 hash / 指纹 / revision pin / run-id / session-execution ID / manifest / 时间顺序证明；路径只作**作用域限定**（先例 `src/storage/free_page_rebuild.rs::arm_free_page_rebuild_fault`），不作身份判定；判定全部在 `cargo test` 内（cargo 的退出码与原生输出）；`ANCHOR_TIMEOUT`（`tests/ddl_rewrite_consistency_test.rs:47`）只在夹具失控时 panic ⇒ 判失败，**不作行为判据**。无框架外判定脚本 / 封装命令 / 专用参数。
- **规格增量核对**：`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/specs/ddl-rewrite-consistency/spec.md:59-87`（R3 及其四个 Scenario）。

**Follow-up Decision**

- **Accept**：Acceptance 1–6 全部满足且无阻塞项；实现与 Plan Context 的语义要求逐条对应（发布点、清除点、异常 / 放弃兜底、13 个解码点全覆盖、改写自身读豁免、拒绝按表）；实现无身份证明机制、无框架外判定层、无 timer 阈值判据；磁盘格式与 `ADD COLUMN` 的窄行解码语义未动。⇒ `Review Result: accepted`，Iteration 003 完成。
- **无需当前 Cycle 修复**：finding 1–6 与 10 全为非阻塞（记录即可）；finding 7–9 是**范围外**的实质缺陷与**文档事实陈述**的不准确，已分别按「Issue 候选（只报告不登记）」与「design / tasks 理由句就地更正」处置，不引入新的执行依据、不需要 Act 返工。
- **ISS25 的处置（供收尾时 Recorder 使用）**：**不得关闭，状态保持 `open`**。ISS25 的登记触发条件含两个变体：(a) 窗口内的**并发读**——**已由 Iteration 003 闭合**（在途窄化状态对读路径可见 ⇒ 点名该表的明确拒绝，不再返回按位置错位值）；(b) 窗口内的**崩溃后重开并查询**——**仍未闭合**（在途状态纯内存，崩溃即消失；重开后目录仍广告旧列数而数据面已是两宽的已提交行 ⇒ 读仍错位）。⇒ Recorder 应把 ISS25 的现场描述从「未构造故障注入夹具、未实跑复现」更正为「可达性已实跑坐实；并发读变体已修复并有常态化用例；崩溃-重开变体仍开放」，并**保持 `open`**，同时登记本 Review 的 Issue 候选（关闭 (b) 需新的设计决定：持久标记会改磁盘格式并破坏 MS36 的 G1/G2/G3 兼容面；移动提交点与 ISS21 / D5 冲突）。
- **Iteration Plan 不变**：Iteration 003 的 Plan（`tasks.md` 的 Iteration Plan 段）不变；本 Iteration 依 `accepted` 完成。

**Iteration Plan Update**

无（Iteration 003 的目标、范围、依赖、requirement、设计、验证方式与验收边界均未改变；仅 design.md D7 与 `tasks.md` Iteration 003 Non-goals 的**理由陈述**就地更正，范围决定不变）。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/004-release-durability-precondition/000-initial.md`
