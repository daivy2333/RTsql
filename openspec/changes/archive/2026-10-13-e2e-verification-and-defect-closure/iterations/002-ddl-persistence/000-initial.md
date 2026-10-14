# Iteration 002 / Cycle 000: DDL 持久化与原子性

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R3（`wal-recovery-replay-integrity` ADDED + `sql-write-surface` ADDED）；design D4/D5/D6；用户决策 D-2（DDL 成功路径有序落盘，存量已坏库不救，`check` 指引文案不改）
- Excluded scope: DROP TABLE / DROP INDEX / DROP VIEW / TRUNCATE / ALTER DROP COLUMN 的崩溃窗口；WAL DDL 记录；存量砖库修复；Iteration 003 的 planner 读面；Iteration 004 的文案

**Objective**

DDL 成功返回前，catalog 效果按「被引用页先落盘、`__tables` 行最后落盘」写入。此后未 checkpoint 的崩溃，重开仍能找到表。建表中途失败没有半表，同名可以再建。DEFAULT 字面量超过 catalog 列行页容量时，计划期点名拒绝，不写任何表。e2e 的 E3b、ISS14、ISS08 三处 ignore 解除后转绿。

**Background**

Iteration 001 已 accepted。索引等值查询与全表 DML 不再占用本轮。本轮只处理建表与其它非 DROP 的 DDL 在崩溃和失败时的持久化。

**Investigation Facts**

- Current Baseline: Iteration 001 Act Response（2026-10-13）全量 1950 passed / 0 failed / 11 ignored。其中 9 个 ignore 仍是 E3b / ISS07 / ISS09 / ISS10–13 / ISS14 / ISS08，留给 Iteration 002–004。产品代码里的索引修复保持不动。
- Current-State Evidence:
  - `TableManager::create_table_with_constraints`（table_manager.rs:398）先分配数据页并 `SlottedPage::init` 成 0x03，再建主键和 UNIQUE 索引，然后在写 catalog 之前把 `TableMeta` 放进内存 `tables`（:518-525）。`Catalog::insert_table`（catalog.rs:218）先 `append_to_chain` 写 `__tables` 行（:224），再逐列写 `__columns`（:231-236）。列行失败时内存表会被摘掉（:563-567），已经写下的表行、列行和分配的页没有清理。表行因此先于它引用的列行成为磁盘上的可见定义。
  - `append_to_chain`（catalog.rs:598）返回 `(PageId, u16)`，`insert_table` 目前丢掉这个页号。
  - `alter_add_column`（table_manager.rs:592）先 `insert_column_row`，再 `update_table_column_count`，成功后才替换内存 `TableMeta`。列行成功、表行计数更新失败时，列行留在 `__columns`，内存表不变。
  - `register_secondary_index`（table_manager.rs:1145）经 `append_secondary_index` 更新 `__tables` 副本，然后才改内存。`Catalog::insert_view_row`（catalog.rs:309）直接调用 `insert_table`，写序与建表相同。
  - `BufferPool::flush_all`（buffer_pool.rs:216）按 DashMap 分片顺序写出全部脏页，并先清 dirty 位。没有按指定页序冲刷的接口，也没有「先写其余页、再写 commit 页」的接口。
  - `validate_default_value`（ddl_dml.rs:59）只做类型校验和 FLOAT 升格、日期解析。不看 `serialize_catalog_column_row` 之后的字节长度。`build_create_table`（ddl_dml.rs:1056）和 `build_alter_table`（ddl_dml.rs:1290）在类型校验之后没有页容量预检。
  - e2e：`e3b_auto_create_crash_recovery`、`e13_oversized_default_named_rejection`、`e13_create_failure_zero_residual` 仍带 `#[ignore]`，分别标 ISS06 / ISS14 / ISS08。超限夹具是 5000 个字符的 STRING DEFAULT。ISS14 断言错误正文（`; statement:` 之前）含 `DEFAULT`，退出码 3。
- Code and Critical Path: `src/storage/catalog.rs` 的 `insert_table` / `insert_column_row` / `insert_view_row` / `append_secondary_index`；`src/storage/data/table_manager.rs` 的 `create_table_with_constraints` / `alter_add_column` / `register_secondary_index`；`src/storage/buffer_pool.rs` 的冲刷；`src/parser/planner/ddl_dml.rs` 的 DEFAULT 预检。CTAS 与 import 自动建表走 `create_table_with_constraints`，不另开写序。

