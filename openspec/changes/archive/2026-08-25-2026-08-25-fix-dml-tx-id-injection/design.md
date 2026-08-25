# Design: 修复 DML `tx_id=0` 占位注入

## 目标

让 `Database::execute_sql` 发出的 INSERT / UPDATE / DELETE 走完整的 `TransactionManager::begin → executor → TransactionManager::commit/abort` 生命周期，且 tx_id 取自 `TransactionManager` 真实分配。

## 现状（修改前）

### 错误链

1. `src/pipeline.rs:336`（Insert）/`src/pipeline.rs:350`（Update）/`src/pipeline.rs:363`（Delete）三处都构造 executor 时传 `0, // placeholder, will be set by execute_inner`
2. `execute_inner` 内**没有任何代码修改 executor 的 `tx_id` 字段**（executor 已构造为 `Box<dyn Executor>`，无法再 mutate）
3. `tx_manager.begin()` 从未被调用 → `active_tx_ids` 永不增 → snapshot 失效
4. `tx_manager.commit()` 从未被调用 → `commit_tx_id` 永不被写入 → 已提交行不可见
5. `tx_manager.abort()` 从未被调用 → 失败 DML 无回滚

### WAL 重复写入

`InsertExecutor::next()` 在 `src/executor/insert.rs:64-65` 写 `WalRecord::BeginTxn { tx_id: 0 }`，`src/executor/insert.rs:129-140` 写 `WalRecord::CommitTxn { tx_id: 0, timestamp }` + `append_commit_and_wait`。`UpdateExecutor` / `DeleteExecutor` 有相同模式。

这些 BeginTxn/CommitTxn **永远是 `tx_id=0`**，且即使后续 `tx_manager.begin()` 修了 tx_id，这些 executor 内的写入会变成"伪事务"。

## 修改方案

### 1. `src/pipeline.rs::execute_inner` DML 路径

**当前代码**（line 178-238）：
```rust
_ => {
    // ... 解析、planner、缓存 ...
    let executor = match create_executor_from_plan(plan, database).await { ... };
    let response = execute_executor(executor).await;
    return response;
}
```

**新代码**：
```rust
_ => {
    // ... 解析、planner、缓存（保持不变） ...

    // 1. 判断是否 DML，决定是否需要事务
    let is_dml = matches!(&plan,
        PhysicalPlan::Insert(_) | PhysicalPlan::Update(_) | PhysicalPlan::Delete(_)
    );

    // 2. 对 DML：begin 一个事务；同时从 plan 中预先取出 table_meta（abort 路径需要）
    let tx = if is_dml {
        Some(database.transaction_manager.begin().await)
    } else {
        None
    };
    let tx_id = tx.as_ref().map(|t| t.id());

    // 3. 预先取 table_meta（abort 需要；executor 构造也会再取一次，但计划已用 clone）
    let table_meta_for_abort: Option<Arc<TableMeta>> = if is_dml {
        let table_name = match &plan {
            PhysicalPlan::Insert(n) => &n.table_name,
            PhysicalPlan::Update(n) => &n.table_name,
            PhysicalPlan::Delete(n) => &n.table_name,
            _ => unreachable!(),
        };
        database.table_manager.get_table(table_name).await.ok()
    } else {
        None
    };

    // 4. 构造 executor（传入真实 tx_id 或 None）
    let executor = match create_executor_from_plan(plan, database, tx_id).await {
        Ok(e) => e,
        Err(e) => {
            // 构造失败：tx 已 begin，需要 abort
            if let (Some(tx), Some(tm)) = (tx, table_meta_for_abort) {
                let _ = database.transaction_manager.abort(tx, &database.buffer_pool, &tm).await;
            }
            return Response::Error { message: e.to_string() };
        }
    };

    // 5. 执行 executor
    let response = execute_executor(executor).await;

    // 6. DML：根据执行结果 commit 或 abort
    if let Some(tx) = tx {
        match &response {
            Response::Error { .. } => {
                if let Some(tm) = table_meta_for_abort {
                    if let Err(abort_err) = database
                        .transaction_manager
                        .abort(tx, &database.buffer_pool, &tm)
                        .await
                    {
                        return Response::Error {
                            message: format!("Abort failed: {}", abort_err),
                        };
                    }
                }
            }
            _ => {
                if let Err(commit_err) = database
                    .transaction_manager
                    .commit(tx, &database.buffer_pool)
                    .await
                {
                    return Response::Error {
                        message: format!("Commit failed: {}", commit_err),
                    };
                }
            }
        }
    }
    return response;
}
```

### 2. `src/pipeline.rs::create_executor_from_plan` 签名

**当前**：
```rust
pub(crate) fn create_executor_from_plan(
    plan: PhysicalPlan,
    database: &Database,
) -> CreateExecutorFuture<'_>
```

**新**：
```rust
pub(crate) fn create_executor_from_plan(
    plan: PhysicalPlan,
    database: &Database,
    tx_id: Option<u64>,  // DML: Some(real_tx_id), SELECT: None
) -> CreateExecutorFuture<'_>
```

DML 三个分支把 `0` 替换为 `tx_id.expect("DML must have tx_id")` 或解构为 `tx_id.unwrap_or(0)`。SELECT / 嵌套子查询分支仍传 `tx_id`（递归调用向下透传 None）。

### 3. executor 改动

