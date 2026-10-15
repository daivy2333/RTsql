# Tasks

> Change: 2026-10-14-iss15-table-truncation-and-combination-gaps
> Status: approved（2026-10-14 用户批准；Gate 2 PASS，Iteration 000 Cycle 000-initial ready）

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T1 | `render_table` 列宽封顶 + 超限单元格省略号截断（lib 单测边界矩阵先行 RED→GREEN） | R1 | 000 |
| T2 | e2e 大值 × 四格式（E8 扩展）+ 大值 × stats/sample/profile（E9 扩展） | R2 | 000 |
| T3 | e2e import × 加密库（对钥/错钥）+ stats × 加密库（E7 扩展） | R3 | 000 |
| T4 | `--readonly` 拒绝矩阵 + stats/sample/profile（readonly_session_test 扩展） | R3 | 000 |
| T5 | 文档同步：README 双语 + SKILL.md table 截断说明 | R1 | 000 |

Requirement 权威文本：`specs/cli-noninteractive-shell/spec.md`（本 change delta）——R1 = MODIFIED「输出格式四态」；R2 = ADDED「大值行集输出组合（锁定）」；R3 = ADDED「import 与分析子命令非默认运行形态（锁定）」。

## Iteration Plan

### Iteration 000: ISS15 修复与组合面见证补齐

- Tasks: T1, T2, T3, T4, T5
- Depends on: None
- Stable baseline: 任意单元格长度下 table 渲染 exit 0 且超限截断（SELECT 与 stats/sample/profile 同面）；四组组合空格各有自动化见证（大值×四格式、大值×分析子命令、import×加密、分析子命令×readonly/加密）；文档同步；全量零回归
- Verification boundary: 全量 `cargo test` 一次，于 T1–T4 全部落地后执行（Iteration Gate）；T5 文档-only 不再重跑。任务内定向验证随时进行（`cargo test --lib render`、`cargo test --test e2e_test`、`cargo test --test readonly_session_test`）
- Diagnostic boundary: `src/cli/render.rs` 与四处测试文件（`tests/e2e_test.rs`、`tests/readonly_session_test.rs`、`README.md`、`.agents/skills/rtsql-docs/SKILL.md`）
- Non-goals: proposal Non-goals 全项（journey 层缺席项、TTY 默认分支、四维穷举审计、管道格式截断）

### 平衡审计

单 Iteration 承载五任务：T1 是唯一产品代码变更（约一个常量 + 一段截断逻辑 + 单测矩阵），T2–T4 是其验收见证与独立组合空格的纯测试扩展，T5 文档收尾。五者共享同一验证边界（CLI 输出面 + e2e/单测两层）与单一故障域（`render.rs` 渲染语义），合成一个可独立验收的阶段成果「ISS15 修复 + 组合面清偿」；不过碎（无单项不成结果的拆分）也不过重（无多个独立验收结果混装）。全量预算一次，符合 Iteration 预算纪律。
