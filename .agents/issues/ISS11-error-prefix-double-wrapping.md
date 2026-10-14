# ISS11 错误消息前缀双层包装（Parse error: Parse error: / Plan error: Parse error:）

- Status: open
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P2-7）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

错误消息前缀嵌套/重复：`Parse error: Parse error: sql parser error: …`（实测）。两层来源叠加：pipeline 层包装一次 `format!("Parse error: {}", e)`（`src/pipeline.rs:113`）+ `PlanError::ParseError` 的 Display 再包装一次 `"Parse error: {}"`（`src/parser/error.rs:74`）；「Plan error: Parse error: …」为同型嵌套形态（Plan 层 Display × Parse 层 Display）。

预期：错误前缀单层、类别准确。实际：同类前缀串叠。既有断言 `src/pipeline.rs:2761` 锁定了双层形态，修复时须一并校准。

## 影响

错误文案可读性与自动化匹配稳定性（依赖前缀做错误分类的消费者受扰）。功能不受影响。

## 事件记录

- 2026-10-12：外部评测实测发现；Explorer 定位两层包装点。

## 处置

待用户裁定立项（小文案批；选定单一包装层，清理另一层，校准既有前缀断言）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P2 批表）；复现：`rtsql <db> "schema t.db"`。
