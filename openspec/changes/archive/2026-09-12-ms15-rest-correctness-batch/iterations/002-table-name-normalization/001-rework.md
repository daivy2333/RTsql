# Iteration 002 / Cycle 001-rework: 转义名 dump 行扫描修复（T9-R1）

## Plan Context

- Status: ready
- Iteration: 002-table-name-normalization
- Cycle: 001-rework
- Cycle Type: rework
- Parent cycle: 000-initial.md

**Iteration Scope**

- Change tasks: T9（本 Cycle 以 repair item T9-R1 承载；T7/T8 已于父 Cycle completed，不在本 Cycle 范围）
- Depends on: Iteration 001（accepted）；父 Cycle 000-initial（T7/T8 完成、T9 阻塞于探针 P5）
- Stable baseline: 转义名（catalog 名含引号字符）表 dump→restore→dump 可用且 DDL 表名恒等；裸名 dump/schema 输出与行为零变化；全量回归零回归
- Verification boundary: T9-R1 全绿 + 全量/clippy/fmt/validate + 探针 P4/P5 复跑
- Diagnostic boundary: `src/cli/lifecycle.rs::select_all_rows` + `tests/cli_test.rs` 新增用例
- Deferred tasks: None（Map 末 Iteration）

**Cycle Scope**

- Trigger: rework-required（父 Cycle Plan Review 2026-09-12）
- Acceptance gaps: R1-S3「引号转义按标识符语义解析」——dump 对转义名 catalog 报 `Table 'items' not found`，多代恒等不可满足（父 Cycle Blocker Handoff + Plan Review Finding 1）
- Repair items: T9-R1
- Inherited scope: R3 delta spec 全部 SHALL 面、design D3（含「dump/schema 输出面不变」）、父 Cycle T7/T8 产物与既有测试零修改约束
- Excluded scope: import 的 `INSERT INTO {实参}` 构造（父 Cycle Plan Review Finding 7，D3 边界）、`quote_ident` 输出语义、引擎/planner 面、历史带引号表名迁移

**Objective**

`select_all_rows` 对全部 catalog 名类可解析：转义名 catalog（如 `"items"`）dump 不再报错，输出与该形态改动前同形（`quote_ident` 恒引号 + 转义），`dump→restore→dump` DDL 表名文本恒等；裸名路径逐字节不变。

**Background**

R3 归一化（父 Cycle T8）使引擎表名取 `Ident.value`，catalog 名含引号字符的表（`CREATE TABLE """items"""` → catalog 名 `"items"`；含历史 restore 产物）经裸插值扫描 SQL `SELECT * FROM "items"` 解析为 `items` 而不可达——改动前 Display 语义下裸插值凑巧可解析（父 Cycle Blocker Handoff；Plan Review Finding 1 独立复现）。修复方向为父 Cycle Act 建议并经 Plan Review 采纳：行扫描 SQL 经既有 `quote_ident` 包裹。

**Investigation Facts**

- Current Baseline: 父 Cycle Act Response（2026-09-12）：全量 866 passed / 0 failed / 2 ignored、clippy/fmt 0、validate PASS、探针 P1-P4 PASS、P5 FAIL；T7/T8 completed。工作区未提交（MS15-T01 + 本 change 三 Iteration），对照基线 f9e1e1f——Act 开始前 `git status`/`git diff` 基线检查。
- Current-State Evidence（2026-09-12 Plan 现场复核）:
  - `src/cli/lifecycle.rs:207` `let sql = format!("SELECT * FROM {}", table);`——唯一缺陷行；`:199-203` 函数头注释记录归一化前旧理由（「引擎以 ObjectName 的 Display 形式为表名……quote_ident 反而会给裸名表附加引号致查表失败」），归一化后失效，随修复一并改写。
  - `quote_ident`（`lifecycle.rs:560-562`）：`format!("\"{}\"", name.replace('"', "\"\""))`——对 `"items"` 产出 `"""items"""`，解析回 content `"items"`，与 catalog 名相等；对裸名 `items` 产出 `"items"`，解析为 `items`，相等（父 Cycle T8 归一化语义）。catalog 名恒 lowercase（建表经 helper 归一化），无大小写形态。
  - dump 流程：catalog 枚举表 → `create_table_sql`（输出面，已用 quote_ident，`:536`）→ `select_all_rows`（行扫描面，本修复对象；grep 实证仅 dump 一个调用点）→ 逐行 `INSERT INTO quote_ident(table)`（`:178-181`，输出面已正确）。restore 经主命令路径逐条执行 dump 文本，`"""items"""` 解析为 content `"items"` → 归一化落同名 catalog（父 Cycle T7/T8 见证）。schema/import 不经 `select_all_rows`。
  - 既有测试锚点：`tests/cli_test.rs::test_dump_restore_roundtrip_full_shape`（裸名一代往返）、`test_dump_restore_dump_table_name_stable`（裸名两代恒等）——dump 输出面不变，应零修改通过。
