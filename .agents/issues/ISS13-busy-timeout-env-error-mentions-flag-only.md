# ISS13 RTSQL_BUSY_TIMEOUT 非法值报错只引用 flag、不指明 env 变量

- Status: open
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P2-9）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

经环境变量设置非法值时，报错只指向 flag：`RTSQL_BUSY_TIMEOUT=abc` → `error: invalid value 'abc' for '--busy-timeout <MS>': invalid digit found in string`（exit 2）——用户可能并未使用 flag 而是设置了 env，报错未提及 `RTSQL_BUSY_TIMEOUT`，排障指向错误入口。来源：clap env 值校验错误模板（env 通道复用 flag 参数错误）。

预期：env 来源的非法值报错点名环境变量。实际：统一渲染为 flag 参数错误。

## 影响

env 设置者的排障入口误导（可能去查不存在的命令行参数）。退出码与拒绝语义正确。

## 事件记录

- 2026-10-12：外部评测实测发现。

## 处置

待用户裁定立项（小文案批；clap 错误定制或 open 前自校验 env 通道并点名；与 `RTSQL_KEY`/`RTSQL_WAL_CHECKPOINT_BYTES` 同族 env 校验面一并核对）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P2 批表）；复现：`RTSQL_BUSY_TIMEOUT=abc rtsql <db> "SELECT 1"`。
