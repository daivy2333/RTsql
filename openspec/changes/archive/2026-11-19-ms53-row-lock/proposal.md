## Why

里程碑 MS53（`.agents/docs/tasks.md`）要求复制、分配和行锁争用都有测量。只有被测量证明值得改的那一处进入实现，SQL 结果不变，其余保留原实现。

复制已经由归档变更 `openspec/changes/archive/2026-11-19-ms53-avoidable-copy/` 交付：嵌套循环的组合行改为移入再移回。分配器已经由归档变更 `openspec/changes/archive/2026-11-19-ms53-system-allocator/` 测量，并保留系统默认分配器。路线里进入实现的名额已经被复制用掉。本次是最后一项，来源是 I050。

I050 的原方向是把 `RowLockTable` 迁到 `DashMap`，前置是先做 mini-bench。本次 goal 把结论定为保留现有实现。测量放在现有符号上，不新做一条用来决定改结构的 mini-bench，也不设数字门槛。不改锁语义、可见性和锁等待的对外错误。不改产品代码，不改 `Cargo.toml`。

## What Changes

- 测量落在 `RowLockTable::get_lock`（`src/transaction/row_lock.rs`）。仓库里的 `.rs` 文件中，调用它的只有同文件三个测试。`src/transaction/mod.rs` 只再导出这个类型。执行器、管道和 `Database` 不调用它。
- 观察 `locks` 的类型，以及 `get_lock` 的加锁路径：已有该行时在读锁下克隆 `Arc`；没有该行时取得写锁，再查一次，然后插入新的 `Mutex`。
- 三个既有测试在基点 `b6e81b3bd781393cef8e9d6a33e15ef391a0749b` 上已经跑过一次，结果是 3 passed。同行等待和不同行不互相阻塞，由这两个测试自己的断言表达。不把其中的 40ms、20ms 写成换结构的门槛。
- 执行记录写明保留现有 `RowLockTable`，不迁到 `DashMap`。
- `src/` 与 `Cargo.toml` 保持与基点相同。

### 场景草图

1. 持锁结构。前置：`RowLockTable.locks` 的类型是 `tokio::sync::RwLock<HashMap<RowId, Arc<tokio::sync::Mutex<()>>>>`。动作：读 `get_lock`。结果：表里已有该行时，在读锁下克隆 `Arc` 并返回；没有该行时取得写锁，再查一次，然后插入新的 `Mutex`。调用者拿到的是这把行锁，要自己 `lock().await`。表锁在 `get_lock` 返回前释放。失败边界：字段类型或这两段加锁与源码不一致。停止，不改文件。
2. 同行等待。前置：`test_row_lock_concurrent_same_row` 仍在。动作：沿用基点上这一次测试运行。结果：该测试通过。第一个任务持有行锁 50ms，第二个任务从计时开始到 `lock().await` 返回，满足测试里已有的 `>= 40ms`。失败边界：把 40ms 当成换 `DashMap` 的条件。这个数只属于该测试的等待检查。
3. 不同行。前置：`test_row_lock_different_rows` 仍在。动作：同一次运行。结果：该测试通过。另一行持锁 50ms 时，本行 `lock().await` 满足测试里已有的 `< 20ms`。失败边界：把 20ms 当成迁移门槛，或为了让上界更稳而重跑、改断言。
4. 没有 SQL 调用者。前置：对仓库 `*.rs` 检索 `RowLockTable` 与 `get_lock(`。动作：核对调用点。结果：调用点只有 `row_lock.rs` 的三个测试。失败边界：实施时若发现执行器或 SQL 入口调用了 `get_lock`，停止并回报。不在本次新增一条 SQL 写路径。
5. 保留。前置：结构、调用点和这一次测试结果已经写入执行记录。动作：写明保留现有 `RowLockTable`。结果：`src/` 与 `Cargo.toml` 没有差异。`StorageError::DatabaseLocked`、锁等待预算、`Snapshot::is_visible` 和 `DataScanExecutor::superseder_suppresses` 都不改。失败边界：要改锁语义、可见性或锁等待的对外错误才能写完记录。停止并回报。

### 已经选定的行为

- 没有 SQL 调用者。测量用现有符号 `RowLockTable::get_lock` 和同文件三个测试。不新增测试文件，也不新增会改变 SQL 结果的写路径。
- 这一次测量包含两件事：`locks` 的类型与 `get_lock` 的加锁路径；基点上那一次 `cargo test --offline --lib transaction::row_lock::tests -- --test-threads=1` 的通过结果。不另做 mini-bench。
- 测试里的 40ms 和 20ms 留在原断言里。不新设一个数字，用来决定是否改表结构。
- 结论是保留现有实现。同行的等待发生在该行的 `Mutex` 上。SQL 执行不进入这张表，本次不把它换成 `DashMap`。
- 产品对外合同不变，因此不写行为规格增量。`.openspec.yaml` 使用 `skip_specs: true`。

## Capabilities

### New Capabilities

- 无。SQL 结果、可见性和锁等待的对外错误保持现状。保留 `RowLockTable` 是本变更的设计结论，写在 `design.md`。

### Modified Capabilities

- 无。`openspec/specs/avoidable-copy/spec.md` 已经要求 `RowLockTable` 仍用 `RwLock<HashMap<...>>`。本次不改这条要求。

## Impact

- 代码：不改 `src/`，不改 `Cargo.toml`。不新增测试文件。
- 引用：MS53、I050、M19、M21、M22。M19 要求这次测试调用带 `--test-threads=1`。计划调查时已经这样跑过，实施沿用该结果，不重跑。M21 不新增从执行层到这张表的调用。M22 不新增诊断计数。
- 测试：沿用上面这一条命令的结果。不跑全量。本变更不改产品代码，不触及跨模块公共面，也没有发布或迁移门槛。全量读不到这次要写入执行记录的持锁结构。
- 数据：无格式变更，无迁移。

## 回滚

没有产品改动。去掉本变更的执行记录，就没有这次保留结论。已有库文件不用迁移。