- Code and Critical Path: `select_all_rows`（lifecycle.rs）→ `parse_stage`/`plan_stage`/`execute_stage` 取行集；修复仅改 SQL 文本构造，不触引擎、不触输出渲染、不触其他生命周期子命令。

**Implementation Guidance**

顺序：先加测试观察 RED（dump 报错形态），再改 `select_all_rows` 一行 + 注释，转 GREEN 后跑收尾验证。注释改写要点：归一化后两种拼写汇聚同一 catalog 名，`quote_ident` 对含引号字符名必需、对裸名无害；行扫描 SQL 与输出面使用同一转义 helper。

**Behavioral Change**

- 当前：转义名 catalog 表 dump 报 `failed to dump table "items": Table 'items' not found`（exit 1）。
- 目标：dump 正常输出 `CREATE TABLE """items"""` + 逐行 INSERT（与改动前该形态输出同形）；`dump→restore→dump` 表名行恒等；裸名行为逐字节不变。
- 接口/错误语义：`select_all_rows` 私有函数签名不变；无新增错误路径。

**Task Contracts**

### T9-R1: select_all_rows 经 quote_ident 构造行扫描 SQL（转义名 dump 可用且多代恒等）

- Requirement/Scenario: R1-S3「引号转义按标识符语义解析」（多代 dump/restore 恒等不继续膨胀）+ R3「既有裸名语义零回归」
- Depends on: None（父 Cycle T7/T8 已完成）
- Targets: `src/cli/lifecycle.rs::select_all_rows`（SQL 构造行 + 头注释）；`tests/cli_test.rs`（Iteration 002 节追加 1 用例）
- Current behavior: `format!("SELECT * FROM {}", table)` 裸插值——转义名 catalog 表 dump 报 Table not found
- Required behavior: `format!("SELECT * FROM {}", quote_ident(table))`；新增测试 `test_escaped_name_dump_restore_identity`：`CREATE TABLE """items""" (id INT PRIMARY KEY, n INT)` + INSERT → `dump a` → restore 空库 b → `dump b`，两代 `CREATE TABLE` 行文本恒等（均 `"""items"""`），b 库经 `"""items"""` 拼写 SELECT 见行
- Required changes: 仅上述两文件；函数头注释按归一化后语义改写
- Preserve: dump/schema 输出文本逐字节（既有两个往返用例零修改通过）；`create_table_sql`/`quote_ident`/`sql_literal` 语义；import/restore 路径；引擎与 planner 全部文件
- Forbidden: lifecycle.rs 其他改动（含 import 的 `INSERT INTO {实参}` 构造——Plan Review Finding 7 边界）；修改既有测试；引擎/planner/catalog
- Test witness: `cargo test --test cli_test test_escaped_name_dump_restore_identity` 修复前 RED（dump 失败致断言失败）；修复后 GREEN
- GREEN condition: 该用例 + `test_dump_restore_dump_table_name_stable` + `test_dump_restore_roundtrip_full_shape` 全绿
- Verification: `cargo test --test cli_test`（全文件）+ 收尾验证（见 Verification）
- Stop when: `quote_ident` 形态对任一 catalog 名类解析失败，或全量出现无法归因本面的失败——返回 Plan

**Invariants**

- dump/schema 输出文本逐字节不变；restore 归一化语义不变；catalog/存储格式不变；引擎表名归一化语义（父 Cycle T8）不变。

**Non-goals**

- import 实参插值面；历史带引号表名经裸名/import 实参的可达性恢复；`quote_ident` 输出语义；I034/I037 面；Remaining Issues 2（预存引擎缺陷，范围外）。

**Acceptance**

R1-S3 全 THEN 满足 + R2/R3 零回归。RTM（本 Cycle 范围）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 | 引号转义按标识符语义解析（dump 可用 + 多代恒等） | D3 | T9-R1 | 002 | `lifecycle.rs::select_all_rows` | `test_escaped_name_dump_restore_identity` + 探针 P5 | None | Covered |
| R2 | dump-restore-dump 表名恒等（转义名形态） | D3 | T9-R1 | 002 | 同上 | 同上 | None | Covered |
| R3 | 裸名全链路与既有测试零回归 | D3 | T9-R1 | 002 | 同上 | 既有往返/生命周期/import 套件 + 全量 | None | Covered |

**Verification**

