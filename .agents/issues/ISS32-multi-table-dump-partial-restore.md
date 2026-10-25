# 多表 dump 中靠后语句被范围外拒绝时，靠前语句已生效（既有 restore 合同的直接后果）

- Status: open
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（MS36 的 M 系列观察之一）+ Plan Review 的判为 Minor / 非阻塞，change `2026-10-24-ms36-legacy-format-and-dump-compat` Iteration 001 Cycle `000-initial.md`（Review Result `accepted`）；该形态由 MS36 的 `design.md` D4「已知边界」显式记录
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `rtsql` CLI 的 `dump` / `restore` 路径（`src/cli/`）；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 现状（**既有合同，不是新缺陷**）：`restore` **逐条执行并各自 auto-commit**（`Executor::execute` 的隐式提交）⇒ 一份**多表** dump 中若**靠后**语句被范围外拒绝，**靠前**语句**已生效**。
- 合同依据：既有规格 `cli-noninteractive-shell` 已把「逐条 auto-commit、失败即停、失败前语句已生效」写成合同；MS36 的 `design.md` D4 决定**不改动** restore 的既有提交语义（不把 restore 改成「先全量校验再执行」的事务化语义）。
- 位置：`src/cli/` 的 `restore` 执行路径与 `src/executor/` 的隐式提交（逐条 `Executor::execute`）。
- 根因：Confirmed（既有 restore 合同的直接后果，MS36 的 D4「已知边界（明确记录，不在本 change 修复）」）。
- 触发条件：一份包含**多张表**的 dump，其中**靠后**的语句落在**范围外**输入上（计划期 `NON-INTEGER PRIMARY KEY` / `UNIQUE (INT columns only)`，或执行期 `NOT NULL constraint violation`）⇒ 范围外拒绝显式发生（错误文本点名拒绝原因、给出「第 k 条 / 共 n 条」、退出码 3），但前段语句**已提交**。

## 影响

- 当前：用户看到**显式拒绝 + 退出码 3**，但目标库处于**部分已恢复**状态；既有合同已把这一形态写成「失败前语句已生效」，因此**不构成静默的数据错误**。
- 与既有规格的一致性：`legacy-format-compat` R 要求「SHALL NOT 静默地部分恢复后报告成功」——当前行为是**显式报错 + 非零退出码**，并说明「previous statement(s) were committed」⇒ **不违反**该条 ⇒ 本条**不是**规格违背，而是一条**已知的范围外边界**。
- 潜在：若未来要让 restore 具备原子性（或全量预检），属**功能语义变更**，需要独立 change 与独立验收。

## 事件记录

- 2026-10-25（MS36 收尾）：作为 MS36 的范围外观察被 Plan Review 判为 Minor / 非阻塞，并由 MS36 的 `design.md` D4「已知边界（明确记录，不在本 change 修复）」显式记录；该 change 只保证该场景**拒绝明确、迁移路径有文档**。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在 **restore 的提交语义 / 范围外拒绝面** → 建议独立 change 承载「restore 原子化或全量预检」这一功能语义变更；在该 change 落地前，本条**不**应被当作缺陷修复（既有合同已覆盖其行为）。
- 与 ISS33 的边界：本条是**多表 dump 的部分恢复**（提交语义），ISS33 是**dump 的 DDL 渲染规则**（产物生成侧），根因不同，**不合并**。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/design.md`（D4「范围外一律显式拒绝 + 迁移路径；不改动 restore 的既有提交语义」及其「已知边界」段）；`openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`（Plan Review 的 M 系列判定）。
- 既有合同：`openspec/specs/cli-noninteractive-shell/spec.md`（逐条 auto-commit、失败即停、失败前语句已生效）、`openspec/specs/legacy-format-compat/spec.md`（范围外一律显式拒绝 + 迁移路径；SHALL NOT 静默地部分恢复后报告成功）。
