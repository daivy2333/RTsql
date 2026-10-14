# ISS12 schema/dump DDL 渲染类型短名（INTEGER→INT、TEXT→STRING）

- Status: open
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P2-8）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

`rtsql schema` 与 `dump` 输出的 DDL 列类型为引擎内部短名而非声明类型：声明的 `INTEGER`/`TEXT` 渲染为 `INT`/`STRING`（`BOOL` 同型）。来源：`src/cli/lifecycle.rs:1025-1034` `column_type_sql` 的映射表。往返仍可达（解析器接受短名），dump→restore 不失数据——属类型名保真度问题而非正确性问题。

预期：DDL 输出与声明类型名一致（或文档明示规范化规则）。实际：静默改写为短名。

## 影响

以 dump 为互操作/审计/迁移输入的消费者看到非标准类型名；易与「声明类型丢失」混淆。数据无损。

## 事件记录

- 2026-10-12：外部评测实测发现（评测方同时确认往返可达）。

## 处置

待用户裁定立项（小文案批；映射表改渲染声明名，或文档声明规范化语义——注意既有 dump→restore 测试与解析器接受面）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P2 批表）；复现：`rtsql dump <db>`（含 INTEGER/TEXT 声明表）。
