# `ADD COLUMN` 在列数刷盘期间被放弃：补偿不执行，列数可能已上调并落盘

- Status: closed（2026-10-27 fixed → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`）
- Filed: 2026-10-26
- Source: Act Response 的未解决问题（Issue 候选②）与遗留 Minor M2，归档变更 `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure` Iteration 006 Cycle `iterations/006-ddl-commit-point-unified-model/000-initial.md`（Plan Review `accepted`；该条明确不修、只报告不登记）
- Environment: 该 Act Response 记录工作区 HEAD `e9cb9d3`，执行日期 2026-10-26。Response 未单列平台与工具链版本。

## 缺陷描述

- 预期：`ALTER TABLE … ADD COLUMN` 在列数刷盘期间被放弃（语句 future 被丢弃）时，目录列数应回到语句执行前的值，被放弃的语句不应留下已经落盘的列数上调。
- 实际（Act Response 确认仍成立）：T7 的补偿只覆盖错误路径。语句 future 在 `update_table_column_count_ddl` 刷盘期间被丢弃时不执行补偿。列数可能已经上调并落盘。该被放弃的语句会留下目录面变更。
- 位置：`src/storage/data/table_manager.rs::alter_add_column`、`src/storage/data/table_manager.rs::compensate_column_add`（只挂在错误路径）、`src/storage/catalog.rs::update_table_column_count_ddl`（刷盘）。行号以该 Cycle 的记录为准，本条不另标当前行号。
- 根因：Confirmed（Act Response 按补偿的挂载点确认：放弃路径不调用补偿）。「列数可能已上调并落盘」沿用该 Response 的措辞，未升级为必然发生。该轮没有单独的失败用例。
- 与已关闭条目的边界：ISS28 是失败的 DDL（语句返回错误）其目录面变更随后被刷盘落地。T7 补偿的是这条错误路径，ISS28 已关闭。本条是刷盘期间丢弃语句 future，不走错误路径。不并入 ISS28。

## 影响

该轮没有观测到故障实例。

潜在：被放弃的 `ADD COLUMN` 留下已经上调并落盘的列数，目录面与语句结果不一致。该归档变更写明不修。是否每次放弃都会落盘，该 Response 写的是「可能」，未测量。

## 事件记录

None

## 处置

- 处置指针：空。
- 2026-10-26 登记。发现于已归档变更 `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure` 的 Iteration 006。该 Act Response 写明本条不修、只报告不登记。不把本条标为已修复，也不写入该归档目录。
- 2026-10-27 关闭。原因：语句 future 在列数刷盘成功之后被丢弃时，列数与本次列行被写回并落盘，由归档变更 Iteration 000 任务 T1 处置。该 Cycle 的 Plan Review 为 `accepted`。处置指针：fixed → `openspec/changes/archive/2026-10-26-ms40-add-column-catalog`。

## 证据

- `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/006-ddl-commit-point-unified-model/000-initial.md`：Act Response「遗留 Minor 问题」M2、「未解决问题 / 交办事项」候选②；Plan Context「Risks and Notes」同名候选，以及 Non-goals 中「`ADD COLUMN` 在列数刷盘期间被放弃时的目录面残留」。Plan Review 为 `accepted`，未要求修复本条。
- 未知：没有单独失败用例；列数上调并落盘的触发频率未测量。
- 2026-10-27 关闭依据：`openspec/changes/archive/2026-10-26-ms40-add-column-catalog/iterations/000-add-column-flush-abandon/000-initial.md` 的 Plan Review 为 `accepted`（T1）。
