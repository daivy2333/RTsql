# `dump` 对非整数 `UNIQUE` 列无条件渲染 ⇒ 该类旧表的 dump 不可恢复

- Status: open
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（MS36 的 M 系列观察之一）+ Plan Review 的判为 Minor / 非阻塞（并明确「这不是缺陷」），change `2026-10-24-ms36-legacy-format-and-dump-compat` Iteration 001 Cycle `000-initial.md`（Review Result `accepted`）；该形态由 MS36 的 `design.md` D9「可选改进」显式记录
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql `rtsql` CLI 的 `dump` / `restore` 路径（`src/cli/`）；登记时工作区 HEAD `0a08b14`（ahead of origin 9），产品改动未提交

## 缺陷描述

- 现状：`dump` 对含**非整数 `UNIQUE` 列**的表**无条件渲染** `UNIQUE` ⇒ 产出的 dump 在新库 `restore` 时被**计划期拒绝**（`UNIQUE (INT columns only)`）⇒ 「旧库 → dump → 新库 restore」在该类表上**走不通**。
- 不对称点：`dump` 的 DDL 渲染对 `PRIMARY KEY` **有**类型条件（`create_table_sql` 仅在主键列声明类型为 INTEGER 时输出 `PRIMARY KEY`），对 `UNIQUE` **没有**同型条件。
- 位置：`src/cli/` 的 `dump` DDL 渲染路径（`create_table_sql` 的 `PRIMARY KEY` 类型条件分支；`UNIQUE` 分支缺同型条件）与 `restore` 的计划期校验（`NON-INTEGER PRIMARY KEY` / `UNIQUE (INT columns only)`）。
- 根因：Confirmed（既有行为；MS36 的 `design.md` D9 明确记为**可选改进**，并明确「这不是缺陷」——既有行为已在 README 记录）。
- 触发条件：被 dump 的表含**非整数类型**的 `UNIQUE` 列。

## 影响

- 当前：该类旧表的**往返恢复**不可用；用户必须**人工**按迁移路径改 dump 的 DDL（MS36 已保证该场景**拒绝明确、迁移路径有文档**）。
- 与已验收合同的一致性：`legacy-format-compat` 的范围外拒绝面（计划期 `UNIQUE (INT columns only)` + 退出码 3 + 迁移路径）**已覆盖**该场景 ⇒ 不构成规格违背。
- 处置形态（MS36 的 D9 已给出）：参照既有的 `PRIMARY KEY` 渲染规则，把非整数 `UNIQUE` 也按同样规则渲染（仅在 `UNIQUE` 列声明类型为 INTEGER 时输出 `UNIQUE`）⇒ 消除该不对称。

## 事件记录

- 2026-10-25（MS36 收尾）：Plan Review 判为 Minor / 非阻塞，并在 `design.md` D9 明确记为**可选改进（不在该 change 范围）**——「这不是缺陷（既有行为已在 README 记录），因此不进入 MS36 的 Acceptance；作为可选改进登记，由后续承载 change 决定」。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在 **`dump` 的 DDL 渲染规则**（`src/cli/` 的 `create_table_sql`）→ 建议独立 change 承载，修复形态即 D9 给出的「按 `PRIMARY KEY` 的同型规则条件渲染 `UNIQUE`」；改动会**改变 dump 的产物内容**（旧库 dump 的 DDL 文本）⇒ 属**功能语义变更**，需要独立验收，并须复核与 `legacy-format-compat` 拒绝面的关系（渲染条件化后，这类表是否从「范围外」变成「支持范围内」）。
- 与 ISS32 的边界：本条是**dump 产物生成侧**的渲染规则，ISS32 是**restore 执行侧**的提交语义，根因不同，**不合并**。

## 证据

- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/design.md`（D9「可选改进（不在本 change 范围）」——现状、参照规则、修复形态与「这不是缺陷」的判定）；`openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`（Plan Review 的 M 系列判定）。
- 相关行为规格与文档：`openspec/specs/legacy-format-compat/spec.md`（范围外拒绝面与迁移路径）、`tests/fixtures/legacy/README.md`（旧库 dump / restore 的迁移路径说明）。