- `cargo test --no-fail-fast`（基线 866 只增不减，预期 867/0/2）、`cargo clippy --all-targets -- -D warnings`（0）、`cargo fmt --check`（0）、`openspec validate --changes`（PASS）。
- 探针 P5 复跑：`CREATE TABLE """items""" (id INT)` + INSERT → `dump` 输出 DDL + INSERT 行（exit 0）；`dump a` → restore → `dump b` 的 CREATE 行 diff 为空。
- Persisted Evidence：none（全部验证可低成本重跑）。

**Gate 2 Readiness**

- 无 Missing requirement：PASS（RTM 3 行全 Covered）
- Simplified requirement 已批准：PASS（无 Simplified 项）
- 调查完整：PASS（缺口面 `select_all_rows`/`quote_ident`/dump-restore 链 2026-09-12 Plan 现场复核 + P5 独立复现；未变化面引用父 Cycle Gate 2 结论——T7/T8 面、既有测试锚点）
- 设计闭合：PASS（修复方向唯一且经父 Cycle Plan Review 采纳；行为差异、边界、禁改面明确）
- 任务可执行：PASS（T9-R1 具备位置/行为/见证/停止条件）
- 分轮合理：PASS（单 repair item：一文件一行 + 一测试，诊断边界单一）
- 追踪完整：PASS（RTM 闭合）
- 验证充分：PASS（RED→GREEN + 既有零修改 + 全量 + 探针）
- 无身份型证据工程：PASS
- 无实质未知项/TBD：PASS（quote_ident 往返语义已由 sqlparser 转义规则与探针实证）
- OpenSpec 产物一致：PASS（proposal/design D3/tasks/delta spec/父 Cycle Review/本 Cycle 交叉一致；delta spec S2/S4 示例语句已经父 Cycle Plan Review 校准——Finding 2/F2）
- Persisted Evidence 模式明确：PASS（none）
- 用户批准计划：PASS（2026-09-12 用户指示 openspec-plan「进行审计，解决回复提到的问题」——Blocker Handoff 已载明修复方向与影响，该指示为对本 Review 修复裁定的授权；风险不超出已批准的 design D3 边界）

**Persisted Evidence**

- Mode: none

Budget: 不适用。

**Risks and Notes**

- `quote_ident` 对含 `"` 名的往返由 sqlparser 转义语义保证（`""`→`"`）+ 探针实证；若全量出现既有 dump/schema 用例失败，按「输出面逐字节不变」先排查是否误改输出渲染（Stop when 面）。
- 工作区未提交产物继续累积——本 Cycle accepted 后 change 实施侧完成，建议用户 commit 后由 docs-maintainer 收尾（沿用 proposal 默认假设 3）。

## Act Response

- Status: reported

**Implemented**

- T9-R1 completed（2026-09-12）：
  - RED 测试见证：`tests/cli_test.rs` Iteration 002 节末新增 `test_escaped_name_dump_restore_identity`——`CREATE TABLE """items""" (id INT PRIMARY KEY, n INT)` + INSERT → `dump a` → restore 空库 b → `dump b`，断言两代 `CREATE TABLE` 行文本恒等（均 `"""items"""`）+ b 库经 `"""items"""` 拼写 SELECT 见行。修复前观察 RED：`dump a` 报 `failed to dump table "items": Table 'items' not found: table not found: items`（exit 1），形态与父 Cycle Blocker Handoff 逐字一致。
  - 实施：`src/cli/lifecycle.rs::select_all_rows` 行扫描 SQL `format!("SELECT * FROM {}", table)` → `format!("SELECT * FROM {}", quote_ident(table))`；函数头注释按归一化后语义改写（两拼写汇聚同一 catalog 名，quote_ident 对含引号字符名必需、对裸名无害；行扫描 SQL 与 dump CREATE/INSERT 输出面同一转义 helper；保留 SELECT * 恒等投影说明）。旧注释「quote_ident 给裸名表附加引号致查表失败」为归一化前 Display 语义下的失效事实，随修复移除。
  - GREEN：新用例 + 既有锚点 `test_dump_restore_dump_table_name_stable`/`test_dump_restore_roundtrip_full_shape` 全绿；T9 收尾验证复跑全过（见 Verification Evidence）——父 Cycle T9 blocked 项全部收口。

**Changed Files and Symbols**

- `src/cli/lifecycle.rs`：`select_all_rows`——SQL 构造行经 `quote_ident` 包裹 + 函数头注释改写（该文件在本 change 前序 Iteration 零修改，本 Cycle 首次进入 diff 面）。
- `tests/cli_test.rs`：Iteration 002 节末追加 `test_escaped_name_dump_restore_identity` 1 用例（+rustfmt 机械重排该用例内 2 处超长实参数组）。
- 其余工作区 diff（MS15-T01 + 本 change Iter 000/001/002 前序产物）未触碰。

