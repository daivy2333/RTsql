# 进程内 checkpoint 前崩溃的复现与判别配方

- Status: active
- Last validated: 2026-10-23
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `Database::open` / `Database::open_with_key`、tokio 多线程 test runtime
- Source: change `2026-10-23-ms33-drop-persistence-failure` Iteration 000 / Cycle 000 Act Response（`openspec/changes/2026-10-23-ms33-drop-persistence-failure/iterations/000-crash-before-checkpoint/000-initial.md`，2026-10-23）

## 适用范围

- 在同一进程内构造「checkpoint 之前进程结束」的持久化状态，并观察重开后能读到什么：见证 DROP TABLE / DROP INDEX / DROP VIEW / ALTER TABLE DROP COLUMN / 加密库同类语句的崩溃语义。
- 判别「某个持久化缺口是真的存在，还是被 WAL 重放掩盖了」。
- 不适用：真实断电（本配方不产生真实进程终止与文件级故障）；跨进程锁冲突（见 `.agents/runbooks/cli-subprocess-witnessing.md`）；普通的 close→reopen 持久化断言（用 `.agents/runbooks/in-process-reopen-testing.md`，该配方明确不含崩溃模拟）。

## 前置条件

- 库文件放在 `tempdir()` 下，路径用 `dir.path().join("x.db")`。
- 崩溃助手形态：`wal_buffer.shutdown()` 后 `drop(db)`，**不调用 `close()`**——`close` 会做 checkpoint 并截断 WAL，脏页因此不丢，崩溃语义就没了。

```rust
async fn crash(db: Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}
```

- **判别持久化缺口前必做**：让「已提交内容」先真正落盘、WAL 不再持有它们，否则缺口观察不到（见下节）。

## 操作步骤

1. 第一会话：建表、写入、提交，然后 `close()`（checkpoint + WAL 截断），并显式 `drop` 释放 advisory 锁：

```rust
{
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    db.close().await.unwrap();
    drop(db); // close(&self) 不释放 advisory 锁，显式 drop 后才能重开
}
```

2. 第二会话：执行被观察的语句，然后崩溃，不 checkpoint：

```rust
let db = Database::open(&path).await.unwrap();
ok(&db, "DROP TABLE t").await;
crash(db).await;
```

3. 第三会话：重新打开，断言重开后的可观察结果（对象是否还在、行是否可查、`check_integrity` 的 error 级发现是否为 0）。

## 验证

- 成功判据：重开后被语句影响的对象与已提交内容仍可查询，且 `check_integrity` 的 error 级发现数为 0。
- 2026-10-23 实例：`cargo test --test drop_persistence_test -- --test-threads=1` → **7 passed / 0 failed**（`drop_table_crash_before_checkpoint_keeps_rows`、`drop_then_allocate_sees_only_new_rows`、`drop_index_crash_before_checkpoint_keeps_index`、`drop_view_crash_before_checkpoint_keeps_view`、`alter_drop_column_crash_keeps_narrowed_rows`、`encrypted_create_table_survives_crash`、`encrypted_drop_table_crash_keeps_rows`）。判定用测试自身退出码与断言输出，无判定层。
- **关键判别点（2026-10-23 实证）**：WAL 重放按 logical_id 幂等补回尚在 WAL 里的行。已提交行若还在 WAL 里，重开时会被补回，持久化缺口被完全掩盖——用例会「改前即通过」。因此缺口要可观察，必须先让行落盘且 WAL 不再持有它们，也就是第 1 步的 `close()` 会话。单会话直接 DROP 再崩溃，看到的是「行还在」，不能据此判定缺口不存在。
- 改前基线（清零发生在释放时）：`drop_table_crash_before_checkpoint_keeps_rows` 与 `encrypted_drop_table_crash_keeps_rows` 失败（原行不可查询）；`drop_index_crash_before_checkpoint_keeps_index` 是**进程 abort（栈溢出 SIGABRT）**——清零后的索引树页在重开时被当作树遍历，该形态下遍历不收敛；DROP VIEW、ALTER DROP COLUMN、加密库 CREATE 三个形态改前即通过，测试用于钉住它们。

## 失败处理

- **用例在改产品代码前就通过**：先按上面「关键判别点」检查已提交行是否还在 WAL 里（WAL 未截断就会被重放补回）。不要据此直接判定缺口不存在，也不要为了让用例先失败去改产品代码——回到计划核对静态结论。
- **重开报 `DatabaseLocked`**：第一会话的绑定仍存活。`close(&self)` 不释放 advisory 锁，必须显式 `drop`（与 `.agents/runbooks/in-process-reopen-testing.md` 同源）。
- **重开时进程 abort / 栈溢出**：多为被释放的页在磁盘上是零页，而 catalog 仍登记该对象——重开后该页被当作 B-Tree 树遍历且遍历不收敛。属持久化边界缺陷，按 change 纪律回报，不在测试内绕过。
- **重开后 `check_integrity` 报 error 级发现**：说明磁盘上的页内容与 catalog 登记的对象不一致，同样按持久化边界缺陷回报。
- 断言撞预存形状或文案缺陷：按 change 纪律登记 Issue 候选，不在本配方内绕过。

## 回滚

不适用（测试配方，无产品状态变更；误用产生的只有 tempdir 残留，随测试结束清理）。

## 证据

- `openspec/changes/2026-10-23-ms33-drop-persistence-failure/iterations/000-crash-before-checkpoint/000-initial.md`（Act Response 的「与计划的偏差及原因」第 1 条、「自检发现」改前基线表、「验证命令与实际结果」）
- 见证代码：`tests/drop_persistence_test.rs`；崩溃助手先例：`tests/ddl_crash_recovery_test.rs`
- 规格来源：change 内 `specs/drop-persistence-boundary/spec.md`