**Implementation Guidance**

列行全部写入之后再写 `__tables` 行。表行所在页是 commit point。成功返回前先 `flush_all_except` 这些 commit 页，再 `flush_pages` 按顺序写 commit 页。`flush_all` 现有的「先收集、再写出、写出时不持 DashMap 锁」保持。

建表失败时：不要留下内存表；删掉已经写入的列行；释放本次分配的数据页和索引页。同名 `CREATE TABLE` 可以再执行。

DEFAULT 预检放在 `validate_default_value` 通过之后。用 `serialize_catalog_column_row` 的长度对照空列页还能放下的槽位容量。超出则计划期拒绝，文案点名 DEFAULT 和列名。ALTER ADD COLUMN 用同一判断。具体阈值从列行序列化和页空闲空间算出，不要写死一个与页格式无关的常数。

**Task Contracts**

### T3: DDL 有序落盘 + 建表写序 + DEFAULT 预检（R3）

- Requirement/Scenario: R3 / S7–S9 + E3b、E13 的 ISS14 与 ISS08
- Depends on: T1（ignore 入口）；Iteration 001 已 accepted，本任务不改 btree
- Targets: `src/storage/buffer_pool.rs` 的冲刷原语；`src/storage/catalog.rs::insert_table`；`src/storage/data/table_manager.rs::create_table_with_constraints`、`alter_add_column`、`register_secondary_index`；`src/storage/catalog.rs::insert_view_row`；`src/parser/planner/ddl_dml.rs` 的 `build_create_table` 与 `build_alter_table`
- Current behavior: `__tables` 行先于列行写入。崩溃发生在建表成功返回之前时，重开可能看见没有列、或者列不完整的表，后续命令失败。超长 DEFAULT 在执行期把页写满，并留下半表。内存表在 catalog 失败时会摘掉，已写的 catalog 行和分配的页还在。
- Required behavior: CREATE TABLE、CREATE INDEX、CREATE VIEW、ALTER ADD COLUMN、CTAS 建表段、import 自动建表，在成功返回前把被引用页先落盘，`__tables` 行（或它的更新副本）最后落盘。此后杀掉进程再读写打开，表、索引、视图或新列仍在，`check` 对这条路径不报结构损坏。建表在列行阶段失败时，内存没有这张表，catalog 没有可用的半表，同名语句可以再建。DEFAULT 序列化后放不进一页列行时，计划期拒绝，正文点名 DEFAULT，不分配这张表。
- Preserve: DROP 类 DDL 仍走现有 checkpoint 边界；不写 DDL 的 WAL 记录；`check` 的指引文案不改；K05 打开失败仍传播；0x03 首数据页初始化保持；索引下探、叶链和 `scan_all` 不改。
- Forbidden: 为存量已坏库做修复工具；改页格式；改 Iteration 003/004 的文件；把超限拒绝留到执行期 `Page full`。
- First test: 先解除 `e13_oversized_default_named_rejection` 的 ignore，运行 `cargo test --test e2e_test e13_oversized_default_named_rejection -- --ignored --test-threads=1`，预期失败（现在是执行期页满或半表，而不是计划期点名 DEFAULT）。再解除 `e13_create_failure_zero_residual` 与 `e3b_auto_create_crash_recovery` 的 ignore，确认当前为红。不要改这三则夹具的 SQL。另写崩溃矩阵：import 自动建表后、以及 CREATE TABLE / CREATE INDEX / CREATE VIEW / ALTER ADD COLUMN / CTAS 各自成功后立即杀掉进程，再打开，对象仍在且 `check` 退出码 0。已有表上的普通写入崩溃恢复保持原样。
- GREEN condition: 上述 e2e 三则去掉 ignore 后通过。崩溃矩阵通过。DEFAULT 预检覆盖字符串超限、未超限的边界值、点名文案、失败后 `schema` 无表且同名再建成功。`checkpoint_skip`、`check_command`、catalog durability 与既有 `recovery_*` 单目标 0 failed。
- Verification: `cargo test --test e2e_test -- --test-threads=1`；新崩溃测试与 DEFAULT 测试的单目标；定向 `checkpoint_skip_test`、`check_command_test`、catalog 与 recovery 相关集成目标。全量按 M19 分批一次，各批 0 failed。本 Iteration 的全量预算用在这里。
- Stop when: 列行先写、表行最后写之后，崩溃仍会打开失败（说明 commit 页不是表行页，或还有一条 DDL 路径没冲刷）；预检阈值把现有合法 DEFAULT 拒绝掉。出现这些情况时停，带回 Plan，不要放宽 `check` 或跳过恢复。