**Deviations from Plan**

1. fmt 机械重排：新增用例 2 处超长行经 `cargo fmt` 重排（契约内等价；先例：父 Cycle Deviation 3）。首次 `cargo fmt --check` 观察到 diff 后修复，复跑 0 diff。

**Blocker Handoff**

None（本 Cycle 无阻塞）

**Blocker Resolution**

None

**Self-Review**

- Spec review：T9-R1 契约逐项核对——Targets 两文件；Required changes 形态逐字（`quote_ident` 包裹 + 头注释改写）；新测试结构与契约一致（建表/INSERT/dump/restore/dump 断言恒等 + b 库转义拼写 SELECT 见行）。Preserve 面核实：既有两个往返用例及全部既有测试零修改通过；dump/schema 输出文本不变（输出面 helper 未触碰，select_all_rows 仅影响取行）；import/restore 路径、引擎/planner/catalog 零改动（git diff 仅 lifecycle.rs 两 hunk + cli_test.rs 新增）。Forbidden 面（lifecycle.rs 其他改动——含 import `INSERT INTO {实参}`、修改既有测试、引擎/planner/catalog）未触碰。RTM 3 行全 Covered：R1-S3「多代 dump/restore 恒等不继续膨胀」由 RED→GREEN + 探针 P5 闭合（父 Cycle 唯一 Acceptance gap），R2/R3 零回归由全量见证。
- Code review：完整 diff 复核（不限于逐任务结论）——本 Cycle 变更面 = lifecycle.rs 注释+一行 + cli_test.rs 新用例；无计划外修改、无新增警告（clippy 0）、无死代码/重复实现（复用既有 `quote_ident`，未新增转义逻辑）、无身份型证据机制；测试失败原因正确（RED 形态 = 契约记载的 dump 失败形态，非环境噪声）。
- 已修复发现：① 首次探针运行环境搭建失误——未预建 `$RTSQL_HOME/db` 目录，one-shot 打不开库致 P5/P4 首轮输出无效，按 cli_test fixture 语义预建目录后重跑，两探针 PASS；② 新增用例 fmt diff——rustfmt 修复后复跑全过。均为 Act 侧操作修正，无产品影响。
- 遗留 Minor：None。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| RED 见证 | `cargo test --test cli_test test_escaped_name_dump_restore_identity`（修复前） | `left: Some(1), right: Some(0)`，消息 `dump a failed: failed to dump table "items": Table 'items' not found`，1 failed | R1-S3 缺陷形态 | RED 确认 |
| GREEN 见证 | 同命令（修复后）+ `cargo test --test cli_test test_dump_restore` | 1 passed；3 passed（roundtrip / dump_table_name_stable / roundtrip_full_shape） | T9-R1 目标 + 既有锚点零回归 | PASS |
| cli_test 全文件 | `cargo test --test cli_test` | 65 passed / 0 failed / 2 ignored | R3 面全量 | PASS |
| 全量测试 | `cargo test --no-fail-fast` | 68 套件合计 867 passed / 0 failed / 2 ignored（基线 866 只增不减；本 run 未触发 I041） | 全仓 | PASS |
| 静态检查 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | 0 warning；0 diff（exit 0） | 全仓 | PASS |
| OpenSpec | `openspec validate --changes` | Totals: 1 passed, 0 failed（1 items） | change 产物 | PASS |
| 探针 P5（R1-S3） | `CREATE TABLE """items""" (id INT PRIMARY KEY, n INT)` + INSERT → `dump a` → restore b → `dump b` | dump exit 0：`CREATE TABLE """items""" ("id" INT PRIMARY KEY, "n" INT);` + `INSERT INTO """items""" VALUES (1, 10);`；两代 CREATE 行 diff 空；b 库 `SELECT * FROM """items"""` → `{"columns":["id","n"],"rows":[[1,10]]}` | 转义名 dump 可用 + 多代恒等 | PASS |
| 探针 P4（裸名对照） | `dump c` → restore d → `dump d` | exit 0；CREATE 行 diff 空（P4_CREATE_LINES_IDENTICAL） | 裸名多代 dump 恒等 | PASS |
| 采信结论 | 探针 P1-P3 | 来源：父 Cycle 000-initial Act Response Verification Evidence + Plan Review Findings 8（同一工作树；本 Cycle 变更面 lifecycle.rs::select_all_rows + 新测试不覆盖其结论域；全量重跑含其测试等价面） | I034 表头 / I037 键位清理 / I039 引号互访 | 采信 PASS |

