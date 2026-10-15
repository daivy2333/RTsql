# Proposal: table 渲染大单元格截断（ISS15）与组合面 e2e 补齐

- Date: 2026-10-14
- Status: approved（2026-10-14 用户批准）
- 来源：2026-10-14 用户实测评测（ISS15，台账 `.agents/issues/ISS15-table-renderer-panics-on-64k-cell.md` / R43）+ 同批组合空格扫描 + 用户裁定「issue 和这些 e2e 的修复一起规划到一个 change」

## Why

ISS15（open）：`--format table`（TTY 默认格式）渲染行集时列宽取各列最大单元格字符数且未封顶，单元格 ≥65536 字符触发 Rust std::fmt 动态宽度上限 panic（`Formatting argument out of range`，进程 exit 101）。影响 SELECT 与 stats/sample/profile 全部 table 渲染面；JSON/CSV/TSV 不受影响。根因已定位：`src/cli/render.rs:85` + std 层最小复现（rustc 1.90.0，阈值恰 65536）。

同一评测的组合空格扫描确认四组「特性组合零测试见证」：

| # | 组合 | 现状 |
|---|---|---|
| 1 | 大值 × csv/tsv/table 渲染 | ≥65536 值仅 `tests/e2e_test.rs` E8 json 臂与库内 serde 断言覆盖；csv/tsv/table × 大值零覆盖 |
| 2 | import × 加密文件库 | `--key` 通道代码存在（lifecycle.rs key 穿线 → `open_with_key`），零见证 |
| 3 | 分析子命令 × 非默认运行形态 | `--readonly` 拒绝矩阵（readonly_session_test.rs:611-616）缺 stats/sample/profile；分析子命令 × 加密库零见证 |
| 4 | 大值 × 分析子命令 | stats 的 String 列 min/max 携带原值（lifecycle.rs:1239）——用户实测正是此路径 panic |

用户裁定（2026-10-14 原话）：「我们把issue和这些e2e的修复一起规划到一个change里面」。

## What Changes

1. **修复**（唯一产品代码变更，`src/cli/render.rs::render_table`）：列宽封顶 + 超限单元格省略号截断。上限常量 64 显示字符；超限单元格显示前 61 字符 + `...`；表头与数据格同规则；按字符（chars）截断，多字节安全。json/csv/tsv 保真逐字节不变。单点修复覆盖主命令与三个分析子命令（lifecycle.rs:1108 `render_rows` 委托 `render::render`）。
2. **见证补齐**（四处测试扩展，无产品代码变更）：
   - `tests/e2e_test.rs` E8 扩展：大值 × 四格式矩阵（csv/tsv 保真、table 截断 exit 0）
   - `tests/e2e_test.rs` E9 扩展：大值 × stats/sample/profile（用户实测 panic 路径）× table/json
   - `tests/e2e_test.rs` E7 扩展：import × 加密库（对钥成功数据可查、错钥 exit 5）+ stats × 加密库
   - `tests/readonly_session_test.rs` 拒绝矩阵扩展：+stats/sample/profile（exit 2 点名 flag）
3. **文档同步**：README（双语）与 `.agents/skills/rtsql-docs/SKILL.md` 的 table 输出截断说明。

Delta specs：`specs/cli-noninteractive-shell/spec.md`——「输出格式四态」MODIFIED（截断语义）+ 2 ADDED（组合面锁定）。

## Non-goals

- e2e journey 层缺席但集成层已有见证的项（TRUNCATE、ALTER TABLE、SIGINT/SIGTERM 优雅停机、WAL 自动水位与 `RTSQL_WAL_CHECKPOINT_BYTES`、completions、readonly 完整旅程、schema/list）——分层选择维持现状，本 change 不提升到 journey 层。
- TTY `is_terminal()` 默认 table 分支——子进程管道恒非 TTY，自动化结构性不可达（用户已接受）；渲染逻辑经显式 `--format table` 覆盖。
- 系统性四维交叉穷举审计（运行形态 × 命令面 × 数据形态 × 失败路径）——未承诺方向，另行决定。
- json/csv/tsv 的任何截断——机器消费契约，截断即数据损坏。
- `--readonly` × 未列举子命令的逐一穷举（同一通用拒绝臂覆盖，矩阵见证代表面）。

## 显式接受的缺口

- TTY 默认格式选择分支无自动化见证（结构性不可达，上表 Non-goals）。
- journey 层与集成层的分层差异维持现状。

## 默认假设（集中决策项——已取默认值闭合，非 TBD；用户审计可调整）

- **A1** 截断上限 = 64 显示字符（含省略号：61 内容字符 + `...`）。备选 32/80，改动为单常量。
- **A2** 表头与数据格同规则封顶——超长列名同样降级，彻底关闭本类 panic。
- **A3** 省略号 = 3 个 ASCII `.`。
- **A4** 仅 table 格式截断。
