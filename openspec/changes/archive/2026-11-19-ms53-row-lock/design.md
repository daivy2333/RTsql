## 背景

MS53 要先测量复制、分配和行锁争用。复制已经改过：嵌套循环组合行改为移入再移回，归档在 `openspec/changes/archive/2026-11-19-ms53-avoidable-copy/`。分配器已经测量并保留系统默认分配器，归档在 `openspec/changes/archive/2026-11-19-ms53-system-allocator/`。路线里进入实现的名额已经被复制用掉。

本次只测量行锁，来源是 I050。结论是保留现有 `RowLockTable`。

当前起点是 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b`。分支 `changes/2026-11-19-ms53-row-lock` 从这里切出。没有其他活跃变更。

## 当前行为

`RowLockTable` 在 `src/transaction/row_lock.rs`。字段 `locks` 的类型是 `tokio::sync::RwLock<HashMap<RowId, Arc<tokio::sync::Mutex<()>>>>`。`RowId` 来自 `crate::storage::RowId`。

`get_lock` 先在一段读锁里查找。找到就克隆 `Arc` 并返回。找不到就放开读锁，再取得写锁，查第二次。第二次找到同样克隆返回。仍然没有，就插入 `Arc::new(Mutex::new(()))` 并返回这个 `Arc`。表上的锁在函数返回时放开，不覆盖调用者随后的 `lock().await`。这个函数不返回 `Result`，也没有超时。

仓库里的 `.rs` 文件中，`get_lock(` 的调用点只有同文件三个测试：

- `test_row_lock_acquire_release`：`RowId::new(1, 2)`，取锁并 `lock().await`，没有计时断言。
- `test_row_lock_concurrent_same_row`：同一行。第一个任务持锁 50ms。第二个任务先等 10ms 再计时，断言从计时到 `lock().await` 返回不少于 40ms。
- `test_row_lock_different_rows`：`RowId::new(1, 1)` 与 `RowId::new(1, 2)`。一个任务持锁 50ms，另一个断言自己的 `lock().await` 在 20ms 内返回。

`src/transaction/mod.rs` 有 `pub use row_lock::RowLockTable`。`src/lib.rs` 公开 `transaction` 模块，因此类型路径是 `rtsql::transaction::RowLockTable`。执行器目录没有 `RowLockTable`，也没有 `get_lock`。SQL 语句不调用这张表。

可见性在 `Snapshot::is_visible` 和 `DataScanExecutor::superseder_suppresses`。文件锁等待的对外错误是 `StorageError::DatabaseLocked`，等待预算在文件打开路径上。这三处都不经过 `get_lock`。

`Cargo.toml` 已有 `dashmap = "6"`，缓冲池和计划缓存在用。`RowLockTable` 没有使用它。

在基点上执行：

`cargo test --offline --lib transaction::row_lock::tests -- --test-threads=1`

退出码 0。输出是：

```
running 3 tests
test transaction::row_lock::tests::test_row_lock_acquire_release ... ok
test transaction::row_lock::tests::test_row_lock_concurrent_same_row ... ok
test transaction::row_lock::tests::test_row_lock_different_rows ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 457 filtered out; finished in 0.10s
```

`457 filtered out` 是这条过滤没有选中的其他库测试。三个行锁测试都跑了。命令没有 `--release`。这一次之后不再跑。

## 目标与非目标

目标：把上面的持锁结构和这一次测试结果写入执行记录，并写明保留现有 `RowLockTable`。

非目标：节点锁、页 I/O、io_uring、导入攒批、再改嵌套循环的移动实现、再改分配器、迁到 `DashMap`、新增测试、新增 SQL 写路径、行为规格增量。

## D1 测量落在现有符号

测量对象是 `RowLockTable::get_lock`，加上同文件三个测试。没有 SQL 调用者，所以不新增一条写语句来制造争用。

实施要抄写的是本设计「当前行为」里的类型、加锁路径、调用点和那一次命令输出。不重跑测试，不打印新的耗时。

```mermaid
flowchart TD
  call["get_lock"]
  read["持有表的读锁查找"]
  hit{"已有该行"}
  give["克隆 Arc 并返回"]
  write["持有表的写锁"]
  again{"写锁下再次查到"}
  insert["插入新的 Mutex"]
  row["调用者再 lock.await"]
  call --> read --> hit
  hit -->|有| give
  hit -->|没有| write --> again
  again -->|有| give
  again -->|没有| insert --> give
  give --> row
```

表锁在 `get_lock` 返回时放开。`row` 这一步持有的是该行自己的 `Mutex`。

## D2 争用看结构和既有断言

同行的等待在该行的 `Mutex` 上。`test_row_lock_concurrent_same_row` 用 50ms 持锁和 10ms 提前量，写出 `>= 40ms`。这次运行该测试通过，所以这个等待检查成立。

不同行各自一把 `Mutex`。第一次插入时，两行会先后占用整张表的写锁。`test_row_lock_different_rows` 的 `< 20ms` 包住了 `get_lock` 和随后的 `lock().await`。这次运行该测试通过。没有单独量出写锁持有了多久，也不补这一次计时。

40ms 和 20ms 留在原测试里。它们不决定是否改表。

## D3 保留现有表

执行记录写明：保留现有 `RowLockTable`，不迁到 `DashMap`。

`locks` 继续是 `RwLock<HashMap<RowId, Arc<Mutex<()>>>>`。不改 `src/transaction/row_lock.rs`，不改 `Cargo.toml`。

## D4 不采用的做法

| 方案 | 不采用的原因 |
|---|---|
| 新增一条 SQL 写路径来制造争用 | 执行器现在不调用 `get_lock`。新写路径会碰到 SQL 结果或锁等待的对外错误 |
| 新的 mini-bench，并设一个决定改结构的数字门槛 | 结论已经是保留。这种门槛没有现成的代码依据 |
| 把 `RowLockTable` 换成 `DashMap` | 依赖里虽有 `dashmap`，本次不改这张表的结构 |
| 重跑三个测试，记录新的毫秒数 | 基点上已经通过一次。新的毫秒数仍然不决定结构 |
| 放宽或收紧 20ms、40ms | 这两个数是既有测试自己的检查 |

## D5 不改的边界

不修改 `src/executor/nested_loop_join.rs` 的移入再移回。不改系统分配器，不改 `Cargo.toml`。不新增诊断阶段或计数字段。不改 `Snapshot::is_visible`、`DataScanExecutor::superseder_suppresses`、`StorageError::DatabaseLocked` 和锁等待预算。不改文件格式。

可见性或锁等待的对外错误若必须改变才能写完记录，停止并回报。

## 风险

`test_row_lock_different_rows` 的 `< 20ms` 在旧记录里曾被怀疑受负载影响。本次这一跑是通过的。不重跑，不改断言。以后若有人看到它失败，也不在本变更里改表或改断言。

同行等待仍然存在于行 `Mutex` 上。这个事实不构成换成 `DashMap` 的条件。

第一次插入不同行时，整张表的写锁会串行这些插入。本次没有单独的写锁耗时。不补计时。

`src/` 若相对基点出现差异，上面那一次检索和测试结果就不再覆盖当前代码。停止，不重跑。

## 回滚

没有产品改动。去掉执行记录里的这次测量和保留结论即可。已有库文件不用迁移。

## 实现顺序

1. 只读核对 `src/` 相对基点 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b` 没有差异。有差异就停止。调用点检索覆盖的是当时的 `src/`。
2. 把 D1、D2 的结构和命令输出写入执行记录。不重跑。
3. 在同一节写明保留现有 `RowLockTable`，不迁到 `DashMap`。
4. 确认相对实施开始时的 HEAD，`src/`、`Cargo.toml` 和 `tests/` 没有差异。
