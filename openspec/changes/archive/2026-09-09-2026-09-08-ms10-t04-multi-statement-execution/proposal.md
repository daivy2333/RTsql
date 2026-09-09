# proposal: MS10-T04 多语句执行修复（`;` 分片逐条执行）

## Why

RTsql 当前对多条语句有两个互为表里的缺陷（R18 分析主题 2/3，tasks.md MS10-T04）：

1. **lib 层静默截断**：`parse_stage` 返回完整 `Vec<Statement>`（`src/pipeline.rs:42-48`），但执行只取 `statements.first()`——`pipeline::execute_inner`（`src/pipeline.rs:341`，网络路径经 `Database::execute_sql`，`src/network/handler.rs:27`）与 `pipeline::execute_in_tx`（`src/pipeline.rs:237`，显式事务路径）第二条起被静默丢弃，无任何报错。对任何脚本化调用是直接脚枪。
2. **CLI 临时护栏**：MS10-T01 在 `run_sql`（`src/cli/mod.rs:203-208`）对多语句显式拒绝（exit 3，文案指向 MS10-T04）。spec `cli-noninteractive-shell` R5（多语句显式拒绝·临时护栏）明确「MS10-T04 落地分片执行后本护栏退役」。

CLI 非交互形态（面向 agent/脚本）下，`rtsql db "INSERT ...; INSERT ...; SELECT ..."` 是日常调用形状；没有分片执行，建表+导入、批量写入等最小脚本都不可达。

**用户决策（2026-09-08，本会话）**：

1. **覆盖范围**：CLI 主命令逐条执行；`pipeline::execute`（网络路径）与 `execute_in_tx` 遇多语句从静默截断改为**显式报错**（`Response::Error`）。已核实无任何测试依赖静默截断（协议测试 grep 零命中），单语句行为零变化。
2. **输出语义**：顺序渲染每条语句的结果——DML/DDL 输出受影响行数（现行 R4 语义的自然推广），SELECT 输出查询结果；`json` 格式每条语句输出一个独立 JSON 文档（逐行 JSONL 风格）。
3. **错误定位**：语句序号 + 失败语句文本（如 `statement 2 of 3 failed: ...`）；语法错误保留 sqlparser 自带的 `at Line: X, Column Y` 定位文本。tasks.md MS10 outcome「报错行号」按此口径简化（已批准）。
4. **事务边界**：逐条 auto-commit——每条语句独立事务，第 k 条失败时前 k-1 条已生效，错误信息明确注明；与单语句 CLI、sqlite CLI 一致。整脚本隐式事务不做（PG simple-query 先例方案放弃，DDL 非事务性使 all-or-nothing 不完整）。

## What Changes

- **CLI 分片执行（`src/cli/mod.rs::run_sql`）**：移除多语句护栏，改为逐条循环——对 `parse_stage` 产出的每条语句调用 `plan_stage` + `execute_stage`（均为现有 pub API），逐条渲染并写 stdout（顺序多段）；任一条失败立即停止，返回 `ExitStatus::Sql`，错误信息含序号（第 k/共 n 条）与失败语句文本。
- **plan_cache 键修正**：逐条执行时 `plan_stage` 的键改传**该语句自身的 canonical 文本**（`Statement::to_string()`），不再传完整多语句串——否则每条 SELECT 覆盖同一键，且后续相同多语句串会 cache-hit 直接执行被缓存的最后一条 plan（静默错结果）。单语句路径统一换键；one-shot CLI 进程内缓存生命周期极短，键形态变化无实际影响（plan_cache 主要服务于长驻网络路径，键输入不变）。
- **lib 显式拒绝（`src/pipeline.rs`）**：`execute_inner` 与 `execute_in_tx` 的 `statements.first()` 截断改为——语句数 > 1 时返回 `Response::Error`（明确文案：该路径仅支持单语句，多语句请用 CLI 分片）。`parse_stage`、`plan_stage`、`execute_stage` 语义不变。
- **护栏退役**：spec `cli-noninteractive-shell` R5（多语句显式拒绝·临时护栏）被新 Requirement「多语句分片逐条执行」替换；R1 文字中「执行单条 SQL」同步修正。
- **测试**：重写 `tests/cli_test.rs::test_multi_statement_rejected`（护栏语义 → 分片执行语义）；新增多语句套件（逐条生效、顺序渲染、fail-fast 部分生效、语法错误零执行、分号边界、lib 显式拒绝）；既有 665 基线回归。

## Out of Scope（本 change 不做）

- SQL 级 `BEGIN/COMMIT/ROLLBACK` 语句（MS11-T02）；多语句脚本作为整体事务。
- 网络协议多结果消息（`Response` 单结果结构不变，网络路径遇多语句显式拒绝）。
- `execute_in_tx` 多语句支持（显式事务路径仍单语句，多语句显式报错）。
- 执行期错误的真·行号定位（token 预扫描；用户决策 3 采用语句序号口径）。
- REPL、生命周期子命令（T05）、Windows、lib API 签名变化（`parse_stage`/`plan_stage`/`execute_stage`/`Response` 签名全部不动）。

## Impact

- **修改**：`src/cli/mod.rs`（`run_sql` 循环 + 渲染序列化 + 护栏移除）、`src/pipeline.rs`（`execute_inner`/`execute_in_tx` 两处 first() → 显式拒绝）。
- **新增**：`tests/cli_test.rs` 多语句用例组（重写 1 + 新增约 6）；无新模块、无新依赖。
- **行为变化**：① `rtsql db "stmt1; stmt2"` 从 exit 3 拒绝 → 逐条执行、顺序输出、exit 0；② 中间失败 → 前 k-1 条已生效 + exit 3 带序号；③ 网络路径/显式事务路径多语句从静默截断 → `Response::Error`；④ 单语句（含尾分号）行为零变化。
- **兼容性**：`parse_stage`/`plan_stage`/`execute_stage`/`Response`/`ExitStatus` 签名不变；`pipeline.rs` 现有 8 个 stage 单测若依赖 first() 截断行为需按显式拒绝语义校准（Phase 2 调查确认）；`plan_cache` 的 CLI 侧键形态变化无跨进程可观察影响。
- **风险**：多段输出对 table/csv/tsv 格式是「多段拼接」，整段 stdout 不再是单一文档——机器消费以 json（逐行独立文档）为准，文案与文档注明；信号中断语义不变（已提交语句保留，close() checkpoint 照常执行）。
