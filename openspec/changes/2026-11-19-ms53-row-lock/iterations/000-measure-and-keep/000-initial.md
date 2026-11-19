# Iteration 000 / Cycle 000: 测量行锁持有结构并保留现有 RowLockTable

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Inherited scope: `proposal.md` 的五个场景，以及 `design.md` 的 D1 到 D5。结论是保留现有 `RowLockTable`。
- Excluded scope: 节点锁、页 I/O、io_uring、导入攒批、嵌套循环移动实现的再改、分配器、`DashMap`、新测试、新的 SQL 写路径、行为规格增量、全量套件。

**Objective**

执行记录写明 `RowLockTable::get_lock` 的持锁结构、调用点，以及基点上三个既有测试的那一次输出，并写明保留现有 `RowLockTable`。本 Cycle 不改产品代码，不重跑测试。

**Investigation Facts**

- Starting point: 分支 `changes/2026-11-19-ms53-row-lock`，基点 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b`。没有其他活跃变更。`src/transaction/row_lock.rs` 的 `locks` 是 `tokio::sync::RwLock<HashMap<RowId, Arc<tokio::sync::Mutex<()>>>>`。`Cargo.toml` 已有 `dashmap = "6"`，这张表没有使用它。在该基点执行 `cargo test --offline --lib transaction::row_lock::tests -- --test-threads=1`，退出码 0。命令没有 `--release`。
- Current-State Evidence: `get_lock` 在读锁下命中则克隆 `Arc` 返回；未命中则取得写锁，再查一次，仍没有就插入新的 `Mutex`。表锁在函数返回时放开。调用者自己 `lock().await`。`get_lock(` 的 `.rs` 调用点只有 `test_row_lock_acquire_release`、`test_row_lock_concurrent_same_row`、`test_row_lock_different_rows`。`src/transaction/mod.rs` 只做 `pub use row_lock::RowLockTable`。执行器目录没有这张表。可见性在 `Snapshot::is_visible` 与 `DataScanExecutor::superseder_suppresses`。文件锁等待的对外错误是 `StorageError::DatabaseLocked`。同行测试持锁 50ms、提前 10ms，断言 `>= 40ms`。不同行测试断言 `< 20ms`。这两个数是测试自己的检查。
- Code and Critical Path: 没有 SQL 入口。测量路径就是 `RowLockTable::get_lock` 和上面三个测试。测试输出如下，`457 filtered out` 是过滤没选中的其他库测试：

```
running 3 tests
test transaction::row_lock::tests::test_row_lock_acquire_release ... ok
test transaction::row_lock::tests::test_row_lock_concurrent_same_row ... ok
test transaction::row_lock::tests::test_row_lock_different_rows ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 457 filtered out; finished in 0.10s
```

**Task Definitions**

### T1: 记下持锁结构、调用点和这一次测试输出

- Requirement/Scenario: `proposal.md` 场景 1、2、3、4。
- Depends on: None
- Targets: 本 Cycle 的 Act Response。不修改 `src/`、`Cargo.toml`、`tests/`。
- Current behavior: 结构、调用点和测试输出见 Investigation Facts。执行记录里还没有这份抄写。
- Required behavior: 先只读核对 `src/` 相对 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b` 没有差异。执行记录抄写 `locks` 的类型、`get_lock` 的读锁命中与写锁插入、三个测试名，以及上面的命令输出原文。写明调用点只有这三个测试，执行器不调用 `get_lock`。写明 40ms 和 20ms 不是换结构的门槛。不重跑测试，不新增耗时字段。
- Preserve: `src/`、`Cargo.toml`、`tests/`、三个测试的断言、嵌套循环的移入再移回、系统分配器、可见性判断、`DatabaseLocked` 和锁等待预算。
- Forbidden: 修改 `src/transaction/row_lock.rs`；把 `locks` 换成 `DashMap`；新增测试文件；新增 SQL 写路径；重跑这三个测试；改 40ms 或 20ms；新增诊断计数；为了得到写锁耗时而加计时。
- First test: 基点上的命令已经退出码 0，三条都是 `ok`，汇总行是 `3 passed; 0 failed`。这是计划在 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b` 上的观察。Act 沿用它，不再跑。
- GREEN condition: 执行记录里的输出原文与 Investigation Facts 的代码块一致，并写有类型、加锁路径和调用点。`src/` 相对基点没有差异。
- Verification: 对照 Investigation Facts 的代码块核对抄写。用 `git diff --exit-code b6e81b3bd781393cef8e9d6a33e15ef391a0749b -- src` 确认没有差异。这条 diff 只用来判断那一次检索和测试是否还覆盖当前 `src/`。
- Stop when: `src/` 相对基点有差异；抄写缺少三条 `ok` 或汇总行。停止并回到计划。不重跑，不改产品代码。

### T2: 写明保留现有 RowLockTable

- Requirement/Scenario: `proposal.md` 场景 5。
- Depends on: T1
- Targets: 本 Cycle 的 Act Response。不修改其他文件。
- Current behavior: T1 写下测量之后，保留结论还没有进入同一节。
- Required behavior: 同一节写明保留现有 `RowLockTable`，不迁到 `DashMap`。说明测试输出和 40ms、20ms 都不作为切换条件。相对实施开始时的 HEAD，`git diff --name-only HEAD -- src Cargo.toml tests` 没有路径。本 Cycle 允许改动的只有这份 Act Response。
- Preserve: T1 抄写的命令输出原文。
- Forbidden: 看到同行等待之后改成迁移；修改 `src/` 或 `Cargo.toml`；新增 `tests/` 下的文件；把测试失败写成保留结论。本次测试是通过的，不存在这个输入。
- First test: 无新测试。输入是 T1 抄进执行记录的那一次输出。
- GREEN condition: 执行记录含「保留现有 RowLockTable，不迁到 DashMap」。`src/`、`Cargo.toml`、`tests/` 相对实施开始时的 HEAD 没有差异。
- Verification: 阅读 Act Response，确认保留句和 T1 的输出都在。再跑 `git diff --name-only HEAD -- src Cargo.toml tests`。`HEAD` 是实施开始时的变更分支顶端。
- Stop when: T1 的输出缺失，或 diff 出现 `src/`、`Cargo.toml`、`tests/`。停止。不要重跑测试，也不要改产品代码来清掉 diff。

**Invariants**

SQL 结果、可见性、`StorageError::DatabaseLocked`、锁等待预算、诊断阶段名、磁盘格式、嵌套循环的移入再移回，以及库的全局分配器，在本 Cycle 结束时与起点相同。`RowLockTable` 仍是 `RwLock<HashMap<RowId, Arc<Mutex<()>>>>`。不新增测试文件。

**Acceptance**

- T1 抄写的命令输出与 Investigation Facts 的代码块一致，退出码 0，`3 passed; 0 failed`。记录写明类型、加锁路径，以及调用点只有三个测试。
- T2 写明保留现有 `RowLockTable`，不迁到 `DashMap`，并写明那些毫秒数不是切换条件。
- `src/`、`Cargo.toml`、`tests/` 相对实施开始时的 HEAD 没有差异。
- 源文件相对基点有差异时，Act Response 为 `blocked`，记录里不出现保留句。

**Verification**

沿用 `cargo test --offline --lib transaction::row_lock::tests -- --test-threads=1` 在基点 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b` 的那一次结果。退出码 0，`3 passed; 0 failed`。这一次覆盖场景 1 到 3 的既有断言。场景 4 由 T1 按调查时的调用点核对 `src/` 相对基点未变。场景 5 由 T2 写入执行记录，不另跑命令。不把 40ms 或 20ms 与新的截止值比较。

**Gate 2 Readiness**

PASS。审查项全部通过。

**Risks and Notes**

- 不同行测试的 `< 20ms` 在旧记录里曾被怀疑受负载影响。本次这一跑通过。不重跑，不改断言。
- 同行等待在行 `Mutex` 上仍然存在。它不决定接口、错误或是否改用 `DashMap`。
- 整张表的写锁会串行不同行的第一次插入。本次没有单独的写锁耗时，不补计时。
- 验证结果写入 Act Response。不建 `evidence/` 目录。

## Act Response

- Status: reported

`src/transaction/row_lock.rs` 里 `RowLockTable` 的 `locks` 是 `tokio::sync::RwLock<HashMap<RowId, Arc<tokio::sync::Mutex<()>>>>`。`get_lock` 在读锁下命中则克隆 `Arc` 返回；未命中则取得写锁，再查一次，仍没有就插入新的 `Mutex`。表锁在函数返回时放开。调用者自己 `lock().await`。

`get_lock(` 的 `.rs` 调用点只有 `test_row_lock_acquire_release`、`test_row_lock_concurrent_same_row`、`test_row_lock_different_rows`。执行器不调用 `get_lock`。`src/transaction/mod.rs` 只做 `pub use row_lock::RowLockTable`。

`test_row_lock_concurrent_same_row` 持锁 50ms，另一侧提前 10ms 开始等，断言 `>= 40ms`。`test_row_lock_different_rows` 断言 `< 20ms`。40ms 和 20ms 是这两个测试自己的检查，不是换结构的门槛。这一次测试输出和这两个毫秒数都不作为切换条件。

保留现有 RowLockTable，不迁到 DashMap。

沿用基点 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b` 上的 `cargo test --offline --lib transaction::row_lock::tests -- --test-threads=1`。退出码 0。命令没有再次执行。输出原文：

```
running 3 tests
test transaction::row_lock::tests::test_row_lock_acquire_release ... ok
test transaction::row_lock::tests::test_row_lock_concurrent_same_row ... ok
test transaction::row_lock::tests::test_row_lock_different_rows ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 457 filtered out; finished in 0.10s
```

`git diff --exit-code b6e81b3bd781393cef8e9d6a33e15ef391a0749b -- src` 退出码 0。`git diff --name-only HEAD -- src Cargo.toml tests` 没有路径。当时的 `HEAD` 是 `263be5df13a23eb72533b497a8dcada375df286f`。

改动是本文件的 Act Response，以及 `openspec/changes/2026-11-19-ms53-row-lock/tasks.md` 的状态行。`src/`、`Cargo.toml`、`tests/` 没有改。没有新测试，也没有耗时字段。

与计划的偏差：任务定义写本 Cycle 允许改动的只有这份 Act Response。`tasks.md` 里 Iteration 000 改为已执行，T1 和 T2 改为已完成，用来让任务状态和这一轮的完成情况一致。任务正文没有改。

Experience Candidates: None

未解决问题：无。

## Plan Review

- Review Result: pending