**Persisted Evidence**

None required（Mode: none；全部验证可低成本重跑，探针为单命令序列）。

**Experience Candidates**

None（探针环境需预建 `$RTSQL_HOME/db` 属一次性操作修正，未造成故障，不满足 Runbook/Incident 白名单）。

**Remaining Issues**

1. 工作区未提交产物继续累积（MS15-T01 + 本 change 全部 Iteration 含本 Cycle）——commit 决定权在用户（默认假设 3）。本 Cycle accepted 后 change 实施侧完成，建议用户 commit 后由 docs-maintainer 收尾。
2. 父 Cycle Plan Review 范围外登记项（不变，本 Cycle 未触碰）：跨进程同键 UPDATE→DELETE 变更丢失（Remaining Issues 2，NEW-EVIDENCE）、import 实参插值边界观察——均由 docs-maintainer 收尾登记。

**Commit or Diff Reference**

未提交。本 Cycle diff = `src/cli/lifecycle.rs`（2 hunk）+ `tests/cli_test.rs`（新增 1 用例 + fmt 重排）；工作树含 MS15-T01 与 Iter 000/001/002 前序产物，对照基线 f9e1e1f。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（2026-09-12）：`git status` 基线复核与本 Cycle 声明一致（lifecycle.rs 新进入 diff 面，其余文件集未变化）；`git diff src/cli/lifecycle.rs` 与 T9-R1 契约逐字一致——SQL 构造行经 `quote_ident(table)` 包裹 + 函数头注释按归一化后语义改写（旧失效理由移除，SELECT * 恒等投影说明保留）；`tests/cli_test.rs` 新增 `test_escaped_name_dump_restore_identity` 与契约一致（转义名建表/INSERT → dump a → restore 空库 b → dump b 两代 CREATE 行恒等断言 + b 库转义拼写 SELECT 见行）。Plan 独立复跑探针：`dump a` exit 0 且 DDL `CREATE TABLE """items""" ("id" INT PRIMARY KEY, "n" INT);`、restore 成功、两代 CREATE 行 diff 空（CREATE_LINES_IDENTICAL）、`SELECT * FROM """items"""` 返回 `[[1,10]]`、裸名对照正常——父 Cycle 唯一 Acceptance gap（R1-S3）闭合。Deviation 1（fmt 机械重排 2 处超长行）为契约内等价，非问题。Self-Review 记录的两处 Act 侧操作修正（探针环境预建 `$RTSQL_HOME/db` 后重跑、fmt diff 修复后复跑）无产品影响。遗留 Minor：None。

**Deviation Classification**

- Deviation 1：None——契约内机械重排（先例：父 Cycle Deviation 3）。

**Acceptance Gaps**

None（R1-S3 闭合——独立探针 + RED→GREEN 见证；R2/R3 零回归由全量 867/0/2 与既有锚点零修改见证）

**Convergence**

reduced（父 Cycle 唯一 gap「R1-S3 dump 不可用」→ 本 Cycle 全部 THEN 满足；无新增 gap）

**Evidence**

- 代码与 diff：`git diff src/cli/lifecycle.rs`（2 hunk）；`tests/cli_test.rs::test_escaped_name_dump_restore_identity` 全文核对。
- 探针（2026-09-12 Plan 独立运行，`target/debug/rtsql`，工作树未变）：P5 全链路（dump1 exit 0 / CREATE_LINES_IDENTICAL / 转义拼写 SELECT `[[1,10]]`）+ 裸名对照正常。
- 采信结论：RED 形态、全量 867 passed / 0 failed / 2 ignored、clippy/fmt 0、validate PASS、探针 P4（来源：本文件 Act Response Verification Evidence，同一工作树覆盖未变化）。

**Follow-up Decision**

Acceptance 已满足且无阻塞项、无 Minor 遗留——`accepted`。Iteration 002 完成；Map 无剩余 Iteration，`Next Iteration: None`，change 实施侧完成。建议用户 commit（默认假设 3，commit 决定权在用户）后调用 openspec-docs-maintainer 收尾：I034/I037/I039 转 promoted；登记新 I 项——I037 邻接 rekey 形态（proposal 默认假设 2）、跨进程同键 UPDATE→DELETE 变更丢失（父 Cycle Plan Review Finding 6 含探针复现）、import 实参插值边界观察（Finding 7）；合并 delta specs（cli-noninteractive-shell 修改 + update-index-maintenance / table-name-resolution 新增）；SNAPSHOT/tasks 同步。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Map 末 Iteration accepted——change 实施侧完成）
