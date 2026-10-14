# ISS14 超大 DEFAULT 建表失败报「Page full」不指名默认值过大

- Status: open
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P2-12）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

DEFAULT 字面量超过 catalog 列行页内容量时建表失败，报错为 `Execution error: Page full`——存储层措辞，不说明是「默认值过大」，使用者无从据此修正 DDL。文档（SKILL.md DEFAULT 字面量 ~4KB catalog 页内容量约束，MS27 T8）已承认该边界，但错误面未承载。与 ISS08 同场景叠加：文案误导之外还有残留半表后果。

预期：报错点名 DEFAULT 值超出约束（可操作的拒绝面）。实际：泛化 Page full。

## 影响

使用者误判为引擎/页损坏类错误；修正路径（缩短 DEFAULT）不可从报错得知。功能拒绝本身正确（零数据损坏）。

## 事件记录

- 2026-10-12：外部评测实测发现（评测方注明文档有承认但措辞对使用者误导）。

## 处置

待用户裁定立项（小文案批；建议与 ISS08 的计划期预检方向合并处置——预检先行拒绝时文案自然点名 DEFAULT，两问题一并收敛）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P1-4/P2-12 段）；复现：`CREATE TABLE t (id INTEGER, pad TEXT DEFAULT '<5000×x>')`。
