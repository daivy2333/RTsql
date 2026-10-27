# `ADD COLUMN` 与在途窄化 DDL 并发：发布用的活布局来自已被加列改过的布局，进程内读不再自愈

- Status: closed（2026-10-27 fixed → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`）
- Filed: 2026-10-26
- Source: Act Response 的未解决问题（Issue 候选①）与遗留 Minor M1，归档变更 `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure` Iteration 006 Cycle `iterations/006-ddl-commit-point-unified-model/000-initial.md`（Plan Review `accepted`；该条明确不修、只报告不登记）
- Environment: 该 Act Response 记录工作区 HEAD `e9cb9d3`，执行日期 2026-10-26。Response 未单列平台与工具链版本。

## 缺陷描述

- 预期：同表的 `ALTER TABLE … ADD COLUMN` 与在途 `ALTER TABLE … DROP COLUMN` 交错时，不应让发布出去的活布局取自已被加列改过的布局，也不应让该窗口内的进程内读失去自愈。
- 实际（Act Response 按路径确认）：竞态窗口是窄化改写结束，到提交，再到发布。`alter_add_column` 全程持有 `self.tables.write()`。`alter_drop_column` 在改写期间不持该锁，两者可以交错。发布入口从当前活布局推导窄化后的布局；窗口内这份活布局可能已经被 `ADD COLUMN` 改过。该窗口内进程内读不再自愈。重启后由目录列数收敛。
- 位置：`src/storage/data/table_manager.rs::alter_add_column`（持有 `self.tables.write()`）、`src/storage/data/table_manager.rs::alter_drop_column`（改写期间不持该锁）、`src/storage/data/table_manager.rs::publish_column_narrowing`（经 `narrowed_meta` 从当前活布局推导）。行号以该 Cycle 的记录为准，本条不另标当前行号。
- 根因：Confirmed（Act Response 按锁持有关系与发布推导路径确认）。该轮没有单独的失败用例，具体错读的行值未写入该 Response。
- 与已关闭条目的边界：ISS29 是同表两条窄化 DDL 争用同一个在途布尔量。该 Cycle 写明 ISS29 不覆盖 `ADD COLUMN` 与窄化 DDL 的交错，并且 `ADD COLUMN` 不纳入窄化令牌。不并入 ISS29。ISS25 是窄化提交后、目录发布前崩溃重开的位置性错读，已由同一轮 T6 关闭。不并入 ISS25。

## 影响

该轮没有观测到故障实例。

潜在：窗口内发布出去的活布局与目录列数不一致，进程内读不再自愈。重启后由目录列数收敛。该归档变更写明不修。

## 事件记录

None

## 处置

- 处置指针：空。
- 2026-10-26 登记。发现于已归档变更 `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure` 的 Iteration 006。该 Act Response 写明本条不修、只报告不登记。不把本条标为已修复，也不写入该归档目录。
- 2026-10-27 关闭。原因：同表 `ADD COLUMN` 在窄化令牌持有期间等待，发布安装的活布局取自取得令牌时捕获的布局，由归档变更 Iteration 001 任务 T2 处置。该 Cycle 的 Plan Review 为 `accepted`。处置指针：fixed → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`。

## 证据

- `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/006-ddl-commit-point-unified-model/000-initial.md`：Act Response「遗留 Minor 问题」M1、「未解决问题 / 交办事项」候选①、「Deviations from Plan」第 5 条；Plan Context「Risks and Notes」同名候选。Plan Review 为 `accepted`，未要求修复本条。
- 未知：没有单独失败用例；错读的具体行值未记录；交错是否在每次并发下都发生，该 Response 未测量。
- 2026-10-27 关闭依据：`openspec/changes/archive/2026-10-26-ms40-add-column-catalog/iterations/001-inflight-narrowing-publish-layout/000-initial.md` 的 Plan Review 为 `accepted`（T2）。