#### `src/executor/insert.rs`
- **删除** `src/executor/insert.rs:63-65` 的 `wal.append(WalRecord::BeginTxn { tx_id: self.tx_id })`
- **删除** `src/executor/insert.rs:129-140` 的 `wal.append(WalRecord::CommitTxn { tx_id, timestamp }) + append_commit_and_wait`
- **保留** `src/executor/insert.rs:107-115` 的 `wal.append(WalRecord::Insert { ... })`（数据 WAL 记录用于 redo）
- **保留** `src/executor/insert.rs:118` 的 `tx_manager.record_version(self.tx_id, row_id)`（commit 标记 versions 用）

#### `src/executor/update.rs`
- **删除** `src/executor/update.rs:98-100` 的 BeginTxn 写入
- **删除** `src/executor/update.rs:130-135` 的 CommitTxn 写入
- **保留** `src/executor/update.rs:117-125` 的 Update WAL 记录
- **保留** `src/executor/update.rs:139` 的 `tx_manager.record_version(self.tx_id, new_row_id)`
- **调整** `src/executor/update.rs:97-99` 的注释（删除 "WAL: BeginTxn (implicit transaction per statement)" 标记，改为说明事务由 pipeline 管理）

#### `src/executor/delete.rs`
- **删除** `src/executor/delete.rs:52-54` 的 BeginTxn 写入
- **删除** `src/executor/delete.rs:100-105` 的 CommitTxn 写入
- **保留** `src/executor/delete.rs:90-95` 的 Delete WAL 记录
- **新增** `tx_manager.record_version(self.tx_id, row_id)` 调用（行 86 之后、wal.append 之前）— 这是 abort 回滚 delete 所必需

### 4. 关键不变量（不修改）

- `TransactionManager::begin()` 自身已写 `WalRecord::BeginTxn`（`src/transaction/manager.rs:83`）
- `TransactionManager::commit()` 自身已写 `WalRecord::CommitTxn` + `append_commit_and_wait`（`src/transaction/manager.rs:110-111`）
- `TransactionManager::abort()` 自身已写 `WalRecord::AbortTxn`（`src/transaction/manager.rs:144`）

executor 删除自己写的 BeginTxn/CommitTxn 后，WAL 仍有正确的事务边界（由 `tx_manager` 写）。

## 行为差异表

| 场景 | 改前 | 改后 |
|---|---|---|
| 单 INSERT | `create_tx_id=0`，`commit_tx_id=None`，行不可见 | `create_tx_id=real`，`commit_tx_id=real`，行可见 |
| 单 UPDATE | `create_tx_id=0`，`commit_tx_id=None` | `create_tx_id=real`，`commit_tx_id=real` |
| 单 DELETE | `is_deleted()=true` 但 `commit_tx_id=0` | `is_deleted()=true` 且 `commit_tx_id=DELETED_TX_ID` 正常路径 |
| 重复 PK 失败 | 行被写入但未 commit；`tx_manager` 不知道此事务 | 行被写入（executor 抛错前），随后 abort 清理 index entry；`active_tx_ids` 移除 |
| 连续 DML | `create_tx_id` 全部 0 | `create_tx_id` 单调递增（来自 `TransactionId::allocate()`） |
| WAL 中 BeginTxn/CommitTxn | 每 DML 写 2 份（executor + tx_manager? 不，tx_manager 没被调用） | 每 DML 写 1 份（仅 tx_manager） |

## 兼容性

- **MVCC 单元测试**（`tests/mvcc_commit_test.rs` / `tests/mvcc_abort_test.rs` / `tests/mvcc_record_test.rs`）直接构造 `InsertExecutor` 等，传 `tx_id` 显式，**不走 pipeline** — 本 change 不影响
- **e2e 测试**（`tests/e2e_test.rs` / `tests/pipeline_test.rs`）走 `Database::execute_sql` — **正是本 change 目标场景**，预期全部通过
- **Recovery 测试**（`tests/recovery_test.rs` / `tests/recovery_e2e_test.rs`）走 WAL 读路径 — BeginTxn/CommitTxn 仍由 `tx_manager` 写，WAL 格式不变
- **WAL 文件格式**：BeginTxn/CommitTxn record 的 schema 不变（仅 tx_id 字段从 0 变成非零），旧 db 文件回放不会破坏

## 风险与缓解

| 风险 | 严重度 | 缓解 |
|---|---|---|
| `create_executor_from_plan` 签名变化破坏内部调用 | 中 | 仅 `pub(crate)`，外部 crate 不依赖；调用点全部在 pipeline.rs 内 |
| DML 失败后 abort 抛错导致 state 错乱 | 中 | abort 失败时返回错误响应（最坏情况：index 未清理，下次启动 recovery 处理） |
| 并发场景下 `tx_manager.begin()` 性能 | 低 | `TransactionId::allocate()` 已是 AtomicU64（K16），5.1 ns/op，可忽略 |
| 删除 mark_deleted 与 commit 顺序 | 中 | 现有 DeleteExecutor 在 mark_deleted 后才删 index；commit 在 abort 之后无法影响已 mark_deleted 的行（语义 OK） |

## 不做（Non-goals）

- 不实现显式 `Database::begin/commit/rollback` 公开 API（属 MS07-T04）
- 不修改 SELECT 路径的 transaction 处理（SELECT 无 DML 副作用，不需要 tx）
- 不修改 `TransactionManager` 自身（其 API 已正确）
- 不优化 Group Commit / fsync 频率（属 MS08 范围）