**Invariants**

- 冲刷时不在持有 DashMap 分片锁或页锁期间 await。先收集页，再写。
- 先看到 e2e ignore 用例失败，再改写序和预检。

**Non-goals**

见 Excluded scope。

**Acceptance**

- R3：S7/S8 由崩溃矩阵和 E3b 覆盖；S9 由 DEFAULT 预检、零残留测试和 E13 两则覆盖。
- RTM：R3 → T3 → 上述测试（Covered）。

**Verification**

崩溃用例从真实进程杀死后再启动 CLI 或 lib 打开，断言退出码、`check` 输出和表是否存在。预检断言计划错误正文。不另做判定层。

**Gate 2 Readiness**

PASS——T3 与 design D4/D5/D6 已在 2026-10-13 的变更计划里获批。本文件只把调查落到当前符号：表行先写、没有按序冲刷、预检不看序列化长度。无 TBD。页容量的具体字节数由序列化结果和空页槽位决定，不改变拒绝语义。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- `e13_create_failure_zero_residual` 与超限预检用的是同一条 5000 字符 DEFAULT。预检生效后，这则用例看到的是计划期拒绝，而不是执行期页满。零残留仍然成立：没有表、同名可以再建。
- `append_secondary_index` 若是追加一份新的 `__tables` 行而不是改列行，commit 页就是那份新表行所在页。索引根页要在它之前落盘。

## Act Response

- Status: reported

**Changes**

- `BufferPool` 增两原语（D4）：`flush_pages(&[PageId])`（按序冲刷指定脏页、跳非脏页）与 `flush_all_except(&HashSet<PageId>)`——均沿 M17 collect-then-write 纪律（帧锁只用于拷贝+清脏位，不跨 await）。
- `Catalog::insert_table` 写序重排（D5）：列行全部先写、`__tables` 行最后写（commit point）；表行落新链页时前导 tail 页的 link 字节并入 commit set；成功后 `flush_commit_point`（引用页先、commit 页后）。`insert_view_row` 经此继承（CREATE VIEW 接线）。
- `Catalog::append_secondary_index`（CREATE INDEX 登记接线）：`update_field_in_chain` 改返回落点页号（`Result<Option<PageId>>`，等长原地/变长新副本两路均返回），登记副本页最后落盘。DROP 类（`remove_secondary_index`、`alter_drop_column` 用的 `update_table_column_count`）不接线，保持 checkpoint-based 边界。
- 新增 `Catalog::update_table_column_count_ddl`（ALTER ADD COLUMN 接线）：count 重写页（等长原地）最后落盘。
- `TableManager::create_table_with_constraints` 失败清理（D5）：内存摘除 + `delete_table` 清理已写列行（TABLES 臂 no-op）+ 释放本次分配的数据页与 PK/UNIQUE 根页（create_index.rs 先例，best-effort）→ 零残留、同名可重试。
- `TableManager::open_or_init` 按表行 `column_count` 裁剪扫描到的列行（D5 孤儿列行不可见——ALTER 冲刷中段崩溃窗的形态）。
- `Catalog::catalog_column_row_fits`（D6）：列行序列化后对 freshly-init 空页做同型 slot 落位探针——阈值由真实页格式导出，无硬编码常数。
- `build_create_table` / `build_alter_table`（ADD 臂）在 `validate_default_value` 归一后接预检：`to_schema_column` 同款转换构造探针行，不fit 即 `DefaultTypeMismatch` 点名（DEFAULT + 列名），零副作用。ALTER 探针的 `column_index` 取 `column_list.len()`。
- e2e 三则解除 ignore（`e3b_auto_create_crash_recovery` / `e13_oversized_default_named_rejection` / `e13_create_failure_zero_residual`），夹具 SQL 未动。
- 新增 `tests/ddl_crash_recovery_test.rs`（6 例：create/index/view/alter-add/ctas/既有表写入对照；崩溃模拟 = WAL shutdown + drop 不 close，夹具纪律沿 recovery_replay_convergence_test 先例）与 `tests/default_precheck_test.rs`（5 例：超限点名 + 零副作用同名重试 / 3900B 边界放行 / 全类型小字面量 / ALTER 同款两例）。

