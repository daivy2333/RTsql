# 进程内 close→reopen 数据库测试配方（集成测试）

- Status: active
- Last validated: 2026-09-23
- Environment: Linux x86_64（WSL2）、Rust/Cargo、tokio 多线程 test runtime；RTsql `Database::open_with_isolation` / `Database::close()`（advisory 独占文件锁 + checkpoint）
- Source: change `2026-09-23-ms17-t02-defect-closeout` Iteration 001/002 Act Response（`archive/2026-09-23-ms17-t02-defect-closeout/iterations/*/000-initial.md`）；见证测试 `tests/isolation_level_test.rs`（9 passed，2026-09-23）

## 适用范围

集成测试（`#[tokio::test]`）在同一进程内对同一库文件先写入、再关闭、再重新打开断言持久化/恢复/可见性行为。适用：WAL 恢复、checkpoint 重开、隔离级别重启语义、（后续）加密 with/without key 重开。不适用：跨进程锁冲突场景（需真实第二进程，见 `tests/database_file_lock_test.rs` 先例）；崩溃模拟（用进程终止或手工构造 WAL，不走本配方）。

## 前置条件

- 库文件放 `tempdir()` 下，路径用 `dir.path().join("x.db")`（**不能把 tempdir 目录本身当 db 文件路径**——`IsADirectory` 错误，Iteration 002 位点单测首跑实证）。
- 不需要预建表结构以外的环境；`open_with_isolation` 对不存在路径静默建库。

## 操作步骤

1. 第一会话放**显式作用域**内，块尾显式调用 `db.close().await.unwrap()`：

```rust
let dir = tempdir().unwrap();
let path = dir.path().join("rc-reopen.db");
{
    let db = Database::open_with_isolation(&path, IsolationLevel::ReadCommitted).await.unwrap();
    // 建表 + 提交写入 ...
    db.close().await.unwrap();
} // dropped：释放 advisory 文件锁
```

2. 作用域外重新 `open_with_isolation`（同 path），执行断言语句。
3. RC 场景重开后**直接** `SELECT` 即可——checkpoint 位点已携带 tx watermark（MS17-T02 Iter002 修复），已提交行立即可见，无需任何 DML 抬水位（修复前的 scratch workaround 已于 2026-09-23 移除，勿再模仿 `tests/` 历史版本中的 scratch 块）。

## 验证

- 成功判据：重开后语句返回重启前已提交行；全套件 `cargo test --test isolation_level_test` → `9 passed; 0 failed`（2026-09-23，`rc_reopen_sees_committed_rows_without_new_dml` 为 S1 直接见证）。
- 判定用测试自身退出码与断言输出，无判定层。

## 失败处理

- 重开报 `DatabaseLocked`：第一会话绑定仍存活——Rust 变量遮蔽不提前 drop，advisory 锁不随遮蔽释放（Iteration 001 Deviations 1a 实证）。修法：显式作用域 + 块尾 `close()`，确认无外部变量再绑定。
- RC 重开后查不到已提交行：先确认第一会话确实调用了 `close()`（未 close 则 catalog 页可能未落盘且 WAL 未截断）；排除后仍复现属可见性/水位缺陷，走 Issue 流程，不得在测试内插 DML 造假见证（Iteration 002 T7 Stop-when 原文）。
- 断言撞预存形状/文案缺陷：按 change 纪律登记 Issue 候选或走校准裁定，不在本配方内绕过。

## 回滚

不适用（测试配方，无产品状态变更；误用产生的只有 tempdir 残留，随测试结束清理）。

## 证据

- `archive/2026-09-23-ms17-t02-defect-closeout/iterations/001-visibility-summary-closeout/000-initial.md`（Deviations 1：DatabaseLocked 首跑 + RC 夹具两坑实证；Experience Candidates Runbook 行）
- `archive/2026-09-23-ms17-t02-defect-closeout/iterations/002-restart-watermark/000-initial.md`（新 e2e 用本配方 GREEN；scratch workaround 移除后仍绿——IsADirectory 夹具坑同记 Deviation 3）
- 见证代码：`tests/isolation_level_test.rs::rc_reopen_sees_committed_rows_without_new_dml`、`rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart`
