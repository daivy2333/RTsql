# Design

## 现状（Current-State Evidence，Plan 2026-10-14 实际读码）

- **渲染单点**：`src/cli/render.rs:62-104` `render_table`——header + rows 经 `value_to_text` 文本化；`widths[i]` = 列内最大 `chars().count()`（含表头，:70-79）；`format!("{:<w$}", cell, w = widths[i])`（:85）。std::fmt 动态宽度 ≥65536 即 panic（ISS15 std 层最小复现，rustc 1.90.0）。全 `src/` grep 无其他动态宽度消费点——本类 panic 单点闭合。
- **分析子命令共用同一渲染点**：`src/cli/lifecycle.rs:1103-1113` `render_rows` 委托 `render::render`（`use super::render::{render, QueryPayload}`，:6）——修复 `render_table` 即同时覆盖主命令 SELECT 与 stats（:1117）/sample（:1342）/profile（:1419）。
- **用户实测 panic 路径**：`column_min_max`（lifecycle.rs:1239）对 String 列 min/max 携带原值 → `compute_stats_rows`（:1175）→ `render_rows` → `render_table`。
- **import × 加密通道已存在**：`import_csv`（lifecycle.rs:680）签名含 `key: Option<&str>`（:686）→ `execute_command_inner_mode(&db_path, key, false, memory, lock_wait, …)`（:712）→ `open_with_key`（mod.rs:609-611 注释确认 readonly/key 分派）。零产品代码缺口，纯见证缺口。
- **分析子命令 × 加密通道已存在**：stats/sample/profile 经 `execute_command_inner(&db_path, key, lock_wait, …)`（lifecycle.rs:1129 等），key 已穿线。
- **`--readonly` × 子命令拒绝臂已存在**：`src/cli/mod.rs:317-322` match 通用臂 `Some(_) if args.readonly` → Usage exit 2（MS26「proposal 假设 10」：readonly 只属主命令）。stats/sample/profile 不在 `lifecycle_db_target`（:438-444 刻意排除），但仍命中通用臂。拒绝矩阵测试 `tests/readonly_session_test.rs:606-640` 仅含 schema/dump/delete/restore/import 五项 + new 早拒——缺 stats/sample/profile。
- **e2e 夹具**：`tests/e2e_test.rs`——`fixture()`（TempDir + `db/` 目录 + RTSQL_HOME）、`run_cli`/`wait_cli`（stdout/stderr 并发读线程防管道死锁，R31；60s 兜底）、`run_cli_stdin`（大 SQL 经 restore stdin 承载）、`run_cli_env`。E7 加密旅程 :998、E8 大值旅程 :592（1MB 值 → REPLACE 后 2MB，仅 json 断言）、E9 分析旅程 :669（仅小值）。
- **基线**：1994 tests / 0 failures / 2 ignored（2026-10-14 change 归档记录，HEAD fbb5c54；此后仅 `.agents/` 文档变更，代码未动，采信）。

## 目标行为

### 截断规则（精确语义）

1. `render.rs` 模块内新增常量 `CELL_WIDTH_CAP: usize = 64`（A1，proposal 默认假设）。
2. `render_table` 在 `value_to_text` 文本化之后、宽度计算之前，逐单元格处理（表头同规则，A2）：`chars().count() > CELL_WIDTH_CAP` → 保留前 `CELL_WIDTH_CAP - 3` 个 char + `"..."`（A3；结果恰 64 个 char）；≤ 上限原样。
3. `widths[i]` 按截断后文本计算（最大 ≤ 64）——`format!` 动态宽度恒 < 65536，panic 类结构性关闭。
4. json / csv / tsv 分支不经过截断（A4）——`render_delimited` 与 serde_json 路径零接触。
5. 截断按 `char` 迭代（非字节），多字节字符不产生部分字符。

## 关键选择与替代方案

- **D1 固定常量 vs 终端宽度**：终端宽度需 TTY 查询，破坏 `render` 纯函数契约（模块头声明「无 IO、无 env、无时钟」）与测试确定性；one-shot 管道场景多数非 TTY → 固定常量。
- **D2 截断 vs 仅钳制列宽**：把 `widths[i]` 钳到 65535 只消除 panic，输出仍是不可用的 65535 宽单行；截断同时解决崩溃与可用性 → 截断。
- **D3 管道格式不截断**：json/csv/tsv 是机器消费契约（导出/管道/程序解析），截断即数据损坏 → 保真；大值保真由 E8 既有 json 臂与本次 csv/tsv 臂锁定。
- **D4 表头同规则**：豁免表头则超长列名保留同类 panic → 均匀封顶（列名 >64 字符在 table 显示层降级，DDL/catalog 语义不受影响）。
- **D5 chars 截断**：字节截断可能劈开 UTF-8 多字节字符（CJK/emoji）→ 按 char。

## 责任边界

- **修改**：`src/cli/render.rs`（常量 + `render_table` 截断步）——唯一产品代码变更。
- **不修改**：`value_to_text`（csv/tsv 共享，截断会泄漏到管道格式）、`render_delimited`、json 分支、`render` 分发、lifecycle 分析命令、import/加密/readonly 语义（后三者为纯见证）。
- **禁止**：为截断引入配置项/CLI 参数/env 通道（YAGNI，A1 是唯一 knob，改常量即可）。

## 见证落位

| 见证 | 位置 | 断言要点 |
|---|---|---|
| 截断单元矩阵（RED→GREEN） | `src/cli/render.rs` tests 模块（lib 单测） | ≤64 原样 / 65 截断恰 64 / 100,000 字符不 panic 且 `...` 结尾 / CJK 多字节边界 / 表头超限 / csv+tsv+json 同输入保真回归锁 |
| 大值 × 四格式 | `e8_large_value_journey` 扩展 | csv/tsv 读回 body 全值保真（长度逐字节）；table exit 0、数据行恰 64 字符 `...` 结尾、行数不变 |
| 大值 × 分析子命令 | `e9_analytics_journey` 扩展 | 70,000 字符 String 列上 stats/sample/profile × table exit 0（截断见证）；stats × json min/max 全值（`rows` 中 s 列行 min/max 长度断言） |
| import × 加密 + stats × 加密 | `e7_encryption_journey` 扩展 | 夹具写 CSV → 预建 s2 → 对钥 import exit 0 + COUNT 可查；错钥 exit 5；`--key pw stats enc s --format json` exit 0 |
| `--readonly` × 分析子命令 | `cli_readonly_rejected_with_lifecycle_subcommands` 矩阵扩展 | 三 vec 追加，exit 2 + stderr 点名 `--readonly` 与 `main command`（复用既有断言式） |

## 风险与既有断言冲突面

- 既有测试可能存在 >64 字符单元格的 table 输出断言（如错误面/表头渲染用例）。T1 验证步骤定向跑 render 相关与 e2e 目标；命中者为预期行为变化（截断语义生效），按白名单校准并记入 Act Response，不视为回归。
- e2e 大值读回体量：csv/tsv 臂各 ~2MB stdout（E8 REPLACE 后 body 2,000,000 字符）——夹具并发读已防管道死锁（R31 先例，E8 json 臂 1MB 已验证）。
- E9 新增大值行不触碰既有 20 行小值表的断言（新建独立表 `big_stats`，避免既有 loose 断言漂移）。