**Deviations from Plan**

三处既有用例按新语义校准（原夹具编码「DDL 只留脏页、靠 close/checkpoint 落盘」旧模型，T3/D4 后该前提消除；规格不变量均保持，校准点逐条记录于各文件注释）：

1. `tests/checkpoint_skip_test.rs` S4 更名 `ddl_only_close_is_zero_work_and_schema_survives`：DDL-only close 从零工作谓词输入状态看**合法跳过**（谓词 `write_count==0 && !has_dirty_pages()` 逐字节未变，`wal-auto-checkpoint`「零工作跳过保持」场景保持）；schema 持久性改由语句边界落盘承载（本 Cycle 矩阵锁定）；表存活断言保留。
2. `tests/wal_auto_checkpoint_test.rs` `pure_select_session_writes_nothing`：规格不变量「纯 SELECT 零 WAL」改为 SELECT 阶段严格断言；空事务尾由阈值 1 自动 checkpoint 收敛，close 对收敛态不追加（长度不变断言）。
3. `tests/recovery_error_surface_test.rs` S22/S23 夹具改 `craft_legacy_brick` 手工构造存量砖形（WAL shutdown+drop 后空 0x03 页重置 `__tables`）——K05 定向错误面（delta R3「K05 SHALL 保持不变」+ D-2 存量已坏库保持拒绝）所需「catalog 缺表 + WAL 引用该表」状态在 T3 后仅存量砖库/冲刷中段 µs 窗可达，手工夹具承载同一错误面。

**Self-Review**

- Gate 3：D4 接线面与 design 清单逐一致（create/INDEX/VIEW/ALTER ADD 接线，CTAS/import 经 create 继承，DROP 类不接线）；写序不变量（列行先、表行最后）在 `insert_table` 单点落实；冲刷纪律（不持 DashMap 分片锁/页锁跨 await）两原语均满足；预检阈值由页格式导出。 Forbidden 面无触碰（无页格式改动、无存量修复工具、无 WAL DDL 记录、`check` 指引文案未动、K05 传播保持——S22 校准用例实证）。
- 遗留 Minor（非阻塞）：`tests/secondary_index_equality_test.rs:26` clippy `match`→`if let` 提示为 Iteration 001 存量（T2 已 accepted 范围），本 Cycle 不扩散处理。

**Verification**

- pre-fix RED 见证：e2e 三则解除 ignore 后 RED（Page full / 半表残留 / WAL redo 缺表硬失败）；崩溃矩阵 5 例 RED（表丢失 / WAL redo 失败），ctas 与对照例 GREEN（既有先例面）。
- `cargo test --test e2e_test -- --test-threads=1`：16 passed / 0 failed / 6 ignored（70.12s；余 6 ignore 属 ISS07/ISS09/ISS10–13，归 Iteration 003/004）。
- 新目标：`ddl_crash_recovery_test` 6/6（0.73s）、`default_precheck_test` 5/5（0.55s）。
- 定向：`checkpoint_skip_test` 6/6、`check_command_test` 8/8、`catalog_durability_test`、`wal_auto_checkpoint_test` 11/11、`recovery_*` 六目标（replay_convergence 3、wal_convergence 5、tail_persistence、e2e 6、error_surface 3〔校准后〕、recovery_test）、`wal_*` 五目标、`full_table_dml_test` 11/11、`secondary_index_equality_test` 3/3、`secondary_index_test` 27/27、`alter_table_test`、`view_test` 15/15、`truncate_test`、`ctas_test` 17/17、`import_input_surface_test` 12/12、`default_declaration_test` 14/14、`schema_persistence_test`、`memory_db_test` 15/15、`readonly_session_test` 15/15、`drop_table_free_test`、`gc_test`、`mvcc_tombstone_visibility_test`、`version_chain_test`、`checkpoint_command_test`、`checkpoint_redo_reduction_test` 9/9、`busy_timeout_test` 等——全部 0 failed。
- 全量（M19，本 Iteration 预算一次）：`tests/*.rs` 名字典序 122 个集成目标 13 批（每批 10）全部 0 failed，合计 1566 passed；`--lib` 398 passed / 0 failed；`--bins` 0 passed / 0 failed。
- `cargo fmt` + `cargo clippy --all-targets`：本 Cycle 触及面无警告（存量 1 条见 Self-Review）。

