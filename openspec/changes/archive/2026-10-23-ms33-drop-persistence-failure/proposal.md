## Why

DROP 类语句的页释放以 checkpoint 为界，但现有进程崩溃记录没有覆盖这个边界，也没有覆盖磁盘满、短写和同步失败。来源是里程碑 MS33（`.agents/docs/tasks.md`）和已提升条目 I086。MS32 已完成，MS33 是路线上下一项。

2026-10-23 用户以「同意你的意见」确认下面的范围。

## What Changes

- 进程在 checkpoint 之前结束时，已成功返回的 `DROP TABLE`、`DROP INDEX`、`DROP VIEW` 不得留下「catalog 仍指向该对象、其页已被清零」的状态。对象和它在 DROP 之前的已提交内容仍然可打开、可查询，`check` 的 error 级发现为 0。
- 同一会话里，随后的分配仍得到空页，不能读回已释放页上的旧行。
- `ALTER TABLE DROP COLUMN` 成功返回后、尚未 close 就崩溃时，重开看到删列后的表，剩余列的已提交值还在，`check` 的 error 级发现为 0。这一条记录现有 `flush_all` 加 WAL 提交的结果，不把删列改回「等到 checkpoint 才生效」。
- 加密库上的 `CREATE TABLE` 在语句成功返回后崩溃，重开后表仍在。加密库上的 `DROP TABLE` 与明文库使用同一条崩溃结果。不加密 `.wal` 和 `.checkpoint`。
- 受控 I/O 失败分成两类结果（2026-10-24 按 checkpoint 九步事实修正类属）：数据文件写入在刷脏页之前失败时，调用返回错误，重开仍是 DROP 前的状态；WAL `sync` 或位点文件写在脏页已经写完之后失败时，调用返回错误，重开时 DROP 已经生效，`check` 的 error 级发现为 0。
- 真实断电不在本轮验证，也不据此宣称断电安全。

不做：

- 空闲页跨重启复用、尾部截断、整库整理（MS35）。
- 历史库和旧 dump 兼容（MS36）。
- 伴生文件加密（I060）。
- 撕裂树的运行期根修（I031）。
- `TRUNCATE`。
- 取消、超时和调用方断开后的事务收尾（MS34）。
- 改 WAL 记录格式、checkpoint 位点格式，或 checkpoint 的九步顺序。
- 把 `DROP TABLE` / `DROP INDEX` / `DROP VIEW` 改成语句返回前就落盘的 commit point。

### 场景草图

1. DROP TABLE 后崩溃。前置：表内有已提交行，另有一张未涉及的表，没有未完成的 checkpoint。动作：`DROP TABLE` 成功返回，按现有崩溃助手关闭 WAL 后 drop 数据库句柄，再打开。结果：被删的表和原行都在，另一张表的行也在，`check` 的 error 级发现为 0。失败边界：表还在但行没了，或 `check` 报 error。
2. DROP INDEX 后崩溃。前置：表上有二级索引，等值查询走该索引。动作：`DROP INDEX` 成功后同样崩溃再打开。结果：索引仍在，等值查询行集与崩溃前一致，`check` 的 error 级发现为 0。失败边界：索引名还在，树页已是空页。
3. DROP VIEW 后崩溃。前置：视图可查询。动作：`DROP VIEW` 成功后崩溃再打开。结果：视图仍可查询出原基表行。失败边界：打开失败，或视图在而基表行变了。
4. 同会话再分配。前置：`DROP TABLE` 已成功，尚未关闭。动作：再建表并插入、查询。结果：新表只有新插入的行。失败边界：查询读到被删表的旧行。
5. ALTER DROP COLUMN 后崩溃。前置：表有两列已提交数据。动作：`ALTER TABLE DROP COLUMN` 成功后崩溃再打开。结果：列已消失，剩余列的值与删列后一致，`check` 的 error 级发现为 0。失败边界：列定义和行宽不一致，或要改刷盘顺序才能达到该结果。后一种情况停止，不在本轮改。
6. 加密库 CREATE 后崩溃。前置：带密钥打开的库。动作：`CREATE TABLE` 成功后崩溃，用同一密钥打开。结果：表存在，可插入并读回。失败边界：修复必须加密伴生文件，或必须改 WAL / checkpoint 格式。碰到就停止。
7. 加密库 DROP TABLE 后崩溃。动作与场景 1 相同，库是加密库。结果与场景 1 相同。
8. 数据文件写在改文件之前失败。前置：`DROP TABLE` 已成功，脏页还在缓冲池。动作：下一次数据文件写在不修改目标字节的情况下返回错误，再执行 checkpoint。结果：checkpoint 返回错误；重新打开后场景 1 的表和行都在。失败边界：写出半页再返回错误。本轮不实现这种半页写入；若只有半页写入才能覆盖某条失败，停止。
8b. WAL `sync` 失败（2026-10-24 修正：WAL fsync 是九步第 3 步，晚于第 2 步刷脏页，原「表和行都在」的预期不可达）。前置：`DROP TABLE` 已成功。动作：下一次 WAL `sync` 不调用 `sync_all` 即失败，再执行 checkpoint。结果：checkpoint 返回错误；重新打开后 DROP 已生效，其他表的已提交行还在，`check` 无 error 级发现。
9. checkpoint 位点写失败。前置：`DROP TABLE` 已成功。动作：把 `.checkpoint` 路径换成目录，再 checkpoint。结果：checkpoint 返回错误；重新打开后表已不在，其他表的已提交行还在，`check` 的 error 级发现为 0。失败边界：表还在但行已被清掉。
10. 真实断电。不执行，不把场景 1 到 9 的结果写成断电安全。

## Capabilities

### New Capabilities

- `drop-persistence-boundary`：DROP 类在 checkpoint 前崩溃的可观察结果，同会话再分配得到空页，以及受控 I/O 失败后的两种恢复结果。

### Modified Capabilities

- `wal-recovery-replay-integrity`：DDL 语句边界那条要求里，把 `ALTER TABLE DROP COLUMN` 从「只维持 checkpoint 边界」的列举中分开。删列成功返回后的崩溃结果改由新能力规定。`DROP TABLE` / `DROP INDEX` / `DROP VIEW` / `TRUNCATE` 仍不是语句返回前的 commit point。

## Impact

- 代码：`src/storage/file_storage.rs` 的 `free_page` 与 `allocate_page`；`src/database.rs`、`src/wal/writer.rs` 上供测试使用的一次 I/O 失败开关。checkpoint 九步和页格式不动。
- 测试：新增 `tests/drop_persistence_test.rs`。既有 `tests/ddl_crash_recovery_test.rs`、`tests/schema_persistence_test.rs`、`tests/pipeline_test.rs` 保持通过。全量回归按项目模型 M19，每个 Iteration 末一次。
- API：库增加测试用的失败开关，不增加 CLI flag，不写入用户文档。未调用时写路径与现在相同。
- 回滚：恢复 `free_page` 的立即清零，并去掉失败开关。无数据迁移。已用本轮之前版本写过的库文件不改格式。
