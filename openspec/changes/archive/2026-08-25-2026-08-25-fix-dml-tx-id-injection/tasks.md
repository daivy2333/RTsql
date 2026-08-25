# 任务清单：fix-dml-tx-id-injection

> 关联里程碑：MS06-T01（MS06 稳定性与正确性收口 / 修 INSERT/UPDATE/DELETE `tx_id=0` 占位注入）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 Iteration：仅一个 Iteration 000

## 1. `src/pipeline.rs::create_executor_from_plan` 签名扩展

- [ ] 1.1 改 `pub(crate) fn create_executor_from_plan(plan: PhysicalPlan, database: &Database)` → `(plan, database, tx_id: Option<u64>)`
- [ ] 1.2 `PhysicalPlan::Insert` 分支：`0,` → `tx_id.expect("DML Insert requires tx_id")`
- [ ] 1.3 `PhysicalPlan::Update` 分支：`0,` → `tx_id.expect("DML Update requires tx_id")`
- [ ] 1.4 `PhysicalPlan::Delete` 分支：`0,` → `tx_id.expect("DML Delete requires tx_id")`
- [ ] 1.5 递归调用（如 `create_executor_from_plan(*node.input, database)`）保持传 `tx_id` 透传
- [ ] 1.6 `cargo build` 通过（无编译错）

## 2. `src/pipeline.rs::execute_inner` DML 事务包裹

- [ ] 2.1 在 DML 路径（`_ => { ... }` 块内，planner 与 cache 之后）判断 `is_dml = matches!(plan, Insert/Update/Delete)`
- [ ] 2.2 `is_dml` 时 `let tx = Some(database.transaction_manager.begin().await);`；非 DML 传 `None`
- [ ] 2.3 从 `plan` 中取出 `table_name`，预先 `database.table_manager.get_table(table_name).await.ok()` 存为 `table_meta_for_abort`
- [ ] 2.4 构造 executor 时把 `tx_id = tx.as_ref().map(|t| t.id())` 传入
- [ ] 2.5 executor 构造失败时：若 `tx` 存在则 `tx_manager.abort(tx, &buffer_pool, &table_meta_for_abort).await`，忽略 abort 自身错误
- [ ] 2.6 executor 执行后按 `response` 类型处理：
  - `Response::Error { .. }` → `tx_manager.abort(tx, &buffer_pool, &table_meta).await`；abort 失败时返回 `format!("Abort failed: {}", e)` 错误
  - `Response::QueryResult { .. }` / `Response::AffectedRows { .. }` → `tx_manager.commit(tx, &buffer_pool).await`；commit 失败时返回 `format!("Commit failed: {}", e)` 错误
- [ ] 2.7 `cargo build` + `cargo clippy` 通过

## 3. `src/executor/insert.rs` 清理冗余 WAL 写入

- [ ] 3.1 删除 `next()` 内 line 63-65 的 `wal.append(WalRecord::BeginTxn { tx_id: self.tx_id })` 块
- [ ] 3.2 删除 `next()` 内 line 129-140 的 CommitTxn 写入 + `append_commit_and_wait` 块
- [ ] 3.3 保留 line 107-115 的 `WalRecord::Insert` 写入
- [ ] 3.4 保留 line 118 的 `tx_manager.record_version(self.tx_id, row_id)` 调用
- [ ] 3.5 `cargo build` 通过

## 4. `src/executor/update.rs` 清理冗余 WAL 写入

- [ ] 4.1 删除 `next()` 内 line 98-100 的 BeginTxn 块
- [ ] 4.2 删除 `next()` 内 line 117-136 的 CommitTxn 写入 + `append_commit_and_wait` 块
- [ ] 4.3 保留 `WalRecord::Update { ... }` 写入
- [ ] 4.4 保留 `tx_manager.record_version(self.tx_id, new_row_id)` 调用
- [ ] 4.5 更新注释（删除 "WAL: BeginTxn (implicit transaction per statement)" 标记）
- [ ] 4.6 `cargo build` 通过

## 5. `src/executor/delete.rs` 清理冗余 WAL 写入 + 新增 record_version

- [ ] 5.1 删除 `next()` 内 line 52-54 的 BeginTxn 块
- [ ] 5.2 删除 `next()` 内 line 100-105 的 CommitTxn 写入 + `append_commit_and_wait` 块
- [ ] 5.3 保留 `WalRecord::Delete { ... }` 写入
- [ ] 5.4 **新增** `self.tx_manager.record_version(self.tx_id, row_id).await` 调用（仅在 row_id 存在时，位置：index.delete 之后、wal.append 之前）
- [ ] 5.5 `cargo build` 通过

## 6. 新增测试 `tests/dml_tx_id_test.rs`

- [ ] 6.1 测试 `test_insert_writes_real_create_tx_id`：通过 `Database::execute_sql("INSERT ...")` 写入，验证 `read_version_header` 拿到 `create_tx_id > 0` 且 `commit_tx_id = create_tx_id`
- [ ] 6.2 测试 `test_update_writes_real_create_tx_id`：通过 SQL UPDATE 改一列，验证新版本的 `create_tx_id > 0` 且 `commit_tx_id` 已设置
- [ ] 6.3 测试 `test_delete_writes_real_create_tx_id`：通过 SQL DELETE 一行，验证 `is_deleted()` 返回 true
- [ ] 6.4 测试 `test_insert_duplicate_pk_aborts_transaction`：重复 PK 插入失败后 `tx_manager.active_transactions()` 不含失败 tx
- [ ] 6.5 测试 `test_consecutive_dml_have_unique_tx_ids`：连续 10 次 INSERT，tx_id 单调递增
- [ ] 6.6 测试 `test_insert_visible_after_commit`：INSERT 后 `SELECT` 能看到（验证 commit_tx_id 真的被设置了）
- [ ] 6.7 `cargo test --test dml_tx_id_test` 全绿

## 7. 全量回归

- [ ] 7.1 `cargo fmt --all` 通过
- [ ] 7.2 `cargo clippy --all-targets -- -D warnings` 通过
- [ ] 7.3 `cargo test --lib` 全绿（基线 113 单元测试 + 现有事务相关测试）
- [ ] 7.4 `cargo test --tests` 全绿（基线 347 集成测试 + 新增 6 个 = 353）
- [ ] 7.5 重点回归 `tests/mvcc_commit_test.rs` / `tests/mvcc_abort_test.rs` / `tests/mvcc_record_test.rs`（直接构造 executor，验证不受影响）
- [ ] 7.6 重点回归 `tests/e2e_test.rs` / `tests/pipeline_test.rs`（走 execute_sql，验证 tx_id 修复生效）
- [ ] 7.7 重点回归 `tests/wal_*_test.rs`（验证 WAL 仍可正常解析与恢复）

## 验收标准

| 标准 | 命令 | 预期 |
|------|------|------|
| `tx_id > 0` after insert | `cargo test --test dml_tx_id_test test_insert_writes_real_create_tx_id` | exit 0 |
| `commit_tx_id` set after insert | `cargo test --test dml_tx_id_test test_insert_visible_after_commit` | exit 0 |
| abort 清理 active list | `cargo test --test dml_tx_id_test test_insert_duplicate_pk_aborts_transaction` | exit 0 |
| 全量回归 | `cargo test --all` | 0 failures |
| 不改 MVCC 单测语义 | `cargo test --test mvcc_commit_test` + `mvcc_abort_test` + `mvcc_record_test` | 0 failures |
| WAL 格式兼容 | `cargo test --test wal_integration_test` + `wal_writer_test` | 0 failures |
| 公共规则符合 | `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` | exit 0 |