**Persisted Evidence**

None required——上述命令均可复跑，崩溃矩阵自含现场构造。

**Experience Candidates**

None

## Plan Review

- Review Result: accepted

**Findings**

独立读了 `BufferPool::flush_pages` / `flush_all_except`、`Catalog::insert_table` / `flush_commit_point` / `update_table_column_count_ddl` / `append_secondary_index`、`TableManager::create_table_with_constraints` 的失败清理、`open_or_init` 的列裁剪、`catalog_column_row_fits`，以及 `build_create_table` / `build_alter_table` 的预检。列行先写，`__tables` 行最后写。新链页时提交序是新页然后前一页的链接，链接最后落盘。冲刷先收集再写，不在持有 DashMap 分片锁或页锁时 await。CREATE VIEW 走 `insert_view_row` → `insert_table`。CREATE INDEX 与 ALTER ADD COLUMN 的表行页最后冲刷。ALTER DROP COLUMN 仍调用不带冲刷的 `update_table_column_count`。建表失败会摘掉内存表、删列行、释放本次数据页和索引根页。预检用空页 `add_slot` 探测，拒绝文案含列名和 DEFAULT。三则 e2e 已去掉 ignore；余下 6 个 ignore 是 ISS07、ISS09、ISS10–ISS13。

三处既有用例的校准与 Act 声明一致：`ddl_only_close_is_zero_work_and_schema_survives` 仍断言两张表在重开后存在，零工作谓词没有改；`pure_select_session_writes_nothing` 仍在 SELECT 阶段断言 WAL 长度不变；`craft_legacy_brick` 仍走缺表打开失败。`e13_create_failure_zero_residual` 走的是计划期拒绝，零残留和同名重建断言还在。这是计划里已经写明的风险，不是缺口。

非阻塞：

- `append_secondary_index` 在变长更新时先删旧表行，再 `flush_all_except` 把这次删除写盘，最后才写新副本页。语句返回之后杀进程是安全的，验收测的是这条路径。冲刷序列内部如果被杀掉，已有表行可能先从磁盘消失。design D4 把这个窗口限定在冲刷中段。
- `open_or_init` 按 `column_count` 截断，能藏住 ALTER 崩溃留下的更高 `column_index` 孤儿列。同名、同序号的列行截断藏不住。当前 S7–S9 不覆盖「冲刷中段崩溃后再用不同列定义重建同名表」。

**Acceptance Gaps**

无。R3 的 S7/S8 由 `ddl_crash_recovery_test` 六例和 E3b 覆盖，S9 由 `default_precheck_test`、E13 两则和失败清理路径覆盖。DROP 类没有接上 `flush_commit_point`。

**Evidence**

`buffer_pool.rs` 239–287、`catalog.rs` 226–273 与 409–430 与 578–603 与 1363–1368、`table_manager.rs` 262–273 与 568–592 与 639–647 与 798、`ddl_dml.rs` 1161–1174 与 1458–1461。`e3b_auto_create_crash_recovery`、`e13_oversized_default_named_rejection`、`e13_create_failure_zero_residual` 无 ignore。验证采信 Iteration 002 Act Response：e2e 16 passed / 6 ignored；`ddl_crash_recovery_test` 6/6；`default_precheck_test` 5/5；定向目标 0 failed；全量集成 1566 passed、库测试 398 passed、二进制 0 failed。Persisted Evidence 为 none，不另建目录。本次审查没有重跑这些命令。

**Follow-up Decision**

验收满足。两处冲刷中段窗口留在记录里，不返工。Iteration 002 完成。按原 Map 展开 Iteration 003。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/003-planner-read-surface/000-initial.md`（已展开，Plan Context ready）
