# ISS10 CLI help 文本泄漏内部任务标记（MS28 T1 / MS26 T1 / proposal 假设 10）

- Status: closed
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P2-6）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

用户可见的 `--help` 输出包含内部开发过程标记：`--busy-timeout` help 含「MS28 T1：」、`--readonly` help 含「MS26 T1：」与「proposal 假设 10」。来源为 clap derive 的 doc comment 直接成为帮助文案：`src/cli/mod.rs:107`、`:112-114`（同类内部编号在 `:279`、`:304-305`、`:570`、`:592` 为纯代码注释不可见，不在缺陷面）。

预期：用户文档面不暴露内部任务编号与提案假设。实际：help 文本携带。

## 影响

产品观感与专业性缺陷；对用户无信息量。未造成功能性影响。

## 事件记录

- 2026-10-12：外部评测 `--help` 实测发现。

## 处置

closed（2026-10-14）——change `2026-10-13-e2e-verification-and-defect-closure` Iteration 004 T6 修复归档。help 用户视角改写（去 MS28 T1 / MS26 T1 / proposal 假设 10 内部标记，内部编号移入普通注释）；`tests/cli_test.rs` 白名单断言锁定。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P2 批表）；本机 `rtsql import --help` / `rtsql --help` 可复现。
