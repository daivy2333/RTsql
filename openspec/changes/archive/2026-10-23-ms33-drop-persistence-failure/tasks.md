# Tasks: DROP 与受控持久化失败

来源：proposal；设计 D1、D2。里程碑 MS33（I086）。

> Status: 计划已批准。Gate 1 为 2026-10-23 用户「同意你的意见」。Gate 2 为同日原话「批准」。Iteration 000 与 Iteration 001（Gate 2 于 2026-10-24 获用户原话「批准」）均经 Plan Review `accepted`——7 任务全部完成，全量终态 2155/0/2。change 待收尾（docs-maintainer 合并规格、同步 tasks/M/R/I、归档）。随 Iteration 001 展开按九步事实修正过 spec delta、design D2、proposal 与本文件的类属表述（WAL sync 失败归入「DROP 已生效」类、数据文件 `sync` 无调用点不设开关位、R1 复用窗口界定），修正记录见该 Cycle 的 Background。

## 1. checkpoint 前崩溃（Iteration 000）

- [x] 1.1 先写 `drop_table_crash_before_checkpoint_keeps_rows`，确认当前是 RED：DROP 成功、不 close 即丢弃句柄后，原行不可查询或 `check_integrity` 出现 error。依赖：无。验收：`cargo test --test drop_persistence_test drop_table_crash_before_checkpoint_keeps_rows -- --test-threads=1` 失败，且失败原因是行丢失或 error 级发现，不是编译错误。
- [x] 1.2 按 D1 把清零从 `free_page` 移到空闲列表弹出时的 `allocate_page`。写零失败则页号回到列表并返回错误。依赖：1.1。验收：1.1 的测试通过；`drop_then_allocate_sees_only_new_rows` 通过。
- [x] 1.3 补上 DROP INDEX、DROP VIEW、ALTER DROP COLUMN、加密库 CREATE TABLE、加密库 DROP TABLE 的崩溃测试。依赖：1.2。验收：`cargo test --test drop_persistence_test -- --test-threads=1` 全部通过。ALTER DROP COLUMN 若达不到规格且必须移动 `flush_all`，停止并回到计划。加密库 CREATE 若必须加密伴生文件才能通过，同样停止。
- [x] 1.4 按 M19 做全量分批回归。依赖：1.3。验收：`--lib --bins` 一批，`tests/*.rs` 按文件名字典序每 10 个一批，每批 `--test-threads=1` 且 0 failed。

## 2. 受控 I/O 失败（Iteration 001）

- [x] 2.1 增加类型化失败开关：下一次数据文件写入或 WAL `fsync` 在系统调用前失败，然后开关清除（数据文件 `sync` 无生产调用点，不设开关位）。未设置时不改变写入。依赖：Iteration 000。验收：开关未设置时既有 `schema_persistence_test` 与 `pipeline_test` 通过；设置后的写入测试收到错误且文件长度与失败前相同。
- [x] 2.2 覆盖三种恢复：写入提前失败（DROP 未生效）、WAL sync 失败（DROP 已生效）、`.checkpoint` 路径为目录（DROP 已生效）。依赖：2.1。验收：`cargo test --test drop_persistence_test -- --test-threads=1` 通过；WAL sync 与位点失败用例看到 DROP 已生效且 error 级发现为 0。
- [x] 2.3 按 1.4 的分批方式再做一次全量回归。依赖：2.2。验收：各批 0 failed。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| checkpoint 前崩溃保留 DROP TABLE 的已提交行 | 1.1、1.2 | `drop_table_crash_before_checkpoint_keeps_rows` | Covered |
| checkpoint 前崩溃保留 DROP INDEX | 1.3 | `drop_index_crash_before_checkpoint_keeps_index` | Covered |
| checkpoint 前崩溃保留 DROP VIEW | 1.3 | `drop_view_crash_before_checkpoint_keeps_view` | Covered |
| 同会话再分配不读到旧行 | 1.2 | `drop_then_allocate_sees_only_new_rows` | Covered |
| 删列成功后的崩溃保留删列结果 | 1.3 | `alter_drop_column_crash_keeps_narrowed_rows` | Covered |
| 加密库 CREATE 后崩溃表仍在 | 1.3 | `encrypted_create_table_survives_crash` | Covered |
| 加密库 DROP TABLE 与明文同类 | 1.3 | `encrypted_drop_table_crash_keeps_rows` | Covered |
| 写入在改文件之前失败则保留 DROP 前状态 | 2.1、2.2 | `checkpoint_write_fault_keeps_dropped_table` | Covered |
| WAL sync 失败时 DROP 已生效且 check 无 error | 2.2 | `checkpoint_wal_sync_fault_drop_is_durable` | Covered |
| 位点文件无法创建时 DROP 已生效且 check 无 error | 2.2 | `checkpoint_site_occupied_drop_is_durable` | Covered |
| DDL 有序落盘要求把删列从 checkpoint 列举中分开 | 1.3 | 不单独新增测试；由删列崩溃测试和既有 `ddl_crash_recovery_test` 一起约束 | Covered |

## Iteration Plan

### Iteration 000: checkpoint 前的 DROP 崩溃结果

- Tasks: 1.1、1.2、1.3、1.4
- Depends on: None
- Stable result: 不 checkpoint 的进程内崩溃后，DROP TABLE / DROP INDEX / DROP VIEW 的对象和已提交内容仍在；删列成功后的崩溃看到窄表；同会话新分配看不到旧行。
- Verification boundary: `tests/drop_persistence_test.rs`。全量分批只在 1.4 做一次。
- Diagnostic boundary: `src/storage/file_storage.rs` 的 `free_page` 与 `allocate_page`；`src/storage/data/table_manager.rs` 的 `drop_table`、`alter_drop_column`；`src/executor/drop_view.rs`
- Non-goals: I/O 失败开关、位点失败、真实断电、空闲页跨重启复用

### Iteration 001: 受控 I/O 失败

- Tasks: 2.1、2.2、2.3
- Depends on: Iteration 000
- Stable result: 写入在改字节之前失败时，DROP 前的行还在；WAL sync 或位点路径失败时（脏页已写完），DROP 已生效且 check 无 error。
- Verification boundary: 同一测试文件中的三个失败用例，外加开关本身不改变文件长度。全量分批只在 2.3 做一次。
- Diagnostic boundary: `Database` 的失败开关、`FileStorage::write_page` / `sync`、`WalWriter::fsync`、checkpoint 位点写入
- Non-goals: 半页写入、断电、改 checkpoint 九步

## 平衡审计

000 只回答「checkpoint 之前崩溃，已提交状态是什么」，产品改动集中在释放与再次分配。001 才回答「写或同步失败之后是哪一种确定状态」，故障从丢弃句柄换成返回错误。两段的验证命令不同，合成一段会在同一次失败里分不清是清零还是注入。拆得再细，1.1 单独不能形成可依赖的崩溃结果，所以不把 RED 测试拆成自己的 Iteration。
